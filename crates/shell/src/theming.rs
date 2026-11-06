use gtk::glib;
use kit::theme::{self, Theme};
use std::path::PathBuf;

const IMPORT: &str = "@import url(\"digit.css\");";

fn gtk_dirs() -> Vec<PathBuf> {
    ["gtk-4.0", "gtk-3.0"].iter().map(|v| glib::user_config_dir().join(v)).collect()
}

fn qt_conf() -> PathBuf {
    glib::user_config_dir().join("qt6ct/qt6ct.conf")
}

fn qt_colors() -> PathBuf {
    glib::user_config_dir().join("qt6ct/colors/digit.conf")
}

fn saved_qt_path() -> PathBuf {
    theme::state_dir().join("qt6ct-color-scheme-path")
}

fn accent_for(t: &Theme) -> String {
    kit::style::look().accent.or_else(|| t.roles.get("primary").cloned()).filter(|c| kit::style::custom_accent(c).is_some()).unwrap_or_else(|| "#3daee9".into())
}

fn gtk_css_for(t: &Theme, gtk4: bool, look: &kit::style::Look) -> String {
    let accent = look.accent.clone().unwrap_or_else(|| t.hex("primary"));
    let on_accent = kit::style::custom_accent(&accent).map(|(_, on)| on.to_owned()).unwrap_or_else(|| t.hex("on_primary"));
    let [base, card, raised, pop, field] = kit::style::surfaces(look, &accent);
    let c = |k: &str| t.hex(k);
    let pairs = [
        ("accent_color", accent.clone()),
        ("accent_bg_color", accent.clone()),
        ("accent_fg_color", on_accent.clone()),
        ("destructive_bg_color", c("error")),
        ("destructive_fg_color", c("on_error")),
        ("destructive_color", c("error")),
        ("error_bg_color", c("error")),
        ("error_fg_color", c("on_error")),
        ("error_color", c("error")),
        ("success_bg_color", c("tertiary")),
        ("success_fg_color", c("on_tertiary")),
        ("warning_bg_color", c("secondary")),
        ("warning_fg_color", c("on_secondary")),
        ("window_bg_color", base.clone()),
        ("window_fg_color", c("on_surface")),
        ("view_bg_color", base.clone()),
        ("view_fg_color", c("on_surface")),
        ("headerbar_bg_color", card.clone()),
        ("headerbar_fg_color", c("on_surface")),
        ("headerbar_border_color", c("outline_variant")),
        ("headerbar_shade_color", c("shadow")),
        ("popover_bg_color", pop.clone()),
        ("popover_fg_color", c("on_surface")),
        ("card_bg_color", card.clone()),
        ("card_fg_color", c("on_surface")),
        ("dialog_bg_color", pop.clone()),
        ("dialog_fg_color", c("on_surface")),
        ("overview_bg_color", c("surface_container")),
        ("overview_fg_color", c("on_surface")),
        ("sidebar_bg_color", card.clone()),
        ("sidebar_fg_color", c("on_surface")),
        ("secondary_sidebar_bg_color", base.clone()),
        ("secondary_sidebar_fg_color", c("on_surface")),
        ("thumbnail_bg_color", raised.clone()),
        ("entry_bg_color", field.clone()),
        ("thumbnail_fg_color", c("on_surface")),
        ("theme_bg_color", base.clone()),
        ("theme_fg_color", c("on_surface")),
        ("theme_base_color", base.clone()),
        ("theme_text_color", c("on_surface")),
        ("theme_selected_bg_color", accent.clone()),
        ("theme_selected_fg_color", on_accent.clone()),
        ("insensitive_fg_color", c("outline")),
        ("borders", c("outline_variant")),
    ];
    let mut out = String::new();
    for (k, v) in &pairs {
        out.push_str(&format!("@define-color {k} {v};\n"));
    }
    for (k, v) in [("headerbar_backdrop_color", "@window_bg_color"), ("sidebar_backdrop_color", "@window_bg_color"), ("sidebar_border_color", "@window_bg_color")] {
        out.push_str(&format!("@define-color {k} {v};\n"));
    }
    if gtk4 {
        out.push_str("\n:root {\n");
        for (k, v) in &pairs {
            if !k.starts_with("theme_") && *k != "borders" {
                out.push_str(&format!("    --{}: {v};\n", k.replace('_', "-")));
            }
        }
        out.push_str("}\n");
    }
    let r = niri_radius();
    out.push_str(&format!(
        "\nwindow.csd:not(.maximized):not(.fullscreen),\nwindow.csd:not(.maximized):not(.fullscreen) > decoration {{\n    border-radius: {r}px;\n}}\n"
    ));
    out
}

