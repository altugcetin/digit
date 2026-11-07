use crate::app::App;
use crate::compositor;
use gtk::{gio, glib, prelude::*};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;
use zbus::zvariant::{OwnedValue, Value};

const IFACE: &str = "org.gnome.Shell.SearchProvider2";
const PER: usize = 5;

#[derive(Clone)]
pub struct Provider {
    pub bus: String,
    pub path: String,
    pub name: String,
    pub icon: Option<gio::Icon>,
}

pub struct Found {
    pub glyph: Option<String>,
    pub provider: usize,
    pub id: String,
    pub name: String,
    pub detail: String,
    pub icon: Option<gio::Icon>,
}

#[derive(Clone)]
pub enum Reach {
    Url(String),
    Switcher { ids: Vec<String>, entry: Option<usize> },
    Store,
    /// digit Settings, searched for what is typed.
    Settings,
}

#[derive(Clone)]
pub struct Scope {
    pub key: String,
    pub name: String,
    pub reach: Reach,
    pub icon: Option<gio::Icon>,
    pub symbol: &'static str,
    pub app: bool,
}

pub fn discover(app: &App, table: &toml::Table) -> Vec<Provider> {
    if !kit::prefs::flag(table, "search_apps", true) {
        return Vec::new();
    }
    let off = kit::search::off_list(table);
    let list = app.apps.list();
    kit::search::listed()
        .into_iter()
        .filter(|l| kit::search::enabled(l, &off))
        .filter_map(|l| {
            let entry = list.get(app.apps.find(&l.id)?)?;
            Some(Provider { bus: l.bus, path: l.path, name: entry.name.clone(), icon: entry.icon.clone() })
        })
        .collect()
}

fn text(m: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    let v: &Value = m.get(key)?;
    <&str>::try_from(v).ok().map(str::to_string)
}

fn icon_of(m: &HashMap<String, OwnedValue>) -> Option<gio::Icon> {
    if let Some(s) = text(m, "gicon") {
        return gio::Icon::for_string(&s).ok();
    }
    let Value::Structure(st) = &**m.get("icon")? else { return None };
    let fields = st.fields();
    let kind = <&str>::try_from(fields.first()?).ok()?;
    let inner = match fields.get(1)? {
        Value::Value(b) => &**b,
        other => other,
    };
    match kind {
        "themed" => {
            let Value::Array(names) = inner else { return None };
            let names: Vec<String> = names.iter().filter_map(|n| <&str>::try_from(n).ok().map(str::to_string)).collect();
            let refs: Vec<&str> = names.iter().map(String::as_str).collect();
            Some(gio::ThemedIcon::from_names(&refs).upcast())
        }
        "file" => {
            let p = <&str>::try_from(inner).ok()?;
            Some(gio::FileIcon::new(&gio::File::for_uri(p)).upcast())
        }
        _ => None,
    }
}

async fn proxy(p: &Provider) -> Option<zbus::Proxy<'static>> {
    let conn = crate::bus::session().await.ok()?;
    zbus::proxy::Builder::<zbus::Proxy>::new(&conn)
        .destination(p.bus.clone())
        .ok()?
        .path(p.path.clone())
        .ok()?
        .interface(IFACE)
        .ok()?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
        .ok()
}

pub async fn query(p: Provider, index: usize, terms: Vec<String>) -> Vec<Found> {
    let call = async move {
        let proxy = proxy(&p).await?;
        let ids: Vec<String> = proxy.call("GetInitialResultSet", &(terms,)).await.ok()?;
        let ids: Vec<String> = ids.into_iter().take(PER).collect();
        if ids.is_empty() {
            return Some(Vec::new());
        }
        let metas: Vec<HashMap<String, OwnedValue>> = proxy.call("GetResultMetas", &(ids,)).await.ok()?;
        let glyphs = p.bus == "org.gnome.Characters";
        Some(
            metas
                .iter()
                .filter_map(|m| {
                    let id = text(m, "id")?;
                    let name = text(m, "name").filter(|n| !n.trim().is_empty())?;
                    let detail = text(m, "description").map(|d| d.lines().next().unwrap_or("").trim().to_string()).unwrap_or_default();
                    let glyph = (glyphs && (1..=8).contains(&id.chars().count()) && !id.chars().any(char::is_control)).then(|| id.clone());
                    Some(Found { glyph, provider: index, id, name: name.lines().next().unwrap_or("").to_string(), detail, icon: icon_of(m).or_else(|| p.icon.clone()) })
                })
                .collect(),
        )
    };
    glib::future_with_timeout(Duration::from_millis(1200), call).await.ok().flatten().unwrap_or_default()
}

