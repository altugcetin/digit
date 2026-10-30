use crate::app::App;
use crate::draw::{self, Face};
use crate::paint::Paint;
use gtk::{cairo, gdk, glib, graphene, gsk, pango, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kit::motion::{self, Curve};
use std::rc::Rc;

/// The wordmark is drawn with dotless i's; the accent dots above them are the mark itself.
const WORD: &str = "dıgıt";
const PX: f64 = 76.0;
const SPRING: Curve = Curve::new(0.34, 1.56, 0.64, 1.0);
const SETTLE: Curve = Curve::new(0.16, 1.0, 0.3, 1.0);
const GLIDE: Curve = Curve::new(0.65, 0.0, 0.35, 1.0);

// Timeline, in milliseconds from the start.
const DOT_IN: f64 = 120.0;
const SPLIT: f64 = 620.0;
const LETTERS: f64 = 820.0;
const STAGGER: f64 = 75.0;
const HOP: f64 = 1750.0;
const OUT: f64 = 2400.0;
const OUT_MS: f64 = 700.0;
const FADE: f64 = 2700.0;
const FADE_MS: f64 = 1500.0;
const END: f64 = FADE + FADE_MS + 40.0;
/// Slow in, slow out: the backdrop melts away instead of being cut.
const MELT: Curve = Curve::new(0.45, 0.0, 0.55, 1.0);

/// Covers every screen when the shell starts: a dot springs up, splits into the dots of the
/// wordmark as its letters rise into place, then the curtain opens from the middle.
pub fn play(app: &Rc<App>) {
    if !app.cfg.intro {
        return;
    }
    let Some(display) = gdk::Display::default() else { return };
    let start = glib::monotonic_time();
    for monitor in display.monitors().iter::<gdk::Monitor>().filter_map(Result::ok) {
        cover(app, &monitor, start);
    }
}

fn cover(app: &Rc<App>, monitor: &gdk::Monitor, start: i64) {
    let window = gtk::Window::new();
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace(Some("digit-intro"));
    for e in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(e, true);
    }
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(KeyboardMode::None);
    window.set_monitor(Some(monitor));
    window.add_css_class("digit");
    let paint = Paint::new(-1, -1);
    window.set_child(Some(&paint));
    let greeting = greeting(&app.clock.borrow());
    let (a, p) = (app.clone(), paint.clone());
    paint.on_draw(move |s| frame(&a, &p, s, start, &greeting));
    window.present();
    if let Some(surface) = window.surface() {
        surface.set_input_region(Some(&cairo::Region::create()));
    }
    let win = window.clone();
    paint.add_tick_callback(move |w, clock| {
        w.queue_draw();
        if ((clock.frame_time() - start) as f64 / 1000.0) < END {
            return glib::ControlFlow::Continue;
        }
        let win = win.clone();
        glib::idle_add_local_once(move || {
            draw::forget(&win);
            win.destroy();
            App::trim();
        });
        glib::ControlFlow::Break
    });
}

fn greeting(clock: &str) -> String {
    let hour = glib::DateTime::now_local().map(|d| d.hour()).unwrap_or(12);
    let part = match hour {
        5..=11 => "Good morning",
        12..=17 => "Good afternoon",
        18..=21 => "Good evening",
        _ => "Good night",
    };
    if clock.is_empty() { part.to_string() } else { format!("{part} · {clock}") }
}

fn ease(c: Curve, t: f64) -> f64 {
    c.at(t.clamp(0.0, 1.0))
}

/// 0 → 1 over `ms` starting at `from`.
fn span(t: f64, from: f64, ms: f64) -> f64 {
    ((t - from) / ms).clamp(0.0, 1.0)
}

fn dot(s: &gtk::Snapshot, x: f64, y: f64, r: f64, color: kit::palette::Rgb) {
    if r > 0.1 {
        draw::fill(s, &draw::round(x - r, y - r, 2.0 * r, 2.0 * r, r), color);
    }
}

