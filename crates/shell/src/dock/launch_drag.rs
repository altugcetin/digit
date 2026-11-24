//! Hyprland can break the dock's pointer grab when no window has keyboard focus.
//! Use a Wayland drag and a temporary drop target so the compositor tracks the gesture.

use super::*;

const MIME: &str = "application/x-digit-dock-app";

/// The picture that follows the pointer. The dock draws the app while it's over the dock, so
/// the picture is empty there; GTK puts up its first frames late and off their place.
pub(super) struct Visual {
    picture: gtk::Picture,
    icon: gdk::Paintable,
    empty: gdk::Paintable,
}

impl Visual {
    fn show(&self, on: bool) {
        self.picture.set_paintable(Some(if on { &self.icon } else { &self.empty }));
    }
}

pub(super) fn enabled(s: &State) -> bool {
    s.lifted.is_none() && s.app.prefs.borrow().dock.drag_to_open && kit::wm::can_place()
}

pub(super) fn wire(st: &Shared, paint: &Paint) {
    let source = gtk::DragSource::new();
    source.set_propagation_phase(gtk::PropagationPhase::Capture);
    source.set_actions(gdk::DragAction::COPY | gdk::DragAction::MOVE);
    let pending = Rc::new(RefCell::new(None));
    let (weak, candidate) = (Rc::downgrade(st), pending.clone());
    source.connect_prepare(move |_, x, y| {
        let st = weak.upgrade()?;
        let s = st.borrow();
        if !enabled(&s) {
            return None;
        }
        *candidate.borrow_mut() = drag_from(&s, x, y);
        candidate.borrow().as_ref()?;
        Some(gdk::ContentProvider::for_bytes(MIME, &glib::Bytes::from_static(b"digit")))
    });
    let weak = Rc::downgrade(st);
    source.connect_drag_begin(move |_, system| {
        let (Some(st), Some(mut drag)) = (weak.upgrade(), pending.borrow_mut().take()) else { return };
        let icon = st.borrow().drag_icon(&drag);
        let visual = icon.map(|(icon, spot)| {
            let picture = kit::drag::set_icon(system, &icon, spot);
            let empty = gdk::Paintable::new_empty(icon.intrinsic_width(), icon.intrinsic_height());
            let visual = Visual { picture, icon, empty };
            visual.show(false);
            visual
        });
        {
            let mut s = st.borrow_mut();
            drag.moving = true;
            s.drag = Some(drag);
            s.drag_visual = visual;
            s.hover = None;
            s.tip.set(0.0);
            begin_placement(&st, &mut s);
        }
        show_target(&st);
    });
    let weak = Rc::downgrade(st);
    source.connect_drag_end(move |_, _, _| {
        let Some(st) = weak.upgrade() else { return };
        close(&st);
        let visual = st.borrow_mut().drag_visual.take();
        drop(visual);
        // With no drop target to close, the drag still ends here.
        drop_drag(&st, false);
        watch(&st);
    });
    paint.add_controller(source);
}

/// Keeps the dock as if the pointer were on it or off it, which it doesn't see itself while
/// the drop target covers it. A drop then leaves it open under the pointer.
fn hold(st: &Shared) {
    let mut s = st.borrow_mut();
    let Some(over) = s.drag.as_ref().map(|d| aimed_at_dock(&s, d)) else { return };
    if over != s.inside {
        s.inside = over;
        s.reveal = over;
        // A leave from before doesn't count any more.
        s.leave += 1;
    }
}

