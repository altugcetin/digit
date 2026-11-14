mod apps;
mod bluetooth;
mod compositor;
mod defaults;
mod desktop;
mod displays;
mod drivers;
mod flatpak;
mod fw;
mod guard;
mod keyboard;
mod niri;
mod overview;
mod pk;
mod prefs;
mod rules;
mod search;
mod style;
mod system;
mod ui;
mod updates;
mod welcome;
mod wifi;
#[cfg(test)]
mod tests;

use gtk::{gio, glib, prelude::*};
use std::cell::RefCell;

/// Sidebar pages: id, name, icon and the group they're listed under.
pub(crate) const PAGES: [(&str, &str, &str, &str); 19] = [
    ("wifi", "Wi-Fi", "wifi", "Connections"),
    ("bluetooth", "Bluetooth", "bluetooth", "Connections"),
    ("appearance", "Appearance", "palette", "Personalize"),
    ("dock", "Dock", "dock_to_bottom", "Personalize"),
    ("notch", "Notch and menu bar", "toolbar", "Personalize"),
    ("desktop", "Desktop", "wallpaper", "Personalize"),
    ("spotlight", "Spotlight", "search", "Personalize"),
    ("compositor", "Windows", "select_window", "Workspace"),
    ("rules", "Window rules", "rule", "Workspace"),
    ("keyboard", "Keyboard", "keyboard", "Workspace"),
    ("displays", "Displays", "monitor", "Workspace"),
    ("power", "Lock and power", "power_settings_new", "System"),
    ("screenshots", "Screenshots", "screenshot_region", "System"),
    ("recording", "Recording", "radio_button_checked", "System"),
    ("files", "Files", "folder", "System"),
    ("apps", "Apps", "apps", "System"),
    ("updates", "Updates", "system_update_alt", "System"),
    ("drivers", "Drivers", "memory", "System"),
    ("overview", "About", "info", "System"),
];

pub(crate) fn page_name(id: &str) -> &'static str {
    PAGES.iter().find(|p| p.0 == id).map_or("", |p| p.1)
}

pub(crate) fn page_icon(id: &str) -> &'static str {
    PAGES.iter().find(|p| p.0 == id).map_or("settings", |p| p.2)
}

struct Shell {
    window: gtk::ApplicationWindow,
    stack: gtk::Stack,
    list: gtk::ListBox,
    find: gtk::SearchEntry,
    brand: gtk::DrawingArea,
    /// The page shown before a search, to go back to when it is cleared.
    current: RefCell<String>,
}

thread_local! {
    static SHELL: RefCell<Option<Shell>> = const { RefCell::new(None) };
}

fn build_page(name: &str) -> gtk::Widget {
    match name {
        "wifi" => wifi::build(),
        "bluetooth" => bluetooth::build(),
        "updates" => updates::build(),
        "apps" => apps::build(),
        "displays" => displays::build(),
        "drivers" => drivers::build(),
        "keyboard" => keyboard::build(),
        "compositor" => compositor::build(),
        "rules" => rules::build(),
        "appearance" => desktop::build(name, "Appearance", "How digit and its apps look. Changes apply right away."),
        "dock" => desktop::build(name, "Dock", ""),
        "notch" => desktop::build(name, "Notch and menu bar", "The notch, the bar beside it, the panels they open and the screen corners."),
        "desktop" => desktop::build(name, "Desktop", "What sits on the wallpaper, behind your windows."),
        "spotlight" => desktop::build(name, "Spotlight", &match kit::keybinds::shown("launcher") {
            Some(k) => format!("{k} searches apps, files, settings and the web."),
            None => "Searches apps, files, settings and the web.".into(),
        }),
        "power" => desktop::build(name, "Lock and power", ""),
        "screenshots" => desktop::build(name, "Screenshots", ""),
        "recording" => desktop::build(name, "Recording", "Changes apply the next time a recording starts."),
        "files" => desktop::build(name, "Files", ""),
        _ => overview::build(),
    }
}

