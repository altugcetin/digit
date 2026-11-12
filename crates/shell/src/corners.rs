use crate::app::App;
use crate::paint::Paint;
use gtk::{cairo, gdk, gsk, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kit::corners::Corner;
use std::cell::{Cell, RefCell};

thread_local! {
    static CORNERS: RefCell<Vec<gtk::Window>> = const { RefCell::new(Vec::new()) };
    static SHOWN: Cell<(i32, usize)> = const { Cell::new((0, 0)) };
}

pub fn apply(app: &App) {
    let r = app.prefs.borrow().screen_corners.clamp(0.0, 40.0).round() as i32;
    let Some(display) = gdk::Display::default() else { return };
    let monitors: Vec<gdk::Monitor> = display.monitors().iter::<gdk::Monitor>().filter_map(Result::ok).collect();
    if SHOWN.with(Cell::get) == (r, monitors.len()) {
        return;
    }
    SHOWN.with(|s| s.set((r, monitors.len())));
    for w in CORNERS.with(|c| std::mem::take(&mut *c.borrow_mut())) {
        w.destroy();
    }
    if r < 2 || app.cfg.dock != crate::config::Dock::Screen {
        return;
    }
    let mut made = Vec::new();
    for m in &monitors {
        for (v, h) in [(Edge::Top, Edge::Left), (Edge::Top, Edge::Right), (Edge::Bottom, Edge::Left), (Edge::Bottom, Edge::Right)] {
            let window = gtk::Window::new();
            window.init_layer_shell();
            // Top, not Overlay: a fullscreen game or video covers the corners like the rest of
            // the shell.
            window.set_layer(Layer::Top);
            window.set_namespace(Some("digit-corner"));
            window.set_keyboard_mode(KeyboardMode::None);
            window.set_exclusive_zone(-1);
            window.set_monitor(Some(m));
            window.set_anchor(v, true);
            window.set_anchor(h, true);
            window.add_css_class("digit");
            let paint = Paint::new(r, r);
            let (top, left) = (v == Edge::Top, h == Edge::Left);
            paint.on_draw(move |s| {
                draw_mask(s, if top {
                    if left { Corner::TopLeft } else { Corner::TopRight }
                } else if left { Corner::BottomLeft } else { Corner::BottomRight }, r as f32);
            });
            window.set_child(Some(&paint));
            window.connect_realize(|w| {
                if let Some(surface) = w.surface() {
                    surface.set_input_region(Some(&cairo::Region::create()));
                }
            });
            window.present();
            made.push(window);
        }
    }
    CORNERS.with(|c| *c.borrow_mut() = made);
}

/// Also used by overlay panels mapped above the screen-corner surfaces.
pub fn draw_mask(s: &gtk::Snapshot, corner: Corner, radius: f32) {
    let (top, left) = (corner.is_top(), corner.is_left());
    let rf = radius;
    let (cx, cy) = (if left { rf } else { 0.0 }, if top { rf } else { 0.0 });
    let (ex, ey) = (if left { 0.0 } else { rf }, if top { 0.0 } else { rf });
    let pb = gsk::PathBuilder::new();
    pb.move_to(ex, ey);
    pb.line_to(if left { rf } else { 0.0 }, ey);
    let k = 0.552_284_8 * rf;
    let sx = if left { -1.0 } else { 1.0 };
    let sy = if top { -1.0 } else { 1.0 };
    pb.cubic_to(cx + sx * k, cy + sy * rf, cx + sx * rf, cy + sy * k, cx + sx * rf, cy);
    pb.line_to(ex, ey);
    pb.close();
    s.append_fill(&pb.to_path(), gsk::FillRule::Winding, &gdk::RGBA::BLACK);
}
