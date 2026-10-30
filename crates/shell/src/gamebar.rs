use crate::app::App;
use crate::audio;
use crate::compositor;
use crate::draw::{self, Face};
use crate::gsr::{self, Mode};
use crate::paint::Paint;
use gtk::{cairo, gdk, gio, glib, graphene, pango, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kit::bluez;
use kit::motion::{Curve, Tween};
use kit::palette::{Palette, Rgb};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

mod cards;

use cards::{Hit, Rec};

const OPEN: Curve = Curve::new(0.16, 1.0, 0.3, 1.0);
const CLOSE: Curve = Curve::new(0.4, 0.0, 0.9, 0.6);
const IN_MS: i64 = 420;
const OUT_MS: i64 = 240;
/// Long enough for the close motion to finish, so the overlay is never in the first frames.
const CLEAR_MS: u64 = 280;
const HEAD: f64 = 46.0;
const PAD: f64 = 16.0;
const RADIUS: f64 = 26.0;
const BAR_TOP: f64 = 16.0;
const BAR_H: f64 = 52.0;
const SLOT: f64 = 40.0;
const MARGIN: f64 = 24.0;
const RED: Rgb = Rgb::new(1.0, 0.27, 0.23);

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Recorder,
    Resources,
    Mixer,
    Media,
    Bluetooth,
    Notes,
    Crosshair,
}

const KINDS: [Kind; 7] = [Kind::Recorder, Kind::Resources, Kind::Mixer, Kind::Media, Kind::Bluetooth, Kind::Notes, Kind::Crosshair];

impl Kind {
    fn title(self) -> &'static str {
        match self {
            Kind::Recorder => "Recorder",
            Kind::Resources => "Resources",
            Kind::Mixer => "Volume Mixer",
            Kind::Media => "Media",
            Kind::Bluetooth => "Bluetooth",
            Kind::Notes => "Notes",
            Kind::Crosshair => "Crosshair",
        }
    }

    fn glyph(self) -> &'static str {
        match self {
            Kind::Recorder => "screen_record",
            Kind::Resources => "monitoring",
            Kind::Mixer => "instant_mix",
            Kind::Media => "music_note",
            Kind::Bluetooth => "bluetooth",
            Kind::Notes => "sticky_note_2",
            Kind::Crosshair => "my_location",
        }
    }

    fn width(self) -> f64 {
        match self {
            Kind::Resources => 360.0,
            Kind::Mixer | Kind::Media | Kind::Crosshair => 340.0,
            Kind::Bluetooth => 300.0,
            Kind::Recorder | Kind::Notes => 320.0,
        }
    }

    /// The narrowest a card can be dragged before its content stops fitting.
    fn min_width(self) -> f64 {
        match self {
            Kind::Notes => 220.0,
            Kind::Bluetooth => 260.0,
            Kind::Resources | Kind::Mixer => 300.0,
            Kind::Media => 310.0,
            Kind::Recorder => 320.0,
            Kind::Crosshair => 340.0,
        }
    }

    /// Whether the card has a use for more height (the notes field, the resource graph).
    fn stretches(self) -> bool {
        matches!(self, Kind::Notes | Kind::Resources)
    }
}

/// How close to a card's edge a press resizes it instead of moving or clicking.
const GRIP: f64 = 6.0;

