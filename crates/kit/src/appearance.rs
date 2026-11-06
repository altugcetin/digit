//! Toolkit appearance shared by Digit apps and the owned desktop session.

/// Automatic icons use Adwaita, consistently across GTK and Qt.
pub fn icon_theme(prefs: &toml::Table) -> String {
    prefs
        .get("icon_theme")
        .and_then(toml::Value::as_str)
        .filter(|s| !s.is_empty() && !s.chars().any(char::is_control))
        .unwrap_or("Adwaita")
        .to_owned()
}

/// Update this process's GTK settings. Safe outside a Digit session.
pub fn apply_gtk() {
    let Some(settings) = gtk::Settings::default() else {
        return;
    };
    let prefs = crate::prefs::read();
    settings.set_gtk_icon_theme_name(Some(&icon_theme(&prefs)));
    let (theme, size) = crate::cursors::current(&prefs);
    if theme.is_empty() {
        settings.reset_property("gtk-cursor-theme-name");
    } else {
        settings.set_gtk_cursor_theme_name(Some(&theme));
    }
    settings.set_gtk_cursor_theme_size(size as i32);
}

/// Set or remove only the named keys in one INI section, preserving unrelated values.
fn ini(old: &str, section: &str, values: &[(&str, Option<&str>)]) -> String {
    let mut lines: Vec<String> = old.lines().map(str::to_owned).collect();
    if !lines.iter().any(|l| l.trim() == section) {
        lines.push(section.into());
    }
    let start = lines.iter().position(|l| l.trim() == section).unwrap();
    let end = lines
        .iter()
        .enumerate()
        .skip(start + 1)
        .find(|(_, l)| l.trim().starts_with('['))
        .map_or(lines.len(), |(i, _)| i);
    for i in (start + 1..end).rev() {
        if lines[i]
            .split_once('=')
            .is_some_and(|(key, _)| values.iter().any(|(k, _)| key.trim() == *k))
        {
            lines.remove(i);
        }
    }
    for (key, value) in values.iter().rev() {
        if let Some(value) = value {
            lines.insert(start + 1, format!("{key}={value}"));
        }
    }
    format!("{}\n", lines.join("\n"))
}

/// Persist GTK/Qt settings and broadcast desktop settings only inside a Digit session.
pub fn apply() -> Result<(), String> {
    if !crate::hyprconf::installed() && !crate::niriconf::installed() {
        return Ok(());
    }
    static WRITING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    let prefs = crate::prefs::read();
    let icons = icon_theme(&prefs);
    let (cursor, size) = crate::cursors::current(&prefs);
    let size = size.to_string();
    for version in ["gtk-3.0", "gtk-4.0"] {
        update(
            &gtk::glib::user_config_dir()
                .join(version)
                .join("settings.ini"),
            "[Settings]",
            &[
                ("gtk-icon-theme-name", Some(&icons)),
                (
                    "gtk-cursor-theme-name",
                    (!cursor.is_empty()).then_some(cursor.as_str()),
                ),
                ("gtk-cursor-theme-size", Some(&size)),
            ],
        )?;
    }
    update(
        &gtk::glib::user_config_dir().join("qt6ct/qt6ct.conf"),
        "[Appearance]",
        &[("icon_theme", Some(&icons))],
    )?;
    for (key, value) in [
        ("icon-theme", icons.as_str()),
        ("cursor-theme", cursor.as_str()),
        ("cursor-size", size.as_str()),
    ] {
        let mut cmd = std::process::Command::new("gsettings");
        if key == "cursor-theme" && value.is_empty() {
            cmd.args(["reset", "org.gnome.desktop.interface", key]);
        } else {
            // String values are GVariant literals, not shell expressions.
            let value = if key == "cursor-size" {
                value.to_owned()
            } else {
                format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
            };
            cmd.args(["set", "org.gnome.desktop.interface", key, &value]);
        }
        let _ = cmd.output();
    }
    Ok(())
}

fn update(
    path: &std::path::Path,
    section: &str,
    values: &[(&str, Option<&str>)],
) -> Result<(), String> {
    let old = match std::fs::read_to_string(path) {
        Ok(old) => old,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.to_string()),
    };
    crate::hyprconf::write_if_changed(path, &ini(&old, section, values))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn qt_icons_follow_the_preference_and_automatic_default() {
        for (prefs, expected) in [
            ("icon_theme = 'Papirus-Dark'", "Papirus-Dark"),
            ("", "Adwaita"),
        ] {
            let icons = icon_theme(&prefs.parse().unwrap());
            let text = ini(
                "[Appearance]\nicon_theme=Old\nstyle=Breeze\n[Other]\nkeep=yes\n",
                "[Appearance]",
                &[("icon_theme", Some(&icons))],
            );
            assert!(text.contains(&format!("icon_theme={expected}\n")));
            assert!(text.contains("style=Breeze\n[Other]\nkeep=yes"));
            assert_eq!(text.matches("icon_theme=").count(), 1);
        }
    }
}
