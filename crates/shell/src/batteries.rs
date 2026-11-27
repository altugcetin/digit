//! Battery rings, shared by Control Center and the desktop widget.

use crate::app::App;
use crate::config::Config;
use crate::devices::Gadget;
use crate::draw::{self, Face};
use gtk::{gsk, pango, prelude::*};
use kit::palette::{Palette, Rgb};

const GREEN: Rgb = Rgb::new(0.19, 0.82, 0.35);
const RED: Rgb = Rgb::new(1.0, 0.27, 0.23);

/// Device batteries, plus the phone's while KDE Connect reaches it.
pub fn list(app: &App) -> Vec<Gadget> {
    let mut list = app.gadgets.borrow().clone();
    // UPower can lag behind the kernel, so the laptop uses the same reading as the bar.
    list.retain(|g| !g.own);
    if let Some(b) = app.battery.get() {
        list.insert(0, Gadget { path: "own".into(), glyph: "laptop", name: "This Computer".into(), level: b.level, coarse: None, charging: b.powered(), own: true });
    }
    if let Some(p) = crate::phone::phone().filter(|p| p.reachable)
        && let Some((level, charging)) = p.battery
    {
        list.push(crate::devices::Gadget { path: format!("kdeconnect:{}", p.id), glyph: p.glyph(), name: p.name, level: level.clamp(0, 100) as u8, coarse: None, charging, own: false });
    }
    list
}

/// One device: a ring around its glyph, its level (unless not `levels`), and its name, centered
/// on `cx` in a cell `cw` wide. `disc` fills the ring for a panel; on the wallpaper it stays
/// clear.
#[allow(clippy::too_many_arguments)]
pub fn cell(s: &gtk::Snapshot, widget: &impl IsA<gtk::Widget>, cfg: &Config, pal: &Palette, g: &Gadget, cx: f64, cy: f64, cw: f64, disc: bool, levels: bool) {
    let low = !g.charging && g.level <= 20;
    let tint = if g.charging { GREEN } else if low { RED } else { pal.ink_main };
    if disc {
        draw::fill(s, &draw::round(cx - 26.0, cy - 26.0, 52.0, 52.0, 26.0), pal.tile(0.05));
    }
    let ring = |frac: f64| {
        let pb = gsk::PathBuilder::new();
        let r = 22.0f64;
        let start = -std::f64::consts::FRAC_PI_2;
        let steps = (64.0 * frac).ceil().max(1.0) as usize;
        for k in 0..=steps {
            let a = start + frac * std::f64::consts::TAU * k as f64 / steps as f64;
            let (px, py) = ((cx + r * a.cos()) as f32, (cy + r * a.sin()) as f32);
            if k == 0 { pb.move_to(px, py) } else { pb.line_to(px, py) }
        }
        pb.to_path()
    };
    let stroke = gsk::Stroke::new(3.5);
    stroke.set_line_cap(gsk::LineCap::Round);
    s.append_stroke(&ring(1.0), &stroke, &pal.ink_main.alpha(0.12).rgba());
    s.append_stroke(&ring((g.level as f64 / 100.0).clamp(0.02, 1.0)), &stroke, &tint.rgba());
    draw::icon(s, widget, cfg, g.glyph, 20.0, true, pal.ink_main, cx, cy);
    let value = match g.coarse {
        Some(l) => l.to_string(),
        None => format!("{}%", g.level),
    };
    if levels {
        draw::text(s, widget, cfg, &value, 13.0, Face::Number(560), if low { RED } else { pal.ink_main }, cx - cw / 2.0 + 3.0, cy + 39.0, cw - 6.0, pango::Alignment::Center);
    }
    let name_y = if levels { cy + 56.0 } else { cy + 39.0 };
    draw::text(s, widget, cfg, &g.name, 11.5, Face::Text(450), pal.ink_meta, cx - cw / 2.0 + 3.0, name_y, cw - 6.0, pango::Alignment::Center);
}
