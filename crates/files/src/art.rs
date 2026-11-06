use gtk::{cairo, gdk, gio, glib, prelude::*};
use kit::paint::{layout, mix, rgb, rounded, show};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

thread_local! {
    static CACHE: RefCell<HashMap<(String, i32), gdk::Texture>> = RefCell::new(HashMap::new());
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
    static ACCENT: RefCell<Option<String>> = const { RefCell::new(None) };
    static GLYPHS: std::cell::Cell<&'static str> = const { std::cell::Cell::new("Material Symbols Rounded") };
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

fn accent() -> String {
    ACCENT.with(|a| {
        let mut a = a.borrow_mut();
        if a.is_none() {
            *a = Some(kit::style::ink().accent);
        }
        a.clone().unwrap_or_default()
    })
}

pub fn folder(glyph: &str, size: i32) -> Option<gdk::Texture> {
    let size = size.clamp(16, 512);
    let key = (glyph.to_string(), size);
    if let Some(t) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return Some(t);
    }
    let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, size, size).ok()?;
    {
        let cr = cairo::Context::new(&surface).ok()?;
        let s = size as f64;
        let a = rgb(&accent());
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
            let l = layout(&cr, glyph, &format!("{} {}px", kit::style::glyph_family(), (s * 0.32).round()));
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
    let tex: gdk::Texture = gdk::MemoryTexture::new(size, size, gdk::MemoryFormat::B8g8r8a8Premultiplied, &glib::Bytes::from_owned(data), stride).upcast();
    CACHE.with(|c| c.borrow_mut().insert(key, tex.clone()));
    Some(tex)
}

pub fn refresh_accent() {
    let next = kit::style::ink().accent;
    let changed = ACCENT.with(|a| {
        let mut current = a.borrow_mut();
        if current.as_ref() == Some(&next) {
            return false;
        }
        *current = Some(next);
        true
    });
    let family = kit::style::glyph_family();
    let glyphs_changed = GLYPHS.with(|current| current.replace(family) != family);
    if !changed && !glyphs_changed {
        return;
    }

    let old = CACHE.with(|cache| std::mem::take(&mut *cache.borrow_mut()));
    if old.is_empty() {
        return;
    }
    // Keep existing file rows and selection intact. Only replace folder artwork
    // still showing one of our old textures, including in the preview windows.
    let icons: HashMap<gdk::Paintable, (String, i32)> = old.into_iter()
        .map(|(key, texture)| (texture.upcast(), key))
        .collect();
    for window in gtk::Window::list_toplevels() {
        recolor(&window, &icons);
    }
}

fn recolor(widget: &gtk::Widget, icons: &HashMap<gdk::Paintable, (String, i32)>) {
    if let Some(image) = widget.downcast_ref::<gtk::Image>()
        && let Some(paintable) = image.paintable()
        && let Some((glyph, size)) = icons.get(&paintable)
        && let Some(texture) = folder(glyph, *size)
    {
        image.set_paintable(Some(&texture));
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        recolor(&widget, icons);
    }
}

pub fn set(img: &gtk::Image, file: &gio::File, px: i32) {
    let sf = img.scale_factor().max(1);
    match folder(glyph(file), px * sf) {
        Some(t) => img.set_paintable(Some(&t)),
        None => img.set_icon_name(Some("folder")),
    }
    img.set_pixel_size(px);
}
