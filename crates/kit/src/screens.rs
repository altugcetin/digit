//! The screens GDK knows, and which of them is the main one. Everything that puts something on
//! "the main screen" (the dock, the app grid, the widgets, the lock screen's password) asks here,
//! so they all agree.

use gtk::{gdk, prelude::*};

/// Every screen, in GDK's order.
pub fn all() -> Vec<gdk::Monitor> {
    gdk::Display::default().map(|d| d.monitors().iter::<gdk::Monitor>().filter_map(Result::ok).collect()).unwrap_or_default()
}

/// A screen's connector ("DP-2"), or "" when it has none.
pub fn name(monitor: &gdk::Monitor) -> String {
    monitor.connector().map(|c| c.to_string()).unwrap_or_default()
}

/// The screen called `output`, if it is plugged in.
pub fn by_name(output: &str) -> Option<gdk::Monitor> {
    all().into_iter().find(|m| m.connector().as_deref() == Some(output))
}

/// Of `screens`, the main one: the one chosen in Settings (`chosen`, the `primary_monitor`
/// setting) when it is there, else the one at the origin of the layout, else the first.
pub fn pick<'a, T>(screens: &'a [T], chosen: &str, name: impl Fn(&T) -> String, at_origin: impl Fn(&T) -> bool) -> Option<&'a T> {
    screens
        .iter()
        .find(|s| !chosen.is_empty() && name(s) == chosen)
        .or_else(|| screens.iter().find(|s| at_origin(s)))
        .or(screens.first())
}

/// The main screen, `chosen` being the `primary_monitor` setting.
pub fn primary(chosen: &str) -> Option<gdk::Monitor> {
    let screens = all();
    pick(&screens, chosen, name, |m| {
        let g = m.geometry();
        g.x() == 0 && g.y() == 0
    })
    .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_chosen_screen_then_the_one_at_the_origin_then_the_first() {
        let screens = [("HDMI-A-1", 1920), ("DP-2", 0), ("DP-3", 3840)];
        let pick = |chosen: &str| pick(&screens, chosen, |s| s.0.to_string(), |s| s.1 == 0).map(|s| s.0);
        assert_eq!(pick("DP-3"), Some("DP-3"));
        assert_eq!(pick(""), Some("DP-2"), "nothing chosen: the screen at the origin");
        assert_eq!(pick("eDP-1"), Some("DP-2"), "the chosen screen is unplugged");
        let none_at_origin = [("A", 5), ("B", 9)];
        assert_eq!(super::pick(&none_at_origin, "", |s| s.0.to_string(), |s| s.1 == 0).map(|s| s.0), Some("A"));
        assert_eq!(super::pick::<(&str, i32)>(&[], "A", |s| s.0.to_string(), |_| true), None);
    }
}
