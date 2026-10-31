use crate::desktop::{choice, field, slider, toggle};
use crate::{prefs, ui};
use gtk::prelude::*;

pub fn build() -> gtk::Widget {
    let (scroll, body) = ui::page("Compositor", "Keyboard, mouse and window settings for Hyprland. They apply right away.");
    let wm = kit::wm::which();
    if wm != kit::wm::Wm::Hyprland {
        let card = ui::card();
        let name = if wm == kit::wm::Wm::Other { "this compositor".to_string() } else { kit::wm::name().to_string() };
        let (row, _) = ui::row(Some("warning"), &format!("digit can't change {name}'s settings"), "These settings are written to Hyprland's config. Set them in your compositor's own config instead");
        card.append(&row);
        body.append(&card);
        return scroll.upcast();
    }
    let Some(mode) = kit::hyprconf::mode() else {
        let card = ui::card();
        let (row, end) = ui::row(Some("warning"), "Hyprland isn't set up for digit", "Choose a Hyprland setup first: Full or Minimal, under Desktop > Windows");
        let b = ui::button("Open Desktop", "accent");
        b.connect_clicked(|_| crate::show("desktop"));
        end.append(&b);
        card.append(&row);
        body.append(&card);
        return scroll.upcast();
    };
    if mode == kit::hyprconf::Mode::Minimal {
        let card = ui::card();
        let (row, _) = ui::row(Some("info"), "Minimal setup", "Your own config decides everything here until you change it; what you change wins over your config");
        card.append(&row);
        body.append(&card);
    }
    let t = prefs::read();
    // Unchanged values show digit's defaults in a full setup, and what Hyprland runs with now
    // in a minimal one, since the user's own config decides those.
    let minimal = mode == kit::hyprconf::Mode::Minimal;
    let s = |k: &str, d: &str| prefs::get_str(&t, k, &(if minimal { live_text(k) } else { None }).unwrap_or_else(|| d.to_string()));
    let f = |k: &str, d: f64| prefs::get_f64(&t, k, (if minimal { live_num(k) } else { None }).unwrap_or(d));
    let b = |k: &str, d: bool| prefs::get_bool(&t, k, (if minimal { live_num(k).map(|v| v != 0.0) } else { None }).unwrap_or(d));

    body.append(&ui::heading("Keyboard"));
    let kb = ui::card();
    field(&kb, "keyboard", "Layouts", "Comma-separated, first is the default: us, tr, de…", "hypr_kb_layout", &s("hypr_kb_layout", "us"), "us", layouts);
    field(&kb, "tune", "Variants", "One per layout, or empty: intl, f, dvorak…", "hypr_kb_variant", &s("hypr_kb_variant", ""), "", variants);
    choice(&kb, "swap_horiz", "Switch layouts with", "Super+Space also opens the notch", "hypr_layout_switch", &[("none", "Nothing"), ("alt_shift", "Alt+Shift"), ("ctrl_shift", "Ctrl+Shift"), ("win_space", "Super+Space"), ("caps", "Caps Lock")], &s("hypr_layout_switch", "none"));
    choice(&kb, "keyboard_capslock", "Caps Lock", "", "hypr_caps", &[("normal", "Caps Lock"), ("escape", "Escape"), ("ctrl", "Ctrl"), ("swapescape", "Swap with Escape")], &s("hypr_caps", "normal"));
    slider(&kb, "timer", "Repeat delay", "Milliseconds before a held key repeats", "hypr_repeat_delay", (150.0, 800.0, 10.0), f("hypr_repeat_delay", 300.0), 0);
    slider(&kb, "speed", "Repeat rate", "Repeats per second", "hypr_repeat_rate", (10.0, 60.0, 1.0), f("hypr_repeat_rate", 30.0), 0);
    body.append(&kb);

    body.append(&ui::heading("Mouse and touchpad"));
    let mouse = ui::card();
    slider(&mouse, "mouse", "Pointer speed", "0 keeps the device's own speed", "hypr_sensitivity", (-1.0, 1.0, 0.05), f("hypr_sensitivity", 0.0), 2);
    choice(&mouse, "trending_up", "Acceleration", "", "hypr_accel", &[("adaptive", "Adaptive"), ("flat", "Flat")], &s("hypr_accel", "adaptive"));
    toggle(&mouse, "swap_vert", "Natural scrolling for mice", "Content follows the wheel, like a touchpad", "hypr_mouse_natural", b("hypr_mouse_natural", false));
    toggle(&mouse, "touch_app", "Natural scrolling for touchpads", "", "hypr_touch_natural", b("hypr_touch_natural", true));
    toggle(&mouse, "ads_click", "Tap to click", "A light tap on the touchpad clicks", "hypr_tap", b("hypr_tap", true));
    toggle(&mouse, "swipe", "Swipe between workspaces", "Three fingers sideways on the touchpad", "hypr_gesture", b("hypr_gesture", true));
    body.append(&mouse);

    body.append(&ui::heading("Windows"));
    let win = ui::card();
    choice(&win, "dashboard", "Tiling", "How new windows share the screen", "hypr_tiling", &[("dwindle", "Dwindle"), ("master", "Master"), ("scrolling", "Scrolling")], &s("hypr_tiling", "dwindle"));
    toggle(&win, "arrow_selector_tool", "Focus follows the mouse", "The window under the pointer takes the keyboard", "hypr_follow_mouse", b("hypr_follow_mouse", true));
    slider(&win, "padding", "Gaps between windows", "", "hypr_gaps_in", (0.0, 24.0, 1.0), f("hypr_gaps_in", 4.0), 0);
    slider(&win, "fit_screen", "Gaps at the screen edges", "", "hypr_gaps_out", (0.0, 48.0, 1.0), f("hypr_gaps_out", 8.0), 0);
    slider(&win, "border_style", "Border width", "", "hypr_border", (0.0, 6.0, 1.0), f("hypr_border", 2.0), 0);
    body.append(&win);
    scroll.upcast()
}

