use gtk::gsk::{Path, PathBuilder};

pub fn docked(cx: f64, top: f64, width: f64, depth: f64, radius: f64, fillet: f64) -> Option<Path> {
    outline(cx, top, width, depth, radius, fillet, true)
}

pub fn rim(cx: f64, top: f64, width: f64, depth: f64, radius: f64, fillet: f64) -> Option<Path> {
    outline(cx, top, width, depth, radius, fillet, false)
}

fn outline(cx: f64, top: f64, width: f64, depth: f64, radius: f64, fillet: f64, closed: bool) -> Option<Path> {
    if width < 1.0 || depth < 0.5 {
        return None;
    }
    let s = fillet.min(depth / 2.0).max(0.0);
    let r = radius.min(depth - s).min(width / 2.0).max(0.0);
    let x0 = cx - width / 2.0 - s;
    let p = |u: f64, v: f64| ((x0 + u) as f32, (top + v) as f32);
    let b = PathBuilder::new();
    let (x, y) = p(0.0, 0.0);
    b.move_to(x, y);
    let arc = |rad: f64, positive: bool, (x, y): (f32, f32)| b.svg_arc_to(rad as f32, rad as f32, 0.0, false, positive, x, y);
    arc(s, true, p(s, s));
    let (x, y) = p(s, depth - r);
    b.line_to(x, y);
    arc(r, false, p(s + r, depth));
    let (x, y) = p(s + width - r, depth);
    b.line_to(x, y);
    arc(r, false, p(s + width, depth - r));
    let (x, y) = p(s + width, s);
    b.line_to(x, y);
    arc(s, true, p(2.0 * s + width, 0.0));
    if closed {
        b.close();
    }
    Some(b.to_path())
}

pub fn rows(cx: f64, top: f64, width: f64, depth: f64, radius: f64, fillet: f64, inset: f64) -> Vec<(i32, i32, i32, i32)> {
    if width < 1.0 || depth < 0.5 {
        return Vec::new();
    }
    let s = fillet.min(depth / 2.0).max(0.0);
    let r = radius.min(depth - s).min(width / 2.0).max(0.0);
    let left = cx - width / 2.0 - s;
    let span = 2.0 * s + width;
    let bottom = depth - inset;
    let mut out: Vec<(i32, i32, i32, i32)> = Vec::new();
    for y in 0..bottom.floor().max(0.0) as i32 {
        let v = y as f64 + 0.5;
        let edge = if v < s {
            ((s + inset).powi(2) - (s - v).powi(2)).max(0.0).sqrt()
        } else if v > depth - r {
            let k = (r - inset).max(0.0);
            s + r - (k * k - (v - (depth - r)).powi(2)).max(0.0).sqrt()
        } else {
            s + inset
        };
        let x0 = (left + edge).ceil() as i32;
        let x1 = (left + span - edge).floor() as i32;
        if x1 <= x0 {
            continue;
        }
        let row = (x0, (top as i32) + y, x1 - x0, 1);
        match out.last_mut() {
            Some(last) if last.0 == row.0 && last.2 == row.2 && last.1 + last.3 == row.1 => last.3 += 1,
            _ => out.push(row),
        }
    }
    out
}
