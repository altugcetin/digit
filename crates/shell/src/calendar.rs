use crate::app::App;
use gtk::{gio, glib, prelude::*};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

#[derive(Clone)]
pub struct Day {
    pub start: i64,
    pub number: i32,
    pub letter: String,
    pub weekday: i32,
    pub today: bool,
}

#[derive(Clone)]
pub struct Event {
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
    pub title: String,
}

thread_local! {
    static EVENTS: RefCell<Vec<Event>> = const { RefCell::new(Vec::new()) };
    static LOADED: RefCell<i64> = const { RefCell::new(i64::MIN) };
}

pub fn month(day: &Day) -> String {
    glib::DateTime::from_unix_local(day.start).ok().and_then(|d| d.format("%b").ok()).map(|s| s.to_string()).unwrap_or_default()
}

pub fn week(shift: i32) -> Vec<Day> {
    let Ok(now) = glib::DateTime::now_local() else { return Vec::new() };
    let Ok(midnight) = glib::DateTime::from_local(now.year(), now.month(), now.day_of_month(), 0, 0, 0.0) else { return Vec::new() };
    (-3..=3)
        .filter_map(|i| {
            let d = midnight.add_days(i + shift).ok()?;
            Some(Day {
                start: d.to_unix(),
                number: d.day_of_month(),
                letter: d.format("%a").ok()?.chars().next()?.to_uppercase().collect(),
                weekday: d.day_of_week(),
                today: i + shift == 0,
            })
        })
        .collect()
}

pub fn label(day: &Day) -> String {
    glib::DateTime::from_unix_local(day.start).ok().and_then(|d| d.format("%a").ok()).map(|s| s.to_uppercase()).unwrap_or_default()
}

pub fn name(day: &Day) -> String {
    glib::DateTime::from_unix_local(day.start).ok().and_then(|d| d.format("%A").ok()).map(|s| s.to_string()).unwrap_or_default()
}

pub fn on(day: &Day) -> Vec<Event> {
    let end = glib::DateTime::from_unix_local(day.start).ok().and_then(|d| d.add_days(1).ok()).map_or(day.start + 86_400, |d| d.to_unix());
    let mut list: Vec<Event> = EVENTS.with(|e| e.borrow().iter().filter(|ev| ev.start < end && ev.end > day.start).cloned().collect());
    list.sort_by_key(|e| (!e.all_day, e.start));
    list
}

pub fn clock(at: i64) -> String {
    glib::DateTime::from_unix_local(at).ok().and_then(|d| d.format("%H:%M").ok()).map(|s| s.to_string()).unwrap_or_default()
}

pub fn refresh(app: &Rc<App>) {
    let now = glib::monotonic_time();
    let stale = LOADED.with(|l| now - *l.borrow() > 15 * 60 * 1_000_000);
    if !stale || app.cfg.calendars.is_empty() {
        return;
    }
    LOADED.with(|l| *l.borrow_mut() = now);
    let sources = app.cfg.calendars.clone();
    let weak = Rc::downgrade(app);
    glib::spawn_future_local(async move {
        let mut all = Vec::new();
        for source in sources {
            let text = if source.starts_with("http://") || source.starts_with("https://") || source.starts_with("webcal://") {
                fetch(&source.replacen("webcal://", "https://", 1)).await
            } else {
                let path = if let Some(rest) = source.strip_prefix("~/") { glib::home_dir().join(rest) } else { source.clone().into() };
                gio::File::for_path(path).load_contents_future().await.ok().map(|(b, _)| String::from_utf8_lossy(&b).into_owned())
            };
            if let Some(text) = text {
                all.extend(gio::spawn_blocking(move || parse(&text)).await.unwrap_or_default());
            }
        }
        EVENTS.with(|e| *e.borrow_mut() = all);
        if let Some(app) = weak.upgrade() {
            app.redraw();
        }
    });
}

async fn fetch(url: &str) -> Option<String> {
    let process = crate::child::spawn(
        &["curl".as_ref(), "-fsSL".as_ref(), "--max-time".as_ref(), "20".as_ref(), url.as_ref()],
        gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_SILENCE,
    )
    .ok()?;
    let timeout = glib::timeout_future(Duration::from_secs(25));
    let read = process.communicate_utf8_future(None);
    futures_util::pin_mut!(timeout, read);
    match futures_util::future::select(read, timeout).await {
        futures_util::future::Either::Left((Ok((out, _)), _)) => out.map(|s| s.to_string()),
        _ => None,
    }
}

fn stamp(value: &str, utc: bool, zone: &glib::TimeZone) -> Option<(i64, bool)> {
    let digits: String = value.chars().filter(|c| c.is_ascii_digit()).collect();
    let num = |a: usize, b: usize| digits.get(a..b)?.parse::<i32>().ok();
    let (y, m, d) = (num(0, 4)?, num(4, 6)?, num(6, 8)?);
    if digits.len() < 14 {
        return Some((glib::DateTime::new(zone, y, m, d, 0, 0, 0.0).ok()?.to_unix(), true));
    }
    let (h, mi, s) = (num(8, 10)?, num(10, 12)?, num(12, 14)?);
    let tz = if utc { glib::TimeZone::utc() } else { zone.clone() };
    Some((glib::DateTime::new(&tz, y, m, d, h, mi, s as f64).ok()?.to_unix(), false))
}

fn parse(text: &str) -> Vec<Event> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.lines() {
        if let (Some(rest), Some(last)) = (raw.strip_prefix([' ', '\t']), lines.last_mut()) {
            last.push_str(rest.trim_end_matches('\r'));
        } else {
            lines.push(raw.trim_end_matches('\r').to_string());
        }
    }
    let zone = glib::TimeZone::local();
    let mut out = Vec::new();
    let mut current: Option<(Option<(i64, bool)>, Option<(i64, bool)>, String)> = None;
    for line in lines {
        if line == "BEGIN:VEVENT" {
            current = Some((None, None, String::new()));
            continue;
        }
        if line == "END:VEVENT" {
            if let Some((Some((start, all_day)), end, title)) = current.take() {
                let end = end.map_or(if all_day { start + 86_400 } else { start + 3600 }, |e| e.0).max(start + 1);
                out.push(Event { start, end, all_day, title: title.replace("\\,", ",").replace("\\;", ";").replace("\\n", " ") });
            }
            continue;
        }
        let Some(event) = current.as_mut() else { continue };
        let Some((key, value)) = line.split_once(':') else { continue };
        let name = key.split(';').next().unwrap_or(key);
        let utc = value.ends_with('Z');
        match name {
            "DTSTART" => event.0 = stamp(value, utc, &zone),
            "DTEND" => event.1 = stamp(value, utc, &zone),
            "SUMMARY" => event.2 = value.to_string(),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn reads_timed_and_all_day_events() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nDTSTART:20261027T090000Z\r\nDTEND:20261027T100000Z\r\nSUMMARY:Stand\r\n up\\, daily\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nDTSTART;VALUE=DATE:20261028\r\nSUMMARY:Holiday\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let events = parse(ics);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].title, "Standup, daily");
        assert_eq!(events[0].end - events[0].start, 3600);
        assert!(events[1].all_day);
        assert_eq!(events[1].end - events[1].start, 86_400);
    }
}
