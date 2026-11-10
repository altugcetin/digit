use gtk::glib;
use kit::theme::{self, Theme};
use std::path::PathBuf;
use std::sync::{Mutex, atomic::{AtomicU64, Ordering}};

static JOBS: ThemeJobs = ThemeJobs { generation: AtomicU64::new(0), publish: Mutex::new(()) };
static WRITING: Mutex<()> = Mutex::new(());

struct ThemeJobs {
    generation: AtomicU64,
    publish: Mutex<()>,
}

impl ThemeJobs {
    fn begin(&self) -> Option<u64> {
        let _publish = self.publish.lock().ok()?;
        Some(self.generation.fetch_add(1, Ordering::Relaxed) + 1)
    }

    fn publish<T>(&self, generation: u64, write: impl FnOnce() -> Option<T>) -> Option<T> {
        let _publish = self.publish.lock().ok()?;
        if self.generation.load(Ordering::Relaxed) != generation { return None; }
        write()
    }
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

fn theme_name_in(roots: Vec<PathBuf>) -> Option<&'static str> {
    let available = |name: &str| roots.iter().any(|root| root.join(name).join("gtk-3.0/gtk.css").is_file());
    if available("adw-gtk3-dark") { Some("adw-gtk3-dark") }
    else if available("adw-gtk3") { Some("adw-gtk3") }
    else { None }
}

fn settings_ini(old: &str, theme: &str) -> String {
    let dark = theme.ends_with("-dark") || theme.ends_with("Dark");
    kit::appearance::ini(old, "[Settings]", &[("gtk-theme-name", Some(theme)), ("gtk-application-prefer-dark-theme", Some(if dark { "1" } else { "0" }))])
}

fn qt_config_for(old: &str, style: &str) -> String {
    kit::appearance::ini(old, "[Appearance]", &[
        ("style", Some(style)), ("custom_palette", Some("false")),
        ("color_scheme_path", None),
    ])
}

/// Runs on a worker. Snapshot, apply, restore and explicit icon changes share one writer.
fn apply(t: &Theme, icons: bool) -> std::io::Result<()> {
    use kit::app_themes::{GSettings, Store};
    let _one = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    // A queued palette job must use the latest published theme after acquiring the writer.
    let latest = theme::load();
    let t = latest.as_ref().unwrap_or(t);
    let prefs = kit::prefs::read();
    let store = Store::default();
    let settings = GSettings::default();
    let qt = kit::prefs::flag(&prefs, "theme_qt", false);
    let platform = if qt { kit::distro::qt_platform() } else { kit::distro::QtPlatform::Missing };
    apply_owned(t, icons, &prefs, &store, &settings, platform)
}

fn apply_owned(t: &Theme, icons: bool, prefs: &toml::Table, store: &kit::app_themes::Store, settings: &impl kit::app_themes::DesktopSettings, platform: kit::distro::QtPlatform) -> std::io::Result<()> {
    use kit::app_themes::{Toolkit, atomic_write};
    let gtk_result = store.apply(Toolkit::Gtk, kit::prefs::flag(prefs, "theme_gtk", false), settings, || {
        let choice = kit::prefs::text(prefs, "gtk_theme", "digit");
        let dynamic = choice == "digit";
        let roots = kit::app_themes::gtk_roots();
        let name = if dynamic { theme_name_in(roots).map(str::to_owned) } else { Some(choice) };
        for (index, version) in ["gtk-3.0", "gtk-4.0"].iter().enumerate() {
            if dynamic {
                if !t.roles.is_empty() { atomic_write(&store.config.join(version).join("gtk.css"), gtk_css(t, index == 1).as_bytes())?; }
            } else { store.original_file(Toolkit::Gtk, index * 2)?; }
            if let Some(name) = &name {
                let path = store.config.join(version).join("settings.ini");
                let old = read_ini(&path)?;
                atomic_write(&path, settings_ini(&old, name).as_bytes())?;
            }
        }
        let dark = dynamic || name.as_ref().is_some_and(|n| n.ends_with("-dark") || n.ends_with("Dark"));
        settings.put("color-scheme", Some(if dark { "'prefer-dark'" } else { "'default'" }))?;
        if let Some(name) = name { settings.put("gtk-theme", Some(&gtk::glib::Variant::from(name.as_str()).print(true)))?; }
        Ok(())
    });
    let qt = kit::prefs::flag(prefs, "theme_qt", false);
    let qt_result = store.apply(Toolkit::Qt, qt && platform != kit::distro::QtPlatform::Missing, settings, || {
        let choice = prefs.get("qt_theme").and_then(toml::Value::as_str);
        let styles = kit::app_themes::qt_styles();
        let style = kit::app_themes::effective_qt_style(choice, &styles);
        let path = store.config.join("qt6ct/qt6ct.conf");
        atomic_write(&path, qt_config_for(&read_ini(&path)?, &style).as_bytes())
    });
    let icons_result = if icons { kit::appearance::apply_icons().map_err(std::io::Error::other) } else { Ok(()) };
    gtk_result.and(qt_result).and(icons_result)
}

