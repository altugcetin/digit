use crate::{prefs, ui};
use gtk::{gio, glib, prelude::*};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

pub(crate) fn save(key: &str, value: toml::Value) -> bool {
    save_many(&[(key, value)])
}

pub(crate) fn save_many(values: &[(&str, toml::Value)]) -> bool {
    if let Err(e) = prefs::set_many(values) {
        ui::fail(&format!("Could not save: {e}"));
        return false;
    }
    // Compositor settings (and the accent, which colors niri's window border) go straight into
    // digit's part of the compositor config.
    if values.iter().any(|(key, _)| key.starts_with("hypr_") || key.starts_with("niri_") || matches!(*key, "ui_accent" | "border_color" | "reduced_effects")) {
        glib::spawn_future_local(async {
            match gio::spawn_blocking(kit::setup::apply).await {
                Ok(Ok(())) => {},
                Ok(Err(e)) => ui::fail(&format!("Saved, but could not apply to the compositor: {e}")),
                Err(_) => ui::fail("Saved, but could not apply to the compositor"),
            }
        });
    }
    true
}

pub(crate) fn accent(card: &gtk::Box, t: &toml::Table) {
    let current = prefs::get_str(t, "ui_accent", "wallpaper");
    // "From the wallpaper" previews the color the wallpaper gives right now.
    let wall = kit::theme::load().map(|th| th.hex("primary")).filter(|h| h.starts_with('#')).unwrap_or_else(|| "#d0bcff".into());
    let block = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let (row, _) = ui::row(Some("colors"), "Accent", "Buttons, switches and highlights");
    block.append(&row);
    let flow = gtk::FlowBox::new();
    flow.add_css_class("corners");
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_homogeneous(true);
    flow.set_max_children_per_line(9);
    flow.set_min_children_per_line(3);
    flow.set_column_spacing(8);
    flow.set_row_spacing(8);
    let mut buttons = Vec::new();
    let mut selected = None;
    for (value, name) in kit::style::ACCENTS {
        let hex = if value == "wallpaper" { wall.clone() } else { value.to_string() };
        let b = gtk::ToggleButton::new();
        b.add_css_class("corner");
        let col = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let area = gtk::DrawingArea::new();
        area.add_css_class("corner-tile");
        // Drawn on the app surface (Black, Graphite or Tinted), as the accent would sit on it.
        area.add_css_class("accent-tile");
        area.set_overflow(gtk::Overflow::Hidden);
        area.set_size_request(76, 56);
        let rgb = hex_rgb(&hex);
        area.set_draw_func(move |a, cr, w, h| accent_preview(a, cr, w as f64, h as f64, rgb));
        col.append(&area);
        let label = gtk::Label::new(Some(if value == "wallpaper" { "Wallpaper" } else { name }));
        label.add_css_class("corner-name");
        col.append(&label);
        b.set_child(Some(&col));
        b.set_tooltip_text(Some(name));
        if current.eq_ignore_ascii_case(value) { selected = Some(buttons.len()); }
        flow.insert(&b, -1);
        buttons.push(b);
    }
    ui::select_buttons(&buttons, selected, |i| save("ui_accent", toml::Value::String(kit::style::ACCENTS[i].0.into())));
    block.append(&flow);
    card.append(&block);
}

fn hex_rgb(hex: &str) -> (f64, f64, f64) {
    let h = hex.trim_start_matches('#');
    let v = |i: usize| h.get(i..i + 2).and_then(|s| u8::from_str_radix(s, 16).ok()).unwrap_or(128) as f64 / 255.0;
    (v(0), v(2), v(4))
}

/// A tiny window in the accent: a filled button, a switch that's on and a selected line.
fn accent_preview(a: &gtk::DrawingArea, cr: &gtk::cairo::Context, w: f64, h: f64, (r, g, b): (f64, f64, f64)) {
    let ink = a.color();
    let (ir, ig, ib) = (ink.red() as f64, ink.green() as f64, ink.blue() as f64);
    let pill = |x: f64, y: f64, pw: f64, ph: f64| {
        let rad = ph / 2.0;
        cr.new_sub_path();
        cr.arc(x + pw - rad, y + rad, rad, -std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2);
        cr.arc(x + rad, y + rad, rad, std::f64::consts::FRAC_PI_2, 1.5 * std::f64::consts::PI);
        cr.close_path();
    };
    // A selected row across the top.
    pill(8.0, 9.0, w - 16.0, 10.0);
    cr.set_source_rgba(r, g, b, 0.28);
    let _ = cr.fill();
    pill(12.0, 13.0, w * 0.4, 2.5);
    cr.set_source_rgba(ir, ig, ib, 0.7);
    let _ = cr.fill();
    // A button and a switch that's on.
    let y = h - 22.0;
    pill(8.0, y, w * 0.42, 13.0);
    cr.set_source_rgb(r, g, b);
    let _ = cr.fill();
    let sx = w - 8.0 - 24.0;
    pill(sx, y, 24.0, 13.0);
    cr.set_source_rgb(r, g, b);
    let _ = cr.fill();
    cr.arc(sx + 24.0 - 6.5, y + 6.5, 4.5, 0.0, std::f64::consts::TAU);
    cr.set_source_rgb(1.0, 1.0, 1.0);
    let _ = cr.fill();
}


pub(crate) struct Choice {
    pub row: gtk::Box,
    controls: Vec<gtk::Widget>,
    values: Rc<Vec<&'static str>>,
    selected: Rc<Cell<usize>>,
}

