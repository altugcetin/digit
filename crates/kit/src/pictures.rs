//! Small pictures for setup and Settings: a sketch of the desktop with the notch style or
//! the window layout each choice gives. Drawn with cairo so they follow the surface and accent.

use gtk::prelude::*;
use std::f64::consts::{FRAC_PI_2, PI, TAU};

type Cr = gtk::cairo::Context;

fn rounded(cr: &Cr, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, PI);
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.close_path();
}

/// A wallpaper-like sky that every picture sits on.
fn sky(cr: &Cr, w: f64, h: f64) {
    let g = gtk::cairo::LinearGradient::new(0.0, 0.0, w, h);
    g.add_color_stop_rgb(0.0, 0.34, 0.43, 0.62);
    g.add_color_stop_rgb(1.0, 0.86, 0.55, 0.42);
    rounded(cr, 0.0, 0.0, w, h, 12.0);
    let _ = cr.set_source(&g);
    let _ = cr.fill();
}

fn window(cr: &Cr, x: f64, y: f64, w: f64, h: f64, alpha: f64) {
    cr.set_source_rgba(1.0, 1.0, 1.0, alpha);
    rounded(cr, x, y, w, h, 6.0);
    let _ = cr.fill();
}

fn area(width: i32, height: i32, draw: impl Fn(&Cr, f64, f64) + 'static) -> gtk::DrawingArea {
    let a = gtk::DrawingArea::new();
    a.set_content_width(width);
    a.set_content_height(height);
    a.set_draw_func(move |_, cr, w, h| draw(cr, w as f64, h as f64));
    a
}

/// The notch styles: "island" hugs the top edge, "dynamic" floats, "bar" spans the edge.
pub fn notch(style: &'static str) -> gtk::DrawingArea {
    area(188, 112, move |cr, w, h| {
        sky(cr, w, h);
        window(cr, 18.0, 30.0, w * 0.52, h - 46.0, 0.16);
        window(cr, w * 0.52 + 26.0, 30.0, w - (w * 0.52 + 26.0) - 18.0, h - 46.0, 0.16);
        draw_notch(cr, w, style);
    })
}

fn draw_notch(cr: &Cr, w: f64, style: &str) {
    cr.set_source_rgb(0.0, 0.0, 0.0);
    match style {
        "dynamic" => {
            rounded(cr, w / 2.0 - 30.0, 5.0, 60.0, 14.0, 7.0);
            let _ = cr.fill();
        }
        "bar" => {
            cr.rectangle(0.0, 0.0, w, 16.0);
            let _ = cr.fill();
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.8);
            rounded(cr, 8.0, 6.0, 18.0, 4.0, 2.0);
            let _ = cr.fill();
            rounded(cr, w - 30.0, 6.0, 22.0, 4.0, 2.0);
            let _ = cr.fill();
        }
        _ => {
            let (cx, half, depth, r, f) = (w / 2.0, 34.0, 15.0, 6.0, 5.0);
            cr.new_sub_path();
            cr.move_to(cx - half - f, 0.0);
            cr.arc(cx - half - f, f, f, -FRAC_PI_2, 0.0);
            cr.line_to(cx - half, depth - r);
            cr.arc_negative(cx - half + r, depth - r, r, PI, FRAC_PI_2);
            cr.line_to(cx + half - r, depth);
            cr.arc_negative(cx + half - r, depth - r, r, FRAC_PI_2, 0.0);
            cr.line_to(cx + half, f);
            cr.arc(cx + half + f, f, f, PI, 1.5 * PI);
            cr.close_path();
            let _ = cr.fill();
        }
    }
}

/// Digit's mark: the pixel "d" on an accent tile, then "Digit" in the same pixels.
pub fn brand(cell: f64, tile: f64) -> gtk::DrawingArea {
    let (_, ww, _) = crate::pixel::cells("Digit");
    let (_, mw, mh) = crate::pixel::cells("d");
    let area = gtk::DrawingArea::new();
    area.set_content_width((tile + 4.0 * cell + ww as f64 * cell) as i32);
    area.set_content_height(tile as i32);
    area.set_draw_func(move |widget, cr, _, _| {
        #[allow(deprecated)]
        let color = |name| widget.style_context().lookup_color(name).unwrap_or_else(|| widget.color());
        let set = |ink: gtk::gdk::RGBA| cr.set_source_rgb(ink.red() as f64, ink.green() as f64, ink.blue() as f64);
        rounded(cr, 0.0, 0.0, tile, tile, tile * 0.265);
        set(color("accent_bg_color"));
        let _ = cr.fill();
        set(color("accent_fg_color"));
        crate::pixel::trace(cr, "d", cell, ((tile - mw as f64 * cell) / 2.0).round(), ((tile - mh as f64 * cell) / 2.0).round());
        let _ = cr.fill();
        set(color("d_ink_main"));
        crate::pixel::trace(cr, "Digit", cell, tile + 4.0 * cell, ((tile - 7.0 * cell) / 2.0).round());
        let _ = cr.fill();
    });
    area
}

