use crate::{prefs, ui};
use gtk::{gio, glib, prelude::*};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

pub(crate) fn save(key: &str, value: toml::Value) -> bool {
    save_many(&[(key, value)])
}

/// Regenerates digit's part of the compositor config in the background.
fn apply_compositor() {
    glib::spawn_future_local(async {
        match gio::spawn_blocking(kit::setup::apply).await {
            Ok(Ok(())) => {},
            Ok(Err(e)) => ui::fail(&format!("Saved, but could not apply to the compositor: {e}")),
            Err(_) => ui::fail("Saved, but could not apply to the compositor"),
        }
    });
}

/// Whether a save went through; a failed one is shown.
pub(crate) fn saved(result: Result<(), String>) -> bool {
    result.map_err(|e| ui::fail(&format!("Could not save: {e}"))).is_ok()
}

pub(crate) fn save_many(values: &[(&str, toml::Value)]) -> bool {
    if let Err(e) = prefs::set_many(values) {
        ui::fail(&format!("Could not save: {e}"));
        return false;
    }
    if values.iter().any(|(key, _)| matches!(*key, "reduce_motion" | "reduced_effects")) {
        kit::motion::refresh();
        kit::effects::invalidate();
        // Rebuild after the switch callback finishes; cached child pages must also be dropped.
        glib::idle_add_local_once(crate::reload_reductions);
    } else if values.iter().any(|(key, _)| *key == "compositor_motion") {
        glib::idle_add_local_once(refresh_controlled);
    }
    // Compositor settings (and the accent, which colors niri's window border) go straight into
    // digit's part of the compositor config.
    if values.iter().any(|(key, _)| key.starts_with("hypr_") || key.starts_with("niri_") || matches!(*key, "spaces_always" | "window_rules" | "ui_accent" | "border_color" | "reduce_motion" | "reduced_effects" | "compositor_motion" | "terminal" | "theme_qt" | "nvidia_env" | "cursor_theme" | "cursor_size" | "hdr_fullscreen")) {
        apply_compositor();
    }
    if values.iter().any(|(key, _)| matches!(*key, "button_maximize" | "button_close")) {
        glib::spawn_future_local(async {
            let _ = gio::spawn_blocking(kit::appearance::apply_buttons).await;
        });
    }
    if values.iter().any(|(key, _)| *key == "theme_qt") { ui::done("Log in again to apply to all apps"); }
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
        let rgb = kit::pictures::rgb(&hex);
        area.set_draw_func(move |a, cr, w, h| kit::pictures::accent(a, cr, w as f64, h as f64, rgb));
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

/// One row state for all controls owned by a reduction master.
pub(crate) fn controlled(key: &str) -> Option<&'static str> {
    let still = !prefs::get_bool(&prefs::read(), "compositor_motion", true);
    controlled_in(key, kit::motion::reduced(), kit::effects::reduced(), still)
}

/// `still`: Compositor animations is off, which stops the compositor's animations on its own.
fn controlled_in(key: &str, motion: bool, effects: bool, still: bool) -> Option<&'static str> {
    match key {
        "smooth_scroll" | "smooth_caret" | "intro" | "compositor_motion" | "hypr_window_anim" | "hypr_workspace_anim" | "niri_motion" if motion => Some("Off while Reduce motion is on"),
        "hypr_window_anim" | "hypr_workspace_anim" | "niri_motion" if still => Some("Off while Compositor animations is off"),
        "hypr_blur" | "hypr_shadow" | "menubar_look" if effects => Some("Off while Reduce effects is on"),
        _ => None,
    }
}

