use crate::desktop::{accent, choice, corners, save, slider, surfaces};
use crate::{prefs, ui};
use gtk::{gio, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::cell::Cell;
use std::rc::Rc;

const STEPS: usize = 7;

pub fn marker() -> std::path::PathBuf {
    kit::theme::state_dir().join("onboarded")
}

fn finish(window: &gtk::Window) {
    let _ = std::fs::create_dir_all(kit::theme::state_dir());
    let _ = std::fs::write(marker(), "1\n");
    window.close();
}

fn spawn(argv: &[&str]) {
    let argv: Vec<&std::ffi::OsStr> = argv.iter().map(std::ffi::OsStr::new).collect();
    if gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE).is_err() {
        ui::fail("digit isn't running");
    }
}

fn notch_tile() -> gtk::Widget {
    let area = gtk::DrawingArea::new();
    area.set_content_width(42);
    area.set_content_height(42);
    area.set_halign(gtk::Align::Center);
    area.set_draw_func(|_, cr, w, h| {
        let (w, h) = (w as f64, h as f64);
        let pi = std::f64::consts::PI;
        let round = |x: f64, y: f64, rw: f64, rh: f64, r: f64| {
            cr.new_sub_path();
            cr.arc(x + rw - r, y + r, r, -pi / 2.0, 0.0);
            cr.arc(x + rw - r, y + rh - r, r, 0.0, pi / 2.0);
            cr.arc(x + r, y + rh - r, r, pi / 2.0, pi);
            cr.arc(x + r, y + r, r, pi, 1.5 * pi);
            cr.close_path();
        };
        let g = gtk::cairo::LinearGradient::new(0.0, 0.0, 0.0, h);
        g.add_color_stop_rgb(0.0, 1.0, 0.70, 0.25);
        g.add_color_stop_rgb(1.0, 0.96, 0.53, 0.0);
        round(0.0, 0.0, w, h, 12.0);
        let _ = cr.set_source(&g);
        let _ = cr.fill();
        let (sx, sy, sw, sh) = (8.0, 11.0, w - 16.0, h - 20.0);
        round(sx, sy, sw, sh, 4.0);
        cr.set_source_rgb(1.0, 1.0, 1.0);
        let _ = cr.fill();
        let (cx, half, depth, r, f) = (w / 2.0, 6.5, 4.5, 2.0, 1.6);
        cr.new_sub_path();
        cr.move_to(cx - half - f, sy);
        cr.arc(cx - half - f, sy + f, f, -pi / 2.0, 0.0);
        cr.line_to(cx - half, sy + depth - r);
        cr.arc_negative(cx - half + r, sy + depth - r, r, pi, pi / 2.0);
        cr.line_to(cx + half - r, sy + depth);
        cr.arc_negative(cx + half - r, sy + depth - r, r, pi / 2.0, 0.0);
        cr.line_to(cx + half, sy + f);
        cr.arc(cx + half + f, sy + f, f, pi, 1.5 * pi);
        cr.close_path();
        cr.set_source_rgb(0.08, 0.08, 0.09);
        let _ = cr.fill();
    });
    area.upcast()
}

fn header(icon: &str, title: &str, lead: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
    b.set_halign(gtk::Align::Center);
    let tile: gtk::Widget = if icon == "notch" {
        notch_tile()
    } else {
        let t = ui::tile(icon);
        t.add_css_class("big");
        t.set_halign(gtk::Align::Center);
        t.upcast()
    };
    tile.set_margin_bottom(6);
    b.append(&tile);
    let t = ui::text(title, "welcome-title");
    t.set_xalign(0.5);
    b.append(&t);
    let l = ui::text(lead, "lead");
    l.set_xalign(0.5);
    l.set_justify(gtk::Justification::Center);
    l.set_wrap(true);
    l.set_max_width_chars(58);
    b.append(&l);
    b
}

