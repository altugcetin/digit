//! Emoji for Spotlight: the names and keywords GTK's own emoji chooser ships, searched by words,
//! and a short list of the ones used last.

use gtk::{gio, glib};
use kit::fuzzy::{fold, score};
use std::cell::RefCell;
use std::rc::Rc;

const RECENT_MAX: usize = 24;

pub struct Emoji {
    pub text: String,
    pub name: String,
    /// Name and keywords, folded for matching.
    words: String,
}

type Raw = Vec<(Vec<u32>, String, String, Vec<String>, u32)>;

fn raw(lang: &str) -> Option<Raw> {
    let bytes = gio::resources_lookup_data(&format!("/org/gtk/libgtk/emoji/{lang}.data"), gio::ResourceLookupFlags::NONE).ok()?;
    glib::Variant::from_bytes::<Raw>(&bytes).get::<Raw>()
}

/// A skin tone modifier; GTK lists an emoji with one in it, and the plain emoji is that without.
fn tone(c: u32) -> bool {
    (0x1F3FB..=0x1F3FF).contains(&c)
}

/// A hair style variant of another emoji.
fn variant(codes: &[u32]) -> bool {
    codes.iter().any(|c| (0x1F9B0..=0x1F9B3).contains(c))
}

fn load() -> Vec<Emoji> {
    let own = glib::language_names().iter().map(|l| l.as_str().split(['.', '@']).next().unwrap_or("").to_string()).find_map(|l| {
        let short = l.split('_').next().unwrap_or("").to_string();
        [l, short].into_iter().find(|n| !n.is_empty() && n != "en" && n != "C" && raw(n).is_some())
    });
    let english = raw("en").unwrap_or_default();
    let local = own.and_then(|l| raw(&l)).unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    english
        .into_iter()
        .filter(|(codes, ..)| !variant(codes))
        .filter_map(|(codes, name, _, keys, _)| {
            let text: String = codes.iter().filter(|c| **c != 0 && !tone(**c)).filter_map(|c| char::from_u32(*c)).collect();
            if text.is_empty() || !seen.insert(text.clone()) {
                return None;
            }
            let (shown, extra) = match local.iter().find(|l| l.0 == codes) {
                Some((_, n, _, k, _)) => (n.clone(), format!("{n} {}", k.join(" "))),
                None => (name.clone(), String::new()),
            };
            let words = fold(&format!("{shown} {name} {} {extra}", keys.join(" ")));
            Some(Emoji { text, name: shown, words })
        })
        .collect()
}

thread_local! {
    static ALL: RefCell<Option<Rc<Vec<Emoji>>>> = const { RefCell::new(None) };
}

pub fn all() -> Rc<Vec<Emoji>> {
    ALL.with(|a| a.borrow_mut().get_or_insert_with(|| Rc::new(load())).clone())
}

fn recent_file() -> std::path::PathBuf {
    kit::theme::state_dir().join("emoji.json")
}

pub fn recent() -> Vec<String> {
    std::fs::read_to_string(recent_file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn remember(text: &str) {
    let mut list = recent();
    list.retain(|t| t != text);
    list.insert(0, text.to_string());
    list.truncate(RECENT_MAX);
    let _ = kit::atomic::write(&recent_file(), serde_json::to_string(&list).unwrap_or_default());
}

/// How well `emoji` answers `query` (already folded): every word of the query has to hit, a
/// word that starts the name counts most.
fn rank(e: &Emoji, name: &str, tokens: &[&str]) -> Option<i32> {
    let mut total = 0;
    for t in tokens {
        let word = e.words.split_whitespace().filter_map(|w| score(t, w)).max();
        let fuzzy = (t.chars().count() >= 3).then(|| score(t, name)).flatten().map(|s| s - 250);
        total += word.into_iter().chain(fuzzy).max().filter(|s| *s >= 500)?;
    }
    Some(total / tokens.len() as i32 - (e.name.len() as i32).min(60))
}

/// The best `limit` emoji for `query`; with nothing typed, the ones used last, then all of them.
pub fn find(query: &str, limit: usize) -> Vec<(String, String, i32)> {
    let list = all();
    let q = fold(query.trim().trim_matches(':'));
    if q.is_empty() {
        let used = recent();
        let first = used.iter().filter_map(|t| list.iter().find(|e| &e.text == t));
        let rest = list.iter().filter(|e| !used.contains(&e.text));
        return first.chain(rest).take(limit).enumerate().map(|(i, e)| (e.text.clone(), e.name.clone(), 100_000 - i as i32)).collect();
    }
    let tokens: Vec<&str> = q.split_whitespace().collect();
    let mut hits: Vec<(usize, i32)> = list.iter().enumerate().filter_map(|(i, e)| rank(e, &fold(&e.name), &tokens).map(|s| (i, s))).collect();
    hits.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    hits.into_iter().take(limit).map(|(i, s)| (list[i].text.clone(), list[i].name.clone(), s)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hair_variants_are_left_out_and_tones_are_stripped() {
        assert!(variant(&[0x1F469, 0x200D, 0x1F9B0]));
        assert!(!variant(&[0x1F44D, 0x1F3FD]));
        assert!(tone(0x1F3FD) && !tone(0x1F44D));
    }

    #[test]
    fn words_find_emoji_when_gtk_has_its_data() {
        if gtk::init().is_err() || raw("en").is_none() {
            return;
        }
        let hits = find("grinning", 3);
        assert!(hits.iter().any(|(text, ..)| text == "😀"));
        assert!(find("thumbs up", 5).iter().any(|(text, ..)| text == "👍"));
        assert!(find("zzzzqqqq", 3).is_empty());
        assert!(find("", 3).len() <= 3);
    }
}