/// After a drag, until the pointer is known to enter the dock or to be away from it: a
/// pointer that moved off meanwhile never leaves a dock it didn't enter.
fn watch(st: &Shared) {
    let (generation, output) = {
        let s = st.borrow();
        (s.leave, s.output.clone())
    };
    let weak = Rc::downgrade(st);
    glib::spawn_future_local(async move {
        loop {
            glib::timeout_future(Duration::from_millis(400)).await;
            let out = output.clone();
            let at = gio::spawn_blocking(move || kit::wm::pointer_on(&out)).await.ok().flatten();
            let Some(st) = weak.upgrade() else { return };
            let over = {
                let Ok(s) = st.try_borrow() else { continue };
                if s.leave != generation || s.drag.is_some() {
                    return;
                }
                let screen = kit::screens::by_name(&s.output).map(|m| (f64::from(m.geometry().width()), f64::from(m.geometry().height())));
                s.inside && at.zip(screen).is_some_and(|(at, screen)| s.over_body(from_screen(s.geo.side, at, screen), glib::monotonic_time()))
            };
            if !over {
                return left(&st);
            }
        }
    });
}

/// Passes the app's icon between the dock and the drag as it leaves the dock or comes back.
fn hand_over(st: &Shared) {
    let visual = {
        let mut s = st.borrow_mut();
        let Some(out) = s.drag.as_ref().map(|d| !aimed_at_dock(&s, d)) else { return };
        let Some(d) = s.drag.as_mut().filter(|d| d.away.is_some() != out) else { return };
        d.away = out.then(glib::monotonic_time);
        s.paint.queue_draw();
        let Some(visual) = s.drag_visual.take() else { return };
        visual
    };
    visual.show(st.borrow().drag.as_ref().is_some_and(|d| d.away.is_some()));
    st.borrow_mut().drag_visual = Some(visual);
    // The dock lets go of it once the drag's icon is up.
    let weak = Rc::downgrade(st);
    glib::timeout_add_local_once(Duration::from_millis(100), move || {
        if let Some(st) = weak.upgrade() && let Ok(s) = st.try_borrow() {
            s.paint.queue_draw();
        }
    });
}

fn show_target(st: &Shared) {
    let Some(monitor) = kit::screens::by_name(&st.borrow().output) else { return };
    let window = gtk::Window::new();
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace(Some("digit-dock-drop"));
    window.set_monitor(Some(&monitor));
    window.set_keyboard_mode(KeyboardMode::None);
    window.set_exclusive_zone(-1);
    window.add_css_class("digit");
    for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(edge, true);
    }
    let action = gdk::DragAction::COPY;
    let target = gtk::DropTargetAsync::new(Some(gdk::ContentFormats::new(&[MIME])), action | gdk::DragAction::MOVE);
    let weak = Rc::downgrade(st);
    let motion = move |target: &gtk::DropTargetAsync, _: &gdk::Drop, x, y| {
        let (Some(st), Some(widget)) = (weak.upgrade(), target.widget()) else { return gdk::DragAction::empty() };
        let point = from_screen(st.borrow().geo.side, (x, y), (f64::from(widget.width()), f64::from(widget.height())));
        drag_at(&st, point);
        // The dock itself doesn't see the pointer under this surface.
        moved_to(&st, point);
        hand_over(&st);
        hold(&st);
        action
    };
    // Hyprland enters a surface at its middle, wherever the pointer is; the motion after it
    // tells the place.
    target.connect_drag_enter(move |_, _, _, _| action);
    target.connect_drag_motion(motion);
    let weak = Rc::downgrade(st);
    target.connect_drop(move |_, drop, _, _| {
        let Some(st) = weak.upgrade() else { return false };
        if st.borrow().drag.is_none() {
            return false;
        }
        // GDK can report the pointer's pre-drag position on release.
        drop_drag(&st, true);
        drop.finish(action);
        true
    });
    let paint = Paint::new(1, 1);
    paint.present();
    paint.add_controller(target);
    window.set_child(Some(&paint));
    st.borrow_mut().drag_target = Some(window.clone());
    window.present();
}

pub(super) fn close(st: &Shared) {
    let window = st.borrow_mut().drag_target.take();
    if let Some(window) = window {
        window.destroy();
        drop_drag(st, false);
    }
}
