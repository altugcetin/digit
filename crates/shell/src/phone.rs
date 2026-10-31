//! The paired phone, as KDE Connect's daemon sees it: battery, signal, the notifications it
//! mirrors and recent calls. The notch's Phone tab draws it; D-Bus signals keep it current.

use crate::app::App;
use gtk::{gdk, gio, glib, prelude::*};
use std::cell::RefCell;
use std::rc::{Rc, Weak};

const BUS: &str = "org.kde.kdeconnect";
const PROPS: &str = "org.freedesktop.DBus.Properties";

#[derive(Clone)]
pub struct Note {
    pub path: String,
    pub app: String,
    pub title: String,
    pub text: String,
    pub icon: Option<gdk::Texture>,
    pub dismissable: bool,
}

#[derive(Clone)]
pub struct Phone {
    pub id: String,
    pub name: String,
    pub reachable: bool,
    /// KDE Connect's device type: "phone", "tablet", "desktop", "laptop" or "tv".
    pub kind: String,
    /// The phone's address on the local network, for mirroring over Wi-Fi.
    pub address: Option<String>,
    pub battery: Option<(i32, bool)>,
    /// Network type ("LTE") and strength, 0 to 4.
    pub signal: Option<(String, i32)>,
    pub notes: Vec<Note>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Action {
    Ring,
    Ping,
    Clipboard,
    Browse,
    Mirror,
}

pub const ACTIONS: [(Action, &str, &str); 5] = [
    (Action::Mirror, "cast", "Mirror screen"),
    (Action::Ring, "ring_volume", "Ring"),
    (Action::Ping, "vibration", "Ping"),
    (Action::Clipboard, "content_paste_go", "Send clipboard"),
    (Action::Browse, "folder_open", "Browse files"),
];

#[derive(Default)]
struct State {
    /// Whether the daemon answered at all; without it the tab explains how to get KDE Connect.
    daemon: bool,
    phone: Option<Phone>,
    paired: Vec<Paired>,
    /// The device picked in the switcher, if any.
    chosen: Option<String>,
    /// The latest call, as "Incoming call from …" and when it came.
    call: Option<(String, i64)>,
    /// A short status line from the last action, such as why mirroring failed, and when it was set.
    note: Option<(String, i64)>,
    loading: bool,
    again: bool,
    watching: bool,
    icons: Vec<(String, gdk::Texture)>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

pub fn phone() -> Option<Phone> {
    STATE.with(|s| s.borrow().phone.clone())
}

/// The last action's status line, for a few seconds after it was set.
pub fn note() -> Option<String> {
    let now = glib::monotonic_time();
    STATE.with(|s| s.borrow().note.clone()).filter(|(_, at)| now - at < 6_000_000).map(|(n, _)| n)
}

fn set_note(app: &Weak<App>, text: &str) {
    STATE.with(|s| s.borrow_mut().note = Some((text.to_string(), glib::monotonic_time())));
    if let Some(app) = app.upgrade() {
        app.redraw();
        // Clear it from the screen once it expires.
        let weak = Rc::downgrade(&app);
        glib::timeout_add_local_once(std::time::Duration::from_millis(6100), move || {
            if let Some(app) = weak.upgrade() {
                app.redraw();
            }
        });
    }
}

/// The paired device the switcher moves to next, when there is more than one.
pub fn next() -> Option<Paired> {
    STATE.with(|s| {
        let s = s.borrow();
        let at = s.paired.iter().position(|p| s.phone.as_ref().is_some_and(|c| c.id == p.id))?;
        (s.paired.len() > 1).then(|| s.paired[(at + 1) % s.paired.len()].clone())
    })
}

pub fn switch(app: &Rc<App>) {
    let Some(n) = next() else { return };
    STATE.with(|s| s.borrow_mut().chosen = Some(n.id));
    refresh(app);
}

impl Phone {
    pub fn glyph(&self) -> &'static str {
        match self.kind.as_str() {
            "tablet" => "tablet_android",
            "desktop" => "desktop_windows",
            "laptop" => "laptop",
            "tv" => "tv",
            _ => "smartphone",
        }
    }
}

pub fn daemon() -> bool {
    STATE.with(|s| s.borrow().daemon)
}

pub fn call() -> Option<String> {
    let now = glib::monotonic_time();
    STATE.with(|s| s.borrow().call.clone()).filter(|(_, at)| now - at < 10 * 60 * 1_000_000).map(|(c, _)| c)
}

async fn call_bus(conn: &gio::DBusConnection, path: &str, iface: &str, method: &str, args: Option<glib::Variant>) -> Option<glib::Variant> {
    conn.call_future(Some(BUS), path, iface, method, args.as_ref(), None, gio::DBusCallFlags::NONE, 3000).await.ok()
}

async fn props(conn: &gio::DBusConnection, path: &str, iface: &str) -> Option<glib::VariantDict> {
    let v = call_bus(conn, path, PROPS, "GetAll", Some((iface,).to_variant())).await?;
    Some(glib::VariantDict::new(Some(&v.child_value(0))))
}

fn get<T: glib::variant::FromVariant>(d: &glib::VariantDict, key: &str) -> Option<T> {
    d.lookup_value(key, None).and_then(|v| v.get::<T>())
}

fn icon(path: &str) -> Option<gdk::Texture> {
    if path.is_empty() {
        return None;
    }
    if let Some(t) = STATE.with(|s| s.borrow().icons.iter().find(|(p, _)| p == path).map(|(_, t)| t.clone())) {
        return Some(t);
    }
    let t = gdk::Texture::from_filename(path).ok()?;
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.icons.len() > 48 {
            s.icons.remove(0);
        }
        s.icons.push((path.to_string(), t.clone()));
    });
    Some(t)
}

