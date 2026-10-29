use crate::app::{App, Peek};
use futures_util::StreamExt;
use gtk::{gdk, gio, glib, prelude::*};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};

const NAME: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";

/// Icon size requested from the theme; drawn at 18 logical px, so this stays sharp at 2x.
const ICON_PX: i32 = 36;

/// Raw icon data as it arrives on the bus. Resolved to a paintable on the GTK thread.
enum IconSource {
    Named(String),
    File(String),
    Pixels { width: i32, height: i32, stride: i32, alpha: bool, data: Vec<u8> },
}

#[derive(Default)]
struct Look {
    app_icon: Option<IconSource>,
    desktop_entry: Option<String>,
    image: Option<IconSource>,
}

enum Event {
    Notify { id: u32, app: String, summary: String, body: String, default: bool, urgent: bool, look: Look, transient: bool, actions: Vec<(String, String)> },
    Close(u32),
}

struct Server {
    tx: async_channel::Sender<Event>,
    next: AtomicU32,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Server {
    fn get_capabilities(&self) -> Vec<String> {
        vec!["body".into(), "actions".into()]
    }

    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        _expire_timeout: i32,
    ) -> u32 {
        let id = if replaces_id > 0 { replaces_id } else { self.next.fetch_add(1, Ordering::Relaxed) };
        let urgency = hints.get("urgency").and_then(|v| u8::try_from(v.try_clone().ok()?).ok()).unwrap_or(1);
        let default = actions.chunks(2).any(|pair| pair[0] == "default");
        let buttons: Vec<(String, String)> = actions
            .chunks(2)
            .filter(|p| p.len() == 2 && p[0] != "default" && !p[1].trim().is_empty())
            .take(3)
            .map(|p| (p[0].chars().take(64).collect(), plain(&p[1]).chars().take(24).collect()))
            .collect();
        let look = Look {
            app_icon: icon_from_str(&app_icon),
            desktop_entry: hint_str(&hints, &["desktop-entry"]),
            image: ["image-data", "image_data", "icon_data"]
                .iter()
                .find_map(|k| hints.get(*k).and_then(|v| pixels(v)))
                .or_else(|| hint_str(&hints, &["image-path", "image_path"]).and_then(|p| icon_from_str(&p))),
        };
        let transient = hints.get("transient").and_then(|v| bool::try_from(v.try_clone().ok()?).ok().or_else(|| u8::try_from(v.try_clone().ok()?).ok().map(|x| x != 0))).unwrap_or(false);
        let _ = self.tx.try_send(Event::Notify {
            transient,
            id,
            app: app_name,
            summary: plain(&summary),
            body: plain(&body),
            default,
            urgent: urgency >= 2,
            look,
            actions: buttons,
        });
        id
    }

    fn close_notification(&self, id: u32) {
        let _ = self.tx.try_send(Event::Close(id));
    }

    fn get_server_information(&self) -> (String, String, String, String) {
        ("digit".into(), "digit".into(), env!("CARGO_PKG_VERSION").into(), "1.2".into())
    }

    #[zbus(signal)]
    async fn notification_closed(emitter: &SignalEmitter<'_>, id: u32, reason: u32) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn action_invoked(emitter: &SignalEmitter<'_>, id: u32, action_key: &str) -> zbus::Result<()>;
}

fn icon_from_str(value: &str) -> Option<IconSource> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else if value.starts_with("file://") || value.starts_with('/') {
        Some(IconSource::File(value.to_string()))
    } else {
        Some(IconSource::Named(value.to_string()))
    }
}

fn hint_str(hints: &HashMap<String, OwnedValue>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| match hints.get(*k).map(|v| &**v) {
        Some(Value::Str(s)) if !s.is_empty() => Some(s.to_string()),
        _ => None,
    })
}

