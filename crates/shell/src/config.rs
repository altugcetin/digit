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
    /// The compositor gaps Settings writes, read so the island keeps a minimum space above windows.
    pub hypr_gaps_in: f64,
    pub hypr_gaps_out: f64,
    pub bar_height: i32,
    pub bar_color: Option<String>,
    pub font: String,
    pub numbers: String,
    pub icon_font: String,
    pub clock: String,
    pub date: String,
    pub outputs: Vec<String>,
    /// The screen chosen as the main one in Settings, or "" (see `kit::screens::primary`).
    pub primary_monitor: String,
    pub glass: f64,
    pub scheme: String,
    pub surface: String,
    pub search_url: String,
    pub calendars: Vec<String>,
    /// Apps kept out of the app grid and Spotlight (see `kit::hidden`).
    pub apps_hidden: Vec<String>,
    pub apps_shown: Vec<String>,
    pub dock_mode: String,
    pub dock_side: String,
    pub dock_size: f64,
    pub dock_magnify: f64,
    pub dock_output: Option<String>,
    pub dock_running: bool,
    pub dock_trash: bool,
    pub dock_recent: bool,
    pub dock_trash_reveal: bool,
    pub dock_badges: bool,
    pub dock_icons: String,
    /// The Apps button at the start of the dock, which opens every app.
    pub dock_apps: bool,
    pub corner_top_left: String,
    pub corner_top_right: String,
    pub corner_bottom_left: String,
    pub corner_bottom_right: String,
    /// Screens with hot corners of their own (see `kit::corners::read_on`).
    pub monitor_corners: toml::Table,
    pub icon_theme: Option<String>,
    pub hover: bool,
    pub intro: bool,
    pub smooth_caret: bool,
    pub fullscreen_edges: bool,
    pub control_follows_button: bool,
    pub ui_round: f64,
    pub ui_surface: String,
    pub ui_accent: String,
    pub notch_round_follow: bool,
    pub notch_round: f64,
    pub notch_close: String,
    pub spaces_close: String,
    pub spaces_always: i64,
    pub control_close: String,
    pub calendar_close: String,
    pub notifications: bool,
    pub polkit: bool,
    pub wallpaper: bool,
    pub wallpaper_layout: String,
    pub wallpapers: std::collections::BTreeMap<String, String>,
    pub clipboard: bool,
    pub notch_style: String,
    pub menubar: bool,
    pub menubar_look: String,
    /// When the workspace pill shows beside the logo: "switch" (while switching) or "always".
    pub menubar_spaces: String,
    pub wide_height: f64,
    pub notch_call: String,
    pub call_apps: Vec<String>,
    /// Which media players the notch shows (see `kit::players`).
    pub media_filter: String,
    pub media_blocked: Vec<String>,
    pub media_allowed: Vec<String>,
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
    pub desktop_icons: bool,
    pub desktop_battery: String,
    /// The corner ("top_left"…) of the Now playing, Weather and System desktop widgets, or "off".
    pub desktop_media: String,
    pub desktop_weather: String,
    pub desktop_system: String,
    pub desktop_screentime: String,
    /// Widget places on screens other than the main one (see `kit::deskwidget`).
    pub desktop_screens: toml::Table,
    /// What each desktop widget shows: the clock's date and weather, the batteries' levels and
    /// the media controls; and whether each sits on a card of its own.
    pub desktop_clock_date: bool,
    pub desktop_clock_weather: bool,
    pub desktop_battery_levels: bool,
    pub desktop_media_controls: bool,
    pub desktop_clock_bg: bool,
    /// Draws the desktop's text through cairo (see `draw::rasterized`). Off unless asked for.
    pub desktop_raster: bool,
    /// The clock's digits roll and the widgets' values glide as they change. On unless turned off.
    pub desktop_motion: bool,
    /// Counts how long each app has the focus (see `screentime`). On unless turned off.
    pub screen_time: bool,
    pub desktop_battery_bg: bool,
    pub desktop_media_bg: bool,
    pub desktop_weather_bg: bool,
    pub desktop_system_bg: bool,
    pub desktop_screentime_bg: bool,
    /// Each widget's color (see `kit::deskwidget`): "auto", "white", "black" or "accent".
    pub desktop_clock_ink: String,
    pub desktop_battery_ink: String,
    pub desktop_media_ink: String,
    pub desktop_weather_ink: String,
    pub desktop_system_ink: String,
    pub desktop_screentime_ink: String,
    /// Each widget's size against as designed, by widget (see `kit::deskwidget::SIZES`).
    pub desktop_sizes: toml::Table,
    /// Each widget's font family and card tone, by widget (see `kit::deskwidget`).
    pub desktop_fonts: toml::Table,
    pub desktop_cards: toml::Table,
    /// Rounded screen corners: on/off, whether they follow the app roundness, and the radius
    /// used when they don't. Older configs only had the radius, where 0 meant off.
    pub screen_round: Option<bool>,
    pub screen_round_follow: Option<bool>,
    pub screen_corners: f64,
    pub tray: bool,
    pub mic_guard: bool,
}

