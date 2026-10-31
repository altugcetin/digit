//! Smooth mouse-wheel scrolling for the app windows. Touchpads already scroll smoothly; a wheel
//! moves in whole steps, so each notch is turned into a short glide of the page under the pointer.

use gtk::{glib, prelude::*};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// How quickly a glide closes in on its target; smaller is snappier.
const TAU_US: f64 = 70_000.0;

thread_local! {
    static ENABLED: Cell<(bool, i64)> = const { Cell::new((true, i64::MIN)) };
}

/// The "smooth_scroll" preference, re-read at most every two seconds so toggling it applies live.
fn enabled() -> bool {
    let now = glib::monotonic_time();
    ENABLED.with(|e| {
        let (on, at) = e.get();
        if now.saturating_sub(at) < 2_000_000 {
            return on;
        }
        let on = crate::prefs::flag(&crate::prefs::read(), "smooth_scroll", true);
        e.set((on, now));
        on
    })
}

struct Glide {
    target: f64,
    /// The value the glide last set, to notice when something else (a scrollbar drag) moved it.
    last: f64,
    at: i64,
    ticking: bool,
}

/// Turns wheel steps anywhere in `window` into smooth scrolling of the scrolled area under the pointer.
pub fn smooth(window: &impl IsA<gtk::Window>) {
    let window = window.as_ref().clone();
    let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
    wheel.set_propagation_phase(gtk::PropagationPhase::Capture);
    // Scroll events carry no position, so the pointer is followed separately.
    let pointer: Rc<Cell<Option<(f64, f64)>>> = Rc::new(Cell::new(None));
    let motion = gtk::EventControllerMotion::new();
    motion.set_propagation_phase(gtk::PropagationPhase::Capture);
    let p = pointer.clone();
    motion.connect_enter(move |_, x, y| p.set(Some((x, y))));
    let p = pointer.clone();
    motion.connect_motion(move |_, x, y| p.set(Some((x, y))));
    let p = pointer.clone();
    motion.connect_leave(move |_| p.set(None));
    window.add_controller(motion);
    let w = window.clone();
    wheel.connect_scroll(move |c, dx, dy| {
        if !enabled() || c.unit() != gtk::gdk::ScrollUnit::Wheel {
            return glib::Propagation::Proceed;
        }
        let Some((x, y)) = pointer.get() else { return glib::Propagation::Proceed };
        let Some(target) = w.pick(x, y, gtk::PickFlags::DEFAULT) else { return glib::Propagation::Proceed };
        // Vertical wheels may scroll sideways where only that is possible, as GTK does.
        let (delta, vertical) = if dy != 0.0 { (dy, true) } else { (dx, false) };
        let mut at = Some(target);
        while let Some(widget) = at {
            if let Some(sw) = widget.downcast_ref::<gtk::ScrolledWindow>() {
                for axis in [vertical, !vertical] {
                    let adj = if axis { sw.vadjustment() } else { sw.hadjustment() };
                    let policy = if axis { sw.vscrollbar_policy() } else { sw.hscrollbar_policy() };
                    if policy != gtk::PolicyType::Never && adj.upper() - adj.page_size() > 0.5 {
                        glide(sw.upcast_ref(), &adj, delta);
                        return glib::Propagation::Stop;
                    }
                }
            }
            at = widget.parent();
        }
        glib::Propagation::Proceed
    });
    window.add_controller(wheel);
}

fn glide(widget: &gtk::Widget, adj: &gtk::Adjustment, delta: f64) {
    let max = adj.upper() - adj.page_size();
    // GTK's own wheel step.
    let step = adj.page_size().powf(2.0 / 3.0);
    let now = widget.frame_clock().map_or_else(glib::monotonic_time, |c| c.frame_time());
    let state = unsafe { adj.data::<Rc<RefCell<Glide>>>("digit-glide").map(|p| p.as_ref().clone()) };
    let state = state.unwrap_or_else(|| {
        let s = Rc::new(RefCell::new(Glide { target: adj.value(), last: adj.value(), at: now, ticking: false }));
        unsafe { adj.set_data("digit-glide", s.clone()) };
        s
    });
    let start = {
        let mut g = state.borrow_mut();
        // Begin from where the page really is unless a glide is still running.
        if !g.ticking || (adj.value() - g.last).abs() > 1.0 {
            g.target = adj.value();
        }
        g.target = (g.target + delta * step).clamp(0.0, max);
        g.last = adj.value();
        g.at = now;
        !std::mem::replace(&mut g.ticking, true)
    };
    if !start {
        return;
    }
    let adj = adj.clone();
    widget.add_tick_callback(move |_, clock| {
        let mut g = state.borrow_mut();
        if (adj.value() - g.last).abs() > 1.0 {
            g.ticking = false;
            return glib::ControlFlow::Break;
        }
        let now = clock.frame_time();
        let dt = (now - g.at) as f64;
        g.at = now;
        let v = g.target + (adj.value() - g.target) * (-dt / TAU_US).exp();
        let done = (v - g.target).abs() < 0.5;
        let v = if done { g.target } else { v };
        adj.set_value(v);
        g.last = adj.value();
        if done {
            g.ticking = false;
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
}
