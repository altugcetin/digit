use crate::app::App;
use crate::compositor;
use crate::draw::{self, Face};
use crate::wallpaper;
use gtk::subclass::prelude::*;
use gtk::{cairo, gdk, gio, glib, graphene, gsk, pango, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kit::motion::{self, Tween};
use kit::palette::Rgb;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::Duration;

const BIG_W: f64 = 600.0;
const BIG_H: f64 = 440.0;
const SMALL_W: f64 = 176.0;
const SMALL_H: f64 = 384.0;
const GAP: f64 = 14.0;
const SKEW: f32 = -21.0;
const RADIUS: f64 = 14.0;
const BAR_H: f64 = 46.0;
const CHIP: f64 = 30.0;
const GRID_W: f64 = 240.0;
const GRID_H: f64 = 136.0;
const GRID_GAP: f64 = 16.0;
const BAND: f64 = 640.0;
const KEEP: usize = 40;
const WORKERS: usize = 2;
const BORN: i64 = 240_000;

struct Job {
    path: String,
    kinds: Vec<&'static str>,
}

#[derive(Clone, Copy, PartialEq)]
enum Source {
    Yours,
    Pictures,
    Downloads,
}

const SOURCES: [(Source, &str); 3] = [(Source::Yours, "Yours"), (Source::Pictures, "Pictures"), (Source::Downloads, "Downloads")];

const HUES: [(&str, &str); 8] = [("Red", "#ff3b30"), ("Orange", "#ff9500"), ("Yellow", "#ffd60a"), ("Green", "#30d158"), ("Blue", "#0a84ff"), ("Purple", "#a23fe0"), ("Pink", "#ff5fa2"), ("Gray", "#a1a1a8")];

#[derive(Clone, Copy, PartialEq)]
enum Hit {
    Card(usize),
    Source,
    Grid,
    Timer,
    Shuffle,
    Hue(usize),
    Search,
    Bar,
}

struct State {
    app: Rc<App>,
    window: gtk::Window,
    canvas: Canvas,
    open: bool,
    scrim: Option<gtk::Window>,
    size: (f64, f64),
    source: Source,
    all: Vec<String>,
    items: Vec<String>,
    pointed: usize,
    original: String,
    thumbs: HashMap<String, gdk::Texture>,
    pending: Vec<String>,
    queue: Vec<Job>,
    busy: usize,
    born: HashMap<String, i64>,
    big: HashMap<String, gdk::Texture>,
    side: HashMap<String, gdk::Texture>,
    ring: Option<gdk::Texture>,
    ghosts: Option<(gdk::Texture, gdk::Texture)>,
    order: Vec<String>,
    output: String,
    timer: bool,
    colors: HashMap<String, u8>,
    hue: Option<usize>,
    query: String,
    grid: bool,
    fade: Tween,
    focus: Tween,
    rise: Tween,
    scroll: Tween,
    preview_wait: u64,
    hits: Vec<(Hit, graphene::Rect)>,
    region: bool,
    ticking: bool,
}

#[derive(Clone)]
pub struct Picker(Rc<RefCell<State>>);

pub fn toggle(app: &Rc<App>) {
    let existing = app.picker.borrow().clone();
    let picker = existing.unwrap_or_else(|| {
        let p = Picker::new(app);
        *app.picker.borrow_mut() = Some(p.clone());
        p
    });
    picker.toggle();
}

thread_local! {
    static SLIDES: RefCell<Option<glib::SourceId>> = const { RefCell::new(None) };
}

pub fn slideshow(app: &Rc<App>) {
    if let Some(id) = SLIDES.with(|s| s.borrow_mut().take()) {
        id.remove();
    }
    let every = kit::prefs::number(&kit::prefs::read(), "wallpaper_every", 0.0);
    if every < 1.0 {
        return;
    }
    let weak = Rc::downgrade(app);
    let id = glib::timeout_add_local(Duration::from_secs((every * 60.0) as u64), move || {
        let Some(app) = weak.upgrade() else { return glib::ControlFlow::Break };
        let list = list(Source::Yours);
        if list.len() > 1 {
            let now = app.wallpapers.shown();
            let at = list.iter().position(|p| *p == now).map(|i| i + 1).unwrap_or(0) % list.len();
            if wallpaper::remember(&list[at]) {
                app.wallpapers.set(list[at].clone());
                crate::retheme(&app);
            }
        }
        glib::ControlFlow::Continue
    });
    SLIDES.with(|s| *s.borrow_mut() = Some(id));
}

fn source_dir(source: Source) -> Option<std::path::PathBuf> {
    match source {
        Source::Yours => None,
        Source::Pictures => glib::user_special_dir(glib::UserDirectory::Pictures),
        Source::Downloads => glib::user_special_dir(glib::UserDirectory::Downloads),
    }
}

fn list(source: Source) -> Vec<String> {
    match source {
        Source::Yours => {
            let mut out = wallpaper::used();
            if let Some(pictures) = source_dir(Source::Pictures) {
                for dir in [pictures.join("Wallpapers"), pictures.join("wallpapers")] {
                    for p in wallpaper::listed(&dir) {
                        if !out.contains(&p) {
                            out.push(p);
                        }
                    }
                }
            }
            out
        }
        other => source_dir(other).map(|d| wallpaper::listed(&d)).unwrap_or_default(),
    }
}

fn colors_file() -> std::path::PathBuf {
    kit::theme::state_dir().join("wallpaper-colors")
}

fn load_colors() -> HashMap<String, u8> {
    std::fs::read_to_string(colors_file())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let mut p = l.splitn(3, '\t');
            let stamp = p.next()?;
            let bucket: u8 = p.next()?.parse().ok()?;
            let path = p.next()?;
            let now = std::fs::metadata(path).ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs().to_string();
            (now == stamp).then(|| (path.to_string(), bucket))
        })
        .collect()
}

