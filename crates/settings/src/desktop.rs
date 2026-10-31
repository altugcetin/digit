use crate::{prefs, ui};
use gtk::{gio, glib, prelude::*};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

pub(crate) fn save(key: &str, value: toml::Value) {
    if let Err(e) = prefs::set(key, value) {
        ui::fail(&format!("Could not save: {e}"));
    }
}

pub(crate) fn accent(card: &gtk::Box, t: &toml::Table) {
    let (row, end) = ui::row(Some("colors"), "Accent", "Buttons, switches and highlights");
    let swatches = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    swatches.set_valign(gtk::Align::Center);
    let current = prefs::get_str(&t, "ui_accent", "wallpaper");
    let mut first: Option<gtk::ToggleButton> = None;
    for (value, tip) in kit::style::ACCENTS {
        let b = gtk::ToggleButton::new();
        b.add_css_class("swatch");
        b.add_css_class(&format!("sw-{}", value.trim_start_matches('#')));
        b.set_tooltip_text(Some(tip));
        if value == "wallpaper" {
            b.set_child(Some(&kit::ui::sym("wallpaper")));
        }
        match &first {
            Some(f) => b.set_group(Some(f)),
            None => first = Some(b.clone()),
        }
        b.set_active(current.eq_ignore_ascii_case(value));
        b.set_focus_on_click(false);
        b.connect_toggled(move |b| {
            if b.is_active() {
                save("ui_accent", toml::Value::String(value.to_string()));
            }
        });
        swatches.append(&b);
    }
    end.append(&swatches);
    card.append(&row);
}

pub(crate) fn choice(card: &gtk::Box, icon: &str, title: &str, sub: &str, key: &'static str, options: &[(&'static str, &'static str)], current: &str) {
    let (row, end) = ui::row(Some(icon), title, sub);
    let labels: Vec<&str> = options.iter().map(|o| o.1).collect();
    let at = options.iter().position(|o| o.0 == current).unwrap_or(0);
    let values: Vec<&'static str> = options.iter().map(|o| o.0).collect();
    let short = options.len() <= 3 && labels.iter().map(|l| l.chars().count()).sum::<usize>() <= 26;
    if short {
        end.append(&kit::ui::segmented(&labels, at, move |i| {
            if let Some(v) = values.get(i) {
                save(key, toml::Value::String(v.to_string()));
            }
        }));
    } else {
        let pick = kit::ui::dropdown(&labels, at as u32);
        pick.connect_selected_notify(move |p| {
            if let Some(v) = values.get(p.selected() as usize) {
                save(key, toml::Value::String(v.to_string()));
            }
        });
        end.append(&pick);
    }
    card.append(&row);
}

/// Preset corner radii, as multiples of the standard roundness.
const CORNERS: [(f64, &str); 4] = [(0.15, "Square"), (0.55, "Subtle"), (1.0, "Rounded"), (1.6, "Round")];

/// A row followed by a strip of picture tiles, one per option; returns the whole block.
fn tiles(card: &gtk::Box, icon: &str, title: &str, sub: &str, items: Vec<(&str, gtk::Widget)>, at: usize, pick: impl Fn(usize) + 'static) -> gtk::Box {
    let block = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let (row, _) = ui::row(Some(icon), title, sub);
    block.append(&row);
    let strip = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    strip.add_css_class("corners");
    strip.set_homogeneous(true);
    strip.set_halign(gtk::Align::Start);
    let pick = Rc::new(pick);
    let mut first: Option<gtk::ToggleButton> = None;
    for (i, (name, picture)) in items.into_iter().enumerate() {
        let b = gtk::ToggleButton::new();
        b.add_css_class("corner");
        let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
        picture.add_css_class("corner-tile");
        picture.set_size_request(136, 72);
        picture.set_overflow(gtk::Overflow::Hidden);
        col.append(&picture);
        let label = gtk::Label::new(Some(name));
        label.add_css_class("corner-name");
        col.append(&label);
        b.set_child(Some(&col));
        match &first {
            Some(f) => b.set_group(Some(f)),
            None => first = Some(b.clone()),
        }
        b.set_active(i == at);
        let pick = pick.clone();
        b.connect_toggled(move |b| {
            if b.is_active() {
                pick(i);
            }
        });
        strip.append(&b);
    }
    block.append(&strip);
    card.append(&block);
    block
}

