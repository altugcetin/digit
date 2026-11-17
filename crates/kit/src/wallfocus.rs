//! Where a wallpaper that fills a screen is cut. The image is scaled to cover the screen and
//! the part that overflows is cropped; the focus says which part stays: 0.5, 0.5 is the middle,
//! 0 the top or left edge and 1 the bottom or right edge. Settings writes it and the shell reads
//! it, so the key, the default and the maths live here.

/// The table in the preferences: one focus per connector, and one for a wallpaper that spans.
pub const KEY: &str = "wallpaper_focus";
/// The key of a wallpaper spanning all screens.
pub const SPAN: &str = "span";

pub type Focus = (f64, f64);

pub const CENTER: Focus = (0.5, 0.5);

/// Keeps a focus in range, on a grid of hundredths so equal choices compare and cache equal.
pub fn clamp(f: Focus) -> Focus {
    let q = |v: f64| if v.is_finite() { (v.clamp(0.0, 1.0) * 100.0).round() / 100.0 } else { 0.5 };
    (q(f.0), q(f.1))
}

/// The focus saved for `key` (a connector name, or [`SPAN`]); the middle when there is none.
pub fn read(prefs: &toml::Table, key: &str) -> Focus {
    let pair = prefs.get(KEY).and_then(toml::Value::as_table).and_then(|t| t.get(key)).and_then(toml::Value::as_array);
    let at = |i: usize| pair.and_then(|a| a.get(i)).and_then(|v| v.as_float().or_else(|| v.as_integer().map(|n| n as f64)));
    match (at(0), at(1)) {
        (Some(x), Some(y)) => clamp((x, y)),
        _ => CENTER,
    }
}

/// Saves the focus for `key`; the middle removes the entry. Other keys, including those of
/// screens that are unplugged now, are kept.
pub fn save(key: &str, focus: Focus) -> Result<(), String> {
    let focus = clamp(focus);
    let mut table = crate::prefs::read().get(KEY).and_then(toml::Value::as_table).cloned().unwrap_or_default();
    if focus == CENTER {
        table.remove(key);
    } else {
        table.insert(key.into(), toml::Value::Array(vec![toml::Value::Float(focus.0), toml::Value::Float(focus.1)]));
    }
    if table.is_empty() { crate::prefs::unset(KEY) } else { crate::prefs::set(KEY, toml::Value::Table(table)) }
}

/// The part of an image of `image` size that covers `target`, as fractions of the image:
/// (x, y, w, h). One of w and h is 1.
pub fn crop(image: (f64, f64), target: (f64, f64), focus: Focus) -> (f64, f64, f64, f64) {
    let (iw, ih) = (image.0.max(1.0), image.1.max(1.0));
    let k = (target.0 / iw).max(target.1 / ih).max(f64::EPSILON);
    let w = (target.0 / (iw * k)).clamp(0.0, 1.0);
    let h = (target.1 / (ih * k)).clamp(0.0, 1.0);
    let focus = clamp(focus);
    ((1.0 - w) * focus.0, (1.0 - h) * focus.1, w, h)
}

/// A suffix that tells cached crops apart; empty for the middle, so older caches stay valid.
pub fn tag(focus: Focus) -> String {
    let f = clamp(focus);
    if f == CENTER { String::new() } else { format!("-f{:.2}x{:.2}", f.0, f.1) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_middle_is_the_default_and_values_stay_in_range() {
        assert_eq!(read(&toml::Table::new(), "DP-1"), CENTER);
        assert_eq!(clamp((-1.0, 2.0)), (0.0, 1.0));
        assert_eq!(clamp((f64::NAN, 0.123)), (0.5, 0.12));
        let t: toml::Table = "[wallpaper_focus]\nDP-1 = [0.5, 0.0]\nbad = [1]\n".parse().unwrap();
        assert_eq!(read(&t, "DP-1"), (0.5, 0.0));
        assert_eq!(read(&t, "bad"), CENTER);
        assert_eq!(read(&t, "HDMI-A-1"), CENTER);
    }

    #[test]
    fn a_crop_moves_only_along_the_side_that_overflows() {
        // A wide image on a square screen overflows sideways.
        assert_eq!(crop((4000.0, 2000.0), (1000.0, 1000.0), CENTER), (0.25, 0.0, 0.5, 1.0));
        assert_eq!(crop((4000.0, 2000.0), (1000.0, 1000.0), (0.0, 0.7)), (0.0, 0.0, 0.5, 1.0));
        assert_eq!(crop((4000.0, 2000.0), (1000.0, 1000.0), (1.0, 0.0)), (0.5, 0.0, 0.5, 1.0));
        // A tall one overflows up and down: 0 keeps the top.
        assert_eq!(crop((2000.0, 4000.0), (1000.0, 1000.0), (0.3, 0.0)), (0.0, 0.0, 1.0, 0.5));
        assert_eq!(crop((2000.0, 4000.0), (1000.0, 1000.0), (0.3, 1.0)), (0.0, 0.5, 1.0, 0.5));
    }

    #[test]
    fn tags_tell_crops_apart_and_leave_the_middle_alone() {
        assert_eq!(tag(CENTER), "");
        assert_ne!(tag((0.5, 0.0)), tag((0.5, 1.0)));
    }
}
