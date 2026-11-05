use std::cell::Cell;

thread_local! {
    static POLICY: Cell<(bool, u64)> = const { Cell::new((false, 0)) };
}

pub fn reduced() -> bool {
    POLICY.get().0
}

pub fn apply(reduce: bool) {
    let (before, generation) = POLICY.get();
    if before == reduce {
        return;
    }
    // An in-flight tween stays settled if motion is turned back on.
    POLICY.set((reduce, generation + u64::from(reduce)));
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
