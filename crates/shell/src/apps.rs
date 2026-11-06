use gio_unix::DesktopAppInfo;
use gtk::{gdk, gio, glib, prelude::*};
use kit::fuzzy::fold;
use serde::{Deserialize, Serialize};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use zbus::zvariant::Value;

pub struct Entry {
    pub id: String,
    pub name: String,
    pub detail: String,
    pub keys: Vec<(String, i32)>,
    pub wm_class: String,
    pub exec: String,
    pub icon: Option<gio::Icon>,
    pub info: gio::AppInfo,
}

#[derive(Serialize, Deserialize, Clone, Copy, Default)]
struct Use {
    count: f64,
    last: i64,
}

pub struct Apps {
    entries: RefCell<Rc<Vec<Entry>>>,
    stale: Cell<bool>,
    used: RefCell<HashMap<String, Use>>,
    _monitor: gio::AppInfoMonitor,
}

fn used_file() -> PathBuf {
    kit::theme::state_dir().join("launches.json")
}

fn unix_now() -> i64 {
    glib::real_time() / 1_000_000
}

impl Apps {
    pub fn new() -> Rc<Self> {
        let monitor = gio::AppInfoMonitor::get();
        let used = std::fs::read_to_string(used_file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let apps = Rc::new(Self { entries: RefCell::new(Rc::new(Vec::new())), stale: Cell::new(true), used: RefCell::new(used), _monitor: monitor.clone() });
        let weak = Rc::downgrade(&apps);
        monitor.connect_changed(move |_| {
            if let Some(apps) = weak.upgrade() {
                apps.stale.set(true);
            }
        });
        apps
    }

    pub fn list(&self) -> Rc<Vec<Entry>> {
        if self.stale.replace(false) {
            *self.entries.borrow_mut() = Rc::new(load());
        }
        self.entries.borrow().clone()
    }

    pub fn boost(&self, id: &str) -> i32 {
        let Some(u) = self.used.borrow().get(id).copied() else { return 0 };
        let days = (unix_now() - u.last).max(0) as f64 / 86_400.0;
        let weight = u.count * 0.5f64.powf(days / 14.0);
        ((1.0 + weight).ln() * 120.0).min(320.0) as i32
    }

    fn remember(&self, id: &str) {
        let now = unix_now();
        {
            let mut used = self.used.borrow_mut();
            let entry = used.entry(id.to_string()).or_default();
            let days = (now - entry.last).max(0) as f64 / 86_400.0;
            entry.count = entry.count * 0.5f64.powf(days / 14.0) + 1.0;
            entry.last = now;
        }
        let text = serde_json::to_string(&*self.used.borrow()).unwrap_or_default();
        let _ = std::fs::create_dir_all(kit::theme::state_dir());
        let _ = std::fs::write(used_file(), text);
    }

    pub fn launch(&self, entry: &Entry) {
        self.remember(&entry.id);
        let Some(display) = gdk::Display::default() else { return };
        let context = display.app_launch_context();
        prepare(context.upcast_ref());
        let id = entry.id.clone();
        context.connect_launched(move |_, _, data| adopt(&id, data));
        if let Err(e) = entry.info.launch(&[], Some(&context)) {
            eprintln!("digit: {}: {e}", entry.id);
        }
    }

    pub fn find(&self, app_id: &str) -> Option<usize> {
        if app_id.is_empty() {
            return None;
        }
        let key = app_id.to_ascii_lowercase();
        let tail = key.rsplit('.').next().unwrap_or(&key).to_string();
        let list = self.list();
        let stem = |e: &Entry| e.id.to_ascii_lowercase().trim_end_matches(".desktop").to_string();
        let rules: [&dyn Fn(&Entry) -> bool; 6] = [
            &|e| stem(e) == key,
            &|e| !e.wm_class.is_empty() && e.wm_class.to_ascii_lowercase() == key,
            &|e| stem(e) == tail || stem(e).rsplit('.').next() == Some(tail.as_str()),
            &|e| e.exec == tail,
            &|e| fold(&e.name).replace(' ', "") == tail,
            &|e| !e.wm_class.is_empty() && e.wm_class.to_ascii_lowercase() == tail,
        ];
        rules.iter().find_map(|rule| list.iter().position(|e| rule(e)))
    }
}

impl Apps {
    /// Display name for an app id or notification app name: the installed app's own name when
    /// we can find it ("firefox" → "Firefox"), otherwise the id with a capital initial ("niri" → "Niri").
    pub fn display_name(&self, app_id: &str) -> String {
        let found = self.find(app_id).and_then(|i| self.list().get(i).map(|e| e.name.clone()));
        found.filter(|n| !n.is_empty()).unwrap_or_else(|| pretty(app_id))
    }
}

pub fn pretty(app_id: &str) -> String {
    let tail = app_id.rsplit('.').next().unwrap_or(app_id).replace(['-', '_'], " ");
    let mut chars = tail.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

pub fn prepare(context: &gio::AppLaunchContext) {
    if kit::setup::installed() {
        let (theme, size) = kit::cursors::current(&kit::prefs::read());
        for key in ["XCURSOR_THEME", "HYPRCURSOR_THEME"] {
            if theme.is_empty() { context.unsetenv(key); } else { context.setenv(key, &theme); }
        }
        for key in ["XCURSOR_SIZE", "HYPRCURSOR_SIZE"] { context.setenv(key, size.to_string()); }
    }
    for (key, value) in crate::gpu::child_env() {
        match value {
            Some(v) => context.setenv(&key, &v),
            None => context.unsetenv(&key),
        }
    }
}

fn adopt(id: &str, data: &glib::Variant) {
    let Some(pid) = glib::VariantDict::new(Some(data)).lookup::<i32>("pid").ok().flatten() else { return };
    let unit: String = id
        .trim_end_matches(".desktop")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '.' { c } else { '_' })
        .collect();
    let unit = format!("app-digit-{unit}-{pid}.scope");
    glib::spawn_future_local(async move {
        let Ok(conn) = crate::bus::session().await else { return };
        let props: Vec<(&str, Value)> = vec![
            ("PIDs", Value::from(vec![pid as u32])),
            ("CollectMode", Value::from("inactive-or-failed")),
        ];
        let aux: Vec<(&str, Vec<(&str, Value)>)> = Vec::new();
        let _ = conn
            .call_method(
                Some("org.freedesktop.systemd1"),
                "/org/freedesktop/systemd1",
                Some("org.freedesktop.systemd1.Manager"),
                "StartTransientUnit",
                &(unit.as_str(), "fail", props, aux),
            )
            .await;
    });
}

fn load() -> Vec<Entry> {
    let mut out = Vec::new();
    for info in gio::AppInfo::all() {
        if !info.should_show() {
            continue;
        }
        let Some(id) = info.id().map(|s| s.to_string()) else { continue };
        let name = info.display_name().to_string();
        let desktop = info.downcast_ref::<DesktopAppInfo>();
        let generic = desktop.and_then(|d| d.generic_name()).map(|s| s.to_string()).unwrap_or_default();
        let comment = info.description().map(|s| s.to_string()).unwrap_or_default();
        let wm_class = desktop.and_then(|d| d.startup_wm_class()).map(|s| s.to_string()).unwrap_or_default();
        let exec = info.executable().file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let mut keys = vec![(fold(&name), 0), (fold(&generic), -120), (fold(&exec), -160)];
        if let Some(d) = desktop {
            keys.extend(d.keywords().iter().map(|k| (fold(k), -200)));
        }
        keys.retain(|(k, _)| !k.is_empty());
        out.push(Entry {
            id,
            detail: if generic.is_empty() { comment } else { generic },
            name,
            keys,
            wm_class,
            exec: exec.to_ascii_lowercase(),
            icon: info.icon(),
            info,
        });
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}
