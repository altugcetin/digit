use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Window {
    pub id: u64,
    pub title: String,
    pub app_id: String,
    pub focused: bool,
    pub space: u64,
    pub floating: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Space {
    pub id: u64,
    pub idx: u64,
    pub output: String,
    pub active: bool,
    pub focused: bool,
    pub filled: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Running {
    pub id: u64,
    pub app_id: String,
    pub focused: bool,
    pub stamp: u64,
}

enum Feed {
    Casts(usize),
    Spaces(Vec<Space>),
    Windows(Vec<Running>),
    Opened(Value),
    Changed(Value),
    Layouts(Value),
    Keyboard(String),
}

fn short_layout(name: &str) -> String {
    name.split(['(', ',']).next().unwrap_or(name).trim().chars().take(28).collect()
}

fn running_of(w: &Value) -> Option<Running> {
    Some(Running {
        id: w.get("id")?.as_u64()?,
        app_id: w.get("app_id").and_then(Value::as_str).unwrap_or_default().to_string(),
        focused: w.get("is_focused").and_then(Value::as_bool).unwrap_or(false),
        stamp: w.get("focus_timestamp").and_then(|t| Some(t.get("secs")?.as_u64()? * 1_000_000_000 + t.get("nanos")?.as_u64()?)).unwrap_or(0),
    })
}

fn read_windows(event: &Value, list: &mut Vec<Running>, tick: &mut u64) -> bool {
    let shape = |l: &Vec<Running>| l.iter().map(|r| (r.id, r.app_id.clone(), r.focused)).collect::<Vec<_>>();
    let before = shape(list);
    if let Some(all) = event.get("WindowsChanged").and_then(|e| e.get("windows")).and_then(Value::as_array) {
        *list = all.iter().filter_map(running_of).collect();
    } else if let Some(w) = event.get("WindowOpenedOrChanged").and_then(|e| e.get("window")).and_then(running_of) {
        if w.focused {
            for r in list.iter_mut() {
                r.focused = false;
            }
        }
        match list.iter_mut().find(|r| r.id == w.id) {
            Some(r) => {
                r.app_id = w.app_id;
                r.focused = w.focused;
            }
            None => list.push(w),
        }
    } else if let Some(id) = event.get("WindowClosed").and_then(|e| e.get("id")).and_then(Value::as_u64) {
        list.retain(|r| r.id != id);
    } else if let Some(e) = event.get("WindowFocusChanged") {
        let id = e.get("id").and_then(Value::as_u64);
        *tick += 1;
        for r in list.iter_mut() {
            r.focused = Some(r.id) == id;
            if r.focused {
                r.stamp = u64::MAX / 2 + *tick;
            }
        }
    }
    list.sort_by_key(|r| r.id);
    shape(list) != before
}

pub fn request(request: Value) -> Option<Value> {
    niri(request)
}

fn niri(request: Value) -> Option<Value> {
    let path = std::env::var_os("NIRI_SOCKET")?;
    let mut stream = UnixStream::connect(path).ok()?;
    stream.set_read_timeout(Some(Duration::from_millis(500))).ok()?;
    writeln!(stream, "{request}").ok()?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).ok()?;
    let reply: Value = serde_json::from_str(&line).ok()?;
    reply.get("Ok").cloned()
}

pub fn windows() -> Vec<Window> {
    kit::wm::windows()
        .into_iter()
        .map(|w| Window { id: w.id, title: w.title, app_id: w.app_id, focused: w.focused, space: w.space, floating: w.floating })
        .collect()
}

pub fn niri_running() -> bool {
    kit::wm::which() == kit::wm::Wm::Niri
}

pub fn focus(id: u64) {
    kit::wm::focus(id);
}

pub fn overview() {
    match kit::wm::which() {
        kit::wm::Wm::Niri => {
            niri(json!({ "Action": { "ToggleOverview": {} } }));
        }
        _ => {}
    }
}

pub fn screenshot() {
    let argv: [&std::ffi::OsStr; 1] = ["digit-snap".as_ref()];
    if gtk::gio::Subprocess::newv(&argv, gtk::gio::SubprocessFlags::STDOUT_SILENCE | gtk::gio::SubprocessFlags::STDERR_SILENCE).is_err() {
        niri(json!({ "Action": { "Screenshot": { "show_pointer": false } } }));
    }
}

pub fn pick_color() -> Option<[f64; 3]> {
    let path = std::env::var_os("NIRI_SOCKET")?;
    let mut stream = UnixStream::connect(path).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(120))).ok()?;
    writeln!(stream, "\"PickColor\"").ok()?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).ok()?;
    let reply: Value = serde_json::from_str(&line).ok()?;
    let rgb = reply.get("Ok")?.get("PickedColor")?.get("rgb")?.as_array()?;
    Some([rgb.first()?.as_f64()?, rgb.get(1)?.as_f64()?, rgb.get(2)?.as_f64()?])
}

