use gtk::{gdk, gio, prelude::*};
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static CACHE: RefCell<HashMap<(String, i32), gdk::Texture>> = RefCell::new(HashMap::new());
    static ACCENT: RefCell<Option<String>> = const { RefCell::new(None) };
    static GLYPHS: std::cell::Cell<&'static str> = const { std::cell::Cell::new("Material Symbols Rounded") };
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
    let tex = kit::folder::texture(&accent(), glyph, size)?;
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
    match folder(kit::folder::glyph(file), px * sf) {
        Some(t) => img.set_paintable(Some(&t)),
        None => img.set_icon_name(Some("folder")),
    }
    img.set_pixel_size(px);
}
