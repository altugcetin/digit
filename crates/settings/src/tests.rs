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
    // A rebuilt page must wait for allocation rather than lose its saved offset.
    let window = gtk::Window::new();
    let (page, body) = ui::page("Motion", "");
    let switch = gtk::Switch::new();
    body.append(&switch);
    window.set_child(Some(&page));
    gtk::prelude::GtkWindowExt::set_focus(&window, Some(&switch));
    let path = crate::focus_path(page.upcast_ref()).expect("focused switch belongs to the page");
    let (next, body) = ui::page("Motion", "");
    let switch = gtk::Switch::new();
    switch.set_active(true);
    body.append(&switch);
    crate::restore_page(next.upcast_ref(), 420.0, Some(path));
    window.set_child(Some(&next));
    let adj = next.vadjustment();
    adj.configure(0.0, 0.0, 100.0, 1.0, 10.0, 100.0);
    assert_eq!(adj.value(), 0.0, "an unallocated range cannot restore the offset");
    adj.configure(0.0, 0.0, 400.0, 1.0, 10.0, 100.0);
    assert_eq!(adj.value(), 300.0, "a short range goes as far down as it can, and keeps waiting");
    adj.configure(0.0, 0.0, 1000.0, 1.0, 10.0, 100.0);
    assert_eq!(adj.value(), 420.0);
    assert_eq!(gtk::prelude::GtkWindowExt::focus(&window), Some(switch.clone().upcast()));
    assert!(switch.is_active());
    adj.set_value(250.0);
    adj.set_upper(1100.0);
    assert_eq!(adj.value(), 250.0, "later layouts must leave scrolling alone");
    window.destroy();

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

    assert!(prefs::set("snap_frame", toml::Value::String("Ocean".into())).is_ok());
    let screenshot_page = desktop::build("screenshots", "Screenshots", "");
    assert!(!widgets(&screenshot_page).into_iter().filter_map(|w| w.downcast::<gtk::Label>().ok()).any(|l| l.text() == "Background frame"));
    let folder = widgets(&screenshot_page).into_iter().find_map(|w| w.downcast::<gtk::Entry>().ok()).unwrap();
    let new_path = format!("/{}", "screenshots/".repeat(12));
    folder.set_text(&new_path);
    folder.emit_activate();
    assert_eq!(prefs::get_str(&prefs::read(), "snap_dir", ""), new_path);
    assert_eq!(prefs::get_str(&prefs::read(), "snap_frame", ""), "Ocean");
    assert!(widgets(&screenshot_page).into_iter().filter_map(|w| w.downcast::<gtk::Button>().ok()).any(|b| b.label().as_deref() == Some("Choose…")));

    let wallpaper_page = desktop::build("desktop", "Desktop", "");
    let folder_row = widgets(&wallpaper_page).into_iter().filter_map(|w| w.downcast::<gtk::Label>().ok()).find(|l| l.text() == "Wallpaper folder").unwrap().parent().unwrap().parent().unwrap();
    let folder = widgets(&folder_row).into_iter().find_map(|w| w.downcast::<gtk::Entry>().ok()).unwrap();
    assert!(folder.text().is_empty(), "an empty folder keeps the default search locations");
    assert!(widgets(&folder_row).into_iter().filter_map(|w| w.downcast::<gtk::Button>().ok()).any(|b| b.label().as_deref() == Some("Choose…")));
    let wall_dir = glib::user_cache_dir().join("wallpapers");
    std::fs::create_dir_all(&wall_dir).unwrap();
    let wall_dir = wall_dir.display().to_string();
    folder.set_text(&wall_dir);
    folder.emit_activate();
    assert_eq!(prefs::get_str(&prefs::read(), "wall_dir", ""), wall_dir);
    for invalid in ["relative/folder".to_string(), format!("{wall_dir}/missing"), format!("{wall_dir}/bad\nfolder"), prefs::path().display().to_string()] {
        folder.set_text(&invalid);
        folder.emit_activate();
        assert!(folder.has_css_class("error"));
        assert_eq!(prefs::get_str(&prefs::read(), "wall_dir", ""), wall_dir);
    }
    let accepted = std::fs::read(prefs::path()).unwrap();
    std::fs::write(prefs::path(), damaged).unwrap();
    folder.set_text("");
    folder.emit_activate();
    assert_eq!(folder.text(), wall_dir, "a failed save restores the last accepted folder");
    assert_eq!(std::fs::read_to_string(prefs::path()).unwrap(), damaged);
    std::fs::write(prefs::path(), accepted).unwrap();
    folder.set_text("");
    folder.emit_activate();
    assert_eq!(prefs::get_str(&prefs::read(), "wall_dir", "missing"), "");

    let notch_page = desktop::build("notch", "Notch and menu bar", "");
    let logo_row = widgets(&notch_page).into_iter().filter_map(|w| w.downcast::<gtk::Label>().ok()).find(|l| l.text() == "Menu bar logo").unwrap().parent().unwrap().parent().unwrap();
    let logos: Vec<_> = widgets(&logo_row).into_iter().filter_map(|w| w.downcast::<gtk::ToggleButton>().ok()).collect();
    assert_eq!(logos.len(), 2);
    assert!(logos[0].is_active(), "the distribution logo is the default");
    assert_eq!(logos[0].label().as_deref(), Some("Distribution"));
    assert_eq!(logos[1].label().as_deref(), Some("Digit"));
    logos[1].set_active(true);
    assert_eq!(prefs::get_str(&prefs::read(), "menubar_logo", ""), "digit");
    let rebuilt = desktop::build("notch", "Notch and menu bar", "");
    assert!(widgets(&rebuilt).into_iter().filter_map(|w| w.downcast::<gtk::ToggleButton>().ok()).any(|b| b.label().as_deref() == Some("Digit") && b.is_active()));
    let accepted = std::fs::read(prefs::path()).unwrap();
    std::fs::write(prefs::path(), damaged).unwrap();
    logos[0].set_active(true);
    assert!(logos[1].is_active(), "a failed save restores the logo choice");
    assert_eq!(std::fs::read_to_string(prefs::path()).unwrap(), damaged);
    std::fs::write(prefs::path(), accepted).unwrap();
    logos[0].set_active(true);
    assert_eq!(prefs::get_str(&prefs::read(), "menubar_logo", ""), "distro");
    let enabled_row = widgets(&notch_page).into_iter().filter_map(|w| w.downcast::<gtk::Label>().ok()).filter(|l| l.text() == "App menus" && l.has_css_class("name")).filter_map(|l| l.parent()?.parent()).find(|row| widgets(row).into_iter().any(|w| w.is::<gtk::Switch>())).unwrap();
    let enabled = widgets(&enabled_row).into_iter().find_map(|w| w.downcast::<gtk::Switch>().ok()).unwrap();
    // App menus only hides the app's name and menus; the logo stays in the bar.
    enabled.set_active(false);
    assert!(logo_row.is_sensitive());
    enabled.set_active(true);
    assert!(logo_row.is_sensitive());

    // The hot corners are four buttons on a picture of a screen, top left first; each opens a menu.
    let desktop_page = desktop::build("desktop", "Desktop", "");
    let corners: Vec<gtk::Label> = widgets(&desktop_page).into_iter().filter_map(|w| w.downcast::<gtk::MenuButton>().ok()).filter_map(|b| b.child()?.downcast::<gtk::Label>().ok()).filter(|l| ["Nothing", "Workspaces", "Control Center"].contains(&l.text().as_str())).collect();
    assert_eq!(corners.len(), 4);
    let (top_left, top_right) = (corners[0].clone(), corners[1].clone());
    assert_eq!((top_left.text().as_str(), top_right.text().as_str()), ("Workspaces", "Nothing"));
    let menu = top_right.ancestor(gtk::MenuButton::static_type()).and_downcast::<gtk::MenuButton>().unwrap().popover().unwrap();
    let choose = widgets(&menu).into_iter().filter_map(|w| w.downcast::<gtk::Button>().ok()).find(|b| b.label().as_deref() == Some("Workspaces")).unwrap();
    choose.emit_clicked();
    assert_eq!(top_left.text(), "Nothing", "a panel lives in one corner, so its old corner clears");
    assert_eq!(prefs::get_str(&prefs::read(), "corner_top_right", ""), "workspaces");
    assert_eq!(prefs::get_str(&prefs::read(), "corner_top_left", ""), "none");

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

