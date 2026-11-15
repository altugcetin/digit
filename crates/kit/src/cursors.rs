//! XCursor themes shared by the compositor and toolkit settings.
use std::collections::BTreeMap;
use std::path::PathBuf;

fn roots() -> Vec<PathBuf> {
    let mut roots = vec![
        gtk::glib::user_data_dir().join("icons"),
        gtk::glib::home_dir().join(".icons"),
    ];
    roots.extend(gtk::glib::system_data_dirs().into_iter().map(|p| p.join("icons")));
    roots
}

/// One of a theme's cursors (the first of `names` it has), as an image about `size` pixels
/// tall, for a preview. Reads the XCursor file directly.
pub fn image(theme: &str, names: &[&str], size: u32) -> Option<gtk::gdk::Texture> {
    if theme.is_empty() || theme.contains('/') || theme.starts_with('.') {
        return None;
    }
    let file = roots().into_iter().flat_map(|r| names.iter().map(move |n| r.join(theme).join("cursors").join(n))).find(|p| p.is_file())?;
    let bytes = std::fs::read(file).ok()?;
    let (width, height, pixels) = xcursor(&bytes, size)?;
    let bytes = gtk::glib::Bytes::from_owned(pixels);
    // XCursor pixels are premultiplied ARGB words, little endian: B, G, R, A in memory.
    use gtk::prelude::Cast;
    Some(gtk::gdk::MemoryTexture::new(width as i32, height as i32, gtk::gdk::MemoryFormat::B8g8r8a8Premultiplied, &bytes, width as usize * 4).upcast())
}

/// The image of the nominal size closest to `size` in an XCursor file: width, height, pixels.
fn xcursor(b: &[u8], size: u32) -> Option<(u32, u32, Vec<u8>)> {
    let word = |at: usize| b.get(at..at + 4).map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]));
    if b.get(0..4)? != b"Xcur" {
        return None;
    }
    let count = word(12)?.min(1024) as usize;
    let mut best: Option<(u32, usize)> = None;
    for i in 0..count {
        let entry = 16 + i * 12;
        if word(entry)? != 0xfffd_0002 {
            continue;
        }
        let (nominal, at) = (word(entry + 4)?, word(entry + 8)? as usize);
        if best.is_none_or(|(n, _)| nominal.abs_diff(size) < n.abs_diff(size)) {
            best = Some((nominal, at));
        }
    }
    let (_, at) = best?;
    let (width, height) = (word(at + 16)?, word(at + 20)?);
    if width == 0 || height == 0 || width > 512 || height > 512 {
        return None;
    }
    let start = at + 36;
    let pixels = b.get(start..start + (width * height * 4) as usize)?.to_vec();
    Some((width, height, pixels))
}

/// Installed cursor themes as (directory id, display name), sorted by id.
pub fn themes() -> Vec<(String, String)> {
    let mut roots = vec![
        gtk::glib::user_data_dir().join("icons"),
        gtk::glib::home_dir().join(".icons"),
    ];
    roots.extend(
        gtk::glib::system_data_dirs()
            .into_iter()
            .map(|p| p.join("icons")),
    );
    scan(&roots)
}

fn scan(roots: &[PathBuf]) -> Vec<(String, String)> {
    let mut found = BTreeMap::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            if !entry.path().join("cursors").is_dir() {
                continue;
            }
            let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let text =
                std::fs::read_to_string(entry.path().join("index.theme")).unwrap_or_default();
            let mut section = false;
            let name = text
                .lines()
                .find_map(|line| {
                    let line = line.trim();
                    if line.starts_with('[') {
                        section = line == "[Icon Theme]";
                    }
                    section
                        .then(|| line.split_once('='))
                        .flatten()
                        .filter(|(key, value)| key.trim() == "Name" && !value.trim().is_empty())
                        .map(|(_, value)| value.trim().to_owned())
                })
                .unwrap_or_else(|| id.clone());
            found.entry(id).or_insert(name);
        }
    }
    found.into_iter().collect()
}

