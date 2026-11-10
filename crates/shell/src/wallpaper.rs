use gtk::gdk_pixbuf::Pixbuf;
use gtk::subclass::prelude::*;
use gtk::{gdk, gio, glib, graphene, gsk, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kit::motion;
use kit::theme;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

const SPAN: i64 = 1_400_000;

fn state_file() -> PathBuf {
    theme::state_dir().join("wallpaper")
}

pub fn current() -> Option<String> {
    let path = std::fs::read_to_string(state_file()).ok()?.trim().to_string();
    std::path::Path::new(&path).is_file().then_some(path)
}

pub fn remember(path: &str) -> bool {
    if !std::path::Path::new(path).is_file() {
        return false;
    }
    if !history(path) {
        return false;
    }
    atomic_write(&state_file(), path).is_ok()
}

fn atomic_write(file: &std::path::Path, text: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(file.parent().unwrap())?;
    let tmp = file.with_extension("new");
    std::fs::write(&tmp, text)?;
    std::fs::rename(tmp, file)
}

fn history(path: &str) -> bool {
    if !std::path::Path::new(path).is_file() { return false; }
    let mut used = used();
    used.retain(|p| p != path);
    used.insert(0, path.to_string());
    used.truncate(60);
    atomic_write(&theme::state_dir().join("wallpapers-used"), &used.join("\n")).is_ok()
}

pub fn used() -> Vec<String> {
    std::fs::read_to_string(theme::state_dir().join("wallpapers-used"))
        .unwrap_or_default()
        .lines()
        .filter(|l| std::path::Path::new(l).is_file())
        .map(str::to_string)
        .collect()
}

pub fn is_image(p: &std::path::Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "jpg" | "jpeg" | "png" | "webp" | "avif" | "jxl" | "bmp" | "tif" | "tiff"))
}

pub fn listed(dir: &std::path::Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<(std::time::SystemTime, String)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_image(p))
        .map(|p| (std::fs::metadata(&p).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), p.to_string_lossy().into_owned()))
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    files.into_iter().map(|(_, p)| p).collect()
}

/// Shared by the picker, slideshow and first-run wallpaper discovery.
pub fn directories() -> Vec<PathBuf> {
    let t = kit::prefs::read();
    let custom = kit::prefs::text(&t, "wall_dir", "");
    let pictures = glib::user_special_dir(glib::UserDirectory::Pictures).unwrap_or_else(|| glib::home_dir().join("Pictures"));
    search_directories(&custom, pictures)
}

fn search_directories(custom: &str, pictures: PathBuf) -> Vec<PathBuf> {
    let custom = custom.trim();
    if !custom.is_empty() && !custom.chars().any(char::is_control) {
        let path = kit::prefs::home(custom);
        if path.is_absolute() {
            return vec![path];
        }
    }
    vec![pictures.join("Wallpapers"), pictures.join("wallpapers"), pictures]
}

pub fn thumbnail(path: &str) -> Option<gdk::Texture> {
    let file = gio::File::for_path(path);
    let uri = file.uri();
    let mtime = std::fs::metadata(path).ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs().to_string();
    let name = glib::compute_checksum_for_string(glib::ChecksumType::Md5, uri.as_str())?;
    let dir = glib::user_cache_dir().join("thumbnails/large");
    let cached = dir.join(format!("{name}.png"));
    let fresh = Pixbuf::from_file(&cached).ok().filter(|p| p.option("tEXt::Thumb::MTime").as_deref() == Some(mtime.as_str()));
    let pixbuf = match fresh {
        Some(p) => p,
        None => {
            let p = Pixbuf::from_file_at_scale(path, 256, 256, true).ok()?;
            let _ = std::fs::create_dir_all(&dir);
            let tmp = dir.join(format!("{name}.digit.png"));
            if p.savev(&tmp, "png", &[("tEXt::Thumb::URI", uri.as_str()), ("tEXt::Thumb::MTime", mtime.as_str())]).is_ok() {
                let _ = std::fs::rename(&tmp, &cached);
            }
            p
        }
    };
    let format = if pixbuf.has_alpha() { gdk::MemoryFormat::R8g8b8a8 } else { gdk::MemoryFormat::R8g8b8 };
    Some(gdk::MemoryTexture::new(pixbuf.width(), pixbuf.height(), format, &pixbuf.read_pixel_bytes(), pixbuf.rowstride() as usize).upcast())
}

pub fn decode(path: &str, w: i32, h: i32) -> Option<gdk::Texture> {
    decode_all(path, &[(w, h)]).into_iter().next().and_then(|(_, t)| t)
}

pub fn decode_all(path: &str, sizes: &[(i32, i32)]) -> Vec<((i32, i32), Option<gdk::Texture>)> {
    let tag = |(w, h): (i32, i32)| format!("screen-{w}x{h}");
    let mut out: Vec<((i32, i32), Option<gdk::Texture>)> = sizes.iter().map(|&s| (s, crate::stash::get(path, &tag(s)).filter(|t| (t.width(), t.height()) == s))).collect();
    let missing: Vec<(i32, i32)> = out.iter().filter(|(_, t)| t.is_none()).map(|(s, _)| *s).collect();
    if missing.is_empty() {
        return out;
    }
    if let Some(master) = crate::stash::master(path, &missing) {
        for (size, slot) in out.iter_mut().filter(|(_, t)| t.is_none()) {
            if let Some(pb) = crate::stash::cover(&master, size.0, size.1) {
                let t = crate::stash::texture(&pb);
                crate::stash::put(path, &tag(*size), t.clone());
                *slot = Some(t);
            }
        }
    }
    unsafe {
        libc::malloc_trim(0);
    }
    out
}

pub fn found() -> Option<String> {
    let image = |p: &std::path::Path| {
        p.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "jpg" | "jpeg" | "png" | "webp" | "avif" | "jxl"))
    };
    for dir in directories() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_file() && image(p)).collect();
        files.sort();
        if let Some(first) = files.first() {
            return Some(first.to_string_lossy().into_owned());
        }
    }
    None
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
    #[default]
    Same,
    Span,
    Separate,
}

impl Layout {
    pub fn read(prefs: &toml::Table) -> Self {
        match prefs.get("wallpaper_layout").and_then(toml::Value::as_str) {
            Some("span") => Self::Span,
            Some("separate") => Self::Separate,
            _ => Self::Same,
        }
    }

