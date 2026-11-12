use crate::app::App;
use crate::draw::{self, Face};
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{glib, graphene, pango};
use std::rc::Rc;

const TEXT: f64 = 15.5;
/// Workspace row beside the logo: dot size, distance between centres, the active pill's size.
const DOT: f64 = 6.0;
const STEP: f64 = 16.0;
const PILL: f64 = 17.0;
const PILL_H: f64 = 8.0;
const EDGE: f64 = 10.0;
/// How long the active pill takes to flow to its new workspace, and how long the row stays out.
const MORPH: i64 = 520_000;
const HOLD: i64 = 1_500_000;
const NUM: f64 = 14.5;
const ICON: f64 = 18.5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Zone {
    Logo,
    Spaces,
    App,
    Menu(u8),
    Win,
    Status,
    Tray(u8),
    Date,
}

#[derive(Clone, Copy, Default)]
pub struct Look {
    pub widen: f64,
    pub slab: f64,
    pub radius: f64,
    pub lift: f64,
    pub menu: f64,
    pub glass: f64,
    pub luma: f64,
    pub blur: bool,
}

#[derive(Clone, Copy)]
struct Battery {
    level: u8,
    charging: bool,
}

/// The workspace pill: which workspaces exist on this output (and whether they hold windows),
/// which is active, and the motion of the active mark between them.
pub struct Spaces {
    list: Vec<bool>,
    active: usize,
    from: f64,
    moved: i64,
    /// Always on, instead of only while switching.
    always: bool,
    shown: bool,
    open: kit::motion::Tween,
    until: i64,
    ticking: bool,
}

impl Default for Spaces {
    fn default() -> Self {
        Self { list: Vec::new(), active: 0, from: 0.0, moved: 0, always: false, shown: false, open: kit::motion::Tween::still(0.0), until: 0, ticking: false }
    }
}

fn ease_out(x: f64) -> f64 {
    1.0 - (1.0 - x.clamp(0.0, 1.0)).powi(3)
}

fn ease_both(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    if x < 0.5 { 4.0 * x * x * x } else { 1.0 - (-2.0 * x + 2.0).powi(3) / 2.0 }
}

/// Front and back of the active mark, in workspace positions, while it moves from `from` to `to`
/// (`t` 0..1): the front leaves first, the back follows, so the mark stretches and then settles.
fn ends(from: f64, to: f64, t: f64) -> (f64, f64) {
    if t >= 1.0 {
        return (to, to);
    }
    (from + (to - from) * ease_out(t / 0.7), from + (to - from) * ease_both((t - 0.2) / 0.8))
}

impl Spaces {
    fn at(&self, now: i64) -> (f64, f64) {
        let t = if kit::motion::reduced() { 1.0 } else { (now - self.moved) as f64 / MORPH as f64 };
        ends(self.from, self.active as f64, t)
    }

    /// The pill's width when fully open.
    fn width(&self) -> f64 {
        if self.list.is_empty() { 0.0 } else { 2.0 * EDGE + (self.list.len() - 1) as f64 * STEP + PILL }
    }
}

fn motion_in(x: f64) -> f64 {
    x * x * (3.0 - 2.0 * x)
}

fn smooth(x: f64) -> f64 {
    motion_in(x.clamp(0.0, 1.0))
}

fn battery() -> Option<Battery> {
    let dir = std::fs::read_dir("/sys/class/power_supply").ok()?;
    for e in dir.flatten() {
        let p = e.path();
        let kind = std::fs::read_to_string(p.join("type")).unwrap_or_default();
        if kind.trim() != "Battery" || std::fs::read_to_string(p.join("scope")).is_ok_and(|s| s.trim() == "Device") {
            continue;
        }
        let level = std::fs::read_to_string(p.join("capacity")).ok()?.trim().parse::<u8>().ok()?;
        let status = std::fs::read_to_string(p.join("status")).unwrap_or_default();
        return Some(Battery { level: level.min(100), charging: matches!(status.trim(), "Charging" | "Full") });
    }
    None
}

