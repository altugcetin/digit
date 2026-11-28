//! The Power section of Lock and power: the power mode, a mode of its own on battery, Power
//! Saver for a low battery, lighter drawing while saving power, and the battery's charge limit.
//! The shell does the switching (its `powermode`); these rows keep the choices.

use crate::desktop::{save, section, slider, sub_label, toggle};
use crate::ui;
use gtk::{gio, glib, prelude::*};
use kit::power;
use std::cell::Cell;
use std::rc::Rc;

/// What the rows say until the battery is known: most computers asking have one.
const ASSUMED: bool = true;

pub fn section_for(body: &gtk::Box, t: &toml::Table) {
    let card = section(body, "Power");
    // A notice goes here when the computer has no power profiles service.
    let notice = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.append(&notice);
    let c = power::Choice::read(t);
    // Plugged in is the mode a computer without a battery always uses.
    let plugged = Modes::new(&card, "bolt", "Power mode", power::PLUGGED_IN, &c.plugged, false);
    let battery = Modes::new(&card, "battery_horiz_050", "On battery", power::ON_BATTERY, &c.battery, true);
    let low_sub = format!("At {} % and below, until you plug in", power::LOW);
    let low = toggle(&card, "battery_alert", "Power Saver when the battery is low", &low_sub, power::LOW_SAVER, c.low_saver);
    let light = toggle(&card, "eco", "Lighter effects in Power Saver", "No blur, shadows or music visualizer while saving power", power::LIGHT, c.light);
    let (off, sleep) = power::battery_idle(t);
    let idle = [
        slider(&card, "monitor", "Turn off displays on battery", "Minutes without input. 0 keeps them on", power::IDLE_OFF_BATTERY, (0.0, 60.0, 1.0), off, power::IDLE_OFF_BATTERY_DEFAULT, 0),
        slider(&card, "bedtime", "Sleep on battery", "Minutes without input. 0 never sleeps on its own", power::IDLE_SLEEP_BATTERY, (0.0, 180.0, 5.0), sleep, power::IDLE_SLEEP_BATTERY_DEFAULT, 0),
    ];
    // The charge limit goes here when the battery has one.
    let limit = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.append(&limit);
    glib::spawn_future_local(async move {
        let Ok(conn) = kit::pk::system().await else { return };
        let (profiles, charge, source) = futures_util::join!(power::profiles(&conn), power::limit(&conn), power::source(&conn));
        let has_battery = source.is_some_and(|s| s.battery);
        plugged.know(has_battery, profiles.as_ref());
        battery.know(has_battery, profiles.as_ref());
        if !has_battery {
            for row in std::iter::once(&low).chain(idle.iter()) {
                row.set_sensitive(false);
                if let Some(sub) = sub_label(row.upcast_ref()) {
                    sub.set_text(NO_BATTERY);
                }
            }
        }
        if profiles.is_none() {
            missing(&notice);
            for row in [&plugged.row, &battery.row, &low, &light] {
                row.set_sensitive(false);
            }
        }
        if let Some(charge) = charge {
            charge_limit(&limit, conn, charge);
        }
    });
}

const NO_BATTERY: &str = "This computer has no battery";

/// A row choosing the mode for one power source: a glyph for each mode, and the chosen mode's
/// name in the subtitle.
struct Modes {
    row: gtk::Box,
    buttons: Vec<gtk::ToggleButton>,
    sub: Option<gtk::Label>,
    chosen: Rc<Cell<usize>>,
    battery: bool,
}

impl Modes {
    fn new(card: &gtk::Box, icon: &str, title: &str, key: &'static str, current: &str, battery: bool) -> Self {
        let at = power::MODES.iter().position(|m| *m == current).unwrap_or(1);
        let (row, end) = ui::row(Some(icon), title, &said(at, battery, ASSUMED));
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        bar.add_css_class("seg");
        bar.set_valign(gtk::Align::Center);
        let buttons: Vec<gtk::ToggleButton> = power::MODES
            .iter()
            .map(|m| {
                let b = gtk::ToggleButton::new();
                b.set_child(Some(&ui::sym(power::glyph(m))));
                b.set_tooltip_text(Some(power::label(m)));
                b.update_property(&[gtk::accessible::Property::Label(power::label(m))]);
                b.set_focus_on_click(false);
                bar.append(&b);
                b
            })
            .collect();
        let chosen = Rc::new(Cell::new(at));
        let sub = sub_label(row.upcast_ref());
        let (picked, label) = (chosen.clone(), sub.clone());
        ui::select_buttons(&buttons, Some(at), move |i| {
            let ok = save(key, toml::Value::String(power::MODES[i].into()));
            if ok {
                picked.set(i);
                if let Some(l) = &label {
                    l.set_text(&said(i, battery, ASSUMED));
                }
            }
            ok
        });
        end.append(&bar);
        card.append(&row);
        Self { row, buttons, sub, chosen, battery }
    }

