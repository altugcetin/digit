use crate::app::App;
use gtk::{glib, prelude::*};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

pub use kit::send::{Peer as Device, Via};

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Asking(String),
    Sending(String),
    Sent(String),
    Failed(String),
}

#[derive(Default)]
struct Shared {
    devices: Vec<Device>,
    scanning: bool,
    scanned: Option<Instant>,
    status: Option<Status>,
}

thread_local! {
    static STATE: RefCell<Shared> = RefCell::new(Shared::default());
}

pub fn devices() -> Vec<Device> {
    STATE.with(|s| s.borrow().devices.clone())
}

pub fn scanning() -> bool {
    STATE.with(|s| s.borrow().scanning)
}

pub fn status() -> Option<Status> {
    STATE.with(|s| s.borrow().status.clone())
}

pub fn scan(app: &Rc<App>) {
    let fresh = STATE.with(|s| {
        let s = s.borrow();
        s.scanning || s.scanned.is_some_and(|t| t.elapsed() < Duration::from_secs(20))
    });
    if fresh {
        return;
    }
    STATE.with(|s| s.borrow_mut().scanning = true);
    let weak = Rc::downgrade(app);
    glib::spawn_future_local(async move {
        let (local, kde) = futures_util::join!(async { gtk::gio::spawn_blocking(kit::send::discover).await.unwrap_or_default() }, kit::send::kde());
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.devices = kde.into_iter().chain(local).collect();
            s.scanning = false;
            s.scanned = Some(Instant::now());
        });
        if let Some(app) = weak.upgrade() {
            app.redraw();
        }
    });
}

fn set_status(app: &std::rc::Weak<App>, status: Option<Status>) {
    STATE.with(|s| s.borrow_mut().status = status);
    if let Some(app) = app.upgrade() {
        app.redraw();
    }
}

pub fn send(app: &Rc<App>, device: &Device, files: Vec<PathBuf>) {
    if files.is_empty() {
        return;
    }
    let weak = Rc::downgrade(app);
    let name = device.name.clone();
    set_status(&weak, Some(Status::Asking(name.clone())));
    match device.via.clone() {
        Via::KdeConnect { id } => {
            glib::spawn_future_local(async move {
                let ok = kit::send::kde_send(&id, &files).await;
                set_status(&weak, Some(if ok { Status::Sent(name) } else { Status::Failed("KDE Connect".into()) }));
            });
        }
        Via::LocalSend { ip, port, https } => {
            let (tx, rx) = async_channel::unbounded::<Status>();
            let _ = std::thread::Builder::new().name("digit-share".into()).spawn(move || {
                let progress = |f: String| {
                    let _ = tx.send_blocking(Status::Sending(f));
                };
                let result = kit::send::localsend(&ip, port, https, &files, &progress);
                let _ = tx.send_blocking(match result {
                    Ok(()) => Status::Sent(name),
                    Err(e) => Status::Failed(e),
                });
            });
            glib::spawn_future_local(async move {
                while let Ok(status) = rx.recv().await {
                    set_status(&weak, Some(status));
                }
            });
        }
    }
    let weak = Rc::downgrade(app);
    glib::timeout_add_local_once(Duration::from_secs(240), move || {
        let done = STATE.with(|s| matches!(s.borrow().status, Some(Status::Sent(_) | Status::Failed(_))));
        if done {
            set_status(&weak, None);
        }
    });
}

#[derive(Clone)]
pub struct Sender {
    pub name: String,
    pub info: Option<gtk::gio::AppInfo>,
    pub web: Option<&'static str>,
    pub icon: Option<gtk::gdk::Paintable>,
}

const KNOWN: &[(&str, &str)] = &[
    ("localsend", "LocalSend"),
    ("blip", "Blip"),
    ("warpinator", "Warpinator"),
    ("nozwock.packet", "Packet"),
    ("rquickshare", "RQuickShare"),
    ("quickshare", "Quick Share"),
    ("drey.warp", "Warp"),
    ("kdeconnect.handler", "KDE Connect"),
    ("kdeconnect-handler", "KDE Connect"),
    ("blueman-sendto", "Bluetooth"),
    ("bluetooth-sendto", "Bluetooth"),
    ("telegram", "Telegram"),
];

thread_local! {
    static SENDERS: RefCell<Option<Vec<Sender>>> = const { RefCell::new(None) };
}

pub fn senders() -> Vec<Sender> {
    if let Some(list) = SENDERS.with(|s| s.borrow().clone()) {
        return list;
    }
    let theme = gtk::gdk::Display::default().map(|d| gtk::IconTheme::for_display(&d));
    let mut list: Vec<Sender> = Vec::new();
    for info in gtk::gio::AppInfo::all() {
        let id = info.id().map(|i| i.to_lowercase()).unwrap_or_default();
        let Some((_, label)) = KNOWN.iter().find(|(k, _)| id.contains(k)) else { continue };
        if list.iter().any(|s| s.name == *label) {
            continue;
        }
        let icon = match (info.icon(), &theme) {
            (Some(i), Some(t)) => Some(t.lookup_by_gicon(&i, 48, 2, gtk::TextDirection::Ltr, gtk::IconLookupFlags::empty()).upcast()),
            _ => None,
        };
        list.push(Sender { name: label.to_string(), info: Some(info), web: None, icon });
    }
    list.push(Sender { name: "PairDrop".into(), info: None, web: Some("https://pairdrop.net/"), icon: None });
    SENDERS.with(|s| *s.borrow_mut() = Some(list.clone()));
    list
}

pub fn forget_senders() {
    SENDERS.with(|s| s.borrow_mut().take());
}

pub fn send_with(app: &Rc<App>, sender: &Sender, files: Vec<PathBuf>) {
    let context = gtk::gdk::Display::default().map(|d| d.app_launch_context());
    if let Some(c) = &context {
        crate::apps::prepare(c.upcast_ref());
    }
    let ctx = context.as_ref().map(|c| c.upcast_ref::<gtk::gio::AppLaunchContext>());
    let result = match (&sender.info, sender.web) {
        (_, Some(url)) => gtk::gio::AppInfo::launch_default_for_uri(url, ctx).map_err(|e| e.to_string()),
        (Some(info), None) => {
            let gfiles: Vec<gtk::gio::File> = files.iter().map(gtk::gio::File::for_path).collect();
            if !gfiles.is_empty() && info.supports_files() {
                info.launch(&gfiles, ctx).map_err(|e| e.to_string())
            } else if !gfiles.is_empty() && info.supports_uris() {
                let uris: Vec<String> = gfiles.iter().map(|f| f.uri().to_string()).collect();
                let refs: Vec<&str> = uris.iter().map(String::as_str).collect();
                info.launch_uris(&refs, ctx).map_err(|e| e.to_string())
            } else {
                info.launch(&[], ctx).map_err(|e| e.to_string())
            }
        }
        _ => Ok(()),
    };
    if let Err(e) = result {
        STATE.with(|s| s.borrow_mut().status = Some(Status::Failed(e)));
        app.redraw();
    }
}
