use crate::app::{App, Peek};
use futures_util::StreamExt;
use gtk::{gdk, gio, glib, prelude::*};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};

const NAME: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";

/// Icon size requested from the theme; drawn at 18 logical px, so this stays sharp at 2x.
const ICON_PX: i32 = 36;
const INGRESS: usize = 64;
const FILE_MAX: u64 = 8 * 1024 * 1024;

fn newest<T>(queue: &mut VecDeque<T>, value: T, cap: usize) -> Option<T> {
    // Overload keeps the newest arrivals; callers explicitly close evicted notifications.
    let dropped = if queue.len() >= cap { queue.pop_front() } else { None };
    queue.push_back(value);
    dropped
}

/// File sources are checked and decoded on workers before GTK resolves paintables.
enum IconSource {
    Named(String),
    Loaded(gdk::Texture),
    File(String),
}

#[derive(Default)]
struct Look {
    app_icon: Option<IconSource>,
    desktop_entry: Option<String>,
    image: Option<IconSource>,
}

enum Event {
    Notify { id: u32, app: String, summary: String, body: String, default: bool, urgent: bool, timeout: Option<u64>, look: Look, transient: bool, actions: Vec<(String, String)> },
    Close(u32),
}

struct Server {
    tx: async_channel::Sender<()>,
    queue: Arc<Mutex<VecDeque<Event>>>,
    image_px: i32,
    next: AtomicU32,
    active: Arc<Mutex<HashSet<u32>>>,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Server {
    fn get_capabilities(&self) -> Vec<String> {
        vec!["body".into(), "actions".into()]
    }

    #[allow(clippy::too_many_arguments)]
    async fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<u32> {
        let id = assign(&self.next, replaces_id);
        let urgency = hints.get("urgency").and_then(|v| u8::try_from(v.try_clone().ok()?).ok()).unwrap_or(1);
        let default = actions.chunks(2).any(|pair| pair[0] == "default");
        let buttons: Vec<(String, String)> = actions
            .chunks(2)
            .filter(|p| p.len() == 2 && p[0] != "default" && !p[1].trim().is_empty())
            .take(3)
            .map(|p| (p[0].chars().take(64).collect(), plain(&p[1].chars().take(256).collect::<String>()).chars().take(24).collect()))
            .collect();
        let look = Look {
            app_icon: icon_from_str(&app_icon),
            desktop_entry: hint_str(&hints, &["desktop-entry"]),
            image: ["image-data", "image_data", "icon_data"]
                .iter()
                .find_map(|k| hints.get(*k).and_then(|v| pixels(v, self.image_px)))
                .or_else(|| hint_str(&hints, &["image-path", "image_path"]).and_then(|p| icon_from_str(&p))),
        };
        let transient = hints.get("transient").and_then(|v| bool::try_from(v.try_clone().ok()?).ok().or_else(|| u8::try_from(v.try_clone().ok()?).ok().map(|x| x != 0))).unwrap_or(false);
        let event = Event::Notify {
            transient,
            timeout: timeout_ms(expire_timeout),
            id,
            app: app_name.chars().take(256).collect(),
            summary: plain(&summary.chars().take(4096).collect::<String>()),
            body: plain(&body.chars().take(16384).collect::<String>()),
            default,
            urgent: urgency >= 2,
            look,
            actions: buttons,
        };
        if let Ok(mut active) = self.active.lock() { active.insert(id); }
        self.enqueue(event, &emitter).await?;
        Ok(id)
    }

    async fn close_notification(&self, id: u32, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) -> zbus::fdo::Result<()> {
        self.enqueue(Event::Close(id), &emitter).await
    }

    fn get_server_information(&self) -> (String, String, String, String) {
        ("digit".into(), "digit".into(), env!("CARGO_PKG_VERSION").into(), "1.2".into())
    }

    #[zbus(signal)]
    async fn notification_closed(emitter: &SignalEmitter<'_>, id: u32, reason: u32) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn action_invoked(emitter: &SignalEmitter<'_>, id: u32, action_key: &str) -> zbus::Result<()>;
}

impl Server {
    async fn enqueue(&self, event: Event, emitter: &SignalEmitter<'_>) -> zbus::fdo::Result<()> {
        let dropped = {
            let mut queue = self.queue.lock().map_err(|_| zbus::fdo::Error::Failed("Notification queue failed".into()))?;
            if let Event::Notify { id, .. } = &event {
                queue.retain(|e| !matches!(e, Event::Notify { id: old, .. } if old == id));
            }
            newest(&mut queue, event, INGRESS)
        };
        match self.tx.try_send(()) {
            Ok(()) | Err(async_channel::TrySendError::Full(())) => {}
            Err(_) => return Err(zbus::fdo::Error::Failed("Notifications are unavailable".into())),
        }
        if let Some(event) = dropped {
            let (id, reason) = match event { Event::Notify { id, .. } => (id, 4), Event::Close(id) => (id, 3) };
            let removed = self.active.lock().map(|mut a| a.remove(&id)).unwrap_or(false);
            if removed { Server::notification_closed(emitter, id, reason).await?; }
        }
        Ok(())
    }
}

/// The id a notification gets: the one it replaces, or the next free one. A client may make up
/// the id it replaces (`notify-send -r 1234`); the ids handed out then move past it, so a later
/// notification never takes that one's place.
fn assign(next: &AtomicU32, replaces: u32) -> u32 {
    if replaces > 0 {
        next.fetch_max(replaces.saturating_add(1), Ordering::Relaxed);
        replaces
    } else {
        next.fetch_add(1, Ordering::Relaxed)
    }
}

fn timeout_ms(timeout: i32) -> Option<u64> {
    (timeout > 0).then(|| (timeout as u64).clamp(100, 300_000))
}

fn icon_from_str(value: &str) -> Option<IconSource> {
    let value = value.trim();
    if value.len() > 4096 { return None; }
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
        Some(Value::Str(s)) if !s.is_empty() => Some(s.chars().take(4096).collect()),
        _ => None,
    })
}

