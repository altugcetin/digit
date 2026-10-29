use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

fn request(body: Value) -> Option<Value> {
    let path = std::env::var_os("NIRI_SOCKET")?;
    let mut stream = UnixStream::connect(path).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    writeln!(stream, "{body}").ok()?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).ok()?;
    serde_json::from_str::<Value>(&line).ok()?.get("Ok").cloned()
}

pub fn act(action: Value) -> bool {
    request(json!({ "Action": action })).is_some()
}

pub fn windows() -> Vec<Value> {
    request(json!("Windows")).and_then(|v| v.get("Windows").and_then(Value::as_array).cloned()).unwrap_or_default()
}

pub fn minimize(app_id: &str) -> bool {
    let Some(me) = request(json!("FocusedWindow")).and_then(|v| v.get("FocusedWindow").cloned()) else { return false };
    if me.get("app_id").and_then(Value::as_str) != Some(app_id) {
        return false;
    }
    let (Some(id), Some(ws)) = (me.get("id").and_then(Value::as_u64), me.get("workspace_id").and_then(Value::as_u64)) else { return false };
    let spaces = request(json!("Workspaces")).and_then(|v| v.get("Workspaces").and_then(Value::as_array).cloned()).unwrap_or_default();
    let output = spaces.iter().find(|w| w.get("id").and_then(Value::as_u64) == Some(ws)).and_then(|w| w.get("output").cloned());
    let target = spaces
        .iter()
        .filter(|w| w.get("output") == output.as_ref())
        .max_by_key(|w| w.get("idx").and_then(Value::as_u64).unwrap_or(0))
        .and_then(|w| w.get("id").and_then(Value::as_u64));
    match target {
        Some(t) if t != ws => act(json!({ "MoveWindowToWorkspace": { "window_id": id, "reference": { "Id": t }, "focus": false } })),
        _ => false,
    }
}

pub fn workspaces() -> Vec<Value> {
    request(json!("Workspaces")).and_then(|v| v.get("Workspaces").and_then(Value::as_array).cloned()).unwrap_or_default()
}

pub fn focused_output() -> Option<String> {
    request(json!("FocusedOutput")).and_then(|v| v.get("FocusedOutput")?.get("name")?.as_str().map(str::to_string))
}

pub fn focused_window() -> Option<Value> {
    request(json!("FocusedWindow")).and_then(|v| v.get("FocusedWindow").cloned()).filter(|v| !v.is_null())
}

pub fn available() -> bool {
    std::env::var_os("NIRI_SOCKET").is_some()
}
