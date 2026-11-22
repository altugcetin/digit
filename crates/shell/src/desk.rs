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

/// How much larger or smaller than as designed the clock is drawn.
fn size(app: &App) -> f64 {
    app.prefs.borrow().size_of("clock")
}

/// The clock's place on the screen `output`: "left", "center" or "off".
fn mode_on(app: &App, output: &str) -> String {
    crate::batterywidget::places(app, output)[0].clone()
}

/// How far a widget at the top left of the screen `output` sits under the clock there: below it
/// and its card. 0 while the clock isn't at the left.
pub fn room(app: &App, output: &str) -> i32 {
    if mode_on(app, output) != "left" {
        return 0;
    }
    let (date, weather) = rows(app);
    let card = if app.prefs.borrow().desktop_clock_bg { PAD } else { 0.0 };
    ((height(date, weather) + card) * size(app) + 8.0).round() as i32
}

/// Where the clock sits on the screen `output`, `screen` big, for desktop icons to keep out of:
/// the column its lines can fill, as tall as what it shows, with its card when it has one.
pub(crate) fn rect(app: &App, output: &str, screen: (i32, i32)) -> Option<crate::batterywidget::CardRect> {
    let mode = mode_on(app, output);
    if mode == "off" || mode.is_empty() || !app.cfg.wallpaper {
        return None;
    }
    let (date, weather) = rows(app);
    let k = size(app);
    let pad = if app.prefs.borrow().desktop_clock_bg { PAD * k } else { 0.0 };
    let (x, y) = if mode == "center" {
        ((screen.0 as f64 - W as f64 * k) / 2.0, (screen.1 as f64 * 0.16).trunc())
    } else {
        (64.0, (app.reserve() + 36) as f64)
    };
    Some(crate::batterywidget::CardRect { x: x - pad, y: y - pad, w: W as f64 * k + 2.0 * pad, h: height(date, weather) * k + 2.0 * pad })
}

/// How long the time's changed digits take to roll in.
const ROLL_MS: i64 = 560;

/// The time as last drawn, and the one it rolls on from while its digits change.
struct Roll {
    shown: String,
    from: Option<String>,
    t: kit::motion::Tween,
    ticking: bool,
}

impl Default for Roll {
    fn default() -> Self {
        Self { shown: String::new(), from: None, t: kit::motion::Tween::still(1.0), ticking: false }
    }
}

impl Roll {
    /// Notes that `time` is to show at `now`; while the digits that changed are on their way,
    /// the time they come from and how far they got. `on`: they roll at all.
    fn step(&mut self, time: &str, now: i64, on: bool) -> Option<(String, f64)> {
        if self.shown != time {
            let was = std::mem::replace(&mut self.shown, time.to_string());
            if on && !was.is_empty() {
                self.from = Some(was);
                self.t = kit::motion::Tween::still(0.0);
                self.t.go(1.0, now, ROLL_MS, kit::motion::Curve::new(0.2, 0.0, 0.0, 1.0));
            }
        }
        if !on || self.t.settled(now) {
            self.from = None;
        }
        Some((self.from.clone()?, self.t.value(now)))
    }
}

/// Where the desktop clock's surface sits on its screen and what it shows there, for the lock
/// screen to settle its own clock onto.
pub struct Spot {
    /// The surface's top left corner, in the screen's coordinates.
    pub origin: (f64, f64),
    /// How much larger than as designed it is drawn.
    pub k: f64,
    pub center: bool,
    /// The middle of the time, as drawn, and how tall its letters are.
    pub time: (f64, f64, f64),
    /// The same for the date, when it shows.
    pub date: Option<(f64, f64, f64)>,
}