/// Opens a page by id, or a row on it by `page/row-name` ("dock/icon-size"), with the
/// section after another slash when two rows share a name ("appearance/surfaces/apps").
pub(crate) fn show(name: &str) {
    let mut parts = name.splitn(3, '/');
    let name = parts.next().unwrap_or_default();
    let row = parts.next().unwrap_or_default();
    let within = parts.next().filter(|s| !s.is_empty());
    // Pages that moved keep their old links working.
    let (name, row) = match name {
        "night" => ("displays", "night-light"),
        other => (other, row),
    };
    SHELL.with(|s| {
        let Some(s) = s.borrow().as_ref().map(|s| (s.stack.clone(), s.list.clone(), s.current.clone())) else { return };
        let (stack, list, current) = s;
        let name = if PAGES.iter().any(|p| p.0 == name) { name } else { "overview" };
        *current.borrow_mut() = name.to_string();
        // Optional recording tools may have been installed since the last visit.
        if name == "recording" && let Some(old) = stack.child_by_name(name) {
            stack.remove(&old);
        }
        if stack.child_by_name(name).is_none() {
            stack.add_named(&build_page(name), Some(name));
            glib::timeout_add_local_once(std::time::Duration::from_millis(1500), || unsafe {
                libc::malloc_trim(0);
            });
        }
        stack.set_visible_child_name(name);
        let keep = name.to_string();
        let st = stack.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
            let mut child = st.first_child();
            let mut gone = false;
            while let Some(c) = child {
                child = c.next_sibling();
                let page = st.page(&c);
                if page.name().is_some_and(|n| n != keep) && st.visible_child_name().is_some_and(|v| v == keep) {
                    st.remove(&c);
                    gone = true;
                }
            }
            if gone {
                glib::timeout_add_local_once(std::time::Duration::from_millis(300), || unsafe {
                    libc::malloc_trim(0);
                });
            }
        });
        let mut child = list.first_child();
        while let Some(c) = child {
            child = c.next_sibling();
            if let Ok(row) = c.downcast::<gtk::ListBoxRow>() {
                if row.widget_name() == name && !row.is_selected() {
                    list.select_row(Some(&row));
                }
            }
        }
    });
    if !row.is_empty() {
        search::reveal(name, within, row);
    }
}

/// The built widget of a page, while it is in the stack.
pub(crate) fn page_widget(name: &str) -> Option<gtk::Widget> {
    SHELL.with(|s| s.borrow().as_ref().and_then(|s| s.stack.child_by_name(name)))
}

/// Empties the search field without going back to the page shown before it.
pub(crate) fn clear_search() {
    let find = SHELL.with(|s| s.borrow().as_ref().map(|s| s.find.clone()));
    if let Some(f) = find {
        unsafe { f.set_data("digit-quiet", true) };
        f.set_text("");
        unsafe { f.steal_data::<bool>("digit-quiet") };
    }
}

fn searched(entry: &gtk::SearchEntry) {
    if unsafe { entry.data::<bool>("digit-quiet") }.is_some() {
        return;
    }
    let Some((stack, list, current)) = SHELL.with(|s| s.borrow().as_ref().map(|s| (s.stack.clone(), s.list.clone(), s.current.borrow().clone()))) else { return };
    let query = entry.text();
    if query.trim().is_empty() {
        show(&current);
        if let Some(old) = stack.child_by_name("search") {
            stack.remove(&old);
        }
        return;
    }
    list.unselect_all();
    search::update(&stack, &query);
}

const APP_ID: &str = "dev.digit.Settings";

fn brand() -> gtk::DrawingArea {
    let area = kit::pictures::brand(3.0, 34.0);
    area.set_halign(gtk::Align::Start);
    area.set_margin_start(12);
    area.set_margin_top(6);
    area.set_margin_bottom(18);
    area
}

fn refresh_brand() {
    SHELL.with(|s| {
        if let Some(shell) = s.borrow().as_ref() {
            shell.brand.queue_draw();
        }
    });
}

