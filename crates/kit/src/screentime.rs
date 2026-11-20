//! Screen time: how long each app had the focus on each day. The shell adds to it while someone
//! is at the computer, Settings reads and clears it. It stays in a file in the state folder, on
//! this computer only.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// Days kept; older ones are dropped when the file is written.
pub const DAYS: usize = 60;
/// Pref that turns the counting off; on by default.
pub const KEY: &str = "screen_time";

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Use {
    pub secs: u64,
    pub name: String,
    /// Seconds in each hour of the day (24 entries), once any were counted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hours: Vec<u64>,
}

/// Day (`2026-11-19`) → app id → time.
pub type Log = BTreeMap<String, BTreeMap<String, Use>>;

pub fn file() -> PathBuf {
    crate::theme::state_dir().join("screentime.json")
}

pub fn today() -> String {
    gtk::glib::DateTime::now_local().ok().and_then(|d| d.format("%F").ok()).map(|s| s.to_string()).unwrap_or_default()
}

pub fn read() -> Log {
    std::fs::read_to_string(file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// Writes the log through a temporary file, so a crash never leaves half of it.
pub fn write(log: &Log) -> std::io::Result<()> {
    let mut log = log.clone();
    while log.len() > DAYS {
        log.pop_first();
    }
    crate::atomic::write(&file(), serde_json::to_string(&log).unwrap_or_default())
}

pub fn clear() {
    let _ = std::fs::remove_file(file());
}

/// Adds `secs` to `app` on `day`, in `hour` (0–23) of it.
pub fn add(log: &mut Log, day: &str, app: &str, name: &str, hour: usize, secs: u64) {
    let e = log.entry(day.to_string()).or_default().entry(app.to_string()).or_default();
    e.secs += secs;
    e.hours.resize(24, 0);
    e.hours[hour.min(23)] += secs;
    if e.name != name && !name.is_empty() {
        e.name = name.to_string();
    }
}

/// One day's total.
pub fn total(day: &BTreeMap<String, Use>) -> u64 {
    day.values().map(|u| u.secs).sum()
}

/// The apps over the given days, longest first, with their total time.
pub fn top<'a>(days: impl IntoIterator<Item = &'a BTreeMap<String, Use>>) -> Vec<(String, Use)> {
    let mut sum: BTreeMap<String, Use> = BTreeMap::new();
    for day in days {
        for (id, u) in day {
            let e = sum.entry(id.clone()).or_default();
            e.secs += u.secs;
            if !u.name.is_empty() {
                e.name = u.name.clone();
            }
        }
    }
    let mut list: Vec<_> = sum.into_iter().collect();
    list.sort_by(|a, b| b.1.secs.cmp(&a.1.secs).then(a.0.cmp(&b.0)));
    list
}

/// "2 h 5 min", "14 min" or "under a minute".
pub fn span(secs: u64) -> String {
    let m = secs / 60;
    match (m / 60, m % 60) {
        (0, 0) => "under a minute".into(),
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_adds_up_per_day_and_app() {
        let mut log = Log::new();
        add(&mut log, "2026-11-19", "firefox", "Firefox", 9, 600);
        add(&mut log, "2026-11-19", "firefox", "Firefox", 9, 60);
        add(&mut log, "2026-11-19", "kitty", "Kitty", 10, 90);
        add(&mut log, "2026-11-18", "kitty", "Kitty", 10, 30);
        assert_eq!(log["2026-11-19"]["firefox"].secs, 660);
        assert_eq!(total(&log["2026-11-19"]), 750);
        let top = top(log.values());
        assert_eq!(top[0].0, "firefox");
        assert_eq!(top[1].1.secs, 120);
        assert_eq!(log["2026-11-19"]["firefox"].hours[9], 660);
        assert_eq!(log["2026-11-19"]["kitty"].hours.iter().sum::<u64>(), 90);
    }

    #[test]
    fn spans_read_plainly() {
        assert_eq!(span(20), "under a minute");
        assert_eq!(span(14 * 60), "14 min");
        assert_eq!(span(2 * 3600), "2 h");
        assert_eq!(span(2 * 3600 + 5 * 60), "2 h 5 min");
    }
}