fn save_colors(colors: &HashMap<String, u8>) {
    let body: String = colors
        .iter()
        .filter_map(|(p, b)| {
            let stamp = std::fs::metadata(p).ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
            Some(format!("{stamp}\t{b}\t{p}\n"))
        })
        .collect();
    let path = colors_file();
    let _ = std::fs::create_dir_all(kit::theme::state_dir());
    let tmp = path.with_extension("new");
    if std::fs::write(&tmp, body).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

fn classify(path: &str) -> Option<u8> {
    let pb = gdk::gdk_pixbuf::Pixbuf::from_file_at_scale(path, 64, 64, true).ok()?;
    let (w, h, n, rs) = (pb.width() as usize, pb.height() as usize, pb.n_channels() as usize, pb.rowstride() as usize);
    let bytes = pb.read_pixel_bytes();
    let (mut x, mut y, mut weight, mut sats, mut count) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for py in 0..h {
        for px in 0..w {
            let i = py * rs + px * n;
            let (r, g, b) = (bytes[i] as f64 / 255.0, bytes[i + 1] as f64 / 255.0, bytes[i + 2] as f64 / 255.0);
            let (max, min) = (r.max(g).max(b), r.min(g).min(b));
            let d = max - min;
            let s = if max > 0.0 { d / max } else { 0.0 };
            sats += s;
            count += 1.0;
            if s < 0.22 || max < 0.12 || d < 1e-6 {
                continue;
            }
            let hue = if max == r { ((g - b) / d).rem_euclid(6.0) } else if max == g { (b - r) / d + 2.0 } else { (r - g) / d + 4.0 } * 60.0;
            let wgt = s * max;
            x += hue.to_radians().cos() * wgt;
            y += hue.to_radians().sin() * wgt;
            weight += wgt;
        }
    }
    if count == 0.0 || sats / count < 0.16 || weight / count < 0.06 {
        return Some(7);
    }
    let hue = y.atan2(x).to_degrees().rem_euclid(360.0);
    Some(match hue {
        h if !(15.0..345.0).contains(&h) => 0,
        h if h < 40.0 => 1,
        h if h < 70.0 => 2,
        h if h < 170.0 => 3,
        h if h < 255.0 => 4,
        h if h < 290.0 => 5,
        _ => 6,
    })
}

fn width_at(t: f64) -> f64 {
    if t < 1.0 {
        let k = (1.0 - t) * (1.0 - t) * (3.0 - 2.0 * (1.0 - t));
        SMALL_W + (BIG_W - SMALL_W) * k
    } else {
        SMALL_W * (1.0 - 0.06 * (t - 1.0)).max(0.62)
    }
}

fn height_at(t: f64) -> f64 {
    if t < 1.0 { BIG_H + (SMALL_H - BIG_H) * t * t * (3.0 - 2.0 * t) } else { (SMALL_H - 26.0 * (t - 1.0)).max(250.0) }
}

fn offset(d: f64) -> f64 {
    let a = d.abs();
    let steps = (a * 24.0).ceil().max(1.0) as usize;
    let dt = a / steps as f64;
    let mut sum = 0.0;
    for k in 0..steps {
        let t = (k as f64 + 0.5) * dt;
        sum += (width_at(t) + width_at(t + 1.0)) / 2.0 * dt;
    }
    let base = sum + GAP * a;
    if d < 0.0 { -base } else { base }
}

fn shape(cr: &cairo::Context, bw: f64, bh: f64, slant: f64, r: f64) {
    let pts = [(slant, 0.0), (bw, 0.0), (bw - slant, bh), (0.0, bh)];
    let n = pts.len();
    for i in 0..n {
        let p = pts[i];
        let a = pts[(i + n - 1) % n];
        let b = pts[(i + 1) % n];
        let towards = |q: (f64, f64)| {
            let (dx, dy) = (q.0 - p.0, q.1 - p.1);
            let len = (dx * dx + dy * dy).sqrt().max(1e-6);
            let rr = r.min(len / 2.0);
            (p.0 + dx / len * rr, p.1 + dy / len * rr)
        };
        let (s0, s1) = (towards(a), towards(b));
        if i == 0 {
            cr.move_to(s0.0, s0.1);
        } else {
            cr.line_to(s0.0, s0.1);
        }
        cr.curve_to(p.0, p.1, p.0, p.1, s1.0, s1.1);
    }
    cr.close_path();
}

fn card_box(kind: &str, sf: f64) -> (i32, i32, f64) {
    let k = (SKEW as f64).to_radians().tan().abs();
    let (w, h) = match kind {
        "big" => (BIG_W, BIG_H),
        "side" => (SMALL_W, SMALL_H),
        _ => (GRID_W, GRID_H),
    };
    let reach = if kind == "grid" { w } else { w + h * k };
    ((reach * sf).round() as i32, (h * sf).round() as i32, h * k * sf)
}

fn card_tag(kind: &str, sf: f64) -> String {
    format!("{kind}@{sf}")
}

fn build(master: &gdk::gdk_pixbuf::Pixbuf, kind: &str, sf: f64) -> Option<gdk::Texture> {
    let (bw, bh, slant) = card_box(kind, sf);
    let pb = crate::stash::cover(master, bw, bh)?;
    if kind == "grid" {
        return Some(crate::stash::texture(&pb));
    }
    let img = surface_of(&pb)?;
    let out = cairo::ImageSurface::create(cairo::Format::ARgb32, bw, bh).ok()?;
    {
        let cr = cairo::Context::new(&out).ok()?;
        shape(&cr, bw as f64, bh as f64, slant, RADIUS * sf);
        cr.clip();
        cr.set_source_surface(&img, 0.0, 0.0).ok()?;
        cr.paint().ok()?;
    }
    let (bytes, pw, ph, stride) = finish(out)?;
    Some(gdk::MemoryTexture::new(pw, ph, gdk::MemoryFormat::B8g8r8a8Premultiplied, &glib::Bytes::from_owned(bytes), stride).upcast())
}

fn cards(path: &str, want: &[&'static str], sf: f64, screen: Option<(i32, i32)>) -> Vec<(&'static str, gdk::Texture)> {
    let mut out = Vec::new();
    let mut make = Vec::new();
    for kind in ["big", "side", "grid"] {
        let tag = card_tag(kind, sf);
        if want.contains(&kind) {
            if let Some(t) = crate::stash::get(path, &tag) {
                out.push((kind, t));
                continue;
            }
        } else if crate::stash::has(path, &tag) {
            continue;
        }
        make.push(kind);
    }
    if !make.iter().any(|k| want.contains(k)) {
        return out;
    }
    let screen = screen.map(|s| (s, format!("screen-{}x{}", s.0, s.1))).filter(|(_, t)| !crate::stash::has(path, t));
    let mut need: Vec<(i32, i32)> = make.iter().map(|k| card_box(k, sf)).map(|(w, h, _)| (w, h)).collect();
    if let Some((size, _)) = &screen {
        need.push(*size);
    }
    let Some(master) = crate::stash::master(path, &need) else { return out };
    for kind in make {
        if let Some(t) = build(&master, kind, sf) {
            crate::stash::put(path, &card_tag(kind, sf), t.clone());
            if want.contains(&kind) {
                out.push((kind, t));
            }
        }
    }
    if let Some(((w, h), tag)) = screen {
        if let Some(pb) = crate::stash::cover(&master, w, h) {
            crate::stash::put(path, &tag, crate::stash::texture(&pb));
        }
    }
    drop(master);
    unsafe {
        libc::malloc_trim(0);
    }
    out
}

fn surface_of(pb: &gdk::gdk_pixbuf::Pixbuf) -> Option<cairo::ImageSurface> {
    let (w, h, n, rs) = (pb.width(), pb.height(), pb.n_channels() as usize, pb.rowstride() as usize);
    let bytes = pb.read_pixel_bytes();
    let mut img = cairo::ImageSurface::create(cairo::Format::Rgb24, w, h).ok()?;
    let stride = img.stride() as usize;
    {
        let mut data = img.data().ok()?;
        for y in 0..h as usize {
            for x in 0..w as usize {
                let i = y * rs + x * n;
                let o = y * stride + x * 4;
                data[o] = bytes[i + 2];
                data[o + 1] = bytes[i + 1];
                data[o + 2] = bytes[i];
                data[o + 3] = 255;
            }
        }
    }
    Some(img)
}

fn finish(mut out: cairo::ImageSurface) -> Option<(Vec<u8>, i32, i32, usize)> {
    out.flush();
    let (w, h, stride) = (out.width(), out.height(), out.stride() as usize);
    let data = out.data().ok()?.to_vec();
    Some((data, w, h, stride))
}

fn outline(w: f64, h: f64, sf: f64, solid: bool) -> Option<gdk::Texture> {
    let k = (SKEW as f64).to_radians().tan().abs();
    let (bw, bh) = (((w + h * k) * sf).round() as i32, (h * sf).round() as i32);
    let out = cairo::ImageSurface::create(cairo::Format::ARgb32, bw, bh).ok()?;
    {
        let cr = cairo::Context::new(&out).ok()?;
        if solid {
            shape(&cr, bw as f64, bh as f64, h * k * sf, RADIUS * sf);
            cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
            cr.fill().ok()?;
        } else {
            let lw = 2.0 * sf;
            cr.translate(lw / 2.0, lw / 2.0);
            shape(&cr, bw as f64 - lw, bh as f64 - lw, (h * k * sf) - lw / 2.0, RADIUS * sf);
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.9);
            cr.set_line_width(lw);
            cr.stroke().ok()?;
        }
    }
    let (bytes, pw, ph, stride) = finish(out)?;
    Some(gdk::MemoryTexture::new(pw, ph, gdk::MemoryFormat::B8g8r8a8Premultiplied, &glib::Bytes::from_owned(bytes), stride).upcast())
}

fn rounded(x: f64, y: f64, w: f64, h: f64, r: f64) -> gsk::RoundedRect {
    gsk::RoundedRect::from_rect(graphene::Rect::new(x as f32, y as f32, w as f32, h as f32), r as f32)
}

impl Picker {
    fn new(app: &Rc<App>) -> Self {
        let window = gtk::Window::new();
        window.init_layer_shell();
        window.set_layer(Layer::Overlay);
        window.set_namespace(Some("digit-wallpapers"));
        for e in [Edge::Left, Edge::Right] {
            window.set_anchor(e, true);
        }
        window.set_exclusive_zone(-1);
        window.set_keyboard_mode(KeyboardMode::None);
        window.add_css_class("digit");
        let canvas: Canvas = glib::Object::new();
        window.set_child(Some(&canvas));
        let state = Rc::new(RefCell::new(State {
            app: app.clone(),
            window,
            canvas: canvas.clone(),
            open: false,
            scrim: None,
            size: (1920.0, 1080.0),
            source: Source::Yours,
            all: Vec::new(),
            items: Vec::new(),
            pointed: 0,
            original: String::new(),
            thumbs: HashMap::new(),
            pending: Vec::new(),
            queue: Vec::new(),
            busy: 0,
            born: HashMap::new(),
            big: HashMap::new(),
            side: HashMap::new(),
            ring: None,
            ghosts: None,
            order: Vec::new(),
            output: String::new(),
            timer: false,
            colors: HashMap::new(),
            hue: None,
            query: String::new(),
            grid: false,
            fade: Tween::still(0.0),
            focus: Tween::still(0.0),
            rise: Tween::still(0.0),
            scroll: Tween::still(0.0),
            preview_wait: 0,
            hits: Vec::new(),
            region: false,
            ticking: false,
        }));
        *canvas.imp().state.borrow_mut() = Rc::downgrade(&state);
        let picker = Picker(state);
        picker.wire();
        picker
    }

    fn wire(&self) {
        let (window, canvas) = {
            let s = self.0.borrow();
            (s.window.clone(), s.canvas.clone())
        };
        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(&self.0);
        keys.connect_key_pressed(move |_, key, _, mods| {
            let Some(s) = weak.upgrade() else { return glib::Propagation::Proceed };
            let me = Picker(s);
            let shift = mods.contains(gdk::ModifierType::SHIFT_MASK);
            let grid = me.0.borrow().grid;
            let cols = me.columns() as i32;
            match key {
                gdk::Key::Escape => {
                    let searching = !me.0.borrow().query.is_empty();
                    if searching {
                        me.0.borrow_mut().query.clear();
                        me.refilter();
                    } else {
                        me.close(false);
                    }
                }
                gdk::Key::Return | gdk::Key::KP_Enter => me.close(true),
                gdk::Key::Right => me.point_by(1),
                gdk::Key::Left => me.point_by(-1),
                gdk::Key::Down if grid => me.point_by(cols),
                gdk::Key::Up if grid => me.point_by(-cols),
                gdk::Key::Home => me.point_by(-100_000),
                gdk::Key::End => me.point_by(100_000),
                gdk::Key::Tab if !shift => me.switch_by(1),
                gdk::Key::ISO_Left_Tab | gdk::Key::Tab => me.switch_by(-1),
                gdk::Key::BackSpace => {
                    let changed = me.0.borrow_mut().query.pop().is_some();
                    if changed {
                        me.refilter();
                    }
                }
                _ => {
                    let Some(c) = key.to_unicode().filter(|c| !c.is_control() && (c.is_alphanumeric() || " -_.".contains(*c))) else { return glib::Propagation::Proceed };
                    if mods.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK) {
                        return glib::Propagation::Proceed;
                    }
                    if c == ' ' && me.0.borrow().query.is_empty() {
                        me.close(true);
                        return glib::Propagation::Stop;
                    }
                    me.0.borrow_mut().query.extend(c.to_lowercase());
                    me.refilter();
                }
            }
            glib::Propagation::Stop
        });
        window.add_controller(keys);
        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(&self.0);
        click.connect_released(move |_, n, x, y| {
            let Some(s) = weak.upgrade() else { return };
            let me = Picker(s);
            match me.hit(x, y) {
                Some(Hit::Card(i)) => {
                    let pointed = me.0.borrow().pointed;
                    if n >= 2 || pointed == i {
                        me.point(i, true);
                        me.close(true);
                    } else {
                        me.point(i, true);
                    }
                }
                Some(Hit::Source) => me.switch_by(1),
                Some(Hit::Grid) => {
                    {
                        let mut s = me.0.borrow_mut();
                        s.grid = !s.grid;
                        let now = glib::monotonic_time();
                        s.rise.set(0.0);
                        s.rise.go(1.0, now, 380, motion::EMPHASIZED_DECEL);
                    }
                    me.refocus();
                }
                Some(Hit::Timer) => {
                    let on = me.0.borrow().timer;
                    me.0.borrow_mut().timer = !on;
                    let _ = kit::prefs::set("wallpaper_every", toml::Value::Integer(if on { 0 } else { 30 }));
                    let app = me.0.borrow().app.clone();
                    slideshow(&app);
                    me.kick();
                }
                Some(Hit::Shuffle) => {
                    let n = me.0.borrow().items.len();
                    if n > 1 {
                        let at = me.0.borrow().pointed;
                        let mut pick = (glib::random_int_range(0, n as i32)) as usize;
                        if pick == at {
                            pick = (pick + 1) % n;
                        }
                        me.point(pick, true);
                    }
                }
                Some(Hit::Hue(h)) => {
                    {
                        let mut s = me.0.borrow_mut();
                        s.hue = if s.hue == Some(h) { None } else { Some(h) };
                    }
                    me.refilter();
                }
                Some(Hit::Search) | Some(Hit::Bar) => {}
                None => me.close(false),
            }
        });
        canvas.add_controller(click);
        let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES | gtk::EventControllerScrollFlags::DISCRETE);
        let weak = Rc::downgrade(&self.0);
        wheel.connect_scroll(move |_, dx, dy| {
            if let Some(s) = weak.upgrade() {
                let me = Picker(s);
                let grid = me.0.borrow().grid;
                let d = if dx.abs() > dy.abs() { dx } else { dy };
                let step = if grid && dy.abs() >= dx.abs() { me.columns() as i32 } else { 1 };
                me.point_by(if d > 0.0 { step } else { -step });
            }
            glib::Propagation::Stop
        });
        canvas.add_controller(wheel);
    }

    fn columns(&self) -> usize {
        let s = self.0.borrow();
        (((s.size.0 - 160.0) + GRID_GAP) / (GRID_W + GRID_GAP)).floor().clamp(2.0, 7.0) as usize
    }

    fn toggle(&self) {
        let open = self.0.borrow().open;
        if open { self.close(false) } else { self.open() }
    }

    fn open(&self) {
        let output = compositor::focused_output();
        {
            let mut s = self.0.borrow_mut();
            let display = gdk::Display::default();
            let monitors: Vec<gdk::Monitor> = display.map(|d| d.monitors().iter::<gdk::Monitor>().filter_map(Result::ok).collect()).unwrap_or_default();
            let monitor = monitors.iter().find(|m| output.as_deref().is_some_and(|o| m.connector().as_deref() == Some(o))).or(monitors.first()).cloned();
            if let Some(m) = &monitor {
                s.size = (m.geometry().width() as f64, BAND);
                s.output = m.connector().map(|c| c.to_string()).unwrap_or_default();
            }
            s.canvas.set_size_request(s.size.0 as i32, BAND as i32);
            s.timer = kit::prefs::number(&kit::prefs::read(), "wallpaper_every", 0.0) >= 1.0;
            if !s.window.is_visible() {
                s.window.set_monitor(monitor.as_ref());
            }
            s.original = s.app.wallpapers.shown();
            s.open = true;
            s.query.clear();
            s.hue = None;
            if s.colors.is_empty() {
                s.colors = load_colors();
            }
            let now = glib::monotonic_time();
            s.fade.go(1.0, now, 220, motion::STANDARD);
            s.rise.set(0.0);
            s.rise.go(1.0, now, 520, motion::EMPHASIZED_DECEL);
            s.window.set_keyboard_mode(KeyboardMode::Exclusive);
            if s.scrim.is_none() {
                let weak = Rc::downgrade(&self.0);
                s.scrim = Some(crate::paint::scrim(monitor.as_ref(), move || {
                    if let Some(s) = weak.upgrade() {
                        Picker(s).close(false);
                    }
                }));
            }
        }
        let window = self.0.borrow().window.clone();
        window.present();
        self.load(Source::Yours);
        self.kick();
        glib::spawn_future_local(async {
            let _ = gio::spawn_blocking(crate::stash::prune).await;
        });
    }

    fn close(&self, keep: bool) {
        let (app, original, chosen) = {
            let mut s = self.0.borrow_mut();
            if !s.open {
                return;
            }
            s.open = false;
            s.preview_wait += 1;
            let now = glib::monotonic_time();
            s.fade.go(0.0, now, 180, motion::FAST_EFFECTS);
            s.rise.go(0.0, now, 220, motion::Curve::new(0.3, 0.0, 0.8, 0.15));
            s.window.set_keyboard_mode(KeyboardMode::None);
            if let Some(scrim) = s.scrim.take() {
                scrim.destroy();
            }
            s.queue.clear();
            (s.app.clone(), s.original.clone(), s.items.get(s.pointed).cloned())
        };
        let output = self.0.borrow().output.clone();
        match (keep, chosen) {
            (true, Some(path)) if path != original => {
                if wallpaper::remember(&path) {
                    app.wallpapers.commit(path, &output);
                    crate::retheme(&app);
                } else {
                    app.wallpapers.unpreview(&output);
                }
            }
            _ => app.wallpapers.unpreview(&output),
        }
        self.kick();
    }

    fn load(&self, source: Source) {
        {
            let mut s = self.0.borrow_mut();
            s.source = source;
            s.all = list(source);
        }
        self.refilter();
        let weak = Rc::downgrade(&self.0);
        glib::timeout_add_local_once(Duration::from_millis(1600), move || {
            if let Some(state) = weak.upgrade() {
                let me = Picker(state);
                if me.0.borrow().open {
                    me.classify_missing();
                }
            }
        });
    }

    fn refilter(&self) {
        {
            let mut s = self.0.borrow_mut();
            let current = s.items.get(s.pointed).cloned().unwrap_or_else(|| s.app.wallpapers.shown());
            let q = s.query.clone();
            let hue = s.hue;
            let colors = s.colors.clone();
            s.items = s
                .all
                .iter()
                .filter(|p| q.is_empty() || std::path::Path::new(p).file_stem().is_some_and(|n| n.to_string_lossy().to_lowercase().contains(&q)))
                .filter(|p| hue.is_none_or(|h| colors.get(*p).copied() == Some(h as u8)))
                .cloned()
                .collect();
            s.pointed = s.items.iter().position(|p| *p == current).unwrap_or(0);
            let p = s.pointed as f64;
            s.focus.set(p);
            let row = s.row_of(s.pointed);
            s.scroll.set(row);
        }
        self.request_thumbs();
        self.kick();
    }

    fn refocus(&self) {
        {
            let mut s = self.0.borrow_mut();
            let p = s.pointed as f64;
            s.focus.set(p);
            let row = s.row_of(s.pointed);
            s.scroll.set(row);
        }
        self.request_thumbs();
        self.kick();
    }

    fn classify_missing(&self) {
        let todo: Vec<String> = {
            let s = self.0.borrow();
            s.all.iter().filter(|p| !s.colors.contains_key(*p)).take(400).cloned().collect()
        };
        if todo.is_empty() {
            return;
        }
        let weak = Rc::downgrade(&self.0);
        glib::spawn_future_local(async move {
            let found = gio::spawn_blocking(move || todo.into_iter().filter_map(|p| classify(&p).map(|b| (p, b))).collect::<Vec<_>>()).await.unwrap_or_default();
            let Some(state) = weak.upgrade() else { return };
            let colors = {
                let mut s = state.borrow_mut();
                s.colors.extend(found);
                s.colors.clone()
            };
            save_colors(&colors);
            let me = Picker(state);
            if me.0.borrow().hue.is_some() {
                me.refilter();
            } else {
                me.kick();
            }
        });
    }

    fn switch_by(&self, by: i32) {
        let at = SOURCES.iter().position(|t| t.0 == self.0.borrow().source).unwrap_or(0) as i32;
        let next = (at + by).rem_euclid(SOURCES.len() as i32) as usize;
        self.load(SOURCES[next].0);
        self.kick();
    }

    fn point_by(&self, by: i32) {
        let (n, at) = {
            let s = self.0.borrow();
            (s.items.len() as i64, s.pointed as i64)
        };
        if n == 0 {
            return;
        }
        self.point((at + by as i64).clamp(0, n - 1) as usize, false);
    }

    fn point(&self, i: usize, now_preview: bool) {
        let generation = {
            let mut s = self.0.borrow_mut();
            if i >= s.items.len() {
                return;
            }
            let now = glib::monotonic_time();
            if s.pointed != i {
                s.pointed = i;
                s.focus.go(i as f64, now, 380, motion::EMPHASIZED_DECEL);
                let row = s.row_of(i);
                s.scroll.go(row, now, 300, motion::STANDARD);
            }
            s.preview_wait += 1;
            s.preview_wait
        };
        self.request_thumbs();
        self.kick();
        let weak = Rc::downgrade(&self.0);
        let delay = if now_preview { 0 } else { 150 };
        glib::timeout_add_local_once(Duration::from_millis(delay), move || {
            let Some(state) = weak.upgrade() else { return };
            let (app, path, output) = {
                let s = state.borrow();
                if s.preview_wait != generation || !s.open {
                    return;
                }
                (s.app.clone(), s.items.get(s.pointed).cloned(), s.output.clone())
            };
            if let Some(path) = path {
                app.wallpapers.preview(path, &output);
            }
        });
    }

    fn request_thumbs(&self) {
        {
            let mut s = self.0.borrow_mut();
            let sf = s.canvas.scale_factor().max(1) as f64;
            let n = s.items.len();
            let p = s.pointed;
            let mut wanted: Vec<(usize, &'static str)> = Vec::new();
            if s.grid {
                let cols = (((s.size.0 - 160.0) + GRID_GAP) / (GRID_W + GRID_GAP)).floor().clamp(2.0, 7.0) as usize;
                let first = (p / cols).saturating_sub(2) * cols;
                let mut idx: Vec<usize> = (first..(first + cols * 5).min(n)).collect();
                idx.sort_by_key(|i| i.abs_diff(p));
                wanted.extend(idx.into_iter().map(|i| (i, "grid")));
            } else {
                for d in 0..=7usize {
                    let mut around = vec![p + d];
                    if d > 0 && p >= d {
                        around.push(p - d);
                    }
                    for i in around.into_iter().filter(|i| *i < n) {
                        if d == 0 {
                            wanted.push((i, "big"));
                        }
                        wanted.push((i, "side"));
                        if d == 1 {
                            wanted.push((i, "big"));
                        }
                    }
                }
            }
            let paths: Vec<String> = wanted.iter().map(|(i, _)| s.items[*i].clone()).collect();
            for path in paths.iter().rev() {
                s.order.retain(|o| o != path);
                s.order.push(path.clone());
            }
            while s.order.len() > KEEP {
                let old = s.order.remove(0);
                s.thumbs.remove(&old);
                s.big.remove(&old);
                s.side.remove(&old);
            }
            let mut jobs: Vec<Job> = Vec::new();
            for ((_, kind), path) in wanted.into_iter().zip(paths) {
                let have = match kind {
                    "big" => s.big.contains_key(&path),
                    "side" => s.side.contains_key(&path),
                    _ => s.thumbs.contains_key(&path),
                };
                if have || s.pending.contains(&format!("{kind}:{path}")) {
                    continue;
                }
                match jobs.iter_mut().find(|j| j.path == path) {
                    Some(j) => j.kinds.push(kind),
                    None => jobs.push(Job { path, kinds: vec![kind] }),
                }
            }
            s.queue = jobs;
            if s.ring.is_none() {
                s.ring = outline(BIG_W, BIG_H, sf, false);
            }
            if s.ghosts.is_none() {
                s.ghosts = outline(SMALL_W, SMALL_H, sf, true).zip(outline(BIG_W, BIG_H, sf, true));
            }
        }
        self.pump();
    }

    fn pump(&self) {
        loop {
            let (job, sf) = {
                let mut s = self.0.borrow_mut();
                if !s.open || s.busy >= WORKERS || s.queue.is_empty() {
                    return;
                }
                let job = s.queue.remove(0);
                s.busy += 1;
                for kind in &job.kinds {
                    s.pending.push(format!("{kind}:{}", job.path));
                }
                (job, s.canvas.scale_factor().max(1) as f64)
            };
            self.fetch(job, sf);
        }
    }

    fn fetch(&self, job: Job, sf: f64) {
        let weak = Rc::downgrade(&self.0);
        let screen = {
            let st = self.0.borrow();
            st.app.wallpapers.size_of(&st.output)
        };
        glib::spawn_future_local(async move {
            let Job { path, kinds } = job;
            let file = path.clone();
            let want = kinds.clone();
            let made = gio::spawn_blocking(move || cards(&file, &want, sf, screen)).await.unwrap_or_default();
            let Some(state) = weak.upgrade() else { return };
            {
                let mut s = state.borrow_mut();
                for kind in &kinds {
                    let tag = format!("{kind}:{path}");
                    s.pending.retain(|p| *p != tag);
                }
                s.busy = s.busy.saturating_sub(1);
                if s.open {
                    let now = glib::monotonic_time();
                    let warm = s.fade.value(now) < 0.05;
                    for (kind, t) in made {
                        match kind {
                            "big" => s.big.insert(path.clone(), t),
                            "side" => s.side.insert(path.clone(), t),
                            _ => s.thumbs.insert(path.clone(), t),
                        };
                        if !warm {
                            s.born.insert(format!("{kind}:{path}"), now);
                        }
                    }
                }
            }
            let me = Picker(state);
            me.kick();
            me.pump();
        });
    }

    fn hit(&self, x: f64, y: f64) -> Option<Hit> {
        let s = self.0.borrow();
        let p = graphene::Point::new(x as f32, y as f32);
        s.hits.iter().rev().find(|(_, r)| r.contains_point(&p)).map(|(h, _)| *h)
    }

    fn kick(&self) {
        let (canvas, already) = {
            let mut s = self.0.borrow_mut();
            let already = s.ticking;
            s.ticking = true;
            (s.canvas.clone(), already)
        };
        if already {
            return;
        }
        let weak: Weak<RefCell<State>> = Rc::downgrade(&self.0);
        canvas.add_tick_callback(move |w, clock| {
            let Some(state) = weak.upgrade() else { return glib::ControlFlow::Break };
            let Ok(mut s) = state.try_borrow_mut() else { return glib::ControlFlow::Continue };
            let busy = s.step(clock.frame_time());
            w.queue_draw();
            if busy {
                glib::ControlFlow::Continue
            } else {
                s.ticking = false;
                glib::ControlFlow::Break
            }
        });
    }
}

