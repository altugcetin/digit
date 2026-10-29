use gtk::gdk_pixbuf::Pixbuf;
use gtk::subclass::prelude::*;
use gtk::{gdk, gio, glib, graphene, gsk, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kit::motion;
use kit::theme;
use std::cell::{Cell, RefCell};
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
    let _ = std::fs::create_dir_all(theme::state_dir());
    let mut used = used();
    used.retain(|p| p != path);
    used.insert(0, path.to_string());
    used.truncate(60);
    let _ = std::fs::write(theme::state_dir().join("wallpapers-used"), used.join("\n"));
    std::fs::write(state_file(), path).is_ok()
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
    let pictures = glib::user_special_dir(glib::UserDirectory::Pictures)?;
    let image = |p: &std::path::Path| {
        p.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "jpg" | "jpeg" | "png" | "webp" | "avif" | "jxl"))
    };
    for dir in [pictures.join("Wallpapers"), pictures.join("wallpapers"), pictures] {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_file() && image(p)).collect();
        files.sort();
        if let Some(first) = files.first() {
            return Some(first.to_string_lossy().into_owned());
        }
    }
    None
}

#[derive(Default)]
struct Scene {
    old: Option<gdk::Texture>,
    new: Option<gdk::Texture>,
    prime: Option<gdk::Texture>,
    warmed: bool,
    start: i64,
    span: i64,
    blend: bool,
    ticking: bool,
}

