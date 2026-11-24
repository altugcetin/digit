//! The dock's answers to clicks, and how it changes as apps come and go: icons that give under
//! the pointer, the hop of an app that starts, a body that grows and shrinks, closing icons and
//! the marks under open apps.

use super::*;
use kit::motion::Part;

const HOP: i64 = 520_000;
const HOPS: i64 = 8;
const PULSE: i64 = 340_000;

/// An app that was started, hopping until it has a window.
pub(super) struct Bounce {
    key: String,
    start: i64,
    end: i64,
}

impl Bounce {
    fn new(key: String, now: i64) -> Self {
        Self { key, start: now, end: now + HOPS * HOP }
    }

    /// How high it is, up to 1. Each hop is lower than the one before.
    fn height(&self, now: i64) -> f64 {
        let dt = now - self.start;
        if dt < 0 || now >= self.end {
            return 0.0;
        }
        let (n, p) = (dt / HOP, (dt % HOP) as f64 / HOP as f64);
        (p * std::f64::consts::PI).sin() * (1.0 - n as f64 / HOPS as f64)
    }

    /// Ends with the hop it's in, so it comes down rather than dropping.
    fn finish(&mut self, now: i64) {
        self.end = self.end.min(self.start + ((now - self.start).max(0) / HOP + 1) * HOP);
    }
}

/// A short swell `dt` µs after it began: up quickly, down slowly.
fn swell(dt: i64) -> f64 {
    if !(0..PULSE).contains(&dt) {
        return 0.0;
    }
    let left = 1.0 - dt as f64 / PULSE as f64;
    (std::f64::consts::PI * (1.0 - left * left)).sin()
}

/// The icon of an app that has left the dock, shrinking away where it stood.
struct Leaving {
    key: String,
    icon: gdk::Paintable,
    center: f64,
    size: f64,
    shown: Tween,
}

/// A line between two parts of the dock.
struct Line {
    /// Where it stands, or stood, without `shift`.
    at: f64,
    shift: Tween,
    shown: Tween,
}

/// The mark under an open app: whether it's there, and whether it's long, for the one in focus.
struct Mark {
    shown: Tween,
    focus: Tween,
}

/// How the dock looked before it was rebuilt.
pub(super) struct Before {
    body: (f64, f64),
    lines: [Option<f64>; 2],
    leaving: Vec<Leaving>,
}

#[derive(Default)]
pub(super) struct Lively {
    press: HashMap<String, Tween>,
    pulse: HashMap<String, i64>,
    bounce: Vec<Bounce>,
    leaving: Vec<Leaving>,
    /// How far the body's start and length still are from where they go.
    body: Option<(Tween, Tween)>,
    lines: [Option<Line>; 2],
    marks: HashMap<String, Mark>,
}

impl Lively {
    pub(super) fn press(&mut self, key: &str, now: i64) {
        self.press.entry(key.to_string()).or_insert_with(|| Tween::still(0.0)).go(1.0, now, 90, motion::STANDARD);
    }

    /// Lets go of whatever is pressed. Returns whether anything was.
    pub(super) fn release(&mut self, now: i64) -> bool {
        let held = self.press.values().any(|t| t.target() > 0.5);
        self.press.values_mut().for_each(|t| t.go(0.0, now, 280, motion::EMPHASIZED_DECEL));
        held
    }

    pub(super) fn pulse(&mut self, key: &str, now: i64) {
        self.pulse.insert(key.to_string(), now);
    }

    pub(super) fn bounce(&mut self, key: String, now: i64) {
        self.bounce.retain(|b| b.key != key);
        self.bounce.push(Bounce::new(key, now));
    }

    /// The app has a window: its icon comes down from its hop and stays.
    pub(super) fn opened(&mut self, key: &str, now: i64) {
        self.bounce.iter_mut().filter(|b| b.key == key).for_each(|b| b.finish(now));
    }

    pub(super) fn hop(&self, key: &str, now: i64) -> f64 {
        self.bounce.iter().find(|b| b.key == key).map_or(0.0, |b| b.height(now))
    }

    /// How big an icon is for being pressed or having answered a click, against its size.
    pub(super) fn scale(&self, key: &str, now: i64) -> f64 {
        1.0 - 0.1 * self.pressed(key, now) + 0.08 * self.pulse.get(key).map_or(0.0, |t| swell(now - t))
    }

