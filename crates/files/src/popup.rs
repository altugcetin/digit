use gtk::{gdk, gio, glib, prelude::*};
use std::rc::Rc;

pub enum Item {
    Act { glyph: &'static str, label: String, action: String, target: Option<glib::Variant>, danger: bool },
    Run { glyph: &'static str, label: String, run: Rc<dyn Fn()>, danger: bool },
    Sub { glyph: &'static str, label: String, items: Vec<Item> },
    Sep,
}

pub fn act(glyph: &'static str, label: &str, action: &str) -> Item {
    Item::Act { glyph, label: label.to_string(), action: action.to_string(), target: None, danger: false }
}

pub fn act_with(glyph: &'static str, label: &str, action: &str, target: glib::Variant) -> Item {
    Item::Act { glyph, label: label.to_string(), action: action.to_string(), target: Some(target), danger: false }
}

pub fn danger(glyph: &'static str, label: &str, action: &str) -> Item {
    Item::Act { glyph, label: label.to_string(), action: action.to_string(), target: None, danger: true }
}

pub fn run(glyph: &'static str, label: &str, f: impl Fn() + 'static) -> Item {
    Item::Run { glyph, label: label.to_string(), run: Rc::new(f), danger: false }
}

pub fn sub(glyph: &'static str, label: &str, items: Vec<Item>) -> Item {
    Item::Sub { glyph, label: label.to_string(), items }
}

fn shortcut(app: Option<gtk::Application>, action: &str, target: Option<&glib::Variant>) -> String {
    let Some(app) = app else { return String::new() };
    let detailed = match target {
        Some(t) => gio::Action::print_detailed_name(action, Some(t)).to_string(),
        None => action.to_string(),
    };
    app.accels_for_action(&detailed)
        .first()
        .and_then(|a| gtk::accelerator_parse(a.as_str()))
        .map(|(k, m)| gtk::accelerator_get_label(k, m).to_string())
        .unwrap_or_default()
}

fn lookup(window: Option<&gtk::ApplicationWindow>, action: &str) -> Option<gio::Action> {
    let name = action.strip_prefix("win.")?;
    window?.lookup_action(name)
}

fn row(glyph: &str, label: &str, accel: &str, trailing: Option<&str>, danger: bool) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("pm-row");
    if danger {
        b.add_css_class("pm-danger");
    }
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let g = kit::ui::sym(glyph);
    g.add_css_class("pm-glyph");
    line.append(&g);
    let l = gtk::Label::new(Some(label));
    l.set_xalign(0.0);
    l.set_hexpand(true);
    l.add_css_class("pm-label");
    line.append(&l);
    if !accel.is_empty() {
        let a = gtk::Label::new(Some(accel));
        a.add_css_class("pm-accel");
        line.append(&a);
    }
    if let Some(t) = trailing {
        let t = kit::ui::sym(t);
        t.add_css_class("pm-trail");
        line.append(&t);
    }
    b.set_child(Some(&line));
    b
}

fn page(pop: &gtk::Popover, stack: &gtk::Stack, items: Vec<Item>, back: Option<&str>, window: Option<gtk::ApplicationWindow>, host: &gtk::Widget, depth: &mut usize) -> gtk::Box {
    let col = gtk::Box::new(gtk::Orientation::Vertical, 0);
    if let Some(title) = back {
        let b = row("chevron_left", title, "", None, false);
        b.add_css_class("pm-back");
        let st = stack.clone();
        b.connect_clicked(move |_| st.set_visible_child_name("main"));
        col.append(&b);
        col.append(&sep());
    }
    let app = window.as_ref().and_then(|w| w.application());
    for item in items {
        match item {
            Item::Sep => {
                if col.last_child().is_some_and(|c| !c.has_css_class("pm-sep")) {
                    col.append(&sep());
                }
            }
            Item::Act { glyph, label, action, target, danger } => {
                let found = lookup(window.as_ref(), &action);
                let state = found.as_ref().and_then(|a| a.state());
                let on = match (&state, &target) {
                    (Some(s), None) => s.get::<bool>(),
                    (Some(s), Some(t)) => Some(s == t),
                    _ => None,
                };
                let accel = shortcut(app.clone(), &action, target.as_ref());
                let b = row(glyph, &label, &accel, on.and_then(|v| v.then_some("check")), danger);
                b.set_sensitive(found.as_ref().is_none_or(|a| a.is_enabled()));
                let (p, h) = (pop.clone(), host.clone());
                b.connect_clicked(move |_| {
                    p.popdown();
                    let (h, action, target) = (h.clone(), action.clone(), target.clone());
                    glib::idle_add_local_once(move || {
                        let _ = h.activate_action(&action, target.as_ref());
                    });
                });
                col.append(&b);
            }
            Item::Run { glyph, label, run, danger } => {
                let b = row(glyph, &label, "", None, danger);
                let p = pop.clone();
                b.connect_clicked(move |_| {
                    p.popdown();
                    let run = run.clone();
                    glib::idle_add_local_once(move || run());
                });
                col.append(&b);
            }
            Item::Sub { glyph, label, items } => {
                *depth += 1;
                let name = format!("sub{depth}");
                let inner = page(pop, stack, items, Some(&label), window.clone(), host, depth);
                stack.add_named(&inner, Some(&name));
                let b = row(glyph, &label, "", Some("chevron_right"), false);
                let st = stack.clone();
                b.connect_clicked(move |_| st.set_visible_child_name(&name));
                col.append(&b);
            }
        }
    }
    while col.last_child().is_some_and(|c| c.has_css_class("pm-sep")) {
        if let Some(c) = col.last_child() {
            col.remove(&c);
        }
    }
    col
}

fn sep() -> gtk::Separator {
    let s = gtk::Separator::new(gtk::Orientation::Horizontal);
    s.add_css_class("pm-sep");
    s
}

pub fn show(host: &impl IsA<gtk::Widget>, x: f64, y: f64, items: Vec<Item>) {
    let host = host.as_ref().clone();
    let window = host.root().and_then(|r| r.downcast::<gtk::ApplicationWindow>().ok());
    let pop = gtk::Popover::new();
    pop.add_css_class("pm");
    pop.set_has_arrow(false);
    pop.set_halign(gtk::Align::Start);
    pop.set_parent(&host);
    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
    stack.set_transition_duration(160);
    stack.set_vhomogeneous(false);
    stack.set_interpolate_size(true);
    let mut depth = 0;
    let main = page(&pop, &stack, items, None, window, &host, &mut depth);
    stack.add_named(&main, Some("main"));
    stack.set_visible_child_name("main");
    pop.set_child(Some(&stack));
    pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    pop.connect_closed(|pop| {
        let pop = pop.clone();
        glib::idle_add_local_once(move || pop.unparent());
    });
    pop.popup();
}