pub fn focused_app() -> Option<String> {
    kit::wm::focused_window().map(|w| w.app_id)
}

pub fn focused_id() -> Option<u64> {
    kit::wm::focused_window().map(|w| w.id)
}

pub fn act(action: Value) -> bool {
    niri(json!({ "Action": action })).is_some()
}

pub fn focus_space(id: u64) {
    kit::wm::focus_space(id);
}

pub fn focused_output() -> Option<String> {
    kit::wm::focused_output()
}

fn space_of(w: &Value) -> Option<Space> {
    Some(Space {
        id: w.get("id")?.as_u64()?,
        idx: w.get("idx")?.as_u64()?,
        output: w.get("output").and_then(Value::as_str).unwrap_or_default().to_string(),
        active: w.get("is_active").and_then(Value::as_bool).unwrap_or(false),
        focused: w.get("is_focused").and_then(Value::as_bool).unwrap_or(false),
        filled: w.get("active_window_id").is_some_and(|v| !v.is_null()),
    })
}

fn read_spaces(event: &Value, spaces: &mut Vec<Space>) -> bool {
    let before = spaces.clone();
    if let Some(all) = event.get("WorkspacesChanged").and_then(|e| e.get("workspaces")).and_then(Value::as_array) {
        *spaces = all.iter().filter_map(space_of).collect();
        spaces.sort_by(|a, b| a.output.cmp(&b.output).then(a.idx.cmp(&b.idx)));
    } else if let Some(e) = event.get("WorkspaceActivated") {
        let id = e.get("id").and_then(Value::as_u64).unwrap_or(0);
        let focused = e.get("focused").and_then(Value::as_bool).unwrap_or(false);
        let output = spaces.iter().find(|s| s.id == id).map(|s| s.output.clone());
        for s in spaces.iter_mut() {
            if Some(&s.output) == output.as_ref() {
                s.active = s.id == id;
            }
            if focused {
                s.focused = s.id == id;
            }
        }
    } else if let Some(e) = event.get("WorkspaceActiveWindowChanged") {
        let id = e.get("workspace_id").and_then(Value::as_u64).unwrap_or(0);
        let filled = e.get("active_window_id").is_some_and(|v| !v.is_null());
        if let Some(s) = spaces.iter_mut().find(|s| s.id == id) {
            s.filled = filled;
        }
    }
    *spaces != before
}