fn styles(t: &toml::Table) -> gtk::Widget {
    let current = prefs::get_str(t, "notch_style", "island");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    row.set_halign(gtk::Align::Center);
    let mut first: Option<gtk::ToggleButton> = None;
    for (key, name, sub) in [("island", "Notch", "Hugs the top edge"), ("dynamic", "Dynamic Island", "A floating pill"), ("bar", "Bar", "Spans the whole edge")] {
        let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let picture = kit::pictures::notch(key);
        picture.set_content_width(188);
        picture.set_content_height(100);
        card.append(&picture);
        let n = ui::text(name, "name");
        n.set_xalign(0.5);
        card.append(&n);
        let s = ui::text(sub, "sub");
        s.set_xalign(0.5);
        card.append(&s);
        let b = gtk::ToggleButton::new();
        b.add_css_class("style-card");
        b.set_child(Some(&card));
        match &first {
            Some(f) => b.set_group(Some(f)),
            None => first = Some(b.clone()),
        }
        b.set_active(current == key);
        b.connect_toggled(move |b| {
            if b.is_active() {
                save("notch_style", toml::Value::String(key.into()));
            }
        });
        row.append(&b);
    }
    row.upcast()
}

fn shortcuts() -> gtk::Widget {
    let card = ui::card();
    for (id, what) in [
        ("notch", "Open the notch"),
        ("launcher", "Spotlight: apps, files, windows, math"),
        ("wallpaper", "Wallpapers"),
        ("control", "Control center"),
        ("clipboard", "Clipboard history"),
        ("windows", "Switch windows"),
        ("spaces", "Workspaces"),
        ("layouts", "Window layouts"),
        ("snap", "Screenshot an area"),
        ("files", "Files"),
        ("lock", "Lock the screen"),
        ("session", "Sleep, restart, shut down or log out"),
    ] {
        // Only the ones that are on, with the keys the user set.
        let Some(keys) = kit::keybinds::shown(id) else { continue };
        let keys = keys.replace('+', " ");
        let (row, end) = ui::row(None, what, "");
        end.append(&ui::badge(&keys, ""));
        card.append(&row);
    }
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
    scroll.set_child(Some(&card));
    scroll.set_vexpand(true);
    scroll.upcast()
}