fn stamp() -> u32 {
    (glib::monotonic_time() / 1000) as u32
}

pub fn activate(p: Provider, id: String, terms: Vec<String>) {
    glib::spawn_future_local(async move {
        if let Some(proxy) = proxy(&p).await {
            let _: zbus::Result<()> = proxy.call("ActivateResult", &(id, terms, stamp())).await;
        }
    });
}

pub fn launch_search(p: Provider, terms: Vec<String>) {
    glib::spawn_future_local(async move {
        if let Some(proxy) = proxy(&p).await {
            let _: zbus::Result<()> = proxy.call("LaunchSearch", &(terms, stamp())).await;
        }
    });
}

const CHATS: [(&str, &str); 6] = [
    ("com.discordapp.Discord", "discord"),
    ("dev.vencord.Vesktop", "vesktop"),
    ("app.legcord.Legcord", "legcord"),
    ("io.github.equicord.equibop", "equibop"),
    ("io.github.spacingbat3.webcord", "webcord"),
    ("discord", "discord"),
];

pub fn scopes(app: &App, table: &toml::Table) -> Vec<Scope> {
    let list = app.apps.list();
    let mut out = Vec::new();
    let installed: Vec<(usize, &str, &str)> = CHATS.iter().filter_map(|(id, short)| app.apps.find(id).map(|i| (i, *id, *short))).collect();
    if !installed.is_empty() && glib::find_program_in_path("wtype").is_some() {
        let running = app.running.borrow().iter().map(|r| r.app_id.to_lowercase()).collect::<Vec<_>>();
        let pick = installed.iter().find(|(_, id, short)| running.iter().any(|r| r == &id.to_lowercase() || r == short)).or(installed.first()).copied();
        if let Some((i, id, short)) = pick {
            let name = list.get(i).map(|e| e.name.clone()).unwrap_or_else(|| "Discord".into());
            out.push(Scope {
                key: "dc".into(),
                name,
                reach: Reach::Switcher { ids: vec![id.to_lowercase(), short.to_string()], entry: Some(i) },
                icon: list.get(i).and_then(|e| e.icon.clone()),
                symbol: "forum",
                app: true,
            });
        }
    }
    for (id, key, url) in [("com.spotify.Client", "sp", "spotify:search:{}"), ("com.valvesoftware.Steam", "st", "steam://openurl/https://store.steampowered.com/search/?term={}"), ("steam", "st", "steam://openurl/https://store.steampowered.com/search/?term={}")] {
        if out.iter().any(|s: &Scope| s.key == key) {
            continue;
        }
        if let Some(e) = app.apps.find(id).and_then(|i| list.get(i)) {
            out.push(Scope { key: key.into(), name: e.name.clone(), reach: Reach::Url(url.into()), icon: e.icon.clone(), symbol: "search", app: true });
        }
    }
    if glib::find_program_in_path("digit-settings").is_some() {
        out.push(Scope { key: "set".into(), name: "Settings".into(), reach: Reach::Settings, icon: None, symbol: "settings", app: false });
    }
    if glib::find_program_in_path("digit-store").is_some() {
        out.push(Scope { key: "app".into(), name: "Digit Store".into(), reach: Reach::Store, icon: None, symbol: "storefront", app: false });
    }
    for (key, name, url, symbol) in [
        ("yt", "YouTube", "https://www.youtube.com/results?search_query={}", "smart_display"),
        ("gh", "GitHub", "https://github.com/search?q={}", "code"),
        ("w", "Wikipedia", "https://en.wikipedia.org/w/index.php?search={}", "menu_book"),
    ] {
        out.push(Scope { key: key.into(), name: name.into(), reach: Reach::Url(url.into()), icon: None, symbol, app: false });
    }
    if let Some(user) = table.get("search_scopes").and_then(|v| v.as_array()) {
        for t in user.iter().filter_map(|v| v.as_table()) {
            let field = |k: &str| t.get(k).and_then(|v| v.as_str()).map(str::trim).unwrap_or("").to_string();
            let (key, name, url) = (field("key"), field("name"), field("url"));
            let scheme_ok = url.split_once(':').is_some_and(|(s, _)| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)) && !matches!(s.to_ascii_lowercase().as_str(), "file" | "javascript" | "data"));
            if key.is_empty() || key.contains(' ') || name.is_empty() || !url.contains("{}") || !scheme_ok {
                continue;
            }
            out.retain(|s| s.key != key);
            out.push(Scope { key, name, reach: Reach::Url(url), icon: None, symbol: "travel_explore", app: false });
        }
    }
    // The ones turned off in Settings > Spotlight.
    let off: Vec<&str> = table.get("search_scopes_off").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|v| v.as_str()).collect()).unwrap_or_default();
    out.retain(|s| !off.contains(&s.key.as_str()));
    out
}

