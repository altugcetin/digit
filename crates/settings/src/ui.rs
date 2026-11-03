use crate::pk;
use gtk::{glib, pango, prelude::*};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
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

/// Grouped choices keep their last saved selection when a write fails.
pub fn select_buttons(buttons: &[gtk::ToggleButton], selected: Option<usize>, changed: impl Fn(usize) -> bool + 'static) {
    for (i, b) in buttons.iter().enumerate() {
        if i > 0 { b.set_group(buttons.first()); }
        b.set_active(Some(i) == selected);
    }
    let accepted = Rc::new(Cell::new(selected));
    let restoring = Rc::new(Cell::new(false));
    let buttons: Rc<Vec<_>> = Rc::new(buttons.iter().map(|b| b.downgrade()).collect());
    let changed = Rc::new(changed);
    for (i, weak) in buttons.iter().enumerate() {
        let Some(b) = weak.upgrade() else { continue };
        let (accepted, restoring, buttons, changed) = (accepted.clone(), restoring.clone(), buttons.clone(), changed.clone());
        b.connect_toggled(move |b| {
            if restoring.get() || !b.is_active() { return; }
            if changed(i) {
                accepted.set(Some(i));
            } else {
                restoring.set(true);
                if let Some(previous) = accepted.get().and_then(|i| buttons.get(i)).and_then(|b| b.upgrade()) {
                    previous.set_active(true);
                } else {
                    b.set_active(false);
                }
                restoring.set(false);
            }
        });
    }
}

pub fn segmented(labels: &[&str], selected: usize, changed: impl Fn(usize) -> bool + 'static) -> gtk::Box {
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    bar.add_css_class("seg");
    bar.set_valign(gtk::Align::Center);
    let buttons: Vec<_> = labels.iter().map(|label| {
        let b = gtk::ToggleButton::with_label(label);
        b.set_focus_on_click(false);
        bar.append(&b);
        b
    }).collect();
    select_buttons(&buttons, Some(selected), changed);
    bar
}

pub fn bind_switch(sw: &gtk::Switch, initial: bool, changed: impl Fn(bool) -> bool + 'static) {
    sw.set_active(initial);
    let accepted = Cell::new(initial);
    let restoring = Cell::new(false);
    sw.connect_active_notify(move |s| {
        if restoring.get() { return; }
        if changed(s.is_active()) {
            accepted.set(s.is_active());
        } else {
            restoring.set(true);
            s.set_active(accepted.get());
            restoring.set(false);
        }
    });
}

/// Text fields share validation, delayed saving and a flush on Enter or focus loss.
pub fn bind_entry(entry: &gtk::Entry, check: impl Fn(&str) -> Result<(), String> + 'static, save: impl Fn(&str) -> bool + 'static) {
    let accepted = Rc::new(RefCell::new(entry.text().to_string()));
    let generation = Rc::new(Cell::new(0u64));
    let restoring = Rc::new(Cell::new(false));
    let check = Rc::new(check);
    let commit: Rc<dyn Fn()> = {
        let (entry, accepted, generation, restoring, check) = (entry.downgrade(), accepted.clone(), generation.clone(), restoring.clone(), check.clone());
        Rc::new(move || {
            generation.set(generation.get() + 1);
            let Some(entry) = entry.upgrade() else { return };
            let text = entry.text().trim().to_string();
            if let Err(e) = check(&text) {
                entry.add_css_class("error");
                entry.set_tooltip_text(Some(&e));
                return;
            }
            entry.remove_css_class("error");
            entry.set_tooltip_text(None);
            if text == *accepted.borrow() { return; }
            if save(&text) { *accepted.borrow_mut() = text; }
            restoring.set(true);
            entry.set_text(&accepted.borrow());
            restoring.set(false);
        })
    };
    let (g, quiet, action) = (generation.clone(), restoring.clone(), commit.clone());
    entry.connect_changed(move |entry| {
        if quiet.get() { return; }
        let n = g.get() + 1;
        g.set(n);
        match check(entry.text().trim()) {
            Ok(()) => {
                entry.remove_css_class("error");
                entry.set_tooltip_text(None);
                let (g, action) = (g.clone(), action.clone());
                glib::timeout_add_local_once(Duration::from_millis(700), move || {
                    if g.get() == n { action(); }
                });
            }
            Err(e) => {
                entry.add_css_class("error");
                entry.set_tooltip_text(Some(&e));
            }
        }
    });
    let action = commit.clone();
    entry.connect_activate(move |_| action());
    let focus = gtk::EventControllerFocus::new();
    focus.connect_leave(move |_| commit());
    entry.add_controller(focus);
}

