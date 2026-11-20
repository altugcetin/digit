//! Shared placement for corner panels. Content stays upright; only the slab is mirrored.
use crate::app::App;
use crate::draw;
use gtk::{gdk, graphene, gsk, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kit::corners::{self, Corner, Panel};
use kit::palette::Rgb;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn contains(self, x: f64, y: f64) -> bool {
        x >= self.x && x <= self.x + self.w && y >= self.y && y <= self.y + self.h
    }

    pub fn region(self) -> gtk::cairo::RectangleInt {
        let (x, y) = (self.x.floor(), self.y.floor());
        gtk::cairo::RectangleInt::new(x as i32, y as i32, (self.x + self.w - x).ceil() as i32, (self.y + self.h - y).ceil() as i32)
    }
}

pub fn placement(corner: Corner, surface: (f64, f64), size: (f64, f64), rise: f64, fillet: f64) -> Rect {
    let (w, h) = size;
    let shift = ((1.0 - rise) * (h + fillet)).round();
    Rect { x: if corner.is_left() { 0.0 } else { surface.0 - w }, y: if corner.is_top() { -shift } else { surface.1 - h + shift }, w, h }
}

/// Include the concave fillets, but never the unused area of the fixed-size surface.
pub fn envelope(corner: Corner, rect: Rect, surface: (f64, f64), fillet: f64) -> Rect {
    let x = (rect.x - if corner.is_left() { 0.0 } else { fillet }).max(0.0);
    let y = (rect.y - if corner.is_top() { 0.0 } else { fillet }).max(0.0);
    let right = (rect.x + rect.w + if corner.is_left() { fillet } else { 0.0 }).min(surface.0);
    let bottom = (rect.y + rect.h + if corner.is_top() { fillet } else { 0.0 }).min(surface.1);
    Rect { x, y, w: (right - x).max(0.0), h: (bottom - y).max(0.0) }
}

/// The corner `panel` opens in on the screen `output`: where that screen has it, else where the
/// shared corners have it (as when it's opened by its shortcut on a screen without it).
pub fn selected(app: &App, panel: Panel, output: &str) -> Corner {
    let prefs = app.prefs.borrow();
    corner_for(&prefs.corners_on(output), &prefs.corners, panel)
}

fn corner_for(own: &[Panel; 4], shared: &[Panel; 4], panel: Panel) -> Corner {
    corners::corner_of(own, panel).or_else(|| corners::corner_of(shared, panel)).unwrap_or(match panel {
        Panel::Control => Corner::BottomRight,
        _ => Corner::TopLeft,
    })
}

pub fn anchor(window: &gtk::Window, corner: Corner) {
    for (edge, on) in [(Edge::Top, corner.is_top()), (Edge::Bottom, !corner.is_top()), (Edge::Left, corner.is_left()), (Edge::Right, !corner.is_left())] {
        window.set_anchor(edge, on);
    }
}

/// Panels attach to the physical screen corner, independently of dock geometry
/// and exclusive zones, matching the original Control Center and Workspaces.
pub fn margins(_corner: Corner) -> [i32; 4] {
    [0; 4]
}

pub fn position(window: &gtk::Window, corner: Corner) -> [i32; 4] {
    anchor(window, corner);
    let margins = margins(corner);
    for (edge, margin) in [Edge::Top, Edge::Left, Edge::Right, Edge::Bottom].into_iter().zip(margins) {
        window.set_margin(edge, margin);
    }
    margins
}

pub fn refresh(app: &App) {
    crate::spaces::reposition();
    if let Some(c) = app.control.borrow().as_ref() {
        c.reposition();
    }
}

pub fn activate(app: &Rc<App>, output: &str, corner: Corner) -> bool {
    let panel = app.prefs.borrow().corners_on(output)[Corner::ALL.iter().position(|c| *c == corner).unwrap()];
    if panel == Panel::None || (panel == Panel::Workspaces && !app.cfg.outputs.is_empty() && !app.cfg.outputs.iter().any(|o| o == output)) {
        return false;
    }
    if app.edges_live(output) {
        match panel {
            Panel::Workspaces => crate::spaces::show_on(app, Some(output.into())),
            Panel::Control => crate::control::show_on(app, Some(output.into())),
            Panel::None => {}
        }
    }
    true
}

pub fn triggers(app: &Rc<App>, monitor: &gdk::Monitor) -> Vec<gtk::Window> {
    let output = monitor.connector().map(|c| c.to_string()).unwrap_or_default();
    let assignment = app.prefs.borrow().corners_on(&output);
    let spaces_output = app.cfg.outputs.is_empty() || app.cfg.outputs.contains(&output);
    Corner::ALL
        .into_iter()
        .zip(assignment)
        .filter(|(_, panel)| *panel != Panel::None && (*panel != Panel::Workspaces || spaces_output))
        .map(|(corner, _)| {
            let window = gtk::Window::new();
            window.init_layer_shell();
            window.set_layer(Layer::Overlay);
            window.set_namespace(Some("digit-corner"));
            anchor(&window, corner);
            window.set_exclusive_zone(-1);
            window.set_keyboard_mode(KeyboardMode::None);
            window.set_monitor(Some(monitor));
            window.add_css_class("digit");
            window.set_default_size(2, 2);
            let paint = crate::paint::Paint::new(2, 2);
            paint.present();
            window.set_child(Some(&paint));
            let motion = gtk::EventControllerMotion::new();
            let weak = Rc::downgrade(app);
            let output = monitor.connector().map(|c| c.to_string()).unwrap_or_default();
            motion.connect_enter(move |_, _, _| {
                if let Some(app) = weak.upgrade() {
                    activate(&app, &output, corner);
                }
            });
            paint.add_controller(motion);
            window.present();
            window
        })
        .collect()
}