#[derive(Clone, Copy, Serialize, Deserialize)]
struct Spot {
    kind: Kind,
    x: f64,
    y: f64,
    open: bool,
    pinned: bool,
    /// The size the user dragged the card to; 0 keeps the natural size.
    #[serde(default)]
    w: f64,
    #[serde(default)]
    h: f64,
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Aim {
    pub on: bool,
    /// 0 dot, 1 cross.
    pub style: u8,
    /// 0 small, 1 medium, 2 large.
    pub size: u8,
    pub color: u8,
}

impl Default for Aim {
    fn default() -> Self {
        Aim { on: false, style: 1, size: 1, color: 0 }
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    cards: Vec<Spot>,
    crosshair: Aim,
}

struct Card {
    kind: Kind,
    /// Top-left on the screen; negative until the card is first placed.
    x: f64,
    y: f64,
    open: bool,
    pinned: bool,
    w: f64,
    h: f64,
    vis: Tween,
}

enum Drag {
    Move { kind: Kind, dx: f64, dy: f64 },
    /// `from` is the pointer where the drag began, `rect` the card's rectangle then.
    Resize { kind: Kind, edges: u8, from: (f64, f64), rect: (f64, f64, f64, f64) },
    Slide { row: usize, x0: f64, w: f64 },
}

struct State {
    app: Rc<App>,
    window: gtk::Window,
    paint: Paint,
    notes: gtk::TextView,
    scroller: gtk::ScrolledWindow,
    css: gtk::CssProvider,
    open: bool,
    rise: Tween,
    cards: Vec<Card>,
    aim: Aim,
    hot: Option<Hit>,
    drag: Option<Drag>,
    dragged: bool,
    ticking: bool,
    pulsing: bool,
    saving: Option<glib::SourceId>,
    data: cards::Data,
}

type Shared = Rc<RefCell<State>>;

thread_local! {
    static ONE: RefCell<Option<Shared>> = const { RefCell::new(None) };
}

fn get(app: &Rc<App>) -> Shared {
    ONE.with(|one| one.borrow_mut().get_or_insert_with(|| build(app)).clone())
}

fn current() -> Option<Shared> {
    ONE.with(|one| one.borrow().clone())
}

fn file() -> std::path::PathBuf {
    kit::theme::state_dir().join("gamebar.json")
}

fn notes_file() -> std::path::PathBuf {
    kit::theme::state_dir().join("notes.txt")
}

fn load() -> Saved {
    let mut saved: Saved = std::fs::read_to_string(file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let fresh = saved.cards.is_empty();
    for kind in KINDS {
        if !saved.cards.iter().any(|c| c.kind == kind) {
            saved.cards.push(Spot { kind, x: -1.0, y: -1.0, open: fresh && kind != Kind::Crosshair, pinned: false, w: 0.0, h: 0.0 });
        }
    }
    saved
}

pub fn toggle(app: &Rc<App>) {
    let st = get(app);
    let open = st.borrow().open;
    if open { close(&st) } else { show(&st) }
}

pub fn open(app: &Rc<App>) {
    let st = get(app);
    if !st.borrow().open {
        show(&st);
    }
}

pub fn close_now() {
    if let Some(st) = current() {
        close(&st);
    }
}

pub fn is_open() -> bool {
    current().is_some_and(|st| st.try_borrow().is_ok_and(|s| s.open))
}

/// Brings back pinned cards and the crosshair after the shell starts.
pub fn restore(app: &Rc<App>) {
    let saved = load();
    if saved.crosshair.on || saved.cards.iter().any(|c| c.open && c.pinned) {
        let st = get(app);
        present(&st);
        region(&st);
        pulse(&st);
    }
}

/// The recorder changed state: redraw so the Recorder card follows it.
pub fn changed() {
    if let Some(st) = current() {
        if let Ok(s) = st.try_borrow() {
            s.paint.queue_draw();
        }
    }
}

/// Closes the overlay, then runs `then` once it is off the screen.
pub fn after_close(then: impl FnOnce() + 'static) {
    match current() {
        Some(st) if st.borrow().open => {
            close(&st);
            glib::timeout_add_local_once(Duration::from_millis(CLEAR_MS), then);
        }
        _ => then(),
    }
}

fn build(app: &Rc<App>) -> Shared {
    let window = gtk::Window::new();
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace(Some("digit-gamebar"));
    for e in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(e, true);
    }
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(KeyboardMode::None);
    window.add_css_class("digit");
    let paint = Paint::new(-1, -1);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&paint));
    let notes = gtk::TextView::new();
    notes.set_wrap_mode(gtk::WrapMode::WordChar);
    notes.add_css_class("digit-notes");
    notes.buffer().set_text(&std::fs::read_to_string(notes_file()).unwrap_or_default());
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroller.set_child(Some(&notes));
    scroller.add_css_class("digit-notes");
    scroller.set_halign(gtk::Align::Start);
    scroller.set_valign(gtk::Align::Start);
    scroller.set_visible(false);
    overlay.add_overlay(&scroller);
    window.set_child(Some(&overlay));
    let css = gtk::CssProvider::new();
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
    }
    let saved = load();
    let st = Rc::new(RefCell::new(State {
        app: app.clone(),
        window: window.clone(),
        paint: paint.clone(),
        notes: notes.clone(),
        scroller,
        css,
        open: false,
        rise: Tween::still(0.0),
        cards: saved.cards.iter().map(|c| Card { kind: c.kind, x: c.x, y: c.y, open: c.open, pinned: c.pinned, w: c.w, h: c.h, vis: Tween::still(if c.open { 1.0 } else { 0.0 }) }).collect(),
        aim: saved.crosshair,
        hot: None,
        drag: None,
        dragged: false,
        ticking: false,
        pulsing: false,
        saving: None,
        data: cards::Data::default(),
    }));
    let weak = Rc::downgrade(&st);
    paint.on_draw(move |s| {
        if let Some(st) = weak.upgrade() {
            if let Ok(st) = st.try_borrow() {
                st.draw(s);
            }
        }
    });
    let weak = Rc::downgrade(&st);
    notes.buffer().connect_changed(move |_| {
        let Some(st) = weak.upgrade() else { return };
        let Ok(mut s) = st.try_borrow_mut() else { return };
        if let Some(id) = s.saving.take() {
            id.remove();
        }
        let weak = Rc::downgrade(&st);
        s.saving = Some(glib::timeout_add_local_once(Duration::from_millis(500), move || {
            let Some(st) = weak.upgrade() else { return };
            st.borrow_mut().saving = None;
            save_notes(&st.borrow());
        }));
        s.paint.queue_draw();
    });
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(&st);
    keys.connect_key_pressed(move |_, key, _, _| {
        let Some(st) = weak.upgrade() else { return glib::Propagation::Proceed };
        if key != gdk::Key::Escape {
            return glib::Propagation::Proceed;
        }
        let open = st.borrow().open;
        if open {
            close(&st);
        } else {
            let window = st.borrow().window.clone();
            gtk::prelude::GtkWindowExt::set_focus(&window, None::<&gtk::Widget>);
        }
        glib::Propagation::Stop
    });
    window.add_controller(keys);
    let motion = gtk::EventControllerMotion::new();
    let weak = Rc::downgrade(&st);
    motion.connect_motion(move |_, x, y| {
        let Some(st) = weak.upgrade() else { return };
        hover(&st, x, y);
    });
    paint.add_controller(motion);
    let click = gtk::GestureClick::new();
    let weak = Rc::downgrade(&st);
    click.connect_released(move |_, _, x, y| {
        let Some(st) = weak.upgrade() else { return };
        if std::mem::take(&mut st.borrow_mut().dragged) {
            return;
        }
        let (at, open) = {
            let s = st.borrow();
            (s.hit_at(x, y), s.open)
        };
        match at {
            Some(hit) => press(&st, hit),
            None if open => close(&st),
            None => {}
        }
    });
    paint.add_controller(click);
    let drag = gtk::GestureDrag::new();
    let weak = Rc::downgrade(&st);
    drag.connect_drag_begin(move |_, x, y| {
        let Some(st) = weak.upgrade() else { return };
        drag_begin(&st, x, y);
    });
    let weak = Rc::downgrade(&st);
    drag.connect_drag_update(move |g, dx, dy| {
        let Some(st) = weak.upgrade() else { return };
        let Some((x0, y0)) = g.start_point() else { return };
        drag_to(&st, x0 + dx, y0 + dy, dx.hypot(dy));
    });
    let weak = Rc::downgrade(&st);
    drag.connect_drag_end(move |_, _, _| {
        let Some(st) = weak.upgrade() else { return };
        drag_end(&st);
    });
    paint.add_controller(drag);
    st
}

