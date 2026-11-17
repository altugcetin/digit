use crate::config::Config;
use gtk::{gdk, glib, graphene, gsk, pango, prelude::*};
use kit::palette::Rgb;
use std::cell::RefCell;
use std::collections::HashMap;

#[derive(Hash, PartialEq, Eq)]
struct Key {
    widget: usize,
    text: String,
    px: u32,
    face: (u8, i32),
    glyphs: &'static str,
    width: i32,
    align: u8,
}

thread_local! {
    /// Layouts by widget address, each with that widget held weakly: a new widget can land on
    /// a dead one's address, and must not get layouts made from the dead one's Pango context.
    static LAYOUTS: RefCell<HashMap<Key, (glib::WeakRef<gtk::Widget>, pango::Layout)>> = RefCell::new(HashMap::new());
    static RASTER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

static RASTER_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Turns the cairo path of [`rasterized`] on or off: the "Draw desktop text with cairo" setting.
pub fn set_raster(on: bool) {
    RASTER_ON.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Runs `draw`, with its text and icons rasterized by cairo instead of GSK's glyph cache when the
/// setting is on (it is off by default).
///
/// For surfaces that redraw rarely, like the desktop clock and widgets: with GTK's OpenGL
/// renderer on NVIDIA, a frame sometimes draws with glyphs missing (cause unknown; it doesn't
/// happen with Vulkan), and such a frame stays on screen until the next redraw, up to a minute
/// later. A cairo node is drawn on the CPU and uploaded as a plain texture, so it never touches
/// the renderer's glyph atlas.
pub fn rasterized(draw: impl FnOnce()) {
    let was = RASTER.with(|r| r.replace(RASTER_ON.load(std::sync::atomic::Ordering::Relaxed)));
    draw();
    RASTER.with(|r| r.set(was));
}

/// Draws `l` at the snapshot's origin, through cairo inside [`rasterized`].
fn put(s: &gtk::Snapshot, l: &pango::Layout, color: Rgb) {
    if !RASTER.with(|r| r.get()) {
        s.append_layout(l, &color.rgba());
        return;
    }
    let (ink, logical) = l.pixel_extents();
    let x0 = ink.x().min(logical.x()) - 2;
    let y0 = ink.y().min(logical.y()) - 2;
    let x1 = (ink.x() + ink.width()).max(logical.x() + logical.width()) + 2;
    let y1 = (ink.y() + ink.height()).max(logical.y() + logical.height()) + 2;
    let cr = s.append_cairo(&graphene::Rect::new(x0 as f32, y0 as f32, (x1 - x0) as f32, (y1 - y0) as f32));
    cr.set_source_rgba(color.r, color.g, color.b, color.a);
    kit::paint::show(&cr, l);
}

fn face_key(face: Face) -> (u8, i32) {
    match face {
        Face::Text(w) => (0, w),
        Face::Number(w) => (1, w),
        Face::Icon(f) => (2, f as i32),
    }
}

fn cached(widget: &impl IsA<gtk::Widget>, cfg: &Config, text: &str, px: f64, face: Face, width: i32, align: pango::Alignment) -> pango::Layout {
    let key = Key {
        widget: widget.as_ref().as_ptr() as usize,
        text: text.to_string(),
        px: (px * 64.0) as u32,
        face: face_key(face),
        glyphs: kit::style::glyph_family(),
        width,
        align: match align {
            pango::Alignment::Left => 0,
            pango::Alignment::Center => 1,
            _ => 2,
        },
    };
    let widget = widget.as_ref();
    let hit = LAYOUTS.with(|c| c.borrow().get(&key).filter(|(owner, _)| owner.upgrade().as_ref() == Some(widget)).map(|(_, l)| l.clone()));
    if let Some(l) = hit {
        return l;
    }
    let l = layout(widget, cfg, text, px, face);
    if width > 0 {
        l.set_width(width * pango::SCALE);
        l.set_ellipsize(pango::EllipsizeMode::End);
        l.set_single_paragraph_mode(true);
        l.set_alignment(align);
    }
    LAYOUTS.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() >= 768 {
            c.clear();
        }
        c.insert(key, (widget.downgrade(), l.clone()));
    });
    l
}

/// Invalidate hand-drawn glyphs in every open panel, including idle canvases.
pub fn glyphs_changed() {
    LAYOUTS.with(|cache| cache.borrow_mut().clear());
    fn redraw(widget: &gtk::Widget) {
        widget.queue_draw();
        let mut child = widget.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            redraw(&widget);
        }
    }
    for window in gtk::Window::list_toplevels() { redraw(&window); }
}

pub fn forget(widget: &impl IsA<gtk::Widget>) {
    let id = widget.as_ref().as_ptr() as usize;
    LAYOUTS.with(|c| c.borrow_mut().retain(|k, _| k.widget != id));
}

pub fn shaped(widget: &impl IsA<gtk::Widget>, cfg: &Config, text: &str, px: f64, face: Face) -> pango::Layout {
    cached(widget, cfg, text, px, face, 0, pango::Alignment::Left)
}

#[derive(Clone, Copy)]
pub enum Face {
    Text(i32),
    Number(i32),
    Icon(bool),
}

fn weight(w: i32) -> pango::Weight {
    if w >= 550 {
        pango::Weight::Semibold
    } else if w >= 480 {
        pango::Weight::Medium
    } else {
        pango::Weight::Normal
    }
}

