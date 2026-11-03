use crate::desktop::{choice, field, save, save_many, slider, tiles, toggle};
use crate::{prefs, ui};
use gtk::prelude::*;

pub fn build() -> gtk::Widget {
    let (scroll, body) = ui::page("Windows", "How digit sets up your compositor, and its keyboard, mouse and window settings. They apply right away.");
    crate::desktop::setup(&body, &prefs::read());
    let Some(kind) = kit::setup::kind() else {
        let card = ui::card();
        let (row, _) = ui::row(Some("warning"), "digit can't change this compositor's settings", "These settings are written to Hyprland's or niri's config. Set them in your compositor's own config instead");
        card.append(&row);
        body.append(&card);
        return scroll.upcast();
    };
    let niri = kind == kit::setup::Kind::Niri;
    // Without a setup, only the Setup row above has anything to offer.
    if !kit::setup::installed() {
        return scroll.upcast();
    }
    let t = prefs::read();
    let s = |k: &str, d: &str| prefs::get_str(&t, k, d);
    let f = |k: &str, d: f64| prefs::get_f64(&t, k, d);
    let b = |k: &str, d: bool| prefs::get_bool(&t, k, d);

    body.append(&ui::heading("Keyboard"));
    let kb = ui::card();
    field(&kb, "keyboard", "Layouts", "Comma-separated, first is the default: us, tr, de…", "hypr_kb_layout", &s("hypr_kb_layout", "us"), "us", layouts);
    field(&kb, "tune", "Variants", "One per layout, or empty: intl, f, dvorak…", "hypr_kb_variant", &s("hypr_kb_variant", ""), "", variants);
    choice(&kb, "swap_horiz", "Switch layouts with", &kit::keybinds::shown("notch").map(|k| format!("{k} also opens the notch")).unwrap_or_default(), "hypr_layout_switch", &[("none", "Nothing"), ("alt_shift", "Alt+Shift"), ("ctrl_shift", "Ctrl+Shift"), ("win_space", "Super+Space"), ("caps", "Caps Lock")], &s("hypr_layout_switch", "none"));
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
    // niri swipes between workspaces on its own.
    if !niri {
        toggle(&mouse, "swipe", "Swipe between workspaces", "Three fingers sideways on the touchpad", "hypr_gesture", b("hypr_gesture", true));
    }
    if niri {
        toggle(&mouse, "keyboard_hide", "Ignore the touchpad while typing", "", "niri_dwt", b("niri_dwt", true));
        choice(&mouse, "swap_vert", "Touchpad scroll speed", "", "niri_scroll", &[("slow", "Slow"), ("normal", "Normal"), ("fast", "Fast")], &s("niri_scroll", "normal"));
        choice(&mouse, "arrow_selector_tool", "Pointer size", "", "niri_cursor_size", &[("small", "Small"), ("medium", "Medium"), ("large", "Large")], &s("niri_cursor_size", "small"));
        toggle(&mouse, "visibility_off", "Hide the pointer while typing", "", "niri_hide_cursor", b("niri_hide_cursor", false));
    }
    body.append(&mouse);

    body.append(&ui::heading("Windows"));
    let win = ui::card();
    // niri always scrolls; there is no other tiling to pick.
    if !niri {
        let current = s("hypr_tiling", "dwindle");
        let at = TILING.iter().position(|t| t.0 == current).unwrap_or(0);
        let items = TILING.iter().map(|(v, name)| (*name, tiling_picture(v))).collect();
        tiles(&win, "dashboard", "Tiling", "How new windows share the screen", items, at, |i| save("hypr_tiling", toml::Value::String(TILING[i].0.into())));
        cycle(&win, &t);
    }
    toggle(&win, "arrow_selector_tool", "Focus follows the mouse", "The window under the pointer takes the keyboard", "hypr_follow_mouse", b("hypr_follow_mouse", true));
    {
        let (gi, go) = (f("hypr_gaps_in", 4.0), f("hypr_gaps_out", 8.0));
        let at = GAPS.iter().position(|g| g.1 == gi && g.2 == go).unwrap_or(usize::MAX);
        let items = GAPS.iter().map(|(name, i, o)| (*name, gaps_picture(*i, *o))).collect();
        tiles(&win, "padding", "Gaps", "Between windows and at the screen edges", items, at, |i| {
            save_many(&[("hypr_gaps_in", toml::Value::Float(GAPS[i].1)), ("hypr_gaps_out", toml::Value::Float(GAPS[i].2))])
        });
    }
    slider(&win, "border_style", "Border width", "", "hypr_border", (0.0, 6.0, 1.0), f("hypr_border", 2.0), 0);
    if niri {
        choice(&win, "highlight", "Mark the focused window", "", "niri_highlight", &[("border", "Border"), ("ring", "Ring"), ("none", "Nothing")], &s("niri_highlight", "border"));
    }
    choice(&win, "palette", "Border color", "The focused window's border", "border_color", &[("accent", "Accent"), ("default", "Grey")], &s("border_color", "accent"));
    body.append(&win);

    if niri {
        body.append(&ui::heading("Columns"));
        let cols = ui::card();
        let current = s("niri_column_width", "half");
        let at = WIDTHS.iter().position(|w| w.0 == current).unwrap_or(1);
        let items = WIDTHS.iter().map(|(_, name, share)| (*name, width_picture(*share))).collect();
        tiles(&cols, "view_column", "New windows open", "As a column this wide", items, at, |i| save("niri_column_width", toml::Value::String(WIDTHS[i].0.into())));
        choice(&cols, "center_focus_strong", "Center the focused column", "", "niri_center", &[("never", "Never"), ("on-overflow", "When it doesn't fit"), ("always", "Always")], &s("niri_center", "never"));
        choice(&cols, "width", "Widths to step through", "With the Cycle the column width shortcut", "niri_presets", &[("thirds", "⅓ · ½ · ⅔"), ("halves", "½ · Full")], &s("niri_presets", "thirds"));
        body.append(&cols);

        body.append(&ui::heading("Overview and motion"));
        let more = ui::card();
        choice(&more, "zoom_out_map", "Overview size", "How small workspaces get in the overview", "niri_overview_zoom", &[("small", "Small"), ("medium", "Medium"), ("large", "Large")], &s("niri_overview_zoom", "medium"));
        choice(&more, "animation", "Animations", "", "niri_motion", &[("normal", "Normal"), ("slow", "Slow"), ("off", "Off")], &s("niri_motion", "normal"));
        body.append(&more);
    }

    // niri's workspaces only stack vertically and its windows only pop in.
    if !niri {
        body.append(&ui::heading("Animations"));
        let motion = ui::card();
        choice(&motion, "open_in_full", "Windows open and close", "", "hypr_window_anim", &[("popin", "Pop in"), ("slide", "Slide")], &s("hypr_window_anim", "popin"));
        choice(&motion, "view_carousel", "Switching workspaces", "", "hypr_workspace_anim", &[("vertical", "Vertical"), ("horizontal", "Horizontal")], &s("hypr_workspace_anim", "vertical"));
        body.append(&motion);
    }

    body.append(&ui::heading("Effects"));
    let fx = ui::card();
    {
        let at = if b("hypr_shadow", true) { SHADOWS.iter().position(|v| v.0 == s("hypr_shadow_size", "medium")).unwrap_or(2) } else { 0 };
        let items = SHADOWS.iter().map(|(v, name, r)| (*name, shadow_picture(if *v == "off" { 0.0 } else { *r }))).collect();
        tiles(&fx, "shadow", "Window shadows", "", items, at, |i| {
            let mut values = vec![("hypr_shadow", toml::Value::Boolean(i != 0))];
            if i != 0 { values.push(("hypr_shadow_size", toml::Value::String(SHADOWS[i].0.into()))); }
            save_many(&values)
        });
    }
    {
        let at = if b("hypr_blur", false) { BLURS.iter().position(|v| v.0 == s("hypr_blur_strength", "medium")).unwrap_or(2) } else { 0 };
        let items = BLURS.iter().map(|(_, name, k)| (*name, blur_picture(*k))).collect();
        tiles(&fx, "blur_on", "Blur behind windows", "Frosts what's behind see-through windows; costs some GPU time", items, at, |i| {
            let mut values = vec![("hypr_blur", toml::Value::Boolean(i != 0))];
            if i != 0 { values.push(("hypr_blur_strength", toml::Value::String(BLURS[i].0.into()))); }
            save_many(&values)
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
/// Which layouts the layout shortcuts step through; at least two stay on.
fn cycle(card: &gtk::Box, t: &toml::Table) {
    let said = ["layouts", "workspace-layout"].iter().filter_map(|id| kit::keybinds::shown(id)).collect::<Vec<_>>().join(" and ");
    let sub = if said.is_empty() { "The layout shortcuts step through these".to_string() } else { format!("{said} step through these") };
    let (row, end) = ui::row(Some("swap_horiz"), "Layout shortcuts", &sub);
    let off: std::rc::Rc<std::cell::RefCell<Vec<String>>> = std::rc::Rc::new(std::cell::RefCell::new(
        t.get("hypr_tiling_cycle_off").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default(),
    ));
    let seg = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    seg.add_css_class("seg");
    seg.set_valign(gtk::Align::Center);
    for (key, name) in TILING {
        let b = gtk::ToggleButton::with_label(name);
        b.set_active(!off.borrow().iter().any(|k| k == key));
        let off = off.clone();
        let restoring = std::cell::Cell::new(false);
        b.connect_toggled(move |b| {
            if restoring.get() { return; }
            let previous = off.borrow().clone();
            let left_on = TILING.len() - off.borrow().iter().filter(|k| *k != key).count();
            if !b.is_active() && left_on <= 2 {
                // Stepping needs two layouts to step between. Switching it back runs this
                // again, so nothing may be borrowed here.
                restoring.set(true);
                b.set_active(true);
                restoring.set(false);
                ui::fail("Keep at least two layouts to switch between");
                return;
            }
            let list = {
                let mut list = off.borrow_mut();
                list.retain(|k| k != key);
                if !b.is_active() {
                    list.push(key.to_string());
                }
                list.clone()
            };
            // Not a compositor setting, so it's saved without rewriting the config.
            if let Err(e) = prefs::set("hypr_tiling_cycle_off", toml::Value::Array(list.into_iter().map(toml::Value::String).collect())) {
                *off.borrow_mut() = previous.clone();
                restoring.set(true);
                b.set_active(!previous.iter().any(|k| k == key));
                restoring.set(false);
                ui::fail(&e);
            }
        });
        seg.append(&b);
    }
    end.append(&seg);
    card.append(&row);
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

const WIDTHS: [(&str, &str, f64); 4] = [("third", "Third", 1.0 / 3.0), ("half", "Half", 0.5), ("two_thirds", "Two thirds", 2.0 / 3.0), ("full", "Full", 1.0)];

fn width_picture(share: f64) -> gtk::Widget {
    picture(move |cr, w, h, ink| {
        // The screen, with the new column at its width and the next one peeking in.
        let (x0, y0, sw, sh) = (12.0, 10.0, w - 24.0, h - 20.0);
        cr.set_source_rgba(ink.0, ink.1, ink.2, 0.35);
        cr.set_line_width(1.0);
        rrect(cr, x0, y0, sw, sh, 5.0);
        let _ = cr.stroke();
        let cw = (sw - 6.0) * share;
        window(cr, x0 + 3.0, y0 + 3.0, cw, sh - 6.0, ink, true);
        let nx = x0 + 3.0 + cw + 3.0;
        if nx < x0 + sw - 6.0 {
            window(cr, nx, y0 + 3.0, x0 + sw - 3.0 - nx, sh - 6.0, ink, false);
        }
    })
}
