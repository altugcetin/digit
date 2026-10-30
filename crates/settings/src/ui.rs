use crate::pk;
use gtk::{glib, pango, prelude::*};
use std::cell::{Cell, RefCell};
use std::time::Duration;

struct Activity {
    revealer: gtk::Revealer,
    label: gtk::Label,
    bar: gtk::ProgressBar,
    generation: Cell<u64>,
}

thread_local! {
    static ACTIVITY: RefCell<Option<Activity>> = const { RefCell::new(None) };
    static BUSY: Cell<bool> = const { Cell::new(false) };
}

pub fn activity() -> gtk::Revealer {
    let revealer = gtk::Revealer::new();
    revealer.set_transition_type(gtk::RevealerTransitionType::SlideUp);
    revealer.set_transition_duration(220);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    body.add_css_class("activity");
    let label = text("", "name");
    let bar = gtk::ProgressBar::new();
    body.append(&label);
    body.append(&bar);
    revealer.set_child(Some(&body));
    ACTIVITY.with(|a| *a.borrow_mut() = Some(Activity { revealer: revealer.clone(), label, bar, generation: Cell::new(0) }));
    revealer
}

fn show(message: &str, fraction: Option<f64>, class: Option<&str>, hide_after: Option<u64>) {
    ACTIVITY.with(|a| {
        let Some(a) = a.borrow().as_ref().map(|a| (a.revealer.clone(), a.label.clone(), a.bar.clone(), a.generation.get() + 1)) else { return };
        let (revealer, label, bar, generation) = a;
        ACTIVITY.with(|x| {
            if let Some(x) = x.borrow().as_ref() {
                x.generation.set(generation);
            }
        });
        label.set_text(message);
        for c in ["warn", "good"] {
            label.remove_css_class(c);
        }
        if let Some(c) = class {
            label.add_css_class(c);
        }
        match fraction {
            Some(f) => {
                bar.set_visible(true);
                bar.set_fraction(f.clamp(0.0, 1.0));
            }
            None if hide_after.is_none() => {
                bar.set_visible(true);
                bar.pulse();
            }
            None => bar.set_visible(false),
        }
        revealer.set_reveal_child(true);
        if let Some(ms) = hide_after {
            glib::timeout_add_local_once(Duration::from_millis(ms), move || {
                let same = ACTIVITY.with(|x| x.borrow().as_ref().is_some_and(|x| x.generation.get() == generation));
                if same {
                    revealer.set_reveal_child(false);
                }
            });
        }
    });
}

pub fn busy(message: &str, fraction: Option<f64>) {
    show(message, fraction, None, None);
}

pub fn done(message: &str) {
    show(message, None, Some("good"), Some(4500));
}

pub fn fail(message: &str) {
    show(message, None, Some("warn"), Some(9000));
}

pub fn progress(prefix: &str) -> impl Fn(pk::Progress) + use<> {
    let prefix = prefix.to_string();
    move |p: pk::Progress| busy(&format!("{prefix} · {}", p.label()), p.percent.map(|x| x as f64 / 100.0))
}

pub fn begin() -> bool {
    BUSY.with(|b| {
        if b.get() {
            fail("Another operation is still running");
            false
        } else {
            b.set(true);
            true
        }
    })
}

pub fn end() {
    BUSY.with(|b| b.set(false));
}

pub use kit::ui::*;

#[allow(clippy::too_many_arguments)]
pub fn confirm(parent: Option<gtk::Window>, title: &str, body: &str, items: &[String], action: &str, kind: &str, ok: impl FnOnce() + 'static) {
    let window = gtk::Window::builder().modal(true).decorated(false).resizable(false).default_width(480).build();
    window.set_transient_for(parent.as_ref());
    window.add_css_class("digit-dialog");
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.add_css_class("dialog");
    let t = text(title, "name");
    t.add_css_class("title");
    content.append(&t);
    let b = text(body, "dim");
    b.set_wrap(true);
    b.set_max_width_chars(54);
    content.append(&b);
    if !items.is_empty() {
        let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
        for it in items {
            let l = text(it, "sub mono");
            l.set_ellipsize(pango::EllipsizeMode::Middle);
            list.append(&l);
        }
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
        scroll.set_max_content_height(240);
        scroll.set_propagate_natural_height(true);
        scroll.set_child(Some(&list));
        let c = card();
        c.append(&scroll);
        content.append(&c);
    }
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    buttons.set_halign(gtk::Align::End);
    buttons.set_margin_top(6);
    let cancel = button("Cancel", "");
    let go = button(action, kind);
    buttons.append(&cancel);
    if !action.is_empty() {
        buttons.append(&go);
    } else {
        cancel.set_label("Close");
    }
    content.append(&buttons);
    window.set_child(Some(&content));
    let w = window.clone();
    cancel.connect_clicked(move |_| w.close());
    let w = window.clone();
    let ok = RefCell::new(Some(ok));
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
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(keys);
    window.present();
    cancel.grab_focus();
}

