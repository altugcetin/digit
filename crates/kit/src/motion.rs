use std::cell::Cell;

/// What Settings can turn off on its own. The rest of the motion has settings from before, which
/// their readers ask for themselves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Buttons,
    DockClicks,
    DockLaunch,
    DockGlide,
    Notifications,
    AppGrid,
    Screenshot,
}

impl Part {
    pub const ALL: [Part; 7] = [Part::Buttons, Part::DockClicks, Part::DockLaunch, Part::DockGlide, Part::Notifications, Part::AppGrid, Part::Screenshot];

    /// Its setting, which is on unless it's turned off.
    pub fn key(self) -> &'static str {
        match self {
            Part::Buttons => "button_motion",
            Part::DockClicks => "dock_click_motion",
            Part::DockLaunch => "dock_launch_motion",
            Part::DockGlide => "dock_glide_motion",
            Part::Notifications => "notification_motion",
            Part::AppGrid => "appgrid_motion",
            Part::Screenshot => "snap_motion",
        }
    }

    pub fn chosen(self, prefs: &toml::Table) -> bool {
        crate::prefs::flag(prefs, self.key(), true)
    }

    fn bit(self) -> u32 {
        1 << self as u32
    }
}

/// Whether `part` moves: its own setting, and Reduce motion over it.
pub fn on(part: Part) -> bool {
    !reduced() && OFF.get() & part.bit() == 0
}

fn off(prefs: &toml::Table) -> u32 {
    Part::ALL.into_iter().filter(|p| !p.chosen(prefs)).map(Part::bit).sum()
}

thread_local! {
    /// The parts that are turned off.
    static OFF: Cell<u32> = const { Cell::new(0) };
    static LOADED: Cell<bool> = const { Cell::new(false) };
    static POLICY: Cell<(bool, u64)> = const { Cell::new((false, 0)) };
}

/// The process motion policy. Worker threads resolve the same preference without GTK access.
pub fn reduced() -> bool {
    if !LOADED.get() { refresh(); }
    POLICY.get().0
}

/// Read the effective master and apply it live. Returns whether the policy changed.
pub fn refresh() -> bool {
    let prefs = crate::prefs::read();
    OFF.set(off(&prefs));
    let on = crate::effects::effective(&prefs, "reduce_motion", crate::effects::software());
    let changed = POLICY.get().0 != on;
    apply(on);
    changed
}

pub fn automatic() -> bool {
    crate::prefs::read().get("reduce_motion").and_then(toml::Value::as_bool).is_none() && crate::effects::software()
}

pub fn enabled(stored: bool) -> bool {
    child_enabled(stored, reduced())
}

fn child_enabled(stored: bool, master: bool) -> bool {
    stored && !master
}