thread_local! {
    /// Rows that Reduce motion or Reduce effects can turn off: the row, its setting and its own
    /// subtitle, so they can follow a change in place instead of the page being rebuilt.
    static CONTROLLED: std::cell::RefCell<Vec<(glib::WeakRef<gtk::Box>, &'static str, String)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Marks `row` (with subtitle `sub`) as turned off while a reduction master owns `key`, and keeps
/// it following later changes.
pub(crate) fn controlled_row(row: &gtk::Box, key: &'static str, sub: &str) {
    if controlled_in(key, true, true, true).is_none() { return; }
    CONTROLLED.with(|c| {
        let mut c = c.borrow_mut();
        c.retain(|(w, _, _)| w.upgrade().is_some());
        c.push((row.downgrade(), key, sub.to_string()));
    });
    show_controlled(row, key, sub);
}

fn show_controlled(row: &gtk::Box, key: &str, sub: &str) {
    let detail = controlled(key);
    if detail.is_some() { row.add_css_class("controlled") } else { row.remove_css_class("controlled") }
    row.set_sensitive(detail.is_none());
    if let Some(label) = sub_label(row.upcast_ref()) {
        label.set_text(detail.unwrap_or(sub));
        label.set_visible(!label.text().is_empty());
    }
}

fn sub_label(w: &gtk::Widget) -> Option<gtk::Label> {
    if let Some(l) = w.downcast_ref::<gtk::Label>().filter(|l| l.has_css_class("sub")) { return Some(l.clone()); }
    let mut child = w.first_child();
    while let Some(c) = child {
        if let Some(l) = sub_label(&c) { return Some(l); }
        child = c.next_sibling();
    }
    None
}

/// Brings every row a reduction master owns up to date where it is, keeping the page's place.
pub(crate) fn refresh_controlled() {
    let rows: Vec<_> = CONTROLLED.with(|c| c.borrow().iter().filter_map(|(w, k, s)| w.upgrade().map(|w| (w, *k, s.clone()))).collect());
    for (row, key, sub) in rows {
        show_controlled(&row, key, &sub);
    }
}

pub(crate) struct Choice {
    pub row: gtk::Box,
    controls: Vec<gtk::Widget>,
    values: Rc<Vec<String>>,
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

fn choice_value(key: &str, value: &str) -> toml::Value {
    if key == "spaces_always" { toml::Value::Integer(value.parse::<i64>().unwrap_or(0).clamp(0, 10)) }
    else if key == "wallpaper_every" { toml::Value::Integer(value.parse::<i64>().unwrap_or(0).clamp(0, 10_080)) }
    else { toml::Value::String(value.to_string()) }
}

pub(crate) fn choice(card: &gtk::Box, icon: &str, title: &str, sub: &str, key: &'static str, options: &[(&str, &str)], current: &str) -> Choice {
    let detail = controlled(key);
    let mut options = options.to_vec();
    let current = if detail.is_some() {
        if key == "menubar_look" { "clear" } else {
            if !options.iter().any(|o| o.0 == "off") { options.push(("off", "Off")); }
            "off"
        }
    } else { current };
    let (row, end) = ui::row(Some(icon), title, detail.unwrap_or(sub));
    controlled_row(&row, key, sub);
    let labels: Vec<&str> = options.iter().map(|o| o.1).collect();
    let at = options.iter().position(|o| o.0 == current).unwrap_or(0);
    let values: Rc<Vec<String>> = Rc::new(options.iter().map(|o| o.0.to_string()).collect());
    let selected = Rc::new(Cell::new(at));
    let mut controls = Vec::new();
    let short = !matches!(key, "gtk_theme" | "qt_theme" | "qt_colors") && options.len() <= 3 && labels.iter().map(|l| l.chars().count()).sum::<usize>() <= 26;
    if short {
        let (values, selected) = (values.clone(), selected.clone());
        let bar = ui::segmented(&labels, at, move |i| {
            let ok = values.get(i).is_some_and(|v| save(key, choice_value(key, v)));
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
            if values.get(i).is_some_and(|v| save(key, choice_value(key, v))) {
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
    // A grid that folds onto more lines when the window is too narrow for one.
    let count = items.len() as u32;
    let strip = gtk::FlowBox::new();
    strip.add_css_class("corners");
    strip.set_selection_mode(gtk::SelectionMode::None);
    strip.set_homogeneous(true);
    strip.set_halign(gtk::Align::Start);
    strip.set_max_children_per_line(count.max(1));
    strip.set_min_children_per_line(count.clamp(1, 2));
    strip.set_column_spacing(10);
    strip.set_row_spacing(10);
    let mut buttons = Vec::new();
    for (name, picture) in items {
        let b = gtk::ToggleButton::new();
        b.add_css_class("corner");
        let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
        picture.add_css_class("corner-tile");
        picture.set_size_request(136, 72);
        if let Some(a) = picture.downcast_ref::<gtk::DrawingArea>() {
            a.set_content_width(136);
            a.set_content_height(72);
        }
        picture.set_overflow(gtk::Overflow::Hidden);
        col.append(&picture);
        let label = gtk::Label::new(Some(name));
        label.add_css_class("corner-name");
        col.append(&label);
        b.set_child(Some(&col));
        strip.insert(&b, -1);
        // The cell takes no focus or hover of its own; the tile shows both.
        if let Some(cell) = b.parent() {
            cell.set_focusable(false);
        }
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
        target.set_sensitive(sw.is_active() && !target.has_css_class("controlled"));
        let target = target.as_ref().downgrade();
        sw.connect_active_notify(move |s| {
            if let Some(target) = target.upgrade() { target.set_sensitive(s.is_active() && !target.has_css_class("controlled")); }
        });
    }
}

/// Shows `target` only while the switch in `source` is on.
fn show_when_on(source: &gtk::Box, target: &impl IsA<gtk::Widget>) {
    if let Some(sw) = find_switch(source.upcast_ref()) {
        target.set_visible(sw.is_active());
        let target = target.as_ref().downgrade();
        sw.connect_active_notify(move |s| {
            if let Some(target) = target.upgrade() { target.set_visible(s.is_active()); }
        });
    }
}

/// A "Show advanced options" row at the end of `card`; returns where the advanced rows go,
/// folded away until it's opened.
fn advanced_rows(card: &gtk::Box) -> gtk::Box {
    let (row, end) = ui::row(Some("tune"), "Advanced options", "");
    let arrow = ui::sym("expand_more");
    end.append(&arrow);
    row.set_cursor_from_name(Some("pointer"));
    card.append(&row);
    let reveal = gtk::Revealer::new();
    reveal.set_transition_type(gtk::RevealerTransitionType::SlideDown);
    reveal.set_transition_duration(200);
    let rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
    reveal.set_child(Some(&rows));
    card.append(&reveal);
    let click = gtk::GestureClick::new();
    click.connect_released(move |_, _, _, _| {
        let open = !reveal.reveals_child();
        reveal.set_reveal_child(open);
        arrow.set_label(if open { "expand_less" } else { "expand_more" });
    });
    row.add_controller(click);
    rows
}

pub(crate) fn toggle(card: &gtk::Box, icon: &str, title: &str, sub: &str, key: &'static str, on: bool) -> gtk::Box {
    let detail = controlled(key);
    let (row, end) = ui::row(Some(icon), title, detail.unwrap_or(sub));
    controlled_row(&row, key, sub);
    let on = on && detail.is_none();
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
    if kit::atomic::write(&file, body).is_err() {
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
        "appearance" => &[shell_look, app_look, roundness, window_buttons, app_themes, effects],
        "dock" => &[dock],
        "notch" => &[notch, menu_bar, workspaces],
        "desktop" => &[crate::wallpaper::section_for, desktop, screen_edges],
        "spotlight" => &[search, scopes, clipboard],
        "compositor" => &[session],
        "displays" => &[night_light],
        "power" => &[power],
        "screenshots" => &[screenshots],
        "recording" => &[recording_capture, recording_sound, recording_replay, recording_saving, recording_streaming, recording_shortcuts],
        "files" => &[files],
        "overview" => &[advanced],
        _ => &[],
    }
}

/// Builds a page made only of sections.
pub fn build(page: &str, title: &str, lead: &str) -> gtk::Widget {
    let (scroll, body) = ui::page(title, lead);
    let available = page != "recording" || needs(&body, &kit::optional::GPU_SCREEN_RECORDER, "Recording");
    let t = prefs::read();
    let notice = body.last_child();
    for section in sections(page) {
        section(&body, &t);
    }
    if !available {
        let mut child = notice.and_then(|w| w.next_sibling());
        while let Some(c) = child {
            child = c.next_sibling();
            if c.has_css_class("card") { c.set_sensitive(false); }
        }
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
pub(crate) fn shortcut(id: &str) -> gtk::Button {
    let b = ui::button(&keys(id).unwrap_or_else(|| "Off".into()), "");
    b.add_css_class("keycap");
    b.set_valign(gtk::Align::Center);
    b.set_tooltip_text(Some("Change in Keyboard"));
    let id = id.to_string();
    b.connect_clicked(move |_| {
        let label = kit::keybinds::all().into_iter().find(|b| b.id == id).map(|b| b.label).unwrap_or_default();
        crate::show(&format!("keyboard/{}", crate::search::slug(&label)));
    });
    b
}

pub(crate) fn section(body: &gtk::Box, title: &str) -> gtk::Box {
    body.append(&ui::heading(title));
    let card = ui::card();
    body.append(&card);
    card
}

/// How app icons are shaped in the dock, the app grid and the switcher.
fn icon_style(card: &gtk::Box, t: &toml::Table) {
    let style = match prefs::get_str(t, "dock_icons", "light").as_str() {
        "tile" => "graphite".to_string(),
        other => other.to_string(),
    };
    const STYLES: [(&str, &str); 5] = [("light", "Big Sur"), ("graphite", "Graphite"), ("mono", "Mono"), ("tinted", "Tinted"), ("native", "Original")];
    let apps = preview_apps();
    let items = STYLES.iter().map(|(v, name)| (*name, dock_icons(v, &apps))).collect();
    tiles(card, "style", "App icon style", "In the dock and the app grid. Big Sur shapes every app's icon alike; Original keeps each app's own", items, STYLES.iter().position(|s| s.0 == style).unwrap_or(0), |i| save("dock_icons", toml::Value::String(STYLES[i].0.into())));
}

fn shell_look(body: &gtk::Box, t: &toml::Table) {
    let look = section(body, "Shell");
    const SURFACES: [(&str, &str); 3] = [("black", "Black"), ("graphite", "Graphite"), ("tinted", "Tinted")];
    // "frosted", from older configs, reads as tinted.
    let current = match prefs::get_str(t, "surface", "black").as_str() {
        "frosted" => "tinted".to_string(),
        s => s.to_string(),
    };
    let at = SURFACES.iter().position(|s| s.0 == current).unwrap_or(0);
    // The same colors as App surfaces: panels are the window color and their tiles the cards.
    let items = SURFACES.iter().map(|(value, name)| (*name, kit::pictures::shell_surface(value).upcast())).collect();
    tiles(&look, "dark_mode", "Shell surfaces", "Notch, dock, menus and panels, in the same colors as App surfaces", items, at, |i| save("surface", toml::Value::String(SURFACES[i].0.into())));
    scheme_choice(&look, t);
    icon_theme_choice(&look, t);
    icon_style(&look, t);
    choice(&look, "palette", "Glyph backgrounds", "The colored squares behind the symbols in Settings, Files and Store", "ui_tiles", &kit::style::TILES, &prefs::get_str(t, "ui_tiles", "colorful"));
    cursor(&look, t);
}

/// A dropdown row with a small preview of the choice to its left, following the selection.
/// `pick` applies a choice and says whether it took; one that didn't puts the last one back.
fn previewed(card: &gtk::Box, icon: &str, title: &str, sub: &str, labels: &[&str], at: usize, preview: impl Fn(usize) -> gtk::Widget + 'static, pick: impl Fn(usize, &gtk::DropDown) -> bool + 'static) -> (gtk::Box, gtk::DropDown) {
    let (row, end) = ui::row(Some(icon), title, sub);
    let slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    slot.set_valign(gtk::Align::Center);
    slot.set_margin_end(6);
    slot.append(&preview(at));
    let pick_box = kit::ui::dropdown(labels, at as u32);
    let accepted = Rc::new(Cell::new(at as u32));
    let restoring = Rc::new(Cell::new(false));
    {
        let (accepted, restoring, slot) = (accepted.clone(), restoring.clone(), slot.downgrade());
        pick_box.connect_selected_notify(move |p| {
            if restoring.get() { return; }
            let i = p.selected();
            if pick(i as usize, p) {
                accepted.set(i);
            } else {
                restoring.set(true);
                p.set_selected(accepted.get());
                restoring.set(false);
            }
            if let Some(slot) = slot.upgrade() {
                while let Some(c) = slot.first_child() { slot.remove(&c); }
                slot.append(&preview(p.selected() as usize));
            }
        });
    }
    end.append(&slot);
    end.append(&pick_box);
    card.append(&row);
    (row, pick_box)
}

/// A scheme's look: three dots in its primary, secondary and tertiary colors on its surface.
fn scheme_swatch(colors: [(f64, f64, f64); 4]) -> gtk::Widget {
    let a = gtk::DrawingArea::new();
    a.set_content_width(58);
    a.set_content_height(24);
    a.set_draw_func(move |_, cr, w, h| {
        let (w, h) = (w as f64, h as f64);
        let [surface, x, y, z] = colors;
        cr.set_source_rgb(surface.0, surface.1, surface.2);
        kit::paint::rounded(cr, 0.0, 0.0, w, h, h / 2.0);
        let _ = cr.fill();
        for (i, c) in [x, y, z].iter().enumerate() {
            cr.set_source_rgb(c.0, c.1, c.2);
            cr.arc(h / 2.0 + i as f64 * (w - h) / 2.0, h / 2.0, h / 2.0 - 4.0, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
        }
    });
    a.upcast()
}

/// Color schemes, made from the wallpaper. While one is being made the choice waits; if it
/// can't be made, the last one comes back.
fn scheme_choice(card: &gtk::Box, t: &toml::Table) {
    let theme = kit::theme::load();
    let current = theme.as_ref().map(|t| t.scheme.clone()).unwrap_or_else(|| prefs::get_str(t, "scheme", "content"));
    let source = theme.map(|t| t.source).unwrap_or_else(|| "#6750a4".into());
    let at = kit::theme::SCHEMES.iter().position(|s| *s == current).unwrap_or(0);
    let colors: Vec<[(f64, f64, f64); 4]> = kit::theme::SCHEMES.iter().map(|s| kit::theme::preview(&source, s).map(|c| c.map(|c| (c.r, c.g, c.b))).unwrap_or([(0.15, 0.15, 0.17), (0.6, 0.5, 0.9), (0.6, 0.6, 0.7), (0.8, 0.6, 0.6)])).collect();
    let accepted = Rc::new(Cell::new(at as u32));
    let undoing = Rc::new(Cell::new(false));
    let (accepted2, undoing2) = (accepted.clone(), undoing.clone());
    previewed(card, "palette", "Color scheme", "Colors generated from your wallpaper", &kit::theme::SCHEMES, at, move |i| scheme_swatch(colors[i]), move |i, p| {
        if undoing2.get() { return true; }
        let Some(name) = kit::theme::SCHEMES.get(i) else { return false };
        let argv: [&std::ffi::OsStr; 4] = ["digit".as_ref(), "theme".as_ref(), "--scheme".as_ref(), name.as_ref()];
        let process = match gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE) {
            Ok(p) => p,
            Err(e) => {
                ui::fail(&format!("Could not apply the color scheme: {e}"));
                return false;
            }
        };
        p.set_sensitive(false);
        let (pick, accepted, undoing) = (p.clone(), accepted2.clone(), undoing2.clone());
        glib::spawn_future_local(async move {
            match process.wait_check_future().await {
                Ok(()) => {
                    accepted.set(i as u32);
                    ui::done(&format!("Color scheme: {name}"));
                }
                Err(e) => {
                    ui::fail(&format!("Could not apply the color scheme: {e}"));
                    undoing.set(true);
                    pick.set_selected(accepted.get());
                    undoing.set(false);
                }
            }
            pick.set_sensitive(true);
        });
        true
    });
}

/// Icon themes, with a few of the chosen one's icons beside the choice.
fn icon_theme_choice(card: &gtk::Box, t: &toml::Table) {
    let themes = icon_themes();
    let chosen = t.get("icon_theme").and_then(|v| v.as_str()).map(str::to_string);
    let mut names: Vec<String> = vec!["System default".into()];
    names.extend(themes.iter().cloned());
    let labels: Vec<&str> = names.iter().map(String::as_str).collect();
    let at = chosen.as_ref().and_then(|c| themes.iter().position(|n| n == c)).map_or(0, |i| i + 1);
    let shown = names.clone();
    previewed(card, "interests", "Icon theme", "Apps, the dock, the switcher and Spotlight", &labels, at, move |i| icon_preview(if i == 0 { "Adwaita" } else { &shown[i] }), move |i, _| {
        let r = if i == 0 { prefs::unset("icon_theme") } else { themes.get(i - 1).map_or(Ok(()), |n| prefs::set("icon_theme", toml::Value::String(n.clone()))) };
        match r {
            Ok(()) => true,
            Err(e) => {
                ui::fail(&e);
                false
            }
        }
    });
}

thread_local! {
    /// Icon themes made for previews, one per name: each new one reads its theme into memory.
    static THEMES: std::cell::RefCell<std::collections::HashMap<String, gtk::IconTheme>> = std::cell::RefCell::new(std::collections::HashMap::new());
}

/// The icon theme `name`, made once and kept for later previews.
fn named_theme(name: &str) -> Option<gtk::IconTheme> {
    let display = gtk::gdk::Display::default()?;
    Some(THEMES.with(|t| {
        t.borrow_mut().entry(name.to_string()).or_insert_with(|| {
            let active = gtk::IconTheme::for_display(&display);
            let it = gtk::IconTheme::new();
            it.set_search_path(&active.search_path().iter().map(|p| p.as_path()).collect::<Vec<_>>());
            it.set_resource_path(&active.resource_path().iter().map(|p| p.as_str()).collect::<Vec<_>>());
            it.set_theme_name(Some(name));
            it
        }).clone()
    }))
}

/// A folder, the home folder, a terminal and the trash from `theme`.
fn icon_preview(theme: &str) -> gtk::Widget {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let Some(it) = named_theme(theme) else { return row.upcast() };
    for name in ["folder", "user-home", "utilities-terminal", "user-trash"] {
        let icon = it.lookup_icon(name, &[], 22, 1, gtk::TextDirection::Ltr, gtk::IconLookupFlags::empty());
        let image = gtk::Image::from_paintable(Some(&icon));
        image.set_pixel_size(22);
        row.append(&image);
    }
    row.upcast()
}

/// A cursor theme's arrow, hand and text cursors.
fn cursor_preview(theme: &str) -> gtk::Widget {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    for names in [&["left_ptr", "default", "arrow"][..], &["hand2", "pointer", "pointing_hand"][..], &["xterm", "text", "ibeam"][..]] {
        if let Some(texture) = kit::cursors::image(theme, names, 24) {
            let picture = gtk::Picture::for_paintable(&texture);
            picture.set_size_request(22, 22);
            picture.set_content_fit(gtk::ContentFit::Contain);
            row.append(&picture);
        }
    }
    row.upcast()
}

/// The pointer's theme and size: "" leaves the system's theme alone.
fn cursor(card: &gtk::Box, t: &toml::Table) {
    const SIZES: [(i64, &str); 4] = [(24, "Small"), (32, "Medium"), (48, "Large"), (64, "Extra large")];
    let (current, size) = kit::cursors::current(t);
    let themes = kit::cursors::themes();
    if !themes.is_empty() {
        let mut options: Vec<(String, String)> = vec![(String::new(), "System default".into())];
        options.extend(themes.iter().cloned());
        // The default theme's own cursors stand for "System default".
        let system = gtk::Settings::default().and_then(|s| s.gtk_cursor_theme_name()).map(|n| n.to_string()).unwrap_or_else(|| "Adwaita".into());
        let labels: Vec<&str> = options.iter().map(|o| o.1.as_str()).collect();
        let at = options.iter().position(|(id, _)| *id == current).unwrap_or(0);
        let ids: Vec<String> = options.iter().map(|(id, _)| id.clone()).collect();
        let shown: Vec<String> = ids.iter().map(|id| if id.is_empty() { system.clone() } else { id.clone() }).collect();
        previewed(card, "arrow_selector_tool", "Cursor", "", &labels, at, move |i| cursor_preview(&shown[i]), move |i, _| save("cursor_theme", toml::Value::String(ids[i].clone())));
    }
    let labels: Vec<&str> = SIZES.iter().map(|s| s.1).collect();
    let (row, end) = ui::row(Some("open_with"), "Cursor size", "");
    let at = SIZES.iter().position(|s| s.0 >= size as i64).unwrap_or(SIZES.len() - 1);
    let pick = ui::dropdown(&labels, at as u32);
    let accepted = Cell::new(at as u32);
    let restoring = Cell::new(false);
    pick.connect_selected_notify(move |p| {
        if restoring.get() { return; }
        let Some((value, _)) = SIZES.get(p.selected() as usize) else { return };
        if save("cursor_size", toml::Value::Integer(*value)) {
            accepted.set(p.selected());
        } else {
            restoring.set(true);
            p.set_selected(accepted.get());
            restoring.set(false);
        }
    });
    end.append(&pick);
    card.append(&row);
}

fn app_look(body: &gtk::Box, t: &toml::Table) {
    let apps = section(body, "Apps");
    surfaces(&apps, "layers", "App surfaces", "Background of Digit apps and the Game Bar", &prefs::get_str(&t, "ui_surface", "black"));
    accent(&apps, &t);
    choice(&apps, "density_medium", "Density", "", "ui_density", &[("comfortable", "Comfortable"), ("compact", "Compact")], &prefs::get_str(&t, "ui_density", "comfortable"));
    toggle(&apps, "select_window", "Open Settings as a window", "Settings opens over everything by default; turn this on for an ordinary window. Takes effect the next time Settings opens", "settings_window", prefs::get_bool(&t, "settings_window", false));
}

fn app_themes(body: &gtk::Box, t: &toml::Table) {
    let card = section(body, "App themes");
    let gtk = toggle(&card, "palette", "Theme GTK apps", "Colors and theme for GTK apps", "theme_gtk", prefs::get_bool(t, "theme_gtk", false));
    // Build searchable rows immediately; discover packages and themes without blocking GTK.
    let gtk_choice = choice(&card, "palette", "GTK theme", "Digit follows your wallpaper and accent", "gtk_theme", &[("digit", "Digit")], "digit");
    show_when_on(&gtk, &gtk_choice.row);
    let qt = toggle(&card, "palette", "Theme Qt apps", "Needs qt6ct", "theme_qt", false);
    qt.set_sensitive(false);
    let qt_choice = choice(&card, "palette", "Qt theme", "Style for Qt apps", "qt_theme", &[], "Fusion");
    show_when_on(&qt, &qt_choice.row);
    let colors_choice = choice(&card, "palette", "Qt colors", "Digit follows your wallpaper and accent", "qt_colors", &[("digit", "Digit"), ("style", "Style default")], "digit");
    show_when_on(&qt, &colors_choice.row);
    let t = t.clone();
    glib::spawn_future_local(async move {
        let Ok((themes, styles, schemes, platform)) = gio::spawn_blocking(|| {
            let platform = kit::distro::qt_platform();
            (kit::app_themes::gtk_themes(&kit::app_themes::gtk_roots()), kit::app_themes::qt_styles(), kit::qt_colors::schemes(platform), platform)
        }).await else { return };
        let replace = |old: &gtk::Box, key, title, detail, names: Vec<String>, default: &str, digit: bool, source: &gtk::Box| {
            let mut options = Vec::new();
            if digit { options.push(("digit", "Digit")); }
            options.extend(names.iter().map(|n| (n.as_str(), n.as_str())));
            let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let selected = if digit { prefs::get_str(&t, key, default) } else { kit::app_themes::effective_qt_style(t.get(key).and_then(toml::Value::as_str), &names) };
            let next = choice(&holder, "palette", title, detail, key, &options, &selected);
            holder.remove(&next.row);
            card.insert_child_after(&next.row, old.prev_sibling().as_ref());
            card.remove(old);
            show_when_on(source, &next.row);
        };
        replace(&gtk_choice.row, "gtk_theme", "GTK theme", "Digit follows your wallpaper and accent", themes, "digit", true, &gtk);
        use kit::distro::QtPlatform;
        let detail = match platform { QtPlatform::Kde => "Includes KDE apps", QtPlatform::Qt6ct => "Install qt6ct-kde to include KDE apps", QtPlatform::Missing => "Needs qt6ct" };
        let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let next = toggle(&holder, "palette", "Theme Qt apps", detail, "theme_qt", platform != QtPlatform::Missing && prefs::get_bool(&t, "theme_qt", false));
        holder.remove(&next);
        card.insert_child_after(&next, qt.prev_sibling().as_ref());
        card.remove(&qt);
        next.set_sensitive(platform != QtPlatform::Missing);
        replace(&qt_choice.row, "qt_theme", "Qt theme", "Style for Qt apps", styles, "Fusion", false, &next);
        let mut options = vec![("digit", "Digit"), ("style", "Style default")];
        options.extend(schemes.iter().map(|s| (s.id.as_str(), s.name.as_str())));
        let selected = kit::qt_colors::effective(t.get("qt_colors").and_then(toml::Value::as_str), &schemes);
        let colors = choice(&holder, "palette", "Qt colors", "Digit follows your wallpaper and accent", "qt_colors", &options, selected);
        holder.remove(&colors.row);
        card.insert_child_after(&colors.row, colors_choice.row.prev_sibling().as_ref());
        card.remove(&colors_choice.row);
        show_when_on(&next, &colors.row);
    });
}

fn effects(body: &gtk::Box, t: &toml::Table) {
    let look = section(body, "Motion and effects");
    kit::motion::refresh();
    let motion_detail = if kit::motion::automatic() { "On automatically for software rendering" } else { "Turn off motion in Digit and compositor animations" };
    toggle(&look, "motion_photos_off", "Reduce motion", motion_detail, "reduce_motion", kit::motion::reduced());
    // The motion Reduce motion turns off, one by one.
    for row in [
        toggle(&look, "swipe_vertical", "Smooth scrolling", "Mouse wheel steps glide instead of jumping. Touchpads always scroll smoothly", "smooth_scroll", prefs::get_bool(&t, "smooth_scroll", true)),
        toggle(&look, "animation", "Compositor animations", "Windows opening and closing, and switching workspaces", "compositor_motion", prefs::get_bool(&t, "compositor_motion", true)),
        toggle(&look, "text_select_move_forward_character", "Smooth caret", "The caret in Spotlight glides as you type and blinks softly", "smooth_caret", prefs::get_bool(&t, "smooth_caret", true)),
        toggle(&look, "auto_awesome", "Startup animation", "Greet each start of the shell with a short full-screen animation", "intro", prefs::get_bool(&t, "intro", true)),
    ] {
        row.add_css_class("nested");
    }
    kit::effects::invalidate();
    let effects_detail = if kit::effects::automatic() { "On automatically for software rendering" } else { "No shadows, blur or frosted panels in digit and the compositor; smoother on older laptops" };
    toggle(&look, "speed", "Reduce effects", effects_detail, "reduced_effects", kit::effects::reduced());
}

/// Every roundness in one place: the apps', and the notch and screen corners that can follow it.
fn roundness(body: &gtk::Box, t: &toml::Table) {
    let card = section(body, "Corners");
    corners(&card, "rounded_corner", "Apps", "Cards, menus, buttons and dialogs", "ui_round", prefs::get_f64(t, "ui_round", 1.0));
    let legacy = prefs::get_f64(t, "screen_corners", 0.0);
    let screen_row = toggle(&card, "fit_screen", "Rounded screen corners", "Softens the corners of every display", "screen_round", prefs::get_bool(t, "screen_round", legacy >= 2.0));
    let screen_follows = prefs::get_bool(t, "screen_round_follow", t.contains_key("screen_round") || legacy < 2.0);
    let follows = prefs::get_bool(t, "notch_round_follow", true) && screen_follows;
    let (follow_row, end) = ui::row(Some("join"), "Notch and screen corners follow", "They round like the apps. Turn off to set their own below");
    let follow = gtk::Switch::new();
    follow.set_valign(gtk::Align::Center);
    ui::bind_switch(&follow, follows, |on| save_many(&[("notch_round_follow", toml::Value::Boolean(on)), ("screen_round_follow", toml::Value::Boolean(on))]));
    end.append(&follow);
    card.append(&follow_row);
    let notch_own = corners(&card, "toolbar", "Notch corners", "", "notch_round", prefs::get_f64(t, "notch_round", 1.0));
    let screen_own = slider(&card, "rounded_corner", "Screen corner radius", "", "screen_corners", (4.0, 24.0, 1.0), if legacy >= 4.0 { legacy.min(24.0) } else { 12.0 }, 12.0, 0);
    // Their own roundness only matters while they don't follow, and the screen's while it's on.
    if let Some(on) = find_switch(screen_row.upcast_ref()) {
        let sync = {
            let (on, follow) = (on.clone(), follow.clone());
            move || {
                notch_own.set_sensitive(!follow.is_active());
                screen_own.set_sensitive(on.is_active() && !follow.is_active());
            }
        };
        sync();
        let again = sync.clone();
        on.connect_active_notify(move |_| again());
        follow.connect_active_notify(move |_| sync());
    }
}

fn dock(body: &gtk::Box, t: &toml::Table) {
    let dock = section(body, "Dock");
    let shows = keys("dock").map(|k| format!("{k} shows or hides it")).unwrap_or_default();
    let visibility = choice(&dock, "dock_to_bottom", "Visibility", &shows, "dock_mode", &[("on", "Always shown"), ("autohide", "Auto-hide"), ("smart", "Hide when windows are open"), ("off", "Off")], &prefs::get_str(t, "dock_mode", "on"));
    // With the dock off, the rest has nothing to change.
    let rest = gtk::Box::new(gtk::Orientation::Vertical, 0);
    visibility.enable_when(&rest, |v| v != "off");
    dock.append(&rest);
    let dock = rest;
    choice(&dock, "side_navigation", "Position", "", "dock_side", &[("bottom", "Bottom"), ("left", "Left"), ("right", "Right")], &prefs::get_str(&t, "dock_side", "bottom"));
    slider(&dock, "photo_size_select_small", "Icon size", "", "dock_size", (32.0, 80.0, 2.0), prefs::get_f64(&t, "dock_size", 50.0), 50.0, 0);
    slider(&dock, "zoom_in", "Magnification", "1.0 turns it off", "dock_magnify", (1.0, 2.0, 0.05), prefs::get_f64(&t, "dock_magnify", 1.5), 1.5, 2);
    toggle(&dock, "apps", "Show running apps", "Apps that are open but not kept in the dock", "dock_running", prefs::get_bool(&t, "dock_running", true));
    // The Trash, and a hidden dock sliding in while something flies to it, as one choice.
    {
        let (row, end) = ui::row(Some("delete"), "Trash", "At the end of the dock. A hidden dock slides in while a deleted file flies to it");
        let sw = gtk::Switch::new();
        sw.set_valign(gtk::Align::Center);
        ui::bind_switch(&sw, prefs::get_bool(&t, "dock_trash", true), |on| save_many(&[("dock_trash", toml::Value::Boolean(on)), ("dock_trash_reveal", toml::Value::Boolean(on))]));
        end.append(&sw);
        dock.append(&row);
    }
    toggle(&dock, "apps", "Apps button", &kit::keybinds::shown("appgrid").map_or("At the start of the dock, opens all your apps".to_string(), |k| format!("At the start of the dock, opens all your apps; so does {k}")), "dock_apps", prefs::get_bool(t, "dock_apps", true));
    toggle(&dock, "history", "Recent files", "A stack of the files you used last", "dock_recent", prefs::get_bool(&t, "dock_recent", true));
    toggle(&dock, "notifications", "Notification badges", "Unread notification counts on icons", "dock_badges", prefs::get_bool(&t, "dock_badges", true));
}

/// Three installed apps whose own icons aren't already plates, so the styles visibly reshape
/// them (Steam, kitty, OBS…). Missing ones fall back to their icon names.
fn preview_apps() -> Vec<(String, gio::Icon)> {
    const WANTED: [&str; 10] = ["steam", "kitty", "com.obsproject.Studio", "vesktop", "discord", "blender", "obsidian", "org.kde.krita", "vlc", "org.gimp.GIMP"];
    let mut out: Vec<(String, gio::Icon)> = WANTED
        .iter()
        .filter_map(|id| {
            let info = gio_unix::DesktopAppInfo::new(&format!("{id}.desktop"))?;
            Some((id.to_string(), info.icon()?))
        })
        .take(3)
        .collect();
    for name in ["steam", "kitty", "vlc"] {
        if out.len() < 3 && !out.iter().any(|(id, _)| id == name) {
            out.push((name.to_string(), gio::ThemedIcon::new(name).upcast()));
        }
    }
    out
}

/// The dock's icons in `style`, drawn by the dock's own code, so the preview is what it shows.
fn dock_icons(style: &'static str, apps: &[(String, gio::Icon)]) -> gtk::Widget {
    const PX: i32 = 30;
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    row.add_css_class("tile-sky");
    row.set_halign(gtk::Align::Fill);
    let inner = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    inner.set_halign(gtk::Align::Center);
    inner.set_valign(gtk::Align::Center);
    inner.set_hexpand(true);
    row.append(&inner);
    let Some(display) = gtk::gdk::Display::default() else { return row.upcast() };
    let parsed = kit::art::Style::parse(style);
    // Styled icons start from the apps' original artwork, as the dock does.
    let theme = if parsed == kit::art::Style::Native { gtk::IconTheme::for_display(&display) } else { named_theme("Adwaita").unwrap_or_else(|| gtk::IconTheme::for_display(&display)) };
    let accent = kit::palette::Rgb::hex(&kit::style::ink().accent).unwrap_or(kit::palette::Rgb::new(0.55, 0.72, 0.95));
    for (id, icon) in apps {
        let paintable = theme.lookup_by_gicon(icon, PX, 1, gtk::TextDirection::Ltr, gtk::IconLookupFlags::empty());
        let image = match kit::art::texture(&inner, id, Some(paintable.upcast_ref()), id, PX as f64, parsed, accent, "Readex Pro") {
            Some(texture) => gtk::Image::from_paintable(Some(&texture)),
            None => gtk::Image::from_paintable(Some(&paintable)),
        };
        image.set_pixel_size(PX);
        inner.append(&image);
    }
    row.upcast()
}

pub fn session_notice(body: &gtk::Box, kind: kit::setup::Kind) {
    let card = ui::card();
    let (row, _) = ui::row(Some("info"), &format!("Log in to {} with Digit to change these", kind.name()), "Choose the session on your login screen");
    card.append(&row);
    body.append(&card);
}

fn window_buttons(body: &gtk::Box, t: &toml::Table) {
    let win = section(body, "Window buttons");
    toggle(&win, "remove", "Minimize button", "In Digit's own apps", "button_minimize", prefs::get_bool(t, "button_minimize", false));
    toggle(&win, "crop_square", "Maximize button", "In Digit's apps and other GTK apps", "button_maximize", prefs::get_bool(t, "button_maximize", false));
    toggle(&win, "close", "Close button", "In Digit's apps and other GTK apps", "button_close", prefs::get_bool(t, "button_close", true));
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
            let picture = kit::pictures::notch(key);
            picture.set_content_width(136);
            picture.set_content_height(72);
            (*name, picture.upcast())
        })
        .collect();
    tiles(&notch, "toolbar", "Style", "Notch hugs the top edge, Dynamic Island floats as a pill, Bar spans the whole edge with the app, status and date", items, at, |i| save("notch_style", toml::Value::String(STYLES[i].0.into())));
    choice(&notch, "call", "Call activity", "A live pill with the call timer and voice waveform while an app uses the microphone", "notch_call", &[("voice", "Voice and video apps"), ("any", "Any app using the microphone"), ("off", "Off")], &prefs::get_str(&t, "notch_call", "voice"));
    media_players(&notch, t);
    toggle(&notch, "mic_off", "Block idle microphone", "Voice apps like Discord can keep listening after a call ends. Digit silences only their microphone stream until a real call starts, then gives it back instantly", "mic_guard", prefs::get_bool(&t, "mic_guard", true));
    toggle(&notch, "touch_app", "Open on hover", "Expand the notch when the pointer rests on it", "hover", prefs::get_bool(&t, "hover", true));
    // One choice for every panel that opens from the notch, the corners and the date.
    const CLOSE: [&str; 4] = ["notch_close", "spaces_close", "control_close", "calendar_close"];
    let (row, end) = ui::row(Some("close_fullscreen"), "Panels close", "The open notch, Workspaces, Control Center and the calendar");
    let at = if prefs::get_str(t, "notch_close", "leave") == "click" { 1 } else { 0 };
    end.append(&ui::segmented(&["Pointer leaves", "Click elsewhere"], at, |i| {
        let value = if i == 1 { "click" } else { "leave" };
        save_many(&CLOSE.map(|k| (k, toml::Value::String(value.into()))))
    }));
    notch.append(&row);
}

/// Which media players the notch shows: all, all but a list, or only a list (see `kit::players`).
fn media_players(card: &gtk::Box, t: &toml::Table) {
    use kit::players::{ALLOWED, BLOCKED, MODE, MODES};
    let mode = choice(card, "music_note", "Media players", "Which players the notch, the Now playing widget and the media keys use", MODE, &MODES, &kit::players::mode(t));
    for (key, icon, title, sub, on) in [(BLOCKED, "music_off", "Hidden players", "Never shown, even while playing", "block"), (ALLOWED, "playlist_add_check", "Shown players", "Only these show. Leave empty to show all", "allow")] {
        let (row, end) = ui::row(Some(icon), title, sub);
        let entry = gtk::Entry::new();
        entry.set_text(&kit::players::join(&kit::prefs::list(t, key)));
        entry.set_placeholder_text(Some("spotify, firefox"));
        entry.set_valign(gtk::Align::Center);
        entry.set_width_chars(18);
        entry.set_max_length(256);
        ui::bind_entry(&entry, |_| Ok(()), move |text| save(key, toml::Value::Array(kit::players::parse(text).into_iter().map(toml::Value::String).collect())));
        end.append(&entry);
        card.append(&row);
        mode.enable_when(&row, move |v| v == on);
    }
}

fn menu_bar(body: &gtk::Box, t: &toml::Table) {
    let bar = section(body, "Menu bar");
    toggle(&bar, "menu", "App menus", "The focused app's name and menus at the top left. The logo, workspaces, status and date always show", "menubar", prefs::get_bool(&t, "menubar", true));
    const LOOKS: [(&str, &str); 3] = [("adaptive", "Adaptive"), ("clear", "Clear"), ("glass", "Glass")];
    let detail = controlled("menubar_look");
    let look = if detail.is_some() { "clear".to_string() } else { prefs::get_str(&t, "menubar_look", "adaptive") };
    let items = LOOKS.iter().map(|(v, name)| (*name, kit::pictures::menubar_look(v).upcast())).collect();
    let backdrop = tiles(&bar, "blur_on", "Menu bar backdrop", detail.unwrap_or("Adaptive stays clear over the wallpaper and turns to glass when windows are open"), items, LOOKS.iter().position(|l| l.0 == look).unwrap_or(0), |i| save("menubar_look", toml::Value::String(LOOKS[i].0.into())));
    controlled_row(&backdrop, "menubar_look", "Adaptive stays clear over the wallpaper and turns to glass when windows are open");
    choice(&bar, "deployed_code", "Menu bar logo", "The mark at the top left", "menubar_logo", &[("distro", "Distribution"), ("digit", "Digit")], &prefs::get_str(t, "menubar_logo", "distro"));
    slider(&bar, "height", "Bar height", "Used by the full-width bar", "wide_height", (28.0, 56.0, 1.0), prefs::get_f64(&t, "wide_height", 34.0), 34.0, 0);
    toggle(&bar, "widgets", "Menu bar app icons", "Shows the icons apps like Discord or Steam put in the system tray. Applies after digit restarts", "tray", prefs::get_bool(&t, "tray", true));
}

/// The workspace pill and the Workspaces panel.
fn workspaces(body: &gtk::Box, t: &toml::Table) {
    let card = section(body, "Workspaces");
    const SPACES: [(&str, &str); 3] = [("switch", "While switching"), ("always", "Always"), ("notch", "In the notch")];
    let current = prefs::get_str(t, "menubar_spaces", "switch");
    let items = SPACES.iter().map(|(v, name)| (*name, kit::pictures::menubar_spaces(v).upcast())).collect();
    tiles(&card, "workspaces", "Workspace pill", "The workspaces on this screen: beside the logo, or in the notch in place of the clock", items, SPACES.iter().position(|s| s.0 == current).unwrap_or(0), |i| save("menubar_spaces", toml::Value::String(SPACES[i].0.into())));
    let always = t.get("spaces_always").and_then(toml::Value::as_integer).unwrap_or(0).clamp(0, 10).to_string();
    choice(&card, "push_pin", "Always show", "Workspaces listed in the Workspaces panel even when empty", "spaces_always", &[("0", "Used only"), ("2", "1–2"), ("3", "1–3"), ("4", "1–4"), ("5", "1–5"), ("6", "1–6"), ("7", "1–7"), ("8", "1–8"), ("9", "1–9"), ("10", "1–10")], &always);
}

/// What each screen corner opens, set on a picture of a screen: each corner is a button that
/// opens a menu. A panel opens from its corner and lives in one corner at most, so choosing it
/// for another clears the old one, and the buttons show that right away. With several screens,
/// each has corners of its own, picked above the picture.
fn corners_picker(card: &gtk::Box, t: &toml::Table) {
    use kit::corners::{self, Corner, Panel};
    const PANELS: [(Panel, &str); 3] = [(Panel::None, "Nothing"), (Panel::Workspaces, "Workspaces"), (Panel::Control, "Control Center")];
    let name_of = |panel: Panel| PANELS.iter().find(|p| p.0 == panel).map_or("Nothing", |p| p.1);
    let screens: Vec<String> = kit::screens::all().iter().filter_map(|m| m.connector().map(|c| c.to_string())).collect();
    // One screen keeps the shared corners; with more, the one being set, the main one first.
    let picked: Rc<std::cell::RefCell<Option<String>>> = Rc::default();
    if screens.len() > 1 {
        let main = kit::screens::primary(&prefs::get_str(t, "primary_monitor", "")).map(|m| kit::screens::name(&m)).unwrap_or_default();
        *picked.borrow_mut() = Some(screens.iter().find(|s| **s == main).unwrap_or(&screens[0]).clone());
    }
    let read_now = {
        let picked = picked.clone();
        move |prefs: &toml::Table| match picked.borrow().as_deref() {
            Some(output) => corners::read_on(prefs, output),
            None => corners::read(prefs),
        }
    };
    let screen = gtk::Overlay::new();
    screen.set_halign(gtk::Align::Center);
    screen.set_margin_top(12);
    screen.set_margin_bottom(12);
    let picture = gtk::DrawingArea::new();
    picture.set_content_width(440);
    picture.set_content_height(248);
    picture.set_draw_func(|_, cr, w, h| {
        let (w, h) = (w as f64, h as f64);
        cr.set_source_rgb(0.1, 0.1, 0.13);
        kit::paint::rounded(cr, 0.0, 0.0, w, h, 14.0);
        let _ = cr.fill();
        cr.set_source_rgb(0.0, 0.0, 0.0);
        kit::paint::rounded(cr, (w - w * 0.22) / 2.0, -h * 0.06, w * 0.22, h * 0.12, h * 0.04);
        let _ = cr.fill();
    });
    screen.set_child(Some(&picture));
    let buttons: Rc<std::cell::RefCell<Vec<gtk::Label>>> = Rc::default();
    let now = read_now(t);
    for (slot, corner) in Corner::ALL.into_iter().enumerate() {
        let button = gtk::MenuButton::new();
        button.add_css_class("pill");
        button.set_always_show_arrow(false);
        button.set_halign(if corner.is_left() { gtk::Align::Start } else { gtk::Align::End });
        button.set_valign(if corner.is_top() { gtk::Align::Start } else { gtk::Align::End });
        button.set_margin_start(10);
        button.set_margin_end(10);
        button.set_margin_top(10);
        button.set_margin_bottom(10);
        let label = gtk::Label::new(Some(name_of(now[slot])));
        button.set_child(Some(&label));
        buttons.borrow_mut().push(label);
        let pop = gtk::Popover::new();
        pop.add_css_class("menu-pop");
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.add_css_class("pop-menu");
        for (panel, name) in PANELS {
            let item = gtk::Button::with_label(name);
            item.add_css_class("pop-item");
            if let Some(l) = item.child().and_downcast::<gtk::Label>() { l.set_xalign(0.0); }
            let (buttons, pop, picked, read_now) = (buttons.clone(), pop.downgrade(), picked.clone(), read_now.clone());
            item.connect_clicked(move |_| {
                let stored = prefs::read();
                let next = corners::assign(read_now(&stored), corner, panel);
                let saved = match picked.borrow().as_deref() {
                    Some(output) => saved(corners::save_monitor(output, &next)),
                    None => save_many(&corners::values(&next)),
                };
                // A failed save shows what is really stored again.
                let shown = if saved { next } else { read_now(&prefs::read()) };
                for (label, panel) in buttons.borrow().iter().zip(shown) {
                    label.set_text(name_of(panel));
                }
                if let Some(pop) = pop.upgrade() { pop.popdown(); }
            });
            list.append(&item);
        }
        pop.set_child(Some(&list));
        button.set_popover(Some(&pop));
        screen.add_overlay(&button);
    }
    if let Some(at) = picked.borrow().as_ref().and_then(|p| screens.iter().position(|s| s == p)) {
        let (row, end) = ui::row(Some("monitor"), "Screen", "Each screen has corners of its own");
        let labels: Vec<&str> = screens.iter().map(String::as_str).collect();
        let (picked, buttons, screens) = (picked.clone(), buttons.clone(), screens.clone());
        end.append(&ui::segmented(&labels, at, move |i| {
            *picked.borrow_mut() = Some(screens[i].clone());
            let now = read_now(&prefs::read());
            for (label, panel) in buttons.borrow().iter().zip(now) {
                label.set_text(name_of(panel));
            }
            true
        }));
        card.append(&row);
    }
    card.append(&screen);
}

fn screen_edges(body: &gtk::Box, t: &toml::Table) {
    let edge = section(body, "Hot corners");
    corners_picker(&edge, t);
    toggle(&edge, "ads_click", "Control Center by its button", "Clicking the bar button opens Control Center in the corner beside it. Its hot corner and shortcut still use the corner above", "control_follows_button", prefs::get_bool(t, "control_follows_button", true));
    toggle(&edge, "fullscreen", "Over fullscreen apps", "Let the dock edge and screen corners open panels while an app or game is fullscreen. Spotlight always opens", "fullscreen_edges", prefs::get_bool(&t, "fullscreen_edges", false));
}

fn power(body: &gtk::Box, t: &toml::Table) {
    let power = section(body, "Lock and power");
    let timeline = gtk::DrawingArea::new();
    timeline.set_content_height(72);
    timeline.set_hexpand(true);
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    holder.add_css_class("corners");
    holder.append(&timeline);
    power.append(&holder);
    let rows = [
        slider(&power, "lock_clock", "Lock after idle", "Minutes without input before the screen locks. 0 never locks on its own", "idle_lock", (0.0, 60.0, 1.0), prefs::get_f64(&t, "idle_lock", 0.0), 0.0, 0),
        slider(&power, "monitor", "Turn off displays", "Minutes without input. The screen dims softly for a few seconds first; any input cancels", "idle_off", (0.0, 60.0, 1.0), prefs::get_f64(&t, "idle_off", 10.0), 10.0, 0),
        slider(&power, "bedtime", "Sleep after idle", "Minutes without input before the computer sleeps. 0 never sleeps on its own", "idle_sleep", (0.0, 180.0, 5.0), prefs::get_f64(&t, "idle_sleep", 0.0), 0.0, 0),
    ];
    // What happens as the computer sits idle, on one line, following the sliders as they move.
    let scales: Vec<gtk::Scale> = rows.iter().filter_map(|r| find_scale(r.upcast_ref())).collect();
    if scales.len() == 3 {
        let read = {
            let scales = scales.clone();
            move || [scales[0].value(), scales[1].value(), scales[2].value()]
        };
        timeline.set_draw_func(move |a, cr, w, h| draw_idle(a, cr, w as f64, h as f64, read()));
        for sc in &scales {
            let a = timeline.downgrade();
            sc.connect_value_changed(move |_| if let Some(a) = a.upgrade() { a.queue_draw(); });
        }
    }
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

fn find_scale(w: &gtk::Widget) -> Option<gtk::Scale> {
    if let Some(s) = w.downcast_ref::<gtk::Scale>() { return Some(s.clone()); }
    let mut child = w.first_child();
    while let Some(c) = child {
        if let Some(s) = find_scale(&c) { return Some(s); }
        child = c.next_sibling();
    }
    None
}

/// The idle timeline: a line from now on, with the screen locking, turning off and the
/// computer sleeping marked where they happen. 0 minutes means never, so it isn't marked.
fn draw_idle(a: &gtk::DrawingArea, cr: &gtk::cairo::Context, w: f64, h: f64, [lock, off, sleep]: [f64; 3]) {
    let ink = a.color();
    let (r, g, b) = (ink.red() as f64, ink.green() as f64, ink.blue() as f64);
    #[allow(deprecated)]
    let accent = a.style_context().lookup_color("d_accent").map_or((0.55, 0.72, 0.95), |c| (c.red() as f64, c.green() as f64, c.blue() as f64));
    let events: Vec<(f64, &str, &str)> = [(lock, "lock", "Lock"), (off, "monitor", "Screen off"), (sleep, "bedtime", "Sleep")].into_iter().filter(|e| e.0 > 0.0).collect();
    let span = events.iter().map(|e| e.0).fold(30.0_f64, f64::max) * 1.1;
    let (x0, x1, y) = (14.0, w - 14.0, h * 0.5);
    let at = |m: f64| x0 + (x1 - x0) * (m / span);
    cr.set_source_rgba(r, g, b, 0.14);
    kit::paint::rounded(cr, x0, y - 2.0, x1 - x0, 4.0, 2.0);
    let _ = cr.fill();
    // Awake until the first thing happens.
    if let Some(first) = events.iter().map(|e| e.0).reduce(f64::min) {
        cr.set_source_rgb(accent.0, accent.1, accent.2);
        kit::paint::rounded(cr, x0, y - 2.0, at(first) - x0, 4.0, 2.0);
        let _ = cr.fill();
    }
    let label = |text: &str, x: f64, ty: f64, alpha: f64, size: f64| {
        let l = kit::paint::layout(cr, text, &format!("Readex Pro {size}"));
        let (_, ext) = l.pixel_extents();
        let tx = (x - ext.width() as f64 / 2.0).clamp(0.0, w - ext.width() as f64);
        cr.move_to(tx, ty);
        cr.set_source_rgba(r, g, b, alpha);
        kit::paint::show(cr, &l);
    };
    label("Now", x0, y + 13.0, 0.55, 10.0);
    if events.is_empty() {
        label("Nothing happens on its own", (x0 + x1) / 2.0, y + 13.0, 0.55, 10.0);
    }
    for (k, (m, icon, name)) in events.iter().enumerate() {
        let x = at(*m);
        cr.set_source_rgb(accent.0, accent.1, accent.2);
        cr.arc(x, y, 9.0, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
        let l = kit::paint::layout(cr, icon, &format!("{} 12", kit::style::glyph_family()));
        let (ink_ext, _) = l.pixel_extents();
        cr.move_to(x - ink_ext.width() as f64 / 2.0 - ink_ext.x() as f64, y - ink_ext.height() as f64 / 2.0 - ink_ext.y() as f64);
        cr.set_source_rgb(0.05, 0.05, 0.06);
        kit::paint::show(cr, &l);
        // Alternating below and above the line, so close ones don't run into each other.
        label(&format!("{name} · {} min", *m as i64), x, if k % 2 == 0 { y + 13.0 } else { y - 27.0 }, 0.85, 10.0);
    }
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
    field(&desk, "location_on", "Weather city", "Shown in Control Center, under the desktop clock and in the Weather widget. The city name goes to open-meteo.com over HTTPS; leave empty to keep weather off", "weather_city", &prefs::get_str(t, "weather_city", ""), "Istanbul", |_| true);
    choice(&desk, "device_thermostat", "Units", "", "weather_unit", &[("celsius", "°C"), ("fahrenheit", "°F")], &prefs::get_str(t, "weather_unit", "celsius"));
    toggle(&desk, "folder", "Desktop icons", "Show what's in your Desktop folder on the wallpaper", "desktop_icons", prefs::get_bool(t, "desktop_icons", true));
    toggle(&desk, "text_fields", "Draw widget text another way", "Can fix text that now and then disappears from the clock and the widgets, on some graphics setups. Leave it off unless that happens", "desktop_raster", prefs::get_bool(t, "desktop_raster", false));
}

/// A picker for the font `widget` draws its text in: the shell's own, or any installed family.
fn font_picker(widget: &'static str, current: &str) -> gtk::DropDown {
    let probe = gtk::Label::new(None);
    let mut families: Vec<String> = probe.pango_context().list_families().iter().map(|f| f.name().to_string()).filter(|n| !n.is_empty()).collect();
    families.sort_by_key(|n| n.to_lowercase());
    families.dedup();
    if !current.is_empty() && !families.iter().any(|f| f == current) {
        families.insert(0, current.to_string());
    }
    let mut labels = vec!["Digit's own"];
    labels.extend(families.iter().map(String::as_str));
    let at = families.iter().position(|f| f == current).map_or(0, |i| i + 1);
    let pick = kit::ui::dropdown(&labels, at as u32);
    pick.set_expression(Some(gtk::PropertyExpression::new(gtk::StringObject::static_type(), gtk::Expression::NONE, "string")));
    pick.set_enable_search(true);
    let accepted = Rc::new(Cell::new(at as u32));
    let restoring = Rc::new(Cell::new(false));
    pick.connect_selected_notify(move |p| {
        if restoring.get() { return; }
        let i = p.selected() as usize;
        let family = if i == 0 { "" } else { families.get(i - 1).map_or("", String::as_str) };
        if saved(kit::deskwidget::save_font(widget, family)) {
            accepted.set(p.selected());
        } else {
            restoring.set(true);
            p.set_selected(accepted.get());
            restoring.set(false);
        }
    });
    pick
}

type Rect = (f64, f64, f64, f64);

/// The places on the wallpaper a widget can take, as fractions of the screen.
const SPOTS: [(&str, &str, Rect); 5] = [
    ("top_left", "Top left", (0.04, 0.13, 0.29, 0.22)),
    ("top_center", "Top center", (0.355, 0.20, 0.29, 0.22)),
    ("top_right", "Top right", (0.67, 0.13, 0.29, 0.22)),
    ("bottom_left", "Bottom left", (0.04, 0.69, 0.29, 0.22)),
    ("bottom_right", "Bottom right", (0.67, 0.69, 0.29, 0.22)),
];

/// A widget on the wallpaper. To add one, add it here and to `kit::deskwidget::WIDGETS`: the
/// spots it can take (with the value its place stores for each) and how it looks on the small
/// screen.
struct DeskWidget {
    id: &'static str,
    name: &'static str,
    places: &'static [(&'static str, &'static str)],
    draw: fn(&gtk::cairo::Context, Rect, f64),
    /// What can be turned on or off for it, offered when its spot is clicked: setting, name and
    /// default. Its "_bg" setting puts it on a card.
    options: &'static [(&'static str, &'static str, bool)],
    /// The name its settings go under (see `kit::deskwidget`): its places and its color.
    ink: &'static str,
}

/// The four corners, each stored under its own name.
const CORNERS_ONLY: &[(&str, &str)] = &[("top_left", "top_left"), ("top_right", "top_right"), ("bottom_left", "bottom_left"), ("bottom_right", "bottom_right")];

const WIDGETS: [DeskWidget; 6] = [
    DeskWidget {
        id: "clock",
        name: "Clock",
        places: &[("top_left", "left"), ("top_center", "center")],
        draw: draw_clock,
        options: &[("desktop_clock_date", "Date", true), ("desktop_clock_weather", "Weather", true), ("desktop_clock_bg", "Background", false)],
        ink: "clock",
    },
    DeskWidget {
        id: "batteries",
        name: "Batteries",
        places: CORNERS_ONLY,
        draw: draw_batteries,
        options: &[("desktop_battery_levels", "Levels", true), ("desktop_battery_bg", "Background", false)],
        ink: "battery",
    },
    DeskWidget {
        id: "media",
        name: "Now playing",
        places: CORNERS_ONLY,
        draw: draw_media,
        options: &[("desktop_media_controls", "Player controls", false), ("desktop_media_bg", "Background", false)],
        ink: "media",
    },
    DeskWidget { id: "weather", name: "Weather", places: CORNERS_ONLY, draw: draw_weather, options: &[("desktop_weather_bg", "Background", false)], ink: "weather" },
    DeskWidget { id: "system", name: "System", places: CORNERS_ONLY, draw: draw_system, options: &[("desktop_system_bg", "Background", false)], ink: "system" },
    DeskWidget { id: "screentime", name: "Screen Time", places: CORNERS_ONLY, draw: draw_screentime, options: &[("desktop_screentime_bg", "Background", false)], ink: "screentime" },
];

/// Whether each widget sits on a card, from its "_bg" option.
fn cards(t: &toml::Table) -> Vec<bool> {
    WIDGETS.iter().map(|w| w.options.iter().find(|o| o.0.ends_with("_bg")).is_some_and(|o| prefs::get_bool(t, o.0, o.2))).collect()
}

/// Each widget's place as stored ("left", "top_right", "off"…) turned into its spot, or None
/// when it's off; `places` in `kit::deskwidget::WIDGETS` order.
fn placed(places: &[String; 6]) -> Vec<Option<&'static str>> {
    WIDGETS.iter().map(|w| {
        let i = kit::deskwidget::WIDGETS.iter().position(|n| *n == w.ink).unwrap_or(0);
        w.places.iter().find(|(_, v)| *v == places[i]).map(|(spot, _)| *spot)
    }).collect()
}

/// Each widget's size, in `WIDGETS` order (see `kit::deskwidget::SIZES`).
fn sizes(t: &toml::Table) -> Vec<f64> {
    WIDGETS.iter().map(|w| kit::deskwidget::size(t, w.ink)).collect()
}

/// Where a widget `k` times as big as designed sits on a box `w` by `h`, in the spot `id` at `r`:
/// grown away from the corner it keeps to, as on the desktop; at the top centre, both ways.
fn sized_rect(id: &str, r: Rect, k: f64, w: f64, h: f64) -> Rect {
    let (x, y, rw, rh) = (r.0 * w, r.1 * h, r.2 * w, r.3 * h);
    let (nw, nh) = (rw * k, rh * k);
    let nx = if id.ends_with("right") { x + rw - nw } else if id.ends_with("center") { x + (rw - nw) / 2.0 } else { x };
    (nx, if id.starts_with("bottom") { y + rh - nh } else { y }, nw, nh)
}

/// The point a widget in the spot `id` grows from, and its corner that resizes it, away from it.
fn grip(id: &str, (x, y, rw, rh): Rect) -> ((f64, f64), (f64, f64)) {
    let (right, bottom) = (id.ends_with("right"), id.starts_with("bottom"));
    let from = (if right { x + rw } else if id.ends_with("center") { x + rw / 2.0 } else { x }, if bottom { y + rh } else { y });
    (from, (if right { x } else { x + rw }, if bottom { y } else { y + rh }))
}

/// The cursor over the corner of a widget in the spot `id`.
fn grip_cursor(id: &str) -> &'static str {
    if id == "top_right" || id == "bottom_left" { "nesw-resize" } else { "nwse-resize" }
}

/// The size a widget `k0` times as big reaches when its corner, `from` its growing point, is
/// pulled by `(dx, dy)`: as far as the pull goes along the line from that point to the corner.
fn pulled(k0: f64, from: (f64, f64), corner: (f64, f64), (dx, dy): (f64, f64)) -> f64 {
    let (vx, vy) = (corner.0 - from.0, corner.1 - from.1);
    let len = vx * vx + vy * vy;
    if len < 1.0 { return k0; }
    kit::deskwidget::clamp_size(k0 * (vx * (vx + dx) + vy * (vy + dy)) / len)
}

/// How near a widget's corner a press takes it.
const GRIP: f64 = 9.0;

/// A widget's corner being pulled: the widget, its spot, its size when taken, and where it grows
/// from and its corner was then.
#[derive(Clone, Copy)]
struct Pull {
    widget: usize,
    spot: usize,
    from_k: f64,
    from: (f64, f64),
    corner: (f64, f64),
}

/// The desktop widgets on the screens' small maps (see `wallpaper::Overlay`): each screen has its
/// own places, set by clicking a spot on its map; what each widget shows and its color are the
/// same everywhere, so those actions are shared by every map.
pub(crate) struct DeskWidgets {
    primary: String,
    places: Rc<std::cell::RefCell<std::collections::BTreeMap<String, [String; 6]>>>,
    backed: Rc<std::cell::RefCell<Vec<bool>>>,
    /// Each widget's size, the same on every screen.
    sized: Rc<std::cell::RefCell<Vec<f64>>>,
    areas: Rc<std::cell::RefCell<Vec<glib::WeakRef<gtk::DrawingArea>>>>,
    group: gio::SimpleActionGroup,
}

impl DeskWidgets {
    /// `primary`: the main screen, whose places are the widgets' own settings.
    pub(crate) fn new(t: &toml::Table, primary: &str) -> Self {
        let backed = Rc::new(std::cell::RefCell::new(cards(t)));
        let areas: Rc<std::cell::RefCell<Vec<glib::WeakRef<gtk::DrawingArea>>>> = Rc::default();
        let group = gio::SimpleActionGroup::new();
        // Each widget's options, as checks in the menu of its spot.
        for (i, w) in WIDGETS.iter().enumerate() {
            for &(key, _, default) in w.options {
                let action = gio::SimpleAction::new_stateful(&option_action(key), None, &prefs::get_bool(t, key, default).to_variant());
                let (bk, areas) = (backed.clone(), areas.clone());
                action.connect_activate(move |action, _| {
                    let on = !action.state().and_then(|v| v.get::<bool>()).unwrap_or(default);
                    if !save(key, toml::Value::Boolean(on)) { return; }
                    action.set_state(&on.to_variant());
                    if key.ends_with("_bg") {
                        bk.borrow_mut()[i] = on;
                        redraw_all(&areas);
                    }
                });
                group.add_action(&action);
            }
        }
        // Each widget's color, picked from the same menu.
        for w in &WIDGETS {
            let key = kit::deskwidget::key(w.ink);
            let now = prefs::get_str(t, &key, "auto");
            let action = gio::SimpleAction::new_stateful(&format!("ink-{}", w.ink), Some(glib::VariantTy::STRING), &now.to_variant());
            action.connect_activate(move |action, param| {
                let Some(value) = param.and_then(|p| p.get::<String>()) else { return };
                if !save(&key, toml::Value::String(value.clone())) { return; }
                action.set_state(&value.to_variant());
            });
            group.add_action(&action);
        }
        // Each widget's card tone, from the same menu: it shows only with a background.
        for w in &WIDGETS {
            let now = match kit::deskwidget::card(t, w.ink) { kit::deskwidget::Card::Light => "light", kit::deskwidget::Card::Dark => "dark", kit::deskwidget::Card::Auto => "auto" };
            let action = gio::SimpleAction::new_stateful(&format!("card-{}", w.ink), Some(glib::VariantTy::STRING), &now.to_variant());
            let widget = w.ink;
            action.connect_activate(move |action, param| {
                let Some(value) = param.and_then(|p| p.get::<String>()) else { return };
                if !saved(kit::deskwidget::save_card(widget, &value)) { return; }
                action.set_state(&value.to_variant());
            });
            group.add_action(&action);
        }
        Self { primary: primary.to_string(), places: Rc::default(), backed, sized: Rc::new(std::cell::RefCell::new(sizes(t))), areas, group }
    }

    /// The widgets on the map of the screen `output`: click a spot to choose what sits there,
    /// and what the widget there shows.
    pub(crate) fn overlay(&self, t: &toml::Table, output: &str) -> crate::wallpaper::Overlay {
        use crate::wallpaper::Overlay;
        let main = output == self.primary;
        self.places.borrow_mut().insert(output.to_string(), kit::deskwidget::places(t, output, main));
        let output = output.to_string();
        let (places, backed, sized, areas) = (self.places.clone(), self.backed.clone(), self.sized.clone(), self.areas.clone());
        let hover: Rc<Cell<Option<usize>>> = Rc::default();
        let pull: Rc<Cell<Option<Pull>>> = Rc::default();
        let spot = Rc::new(Cell::new(0usize));
        let put = gio::SimpleAction::new_stateful("put", Some(glib::VariantTy::STRING), &"none".to_variant());
        let here_now = {
            let (places, output) = (places.clone(), output.clone());
            move || placed(places.borrow().get(&output).unwrap_or(&std::array::from_fn(|_| "off".to_string())))
        };
        // Each widget that is out: its spot, and where it is drawn at its size.
        let shown = {
            let (now, sized) = (here_now.clone(), sized.clone());
            move |w: f64, h: f64| -> Vec<(usize, usize, Rect)> {
                let sized = sized.borrow();
                now().iter().enumerate().filter_map(|(i, at)| {
                    let s = SPOTS.iter().position(|(id, _, _)| Some(*id) == *at)?;
                    Some((i, s, sized_rect(SPOTS[s].0, SPOTS[s].2, sized[i], w, h)))
                }).collect()
            }
        };
        // The spot under a place in the box: a widget's, as large as it is drawn, else an empty one.
        let spot_at = {
            let shown = shown.clone();
            move |x: f64, y: f64, w: f64, h: f64| {
                let within = |(rx, ry, rw, rh): Rect| x >= rx && x <= rx + rw && y >= ry && y <= ry + rh;
                shown(w, h).into_iter().find(|(_, _, r)| within(*r)).map(|(_, s, _)| s).or_else(|| {
                    SPOTS.iter().position(|(id, _, r)| WIDGETS.iter().any(|wd| wd.places.iter().any(|p| p.0 == *id)) && within((r.0 * w, r.1 * h, r.2 * w, r.3 * h)))
                })
            }
        };
        // The widget whose corner is at a place in the box.
        let grip_at = {
            let shown = shown.clone();
            move |x: f64, y: f64, w: f64, h: f64| {
                shown(w, h).into_iter().find(|(_, s, r)| {
                    let (_, corner) = grip(SPOTS[*s].0, *r);
                    (x - corner.0).abs() <= GRIP && (y - corner.1).abs() <= GRIP
                })
            }
        };
        let on_grip = Rc::new(Cell::new(false));
        let draw: crate::wallpaper::DrawFn = {
            let (now, hv, bk, sz, pl, og) = (here_now.clone(), hover.clone(), backed.clone(), sized.clone(), pull.clone(), on_grip.clone());
            Box::new(move |cr, w, h| {
                // The corner shows on the widget the pointer is over, and on the one being resized.
                let gripped = pl.get().map(|p| p.spot).or_else(|| hv.get());
                draw_desk_map(cr, w, h, &now(), &bk.borrow(), &sz.borrow(), hv.get(), gripped.map(|s| (s, og.get() || pl.get().is_some())), pl.get().map(|p| sz.borrow()[p.widget]));
            })
        };
        let moved: crate::wallpaper::MovedFn = {
            let (hv, og, spot_at, grip_at) = (hover.clone(), on_grip.clone(), spot_at.clone(), grip_at.clone());
            Box::new(move |at, w, h| {
                let corner = at.and_then(|(x, y)| grip_at(x, y, w, h));
                let now = corner.map(|(_, s, _)| s).or_else(|| at.and_then(|(x, y)| spot_at(x, y, w, h)));
                let redraw = hv.replace(now) != now || og.replace(corner.is_some()) != corner.is_some();
                (redraw, corner.map(|(_, s, _)| grip_cursor(SPOTS[s].0)).or(now.map(|_| "pointer")))
            })
        };
        let grab: crate::wallpaper::GrabFn = {
            let (pl, sz) = (pull.clone(), sized.clone());
            Box::new(move |(x, y), w, h| {
                let Some((widget, spot, r)) = grip_at(x, y, w, h) else { return false };
                let (from, corner) = grip(SPOTS[spot].0, r);
                pl.set(Some(Pull { widget, spot, from_k: sz.borrow()[widget], from, corner }));
                true
            })
        };
        let drag: Box<dyn Fn(f64, f64)> = {
            let (pl, sz, areas) = (pull.clone(), sized.clone(), areas.clone());
            Box::new(move |dx, dy| {
                let Some(p) = pl.get() else { return };
                let k = pulled(p.from_k, p.from, p.corner, (dx, dy));
                if sz.borrow()[p.widget] != k {
                    sz.borrow_mut()[p.widget] = k;
                    redraw_all(&areas);
                }
            })
        };
        // The size is kept when the corner is let go; one that can't be saved goes back.
        let drop: Box<dyn Fn()> = {
            let (pl, sz, areas) = (pull.clone(), sized.clone(), areas.clone());
            Box::new(move || {
                let Some(p) = pl.take() else { return };
                let k = sz.borrow()[p.widget];
                if k != p.from_k && !saved(kit::deskwidget::save_size(WIDGETS[p.widget].ink, k)) {
                    sz.borrow_mut()[p.widget] = p.from_k;
                }
                redraw_all(&areas);
            })
        };
        // What a spot holds is picked from a menu at the spot.
        {
            let (places, spot, areas, output) = (places.clone(), spot.clone(), areas.clone(), output.clone());
            put.connect_activate(move |action, param| {
                let Some(choice) = param.and_then(|p| p.get::<String>()) else { return };
                let here = SPOTS[spot.get()].0;
                let mut next = places.borrow().get(&output).cloned().unwrap_or_else(|| std::array::from_fn(|_| "off".to_string()));
                for w in &WIDGETS {
                    let i = kit::deskwidget::WIDGETS.iter().position(|n| *n == w.ink).unwrap_or(0);
                    let at = w.places.iter().find(|(_, v)| *v == next[i]).map(|(s, _)| *s);
                    if w.id == choice {
                        next[i] = w.places.iter().find(|p| p.0 == here).map_or("off", |p| p.1).to_string();
                    } else if at == Some(here) {
                        // One widget to a spot: whatever was here makes way.
                        next[i] = "off".into();
                    }
                }
                // The main screen's places are the widgets' own settings; another screen's go in
                // its own table, the others kept.
                let saved = if main {
                    let values: Vec<(&str, toml::Value)> = kit::deskwidget::WIDGETS.iter().zip(next.iter()).map(|(w, v)| (kit::deskwidget::place_key(w), toml::Value::String(v.clone()))).collect();
                    save_many(&values)
                } else {
                    saved(kit::deskwidget::save_screen(&output, &next))
                };
                if !saved { return; }
                places.borrow_mut().insert(output.clone(), next);
                action.set_state(&choice.to_variant());
                redraw_all(&areas);
            });
        }
        let install: Box<dyn Fn(&gtk::DrawingArea)> = {
            let (areas, group) = (areas.clone(), self.group.clone());
            Box::new(move |area| {
                areas.borrow_mut().push(area.downgrade());
                area.insert_action_group("desk", Some(&group));
            })
        };
        let click: crate::wallpaper::ClickFn = {
            let (now, put, spot) = (here_now, put.clone(), spot.clone());
            Box::new(move |area, (ox, oy), (w, h), (x, y)| {
                let Some(i) = spot_at(x, y, w, h) else { return };
                spot.set(i);
                let (id, _, base) = SPOTS[i];
                // The popover points at the widget as large as it is drawn.
                let r = shown(w, h).into_iter().find(|(_, s, _)| *s == i).map_or((base.0 * w, base.1 * h, base.2 * w, base.3 * h), |(_, _, r)| r);
                let here = WIDGETS.iter().zip(now().iter()).find(|(_, at)| **at == Some(id)).map_or("none", |(wd, _)| wd.id);
                put.set_state(&here.to_variant());
                // What can sit here on the left; what the one here shows on the right, beside it.
                let pop = gtk::Popover::new();
                pop.add_css_class("menu-pop");
                pop.set_parent(area);
                let both = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
                list.add_css_class("pop-menu");
                let entries = WIDGETS.iter().filter(|wd| wd.places.iter().any(|p| p.0 == id)).map(|wd| (wd.id, wd.name)).chain([("none", "Nothing")]);
                for (choice, name) in entries {
                    let item = gtk::Button::new();
                    item.add_css_class("pop-item");
                    let line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                    let label = gtk::Label::new(Some(name));
                    label.set_xalign(0.0);
                    label.set_hexpand(true);
                    line.append(&label);
                    let mark = gtk::Label::new(Some("check"));
                    mark.add_css_class("sym");
                    mark.set_opacity(if choice == here { 1.0 } else { 0.0 });
                    line.append(&mark);
                    item.set_child(Some(&line));
                    let (put, pop) = (put.clone(), pop.downgrade());
                    item.connect_clicked(move |_| {
                        put.activate(Some(&choice.to_variant()));
                        if let Some(pop) = pop.upgrade() { pop.popdown(); }
                    });
                    list.append(&item);
                }
                both.append(&list);
                if let Some(wd) = WIDGETS.iter().find(|wd| wd.id == here) {
                    both.append(&gtk::Separator::new(gtk::Orientation::Vertical));
                    let panel = gtk::Box::new(gtk::Orientation::Vertical, 2);
                    panel.add_css_class("pop-panel");
                    panel.set_valign(gtk::Align::Start);
                    let title = gtk::Label::new(Some(wd.name));
                    title.add_css_class("heading");
                    title.set_xalign(0.0);
                    panel.append(&title);
                    for (key, name, _) in wd.options {
                        let check = gtk::CheckButton::with_label(name);
                        check.set_action_name(Some(&format!("desk.{}", option_action(key))));
                        panel.append(&check);
                    }
                    // Automatic follows the wallpaper; the others hold whatever is behind.
                    let color = gtk::Label::new(Some("Color"));
                    color.add_css_class("heading");
                    color.set_xalign(0.0);
                    panel.append(&color);
                    let mut first: Option<gtk::CheckButton> = None;
                    for (value, name) in kit::deskwidget::CHOICES {
                        let check = gtk::CheckButton::with_label(name);
                        check.set_action_name(Some(&format!("desk.ink-{}", wd.ink)));
                        check.set_action_target_value(Some(&value.to_variant()));
                        match &first {
                            Some(f) => check.set_group(Some(f)),
                            None => first = Some(check.clone()),
                        }
                        panel.append(&check);
                    }
                    let font = gtk::Label::new(Some("Font"));
                    font.add_css_class("heading");
                    font.set_xalign(0.0);
                    panel.append(&font);
                    let pick = font_picker(wd.ink, &kit::deskwidget::font(&prefs::read(), wd.ink));
                    pick.set_halign(gtk::Align::Start);
                    panel.append(&pick);
                    // Light or dark under the widget, when it has a background.
                    let tone = gtk::Label::new(Some("Background color"));
                    tone.add_css_class("heading");
                    tone.set_xalign(0.0);
                    panel.append(&tone);
                    let mut first: Option<gtk::CheckButton> = None;
                    for (value, name) in kit::deskwidget::CARD_TONES {
                        let check = gtk::CheckButton::with_label(name);
                        check.set_action_name(Some(&format!("desk.card-{}", wd.ink)));
                        check.set_action_target_value(Some(&value.to_variant()));
                        match &first {
                            Some(f) => check.set_group(Some(f)),
                            None => first = Some(check.clone()),
                        }
                        panel.append(&check);
                    }
                    both.append(&panel);
                }
                pop.set_child(Some(&both));
                pop.set_has_arrow(false);
                pop.set_pointing_to(Some(&gtk::gdk::Rectangle::new((ox + r.0) as i32, (oy + r.1) as i32, r.2 as i32, r.3 as i32)));
                pop.connect_closed(|p| {
                    let p = p.clone();
                    glib::idle_add_local_once(move || p.unparent());
                });
                pop.popup();
            })
        };
        Overlay { draw, moved, grab, drag, drop, click, install }
    }
}

fn redraw_all(areas: &std::cell::RefCell<Vec<glib::WeakRef<gtk::DrawingArea>>>) {
    areas.borrow_mut().retain(|a| a.upgrade().inspect(|a| a.queue_draw()).is_some());
}

fn draw_clock(cr: &gtk::cairo::Context, (x, y, rw, rh): Rect, alpha: f64) {
    let l = kit::paint::layout(cr, "12:30", &format!("Readex Pro Light {}", (rh * 0.46).round()));
    let (_, ext) = l.pixel_extents();
    let tx = x + (rw - ext.width() as f64) / 2.0;
    cr.move_to(tx - ext.x() as f64, y + rh * 0.1 - ext.y() as f64);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.95 * alpha);
    kit::paint::show(cr, &l);
    let line = rw * 0.42;
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.7 * alpha);
    kit::paint::rounded(cr, x + (rw - line) / 2.0, y + rh * 0.1 + ext.height() as f64 + rh * 0.08, line, rh * 0.07, rh * 0.035);
    let _ = cr.fill();
}

/// Where the `i`th of `n` rings of radius `rad` sits: packed together and centred, as on the
/// desktop, rather than spread over the whole spot.
fn ring_x(x: f64, rw: f64, rad: f64, n: usize, i: usize) -> f64 {
    let step = rad * 2.7;
    x + rw / 2.0 - step * (n as f64 - 1.0) / 2.0 + step * i as f64
}

fn draw_batteries(cr: &gtk::cairo::Context, (x, y, rw, rh): Rect, alpha: f64) {
    let rad = rh * 0.24;
    cr.set_line_width(rh * 0.08);
    for (i, level) in [0.8, 0.55, 0.3].iter().enumerate() {
        let (cx, cy) = (ring_x(x, rw, rad, 3, i), y + rh * 0.38);
        blob(cr, cx - rh * 0.08, y + rh * 0.76, rh * 0.16, rh * 0.06, 0.7 * alpha);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.25 * alpha);
        cr.arc(cx, cy, rad, 0.0, std::f64::consts::TAU);
        let _ = cr.stroke();
        cr.set_source_rgba(0.19, 0.82, 0.35, alpha);
        let start = -std::f64::consts::FRAC_PI_2;
        cr.arc(cx, cy, rad, start, start + std::f64::consts::TAU * level);
        let _ = cr.stroke();
    }
}

/// The menu action of a widget option: its setting, as action names allow.
fn option_action(key: &str) -> String {
    format!("opt-{}", key.replace('_', "-"))
}

/// A white shape on the small screen.
fn blob(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, alpha: f64) {
    cr.set_source_rgba(1.0, 1.0, 1.0, alpha);
    kit::paint::rounded(cr, x, y, w, h, w.min(h) / 2.0);
    let _ = cr.fill();
}

fn draw_media(cr: &gtk::cairo::Context, (x, y, rw, rh): Rect, alpha: f64) {
    let a = rh * 0.62;
    let (ax, ay) = (x + rw * 0.06, y + (rh - a) / 2.0);
    let g = gtk::cairo::LinearGradient::new(ax, ay, ax + a, ay + a);
    g.add_color_stop_rgba(0.0, 0.93, 0.36, 0.30, alpha);
    g.add_color_stop_rgba(1.0, 0.98, 0.78, 0.40, alpha);
    kit::paint::rounded(cr, ax, ay, a, a, a * 0.2);
    let _ = cr.set_source(&g);
    let _ = cr.fill();
    let tx = ax + a + rw * 0.07;
    let tw = x + rw * 0.94 - tx;
    blob(cr, tx, ay + a * 0.2, tw * 0.8, rh * 0.08, 0.95 * alpha);
    blob(cr, tx, ay + a * 0.42, tw * 0.5, rh * 0.06, 0.6 * alpha);
    blob(cr, tx, ay + a * 0.68, tw, rh * 0.035, 0.3 * alpha);
    blob(cr, tx, ay + a * 0.68, tw * 0.6, rh * 0.035, 0.9 * alpha);
}

fn draw_weather(cr: &gtk::cairo::Context, (x, y, rw, rh): Rect, alpha: f64) {
    // A sun behind a cloud, with the temperature and the day's range beside it.
    let (cx, cy, r) = (x + rw * 0.14, y + rh * 0.5, rh * 0.13);
    cr.set_source_rgba(1.0, 0.82, 0.35, alpha);
    cr.arc(cx + r * 0.5, cy - r * 0.4, r, 0.0, std::f64::consts::TAU);
    let _ = cr.fill();
    blob(cr, cx - r * 1.1, cy, r * 2.6, r * 1.2, 0.95 * alpha);
    let l = kit::paint::layout(cr, "11°", &format!("Readex Pro Light {}", (rh * 0.36).round()));
    let (_, ext) = l.pixel_extents();
    cr.move_to(x + rw * 0.3 - ext.x() as f64, cy - ext.height() as f64 / 2.0 - ext.y() as f64);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.95 * alpha);
    kit::paint::show(cr, &l);
    let rx = x + rw * 0.3 + ext.width() as f64 + rw * 0.06;
    blob(cr, rx, cy - rh * 0.1, rw * 0.22, rh * 0.06, 0.85 * alpha);
    blob(cr, rx, cy + rh * 0.04, rw * 0.16, rh * 0.05, 0.55 * alpha);
}

fn draw_screentime(cr: &gtk::cairo::Context, (x, y, rw, rh): Rect, alpha: f64) {
    // A badge and the total, columns over the day and a list of apps at the right.
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.2 * alpha);
    cr.arc(x + rw * 0.1, y + rh * 0.24, rh * 0.11, 0.0, std::f64::consts::TAU);
    let _ = cr.fill();
    blob(cr, x + rw * 0.2, y + rh * 0.17, rw * 0.2, rh * 0.1, 0.95 * alpha);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.22 * alpha);
    for i in 0..3 {
        cr.rectangle(x + rw * 0.05, y + rh * (0.5 + 0.22 * i as f64), rw * 0.6, 1.0);
        let _ = cr.fill();
    }
    for (i, level) in [0.0, 0.1, 0.5, 0.8, 0.3, 0.0, 0.4, 0.6, 0.2].iter().enumerate() {
        let bh = rh * 0.4 * level;
        cr.set_source_rgba(0.04, 0.52, 1.0, alpha);
        kit::paint::rounded(cr, x + rw * (0.07 + 0.06 * i as f64), y + rh * 0.94 - bh, rw * 0.035, bh.max(0.0), 1.0);
        let _ = cr.fill();
    }
    for i in 0..4 {
        let ry = y + rh * (0.12 + 0.24 * i as f64);
        blob(cr, x + rw * 0.76, ry, rw * 0.06, rh * 0.12, 0.8 * alpha);
        blob(cr, x + rw * 0.85, ry + rh * 0.03, rw * 0.1, rh * 0.05, 0.5 * alpha);
    }
}

fn draw_system(cr: &gtk::cairo::Context, (x, y, rw, rh): Rect, alpha: f64) {
    let rad = rh * 0.2;
    cr.set_line_width(rh * 0.07);
    for (i, level) in [0.46, 0.62, 0.3, 0.55].iter().enumerate() {
        let (cx, cy) = (ring_x(x, rw, rad, 4, i), y + rh * 0.42);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.25 * alpha);
        cr.arc(cx, cy, rad, 0.0, std::f64::consts::TAU);
        let _ = cr.stroke();
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.95 * alpha);
        let start = -std::f64::consts::FRAC_PI_2;
        cr.arc(cx, cy, rad, start, start + std::f64::consts::TAU * level);
        let _ = cr.stroke();
        blob(cr, cx - rh * 0.09, y + rh * 0.78, rh * 0.18, rh * 0.06, 0.7 * alpha);
    }
}