pub fn clip(corner: Corner, rect: Rect, radius: f64, fillet: f64) -> gsk::RoundedRect {
    let fillet = fillet.min(rect.w / 2.0).min(rect.h / 2.0).max(0.0);
    let r = radius.min(rect.w - fillet).min(rect.h - fillet).max(0.0) as f32;
    let zero = graphene::Size::zero();
    let round = graphene::Size::new(r, r);
    gsk::RoundedRect::new(
        graphene::Rect::new(rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32),
        if corner == Corner::BottomRight { round } else { zero },
        if corner == Corner::BottomLeft { round } else { zero },
        if corner == Corner::TopLeft { round } else { zero },
        if corner == Corner::TopRight { round } else { zero },
    )
}

pub fn slab(snapshot: &gtk::Snapshot, corner: Corner, rect: Rect, radius: f64, fillet: f64, color: Rgb) {
    snapshot.save();
    snapshot.translate(&graphene::Point::new(
        (rect.x + if corner.is_left() { rect.w } else { 0.0 }) as f32,
        (rect.y + if corner.is_top() { rect.h } else { 0.0 }) as f32,
    ));
    snapshot.scale(if corner.is_left() { -1.0 } else { 1.0 }, if corner.is_top() { -1.0 } else { 1.0 });
    if color.a >= 1.0 {
        if let Some(parts) = kit::shape::cornered_parts(rect.w, rect.h, rect.w, rect.h, radius, fillet) {
            draw::slab(snapshot, &parts, color);
        }
    } else if let Some(path) = kit::shape::cornered(rect.w, rect.h, rect.w, rect.h, radius, fillet) {
        snapshot.append_fill(&path, gsk::FillRule::Winding, &color.rgba());
    }
    snapshot.restore();
}

/// The corners a hanging panel rounds: only those away from the screen edge, and on a full
/// bar not the top one, which sits flat against it.
fn hang_corners(left: bool, bar: bool, radius: f64) -> [graphene::Size; 4] {
    let (z, r) = (graphene::Size::zero(), graphene::Size::new(radius as f32, radius as f32));
    let top = if bar { z } else { r };
    // top-left, top-right, bottom-right, bottom-left
    if left { [z, top, r, z] } else { [top, z, z, r] }
}

pub fn hang_clip(rect: Rect, left: bool, bar: bool, radius: f64) -> gsk::RoundedRect {
    let [a, b, c, d] = hang_corners(left, bar, radius);
    gsk::RoundedRect::new(graphene::Rect::new(rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32), a, b, c, d)
}

/// A panel hanging from the top bar against the screen's side edge, as the calendar does:
/// flat against a full bar with a cove into the side edge below it, or under a notch with
/// coves above and below, the top-edge notch shape turned a quarter.
pub fn hang(snapshot: &gtk::Snapshot, rect: Rect, left: bool, bar: bool, radius: f64, fillet: f64, color: Rgb) {
    snapshot.save();
    // Draw as if on the right side; mirror for the left.
    snapshot.translate(&graphene::Point::new(rect.x as f32, rect.y as f32));
    if left {
        snapshot.translate(&graphene::Point::new(rect.w as f32, 0.0));
        snapshot.scale(-1.0, 1.0);
    }
    let (w, h) = (rect.w as f32, rect.h as f32);
    if bar {
        let local = Rect { x: 0.0, y: 0.0, ..rect };
        let body = hang_clip(local, false, true, radius);
        let c = (fillet as f32).min(h / 2.0);
        let cove = gsk::PathBuilder::new();
        cove.move_to(w - c, h - 1.0);
        cove.line_to(w, h - 1.0);
        cove.line_to(w, h + c);
        cove.svg_arc_to(c, c, 0.0, false, false, w - c, h);
        cove.close();
        draw::slab(snapshot, &(body, Some(cove.to_path())), color);
    } else {
        snapshot.translate(&graphene::Point::new(w, -(fillet as f32)));
        snapshot.rotate(90.0);
        if let Some(parts) = kit::shape::docked_parts(rect.h / 2.0 + fillet, 0.0, rect.h, rect.w, radius, fillet) {
            draw::slab(snapshot, &parts, color);
        }
    }
    snapshot.restore();
}

