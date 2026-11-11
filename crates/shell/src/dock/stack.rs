//! The dock's stacks: Recent Files fans out above its icon, and things sent to the trash fly
//! into the Trash icon. Both draw in their own overlay on the dock's monitor.

use super::{Kind, LIFT, Shared, Side, State, kick};
use crate::draw::{self, Face};
use crate::paint::Paint;
use gtk::{cairo, gdk, gio, glib, graphene, pango, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kit::motion;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

/// How long the Trash icon swells after swallowing something, in µs.
pub const GULP: i64 = 320_000;
const FLY: i64 = 560_000;
const TILE: f64 = 52.0;
const STEP: f64 = TILE + 12.0;
const MOST: usize = 12;
const OPEN: f64 = 300.0;
const STAGGER: f64 = 28.0;
const CLOSE: f64 = 170.0;

/// The Trash icon's swell `dt` µs after it swallowed something: 0 → 0.14 → 0.
pub fn gulp(dt: i64) -> f64 {
    if !(0..GULP).contains(&dt) {
        return 0.0;
    }
    0.14 * (std::f64::consts::PI * dt as f64 / GULP as f64).sin()
}

fn ease(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    1.0 - (1.0 - x).powi(3)
}

/// Where item `i` sits on the dock's monitor (monitor-local, logical px): its centre and size.
fn point(s: &State, i: usize, now: i64) -> Option<((f64, f64), f64)> {
    let (a, size) = *s.layout(now).get(i)?;
    let g = s.geo;
    let shown = s.shown.value(now).clamp(0.0, 1.0);
    let off = -((1.0 - shown) * (LIFT + g.body() + 36.0)).round();
    let c = LIFT + off + g.pad() + size / 2.0;
    let along = a + size / 2.0;
    let geo = s.window.monitor()?.geometry();
    let (w, h) = (geo.width() as f64, geo.height() as f64);
    Some((
        match g.side {
            Side::Bottom => (along, h - c),
            Side::Left => (c, along),
            Side::Right => (w - c, along),
        },
        size,
    ))
}

fn paintable(widget: &impl IsA<gtk::Widget>, icon: Option<&gio::Icon>, px: f64) -> Option<gdk::Paintable> {
    let display = widget.display();
    let theme = gtk::IconTheme::for_display(&display);
    let scale = widget.scale_factor().max(1);
    let fallback: gio::Icon = gio::ThemedIcon::from_names(&["text-x-generic", "application-x-generic"]).upcast();
    let icon = icon.unwrap_or(&fallback);
    Some(theme.lookup_by_gicon(icon, px.ceil() as i32, scale, gtk::TextDirection::Ltr, gtk::IconLookupFlags::empty()).upcast())
}

/// Draws `p` inside a `size` square centred on (cx, cy), keeping its aspect (thumbnails aren't square).
fn fit(s: &gtk::Snapshot, p: &gdk::Paintable, cx: f64, cy: f64, size: f64) {
    let (iw, ih) = (p.intrinsic_width() as f64, p.intrinsic_height() as f64);
    let (w, h) = if iw > 0.0 && ih > 0.0 {
        let k = size / iw.max(ih);
        (iw * k, ih * k)
    } else {
        (size, size)
    };
    s.save();
    s.translate(&graphene::Point::new((cx - w / 2.0) as f32, (cy - h / 2.0) as f32));
    p.snapshot(s, w, h);
    s.restore();
}

fn overlay(monitor: Option<&gdk::Monitor>, namespace: &str, keys: bool) -> (gtk::Window, Paint) {
    let window = gtk::Window::new();
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace(Some(namespace));
    for e in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(e, true);
    }
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(if keys { KeyboardMode::Exclusive } else { KeyboardMode::None });
    window.set_monitor(monitor);
    window.add_css_class("digit");
    let paint = Paint::new(1, 1);
    paint.set_hexpand(true);
    paint.set_vexpand(true);
    window.set_child(Some(&paint));
    if !keys {
        window.connect_map(|w| {
            if let Some(surface) = w.surface() {
                surface.set_input_region(Some(&cairo::Region::create()));
            }
        });
    }
    (window, paint)
}

