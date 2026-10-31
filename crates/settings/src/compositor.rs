use crate::desktop::{choice, field, save, slider, tiles, toggle};
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
    {
        let current = s("hypr_tiling", "dwindle");
        let at = TILING.iter().position(|t| t.0 == current).unwrap_or(0);
        let items = TILING.iter().map(|(v, name)| (*name, tiling_picture(v))).collect();
        tiles(&win, "dashboard", "Tiling", "How new windows share the screen", items, at, |i| save("hypr_tiling", toml::Value::String(TILING[i].0.into())));
    }
    toggle(&win, "arrow_selector_tool", "Focus follows the mouse", "The window under the pointer takes the keyboard", "hypr_follow_mouse", b("hypr_follow_mouse", true));
    {
        let (gi, go) = (f("hypr_gaps_in", 4.0), f("hypr_gaps_out", 8.0));
        let at = GAPS.iter().position(|g| g.1 == gi && g.2 == go).unwrap_or(usize::MAX);
        let items = GAPS.iter().map(|(name, i, o)| (*name, gaps_picture(*i, *o))).collect();
        tiles(&win, "padding", "Gaps", "Between windows and at the screen edges; fine-tune them below", items, at, |i| {
            save("hypr_gaps_in", toml::Value::Float(GAPS[i].1));
            save("hypr_gaps_out", toml::Value::Float(GAPS[i].2));
        });
    }
    slider(&win, "padding", "Gaps between windows", "", "hypr_gaps_in", (0.0, 24.0, 1.0), f("hypr_gaps_in", 4.0), 0);
    slider(&win, "fit_screen", "Gaps at the screen edges", "", "hypr_gaps_out", (0.0, 48.0, 1.0), f("hypr_gaps_out", 8.0), 0);
    slider(&win, "border_style", "Border width", "", "hypr_border", (0.0, 6.0, 1.0), f("hypr_border", 2.0), 0);
    body.append(&win);

    body.append(&ui::heading("Effects"));
    let fx = ui::card();
    {
        let at = if b("hypr_shadow", true) { SHADOWS.iter().position(|v| v.0 == s("hypr_shadow_size", "medium")).unwrap_or(2) } else { 0 };
        let items = SHADOWS.iter().map(|(v, name, r)| (*name, shadow_picture(if *v == "off" { 0.0 } else { *r }))).collect();
        tiles(&fx, "shadow", "Window shadows", "", items, at, |i| {
            save("hypr_shadow", toml::Value::Boolean(i != 0));
            if i != 0 {
                save("hypr_shadow_size", toml::Value::String(SHADOWS[i].0.into()));
            }
        });
    }
    {
        let at = if b("hypr_blur", false) { BLURS.iter().position(|v| v.0 == s("hypr_blur_strength", "medium")).unwrap_or(2) } else { 0 };
        let items = BLURS.iter().map(|(_, name, k)| (*name, blur_picture(*k))).collect();
        tiles(&fx, "blur_on", "Blur behind windows", "Frosts what's behind see-through windows; costs some GPU time", items, at, |i| {
            save("hypr_blur", toml::Value::Boolean(i != 0));
            if i != 0 {
                save("hypr_blur_strength", toml::Value::String(BLURS[i].0.into()));
            }
        });
    }
    body.append(&fx);
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
        "hypr_shadow" => "decoration:shadow:enabled",
        "hypr_blur" => "decoration:blur:enabled",
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

const TILING: [(&str, &str); 3] = [("dwindle", "Dwindle"), ("master", "Master"), ("scrolling", "Scrolling")];
const GAPS: [(&str, f64, f64); 4] = [("None", 0.0, 0.0), ("Tight", 2.0, 4.0), ("Comfortable", 4.0, 8.0), ("Wide", 8.0, 16.0)];
const SHADOWS: [(&str, &str, f64); 4] = [("off", "Off", 0.0), ("soft", "Soft", 5.0), ("medium", "Medium", 9.0), ("large", "Large", 14.0)];
const BLURS: [(&str, &str, f64); 4] = [("off", "Off", 0.0), ("light", "Light", 0.35), ("medium", "Medium", 0.65), ("strong", "Strong", 1.0)];

/// A small picture drawn in the tile's text color.
fn picture(draw: impl Fn(&gtk::cairo::Context, f64, f64, (f64, f64, f64)) + 'static) -> gtk::Widget {
    let area = gtk::DrawingArea::new();
    area.set_draw_func(move |a, cr, w, h| {
        let c = a.color();
        draw(cr, w as f64, h as f64, (c.red() as f64, c.green() as f64, c.blue() as f64));
    });
    area.upcast()
}