/// The small screen: the wallpaper, the notch, every widget in its spot at its size and the
/// empty spots, dashed with a plus. `grip`: the spot whose widget shows its resizing corner, and
/// whether that corner is lit; `pulling`: the size of the widget being resized, shown by it.
#[allow(clippy::too_many_arguments)]
fn draw_desk_map(cr: &gtk::cairo::Context, w: f64, h: f64, placed: &[Option<&'static str>], backed: &[bool], sizes: &[f64], hover: Option<usize>, grip: Option<(usize, bool)>, pulling: Option<f64>) {
    let nw = w * 0.25;
    cr.set_source_rgb(0.0, 0.0, 0.0);
    kit::paint::rounded(cr, (w - nw) / 2.0, -h * 0.06, nw, h * 0.125, h * 0.045);
    let _ = cr.fill();
    for (i, (id, _, r)) in SPOTS.iter().enumerate() {
        if !WIDGETS.iter().any(|wd| wd.places.iter().any(|p| p.0 == *id)) { continue; }
        // Two widgets can share a spot from older settings: the second sits under the first.
        let here: Vec<(&DeskWidget, bool, f64)> = WIDGETS.iter().zip(placed.iter()).zip(backed.iter()).zip(sizes.iter()).filter(|(((_, at), _), _)| **at == Some(*id)).map(|(((wd, _), card), k)| (wd, *card, *k)).collect();
        let rect = sized_rect(id, *r, here.first().map_or(1.0, |h| h.2), w, h);
        let hot = hover == Some(i);
        if hot {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.16);
            kit::paint::rounded(cr, rect.0, rect.1, rect.2, rect.3, rect.3 * 0.22);
            let _ = cr.fill();
        }
        if here.is_empty() {
            cr.set_source_rgba(1.0, 1.0, 1.0, if hot { 0.95 } else { 0.6 });
            cr.set_line_width(1.0);
            cr.set_dash(&[3.0, 3.0], 0.0);
            kit::paint::rounded(cr, rect.0 + 0.5, rect.1 + 0.5, rect.2 - 1.0, rect.3 - 1.0, rect.3 * 0.22);
            let _ = cr.stroke();
            cr.set_dash(&[], 0.0);
            let (cx, cy, arm) = (rect.0 + rect.2 / 2.0, rect.1 + rect.3 / 2.0, rect.3 * 0.16);
            cr.set_line_width(1.4);
            cr.move_to(cx - arm, cy);
            cr.line_to(cx + arm, cy);
            cr.move_to(cx, cy - arm);
            cr.line_to(cx, cy + arm);
            let _ = cr.stroke();
        }
        for (k, (wd, card, _)) in here.iter().enumerate() {
            let dy = k as f64 * (rect.3 + h * 0.03);
            if *card {
                cr.set_source_rgba(0.08, 0.08, 0.1, 0.42);
                kit::paint::rounded(cr, rect.0, rect.1 + dy, rect.2, rect.3, rect.3 * 0.24);
                let _ = cr.fill();
            }
            (wd.draw)(cr, (rect.0, rect.1 + dy, rect.2, rect.3), 1.0);
        }
        if let Some((_, lit)) = grip.filter(|(s, _)| *s == i && !here.is_empty()) {
            draw_grip(cr, id, rect, lit);
            if let Some(k) = pulling {
                draw_percent(cr, rect, k);
            }
        }
    }
}

