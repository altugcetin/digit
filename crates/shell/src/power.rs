use crate::app::{App, Peek};
use gtk::glib;
use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Own {
    pub level: u8,
    pub charging: bool,
    pub plugged: bool,
}

impl Own {
    pub fn powered(&self) -> bool {
        self.charging || self.plugged
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Reading {
    now: Option<f64>,
    full: Option<f64>,
    capacity: Option<f64>,
    status: String,
}

fn combine(cells: &[Reading], plugged: bool) -> Option<Own> {
    if cells.is_empty() {
        return None;
    }
    let held: Option<(f64, f64)> = cells.iter().try_fold((0.0, 0.0), |(n, f), c| Some((n + c.now?, f + c.full.filter(|f| *f > 0.0)?)));
    let level = match held {
        Some((now, full)) => 100.0 * now / full,
        None => {
            let known: Vec<f64> = cells.iter().filter_map(|c| c.capacity).collect();
            if known.is_empty() {
                return None;
            }
            known.iter().sum::<f64>() / known.len() as f64
        }
    };
    let charging = cells.iter().any(|c| c.status == "Charging");
    let full = cells.iter().all(|c| c.status == "Full");
    Some(Own { level: level.round().clamp(0.0, 100.0) as u8, charging, plugged: plugged || full || charging })
}

fn number(p: &Path, name: &str) -> Option<f64> {
    std::fs::read_to_string(p.join(name)).ok()?.trim().parse().ok()
}

fn text(p: &Path, name: &str) -> String {
    std::fs::read_to_string(p.join(name)).map(|s| s.trim().to_owned()).unwrap_or_default()
}

fn read() -> Option<Own> {
    let dir = std::fs::read_dir("/sys/class/power_supply").ok()?;
    let mut cells = Vec::new();
    let mut plugged = false;
    for e in dir.flatten() {
        let p = e.path();
        if text(&p, "scope") == "Device" {
            continue;
        }
        match text(&p, "type").as_str() {
            "Battery" => {
                let (now, full) = match (number(&p, "energy_now"), number(&p, "energy_full")) {
                    (Some(n), Some(f)) => (Some(n), Some(f)),
                    _ => (number(&p, "charge_now"), number(&p, "charge_full")),
                };
                cells.push(Reading { now, full, capacity: number(&p, "capacity"), status: text(&p, "status") });
            }
            "Mains" | "USB" => plugged |= number(&p, "online").is_some_and(|o| o > 0.0),
            _ => {}
        }
    }
    combine(&cells, plugged)
}

thread_local! {
    static LAST: Cell<Option<Own>> = const { Cell::new(None) };
    static READING: Cell<bool> = const { Cell::new(false) };
}

pub fn own() -> Option<Own> {
    LAST.with(Cell::get)
}

pub fn battery() -> Option<(u8, bool)> {
    own().map(|b| (b.level, b.powered()))
}

pub fn refresh(app: &Rc<App>) {
    if READING.with(|r| r.replace(true)) {
        return;
    }
    let weak = Rc::downgrade(app);
    glib::spawn_future_local(async move {
        let now = gtk::gio::spawn_blocking(read).await.ok().flatten();
        READING.with(|r| r.set(false));
        LAST.with(|l| l.set(now));
        let Some(app) = weak.upgrade() else { return };
        if app.battery.replace(now) == now {
            return;
        }
        crate::batterywidget::changed();
        if let Some(c) = app.control.borrow().as_ref() {
            c.changed();
        }
        for n in app.notches.borrow().iter() {
            n.restrip();
        }
    });
}

pub fn start(app: &Rc<App>) {
    refresh(app);
    let weak = Rc::downgrade(app);
    let warned = Rc::new(Cell::new(101u8));
    let check = move || {
        let Some(app) = weak.upgrade() else { return glib::ControlFlow::Break };
        let Some(b) = app.battery.get() else { return glib::ControlFlow::Break };
        if b.powered() {
            warned.set(101);
        } else if let Some(t) = [5u8, 10, 20].into_iter().find(|t| b.level <= *t && warned.get() > *t) {
            warned.set(t);
            app.request(Peek::Battery { level: b.level, device: None });
        }
        refresh(&app);
        glib::ControlFlow::Continue
    };
    glib::timeout_add_local_once(Duration::from_secs(8), move || {
        if check() == glib::ControlFlow::Continue {
            glib::timeout_add_local(Duration::from_secs(60), check);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(now: f64, full: f64, capacity: f64, status: &str) -> Reading {
        Reading { now: Some(now), full: Some(full), capacity: Some(capacity), status: status.into() }
    }

    #[test]
    fn two_batteries_count_by_what_each_holds() {
        let both = [cell(2.0, 20.0, 10.0, "Discharging"), cell(60.0, 60.0, 100.0, "Discharging")];
        assert_eq!(combine(&both, false), Some(Own { level: 78, charging: false, plugged: false }));
        assert_eq!(combine(&[cell(28.28, 35.8, 79.0, "Not charging")], true).map(|b| b.level), Some(79));
    }

    #[test]
    fn without_energy_it_averages_the_percentages() {
        let bare = [Reading { capacity: Some(40.0), ..Default::default() }, Reading { capacity: Some(80.0), now: Some(1.0), ..Default::default() }];
        assert_eq!(combine(&bare, false).map(|b| b.level), Some(60));
        assert_eq!(combine(&[], true), None);
        assert_eq!(combine(&[Reading::default()], true), None);
    }

    #[test]
    fn held_at_a_charge_limit_is_on_the_charger_but_not_charging() {
        let held = combine(&[cell(28.28, 35.8, 79.0, "Not charging")], true).unwrap();
        assert!(!held.charging && held.plugged && held.powered());
        let charging = combine(&[cell(20.0, 35.8, 56.0, "Charging")], true).unwrap();
        assert!(charging.charging && charging.powered());
        let away = combine(&[cell(20.0, 35.8, 56.0, "Discharging")], false).unwrap();
        assert!(!away.powered());
        assert!(combine(&[cell(35.8, 35.8, 100.0, "Full")], false).unwrap().plugged);
    }
}
