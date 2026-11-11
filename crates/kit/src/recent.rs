//! Freedesktop recent files. Listing uses GLib/GIO only and may run on a worker.
use gtk::{gio, glib, prelude::*};
use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};

pub struct Recent {
    pub path: PathBuf,
    pub name: String,
    /// Last use, in Unix seconds (the later of modified and visited).
    pub modified: i64,
    pub icon: gio::Icon,
}

const MAX_BYTES: u64 = 8 * 1024 * 1024;

fn xdg(key: &str, fallback: &str) -> PathBuf {
    std::env::var_os(key).map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| glib::home_dir().join(fallback))
}

fn location() -> PathBuf { xdg("XDG_DATA_HOME", ".local/share").join("recently-used.xbel") }

pub fn list(limit: usize) -> Vec<Recent> {
    read(&location(), &xdg("XDG_CACHE_HOME", ".cache"), limit)
}

fn read(file: &Path, cache: &Path, limit: usize) -> Vec<Recent> {
    if limit == 0 { return Vec::new(); }
    let Ok(file) = std::fs::File::open(file) else { return Vec::new() };
    let mut bytes = Vec::new();
    if file.take(MAX_BYTES + 1).read_to_end(&mut bytes).is_err() || bytes.len() as u64 > MAX_BYTES { return Vec::new(); }
    let Ok(text) = std::str::from_utf8(&bytes) else { return Vec::new() };
    let Ok(bookmarks) = roxmltree::Document::parse(text) else { return Vec::new() };
    let root = bookmarks.root_element();
    if !root.has_tag_name("xbel") { return Vec::new(); }
    let mut entries: Vec<_> = root.children().filter(|n| n.has_tag_name("bookmark")).filter_map(|bookmark| {
        let uri = bookmark.attribute("href")?;
        let (path, host) = glib::filename_from_uri(uri).ok()?;
        if host.as_deref().is_some_and(|h| !h.is_empty() && !h.eq_ignore_ascii_case("localhost")) || !path.is_file() { return None; }
        // Missing timestamps must not be synthesized as 'now' (GBookmarkFile does that).
        let modified = ["modified", "visited"].into_iter().filter_map(|key| bookmark.attribute(key))
            .filter_map(|s| glib::DateTime::from_iso8601(s, None).ok()).map(|t| t.to_unix()).max().unwrap_or(0);
        Some((path, uri, modified))
    }).collect();
    entries.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
    let mut seen = HashSet::new();
    entries.into_iter().filter(|(path, _, _)| seen.insert(path.clone())).take(limit).map(|(path, uri, modified)| {
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let hash = glib::compute_checksum_for_string(glib::ChecksumType::Md5, uri).unwrap();
        let icon = ["large", "normal"].into_iter().map(|size| cache.join("thumbnails").join(size).join(format!("{hash}.png")))
            .find(|p| p.is_file()).map(|p| gio::FileIcon::new(&gio::File::for_path(p)).upcast())
            .unwrap_or_else(|| gio::content_type_get_icon(&gio::content_type_guess(Some(&path), None).0));
        Recent { path, name, modified, icon }
    }).collect()
}

/// Watch the parent so atomic replacement of the XBEL file keeps being observed.
/// Call on the main thread and retain the returned monitor.
pub fn watch(f: impl Fn() + 'static) -> Option<gio::FileMonitor> {
    let file = gio::File::for_path(location());
    let monitor = file.parent()?.monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE).ok()?;
    monitor.connect_changed(move |_, changed, other, _| {
        if changed == &file || other == Some(&file) { f(); }
    });
    Some(monitor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!("digit-recent-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn uri(&self, name: &str) -> String { gio::File::for_path(self.0.join(name)).uri().to_string() }
    }
    impl Drop for Fixture { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }

    #[test]
    fn recent_fixture_filters_sorts_deduplicates_limits_and_uses_thumbnails() {
        let dir = Fixture::new();
        for name in ["older.txt", "new & last.txt"] { std::fs::write(dir.0.join(name), "file").unwrap(); }
        let old = dir.uri("older.txt");
        let new = dir.uri("new & last.txt");
        let alias = new.replacen("file:///", "file://localhost/", 1);
        let xbel = format!(r#"<?xml version="1.0"?><xbel version="1.0">
<bookmark href="{old}" modified="2026-01-01T00:00:00Z" visited="2026-01-02T00:00:00Z"/>
<bookmark href="{new}" modified="2026-01-03T00:00:00Z" visited="2026-01-05T00:00:00Z"/>
<bookmark href="{alias}" modified="2026-01-04T00:00:00Z"/>
<bookmark href="{}" modified="2026-01-06T00:00:00Z"/>
<bookmark href="https://example.com/remote" modified="2026-01-07T00:00:00Z"/>
<bookmark href="file://remotehost/tmp/file" modified="2026-01-08T00:00:00Z"/>
</xbel>"#, dir.uri("missing.txt")).replace("&", "&amp;");
        let file = dir.0.join("recently-used.xbel");
        std::fs::write(&file, xbel).unwrap();
        let hash = glib::compute_checksum_for_string(glib::ChecksumType::Md5, &new).unwrap();
        let thumb = dir.0.join("thumbnails/large").join(format!("{hash}.png"));
        std::fs::create_dir_all(thumb.parent().unwrap()).unwrap();
        std::fs::write(&thumb, b"thumbnail fixture").unwrap();
        let entries = read(&file, &dir.0, 20);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "new & last.txt");
        assert_eq!(entries[0].modified, 1767571200);
        assert_eq!(entries[0].icon.downcast_ref::<gio::FileIcon>().unwrap().file().path(), Some(thumb));
        assert!(entries[1].icon.is::<gio::ThemedIcon>());
        assert_eq!(read(&file, &dir.0, 1).len(), 1);
        assert!(read(&file, &dir.0, 0).is_empty());
    }

    #[test]
    fn rejects_missing_invalid_and_oversized_lists() {
        let dir = Fixture::new();
        let file = dir.0.join("recently-used.xbel");
        assert!(read(&file, &dir.0, 10).is_empty());
        std::fs::write(&file, "not XML").unwrap();
        assert!(read(&file, &dir.0, 10).is_empty());
        std::fs::File::create(&file).unwrap().set_len(MAX_BYTES + 1).unwrap();
        assert!(read(&file, &dir.0, 10).is_empty());
    }
}
