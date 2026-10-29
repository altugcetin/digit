use crate::app::App;
use crate::draw::{self, Face};
use crate::paint::Paint;
use gtk::{cairo, gdk, glib, pango, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kit::palette::Rgb;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

const W: i32 = 560;
const H: i32 = 210;

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
        window.set_anchor(Edge::Top, true);
        if !center {
            window.set_anchor(Edge::Left, true);
            window.set_margin(Edge::Left, 64);
        }
        window.set_margin(Edge::Top, if center { (m.geometry().height() as f64 * 0.16) as i32 } else { top });
        let paint = Paint::new(W, H);
        let weak = paint.downgrade();
        let a = Rc::downgrade(app);
        let output = name.clone();
        paint.on_draw(move |s| {
            let (Some(p), Some(app)) = (weak.upgrade(), a.upgrade()) else { return };
            draw_desk(s, &p, &app, &output, center);
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
    let luma = app.wallpapers.top_luma(output, 0.32).unwrap_or(0.0);
    let dark = luma > 0.42;
    let ink = if dark { Rgb::new(0.08, 0.08, 0.1) } else { Rgb::new(1.0, 1.0, 1.0) };
    let shade = if dark { gdk::RGBA::new(1.0, 1.0, 1.0, 0.28) } else { gdk::RGBA::new(0.0, 0.0, 0.0, 0.32) };
    let dt = glib::DateTime::now_local().ok();
    let time = dt.as_ref().and_then(|d| d.format(&cfg.clock).ok()).map(|s| s.to_string()).unwrap_or_default();
    let date = dt.as_ref().and_then(|d| d.format("%A, %-d %B").ok()).map(|s| s.to_string()).unwrap_or_default();
    let align = if center { pango::Alignment::Center } else { pango::Alignment::Left };
    let w = W as f64;
    s.push_shadow(&[gtk::gsk::Shadow::new(shade, 0.0, 1.0, 14.0)]);
    draw::text(s, p, cfg, &date, 21.0, Face::Text(560), ink.alpha(0.88), 0.0, 22.0, w, align);
    draw::text(s, p, cfg, &time, 96.0, Face::Text(330), ink.alpha(0.96), 0.0, 92.0, w, align);
    if let Some(wx) = app.weather.borrow().as_ref() {
        let line = format!("{}°  {}  ·  H {}°  L {}°", wx.temp.round(), crate::weather::describe(wx.code), wx.hi.round(), wx.lo.round());
        let tw = draw::width(p, cfg, &line, 17.0, Face::Text(500)).ceil();
        let gx = if center { (w - tw - 30.0) / 2.0 } else { 2.0 };
        draw::icon(s, p, cfg, crate::weather::glyph(wx.code, wx.day), 22.0, true, ink.alpha(0.9), gx + 11.0, 168.0);
        draw::text(s, p, cfg, &line, 17.0, Face::Text(500), ink.alpha(0.88), gx + 30.0, 168.0, tw + 2.0, pango::Alignment::Left);
        draw::text(s, p, cfg, &wx.place, 14.0, Face::Text(450), ink.alpha(0.66), 0.0, 194.0, if center { w } else { w - 4.0 }, align);
    }
    s.pop();
}
