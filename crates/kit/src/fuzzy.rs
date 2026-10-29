pub fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase) {
        let c = match c {
            'ı' | 'î' | 'ï' | 'í' | 'ì' => 'i',
            'ş' | 'ś' | 'š' => 's',
            'ğ' => 'g',
            'ü' | 'û' | 'ú' | 'ù' => 'u',
            'ö' | 'ô' | 'ó' | 'ò' | 'ø' => 'o',
            'ç' | 'ć' | 'č' => 'c',
            'â' | 'á' | 'à' | 'ä' | 'ã' | 'å' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'ñ' => 'n',
            '\u{300}'..='\u{36f}' => continue,
            c => c,
        };
        out.push(c);
    }
    out
}

pub fn score(query: &str, hay: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    if hay == query {
        return Some(1000);
    }
    if hay.starts_with(query) {
        return Some(900 - (hay.len() as i32 - query.len() as i32).min(100));
    }
    let boundary = hay.char_indices().any(|(i, _)| {
        let before = hay[..i].chars().next_back();
        (i == 0 || before.is_some_and(|b| !b.is_alphanumeric())) && hay[i..].starts_with(query)
    });
    if boundary {
        return Some(780 - (hay.len() as i32).min(80));
    }
    if hay.contains(query) {
        return Some(600 - (hay.len() as i32).min(80));
    }
    let mut chars = hay.chars().peekable();
    let mut gaps = 0;
    let mut started = false;
    for q in query.chars() {
        let mut found = false;
        for h in chars.by_ref() {
            if h == q {
                found = true;
                break;
            }
            if started {
                gaps += 1;
            }
        }
        if !found {
            return None;
        }
        started = true;
    }
    (gaps <= query.len() as i32 * 3).then_some(400 - gaps * 8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folding() {
        assert_eq!(fold("Dosyalar Şİİr ĞÜÇÖ"), "dosyalar siir guco");
    }

    #[test]
    fn ranking() {
        let s = |q: &str, h: &str| score(&fold(q), &fold(h));
        assert!(s("fire", "Firefox").unwrap() > s("fire", "Bonfire Editor").unwrap());
        assert!(s("edit", "Text Editor").unwrap() > s("edit", "Credits").unwrap());
        assert!(s("ffx", "Firefox").is_some());
        assert!(s("zzz", "Firefox").is_none());
        assert!(s("ayar", "Ayarlar").unwrap() > 800);
    }
}