fn monitor() -> Option<gdk::Monitor> {
    let output = compositor::focused_output();
    let monitors: Vec<gdk::Monitor> = gdk::Display::default().map(|d| d.monitors().iter::<gdk::Monitor>().filter_map(Result::ok).collect()).unwrap_or_default();
    monitors.iter().find(|m| output.as_deref().is_some_and(|o| m.connector().as_deref() == Some(o))).or(monitors.first()).cloned()
}

fn present(st: &Shared) {
    let window = st.borrow().window.clone();
    if !window.is_visible() {
        window.set_monitor(monitor().as_ref());
    }
    restyle(&st.borrow());
    window.present();
    kick(st);
}

fn restyle(s: &State) {
    let pal = s.app.palette_at(App::now());
    s.css.load_from_string(&format!(
        "scrolledwindow.digit-notes, scrolledwindow.digit-notes > viewport, textview.digit-notes, textview.digit-notes > text {{ background: transparent; color: {}; caret-color: {}; font-family: \"{}\"; font-size: 14px; }} textview.digit-notes > text > selection {{ background: {}; }}",
        draw::css(pal.ink_main),
        draw::css(pal.accent),
        s.app.cfg.font,
        draw::css(pal.accent.alpha(0.32)),
    ));
}

fn show(st: &Shared) {
    {
        let mut s = st.borrow_mut();
        s.open = true;
        s.hot = None;
        let now = glib::monotonic_time();
        s.rise.go(1.0, now, IN_MS, OPEN);
        s.window.set_keyboard_mode(KeyboardMode::OnDemand);
    }
    present(st);
    region(st);
    pulse(st);
    refresh(st, true);
}

fn close(st: &Shared) {
    {
        let mut s = st.borrow_mut();
        if !s.open {
            return;
        }
        s.open = false;
        s.drag = None;
        let now = glib::monotonic_time();
        s.rise.go(0.0, now, OUT_MS, CLOSE);
        s.paint.set_cursor_from_name(None);
        gtk::prelude::GtkWindowExt::set_focus(&s.window, None::<&gtk::Widget>);
        s.save();
    }
    keyboard(st);
    region(st);
    kick(st);
}

/// On demand rather than exclusive, so the compositor's own shortcuts (the one that closes the
/// overlay too) keep working. With the overlay closed, a pinned Notes card can still take typing.
fn keyboard(st: &Shared) {
    let s = st.borrow();
    let mode = if s.open || s.cards.iter().any(|c| c.kind == Kind::Notes && c.open && c.pinned) {
        KeyboardMode::OnDemand
    } else {
        KeyboardMode::None
    };
    s.window.set_keyboard_mode(mode);
}

/// Open: the whole screen takes input. Closed: only the pinned cards do.
fn region(st: &Shared) {
    let s = st.borrow();
    let Some(surface) = s.window.surface() else { return };
    if s.open {
        surface.set_input_region(None);
        return;
    }
    let r = cairo::Region::create();
    for c in s.cards.iter().filter(|c| c.open && c.pinned) {
        let (x, y, w, h) = s.rect(c);
        let _ = r.union_rectangle(&cairo::RectangleInt::new(x as i32, y as i32, w.ceil() as i32, h.ceil() as i32));
    }
    surface.set_input_region(Some(&r));
}

fn save_notes(s: &State) {
    let b = s.notes.buffer();
    let text = b.text(&b.start_iter(), &b.end_iter(), false);
    let _ = std::fs::create_dir_all(kit::theme::state_dir());
    let _ = std::fs::write(notes_file(), text.as_str());
}

/// Runs once a second while cards are on screen: samples, rereads the mixer and moves the clocks.
fn pulse(st: &Shared) {
    if std::mem::replace(&mut st.borrow_mut().pulsing, true) {
        return;
    }
    tick(st);
    let weak = Rc::downgrade(st);
    glib::timeout_add_local(Duration::from_millis(1000), move || {
        let Some(st) = weak.upgrade() else { return glib::ControlFlow::Break };
        let live = st.try_borrow().map(|s| s.window.is_visible() && (s.open || s.cards.iter().any(|c| c.open && c.pinned))).unwrap_or(true);
        if !live {
            st.borrow_mut().pulsing = false;
            return glib::ControlFlow::Break;
        }
        tick(&st);
        glib::ControlFlow::Continue
    });
}

fn tick(st: &Shared) {
    let Ok(mut s) = st.try_borrow_mut() else { return };
    s.data.beat += 1;
    if s.shown(Kind::Resources) {
        let mut stats = s.data.sampler.sample();
        if stats.gpu.is_none() {
            stats.gpu = s.data.nvidia;
        }
        s.data.push(stats);
        if s.data.beat % 2 == 0 && s.data.sampler.wants_nvidia() {
            let weak = Rc::downgrade(st);
            glib::spawn_future_local(async move {
                let gpu = crate::sysmon::nvidia().await;
                if let Some(st) = weak.upgrade() {
                    st.borrow_mut().data.nvidia = gpu;
                }
            });
        }
    }
    let beat = s.data.beat;
    s.paint.queue_draw();
    drop(s);
    refresh(st, beat % 3 == 0);
}

