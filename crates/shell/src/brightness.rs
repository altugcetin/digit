use crate::app::{App, Peek};
use gtk::glib;
use std::path::{Path, PathBuf};
use std::rc::Rc;

fn read(p: &Path) -> Option<u32> {
    std::fs::read_to_string(p).ok()?.trim().parse().ok()
}

fn devices(class: &str, kbd: bool) -> Vec<PathBuf> {
    let Ok(dir) = std::fs::read_dir(format!("/sys/class/{class}")) else { return Vec::new() };
    let mut list: Vec<PathBuf> = dir
        .flatten()
        .map(|e| e.path())
        .filter(|p| !kbd || p.file_name().is_some_and(|n| n.to_string_lossy().contains("kbd_backlight")))
        .filter(|p| read(&p.join("max_brightness")).is_some_and(|m| m > 0))
        .collect();
    list.sort_by_key(|p| {
        let kind = std::fs::read_to_string(p.join("type")).unwrap_or_default();
        match kind.trim() {
            "firmware" => 0,
            "platform" => 1,
            _ => 2,
        }
    });
    list
}

/// Sets a screen or keyboard light through logind, which lets the session's own user do it
/// without root or a password. `subsystem` is "backlight" or "leds" and `name` a device found
/// under /sys/class; logind checks both again before it writes.
async fn set(subsystem: &str, name: &str, value: u32) -> bool {
    let Ok(conn) = crate::bus::system().await else { return false };
    let Ok(session) = zbus::Proxy::new_owned(conn, "org.freedesktop.login1", "/org/freedesktop/login1/session/auto", "org.freedesktop.login1.Session").await else { return false };
    session.call_method("SetBrightness", &(subsystem, name, value)).await.is_ok()
}

/// The keyboard light: its LED under /sys/class/leds, its highest level and where it is now.
#[derive(Clone, Debug, PartialEq)]
pub struct Keyboard {
    pub name: String,
    pub max: u32,
    pub level: u32,
}

/// The keyboard light, if the computer has one. Reads sysfs, which may ask the firmware, so
/// it's called off the main thread.
pub fn keyboard() -> Option<Keyboard> {
    let dev = devices("leds", true).into_iter().next()?;
    let max = read(&dev.join("max_brightness"))?;
    let level = read(&dev.join("brightness")).unwrap_or(0).min(max);
    Some(Keyboard { name: dev.file_name()?.to_string_lossy().into_owned(), max, level })
}

impl Keyboard {
    /// The level a press of its tile moves to: each level in turn when there are a few, as the
    /// keyboard's own key does, otherwise a third, two thirds and full; after the last, off.
    pub fn next(&self) -> u32 {
        let steps: Vec<u32> = if self.max <= 3 { (1..=self.max).collect() } else { vec![self.max.div_ceil(3), (2 * self.max).div_ceil(3), self.max] };
        steps.into_iter().find(|&s| s > self.level).unwrap_or(0)
    }

    pub fn label(&self) -> String {
        match (self.level, self.max) {
            (0, _) => "Off".into(),
            (_, 1) => "On".into(),
            (1, 2 | 3) => "Low".into(),
            (2, 3) => "Medium".into(),
            (l, 2 | 3) if l == self.max => "High".into(),
            (l, m) => format!("{}%", (l as f64 / m as f64 * 100.0).round()),
        }
    }

    /// Moves the light to `level` (never past its highest); whether logind did it.
    pub async fn set(&self, level: u32) -> bool {
        set("leds", &self.name, level.min(self.max)).await
    }
}

pub fn step(app: &Rc<App>, action: &str) -> bool {
    let (class, kbd, up, icon, label) = match action {
        "brightness-up" => ("backlight", false, true, "light_mode", "Brightness"),
        "brightness-down" => ("backlight", false, false, "light_mode", "Brightness"),
        "kbd-up" => ("leds", true, true, "keyboard", "Keyboard light"),
        "kbd-down" => ("leds", true, false, "keyboard", "Keyboard light"),
        _ => return false,
    };
    let Some(dev) = devices(class, kbd).into_iter().next() else { return false };
    let Some(max) = read(&dev.join("max_brightness")) else { return false };
    let now = read(&dev.join("actual_brightness")).or_else(|| read(&dev.join("brightness"))).unwrap_or(0);
    let unit = if kbd { 1.0 / max.min(10) as f64 } else { 0.05 };
    let level = now as f64 / max as f64;
    let next = if up { (level + unit).min(1.0) } else { (level - unit).max(if kbd { 0.0 } else { 0.01 }) };
    let value = ((next * max as f64).round() as u32).min(max);
    let name = dev.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    glib::spawn_future_local(async move {
        set(class, &name, value).await;
    });
    app.request(Peek::Osd { icon, text: label.into(), level: Some(value as f64 / max as f64) });
    true
}

#[cfg(test)]
mod tests {
    use super::Keyboard;

    fn kbd(level: u32, max: u32) -> Keyboard {
        Keyboard { name: "tpacpi::kbd_backlight".into(), max, level }
    }

    #[test]
    fn a_press_steps_through_the_levels_then_turns_the_light_off() {
        assert_eq!([0, 1].map(|l| kbd(l, 1).next()), [1, 0]);
        assert_eq!([0, 1, 2].map(|l| kbd(l, 2).next()), [1, 2, 0]);
        assert_eq!([0, 1, 2, 3].map(|l| kbd(l, 3).next()), [1, 2, 3, 0]);
        // Many levels: a third, two thirds, full, off; a level set by the keys goes up to the next.
        assert_eq!([0, 34, 67, 100].map(|l| kbd(l, 100).next()), [34, 67, 100, 0]);
        assert_eq!(kbd(50, 100).next(), 67);
        assert_eq!([0, 3, 4, 7, 9].map(|l| kbd(l, 10).next()), [4, 4, 7, 10, 10]);
    }

    #[test]
    fn levels_have_plain_names() {
        assert_eq!(kbd(0, 2).label(), "Off");
        assert_eq!(kbd(1, 1).label(), "On");
        assert_eq!([1, 2].map(|l| kbd(l, 2).label()), ["Low", "High"]);
        assert_eq!([1, 2, 3].map(|l| kbd(l, 3).label()), ["Low", "Medium", "High"]);
        assert_eq!(kbd(128, 255).label(), "50%");
    }
}