// ---------------------------------------------------------------------------------------------
// Recent Files

struct Entry {
    name: String,
    uri: String,
    icon: Option<gdk::Paintable>,
    /// The label's measured width, kept so drawing and hit tests never lay out text again.
    label: f64,
}

struct Stack {
    dock: Weak<RefCell<State>>,
    window: gtk::Window,
    scrim: gtk::Window,
    paint: Paint,
    side: Side,
    /// Monitor size, and where this window's top-left sits on the monitor.
    screen: (f64, f64),
    origin: (f64, f64),
    /// The Recent icon's centre and size when the stack opened (monitor coordinates).
    from: (f64, f64),
    size: f64,
    entries: Vec<Entry>,
    more: f64,
    hover: Option<usize>,
    opened: i64,
    closing: Option<i64>,
}

thread_local! {
    static STACK: RefCell<Option<Rc<RefCell<Stack>>>> = const { RefCell::new(None) };
    static FX: RefCell<Option<Rc<RefCell<Fx>>>> = const { RefCell::new(None) };
}

/// What a stack slot holds: a file, or the last row that opens Recent in Files.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Slot {
    File(usize),
    More,
}

/// Centre of slot `i` of `n` once fully open, and its tilt in degrees (the bottom fan leans right).
fn slot_at(side: Side, from: (f64, f64), size: f64, screen: (f64, f64), n: usize, i: usize) -> (f64, f64, f64) {
    let (px, py) = from;
    let (n, i) = (n as f64, i as f64);
    match side {
        Side::Bottom => (px + 1.3 * i * i, py - size / 2.0 - 18.0 - TILE / 2.0 - i * STEP, 1.1 * i),
        Side::Left | Side::Right => {
            let x = if side == Side::Left { px + size / 2.0 + 18.0 + TILE / 2.0 } else { px - size / 2.0 - 18.0 - TILE / 2.0 };
            let low = 16.0 + TILE / 2.0;
            let top = (py - (n - 1.0) * STEP / 2.0).clamp(low, (screen.1 - low - (n - 1.0) * STEP).max(low));
            (x, top + i * STEP, 0.0)
        }
    }
}

/// The clickable box of a slot centred at (cx, cy) with a `label` px label (monitor coordinates).
fn slot_box(side: Side, cx: f64, cy: f64, label: f64) -> (f64, f64, f64, f64) {
    let x0 = if side == Side::Left { cx - TILE / 2.0 } else { cx - TILE / 2.0 - 10.0 - label };
    (x0, cy - TILE / 2.0 - 4.0, TILE + 10.0 + label, TILE + 8.0)
}

impl Stack {
    fn count(&self) -> usize {
        self.entries.len() + 1
    }

    fn slot(&self, i: usize) -> (f64, f64, f64) {
        slot_at(self.side, self.from, self.size, self.screen, self.count(), i)
    }

    fn slot_kind(&self, i: usize) -> Slot {
        if i < self.entries.len() { Slot::File(i) } else { Slot::More }
    }

    fn label(&self, i: usize) -> f64 {
        match self.slot_kind(i) {
            Slot::File(k) => self.entries[k].label,
            Slot::More => self.more,
        }
    }

    /// 0 → 1 as slot `i` arrives, and back to 0 while closing.
    fn progress(&self, i: usize, now: i64) -> f64 {
        if motion::reduced() {
            return if self.closing.is_some() { 0.0 } else { 1.0 };
        }
        let open = ease(((now - self.opened) as f64 / 1000.0 - i as f64 * STAGGER) / OPEN);
        match self.closing {
            Some(t) => open * (1.0 - ease((now - t) as f64 / 1000.0 / CLOSE)),
            None => open,
        }
    }