/// A paired device in the switcher.
#[derive(Clone)]
pub struct Paired {
    pub id: String,
    pub name: String,
}

async fn fetch(conn: &gio::DBusConnection, chosen: Option<String>) -> (bool, Vec<Paired>, Option<Phone>) {
    let Some(ids) = call_bus(conn, "/modules/kdeconnect", "org.kde.kdeconnect.daemon", "devices", Some((false, true).to_variant())).await else { return (false, Vec::new(), None) };
    let ids: Vec<String> = ids.child_value(0).get().unwrap_or_default();
    let mut paired = Vec::new();
    // The one picked in the switcher, else a reachable phone, then a reachable tablet or laptop,
    // then anything out of reach.
    let mut best: Option<(u8, String, glib::VariantDict)> = None;
    for id in ids {
        let path = format!("/modules/kdeconnect/devices/{id}");
        let Some(d) = props(conn, &path, "org.kde.kdeconnect.device").await else { continue };
        let reachable = get::<bool>(&d, "isReachable").unwrap_or(false);
        paired.push(Paired { id: id.clone(), name: get::<String>(&d, "name").unwrap_or_else(|| id.clone()) });
        let rank = if chosen.as_deref() == Some(id.as_str()) { 9 } else { reachable as u8 * 2 + (get::<String>(&d, "type").as_deref() == Some("phone")) as u8 };
        if best.as_ref().is_none_or(|(r, _, _)| rank > *r) {
            best = Some((rank, id, d));
        }
    }
    let Some((_, id, d)) = best else { return (true, paired, None) };
    (true, paired, Some(detail(conn, id, &d).await))
}

