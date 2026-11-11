//! Qt palette discovery and generation. Filesystem calls belong on a worker.
use crate::{distro::QtPlatform, theme::Theme};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Debug, PartialEq)]
pub struct Scheme {
    pub id: String,
    pub name: String,
}

pub fn schemes(platform: QtPlatform) -> Vec<Scheme> {
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| gtk::glib::home_dir().join(".local/share"));
    let dirs = std::env::var_os("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    schemes_in(gtk::glib::user_config_dir(), data, std::env::split_paths(&dirs).collect(), platform)
}

pub fn schemes_in(config: PathBuf, data: PathBuf, dirs: Vec<PathBuf>, platform: QtPlatform) -> Vec<Scheme> {
    let mut roots = vec![(config.join("qt6ct/colors"), "conf")];
    roots.extend(dirs.iter().map(|p| (p.join("qt6ct/colors"), "conf")));
    if platform == QtPlatform::Kde {
        roots.push((data.join("color-schemes"), "colors"));
        roots.extend(dirs.iter().map(|p| (p.join("color-schemes"), "colors")));
    }
    let mut found = BTreeMap::new();
    for (root, extension) in roots {
        for entry in std::fs::read_dir(root).into_iter().flatten().flatten() {
            let path = entry.path();
            if !path.is_file() || path.extension().is_none_or(|e| e != extension) {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if stem.eq_ignore_ascii_case("digit") {
                continue;
            }
            let name = if extension == "colors" {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                let mut general = false;
                text.lines()
                    .find_map(|l| {
                        let l = l.trim();
                        if l.starts_with('[') {
                            general = l == "[General]";
                        }
                        if general {
                            l.split_once('=').filter(|(k, v)| k.trim() == "Name" && !v.trim().is_empty()).map(|(_, v)| v.trim().to_owned())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| stem.into())
            } else {
                stem.into()
            };
            let Ok(path) = path.canonicalize() else {
                continue;
            };
            let Some(id) = path.to_str().filter(|s| !s.chars().any(char::is_control)) else {
                continue;
            };
            if name.chars().any(char::is_control) {
                continue;
            }
            found.entry(id.to_owned()).or_insert(Scheme { id: id.into(), name });
        }
    }
    let mut found: Vec<_> = found.into_values().collect();
    found.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    found
}

pub fn effective<'a>(choice: Option<&'a str>, schemes: &[Scheme]) -> &'a str {
    choice.filter(|c| *c == "style" || schemes.iter().any(|s| s.id == *c)).unwrap_or("digit")
}

fn rgb(hex: &str) -> [u8; 3] {
    [1, 3, 5].map(|i| hex.get(i..i + 2).and_then(|s| u8::from_str_radix(s, 16).ok()).unwrap_or(0))
}
fn mix(fg: &str, bg: &str) -> String {
    let a = rgb(fg);
    let b = rgb(bg);
    let c = [0, 1, 2].map(|i| (u16::from(a[i]) + u16::from(b[i])) / 2);
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}
fn luminance(hex: &str) -> f64 {
    let [r, g, b] = rgb(hex).map(|v| {
        let v = f64::from(v) / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    });
    0.2126 * r + 0.7152 * g + 0.0722 * b
}
fn semantic(bg: &str, light: &str, dark: &str) -> String {
    let contrast = |fg: &str| {
        let a = luminance(fg);
        let b = luminance(bg);
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    };
    let mut color = if contrast(light) >= contrast(dark) { light } else { dark }.to_owned();
    let target = if luminance(bg) < 0.179 { "#ffffff" } else { "#000000" };
    // Preserve the semantic hue while moving its tone to readable contrast.
    for _ in 0..8 {
        if contrast(&color) >= 4.5 {
            break;
        }
        color = mix(&color, target);
    }
    color
}

/// qt6ct's shipped palettes contain 21 QPalette roles, including NoRole (17),
/// through PlaceholderText (20); they do not serialize Qt 6.6's Accent (21).
pub fn palette(t: &Theme, accent: &str, on_accent: &str) -> String {
    let roles = [
        "on_surface_variant",
        "surface_container_high",
        "surface_container_highest",
        "surface_container_high",
        "surface_container_lowest",
        "surface_container",
        "on_surface",
        "on_surface",
        "on_surface",
        "surface",
        "surface_container",
        "shadow",
        "",
        "",
        "tertiary",
        "secondary",
        "surface_container",
        "on_surface",
        "surface_container",
        "on_surface",
        "outline",
    ];
    let active: Vec<_> = roles
        .iter()
        .enumerate()
        .map(|(i, r)| match i {
            12 => accent.into(),
            13 => on_accent.into(),
            _ => t.hex(r),
        })
        .collect();
    let mut disabled = active.clone();
    for (i, bg) in [(0, 10), (6, 9), (7, 9), (8, 1), (13, 12), (14, 9), (15, 9), (19, 18), (20, 9)] {
        disabled[i] = mix(&active[i], &active[bg]);
    }
    format!(
        "[ColorScheme]\nactive_colors={}\ninactive_colors={}\ndisabled_colors={}\n",
        active.join(", "),
        active.join(", "),
        disabled.join(", ")
    )
}

pub fn kde(t: &Theme, accent: &str, on_accent: &str) -> String {
    let c = |r| t.hex(r);
    let mut out = format!(
        "[General]\nName=Digit\nColorScheme=Digit\n[KDE]\ncontrast=4\n[ColorEffects:Inactive]\nChangeSelectionColor=true\nColor={}\nColorAmount=0.025\nColorEffect=0\nContrastAmount=0.1\nContrastEffect=0\nEnable=true\n[ColorEffects:Disabled]\nColor={}\nColorAmount=0.5\nColorEffect=3\nContrastAmount=0\nContrastEffect=0\nEnable=false\n",
        c("surface_container_lowest"),
        c("surface_container")
    );
    for (section, bg, alt, normal, active, inactive, hover) in [
        ("Window", "surface_container", "surface_container_highest", "on_surface_variant", "primary", "outline", ""),
        ("View", "surface", "surface_container", "on_surface", "on_surface", "outline", "primary_container"),
        ("Button", "surface_container_high", "surface_container_highest", "on_surface", "on_surface", "outline", ""),
        ("Selection", "", "", "", "", "", "secondary"),
        ("Tooltip", "surface_container", "surface_container_highest", "on_surface", "on_surface", "outline", ""),
        ("Header", "surface_container", "surface_container", "on_surface_variant", "on_surface", "outline", ""),
        ("Header][Inactive", "surface_container", "surface_container", "on_surface_variant", "on_surface", "outline", ""),
        ("Complementary", "surface_container", "surface_container_lowest", "on_surface_variant", "on_surface", "outline", ""),
    ] {
        let bg = if bg.is_empty() { accent.into() } else { c(bg) };
        out.push_str(&format!("[Colors:{section}]\n"));
        for (key, value) in [
            ("BackgroundNormal", bg.clone()),
            ("BackgroundAlternate", if alt.is_empty() { accent.into() } else { c(alt) }),
            ("ForegroundNormal", if normal.is_empty() { on_accent.into() } else { c(normal) }),
            ("ForegroundActive", if active.is_empty() { on_accent.into() } else { c(active) }),
            ("ForegroundInactive", if inactive.is_empty() { on_accent.into() } else { c(inactive) }),
            ("DecorationFocus", accent.into()),
            ("DecorationHover", if hover.is_empty() { accent.into() } else { c(hover) }),
            ("ForegroundLink", c("tertiary")),
            ("ForegroundVisited", c("secondary")),
            ("ForegroundNegative", c("error")),
            ("ForegroundNeutral", semantic(&bg, "#ffb86c", "#854000")),
            ("ForegroundPositive", semantic(&bg, "#79dc94", "#006b35")),
        ] {
            out.push_str(&format!("{key}={value}\n"));
        }
    }
    out.push_str("[WM]\n");
    for (key, role) in [
        ("activeBackground", "surface_container_high"),
        ("activeForeground", "on_surface"),
        ("inactiveBackground", "surface_container"),
        ("inactiveForeground", "on_surface_variant"),
        ("activeBlend", "outline"),
        ("inactiveBlend", "outline"),
    ] {
        let [r, g, b] = rgb(&c(role));
        out.push_str(&format!("{key}={r},{g},{b}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_themes::atomic_write;

    fn theme() -> Theme {
        let roles = [
            "on_surface_variant",
            "surface_container_high",
            "surface_container_highest",
            "surface_container_lowest",
            "surface_container",
            "on_surface",
            "on_error",
            "surface",
            "shadow",
            "tertiary",
            "secondary",
            "outline",
            "primary",
            "primary_container",
        ];
        Theme {
            roles: roles.iter().enumerate().map(|(i, r)| (r.to_string(), format!("#{:06x}", 0x102030 + i * 0x080808))).collect(),
            ..Theme::default()
        }
    }

    fn values<'a>(text: &'a str, key: &str) -> Vec<&'a str> {
        text.lines().find_map(|l| l.strip_prefix(&format!("{key}="))).unwrap().split(", ").collect()
    }

    #[test]
    fn palette_matches_shipped_qt6ct_order_and_tones_disabled_text() {
        // Copied from /usr/share/qt6ct/colors/darker.conf (21 roles).
        let fixture = include_str!("fixtures/qt6ct-darker.conf");
        let t = theme();
        let output = palette(&t, "#123456", "#ffffff");
        let active = values(&output, "active_colors");
        assert_eq!(active.len(), values(fixture, "active_colors").len());
        let expected = [
            "on_surface_variant",
            "surface_container_high",
            "surface_container_highest",
            "surface_container_high",
            "surface_container_lowest",
            "surface_container",
            "on_surface",
            "on_surface",
            "on_surface",
            "surface",
            "surface_container",
            "shadow",
            "accent",
            "on_accent",
            "tertiary",
            "secondary",
            "surface_container",
            "on_surface",
            "surface_container",
            "on_surface",
            "outline",
        ];
        for (i, role) in expected.iter().enumerate() {
            let color = match *role {
                "accent" => "#123456".into(),
                "on_accent" => "#ffffff".into(),
                _ => t.hex(role),
            };
            assert_eq!(active[i], color, "role {i}");
        }
        assert_eq!(active, values(&output, "inactive_colors"));
        let disabled = values(&output, "disabled_colors");
        assert_eq!(disabled.len(), active.len());
        for (i, bg) in [(0, 10), (6, 9), (7, 9), (8, 1), (13, 12), (14, 9), (15, 9), (19, 18), (20, 9)] {
            assert_eq!(disabled[i], mix(active[i], active[bg]));
        }
    }

    #[test]
    fn kde_contains_complete_sections_and_valid_colors_for_dark_and_light() {
        for background in ["#101010", "#eeeeee"] {
            let mut t = theme();
            t.roles.insert("surface_container".into(), background.into());
            let output = kde(&t, "#123456", "#ffffff");
            let mut sections: BTreeMap<&str, BTreeMap<&str, &str>> = BTreeMap::new();
            let mut section = "";
            for line in output.lines() {
                if line.starts_with('[') {
                    section = line;
                } else if let Some((k, v)) = line.split_once('=') {
                    sections.entry(section).or_default().insert(k, v);
                }
            }
            assert_eq!(sections.len(), 13);
            for name in ["Button", "Complementary", "Header", "Header][Inactive", "Selection", "Tooltip", "View", "Window"] {
                let keys = &sections[format!("[Colors:{name}]").as_str()];
                assert_eq!(keys.len(), 12);
                for key in [
                    "BackgroundNormal",
                    "BackgroundAlternate",
                    "DecorationFocus",
                    "DecorationHover",
                    "ForegroundActive",
                    "ForegroundInactive",
                    "ForegroundLink",
                    "ForegroundNegative",
                    "ForegroundNeutral",
                    "ForegroundNormal",
                    "ForegroundPositive",
                    "ForegroundVisited",
                ] {
                    let value = keys[key];
                    assert!(value.len() == 7 && value.starts_with('#') && value[1..].bytes().all(|b| b.is_ascii_hexdigit()));
                }
                for key in ["ForegroundNeutral", "ForegroundPositive"] {
                    let a = luminance(keys[key]);
                    let b = luminance(keys["BackgroundNormal"]);
                    assert!((a.max(b) + 0.05) / (a.min(b) + 0.05) >= 4.5);
                }
            }
            assert_eq!(sections["[General]"]["Name"], "Digit");
            assert_eq!(sections["[General]"]["ColorScheme"], "Digit");
            assert_eq!(sections["[KDE]"]["contrast"], "4");
            for (section, expected) in [
                (
                    "[ColorEffects:Inactive]",
                    vec![
                        ("ChangeSelectionColor", "true"),
                        ("ColorAmount", "0.025"),
                        ("ColorEffect", "0"),
                        ("ContrastAmount", "0.1"),
                        ("ContrastEffect", "0"),
                        ("Enable", "true"),
                    ],
                ),
                (
                    "[ColorEffects:Disabled]",
                    vec![
                        ("ColorAmount", "0.5"),
                        ("ColorEffect", "3"),
                        ("ContrastAmount", "0"),
                        ("ContrastEffect", "0"),
                        ("Enable", "false"),
                    ],
                ),
            ] {
                for (key, value) in expected {
                    assert_eq!(sections[section][key], value);
                }
                let color = sections[section]["Color"];
                assert!(color.len() == 7 && color.starts_with('#') && color[1..].bytes().all(|b| b.is_ascii_hexdigit()));
            }
            assert_eq!(sections["[WM]"].len(), 6);
            for key in ["activeBackground", "activeForeground", "inactiveBackground", "inactiveForeground", "activeBlend", "inactiveBlend"]
            {
                let channels: Vec<_> = sections["[WM]"][key].split(',').collect();
                assert_eq!(channels.len(), 3);
                assert!(channels.iter().all(|v| v.parse::<u8>().is_ok()));
            }
            assert_eq!(sections["[Colors:Window]"]["BackgroundNormal"], background);
            assert_eq!(sections["[Colors:Selection]"]["BackgroundNormal"], "#123456");
            assert_eq!(sections["[Colors:Selection]"]["ForegroundNormal"], "#ffffff");
        }
    }

    #[test]
    fn discovery_filters_platform_deduplicates_and_prefers_user_names() {
        let root = std::env::temp_dir().join(format!("digit-qt-schemes-{}", std::process::id()));
        for (p, text) in [
            ("config/qt6ct/colors/Z.conf", ""),
            ("share/qt6ct/colors/A.conf", ""),
            ("config/qt6ct/colors/digit.conf", ""),
            ("share/color-schemes/Digit.colors", "[General]\nName=Digit\n"),
            ("data/color-schemes/User.colors", "[Other]\nName=Wrong\n[General]\nName = User name\n"),
            ("share/color-schemes/Fallback.colors", ""),
            ("share/qt6ct/colors/ignore.colors", ""),
        ] {
            atomic_write(&root.join(p), text.as_bytes()).unwrap();
        }
        // A system symlink to the user file has the same canonical identifier.
        std::os::unix::fs::symlink(root.join("data/color-schemes/User.colors"), root.join("share/color-schemes/Alias.colors")).unwrap();
        std::os::unix::fs::symlink(root.join("config/qt6ct/colors/Z.conf"), root.join("share/qt6ct/colors/SystemAlias.conf")).unwrap();
        let discover =
            |platform| schemes_in(root.join("config"), root.join("data"), vec![root.join("share"), root.join("share")], platform);
        for platform in [QtPlatform::Missing, QtPlatform::Qt6ct] {
            assert_eq!(discover(platform).iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["A", "Z"]);
        }
        let all = discover(QtPlatform::Kde);
        assert_eq!(all.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["A", "Fallback", "User name", "Z"]);
        assert!(all.iter().all(|s| std::path::Path::new(&s.id).is_absolute()));
        assert_eq!(effective(Some(&all[0].id), &all), all[0].id);
        assert_eq!(effective(Some("/removed"), &all), "digit");
        assert_eq!(effective(Some("style"), &all), "style");
        assert_eq!(effective(None, &all), "digit");
        std::fs::remove_dir_all(root).unwrap();
    }
}
