use crate::capture::Shot;
use serde_json::Value;

pub type Rect = (f64, f64, f64, f64);

fn gaps() -> f64 {
    let path = kit::niriconf::dir().join("digit-compositor.kdl");
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut depth = 0i32;
    let mut in_layout = false;
    for line in text.lines() {
        let t = line.split("//").next().unwrap_or("").trim();
        if depth == 0 && t.starts_with("layout") {
            in_layout = true;
        }
        if in_layout && depth == 1 {
            if let Some(v) = t.strip_prefix("gaps") {
                if let Ok(g) = v.trim().trim_end_matches(';').parse::<f64>() {
                    return g;
                }
            }
        }
        depth += t.matches('{').count() as i32 - t.matches('}').count() as i32;
        if depth == 0 {
            in_layout = false;
        }
    }
    16.0
}

struct Lum<'a> {
    s: &'a Shot,
    k: f64,
}

impl Lum<'_> {
    fn at(&self, x: f64, y: f64) -> i32 {
        let px = ((x * self.k) as i32).clamp(0, self.s.pw - 1) as usize;
        let py = ((y * self.k) as i32).clamp(0, self.s.ph - 1) as usize;
        let i = py * self.s.stride as usize + px * 4;
        let d = &self.s.data;
        (d[i + 2] as i32 * 3 + d[i + 1] as i32 * 6 + d[i] as i32) / 10
    }
}

fn num(v: &Value, path: &[&str]) -> Option<f64> {
    let mut cur = v;
    for p in path {
        cur = cur.get(p)?;
    }
    cur.as_f64()
}

fn pair(v: &Value, key: &str) -> Option<(f64, f64)> {
    let a = v.get("layout")?.get(key)?.as_array()?;
    Some((a.first()?.as_f64()?, a.get(1)?.as_f64()?))
}

pub fn rects(shots: &[Shot]) -> Vec<Vec<Rect>> {
    if kit::wm::which() == kit::wm::Wm::Niri {
        return niri(shots);
    }
    let mut out = vec![Vec::new(); shots.len()];
    let outputs = kit::wm::outputs();
    let spaces = kit::wm::workspaces();
    let mut wins = kit::wm::windows();
    wins.sort_by_key(|w| (!w.focused, !w.floating));
    for w in wins {
        let Some((x, y, ww, hh)) = w.rect.filter(|r| r.2 > 1.0 && r.3 > 1.0) else { continue };
        if !spaces.is_empty() && !spaces.iter().any(|s| s.id == w.space && s.active) {
            continue;
        }
        let Some(si) = shots.iter().position(|s| s.name == w.output) else { continue };
        let origin = outputs.iter().find(|o| o.name == w.output).map(|o| (o.x, o.y)).unwrap_or((0.0, 0.0));
        out[si].push((x - origin.0, y - origin.1, ww, hh));
    }
    out
}

fn niri(shots: &[Shot]) -> Vec<Vec<Rect>> {
    let mut out = vec![Vec::new(); shots.len()];
    if !kit::niri::available() {
        return out;
    }
    let spaces = kit::niri::workspaces();
    let windows = kit::niri::windows();
    let gap = gaps();
    for (si, shot) in shots.iter().enumerate() {
        let Some(ws) = spaces.iter().find(|w| w.get("output").and_then(Value::as_str) == Some(shot.name.as_str()) && w.get("is_active").and_then(Value::as_bool) == Some(true)) else { continue };
        let wid = ws.get("id").and_then(Value::as_u64);
        let mine: Vec<&Value> = windows.iter().filter(|w| w.get("workspace_id").and_then(Value::as_u64) == wid).collect();
        let mut floating = Vec::new();
        let mut tiles: Vec<(i64, i64, f64, f64, (f64, f64), (f64, f64))> = Vec::new();
        for w in &mine {
            let tile = pair(w, "tile_size").unwrap_or((0.0, 0.0));
            let size = pair(w, "window_size").unwrap_or(tile);
            let off = pair(w, "window_offset_in_tile").unwrap_or((0.0, 0.0));
            if let Some(pos) = pair(w, "tile_pos_in_workspace_view") {
                floating.push((pos.0 + off.0, pos.1 + off.1, size.0, size.1));
                continue;
            }
            if w.get("is_floating").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            let col = num(w, &["layout", "pos_in_scrolling_layout", "0"]).or_else(|| w.get("layout")?.get("pos_in_scrolling_layout")?.as_array()?.first()?.as_f64()).unwrap_or(0.0) as i64;
            let row = w.get("layout").and_then(|l| l.get("pos_in_scrolling_layout")).and_then(|p| p.as_array()).and_then(|a| a.get(1)).and_then(Value::as_f64).unwrap_or(1.0) as i64;
            tiles.push((col, row, tile.0, tile.1, off, size));
        }
        tiles.sort_by_key(|t| (t.0, t.1));
        out[si].extend(fit(shot, &tiles, gap));
        out[si].extend(floating);
    }
    out
}

