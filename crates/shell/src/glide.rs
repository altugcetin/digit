//! Values on the desktop widgets that glide to a new one instead of jumping to it, and the frames
//! that draw them on the way.

use crate::paint::Paint;
use gtk::{glib, prelude::*};
use kit::motion::{self, Tween};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// How long a value takes to reach a new one.
const MS: i64 = 650;

/// One widget's gliding values, by name.
#[derive(Default)]
pub struct Glides {
    values: HashMap<String, (Tween, bool)>,
    on: bool,
    ticking: bool,
}

impl Glides {
    /// Starts a frame; `on`: values glide (the desktop widgets' animations are on). Values not
    /// asked for since the last frame are forgotten.
    pub fn begin(&mut self, on: bool) {
        self.on = on;
        self.values.retain(|_, (_, seen)| std::mem::replace(seen, false));
    }

    /// `key`'s value as shown at `now`: `to` once it got there, on its way from what it showed
    /// before when `to` changed by at least `least`; a smaller change, and the first value, show
    /// at once.
    pub fn at(&mut self, key: &str, to: f64, least: f64, now: i64) -> f64 {
        let on = self.on;
        let (t, seen) = self.values.entry(key.to_string()).or_insert_with(|| (Tween::still(to), false));
        *seen = true;
        let change = (t.target() - to).abs();
        if !on || (change > 1e-9 && change < least) {
            t.set(to);
        } else if change > 1e-9 {
            t.go(to, now, MS, motion::STANDARD);
        }
        t.value(now)
    }

    /// Whether any is still on its way at `now`.
    pub fn moving(&self, now: i64) -> bool {
        self.values.values().any(|(t, _)| !t.settled(now))
    }
}

/// After a frame of `paint`: keeps drawing frames while any of `glides` is on its way.
pub fn follow(paint: &Paint, glides: &Rc<RefCell<Glides>>) {
    let start = {
        let Ok(mut g) = glides.try_borrow_mut() else { return };
        g.moving(glib::monotonic_time()) && !std::mem::replace(&mut g.ticking, true)
    };
    if !start {
        return;
    }
    let glides = glides.clone();
    paint.add_tick_callback(move |paint, clock| {
        paint.queue_draw();
        let mut g = glides.borrow_mut();
        if g.moving(clock.frame_time()) {
            glib::ControlFlow::Continue
        } else {
            g.ticking = false;
            glib::ControlFlow::Break
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_shows_at_once_then_glides_to_each_new_one() {
        let mut g = Glides::default();
        g.begin(true);
        assert_eq!(g.at("cpu", 0.4, 0.1, 0), 0.4);
        g.begin(true);
        let half = g.at("cpu", 0.8, 0.1, 1_000_000);
        assert_eq!(half, 0.4, "it starts where it was");
        g.begin(true);
        let later = g.at("cpu", 0.8, 0.1, 1_000_000 + MS * 500);
        assert!(later > 0.4 && later < 0.8);
        g.begin(true);
        assert_eq!(g.at("cpu", 0.8, 0.1, 1_000_000 + MS * 1000), 0.8);
        // A small change just shows.
        g.begin(true);
        assert_eq!(g.at("cpu", 0.85, 0.1, 2_000_000), 0.85);
        assert!(!g.moving(2_000_000));
        // Off, a value jumps.
        g.begin(false);
        assert_eq!(g.at("cpu", 0.1, 0.1, 3_000_000), 0.1);
    }

    #[test]
    fn values_no_longer_asked_for_are_forgotten() {
        let mut g = Glides::default();
        g.begin(true);
        g.at("phone", 0.5, 0.1, 0);
        g.at("mouse", 0.2, 0.1, 0);
        g.begin(true);
        g.at("mouse", 0.2, 0.1, 0);
        g.begin(true);
        assert_eq!(g.values.len(), 1);
        assert!(g.values.contains_key("mouse"));
    }
}
