use crate::app::App;
use crate::draw::{self, Face};
use crate::paint::Paint;
use gtk::{cairo, gdk, glib, pango, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

const W: i32 = 560;
const H: i32 = 210;
/// Room around the clock for its card, which reaches this far past what it shows.
const PAD: f64 = crate::deskcards::CARD_PAD;
/// The heights of the date, the time and the weather (with the gap above it).
const DATE_H: f64 = 44.0;
const TIME_H: f64 = 96.0;
const WEATHER_H: f64 = 66.0;

/// Which of the date and the weather show under the clock.
fn rows(app: &App) -> (bool, bool) {
    let p = app.prefs.borrow();
    (p.desktop_clock_date, p.desktop_clock_weather && !p.weather_city.trim().is_empty())
}

/// How tall what the clock shows is.
fn height(date: bool, weather: bool) -> f64 {
    TIME_H + if date { DATE_H } else { 0.0 } + if weather { WEATHER_H } else { 0.0 }
}

/// How far a widget at the top left sits under the clock there: below it and its card. 0 while
/// the clock isn't at the left.
pub fn room(app: &App) -> i32 {
    if app.prefs.borrow().desktop_widget != "left" {
        return 0;
    }
    let (date, weather) = rows(app);
    let card = if app.prefs.borrow().desktop_clock_bg { PAD } else { 0.0 };
    (height(date, weather) + card + 8.0).round() as i32
}

/// Where the clock sits on a screen `screen` big, for desktop icons to keep out of: the
/// column its lines can fill, as tall as what it shows, with its card when it has one.
pub(crate) fn rect(app: &App, screen: (i32, i32)) -> Option<crate::batterywidget::CardRect> {
    let mode = app.prefs.borrow().desktop_widget.clone();
    if mode == "off" || mode.is_empty() || !app.cfg.wallpaper {
        return None;
    }
    let (date, weather) = rows(app);
    let pad = if app.prefs.borrow().desktop_clock_bg { PAD } else { 0.0 };
    let (x, y) = if mode == "center" {
        ((screen.0 - W) as f64 / 2.0, (screen.1 as f64 * 0.16).trunc())
    } else {
        (64.0, (app.reserve() + 36) as f64)
    };
    Some(crate::batterywidget::CardRect { x: x - pad, y: y - pad, w: W as f64 + 2.0 * pad, h: height(date, weather) + 2.0 * pad })
}

thread_local! {
    static DESKS: RefCell<Vec<(String, gtk::Window, Paint)>> = const { RefCell::new(Vec::new()) };
    static MODE: RefCell<String> = const { RefCell::new(String::new()) };
    static TIMER: RefCell<Option<glib::SourceId>> = const { RefCell::new(None) };
}

pub fn changed(_app: &App) {
    DESKS.with(|d| d.borrow().iter().for_each(|(_, _, p)| p.queue_draw()));
}

fn clear() {
    let list = DESKS.with(|d| std::mem::take(&mut *d.borrow_mut()));
    for (_, w, _) in list {
        w.destroy();
    }
    if let Some(id) = TIMER.with(|t| t.borrow_mut().take()) {
        id.remove();
    }
}

fn tick() {
    let wait = 60 - glib::DateTime::now_local().map(|d| d.second()).unwrap_or(0).clamp(0, 59) as u64;
    let id = glib::timeout_add_local_once(Duration::from_secs(wait.max(1)), || {
        TIMER.with(|t| t.borrow_mut().take());
        DESKS.with(|d| d.borrow().iter().for_each(|(_, _, p)| p.queue_draw()));
        if DESKS.with(|d| !d.borrow().is_empty()) {
            tick();
        }
    });
    TIMER.with(|t| *t.borrow_mut() = Some(id));
}

pub fn apply(app: &Rc<App>) {
    let mode = app.prefs.borrow().desktop_widget.clone();
    let same = MODE.with(|m| *m.borrow() == mode);
    let Some(display) = gdk::Display::default() else { return };
    let monitors: Vec<gdk::Monitor> = display.monitors().iter::<gdk::Monitor>().filter_map(Result::ok).collect();
    let names: Vec<String> = monitors.iter().filter_map(|m| m.connector().map(|c| c.to_string())).collect();
    let current: Vec<String> = DESKS.with(|d| d.borrow().iter().map(|(n, _, _)| n.clone()).collect());
    if same && current == names {
        return;
    }
    clear();
    MODE.with(|m| *m.borrow_mut() = mode.clone());
    if mode == "off" || mode.is_empty() || !app.cfg.wallpaper {
        return;
    }
    let top = app.reserve() + 36;
    for m in monitors {
        let Some(name) = m.connector().map(|c| c.to_string()) else { continue };
        let window = gtk::Window::new();
        window.init_layer_shell();
        window.set_layer(Layer::Bottom);
        window.set_namespace(Some("digit-desktop"));
        window.set_keyboard_mode(KeyboardMode::None);
        window.set_exclusive_zone(-1);
        window.set_monitor(Some(&m));
        window.add_css_class("digit");
        let center = mode == "center";
        // The surface reaches PAD past the clock on every side, room for its card.
        let pad = PAD as i32;
        window.set_anchor(Edge::Top, true);
        if !center {
            window.set_anchor(Edge::Left, true);
            window.set_margin(Edge::Left, 64 - pad);
        }
        window.set_margin(Edge::Top, if center { (m.geometry().height() as f64 * 0.16) as i32 } else { top } - pad);
        let paint = Paint::new(W + 2 * pad, H + 2 * pad);
        let weak = paint.downgrade();
        let a = Rc::downgrade(app);
        let output = name.clone();
        paint.on_draw(move |s| {
            let (Some(p), Some(app)) = (weak.upgrade(), a.upgrade()) else { return };
            draw::rasterized(|| draw_desk(s, &p, &app, &output, center));
        });
        window.set_child(Some(&paint));
        window.connect_realize(|w| {
            if let Some(surface) = w.surface() {
                surface.set_input_region(Some(&cairo::Region::create()));
            }
        });
        window.present();
        DESKS.with(|d| d.borrow_mut().push((name, window, paint)));
    }
    tick();
}

fn draw_desk(s: &gtk::Snapshot, p: &Paint, app: &App, output: &str, center: bool) {
    let cfg = &app.cfg;
    let (date_on, weather_on) = rows(app);
    let look = crate::deskcards::look(app, output, true, app.prefs.borrow().desktop_clock_bg, "clock");
    let ink = look.ink;
    let dt = glib::DateTime::now_local().ok();
    let time = dt.as_ref().and_then(|d| d.format(&cfg.clock).ok()).map(|s| s.to_string()).unwrap_or_default();
    let date = dt.as_ref().and_then(|d| d.format("%A, %-d %B").ok()).map(|s| s.to_string()).unwrap_or_default();
    let wx = app.weather.borrow().clone().filter(|_| weather_on);
    let line = wx.as_ref().map(|wx| format!("{}°  {}  ·  H {}°  L {}°", wx.temp.round(), crate::weather::describe(wx.code), wx.hi.round(), wx.lo.round()));
    let align = if center { pango::Alignment::Center } else { pango::Alignment::Left };
    let w = W as f64;
    // What shows, top to bottom, and how wide its widest line is.
    let mut widths = vec![draw::width(p, cfg, &time, 96.0, Face::Text(330))];
    if date_on {
        widths.push(draw::width(p, cfg, &date, 21.0, Face::Text(560)));
    }
    if let (Some(line), Some(wx)) = (&line, &wx) {
        widths.push(draw::width(p, cfg, line, 17.0, Face::Text(500)) + 30.0);
        widths.push(draw::width(p, cfg, &wx.place, 14.0, Face::Text(450)));
    }
    let cw = widths.into_iter().fold(0.0_f64, f64::max).ceil().min(w);
    if let Some(fill) = look.card {
        let x0 = if center { PAD + (w - cw) / 2.0 } else { PAD };
        let h = height(date_on, wx.is_some());
        crate::deskcards::card(s, app, (x0 - PAD, 0.0, cw + 2.0 * PAD, h + 2.0 * PAD), fill);
    }
    if let Some(shade) = look.shade {
        s.push_shadow(&[gtk::gsk::Shadow::new(shade, 0.0, 1.0, 14.0)]);
    }
    let mut y = PAD;
    if date_on {
        draw::text(s, p, cfg, &date, 21.0, Face::Text(560), ink.alpha(0.88), PAD, y + 22.0, w, align);
        y += DATE_H;
    }
    draw::text(s, p, cfg, &time, 96.0, Face::Text(330), ink.alpha(0.96), PAD, y + 48.0, w, align);
    y += TIME_H;
    if let (Some(line), Some(wx)) = (line, wx) {
        y += 10.0;
        let tw = draw::width(p, cfg, &line, 17.0, Face::Text(500)).ceil();
        let gx = PAD + if center { (w - tw - 30.0) / 2.0 } else { 2.0 };
        draw::icon(s, p, cfg, crate::weather::glyph(wx.code, wx.day), 22.0, true, ink.alpha(0.9), gx + 11.0, y + 18.0);
        draw::text(s, p, cfg, &line, 17.0, Face::Text(500), ink.alpha(0.88), gx + 30.0, y + 18.0, tw + 2.0, pango::Alignment::Left);
        draw::text(s, p, cfg, &wx.place, 14.0, Face::Text(450), ink.alpha(0.66), PAD, y + 44.0, if center { w } else { w - 4.0 }, align);
    }
    if look.shade.is_some() {
        s.pop();
    }
}
