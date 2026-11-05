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
        Err(_) => Ok(toml::Table::new()),
    }
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
    let mut table = for_writing()?;
    if table.get(key) == Some(&value) {
        return Ok(());
    }
    table.insert(key.to_string(), value);
    write(&table)
}

pub fn home(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => gtk::glib::home_dir().join(rest),
        None if path == "~" => gtk::glib::home_dir(),
        None => PathBuf::from(path),
    }
}
