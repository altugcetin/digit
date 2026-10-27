use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize, Clone, Copy, Debug, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Dock {
    Bar,
    #[default]
    Screen,
}

#[derive(Deserialize, Clone, Debug)]
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
    pub hover: bool,
    pub takeover: bool,
    pub notifications: bool,
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
            glass: 0.2,
            hover: true,
            takeover: true,
            notifications: true,
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