/// Rereads the mixer, and Bluetooth too when `all`.
fn refresh(st: &Shared, all: bool) {
    let (mixer, bt) = {
        let s = st.borrow();
        (s.shown(Kind::Mixer) && s.drag.is_none(), all && s.shown(Kind::Bluetooth))
    };
    if mixer {
        refresh_mix(st);
    }
    if bt {
        refresh_bt(st);
    }
}

fn refresh_mix(st: &Shared) {
    let input = {
        let mut s = st.borrow_mut();
        if s.data.mixing || s.data.sliding.is_some() {
            return;
        }
        s.data.mixing = true;
        s.data.mix_tab == 1
    };
    let weak = Rc::downgrade(st);
    glib::spawn_future_local(async move {
        let mix = gio::spawn_blocking(move || audio::mix(input)).await.ok().flatten();
        let Some(st) = weak.upgrade() else { return };
        {
            let mut s = st.borrow_mut();
            s.data.mixing = false;
            if (s.data.mix_tab == 1) != input || s.data.sliding.is_some() || s.data.mix == mix {
                return;
            }
            s.data.mix = mix;
            s.paint.queue_draw();
        }
        region(&st);
    });
}

fn refresh_bt(st: &Shared) {
    if std::mem::replace(&mut st.borrow_mut().data.bt_reading, true) {
        return;
    }
    let weak = Rc::downgrade(st);
    glib::spawn_future_local(async move {
        let snap = match crate::bus::system().await {
            Ok(conn) => bluez::read(&conn).await,
            Err(_) => bluez::Snapshot::default(),
        };
        let Some(st) = weak.upgrade() else { return };
        {
            let mut s = st.borrow_mut();
            s.data.bt_reading = false;
            if s.data.bt == snap {
                return;
            }
            s.data.bt = snap;
            s.paint.queue_draw();
        }
        region(&st);
    });
}

fn hover(st: &Shared, x: f64, y: f64) {
    let mut s = st.borrow_mut();
    if s.drag.is_some() {
        return;
    }
    let at = s.hit_at(x, y);
    if s.hot == at {
        return;
    }
    s.hot = at;
    let cursor = match at {
        Some(Hit::Head(_)) => Some("grab"),
        Some(Hit::Resize(_, e)) => Some(match e {
            1 | 2 => "ew-resize",
            4 | 8 => "ns-resize",
            5 | 10 => "nwse-resize",
            _ => "nesw-resize",
        }),
        Some(Hit::Body(_) | Hit::Bare) | None => None,
        Some(_) => Some("pointer"),
    };
    s.paint.set_cursor_from_name(cursor);
    s.paint.queue_draw();
}

fn raise(s: &mut State, kind: Kind) {
    if let Some(i) = s.cards.iter().position(|c| c.kind == kind) {
        let card = s.cards.remove(i);
        s.cards.push(card);
    }
}

fn drag_begin(st: &Shared, x: f64, y: f64) {
    let mut s = st.borrow_mut();
    s.dragged = false;
    let Some(hit) = s.hit_at(x, y) else { return };
    match hit {
        Hit::Head(kind) | Hit::Body(kind) => {
            raise(&mut s, kind);
            if matches!(hit, Hit::Head(_)) {
                let Some(c) = s.cards.iter().find(|c| c.kind == kind) else { return };
                let (cx, cy, _, _) = s.rect(c);
                s.drag = Some(Drag::Move { kind, dx: x - cx, dy: y - cy });
                s.paint.set_cursor_from_name(Some("grabbing"));
            }
            s.paint.queue_draw();
        }
        Hit::Resize(kind, edges) => {
            raise(&mut s, kind);
            let Some(c) = s.cards.iter().find(|c| c.kind == kind) else { return };
            let rect = s.rect(c);
            s.drag = Some(Drag::Resize { kind, edges, from: (x, y), rect });
        }
        Hit::Level(row) => {
            let Some(c) = s.cards.iter().find(|c| c.kind == Kind::Mixer) else { return };
            let (cx, _, w, _) = s.rect(c);
            let (x0, sw) = cards::slider(w);
            s.drag = Some(Drag::Slide { row, x0: cx + x0, w: sw });
            drop(s);
            drag_to(st, x, y, 0.0);
        }
        _ => {}
    }
}

fn drag_to(st: &Shared, x: f64, y: f64, moved: f64) {
    let mut s = st.borrow_mut();
    if moved > 4.0 && s.drag.is_some() {
        s.dragged = true;
    }
    match s.drag {
        Some(Drag::Move { kind, dx, dy }) => {
            let (sw, sh) = s.size();
            let Some((w, h)) = s.cards.iter().find(|c| c.kind == kind).map(|c| s.card_size(c)) else { return };
            let Some(c) = s.cards.iter_mut().find(|c| c.kind == kind) else { return };
            c.x = (x - dx).clamp(0.0, (sw - w).max(0.0)).round();
            c.y = (y - dy).clamp(0.0, (sh - h).max(0.0)).round();
            s.place_notes(glib::monotonic_time());
            s.paint.queue_draw();
        }
        Some(Drag::Resize { kind, edges, from, rect: (rx, ry, rw, rh) }) => {
            let (sw, sh) = s.size();
            let (dx, dy) = (x - from.0, y - from.1);
            let (min_w, max_w) = (kind.min_width(), (kind.width() * 2.2).min(sw));
            let min_h = s.height(kind).min(if kind == Kind::Notes { 160.0 } else { f64::MAX });
            let max_h = sh * 0.9;
            let (mut nx, mut ny, mut nw, mut nh) = (rx, ry, rw, rh);
            if edges & 2 != 0 {
                nw = (rw + dx).clamp(min_w, max_w.min(sw - rx));
            }
            if edges & 1 != 0 {
                nw = (rw - dx).clamp(min_w, max_w.min(rx + rw));
                nx = rx + rw - nw;
            }
            if edges & 8 != 0 {
                nh = (rh + dy).clamp(min_h, max_h.min(sh - ry));
            }
            if edges & 4 != 0 {
                nh = (rh - dy).clamp(min_h, max_h.min(ry + rh));
                ny = ry + rh - nh;
            }
            let Some(c) = s.cards.iter_mut().find(|c| c.kind == kind) else { return };
            (c.x, c.y, c.w) = (nx.round(), ny.round(), nw.round());
            if kind.stretches() {
                c.h = nh.round();
            }
            s.place_notes(glib::monotonic_time());
            s.paint.queue_draw();
        }
        Some(Drag::Slide { row, x0, w }) => {
            let level = ((x - x0) / w).clamp(0.0, 1.0);
            s.data.sliding = Some((row, (level * 100.0).round() / 100.0));
            s.data.held = true;
            s.paint.queue_draw();
            drop(s);
            push_level(st);
        }
        None => {}
    }
}

