mod apps;
mod desktop;
mod drivers;
mod flatpak;
mod fw;
mod guard;
mod niri;
mod overview;
mod pk;
mod prefs;
mod style;
mod system;
mod ui;
mod updates;
mod welcome;

use gtk::{gio, glib, prelude::*};
use std::cell::RefCell;

const PAGES: [(&str, &str, &str); 5] = [
    ("overview", "Overview", "computer"),
    ("updates", "Updates", "system_update_alt"),
    ("apps", "Apps", "apps"),
    ("drivers", "Drivers", "memory"),
    ("desktop", "Desktop", "palette"),
];

struct Shell {
    window: gtk::ApplicationWindow,
    stack: gtk::Stack,
    list: gtk::ListBox,
}

thread_local! {
    static SHELL: RefCell<Option<Shell>> = const { RefCell::new(None) };
}

fn build_page(name: &str) -> gtk::Widget {
    match name {
        "updates" => updates::build(),
        "apps" => apps::build(),
        "drivers" => drivers::build(),
        "desktop" => desktop::build(),
        _ => overview::build(),
    }
}

fn show(name: &str) {
    SHELL.with(|s| {
        let Some(s) = s.borrow().as_ref().map(|s| (s.stack.clone(), s.list.clone())) else { return };
        let (stack, list) = s;
        let name = if PAGES.iter().any(|p| p.0 == name) { name } else { "overview" };
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
        if let Some(i) = PAGES.iter().position(|p| p.0 == name) {
            if let Some(row) = list.row_at_index(i as i32) {
                if !row.is_selected() {
                    list.select_row(Some(&row));
                }
            }
        }
    });
}

const APP_ID: &str = "dev.digit.Settings";

fn rgb(hex: &str) -> (f64, f64, f64) {
    let v = |i: usize| u8::from_str_radix(hex.get(i..i + 2).unwrap_or("ff"), 16).unwrap_or(255) as f64 / 255.0;
    (v(1), v(3), v(5))
}

fn brand() -> gtk::Widget {
    let ink = style::ink();
    let (_, ww, _) = kit::pixel::cells("Digit");
    let (_, mw, mh) = kit::pixel::cells("d");
    let cell = 3.0;
    let tile = 34.0;
    let width = tile + 12.0 + ww as f64 * cell;
    let area = gtk::DrawingArea::new();
    area.set_content_width(width as i32);
    area.set_content_height(tile as i32);
    area.set_halign(gtk::Align::Start);
    area.set_margin_start(12);
    area.set_margin_top(6);
    area.set_margin_bottom(18);
    let (accent, on_accent, main) = (rgb(&ink.accent), rgb(&ink.on_accent), rgb(&ink.main));
    area.set_draw_func(move |_, cr, _, _| {
        let r = 9.0;
        cr.new_sub_path();
        cr.arc(tile - r, r, r, -std::f64::consts::FRAC_PI_2, 0.0);
        cr.arc(tile - r, tile - r, r, 0.0, std::f64::consts::FRAC_PI_2);
        cr.arc(r, tile - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
        cr.arc(r, r, r, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
        cr.close_path();
        cr.set_source_rgb(accent.0, accent.1, accent.2);
        let _ = cr.fill();
        let ox = ((tile - mw as f64 * cell) / 2.0).round();
        let oy = ((tile - mh as f64 * cell) / 2.0).round();
        cr.set_source_rgb(on_accent.0, on_accent.1, on_accent.2);
        kit::pixel::trace(cr, "d", cell, ox, oy);
        let _ = cr.fill();
        let tx = tile + 12.0;
        let ty = ((tile - 7.0 * cell) / 2.0).round();
        cr.set_source_rgb(main.0, main.1, main.2);
        kit::pixel::trace(cr, "Digit", cell, tx, ty);
        let _ = cr.fill();
    });
    area.upcast()
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
    bar.append(&min);
    bar.append(&max);
    bar.append(&close);
    bar.upcast()
}

fn reload(name: &str) {
    SHELL.with(|s| {
        let Some(stack) = s.borrow().as_ref().map(|s| s.stack.clone()) else { return };
        let Some(old) = stack.child_by_name(name) else { return };
        let visible = stack.visible_child_name().is_some_and(|v| v == name);
        stack.remove(&old);
        if visible {
            stack.add_named(&build_page(name), Some(name));
            stack.set_visible_child_name(name);
        }
    });
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
                reload("desktop");
            }
        });
    });
    unsafe { window.set_data("digit-watch", monitor) };
}

