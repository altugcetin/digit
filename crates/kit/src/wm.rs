use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Wm {
    Niri,
    Hyprland,
    Other,
}

pub fn which() -> Wm {
    if std::env::var_os("NIRI_SOCKET").is_some() {
        Wm::Niri
    } else if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        Wm::Hyprland
    } else {
        Wm::Other
    }
}

pub fn name() -> &'static str {
    match which() {
        Wm::Niri => "niri",
        Wm::Hyprland => "Hyprland",
        Wm::Other => "Wayland",
    }
}

#[derive(Clone, Debug, Default)]
pub struct Win {
    pub id: u64,
    pub title: String,
    pub app_id: String,
    pub focused: bool,
    pub minimized: bool,
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

static HYPR_LUA: AtomicBool = AtomicBool::new(false);

/// Hyprland with a Lua config reads `dispatch X` as `hl.dispatch(X)`, so each command carries both spellings.
fn hypr_dispatch(legacy: &str, lua: &str) -> bool {
    let ok = |r: Option<String>| r.is_some_and(|r| r.trim() == "ok");
    if !HYPR_LUA.load(Ordering::Relaxed) {
        match hypr(&format!("dispatch {legacy}")) {
            Some(r) if r.contains("hl.dispatch") => HYPR_LUA.store(true, Ordering::Relaxed),
            r => return ok(r),
        }
    }
    ok(hypr(&format!("dispatch {lua}")))
}

fn hypr_json(command: &str) -> Option<Value> {
    serde_json::from_str(&hypr(&format!("j/{command}"))?).ok()
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
        Wm::Other => Vec::new(),
    }
}

pub fn focused_output() -> Option<String> {
    match which() {
        Wm::Niri => niri(json!("FocusedOutput"))?.get("FocusedOutput")?.get("name")?.as_str().map(str::to_string),
        _ => outputs().into_iter().find(|o| o.focused).map(|o| o.name),
    }
}

