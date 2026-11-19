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

/// What the chart draws: for each column the time of the three apps with the most time and of
/// all the others, and the labels under some columns.
#[derive(Default)]
struct Chart {
    cols: Vec<[u64; 4]>,
    labels: Vec<(usize, String)>,
}

const TINTS: [(f64, f64, f64); 3] = [(0.04, 0.52, 1.0), (0.35, 0.68, 0.8), (1.0, 0.62, 0.04)];

/// A round axis top over `most` seconds.
fn axis_top(most: u64) -> u64 {
    [1800, 3600, 7200, 14400, 21600, 28800, 43200, 57600, 86400].into_iter().find(|t| *t >= most).unwrap_or(86400)
}

fn axis_label(secs: u64) -> String {
    if secs == 0 { "0".into() } else if secs < 3600 { format!("{}m", secs / 60) } else { format!("{}h", secs / 3600) }
}

/// The columns for the last `range` days: one for each hour of today, or one for each day.
fn chart_for(data: &log::Log, range: usize) -> Chart {
    let list = days(range);
    let top = log::top(list.iter().filter_map(|(k, _)| data.get(k)));
    let rank = |id: &str| top.iter().take(3).position(|(t, _)| t == id).unwrap_or(3);
    let mut chart = Chart::default();
    if range == 1 {
        let today = list.first().and_then(|(k, _)| data.get(k));
        chart.cols = (0..24).map(|h| {
            let mut col = [0u64; 4];
            for (id, u) in today.into_iter().flatten() {
                col[rank(id)] += u.hours.get(h).copied().unwrap_or(0);
            }
            col
        }).collect();
        chart.labels = [(0, "12 AM"), (6, "6 AM"), (12, "12 PM"), (18, "6 PM")].map(|(i, l)| (i, l.to_string())).into();
        return chart;
    }
    for (i, (key, date)) in list.iter().rev().enumerate() {
        let mut col = [0u64; 4];
        for (id, u) in data.get(key).into_iter().flatten() {
            col[rank(id)] += u.secs;
        }
        chart.cols.push(col);
        let label = if range <= 7 { date.format("%a") } else if (range - 1 - i).is_multiple_of(7) { date.format("%-d %b") } else { continue };
        chart.labels.push((i, label.map(|l| l.to_string()).unwrap_or_default()));
    }
    chart
}

fn draw_chart(cr: &gtk::cairo::Context, w: f64, h: f64, (r, g, b): (f64, f64, f64), chart: &Chart) {
    let (left, right, top, base) = (4.0, w - 38.0, 8.0, h - 24.0);
    let most = chart.cols.iter().map(|c| c.iter().sum::<u64>()).max().unwrap_or(0);
    let axis = axis_top(most.max(1));
    let tall = base - top;
    cr.set_line_width(1.0);
    for i in 0..3 {
        let y = (top + tall * i as f64 / 2.0).round() + 0.5;
        cr.set_source_rgba(r, g, b, 0.16);
        cr.move_to(left, y);
        cr.line_to(right, y);
        let _ = cr.stroke();
        let l = kit::paint::layout(cr, &axis_label(axis - axis / 2 * i), "Readex Pro 10");
        let (_, ext) = l.pixel_extents();
        cr.move_to(right + 8.0, y - ext.height() as f64 / 2.0 - ext.y() as f64);
        cr.set_source_rgba(r, g, b, 0.6);
        kit::paint::show(cr, &l);
    }
    let n = chart.cols.len().max(1) as f64;
    let col = (right - left) / n;
    let slim = if n > 12.0 { 0.62 } else { 0.5 };
    for (i, c) in chart.cols.iter().enumerate() {
        let mut at = base;
        for (k, secs) in c.iter().enumerate() {
            if *secs == 0 { continue; }
            let part = (tall * *secs as f64 / axis as f64).max(2.0).min(at - top);
            let (cr_, cg, cb, ca) = TINTS.get(k).map(|t| (t.0, t.1, t.2, 1.0)).unwrap_or((r, g, b, 0.35));
            cr.set_source_rgba(cr_, cg, cb, ca);
            kit::paint::rounded(cr, left + col * i as f64 + col * (1.0 - slim) / 2.0, at - part, col * slim, part, (col * slim / 2.0).min(2.0));
            let _ = cr.fill();
            at -= part;
        }
    }
    for (i, text) in &chart.labels {
        let l = kit::paint::layout(cr, text, "Readex Pro 10");
        let (_, ext) = l.pixel_extents();
        let x = if chart.cols.len() == 24 { left + col * *i as f64 } else { left + col * (*i as f64 + 0.5) - ext.width() as f64 / 2.0 };
        cr.move_to(x.clamp(0.0, w - ext.width() as f64), base + 6.0);
        cr.set_source_rgba(r, g, b, 0.6);
        kit::paint::show(cr, &l);
    }
}