fn typed(query: &str) -> String {
    query.chars().filter(|c| !c.is_control()).take(200).collect()
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    gio::spawn_blocking(f).await.ok()
}

async fn wtype(args: &[&str], stdin: Option<&str>) -> bool {
    let argv: Vec<&std::ffi::OsStr> = std::iter::once("wtype").chain(args.iter().copied()).map(std::ffi::OsStr::new).collect();
    let flags = if stdin.is_some() { gio::SubprocessFlags::STDIN_PIPE } else { gio::SubprocessFlags::NONE } | gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE;
    let Ok(p) = gio::Subprocess::newv(&argv, flags) else { return false };
    let input = stdin.map(|s| glib::Bytes::from(s.as_bytes()));
    p.communicate_future(input.as_ref()).await.is_ok() && p.is_successful()
}

pub fn switcher(app: Rc<App>, ids: Vec<String>, entry: Option<usize>, query: String) {
    let text = typed(&query);
    glib::spawn_future_local(async move {
        let wanted = |a: &str| ids.iter().any(|i| i.eq_ignore_ascii_case(a));
        let find = {
            let ids = ids.clone();
            move || compositor::windows().into_iter().find(|w| ids.iter().any(|i| i.eq_ignore_ascii_case(&w.app_id))).map(|w| w.id)
        };
        let mut target = blocking(find.clone()).await.flatten();
        let mut fresh = false;
        if target.is_none() {
            let list = app.apps.list();
            let Some(e) = entry.and_then(|i| list.get(i)) else { return };
            app.apps.launch(e);
            for _ in 0..60 {
                glib::timeout_future(Duration::from_millis(300)).await;
                target = blocking(find.clone()).await.flatten();
                if target.is_some() {
                    break;
                }
            }
            fresh = true;
        }
        let Some(id) = target else { return };
        let _ = blocking(move || compositor::focus_after_panel(id)).await;
        glib::timeout_future(Duration::from_millis(if fresh { 4500 } else { 280 })).await;
        let focused = blocking(compositor::focused_app).await.flatten().unwrap_or_default();
        if !wanted(&focused) {
            return;
        }
        if !wtype(&["-M", "ctrl", "k", "-m", "ctrl"], None).await || text.is_empty() {
            return;
        }
        glib::timeout_future(Duration::from_millis(260)).await;
        let focused = blocking(compositor::focused_app).await.flatten().unwrap_or_default();
        if wanted(&focused) {
            wtype(&["-"], Some(&text)).await;
        }
    });
}