#[test]
fn recording_page_tracks_optional_recorder_and_indexes_rows() {
    use std::os::unix::fs::PermissionsExt;
    let Some(mode) = std::env::var_os("DIGIT_SETTINGS_RECORDING_TEST") else {
        let dir = std::env::temp_dir().join(format!("digit-recording-test-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        for mode in ["missing", "present"] {
            let root = dir.join(mode);
            let bin = root.join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            if mode == "present" {
                let program = bin.join("gpu-screen-recorder");
                std::fs::write(&program, "#!/bin/sh\nexit 0\n").unwrap();
                std::fs::set_permissions(program, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "tests::recording_page_tracks_optional_recorder_and_indexes_rows", "--nocapture"])
                .env("DIGIT_SETTINGS_RECORDING_TEST", mode)
                .env("XDG_CONFIG_HOME", root.join("config"))
                .env("XDG_STATE_HOME", root.join("state"))
                .env("XDG_CACHE_HOME", root.join("cache"))
                .env("XDG_DATA_HOME", root.join("data"))
                .env("PATH", &bin)
                .status().unwrap();
            assert!(status.success(), "recording page with {mode} recorder");
        }
        std::fs::remove_dir_all(dir).unwrap();
        return;
    };
    let present = mode == "present";
    assert_eq!(kit::optional::present(&kit::optional::GPU_SCREEN_RECORDER), present);
    if let Err(e) = gtk::init() {
        eprintln!("Skipping recording GTK checks without a display: {e}");
        return;
    }
    let page = crate::build_page("recording");
    let all = widgets(&page);
    let cards: Vec<_> = all.iter().filter(|w| w.has_css_class("card")).collect();
    assert_eq!(cards.len(), if present { 6 } else { 7 });
    let settings = if present { &cards[..] } else { &cards[1..] };
    assert!(settings.iter().all(|card| card.is_sensitive() == present));
    let store = all.iter().filter_map(|w| w.downcast_ref::<gtk::Button>()).find(|b| b.label().as_deref() == Some("Open Store"));
    assert_eq!(store.is_some(), !present);
    if let Some(store) = store { assert!(store.is_sensitive()); }
    let headings: Vec<_> = all.iter().filter(|w| w.has_css_class("heading")).filter_map(|w| w.downcast_ref::<gtk::Label>().map(|l| l.text().to_string())).collect();
    assert_eq!(headings, ["Capture", "Sound", "Instant Replay", "Saving", "Streaming", "Shortcuts"]);
    let rows: Vec<_> = all.iter().filter(|w| w.has_css_class("row")).collect();
    let shortcuts = widgets(*settings.last().unwrap());
    assert_eq!(shortcuts.iter().filter(|w| w.has_css_class("row")).count(), 1);
    assert!(shortcuts.iter().filter_map(|w| w.downcast_ref::<gtk::Label>()).any(|l| l.has_css_class("name") && l.text() == "Shortcuts"));
    assert!(shortcuts.iter().filter_map(|w| w.downcast_ref::<gtk::Label>()).any(|l| l.text() == "Start, stop, pause and replay recordings with keys"));
    let buttons: Vec<_> = shortcuts.iter().filter_map(|w| w.downcast_ref::<gtk::Button>()).collect();
    assert_eq!(buttons.len(), 1);
    assert_eq!(buttons[0].label().as_deref(), Some("Change…"));
    assert!(!all.iter().any(|w| w.has_css_class("keycap")));
    for bind in kit::keybinds::all().into_iter().filter(|b| b.group == "Recording") {
        assert!(!all.iter().filter_map(|w| w.downcast_ref::<gtk::Label>()).any(|l| l.text() == bind.label));
    }
    for (title, expected) in [("What to record", "The focused screen"), ("Frame rate", "60 fps"), ("Quality", "Very high"), ("Codec", "Automatic (H.264)"), ("Keep the last", "1 minute")] {
        let row = rows.iter().find(|r| widgets(**r).iter().filter_map(|w| w.downcast_ref::<gtk::Label>()).any(|l| l.text() == title)).unwrap();
        let pick = widgets(*row).into_iter().find_map(|w| w.downcast::<gtk::DropDown>().ok()).unwrap();
        assert_eq!(pick.selected_item().unwrap().downcast::<gtk::StringObject>().unwrap().string(), expected);
    }
    for (title, expected) in [("Show the pointer", true), ("System sound", true), ("Microphone", false)] {
        let row = rows.iter().find(|r| widgets(**r).iter().filter_map(|w| w.downcast_ref::<gtk::Label>()).any(|l| l.text() == title)).unwrap();
        let switch = widgets(*row).into_iter().find_map(|w| w.downcast::<gtk::Switch>().ok()).unwrap();
        assert_eq!(switch.is_active(), expected);
    }
    assert!(!prefs::path().exists(), "building the page must not write defaults");
    if present {
        let stream = all.iter().find_map(|w| w.downcast_ref::<gtk::PasswordEntry>()).unwrap();
        let path = glib::user_config_dir().join("digit/rec-stream");
        stream.set_text("rtmps://example.test/live/private-key");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), stream.text());
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        // Replacing a pre-existing file must repair its permissions too.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        stream.set_text("srt://example.test/private-key");
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        let rebuilt = crate::build_page("recording");
        assert_eq!(widgets(&rebuilt).iter().find_map(|w| w.downcast_ref::<gtk::PasswordEntry>()).unwrap().text(), stream.text());
        assert!(!prefs::path().exists(), "stream keys stay outside preferences");
        crate::search::refresh();
        let cache = std::fs::read_to_string(crate::search::cache()).unwrap();
        assert!(!cache.contains("private-key"));
        let index: Vec<serde_json::Value> = serde_json::from_str(&cache).unwrap();
        for (title, section) in [("What to record", "capture"), ("Frame rate", "capture"), ("System sound", "sound"), ("Keep the last", "instant-replay"), ("Save to", "saving"), ("Stream address", "streaming"), ("Shortcuts", "shortcuts")] {
            let link = format!("recording/{}/{section}", crate::search::slug(title));
            assert!(index.iter().any(|hit| hit["title"] == title && hit["link"] == link), "missing index row: {link}");
        }
        stream.set_text("");
        assert!(!path.exists(), "clearing the stream address removes its private file");
    }
}