fn drag_end(st: &Shared) {
    let slide = {
        let mut s = st.borrow_mut();
        let drag = s.drag.take();
        s.paint.set_cursor_from_name(None);
        s.hot = None;
        match drag {
            Some(Drag::Move { .. } | Drag::Resize { .. }) => {
                s.save();
                false
            }
            Some(Drag::Slide { .. }) => {
                s.data.held = false;
                true
            }
            None => false,
        }
    };
    if slide {
        push_level(st);
    } else {
        region(st);
    }
}

/// Sends the slider's level to the sound server, one request at a time, always ending on the
/// last value the pointer left.
fn push_level(st: &Shared) {
    let job = {
        let mut s = st.borrow_mut();
        if s.data.pushing {
            return;
        }
        let Some((row, level)) = s.data.sliding else { return };
        if s.data.sent == Some(level) {
            if !s.data.held {
                s.data.sliding = None;
                s.data.sent = None;
                drop(s);
                refresh_mix(st);
            }
            return;
        }
        let Some(ch) = s.data.channel(row) else {
            s.data.sliding = None;
            return;
        };
        s.data.pushing = true;
        s.data.sent = Some(level);
        (s.data.mix_tab == 1, row == 0, ch, level)
    };
    let weak = Rc::downgrade(st);
    glib::spawn_future_local(async move {
        let (input, device, ch, level) = job;
        let _ = gio::spawn_blocking(move || audio::set_channel(input, device, &ch, Some(level), None)).await;
        let Some(st) = weak.upgrade() else { return };
        st.borrow_mut().data.pushing = false;
        push_level(&st);
    });
}

fn mute(st: &Shared, row: usize) {
    let job = {
        let s = st.borrow();
        let Some(ch) = s.data.channel(row) else { return };
        (s.data.mix_tab == 1, row == 0, ch)
    };
    let weak = Rc::downgrade(st);
    glib::spawn_future_local(async move {
        let (input, device, ch) = job;
        let muted = ch.muted;
        let _ = gio::spawn_blocking(move || audio::set_channel(input, device, &ch, None, Some(!muted))).await;
        if let Some(st) = weak.upgrade() {
            refresh_mix(&st);
        }
    });
}

fn bluetooth(st: &Shared, device: Option<usize>) {
    let job = {
        let mut s = st.borrow_mut();
        let Some(adapter) = s.data.bt.chosen().cloned() else { return };
        match device {
            None => Some((adapter.path.clone(), None, !adapter.powered)),
            Some(i) => {
                let Some(d) = cards::devices(&s.data.bt).get(i).cloned() else { return };
                if s.data.bt_busy.contains(&d.path) || !adapter.powered {
                    return;
                }
                s.data.bt_busy.push(d.path.clone());
                s.paint.queue_draw();
                Some((d.path.clone(), Some(()), !d.connected))
            }
        }
    };
    let Some((path, device, on)) = job else { return };
    let weak = Rc::downgrade(st);
    glib::spawn_future_local(async move {
        let done = match crate::bus::system().await {
            Ok(c) if device.is_some() => bluez::connect(&c, &path, on).await,
            Ok(c) => bluez::set_adapter(&c, &path, "Powered", on).await,
            Err(e) => Err(e.to_string()),
        };
        if let Err(e) = done {
            gsr::notify("Bluetooth", &e);
        }
        let Some(st) = weak.upgrade() else { return };
        st.borrow_mut().data.bt_busy.retain(|p| *p != path);
        refresh_bt(&st);
    });
}

fn snap(mode: &'static str) {
    after_close(move || {
        let argv: [&std::ffi::OsStr; 2] = ["digit-snap".as_ref(), mode.as_ref()];
        if crate::child::spawn(&argv, gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE).is_err() {
            gsr::notify("Can't take a screenshot", "digit-snap is missing");
        }
    });
}

fn flip(st: &Shared, kind: Kind, open: bool) {
    {
        let mut s = st.borrow_mut();
        let now = glib::monotonic_time();
        if open {
            raise(&mut s, kind);
        }
        let Some(c) = s.cards.iter_mut().find(|c| c.kind == kind) else { return };
        if c.open == open {
            return;
        }
        c.open = open;
        if !open {
            c.pinned = false;
        }
        c.vis.go(if open { 1.0 } else { 0.0 }, now, if open { IN_MS } else { OUT_MS }, if open { OPEN } else { CLOSE });
        s.save();
    }
    keyboard(st);
    region(st);
    kick(st);
    if open {
        refresh(st, true);
        tick(st);
    }
}

