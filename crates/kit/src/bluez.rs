use std::collections::HashMap;
use std::sync::OnceLock;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue};

const AGENT: &str = "/dev/digit/bluez/agent";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Adapter {
    pub path: String,
    pub id: String,
    pub name: String,
    pub address: String,
    pub powered: bool,
    pub discoverable: bool,
    pub discovering: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Device {
    pub path: String,
    pub adapter: String,
    pub name: String,
    pub named: bool,
    pub address: String,
    pub icon: String,
    pub paired: bool,
    pub trusted: bool,
    pub connected: bool,
    pub rssi: Option<i16>,
    pub battery: Option<u8>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub adapters: Vec<Adapter>,
    pub devices: Vec<Device>,
}

impl Snapshot {
    /// The adapter the user picked (saved as `bluetooth_adapter`), else the first powered one, else the first.
    pub fn chosen(&self) -> Option<&Adapter> {
        let want = chosen_id();
        self.adapters.iter().find(|a| Some(&a.id) == want.as_ref()).or_else(|| self.adapters.iter().find(|a| a.powered)).or(self.adapters.first())
    }

    pub fn on(&self, adapter: &str) -> impl Iterator<Item = &Device> {
        let adapter = adapter.to_string();
        self.devices.iter().filter(move |d| d.adapter == adapter)
    }
}

pub fn chosen_id() -> Option<String> {
    let t = crate::prefs::read();
    let id = crate::prefs::text(&t, "bluetooth_adapter", "");
    (!id.is_empty()).then_some(id)
}

pub fn choose(id: &str) -> Result<(), String> {
    crate::prefs::set("bluetooth_adapter", toml::Value::String(id.to_string()))
}

pub fn symbol(icon: &str) -> &'static str {
    match icon {
        i if i.contains("headset") || i.contains("headphone") => "headphones",
        i if i.contains("audio") || i.contains("speaker") => "speaker",
        i if i.contains("mouse") => "mouse",
        i if i.contains("keyboard") => "keyboard",
        i if i.contains("gaming") || i.contains("joystick") => "sports_esports",
        i if i.contains("phone") => "smartphone",
        i if i.contains("computer") => "computer",
        i if i.contains("watch") => "watch",
        i if i.contains("camera") => "photo_camera",
        i if i.contains("printer") => "print",
        _ => "bluetooth",
    }
}

fn flag(props: &HashMap<String, OwnedValue>, key: &str) -> bool {
    props.get(key).and_then(|v| bool::try_from(v.try_clone().ok()?).ok()).unwrap_or(false)
}

fn text(props: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    props.get(key).and_then(|v| String::try_from(v.try_clone().ok()?).ok())
}

pub async fn read(conn: &zbus::Connection) -> Snapshot {
    let mut snap = Snapshot::default();
    let Ok(om) = zbus::fdo::ObjectManagerProxy::builder(conn).destination("org.bluez").and_then(|b| b.path("/")) else { return snap };
    let Ok(om) = om.build().await else { return snap };
    let Ok(objects) = om.get_managed_objects().await else { return snap };
    for (path, ifaces) in &objects {
        if let Some(props) = ifaces.get("org.bluez.Adapter1") {
            let id = path.as_str().rsplit('/').next().unwrap_or_default().to_string();
            snap.adapters.push(Adapter {
                path: path.to_string(),
                name: text(props, "Alias").or_else(|| text(props, "Name")).unwrap_or_else(|| id.clone()),
                id,
                address: text(props, "Address").unwrap_or_default(),
                powered: flag(props, "Powered"),
                discoverable: flag(props, "Discoverable"),
                discovering: flag(props, "Discovering"),
            });
        }
        if let Some(props) = ifaces.get("org.bluez.Device1") {
            let address = text(props, "Address").unwrap_or_default();
            let name = text(props, "Name");
            snap.devices.push(Device {
                path: path.to_string(),
                adapter: props.get("Adapter").and_then(|v| OwnedObjectPath::try_from(v.try_clone().ok()?).ok()).map(|p| p.to_string()).unwrap_or_default(),
                named: name.is_some(),
                name: text(props, "Alias").or(name).unwrap_or_else(|| address.clone()),
                address,
                icon: text(props, "Icon").unwrap_or_default(),
                paired: flag(props, "Paired"),
                trusted: flag(props, "Trusted"),
                connected: flag(props, "Connected"),
                rssi: props.get("RSSI").and_then(|v| i16::try_from(v.try_clone().ok()?).ok()),
                battery: ifaces.get("org.bluez.Battery1").and_then(|b| b.get("Percentage")).and_then(|v| u8::try_from(v.try_clone().ok()?).ok()),
            });
        }
    }
    snap.adapters.sort_by(|a, b| a.id.cmp(&b.id));
    snap.devices.sort_by(|a, b| {
        (b.connected, b.paired, b.named)
            .cmp(&(a.connected, a.paired, a.named))
            .then(b.rssi.unwrap_or(i16::MIN).cmp(&a.rssi.unwrap_or(i16::MIN)).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())))
    });
    snap
}

