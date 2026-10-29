use crate::app::{App, Peek};
use gtk::glib;
use std::path::{Path, PathBuf};
use std::rc::Rc;

fn read(p: &Path) -> Option<u32> {
    std::fs::read_to_string(p).ok()?.trim().parse().ok()
}

fn devices(class: &str, kbd: bool) -> Vec<PathBuf> {
    let Ok(dir) = std::fs::read_dir(format!("/sys/class/{class}")) else { return Vec::new() };
    let mut list: Vec<PathBuf> = dir
        .flatten()
        .map(|e| e.path())
        .filter(|p| !kbd || p.file_name().is_some_and(|n| n.to_string_lossy().contains("kbd_backlight")))
        .filter(|p| read(&p.join("max_brightness")).is_some_and(|m| m > 0))
        .collect();
    list.sort_by_key(|p| {
        let kind = std::fs::read_to_string(p.join("type")).unwrap_or_default();
        match kind.trim() {
            "firmware" => 0,
            "platform" => 1,
            _ => 2,
        }
    });
    list
}

pub fn step(app: &Rc<App>, action: &str) -> bool {
    let (class, kbd, up, icon, label) = match action {
        "brightness-up" => ("backlight", false, true, "light_mode", "Brightness"),
        "brightness-down" => ("backlight", false, false, "light_mode", "Brightness"),
        "kbd-up" => ("leds", true, true, "keyboard", "Keyboard light"),
        "kbd-down" => ("leds", true, false, "keyboard", "Keyboard light"),
        _ => return false,
    };
    let Some(dev) = devices(class, kbd).into_iter().next() else { return false };
    let Some(max) = read(&dev.join("max_brightness")) else { return false };
    let now = read(&dev.join("actual_brightness")).or_else(|| read(&dev.join("brightness"))).unwrap_or(0);
    let unit = if kbd { 1.0 / max.min(10) as f64 } else { 0.05 };
    let level = now as f64 / max as f64;
    let next = if up { (level + unit).min(1.0) } else { (level - unit).max(if kbd { 0.0 } else { 0.01 }) };
    let value = ((next * max as f64).round() as u32).min(max);
    let name = dev.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let subsystem = class.to_string();
    glib::spawn_future_local(async move {
        let Ok(conn) = crate::bus::system().await else { return };
        let Ok(session) = zbus::Proxy::new_owned(conn, "org.freedesktop.login1", "/org/freedesktop/login1/session/auto", "org.freedesktop.login1.Session").await else { return };
        let _ = session.call_method("SetBrightness", &(subsystem.as_str(), name.as_str(), value)).await;
    });
    app.request(Peek::Osd { icon, text: label.into(), level: Some(value as f64 / max as f64) });
    true
}
