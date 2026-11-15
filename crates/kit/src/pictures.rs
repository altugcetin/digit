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
    tile(move |cr| {
        pane(cr, 8.0, 15.0, 66.0, 48.0, 0.16);
        pane(cr, 80.0, 15.0, 48.0, 48.0, 0.16);
        top(cr, style);
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


// Tile pictures: one sketch language for every choice that is shown as a picture. Each is drawn
// on a 136×72 canvas (the tiles' size), scaled evenly to whatever size it gets; the tile clips
// the corners, so they round with the rest of the app.

const TW: f64 = 136.0;

/// A window on a tile.
fn pane(cr: &Cr, x: f64, y: f64, w: f64, h: f64, alpha: f64) {
    cr.set_source_rgba(1.0, 1.0, 1.0, alpha);
    rounded(cr, x, y, w, h, 4.0);
    let _ = cr.fill();
}
const TH: f64 = 72.0;

/// A tile picture: the sky fills the whole area, the sketch sits on the 136×72 canvas, centered.
fn tile(draw: impl Fn(&Cr) + 'static) -> gtk::DrawingArea {
    zoomed(1.0, move |cr, _| draw(cr))
}

/// Like `tile`, closer in by `zoom` on the top of the screen: the sketch gets a canvas
/// `zoom` times smaller (its width is passed in) and is drawn that much larger.
fn zoomed(zoom: f64, draw: impl Fn(&Cr, f64) + 'static) -> gtk::DrawingArea {
    let a = gtk::DrawingArea::new();
    a.set_content_width(TW as i32);
    a.set_content_height(TH as i32);
    a.set_draw_func(move |_, cr, w, h| {
        let (w, h) = (w as f64, h as f64);
        let g = gtk::cairo::LinearGradient::new(0.0, 0.0, w, h);
        g.add_color_stop_rgb(0.0, 0.34, 0.43, 0.62);
        g.add_color_stop_rgb(1.0, 0.86, 0.55, 0.42);
        cr.rectangle(0.0, 0.0, w, h);
        let _ = cr.set_source(&g);
        let _ = cr.fill();
        let k = (w / TW).min(h / TH);
        cr.translate((w - TW * k) / 2.0, (h - TH * k) / 2.0);
        cr.scale(k * zoom, k * zoom);
        draw(cr, TW / zoom);
    });
    a
}

const NOTCH_W: f64 = 34.0;
const BAND: f64 = 9.0;

/// The top of the screen: the notch ("island"), the floating island ("dynamic") or a full bar.
fn top(cr: &Cr, style: &str) {
    top_in(cr, TW, style)
}

fn top_in(cr: &Cr, cw: f64, style: &str) {
    cr.set_source_rgb(0.0, 0.0, 0.0);
    match style {
        "dynamic" => {
            rounded(cr, (cw - NOTCH_W) / 2.0, 2.5, NOTCH_W, 8.0, 4.0);
            let _ = cr.fill();
        }
        "bar" => {
            cr.rectangle(0.0, 0.0, cw, BAND);
            let _ = cr.fill();
            menu_items(cr, cw, 0.0, 1.0);
        }
        _ => notch_shape(cr, cw, NOTCH_W, BAND, (0.0, 0.0, 0.0), 1.0),
    }
}

/// The notch hanging from the top edge, with concave fillets where it meets it.
fn notch_shape(cr: &Cr, cw: f64, width: f64, depth: f64, (r, g, b): (f64, f64, f64), alpha: f64) {
    let (cx, half, rad, f) = (cw / 2.0, width / 2.0, 3.5, 2.5);
    cr.new_sub_path();
    cr.move_to(cx - half - f, 0.0);
    cr.arc(cx - half - f, f, f, -FRAC_PI_2, 0.0);
    cr.line_to(cx - half, depth - rad);
    cr.arc_negative(cx - half + rad, depth - rad, rad, PI, FRAC_PI_2);
    cr.line_to(cx + half - rad, depth);
    cr.arc_negative(cx + half - rad, depth - rad, rad, FRAC_PI_2, 0.0);
    cr.line_to(cx + half, f);
    cr.arc(cx + half + f, f, f, PI, 1.5 * PI);
    cr.close_path();
    cr.set_source_rgba(r, g, b, alpha);
    let _ = cr.fill();
}

/// The menu bar's logo and menus at the left (after `from`, past anything already there) and
/// the date at the right, with whatever fits clear of a notch in the middle.
fn menu_items(cr: &Cr, cw: f64, from: f64, alpha: f64) {
    let y = BAND / 2.0;
    let clear = cw / 2.0 - NOTCH_W / 2.0 - 3.0;
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.9 * alpha);
    rounded(cr, 6.0, y - 2.0, 4.0, 4.0, 1.2);
    let _ = cr.fill();
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.75 * alpha);
    let mut x = from.max(14.0);
    for width in [9.0, 6.0, 8.0] {
        if x + width > clear {
            break;
        }
        rounded(cr, x, y - 1.0, width, 2.0, 1.0);
        let _ = cr.fill();
        x += width + 3.0;
    }
    rounded(cr, cw - 6.0 - 13.0, y - 1.0, 13.0, 2.0, 1.0);
    let _ = cr.fill();
}

