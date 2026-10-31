//! Smooth, Apple-like scrolling for the app windows. A wheel moves in whole steps, so each notch
//! becomes a short glide of the page under the pointer. At the top and bottom the page stretches
//! like rubber instead of stopping dead: a touchpad pulled past the edge meets growing resistance
//! and springs back when the fingers lift, and a glide or fling that runs into the edge bounces.

use gtk::{glib, prelude::*};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// How quickly a glide closes in on its target; smaller is snappier.
const TAU_US: f64 = 70_000.0;
/// How stiff the spring that pulls a stretched page back is (rad/s, critically damped).
const SPRING: f64 = 14.0;
/// How far, as a share of the visible height, the page can ever stretch.
const STRETCH: f64 = 0.35;

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
    /// Distance still to travel. The glide moves the page by part of it each frame instead of
    /// toward a fixed value, since list and grid views re-anchor their adjustment as rows are
    /// measured and a fixed target would fight them.
    left: f64,
    /// Sub-pixel travel not applied yet; the page only moves in whole pixels so text stays crisp.
    carry: f64,
    at: i64,
    ticking: bool,
}

/// The stretch past an edge: positive pulls the content down from the top, negative up from the bottom.
#[derive(Default)]
struct Elastic {
    /// How far the fingers have pulled; the page shows less of it, with growing resistance.
    pull: f64,
    /// The stretch on screen, and its speed in px/s while springing.
    shown: f64,
    speed: f64,
    /// Fingers are on the touchpad, so the spring waits.
    held: bool,
    ticking: bool,
    at: i64,
    /// The stretched widget's own margins, put back when the page settles.
    base: Option<(gtk::Widget, i32, i32)>,
    /// Recent travel of the page, to tell a fling that hits the edge from a jump.
    last: (f64, i64),
    velocity: f64,
}

/// Apple's rubber-band curve: the further the pull, the less the page follows.
fn rubber(pull: f64, size: f64) -> f64 {
    let d = size * STRETCH;
    pull.signum() * (1.0 - 1.0 / (pull.abs() * 0.55 / d + 1.0)) * d
}

fn unrubber(shown: f64, size: f64) -> f64 {
    let d = size * STRETCH;
    let k = (shown.abs() / d).min(0.999);
    shown.signum() * (1.0 / (1.0 - k) - 1.0) * d / 0.55
}

/// The widget whose margin stretches: a viewport's child, a list or grid view that scrolls
/// itself, or the rows inside a column view. Stretching the column view itself makes its
/// sticky header and rows re-anchor against each other and the text jumps; its rows alone
/// stretch cleanly under a header that stays put.
fn stretched(sw: &gtk::ScrolledWindow) -> Option<gtk::Widget> {
    let child = sw.child()?;
    if let Some(v) = child.downcast_ref::<gtk::Viewport>() {
        return v.child();
    }
    if child.is::<gtk::ColumnView>() {
        let mut c = child.first_child();
        while let Some(w) = c {
            if w.is::<gtk::ListView>() {
                return Some(w);
            }
            c = w.next_sibling();
        }
        return None;
    }
    Some(child)
}

fn elastic(sw: &gtk::ScrolledWindow) -> Rc<RefCell<Elastic>> {
    if let Some(e) = unsafe { sw.data::<Rc<RefCell<Elastic>>>("digit-elastic") } {
        return unsafe { e.as_ref() }.clone();
    }
    let e = Rc::new(RefCell::new(Elastic::default()));
    unsafe { sw.set_data("digit-elastic", e.clone()) };
    // Watch the page move so a fling that runs into an edge can bounce off it.
    let adj = sw.vadjustment();
    let (weak, state) = (sw.downgrade(), e.clone());
    adj.connect_value_changed(move |a| {
        let Some(sw) = weak.upgrade() else { return };
        let now = glib::monotonic_time();
        let max = a.upper() - a.page_size();
        // The stretch itself moves the page while it holds the state; that isn't a fling.
        let Ok(mut st) = state.try_borrow_mut() else { return };
        let (was, at) = st.last;
        let dt = (now - at) as f64 / 1e6;
        let step = a.value() - was;
        st.last = (a.value(), now);
        // Only steady motion counts; a jump (Page Down, Home) has no speed to carry.
        st.velocity = if dt > 0.0 && dt < 0.05 && step.abs() < 120.0 { step / dt } else { 0.0 };
        let edge = (a.value() <= 0.0 && st.velocity < 0.0) || (a.value() >= max && st.velocity > 0.0);
        if edge && st.velocity.abs() > 900.0 && !st.held && st.shown == 0.0 && enabled() {
            let v = -st.velocity.clamp(-4000.0, 4000.0) * 0.35;
            st.velocity = 0.0;
            drop(st);
            kick(&sw, v);
        }
    });
    e
}

