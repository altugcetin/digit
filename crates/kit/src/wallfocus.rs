//! Where a wallpaper that fills a screen is cut. The image is scaled to cover the screen, and
//! further by the zoom, and the part that overflows is cropped; the focus says which part stays:
//! 0.5, 0.5 is the middle, 0 the top or left edge and 1 the bottom or right edge. Settings writes
//! it and the shell reads it, so the key, the default and the maths live here.

/// The table in the preferences: one focus per connector, and one for a wallpaper that spans.
pub const KEY: &str = "wallpaper_focus";
/// The key of a wallpaper spanning all screens.
pub const SPAN: &str = "span";

/// Across, down, and the zoom: 1 just covers the screen, 2 shows half as much of the picture.
pub type Focus = (f64, f64, f64);

pub const CENTER: Focus = (0.5, 0.5, 1.0);

/// How far a picture can be zoomed in.
pub const MAX_ZOOM: f64 = 4.0;

/// Keeps a focus in range, on a grid of hundredths so equal choices compare and cache equal.
pub fn clamp(f: Focus) -> Focus {
    let q = |v: f64| if v.is_finite() { (v.clamp(0.0, 1.0) * 100.0).round() / 100.0 } else { 0.5 };
    let z = if f.2.is_finite() { (f.2.clamp(1.0, MAX_ZOOM) * 100.0).round() / 100.0 } else { 1.0 };
    (q(f.0), q(f.1), z)
}

/// The focus saved for `key` (a connector name, or [`SPAN`]); the middle when there is none.
pub fn read(prefs: &toml::Table, key: &str) -> Focus {
    let pair = prefs.get(KEY).and_then(toml::Value::as_table).and_then(|t| t.get(key)).and_then(toml::Value::as_array);
    let at = |i: usize| pair.and_then(|a| a.get(i)).and_then(|v| v.as_float().or_else(|| v.as_integer().map(|n| n as f64)));
    match (at(0), at(1)) {
        (Some(x), Some(y)) => clamp((x, y, at(2).unwrap_or(1.0))),
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
        let mut pair = vec![toml::Value::Float(focus.0), toml::Value::Float(focus.1)];
        // Only a zoom is written as a third number, so older files read the same.
        if focus.2 != 1.0 {
            pair.push(toml::Value::Float(focus.2));
        }
        table.insert(key.into(), toml::Value::Array(pair));
    }
    if table.is_empty() { crate::prefs::unset(KEY) } else { crate::prefs::set(KEY, toml::Value::Table(table)) }
}

/// The part of an image of `image` size that covers `target`, as fractions of the image:
/// (x, y, w, h). Without a zoom one of w and h is 1.
pub fn crop(image: (f64, f64), target: (f64, f64), focus: Focus) -> (f64, f64, f64, f64) {
    let (iw, ih) = (image.0.max(1.0), image.1.max(1.0));
    let focus = clamp(focus);
    let k = (target.0 / iw).max(target.1 / ih).max(f64::EPSILON) * focus.2;
    let w = (target.0 / (iw * k)).clamp(0.0, 1.0);
    let h = (target.1 / (ih * k)).clamp(0.0, 1.0);
    ((1.0 - w) * focus.0, (1.0 - h) * focus.1, w, h)
}

/// A suffix that tells cached crops apart; empty for the middle, so older caches stay valid.
pub fn tag(focus: Focus) -> String {
    let f = clamp(focus);
    if f == CENTER {
        String::new()
    } else if f.2 == 1.0 {
        format!("-f{:.2}x{:.2}", f.0, f.1)
    } else {
        format!("-f{:.2}x{:.2}z{:.2}", f.0, f.1, f.2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_middle_is_the_default_and_values_stay_in_range() {
        assert_eq!(read(&toml::Table::new(), "DP-1"), CENTER);
        assert_eq!(clamp((-1.0, 2.0, 1.0)), (0.0, 1.0, 1.0));
        assert_eq!(clamp((f64::NAN, 0.123, f64::NAN)), (0.5, 0.12, 1.0));
        assert_eq!(clamp((0.5, 0.5, 0.2)), CENTER, "a zoom never shows less than the screen");
        assert_eq!(clamp((0.5, 0.5, 9.0)).2, MAX_ZOOM);
        let t: toml::Table = "[wallpaper_focus]\nDP-1 = [0.5, 0.0]\nbad = [1]\nHDMI = [0.2, 0.3, 2.5]\n".parse().unwrap();
        assert_eq!(read(&t, "DP-1"), (0.5, 0.0, 1.0));
        assert_eq!(read(&t, "HDMI"), (0.2, 0.3, 2.5));
        assert_eq!(read(&t, "bad"), CENTER);
        assert_eq!(read(&t, "HDMI-A-1"), CENTER);
    }

    #[test]
    fn a_crop_moves_only_along_the_side_that_overflows() {
        // A wide image on a square screen overflows sideways.
        assert_eq!(crop((4000.0, 2000.0), (1000.0, 1000.0), CENTER), (0.25, 0.0, 0.5, 1.0));
        assert_eq!(crop((4000.0, 2000.0), (1000.0, 1000.0), (0.0, 0.7, 1.0)), (0.0, 0.0, 0.5, 1.0));
        assert_eq!(crop((4000.0, 2000.0), (1000.0, 1000.0), (1.0, 0.0, 1.0)), (0.5, 0.0, 0.5, 1.0));
        // A tall one overflows up and down: 0 keeps the top.
        assert_eq!(crop((2000.0, 4000.0), (1000.0, 1000.0), (0.3, 0.0, 1.0)), (0.0, 0.0, 1.0, 0.5));
        assert_eq!(crop((2000.0, 4000.0), (1000.0, 1000.0), (0.3, 1.0, 1.0)), (0.0, 0.5, 1.0, 0.5));
        // Zoomed in twice, half as much shows each way, and the focus moves it along both.
        assert_eq!(crop((4000.0, 2000.0), (1000.0, 1000.0), (0.5, 0.5, 2.0)), (0.375, 0.25, 0.25, 0.5));
        assert_eq!(crop((4000.0, 2000.0), (1000.0, 1000.0), (0.0, 1.0, 2.0)), (0.0, 0.5, 0.25, 0.5));
    }

    #[test]
    fn tags_tell_crops_apart_and_leave_the_middle_alone() {
        assert_eq!(tag(CENTER), "");
        assert_ne!(tag((0.5, 0.0, 1.0)), tag((0.5, 1.0, 1.0)));
        assert_eq!(tag((0.5, 0.0, 1.0)), "-f0.50x0.00", "an older crop keeps its name");
        assert_ne!(tag((0.5, 0.5, 2.0)), tag(CENTER));
    }
}