    pub fn key(self) -> &'static str {
        match self { Self::Same => "same", Self::Span => "span", Self::Separate => "separate" }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    fn sub(self, part: Self) -> Self {
        Self { x: self.x + part.x * self.w, y: self.y + part.y * self.h, w: part.w * self.w, h: part.h * self.h }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Monitor {
    pub output: String,
    pub rect: Rect,
    pub scale: f64,
}

impl Monitor {
    fn pixels(&self) -> (i32, i32) {
        ((self.rect.w * self.scale).ceil().max(1.0) as i32, (self.rect.h * self.scale).ceil().max(1.0) as i32)
    }
}

pub fn bounds(monitors: &[Monitor]) -> Rect {
    let x = monitors.iter().map(|m| m.rect.x).reduce(f64::min).unwrap_or(0.0);
    let y = monitors.iter().map(|m| m.rect.y).reduce(f64::min).unwrap_or(0.0);
    let right = monitors.iter().map(|m| m.rect.x + m.rect.w).reduce(f64::max).unwrap_or(1.0);
    let bottom = monitors.iter().map(|m| m.rect.y + m.rect.h).reduce(f64::max).unwrap_or(1.0);
    Rect { x, y, w: (right - x).max(1.0), h: (bottom - y).max(1.0) }
}

/// Logical positions and dimensions already account for scale; scale only sizes the decode.
pub fn span_regions(monitors: &[Monitor]) -> Vec<Rect> {
    let b = bounds(monitors);
    monitors.iter().map(|m| Rect { x: (m.rect.x - b.x) / b.w, y: (m.rect.y - b.y) / b.h, w: m.rect.w / b.w, h: m.rect.h / b.h }).collect()
}

/// The normalized source rectangle for a centered cover crop, without distortion.
pub fn cover_crop(image: (f64, f64), target: (f64, f64)) -> Rect {
    let k = (target.0 / image.0.max(1.0)).max(target.1 / image.1.max(1.0)).max(f64::EPSILON);
    let w = (target.0 / (image.0.max(1.0) * k)).clamp(0.0, 1.0);
    let h = (target.1 / (image.1.max(1.0) * k)).clamp(0.0, 1.0);
    Rect { x: (1.0 - w) / 2.0, y: (1.0 - h) / 2.0, w, h }
}

pub fn image_for<'a>(layout: Layout, prefs: &'a toml::Table, output: &str, global: &'a str) -> &'a str {
    if layout == Layout::Separate {
        prefs.get("wallpapers").and_then(toml::Value::as_table).and_then(|t| t.get(output)).and_then(toml::Value::as_str).filter(|p| !p.is_empty()).unwrap_or(global)
    } else { global }
}

fn connector_choices(prefs: &toml::Table, changes: impl IntoIterator<Item = (String, String)>) -> toml::Table {
    let mut choices = prefs.get("wallpapers").and_then(toml::Value::as_table).cloned().unwrap_or_default();
    for (output, path) in changes { choices.insert(output, toml::Value::String(path)); }
    choices
}

pub fn theme_source<'a>(layout: Layout, prefs: &'a toml::Table, monitors: &[Monitor], global: &'a str) -> &'a str {
    let primary = prefs.get("primary_monitor").and_then(toml::Value::as_str).unwrap_or("");
    let monitor = monitors.iter().find(|m| m.output == primary).or(monitors.first());
    monitor.map_or(global, |m| image_for(layout, prefs, &m.output, global))
}

/// Consecutive folder entries, wrapping only when there are fewer images than outputs.
fn rotation_images<'a>(images: &'a [String], current: &str, count: usize) -> Vec<&'a str> {
    if images.is_empty() { return Vec::new(); }
    let start = images.iter().position(|p| p == current).map_or(0, |i| i + 1);
    (0..count).map(|i| images[(start + i) % images.len()].as_str()).collect()
}

#[derive(Clone)]
struct Frame {
    texture: gdk::Texture,
    path: String,
    region: Rect,
    span_size: Option<(i32, i32)>,
    luma: std::sync::Arc<Vec<f32>>,
}

impl Frame {
    fn region_for(&self, w: f64, h: f64) -> Rect {
        let image = (self.texture.width() as f64 * self.region.w, self.texture.height() as f64 * self.region.h);
        self.region.sub(cover_crop(image, (w, h)))
    }

    fn fit(&self, w: f64, h: f64, zoom: f64) -> graphene::Rect {
        let zoom = if self.span_size.is_some() { 1.0 } else { zoom };
        let r = self.region_for(w, h);
        let dw = w / r.w * zoom;
        let dh = h / r.h * zoom;
        graphene::Rect::new(((w - w * zoom) / 2.0 - r.x * dw) as f32, ((h - h * zoom) / 2.0 - r.y * dh) as f32, dw as f32, dh as f32)
    }
}

#[derive(Clone, Default)]
struct Scene {
    old: Option<Frame>,
    new: Option<Frame>,
    prime: Option<Frame>,
    warmed: bool,
    start: i64,
    span: i64,
    blend: bool,
    ticking: bool,
}

impl Scene {
    fn progress(&self, now: i64) -> f64 {
        if motion::reduced() || self.old.is_none() || self.start == 0 {
            return 1.0;
        }
        let span = if self.blend { self.span.max(1) } else { SPAN };
        ((now - self.start) as f64 / span as f64).clamp(0.0, 1.0)
    }

    fn moving(&self, now: i64) -> bool {
        self.prime.is_some() || self.progress(now) < 1.0
    }

    fn step(&mut self, now: i64) {
        if self.prime.is_some() {
            if self.warmed {
                self.old = self.new.take();
                self.new = self.prime.take();
                self.warmed = false;
                self.blend = false;
                self.start = now;
            } else {
                self.warmed = true;
            }
        } else if self.progress(now) >= 1.0 {
            self.old = None;
        }
    }