/// Puts the stretch on screen by growing the margin on the pulled side.
fn show(sw: &gtk::ScrolledWindow, st: &mut Elastic) {
    let Some(w) = stretched(sw) else { return };
    if st.base.as_ref().is_none_or(|(b, _, _)| b != &w) {
        st.base = Some((w.clone(), w.margin_top(), w.margin_bottom()));
    }
    let (_, top, bottom) = st.base.clone().unwrap();
    let px = st.shown.round() as i32;
    w.set_margin_top(top + px.max(0));
    w.set_margin_bottom(bottom + (-px).max(0));
    if px < 0 {
        // A bottom margin only grows the page; keep it scrolled to the end so the content rises.
        let adj = sw.vadjustment();
        adj.set_value(adj.upper() - adj.page_size());
    }
}

fn settle(sw: &gtk::ScrolledWindow, st: &mut Elastic) {
    if let Some((w, top, bottom)) = st.base.take() {
        w.set_margin_top(top);
        w.set_margin_bottom(bottom);
    }
    st.shown = 0.0;
    st.pull = 0.0;
    st.speed = 0.0;
    let _ = sw;
}

/// Runs the spring that brings a stretched page back to rest.
fn spring(sw: &gtk::ScrolledWindow) {
    let e = elastic(sw);
    {
        let mut st = e.borrow_mut();
        if std::mem::replace(&mut st.ticking, true) {
            return;
        }
        st.at = sw.frame_clock().map_or_else(glib::monotonic_time, |c| c.frame_time());
    }
    sw.add_tick_callback(move |sw, clock| {
        let sw = sw.downcast_ref::<gtk::ScrolledWindow>().expect("scrolled window");
        let mut st = e.borrow_mut();
        let now = clock.frame_time();
        let mut dt = ((now - st.at).max(0) as f64 / 1e6).min(0.05);
        st.at = now;
        if st.held {
            st.ticking = false;
            return glib::ControlFlow::Break;
        }
        // Critically damped spring, in small steps so it stays stable on slow frames.
        while dt > 0.0 {
            let h = dt.min(1.0 / 240.0);
            let a = -SPRING * SPRING * st.shown - 2.0 * SPRING * st.speed;
            st.speed += a * h;
            st.shown += st.speed * h;
            dt -= h;
        }
        let size = sw.height().max(1) as f64;
        st.shown = st.shown.clamp(-size * STRETCH, size * STRETCH);
        if st.shown.abs() < 0.5 && st.speed.abs() < 20.0 {
            settle(sw, &mut st);
            st.ticking = false;
            return glib::ControlFlow::Break;
        }
        st.pull = unrubber(st.shown, size);
        show(sw, &mut st);
        glib::ControlFlow::Continue
    });
}

/// Bounces the page off an edge: `speed` in px/s, positive out of the top, negative out of the bottom.
fn kick(sw: &gtk::ScrolledWindow, speed: f64) {
    let e = elastic(sw);
    {
        let mut st = e.borrow_mut();
        if st.held {
            return;
        }
        st.speed += speed;
    }
    spring(sw);
}

/// The scrolled window under the pointer that can scroll along an axis, and its adjustment.
fn scroller(target: gtk::Widget, vertical: bool) -> Option<(gtk::ScrolledWindow, gtk::Adjustment)> {
    let mut at = Some(target);
    while let Some(widget) = at {
        if let Some(sw) = widget.downcast_ref::<gtk::ScrolledWindow>() {
            let adj = if vertical { sw.vadjustment() } else { sw.hadjustment() };
            let policy = if vertical { sw.vscrollbar_policy() } else { sw.hscrollbar_policy() };
            if policy != gtk::PolicyType::Never && adj.upper() - adj.page_size() > 0.5 {
                return Some((sw.clone(), adj));
            }
        }
        at = widget.parent();
    }
    None
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
    // The page being pulled with the touchpad, so lifting the fingers can let it spring back.
    let pulling: Rc<RefCell<Option<gtk::ScrolledWindow>>> = Rc::new(RefCell::new(None));
    let w = window.clone();
    let held = pulling.clone();
    wheel.connect_scroll(move |c, dx, dy| {
        // Ctrl, Shift or Alt with the wheel mean something else (zoom, sideways), so leave those alone.
        let mods = gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK | gtk::gdk::ModifierType::ALT_MASK;
        if !enabled() || c.current_event_state().intersects(mods) {
            return glib::Propagation::Proceed;
        }
        let Some((x, y)) = pointer.get() else { return glib::Propagation::Proceed };
        let Some(target) = w.pick(x, y, gtk::PickFlags::DEFAULT) else { return glib::Propagation::Proceed };
        if c.unit() != gtk::gdk::ScrollUnit::Wheel {
            return pull(&held, target, dy);
        }
        // Vertical wheels may scroll sideways where only that is possible, as GTK does.
        let (delta, vertical) = if dy != 0.0 { (dy, true) } else { (dx, false) };
        let Some((sw, adj)) = scroller(target.clone(), vertical).or_else(|| scroller(target, !vertical)) else { return glib::Propagation::Proceed };
        let bounce = (adj == sw.vadjustment() && stretched(&sw).is_some()).then(|| sw.clone());
        if bounce.is_some() {
            let e = elastic(&sw);
            let mut st = e.borrow_mut();
            if st.shown != 0.0 {
                // Scrolling away from a bouncing edge drops the stretch at once and scrolls on,
                // as on a Mac; scrolling further into it lets the bounce play out.
                if st.shown.signum() == delta.signum() {
                    settle(&sw, &mut st);
                } else {
                    return glib::Propagation::Stop;
                }
            }
        }
        glide(sw.upcast_ref(), &adj, delta, bounce);
        glib::Propagation::Stop
    });
    wheel.connect_scroll_end(move |_| {
        if let Some(sw) = pulling.borrow_mut().take() {
            elastic(&sw).borrow_mut().held = false;
            spring(&sw);
        }
    });
    window.add_controller(wheel);
}