fn battery_glyph(b: Battery) -> &'static str {
    if b.charging {
        return "battery_charging_full";
    }
    match b.level {
        0..=10 => "battery_alert",
        11..=30 => "battery_2_bar",
        31..=55 => "battery_4_bar",
        56..=85 => "battery_6_bar",
        _ => "battery_full",
    }
}

impl Strip {
    pub fn new(app: &Rc<App>, header: f64, keep: f64) -> Self {
        let s: Self = glib::Object::new();
        let imp = s.imp();
        *imp.app.borrow_mut() = Rc::downgrade(app);
        imp.header.set(header);
        imp.keep.set(keep);
        s
    }

    pub fn follow(&self, f: impl Fn(i64) -> Option<Look> + 'static) {
        *self.imp().follow.borrow_mut() = Some(Box::new(f));
    }

    pub fn set_header(&self, header: f64) {
        self.imp().header.set(header);
        self.queue_draw();
    }

    pub fn on_resize(&self, f: impl Fn(i32) + 'static) {
        *self.imp().resize.borrow_mut() = Some(Box::new(f));
    }

    pub fn zone_x(&self, zone: Zone) -> Option<f64> {
        self.imp().zones.borrow().iter().find(|(z, _)| *z == zone).map(|(_, r)| r.0)
    }

    pub fn zone_end(&self, zone: Zone) -> Option<f64> {
        self.imp().zones.borrow().iter().find(|(z, _)| *z == zone).map(|(_, r)| r.0 + r.2)
    }

    pub fn zone_at(&self, x: f64, y: f64) -> Option<Zone> {
        self.imp().pointer.set(x);
        if y > self.imp().header.get() + 2.0 * self.imp().lift.get() {
            return None;
        }
        self.imp().zones.borrow().iter().find(|(_, r)| x >= r.0 && x <= r.0 + r.2).map(|(z, _)| *z)
    }

    pub fn hover(&self, zone: Option<Zone>) {
        if self.imp().hot.replace(zone) != zone {
            self.queue_draw();
        }
    }

    pub fn hold(&self, zone: Option<Zone>) {
        if self.imp().held.replace(zone) != zone {
            self.queue_draw();
        }
    }

    /// The zone under the pointer, as last set by `hover`.
    pub fn hot(&self) -> Option<Zone> {
        self.imp().hot.get()
    }

    pub fn held(&self) -> Option<Zone> {
        self.imp().held.get()
    }

    /// The workspaces on this output changed. When the active one moves, the workspace pill opens
    /// beside the logo (unless it is always shown) and the active mark slides to its new place;
    /// the pill folds away again a moment later.
    pub fn set_spaces(&self, list: Vec<bool>, active: usize, always: bool) {
        let imp = self.imp();
        let now = App::now();
        {
            let mut sp = imp.spaces.borrow_mut();
            if sp.list == list && sp.active == active && sp.always == always {
                return;
            }
            let first = sp.list.is_empty();
            let moved = !first && sp.active != active;
            if moved {
                // Leave from wherever the mark is drawn now, even mid-move.
                let (head, _) = sp.at(now);
                sp.from = head.min(list.len().saturating_sub(1) as f64);
                sp.moved = now;
                sp.until = now + HOLD;
            } else if first {
                sp.from = active as f64;
                sp.moved = now - MORPH;
            }
            sp.always = always;
            let want = always || (moved || sp.shown) && now < sp.until;
            if want != sp.shown {
                if want {
                    sp.open.go(1.0, now, 280, kit::motion::EMPHASIZED_DECEL);
                } else {
                    sp.open.go(0.0, now, 320, kit::motion::STANDARD);
                }
                sp.shown = want;
            }
            sp.list = list;
            sp.active = active.min(sp.list.len().saturating_sub(1));
            if std::mem::replace(&mut sp.ticking, true) {
                drop(sp);
                self.queue_draw();
                return;
            }
        }
        self.add_tick_callback(|w, clock| {
            let now = clock.frame_time().max(App::now());
            w.queue_draw();
            let mut sp = w.imp().spaces.borrow_mut();
            if sp.shown && !sp.always && now >= sp.until {
                sp.open.go(0.0, now, 320, kit::motion::STANDARD);
                sp.shown = false;
            }
            let busy = (sp.shown && !sp.always) || !sp.open.settled(now) || now - sp.moved < MORPH;
            if busy {
                glib::ControlFlow::Continue
            } else {
                sp.ticking = false;
                glib::ControlFlow::Break
            }
        });
    }