fn step(index: usize, t: &toml::Table) -> gtk::Widget {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 18);
    page.set_margin_start(48);
    page.set_margin_end(48);
    page.set_margin_top(10);
    match index {
        0 => {
            page.set_valign(gtk::Align::Center);
            let logo = kit::ui::distro_logo(72, false);
            logo.set_halign(gtk::Align::Center);
            page.append(&logo);
            let title = ui::text("Welcome to Digit", "welcome-hero");
            title.set_xalign(0.5);
            page.append(&title);
            let lead = ui::text(&format!("A calm, fast shell for {}. Let's make it yours: every choice here applies instantly, and you can change all of it later in Settings.", kit::distro::name()), "lead");
            lead.set_xalign(0.5);
            lead.set_justify(gtk::Justification::Center);
            lead.set_wrap(true);
            lead.set_max_width_chars(56);
            lead.set_halign(gtk::Align::Center);
            page.append(&lead);
        }
        1 => {
            page.append(&header("notch", "Pick your notch", "The notch shows the time, music, calls and notifications. Choose how it sits on the screen."));
            page.append(&styles(t));
            let card = ui::card();
            crate::desktop::toggle(&card, "menu", "App menus", "The focused app's name and menus at the top left", "menubar", prefs::get_bool(t, "menubar", true));
            choice(&card, "blur_on", "Menu bar backdrop", "Clear over the wallpaper, frosted when windows are open", "menubar_look", &[("adaptive", "Adaptive"), ("clear", "Clear"), ("glass", "Glass")], &prefs::get_str(t, "menubar_look", "adaptive"));
            choice(&card, "call", "Call activity", "A live pill with the call timer while an app uses the microphone", "notch_call", &[("voice", "Voice and video apps"), ("any", "Any app using the microphone"), ("off", "Off")], &prefs::get_str(t, "notch_call", "voice"));
            page.append(&card);
        }
        2 => {
            page.append(&header("palette", "Look and feel", "Surfaces, accent color and how round things are. Settings, Store and Files follow along."));
            let card = ui::card();
            surfaces(&card, "layers", "App surfaces", "", &prefs::get_str(t, "ui_surface", "black"));
            accent(&card, t);
            corners(&card, "rounded_corner", "Corners", "", "ui_round", prefs::get_f64(t, "ui_round", 1.0));
            choice(&card, "dark_mode", "Notch and dock", "", "surface", &[("black", "Pure black"), ("tinted", "Tinted")], &prefs::get_str(t, "surface", "black"));
            page.append(&card);
        }
        3 => {
            page.append(&header("wallpaper", "Wallpaper and colors", "Colors are generated from your wallpaper. Pick one you love and Digit repaints itself around it."));
            let card = ui::card();
            let (row, end) = ui::row(Some("image"), "Wallpaper", &match kit::keybinds::shown("wallpaper") {
                Some(k) => format!("Opens the wallpaper picker on this screen ({k})"),
                None => "Opens the wallpaper picker on this screen".into(),
            });
            let b = ui::button("Choose…", "accent");
            b.connect_clicked(|_| spawn(&["digitctl", "key", "wallpaper"]));
            end.append(&b);
            card.append(&row);
            let (row, end) = ui::row(Some("palette"), "Color scheme", "How colors are derived from the wallpaper");
            let current = kit::theme::load().map(|t| t.scheme).unwrap_or_else(|| prefs::get_str(t, "scheme", "content"));
            let schemes: Vec<&str> = kit::theme::SCHEMES.to_vec();
            let pick = kit::ui::dropdown(&schemes, schemes.iter().position(|s| *s == current).unwrap_or(0) as u32);
            pick.connect_selected_notify(|p| {
                if let Some(name) = kit::theme::SCHEMES.get(p.selected() as usize) {
                    spawn(&["digit", "theme", "--scheme", name]);
                }
            });
            end.append(&pick);
            card.append(&row);
            page.append(&card);
        }
        4 => {
            page.append(&header("dock_to_bottom", "Your dock", "Pinned and running apps. Right-click an icon for windows, Keep in Dock and Quit."));
            let card = ui::card();
            choice(&card, "side_navigation", "Position", "", "dock_side", &[("bottom", "Bottom"), ("left", "Left"), ("right", "Right")], &prefs::get_str(t, "dock_side", "bottom"));
            choice(&card, "visibility", "Visibility", "Auto-hide reveals it when the pointer touches the edge", "dock_mode", &[("on", "Always shown"), ("autohide", "Auto-hide"), ("smart", "Hide when windows are open"), ("off", "Off")], &prefs::get_str(t, "dock_mode", "on"));
            choice(&card, "style", "Icon style", "", "dock_icons", &[("light", "Big Sur"), ("graphite", "Big Sur graphite"), ("mono", "Mono"), ("tinted", "Tinted"), ("native", "Original")], &prefs::get_str(t, "dock_icons", "light"));
            slider(&card, "zoom_in", "Magnification", "1.0 turns it off", "dock_magnify", (1.0, 2.0, 0.05), prefs::get_f64(t, "dock_magnify", 1.5), 1.5, 2);
            crate::desktop::toggle(&card, "fullscreen", "Edges over fullscreen apps", "Let the dock, workspaces and Control Center open from screen edges while a game or video is fullscreen", "fullscreen_edges", prefs::get_bool(t, "fullscreen_edges", false));
            page.append(&card);
        }
        5 => {
            page.append(&header("nightlight", "Comfort and safety", "Gentle on your eyes in the evening, locked when you step away."));
            let card = ui::card();
            choice(&card, "nightlight", "Night Light", "Warmer colors in the evening", "night_light", &[("off", "Off"), ("sunset", "Sunset to sunrise"), ("schedule", "20:00 to 07:00"), ("always", "Always on")], &prefs::get_str(t, "night_light", "off"));
            slider(&card, "lock_clock", "Lock after idle", "Minutes. 0 never locks on its own", "idle_lock", (0.0, 60.0, 1.0), prefs::get_f64(t, "idle_lock", 0.0), 0.0, 0);
            slider(&card, "monitor", "Turn off displays", "Minutes without input", "idle_off", (0.0, 60.0, 1.0), prefs::get_f64(t, "idle_off", 10.0), 10.0, 0);
            crate::desktop::toggle(&card, "lock", "Lock before sleep", "Waking up asks for your password", "lock_on_sleep", prefs::get_bool(t, "lock_on_sleep", true));
            choice(&card, "schedule", "Desktop clock", "", "desktop_widget", &[("off", "Off"), ("left", "Top left"), ("center", "Center")], &prefs::get_str(t, "desktop_widget", "off"));
            page.append(&card);
        }
        _ => {
            page.append(&header("keyboard_command_key", "You're all set", "A few shortcuts worth remembering. Settings lives in Spotlight and behind the gear in the notch."));
            page.append(&shortcuts());
        }
    }
    page.upcast()
}

thread_local! {
    static OPEN: std::cell::RefCell<Option<gtk::ApplicationWindow>> = const { std::cell::RefCell::new(None) };
}

