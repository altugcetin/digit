use crate::app::{App, Peek};
use futures_util::StreamExt;
use gtk::glib;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;
use zbus::zvariant::OwnedValue;

fn glyph(icon: &str) -> &'static str {
    match icon {
        i if i.contains("headset") || i.contains("headphone") => "headphones",
        i if i.contains("speaker") || i.contains("audio") => "speaker",
        i if i.contains("mouse") => "mouse",
        i if i.contains("keyboard") => "keyboard",
        i if i.contains("gaming") || i.contains("joystick") => "sports_esports",
        i if i.contains("phone") => "smartphone",
        i if i.contains("watch") => "watch",
        _ => "bluetooth",
    }
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