fn read_ini(path: &std::path::Path) -> std::io::Result<String> {
    match std::fs::read_to_string(path) { Ok(s) => Ok(s), Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()), Err(e) => Err(e) }
}

fn write(t: &Theme, icons: bool) {
    if let Err(e) = apply(t, icons) { eprintln!("digit: app themes: {e}"); }
    if kit::setup::installed() && let Err(e) = kit::setup::apply() { eprintln!("digit: {e}"); }
}

pub fn changed(icons: bool) {
    glib::spawn_future_local(async move {
        let _ = gtk::gio::spawn_blocking(move || {
            write(&theme::load().unwrap_or_default(), icons);
        }).await;
    });
}

pub fn cursor_changed() {
    glib::spawn_future_local(async {
        let _ = gtk::gio::spawn_blocking(|| {
            let _one = WRITING.lock().unwrap_or_else(|e| e.into_inner());
            if let Err(e) = kit::appearance::apply_cursor() { eprintln!("digit: {e}"); }
        }).await;
    });
}

pub fn refresh(chosen: Option<&str>, fallback: &str, image: Option<String>, force: bool) -> Option<Theme> {
    let generation = JOBS.begin()?;
    let current = theme::load();
    let scheme = chosen.map(str::to_string).or_else(|| current.as_ref().map(|t| t.scheme.clone())).unwrap_or_else(|| fallback.to_string());
    let scheme = scheme.as_str();
    let image = image.or_else(crate::wallpaper::current).or_else(crate::wallpaper::found).or_else(|| current.as_ref().map(|t| t.wallpaper.clone()))?;
    let fresh = match &current {
        Some(t) if !force && t.wallpaper == image && t.scheme == scheme => t.clone(),
        _ => theme::generate(std::path::Path::new(&image), scheme)?,
    };
    JOBS.publish(generation, || {
        theme::save(&fresh).ok()?;
        Some(fresh)
    })
}

pub fn restyle() {
    changed(false);
    crate::deskicons::restyle();
}

#[cfg(test)]
mod tests {
    use super::*;

