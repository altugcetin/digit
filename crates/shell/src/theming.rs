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

fn gtk_css(t: &Theme) -> String {
    let c = |k: &str| t.hex(k);
    let pairs = [
        ("accent_color", c("primary")),
        ("accent_bg_color", c("primary")),
        ("accent_fg_color", c("on_primary")),
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
        ("window_bg_color", c("surface")),
        ("window_fg_color", c("on_surface")),
        ("view_bg_color", c("surface")),
        ("view_fg_color", c("on_surface")),
        ("headerbar_bg_color", c("surface")),
        ("headerbar_fg_color", c("on_surface")),
        ("headerbar_border_color", c("outline_variant")),
        ("headerbar_shade_color", c("shadow")),
        ("popover_bg_color", c("surface_container")),
        ("popover_fg_color", c("on_surface")),
        ("card_bg_color", c("surface_container")),
        ("card_fg_color", c("on_surface")),
        ("dialog_bg_color", c("surface_container_low")),
        ("dialog_fg_color", c("on_surface")),
        ("overview_bg_color", c("surface_container")),
        ("overview_fg_color", c("on_surface")),
        ("sidebar_bg_color", c("surface_container_low")),
        ("sidebar_fg_color", c("on_surface")),
        ("secondary_sidebar_bg_color", c("surface")),
        ("secondary_sidebar_fg_color", c("on_surface")),
        ("thumbnail_bg_color", c("surface_container_high")),
        ("thumbnail_fg_color", c("on_surface")),
        ("theme_bg_color", c("surface")),
        ("theme_fg_color", c("on_surface")),
        ("theme_base_color", c("surface")),
        ("theme_text_color", c("on_surface")),
        ("theme_selected_bg_color", c("primary")),
        ("theme_selected_fg_color", c("on_primary")),
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
    out.push_str("\n:root {\n");
    for (k, v) in &pairs {
        if !k.starts_with("theme_") && *k != "borders" {
            out.push_str(&format!("    --{}: {v};\n", k.replace('_', "-")));
        }
    }
    out.push_str("}\n");
    let r = niri_radius();
    out.push_str(&format!(
        "\nwindow.csd:not(.maximized):not(.fullscreen),\nwindow.csd:not(.maximized):not(.fullscreen) > decoration {{\n    border-radius: {r}px;\n}}\n"
    ));
    out
}

fn niri_radius() -> u32 {
    let path = gtk::glib::user_config_dir().join("niri/config.kdl");
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

fn qt_scheme(t: &Theme) -> String {
    let c = |k: &str| t.hex(k);
    let row = |text: &str| {
        [
            text, &c("surface_container"), &c("surface_container_highest"), &c("surface_container_high"), &c("surface_container_lowest"),
            &c("surface_container"), text, &c("on_primary_container"), text, &c("surface_container_low"), &c("surface"), &c("shadow"),
            &c("primary"), &c("on_primary"), &c("primary"), &c("tertiary"), &c("surface_container"), &c("surface"), &c("inverse_surface"),
            &c("inverse_on_surface"), &c("on_surface_variant"), &c("primary"),
        ]
        .join(", ")
    };
    let on = c("on_surface");
    let off = c("outline");
    format!("[ColorScheme]\nactive_colors={}\ndisabled_colors={}\ninactive_colors={}\n", row(&on), row(&off), row(&on))
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
    let css = gtk_css(t);
    for dir in gtk_dirs().into_iter().filter(|d| d.is_dir()) {
        write_if_changed(&dir.join("digit.css"), &css);
    }
    if let Some(parent) = qt_colors().parent().filter(|p| p.is_dir()) {
        write_if_changed(&parent.join("digit.conf"), &qt_scheme(t));
    }
}

pub fn take(qt: bool) {
    for dir in gtk_dirs().into_iter().filter(|d| d.is_dir()) {
        let file = dir.join("gtk.css");
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        if !text.lines().any(|l| l.trim() == IMPORT) {
            let joined = if text.is_empty() || text.ends_with('\n') { format!("{text}{IMPORT}\n") } else { format!("{text}\n{IMPORT}\n") };
            write_if_changed(&file, &joined);
        }
    }
    if !qt {
        give_back_qt();
        return;
    }
    let conf = qt_conf();
    let Ok(text) = std::fs::read_to_string(&conf) else { return };
    let ours = qt_colors().to_string_lossy().into_owned();
    let Some(current) = text.lines().find_map(|l| l.strip_prefix("color_scheme_path=")) else { return };
    if current == ours {
        return;
    }
    let _ = std::fs::create_dir_all(theme::state_dir());
    let _ = std::fs::write(saved_qt_path(), current);
    write_if_changed(&conf, &text.replace(&format!("color_scheme_path={current}"), &format!("color_scheme_path={ours}")));
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
        write_if_changed(&conf, &text.replace(&format!("color_scheme_path={ours}"), &format!("color_scheme_path={}", original.trim())));
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