    pub(super) fn opacity(&self, key: &str, now: i64) -> f64 {
        1.0 - 0.25 * self.pressed(key, now)
    }

    fn pressed(&self, key: &str, now: i64) -> f64 {
        self.press.get(key).map_or(0.0, |t| t.value(now).clamp(0.0, 1.0))
    }

    /// Where an app dragged off the dock is let go, for its icon to go from there.
    pub(super) fn leave_from(&mut self, key: &str, center: Option<f64>) {
        match center {
            Some(center) => self.leaving.iter_mut().filter(|l| l.key == key).for_each(|l| l.center = center),
            None => self.leaving.retain(|l| l.key != key),
        }
    }

    pub(super) fn busy(&self, now: i64) -> bool {
        self.press.values().any(|t| !t.settled(now))
            || self.pulse.values().any(|t| now - t < PULSE)
            || self.bounce.iter().any(|b| now < b.end)
            || self.leaving.iter().any(|l| !l.shown.settled(now))
            || self.body.iter().any(|(a, b)| !a.settled(now) || !b.settled(now))
            || self.lines.iter().flatten().any(|l| !l.shift.settled(now) || !l.shown.settled(now))
            || self.marks.values().any(|m| !m.shown.settled(now) || !m.focus.settled(now))
    }

    /// Drops what has ended.
    pub(super) fn tidy(&mut self, now: i64) {
        self.press.retain(|_, t| t.target() > 0.5 || !t.settled(now));
        self.pulse.retain(|_, t| now - *t < PULSE);
        self.bounce.retain(|b| now < b.end);
        self.leaving.retain(|l| !l.shown.settled(now));
        self.body = self.body.filter(|(a, b)| !a.settled(now) || !b.settled(now));
        for line in &mut self.lines {
            if line.as_ref().is_some_and(|l| l.shown.target() < 0.5 && l.shown.settled(now)) {
                *line = None;
            }
        }
    }

    /// Stops the movement at once, for Reduce motion.
    pub(super) fn still(&mut self) {
        self.press.clear();
        self.pulse.clear();
        self.bounce.clear();
        self.leaving.clear();
        self.body = None;
    }
}

/// A tween that goes from `from` to 0 the way the icons slide into place.
fn glide(from: f64, now: i64) -> Tween {
    let mut t = Tween::still(from);
    t.go(0.0, now, 300, motion::EMPHASIZED_DECEL);
    t
}

impl State {
    /// Where the body is drawn, on its way to `body_span`.
    fn drawn_span(&self, now: i64) -> (f64, f64) {
        let (start, len) = self.body_span(now);
        self.lively.body.as_ref().map_or((start, len), |(a, b)| (start + a.value(now), len + b.value(now)))
    }

    /// Where the line after the kept apps and the one before the stacks stand.
    fn lines(&self, now: i64) -> [Option<f64>; 2] {
        let layout = self.layout(now);
        let mut out = [None; 2];
        for k in self.separators() {
            out[usize::from(self.items[k].kind != Kind::App)] = Some(layout[k].0 - self.geo.pad() - 1.0);
        }
        out
    }

    /// What is about to change, taken before `items` replace the ones shown. None when nothing
    /// is shown yet, or it keeps still.
    pub(super) fn before(&self, items: &[Item], now: i64) -> Option<Before> {
        if self.items.is_empty() || !motion::on(Part::DockGlide) {
            return None;
        }
        let pal = self.app.palette_at(now);
        let dragged = self.drag.as_ref().filter(|d| d.away.is_some()).map(|d| d.key.as_str());
        let leaving = self.items.iter().zip(self.layout(now))
            .filter(|(old, _)| old.kind == Kind::App && Some(old.key.as_str()) != dragged && !items.iter().any(|i| i.key == old.key))
            .filter_map(|(old, (a, size))| {
                let mut shown = Tween::still(1.0);
                shown.go(0.0, now, 220, motion::STANDARD);
                let slide = self.slide.get(&old.key).map_or(0.0, |t| t.value(now));
                Some(Leaving { key: old.key.clone(), icon: self.icon_of(old, size, &pal)?, center: a + slide + size / 2.0, size, shown })
            })
            .collect();
        let shifted = |(i, at): (usize, Option<f64>)| at.map(|at| at + self.lively.lines[i].as_ref().map_or(0.0, |l| l.shift.value(now)));
        let [a, b] = self.lines(now);
        Some(Before { body: self.drawn_span(now), lines: [shifted((0, a)), shifted((1, b))], leaving })
    }

