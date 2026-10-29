use crate::app::App;
use crate::compositor;
use gtk::{gdk, gio, glib, prelude::*};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Geo {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[derive(Default)]
struct Track {
    floating: HashMap<u64, Geo>,
    maxed: HashSet<u64>,
    saved: HashMap<u64, Geo>,
}

thread_local! {
    static TRACK: RefCell<Track> = RefCell::new(Track::default());
    static MODE: RefCell<(bool, Option<std::time::SystemTime>)> = const { RefCell::new((false, None)) };
}

pub fn enabled() -> bool {
    if !crate::compositor::niri_running() {
        return false;
    }
    let path = glib::user_config_dir().join("niri/digit-windows.kdl");
    let stamp = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    MODE.with(|m| {
        let mut m = m.borrow_mut();
        if m.1 != stamp || stamp.is_none() {
            m.0 = std::fs::read_to_string(&path).is_ok_and(|t| t.contains("open-floating true"));
            m.1 = stamp;
        }
        m.0
    })
}

fn pair(v: &Value, key: &str) -> Option<(f64, f64)> {
    let a = v.get(key)?.as_array()?;
    Some((a.first()?.as_f64()?, a.get(1)?.as_f64()?))
}

fn geo(layout: &Value) -> Option<Geo> {
    let (x, y) = pair(layout, "tile_pos_in_workspace_view")?;
    let (ox, oy) = pair(layout, "window_offset_in_tile").unwrap_or((0.0, 0.0));
    let (w, h) = pair(layout, "window_size")?;
    Some(Geo { x: x + ox, y: y + oy, w, h })
}

fn act(v: Value) {
    compositor::request(json!({ "Action": v }));
}

fn window(id: u64) -> Option<Value> {
    compositor::request(json!("Windows"))?.get("Windows")?.as_array()?.iter().find(|w| w.get("id").and_then(Value::as_u64) == Some(id)).cloned()
}

fn widest() -> f64 {
    gdk::Display::default().map(|d| d.monitors().iter::<gdk::Monitor>().filter_map(Result::ok).map(|m| m.geometry().width() as f64).fold(0.0, f64::max)).unwrap_or(1920.0)
}

fn screen() -> (f64, f64) {
    let output = compositor::focused_output();
    let monitors: Vec<gdk::Monitor> = gdk::Display::default().map(|d| d.monitors().iter::<gdk::Monitor>().filter_map(Result::ok).collect()).unwrap_or_default();
    let m = monitors.iter().find(|m| output.as_deref().is_some_and(|o| m.connector().as_deref() == Some(o))).or(monitors.first());
    m.map_or((1920.0, 1080.0), |m| (m.geometry().width() as f64, m.geometry().height() as f64))
}

pub fn opened(app: &Rc<App>, w: Value) {
    if !enabled() || w.get("is_floating").and_then(Value::as_bool) != Some(true) {
        return;
    }
    let Some(id) = w.get("id").and_then(Value::as_u64) else { return };
    let (sw, sh) = screen();
    let top = app.reserve().max(0) as f64;
    glib::spawn_future_local(async move {
        let _ = gio::spawn_blocking(move || {
            std::thread::sleep(Duration::from_millis(140));
            let Some(all) = compositor::request(json!("Windows")).and_then(|v| v.get("Windows").and_then(Value::as_array).cloned()) else { return };
            let Some(me) = all.iter().find(|x| x.get("id").and_then(Value::as_u64) == Some(id)) else { return };
            let Some(g) = me.get("layout").and_then(geo) else { return };
            let space = me.get("workspace_id").and_then(Value::as_u64);
            let others: Vec<Geo> = all
                .iter()
                .filter(|x| x.get("id").and_then(Value::as_u64) != Some(id) && x.get("workspace_id").and_then(Value::as_u64) == space && x.get("is_floating").and_then(Value::as_bool) == Some(true))
                .filter_map(|x| x.get("layout").and_then(geo))
                .collect();
            let (mut tx, mut ty) = (g.x, g.y);
            let mut k = 0;
            while others.iter().any(|o| (o.x - tx).abs() < 28.0 && (o.y - ty).abs() < 28.0) && k < 24 {
                tx += 36.0;
                ty += 36.0;
                k += 1;
                if tx + g.w > sw - 12.0 || ty + g.h > sh - 12.0 {
                    tx = 40.0 + (k % 6) as f64 * 14.0;
                    ty = top + 24.0 + (k % 6) as f64 * 14.0;
                }
            }
            if (tx - g.x).abs() > 0.5 || (ty - g.y).abs() > 0.5 {
                act(json!({ "MoveFloatingWindow": { "id": id, "x": { "AdjustFixed": tx - g.x }, "y": { "AdjustFixed": ty - g.y } } }));
            }
        })
        .await;
    });
}

pub fn changed(_app: &Rc<App>, w: Value) {
    let Some(id) = w.get("id").and_then(Value::as_u64) else { return };
    let floating = w.get("is_floating").and_then(Value::as_bool) == Some(true);
    let g = w.get("layout").and_then(geo);
    TRACK.with(|t| {
        let mut t = t.borrow_mut();
        if floating {
            if let Some(g) = g {
                t.floating.insert(id, g);
            }
            t.maxed.remove(&id);
        } else if enabled() && t.floating.contains_key(&id) {
            let size = w.get("layout").and_then(|l| pair(l, "window_size")).unwrap_or((0.0, 0.0));
            if size.0 >= widest() * 0.9 {
                t.maxed.insert(id);
            }
        }
    });
}

pub fn layouts(_app: &Rc<App>, changes: Value) {
    let Some(list) = changes.as_array() else { return };
    let wide = widest();
    for pair_v in list {
        let (Some(id), Some(layout)) = (pair_v.get(0).and_then(Value::as_u64), pair_v.get(1)) else { continue };
        let size = pair(layout, "window_size").unwrap_or((0.0, 0.0));
        let restore = TRACK.with(|t| {
            let mut t = t.borrow_mut();
            if let Some(g) = geo(layout) {
                if !t.maxed.contains(&id) {
                    t.floating.insert(id, g);
                }
                return None;
            }
            if !enabled() {
                return None;
            }
            if t.maxed.contains(&id) && size.0 > 0.0 && size.0 < wide * 0.88 {
                t.maxed.remove(&id);
                return t.floating.get(&id).copied();
            }
            if !t.maxed.contains(&id) && t.floating.contains_key(&id) && size.0 >= wide * 0.9 {
                t.maxed.insert(id);
            }
            None
        });
        if let Some(g) = restore {
            glib::spawn_future_local(async move {
                let _ = gio::spawn_blocking(move || place(id, g, true)).await;
            });
        }
    }
}

fn place(id: u64, g: Geo, float: bool) {
    if float {
        act(json!({ "MoveWindowToFloating": { "id": id } }));
        std::thread::sleep(Duration::from_millis(60));
    }
    act(json!({ "SetWindowWidth": { "id": id, "change": { "SetFixed": g.w.round() as i32 } } }));
    act(json!({ "SetWindowHeight": { "id": id, "change": { "SetFixed": g.h.round() as i32 } } }));
    std::thread::sleep(Duration::from_millis(60));
    if let Some(now) = window(id).and_then(|w| w.get("layout").and_then(geo)) {
        let (dx, dy) = (g.x - now.x, g.y - now.y);
        if dx.abs() > 0.5 || dy.abs() > 0.5 {
            act(json!({ "MoveFloatingWindow": { "id": id, "x": { "AdjustFixed": dx }, "y": { "AdjustFixed": dy } } }));
        }
    }
}

pub fn snap(app: &Rc<App>, which: &str) {
    if !crate::compositor::niri_running() {
        return;
    }
    let which = which.to_string();
    let (aw, ah) = crate::layouts::working_area(app);
    let (sw, sh) = screen();
    let (ox, oy) = (sw - aw, sh - ah);
    let top = app.reserve().max(0) as f64;
    let dock_left = {
        let d = app.prefs.borrow().dock.clone();
        d.mode == "on" && d.side == "left"
    };
    let x0 = if dock_left { ox } else { 0.0 };
    let y0 = top.min(oy);
    glib::spawn_future_local(async move {
        let Some((id, cur, floating)) = gio::spawn_blocking(|| {
            let w = compositor::request(json!("FocusedWindow"))?.get("FocusedWindow").cloned().filter(|v| !v.is_null())?;
            let id = w.get("id")?.as_u64()?;
            let floating = w.get("is_floating").and_then(Value::as_bool) == Some(true);
            Some((id, w.get("layout").and_then(geo), floating))
        })
        .await
        .ok()
        .flatten() else {
            return;
        };
        let gap = crate::layouts::niri_gap();
        let rect = |fx: f64, fy: f64, fw: f64, fh: f64| Geo {
            x: x0 + aw * fx + if fx > 0.0 { gap / 2.0 } else { gap },
            y: y0 + ah * fy + if fy > 0.0 { gap / 2.0 } else { gap },
            w: aw * fw - gap * 1.5,
            h: ah * fh - gap * 1.5,
        };
        let max = rect(0.0, 0.0, 1.0, 1.0);
        let near = |a: Geo, b: Geo| (a.x - b.x).abs() < 6.0 && (a.y - b.y).abs() < 6.0 && (a.w - b.w).abs() < 6.0 && (a.h - b.h).abs() < 6.0;
        let target = TRACK.with(|t| {
            let mut t = t.borrow_mut();
            let remember = |t: &mut Track| {
                if let Some(c) = cur {
                    t.saved.entry(id).or_insert(c);
                }
            };
            match which.as_str() {
                "max" => {
                    if cur.is_some_and(|c| near(c, max)) {
                        t.saved.remove(&id).or(Some(rect(0.15, 0.1, 0.7, 0.8)))
                    } else {
                        remember(&mut t);
                        Some(max)
                    }
                }
                "left" => {
                    remember(&mut t);
                    Some(rect(0.0, 0.0, 0.5, 1.0))
                }
                "right" => {
                    remember(&mut t);
                    Some(rect(0.5, 0.0, 0.5, 1.0))
                }
                _ => t.saved.remove(&id).or(Some(rect(0.15, 0.1, 0.7, 0.8))),
            }
        });
        if let Some(g) = target {
            let _ = gio::spawn_blocking(move || place(id, g, !floating)).await;
        }
    });
}
