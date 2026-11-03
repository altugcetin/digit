use crate::ui;
use gtk::{gio, prelude::*};
use std::rc::Rc;

type Kind = (&'static str, &'static str, &'static str, &'static [&'static str]);

const KINDS: [Kind; 11] = [
    ("public", "Web browser", "x-scheme-handler/https", &["x-scheme-handler/http", "text/html", "application/xhtml+xml"]),
    ("folder", "File manager", "inode/directory", &[]),
    ("mail", "Email", "x-scheme-handler/mailto", &[]),
    ("map", "Maps", "x-scheme-handler/geo", &[]),
    ("call", "Phone calls", "x-scheme-handler/tel", &[]),
    ("image", "Images", "image/png", &["image/jpeg", "image/gif", "image/webp", "image/bmp", "image/tiff", "image/avif", "image/heic", "image/svg+xml"]),
    ("music_note", "Music", "audio/mpeg", &["audio/flac", "audio/ogg", "audio/x-vorbis+ogg", "audio/x-wav", "audio/wav", "audio/mp4", "audio/aac", "audio/opus"]),
    ("movie", "Videos", "video/mp4", &["video/x-matroska", "video/webm", "video/quicktime", "video/x-msvideo", "video/mpeg", "video/ogg"]),
    ("description", "Text files", "text/plain", &["text/markdown", "application/json", "text/x-log", "application/x-yaml", "application/toml"]),
    ("picture_as_pdf", "PDF documents", "application/pdf", &[]),
    ("folder_zip", "Archives", "application/zip", &["application/x-tar", "application/gzip", "application/x-compressed-tar", "application/x-7z-compressed", "application/vnd.rar", "application/x-xz-compressed-tar", "application/zstd"]),
];

fn candidates(mime: &str) -> Vec<gio::AppInfo> {
    let mut out: Vec<gio::AppInfo> = Vec::new();
    for a in gio::AppInfo::recommended_for_type(mime).into_iter().chain(gio::AppInfo::all_for_type(mime)) {
        if !a.should_show() && a.id().is_none_or(|id| id.as_str() != "dev.digit.Files.desktop") {
            continue;
        }
        if out.iter().any(|o| o.id() == a.id()) {
            continue;
        }
        out.push(a);
    }
    out
}

pub fn section(body: &gtk::Box) {
    body.append(&ui::heading("Default apps"));
    let card = ui::card();
    for (glyph, title, main, more) in KINDS {
        let apps = candidates(main);
        if apps.is_empty() {
            continue;
        }
        let current = gio::AppInfo::default_for_type(main, false).and_then(|a| a.id());
        let mut labels: Vec<String> = Vec::new();
        let unset = current.as_ref().is_none_or(|c| !apps.iter().any(|a| a.id().as_ref() == Some(c)));
        if unset {
            labels.push("Not set".into());
        }
        labels.extend(apps.iter().map(|a| a.display_name().to_string()));
        let at = if unset { 0 } else { apps.iter().position(|a| a.id() == current).unwrap_or(0) };
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let (row, end) = ui::row(Some(glyph), title, "");
        let pick = kit::ui::dropdown(&refs, at as u32);
        let apps = Rc::new(apps);
        let accepted = std::cell::Cell::new(at as u32);
        let restoring = std::cell::Cell::new(false);
        pick.connect_selected_notify(move |p| {
            if restoring.get() { return; }
            let i = p.selected() as usize;
            let Some(i) = (if unset { i.checked_sub(1) } else { Some(i) }) else { return };
            let Some(app) = apps.get(i) else { return };
            let mut ok = true;
            let types = app.supported_types();
            for mime in std::iter::once(main).chain(more.iter().copied()) {
                if mime != main && !types.is_empty() && !types.iter().any(|t| t.as_str() == mime) {
                    continue;
                }
                ok &= app.set_as_default_for_type(mime).is_ok();
            }
            if ok {
                accepted.set(p.selected());
                ui::done(&format!("{} opens {}", app.display_name(), title.to_lowercase()));
            } else {
                ui::fail("Could not save the default app");
                // Some MIME types may have succeeded. Show the actual primary association.
                let actual = gio::AppInfo::default_for_type(main, false).and_then(|a| a.id());
                if let Some(i) = apps.iter().position(|a| a.id() == actual) {
                    restoring.set(true);
                    p.set_selected((i + usize::from(unset)) as u32);
                    restoring.set(false);
                } else if unset {
                    restoring.set(true);
                    p.set_selected(0);
                    restoring.set(false);
                } else {
                    restoring.set(true);
                    p.set_selected(accepted.get());
                    restoring.set(false);
                }
            }
        });
        end.append(&pick);
        card.append(&row);
    }
    body.append(&card);
}
