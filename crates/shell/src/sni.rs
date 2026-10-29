use crate::app::App;
use crate::popmenu::{self, Row};
use futures_util::StreamExt;
use gtk::{gdk, glib, prelude::*};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::Duration;
use zbus::message::Header;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};

const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WPATH: &str = "/StatusNotifierWatcher";
const ITEM: &str = "org.kde.StatusNotifierItem";
const MENU: &str = "com.canonical.dbusmenu";
const MAX: usize = 12;

#[derive(Clone)]
pub struct Item {
    pub key: String,
    bus: String,
    owner: String,
    path: String,
    pub id: String,
    pub title: String,
    pub icon: Option<gdk::Paintable>,
    menu: Option<String>,
    is_menu: bool,
    pub status: String,
}

struct Watcher {
    items: Vec<String>,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    async fn register_status_notifier_item(&mut self, service: &str, #[zbus(header)] hdr: Header<'_>, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
        let sender = hdr.sender().map(|s| s.to_string()).unwrap_or_default();
        let full = if service.starts_with('/') { format!("{sender}{service}") } else if service.contains('/') { service.to_string() } else { format!("{service}/StatusNotifierItem") };
        if full.len() > 255 || self.items.len() >= 64 || self.items.contains(&full) {
            return;
        }
        self.items.push(full.clone());
        let _ = Self::status_notifier_item_registered(&emitter, &full).await;
    }