/// A newly mapped Overlay panel can be above the persistent screen-corner masks.
/// Redraw the mask at its attached physical corner, outside the animated slab.
pub fn screen_mask(snapshot: &gtk::Snapshot, app: &App, corner: Corner, surface: (f64, f64), margins: [i32; 4]) {
    let radius = app.prefs.borrow().screen_corners.clamp(0.0, 40.0).round() as f32;
    if radius < 2.0
        || app.cfg.dock != crate::config::Dock::Screen
        || margins[if corner.is_top() { 0 } else { 3 }] != 0
        || margins[if corner.is_left() { 1 } else { 2 }] != 0
    {
        return;
    }
    snapshot.save();
    snapshot.translate(&graphene::Point::new(
        if corner.is_left() { 0.0 } else { surface.0 as f32 - radius },
        if corner.is_top() { 0.0 } else { surface.1 as f32 - radius },
    ));
    crate::corners::draw_mask(snapshot, corner, radius);
    snapshot.restore();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_use_assignments_or_the_original_default_corner() {
        let empty = [Panel::None; 4];
        assert_eq!(corner_for(&empty, &empty, Panel::Workspaces), Corner::TopLeft);
        assert_eq!(corner_for(&empty, &empty, Panel::Control), Corner::BottomRight);
        for corner in Corner::ALL {
            for panel in [Panel::Workspaces, Panel::Control] {
                assert_eq!(corner_for(&corners::assign(empty, corner, panel), &empty, panel), corner);
                // A screen without it opens it where the shared corners have it.
                assert_eq!(corner_for(&empty, &corners::assign(empty, corner, panel), panel), corner);
            }
        }
        // A screen's own corner wins over the shared one.
        let own = corners::assign(empty, Corner::TopRight, Panel::Control);
        let shared = corners::assign(empty, Corner::BottomLeft, Panel::Control);
        assert_eq!(corner_for(&own, &shared, Panel::Control), Corner::TopRight);
    }

    #[test]
    fn panels_place_against_each_assigned_edge() {
        let expected = [(0.0, 0.0), (300.0, 0.0), (0.0, 200.0), (300.0, 200.0)];
        for (corner, (x, y)) in Corner::ALL.into_iter().zip(expected) {
            let rect = placement(corner, (500.0, 400.0), (200.0, 200.0), 1.0, 12.0);
            assert_eq!(rect, Rect { x, y, w: 200.0, h: 200.0 });
            assert!(rect.contains(x + 16.0, y + 16.0));
            assert!(!rect.contains(x - 1.0, y));
            let env = envelope(corner, rect, (500.0, 400.0), 12.0);
            assert!(env.contains(x, y));
            assert!(env.x >= 0.0 && env.y >= 0.0 && env.x + env.w <= 500.0 && env.y + env.h <= 400.0);
            let hidden = placement(corner, (500.0, 400.0), (200.0, 200.0), 0.0, 12.0);
            assert_eq!(hidden.x, x);
            assert_eq!(hidden.y - y, if corner.is_top() { -212.0 } else { 212.0 });
            assert_eq!(envelope(corner, hidden, (500.0, 400.0), 12.0).h, 0.0);
        }
    }

    #[test]
    fn panels_touch_screen_edges_even_when_the_dock_overlaps() {
        let screen = (1280.0, 720.0);
        for corner in Corner::ALL {
            assert_eq!(margins(corner), [0; 4]);
            let panel = placement(corner, screen, (380.0, 600.0), 1.0, 12.0);
            assert_eq!(if corner.is_left() { panel.x } else { screen.0 - panel.x - panel.w }, 0.0);
            assert_eq!(if corner.is_top() { panel.y } else { screen.1 - panel.y - panel.h }, 0.0);
            // A dock at either touching edge may overlap the panel; it must never
            // displace it, even at its largest magnified size.
            for depth in [0.0, 28.0, 96.0, 240.0] {
                for dock in [
                    Rect { x: 0.0, y: 0.0, w: depth, h: screen.1 },
                    Rect { x: screen.0 - depth, y: 0.0, w: depth, h: screen.1 },
                    Rect { x: 0.0, y: screen.1 - depth, w: screen.0, h: depth },
                ] {
                    let edge = (if corner.is_left() { 0.0 } else { screen.0 }, if corner.is_top() { 0.0 } else { screen.1 });
                    if dock.contains(edge.0, edge.1) {
                        assert!(panel.contains(edge.0, edge.1));
                    }
                }
            }
        }
    }

    #[test]
    fn fillets_extend_only_toward_the_screen_interior() {
        for corner in Corner::ALL {
            let rect = placement(corner, (212.0, 312.0), (200.0, 300.0), 1.0, 12.0);
            assert_eq!(envelope(corner, rect, (212.0, 312.0), 12.0), Rect { x: 0.0, y: 0.0, w: 212.0, h: 312.0 });
            let clip = clip(corner, rect, 26.0, 12.0);
            let radii = clip.corner().map(|r| r.width());
            let opposite = match corner {
                Corner::TopLeft => 2,
                Corner::TopRight => 3,
                Corner::BottomLeft => 1,
                Corner::BottomRight => 0,
            };
            for (i, radius) in radii.into_iter().enumerate() {
                assert_eq!(radius, if i == opposite { 26.0 } else { 0.0 });
            }
        }
    }
}