impl Scene {
    fn progress(&self, now: i64) -> f64 {
        if self.old.is_none() || self.start == 0 {
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

    fn fit(texture: &gdk::Texture, w: f64, h: f64, scale: f64) -> graphene::Rect {
        let (tw, th) = (texture.width() as f64, texture.height() as f64);
        let k = (w / tw).max(h / th) * scale;
        let (dw, dh) = (tw * k, th * k);
        graphene::Rect::new(((w - dw) / 2.0) as f32, ((h - dh) / 2.0) as f32, dw as f32, dh as f32)
    }

    fn draw(&self, s: &gtk::Snapshot, w: f64, h: f64, now: i64) {
        let bounds = graphene::Rect::new(0.0, 0.0, w as f32, h as f32);
        let t = self.progress(now);
        let p = motion::REVEAL.at(t);
        s.push_clip(&bounds);
        if let (Some(old), true) = (&self.old, t < 1.0) {
            let grow = if self.blend { 0.0 } else { 0.015 * p };
            s.append_scaled_texture(old, gsk::ScalingFilter::Linear, &Self::fit(old, w, h, 1.0 + grow));
        }
        if let Some(new) = &self.new {
            if t >= 1.0 {
                s.append_texture(new, &Self::fit(new, w, h, 1.0));
            } else if self.blend {
                let e = motion::STANDARD.at(t);
                s.push_opacity(e);
                s.append_scaled_texture(new, gsk::ScalingFilter::Linear, &Self::fit(new, w, h, 1.02 - 0.02 * e));
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
                s.append_scaled_texture(new, gsk::ScalingFilter::Linear, &Self::fit(new, w, h, 1.035 - 0.035 * p));
                s.pop();
            }
        }
        if let Some(prime) = &self.prime {
            s.push_opacity(0.004);
            s.append_texture(prime, &bounds);
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
            self.scene.borrow().draw(snapshot, w.width() as f64, w.height() as f64, now);
        }
    }
}

glib::wrapper! {
    pub struct Canvas(ObjectSubclass<imp::Canvas>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Canvas {
    fn show(&self, texture: gdk::Texture) {
        {
            let mut scene = self.imp().scene.borrow_mut();
            if scene.new.is_none() {
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

    fn swap(&self, texture: gdk::Texture, ms: i64) -> Option<gdk::Texture> {
        let before = {
            let mut scene = self.imp().scene.borrow_mut();
            let now = glib::monotonic_time();
            let early = scene.old.is_some() && scene.progress(now) < 0.5;
            scene.prime = None;
            scene.warmed = false;
            let before = scene.new.replace(texture);
            if !early {
                scene.old = before.clone();
            }
            scene.start = now;
            scene.span = ms * 1000;
            scene.blend = true;
            before
        };
        self.tick();
        before
    }

    fn remaining_ms(&self) -> i64 {
        let scene = self.imp().scene.borrow();
        if scene.prime.is_some() {
            return SPAN / 1000;
        }
        ((1.0 - scene.progress(glib::monotonic_time())) * (SPAN / 1000) as f64) as i64
    }
}

struct Screen {
    output: String,
    size: (i32, i32),
    window: gtk::Window,
    canvas: Canvas,
    backup: RefCell<Option<gdk::Texture>>,
    showing: RefCell<String>,
    luma: RefCell<Vec<(usize, u64, f64)>>,
}

#[derive(Default)]
pub struct Wallpapers {
    screens: RefCell<Vec<Screen>>,
    shown: RefCell<String>,
    loading: Cell<u64>,
    ready: RefCell<Option<Box<dyn FnOnce()>>>,
}

impl Wallpapers {
    pub fn place(self: &Rc<Self>, monitors: &[gdk::Monitor]) {
        let names: Vec<String> = monitors.iter().map(|m| m.connector().map(|c| c.to_string()).unwrap_or_default()).collect();
        let mut gone = Vec::new();
        self.screens.borrow_mut().retain(|s| {
            let keep = names.contains(&s.output);
            if !keep {
                gone.push(s.window.clone());
            }
            keep
        });
        for w in gone {
            w.destroy();
        }
        let mut fresh = false;
        for (monitor, name) in monitors.iter().zip(names) {
            if self.screens.borrow().iter().any(|s| s.output == name) {
                continue;
            }
            let g = monitor.geometry();
            let k = monitor.scale_factor().max(1);
            let window = gtk::Window::new();
            window.init_layer_shell();
            window.set_layer(Layer::Background);
            window.set_namespace(Some("digit-wallpaper"));
            for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
                window.set_anchor(edge, true);
            }
            window.set_exclusive_zone(-1);
            window.set_keyboard_mode(KeyboardMode::None);
            window.set_monitor(Some(monitor));
            window.add_css_class("digit-wallpaper");
            let canvas: Canvas = glib::Object::new();
            canvas.set_hexpand(true);
            canvas.set_vexpand(true);
            window.set_child(Some(&canvas));
            window.present();
            self.screens.borrow_mut().push(Screen { output: name, size: (g.width() * k, g.height() * k), window, canvas, backup: RefCell::new(None), showing: RefCell::new(String::new()), luma: RefCell::new(Vec::new()) });
            fresh = true;
        }
        let path = self.shown.borrow().clone();
        if fresh && !path.is_empty() {
            self.load(path);
        }
    }

    pub fn size_of(&self, output: &str) -> Option<(i32, i32)> {
        self.screens.borrow().iter().find(|s| s.output == output).map(|s| s.size)
    }

    pub fn is_empty(&self) -> bool {
        self.shown.borrow().is_empty()
    }

    pub fn when_ready(&self, f: impl FnOnce() + 'static) {
        *self.ready.borrow_mut() = Some(Box::new(f));
    }

    pub fn remaining_ms(&self) -> i64 {
        self.screens.borrow().iter().map(|s| s.canvas.remaining_ms()).max().unwrap_or(0)
    }

    pub fn preview(self: &Rc<Self>, path: String, output: &str) {
        let generation = self.loading.get() + 1;
        self.loading.set(generation);
        let Some(size) = self.screens.borrow().iter().find(|s| s.output == output).map(|s| s.size) else { return };
        let this = self.clone();
        let output = output.to_string();
        glib::spawn_future_local(async move {
            let file = path.clone();
            let Ok(Some(texture)) = gio::spawn_blocking(move || decode(&file, size.0, size.1)).await else { return };
            if this.loading.get() != generation {
                return;
            }
            for screen in this.screens.borrow().iter().filter(|s| s.output == output) {
                let before = screen.canvas.swap(texture.clone(), 340);
                if screen.backup.borrow().is_none() {
                    *screen.backup.borrow_mut() = before;
                }
                *screen.showing.borrow_mut() = path.clone();
            }
        });
    }

    pub fn unpreview(&self, output: &str) {
        self.loading.set(self.loading.get() + 1);
        let shown = self.shown.borrow().clone();
        for screen in self.screens.borrow().iter().filter(|s| s.output == output) {
            if let Some(t) = screen.backup.borrow_mut().take() {
                screen.canvas.swap(t, 420);
            }
            *screen.showing.borrow_mut() = shown.clone();
        }
    }

    pub fn commit(self: &Rc<Self>, path: String, output: &str) {
        for screen in self.screens.borrow().iter().filter(|s| s.output == output) {
            screen.backup.borrow_mut().take();
        }
        if *self.shown.borrow() == path {
            return;
        }
        *self.shown.borrow_mut() = path.clone();
        self.load(path);
    }

    pub fn shown(&self) -> String {
        self.shown.borrow().clone()
    }

    pub fn texture(&self, output: &str) -> Option<gdk::Texture> {
        let screens = self.screens.borrow();
        let screen = screens.iter().find(|s| s.output == output).or(screens.first())?;
        let t = screen.canvas.imp().scene.borrow().new.clone();
        t
    }

    pub fn top_luma(&self, output: &str, band: f64) -> Option<f64> {
        let screens = self.screens.borrow();
        let screen = screens.iter().find(|s| s.output == output)?;
        let texture = screen.canvas.imp().scene.borrow().new.clone()?;
        let key = texture.as_ptr() as usize;
        let bits = band.to_bits();
        if let Some((_, _, v)) = screen.luma.borrow().iter().find(|(k, b, _)| *k == key && *b == bits) {
            return Some(*v);
        }
        let (w, h) = (texture.width() as usize, texture.height() as usize);
        let stride = w * 4;
        let mut data = vec![0u8; stride * h];
        texture.download(&mut data, stride);
        let rows = ((h as f64 * band).ceil() as usize).clamp(1, h);
        let (mut sum, mut n) = (0.0f64, 0usize);
        for y in (0..rows).step_by(2) {
            for x in (0..w).step_by(6) {
                let i = y * stride + x * 4;
                let lin = |c: u8| {
                    let c = c as f64 / 255.0;
                    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
                };
                sum += 0.0722 * lin(data[i]) + 0.7152 * lin(data[i + 1]) + 0.2126 * lin(data[i + 2]);
                n += 1;
            }
        }
        drop(data);
        let v = if n == 0 { 0.0 } else { sum / n as f64 };
        let mut cache = screen.luma.borrow_mut();
        cache.retain(|(k, _, _)| *k == key);
        cache.push((key, bits, v));
        Some(v)
    }

    pub fn set(self: &Rc<Self>, path: String) {
        if *self.shown.borrow() == path {
            return;
        }
        *self.shown.borrow_mut() = path.clone();
        self.load(path);
    }

    fn load(self: &Rc<Self>, path: String) {
        let generation = self.loading.get() + 1;
        self.loading.set(generation);
        let mut sizes: Vec<(i32, i32)> = self.screens.borrow().iter().filter(|s| *s.showing.borrow() != path).map(|s| s.size).collect();
        sizes.sort();
        sizes.dedup();
        if sizes.is_empty() {
            if let Some(ready) = self.ready.borrow_mut().take() {
                glib::timeout_add_local_once(std::time::Duration::from_millis(150), ready);
            }
            return;
        }
        let this = self.clone();
        glib::spawn_future_local(async move {
            let file = path.clone();
            let decoded = gio::spawn_blocking(move || decode_all(&file, &sizes)).await;
            let Ok(decoded) = decoded else { return };
            if this.loading.get() != generation {
                return;
            }
            let mut shown_any = false;
            for screen in this.screens.borrow().iter() {
                if *screen.showing.borrow() == path {
                    continue;
                }
                if let Some((_, Some(texture))) = decoded.iter().find(|(size, _)| *size == screen.size) {
                    screen.canvas.show(texture.clone());
                    *screen.showing.borrow_mut() = path.clone();
                    shown_any = true;
                }
            }
            if !shown_any {
                eprintln!("digit: wallpaper {path} could not be read");
                return;
            }
            if let Some(ready) = this.ready.borrow_mut().take() {
                glib::timeout_add_local_once(std::time::Duration::from_millis(150), ready);
            }
        });
    }

    pub fn destroy(&self) {
        let screens: Vec<Screen> = self.screens.borrow_mut().drain(..).collect();
        for s in screens {
            s.window.destroy();
        }
    }
}
