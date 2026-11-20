use crate::app::App;
use crate::compositor;
use crate::draw::{self, Face};
use crate::paint::Paint;
use gtk::{cairo, gdk, gio, glib, graphene, gsk, pango, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kit::motion::{self, Tween};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

mod stack;

const LIFT: f64 = 8.0;
const BOUNCE: i64 = 1_400_000;
const MENU_W: f64 = 236.0;
const ROW: f64 = 34.0;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Side {
    Bottom,
    Left,
    Right,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    On,
    Autohide,
    Smart,
}

#[derive(Clone, Copy, PartialEq)]
struct Geo {
    side: Side,
    base: f64,
    mag: f64,
}

impl Geo {
    fn from(p: &crate::config::DockPrefs) -> Self {
        let side = match p.side.as_str() {
            "left" => Side::Left,
            "right" => Side::Right,
            _ => Side::Bottom,
        };
        Self { side, base: p.size.clamp(28.0, 96.0).round(), mag: p.magnify.clamp(1.0, 2.0) }
    }

    fn pad(&self) -> f64 {
        (self.base * 0.18).round().max(5.0)
    }

    fn gap(&self) -> f64 {
        (self.base * 0.16).round().max(4.0)
    }

    fn body(&self) -> f64 {
        self.base + 2.0 * self.pad()
    }

    fn reserve(&self) -> f64 {
        LIFT + self.body()
    }

    fn spread(&self) -> f64 {
        self.base * 1.9
    }

    fn reach(&self) -> f64 {
        LIFT + self.pad() + self.base * self.mag + 20.0
    }

    fn depth(&self) -> f64 {
        match self.side {
            Side::Bottom => self.reach() + 46.0,
            _ => self.reach() + 250.0,
        }
    }

    fn radius(&self) -> f64 {
        (self.body() * 0.32).round()
    }

    fn icon_px(&self) -> i32 {
        (self.base * self.mag).ceil() as i32
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Window {
    id: u64,
    stamp: u64,
    focused: bool,
    minimized: bool,
}

fn target_window(windows: &[Window]) -> Option<Window> {
    let mut ids = windows.to_vec();
    ids.sort_by_key(|w| w.id);
    if let Some(p) = ids.iter().position(|w| w.focused) {
        return Some(ids[(p + 1) % ids.len()]);
    }
    ids.iter().filter(|w| !w.minimized).max_by_key(|w| w.stamp)
        .or_else(|| ids.iter().max_by_key(|w| w.stamp)).copied()
}

/// Apps, then the stacks macOS keeps after a second separator.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    App,
    Recent,
    Trash,
    /// The Apps button at the start, which opens the app grid.
    Apps,
}

const APPS: &str = "digit:apps";

const RECENT: &str = "digit:recent";
const TRASH: &str = "digit:trash";

struct Item {
    kind: Kind,
    key: String,
    name: String,
    entry: Option<usize>,
    pinned: bool,
    windows: Vec<Window>,
    badge: usize,
}

impl Item {
    fn focused(&self) -> bool {
        self.windows.iter().any(|w| w.focused)
    }
}

struct Drag {
    key: String,
    origin: (f64, f64),
    at: (f64, f64),
    moving: bool,
}

struct State {
    app: Rc<App>,
    window: gtk::Window,
    keep: gtk::Window,
    paint: Paint,
    geo: Geo,
    mode: Mode,
    output: String,
    running: bool,
    badges: bool,
    /// Whether the Trash and Recent files stacks are shown, and whether the trash holds anything.
    trash: bool,
    recent: bool,
    apps: bool,
    /// Raised over the app grid while it's open, and kept shown: the layer it had before.
    lifted: Option<Layer>,
    /// When a drag of an app last ended, so the release that ended it doesn't also count as a
    /// click on whatever is under it.
    dragged: i64,
    /// An app dragged in from the app grid, held in place among the kept apps until it's let go
    /// (see `incoming`).
    incoming: Option<String>,
    /// Slide a hidden dock in while something flies to the Trash; `trash_shown` marks that it did.
    trash_reveal: bool,
    trash_shown: bool,
    trash_full: bool,
    trash_watch: Option<gio::FileMonitor>,
    /// When the Trash last swallowed something that flew into it.
    gulp: Option<i64>,
    style: crate::art::Style,
    art: HashMap<String, gdk::Texture>,
    items: Vec<Item>,
    pins: Vec<String>,
    /// Order of the running, unpinned apps after the user rearranged them.
    order: Vec<String>,
    icons: HashMap<String, Option<gdk::Paintable>>,
    icon_key: Option<crate::icons::DockKey>,
    away: HashMap<String, Tween>,
    slide: HashMap<String, Tween>,
    appear: HashMap<String, Tween>,
    pointer: Option<(f64, f64)>,
    magnify: Tween,
    shown: Tween,
    tip: Tween,
    hover: Option<usize>,
    bounce: Vec<(String, i64)>,
    inside: bool,
    reveal: bool,
    leave: u64,
    drag: Option<Drag>,
    menu: bool,
    ticking: bool,
    region: Option<(i32, i32, i32, i32)>,
    reserved: i32,
    triggers: Vec<gtk::Window>,
    panel_triggers: Vec<gtk::Window>,
    edge: Option<gtk::Window>,
}

type Shared = Rc<RefCell<State>>;

thread_local! {
    static ONE: RefCell<Option<Shared>> = const { RefCell::new(None) };
    static PREFER: RefCell<Option<String>> = const { RefCell::new(None) };
    static MENU: RefCell<Option<Rc<RefCell<Menu>>>> = const { RefCell::new(None) };
}

fn current() -> Option<Shared> {
    ONE.with(|one| one.borrow().clone())
}

fn pins_path() -> std::path::PathBuf {
    kit::theme::state_dir().join("dock")
}

fn load_pins(app: &App) -> Vec<String> {
    if let Ok(text) = std::fs::read_to_string(pins_path()) {
        return text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect();
    }
    let list = app.apps.list();
    let mut ranked: Vec<(i32, String)> = list.iter().map(|e| (app.apps.boost(&e.id), e.id.clone())).filter(|(b, _)| *b > 0).collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0));
    ranked.into_iter().take(6).map(|(_, id)| id).collect()
}

fn save_pins(pins: &[String]) {
    let _ = std::fs::create_dir_all(kit::theme::state_dir());
    let path = pins_path();
    let tmp = path.with_extension("new");
    if std::fs::write(&tmp, pins.join("\n") + "\n").is_ok() {
        let _ = std::fs::rename(tmp, path);
    }
}

/// The screen `output` when it is plugged in, else the main one.
fn monitor_for(app: &App, output: Option<&str>) -> Option<gdk::Monitor> {
    output.and_then(kit::screens::by_name).or_else(|| app.primary())
}

/// How far the always-shown dock keeps windows off the left, right and bottom edges of
/// `output`, 0 where it doesn't, so desktop widgets in those corners can sit clear of it.
pub fn room(output: &str) -> [i32; 3] {
    let Some(st) = current() else { return [0; 3] };
    let Ok(s) = st.try_borrow() else { return [0; 3] };
    if s.reserved <= 0 || s.output != output {
        return [0; 3];
    }
    match s.geo.side {
        Side::Left => [s.reserved, 0, 0],
        Side::Right => [0, s.reserved, 0],
        Side::Bottom => [0, 0, s.reserved],
    }
}

/// A point in the dock's own terms (along its edge, and out from it) on a screen `screen` big.
fn to_screen(side: Side, (a, c): (f64, f64), screen: (f64, f64)) -> (f64, f64) {
    match side {
        Side::Bottom => (a, screen.1 - c),
        Side::Left => (c, a),
        Side::Right => (screen.0 - c, a),
    }
}

fn from_screen(side: Side, (x, y): (f64, f64), screen: (f64, f64)) -> (f64, f64) {
    match side {
        Side::Bottom => (x, screen.1 - y),
        Side::Left => (y, x),
        Side::Right => (y, screen.0 - x),
    }
}

/// The dock as the app grid sees it on a screen `screen` big: its side ("bottom", "left" or
/// "right"), how far in from that edge it reaches, and its apps' ids and icon centres.
pub fn seen(screen: (f64, f64)) -> Option<(&'static str, f64, Vec<(String, (f64, f64))>)> {
    let st = current()?;
    let s = st.try_borrow().ok()?;
    let now = glib::monotonic_time();
    let c = LIFT + s.geo.body() / 2.0;
    let apps = s.items.iter().zip(s.layout(now)).filter(|(i, _)| i.kind == Kind::App).map(|(i, (a, size))| (i.key.clone(), to_screen(s.geo.side, (a + size / 2.0, c), screen))).collect();
    let side = match s.geo.side {
        Side::Bottom => "bottom",
        Side::Left => "left",
        Side::Right => "right",
    };
    Some((side, s.geo.reach(), apps))
}

/// The apps kept in the dock.
pub fn pinned() -> Vec<String> {
    current().and_then(|st| st.try_borrow().ok().map(|s| s.pins.clone())).unwrap_or_default()
}

/// Whether a point on a screen `screen` big is over the dock.
pub fn over(at: (f64, f64), screen: (f64, f64)) -> bool {
    let Some(st) = current() else { return false };
    let Ok(s) = st.try_borrow() else { return false };
    from_screen(s.geo.side, at, screen).1 <= s.geo.reach()
}

/// Keeps app `id` in the dock, among the kept apps where the screen point `at` falls.
pub fn pin_at(id: &str, at: (f64, f64), screen: (f64, f64)) {
    let Some(st) = current() else { return };
    {
        let mut s = st.borrow_mut();
        if s.pins.iter().any(|p| p == id) {
            return;
        }
        let a = from_screen(s.geo.side, at, screen).0;
        let now = glib::monotonic_time();
        let before = s.items.iter().zip(s.layout(now)).filter(|(i, (x, size))| i.pinned && x + size / 2.0 < a).count();
        let k = before.min(s.pins.len());
        s.pins.insert(k, id.to_string());
        save_pins(&s.pins);
    }
    rebuild(&st);
    settle(&st);
}