pub fn terminal(argv: &[&str]) -> Result<(), String> {
    let term = std::env::var("TERMINAL").ok().filter(|t| !t.is_empty() && t.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/')));
    let term = term.unwrap_or_else(|| "kitty".into());
    let base = std::path::Path::new(&term).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let mut full: Vec<String> = vec![term.clone()];
    match base.as_str() {
        "kitty" => full.push("--hold".into()),
        "foot" => full.push("--hold".into()),
        "alacritty" | "wezterm" | "ghostty" => full.push("-e".into()),
        _ => full.push("-e".into()),
    }
    full.extend(argv.iter().map(|s| s.to_string()));
    let os: Vec<&std::ffi::OsStr> = full.iter().map(std::ffi::OsStr::new).collect();
    gtk::gio::Subprocess::newv(&os, gtk::gio::SubprocessFlags::NONE).map(|_| ()).map_err(|e| e.to_string())
}

pub fn open_uri(uri: &str) {
    let _ = gtk::gio::AppInfo::launch_default_for_uri(uri, gtk::gio::AppLaunchContext::NONE);
}

/// A small dialog that may ask for text. `done` gets `Some(text)` on the action button
/// (empty when there is no field) and `None` when it is cancelled or closed some other way.
pub fn prompt(parent: Option<gtk::Window>, title: &str, detail: &str, field: Option<(&str, bool)>, action: &str, done: impl FnOnce(Option<String>) + 'static) -> gtk::Window {
    let window = gtk::Window::builder().modal(true).decorated(false).resizable(false).build();
    window.set_transient_for(parent.as_ref());
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
    let entry = field.map(|(hint, secret)| {
        let e = gtk::Entry::new();
        e.add_css_class("search");
        e.set_placeholder_text(Some(hint));
        e.set_visibility(!secret);
        if secret {
            e.set_input_purpose(gtk::InputPurpose::Password);
            e.set_secondary_icon_name(Some("view-reveal-symbolic"));
            e.connect_icon_press(|e, _| e.set_visibility(!EntryExt::is_visible(e)));
        }
        e.set_margin_top(4);
        content.append(&e);
        e
    });
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    buttons.set_halign(gtk::Align::End);
    buttons.set_margin_top(10);
    let no = button("Cancel", "");
    let go = button(action, "accent");
    buttons.append(&no);
    if !action.is_empty() {
        buttons.append(&go);
    } else {
        no.set_label("Close");
    }
    content.append(&buttons);
    window.set_child(Some(&content));
    let done: std::rc::Rc<RefCell<Option<Box<dyn FnOnce(Option<String>)>>>> = std::rc::Rc::new(RefCell::new(Some(Box::new(done))));
    let answer = {
        let done = done.clone();
        move |v: Option<String>| {
            if let Some(f) = done.borrow_mut().take() {
                f(v);
            }
        }
    };
    let w = window.clone();
    let a = answer.clone();
    no.connect_clicked(move |_| {
        a(None);
        w.close();
    });
    let w = window.clone();
    let a = answer.clone();
    let e = entry.clone();
    let accept = std::rc::Rc::new(move || {
        a(Some(e.as_ref().map(|e| e.text().to_string()).unwrap_or_default()));
        w.close();
    });
    let ok = accept.clone();
    go.connect_clicked(move |_| ok());
    if let Some(e) = &entry {
        let ok = accept.clone();
        e.connect_activate(move |_| ok());
    }
    let a = answer.clone();
    window.connect_close_request(move |_| {
        a(None);
        glib::Propagation::Proceed
    });
    let keys = gtk::EventControllerKey::new();
    let w = window.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            w.close();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(keys);
    window.present();
    match &entry {
        Some(e) => {
            e.grab_focus();
        }
        None if !action.is_empty() => {
            go.grab_focus();
        }
        None => {
            no.grab_focus();
        }
    }
    window
}