fn install() {
    let ink = style::ink();
    let (_, w, h) = kit::pixel::cells("d");
    let cell = 12.0;
    let (ox, oy) = ((128.0 - w as f64 * cell) / 2.0, (128.0 - h as f64 * cell) / 2.0);
    let (d, _, _) = kit::pixel::path("d", cell, ox, oy);
    let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128" viewBox="0 0 128 128"><rect width="128" height="128" rx="30" fill="{}"/><path d="{d}" fill="{}"/></svg>"#, ink.accent, ink.on_accent);
    let data = glib::user_data_dir();
    let desktop = format!("[Desktop Entry]\nType=Application\nName=Digit Settings\nGenericName=Settings\nComment=System, apps, drivers and desktop settings\nExec=digit-settings\nIcon={APP_ID}\nCategories=Settings;System;\nStartupNotify=true\n");
    for (path, body) in [(data.join(format!("icons/hicolor/scalable/apps/{APP_ID}.svg")), svg), (data.join(format!("applications/{APP_ID}.desktop")), desktop)] {
        if std::fs::read_to_string(&path).ok().as_deref() == Some(body.as_str()) {
            continue;
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("new");
        if std::fs::write(&tmp, &body).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
        if path.extension().is_some_and(|e| e == "svg") {
            let theme = data.join("icons/hicolor");
            let argv: [&std::ffi::OsStr; 5] = ["gtk4-update-icon-cache".as_ref(), "-q".as_ref(), "-t".as_ref(), "-f".as_ref(), theme.as_os_str()];
            if gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE).is_err() {
                let _ = std::fs::File::open(&theme).and_then(|f| f.set_modified(std::time::SystemTime::now()));
            }
        }
    }
}

fn control(icon: &str, kind: &str) -> gtk::Button {
    let b = gtk::Button::new();
    b.set_child(Some(&ui::sym(icon)));
    b.add_css_class("ctl");
    if !kind.is_empty() {
        b.add_css_class(kind);
    }
    b.set_valign(gtk::Align::Center);
    b
}

fn header(window: &gtk::ApplicationWindow) -> gtk::Widget {
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    bar.add_css_class("controls");
    bar.set_halign(gtk::Align::End);
    bar.set_valign(gtk::Align::Start);
    let min = control("remove", "");
    let max = control(if window.is_maximized() { "filter_none" } else { "crop_square" }, "");
    let close = control("close", "close");
    min.set_tooltip_text(Some("Minimize"));
    max.set_tooltip_text(Some("Maximize"));
    close.set_tooltip_text(Some("Close"));
    min.connect_clicked(|_| {
        glib::spawn_future_local(async {
            let moved = gio::spawn_blocking(|| kit::wm::minimize(APP_ID)).await.unwrap_or(false);
            if !moved {
                ui::fail("Nowhere to minimize to");
            }
        });
    });
    let w = window.clone();
    max.connect_clicked(move |_| {
        if w.is_maximized() {
            w.unmaximize();
        } else {
            w.maximize();
        }
    });
    let m = max.clone();
    window.connect_maximized_notify(move |w| {
        m.set_child(Some(&ui::sym(if w.is_maximized() { "filter_none" } else { "crop_square" })));
    });
    let w = window.clone();
    close.connect_clicked(move |_| w.close());
    // An overlay can't be minimized or maximized; it only closes.
    if !gtk4_layer_shell::LayerShell::is_layer_window(window) {
        bar.append(&min);
        bar.append(&max);
    }
    bar.append(&close);
    bar.upcast()
}

/// The main Settings window, while it is open.
pub fn main_window() -> Option<gtk::ApplicationWindow> {
    SHELL.with(|s| s.borrow().as_ref().map(|s| s.window.clone()))
}

/// A modal owns typing; the search field must not capture keys from behind it.
pub(crate) fn search_enabled(window: &gtk::Window, enabled: bool) {
    SHELL.with(|s| {
        if let Some(s) = s.borrow().as_ref().filter(|s| s.window.upcast_ref::<gtk::Window>() == window) {
            s.find.set_sensitive(enabled);
            s.find.set_key_capture_widget(if enabled { Some(window.upcast_ref::<gtk::Widget>()) } else { None });
        }
    });
}

/// Child indices keep focus on the same control when a settings page is rebuilt.
fn focus_path(page: &gtk::Widget) -> Option<Vec<usize>> {
    let mut widget = page.root()?.focus()?;
    let mut path = Vec::new();
    while widget != *page {
        let mut index = 0;
        let mut sibling = widget.prev_sibling();
        while let Some(w) = sibling { index += 1; sibling = w.prev_sibling(); }
        path.push(index);
        widget = widget.parent()?;
    }
    path.reverse();
    Some(path)
}