    /// Slot under window-local (x, y).
    fn at(&self, x: f64, y: f64) -> Option<usize> {
        let (x, y) = (x + self.origin.0, y + self.origin.1);
        (0..self.count()).find(|&i| {
            let (cx, cy, _) = self.slot(i);
            let (bx, by, bw, bh) = slot_box(self.side, cx, cy, self.label(i));
            x >= bx && x <= bx + bw && y >= by && y <= by + bh
        })
    }

    fn draw(&self, s: &gtk::Snapshot) {
        let Some(st) = self.dock.upgrade() else { return };
        let Ok(dock) = st.try_borrow() else { return };
        let now = glib::monotonic_time();
        let pal = dock.app.palette_at(now);
        let cfg = &dock.app.cfg;
        s.save();
        s.translate(&graphene::Point::new(-self.origin.0 as f32, -self.origin.1 as f32));
        for i in 0..self.count() {
            let p = self.progress(i, now);
            if p <= 0.001 {
                continue;
            }
            let (tx, ty, tilt) = self.slot(i);
            let (cx, cy) = (self.from.0 + (tx - self.from.0) * p, self.from.1 + (ty - self.from.1) * p);
            let scale = 0.3 + 0.7 * p;
            let hot = self.hover == Some(i);
            s.push_opacity(p);
            s.save();
            s.translate(&graphene::Point::new(cx as f32, cy as f32));
            s.rotate((tilt * p) as f32);
            s.scale(scale as f32, scale as f32);
            let (text, icon) = match self.slot_kind(i) {
                Slot::File(k) => (self.entries[k].name.as_str(), self.entries[k].icon.as_ref()),
                Slot::More => ("Open in Files", None),
            };
            // The label sits on the side of the tile away from the screen edge.
            let lw = self.label(i);
            let lx = if self.side == Side::Left { TILE / 2.0 + 10.0 } else { -TILE / 2.0 - 10.0 - lw };
            draw::fill(s, &draw::round(lx, -14.0, lw, 28.0, 14.0), if hot { pal.accent } else { pal.body.alpha(1.0).mix(pal.ink_main, 0.9) });
            draw::text(s, &self.paint, cfg, text, 13.0, Face::Text(520), if hot { pal.on_accent } else { pal.ink_main }, lx + 12.0, 0.0, lw - 24.0, pango::Alignment::Left);
            match (self.slot_kind(i), icon) {
                (Slot::File(_), Some(icon)) => fit(s, icon, 0.0, 0.0, TILE),
                _ => {
                    draw::fill(s, &draw::round(-TILE / 2.0, -TILE / 2.0, TILE, TILE, TILE * 0.28), pal.raised);
                    draw::text(s, &self.paint, cfg, "\u{2197}", 22.0, Face::Text(600), pal.ink_main, -TILE / 2.0, 0.0, TILE, pango::Alignment::Center);
                }
            }
            s.restore();
            s.pop();
        }
        s.restore();
    }
}

/// Opens the Recent Files stack above (or beside) dock item `i`; a second click closes it.
pub fn open(st: &Shared, i: usize) {
    if STACK.with(|s| s.borrow().is_some()) {
        close_stack();
        return;
    }
    super::close_menu();
    let now = glib::monotonic_time();
    let (monitor, from, side) = {
        let s = st.borrow();
        let Some((from, size)) = point(&s, i, now) else { return };
        (s.window.monitor(), (from, size), s.geo.side)
    };
    let weak = Rc::downgrade(st);
    glib::spawn_future_local(async move {
        // gio::Icon can't cross threads: carry the thumbnail path, or the icon's serialized form.
        let recent = gio::spawn_blocking(|| {
            kit::recent::list(MOST)
                .into_iter()
                .map(|r| {
                    let thumb = r.icon.downcast_ref::<gio::FileIcon>().and_then(|f| f.file().path());
                    let icon = if thumb.is_some() { None } else { gio::prelude::IconExt::to_string(&r.icon).map(|s| s.to_string()) };
                    (r.path, r.name, icon, thumb)
                })
                .collect::<Vec<_>>()
        })
        .await
        .unwrap_or_default();
        let Some(st) = weak.upgrade() else { return };
        show(&st, monitor, from, side, recent);
    });
}

