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

pub fn set(key: &str, value: toml::Value) -> Result<(), String> {
    set_many(&[(key, value)])
}

/// Save related controls together, so a preset never leaves half of its values behind.
pub fn set_many(values: &[(&str, toml::Value)]) -> Result<(), String> {
    let table = read();
    if values.iter().any(|(key, value)| table.get(*key) != Some(value)) {
        mark();
    }
    kit::prefs::set_many(values)
}

pub fn unset(key: &str) -> Result<(), String> {
    if read().contains_key(key) {
        mark();
    }
    kit::prefs::unset(key)
}