async fn detail(conn: &gio::DBusConnection, id: String, d: &glib::VariantDict) -> Phone {
    let path = format!("/modules/kdeconnect/devices/{id}");
    let reachable = get::<bool>(d, "isReachable").unwrap_or(false);
    {
        let name = get::<String>(d, "name").unwrap_or_else(|| id.clone());
        let address = get::<Vec<String>>(d, "reachableAddresses").and_then(|a| a.into_iter().next());
        let kind = get::<String>(d, "type").unwrap_or_else(|| "phone".into());
        let mut phone = Phone { id: id.clone(), name, reachable, kind, address, battery: None, signal: None, notes: Vec::new() };
        if reachable {
            if let Some(b) = props(conn, &format!("{path}/battery"), "org.kde.kdeconnect.device.battery").await {
                if get::<bool>(&b, "hasBattery").unwrap_or(true) {
                    phone.battery = get::<i32>(&b, "charge").filter(|c| *c >= 0).map(|c| (c, get::<bool>(&b, "isCharging").unwrap_or(false)));
                }
            }
            if let Some(c) = props(conn, &format!("{path}/connectivity_report"), "org.kde.kdeconnect.device.connectivity_report").await {
                let kind = get::<String>(&c, "cellularNetworkType").unwrap_or_default();
                if !kind.is_empty() && kind != "Unknown" {
                    phone.signal = Some((kind, get::<i32>(&c, "cellularNetworkStrength").unwrap_or(0).clamp(0, 4)));
                }
            }
            let notes_path = format!("{path}/notifications");
            if let Some(v) = call_bus(conn, &notes_path, "org.kde.kdeconnect.device.notifications", "activeNotifications", None).await {
                let list: Vec<String> = v.child_value(0).get().unwrap_or_default();
                // Newest last on the daemon's side; show newest first.
                for nid in list.iter().rev().take(12) {
                    let npath = format!("{notes_path}/{nid}");
                    let Some(n) = props(conn, &npath, "org.kde.kdeconnect.device.notifications.notification").await else { continue };
                    let icon_path = if get::<bool>(&n, "hasIcon").unwrap_or(false) { get::<String>(&n, "iconPath").unwrap_or_default() } else { String::new() };
                    phone.notes.push(Note {
                        path: npath,
                        app: get::<String>(&n, "appName").unwrap_or_default(),
                        title: get::<String>(&n, "title").unwrap_or_default(),
                        text: get::<String>(&n, "text").unwrap_or_default().replace('\n', " "),
                        icon: icon(&icon_path),
                        dismissable: get::<bool>(&n, "dismissable").unwrap_or(false),
                    });
                }
            }
        }
        phone
    }
}

/// Re-reads the phone; requests that land while one is running are folded into one more pass.
pub fn refresh(app: &Rc<App>) {
    watch(app);
    let busy = STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.loading {
            s.again = true;
            return true;
        }
        s.loading = true;
        false
    });
    if busy {
        return;
    }
    let weak = Rc::downgrade(app);
    glib::spawn_future_local(async move {
        let result = match gio::bus_get_future(gio::BusType::Session).await {
            Ok(conn) => fetch(&conn, STATE.with(|s| s.borrow().chosen.clone())).await,
            Err(_) => (false, Vec::new(), None),
        };
        let again = STATE.with(|s| {
            let mut s = s.borrow_mut();
            (s.daemon, s.paired, s.phone) = result;
            s.loading = false;
            std::mem::take(&mut s.again)
        });
        let Some(app) = weak.upgrade() else { return };
        app.redraw();
        if let Some(c) = app.control.borrow().as_ref() {
            c.changed();
        }
        if again {
            refresh(&app);
        }
    });
}

/// Follows the daemon's signals (notifications posted or removed, battery, reachability, calls)
/// so the tab stays live without polling.
fn watch(app: &Rc<App>) {
    if STATE.with(|s| std::mem::replace(&mut s.borrow_mut().watching, true)) {
        return;
    }
    let weak: Weak<App> = Rc::downgrade(app);
    glib::spawn_future_local(async move {
        let Ok(conn) = gio::bus_get_future(gio::BusType::Session).await else { return };
        let pending = Rc::new(std::cell::Cell::new(false));
        let id = conn.subscribe_to_signal(Some(BUS), None, None, None, None, gio::DBusSignalFlags::NONE, move |sig| {
            if sig.interface_name == "org.kde.kdeconnect.device.telephony" && sig.signal_name == "callReceived" {
                let (event, number, contact): (String, String, String) = sig.parameters.get().unwrap_or_default();
                let who = if contact.is_empty() { number } else { contact };
                let label = match event.as_str() {
                    "ringing" => format!("Incoming call from {who}"),
                    "missedCall" => format!("Missed call from {who}"),
                    "talking" => format!("On a call with {who}"),
                    _ => return,
                };
                STATE.with(|s| s.borrow_mut().call = Some((label, glib::monotonic_time())));
            }
            if pending.replace(true) {
                return;
            }
            let (pending, weak) = (pending.clone(), weak.clone());
            glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || {
                pending.set(false);
                if let Some(app) = weak.upgrade() {
                    refresh(&app);
                }
            });
        });
        // The subscription lives as long as the shell.
        std::mem::forget(id);
    });
}

