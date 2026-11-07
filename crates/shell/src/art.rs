use gtk::{gdk, graphene, gsk, prelude::*};
use kit::palette::Rgb;
use std::cell::RefCell;
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Style {
    Native,
    Light,
    Graphite,
    Mono,
    Tinted,
}

impl Style {
    pub fn parse(s: &str) -> Self {
        match s {
            "native" => Style::Native,
            "graphite" | "tile" => Style::Graphite,
            "mono" => Style::Mono,
            "tinted" => Style::Tinted,
            _ => Style::Light,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Shape {
    Free,
    Plate,
    Disc(Rgb),
}

#[derive(Clone, Copy)]
struct Look {
    shape: Shape,
    color: Option<Rgb>,
    bbox: (f64, f64, f64, f64),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct CacheKey {
    app: String,
    source: String,
    name: String,
    style: Style,
    size: i32,
    tint: u32,
}

fn cache_key(app: &str, source: &str, name: &str, style: Style, size: i32, accent: Rgb) -> CacheKey {
    let tint = if style == Style::Tinted {
        ((accent.r * 255.0) as u32) << 16 | ((accent.g * 255.0) as u32) << 8 | (accent.b * 255.0) as u32
    } else { 0 };
    CacheKey { app: app.into(), source: source.into(), name: name.into(), style, size, tint }
}

thread_local! {
    static LOOKS: RefCell<HashMap<(String, String), Look>> = RefCell::new(HashMap::new());
    static CACHE: RefCell<HashMap<CacheKey, gdk::Texture>> = RefCell::new(HashMap::new());
    static CPU: RefCell<Option<gsk::Renderer>> = const { RefCell::new(None) };
}

fn cpu() -> Option<gsk::Renderer> {
    CPU.with(|c| {
        if c.borrow().is_none() {
            let display = gdk::Display::default()?;
            let r = gsk::CairoRenderer::new();
            r.realize_for_display(&display).ok()?;
            *c.borrow_mut() = Some(r.upcast());
        }
        c.borrow().clone()
    })
}

pub fn forget() {
    CACHE.with(|c| c.borrow_mut().clear());
    LOOKS.with(|l| l.borrow_mut().clear());
}

fn render(renderer: &gsk::Renderer, size: f64, draw: impl FnOnce(&gtk::Snapshot)) -> Option<gdk::Texture> {
    let s = gtk::Snapshot::new();
    draw(&s);
    let node = s.to_node()?;
    Some(renderer.render_texture(&node, Some(&graphene::Rect::new(0.0, 0.0, size as f32, size as f32))))
}

fn analyze(renderer: &gsk::Renderer, icon: &gdk::Paintable) -> Look {
    const N: usize = 64;
    let Some(tex) = render(renderer, N as f64, |s| icon.snapshot(s, N as f64, N as f64)) else { return Look { shape: Shape::Free, color: None, bbox: (0.0, 0.0, 1.0, 1.0) } };
    let (w, h) = (tex.width().max(1) as usize, tex.height().max(1) as usize);
    let stride = w * 4;
    let mut px = vec![0u8; stride * h];
    tex.download(&mut px, stride);
    let alpha = |x: usize, y: usize| px[y.min(h - 1) * stride + x.min(w - 1) * 4 + 3];
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0usize, 0usize);
    let mut solid = 0usize;
    for y in 0..h {
        for x in 0..w {
            if alpha(x, y) > 200 {
                solid += 1;
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if solid == 0 || x1 <= x0 || y1 <= y0 {
        return Look { shape: Shape::Free, color: None, bbox: (0.0, 0.0, 1.0, 1.0) };
    }
    let (bw, bh) = ((x1 - x0) as f64, (y1 - y0) as f64);
    let fill = solid as f64 / (bw * bh);
    let square = (bw - bh).abs() <= bw.max(bh) * 0.12 && bw.max(bh) >= w as f64 * 0.5;
    let colour = |x: usize, y: usize| {
        let i = y.min(h - 1) * stride + x.min(w - 1) * 4;
        let a = (px[i + 3] as f64 / 255.0).max(1e-3);
        (px[i + 2] as f64 / 255.0 / a, px[i + 1] as f64 / 255.0 / a, px[i] as f64 / 255.0 / a)
    };
    let disc = || {
        let (cx, cy, r) = ((x0 + x1) as f64 / 2.0, (y0 + y1) as f64 / 2.0, bw.min(bh) / 2.0 - 2.5);
        let samples: Vec<(f64, f64, f64)> = (0..12)
            .map(|k| {
                let t = k as f64 / 12.0 * std::f64::consts::TAU;
                colour((cx + r * t.cos()) as usize, (cy + r * t.sin()) as usize)
            })
            .collect();
        let n = samples.len() as f64;
        let mean = samples.iter().fold((0.0, 0.0, 0.0), |a, c| (a.0 + c.0 / n, a.1 + c.1 / n, a.2 + c.2 / n));
        let spread = samples.iter().map(|c| (c.0 - mean.0).abs().max((c.1 - mean.1).abs()).max((c.2 - mean.2).abs())).fold(0.0f64, f64::max);
        (spread < 0.08).then(|| Rgb::new(mean.0.min(1.0), mean.1.min(1.0), mean.2.min(1.0)))
    };
    let shape = if square && fill >= 0.86 {
        Shape::Plate
    } else if square && (0.72..0.86).contains(&fill) {
        disc().map_or(Shape::Free, Shape::Disc)
    } else {
        Shape::Free
    };
    let bbox = (x0 as f64 / w as f64, y0 as f64 / h as f64, bw / w as f64, bh / h as f64);
    let mut bins = [(0.0f64, 0.0f64, 0.0f64, 0.0f64); 12];
    let mut seen = 0.0;
    for y in 0..h {
        for x in 0..w {
            let i = y * stride + x * 4;
            let a = px[i + 3] as f64 / 255.0;
            if a < 0.6 {
                continue;
            }
            let (b, g, r) = (px[i] as f64 / 255.0 / a, px[i + 1] as f64 / 255.0 / a, px[i + 2] as f64 / 255.0 / a);
            let (max, min) = (r.max(g).max(b), r.min(g).min(b));
            seen += 1.0;
            if max < 0.15 || max - min < 0.12 {
                continue;
            }
            let d = max - min;
            let hue = if max == r { ((g - b) / d).rem_euclid(6.0) } else if max == g { (b - r) / d + 2.0 } else { (r - g) / d + 4.0 };
            let weight = (d / max).powi(2);
            let bin = &mut bins[((hue * 2.0) as usize).min(11)];
            bin.0 += r * weight;
            bin.1 += g * weight;
            bin.2 += b * weight;
            bin.3 += weight;
        }
    }
    let best = bins.iter().copied().fold((0.0, 0.0, 0.0, 0.0), |a, b| if b.3 > a.3 { b } else { a });
    let color = (best.3 > seen * 0.04).then(|| Rgb::new((best.0 / best.3).min(1.0), (best.1 / best.3).min(1.0), (best.2 / best.3).min(1.0)));
    Look { shape, color, bbox }
}

fn squircle(x: f64, y: f64, size: f64) -> gsk::Path {
    let b = gsk::PathBuilder::new();
    let (cx, cy, r) = (x + size / 2.0, y + size / 2.0, size / 2.0);
    let n = 5.0;
    let steps = 160;
    for i in 0..=steps {
        let t = i as f64 / steps as f64 * std::f64::consts::TAU;
        let (c, s) = (t.cos(), t.sin());
        let px = cx + r * c.signum() * c.abs().powf(2.0 / n);
        let py = cy + r * s.signum() * s.abs().powf(2.0 / n);
        if i == 0 {
            b.move_to(px as f32, py as f32);
        } else {
            b.line_to(px as f32, py as f32);
        }
    }
    b.close();
    b.to_path()
}

fn matrix(weights: [[f32; 3]; 3]) -> graphene::Matrix {
    let mut m = [0f32; 16];
    for (j, row) in weights.iter().enumerate() {
        for (c, w) in row.iter().enumerate() {
            m[j * 4 + c] = *w;
        }
    }
    m[15] = 1.0;
    graphene::Matrix::from_float(m)
}

fn filter(style: Style, accent: Rgb) -> Option<graphene::Matrix> {
    let l = [0.2126f32, 0.7152, 0.0722];
    match style {
        Style::Mono => Some(matrix([[l[0]; 3], [l[1]; 3], [l[2]; 3]])),
        Style::Tinted => {
            let a = [accent.r as f32 * 1.3, accent.g as f32 * 1.3, accent.b as f32 * 1.3];
            Some(matrix([[l[0] * a[0], l[0] * a[1], l[0] * a[2]], [l[1] * a[0], l[1] * a[1], l[1] * a[2]], [l[2] * a[0], l[2] * a[1], l[2] * a[2]]]))
        }
        _ => None,
    }
}

fn stop(offset: f32, c: Rgb) -> gsk::ColorStop {
    gsk::ColorStop::new(offset, c.rgba())
}

fn compose(s: &gtk::Snapshot, icon: Option<&gdk::Paintable>, look: Look, size: f64, style: Style, accent: Rgb, layout: Option<gtk::pango::Layout>) {
    let tile = size * 0.84;
    let (x0, y0) = ((size - tile) / 2.0, size * 0.055);
    let white = Rgb::new(1.0, 1.0, 1.0);
    let rounded = gsk::RoundedRect::from_rect(graphene::Rect::new(x0 as f32, y0 as f32, tile as f32, tile as f32), (tile * 0.225) as f32);
    if !kit::effects::reduced() {
        s.append_outset_shadow(&rounded, &gdk::RGBA::new(0.0, 0.0, 0.0, 0.34), 0.0, (size * 0.02) as f32, 0.0, (size * 0.035) as f32);
    }
    let path = squircle(x0, y0, tile);
    let bounds = graphene::Rect::new(x0 as f32, y0 as f32, tile as f32, tile as f32);
    s.push_fill(&path, gsk::FillRule::Winding);
    let fx = filter(style, accent);
    let fit = |s: &gtk::Snapshot, p: &gdk::Paintable, target: f64| {
        let (bx, by, bw, bh) = look.bbox;
        let k = target / bw.max(bh).max(1e-3);
        let (cx, cy) = (x0 + tile / 2.0, y0 + tile / 2.0);
        s.save();
        s.translate(&graphene::Point::new((cx - (bx + bw / 2.0) * k) as f32, (cy - (by + bh / 2.0) * k) as f32));
        p.snapshot(s, k, k);
        s.restore();
    };
    if let (Shape::Plate | Shape::Disc(_), Some(p)) = (look.shape, icon) {
        if let Some(m) = &fx {
            s.push_color_matrix(m, &graphene::Vec4::zero());
        }
        if let Shape::Disc(c) = look.shape {
            s.append_linear_gradient(&bounds, &graphene::Point::new(0.0, y0 as f32), &graphene::Point::new(0.0, (y0 + tile) as f32), &[stop(0.0, c.mix(white, 0.94)), stop(1.0, c.mix(Rgb::new(0.0, 0.0, 0.0), 0.92))]);
        }
        fit(s, p, if look.shape == Shape::Plate { tile * 1.06 } else { tile * 0.97 });
        if fx.is_some() {
            s.pop();
        }
    } else {
        let neutral = look.color.is_none() || matches!(style, Style::Mono | Style::Tinted);
        let base = look.color.unwrap_or(Rgb::new(0.6, 0.6, 0.64));
        let (top, bottom) = match style {
            Style::Light if neutral => (Rgb::new(0.99, 0.99, 1.0), Rgb::new(0.86, 0.86, 0.89)),
            Style::Light => (base.mix(white, 0.12), base.mix(white, 0.30)),
            _ if neutral => (Rgb::new(0.25, 0.25, 0.27), Rgb::new(0.11, 0.11, 0.12)),
            _ => (base.mix(Rgb::new(0.25, 0.25, 0.27), 0.16), base.mix(Rgb::new(0.11, 0.11, 0.12), 0.10)),
        };
        s.append_linear_gradient(&bounds, &graphene::Point::new(0.0, y0 as f32), &graphene::Point::new(0.0, (y0 + tile) as f32), &[stop(0.0, top), stop(1.0, bottom)]);
        if let Some(m) = &fx {
            s.push_color_matrix(m, &graphene::Vec4::zero());
        }
        let inner = (tile * 0.66).round();
        let (ix, iy) = (x0 + (tile - inner) / 2.0, y0 + (tile - inner) / 2.0);
        match (icon, layout) {
            (Some(p), _) => {
                s.save();
                s.translate(&graphene::Point::new(ix as f32, iy as f32));
                p.snapshot(s, inner, inner);
                s.restore();
            }
            (None, Some(l)) => {
                let (_, logical) = l.pixel_extents();
                s.save();
                s.translate(&graphene::Point::new((x0 + (tile - logical.width() as f64) / 2.0) as f32, (y0 + (tile - logical.height() as f64) / 2.0) as f32));
                let ink = if style == Style::Light { Rgb::new(0.2, 0.2, 0.22) } else { white };
                s.append_layout(&l, &ink.rgba());
                s.restore();
            }
            _ => {}
        }
        if fx.is_some() {
            s.pop();
        }
    }
    s.append_linear_gradient(&bounds, &graphene::Point::new(0.0, y0 as f32), &graphene::Point::new(0.0, (y0 + tile * 0.5) as f32), &[stop(0.0, white.alpha(0.22)), stop(1.0, white.alpha(0.0))]);
    s.pop();
    let stroke = gsk::Stroke::new((size / 96.0).max(1.0) as f32);
    s.append_stroke(&path, &stroke, &white.alpha(if style == Style::Light { 0.25 } else { 0.10 }).rgba());
}

#[allow(clippy::too_many_arguments)]
pub fn texture(widget: &impl IsA<gtk::Widget>, key: &str, icon: Option<&gdk::Paintable>, name: &str, px: f64, style: Style, accent: Rgb, cfg: &crate::config::Config) -> Option<gdk::Texture> {
    if style == Style::Native {
        return None;
    }
    let renderer = cpu()?;
    let scale = widget.scale_factor().max(1);
    let size = (px * scale as f64).ceil();
    // File identity separates analysis and textures if the resolved artwork
    // changes. Non-file paintables use object identity instead of sharing a slot.
    let source = icon.map(|p| {
        p.downcast_ref::<gtk::IconPaintable>().and_then(|p| p.file()).map(|f| f.uri().to_string())
            .unwrap_or_else(|| format!("{:p}", p.as_ptr()))
    }).unwrap_or_default();
    let id = cache_key(key, &source, name, style, size as i32, accent);
    let look_id = (key.to_owned(), source);
    if let Some(t) = CACHE.with(|c| c.borrow().get(&id).cloned()) {
        return Some(t);
    }
    let look = match icon {
        Some(p) => LOOKS.with(|l| l.borrow().get(&look_id).copied()).unwrap_or_else(|| {
            let look = analyze(&renderer, p);
            LOOKS.with(|l| l.borrow_mut().insert(look_id, look));
            look
        }),
        None => Look { shape: Shape::Free, color: None, bbox: (0.0, 0.0, 1.0, 1.0) },
    };
    let letter: String = name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_default();
    let layout = icon.is_none().then(|| crate::draw::shaped(widget, cfg, &letter, (size * 0.38).round(), crate::draw::Face::Text(650)));
    let tex = render(&renderer, size, |s| compose(s, icon, look, size, style, accent, layout))?;
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 160 {
            c.clear();
        }
        c.insert(id, tex.clone());
    });
    Some(tex)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texture_cache_separates_artwork_style_size_and_monogram() {
        let accent = Rgb::new(0.2, 0.4, 0.6);
        let key = cache_key("app", "file:///hicolor/app.svg", "App", Style::Light, 64, accent);
        assert_ne!(key, cache_key("other", "file:///hicolor/app.svg", "App", Style::Light, 64, accent));
        assert_ne!(key, cache_key("app", "file:///Papirus/app.svg", "App", Style::Light, 64, accent));
        assert_ne!(key, cache_key("app", "file:///hicolor/app.svg", "App", Style::Graphite, 64, accent));
        assert_ne!(key, cache_key("app", "file:///hicolor/app.svg", "App", Style::Light, 128, accent));
        assert_ne!(cache_key("app", "", "App", Style::Light, 64, accent), cache_key("app", "", "Other", Style::Light, 64, accent));
    }

    #[test]
    fn only_tinted_cache_depends_on_accent() {
        let a = Rgb::new(0.2, 0.4, 0.6);
        let b = Rgb::new(0.8, 0.6, 0.4);
        for style in [Style::Light, Style::Graphite, Style::Mono, Style::Native] {
            assert_eq!(cache_key("app", "source", "App", style, 64, a), cache_key("app", "source", "App", style, 64, b));
        }
        assert_ne!(cache_key("app", "source", "App", Style::Tinted, 64, a), cache_key("app", "source", "App", Style::Tinted, 64, b));
    }
}