fn gtk_css(t: &Theme, gtk4: bool) -> String {
    gtk_css_for(t, gtk4, &kit::style::look())
}

fn niri_radius() -> u32 {
    let path = kit::niriconf::dir().join("digit-decorations.kdl");
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| {
            text.lines()
                .map(str::trim)
                .filter(|l| l.starts_with("geometry-corner-radius"))
                .find_map(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok())
        })
        .map_or(12, |r| r.round().clamp(0.0, 64.0) as u32)
}

fn color_line(source: &str, section: &str, key: &str) -> Option<String> {
    let mut current = "";
    for line in source.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') { current = line; }
        else if current == section && line.starts_with(&format!("{key}=")) { return line.split_once('=') .map(|(_, value)| value.trim().to_owned()); }
    }
    None
}

fn theme_color(t: &Theme, key: &str, fallback: &str) -> String {
    t.roles.get(key).and_then(|value| kit::palette::Rgb::hex(value)).map(|_| t.hex(key)).unwrap_or_else(|| fallback.to_owned())
}

fn qt_scheme(t: &Theme, breeze: Option<&str>) -> String {
    let accent = accent_for(t);
    let on_accent = kit::style::custom_accent(&accent).map(|(_, on)| on.to_owned()).unwrap_or_else(|| t.hex("on_primary"));
    let color = |section: &str, key: &str, palette_key: &str, fallback: &str| {
        breeze.and_then(|source| color_line(source, section, key)).unwrap_or_else(|| theme_color(t, palette_key, fallback))
    };
    let roles = [
        color("[Colors:Window]", "ForegroundNormal", "on_surface", "#eeeeee"),
        color("[Colors:Button]", "BackgroundNormal", "surface_container_high", "#292c30"),
        theme_color(t, "surface_container_high", "#3d4044"),
        theme_color(t, "surface_container", "#33363a"),
        theme_color(t, "surface_container_lowest", "#0a0c0e"),
        theme_color(t, "surface_container_low", "#1c1e20"),
        color("[Colors:View]", "ForegroundNormal", "on_surface", "#eeeeee"),
        theme_color(t, "on_error", "#ffffff"),
        color("[Colors:Button]", "BackgroundNormal", "surface_container_high", "#292c30"),
        color("[Colors:Button]", "ForegroundNormal", "on_surface", "#eeeeee"),
        color("[Colors:View]", "BackgroundNormal", "surface", "#141618"),
        color("[Colors:Window]", "BackgroundNormal", "surface_container_low", "#202326"),
        "0,0,0".into(),
        color("[Colors:Selection]", "BackgroundNormal", "primary", "#3daee9"),
        color("[Colors:Selection]", "ForegroundNormal", "on_primary", "#ffffff"),
        color("[Colors:View]", "ForegroundLink", "primary", "#3daee9"),
        theme_color(t, "secondary", "#9b59b6"),
        color("[Colors:View]", "BackgroundAlternate", "surface_container_low", "#1d1f22"),
        color("[Colors:Tooltip]", "BackgroundNormal", "surface_container_high", "#292c30"),
        color("[Colors:Tooltip]", "ForegroundNormal", "on_surface", "#eeeeee"),
        color("[Colors:View]", "ForegroundInactive", "outline", "#a1a9b1"),
        accent.clone(),
    ];
    let mut roles = roles;
    roles[12] = accent;
    roles[13] = on_accent;
    let row = roles.join(", ");
    format!("[ColorScheme]\nactive_colors={row}\ndisabled_colors={row}\ninactive_colors={row}\n")
}

fn breeze_scheme() -> Option<String> {
    let data_dirs = std::env::var_os("XDG_DATA_DIRS")
        .map(|v| std::env::split_paths(&v).collect::<Vec<_>>())
        .unwrap_or_else(|| vec![PathBuf::from("/usr/local/share"), PathBuf::from("/usr/share")]);
    std::iter::once(glib::user_data_dir().join("color-schemes/BreezeDark.colors"))
        .chain(data_dirs.into_iter().map(|dir| dir.join("color-schemes/BreezeDark.colors")))
        .find_map(|path| std::fs::read_to_string(path).ok())
}

fn theme_name_in(roots: Vec<PathBuf>) -> Option<&'static str> {
    let available = |name: &str| roots.iter().any(|root| root.join(name).join("gtk-3.0/gtk.css").is_file());
    if available("adw-gtk3-dark") { Some("adw-gtk3-dark") }
    else if available("adw-gtk3") { Some("adw-gtk3") }
    else { None }
}

