use gtk::{gio, glib};
use std::collections::HashMap;
use std::time::Duration;
use zbus::zvariant::{OwnedValue, Value};

pub type Item = (&'static str, &'static str);
pub type Menu = (&'static str, &'static [Item]);
pub const SEP: Item = ("", "");

pub struct Profile {
    pub menus: &'static [Menu],
    pub settings: Option<&'static str>,
}

pub use crate::menus::profile;

pub fn titles(app_id: &str) -> Vec<&'static str> {
    profile(app_id).menus.iter().map(|(t, _)| *t).collect()
}

fn key_name(k: &str) -> String {
    match k {
        "equal" => "=".into(),
        "minus" => "−".into(),
        "comma" => ",".into(),
        "slash" => "/".into(),
        "grave" => "`".into(),
        "Left" => "←".into(),
        "Right" => "→".into(),
        "Up" => "↑".into(),
        "Down" => "↓".into(),
        "Delete" => "Del".into(),
        "Escape" => "Esc".into(),
        "Return" => "Enter".into(),
        "BackSpace" => "Backspace".into(),
        "space" => "Space".into(),
        "Shift_L" => "Shift".into(),
        "plus" => "+".into(),
        "period" => ".".into(),
        "backslash" => "\\".into(),
        "bracketleft" => "[".into(),
        "bracketright" => "]".into(),
        "parenleft" => "(".into(),
        "parenright" => ")".into(),
        "apostrophe" => "'".into(),
        "quotedbl" => "\"".into(),
        "percent" => "%".into(),
        "asterisk" => "*".into(),
        "asciicircum" => "^".into(),
        "less" => "<".into(),
        "greater" => ">".into(),
        "numbersign" => "#".into(),
        "question" => "?".into(),
        "Page_Up" => "PgUp".into(),
        "Page_Down" => "PgDn".into(),
        "Insert" => "Ins".into(),
        k if k.chars().count() == 1 && k.chars().all(|c| c.is_ascii_uppercase()) => format!("Shift+{k}"),
        k if k.chars().count() == 1 => k.to_uppercase(),
        k => k.into(),
    }
}

pub fn hint(keys: &str) -> String {
    keys.split(' ')
        .map(|combo| {
            combo
                .split('+')
                .map(|p| match p {
                    "ctrl" => "Ctrl".to_string(),
                    "shift" => "Shift".into(),
                    "alt" => "Alt".into(),
                    "super" => "Super".into(),
                    k => key_name(k),
                })
                .collect::<Vec<_>>()
                .join("+")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn wtype_args(keys: &str) -> Vec<String> {
    let mut args = Vec::new();
    for (i, combo) in keys.split(' ').filter(|c| !c.is_empty()).enumerate() {
        if i > 0 {
            args.extend(["-s".to_string(), "45".to_string()]);
        }
        let parts: Vec<&str> = combo.split('+').collect();
        let (key, mods) = parts.split_last().map(|(k, m)| (*k, m.to_vec())).unwrap_or(("", Vec::new()));
        let mods: Vec<&str> = mods
            .into_iter()
            .filter_map(|m| match m {
                "ctrl" => Some("ctrl"),
                "shift" => Some("shift"),
                "alt" => Some("alt"),
                "super" => Some("logo"),
                _ => None,
            })
            .collect();
        for m in &mods {
            args.extend(["-M".to_string(), m.to_string()]);
        }
        args.extend(["-k".to_string(), key.to_string()]);
        for m in mods.iter().rev() {
            args.extend(["-m".to_string(), m.to_string()]);
        }
    }
    args
}

pub fn can_send() -> bool {
    thread_local! {
        static HAS: bool = glib::find_program_in_path("wtype").is_some();
    }
    HAS.with(|h| *h)
}

async fn focused_is(id: u64) -> bool {
    gio::spawn_blocking(move || kit::wm::focused_window().is_some_and(|w| w.id == id)).await.unwrap_or(false)
}

pub fn send(target: u64, keys: &'static str) {
    if keys.is_empty() || !can_send() {
        return;
    }
    glib::spawn_future_local(async move {
        glib::timeout_future(Duration::from_millis(110)).await;
        if !focused_is(target).await {
            let _ = gio::spawn_blocking(move || kit::wm::focus(target)).await;
            glib::timeout_future(Duration::from_millis(180)).await;
            if !focused_is(target).await {
                return;
            }
        }
        let args = wtype_args(keys);
        let argv: Vec<&std::ffi::OsStr> = std::iter::once("wtype").chain(args.iter().map(String::as_str)).map(std::ffi::OsStr::new).collect();
        if let Ok(p) = gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE) {
            let _ = p.wait_future().await;
        }
    });
}

pub struct Gtk {
    pub name: String,
    pub path: String,
    pub actions: Vec<String>,
}

pub async fn gtk_actions(app_id: &str) -> Option<Gtk> {
    if app_id.split('.').count() < 2 || !app_id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) {
        return None;
    }
    let conn = crate::bus::session().await.ok()?;
    let dbus = zbus::fdo::DBusProxy::new(&conn).await.ok()?;
    let name = zbus::names::BusName::try_from(app_id.to_string()).ok()?;
    if !dbus.name_has_owner(name.clone()).await.unwrap_or(false) {
        return None;
    }
    let path = format!("/{}", app_id.replace('.', "/").replace('-', "_"));
    let proxy = zbus::Proxy::new_owned(conn, app_id.to_string(), path.clone(), "org.gtk.Actions").await.ok()?;
    let call = proxy.call::<_, _, HashMap<String, (bool, zbus::zvariant::Signature, Vec<OwnedValue>)>>("DescribeAll", &());
    let timeout = glib::timeout_future(Duration::from_millis(220));
    let all = match futures_util::future::select(Box::pin(call), timeout).await {
        futures_util::future::Either::Left((Ok(all), _)) => all,
        _ => return None,
    };
    let actions = all.into_iter().filter(|(_, (on, sig, _))| *on && sig.to_string().is_empty()).map(|(k, _)| k).collect();
    Some(Gtk { name: app_id.to_string(), path, actions })
}

pub fn activate(name: String, path: String, action: String) {
    glib::spawn_future_local(async move {
        let Ok(conn) = crate::bus::session().await else { return };
        let Ok(proxy) = zbus::Proxy::new_owned(conn, name, path, "org.gtk.Actions").await else { return };
        let params: Vec<Value> = Vec::new();
        let data: HashMap<&str, Value> = HashMap::new();
        let _ = proxy.call_method("Activate", &(action.as_str(), params, data)).await;
    });
}