    /// Draws the workspace dots inside the pill that starts at `x0`, and the active mark: a bar of
    /// constant height that stretches toward the new workspace and settles there.
    fn draw_spaces(&self, s: &gtk::Snapshot, sp: &Spaces, x0: f64, mid: f64, now: i64, ink: kit::palette::Rgb, accent: kit::palette::Rgb) {
        let first = x0 + EDGE + PILL / 2.0;
        let at = |i: f64| first + i * STEP;
        let (head, tail) = sp.at(now);
        let (lo, hi) = (head.min(tail), head.max(tail));
        for (i, filled) in sp.list.iter().enumerate() {
            let i = i as f64;
            if i >= lo - 0.45 && i <= hi + 0.45 {
                continue;
            }
            let d = if *filled { DOT } else { DOT * 0.8 };
            let color = if *filled { ink.alpha(0.88) } else { ink.alpha(0.38) };
            let c = at(i);
            draw::fill(s, &draw::round(c - d / 2.0, mid - d / 2.0, d, d, d / 2.0), color);
        }
        let (l, r) = (at(lo) - PILL / 2.0, at(hi) + PILL / 2.0);
        draw::fill(s, &draw::round(l, mid - PILL_H / 2.0, r - l, PILL_H, PILL_H / 2.0), accent);
    }

    /// The workspace dot under the pointer, as last seen by `zone_at`.
    pub fn hovered_space(&self) -> Option<usize> {
        self.space_at(self.imp().pointer.get())
    }

    /// Which workspace dot of the pill is under `x`, if any.
    pub fn space_at(&self, x: f64) -> Option<usize> {
        let (x0, _, w, _) = self.imp().zones.borrow().iter().find(|(z, _)| *z == Zone::Spaces).map(|(_, r)| *r)?;
        let n = self.imp().spaces.borrow().list.len();
        if n == 0 || x < x0 || x > x0 + w {
            return None;
        }
        let first = x0 + EDGE + PILL / 2.0;
        Some((((x - first) / STEP).round().max(0.0) as usize).min(n - 1))
    }

