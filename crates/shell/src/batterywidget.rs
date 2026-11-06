//! The batteries widget on the desktop: Control Center's rings, straight on the wallpaper.

use crate::app::App;
use crate::paint::Paint;
use gtk::{cairo, gdk, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kit::palette::Rgb;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

pub(crate) const SHOWN: usize = 4;
const CELL_W: f64 = 92.0;
const CARD_H: f64 = 120.0;
const INSET: f64 = 12.0;
// The gap to the screen's edges, the same on every side.
const AIR: f64 = 28.0;
const SURFACE_W: f64 = SHOWN as f64 * CELL_W + 2.0 * INSET + 2.0 * AIR;
const SURFACE_H: f64 = CARD_H + 2.0 * AIR;
// Clears the desktop clock when both sit top left.
const CLOCK_ROOM: i32 = 214;

/// The card face (excluding its shadow), in logical monitor coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CardRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

fn card_width(count: usize) -> f64 {
    count.min(SHOWN) as f64 * CELL_W + 2.0 * INSET
}

fn card_x(left: bool, width: f64, card_width: f64) -> f64 {
    if left { AIR } else { width - AIR - card_width }
}

fn surface_margin(top: bool, left: bool, top_margin: i32, clock: &str) -> i32 {
    if top {
        top_margin
            + if left && clock == "left" {
                CLOCK_ROOM
            } else {
                0
            }
    } else {
        0
    }
}

pub(crate) fn card_rect(
    placement: &str,
    device_count: usize,
    screen: (i32, i32),
    top_margin: i32,
    clock: &str,
) -> Option<CardRect> {
    if matches!(placement, "" | "off") || device_count == 0 {
        return None;
    }
    let (top, left) = (placement.starts_with("top"), placement.ends_with("left"));
    let margin = surface_margin(top, left, top_margin, clock) as f64;
    let w = card_width(device_count);
    Some(CardRect {
        x: card_x(left, screen.0 as f64, w),
        y: if top {
            margin + AIR
        } else {
            screen.1 as f64 - margin - AIR - CARD_H
        },
        w,
        h: CARD_H,
    })
}

thread_local! {
    static WIDGET: RefCell<Option<(gtk::Window, Paint)>> = const { RefCell::new(None) };
    static SHOWN_AT: RefCell<(String, String)> = const { RefCell::new((String::new(), String::new())) };
    static TIMER: RefCell<Option<glib::SourceId>> = const { RefCell::new(None) };
}

/// A device's level changed.
pub fn changed() {
    WIDGET.with(|w| {
        if let Some((_, paint)) = w.borrow().as_ref() {
            paint.queue_draw();
        }
    });
}

fn clear() {
    if let Some((window, _)) = WIDGET.with(|w| w.borrow_mut().take()) {
        window.destroy();
    }
    if let Some(id) = TIMER.with(|t| t.borrow_mut().take()) {
        id.remove();
    }
}

/// The primary display, or the first one.
fn monitor() -> Option<gdk::Monitor> {
    let primary = kit::prefs::text(&kit::prefs::read(), "primary_monitor", "");
    let all: Vec<gdk::Monitor> = gdk::Display::default()?.monitors().iter::<gdk::Monitor>().filter_map(Result::ok).collect();
    all.iter().find(|m| m.connector().is_some_and(|c| c == primary)).or(all.first()).cloned()
}

pub fn apply(app: &Rc<App>) {
    let (place, clock) = {
        let prefs = app.prefs.borrow();
        (prefs.desktop_battery.clone(), prefs.desktop_widget.clone())
    };
    let target = monitor();
    let output = target.as_ref().and_then(|m| m.connector()).map(|c| c.to_string()).unwrap_or_default();
    let key = (format!("{place} {clock}"), output.clone());
    if SHOWN_AT.with(|s| *s.borrow() == key) && WIDGET.with(|w| w.borrow().is_some()) {
        return;
    }
    clear();
    SHOWN_AT.with(|s| *s.borrow_mut() = key);
    let Some(m) = target.filter(|_| !matches!(place.as_str(), "" | "off") && app.cfg.wallpaper) else { return };
    let (top, left) = (place.starts_with("top"), place.ends_with("left"));
    let window = gtk::Window::new();
    window.init_layer_shell();
    window.set_layer(Layer::Bottom);
    window.set_namespace(Some("digit-desktop"));
    window.set_keyboard_mode(KeyboardMode::None);
    window.set_exclusive_zone(-1);
    window.set_monitor(Some(&m));
    window.add_css_class("digit");
    window.set_anchor(if top { Edge::Top } else { Edge::Bottom }, true);
    window.set_anchor(if left { Edge::Left } else { Edge::Right }, true);
    window.set_margin(
        if top { Edge::Top } else { Edge::Bottom },
        surface_margin(top, left, app.reserve(), &clock),
    );
    let paint = Paint::new(SURFACE_W as i32, SURFACE_H as i32);
    let weak = paint.downgrade();
    let a = Rc::downgrade(app);
    paint.on_draw(move |s| {
        let (Some(p), Some(app)) = (weak.upgrade(), a.upgrade()) else { return };
        draw(s, &p, &app, &output, left, top);
    });
    window.set_child(Some(&paint));
    window.connect_realize(|w| {
        if let Some(surface) = w.surface() {
            surface.set_input_region(Some(&cairo::Region::create()));
        }
    });
    window.present();
    WIDGET.with(|w| *w.borrow_mut() = Some((window, paint)));
    // The phone's level and the palette have no signal of their own here.
    let id = glib::timeout_add_local(Duration::from_secs(30), || {
        changed();
        glib::ControlFlow::Continue
    });
    TIMER.with(|t| *t.borrow_mut() = Some(id));
}

fn draw(s: &gtk::Snapshot, p: &Paint, app: &App, output: &str, left: bool, top: bool) {
    let mut list = crate::batteries::list(app);
    if list.is_empty() {
        return;
    }
    // This computer's own battery first, like the Mac's widget.
    list.sort_by_key(|g| !g.own);
    list.truncate(SHOWN);
    // No background: like the desktop clock, the ink follows the wallpaper behind it.
    let luma = app.wallpapers.top_luma(output, if top { 0.32 } else { 1.0 }).unwrap_or(0.0);
    let dark = luma > 0.42;
    let ink = if dark { Rgb::new(0.08, 0.08, 0.1) } else { Rgb::new(1.0, 1.0, 1.0) };
    let shade = if dark { gdk::RGBA::new(1.0, 1.0, 1.0, 0.28) } else { gdk::RGBA::new(0.0, 0.0, 0.0, 0.32) };
    let mut pal = app.palette_at(glib::monotonic_time());
    pal.ink_main = ink;
    pal.ink_meta = ink;
    let x = card_x(left, SURFACE_W, card_width(list.len()));
    s.push_shadow(&[gtk::gsk::Shadow::new(shade, 0.0, 1.0, 10.0)]);
    for (i, g) in list.iter().enumerate() {
        let cx = x + INSET + CELL_W * (i as f64 + 0.5);
        crate::batteries::cell(s, p, &app.cfg, &pal, g, cx, AIR + 42.0, CELL_W, false);
    }
    s.pop();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_rect_matches_surface_anchors_and_device_counts() {
        for count in 1..=SHOWN {
            let w = count as f64 * CELL_W + 2.0 * INSET;
            for (placement, x, y) in [
                ("top_left", AIR, 26.0 + AIR),
                ("top_right", 800.0 - AIR - w, 26.0 + AIR),
                ("bottom_left", AIR, 600.0 - AIR - CARD_H),
                (
                    "bottom_right",
                    800.0 - AIR - w,
                    600.0 - AIR - CARD_H,
                ),
            ] {
                assert_eq!(
                    card_rect(placement, count, (800, 600), 26, "off"),
                    Some(CardRect { x, y, w, h: CARD_H })
                );
            }
        }
        let normal = card_rect("top_left", SHOWN, (800, 600), 26, "off").unwrap();
        let clock = card_rect("top_left", SHOWN, (800, 600), 26, "left").unwrap();
        assert_eq!(clock.y, normal.y + CLOCK_ROOM as f64);
        assert_eq!(
            card_rect("top_left", 99, (800, 600), 26, "off"),
            Some(normal)
        );
        assert!(card_rect("off", SHOWN, (800, 600), 26, "off").is_none());
        assert!(card_rect("top_right", 0, (800, 600), 26, "off").is_none());
    }
}