/// The shade a clear bar puts over the wallpaper, from `x` for `width`.
fn shade(cr: &Cr, x: f64, width: f64) {
    let g = gtk::cairo::LinearGradient::new(0.0, 0.0, 0.0, BAND + 7.0);
    g.add_color_stop_rgba(0.0, 0.0, 0.0, 0.0, 0.40);
    g.add_color_stop_rgba(1.0, 0.0, 0.0, 0.0, 0.0);
    cr.rectangle(x, 0.0, width, BAND + 7.0);
    let _ = cr.set_source(&g);
    let _ = cr.fill();
}

/// A glass bar band from `x` for `width`.
fn glass(cr: &Cr, x: f64, width: f64) {
    cr.set_source_rgba(0.10, 0.10, 0.13, 0.78);
    cr.rectangle(x, 0.0, width, BAND);
    let _ = cr.fill();
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.16);
    cr.rectangle(x, BAND - 0.6, width, 0.6);
    let _ = cr.fill();
}

thread_local! {
    /// Pictures drawn in the app's live colors, redrawn when the style changes.
    static LIVE: std::cell::RefCell<Vec<gtk::glib::WeakRef<gtk::DrawingArea>>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Redraws the pictures that follow the app's colors, after the style changed (a new accent).
pub fn redraw_live() {
    LIVE.with(|l| {
        let mut l = l.borrow_mut();
        l.retain(|w| w.upgrade().is_some());
        for a in l.iter().filter_map(|w| w.upgrade()) {
            a.queue_draw();
        }
    });
}

/// A shell surface ("black", "graphite" or "tinted"), pictured as the top corner of Control
/// Center over the wallpaper: the weather, then four tiles, the top two on. The panel is the
/// surface's color and the tiles the apps' cards, worked out from the live accent as it draws.
pub fn shell_surface(surface: &'static str) -> gtk::DrawingArea {
    let a = gtk::DrawingArea::new();
    a.set_content_width(TW as i32);
    a.set_content_height(TH as i32);
    a.set_draw_func(move |a, cr, w, h| {
        let (w, h) = (w as f64, h as f64);
        let g = gtk::cairo::LinearGradient::new(0.0, 0.0, w, h);
        g.add_color_stop_rgb(0.0, 0.34, 0.43, 0.62);
        g.add_color_stop_rgb(1.0, 0.86, 0.55, 0.42);
        cr.rectangle(0.0, 0.0, w, h);
        let _ = cr.set_source(&g);
        let _ = cr.fill();
        let k = (w / TW).min(h / TH);
        cr.translate((w - TW * k) / 2.0, (h - TH * k) / 2.0);
        cr.scale(k, k);
        let accent = named(a, "d_accent", (0.82, 0.74, 1.0));
        let on_accent = named(a, "d_on_accent", (0.1, 0.1, 0.1));
        let tone = crate::palette::Rgb::new(accent.0, accent.1, accent.2);
        let rgb = |c: crate::palette::Rgb| (c.r, c.g, c.b);
        let (body, card) = (rgb(crate::style::base_color(surface, tone)), rgb(crate::style::card_color(surface, tone)));
        let fill = |x: f64, y: f64, w: f64, h: f64, r: f64, (cr_, cg, cb): (f64, f64, f64), alpha: f64| {
            cr.set_source_rgba(cr_, cg, cb, alpha);
            rounded(cr, x, y, w, h, r);
            let _ = cr.fill();
        };
        // The panel, running on past the right and bottom edges, its top-left corner showing as
        // far in from the left as from the top.
        let (px, py) = (14.0, 14.0);
        fill(px, py, TW - px + 12.0, TH - py + 12.0, 11.0, body, 1.0);
        let white = (1.0, 1.0, 1.0);
        // The weather: a cloud, the temperature, the sky and the place.
        // Inset from the panel's edges as much on the left as on top.
        fill(px + 5.0, py + 7.0, 9.0, 5.0, 2.5, white, 0.9);
        fill(px + 7.0, py + 5.0, 5.0, 5.0, 2.5, white, 0.9);
        fill(px + 17.0, py + 5.5, 11.0, 6.0, 1.5, white, 0.92);
        fill(px + 31.0, py + 5.5, 9.0, 2.2, 1.1, white, 0.8);
        fill(px + 31.0, py + 9.5, 20.0, 1.6, 0.8, white, 0.45);
        // Four tiles: the top two on, in the accent, the others cards.
        let (pad, gap, th) = (5.0, 4.0, 15.0);
        let tw = (TW - px - 2.0 * pad - gap) / 2.0;
        for (i, (col, row)) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)].into_iter().enumerate() {
            let on = i < 2;
            let (x, y) = (px + pad + col * (tw + gap), py + 16.0 + row * (th + gap));
            fill(x, y, tw, th, 6.0, if on { accent } else { card }, 1.0);
            let (ink, soft) = if on { (on_accent, 0.8) } else { (white, 0.9) };
            fill(x + 4.5, y + th / 2.0 - 2.5, 5.0, 5.0, 2.5, ink, soft * 0.8);
            fill(x + 13.0, y + 4.5, 16.0, 2.4, 1.2, ink, soft);
            fill(x + 13.0, y + 9.5, 11.0, 1.8, 0.9, ink, soft * 0.6);
        }
    });
    LIVE.with(|l| l.borrow_mut().push(a.downgrade()));
    a
}