/// A widget's resizing corner on the small screen: a bracket at its corner, dark under light so
/// it reads on any picture, as the picture's own zoom corner.
fn draw_grip(cr: &gtk::cairo::Context, id: &str, rect: Rect, lit: bool) {
    let (_, (cx, cy)) = grip(id, rect);
    // The bracket's arms run back along the widget's edges from its corner.
    let (sx, sy) = (if id.ends_with("right") { 1.0 } else { -1.0 }, if id.starts_with("bottom") { 1.0 } else { -1.0 });
    let len = 9.0;
    cr.set_line_cap(gtk::cairo::LineCap::Round);
    cr.set_line_join(gtk::cairo::LineJoin::Round);
    for (width, (r, g, b, a)) in [(4.5, (0.0, 0.0, 0.0, 0.35)), (2.25, (1.0, 1.0, 1.0, if lit { 1.0 } else { 0.8 }))] {
        cr.set_line_width(width);
        cr.set_source_rgba(r, g, b, a);
        cr.move_to(cx + sx * len, cy);
        cr.line_to(cx, cy);
        cr.line_to(cx, cy + sy * len);
        let _ = cr.stroke();
    }
}

/// The size of the widget being resized, in a small dark pill in its middle.
fn draw_percent(cr: &gtk::cairo::Context, (x, y, rw, rh): Rect, k: f64) {
    let l = kit::paint::layout(cr, &format!("{}%", (k * 100.0).round()), "Readex Pro Medium 9");
    let (_, ext) = l.pixel_extents();
    let (pw, ph) = (ext.width() as f64 + 14.0, ext.height() as f64 + 8.0);
    let (px, py) = (x + (rw - pw) / 2.0, y + (rh - ph) / 2.0);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.62);
    kit::paint::rounded(cr, px, py, pw, ph, ph / 2.0);
    let _ = cr.fill();
    cr.move_to(px + 7.0 - ext.x() as f64, py + 4.0 - ext.y() as f64);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.95);
    kit::paint::show(cr, &l);
}

