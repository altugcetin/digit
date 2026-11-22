//! Phones and tablets in Devices: those KDE Connect reaches, whose files open over its SFTP
//! link, and one plugged in over USB while GVfs can't read phones yet, which installs that.

use gtk::{gio, glib, prelude::*};
use std::path::{Path, PathBuf};
use std::time::Duration;

const KDE: &str = "org.kde.kdeconnect";
const DAEMON: &str = "/modules/kdeconnect";

/// A device KDE Connect reaches that lends its files.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Kde {
    pub id: String,
    pub name: String,
    pub tablet: bool,
    pub mounted: bool,
}

async fn call(path: &str, iface: &str, method: &str, args: Option<glib::Variant>, wait: Duration) -> Option<glib::Variant> {
    let conn = gio::bus_get_future(gio::BusType::Session).await.ok()?;
    conn.call_future(Some(KDE), path, iface, method, args.as_ref(), None, gio::DBusCallFlags::NONE, wait.as_millis() as i32).await.ok()
}

async fn prop(path: &str, name: &str) -> Option<glib::Variant> {
    let v = call(path, "org.freedesktop.DBus.Properties", "Get", Some(("org.kde.kdeconnect.device", name).to_variant()), Duration::from_secs(2)).await?;
    v.child_value(0).as_variant()
}

fn first<T: glib::variant::FromVariant>(v: Option<glib::Variant>) -> Option<T> {
    v.and_then(|v| v.child_value(0).get())
}

/// The paired devices KDE Connect reaches now that share their files.
pub(super) async fn kde() -> Vec<Kde> {
    let Some(ids) = first::<Vec<String>>(call(DAEMON, "org.kde.kdeconnect.daemon", "devices", Some((true, true).to_variant()), Duration::from_secs(2)).await) else { return Vec::new() };
    let mut out = Vec::new();
    for id in ids {
        let path = format!("{DAEMON}/devices/{id}");
        let lends = first::<bool>(call(&path, "org.kde.kdeconnect.device", "hasPlugin", Some(("kdeconnect_sftp",).to_variant()), Duration::from_secs(2)).await).unwrap_or(false);
        if !lends {
            continue;
        }
        let name = prop(&path, "name").await.and_then(|v| v.get::<String>()).unwrap_or_else(|| id.clone());
        let tablet = prop(&path, "type").await.and_then(|v| v.get::<String>()).is_some_and(|t| t == "tablet");
        let mounted = first::<bool>(call(&format!("{path}/sftp"), "org.kde.kdeconnect.device.sftp", "isMounted", None, Duration::from_secs(2)).await).unwrap_or(false);
        out.push(Kde { id, name, tablet, mounted });
    }
    out
}

/// Mounts the device `id`'s files and gives where they are: its storage KDE Connect names
/// first, else where it mounted them.
pub(super) async fn open(id: &str) -> Result<gio::File, String> {
    let path = format!("{DAEMON}/devices/{id}/sftp");
    let iface = "org.kde.kdeconnect.device.sftp";
    let mounted = first::<bool>(call(&path, iface, "mountAndWait", None, Duration::from_secs(30)).await).unwrap_or(false);
    if !mounted {
        let why = first::<String>(call(&path, iface, "getMountError", None, Duration::from_secs(2)).await).filter(|e| !e.trim().is_empty());
        return Err(why.unwrap_or_else(|| "The device didn't answer".into()));
    }
    let dirs = call(&path, iface, "getDirectories", None, Duration::from_secs(5)).await.map(|v| v.child_value(0));
    let named = dirs.and_then(|d| (0..d.n_children()).find_map(|i| d.child_value(i).child_value(0).get::<String>()));
    let at = match named {
        Some(p) => p,
        None => first::<String>(call(&path, iface, "mountPoint", None, Duration::from_secs(2)).await).ok_or("The device's files can't be found")?,
    };
    Ok(gio::File::for_path(at))
}

/// Unmounts the device `id`'s files.
pub(super) async fn close(id: &str) {
    let _ = call(&format!("{DAEMON}/devices/{id}/sftp"), "org.kde.kdeconnect.device.sftp", "unmount", None, Duration::from_secs(5)).await;
}

/// Calls `changed` whenever a device of KDE Connect's comes, goes, or mounts or unmounts its
/// files. The subscription lives as long as the connection.
pub(super) fn watch(changed: impl Fn() + 'static) {
    glib::spawn_future_local(async move {
        let Ok(conn) = gio::bus_get_future(gio::BusType::Session).await else { return };
        // Only what says devices came, went, or mounted or unmounted their files.
        let watching = conn.subscribe_to_signal(Some(KDE), None, None, None, None, gio::DBusSignalFlags::NONE, move |sig| {
            if matches!(sig.signal_name, "deviceAdded" | "deviceRemoved" | "deviceVisibilityChanged" | "deviceListChanged" | "reachableChanged" | "pairStateChanged" | "mounted" | "unmounted" | "pluginsChanged") {
                changed();
            }
        });
        // Kept for the life of the program, as the sidebar is.
        std::mem::forget(watching);
    });
}

/// What a phone plugged in over USB needs before GVfs can read it, when it does: its name in
/// Devices and the packages that teach GVfs, by the names distributions give them.
pub(super) fn waiting() -> Option<(&'static str, &'static [&'static str])> {
    let monitors = Path::new("/usr/share/gvfs/remote-volume-monitors");
    let (mut phone, mut apple) = (false, false);
    for dev in std::fs::read_dir("/sys/bus/usb/devices").ok()?.flatten() {
        let p = dev.path();
        let read = |name: &str| std::fs::read_to_string(p.join(name)).unwrap_or_default().trim().to_string();
        // An interface: still images (MTP and PTP), or one that calls itself MTP.
        if p.join("bInterfaceClass").exists() {
            phone |= read("bInterfaceClass") == "06" || read("interface").eq_ignore_ascii_case("MTP");
        } else if read("idVendor") == "05ac" && read("idProduct").starts_with("12") {
            apple = true;
        }
    }
    if apple && !monitors.join("afc.monitor").exists() {
        return Some(("iPhone", &["gvfs-afc", "gvfs-backends"]));
    }
    if phone && !monitors.join("mtp.monitor").exists() {
        return Some(("Phone", &["gvfs-mtp", "gvfs-backends"]));
    }
    None
}

/// The folders USB devices show up in, to notice a phone being plugged in.
pub(super) fn usb_buses() -> Vec<PathBuf> {
    std::fs::read_dir("/dev/bus/usb").map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default()
}

/// Installs `names` that the distribution has and aren't installed, through PackageKit.
pub(super) async fn install(names: &[&str]) -> Result<(), String> {
    let found = kit::pk::resolve(names, kit::pk::FILTER_NONE).await?;
    let ids: Vec<String> = found.iter().filter(|p| !p.installed()).map(|p| p.id.clone()).collect();
    if ids.is_empty() {
        return if found.is_empty() { Err("This system has no package for it".into()) } else { Ok(()) };
    }
    kit::pk::install(&ids, kit::pk::ONLY_TRUSTED, |_| {}).await.map(|_| ())
}