fn theme_name() -> Option<(&'static str, Vec<PathBuf>)> {
    let data_home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| glib::home_dir().join(".local/share"));
    let data_dirs = std::env::var_os("XDG_DATA_DIRS").map(|v| std::env::split_paths(&v).collect::<Vec<_>>()).unwrap_or_else(|| vec![PathBuf::from("/usr/local/share"), PathBuf::from("/usr/share")]);
    let roots = std::iter::once(data_home.join("themes")).chain(data_dirs.into_iter().map(|p| p.join("themes"))).collect::<Vec<_>>();
    Some((theme_name_in(roots.clone())?, roots))
}

fn settings_ini(old: &str, theme: &str) -> String {
    let mut lines: Vec<String> = old.lines().map(str::to_owned).collect();
    let start = lines.iter().position(|line| line.trim() == "[Settings]");
    let at = if let Some(start) = start {
        lines.iter().enumerate().skip(start + 1).find(|(_, line)| line.trim().starts_with('[')).map(|(i, _)| i).unwrap_or(lines.len())
    } else { lines.push("[Settings]".into()); lines.len() };
    for (key, value) in [("gtk-theme-name", theme), ("gtk-application-prefer-dark-theme", "1")] {
        if let Some(i) = lines.iter().enumerate().skip(start.unwrap_or(at.saturating_sub(1)) + 1).take_while(|(_, line)| !line.trim().starts_with('[')).find_map(|(i, line)| line.trim().starts_with(&format!("{key}=")).then_some(i)) {
            lines[i] = format!("{key}={value}");
        } else { lines.insert(at, format!("{key}={value}")); }
    }
    format!("{}\n", lines.join("\n"))
}

fn qt_config(old: &str) -> String {
    fn set_appearance(old: &str, values: &[(&str, &str)]) -> String {
        let mut lines: Vec<String> = old.lines().map(str::to_owned).collect();
        let start = lines.iter().position(|line| line.trim() == "[Appearance]");
        let end = start.and_then(|s| lines.iter().enumerate().skip(s + 1).find(|(_, line)| line.trim().starts_with('[')).map(|(i, _)| i)).unwrap_or(lines.len());
        let at = if start.is_some() { end } else { lines.push("[Appearance]".into()); lines.len() };
        for (key, value) in values {
            let search_from = start.map_or(at, |s| s + 1);
            let found = lines.iter().enumerate().skip(search_from).take_while(|(_, line)| !line.trim().starts_with('[')).find_map(|(i, line)| line.trim().starts_with(&format!("{key}=")).then_some(i));
            if let Some(i) = found { lines[i] = format!("{key}={value}"); } else { lines.insert(at, format!("{key}={value}")); }
        }
        format!("{}\n", lines.join("\n"))
    }
    let path = qt_colors().to_string_lossy().into_owned();
    set_appearance(old, &[("style", "Breeze"), ("icon_theme", "Adwaita"), ("color_scheme_path", &path)])
}

fn apply_gtk_settings() {
    let Some((theme, _)) = theme_name() else { return };
    for dir in gtk_dirs().into_iter().filter(|d| d.is_dir()) {
        let file = dir.join("settings.ini");
        let old = std::fs::read_to_string(&file).unwrap_or_default();
        write_if_changed(&file, &settings_ini(&old, theme));
    }
    for (key, value) in [("color-scheme", "prefer-dark"), ("gtk-theme", theme)] {
        if std::process::Command::new("gsettings").args(["writable", "org.gnome.desktop.interface", key]).output().is_ok_and(|out| out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "true") {
            let _ = std::process::Command::new("gsettings").args(["set", "org.gnome.desktop.interface", key, value]).output();
        }
    }
}

fn ini_value<'a>(text: &'a str, section: &str, key: &str) -> Option<&'a str> {
    let mut current = "";
    text.lines().find_map(|line| {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') { current = line; None }
        else if current == section { line.strip_prefix(&format!("{key}=")) }
        else { None }
    })
}

fn write_if_changed(path: &PathBuf, text: &str) {
    if std::fs::read_to_string(path).ok().as_deref() != Some(text) {
        let tmp = path.with_extension("new");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(tmp, path);
        }
    }
}

pub fn write(t: &Theme) {
    for (dir, gtk4) in gtk_dirs().into_iter().zip([true, false]) {
        if dir.is_dir() {
            write_if_changed(&dir.join("digit.css"), &gtk_css(t, gtk4));
        }
    }
    if let Some(parent) = qt_colors().parent() {
        let _ = std::fs::create_dir_all(parent);
        write_if_changed(&qt_colors(), &qt_scheme(t, breeze_scheme().as_deref()));
    }
    apply_gtk_settings();
    // The compositor's window border follows the accent, which can come from the wallpaper.
    if kit::setup::installed() {
        std::thread::spawn(|| {
            let _ = kit::setup::apply();
        });
    }
}