async fn proxy(conn: &zbus::Connection, path: &str, iface: &'static str) -> Result<zbus::Proxy<'static>, String> {
    zbus::proxy::Builder::new(conn)
        .destination("org.bluez")
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
        zbus::Error::MethodError(name, None, _) => name.trim_start_matches("org.bluez.Error.").to_string(),
        _ => e.to_string(),
    }
}

pub async fn set_adapter(conn: &zbus::Connection, adapter: &str, key: &'static str, on: bool) -> Result<(), String> {
    proxy(conn, adapter, "org.bluez.Adapter1").await?.set_property(key, on).await.map_err(|e| e.to_string())
}

pub async fn set_device(conn: &zbus::Connection, device: &str, key: &'static str, on: bool) -> Result<(), String> {
    proxy(conn, device, "org.bluez.Device1").await?.set_property(key, on).await.map_err(|e| e.to_string())
}

pub async fn scan(conn: &zbus::Connection, adapter: &str, on: bool) -> Result<(), String> {
    let p = proxy(conn, adapter, "org.bluez.Adapter1").await?;
    match p.call_method(if on { "StartDiscovery" } else { "StopDiscovery" }, &()).await {
        Ok(_) => Ok(()),
        Err(zbus::Error::MethodError(name, _, _)) if !on || name.as_str().ends_with("InProgress") => Ok(()),
        Err(e) => Err(tidy(e)),
    }
}

pub async fn connect(conn: &zbus::Connection, device: &str, on: bool) -> Result<(), String> {
    let p = proxy(conn, device, "org.bluez.Device1").await?;
    p.call_method(if on { "Connect" } else { "Disconnect" }, &()).await.map(|_| ()).map_err(tidy)
}

/// Pairs through our own agent, trusts the device and connects to it, as blueman does.
pub async fn pair(conn: &zbus::Connection, device: &str) -> Result<(), String> {
    let p = proxy(conn, device, "org.bluez.Device1").await?;
    match p.call_method("Pair", &()).await {
        Ok(_) => {}
        Err(zbus::Error::MethodError(name, _, _)) if name.as_str().ends_with("AlreadyExists") => {}
        Err(e) => return Err(tidy(e)),
    }
    let _ = p.set_property("Trusted", true).await;
    let _ = p.call_method("Connect", &()).await;
    Ok(())
}

pub async fn cancel_pair(conn: &zbus::Connection, device: &str) {
    if let Ok(p) = proxy(conn, device, "org.bluez.Device1").await {
        let _ = p.call_method("CancelPairing", &()).await;
    }
}

pub async fn remove(conn: &zbus::Connection, adapter: &str, device: &str) -> Result<(), String> {
    let p = proxy(conn, adapter, "org.bluez.Adapter1").await?;
    let dev = ObjectPath::try_from(device).map_err(|e| e.to_string())?;
    p.call_method("RemoveDevice", &(dev,)).await.map(|_| ()).map_err(tidy)
}

/// Every signal org.bluez sends: devices appearing, properties changing, adapters coming and going.
pub async fn changes(conn: &zbus::Connection) -> Option<zbus::MessageStream> {
    let rule = zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).sender("org.bluez").ok()?.build();
    zbus::MessageStream::for_match_rule(rule, conn, Some(64)).await.ok()
}

#[derive(Debug)]
pub enum Ask {
    /// Show the code and ask whether it matches the one on the device.
    Confirm(String, String),
    /// The device wants a PIN typed in (legacy pairing).
    Pin(String),
    /// The device wants a numeric passkey typed in.
    Passkey(String),
    /// Type this code on the device.
    Show(String, String),
    Cancel,
}

