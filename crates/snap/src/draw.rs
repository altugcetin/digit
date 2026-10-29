use gtk::{cairo, pango};

pub use kit::paint::{layout, show};

fn outline(cr: &cairo::Context, l: &pango::Layout) {
    kit::paint::path(cr, l)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Select,
    Pen,
    Marker,
    Line,
    Arrow,
    Rect,
    Ellipse,
    Text,
    Step,
    Pixelate,
    Blur,
    Spotlight,
    Sticker,
    Crop,
}

pub const TOOLS: [(Tool, &str, &str, char); 14] = [
    (Tool::Select, "arrow_selector_tool", "Select and move", 'v'),
    (Tool::Arrow, "north_east", "Arrow", 'a'),
    (Tool::Rect, "rectangle", "Rectangle", 'r'),
    (Tool::Ellipse, "circle", "Ellipse", 'o'),
    (Tool::Line, "horizontal_rule", "Line", 'l'),
    (Tool::Pen, "draw", "Pen", 'p'),
    (Tool::Marker, "ink_highlighter", "Highlighter", 'h'),
    (Tool::Text, "title", "Text", 't'),
    (Tool::Step, "counter_1", "Numbered step", 'n'),
    (Tool::Sticker, "add_reaction", "Sticker", 'e'),
    (Tool::Pixelate, "blur_on", "Pixelate (permanent)", 'x'),
    (Tool::Blur, "blur_circular", "Blur (permanent)", 'b'),
    (Tool::Spotlight, "highlight", "Spotlight", 's'),
    (Tool::Crop, "crop", "Crop", 'c'),
];

pub type Rgba = [f64; 4];

#[derive(Clone, Debug)]
pub struct Mark {
    pub tool: Tool,
    pub pts: Vec<(f64, f64)>,
    pub color: Rgba,
    pub width: f64,
    pub fill: bool,
    pub text: String,
    pub n: u32,
}

impl Mark {
    pub fn new(tool: Tool, at: (f64, f64), color: Rgba, width: f64, fill: bool) -> Mark {
        Mark { tool, pts: vec![at, at], color, width, fill, text: String::new(), n: 0 }
    }

    pub fn rect(&self) -> (f64, f64, f64, f64) {
        let (a, b) = (self.pts[0], *self.pts.last().unwrap_or(&self.pts[0]));
        (a.0.min(b.0), a.1.min(b.1), (a.0 - b.0).abs(), (a.1 - b.1).abs())
    }

    pub fn bounds(&self, cr: Option<&cairo::Context>) -> (f64, f64, f64, f64) {
        let pad = self.width * 2.0 + 6.0;
        match self.tool {
            Tool::Text | Tool::Sticker => {
                let (w, h) = cr.map(|cr| {
                    let l = layout(cr, &self.text, &self.font());
                    let (w, h) = l.pixel_size();
                    (w as f64, h as f64)
                }).unwrap_or((self.text.chars().count() as f64 * self.size() * 0.6, self.size() * 1.4));
                (self.pts[0].0 - 10.0, self.pts[0].1 - 6.0, w + 20.0, h + 12.0)
            }
            Tool::Step => {
                let r = self.radius();
                (self.pts[0].0 - r, self.pts[0].1 - r, r * 2.0, r * 2.0)
            }
            Tool::Pen | Tool::Marker => {
                let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
                for p in &self.pts {
                    x0 = x0.min(p.0);
                    y0 = y0.min(p.1);
                    x1 = x1.max(p.0);
                    y1 = y1.max(p.1);
                }
                (x0 - pad, y0 - pad, x1 - x0 + pad * 2.0, y1 - y0 + pad * 2.0)
            }
            _ => {
                let (x, y, w, h) = self.rect();
                (x - pad, y - pad, w + pad * 2.0, h + pad * 2.0)
            }
        }
    }

    pub fn shift(&mut self, dx: f64, dy: f64) {
        for p in &mut self.pts {
            p.0 += dx;
            p.1 += dy;
        }
    }

    pub fn size(&self) -> f64 {
        12.0 + self.width * 4.0
    }

    fn font(&self) -> String {
        match self.tool {
            Tool::Sticker => format!("Noto Color Emoji {}px", (self.size() * 2.4).round()),
            _ => format!("Readex Pro Bold {}px", self.size().round()),
        }
    }

    fn radius(&self) -> f64 {
        11.0 + self.width * 2.4
    }

