use crate::app::{App, Peek};
use futures_util::StreamExt;
use gtk::glib;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;
use zbus::zvariant::OwnedValue;

fn glyph(icon: &str) -> &'static str {
    kit::bluez::symbol(icon)
}

/// Answers BlueZ when a pairing started from the Control Center needs it. Only pairings this
/// process starts reach us, so a code to compare is shown and accepted, and a code to type is shown.
pub fn agent(app: &Rc<App>) {
    thread_local! {
        static ON: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    if ON.with(|o| o.replace(true)) {
        return;
    }
    let weak = Rc::downgrade(app);
    glib::spawn_future_local(async move {
        let Ok(conn) = crate::bus::system().await else { return };
        let Some(rx) = kit::bluez::agent(&conn).await else {
            ON.with(|o| o.set(false));
            return;
        };
        while let Ok(req) = rx.recv().await {
            let note = |text: String| {
                if let Some(app) = weak.upgrade() {
                    app.request(Peek::Note { text });
                }
            };
            let answer = match req.ask {
                kit::bluez::Ask::Confirm(name, code) => {
                    if !code.is_empty() {
                        note(format!("Pairing with {name} · code {code}"));
                    }
                    Some(String::new())
                }
                kit::bluez::Ask::Pin(_) => Some("0000".into()),
                kit::bluez::Ask::Show(name, code) => {
                    note(format!("Type {code} on {name}"));
                    None
                }
                kit::bluez::Ask::Passkey(name) => {
                    note(format!("Pair {name} from Settings to type its code"));
                    None
                }
                kit::bluez::Ask::Cancel => None,
            };
            let _ = req.reply.try_send(answer);
        }
        ON.with(|o| o.set(false));
    });
}

pub fn start(app: &Rc<App>) {
    let weak = Rc::downgrade(app);
    glib::spawn_future_local(async move {
        let Ok(conn) = crate::bus::system().await else { return };
        let Ok(rule) = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender("org.bluez")
            .and_then(|b| b.interface("org.freedesktop.DBus.Properties"))
            .and_then(|b| b.member("PropertiesChanged"))
            .and_then(|b| b.add_arg("org.bluez.Device1"))
            .map(|b| b.build())
        else {
            return;
        };
        let Ok(mut stream) = zbus::MessageStream::for_match_rule(rule, &conn, Some(16)).await else { return };
        while let Some(Ok(msg)) = stream.next().await {
            let Some(path) = msg.header().path().map(|p| p.to_string()) else { continue };
            let Ok((_, changed, _)) = msg.body().deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>() else { continue };
            let Some(on) = changed.get("Connected").and_then(|v| bool::try_from(v.try_clone().ok()?).ok()) else { continue };
            if on {
                glib::timeout_future(Duration::from_millis(1200)).await;
            }
            let Ok(dev) = zbus::Proxy::new_owned(conn.clone(), "org.bluez", path.clone(), "org.bluez.Device1").await else { continue };
            let name: String = dev.get_property("Alias").await.unwrap_or_else(|_| "Bluetooth device".into());
            let icon: String = dev.get_property("Icon").await.unwrap_or_default();
            let battery = match zbus::Proxy::new_owned(conn.clone(), "org.bluez", path, "org.bluez.Battery1").await {
                Ok(b) if on => b.get_property::<u8>("Percentage").await.ok(),
                _ => None,
            };
            let Some(app) = weak.upgrade() else { break };
            app.request(Peek::Device { name: name.chars().take(32).collect(), icon: glyph(&icon), battery, on });
        }
    });
}