    /// Follows what the computer turned out to have: a battery, and which modes.
    fn know(&self, has_battery: bool, profiles: Option<&power::Profiles>) {
        if self.battery && !has_battery {
            self.row.set_sensitive(false);
        }
        if let Some(l) = &self.sub {
            l.set_text(&if self.battery && !has_battery { NO_BATTERY.to_string() } else { said(self.chosen.get(), self.battery, has_battery) });
        }
        let Some(p) = profiles else { return };
        for (b, mode) in self.buttons.iter().zip(power::MODES) {
            if !p.list.iter().any(|m| m == mode) {
                b.set_sensitive(false);
                b.set_tooltip_text(Some(&format!("This computer has no {} mode", power::label(mode))));
            }
        }
    }
}

/// The subtitle for the mode at `at` in `power::MODES`, used on battery or plugged in.
fn said(at: usize, battery: bool, has_battery: bool) -> String {
    let name = power::label(power::MODES.get(at).copied().unwrap_or(power::BALANCED));
    match (battery, has_battery) {
        (true, _) => format!("{name} once you unplug"),
        (false, true) => format!("{name} while plugged in"),
        (false, false) => name.to_string(),
    }
}

/// Power modes need a service that switches them; Digit Store has one.
fn missing(notice: &gtk::Box) {
    let (row, end) = ui::row(Some("download"), "Power modes need power-profiles-daemon", "It tunes the processor and the firmware for each mode. Get it from Digit Store");
    let open = ui::button("Open Store", "accent");
    open.set_valign(gtk::Align::Center);
    open.connect_clicked(|_| {
        let argv: [&std::ffi::OsStr; 2] = ["digit-store".as_ref(), "search:power-profiles-daemon".as_ref()];
        if let Err(e) = gio::Subprocess::newv(&argv, gio::SubprocessFlags::NONE) {
            ui::fail(&format!("Could not open Digit Store: {e}"));
        }
    });
    end.append(&open);
    notice.append(&row);
}

/// A switch for the battery's charge limit, which UPower keeps (and asks polkit about).
fn charge_limit(holder: &gtk::Box, conn: zbus::Connection, charge: power::Limit) {
    let sub = format!("Stops at {} % and charges again below {} %, so the battery ages slower", charge.end, charge.start);
    let (row, end) = ui::row(Some("health_and_safety"), "Charge limit", &sub);
    let sw = gtk::Switch::new();
    sw.set_valign(gtk::Align::Center);
    sw.set_active(charge.on);
    let busy = Rc::new(std::cell::Cell::new(false));
    sw.connect_active_notify(move |sw| {
        if busy.get() {
            return;
        }
        let (on, sw, conn, charge, busy) = (sw.is_active(), sw.clone(), conn.clone(), charge.clone(), busy.clone());
        sw.set_sensitive(false);
        glib::spawn_future_local(async move {
            let result = power::set_limit(&conn, &charge, on).await;
            if let Err(e) = result {
                ui::fail(&format!("Couldn't change the charge limit: {e}"));
                busy.set(true);
                sw.set_active(!on);
                busy.set(false);
            }
            sw.set_sensitive(true);
        });
    });
    end.append(&sw);
    holder.append(&row);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_row_names_its_mode_and_when_it_is_used() {
        assert_eq!(said(0, true, true), "Power Saver once you unplug");
        assert_eq!(said(2, false, true), "Performance while plugged in");
        // Without a battery the one mode is used all the time.
        assert_eq!(said(1, false, false), "Balanced");
    }
}