fn hypr_minimized(client: &Value) -> bool {
    client.get("workspace").and_then(|w| w.get("name")).and_then(Value::as_str) == Some("special:minimized")
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
                    minimized: false,
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
                    // Special workspaces report no id; their windows still belong in the dock.
                    let space = c.get("workspace").and_then(|w| w.get("id")).and_then(Value::as_i64).unwrap_or(0);
                    let pair = |k: &str| c.get(k).and_then(Value::as_array).map(|a| (a.first().and_then(Value::as_f64).unwrap_or(0.0), a.get(1).and_then(Value::as_f64).unwrap_or(0.0)));
                    let (at, size) = (pair("at"), pair("size"));
                    let history = c.get("focusHistoryID").and_then(Value::as_i64).unwrap_or(1000);
                    Some(Win {
                        id: hex(c)?,
                        title: text(c, "title"),
                        app_id: text(c, "class"),
                        focused: history == 0,
                        minimized: hypr_minimized(c),
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
        Wm::Hyprland => {
            let focused = hypr_dispatch(&format!("focuswindow address:0x{id:x}"), &format!("hl.dsp.focus({{ window = \"address:0x{id:x}\" }})"));
            // Focus alone leaves a floating window behind the ones over it.
            if focused {
                hypr_dispatch(&format!("alterzorder top,address:0x{id:x}"), &format!("hl.dsp.window.alter_zorder({{ mode = \"top\", window = \"address:0x{id:x}\" }})"));
            }
            focused
        }
        Wm::Other => false,
    }
}

static HOLDS: AtomicU64 = AtomicU64::new(0);
static FOLLOW_MOUSE: Mutex<Option<i64>> = Mutex::new(None);

fn set_follow_mouse(value: i64) {
    hypr(&format!("eval hl.config({{ input = {{ follow_mouse = {value} }} }})"));
}

/// Hyprland: for this long the pointer can't move focus, only a click can. Asking again extends it.
pub fn hold_pointer_focus(span: Duration) {
    if which() != Wm::Hyprland {
        return;
    }
    let hold = HOLDS.fetch_add(1, Ordering::Relaxed) + 1;
    let Ok(mut before) = FOLLOW_MOUSE.lock() else { return };
    if before.is_none() {
        *before = hypr_json("getoption input:follow_mouse").and_then(|o| o.get("int")?.as_i64());
        if before.is_none_or(|value| value == 0) {
            *before = None;
            return;
        }
        set_follow_mouse(0);
    }
    drop(before);
    std::thread::spawn(move || {
        std::thread::sleep(span);
        if HOLDS.load(Ordering::Relaxed) == hold {
            release_pointer_focus();
        }
    });
}

pub fn release_pointer_focus() {
    let before = FOLLOW_MOUSE.lock().ok().and_then(|mut before| before.take());
    if let Some(value) = before {
        set_follow_mouse(value);
    }
}

pub fn close(id: u64) -> bool {
    match which() {
        Wm::Niri => niri(json!({ "Action": { "CloseWindow": { "id": id } } })).is_some(),
        Wm::Hyprland => hypr_dispatch(&format!("closewindow address:0x{id:x}"), &format!("hl.dsp.window.close({{ window = \"address:0x{id:x}\" }})")),
        Wm::Other => false,
    }
}

pub fn focus_space(id: u64) -> bool {
    match which() {
        Wm::Niri => niri(json!({ "Action": { "FocusWorkspace": { "reference": { "Id": id } } } })).is_some(),
        Wm::Hyprland => hypr_dispatch(&format!("workspace {id}"), &format!("hl.dsp.focus({{ workspace = {id} }})")),
        Wm::Other => false,
    }
}

/// An index can create a Hyprland workspace; niri clamps it to its trailing empty slot.
pub fn focus_space_index(index: u64, output: &str) -> bool {
    match which() {
        Wm::Niri => {
            if niri(json!({ "Action": { "FocusMonitor": { "output": output } } })).is_none() { return false; }
            niri(json!({ "Action": { "FocusWorkspace": { "reference": { "Index": index } } } })).is_some()
        }
        Wm::Hyprland => {
            if !hypr_dispatch(&format!("focusmonitor {output}"), &format!("hl.dsp.focus({{ monitor = {} }})", serde_json::to_string(output).unwrap_or_default())) { return false; }
            focus_space(index)
        },
        Wm::Other => false,
    }
}

/// Select the panel's output before sending niri an index reference.
pub fn move_to_space_index(id: u64, index: u64, output: &str) -> bool {
    match which() {
        Wm::Niri => {
            if niri(json!({ "Action": { "FocusMonitor": { "output": output } } })).is_none() { return false; }
            niri(json!({ "Action": { "MoveWindowToWorkspace": { "window_id": id, "reference": { "Index": index }, "focus": false } } })).is_some()
        }
        Wm::Hyprland => move_to_space(id, index),
        Wm::Other => false,
    }
}

/// Moves a window to another workspace without following it there.
pub fn move_to_space(id: u64, space: u64) -> bool {
    match which() {
        Wm::Niri => niri(json!({ "Action": { "MoveWindowToWorkspace": { "window_id": id, "reference": { "Id": space }, "focus": false } } })).is_some(),
        Wm::Hyprland => hypr_dispatch(
            &format!("movetoworkspacesilent {space},address:0x{id:x}"),
            &format!("hl.dsp.window.move({{ workspace = {space}, follow = false, window = \"address:0x{id:x}\" }})"),
        ),
        Wm::Other => false,
    }
}

/// Whether the workspace shown on `output` has a fullscreen window.
pub fn fullscreen_on(output: &str) -> bool {
    match which() {
        Wm::Niri => niri_fullscreen(output).unwrap_or(false),
        Wm::Hyprland => {
            let Some(id) = hypr_outputs().iter().find(|m| text(m, "name") == output).and_then(|m| m.get("activeWorkspace")?.get("id")?.as_i64()) else { return false };
            hypr_json("workspaces")
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default()
                .iter()
                .any(|w| w.get("id").and_then(Value::as_i64) == Some(id) && w.get("hasfullscreen").and_then(Value::as_bool).unwrap_or(false))
        }
        Wm::Other => false,
    }
}

fn niri_fullscreen(output: &str) -> Option<bool> {
    let spaces = niri(json!("Workspaces"))?;
    let active = spaces.get("Workspaces")?.as_array()?.iter().find(|w| text(w, "output") == output && w.get("is_active").and_then(Value::as_bool) == Some(true))?.get("active_window_id")?.as_u64()?;
    let outputs = niri(json!("Outputs"))?;
    let logical = outputs.get("Outputs")?.get(output)?.get("logical")?;
    let (w, h) = (logical.get("width")?.as_f64()?, logical.get("height")?.as_f64()?);
    let windows = niri(json!("Windows"))?;
    let size = windows.get("Windows")?.as_array()?.iter().find(|v| v.get("id").and_then(Value::as_u64) == Some(active))?.get("layout")?.get("window_size")?.as_array()?.clone();
    let (ww, wh) = (size.first()?.as_f64()?, size.get(1)?.as_f64()?);
    Some(ww >= w - 1.0 && wh >= h - 1.0)
}

/// A Hyprland special workspace (scratchpad) that currently holds windows.
#[derive(Clone, Debug, Default)]
pub struct Special {
    pub name: String,
    pub output: String,
    pub shown: bool,
    pub apps: Vec<String>,
}

pub fn specials() -> Vec<Special> {
    if which() != Wm::Hyprland {
        return Vec::new();
    }
    let shown: Vec<String> = hypr_outputs().iter().filter_map(|m| m.get("specialWorkspace").map(|s| text(s, "name"))).filter(|n| !n.is_empty()).collect();
    let clients = hypr_json("clients").and_then(|v| v.as_array().cloned()).unwrap_or_default();
    let mut out: Vec<Special> = hypr_json("workspaces")
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter(|w| text(w, "name").starts_with("special:") || w.get("id").and_then(Value::as_i64).is_some_and(|i| i < 0))
        .map(|w| {
            let full = text(w, "name");
            let apps = clients.iter().filter(|c| c.get("workspace").is_some_and(|s| text(s, "name") == full)).map(|c| text(c, "class")).collect();
            Special { name: full.strip_prefix("special:").unwrap_or(&full).to_string(), output: text(w, "monitor"), shown: shown.contains(&full), apps }
        })
        .filter(|s| !s.apps.is_empty())
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub fn toggle_special(name: &str) -> bool {
    let arg = if name == "special" { String::new() } else { name.to_string() };
    hypr_dispatch(&format!("togglespecialworkspace {arg}"), &format!("hl.dsp.workspace.toggle_special({})", serde_json::to_string(&arg).unwrap_or_default()))
}

/// Restore to the focused monitor's normal workspace, then focus without opening a special workspace.
pub fn restore(id: u64) -> bool {
    if which() != Wm::Hyprland { return false; }
    let Some(monitor) = hypr_outputs().into_iter().find(|m| m.get("focused").and_then(Value::as_bool) == Some(true)) else { return false };
    let Some(space) = monitor.get("activeWorkspace").and_then(|w| w.get("id")).and_then(Value::as_i64).filter(|id| *id > 0) else { return false };
    // Close first: moving the last special window can close its overlay automatically.
    if let Some(name) = monitor.get("specialWorkspace").and_then(|w| w.get("name")).and_then(Value::as_str).filter(|s| !s.is_empty()) {
        if !toggle_special(name.strip_prefix("special:").unwrap_or(name)) { return false; }
    }
    if !move_to_space(id, space as u64) { return false; }
    focus(id)
}

/// Silently minimize a known Hyprland window without switching workspaces.
pub fn minimize_window(id: u64) -> bool {
    if which() != Wm::Hyprland { return false; }
    let Some(clients) = hypr_json("clients").and_then(|v| v.as_array().cloned()) else { return false };
    let Some(client) = clients.iter().find(|c| hex(c) == Some(id)) else { return false };
    if hypr_minimized(client) || client.get("mapped").and_then(Value::as_bool) == Some(false) { return false; }
    hypr_dispatch(
        &format!("movetoworkspacesilent special:minimized,address:0x{id:x}"),
        &format!("hl.dsp.window.move({{ workspace = \"special:minimized\", follow = false, window = \"address:0x{id:x}\" }})"),
    )
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
        Wm::Hyprland => minimize_window(me.id),
        Wm::Other => false,
    }
}

pub fn version() -> Option<String> {
    let run = |argv: &[&str]| std::process::Command::new(argv[0]).args(&argv[1..]).stderr(std::process::Stdio::null()).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or("").trim().to_string());
    match which() {
        Wm::Niri => run(&["niri", "--version"]),
        Wm::Hyprland => hypr_json("version").and_then(|v| v.get("tag").and_then(Value::as_str).map(|t| format!("Hyprland {t}"))),
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
            let _ = hypr_dispatch(&format!("focuswindow {addr}"), &format!("hl.dsp.focus({{ window = \"{addr}\" }})"));
            let (legacy, lua) = match act {
                WinAct::Fullscreen => ("fullscreen 0".to_string(), format!("hl.dsp.window.fullscreen({{ mode = \"fullscreen\", window = \"{addr}\" }})")),
                WinAct::Maximize => ("fullscreen 1".to_string(), format!("hl.dsp.window.fullscreen({{ mode = \"maximized\", window = \"{addr}\" }})")),
                WinAct::Float => (format!("togglefloating {addr}"), format!("hl.dsp.window.float({{ window = \"{addr}\" }})")),
                WinAct::Center => ("centerwindow".to_string(), format!("hl.dsp.window.center({{ window = \"{addr}\" }})")),
                WinAct::NextOutput => ("movewindow mon:+1".to_string(), format!("hl.dsp.window.move({{ monitor = \"+1\", window = \"{addr}\" }})")),
            };
            hypr_dispatch(&legacy, &lua)
        }
        Wm::Other => false,
    }
}

pub fn quit() -> bool {
    match which() {
        Wm::Niri => niri(json!({ "Action": { "Quit": { "skip_confirmation": true } } })).is_some(),
        Wm::Hyprland => hypr_dispatch("exit", "hl.dsp.exit()"),
        Wm::Other => false,
    }
}

pub fn screens(on: bool) -> bool {
    match which() {
        Wm::Niri => niri(json!({ "Action": if on { json!({ "PowerOnMonitors": {} }) } else { json!({ "PowerOffMonitors": {} }) } })).is_some(),
        Wm::Hyprland => if on { hypr_dispatch("dpms on", "hl.dsp.dpms({ action = \"on\" })") } else { hypr_dispatch("dpms off", "hl.dsp.dpms({ action = \"off\" })") },
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

#[cfg(test)]
mod minimized_tests {
    use super::*;

    #[test]
    fn only_the_minimized_special_workspace_is_minimized() {
        for (name, expected) in [("special:minimized", true), ("minimized", false), ("special:other", false), ("1", false)] {
            assert_eq!(hypr_minimized(&json!({ "workspace": { "id": -99, "name": name } })), expected);
        }
        assert!(!hypr_minimized(&json!({})));
    }
}