    /// Starts what changed since `before`, and marks the windows `old` didn't have minimized.
    pub(super) fn changed(&mut self, before: Option<Before>, old: &[Item], now: i64) {
        let animated = before.is_some();
        for i in 0..self.items.len() {
            let (key, open, focused) = { let it = &self.items[i]; (it.key.clone(), !it.windows.is_empty(), it.focused()) };
            if open {
                self.lively.opened(&key, now);
            }
            let was = old.iter().find(|o| o.key == key);
            let minimized = was.is_some_and(|o| self.items[i].windows.iter().any(|w| w.minimized && o.windows.iter().any(|x| x.id == w.id && !x.minimized)));
            if minimized && !self.items.is_empty() && motion::on(Part::DockClicks) {
                self.lively.pulse(&key, now);
            }
            let mark = self.lively.marks.entry(key).or_insert_with(|| Mark { shown: Tween::still(0.0), focus: Tween::still(0.0) });
            let (shown, focus) = (if open { 1.0 } else { 0.0 }, if focused { 1.0 } else { 0.0 });
            if animated {
                mark.shown.go(shown, now, 240, motion::STANDARD);
                mark.focus.go(focus, now, 240, motion::STANDARD);
            } else {
                (mark.shown, mark.focus) = (Tween::still(shown), Tween::still(focus));
            }
        }
        let items = &self.items;
        self.lively.marks.retain(|k, _| items.iter().any(|i| i.key == *k));
        self.lively.leaving.retain(|l| !items.iter().any(|i| i.key == l.key));
        let (body, lines) = (self.body_span(now), self.lines(now));
        let Some(before) = before else {
            self.lively.body = None;
            self.lively.lines = lines.map(|at| at.map(|at| Line { at, shift: Tween::still(0.0), shown: Tween::still(1.0) }));
            return;
        };
        self.lively.leaving.extend(before.leaving);
        let (da, db) = (before.body.0 - body.0, before.body.1 - body.1);
        if da.abs() > 0.5 || db.abs() > 0.5 {
            self.lively.body = Some((glide(da, now), glide(db, now)));
        } else {
            self.lively.body = None;
        }
        for (i, at) in lines.into_iter().enumerate() {
            let slot = &mut self.lively.lines[i];
            match (at, before.lines[i]) {
                (Some(at), was) => {
                    let line = slot.get_or_insert_with(|| Line { at, shift: Tween::still(0.0), shown: Tween::still(0.0) });
                    line.at = at;
                    match was {
                        Some(was) if (was - at).abs() > 0.5 => line.shift = glide(was - at, now),
                        Some(_) => {}
                        None => line.shift = Tween::still(0.0),
                    }
                    line.shown.go(1.0, now, 240, motion::STANDARD);
                }
                (None, Some(was)) => {
                    if let Some(line) = slot {
                        (line.at, line.shift) = (was, Tween::still(0.0));
                        line.shown.go(0.0, now, 180, motion::STANDARD);
                    }
                }
                (None, None) => {}
            }
        }
    }

    /// The body, its lines and the icons that are leaving, under the dock's icons.
    pub(super) fn draw_body(&self, s: &gtk::Snapshot, off: f64, pal: &kit::palette::Palette, now: i64) {
        let g = self.geo;
        let (start, len) = self.drawn_span(now);
        self.fill(s, start, LIFT + off, len, g.body(), g.radius(), pal.body.alpha(1.0));
        let lines = self.lines(now);
        for (line, at) in self.lively.lines.iter().zip(lines) {
            let Some(line) = line else { continue };
            // A line that's there follows the magnified icons; one that's going stays put.
            let a = (at.unwrap_or(line.at) + line.shift.value(now)).round();
            let shown = line.shown.value(now).clamp(0.0, 1.0);
            let (x, y, w, h) = self.map(a, LIFT + off + g.pad() + 4.0, 1.0, g.base - 8.0);
            s.append_color(&pal.ink_main.alpha(0.16 * shown).rgba(), &graphene::Rect::new(x as f32, y as f32, w as f32, h as f32));
        }
        for l in &self.lively.leaving {
            let shown = l.shown.value(now).clamp(0.0, 1.0);
            if shown < 0.01 {
                continue;
            }
            // It stays on the body as that closes over its place.
            let half = l.size / 2.0 + g.pad();
            let center = if len > 2.0 * half { l.center.clamp(start + half, start + len - half) } else { start + len / 2.0 };
            let size = l.size * (0.4 + 0.6 * shown);
            let (x, y, w, _) = self.map(center - size / 2.0, LIFT + off + g.pad() + (l.size - size) / 2.0, size, size);
            s.push_opacity(shown);
            draw::paintable(s, &l.icon, x, y, w);
            s.pop();
        }
    }