pub fn take() {
    apply_gtk_settings();
    for dir in gtk_dirs().into_iter().filter(|d| d.is_dir()) {
        let file = dir.join("gtk.css");
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        if !text.lines().any(|l| l.trim() == IMPORT) {
            let joined = if text.is_empty() || text.ends_with('\n') { format!("{text}{IMPORT}\n") } else { format!("{text}\n{IMPORT}\n") };
            write_if_changed(&file, &joined);
        }
    }
    let conf = qt_conf();
    let _ = std::fs::create_dir_all(conf.parent().unwrap_or_else(|| std::path::Path::new(".")));
    let _ = std::fs::create_dir_all(qt_colors().parent().unwrap_or_else(|| std::path::Path::new(".")));
    let theme = theme::load().unwrap_or_default();
    write_if_changed(&qt_colors(), &qt_scheme(&theme, breeze_scheme().as_deref()));
    let text = std::fs::read_to_string(&conf).unwrap_or_default();
    let ours = qt_colors().to_string_lossy().into_owned();
    let current = ini_value(&text, "[Appearance]", "color_scheme_path").unwrap_or("");
    if current != ours {
        let _ = std::fs::create_dir_all(theme::state_dir());
        let _ = std::fs::write(saved_qt_path(), current);
    }
    let generated = qt_config(&text);
    write_if_changed(&conf, &generated);
}

pub fn give_back() {
    for dir in gtk_dirs().into_iter().filter(|d| d.is_dir()) {
        let file = dir.join("gtk.css");
        let Ok(text) = std::fs::read_to_string(&file) else { continue };
        if text.lines().any(|l| l.trim() == IMPORT) {
            let kept: Vec<&str> = text.lines().filter(|l| l.trim() != IMPORT).collect();
            let mut joined = kept.join("\n");
            while joined.ends_with('\n') {
                joined.pop();
            }
            write_if_changed(&file, &format!("{joined}\n"));
        }
    }
    give_back_qt();
}

fn give_back_qt() {
    let Ok(original) = std::fs::read_to_string(saved_qt_path()) else { return };
    let conf = qt_conf();
    if let Ok(text) = std::fs::read_to_string(&conf) {
        let ours = qt_colors().to_string_lossy().into_owned();
        write_if_changed(&conf, &qt_config(&text).replace(&format!("color_scheme_path={ours}"), &format!("color_scheme_path={}", original.trim())));
    }
    let _ = std::fs::remove_file(saved_qt_path());
}

pub fn refresh(chosen: Option<&str>, fallback: &str, image: Option<String>, force: bool) -> Option<Theme> {
    let current = theme::load();
    let scheme = chosen.map(str::to_string).or_else(|| current.as_ref().map(|t| t.scheme.clone())).unwrap_or_else(|| fallback.to_string());
    let scheme = scheme.as_str();
    let image = image.or_else(crate::wallpaper::current).or_else(crate::wallpaper::found).or_else(|| current.as_ref().map(|t| t.wallpaper.clone()))?;
    let fresh = match &current {
        Some(t) if !force && t.wallpaper == image && t.scheme == scheme => t.clone(),
        _ => {
            let t = theme::generate(std::path::Path::new(&image), scheme)?;
            theme::save(&t).ok()?;
            t
        }
    };
    write(&fresh);
    Some(fresh)
}