/// Corner presets, each pictured as the top-left corner of a window.
pub(crate) fn corners(card: &gtk::Box, icon: &str, title: &str, sub: &str, key: &'static str, value: f64) -> gtk::Box {
    let at = CORNERS.iter().enumerate().min_by(|a, b| (a.1.0 - value).abs().total_cmp(&(b.1.0 - value).abs())).map_or(2, |(i, _)| i);
    let items = CORNERS
        .into_iter()
        .map(|(r, name)| {
            let area = gtk::DrawingArea::new();
            area.set_draw_func(move |a, cr, w, h| {
                let (w, h) = (w as f64, h as f64);
                let (x0, y0) = (w * 0.26, h * 0.32);
                let rad = (22.0 * r).min(h - y0 - 4.0);
                let ink = a.color();
                let edge = || {
                    cr.move_to(x0, h);
                    cr.line_to(x0, y0 + rad);
                    cr.arc(x0 + rad, y0 + rad, rad, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
                    cr.line_to(w, y0);
                };
                edge();
                cr.line_to(w, h);
                cr.close_path();
                cr.set_source_rgba(ink.red() as f64, ink.green() as f64, ink.blue() as f64, 0.06);
                let _ = cr.fill();
                edge();
                cr.set_source_rgba(ink.red() as f64, ink.green() as f64, ink.blue() as f64, 0.7);
                cr.set_line_width(2.0);
                let _ = cr.stroke();
            });
            (name, area.upcast())
        })
        .collect();
    tiles(card, icon, title, sub, items, at, move |i| save(key, toml::Value::Float(CORNERS[i].0)))
}

/// App surfaces, each pictured as the corner of a window in that surface with a card inside.
pub(crate) fn surfaces(card: &gtk::Box, icon: &str, title: &str, sub: &str, current: &str) -> gtk::Box {
    const SURFACES: [(&str, &str); 3] = [("black", "Black"), ("graphite", "Graphite"), ("tinted", "Tinted")];
    let at = SURFACES.iter().position(|s| s.0 == current).unwrap_or(0);
    let items = SURFACES
        .into_iter()
        .map(|(value, name)| {
            let tile = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            let win = gtk::Box::new(gtk::Orientation::Vertical, 5);
            win.add_css_class("surf-win");
            win.add_css_class(&format!("surf-{value}"));
            win.set_hexpand(true);
            // Same spot as the corner previews: a quarter in from the left, a third down.
            win.set_margin_start(35);
            win.set_margin_top(23);
            let inner = gtk::Box::new(gtk::Orientation::Vertical, 5);
            inner.add_css_class("surf-card");
            inner.set_vexpand(true);
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            line.add_css_class("surf-line");
            line.set_size_request(44, 5);
            line.set_halign(gtk::Align::Start);
            let pill = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            pill.add_css_class("surf-pill");
            pill.set_size_request(22, 5);
            pill.set_halign(gtk::Align::Start);
            inner.append(&line);
            inner.append(&pill);
            win.append(&inner);
            tile.append(&win);
            (name, tile.upcast())
        })
        .collect();
    tiles(card, icon, title, sub, items, at, move |i| save("ui_surface", toml::Value::String(SURFACES[i].0.to_string())))
}

fn find_switch(w: &gtk::Widget) -> Option<gtk::Switch> {
    if let Some(s) = w.downcast_ref::<gtk::Switch>() {
        return Some(s.clone());
    }
    let mut child = w.first_child();
    while let Some(c) = child {
        if let Some(s) = find_switch(&c) {
            return Some(s);
        }
        child = c.next_sibling();
    }
    None
}

pub(crate) fn toggle(card: &gtk::Box, icon: &str, title: &str, sub: &str, key: &'static str, on: bool) {
    let (row, end) = ui::row(Some(icon), title, sub);
    let sw = gtk::Switch::new();
    sw.set_active(on);
    sw.set_valign(gtk::Align::Center);
    sw.connect_active_notify(move |s| save(key, toml::Value::Boolean(s.is_active())));
    end.append(&sw);
    card.append(&row);
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn slider(card: &gtk::Box, icon: &str, title: &str, sub: &str, key: &'static str, range: (f64, f64, f64), value: f64, digits: i32) {
    let (row, end) = ui::row(Some(icon), title, sub);
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, range.0, range.1, range.2);
    scale.set_value(value);
    scale.set_digits(digits);
    scale.set_draw_value(true);
    scale.set_value_pos(gtk::PositionType::Left);
    scale.set_size_request(240, -1);
    let generation = Rc::new(Cell::new(0u64));
    scale.connect_value_changed(move |s| {
        let g = generation.get() + 1;
        generation.set(g);
        let generation = generation.clone();
        let v = (s.value() * 100.0).round() / 100.0;
        glib::timeout_add_local_once(Duration::from_millis(220), move || {
            if generation.get() == g {
                save(key, toml::Value::Float(v));
            }
        });
    });
    let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    wheel.set_propagation_phase(gtk::PropagationPhase::Capture);
    let page = scale.clone();
    wheel.connect_scroll(move |_, _, dy| {
        if let Some(sw) = page.ancestor(gtk::ScrolledWindow::static_type()).and_then(|w| w.downcast::<gtk::ScrolledWindow>().ok()) {
            let adj = sw.vadjustment();
            adj.set_value(adj.value() + dy * 48.0);
        }
        glib::Propagation::Stop
    });
    scale.add_controller(wheel);
    end.append(&scale);
    card.append(&row);
}

pub(crate) fn field(card: &gtk::Box, icon: &str, title: &str, sub: &str, key: &'static str, current: &str, hint: &str, check: fn(&str) -> bool) {
    let (row, end) = ui::row(Some(icon), title, sub);
    let entry = gtk::Entry::new();
    entry.set_text(current);
    entry.set_placeholder_text(Some(hint));
    entry.set_valign(gtk::Align::Center);
    entry.set_width_chars(14);
    entry.set_max_length(64);
    let generation = Rc::new(Cell::new(0u64));
    entry.connect_changed(move |e| {
        let g = generation.get() + 1;
        generation.set(g);
        let generation = generation.clone();
        let text = e.text().trim().to_string();
        let ok = check(&text);
        if ok { e.remove_css_class("error") } else { e.add_css_class("error") }
        glib::timeout_add_local_once(Duration::from_millis(700), move || {
            if generation.get() == g && ok {
                save(key, toml::Value::String(text));
            }
        });
    });
    end.append(&entry);
    card.append(&row);
}

fn hhmm(s: &str) -> bool {
    s.split_once(':').is_some_and(|(h, m)| h.len() <= 2 && m.len() == 2 && h.parse::<u32>().is_ok_and(|h| h < 24) && m.parse::<u32>().is_ok_and(|m| m < 60))
}

fn icon_themes() -> Vec<String> {
    let mut dirs = vec![std::path::PathBuf::from("/usr/share/icons"), glib::user_data_dir().join("icons"), glib::home_dir().join(".icons")];
    dirs.dedup();
    let mut out: Vec<String> = Vec::new();
    for d in dirs {
        let Ok(entries) = std::fs::read_dir(d) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            let index = std::fs::read_to_string(p.join("index.theme")).unwrap_or_default();
            let only_cursor = p.join("cursors").is_dir() && !index.contains("Directories=");
            let name = e.file_name().to_string_lossy().to_string();
            if !index.is_empty() && !only_cursor && name != "default" && name != "hicolor" && !out.contains(&name) {
                out.push(name);
            }
        }
    }
    out.sort_by_key(|n| n.to_lowercase());
    out
}