fn layouts(s: &str) -> bool {
    !s.is_empty() && s.split(',').all(|l| {
        let l = l.trim();
        !l.is_empty() && l.len() <= 12 && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    })
}

fn variants(s: &str) -> bool {
    s.split(',').all(|l| l.trim().chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')) && s.len() <= 64
}

/// The Hyprland option behind a setting.
fn option(key: &str) -> Option<&'static str> {
    Some(match key {
        "hypr_kb_layout" => "input:kb_layout",
        "hypr_kb_variant" => "input:kb_variant",
        "hypr_layout_switch" | "hypr_caps" => "input:kb_options",
        "hypr_repeat_delay" => "input:repeat_delay",
        "hypr_repeat_rate" => "input:repeat_rate",
        "hypr_sensitivity" => "input:sensitivity",
        "hypr_accel" => "input:accel_profile",
        "hypr_mouse_natural" => "input:natural_scroll",
        "hypr_touch_natural" => "input:touchpad:natural_scroll",
        "hypr_tap" => "input:touchpad:tap-to-click",
        "hypr_follow_mouse" => "input:follow_mouse",
        "hypr_tiling" => "general:layout",
        "hypr_gaps_in" => "general:gaps_in",
        "hypr_gaps_out" => "general:gaps_out",
        "hypr_border" => "general:border_size",
        _ => return None,
    })
}

fn live(key: &str) -> Option<serde_json::Value> {
    let out = std::process::Command::new("hyprctl").args(["-j", "getoption", option(key)?]).output().ok()?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    ["str", "int", "float", "bool", "css"].iter().find_map(|k| v.get(*k).cloned())
}

fn live_text(key: &str) -> Option<String> {
    let v = live(key)?;
    let s = v.as_str()?.to_string();
    let s = if s == "[[EMPTY]]" { String::new() } else { s };
    // Caps Lock and the layout switch both live in kb_options.
    Some(match key {
        "hypr_caps" => (if s.contains("caps:swapescape") { "swapescape" } else if s.contains("caps:escape") { "escape" } else if s.contains("ctrl:nocaps") { "ctrl" } else { "normal" }).into(),
        "hypr_layout_switch" => (if s.contains("grp:alt_shift_toggle") { "alt_shift" } else if s.contains("grp:ctrl_shift_toggle") { "ctrl_shift" } else if s.contains("grp:win_space_toggle") { "win_space" } else if s.contains("grp:caps_toggle") { "caps" } else { "none" }).into(),
        "hypr_accel" if s.is_empty() => "adaptive".into(),
        _ => s,
    })
}

fn live_num(key: &str) -> Option<f64> {
    match live(key)? {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::Bool(b) => Some(if b { 1.0 } else { 0.0 }),
        // Gaps come as "top right bottom left".
        serde_json::Value::String(s) => s.split_whitespace().next()?.parse().ok(),
        _ => None,
    }
}
