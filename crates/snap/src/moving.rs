//! What moves on the selection screen: the window under the pointer lights up and the light
//! glides on to the next one, the dark opens over a selection, and the magnifier comes and goes.

use crate::windows::Rect;
use kit::motion::{self, Tween};

/// How dark the screen is outside the selection.
pub const DARK: f64 = 0.45;

/// One screen's movement.
pub struct Moving {
    dark: Tween,
    light: Tween,
    aim: [Tween; 4],
    /// The selection on this screen as it was last drawn, and how far the dark over it has gone.
    part: Option<Rect>,
    open: Tween,
    grips: Tween,
    /// A selection that was dropped, darkening again.
    dropped: Option<(Rect, Tween)>,
    loupe: Tween,
    /// Whether the magnifier is on the pointer's other side, across and down.
    side: [Tween; 2],
    at: (f64, f64),
}

impl Default for Moving {
    fn default() -> Self {
        let still = Tween::still(0.0);
        Self { dark: still, light: still, aim: [still; 4], part: None, open: still, grips: still, dropped: None, loupe: still, side: [still; 2], at: (0.0, 0.0) }
    }
}

/// The magnifier as it's drawn now.
pub struct Loupe {
    pub at: (f64, f64),
    pub side: (f64, f64),
    pub shown: f64,
}

impl Moving {
    /// How dark it is; it gets dark as the screen opens.
    pub fn dark(&mut self, now: i64) -> f64 {
        self.dark.go(1.0, now, 320, motion::GENTLE);
        DARK * self.dark.value(now).clamp(0.0, 1.0)
    }

    /// The light over `window`, and how strong it is: on from where it was to the window it's
    /// on now, or fading where it last was.
    pub fn light(&mut self, window: Option<Rect>, now: i64) -> (Rect, f64) {
        match window {
            Some(r) => {
                if self.light.value(now) < 0.01 {
                    // It grows into its first window.
                    let (dx, dy) = ((r.2 * 0.02).min(16.0), (r.3 * 0.02).min(16.0));
                    for (t, v) in self.aim.iter_mut().zip([r.0 + dx, r.1 + dy, r.2 - 2.0 * dx, r.3 - 2.0 * dy]) {
                        t.set(v);
                    }
                }
                for (t, v) in self.aim.iter_mut().zip([r.0, r.1, r.2, r.3]) {
                    t.go(v, now, 400, motion::GENTLE);
                }
                self.light.go(1.0, now, 260, motion::GENTLE);
            }
            None => self.light.go(0.0, now, 240, motion::GENTLE),
        }
        let [x, y, w, h] = self.aim.map(|t| t.value(now));
        ((x, y, w, h), self.light.value(now).clamp(0.0, 1.0))
    }

    /// Takes in the selection's part on this screen. One that comes `by_hand` is clear from the
    /// start; one that came at once opens. It has no handles while it's `drawn`.
    pub fn select(&mut self, part: Option<Rect>, by_hand: bool, drawn: bool, now: i64) {
        match (self.part, part) {
            (None, Some(_)) => {
                self.dropped = None;
                self.open.set(if by_hand { 1.0 } else { 0.0 });
                self.open.go(1.0, now, 320, motion::GENTLE);
            }
            (Some(was), None) => {
                let mut back = Tween::still(0.0);
                back.go(1.0, now, 280, motion::GENTLE);
                self.dropped = Some((was, back));
            }
            _ => {}
        }
        self.part = part;
        if part.is_some() && !drawn {
            self.grips.go(1.0, now, 300, motion::GENTLE);
        } else {
            self.grips.set(0.0);
        }
    }

    /// How much of the dark over the selection is left.
    pub fn over(&self, now: i64) -> f64 {
        1.0 - self.open.value(now).clamp(0.0, 1.0)
    }

    /// How big the selection's handles are.
    pub fn grips(&self, now: i64) -> f64 {
        self.grips.value(now).clamp(0.0, 1.0)
    }