    // GLib caches XDG paths, so run each filesystem case in a fresh test process.
    fn isolated(name: &str, check: impl FnOnce(&std::path::Path)) {
        if std::env::var("DIGIT_THEME_TEST").as_deref() == Ok(name) {
            check(std::path::Path::new(&std::env::var("XDG_CONFIG_HOME").unwrap()));
            return;
        }
        let root = std::env::temp_dir().join(format!("digit-themes-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &format!("theming::tests::{name}"), "--nocapture"])
            .env("DIGIT_THEME_TEST", name).env("XDG_CONFIG_HOME", root.join("config"))
            .env("XDG_STATE_HOME", root.join("state")).env("XDG_DATA_HOME", root.join("data"))
            .env("XDG_DATA_DIRS", root.join("share")).env("HOME", &root).env("GSETTINGS_BACKEND", "memory")
            .env("GSETTINGS_SCHEMA_DIR", "/usr/share/glib-2.0/schemas")
            .env_remove("DBUS_SESSION_BUS_ADDRESS").env_remove("NIRI_SOCKET").env_remove("HYPRLAND_INSTANCE_SIGNATURE")
            .env_remove("DIGIT_SESSION").env_remove("DISPLAY").env_remove("WAYLAND_DISPLAY").output().unwrap();
        std::fs::remove_dir_all(root).unwrap();
        assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    }

    #[test]
    fn off_apply_entry_points_leave_xdg_empty() {
        isolated("off_apply_entry_points_leave_xdg_empty", |config| {
            apply(&theme(), false).unwrap();
            write(&theme(), false);
            assert_eq!(std::fs::read_dir(config.parent().unwrap()).unwrap().count(), 0);
        });
    }

    #[test]
    fn gtk_switching_from_digit_to_named_restores_user_css_then_disable_restores_everything() {
        isolated("gtk_switching_from_digit_to_named_restores_user_css_then_disable_restores_everything", |config| {
            use kit::app_themes::{atomic_write, DesktopSettings, GSettings};
            let settings = GSettings::default();
            settings.put("gtk-theme", Some("'Original'")).unwrap();
            atomic_write(&config.join("gtk-3.0/gtk.css"), b"user css\r\n").unwrap();
            atomic_write(&config.join("gtk-4.0/settings.ini"), b"[Settings]\nuser-key=keep\n").unwrap();
            kit::prefs::set("theme_gtk", true.into()).unwrap();
            apply(&theme(), false).unwrap();
            assert!(std::fs::read_to_string(config.join("gtk-3.0/gtk.css")).unwrap().contains("@define-color"));
            assert!(!std::fs::read_to_string(config.join("gtk-4.0/gtk.css")).unwrap().contains("@import"));
            kit::prefs::set("gtk_theme", "ExampleDark".into()).unwrap();
            apply(&theme(), false).unwrap();
            assert_eq!(std::fs::read(config.join("gtk-3.0/gtk.css")).unwrap(), b"user css\r\n");
            assert!(!config.join("gtk-4.0/gtk.css").exists());
            let text = std::fs::read_to_string(config.join("gtk-4.0/settings.ini")).unwrap();
            assert!(text.contains("user-key=keep"));
            assert!(text.contains("gtk-theme-name=ExampleDark"));
            kit::prefs::set("gtk_theme", "Example".into()).unwrap();
            apply(&theme(), false).unwrap();
            assert_eq!(settings.get("color-scheme").unwrap().as_deref(), Some("'default'"));
            kit::prefs::set("theme_gtk", false.into()).unwrap();
            apply(&theme(), false).unwrap();
            assert!(!config.join("gtk-3.0/settings.ini").exists());
            assert_eq!(std::fs::read(config.join("gtk-4.0/settings.ini")).unwrap(), b"[Settings]\nuser-key=keep\n");
            assert_eq!(settings.get("gtk-theme").unwrap().as_deref(), Some("'Original'"));
            assert_eq!(settings.get("color-scheme").unwrap(), None);
        });
    }

    #[test]
    fn qt_modes_restore_originals_and_require_a_platform_theme() {
        isolated("qt_modes_restore_originals_and_require_a_platform_theme", |config| {
            use kit::{app_themes::{GSettings, Store, atomic_write}, distro::QtPlatform};
            let store = Store::default();
            let settings = GSettings::default();
            let mut prefs: toml::Table = "theme_qt = true".parse().unwrap();
            apply_owned(&theme(), false, &prefs, &store, &settings, QtPlatform::Missing).unwrap();
            assert!(!config.exists());
            let conf = config.join("qt6ct/qt6ct.conf");
            let original = b"[Appearance]\nicon_theme=Keep\n[Fonts]\ngeneral=User Font\n";
            atomic_write(&conf, original).unwrap();
            apply_owned(&theme(), false, &prefs, &store, &settings, QtPlatform::Qt6ct).unwrap();
            let plain = std::fs::read_to_string(&conf).unwrap();
            apply_owned(&theme(), false, &prefs, &store, &settings, QtPlatform::Kde).unwrap();
            assert!(plain.contains("custom_palette=false"));
            assert!(!plain.contains("color_scheme_path="));
            assert!(!config.join("qt6ct/colors/digit.conf").exists());
            assert!(!theme::state_dir().join("app-themes/digit.colors").exists());
            prefs.insert("qt_theme".into(), "Fusion".into());
            apply_owned(&theme(), false, &prefs, &store, &settings, QtPlatform::Kde).unwrap();
            let named = std::fs::read_to_string(&conf).unwrap();
            assert!(named.contains("style=Fusion"));
            assert!(named.contains("custom_palette=false"));
            assert!(!named.contains("color_scheme_path="));
            assert!(named.contains("icon_theme=Keep"));
            assert!(named.contains("general=User Font"));
            atomic_write(&conf, b"hand edit").unwrap();
            prefs.insert("theme_qt".into(), false.into());
            apply_owned(&theme(), false, &prefs, &store, &settings, QtPlatform::Kde).unwrap();
            assert_eq!(std::fs::read(conf).unwrap(), original);
            assert!(!theme::state_dir().join("app-themes/qt.json").exists());
        });
    }

    #[test]
    fn shell_never_applies_external_button_layout() {
        assert!(!include_str!("main.rs").contains("appearance::apply_buttons"));
        assert!(!include_str!("app.rs").contains("appearance::apply_buttons"));
        let main = include_str!("main.rs");
        let watcher = main.split("fn watch(app:").nth(1).unwrap().split("fn watch_palette").next().unwrap();
        assert!(!watcher.contains("create_dir_all"), "watching GTK must not create its configuration directory");
    }

    #[test]
    fn qt_config_disables_custom_palette_preserves_unrelated_keys_and_removes_scheme() {
        let old = "[Appearance]\nicon_theme=User\nstyle=Breeze\ncustom_palette=true\ncolor_scheme_path=/old\n[Fonts]\ngeneral=User Font\n";
        let text = qt_config_for(old, "kvantum");
        assert!(text.contains("style=kvantum\n"));
        assert!(text.contains("custom_palette=false\n"));
        assert!(!text.contains("color_scheme_path="));
        assert!(text.contains("icon_theme=User\n"));
        assert!(text.contains("general=User Font\n"));
    }

    #[test]
    fn stale_theme_jobs_cannot_publish_after_a_newer_request() {
        let jobs = ThemeJobs { generation: AtomicU64::new(0), publish: Mutex::new(()) };
        let old = jobs.begin().unwrap();
        let new = jobs.begin().unwrap();
        assert_eq!(jobs.publish(new, || Some("new")), Some("new"));
        assert_eq!(jobs.publish(old, || panic!("obsolete theme published")), None::<()>);
    }

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
    fn qt6ct_config_preserves_unrelated_appearance_keys() {
        let config = qt_config_for("[Appearance]\nstyle=Fusion\nicon_theme=Adwaita\ncolor_scheme_path=/old\n", "Breeze");
        assert!(config.contains("style=Breeze"));
        assert!(config.contains("custom_palette=false"));
        assert!(config.contains("icon_theme=Adwaita\n"));
        assert!(!config.contains("color_scheme_path="));
        assert_eq!(config.matches("[Appearance]").count(), 1);
    }

    #[test]
    fn qt_style_falls_back_to_breeze_then_fusion() {
        use kit::app_themes::effective_qt_style;
        let styles = vec!["Fusion".into(), "Breeze".into(), "kvantum".into()];
        assert_eq!(effective_qt_style(None, &styles), "Breeze");
        assert_eq!(effective_qt_style(Some("digit"), &styles), "Breeze");
        assert_eq!(effective_qt_style(Some("Removed"), &styles), "Breeze");
        assert_eq!(effective_qt_style(Some("kvantum"), &styles), "kvantum");
        assert_eq!(effective_qt_style(None, &["Fusion".into()]), "Fusion");
    }

    #[test]
    fn wallpaper_retheme_does_not_write_qt_config_again() {
        isolated("wallpaper_retheme_does_not_write_qt_config_again", |config| {
            use kit::{app_themes::{GSettings, Store}, distro::QtPlatform};
            let store = Store::default();
            let settings = GSettings::default();
            let prefs: toml::Table = "theme_qt = true\nqt_theme = \"Fusion\"".parse().unwrap();
            apply_owned(&theme(), false, &prefs, &store, &settings, QtPlatform::Qt6ct).unwrap();
            let qt_dir = config.join("qt6ct");
            let conf = qt_dir.join("qt6ct.conf");
            let before = std::fs::metadata(&conf).unwrap().modified().unwrap();
            let contents = std::fs::read(&conf).unwrap();
            let mut changed_theme = theme();
            changed_theme.roles.insert("primary".into(), "#123456".into());
            apply_owned(&changed_theme, false, &prefs, &store, &settings, QtPlatform::Qt6ct).unwrap();
            assert_eq!(std::fs::read(&conf).unwrap(), contents);
            assert_eq!(std::fs::metadata(&conf).unwrap().modified().unwrap(), before);
            assert_eq!(std::fs::read_dir(qt_dir).unwrap().count(), 1);
        });
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
