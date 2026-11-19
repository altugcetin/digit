use crate::{desktop, prefs, ui};
use gtk::{glib, prelude::*};
use kit::screentime as log;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The days the apps list can cover, as the chooser offers them.
const RANGES: [(&str, usize); 3] = [("Today", 1), ("7 days", 7), ("30 days", 30)];
const SHOWN: usize = 12;

/// The last `n` days, today first, as `YYYY-MM-DD`.
fn days(n: usize) -> Vec<(String, glib::DateTime)> {
    let Ok(now) = glib::DateTime::now_local() else { return Vec::new() };
    (0..n as i32).filter_map(|i| now.add_days(-i).ok()).filter_map(|d| Some((d.format("%F").ok()?.to_string(), d))).collect()
}

fn name_of(i: usize, d: &glib::DateTime) -> String {
    match i {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        _ => d.format("%A, %-d %B").map(|s| s.to_string()).unwrap_or_default(),
    }
}

fn bar(fraction: f64) -> gtk::ProgressBar {
    let b = gtk::ProgressBar::new();
    b.set_fraction(fraction.clamp(0.0, 1.0));
    b.set_size_request(160, -1);
    b.set_valign(gtk::Align::Center);
    b
}

fn app_icon(id: &str) -> gtk::Image {
    let themed = gtk::gdk::Display::default().is_some_and(|d| gtk::IconTheme::for_display(&d).has_icon(id));
    let image = match gio_unix::DesktopAppInfo::new(id).and_then(|a| a.icon()) {
        Some(i) => gtk::Image::from_gicon(&i),
        None if themed => gtk::Image::from_icon_name(id),
        None => gtk::Image::from_icon_name("application-x-executable"),
    };
    image.set_pixel_size(32);
    image
}

fn clear(card: &gtk::Box) {
    while let Some(c) = card.first_child() {
        card.remove(&c);
    }
}

/// The seven-day card: one bar for each day.
fn week(card: &gtk::Box, data: &log::Log) {
    clear(card);
    let list = days(7);
    let totals: Vec<u64> = list.iter().map(|(k, _)| data.get(k).map(log::total).unwrap_or(0)).collect();
    let most = totals.iter().copied().max().unwrap_or(0).max(1);
    for (i, ((_, date), secs)) in list.iter().zip(&totals).enumerate() {
        let (row, end) = ui::row(None, &name_of(i, date), "");
        end.append(&bar(*secs as f64 / most as f64));
        let t = ui::text(&if *secs == 0 { "—".into() } else { log::span(*secs) }, "sub");
        t.set_width_chars(13);
        t.set_xalign(1.0);
        end.append(&t);
        card.append(&row);
    }
}

/// The apps card: the apps that had the most time over `range` days.
fn apps(card: &gtk::Box, data: &log::Log, range: usize) {
    clear(card);
    let keys: Vec<String> = days(range).into_iter().map(|(k, _)| k).collect();
    let top = log::top(keys.iter().filter_map(|k| data.get(k)));
    if top.is_empty() {
        card.append(&ui::row(Some("hourglass_empty"), "Nothing yet", "Time shows up here as you use apps").0);
        return;
    }
    let most = top[0].1.secs.max(1);
    for (id, u) in top.iter().take(SHOWN) {
        let (row, end) = ui::row(None, if u.name.is_empty() { id } else { &u.name }, "");
        row.prepend(&app_icon(id));
        end.append(&bar(u.secs as f64 / most as f64));
        let t = ui::text(&log::span(u.secs), "sub");
        t.set_width_chars(13);
        t.set_xalign(1.0);
        end.append(&t);
        card.append(&row);
    }
    if top.len() > SHOWN {
        let rest: u64 = top[SHOWN..].iter().map(|(_, u)| u.secs).sum();
        card.append(&ui::row(Some("more_horiz"), &format!("{} more apps", top.len() - SHOWN), &log::span(rest)).0);
    }
}

pub fn build() -> gtk::Widget {
    let (scroll, body) = ui::page("Screen Time", "How long each app had your attention. It counts while you are at the computer and stays on this computer.");
    let data = Rc::new(RefCell::new(log::read()));
    let range = Rc::new(Cell::new(0usize));

    body.append(&ui::heading("This week"));
    let week_card = ui::card();
    body.append(&week_card);

    body.append(&ui::heading("Apps"));
    let apps_card = ui::card();
    let (pick, end) = ui::row(Some("schedule"), "Show", "");
    let seg = {
        let (data, range, apps_card) = (data.clone(), range.clone(), apps_card.clone());
        ui::segmented(&RANGES.map(|r| r.0), 0, move |i| {
            range.set(i);
            apps(&apps_card, &data.borrow(), RANGES[i].1);
            true
        })
    };
    end.append(&seg);
    let picker = ui::card();
    picker.append(&pick);
    body.append(&picker);
    body.append(&apps_card);

    let refresh = {
        let (data, range, week_card, apps_card) = (data.clone(), range.clone(), week_card.clone(), apps_card.clone());
        move || {
            *data.borrow_mut() = log::read();
            week(&week_card, &data.borrow());
            apps(&apps_card, &data.borrow(), RANGES[range.get()].1);
        }
    };
    refresh();
    // The shell saves every minute; keep what is shown close to that while the page is open.
    let weak = week_card.downgrade();
    glib::timeout_add_seconds_local(30, move || {
        if weak.upgrade().is_some_and(|w| w.is_mapped()) {
            refresh();
        }
        if weak.upgrade().is_some() { glib::ControlFlow::Continue } else { glib::ControlFlow::Break }
    });

    body.append(&ui::heading("Counting"));
    let settings = ui::card();
    desktop::toggle(&settings, "hourglass_top", "Count screen time", "Adds up the time each app has the focus", log::KEY, prefs::get_bool(&prefs::read(), log::KEY, true));
    let (row, end) = ui::row(Some("delete_sweep"), "Clear history", "Removes the time counted so far");
    let clear_button = ui::button("Clear…", "danger");
    end.append(&clear_button);
    {
        let (week_card, apps_card, data) = (week_card.clone(), apps_card.clone(), data.clone());
        clear_button.connect_clicked(move |b| {
            let (week_card, apps_card, data, range) = (week_card.clone(), apps_card.clone(), data.clone(), range.clone());
            ui::ask(b.root().and_downcast::<gtk::Window>().as_ref(), "Clear screen time?", "The time counted for every app is removed. This can't be undone.", "Cancel", "Clear", "danger", move || {
                log::clear();
                *data.borrow_mut() = log::Log::new();
                week(&week_card, &data.borrow());
                apps(&apps_card, &data.borrow(), RANGES[range.get()].1);
            });
        });
    }
    settings.append(&row);
    body.append(&settings);
    scroll.upcast()
}