/// Where the clock is on the screen `output`, `screen` big, laid out by `p`; None where it's off.
pub fn spot(app: &App, p: &Paint, output: &str, screen: (i32, i32)) -> Option<Spot> {
    let mode = mode_on(app, output);
    if mode == "off" || mode.is_empty() || !app.cfg.wallpaper {
        return None;
    }
    let center = mode == "center";
    let k = size(app);
    let pad = (PAD * k).round();
    let surface_w = ((W as f64 + 2.0 * PAD) * k).ceil();
    let origin = if center {
        (((screen.0 as f64 - surface_w) / 2.0).floor(), (screen.1 as f64 * 0.16).trunc() - pad)
    } else {
        (64.0 - pad, (app.reserve() + 36) as f64 - pad)
    };
    let (date_on, _) = rows(app);
    let cfg = &app.cfg;
    let time = glib::DateTime::now_local().ok().and_then(|d| d.format(&cfg.clock).ok()).map(|s| s.to_string()).unwrap_or_default();
    let date = glib::DateTime::now_local().ok().and_then(|d| d.format("%A, %-d %B").ok()).map(|s| s.to_string()).unwrap_or_default();
    let font = app.prefs.borrow().font_of("clock");
    // The middle of a line, from its left as drawn: centered in the clock's width, or at its left.
    let mid = |text: &str, px: f64, face: Face| {
        if center {
            return PAD + W as f64 / 2.0;
        }
        let wide = std::cell::Cell::new(0.0);
        draw::zoomed(k, || draw::in_font(&font, || wide.set(draw::width(p, cfg, text, px, face))));
        PAD + wide.get() / 2.0
    };
    let at = |x: f64, y: f64, px: f64| (origin.0 + x * k, origin.1 + y * k, px * k);
    let time_y = PAD + if date_on { DATE_H } else { 0.0 } + 48.0;
    Some(Spot {
        origin,
        k,
        center,
        time: at(mid(&time, 96.0, Face::Text(330)), time_y, 96.0),
        date: date_on.then(|| at(mid(&date, 21.0, Face::Text(560)), PAD + 22.0, 21.0)),
    })
}

/// Draws the desktop clock as its surface on `spot`'s screen shows it, onto `s` covering that
/// screen, laid out by `p`.
pub fn paint_at(s: &gtk::Snapshot, p: &Paint, app: &App, output: &str, spot: &Spot) {
    s.save();
    s.translate(&gtk::graphene::Point::new(spot.origin.0 as f32, spot.origin.1 as f32));
    s.scale(spot.k as f32, spot.k as f32);
    let font = app.prefs.borrow().font_of("clock");
    draw::zoomed(spot.k, || draw::in_font(&font, || draw::rasterized(|| draw_desk(s, p, app, output, spot.center, None))));
    s.restore();
}

/// Hides the clock on the screen `output` while the lock screen's settles onto its place, or
/// shows it again; whether there is one there.
pub fn hold(output: &str, hide: bool) -> bool {
    let window = DESKS.with(|d| d.borrow().iter().find(|(n, _, _)| n == output).map(|(_, w, _)| w.clone()));
    let Some(window) = window else { return false };
    if hide {
        window.set_visible(false);
    } else if !window.is_visible() {
        window.present();
    }
    true
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
    let monitors = kit::screens::all();
    // Each screen has the clock where its own places put it.
    let modes: Vec<(gdk::Monitor, String, String)> = monitors
        .into_iter()
        .filter_map(|m| {
            let name = m.connector()?.to_string();
            let mode = mode_on(app, &name);
            Some((m, name, mode))
        })
        .collect();
    let k = size(app);
    let key = format!("{k} {}", modes.iter().map(|(_, n, m)| format!("{n}={m}")).collect::<Vec<_>>().join(" "));
    if MODE.with(|m| *m.borrow() == key) {
        return;
    }
    clear();
    MODE.with(|m| *m.borrow_mut() = key);
    if !app.cfg.wallpaper {
        return;
    }
    let top = app.reserve() + 36;
    for (m, name, mode) in modes {
        if mode == "off" || mode.is_empty() {
            continue;
        }
        let window = gtk::Window::new();
        window.init_layer_shell();
        window.set_layer(Layer::Bottom);
        window.set_namespace(Some("digit-desktop"));
        window.set_keyboard_mode(KeyboardMode::None);
        window.set_exclusive_zone(-1);
        window.set_monitor(Some(&m));
        window.add_css_class("digit");
        let center = mode == "center";
        // The surface reaches PAD past the clock on every side, room for its card; sized, all of
        // it grows from its corner and the clock stays where it starts.
        let pad = (PAD * k).round() as i32;
        window.set_anchor(Edge::Top, true);
        if !center {
            window.set_anchor(Edge::Left, true);
            window.set_margin(Edge::Left, 64 - pad);
        }
        window.set_margin(Edge::Top, if center { (m.geometry().height() as f64 * 0.16) as i32 } else { top } - pad);
        let paint = Paint::new(((W as f64 + 2.0 * PAD) * k).ceil() as i32, ((H as f64 + 2.0 * PAD) * k).ceil() as i32);
        let weak = paint.downgrade();
        let a = Rc::downgrade(app);
        let output = name.clone();
        let roll = Rc::new(RefCell::new(Roll::default()));
        paint.on_draw(move |s| {
            let (Some(p), Some(app)) = (weak.upgrade(), a.upgrade()) else { return };
            s.scale(k as f32, k as f32);
            let font = app.prefs.borrow().font_of("clock");
            let rolling = {
                let mut r = roll.borrow_mut();
                draw::zoomed(k, || draw::in_font(&font, || draw::rasterized(|| draw_desk(s, &p, &app, &output, center, Some(&mut r)))));
                r.from.is_some() && !std::mem::replace(&mut r.ticking, true)
            };
            // Frames until the digits have rolled in.
            if rolling {
                let roll = roll.clone();
                p.add_tick_callback(move |p, _| {
                    p.queue_draw();
                    let mut r = roll.borrow_mut();
                    if r.from.is_some() {
                        glib::ControlFlow::Continue
                    } else {
                        r.ticking = false;
                        glib::ControlFlow::Break
                    }
                });
            }
        });
        window.set_child(Some(&paint));
        // Shown again after unlocking (see `hold`), it takes no clicks either.
        let clickless = |w: &gtk::Window| {
            if let Some(surface) = w.surface() {
                surface.set_input_region(Some(&cairo::Region::create()));
            }
        };
        window.connect_realize(clickless);
        window.connect_map(move |w| clickless(w.upcast_ref()));
        window.present();
        DESKS.with(|d| d.borrow_mut().push((name, window, paint)));
    }
    tick();
}