/// The screen-corner radius at the default Roundness; Corners in Appearance scales it.
const SCREEN_CORNER: f64 = 12.0;

/// The radius the screen corners are drawn with, in pixels; 0 when they're off.
fn screen_radius(c: &Config) -> f64 {
    let legacy = c.screen_corners >= 2.0;
    if !c.screen_round.unwrap_or(legacy) {
        return 0.0;
    }
    // A radius set before the toggle existed keeps applying until the user picks otherwise.
    let follow = c.screen_round_follow.unwrap_or(c.screen_round.is_some() || !legacy);
    let r = if follow {
        SCREEN_CORNER * c.ui_round.clamp(0.1, 1.8)
    } else if c.screen_corners >= 4.0 {
        c.screen_corners
    } else {
        // Settings shows the default radius until the slider is first moved.
        SCREEN_CORNER
    };
    r.clamp(4.0, 40.0).round()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            dock: Dock::Screen,
            hypr_gaps_in: 4.0,
            hypr_gaps_out: 8.0,
            bar_height: 44,
            bar_color: None,
            font: "Readex Pro".into(),
            numbers: "Rubik".into(),
            icon_font: "Material Symbols Rounded".into(),
            clock: "%H:%M".into(),
            date: "%a %-d %b".into(),
            outputs: Vec::new(),
            primary_monitor: String::new(),
            glass: 0.0,
            scheme: "content".into(),
            surface: "black".into(),
            search_url: "https://duckduckgo.com/?q={}".into(),
            calendars: Vec::new(),
            apps_hidden: Vec::new(),
            apps_shown: Vec::new(),
            dock_mode: "on".into(),
            dock_side: "bottom".into(),
            dock_size: 50.0,
            dock_magnify: 1.5,
            dock_output: None,
            dock_running: true,
            dock_trash: true,
            dock_recent: true,
            dock_trash_reveal: true,
            dock_badges: true,
            dock_icons: "light".into(),
            dock_apps: true,
            corner_top_left: "workspaces".into(),
            corner_top_right: "none".into(),
            corner_bottom_left: "none".into(),
            corner_bottom_right: "control".into(),
            monitor_corners: toml::Table::new(),
            icon_theme: None,
            hover: true,
            intro: true,
            smooth_caret: true,
            fullscreen_edges: false,
            control_follows_button: true,
            ui_round: 1.0,
            ui_surface: "black".into(),
            ui_accent: "wallpaper".into(),
            notch_round_follow: true,
            notch_round: 1.0,
            notch_close: "leave".into(),
            spaces_close: "leave".into(),
            spaces_always: 0,
            control_close: "leave".into(),
            calendar_close: "leave".into(),
            notifications: true,
            polkit: true,
            wallpaper: true,
            wallpaper_layout: "same".into(),
            wallpapers: std::collections::BTreeMap::new(),
            clipboard: true,
            notch_style: "island".into(),
            menubar: true,
            menubar_look: "adaptive".into(),
            menubar_spaces: "switch".into(),
            wide_height: 34.0,
            notch_call: "voice".into(),
            call_apps: Vec::new(),
            media_filter: "off".into(),
            media_blocked: Vec::new(),
            media_allowed: Vec::new(),
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
            desktop_icons: true,
            desktop_battery: "off".into(),
            desktop_media: "off".into(),
            desktop_weather: "off".into(),
            desktop_system: "off".into(),
            desktop_screentime: "off".into(),
            desktop_screens: toml::Table::new(),
            desktop_clock_date: true,
            desktop_clock_weather: true,
            desktop_battery_levels: true,
            desktop_media_controls: false,
            desktop_clock_bg: false,
            desktop_raster: false,
            desktop_motion: true,
            screen_time: true,
            desktop_battery_bg: false,
            desktop_media_bg: false,
            desktop_weather_bg: false,
            desktop_system_bg: false,
            desktop_screentime_bg: false,
            desktop_clock_ink: "auto".into(),
            desktop_battery_ink: "auto".into(),
            desktop_media_ink: "auto".into(),
            desktop_weather_ink: "auto".into(),
            desktop_system_ink: "auto".into(),
            desktop_screentime_ink: "auto".into(),
            desktop_sizes: toml::Table::new(),
            desktop_fonts: toml::Table::new(),
            desktop_cards: toml::Table::new(),
            screen_round: None,
            screen_round_follow: None,
            screen_corners: 0.0,
            tray: true,
            mic_guard: true,
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
    pub trash: bool,
    pub recent: bool,
    pub trash_reveal: bool,
    pub badges: bool,
    pub icons: String,
    pub apps: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    pub dock: DockPrefs,
    pub surface: String,
    pub hover: bool,
    pub corners: [kit::corners::Panel; 4],
    pub primary_monitor: String,
    /// The screens whose hot corners are their own, by connector.
    pub monitor_corners: std::collections::BTreeMap<String, [kit::corners::Panel; 4]>,
    pub icon_theme: Option<String>,
    pub style: String,
    /// Extra space the shell reserves under the floating island when the compositor's own gap is
    /// smaller than the Tight preset's, so None and Tight leave windows the same distance below it.
    pub under: f64,
    /// The compositor's space between the screen edge and windows: Hyprland's outer gap, niri's
    /// single gap (twice the inner one).
    pub edge_gap: f64,
    pub menubar: bool,
    pub menubar_look: String,
    pub bar_spaces: bool,
    /// The workspace pill shows in the notch, in place of the clock, while switching.
    pub notch_spaces: bool,
    pub wide_height: f64,
    pub call: String,
    pub call_apps: Vec<String>,
    pub media: kit::players::Filter,
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
    pub desktop_icons: bool,
    pub desktop_battery: String,
    pub apps_hidden: Vec<String>,
    pub apps_shown: Vec<String>,
    pub desktop_media: String,
    pub desktop_weather: String,
    pub desktop_system: String,
    pub desktop_screentime: String,
    pub desktop_screens: toml::Table,
    /// What each desktop widget shows: the clock's date and weather, the batteries' levels and
    /// the media controls; and whether each sits on a card of its own.
    pub desktop_clock_date: bool,
    pub desktop_clock_weather: bool,
    pub desktop_battery_levels: bool,
    pub desktop_media_controls: bool,
    pub desktop_clock_bg: bool,
    /// Draws the desktop's text through cairo (see `draw::rasterized`). Off unless asked for.
    pub desktop_raster: bool,
    /// The clock's digits roll and the widgets' values glide as they change. On unless turned off.
    pub desktop_motion: bool,
    /// Counts how long each app has the focus (see `screentime`). On unless turned off.
    pub screen_time: bool,
    pub desktop_battery_bg: bool,
    pub desktop_media_bg: bool,
    pub desktop_weather_bg: bool,
    pub desktop_system_bg: bool,
    pub desktop_screentime_bg: bool,
    pub desktop_clock_ink: String,
    pub desktop_battery_ink: String,
    pub desktop_media_ink: String,
    pub desktop_weather_ink: String,
    pub desktop_system_ink: String,
    pub desktop_screentime_ink: String,
    pub desktop_sizes: toml::Table,
    pub desktop_fonts: toml::Table,
    pub desktop_cards: toml::Table,
    /// The screen-corner radius in pixels, already resolved from on/off and roundness; 0 is off.
    pub screen_corners: f64,
    pub tray: bool,
    pub mic_guard: bool,
    pub fullscreen_edges: bool,
    /// Control Center opens under the bar button that opened it, not in its hot corner.
    pub control_follows_button: bool,
    /// Spotlight draws a caret that glides between positions and blinks softly.
    pub smooth_caret: bool,
    /// Settings > Apps: corner roundness and card surface, shared with the shell's panels.
    pub ui_round: f64,
    pub ui_surface: String,
    pub ui_accent: String,
    /// The notch follows the app roundness unless told otherwise.
    pub notch_round_follow: bool,
    pub notch_round: f64,
    /// How each panel closes: "leave" when the pointer leaves it, "click" on a click elsewhere.
    pub notch_close: String,
    pub spaces_close: String,
    pub spaces_always: i64,
    pub control_close: String,
    pub calendar_close: String,
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
            trash: c.dock_trash,
            recent: c.dock_recent,
            trash_reveal: c.dock_trash_reveal,
            badges: c.dock_badges,
            icons: c.dock_icons.clone(),
            apps: c.dock_apps,
        };
        Self {
            dock,
            surface: c.surface.clone(),
            hover: c.hover,
            corners: kit::corners::read(&[
                ("corner_top_left".into(), toml::Value::String(c.corner_top_left.clone())),
                ("corner_top_right".into(), toml::Value::String(c.corner_top_right.clone())),
                ("corner_bottom_left".into(), toml::Value::String(c.corner_bottom_left.clone())),
                ("corner_bottom_right".into(), toml::Value::String(c.corner_bottom_right.clone())),
            ].into_iter().collect()),
            primary_monitor: c.primary_monitor.clone(),
            monitor_corners: kit::corners::monitors(&[(kit::corners::MONITORS.to_string(), toml::Value::Table(c.monitor_corners.clone()))].into_iter().collect()).into_iter().collect(),
            icon_theme: c.icon_theme.clone(),
            style: c.notch_style.clone(),
            under: under_island(c.hypr_gaps_in, c.hypr_gaps_out, kit::wm::which() == kit::wm::Wm::Niri),
            edge_gap: edge_gap(c.hypr_gaps_in, c.hypr_gaps_out, kit::wm::which() == kit::wm::Wm::Niri),
            menubar: c.menubar,
            menubar_look: c.menubar_look.clone(),
            bar_spaces: c.menubar_spaces == "always",
            notch_spaces: c.menubar_spaces == "notch",
            wide_height: c.wide_height,
            call: c.notch_call.clone(),
            call_apps: c.call_apps.clone(),
            media: kit::players::Filter::new(&c.media_filter, &c.media_blocked, &c.media_allowed),
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
            desktop_icons: c.desktop_icons,
            desktop_battery: c.desktop_battery.clone(),
            apps_hidden: c.apps_hidden.clone(),
            apps_shown: c.apps_shown.clone(),
            desktop_media: c.desktop_media.clone(),
            desktop_weather: c.desktop_weather.clone(),
            desktop_system: c.desktop_system.clone(),
            desktop_screentime: c.desktop_screentime.clone(),
            desktop_screens: c.desktop_screens.clone(),
            desktop_clock_date: c.desktop_clock_date,
            desktop_clock_weather: c.desktop_clock_weather,
            desktop_battery_levels: c.desktop_battery_levels,
            desktop_media_controls: c.desktop_media_controls,
            desktop_clock_bg: c.desktop_clock_bg,
            desktop_raster: c.desktop_raster,
            desktop_motion: c.desktop_motion,
            screen_time: c.screen_time,
            desktop_battery_bg: c.desktop_battery_bg,
            desktop_media_bg: c.desktop_media_bg,
            desktop_weather_bg: c.desktop_weather_bg,
            desktop_system_bg: c.desktop_system_bg,
            desktop_screentime_bg: c.desktop_screentime_bg,
            desktop_clock_ink: c.desktop_clock_ink.clone(),
            desktop_battery_ink: c.desktop_battery_ink.clone(),
            desktop_media_ink: c.desktop_media_ink.clone(),
            desktop_weather_ink: c.desktop_weather_ink.clone(),
            desktop_system_ink: c.desktop_system_ink.clone(),
            desktop_screentime_ink: c.desktop_screentime_ink.clone(),
            desktop_sizes: c.desktop_sizes.clone(),
            desktop_fonts: c.desktop_fonts.clone(),
            desktop_cards: c.desktop_cards.clone(),
            screen_corners: screen_radius(c),
            tray: c.tray,
            mic_guard: c.mic_guard,
            fullscreen_edges: c.fullscreen_edges,
            control_follows_button: c.control_follows_button,
            smooth_caret: c.smooth_caret,
            ui_round: c.ui_round,
            ui_surface: c.ui_surface.clone(),
            ui_accent: c.ui_accent.clone(),
            notch_round_follow: c.notch_round_follow,
            notch_round: c.notch_round,
            notch_close: c.notch_close.clone(),
            spaces_close: c.spaces_close.clone(),
            spaces_always: c.spaces_always.clamp(0, 10),
            control_close: c.control_close.clone(),
            calendar_close: c.calendar_close.clone(),
        }
    }
}

