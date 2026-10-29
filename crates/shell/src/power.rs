use crate::app::{App, Peek};
use gtk::glib;
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

pub fn battery() -> Option<(u8, bool)> {
    let dir = std::fs::read_dir("/sys/class/power_supply").ok()?;
    for e in dir.flatten() {
        let p = e.path();
        let kind = std::fs::read_to_string(p.join("type")).unwrap_or_default();
        if kind.trim() != "Battery" || std::fs::read_to_string(p.join("scope")).is_ok_and(|s| s.trim() == "Device") {
            continue;
        }
        let level = std::fs::read_to_string(p.join("capacity")).ok()?.trim().parse::<u8>().ok()?;
        let status = std::fs::read_to_string(p.join("status")).unwrap_or_default();
        return Some((level.min(100), matches!(status.trim(), "Charging" | "Full")));
    }
    None
}

pub fn start(app: &Rc<App>) {
    if battery().is_none() {
        return;
    }
    let weak = Rc::downgrade(app);
    let warned = Rc::new(Cell::new(101u8));
    let check = move || {
        let Some(app) = weak.upgrade() else { return glib::ControlFlow::Break };
        if let Some((level, charging)) = battery() {
            if charging {
                warned.set(101);
            } else if let Some(t) = [5u8, 10, 20].into_iter().find(|t| level <= *t && warned.get() > *t) {
                warned.set(t);
                app.request(Peek::Battery { level, device: None });
            }
        }
        glib::ControlFlow::Continue
    };
    let first = check.clone();
    glib::timeout_add_local_once(Duration::from_secs(8), move || {
        first();
    });
    glib::timeout_add_local(Duration::from_secs(60), check);
}