fn restore_page(page: &gtk::Widget, value: f64, focus: Option<Vec<usize>>) {
    let Some(scroll) = page.downcast_ref::<gtk::ScrolledWindow>() else { return };
    let target = focus.and_then(|path| {
        let mut widget = page.clone();
        for index in path {
            widget = widget.first_child()?;
            for _ in 0..index { widget = widget.next_sibling()?; }
        }
        Some(widget.downgrade())
    });
    let viewport = scroll.child().and_downcast::<gtk::Viewport>().map(|v| v.downgrade());
    let restored = std::cell::Cell::new(false);
    // A new adjustment has no range until layout. Restore during its first usable
    // range change, before painting, so set_value cannot be clamped to zero.
    scroll.vadjustment().connect_changed(move |adj| {
        if restored.get() || adj.page_size() <= 0.0 || adj.upper() - adj.page_size() < value { return; }
        restored.set(true);
        if let Some(target) = target.as_ref().and_then(|w| w.upgrade()) {
            let viewport = viewport.as_ref().and_then(|v| v.upgrade());
            let follow = viewport.as_ref().is_some_and(|v| v.is_scroll_to_focus());
            if let Some(v) = &viewport { v.set_scroll_to_focus(false); }
            target.grab_focus();
            if let Some(v) = &viewport { v.set_scroll_to_focus(follow); }
        }
        adj.set_value(value);
    });
}

pub(crate) fn reload(name: &str) {
    search::forget();
    SHELL.with(|s| {
        let Some(stack) = s.borrow().as_ref().map(|s| s.stack.clone()) else { return };
        let Some(old) = stack.child_by_name(name) else { return };
        let visible = stack.visible_child_name().is_some_and(|v| v == name);
        let value = old.downcast_ref::<gtk::ScrolledWindow>().map_or(0.0, |s| s.vadjustment().value());
        let focus = if visible { focus_path(&old) } else { None };
        stack.remove(&old);
        if visible {
            let page = build_page(name);
            restore_page(&page, value, focus);
            stack.add_named(&page, Some(name));
            stack.set_visible_child_name(name);
        }
    });
}

pub(crate) fn reload_reductions() {
    for name in ["appearance", "spotlight", "compositor", "notch"] {
        reload(name);
    }
}

fn watch(window: &gtk::ApplicationWindow) {
    let dir = gio::File::for_path(prefs::path().parent().map(|p| p.to_path_buf()).unwrap_or_default());
    let _ = std::fs::create_dir_all(prefs::path().parent().map(|p| p.to_path_buf()).unwrap_or_default());
    let Ok(monitor) = dir.monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE) else { return };
    let pending = std::rc::Rc::new(std::cell::Cell::new(false));
    monitor.connect_changed(move |_, file, other, _| {
        let hit = |f: &gio::File| f.basename().is_some_and(|b| b.as_os_str() == "config.toml");
        if !(hit(file) || other.is_some_and(hit)) || prefs::ours() || pending.replace(true) {
            return;
        }
        let pending = pending.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || {
            pending.set(false);
            if !prefs::ours() {
                kit::motion::refresh();
                kit::effects::invalidate();
                // Pages made of settings show the new values; the others don't read config.toml.
                let visible = SHELL.with(|s| s.borrow().as_ref().map(|s| s.current.borrow().clone())).unwrap_or_default();
                reload_reductions();
                if !["appearance", "spotlight", "compositor", "notch"].contains(&visible.as_str()) && !desktop::sections(&visible).is_empty() && visible != "displays" {
                    reload(&visible);
                }
            }
        });
    });
    unsafe { window.set_data("digit-watch", monitor) };
}