/// The notch style and window layout for each scene of the showcase; 1.0 is niri's columns.
const SCENES: [(&str, f64); 6] = [("island", 0.0), ("island", 1.0), ("dynamic", 1.0), ("dynamic", 0.0), ("bar", 0.0), ("bar", 1.0)];
const SCENE_MS: u64 = 2400;
const MORPH_MS: f64 = 700.0;

/// Three windows, tiled (0.0) or as scrolling columns (1.0), anything in between while morphing.
fn windows(cr: &Cr, w: f64, h: f64, columns: f64) {
    let (top, bottom, gap) = (24.0, h - 24.0, 6.0);
    let half = (w - 24.0 - gap) / 2.0;
    let mid = (top + bottom) / 2.0;
    let tiled = [(12.0, top, half, bottom - top), (12.0 + half + gap, top, half, mid - top - gap / 2.0), (12.0 + half + gap, mid + gap / 2.0, half, bottom - mid - gap / 2.0)];
    let (a, b) = (40.0, 92.0);
    let scrolled = [(12.0 - 10.0, top, a, bottom - top), (12.0 - 10.0 + a + gap, top, b, bottom - top), (12.0 - 10.0 + a + b + 2.0 * gap, top, 76.0, bottom - top)];
    let mix = |x: f64, y: f64| x + (y - x) * columns;
    for (i, (t, c)) in tiled.iter().zip(scrolled.iter()).enumerate() {
        let alpha = if i == 0 { mix(0.42, 0.22) } else if i == 1 { mix(0.22, 0.42) } else { 0.22 };
        window(cr, mix(t.0, c.0), mix(t.1, c.1), mix(t.2, c.2), mix(t.3, c.3), alpha);
    }
}

fn dock(cr: &Cr, w: f64, h: f64) {
    let (count, icon, pad) = (5.0, 7.0, 3.0);
    let long = count * icon + (count + 1.0) * pad;
    let (x, y) = ((w - long) / 2.0, h - 4.0 - icon - 2.0 * pad);
    cr.set_source_rgba(0.06, 0.06, 0.07, 0.78);
    rounded(cr, x, y, long, icon + 2.0 * pad, 5.0);
    let _ = cr.fill();
    for (i, (r, g, b)) in [(0.29, 0.64, 1.0), (0.31, 0.86, 0.47), (1.0, 0.70, 0.25), (0.81, 0.48, 0.96), (1.0, 0.42, 0.53)].iter().enumerate() {
        cr.set_source_rgb(*r, *g, *b);
        rounded(cr, x + pad + i as f64 * (icon + pad), y + pad, icon, icon, 2.0);
        let _ = cr.fill();
    }
}

