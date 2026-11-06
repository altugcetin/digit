use gtk::{gdk, gio, prelude::*};
use std::cell::RefCell;
use std::path::PathBuf;

mod drop;
mod drop_data;

pub use drop::{accepts_drop, receive_drop};

pub struct Item {
    pub path: PathBuf,
    pub name: String,
    pub icon: Option<gdk::Paintable>,
}

thread_local! {
    static ITEMS: RefCell<Option<Vec<Item>>> = const { RefCell::new(None) };
}

const MAX: usize = 24;

fn store() -> PathBuf {
    kit::theme::state_dir().join("tray")
}

fn item(path: PathBuf) -> Option<Item> {
    let file = gio::File::for_path(&path);
    let info = file.query_info("standard::icon,standard::display-name,standard::content-type,thumbnail::path", gio::FileQueryInfoFlags::NONE, gio::Cancellable::NONE).ok()?;
    let display = gdk::Display::default()?;
    let theme = gtk::IconTheme::for_display(&display);
    let thumb = info.attribute_byte_string("thumbnail::path").map(PathBuf::from).filter(|p| p.exists());
    let image = info.content_type().is_some_and(|t| t.starts_with("image/"));
    let icon: Option<gdk::Paintable> = match thumb {
        Some(t) => gdk::Texture::from_filename(t).ok().map(|t| t.upcast()),
        None if image => crate::wallpaper::thumbnail(&path.to_string_lossy()).map(|t| t.upcast()),
        None => info.icon().map(|i| theme.lookup_by_gicon(&i, 48, 2, gtk::TextDirection::Ltr, gtk::IconLookupFlags::empty()).upcast()),
    };
    Some(Item { name: info.display_name().to_string(), path, icon })
}

fn with<R>(f: impl FnOnce(&mut Vec<Item>) -> R) -> R {
    ITEMS.with(|cell| {
        let mut slot = cell.borrow_mut();
        let items = slot.get_or_insert_with(|| {
            std::fs::read_to_string(store()).unwrap_or_default().lines().map(PathBuf::from).filter(|p| p.exists()).filter_map(item).collect()
        });
        f(items)
    })
}

fn save(items: &[Item]) {
    let text: String = items.iter().map(|i| format!("{}\n", i.path.display())).collect();
    let _ = std::fs::create_dir_all(kit::theme::state_dir());
    let _ = std::fs::write(store(), text);
}

pub fn len() -> usize {
    with(|items| items.len())
}

pub fn each(mut f: impl FnMut(usize, &Item)) {
    with(|items| items.iter().enumerate().for_each(|(i, it)| f(i, it)));
}

pub fn path(i: usize) -> Option<PathBuf> {
    with(|items| items.get(i).map(|it| it.path.clone()))
}

pub fn icon(i: usize) -> Option<gdk::Paintable> {
    with(|items| items.get(i).and_then(|it| it.icon.clone()))
}

pub fn add(paths: Vec<PathBuf>) {
    with(|items| {
        for p in paths {
            if items.iter().any(|it| it.path == p) {
                continue;
            }
            if let Some(it) = item(p) {
                items.insert(0, it);
            }
        }
        items.truncate(MAX);
        save(items);
    });
}

pub fn remove(i: usize) {
    with(|items| {
        if i < items.len() {
            let gone = items.remove(i);
            if gone.path.starts_with(stash()) {
                let _ = std::fs::remove_file(&gone.path);
            }
            save(items);
        }
    });
}

fn stash() -> PathBuf {
    gtk::glib::user_data_dir().join("digit/tray")
}

fn stamp() -> String {
    gtk::glib::DateTime::now_local().ok().and_then(|d| d.format("%Y-%m-%d %H.%M.%S").ok()).map(|s| s.to_string()).unwrap_or_else(|| "now".into())
}

fn fresh(name: &str) -> Option<PathBuf> {
    let dir = stash();
    std::fs::create_dir_all(&dir).ok()?;
    let clean: String = name.chars().map(|c| if c == '/' || c.is_control() { '-' } else { c }).take(80).collect();
    Some(dir.join(clean))
}

pub fn keep_texture(texture: &gdk::Texture) -> Option<PathBuf> {
    let path = fresh(&format!("Image {}.png", stamp()))?;
    texture.save_to_png(&path).ok()?;
    Some(path)
}

pub fn keep_text(text: &str) -> Vec<PathBuf> {
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if !lines.is_empty() && lines.iter().all(|l| l.starts_with("file://")) {
        return lines.iter().filter_map(|l| gio::File::for_uri(l).path()).collect();
    }
    let text = if text.len() > 1 << 20 { &text[..text.floor_char_boundary(1 << 20)] } else { text };
    let link = lines.len() == 1 && (lines[0].starts_with("https://") || lines[0].starts_with("http://"));
    let name = if link {
        let host = lines[0].split("://").nth(1).and_then(|r| r.split('/').next()).unwrap_or("link");
        format!("Link {host} {}.txt", stamp())
    } else {
        format!("Text {}.txt", stamp())
    };
    fresh(&name).filter(|p| std::fs::write(p, text).is_ok()).into_iter().collect()
}
