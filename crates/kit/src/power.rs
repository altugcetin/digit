//! Power modes and the battery: the power profiles service (power-profiles-daemon, or tuned-ppd,
//! which answers the same way), the mode chosen for battery and for plugged in, and the battery's
//! charge limit through UPower. The services check the caller themselves (polkit lets the active
//! session's user switch modes and set the limit), so nothing here needs root.

use std::collections::HashMap;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

pub const SAVER: &str = "power-saver";
pub const BALANCED: &str = "balanced";
pub const PERFORMANCE: &str = "performance";
/// Every mode, from the least power to the most.
pub const MODES: [&str; 3] = [SAVER, BALANCED, PERFORMANCE];

/// The mode used on battery, and the one used plugged in.
pub const ON_BATTERY: &str = "power_on_battery";
pub const PLUGGED_IN: &str = "power_plugged_in";
/// Power Saver while the battery is low, until the charger is plugged in.
pub const LOW_SAVER: &str = "power_low_saver";
/// The shell draws without blur, shadows or the music visualizer in Power Saver.
pub const LIGHT: &str = "power_light";
/// At or below this many percent the battery counts as low.
pub const LOW: f64 = 20.0;

pub const IDLE_OFF_BATTERY: &str = "idle_off_battery";
pub const IDLE_SLEEP_BATTERY: &str = "idle_sleep_battery";
pub const IDLE_OFF_BATTERY_DEFAULT: f64 = 5.0;
pub const IDLE_SLEEP_BATTERY_DEFAULT: f64 = 15.0;

pub fn battery_idle(t: &toml::Table) -> (f64, f64) {
    (crate::prefs::number(t, IDLE_OFF_BATTERY, IDLE_OFF_BATTERY_DEFAULT).max(0.0), crate::prefs::number(t, IDLE_SLEEP_BATTERY, IDLE_SLEEP_BATTERY_DEFAULT).max(0.0))
}

pub fn label(mode: &str) -> &'static str {
    match mode {
        SAVER => "Power Saver",
        PERFORMANCE => "Performance",
        _ => "Balanced",
    }
}

/// The mode's name where room is short, as on Control Center's buttons.
pub fn short(mode: &str) -> &'static str {
    match mode {
        SAVER => "Saver",
        PERFORMANCE => "Performance",
        _ => "Balanced",
    }
}

pub fn glyph(mode: &str) -> &'static str {
    match mode {
        SAVER => "eco",
        PERFORMANCE => "speed",
        _ => "balance",
    }
}

/// What was chosen for power, read from the settings in one place for the shell and Settings.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub battery: String,
    pub plugged: String,
    pub low_saver: bool,
    pub light: bool,
}

impl Choice {
    pub fn read(t: &toml::Table) -> Self {
        let mode = |key| Some(crate::prefs::text(t, key, BALANCED)).filter(|m| MODES.contains(&m.as_str())).unwrap_or_else(|| BALANCED.into());
        Self { battery: mode(ON_BATTERY), plugged: mode(PLUGGED_IN), low_saver: crate::prefs::flag(t, LOW_SAVER, true), light: crate::prefs::flag(t, LIGHT, true) }
    }

    /// The mode for the power source in use.
    pub fn for_source(&self, on_battery: bool) -> &str {
        if on_battery { &self.battery } else { &self.plugged }
    }

    pub fn key(on_battery: bool) -> &'static str {
        if on_battery { ON_BATTERY } else { PLUGGED_IN }
    }
}

/// The mode after `active` among those the computer has, for a button that steps through them.
pub fn next(active: &str, list: &[String]) -> Option<String> {
    let at = list.iter().position(|m| m == active);
    list.get(at.map_or(0, |i| (i + 1) % list.len())).cloned()
}

/// `wanted` when the computer has it; otherwise Balanced, which every profile service has.
pub fn usable(wanted: &str, list: &[String]) -> String {
    if list.iter().any(|m| m == wanted) { wanted.into() } else { BALANCED.into() }
}

/// The power profiles service as found: where it answers, the mode now, the modes this computer
/// has (least power first), why Performance is held back if it is, and whether something holds
/// a mode.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Profiles {
    pub bus: String,
    pub path: String,
    pub active: String,
    pub list: Vec<String>,
    pub degraded: String,
    pub held: bool,
}

/// Where the service answers: the current name, then the name older versions and tuned-ppd use.
const SERVICES: [(&str, &str); 2] = [("org.freedesktop.UPower.PowerProfiles", "/org/freedesktop/UPower/PowerProfiles"), ("net.hadess.PowerProfiles", "/net/hadess/PowerProfiles")];