    pub fn trivial(&self) -> bool {
        match self.tool {
            Tool::Text | Tool::Sticker | Tool::Step => false,
            Tool::Pen | Tool::Marker => self.pts.len() < 2,
            _ => {
                let (_, _, w, h) = self.rect();
                w < 3.0 && h < 3.0
            }
        }
    }
}

pub struct Base {
    pub surface: cairo::ImageSurface,
    pub pixels: Vec<u8>,
    pub stride: usize,
    pub w: i32,
    pub h: i32,
}

impl Base {
    pub fn new(mut data: Vec<u8>, w: i32, h: i32, stride: i32) -> Option<Base> {
        let need = (stride * h) as usize;
        if data.len() < need {
            data.resize(need, 0);
        }
        let pixels = data.clone();
        let surface = cairo::ImageSurface::create_for_data(data, cairo::Format::ARgb32, w, h, stride).ok()?;
        Some(Base { surface, pixels, stride: stride as usize, w, h })
    }

    pub fn pixel(&self, x: i32, y: i32) -> (u8, u8, u8) {
        let (x, y) = (x.clamp(0, self.w - 1) as usize, y.clamp(0, self.h - 1) as usize);
        let i = y * self.stride + x * 4;
        (self.pixels[i + 2], self.pixels[i + 1], self.pixels[i])
    }
}

pub fn luminance(c: Rgba) -> f64 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
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

fn src(cr: &cairo::Context, c: Rgba) {
    cr.set_source_rgba(c[0], c[1], c[2], c[3]);
}

fn pixelate(cr: &cairo::Context, base: &Base, m: &Mark) {
    let (x, y, w, h) = m.rect();
    let block = (6.0 + m.width * 3.0).round().max(4.0) as i32;
    let (x0, y0) = (x.floor() as i32, y.floor() as i32);
    let (x1, y1) = ((x + w).ceil() as i32, (y + h).ceil() as i32);
    let mut by = y0;
    while by < y1 {
        let mut bx = x0;
        while bx < x1 {
            let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
            let mut sy = by;
            while sy < (by + block).min(y1) {
                let mut sx = bx;
                while sx < (bx + block).min(x1) {
                    let p = base.pixel(sx, sy);
                    r += p.0 as u32;
                    g += p.1 as u32;
                    b += p.2 as u32;
                    n += 1;
                    sx += 2;
                }
                sy += 2;
            }
            if n > 0 {
                cr.set_source_rgb(r as f64 / n as f64 / 255.0, g as f64 / n as f64 / 255.0, b as f64 / n as f64 / 255.0);
                cr.rectangle(bx as f64, by as f64, ((bx + block).min(x1) - bx) as f64, ((by + block).min(y1) - by) as f64);
                let _ = cr.fill();
            }
            bx += block;
        }
        by += block;
    }
}

fn blur(cr: &cairo::Context, base: &Base, m: &Mark) {
    let (x, y, w, h) = m.rect();
    if w < 2.0 || h < 2.0 {
        return;
    }
    let k = 8.0 + m.width * 2.0;
    let (sw, sh) = ((w / k).ceil().max(1.0) as i32, (h / k).ceil().max(1.0) as i32);
    let Ok(small) = cairo::ImageSurface::create(cairo::Format::ARgb32, sw, sh) else { return };
    if let Ok(c) = cairo::Context::new(&small) {
        c.scale(sw as f64 / w, sh as f64 / h);
        let _ = c.set_source_surface(&base.surface, -x, -y);
        c.source().set_filter(cairo::Filter::Good);
        let _ = c.paint();
    }
    let Ok(mid) = cairo::ImageSurface::create(cairo::Format::ARgb32, (sw * 3).max(1), (sh * 3).max(1)) else { return };
    if let Ok(c) = cairo::Context::new(&mid) {
        c.scale(3.0, 3.0);
        let _ = c.set_source_surface(&small, 0.0, 0.0);
        c.source().set_filter(cairo::Filter::Bilinear);
        let _ = c.paint();
    }
    cr.save().ok();
    cr.rectangle(x, y, w, h);
    cr.clip();
    cr.translate(x, y);
    cr.scale(w / (sw * 3) as f64, h / (sh * 3) as f64);
    let _ = cr.set_source_surface(&mid, 0.0, 0.0);
    cr.source().set_filter(cairo::Filter::Bilinear);
    let _ = cr.paint();
    cr.restore().ok();
}