fn events(tx: async_channel::Sender<Feed>) {
    let mut wait = 1;
    loop {
        if let Some(mut stream) = std::env::var_os("NIRI_SOCKET").and_then(|p| UnixStream::connect(p).ok()) {
            if writeln!(stream, "\"EventStream\"").is_ok() {
                wait = 1;
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                let mut live: std::collections::HashMap<u64, bool> = std::collections::HashMap::new();
                let mut spaces: Vec<Space> = Vec::new();
                let mut windows: Vec<Running> = Vec::new();
                let mut tick = 0u64;
                let mut sent = usize::MAX;
                let mut keyboard: (Vec<String>, Option<u64>) = (Vec::new(), None);
                while reader.read_line(&mut line).is_ok_and(|n| n > 0) {
                    if line.starts_with("{\"KeyboardLayout") {
                        if let Ok(event) = serde_json::from_str::<Value>(&line) {
                            if let Some(l) = event.get("KeyboardLayoutsChanged").and_then(|e| e.get("keyboard_layouts")) {
                                keyboard.0 = l.get("names").and_then(Value::as_array).map(|a| a.iter().filter_map(|n| n.as_str().map(str::to_string)).collect()).unwrap_or_default();
                                keyboard.1 = l.get("current_idx").and_then(Value::as_u64);
                            } else if let Some(idx) = event.get("KeyboardLayoutSwitched").and_then(|e| e.get("idx")).and_then(Value::as_u64) {
                                let changed = keyboard.1.is_some_and(|old| old != idx);
                                keyboard.1 = Some(idx);
                                if let Some(name) = keyboard.0.get(idx as usize).filter(|_| changed && keyboard.0.len() > 1) {
                                    if tx.send_blocking(Feed::Keyboard(short_layout(name))).is_err() {
                                        return;
                                    }
                                }
                            }
                        }
                        line.clear();
                        continue;
                    }
                    if line.starts_with("{\"Cast") {
                        if let Ok(event) = serde_json::from_str::<Value>(&line) {
                            let entry = |c: &Value| Some((c.get("stream_id")?.as_u64()?, c.get("is_active").and_then(Value::as_bool).unwrap_or(true)));
                            if let Some(all) = event.get("CastsChanged").and_then(|e| e.get("casts")).and_then(Value::as_array) {
                                live = all.iter().filter_map(entry).collect();
                            } else if let Some((id, active)) = event.get("CastStartedOrChanged").and_then(|e| e.get("cast")).and_then(entry) {
                                live.insert(id, active);
                            } else if let Some(id) = event.get("CastStopped").and_then(|e| e.get("stream_id")).and_then(Value::as_u64) {
                                live.remove(&id);
                            }
                            let n = live.values().filter(|a| **a).count();
                            if n != sent {
                                sent = n;
                                if tx.send_blocking(Feed::Casts(n)).is_err() {
                                    return;
                                }
                            }
                        }
                    } else if line.starts_with("{\"Window") {
                        if let Ok(event) = serde_json::from_str::<Value>(&line) {
                            if let Some(w) = event.get("WindowOpenedOrChanged").and_then(|e| e.get("window")) {
                                let id = w.get("id").and_then(Value::as_u64);
                                let fresh = !windows.is_empty() && id.is_some_and(|id| !windows.iter().any(|r| r.id == id));
                                let feed = if fresh { Feed::Opened(w.clone()) } else { Feed::Changed(w.clone()) };
                                if tx.send_blocking(feed).is_err() {
                                    return;
                                }
                            } else if let Some(changes) = event.get("WindowLayoutsChanged").and_then(|e| e.get("changes")) {
                                if tx.send_blocking(Feed::Layouts(changes.clone())).is_err() {
                                    return;
                                }
                            }
                            if read_windows(&event, &mut windows, &mut tick) && tx.send_blocking(Feed::Windows(windows.clone())).is_err() {
                                return;
                            }
                        }
                    } else if line.starts_with("{\"Workspace") {
                        if let Ok(event) = serde_json::from_str::<Value>(&line) {
                            if read_spaces(&event, &mut spaces) && tx.send_blocking(Feed::Spaces(spaces.clone())).is_err() {
                                return;
                            }
                        }
                    }
                    line.clear();
                }
                if sent > 0 && sent != usize::MAX && tx.send_blocking(Feed::Casts(0)).is_err() {
                    return;
                }
            }
        }
        if tx.is_closed() {
            return;
        }
        std::thread::sleep(Duration::from_secs(wait));
        wait = (wait * 2).min(30);
    }
}

fn running_list() -> Vec<Running> {
    let mut list: Vec<Running> = kit::wm::windows().into_iter().map(|w| Running { id: w.id, app_id: w.app_id, focused: w.focused, stamp: w.stamp }).collect();
    list.sort_by_key(|r| r.id);
    list
}

fn space_list() -> Vec<Space> {
    kit::wm::workspaces().into_iter().map(|w| Space { id: w.id, idx: w.idx, output: w.output, active: w.active, focused: w.focused, filled: w.filled }).collect()
}