/// Decodes the `(iiibiiay)` image hint: width, height, rowstride, alpha, bits, channels, data.
fn pixels(value: &OwnedValue) -> Option<IconSource> {
    let Value::Structure(st) = &**value else { return None };
    let f = st.fields();
    let int = |i: usize| match f.get(i) {
        Some(Value::I32(v)) => Some(*v),
        _ => None,
    };
    let (width, height, stride) = (int(0)?, int(1)?, int(2)?);
    let alpha = matches!(f.get(3), Some(Value::Bool(true)));
    let (bits, channels) = (int(4)?, int(5)?);
    if bits != 8 || channels != if alpha { 4 } else { 3 } || width <= 0 || height <= 0 {
        return None;
    }
    let Some(Value::Array(array)) = f.get(6) else { return None };
    let data: Vec<u8> = array.iter().filter_map(|v| if let Value::U8(b) = v { Some(*b) } else { None }).collect();
    let needed = stride as usize * (height as usize - 1) + width as usize * channels as usize;
    (data.len() >= needed).then_some(IconSource::Pixels { width, height, stride, alpha, data })
}

fn paint(theme: &gtk::IconTheme, source: &IconSource, require_known_name: bool) -> Option<gdk::Paintable> {
    paint_at(theme, source, require_known_name, ICON_PX)
}

fn paint_at(theme: &gtk::IconTheme, source: &IconSource, require_known_name: bool, px: i32) -> Option<gdk::Paintable> {
    let lookup = |icon: &gio::Icon| {
        theme.lookup_by_gicon(icon, px, 1, gtk::TextDirection::Ltr, gtk::IconLookupFlags::empty()).upcast::<gdk::Paintable>()
    };
    match source {
        IconSource::Named(name) => {
            if require_known_name && !theme.has_icon(name) {
                return None;
            }
            Some(lookup(gio::ThemedIcon::new(name).upcast_ref()))
        }
        IconSource::File(location) => {
            let file = if location.starts_with("file://") { gio::File::for_uri(location) } else { gio::File::for_path(location) };
            file.query_exists(gio::Cancellable::NONE).then(|| lookup(gio::FileIcon::new(&file).upcast_ref()))
        }
        IconSource::Pixels { width, height, stride, alpha, data } => {
            let format = if *alpha { gdk::MemoryFormat::R8g8b8a8 } else { gdk::MemoryFormat::R8g8b8 };
            let bytes = glib::Bytes::from(data.as_slice());
            Some(gdk::MemoryTexture::new(*width, *height, format, &bytes, *stride as usize).upcast())
        }
    }
}

/// Square-ish images can stand in for a logo; wide ones (screenshots, photos) are content and
/// would be an unreadable smear at header size.
fn icon_like(source: &IconSource) -> bool {
    let (w, h) = match source {
        IconSource::Named(_) => return true,
        IconSource::Pixels { width, height, .. } => (*width, *height),
        IconSource::File(location) => {
            let path = location.strip_prefix("file://").map(|p| gio::File::for_uri(&format!("file://{p}"))).unwrap_or_else(|| gio::File::for_path(location));
            let Some(path) = path.path() else { return false };
            match gtk::gdk_pixbuf::Pixbuf::file_info(&path) {
                Some((_, w, h)) => (w, h),
                None => return false,
            }
        }
    };
    h > 0 && (0.8..=1.25).contains(&(f64::from(w) / f64::from(h)))
}

fn avatar(look: &Look) -> Option<gdk::Paintable> {
    let display = gdk::Display::default()?;
    let theme = gtk::IconTheme::for_display(&display);
    let content = |s: &&IconSource| !matches!(s, IconSource::Named(_)) && icon_like(s);
    look.image.as_ref().filter(content).or_else(|| look.app_icon.as_ref().filter(content)).and_then(|s| paint_at(&theme, s, true, 96))
}