pub const VOICE: [&str; 24] = [
    "discord", "vesktop", "webcord", "legcord", "armcord", "equibop", "telegram", "signal", "zoom", "teams", "skype", "slack", "element", "whatsapp", "mumble", "teamspeak", "jitsi", "viber", "jami", "linphone", "dino", "fractal", "nheko", "revolt",
];

impl Prefs {
    /// Where each desktop widget sits on the screen `output` (see `kit::deskwidget::places`).
    pub fn places_on(&self, output: &str, primary: bool) -> [String; 6] {
        let main = [&self.desktop_widget, &self.desktop_battery, &self.desktop_media, &self.desktop_weather, &self.desktop_system, &self.desktop_screentime].map(|p| p.clone());
        kit::deskwidget::places_from(&main, &self.desktop_screens, output, primary)
    }

    /// How much larger or smaller than as designed `widget` is drawn (see `kit::deskwidget`).
    pub fn size_of(&self, widget: &str) -> f64 {
        kit::deskwidget::size_from(&self.desktop_sizes, widget)
    }

    /// The family `widget`'s text is drawn in, "" for the shell's own (see `kit::deskwidget::FONTS`).
    pub fn font_of(&self, widget: &str) -> String {
        kit::deskwidget::font_from(&self.desktop_fonts, widget)
    }