fn device_call(path: String, iface: &'static str, method: &'static str, args: Option<glib::Variant>) {
    glib::spawn_future_local(async move {
        if let Ok(conn) = gio::bus_get_future(gio::BusType::Session).await {
            let _ = call_bus(&conn, &path, iface, method, args).await;
        }
    });
}

pub fn act(app: &Rc<App>, action: Action) {
    let Some(p) = phone() else { return };
    let base = format!("/modules/kdeconnect/devices/{}", p.id);
    match action {
        Action::Ring => device_call(format!("{base}/findmyphone"), "org.kde.kdeconnect.device.findmyphone", "ring", None),
        Action::Ping => device_call(format!("{base}/ping"), "org.kde.kdeconnect.device.ping", "sendPing", None),
        Action::Clipboard => device_call(format!("{base}/clipboard"), "org.kde.kdeconnect.device.clipboard", "sendClipboard", None),
        Action::Browse => device_call(format!("{base}/sftp"), "org.kde.kdeconnect.device.sftp", "startBrowsing", None),
        Action::Mirror => mirror(app, &p),
    }
}

/// Mirrors the phone's screen with scrcpy: over USB when adb sees the phone plugged in,
/// otherwise over Wi-Fi at the address KDE Connect knows (which needs wireless debugging).
fn mirror(app: &Rc<App>, p: &Phone) {
    let weak = Rc::downgrade(app);
    if glib::find_program_in_path("scrcpy").is_none() {
        set_note(&weak, "Install scrcpy to mirror the screen");
        return;
    }
    let (name, address) = (p.name.clone(), p.address.clone());
    set_note(&weak, "Starting screen mirror…");
    glib::spawn_future_local(async move {
        let usb = gio::spawn_blocking(|| {
            std::process::Command::new("adb").arg("devices").output().ok().is_some_and(|o| String::from_utf8_lossy(&o.stdout).lines().skip(1).any(|l| l.trim_end().ends_with("\tdevice")))
        })
        .await
        .unwrap_or(false);
        let mut argv = vec!["scrcpy".to_string(), format!("--window-title={name}")];
        match (usb, address) {
            (true, _) => {}
            (false, Some(ip)) => argv.push(format!("--tcpip={ip}")),
            (false, None) => {
                set_note(&weak, "Plug the phone in over USB to mirror it");
                return;
            }
        }
        let os: Vec<&std::ffi::OsStr> = argv.iter().map(std::ffi::OsStr::new).collect();
        let Ok(child) = gio::Subprocess::newv(&os, gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE) else {
            set_note(&weak, "Couldn't start scrcpy");
            return;
        };
        let started = glib::monotonic_time();
        let ok = child.wait_check_future().await.is_ok();
        // A quick failure means it never connected; a long run that ends is just the window closing.
        if !ok && glib::monotonic_time() - started < 15_000_000 {
            let why = if usb { "Mirroring failed. Allow USB debugging on the phone" } else { "Couldn't reach the phone. Plug it in over USB or turn on wireless debugging" };
            set_note(&weak, why);
        }
    });
}

pub fn dismiss(note: &Note) {
    device_call(note.path.clone(), "org.kde.kdeconnect.device.notifications.notification", "dismiss", None);
}