fn logo(app: &App, source_app: &str, look: &Look) -> Option<gdk::Paintable> {
    let display = gdk::Display::default()?;
    let theme = gtk::IconTheme::for_display(&display);
    let named_app_icon = look.app_icon.as_ref().filter(|s| matches!(s, IconSource::Named(_)));
    if let Some(p) = named_app_icon.and_then(|s| paint(&theme, s, true)) {
        return Some(p);
    }
    let installed = |key: &str| {
        let list = app.apps.list();
        let index = app.apps.find(key.trim_end_matches(".desktop"))?;
        let icon = list.get(index)?.icon.clone()?;
        Some(theme.lookup_by_gicon(&icon, ICON_PX, 1, gtk::TextDirection::Ltr, gtk::IconLookupFlags::empty()).upcast())
    };
    if let Some(p) = look.desktop_entry.as_deref().and_then(installed) {
        return Some(p);
    }
    if let Some(p) = installed(source_app) {
        return Some(p);
    }
    let by_name = IconSource::Named(source_app.to_ascii_lowercase().replace(' ', "-"));
    (!source_app.is_empty()).then(|| paint(&theme, &by_name, true)).flatten()
}

/// App logo first (themed icon name, desktop entry, installed app or theme icon matching the
/// app name), then square content images (file icon, pixel data, image path). `None` lets the
/// notch draw a monogram badge.
fn resolve(app: &App, source_app: &str, look: &Look) -> Option<gdk::Paintable> {
    let display = gdk::Display::default()?;
    let theme = gtk::IconTheme::for_display(&display);
    let named_app_icon = look.app_icon.as_ref().filter(|s| matches!(s, IconSource::Named(_)));
    if let Some(p) = named_app_icon.and_then(|s| paint(&theme, s, true)) {
        return Some(p);
    }
    let installed = |key: &str| {
        let list = app.apps.list();
        let index = app.apps.find(key.trim_end_matches(".desktop"))?;
        let icon = list.get(index)?.icon.clone()?;
        Some(theme.lookup_by_gicon(&icon, ICON_PX, 1, gtk::TextDirection::Ltr, gtk::IconLookupFlags::empty()).upcast())
    };
    if let Some(p) = look.desktop_entry.as_deref().and_then(installed) {
        return Some(p);
    }
    if let Some(p) = installed(source_app) {
        return Some(p);
    }
    let by_name = IconSource::Named(source_app.to_ascii_lowercase().replace(' ', "-"));
    if let Some(p) = (!source_app.is_empty()).then(|| paint(&theme, &by_name, true)).flatten() {
        return Some(p);
    }
    let file_app_icon = look.app_icon.as_ref().filter(|s| !matches!(s, IconSource::Named(_)));
    file_app_icon
        .filter(|s| icon_like(s))
        .and_then(|s| paint(&theme, s, true))
        .or_else(|| look.image.as_ref().filter(|s| icon_like(s)).and_then(|s| paint(&theme, s, true)))
}

fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut tag = false;
    for c in text.chars() {
        match c {
            '<' => tag = true,
            '>' if tag => tag = false,
            _ if !tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub struct Handle {
    conn: zbus::Connection,
    defaults: RefCell<HashSet<u32>>,
}

impl Handle {
    fn emit(&self, id: u32, reason: u32, action: Option<String>) {
        let conn = self.conn.clone();
        glib::spawn_future_local(async move {
            let Ok(iface) = conn.object_server().interface::<_, Server>(PATH).await else { return };
            if let Some(key) = action {
                let _ = Server::action_invoked(iface.signal_emitter(), id, &key).await;
            }
            let _ = Server::notification_closed(iface.signal_emitter(), id, reason).await;
        });
    }

    pub fn release_name(&self) {
        let _ = zbus::block_on(self.conn.release_name(NAME));
    }

    pub fn closed(&self, id: u32, reason: u32) {
        self.defaults.borrow_mut().remove(&id);
        self.emit(id, reason, None);
    }

    pub fn clicked(&self, id: u32) {
        let default = self.defaults.borrow_mut().remove(&id);
        self.emit(id, 2, default.then(|| "default".to_string()));
    }

    pub fn invoke(&self, id: u32, key: &str) {
        self.defaults.borrow_mut().remove(&id);
        self.emit(id, 2, Some(key.to_string()));
    }

    pub fn has_default(&self, id: u32) -> bool {
        self.defaults.borrow().contains(&id)
    }
}

pub fn start(app: &Rc<App>) {
    let (tx, rx) = async_channel::unbounded();
    let weak = Rc::downgrade(app);
    glib::spawn_future_local(async move {
        let server = Server { tx, next: AtomicU32::new(1) };
        let conn = match async {
            let conn = crate::bus::session().await?;
            conn.object_server().at(PATH, server).await?;
            zbus::Result::Ok(conn)
        }
        .await
        {
            Ok(conn) => conn,
            Err(e) => {
                eprintln!("digit: notifications: {e}");
                return;
            }
        };
        let Some(app) = weak.upgrade() else { return };
        *app.notify.borrow_mut() = Some(Handle { conn: conn.clone(), defaults: RefCell::new(HashSet::new()) });
        drop(app);
        glib::spawn_future_local(acquire(conn));
        while let Ok(event) = rx.recv().await {
            let Some(app) = weak.upgrade() else { break };
            match event {
                Event::Notify { id, app: source, summary, body, default, urgent, look, transient, actions } => {
                    if let Some(h) = app.notify.borrow().as_ref() {
                        if default {
                            h.defaults.borrow_mut().insert(id);
                        }
                    }
                    let full = body.clone();
                    let summary = if summary.is_empty() { body } else { summary };
                    let line = if full == summary { String::new() } else { full.split_whitespace().collect::<Vec<_>>().join(" ") };
                    let icon = resolve(&app, &source, &look);
                    let face = avatar(&look);
                    let badge = if face.is_some() { logo(&app, &source, &look) } else { icon.clone() };
                    let name = app.apps.display_name(look.desktop_entry.as_deref().unwrap_or(&source));
                    let name = if name.is_empty() { app.apps.display_name(&source) } else { name };
                    if !transient {
                        let mut history = app.history.borrow_mut();
                        history.retain(|n| n.id != id);
                        history.insert(0, crate::app::Notice {
                            id,
                            app: name.clone(),
                            summary: summary.clone(),
                            body: if full == summary { String::new() } else { full },
                            icon: icon.clone(),
                            at: glib::real_time() / 1_000_000,
                        });
                        history.truncate(50);
                    }
                    app.history_changed();
                    if !app.dnd.get() || urgent {
                        app.request(Peek::Notification { id, summary, body: line, app: name, urgent, icon: badge, avatar: face, actions });
                    }
                }
                Event::Close(id) => {
                    app.history.borrow_mut().retain(|n| n.id != id);
                    app.history_changed();
                    app.cancel_notification(id);
                    if let Some(h) = app.notify.borrow().as_ref() {
                        h.closed(id, 3);
                    }
                }
            }
        }
    });
}

async fn acquire(conn: zbus::Connection) {
    let Ok(dbus) = zbus::fdo::DBusProxy::new(&conn).await else { return };
    let Ok(mut changes) = dbus.receive_name_owner_changed().await else { return };
    loop {
        match conn.request_name_with_flags(NAME, RequestNameFlags::DoNotQueue.into()).await {
            Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => return,
            _ => {}
        }
        let released = async {
            while let Some(signal) = changes.next().await {
                if let Ok(args) = signal.args() {
                    if args.name().as_str() == NAME && args.new_owner().is_none() {
                        return;
                    }
                }
            }
        };
        let timeout = glib::timeout_future(Duration::from_secs(5));
        futures_util::pin_mut!(released, timeout);
        futures_util::future::select(released, timeout).await;
    }
}