pub fn layout(widget: &impl IsA<gtk::Widget>, cfg: &Config, text: &str, px: f64, face: Face) -> pango::Layout {
    let l = widget.as_ref().create_pango_layout(Some(text));
    let mut fd = pango::FontDescription::new();
    fd.set_absolute_size(px * pango::SCALE as f64);
    match face {
        Face::Text(w) => {
            fd.set_family(&cfg.font);
            fd.set_variations(Some(&format!("wght={w}")));
            fd.set_weight(weight(w));
        }
        Face::Number(w) => {
            fd.set_family(&cfg.numbers);
            fd.set_variations(Some(&format!("wght={w}")));
            fd.set_weight(weight(w));
        }
        Face::Icon(fill) => {
            fd.set_family(kit::style::glyph_family());
            fd.set_variations(Some(&format!("FILL={},wght=400,opsz={}", fill as i32, px.clamp(20.0, 48.0))));
        }
    }
    l.set_font_description(Some(&fd));
    if matches!(face, Face::Number(_)) {
        let attrs = pango::AttrList::new();
        attrs.insert(pango::AttrFontFeatures::new("tnum 1"));
        l.set_attributes(Some(&attrs));
    }
    l
}

pub fn width(widget: &impl IsA<gtk::Widget>, cfg: &Config, text: &str, px: f64, face: Face) -> f64 {
    shaped(widget, cfg, text, px, face).pixel_extents().1.width() as f64
}

#[allow(clippy::too_many_arguments)]
pub fn text(s: &gtk::Snapshot, widget: &impl IsA<gtk::Widget>, cfg: &Config, text: &str, px: f64, face: Face, color: Rgb, x: f64, mid: f64, width: f64, align: pango::Alignment) {
    if width <= 4.0 || text.is_empty() {
        return;
    }
    let l = cached(widget, cfg, text, px, face, width.round().max(1.0) as i32, align);
    let h = l.pixel_extents().1.height() as f64;
    s.save();
    s.translate(&graphene::Point::new(x.round() as f32, (mid - h / 2.0).round() as f32));
    put(s, &l, color);
    s.restore();
}

#[allow(clippy::too_many_arguments)]
pub fn icon(s: &gtk::Snapshot, widget: &impl IsA<gtk::Widget>, cfg: &Config, name: &str, px: f64, fill: bool, color: Rgb, cx: f64, mid: f64) {
    let l = shaped(widget, cfg, name, px, Face::Icon(fill));
    let (ink, _) = l.pixel_extents();
    s.save();
    s.translate(&graphene::Point::new(
        (cx - ink.x() as f64 - ink.width() as f64 / 2.0).round() as f32,
        (mid - ink.y() as f64 - ink.height() as f64 / 2.0).round() as f32,
    ));
    put(s, &l, color);
    s.restore();
}

pub fn css(c: Rgb) -> String {
    format!("rgba({},{},{},{:.3})", (c.r * 255.0).round(), (c.g * 255.0).round(), (c.b * 255.0).round(), c.a)
}

pub fn slab(s: &gtk::Snapshot, parts: &kit::shape::Parts, color: Rgb) {
    let (body, coves) = parts;
    s.push_rounded_clip(body);
    s.append_color(&color.rgba(), body.bounds());
    s.pop();
    if let Some(c) = coves {
        s.append_fill(c, gsk::FillRule::Winding, &color.rgba());
    }
}

pub fn round(x: f64, y: f64, w: f64, h: f64, r: f64) -> gsk::RoundedRect {
    gsk::RoundedRect::from_rect(graphene::Rect::new(x as f32, y as f32, w as f32, h as f32), r as f32)
}

pub fn fill(s: &gtk::Snapshot, shape: &gsk::RoundedRect, color: Rgb) {
    s.push_rounded_clip(shape);
    s.append_color(&color.rgba(), shape.bounds());
    s.pop();
}

pub fn app_icon(apps: &crate::apps::Apps, app_id: &str, px: i32, scale: i32) -> Option<gdk::Paintable> {
    let display = gdk::Display::default()?;
    let theme = gtk::IconTheme::for_display(&display);
    let gicon = apps.find(app_id).and_then(|i| apps.list().get(i)?.icon.clone());
    let paintable = match gicon {
        Some(g) => theme.lookup_by_gicon(&g, px, scale, gtk::TextDirection::Ltr, gtk::IconLookupFlags::empty()),
        None if !app_id.is_empty() && theme.has_icon(app_id) => theme.lookup_icon(app_id, &[], px, scale, gtk::TextDirection::Ltr, gtk::IconLookupFlags::empty()),
        None => return None,
    };
    Some(paintable.upcast())
}

pub fn paintable(s: &gtk::Snapshot, p: &gdk::Paintable, x: f64, y: f64, size: f64) {
    s.save();
    s.translate(&graphene::Point::new(x as f32, y as f32));
    p.snapshot(s, size, size);
    s.restore();
}

#[allow(clippy::too_many_arguments)]
pub fn monogram(s: &gtk::Snapshot, widget: &impl IsA<gtk::Widget>, cfg: &Config, name: &str, x: f64, y: f64, size: f64, back: Rgb, ink: Rgb) {
    fill(s, &round(x, y, size, size, size * 0.26), back);
    let letter: String = name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "·".into());
    text(s, widget, cfg, &letter, (size * 0.45).round(), Face::Text(650), ink, x, y + size / 2.0, size, pango::Alignment::Center);
}
