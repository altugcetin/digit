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

struct Item {
    key: String,
    name: String,
    entry: Option<usize>,
    pinned: bool,
    windows: Vec<(u64, u64, bool)>,
    badge: usize,
}

impl Item {
    fn focused(&self) -> bool {
        self.windows.iter().any(|w| w.2)
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
    style: crate::art::Style,
    art: HashMap<String, gdk::Texture>,
    items: Vec<Item>,
    pins: Vec<String>,
    icons: HashMap<String, Option<gdk::Paintable>>,
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

fn monitor_for(output: Option<&str>) -> Option<gdk::Monitor> {
    let monitors: Vec<gdk::Monitor> = gdk::Display::default()?.monitors().iter::<gdk::Monitor>().filter_map(Result::ok).collect();
    monitors
        .iter()
        .find(|m| output.is_some_and(|o| m.connector().as_deref() == Some(o)))
        .or_else(|| monitors.iter().find(|m| m.geometry().x() == 0 && m.geometry().y() == 0))
        .or(monitors.first())
        .cloned()
}

fn teardown(st: &Shared) {
    close_menu();
    let (keep, window, paint, triggers, edge) = {
        let mut s = st.borrow_mut();
        (s.keep.clone(), s.window.clone(), s.paint.clone(), std::mem::take(&mut s.triggers), s.edge.take())
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
    let check = move |x: f64| {
        let far = probe.width() as f64 - 3.0;
        if side == Side::Bottom && x >= far {
            if let Some(app) = corner_app.upgrade() {
                if app.prefs.borrow().control_corner {
                    crate::control::show_on(&app, Some(corner_out.clone()));
                    return true;
                }
            }
        }
        false
    };
    let near = check.clone();
    motion.connect_motion(move |_, x, _| {
        near(x);
    });
    motion.connect_enter(move |_, x, _| {
        if check(x) {
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
    w.present();
    w
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
    let monitor = monitor_for(prefs.output.as_deref().or(prefer.as_deref()));
    let output = monitor.as_ref().and_then(|m| m.connector()).map(|c| c.to_string()).unwrap_or_default();
    let old = current();
    let Some(mode) = mode else {
        if let Some(st) = old {
            teardown(&st);
            ONE.with(|one| one.borrow_mut().take());
            App::trim();
        }
        return;
    };
    let layer = if mode == Mode::On { Layer::Top } else { Layer::Overlay };
    let reuse = old.filter(|st| {
        let s = st.borrow();
        s.geo.side == geo.side && s.output == output && s.window.layer() == layer
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
                let others: Vec<gdk::Monitor> = gdk::Display::default()
                    .map(|d| d.monitors().iter::<gdk::Monitor>().filter_map(Result::ok).filter(|m| m.connector().as_deref() != Some(output.as_str())).collect())
                    .unwrap_or_default();
                let triggers: Vec<gtk::Window> = others.iter().map(|m| trigger(app, m, geo.side)).collect();
                st.borrow_mut().triggers = triggers;
            }
            ONE.with(|one| *one.borrow_mut() = Some(st.clone()));
            st
        }
    };
    {
        let mut s = st.borrow_mut();
        if s.geo != geo {
            s.paint.set_size_request(if geo.side == Side::Bottom { 1 } else { geo.depth() as i32 }, if geo.side == Side::Bottom { geo.depth() as i32 } else { 1 });
        }
        s.geo = geo;
        s.mode = mode;
        s.running = prefs.running;
        s.badges = prefs.badges;
        s.style = crate::art::Style::parse(&prefs.icons);
        s.icons.clear();
        s.art.clear();
        crate::art::forget();
    }
    rebuild(&st);
    settle(&st);
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
        style: crate::art::Style::Light,
        art: HashMap::new(),
        items: Vec::new(),
        pins: load_pins(app),
        icons: HashMap::new(),
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
        reserved: -1,
        triggers: Vec::new(),
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
            let (a, _) = s.local(x, y);
            s.index_at(a, glib::monotonic_time())
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
    let weak = Rc::downgrade(st);
    window.connect_map(move |_| {
        if let Some(st) = weak.upgrade() {
            st.borrow_mut().region = None;
            region(&st);
            let weak = Rc::downgrade(&st);
            glib::idle_add_local_once(move || {
                if let Some(st) = weak.upgrade() {
                    paint_art(&st);
                    st.borrow().paint.queue_draw();
                }
            });
        }
    });
}

fn rebuild(st: &Shared) {
    let mut s = st.borrow_mut();
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
            Some(Item { key: key.clone(), name: list[idx].name.clone(), entry: Some(idx), pinned: true, windows: Vec::new(), badge: 0 })
        })
        .collect();
    for r in running.iter().filter(|r| !r.app_id.is_empty()) {
        let idx = app.apps.find(&r.app_id);
        let key = idx.map(|i| list[i].id.clone()).unwrap_or_else(|| r.app_id.clone());
        match items.iter_mut().find(|it| it.key == key) {
            Some(it) => it.windows.push((r.id, r.stamp, r.focused)),
            None if s.running => items.push(Item {
                name: idx.map(|i| list[i].name.clone()).unwrap_or_else(|| crate::apps::pretty(&r.app_id)),
                key,
                entry: idx,
                pinned: false,
                windows: vec![(r.id, r.stamp, r.focused)],
                badge: 0,
            }),
            None => {}
        }
    }
    for it in items.iter_mut() {
        it.badge = badges.get(&it.name).copied().unwrap_or(0);
    }
    let scale = s.window.scale_factor().max(1);
    let px = s.geo.icon_px();
    for it in &items {
        if !s.icons.contains_key(&it.key) {
            let icon = it.entry.and_then(|i| list.get(i)?.icon.clone()).and_then(|g| {
                let display = gdk::Display::default()?;
                Some(gtk::IconTheme::for_display(&display).lookup_by_gicon(&g, px, scale, gtk::TextDirection::Ltr, gtk::IconLookupFlags::empty()).upcast::<gdk::Paintable>())
            });
            let icon = icon.or_else(|| draw::app_icon(&app.apps, &it.key, px, scale));
            s.icons.insert(it.key.clone(), icon);
        }
    }
    let live: Vec<String> = items.iter().filter(|i| !i.windows.is_empty()).map(|i| i.key.clone()).collect();
    s.bounce.retain(|(k, t)| !live.contains(k) && now - t < 4 * BOUNCE);
    if s.icons.len() > 64 {
        let keep: Vec<String> = items.iter().map(|i| i.key.clone()).collect();
        s.icons.retain(|k, _| keep.contains(k));
    }
    if s.hover.is_some_and(|h| h >= items.len()) {
        s.hover = None;
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
    st.borrow().paint.queue_draw();
}

fn paint_art(st: &Shared) {
    let mut s = st.borrow_mut();
    if s.paint.native().and_then(|n| n.renderer()).is_none() {
        return;
    }
    let px = s.geo.icon_px() as f64;
    let accent = s.app.palette.get().accent;
    let style = s.style;
    let paint = s.paint.clone();
    let app = s.app.clone();
    let wanted: Vec<(String, String)> = s.items.iter().filter(|i| !s.art.contains_key(&i.key)).map(|i| (i.key.clone(), i.name.clone())).collect();
    for (key, name) in wanted {
        let icon = s.icons.get(&key).cloned().flatten();
        if matches!(style, crate::art::Style::Light | crate::art::Style::Graphite) && themed(icon.as_ref()) {
            continue;
        }
        if let Some(t) = crate::art::texture(&paint, &key, icon.as_ref(), &name, px, style, accent, &app.cfg) {
            s.art.insert(key, t);
        }
    }
    let keep: Vec<String> = s.items.iter().map(|i| i.key.clone()).collect();
    s.art.retain(|k, _| keep.contains(k));
}

fn wants(s: &State) -> bool {
    if s.reveal || s.menu || s.drag.is_some() {
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

fn themed(icon: Option<&gdk::Paintable>) -> bool {
    let Some(theme) = gtk::Settings::default().and_then(|s| s.gtk_icon_theme_name()) else { return false };
    let Some(path) = icon.and_then(|p| p.downcast_ref::<gtk::IconPaintable>()).and_then(|p| p.file()).and_then(|f| f.path()) else { return false };
    path.to_string_lossy().contains(&format!("/icons/{theme}/"))
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
        s.pointer = None;
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
        let pinned = s.items.iter().filter(|i| i.pinned).count();
        let target = s.index_at(p.0, now).or_else(|| {
            let l = s.layout(now);
            let first = l.first().map_or(0.0, |f| f.0);
            Some(if p.0 < first { 0 } else { l.len().saturating_sub(1) })
        });
        let from = s.pins.iter().position(|k| *k == key);
        match (target, from) {
            (Some(t), Some(f)) if t != f && t < pinned => {
                let k = s.pins.remove(f);
                let at = t.min(s.pins.len());
                s.pins.insert(at, k);
                true
            }
            (Some(t), None) if t < pinned => {
                let at = t.min(s.pins.len());
                s.pins.insert(at, key);
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
        let far = d.at.1 > s.geo.reach() + 24.0;
        let name = s.items.iter().find(|i| i.key == d.key).map(|i| i.name.clone()).unwrap_or_default();
        let note = if far && s.pins.contains(&d.key) {
            s.pins.retain(|k| *k != d.key);
            Some(format!("{name} removed from Dock"))
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
    let (app, windows, entry, key) = {
        let s = st.borrow();
        let Some(it) = s.items.get(i) else { return };
        (s.app.clone(), it.windows.clone(), it.entry, it.key.clone())
    };
    if windows.is_empty() {
        let list = app.apps.list();
        if let Some(e) = entry.and_then(|idx| list.get(idx)) {
            app.apps.launch(e);
            st.borrow_mut().bounce.push((key, glib::monotonic_time()));
            kick(st);
        }
        return;
    }
    let mut ids = windows;
    ids.sort_by_key(|w| w.0);
    let target = match ids.iter().position(|w| w.2) {
        Some(p) => ids[(p + 1) % ids.len()].0,
        None => ids.iter().max_by_key(|w| w.1).map_or(ids[0].0, |w| w.0),
    };
    glib::spawn_future_local(async move {
        let _ = gio::spawn_blocking(move || compositor::focus(target)).await;
    });
}

fn launch(st: &Shared, i: usize) {
    let (app, entry, key) = {
        let s = st.borrow();
        let Some(it) = s.items.get(i) else { return };
        (s.app.clone(), it.entry, it.key.clone())
    };
    let list = app.apps.list();
    if let Some(e) = entry.and_then(|idx| list.get(idx)) {
        app.apps.launch(e);
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
        let bouncing = s.bounce.iter().any(|(_, t)| now - t < 3 * BOUNCE);
        let busy = [&s.magnify, &s.shown, &s.tip].iter().any(|t| !t.settled(now))
            || s.slide.values().chain(s.appear.values()).any(|t| !t.settled(now))
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

    fn separator(&self) -> Option<usize> {
        let pinned = self.items.iter().filter(|i| i.pinned).count();
        (pinned > 0 && pinned < self.items.len()).then_some(pinned)
    }

    fn layout(&self, now: i64) -> Vec<(f64, f64)> {
        let n = self.items.len();
        if n == 0 {
            return Vec::new();
        }
        let g = self.geo;
        let split = self.separator();
        let sep = if split.is_some() { g.gap() * 2.0 + 1.0 } else { 0.0 };
        let total0 = n as f64 * g.base + (n - 1) as f64 * g.gap() + sep;
        let start0 = self.along_len() / 2.0 - total0 / 2.0;
        let m = if g.mag > 1.0 { self.magnify.value(now).clamp(0.0, 1.0) } else { 0.0 };
        let pa = self.pointer.map_or(-1e4, |p| p.0);
        let mut sizes = Vec::with_capacity(n);
        for i in 0..n {
            let extra = if split.is_some_and(|k| i >= k) { sep } else { 0.0 };
            let c = start0 + i as f64 * (g.base + g.gap()) + extra + g.base / 2.0;
            let d = (pa - c) / g.spread();
            sizes.push(g.base * (1.0 + (g.mag - 1.0) * m * (-d * d).exp()));
        }
        let total = sizes.iter().sum::<f64>() + (n - 1) as f64 * g.gap() + sep;
        let mut a = self.along_len() / 2.0 - total / 2.0;
        let mut out = Vec::with_capacity(n);
        for (i, size) in sizes.into_iter().enumerate() {
            if split == Some(i) {
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
        let sep = if self.separator().is_some() { g.gap() * 2.0 + 1.0 } else { 0.0 };
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
        if let Some(k) = self.separator() {
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
                draw::fill(s, &draw::round(x, y, w, h, r), if item.focused() { pal.ink_main } else { pal.ink_meta.alpha(0.7) });
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
        let shown = size * (0.4 + 0.6 * grow);
        let (a, c) = (a + (size - shown) / 2.0, c + (size - shown) / 2.0);
        let (x, y, w, _) = self.map(a, c, shown, shown);
        let alpha = alpha * grow;
        if alpha < 0.999 {
            s.push_opacity(alpha);
        }
        match (self.art.get(&item.key), self.icons.get(&item.key).and_then(|p| p.as_ref())) {
            (Some(t), _) => s.append_scaled_texture(t, gsk::ScalingFilter::Trilinear, &graphene::Rect::new(x.round() as f32, y.round() as f32, w as f32, w as f32)),
            (None, Some(p)) => draw::paintable(s, p, x.round(), y.round(), w),
            (None, None) => draw::monogram(s, &self.paint, &self.app.cfg, &item.name, x, y, w, pal.raised, pal.ink_main),
        }
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
                let a = (center - bw / 2.0).clamp(4.0, self.along_len() - bw - 4.0);
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
    Focus(u64),
    Launch,
    Pin,
    Quit(Vec<u64>),
    Hide,
}

struct Menu {
    dock: std::rc::Weak<RefCell<State>>,
    window: gtk::Window,
    paint: Paint,
    title: String,
    key: String,
    index: usize,
    rows: Vec<(String, Act, bool)>,
    hover: Option<usize>,
    fade: Tween,
    seen: bool,
}

fn close_menu() {
    let Some(m) = MENU.with(|m| m.borrow_mut().take()) else { return };
    let (window, dock) = {
        let m = m.borrow();
        (m.window.clone(), m.dock.upgrade())
    };
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
        let mut ids = item.windows.clone();
        ids.sort_by_key(|w| w.0);
        for (id, _, focused) in ids.iter().take(6) {
            let t = titles.get(id).cloned().filter(|t| !t.is_empty()).unwrap_or_else(|| item.name.clone());
            rows.push((if *focused { format!("● {t}") } else { t }, Act::Focus(*id), false));
        }
        if item.entry.is_some() {
            rows.push((if item.windows.is_empty() { "Open".into() } else { "New Window".into() }, Act::Launch, false));
            rows.push((if item.pinned { "Remove from Dock".into() } else { "Keep in Dock".into() }, Act::Pin, false));
        }
        rows.push(("Hide Dock".into(), Act::Hide, false));
        if !item.windows.is_empty() {
            rows.push(("Quit".into(), Act::Quit(item.windows.iter().map(|w| w.0).collect()), true));
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
            Side::Bottom => (Edge::Bottom, (center - MENU_W / 2.0).clamp(8.0, s.along_len() - MENU_W - 8.0)),
            Side::Left => (Edge::Left, (center - h / 2.0).clamp(8.0, s.along_len() - h - 8.0)),
            Side::Right => (Edge::Right, (center - h / 2.0).clamp(8.0, s.along_len() - h - 8.0)),
        };
        (rows, item.name.clone(), item.key.clone(), s.window.monitor(), anchor, gap, main, h)
    };
    let window = gtk::Window::new();
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace(Some("digit-dock-menu"));
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(KeyboardMode::OnDemand);
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
    let menu = Rc::new(RefCell::new(Menu { dock: Rc::downgrade(st), window: window.clone(), paint: paint.clone(), title, key, index: i, rows, hover: None, fade, seen: false }));
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
    let weak = Rc::downgrade(&menu);
    window.connect_is_active_notify(move |w| {
        let Some(m) = weak.upgrade() else { return };
        let Ok(mut mm) = m.try_borrow_mut() else { return };
        if w.is_active() {
            mm.seen = true;
        } else if mm.seen {
            drop(mm);
            glib::idle_add_local_once(close_menu);
        }
    });
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
    let (dock, act, key, index) = {
        let m = m.borrow();
        let Some(row) = m.rows.get(r) else { return };
        (m.dock.upgrade(), row.1.clone(), m.key.clone(), m.index)
    };
    close_menu();
    let Some(st) = dock else { return };
    match act {
        Act::Focus(id) => {
            glib::spawn_future_local(async move {
                let _ = gio::spawn_blocking(move || compositor::focus(id)).await;
            });
        }
        Act::Launch => launch(&st, index),
        Act::Pin => toggle_pin(&st, &key),
        Act::Hide => {
            let app = st.borrow().app.clone();
            command(&app, "hide");
        }
        Act::Quit(ids) => {
            let app = st.borrow().app.clone();
            crate::quit::quit(&app, &key, ids);
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
