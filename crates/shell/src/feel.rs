//! How drawn buttons answer the pointer, shared by the panels that draw their own: they light up
//! under it, give when pressed, fill when turned on and swell a little when what they show
//! changes. Each button is known by its panel's own key.

use gtk::{glib, graphene, gsk, prelude::*};
use kit::motion::{self, Curve, Part, Tween};
use kit::palette::Rgb;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const SWELL: i64 = 340_000;

/// How far a pressed button shrinks: a small round one, a pill, a tile and a whole row.
pub const SMALL: f64 = 0.10;
pub const PILL: f64 = 0.05;
pub const TILE: f64 = 0.04;
pub const ROW: f64 = 0.02;

struct Mark<K> {
    key: K,
    glow: Tween,
    press: Tween,
    /// Whether it's on, once it has been asked for.
    on: Option<Tween>,
    /// What it last showed, once it has been told.
    face: Option<&'static str>,
    swell: Option<i64>,
}

pub struct Feel<K>(RefCell<Vec<Mark<K>>>);

impl<K> Default for Feel<K> {
    fn default() -> Self {
        Self(RefCell::new(Vec::new()))
    }
}

/// Takes `t` to `to`, at once where buttons keep still.
fn go(t: &mut Tween, to: f64, now: i64, ms: i64, curve: Curve) {
    if motion::on(Part::Buttons) {
        t.go(to, now, ms, curve);
    } else {
        t.set(to);
    }
}

impl<K: Copy + PartialEq> Feel<K> {
    fn with<R>(&self, key: K, f: impl FnOnce(&mut Mark<K>) -> R) -> R {
        let mut marks = self.0.borrow_mut();
        let i = marks.iter().position(|m| m.key == key).unwrap_or_else(|| {
            marks.push(Mark { key, glow: Tween::still(0.0), press: Tween::still(0.0), on: None, face: None, swell: None });
            marks.len() - 1
        });
        f(&mut marks[i])
    }

    fn get(&self, key: K, read: impl Fn(&Mark<K>) -> f64) -> f64 {
        self.0.borrow().iter().find(|m| m.key == key).map_or(0.0, read).clamp(0.0, 1.0)
    }

    /// The pointer is on `keys`: a button, and what it stands on.
    pub fn hover(&self, keys: &[K], now: i64) {
        self.light(keys, 160, 280, now);
    }

    /// `hover` for the rows of a menu, which the pointer runs over quickly.
    pub fn hover_row(&self, row: Option<K>, now: i64) {
        self.light(&row.into_iter().collect::<Vec<_>>(), 80, 110, now);
    }

    fn light(&self, keys: &[K], on: i64, off: i64, now: i64) {
        for key in keys {
            self.with(*key, |m| go(&mut m.glow, 1.0, now, on, motion::STANDARD));
        }
        self.0.borrow_mut().iter_mut().filter(|m| !keys.contains(&m.key)).for_each(|m| go(&mut m.glow, 0.0, now, off, motion::STANDARD));
    }

    pub fn press(&self, key: K, now: i64) {
        if motion::on(Part::Buttons) {
            self.with(key, |m| m.press.go(1.0, now, 90, motion::STANDARD));
        }
    }

    /// Lets go of whatever is pressed.
    pub fn release(&self, now: i64) {
        self.0.borrow_mut().iter_mut().for_each(|m| go(&mut m.press, 0.0, now, 280, motion::EMPHASIZED_DECEL));
    }

    /// How lit it is.
    pub fn glow(&self, key: K, now: i64) -> f64 {
        self.get(key, |m| m.glow.value(now))
    }

    /// Its size against its own, for being pressed; `depth` is how far it gives.
    pub fn scale(&self, key: K, depth: f64, now: i64) -> f64 {
        1.0 - depth * self.get(key, |m| m.press.value(now))
    }

    /// How far on it is, on its way to `on`. The first time there's nothing to come from.
    pub fn on(&self, key: K, on: bool, now: i64) -> f64 {
        let to = if on { 1.0 } else { 0.0 };
        self.with(key, |m| {
            let tween = m.on.get_or_insert_with(|| Tween::still(to));
            if (tween.target() - to).abs() > 0.5 {
                go(tween, to, now, 280, motion::STANDARD);
                m.swell = Some(now);
            }
            tween.value(now).clamp(0.0, 1.0)
        })
    }

    /// The size of the icon `face`, which swells when it isn't the one it was, or it turned on
    /// or off.
    pub fn icon(&self, key: K, face: &'static str, now: i64) -> f64 {
        self.with(key, |m| {
            if m.face.replace(face).is_some_and(|was| was != face) {
                m.swell = Some(now);
            }
            1.0 + 0.16 * m.swell.map_or(0.0, |t| swell(now - t))
        })
    }

    /// Forgets what doesn't `keep`, so it doesn't animate from what it was when it's shown again.
    pub fn retain(&self, keep: impl Fn(K) -> bool) {
        self.0.borrow_mut().retain(|m| keep(m.key));
    }

    pub fn busy(&self, now: i64) -> bool {
        self.0.borrow().iter().any(|m| {
            !m.glow.settled(now) || !m.press.settled(now) || m.on.as_ref().is_some_and(|t| !t.settled(now)) || m.swell.is_some_and(|t| swelling(now - t))
        })
    }
}

/// Draws `w` each frame while it's `busy`; once at a time, as `ticking` tells.
pub fn run(w: &impl IsA<gtk::Widget>, ticking: &Rc<Cell<bool>>, busy: impl Fn(i64) -> bool + 'static) {
    if ticking.replace(true) {
        return;
    }
    let ticking = ticking.clone();
    w.add_tick_callback(move |w, clock| {
        w.queue_draw();
        if busy(clock.frame_time()) {
            return glib::ControlFlow::Continue;
        }
        ticking.set(false);
        glib::ControlFlow::Break
    });
}

