//! Counts the focused app's time into `kit::screentime`, for the Screen Time page in Settings.
//! It only counts while someone is at the computer: not when it is idle or locked, and not when
//! no window has the focus.

use crate::app::App;
use gtk::glib;
use kit::screentime as log;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const TICK: u32 = 10;
/// The log is written every this many ticks, when it changed.
const SAVE: u32 = 6;

thread_local! {
    static LOG: RefCell<log::Log> = RefCell::new(log::read());
}

/// What the desktop widget shows for today: the total, the time in each hour for the three apps
/// with the most time and for all the others, and the four apps with the most time.
pub struct Summary {
    pub total: u64,
    pub hours: [[u64; 24]; 4],
    /// App id, name and time.
    pub top: Vec<(String, String, u64)>,
}

pub fn summary() -> Summary {
    LOG.with(|l| {
        let l = l.borrow();
        let day = l.get(&log::today());
        let top = day.map(|d| log::top([d])).unwrap_or_default();
        let mut hours = [[0u64; 24]; 4];
        for (i, (id, _)) in top.iter().enumerate() {
            for (h, secs) in day.and_then(|d| d.get(id)).map(|u| u.hours.as_slice()).unwrap_or(&[]).iter().enumerate().take(24) {
                hours[i.min(3)][h] += secs;
            }
        }
        Summary {
            total: day.map(log::total).unwrap_or(0),
            hours,
            top: top.into_iter().take(4).map(|(id, u)| (id.clone(), if u.name.is_empty() { id } else { u.name }, u.secs)).collect(),
        }
    })
}

pub fn start(app: &Rc<App>) {
    let state = Rc::new((Cell::new(glib::monotonic_time()), Cell::new(0u32), Cell::new(false)));
    let weak = Rc::downgrade(app);
    glib::timeout_add_seconds_local(TICK, move || {
        let Some(app) = weak.upgrade() else { return glib::ControlFlow::Break };
        let (last, ticks, dirty) = (&state.0, &state.1, &state.2);
        let now = glib::monotonic_time();
        let secs = ((now - last.replace(now)).max(0) / 1_000_000) as u64;
        // Settings clears the history by removing the file; start over rather than write it back.
        if !log::file().exists() && !dirty.get() && LOG.with(|l| !l.borrow().is_empty()) {
            LOG.with(|l| l.borrow_mut().clear());
            crate::deskcards::screentime_changed();
        }
        // A tick that came much late means the timer was held up; nobody can say who was there.
        if secs > 0 && secs <= 3 * TICK as u64 && app.prefs.borrow().screen_time && !crate::idle::away() {
            let focused = app.running.borrow().iter().find(|r| r.focused && !r.app_id.is_empty()).map(|r| r.app_id.clone());
            if let Some(id) = focused {
                let list = app.apps.list();
                let (key, name) = match app.apps.find(&id) {
                    Some(i) => (list[i].id.clone(), list[i].name.clone()),
                    None => (id.clone(), crate::apps::pretty(&id)),
                };
                LOG.with(|l| log::add(&mut l.borrow_mut(), &log::today(), &key, &name, glib::DateTime::now_local().map(|d| d.hour() as usize).unwrap_or(0), secs));
                dirty.set(true);
                crate::deskcards::screentime_changed();
            }
        }
        ticks.set(ticks.get() + 1);
        if dirty.get() && ticks.get() % SAVE == 0 {
            if let Err(e) = LOG.with(|l| log::write(&l.borrow())) {
                eprintln!("digit: screen time: {e}");
            }
            dirty.set(false);
        }
        glib::ControlFlow::Continue
    });
}