impl State {
    fn row_of(&self, i: usize) -> f64 {
        let cols = (((self.size.0 - 160.0) + GRID_GAP) / (GRID_W + GRID_GAP)).floor().clamp(2.0, 7.0) as usize;
        (i / cols) as f64
    }

    fn step(&mut self, now: i64) -> bool {
        if self.open != self.region {
            if let Some(surface) = self.window.surface() {
                let r = if self.open { cairo::RectangleInt::new(0, 0, self.size.0 as i32, self.size.1 as i32) } else { cairo::RectangleInt::new(0, 0, 1, 1) };
                surface.set_input_region(Some(&cairo::Region::create_rectangle(&r)));
                self.region = self.open;
            }
        }
        self.born.retain(|_, t| !motion::reduced() && now - *t < BORN);
        let moving = [&self.fade, &self.focus, &self.rise, &self.scroll].iter().any(|t| !t.settled(now)) || self.app.palette_moving(now) || !self.born.is_empty();
        if !moving && !self.open && self.window.is_visible() {
            let window = self.window.clone();
            glib::idle_add_local_once(move || window.set_visible(false));
            self.thumbs.clear();
            self.thumbs.shrink_to_fit();
            self.big.clear();
            self.side.clear();
            self.born.clear();
            self.queue.clear();
            self.order.clear();
            self.items.clear();
            self.all.clear();
            draw::forget(&self.canvas);
            App::trim();
        }
        moving
    }