    async fn register_status_notifier_host(&mut self, _service: &str, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
        let _ = Self::status_notifier_host_registered(&emitter).await;
    }

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.items.clone()
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(signal)]
    async fn status_notifier_item_registered(emitter: &SignalEmitter<'_>, service: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_item_unregistered(emitter: &SignalEmitter<'_>, service: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_host_registered(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

thread_local! {
    static CONN: RefCell<Option<zbus::Connection>> = const { RefCell::new(None) };
}

async fn within<T>(ms: u64, f: impl std::future::Future<Output = T>) -> Option<T> {
    let f = std::pin::pin!(f);
    let t = std::pin::pin!(glib::timeout_future(Duration::from_millis(ms)));
    match futures_util::future::select(f, t).await {
        futures_util::future::Either::Left((v, _)) => Some(v),
        _ => None,
    }
}

fn split(key: &str) -> (String, String) {
    match key.find('/') {
        Some(i) => (key[..i].to_string(), key[i..].to_string()),
        None => (key.to_string(), "/StatusNotifierItem".to_string()),
    }
}

fn string(map: &HashMap<String, OwnedValue>, k: &str) -> String {
    map.get(k).and_then(|v| v.downcast_ref::<String>().ok()).unwrap_or_default()
}

fn pixmap(map: &HashMap<String, OwnedValue>, k: &str, want: i32) -> Option<gdk::Paintable> {
    let v = map.get(k)?.try_clone().ok()?;
    let list: Vec<(i32, i32, Vec<u8>)> = Vec::try_from(v).ok()?;
    let pick = list.iter().filter(|(w, h, d)| *w > 0 && *h > 0 && d.len() >= (*w * *h * 4) as usize).min_by_key(|(w, _, _)| if *w >= want { *w - want } else { 10_000 + want - *w })?;
    let (w, h, data) = pick;
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for px in data.chunks_exact(4).take((w * h) as usize) {
        rgba.extend_from_slice(&[px[1], px[2], px[3], px[0]]);
    }
    let bytes = glib::Bytes::from_owned(rgba);
    Some(gdk::MemoryTexture::new(*w, *h, gdk::MemoryFormat::R8g8b8a8, &bytes, (*w * 4) as usize).upcast())
}

fn named(name: &str, theme_path: &str, px: i32, scale: i32) -> Option<gdk::Paintable> {
    if name.is_empty() {
        return None;
    }
    if name.starts_with('/') && std::path::Path::new(name).is_file() {
        return gdk::Texture::from_filename(name).ok().map(|t| t.upcast());
    }
    if !theme_path.is_empty() {
        for ext in ["png", "svg"] {
            let p = std::path::Path::new(theme_path).join(format!("{name}.{ext}"));
            if p.is_file() {
                return gdk::Texture::from_filename(p).ok().map(|t| t.upcast());
            }
        }
    }
    let display = gdk::Display::default()?;
    let theme = gtk::IconTheme::for_display(&display);
    theme.has_icon(name).then(|| theme.lookup_icon(name, &[], px, scale, gtk::TextDirection::Ltr, gtk::IconLookupFlags::empty()).upcast())
}

async fn load(conn: &zbus::Connection, key: &str) -> Option<Item> {
    let (bus, path) = split(key);
    let props = zbus::fdo::PropertiesProxy::builder(conn).destination(bus.clone()).ok()?.path(path.clone()).ok()?.cache_properties(zbus::proxy::CacheProperties::No).build().await.ok()?;
    let iface = zbus::names::InterfaceName::try_from(ITEM).ok()?;
    let map = within(900, props.get_all(iface)).await?.ok()?;
    let owner = if bus.starts_with(':') {
        bus.clone()
    } else {
        let dbus = zbus::fdo::DBusProxy::new(conn).await.ok()?;
        let name = zbus::names::BusName::try_from(bus.clone()).ok()?;
        within(500, dbus.get_name_owner(name)).await.and_then(|r| r.ok()).map(|o| o.to_string()).unwrap_or_default()
    };
    let status = string(&map, "Status");
    let attention = status == "NeedsAttention";
    let scale = gdk::Display::default().and_then(|d| d.monitors().item(0)).and_then(|m| m.downcast::<gdk::Monitor>().ok()).map(|m| m.scale_factor()).unwrap_or(1);
    let want = 20 * scale;
    let theme_path = string(&map, "IconThemePath");
    let icon = if attention { named(&string(&map, "AttentionIconName"), &theme_path, 20, scale).or_else(|| pixmap(&map, "AttentionIconPixmap", want)) } else { None }
        .or_else(|| named(&string(&map, "IconName"), &theme_path, 20, scale))
        .or_else(|| pixmap(&map, "IconPixmap", want));
    let menu = map.get("Menu").and_then(|v| v.downcast_ref::<zbus::zvariant::ObjectPath>().ok()).map(|p| p.to_string()).filter(|p| p != "/");
    let is_menu = map.get("ItemIsMenu").and_then(|v| v.downcast_ref::<bool>().ok()).unwrap_or(false);
    let title = map.get("ToolTip").and_then(|v| v.try_clone().ok()).and_then(|v| <(String, Vec<(i32, i32, Vec<u8>)>, String, String)>::try_from(v).ok()).map(|t| t.2).filter(|t| !t.is_empty()).unwrap_or_else(|| string(&map, "Title"));
    Some(Item { key: key.to_string(), bus, owner, path, id: string(&map, "Id"), title: title.chars().take(64).collect(), icon, menu, is_menu, status })
}

fn publish(app: &App, list: &[Item]) {
    let shown: Vec<Item> = list.iter().filter(|i| i.status != "Passive").take(MAX).cloned().collect();
    *app.sni.borrow_mut() = shown;
    for n in app.notches.borrow().iter() {
        n.restrip();
    }
}

async fn serve(conn: &zbus::Connection) -> bool {
    if conn.object_server().at(WPATH, Watcher { items: Vec::new() }).await.is_err() {
        return false;
    }
    let ok = matches!(conn.request_name_with_flags(WATCHER, zbus::fdo::RequestNameFlags::DoNotQueue.into()).await, Ok(zbus::fdo::RequestNameReply::PrimaryOwner));
    if !ok {
        let _ = conn.object_server().remove::<Watcher, _>(WPATH).await;
    }
    ok
}

async fn register(conn: &zbus::Connection, host: &str) -> Vec<String> {
    let Ok(watcher) = zbus::Proxy::new_owned(conn.clone(), WATCHER, WPATH, WATCHER).await else { return Vec::new() };
    let _ = within(1500, watcher.call_method("RegisterStatusNotifierHost", &(host,))).await;
    within(1500, watcher.get_property::<Vec<String>>("RegisteredStatusNotifierItems")).await.and_then(|r| r.ok()).unwrap_or_default()
}

async fn forget(conn: &zbus::Connection, key: &str) {
    let Ok(iface) = conn.object_server().interface::<_, Watcher>(WPATH).await else { return };
    let removed = {
        let mut w = iface.get_mut().await;
        let before = w.items.len();
        w.items.retain(|k| k != key);
        before != w.items.len()
    };
    if removed {
        let _ = Watcher::status_notifier_item_unregistered(iface.signal_emitter(), key).await;
    }
}

pub fn start(app: &Rc<App>) {
    if !app.cfg.tray {
        return;
    }
    let weak: Weak<App> = Rc::downgrade(app);
    glib::spawn_future_local(async move {
        let Ok(conn) = crate::bus::session().await else { return };
        CONN.with(|c| *c.borrow_mut() = Some(conn.clone()));
        let host = format!("org.kde.StatusNotifierHost-{}", std::process::id());
        let _ = conn.request_name(host.as_str()).await;
        let Ok(dbus) = zbus::fdo::DBusProxy::new(&conn).await else { return };
        let mut own = false;
        if !dbus.name_has_owner(zbus::names::BusName::try_from(WATCHER).unwrap()).await.unwrap_or(false) {
            own = serve(&conn).await;
        }
        let keys = register(&conn, &host).await;
        let mut items: Vec<Item> = Vec::new();
        for k in keys.iter().take(32) {
            if let Some(i) = load(&conn, k).await {
                items.push(i);
            }
        }
        let Some(app) = weak.upgrade() else { return };
        publish(&app, &items);
        drop(app);
        let rule = |iface: &'static str| zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).interface(iface).map(|b| b.build());
        let streams: Vec<_> = {
            let mut v = Vec::new();
            for iface in [WATCHER, ITEM, "org.freedesktop.DBus"] {
                if let Ok(r) = rule(iface) {
                    if let Ok(s) = zbus::MessageStream::for_match_rule(r, &conn, Some(64)).await {
                        v.push(s);
                    }
                }
            }
            v
        };
        let mut all = futures_util::stream::select_all(streams);
        while let Some(Ok(msg)) = all.next().await {
            let header = msg.header();
            let member = header.member().map(|m| m.to_string()).unwrap_or_default();
            let iface = header.interface().map(|i| i.to_string()).unwrap_or_default();
            let sender = header.sender().map(|s| s.to_string()).unwrap_or_default();
            let mut changed = false;
            match (iface.as_str(), member.as_str()) {
                (WATCHER, "StatusNotifierItemRegistered") => {
                    if let Ok(key) = msg.body().deserialize::<String>() {
                        if !items.iter().any(|i| i.key == key) {
                            if let Some(i) = load(&conn, &key).await {
                                items.push(i);
                                changed = true;
                            }
                        }
                    }
                }
                (WATCHER, "StatusNotifierItemUnregistered") => {
                    if let Ok(key) = msg.body().deserialize::<String>() {
                        let before = items.len();
                        items.retain(|i| i.key != key);
                        changed = before != items.len();
                    }
                }
                (ITEM, m) if m.starts_with("New") => {
                    glib::timeout_future(Duration::from_millis(120)).await;
                    let keys: Vec<String> = items.iter().filter(|i| i.owner == sender || i.bus == sender).map(|i| i.key.clone()).collect();
                    for k in keys {
                        if let Some(fresh) = load(&conn, &k).await {
                            if let Some(slot) = items.iter_mut().find(|i| i.key == k) {
                                *slot = fresh;
                                changed = true;
                            }
                        }
                    }
                }
                ("org.freedesktop.DBus", "NameOwnerChanged") => {
                    let Ok((name, _old, new)) = msg.body().deserialize::<(String, String, String)>() else { continue };
                    if name == WATCHER {
                        if new.is_empty() && !own {
                            own = serve(&conn).await;
                            items.clear();
                            changed = true;
                        } else if !new.is_empty() && !own {
                            let keys = register(&conn, &host).await;
                            items.clear();
                            for k in keys.iter().take(32) {
                                if let Some(i) = load(&conn, k).await {
                                    items.push(i);
                                }
                            }
                            changed = true;
                        }
                    } else if new.is_empty() {
                        let gone: Vec<String> = items.iter().filter(|i| i.owner == name || i.bus == name).map(|i| i.key.clone()).collect();
                        if !gone.is_empty() {
                            items.retain(|i| !gone.contains(&i.key));
                            changed = true;
                        }
                        if own {
                            let stale: Vec<String> = match conn.object_server().interface::<_, Watcher>(WPATH).await {
                                Ok(iface) => iface.get().await.items.iter().filter(|k| split(k).0 == name).cloned().collect(),
                                Err(_) => Vec::new(),
                            };
                            for k in stale {
                                forget(&conn, &k).await;
                            }
                        }
                    }
                }
                _ => {}
            }
            if changed {
                let Some(app) = weak.upgrade() else { break };
                publish(&app, &items);
            }
        }
    });
}

fn label(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '_' {
            if chars.peek() == Some(&'_') {
                out.push('_');
                chars.next();
            }
            continue;
        }
        out.push(c);
    }
    out
}

#[derive(zbus::zvariant::Type, serde::Deserialize, zbus::zvariant::Value, zbus::zvariant::OwnedValue)]
struct Node {
    id: i32,
    props: HashMap<String, OwnedValue>,
    kids: Vec<OwnedValue>,
}

fn rows(conn: &zbus::Connection, bus: &str, path: &str, node: &Node, depth: usize, out: &mut Vec<Row>) {
    for kid in &node.kids {
        let Ok(v) = kid.try_clone() else { continue };
        let Ok(child) = Node::try_from(v) else { continue };
        let get_s = |k: &str| child.props.get(k).and_then(|v| v.downcast_ref::<String>().ok());
        let get_b = |k: &str| child.props.get(k).and_then(|v| v.downcast_ref::<bool>().ok());
        if get_b("visible") == Some(false) {
            continue;
        }
        if get_s("type").as_deref() == Some("separator") {
            if !matches!(out.last(), None | Some(Row::Sep)) {
                out.push(Row::Sep);
            }
            continue;
        }
        let text = label(&get_s("label").unwrap_or_default());
        if text.is_empty() {
            continue;
        }
        let enabled = get_b("enabled").unwrap_or(true);
        let toggle = get_s("toggle-type").unwrap_or_default();
        let on = child.props.get("toggle-state").and_then(|v| v.downcast_ref::<i32>().ok()).unwrap_or(0) == 1;
        let indent = "    ".repeat(depth);
        let mark = match (toggle.as_str(), on) {
            ("checkmark" | "radio", true) => "✓  ",
            ("checkmark" | "radio", false) => "    ",
            _ => "",
        };
        let submenu = get_s("children-display").as_deref() == Some("submenu") || !child.kids.is_empty();
        if submenu {
            out.push(Row::item(format!("{indent}{text}"), || {}).off(true));
            rows(conn, bus, path, &child, depth + 1, out);
            continue;
        }
        let (c, b, p, id) = (conn.clone(), bus.to_string(), path.to_string(), child.id);
        out.push(
            Row::item(format!("{indent}{mark}{text}"), move || {
                let (c, b, p) = (c.clone(), b.clone(), p.clone());
                glib::spawn_future_local(async move {
                    let Ok(menu) = zbus::Proxy::new_owned(c, b, p, MENU).await else { return };
                    let stamp = (glib::monotonic_time() / 1000) as u32;
                    let _ = within(1500, menu.call_method("Event", &(id, "clicked", Value::from(0i32), stamp))).await;
                });
            })
            .off(!enabled),
        );
    }
}

pub fn menu(app: &Rc<App>, index: usize, monitor: Option<gdk::Monitor>, x: f64, y: f64) {
    let Some(item) = app.sni.borrow().get(index).cloned() else { return };
    let Some(conn) = CONN.with(|c| c.borrow().clone()) else { return };
    let Some(path) = item.menu.clone() else { return };
    let app = app.clone();
    glib::spawn_future_local(async move {
        let Ok(proxy) = zbus::Proxy::new_owned(conn.clone(), item.bus.clone(), path.clone(), MENU).await else { return };
        let _ = within(400, proxy.call_method("AboutToShow", &(0i32,))).await;
        let names: Vec<&str> = Vec::new();
        let Some(Ok(reply)) = within(1500, proxy.call::<_, _, (u32, Node)>("GetLayout", &(0i32, -1i32, names))).await else {
            popmenu::settle();
            return;
        };
        let mut list = Vec::new();
        rows(&conn, &item.bus, &path, &reply.1, 0, &mut list);
        while matches!(list.last(), Some(Row::Sep)) {
            list.pop();
        }
        let title = if item.title.is_empty() { item.id.clone() } else { item.title.clone() };
        popmenu::show(&app, monitor, x, y, &title, list);
    });
}

pub fn activate(app: &Rc<App>, index: usize, monitor: Option<gdk::Monitor>, x: f64, y: f64, secondary: bool) {
    let Some(item) = app.sni.borrow().get(index).cloned() else { return };
    if item.is_menu && !secondary {
        menu(app, index, monitor, x, y);
        return;
    }
    let Some(conn) = CONN.with(|c| c.borrow().clone()) else { return };
    let app = app.clone();
    glib::spawn_future_local(async move {
        let Ok(proxy) = zbus::Proxy::new_owned(conn, item.bus.clone(), item.path.clone(), ITEM).await else { return };
        let method = if secondary { "SecondaryActivate" } else { "Activate" };
        let ok = matches!(within(1200, proxy.call_method(method, &(x as i32, y as i32))).await, Some(Ok(_)));
        if !ok && !secondary && item.menu.is_some() {
            menu(&app, index, monitor, x, y);
        } else {
            popmenu::settle();
        }
    });
}