    /// The mark under an open app, centred at `center`.
    pub(super) fn draw_mark(&self, s: &gtk::Snapshot, key: &str, center: f64, off: f64, pal: &kit::palette::Palette, now: i64) {
        let Some(mark) = self.lively.marks.get(key) else { return };
        let (shown, focus) = (mark.shown.value(now).clamp(0.0, 1.0), mark.focus.value(now).clamp(0.0, 1.0));
        if shown < 0.01 {
            return;
        }
        let full = (self.geo.base * 0.045).max(2.0);
        let r = full * shown;
        let half = r + self.geo.base * 0.08 * focus * shown;
        let (x, y, w, h) = self.map(center - half, LIFT + off + 3.5 + full - r, 2.0 * half, 2.0 * r);
        let away = self.away.get(key).map_or(0.0, |t| t.value(now));
        let ink = pal.ink_main.mix(pal.ink_meta.alpha(0.7), focus);
        draw::fill(s, &draw::round(x, y, w, h, r), ink.mix(pal.ink_faint, 1.0 - away));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_started_app_hops_at_once_and_comes_down_when_its_window_opens() {
        let mut b = Bounce::new("a".into(), 1_000);
        assert_eq!(b.height(1_000), 0.0);
        assert!(b.height(1_000 + HOP / 8) > 0.3, "it answers within a few frames");
        assert!(b.height(1_000 + HOP / 2) > b.height(1_000 + HOP + HOP / 2));
        assert_eq!(b.height(1_000 + HOPS * HOP), 0.0);
        // The window opens in the middle of the second hop: that one is finished, no other starts.
        b.finish(1_000 + HOP + HOP / 2);
        assert!(b.height(1_000 + HOP + HOP * 3 / 4) > 0.0);
        assert_eq!(b.height(1_000 + 2 * HOP), 0.0);
        assert_eq!(b.height(1_000 + 2 * HOP + HOP / 2), 0.0);
        // Opening again doesn't make it longer.
        b.finish(1_000 + 5 * HOP);
        assert_eq!(b.end, 1_000 + 2 * HOP);
    }

    #[test]
    fn a_swell_starts_and_ends_at_rest() {
        assert_eq!([swell(-1), swell(0), swell(PULSE)], [0.0; 3]);
        assert!(swell(PULSE / 4) > swell(PULSE * 3 / 4));
        assert!((0..PULSE).step_by(10_000).all(|t| (0.0..=1.0).contains(&swell(t))));
    }

    #[test]
    fn a_press_that_is_let_go_doesnt_hold() {
        let mut l = Lively::default();
        assert!(!l.release(0));
        l.press("a", 0);
        assert!(l.release(1_000));
        assert!(!l.release(2_000));
        l.tidy(10_000_000);
        assert!(l.press.is_empty() && !l.busy(10_000_000));
    }

    #[test]
    fn an_app_dragged_off_the_dock_leaves_no_icon_behind() {
        let leaving = |key: &str| Leaving { key: key.into(), icon: gdk::Paintable::new_empty(1, 1), center: 10.0, size: 50.0, shown: Tween::still(1.0) };
        let mut l = Lively { leaving: vec![leaving("a"), leaving("b")], ..Default::default() };
        l.leave_from("a", None);
        assert_eq!(l.leaving.len(), 1);
        l.leave_from("b", Some(70.0));
        assert_eq!(l.leaving[0].center, 70.0);
    }
}