fn niri_dir() -> std::path::PathBuf {
    glib::user_config_dir().join("niri")
}

fn free_windows() -> bool {
    std::fs::read_to_string(niri_dir().join("digit-windows.kdl")).is_ok_and(|t| t.contains("open-floating true"))
}

fn set_windows(free: bool) {
    let include = "include optional=true \"digit-windows.kdl\"";
    let digit = niri_dir().join("digit.kdl");
    let current = std::fs::read_to_string(&digit).unwrap_or_default();
    if current.is_empty() {
        ui::fail("digit's niri file is missing, so the window mode can't be applied");
        return;
    }
    if !current.contains(include) {
        let tmp = digit.with_extension("kdl.new");
        if std::fs::write(&tmp, format!("{}\n{include}\n", current.trim_end())).and_then(|_| std::fs::rename(&tmp, &digit)).is_err() {
            ui::fail("Could not update digit's niri file");
            return;
        }
    }
    let file = niri_dir().join("digit-windows.kdl");
    let body = if free {
        "window-rule {\n    open-floating true\n}\n\nbinds {\n    Super+Up repeat=false { spawn \"digitctl\" \"key\" \"snap-max\"; }\n    Super+M repeat=false { spawn \"digitctl\" \"key\" \"snap-max\"; }\n    Super+Down repeat=false { spawn \"digitctl\" \"key\" \"snap-restore\"; }\n    Super+Left repeat=false { spawn \"digitctl\" \"key\" \"snap-left\"; }\n    Super+Right repeat=false { spawn \"digitctl\" \"key\" \"snap-right\"; }\n}\n"
    } else {
        ""
    };
    let tmp = file.with_extension("kdl.new");
    if std::fs::write(&tmp, body).and_then(|_| std::fs::rename(&tmp, &file)).is_err() {
        ui::fail("Could not write the window mode");
        return;
    }
    glib::spawn_future_local(async move {
        glib::timeout_future(Duration::from_millis(400)).await;
        let _ = gio::spawn_blocking(move || crate::niri::retile(free)).await;
        ui::done(if free { "Windows now float freely" } else { "Windows are tiled in columns again" });
    });
}

