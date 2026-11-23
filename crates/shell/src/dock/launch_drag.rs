//! Hyprland can break the dock's pointer grab when no window has keyboard focus.
//! Use a Wayland drag and a temporary drop target so the compositor tracks the gesture.

use super::*;

const MIME: &str = "application/x-digit-dock-app";

pub(super) fn enabled(s: &State) -> bool {
    kit::wm::which() == kit::wm::Wm::Hyprland && placement_enabled(s)
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
        {
            let mut s = st.borrow_mut();
            if let Some(icon) = s.icons.get(&drag.key).and_then(Option::as_ref) {
                let half = (s.geo.base / 2.0) as i32;
                kit::drag::set_icon(system, icon, (half, half));
            }
            drag.moving = true;
            s.drag = Some(drag);
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
    });
    paint.add_controller(source);
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
        action
    };
    target.connect_drag_enter(motion.clone());
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
