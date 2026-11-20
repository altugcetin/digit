//! The desktop widgets' settings that Settings writes and the shell reads: where each sits on
//! each screen, and the color of its text and marks.
//!
//! The main screen keeps the widgets' places under their own keys (`desktop_widget` for the
//! clock, `desktop_media`…). Any other screen can have places of its own, in the
//! [`SCREENS`] table by connector; one without them shows the clock where the main screen has
//! it, and nothing else, as before there were places per screen.
//!
//! The color: Automatic follows the wallpaper behind, light over dark and dark over light; the
//! others hold whatever is behind.

/// The widgets, by the name their settings go under, in the order places are given in.
pub const WIDGETS: [&str; 6] = ["clock", "battery", "media", "weather", "system", "screentime"];

/// The table of the places on screens other than the main one.
pub const SCREENS: &str = "desktop_screens";

/// The setting that holds `widget`'s place on the main screen.
pub fn place_key(widget: &str) -> &'static str {
    match widget {
        "clock" => "desktop_widget",
        "battery" => "desktop_battery",
        "media" => "desktop_media",
        "weather" => "desktop_weather",
        "system" => "desktop_system",
        _ => "desktop_screentime",
    }
}

/// Where each widget sits on the screen `output` ("off" where it doesn't), in [`WIDGETS`] order,
/// from the main screen's places `main` and the [`SCREENS`] table `screens`.
pub fn places_from(main: &[String; 6], screens: &toml::Table, output: &str, primary: bool) -> [String; 6] {
    if primary {
        return main.clone();
    }
    match screens.get(output).and_then(toml::Value::as_table) {
        Some(own) => WIDGETS.map(|w| own.get(w).and_then(toml::Value::as_str).unwrap_or("off").to_string()),
        None => std::array::from_fn(|i| if i == 0 { main[0].clone() } else { "off".into() }),
    }
}

/// [`places_from`], read from the preferences.
pub fn places(prefs: &toml::Table, output: &str, primary: bool) -> [String; 6] {
    let main = WIDGETS.map(|w| crate::prefs::text(prefs, place_key(w), "off"));
    let screens = prefs.get(SCREENS).and_then(toml::Value::as_table).cloned().unwrap_or_default();
    places_from(&main, &screens, output, primary)
}

/// The value of [`SCREENS`] once the screen `output` has `places`, the other screens kept.
pub fn with_screen(prefs: &toml::Table, output: &str, places: &[String; 6]) -> toml::Value {
    let mut all = prefs.get(SCREENS).and_then(toml::Value::as_table).cloned().unwrap_or_default();
    all.insert(output.to_string(), toml::Value::Table(WIDGETS.iter().zip(places).map(|(w, p)| (w.to_string(), toml::Value::String(p.clone()))).collect()));
    toml::Value::Table(all)
}

/// Each choice as stored, and as Settings names it.
pub const CHOICES: [(&str, &str); 4] = [("auto", "Automatic"), ("white", "White"), ("black", "Black"), ("accent", "Accent")];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Ink {
    #[default]
    Auto,
    White,
    Black,
    Accent,
}

impl Ink {
    pub fn parse(value: &str) -> Self {
        match value {
            "white" => Ink::White,
            "black" => Ink::Black,
            "accent" => Ink::Accent,
            _ => Ink::Auto,
        }
    }
}

/// The setting that holds `widget`'s color ("clock" → `desktop_clock_ink`).
pub fn key(widget: &str) -> String {
    format!("desktop_{widget}_ink")
}

/// `widget`'s color as set, Automatic when it isn't.
pub fn read(prefs: &toml::Table, widget: &str) -> Ink {
    Ink::parse(&crate::prefs::text(prefs, &key(widget), "auto"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_screen_has_its_places_and_one_without_shows_the_clock_alone() {
        let prefs: toml::Table = "desktop_widget = 'left'\ndesktop_media = 'top_right'".parse().unwrap();
        let main = places(&prefs, "DP-2", true);
        assert_eq!(main[0], "left");
        assert_eq!(main[2], "top_right");
        assert_eq!(main[1], "off");
        // Another screen without places of its own: the clock as on the main one, nothing else.
        let other = places(&prefs, "HDMI-A-1", false);
        assert_eq!(other[0], "left");
        assert!(other[1..].iter().all(|p| p == "off"));
        // Given its own, it keeps them, and the main screen's stay as they were.
        let mut own = other.clone();
        own[0] = "off".into();
        own[4] = "bottom_left".into();
        let mut prefs = prefs;
        prefs.insert(SCREENS.into(), with_screen(&prefs, "HDMI-A-1", &own));
        assert_eq!(places(&prefs, "HDMI-A-1", false), own);
        assert_eq!(places(&prefs, "DP-2", true), main);
        assert_eq!(place_key("screentime"), "desktop_screentime");
    }

    #[test]
    fn colors_read_back_and_unknown_ones_are_automatic() {
        let prefs: toml::Table = "desktop_clock_ink = 'accent'\ndesktop_media_ink = 'pink'".parse().unwrap();
        assert_eq!(read(&prefs, "clock"), Ink::Accent);
        assert_eq!(read(&prefs, "media"), Ink::Auto);
        assert_eq!(read(&prefs, "weather"), Ink::Auto);
        assert!(CHOICES.iter().all(|(v, _)| *v == "auto" || Ink::parse(v) != Ink::Auto));
        assert_eq!(key("screentime"), "desktop_screentime_ink");
    }
}
