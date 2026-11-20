use gtk::{cairo, glib, pango};

#[link(name = "pangocairo-1.0")]
unsafe extern "C" {
    fn pango_cairo_create_layout(cr: *mut cairo::ffi::cairo_t) -> *mut pango::ffi::PangoLayout;
    fn pango_cairo_show_layout(cr: *mut cairo::ffi::cairo_t, layout: *mut pango::ffi::PangoLayout);
    fn pango_cairo_layout_path(cr: *mut cairo::ffi::cairo_t, layout: *mut pango::ffi::PangoLayout);
}

pub fn layout(cr: &cairo::Context, text: &str, font: &str) -> pango::Layout {
    // SAFETY: `cr` is a live cairo context; pango_cairo_create_layout returns a new layout with a reference we take over.
    let l: pango::Layout = unsafe { glib::translate::from_glib_full(pango_cairo_create_layout(cr.to_raw_none())) };
    l.set_text(text);
    l.set_font_description(Some(&pango::FontDescription::from_string(font)));
    l
}

pub fn show(cr: &cairo::Context, l: &pango::Layout) {
    // SAFETY: both pointers come from live, borrowed objects and are only used for the duration of the call.
    unsafe { pango_cairo_show_layout(cr.to_raw_none(), glib::translate::ToGlibPtr::to_glib_none(l).0) }
}

pub fn path(cr: &cairo::Context, l: &pango::Layout) {
    // SAFETY: both pointers come from live, borrowed objects and are only used for the duration of the call.
    unsafe { pango_cairo_layout_path(cr.to_raw_none(), glib::translate::ToGlibPtr::to_glib_none(l).0) }
}

pub fn rgb(hex: &str) -> [f64; 3] {
    let v = |i: usize| u8::from_str_radix(hex.get(i..i + 2).unwrap_or("80"), 16).unwrap_or(128) as f64 / 255.0;
    [v(1), v(3), v(5)]
}

pub fn mix(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

pub fn rounded(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
    cr.arc(x + r, y + r, r, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
    cr.close_path();
}
