use crate::app::App;
use gtk::{gio, glib};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub struct Weather {
    pub temp: f64,
    pub hi: f64,
    pub lo: f64,
    pub code: i64,
    pub day: bool,
    pub place: String,
    pub unit: char,
}

thread_local! {
    static PLACE: RefCell<Option<(String, f64, f64, String)>> = const { RefCell::new(None) };
    static GEN: Cell<u64> = const { Cell::new(0) };
}

pub fn glyph(code: i64, day: bool) -> &'static str {
    match code {
        0 => if day { "sunny" } else { "clear_night" },
        1 | 2 => if day { "partly_cloudy_day" } else { "partly_cloudy_night" },
        3 => "cloud",
        45 | 48 => "foggy",
        51..=67 | 80..=82 => "rainy",
        71..=77 | 85 | 86 => "weather_snowy",
        95..=99 => "thunderstorm",
        _ => "cloud",
    }
}

pub fn describe(code: i64) -> &'static str {
    match code {
        0 => "Clear",
        1 => "Mostly clear",
        2 => "Partly cloudy",
        3 => "Cloudy",
        45 | 48 => "Fog",
        51..=57 => "Drizzle",
        61..=67 => "Rain",
        71..=77 => "Snow",
        80..=82 => "Showers",
        85 | 86 => "Snow showers",
        95..=99 => "Thunderstorm",
        _ => "",
    }
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.') { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

async fn fetch(url: &str) -> Option<Value> {
    let argv = ["curl", "-fsS", "--max-time", "10", "--proto", "=https", "--max-filesize", "200000", url].map(std::ffi::OsStr::new);
    let p = gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_SILENCE).ok()?;
    let (out, _) = p.communicate_utf8_future(None).await.ok()?;
    serde_json::from_str(out?.as_str()).ok()
}

async fn locate(city: &str) -> Option<(f64, f64, String)> {
    let v = fetch(&format!("https://geocoding-api.open-meteo.com/v1/search?name={}&count=1&format=json", encode(city))).await?;
    let r = v.get("results")?.as_array()?.first()?;
    Some((r.get("latitude")?.as_f64()?, r.get("longitude")?.as_f64()?, r.get("name")?.as_str()?.chars().take(40).collect()))
}

fn minutes(iso: &str) -> Option<f64> {
    let t = iso.split('T').nth(1)?;
    let (h, m) = t.split_once(':')?;
    Some(h.parse::<f64>().ok()? * 60.0 + m.get(..2)?.parse::<f64>().ok()?)
}

async fn refresh(app: Rc<App>, generation: u64) -> bool {
    let current = || GEN.with(Cell::get) == generation;
    let (city, unit) = {
        let p = app.prefs.borrow();
        (p.weather_city.trim().to_string(), p.weather_unit.clone())
    };
    if !current() { return false; }
    if city.is_empty() {
        *app.weather.borrow_mut() = None;
        return true;
    }
    let known = PLACE.with(|p| p.borrow().clone()).filter(|p| p.0 == city);
    let (lat, lon, place) = match known {
        Some((_, lat, lon, place)) => (lat, lon, place),
        None => {
            let Some((lat, lon, place)) = locate(&city).await else { return false };
            if !current() { return false; }
            PLACE.with(|p| *p.borrow_mut() = Some((city.clone(), lat, lon, place.clone())));
            (lat, lon, place)
        }
    };
    let f = unit == "fahrenheit";
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat:.3}&longitude={lon:.3}&current=temperature_2m,weather_code,is_day&daily=temperature_2m_max,temperature_2m_min,sunrise,sunset&timezone=auto&forecast_days=1{}",
        if f { "&temperature_unit=fahrenheit" } else { "" }
    );
    let Some(v) = fetch(&url).await else { return false };
    let cur = v.get("current");
    let daily = v.get("daily");
    let first = |k: &str| daily.and_then(|d| d.get(k)).and_then(|a| a.as_array()).and_then(|a| a.first().cloned());
    let Some(temp) = cur.and_then(|c| c.get("temperature_2m")).and_then(Value::as_f64) else { return false };
    let w = Weather {
        temp,
        hi: first("temperature_2m_max").and_then(|v| v.as_f64()).unwrap_or(temp),
        lo: first("temperature_2m_min").and_then(|v| v.as_f64()).unwrap_or(temp),
        code: cur.and_then(|c| c.get("weather_code")).and_then(Value::as_i64).unwrap_or(3),
        day: cur.and_then(|c| c.get("is_day")).and_then(Value::as_i64).unwrap_or(1) == 1,
        place,
        unit: if f { 'F' } else { 'C' },
    };
    if !current() { return false; }
    if let (Some(rise), Some(set)) = (first("sunrise").and_then(|v| v.as_str().and_then(minutes)), first("sunset").and_then(|v| v.as_str().and_then(minutes))) {
        crate::night::set_sun(rise, set);
        crate::night::apply(&app);
    }
    if !current() { return false; }
    if app.weather.borrow().as_ref() != Some(&w) {
        *app.weather.borrow_mut() = Some(w);
        let control = app.control.borrow().clone();
        if let Some(c) = control {
            c.changed();
        }
        if current() {
            crate::desk::changed(&app);
            crate::deskcards::changed();
        }
    }
    true
}

pub fn start(app: &Rc<App>) {
    let generation = GEN.with(|g| {
        g.set(g.get() + 1);
        g.get()
    });
    let weak: Weak<App> = Rc::downgrade(app);
    glib::spawn_future_local(async move {
        loop {
            if GEN.with(Cell::get) != generation {
                return;
            }
            let Some(app) = weak.upgrade() else { return };
            if app.prefs.borrow().weather_city.trim().is_empty() {
                *app.weather.borrow_mut() = None;
                crate::desk::changed(&app);
                return;
            }
            let ok = refresh(app, generation).await;
            glib::timeout_future(Duration::from_secs(if ok { 1800 } else { 300 })).await;
        }
    });
}