    /// How light `widget`'s card is drawn (see `kit::deskwidget::CARDS`).
    pub fn tone_of(&self, widget: &str) -> kit::deskwidget::Tone {
        kit::deskwidget::tone_from(&self.desktop_cards, widget)
    }

    /// The hot corners of the screen `output`: its own, or the shared ones.
    pub fn corners_on(&self, output: &str) -> [kit::corners::Panel; 4] {
        self.monitor_corners.get(output).copied().unwrap_or(self.corners)
    }

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

/// The smallest space between the floating island and windows: the Tight gap preset's.
pub const UNDER_ISLAND: f64 = 4.0;

/// What the shell adds under the island on top of the compositor's edge gap (Hyprland's outer
/// gap; niri's single gap, twice the inner one) so windows never come closer than `UNDER_ISLAND`.
pub fn under_island(gaps_in: f64, gaps_out: f64, niri: bool) -> f64 {
    let gap = if niri { gaps_in * 2.0 } else { gaps_out };
    if gap.is_finite() { (UNDER_ISLAND - gap.max(0.0)).max(0.0) } else { 0.0 }
}

/// How far the compositor keeps windows from the screen edge.
pub fn edge_gap(gaps_in: f64, gaps_out: f64, niri: bool) -> f64 {
    let gap = if niri { gaps_in * 2.0 } else { gaps_out };
    if gap.is_finite() { gap.max(0.0) } else { 0.0 }
}

pub fn dir() -> PathBuf {
    gtk::glib::user_config_dir().join("digit")
}

pub fn settings(page: &str, cfg: &Config) {
    let page = if page.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '/' | '-')) { page } else { "overview" };
    let argv: [&std::ffi::OsStr; 2] = ["digit-settings".as_ref(), page.as_ref()];
    if gtk::gio::Subprocess::newv(&argv, gtk::gio::SubprocessFlags::STDOUT_SILENCE | gtk::gio::SubprocessFlags::STDERR_SILENCE).is_err() {
        edit(cfg);
    }
}