impl Choice {
    pub fn enable_when(&self, target: &impl IsA<gtk::Widget>, enabled: impl Fn(&str) -> bool + 'static) {
        let (target, values, selected) = (target.as_ref().downgrade(), self.values.clone(), self.selected.clone());
        let update = Rc::new(move || {
            if let Some(target) = target.upgrade() {
                target.set_sensitive(values.get(selected.get()).is_some_and(|v| enabled(v)));
            }
        });
        update();
        for control in &self.controls {
            let update = update.clone();
            if let Some(button) = control.downcast_ref::<gtk::ToggleButton>() {
                button.connect_toggled(move |_| update());
            } else {
                control.connect_notify_local(Some("selected"), move |_, _| update());
            }
        }
    }
}

pub(crate) fn choice(card: &gtk::Box, icon: &str, title: &str, sub: &str, key: &'static str, options: &[(&'static str, &'static str)], current: &str) -> Choice {
    let (row, end) = ui::row(Some(icon), title, sub);
    let labels: Vec<&str> = options.iter().map(|o| o.1).collect();
    let at = options.iter().position(|o| o.0 == current).unwrap_or(0);
    let values: Rc<Vec<&'static str>> = Rc::new(options.iter().map(|o| o.0).collect());
    let selected = Rc::new(Cell::new(at));
    let mut controls = Vec::new();
    let short = options.len() <= 3 && labels.iter().map(|l| l.chars().count()).sum::<usize>() <= 26;
    if short {
        let (values, selected) = (values.clone(), selected.clone());
        let bar = ui::segmented(&labels, at, move |i| {
            let ok = values.get(i).is_some_and(|v| save(key, toml::Value::String(v.to_string())));
            if ok { selected.set(i); }
            ok
        });
        let mut child = bar.first_child();
        while let Some(c) = child { child = c.next_sibling(); controls.push(c); }
        end.append(&bar);
    } else {
        let pick = kit::ui::dropdown(&labels, at as u32);
        let (values, selected) = (values.clone(), selected.clone());
        let restoring = Cell::new(false);
        pick.connect_selected_notify(move |p| {
            if restoring.get() { return; }
            let i = p.selected() as usize;
            if values.get(i).is_some_and(|v| save(key, toml::Value::String(v.to_string()))) {
                selected.set(i);
            } else {
                restoring.set(true);
                p.set_selected(selected.get() as u32);
                restoring.set(false);
            }
        });
        controls.push(pick.clone().upcast());
        end.append(&pick);
    }
    card.append(&row);
    Choice { row, controls, values, selected }
}

/// Preset corner radii, as multiples of the standard roundness.
const CORNERS: [(f64, &str); 4] = [(0.15, "Square"), (0.55, "Subtle"), (1.0, "Rounded"), (1.6, "Round")];