/// Decodes the `(iiibiiay)` image hint: width, height, rowstride, alpha, bits, channels, data.
fn pixels(value: &OwnedValue, px: i32) -> Option<IconSource> {
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
    let channels = channels as usize;
    pixel_layout(width, height, stride, channels, array.len())?;
    let (w, h) = crate::images::fit(width, height, px)?;
    let src: Vec<u8> = (0..array.len()).map(|i| array.get::<u8>(i).ok().flatten()).collect::<Option<_>>()?;
    let data = shrink(&src, (width as usize, height as usize, stride as usize), channels, (w as usize, h as usize));
    // Made here, off the main thread, so the notch only draws it.
    let format = if alpha { gdk::MemoryFormat::R8g8b8a8 } else { gdk::MemoryFormat::R8g8b8 };
    let texture = gdk::MemoryTexture::new(w, h, format, &glib::Bytes::from_owned(data), w as usize * channels);
    Some(IconSource::Loaded(texture.upcast()))
}

/// Scales an image down by averaging each output pixel's whole source area, so a big avatar
/// keeps smooth edges at icon size instead of the jagged look of picking single pixels.
fn shrink(src: &[u8], (sw, sh, stride): (usize, usize, usize), channels: usize, (w, h): (usize, usize)) -> Vec<u8> {
    let mut out = Vec::with_capacity(w * h * channels);
    for y in 0..h {
        let (y0, y1) = (y * sh / h, ((y + 1) * sh / h).max(y * sh / h + 1).min(sh));
        for x in 0..w {
            let (x0, x1) = (x * sw / w, ((x + 1) * sw / w).max(x * sw / w + 1).min(sw));
            let mut sum = [0u32; 4];
            for row in y0..y1 {
                for col in x0..x1 {
                    let at = row * stride + col * channels;
                    for c in 0..channels {
                        sum[c] += u32::from(src[at + c]);
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            out.extend(sum[..channels].iter().map(|v| (v / n.max(1)) as u8));
        }
    }
    out
}

fn pixel_layout(w: i32, h: i32, stride: i32, channels: usize, len: usize) -> Option<usize> {
    if w <= 0 || h <= 0 || stride <= 0 || !(3..=4).contains(&channels) { return None; }
    let row = (w as usize).checked_mul(channels)?;
    if (stride as usize) < row { return None; }
    let needed = (stride as usize).checked_mul(h as usize - 1)?.checked_add(row)?;
    (needed <= len && needed <= 32 * 1024 * 1024).then_some(needed)
}

fn read_image(location: &str, px: i32) -> Option<gdk::Texture> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let path = if location.starts_with("file://") { gio::File::for_uri(location).path()? } else { std::path::PathBuf::from(location) };
    // O_NONBLOCK prevents a raced FIFO from blocking open; inspect the opened descriptor.
    let file = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW).open(path).ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() > FILE_MAX { return None; }
    let mut bytes = Vec::new();
    file.take(FILE_MAX + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > FILE_MAX { return None; }
    crate::images::decode(&bytes, px)
}

async fn load_files(mut look: Look, px: i32) -> Look {
    use futures_util::future::{Either, select};
    static ACTIVE: AtomicU32 = AtomicU32::new(0);
    struct Permit;
    impl Drop for Permit { fn drop(&mut self) { ACTIVE.fetch_sub(1, Ordering::Relaxed); } }
    let has_file = [&look.app_icon, &look.image].iter().any(|s| matches!(s, Some(IconSource::File(_))));
    if !has_file { return look; }
    let strip = |s: Option<IconSource>| s.filter(|s| !matches!(s, IconSource::File(_)));
    if ACTIVE.try_update(Ordering::Relaxed, Ordering::Relaxed, |n| (n < 2).then_some(n + 1)).is_err() {
        look.app_icon = strip(look.app_icon);
        look.image = strip(look.image);
        return look;
    }
    let fallback = Look { app_icon: None, desktop_entry: look.desktop_entry.clone(), image: None };
    let permit = Permit;
    let job = gio::spawn_blocking(move || {
        let _permit = permit;
        let load = |s| match s {
            Some(IconSource::File(path)) => read_image(&path, px).map(IconSource::Loaded),
            other => other,
        };
        look.app_icon = load(look.app_icon);
        look.image = load(look.image);
        look
    });
    let timeout = glib::timeout_future(Duration::from_millis(500));
    futures_util::pin_mut!(job, timeout);
    match select(job, timeout).await {
        Either::Left((Ok(look), _)) => look,
        _ => fallback,
    }
}

/// A theme icon as a texture decoded off the main thread at `px`. Symbolic icons stay as they
/// are (they're recolored as they're drawn), and anything slow or unreadable keeps the original.
async fn bake(p: Option<gdk::Paintable>, px: i32) -> Option<gdk::Paintable> {
    use futures_util::future::{Either, select};
    let p = p?;
    let Some(path) = p.downcast_ref::<gtk::IconPaintable>().filter(|i| !i.is_symbolic()).and_then(|i| i.file()).and_then(|f| f.path()) else { return Some(kit::svgicon::steady(&p)) };
    let job = gio::spawn_blocking(move || {
        let bytes = std::fs::read(&path).ok().filter(|b| (b.len() as u64) <= FILE_MAX)?;
        crate::images::decode(&bytes, px)
    });
    let timeout = glib::timeout_future(Duration::from_millis(300));
    futures_util::pin_mut!(job, timeout);
    match select(job, timeout).await {
        Either::Left((Ok(Some(t)), _)) => Some(t.upcast()),
        // Kept as it is: the island grows and shrinks it.
        _ => Some(kit::svgicon::steady(&p)),
    }
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
        IconSource::Loaded(t) => Some(t.clone().upcast()),
        IconSource::File(_) => None,
    }
}

/// Square-ish images can stand in for a logo; wide ones (screenshots, photos) are content and
/// would be an unreadable smear at header size.
fn icon_like(source: &IconSource) -> bool {
    let (w, h) = match source {
        IconSource::Named(_) => return true,
        IconSource::Loaded(t) => (t.width(), t.height()),
        IconSource::File(_) => return false,
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

#[derive(Clone)]
pub struct Handle {
    conn: zbus::Connection,
    defaults: Rc<RefCell<VecDeque<u32>>>,
    active: Arc<Mutex<HashSet<u32>>>,
    timeouts: Rc<RefCell<HashMap<u32, u64>>>,
}

impl Handle {
    fn take_default(&self, id: u32) -> bool {
        let mut defaults = self.defaults.borrow_mut();
        let found = defaults.contains(&id);
        defaults.retain(|i| *i != id);
        found
    }

    fn set_default(&self, id: u32, enabled: bool) {
        self.take_default(id);
        if enabled {
            let mut defaults = self.defaults.borrow_mut();
            newest(&mut defaults, id, 256);
        }
    }

    pub fn timeout(&self, id: u32) -> Option<u64> { self.timeouts.borrow().get(&id).copied() }

    fn emit(&self, id: u32, reason: u32, action: Option<String>) {
        if !self.active.lock().map(|mut a| a.remove(&id)).unwrap_or(false) { return; }
        self.timeouts.borrow_mut().remove(&id);
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
        self.take_default(id);
        self.emit(id, reason, None);
    }

    pub fn clicked(&self, id: u32) {
        let default = self.take_default(id);
        self.emit(id, 2, default.then(|| "default".to_string()));
    }

    pub fn invoke(&self, id: u32, key: &str) {
        self.take_default(id);
        self.emit(id, 2, Some(key.to_string()));
    }

    pub fn has_default(&self, id: u32) -> bool {
        self.defaults.borrow().contains(&id)
    }
}

pub fn start(app: &Rc<App>) {
    let (tx, rx) = async_channel::bounded(1);
    let queue = Arc::new(Mutex::new(VecDeque::new()));
    let image_px = 96 * crate::images::scale();
    let weak = Rc::downgrade(app);
    glib::spawn_future_local(async move {
        let active = Arc::new(Mutex::new(HashSet::new()));
        let server = Server { tx, queue: queue.clone(), image_px, next: AtomicU32::new(1), active: active.clone() };
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
        *app.notify.borrow_mut() = Some(Handle { conn: conn.clone(), defaults: Rc::new(RefCell::new(VecDeque::new())), active, timeouts: Rc::new(RefCell::new(HashMap::new())) });
        drop(app);
        glib::spawn_future_local(acquire(conn));
        loop {
            let event = queue.lock().ok().and_then(|mut q| q.pop_front());
            let Some(event) = event else {
                if rx.recv().await.is_err() { break; }
                continue;
            };
            let Some(app) = weak.upgrade() else { break };
            match event {
                Event::Notify { id, app: source, summary, body, default, urgent, timeout, look, transient, actions } => {
                    // Resolve in one serial consumer so slow file loads cannot reorder arrivals.
                    let look = load_files(look, image_px).await;
                    let handle = app.notify.borrow().clone();
                    if let Some(h) = handle {
                        h.set_default(id, default);
                        h.timeouts.borrow_mut().remove(&id);
                        if let Some(timeout) = timeout { h.timeouts.borrow_mut().insert(id, timeout); }
                    }
                    let full = body.clone();
                    let summary = if summary.is_empty() { body } else { summary };
                    let line = if full == summary { String::new() } else { full.split_whitespace().collect::<Vec<_>>().join(" ") };
                    let icon = resolve(&app, &source, &look);
                    let face = avatar(&look);
                    let badge = if face.is_some() { logo(&app, &source, &look) } else { icon.clone() };
                    // Theme icons read and rasterize their file the first time they're drawn,
                    // which would land in the frame the notch opens; do it here, off the thread.
                    let scale = crate::images::scale();
                    let (icon, badge, face) = (bake(icon, 48 * scale).await, bake(badge, 48 * scale).await, bake(face, 96 * scale).await);
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
                    let handle = app.notify.borrow().clone();
                    if let Some(h) = handle { h.closed(id, 3); }
                }
            }
        }
    });
}

async fn acquire(conn: zbus::Connection) {
    let Ok(dbus) = zbus::fdo::DBusProxy::new(&conn).await else { return };
    let Ok(mut changes) = dbus.receive_name_owner_changed_with_args(&[(0, NAME)]).await else { return };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ingress_keeps_newest_in_arrival_order_and_reports_evictions() {
        let mut q = VecDeque::new();
        assert_eq!(newest(&mut q, 1, 2), None);
        assert_eq!(newest(&mut q, 2, 2), None);
        assert_eq!(newest(&mut q, 3, 2), Some(1));
        assert_eq!(q.into_iter().collect::<Vec<_>>(), vec![2, 3]);
    }

    #[test]
    fn pixel_hints_are_downsampled_before_entering_the_queue() {
        let pixels = vec![127u8; 800 * 400 * 4];
        let st = zbus::zvariant::Structure::from((800i32, 400i32, 3200i32, true, 8i32, 4i32, pixels));
        let value = OwnedValue::try_from(Value::from(st)).unwrap();
        let Some(IconSource::Loaded(t)) = super::pixels(&value, 96) else { panic!("pixel hint rejected") };
        assert_eq!((t.width(), t.height()), (96, 48));
    }

    #[test]
    fn shrinking_averages_each_area_instead_of_picking_one_pixel() {
        // A 4x2 one-channel-like RGB image of black and white columns shrinks to grey.
        let row = [0u8, 0, 0, 255, 255, 255, 0, 0, 0, 255, 255, 255];
        let src: Vec<u8> = row.iter().chain(row.iter()).copied().collect();
        let out = super::shrink(&src, (4, 2, 12), 3, (2, 1));
        assert_eq!(out, vec![127, 127, 127, 127, 127, 127]);
    }

    #[test]
    fn image_layout_rejects_invalid_strides_overflow_and_oversized_payloads() {
        assert_eq!(pixel_layout(2, 2, 8, 4, 16), Some(16));
        assert_eq!(pixel_layout(2, 2, 7, 4, 16), None);
        assert_eq!(pixel_layout(2, 2, 8, 4, 15), None);
        assert_eq!(pixel_layout(-1, 2, 8, 4, 16), None);
        assert_eq!(pixel_layout(i32::MAX, i32::MAX, i32::MAX, 4, usize::MAX), None);
        assert_eq!(pixel_layout(4096, 4096, 16384, 4, usize::MAX), None);
    }

    #[test]
    fn a_made_up_id_is_never_handed_out_again() {
        let next = AtomicU32::new(1);
        assert_eq!(assign(&next, 0), 1);
        assert_eq!(assign(&next, 5), 5);
        assert_eq!(assign(&next, 0), 6, "past the id a client chose");
        assert_eq!(assign(&next, 2), 2);
        assert_eq!(assign(&next, 0), 7, "an older id doesn't move the count back");
    }

    #[test]
    fn notification_timeout_uses_defaults_and_bounds() {
        assert_eq!(super::timeout_ms(-1), None);
        assert_eq!(super::timeout_ms(0), None);
        assert_eq!(super::timeout_ms(1), Some(100));
        assert_eq!(super::timeout_ms(2400), Some(2400));
        assert_eq!(super::timeout_ms(i32::MAX), Some(300_000));
    }

    #[test]
    fn notification_images_reject_fifos_without_opening_a_stream() {
        use std::os::unix::ffi::OsStrExt;
        let path = std::env::temp_dir().join(format!("digit-notify-fifo-{}", glib::uuid_string_random()));
        let cpath = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) }, 0);
        assert!(read_image(path.to_str().unwrap(), 96).is_none());
        std::fs::remove_file(path).unwrap();
    }
}