/// Keeps app `id` in the dock, after the other kept apps.
pub fn pin(id: &str) {
    let Some(st) = current() else { return };
    {
        let mut s = st.borrow_mut();
        if s.pins.iter().any(|p| p == id) {
            return;
        }
        s.pins.push(id.to_string());
        save_pins(&s.pins);
    }
    rebuild(&st);
    settle(&st);
}

/// Lets app `id` go from the dock.
pub fn unpin(id: &str) {
    let Some(st) = current() else { return };
    {
        let mut s = st.borrow_mut();
        s.pins.retain(|p| p != id);
        save_pins(&s.pins);
    }
    rebuild(&st);
    settle(&st);
}

/// A point, or a size, on a screen.
pub type Point = (f64, f64);

/// An app dragged from the app grid over the dock (`Some`: its id, and where it is on a screen
/// `screen` big): the dock makes room for it where it would land and draws it under the pointer,
/// as it does its own while they're dragged. `None` once it leaves the dock: the room closes.
pub fn incoming(app: Option<(&str, Point, Point)>) {
    let Some(st) = current() else { return };
    let changed = {
        let Ok(mut s) = st.try_borrow_mut() else { return };
        match app {
            Some((id, at, screen)) => {
                if s.incoming.as_deref() != Some(id) && s.pins.iter().any(|p| p == id) {
                    return;
                }
                let p = from_screen(s.geo.side, at, screen);
                let now = glib::monotonic_time();
                let before = s.items.iter().zip(s.layout(now)).filter(|(i, (x, size))| i.pinned && i.key != id && x + size / 2.0 < p.0).count();
                let had = s.pins.iter().position(|k| k == id);
                s.drag = Some(Drag { key: id.to_string(), origin: p, at: p, moving: true });
                if had == Some(before) {
                    false
                } else {
                    s.pins.retain(|k| k != id);
                    let k = before.min(s.pins.len());
                    s.pins.insert(k, id.to_string());
                    s.incoming = Some(id.to_string());
                    true
                }
            }
            None => match s.incoming.take() {
                Some(id) => {
                    s.pins.retain(|k| *k != id);
                    s.drag = None;
                    true
                }
                None => false,
            },
        }
    };
    if changed {
        rebuild(&st);
    }
    settle(&st);
}

/// The app dragged in from the app grid was let go over the dock: it stays where it was shown.
/// Returns whether one was there to keep.
pub fn keep_incoming() -> bool {
    let Some(st) = current() else { return false };
    {
        let mut s = st.borrow_mut();
        if s.incoming.take().is_none() {
            return false;
        }
        s.drag = None;
        s.dragged = glib::monotonic_time();
        save_pins(&s.pins);
    }
    rebuild(&st);
    settle(&st);
    true
}

/// Shows the dock over the app grid while it's open (`on`), so apps can be dragged between them.
pub fn lift(on: bool) {
    let Some(st) = current() else { return };
    {
        let mut s = st.borrow_mut();
        if on && s.lifted.is_none() {
            s.lifted = Some(s.window.layer());
            s.window.set_layer(Layer::Overlay);
        } else if !on && let Some(layer) = s.lifted.take() {
            s.window.set_layer(layer);
        }
    }
    settle(&st);
}

pub fn primary_changed(app: &Rc<App>) {
    PREFER.with(|p| p.borrow_mut().take());
    apply(app);
}

fn teardown(st: &Shared) {
    close_menu();
    let (keep, window, paint, triggers, edge) = {
        let mut s = st.borrow_mut();
        let mut triggers = std::mem::take(&mut s.triggers);
        triggers.append(&mut s.panel_triggers);
        (s.keep.clone(), s.window.clone(), s.paint.clone(), triggers, s.edge.take())
    };
    for t in triggers.into_iter().chain(edge) {
        t.destroy();
    }
    keep.destroy();
    window.destroy();
    draw::forget(&paint);
}

fn trigger(app: &Rc<App>, monitor: &gdk::Monitor, side: Side) -> gtk::Window {
    let w = gtk::Window::new();
    w.init_layer_shell();
    w.set_layer(Layer::Overlay);
    w.set_namespace(Some("digit-dock-edge"));
    let edges: [Edge; 3] = match side {
        Side::Bottom => [Edge::Bottom, Edge::Left, Edge::Right],
        Side::Left => [Edge::Left, Edge::Top, Edge::Bottom],
        Side::Right => [Edge::Right, Edge::Top, Edge::Bottom],
    };
    for e in edges {
        w.set_anchor(e, true);
    }
    w.set_exclusive_zone(-1);
    w.set_keyboard_mode(KeyboardMode::None);
    w.set_monitor(Some(monitor));
    w.add_css_class("digit");
    let (pw, ph) = if side == Side::Bottom { (1, 2) } else { (2, 1) };
    w.set_default_size(pw, ph);
    let paint = Paint::new(pw, ph);
    paint.present();
    if side == Side::Bottom {
        paint.set_hexpand(true);
    } else {
        paint.set_vexpand(true);
    }
    w.set_child(Some(&paint));
    let motion = gtk::EventControllerMotion::new();
    let inside = Rc::new(std::cell::Cell::new(0u64));
    let output = monitor.connector().map(|c| c.to_string()).unwrap_or_default();
    let weak = Rc::downgrade(app);
    let flag = inside.clone();
    let probe = w.clone();
    let corner_app = weak.clone();
    let corner_out = output.clone();
    let check = move |x: f64, y: f64| {
        use kit::corners::Corner;
        let corner = match side {
            Side::Bottom if x <= 3.0 => Some(Corner::BottomLeft),
            Side::Bottom if x >= probe.width() as f64 - 3.0 => Some(Corner::BottomRight),
            Side::Left if y <= 3.0 => Some(Corner::TopLeft),
            Side::Left if y >= probe.height() as f64 - 3.0 => Some(Corner::BottomLeft),
            Side::Right if y <= 3.0 => Some(Corner::TopRight),
            Side::Right if y >= probe.height() as f64 - 3.0 => Some(Corner::BottomRight),
            _ => None,
        };
        corner.is_some_and(|corner| corner_app.upgrade().is_some_and(|app| crate::panel_corner::activate(&app, &corner_out, corner)))
    };
    let near = check.clone();
    motion.connect_motion(move |_, x, y| {
        near(x, y);
    });
    motion.connect_enter(move |_, x, y| {
        if check(x, y) {
            return;
        }
        let generation = flag.get() + 1;
        flag.set(generation);
        let (weak, flag, output) = (weak.clone(), flag.clone(), output.clone());
        glib::timeout_add_local_once(Duration::from_millis(180), move || {
            if flag.get() != generation {
                return;
            }
            let Some(app) = weak.upgrade() else { return };
            if !app.edges_live(&output) {
                return;
            }
            let same = current().is_some_and(|st| st.try_borrow().is_ok_and(|s| s.output == output));
            if !same {
                PREFER.with(|p| *p.borrow_mut() = Some(output.clone()));
                apply(&app);
            }
            if let Some(st) = current() {
                st.borrow_mut().reveal = true;
                settle(&st);
                let weak = Rc::downgrade(&st);
                glib::timeout_add_local_once(Duration::from_millis(1400), move || {
                    let Some(st) = weak.upgrade() else { return };
                    let idle = st.try_borrow().is_ok_and(|s| !s.inside && s.drag.is_none() && !s.menu);
                    if idle {
                        st.borrow_mut().reveal = false;
                        settle(&st);
                    }
                });
            }
        });
    });
    let flag = inside;
    motion.connect_leave(move |_| flag.set(flag.get() + 1));
    paint.add_controller(motion);
    // A file dragged to the edge brings a hidden dock out, so it can be dropped on the Trash.
    let dnd = gtk::DropControllerMotion::new();
    let weak = Rc::downgrade(app);
    let edge_output = monitor.connector().map(|c| c.to_string()).unwrap_or_default();
    dnd.connect_enter(move |_, _, _| {
        let Some(app) = weak.upgrade() else { return };
        if !app.edges_live(&edge_output) {
            return;
        }
        if let Some(st) = current().filter(|st| st.try_borrow().is_ok_and(|s| s.output == edge_output && s.trash)) {
            st.borrow_mut().reveal = true;
            settle(&st);
        }
    });
    paint.add_controller(dnd);
    w.present();
    w
}

/// While files are dragged over the dock: COPY when the pointer is on the Trash, else nothing,
/// so the source never deletes anything itself.
fn trash_action(st: &Shared, x: f64, y: f64) -> gdk::DragAction {
    let s = st.borrow();
    let (a, _) = s.local(x, y);
    let on = s.index_at(a, glib::monotonic_time()).and_then(|i| s.items.get(i)).is_some_and(|i| i.kind == Kind::Trash);
    if on { gdk::DragAction::COPY } else { gdk::DragAction::empty() }
}