fn window(app: &gtk::Application) -> gtk::ApplicationWindow {
    let window = gtk::ApplicationWindow::builder().application(app).title("Settings").default_width(1080).default_height(720).build();
    window.add_css_class("digit-settings");
    window.set_decorated(false);
    let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let side = gtk::Box::new(gtk::Orientation::Vertical, 0);
    side.add_css_class("side");
    side.set_size_request(232, -1);
    let top = gtk::WindowHandle::new();
    top.set_child(Some(&brand()));
    side.append(&top);
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);
    for (_, label, icon) in PAGES {
        let b = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let t = ui::tile(icon);
        t.add_css_class("small");
        b.append(&t);
        b.append(&ui::text(label, "name"));
        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&b));
        list.append(&row);
    }
    list.connect_row_selected(|_, row| {
        if let Some(r) = row {
            if let Some(p) = PAGES.get(r.index() as usize) {
                show(p.0);
            }
        }
    });
    side.append(&list);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.set_hexpand(true);
    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_transition_duration(160);
    stack.set_vexpand(true);
    let layer = gtk::Overlay::new();
    layer.set_child(Some(&stack));
    let strip = gtk::WindowHandle::new();
    strip.set_valign(gtk::Align::Start);
    strip.set_size_request(-1, 26);
    layer.add_overlay(&strip);
    layer.add_overlay(&header(&window));
    content.append(&layer);
    content.append(&ui::activity());
    root.append(&side);
    root.append(&content);
    window.set_child(Some(&root));
    let keys = gtk::EventControllerKey::new();
    let w = window.clone();
    keys.connect_key_pressed(move |_, key, _, mods| {
        if key == gtk::gdk::Key::w && mods.contains(gtk::gdk::ModifierType::CONTROL_MASK) || key == gtk::gdk::Key::q && mods.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
            w.close();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(keys);
    SHELL.with(|s| *s.borrow_mut() = Some(Shell { window: window.clone(), stack, list }));
    watch(&window);
    window
}

fn present(app: &gtk::Application, page: Option<&str>) {
    if page == Some("welcome") {
        if let Some(display) = gtk::gdk::Display::default() {
            if SHELL.with(|s| s.borrow().is_none()) {
                kit::style::live(&display, style::css);
            }
        }
        welcome::open(app);
        return;
    }
    let existing = SHELL.with(|s| s.borrow().as_ref().map(|s| s.window.clone()));
    let window = existing.unwrap_or_else(|| {
        if let Some(display) = gtk::gdk::Display::default() {
            kit::style::live(&display, style::css);
        }
        window(app)
    });
    show(page.unwrap_or("overview"));
    window.present();
}

fn main() -> glib::ExitCode {
    if std::env::args().nth(1).as_deref() == Some("--register") {
        install();
        return glib::ExitCode::SUCCESS;
    }
    let renderer = std::env::var("DIGIT_SETTINGS_RENDERER").ok().filter(|r| matches!(r.as_str(), "cairo" | "gl" | "ngl" | "vulkan")).unwrap_or_else(|| "cairo".into());
    unsafe {
        std::env::set_var("GSK_RENDERER", renderer);
        libc::mallopt(libc::M_MMAP_THRESHOLD, 256 * 1024);
        libc::mallopt(libc::M_ARENA_MAX, 2);
    }
    install();
    let app = gtk::Application::new(Some(APP_ID), gio::ApplicationFlags::HANDLES_COMMAND_LINE);
    app.connect_startup(|_| gtk::Window::set_default_icon_name(APP_ID));
    app.connect_command_line(|app, cmd| {
        let args = cmd.arguments();
        let page = args.get(1).and_then(|a| a.to_str().map(str::to_string));
        present(app, page.as_deref());
        glib::ExitCode::SUCCESS
    });
    app.run()
}