/// A touchpad scroll: past an edge it stretches the page instead of letting GTK stop it dead.
fn pull(held: &Rc<RefCell<Option<gtk::ScrolledWindow>>>, target: gtk::Widget, dy: f64) -> glib::Propagation {
    let Some((sw, adj)) = scroller(target, true) else { return glib::Propagation::Proceed };
    if stretched(&sw).is_none() {
        return glib::Propagation::Proceed;
    }
    let e = elastic(&sw);
    let mut st = e.borrow_mut();
    let max = adj.upper() - adj.page_size();
    let at_top = adj.value() <= 0.0 && dy < 0.0;
    let at_bottom = adj.value() >= max - 0.5 && dy > 0.0;
    if st.shown == 0.0 && !at_top && !at_bottom {
        return glib::Propagation::Proceed;
    }
    let size = sw.height().max(1) as f64;
    if !st.held {
        // Catch a page that is still springing back, from where it is now.
        st.pull = unrubber(st.shown, size);
        st.speed = 0.0;
    }
    st.held = true;
    let was = st.pull;
    st.pull -= dy;
    // Pushing back past the edge ends the stretch; the rest scrolls normally next time.
    if was != 0.0 && st.pull.signum() != was.signum() {
        st.pull = 0.0;
    }
    st.shown = rubber(st.pull, size);
    if st.shown == 0.0 {
        st.held = false;
        settle(&sw, &mut st);
        held.borrow_mut().take();
        return glib::Propagation::Stop;
    }
    show(&sw, &mut st);
    drop(st);
    *held.borrow_mut() = Some(sw);
    glib::Propagation::Stop
}

fn glide(widget: &gtk::Widget, adj: &gtk::Adjustment, delta: f64, bounce: Option<gtk::ScrolledWindow>) {
    // GTK's own wheel step.
    let step = adj.page_size().powf(2.0 / 3.0);
    let now = widget.frame_clock().map_or_else(glib::monotonic_time, |c| c.frame_time());
    let state = unsafe { adj.data::<Rc<RefCell<Glide>>>("digit-glide").map(|p| p.as_ref().clone()) };
    let state = state.unwrap_or_else(|| {
        let s = Rc::new(RefCell::new(Glide { left: 0.0, carry: 0.0, at: now, ticking: false }));
        unsafe { adj.set_data("digit-glide", s.clone()) };
        s
    });
    let start = {
        let mut g = state.borrow_mut();
        // Turning back cancels what was left in the old direction.
        if g.left * delta < 0.0 {
            g.left = 0.0;
        }
        g.left += delta * step;
        if !g.ticking {
            g.at = now;
            g.carry = 0.0;
        }
        !std::mem::replace(&mut g.ticking, true)
    };
    if !start {
        return;
    }
    let adj = adj.clone();
    widget.add_tick_callback(move |_, clock| {
        let mut g = state.borrow_mut();
        let now = clock.frame_time();
        let dt = (now - g.at).max(0) as f64;
        g.at = now;
        let mut part = g.left * (1.0 - (-dt / TAU_US).exp());
        if (g.left - part).abs() < 0.5 {
            part = g.left;
        }
        g.left -= part;
        let travel = part + g.carry;
        let whole = travel.trunc();
        g.carry = travel - whole;
        let before = adj.value();
        let max = adj.upper() - adj.page_size();
        adj.set_value((before + whole).clamp(0.0, max.max(0.0)));
        // Stop at either end, or once the distance is used up.
        let stuck = whole != 0.0 && (adj.value() - before).abs() < 0.5;
        if g.left == 0.0 || stuck {
            if stuck {
                // Running into the edge with distance to spare bounces off it, gently.
                if let Some(sw) = &bounce {
                    let speed = (g.left + whole) / (TAU_US / 1e6);
                    kick(sw, -speed.clamp(-1600.0, 1600.0) * 0.3);
                }
            } else if g.carry.abs() >= 0.5 {
                adj.set_value((adj.value() + g.carry.signum()).clamp(0.0, max.max(0.0)));
            }
            g.left = 0.0;
            g.carry = 0.0;
            g.ticking = false;
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
}
