//! Where a dragged window would land: a soft accent area, drawn under no pointer and no keys.

use crate::app::App;
use crate::draw;
use crate::paint::Paint;
use gtk::{cairo, gdk, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kit::motion::{self, Tween};
use std::cell::RefCell;
use std::rc::Rc;

const RADIUS: f64 = 18.0;
const GLIDE_MS: i64 = 140;
const FADE_IN_MS: i64 = 160;
const FADE_OUT_MS: i64 = 180;

struct Zone {
    window: gtk::Window,
    paint: Paint,
    output: String,
    rect: [Tween; 4],
    fade: Tween,
}

thread_local! {
    static ZONE: RefCell<Option<Rc<RefCell<Zone>>>> = const { RefCell::new(None) };
}

/// Shows or glides the area; the rectangle is in the monitor's logical pixels.
pub fn show(app: &Rc<App>, output: &str, rect: [f64; 4]) {
    let now = glib::monotonic_time();
    let current = ZONE.with(|z| z.borrow().clone());
    if let Some(zone) = current.filter(|z| z.borrow().output == output) {
        let mut z = zone.borrow_mut();
        let appearing = z.fade.target() < 0.5;
        for (tween, to) in z.rect.iter_mut().zip(rect) {
            if appearing {
                tween.set(to);
            } else {
                tween.go(to, now, GLIDE_MS, motion::STANDARD);
            }
        }
        z.fade.go(1.0, now, FADE_IN_MS, motion::STANDARD);
        let paint = z.paint.clone();
        drop(z);
        animate(&zone, &paint);
        return;
    }
    remove();
    let Some(monitor) = monitor(output) else { return };
    let window = gtk::Window::new();
    window.init_layer_shell();
    window.set_layer(Layer::Top);
    window.set_namespace(Some("digit-snap-zone"));
    window.set_keyboard_mode(KeyboardMode::None);
    window.set_exclusive_zone(-1);
    window.set_monitor(Some(&monitor));
    window.add_css_class("digit");
    for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(edge, true);
    }
    let paint = Paint::new(1, 1);
    window.set_child(Some(&paint));
    window.connect_realize(|w| {
        if let Some(surface) = w.surface() {
            surface.set_input_region(Some(&cairo::Region::create()));
        }
    });
    let mut fade = Tween::still(0.0);
    fade.go(1.0, now, FADE_IN_MS, motion::STANDARD);
    let zone = Rc::new(RefCell::new(Zone { window: window.clone(), paint: paint.clone(), output: output.to_string(), rect: rect.map(Tween::still), fade }));
    let (weak, app) = (Rc::downgrade(&zone), Rc::downgrade(app));
    let clock = paint.downgrade();
    paint.on_draw(move |s| {
        let (Some(zone), Some(app), Some(paint)) = (weak.upgrade(), app.upgrade(), clock.upgrade()) else { return };
        let now = paint.frame_clock().map_or_else(glib::monotonic_time, |c| c.frame_time());
        let z = zone.borrow();
        let [x, y, w, h] = [0, 1, 2, 3].map(|i| z.rect[i].value(now));
        let accent = app.palette_at(now).accent;
        draw::fill(s, &draw::round(x, y, w, h, RADIUS * app.round()), accent.alpha(0.22 * z.fade.value(now)));
    });
    ZONE.with(|z| *z.borrow_mut() = Some(zone.clone()));
    window.present();
    animate(&zone, &paint);
}

/// Fades the area out. Showing it again before that ends turns it back.
pub fn hide() {
    let Some(zone) = ZONE.with(|z| z.borrow().clone()) else { return };
    let paint = {
        let mut z = zone.borrow_mut();
        z.fade.go(0.0, glib::monotonic_time(), FADE_OUT_MS, motion::STANDARD);
        z.paint.clone()
    };
    animate(&zone, &paint);
}

fn animate(zone: &Rc<RefCell<Zone>>, paint: &Paint) {
    let zone = Rc::downgrade(zone);
    paint.add_tick_callback(move |w, clock| {
        w.queue_draw();
        let Some(zone) = zone.upgrade() else { return glib::ControlFlow::Break };
        let now = clock.frame_time();
        let (moving, gone) = {
            let z = zone.borrow();
            (!z.fade.settled(now) || z.rect.iter().any(|r| !r.settled(now)), z.fade.target() < 0.5)
        };
        if moving {
            return glib::ControlFlow::Continue;
        }
        if gone && ZONE.with(|z| z.borrow().as_ref().is_some_and(|c| Rc::ptr_eq(c, &zone))) {
            remove();
        }
        glib::ControlFlow::Break
    });
}

fn remove() {
    let zone = ZONE.with(|z| z.borrow_mut().take());
    if let Some(zone) = zone {
        let window = zone.borrow().window.clone();
        window.destroy();
    }
}

fn monitor(output: &str) -> Option<gdk::Monitor> {
    let display = gdk::Display::default()?;
    display.monitors().iter::<gdk::Monitor>().filter_map(Result::ok).find(|m| m.connector().as_deref() == Some(output))
}