    fn arrived(&self, kind: &str, path: &str, now: i64) -> f64 {
        if motion::reduced() {
            return 1.0;
        }
        self.born.get(&format!("{kind}:{path}")).map_or(1.0, |t| motion::STANDARD.at(((now - t) as f64 / BORN as f64).clamp(0.0, 1.0)))
    }

    fn texture(&self, s: &gtk::Snapshot, path: &str, x: f64, y: f64, w: f64, h: f64, now: i64) {
        if let Some(t) = self.thumbs.get(path) {
            let (tw, th) = (t.width() as f64, t.height() as f64);
            let c = (w / tw).max(h / th);
            let bounds = graphene::Rect::new((x + (w - tw * c) / 2.0) as f32, (y + (h - th * c) / 2.0) as f32, (tw * c) as f32, (th * c) as f32);
            let a = self.arrived("grid", path, now);
            if a < 1.0 {
                s.push_opacity(a);
            }
            s.append_scaled_texture(t, gsk::ScalingFilter::Linear, &bounds);
            if a < 1.0 {
                s.pop();
            }
        }
    }

    fn draw(&mut self, s: &gtk::Snapshot, now: i64) {
        let fade = self.fade.value(now).clamp(0.0, 1.0);
        if fade < 0.003 {
            return;
        }
        self.hits.clear();
        let pal = self.app.palette_at(now);
        let cfg = self.app.cfg.clone();
        let canvas = self.canvas.clone();
        let (sw, sh) = self.size;
        let rise = self.rise.value(now).clamp(0.0, 1.2);
        s.push_opacity(fade);
        let bar_y = (10.0 + (1.0 - rise) * -14.0).round();
        let cy = (10.0 + BAR_H + 28.0 + BIG_H / 2.0).round();
        let ink = pal.ink_main;
        let label = match (&self.hue, self.query.is_empty()) {
            (_, false) => format!("“{}”", self.query),
            (Some(h), true) => HUES[*h].0.to_string(),
            (None, true) => SOURCES.iter().find(|t| t.0 == self.source).map(|t| t.1).unwrap_or("Yours").to_string(),
        };
        let label_w = draw::width(&canvas, &cfg, &label, 13.0, Face::Text(560)) + 28.0;
        let buttons = 3.0 * (CHIP + 6.0);
        let hues_w = HUES.len() as f64 * (CHIP + 6.0);
        let bar_w = 10.0 + label_w + 8.0 + buttons + 8.0 + hues_w + 6.0 + CHIP + 10.0;
        let bx = ((sw - bar_w) / 2.0).round();
        let bar = rounded(bx, bar_y, bar_w, BAR_H, 16.0);
        s.push_rounded_clip(&bar);
        s.append_color(&pal.body.alpha(0.92).rgba(), bar.bounds());
        s.pop();
        self.hits.push((Hit::Bar, *bar.bounds()));
        let mid = bar_y + BAR_H / 2.0;
        let mut x = bx + 8.0;
        let chip = |s: &gtk::Snapshot, x: f64, w: f64, on: bool| {
            let r = rounded(x, mid - CHIP / 2.0 - 1.0, w, CHIP + 2.0, 9.0);
            s.push_rounded_clip(&r);
            s.append_color(&(if on { pal.accent } else { pal.raised }).rgba(), r.bounds());
            s.pop();
            *r.bounds()
        };
        let r = chip(s, x, label_w, false);
        draw::text(s, &canvas, &cfg, &label, 13.0, Face::Text(560), ink, x, mid, label_w, pango::Alignment::Center);
        self.hits.push((Hit::Source, r));
        x += label_w + 8.0;
        let timer = self.timer;
        for (hit, icon, on) in [(Hit::Grid, if self.grid { "view_carousel" } else { "grid_view" }, self.grid), (Hit::Timer, "timer", timer), (Hit::Shuffle, "shuffle", false)] {
            let r = chip(s, x, CHIP, on);
            draw::icon(s, &canvas, &cfg, icon, 18.0, on, if on { pal.on_accent } else { ink }, x + CHIP / 2.0, mid);
            self.hits.push((hit, r));
            x += CHIP + 6.0;
        }
        x += 4.0;
        for (i, (_, hex)) in HUES.iter().enumerate() {
            let c = Rgb::hex(hex).unwrap_or(pal.accent);
            let on = self.hue == Some(i);
            let size = CHIP - 2.0;
            if on {
                let ring = rounded(x - 2.0, mid - size / 2.0 - 3.0, size + 4.0, size + 4.0, 10.0);
                s.push_rounded_clip(&ring);
                s.append_color(&ink.rgba(), ring.bounds());
                s.pop();
            }
            let r = rounded(x, mid - size / 2.0 - 1.0, size, size, 8.0);
            s.push_rounded_clip(&r);
            s.append_color(&c.rgba(), r.bounds());
            s.pop();
            self.hits.push((Hit::Hue(i), *r.bounds()));
            x += CHIP + 6.0;
        }
        let r = chip(s, x, CHIP, !self.query.is_empty());
        draw::icon(s, &canvas, &cfg, "search", 18.0, false, if self.query.is_empty() { ink } else { pal.on_accent }, x + CHIP / 2.0, mid);
        self.hits.push((Hit::Search, r));
        if self.items.is_empty() {
            let msg = if self.all.is_empty() { "No pictures here" } else { "Nothing matches" };
            draw::text(s, &canvas, &cfg, msg, 16.0, Face::Text(480), ink, 0.0, cy, sw, pango::Alignment::Center);
        } else if self.grid {
            self.draw_grid(s, now, bar_y + BAR_H + 24.0, rise);
        } else {
            self.draw_carousel(s, now, cy, rise);
        }
        if let Some(path) = self.items.get(self.pointed) {
            let name = std::path::Path::new(path).file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let count = format!("{} / {}", self.pointed + 1, self.items.len());
            let y = if self.grid { sh - 34.0 } else { cy + BIG_H / 2.0 + 30.0 };
            draw::text(s, &canvas, &cfg, &name, 14.0, Face::Text(520), ink, 0.0, y, sw, pango::Alignment::Center);
            draw::text(s, &canvas, &cfg, &count, 12.0, Face::Text(420), pal.ink_meta, 0.0, y + 20.0, sw, pango::Alignment::Center);
        }
        s.pop();
    }