    fn draw(&self, s: &gtk::Snapshot, w: f64, h: f64, now: i64) {
        let bounds = graphene::Rect::new(0.0, 0.0, w as f32, h as f32);
        let t = self.progress(now);
        let p = motion::REVEAL.at(t);
        s.push_clip(&bounds);
        if let (Some(old), true) = (&self.old, t < 1.0) {
            let grow = if self.blend { 0.0 } else { 0.015 * p };
            s.append_scaled_texture(&old.texture, gsk::ScalingFilter::Linear, &old.fit(w, h, 1.0 + grow));
        }
        if let Some(new) = &self.new {
            if t >= 1.0 {
                s.append_texture(&new.texture, &new.fit(w, h, 1.0));
            } else if self.blend {
                let e = motion::STANDARD.at(t);
                s.push_opacity(e);
                s.append_scaled_texture(&new.texture, gsk::ScalingFilter::Linear, &new.fit(w, h, 1.02 - 0.02 * e));
                s.pop();
            } else {
                let reach = (w * w + h * h).sqrt() / 2.0;
                let soft = reach * 0.35;
                let radius = (p * (reach + soft)).max(1.0);
                let hard = ((radius - soft) / radius).clamp(0.0, 1.0) as f32;
                let white = |a: f32| gdk::RGBA::new(1.0, 1.0, 1.0, a);
                s.push_mask(gsk::MaskMode::Alpha);
                s.append_radial_gradient(
                    &bounds,
                    &graphene::Point::new((w / 2.0) as f32, (h / 2.0) as f32),
                    radius as f32,
                    radius as f32,
                    0.0,
                    1.0,
                    &[gsk::ColorStop::new(0.0, white(1.0)), gsk::ColorStop::new(hard, white(1.0)), gsk::ColorStop::new(1.0, white(0.0))],
                );
                s.pop();
                s.append_scaled_texture(&new.texture, gsk::ScalingFilter::Linear, &new.fit(w, h, 1.035 - 0.035 * p));
                s.pop();
            }
        }
        if let Some(prime) = &self.prime {
            s.push_opacity(0.004);
            s.append_texture(&prime.texture, &prime.fit(w, h, 1.0));
            s.pop();
        }
        s.pop();
    }
}

mod imp {
    use super::Scene;
    use gtk::subclass::prelude::*;
    use gtk::{glib, prelude::*};
    use std::cell::RefCell;

