use gtk::{pango, prelude::*};

pub fn hue(name: &str) -> &'static str {
    match name {
        "computer" | "wifi" | "wifi_off" | "wifi_find" | "wifi_lock" | "signal_wifi_0_bar" | "network_wifi_1_bar" | "network_wifi_2_bar" | "network_wifi_3_bar" | "signal_wifi_4_bar" | "bluetooth" | "bluetooth_disabled" | "bluetooth_searching" | "view_column" | "package_2" | "dock_to_bottom" | "side_navigation" | "select_window" | "description" => "blue",
        "system_update_alt" | "check_circle" | "memory" | "good" => "green",
        "apps" | "dashboard" | "swap_vert" => "purple",
        "palette" | "style" | "interests" => "pink",
        "developer_board" | "dark_mode" => "indigo",
        "memory_alt" | "photo_size_select_small" | "zoom_in" => "teal",
        "deployed_code" | "touch_app" => "orange",
        "bolt" => "yellow",
        "error" | "notifications" => "red",
        "hard_drive" | "terminal" | "info" | "north_west" => "gray",
        _ => ["blue", "green", "purple", "pink", "indigo", "teal", "orange"][name.bytes().map(|b| b as usize).sum::<usize>() % 7],
    }
}

pub fn tile(name: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(name));
    l.add_css_class("sym");
    l.add_css_class("tile");
    l.add_css_class(hue(name));
    l.set_valign(gtk::Align::Center);
    l
}

pub fn sym(name: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(name));
    l.add_css_class("sym");
    l
}

pub fn text(value: &str, classes: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(value));
    l.set_xalign(0.0);
    for c in classes.split_whitespace() {
        l.add_css_class(c);
    }
    l
}

pub fn page(title: &str, lead: &str) -> (gtk::ScrolledWindow, gtk::Box) {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 4);
    body.add_css_class("page");
    body.append(&text(title, "title"));
    if !lead.is_empty() {
        let l = text(lead, "lead");
        l.set_wrap(true);
        body.append(&l);
    }
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
    scroll.set_vexpand(true);
    scroll.set_child(Some(&body));
    (scroll, body)
}

pub fn heading(value: &str) -> gtk::Label {
    text(value, "heading")
}

pub fn card() -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 2);
    b.add_css_class("card");
    b
}

pub fn row(icon: Option<&str>, name: &str, sub: &str) -> (gtk::Box, gtk::Box) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    row.add_css_class("row");
    if let Some(i) = icon {
        row.append(&tile(i));
    }
    let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
    col.set_hexpand(true);
    col.set_valign(gtk::Align::Center);
    let n = text(name, "name");
    n.set_ellipsize(pango::EllipsizeMode::End);
    col.append(&n);
    if !sub.is_empty() {
        let s = text(sub, "sub");
        s.set_ellipsize(pango::EllipsizeMode::End);
        col.append(&s);
    }
    row.append(&col);
    let end = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    end.set_valign(gtk::Align::Center);
    row.append(&end);
    (row, end)
}

pub fn button(label: &str, kind: &str) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    b.add_css_class("pill");
    if !kind.is_empty() {
        b.add_css_class(kind);
    }
    b.set_valign(gtk::Align::Center);
    b
}

pub fn badge(value: &str, kind: &str) -> gtk::Label {
    let l = text(value, "badge");
    if !kind.is_empty() {
        l.add_css_class(kind);
    }
    l.set_valign(gtk::Align::Center);
    l
}

pub fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

pub fn root(w: &impl IsA<gtk::Widget>) -> Option<gtk::Window> {
    w.root().and_then(|r| r.downcast::<gtk::Window>().ok())
}


/// The overlay a layer-shell surface keeps over its content, where its dialogs sit
/// (a separate toplevel would open underneath the overlay layer).
pub const SHEET_LAYER: &str = "sheet-layer";