/// A row followed by a strip of picture tiles, one per option; returns the whole block.
pub(crate) fn tiles(card: &gtk::Box, icon: &str, title: &str, sub: &str, items: Vec<(&str, gtk::Widget)>, at: usize, pick: impl Fn(usize) -> bool + 'static) -> gtk::Box {
    let block = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let (row, _) = ui::row(Some(icon), title, sub);
    block.append(&row);
    let strip = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    strip.add_css_class("corners");
    strip.set_homogeneous(true);
    strip.set_halign(gtk::Align::Start);
    let mut buttons = Vec::new();
    for (name, picture) in items {
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
        strip.append(&b);
        buttons.push(b);
    }
    ui::select_buttons(&buttons, (at < buttons.len()).then_some(at), pick);
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

fn when_on(source: &gtk::Box, target: &impl IsA<gtk::Widget>) {
    if let Some(sw) = find_switch(source.upcast_ref()) {
        target.set_sensitive(sw.is_active());
        let target = target.as_ref().downgrade();
        sw.connect_active_notify(move |s| {
            if let Some(target) = target.upgrade() { target.set_sensitive(s.is_active()); }
        });
    }
}

pub(crate) fn toggle(card: &gtk::Box, icon: &str, title: &str, sub: &str, key: &'static str, on: bool) -> gtk::Box {
    let (row, end) = ui::row(Some(icon), title, sub);
    let sw = gtk::Switch::new();
    sw.set_valign(gtk::Align::Center);
    ui::bind_switch(&sw, on, move |on| save(key, toml::Value::Boolean(on)));
    end.append(&sw);
    card.append(&row);
    row
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn slider(card: &gtk::Box, icon: &str, title: &str, sub: &str, key: &'static str, range: (f64, f64, f64), value: f64, default: f64, digits: i32) -> gtk::Box {
    let (row, end) = ui::row(Some(icon), title, sub);
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, range.0, range.1, range.2);
    scale.set_value(value);
    scale.set_digits(digits);
    scale.set_draw_value(false);
    scale.set_size_request(240, -1);
    let reset = gtk::Button::new();
    reset.set_child(Some(&ui::sym("restart_alt")));
    reset.add_css_class("ctl");
    reset.set_tooltip_text(Some("Reset to default"));
    reset.set_valign(gtk::Align::Center);
    let s = scale.downgrade();
    reset.connect_clicked(move |_| {
        if let Some(s) = s.upgrade() { s.set_value(default); }
    });

    let precision = digits.max(0) as usize;
    let width = [range.0, range.1].iter().map(|v| format!("{v:.precision$}").len()).max().unwrap_or(1).max(4) as i32;
    let label = ui::text(&format!("{:.precision$}", scale.value()), "mono");
    label.set_xalign(1.0);
    label.set_width_chars(width);
    label.set_max_width_chars(width);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_tooltip_text(Some("Double-click to type a value"));
    let entry = gtk::Entry::new();
    entry.add_css_class("search");
    entry.add_css_class("mono");
    entry.set_width_chars(width);
    entry.set_max_width_chars(width);
    gtk::prelude::EditableExt::set_alignment(&entry, 1.0);
    entry.set_input_purpose(gtk::InputPurpose::Number);
    let value_slot = gtk::Stack::new();
    value_slot.set_hhomogeneous(true);
    value_slot.set_vhomogeneous(true);
    value_slot.set_transition_type(gtk::StackTransitionType::Crossfade);
    value_slot.set_transition_duration(160);
    value_slot.add_named(&label, Some("value"));
    value_slot.add_named(&entry, Some("entry"));
    value_slot.set_visible_child_name("value");
    let editing = Rc::new(Cell::new(false));
    let finish: Rc<dyn Fn(bool)> = {
        let (s, slot, entry) = (scale.downgrade(), value_slot.downgrade(), entry.downgrade());
        let editing = editing.clone();
        Rc::new(move |commit| {
            // Hide/focus changes can re-enter this handler; Escape must never commit.
            if !editing.replace(false) { return; }
            if let (Some(s), Some(slot), Some(entry)) = (s.upgrade(), slot.upgrade(), entry.upgrade()) {
                if commit && let Ok(v) = entry.text().trim().replace(',', ".").parse::<f64>() && v.is_finite() {
                    let v = v.clamp(range.0, range.1);
                    s.set_value((range.0 + ((v - range.0) / range.2).round() * range.2).clamp(range.0, range.1));
                }
                slot.set_visible_child_name("value");
            }
        })
    };
    let commit = finish.clone();
    entry.connect_activate(move |_| commit(true));
    let focus = gtk::EventControllerFocus::new();
    let commit = finish.clone();
    focus.connect_leave(move |_| commit(true));
    entry.add_controller(focus);
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            finish(false);
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    entry.add_controller(keys);
    let click = gtk::GestureClick::new();
    click.set_button(1);
    let (s, slot, input) = (scale.downgrade(), value_slot.downgrade(), entry.downgrade());
    click.connect_pressed(move |_, n, _, _| {
        if n == 2 && let (Some(s), Some(slot), Some(input)) = (s.upgrade(), slot.upgrade(), input.upgrade()) {
            input.set_text(&format!("{:.precision$}", s.value()));
            editing.set(true);
            slot.set_visible_child_name("entry");
            input.grab_focus();
            input.select_region(0, -1);
        }
    });
    label.add_controller(click);
    end.append(&reset);
    end.append(&value_slot);
    // Opacity preserves the button's slot; sensitivity removes the invisible focus target.
    let update = move |s: &gtk::Scale| {
        label.set_text(&format!("{:.precision$}", s.value()));
        let changed = (s.value() - default).abs() > 1e-9;
        reset.set_sensitive(changed);
        reset.set_opacity(if changed { 1.0 } else { 0.0 });
    };
    update(&scale);
    scale.connect_value_changed(update);
    let generation = Rc::new(Cell::new(0u64));
    let accepted = Rc::new(Cell::new(scale.value()));
    let restoring = Rc::new(Cell::new(false));
    scale.connect_value_changed(move |s| {
        if restoring.get() { return; }
        let g = generation.get() + 1;
        generation.set(g);
        let generation = generation.clone();
        let v = (s.value() * 100.0).round() / 100.0;
        let (s, accepted, restoring) = (s.downgrade(), accepted.clone(), restoring.clone());
        glib::timeout_add_local_once(Duration::from_millis(220), move || {
            if generation.get() == g {
                if save(key, toml::Value::Float(v)) {
                    accepted.set(v);
                } else if let Some(s) = s.upgrade() {
                    restoring.set(true);
                    s.set_value(accepted.get());
                    restoring.set(false);
                }
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
    row
}

pub(crate) fn field(card: &gtk::Box, icon: &str, title: &str, sub: &str, key: &'static str, current: &str, hint: &str, check: fn(&str) -> bool) -> gtk::Box {
    let (row, end) = ui::row(Some(icon), title, sub);
    let entry = gtk::Entry::new();
    entry.set_text(current);
    entry.set_placeholder_text(Some(hint));
    entry.set_valign(gtk::Align::Center);
    entry.set_width_chars(14);
    entry.set_max_length(64);
    let help = if sub.is_empty() { format!("Enter a valid {}", title.to_lowercase()) } else { sub.to_string() };
    ui::bind_entry(&entry, move |s| if check(s) { Ok(()) } else { Err(help.clone()) }, move |text| save(key, toml::Value::String(text.into())));
    end.append(&entry);
    card.append(&row);
    row
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
    kit::niriconf::niri()
}

fn free_windows() -> bool {
    std::fs::read_to_string(niri_dir().join("digit-windows.kdl")).is_ok_and(|t| t.contains("open-floating true"))
}

fn set_windows(free: bool) -> bool {
    // digit's niri rules include this file, so writing it is enough.
    if !kit::niriconf::installed() {
        ui::fail("Log in to niri with Digit to change these");
        return false;
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
        return false;
    }
    glib::spawn_future_local(async move {
        glib::timeout_future(Duration::from_millis(400)).await;
        if gio::spawn_blocking(move || crate::niri::retile(free)).await.is_ok() {
            ui::done(if free { "Windows now float freely" } else { "Windows are tiled in columns again" });
        } else {
            ui::fail("Window mode saved, but existing windows could not be rearranged");
        }
    });
    true
}

/// A settings page: which sections it shows, in order. Search builds the same sections to
/// index their rows, so a section is the unit both share.
pub type Section = fn(&gtk::Box, &toml::Table);

pub fn sections(page: &str) -> &'static [Section] {
    match page {
        "appearance" => &[shell_look, app_look, effects],
        "dock" => &[dock],
        "notch" => &[notch, menu_bar, panels, screen_edges],
        "desktop" => &[desktop, weather],
        "spotlight" => &[search, scopes, clipboard],
        "compositor" => &[session],
        "displays" => &[night_light],
        "power" => &[power],
        "screenshots" => &[screenshots],
        "files" => &[files],
        "overview" => &[advanced],
        _ => &[],
    }
}

/// Builds a page made only of sections.
pub fn build(page: &str, title: &str, lead: &str) -> gtk::Widget {
    let (scroll, body) = ui::page(title, lead);
    let t = prefs::read();
    for section in sections(page) {
        section(&body, &t);
    }
    // A page of one section doesn't repeat its name under the title.
    let mut child = body.first_child();
    while let Some(c) = child {
        child = c.next_sibling();
        if c.has_css_class("heading") && c.downcast_ref::<gtk::Label>().is_some_and(|l| l.text() == title) {
            c.set_visible(false);
        }
    }
    scroll.upcast()
}

/// The keys a shortcut is on now (Settings > Keyboard), or None when it is off.
fn keys(id: &str) -> Option<String> {
    kit::keybinds::shown(id)
}

/// A shortcut's keys as a badge that opens it in Settings > Keyboard.
pub(crate) fn shortcut(id: &'static str) -> gtk::Button {
    let b = ui::button(&keys(id).unwrap_or_else(|| "Off".into()), "");
    b.add_css_class("keycap");
    b.set_valign(gtk::Align::Center);
    b.set_tooltip_text(Some("Change in Keyboard"));
    b.connect_clicked(move |_| {
        let label = kit::keybinds::all().into_iter().find(|b| b.id == id).map(|b| b.label).unwrap_or_default();
        crate::show(&format!("keyboard/{}", crate::search::slug(&label)));
    });
    b
}

fn section(body: &gtk::Box, title: &str) -> gtk::Box {
    body.append(&ui::heading(title));
    let card = ui::card();
    body.append(&card);
    card
}

fn shell_look(body: &gtk::Box, t: &toml::Table) {
    let look = section(body, "Shell");
    choice(&look, "dark_mode", "Shell surfaces", "Notch, dock, menus and panels", "surface", &[("black", "Pure black"), ("tinted", "Tinted by wallpaper")], &prefs::get_str(&t, "surface", "black"));
    let (row, end) = ui::row(Some("palette"), "Color scheme", "Colors generated from your wallpaper");
    let current = kit::theme::load().map(|t| t.scheme).unwrap_or_else(|| prefs::get_str(&t, "scheme", "content"));
    let schemes: Vec<&str> = kit::theme::SCHEMES.to_vec();
    let pick = kit::ui::dropdown(&schemes, 0);
    let selected = schemes.iter().position(|s| *s == current).unwrap_or(0) as u32;
    pick.set_selected(selected);
    let accepted = Rc::new(Cell::new(selected));
    let restoring = Rc::new(Cell::new(false));
    pick.connect_selected_notify(move |p| {
        if restoring.get() { return; }
        let Some(name) = kit::theme::SCHEMES.get(p.selected() as usize) else { return };
        let selected = p.selected();
        p.set_sensitive(false);
        let argv: [&std::ffi::OsStr; 4] = ["digit".as_ref(), "theme".as_ref(), "--scheme".as_ref(), name.as_ref()];
        let process = gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE);
        let (pick, accepted, restoring) = (p.clone(), accepted.clone(), restoring.clone());
        glib::spawn_future_local(async move {
            let result = match process { Ok(p) => p.wait_check_future().await, Err(e) => Err(e) };
            match result {
                Ok(()) => { accepted.set(selected); ui::done(&format!("Color scheme: {name}")); }
                Err(e) => {
                    restoring.set(true);
                    pick.set_selected(accepted.get());
                    restoring.set(false);
                    ui::fail(&format!("Could not apply the color scheme: {e}"));
                }
            }
            pick.set_sensitive(true);
        });
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
    let accepted = Cell::new(pick.selected());
    let restoring = Cell::new(false);
    pick.connect_selected_notify(move |p| {
        if restoring.get() { return; }
        let i = p.selected() as usize;
        let r = if i == 0 { prefs::unset("icon_theme") } else { themes.get(i - 1).map_or(Ok(()), |n| prefs::set("icon_theme", toml::Value::String(n.clone()))) };
        if let Err(e) = r {
            ui::fail(&e);
            restoring.set(true);
            p.set_selected(accepted.get());
            restoring.set(false);
        } else {
            accepted.set(p.selected());
        }
    });
    end.append(&pick);
    look.append(&row);
}

fn app_look(body: &gtk::Box, t: &toml::Table) {
    let apps = section(body, "Apps");
    surfaces(&apps, "layers", "App surfaces", "Background of Digit apps and the Game Bar", &prefs::get_str(&t, "ui_surface", "black"));
    accent(&apps, &t);
    corners(&apps, "rounded_corner", "Corners", "Cards, menus, buttons and dialogs", "ui_round", prefs::get_f64(&t, "ui_round", 1.0));
    choice(&apps, "density_medium", "Density", "", "ui_density", &[("comfortable", "Comfortable"), ("compact", "Compact")], &prefs::get_str(&t, "ui_density", "comfortable"));
    toggle(&apps, "select_window", "Open Settings as a window", "Settings opens over everything by default; turn this on for an ordinary window. Takes effect the next time Settings opens", "settings_window", prefs::get_bool(&t, "settings_window", false));
}

fn effects(body: &gtk::Box, t: &toml::Table) {
    let look = section(body, "Motion and effects");
    toggle(&look, "motion_photos_off", "Reduce motion", "Turn off panel transitions and decorative animations in Digit", "reduce_motion", prefs::get_bool(&t, "reduce_motion", false));
    toggle(&look, "swipe_vertical", "Smooth scrolling", "Mouse wheel steps glide instead of jumping. Touchpads always scroll smoothly", "smooth_scroll", prefs::get_bool(&t, "smooth_scroll", true));
    toggle(&look, "speed", "Reduce effects", "No shadows, blur or frosted panels in digit and the compositor; smoother on older laptops", "reduced_effects", prefs::get_bool(&t, "reduced_effects", false));
    choice(&look, "memory", "Rendering", "How digit and its apps draw. Takes effect when they next start", "renderer", &[("auto", "Automatic"), ("gl", "GPU (OpenGL)"), ("vulkan", "GPU (Vulkan)"), ("cairo", "Software")], &prefs::get_str(&t, "renderer", "auto"));
    toggle(&look, "auto_awesome", "Startup animation", "Greet each start of the shell with a short full-screen animation", "intro", prefs::get_bool(&t, "intro", true));
}

fn dock(body: &gtk::Box, t: &toml::Table) {
    let dock = section(body, "Dock");
    let shows = keys("dock").map(|k| format!("{k} shows or hides it at any time")).unwrap_or_default();
    let visibility = choice(&dock, "dock_to_bottom", "Visibility", &shows, "dock_mode", &[("on", "Always shown"), ("autohide", "Auto-hide"), ("smart", "Hide when windows are open"), ("off", "Off")], &prefs::get_str(&t, "dock_mode", "on"));
    choice(&dock, "side_navigation", "Position", "", "dock_side", &[("bottom", "Bottom"), ("left", "Left"), ("right", "Right")], &prefs::get_str(&t, "dock_side", "bottom"));
    slider(&dock, "photo_size_select_small", "Icon size", "", "dock_size", (32.0, 80.0, 2.0), prefs::get_f64(&t, "dock_size", 50.0), 50.0, 0);
    slider(&dock, "zoom_in", "Magnification", "1.0 turns it off", "dock_magnify", (1.0, 2.0, 0.05), prefs::get_f64(&t, "dock_magnify", 1.5), 1.5, 2);
    toggle(&dock, "apps", "Show running apps", "Apps that are open but not kept in the dock", "dock_running", prefs::get_bool(&t, "dock_running", true));
    toggle(&dock, "notifications", "Notification badges", "Unread notification counts on icons", "dock_badges", prefs::get_bool(&t, "dock_badges", true));
    let style = match prefs::get_str(&t, "dock_icons", "light").as_str() {
        "tile" => "graphite".to_string(),
        other => other.to_string(),
    };
    choice(&dock, "style", "Icon style", "Big Sur shaped icons for every app", "dock_icons", &[("light", "Big Sur"), ("graphite", "Big Sur graphite"), ("mono", "Mono"), ("tinted", "Tinted"), ("native", "Original")], &style);
    let mut child = dock.first_child();
    while let Some(row) = child {
        child = row.next_sibling();
        if row != visibility.row { visibility.enable_when(&row, |v| v != "off"); }
    }
}

pub fn session_notice(body: &gtk::Box, kind: kit::setup::Kind) {
    let card = ui::card();
    let (row, _) = ui::row(Some("info"), &format!("Log in to {} with Digit to change these", kind.name()), "Choose the session on your login screen");
    card.append(&row);
    body.append(&card);
}

pub fn session(body: &gtk::Box, _: &toml::Table) {
    let Some(kind) = kit::setup::kind() else { return };
    if !kit::setup::installed() {
        session_notice(body, kind);
        return;
    }
    let win = section(body, "Session");
    let user = match kind {
        kit::setup::Kind::Hyprland => "~/.config/digit/hyprland/user.lua",
        kit::setup::Kind::Niri => "~/.config/digit/niri/user.kdl",
    };
    let (row, _) = ui::row(Some("tune"), &format!("{} with Digit", kind.name()), &format!("Your additions in {user} load last"));
    win.append(&row);
    if kit::setup::kind() == Some(kit::setup::Kind::Niri) {
        let (row, end) = ui::row(Some("select_window"), "Window placement", "Free: windows open side by side; drag by the title bar or Super + drag, resize with Super + right drag. Super+Up maximizes, Super+Left/Right fills a half, Super+Down restores");
        let modes = ["Tiling columns", "Free placement"];
        let pick = kit::ui::dropdown(&modes, 0);
        pick.set_selected(if free_windows() { 1 } else { 0 });
        let accepted = Cell::new(pick.selected());
        let restoring = Cell::new(false);
        pick.connect_selected_notify(move |p| {
            if restoring.get() { return; }
            if set_windows(p.selected() == 1) {
                accepted.set(p.selected());
            } else {
                restoring.set(true);
                p.set_selected(accepted.get());
                restoring.set(false);
            }
        });
        end.append(&pick);
        win.append(&row);
        let (row, end) = ui::row(Some("dashboard"), "Window layouts", "Arranges the focused window: halves, thirds, corners and more");
        end.append(&shortcut("layouts"));
        win.append(&row);
    }
}

fn notch(body: &gtk::Box, t: &toml::Table) {
    let notch = section(body, "Notch");
    // The same pictures as the setup screen.
    const STYLES: [(&str, &str); 3] = [("island", "Notch"), ("dynamic", "Dynamic Island"), ("bar", "Bar")];
    let current = prefs::get_str(t, "notch_style", "island");
    let at = STYLES.iter().position(|s| s.0 == current).unwrap_or(0);
    let items = STYLES
        .iter()
        .map(|(key, name)| {
            let picture = crate::welcome::preview(key);
            picture.set_content_width(136);
            picture.set_content_height(72);
            (*name, picture.upcast())
        })
        .collect();
    tiles(&notch, "toolbar", "Style", "Notch hugs the top edge, Dynamic Island floats as a pill, Bar spans the whole edge with the app, status and date", items, at, |i| save("notch_style", toml::Value::String(STYLES[i].0.into())));
    choice(&notch, "call", "Call activity", "A live pill with the call timer and voice waveform while an app uses the microphone", "notch_call", &[("voice", "Voice and video apps"), ("any", "Any app using the microphone"), ("off", "Off")], &prefs::get_str(&t, "notch_call", "voice"));
    toggle(&notch, "mic_off", "Block idle microphone", "Voice apps like Discord can keep listening after a call ends. Digit silences only their microphone stream until a real call starts, then gives it back instantly", "mic_guard", prefs::get_bool(&t, "mic_guard", true));
    toggle(&notch, "touch_app", "Open on hover", "Expand the notch when the pointer rests on it", "hover", prefs::get_bool(&t, "hover", true));
    toggle(&notch, "rounded_corner", "Match app roundness", "The notch's corners follow Corners in Appearance", "notch_round_follow", prefs::get_bool(&t, "notch_round_follow", true));
    corners(&notch, "rounded_corner", "Notch corners", "Used when the notch doesn't match the apps", "notch_round", prefs::get_f64(&t, "notch_round", 1.0));
    // The notch's own roundness only applies while it doesn't follow the apps.
    if let (Some(own), Some(follow)) = (notch.last_child(), notch.last_child().and_then(|w| w.prev_sibling()).and_then(|row| find_switch(&row))) {
        own.set_sensitive(!follow.is_active());
        follow.connect_active_notify(move |s| own.set_sensitive(!s.is_active()));
    }
}

fn menu_bar(body: &gtk::Box, t: &toml::Table) {
    let bar = section(body, "Menu bar");
    let enabled = toggle(&bar, "menu", "Menu bar", "Distro menu, the focused app's menus, status and date beside the notch", "menubar", prefs::get_bool(&t, "menubar", true));
    let backdrop = choice(&bar, "blur_on", "Menu bar backdrop", "Adaptive stays clear over the wallpaper and turns to frosted glass when windows are open", "menubar_look", &[("adaptive", "Adaptive"), ("clear", "Clear"), ("glass", "Glass")], &prefs::get_str(&t, "menubar_look", "adaptive"));
    slider(&bar, "height", "Bar height", "Used by the full-width bar", "wide_height", (28.0, 56.0, 1.0), prefs::get_f64(&t, "wide_height", 34.0), 34.0, 0);
    let tray = toggle(&bar, "widgets", "Menu bar app icons", "Shows the icons apps like Discord or Steam put in the system tray. Applies after digit restarts", "tray", prefs::get_bool(&t, "tray", true));
    when_on(&enabled, &backdrop.row);
    when_on(&enabled, &tray);
}

fn panels(body: &gtk::Box, t: &toml::Table) {
    let panels = section(body, "Panels close");
    let close = [("leave", "When the pointer leaves"), ("click", "On a click elsewhere")];
    choice(&panels, "toolbar", "Notch", "When the open notch folds away", "notch_close", &close, &prefs::get_str(&t, "notch_close", "leave"));
    choice(&panels, "grid_view", "Workspaces", "When the workspaces panel in the top-left corner closes", "spaces_close", &close, &prefs::get_str(&t, "spaces_close", "leave"));
    choice(&panels, "tune", "Control Center", "When Control Center in the bottom-right corner closes", "control_close", &close, &prefs::get_str(&t, "control_close", "leave"));
    choice(&panels, "calendar_month", "Calendar", "When the calendar under the date closes", "calendar_close", &close, &prefs::get_str(&t, "calendar_close", "leave"));
}

fn screen_edges(body: &gtk::Box, t: &toml::Table) {
    let edge = section(body, "Screen corners");
    toggle(&edge, "north_west", "Workspace corner", "Move the pointer to the top-left corner to see workspaces", "hot_corner", prefs::get_bool(&t, "hot_corner", true));
    toggle(&edge, "south_east", "Control Center corner", "Move the pointer to the bottom-right corner to open Control Center", "control_corner", prefs::get_bool(&t, "control_corner", true));
    toggle(&edge, "fullscreen", "Over fullscreen apps", "Let the dock edge and screen corners open panels while an app or game is fullscreen. Spotlight always opens", "fullscreen_edges", prefs::get_bool(&t, "fullscreen_edges", false));
}

fn power(body: &gtk::Box, t: &toml::Table) {
    let power = section(body, "Lock and power");
    slider(&power, "lock_clock", "Lock after idle", "Minutes without input before the screen locks. 0 never locks on its own", "idle_lock", (0.0, 60.0, 1.0), prefs::get_f64(&t, "idle_lock", 0.0), 0.0, 0);
    slider(&power, "monitor", "Turn off displays", "Minutes without input. The screen dims softly for a few seconds first; any input cancels", "idle_off", (0.0, 60.0, 1.0), prefs::get_f64(&t, "idle_off", 10.0), 10.0, 0);
    slider(&power, "bedtime", "Sleep after idle", "Minutes without input before the computer sleeps. 0 never sleeps on its own", "idle_sleep", (0.0, 180.0, 5.0), prefs::get_f64(&t, "idle_sleep", 0.0), 0.0, 0);
    toggle(&power, "lock", "Lock before sleep", "The lock screen is ready before the computer suspends, so waking needs your password", "lock_on_sleep", prefs::get_bool(&t, "lock_on_sleep", true));
    let mut said = Vec::new();
    if let Some(k) = keys("lock") {
        said.push(format!("{k} locks"));
    }
    if let Some(k) = keys("session") {
        said.push(format!("{k} opens Lock, Sleep, Restart, Shut Down and Log Out"));
    }
    said.push("Passwords are checked by PAM in a separate process".into());
    let (row, end) = ui::row(Some("preview"), "Lock screen", &said.join(". "));
    let look = gtk::Button::with_label("Preview");
    look.set_valign(gtk::Align::Center);
    look.connect_clicked(|_| {
        let _ = gio::Subprocess::newv(&[std::ffi::OsStr::new("digitctl"), std::ffi::OsStr::new("msg"), std::ffi::OsStr::new("lock"), std::ffi::OsStr::new("preview")], gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE);
    });
    end.append(&look);
    power.append(&row);
}

pub fn night_light(body: &gtk::Box, t: &toml::Table) {
    let night = section(body, "Night Light");
    let mode = choice(&night, "nightlight", "Night Light", "Warms the screen in the evening to be easier on your eyes. Sunset uses the weather city set in Desktop", "night_light", &[("off", "Off"), ("sunset", "Sunset to sunrise"), ("schedule", "Custom hours"), ("always", "Always on")], &prefs::get_str(&t, "night_light", "off"));
    let warmth = slider(&night, "thermostat", "Warmth", "Color temperature at night, in kelvin. Lower is warmer", "night_temp", (2500.0, 6000.0, 100.0), prefs::get_f64(&t, "night_temp", 4200.0), 4200.0, 0);
    let from = field(&night, "schedule", "From", "Custom hours start, 24-hour HH:MM", "night_from", &prefs::get_str(&t, "night_from", "20:00"), "20:00", hhmm);
    let until = field(&night, "wb_sunny", "Until", "Custom hours end, 24-hour HH:MM", "night_to", &prefs::get_str(&t, "night_to", "07:00"), "07:00", hhmm);
    mode.enable_when(&warmth, |v| v != "off");
    mode.enable_when(&from, |v| v == "schedule");
    mode.enable_when(&until, |v| v == "schedule");
}

fn desktop(body: &gtk::Box, t: &toml::Table) {
    let desk = section(body, "Desktop");
    choice(&desk, "schedule", "Desktop clock", "A large clock with the date and weather on the wallpaper, behind your windows", "desktop_widget", &[("off", "Off"), ("left", "Top left"), ("center", "Center")], &prefs::get_str(&t, "desktop_widget", "off"));
    slider(&desk, "rounded_corner", "Rounded screen corners", "Softens the corners of every display. 0 turns it off", "screen_corners", (0.0, 24.0, 1.0), prefs::get_f64(&t, "screen_corners", 0.0), 0.0, 0);
}

fn weather(body: &gtk::Box, t: &toml::Table) {
    let sky = section(body, "Weather");
    field(&sky, "location_on", "City", "Shown in Control Center and on the desktop clock. The city name goes to open-meteo.com over HTTPS; leave empty to keep weather off", "weather_city", &prefs::get_str(&t, "weather_city", ""), "Istanbul", |_| true);
    choice(&sky, "device_thermostat", "Units", "", "weather_unit", &[("celsius", "°C"), ("fahrenheit", "°F")], &prefs::get_str(&t, "weather_unit", "celsius"));
}

fn search(body: &gtk::Box, t: &toml::Table) {
    let find = section(body, "Search");
    toggle(&find, "text_select_move_forward_character", "Smooth caret", "The caret in Spotlight glides as you type and blinks softly, like VS Code", "smooth_caret", prefs::get_bool(&t, "smooth_caret", true));
    let enabled = toggle(&find, "manage_search", "Search inside apps", "Spotlight asks apps that offer results, such as Files, Firefox tabs and Characters", "search_apps", prefs::get_bool(&t, "search_apps", true));
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
        sw.set_valign(gtk::Align::Center);
        let initial = states[i].1.get();
        let states = states.clone();
        ui::bind_switch(&sw, initial, move |on| {
            let list: Vec<toml::Value> = states.iter().enumerate().filter(|(j, (_, state))| !if *j == i { on } else { state.get() }).map(|(_, (id, _))| toml::Value::String(id.clone())).collect();
            let ok = save("search_providers_off", toml::Value::Array(list));
            if ok { states[i].1.set(on); }
            ok
        });
        end.append(&sw);
        when_on(&enabled, &row);
        find.append(&row);
    }
}

/// Spotlight's search shortcuts: key, name and when they're offered. Keep in step with
/// `providers::scopes` in the shell.
const SCOPES: [(&str, &str, &str, &str); 8] = [
    ("set", "Settings", "settings", "Finds a setting and opens it"),
    ("dc", "Discord", "forum", "When Discord is installed"),
    ("sp", "Spotify", "music_note", "When Spotify is installed"),
    ("st", "Steam store", "sports_esports", "When Steam is installed"),
    ("app", "Digit Store", "storefront", ""),
    ("yt", "YouTube", "smart_display", ""),
    ("gh", "GitHub", "code", ""),
    ("w", "Wikipedia", "menu_book", ""),
];

fn scopes(body: &gtk::Box, t: &toml::Table) {
    let card = section(body, "Search shortcuts");
    let (row, end) = ui::row(Some("keyboard_command_key"), "How they work", "Type a key and a space in Spotlight, then your search");
    end.append(&ui::badge("yt cats", ""));
    card.append(&row);
    let off: Rc<std::cell::RefCell<Vec<String>>> = Rc::new(std::cell::RefCell::new(
        t.get("search_scopes_off").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default(),
    ));
    for (key, name, icon, note) in SCOPES {
        let (row, end) = ui::row(Some(icon), name, note);
        end.append(&ui::badge(key, ""));
        let sw = gtk::Switch::new();
        sw.set_valign(gtk::Align::Center);
        let initial = !off.borrow().iter().any(|k| k == key);
        let off = off.clone();
        ui::bind_switch(&sw, initial, move |on| {
            let mut list = off.borrow().clone();
            list.retain(|k| k != key);
            if !on { list.push(key.to_string()); }
            let ok = save("search_scopes_off", toml::Value::Array(list.iter().cloned().map(toml::Value::String).collect()));
            if ok { *off.borrow_mut() = list; }
            ok
        });
        end.append(&sw);
        card.append(&row);
    }
}

fn clipboard(body: &gtk::Box, t: &toml::Table) {
    let clip = section(body, "Clipboard");
    let lists = keys("clipboard").map(|k| format!("{k} lists")).unwrap_or_else(|| "Lists".into());
    toggle(&clip, "content_paste", "Clipboard history", &format!("{lists} recent copies with color conversions. Kept only in memory; password managers' copies are skipped. Applies after digit restarts"), "clipboard", prefs::get_bool(&t, "clipboard", true));
}

fn screenshots(body: &gtk::Box, t: &toml::Table) {
    let snap = section(body, "Screenshots");
    let said: Vec<String> = [("snap", "area"), ("snap-area", "area"), ("snap-screen", "screen"), ("snap-window", "window"), ("snap-delay", "3 s timer")]
        .iter()
        .filter_map(|(id, what)| keys(id).map(|k| format!("{k}: {what}")))
        .collect();
    let said = if said.is_empty() { "None set".to_string() } else { said.join(" · ") };
    let (row, end) = ui::row(Some("screenshot_region"), "Shortcuts", &said);
    let change = ui::button("Change…", "");
    change.connect_clicked(|_| crate::show("keyboard/screenshots"));
    end.append(&change);
    snap.append(&row);
    choice(&snap, "keyboard_return", "Enter after selecting", "Double-click inside the selection does the same", "snap_after", &[("copy", "Copy"), ("save", "Save"), ("edit", "Mark up"), ("pin", "Pin to screen")], &prefs::get_str(&t, "snap_after", "copy"));
    toggle(&snap, "save", "Also save copies", "Keep a file every time a screenshot is copied", "snap_save_on_copy", prefs::get_bool(&t, "snap_save_on_copy", true));
    let dir = prefs::get_str(&t, "snap_dir", "");
    let default = glib::user_special_dir(glib::UserDirectory::Pictures).unwrap_or_else(glib::home_dir).join("Screenshots").display().to_string();
    let shown = if dir.is_empty() { default.clone() } else { dir };
    let (row, end) = ui::row(Some("folder"), "Save to", "");
    let entry = gtk::Entry::new();
    entry.set_text(&shown);
    entry.set_width_chars(22);
    entry.set_hexpand(true);
    entry.set_placeholder_text(Some(&default));
    ui::bind_entry(&entry, screenshot_folder, |text| save("snap_dir", toml::Value::String(text.into())));
    end.append(&entry);
    let pickb = ui::button("Choose…", "");
    let target = entry.downgrade();
    pickb.connect_clicked(move |b| {
        let Some(entry) = target.upgrade() else { return };
        let path = kit::prefs::home(entry.text().trim());
        let target = target.clone();
        ui::choose_folder(ui::root(b), &path, move |path| {
            if let Some(entry) = target.upgrade() {
                entry.set_text(&path.display().to_string());
                entry.emit_activate();
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
}

fn screenshot_folder(text: &str) -> Result<(), String> {
    if text.is_empty() { return Ok(()); }
    let path = kit::prefs::home(text);
    if !path.is_absolute() || text.chars().any(char::is_control) {
        return Err("Enter an absolute path or a path starting with ~/".into());
    }
    if path.exists() && !path.is_dir() {
        return Err("Choose a folder, not a file".into());
    }
    Ok(())
}

fn files(body: &gtk::Box, t: &toml::Table) {
    let files = section(body, "Files");
    let tips = keys("files").into_iter().chain(["Space previews".into(), "F3 splits".into(), "type to jump".into()]).collect::<Vec<_>>().join(" · ");
    let (row, end) = ui::row(Some("folder_open"), "Open Files", &tips);
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
}

pub fn advanced(body: &gtk::Box, _: &toml::Table) {
    let adv = section(body, "Advanced");
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
}

#[cfg(test)]
mod tests {
    #[test]
    fn screenshot_folder_accepts_paths_and_rejects_files() {
        assert!(super::screenshot_folder("").is_ok());
        assert!(super::screenshot_folder("~/Pictures/Screenshots").is_ok());
        assert!(super::screenshot_folder("/tmp/digit-screenshots/new-folder").is_ok());
        assert!(super::screenshot_folder("relative/folder").is_err());
        assert!(super::screenshot_folder("/tmp/bad\nfolder").is_err());
        let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        assert!(super::screenshot_folder(file.to_str().unwrap()).is_err());
    }
}