/// The menu bar's backdrop: "clear" shades the top, "glass" is a band, "adaptive" is half of
/// each: clear over the wallpaper, glass once windows open.
pub fn menubar_look(look: &'static str) -> gtk::DrawingArea {
    zoomed(ZOOM, move |cr, cw| {
        match look {
            "glass" => {
                glass(cr, 0.0, cw);
            }
            "adaptive" => {
                shade(cr, 0.0, cw / 2.0);
                glass(cr, cw / 2.0, cw / 2.0);
            }
            _ => shade(cr, 0.0, cw),
        }
        notch_shape(cr, cw, NOTCH_W, BAND, (0.0, 0.0, 0.0), 1.0);
        menu_items(cr, cw, 0.0, 1.0);
    })
}

/// How far the menu bar pictures look in on the top of the screen.
const ZOOM: f64 = 1.45;

/// The workspace pill: three dots, the current one long.
fn spaces_pill(cr: &Cr, x: f64, cy: f64, solid: bool) {
    let (w, h) = (17.0, 6.0);
    if solid {
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.24);
        rounded(cr, x, cy - h / 2.0, w, h, h / 2.0);
        let _ = cr.fill();
    } else {
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.8);
        cr.set_line_width(0.7);
        cr.set_dash(&[1.6, 1.2], 0.0);
        rounded(cr, x + 0.35, cy - h / 2.0 + 0.35, w - 0.7, h - 0.7, h / 2.0);
        let _ = cr.stroke();
        cr.set_dash(&[], 0.0);
    }
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.95);
    rounded(cr, x + 2.0, cy - 1.0, 6.0, 2.0, 1.0);
    let _ = cr.fill();
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.5);
    for dx in [10.5, 13.5] {
        cr.arc(x + dx, cy, 1.0, 0.0, TAU);
        let _ = cr.fill();
    }
}

/// Where the workspace pill shows: "switch" beside the logo while switching (drawn as an
/// outline), "always" there all the time, "notch" in the notch in place of the clock.
pub fn menubar_spaces(mode: &'static str) -> gtk::DrawingArea {
    zoomed(ZOOM, move |cr, cw| {
        shade(cr, 0.0, cw);
        let y = BAND / 2.0;
        if mode == "notch" {
            notch_shape(cr, cw, NOTCH_W, BAND + 1.0, (0.0, 0.0, 0.0), 1.0);
            spaces_pill(cr, (cw - 17.0) / 2.0, y, true);
            menu_items(cr, cw, 0.0, 1.0);
        } else {
            notch_shape(cr, cw, NOTCH_W, BAND, (0.0, 0.0, 0.0), 1.0);
            spaces_pill(cr, 13.0, y, mode == "always");
            menu_items(cr, cw, 13.0 + 17.0 + 3.0, 1.0);
        }
    })
}