fn ask_card(title: &str, detail: &str, cancel: &str, action: &str, kind: &str) -> (gtk::Box, gtk::Button, gtk::Button) {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
    content.add_css_class("dialog");
    content.set_size_request(420, -1);
    let t = text(title, "name");
    t.add_css_class("dialog-title");
    t.set_wrap(true);
    t.set_max_width_chars(44);
    content.append(&t);
    if !detail.is_empty() {
        let d = text(detail, "dim");
        d.set_wrap(true);
        d.set_max_width_chars(52);
        content.append(&d);
    }
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    buttons.set_halign(gtk::Align::End);
    buttons.set_margin_top(10);
    let no = button(cancel, "");
    let go = button(action, kind);
    buttons.append(&no);
    buttons.append(&go);
    content.append(&buttons);
    (content, no, go)
}

pub fn ask(parent: Option<&gtk::Window>, title: &str, detail: &str, cancel: &str, action: &str, kind: &str, ok: impl FnOnce() + 'static) {
    let (content, no, go) = ask_card(title, detail, cancel, action, kind);
    if let Some(layer) = parent.and_then(|p| p.child()).and_downcast::<gtk::Overlay>().filter(|o| o.widget_name() == SHEET_LAYER) {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("sheet-backdrop");
        root.set_hexpand(true);
        root.set_vexpand(true);
        content.set_halign(gtk::Align::Center);
        content.set_valign(gtk::Align::Center);
        content.set_vexpand(true);
        root.append(&content);
        root.set_opacity(0.0);
        layer.add_overlay(&root);
        settle(&root, &content, 0, true, || {});
        let closing = std::rc::Rc::new(std::cell::Cell::new(false));
        let close: std::rc::Rc<dyn Fn()> = {
            let (root, content) = (root.clone(), content.clone());
            std::rc::Rc::new(move || {
                if closing.replace(true) {
                    return;
                }
                let (layer, gone) = (layer.clone(), root.clone());
                settle(&root, &content, 0, false, move || layer.remove_overlay(&gone));
            })
        };
        let c = close.clone();
        no.connect_clicked(move |_| c());
        let c = close.clone();
        let ok = std::cell::RefCell::new(Some(ok));
        go.connect_clicked(move |_| {
            c();
            let confirmed = ok.borrow_mut().take();
            if let Some(f) = confirmed {
                f();
            }
        });
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                close();
                return gtk::glib::Propagation::Stop;
            }
            gtk::glib::Propagation::Proceed
        });
        root.add_controller(keys);
        no.grab_focus();
        return;
    }
    let window = gtk::Window::builder().modal(true).decorated(false).resizable(false).build();
    window.set_transient_for(parent);
    window.set_title(Some(title));
    window.add_css_class("digit-dialog");
    window.set_child(Some(&content));
    float_over(&window, parent);
    let w = window.clone();
    no.connect_clicked(move |_| w.close());
    let w = window.clone();
    let ok = std::cell::RefCell::new(Some(ok));
    go.connect_clicked(move |_| {
        w.close();
        let confirmed = ok.borrow_mut().take();
        if let Some(f) = confirmed {
            f();
        }
    });
    let keys = gtk::EventControllerKey::new();
    let w = window.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            w.close();
            return gtk::glib::Propagation::Stop;
        }
        gtk::glib::Propagation::Proceed
    });
    window.add_controller(keys);
    window.present();
    no.grab_focus();
}

/// Makes a small dialog window float over `parent` when the parent is a layer-shell overlay;
/// as an ordinary toplevel it would open underneath it. Call before presenting the dialog.
pub fn float_over(dialog: &gtk::Window, parent: Option<&gtk::Window>) {
    use gtk4_layer_shell::{KeyboardMode, Layer, LayerShell};
    if !parent.is_some_and(|p| p.is_layer_window()) || dialog.is_layer_window() {
        return;
    }
    dialog.init_layer_shell();
    dialog.set_namespace(Some("digit-dialog"));
    dialog.set_layer(Layer::Overlay);
    dialog.set_keyboard_mode(KeyboardMode::Exclusive);
}

/// Lets an overlay step below ordinary windows while something it opened (a file chooser,
/// a terminal, a browser) is in use; it comes back on top once it gets focus again.
pub fn step_aside(w: &gtk::Window) {
    use gtk4_layer_shell::{KeyboardMode, Layer, LayerShell};
    if !w.is_layer_window() {
        return;
    }
    w.set_layer(Layer::Bottom);
    w.set_keyboard_mode(KeyboardMode::OnDemand);
    unsafe {
        if w.data::<bool>("digit-aside").is_none() {
            w.connect_is_active_notify(|w| {
                if w.is_active() && w.layer() == Layer::Bottom {
                    w.set_layer(Layer::Overlay);
                    w.set_keyboard_mode(KeyboardMode::Exclusive);
                }
            });
            w.set_data("digit-aside", true);
        }
    }
}