pub fn edit(cfg: &Config) {
    use gtk::prelude::*;
    let path = dir().join("config.toml");
    if !path.exists() {
        if let Ok(text) = toml::to_string_pretty(cfg) {
            let _ = kit::atomic::write(&path, text);
        }
    }
    let uri = gtk::gio::File::for_path(&path).uri();
    let context = gtk::gio::AppLaunchContext::new();
    crate::apps::prepare(&context);
    let _ = gtk::gio::AppInfo::launch_default_for_uri(&uri, Some(&context));
}

thread_local! {
    static LAST: std::cell::RefCell<Config> = std::cell::RefCell::new(Config::default());
    /// The settings file as last loaded, for the settings `Config` has no field for.
    static TABLE: std::cell::RefCell<std::rc::Rc<toml::Table>> = std::cell::RefCell::new(std::rc::Rc::new(toml::Table::new()));
}

/// The settings as the shell last loaded them (see `load`), without reading the file again. Off
/// the main thread, where nothing is loaded, it reads the file.
pub fn table() -> std::rc::Rc<toml::Table> {
    if gtk::glib::MainContext::default().is_owner() {
        TABLE.with(|t| t.borrow().clone())
    } else {
        std::rc::Rc::new(kit::prefs::read())
    }
}

fn update(last: &mut Config, text: &str) -> Result<(), toml::de::Error> {
    *last = toml::from_str(text)?;
    Ok(())
}

