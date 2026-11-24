//! Files dragged out of Ark. It gives no files, which don't exist yet, but the place on D-Bus
//! that extracts what is dragged to a folder it's told.

use gtk::{gdk, gio, glib, prelude::*};

const SERVICE: &str = "application/x-kde-ark-dndextract-service";
const PATH: &str = "application/x-kde-ark-dndextract-path";

/// Takes such a drag to the folder `dest` gives, where it gives one.
pub fn target(dest: impl Fn() -> Option<gio::File> + 'static) -> gtk::DropTargetAsync {
    // Hyprland offers only MOVE before negotiation, so both are taken; nothing is ever moved.
    let target = gtk::DropTargetAsync::new(Some(gdk::ContentFormats::new(&[SERVICE, PATH])), gdk::DragAction::COPY | gdk::DragAction::MOVE);
    let dest = std::rc::Rc::new(dest);
    let to = dest.clone();
    target.connect_accept(move |_, drop| {
        let formats = drop.formats();
        formats.contain_mime_type(SERVICE) && formats.contain_mime_type(PATH) && to().is_some()
    });
    target.connect_drop(move |_, drop, _, _| {
        let Some(dir) = dest() else { return false };
        let drop = drop.clone();
        glib::spawn_future_local(async move {
            let from = (text(&drop, SERVICE).await, text(&drop, PATH).await);
            let ok = matches!(&from, (Some(s), Some(p)) if valid(s, p)) && dir.path().is_some();
            drop.finish(if ok { gdk::DragAction::COPY } else { gdk::DragAction::empty() });
            let Some(dir) = dir.path() else { return crate::toast("Ark extracts only to local folders") };
            if let (true, (Some(service), Some(path))) = (ok, from)
                && let Err(e) = extract(&service, &path, &dir).await
            {
                crate::toast(&format!("Ark couldn't extract: {e}"));
            }
        });
        true
    });
    target
}

async fn extract(service: &str, path: &str, dir: &std::path::Path) -> Result<(), glib::Error> {
    let bus = gio::bus_get_future(gio::BusType::Session).await?;
    let told = bus.call_future(Some(service), path, "org.kde.ark.DndExtract", "extractSelectedFilesTo", Some(&(dir.to_string_lossy().into_owned(),).to_variant()), None, gio::DBusCallFlags::NO_AUTO_START, -1).await;
    match told {
        // Ark may ask about files that are there already, for as long as it takes.
        Err(e) if e.matches(gio::IOErrorEnum::TimedOut) || e.matches(gio::DBusError::NoReply) => Ok(()),
        other => other.map(|_| ()),
    }
}

async fn text(drop: &gdk::Drop, mime: &str) -> Option<String> {
    let (stream, _) = drop.read_future(&[mime], glib::Priority::DEFAULT).await.ok()?;
    let bytes = stream.read_bytes_future(512, glib::Priority::DEFAULT).await.ok()?;
    Some(String::from_utf8_lossy(&bytes).trim_end_matches('\0').to_string())
}

fn valid(service: &str, path: &str) -> bool {
    gio::dbus_is_name(service) && glib::Variant::is_object_path(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_place_on_the_bus_is_called() {
        assert!(valid(":1.234", "/DndExtract/1"));
        assert!(valid("org.kde.ark-4821", "/DndExtract/1"));
        assert!(!valid("", "/DndExtract/1"));
        assert!(!valid(":1.234", "DndExtract"));
        assert!(!valid("not a name", "/"));
    }
}
