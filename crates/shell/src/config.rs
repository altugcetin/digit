use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Deserialize, Serialize, Clone, Copy, Debug, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Dock {
    Bar,
    #[default]
    Screen,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    pub dock: Dock,
    pub bar_height: i32,
    pub bar_color: Option<String>,
    pub font: String,
    pub numbers: String,
    pub icon_font: String,
    pub clock: String,
    pub date: String,
    pub outputs: Vec<String>,
    pub glass: f64,
    pub scheme: String,
    pub tint: f64,
    pub surface: String,
    pub search_url: String,
    pub calendars: Vec<String>,
    pub dock_mode: String,
    pub dock_side: String,
    pub dock_size: f64,
    pub dock_magnify: f64,
    pub dock_output: Option<String>,
    pub dock_running: bool,
    pub dock_badges: bool,
    pub dock_icons: String,
    pub hot_corner: bool,
    pub icon_theme: Option<String>,
    pub hover: bool,
    pub notifications: bool,
    pub polkit: bool,
    pub wallpaper: bool,
    pub clipboard: bool,
    pub notch_style: String,
    pub menubar: bool,
    pub menubar_look: String,
    pub wide_height: f64,
    pub notch_call: String,
    pub call_apps: Vec<String>,
    pub idle_lock: f64,
    pub idle_off: f64,
    pub idle_sleep: f64,
    pub lock_on_sleep: bool,
    pub night_light: String,
    pub night_temp: f64,
    pub night_from: String,
    pub night_to: String,
    pub weather_city: String,
    pub weather_unit: String,
    pub desktop_widget: String,
    pub screen_corners: f64,
    pub tray: bool,
    pub mic_guard: bool,
    pub control_corner: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            dock: Dock::Screen,
            bar_height: 44,
            bar_color: None,
            font: "Readex Pro".into(),
            numbers: "Rubik".into(),
            icon_font: "Material Symbols Rounded".into(),
            clock: "%H:%M".into(),
            date: "%a %-d %b".into(),
            outputs: Vec::new(),
            glass: 0.0,
            scheme: "content".into(),
            tint: 0.16,
            surface: "black".into(),
            search_url: "https://duckduckgo.com/?q={}".into(),
            calendars: Vec::new(),
            dock_mode: "on".into(),
            dock_side: "bottom".into(),
            dock_size: 50.0,
            dock_magnify: 1.5,
            dock_output: None,
            dock_running: true,
            dock_badges: true,
            dock_icons: "light".into(),
            hot_corner: true,
            icon_theme: None,
            hover: true,
            notifications: true,
            polkit: true,
            wallpaper: true,
            clipboard: true,
            notch_style: "island".into(),
            menubar: true,
            menubar_look: "adaptive".into(),
            wide_height: 34.0,
            notch_call: "voice".into(),
            call_apps: Vec::new(),
            idle_lock: 0.0,
            idle_off: 10.0,
            idle_sleep: 0.0,
            lock_on_sleep: true,
            night_light: "off".into(),
            night_temp: 4200.0,
            night_from: "20:00".into(),
            night_to: "07:00".into(),
            weather_city: String::new(),
            weather_unit: "celsius".into(),
            desktop_widget: "off".into(),
            screen_corners: 0.0,
            tray: true,
            mic_guard: true,
            control_corner: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DockPrefs {
    pub mode: String,
    pub side: String,
    pub size: f64,
    pub magnify: f64,
    pub output: Option<String>,
    pub running: bool,
    pub badges: bool,
    pub icons: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    pub dock: DockPrefs,
    pub surface: String,
    pub hover: bool,
    pub hot_corner: bool,
    pub icon_theme: Option<String>,
    pub style: String,
    pub menubar: bool,
    pub menubar_look: String,
    pub wide_height: f64,
    pub call: String,
    pub call_apps: Vec<String>,
    pub idle_lock: f64,
    pub idle_off: f64,
    pub idle_sleep: f64,
    pub lock_on_sleep: bool,
    pub night_light: String,
    pub night_temp: f64,
    pub night_from: String,
    pub night_to: String,
    pub weather_city: String,
    pub weather_unit: String,
    pub desktop_widget: String,
    pub screen_corners: f64,
    pub tray: bool,
    pub mic_guard: bool,
    pub control_corner: bool,
}

impl From<&Config> for Prefs {
    fn from(c: &Config) -> Self {
        let dock = DockPrefs {
            mode: c.dock_mode.clone(),
            side: c.dock_side.clone(),
            size: c.dock_size,
            magnify: c.dock_magnify,
            output: c.dock_output.clone(),
            running: c.dock_running,
            badges: c.dock_badges,
            icons: c.dock_icons.clone(),
        };
        Self {
            dock,
            surface: c.surface.clone(),
            hover: c.hover,
            hot_corner: c.hot_corner,
            icon_theme: c.icon_theme.clone(),
            style: c.notch_style.clone(),
            menubar: c.menubar,
            menubar_look: c.menubar_look.clone(),
            wide_height: c.wide_height,
            call: c.notch_call.clone(),
            call_apps: c.call_apps.clone(),
            idle_lock: c.idle_lock,
            idle_off: c.idle_off,
            idle_sleep: c.idle_sleep,
            lock_on_sleep: c.lock_on_sleep,
            night_light: c.night_light.clone(),
            night_temp: c.night_temp,
            night_from: c.night_from.clone(),
            night_to: c.night_to.clone(),
            weather_city: c.weather_city.clone(),
            weather_unit: c.weather_unit.clone(),
            desktop_widget: c.desktop_widget.clone(),
            screen_corners: c.screen_corners,
            tray: c.tray,
            mic_guard: c.mic_guard,
            control_corner: c.control_corner,
        }
    }
}

pub const VOICE: [&str; 24] = [
    "discord", "vesktop", "webcord", "legcord", "armcord", "equibop", "telegram", "signal", "zoom", "teams", "skype", "slack", "element", "whatsapp", "mumble", "teamspeak", "jitsi", "viber", "jami", "linphone", "dino", "fractal", "nheko", "revolt",
];

impl Prefs {
    pub fn wide(&self, cfg: &Config) -> bool {
        cfg.dock == Dock::Screen && self.style == "bar"
    }

    pub fn voice(&self, who: &str) -> bool {
        let who = who.to_lowercase();
        if who.is_empty() {
            return false;
        }
        if !self.call_apps.is_empty() {
            return self.call_apps.iter().any(|a| !a.is_empty() && who.contains(&a.to_lowercase()));
        }
        VOICE.iter().any(|a| who.contains(a))
    }

    pub fn calls(&self, who: &str) -> bool {
        let who = who.to_lowercase();
        match self.call.as_str() {
            "off" => false,
            "any" => true,
            _ if !self.call_apps.is_empty() => self.call_apps.iter().any(|a| !a.is_empty() && who.contains(&a.to_lowercase())),
            _ => VOICE.iter().any(|a| who.contains(a)),
        }
    }
}

impl Config {
    pub fn header(&self) -> f64 {
        match self.dock {
            Dock::Bar => self.bar_height as f64,
            Dock::Screen => 34.0,
        }
    }

    pub fn reserve(&self) -> i32 {
        match self.dock {
            Dock::Bar => -1,
            Dock::Screen => 38,
        }
    }
}

pub fn dir() -> PathBuf {
    gtk::glib::user_config_dir().join("digit")
}

pub fn settings(page: &str, cfg: &Config) {
    let page = if page.chars().all(|c| c.is_ascii_lowercase()) { page } else { "overview" };
    let argv: [&std::ffi::OsStr; 2] = ["digit-settings".as_ref(), page.as_ref()];
    if gtk::gio::Subprocess::newv(&argv, gtk::gio::SubprocessFlags::STDOUT_SILENCE | gtk::gio::SubprocessFlags::STDERR_SILENCE).is_err() {
        edit(cfg);
    }
}

pub fn edit(cfg: &Config) {
    use gtk::prelude::*;
    let path = dir().join("config.toml");
    if !path.exists() {
        let _ = std::fs::create_dir_all(dir());
        if let Ok(text) = toml::to_string_pretty(cfg) {
            let _ = std::fs::write(&path, text);
        }
    }
    let uri = gtk::gio::File::for_path(&path).uri();
    let context = gtk::gio::AppLaunchContext::new();
    crate::apps::prepare(&context);
    let _ = gtk::gio::AppInfo::launch_default_for_uri(&uri, Some(&context));
}

pub fn set(key: &str, value: toml::Value) -> Result<(), String> {
    let path = dir().join("config.toml");
    let mut table: toml::Table = std::fs::read_to_string(&path).ok().and_then(|t| t.parse().ok()).unwrap_or_default();
    if table.get(key) == Some(&value) {
        return Ok(());
    }
    table.insert(key.to_string(), value);
    let text = toml::to_string_pretty(&table).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("toml.new");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

pub fn load() -> Config {
    let path = dir().join("config.toml");
    match std::fs::read_to_string(&path) {
        Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
            eprintln!("digit: {}: {e}", path.display());
            Config::default()
        }),
        Err(_) => Config::default(),
    }
}