pub fn apply(app: &Rc<App>) {
    let prefs = app.prefs.borrow().dock.clone();
    let mode = match prefs.mode.as_str() {
        "off" => None,
        "autohide" => Some(Mode::Autohide),
        "smart" => Some(Mode::Smart),
        _ => Some(Mode::On),
    };
    let geo = Geo::from(&prefs);
    let prefer = if prefs.output.is_none() { PREFER.with(|p| p.borrow().clone()) } else { None };
    let monitor = monitor_for(app, prefs.output.as_deref().or(prefer.as_deref()));
    let output = monitor.as_ref().and_then(|m| m.connector()).map(|c| c.to_string()).unwrap_or_default();
    let old = current();
    let Some(mode) = mode else {
        if let Some(st) = old {
            teardown(&st);
            ONE.with(|one| one.borrow_mut().take());
            App::trim();
        }
        crate::panel_corner::refresh(app);
        return;
    };
    let layer = if mode == Mode::On { Layer::Top } else { Layer::Overlay };
    let reuse = old.filter(|st| {
        let (side, old_output, window) = {
            let s = st.borrow();
            (s.geo.side, s.output.clone(), s.window.clone())
        };
        side == geo.side && old_output == output && window.layer() == layer
    });
    let st = match reuse {
        Some(st) => st,
        None => {
            if let Some(st) = current() {
                teardown(&st);
            }
            let st = build(app, geo, monitor.as_ref(), output.clone(), layer);
            if let Some(m) = monitor.as_ref() {
                let edge = trigger(app, m, geo.side);
                edge.set_visible(false);
                st.borrow_mut().edge = Some(edge);
            }
            if prefs.output.is_none() {
                let others: Vec<gdk::Monitor> = kit::screens::all().into_iter().filter(|m| m.connector().as_deref() != Some(output.as_str())).collect();
                let triggers: Vec<gtk::Window> = others.iter().map(|m| trigger(app, m, geo.side)).collect();
                st.borrow_mut().triggers = triggers;
            }
            ONE.with(|one| *one.borrow_mut() = Some(st.clone()));
            st
        }
    };
    let resize = {
        let s = st.borrow();
        (s.geo != geo).then(|| s.paint.clone())
    };
    {
        let mut s = st.borrow_mut();
        s.geo = geo;
        s.mode = mode;
        s.running = prefs.running;
        s.badges = prefs.badges;
        s.trash = prefs.trash;
        s.recent = prefs.recent;
        s.apps = prefs.apps;
        s.trash_reveal = prefs.trash_reveal;
        s.style = crate::art::Style::parse(&prefs.icons);
        s.icons.clear();
        s.art.clear();
        crate::art::forget();
    }
    if let Some(paint) = resize {
        paint.set_size_request(if geo.side == Side::Bottom { 1 } else { geo.depth() as i32 }, if geo.side == Side::Bottom { geo.depth() as i32 } else { 1 });
    }
    // Notches own triggers on configured shell outputs. The dock historically also
    // offered Control Center on its other outputs; keep that scope at any corner.
    let triggers = std::mem::take(&mut st.borrow_mut().panel_triggers);
    for trigger in triggers { trigger.destroy(); }
    let extra: Vec<gdk::Monitor> = kit::screens::all().into_iter().filter(|m| {
        let name = kit::screens::name(m);
        !app.cfg.outputs.is_empty() && !app.cfg.outputs.contains(&name) && (prefs.output.is_none() || name == output)
    }).collect();
    let triggers = extra.iter().flat_map(|m| crate::panel_corner::triggers(app, m)).collect();
    st.borrow_mut().panel_triggers = triggers;
    watch_trash(&st);
    rebuild(&st);
    settle(&st);
    crate::panel_corner::refresh(app);
}

pub fn changed() {
    if let Some(st) = current() {
        rebuild(&st);
        kick(&st);
    }
}

pub fn settle_all() {
    if let Some(st) = current() {
        settle(&st);
    }
}

pub fn badges(app: &App) {
    let Some(st) = current() else { return };
    let Ok(mut s) = st.try_borrow_mut() else { return };
    if !s.badges {
        return;
    }
    let counts = counts(app);
    let mut dirty = false;
    for it in s.items.iter_mut() {
        let n = counts.get(&it.name).copied().unwrap_or(0);
        dirty |= it.badge != n;
        it.badge = n;
    }
    if dirty {
        s.paint.queue_draw();
    }
}

fn counts(app: &App) -> HashMap<String, usize> {
    let seen = app.seen.borrow();
    let mut map = HashMap::new();
    for n in app.history.borrow().iter() {
        if seen.get(&n.app.to_lowercase()).is_some_and(|t| *t >= n.at) {
            continue;
        }
        *map.entry(n.app.clone()).or_insert(0) += 1;
    }
    map
}

/// Something just went to the trash from `at` (global logical coordinates, the item's centre)
/// at `size` px: when the dock shows its Trash, the item flies into it. Returns whether it did.
/// Digit's accent folder (as Files and the desktop draw it) as an icon for a trash flight.
pub fn folder_icon(glyph: &str, size: f64) -> Option<gio::Icon> {
    let px = (stack::flight_size(size) * 2.0).ceil() as i32;
    kit::folder::texture(&kit::style::ink().accent, glyph, px).map(|t| t.upcast())
}

pub fn fly_to_trash(app: &Rc<App>, at: (f64, f64), size: f64, icon: Option<gio::Icon>) -> bool {
    let _ = app;
    current().is_some_and(|st| stack::fly(&st, at, size, icon))
}

/// Follow the trash so its dock icon shows whether it holds anything.
fn watch_trash(st: &Shared) {
    let on = st.borrow().trash;
    if !on {
        st.borrow_mut().trash_watch = None;
        return;
    }
    if st.borrow().trash_watch.is_some() {
        return;
    }
    let weak = Rc::downgrade(st);
    let monitor = kit::trash::watch(move |full| {
        let Some(st) = weak.upgrade() else { return };
        let changed = {
            let mut s = st.borrow_mut();
            let changed = s.trash_full != full;
            s.trash_full = full;
            if changed {
                s.icons.remove(TRASH);
                s.art.remove(TRASH);
            }
            changed
        };
        if changed {
            rebuild(&st);
        }
    });
    st.borrow_mut().trash_watch = monitor;
    let weak = Rc::downgrade(st);
    glib::spawn_future_local(async move {
        let full = gio::spawn_blocking(kit::trash::is_full).await.unwrap_or(false);
        let Some(st) = weak.upgrade() else { return };
        if st.borrow().trash_full != full {
            {
                let mut s = st.borrow_mut();
                s.trash_full = full;
                s.icons.remove(TRASH);
                s.art.remove(TRASH);
            }
            rebuild(&st);
        }
    });
}

pub fn command(app: &Rc<App>, verb: &str) -> bool {
    let mode = app.prefs.borrow().dock.mode.clone();
    let remembered = kit::theme::state_dir().join("dock-mode");
    let visible = mode != "off";
    let want = match verb {
        "show" => true,
        "hide" => false,
        _ => !visible,
    };
    if want == visible {
        return true;
    }
    let next = if want {
        std::fs::read_to_string(&remembered).ok().map(|s| s.trim().to_string()).filter(|m| matches!(m.as_str(), "on" | "autohide" | "smart")).unwrap_or_else(|| "on".into())
    } else {
        let _ = std::fs::create_dir_all(kit::theme::state_dir());
        let _ = std::fs::write(&remembered, &mode);
        "off".into()
    };
    close_menu();
    if let Err(e) = crate::config::set("dock_mode", toml::Value::String(next)) {
        eprintln!("digit: dock: {e}");
        return false;
    }
    app.apply_prefs(crate::config::Prefs::from(&crate::config::load()));
    true
}

fn build(app: &Rc<App>, geo: Geo, monitor: Option<&gdk::Monitor>, output: String, layer: Layer) -> Shared {
    let edges: [Edge; 3] = match geo.side {
        Side::Bottom => [Edge::Bottom, Edge::Left, Edge::Right],
        Side::Left => [Edge::Left, Edge::Top, Edge::Bottom],
        Side::Right => [Edge::Right, Edge::Top, Edge::Bottom],
    };
    let keep = gtk::Window::new();
    keep.init_layer_shell();
    keep.set_layer(Layer::Bottom);
    keep.set_namespace(Some("digit-dock-space"));
    for e in edges {
        keep.set_anchor(e, true);
    }
    keep.set_keyboard_mode(KeyboardMode::None);
    keep.set_exclusive_zone(0);
    keep.set_monitor(monitor);
    keep.add_css_class("digit");
    keep.set_default_size(1, 1);
    keep.set_child(Some(&Paint::new(1, 1)));
    keep.connect_map(|w| {
        if let Some(surface) = w.surface() {
            surface.set_input_region(Some(&cairo::Region::create()));
        }
    });
    keep.present();

    let window = gtk::Window::new();
    window.init_layer_shell();
    window.set_layer(layer);
    window.set_namespace(Some("digit-dock"));
    for e in edges {
        window.set_anchor(e, true);
    }
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(KeyboardMode::None);
    window.set_monitor(monitor);
    window.add_css_class("digit");
    let depth = geo.depth() as i32;
    let paint = if geo.side == Side::Bottom {
        window.set_default_size(-1, depth);
        let p = Paint::new(1, depth);
        p.set_hexpand(true);
        p
    } else {
        window.set_default_size(depth, -1);
        let p = Paint::new(depth, 1);
        p.set_vexpand(true);
        p
    };
    window.set_child(Some(&paint));
    let st = Rc::new(RefCell::new(State {
        app: app.clone(),
        window: window.clone(),
        keep,
        paint: paint.clone(),
        geo,
        mode: Mode::On,
        output,
        running: true,
        badges: true,
        trash: false,
        recent: false,
        apps: false,
        lifted: None,
        dragged: 0,
        incoming: None,
        trash_reveal: true,
        trash_shown: false,
        trash_full: false,
        trash_watch: None,
        gulp: None,
        style: crate::art::Style::Light,
        art: HashMap::new(),
        items: Vec::new(),
        pins: load_pins(app),
        icons: HashMap::new(),
        icon_key: None,
        away: HashMap::new(),
        slide: HashMap::new(),
        appear: HashMap::new(),
        pointer: None,
        magnify: Tween::still(0.0),
        shown: Tween::still(0.0),
        tip: Tween::still(0.0),
        hover: None,
        bounce: Vec::new(),
        inside: false,
        reveal: false,
        leave: 0,
        drag: None,
        menu: false,
        ticking: false,
        region: None,
        order: Vec::new(),
        reserved: -1,
        triggers: Vec::new(),
        panel_triggers: Vec::new(),
        edge: None,
    }));
    wire(&st);
    window.present();
    st
}