async fn proxy(conn: &zbus::Connection, bus: &str, path: &str, iface: &str) -> Option<zbus::Proxy<'static>> {
    zbus::proxy::Builder::new(conn)
        .destination(bus.to_owned())
        .ok()?
        .path(path.to_owned())
        .ok()?
        .interface(iface.to_owned())
        .ok()?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
        .ok()
}

/// The power profiles service; `None` when the computer has none.
pub async fn profiles(conn: &zbus::Connection) -> Option<Profiles> {
    for (bus, path) in SERVICES {
        let Some(p) = proxy(conn, bus, path, bus).await else { continue };
        // A service that isn't installed fails here at once; give a slow one a moment only.
        let active = p.get_property::<String>("ActiveProfile");
        let wait = gtk::glib::timeout_future(std::time::Duration::from_millis(1500));
        let active = match futures_util::future::select(Box::pin(active), Box::pin(wait)).await {
            futures_util::future::Either::Left((Ok(a), _)) => a,
            _ => continue,
        };
        let listed: Vec<String> = p
            .get_property::<Vec<HashMap<String, OwnedValue>>>("Profiles")
            .await
            .map(|v| v.iter().filter_map(|m| m.get("Profile").and_then(|x| x.downcast_ref::<String>().ok())).collect())
            .unwrap_or_default();
        let list = MODES.iter().filter(|m| listed.iter().any(|l| l == *m)).map(|m| m.to_string()).collect();
        let degraded = p.get_property::<String>("PerformanceDegraded").await.unwrap_or_default();
        let held = p.get_property::<Vec<HashMap<String, OwnedValue>>>("ActiveProfileHolds").await.is_ok_and(|h| !h.is_empty());
        return Some(Profiles { bus: bus.into(), path: path.into(), active, list, degraded, held });
    }
    None
}

/// Switches to `mode`. Setting a mode also lets go of any mode held (a low battery's).
pub async fn set(conn: &zbus::Connection, p: &Profiles, mode: &str) -> Result<(), String> {
    let props = zbus::fdo::PropertiesProxy::builder(conn)
        .destination(p.bus.clone())
        .and_then(|b| b.path(p.path.clone()))
        .map_err(|e| e.to_string())?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
        .map_err(|e| e.to_string())?;
    let iface = zbus::names::InterfaceName::try_from(p.bus.clone()).map_err(|e| e.to_string())?;
    props.set(iface, "ActiveProfile", zbus::zvariant::Value::from(mode)).await.map_err(|e| e.to_string())
}

/// Holds `mode` until released (or until this connection closes, so it never outlives the shell);
/// the number given back releases it.
pub async fn hold(conn: &zbus::Connection, p: &Profiles, mode: &str, reason: &str) -> Result<u32, String> {
    let proxy = proxy(conn, &p.bus, &p.path, &p.bus).await.ok_or("no power profiles service")?;
    proxy.call::<_, _, u32>("HoldProfile", &(mode, reason, "dev.digit.Shell")).await.map_err(|e| e.to_string())
}

pub async fn release(conn: &zbus::Connection, p: &Profiles, cookie: u32) -> Result<(), String> {
    let proxy = proxy(conn, &p.bus, &p.path, &p.bus).await.ok_or("no power profiles service")?;
    proxy.call::<_, _, ()>("ReleaseProfile", &(cookie,)).await.map_err(|e| e.to_string())
}

const UPOWER: &str = "org.freedesktop.UPower";
const DISPLAY: &str = "/org/freedesktop/UPower/devices/DisplayDevice";

/// Where the power comes from: whether the computer runs on battery, whether it has one, and how
/// full it is.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Source {
    pub on_battery: bool,
    pub battery: bool,
    pub level: f64,
}

pub async fn source(conn: &zbus::Connection) -> Option<Source> {
    let root = proxy(conn, UPOWER, "/org/freedesktop/UPower", UPOWER).await?;
    let on_battery = root.get_property::<bool>("OnBattery").await.ok()?;
    let mut s = Source { on_battery, ..Default::default() };
    if let Some(d) = proxy(conn, UPOWER, DISPLAY, "org.freedesktop.UPower.Device").await {
        // Type 2 is a battery; the display device stands for all of them together.
        s.battery = d.get_property::<bool>("IsPresent").await.unwrap_or(false) && d.get_property::<u32>("Type").await.unwrap_or(0) == 2;
        s.level = d.get_property::<f64>("Percentage").await.unwrap_or(0.0);
    }
    Some(s)
}