    /// A dropped selection, and how dark it is again.
    pub fn dropped(&self, now: i64) -> Option<(Rect, f64)> {
        self.dropped.as_ref().filter(|(_, t)| !t.settled(now)).map(|(r, t)| (*r, t.value(now).clamp(0.0, 1.0)))
    }

    /// The magnifier at the pointer, on its `other` sides where there's no room.
    pub fn loupe(&mut self, at: Option<(f64, f64)>, other: impl FnOnce(f64, f64) -> (bool, bool), now: i64) -> Option<Loupe> {
        match at {
            Some((x, y)) => {
                let (across, down) = other(x, y);
                let fresh = self.loupe.value(now) < 0.01;
                for (t, over) in self.side.iter_mut().zip([across, down]) {
                    let to = if over { 1.0 } else { 0.0 };
                    if fresh {
                        t.set(to);
                    } else {
                        t.go(to, now, 340, motion::GENTLE);
                    }
                }
                self.at = (x, y);
                self.loupe.go(1.0, now, 220, motion::GENTLE);
            }
            None => self.loupe.go(0.0, now, 180, motion::GENTLE),
        }
        let shown = self.loupe.value(now).clamp(0.0, 1.0);
        (shown > 0.01).then(|| Loupe { at: self.at, side: (self.side[0].value(now), self.side[1].value(now)), shown })
    }

    pub fn busy(&self, now: i64) -> bool {
        [&self.dark, &self.light, &self.open, &self.grips, &self.loupe].into_iter().chain(&self.aim).chain(&self.side).any(|t| !t.settled(now))
            || self.dropped.as_ref().is_some_and(|(_, t)| !t.settled(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Rect = (0.0, 0.0, 400.0, 300.0);
    const B: Rect = (500.0, 100.0, 200.0, 200.0);
    const LATE: i64 = 10_000_000;

    #[test]
    fn the_light_grows_into_the_first_window_and_glides_to_the_next() {
        let mut m = Moving::default();
        let (first, shown) = m.light(Some(A), 0);
        if !motion::reduced() {
            assert!(shown < 0.01 && first.0 > A.0 && first.2 < A.2);
        }
        assert_eq!(m.light(Some(A), LATE), (A, 1.0));
        let (next, shown) = m.light(Some(B), LATE);
        assert_eq!(shown, 1.0, "it doesn't go out in between");
        if !motion::reduced() {
            assert_eq!(next, A);
        }
        assert_eq!(m.light(Some(B), 2 * LATE), (B, 1.0));
        m.light(None, 2 * LATE);
        assert_eq!(m.light(None, 3 * LATE), (B, 0.0), "it fades where it was");
        assert!(!m.busy(3 * LATE));
    }

    #[test]
    fn a_selection_drawn_by_hand_is_clear_at_once_and_one_by_a_click_opens() {
        let mut m = Moving::default();
        m.select(Some(A), true, true, 0);
        assert_eq!((m.over(0), m.grips(0)), (0.0, 0.0));
        m.select(Some(A), false, false, 0);
        assert_eq!((m.over(LATE), m.grips(LATE)), (0.0, 1.0));
        m.select(None, false, false, LATE);
        if !motion::reduced() {
            assert_eq!(m.dropped(LATE), Some((A, 0.0)));
            m.select(Some(B), false, false, LATE);
            assert_eq!(m.over(LATE), 1.0);
        }
        assert_eq!(m.dropped(2 * LATE), None);
    }

    #[test]
    fn the_magnifier_starts_on_the_side_it_belongs_on() {
        let mut m = Moving::default();
        assert!(m.loupe(None, |_, _| (false, false), 0).is_none());
        m.loupe(Some((5.0, 6.0)), |_, _| (true, false), 0);
        let l = m.loupe(Some((5.0, 6.0)), |_, _| (true, false), LATE).unwrap();
        assert_eq!((l.at, l.side, l.shown), ((5.0, 6.0), (1.0, 0.0), 1.0));
        m.loupe(None, |_, _| (false, false), LATE);
        assert!(m.loupe(None, |_, _| (false, false), 2 * LATE).is_none());
    }
}