fn press(st: &Shared, hit: Hit) {
    let app = st.borrow().app.clone();
    match hit {
        Hit::Bar(kind) => {
            let open = st.borrow().cards.iter().any(|c| c.kind == kind && c.open);
            flip(st, kind, !open);
        }
        Hit::Close(kind) => flip(st, kind, false),
        Hit::Pin(kind) => {
            {
                let mut s = st.borrow_mut();
                if let Some(c) = s.cards.iter_mut().find(|c| c.kind == kind) {
                    c.pinned = !c.pinned;
                }
                s.save();
            }
            keyboard(st);
            region(st);
            kick(st);
        }
        // A click on an edge without dragging does nothing.
        Hit::Resize(..) => {}
        Hit::Head(kind) | Hit::Body(kind) => {
            if kind == Kind::Notes {
                let notes = st.borrow().notes.clone();
                notes.grab_focus();
            }
        }
        Hit::Rec(rec) => match rec {
            Rec::Region => snap("region"),
            Rec::Screen => snap("screen"),
            Rec::Record => {
                if gsr::status().is_some() {
                    gsr::toggle_record(&app);
                } else {
                    after_close(move || gsr::toggle_record(&app));
                }
            }
            Rec::Save => {
                if gsr::status().is_some_and(|s| s.mode == Mode::Replay) {
                    after_close(|| {
                        gsr::save_replay(None);
                    });
                }
            }
            Rec::Replay => {
                if let Err(e) = gsr::switch(&app, Mode::Replay, None) {
                    gsr::notify("Instant replay", &e);
                }
            }
            Rec::Pause => {
                gsr::pause();
            }
            Rec::Folder => {
                close(st);
                gsr::open_folder();
            }
            Rec::Settings => {
                close(st);
                gsr::settings();
            }
        },
        Hit::ResTab(i) => st.borrow_mut().data.res_tab = i,
        Hit::MixTab(i) => {
            {
                let mut s = st.borrow_mut();
                if s.data.mix_tab == i {
                    return;
                }
                s.data.mix_tab = i;
                s.data.mix = None;
            }
            refresh_mix(st);
            region(st);
        }
        Hit::Level(_) | Hit::Bare => {}
        Hit::Mute(row) => mute(st, row),
        Hit::Media(which) => {
            if let Some(p) = app.players.borrow().as_ref() {
                p.command(match which {
                    0 => crate::media::Command::Previous,
                    1 => crate::media::Command::PlayPause,
                    _ => crate::media::Command::Next,
                });
            }
        }
        Hit::BtPower => bluetooth(st, None),
        Hit::BtDevice(i) => bluetooth(st, Some(i)),
        Hit::AimOn | Hit::AimStyle(_) | Hit::AimSize(_) | Hit::AimColor(_) => {
            {
                let mut s = st.borrow_mut();
                match hit {
                    Hit::AimOn => s.aim.on = !s.aim.on,
                    Hit::AimStyle(v) => s.aim.style = v,
                    Hit::AimSize(v) => s.aim.size = v,
                    Hit::AimColor(v) => s.aim.color = v,
                    _ => {}
                }
                s.save();
            }
            kick(st);
        }
    }
    st.borrow().paint.queue_draw();
}

/// Keeps frames coming while anything moves; hides the surface once nothing is left on it.
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
        let now = clock.frame_time();
        s.place_notes(now);
        w.queue_draw();
        if !s.rise.settled(now) || s.cards.iter().any(|c| !c.vis.settled(now)) {
            return glib::ControlFlow::Continue;
        }
        s.ticking = false;
        if !s.needed() && s.window.is_visible() {
            let window = s.window.clone();
            draw::forget(&s.paint);
            drop(s);
            window.set_visible(false);
            glib::idle_add_local_once(move || {
                let idle = ONE.with(|one| one.borrow().as_ref().is_some_and(|st| st.try_borrow().is_ok_and(|s| !s.needed())));
                if idle {
                    let gone = ONE.with(|one| one.borrow_mut().take());
                    if let Some(st) = &gone {
                        let s = st.borrow();
                        if s.saving.is_some() {
                            save_notes(&s);
                        }
                        if let Some(display) = gdk::Display::default() {
                            gtk::style_context_remove_provider_for_display(&display, &s.css);
                        }
                    }
                    window.destroy();
                    drop(gone);
                    App::trim();
                }
            });
        }
        glib::ControlFlow::Break
    });
}

impl State {
    fn needed(&self) -> bool {
        self.open || self.aim.on || self.cards.iter().any(|c| c.open && c.pinned)
    }

    /// The card is on screen and takes input.
    fn live(&self, c: &Card) -> bool {
        c.open && (self.open || c.pinned)
    }

    fn shown(&self, kind: Kind) -> bool {
        self.window.is_visible() && self.cards.iter().any(|c| c.kind == kind && self.live(c))
    }

    fn size(&self) -> (f64, f64) {
        (self.paint.width().max(1) as f64, self.paint.height().max(1) as f64)
    }

    fn height(&self, kind: Kind) -> f64 {
        cards::height(self, kind)
    }

    /// Where a card that was never moved sits: the left column for sound and devices, the
    /// right one for system and capture.
    fn home(&self, kind: Kind) -> (f64, f64) {
        let (sw, sh) = self.size();
        let top = BAR_TOP + BAR_H + 28.0;
        let right = |k: Kind| sw - k.width() - MARGIN;
        match kind {
            Kind::Bluetooth => (MARGIN, top),
            Kind::Media => (MARGIN, top + self.height(Kind::Bluetooth) + 16.0),
            Kind::Mixer => (MARGIN, sh - self.height(Kind::Mixer) - MARGIN),
            Kind::Resources => (right(kind), top),
            Kind::Recorder => (right(kind), top + self.height(Kind::Resources) + 16.0),
            Kind::Notes => (right(kind), sh - self.height(Kind::Notes) - MARGIN),
            Kind::Crosshair => ((sw - kind.width()) / 2.0, sh - self.height(Kind::Crosshair) - MARGIN),
        }
    }