/// Every signal of UPower (the charger, the battery's level) and of the power profiles service,
/// under either of its names, and the service starting or stopping (installed, or restarted by
/// an update, which also ends its holds); read them again on these.
pub async fn changes(conn: &zbus::Connection) -> Option<futures_util::stream::LocalBoxStream<'static, zbus::Result<zbus::Message>>> {
    use futures_util::StreamExt;
    let rule = |path: &'static str| zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).path_namespace(path).map(|b| b.build());
    let upower = zbus::MessageStream::for_match_rule(rule("/org/freedesktop/UPower").ok()?, conn, Some(32)).await.ok()?;
    let legacy = zbus::MessageStream::for_match_rule(rule("/net/hadess/PowerProfiles").ok()?, conn, Some(16)).await.ok()?;
    let owner = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.DBus")
        .and_then(|b| b.member("NameOwnerChanged"))
        .and_then(|b| b.arg(0, SERVICES[0].0))
        .ok()?
        .build();
    let owner = zbus::MessageStream::for_match_rule(owner, conn, Some(4)).await.ok()?;
    Some(futures_util::stream::select(futures_util::stream::select(upower, legacy), owner).boxed_local())
}

/// The battery's charge limit: whether it has one, whether it's on, and where charging stops and
/// starts again.
#[derive(Clone, Debug, PartialEq)]
pub struct Limit {
    pub battery: String,
    pub on: bool,
    pub start: u32,
    pub end: u32,
}

/// The first battery that can limit its charge.
pub async fn limit(conn: &zbus::Connection) -> Option<Limit> {
    let root = proxy(conn, UPOWER, "/org/freedesktop/UPower", UPOWER).await?;
    let devices: Vec<OwnedObjectPath> = root.call("EnumerateDevices", &()).await.ok()?;
    for path in devices {
        let Some(d) = proxy(conn, UPOWER, path.as_str(), "org.freedesktop.UPower.Device").await else { continue };
        if d.get_property::<u32>("Type").await.unwrap_or(0) != 2 || !d.get_property::<bool>("ChargeThresholdSupported").await.unwrap_or(false) {
            continue;
        }
        return Some(Limit {
            battery: path.to_string(),
            on: d.get_property::<bool>("ChargeThresholdEnabled").await.unwrap_or(false),
            start: d.get_property::<u32>("ChargeStartThreshold").await.unwrap_or(0),
            end: d.get_property::<u32>("ChargeEndThreshold").await.unwrap_or(100),
        });
    }
    None
}

pub async fn set_limit(conn: &zbus::Connection, limit: &Limit, on: bool) -> Result<(), String> {
    let d = proxy(conn, UPOWER, &limit.battery, "org.freedesktop.UPower.Device").await.ok_or("battery not found")?;
    d.call::<_, _, ()>("EnableChargeThreshold", &(on,)).await.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(modes: &[&str]) -> Vec<String> {
        modes.iter().map(|m| m.to_string()).collect()
    }

    #[test]
    fn the_button_steps_through_the_modes_the_computer_has() {
        let all = list(&MODES);
        assert_eq!(next(SAVER, &all).as_deref(), Some(BALANCED));
        assert_eq!(next(BALANCED, &all).as_deref(), Some(PERFORMANCE));
        assert_eq!(next(PERFORMANCE, &all).as_deref(), Some(SAVER));
        let two = list(&[SAVER, BALANCED]);
        assert_eq!(next(BALANCED, &two).as_deref(), Some(SAVER));
        // A mode it doesn't list (another tool's) starts it from the first.
        assert_eq!(next("custom", &two).as_deref(), Some(SAVER));
        assert_eq!(next(SAVER, &[]), None);
    }

    #[test]
    fn on_battery_displays_go_off_after_five_minutes_and_it_sleeps_after_fifteen() {
        assert_eq!(battery_idle(&toml::Table::new()), (5.0, 15.0));
        let t: toml::Table = toml::from_str("idle_off_battery = 2.0\nidle_sleep_battery = 0.0").unwrap();
        assert_eq!(battery_idle(&t), (2.0, 0.0));
        let t: toml::Table = toml::from_str("idle_off_battery = -3.0").unwrap();
        assert_eq!(battery_idle(&t).0, 0.0);
    }

    #[test]
    fn a_mode_the_computer_lacks_falls_back_to_balanced() {
        let two = list(&[SAVER, BALANCED]);
        assert_eq!(usable(PERFORMANCE, &two), BALANCED);
        assert_eq!(usable(SAVER, &two), SAVER);
    }

    #[test]
    fn choices_default_to_balanced_and_ignore_unknown_modes() {
        let c = Choice::read(&toml::Table::new());
        assert_eq!((c.battery.as_str(), c.plugged.as_str(), c.low_saver, c.light), (BALANCED, BALANCED, true, true));
        let t: toml::Table = toml::from_str("power_on_battery = 'power-saver'\npower_plugged_in = 'turbo'\npower_low_saver = false").unwrap();
        let c = Choice::read(&t);
        assert_eq!((c.for_source(true), c.for_source(false), c.low_saver), (SAVER, BALANCED, false));
        assert_eq!((Choice::key(true), Choice::key(false)), (ON_BATTERY, PLUGGED_IN));
    }
}