fn search(body: &gtk::Box, t: &toml::Table) {
    let find = section(body, "Search");
    toggle(&find, "deployed_code", "Minecraft instances", "PrismLauncher's instances show up as you type their name; pl and a space lists them all", "spotlight_prism", prefs::get_bool(&t, "spotlight_prism", false));
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
const SCOPES: [(&str, &str, &str, &str); 9] = [
    ("set", "Settings", "settings", "Finds a setting and opens it"),
    ("f", "Files", "folder", "Suggested while Files is in front"),
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
    let said: Vec<String> = [("snap", "area"), ("snap-screen", "screen"), ("snap-window", "window"), ("snap-delay", "3 s timer")]
        .iter()
        .filter_map(|(id, what)| kit::keybinds::shown_all(id).map(|k| format!("{k}: {what}")))
        .collect();
    let said = if said.is_empty() { "None set".to_string() } else { said.join(" · ") };
    let (row, end) = ui::row(Some("screenshot_region"), "Shortcuts", &said);
    let change = ui::button("Change…", "");
    change.connect_clicked(|_| crate::show("keyboard/screenshots"));
    end.append(&change);
    snap.append(&row);
    choice(&snap, "keyboard_return", "Enter after selecting", "Double-click inside the selection does the same", "snap_after", &[("copy", "Copy"), ("save", "Save"), ("edit", "Mark up"), ("pin", "Pin to screen")], &prefs::get_str(&t, "snap_after", "copy"));
    choice(&snap, "open_in_new", "Mark-up editor", "A window you can move and tile, or a layer over everything", "snap_editor", &[("window", "Window"), ("overlay", "Overlay")], &prefs::get_str(&t, "snap_editor", "window"));
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
    toggle(&snap, "zoom_in", "Magnifier", "Pixel loupe with coordinates while selecting (the color picker lives in the notch)", "snap_loupe", prefs::get_bool(&t, "snap_loupe", false));
    toggle(&snap, "arrow_selector_tool", "Include the pointer", "", "snap_cursor", prefs::get_bool(&t, "snap_cursor", false));
    toggle(&snap, "notifications", "Notify when done", "Shows the screenshot in the notch", "snap_notify", prefs::get_bool(&t, "snap_notify", true));
}

/// Adds an install notice before a page's sections, leaving its controls visible.
fn needs(body: &gtk::Box, need: &kit::optional::Need, what_for: &str) -> bool {
    if kit::optional::present(need) { return true; }
    let card = ui::card();
    let (row, end) = ui::row(Some("download"), &format!("{} isn't installed", need.name), &format!("{what_for} needs it. Get it from Digit Store."));
    let open = ui::button("Open Store", "accent");
    let query = format!("search:{}", need.store_query);
    open.connect_clicked(move |_| {
        let argv: [&std::ffi::OsStr; 2] = ["digit-store".as_ref(), query.as_ref()];
        if let Err(e) = gio::Subprocess::newv(&argv, gio::SubprocessFlags::NONE) {
            ui::fail(&format!("Could not open Digit Store: {e}"));
        }
    });
    end.append(&open);
    card.append(&row);
    body.append(&card);
    false
}

fn recording_capture(body: &gtk::Box, t: &toml::Table) {
    let cap = section(body, "Capture");
    let mut targets = vec![("monitor".to_string(), "The focused screen".to_string())];
    {
        for monitor in kit::screens::all() {
            if let Some(connector) = monitor.connector() {
                let g = monitor.geometry();
                targets.push((connector.to_string(), format!("Screen {connector} · {}×{}", g.width() * monitor.scale_factor(), g.height() * monitor.scale_factor())));
            }
        }
    }
    targets.push(("region".into(), "A region you draw".into()));
    targets.push(("portal".into(), "A window, chosen in the system picker".into()));
    let pairs: Vec<(&str, &str)> = targets.iter().map(|(value, label)| (value.as_str(), label.as_str())).collect();
    choice(&cap, "screenshot_monitor", "What to record", "", "rec_target", &pairs, &prefs::get_str(t, "rec_target", "monitor"));
    // Only the rates a screen here can show, and the one chosen even if it's no longer one.
    const RATES: [(&str, &str); 6] = [("30", "30 fps"), ("60", "60 fps"), ("120", "120 fps"), ("144", "144 fps"), ("165", "165 fps"), ("240", "240 fps")];
    let fps = prefs::get_str(t, "rec_fps", "60");
    let fastest = Some(kit::screens::all()).map_or(60, |all| all.iter().map(|m| (m.refresh_rate() + 999) / 1000).max().unwrap_or(60));
    let rates: Vec<(&str, &str)> = RATES.iter().copied().filter(|(v, _)| *v == fps || v.parse::<i32>().is_ok_and(|r| r <= 60 || r <= fastest)).collect();
    choice(&cap, "speed", "Frame rate", "", "rec_fps", &rates, &fps);
    choice(&cap, "high_quality", "Quality", "", "rec_quality", &[("medium", "Medium"), ("high", "High"), ("very_high", "Very high"), ("ultra", "Ultra")], &prefs::get_str(t, "rec_quality", "very_high"));
    toggle(&cap, "arrow_selector_tool", "Show the pointer", "", "rec_cursor", prefs::get_bool(t, "rec_cursor", true));
    let more = advanced_rows(&cap);
    choice(&more, "movie", "Codec", "H.264 plays everywhere; HEVC and AV1 are smaller, VP9 suits the web", "rec_codec", &[("auto", "Automatic (H.264)"), ("hevc", "HEVC"), ("av1", "AV1"), ("vp9", "VP9")], &prefs::get_str(t, "rec_codec", "auto"));
    choice(&more, "video_file", "File type", "", "rec_container", &[("mp4", "MP4"), ("mkv", "MKV"), ("mov", "MOV")], &prefs::get_str(t, "rec_container", "mp4"));
}

fn recording_sound(body: &gtk::Box, t: &toml::Table) {
    let sound = section(body, "Sound");
    toggle(&sound, "volume_up", "System sound", "Records what you hear.", "rec_system_audio", prefs::get_bool(t, "rec_system_audio", true));
    toggle(&sound, "mic", "Microphone", "Mixed into the same track.", "rec_mic", prefs::get_bool(t, "rec_mic", false));
}

fn recording_replay(body: &gtk::Box, t: &toml::Table) {
    let replay = section(body, "Instant Replay");
    choice(&replay, "history", "Keep the last", "", "rec_replay", &[("15", "15 seconds"), ("30", "30 seconds"), ("60", "1 minute"), ("120", "2 minutes"), ("300", "5 minutes"), ("600", "10 minutes")], &prefs::get_str(t, "rec_replay", "60"));
}

fn recording_folder(t: &toml::Table) -> std::path::PathBuf {
    let custom = prefs::get_str(t, "rec_dir", "");
    if !custom.is_empty() { return kit::prefs::home(&custom); }
    glib::user_special_dir(glib::UserDirectory::Videos).unwrap_or_else(|| glib::home_dir().join("Videos")).join("Recordings")
}

fn recording_saving(body: &gtk::Box, t: &toml::Table) {
    let saving = section(body, "Saving");
    let (row, end) = ui::row(Some("folder"), "Save to", "");
    let entry = gtk::Entry::new();
    entry.set_text(&recording_folder(t).display().to_string());
    entry.set_width_chars(22);
    entry.set_hexpand(true);
    ui::bind_entry(&entry, screenshot_folder, |text| save("rec_dir", toml::Value::String(text.into())));
    end.append(&entry);
    let pick = ui::button("Choose…", "");
    let target = entry.downgrade();
    pick.connect_clicked(move |b| {
        let path = recording_folder(&prefs::read());
        let target = target.clone();
        choose_recording_folder(ui::root(b), &path, move |path| {
            if let Some(entry) = target.upgrade() {
                entry.set_text(&path.display().to_string());
                entry.emit_activate();
            }
        });
    });
    end.append(&pick);
    saving.append(&row);
    let (row, end) = ui::row(Some("folder_open"), "Open the folder", "");
    let open = ui::button("Open", "");
    open.connect_clicked(|_| {
        let path = recording_folder(&prefs::read());
        if let Err(e) = std::fs::create_dir_all(&path) {
            ui::fail(&format!("Could not open the folder: {e}"));
            return;
        }
        ui::open_uri(&gio::File::for_path(path).uri());
    });
    end.append(&open);
    saving.append(&row);
}

// Match ui::choose_folder's portal and layer-shell handling with a recording-specific title.
fn choose_recording_folder(parent: Option<gtk::Window>, initial: &std::path::Path, done: impl FnOnce(std::path::PathBuf) + 'static) {
    use gtk4_layer_shell::LayerShell;
    let chooser = gtk::FileDialog::new();
    chooser.set_title("Save recordings to");
    chooser.set_accept_label(Some("Choose"));
    if initial.is_dir() { chooser.set_initial_folder(Some(&gio::File::for_path(initial))); }
    let overlay = parent.as_ref().is_some_and(|w| w.is_layer_window());
    let restore = parent.as_ref().filter(|w| overlay && w.is_visible()).cloned();
    if let Some(window) = &restore { window.set_visible(false); }
    let native_parent = if overlay { None } else { parent.as_ref() };
    chooser.select_folder(native_parent, gio::Cancellable::NONE, move |result| {
        if let Some(window) = restore { window.present(); }
        match result {
            Ok(file) => match file.path() {
                Some(path) => done(path),
                None => ui::fail("Choose a folder on this computer"),
            },
            Err(e) if e.matches(gtk::DialogError::Cancelled) || e.matches(gtk::DialogError::Dismissed) => {},
            Err(e) => ui::fail(&format!("Could not open the folder chooser: {e}")),
        }
    });
}

fn recording_stream_path() -> std::path::PathBuf {
    glib::user_config_dir().join("digit/rec-stream")
}

fn save_recording_stream(url: &str) -> Result<(), String> {
    let path = recording_stream_path();
    if url.is_empty() {
        return match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        };
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    // Replace atomically with a new private file, even if the old file had wider permissions.
    gio::File::for_path(path).replace_contents(url.as_bytes(), None, false, gio::FileCreateFlags::PRIVATE | gio::FileCreateFlags::REPLACE_DESTINATION, gio::Cancellable::NONE)
        .map(|_| ()).map_err(|e| e.to_string())
}

fn recording_streaming(body: &gtk::Box, _: &toml::Table) {
    let streaming = section(body, "Streaming");
    let (row, end) = ui::row(Some("cell_tower"), "Stream address", "Use rtmp://, rtmps:// or srt:// with your stream key. Only you can read the private file.");
    let entry = gtk::PasswordEntry::new();
    entry.set_show_peek_icon(true);
    entry.set_text(std::fs::read_to_string(recording_stream_path()).unwrap_or_default().trim());
    entry.set_width_chars(22);
    entry.add_css_class("search");
    entry.connect_changed(|e| {
        if let Err(error) = save_recording_stream(e.text().trim()) {
            ui::fail(&format!("Could not save the stream address: {error}"));
        }
    });
    end.append(&entry);
    streaming.append(&row);
}

fn recording_shortcuts(body: &gtk::Box, _: &toml::Table) {
    let shortcuts = section(body, "Shortcuts");
    let (row, end) = ui::row(Some("keyboard"), "Shortcuts", "Start, stop, pause and replay recordings with keys");
    let change = ui::button("Change…", "");
    change.connect_clicked(|_| crate::show("keyboard/recording"));
    end.append(&change);
    shortcuts.append(&row);
}

pub(crate) fn screenshot_folder(text: &str) -> Result<(), String> {
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
}

pub fn advanced(body: &gtk::Box, t: &toml::Table) {
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
    choice(&adv, "memory", "Rendering", "How digit and its apps draw. Takes effect when they next start", "renderer", &[("auto", "Automatic"), ("gl", "GPU (OpenGL)"), ("vulkan", "GPU (Vulkan)"), ("cairo", "Software")], &prefs::get_str(t, "renderer", "auto"));
    if crate::compositor::show_nvidia_variables(kit::hyprconf::nvidia(), prefs::get_bool(t, "nvidia_env", false)) {
        toggle(&adv, "developer_board", "NVIDIA variables", "For hardware video and older apps. Takes effect at the next login", "nvidia_env", prefs::get_bool(t, "nvidia_env", false));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn workspace_choice_stores_an_integer() {
        assert_eq!(super::choice_value("spaces_always", "2"), toml::Value::Integer(2));
        assert_eq!(super::choice_value("spaces_always", "0"), toml::Value::Integer(0));
        assert_eq!(super::choice_value("spaces_always", "99"), toml::Value::Integer(10));
        assert_eq!(super::choice_value("spaces_close", "leave"), toml::Value::String("leave".into()));
    }

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

#[cfg(test)]
mod reduction_tests {
    use super::*;

    #[test]
    fn a_widget_on_the_map_grows_away_from_its_corner_and_its_own_corner_resizes_it() {
        let spot = |id: &str| SPOTS.iter().find(|s| s.0 == id).unwrap().2;
        let (w, h) = (400.0, 225.0);
        let (one, big) = (sized_rect("bottom_right", spot("bottom_right"), 1.0, w, h), sized_rect("bottom_right", spot("bottom_right"), 1.5, w, h));
        assert!((big.0 + big.2 - (one.0 + one.2)).abs() < 1e-9 && (big.1 + big.3 - (one.1 + one.3)).abs() < 1e-9, "the bottom right corner stays");
        assert!((big.2 - one.2 * 1.5).abs() < 1e-9);
        let centre = sized_rect("top_center", spot("top_center"), 1.4, w, h);
        let base = sized_rect("top_center", spot("top_center"), 1.0, w, h);
        assert!((centre.0 + centre.2 / 2.0 - (base.0 + base.2 / 2.0)).abs() < 1e-9 && centre.1 == base.1, "the centre stays, the top too");
        // The corner is the one away from where it grows from.
        let ((fx, fy), (cx, cy)) = grip("top_right", one);
        assert_eq!((fx, fy, cx, cy), (one.0 + one.2, one.1, one.0, one.1 + one.3));
        assert_eq!(grip_cursor("top_right"), "nesw-resize");
        // Pulling the corner outwards along the line doubles it, inwards halves it, sideways leaves it.
        let (from, corner) = ((0.0, 0.0), (100.0, 50.0));
        assert_eq!(pulled(1.0, from, corner, (50.0, 25.0)), 1.5);
        assert_eq!(pulled(1.0, from, corner, (-40.0, -20.0)), kit::deskwidget::clamp_size(0.6));
        assert_eq!(pulled(1.0, from, corner, (-10.0, 20.0)), 1.0);
        assert_eq!(pulled(1.2, from, corner, (500.0, 500.0)), kit::deskwidget::SIZE_RANGE.1);
    }

    #[test]
    fn every_desktop_widget_has_a_color_the_shell_reads() {
        for w in &WIDGETS {
            assert!(kit::deskwidget::WIDGETS.contains(&w.ink), "{}", w.id);
        }
        assert_eq!(WIDGETS.len(), kit::deskwidget::WIDGETS.len());
    }

    #[test]
    fn each_child_is_controlled_only_by_its_own_master() {
        for (key, motion) in [("smooth_scroll", true), ("smooth_caret", true), ("intro", true), ("compositor_motion", true), ("hypr_window_anim", true), ("hypr_workspace_anim", true), ("niri_motion", true), ("hypr_shadow", false), ("hypr_blur", false), ("menubar_look", false)] {
            assert_eq!(controlled_in(key, false, false, false), None);
            assert_eq!(controlled_in(key, !motion, motion, false), None);
            assert_eq!(controlled_in(key, motion, !motion, false), Some(if motion { "Off while Reduce motion is on" } else { "Off while Reduce effects is on" }));
        }
        for key in ["reduce_motion", "reduced_effects", "renderer"] {
            assert_eq!(controlled_in(key, true, true, true), None);
        }
    }

    #[test]
    fn compositor_animations_off_greys_only_the_compositor_animations() {
        for key in ["hypr_window_anim", "hypr_workspace_anim", "niri_motion"] {
            assert_eq!(controlled_in(key, false, false, true), Some("Off while Compositor animations is off"));
            assert_eq!(controlled_in(key, true, false, true), Some("Off while Reduce motion is on"));
        }
        for key in ["smooth_scroll", "smooth_caret", "intro", "compositor_motion", "hypr_blur"] {
            assert_eq!(controlled_in(key, false, false, true), None);
        }
    }
}