    #[derive(Default)]
    pub struct Canvas {
        pub(super) scene: RefCell<Scene>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Canvas {
        const NAME: &'static str = "DigitWallpaper";
        type Type = super::Canvas;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Canvas {}

    impl WidgetImpl for Canvas {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let w = self.obj();
            let now = w.frame_clock().map(|c| c.frame_time()).unwrap_or_else(glib::monotonic_time);
            let scene = self.scene.borrow().clone();
            scene.draw(snapshot, w.width() as f64, w.height() as f64, now);
        }
    }
}

glib::wrapper! {
    pub struct Canvas(ObjectSubclass<imp::Canvas>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Canvas {
    fn show(&self, texture: Frame) {
        {
            let mut scene = self.imp().scene.borrow_mut();
            if scene.new.is_none() || motion::reduced() {
                scene.old = None;
                scene.prime = None;
                scene.new = Some(texture);
                drop(scene);
                self.queue_draw();
                return;
            }
            scene.prime = Some(texture);
            scene.warmed = false;
        }
        self.tick();
    }

    fn tick(&self) {
        if self.imp().scene.borrow_mut().ticking {
            return;
        }
        self.imp().scene.borrow_mut().ticking = true;
        self.add_tick_callback(|w, clock| {
            let now = clock.frame_time();
            let mut scene = w.imp().scene.borrow_mut();
            scene.step(now);
            let moving = scene.moving(now);
            if !moving {
                scene.ticking = false;
            }
            drop(scene);
            w.queue_draw();
            if moving {
                glib::ControlFlow::Continue
            } else {
                crate::app::App::trim();
                glib::ControlFlow::Break
            }
        });
    }

    fn swap(&self, texture: Frame, ms: i64) {
        if motion::reduced() { self.show(texture); return; }
        let now = glib::monotonic_time();
        {
            let mut scene = self.imp().scene.borrow_mut();
            let early = scene.old.is_some() && scene.progress(now) < 0.5;
            scene.prime = None;
            scene.warmed = false;
            let before = scene.new.replace(texture);
            if !early {
                scene.old = before;
            }
            scene.start = now;
            scene.span = ms * 1000;
            scene.blend = true;
        }
        self.tick();
    }

    fn remaining_ms(&self) -> i64 {
        let now = glib::monotonic_time();
        let scene = self.imp().scene.borrow();
        if scene.prime.is_some() {
            return SPAN / 1000;
        }
        ((1.0 - scene.progress(now)) * (SPAN / 1000) as f64) as i64
    }
}

struct Render {
    output: String,
    path: String,
    size: (i32, i32),
    view: Rect,
}

fn decode_frames(requests: Vec<Render>, layout: Layout, b: Rect, global: String, live: Vec<Frame>) -> Vec<(String, Frame)> {
    render_frames(requests, layout, b, global, |path, size| {
        if let Some(frame) = live.iter().find(|f| f.path == path &&
            if layout == Layout::Span { f.span_size == Some(size) }
            else { f.span_size.is_none() && (f.texture.width(), f.texture.height()) == size }) {
            return Some(frame.texture.clone());
        }
        if layout == Layout::Span { crate::stash::master(path, &[size]).and_then(|pb| crate::stash::cover(&pb, size.0, size.1)).map(|pb| crate::stash::texture(&pb)) }
        else { decode(path, size.0, size.1) }
    })
}

fn render_frames(requests: Vec<Render>, layout: Layout, b: Rect, global: String, mut decode: impl FnMut(&str, (i32, i32)) -> Option<gdk::Texture>) -> Vec<(String, Frame)> {
    let mut decoded = HashMap::new();
    let mut get = |path: &str, size| decoded.entry((path.to_string(), size)).or_insert_with(|| decode(path, size)).clone();
    let mut maps: HashMap<usize, std::sync::Arc<Vec<f32>>> = HashMap::new();
    let mut out = Vec::new();
    for Render { output, mut path, size, view } in requests {
        let mut texture = get(&path, size);
        if texture.is_none() && path != global {
            path = global.clone();
            texture = get(&path, size);
        }
        if let Some(texture) = texture {
            let crop = cover_crop((texture.width() as f64, texture.height() as f64), if layout == Layout::Span { (b.w, b.h) } else { (size.0 as f64, size.1 as f64) });
            let region = if layout == Layout::Span { crop.sub(view) } else { crop };
            let luma = maps.entry(texture.as_ptr() as usize).or_insert_with(|| {
                let mut downloader = gdk::TextureDownloader::new(&texture);
                downloader.set_format(gdk::MemoryFormat::B8g8r8a8);
                let (bytes, stride) = downloader.download_bytes();
                let pb = Pixbuf::from_bytes(&bytes, gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, texture.width(), texture.height(), stride as i32);
                let small = pb.scale_simple(128, 72, gtk::gdk_pixbuf::InterpType::Bilinear).unwrap();
                std::sync::Arc::new(small.read_pixel_bytes().chunks_exact(4).map(|p| luminance(p[0], p[1], p[2]) as f32).collect())
            }).clone();
            out.push((output, Frame { texture, path, region, luma, span_size: (layout == Layout::Span).then_some(size) }));
        }
    }
    out
}

struct Screen {
    monitor: RefCell<Monitor>,
    window: gtk::Window,
    canvas: Canvas,
}

#[derive(Default)]
struct DecodeQueue {
    running: Cell<bool>,
    pending: Cell<Option<Option<i64>>>,
}

impl DecodeQueue {
    fn begin(&self, fade: Option<i64>) -> bool {
        if self.running.replace(true) { self.pending.set(Some(fade)); false } else { true }
    }

    fn finish(&self) -> Option<Option<i64>> {
        self.running.set(false);
        self.pending.take()
    }
}

#[derive(Default)]
pub struct Wallpapers {
    screens: RefCell<Vec<Rc<Screen>>>,
    shown: RefCell<String>,
    prefs: RefCell<toml::Table>,
    previews: RefCell<HashMap<String, String>>,
    loading: Cell<u64>,
    decode_queue: DecodeQueue,
    ready: RefCell<Option<Box<dyn FnOnce()>>>,
}

impl Wallpapers {
    fn screens(&self) -> Vec<Rc<Screen>> { self.screens.borrow().clone() }

    pub fn monitors(&self) -> Vec<Monitor> {
        self.screens().iter().map(|s| s.monitor.borrow().clone()).collect()
    }

    pub fn layout(&self) -> Layout { Layout::read(&self.prefs.borrow()) }

    pub fn place(self: &Rc<Self>, monitors: &[gdk::Monitor]) {
        // GDK supplies logical sizes and fractional scales. Compositor positions also cover
        // backends that don't expose output positions through xdg-output.
        self.place_at(monitors, &kit::wm::outputs());
    }

    fn place_at(self: &Rc<Self>, monitors: &[gdk::Monitor], positions: &[kit::wm::Output]) {
        let info: Vec<Monitor> = monitors.iter().map(|m| {
            let g = m.geometry();
            let output = m.connector().map(|c| c.to_string()).unwrap_or_default();
            let pos = positions.iter().find(|p| p.name == output);
            Monitor { output, rect: Rect { x: pos.map_or(g.x() as f64, |p| p.x), y: pos.map_or(g.y() as f64, |p| p.y), w: g.width().max(1) as f64, h: g.height().max(1) as f64 }, scale: m.scale().max(0.25) }
        }).collect();
        let old = self.screens();
        let mut next = Vec::new();
        let mut changed = self.monitors() != info;
        for (monitor, meta) in monitors.iter().zip(info) {
            if let Some(screen) = old.iter().find(|s| s.monitor.borrow().output == meta.output) {
                *screen.monitor.borrow_mut() = meta;
                next.push(screen.clone());
                continue;
            }
            let window = gtk::Window::new();
            window.init_layer_shell();
            window.set_layer(Layer::Background);
            window.set_namespace(Some("digit-wallpaper"));
            for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] { window.set_anchor(edge, true); }
            window.set_exclusive_zone(-1);
            window.set_keyboard_mode(KeyboardMode::None);
            window.set_monitor(Some(monitor));
            window.add_css_class("digit-wallpaper");
            let canvas: Canvas = glib::Object::new();
            canvas.set_hexpand(true);
            canvas.set_vexpand(true);
            window.set_child(Some(&canvas));
            // Seed a hotplugged surface immediately; the correct region replaces it after decode.
            let seed = old.iter().find_map(|s| s.canvas.imp().scene.borrow().new.clone());
            if let Some(frame) = seed { canvas.show(frame); }
            window.present();
            next.push(Rc::new(Screen { monitor: RefCell::new(meta), window, canvas }));
            changed = true;
        }
        *self.screens.borrow_mut() = next.clone();
        for screen in old.iter().filter(|s| !next.iter().any(|n| Rc::ptr_eq(s, n))) { screen.window.destroy(); }
        let prefs = kit::prefs::read();
        if *self.prefs.borrow() != prefs {
            *self.prefs.borrow_mut() = prefs;
            changed = true;
        }
        if changed { self.load(None); }
    }

    /// Also called after Settings changes, including connector choices and the primary output.
    pub fn prefs_changed(self: &Rc<Self>) -> bool {
        let before = self.theme_image();
        let prefs = kit::prefs::read();
        let changed = {
            let before = self.prefs.borrow();
            ["wallpaper_layout", "wallpapers"].iter().any(|key| before.get(*key) != prefs.get(*key))
        };
        *self.prefs.borrow_mut() = prefs;
        if changed {
            self.previews.borrow_mut().clear();
            self.load(Some(420));
        }
        before != self.theme_image()
    }

    pub fn size_of(&self, output: &str) -> Option<(i32, i32)> {
        self.monitors().iter().find(|m| m.output == output).map(Monitor::pixels)
    }

    pub fn is_empty(&self) -> bool { self.shown.borrow().is_empty() }

    pub fn when_ready(&self, f: impl FnOnce() + 'static) { *self.ready.borrow_mut() = Some(Box::new(f)); }

    pub fn remaining_ms(&self) -> i64 {
        self.screens().iter().map(|s| s.canvas.remaining_ms()).max().unwrap_or(0)
    }

    pub fn preview(self: &Rc<Self>, path: String, output: &str) {
        self.previews.borrow_mut().clear();
        self.previews.borrow_mut().insert(output.to_string(), path);
        self.load(Some(340));
    }

    pub fn unpreview(self: &Rc<Self>, _output: &str) {
        self.previews.borrow_mut().clear();
        self.load(Some(420));
    }

    pub fn commit(self: &Rc<Self>, path: String, output: &str) -> bool {
        if self.layout() == Layout::Separate {
            if !history(&path) { return false; }
            let prefs = kit::prefs::read();
            let choices = connector_choices(&prefs, [(output.to_string(), path)]);
            if let Err(e) = kit::prefs::set("wallpapers", toml::Value::Table(choices)) {
                eprintln!("digit: {e}");
                return false;
            }
            *self.prefs.borrow_mut() = kit::prefs::read();
        } else {
            if !remember(&path) { return false; }
            *self.shown.borrow_mut() = path;
        }
        self.previews.borrow_mut().clear();
        self.load(None);
        true
    }

    pub fn shown(&self) -> String { self.shown.borrow().clone() }

    pub fn chosen(&self, output: &str) -> String {
        let global = self.shown();
        let prefs = self.prefs.borrow();
        image_for(Layout::read(&prefs), &prefs, output, &global).to_string()
    }

    pub fn theme_image(&self) -> Option<String> {
        let global = self.shown();
        let prefs = self.prefs.borrow();
        let path = theme_source(Layout::read(&prefs), &prefs, &self.monitors(), &global).to_string();
        if std::path::Path::new(&path).is_file() { Some(path) } else { current().or_else(found) }
    }

    pub fn preview_frames(&self) -> Vec<(String, gdk::Texture, Rect)> {
        self.screens().iter().filter_map(|s| {
            let monitor = s.monitor.borrow().clone();
            let frame = s.canvas.imp().scene.borrow().new.clone()?;
            let region = frame.region_for(monitor.rect.w, monitor.rect.h);
            Some((monitor.output, frame.texture, region))
        }).collect()
    }

    pub fn texture(&self, output: &str) -> Option<gdk::Texture> {
        let screens = self.screens();
        let screen = screens.iter().find(|s| s.monitor.borrow().output == output).or(screens.first())?;
        let frame = screen.canvas.imp().scene.borrow().new.clone()?;
        // The lock screen consumes a texture, so crop only on that explicit request.
        let monitor = screen.monitor.borrow().clone();
        let r = frame.region_for(monitor.rect.w, monitor.rect.h);
        if r == (Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }) { return Some(frame.texture); }
        let t = frame.texture;
        let mut downloader = gdk::TextureDownloader::new(&t);
        downloader.set_format(gdk::MemoryFormat::R8g8b8a8);
        let (bytes, stride) = downloader.download_bytes();
        let pb = Pixbuf::from_bytes(&bytes, gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, t.width(), t.height(), stride as i32);
        let (w, h) = (t.width() as f64, t.height() as f64);
        let crop = pb.new_subpixbuf((r.x * w).floor() as i32, (r.y * h).floor() as i32, (r.w * w).floor().max(1.0) as i32, (r.h * h).floor().max(1.0) as i32).copy()?;
        Some(crate::stash::texture(&crop))
    }

    pub fn top_luma(&self, output: &str, band: f64) -> Option<f64> {
        let screen = self.screens().into_iter().find(|s| s.monitor.borrow().output == output)?;
        let frame = screen.canvas.imp().scene.borrow().new.clone()?;
        let monitor = screen.monitor.borrow().clone();
        let region = frame.region_for(monitor.rect.w, monitor.rect.h);
        Some(region_luma(&frame.luma, 128, 72, region, band))
    }

    pub fn set(self: &Rc<Self>, path: String) {
        *self.shown.borrow_mut() = path;
        *self.prefs.borrow_mut() = kit::prefs::read();
        self.previews.borrow_mut().clear();
        self.load(None);
    }

    pub fn rotate(self: &Rc<Self>, images: &[String]) -> bool {
        let monitors = self.monitors();
        let choices = rotation_images(images, &self.shown(), if self.layout() == Layout::Separate { monitors.len().max(1) } else { 1 });
        let Some(first) = choices.first() else { return false };
        if self.layout() == Layout::Separate {
            let prefs = kit::prefs::read();
            let table = connector_choices(&prefs, monitors.iter().zip(&choices).map(|(monitor, image)| (monitor.output.clone(), (*image).to_string())));
            if let Err(e) = kit::prefs::set("wallpapers", toml::Value::Table(table)) {
                eprintln!("digit: {e}");
                return false;
            }
        }
        if !remember(first) { return false; }
        self.set((*first).to_string());
        true
    }

    fn load(self: &Rc<Self>, fade: Option<i64>) {
        let generation = self.loading.get() + 1;
        self.loading.set(generation);
        let monitors = self.monitors();
        if monitors.is_empty() || self.is_empty() { return; }
        if !self.decode_queue.begin(fade) { return; }
        let layout = self.layout();
        let b = bounds(&monitors);
        let scale = monitors.iter().map(|m| m.scale).fold(1.0, f64::max);
        let span_size = ((b.w * scale).ceil() as i32, (b.h * scale).ceil() as i32);
        let regions = span_regions(&monitors);
        let prefs = self.prefs.borrow().clone();
        let previews = self.previews.borrow().clone();
        let global = self.shown();
        let mut requests = Vec::new();
        for (monitor, region) in monitors.iter().zip(regions) {
            let path = if layout == Layout::Span { previews.values().next() } else { previews.get(&monitor.output) };
            let path = path.cloned().unwrap_or_else(|| image_for(layout, &prefs, &monitor.output, &global).to_string());
            requests.push(Render { output: monitor.output.clone(), path, size: if layout == Layout::Span { span_size } else { monitor.pixels() }, view: region });
        }
        // Reuse live frames as well as the disk cache. A span keeps one shared texture,
        // including while changing geometry or previewing on multiple outputs.
        let live: Vec<Frame> = self.screens().iter().flat_map(|s| {
            let scene = s.canvas.imp().scene.borrow();
            [scene.new.clone(), scene.old.clone(), scene.prime.clone()].into_iter().flatten().collect::<Vec<_>>()
        }).collect();
        let this = self.clone();
        glib::spawn_future_local(async move {
            let frames = gio::spawn_blocking(move || {
                decode_frames(requests, layout, b, global, live)
            }).await;
            if let Some(fade) = this.decode_queue.finish() { this.load(fade); return; }
            let Ok(frames) = frames else { return };
            if this.loading.get() != generation { return; }
            let screens = this.screens();
            for (output, frame) in frames {
                if let Some(screen) = screens.iter().find(|s| s.monitor.borrow().output == output) {
                    let before = screen.canvas.imp().scene.borrow().new.clone();
                    if before.as_ref().is_some_and(|f| f.path == frame.path && f.region == frame.region && f.span_size == frame.span_size && f.texture.width() == frame.texture.width() && f.texture.height() == frame.texture.height()) { continue; }
                    let geometry_changed = before.as_ref().is_some_and(|f| f.path == frame.path && (f.region != frame.region || f.span_size != frame.span_size));
                    if let Some(ms) = fade.or(geometry_changed.then_some(420)) { screen.canvas.swap(frame, ms); } else { screen.canvas.show(frame); }
                }
            }
            let ready = this.ready.borrow_mut().take();
            if let Some(ready) = ready { glib::timeout_add_local_once(std::time::Duration::from_millis(150), ready); }
        });
    }

    pub fn destroy(&self) {
        self.loading.set(self.loading.get() + 1);
        self.decode_queue.pending.set(None);
        let screens: Vec<_> = self.screens.borrow_mut().drain(..).collect();
        for s in screens { s.window.destroy(); }
    }
}

/// Position-only changes aren't signalled by every Wayland backend. Read the compositor's
/// layout off the GTK thread too; this never changes compositor state.
pub fn watch_layout(app: &Rc<crate::app::App>) {
    let weak = Rc::downgrade(app);
    let pending = Rc::new(Cell::new(false));
    glib::timeout_add_local(std::time::Duration::from_millis(1500), move || {
        let Some(app) = weak.upgrade() else { return glib::ControlFlow::Break };
        if pending.replace(true) { return glib::ControlFlow::Continue; }
        let (weak, pending) = (Rc::downgrade(&app), pending.clone());
        glib::spawn_future_local(async move {
            let positions = gio::spawn_blocking(kit::wm::outputs).await.unwrap_or_default();
            pending.set(false);
            if positions.is_empty() && kit::wm::which() != kit::wm::Wm::Other { return; }
            let (Some(app), Some(display)) = (weak.upgrade(), gdk::Display::default()) else { return };
            let monitors = display.monitors().iter::<gdk::Monitor>().filter_map(Result::ok).collect::<Vec<_>>();
            let before = app.wallpapers.monitors();
            let theme = app.wallpapers.theme_image();
            app.wallpapers.place_at(&monitors, &positions);
            if theme != app.wallpapers.theme_image() { crate::retheme(&app); }
            if before != app.wallpapers.monitors() {
                let picker = app.picker.borrow().clone();
                if let Some(picker) = picker { picker.refresh_layout(); }
            }
        });
        glib::ControlFlow::Continue
    });
}

fn luminance(b: u8, g: u8, r: u8) -> f64 {
    let lin = |c: u8| { let c = c as f64 / 255.0; if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) } };
    0.0722 * lin(b) + 0.7152 * lin(g) + 0.2126 * lin(r)
}