/// A living sketch of the desktop: every few seconds the notch changes style or the windows
/// glide between tiles and columns. Still when motion is reduced.
pub fn showcase(width: i32, height: i32) -> gtk::DrawingArea {
    use std::{cell::Cell, rc::Rc};
    let area = gtk::DrawingArea::new();
    area.set_content_width(width);
    area.set_content_height(height);
    // The scene shown now and when the move into it started (frame time, µs).
    let state = Rc::new(Cell::new((0usize, 0i64)));
    let curve = crate::motion::Curve::new(0.16, 1.0, 0.3, 1.0);
    let st = state.clone();
    area.set_draw_func(move |a, cr, w, h| {
        let (index, start) = st.get();
        let now = a.frame_clock().map_or(start, |c| c.frame_time());
        let t = if crate::motion::reduced() || start == 0 { 1.0 } else { curve.at(((now - start) as f64 / (MORPH_MS * 1000.0)).clamp(0.0, 1.0)) };
        let (from, to) = (SCENES[(index + SCENES.len() - 1) % SCENES.len()], SCENES[index]);
        let (sx, sy) = (w as f64 / 188.0, h as f64 / 112.0);
        let s = sx.min(sy);
        cr.translate((w as f64 - 188.0 * s) / 2.0, (h as f64 - 112.0 * s) / 2.0);
        cr.scale(s, s);
        sky(cr, 188.0, 112.0);
        rounded(cr, 0.0, 0.0, 188.0, 112.0, 12.0);
        cr.clip();
        windows(cr, 188.0, 112.0, from.1 + (to.1 - from.1) * t);
        dock(cr, 188.0, 112.0);
        if from.0 == to.0 || t >= 1.0 {
            draw_notch(cr, 188.0, to.0);
        } else {
            for (style, alpha) in [(from.0, 1.0 - t), (to.0, t)] {
                cr.push_group();
                draw_notch(cr, 188.0, style);
                let _ = cr.pop_group_to_source();
                let _ = cr.paint_with_alpha(alpha);
            }
        }
    });
    let weak = area.downgrade();
    gtk::glib::timeout_add_local(std::time::Duration::from_millis(SCENE_MS), move || {
        let Some(area) = weak.upgrade() else { return gtk::glib::ControlFlow::Break };
        if crate::motion::reduced() || !area.is_mapped() {
            return gtk::glib::ControlFlow::Continue;
        }
        let Some(clock) = area.frame_clock() else { return gtk::glib::ControlFlow::Continue };
        let start = clock.frame_time();
        state.set(((state.get().0 + 1) % SCENES.len(), start));
        // Tick only while the move lasts, then let the frame clock rest.
        area.add_tick_callback(move |a, clock| {
            a.queue_draw();
            if crate::motion::reduced() || (clock.frame_time() - start) as f64 > MORPH_MS * 1000.0 { gtk::glib::ControlFlow::Break } else { gtk::glib::ControlFlow::Continue }
        });
        gtk::glib::ControlFlow::Continue
    });
    area
}

/// How each compositor arranges windows: Hyprland tiles them into the screen, niri lays them
/// out as columns on a strip that scrolls sideways.
pub fn compositor(wm: &'static str) -> gtk::DrawingArea {
    area(188, 112, move |cr, w, h| {
        sky(cr, w, h);
        let (top, bottom, gap) = (12.0, h - 12.0, 6.0);
        if wm == "niri" {
            // The third column runs past the edge: there is more strip to scroll to.
            let mut x = 12.0;
            for (width, alpha) in [(54.0, 0.22), (78.0, 0.42), (70.0, 0.22)] {
                window(cr, x, top, width, bottom - top, alpha);
                x += width + gap;
            }
        } else {
            let half = (w - 24.0 - gap) / 2.0;
            let mid = (top + bottom) / 2.0;
            window(cr, 12.0, top, half, bottom - top, 0.42);
            window(cr, 12.0 + half + gap, top, half, mid - top - gap / 2.0, 0.22);
            window(cr, 12.0 + half + gap, mid + gap / 2.0, half, bottom - mid - gap / 2.0, 0.22);
        }
    })
}

/// An accent color as it sits in the apps: a selected row, a button and a switch that's on.
pub fn accent(a: &gtk::DrawingArea, cr: &Cr, w: f64, h: f64, (r, g, b): (f64, f64, f64)) {
    let ink = a.color();
    let (ir, ig, ib) = (ink.red() as f64, ink.green() as f64, ink.blue() as f64);
    let pill = |x: f64, y: f64, pw: f64, ph: f64| rounded(cr, x, y, pw, ph, ph / 2.0);
    pill(8.0, 9.0, w - 16.0, 10.0);
    cr.set_source_rgba(r, g, b, 0.28);
    let _ = cr.fill();
    pill(12.0, 13.0, w * 0.4, 2.5);
    cr.set_source_rgba(ir, ig, ib, 0.7);
    let _ = cr.fill();
    let y = h - 22.0;
    pill(8.0, y, w * 0.42, 13.0);
    cr.set_source_rgb(r, g, b);
    let _ = cr.fill();
    let sx = w - 8.0 - 24.0;
    pill(sx, y, 24.0, 13.0);
    cr.set_source_rgb(r, g, b);
    let _ = cr.fill();
    cr.arc(sx + 24.0 - 6.5, y + 6.5, 4.5, 0.0, TAU);
    cr.set_source_rgb(1.0, 1.0, 1.0);
    let _ = cr.fill();
}

/// "#rrggbb" as cairo components; a soft violet when it doesn't parse.
pub fn rgb(hex: &str) -> (f64, f64, f64) {
    let hex = hex.trim_start_matches('#');
    let part = |i: usize| hex.get(i..i + 2).and_then(|s| u8::from_str_radix(s, 16).ok()).map(|v| v as f64 / 255.0);
    match (part(0), part(2), part(4)) {
        (Some(r), Some(g), Some(b)) => (r, g, b),
        _ => (0.82, 0.74, 1.0),
    }
}
