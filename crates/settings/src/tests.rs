use crate::{desktop, prefs, ui};
use gtk::{glib, prelude::*};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

fn widgets(root: &impl IsA<gtk::Widget>) -> Vec<gtk::Widget> {
    let mut result = vec![root.as_ref().clone()];
    let mut child = root.first_child();
    while let Some(c) = child {
        child = c.next_sibling();
        result.extend(widgets(&c));
    }
    result
}

fn host() -> (gtk::Window, gtk::Overlay) {
    let window = gtk::Window::new();
    let layer = gtk::Overlay::new();
    layer.set_widget_name(ui::SHEET_LAYER);
    layer.set_child(Some(&gtk::Box::new(gtk::Orientation::Vertical, 0)));
    window.set_child(Some(&layer));
    (window, layer)
}

/// These widgets are never presented: the tests do not open windows on the desktop.
#[test]
fn settings_controls_and_dialogs() {
    // Exercise real settings writes in a child with its own config and a failing theme command.
    if std::env::var_os("DIGIT_SETTINGS_UI_TEST").is_none() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("digit-settings-test-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("digit"), "#!/bin/sh\nexit 1\n").unwrap();
        std::fs::set_permissions(bin.join("digit"), std::fs::Permissions::from_mode(0o755)).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "tests::settings_controls_and_dialogs", "--nocapture"])
            .env("DIGIT_SETTINGS_UI_TEST", "1")
            .env("XDG_CONFIG_HOME", dir.join("config"))
            .env("XDG_STATE_HOME", dir.join("state"))
            .env("XDG_CACHE_HOME", dir.join("cache"))
            .env("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()))
            .status().unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(status.success());
        return;
    }
    if let Err(e) = gtk::init() {
        eprintln!("Skipping GTK checks without a display: {e}");
        return;
    }
    // Closing with reduced motion must complete even without a mapped frame clock.
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let closed = Rc::new(Cell::new(false));
    kit::motion::apply(true);
    let done = closed.clone();
    kit::ui::settle(&root, &card, 0, false, move || done.set(true));
    assert_eq!(root.opacity(), 0.0);
    let context = glib::MainContext::default();
    while context.pending() {
        context.iteration(false);
    }
    assert!(closed.get(), "closing must not wait for an animation frame");
    kit::motion::apply(false);

    let calls = Rc::new(Cell::new(0));
    let count = calls.clone();
    let bar = ui::segmented(&["Old", "New"], 0, move |_| { count.set(count.get() + 1); false });
    let buttons: Vec<_> = widgets(&bar).into_iter().filter_map(|w| w.downcast::<gtk::ToggleButton>().ok()).collect();
    buttons[1].set_active(true);
    assert!(buttons[0].is_active());
    assert!(!buttons[1].is_active());
    assert_eq!(calls.get(), 1, "restoring a selection must not try to save again");

    let sw = gtk::Switch::new();
    ui::bind_switch(&sw, true, |_| false);
    sw.set_active(false);
    assert!(sw.is_active());

    let entry = gtk::Entry::new();
    entry.set_text("/old");
    let saved = Rc::new(RefCell::new(Vec::new()));
    let writes = saved.clone();
    ui::bind_entry(&entry, |s| if s.starts_with('/') { Ok(()) } else { Err("Use an absolute path".into()) }, move |s| {
        writes.borrow_mut().push(s.to_string());
        s != "/rejected"
    });
    entry.set_text("/rejected");
    entry.emit_activate();
    assert_eq!(entry.text(), "/old");
    entry.set_text("relative");
    entry.emit_activate();
    assert!(entry.has_css_class("error"));
    assert_eq!(saved.borrow().len(), 1);
    let folder = format!("/{}", "long-folder/".repeat(20));
    entry.set_text(&folder);
    entry.emit_activate();
    assert_eq!(entry.text(), folder);
    assert!(!entry.has_css_class("error"));
    assert_eq!(saved.borrow().last(), Some(&folder));

    for (mode, warmth, hours) in [("off", false, false), ("sunset", true, false), ("schedule", true, true), ("always", true, false)] {
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let mut table = toml::Table::new();
        table.insert("night_light".into(), toml::Value::String(mode.into()));
        desktop::night_light(&body, &table);
        for (name, expected) in [("Warmth", warmth), ("From", hours), ("Until", hours)] {
            let label = widgets(&body).into_iter().filter_map(|w| w.downcast::<gtk::Label>().ok()).find(|w| w.text() == name).unwrap();
            let row = label.parent().unwrap().parent().unwrap();
            assert_eq!(row.is_sensitive(), expected, "{name} in {mode}");
        }
    }

    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    desktop::night_light(&body, &toml::Table::new());
    let mode = widgets(&body).into_iter().find_map(|w| w.downcast::<gtk::DropDown>().ok()).unwrap();
    let inputs: Vec<_> = widgets(&body).into_iter().filter_map(|w| w.downcast::<gtk::Entry>().ok()).filter(|w| w.placeholder_text().is_some()).collect();
    assert_eq!(inputs.len(), 2);
    mode.set_selected(2); // Custom hours applies without rebuilding the page.
    assert!(inputs.iter().all(|w| w.is_sensitive()));
    mode.set_selected(3); // Always on keeps warmth, disables the custom times.
    assert!(inputs.iter().all(|w| !w.is_sensitive()));

    let accepted_config = std::fs::read(prefs::path()).unwrap();
    let damaged = "[broken";
    std::fs::write(prefs::path(), damaged).unwrap();
    mode.set_selected(2);
    assert_eq!(mode.selected(), 3);
    assert!(inputs.iter().all(|w| !w.is_sensitive()));
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let row = desktop::toggle(&body, "test", "Test switch", "", "test_switch", true);
    let sw = widgets(&row).into_iter().find_map(|w| w.downcast::<gtk::Switch>().ok()).unwrap();
    sw.set_active(false);
    assert!(sw.is_active());
    desktop::choice(&body, "test", "Test choice", "", "test_choice", &[("a", "A"), ("b", "B")], "a");
    let buttons: Vec<_> = widgets(&body).into_iter().filter_map(|w| w.downcast::<gtk::ToggleButton>().ok()).collect();
    buttons[1].set_active(true);
    assert!(buttons[0].is_active());
    let tiles = desktop::corners(&body, "test", "Test corners", "", "test_corners", 1.0);
    let buttons: Vec<_> = widgets(&tiles).into_iter().filter_map(|w| w.downcast::<gtk::ToggleButton>().ok()).collect();
    buttons[0].set_active(true);
    assert!(buttons[2].is_active());
    let row = desktop::slider(&body, "test", "Test slider", "", "test_slider", (0.0, 10.0, 1.0), 5.0, 5.0, 0);
    let scale = widgets(&row).into_iter().find_map(|w| w.downcast::<gtk::Scale>().ok()).unwrap();
    let reset = widgets(&row).into_iter().filter_map(|w| w.downcast::<gtk::Button>().ok()).find(|b| b.tooltip_text().as_deref() == Some("Reset to default")).unwrap();
    let label = widgets(&row).into_iter().filter_map(|w| w.downcast::<gtk::Label>().ok()).find(|l| l.tooltip_text().as_deref() == Some("Double-click to type a value")).unwrap();
    assert!(!reset.is_sensitive());
    assert_eq!(reset.opacity(), 0.0);
    scale.set_value(8.0);
    assert!(reset.is_sensitive());
    assert_eq!(label.text(), "8");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let context = glib::MainContext::default();
    while scale.value() != 5.0 && std::time::Instant::now() < deadline {
        context.iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(scale.value(), 5.0);
    assert_eq!(label.text(), "5");
    assert!(!reset.is_sensitive(), "failed saves must restore the reset state too");
    assert_eq!(std::fs::read_to_string(prefs::path()).unwrap(), damaged);
    std::fs::write(prefs::path(), accepted_config).unwrap();

    scale.set_value(8.0);
    reset.emit_clicked();
    assert_eq!(scale.value(), 5.0, "reset uses the slider's default");
    assert!(!reset.is_sensitive());
    assert_eq!(reset.opacity(), 0.0);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while prefs::get_f64(&prefs::read(), "test_slider", -1.0) != 5.0 && std::time::Instant::now() < deadline {
        context.iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(prefs::get_f64(&prefs::read(), "test_slider", -1.0), 5.0, "reset must save through the debounce");

    let slot = label.parent().unwrap().downcast::<gtk::Stack>().unwrap();
    let entry = widgets(&row).into_iter().find_map(|w| w.downcast::<gtk::Entry>().ok()).unwrap();
    let click = label.observe_controllers().iter::<glib::Object>().filter_map(Result::ok).find_map(|c| c.downcast::<gtk::GestureClick>().ok()).unwrap();
    click.emit_by_name::<()>("pressed", &[&1i32, &0.0f64, &0.0f64]);
    assert_eq!(slot.visible_child_name().as_deref(), Some("value"));
    click.emit_by_name::<()>("pressed", &[&2i32, &0.0f64, &0.0f64]);
    assert_eq!(slot.visible_child_name().as_deref(), Some("entry"));
    assert_eq!(entry.text(), "5");
    assert_eq!(entry.selection_bounds(), Some((0, 1)));
    entry.set_text("7,4");
    entry.emit_activate();
    assert_eq!(scale.value(), 7.0, "typed values accept decimal commas and round to the step");
    assert_eq!(label.text(), "7");
    assert_eq!(slot.visible_child_name().as_deref(), Some("value"));
    assert!(reset.is_sensitive());

    let focus = entry.observe_controllers().iter::<glib::Object>().filter_map(Result::ok).find_map(|c| c.downcast::<gtk::EventControllerFocus>().ok()).unwrap();
    for (text, expected) in [("99", 10.0), ("-99", 0.0), ("invalid", 0.0), ("NaN", 0.0), ("inf", 0.0)] {
        click.emit_by_name::<()>("pressed", &[&2i32, &0.0f64, &0.0f64]);
        entry.set_text(text);
        focus.emit_by_name::<()>("leave", &[]);
        assert_eq!(scale.value(), expected);
        assert_eq!(slot.visible_child_name().as_deref(), Some("value"));
    }
    click.emit_by_name::<()>("pressed", &[&2i32, &0.0f64, &0.0f64]);
    entry.set_text("9");
    let keys = entry.observe_controllers().iter::<glib::Object>().filter_map(Result::ok).find_map(|c| c.downcast::<gtk::EventControllerKey>().ok()).unwrap();
    assert!(keys.emit_by_name::<bool>("key-pressed", &[&gtk::gdk::Key::Escape, &0u32, &gtk::gdk::ModifierType::empty()]));
    focus.emit_by_name::<()>("leave", &[]);
    assert_eq!(scale.value(), 0.0, "Escape cancels even when focus leaves afterward");
    assert_eq!(slot.visible_child_name().as_deref(), Some("value"));

    let screenshot_page = desktop::build("screenshots", "Screenshots", "");
    let folder = widgets(&screenshot_page).into_iter().find_map(|w| w.downcast::<gtk::Entry>().ok()).unwrap();
    let new_path = format!("/{}", "screenshots/".repeat(12));
    folder.set_text(&new_path);
    folder.emit_activate();
    assert_eq!(prefs::get_str(&prefs::read(), "snap_dir", ""), new_path);
    assert!(widgets(&screenshot_page).into_iter().filter_map(|w| w.downcast::<gtk::Button>().ok()).any(|b| b.label().as_deref() == Some("Choose…")));

    let appearance = desktop::build("appearance", "Appearance", "");
    let color_row = widgets(&appearance).into_iter().filter_map(|w| w.downcast::<gtk::Label>().ok()).find(|l| l.text() == "Color scheme").unwrap().parent().unwrap().parent().unwrap();
    let color = widgets(&color_row).into_iter().find_map(|w| w.downcast::<gtk::DropDown>().ok()).unwrap();
    let previous = color.selected();
    color.set_selected((previous + 1) % kit::theme::SCHEMES.len() as u32);
    assert!(!color.is_sensitive());
    let limit = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let context = glib::MainContext::default();
    while !color.is_sensitive() && std::time::Instant::now() < limit {
        context.iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(color.is_sensitive());
    assert_eq!(color.selected(), previous, "an unsuccessful theme process must restore the choice");

    let (window, layer) = host();
    let top_levels = gtk::Window::toplevels().n_items();
    let answers = Rc::new(RefCell::new(Vec::new()));
    let result = answers.clone();
    let dialog = ui::prompt(Some(window.clone()), "Password", "", Some(("Password", true)), "Join", move |v| result.borrow_mut().push(v));
    assert_eq!(gtk::Window::toplevels().n_items(), top_levels);
    assert!(!window.is_visible());
    let root = layer.last_child().unwrap();
    let input = widgets(&root).into_iter().find_map(|w| w.downcast::<gtk::Entry>().ok()).unwrap();
    input.set_text("answer");
    input.emit_activate();
    dialog.close();
    assert_eq!(*answers.borrow(), vec![Some("answer".into())]);
    window.destroy();

    let (window, layer) = host();
    let answers = Rc::new(RefCell::new(Vec::new()));
    let result = answers.clone();
    let dialog = ui::prompt(Some(window.clone()), "Pair", "", None, "Pair", move |v| result.borrow_mut().push(v));
    let root = layer.last_child().unwrap();
    let keys = root.observe_controllers().iter::<glib::Object>().filter_map(Result::ok).find_map(|c| c.downcast::<gtk::EventControllerKey>().ok()).unwrap();
    let stopped = keys.emit_by_name::<bool>("key-pressed", &[&gtk::gdk::Key::Escape, &0u32, &gtk::gdk::ModifierType::empty()]);
    assert!(stopped);
    dialog.close();
    assert_eq!(*answers.borrow(), vec![None]);
    window.destroy();

    let (window, layer) = host();
    let accepted = Rc::new(Cell::new(0));
    let count = accepted.clone();
    let top_levels = gtk::Window::toplevels().n_items();
    ui::confirm(Some(window.clone()), "Remove?", "Review the affected packages", &["example 1.0".into()], "Remove", "danger", move || count.set(count.get() + 1));
    assert_eq!(gtk::Window::toplevels().n_items(), top_levels);
    let root = layer.last_child().unwrap();
    let go = widgets(&root).into_iter().filter_map(|w| w.downcast::<gtk::Button>().ok()).find(|b| b.label().as_deref() == Some("Remove")).unwrap();
    go.emit_clicked();
    go.emit_clicked();
    assert_eq!(accepted.get(), 1);
    window.destroy();
    // Drain queued GTK cleanup without waiting for delayed writes.
    let context = glib::MainContext::default();
    while context.pending() { context.iteration(false); }
}