fn region_luma(data: &[f32], w: usize, h: usize, region: Rect, band: f64) -> f64 {
    let left = (region.x * w as f64).floor() as usize;
    let top = (region.y * h as f64).floor() as usize;
    let right = ((region.x + region.w) * w as f64).ceil().min(w as f64) as usize;
    let bottom = ((region.y + region.h * band.clamp(0.0, 1.0)) * h as f64).ceil().max((top + 1) as f64).min(h as f64) as usize;
    let (mut sum, mut n) = (0.0, 0usize);
    for y in top..bottom {
        for x in left..right {
            sum += data[y * w + x] as f64;
            n += 1;
        }
    }
    if n == 0 { 0.0 } else { sum / n as f64 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_queue_replaces_pending_requests_with_the_latest() {
        let queue = DecodeQueue::default();
        assert!(queue.begin(None));
        assert!(!queue.begin(Some(340)));
        assert!(!queue.begin(Some(420)));
        assert!(!queue.begin(None));
        assert_eq!(queue.finish(), Some(None));
        assert!(queue.begin(Some(420)));
        assert_eq!(queue.finish(), None);
    }

    fn monitor(output: &str, x: f64, y: f64, w: f64, h: f64, scale: f64) -> Monitor {
        Monitor { output: output.into(), rect: Rect { x, y, w, h }, scale }
    }

    fn close(a: f64, b: f64) { assert!((a - b).abs() < 1e-9, "{a} != {b}"); }

    #[test]
    fn span_regions_follow_side_by_side_outputs_and_negative_origin() {
        let monitors = [monitor("DP-1", -1920.0, 0.0, 1920.0, 1080.0, 1.0), monitor("DP-2", 0.0, 0.0, 1920.0, 1080.0, 1.0)];
        assert_eq!(bounds(&monitors), Rect { x: -1920.0, y: 0.0, w: 3840.0, h: 1080.0 });
        assert_eq!(span_regions(&monitors), [Rect { x: 0.0, y: 0.0, w: 0.5, h: 1.0 }, Rect { x: 0.5, y: 0.0, w: 0.5, h: 1.0 }]);
    }

    #[test]
    fn span_regions_follow_stacked_outputs_with_a_gap() {
        let monitors = [monitor("DP-1", 0.0, -1200.0, 1920.0, 1080.0, 1.0), monitor("DP-2", 0.0, 0.0, 1920.0, 1080.0, 1.0)];
        let regions = span_regions(&monitors);
        close(regions[0].h, 1080.0 / 2280.0);
        close(regions[1].y, 1200.0 / 2280.0);
        close(regions[1].h, regions[0].h);
        assert!(regions[0].y + regions[0].h < regions[1].y);
    }

    #[test]
    fn span_uses_logical_sizes_for_mixed_size_and_fractional_scale_outputs() {
        // A 4K output at 2x and a 2560x1440 output at 1.25x.
        let monitors = [monitor("DP-1", 0.0, 36.0, 1920.0, 1080.0, 2.0), monitor("DP-2", 1920.0, 0.0, 2048.0, 1152.0, 1.25)];
        assert_eq!(monitors[0].pixels(), (3840, 2160));
        assert_eq!(monitors[1].pixels(), (2560, 1440));
        let regions = span_regions(&monitors);
        close(regions[0].w, 1920.0 / 3968.0);
        close(regions[0].y, 36.0 / 1152.0);
        close(regions[0].h, 1080.0 / 1152.0);
        close(regions[1].x, regions[0].w);
        close(regions[1].x + regions[1].w, 1.0);
    }

    #[test]
    fn cover_crop_centers_without_changing_aspect_ratio() {
        assert_eq!(cover_crop((4000.0, 2000.0), (1000.0, 1000.0)), Rect { x: 0.25, y: 0.0, w: 0.5, h: 1.0 });
        assert_eq!(cover_crop((2000.0, 4000.0), (1000.0, 1000.0)), Rect { x: 0.0, y: 0.25, w: 1.0, h: 0.5 });
        for (image, target) in [((3840.0, 2160.0), (3968.0, 1152.0)), ((1080.0, 1920.0), (1920.0, 3000.0))] {
            let r = cover_crop(image, target);
            close(image.0 * r.w / (image.1 * r.h), target.0 / target.1);
        }
    }

    #[test]
    fn cover_crop_composes_with_monitor_regions_to_keep_seams_aligned() {
        let crop = cover_crop((3840.0, 2160.0), (3840.0, 1080.0));
        let left = crop.sub(Rect { x: 0.0, y: 0.0, w: 0.5, h: 1.0 });
        let right = crop.sub(Rect { x: 0.5, y: 0.0, w: 0.5, h: 1.0 });
        close(left.x + left.w, right.x);
        assert_eq!(left, Rect { x: 0.0, y: 0.25, w: 0.5, h: 0.5 });
        assert_eq!(right, Rect { x: 0.5, y: 0.25, w: 0.5, h: 0.5 });
    }

    fn choices() -> toml::Table {
        "wallpaper_layout = 'separate'\nprimary_monitor = 'DP-2'\n[wallpapers]\n'DP-1' = '/one.png'\n'DP-2' = '/two.png'\n'HDMI-A-1' = '/unplugged.png'\n'empty' = ''".parse().unwrap()
    }

    #[test]
    fn separate_lookup_is_connector_specific_and_falls_back_to_global() {
        let prefs = choices();
        assert_eq!(image_for(Layout::Separate, &prefs, "DP-1", "/global.png"), "/one.png");
        assert_eq!(image_for(Layout::Separate, &prefs, "unknown", "/global.png"), "/global.png");
        assert_eq!(image_for(Layout::Separate, &prefs, "empty", "/global.png"), "/global.png");
        for layout in [Layout::Same, Layout::Span] { assert_eq!(image_for(layout, &prefs, "DP-1", "/global.png"), "/global.png"); }
    }

    #[test]
    fn connector_updates_preserve_disconnected_choices_and_explicit_global_choices() {
        let mut prefs = choices();
        let next = connector_choices(&prefs, [("DP-1".into(), "/global.png".into())]);
        assert_eq!(next["HDMI-A-1"].as_str(), Some("/unplugged.png"));
        assert_eq!(next["DP-2"].as_str(), Some("/two.png"));
        prefs.insert("wallpapers".into(), toml::Value::Table(next));
        assert_eq!(image_for(Layout::Separate, &prefs, "DP-1", "/new-global.png"), "/global.png");
        let serialized = toml::to_string(&prefs).unwrap();
        assert_eq!(serialized.parse::<toml::Table>().unwrap(), prefs);
    }

    #[test]
    fn theme_uses_primary_wallpaper_in_separate_and_global_in_other_layouts() {
        let prefs = choices();
        let monitors = [monitor("DP-1", 0.0, 0.0, 1920.0, 1080.0, 1.0), monitor("DP-2", 1920.0, 0.0, 1920.0, 1080.0, 1.0)];
        assert_eq!(theme_source(Layout::Separate, &prefs, &monitors, "/global.png"), "/two.png");
        for layout in [Layout::Same, Layout::Span] { assert_eq!(theme_source(layout, &prefs, &monitors, "/global.png"), "/global.png"); }
        assert_eq!(theme_source(Layout::Separate, &prefs, &monitors[..1], "/global.png"), "/one.png");
        assert_eq!(theme_source(Layout::Separate, &prefs, &[], "/global.png"), "/global.png");
    }

    #[test]
    fn theme_uses_first_monitor_when_primary_is_unset_or_has_no_choice() {
        let mut prefs = choices();
        prefs.remove("primary_monitor");
        let monitors = [monitor("DP-1", 0.0, 0.0, 1920.0, 1080.0, 1.0)];
        assert_eq!(theme_source(Layout::Separate, &prefs, &monitors, "/global.png"), "/one.png");
        let unknown = [monitor("new-output", 0.0, 0.0, 1920.0, 1080.0, 1.0)];
        assert_eq!(theme_source(Layout::Separate, &prefs, &unknown, "/global.png"), "/global.png");
    }

    #[test]
    fn rotation_assigns_different_consecutive_images_and_wraps_when_exhausted() {
        let images = vec!["a".into(), "b".into(), "c".into()];
        assert_eq!(rotation_images(&images, "a", 3), ["b", "c", "a"]);
        assert_eq!(rotation_images(&images, "c", 2), ["a", "b"]);
        assert_eq!(rotation_images(&images, "missing", 4), ["a", "b", "c", "a"]);
        assert!(rotation_images(&[], "a", 3).is_empty());
        assert_eq!(rotation_images(&["only".into()], "only", 3), ["only", "only", "only"]);
        let next = connector_choices(&choices(), [("DP-1".into(), "b".into()), ("DP-2".into(), "c".into())]);
        assert_eq!(next["HDMI-A-1"].as_str(), Some("/unplugged.png"));
    }

    #[test]
    fn luminance_uses_linear_light_and_bgra_channel_order() {
        close(luminance(0, 0, 255), 0.2126);
        close(luminance(255, 0, 0), 0.0722);
        close(luminance(255, 255, 255), 1.0);
        assert!(luminance(128, 128, 128) < 0.22);
    }

    #[test]
    fn luma_samples_each_monitors_region_and_its_top_band() {
        let (w, h) = (24, 12);
        let mut data = vec![0.0; w * h];
        // Upper-right is white, lower-left is white; the other two quadrants are black.
        for y in 0..h { for x in 0..w {
            if (x >= w / 2) == (y < h / 2) { data[y * w + x] = 1.0; }
        } }
        let left = Rect { x: 0.0, y: 0.0, w: 0.5, h: 1.0 };
        let right = Rect { x: 0.5, y: 0.0, w: 0.5, h: 1.0 };
        close(region_luma(&data, w, h, left, 0.5), 0.0);
        close(region_luma(&data, w, h, right, 0.5), 1.0);
        close(region_luma(&data, w, h, left, 1.0), 0.5);
        close(region_luma(&data, w, h, right, 1.0), 0.5);
        let bottom = Rect { x: 0.0, y: 0.5, w: 0.5, h: 0.5 };
        close(region_luma(&data, w, h, bottom, 0.32), 1.0);
    }

    #[test]
    fn layout_defaults_to_same_and_ignores_unknown_values() {
        assert_eq!(Layout::read(&toml::Table::new()), Layout::Same);
        assert_eq!(Layout::read(&"wallpaper_layout = 'invalid'".parse().unwrap()), Layout::Same);
        for layout in [Layout::Same, Layout::Span, Layout::Separate] {
            let prefs = [("wallpaper_layout".into(), toml::Value::String(layout.key().into()))].into_iter().collect();
            assert_eq!(Layout::read(&prefs), layout);
        }
        assert_eq!(span_regions(&[monitor("only", -20.0, 10.0, 1920.0, 1080.0, 1.5)]), [Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }]);
        assert!(span_regions(&[]).is_empty());
    }

    fn test_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("digit-{name}-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()))
    }

    #[test]
    fn span_decodes_one_shared_texture_for_all_outputs() {
        let path = "/no-file-needed.png".to_string();
        let texture: gdk::Texture = gdk::MemoryTexture::new(80, 40, gdk::MemoryFormat::R8g8b8a8, &glib::Bytes::from_owned(vec![255u8; 80 * 40 * 4]), 80 * 4).upcast();
        let monitors = [monitor("left", 0.0, 0.0, 40.0, 40.0, 1.0), monitor("right", 40.0, 0.0, 40.0, 40.0, 1.0)];
        let requests = monitors.iter().zip(span_regions(&monitors)).map(|(m, view)| Render { output: m.output.clone(), path: path.clone(), size: (80, 40), view }).collect();
        let mut decodes = 0;
        let decoded = render_frames(requests, Layout::Span, bounds(&monitors), path.clone(), |_, _| {
            decodes += 1;
            Some(texture.clone())
        });
        assert_eq!(decodes, 1);
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].1.texture.as_ptr(), decoded[1].1.texture.as_ptr());
        close(decoded[0].1.region.x + decoded[0].1.region.w, decoded[1].1.region.x);
        // A cancelled preview can reuse the previous frame even if the file has gone away.
        let restored = decode_frames(vec![Render { output: "left".into(), path: path.clone(), size: (80, 40), view: span_regions(&monitors)[0] }], Layout::Span, bounds(&monitors), path, vec![decoded[0].1.clone()]);
        assert_eq!(restored[0].1.texture.as_ptr(), decoded[0].1.texture.as_ptr());
    }

    #[test]
    fn old_frame_covers_a_resized_output_without_distortion() {
        let texture = gdk::MemoryTexture::new(80, 40, gdk::MemoryFormat::R8g8b8a8, &glib::Bytes::from_owned(vec![255u8; 80 * 40 * 4]), 80 * 4).upcast();
        let frame = Frame { texture, luma: Default::default(), path: "image".into(), region: Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }, span_size: None };
        assert_eq!(frame.region_for(40.0, 40.0), Rect { x: 0.25, y: 0.0, w: 0.5, h: 1.0 });
        let fit = frame.fit(40.0, 40.0, 1.0);
        close(fit.width() as f64 / fit.height() as f64, 2.0);
        close(fit.x() as f64, -20.0);
    }

    #[test]
    fn transition_keeps_the_previous_crop_until_the_new_region_is_warm() {
        let texture = gdk::MemoryTexture::new(2, 1, gdk::MemoryFormat::R8g8b8a8, &glib::Bytes::from_owned(vec![255u8; 8]), 8).upcast();
        let old = Frame { texture, luma: Default::default(), path: "image".into(), region: Rect { x: 0.0, y: 0.0, w: 0.5, h: 1.0 }, span_size: Some((2, 1)) };
        let new = Frame { region: Rect { x: 0.5, ..old.region }, ..old.clone() };
        let mut scene = Scene { new: Some(old.clone()), prime: Some(new.clone()), ..Scene::default() };
        scene.step(100);
        assert_eq!(scene.new.as_ref().unwrap().region, old.region);
        scene.step(200);
        assert_eq!(scene.old.as_ref().unwrap().region, old.region);
        assert_eq!(scene.new.as_ref().unwrap().region, new.region);
        assert_eq!(scene.moving(201), !motion::reduced());
        scene.step(200 + SPAN);
        assert!(scene.old.is_none());
        assert_eq!(scene.new.as_ref().unwrap().region, new.region);
    }

    #[test]
    fn failed_atomic_write_preserves_the_previous_state() {
        let dir = test_dir("state");
        let file = dir.join("wallpaper");
        atomic_write(&file, "original").unwrap();
        std::fs::create_dir(file.with_extension("new")).unwrap();
        assert!(atomic_write(&file, "replacement").is_err());
        assert_eq!(std::fs::read_to_string(file).unwrap(), "original");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn custom_folder_replaces_default_search_locations() {
        let pictures = PathBuf::from("/pictures");
        let defaults = vec![pictures.join("Wallpapers"), pictures.join("wallpapers"), pictures.clone()];
        for value in ["", "  ", "relative/folder", "/bad\nfolder"] {
            assert_eq!(search_directories(value, pictures.clone()), defaults);
        }
        assert_eq!(search_directories(" /custom/wallpapers ", pictures.clone()), vec![PathBuf::from("/custom/wallpapers")]);
        assert_eq!(search_directories("~/Art", pictures), vec![glib::home_dir().join("Art")]);
    }

    #[test]
    fn wallpaper_scan_only_lists_images_in_the_selected_folder() {
        let dir = std::env::temp_dir().join(format!("digit-wallpapers-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(dir.join("nested.png")).unwrap();
        std::fs::write(dir.join("wall.PNG"), "image").unwrap();
        std::fs::write(dir.join("notes.txt"), "text").unwrap();
        std::fs::write(dir.join("nested.png/inside.jpg"), "image").unwrap();
        assert_eq!(listed(&dir), vec![dir.join("wall.PNG").display().to_string()]);
        assert!(listed(&dir.join("missing")).is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