/// A tile drawn on the app surface (the tile's own background) rather than the sky, for the
/// choices that change the apps' insides. The drawing gets the widget for its colors.
fn surface_tile(draw: impl Fn(&gtk::DrawingArea, &Cr) + 'static) -> gtk::DrawingArea {
    let a = gtk::DrawingArea::new();
    a.set_content_width(TW as i32);
    a.set_content_height(TH as i32);
    a.add_css_class("accent-tile");
    a.set_draw_func(move |a, cr, w, h| {
        let (w, h) = (w as f64, h as f64);
        let k = (w / TW).min(h / TH);
        cr.translate((w - TW * k) / 2.0, (h - TH * k) / 2.0);
        cr.scale(k, k);
        draw(a, cr);
    });
    a
}

/// A named color of the app's style, or `fallback`.
fn named(a: &gtk::DrawingArea, name: &str, fallback: (f64, f64, f64)) -> (f64, f64, f64) {
    #[allow(deprecated)]
    a.style_context().lookup_color(name).map_or(fallback, |c| (c.red() as f64, c.green() as f64, c.blue() as f64))
}

/// Two screens side by side with the wallpaper "same" on each, "span" across, or a different
/// one on each ("separate").
pub fn wallpapers(layout: &'static str) -> gtk::DrawingArea {
    surface_tile(move |_, cr| {
        let (w, h, gap, top) = (54.0, 34.0, 6.0, 15.0);
        let left = (TW - 2.0 * w - gap) / 2.0;
        let paint = |x: f64, from: (f64, f64, f64), to: (f64, f64, f64), x0: f64, x1: f64| {
            let g = gtk::cairo::LinearGradient::new(x0, top, x1, top + h);
            g.add_color_stop_rgb(0.0, from.0, from.1, from.2);
            g.add_color_stop_rgb(1.0, to.0, to.1, to.2);
            rounded(cr, x, top, w, h, 4.0);
            let _ = cr.set_source(&g);
            let _ = cr.fill();
        };
        let (sky_a, sky_b) = ((0.34, 0.43, 0.62), (0.86, 0.55, 0.42));
        let right = left + w + gap;
        match layout {
            "span" => {
                paint(left, sky_a, sky_b, left, right + w);
                paint(right, sky_a, sky_b, left, right + w);
            }
            "separate" => {
                paint(left, sky_a, sky_b, left, left + w);
                paint(right, (0.20, 0.55, 0.45), (0.85, 0.80, 0.45), right, right + w);
            }
            _ => {
                paint(left, sky_a, sky_b, left, left + w);
                paint(right, sky_a, sky_b, right, right + w);
            }
        }
        // Their stands.
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.25);
        for x in [left, right] {
            rounded(cr, x + w / 2.0 - 7.0, top + h + 4.0, 14.0, 3.0, 1.5);
            let _ = cr.fill();
        }
    })
}

/// An arrow from (x0, y0) to (x1, y1), with a head at the end.
fn arrow(cr: &Cr, x0: f64, y0: f64, x1: f64, y1: f64) {
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.9);
    cr.set_line_width(1.6);
    cr.set_line_cap(gtk::cairo::LineCap::Round);
    cr.move_to(x0, y0);
    cr.line_to(x1, y1);
    let angle = (y1 - y0).atan2(x1 - x0);
    for side in [-1.0, 1.0] {
        let a = angle + PI - side * 0.55;
        cr.move_to(x1, y1);
        cr.line_to(x1 + 4.5 * a.cos(), y1 + 4.5 * a.sin());
    }
    let _ = cr.stroke();
}

/// How Hyprland opens windows: "popin" grows them from smaller, "slide" brings them in from
/// below, "none" shows them at once.
pub fn window_motion(kind: &'static str) -> gtk::DrawingArea {
    tile(move |cr| {
        let (x, y, w, h) = (40.0, 14.0, 56.0, 40.0);
        match kind {
            "slide" => {
                pane(cr, x, y + 14.0, w, h, 0.10);
                pane(cr, x, y, w, h, 0.32);
                arrow(cr, x + w + 12.0, y + h + 4.0, x + w + 12.0, y + 10.0);
            }
            "none" => still(cr),
            _ => {
                pane(cr, x + 10.0, y + 8.0, w - 20.0, h - 16.0, 0.12);
                pane(cr, x, y, w, h, 0.32);
                arrow(cr, x + 14.0, y + 12.0, x + 3.0, y + 3.0);
                arrow(cr, x + w - 14.0, y + h - 12.0, x + w - 3.0, y + h - 3.0);
            }
        }
    })
}