pub fn open(app: &gtk::Application) {
    if let Some(w) = OPEN.with(|o| o.borrow().clone()) {
        w.present();
        return;
    }
    let t = prefs::read();
    let window = gtk::ApplicationWindow::new(app);
    kit::scroll::smooth(&window);
    window.set_title(Some("Welcome to Digit"));
    window.add_css_class("digit-dialog");
    window.add_css_class("welcome");
    // A full-screen overlay rather than a window: it stays on top and only Skip or Get started closes it.
    window.init_layer_shell();
    window.set_namespace(Some("digit-welcome"));
    window.set_layer(Layer::Overlay);
    window.set_keyboard_mode(KeyboardMode::Exclusive);
    for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(edge, true);
    }
    window.set_exclusive_zone(-1);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.set_size_request(820, 640);
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    top.set_margin_top(14);
    top.set_margin_start(18);
    top.set_margin_end(14);
    let dots = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    dots.set_hexpand(true);
    dots.set_valign(gtk::Align::Center);
    for _ in 0..STEPS {
        let d = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        d.add_css_class("step-dot");
        dots.append(&d);
    }
    top.append(&dots);
    let skip = ui::button("Skip", "");
    top.append(&skip);
    root.append(&top);
    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
    stack.set_transition_duration(280);
    stack.set_vexpand(true);
    for i in 0..STEPS {
        stack.add_named(&step(i, &t), Some(&i.to_string()));
    }
    root.append(&stack);
    let bottom = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    bottom.set_margin_start(24);
    bottom.set_margin_end(24);
    bottom.set_margin_bottom(20);
    bottom.set_margin_top(10);
    let back = ui::button("Back", "");
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    let next = ui::button("Next", "accent");
    bottom.append(&back);
    bottom.append(&spacer);
    bottom.append(&next);
    root.append(&bottom);
    let (layer, backdrop) = kit::ui::overlay_card(&root);
    window.set_child(Some(&layer));
    let at = Rc::new(Cell::new(0usize));
    let sync = {
        let (stack, dots, back, next, at) = (stack.clone(), dots.clone(), back.clone(), next.clone(), at.clone());
        move || {
            let i = at.get();
            stack.set_visible_child_name(&i.to_string());
            let mut d = dots.first_child();
            let mut k = 0;
            while let Some(w) = d {
                if k == i {
                    w.add_css_class("on");
                } else {
                    w.remove_css_class("on");
                }
                d = w.next_sibling();
                k += 1;
            }
            back.set_sensitive(i > 0);
            next.set_label(if i + 1 == STEPS { "Get started" } else { "Next" });
        }
    };
    sync();
    let sync = Rc::new(sync);
    {
        let (at, sync) = (at.clone(), sync.clone());
        back.connect_clicked(move |_| {
            if at.get() > 0 {
                at.set(at.get() - 1);
                sync();
            }
        });
    }
    {
        let (at, sync, window) = (at.clone(), sync.clone(), window.clone());
        next.connect_clicked(move |_| {
            if at.get() + 1 >= STEPS {
                finish(window.upcast_ref());
            } else {
                at.set(at.get() + 1);
                sync();
            }
        });
    }
    {
        let window = window.clone();
        skip.connect_clicked(move |_| finish(window.upcast_ref()));
    }
    let keys = gtk::EventControllerKey::new();
    {
        let (at, sync) = (at.clone(), sync.clone());
        keys.connect_key_pressed(move |_, key, _, _| {
            match key {
                gtk::gdk::Key::Right if at.get() + 1 < STEPS => {
                    at.set(at.get() + 1);
                    sync();
                }
                gtk::gdk::Key::Left if at.get() > 0 => {
                    at.set(at.get() - 1);
                    sync();
                }
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
    }
    window.add_controller(keys);
    let closing = Rc::new(Cell::new(false));
    {
        let (backdrop, root) = (backdrop.clone(), root.clone());
        window.connect_close_request(move |w| {
            if closing.replace(true) {
                OPEN.with(|o| o.borrow_mut().take());
                return glib::Propagation::Proceed;
            }
            let w = w.clone();
            kit::ui::settle(&backdrop, &root, 0, false, move || w.close());
            glib::Propagation::Stop
        });
    }
    OPEN.with(|o| *o.borrow_mut() = Some(window.clone()));
    backdrop.set_opacity(0.0);
    window.present();
    kit::ui::settle(&backdrop, &root, 0, true, || {});
}