fn wire(st: &Shared) {
    let (paint, window) = {
        let s = st.borrow();
        (s.paint.clone(), s.window.clone())
    };
    let weak = Rc::downgrade(st);
    paint.on_draw(move |s| {
        if let Some(st) = weak.upgrade() {
            if let Ok(st) = st.try_borrow() {
                st.draw(s);
            }
        }
    });
    let motion = gtk::EventControllerMotion::new();
    let weak = Rc::downgrade(st);
    motion.connect_enter(move |_, x, y| {
        if let Some(st) = weak.upgrade() {
            enter(&st, x, y);
        }
    });
    let weak = Rc::downgrade(st);
    motion.connect_motion(move |_, x, y| {
        if let Some(st) = weak.upgrade() {
            moved(&st, x, y);
        }
    });
    let weak = Rc::downgrade(st);
    motion.connect_leave(move |_| {
        if let Some(st) = weak.upgrade() {
            left(&st);
        }
    });
    paint.add_controller(motion);
    let click = gtk::GestureClick::new();
    click.set_button(0);
    let weak = Rc::downgrade(st);
    click.connect_released(move |g, _, x, y| {
        let Some(st) = weak.upgrade() else { return };
        let at = {
            let s = st.borrow();
            let now = glib::monotonic_time();
            let p = s.local(x, y);
            // The release that ends a drag isn't a click, nor is one away from the dock (an app
            // let go over the app grid).
            let dragging = s.drag.as_ref().is_some_and(|d| d.moving) || now - s.dragged < 300_000;
            if dragging || !s.over_body(p, now) { None } else { s.index_at(p.0, now) }
        };
        let button = g.current_button();
        if button != 3 {
            close_menu();
        }
        if let Some(i) = at {
            match button {
                2 => launch(&st, i),
                3 => open_menu(&st, i),
                _ => activate(&st, i),
            }
        }
    });
    paint.add_controller(click);
    let drag = gtk::GestureDrag::new();
    let weak = Rc::downgrade(st);
    drag.connect_drag_begin(move |_, x, y| {
        let Some(st) = weak.upgrade() else { return };
        let mut s = st.borrow_mut();
        let p = s.local(x, y);
        let key = s.index_at(p.0, glib::monotonic_time()).and_then(|i| s.items.get(i)).filter(|it| it.entry.is_some()).map(|it| it.key.clone());
        s.drag = key.map(|key| Drag { key, origin: p, at: p, moving: false });
    });
    let weak = Rc::downgrade(st);
    drag.connect_drag_update(move |g, dx, dy| {
        let (Some(st), Some((x, y))) = (weak.upgrade(), g.start_point()) else { return };
        drag_to(&st, x + dx, y + dy);
    });
    let weak = Rc::downgrade(st);
    drag.connect_drag_end(move |_, _, _| {
        if let Some(st) = weak.upgrade() {
            drop_drag(&st);
        }
    });
    paint.add_controller(drag);
    // Hyprland offers only MOVE before negotiation, so accept both and answer COPY: the dock
    // trashes the files itself and the source must never delete them. Same as the notch's target.
    let drop = gtk::DropTargetAsync::new(None, gdk::DragAction::COPY | gdk::DragAction::MOVE);
    drop.connect_accept(|_, drop| {
        let formats = drop.formats();
        formats.contains_type(gdk::FileList::static_type()) || formats.contain_mime_type("text/uri-list")
    });
    let weak = Rc::downgrade(st);
    drop.connect_drag_enter(move |_, _, x, y| {
        let Some(st) = weak.upgrade() else { return gdk::DragAction::empty() };
        enter(&st, x, y);
        trash_action(&st, x, y)
    });
    let weak = Rc::downgrade(st);
    drop.connect_drag_motion(move |_, _, x, y| {
        let Some(st) = weak.upgrade() else { return gdk::DragAction::empty() };
        moved(&st, x, y);
        trash_action(&st, x, y)
    });
    let weak = Rc::downgrade(st);
    drop.connect_drag_leave(move |_, _| {
        if let Some(st) = weak.upgrade() {
            left(&st);
        }
    });
    let weak = Rc::downgrade(st);
    drop.connect_drop(move |_, drop, x, y| {
        let Some(st) = weak.upgrade() else { return false };
        if trash_action(&st, x, y).is_empty() {
            return false;
        }
        let weak = Rc::downgrade(&st);
        let drop = drop.clone();
        glib::spawn_future_local(async move {
            let value = drop.read_value_future(gdk::FileList::static_type(), glib::Priority::DEFAULT).await;
            let paths: Vec<std::path::PathBuf> = value.ok().and_then(|v| v.get::<gdk::FileList>().ok()).map(|l| l.files().iter().filter_map(|f| f.path()).collect()).unwrap_or_default();
            let ok = !paths.is_empty();
            if ok {
                kit::trash::put(paths);
                if let Some(st) = weak.upgrade() {
                    st.borrow_mut().gulp = Some(glib::monotonic_time());
                    kick(&st);
                }
            }
            drop.finish(if ok { gdk::DragAction::COPY } else { gdk::DragAction::empty() });
        });
        true
    });
    paint.add_controller(drop);
    let weak = Rc::downgrade(st);
    window.connect_map(move |_| {
        if let Some(st) = weak.upgrade() {
            st.borrow_mut().region = None;
            region(&st);
            let weak = Rc::downgrade(&st);
            glib::idle_add_local_once(move || {
                if let Some(st) = weak.upgrade() {
                    paint_art(&st);
                    let paint = st.borrow().paint.clone();
                    paint.queue_draw();
                }
            });
        }
    });
}

fn rebuild(st: &Shared) {
    let s = st.borrow();
    let now = glib::monotonic_time();
    let before = s.centers(now);
    let app = s.app.clone();
    let list = app.apps.list();
    let running = app.running.borrow().clone();
    let badges = if s.badges { counts(&app) } else { HashMap::new() };
    let mut items: Vec<Item> = s
        .pins
        .iter()
        .filter_map(|key| {
            let idx = list.iter().position(|e| e.id == *key)?;
            Some(Item { kind: Kind::App, key: key.clone(), name: list[idx].name.clone(), entry: Some(idx), pinned: true, windows: Vec::new(), badge: 0 })
        })
        .collect();
    for r in running.iter().filter(|r| !r.app_id.is_empty()) {
        let idx = app.apps.find(&r.app_id);
        let key = idx.map(|i| list[i].id.clone()).unwrap_or_else(|| r.app_id.clone());
        match items.iter_mut().find(|it| it.key == key) {
            Some(it) => it.windows.push(Window { id: r.id, stamp: r.stamp, focused: r.focused, minimized: r.minimized }),
            None if s.running => items.push(Item {
                kind: Kind::App,
                name: idx.map(|i| list[i].name.clone()).unwrap_or_else(|| crate::apps::pretty(&r.app_id)),
                key,
                entry: idx,
                pinned: false,
                windows: vec![Window { id: r.id, stamp: r.stamp, focused: r.focused, minimized: r.minimized }],
                badge: 0,
            }),
            None => {}
        }
    }
    let pinned = items.iter().filter(|i| i.pinned).count();
    items[pinned..].sort_by_key(|it| s.order.iter().position(|k| *k == it.key).unwrap_or(usize::MAX));
    for it in items.iter_mut() {
        it.badge = badges.get(&it.name).copied().unwrap_or(0);
    }
    if s.apps {
        items.insert(0, Item { kind: Kind::Apps, key: APPS.into(), name: "Apps".into(), entry: None, pinned: false, windows: Vec::new(), badge: 0 });
    }
    for (on, kind, key, name) in [(s.recent, Kind::Recent, RECENT, "Recent Files"), (s.trash, Kind::Trash, TRASH, "Trash")] {
        if on {
            items.push(Item { kind, key: key.into(), name: name.into(), entry: None, pinned: false, windows: Vec::new(), badge: 0 });
        }
    }
    // Resolve through GTK only after releasing the state borrow: lookups and
    // rendering can synchronously emit signals back into the dock.
    let window = s.window.clone();
    let style = s.style;
    let px = s.geo.icon_px();
    let full = s.trash_full;
    let wanted: Vec<_> = items.iter().map(|it| {
        let icon = match it.kind {
            Kind::App => it.entry.and_then(|i| list.get(i)?.icon.clone()),
            Kind::Recent => Some(gio::ThemedIcon::from_names(&["folder-recent", "document-open-recent", "folder"]).upcast()),
            Kind::Trash => Some(gio::ThemedIcon::from_names(if full { &["user-trash-full", "user-trash"][..] } else { &["user-trash"][..] }).upcast()),
            Kind::Apps => None,
        };
        (it.key.clone(), icon)
    }).collect();
    drop(s);
    let scale = window.scale_factor().max(1);
    let active = gtk::Settings::default().and_then(|s| s.gtk_icon_theme_name()).unwrap_or_else(|| "Adwaita".into());
    let key = crate::icons::DockKey::new(style, &active, px, scale);
    {
        let mut s = st.borrow_mut();
        if s.icon_key.as_ref() != Some(&key) {
            s.icons.clear();
            s.art.clear();
            s.icon_key = Some(key.clone());
        }
    }
    let missing: Vec<_> = {
        let s = st.borrow();
        wanted.into_iter().filter(|(id, _)| !s.icons.contains_key(id)).collect()
    };
    if let Some(display) = gdk::Display::default() {
        let theme = crate::icons::dock_theme(&display, &key);
        for (id, icon) in missing {
            let icon = crate::icons::dock_icon(&theme, icon.as_ref(), &id, &key);
            st.borrow_mut().icons.insert(id, icon);
        }
    }
    let mut s = st.borrow_mut();
    let live: Vec<String> = items.iter().filter(|i| !i.windows.is_empty()).map(|i| i.key.clone()).collect();
    s.bounce.retain(|(k, t)| !live.contains(k) && now - t < 4 * BOUNCE);
    if s.icons.len() > 64 {
        let keep: Vec<String> = items.iter().map(|i| i.key.clone()).collect();
        s.icons.retain(|k, _| keep.contains(k));
    }
    if s.hover.is_some_and(|h| h >= items.len()) {
        s.hover = None;
    }
    s.away.retain(|k, _| items.iter().any(|i| i.key == *k));
    for item in &items {
        let away = if !item.windows.is_empty() && item.windows.iter().all(|w| w.minimized) { 1.0 } else { 0.0 };
        let tween = s.away.entry(item.key.clone()).or_insert_with(|| Tween::still(0.0));
        tween.go(away, now, 200, motion::STANDARD);
    }
    s.items = items;
    let after = s.centers(now);
    let dragged = s.drag.as_ref().filter(|d| d.moving).map(|d| d.key.clone());
    for (k, c) in &after {
        if Some(k) == dragged.as_ref() {
            continue;
        }
        match before.get(k) {
            Some(b) if (b - c).abs() > 0.5 => {
                let carried = s.slide.get(k).map_or(0.0, |t| t.value(now));
                let mut t = Tween::still(b - c + carried);
                t.go(0.0, now, 300, motion::EMPHASIZED_DECEL);
                s.slide.insert(k.clone(), t);
            }
            None if !before.is_empty() => {
                let mut t = Tween::still(0.0);
                t.go(1.0, now, 340, motion::EMPHASIZED_DECEL);
                s.appear.insert(k.clone(), t);
            }
            _ => {}
        }
    }
    s.slide.retain(|k, t| after.contains_key(k) && !t.settled(now));
    s.appear.retain(|k, t| after.contains_key(k) && !t.settled(now));
    drop(s);
    paint_art(st);
    region(st);
    let paint = st.borrow().paint.clone();
    paint.queue_draw();
    kick(st);
}