    /// The card's size: what the user dragged it to, never below what its content needs.
    fn card_size(&self, c: &Card) -> (f64, f64) {
        let natural = self.height(c.kind);
        let w = if c.w > 0.0 { c.w.max(c.kind.min_width()) } else { c.kind.width() };
        let h = if c.kind.stretches() && c.h > 0.0 { c.h.max(if c.kind == Kind::Notes { 160.0 } else { natural }) } else { natural };
        (w, h)
    }

    fn rect(&self, c: &Card) -> (f64, f64, f64, f64) {
        let (sw, sh) = self.size();
        let (w, h) = self.card_size(c);
        let (x, y) = if c.x < 0.0 || c.y < 0.0 { self.home(c.kind) } else { (c.x, c.y) };
        (x.clamp(0.0, (sw - w).max(0.0)).round(), y.clamp(0.0, (sh - h).max(0.0)).round(), w, h)
    }

    fn save(&mut self) {
        let placed: Vec<(f64, f64)> = self.cards.iter().map(|c| if c.x < 0.0 && self.paint.width() > 0 { let r = self.rect(c); (r.0, r.1) } else { (c.x, c.y) }).collect();
        for (c, (x, y)) in self.cards.iter_mut().zip(placed) {
            c.x = x;
            c.y = y;
        }
        let saved = Saved { cards: self.cards.iter().map(|c| Spot { kind: c.kind, x: c.x, y: c.y, open: c.open, pinned: c.pinned, w: c.w, h: c.h }).collect(), crosshair: self.aim };
        if let Ok(text) = serde_json::to_string_pretty(&saved) {
            let _ = std::fs::create_dir_all(kit::theme::state_dir());
            let _ = std::fs::write(file(), text);
        }
    }

    /// How much of a card shows and how far it is from its spot, for the open and close motion.
    fn presence(&self, c: &Card, now: i64) -> (f64, f64) {
        let rise = self.rise.value(now).clamp(0.0, 1.0);
        let vis = c.vis.value(now).clamp(0.0, 1.0);
        if c.pinned {
            (vis, (1.0 - vis) * 14.0)
        } else {
            (vis * rise, (1.0 - rise) * 28.0 + (1.0 - vis) * 14.0)
        }
    }

    /// Lays the notes field over the Notes card. Runs outside drawing, since it moves a widget.
    fn place_notes(&self, now: i64) {
        let Some(c) = self.cards.iter().find(|c| c.kind == Kind::Notes) else { return };
        let (alpha, dy) = self.presence(c, now);
        let show = alpha > 0.01 && self.paint.width() > 0;
        if self.scroller.is_visible() != show {
            self.scroller.set_visible(show);
        }
        if !show {
            return;
        }
        let (x, y, w, h) = self.rect(c);
        let (fx, fy, fw, fh) = cards::notes_field(w, h);
        self.scroller.set_margin_start((x + fx + 10.0) as i32);
        self.scroller.set_margin_top((y + fy + 8.0 + dy).max(0.0) as i32);
        self.scroller.set_size_request((fw - 20.0) as i32, (fh - 16.0) as i32);
        self.scroller.set_opacity(alpha);
    }

    fn bar(&self) -> (f64, f64, Vec<(Kind, f64)>) {
        let cfg = &self.app.cfg;
        let mut w = 10.0 + KINDS.len() as f64 * (SLOT + 4.0) + 14.0;
        let slots = KINDS.iter().enumerate().map(|(i, k)| (*k, 10.0 + i as f64 * (SLOT + 4.0))).collect();
        w += draw::width(&self.paint, cfg, &self.app.clock.borrow(), 15.0, Face::Number(600)).ceil() + 18.0;
        if let Some((level, _)) = crate::power::battery() {
            w += 24.0 + draw::width(&self.paint, cfg, &format!("{level}%"), 14.0, Face::Number(560)).ceil() + 16.0;
        }
        let x0 = ((self.size().0 - w) / 2.0).round();
        (x0, w, slots)
    }

    fn hit_at(&self, x: f64, y: f64) -> Option<Hit> {
        if self.open {
            let (x0, w, slots) = self.bar();
            if y >= BAR_TOP && y <= BAR_TOP + BAR_H && x >= x0 && x <= x0 + w {
                return Some(slots.iter().find(|(_, sx)| x >= x0 + sx && x <= x0 + sx + SLOT).map_or(Hit::Bare, |(k, _)| Hit::Bar(*k)));
            }
        }
        for c in self.cards.iter().rev().filter(|c| self.live(c)) {
            let (cx, cy, w, h) = self.rect(c);
            if x < cx || x > cx + w || y < cy || y > cy + h {
                continue;
            }
            let (lx, ly) = (x - cx, y - cy);
            let mut edges = 0u8;
            if lx <= GRIP {
                edges |= 1;
            } else if lx >= w - GRIP {
                edges |= 2;
            }
            if c.kind.stretches() {
                if ly <= GRIP {
                    edges |= 4;
                } else if ly >= h - GRIP {
                    edges |= 8;
                }
            }
            if edges != 0 {
                return Some(Hit::Resize(c.kind, edges));
            }
            let found = cards::head_items(c.kind, w).into_iter().chain(cards::items(self, c.kind, w)).find(|i| lx >= i.x && lx <= i.x + i.w && ly >= i.y && ly <= i.y + i.h).map(|i| i.hit);
            return Some(found.unwrap_or(if ly <= HEAD { Hit::Head(c.kind) } else { Hit::Body(c.kind) }));
        }
        None
    }