/// Selected theme (empty means system default) and size in logical pixels.
pub fn current(prefs: &toml::Table) -> (String, u32) {
    let theme = crate::prefs::text(prefs, "cursor_theme", "");
    let theme = if theme.chars().any(char::is_control) {
        String::new()
    } else {
        theme
    };
    let size = prefs
        .get("cursor_size")
        .and_then(toml::Value::as_integer)
        .unwrap_or(24)
        .clamp(1, 256) as u32;
    (theme, size)
}

pub(crate) fn hypr(prefs: &toml::Table) -> String {
    if !prefs.contains_key("cursor_theme") && !prefs.contains_key("cursor_size") { return String::new() }
    let (theme, size) = current(prefs);
    let mut out =
        format!("hl.env(\"XCURSOR_SIZE\", \"{size}\")\nhl.env(\"HYPRCURSOR_SIZE\", \"{size}\")\n");
    if !theme.is_empty() {
        let quoted = serde_json::to_string(&theme).unwrap();
        out.push_str(&format!(
            "hl.env(\"XCURSOR_THEME\", {quoted})\nhl.env(\"HYPRCURSOR_THEME\", {quoted})\n"
        ));
    }
    out
}

pub(crate) fn niri(prefs: &toml::Table) -> Vec<String> {
    let (theme, size) = current(prefs);
    let mut out = vec![format!("    xcursor-size {size}")];
    if !theme.is_empty() {
        out.push(format!(
            "    xcursor-theme {}",
            serde_json::to_string(&theme).unwrap()
        ));
    }
    out
}

/// Apply the cursor immediately after regenerating Hyprland's environment.
pub(crate) fn apply_live() {
    let prefs = crate::prefs::read();
    if !prefs.contains_key("cursor_theme") && !prefs.contains_key("cursor_size") { return; }
    if crate::setup::installed()
        && crate::setup::kind() == Some(crate::setup::Kind::Hyprland)
        && std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some()
    {
        let (theme, size) = current(&crate::prefs::read());
        // Removing env declarations on reload does not clear the previous theme.
        // Reset the compositor child environment too when returning to system default.
        if theme.is_empty() {
            let _ = std::process::Command::new("hyprctl").args(["eval", "hl.env(\"XCURSOR_THEME\", \"default\"); hl.env(\"HYPRCURSOR_THEME\", \"default\")"]).output();
        }
        let _ = std::process::Command::new("hyprctl")
            .args([
                "setcursor",
                if theme.is_empty() { "default" } else { &theme },
                &size.to_string(),
            ])
            .output();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cursor_environment_requires_an_explicit_choice() {
        assert!(hypr(&toml::Table::new()).is_empty());
        assert!(hypr(&"cursor_size = 32".parse().unwrap()).contains("XCURSOR_SIZE"));
    }

    #[test]
    fn scans_cursor_folders_and_names_with_user_precedence() {
        let root = std::env::temp_dir().join(format!("digit-cursors-{}", std::process::id()));
        let user = root.join("user");
        let system = root.join("system");
        for p in [
            user.join("Zed/cursors"),
            system.join("Zed/cursors"),
            system.join("Alpha/cursors"),
            user.join("Icons"),
        ] {
            std::fs::create_dir_all(p).unwrap();
        }
        std::fs::write(
            user.join("Zed/index.theme"),
            "[Other]\nName=Wrong\n[Icon Theme]\nName=My cursor\n",
        )
        .unwrap();
        assert_eq!(
            scan(&[user, system]),
            vec![
                ("Alpha".into(), "Alpha".into()),
                ("Zed".into(), "My cursor".into())
            ]
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn renders_selected_and_system_cursor() {
        let prefs = "cursor_theme = 'Breeze'\ncursor_size = 32".parse().unwrap();
        assert!(hypr(&prefs).contains("hl.env(\"HYPRCURSOR_THEME\", \"Breeze\")"));
        assert!(niri(&prefs).contains(&"    xcursor-theme \"Breeze\"".into()));
        let empty = toml::Table::new();
        assert_eq!(current(&empty), (String::new(), 24));
        assert!(!hypr(&empty).contains("_THEME"));
        assert_eq!(niri(&empty), vec!["    xcursor-size 24"]);
    }
}