pub fn apply(reduce: bool) {
    LOADED.set(true);
    let (before, generation) = POLICY.get();
    if before == reduce {
        return;
    }
    // An in-flight tween stays settled if motion is turned back on.
    POLICY.set((reduce, generation + u64::from(reduce)));
    if !gtk::is_initialized_main_thread() { return; }
    if let Some(settings) = gtk::Settings::default() {
        if reduce {
            settings.set_gtk_enable_animations(false);
            settings.set_gtk_cursor_blink(false);
        } else {
            settings.reset_property("gtk-enable-animations");
            settings.reset_property("gtk-cursor-blink");
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Curve(f64, f64, f64, f64);

pub const EMPHASIZED_DECEL: Curve = Curve(0.05, 0.7, 0.1, 1.0);
pub const FAST_SPATIAL: Curve = Curve(0.42, 1.67, 0.21, 0.90);
pub const DEFAULT_SPATIAL: Curve = Curve(0.38, 1.21, 0.22, 1.0);
pub const FAST_EFFECTS: Curve = Curve(0.31, 0.94, 0.34, 1.0);
pub const STANDARD: Curve = Curve(0.2, 0.0, 0.0, 1.0);
pub const IN_OUT_CUBIC: Curve = Curve(0.65, 0.0, 0.35, 1.0);
/// Sets off gently and settles gently, for what crosses much of the screen.
pub const GENTLE: Curve = Curve(0.33, 0.0, 0.15, 1.0);
pub const REVEAL: Curve = Curve(0.45, 0.0, 0.15, 1.0);

impl Curve {
    pub const fn new(x1: f64, y1: f64, x2: f64, y2: f64) -> Self {
        Self(x1, y1, x2, y2)
    }

    fn axis(a: f64, b: f64, t: f64) -> f64 {
        let u = 1.0 - t;
        3.0 * u * u * t * a + 3.0 * u * t * t * b + t * t * t
    }

    fn slope(a: f64, b: f64, t: f64) -> f64 {
        let u = 1.0 - t;
        3.0 * u * u * a + 6.0 * u * t * (b - a) + 3.0 * t * t * (1.0 - b)
    }

    pub fn at(&self, x: f64) -> f64 {
        if x <= 0.0 {
            return 0.0;
        }
        if x >= 1.0 {
            return 1.0;
        }
        let mut t = x;
        for _ in 0..8 {
            let d = Self::slope(self.0, self.2, t);
            if d.abs() < 1e-6 {
                break;
            }
            t -= (Self::axis(self.0, self.2, t) - x) / d;
        }
        if !(0.0..=1.0).contains(&t) || (Self::axis(self.0, self.2, t) - x).abs() > 1e-4 {
            let (mut lo, mut hi) = (0.0, 1.0);
            for _ in 0..30 {
                t = (lo + hi) / 2.0;
                if Self::axis(self.0, self.2, t) < x { lo = t } else { hi = t }
            }
        }
        Self::axis(self.1, self.3, t)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Tween {
    from: f64,
    to: f64,
    start: i64,
    span: i64,
    curve: Curve,
    generation: u64,
}

impl Tween {
    pub fn still(v: f64) -> Self {
        Self { from: v, to: v, start: 0, span: 0, curve: STANDARD, generation: 0 }
    }

    pub fn value(&self, now: i64) -> f64 {
        if self.settled(now) {
            return self.to;
        }
        let x = (now - self.start).max(0) as f64 / self.span as f64;
        self.from + (self.to - self.from) * self.curve.at(x)
    }

    pub fn target(&self) -> f64 {
        self.to
    }

    pub fn left_ms(&self, now: i64) -> i64 {
        if self.settled(now) { 0 } else { (self.start + self.span - now) / 1000 }
    }

    pub fn settled(&self, now: i64) -> bool {
        self.span <= 0 || now >= self.start + self.span || self.generation != POLICY.get().1
    }

    pub fn go(&mut self, to: f64, now: i64, ms: i64, curve: Curve) {
        if reduced() {
            self.set(to);
            return;
        }
        if (to - self.to).abs() < 1e-6 {
            return;
        }
        self.from = self.value(now);
        self.to = to;
        self.start = now;
        self.span = ms * 1000;
        self.curve = curve;
        self.generation = POLICY.get().1;
    }

    pub fn after(&mut self, to: f64, now: i64, delay_ms: i64, ms: i64, curve: Curve) {
        if reduced() {
            self.set(to);
            return;
        }
        self.from = self.value(now);
        self.to = to;
        self.start = now + delay_ms * 1000;
        self.span = ms * 1000;
        self.curve = curve;
        self.generation = POLICY.get().1;
    }

    pub fn set(&mut self, v: f64) {
        *self = Self::still(v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_move_unless_they_are_turned_off_one_by_one() {
        assert_eq!(off(&toml::Table::new()), 0);
        let mut keys = std::collections::HashSet::new();
        for part in Part::ALL {
            assert!(keys.insert(part.key()));
            let prefs: toml::Table = format!("{} = false", part.key()).parse().unwrap();
            assert_eq!(off(&prefs), part.bit());
            assert!(Part::ALL.into_iter().all(|p| p.chosen(&prefs) == (p != part)));
        }
    }


    #[test]
    fn masters_honor_explicit_values_and_software_defaults_independently() {
        for software in [false, true] {
            for key in ["reduce_motion", "reduced_effects"] {
                let mut t = toml::Table::new();
                assert_eq!(crate::effects::effective(&t, key, software), software);
                for stored in [false, true] {
                    t.insert(key.into(), stored.into());
                    assert_eq!(crate::effects::effective(&t, key, software), stored);
                }
            }
            for motion in [false, true] {
                let mut t = toml::Table::new();
                t.insert("reduce_motion".into(), motion.into());
                for effects in [false, true] {
                    t.insert("reduced_effects".into(), effects.into());
                    assert_eq!(crate::effects::effective(&t, "reduce_motion", software), motion);
                    assert_eq!(crate::effects::effective(&t, "reduced_effects", software), effects);
                }
            }
        }
    }

    #[test]
    fn turning_motion_back_on_allows_new_tweens_without_resuming_old_ones() {
        apply(false);
        let mut tween = Tween::still(0.0);
        tween.go(1.0, 1_000, 100, STANDARD);
        assert!(!tween.settled(2_000));
        apply(true);
        assert_eq!(tween.value(2_000), 1.0);
        apply(false);
        assert_eq!(tween.value(2_000), 1.0);
        tween.go(2.0, 2_000, 100, STANDARD);
        assert!(!tween.settled(3_000));
        assert!(tween.value(3_000) < 2.0);
    }

    #[test]
    fn children_resume_their_stored_value_after_the_master_is_off() {
        for stored in [false, true] {
            assert!(!child_enabled(stored, true));
            assert_eq!(child_enabled(stored, false), stored);
        }
    }
}