fn headline(total: u64, range: usize, data: &log::Log) -> (String, String) {
    if range == 1 {
        return (log::span(total), "Today".into());
    }
    let used = days(range).iter().filter(|(k, _)| data.get(k).is_some_and(|d| log::total(d) > 0)).count().max(1) as u64;
    (log::span(total / used), "Daily average".into())
}

pub fn build() -> gtk::Widget {
    let (scroll, body) = ui::page("Screen Time", "How long each app had your attention. It counts while you are at the computer and stays on this computer.");
    let data = Rc::new(RefCell::new(log::read()));
    let range = Rc::new(Cell::new(0usize));

    let (pick, end) = ui::row(Some("schedule"), "Show", "");
    let picker = ui::card();
    picker.append(&pick);
    body.append(&picker);

    // The chart card: the total over the columns.
    let summary = ui::card();
    let top = gtk::Box::new(gtk::Orientation::Vertical, 2);
    top.set_margin_top(16);
    top.set_margin_start(18);
    top.set_margin_end(18);
    let caption = ui::text("Today", "sub");
    caption.set_xalign(0.0);
    let total = gtk::Label::new(None);
    total.set_xalign(0.0);
    top.append(&caption);
    top.append(&total);
    let chart = Rc::new(RefCell::new(Chart::default()));
    let area = gtk::DrawingArea::new();
    area.set_content_height(190);
    area.set_margin_start(18);
    area.set_margin_end(18);
    area.set_margin_top(10);
    area.set_margin_bottom(14);
    {
        let chart = chart.clone();
        area.set_draw_func(move |a, cr, w, h| {
            let c = a.color();
            draw_chart(cr, w as f64, h as f64, (c.red() as f64, c.green() as f64, c.blue() as f64), &chart.borrow());
        });
    }
    summary.append(&top);
    summary.append(&area);
    body.append(&summary);

    body.append(&ui::heading("Apps"));
    let apps_card = ui::card();
    body.append(&apps_card);

    let show = {
        let (data, range, chart, area, total, caption, apps_card) = (data.clone(), range.clone(), chart.clone(), area.clone(), total.clone(), caption.clone(), apps_card.clone());
        Rc::new(move || {
            let n = RANGES[range.get()].1;
            let data = data.borrow();
            *chart.borrow_mut() = chart_for(&data, n);
            let sum: u64 = days(n).iter().filter_map(|(k, _)| data.get(k)).map(log::total).sum();
            let (big, small) = headline(sum, n, &data);
            total.set_markup(&format!("<span size=\"xx-large\" weight=\"600\">{}</span>", glib::markup_escape_text(&big)));
            caption.set_text(&small);
            area.queue_draw();
            apps(&apps_card, &data, n);
        })
    };
    let seg = {
        let (range, show) = (range.clone(), show.clone());
        ui::segmented(&RANGES.map(|r| r.0), 0, move |i| {
            range.set(i);
            show();
            true
        })
    };
    end.append(&seg);
    show();

    let refresh = {
        let (data, show) = (data.clone(), show.clone());
        move || {
            *data.borrow_mut() = log::read();
            show();
        }
    };
    // The shell saves every minute; keep what is shown close to that while the page is open.
    let weak = area.downgrade();
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
        let (data, show) = (data.clone(), show.clone());
        clear_button.connect_clicked(move |b| {
            let (data, show) = (data.clone(), show.clone());
            ui::ask(b.root().and_downcast::<gtk::Window>().as_ref(), "Clear screen time?", "The time counted for every app is removed. This can't be undone.", "Cancel", "Clear", "danger", move || {
                log::clear();
                *data.borrow_mut() = log::Log::new();
                show();
            });
        });
    }
    settings.append(&row);
    body.append(&settings);
    scroll.upcast()
}