fn arrow(cr: &cairo::Context, m: &Mark) {
    let (a, b) = (m.pts[0], m.pts[1]);
    let ang = (b.1 - a.1).atan2(b.0 - a.0);
    let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
    let head = (m.width * 3.6 + 10.0).min(len * 0.6);
    let wing = 0.45;
    let base = (b.0 - head * 0.82 * ang.cos(), b.1 - head * 0.82 * ang.sin());
    src(cr, m.color);
    cr.set_line_width(m.width);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.move_to(a.0, a.1);
    cr.line_to(base.0, base.1);
    let _ = cr.stroke();
    cr.move_to(b.0, b.1);
    cr.line_to(b.0 - head * (ang - wing).cos(), b.1 - head * (ang - wing).sin());
    cr.line_to(base.0, base.1);
    cr.line_to(b.0 - head * (ang + wing).cos(), b.1 - head * (ang + wing).sin());
    cr.close_path();
    cr.set_line_join(cairo::LineJoin::Round);
    cr.set_line_width(m.width.min(4.0));
    let _ = cr.stroke_preserve();
    let _ = cr.fill();
}

pub fn mark(cr: &cairo::Context, base: &Base, m: &Mark) {
    cr.save().ok();
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    match m.tool {
        Tool::Pen | Tool::Marker => {
            if m.tool == Tool::Marker {
                cr.set_source_rgba(m.color[0], m.color[1], m.color[2], 0.38);
                cr.set_line_width(m.width * 5.0 + 6.0);
                cr.set_line_cap(cairo::LineCap::Butt);
            } else {
                src(cr, m.color);
                cr.set_line_width(m.width);
            }
            if let Some(f) = m.pts.first() {
                cr.move_to(f.0, f.1);
                for p in &m.pts[1..] {
                    cr.line_to(p.0, p.1);
                }
                let _ = cr.stroke();
            }
        }
        Tool::Line => {
            src(cr, m.color);
            cr.set_line_width(m.width);
            cr.move_to(m.pts[0].0, m.pts[0].1);
            cr.line_to(m.pts[1].0, m.pts[1].1);
            let _ = cr.stroke();
        }
        Tool::Arrow => arrow(cr, m),
        Tool::Rect => {
            let (x, y, w, h) = m.rect();
            rounded(cr, x, y, w, h, (4.0 + m.width).min(14.0));
            if m.fill {
                cr.set_source_rgba(m.color[0], m.color[1], m.color[2], 0.28);
                let _ = cr.fill_preserve();
            }
            src(cr, m.color);
            cr.set_line_width(m.width);
            let _ = cr.stroke();
        }
        Tool::Ellipse => {
            let (x, y, w, h) = m.rect();
            if w > 0.5 && h > 0.5 {
                cr.save().ok();
                cr.translate(x + w / 2.0, y + h / 2.0);
                cr.scale(w / 2.0, h / 2.0);
                cr.arc(0.0, 0.0, 1.0, 0.0, std::f64::consts::TAU);
                cr.restore().ok();
                if m.fill {
                    cr.set_source_rgba(m.color[0], m.color[1], m.color[2], 0.28);
                    let _ = cr.fill_preserve();
                }
                src(cr, m.color);
                cr.set_line_width(m.width);
                let _ = cr.stroke();
            }
        }
        Tool::Text => {
            let l = layout(cr, &m.text, &m.font());
            let (w, h) = l.pixel_size();
            let (x, y) = m.pts[0];
            if m.fill {
                rounded(cr, x - 10.0, y - 5.0, w as f64 + 20.0, h as f64 + 10.0, 10.0);
                src(cr, m.color);
                let _ = cr.fill();
                let dark = luminance(m.color) > 0.6;
                cr.set_source_rgb(if dark { 0.06 } else { 1.0 }, if dark { 0.06 } else { 1.0 }, if dark { 0.06 } else { 1.0 });
                cr.move_to(x, y);
                show(cr, &l);
            } else {
                cr.move_to(x, y);
                outline(cr, &l);
                let dark = luminance(m.color) > 0.6;
                cr.set_source_rgba(if dark { 0.0 } else { 1.0 }, if dark { 0.0 } else { 1.0 }, if dark { 0.0 } else { 1.0 }, 0.85);
                cr.set_line_width((m.size() / 6.0).max(2.0));
                let _ = cr.stroke();
                cr.move_to(x, y);
                src(cr, m.color);
                show(cr, &l);
            }
        }
        Tool::Step => {
            let (x, y) = m.pts[0];
            let r = m.radius();
            cr.arc(x, y + 1.5, r, 0.0, std::f64::consts::TAU);
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.25);
            let _ = cr.fill();
            cr.arc(x, y, r, 0.0, std::f64::consts::TAU);
            src(cr, m.color);
            let _ = cr.fill();
            let l = layout(cr, &m.n.to_string(), &format!("Readex Pro Bold {}px", (r * 1.05).round()));
            let (w, h) = l.pixel_size();
            let dark = luminance(m.color) > 0.6;
            cr.set_source_rgb(if dark { 0.05 } else { 1.0 }, if dark { 0.05 } else { 1.0 }, if dark { 0.05 } else { 1.0 });
            cr.move_to(x - w as f64 / 2.0, y - h as f64 / 2.0);
            show(cr, &l);
        }
        Tool::Sticker => {
            let l = layout(cr, &m.text, &m.font());
            cr.move_to(m.pts[0].0, m.pts[0].1);
            show(cr, &l);
        }
        Tool::Pixelate => pixelate(cr, base, m),
        Tool::Blur => blur(cr, base, m),
        Tool::Spotlight | Tool::Select | Tool::Crop => {}
    }
    cr.restore().ok();
}