type Found = (std::path::PathBuf, String, Option<String>, Option<std::path::PathBuf>);

fn show(st: &Shared, monitor: Option<gdk::Monitor>, from: ((f64, f64), f64), side: Side, recent: Vec<Found>) {
    let Some(monitor) = monitor else { return };
    let geo = monitor.geometry();
    let screen = (geo.width() as f64, geo.height() as f64);
    let (from, size) = from;
    // On the bottom dock the fan must fit between the icon and the top of the screen.
    let room = if side == Side::Bottom { ((from.1 - size / 2.0 - 40.0) / STEP).floor().max(1.0) as usize } else { (screen.1 / STEP).floor().max(1.0) as usize };
    let recent: Vec<Found> = recent.into_iter().take(room.saturating_sub(1).min(MOST)).collect();
    let app = st.borrow().app.clone();
    let measure = Paint::new(1, 1);
    let width = |text: &str| (draw::width(&measure, &app.cfg, text, 13.0, Face::Text(520)).ceil() + 24.0).min(260.0);
    let mut entries: Vec<Entry> = Vec::with_capacity(recent.len());
    let mut thumbs = Vec::new();
    for (k, (path, name, icon, thumb)) in recent.into_iter().enumerate() {
        // Thumbnails are decoded off the main thread below; until then the file type's icon shows.
        let fallback = icon.and_then(|i| gio::Icon::for_string(&i).ok()).or_else(|| {
            let (kind, _) = gio::content_type_guess(Some(path.as_path()), None);
            Some(gio::content_type_get_icon(&kind))
        });
        if let Some(t) = thumb {
            thumbs.push((k, t));
        }
        entries.push(Entry { icon: paintable(&measure, fallback.as_ref(), TILE), uri: gio::File::for_path(&path).uri().to_string(), label: width(&name), name });
    }
    let more = width("Open in Files");
    // The window covers just the fan and the icon it grows from, so each frame repaints only that.
    let n = entries.len() + 1;
    let (mut x0, mut y0, mut x1, mut y1) = (from.0 - size / 2.0, from.1 - size / 2.0, from.0 + size / 2.0, from.1 + size / 2.0);
    for i in 0..n {
        let (cx, cy, _) = slot_at(side, from, size, screen, n, i);
        let label = if i < entries.len() { entries[i].label } else { more };
        let (bx, by, bw, bh) = slot_box(side, cx, cy, label);
        (x0, y0, x1, y1) = (x0.min(bx), y0.min(by), x1.max(bx + bw), y1.max(by + bh));
    }
    let pad = 28.0;
    let (x0, y0) = ((x0 - pad).max(0.0).floor(), (y0 - pad).max(0.0).floor());
    let (x1, y1) = ((x1 + pad).min(screen.0).ceil(), (y1 + pad).min(screen.1).ceil());
    let (w, h) = ((x1 - x0).max(1.0) as i32, (y1 - y0).max(1.0) as i32);
    let window = gtk::Window::new();
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace(Some("digit-dock-stack"));
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Left, true);
    window.set_margin(Edge::Top, y0 as i32);
    window.set_margin(Edge::Left, x0 as i32);
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(KeyboardMode::Exclusive);
    window.set_monitor(Some(&monitor));
    window.add_css_class("digit");
    window.set_default_size(w, h);
    let paint = Paint::new(w, h);
    window.set_child(Some(&paint));
    let scrim = crate::paint::scrim(Some(&monitor), close_stack);
    let stack = Rc::new(RefCell::new(Stack {
        dock: Rc::downgrade(st),
        window: window.clone(),
        scrim,
        paint: paint.clone(),
        side,
        screen,
        origin: (x0, y0),
        from,
        size,
        entries,
        more,
        hover: None,
        opened: glib::monotonic_time(),
        closing: None,
    }));
    let weak = Rc::downgrade(&stack);
    paint.on_draw(move |s| {
        if let Some(m) = weak.upgrade() {
            if let Ok(m) = m.try_borrow() {
                m.draw(s);
            }
        }
    });
    let motion = gtk::EventControllerMotion::new();
    let weak = Rc::downgrade(&stack);
    motion.connect_motion(move |_, x, y| {
        let Some(m) = weak.upgrade() else { return };
        let mut m = m.borrow_mut();
        let at = if m.closing.is_none() { m.at(x, y) } else { None };
        if at != m.hover {
            m.hover = at;
            m.paint.set_cursor_from_name(at.map(|_| "pointer"));
            m.paint.queue_draw();
        }
    });
    paint.add_controller(motion);
    let click = gtk::GestureClick::new();
    let weak = Rc::downgrade(&stack);
    click.connect_released(move |_, _, x, y| {
        let Some(m) = weak.upgrade() else { return };
        let picked = {
            let m = m.borrow();
            m.at(x, y).map(|i| match m.slot_kind(i) {
                Slot::File(k) => m.entries[k].uri.clone(),
                Slot::More => String::new(),
            })
        };
        match picked {
            Some(uri) if uri.is_empty() => {
                let _ = gio::Subprocess::newv(&[std::ffi::OsStr::new("digit-files"), std::ffi::OsStr::new("recent:///")], gio::SubprocessFlags::NONE);
            }
            Some(uri) => {
                gio::AppInfo::launch_default_for_uri_async(&uri, None::<&gio::AppLaunchContext>, None::<&gio::Cancellable>, |r| {
                    if let Err(e) = r {
                        eprintln!("digit: recent: {e}");
                    }
                });
            }
            None => {}
        }
        close_stack();
    });
    paint.add_controller(click);
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(|_, key, _, _| {
        if key == gdk::Key::Escape {
            close_stack();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(keys);
    STACK.with(|slot| *slot.borrow_mut() = Some(stack.clone()));
    {
        let mut s = st.borrow_mut();
        s.menu = true;
        s.hover = None;
        s.tip.set(0.0);
    }
    super::settle(st);
    window.present();
    let px = (TILE * window.scale_factor().max(1) as f64).ceil() as i32;
    for (k, thumb) in thumbs {
        let weak = Rc::downgrade(&stack);
        glib::spawn_future_local(async move {
            let Some(texture) = crate::sni::file_icon(thumb.to_string_lossy().into_owned(), px).await else { return };
            let Some(m) = weak.upgrade() else { return };
            let mut m = m.borrow_mut();
            if let Some(e) = m.entries.get_mut(k) {
                e.icon = Some(texture.upcast());
            }
            m.paint.queue_draw();
        });
    }
    let weak = Rc::downgrade(&stack);
    paint.add_tick_callback(move |w, _| {
        let Some(m) = weak.upgrade() else { return glib::ControlFlow::Break };
        w.queue_draw();
        let now = glib::monotonic_time();
        let m = m.borrow();
        let last = m.count().saturating_sub(1);
        let settling = match m.closing {
            Some(t) => now - t < (CLOSE * 1000.0) as i64,
            None => (now - m.opened) as f64 / 1000.0 < OPEN + STAGGER * last as f64,
        };
        if settling && !motion::reduced() { glib::ControlFlow::Continue } else { glib::ControlFlow::Break }
    });
}

/// Folds the stack back into its icon, then removes it.
pub fn close_stack() {
    let Some(m) = STACK.with(|s| s.borrow_mut().take()) else { return };
    let (window, scrim, paint, dock) = {
        let mut mm = m.borrow_mut();
        mm.closing = Some(glib::monotonic_time());
        mm.hover = None;
        (mm.window.clone(), mm.scrim.clone(), mm.paint.clone(), mm.dock.upgrade())
    };
    scrim.destroy();
    if let Some(surface) = window.surface() {
        surface.set_input_region(Some(&cairo::Region::create()));
    }
    if let Some(st) = dock {
        st.borrow_mut().menu = false;
        super::settle(&st);
    }
    if motion::reduced() {
        window.destroy();
        return;
    }
    paint.add_tick_callback(move |w, _| {
        w.queue_draw();
        let done = m.borrow().closing.is_some_and(|t| glib::monotonic_time() - t >= (CLOSE * 1000.0) as i64);
        if done {
            window.destroy();
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

// ---------------------------------------------------------------------------------------------
// Into the Trash

struct Flight {
    from: (f64, f64),
    size: f64,
    icon: Option<gdk::Paintable>,
    start: i64,
    landed: bool,
}

struct Fx {
    dock: Weak<RefCell<State>>,
    window: gtk::Window,
    paint: Paint,
    flights: Vec<Flight>,
}

/// Where something flying from `from` to `to` is at `t` (0..1): a gentle arc that rises first.
fn arc(from: (f64, f64), to: (f64, f64), t: f64) -> (f64, f64) {
    let d = ((to.0 - from.0).powi(2) + (to.1 - from.1).powi(2)).sqrt();
    let lift = (d * 0.35).min(220.0);
    let ctrl = ((from.0 + to.0) / 2.0, from.1.min(to.1) - lift);
    let u = 1.0 - t;
    (u * u * from.0 + 2.0 * u * t * ctrl.0 + t * t * to.0, u * u * from.1 + 2.0 * u * t * ctrl.1 + t * t * to.1)
}

/// Sends an icon from `at` (global logical coordinates) into the dock's Trash. False when the
/// dock shows no Trash, so the caller knows nothing visible happened.
pub fn fly(st: &Shared, at: (f64, f64), size: f64, icon: Option<gio::Icon>) -> bool {
    let now = glib::monotonic_time();
    let (monitor, origin, hidden) = {
        let s = st.borrow();
        if !s.items.iter().any(|i| i.kind == Kind::Trash) {
            return false;
        }
        let hidden = s.shown.target() < 0.5;
        if hidden && !s.trash_reveal {
            return false;
        }
        let Some(monitor) = s.window.monitor() else { return false };
        let geo = monitor.geometry();
        (monitor, (geo.x() as f64, geo.y() as f64, geo.width() as f64, geo.height() as f64), hidden)
    };
    if hidden {
        // The dock slides in for the flight and hides again once the Trash has swallowed it.
        {
            let mut s = st.borrow_mut();
            s.reveal = true;
            s.trash_shown = true;
        }
        super::settle(st);
    }
    let from = (at.0 - origin.0, at.1 - origin.1);
    let inside = from.0 >= 0.0 && from.1 >= 0.0 && from.0 <= origin.2 && from.1 <= origin.3;
    if !inside || motion::reduced() {
        // Nothing to fly across on this monitor: the Trash still takes it in.
        st.borrow_mut().gulp = Some(now);
        kick(st);
        hide_later(st);
        return true;
    }
    let fx = FX.with(|f| f.borrow().clone()).unwrap_or_else(|| {
        let (window, paint) = overlay(Some(&monitor), "digit-dock-fx", false);
        let fx = Rc::new(RefCell::new(Fx { dock: Rc::downgrade(st), window: window.clone(), paint: paint.clone(), flights: Vec::new() }));
        let weak = Rc::downgrade(&fx);
        paint.on_draw(move |s| {
            if let Some(fx) = weak.upgrade() {
                if let Ok(fx) = fx.try_borrow() {
                    fx.draw(s);
                }
            }
        });
        window.present();
        FX.with(|f| *f.borrow_mut() = Some(fx.clone()));
        let weak = Rc::downgrade(&fx);
        paint.add_tick_callback(move |w, _| {
            let Some(fx) = weak.upgrade() else { return glib::ControlFlow::Break };
            w.queue_draw();
            step(&fx)
        });
        fx
    });
    let mut f = fx.borrow_mut();
    // Several items leave together: let them follow each other in.
    let queued = f.flights.iter().filter(|q| now - q.start < 60_000).count() as i64;
    let px = size.clamp(24.0, 128.0);
    let paintable = paintable(&f.paint, icon.as_ref(), px);
    f.flights.push(Flight { from, size: px, icon: paintable, start: now + queued * 70_000, landed: false });
    true
}

fn step(fx: &Rc<RefCell<Fx>>) -> glib::ControlFlow {
    let now = glib::monotonic_time();
    let (dock, landed, done, window) = {
        let mut f = fx.borrow_mut();
        let mut landed = false;
        for q in f.flights.iter_mut() {
            if !q.landed && now - q.start >= FLY {
                q.landed = true;
                landed = true;
            }
        }
        f.flights.retain(|q| now - q.start < FLY + 16_000);
        (f.dock.upgrade(), landed, f.flights.is_empty(), f.window.clone())
    };
    if landed && let Some(st) = &dock {
        st.borrow_mut().gulp = Some(now);
        kick(st);
    }
    if done {
        FX.with(|f| f.borrow_mut().take());
        window.destroy();
        if let Some(st) = &dock {
            hide_later(st);
        }
        return glib::ControlFlow::Break;
    }
    glib::ControlFlow::Continue
}

/// A dock that slid in only for the Trash hides again once the swell has played, unless the
/// pointer came to it meanwhile.
fn hide_later(st: &Shared) {
    if !st.borrow().trash_shown {
        return;
    }
    let weak = Rc::downgrade(st);
    glib::timeout_add_local_once(std::time::Duration::from_millis((GULP / 1000) as u64 + 450), move || {
        let Some(st) = weak.upgrade() else { return };
        let busy = FX.with(|f| f.borrow().is_some());
        {
            let mut s = st.borrow_mut();
            if busy || !s.trash_shown {
                return;
            }
            s.trash_shown = false;
            if s.inside {
                return;
            }
            s.reveal = false;
        }
        super::settle(&st);
    });
}

impl Fx {
    fn draw(&self, s: &gtk::Snapshot) {
        let Some(st) = self.dock.upgrade() else { return };
        let Ok(dock) = st.try_borrow() else { return };
        let now = glib::monotonic_time();
        let Some(index) = dock.items.iter().position(|i| i.kind == Kind::Trash) else { return };
        let Some((to, trash)) = point(&dock, index, now) else { return };
        for q in &self.flights {
            let t = ((now - q.start) as f64 / FLY as f64).clamp(0.0, 1.0);
            if now < q.start {
                continue;
            }
            let e = t * t * (3.0 - 2.0 * t);
            let (x, y) = arc(q.from, to, e);
            let size = q.size + (trash * 0.45 - q.size) * e;
            let alpha = if t < 0.82 { 1.0 } else { 1.0 - (t - 0.82) / 0.18 };
            s.push_opacity(alpha.clamp(0.0, 1.0));
            match &q.icon {
                Some(p) => fit(s, p, x, y, size),
                None => draw::fill(s, &draw::round(x - size / 2.0, y - size / 2.0, size, size, size * 0.2), dock.app.palette_at(now).raised),
            }
            s.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trash_swells_once_and_settles() {
        assert_eq!(gulp(-1), 0.0);
        assert_eq!(gulp(0), 0.0);
        assert!((gulp(GULP / 2) - 0.14).abs() < 1e-9);
        assert_eq!(gulp(GULP), 0.0);
    }

    #[test]
    fn flights_start_and_end_on_their_points_and_rise_between() {
        let (from, to) = ((100.0, 500.0), (900.0, 1040.0));
        let start = arc(from, to, 0.0);
        let end = arc(from, to, 1.0);
        assert!((start.0 - from.0).abs() < 1e-9 && (start.1 - from.1).abs() < 1e-9);
        assert!((end.0 - to.0).abs() < 1e-9 && (end.1 - to.1).abs() < 1e-9);
        assert!(arc(from, to, 0.25).1 < from.1, "it rises before it falls");
    }
}
