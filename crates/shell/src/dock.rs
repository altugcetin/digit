use crate::app::App;
use crate::compositor;
use crate::draw::{self, Face};
use crate::paint::Paint;
use gtk::{cairo, gdk, gio, glib, graphene, gsk, pango, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kit::motion::{self, Tween};
use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

mod launch_drag;
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

    /// Between two parts of the dock: the dock's own padding on both sides of the line, so a
    /// part has the same room at each end wherever it stands.
    fn split(&self) -> f64 {
        2.0 * self.pad() + 1.0
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
    /// Where the icon was taken, from its centre, so it doesn't jump under the pointer.
    grab: (f64, f64),
    moving: bool,
    dock_span: (f64, f64),
    /// Where the app opens if dropped now, out on the screen (see `kit::wm::Zone`).
    zone: Option<kit::wm::Zone>,
    /// The compositor state for this drag's preview.
    preview: Option<Rc<OnceCell<Option<kit::wm::HyprPreview>>>>,
    /// Since when the drag's own icon shows the app, out on the screen, instead of the dock.
    away: Option<i64>,
    /// Whether a drop here, among the apps that aren't kept, takes a closed app off the dock.
    unpin: bool,
    /// Whether the drag went out onto the screen; a drop out there never unpins.
    out: bool,
    /// The pins and order before the drag, put back when the app is dropped on the screen.
    before: (Vec<String>, Vec<String>),
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
    drag_target: Option<gtk::Window>,
    drag_visual: Option<launch_drag::Visual>,
    /// An app let go over the dock, coming down into its place.
    landing: Option<(String, Tween)>,
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
    /// An always shown dock the shortcut has hidden, until the setting changes or the shell
    /// starts again.
    static HIDDEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The windows `open_in` has put where their app was dropped.
    static PLACED: RefCell<std::collections::BTreeSet<u64>> = const { RefCell::new(std::collections::BTreeSet::new()) };
}

/// The dock's mode as it is now: as set, unless the shortcut has hidden an always shown one.
pub fn mode(app: &App) -> String {
    let set = app.prefs.borrow().dock.mode.clone();
    shown_mode(set, HIDDEN.with(std::cell::Cell::get))
}

fn shown_mode(set: String, hidden: bool) -> String {
    if hidden && set == "on" { "off".into() } else { set }
}

/// Its setting changed: the dock is as set again.
pub fn follow_setting() {
    HIDDEN.with(|h| h.set(false));
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
    let _ = kit::atomic::write(&pins_path(), pins.join("\n") + "\n");
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
                let kept = (s.pins.clone(), s.order.clone());
                s.drag = Some(Drag { key: id.to_string(), origin: p, at: p, grab: (0.0, 0.0), moving: true, dock_span: s.rest_span(), zone: None, preview: None, away: None, unpin: false, out: false, before: kept });
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
    launch_drag::close(st);
    close_menu();
    crate::snapzone::hide();
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
    let mode = match mode(app).as_str() {
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

/// The shortcut, the dock's setting kept: an always shown dock hides or shows for now, an
/// auto-hiding one comes out as with the pointer at the edge (and goes once the pointer has been
/// over it and left), and one that is off stays off.
pub fn command(app: &Rc<App>, verb: &str) {
    let set = app.prefs.borrow().dock.mode.clone();
    match set.as_str() {
        "off" => {}
        "on" => {
            let hidden = HIDDEN.with(std::cell::Cell::get);
            let hide = !wanted(verb, !hidden);
            if hide != hidden {
                close_menu();
                HIDDEN.with(|h| h.set(hide));
                apply(app);
            }
        }
        _ => {
            if let Some(st) = current() {
                peek(&st, verb);
            }
        }
    }
}

/// Whether the dock is to show after "show", "hide" or a toggle, `shown` as it is.
fn wanted(verb: &str, shown: bool) -> bool {
    match verb {
        "show" => true,
        "hide" => false,
        _ => !shown,
    }
}

/// Brings an auto-hiding dock out, or puts it away, as the pointer at its edge would.
fn peek(st: &Shared, verb: &str) {
    let out = st.borrow().shown.target() > 0.5;
    let want = wanted(verb, out);
    if want == out {
        return;
    }
    if !want {
        close_menu();
    }
    st.borrow_mut().reveal = want;
    settle(st);
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
        drag_target: None,
        drag_visual: None,
        landing: None,
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
        s.drag = drag_from(&s, x, y);
    });
    let weak = Rc::downgrade(st);
    drag.connect_drag_update(move |g, dx, dy| {
        let (Some(st), Some((x, y))) = (weak.upgrade(), g.start_point()) else { return };
        drag_to(&st, x + dx, y + dy);
    });
    let weak = Rc::downgrade(st);
    drag.connect_drag_end(move |g, _, _| {
        if let Some(st) = weak.upgrade() {
            let released = g.current_event().is_some_and(|event| {
                matches!(event.event_type(), gdk::EventType::ButtonRelease | gdk::EventType::TouchEnd)
            });
            if st.borrow().drag_target.is_none() {
                drop_drag(&st, released);
            }
        }
    });
    paint.add_controller(drag);
    launch_drag::wire(st, &paint);
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
    let Ok(p) = st.try_borrow().map(|s| s.local(x, y)) else { return };
    moved_to(st, p);
}

fn moved_to(st: &Shared, p: Point) {
    {
        let Ok(mut s) = st.try_borrow_mut() else { return };
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
        // A drag's drop target covers the dock and tells it where the pointer is.
        if !s.drag.as_ref().is_some_and(|d| d.preview.is_some()) {
            s.magnify.go(0.0, now, 300, motion::STANDARD);
        }
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

fn drag_from(s: &State, x: f64, y: f64) -> Option<Drag> {
    let p = s.local(x, y);
    let now = glib::monotonic_time();
    let i = s.index_at(p.0, now)?;
    let item = s.items.get(i).filter(|it| it.kind == Kind::App)?;
    let (a, size) = *s.layout(now).get(i)?;
    let grab = (p.0 - a - size / 2.0, p.1 - s.row() - size / 2.0);
    let before = (s.pins.clone(), s.order.clone());
    Some(Drag { key: item.key.clone(), origin: p, at: p, grab, moving: false, dock_span: s.rest_span(), zone: None, preview: None, away: None, unpin: false, out: false, before })
}

fn drag_to(st: &Shared, x: f64, y: f64) {
    let p = {
        let mut s = st.borrow_mut();
        let p = s.local(x, y);
        if launch_drag::enabled(&s) {
            // If the native drag cannot start, its release still must not activate an app.
            if s.drag.as_ref().is_some_and(|d| (p.0 - d.origin.0).hypot(p.1 - d.origin.1) >= 8.0) {
                s.dragged = glib::monotonic_time();
            }
            return;
        }
        p
    };
    drag_at(st, p);
}

fn drag_at(st: &Shared, p: Point) {
    let reorder = {
        let mut s = st.borrow_mut();
        // Out over the open app grid, it draws the app under the pointer.
        if let Some(d) = s.drag.as_ref().filter(|d| d.moving && s.installed(&d.key)) {
            let side = s.geo.side;
            let out = p.1 > s.geo.reach() + 24.0;
            crate::appgrid::ghost(out.then(|| (d.key.clone(), Box::new(move |screen| to_screen(side, p, screen)) as crate::appgrid::Place)));
        }
        let Some(d) = s.drag.as_mut() else { return };
        d.at = p;
        d.unpin = false;
        if !d.moving {
            if (p.0 - d.origin.0).hypot(p.1 - d.origin.1) < 8.0 {
                return;
            }
            d.moving = true;
            s.hover = None;
            s.tip.set(0.0);
        }
        if aim(&mut s) {
            s.paint.queue_draw();
            return;
        }
        let key = s.drag.as_ref().map(|d| d.key.clone()).unwrap_or_default();
        let now = glib::monotonic_time();
        let target = s.index_at(p.0, now).or_else(|| {
            // Beside the dock or on a separator: the nearest one.
            let away = |(a, size): (f64, f64)| (a + size / 2.0 - p.0).abs();
            s.layout(now).into_iter().map(away).enumerate().min_by(|a, b| a.1.total_cmp(&b.1)).map(|(i, _)| i)
        });
        // The Apps button stays first: places count from after it.
        let lead = s.items.iter().take_while(|i| i.kind == Kind::Apps).count();
        // It stays among the apps, and one that can't be pinned among the unpinned ones.
        let apps = s.items.iter().filter(|i| i.kind == Kind::App).count();
        let least = if s.installed(&key) { 0 } else { s.items.iter().filter(|i| i.pinned).count() };
        let target = target.map(|t| t.saturating_sub(lead));
        // Over Recent Files or the Trash, past the apps.
        let past = target.is_some_and(|t| t >= apps);
        let target = target.map(|t| t.min(apps.saturating_sub(1)).max(least));
        let mut order: Vec<String> = s.items.iter().filter(|i| i.kind != Kind::Apps).map(|i| i.key.clone()).collect();
        match (target, order.iter().position(|k| *k == key)) {
            (Some(t), Some(c)) => {
                // Landing among the other pinned apps pins it, among the unpinned ones unpins it, and on the border keeps it as it was.
                let others = s.items.iter().filter(|i| i.pinned && i.key != key).count();
                let item = s.items.iter().find(|i| i.key == key);
                let was = item.is_some_and(|i| i.pinned);
                let closed = item.is_some_and(|i| i.windows.is_empty());
                let front = others == 0 && !was && s.layout(now).get(lead).is_some_and(|&first| {
                    in_front(s.geo, p.0, s.drag.as_ref().map_or(p.0, |d| d.origin.0), first, lead > 0)
                });
                let side = if past { std::cmp::Ordering::Greater } else if front { std::cmp::Ordering::Less } else { t.cmp(&others) };
                let pin = match side {
                    std::cmp::Ordering::Less => true,
                    std::cmp::Ordering::Greater => was && closed,
                    std::cmp::Ordering::Equal => was,
                } && item.is_some_and(|i| i.entry.is_some());
                // A closed app isn't among the others until it's dropped there.
                let unpin = side == std::cmp::Ordering::Greater && was && closed;
                let t = if pin { t.min(others) } else { t };
                if let Some(d) = s.drag.as_mut() {
                    d.unpin = unpin && d.preview.is_some();
                }
                if t == c && pin == was {
                    s.paint.queue_draw();
                    return;
                }
                let k = order.remove(c);
                order.insert(t.min(order.len()), k);
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

/// Whether a drag point is on the dock's body, where a drop rearranges the dock.
fn aimed_at_dock(s: &State, drag: &Drag) -> bool {
    let (a, c) = drag.at;
    // At least the resting dock, and as much more as its magnified icons take.
    let (start, width) = drag.dock_span;
    let now = glib::monotonic_time();
    let grown = s.magnify.target() > 0.5 && c >= 0.0 && s.over_body(drag.at, now);
    grown || (0.0..=s.geo.reserve()).contains(&c) && (start..=start + width).contains(&a)
}

fn begin_placement(st: &Shared, s: &mut State) {
    let preview = Rc::new(OnceCell::new());
    let Some(key) = s.drag.as_ref().map(|d| d.key.clone()) else { return };
    let moving = s.moves(&key).map(|w| w.id);
    if let Some(d) = s.drag.as_mut() {
        d.preview = Some(preview.clone());
    }
    let (weak, output) = (Rc::downgrade(st), s.output.clone());
    glib::spawn_future_local(async move {
        let _ = preview.set(gio::spawn_blocking(move || kit::wm::HyprPreview::read(&output, moving)).await.ok().flatten());
        if let Some(st) = weak.upgrade() {
            let mut s = st.borrow_mut();
            if s.drag.as_ref().and_then(|d| d.preview.as_ref()).is_some_and(|p| Rc::ptr_eq(p, &preview)) {
                aim(&mut s);
            }
        }
    });
}

/// Shows the drop zone while an app is dragged off the dock.
fn aim(s: &mut State) -> bool {
    // Out means off the dock's body, not just above its band: the band spans the whole
    // edge, and the bottom corners of the screen are in it.
    // Only a drag that opens apps has a preview; any other keeps its own way off the dock.
    let out = s.drag.as_ref().is_some_and(|d| d.preview.is_some() && !aimed_at_dock(s, d));
    let monitor = if out { kit::screens::by_name(&s.output).or_else(crate::screens::main) } else { None };
    let Some(g) = monitor.map(|m| m.geometry()) else {
        if s.drag.as_mut().and_then(|d| d.zone.take()).is_some() {
            crate::snapzone::hide();
        }
        return false;
    };
    let (mw, mh) = (f64::from(g.width()), f64::from(g.height()));
    let (app, output) = (s.app.clone(), s.output.clone());
    let Some(d) = s.drag.as_mut() else { return false };
    let (sx, sy) = to_screen(s.geo.side, d.at, (mw, mh));
    let zone = kit::wm::Zone::at(sx, sy, mw, mh);
    let preview = d.preview.as_ref().and_then(|p| p.get()).and_then(Option::as_ref);
    // Until Hyprland's layout is read, only the floating center can be shown.
    let rect = zone.and_then(|z| match preview {
        Some(preview) => preview.rect(z, (sx, sy)),
        None => (z == kit::wm::Zone::Float).then(|| kit::wm::hypr_float_rect([0.0, 0.0, mw, mh])),
    });
    // A drop opens the app only where the preview can show its tile.
    let zone = zone.filter(|_| rect.is_some());
    if let Some(rect) = rect {
        crate::snapzone::show(&app, &output, rect);
    } else {
        crate::snapzone::hide();
    }
    d.zone = zone;
    d.out = true;
    true
}

/// Moves an open app's window to `zone`. Otherwise opens the app and, once its window shows
/// up, puts that window there.
fn open_in(st: &Shared, key: String, zone: kit::wm::Zone) {
    let (app, output, moves) = { let s = st.borrow(); (s.app.clone(), s.output.clone(), s.moves(&key)) };
    if let Some(w) = moves {
        gio::spawn_blocking(move || kit::wm::bring(w.id, zone, &output));
        return;
    }
    let windows = move |key: &str| -> Vec<u64> {
        let list = app.apps.list();
        app.running.borrow().iter().filter(|r| app.apps.find(&r.app_id).and_then(|i| list.get(i)).is_some_and(|e| e.id == key)).map(|r| r.id).collect()
    };
    launch_key(st, key.clone());
    let started = glib::monotonic_time();
    glib::timeout_add_local(Duration::from_millis(150), move || {
        // Two drops of one app each take a window of their own.
        let new = windows(&key).into_iter().find(|id| PLACED.with(|p| p.borrow_mut().insert(*id)));
        if let Some(id) = new {
            let output = output.clone();
            gio::spawn_blocking(move || kit::wm::place(id, zone, &output));
            return glib::ControlFlow::Break;
        }
        if glib::monotonic_time() - started > 15_000_000 { glib::ControlFlow::Break } else { glib::ControlFlow::Continue }
    });
}

/// Glides an app let go at `at`, over the dock, into its place.
fn land(st: &Shared, key: &str, at: Point, grab: Point) {
    if motion::reduced() {
        return;
    }
    {
        let mut s = st.borrow_mut();
        let now = glib::monotonic_time();
        let Some(i) = s.items.iter().position(|it| it.key == key) else { return };
        let Some(&(a, size)) = s.layout(now).get(i) else { return };
        let glide = |from: f64| {
            let mut t = Tween::still(from);
            t.go(0.0, now, 300, motion::EMPHASIZED_DECEL);
            t
        };
        let (along, across) = (at.0 - grab.0 - size / 2.0 - a, (at.1 - grab.1 - size / 2.0 - s.row()).max(0.0));
        s.slide.insert(key.to_string(), glide(along));
        s.landing = Some((key.to_string(), glide(across)));
    }
    kick(st);
}

fn drop_drag(st: &Shared, released: bool) {
    let (note, landed) = {
        let mut s = st.borrow_mut();
        let Some(d) = s.drag.take() else { return };
        if !d.moving {
            return;
        }
        s.dragged = glib::monotonic_time();
        if d.out {
            crate::snapzone::hide();
        }
        let outside = d.out && !aimed_at_dock(&s, &d);
        if outside || (!released && d.preview.is_some()) {
            // Reordering during the drag was only a preview of the drop.
            (s.pins, s.order) = d.before;
            drop(s);
            rebuild(st);
            if !outside {
                land(st, &d.key, d.at, d.grab);
            }
            settle(st);
            if released && let Some(zone) = d.zone {
                open_in(st, d.key, zone);
            }
            return;
        }
        let far = d.at.1 > s.geo.reach() + 24.0;
        let name = s.items.iter().find(|i| i.key == d.key).map(|i| i.name.clone()).unwrap_or_default();
        // Over the open app grid, the app goes into it where it's let go, quietly.
        let side = s.geo.side;
        let into_grid = far && s.installed(&d.key) && crate::appgrid::take_from_dock(&d.key, |screen| to_screen(side, d.at, screen));
        let note = if (far || d.unpin) && s.pins.contains(&d.key) {
            s.pins.retain(|k| *k != d.key);
            (!into_grid).then(|| format!("{name} removed from Dock"))
        } else {
            None
        };
        save_pins(&s.pins);
        (note, (!far && !d.unpin).then_some((d.key, d.at, d.grab)))
    };
    rebuild(st);
    if let Some((key, at, grab)) = landed {
        land(st, &key, at, grab);
    }
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
            || s.landing.as_ref().is_some_and(|(_, t)| !t.settled(now))
            || bouncing;
        if busy {
            return glib::ControlFlow::Continue;
        }
        s.slide.clear();
        s.appear.clear();
        s.landing = None;
        s.ticking = false;
        glib::ControlFlow::Break
    });
}

/// Where the dock's parts begin after the first: what's kept (the Apps button with it), the
/// apps that are just open, and the stacks. A part with nothing in it has no line.
fn splits(items: &[(Kind, bool)]) -> Vec<usize> {
    let lead = items.iter().take_while(|i| i.0 == Kind::Apps).count();
    let apps = lead + items[lead..].iter().position(|i| i.0 != Kind::App).unwrap_or(items.len() - lead);
    let pinned = lead + items.iter().filter(|i| i.1).count();
    let mut out = Vec::new();
    if pinned > 0 && pinned < apps {
        out.push(pinned);
    }
    if apps > 0 && apps < items.len() {
        out.push(apps);
    }
    out
}

/// Where each of `n` icons starts along the dock and how big it is, with the dock in the middle
/// of `len` and the icons magnified around `pointer` by `m` from 0 to 1.
fn slots(g: Geo, n: usize, splits: &[usize], len: f64, pointer: Option<f64>, m: f64) -> Vec<(f64, f64)> {
    if n == 0 {
        return Vec::new();
    }
    let room = |sizes: f64| sizes + (n - 1 - splits.len()) as f64 * g.gap() + splits.len() as f64 * g.split();
    let before = |i: usize| if splits.contains(&i) { g.split() } else if i == 0 { 0.0 } else { g.gap() };
    // On whole pixels at rest, so the body's edges and the lines stand as far from the icons as
    // they should. The middle stays there as icons grow.
    let start0 = (len / 2.0 - room(n as f64 * g.base) / 2.0).round();
    let middle = start0 + room(n as f64 * g.base) / 2.0;
    let mut at = start0;
    let sizes: Vec<f64> = (0..n).map(|i| {
        at += before(i);
        let d = (pointer.unwrap_or(-1e4) - at - g.base / 2.0) / g.spread();
        at += g.base;
        g.base * (1.0 + (g.mag - 1.0) * m * (-d * d).exp())
    }).collect();
    let mut at = middle - room(sizes.iter().sum()) / 2.0;
    sizes.into_iter().enumerate().map(|(i, size)| {
        at += before(i);
        let slot = (at, size);
        at += size;
        slot
    }).collect()
}

/// The dock's body around its icons: where it starts and how long it is.
fn span(g: Geo, slots: &[(f64, f64)]) -> Option<(f64, f64)> {
    let (f, b) = (slots.first()?, slots.last()?);
    Some((f.0 - g.pad(), b.0 + b.1 - f.0 + 2.0 * g.pad()))
}

/// Whether an app dragged to `at` from `origin` is taken in front of the open apps, to be the
/// first one kept, when none is: past the line to the Apps button's side, or with no such line,
/// onto the first app's near half. `first` is the first app's place.
fn in_front(g: Geo, at: f64, origin: f64, first: (f64, f64), line: bool) -> bool {
    if line { at < first.0 - g.pad() } else { at < first.0 + first.1 / 2.0 && origin - at >= first.1 * 0.25 }
}

impl State {
    fn along_len(&self) -> f64 {
        match self.geo.side {
            Side::Bottom => self.paint.width().max(1) as f64,
            _ => self.paint.height().max(1) as f64,
        }
    }

    /// How far in from the edge the icons stand in the shown dock.
    fn row(&self) -> f64 {
        LIFT + self.geo.pad()
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
        splits(&self.items.iter().map(|i| (i.kind, i.pinned)).collect::<Vec<_>>())
    }

    fn layout(&self, now: i64) -> Vec<(f64, f64)> {
        let m = if self.geo.mag > 1.0 && !motion::reduced() { self.magnify.value(now).clamp(0.0, 1.0) } else { 0.0 };
        slots(self.geo, self.items.len(), &self.separators(), self.along_len(), self.pointer.map(|p| p.0), m)
    }

    fn centers(&self, now: i64) -> HashMap<String, f64> {
        self.items.iter().zip(self.layout(now)).map(|(it, (a, size))| (it.key.clone(), a + size / 2.0)).collect()
    }

    fn rest_span(&self) -> (f64, f64) {
        span(self.geo, &slots(self.geo, self.items.len(), &self.separators(), self.along_len(), None, 0.0)).unwrap_or((self.along_len() / 2.0, 0.0))
    }

    fn body_span(&self, now: i64) -> (f64, f64) {
        span(self.geo, &self.layout(now)).unwrap_or((self.along_len() / 2.0, 0.0))
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
            let a = (layout[k].0 - g.pad() - 1.0).round();
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
            let rise = self.landing.as_ref().filter(|(k, _)| *k == item.key).map_or(0.0, |(_, t)| t.value(now));
            self.draw_item(s, item, a + slide, c + rise, *size, 1.0, &pal, now);
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
        // Out on the screen the drag's own icon shows it. The two overlap for a moment, so
        // neither is missing while the other comes up.
        let here = |d: &&Drag| d.away.is_none_or(|t| now - t < 80_000);
        if let (Some((i, c, size)), Some(d)) = (top, self.drag.as_ref().filter(here)) {
            let item = &self.items[i];
            let along = d.at.0 - d.grab.0 - size / 2.0;
            let across = (d.at.1 - d.grab.1 - size / 2.0).max(c);
            let far = d.preview.is_none() && d.at.1 > g.reach() + 24.0 && item.pinned;
            self.draw_item(s, item, along, across, size, if far { 0.55 } else { 1.0 }, &pal, now);
            if far || d.unpin {
                self.draw_tip(s, "Remove", along + size / 2.0, across + size, &pal, 1.0);
            }
        }
    }

    /// The window dragging `key` out of the dock moves, when the app is open.
    fn moves(&self, key: &str) -> Option<Window> {
        self.items.iter().find(|i| i.key == key).and_then(|i| target_window(&i.windows))
    }

    /// Whether `key` is an installed app, and not just a window no app entry matches.
    fn installed(&self, key: &str) -> bool {
        self.items.iter().any(|i| i.key == key && i.entry.is_some())
    }

    fn draw_icon(&self, s: &gtk::Snapshot, item: &Item, x: f64, y: f64, w: f64, pal: &kit::palette::Palette) {
        match (self.art.get(&item.key), self.icons.get(&item.key).and_then(|p| p.as_ref())) {
            (Some(t), _) => s.append_scaled_texture(t, gsk::ScalingFilter::Trilinear, &graphene::Rect::new(x.round() as f32, y.round() as f32, w as f32, w as f32)),
            (None, Some(p)) => draw::paintable(s, p, x.round(), y.round(), w),
            (None, None) => draw::monogram(s, &self.paint, &self.app.cfg, &item.name, x, y, w, pal.raised, pal.ink_main),
        }
    }

    /// The icon of the dragged app as the dock draws it now, and where on it the pointer is.
    fn drag_icon(&self, drag: &Drag) -> Option<(gdk::Paintable, (i32, i32))> {
        let now = glib::monotonic_time();
        let i = self.items.iter().position(|i| i.key == drag.key)?;
        let size = self.layout(now).get(i)?.1.round();
        let s = gtk::Snapshot::new();
        self.draw_icon(&s, &self.items[i], 0.0, 0.0, size, &self.app.palette_at(now));
        let icon = s.to_paintable(Some(&graphene::Size::new(size as f32, size as f32)))?;
        let (a, c) = drag.grab;
        let (x, y) = match self.geo.side {
            Side::Bottom => (a, -c),
            Side::Left => (c, a),
            Side::Right => (-c, a),
        };
        let spot = |v: f64| (size / 2.0 + v).clamp(0.0, size).round() as i32;
        Some((icon, (spot(x), spot(y))))
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
        self.draw_icon(s, item, x, y, w, pal);
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

#[cfg(test)]
mod layout_tests {
    use super::*;

    const KEPT: (Kind, bool) = (Kind::App, true);
    const OPEN: (Kind, bool) = (Kind::App, false);

    fn items(apps: bool, kept: usize, open: usize, recent: bool, trash: bool) -> Vec<(Kind, bool)> {
        let mut out = Vec::new();
        out.extend(apps.then_some((Kind::Apps, false)));
        out.extend([KEPT].repeat(kept));
        out.extend([OPEN].repeat(open));
        out.extend(recent.then_some((Kind::Recent, false)));
        out.extend(trash.then_some((Kind::Trash, false)));
        out
    }

    #[test]
    fn every_part_of_the_dock_has_the_same_room_at_both_ends() {
        for (bits, base) in (0..16).flat_map(|b| [28.0, 50.0, 51.0, 96.0].map(|base| (b, base))) {
            let on = |k: i32| bits & (1 << k) != 0;
            for (kept, open) in [(0, 0), (0, 1), (0, 2), (1, 0), (3, 0), (3, 2)] {
                let list = items(on(0), kept, open, on(1), on(2));
                let g = Geo { side: Side::Bottom, base, mag: 1.5 };
                let cuts = splits(&list);
                let l = slots(g, list.len(), &cuts, if on(3) { 1920.0 } else { 1365.0 }, None, 0.0);
                let Some((start, len)) = span(g, &l) else { continue };
                assert_eq!(start.fract(), 0.0);
                for (i, (a, size)) in l.iter().enumerate() {
                    // What bounds it: the body's edge, a line or the next icon.
                    let before = if i == 0 { a - start } else if cuts.contains(&i) { g.pad() } else { a - (l[i - 1].0 + l[i - 1].1) };
                    let after = if i + 1 == l.len() { start + len - a - size } else { l[i + 1].0 - a - size };
                    let line = cuts.contains(&(i + 1));
                    assert_eq!(after, if i + 1 == l.len() { g.pad() } else if line { 2.0 * g.pad() + 1.0 } else { g.gap() });
                    assert_eq!(before, if i == 0 || cuts.contains(&i) { g.pad() } else { g.gap() });
                }
            }
        }
    }

    #[test]
    fn lines_stand_only_between_parts_that_hold_something() {
        assert_eq!(splits(&items(true, 2, 2, true, true)), [3, 5]);
        assert_eq!(splits(&items(true, 0, 2, true, true)), [1, 3]);
        assert_eq!(splits(&items(true, 0, 0, false, true)), [1]);
        assert_eq!(splits(&items(false, 0, 2, true, false)), [2]);
        assert_eq!(splits(&items(false, 2, 0, false, false)), [0usize; 0]);
        assert_eq!(splits(&items(false, 0, 0, true, true)), [0usize; 0]);
        assert_eq!(splits(&items(true, 0, 0, false, false)), [0usize; 0]);
        assert_eq!(splits(&[]), [0usize; 0]);
    }

    #[test]
    fn magnifying_keeps_the_dock_centered() {
        let g = Geo { side: Side::Bottom, base: 50.0, mag: 1.5 };
        let rest = span(g, &slots(g, 5, &[2], 1365.0, None, 0.0)).unwrap();
        let grown = span(g, &slots(g, 5, &[2], 1365.0, Some(690.0), 1.0)).unwrap();
        assert!(grown.1 > rest.1);
        assert!((rest.0 + rest.1 / 2.0 - grown.0 - grown.1 / 2.0).abs() < 1e-6);
    }

    #[test]
    fn the_first_app_is_kept_by_taking_it_in_front_of_the_open_ones() {
        let g = Geo { side: Side::Bottom, base: 50.0, mag: 1.0 };
        let first = (100.0, 50.0);
        // Past the line beside the Apps button.
        assert!(in_front(g, 85.0, 180.0, first, true));
        assert!(!in_front(g, 95.0, 180.0, first, true));
        // With no line, the first place's near half, but not by a small pull of the app there.
        assert!(in_front(g, 110.0, 180.0, first, false));
        assert!(!in_front(g, 130.0, 180.0, first, false));
        assert!(!in_front(g, 106.0, 115.0, first, false));
        assert!(in_front(g, 102.0, 115.0, first, false));
    }
}

#[cfg(test)]
mod shortcut_tests {
    use super::*;

    #[test]
    fn the_shortcut_hides_only_an_always_shown_dock_and_keeps_its_setting() {
        assert_eq!(shown_mode("on".into(), false), "on");
        assert_eq!(shown_mode("on".into(), true), "off");
        assert_eq!(shown_mode("autohide".into(), true), "autohide");
        assert_eq!(shown_mode("off".into(), true), "off");
        assert!(wanted("toggle", false) && !wanted("toggle", true));
        assert!(wanted("show", true) && !wanted("hide", true));
    }
}