fn rrect(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
    cr.arc(x + r, y + r, r, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
    cr.close_path();
}

/// Windows as light boxes with an outline, like the corner previews.
fn window(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, ink: (f64, f64, f64), strong: bool) {
    rrect(cr, x, y, w, h, 4.0);
    cr.set_source_rgba(ink.0, ink.1, ink.2, if strong { 0.22 } else { 0.1 });
    let _ = cr.fill_preserve();
    cr.set_source_rgba(ink.0, ink.1, ink.2, 0.55);
    cr.set_line_width(1.2);
    let _ = cr.stroke();
}

fn tiling_picture(kind: &'static str) -> gtk::Widget {
    picture(move |cr, w, h, ink| {
        let (x0, y0, x1, y1, g) = (12.0, 10.0, w - 12.0, h - 10.0, 4.0);
        let (wd, ht) = (x1 - x0, y1 - y0);
        match kind {
            "master" => {
                window(cr, x0, y0, wd * 0.55 - g / 2.0, ht, ink, true);
                let sx = x0 + wd * 0.55 + g / 2.0;
                let sh = (ht - 2.0 * g) / 3.0;
                for i in 0..3 {
                    window(cr, sx, y0 + i as f64 * (sh + g), x1 - sx, sh, ink, false);
                }
            }
            "scrolling" => {
                // Columns run past the screen edges; the dashed frame is the screen.
                let cw = wd * 0.42;
                for (i, x) in [x0 - cw * 0.55, x0 + wd / 2.0 - cw / 2.0, x1 - cw * 0.45].iter().enumerate() {
                    window(cr, *x, y0, cw, ht, ink, i == 1);
                }
                cr.set_source_rgba(ink.0, ink.1, ink.2, 0.5);
                cr.set_line_width(1.0);
                cr.set_dash(&[3.0, 3.0], 0.0);
                rrect(cr, x0 + wd * 0.12, y0 - 4.0, wd * 0.76, ht + 8.0, 5.0);
                let _ = cr.stroke();
                cr.set_dash(&[], 0.0);
            }
            _ => {
                let half = wd / 2.0 - g / 2.0;
                window(cr, x0, y0, half, ht, ink, true);
                let rx = x0 + half + g;
                let rh = ht / 2.0 - g / 2.0;
                window(cr, rx, y0, x1 - rx, rh, ink, false);
                let ry = y0 + rh + g;
                let rw = (x1 - rx) / 2.0 - g / 2.0;
                window(cr, rx, ry, rw, y1 - ry, ink, false);
                window(cr, rx + rw + g, ry, x1 - rx - rw - g, y1 - ry, ink, false);
            }
        }
    })
}

fn gaps_picture(inner: f64, outer: f64) -> gtk::Widget {
    picture(move |cr, w, h, ink| {
        // A screen frame with two windows set into it, at a scale the difference shows.
        let (sx, sy, sw, sh) = (12.0, 10.0, w - 24.0, h - 20.0);
        cr.set_source_rgba(ink.0, ink.1, ink.2, 0.35);
        cr.set_line_width(1.0);
        rrect(cr, sx, sy, sw, sh, 5.0);
        let _ = cr.stroke();
        let (o, i) = (outer * 0.5, inner * 0.6);
        let ww = (sw - 2.0 * o - i) / 2.0;
        window(cr, sx + o, sy + o, ww, sh - 2.0 * o, ink, true);
        window(cr, sx + o + ww + i, sy + o, ww, sh - 2.0 * o, ink, false);
    })
}

fn shadow_picture(range: f64) -> gtk::Widget {
    picture(move |cr, w, h, ink| {
        let (x, y, ww, hh) = (w * 0.28, h * 0.22, w * 0.44, h * 0.5);
        // Fake the soft shadow with stacked translucent rounds, offset a little down.
        let steps = (range * 1.5) as i32;
        for k in (1..=steps).rev() {
            let s = k as f64 / 1.5;
            rrect(cr, x - s, y - s + range * 0.35, ww + 2.0 * s, hh + 2.0 * s, 4.0 + s);
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.5 / steps as f64);
            let _ = cr.fill();
        }
        rrect(cr, x, y, ww, hh, 4.0);
        cr.set_source_rgba(0.14, 0.14, 0.15, 1.0);
        let _ = cr.fill_preserve();
        cr.set_source_rgba(ink.0, ink.1, ink.2, 0.4);
        cr.set_line_width(1.0);
        let _ = cr.stroke();
    })
}

fn blur_picture(strength: f64) -> gtk::Widget {
    picture(move |cr, w, h, ink| {
        // Colorful stripes behind a see-through window; inside the window they wash out with blur.
        let colors = [(0.95, 0.42, 0.42), (0.98, 0.75, 0.3), (0.35, 0.8, 0.55), (0.35, 0.6, 0.98)];
        let band = w / colors.len() as f64;
        for (i, c) in colors.iter().enumerate() {
            cr.rectangle(i as f64 * band, 0.0, band, h);
            cr.set_source_rgba(c.0, c.1, c.2, 0.55);
            let _ = cr.fill();
        }
        let (x, y, ww, hh) = (w * 0.2, h * 0.24, w * 0.6, h * 0.52);
        rrect(cr, x, y, ww, hh, 5.0);
        let _ = cr.save();
        cr.clip_preserve();
        // The stronger the blur, the more the stripes melt into their average.
        let avg = colors.iter().fold((0.0, 0.0, 0.0), |a, c| (a.0 + c.0 / 4.0, a.1 + c.1 / 4.0, a.2 + c.2 / 4.0));
        cr.set_source_rgba(avg.0, avg.1, avg.2, 0.55 * strength);
        let _ = cr.fill_preserve();
        cr.set_source_rgba(0.12, 0.12, 0.13, 0.45);
        let _ = cr.fill_preserve();
        let _ = cr.restore();
        cr.set_source_rgba(ink.0, ink.1, ink.2, 0.6);
        cr.set_line_width(1.0);
        let _ = cr.stroke();
    })
}