pub fn load() -> Config {
    let path = dir().join("config.toml");
    LAST.with(|last| {
        let mut last = last.borrow_mut();
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Err(e) = update(&mut last, &text) {
                eprintln!("digit: {}: {e}", path.display());
            } else if let Ok(table) = text.parse::<toml::Table>() {
                TABLE.with(|t| *t.borrow_mut() = std::rc::Rc::new(table));
            }
        }
        last.clone()
    })
}

#[cfg(test)]
mod persistence_tests {
    use super::*;

    #[test]
    fn no_gaps_keep_the_tight_space_under_the_island() {
        assert_eq!(under_island(0.0, 0.0, false), 4.0);
        assert_eq!(under_island(2.0, 4.0, false), 0.0);
        assert_eq!(under_island(4.0, 8.0, false), 0.0);
        assert_eq!(under_island(0.0, 0.0, true), 4.0);
        assert_eq!(under_island(2.0, 4.0, true), 0.0);
        assert_eq!(under_island(0.0, f64::NAN, false), 0.0);
    }

    #[test]
    fn screen_corners_toggle_follow_roundness_or_keep_their_own_radius() {
        let radius = |text: &str| screen_radius(&toml::from_str::<Config>(text).unwrap());
        assert_eq!(radius(""), 0.0);
        assert_eq!(radius("screen_round = true"), 12.0);
        assert_eq!(radius("screen_round = true\nui_round = 1.6"), 19.0);
        assert_eq!(radius("screen_round = true\nui_round = 0.15"), 4.0);
        assert_eq!(radius("screen_round = true\nscreen_round_follow = false"), 12.0);
        assert_eq!(radius("screen_round = true\nscreen_round_follow = false\nscreen_corners = 18.0"), 18.0);
        assert_eq!(radius("screen_round = false\nscreen_corners = 18.0"), 0.0);
        // A radius from before the toggle keeps applying as an override.
        assert_eq!(radius("screen_corners = 10.0\nui_round = 1.6"), 10.0);
    }

    #[test]
    fn trash_and_recent_default_on_and_change_dock_preferences() {
        let mut config: Config = toml::from_str("").unwrap();
        let before = Prefs::from(&config).dock;
        assert!(config.dock_trash && config.dock_recent && before.trash && before.recent);
        update(&mut config, "dock_trash = false").unwrap();
        let trash_off = Prefs::from(&config).dock;
        assert!(!trash_off.trash && trash_off.recent);
        assert_ne!(before, trash_off);
        update(&mut config, "dock_recent = false").unwrap();
        let recent_off = Prefs::from(&config).dock;
        assert!(recent_off.trash && !recent_off.recent);
        assert_ne!(before, recent_off);
    }

    #[test]
    fn invalid_typed_value_keeps_previous_config() {
        let mut last = Config::default();
        update(&mut last, "idle_lock = 300.0\nfont = 'Saved font'").unwrap();
        let previous = toml::to_string(&last).unwrap();
        assert!(update(&mut last, "idle_lock = 'never'\nfont = 'Ignored font'").is_err());
        assert_eq!(toml::to_string(&last).unwrap(), previous);
        update(&mut last, "idle_lock = 600.0").unwrap();
        assert_eq!(last.idle_lock, 600.0);
    }

    #[test]
    fn set_preserves_broken_config() {
        if std::env::var_os("DIGIT_CONFIG_TEST").is_some() {
            assert_eq!(dir().join("config.toml"), kit::prefs::path());
            std::fs::create_dir_all(dir()).unwrap();
            for body in [b"dock_mode = [".as_slice(), b"dock_mode = \xff".as_slice()] {
                std::fs::write(kit::prefs::path(), body).unwrap();
                assert!(kit::prefs::set("dock_mode", "bar".into()).is_err());
                assert_eq!(std::fs::read(kit::prefs::path()).unwrap(), body);
            }
            return;
        }
        let root = std::env::temp_dir().join(format!("digit-config-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "config::persistence_tests::set_preserves_broken_config", "--nocapture"])
            .env("DIGIT_CONFIG_TEST", "1").env("XDG_CONFIG_HOME", &root).output().unwrap();
        std::fs::remove_dir_all(root).unwrap();
        assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    }
}
