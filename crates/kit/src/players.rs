//! Which media players the notch may show. `media_filter` is "off" (every player), "block"
//! (every player except `media_blocked`) or "allow" (only `media_allowed`). Each list keeps its
//! names while the other mode is chosen. A name matches a player whose desktop entry, bus name
//! or app name contains it, ignoring case: "firefox" matches "org.mozilla.firefox".

pub const MODE: &str = "media_filter";
pub const BLOCKED: &str = "media_blocked";
pub const ALLOWED: &str = "media_allowed";
pub const MODES: [(&str, &str); 3] = [("off", "All players"), ("block", "All except"), ("allow", "Only")];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filter {
    mode: String,
    blocked: Vec<String>,
    allowed: Vec<String>,
}

impl Filter {
    pub fn new(mode: &str, blocked: &[String], allowed: &[String]) -> Self {
        Filter { mode: mode.to_string(), blocked: tidy(blocked), allowed: tidy(allowed) }
    }

    pub fn read(t: &toml::Table) -> Self {
        Self::new(&mode(t), &crate::hidden::list(t, BLOCKED), &crate::hidden::list(t, ALLOWED))
    }

    /// Whether a player known by these names (desktop entry, bus name, app name) may show.
    /// "Only" with an empty list shows every player rather than none.
    pub fn shows(&self, names: &[&str]) -> bool {
        let hit = |list: &[String]| names.iter().map(|n| n.to_lowercase()).any(|n| list.iter().any(|l| n.contains(l.as_str())));
        match self.mode.as_str() {
            "block" => !hit(&self.blocked),
            "allow" => self.allowed.is_empty() || hit(&self.allowed),
            _ => true,
        }
    }
}

/// The chosen mode, "off" when unset or unknown.
pub fn mode(t: &toml::Table) -> String {
    let m = t.get(MODE).and_then(|v| v.as_str()).unwrap_or("off");
    if MODES.iter().any(|(k, _)| *k == m) { m.to_string() } else { "off".to_string() }
}

/// Names typed as "Spotify, firefox": trimmed, lowercased, without blanks or repeats.
pub fn parse(text: &str) -> Vec<String> {
    tidy(&text.split(',').map(str::to_string).collect::<Vec<_>>())
}

/// A list as Settings shows it.
pub fn join(list: &[String]) -> String {
    list.join(", ")
}

fn tidy(list: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for name in list.iter().map(|n| n.trim().to_lowercase()).filter(|n| !n.is_empty()) {
        if !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(text: &str) -> toml::Table {
        text.parse().unwrap()
    }

    #[test]
    fn every_player_shows_until_a_mode_is_chosen() {
        let f = Filter::read(&table("media_blocked = [\"spotify\"]"));
        assert!(f.shows(&["spotify", "org.mpris.MediaPlayer2.spotify", "Spotify"]));
        assert!(Filter::read(&table("media_filter = \"nonsense\"\nmedia_blocked = [\"spotify\"]")).shows(&["spotify"]));
    }

    #[test]
    fn blocked_players_never_show_and_matching_ignores_case() {
        let f = Filter::read(&table("media_filter = \"block\"\nmedia_blocked = [\"Firefox\"]\nmedia_allowed = [\"spotify\"]"));
        assert!(!f.shows(&["org.mozilla.firefox", "firefox.instance_1_501", "Firefox"]));
        assert!(f.shows(&["spotify", "spotify", "Spotify"]));
    }

    #[test]
    fn only_allowed_players_show_and_an_empty_list_hides_nothing() {
        let f = Filter::read(&table("media_filter = \"allow\"\nmedia_allowed = [\"spotify\"]\nmedia_blocked = [\"spotify\"]"));
        assert!(f.shows(&["", "spotify", "Spotify"]));
        assert!(!f.shows(&["org.mozilla.firefox", "firefox", "Firefox"]));
        assert!(Filter::read(&table("media_filter = \"allow\"")).shows(&["firefox"]));
    }

    #[test]
    fn typed_lists_are_tidied() {
        assert_eq!(parse(" Spotify, ,firefox,spotify ,"), ["spotify", "firefox"]);
        assert_eq!(join(&parse("mpv,VLC")), "mpv, vlc");
        assert!(parse("  ").is_empty());
    }
}
