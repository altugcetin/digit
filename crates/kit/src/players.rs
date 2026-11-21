//! Which media players the notch, the Now playing widget and the media keys may use.
//! `media_filter` is "off" (every player), "block" (every player except `media_blocked`) or
//! "allow" (only `media_allowed`). Each list keeps its names while the other mode is chosen.
//!
//! A name matches a player whose desktop entry, bus name or app name has it as whole words,
//! ignoring case: "firefox" matches "org.mozilla.firefox" and "firefox.instance_1_501", but
//! "fox" matches neither, and "chrome" doesn't match "chromium".

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
        Self::new(&mode(t), &crate::prefs::list(t, BLOCKED), &crate::prefs::list(t, ALLOWED))
    }

    /// Whether a player known by these names (desktop entry, bus name, app name) may show.
    /// "Only" with an empty list shows every player rather than none.
    pub fn shows(&self, names: &[&str]) -> bool {
        let hit = |list: &[String]| names.iter().any(|n| list.iter().any(|l| names_match(n, l)));
        match self.mode.as_str() {
            "block" => !hit(&self.blocked),
            "allow" => self.allowed.is_empty() || hit(&self.allowed),
            _ => true,
        }
    }

    /// Whether some players may be kept out: a mode is chosen and its list has names.
    pub fn narrows(&self) -> bool {
        match self.mode.as_str() {
            "block" => !self.blocked.is_empty(),
            "allow" => !self.allowed.is_empty(),
            _ => false,
        }
    }
}

/// The words of a name: "org.mozilla.firefox" is org, mozilla and firefox.
fn words(name: &str) -> Vec<String> {
    name.to_lowercase().split(['.', '_', '-', ' ']).filter(|w| !w.is_empty()).map(str::to_string).collect()
}

/// Whether the typed name `typed` is whole words of the player name `name`, in a row.
fn names_match(name: &str, typed: &str) -> bool {
    let (name, typed) = (words(name), words(typed));
    !typed.is_empty() && name.windows(typed.len()).any(|w| w == typed.as_slice())
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
    fn a_name_matches_whole_words_only() {
        let f = Filter::new("block", &["fox".into(), "a".into(), "chrome".into()], &[]);
        assert!(f.shows(&["org.mozilla.firefox", "firefox.instance_1_501", "Firefox"]));
        assert!(f.shows(&["spotify", "spotify", "Spotify"]));
        assert!(f.shows(&["chromium", "chromium.instance42", "Chromium"]));
        assert!(!f.shows(&["google-chrome", "chrome.instance7", "Google Chrome"]));
        // Several words in a row, as typed, with any separator.
        let f = Filter::new("allow", &[], &["mozilla firefox".into()]);
        assert!(f.shows(&["org.mozilla.firefox", "", ""]));
        assert!(!f.shows(&["org.mozilla.thunderbird", "", ""]));
    }

    #[test]
    fn only_a_mode_with_names_narrows() {
        assert!(!Filter::read(&table("media_blocked = [\"spotify\"]")).narrows());
        assert!(!Filter::read(&table("media_filter = \"allow\"")).narrows());
        assert!(Filter::read(&table("media_filter = \"block\"\nmedia_blocked = [\"spotify\"]")).narrows());
        assert!(Filter::read(&table("media_filter = \"allow\"\nmedia_allowed = [\"mpv\"]")).narrows());
    }

    #[test]
    fn typed_lists_are_tidied() {
        assert_eq!(parse(" Spotify, ,firefox,spotify ,"), ["spotify", "firefox"]);
        assert_eq!(join(&parse("mpv,VLC")), "mpv, vlc");
        assert!(parse("  ").is_empty());
    }
}
