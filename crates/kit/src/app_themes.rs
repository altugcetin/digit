//! Opt-in toolkit ownership. Called by the shell's serialized worker only.
use gtk::{gio, glib, prelude::*};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Copy)]
pub enum Toolkit {
    Gtk,
    Qt,
}
impl Toolkit {
    fn name(self) -> &'static str {
        match self {
            Self::Gtk => "gtk",
            Self::Qt => "qt",
        }
    }
    pub fn files(self) -> &'static [&'static str] {
        match self {
            Self::Gtk => &["gtk-3.0/gtk.css", "gtk-3.0/settings.ini", "gtk-4.0/gtk.css", "gtk-4.0/settings.ini"],
            Self::Qt => &["qt6ct/qt6ct.conf", "qt6ct/colors/digit.conf"],
        }
    }
}

pub trait DesktopSettings {
    fn get(&self, key: &str) -> io::Result<Option<String>>;
    fn put(&self, key: &str, value: Option<&str>) -> io::Result<()>;
}

#[derive(Default)]
pub struct GSettings(std::cell::OnceCell<Option<gio::Settings>>);
impl GSettings {
    fn settings(&self) -> Option<&gio::Settings> {
        self.0
            .get_or_init(|| {
                let schema = gio::SettingsSchemaSource::default().and_then(|s| s.lookup("org.gnome.desktop.interface", true));
                schema.map(|s| gio::Settings::new_full(&s, gio::SettingsBackend::NONE, None))
            })
            .as_ref()
    }
}
impl DesktopSettings for GSettings {
    fn get(&self, key: &str) -> io::Result<Option<String>> {
        Ok(self.settings().and_then(|s| s.user_value(key)).map(|v| v.print(true).to_string()))
    }
    fn put(&self, key: &str, value: Option<&str>) -> io::Result<()> {
        let Some(s) = self.settings() else { return Ok(()) };
        if !s.is_writable(key) {
            return Err(io::Error::other(format!("{key} is not writable")));
        }
        if let Some(value) = value {
            let value = glib::Variant::parse(None, value).map_err(io::Error::other)?;
            s.set_value(key, &value).map_err(io::Error::other)?;
        } else {
            s.reset(key);
        }
        gio::Settings::sync();
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
struct Snapshot {
    files: Vec<Option<Vec<u8>>>,
    settings: BTreeMap<String, Option<String>>,
}

pub struct Store {
    pub config: PathBuf,
    state: PathBuf,
}
impl Default for Store {
    fn default() -> Self {
        Self::new(glib::user_config_dir(), crate::theme::state_dir().join("app-themes"))
    }
}
impl Store {
    pub fn new(config: PathBuf, state: PathBuf) -> Self {
        Self { config, state }
    }
    fn path(&self, kind: Toolkit) -> PathBuf {
        self.state.join(format!("{}.json", kind.name()))
    }
    fn snapshot(&self, kind: Toolkit) -> io::Result<Option<Snapshot>> {
        let Some(bytes) = read(&self.path(kind))? else { return Ok(None) };
        let snapshot: Snapshot = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        let keys: &[&str] = if matches!(kind, Toolkit::Gtk) { &["color-scheme", "gtk-theme"] } else { &[] };
        if snapshot.files.len() != kind.files().len()
            || snapshot.settings.len() != keys.len()
            || keys.iter().any(|k| !snapshot.settings.contains_key(*k))
        {
            return Err(io::Error::other("Incomplete theme snapshot"));
        }
        Ok(Some(snapshot))
    }
    pub fn enable(&self, kind: Toolkit, settings: &impl DesktopSettings) -> io::Result<()> {
        if self.snapshot(kind)?.is_some() {
            return Ok(());
        }
        let files = kind.files().iter().map(|p| read(&self.config.join(p))).collect::<io::Result<_>>()?;
        let mut values = BTreeMap::new();
        if matches!(kind, Toolkit::Gtk) {
            for key in ["color-scheme", "gtk-theme"] {
                values.insert(key.into(), settings.get(key)?);
            }
        }
        atomic_write(&self.path(kind), &serde_json::to_vec(&Snapshot { files, settings: values }).map_err(io::Error::other)?)
    }
    pub fn original_file(&self, kind: Toolkit, index: usize) -> io::Result<()> {
        if let Some(snapshot) = self.snapshot(kind)? {
            restore(&self.config.join(kind.files()[index]), snapshot.files[index].as_deref())?;
        }
        Ok(())
    }
    pub fn disable(&self, kind: Toolkit, settings: &impl DesktopSettings) -> io::Result<()> {
        let Some(snapshot) = self.snapshot(kind)? else { return Ok(()) };
        for (path, contents) in kind.files().iter().zip(&snapshot.files) {
            restore(&self.config.join(path), contents.as_deref())?;
        }
        for (key, value) in &snapshot.settings {
            settings.put(key, value.as_deref())?;
        }
        std::fs::remove_file(self.path(kind))
    }
    pub fn apply(
        &self,
        kind: Toolkit,
        on: bool,
        settings: &impl DesktopSettings,
        write: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<()> {
        if on {
            self.enable(kind, settings)?;
            write()
        } else {
            self.disable(kind, settings)
        }
    }
}

fn read(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}
fn restore(path: &Path, bytes: Option<&[u8]>) -> io::Result<()> {
    if let Some(bytes) = bytes {
        atomic_write(path, bytes)
    } else {
        match std::fs::remove_file(path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    if read(path)?.as_deref() == Some(bytes) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let tmp = path.with_extension(format!("{}.{}.new", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        file.write_all(bytes)?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(tmp);
    }
    result
}

/// Filesystem discovery is always called on a worker.
pub fn gtk_roots() -> Vec<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| glib::home_dir().join(".local/share"));
    let dirs = std::env::var_os("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    std::iter::once(glib::home_dir().join(".themes"))
        .chain(std::iter::once(data.join("themes")))
        .chain(std::env::split_paths(&dirs).map(|p| p.join("themes")))
        .collect()
}
pub fn gtk_themes(roots: &[PathBuf]) -> Vec<String> {
    let mut names = BTreeSet::new();
    for root in roots {
        for entry in std::fs::read_dir(root).into_iter().flatten().flatten() {
            if ["gtk-3.0/gtk.css", "gtk-4.0/gtk.css"].iter().any(|p| entry.path().join(p).is_file())
                && let Some(name) = entry.file_name().to_str().filter(|n| !n.chars().any(char::is_control))
            {
                names.insert(name.to_owned());
            }
        }
    }
    names.into_iter().collect()
}
pub fn qt_style_name(file: &str) -> Option<String> {
    let name = file.strip_suffix(".so")?.trim_start_matches("lib");
    Some(match name {
        "breeze6" | "breeze" => "Breeze".into(),
        "kvantum" => "kvantum".into(),
        "qcleanlooksstyle" => "Cleanlooks".into(),
        "qplastiquestyle" => "Plastique".into(),
        "qwindowsvistastyle" => "windowsvista".into(),
        _ => return None, // Unknown plugin names do not reliably identify QStyle keys.
    })
}
// Qt 6 embeds CBOR plugin metadata. Read its QStyle Keys without loading code or Qt.
fn plugin_styles(bytes: &[u8]) -> Vec<String> {
    fn length(bytes: &mut &[u8], major: u8) -> Option<usize> {
        let (tag, rest) = bytes.split_first()?;
        *bytes = rest;
        if tag >> 5 != major {
            return None;
        }
        match tag & 31 {
            n @ 0..=23 => Some(n as usize),
            24 => {
                let (n, rest) = bytes.split_first()?;
                *bytes = rest;
                Some(*n as usize)
            }
            25 => {
                let n = u16::from_be_bytes(bytes.get(..2)?.try_into().ok()?);
                *bytes = &bytes[2..];
                Some(n as usize)
            }
            _ => None,
        }
    }
    let mut names = Vec::new();
    for at in bytes.windows(5).enumerate().filter_map(|(i, b)| (b == b"\x64Keys").then_some(i + 5)) {
        let mut data = &bytes[at..];
        let Some(count) = length(&mut data, 4).filter(|n| *n <= 64) else { continue };
        for _ in 0..count {
            let Some(len) = length(&mut data, 3).filter(|n| *n <= 256) else { break };
            let Some(text) = data.get(..len).and_then(|b| std::str::from_utf8(b).ok()) else { break };
            if !text.is_empty() && !text.chars().any(char::is_control) && !matches!(text, "qt6ct-style" | "qt5ct-style") {
                names.push(text.to_owned());
            }
            data = &data[len..];
        }
    }
    names
}
pub fn qt_styles_in(roots: &[PathBuf]) -> Vec<String> {
    let mut names = BTreeSet::from(["Fusion".into()]);
    for root in roots {
        for entry in std::fs::read_dir(root).into_iter().flatten().flatten() {
            if !entry.path().is_file() || entry.path().extension().is_none_or(|e| e != "so") {
                continue;
            }
            let found = std::fs::read(entry.path()).map(|b| plugin_styles(&b)).unwrap_or_default();
            if found.is_empty() {
                if let Some(name) = entry.file_name().to_str().and_then(qt_style_name) {
                    names.insert(name);
                }
            } else {
                names.extend(found);
            }
        }
    }
    names.into_iter().collect()
}
pub fn qt_styles() -> Vec<String> {
    let mut roots: Vec<PathBuf> = [
        "/usr/lib/qt6/plugins/styles",
        "/usr/lib64/qt6/plugins/styles",
        "/usr/lib/x86_64-linux-gnu/qt6/plugins/styles",
        "/usr/local/lib/qt6/plugins/styles",
    ]
    .iter()
    .map(PathBuf::from)
    .collect();
    if let Some(paths) = std::env::var_os("QT_PLUGIN_PATH") {
        roots.extend(std::env::split_paths(&paths).map(|p| p.join("styles")));
    }
    qt_styles_in(&roots)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Settings(std::cell::RefCell<BTreeMap<String, Option<String>>>);
    impl DesktopSettings for Settings {
        fn get(&self, k: &str) -> io::Result<Option<String>> {
            Ok(self.0.borrow().get(k).cloned().flatten())
        }
        fn put(&self, k: &str, v: Option<&str>) -> io::Result<()> {
            self.0.borrow_mut().insert(k.into(), v.map(str::to_owned));
            Ok(())
        }
    }
    fn root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("digit-app-themes-{name}-{}", std::process::id()))
    }
    #[test]
    fn enable_twice_preserves_originals_and_disable_restores_bytes_and_absence() {
        let root = root("restore");
        let store = Store::new(root.join("config"), root.join("state"));
        let settings = Settings::default();
        settings.put("gtk-theme", Some("'Original'")).unwrap();
        for kind in [Toolkit::Gtk, Toolkit::Qt] {
            let path = store.config.join(kind.files()[0]);
            atomic_write(&path, b"original\0\xff\r\n").unwrap();
            store
                .apply(kind, true, &settings, || {
                    for p in kind.files() {
                        atomic_write(&store.config.join(p), b"Digit")?;
                    }
                    Ok(())
                })
                .unwrap();
            settings.put("gtk-theme", Some("'Digit'")).unwrap();
            store.enable(kind, &settings).unwrap();
            assert_eq!(store.snapshot(kind).unwrap().unwrap().files[0].as_deref(), Some(b"original\0\xff\r\n".as_slice()));
            atomic_write(&path, b"hand edit").unwrap();
            store.disable(kind, &settings).unwrap();
            assert_eq!(std::fs::read(path).unwrap(), b"original\0\xff\r\n");
            for p in &kind.files()[1..] {
                assert!(!store.config.join(p).exists());
            }
            assert!(!store.path(kind).exists());
            if matches!(kind, Toolkit::Gtk) {
                assert_eq!(settings.get("gtk-theme").unwrap().as_deref(), Some("'Original'"));
                assert_eq!(settings.get("color-scheme").unwrap(), None);
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn both_masters_off_write_nothing_and_missing_snapshot_leaves_files_alone() {
        let root = root("off");
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::new(root.join("config"), root.join("state"));
        for kind in [Toolkit::Gtk, Toolkit::Qt] {
            store.apply(kind, false, &Settings::default(), || panic!("off master wrote themes")).unwrap();
        }
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        let path = store.config.join(Toolkit::Gtk.files()[0]);
        atomic_write(&path, b"user").unwrap();
        store.disable(Toolkit::Gtk, &Settings::default()).unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"user");
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn restore_failure_keeps_the_snapshot_for_retry() {
        let root = root("failure");
        let store = Store::new(root.join("config"), root.join("state"));
        let settings = Settings::default();
        let path = store.config.join(Toolkit::Qt.files()[0]);
        atomic_write(&path, b"original").unwrap();
        store.enable(Toolkit::Qt, &settings).unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(store.disable(Toolkit::Qt, &settings).is_err());
        assert!(store.path(Toolkit::Qt).is_file());
        std::fs::remove_dir(&path).unwrap();
        store.disable(Toolkit::Qt, &settings).unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"original");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn enumeration_deduplicates_and_requires_a_toolkit_stylesheet() {
        let root = root("themes");
        for p in ["one/Z/gtk-3.0/gtk.css", "two/Z/gtk-4.0/gtk.css", "one/A/gtk-4.0/gtk.css", "one/Invalid/readme"] {
            atomic_write(&root.join(p), b"").unwrap();
        }
        assert_eq!(gtk_themes(&[root.join("one"), root.join("two")]), ["A", "Z"]);
        for p in ["styles/breeze6.so", "styles/libbreeze.so", "styles/libkvantum.so"] {
            atomic_write(&root.join(p), b"").unwrap();
        }
        assert_eq!(qt_styles_in(&[root.join("styles")]), ["Breeze", "Fusion", "kvantum"]);
        assert_eq!(qt_style_name("README"), None);
        assert_eq!(qt_style_name("libbreeze6.so"), Some("Breeze".into()));
        atomic_write(&root.join("styles/UnionWidgetsStyle.so"), b"metadata\x64Keys\x81eUnion\xff").unwrap();
        assert_eq!(qt_styles_in(&[root.join("styles")]), ["Breeze", "Fusion", "Union", "kvantum"]);
        assert!(plugin_styles(b"\x64Keys\x81\x78\xff").is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
}
