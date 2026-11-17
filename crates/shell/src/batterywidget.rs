//! The batteries widget on the desktop: Control Center's rings, straight on the wallpaper.

use crate::app::App;
use crate::paint::Paint;
use gtk::{cairo, gdk, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
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

/// The surface's margin from the top or bottom edge: under the bar (and `clock`, the room the
/// clock takes at the top left) at the top, clear of the dock (`dock`) at the bottom.
fn surface_margin(top: bool, left: bool, top_margin: i32, clock: i32, dock: i32) -> i32 {
    if top { top_margin + if left { clock } else { 0 } } else { dock }
}

/// Where the batteries sit on a screen `screen` big. `clock` is the room the desktop clock takes
/// at the top left (see `desk::room`) and `dock` the room the dock keeps on the left, right and
/// bottom (see `dock::room`).
pub(crate) fn card_rect(placement: &str, device_count: usize, screen: (i32, i32), top_margin: i32, clock: i32, dock: [i32; 3]) -> Option<CardRect> {
    if matches!(placement, "" | "off") || device_count == 0 {
        return None;
    }
    let (top, left) = (placement.starts_with("top"), placement.ends_with("left"));
    let margin = surface_margin(top, left, top_margin, clock, dock[2]) as f64;
    let side = if left { dock[0] } else { dock[1] } as f64;
    let w = card_width(device_count);
    Some(CardRect {
        x: if left { side + card_x(true, screen.0 as f64, w) } else { card_x(false, screen.0 as f64 - side, w) },
        y: if top { margin + AIR } else { screen.1 as f64 - margin - AIR - CARD_H },
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
pub(crate) fn monitor() -> Option<gdk::Monitor> {
    let primary = kit::prefs::text(&kit::prefs::read(), "primary_monitor", "");
    let all: Vec<gdk::Monitor> = gdk::Display::default()?.monitors().iter::<gdk::Monitor>().filter_map(Result::ok).collect();
    all.iter().find(|m| m.connector().is_some_and(|c| c == primary)).or(all.first()).cloned()
}

pub fn apply(app: &Rc<App>) {
    let place = app.prefs.borrow().desktop_battery.clone();
    let clock = crate::desk::room(app);
    let target = monitor();
    let output = target.as_ref().and_then(|m| m.connector()).map(|c| c.to_string()).unwrap_or_default();
    let dock = crate::dock::room(&output);
    let key = (format!("{place} {clock} {} {dock:?}", app.reserve()), output.clone());
    if SHOWN_AT.with(|s| *s.borrow() == key) && WIDGET.with(|w| w.borrow().is_some()) {
        // Same place: options (levels, the card) only need a redraw.
        changed();
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
    window.set_margin(if top { Edge::Top } else { Edge::Bottom }, surface_margin(top, left, app.reserve(), clock, dock[2]));
    window.set_margin(if left { Edge::Left } else { Edge::Right }, if left { dock[0] } else { dock[1] });
    let paint = Paint::new(SURFACE_W as i32, SURFACE_H as i32);
    let weak = paint.downgrade();
    let a = Rc::downgrade(app);
    paint.on_draw(move |s| {
        let (Some(p), Some(app)) = (weak.upgrade(), a.upgrade()) else { return };
        crate::draw::rasterized(|| draw(s, &p, &app, &output, left, top));
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
    let (with_card, levels) = {
        let prefs = app.prefs.borrow();
        (prefs.desktop_battery_bg, prefs.desktop_battery_levels)
    };
    // Like the desktop clock, the ink follows the wallpaper behind it, or sits on a card.
    let look = crate::deskcards::look(app, output, top, with_card);
    let mut pal = app.palette_at(glib::monotonic_time());
    pal.ink_main = look.ink;
    pal.ink_meta = look.ink;
    let w = card_width(list.len());
    let x = card_x(left, SURFACE_W, w);
    // Without the levels, the names move up under the rings and the card shrinks to fit.
    let h = if levels { CARD_H } else { CARD_H - 18.0 };
    let top_y = if top { AIR } else { AIR + CARD_H - h };
    if let Some(fill) = look.card {
        crate::deskcards::card(s, app, (x, top_y, w, h), fill);
    }
    if let Some(shade) = look.shade {
        s.push_shadow(&[gtk::gsk::Shadow::new(shade, 0.0, 1.0, 10.0)]);
    }
    for (i, g) in list.iter().enumerate() {
        let cx = x + INSET + CELL_W * (i as f64 + 0.5);
        crate::batteries::cell(s, p, &app.cfg, &pal, g, cx, top_y + 42.0, CELL_W, false, levels);
    }
    if look.shade.is_some() {
        s.pop();
    }
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
                ("bottom_right", 800.0 - AIR - w, 600.0 - AIR - CARD_H),
            ] {
                assert_eq!(card_rect(placement, count, (800, 600), 26, 0, [0; 3]), Some(CardRect { x, y, w, h: CARD_H }));
            }
        }
        let normal = card_rect("top_left", SHOWN, (800, 600), 26, 0, [0; 3]).unwrap();
        let clock = card_rect("top_left", SHOWN, (800, 600), 26, 214, [0; 3]).unwrap();
        assert_eq!(clock.y, normal.y + 214.0);
        // The clock only pushes the left corner down.
        assert_eq!(card_rect("top_right", SHOWN, (800, 600), 26, 214, [0; 3]).unwrap().y, normal.y);
        assert_eq!(card_rect("top_left", 99, (800, 600), 26, 0, [0; 3]), Some(normal));
        assert!(card_rect("off", SHOWN, (800, 600), 26, 0, [0; 3]).is_none());
        assert!(card_rect("top_right", 0, (800, 600), 26, 0, [0; 3]).is_none());
    }

    #[test]
    fn the_dock_pushes_the_batteries_off_its_edge() {
        let w = card_width(SHOWN);
        let bottom = card_rect("bottom_left", SHOWN, (800, 600), 26, 0, [0, 0, 70]).unwrap();
        assert_eq!(bottom.y, 600.0 - 70.0 - AIR - CARD_H);
        assert_eq!(card_rect("bottom_right", SHOWN, (800, 600), 26, 0, [0, 60, 0]).unwrap().x, 800.0 - 60.0 - AIR - w);
        assert_eq!(card_rect("top_left", SHOWN, (800, 600), 26, 0, [60, 0, 0]).unwrap().x, 60.0 + AIR);
        // A dock at the bottom leaves the top corners where they were.
        assert_eq!(card_rect("top_left", SHOWN, (800, 600), 26, 0, [0, 0, 70]).unwrap().y, 26.0 + AIR);
    }
}