fn paint_art(st: &Shared) {
    let (paint, app, px, style, wanted) = {
        let s = st.borrow();
        let wanted: Vec<_> = s.items.iter().map(|i| (i.key.clone(), i.name.clone(), s.icons.get(&i.key).cloned().flatten())).collect();
        (s.paint.clone(), s.app.clone(), s.geo.icon_px() as f64, s.style, wanted)
    };
    if paint.native().and_then(|n| n.renderer()).is_none() {
        return;
    }
    let accent = app.palette.get().accent;
    let mut art = HashMap::new();
    for (key, name, icon) in wanted {
        if key == APPS {
            // Drawn by digit: a tile of colored squares, like Launchpad's.
            if let Some(t) = kit::folder::grid_texture((px * paint.scale_factor().max(1) as f64).round() as i32) {
                art.insert(key, t);
            }
            continue;
        }
        if let Some(t) = crate::art::texture(&paint, &key, icon.as_ref(), &name, px, style, accent, &app.cfg.font) {
            art.insert(key, t);
        }
    }
    st.borrow_mut().art = art;
}

fn wants(s: &State) -> bool {
    if s.reveal || s.menu || s.drag.is_some() || s.lifted.is_some() {
        return true;
    }
    match s.mode {
        Mode::On => true,
        Mode::Autohide => false,
        Mode::Smart => !s.app.spaces.borrow().iter().any(|sp| sp.output == s.output && sp.active && sp.filled),
    }
}

fn settle(st: &Shared) {
    {
        let mut s = st.borrow_mut();
        let now = glib::monotonic_time();
        if wants(&s) {
            s.shown.go(1.0, now, 360, motion::EMPHASIZED_DECEL);
        } else {
            s.shown.go(0.0, now, 260, motion::STANDARD);
        }
        let hidden = s.shown.target() < 0.5;
        if let Some(edge) = s.edge.clone() {
            if edge.is_visible() != hidden {
                glib::idle_add_local_once(move || edge.set_visible(hidden));
            }
        }
        let zone = if s.mode == Mode::On { s.geo.reserve() as i32 } else { 0 };
        if zone != s.reserved {
            s.keep.set_exclusive_zone(zone);
            s.reserved = zone;
            // Desktop widgets in the dock's corners move clear of it.
            glib::idle_add_local_once(crate::deskcards::dock_moved);
        }
    }
    region(st);
    kick(st);
    let weak = Rc::downgrade(st);
    glib::timeout_add_local_once(Duration::from_millis(420), move || {
        if let Some(st) = weak.upgrade() {
            if let Ok(s) = st.try_borrow() {
                s.paint.queue_draw();
            }
            region(&st);
        }
    });
}

fn enter(st: &Shared, x: f64, y: f64) {
    let (shown, generation) = {
        let Ok(mut s) = st.try_borrow_mut() else { return };
        s.inside = true;
        s.leave += 1;
        (s.shown.target() > 0.5, s.leave)
    };
    if shown {
        st.borrow_mut().reveal = true;
        settle(st);
    } else {
        let weak = Rc::downgrade(st);
        glib::timeout_add_local_once(Duration::from_millis(140), move || {
            let Some(st) = weak.upgrade() else { return };
            let go = st.try_borrow().is_ok_and(|s| s.inside && s.leave == generation);
            if go {
                st.borrow_mut().reveal = true;
                settle(&st);
            }
        });
    }
    moved(st, x, y);
}

fn moved(st: &Shared, x: f64, y: f64) {
    {
        let Ok(mut s) = st.try_borrow_mut() else { return };
        let p = s.local(x, y);
        s.pointer = Some(p);
        let now = glib::monotonic_time();
        let over = s.over_body(p, now) && s.geo.mag > 1.0;
        s.magnify.go(if over { 1.0 } else { 0.0 }, now, if over { 220 } else { 300 }, motion::STANDARD);
        let at = if s.over_body(p, now) && s.drag.as_ref().is_none_or(|d| !d.moving) { s.index_at(p.0, now) } else { None };
        if at != s.hover {
            s.hover = at;
            if at.is_some() {
                s.tip.set(0.0);
                s.tip.go(1.0, now, 160, motion::STANDARD);
            } else {
                s.tip.go(0.0, now, 120, motion::FAST_EFFECTS);
            }
        }
        s.paint.set_cursor_from_name(at.map(|_| "pointer"));
        s.paint.queue_draw();
    }
    region(st);
    kick(st);
}

fn left(st: &Shared) {
    let generation = {
        let Ok(mut s) = st.try_borrow_mut() else { return };
        s.inside = false;
        s.hover = None;
        // Keep the last pointer position: the magnification shrinks around it as `magnify` fades.
        s.leave += 1;
        let now = glib::monotonic_time();
        s.magnify.go(0.0, now, 300, motion::STANDARD);
        s.tip.go(0.0, now, 120, motion::FAST_EFFECTS);
        s.leave
    };
    region(st);
    kick(st);
    let weak = Rc::downgrade(st);
    glib::timeout_add_local_once(Duration::from_millis(520), move || {
        let Some(st) = weak.upgrade() else { return };
        let same = st.try_borrow().is_ok_and(|s| s.leave == generation && !s.inside);
        if same {
            st.borrow_mut().reveal = false;
            settle(&st);
        }
    });
}

fn drag_to(st: &Shared, x: f64, y: f64) {
    let reorder = {
        let mut s = st.borrow_mut();
        let p = s.local(x, y);
        // Out over the open app grid, it draws the app under the pointer.
        if let Some(d) = s.drag.as_ref().filter(|d| d.moving) {
            let side = s.geo.side;
            let out = p.1 > s.geo.reach() + 24.0;
            crate::appgrid::ghost(out.then(|| (d.key.clone(), Box::new(move |screen| to_screen(side, p, screen)) as crate::appgrid::Place)));
        }
        let Some(d) = s.drag.as_mut() else { return };
        d.at = p;
        if !d.moving {
            if (p.0 - d.origin.0).hypot(p.1 - d.origin.1) < 8.0 {
                return;
            }
            d.moving = true;
            s.hover = None;
            s.tip.set(0.0);
        }
        let key = s.drag.as_ref().map(|d| d.key.clone()).unwrap_or_default();
        let now = glib::monotonic_time();
        let target = s.index_at(p.0, now).or_else(|| {
            let l = s.layout(now);
            let first = l.first().map_or(0.0, |f| f.0);
            Some(if p.0 < first { 0 } else { l.len().saturating_sub(1) })
        });
        // The Apps button stays first: places count from after it.
        let lead = s.items.iter().take_while(|i| i.kind == Kind::Apps).count();
        let target = target.map(|t| t.saturating_sub(lead));
        let mut order: Vec<String> = s.items.iter().filter(|i| i.kind != Kind::Apps).map(|i| i.key.clone()).collect();
        match (target, order.iter().position(|k| *k == key)) {
            (Some(t), Some(c)) if t != c => {
                let k = order.remove(c);
                let t = t.min(order.len());
                order.insert(t, k);
                // Landing among the other pinned apps pins it, among the unpinned ones unpins it, and on the border keeps it as it was.
                let others = s.items.iter().filter(|i| i.pinned && i.key != key).count();
                let item = s.items.iter().find(|i| i.key == key);
                let was = item.is_some_and(|i| i.pinned);
                let pin = match t.cmp(&others) {
                    std::cmp::Ordering::Less => true,
                    std::cmp::Ordering::Greater => was && item.is_some_and(|i| i.windows.is_empty()),
                    std::cmp::Ordering::Equal => was,
                };
                let pinned = |k: &String| if *k == key { pin } else { s.items.iter().any(|i| i.key == *k && i.pinned) };
                let stale: Vec<String> = s.pins.iter().filter(|k| !order.contains(k)).cloned().collect();
                let pins: Vec<String> = order.iter().filter(|k| pinned(k)).cloned().chain(stale).collect();
                s.order = order.iter().filter(|k| !pinned(k)).cloned().collect();
                s.pins = pins;
                true
            }
            _ => false,
        }
    };
    if reorder {
        rebuild(st);
    }
    st.borrow().paint.queue_draw();
    kick(st);
}