fn fit(shot: &Shot, tiles: &[(i64, i64, f64, f64, (f64, f64), (f64, f64))], gap: f64) -> Vec<Rect> {
    if tiles.is_empty() || shot.w <= 0 || shot.h <= 0 {
        return Vec::new();
    }
    let lum = Lum { s: shot, k: shot.pw as f64 / shot.w as f64 };
    let (w, h) = (shot.w as f64, shot.h as f64);
    let mut cols: Vec<(i64, f64, Vec<(f64, f64, (f64, f64), (f64, f64))>)> = Vec::new();
    for t in tiles {
        match cols.last_mut() {
            Some(c) if c.0 == t.0 => {
                c.1 = c.1.max(t.2);
                c.2.push((t.2, t.3, t.4, t.5));
            }
            _ => cols.push((t.0, t.2, vec![(t.2, t.3, t.4, t.5)])),
        }
    }
    let height = |c: &(i64, f64, Vec<(f64, f64, (f64, f64), (f64, f64))>)| c.2.iter().map(|t| t.1).sum::<f64>() + gap * (c.2.len() as f64 - 1.0);
    let tall = cols.iter().map(height).fold(0.0f64, f64::max);
    if tall <= 0.0 || tall > h {
        return Vec::new();
    }
    let rows: Vec<f64> = (0..h as i32).map(|y| {
        if y == 0 {
            return 0.0;
        }
        let mut s = 0i64;
        let mut n = 0i64;
        let mut x = 2.0;
        while x < w - 2.0 {
            s += (lum.at(x, y as f64) - lum.at(x, y as f64 - 1.0)).abs() as i64;
            n += 1;
            x += 6.0;
        }
        s as f64 / n.max(1) as f64
    }).collect();
    let row = |y: f64| rows.get(y.round() as usize).copied().unwrap_or(0.0);
    let mut best_y = (0.0, -1.0);
    let mut y0 = 0.0;
    while y0 + tall <= h {
        let s = row(y0) + row(y0 + tall);
        if s > best_y.1 {
            best_y = (y0, s);
        }
        y0 += 1.0;
    }
    let y0 = best_y.0;
    let (ya, yb) = (y0 + 12.0, y0 + tall - 12.0);
    let colp: Vec<f64> = (0..w as i32).map(|x| {
        if x == 0 {
            return 0.0;
        }
        let mut s = 0i64;
        let mut n = 0i64;
        let mut y = ya;
        while y < yb {
            s += (lum.at(x as f64, y) - lum.at(x as f64 - 1.0, y)).abs() as i64;
            n += 1;
            y += 5.0;
        }
        s as f64 / n.max(1) as f64
    }).collect();
    let edge = |x: f64| -> Option<f64> {
        let xi = x.round();
        if xi < 1.0 || xi >= w - 1.0 {
            return None;
        }
        let i = xi as usize;
        Some(colp[i].max(colp[i - 1]).max(colp.get(i + 1).copied().unwrap_or(0.0)))
    };
    let total: f64 = cols.iter().map(|c| c.1).sum::<f64>() + gap * (cols.len() as f64 - 1.0);
    let mut best = (0.0, -1.0);
    let mut x0 = -total;
    while x0 <= w {
        let mut s = 0.0;
        let mut n = 0;
        let mut x = x0;
        for c in &cols {
            for e in [x, x + c.1] {
                if let Some(v) = edge(e) {
                    s += v;
                    n += 1;
                }
            }
            x += c.1 + gap;
        }
        if n >= 2 {
            let score = s / n as f64;
            if score > best.1 {
                best = (x0, score);
            }
        }
        x0 += 1.0;
    }
    let mut sorted = colp.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = sorted.get(sorted.len() / 2).copied().unwrap_or(0.0);
    if best.1 < 10.0 || best.1 < median * 4.0 {
        return Vec::new();
    }
    let mut rects = Vec::new();
    let mut x = best.0;
    for c in &cols {
        let mut y = y0 + (tall - height(c)) / 2.0 * 0.0;
        for t in &c.2 {
            let (rx, ry, rw, rh) = (x + t.2.0, y + t.2.1, t.3.0, t.3.1);
            if rx + rw > 0.0 && rx < w && ry + rh > 0.0 && ry < h {
                let cx = rx.max(0.0);
                let cy = ry.max(0.0);
                rects.push((cx, cy, (rx + rw).min(w) - cx, (ry + rh).min(h) - cy));
            }
            y += t.1 + gap;
        }
        x += c.1 + gap;
    }
    rects
}
