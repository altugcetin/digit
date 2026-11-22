//! The desktop widgets' settings that Settings writes and the shell reads: where each sits on
//! each screen, and the color of its text and marks.
//!
//! The main screen keeps the widgets' places under their own keys (`desktop_widget` for the
//! clock, `desktop_media`…). Any other screen can have places of its own, in the
//! [`SCREENS`] table by connector; one without them shows the clock where the main screen has
//! it, and nothing else, as before there were places per screen.
//!
//! The size: how much larger or smaller than as designed each widget is drawn, the same on every
//! screen, in the [`SIZES`] table by widget; one without a size is drawn as designed.
//!
//! The color: Automatic follows the wallpaper behind, light over dark and dark over light; the
//! others hold whatever is behind.
//!
//! The card: a widget with a background draws it light or dark as its color asks, or as set in
//! the [`CARDS`] table by widget. The font: each widget's text in a family of its own, in the
//! [`FONTS`] table by widget, or the shell's own when it has none.

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
    all.insert(output.to_string(), screen_value(places));
    toml::Value::Table(all)
}

fn screen_value(places: &[String; 6]) -> toml::Value {
    toml::Value::Table(WIDGETS.iter().zip(places).map(|(w, p)| (w.to_string(), toml::Value::String(p.clone()))).collect())
}

/// Gives the screen `output` places of its own, the other screens kept.
pub fn save_screen(output: &str, places: &[String; 6]) -> Result<(), String> {
    crate::prefs::update_table(SCREENS, |all| {
        all.insert(output.to_string(), screen_value(places));
    })
}

/// The table of the widgets' sizes, by widget.
pub const SIZES: &str = "desktop_sizes";

/// The smallest and the largest a widget can be drawn, against as designed.
pub const SIZE_RANGE: (f64, f64) = (0.6, 1.6);

/// `k` within [`SIZE_RANGE`], in steps of 5%; as designed when it isn't a number.
pub fn clamp_size(k: f64) -> f64 {
    if !k.is_finite() {
        return 1.0;
    }
    (k.clamp(SIZE_RANGE.0, SIZE_RANGE.1) * 20.0).round() / 20.0
}

/// `widget`'s size in the [`SIZES`] table `sizes`.
pub fn size_from(sizes: &toml::Table, widget: &str) -> f64 {
    match sizes.get(widget) {
        Some(toml::Value::Float(k)) => clamp_size(*k),
        Some(toml::Value::Integer(k)) => clamp_size(*k as f64),
        _ => 1.0,
    }
}

/// [`size_from`], read from the preferences.
pub fn size(prefs: &toml::Table, widget: &str) -> f64 {
    prefs.get(SIZES).and_then(toml::Value::as_table).map_or(1.0, |t| size_from(t, widget))
}

/// Sets `widget`'s size, the others kept; as designed takes it out of the table.
pub fn save_size(widget: &str, k: f64) -> Result<(), String> {
    let k = clamp_size(k);
    crate::prefs::update_table(SIZES, |all| {
        if k == 1.0 {
            all.remove(widget);
        } else {
            all.insert(widget.to_string(), toml::Value::Float(k));
        }
    })
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

/// The table of the widgets' font families, by widget; one without is drawn in the shell's own
/// fonts (Readex Pro, and Rubik for numbers).
pub const FONTS: &str = "desktop_fonts";

/// `widget`'s family in the [`FONTS`] table `fonts`, "" for the shell's own.
pub fn font_from(fonts: &toml::Table, widget: &str) -> String {
    fonts.get(widget).and_then(toml::Value::as_str).unwrap_or("").trim().to_string()
}

/// [`font_from`], read from the preferences.
pub fn font(prefs: &toml::Table, widget: &str) -> String {
    prefs.get(FONTS).and_then(toml::Value::as_table).map_or_else(String::new, |t| font_from(t, widget))
}

/// Sets `widget`'s family, the others kept; "" takes it out of the table.
pub fn save_font(widget: &str, family: &str) -> Result<(), String> {
    let family = family.trim();
    crate::prefs::update_table(FONTS, |all| {
        if family.is_empty() {
            all.remove(widget);
        } else {
            all.insert(widget.to_string(), toml::Value::String(family.to_string()));
        }
    })
}

/// The table of the widgets' card tones, by widget.
pub const CARDS: &str = "desktop_cards";

/// Each card tone as stored, and as Settings names it.
pub const CARD_TONES: [(&str, &str); 3] = [("auto", "Automatic"), ("light", "Light"), ("dark", "Dark")];

/// How light a widget's card is: Automatic follows the widget's color, light under dark text
/// and dark under light text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Card {
    #[default]
    Auto,
    Light,
    Dark,
}

