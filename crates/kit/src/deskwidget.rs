//! The color of a desktop widget's text and marks. Automatic follows the wallpaper behind it,
//! light over dark and dark over light; the others hold whatever is behind.

/// The widgets that take a color, by the name their setting goes under.
pub const WIDGETS: [&str; 6] = ["clock", "battery", "media", "weather", "system", "screentime"];

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
    fn colors_read_back_and_unknown_ones_are_automatic() {
        let prefs: toml::Table = "desktop_clock_ink = 'accent'\ndesktop_media_ink = 'pink'".parse().unwrap();
        assert_eq!(read(&prefs, "clock"), Ink::Accent);
        assert_eq!(read(&prefs, "media"), Ink::Auto);
        assert_eq!(read(&prefs, "weather"), Ink::Auto);
        assert!(CHOICES.iter().all(|(v, _)| *v == "auto" || Ink::parse(v) != Ink::Auto));
        assert_eq!(key("screentime"), "desktop_screentime_ink");
    }
}
