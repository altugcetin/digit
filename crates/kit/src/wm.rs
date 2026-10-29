use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Wm {
    Niri,
    Hyprland,
    Mango,
    Other,
}

pub fn which() -> Wm {
    if std::env::var_os("NIRI_SOCKET").is_some() {
        Wm::Niri
    } else if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        Wm::Hyprland
    } else if std::env::var_os("MANGO_INSTANCE_SIGNATURE").is_some() {
        Wm::Mango
    } else {
        Wm::Other
    }
}

pub fn name() -> &'static str {
    match which() {
        Wm::Niri => "niri",
        Wm::Hyprland => "Hyprland",
        Wm::Mango => "MangoWC",
        Wm::Other => "Wayland",
    }
}

#[derive(Clone, Debug, Default)]
pub struct Win {
    pub id: u64,
    pub title: String,
    pub app_id: String,
    pub focused: bool,
    pub space: u64,
    pub floating: bool,
    pub stamp: u64,
    pub output: String,
    pub rect: Option<(f64, f64, f64, f64)>,
    pub pid: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ws {
    pub id: u64,
    pub idx: u64,
    pub output: String,
    pub active: bool,
    pub focused: bool,
    pub filled: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Output {
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub focused: bool,
}

fn exchange(path: PathBuf, body: &str, timeout: Duration) -> Option<String> {
    let mut stream = UnixStream::connect(path).ok()?;
    stream.set_read_timeout(Some(timeout)).ok()?;
    stream.set_write_timeout(Some(timeout)).ok()?;
    stream.write_all(body.as_bytes()).ok()?;
    let mut out = String::new();
    stream.take(8 << 20).read_to_string(&mut out).ok()?;
    Some(out)
}

pub fn hypr_dir() -> Option<PathBuf> {
    let sig = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
    let sig = sig.to_str()?;
    if sig.is_empty() || sig.contains('/') {
        return None;
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)?;
    let dir = runtime.join("hypr").join(sig);
    if dir.is_dir() { Some(dir) } else { std::env::temp_dir().join("hypr").join(sig).is_dir().then(|| std::env::temp_dir().join("hypr").join(sig)) }
}

pub fn hypr(command: &str) -> Option<String> {
    exchange(hypr_dir()?.join(".socket.sock"), command, Duration::from_millis(800))
}

fn hypr_json(command: &str) -> Option<Value> {
    serde_json::from_str(&hypr(&format!("j/{command}"))?).ok()
}

pub fn mango_socket() -> Option<PathBuf> {
    std::env::var_os("MANGO_INSTANCE_SIGNATURE").map(PathBuf::from).filter(|p| p.is_absolute())
}

pub fn mango(command: &str) -> Option<Value> {
    let mut stream = UnixStream::connect(mango_socket()?).ok()?;
    stream.set_read_timeout(Some(Duration::from_millis(800))).ok()?;
    stream.write_all(format!("{command}\n").as_bytes()).ok()?;
    let mut line = String::new();
    BufReader::new(stream.take(8 << 20)).read_line(&mut line).ok()?;
    serde_json::from_str(&line).ok()
}

fn niri(body: Value) -> Option<Value> {
    let path = std::env::var_os("NIRI_SOCKET")?;
    let mut stream = UnixStream::connect(path).ok()?;
    stream.set_read_timeout(Some(Duration::from_millis(800))).ok()?;
    writeln!(stream, "{body}").ok()?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).ok()?;
    serde_json::from_str::<Value>(&line).ok()?.get("Ok").cloned()
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn hex(v: &Value) -> Option<u64> {
    u64::from_str_radix(v.get("address")?.as_str()?.trim_start_matches("0x"), 16).ok()
}

fn hypr_outputs() -> Vec<Value> {
    hypr_json("monitors").and_then(|v| v.as_array().cloned()).unwrap_or_default()
}

pub fn outputs() -> Vec<Output> {
    match which() {
        Wm::Niri => {
            let focused = focused_output();
            niri(json!("Outputs"))
                .and_then(|v| v.get("Outputs").and_then(Value::as_object).cloned())
                .map(|m| {
                    m.values()
                        .map(|o| {
                            let l = o.get("logical");
                            let at = |k: &str| l.and_then(|l| l.get(k)).and_then(Value::as_f64).unwrap_or(0.0);
                            let name = text(o, "name");
                            Output { focused: focused.as_deref() == Some(name.as_str()), name, x: at("x"), y: at("y") }
                        })
                        .collect()
                })
                .unwrap_or_default()
        }
        Wm::Hyprland => hypr_outputs()
            .iter()
            .map(|m| Output { name: text(m, "name"), x: m.get("x").and_then(Value::as_f64).unwrap_or(0.0), y: m.get("y").and_then(Value::as_f64).unwrap_or(0.0), focused: m.get("focused").and_then(Value::as_bool).unwrap_or(false) })
            .collect(),
        Wm::Mango => mango("get all-monitors")
            .and_then(|v| v.get("monitors").and_then(Value::as_array).cloned())
            .unwrap_or_default()
            .iter()
            .map(|m| Output { name: text(m, "name"), x: m.get("x").and_then(Value::as_f64).unwrap_or(0.0), y: m.get("y").and_then(Value::as_f64).unwrap_or(0.0), focused: m.get("active").and_then(Value::as_bool).unwrap_or(false) })
            .collect(),
        Wm::Other => Vec::new(),
    }
}

pub fn focused_output() -> Option<String> {
    match which() {
        Wm::Niri => niri(json!("FocusedOutput"))?.get("FocusedOutput")?.get("name")?.as_str().map(str::to_string),
        _ => outputs().into_iter().find(|o| o.focused).map(|o| o.name),
    }
}

fn mango_space(output_index: usize, tag: u64) -> u64 {
    (output_index as u64 + 1) * 1000 + tag
}

pub fn windows() -> Vec<Win> {
    let mut out: Vec<Win> = match which() {
        Wm::Niri => niri(json!("Windows"))
            .and_then(|v| v.get("Windows").and_then(Value::as_array).cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|w| {
                Some(Win {
                    id: w.get("id")?.as_u64()?,
                    title: text(w, "title"),
                    app_id: text(w, "app_id"),
                    focused: w.get("is_focused").and_then(Value::as_bool).unwrap_or(false),
                    space: w.get("workspace_id").and_then(Value::as_u64).unwrap_or(0),
                    floating: w.get("is_floating").and_then(Value::as_bool).unwrap_or(false),
                    stamp: w.get("focus_timestamp").and_then(|t| Some(t.get("secs")?.as_u64()? * 1_000_000_000 + t.get("nanos")?.as_u64()?)).unwrap_or(0),
                    output: String::new(),
                    rect: None,
                    pid: w.get("pid").and_then(Value::as_u64).map(|p| p as u32),
                })
            })
            .collect(),
        Wm::Hyprland => {
            let monitors = hypr_outputs();
            let name_of = |id: i64| monitors.iter().find(|m| m.get("id").and_then(Value::as_i64) == Some(id)).map(|m| text(m, "name")).unwrap_or_default();
            hypr_json("clients")
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default()
                .iter()
                .filter(|c| c.get("mapped").and_then(Value::as_bool).unwrap_or(true) && !c.get("hidden").and_then(Value::as_bool).unwrap_or(false))
                .filter_map(|c| {
                    let space = c.get("workspace")?.get("id")?.as_i64()?;
                    let pair = |k: &str| c.get(k).and_then(Value::as_array).map(|a| (a.first().and_then(Value::as_f64).unwrap_or(0.0), a.get(1).and_then(Value::as_f64).unwrap_or(0.0)));
                    let (at, size) = (pair("at"), pair("size"));
                    let history = c.get("focusHistoryID").and_then(Value::as_i64).unwrap_or(1000);
                    Some(Win {
                        id: hex(c)?,
                        title: text(c, "title"),
                        app_id: text(c, "class"),
                        focused: history == 0,
                        space: if space > 0 { space as u64 } else { 0 },
                        floating: c.get("floating").and_then(Value::as_bool).unwrap_or(false),
                        stamp: (1_000_000 - history.clamp(0, 1_000_000)) as u64,
                        output: name_of(c.get("monitor").and_then(Value::as_i64).unwrap_or(-1)),
                        rect: at.zip(size).map(|(a, s)| (a.0, a.1, s.0, s.1)),
                        pid: c.get("pid").and_then(Value::as_i64).filter(|p| *p > 0).map(|p| p as u32),
                    })
                })
                .collect()
        }
        Wm::Mango => {
            let names: Vec<String> = outputs().into_iter().map(|o| o.name).collect();
            mango("get all-clients")
                .and_then(|v| v.get("clients").and_then(Value::as_array).cloned())
                .unwrap_or_default()
                .iter()
                .filter(|c| !c.get("is_minimized").and_then(Value::as_bool).unwrap_or(false))
                .filter_map(|c| {
                    let output = text(c, "monitor");
                    let tag = c.get("tags").and_then(Value::as_array).and_then(|t| t.first()).and_then(Value::as_u64).unwrap_or(0);
                    let at = |k: &str| c.get(k).and_then(Value::as_f64).unwrap_or(0.0);
                    let focused = c.get("is_focused").and_then(Value::as_bool).unwrap_or(false);
                    Some(Win {
                        id: c.get("id")?.as_u64()?,
                        title: text(c, "title"),
                        app_id: text(c, "appid"),
                        focused,
                        space: names.iter().position(|n| *n == output).map(|i| mango_space(i, tag)).unwrap_or(0),
                        floating: c.get("is_floating").and_then(Value::as_bool).unwrap_or(false),
                        stamp: u64::from(focused),
                        rect: Some((at("x"), at("y"), at("width"), at("height"))),
                        pid: c.get("pid").and_then(Value::as_u64).map(|p| p as u32),
                        output,
                    })
                })
                .collect()
        }
        Wm::Other => Vec::new(),
    };
    out.sort_by(|a, b| b.stamp.cmp(&a.stamp));
    out
}

pub fn workspaces() -> Vec<Ws> {
    let mut out: Vec<Ws> = match which() {
        Wm::Niri => niri(json!("Workspaces"))
            .and_then(|v| v.get("Workspaces").and_then(Value::as_array).cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|w| {
                Some(Ws {
                    id: w.get("id")?.as_u64()?,
                    idx: w.get("idx")?.as_u64()?,
                    output: text(w, "output"),
                    active: w.get("is_active").and_then(Value::as_bool).unwrap_or(false),
                    focused: w.get("is_focused").and_then(Value::as_bool).unwrap_or(false),
                    filled: w.get("active_window_id").is_some_and(|v| !v.is_null()),
                })
            })
            .collect(),
        Wm::Hyprland => {
            let monitors = hypr_outputs();
            let shown: Vec<(String, i64, bool)> = monitors.iter().map(|m| (text(m, "name"), m.get("activeWorkspace").and_then(|w| w.get("id")).and_then(Value::as_i64).unwrap_or(0), m.get("focused").and_then(Value::as_bool).unwrap_or(false))).collect();
            hypr_json("workspaces")
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default()
                .iter()
                .filter_map(|w| {
                    let id = w.get("id")?.as_i64().filter(|i| *i > 0)?;
                    let output = text(w, "monitor");
                    let on = shown.iter().find(|s| s.0 == output);
                    let active = on.is_some_and(|s| s.1 == id);
                    Some(Ws { id: id as u64, idx: id as u64, active, focused: active && on.is_some_and(|s| s.2), filled: w.get("windows").and_then(Value::as_u64).unwrap_or(0) > 0, output })
                })
                .collect()
        }
        Wm::Mango => {
            let monitors = outputs();
            mango("get all-tags")
                .and_then(|v| v.get("all_tags").and_then(Value::as_array).cloned())
                .unwrap_or_default()
                .iter()
                .flat_map(|entry| {
                    let output = text(entry, "monitor");
                    let i = monitors.iter().position(|m| m.name == output).unwrap_or(0);
                    let focused_out = monitors.get(i).is_some_and(|m| m.focused);
                    entry
                        .get("tags")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default()
                        .iter()
                        .filter_map(|t| {
                            let idx = t.get("index")?.as_u64()?;
                            let active = t.get("is_active").and_then(Value::as_bool).unwrap_or(false);
                            Some(Ws { id: mango_space(i, idx), idx, output: output.clone(), active, focused: active && focused_out, filled: t.get("client_count").and_then(Value::as_u64).unwrap_or(0) > 0 })
                        })
                        .collect::<Vec<_>>()
                })
                .collect()
        }
        Wm::Other => Vec::new(),
    };
    out.sort_by(|a, b| a.output.cmp(&b.output).then(a.idx.cmp(&b.idx)));
    out
}

pub fn focused_window() -> Option<Win> {
    windows().into_iter().find(|w| w.focused)
}

pub fn focus(id: u64) -> bool {
    match which() {
        Wm::Niri => niri(json!({ "Action": { "FocusWindow": { "id": id } } })).is_some(),
        Wm::Hyprland => hypr(&format!("dispatch focuswindow address:0x{id:x}")).is_some_and(|r| r.trim() == "ok"),
        Wm::Mango => mango(&format!("dispatch focus_by_id client,{id}")).is_some_and(|r| r.get("success").is_some()),
        Wm::Other => false,
    }
}

pub fn close(id: u64) -> bool {
    match which() {
        Wm::Niri => niri(json!({ "Action": { "CloseWindow": { "id": id } } })).is_some(),
        Wm::Hyprland => hypr(&format!("dispatch closewindow address:0x{id:x}")).is_some_and(|r| r.trim() == "ok"),
        Wm::Mango => mango(&format!("dispatch killclient client,{id}")).is_some_and(|r| r.get("success").is_some()),
        Wm::Other => false,
    }
}

pub fn focus_space(id: u64) -> bool {
    match which() {
        Wm::Niri => niri(json!({ "Action": { "FocusWorkspace": { "reference": { "Id": id } } } })).is_some(),
        Wm::Hyprland => hypr(&format!("dispatch workspace {id}")).is_some_and(|r| r.trim() == "ok"),
        Wm::Mango => {
            let (monitor, tag) = ((id / 1000).saturating_sub(1) as usize, id % 1000);
            if let Some(o) = outputs().get(monitor).filter(|o| !o.focused) {
                mango(&format!("dispatch focusmon,{}", o.name));
            }
            mango(&format!("dispatch view,{tag}")).is_some_and(|r| r.get("success").is_some())
        }
        Wm::Other => false,
    }
}

pub fn minimize(app_id: &str) -> bool {
    let Some(me) = focused_window().filter(|w| w.app_id == app_id) else { return false };
    match which() {
        Wm::Niri => {
            let spaces = workspaces();
            let output = spaces.iter().find(|w| w.id == me.space).map(|w| w.output.clone());
            let target = spaces.iter().filter(|w| Some(&w.output) == output.as_ref()).max_by_key(|w| w.idx).map(|w| w.id);
            match target {
                Some(t) if t != me.space => niri(json!({ "Action": { "MoveWindowToWorkspace": { "window_id": me.id, "reference": { "Id": t }, "focus": false } } })).is_some(),
                _ => false,
            }
        }
        Wm::Hyprland => hypr(&format!("dispatch movetoworkspacesilent special:minimized,address:0x{:x}", me.id)).is_some_and(|r| r.trim() == "ok"),
        Wm::Mango => mango(&format!("dispatch minimized client,{}", me.id)).is_some_and(|r| r.get("success").is_some()),
        Wm::Other => false,
    }
}

pub fn version() -> Option<String> {
    let run = |argv: &[&str]| std::process::Command::new(argv[0]).args(&argv[1..]).stderr(std::process::Stdio::null()).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or("").trim().to_string());
    match which() {
        Wm::Niri => run(&["niri", "--version"]),
        Wm::Hyprland => hypr_json("version").and_then(|v| v.get("tag").and_then(Value::as_str).map(|t| format!("Hyprland {t}"))),
        Wm::Mango => mango("get version").map(|v| v.get("version").and_then(Value::as_str).map(|s| format!("MangoWC {s}")).unwrap_or_else(|| format!("MangoWC {v}"))),
        Wm::Other => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WinAct {
    Fullscreen,
    Maximize,
    Float,
    Center,
    NextOutput,
}

pub fn window_act(id: u64, act: WinAct) -> bool {
    match which() {
        Wm::Niri => {
            let body = match act {
                WinAct::Fullscreen => json!({ "Action": { "FullscreenWindow": { "id": id } } }),
                WinAct::Float => json!({ "Action": { "ToggleWindowFloating": { "id": id } } }),
                WinAct::Center => json!({ "Action": { "CenterWindow": { "id": id } } }),
                WinAct::Maximize | WinAct::NextOutput => {
                    niri(json!({ "Action": { "FocusWindow": { "id": id } } }));
                    if act == WinAct::Maximize { json!({ "Action": { "MaximizeColumn": {} } }) } else { json!({ "Action": { "MoveWindowToMonitorNext": {} } }) }
                }
            };
            niri(body).is_some()
        }
        Wm::Hyprland => {
            let addr = format!("address:0x{id:x}");
            let _ = hypr(&format!("dispatch focuswindow {addr}"));
            let cmd = match act {
                WinAct::Fullscreen => "dispatch fullscreen 0".to_string(),
                WinAct::Maximize => "dispatch fullscreen 1".to_string(),
                WinAct::Float => format!("dispatch togglefloating {addr}"),
                WinAct::Center => "dispatch centerwindow".to_string(),
                WinAct::NextOutput => "dispatch movewindow mon:+1".to_string(),
            };
            hypr(&cmd).is_some_and(|r| r.trim() == "ok")
        }
        Wm::Mango => {
            let f = match act {
                WinAct::Fullscreen => "togglefullscreen",
                WinAct::Maximize => "togglemaximizescreen",
                WinAct::Float => "togglefloating",
                WinAct::Center => "centerwin",
                WinAct::NextOutput => "tagmon,right",
            };
            mango(&format!("dispatch {f} client,{id}")).is_some_and(|r| r.get("success").is_some())
        }
        Wm::Other => false,
    }
}

pub fn quit() -> bool {
    match which() {
        Wm::Niri => niri(json!({ "Action": { "Quit": { "skip_confirmation": true } } })).is_some(),
        Wm::Hyprland => hypr("dispatch exit").is_some(),
        Wm::Mango => mango("dispatch quit").is_some(),
        Wm::Other => false,
    }
}

pub fn screens(on: bool) -> bool {
    match which() {
        Wm::Niri => niri(json!({ "Action": if on { json!({ "PowerOnMonitors": {} }) } else { json!({ "PowerOffMonitors": {} }) } })).is_some(),
        Wm::Hyprland => hypr(if on { "dispatch dpms on" } else { "dispatch dpms off" }).is_some(),
        Wm::Mango => mango(if on { "dispatch enable_monitor" } else { "dispatch disable_monitor" }).is_some(),
        Wm::Other => false,
    }
}

pub fn layouts() -> Option<(Vec<String>, usize)> {
    match which() {
        Wm::Niri => {
            let v = niri(json!("KeyboardLayouts"))?;
            let l = v.get("Ok")?.get("KeyboardLayouts")?;
            let names = l.get("names")?.as_array()?.iter().filter_map(|n| n.as_str().map(str::to_string)).collect();
            Some((names, l.get("current_idx")?.as_u64()? as usize))
        }
        Wm::Hyprland => {
            let v = hypr_json("devices")?;
            let kb = v.get("keyboards")?.as_array()?.iter().find(|k| k.get("main").and_then(Value::as_bool).unwrap_or(false))?.clone();
            let names: Vec<String> = kb.get("layout")?.as_str()?.split(',').map(|s| s.trim().to_string()).collect();
            let active = kb.get("active_keymap")?.as_str()?.to_string();
            let idx = names.iter().position(|n| active.to_lowercase().contains(&n.to_lowercase())).unwrap_or(0);
            Some((names, idx))
        }
        _ => None,
    }
}