pub fn restyle() {
    if let Some(theme) = theme::load() { write(&theme); }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme() -> Theme {
        Theme { roles: [
            ("primary".into(), "#7c83ff".into()), ("on_primary".into(), "#ffffff".into()),
            ("error".into(), "#ff0000".into()), ("on_error".into(), "#ffffff".into()),
            ("tertiary".into(), "#00ff00".into()), ("on_tertiary".into(), "#000000".into()),
            ("secondary".into(), "#0000ff".into()), ("on_secondary".into(), "#ffffff".into()),
            ("outline_variant".into(), "#333333".into()), ("shadow".into(), "#000000".into()),
            ("on_surface".into(), "#eeeeee".into()), ("outline".into(), "#888888".into()),
            ("surface".into(), "#000000".into()), ("surface_container".into(), "#111111".into()),
            ("surface_container_low".into(), "#080808".into()), ("surface_container_high".into(), "#222222".into()),
            ("surface_container_highest".into(), "#292929".into()), ("surface_container_lowest".into(), "#040404".into()),
            ("primary_container".into(), "#444444".into()), ("on_primary_container".into(), "#eeeeee".into()),
            ("inverse_surface".into(), "#dddddd".into()), ("inverse_on_surface".into(), "#111111".into()),
            ("on_surface_variant".into(), "#aaaaaa".into()),
        ].into(), ..Theme::default() }
    }

    #[test]
    fn qt6ct_configuration_uses_breeze_sample_and_digit_fallback() {
        let config = qt_config("[Appearance]\nstyle=Fusion\nicon_theme=old\n");
        assert!(config.contains("style=Breeze"));
        assert!(config.contains("icon_theme=Adwaita"));
        assert!(config.contains("color_scheme_path="));
        assert_eq!(config.matches("[Appearance]").count(), 1);
        let sample = "[Colors:Window]\nForegroundNormal=220,221,222\nBackgroundNormal=30,31,32\n[Colors:Button]\nBackgroundNormal=40,41,42\nForegroundNormal=230,231,232\n[Colors:View]\nForegroundNormal=240,241,242\nBackgroundNormal=50,51,52\nBackgroundAlternate=60,61,62\nForegroundLink=70,71,72\nForegroundVisited=80,81,82\nForegroundInactive=90,91,92\n[Colors:Selection]\nBackgroundNormal=100,101,102\nForegroundNormal=110,111,112\n[Colors:Tooltip]\nBackgroundNormal=120,121,122\nForegroundNormal=130,131,132\n";
        let scheme = qt_scheme(&theme(), Some(sample));
        assert!(scheme.contains("active_colors="));
        assert!(scheme.contains("220,221,222"));
        assert!(scheme.contains("40,41,42"));
        assert!(scheme.contains("#7c83ff"));
        let fallback = qt_scheme(&theme(), None);
        assert!(fallback.contains("#000000"));
        assert!(fallback.contains("#7c83ff"));
    }

    #[test]
    fn gtk_surface_colors_cover_black_graphite_and_tinted() {
        let t = theme();
        let colors: Vec<_> = ["black", "graphite", "tinted"].map(|surface| {
            let look = kit::style::Look { round: 1.0, surface: surface.into(), accent: None, dense: false };
            gtk_css_for(&t, true, &look)
        }).into();
        for css in &colors {
            assert!(css.contains("@define-color window_bg_color"));
            assert!(css.contains("@define-color view_bg_color"));
            assert!(css.contains("@define-color card_bg_color"));
            assert!(css.contains("@define-color headerbar_bg_color"));
            assert!(css.contains("@define-color sidebar_bg_color"));
            assert!(css.contains("@define-color popover_bg_color"));
            assert!(css.contains("@define-color dialog_bg_color"));
        }
        assert_ne!(colors[0], colors[1]);
        assert_ne!(colors[1], colors[2]);
    }

    #[test]
    fn gtk3_stylesheet_omits_gtk4_root_custom_properties() {
        let look = kit::style::Look { round: 1.0, surface: "black".into(), accent: None, dense: false };
        let css = gtk_css_for(&theme(), false, &look);
        assert!(!css.contains(":root"));
        assert!(gtk_css_for(&theme(), true, &look).contains(":root"));
    }

    #[test]
    fn gtk_theme_selection_prefers_dark_and_leaves_missing_theme_alone() {
        let root = std::env::temp_dir().join(format!("digit-gtk-theme-{}", std::process::id()));
        let themes = root.join("themes");
        assert_eq!(theme_name_in(vec![themes.clone()]), None);
        std::fs::create_dir_all(themes.join("adw-gtk3/gtk-3.0")).unwrap();
        std::fs::write(themes.join("adw-gtk3/gtk-3.0/gtk.css"), "").unwrap();
        assert_eq!(theme_name_in(vec![themes.clone()]), Some("adw-gtk3"));
        std::fs::create_dir_all(themes.join("adw-gtk3-dark/gtk-3.0")).unwrap();
        std::fs::write(themes.join("adw-gtk3-dark/gtk-3.0/gtk.css"), "").unwrap();
        assert_eq!(theme_name_in(vec![themes]), Some("adw-gtk3-dark"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn gtk_settings_ini_preserves_unrelated_user_values() {
        let settings = settings_ini("[Settings]\ngtk-font-name=Sans 10\ngtk-theme-name=Old\n", "adw-gtk3-dark");
        assert!(settings.contains("gtk-font-name=Sans 10"));
        assert!(settings.contains("gtk-theme-name=adw-gtk3-dark"));
        assert!(settings.contains("gtk-application-prefer-dark-theme=1"));
    }
}
