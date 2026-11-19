use crate::art::Style;
use gtk::{gdk, gio, glib, prelude::*};

const BUNDLED: &[(&str, &[u8])] = &[("niri", include_bytes!("../../../data/icons/niri.svg"))];

pub fn install(display: &gdk::Display) {
    let root = glib::user_data_dir().join("digit/icons");
    for (name, bytes) in BUNDLED {
        let path = root.join(format!("{name}.svg"));
        if std::fs::read(&path).ok().as_deref() != Some(*bytes) {
            let _ = std::fs::create_dir_all(&root);
            let tmp = root.join(format!(".{name}.svg"));
            if std::fs::write(&tmp, bytes).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }
    gtk::IconTheme::for_display(display).add_search_path(&root);
}

pub fn theme(name: Option<&str>) {
    let Some(settings) = gtk::Settings::default() else { return };
    match name.filter(|n| !n.is_empty() && !n.chars().any(char::is_control)) {
        Some(n) => settings.set_gtk_icon_theme_name(Some(n)),
        None => settings.set_gtk_icon_theme_name(Some("Adwaita")),
    }
}

/// Styled icons use the original application artwork (Adwaita/hicolor), never
/// replacements from the desktop's selected icon pack.
pub fn source_theme(style: Style, active: &str) -> &str {
    if style == Style::Native { active } else { "Adwaita" }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DockKey {
    pub style: Style,
    pub theme: String,
    pub px: i32,
    pub scale: i32,
}

impl DockKey {
    pub fn new(style: Style, active: &str, px: i32, scale: i32) -> Self {
        Self { style, theme: source_theme(style, active).to_owned(), px, scale }
    }
}

thread_local! {
    /// The theme styled icons come from, made once: each new one reads its theme's directories
    /// and caches into memory again on the first lookup.
    static STYLED: std::cell::RefCell<Option<(String, gtk::IconTheme)>> = const { std::cell::RefCell::new(None) };
}

pub fn dock_theme(display: &gdk::Display, key: &DockKey) -> gtk::IconTheme {
    let active = gtk::IconTheme::for_display(display);
    if key.style == Style::Native {
        return active;
    }
    STYLED.with(|slot| {
        if let Some((name, theme)) = slot.borrow().as_ref()
            && *name == key.theme
        {
            return theme.clone();
        }
        let theme = gtk::IconTheme::new();
        let paths = active.search_path();
        theme.set_search_path(&paths.iter().map(|p| p.as_path()).collect::<Vec<_>>());
        let paths = active.resource_path();
        theme.set_resource_path(&paths.iter().map(|p| p.as_str()).collect::<Vec<_>>());
        theme.set_theme_name(Some(&key.theme));
        *slot.borrow_mut() = Some((key.theme.clone(), theme.clone()));
        theme
    })
}

pub fn dock_icon(theme: &gtk::IconTheme, icon: Option<&gio::Icon>, app_id: &str, key: &DockKey) -> Option<gdk::Paintable> {
    let paintable = match icon {
        Some(icon) => theme.lookup_by_gicon(icon, key.px, key.scale, gtk::TextDirection::Ltr, gtk::IconLookupFlags::empty()),
        None if !app_id.is_empty() && theme.has_icon(app_id) => theme.lookup_icon(app_id, &[], key.px, key.scale, gtk::TextDirection::Ltr, gtk::IconLookupFlags::empty()),
        // Steam installs a game's icon under its number, not under its window class.
        None => {
            let name = kit::gamescope::steam_id(app_id).map(|n| format!("steam_icon_{n}")).filter(|n| theme.has_icon(n))?;
            theme.lookup_icon(&name, &[], key.px, key.scale, gtk::TextDirection::Ltr, gtk::IconLookupFlags::empty())
        }
    };
    Some(paintable.upcast())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_original_uses_the_selected_icon_pack() {
        for active in ["Adwaita", "Papirus-Dark", "Tela-circle-dark"] {
            assert_eq!(source_theme(Style::Native, active), active);
            for preference in ["light", "graphite", "tile", "mono", "tinted"] {
                assert_eq!(source_theme(Style::parse(preference), active), "Adwaita");
            }
            assert_eq!(source_theme(Style::parse("native"), active), active);
        }
    }

    #[test]
    fn dock_cache_tracks_style_source_size_and_scale() {
        let key = DockKey::new(Style::Light, "Adwaita", 64, 1);
        assert_eq!(key, DockKey::new(Style::Light, "Papirus", 64, 1));
        assert_ne!(key, DockKey::new(Style::Graphite, "Adwaita", 64, 1));
        assert_ne!(key, DockKey::new(Style::Light, "Adwaita", 96, 1));
        assert_ne!(key, DockKey::new(Style::Light, "Adwaita", 64, 2));
        assert_ne!(DockKey::new(Style::Native, "Adwaita", 64, 1), DockKey::new(Style::Native, "Papirus", 64, 1));
    }
}
