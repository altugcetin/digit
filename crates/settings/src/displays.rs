use crate::ui;
use gtk::{cairo, glib, prelude::*};
use serde_json::Value;
use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Mode {
    w: i32,
    h: i32,
    mhz: i32,
}

impl Mode {
    fn hz(&self) -> String {
        let hz = self.mhz as f64 / 1000.0;
        if (hz - hz.round()).abs() < 0.05 { format!("{:.0} Hz", hz.round()) } else { format!("{hz:.2} Hz") }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Out {
    name: String,
    label: String,
    modes: Vec<Mode>,
    mode: Mode,
    x: i32,
    y: i32,
    scale: f64,
    turn: u8,
    flip: bool,
    on: bool,
}

impl Out {
    fn size(&self) -> (f64, f64) {
        let (w, h) = (self.mode.w as f64 / self.scale, self.mode.h as f64 / self.scale);
        if self.turn % 2 == 1 { (h, w) } else { (w, h) }
    }

    fn niri_transform(&self) -> String {
        let base = ["normal", "90", "180", "270"][self.turn as usize % 4];
        match (self.flip, self.turn) {
            (false, _) => base.to_string(),
            (true, 0) => "flipped".into(),
            (true, _) => format!("flipped-{base}"),
        }
    }

    fn hypr_transform(&self) -> u8 {
        self.turn % 4 + if self.flip { 4 } else { 0 }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Wm {
    Niri,
    Hypr,
    Other,
}

fn run(argv: &[&str]) -> Option<String> {
    let out = Command::new(argv[0]).args(&argv[1..]).stderr(std::process::Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn read() -> (Wm, Vec<Out>) {
    if std::env::var_os("NIRI_SOCKET").is_some() {
        let Some(v) = run(&["niri", "msg", "-j", "outputs"]).and_then(|t| serde_json::from_str::<Value>(&t).ok()) else { return (Wm::Niri, Vec::new()) };
        let mut outs: Vec<Out> = v
            .as_object()
            .map(|m| {
                m.values()
                    .filter_map(|o| {
                        let name = o.get("name")?.as_str()?.to_string();
                        let label = format!("{} {}", o.get("make").and_then(Value::as_str).unwrap_or(""), o.get("model").and_then(Value::as_str).unwrap_or("")).trim().to_string();
                        let modes: Vec<Mode> = o.get("modes")?.as_array()?.iter().filter_map(|m| Some(Mode { w: m.get("width")?.as_i64()? as i32, h: m.get("height")?.as_i64()? as i32, mhz: m.get("refresh_rate")?.as_i64()? as i32 })).collect();
                        let logical = o.get("logical");
                        let on = logical.is_some_and(|l| !l.is_null());
                        let mode = o.get("current_mode").and_then(Value::as_u64).and_then(|i| modes.get(i as usize).copied()).or_else(|| modes.first().copied())?;
                        let t = logical.and_then(|l| l.get("transform")).and_then(Value::as_str).unwrap_or("Normal");
                        let turn = if t.contains("90") { 1 } else if t.contains("180") { 2 } else if t.contains("270") { 3 } else { 0 };
                        Some(Out {
                            name,
                            label,
                            modes,
                            mode,
                            x: logical.and_then(|l| l.get("x")).and_then(Value::as_i64).unwrap_or(0) as i32,
                            y: logical.and_then(|l| l.get("y")).and_then(Value::as_i64).unwrap_or(0) as i32,
                            scale: logical.and_then(|l| l.get("scale")).and_then(Value::as_f64).unwrap_or(1.0),
                            turn,
                            flip: t.starts_with("Flipped"),
                            on,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        outs.sort_by_key(|o| (o.x, o.y));
        return (Wm::Niri, outs);
    }
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        let Some(v) = run(&["hyprctl", "-j", "monitors", "all"]).and_then(|t| serde_json::from_str::<Value>(&t).ok()) else { return (Wm::Hypr, Vec::new()) };
        let mut outs: Vec<Out> = v
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|o| {
                        let name = o.get("name")?.as_str()?.to_string();
                        let label = format!("{} {}", o.get("make").and_then(Value::as_str).unwrap_or(""), o.get("model").and_then(Value::as_str).unwrap_or("")).trim().to_string();
                        let mut modes: Vec<Mode> = o
                            .get("availableModes")
                            .and_then(Value::as_array)
                            .map(|l| {
                                l.iter()
                                    .filter_map(|m| {
                                        let s = m.as_str()?;
                                        let (res, hz) = s.trim_end_matches("Hz").split_once('@')?;
                                        let (w, h) = res.split_once('x')?;
                                        Some(Mode { w: w.parse().ok()?, h: h.parse().ok()?, mhz: (hz.parse::<f64>().ok()? * 1000.0).round() as i32 })
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        let current = Mode { w: o.get("width")?.as_i64()? as i32, h: o.get("height")?.as_i64()? as i32, mhz: (o.get("refreshRate")?.as_f64()? * 1000.0).round() as i32 };
                        let mode = modes.iter().copied().min_by_key(|m| (m.w - current.w).abs() + (m.h - current.h).abs() + (m.mhz - current.mhz).abs() / 100).unwrap_or(current);
                        if modes.is_empty() {
                            modes.push(current);
                        }
                        let t = o.get("transform").and_then(Value::as_u64).unwrap_or(0) as u8;
                        Some(Out {
                            name,
                            label,
                            modes,
                            mode,
                            x: o.get("x").and_then(Value::as_i64).unwrap_or(0) as i32,
                            y: o.get("y").and_then(Value::as_i64).unwrap_or(0) as i32,
                            scale: o.get("scale").and_then(Value::as_f64).unwrap_or(1.0),
                            turn: t % 4,
                            flip: t >= 4,
                            on: !o.get("disabled").and_then(Value::as_bool).unwrap_or(false),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        outs.sort_by_key(|o| (o.x, o.y));
        return (Wm::Hypr, outs);
    }
    (Wm::Other, Vec::new())
}

fn trim(v: f64) -> String {
    let s = format!("{v:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn apply_now(wm: Wm, outs: &[Out]) -> Result<(), String> {
    if wm != Wm::Niri {
        return Err("Display settings work on niri and Hyprland".into());
    }
    for o in outs.iter().filter(|o| o.on) {
        let mode = format!("{}x{}@{:.3}", o.mode.w, o.mode.h, o.mode.mhz as f64 / 1000.0);
        let (x, y) = (o.x.to_string(), o.y.to_string());
        for argv in [
            vec!["niri", "msg", "output", &o.name, "on"],
            vec!["niri", "msg", "output", &o.name, "mode", &mode],
            vec!["niri", "msg", "output", &o.name, "scale", &trim(o.scale)],
            vec!["niri", "msg", "output", &o.name, "transform", &o.niri_transform()],
            vec!["niri", "msg", "output", &o.name, "position", "set", &x, &y],
        ] {
            run(&argv).ok_or_else(|| format!("niri refused: {}", argv[3..].join(" ")))?;
        }
    }
    for o in outs.iter().filter(|o| !o.on) {
        run(&["niri", "msg", "output", &o.name, "off"]);
    }
    Ok(())
}

fn stamp() -> String {
    glib::DateTime::now_local().and_then(|d| d.format("%Y%m%d-%H%M%S")).map(|s| s.to_string()).unwrap_or_else(|_| "backup".into())
}

// hl.monitor({...}) blocks in an existing monitors.lua, keyed by output, so
// rules for screens that are unplugged right now survive a rewrite.
fn lua_monitor_blocks(text: &str) -> Vec<(&str, &str)> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("hl.monitor(") {
        let commented = rest[..i].rsplit('\n').next().is_some_and(|l| l.trim_start().starts_with("--"));
        let tail = &rest[i..];
        let Some(j) = tail.find("})") else { break };
        let block = &tail[..j + 2];
        if let Some(name) = block.split_once("output").and_then(|(_, s)| s.split('"').nth(1)).filter(|_| !commented) {
            found.push((name, block));
        }
        rest = &tail[j + 2..];
    }
    found
}

fn hypr_lua(outs: &[Out], old: &str) -> String {
    let mut text = String::from("-- Generated by Digit Settings (Displays). Changes made there replace this file.\n\n");
    for o in outs {
        let mut props = vec![format!("    output = \"{}\"", o.name)];
        if o.on {
            props.push(format!("    mode = \"{}x{}@{:.3}\"", o.mode.w, o.mode.h, o.mode.mhz as f64 / 1000.0));
            props.push(format!("    position = \"{}x{}\"", o.x, o.y));
            props.push(format!("    scale = {}", trim(o.scale)));
            if o.hypr_transform() != 0 {
                props.push(format!("    transform = {}", o.hypr_transform()));
            }
        } else {
            props.push("    disabled = true".into());
        }
        text.push_str(&format!("hl.monitor({{\n{}\n}})\n", props.join(",\n")));
    }
    for (name, block) in lua_monitor_blocks(old) {
        if !outs.iter().any(|o| o.name == name) {
            text.push_str(block);
            text.push('\n');
        }
    }
    text
}

fn has_require(text: &str, module: &str) -> bool {
    text.lines().any(|l| {
        let code = l.split("--").next().unwrap_or("").replace([' ', '\t', '(', ')'], "").replace('\'', "\"");
        code == format!("require\"{module}\"")
    })
}

// Hyprland watches every require()'d file and reloads by itself, unless the
// user turned that off.
fn hypr_reload_if_needed() {
    let off = run(&["hyprctl", "-j", "getoption", "misc:disable_autoreload"])
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.get("int").and_then(Value::as_i64))
        .is_some_and(|v| v != 0);
    if off {
        run(&["hyprctl", "reload"]);
    }
}

/// Writes ~/.config/hypr/monitors.lua and makes hyprland.lua require it.
/// Returns what monitors.lua held before, for Revert.
fn hypr_write(outs: &[Out]) -> Result<Option<String>, String> {
    let dir = glib::user_config_dir().join("hypr");
    let main = dir.join("hyprland.lua");
    let current = std::fs::read_to_string(&main).map_err(|_| "Display settings need Hyprland's Lua config (~/.config/hypr/hyprland.lua)".to_string())?;
    let s = stamp();
    let monitors = dir.join("monitors.lua");
    let old = std::fs::read_to_string(&monitors).ok();
    if old.is_some() {
        std::fs::copy(&monitors, dir.join(format!("monitors.lua.bak-{s}"))).map_err(|e| format!("Could not back up monitors.lua: {e}"))?;
    }
    save_atomic(&monitors, &hypr_lua(outs, old.as_deref().unwrap_or(""))).map_err(|e| e.to_string())?;
    let workspaces = dir.join("workspaces.lua");
    if !workspaces.exists() {
        save_atomic(&workspaces, "-- Workspace rules, e.g. hl.workspace_rule({ workspace = \"1\", monitor = \"DP-1\" })\n").map_err(|e| e.to_string())?;
    }
    let missing: Vec<&str> = ["monitors", "workspaces"].into_iter().filter(|m| !has_require(&current, m)).collect();
    if !missing.is_empty() {
        std::fs::copy(&main, dir.join(format!("hyprland.lua.bak-{s}"))).map_err(|e| format!("Could not back up hyprland.lua: {e}"))?;
        let lines: String = missing.iter().map(|m| format!("require(\"{m}\")\n")).collect();
        save_atomic(&main, &format!("{}\n{lines}", current.trim_end())).map_err(|e| e.to_string())?;
    }
    hypr_reload_if_needed();
    Ok(old)
}

fn hypr_restore(old: Option<String>) {
    let path = glib::user_config_dir().join("hypr").join("monitors.lua");
    match old {
        Some(text) => {
            let _ = save_atomic(&path, &text);
        }
        None => {
            let _ = std::fs::remove_file(path);
        }
    }
    hypr_reload_if_needed();
}

fn hypr_error() -> Option<String> {
    run(&["hyprctl", "configerrors"])?.lines().map(str::trim).find(|l| ["monitors.lua", "workspaces.lua", "hl.monitor", "\"monitors\"", "\"workspaces\""].iter().any(|k| l.contains(k))).map(str::to_string)
}

fn save_atomic(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("new");
    std::fs::write(&tmp, text)?;
    std::fs::rename(tmp, path)
}

fn persist(wm: Wm, outs: &[Out]) -> Result<String, String> {
    let conf = glib::user_config_dir();
    match wm {
        Wm::Niri => {
            let dir = conf.join("niri");
            let mut text = String::new();
            for o in outs {
                if o.on {
                    text.push_str(&format!(
                        "output \"{}\" {{\n    mode \"{}x{}@{:.3}\"\n    scale {}\n    transform \"{}\"\n    position x={} y={}\n}}\n\n",
                        o.name,
                        o.mode.w,
                        o.mode.h,
                        o.mode.mhz as f64 / 1000.0,
                        trim(o.scale),
                        o.niri_transform(),
                        o.x,
                        o.y
                    ));
                } else {
                    text.push_str(&format!("output \"{}\" {{\n    off\n}}\n\n", o.name));
                }
            }
            let main = dir.join("config.kdl");
            let current = std::fs::read_to_string(&main).map_err(|e| format!("Could not read config.kdl: {e}"))?;
            let line = "include optional=true \"digit-outputs.kdl\"";
            let needs = !current.lines().any(|l| l.trim() == line);
            let test = glib::user_cache_dir().join("digit").join("niri-check");
            let _ = std::fs::remove_dir_all(&test);
            std::fs::create_dir_all(&test).map_err(|e| e.to_string())?;
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for e in entries.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "kdl")) {
                    let _ = std::fs::copy(e.path(), test.join(e.file_name()));
                }
            }
            std::fs::write(test.join("digit-outputs.kdl"), &text).map_err(|e| e.to_string())?;
            if needs {
                std::fs::write(test.join("config.kdl"), format!("{}\n{line}\n", current.trim_end())).map_err(|e| e.to_string())?;
            }
            let ok = Command::new("niri").args(["validate", "-c"]).arg(test.join("config.kdl")).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success());
            let _ = std::fs::remove_dir_all(&test);
            if !ok {
                return Err("niri did not accept the saved display settings; they stay active until you log out".into());
            }
            save_atomic(&dir.join("digit-outputs.kdl"), &text).map_err(|e| e.to_string())?;
            if needs {
                let _ = std::fs::copy(&main, dir.join("config.kdl.before-displays"));
                save_atomic(&main, &format!("{}\n{line}\n", current.trim_end())).map_err(|e| e.to_string())?;
            }
            Ok("Saved to ~/.config/niri/digit-outputs.kdl".into())
        }
        Wm::Hypr => Ok("Saved to ~/.config/hypr/monitors.lua".into()),
        Wm::Other => Err("Display settings work on niri and Hyprland".into()),
    }
}

struct State {
    wm: Wm,
    outs: Vec<Out>,
    saved: Vec<Out>,
    pick: usize,
    drag: Option<(usize, i32, i32)>,
    fit: (f64, f64, f64),
    pending: Option<glib::SourceId>,
    undo: Option<Option<String>>,
}

fn normalize(outs: &mut [Out]) {
    let on: Vec<&Out> = outs.iter().filter(|o| o.on).collect();
    let (mx, my) = (on.iter().map(|o| o.x).min().unwrap_or(0), on.iter().map(|o| o.y).min().unwrap_or(0));
    for o in outs.iter_mut() {
        o.x -= mx;
        o.y -= my;
    }
}

fn snap(outs: &mut [Out], i: usize) {
    let (rw, rh) = outs[i].size();
    let (rx, ry) = (outs[i].x as f64, outs[i].y as f64);
    let mut best: Option<(f64, f64, f64)> = None;
    for (j, o) in outs.iter().enumerate() {
        if j == i || !o.on {
            continue;
        }
        let (ow, oh) = o.size();
        let (ox, oy) = (o.x as f64, o.y as f64);
        let align = |v: f64, lo: f64, len: f64, own: f64| {
            let v = v.clamp(lo - own + 1.0, lo + len - 1.0);
            [lo, lo + len - own].into_iter().find(|t| (v - t).abs() < 80.0).unwrap_or(v)
        };
        let ya = align(ry, oy, oh, rh);
        let xa = align(rx, ox, ow, rw);
        for (cx, cy) in [(ox + ow, ya), (ox - rw, ya), (xa, oy + oh), (xa, oy - rh)] {
            let overlaps = outs.iter().enumerate().any(|(k, p)| {
                if k == i || !p.on {
                    return false;
                }
                let (pw, ph) = p.size();
                cx < p.x as f64 + pw && cx + rw > p.x as f64 && cy < p.y as f64 + ph && cy + rh > p.y as f64
            });
            if overlaps {
                continue;
            }
            let d = (cx - rx).powi(2) + (cy - ry).powi(2);
            if best.is_none_or(|b| d < b.2) {
                best = Some((cx, cy, d));
            }
        }
    }
    if let Some((x, y, _)) = best {
        outs[i].x = x.round() as i32;
        outs[i].y = y.round() as i32;
    }
    normalize(outs);
}

fn hit(st: &State, x: f64, y: f64) -> Option<usize> {
    let (s, ox, oy) = st.fit;
    st.outs.iter().enumerate().rev().find(|(_, o)| {
        let (w, h) = o.size();
        let (rx, ry) = (ox + o.x as f64 * s, oy + o.y as f64 * s);
        o.on && x >= rx && x <= rx + w * s && y >= ry && y <= ry + h * s
    }).map(|(i, _)| i)
}

fn draw(st: &mut State, cr: &cairo::Context, w: f64, h: f64, accent: (f64, f64, f64), ink: (f64, f64, f64)) {
    let on: Vec<&Out> = st.outs.iter().filter(|o| o.on).collect();
    if on.is_empty() {
        return;
    }
    let maxx = on.iter().map(|o| o.x as f64 + o.size().0).fold(0.0, f64::max);
    let maxy = on.iter().map(|o| o.y as f64 + o.size().1).fold(0.0, f64::max);
    let s = ((w - 48.0) / maxx).min((h - 40.0) / maxy).min(0.25);
    let (ox, oy) = ((w - maxx * s) / 2.0, (h - maxy * s) / 2.0);
    st.fit = (s, ox, oy);
    for (i, o) in st.outs.iter().enumerate().filter(|(_, o)| o.on) {
        let (ow, oh) = o.size();
        let (x, y, rw, rh) = (ox + o.x as f64 * s + 2.0, oy + o.y as f64 * s + 2.0, ow * s - 4.0, oh * s - 4.0);
        let r = 10.0;
        cr.new_sub_path();
        cr.arc(x + rw - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
        cr.arc(x + rw - r, y + rh - r, r, 0.0, std::f64::consts::FRAC_PI_2);
        cr.arc(x + r, y + rh - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
        cr.arc(x + r, y + r, r, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
        cr.close_path();
        let picked = i == st.pick;
        if picked {
            cr.set_source_rgba(accent.0, accent.1, accent.2, 0.85);
        } else {
            cr.set_source_rgba(ink.0, ink.1, ink.2, 0.12);
        }
        let _ = cr.fill_preserve();
        cr.set_source_rgba(ink.0, ink.1, ink.2, if picked { 0.0 } else { 0.25 });
        cr.set_line_width(1.0);
        let _ = cr.stroke();
        let text = if picked { (0.05, 0.05, 0.05) } else { ink };
        cr.set_source_rgb(text.0, text.1, text.2);
        cr.select_font_face("Readex Pro", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
        cr.set_font_size(14.0);
        if let Ok(ext) = cr.text_extents(&o.name) {
            cr.move_to(x + (rw - ext.width()) / 2.0, y + rh / 2.0);
            let _ = cr.show_text(&o.name);
        }
        cr.select_font_face("Readex Pro", cairo::FontSlant::Normal, cairo::FontWeight::Normal);
        cr.set_font_size(11.5);
        let sub = format!("{}×{} · {}", o.mode.w, o.mode.h, o.mode.hz());
        if let Ok(ext) = cr.text_extents(&sub) {
            cr.move_to(x + (rw - ext.width()) / 2.0, y + rh / 2.0 + 18.0);
            let _ = cr.show_text(&sub);
        }
    }
}

fn rgb(c: gtk::gdk::RGBA) -> (f64, f64, f64) {
    (c.red() as f64, c.green() as f64, c.blue() as f64)
}

pub fn build() -> gtk::Widget {
    let (scroll, body) = ui::page("Displays", "Drag screens to arrange them, then choose resolution, refresh rate, scale and rotation. New settings undo themselves after 15 seconds unless you keep them.");
    let (wm, outs) = read();
    if wm == Wm::Other || outs.is_empty() {
        let card = ui::card();
        let (row, _) = ui::row(Some("monitor"), "Displays", "Display settings work on niri and Hyprland.");
        card.append(&row);
        body.append(&card);
        return scroll.upcast();
    }
    let st = Rc::new(RefCell::new(State { wm, saved: outs.clone(), outs, pick: 0, drag: None, fit: (0.1, 0.0, 0.0), pending: None, undo: None }));
    let area = gtk::DrawingArea::new();
    area.set_content_height(250);
    area.set_hexpand(true);
    let card = ui::card();
    card.append(&area);
    body.append(&card);
    let detail = ui::card();
    body.append(&detail);
    let confirm = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    confirm.add_css_class("row");
    let ask = kit::ui::text("", "name");
    ask.set_hexpand(true);
    let revert = ui::button("Revert", "");
    let keep = ui::button("Keep Changes", "accent");
    confirm.append(&ask);
    confirm.append(&revert);
    confirm.append(&keep);
    let bar = ui::card();
    bar.append(&confirm);
    bar.set_visible(false);
    body.append(&bar);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_halign(gtk::Align::End);
    actions.set_margin_top(4);
    let reset = ui::button("Reset", "");
    let apply = ui::button("Apply", "accent");
    actions.append(&reset);
    actions.append(&apply);
    body.append(&actions);

    let refill: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    {
        let (st, area) = (st.clone(), area.clone());
        let widget = area.clone();
        area.set_draw_func(move |_, cr, w, h| {
            #[allow(deprecated)]
            let accent = widget.style_context().lookup_color("accent_bg_color").map(rgb).unwrap_or((0.45, 0.65, 1.0));
            let ink = rgb(widget.color());
            if let Ok(mut s) = st.try_borrow_mut() {
                draw(&mut s, cr, w as f64, h as f64, accent, ink);
            }
        });
    }
    let fill = {
        let (st, detail, area, refill) = (st.clone(), detail.clone(), area.clone(), refill.clone());
        Rc::new(move || {
            kit::ui::clear(&detail);
            let (o, count_on) = {
                let s = st.borrow();
                (s.outs.get(s.pick).cloned(), s.outs.iter().filter(|o| o.on).count())
            };
            let Some(o) = o else { return };
            let head = kit::ui::text(&format!("{}  ·  {}", o.name, o.label), "name");
            head.set_margin_start(12);
            head.set_margin_top(8);
            head.set_margin_bottom(4);
            detail.append(&head);
            let (row, end) = ui::row(Some("power_settings_new"), "Use this display", "");
            let sw = gtk::Switch::new();
            sw.set_active(o.on);
            sw.set_valign(gtk::Align::Center);
            sw.set_sensitive(!(o.on && count_on <= 1));
            let (s2, a2, r2) = (st.clone(), area.clone(), refill.clone());
            sw.connect_active_notify(move |w| {
                {
                    let mut s = s2.borrow_mut();
                    let i = s.pick;
                    s.outs[i].on = w.is_active();
                    normalize(&mut s.outs);
                }
                a2.queue_draw();
                if let Some(f) = r2.borrow().clone() {
                    glib::idle_add_local_once(move || f());
                }
            });
            end.append(&sw);
            detail.append(&row);
            if !o.on {
                return;
            }
            let mut sizes: Vec<(i32, i32)> = Vec::new();
            for m in &o.modes {
                if !sizes.contains(&(m.w, m.h)) {
                    sizes.push((m.w, m.h));
                }
            }
            sizes.sort_by_key(|(w, h)| std::cmp::Reverse(w * h));
            let labels: Vec<String> = sizes.iter().map(|(w, h)| format!("{w} × {h}")).collect();
            let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
            let at = sizes.iter().position(|s| *s == (o.mode.w, o.mode.h)).unwrap_or(0);
            let (row, end) = ui::row(Some("aspect_ratio"), "Resolution", "");
            let pick = kit::ui::dropdown(&refs, at as u32);
            let (s2, a2, r2) = (st.clone(), area.clone(), refill.clone());
            pick.connect_selected_notify(move |p| {
                let Some((w, h)) = sizes.get(p.selected() as usize).copied() else { return };
                {
                    let mut s = s2.borrow_mut();
                    let i = s.pick;
                    let best = s.outs[i].modes.iter().filter(|m| m.w == w && m.h == h).max_by_key(|m| m.mhz).copied();
                    if let Some(m) = best {
                        s.outs[i].mode = m;
                    }
                    snap(&mut s.outs, i);
                }
                a2.queue_draw();
                if let Some(f) = r2.borrow().clone() {
                    glib::idle_add_local_once(move || f());
                }
            });
            end.append(&pick);
            detail.append(&row);
            let mut rates: Vec<Mode> = o.modes.iter().filter(|m| m.w == o.mode.w && m.h == o.mode.h).copied().collect();
            rates.sort_by_key(|m| std::cmp::Reverse(m.mhz));
            rates.dedup_by_key(|m| m.mhz);
            let labels: Vec<String> = rates.iter().map(Mode::hz).collect();
            let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
            let at = rates.iter().position(|m| m.mhz == o.mode.mhz).unwrap_or(0);
            let (row, end) = ui::row(Some("speed"), "Refresh rate", "");
            let pick = kit::ui::dropdown(&refs, at as u32);
            let (s2, a2) = (st.clone(), area.clone());
            pick.connect_selected_notify(move |p| {
                let Some(m) = rates.get(p.selected() as usize).copied() else { return };
                let mut s = s2.borrow_mut();
                let i = s.pick;
                s.outs[i].mode = m;
                drop(s);
                a2.queue_draw();
            });
            end.append(&pick);
            detail.append(&row);
            let mut scales = vec![0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0];
            if !scales.iter().any(|v: &f64| (v - o.scale).abs() < 0.001) {
                scales.push(o.scale);
                scales.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            }
            let labels: Vec<String> = scales.iter().map(|v| format!("{}%", (v * 100.0).round())).collect();
            let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
            let at = scales.iter().position(|v| (v - o.scale).abs() < 0.001).unwrap_or(1);
            let (row, end) = ui::row(Some("zoom_in"), "Scale", "Bigger text and controls at higher values");
            let pick = kit::ui::dropdown(&refs, at as u32);
            let (s2, a2) = (st.clone(), area.clone());
            pick.connect_selected_notify(move |p| {
                let Some(v) = scales.get(p.selected() as usize).copied() else { return };
                let mut s = s2.borrow_mut();
                let i = s.pick;
                s.outs[i].scale = v;
                snap(&mut s.outs, i);
                drop(s);
                a2.queue_draw();
            });
            end.append(&pick);
            detail.append(&row);
            let (row, end) = ui::row(Some("screen_rotation"), "Rotation", "");
            let pick = kit::ui::dropdown(&["Normal", "90°", "180°", "270°"], o.turn as u32 % 4);
            let (s2, a2) = (st.clone(), area.clone());
            pick.connect_selected_notify(move |p| {
                let mut s = s2.borrow_mut();
                let i = s.pick;
                s.outs[i].turn = p.selected() as u8 % 4;
                snap(&mut s.outs, i);
                drop(s);
                a2.queue_draw();
            });
            end.append(&pick);
            detail.append(&row);
        })
    };
    *refill.borrow_mut() = Some(fill.clone());
    fill();
    let drag = gtk::GestureDrag::new();
    {
        let (s2, a2, f2) = (st.clone(), area.clone(), fill.clone());
        drag.connect_drag_begin(move |_, x, y| {
            let mut s = s2.borrow_mut();
            if let Some(i) = hit(&s, x, y) {
                let changed = s.pick != i;
                s.pick = i;
                s.drag = Some((i, s.outs[i].x, s.outs[i].y));
                drop(s);
                a2.queue_draw();
                if changed {
                    f2();
                }
            }
        });
    }
    {
        let (s2, a2) = (st.clone(), area.clone());
        drag.connect_drag_update(move |_, dx, dy| {
            let mut s = s2.borrow_mut();
            let Some((i, x0, y0)) = s.drag else { return };
            let k = s.fit.0.max(0.0001);
            s.outs[i].x = x0 + (dx / k).round() as i32;
            s.outs[i].y = y0 + (dy / k).round() as i32;
            drop(s);
            a2.queue_draw();
        });
    }
    {
        let (s2, a2) = (st.clone(), area.clone());
        drag.connect_drag_end(move |_, _, _| {
            let mut s = s2.borrow_mut();
            if let Some((i, _, _)) = s.drag.take() {
                snap(&mut s.outs, i);
            }
            drop(s);
            a2.queue_draw();
        });
    }
    area.add_controller(drag);
    {
        let (s2, a2, f2) = (st.clone(), area.clone(), fill.clone());
        reset.connect_clicked(move |_| {
            let (_, outs) = read();
            {
                let mut s = s2.borrow_mut();
                s.saved = outs.clone();
                s.outs = outs;
                s.pick = s.pick.min(s.outs.len().saturating_sub(1));
            }
            a2.queue_draw();
            f2();
        });
    }
    let finish = {
        let (s2, a2, f2, bar, apply) = (st.clone(), area.clone(), fill.clone(), bar.clone(), apply.clone());
        Rc::new(move |keep: bool| {
            let (wm, outs, saved, pending, undo) = {
                let mut s = s2.borrow_mut();
                (s.wm, s.outs.clone(), s.saved.clone(), s.pending.take(), s.undo.take())
            };
            if let Some(id) = pending {
                id.remove();
            }
            bar.set_visible(false);
            apply.set_sensitive(true);
            if keep {
                match persist(wm, &outs) {
                    Ok(msg) => ui::done(&msg),
                    Err(e) => ui::fail(&e),
                }
                s2.borrow_mut().saved = outs;
            } else {
                match (wm, undo) {
                    (Wm::Hypr, Some(old)) => hypr_restore(old),
                    _ => {
                        let _ = apply_now(wm, &saved);
                    }
                }
                s2.borrow_mut().outs = saved;
                ui::done("Display settings restored");
            }
            a2.queue_draw();
            f2();
        })
    };
    {
        let f = finish.clone();
        keep.connect_clicked(move |_| f(true));
        let f = finish.clone();
        revert.connect_clicked(move |_| f(false));
    }
    {
        let (s2, bar, ask, finish, apply2) = (st.clone(), bar.clone(), ask.clone(), finish.clone(), apply.clone());
        apply.connect_clicked(move |_| {
            let (wm, outs, saved) = {
                let s = s2.borrow();
                (s.wm, s.outs.clone(), s.saved.clone())
            };
            if outs == saved {
                ui::done("Nothing changed");
                return;
            }
            if wm == Wm::Hypr {
                match hypr_write(&outs) {
                    Ok(old) => s2.borrow_mut().undo = Some(old),
                    Err(e) => {
                        ui::fail(&e);
                        return;
                    }
                }
                let (finish, s3) = (finish.clone(), s2.clone());
                glib::timeout_add_local_once(Duration::from_millis(900), move || {
                    if s3.borrow().undo.is_none() {
                        return;
                    }
                    if let Some(e) = hypr_error() {
                        finish(false);
                        ui::fail(&format!("Hyprland did not accept the new display settings: {e}"));
                    }
                });
            } else if let Err(e) = apply_now(wm, &outs) {
                let _ = apply_now(wm, &saved);
                ui::fail(&e);
                return;
            }
            apply2.set_sensitive(false);
            bar.set_visible(true);
            let left = Rc::new(std::cell::Cell::new(15));
            ask.set_text("Keep these display settings? Reverting in 15 s");
            let (ask, finish, left, s3) = (ask.clone(), finish.clone(), left.clone(), s2.clone());
            let id = glib::timeout_add_local(Duration::from_secs(1), move || {
                let n = left.get() - 1;
                left.set(n);
                if n <= 0 {
                    let _ = s3.borrow_mut().pending.take();
                    finish(false);
                    return glib::ControlFlow::Break;
                }
                ask.set_text(&format!("Keep these display settings? Reverting in {n} s"));
                glib::ControlFlow::Continue
            });
            s2.borrow_mut().pending = Some(id);
        });
    }
    scroll.upcast()
}
