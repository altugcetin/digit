use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

pub fn path() -> PathBuf {
    gtk::glib::user_config_dir().join("digit/config.toml")
}

pub fn read() -> toml::Table {
    std::fs::read_to_string(path()).ok().and_then(|t| t.parse::<toml::Table>().ok()).unwrap_or_default()
}

pub fn text(t: &toml::Table, key: &str, fallback: &str) -> String {
    t.get(key).and_then(|v| v.as_str()).unwrap_or(fallback).to_string()
}

pub fn flag(t: &toml::Table, key: &str, fallback: bool) -> bool {
    t.get(key).and_then(|v| v.as_bool()).unwrap_or(fallback)
}

pub fn number(t: &toml::Table, key: &str, fallback: f64) -> f64 {
    t.get(key).and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64))).unwrap_or(fallback)
}

pub fn list(t: &toml::Table, key: &str) -> Vec<String> {
    t.get(key).and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// The settings file for changing it: an error rather than an empty table when it exists but
/// doesn't parse, so a typo never gets every setting overwritten.
fn for_writing() -> Result<toml::Table, String> {
    match std::fs::read_to_string(path()) {
        Ok(text) => text.parse::<toml::Table>().map_err(|e| format!("{} has an error, so nothing was saved: {e}", path().display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(toml::Table::new()),
        Err(e) => Err(format!("{} could not be read, so nothing was saved: {e}", path().display())),
    }
}

/// Reports a broken or unreadable settings file without changing it.
pub fn problem() -> Option<String> {
    let text = match std::fs::read_to_string(path()) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => return Some(format!("Can't read config.toml: {e}")),
    };
    text.parse::<toml::Table>().err().map(|e| {
        let line = e.span().map(|span| format!(" on line {}", text.as_bytes()[..span.start.min(text.len())].iter().filter(|&&b| b == b'\n').count() + 1)).unwrap_or_default();
        format!("Invalid config.toml{line}: {}", e.message())
    })
}

fn write(table: &toml::Table) -> Result<(), String> {
    let body = toml::to_string_pretty(table).map_err(|e| e.to_string())?;
    let file = path();
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let (tmp, mut out) = loop {
        let tmp = file.with_extension(format!("toml.{}.{}.new", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(out) => break (tmp, out),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    };
    let result = out.write_all(body.as_bytes()).and_then(|()| std::fs::rename(&tmp, &file));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map_err(|e| e.to_string())
}

pub fn set(key: &str, value: toml::Value) -> Result<(), String> {
    set_many(&[(key, value)])
}

/// Save related values together, preserving the old file if any part cannot be written.
pub fn set_many(values: &[(&str, toml::Value)]) -> Result<(), String> {
    update(|table| {
        let mut changed = false;
        for (key, value) in values {
            if table.get(*key) != Some(value) {
                table.insert((*key).into(), value.clone());
                changed = true;
            }
        }
        changed
    })
}

pub fn unset(key: &str) -> Result<(), String> {
    update(|table| table.remove(key).is_some())
}

fn update(change: impl FnOnce(&mut toml::Table) -> bool) -> Result<(), String> {
    let file = path();
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    // Lock a stable sibling: the config inode is replaced by every successful write.
    let lock = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false)
        .open(file.with_extension("toml.lock")).map_err(|e| e.to_string())?;
    lock.lock().map_err(|e| e.to_string())?;
    let mut table = for_writing()?;
    if change(&mut table) { write(&table) } else { Ok(()) }
}

pub fn home(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => gtk::glib::home_dir().join(rest),
        None if path == "~" => gtk::glib::home_dir(),
        None => PathBuf::from(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_writers_preserve_every_key() {
        const CASE: &str = "prefs::tests::concurrent_writers_preserve_every_key";
        if std::env::var_os("DIGIT_PREFS_CONCURRENT_TEST").is_some() {
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(16));
            let threads: Vec<_> = (0..16).map(|i| {
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    for value in 0..8 {
                        set(&format!("key_{i}"), value.into()).unwrap();
                    }
                })
            }).collect();
            for thread in threads {
                thread.join().unwrap();
            }
            let table = read();
            for i in 0..16 {
                assert_eq!(table.get(&format!("key_{i}")).and_then(toml::Value::as_integer), Some(7));
            }
            unset("key_0").unwrap();
            assert!(!read().contains_key("key_0"));
            assert_eq!(read().len(), 15);
            // A failed rename must not leave a temporary file behind.
            std::fs::remove_file(path()).unwrap();
            std::fs::create_dir(path()).unwrap();
            assert!(write(&table).is_err());
            assert!(std::fs::read_dir(path().parent().unwrap()).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".new")));
            return;
        }
        let root = std::env::temp_dir().join(format!("digit-prefs-concurrent-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", CASE, "--nocapture"]).env("DIGIT_PREFS_CONCURRENT_TEST", "1").env("XDG_CONFIG_HOME", &root).output().unwrap();
        std::fs::remove_dir_all(root).unwrap();
        assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    }

    #[test]
    fn writes_preserve_unreadable_and_broken_settings() {
        const CASE: &str = "prefs::tests::writes_preserve_unreadable_and_broken_settings";
        if std::env::var_os("DIGIT_PREFS_TEST").is_some() {
            assert!(problem().is_none());
            set("answer", 42.into()).unwrap();
            assert_eq!(read().get("answer").and_then(toml::Value::as_integer), Some(42));
            assert!(problem().is_none());
            set_many(&[("hypr_tiling", "floating".into()), ("hypr_tiled_layout", "master".into())]).unwrap();
            let prefs = read();
            assert_eq!(text(&prefs, "hypr_tiling", ""), "floating");
            assert_eq!(text(&prefs, "hypr_tiled_layout", ""), "master");
            for body in [b"answer = 42\n\xff".as_slice(), b"answer = 42\nbroken = [".as_slice()] {
                std::fs::write(path(), body).unwrap();
                assert!(set("answer", 43.into()).is_err());
                assert!(unset("answer").is_err());
                assert!(set_many(&[("hypr_tiling", "dwindle".into()), ("hypr_tiled_layout", "scrolling".into())]).is_err());
                assert!(problem().is_some());
                if body.is_ascii() {
                    assert!(problem().unwrap().contains("line 2"));
                }
                assert_eq!(std::fs::read(path()).unwrap(), body);
                assert!(!path().with_extension("toml.new").exists());
            }
            std::fs::remove_file(path()).unwrap();
            std::fs::create_dir(path()).unwrap();
            assert!(problem().is_some());
            assert!(set("answer", 43.into()).is_err());
            return;
        }
        let root = std::env::temp_dir().join(format!("digit-prefs-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", CASE, "--nocapture"]).env("DIGIT_PREFS_TEST", "1").env("XDG_CONFIG_HOME", &root).output().unwrap();
        std::fs::remove_dir_all(root).unwrap();
        assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    }
}