/// A dialog can live inside Settings or in a transient window when no host is available.
#[derive(Clone)]
pub struct Dialog(Rc<dyn Fn()>);

impl Dialog {
    pub fn close(&self) { (self.0)(); }
}

fn dialog_content(title: &str, detail: &str) -> gtk::Box {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
    content.add_css_class("dialog");
    content.set_size_request(420, -1);
    let title = text(title, "dialog-title");
    title.set_wrap(true);
    title.set_max_width_chars(44);
    content.append(&title);
    if !detail.is_empty() {
        let label = text(detail, "dim");
        label.set_wrap(true);
        label.set_max_width_chars(52);
        content.append(&label);
    }
    content
}

fn dialog_buttons(content: &gtk::Box, cancel: &str, action: &str, kind: &str) -> (gtk::Button, gtk::Button) {
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    buttons.set_halign(gtk::Align::End);
    buttons.set_margin_top(10);
    let no = button(if action.is_empty() { "Close" } else { cancel }, "");
    let go = button(action, kind);
    buttons.append(&no);
    if !action.is_empty() { buttons.append(&go); }
    content.append(&buttons);
    (no, go)
}

fn present_dialog(parent: Option<gtk::Window>, content: &gtk::Box, focus: &gtk::Widget, cancelled: impl FnOnce() + 'static) -> Dialog {
    let parent = parent.or_else(|| crate::main_window().map(|w| w.upcast()));
    let cancelled: Rc<RefCell<Option<Box<dyn FnOnce()>>>> = Rc::new(RefCell::new(Some(Box::new(cancelled))));
    let cancel: Rc<dyn Fn()> = {
        let cancelled = cancelled.clone();
        Rc::new(move || { let f = cancelled.borrow_mut().take(); if let Some(f) = f { f(); } })
    };
    let layer = parent.as_ref().and_then(|w| w.child()).and_downcast::<gtk::Overlay>().filter(|o| o.widget_name() == SHEET_LAYER);
    if let (Some(parent), Some(layer)) = (parent.as_ref(), layer) {
        let previous = gtk::prelude::GtkWindowExt::focus(parent).map(|w| w.downgrade());
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("sheet-backdrop");
        root.add_css_class("settings-dialog");
        root.set_hexpand(true);
        root.set_vexpand(true);
        content.set_halign(gtk::Align::Center);
        content.set_valign(gtk::Align::Center);
        content.set_vexpand(true);
        root.append(content);
        root.set_opacity(0.0);
        layer.add_overlay(&root);
        crate::search_enabled(parent, false);
        settle(&root, content, 0, true, || {});
        let closing = Rc::new(Cell::new(false));
        let close: Rc<dyn Fn()> = {
            let (root, content, layer, closing, cancel, parent) = (root.downgrade(), content.downgrade(), layer.downgrade(), closing.clone(), cancel.clone(), parent.downgrade());
            Rc::new(move || {
                if closing.replace(true) { return; }
                cancel();
                if let (Some(root), Some(content), Some(layer)) = (root.upgrade(), content.upgrade(), layer.upgrade()) {
                    let gone = root.clone();
                    let previous = previous.clone();
                    let parent = parent.clone();
                    settle(&root, &content, 0, false, move || {
                        layer.remove_overlay(&gone);
                        if let Some(parent) = parent.upgrade() {
                            let mut child = layer.first_child();
                            let mut modal = false;
                            while let Some(c) = child {
                                child = c.next_sibling();
                                modal |= c.has_css_class("settings-dialog");
                            }
                            crate::search_enabled(&parent, !modal);
                        }
                        if let Some(focus) = previous.and_then(|w| w.upgrade()).filter(|w| w.is_mapped()) { focus.grab_focus(); }
                    });
                }
            })
        };
        root.connect_unrealize(move |_| {
            if !closing.replace(true) { cancel(); }
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let (weak, window, done) = (root.downgrade(), parent.downgrade(), close.clone());
        keys.connect_key_pressed(move |_, key, _, mods| {
            if key == gtk::gdk::Key::Escape { done(); return glib::Propagation::Stop; }
            if key == gtk::gdk::Key::Tab || key == gtk::gdk::Key::ISO_Left_Tab {
                let direction = if key == gtk::gdk::Key::ISO_Left_Tab || mods.contains(gtk::gdk::ModifierType::SHIFT_MASK) { gtk::DirectionType::TabBackward } else { gtk::DirectionType::TabForward };
                if let (Some(root), Some(window)) = (weak.upgrade(), window.upgrade()) {
                    if !root.child_focus(direction) {
                        gtk::prelude::GtkWindowExt::set_focus(&window, None::<&gtk::Widget>);
                        root.child_focus(direction);
                    }
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        root.add_controller(keys);
        let click = gtk::GestureClick::new();
        let (card, backdrop, done) = (content.downgrade(), root.downgrade(), close.clone());
        click.connect_released(move |_, _, x, y| {
            if let (Some(card), Some(backdrop)) = (card.upgrade(), backdrop.upgrade()) {
                if card.compute_bounds(&backdrop).is_some_and(|b| !b.contains_point(&gtk::graphene::Point::new(x as f32, y as f32))) { done(); }
            }
        });
        root.add_controller(click);
        focus.grab_focus();
        return Dialog(close);
    }
    let window = gtk::Window::builder().modal(true).decorated(false).resizable(false).build();
    window.set_transient_for(parent.as_ref());
    window.add_css_class("digit-dialog");
    window.set_child(Some(content));
    float_over(&window, parent.as_ref());
    window.connect_close_request(move |_| { cancel(); glib::Propagation::Proceed });
    let weak = window.downgrade();
    let close: Rc<dyn Fn()> = Rc::new(move || { if let Some(w) = weak.upgrade() { w.close(); } });
    let keys = gtk::EventControllerKey::new();
    let done = close.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape { done(); glib::Propagation::Stop } else { glib::Propagation::Proceed }
    });
    window.add_controller(keys);
    window.present();
    focus.grab_focus();
    Dialog(close)
}

#[allow(clippy::too_many_arguments)]
fn decision(parent: Option<gtk::Window>, title: &str, detail: &str, items: &[String], cancel: &str, action: &str, kind: &str, ok: impl FnOnce() + 'static) {
    let content = dialog_content(title, detail);
    if !items.is_empty() {
        let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
        for item in items {
            let label = text(item, "sub mono");
            label.set_ellipsize(pango::EllipsizeMode::Middle);
            list.append(&label);
        }
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
        scroll.set_max_content_height(240);
        scroll.set_propagate_natural_height(true);
        scroll.set_child(Some(&list));
        content.append(&scroll);
    }
    let (no, go) = dialog_buttons(&content, cancel, action, kind);
    let accepted = Rc::new(RefCell::new(Some(ok)));
    let abandoned = accepted.clone();
    let dialog = present_dialog(parent, &content, no.upcast_ref(), move || { abandoned.borrow_mut().take(); });
    let done = dialog.clone();
    no.connect_clicked(move |_| done.close());
    go.connect_clicked(move |_| {
        let ok = accepted.borrow_mut().take();
        dialog.close();
        if let Some(ok) = ok { ok(); }
    });
}

#[allow(clippy::too_many_arguments)]
pub fn confirm(parent: Option<gtk::Window>, title: &str, detail: &str, items: &[String], action: &str, kind: &str, ok: impl FnOnce() + 'static) {
    decision(parent, title, detail, items, "Cancel", action, kind, ok);
}

#[allow(clippy::too_many_arguments)]
pub fn ask(parent: Option<&gtk::Window>, title: &str, detail: &str, cancel: &str, action: &str, kind: &str, ok: impl FnOnce() + 'static) {
    decision(parent.cloned(), title, detail, &[], cancel, action, kind, ok);
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
    aside();
    gtk::gio::Subprocess::newv(&os, gtk::gio::SubprocessFlags::NONE).map(|_| ()).map_err(|e| e.to_string())
}

/// Moves the Settings overlay out of the way of a window it is about to open.
pub fn aside() {
    if let Some(w) = crate::main_window() {
        step_aside(w.upcast_ref());
    }
}

/// Use the system chooser. Unmap a layer-shell host while it is open: it cannot
/// be exported as an ordinary portal parent, and must not cover the chooser.
pub fn choose_folder(parent: Option<gtk::Window>, initial: &std::path::Path, done: impl FnOnce(std::path::PathBuf) + 'static) {
    use gtk4_layer_shell::LayerShell;
    let chooser = gtk::FileDialog::new();
    chooser.set_title("Save screenshots to");
    chooser.set_accept_label(Some("Choose"));
    if initial.is_dir() { chooser.set_initial_folder(Some(&gtk::gio::File::for_path(initial))); }
    let overlay = parent.as_ref().is_some_and(|w| w.is_layer_window());
    let restore = parent.as_ref().filter(|w| overlay && w.is_visible()).cloned();
    if let Some(window) = &restore { window.set_visible(false); }
    let native_parent = if overlay { None } else { parent.as_ref() };
    chooser.select_folder(native_parent, gtk::gio::Cancellable::NONE, move |result| {
        if let Some(window) = restore { window.present(); }
        match result {
            Ok(file) => match file.path() {
                Some(path) => done(path),
                None => fail("Choose a folder on this computer"),
            },
            Err(e) if e.matches(gtk::DialogError::Cancelled) || e.matches(gtk::DialogError::Dismissed) => {},
            Err(e) => fail(&format!("Could not open the folder chooser: {e}")),
        }
    });
}

pub fn open_uri(uri: &str) {
    aside();
    let _ = gtk::gio::AppInfo::launch_default_for_uri(uri, gtk::gio::AppLaunchContext::NONE);
}

/// A prompt reports one answer, including cancellation by Escape, the host closing or BlueZ.
pub fn prompt(parent: Option<gtk::Window>, title: &str, detail: &str, field: Option<(&str, bool)>, action: &str, done: impl FnOnce(Option<String>) + 'static) -> Dialog {
    let content = dialog_content(title, detail);
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
    let (no, go) = dialog_buttons(&content, "Cancel", action, "accent");
    let done: Rc<RefCell<Option<Box<dyn FnOnce(Option<String>)>>>> = Rc::new(RefCell::new(Some(Box::new(done))));
    let answer: Rc<dyn Fn(Option<String>)> = Rc::new(move |value| {
        let done = done.borrow_mut().take();
        if let Some(done) = done { done(value); }
    });
    let cancelled = answer.clone();
    let focus = entry.as_ref().map(|e| e.upcast_ref()).unwrap_or(no.upcast_ref());
    let dialog = present_dialog(parent, &content, focus, move || cancelled(None));
    let close = dialog.clone();
    no.connect_clicked(move |_| close.close());
    let (close, field) = (dialog.clone(), entry.as_ref().map(|e| e.downgrade()));
    let accept = Rc::new(move || {
        let value = field.as_ref().and_then(|e| e.upgrade()).map(|e| e.text().to_string()).unwrap_or_default();
        answer(Some(value));
        close.close();
    });
    let ok = accept.clone();
    go.connect_clicked(move |_| ok());
    if let Some(entry) = entry { entry.connect_activate(move |_| accept()); }
    dialog
}