/// `roll`: the time's digits as they roll in, where the clock draws them.
/// The time on its way from `from` to `to`, `e` of the way there: what both share stays, sliding
/// to where it sits in `to`, and the digits that changed roll up out of the line and in from
/// under it. `row`: the line's left, its top and how wide it is.
#[allow(clippy::too_many_arguments)]
fn rolled(s: &gtk::Snapshot, p: &Paint, cfg: &crate::config::Config, (from, to): (&str, &str), e: f64, ink: kit::palette::Rgb, (x0, top, w): (f64, f64, f64), align: pango::Alignment) {
    let face = Face::Text(330);
    let same = from.chars().zip(to.chars()).take_while(|(a, b)| a == b).count();
    let cut = |t: &str| t.char_indices().nth(same).map_or(t.len(), |(i, _)| i);
    let (head, gone, come) = (&to[..cut(to)], &from[cut(from)..], &to[cut(to)..]);
    let start = |t: &str| if align == pango::Alignment::Center { x0 + (w - draw::width(p, cfg, t, 96.0, face)) / 2.0 } else { x0 };
    let x = start(from) + (start(to) - start(from)) * e;
    let tail = x + if head.is_empty() { 0.0 } else { draw::width(p, cfg, head, 96.0, face) };
    let mid = top + 48.0;
    let left = pango::Alignment::Left;
    draw::text(s, p, cfg, head, 96.0, face, ink, x, mid, w, left);
    s.push_clip(&gtk::graphene::Rect::new((x0 - 48.0) as f32, top as f32, (w + 96.0) as f32, TIME_H as f32));
    let reach = TIME_H * 0.62;
    s.push_opacity((1.0 - e).clamp(0.0, 1.0));
    draw::text(s, p, cfg, gone, 96.0, face, ink, tail, mid - reach * e, w, left);
    s.pop();
    s.push_opacity(e.clamp(0.0, 1.0));
    draw::text(s, p, cfg, come, 96.0, face, ink, tail, mid + reach * (1.0 - e), w, left);
    s.pop();
    s.pop();
}

fn draw_desk(s: &gtk::Snapshot, p: &Paint, app: &App, output: &str, center: bool, roll: Option<&mut Roll>) {
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
    let on = app.prefs.borrow().desktop_motion;
    match roll.and_then(|r| r.step(&time, glib::monotonic_time(), on)) {
        Some((from, e)) => rolled(s, p, cfg, (&from, &time), e, ink.alpha(0.96), (PAD, y, w), align),
        None => draw::text(s, p, cfg, &time, 96.0, Face::Text(330), ink.alpha(0.96), PAD, y + 48.0, w, align),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_time_rolls_only_when_it_changes_and_only_with_animations_on() {
        let mut r = Roll::default();
        assert!(r.step("12:30", 0, true).is_none(), "the first time shows at once");
        assert!(r.step("12:30", 1_000, true).is_none());
        let (from, e) = r.step("12:31", 2_000_000, true).expect("a new minute rolls in");
        assert_eq!((from.as_str(), e), ("12:30", 0.0));
        let (_, half) = r.step("12:31", 2_000_000 + ROLL_MS * 500, true).unwrap();
        assert!(half > 0.0 && half < 1.0);
        assert!(r.step("12:31", 2_000_000 + ROLL_MS * 1000, true).is_none(), "and settles");
        // Off, a new minute just shows.
        assert!(r.step("12:32", 9_000_000, false).is_none());
    }
}