/// Builds the inside of a full-screen layer surface: a dimmed backdrop with `card` centred on it,
/// under an overlay that `ask` puts its dialogs into. Returns the overlay and the backdrop.
pub fn overlay_card(card: &impl IsA<gtk::Widget>) -> (gtk::Overlay, gtk::Box) {
    let layer = gtk::Overlay::new();
    layer.set_widget_name(SHEET_LAYER);
    let backdrop = gtk::Box::new(gtk::Orientation::Vertical, 0);
    backdrop.add_css_class("overlay-backdrop");
    let card = card.as_ref();
    card.add_css_class("overlay-card");
    card.set_halign(gtk::Align::Center);
    card.set_valign(gtk::Align::Center);
    card.set_vexpand(true);
    card.set_overflow(gtk::Overflow::Hidden);
    backdrop.append(card);
    layer.set_child(Some(&backdrop));
    (layer, backdrop)
}

/// How close to a card's edge, inside or out, a press starts a resize.
const GRIP: f64 = 10.0;

/// A size kept by `overlay_handling` under `key`, if there is one.
pub fn remembered(key: &str) -> Option<(i32, i32)> {
    let t = crate::prefs::read();
    let a = t.get(key)?.as_array()?;
    Some((a.first()?.as_integer()? as i32, a.get(1)?.as_integer()? as i32))
}

/// Gives an overlay card from `overlay_card` window-like handling: dragging any of its edges or
/// corners resizes it (it stays centred, so the opposite side moves too), and a click on the empty
/// backdrop closes the window.
pub fn overlay_handling(window: &gtk::Window, backdrop: &gtk::Box, card: &impl IsA<gtk::Widget>, remember: Option<&'static str>) {
    use std::cell::Cell;
    use std::rc::Rc;
    let card = card.as_ref().clone();
    // The sides being dragged (-1, 0 or 1 across and down) and the size it started from.
    type Grab = ((f64, f64), (i32, i32));
    let resize: Rc<Cell<Option<Grab>>> = Rc::new(Cell::new(None));
    let edge = {
        let (card, backdrop) = (card.clone(), backdrop.clone());
        move |x: f64, y: f64| -> Option<(f64, f64)> {
            let b = card.compute_bounds(&backdrop)?;
            let (l, t, r, btm) = (b.x() as f64, b.y() as f64, (b.x() + b.width()) as f64, (b.y() + b.height()) as f64);
            if x < l - GRIP || x > r + GRIP || y < t - GRIP || y > btm + GRIP {
                return None;
            }
            let near = |v: f64, a: f64, z: f64| if (v - a).abs() <= GRIP { -1.0 } else if (v - z).abs() <= GRIP { 1.0 } else { 0.0 };
            let (sx, sy) = (near(x, l, r), near(y, t, btm));
            (sx != 0.0 || sy != 0.0).then_some((sx, sy))
        }
    };
    let cursor = |(sx, sy): (f64, f64)| match (sx as i32, sy as i32) {
        (0, _) => "ns-resize",
        (_, 0) => "ew-resize",
        (a, b) if a == b => "nwse-resize",
        _ => "nesw-resize",
    };
    let inside = {
        let (card, backdrop) = (card.clone(), backdrop.clone());
        move |x: f64, y: f64| card.compute_bounds(&backdrop).is_some_and(|b| b.contains_point(&gtk::graphene::Point::new(x as f32, y as f32)))
    };
    let motion = gtk::EventControllerMotion::new();
    motion.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let (edge, backdrop, resize) = (edge.clone(), backdrop.clone(), resize.clone());
        motion.connect_motion(move |_, x, y| {
            if resize.get().is_none() {
                backdrop.set_cursor_from_name(edge(x, y).map(cursor));
            }
        });
    }
    backdrop.add_controller(motion);
    let drag = gtk::GestureDrag::new();
    drag.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let (edge, card, resize) = (edge.clone(), card.clone(), resize.clone());
        drag.connect_drag_begin(move |g, x, y| match edge(x, y) {
            Some(sides) => {
                resize.set(Some((sides, (card.width(), card.height()))));
                g.set_state(gtk::EventSequenceState::Claimed);
            }
            None => {
                g.set_state(gtk::EventSequenceState::Denied);
            }
        });
    }
    {
        let (card, backdrop, resize) = (card.clone(), backdrop.clone(), resize.clone());
        drag.connect_drag_update(move |_, dx, dy| {
            let Some(((sx, sy), (w0, h0))) = resize.get() else { return };
            let (wmax, hmax) = ((backdrop.width() - 32).max(480), (backdrop.height() - 32).max(360));
            let (cw, ch) = card.size_request();
            let w = if sx != 0.0 { ((w0 as f64 + 2.0 * dx * sx).round() as i32).clamp(480, wmax) } else { cw };
            let h = if sy != 0.0 { ((h0 as f64 + 2.0 * dy * sy).round() as i32).clamp(360, hmax) } else { ch };
            card.set_size_request(w, h);
        });
    }
    {
        let resize = resize.clone();
        let card = card.clone();
        drag.connect_drag_end(move |_, _, _| {
            // Keep the size for next time, when the caller asks for it.
            if resize.take().is_some() {
                if let Some(key) = remember {
                    let (w, h) = card.size_request();
                    let _ = crate::prefs::set(key, toml::Value::Array(vec![toml::Value::Integer(w as i64), toml::Value::Integer(h as i64)]));
                }
            }
        });
    }
    backdrop.add_controller(drag);
    let click = gtk::GestureClick::new();
    let pressed_out = Rc::new(Cell::new(false));
    {
        let (inside, edge, pressed_out) = (inside.clone(), edge.clone(), pressed_out.clone());
        click.connect_pressed(move |_, _, x, y| pressed_out.set(!inside(x, y) && edge(x, y).is_none()));
    }
    {
        let window = window.clone();
        click.connect_released(move |_, _, x, y| {
            if pressed_out.replace(false) && !inside(x, y) && edge(x, y).is_none() && resize.get().is_none() {
                window.close();
            }
        });
    }
    backdrop.add_controller(click);
}