fn frame(app: &App, paint: &Paint, s: &gtk::Snapshot, start: i64, greeting: &str) {
    let now = paint.frame_clock().map(|c| c.frame_time()).unwrap_or_else(glib::monotonic_time);
    let t = (now - start) as f64 / 1000.0;
    let (w, h) = (paint.width() as f64, paint.height() as f64);
    if w < 1.0 || h < 1.0 {
        return;
    }
    let pal = app.palette_at(now);
    let cfg = &app.cfg;
    let (cx, cy) = (w / 2.0, h / 2.0 - 10.0);

    // The backdrop: the shell's own surface, solid until the mark has gone, then a long fade.
    let melt = ease(MELT, span(t, FADE, FADE_MS));
    if melt >= 1.0 {
        return;
    }
    let full = graphene::Rect::new(0.0, 0.0, w as f32, h as f32);
    s.append_color(&pal.body.alpha(1.0 - melt).rgba(), &full);
    let glow = ease(motion::STANDARD, span(t, 0.0, 900.0)) * (1.0 - ease(motion::STANDARD, span(t, OUT, OUT_MS + 300.0)));
    let halo = (w.min(h) * 0.55) as f32;
    let stops = [
        gsk::ColorStop::new(0.0, pal.accent.alpha(0.13 * glow).rgba()),
        gsk::ColorStop::new(0.45, pal.accent.alpha(0.04 * glow).rgba()),
        gsk::ColorStop::new(1.0, pal.accent.alpha(0.0).rgba()),
    ];
    s.append_radial_gradient(&full, &graphene::Point::new(cx as f32, cy as f32), halo, halo, 0.0, 1.0, &stops);

    // Lay out the wordmark letter by letter so each can move on its own.
    let face = Face::Text(700);
    let letters: Vec<pango::Layout> = WORD.chars().map(|c| draw::shaped(paint, cfg, &c.to_string(), PX, face)).collect();
    let advance: Vec<f64> = letters.iter().map(|l| l.extents().1.width() as f64 / pango::SCALE as f64).collect();
    let height = letters.first().map_or(PX, |l| l.extents().1.height() as f64 / pango::SCALE as f64);
    let gather = ease(SETTLE, span(t, LETTERS, 900.0));
    let leave = ease(Curve::new(0.4, 0.0, 0.2, 1.0), span(t, OUT, OUT_MS));
    let track = 22.0 * (1.0 - gather) + 10.0 * leave;
    let total: f64 = advance.iter().sum::<f64>() + track * (advance.len() as f64 - 1.0);
    let top = cy - height / 2.0;
    let mut xs = Vec::with_capacity(letters.len());
    let mut x = cx - total / 2.0;
    for a in &advance {
        xs.push(x);
        x += a + track;
    }

    // Leaving: the mark drifts up, eases back and dissolves before the backdrop goes.
    let size = 1.0 - 0.06 * leave;
    s.save();
    s.translate(&graphene::Point::new(cx as f32, (cy - 16.0 * leave) as f32));
    s.scale(size as f32, size as f32);
    s.translate(&graphene::Point::new(-cx as f32, -cy as f32));
    s.push_opacity(1.0 - leave);

    for (i, l) in letters.iter().enumerate() {
        let p = span(t, LETTERS + i as f64 * STAGGER, 560.0);
        let k = ease(SETTLE, p);
        if k <= 0.0 {
            continue;
        }
        s.save();
        s.translate(&graphene::Point::new(xs[i] as f32, (top + 26.0 * (1.0 - k)) as f32));
        s.append_layout(l, &pal.ink_main.alpha(ease(motion::STANDARD, p)).rgba());
        s.restore();
    }

    // The mark: one dot springs up in the middle, then splits into the dots of the two i's.
    let r = PX * 0.085;
    let targets: Vec<(f64, f64)> = WORD
        .chars()
        .enumerate()
        .filter(|(_, c)| *c == 'ı')
        .map(|(i, _)| {
            let (ink, _) = letters[i].extents();
            let ix = xs[i] + (ink.x() as f64 + ink.width() as f64 / 2.0) / pango::SCALE as f64;
            let iy = top + ink.y() as f64 / pango::SCALE as f64 - PX * 0.11 - r;
            (ix, iy)
        })
        .collect();
    let born = ease(SPRING, span(t, DOT_IN, 520.0));
    let split = ease(GLIDE, span(t, SPLIT, 560.0));
    for ring in 0..2 {
        let p = span(t, DOT_IN + 140.0 + ring as f64 * 160.0, 900.0);
        if p > 0.0 && p < 1.0 {
            let rr = r * (1.5 + 10.0 * ease(motion::EMPHASIZED_DECEL, p));
            draw::fill(s, &draw::round(cx - rr, cy - rr, 2.0 * rr, 2.0 * rr, rr), pal.accent.alpha(0.16 * (1.0 - p)));
        }
    }
    for (k, (tx, ty)) in targets.iter().enumerate() {
        let hop = span(t, HOP + k as f64 * 110.0, 380.0);
        let lift = 12.0 * (std::f64::consts::PI * hop).sin();
        let (x, y) = (cx + (tx - cx) * split, cy + (ty - cy) * split - lift);
        let size = r * born * (1.35 - 0.35 * split);
        dot(s, x, y, size, pal.accent);
    }

    let hello = ease(SETTLE, span(t, 1450.0, 500.0));
    if hello > 0.0 {
        draw::text(s, paint, cfg, greeting, 15.0, Face::Text(450), pal.ink_meta.alpha(hello), cx - 200.0, cy + height / 2.0 + 34.0 + 8.0 * (1.0 - hello), 400.0, pango::Alignment::Center);
    }
    s.pop();
    s.restore();
}
