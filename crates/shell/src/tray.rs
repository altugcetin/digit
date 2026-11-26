use gtk::{gdk, gio, prelude::*};
use std::cell::RefCell;
use std::path::PathBuf;

mod drop;
mod drop_data;

pub use drop::{accepts_drop, receive_drop};

#[derive(Clone)]
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
        None => info.icon().map(|i| kit::svgicon::steady(theme.lookup_by_gicon(&i, 48, 2, gtk::TextDirection::Ltr, gtk::IconLookupFlags::empty()).upcast_ref())),
    };
    Some(Item { name: info.display_name().to_string(), path, icon })
}

fn with<R>(f: impl FnOnce(&mut Vec<Item>) -> R) -> R {
    ITEMS.with(|cell| {
        if cell.borrow().is_none() {
            let paths = std::fs::read_to_string(store()).unwrap_or_default().lines().map(PathBuf::from).filter(|p| p.exists()).take(MAX).collect::<Vec<_>>();
            let items = paths.into_iter().filter_map(item).collect();
            let mut slot = cell.borrow_mut();
            if slot.is_none() { *slot = Some(items); }
        }
        f(cell.borrow_mut().as_mut().unwrap())
    })
}

fn write_state(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    kit::atomic::write(path, text)
}

fn save(items: &[Item]) {
    let text: String = items.iter().map(|i| format!("{}\n", i.path.display())).collect();
    if let Err(e) = write_state(&store(), &text) { eprintln!("digit: tray save: {e}"); }
}

pub fn len() -> usize {
    with(|items| items.len())
}

pub fn each(mut f: impl FnMut(usize, &Item)) {
    let items = with(|items| items.clone());
    items.iter().enumerate().for_each(|(i, it)| f(i, it));
}

pub fn path(i: usize) -> Option<PathBuf> {
    with(|items| items.get(i).map(|it| it.path.clone()))
}

pub fn icon(i: usize) -> Option<gdk::Paintable> {
    with(|items| items.get(i).and_then(|it| it.icon.clone()))
}

fn incoming_paths(paths: Vec<PathBuf>, existing: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen: std::collections::HashSet<_> = existing.iter().cloned().collect();
    let mut incoming = std::collections::VecDeque::new();
    for path in paths {
        if seen.insert(path.clone()) {
            incoming.push_back(path);
            if incoming.len() > MAX { incoming.pop_front(); }
        }
    }
    incoming.into_iter().collect()
}

pub fn add(paths: Vec<PathBuf>) {
    let existing = with(|items| items.iter().map(|it| it.path.clone()).collect::<Vec<_>>());
    let incoming = incoming_paths(paths, &existing).into_iter().filter_map(item).collect::<Vec<_>>();
    let (_discarded, saved) = with(|items| {
        let mut discarded = Vec::new();
        for it in incoming {
            if items.iter().any(|old| old.path == it.path) { discarded.push(it); }
            else { items.insert(0, it); }
        }
        discarded.extend(items.split_off(MAX.min(items.len())));
        (discarded, items.clone())
    });
    save(&saved);
}

pub fn remove(i: usize) {
    let removed = with(|items| {
        (i < items.len()).then(|| (items.remove(i), items.clone()))
    });
    if let Some((gone, saved)) = removed {
        if gone.path.starts_with(stash()) {
            let _ = std::fs::remove_file(&gone.path);
        }
        save(&saved);
    }
}

fn stash() -> PathBuf {
    gtk::glib::user_data_dir().join("digit/tray")
}

fn stamp() -> String {
    gtk::glib::DateTime::now_local().ok().and_then(|d| d.format("%Y-%m-%d %H.%M.%S").ok()).map(|s| s.to_string()).unwrap_or_else(|| "now".into())
}

fn keep_bytes(dir: &std::path::Path, name: &str, bytes: &[u8]) -> Option<PathBuf> {
    use std::io::Write;
    std::fs::create_dir_all(dir).ok()?;
    let clean: String = name.chars().map(|c| if c == '/' || c.is_control() { '-' } else { c }).take(120).collect();
    let name = std::path::Path::new(&clean);
    let stem = name.file_stem()?.to_string_lossy();
    let ext = name.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    for n in 1.. {
        let path = dir.join(if n == 1 { clean.clone() } else { format!("{stem} ({n}){ext}") });
        let file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path);
        let mut file = match file {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return None,
        };
        if file.write_all(bytes).and_then(|_| file.sync_all()).is_err() {
            let _ = std::fs::remove_file(&path);
            return None;
        }
        return Some(path);
    }
    None
}

pub fn keep_texture(texture: &gdk::Texture) -> Option<PathBuf> {
    keep_bytes(&stash(), &format!("Image {}.png", stamp()), &texture.save_to_png_bytes())
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
    keep_bytes(&stash(), &name, text.as_bytes()).into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incoming_paths_are_limited_before_loading_and_keep_first_duplicate_order() {
        let paths: Vec<_> = (0..MAX + 10).map(|n| PathBuf::from(n.to_string())).collect();
        let mut repeated = paths.clone();
        repeated.push(paths[10].clone());
        let selected = incoming_paths(repeated, &[paths[MAX + 9].clone()]);
        assert_eq!(selected, paths[9..MAX + 9]);
        assert_eq!(selected.len(), MAX);
    }

    #[test]
    fn state_replaces_complete_file_and_cleans_up_failed_rename() {
        let dir = std::env::temp_dir().join(format!("digit-tray-state-{}", gtk::glib::uuid_string_random()));
        let path = dir.join("tray");
        write_state(&path, "first\n").unwrap();
        write_state(&path, "second\nthird\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second\nthird\n");
        let blocked = dir.join("blocked");
        std::fs::create_dir(&blocked).unwrap();
        assert!(write_state(&blocked, "replacement").is_err());
        assert!(blocked.is_dir());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn concurrent_drops_create_exclusive_files() {
        let dir = std::env::temp_dir().join(format!("digit-tray-race-{}", gtk::glib::uuid_string_random()));
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let workers: Vec<_> = (0u8..8).map(|n| {
            let dir = dir.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                (keep_bytes(&dir, "Image today.png", &[n]).unwrap(), n)
            })
        }).collect();
        let files: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
        for (path, n) in files { assert_eq!(std::fs::read(path).unwrap(), [n]); }
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 8);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn repeated_drop_names_keep_both_files() {
        let dir = std::env::temp_dir().join(format!("digit-tray-test-{}", gtk::glib::uuid_string_random()));
        let a = keep_bytes(&dir, "Text today.txt", b"first").unwrap();
        let b = keep_bytes(&dir, "Text today.txt", b"second").unwrap();
        assert_eq!(b.file_name().unwrap(), "Text today (2).txt");
        assert_eq!(std::fs::read(a).unwrap(), b"first");
        assert_eq!(std::fs::read(b).unwrap(), b"second");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
