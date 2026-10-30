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


pub fn ask(parent: Option<&gtk::Window>, title: &str, detail: &str, cancel: &str, action: &str, kind: &str, ok: impl FnOnce() + 'static) {
    let window = gtk::Window::builder().modal(true).decorated(false).resizable(false).build();
    window.set_transient_for(parent);
    window.set_title(Some(title));
    window.add_css_class("digit-dialog");
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
    window.set_child(Some(&content));
    let w = window.clone();
    no.connect_clicked(move |_| w.close());
    let w = window.clone();
    let ok = std::cell::RefCell::new(Some(ok));
    go.connect_clicked(move |_| {
        w.close();
        if let Some(f) = ok.borrow_mut().take() {
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
    let face = gtk::SignalListItemFactory::new();
    face.connect_setup(|_, obj| {
        let Some(item) = obj.downcast_ref::<gtk::ListItem>() else { return };
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let l = gtk::Label::new(None);
        l.set_xalign(0.0);
        l.set_ellipsize(pango::EllipsizeMode::End);
        l.set_max_width_chars(28);
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