    fn draw(&self, s: &gtk::Snapshot) {
        let now = self.paint.frame_clock().map(|c| c.frame_time()).unwrap_or_else(glib::monotonic_time);
        // Cards sit on the surface chosen in Settings (Black, Graphite or Tinted), like the apps.
        let mut pal = self.app.palette_at(now);
        pal.body = self.app.card(&pal);
        let (sw, sh) = self.size();
        let rise = self.rise.value(now).clamp(0.0, 1.0);
        if rise > 0.003 {
            s.append_color(&Rgb::new(0.0, 0.0, 0.0).alpha(0.45 * rise).rgba(), &graphene::Rect::new(0.0, 0.0, sw as f32, sh as f32));
        }
        for c in &self.cards {
            let (alpha, dy) = self.presence(c, now);
            if alpha < 0.003 {
                continue;
            }
            let (x, y, w, h) = self.rect(c);
            s.save();
            s.translate(&graphene::Point::new(x as f32, (y + dy) as f32));
            s.push_opacity(alpha);
            self.draw_card(s, c, &pal, w, h);
            s.pop();
            s.restore();
        }
        if rise > 0.003 {
            s.save();
            s.translate(&graphene::Point::new(0.0, (-(1.0 - rise) * (BAR_TOP + BAR_H + 12.0)) as f32));
            s.push_opacity(rise);
            self.draw_bar(s, &pal);
            s.pop();
            s.restore();
        }
        if self.aim.on {
            cards::crosshair(s, &self.aim, sw / 2.0, sh / 2.0);
        }
    }

    fn draw_card(&self, s: &gtk::Snapshot, c: &Card, pal: &Palette, w: f64, h: f64) {
        let cfg = &self.app.cfg;
        let shape = draw::round(0.0, 0.0, w, h, RADIUS * self.app.round());
        // Same surface as the shell's panels: solid body, no outline, a soft shadow to lift it off windows.
        s.append_outset_shadow(&shape, &Rgb::new(0.0, 0.0, 0.0).alpha(0.24).rgba(), 0.0, 10.0, 0.0, 30.0);
        draw::fill(s, &shape, pal.body.alpha(1.0));
        let mid = HEAD / 2.0 + 2.0;
        draw::icon(s, &self.paint, cfg, c.kind.glyph(), 18.0, false, pal.ink_meta, PAD + 9.0, mid);
        draw::text(s, &self.paint, cfg, c.kind.title(), 14.0, Face::Text(600), pal.ink_main, PAD + 26.0, mid, w - 2.0 * PAD - 100.0, pango::Alignment::Left);
        for item in cards::head_items(c.kind, w) {
            let hot = self.hot == Some(item.hit);
            let (glyph, on) = match item.hit {
                Hit::Pin(_) => ("keep", c.pinned),
                _ => ("close", false),
            };
            let back = if on { pal.accent } else if hot { pal.ink_main.mix(pal.body, 0.12) } else { pal.body.alpha(0.0) };
            draw::fill(s, &draw::round(item.x, item.y, item.w, item.h, item.h / 2.0), back);
            draw::icon(s, &self.paint, cfg, glyph, 18.0, on, if on { pal.on_accent } else { pal.ink_meta }, item.x + item.w / 2.0, item.y + item.h / 2.0);
        }
        cards::draw(self, s, c.kind, pal, w, h);
    }

    fn draw_bar(&self, s: &gtk::Snapshot, pal: &Palette) {
        let cfg = &self.app.cfg;
        let (x0, w, slots) = self.bar();
        let shape = draw::round(x0, BAR_TOP, w, BAR_H, BAR_H / 2.0);
        s.append_outset_shadow(&shape, &Rgb::new(0.0, 0.0, 0.0).alpha(0.24).rgba(), 0.0, 10.0, 0.0, 30.0);
        draw::fill(s, &shape, pal.body.alpha(1.0));
        let mid = BAR_TOP + BAR_H / 2.0;
        for (kind, sx) in &slots {
            let on = self.cards.iter().any(|c| c.kind == *kind && c.open);
            let hot = self.hot == Some(Hit::Bar(*kind));
            let back = if on { pal.accent } else if hot { pal.ink_main.mix(pal.body, 0.12) } else { pal.body.alpha(0.0) };
            draw::fill(s, &draw::round(x0 + sx, mid - SLOT / 2.0, SLOT, SLOT, SLOT / 2.0), back);
            let glyph = if *kind == Kind::Recorder && gsr::status().is_some() { "radio_button_checked" } else { kind.glyph() };
            let ink = if *kind == Kind::Recorder && gsr::status().is_some() { RED } else if on { pal.on_accent } else { pal.ink_main };
            draw::icon(s, &self.paint, cfg, glyph, 20.0, on, ink, x0 + sx + SLOT / 2.0, mid);
        }
        let mut x = x0 + 10.0 + KINDS.len() as f64 * (SLOT + 4.0) + 4.0;
        x += 10.0;
        let clock = self.app.clock.borrow().clone();
        let cw = draw::width(&self.paint, cfg, &clock, 15.0, Face::Number(600)).ceil();
        draw::text(s, &self.paint, cfg, &clock, 15.0, Face::Number(600), pal.ink_main, x, mid, cw + 2.0, pango::Alignment::Left);
        x += cw + 18.0;
        if let Some((level, charging)) = crate::power::battery() {
            let glyph = if charging {
                "battery_charging_full"
            } else {
                ["battery_0_bar", "battery_1_bar", "battery_2_bar", "battery_3_bar", "battery_4_bar", "battery_5_bar", "battery_6_bar", "battery_full"][(level as usize * 7 / 100).min(7)]
            };
            draw::icon(s, &self.paint, cfg, glyph, 20.0, true, if level <= 15 && !charging { RED } else { pal.ink_main }, x + 8.0, mid);
            let text = format!("{level}%");
            draw::text(s, &self.paint, cfg, &text, 14.0, Face::Number(560), pal.ink_main, x + 20.0, mid, 60.0, pango::Alignment::Left);
        }
    }
}
