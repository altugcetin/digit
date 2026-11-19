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

pub fn start(app: &Rc<App>) {
    let log = Rc::new(RefCell::new(log::read()));
    let state = Rc::new((Cell::new(glib::monotonic_time()), Cell::new(0u32), Cell::new(false)));
    let weak = Rc::downgrade(app);
    glib::timeout_add_seconds_local(TICK, move || {
        let Some(app) = weak.upgrade() else { return glib::ControlFlow::Break };
        let (last, ticks, dirty) = (&state.0, &state.1, &state.2);
        let now = glib::monotonic_time();
        let secs = ((now - last.replace(now)).max(0) / 1_000_000) as u64;
        // Settings clears the history by removing the file; start over rather than write it back.
        if !log::file().exists() && !log.borrow().is_empty() && !dirty.get() {
            log.borrow_mut().clear();
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
                log::add(&mut log.borrow_mut(), &log::today(), &key, &name, secs);
                dirty.set(true);
            }
        }
        ticks.set(ticks.get() + 1);
        if dirty.get() && ticks.get() % SAVE == 0 {
            if let Err(e) = log::write(&log.borrow()) {
                eprintln!("digit: screen time: {e}");
            }
            dirty.set(false);
        }
        glib::ControlFlow::Continue
    });
}
