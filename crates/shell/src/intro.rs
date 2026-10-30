use crate::app::App;
use crate::draw::{self, Face};
use crate::paint::Paint;
use gtk::{cairo, gdk, glib, graphene, gsk, pango, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kit::motion::{self, Curve};
use kit::palette::Rgb;
use std::rc::Rc;

const GROW_MS: f64 = 520.0;
const REVEAL_AT: f64 = 950.0;
const REVEAL_MS: f64 = 640.0;
const PILL_W: f64 = 188.0;
const PILL_H: f64 = 46.0;
const NOTCH_W: f64 = 128.0;
const BLACK: Rgb = Rgb::new(0.0, 0.0, 0.0);
const LIGHT: Rgb = Rgb::new(0.96, 0.96, 0.97);
const SETTLE: Curve = Curve::new(0.16, 1.0, 0.3, 1.0);

/// Covers every screen when the shell starts: a notch grows in the middle and says hello,
/// then flies up into its place while the desktop opens out from behind it.
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
        if ((clock.frame_time() - start) as f64 / 1000.0) < REVEAL_AT + REVEAL_MS + 40.0 {
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

fn frame(app: &App, paint: &Paint, s: &gtk::Snapshot, start: i64, greeting: &str) {
    let now = paint.frame_clock().map(|c| c.frame_time()).unwrap_or_else(glib::monotonic_time);
    let t = (now - start) as f64 / 1000.0;
    let (w, h) = (paint.width() as f64, paint.height() as f64);
    if w < 1.0 || h < 1.0 {
        return;
    }
    let pal = app.palette_at(now);
    let cfg = &app.cfg;
    let grow = ease(SETTLE, t / GROW_MS);
    let open = ease(motion::EMPHASIZED_DECEL, (t - REVEAL_AT) / REVEAL_MS);
    let fly = ease(Curve::new(0.5, 0.0, 0.0, 1.0), (t - REVEAL_AT) / (REVEAL_MS * 0.8));
    let (cx, from_y, to_y) = (w / 2.0, h / 2.0 - 14.0, app.header() / 2.0);
    let cy = from_y + (to_y - from_y) * fly;

    // The curtain, with a hole that widens from the notch as it lifts.
    if open < 1.0 {
        let full = graphene::Rect::new(0.0, 0.0, w as f32, h as f32);
        let radius = open * w.hypot(h);
        s.push_mask(gsk::MaskMode::InvertedAlpha);
        s.append_color(&pal.body.alpha(1.0).rgba(), &full);
        s.pop();
        if radius > 0.5 {
            draw::fill(s, &draw::round(cx - radius, cy - radius, 2.0 * radius, 2.0 * radius, radius), BLACK);
        }
        s.pop();
    }

    // The notch: grows into a pill, then shrinks to notch size on its way up and melts into the real one.
    let pw = PILL_W * grow + (NOTCH_W - PILL_W) * fly;
    let ph = PILL_H * grow.min(1.0) + (app.header() - PILL_H) * fly;
    let gone = ease(motion::STANDARD, (t - REVEAL_AT - REVEAL_MS * 0.45) / (REVEAL_MS * 0.55));
    if pw > 1.0 && gone < 1.0 {
        s.push_opacity(1.0 - gone);
        let glow = (1.0 - ease(motion::STANDARD, (t - GROW_MS) / 600.0)) * grow;
        if glow > 0.01 {
            let g = 10.0 * glow;
            draw::fill(s, &draw::round(cx - pw / 2.0 - g, cy - ph / 2.0 - g, pw + 2.0 * g, ph + 2.0 * g, ph / 2.0 + g), pal.accent.alpha(0.18 * glow));
        }
        draw::fill(s, &draw::round(cx - pw / 2.0, cy - ph / 2.0, pw, ph, ph / 2.0), BLACK);
        let word = ease(SETTLE, (t - 160.0) / 420.0) * (1.0 - (fly * 2.5).min(1.0));
        if word > 0.0 {
            let span = 120.0;
            s.push_clip(&graphene::Rect::new((cx - span / 2.0) as f32, (cy - ph / 2.0) as f32, (span * word) as f32, ph as f32));
            draw::text(s, paint, cfg, "digit", 19.0, Face::Text(600), LIGHT.alpha(word), cx - span / 2.0, cy + 3.0 * (1.0 - word), span, pango::Alignment::Center);
            s.pop();
        }
        s.pop();
    }

    // A quiet hello under the notch, gone as soon as the curtain moves.
    let hello = ease(SETTLE, (t - 320.0) / 380.0) * (1.0 - (open * 4.0).min(1.0));
    if hello > 0.0 {
        draw::text(s, paint, cfg, greeting, 13.0, Face::Text(450), pal.ink_meta.alpha(hello), cx - 160.0, from_y + PILL_H / 2.0 + 26.0 + 6.0 * (1.0 - hello), 320.0, pango::Alignment::Center);
    }
}