    fn draw(&self, s: &gtk::Snapshot) {
        let imp = self.imp();
        let Some(app) = imp.app.borrow().upgrade() else { return };
        let now = self.frame_clock().map(|c| c.frame_time()).unwrap_or_else(App::now);
        let pal = app.palette_at(now);
        let w = self.width() as f64;
        let h = imp.header.get();
        let look = imp.follow.borrow().as_ref().and_then(|f| f(now)).unwrap_or(Look { slab: imp.keep.get(), radius: 14.0, ..Look::default() });
        let (t, slab, radius, lift, menu) = (look.widen, look.slab, look.radius, look.lift, look.menu);
        imp.lift.set(lift);
        let mid = lift * (1.0 - t) + h / 2.0;
        if (t <= 0.002 && menu <= 0.002) || w <= imp.keep.get() + 1.0 {
            imp.zones.borrow_mut().clear();
            return;
        }
        let zero = graphene::Size::zero();
        let band = h + 2.0 * lift * (1.0 - t);
        let full = gtk::gsk::RoundedRect::new(graphene::Rect::new(0.0, 0.0, w as f32, band as f32), zero, zero, zero, zero);
        let bar = menu * (1.0 - t);
        let glass = bar * look.glass;
        let clear = bar * (1.0 - look.glass);
        let dark = smooth((look.luma - 0.32) / 0.22) * (1.0 - look.glass);
        let stop = |at: f32, c: kit::palette::Rgb| gtk::gsk::ColorStop::new(at, c.rgba());
        let shade = kit::palette::Rgb::new(0.0, 0.0, 0.0).mix(kit::palette::Rgb::new(1.0, 1.0, 1.0), 1.0 - dark);
        if clear > 0.002 {
            let reach = band + 14.0;
            s.append_linear_gradient(
                &graphene::Rect::new(0.0, 0.0, w as f32, reach as f32),
                &graphene::Point::new(0.0, 0.0),
                &graphene::Point::new(0.0, reach as f32),
                &[
                    stop(0.0, shade.alpha((0.30 - 0.12 * dark) * clear)),
                    stop(0.45, shade.alpha((0.17 - 0.07 * dark) * clear)),
                    stop(0.8, shade.alpha((0.05 - 0.02 * dark) * clear)),
                    stop(1.0, shade.alpha(0.0)),
                ],
            );
        }
        if glass > 0.002 {
            let tint = pal.body.alpha((if look.blur { 0.46 } else { 0.86 }) * glass);
            s.append_color(&tint.rgba(), &graphene::Rect::new(0.0, 0.0, w as f32, band as f32));
            s.append_linear_gradient(
                &graphene::Rect::new(0.0, 0.0, w as f32, band as f32),
                &graphene::Point::new(0.0, 0.0),
                &graphene::Point::new(0.0, band as f32),
                &[stop(0.0, kit::palette::Rgb::new(1.0, 1.0, 1.0).alpha(0.07 * glass)), stop(1.0, kit::palette::Rgb::new(1.0, 1.0, 1.0).alpha(0.0))],
            );
            s.append_color(&kit::palette::Rgb::new(1.0, 1.0, 1.0).alpha(0.075 * glass).rgba(), &graphene::Rect::new(0.0, (band - 1.0) as f32, w as f32, 1.0));
            s.append_linear_gradient(
                &graphene::Rect::new(0.0, band as f32, w as f32, 10.0),
                &graphene::Point::new(0.0, band as f32),
                &graphene::Point::new(0.0, (band + 10.0) as f32),
                &[stop(0.0, kit::palette::Rgb::new(0.0, 0.0, 0.0).alpha(0.16 * glass)), stop(1.0, kit::palette::Rgb::new(0.0, 0.0, 0.0).alpha(0.0))],
            );
        }
        let body = if t > 0.002 {
            let bw = slab + (w - slab) * t;
            let r = (radius * (1.0 - t)).max(0.0) as f32;
            let body = gtk::gsk::RoundedRect::new(graphene::Rect::new(((w - bw) / 2.0) as f32, 0.0, bw as f32, h as f32), zero, zero, graphene::Size::new(r, r), graphene::Size::new(r, r));
            s.push_rounded_clip(&body);
            s.append_color(&pal.body.rgba(), body.bounds());
            s.pop();
            if menu > 0.5 { full.clone() } else { body }
        } else {
            full.clone()
        };
        // The middle kept free for the notch shrinks with the screen, so narrow or scaled-up
        // screens still have room for the app menus.
        let edge = ((w - imp.keep.get().min(w * 0.36)) / 2.0).floor() - 10.0;
        let show = motion_in(((t - 0.3) / 0.55).clamp(0.0, 1.0)).max(menu);
        if edge < 80.0 || show <= 0.01 {
            imp.zones.borrow_mut().clear();
            return;
        }
        let white = kit::palette::Rgb::new(1.0, 1.0, 1.0);
        let night = kit::palette::Rgb::new(0.07, 0.07, 0.09);
        let top = white.mix(night, 1.0 - dark);
        let ink_main = top.mix(pal.ink_main, 1.0 - t);
        let ink_meta = top.alpha(0.80).mix(pal.ink_meta.mix(pal.ink_main, 0.4), 1.0 - t);
        s.push_rounded_clip(&body);
        s.push_opacity(show);
        let halo = if kit::effects::reduced() { 0.0 } else { clear * (1.0 - t) };
        if halo > 0.02 {
            let c = if dark > 0.5 { white.alpha(0.30 * halo) } else { kit::palette::Rgb::new(0.0, 0.0, 0.0).alpha(0.42 * halo) };
            s.push_shadow(&[gtk::gsk::Shadow::new(c.rgba(), 0.0, 0.5, 5.0)]);
        }
        let cfg = &app.cfg;
        let hot = imp.hot.get();
        let held = imp.held.get();
        let pill = |s: &gtk::Snapshot, zone: Zone, x: f64, width: f64| {
            if held == Some(zone) {
                draw::fill(s, &draw::round(x, mid - 14.0, width, 28.0, 14.0), ink_main.alpha(0.22));
            } else if hot == Some(zone) {
                draw::fill(s, &draw::round(x, mid - 14.0, width, 28.0, 14.0), ink_main.alpha(0.14));
            }
        };
        let mut zones = Vec::with_capacity(4);
        let scale = self.scale_factor().max(1);
        let logo_px = 20;
        if imp.logo.borrow().as_ref().is_none_or(|(k, _)| *k != scale) {
            *imp.logo.borrow_mut() = Some((scale, kit::distro::logo(logo_px, scale, true)));
        }
        let mut x = 10.0;
        pill(s, Zone::Logo, x, 36.0);
        let logo = imp.logo.borrow().as_ref().and_then(|(_, l)| l.clone());
        let digit = crate::menubar::logo(s, &app, x + 18.0 - logo_px as f64 / 2.0, mid - logo_px as f64 / 2.0, logo_px as f64);
        if !digit {
            match logo {
                Some(l) => {
                    s.save();
                    s.translate(&graphene::Point::new((x + 18.0 - logo_px as f64 / 2.0) as f32, (mid - logo_px as f64 / 2.0) as f32));
                    l.snapshot_symbolic(s, logo_px as f64, logo_px as f64, &[ink_main.rgba()]);
                    s.restore();
                }
                None => draw::icon(s, self, cfg, "deployed_code", 20.0, true, ink_main, x + 18.0, mid),
            }
        }
        zones.push((Zone::Logo, (x, 0.0, 36.0, band)));
        x += 42.0;
        // The workspace pill, its own pill beside the logo: it grows open from the left.
        let (open, row) = {
            let sp = imp.spaces.borrow();
            (sp.open.value(now).clamp(0.0, 1.0), sp.width())
        };
        if open > 0.002 && row > 0.0 {
            let pw = (row * open).max(1.0);
            let shape = draw::round(x, mid - 14.0, pw, 28.0, 14.0_f64.min(pw / 2.0));
            pill(s, Zone::Spaces, x, pw);
            s.push_rounded_clip(&shape);
            s.push_opacity(open);
            self.draw_spaces(s, &imp.spaces.borrow(), x, mid, now, ink_main, pal.accent);
            s.pop();
            s.pop();
            if open > 0.5 {
                zones.push((Zone::Spaces, (x, 0.0, pw, band)));
            }
            x += pw + 6.0 * open;
        }
        let focused = {
            let list = app.running.borrow();
            list.iter().find(|r| r.focused).or_else(|| list.iter().max_by_key(|r| r.stamp)).map(|r| r.app_id.clone())
        };
        if let Some(id) = focused.filter(|i| !i.is_empty() && app.app_menus()) {
            if imp.icon.borrow().as_ref().is_none_or(|(k, _)| *k != id) {
                *imp.icon.borrow_mut() = Some((id.clone(), draw::app_icon(&app.apps, &id, 18, scale)));
            }
            let name = app.apps.display_name(&id);
            let text_w = draw::width(self, cfg, &name, TEXT, Face::Text(650)).ceil().min(edge - x - 140.0).max(0.0);
            if text_w > 8.0 {
                let width = 12.0 + 20.0 + 8.0 + text_w + 12.0;
                pill(s, Zone::App, x, width);
                if let Some(p) = imp.icon.borrow().as_ref().and_then(|(_, p)| p.clone()) {
                    draw::paintable(s, &p, x + 12.0, mid - 10.0, 20.0);
                }
                draw::text(s, self, cfg, &name, TEXT, Face::Text(650), ink_main, x + 40.0, mid, text_w + 1.0, pango::Alignment::Left);
                zones.push((Zone::App, (x, 0.0, width, band)));
                x += width + 2.0;
                let ww = draw::width(self, cfg, "Window", TEXT, Face::Text(450)).ceil() + 24.0;
                if imp.titles.borrow().0 != id {
                    *imp.titles.borrow_mut() = (id.clone(), crate::appmenu::titles(&id));
                }
                let titles = imp.titles.borrow().1.clone();
                for (i, title) in titles.iter().enumerate() {
                    let tw = draw::width(self, cfg, title, TEXT, Face::Text(450)).ceil() + 24.0;
                    if x + tw + ww >= edge {
                        break;
                    }
                    let zone = Zone::Menu(i as u8);
                    pill(s, zone, x, tw);
                    draw::text(s, self, cfg, title, TEXT, Face::Text(450), ink_main, x + 12.0, mid, tw - 23.0, pango::Alignment::Left);
                    zones.push((zone, (x, 0.0, tw, band)));
                    x += tw;
                }
                if x + ww < edge {
                    pill(s, Zone::Win, x, ww);
                    draw::text(s, self, cfg, "Window", TEXT, Face::Text(450), ink_main, x + 12.0, mid, ww - 23.0, pango::Alignment::Left);
                    zones.push((Zone::Win, (x, 0.0, ww, band)));
                }
            }
        }
        let date = app.date.borrow().clone();
        let date_w = draw::width(self, cfg, &date, TEXT, Face::Text(500)).ceil();
        let mut r = w - 10.0;
        if !date.is_empty() {
            let width = date_w + 24.0;
            r -= width;
            pill(s, Zone::Date, r, width);
            draw::text(s, self, cfg, &date, TEXT, Face::Text(500), ink_meta, r + 12.0, mid, date_w + 1.0, pango::Alignment::Left);
            zones.push((Zone::Date, (r, 0.0, width, band)));
            r -= 4.0;
        }
        let fresh = imp.battery_at.get().is_none_or(|t| now - t > 30_000_000);
        if fresh {
            imp.battery_at.set(Some(now));
            imp.battery.set(battery());
        }
        let mut glyphs: Vec<(&str, Option<String>)> = Vec::with_capacity(3);
        if app.dnd.get() {
            glyphs.push(("do_not_disturb_on", None));
        }
        if let Some((level, muted)) = app.volume.get() {
            let g = if muted || level <= 0.001 { "volume_off" } else if level < 0.5 { "volume_down" } else { "volume_up" };
            glyphs.push((g, None));
        }
        if let Some(g) = crate::devices::summary(&app.gadgets.borrow()) {
            glyphs.push((g.glyph, Some(format!("{}%", g.level))));
        }
        if let Some(b) = imp.battery.get() {
            glyphs.push((battery_glyph(b), Some(format!("{}%", b.level))));
        }
        glyphs.push(("toggle_on", None));
        let widths: Vec<f64> = glyphs.iter().map(|(_, t)| 22.0 + t.as_ref().map_or(0.0, |t| 2.0 + draw::width(self, cfg, t, NUM, Face::Number(500)).ceil())).collect();
        let total = widths.iter().sum::<f64>() + 6.0 * (glyphs.len().saturating_sub(1)) as f64 + 20.0;
        if r - total >= w - edge {
            r -= total;
            pill(s, Zone::Status, r, total);
            let mut gx = r + 10.0;
            for ((g, t), gw) in glyphs.iter().zip(widths.iter()) {
                draw::icon(s, self, cfg, g, ICON, true, ink_main, gx + 11.0, mid);
                if let Some(t) = t {
                    draw::text(s, self, cfg, t, NUM, Face::Number(500), ink_meta, gx + 24.0, mid, gw - 23.0, pango::Alignment::Left);
                }
                gx += gw + 6.0;
            }
            zones.push((Zone::Status, (r, 0.0, total, band)));
            let tray: Vec<(Option<gtk::gdk::Paintable>, String)> = app.sni.borrow().iter().map(|i| (i.icon.clone(), if i.id.is_empty() { i.title.clone() } else { i.id.clone() })).collect();
            r -= 4.0;
            for (i, (icon, name)) in tray.iter().enumerate().rev() {
                if r - 30.0 < w - edge {
                    break;
                }
                r -= 30.0;
                let zone = Zone::Tray(i as u8);
                pill(s, zone, r, 30.0);
                match icon {
                    Some(p) => draw::paintable(s, p, r + 6.0, mid - 9.0, 18.0),
                    None => draw::monogram(s, self, cfg, name, r + 6.0, mid - 9.0, 18.0, ink_main.alpha(0.2), ink_main),
                }
                zones.push((zone, (r, 0.0, 30.0, band)));
            }
        }
        if halo > 0.02 {
            s.pop();
        }
        s.pop();
        s.pop();
        if show > 0.5 {
            *imp.zones.borrow_mut() = zones;
        } else {
            imp.zones.borrow_mut().clear();
        }
    }
}