pub fn build() -> gtk::Widget {
    let (scroll, body) = ui::page("Desktop", "digit applies these instantly.");
    let t = prefs::read();
    body.append(&ui::heading("Appearance"));
    let look = ui::card();
    choice(&look, "dark_mode", "Surfaces", "Notch, dock, menus and panels", "surface", &[("black", "Pure black"), ("tinted", "Tinted by wallpaper")], &prefs::get_str(&t, "surface", "black"));
    let (row, end) = ui::row(Some("palette"), "Color scheme", "Colors generated from your wallpaper");
    let current = kit::theme::load().map(|t| t.scheme).unwrap_or_else(|| prefs::get_str(&t, "scheme", "content"));
    let schemes: Vec<&str> = kit::theme::SCHEMES.to_vec();
    let pick = kit::ui::dropdown(&schemes, 0);
    pick.set_selected(schemes.iter().position(|s| *s == current).unwrap_or(0) as u32);
    pick.connect_selected_notify(move |p| {
        let Some(name) = kit::theme::SCHEMES.get(p.selected() as usize) else { return };
        let argv: [&std::ffi::OsStr; 4] = ["digit".as_ref(), "theme".as_ref(), "--scheme".as_ref(), name.as_ref()];
        match gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE) {
            Ok(_) => ui::done(&format!("Color scheme: {name}")),
            Err(e) => ui::fail(&e.to_string()),
        }
    });
    end.append(&pick);
    look.append(&row);
    let themes = icon_themes();
    let (row, end) = ui::row(Some("interests"), "Icon theme", "Used by the dock, switcher and launcher");
    let mut labels: Vec<String> = vec!["System default".into()];
    labels.extend(themes.iter().cloned());
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let pick = kit::ui::dropdown(&refs, 0);
    let chosen = t.get("icon_theme").and_then(|v| v.as_str()).map(str::to_string);
    pick.set_selected(chosen.as_ref().and_then(|c| themes.iter().position(|n| n == c)).map_or(0, |i| i + 1) as u32);
    pick.connect_selected_notify(move |p| {
        let i = p.selected() as usize;
        let r = if i == 0 { prefs::unset("icon_theme") } else { themes.get(i - 1).map_or(Ok(()), |n| prefs::set("icon_theme", toml::Value::String(n.clone()))) };
        if let Err(e) = r {
            ui::fail(&e);
        }
    });
    end.append(&pick);
    look.append(&row);
    toggle(&look, "auto_awesome", "Startup animation", "Greet each start of the shell with a short full-screen animation", "intro", prefs::get_bool(&t, "intro", true));
    body.append(&look);

    body.append(&ui::heading("Apps"));
    let apps = ui::card();
    surfaces(&apps, "layers", "Surfaces", "Background of Settings, Store, Files and the Game Bar", &prefs::get_str(&t, "ui_surface", "black"));
    accent(&apps, &t);
    corners(&apps, "rounded_corner", "Corners", "Cards, menus, buttons and dialogs", "ui_round", prefs::get_f64(&t, "ui_round", 1.0));
    choice(&apps, "density_medium", "Density", "", "ui_density", &[("comfortable", "Comfortable"), ("compact", "Compact")], &prefs::get_str(&t, "ui_density", "comfortable"));
    toggle(&apps, "swipe_vertical", "Smooth scrolling", "Mouse wheel steps glide instead of jumping. Touchpads always scroll smoothly", "smooth_scroll", prefs::get_bool(&t, "smooth_scroll", true));
    toggle(&apps, "select_window", "Open Settings as a window", "Settings opens over everything by default; turn this on for an ordinary window. Takes effect the next time Settings opens", "settings_window", prefs::get_bool(&t, "settings_window", false));
    body.append(&apps);

    body.append(&ui::heading("Dock"));
    let dock = ui::card();
    choice(&dock, "dock_to_bottom", "Visibility", "Alt+D shows or hides it at any time", "dock_mode", &[("on", "Always shown"), ("autohide", "Auto-hide"), ("smart", "Hide when windows are open"), ("off", "Off")], &prefs::get_str(&t, "dock_mode", "on"));
    choice(&dock, "side_navigation", "Position", "", "dock_side", &[("bottom", "Bottom"), ("left", "Left"), ("right", "Right")], &prefs::get_str(&t, "dock_side", "bottom"));
    slider(&dock, "photo_size_select_small", "Icon size", "", "dock_size", (32.0, 80.0, 2.0), prefs::get_f64(&t, "dock_size", 50.0), 0);
    slider(&dock, "zoom_in", "Magnification", "1.0 turns it off", "dock_magnify", (1.0, 2.0, 0.05), prefs::get_f64(&t, "dock_magnify", 1.5), 2);
    toggle(&dock, "apps", "Show running apps", "Apps that are open but not kept in the dock", "dock_running", prefs::get_bool(&t, "dock_running", true));
    toggle(&dock, "notifications", "Notification badges", "Unread notification counts on icons", "dock_badges", prefs::get_bool(&t, "dock_badges", true));
    let style = match prefs::get_str(&t, "dock_icons", "light").as_str() {
        "tile" => "graphite".to_string(),
        other => other.to_string(),
    };
    choice(&dock, "style", "Icon style", "Big Sur shaped icons for every app", "dock_icons", &[("light", "Big Sur"), ("graphite", "Big Sur graphite"), ("mono", "Mono"), ("tinted", "Tinted"), ("native", "Original")], &style);
    body.append(&dock);

    body.append(&ui::heading("Windows"));
    let win = ui::card();
    let wm = kit::wm::which();
    if wm == kit::wm::Wm::Hyprland {
        let (file, text, include) = (glib::user_config_dir().join("hypr").join("digit.conf"), include_str!("../../../data/hypr/digit.conf"), Some((glib::user_config_dir().join("hypr").join("hyprland.conf"), "source = ~/.config/hypr/digit.conf")));
        let installed = file.exists();
        let (row, end) = ui::row(Some("keyboard"), &format!("{} shortcuts", kit::wm::name()), &format!("Writes {} with digit's keys and starts digit with the session", file.display()));
        let b = ui::button(if installed { "Reinstall" } else { "Install" }, "accent");
        b.connect_clicked(move |b| {
            let parent = ui::root(b);
            let (file, text, include) = (file.clone(), text, include.clone());
            let detail = match &include {
                Some((conf, line)) => format!("digit will write {} and add “{line}” to {} if it isn't there yet.", file.display(), conf.display()),
                None => format!("digit will write {}.", file.display()),
            };
            kit::ui::ask(parent.as_ref(), "Install shortcuts?", &detail, "Cancel", "Install", "accent", move || {
                let write = |path: &std::path::Path, body: &str| -> std::io::Result<()> {
                    if let Some(dir) = path.parent() {
                        std::fs::create_dir_all(dir)?;
                    }
                    let tmp = path.with_extension("digit-new");
                    std::fs::write(&tmp, body)?;
                    std::fs::rename(&tmp, path)
                };
                if let Err(e) = write(&file, text) {
                    ui::fail(&format!("Could not write the shortcuts: {e}"));
                    return;
                }
                if let Some((conf, line)) = include {
                    let current = std::fs::read_to_string(&conf).unwrap_or_default();
                    if !current.lines().any(|l| l.trim() == line) {
                        let backup = conf.with_extension("conf.before-digit");
                        if conf.exists() && !backup.exists() {
                            let _ = std::fs::copy(&conf, &backup);
                        }
                        if let Err(e) = write(&conf, &format!("{}\n\n{line}\n", current.trim_end())) {
                            ui::fail(&format!("Could not update {}: {e}", conf.display()));
                            return;
                        }
                    }
                }
                ui::done("Shortcuts installed; they apply when the compositor reloads its config");
            });
        });
        end.append(&b);
        win.append(&row);
        body.append(&win);
    } else {
        let (row, end) = ui::row(Some("select_window"), "Window placement", "Free: windows open side by side; drag by the title bar or Super + drag, resize with Super + right drag. Super+Up maximizes, Super+Left/Right fills a half, Super+Down restores");
        let modes = ["Tiling columns", "Free placement"];
        let pick = kit::ui::dropdown(&modes, 0);
        pick.set_selected(if free_windows() { 1 } else { 0 });
        pick.connect_selected_notify(|p| set_windows(p.selected() == 1));
        end.append(&pick);
        win.append(&row);
        let (row, end) = ui::row(Some("dashboard"), "Window layouts", "Super+Z arranges the focused window: halves, thirds, corners and more");
        end.append(&ui::badge("Super Z", ""));
        win.append(&row);
        body.append(&win);
    }

    body.append(&ui::heading("Notch and corners"));
    let notch = ui::card();
    choice(&notch, "toolbar", "Style", "Notch hugs the top edge, Dynamic Island floats as a pill, Bar spans the whole edge with the app, status and date", "notch_style", &[("island", "Notch"), ("dynamic", "Dynamic Island"), ("bar", "Bar")], &prefs::get_str(&t, "notch_style", "island"));
    toggle(&notch, "menu", "Menu bar", "Distro menu, the focused app's menus, status and date beside the notch", "menubar", prefs::get_bool(&t, "menubar", true));
    choice(&notch, "blur_on", "Menu bar backdrop", "Adaptive stays clear over the wallpaper and turns to frosted glass when windows are open", "menubar_look", &[("adaptive", "Adaptive"), ("clear", "Clear"), ("glass", "Glass")], &prefs::get_str(&t, "menubar_look", "adaptive"));
    slider(&notch, "height", "Bar height", "Used by the full-width bar", "wide_height", (28.0, 56.0, 1.0), prefs::get_f64(&t, "wide_height", 34.0), 0);
    choice(&notch, "call", "Call activity", "A live pill with the call timer and voice waveform while an app uses the microphone", "notch_call", &[("voice", "Voice and video apps"), ("any", "Any app using the microphone"), ("off", "Off")], &prefs::get_str(&t, "notch_call", "voice"));
    toggle(&notch, "mic_off", "Block idle microphone", "Voice apps like Discord can keep listening after a call ends. Digit silences only their microphone stream until a real call starts, then gives it back instantly", "mic_guard", prefs::get_bool(&t, "mic_guard", true));
        toggle(&notch, "touch_app", "Open on hover", "Expand the notch when the pointer rests on it", "hover", prefs::get_bool(&t, "hover", true));
    toggle(&notch, "rounded_corner", "Match app roundness", "The notch's corners follow Corners under Apps", "notch_round_follow", prefs::get_bool(&t, "notch_round_follow", true));
    corners(&notch, "rounded_corner", "Notch corners", "Used when the notch doesn't match the apps", "notch_round", prefs::get_f64(&t, "notch_round", 1.0));
    // The notch's own roundness only applies while it doesn't follow the apps.
    if let (Some(own), Some(follow)) = (notch.last_child(), notch.last_child().and_then(|w| w.prev_sibling()).and_then(|row| find_switch(&row))) {
        own.set_sensitive(!follow.is_active());
        follow.connect_active_notify(move |s| own.set_sensitive(!s.is_active()));
    }
    toggle(&notch, "north_west", "Workspace corner", "Move the pointer to the top-left corner to see workspaces", "hot_corner", prefs::get_bool(&t, "hot_corner", true));
    toggle(&notch, "south_east", "Control Center corner", "Move the pointer to the bottom-right corner to open Control Center", "control_corner", prefs::get_bool(&t, "control_corner", true));
    toggle(&notch, "fullscreen", "Over fullscreen apps", "Let the dock edge and screen corners open panels while an app or game is fullscreen. Spotlight always opens", "fullscreen_edges", prefs::get_bool(&t, "fullscreen_edges", false));
    body.append(&notch);

    body.append(&ui::heading("Panels"));
    let panels = ui::card();
    let close = [("leave", "When the pointer leaves"), ("click", "On a click elsewhere")];
    choice(&panels, "toolbar", "Notch", "When the open notch folds away", "notch_close", &close, &prefs::get_str(&t, "notch_close", "leave"));
    choice(&panels, "grid_view", "Workspaces", "When the workspaces panel in the top-left corner closes", "spaces_close", &close, &prefs::get_str(&t, "spaces_close", "leave"));
    choice(&panels, "tune", "Control Center", "When Control Center in the bottom-right corner closes", "control_close", &close, &prefs::get_str(&t, "control_close", "leave"));
    choice(&panels, "calendar_month", "Calendar", "When the calendar under the date closes", "calendar_close", &close, &prefs::get_str(&t, "calendar_close", "leave"));
    body.append(&panels);

    body.append(&ui::heading("Lock and power"));
    let power = ui::card();
    slider(&power, "lock_clock", "Lock after idle", "Minutes without input before the screen locks. 0 never locks on its own", "idle_lock", (0.0, 60.0, 1.0), prefs::get_f64(&t, "idle_lock", 0.0), 0);
    slider(&power, "monitor", "Turn off displays", "Minutes without input. The screen dims softly for a few seconds first; any input cancels", "idle_off", (0.0, 60.0, 1.0), prefs::get_f64(&t, "idle_off", 10.0), 0);
    slider(&power, "bedtime", "Sleep after idle", "Minutes without input before the computer sleeps. 0 never sleeps on its own", "idle_sleep", (0.0, 180.0, 5.0), prefs::get_f64(&t, "idle_sleep", 0.0), 0);
    toggle(&power, "lock", "Lock before sleep", "The lock screen is ready before the computer suspends, so waking needs your password", "lock_on_sleep", prefs::get_bool(&t, "lock_on_sleep", true));
    let (row, end) = ui::row(Some("preview"), "Lock screen", "Super+L locks, Super+Shift+E opens Lock, Sleep, Restart, Shut Down and Log Out. Passwords are checked by PAM in a separate process");
    let look = gtk::Button::with_label("Preview");
    look.set_valign(gtk::Align::Center);
    look.connect_clicked(|_| {
        let _ = gio::Subprocess::newv(&[std::ffi::OsStr::new("digitctl"), std::ffi::OsStr::new("msg"), std::ffi::OsStr::new("lock"), std::ffi::OsStr::new("preview")], gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE);
    });
    end.append(&look);
    power.append(&row);
    body.append(&power);

    body.append(&ui::heading("Night Light"));
    let night = ui::card();
    choice(&night, "nightlight", "Night Light", "Warms the screen in the evening to be easier on your eyes. Sunset uses the weather city below", "night_light", &[("off", "Off"), ("sunset", "Sunset to sunrise"), ("schedule", "Custom hours"), ("always", "Always on")], &prefs::get_str(&t, "night_light", "off"));
    slider(&night, "thermostat", "Warmth", "Color temperature at night, in kelvin. Lower is warmer", "night_temp", (2500.0, 6000.0, 100.0), prefs::get_f64(&t, "night_temp", 4200.0), 0);
    field(&night, "schedule", "From", "Custom hours start, 24-hour HH:MM", "night_from", &prefs::get_str(&t, "night_from", "20:00"), "20:00", hhmm);
    field(&night, "wb_sunny", "Until", "Custom hours end, 24-hour HH:MM", "night_to", &prefs::get_str(&t, "night_to", "07:00"), "07:00", hhmm);
    body.append(&night);

    body.append(&ui::heading("Desktop"));
    let desk = ui::card();
    choice(&desk, "schedule", "Desktop clock", "A large clock with the date and weather on the wallpaper, behind your windows", "desktop_widget", &[("off", "Off"), ("left", "Top left"), ("center", "Center")], &prefs::get_str(&t, "desktop_widget", "off"));
    slider(&desk, "rounded_corner", "Rounded screen corners", "Softens the corners of every display. 0 turns it off", "screen_corners", (0.0, 24.0, 1.0), prefs::get_f64(&t, "screen_corners", 0.0), 0);
    toggle(&desk, "widgets", "Menu bar app icons", "Shows the icons apps like Discord or Steam put in the system tray. Applies after digit restarts", "tray", prefs::get_bool(&t, "tray", true));
    body.append(&desk);

    body.append(&ui::heading("Weather"));
    let sky = ui::card();
    field(&sky, "location_on", "City", "Shown in Control Center and on the desktop clock. The city name goes to open-meteo.com over HTTPS; leave empty to keep weather off", "weather_city", &prefs::get_str(&t, "weather_city", ""), "Istanbul", |_| true);
    choice(&sky, "device_thermostat", "Units", "", "weather_unit", &[("celsius", "°C"), ("fahrenheit", "°F")], &prefs::get_str(&t, "weather_unit", "celsius"));
    body.append(&sky);

    body.append(&ui::heading("Search"));
    let find = ui::card();
    toggle(&find, "manage_search", "Search inside apps", "Spotlight asks apps that offer results, such as Files, Firefox tabs and Characters", "search_apps", prefs::get_bool(&t, "search_apps", true));
    let listed = kit::search::listed();
    let off = kit::search::off_list(&t);
    let states: Rc<Vec<(String, Cell<bool>)>> = Rc::new(listed.iter().map(|l| (l.id.clone(), Cell::new(kit::search::enabled(l, &off)))).collect());
    let installed = gio::AppInfo::all();
    for (i, l) in listed.iter().enumerate() {
        let want = format!("{}.desktop", l.id);
        let Some(info) = installed.iter().find(|a| a.id().is_some_and(|id| id.as_str() == want)) else { continue };
        let note = if l.id.to_lowercase().contains("seahorse") { "Shows the names of saved passwords" } else if l.default_on { "" } else { "Starts a background service" };
        let (row, end) = ui::row(None, &info.name(), note);
        let icon = gtk::Image::new();
        if let Some(g) = info.icon() {
            icon.set_from_gicon(&g);
        }
        icon.set_pixel_size(28);
        icon.set_valign(gtk::Align::Center);
        row.prepend(&icon);
        let sw = gtk::Switch::new();
        sw.set_active(states[i].1.get());
        sw.set_valign(gtk::Align::Center);
        let states = states.clone();
        sw.connect_active_notify(move |s| {
            states[i].1.set(s.is_active());
            let list: Vec<toml::Value> = states.iter().filter(|(_, on)| !on.get()).map(|(id, _)| toml::Value::String(id.clone())).collect();
            save("search_providers_off", toml::Value::Array(list));
        });
        end.append(&sw);
        find.append(&row);
    }
    let (row, end) = ui::row(Some("keyboard_command_key"), "Search shortcuts", "Type a key and a space: dc searches Discord, yt YouTube, gh GitHub, w Wikipedia, app Digit Store. Add your own as [[search_scopes]] with key, name and url ({} is the query) in config.toml");
    end.append(&ui::badge("dc …", ""));
    find.append(&row);
    body.append(&find);

    body.append(&ui::heading("Clipboard"));
    let clip = ui::card();
    toggle(&clip, "content_paste", "Clipboard history", "Alt+V lists recent copies with color conversions. Kept only in memory; password managers' copies are skipped. Applies after digit restarts", "clipboard", prefs::get_bool(&t, "clipboard", true));
    body.append(&clip);

    body.append(&ui::heading("Screenshots"));
    let snap = ui::card();
    let (row, end) = ui::row(Some("screenshot_region"), "Shortcuts", "Print: area · Ctrl+Print: screen · Alt+Print: window · Shift+Print: 3 s timer · Super+Shift+S: area");
    end.append(&ui::badge("Print", ""));
    snap.append(&row);
    choice(&snap, "keyboard_return", "Enter after selecting", "Double-click inside the selection does the same", "snap_after", &[("copy", "Copy"), ("save", "Save"), ("edit", "Mark up"), ("pin", "Pin to screen")], &prefs::get_str(&t, "snap_after", "copy"));
    toggle(&snap, "save", "Also save copies", "Keep a file every time a screenshot is copied", "snap_save_on_copy", prefs::get_bool(&t, "snap_save_on_copy", true));
    let dir = prefs::get_str(&t, "snap_dir", "");
    let shown = if dir.is_empty() { glib::user_special_dir(glib::UserDirectory::Pictures).unwrap_or_else(glib::home_dir).join("Screenshots").display().to_string() } else { dir };
    let (row, end) = ui::row(Some("folder"), "Save to", &shown);
    let pickb = ui::button("Choose…", "");
    pickb.connect_clicked(|b| {
        let d = gtk::FileDialog::new();
        d.set_title("Save Screenshots To");
        let parent = ui::root(b);
        ui::aside();
        d.select_folder(parent.as_ref(), gio::Cancellable::NONE, |r| {
            if let Ok(f) = r {
                if let Some(p) = f.path() {
                    save("snap_dir", toml::Value::String(p.display().to_string()));
                    ui::done("Screenshot folder changed");
                }
            }
        });
    });
    end.append(&pickb);
    snap.append(&row);
    choice(&snap, "image", "File format", "", "snap_format", &[("png", "PNG (lossless)"), ("jpg", "JPEG (smaller)")], &prefs::get_str(&t, "snap_format", "png"));
    choice(&snap, "gradient", "Background frame", "Padding, gradient and shadow added in the editor", "snap_frame", &[("None", "None"), ("Ocean", "Ocean"), ("Sunset", "Sunset"), ("Grape", "Grape"), ("Mint", "Mint"), ("Graphite", "Graphite"), ("Paper", "Paper")], &prefs::get_str(&t, "snap_frame", "None"));
    toggle(&snap, "zoom_in", "Magnifier", "Pixel loupe with coordinates while selecting (the color picker lives in the notch)", "snap_loupe", prefs::get_bool(&t, "snap_loupe", false));
    toggle(&snap, "arrow_selector_tool", "Include the pointer", "", "snap_cursor", prefs::get_bool(&t, "snap_cursor", false));
    toggle(&snap, "notifications", "Notify when done", "Shows the screenshot in the notch", "snap_notify", prefs::get_bool(&t, "snap_notify", true));
    body.append(&snap);

    body.append(&ui::heading("Files"));
    let files = ui::card();
    let (row, end) = ui::row(Some("folder_open"), "Open Files", "Super+E · Space previews · F3 splits · type to jump");
    let open_files = ui::button("Open", "");
    open_files.connect_clicked(|_| {
        let argv: [&std::ffi::OsStr; 1] = ["digit-files".as_ref()];
        if gio::Subprocess::newv(&argv, gio::SubprocessFlags::NONE).is_err() {
            ui::fail("digit-files isn't installed");
        }
    });
    end.append(&open_files);
    files.append(&row);
    choice(&files, "view_list", "View", "", "files_view", &[("grid", "Icons"), ("list", "List")], &prefs::get_str(&t, "files_view", "grid"));
    toggle(&files, "visibility", "Show hidden files", "Ctrl+H toggles it in the window", "files_hidden", prefs::get_bool(&t, "files_hidden", false));
    choice(&files, "photo_size_select_small", "Thumbnails", "Videos, PDFs and documents are rendered in a sandbox", "files_thumbnails", &[("local", "This computer only"), ("all", "Everywhere, also network"), ("off", "Never")], &prefs::get_str(&t, "files_thumbnails", "local"));
    let terms: Vec<&str> = ["ghostty", "kitty", "foot", "alacritty", "wezterm", "konsole", "gnome-terminal", "ptyxis", "xfce4-terminal", "xterm"].into_iter().filter(|b| glib::find_program_in_path(b).is_some()).collect();
    if !terms.is_empty() {
        let mut opts: Vec<(&'static str, &'static str)> = vec![("", "Automatic")];
        for b in terms {
            let s: &'static str = Box::leak(b.to_string().into_boxed_str());
            opts.push((s, s));
        }
        choice(&files, "terminal", "Terminal", "Used by Open Terminal Here (F4)", "files_terminal", &opts, &prefs::get_str(&t, "files_terminal", ""));
    }
    body.append(&files);

    body.append(&ui::heading("Advanced"));
    let adv = ui::card();
    let (row, end) = ui::row(Some("description"), "Configuration file", &prefs::path().display().to_string());
    let open = ui::button("Open", "");
    open.connect_clicked(|_| {
        let file = gio::File::for_path(prefs::path());
        if file.query_exists(gio::Cancellable::NONE) {
            ui::open_uri(&file.uri());
        } else {
            ui::fail("No configuration file yet; change any setting to create it");
        }
    });
    end.append(&open);
    adv.append(&row);
    body.append(&adv);
    scroll.upcast()
}
