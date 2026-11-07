//! Says so when config.toml can't be used: the shell runs on defaults and saves nothing to it.
use crate::config;
use gtk::{gio, glib, prelude::*};
use std::cell::RefCell;

const IFACE: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";

thread_local! {
    static SHOWN: RefCell<Option<String>> = const { RefCell::new(None) };
    static WAITING: RefCell<Option<gio::SignalSubscription>> = const { RefCell::new(None) };
}

/// Notifies once per distinct problem, and again if it comes back after being fixed.
pub fn check() {
    let problem = kit::prefs::problem();
    if SHOWN.with(|s| s.replace(problem.clone())) == problem {
        return;
    }
    let Some(problem) = problem else { return };
    glib::spawn_future_local(async move {
        let Ok(conn) = gio::bus_get_future(gio::BusType::Session).await else { return };
        let body = format!("{problem}. Changes aren't saved until it's fixed.");
        let actions = vec!["default".to_string(), "Open".to_string()];
        let hints = glib::VariantDict::new(None);
        let args = ("Digit", 0u32, "dev.digit.Settings", "Settings file has an error", body.as_str(), actions, hints.end(), 0i32).to_variant();
        let reply = conn.call_future(Some(IFACE), PATH, IFACE, "Notify", Some(&args), None, gio::DBusCallFlags::NONE, 2000).await;
        let Some(id) = reply.ok().and_then(|r| r.get::<(u32,)>()).map(|r| r.0) else { return };
        let waiting = conn.subscribe_to_signal(Some(IFACE), Some(IFACE), Some("ActionInvoked"), Some(PATH), None, gio::DBusSignalFlags::NONE, move |signal| {
            if signal.parameters.get::<(u32, String)>().is_some_and(|(invoked, _)| invoked == id) {
                config::edit(&config::Config::default());
            }
        });
        WAITING.with(|w| *w.borrow_mut() = Some(waiting));
    });
}
