//! Digit's folder artwork: the accent-colored folder with an optional glyph, used by Files and
//! the desktop.

use crate::paint::{layout, mix, rgb, rounded, show};
use gtk::{cairo, gdk, gio, glib, prelude::*};
use std::path::PathBuf;

thread_local! {
    static SPECIAL: Vec<(PathBuf, &'static str)> = {
        let mut v = vec![(glib::home_dir(), "home")];
        for (d, g) in [
            (glib::UserDirectory::Desktop, "desktop_windows"),
            (glib::UserDirectory::Documents, "description"),
            (glib::UserDirectory::Downloads, "download"),
            (glib::UserDirectory::Music, "music_note"),
            (glib::UserDirectory::Pictures, "image"),
            (glib::UserDirectory::Videos, "movie"),
            (glib::UserDirectory::Templates, "file_copy"),
            (glib::UserDirectory::PublicShare, "group"),
        ] {
            if let Some(p) = glib::user_special_dir(d) {
                if p != glib::home_dir() {
                    v.push((p, g));
                }
            }
        }
        v
    };
}

pub fn glyph(file: &gio::File) -> &'static str {
    if let Some(p) = file.path() {
        if let Some(g) = SPECIAL.with(|s| s.iter().find(|(sp, _)| *sp == p).map(|(_, g)| *g)) {
            return g;
        }
        if p == std::path::Path::new("/") {
            return "hard_drive";
        }
    }
    let name = file.basename().map(|b| b.to_string_lossy().to_lowercase()).unwrap_or_default();
    match name.as_str() {
        "screenshots" => "screenshot_region",
        "wallpapers" | "backgrounds" => "wallpaper",
        "projects" | "code" | "src" | "dev" | "repos" | "git" => "code",
        "games" => "sports_esports",
        "applications" | "apps" => "apps",
        "books" => "menu_book",
        "backups" | "backup" => "backup",
        "fonts" | ".fonts" => "font_download",
        "dropbox" | "nextcloud" | "onedrive" | "google drive" => "cloud",
        "camera" | "photos" | "dcim" => "photo_camera",
        "recordings" | "podcasts" => "mic",
        _ => "",
    }
}

/// Draws the folder for an accent color ("#rrggbb"). Callers keep their own cache.
pub fn texture(accent: &str, glyph: &str, size: i32) -> Option<gdk::Texture> {
    let size = size.clamp(16, 512);
    let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, size, size).ok()?;
    {
        let cr = cairo::Context::new(&surface).ok()?;
        let s = size as f64;
        let a = rgb(accent);
        let back = mix(a, [0.0, 0.0, 0.0], 0.30);
        let top = mix(a, [1.0, 1.0, 1.0], 0.16);
        let bottom = mix(a, [0.0, 0.0, 0.0], 0.06);
        let (x, w) = (s * 0.06, s * 0.88);
        rounded(&cr, x, s * 0.13, w * 0.40, s * 0.16, s * 0.045);
        cr.set_source_rgb(back[0], back[1], back[2]);
        cr.fill().ok()?;
        rounded(&cr, x, s * 0.18, w, s * 0.64, s * 0.07);
        cr.set_source_rgb(back[0], back[1], back[2]);
        cr.fill().ok()?;
        rounded(&cr, x + s * 0.03, s * 0.21, w - s * 0.06, s * 0.2, s * 0.04);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.85);
        cr.fill().ok()?;
        for (dy, alpha) in [(s * 0.025, 0.10), (s * 0.012, 0.16)] {
            rounded(&cr, x, s * 0.28 + dy, w, s * 0.58, s * 0.075);
            cr.set_source_rgba(0.0, 0.0, 0.0, alpha);
            cr.fill().ok()?;
        }
        rounded(&cr, x, s * 0.28, w, s * 0.58, s * 0.075);
        let g = cairo::LinearGradient::new(0.0, s * 0.28, 0.0, s * 0.86);
        g.add_color_stop_rgb(0.0, top[0], top[1], top[2]);
        g.add_color_stop_rgb(1.0, bottom[0], bottom[1], bottom[2]);
        cr.set_source(&g).ok()?;
        cr.fill().ok()?;
        cr.move_to(x + s * 0.07, s * 0.28 + 0.75);
        cr.line_to(x + w - s * 0.07, s * 0.28 + 0.75);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.35);
        cr.set_line_width(1.0_f64.max(s / 96.0));
        cr.stroke().ok()?;
        if !glyph.is_empty() && size >= 28 {
            let l = layout(&cr, glyph, &format!("{} {}px", crate::style::glyph_family(), (s * 0.32).round()));
            let (tw, th) = l.pixel_size();
            let (gx, gy) = (s / 2.0 - tw as f64 / 2.0, s * 0.57 - th as f64 / 2.0);
            let ink = mix(a, [0.0, 0.0, 0.0], 0.52);
            cr.move_to(gx, gy + (s * 0.012).max(1.0));
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.28);
            show(&cr, &l);
            cr.move_to(gx, gy);
            cr.set_source_rgba(ink[0], ink[1], ink[2], 0.78);
            show(&cr, &l);
        }
    }
    surface.flush();
    let stride = surface.stride() as usize;
    let data = surface.data().ok()?.to_vec();
    let texture: gdk::Texture = gdk::MemoryTexture::new(size, size, gdk::MemoryFormat::B8g8r8a8Premultiplied, &glib::Bytes::from_owned(data), stride).upcast();
    Some(texture)
}