/// A question from BlueZ while pairing. Answer with `Some(text)` (or `Some("")` for yes) or `None` to refuse.
pub struct Request {
    pub ask: Ask,
    pub reply: async_channel::Sender<Option<String>>,
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.bluez.Error")]
enum Refusal {
    #[zbus(error)]
    ZBus(zbus::Error),
    Rejected,
    Canceled,
}

struct Agent {
    conn: zbus::Connection,
    tx: async_channel::Sender<Request>,
}

impl Agent {
    async fn name(&self, device: &ObjectPath<'_>) -> String {
        match proxy(&self.conn, device.as_str(), "org.bluez.Device1").await {
            Ok(p) => p.get_property::<String>("Alias").await.unwrap_or_else(|_| "Device".into()),
            Err(_) => "Device".into(),
        }
    }

    async fn ask(&self, ask: Ask) -> Option<String> {
        let (reply, answer) = async_channel::bounded(1);
        self.tx.send(Request { ask, reply }).await.ok()?;
        answer.recv().await.ok().flatten()
    }
}

#[zbus::interface(name = "org.bluez.Agent1")]
impl Agent {
    fn release(&self) {}

    async fn request_pin_code(&self, device: ObjectPath<'_>) -> Result<String, Refusal> {
        let name = self.name(&device).await;
        self.ask(Ask::Pin(name)).await.filter(|p| !p.is_empty()).ok_or(Refusal::Rejected)
    }

    async fn display_pin_code(&self, device: ObjectPath<'_>, code: String) {
        let name = self.name(&device).await;
        let tx = self.tx.clone();
        let (reply, _) = async_channel::bounded(1);
        let _ = tx.send(Request { ask: Ask::Show(name, code), reply }).await;
    }

    async fn request_passkey(&self, device: ObjectPath<'_>) -> Result<u32, Refusal> {
        let name = self.name(&device).await;
        self.ask(Ask::Passkey(name)).await.and_then(|p| p.trim().parse().ok()).ok_or(Refusal::Rejected)
    }

    async fn display_passkey(&self, device: ObjectPath<'_>, passkey: u32, _entered: u16) {
        let name = self.name(&device).await;
        let (reply, _) = async_channel::bounded(1);
        let _ = self.tx.send(Request { ask: Ask::Show(name, format!("{passkey:06}")), reply }).await;
    }

    async fn request_confirmation(&self, device: ObjectPath<'_>, passkey: u32) -> Result<(), Refusal> {
        let name = self.name(&device).await;
        self.ask(Ask::Confirm(name, format!("{passkey:06}"))).await.map(|_| ()).ok_or(Refusal::Rejected)
    }

    async fn request_authorization(&self, device: ObjectPath<'_>) -> Result<(), Refusal> {
        let name = self.name(&device).await;
        self.ask(Ask::Confirm(name, String::new())).await.map(|_| ()).ok_or(Refusal::Rejected)
    }

    async fn authorize_service(&self, device: ObjectPath<'_>, _uuid: String) -> Result<(), Refusal> {
        let trusted = match proxy(&self.conn, device.as_str(), "org.bluez.Device1").await {
            Ok(p) => p.get_property::<bool>("Trusted").await.unwrap_or(false) || p.get_property::<bool>("Paired").await.unwrap_or(false),
            Err(_) => false,
        };
        if trusted { Ok(()) } else { Err(Refusal::Canceled) }
    }

    async fn cancel(&self) {
        let (reply, _) = async_channel::bounded(1);
        let _ = self.tx.send(Request { ask: Ask::Cancel, reply }).await;
    }
}

static REQUESTS: OnceLock<async_channel::Receiver<Request>> = OnceLock::new();

/// Registers a pairing agent on `conn` (once per process). BlueZ sends this process's
/// Pair() questions to it; read them from the returned channel and answer each one.
pub async fn agent(conn: &zbus::Connection) -> Option<async_channel::Receiver<Request>> {
    if let Some(rx) = REQUESTS.get() {
        return Some(rx.clone());
    }
    let (tx, rx) = async_channel::unbounded();
    conn.object_server().at(AGENT, Agent { conn: conn.clone(), tx }).await.ok()?;
    let manager = proxy(conn, "/org/bluez", "org.bluez.AgentManager1").await.ok()?;
    let path = ObjectPath::try_from(AGENT).ok()?;
    if let Err(e) = manager.call_method("RegisterAgent", &(&path, "KeyboardDisplay")).await {
        if !matches!(&e, zbus::Error::MethodError(n, _, _) if n.as_str().ends_with("AlreadyExists")) {
            return None;
        }
    }
    let _ = REQUESTS.set(rx.clone());
    Some(rx)
}