fn snapshot(tx: &async_channel::Sender<Feed>, last: &mut (Vec<Running>, Vec<Space>)) -> bool {
    let (windows, spaces) = (running_list(), space_list());
    if windows != last.0 {
        last.0 = windows.clone();
        if tx.send_blocking(Feed::Windows(windows)).is_err() {
            return false;
        }
    }
    if spaces != last.1 {
        last.1 = spaces.clone();
        if tx.send_blocking(Feed::Spaces(spaces)).is_err() {
            return false;
        }
    }
    true
}

fn hypr_events(tx: async_channel::Sender<Feed>) {
    let mut wait = 1;
    loop {
        if let Some(stream) = kit::wm::hypr_dir().and_then(|d| UnixStream::connect(d.join(".socket2.sock")).ok()) {
            wait = 1;
            let mut last = (Vec::new(), Vec::new());
            if !snapshot(&tx, &mut last) {
                return;
            }
            let mut casts = 0usize;
            for line in BufReader::new(stream).lines() {
                let Ok(line) = line else { break };
                let (event, data) = line.split_once(">>").unwrap_or((line.as_str(), ""));
                match event {
                    "activelayout" => {
                        let name = data.split_once(',').map(|(_, n)| n).unwrap_or(data);
                        if !name.is_empty() && tx.send_blocking(Feed::Keyboard(short_layout(name))).is_err() {
                            return;
                        }
                    }
                    "screencast" => {
                        let n = usize::from(data.starts_with('1'));
                        if n != casts {
                            casts = n;
                            if tx.send_blocking(Feed::Casts(n)).is_err() {
                                return;
                            }
                        }
                    }
                    "workspace" | "workspacev2" | "focusedmon" | "focusedmonv2" | "activewindowv2" | "openwindow" | "closewindow" | "movewindow" | "movewindowv2" | "createworkspacev2" | "destroyworkspacev2" | "moveworkspacev2" | "monitoradded" | "monitorremoved" | "changefloatingmode" => {
                        if !snapshot(&tx, &mut last) {
                            return;
                        }
                    }
                    _ => {}
                }
            }
        }
        if tx.is_closed() {
            return;
        }
        std::thread::sleep(Duration::from_secs(wait));
        wait = (wait * 2).min(30);
    }
}

pub fn watch(app: &std::rc::Rc<crate::app::App>) {
    let (tx, rx) = async_channel::bounded::<Feed>(16);
    match kit::wm::which() {
        kit::wm::Wm::Niri => {
            let _ = std::thread::Builder::new().name("digit-niri".into()).stack_size(128 * 1024).spawn(move || events(tx));
        }
        kit::wm::Wm::Hyprland => {
            let _ = std::thread::Builder::new().name("digit-hypr".into()).stack_size(128 * 1024).spawn(move || hypr_events(tx));
        }
        kit::wm::Wm::Other => return,
    }
    let weak = std::rc::Rc::downgrade(app);
    let quiet = std::rc::Rc::new(std::cell::Cell::new(0u64));
    gtk::glib::spawn_future_local(async move {
        while let Ok(feed) = rx.recv().await {
            let Some(app) = weak.upgrade() else { break };
            match feed {
                Feed::Casts(n) if n > 0 => {
                    quiet.set(quiet.get() + 1);
                    app.set_lights(|l| l.screen = n);
                }
                Feed::Casts(_) => {
                    let generation = quiet.get() + 1;
                    quiet.set(generation);
                    let (later, quiet) = (std::rc::Rc::downgrade(&app), quiet.clone());
                    gtk::glib::timeout_add_local_once(Duration::from_secs(3), move || {
                        if quiet.get() != generation {
                            return;
                        }
                        if let Some(app) = later.upgrade() {
                            app.set_lights(|l| l.screen = 0);
                        }
                    });
                }
                Feed::Spaces(spaces) => app.set_spaces(spaces),
                Feed::Windows(list) => app.set_running(list),
                Feed::Opened(w) => crate::free::opened(&app, w),
                Feed::Changed(w) => crate::free::changed(&app, w),
                Feed::Layouts(c) => crate::free::layouts(&app, c),
                Feed::Keyboard(name) => app.request(crate::app::Peek::Osd { icon: "keyboard", text: name, level: None }),
            }
        }
    });
}