    fn draw_carousel(&mut self, s: &gtk::Snapshot, now: i64, cy: f64, rise: f64) {
        let (sw, _) = self.size;
        let focus = self.focus.value(now);
        let cx = sw / 2.0;
        let lo = (focus - 9.0).floor().max(0.0) as usize;
        let hi = ((focus + 9.0).ceil() as usize).min(self.items.len().saturating_sub(1));
        let mut order: Vec<usize> = (lo..=hi).collect();
        order.sort_by(|a, b| (*b as f64 - focus).abs().partial_cmp(&(*a as f64 - focus).abs()).unwrap_or(std::cmp::Ordering::Equal));
        let k = (SKEW as f64).to_radians().tan().abs();
        for i in order {
            let d = i as f64 - focus;
            let t = d.abs();
            let (w, h) = (width_at(t), height_at(t));
            let enter = (rise - t * 0.04).clamp(0.0, 1.0);
            let x = cx + offset(d) * (0.86 + 0.14 * enter);
            let reach = w + h * k;
            if x + reach < 0.0 || x - reach > sw {
                continue;
            }
            let lift = (1.0 - enter) * 26.0;
            let rect = graphene::Rect::new((x - reach / 2.0) as f32, (cy + lift - h / 2.0) as f32, reach as f32, h as f32);
            let path = &self.items[i];
            let bigness = (1.0 - t).clamp(0.0, 1.0);
            let big = self.big.get(path).filter(|_| bigness > 0.0);
            let side = self.side.get(path);
            let a_side = if side.is_some() { self.arrived("side", path, now) } else { 0.0 };
            let a_big = if big.is_some() { self.arrived("big", path, now) * if side.is_some() { bigness } else { 1.0 } } else { 0.0 };
            let cover = a_side.max(a_big);
            if cover < 1.0 {
                if let Some((small, large)) = &self.ghosts {
                    s.push_opacity(0.07 * (1.0 - cover) * enter);
                    s.append_scaled_texture(if bigness > 0.5 { large } else { small }, gsk::ScalingFilter::Linear, &rect);
                    s.pop();
                }
            }
            let layer = |s: &gtk::Snapshot, tex: &gdk::Texture, a: f64| {
                if a <= 0.003 {
                    return;
                }
                if a < 1.0 {
                    s.push_opacity(a);
                }
                s.append_scaled_texture(tex, gsk::ScalingFilter::Linear, &rect);
                if a < 1.0 {
                    s.pop();
                }
            };
            if let Some(sd) = side {
                if a_big < 1.0 {
                    layer(s, sd, a_side);
                }
            }
            if let Some(b) = big {
                layer(s, b, a_big);
            }
            if t < 0.5 {
                if let Some(r) = &self.ring {
                    s.push_opacity(((0.5 - t) * 1.4).min(0.7));
                    s.append_scaled_texture(r, gsk::ScalingFilter::Linear, &rect);
                    s.pop();
                }
            }
            let hit_w = (w * 0.9).max(40.0);
            self.hits.push((Hit::Card(i), graphene::Rect::new((x - hit_w / 2.0) as f32, (cy - h / 2.0) as f32, hit_w as f32, h as f32)));
        }
    }

