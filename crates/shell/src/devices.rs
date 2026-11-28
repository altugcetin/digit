use crate::app::{App, Peek};
use gtk::glib;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;
use zbus::zvariant::OwnedObjectPath;

const UPOWER: &str = "org.freedesktop.UPower";
const DEVICE: &str = "org.freedesktop.UPower.Device";

#[derive(Clone, Debug, PartialEq)]
pub struct Gadget {
    pub path: String,
    pub name: String,
    pub glyph: &'static str,
    pub level: u8,
    pub coarse: Option<&'static str>,
    pub charging: bool,
    pub own: bool,
}

fn glyph(kind: u32) -> &'static str {
    match kind {
        2 => "battery_full",
        3 | 4 => "battery_charging_full",
        5 => "mouse",
        6 => "keyboard",
        7 | 8 => "smartphone",
        9 => "music_note",
        10 => "tablet",
        11 => "computer",
        12 => "sports_esports",
        13 => "edit",
        14 => "touch_app",
        17 => "headset_mic",
        18 | 21 => "speaker",
        19 => "headphones",
        20 => "videocam",
        22 => "settings_remote",
        23 => "print",
        24 => "scanner",
        25 => "photo_camera",
        26 => "watch",
        27 => "toys",
        _ => "bluetooth",
    }
}

fn kind_name(kind: u32) -> &'static str {
    match kind {
        5 => "Mouse",
        6 => "Keyboard",
        8 => "Phone",
        10 => "Tablet",
        12 => "Controller",
        13 => "Pen",
        14 => "Touchpad",
        17 => "Headset",
        18 => "Speaker",
        19 => "Headphones",
        26 => "Watch",
        _ => "Device",
    }
}

fn coarse(level: u32) -> Option<(u8, &'static str)> {
    match level {
        3 => Some((20, "Low")),
        4 => Some((5, "Critical")),
        6 => Some((55, "Normal")),
        7 => Some((80, "High")),
        8 => Some((100, "Full")),
        _ => None,
    }
}

fn tidy(model: &str) -> String {
    let m = model.trim();
    let m = m.strip_prefix("Logitech ").filter(|_| m.len() > 14).unwrap_or(m);
    m.chars().filter(|c| !c.is_control()).take(28).collect()
}

async fn read(conn: &zbus::Connection, path: &str) -> Option<Gadget> {
    let proxy = zbus::Proxy::new_owned(conn.clone(), UPOWER, path.to_string(), DEVICE).await.ok()?;
    let kind: u32 = proxy.get_property("Type").await.ok()?;
    let supply: bool = proxy.get_property("PowerSupply").await.unwrap_or(false);
    let present: bool = proxy.get_property("IsPresent").await.unwrap_or(true);
    if kind == 1 || !present {
        return None;
    }
    let own = supply && kind == 2;
    if supply && !own {
        return None;
    }
    let percent: f64 = proxy.get_property("Percentage").await.unwrap_or(0.0);
    let level: u32 = proxy.get_property("BatteryLevel").await.unwrap_or(0);
    let state: u32 = proxy.get_property("State").await.unwrap_or(0);
    let model: String = proxy.get_property("Model").await.unwrap_or_default();
    let (value, label) = match coarse(level) {
        Some((v, l)) => (v, Some(l)),
        None => {
            if percent <= 0.0 && state == 0 {
                return None;
            }
            (percent.round().clamp(0.0, 100.0) as u8, None)
        }
    };
    let name = if own {
        "This Computer".to_string()
    } else if model.trim().is_empty() {
        kind_name(kind).to_string()
    } else {
        tidy(&model)
    };
    Some(Gadget { path: path.to_string(), name, glyph: if own { "laptop" } else { glyph(kind) }, level: value, coarse: label, charging: state == 1, own })
}

async fn scan(conn: &zbus::Connection) -> Vec<Gadget> {
    let Ok(proxy) = zbus::Proxy::new_owned(conn.clone(), UPOWER, "/org/freedesktop/UPower", UPOWER).await else { return Vec::new() };
    let Ok(paths) = proxy.call::<_, _, Vec<OwnedObjectPath>>("EnumerateDevices", &()).await else { return Vec::new() };
    let mut out = Vec::new();
    for p in paths.iter().take(24) {
        if let Some(g) = read(conn, p.as_str()).await {
            out.push(g);
        }
    }
    out.sort_by(|a, b| b.own.cmp(&a.own).then_with(|| a.name.cmp(&b.name)));
    out
}

pub fn start(app: &Rc<App>) {
    let weak = Rc::downgrade(app);
    glib::spawn_future_local(async move {
        let Ok(conn) = crate::bus::system().await else { return };
        let Ok(rule) = zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).sender(UPOWER).map(|b| b.build()) else { return };
        let Ok(stream) = zbus::MessageStream::for_match_rule(rule, &conn, Some(32)).await else { return };
        let stream = kit::signals::drained(stream);
        let mut warned: HashMap<String, u8> = HashMap::new();
        let mut apply = |app: &Rc<App>, list: Vec<Gadget>| {
            for g in list.iter().filter(|g| !g.own) {
                let mark = warned.entry(g.path.clone()).or_insert(101);
                if g.charging || g.level > 20 {
                    *mark = 101;
                } else if let Some(t) = [5u8, 10, 20].into_iter().find(|t| g.level <= *t && *mark > *t) {
                    let first = *mark == 101 && !app.gadgets.borrow().iter().any(|o| o.path == g.path);
                    *mark = t;
                    if !first || g.level <= 10 {
                        app.request(Peek::Battery { level: g.level, device: Some((g.name.clone(), g.glyph)) });
                    }
                }
            }
            app.set_gadgets(list);
        };
        let first = scan(&conn).await;
        let Some(app) = weak.upgrade() else { return };
        apply(&app, first);
        drop(app);
        while let Ok(msg) = stream.recv().await {
            let Ok(msg) = msg else { continue };
            let member = msg.header().member().map(|m| m.to_string()).unwrap_or_default();
            if !matches!(member.as_str(), "PropertiesChanged" | "DeviceAdded" | "DeviceRemoved") {
                continue;
            }
            glib::timeout_future(Duration::from_millis(350)).await;
            kit::signals::pending(&stream);
            let list = scan(&conn).await;
            let Some(app) = weak.upgrade() else { break };
            if *app.gadgets.borrow() != list {
                apply(&app, list);
            }
        }
    });
}

pub fn summary(list: &[Gadget]) -> Option<&Gadget> {
    list.iter().filter(|g| !g.own && !g.charging && g.level <= 20).min_by_key(|g| g.level)
}