fn plain_popovers(w: &impl IsA<gtk::Widget>) {
    let mut child = w.first_child();
    while let Some(c) = child {
        if let Some(p) = c.downcast_ref::<gtk::Popover>() {
            p.set_has_arrow(false);
            p.add_css_class("menu-pop");
        }
        child = c.next_sibling();
    }
}

fn item_text(item: &gtk::ListItem) -> String {
    item.item().and_downcast::<gtk::StringObject>().map(|s| s.string().to_string()).unwrap_or_default()
}

pub fn dropdown(labels: &[&str], selected: u32) -> gtk::DropDown {
    let pick = gtk::DropDown::new(Some(gtk::StringList::new(labels)), gtk::Expression::NONE);
    pick.add_css_class("picker");
    pick.set_valign(gtk::Align::Center);
    // Reserve the same text slot for every selection, keeping long options ellipsized.
    let width = labels.iter().map(|s| s.chars().count()).max().unwrap_or(0).min(28) as i32;
    let face = gtk::SignalListItemFactory::new();
    face.connect_setup(move |_, obj| {
        let Some(item) = obj.downcast_ref::<gtk::ListItem>() else { return };
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let l = gtk::Label::new(None);
        l.set_xalign(0.0);
        l.set_ellipsize(pango::EllipsizeMode::End);
        l.set_width_chars(width);
        l.set_max_width_chars(width);
        row.append(&l);
        let chevron = sym("expand_more");
        chevron.add_css_class("pick-chevron");
        row.append(&chevron);
        item.set_child(Some(&row));
    });
    face.connect_bind(|_, obj| {
        let Some(item) = obj.downcast_ref::<gtk::ListItem>() else { return };
        if let Some(l) = item.child().and_then(|c| c.first_child()).and_downcast::<gtk::Label>() {
            l.set_text(&item_text(item));
        }
    });
    pick.set_factory(Some(&face));
    let rows = gtk::SignalListItemFactory::new();
    rows.connect_setup(|_, obj| {
        let Some(item) = obj.downcast_ref::<gtk::ListItem>() else { return };
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        row.add_css_class("pick-row");
        let l = gtk::Label::new(None);
        l.set_xalign(0.0);
        l.set_hexpand(true);
        row.append(&l);
        let check = sym("check");
        check.add_css_class("pick-check");
        check.set_opacity(0.0);
        row.append(&check);
        item.set_child(Some(&row));
        item.connect_selected_notify(move |it| check.set_opacity(if it.is_selected() { 1.0 } else { 0.0 }));
    });
    rows.connect_bind(|_, obj| {
        let Some(item) = obj.downcast_ref::<gtk::ListItem>() else { return };
        let Some(row) = item.child() else { return };
        if let Some(l) = row.first_child().and_downcast::<gtk::Label>() {
            l.set_text(&item_text(item));
        }
        if let Some(c) = row.last_child() {
            c.set_opacity(if item.is_selected() { 1.0 } else { 0.0 });
        }
    });
    pick.set_list_factory(Some(&rows));
    pick.set_selected(selected);
    plain_popovers(&pick);
    pick
}