    fn draw_grid(&mut self, s: &gtk::Snapshot, now: i64, top: f64, rise: f64) {
        let (sw, sh) = self.size;
        let cols = (((sw - 160.0) + GRID_GAP) / (GRID_W + GRID_GAP)).floor().clamp(2.0, 7.0) as usize;
        let total_w = cols as f64 * (GRID_W + GRID_GAP) - GRID_GAP;
        let left = ((sw - total_w) / 2.0).round();
        let rows_visible = ((sh - top - 80.0) / (GRID_H + GRID_GAP)).floor().max(1.0);
        let scroll = (self.scroll.value(now) - (rows_visible / 2.0).floor()).max(0.0);
        let focus = self.focus.value(now);
        s.push_clip(&graphene::Rect::new(0.0, top as f32, sw as f32, (sh - top - 70.0) as f32));
        for (i, path) in self.items.iter().enumerate() {
            let (row, col) = ((i / cols) as f64, (i % cols) as f64);
            let y = top + (row - scroll) * (GRID_H + GRID_GAP) + (1.0 - rise) * 20.0;
            if y + GRID_H < top - 10.0 || y > sh {
                continue;
            }
            let x = left + col * (GRID_W + GRID_GAP);
            let near = (1.0 - (focus - i as f64).abs()).clamp(0.0, 1.0);
            if near > 0.01 {
                let ring = rounded(x - 3.0, y - 3.0, GRID_W + 6.0, GRID_H + 6.0, 16.0);
                let c = gdk::RGBA::new(1.0, 1.0, 1.0, near as f32 * 0.9);
                s.append_border(&ring, &[2.0; 4], &[c, c, c, c]);
            }
            let card = rounded(x, y, GRID_W, GRID_H, 13.0);
            s.push_rounded_clip(&card);
            s.append_color(&gdk::RGBA::new(0.08, 0.08, 0.09, 1.0), card.bounds());
            self.texture(s, path, x, y, GRID_W, GRID_H, now);
            s.pop();
            self.hits.push((Hit::Card(i), *card.bounds()));
        }
        s.pop();
    }
}

mod imp {
    use super::State;
    use gtk::subclass::prelude::*;
    use gtk::{glib, prelude::*};
    use std::cell::RefCell;
    use std::rc::Weak;

    #[derive(Default)]
    pub struct Canvas {
        pub(super) state: RefCell<Weak<RefCell<State>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Canvas {
        const NAME: &'static str = "DigitPicker";
        type Type = super::Canvas;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Canvas {}

    impl WidgetImpl for Canvas {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let Some(state) = self.state.borrow().upgrade() else { return };
            let Ok(mut s) = state.try_borrow_mut() else {
                self.obj().queue_draw();
                return;
            };
            let now = self.obj().frame_clock().map(|c| c.frame_time()).unwrap_or_else(glib::monotonic_time);
            s.draw(snapshot, now);
        }
    }
}

glib::wrapper! {
    pub struct Canvas(ObjectSubclass<imp::Canvas>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}
