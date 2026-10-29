use gtk::gsk::{self, Path, PathBuilder};
use gtk::graphene;

pub fn docked(cx: f64, top: f64, width: f64, depth: f64, radius: f64, fillet: f64) -> Option<Path> {
    outline(cx, top, 1.0, width, depth, radius, fillet)
}

pub type Parts = (gsk::RoundedRect, Option<Path>);

pub fn docked_parts(cx: f64, top: f64, width: f64, depth: f64, radius: f64, fillet: f64) -> Option<Parts> {
    edge_parts(cx, top, 1.0, width, depth, radius, fillet)
}

pub fn island_parts(cx: f64, top: f64, width: f64, depth: f64, radius: f64, fillet: f64, top_radius: f64) -> Option<Parts> {
    let (body, coves) = edge_parts(cx, top, 1.0, width, depth, radius, fillet)?;
    let t = top_radius.min(depth / 2.0).min(width / 2.0).max(0.0) as f32;
    if t < 0.25 {
        return Some((body, coves));
    }
    let b = body.corner();
    let rounded = gsk::RoundedRect::new(*body.bounds(), graphene::Size::new(t, t), graphene::Size::new(t, t), b[2], b[3]);
    Some((rounded, coves))
}

pub fn docked_up_parts(cx: f64, bottom: f64, width: f64, depth: f64, radius: f64, fillet: f64) -> Option<Parts> {
    edge_parts(cx, bottom, -1.0, width, depth, radius, fillet)
}

fn edge_parts(cx: f64, edge: f64, dir: f64, width: f64, depth: f64, radius: f64, fillet: f64) -> Option<Parts> {
    if width < 1.0 || depth < 0.5 {
        return None;
    }
    let s = fillet.min(depth / 2.0).max(0.0);
    let r = radius.min(depth - s).min(width / 2.0).max(0.0) as f32;
    let (xl, xr) = (cx - width / 2.0, cx + width / 2.0);
    let y = if dir > 0.0 { edge } else { edge - depth };
    let bounds = graphene::Rect::new(xl as f32, y as f32, width as f32, depth as f32);
    let (zero, round) = (graphene::Size::zero(), graphene::Size::new(r, r));
    let body = if dir > 0.0 {
        gsk::RoundedRect::new(bounds, zero, zero, round, round)
    } else {
        gsk::RoundedRect::new(bounds, round, round, zero, zero)
    };
    if s < 0.5 {
        return Some((body, None));
    }
    let (s, e, d) = (s as f32, edge as f32, dir as f32);
    let (xl, xr) = (xl as f32, xr as f32);
    let down = dir > 0.0;
    let b = PathBuilder::new();
    b.move_to(xl - s, e);
    b.svg_arc_to(s, s, 0.0, false, down, xl, e + d * s);
    b.line_to(xl + 1.0, e + d * s);
    b.line_to(xl + 1.0, e);
    b.close();
    b.move_to(xr - 1.0, e);
    b.line_to(xr - 1.0, e + d * s);
    b.line_to(xr, e + d * s);
    b.svg_arc_to(s, s, 0.0, false, down, xr + s, e);
    b.close();
    Some((body, Some(b.to_path())))
}

pub fn cornered_parts(right: f64, bottom: f64, width: f64, height: f64, radius: f64, fillet: f64) -> Option<Parts> {
    if width < 1.0 || height < 0.5 {
        return None;
    }
    let s = fillet.min(height / 2.0).min(width / 2.0).max(0.0) as f32;
    let r = radius.min(height - s as f64).min(width - s as f64).max(0.0) as f32;
    let (left, top) = ((right - width) as f32, (bottom - height) as f32);
    let (right, bottom) = (right as f32, bottom as f32);
    let zero = graphene::Size::zero();
    let body = gsk::RoundedRect::new(graphene::Rect::new(left, top, width as f32, height as f32), graphene::Size::new(r, r), zero, zero, zero);
    if s < 0.5 {
        return Some((body, None));
    }
    let b = PathBuilder::new();
    b.move_to(right, top - s);
    b.svg_arc_to(s, s, 0.0, false, true, right - s, top);
    b.line_to(right - s, top + 1.0);
    b.line_to(right, top + 1.0);
    b.close();
    b.move_to(left + 1.0, bottom - s);
    b.line_to(left, bottom - s);
    b.svg_arc_to(s, s, 0.0, false, true, left - s, bottom);
    b.line_to(left + 1.0, bottom);
    b.close();
    Some((body, Some(b.to_path())))
}

pub fn docked_up(cx: f64, bottom: f64, width: f64, depth: f64, radius: f64, fillet: f64) -> Option<Path> {
    outline(cx, bottom, -1.0, width, depth, radius, fillet)
}

fn outline(cx: f64, edge: f64, dir: f64, width: f64, depth: f64, radius: f64, fillet: f64) -> Option<Path> {
    if width < 1.0 || depth < 0.5 {
        return None;
    }
    let s = fillet.min(depth / 2.0).max(0.0);
    let r = radius.min(depth - s).min(width / 2.0).max(0.0);
    let x0 = cx - width / 2.0 - s;
    let p = |u: f64, v: f64| ((x0 + u) as f32, (edge + dir * v) as f32);
    let up = dir < 0.0;
    let b = PathBuilder::new();
    let (x, y) = p(0.0, 0.0);
    b.move_to(x, y);
    let arc = |rad: f64, positive: bool, (x, y): (f32, f32)| b.svg_arc_to(rad as f32, rad as f32, 0.0, false, positive != up, x, y);
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
    b.close();
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
        let x0 = (left + edge).round() as i32;
        let x1 = (left + span - edge).round() as i32;
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

pub fn cornered(right: f64, bottom: f64, width: f64, height: f64, radius: f64, fillet: f64) -> Option<Path> {
    if width < 1.0 || height < 0.5 {
        return None;
    }
    let s = fillet.min(height / 2.0).min(width / 2.0).max(0.0);
    let r = radius.min(height - s).min(width - s).max(0.0);
    let (left, top) = (right - width, bottom - height);
    let f = |v: f64| v as f32;
    let b = PathBuilder::new();
    b.move_to(f(right), f(top - s));
    b.svg_arc_to(f(s), f(s), 0.0, false, true, f(right - s), f(top));
    b.line_to(f(left + r), f(top));
    b.svg_arc_to(f(r), f(r), 0.0, false, false, f(left), f(top + r));
    b.line_to(f(left), f(bottom - s));
    b.svg_arc_to(f(s), f(s), 0.0, false, true, f(left - s), f(bottom));
    b.line_to(f(right), f(bottom));
    b.close();
    Some(b.to_path())
}