mod imp {
    use gtk::subclass::prelude::*;
    use gtk::{gdk, glib};
    use std::cell::{Cell, RefCell};
    use std::rc::Weak;

    #[derive(Default)]
    pub struct Strip {
        pub(super) app: RefCell<Weak<crate::app::App>>,
        pub(super) header: Cell<f64>,
        pub(super) keep: Cell<f64>,
        #[allow(clippy::type_complexity)]
        pub(super) follow: RefCell<Option<Box<dyn Fn(i64) -> Option<super::Look>>>>,
        pub(super) lift: Cell<f64>,
        pub(super) zones: RefCell<Vec<(super::Zone, (f64, f64, f64, f64))>>,
        pub(super) hot: Cell<Option<super::Zone>>,
        pub(super) held: Cell<Option<super::Zone>>,
        pub(super) titles: RefCell<(String, Vec<&'static str>)>,
        pub(super) logo: RefCell<Option<(i32, Option<gtk::IconPaintable>)>>,
        pub(super) icon: RefCell<Option<(String, Option<gdk::Paintable>)>>,
        pub(super) battery: Cell<Option<super::Battery>>,
        pub(super) battery_at: Cell<Option<i64>>,
        pub(super) spaces: RefCell<super::Spaces>,
        pub(super) pointer: Cell<f64>,
        #[allow(clippy::type_complexity)]
        pub(super) resize: RefCell<Option<Box<dyn Fn(i32)>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Strip {
        const NAME: &'static str = "DigitStrip";
        type Type = super::Strip;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Strip {}

    impl WidgetImpl for Strip {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            self.obj().draw(snapshot);
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            if let Some(f) = self.resize.borrow().as_ref() {
                f(width);
            }
        }
    }
}

glib::wrapper! {
    pub struct Strip(ObjectSubclass<imp::Strip>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}
