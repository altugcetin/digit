//! Apps kept out of the app grid and Spotlight. Developer and system tools that come with other
//! software (Qt's tools, lstopo, the JDK's console) are hidden until the user shows them;
//! Settings > Apps hides or shows any app. Stored as `apps_hidden` (hidden by the user) and
//! `apps_shown` (hidden by default, shown by the user), by desktop file id.

/// Desktop file ids, without ".desktop", hidden unless shown: exactly these, or starting with
/// one that ends in '-'.
const BY_DEFAULT: &[&str] = &[
    "assistant", "designer", "linguist", "qdbusviewer", "qv4l2", "qvidcap", "assistant-qt5", "designer-qt5", "linguist-qt5", "qdbusviewer-qt5",
    "lstopo", "jconsole-", "jshell-", "avahi-discover", "bssh", "bvnc", "xterm", "uxterm", "vim", "nvim", "gvim",
];

/// Whether app `id` ("lstopo.desktop") is hidden unless the user shows it.
pub fn by_default(id: &str) -> bool {
    let stem = id.strip_suffix(".desktop").unwrap_or(id);
    BY_DEFAULT.iter().any(|d| if let Some(prefix) = d.strip_suffix('-') { stem.starts_with(&format!("{prefix}-")) } else { stem == *d })
}

/// Whether app `id` is hidden, given the user's hidden and shown lists.
pub fn hidden(id: &str, hidden: &[String], shown: &[String]) -> bool {
    hidden.iter().any(|h| h == id) || (by_default(id) && !shown.iter().any(|s| s == id))
}

/// The ids in a preferences list.
pub fn list(t: &toml::Table, key: &str) -> Vec<String> {
    t.get(key).and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// The preference changes that hide or show app `id`, given the current table.
pub fn change(t: &toml::Table, id: &str, hide: bool) -> Vec<(&'static str, toml::Value)> {
    let mut hidden = list(t, "apps_hidden");
    let mut shown = list(t, "apps_shown");
    hidden.retain(|h| h != id);
    shown.retain(|s| s != id);
    if hide && !by_default(id) {
        hidden.push(id.to_string());
    }
    if !hide && by_default(id) {
        shown.push(id.to_string());
    }
    let arr = |v: Vec<String>| toml::Value::Array(v.into_iter().map(toml::Value::String).collect());
    vec![("apps_hidden", arr(hidden)), ("apps_shown", arr(shown))]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_start_hidden_and_the_user_has_the_last_word() {
        assert!(by_default("lstopo.desktop") && by_default("jshell-java25-openjdk.desktop") && by_default("qv4l2.desktop"));
        assert!(!by_default("firefox.desktop") && !by_default("jshellfish.desktop") && !by_default("designerapp.desktop"));
        let none: Vec<String> = Vec::new();
        assert!(hidden("lstopo.desktop", &none, &none));
        assert!(!hidden("lstopo.desktop", &none, &["lstopo.desktop".into()]));
        assert!(hidden("firefox.desktop", &["firefox.desktop".into()], &none));
        let mut t = toml::Table::new();
        for (k, v) in change(&t, "lstopo.desktop", false).into_iter().chain(change(&t, "firefox.desktop", true)) {
            let merged: Vec<toml::Value> = t.get(k).and_then(|x| x.as_array()).cloned().unwrap_or_default().into_iter().chain(v.as_array().cloned().unwrap_or_default()).collect();
            t.insert(k.into(), toml::Value::Array(merged));
        }
        assert_eq!(list(&t, "apps_shown"), ["lstopo.desktop"]);
        assert_eq!(list(&t, "apps_hidden"), ["firefox.desktop"]);
        let back = change(&t, "firefox.desktop", false);
        assert!(back[0].1.as_array().unwrap().is_empty());
    }
}