fn window(app: &gtk::Application) -> gtk::ApplicationWindow {
    let window = gtk::ApplicationWindow::builder().application(app).title("Settings").default_width(1080).default_height(720).build();
    window.connect_close_request(|_| {
        displays::revert_pending();
        glib::Propagation::Proceed
    });
    kit::scroll::smooth(&window);
    window.set_decorated(false);
    // Settings opens as a full-screen overlay unless it is set to open as a window.
    let overlay = !prefs::get_bool(&prefs::read(), "settings_window", false);
    if overlay {
        use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
        window.add_css_class("digit-dialog");
        window.init_layer_shell();
        window.set_namespace(Some("digit-settings"));
        window.set_layer(Layer::Overlay);
        window.set_keyboard_mode(KeyboardMode::Exclusive);
        for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            window.set_anchor(edge, true);
        }
        window.set_exclusive_zone(-1);
    } else {
        window.add_css_class("digit-settings");
    }
    let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let side = gtk::Box::new(gtk::Orientation::Vertical, 0);
    side.add_css_class("side");
    side.set_size_request(232, -1);
    let top = gtk::WindowHandle::new();
    let brand = brand();
    top.set_child(Some(&brand));
    side.append(&top);
    let find = gtk::SearchEntry::new();
    find.add_css_class("search");
    find.add_css_class("side-search");
    find.set_placeholder_text(Some("Search settings"));
    side.append(&find);
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);
    let mut group = "";
    for (id, label, icon, g) in PAGES {
        if g != group {
            let head = gtk::ListBoxRow::new();
            head.set_child(Some(&ui::text(g, "")));
            head.add_css_class("group");
            if group.is_empty() {
                head.add_css_class("first");
            }
            head.set_selectable(false);
            head.set_activatable(false);
            head.set_can_focus(false);
            list.append(&head);
            group = g;
        }
        let b = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let t = ui::tile(icon);
        t.add_css_class("small");
        b.append(&t);
        b.append(&ui::text(label, "name"));
        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&b));
        row.set_widget_name(id);
        list.append(&row);
    }
    list.connect_row_selected(|_, row| {
        if let Some(r) = row {
            show(&r.widget_name());
        }
    });
    let pages = gtk::ScrolledWindow::new();
    // The list scrolls without a scrollbar.
    pages.set_policy(gtk::PolicyType::Never, gtk::PolicyType::External);
    pages.set_vexpand(true);
    pages.set_child(Some(&list));
    side.append(&pages);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.set_hexpand(true);
    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_transition_duration(160);
    stack.set_vexpand(true);
    let layer = gtk::Overlay::new();
    layer.set_child(Some(&stack));
    LAYER.with(|l| *l.borrow_mut() = Some(layer.clone()));
    let strip = gtk::WindowHandle::new();
    strip.set_valign(gtk::Align::Start);
    strip.set_size_request(-1, 26);
    layer.add_overlay(&strip);
    layer.add_overlay(&header(&window));
    content.append(&layer);
    content.append(&ui::activity());
    root.append(&side);
    root.append(&content);
    if overlay {
        let mon = gtk::gdk::Display::default().and_then(|d| d.monitors().item(0)).and_downcast::<gtk::gdk::Monitor>().map(|m| m.geometry());
        let (mw, mh) = mon.map(|g| (g.width(), g.height())).unwrap_or((1600, 900));
        // The size it was last resized to, or a default; never bigger than the screen.
        let (w, h) = kit::ui::remembered("settings_size").unwrap_or((1080.min(mw * 92 / 100), 720.min(mh * 88 / 100)));
        root.set_size_request(w.clamp(480, (mw - 32).max(480)), h.clamp(360, (mh - 32).max(360)));
        let (layer, backdrop) = ui::overlay_card(&root);
        window.set_child(Some(&layer));
        ui::overlay_handling(window.upcast_ref(), &backdrop, &root, Some("settings_size"));
        backdrop.set_opacity(0.0);
        ui::settle(&backdrop, &root, 0, true, || {});
        let closing = std::rc::Rc::new(std::cell::Cell::new(false));
        let root = root.clone();
        window.connect_close_request(move |w| {
            if closing.replace(true) {
                return glib::Propagation::Proceed;
            }
            let w = w.clone();
            ui::settle(&backdrop, &root, 0, false, move || w.close());
            glib::Propagation::Stop
        });
    } else {
        let layer = gtk::Overlay::new();
        layer.set_widget_name(ui::SHEET_LAYER);
        layer.set_child(Some(&root));
        window.set_child(Some(&layer));
    }
    let keys = gtk::EventControllerKey::new();
    let w = window.clone();
    keys.connect_key_pressed(move |_, key, _, mods| {
        if overlay && key == gtk::gdk::Key::Escape {
            w.close();
            return glib::Propagation::Stop;
        }
        if key == gtk::gdk::Key::w && mods.contains(gtk::gdk::ModifierType::CONTROL_MASK) || key == gtk::gdk::Key::q && mods.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
            w.close();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(keys);
    find.connect_search_changed(searched);
    find.connect_activate(|_| search::activate_first());
    // Esc empties the field first, then closes Settings like it does elsewhere.
    let w = window.clone();
    find.connect_stop_search(move |f| {
        if f.text().is_empty() {
            if overlay {
                w.close();
            }
        } else {
            f.set_text("");
        }
    });
    // Typing anywhere that isn't a text field starts a search.
    find.set_key_capture_widget(Some(&window));
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let f = find.clone();
    keys.connect_key_pressed(move |_, key, _, mods| {
        if key == gtk::gdk::Key::f && mods.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
            f.grab_focus();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(keys);
    SHELL.with(|s| *s.borrow_mut() = Some(Shell { window: window.clone(), stack, list, find, brand, current: RefCell::new("overview".into()) }));
    watch(&window);
    window
}

fn present(app: &gtk::Application, page: Option<&str>) {
    if page == Some("welcome") {
        if let Some(display) = gtk::gdk::Display::default() {
            if SHELL.with(|s| s.borrow().is_none()) {
                kit::style::live_with(&display, style::css, refresh_brand);
            }
        }
        welcome::open(app);
        return;
    }
    let existing = SHELL.with(|s| s.borrow().as_ref().map(|s| s.window.clone()));
    let window = existing.unwrap_or_else(|| {
        if let Some(display) = gtk::gdk::Display::default() {
            kit::style::live_with(&display, style::css, refresh_brand);
        }
        window(app)
    });
    // "?words" opens Settings searching for them, as Spotlight's "set" key does.
    match page.and_then(|p| p.strip_prefix('?')) {
        Some(words) => {
            if let Some(find) = SHELL.with(|s| s.borrow().as_ref().map(|s| s.find.clone())) {
                find.set_text(words);
                find.grab_focus();
                find.set_position(-1);
            }
        }
        None => show(page.unwrap_or("overview")),
    }
    window.present();
}

kit::layer_shell_first!();

fn main() -> glib::ExitCode {
    if std::env::args().nth(1).as_deref() == Some("--register") {
        install();
        return glib::ExitCode::SUCCESS;
    }
    if std::env::args().nth(1).as_deref() == Some("--prepare-session") {
        return match kit::setup::prepare(&std::env::args().nth(2).unwrap_or_default()) {
            Ok(()) => glib::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                glib::ExitCode::FAILURE
            }
        };
    }
    // The installer's "Add NVIDIA variables", set before it writes the session config.
    if std::env::args().nth(1).as_deref() == Some("--nvidia-env") {
        let on = match std::env::args().nth(2).as_deref() {
            Some("on") => true,
            Some("off") => false,
            _ => {
                eprintln!("Usage: digit-settings --nvidia-env on|off");
                return glib::ExitCode::FAILURE;
            }
        };
        return match prefs::set("nvidia_env", toml::Value::Boolean(on)) {
            Ok(()) => glib::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                glib::ExitCode::FAILURE
            }
        };
    }
    if matches!(std::env::args().nth(1).as_deref(), Some("--hypr-setup" | "--niri-setup")) {
        println!("Log out and select Hyprland with Digit or niri with Digit.");
        return glib::ExitCode::SUCCESS;
    }
    // Spotlight asks for the settings index this way when it has none or it is out of date.
    if std::env::args().nth(1).as_deref() == Some("--index") {
        if gtk::init().is_err() {
            return glib::ExitCode::FAILURE;
        }
        search::refresh();
        return glib::ExitCode::SUCCESS;
    }
    if std::env::args().nth(1).as_deref() == Some("--sync") {
        return match kit::setup::sync_all() {
            Ok(_) => glib::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                glib::ExitCode::FAILURE
            }
        };
    }
    if std::env::args().nth(1).as_deref() == Some("--hypr-sync") {
        return match kit::hyprconf::apply() {
            Ok(()) => glib::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                glib::ExitCode::FAILURE
            }
        };
    }
    kit::effects::select("DIGIT_SETTINGS_RENDERER");
    unsafe {
        libc::mallopt(libc::M_MMAP_THRESHOLD, 256 * 1024);
        libc::mallopt(libc::M_ARENA_MAX, 2);
    }
    install();
    let app = gtk::Application::new(Some(APP_ID), gio::ApplicationFlags::HANDLES_COMMAND_LINE);
    app.connect_startup(|_| gtk::Window::set_default_icon_name(APP_ID));
    app.connect_shutdown(|_| displays::revert_pending());
    app.connect_command_line(|app, cmd| {
        let args = cmd.arguments();
        let page = args.get(1).and_then(|a| a.to_str().map(str::to_string));
        present(app, page.as_deref());
        glib::ExitCode::SUCCESS
    });
    app.run()
}

thread_local! {
    static LAYER: std::cell::RefCell<Option<gtk::Overlay>> = const { std::cell::RefCell::new(None) };
}

/// The overlay over the page area, where in-window dialogs sit.
pub fn layer() -> Option<gtk::Overlay> {
    LAYER.with(|l| l.borrow().clone())
}