pub fn spotlights(cr: &cairo::Context, w: f64, h: f64, marks: &[&Mark]) {
    let spots: Vec<&&Mark> = marks.iter().filter(|m| m.tool == Tool::Spotlight).collect();
    if spots.is_empty() {
        return;
    }
    cr.save().ok();
    cr.set_fill_rule(cairo::FillRule::EvenOdd);
    cr.rectangle(0.0, 0.0, w, h);
    for m in &spots {
        let (x, y, sw, sh) = m.rect();
        rounded(cr, x, y, sw, sh, 10.0);
    }
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.58);
    let _ = cr.fill();
    cr.restore().ok();
}

pub fn scene(cr: &cairo::Context, base: &Base, marks: &[Mark], live: Option<&Mark>) {
    let _ = cr.set_source_surface(&base.surface, 0.0, 0.0);
    let _ = cr.paint();
    let all: Vec<&Mark> = marks.iter().chain(live).collect();
    for m in all.iter().filter(|m| matches!(m.tool, Tool::Pixelate | Tool::Blur)) {
        mark(cr, base, m);
    }
    spotlights(cr, base.w as f64, base.h as f64, &all);
    for m in all.iter().filter(|m| !matches!(m.tool, Tool::Pixelate | Tool::Blur | Tool::Spotlight)) {
        mark(cr, base, m);
    }
}

pub const FRAMES: [(&str, &str, &str); 7] = [
    ("None", "", ""),
    ("Ocean", "#2193b0", "#6dd5ed"),
    ("Sunset", "#ff7e5f", "#feb47b"),
    ("Grape", "#8e2de2", "#4a00e0"),
    ("Mint", "#11998e", "#38ef7d"),
    ("Graphite", "#3a3a3c", "#0b0b0c"),
    ("Paper", "#f5f5f7", "#d9d9de"),
];

pub fn hex(h: &str) -> Rgba {
    let v = |i: usize| u8::from_str_radix(h.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0) as f64 / 255.0;
    [v(1), v(3), v(5), 1.0]
}

pub fn frame(cr: &cairo::Context, preset: usize, w: f64, h: f64, pad: f64) {
    let Some((_, a, b)) = FRAMES.get(preset) else { return };
    if a.is_empty() {
        return;
    }
    let (ca, cb) = (hex(a), hex(b));
    let g = cairo::LinearGradient::new(0.0, 0.0, w + pad * 2.0, h + pad * 2.0);
    g.add_color_stop_rgb(0.0, ca[0], ca[1], ca[2]);
    g.add_color_stop_rgb(1.0, cb[0], cb[1], cb[2]);
    let _ = cr.set_source(&g);
    cr.rectangle(0.0, 0.0, w + pad * 2.0, h + pad * 2.0);
    let _ = cr.fill();
    for i in 0..10 {
        let s = i as f64 * 2.4;
        rounded(cr, pad - s * 0.5, pad - s * 0.2 + 6.0, w + s, h + s, 14.0 + s);
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.035);
        let _ = cr.fill();
    }
}
