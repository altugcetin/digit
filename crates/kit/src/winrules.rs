//! Window rules from Settings > Window rules: how chosen apps' windows open, written into
//! Hyprland's and niri's part of digit's compositor config.

/// One rule: the windows it matches and what they open as.
#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    /// The app ID (Wayland app-id, Hyprland class) the window must have, exactly.
    pub app: String,
    /// Words the window title must contain; empty matches any title.
    pub title: String,
    pub float: bool,
    pub maximize: bool,
    pub fullscreen: bool,
    /// 1.0 leaves the window opaque.
    pub opacity: f64,
}

impl Default for Rule {
    fn default() -> Self {
        Self { app: String::new(), title: String::new(), float: false, maximize: false, fullscreen: false, opacity: 1.0 }
    }
}

impl Rule {
    /// A rule matching nothing or doing nothing isn't written.
    pub fn active(&self) -> bool {
        (!self.app.trim().is_empty() || !self.title.trim().is_empty()) && (self.float || self.maximize || self.fullscreen || self.opacity < 0.995)
    }
}

/// The rules saved under `window_rules`.
pub fn read(t: &toml::Table) -> Vec<Rule> {
    let Some(list) = t.get("window_rules").and_then(toml::Value::as_array) else { return Vec::new() };
    list.iter()
        .filter_map(toml::Value::as_table)
        .map(|r| {
            let text = |k: &str| r.get(k).and_then(toml::Value::as_str).unwrap_or_default().to_string();
            let flag = |k: &str| r.get(k).and_then(toml::Value::as_bool).unwrap_or(false);
            let opacity = r.get("opacity").and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64))).unwrap_or(1.0);
            Rule { app: text("app"), title: text("title"), float: flag("float"), maximize: flag("maximize"), fullscreen: flag("fullscreen"), opacity: opacity.clamp(0.1, 1.0) }
        })
        .collect()
}

/// The rules as the value saved under `window_rules`.
pub fn value(rules: &[Rule]) -> toml::Value {
    toml::Value::Array(
        rules
            .iter()
            .map(|r| {
                let mut t = toml::Table::new();
                t.insert("app".into(), toml::Value::String(r.app.trim().into()));
                t.insert("title".into(), toml::Value::String(r.title.trim().into()));
                t.insert("float".into(), toml::Value::Boolean(r.float));
                t.insert("maximize".into(), toml::Value::Boolean(r.maximize));
                t.insert("fullscreen".into(), toml::Value::Boolean(r.fullscreen));
                t.insert("opacity".into(), toml::Value::Float((r.opacity * 100.0).round() / 100.0));
                toml::Value::Table(t)
            })
            .collect(),
    )
}

/// `text` as a regular expression matching it literally.
fn literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.trim().chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The app ID matched exactly, the title by containing the words.
fn patterns(r: &Rule) -> (Option<String>, Option<String>) {
    let app = (!r.app.trim().is_empty()).then(|| format!("^{}$", literal(&r.app)));
    let title = (!r.title.trim().is_empty()).then(|| literal(&r.title));
    (app, title)
}

/// Hyprland's Lua rules.
pub fn lua(rules: &[Rule]) -> String {
    let mut out = String::new();
    for (i, r) in rules.iter().filter(|r| r.active()).enumerate() {
        let (app, title) = patterns(r);
        let mut matches = Vec::new();
        if let Some(app) = app {
            matches.push(format!("class = {}", crate::hyprconf::lua_str(&app)));
        }
        if let Some(title) = title {
            matches.push(format!("title = {}", crate::hyprconf::lua_str(&title)));
        }
        let mut effects = Vec::new();
        if r.float {
            effects.push("float = true".to_string());
        }
        if r.maximize {
            effects.push("maximize = true".to_string());
        }
        if r.fullscreen {
            effects.push("fullscreen = true".to_string());
        }
        if r.opacity < 0.995 {
            effects.push(format!("opacity = {:.2}", r.opacity));
        }
        out.push_str(&format!("hl.window_rule({{ name = \"digit-user-{}\", match = {{ {} }}, {} }})\n", i + 1, matches.join(", "), effects.join(", ")));
    }
    if !out.is_empty() {
        out.insert_str(0, "\n-- Settings > Window rules\n");
    }
    out
}

/// niri's window rules.
pub fn kdl(rules: &[Rule]) -> String {
    // A raw string can't hold the quote-and-hash that ends it.
    let raw = |s: &str| format!("r#\"{}\"#", s.replace("\"#", "\"\\#"));
    let mut out = String::new();
    for r in rules.iter().filter(|r| r.active()) {
        let (app, title) = patterns(r);
        let mut matches = Vec::new();
        if let Some(app) = app {
            matches.push(format!("app-id={}", raw(&app)));
        }
        if let Some(title) = title {
            matches.push(format!("title={}", raw(&title)));
        }
        out.push_str(&format!("\nwindow-rule {{\n    match {}\n", matches.join(" ")));
        if r.float {
            out.push_str("    open-floating true\n");
        }
        if r.maximize {
            out.push_str("    open-maximized true\n");
        }
        if r.fullscreen {
            out.push_str("    open-fullscreen true\n");
        }
        if r.opacity < 0.995 {
            out.push_str(&format!("    opacity {:.2}\n", r.opacity));
        }
        out.push_str("}\n");
    }
    if !out.is_empty() {
        out.insert_str(0, "\n// Settings > Window rules\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn firefox() -> Rule {
        Rule { app: "org.mozilla.firefox".into(), title: "Picture-in-Picture".into(), float: true, opacity: 0.9, ..Rule::default() }
    }

    #[test]
    fn rules_round_trip_through_preferences() {
        let rules = vec![firefox(), Rule { app: "mpv".into(), fullscreen: true, ..Rule::default() }];
        let mut t = toml::Table::new();
        t.insert("window_rules".into(), value(&rules));
        assert_eq!(read(&t), rules);
    }

    #[test]
    fn empty_or_effectless_rules_are_not_written() {
        let idle = [Rule::default(), Rule { app: "mpv".into(), ..Rule::default() }, Rule { float: true, ..Rule::default() }];
        assert!(lua(&idle).is_empty());
        assert!(kdl(&idle).is_empty());
    }

    #[test]
    fn hyprland_matches_the_app_exactly_and_the_title_literally() {
        let text = lua(&[firefox()]);
        assert!(text.contains(r#"class = "^org\\.mozilla\\.firefox$""#), "{text}");
        assert!(text.contains(r#"title = "Picture\\-in\\-Picture""#), "{text}");
        assert!(text.contains("float = true") && text.contains("opacity = 0.90"));
    }

    #[test]
    fn niri_writes_one_window_rule_per_rule() {
        let text = kdl(&[firefox(), Rule { app: "mpv".into(), maximize: true, ..Rule::default() }]);
        assert_eq!(text.matches("window-rule {").count(), 2);
        assert!(text.contains(r##"match app-id=r#"^org\.mozilla\.firefox$"# title=r#"Picture\-in\-Picture"#"##), "{text}");
        assert!(text.contains("open-floating true") && text.contains("opacity 0.90") && text.contains("open-maximized true"));
    }
}
