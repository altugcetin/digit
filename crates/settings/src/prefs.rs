use std::cell::Cell;
use std::path::PathBuf;
use std::time::Instant;

thread_local! {
    static WROTE: Cell<Option<Instant>> = const { Cell::new(None) };
}

pub fn ours() -> bool {
    WROTE.with(|w| w.get().is_some_and(|t| t.elapsed().as_millis() < 1500))
}

fn mark() {
    WROTE.with(|w| w.set(Some(Instant::now())));
}

pub fn path() -> PathBuf {
    gtk::glib::user_config_dir().join("digit/config.toml")
}

pub fn read() -> toml::Table {
    std::fs::read_to_string(path()).ok().and_then(|t| t.parse::<toml::Table>().ok()).unwrap_or_default()
}

pub fn get_str(t: &toml::Table, key: &str, fallback: &str) -> String {
    t.get(key).and_then(|v| v.as_str()).unwrap_or(fallback).to_string()
}

pub fn get_bool(t: &toml::Table, key: &str, fallback: bool) -> bool {
    t.get(key).and_then(|v| v.as_bool()).unwrap_or(fallback)
}

pub fn get_f64(t: &toml::Table, key: &str, fallback: f64) -> f64 {
    t.get(key).and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64))).unwrap_or(fallback)
}

/// The settings file for changing it; an error when it exists but doesn't parse, so a typo
/// never gets every setting overwritten.
fn for_writing() -> Result<toml::Table, String> {
    match std::fs::read_to_string(path()) {
        Ok(text) => text.parse::<toml::Table>().map_err(|e| format!("{} has an error, so nothing was saved: {e}", path().display())),
        Err(_) => Ok(toml::Table::new()),
    }
}

pub fn set(key: &str, value: toml::Value) -> Result<(), String> {
    let mut table = for_writing()?;
    if table.get(key) == Some(&value) {
        return Ok(());
    }
    table.insert(key.to_string(), value);
    let text = toml::to_string_pretty(&table).map_err(|e| e.to_string())?;
    let file = path();
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = file.with_extension("toml.new");
    mark();
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &file).map_err(|e| e.to_string())
}

pub fn unset(key: &str) -> Result<(), String> {
    let mut table = for_writing()?;
    if table.remove(key).is_none() {
        return Ok(());
    }
    let text = toml::to_string_pretty(&table).map_err(|e| e.to_string())?;
    let file = path();
    let tmp = file.with_extension("toml.new");
    mark();
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &file).map_err(|e| e.to_string())
}
