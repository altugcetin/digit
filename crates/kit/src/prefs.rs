use std::path::PathBuf;

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
    let tmp = file.with_extension("toml.new");
    std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &file).map_err(|e| e.to_string())
}

pub fn set(key: &str, value: toml::Value) -> Result<(), String> {
    set_many(&[(key, value)])
}

/// Save related values together, preserving the old file if any part cannot be written.
pub fn set_many(values: &[(&str, toml::Value)]) -> Result<(), String> {
    let mut table = for_writing()?;
    let mut changed = false;
    for (key, value) in values {
        if table.get(*key) != Some(value) {
            table.insert((*key).into(), value.clone());
            changed = true;
        }
    }
    if changed { write(&table) } else { Ok(()) }
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