fn swelling(dt: i64) -> bool {
    motion::on(Part::Buttons) && (0..SWELL).contains(&dt)
}

/// A short swell `dt` µs after it began, up to 1: up quickly, down slowly.
fn swell(dt: i64) -> f64 {
    if !swelling(dt) {
        return 0.0;
    }
    let left = 1.0 - dt as f64 / SWELL as f64;
    (std::f64::consts::PI * (1.0 - left * left)).sin()
}

/// Draws `k` times as big around a point.
pub fn scaled(s: &gtk::Snapshot, k: f64, cx: f64, cy: f64, draw: impl FnOnce()) {
    if (k - 1.0).abs() < 1e-3 {
        return draw();
    }
    let at = |x: f64, y: f64| graphene::Point::new(x as f32, y as f32);
    s.save();
    s.translate(&at(cx, cy));
    s.scale(k as f32, k as f32);
    s.translate(&at(-cx, -cy));
    crate::draw::grown(k, draw);
    s.restore();
}

/// Draws turned by `degrees` around a point.
pub fn turned(s: &gtk::Snapshot, degrees: f64, cx: f64, cy: f64, draw: impl FnOnce()) {
    if degrees.abs() < 0.1 {
        return draw();
    }
    let at = |x: f64, y: f64| graphene::Point::new(x as f32, y as f32);
    s.save();
    s.translate(&at(cx, cy));
    s.rotate(degrees as f32);
    s.translate(&at(-cx, -cy));
    draw();
    s.restore();
}

/// Fills `shape` with `base`, and with `tint` as far as `on` has spread from a point.
pub fn flood(s: &gtk::Snapshot, shape: &gsk::RoundedRect, base: Rgb, tint: Rgb, on: f64, cx: f64, cy: f64) {
    let b = shape.bounds();
    s.push_rounded_clip(shape);
    if on < 0.999 {
        s.append_color(&base.rgba(), b);
    }
    if on >= 0.999 {
        s.append_color(&tint.rgba(), b);
    } else if on > 0.001 {
        let r = reach((b.x() as f64, b.y() as f64, b.width() as f64, b.height() as f64), cx, cy) * on;
        let spot = graphene::Rect::new((cx - r) as f32, (cy - r) as f32, (2.0 * r) as f32, (2.0 * r) as f32);
        s.push_rounded_clip(&gsk::RoundedRect::from_rect(spot, r as f32));
        s.append_color(&tint.rgba(), b);
        s.pop();
    }
    s.pop();
}

/// How far the farthest corner of a box is from a point.
fn reach((x, y, w, h): (f64, f64, f64, f64), cx: f64, cy: f64) -> f64 {
    (cx - x).abs().max((x + w - cx).abs()).hypot((cy - y).abs().max((y + h - cy).abs()))
}

/// `c`, only `k` as strong.
pub fn faint(c: Rgb, k: f64) -> Rgb {
    c.alpha(c.a * k)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hover_lights_one_button_and_what_it_stands_on() {
        let f: Feel<u8> = Feel::default();
        f.hover(&[1, 2], 0);
        let late = 10_000_000;
        assert_eq!([f.glow(1, late), f.glow(2, late), f.glow(3, late)], [1.0, 1.0, 0.0]);
        f.hover(&[2], late);
        assert_eq!([f.glow(1, 2 * late), f.glow(2, 2 * late)], [0.0, 1.0]);
        f.hover(&[], 2 * late);
        assert_eq!(f.glow(2, 3 * late), 0.0);
        assert!(!f.busy(3 * late));
    }

    #[test]
    fn a_button_starts_as_it_is_and_animates_only_when_it_changes() {
        let f: Feel<u8> = Feel::default();
        assert_eq!(f.on(1, true, 0), 1.0);
        assert!(!f.busy(0));
        assert_eq!(f.icon(1, "wifi", 0), 1.0);
        f.on(1, false, 1_000);
        assert!(!motion::on(Part::Buttons) || f.busy(1_000));
        assert_eq!(f.on(1, false, 10_000_000), 0.0);
        // Forgotten, it starts anew from whatever it is then.
        f.retain(|_| false);
        assert_eq!(f.on(1, true, 10_000_000), 1.0);
        assert!(!f.busy(10_000_000));
    }

    #[test]
    fn a_press_gives_and_comes_back() {
        let f: Feel<u8> = Feel::default();
        f.press(1, 0);
        if motion::on(Part::Buttons) {
            assert!((f.scale(1, 0.1, 1_000_000) - 0.9).abs() < 1e-9);
        }
        f.release(1_000_000);
        assert_eq!(f.scale(1, 0.1, 10_000_000), 1.0);
    }

    #[test]
    fn a_flood_reaches_every_corner() {
        let tile = (0.0, 0.0, 160.0, 64.0);
        assert!((reach(tile, 30.0, 32.0) - 130.0f64.hypot(32.0)).abs() < 1e-9);
        assert!((reach(tile, 80.0, 32.0) - 80.0f64.hypot(32.0)).abs() < 1e-9);
    }

    #[test]
    fn a_swell_starts_and_ends_at_rest() {
        assert_eq!([swell(-1), swell(0), swell(SWELL)], [0.0; 3]);
        assert!((0..SWELL).step_by(10_000).all(|t| (0.0..=1.0).contains(&swell(t))));
    }
}
