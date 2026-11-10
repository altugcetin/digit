use std::collections::HashMap;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

const NM: &str = "org.freedesktop.NetworkManager";
const ROOT: &str = "/org/freedesktop/NetworkManager";
const SETTINGS: &str = "/org/freedesktop/NetworkManager/Settings";

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Security {
    #[default]
    Open,
    Owe,
    Wep,
    Wpa,
    Wpa2,
    Wpa3,
    Mixed,
    Enterprise,
}

impl Security {
    pub fn label(self) -> &'static str {
        match self {
            Security::Open => "Open",
            Security::Owe => "Enhanced Open",
            Security::Wep => "WEP",
            Security::Wpa => "WPA",
            Security::Wpa2 => "WPA2",
            Security::Wpa3 => "WPA3",
            Security::Mixed => "WPA2/WPA3",
            Security::Enterprise => "Enterprise",
        }
    }

    pub fn locked(self) -> bool {
        !matches!(self, Security::Open | Security::Owe)
    }

    fn from_flags(flags: u32, wpa: u32, rsn: u32) -> Self {
        let all = wpa | rsn;
        if all & (0x200 | 0x2000) != 0 {
            Security::Enterprise
        } else if all & 0x100 != 0 && all & 0x400 != 0 {
            Security::Mixed
        } else if all & 0x400 != 0 {
            Security::Wpa3
        } else if rsn & 0x100 != 0 {
            Security::Wpa2
        } else if wpa & 0x100 != 0 {
            Security::Wpa
        } else if all & 0x800 != 0 {
            Security::Owe
        } else if flags & 1 != 0 {
            Security::Wep
        } else {
            Security::Open
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Network {
    pub ssid: String,
    pub ap: String,
    pub strength: u8,
    pub frequency: u32,
    pub security: Security,
    pub saved: Option<String>,
    pub active: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Saved {
    pub path: String,
    pub id: String,
    pub ssid: String,
    pub used: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Details {
    pub ssid: String,
    pub security: Security,
    pub strength: u8,
    pub frequency: u32,
    pub bitrate: u32,
    pub address: String,
    pub gateway: String,
    pub dns: Vec<String>,
    pub ipv6: String,
    pub mac: String,
    pub bssid: String,
}

impl Details {
    pub fn band(&self) -> String {
        band(self.frequency)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Wifi {
    pub enabled: bool,
    pub hardware: bool,
    pub device: Option<String>,
    pub interface: String,
    pub state: u32,
    pub networks: Vec<Network>,
    pub saved: Vec<Saved>,
    pub current: Option<Details>,
}

impl Wifi {
    pub fn connecting(&self) -> bool {
        (40..100).contains(&self.state)
    }
}

pub fn band(frequency: u32) -> String {
    match frequency {
        0 => String::new(),
        f if f < 3000 => "2.4 GHz".into(),
        f if f < 5925 => "5 GHz".into(),
        _ => "6 GHz".into(),
    }
}

pub fn bars(strength: u8) -> &'static str {
    match strength {
        0..=19 => "signal_wifi_0_bar",
        20..=44 => "network_wifi_1_bar",
        45..=64 => "network_wifi_2_bar",
        65..=79 => "network_wifi_3_bar",
        _ => "signal_wifi_4_bar",
    }
}

async fn proxy(conn: &zbus::Connection, path: &str, iface: &'static str) -> Result<zbus::Proxy<'static>, String> {
    zbus::proxy::Builder::new(conn)
        .destination(NM)
        .and_then(|b| b.path(path.to_string()))
        .and_then(|b| b.interface(iface))
        .map(|b| b.cache_properties(zbus::proxy::CacheProperties::No))
        .map_err(|e| e.to_string())?
        .build()
        .await
        .map_err(|e| e.to_string())
}

fn tidy(e: zbus::Error) -> String {
    match &e {
        zbus::Error::MethodError(_, Some(detail), _) => detail.clone(),
        _ => e.to_string(),
    }
}

fn ssid_text(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw).trim_end_matches('\0').to_string()
}

async fn saved(conn: &zbus::Connection) -> Vec<Saved> {
    let mut out = Vec::new();
    let Ok(settings) = proxy(conn, SETTINGS, "org.freedesktop.NetworkManager.Settings").await else { return out };
    let Ok(paths) = settings.call::<_, _, Vec<OwnedObjectPath>>("ListConnections", &()).await else { return out };
    for path in paths {
        let Ok(c) = proxy(conn, path.as_str(), "org.freedesktop.NetworkManager.Settings.Connection").await else { continue };
        let Ok(s) = c.call::<_, _, HashMap<String, HashMap<String, OwnedValue>>>("GetSettings", &()).await else { continue };
        let Some(wifi) = s.get("802-11-wireless") else { continue };
        let ssid = wifi.get("ssid").and_then(|v| Vec::<u8>::try_from(v.try_clone().ok()?).ok()).map(|b| ssid_text(&b)).unwrap_or_default();
        let base = s.get("connection");
        let id = base.and_then(|c| c.get("id")).and_then(|v| String::try_from(v.try_clone().ok()?).ok()).unwrap_or_else(|| ssid.clone());
        let used = base.and_then(|c| c.get("timestamp")).and_then(|v| u64::try_from(v.try_clone().ok()?).ok()).unwrap_or(0);
        out.push(Saved { path: path.to_string(), id, ssid, used });
    }
    out.sort_by(|a, b| b.used.cmp(&a.used).then(a.id.to_lowercase().cmp(&b.id.to_lowercase())));
    out
}

struct Ap {
    path: String,
    ssid: String,
    strength: u8,
    frequency: u32,
    security: Security,
    bssid: String,
}

async fn ap(conn: &zbus::Connection, path: &str) -> Option<Ap> {
    let p = proxy(conn, path, "org.freedesktop.NetworkManager.AccessPoint").await.ok()?;
    let ssid = ssid_text(&p.get_property::<Vec<u8>>("Ssid").await.ok()?);
    let flags = p.get_property::<u32>("Flags").await.unwrap_or(0);
    let wpa = p.get_property::<u32>("WpaFlags").await.unwrap_or(0);
    let rsn = p.get_property::<u32>("RsnFlags").await.unwrap_or(0);
    Some(Ap {
        path: path.to_string(),
        ssid,
        strength: p.get_property::<u8>("Strength").await.unwrap_or(0),
        frequency: p.get_property::<u32>("Frequency").await.unwrap_or(0),
        security: Security::from_flags(flags, wpa, rsn),
        bssid: p.get_property::<String>("HwAddress").await.unwrap_or_default(),
    })
}

async fn ip(conn: &zbus::Connection, dev: &zbus::Proxy<'_>, d: &mut Details) {
    if let Ok(path) = dev.get_property::<OwnedObjectPath>("Ip4Config").await {
        if path.as_str() != "/" {
            if let Ok(c) = proxy(conn, path.as_str(), "org.freedesktop.NetworkManager.IP4Config").await {
                let list = c.get_property::<Vec<HashMap<String, OwnedValue>>>("AddressData").await.unwrap_or_default();
                if let Some(a) = list.first() {
                    let addr = a.get("address").and_then(|v| String::try_from(v.try_clone().ok()?).ok()).unwrap_or_default();
                    let prefix = a.get("prefix").and_then(|v| u32::try_from(v.try_clone().ok()?).ok()).unwrap_or(0);
                    d.address = format!("{addr}/{prefix}");
                }
                d.gateway = c.get_property::<String>("Gateway").await.unwrap_or_default();
                d.dns = c
                    .get_property::<Vec<HashMap<String, OwnedValue>>>("NameserverData")
                    .await
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|m| m.get("address").and_then(|v| String::try_from(v.try_clone().ok()?).ok()))
                    .collect();
            }
        }
    }
    if let Ok(path) = dev.get_property::<OwnedObjectPath>("Ip6Config").await {
        if path.as_str() != "/" {
            if let Ok(c) = proxy(conn, path.as_str(), "org.freedesktop.NetworkManager.IP6Config").await {
                let list = c.get_property::<Vec<HashMap<String, OwnedValue>>>("AddressData").await.unwrap_or_default();
                d.ipv6 = list
                    .iter()
                    .filter_map(|a| a.get("address").and_then(|v| String::try_from(v.try_clone().ok()?).ok()))
                    .find(|a| !a.starts_with("fe80"))
                    .unwrap_or_default();
            }
        }
    }
}

pub async fn read(conn: &zbus::Connection) -> Wifi {
    let mut w = Wifi::default();
    let Ok(root) = proxy(conn, ROOT, NM).await else { return w };
    w.enabled = root.get_property::<bool>("WirelessEnabled").await.unwrap_or(false);
    w.hardware = root.get_property::<bool>("WirelessHardwareEnabled").await.unwrap_or(true);
    let mut dev = None;
    for path in root.get_property::<Vec<OwnedObjectPath>>("Devices").await.unwrap_or_default() {
        let Ok(d) = proxy(conn, path.as_str(), "org.freedesktop.NetworkManager.Device").await else { continue };
        if d.get_property::<u32>("DeviceType").await.unwrap_or(0) == 2 {
            dev = Some((path.to_string(), d));
            break;
        }
    }
    w.saved = saved(conn).await;
    let Some((path, dev)) = dev else { return w };
    w.interface = dev.get_property::<String>("Interface").await.unwrap_or_default();
    w.state = dev.get_property::<u32>("State").await.unwrap_or(0);
    w.device = Some(path.clone());
    let Ok(wl) = proxy(conn, &path, "org.freedesktop.NetworkManager.Device.Wireless").await else { return w };
    let active = wl.get_property::<OwnedObjectPath>("ActiveAccessPoint").await.ok().filter(|p| p.as_str() != "/").map(|p| p.to_string());
    let mut best: HashMap<String, Network> = HashMap::new();
    if w.enabled {
        for p in wl.call::<_, _, Vec<OwnedObjectPath>>("GetAllAccessPoints", &()).await.unwrap_or_default() {
            let Some(a) = ap(conn, p.as_str()).await else { continue };
            let is_active = active.as_deref() == Some(a.path.as_str());
            if is_active && w.state == 100 {
                let mut d = Details { ssid: a.ssid.clone(), security: a.security, strength: a.strength, frequency: a.frequency, bssid: a.bssid.clone(), ..Default::default() };
                d.bitrate = wl.get_property::<u32>("Bitrate").await.unwrap_or(0);
                d.mac = dev.get_property::<String>("HwAddress").await.or(wl.get_property::<String>("HwAddress").await).unwrap_or_default();
                ip(conn, &dev, &mut d).await;
                w.current = Some(d);
            }
            if a.ssid.is_empty() {
                continue;
            }
            let entry = best.entry(a.ssid.clone()).or_default();
            if is_active || (!entry.active && a.strength > entry.strength) {
                *entry = Network { ssid: a.ssid.clone(), ap: a.path, strength: a.strength, frequency: a.frequency, security: a.security, saved: None, active: is_active && w.state == 100 };
            }
        }
    }
    let mut nets: Vec<Network> = best.into_values().collect();
    for n in &mut nets {
        n.saved = w.saved.iter().find(|s| s.ssid == n.ssid).map(|s| s.path.clone());
    }
    nets.sort_by(|a, b| (b.active, b.saved.is_some()).cmp(&(a.active, a.saved.is_some())).then(b.strength.cmp(&a.strength)));
    w.networks = nets;
    w
}

pub async fn set_enabled(conn: &zbus::Connection, on: bool) -> Result<(), String> {
    proxy(conn, ROOT, NM).await?.set_property("WirelessEnabled", on).await.map_err(|e| e.to_string())
}

pub async fn scan(conn: &zbus::Connection, device: &str) -> Result<(), String> {
    let wl = proxy(conn, device, "org.freedesktop.NetworkManager.Device.Wireless").await?;
    let opts: HashMap<&str, Value> = HashMap::new();
    wl.call_method("RequestScan", &(opts,)).await.map(|_| ()).map_err(tidy)
}

pub async fn disconnect(conn: &zbus::Connection, device: &str) -> Result<(), String> {
    proxy(conn, device, "org.freedesktop.NetworkManager.Device").await?.call_method("Disconnect", &()).await.map(|_| ()).map_err(tidy)
}

pub async fn forget(conn: &zbus::Connection, saved: &str) -> Result<(), String> {
    proxy(conn, saved, "org.freedesktop.NetworkManager.Settings.Connection").await?.call_method("Delete", &()).await.map(|_| ()).map_err(tidy)
}

/// Joins `net`: a saved network is activated as it is, a new one is added with `password`.
/// Waits until the link is up or has failed; a new profile that failed is removed again.
pub async fn join(conn: &zbus::Connection, device: &str, net: &Network, password: Option<&str>) -> Result<(), String> {
    let root = proxy(conn, ROOT, NM).await?;
    let dev = ObjectPath::try_from(device).map_err(|e| e.to_string())?;
    let ap = ObjectPath::try_from(net.ap.as_str()).map_err(|e| e.to_string())?;
    let (active, created) = match &net.saved {
        Some(saved) if password.is_none() => {
            let saved = ObjectPath::try_from(saved.as_str()).map_err(|e| e.to_string())?;
            let a: OwnedObjectPath = root.call("ActivateConnection", &(saved, &dev, &ap)).await.map_err(tidy)?;
            (a, None)
        }
        _ => {
            let mut base: HashMap<&str, Value> = HashMap::new();
            base.insert("id", Value::from(net.ssid.clone()));
            base.insert("type", Value::from("802-11-wireless"));
            let mut wifi: HashMap<&str, Value> = HashMap::new();
            wifi.insert("ssid", Value::from(net.ssid.as_bytes().to_vec()));
            wifi.insert("mode", Value::from("infrastructure"));
            let mut all: HashMap<&str, HashMap<&str, Value>> = HashMap::new();
            all.insert("connection", base);
            all.insert("802-11-wireless", wifi);
            let key = password.unwrap_or_default().to_string();
            let mut sec: HashMap<&str, Value> = HashMap::new();
            match net.security {
                Security::Open => {}
                Security::Owe => {
                    sec.insert("key-mgmt", Value::from("owe"));
                }
                Security::Wep => {
                    sec.insert("key-mgmt", Value::from("none"));
                    let raw = matches!(key.len(), 5 | 13) || (matches!(key.len(), 10 | 26) && key.chars().all(|c| c.is_ascii_hexdigit()));
                    sec.insert("wep-key-type", Value::from(if raw { 1u32 } else { 2u32 }));
                    sec.insert("wep-key0", Value::from(key));
                }
                Security::Wpa3 => {
                    sec.insert("key-mgmt", Value::from("sae"));
                    sec.insert("psk", Value::from(key));
                }
                Security::Wpa | Security::Wpa2 | Security::Mixed => {
                    sec.insert("key-mgmt", Value::from("wpa-psk"));
                    sec.insert("psk", Value::from(key));
                }
                Security::Enterprise => return Err("Enterprise networks need nmtui or nm-connection-editor".into()),
            }
            if !sec.is_empty() {
                all.insert("802-11-wireless-security", sec);
            }
            let (saved, active): (OwnedObjectPath, OwnedObjectPath) = root.call("AddAndActivateConnection", &(all, &dev, &ap)).await.map_err(tidy)?;
            (active, Some(saved.to_string()))
        }
    };
    let mut state = 0u32;
    for _ in 0..90 {
        gtk::glib::timeout_future(std::time::Duration::from_millis(500)).await;
        state = match proxy(conn, active.as_str(), "org.freedesktop.NetworkManager.Connection.Active").await {
            Ok(a) => a.get_property::<u32>("State").await.unwrap_or(4),
            Err(_) => 4,
        };
        if state >= 2 {
            break;
        }
    }
    if state == 2 {
        if created.is_some() {
            if let Some(old) = &net.saved {
                let _ = forget(conn, old).await;
            }
        }
        return Ok(());
    }
    if let Some(path) = created {
        let _ = forget(conn, &path).await;
    }
    Err(if net.security.locked() { format!("Couldn't join {}. Check the password.", net.ssid) } else { format!("Couldn't join {}", net.ssid) })
}

/// Every signal NetworkManager sends; re-read on these (debounced).
pub async fn changes(conn: &zbus::Connection) -> Option<zbus::MessageStream> {
    let rule = zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).sender(NM).ok()?.build();
    zbus::MessageStream::for_match_rule(rule, conn, Some(64)).await.ok()
}