impl Card {
    pub fn parse(value: &str) -> Self {
        match value {
            "light" => Card::Light,
            "dark" => Card::Dark,
            _ => Card::Auto,
        }
    }
}

/// `widget`'s card tone in the [`CARDS`] table `cards`.
pub fn card_from(cards: &toml::Table, widget: &str) -> Card {
    Card::parse(cards.get(widget).and_then(toml::Value::as_str).unwrap_or("auto"))
}

/// [`card_from`], read from the preferences.
pub fn card(prefs: &toml::Table, widget: &str) -> Card {
    prefs.get(CARDS).and_then(toml::Value::as_table).map_or(Card::Auto, |t| card_from(t, widget))
}

/// Sets `widget`'s card tone, the others kept; Automatic takes it out of the table.
pub fn save_card(widget: &str, tone: &str) -> Result<(), String> {
    let tone = Card::parse(tone);
    crate::prefs::update_table(CARDS, |all| match tone {
        Card::Auto => {
            all.remove(widget);
        }
        Card::Light => {
            all.insert(widget.to_string(), toml::Value::String("light".into()));
        }
        Card::Dark => {
            all.insert(widget.to_string(), toml::Value::String("dark".into()));
        }
    })
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

    #[test]
    fn card_tones_and_the_font_read_back_and_default_to_automatic() {
        let prefs: toml::Table = "[desktop_fonts]\nclock = ' Inter '\nmedia = 4\n[desktop_cards]\nclock = 'light'\nmedia = 'dark'\nweather = 'pink'\nsystem = 3".parse().unwrap();
        assert_eq!(card(&prefs, "clock"), Card::Light);
        assert_eq!(card(&prefs, "media"), Card::Dark);
        assert_eq!(card(&prefs, "weather"), Card::Auto);
        assert_eq!(card(&prefs, "system"), Card::Auto);
        assert_eq!(card(&prefs, "battery"), Card::Auto);
        assert_eq!(card(&toml::Table::new(), "clock"), Card::Auto);
        assert!(CARD_TONES.iter().all(|(v, _)| *v == "auto" || Card::parse(v) != Card::Auto));
        assert_eq!(font(&prefs, "clock"), "Inter");
        assert_eq!(font(&prefs, "media"), "");
        assert_eq!(font(&prefs, "weather"), "");
        assert_eq!(font(&toml::Table::new(), "clock"), "");
    }

    #[test]
    fn sizes_stay_in_range_in_steps_and_default_to_as_designed() {
        let prefs: toml::Table = "[desktop_sizes]\nclock = 1.23\nmedia = 9\nweather = 'big'\nsystem = 0.1".parse().unwrap();
        assert_eq!(size(&prefs, "clock"), 1.25);
        assert_eq!(size(&prefs, "media"), SIZE_RANGE.1);
        assert_eq!(size(&prefs, "weather"), 1.0);
        assert_eq!(size(&prefs, "system"), SIZE_RANGE.0);
        assert_eq!(size(&prefs, "battery"), 1.0);
        assert_eq!(size(&toml::Table::new(), "clock"), 1.0);
        assert_eq!(clamp_size(f64::NAN), 1.0);
    }
}