pub fn segmented(labels: &[&str], selected: usize, changed: impl Fn(usize) + 'static) -> gtk::Box {
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    bar.add_css_class("seg");
    bar.set_valign(gtk::Align::Center);
    let changed = std::rc::Rc::new(changed);
    let mut first: Option<gtk::ToggleButton> = None;
    for (i, label) in labels.iter().enumerate() {
        let b = gtk::ToggleButton::with_label(label);
        match &first {
            Some(f) => b.set_group(Some(f)),
            None => first = Some(b.clone()),
        }
        b.set_active(i == selected);
        b.set_focus_on_click(false);
        let changed = changed.clone();
        b.connect_toggled(move |b| {
            if b.is_active() {
                changed(i);
            }
        });
        bar.append(&b);
    }
    bar
}

pub fn menu_popover(p: &impl IsA<gtk::Popover>) {
    let p = p.upcast_ref::<gtk::Popover>();
    p.set_has_arrow(false);
    p.add_css_class("menu-pop");
}

pub fn distro_logo(px: i32, symbolic: bool) -> gtk::Widget {
    match crate::distro::logo_name(symbolic) {
        Some(name) => {
            let img = gtk::Image::from_icon_name(&name);
            img.set_pixel_size(px);
            img.add_css_class("distro-logo");
            img.set_valign(gtk::Align::Center);
            img.upcast()
        }
        None => {
            let t = tile("deployed_code");
            t.add_css_class("big");
            t.upcast()
        }
    }
}

/// How far a dialog card starts below its resting place.
pub const SETTLE_DROP: f64 = 18.0;

/// Fades a dialog's backdrop and settles its card from slightly below, like the shell's panels;
/// closing runs it back, quicker. `margin` is the card's resting top and bottom margin.
pub fn settle(root: &gtk::Box, card: &gtk::Box, margin: i32, opening: bool, done: impl FnOnce() + 'static) {
    use crate::motion::{Curve, Tween};
    if crate::motion::reduced() {
        root.set_opacity(if opening { 1.0 } else { 0.0 });
        card.set_margin_top(margin);
        card.set_margin_bottom(margin);
        gtk::glib::idle_add_local_once(done);
        return;
    }
    let now = gtk::glib::monotonic_time();
    let mut t = Tween::still(root.opacity());
    if opening {
        t.go(1.0, now, 260, Curve::new(0.16, 1.0, 0.3, 1.0));
    } else {
        t.go(0.0, now, 150, Curve::new(0.4, 0.0, 0.9, 0.6));
    }
    let card = card.clone();
    let done = std::cell::Cell::new(Some(done));
    root.add_tick_callback(move |w, clock| {
        let now = clock.frame_time();
        let v = t.value(now).clamp(0.0, 1.0);
        w.set_opacity(v);
        let off = ((1.0 - v) * SETTLE_DROP).round() as i32;
        card.set_margin_top(margin + off);
        card.set_margin_bottom(margin - off);
        if !t.settled(now) {
            return gtk::glib::ControlFlow::Continue;
        }
        if let Some(f) = done.take() {
            f();
        }
        gtk::glib::ControlFlow::Break
    });
}