/// No motion: a single window, where it ends up.
fn still(cr: &Cr) {
    pane(cr, 40.0, 16.0, 56.0, 40.0, 0.32);
}

/// A pane whose opacity goes from `from` to `to` along the way it moves, as it fades.
fn fading(cr: &Cr, (x, y, w, h): (f64, f64, f64, f64), across: bool, from: f64, to: f64) {
    let g = if across { gtk::cairo::LinearGradient::new(x, 0.0, x + w, 0.0) } else { gtk::cairo::LinearGradient::new(0.0, y + h, 0.0, y) };
    g.add_color_stop_rgba(0.0, 1.0, 1.0, 1.0, from);
    g.add_color_stop_rgba(1.0, 1.0, 1.0, 1.0, to);
    rounded(cr, x, y, w, h, 4.0);
    let _ = cr.set_source(&g);
    let _ = cr.fill();
}

/// How Hyprland switches workspaces: "vertical" slides them up, "horizontal" sideways, the
/// "_fade" ones fade them while they slide, and "none" swaps them at once.
pub fn workspace_motion(kind: &'static str) -> gtk::DrawingArea {
    tile(move |cr| {
        let fade = kind.ends_with("_fade");
        if kind == "none" {
            still(cr);
        } else if kind.starts_with("horizontal") {
            let (from, to) = ((10.0, 16.0, 50.0, 40.0), (76.0, 16.0, 50.0, 40.0));
            if fade {
                fading(cr, from, true, 0.32, 0.06);
                fading(cr, to, true, 0.04, 0.22);
            } else {
                pane(cr, from.0, from.1, from.2, from.3, 0.32);
                pane(cr, to.0, to.1, to.2, to.3, 0.14);
            }
            arrow(cr, 62.0, 36.0, 74.0, 36.0);
        } else {
            let (from, to) = ((43.0, 40.0, 50.0, 28.0), (43.0, 4.0, 50.0, 28.0));
            if fade {
                fading(cr, to, false, 0.06, 0.32);
                fading(cr, from, false, 0.22, 0.04);
            } else {
                pane(cr, to.0, to.1, to.2, to.3, 0.32);
                pane(cr, from.0, from.1, from.2, from.3, 0.14);
            }
            arrow(cr, 100.0, 40.0, 100.0, 30.0);
        }
    })
}

/// How niri marks the focused window: a "border" on its edge, a "ring" around it, or "none".
pub fn focus_mark(kind: &'static str) -> gtk::DrawingArea {
    let a = tile(|_| {});
    a.set_draw_func(move |a, cr, w, h| {
        let (w, h) = (w as f64, h as f64);
        let g = gtk::cairo::LinearGradient::new(0.0, 0.0, w, h);
        g.add_color_stop_rgb(0.0, 0.34, 0.43, 0.62);
        g.add_color_stop_rgb(1.0, 0.86, 0.55, 0.42);
        cr.rectangle(0.0, 0.0, w, h);
        let _ = cr.set_source(&g);
        let _ = cr.fill();
        let k = (w / TW).min(h / TH);
        cr.translate((w - TW * k) / 2.0, (h - TH * k) / 2.0);
        cr.scale(k, k);
        let accent = named(a, "d_accent", (0.55, 0.72, 0.95));
        pane(cr, 10.0, 14.0, 52.0, 44.0, 0.16);
        let (x, y, ww, hh) = (72.0, 14.0, 54.0, 44.0);
        pane(cr, x, y, ww, hh, 0.32);
        cr.set_source_rgb(accent.0, accent.1, accent.2);
        match kind {
            "ring" => {
                cr.set_line_width(2.0);
                rounded(cr, x - 3.0, y - 3.0, ww + 6.0, hh + 6.0, 6.5);
                let _ = cr.stroke();
            }
            "none" => {}
            _ => {
                cr.set_line_width(2.0);
                rounded(cr, x + 1.0, y + 1.0, ww - 2.0, hh - 2.0, 3.5);
                let _ = cr.stroke();
            }
        }
    });
    a
}