fn drop_drag(st: &Shared) {
    let note = {
        let mut s = st.borrow_mut();
        let Some(d) = s.drag.take() else { return };
        if !d.moving {
            return;
        }
        s.dragged = glib::monotonic_time();
        let far = d.at.1 > s.geo.reach() + 24.0;
        let name = s.items.iter().find(|i| i.key == d.key).map(|i| i.name.clone()).unwrap_or_default();
        // Over the open app grid, the app goes into it where it's let go, quietly.
        let side = s.geo.side;
        let into_grid = far && crate::appgrid::take_from_dock(&d.key, |screen| to_screen(side, d.at, screen));
        let note = if far && s.pins.contains(&d.key) {
            s.pins.retain(|k| *k != d.key);
            (!into_grid).then(|| format!("{name} removed from Dock"))
        } else {
            None
        };
        save_pins(&s.pins);
        note
    };
    rebuild(st);
    settle(st);
    if let Some(text) = note {
        let app = st.borrow().app.clone();
        app.request(crate::app::Peek::Note { text });
    }
}

fn activate(st: &Shared, i: usize) {
    let (app, windows, key, kind) = {
        let s = st.borrow();
        let Some(it) = s.items.get(i) else { return };
        (s.app.clone(), it.windows.clone(), it.key.clone(), it.kind)
    };
    match kind {
        Kind::Trash => return kit::trash::open(),
        Kind::Recent => return stack::open(st, i),
        Kind::Apps => return crate::appgrid::toggle(&app),
        // Opening an app from the dock closes the app grid over it, as macOS does.
        Kind::App => crate::appgrid::close_if_open(),
    }
    if windows.is_empty() {
        let list = app.apps.list();
        if let Some(e) = list.iter().find(|e| e.id == key) {
            app.apps.launch(e);
            bounce(st, key);
        }
        return;
    }
    if let Some(target) = target_window(&windows) { focus_window(target); }
}

fn focus_window(target: Window) {
    glib::spawn_future_local(async move {
        let _ = gio::spawn_blocking(move || {
            if target.minimized { let _ = kit::wm::restore(target.id); } else { compositor::focus(target.id); }
        }).await;
    });
}

fn launch(st: &Shared, i: usize) {
    let key = st.borrow().items.get(i).filter(|it| it.kind == Kind::App).map(|it| it.key.clone());
    if let Some(key) = key {
        launch_key(st, key);
    }
}

fn launch_key(st: &Shared, key: String) {
    let app = st.borrow().app.clone();
    let list = app.apps.list();
    if let Some(e) = list.iter().find(|e| e.id == key) {
        app.apps.launch(e);
        bounce(st, key);
    }
}

fn bounce(st: &Shared, key: String) {
    if !motion::reduced() {
        st.borrow_mut().bounce.push((key, glib::monotonic_time()));
        kick(st);
    }
}

fn toggle_pin(st: &Shared, key: &str) {
    let (app, text) = {
        let mut s = st.borrow_mut();
        let Some(it) = s.items.iter().find(|i| i.key == key) else { return };
        if it.entry.is_none() {
            return;
        }
        let (name, pinned) = (it.name.clone(), it.pinned);
        if pinned {
            s.pins.retain(|k| k != key);
        } else {
            s.pins.push(key.to_string());
        }
        save_pins(&s.pins);
        (s.app.clone(), if pinned { format!("{name} removed from Dock") } else { format!("{name} kept in Dock") })
    };
    rebuild(st);
    app.request(crate::app::Peek::Note { text });
}

fn region(st: &Shared) {
    let mut s = st.borrow_mut();
    let Some(surface) = s.window.surface() else { return };
    let (b0, blen) = s.rest_span();
    let open = s.shown.target() > 0.5;
    let g = s.geo;
    let extra = (g.mag - 1.0) * g.base * 1.6 + 10.0;
    let local = if s.items.is_empty() {
        None
    } else if !open {
        None
    } else {
        let reach = if s.inside || s.drag.is_some() || s.menu { g.reach() } else { g.reserve() };
        Some((b0 - extra, 0.0, blen + 2.0 * extra, reach))
    };
    let rect = local.map(|(a, c, la, lc)| {
        let (x, y, w, h) = s.map(a, c, la, lc);
        (x.floor() as i32, y.floor() as i32, w.ceil() as i32, h.ceil() as i32)
    });
    if s.region == Some(rect.unwrap_or((0, 0, 0, 0))) {
        return;
    }
    let reg = match rect {
        Some((x, y, w, h)) => cairo::Region::create_rectangle(&cairo::RectangleInt::new(x, y, w, h)),
        None => cairo::Region::create(),
    };
    surface.set_input_region(Some(&reg));
    s.region = Some(rect.unwrap_or((0, 0, 0, 0)));
    s.paint.queue_draw();
}

fn kick(st: &Shared) {
    let paint = {
        let mut s = st.borrow_mut();
        if s.ticking {
            return;
        }
        s.ticking = true;
        s.paint.clone()
    };
    let weak = Rc::downgrade(st);
    paint.add_tick_callback(move |w, clock| {
        let Some(st) = weak.upgrade() else { return glib::ControlFlow::Break };
        let Ok(mut s) = st.try_borrow_mut() else { return glib::ControlFlow::Continue };
        let now = glib::monotonic_time().max(clock.frame_time());
        w.queue_draw();
        if motion::reduced() {
            s.bounce.clear();
        }
        let bouncing = s.bounce.iter().any(|(_, t)| now - t < 3 * BOUNCE);
        let gulping = s.gulp.is_some_and(|t| now - t < stack::GULP);
        let busy = gulping || [&s.magnify, &s.shown, &s.tip].iter().any(|t| !t.settled(now))
            || s.slide.values().chain(s.appear.values()).chain(s.away.values()).any(|t| !t.settled(now))
            || bouncing;
        if busy {
            return glib::ControlFlow::Continue;
        }
        s.slide.clear();
        s.appear.clear();
        s.ticking = false;
        glib::ControlFlow::Break
    });
}

impl State {
    fn along_len(&self) -> f64 {
        match self.geo.side {
            Side::Bottom => self.paint.width().max(1) as f64,
            _ => self.paint.height().max(1) as f64,
        }
    }

    fn local(&self, x: f64, y: f64) -> (f64, f64) {
        let d = self.geo.depth();
        match self.geo.side {
            Side::Bottom => (x, d - y),
            Side::Left => (y, x),
            Side::Right => (y, d - x),
        }
    }

    fn map(&self, a: f64, c: f64, la: f64, lc: f64) -> (f64, f64, f64, f64) {
        let d = self.geo.depth();
        match self.geo.side {
            Side::Bottom => (a, d - c - lc, la, lc),
            Side::Left => (c, a, lc, la),
            Side::Right => (d - c - lc, a, lc, la),
        }
    }

    /// Where separators go: after the kept apps, and before the stacks.
    fn separators(&self) -> Vec<usize> {
        let lead = self.items.iter().take_while(|i| i.kind == Kind::Apps).count();
        let apps = lead + self.items[lead..].iter().position(|i| i.kind != Kind::App).unwrap_or(self.items.len() - lead);
        let pinned = lead + self.items.iter().filter(|i| i.pinned).count();
        let mut out = Vec::new();
        if pinned > 0 && pinned < apps {
            out.push(pinned);
        }
        if apps > 0 && apps < self.items.len() {
            out.push(apps);
        }
        out
    }

    fn layout(&self, now: i64) -> Vec<(f64, f64)> {
        let n = self.items.len();
        if n == 0 {
            return Vec::new();
        }
        let g = self.geo;
        let splits = self.separators();
        let sep = g.gap() * 2.0 + 1.0;
        let total0 = n as f64 * g.base + (n - 1) as f64 * g.gap() + sep * splits.len() as f64;
        let start0 = self.along_len() / 2.0 - total0 / 2.0;
        let m = if g.mag > 1.0 && !motion::reduced() { self.magnify.value(now).clamp(0.0, 1.0) } else { 0.0 };
        let pa = self.pointer.map_or(-1e4, |p| p.0);
        let mut sizes = Vec::with_capacity(n);
        for i in 0..n {
            let extra = sep * splits.iter().filter(|&&k| i >= k).count() as f64;
            let c = start0 + i as f64 * (g.base + g.gap()) + extra + g.base / 2.0;
            let d = (pa - c) / g.spread();
            sizes.push(g.base * (1.0 + (g.mag - 1.0) * m * (-d * d).exp()));
        }
        let total = sizes.iter().sum::<f64>() + (n - 1) as f64 * g.gap() + sep * splits.len() as f64;
        let mut a = self.along_len() / 2.0 - total / 2.0;
        let mut out = Vec::with_capacity(n);
        for (i, size) in sizes.into_iter().enumerate() {
            if splits.contains(&i) {
                a += sep;
            }
            out.push((a, size));
            a += size + g.gap();
        }
        out
    }

    fn centers(&self, now: i64) -> HashMap<String, f64> {
        self.items.iter().zip(self.layout(now)).map(|(it, (a, size))| (it.key.clone(), a + size / 2.0)).collect()
    }

    fn rest_span(&self) -> (f64, f64) {
        let n = self.items.len();
        if n == 0 {
            return (self.along_len() / 2.0, 0.0);
        }
        let g = self.geo;
        let sep = (g.gap() * 2.0 + 1.0) * self.separators().len() as f64;
        let total = n as f64 * g.base + (n - 1) as f64 * g.gap() + sep + 2.0 * g.pad();
        (self.along_len() / 2.0 - total / 2.0, total)
    }

    fn body_span(&self, now: i64) -> (f64, f64) {
        let l = self.layout(now);
        let pad = self.geo.pad();
        match (l.first(), l.last()) {
            (Some(f), Some(b)) => (f.0 - pad, b.0 + b.1 - f.0 + 2.0 * pad),
            _ => (self.along_len() / 2.0, 0.0),
        }
    }

