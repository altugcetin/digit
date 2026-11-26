//! The Power section of Lock and power: the mode to use on battery and plugged in, Power Saver
//! for a low battery, lighter drawing while saving power, and the battery's charge limit. The
//! shell does the switching (its `powermode`); these rows keep the choices.

use crate::desktop::{choice, section, toggle};
use crate::ui;
use gtk::{gio, glib, prelude::*};
use kit::power;
use std::rc::Rc;

pub fn section_for(body: &gtk::Box, t: &toml::Table) {
    let card = section(body, "Power");
    // A notice goes here when the computer has no power profiles service.
    let notice = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.append(&notice);
    let c = power::Choice::read(t);
    let options = [(power::SAVER, power::short(power::SAVER)), (power::BALANCED, power::short(power::BALANCED)), (power::PERFORMANCE, power::short(power::PERFORMANCE))];
    let battery = choice(&card, "battery_horiz_050", "On battery", "Used once you unplug the charger", power::ON_BATTERY, &options, &c.battery);
    let plugged = choice(&card, "power", "Plugged in", "Used while the charger is plugged in", power::PLUGGED_IN, &options, &c.plugged);
    let low = toggle(&card, "battery_alert", "Power Saver when the battery is low", &format!("At {} % and below, until you plug in", power::LOW), power::LOW_SAVER, c.low_saver);
    let light = toggle(&card, "eco", "Lighter effects in Power Saver", "No blur, shadows or music visualizer while saving power", power::LIGHT, c.light);
    // The charge limit goes here when the battery has one.
    let limit = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.append(&limit);
    let modes = [battery.row, plugged.row, low, light];
    glib::spawn_future_local(async move {
        let Ok(conn) = kit::pk::system().await else { return };
        let (profiles, charge) = futures_util::join!(power::profiles(&conn), power::limit(&conn));
        if profiles.is_none() {
            missing(&notice);
            for row in &modes {
                row.set_sensitive(false);
            }
        }
        if let Some(charge) = charge {
            charge_limit(&limit, conn, charge);
        }
    });
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
