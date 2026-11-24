//! The notifications' movement: one that goes slides aside, and the ones under it move up.

use super::ROW;
use crate::app::Notice;
use kit::motion::{self, Part, Tween};

const LEAVE_MS: i64 = 220;
/// Between one row's leaving and the next one's, when they all go.
const STAGGER_MS: i64 = 35;
const STAGGERED: i64 = 8;

struct Here {
    id: u32,
    /// How far it still is from its place.
    shift: Tween,
    shown: Tween,
}

struct Gone {
    notice: Notice,
    at: f64,
    shown: Tween,
}

#[derive(Default)]
pub(super) struct Notes {
    here: Vec<Here>,
    gone: Vec<Gone>,
    last: Vec<Notice>,
    /// Nothing has been shown yet, so nothing moves.
    known: bool,
    /// While they're all swept away, the room they had and when it goes.
    hold: Option<(f64, i64)>,
}

pub(super) struct Row<'a> {
    pub(super) notice: &'a Notice,
    /// Its place in the list, unless it's leaving.
    pub(super) index: Option<usize>,
    pub(super) at: f64,
    pub(super) shown: f64,
}

impl Notes {
    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }

    /// Takes in what the list holds now.
    pub(super) fn sync(&mut self, list: &[Notice], now: i64) {
        if self.known && self.last.len() == list.len() && self.here.iter().zip(list).all(|(h, n)| h.id == n.id) {
            return;
        }
        let place = |i: usize| i as f64 * ROW;
        let still = !std::mem::replace(&mut self.known, true) || !motion::on(Part::Notifications);
        let old = std::mem::take(&mut self.here);
        if !still {
            let sweep = list.is_empty() && self.last.len() > 1;
            let mut order = 0;
            for (i, notice) in self.last.iter().enumerate().filter(|(_, n)| !list.iter().any(|l| l.id == n.id)) {
                let mut shown = Tween::still(1.0);
                let delay = if sweep { order.min(STAGGERED) * STAGGER_MS } else { 0 };
                shown.after(0.0, now, delay, LEAVE_MS, motion::STANDARD);
                order += 1;
                self.gone.push(Gone { notice: notice.clone(), at: place(i), shown });
            }
            if sweep {
                self.hold = Some((place(self.last.len()), now + ((order - 1).min(STAGGERED) * STAGGER_MS + LEAVE_MS / 2) * 1000));
            }
        }
        self.gone.retain(|g| !list.iter().any(|n| n.id == g.notice.id));
        self.here = list.iter().enumerate().map(|(i, n)| {
            let was = old.iter().position(|h| h.id == n.id);
            let mut shift = Tween::still(was.filter(|_| !still).map_or(0.0, |w| place(w) + old[w].shift.value(now) - place(i)));
            shift.go(0.0, now, 260, motion::STANDARD);
            let shown = match was {
                Some(w) => old[w].shown,
                None if still => Tween::still(1.0),
                None => {
                    let mut t = Tween::still(0.0);
                    t.go(1.0, now, 220, motion::STANDARD);
                    t
                }
            };
            Here { id: n.id, shift, shown }
        }).collect();
        self.last = list.to_vec();
    }

    /// The rows to draw, the leaving ones under the others.
    pub(super) fn rows<'a>(&'a self, list: &'a [Notice], now: i64) -> Vec<Row<'a>> {
        let gone = self.gone.iter().map(|g| Row { notice: &g.notice, index: None, at: g.at, shown: g.shown.value(now).clamp(0.0, 1.0) });
        let here = list.iter().zip(&self.here).enumerate().map(|(i, (notice, h))| {
            Row { notice, index: Some(i), at: (i as f64 * ROW + h.shift.value(now)).round(), shown: h.shown.value(now).clamp(0.0, 1.0) }
        });
        gone.chain(here).filter(|r| r.shown > 0.01).collect()
    }

    /// How much of the leaving rows is still there.
    pub(super) fn leaving(&self, now: i64) -> f64 {
        self.gone.iter().map(|g| g.shown.value(now)).fold(0.0, f64::max).clamp(0.0, 1.0)
    }

    /// The height kept for rows being swept away.
    pub(super) fn held(&self) -> f64 {
        self.hold.map_or(0.0, |(h, _)| h)
    }

    /// Drops what has left. Returns whether the held room goes now.
    pub(super) fn step(&mut self, now: i64) -> bool {
        self.gone.retain(|g| !g.shown.settled(now));
        self.hold.take_if(|(_, until)| now >= *until).is_some()
    }

    pub(super) fn busy(&self, now: i64) -> bool {
        self.hold.is_some() || !self.gone.is_empty() || self.here.iter().any(|h| !h.shift.settled(now) || !h.shown.settled(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: u32) -> Notice {
        Notice { id, app: String::new(), summary: String::new(), body: String::new(), icon: None, at: 0 }
    }

    fn ids(n: &Notes, list: &[Notice], now: i64) -> Vec<(u32, Option<usize>)> {
        n.rows(list, now).iter().map(|r| (r.notice.id, r.index)).collect()
    }

    #[test]
    fn a_removed_row_leaves_and_the_rest_keep_their_places_in_the_list() {
        let mut n = Notes::default();
        let all = [note(1), note(2), note(3)];
        n.sync(&all, 0);
        assert!(!n.busy(0), "nothing moves as the panel opens");
        let rest = [note(1), note(3)];
        n.sync(&rest, 1_000);
        assert_eq!(n.held(), 0.0);
        if motion::on(Part::Notifications) {
            assert_eq!(ids(&n, &rest, 1_000), [(2, None), (1, Some(0)), (3, Some(1))]);
            assert_eq!(n.rows(&rest, 1_000)[2].at, 2.0 * ROW, "it starts from where it was");
        }
        n.step(10_000_000);
        assert_eq!(ids(&n, &rest, 10_000_000), [(1, Some(0)), (3, Some(1))]);
        assert_eq!(n.rows(&rest, 10_000_000)[1].at, ROW);
        assert!(!n.busy(10_000_000));
    }

    #[test]
    fn clearing_keeps_the_room_until_the_rows_are_gone() {
        let mut n = Notes::default();
        n.sync(&[note(1), note(2)], 0);
        n.sync(&[], 1_000);
        if !motion::on(Part::Notifications) {
            return;
        }
        assert_eq!(n.held(), 2.0 * ROW);
        assert!(n.leaving(1_000) > 0.9 && !n.step(2_000));
        assert!(n.step(10_000_000));
        assert_eq!((n.held(), n.leaving(10_000_000)), (0.0, 0.0));
        assert!(!n.busy(10_000_000));
    }

    #[test]
    fn a_notification_that_comes_back_isnt_shown_twice() {
        let mut n = Notes::default();
        n.sync(&[note(1), note(2)], 0);
        n.sync(&[note(1)], 1_000);
        let back = [note(1), note(2)];
        n.sync(&back, 2_000);
        assert_eq!(ids(&n, &back, 10_000_000), [(1, Some(0)), (2, Some(1))]);
    }
}