    fn over_body(&self, p: (f64, f64), now: i64) -> bool {
        let (b0, blen) = self.body_span(now);
        p.0 >= b0 && p.0 <= b0 + blen && p.1 <= self.geo.reach()
    }

    fn index_at(&self, a: f64, now: i64) -> Option<usize> {
        let gap = self.geo.gap();
        self.layout(now).iter().position(|(x, size)| a >= x - gap / 2.0 && a <= x + size + gap / 2.0)
    }

    fn fill(&self, s: &gtk::Snapshot, a: f64, c: f64, la: f64, lc: f64, r: f64, color: kit::palette::Rgb) {
        let (x, y, w, h) = self.map(a, c, la, lc);
        draw::fill(s, &draw::round(x, y, w, h, r), color);
    }

    fn draw(&self, s: &gtk::Snapshot) {
        let now = glib::monotonic_time();
        if self.items.is_empty() {
            return;
        }
        let shown = self.shown.value(now).clamp(0.0, 1.0);
        if shown < 0.005 {
            s.append_color(&gdk::RGBA::new(0.0, 0.0, 0.0, 0.004), &graphene::Rect::new(0.0, 0.0, 1.0, 1.0));
            return;
        }
        let pal = self.app.palette_at(now);
        let g = self.geo;
        let off = -((1.0 - shown) * (LIFT + g.body() + 36.0)).round();
        let layout = self.layout(now);
        let (b0, blen) = self.body_span(now);
        self.fill(s, b0, LIFT + off, blen, g.body(), g.radius(), pal.body.alpha(1.0));
        for k in self.separators() {
            let a = (layout[k].0 - g.gap() - 0.5).round();
            let (x, y, w, h) = self.map(a, LIFT + off + g.pad() + 4.0, 1.0, g.base - 8.0);
            s.append_color(&pal.ink_main.alpha(0.16).rgba(), &graphene::Rect::new(x as f32, y as f32, w as f32, h as f32));
        }
        let dragged = self.drag.as_ref().filter(|d| d.moving).map(|d| d.key.clone());
        let mut top: Option<(usize, f64, f64)> = None;
        for (i, (item, (a, size))) in self.items.iter().zip(layout.iter()).enumerate() {
            let slide = self.slide.get(&item.key).map_or(0.0, |t| t.value(now));
            let hop = self.bounce.iter().find(|(k, _)| *k == item.key).map_or(0.0, |(_, t)| {
                let p = (now - t) as f64 / BOUNCE as f64;
                if p >= 3.0 { 0.0 } else { (p * std::f64::consts::PI).sin().abs() * g.base * 0.32 * (1.0 - p / 3.0) }
            });
            let c = LIFT + off + g.pad() + hop;
            if Some(&item.key) == dragged.as_ref() {
                top = Some((i, c, *size));
                continue;
            }
            self.draw_item(s, item, a + slide, c, *size, 1.0, &pal, now);
            if !item.windows.is_empty() {
                let r = (g.base * 0.045).max(2.0);
                let center = a + slide + size / 2.0;
                let (x, y, w, h) = self.map(center - r, LIFT + off + 3.5, 2.0 * r, 2.0 * r);
                let away = self.away.get(&item.key).map_or(0.0, |t| t.value(now));
                let ink = if item.focused() { pal.ink_main } else { pal.ink_meta.alpha(0.7) };
                draw::fill(s, &draw::round(x, y, w, h, r), ink.mix(pal.ink_faint, 1.0 - away));
            }
            if self.hover == Some(i) {
                let tip = if self.badges && item.badge > 0 { format!("{} · {} new notification{}", item.name, item.badge, if item.badge == 1 { "" } else { "s" }) } else { item.name.clone() };
                self.draw_tip(s, &tip, a + slide + size / 2.0, c + size, &pal, self.tip.value(now).clamp(0.0, 1.0));
            }
        }
        if let (Some((i, c, size)), Some(d)) = (top, self.drag.as_ref()) {
            let item = &self.items[i];
            let along = d.at.0 - size / 2.0;
            let across = (d.at.1 - size / 2.0).max(c);
            let far = d.at.1 > g.reach() + 24.0 && item.pinned;
            self.draw_item(s, item, along, across, size, if far { 0.55 } else { 1.0 }, &pal, now);
            if far {
                self.draw_tip(s, "Remove", along + size / 2.0, across + size, &pal, 1.0);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_item(&self, s: &gtk::Snapshot, item: &Item, a: f64, c: f64, size: f64, alpha: f64, pal: &kit::palette::Palette, now: i64) {
        let grow = self.appear.get(&item.key).map_or(1.0, |t| t.value(now).clamp(0.0, 1.0));
        let gulp = if item.kind == Kind::Trash { self.gulp.map_or(0.0, |t| stack::gulp(now - t)) } else { 0.0 };
        let shown = size * (0.4 + 0.6 * grow) * (1.0 + gulp);
        let (a, c) = (a + (size - shown) / 2.0, c + (size - shown) / 2.0);
        let (x, y, w, _) = self.map(a, c, shown, shown);
        let alpha = alpha * grow;
        if alpha < 0.999 {
            s.push_opacity(alpha);
        }
        let away = self.away.get(&item.key).map_or(0.0, |t| t.value(now));
        s.push_opacity(1.0 - 0.5 * away);
        match (self.art.get(&item.key), self.icons.get(&item.key).and_then(|p| p.as_ref())) {
            (Some(t), _) => s.append_scaled_texture(t, gsk::ScalingFilter::Trilinear, &graphene::Rect::new(x.round() as f32, y.round() as f32, w as f32, w as f32)),
            (None, Some(p)) => draw::paintable(s, p, x.round(), y.round(), w),
            (None, None) => draw::monogram(s, &self.paint, &self.app.cfg, &item.name, x, y, w, pal.raised, pal.ink_main),
        }
        s.pop();
        if self.badges && item.badge > 0 {
            let text = if item.badge > 99 { "99+".to_string() } else { item.badge.to_string() };
            let px = (shown * 0.24).clamp(10.0, 16.0);
            let h = px + 6.0;
            let tw = draw::width(&self.paint, &self.app.cfg, &text, px, Face::Number(600)).ceil();
            let bw = (tw + 8.0).max(h);
            let (bx, by) = (x + w - bw * 0.75, y - h * 0.25);
            draw::fill(s, &draw::round(bx - 1.5, by - 1.5, bw + 3.0, h + 3.0, h / 2.0 + 1.5), kit::palette::Rgb::new(0.0, 0.0, 0.0).alpha(0.35));
            draw::fill(s, &draw::round(bx, by, bw, h, h / 2.0), kit::palette::Rgb::new(1.0, 0.23, 0.19));
            draw::text(s, &self.paint, &self.app.cfg, &text, px, Face::Number(600), kit::palette::Rgb::new(1.0, 1.0, 1.0), bx, by + h / 2.0, bw, pango::Alignment::Center);
        }
        if alpha < 0.999 {
            s.pop();
        }
    }

    fn draw_tip(&self, s: &gtk::Snapshot, text: &str, center: f64, far: f64, pal: &kit::palette::Palette, tip: f64) {
        if tip < 0.01 {
            return;
        }
        let cfg = &self.app.cfg;
        let tw = draw::width(&self.paint, cfg, text, 13.0, Face::Text(520)).ceil().min(220.0);
        let (bw, bh) = (tw + 24.0, 28.0);
        let lift = 4.0 * (1.0 - tip);
        let rect = match self.geo.side {
            Side::Bottom => {
                let a = (center - bw / 2.0).clamp(4.0, (self.along_len() - bw - 4.0).max(4.0));
                self.map(a, far + 10.0 + lift, bw, bh)
            }
            _ => self.map((center - bh / 2.0).round(), far + 10.0 + lift, bh, bw),
        };
        let (x, y, w, h) = (rect.0.round(), rect.1.round(), rect.2, rect.3);
        s.push_opacity(tip);
        draw::fill(s, &draw::round(x, y, w, h, 14.0), pal.body.alpha(1.0).mix(pal.ink_main, 0.9));
        draw::text(s, &self.paint, cfg, text, 13.0, Face::Text(520), pal.ink_main, x, y + h / 2.0, w, pango::Alignment::Center);
        s.pop();
    }
}

#[derive(Clone)]
enum Act {
    Focus(Window),
    Launch,
    Pin,
    Quit(Vec<u64>),
    OpenTrash,
    /// Asks once: the first click turns the row into a confirmation.
    EmptyTrash(bool),
    Stack,
    OpenRecent,
}

struct Menu {
    dock: std::rc::Weak<RefCell<State>>,
    window: gtk::Window,
    scrim: gtk::Window,
    paint: Paint,
    title: String,
    key: String,
    rows: Vec<(String, Act, bool)>,
    hover: Option<usize>,
    fade: Tween,
}

fn close_menu() {
    let Some(m) = MENU.with(|m| m.borrow_mut().take()) else { return };
    let (window, scrim, dock) = {
        let m = m.borrow();
        (m.window.clone(), m.scrim.clone(), m.dock.upgrade())
    };
    scrim.destroy();
    window.destroy();
    if let Some(st) = dock {
        st.borrow_mut().menu = false;
        settle(&st);
    }
}

fn open_menu(st: &Shared, i: usize) {
    close_menu();
    let titles: HashMap<u64, String> = compositor::windows().into_iter().map(|w| (w.id, w.title)).collect();
    let (rows, title, key, monitor, anchor, margin_side, margin_main, h) = {
        let s = st.borrow();
        let Some(item) = s.items.get(i) else { return };
        let mut rows: Vec<(String, Act, bool)> = Vec::new();
        match item.kind {
            Kind::Trash => {
                rows.push(("Open".into(), Act::OpenTrash, false));
                if s.trash_full {
                    rows.push(("Empty Trash".into(), Act::EmptyTrash(false), true));
                }
            }
            Kind::Recent => {
                rows.push(("Show Recent Files".into(), Act::Stack, false));
                rows.push(("Open in Files".into(), Act::OpenRecent, false));
            }
            Kind::App | Kind::Apps => {}
        }
        let mut ids = item.windows.clone();
        ids.sort_by_key(|w| w.id);
        for window in ids.iter().take(6) {
            let t = titles.get(&window.id).cloned().filter(|t| !t.is_empty()).unwrap_or_else(|| item.name.clone());
            rows.push((if window.focused { format!("● {t}") } else { t }, Act::Focus(*window), false));
        }
        if item.entry.is_some() {
            rows.push((if item.windows.is_empty() { "Open".into() } else { "New Window".into() }, Act::Launch, false));
            rows.push((if item.pinned { "Remove from Dock".into() } else { "Keep in Dock".into() }, Act::Pin, false));
        }
        if !item.windows.is_empty() {
            rows.push(("Quit".into(), Act::Quit(item.windows.iter().map(|w| w.id).collect()), true));
        } else if item.entry.is_some() && crate::quit::running(&s.app, &item.key) {
            rows.push(("Quit".into(), Act::Quit(Vec::new()), true));
        }
        let h = 18.0 + 30.0 + rows.len() as f64 * ROW;
        let now = glib::monotonic_time();
        let layout = s.layout(now);
        let center = layout.get(i).map_or(s.along_len() / 2.0, |(a, size)| a + size / 2.0);
        let g = s.geo;
        let gap = LIFT + g.pad() + g.base * g.mag.max(1.0) + 12.0;
        let (anchor, main) = match g.side {
            Side::Bottom => (Edge::Bottom, (center - MENU_W / 2.0).clamp(8.0, (s.along_len() - MENU_W - 8.0).max(8.0))),
            Side::Left => (Edge::Left, (center - h / 2.0).clamp(8.0, (s.along_len() - h - 8.0).max(8.0))),
            Side::Right => (Edge::Right, (center - h / 2.0).clamp(8.0, (s.along_len() - h - 8.0).max(8.0))),
        };
        (rows, item.name.clone(), item.key.clone(), s.window.monitor(), anchor, gap, main, h)
    };
    let window = gtk::Window::new();
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace(Some("digit-dock-menu"));
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(KeyboardMode::Exclusive);
    window.set_monitor(monitor.as_ref());
    window.add_css_class("digit");
    window.set_anchor(anchor, true);
    window.set_margin(anchor, margin_side as i32);
    let start = if anchor == Edge::Bottom { Edge::Left } else { Edge::Top };
    window.set_anchor(start, true);
    window.set_margin(start, margin_main as i32);
    window.set_default_size(MENU_W as i32, h as i32);
    let paint = Paint::new(MENU_W as i32, h as i32);
    window.set_child(Some(&paint));
    let mut fade = Tween::still(0.0);
    fade.go(1.0, glib::monotonic_time(), 160, motion::STANDARD);
    let scrim = crate::paint::scrim(monitor.as_ref(), close_menu);
    let menu = Rc::new(RefCell::new(Menu { dock: Rc::downgrade(st), window: window.clone(), scrim, paint: paint.clone(), title, key, rows, hover: None, fade }));
    let weak = Rc::downgrade(&menu);
    paint.on_draw(move |s| {
        if let Some(m) = weak.upgrade() {
            if let Ok(m) = m.try_borrow() {
                m.draw(s);
            }
        }
    });
    let motion = gtk::EventControllerMotion::new();
    let weak = Rc::downgrade(&menu);
    motion.connect_motion(move |_, _, y| {
        if let Some(m) = weak.upgrade() {
            let mut m = m.borrow_mut();
            let at = m.row_at(y);
            if at != m.hover {
                m.hover = at;
                m.paint.set_cursor_from_name(at.map(|_| "pointer"));
                m.paint.queue_draw();
            }
        }
    });
    let weak = Rc::downgrade(&menu);
    motion.connect_leave(move |_| {
        if let Some(m) = weak.upgrade() {
            m.borrow_mut().hover = None;
            m.borrow().paint.queue_draw();
        }
    });
    paint.add_controller(motion);
    let click = gtk::GestureClick::new();
    let weak = Rc::downgrade(&menu);
    click.connect_released(move |_, _, _, y| {
        if let Some(m) = weak.upgrade() {
            let at = m.borrow().row_at(y);
            if let Some(r) = at {
                run(&m, r);
            }
        }
    });
    paint.add_controller(click);
    let keys = gtk::EventControllerKey::new();
    let weak = Rc::downgrade(&menu);
    keys.connect_key_pressed(move |_, key, _, _| {
        let Some(m) = weak.upgrade() else { return glib::Propagation::Proceed };
        let n = m.borrow().rows.len();
        match key {
            gdk::Key::Escape => close_menu(),
            gdk::Key::Down | gdk::Key::Up => {
                let mut mm = m.borrow_mut();
                let cur = mm.hover.map_or(if key == gdk::Key::Down { n - 1 } else { 0 }, |h| h);
                mm.hover = Some(if key == gdk::Key::Down { (cur + 1) % n } else { (cur + n - 1) % n });
                mm.paint.queue_draw();
            }
            gdk::Key::Return | gdk::Key::KP_Enter => {
                let at = m.borrow().hover;
                if let Some(r) = at {
                    run(&m, r);
                }
            }
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    window.add_controller(keys);
    MENU.with(|slot| *slot.borrow_mut() = Some(menu.clone()));
    st.borrow_mut().menu = true;
    {
        let mut s = st.borrow_mut();
        s.hover = None;
        s.tip.set(0.0);
    }
    settle(st);
    window.present();
    let weak = Rc::downgrade(&menu);
    paint.add_tick_callback(move |w, clock| {
        let Some(m) = weak.upgrade() else { return glib::ControlFlow::Break };
        w.queue_draw();
        if m.borrow().fade.settled(clock.frame_time()) { glib::ControlFlow::Break } else { glib::ControlFlow::Continue }
    });
}

fn run(m: &Rc<RefCell<Menu>>, r: usize) {
    let (dock, act, key) = {
        let m = m.borrow();
        let Some(row) = m.rows.get(r) else { return };
        (m.dock.upgrade(), row.1.clone(), m.key.clone())
    };
    if matches!(act, Act::EmptyTrash(false)) {
        let mut m = m.borrow_mut();
        m.rows[r] = ("Empty Trash? Click to confirm".into(), Act::EmptyTrash(true), true);
        m.paint.queue_draw();
        return;
    }
    close_menu();
    let Some(st) = dock else { return };
    match act {
        Act::Focus(window) => focus_window(window),
        Act::Launch => launch_key(&st, key),
        Act::Pin => toggle_pin(&st, &key),
        Act::Quit(ids) => {
            let app = st.borrow().app.clone();
            crate::quit::quit(&app, &key, ids);
        }
        Act::OpenTrash => kit::trash::open(),
        Act::EmptyTrash(_) => kit::trash::empty(),
        Act::Stack => {
            let at = st.borrow().items.iter().position(|i| i.kind == Kind::Recent);
            if let Some(i) = at {
                stack::open(&st, i);
            }
        }
        Act::OpenRecent => {
            let _ = gio::Subprocess::newv(&[std::ffi::OsStr::new("digit-files"), std::ffi::OsStr::new("recent:///")], gio::SubprocessFlags::NONE);
        }
    }
}

impl Menu {
    fn row_at(&self, y: f64) -> Option<usize> {
        let top = 9.0 + 30.0;
        if y < top {
            return None;
        }
        let i = ((y - top) / ROW) as usize;
        (i < self.rows.len()).then_some(i)
    }

    fn draw(&self, s: &gtk::Snapshot) {
        let Some(st) = self.dock.upgrade() else { return };
        let Ok(dock) = st.try_borrow() else { return };
        let now = self.paint.frame_clock().map(|c| c.frame_time()).unwrap_or_else(glib::monotonic_time);
        let fade = self.fade.value(now).clamp(0.0, 1.0);
        let pal = dock.app.palette_at(now);
        let cfg = &dock.app.cfg;
        let (w, h) = (MENU_W, 18.0 + 30.0 + self.rows.len() as f64 * ROW);
        s.push_opacity(fade);
        draw::fill(s, &draw::round(0.0, 0.0, w, h, 18.0), pal.body.alpha(1.0));
        draw::text(s, &self.paint, cfg, &self.title, 13.0, Face::Text(600), pal.ink_meta, 16.0, 9.0 + 15.0, w - 32.0, pango::Alignment::Left);
        for (i, (label, _, danger)) in self.rows.iter().enumerate() {
            let y = 9.0 + 30.0 + i as f64 * ROW;
            if self.hover == Some(i) {
                draw::fill(s, &draw::round(6.0, y + 1.0, w - 12.0, ROW - 2.0, 11.0), if *danger { pal.risk.alpha(0.18) } else { pal.ink_main.alpha(0.1) });
            }
            let ink = if *danger { pal.risk } else { pal.ink_main };
            draw::text(s, &self.paint, cfg, label, 14.0, Face::Text(450), ink, 16.0, y + ROW / 2.0, w - 32.0, pango::Alignment::Left);
        }
        s.pop();
    }
}

#[cfg(test)]
mod target_tests {
    use super::*;

    #[test]
    fn activation_cycles_and_prefers_recent_normal_windows() {
        let a = Window { id: 1, stamp: 10, focused: false, minimized: false };
        let b = Window { id: 2, stamp: 30, minimized: true, ..a };
        let c = Window { id: 3, stamp: 20, ..a };
        assert_eq!(target_window(&[]), None);
        assert_eq!(target_window(&[b, a, c]), Some(c));
        assert_eq!(target_window(&[b, Window { minimized: true, ..a }]), Some(b));
        assert_eq!(target_window(&[b, Window { focused: true, ..a }, c]), Some(b));
        assert_eq!(target_window(&[b, a, Window { focused: true, ..c }]), Some(a));
        assert_eq!(target_window(&[b]), Some(b));
    }
}
