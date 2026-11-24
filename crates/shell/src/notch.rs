use crate::app::{App, Ending, Peek, Timing};
use crate::config::Dock;
use kit::motion::{self, Tween};
use kit::palette::Rgb;
use crate::draw::{self, Face};
use crate::audio::{self, Ask};
use crate::metrics::{CARD_RADIUS, GAP as GAP_CONTROLS, PANEL_PAD};
use crate::{blur, media};
use kit::shape;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{cairo, gdk, glib, graphene, gsk, pango};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

mod input;
mod island;
mod phone;
mod sheet;
mod tray;

const WIDTH: f64 = 860.0;
const ROOM: f64 = 420.0;
const CENTRE: f64 = 116.0;
const CX: f64 = WIDTH / 2.0;
const SHEET_W: f64 = 780.0;
const OPEN_DEPTH: f64 = 153.0;
/// Extra room above an open Dynamic Island's content, so its top edge matches the sides.
const ISLAND_PAD: f64 = 9.0;
const SIDE_PAD: f64 = 22.0;
const LEFT: f64 = CX - SHEET_W / 2.0 + SIDE_PAD;
const RIGHT: f64 = CX + SHEET_W / 2.0 - SIDE_PAD;
const MEDIA_W: f64 = 318.0;
const CAL_X: f64 = LEFT + MEDIA_W + 28.0;
const CAL_W: f64 = 262.0;
const TOOLS_X: f64 = CAL_X + CAL_W + 28.0;
const TOOLS: [(&str, &str); 4] = [("screenshot_region", "Screenshot"), ("colorize", "Pick color"), ("do_not_disturb_on", "Do not disturb"), ("coffee", "Keep awake")];
const FILES_W: f64 = 452.0;
const TILE: f64 = 82.0;
const TILES: usize = 5;
const PHONE_W: f64 = 300.0;
const NOTES: usize = 3;

const DEVICES: usize = 2;
const CHIP: f64 = 28.0;
const CHIPS: usize = 7;
const ROLL: i64 = 260_000;
/// Space between the workspace pill and the island's sides when the pill widens it.
const PILL_MARGIN: f64 = 14.0;
const GREEN: Rgb = Rgb::new(0.19, 0.82, 0.35);
const MORPH_MS: i64 = 680;
const MORPH: motion::Curve = motion::Curve::new(0.33, 0.0, 0.15, 1.0);
const ORANGE: Rgb = Rgb::new(1.0, 0.62, 0.04);
const PURPLE: Rgb = Rgb::new(0.75, 0.35, 0.95);
const ACTION_BLUE: Rgb = Rgb::new(0.04, 0.52, 1.0);
const RED: Rgb = Rgb::new(1.0, 0.27, 0.23);

fn phone_rows(call: bool, notes: usize) -> impl Iterator<Item = (usize, usize)> {
    (usize::from(call)..NOTES).zip(0..notes)
}

#[derive(Clone, Copy, PartialEq)]
enum Hot {
    Control(usize),
    Seek,
    Volume,
    Tab(usize),
    Gear,
    Tool(usize),
    Day(usize),
    Tile(usize),
    Remove(usize),
    Device(usize),
    Sender(usize),
    CallMute,
    CallOpen,
    CallSwitch,
    PhoneAction(usize),
    PhoneSwitch,
    PhoneDismiss(usize),
    Action(usize),
}

impl Hot {
    fn index(self) -> usize {
        match self {
            Hot::Control(i) => i,
            Hot::CallMute => 0,
            Hot::CallOpen => 1,
            Hot::CallSwitch => 10,
            Hot::Action(_) => 8,
            Hot::Seek => 3,
            Hot::Volume => 4,
            Hot::Tab(i) => if i < 2 { 5 + i } else { 11 },
            Hot::Gear => 7,
            Hot::Day(_) | Hot::Tile(_) | Hot::Remove(_) | Hot::Device(_) | Hot::Tool(_) | Hot::Sender(_) | Hot::PhoneAction(_) | Hot::PhoneSwitch | Hot::PhoneDismiss(_) => 9,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Nook,
    Tray,
    Phone,
}

const TABS: [(Tab, &str, &str); 3] = [(Tab::Nook, "lightbulb", "Nook"), (Tab::Tray, "inbox", "Tray"), (Tab::Phone, "smartphone", "Phone")];

#[derive(Clone, Copy, PartialEq)]
enum Landing {
    Files,
    Share,
    Device(usize),
    Sender(usize),
}

enum DropLanding {
    Files,
    Device(crate::share::Device),
    Sender(crate::share::Sender),
}

impl DropLanding {
    fn capture(landing: Landing) -> Self {
        let devices = crate::share::devices();
        match landing {
            Landing::Device(i) => devices.get(i).cloned().map_or(Self::Files, Self::Device),
            Landing::Sender(i) => crate::share::senders().get(i).cloned().map_or(Self::Files, Self::Sender),
            Landing::Share if devices.len() == 1 => Self::Device(devices[0].clone()),
            _ => Self::Files,
        }
    }
}

struct Tuner {
    icon: Rect,
    track: Rect,
    label: f64,
}

#[derive(Clone, Debug, PartialEq)]
enum Head {
    Idle(bool),
    Call,
    Open,
    Peek(Peek),
}

impl Head {
    fn same(&self, other: &Head) -> bool {
        match (self, other) {
            (Head::Peek(a), Head::Peek(b)) => a.same_kind(b),
            _ => self == other,
        }
    }
}

#[derive(Clone, Copy)]
enum Action {
    Previous,
    Toggle,
    Next,
}

struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Rect {
    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x <= self.x + self.w && y >= self.y && y <= self.y + self.h
    }

    fn graphene(&self) -> graphene::Rect {
        graphene::Rect::new(self.x as f32, self.y as f32, self.w as f32, self.h as f32)
    }
}

struct Player {
    art: Rect,
    info_x: f64,
    info_w: f64,
    top: f64,
    /// Generous hit area around the progress bar for click/drag seeking.
    seek: Rect,
    controls: [(Rect, Action); 3],
}

struct View {
    app: Rc<App>,
    window: gtk::Window,
    canvas: Canvas,
    bars: crate::bars::Bars,
    output: String,
    open: bool,
    hovered: bool,
    dwell: u64,
    leave: u64,
    w: Tween,
    depth: Tween,
    r: Tween,
    fade: Tween,
    sheet: Tween,
    back: Tween,
    meter: Tween,
    art_scale: Tween,
    head: Head,
    want: Head,
    shown: Vec<char>,
    was: Vec<char>,
    roll: Vec<Option<i64>>,
    region: (i32, i32, i32, i32),
    effect: Option<(blur::Effect, wayland_client::backend::ObjectId)>,
    pal: std::cell::Cell<kit::palette::Palette>,
    frost: Vec<(i32, i32, i32, i32)>,
    /// With GTK 4.23.3 and newer, which owns the surface's blur: a pane under the menu bar's band.
    panes: blur::Frost,
    ticking: bool,
    /// How many characters the call's clock had when the island was last sized for it.
    call_len: Cell<usize>,
    /// Fraction of the track under the pointer while the progress bar is being dragged.
    scrub: Option<f64>,
    fill: Tween,
    timed: bool,
    pointer: Option<(f64, f64)>,
    hot: Option<Hot>,
    glow: [Tween; 12],
    feel: crate::feel::Feel<Hot>,
    /// The open notch shows the player instead of the call it would normally show; cleared when the call ends.
    media_over_call: std::cell::Cell<bool>,
    /// Scrolling over the workspace dots: wheel travel not yet turned into a step, and the
    /// workspace the last step went to (with when), since the compositor reports it later.
    space_scroll: Cell<f64>,
    /// The workspace pill when it shows in the notch instead of beside the logo.
    pill: crate::strip::Spaces,
    space_aim: Cell<Option<(u64, i64)>>,
    /// Fades the media spot back in after flipping between the call and the player.
    flip: Tween,
    tuning: bool,
    tune: Tween,
    sliding: bool,
    dots: [Tween; 2],
    capture: std::cell::Cell<Rgb>,
    hush: Tween,
    panel_triggers: Vec<gtk::Window>,
    /// Catches clicks elsewhere while the notch is open in "click" mode; the notch itself is cut out of it.
    scrim: Option<gtk::Window>,
    tab: Tab,
    day: i32,
    tab_fade: Tween,
    landing: Option<Landing>,
    dropping: bool,
    badge: RefCell<Option<(String, Option<gdk::Paintable>)>>,
    strip: Option<crate::strip::Strip>,
    span: i32,
    float: Tween,
    widen: Tween,
    grown: bool,
    monitor: gdk::Monitor,
    menu: Tween,
    glass: Tween,
    opener: Option<Rc<dyn Fn(crate::strip::Zone, u32)>>,
}

#[derive(Clone)]
pub struct Notch(Rc<RefCell<View>>);

impl Notch {
    pub fn new(app: &Rc<App>, monitor: &gdk::Monitor) -> Self {
        let header = app.header();
        let wide = app.wide();
        let floating = app.floating();
        let window = gtk::Window::new();
        window.init_layer_shell();
        window.set_layer(Layer::Top);
        window.set_namespace(Some("digit"));
        window.set_anchor(Edge::Top, true);
        window.set_exclusive_zone(app.reserve());
        window.set_keyboard_mode(KeyboardMode::None);
        window.set_monitor(Some(monitor));
        window.add_css_class("digit");
        let bar = wide || app.menubar();
        let span = if bar { monitor.geometry().width().max(WIDTH as i32) } else { WIDTH as i32 };
        window.set_default_size(span, (header + ROOM) as i32);
        let canvas = Canvas::default();
        canvas.set_size_request(WIDTH as i32, (header + ROOM) as i32);
        let bars = crate::bars::Bars::default();
        bars.set_visible(false);
        let layer = gtk::Overlay::new();
        layer.set_child(Some(&canvas));
        layer.add_overlay(&bars);
        let strip = Some(crate::strip::Strip::new(app, header, WIDTH));
        let panes = match &strip {
            Some(st) => {
                st.set_size_request(span, (header + ROOM) as i32);
                st.set_hexpand(true);
                layer.set_halign(gtk::Align::Center);
                layer.set_valign(gtk::Align::Start);
                let root = gtk::Overlay::new();
                root.set_child(Some(st));
                root.add_overlay(&layer);
                let (frosted, panes) = blur::Frost::wrap(&root);
                window.set_child(Some(&frosted));
                panes
            }
            None => {
                let (frosted, panes) = blur::Frost::wrap(&layer);
                window.set_child(Some(&frosted));
                panes
            }
        };
        crate::spectrum::watch(app, &bars);
        let clock: Vec<char> = app.clock.borrow().chars().collect();
        let view = Rc::new(RefCell::new(View {
            app: app.clone(),
            window: window.clone(),
            canvas: canvas.clone(),
            bars: bars.clone(),
            output: monitor.connector().map(|c| c.to_string()).unwrap_or_default(),
            open: false,
            hovered: false,
            dwell: 0,
            leave: 0,
            w: Tween::still(CENTRE),
            depth: Tween::still(0.0),
            r: Tween::still(14.0),
            fade: Tween::still(1.0),
            sheet: Tween::still(0.0),
            back: Tween::still(0.0),
            meter: Tween::still(0.0),
            art_scale: Tween::still(1.0),
            head: Head::Idle(false),
            want: Head::Idle(false),
            roll: vec![None; clock.len()],
            was: clock.clone(),
            shown: clock,
            region: (0, 0, 0, 0),
            effect: None,
            pal: std::cell::Cell::new(app.palette.get()),
            frost: Vec::new(),
            panes,
            ticking: false,
            call_len: Cell::new(0),
            scrub: None,
            fill: Tween::still(1.0),
            timed: true,
            pointer: None,
            hot: None,
            glow: [Tween::still(0.0); 12],
            feel: crate::feel::Feel::default(),
            media_over_call: std::cell::Cell::new(false),
            space_scroll: Cell::new(0.0),
            pill: crate::strip::Spaces::default(),
            space_aim: Cell::new(None),
            flip: Tween::still(1.0),
            tuning: false,
            tune: Tween::still(0.0),
            sliding: false,
            dots: [Tween::still(0.0); 2],
            capture: std::cell::Cell::new(ORANGE),
            hush: Tween::still(1.0),
            panel_triggers: Vec::new(),
            scrim: None,
            tab: Tab::Nook,
            day: 0,
            tab_fade: Tween::still(1.0),
            landing: None,
            dropping: false,
            badge: RefCell::new(None),
            strip,
            span: WIDTH as i32,
            float: Tween::still(if floating { 1.0 } else { 0.0 }),
            widen: Tween::still(if wide { 1.0 } else { 0.0 }),
            grown: bar,
            monitor: monitor.clone(),
            menu: Tween::still(if app.menubar() { 1.0 } else { 0.0 }),
            glass: Tween::still(0.0),
            opener: None,
        }));
        *canvas.imp().view.borrow_mut() = Rc::downgrade(&view);
        let notch = Notch(view);
        notch.wire();
        window.present();
        {
            let mut v = notch.0.borrow_mut();
            v.panel_triggers = crate::panel_corner::triggers(app, monitor);
        }
        notch.refresh();
        notch
    }

    fn wire(&self) {
        let canvas = self.0.borrow().canvas.clone();
        let motion = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(&self.0);
        motion.connect_enter(move |_, x, y| {
            if let Some(v) = weak.upgrade() {
                let y = lifted(&v, y);
                Notch(v).point(Some((x, y)));
            }
        });
        let weak = Rc::downgrade(&self.0);
        motion.connect_motion(move |_, x, y| {
            if let Some(v) = weak.upgrade() {
                let y = lifted(&v, y);
                Notch(v).point(Some((x, y)));
            }
        });
        let weak = Rc::downgrade(&self.0);
        motion.connect_leave(move |_| {
            if let Some(v) = weak.upgrade() {
                let notch = Notch(v);
                notch.point(None);
                notch.hover(false);
            }
        });
        canvas.add_controller(motion);
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        let weak = Rc::downgrade(&self.0);
        scroll.connect_scroll(move |_, _, dy| {
            if weak.upgrade().is_some_and(|v| {
                let notch = Notch(v);
                notch.wheel(dy) || notch.scroll_spaces_at_pointer(dy)
            }) {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        canvas.add_controller(scroll);
        // Hyprland offers MOVE as the only action before any negotiation; files are still only read.
        let target = gtk::DropTargetAsync::new(None, gdk::DragAction::COPY | gdk::DragAction::MOVE);
        target.connect_accept(|_, drop| crate::tray::accepts_drop(drop));
        let host: gtk::Widget = self.0.borrow().window.child().unwrap_or_else(|| canvas.clone().upcast());
        let to_canvas = {
            let (host, canvas) = (host.downgrade(), canvas.downgrade());
            move |x: f64, y: f64| host.upgrade().zip(canvas.upgrade()).and_then(|(host, canvas)| host.compute_point(&canvas, &graphene::Point::new(x as f32, y as f32))).map(|p| (p.x() as f64, p.y() as f64)).unwrap_or((x, y))
        };
        let weak = Rc::downgrade(&self.0);
        target.connect_drag_enter(move |_, _, _, _| {
            if let Some(v) = weak.upgrade() {
                Notch(v).drag_enter();
            }
            gdk::DragAction::COPY
        });
        let weak = Rc::downgrade(&self.0);
        let map = to_canvas.clone();
        target.connect_drag_motion(move |_, _, x, y| {
            if let Some(v) = weak.upgrade() {
                let (x, y) = map(x, y);
                let y = lifted(&v, y);
                Notch(v).drag_motion(x, y);
            }
            gdk::DragAction::COPY
        });
        let weak = Rc::downgrade(&self.0);
        target.connect_drag_leave(move |_, _| {
            if let Some(v) = weak.upgrade() {
                Notch(v).drag_leave();
            }
        });
        let weak = Rc::downgrade(&self.0);
        let map = to_canvas.clone();
        target.connect_drop(move |_, drop, x, y| {
            let Some(v) = weak.upgrade() else { return false };
            let (x, y) = map(x, y);
            let landing = {
                let y = lifted(&v, y);
                let mut view = v.borrow_mut();
                let at = view.landing_at(x, y).unwrap_or(Landing::Files);
                view.dropping = false;
                view.landing = None;
                view.canvas.queue_draw();
                DropLanding::capture(at)
            };
            let weak = Rc::downgrade(&v);
            let drop = drop.clone();
            glib::spawn_future_local(async move {
                let paths = crate::tray::receive_drop(&drop).await;
                let ok = weak.upgrade().is_some_and(|v| Notch(v).dropped(paths, landing));
                drop.finish(if ok { gdk::DragAction::COPY } else { gdk::DragAction::empty() });
            });
            true
        });
        host.add_controller(target);
        let source = gtk::DragSource::new();
        source.set_actions(gdk::DragAction::COPY);
        let weak = Rc::downgrade(&self.0);
        source.connect_prepare(move |src, x, y| {
            let v = weak.upgrade()?;
            let y = lifted(&v, y);
            let i = Notch(v).tile_at(x, y)?;
            let path = crate::tray::path(i)?;
            if let Some(icon) = crate::tray::icon(i) {
                src.set_icon(Some(&icon), 24, 24);
            }
            let list = gdk::FileList::from_array(&[gtk::gio::File::for_path(path)]);
            Some(gdk::ContentProvider::for_value(&list.to_value()))
        });
        canvas.add_controller(source);
        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(&self.0);
        click.connect_released(move |_, _, x, y| {
            if let Some(v) = weak.upgrade() {
                let y = lifted(&v, y);
                Notch(v).click(x, y);
            }
        });
        let weak = Rc::downgrade(&self.0);
        click.connect_pressed(move |_, _, x, y| {
            if let Some(v) = weak.upgrade() {
                let y = lifted(&v, y);
                Notch(v).press(x, y);
            }
        });
        let weak = Rc::downgrade(&self.0);
        click.connect_end(move |_, _| {
            if let Some(v) = weak.upgrade() {
                Notch(v).release();
            }
        });
        canvas.add_controller(click);
        let drag = gtk::GestureDrag::new();
        let weak = Rc::downgrade(&self.0);
        drag.connect_drag_begin(move |g, x, y| {
            if let Some(v) = weak.upgrade() {
                let y = lifted(&v, y);
                if Notch(v).begin_drag(x, y) {
                    g.set_state(gtk::EventSequenceState::Claimed);
                }
            }
        });
        let weak = Rc::downgrade(&self.0);
        drag.connect_drag_update(move |g, dx, _| {
            if let (Some(v), Some((x, _))) = (weak.upgrade(), g.start_point()) {
                Notch(v).move_drag(x + dx);
            }
        });
        let weak = Rc::downgrade(&self.0);
        drag.connect_drag_end(move |g, dx, _| {
            if let (Some(v), Some((x, _))) = (weak.upgrade(), g.start_point()) {
                Notch(v).end_drag(x + dx);
            }
        });
        canvas.add_controller(drag);
        let strip = self.0.borrow().strip.clone();
        if let Some(st) = strip {
            let weak = Rc::downgrade(&self.0);
            st.follow(move |now| {
                let v = weak.upgrade()?;
                let v = v.try_borrow().ok()?;
                let luma = v.app.wallpapers.top_luma(&v.output, 0.04).unwrap_or(0.0);
                Some(crate::strip::Look {
                    widen: v.widen.value(now).clamp(0.0, 1.0),
                    slab: v.w.value(now),
                    radius: v.r.value(now),
                    lift: v.lift(now),
                    menu: v.menu.value(now).clamp(0.0, 1.0),
                    glass: v.glass.value(now).clamp(0.0, 1.0),
                    luma,
                    blur: v.effect.is_some(),
                })
            });
            let weak = Rc::downgrade(&self.0);
            st.on_resize(move |w| {
                let Some(v) = weak.upgrade() else { return };
                let notch = Notch(v);
                let fresh = notch.0.try_borrow_mut().map(|mut v| {
                    v.span = w;
                    v.region = (0, 0, 0, 0);
                });
                if fresh.is_ok() {
                    notch.kick();
                } else {
                    glib::idle_add_local_once(move || {
                        if let Ok(mut v) = notch.0.try_borrow_mut() {
                            v.span = w;
                            v.region = (0, 0, 0, 0);
                        }
                        notch.kick();
                    });
                }
            });
            use crate::strip::Zone;
            let weak = Rc::downgrade(&self.0);
            let target = st.downgrade();
            let open_menu: Rc<dyn Fn(Zone, u32)> = Rc::new(move |zone: Zone, button: u32| {
                let Some(target) = target.upgrade() else { return };
                let Some(v) = weak.upgrade() else { return };
                if zone == Zone::Spaces {
                    // A dot in the workspace pill switches to its workspace.
                    let picked = target.hovered_space().and_then(|i| v.borrow().spaces().0.get(i).cloned());
                    if let Some(space) = picked {
                        glib::spawn_future_local(async move {
                            // The empty one after the last may not exist yet: make it on this screen.
                            let _ = gtk::gio::spawn_blocking(move || if space.filled || space.active { kit::wm::focus_space(space.id) } else { kit::wm::focus_space_index(space.idx, &space.output) }).await;
                        });
                    }
                    return;
                }
                let (app, below, monitor, origin) = {
                    let v = v.borrow();
                    // The notch window is centred on its monitor: where its x = 0 lands.
                    let origin = (v.monitor.geometry().width() as f64 - v.window.width() as f64) / 2.0;
                    (v.app.clone(), v.band(App::now()) + 5.0, Some(v.monitor.clone()), origin)
                };
                let Some(left) = target.zone_x(zone) else { return };
                target.hold(Some(zone));
                let held = target.downgrade();
                crate::popmenu::then(move || {
                    let Some(held) = held.upgrade() else { return };
                    if held.held() == Some(zone) {
                        held.hold(None);
                    }
                });
                let later = match zone {
                    Zone::Logo => {
                        crate::menubar::system(&app, monitor, left, below);
                        false
                    }
                    Zone::App => crate::menubar::app_menu(&app, monitor, left, below),
                    Zone::Menu(i) => {
                        crate::menubar::section(&app, monitor, left, below, i as usize);
                        false
                    }
                    Zone::Win => {
                        crate::menubar::window_menu(&app, monitor, left, below);
                        false
                    }
                    Zone::Date => {
                        // Docked under a notch, the panel keeps the pill's own gap below it.
                        let y = if app.floating() { below } else { target.under_pill().unwrap_or(below) };
                        crate::datepop::show(&app, monitor, origin + target.zone_end(zone).unwrap_or(left), y);
                        false
                    }
                    Zone::Tray(i) => {
                        match button {
                            3 => crate::sni::menu(&app, i as usize, monitor, left, below),
                            2 => crate::sni::activate(&app, i as usize, monitor, left, below, true),
                            _ => crate::sni::activate(&app, i as usize, monitor, left, below, false),
                        }
                        true
                    }
                    _ => false,
                };
                if !later {
                    crate::popmenu::settle();
                }
            });
            self.0.borrow_mut().opener = Some(open_menu.clone());
            let menu_zone = |z: Option<Zone>| matches!(z, Some(Zone::Logo | Zone::App | Zone::Menu(_) | Zone::Win));
            let motion = gtk::EventControllerMotion::new();
            let hover = st.downgrade();
            let slide = open_menu.clone();
            let cornered = Rc::new(Cell::new(None));
            let motion_cornered = cornered.clone();
            let weak = Rc::downgrade(&self.0);
            motion.connect_motion(move |_, x, y| {
                let Some(hover) = hover.upgrade() else { return };
                let full = hover.width() as f64 >= hover.root().map(|r| r.width() as f64).unwrap_or(0.0) - 1.0;
                let corner = if full && y <= 1.5 {
                    if x <= 1.5 { Some(kit::corners::Corner::TopLeft) }
                    else if x >= hover.width() as f64 - 1.5 { Some(kit::corners::Corner::TopRight) }
                    else { None }
                } else { None };
                let previous = motion_cornered.replace(corner);
                if let Some(corner) = corner && previous != Some(corner) && let Some(v) = weak.upgrade() {
                    let (app, output) = { let v = v.borrow(); (v.app.clone(), v.output.clone()) };
                    crate::panel_corner::activate(&app, &output, corner);
                }
                let zone = hover.zone_at(x, y);
                hover.hover(zone);
                if menu_zone(zone) && hover.held().is_some() && hover.held() != zone && (crate::popmenu::is_open() || crate::datepop::is_open()) {
                    if let Some(z) = zone {
                        slide(z, 1);
                    }
                }
            });
            let hover = st.downgrade();
            motion.connect_leave(move |_| {
                let Some(hover) = hover.upgrade() else { return };
                cornered.set(None);
                hover.hover(None);
            });
            st.add_controller(motion);
            let click = gtk::GestureClick::new();
            click.set_button(0);
            let pressed: Rc<Cell<Option<Zone>>> = Rc::new(Cell::new(None));
            let was = pressed.clone();
            let target = st.downgrade();
            click.connect_pressed(move |_, _, x, y| {
                let Some(target) = target.upgrade() else { return };
                target.press(target.zone_at(x, y));
                was.set(if crate::popmenu::is_open() || crate::datepop::is_open() { target.held() } else { None });
            });
            let target = st.downgrade();
            click.connect_end(move |_, _| {
                if let Some(target) = target.upgrade() {
                    target.release();
                }
            });
            let weak = Rc::downgrade(&self.0);
            let target = st.downgrade();
            click.connect_released(move |g, _, x, y| {
                let Some(target) = target.upgrade() else { return };
                let Some(v) = weak.upgrade() else { return };
                let notch = Notch(v);
                let app = notch.0.borrow().app.clone();
                let zone = target.zone_at(x, y);
                let button = g.current_button();
                if zone.is_some() && zone == pressed.take() {
                    crate::popmenu::close();
                    crate::datepop::close();
                    target.hold(None);
                    return;
                }
                match zone {
                    Some(z) if menu_zone(Some(z)) || matches!(z, Zone::Tray(_) | Zone::Date | Zone::Spaces) => open_menu(z, button),
                    Some(Zone::Status) => {
                        // Beside the button: the same origin and bar bottom the menus use.
                        let (below, screen_w, origin) = {
                            let v = notch.0.borrow();
                            let screen_w = v.monitor.geometry().width() as f64;
                            (v.band(App::now()) + 5.0, screen_w, (screen_w - v.window.width() as f64) / 2.0)
                        };
                        // A card lines up with the bar's right end, as far right as the screen allows.
                        let right = screen_w - origin.max(0.0);
                        target.hold(Some(Zone::Status));
                        let held = target.downgrade();
                        let below = if app.floating() { below } else { target.under_pill().unwrap_or(below) };
                        crate::control::toggle_from(&app, right, below, screen_w, move || {
                            if let Some(held) = held.upgrade() && held.held() == Some(Zone::Status) {
                                held.hold(None);
                            }
                        });
                        if !app.control.borrow().as_ref().is_some_and(|c| c.is_open()) {
                            target.hold(None);
                        }
                    }
                    _ => {}
                }
            });
            st.add_controller(click);
            // Scrolling over the workspace pill steps to the previous or next workspace.
            let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
            let weak = Rc::downgrade(&self.0);
            let target = st.downgrade();
            scroll.connect_scroll(move |_, _, dy| {
                let over = target.upgrade().is_some_and(|t| t.hot() == Some(Zone::Spaces));
                if over && let Some(v) = weak.upgrade() {
                    Notch(v).scroll_spaces(dy);
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            st.add_controller(scroll);
        }
        let weak = Rc::downgrade(&self.0);
        self.0.borrow().window.connect_realize(move |_| {
            if let Some(v) = weak.upgrade() {
                Notch(v).kick();
            }
        });
        let weak = Rc::downgrade(&self.0);
        self.0.borrow().window.connect_unmap(move |_| {
            let Some(v) = weak.upgrade() else { return };
            let Ok(mut v) = v.try_borrow_mut() else { return };
            let app = v.app.clone();
            if let (Some((effect, _)), Some(blur)) = (v.effect.take(), app.blur.as_ref()) {
                blur.detach(effect);
            }
            v.frost.clear();
        });
    }

    pub fn output(&self) -> String {
        self.0.borrow().output.clone()
    }

    pub fn destroy(&self) {
        let (window, canvas, open, app) = {
            let v = self.0.borrow();
            (v.window.clone(), v.canvas.clone(), v.open, v.app.clone())
        };
        if open {
            app.set_open(false);
        }
        draw::forget(&canvas);
        if let Some(st) = self.0.borrow().strip.as_ref() {
            draw::forget(st);
        }
        let triggers = std::mem::take(&mut self.0.borrow_mut().panel_triggers);
        for trigger in triggers {
            trigger.destroy();
        }
        let scrim = self.0.borrow_mut().scrim.take();
        if let Some(c) = scrim {
            c.destroy();
        }
        let effect = self.0.borrow_mut().effect.take();
        if let (Some((effect, _)), Some(blur)) = (effect, app.blur.as_ref()) {
            blur.detach(effect);
        }
        window.destroy();
    }

    pub fn refresh_corners(&self) {
        let (app, monitor, old) = {
            let mut v = self.0.borrow_mut();
            (v.app.clone(), v.monitor.clone(), std::mem::take(&mut v.panel_triggers))
        };
        for window in old { window.destroy(); }
        self.0.borrow_mut().panel_triggers = crate::panel_corner::triggers(&app, &monitor);
    }

    pub fn surface(&self) -> Option<gdk::Surface> {
        self.0.borrow().window.surface()
    }

    pub fn redraw(&self) {
        let resize = {
            let v = self.0.borrow();
            v.canvas.queue_draw();
            if let Some(st) = &v.strip {
                st.queue_draw();
            }
            // The call's clock gains a digit (9:59 to 10:00, 59:59 to 1:00:00): the island is
            // sized again, or the time runs out of it.
            let len = if v.app.call.borrow().is_some() { v.call_time(App::now()).len() } else { 0 };
            len > 0 && v.call_len.replace(len) != len
        };
        if resize {
            self.refresh();
        }
    }

    pub fn restyle(&self) {
        let app = self.0.borrow().app.clone();
        let (wide, floating, reserve, header) = (app.wide(), app.floating(), app.reserve(), app.header());
        {
            let mut v = self.0.borrow_mut();
            let now = App::now();
            v.window.set_exclusive_zone(reserve);
            let bar = wide || app.menubar();
            if bar && !v.grown {
                v.grow(true);
            }
            v.menu.go(if app.menubar() { 1.0 } else { 0.0 }, now, MORPH_MS, MORPH);
            v.widen.go(if wide { 1.0 } else { 0.0 }, now, MORPH_MS, MORPH);
            v.float.go(if floating { 1.0 } else { 0.0 }, now, MORPH_MS, MORPH);
            let head = v.target(now);
            let (w, depth, r) = v.geometry(&head, now);
            let r = v.rounded(r, depth);
            v.w.go(w, now, MORPH_MS, MORPH);
            v.r.go(r, now, MORPH_MS, MORPH);
            if let Some(st) = &v.strip {
                st.set_header(header);
            }
            v.region = (0, 0, 0, 0);
        }
        self.refresh();
    }

    pub fn redraw_strip(&self) {
        if let Some(st) = self.0.borrow().strip.as_ref() {
            st.queue_draw();
        }
    }

    pub fn redraw_wings(&self) {
        let v = self.0.borrow();
        if v.bars.is_visible() {
            v.bars.set(&v.app.spectrum.borrow());
        }
    }

    pub fn open_zone(&self, zone: crate::strip::Zone, button: u32) -> bool {
        let opener = self.0.borrow().opener.clone();
        match opener {
            Some(f) => {
                f(zone, button);
                true
            }
            None => false,
        }
    }

    pub fn restrip(&self) {
        if let Ok(v) = self.0.try_borrow() {
            if let Some(st) = &v.strip {
                st.queue_draw();
            }
        }
    }

    pub fn is_open(&self) -> bool {
        self.0.borrow().open
    }

    pub fn show_phone(&self) {
        let app = self.0.borrow().app.clone();
        crate::phone::refresh(&app);
        if self.is_open() {
            self.switch_tab(Tab::Phone);
        } else {
            self.0.borrow_mut().tab = Tab::Phone;
            self.set_open(true);
        }
    }

    pub fn show_tray(&self) {
        if self.is_open() {
            self.switch_tab(Tab::Tray);
        } else {
            self.0.borrow_mut().tab = Tab::Tray;
            self.set_open(true);
        }
    }

    pub fn hovered(&self) -> bool {
        self.0.try_borrow().is_ok_and(|v| v.hovered)
    }

    pub fn set_open(&self, open: bool) {
        let mut closed_scrim = None;
        let app = {
            let mut v = self.0.borrow_mut();
            if v.open == open {
                return;
            }
            v.open = open;
            v.dwell += 1;
            v.leave += 1;
            if open {
                v.feel.retain(|_| false);
            }
            if !open {
                v.tuning = false;
                v.sliding = false;
                v.tune.go(0.0, App::now(), 160, motion::FAST_EFFECTS);
                if !v.dropping {
                    v.tab = Tab::Nook;
                }
                v.day = 0;
            }
            if open && v.app.prefs.borrow().notch_close == "click" && v.scrim.is_none() {
                let weak = Rc::downgrade(&self.0);
                v.scrim = Some(crate::paint::scrim(Some(&v.monitor), move || {
                    if let Some(v) = weak.upgrade() {
                        Notch(v).set_open(false);
                    }
                }));
                v.region = (0, 0, 0, 0);
            } else if !open {
                closed_scrim = v.scrim.take();
            }
            v.app.clone()
        };
        if let Some(scrim) = closed_scrim {
            scrim.destroy();
        }
        if open {
            crate::calendar::refresh(&app);
            if self.0.borrow().tab == Tab::Tray {
                crate::share::scan(&app);
            }
        }
        app.set_open(open);
        self.refresh();
    }

    pub fn refresh(&self) {
        let start_dwell = {
            let Ok(mut v) = self.0.try_borrow_mut() else {
                let weak = Rc::downgrade(&self.0);
                glib::idle_add_local_once(move || {
                    if let Some(v) = weak.upgrade() {
                        Notch(v).refresh();
                    }
                });
                return;
            };
            if let Some(st) = &v.strip {
                st.queue_draw();
            }
            v.retarget()
        };
        if start_dwell {
            self.dwell();
        }
        self.kick();
    }

}

const POINT: Face = Face::Text(560);
const REST: Face = Face::Text(450);

impl View {
    fn header(&self) -> f64 {
        self.app.header()
    }

    fn target(&self, now: i64) -> Head {
        if self.open {
            return Head::Open;
        }
        if let Some(p) = self.app.peek.borrow().clone() {
            return Head::Peek(p);
        }
        if self.app.call.borrow().is_some() {
            return Head::Call;
        }
        Head::Idle(self.app.media.borrow().wings(now))
    }

    fn clock_width(&self) -> f64 {
        let text: String = self.shown.iter().collect();
        self.text_width(&text, 16.0, Face::Number(500)).max(self.text_width("00:00", 16.0, Face::Number(500)))
    }

    fn centre(&self) -> f64 {
        let clock = CENTRE + (self.clock_width() - self.text_width("00:00", 16.0, Face::Number(500))).max(0.0);
        // A workspace pill wider than the clock widens the island while it shows.
        if self.pill.showing() { clock.max(self.pill.width() + 2.0 * PILL_MARGIN) } else { clock }
    }

    /// The workspace pill shows in the notch: chosen in Settings, or when there is no top bar to
    /// hold it.
    fn pill_in_notch(&self) -> bool {
        self.app.prefs.borrow().notch_spaces || !self.bar_spaces()
    }

    fn note_buttons(&self, peek: &Peek) -> Vec<(Rect, String)> {
        let Peek::Notification { actions, .. } = peek else { return Vec::new() };
        if actions.is_empty() {
            return Vec::new();
        }
        let top = match self.app.dock() {
            Dock::Screen => 0.0,
            Dock::Bar => self.header(),
        };
        let half = self.w.target() / 2.0;
        let (x0, x1) = (CX - half + PANEL_PAD, CX + half - PANEL_PAD);
        let gap = GAP_CONTROLS;
        let n = actions.len() as f64;
        let bw = ((x1 - x0) - gap * (n - 1.0)) / n;
        // Buttons keep the note's side padding at the bottom too.
        let y = top + peek.height() - PANEL_PAD - 34.0;
        actions.iter().enumerate().map(|(i, (_, label))| (Rect { x: x0 + i as f64 * (bw + gap), y, w: bw, h: 34.0 }, label.clone())).collect()
    }

    fn hot_at(&self, x: f64, y: f64, now: i64) -> Option<Hot> {
        if !self.open {
            if let Head::Peek(p @ Peek::Notification { .. }) = &self.head {
                return self.note_buttons(p).iter().position(|(r, _)| r.contains(x, y)).map(Hot::Action);
            }
            return None;
        }
        if self.volume_zone(now).is_some_and(|z| z.contains(x, y)) {
            return Some(Hot::Volume);
        }
        if self.gear_rect(now).contains(x, y) {
            return Some(Hot::Gear);
        }
        if let Some(i) = self.tab_rects().iter().position(|(r, _, _)| r.contains(x, y)) {
            return Some(Hot::Tab(i));
        }
        match self.tab {
            Tab::Nook => {
                if self.can_switch(now) && self.switch_rect().contains(x, y) {
                    return Some(Hot::CallSwitch);
                }
                if self.showing_call(now) {
                    let (mute, open) = self.call_buttons();
                    if mute.contains(x, y) {
                        return Some(Hot::CallMute);
                    }
                    if open.contains(x, y) {
                        return Some(Hot::CallOpen);
                    }
                }
                if let Some(i) = (0..TOOLS.len()).find(|&i| self.tool_rect(i).contains(x, y)) {
                    return Some(Hot::Tool(i));
                }
                if let Some(i) = self.day_rects().iter().position(|(r, _)| r.contains(x, y)) {
                    return Some(Hot::Day(i));
                }
                let p = self.player(now)?;
                if let Some(i) = p.controls.iter().position(|(r, _)| r.contains(x, y)) {
                    return Some(Hot::Control(i));
                }
                let m = self.app.media.borrow();
                (p.seek.contains(x, y) && m.timing == Timing::Timed && m.can_seek).then_some(Hot::Seek)
            }
            Tab::Phone => {
                let phone = crate::phone::phone()?;
                if crate::phone::next().is_some() && self.phone_switch_rect().contains(x, y) {
                    return Some(Hot::PhoneSwitch);
                }
                if !phone.reachable {
                    return None;
                }
                if let Some(i) = (0..crate::phone::ACTIONS.len()).find(|&i| self.phone_action_rect(i).contains(x, y)) {
                    return Some(Hot::PhoneAction(i));
                }
                phone_rows(crate::phone::call().is_some(), phone.notes.len()).find(|&(row, i)| phone.notes[i].dismissable && self.note_rect(row).contains(x, y)).map(|(_, i)| Hot::PhoneDismiss(i))
            }
            Tab::Tray => {
                for i in 0..crate::tray::len().min(TILES) {
                    if self.remove_rect(i).contains(x, y) {
                        return Some(Hot::Remove(i));
                    }
                    if self.tile_rect(i).contains(x, y) {
                        return Some(Hot::Tile(i));
                    }
                }
                if let Some(i) = (0..crate::share::senders().len().min(CHIPS)).find(|&i| self.chip_rect(i).contains(x, y)) {
                    return Some(Hot::Sender(i));
                }
                (0..crate::share::devices().len().min(DEVICES)).find(|&i| self.device_rect(i).contains(x, y)).map(Hot::Device)
            }
        }
    }

    /// A card or tile corner in the open sheet: `radius` at the default Roundness, following the
    /// notch's own roundness and never past a pill.
    fn corner(&self, radius: f64, h: f64) -> f32 {
        crate::metrics::scaled(radius, self.app.notch_round(), h) as f32
    }

    fn body_top(&self) -> f64 {
        self.header() + 10.0
    }

    fn tab_rects(&self) -> Vec<(Rect, &'static str, &'static str)> {
        let mid = self.header() / 2.0;
        let mut x = CX - SHEET_W / 2.0 + 14.0;
        let mut out = Vec::with_capacity(TABS.len());
        for (_, icon, label) in TABS {
            let w = 40.0 + self.text_width(label, 14.0, POINT).ceil() + 12.0;
            out.push((Rect { x, y: mid - 13.0, w, h: 26.0 }, icon, label));
            x += w + 6.0;
        }
        out
    }

    fn gear_rect(&self, now: i64) -> Rect {
        let r1 = self.wings(&Head::Open, now).3;
        let mid = self.header() / 2.0;
        Rect { x: r1 - 22.0, y: mid - 13.0, w: 26.0, h: 26.0 }
    }

    fn over_body(&self, x: f64, y: f64, now: i64) -> bool {
        let half = self.w.value(now).max(self.w.target()) / 2.0 + 4.0;
        let depth = self.header() + self.depth.value(now).max(self.depth.target());
        (x - CX).abs() <= half && y <= depth
    }

    fn landing_at(&self, x: f64, y: f64) -> Option<Landing> {
        if !self.open || self.tab != Tab::Tray {
            return None;
        }
        if let Some(i) = (0..crate::share::senders().len().min(CHIPS)).find(|&i| self.chip_rect(i).contains(x, y)) {
            return Some(Landing::Sender(i));
        }
        if let Some(i) = (0..crate::share::devices().len().min(DEVICES)).find(|&i| self.device_rect(i).contains(x, y)) {
            return Some(Landing::Device(i));
        }
        if self.share_rect().contains(x, y) {
            return Some(Landing::Share);
        }
        Some(Landing::Files)
    }

    fn badge_icon(&self) -> Option<gdk::Paintable> {
        let player = self.app.media.borrow().player.clone();
        let mut cache = self.badge.borrow_mut();
        if cache.as_ref().is_none_or(|(k, _)| *k != player) {
            let scale = self.window.scale_factor().max(1);
            *cache = Some((player.clone(), draw::app_icon(&self.app.apps, &player, 26, scale)));
        }
        cache.as_ref().and_then(|(_, p)| p.clone())
    }

    fn island_width(&self, summary: &str, body: &str) -> f64 {
        let size = if body.is_empty() { 36.0 } else { 46.0 };
        let title = self.text_width(summary, 16.0, Face::Text(600));
        let line = self.text_width(body, 15.0, REST);
        let stamp = self.text_width("now", 14.0, REST);
        (18.0 + size + 14.0 + (title + 12.0 + stamp).max(line) + 22.0).clamp(340.0, 580.0)
    }

    /// The top bar shows the workspace row beside its logo, so the notch leaves the dots out.
    fn bar_spaces(&self) -> bool {
        self.strip.is_some() && (self.app.wide() || self.app.menubar())
    }

    fn spaces(&self) -> (Vec<crate::compositor::Space>, usize) {
        let all = self.app.spaces.borrow();
        let list: Vec<_> = all.iter().filter(|s| s.output == self.output && (s.filled || s.active)).cloned().collect();
        let active = list.iter().position(|s| s.active).unwrap_or(0);
        (with_next(list, &all, &self.output), active)
    }


    fn room(&self) -> f64 {
        let n = self.app.lights.borrow().count();
        if n == 0 { 0.0 } else { 8.0 + 12.0 * n as f64 }
    }

    /// Left edge for a leading icon or disc `size` tall: as far from the shape's left edge as
    /// from its top and bottom. `l0` is the left wing's start (12 px in from the edge).
    fn lead(&self, l0: f64, size: f64) -> f64 {
        l0 - 12.0 + ((self.header() - size) / 2.0).max(0.0)
    }

    /// Applies the notch roundness to a corner radius. A closed notch can grow no rounder
    /// than a pill; an open one has more room.
    fn rounded(&self, r: f64, depth: f64) -> f64 {
        let limit = if depth > 0.0 { 60.0 } else { (self.header() / 2.0).floor() };
        (r * self.app.notch_round()).clamp(0.0, limit.max(r))
    }

    fn geometry(&self, head: &Head, _now: i64) -> (f64, f64, f64) {
        let room = 2.0 * self.room();
        match head {
            Head::Idle(media) => {
                // The cover sits in the left wing and the wave on the right, like the iPhone's island.
                let (left, right): (f64, f64) = if *media { (28.0, 8.0 + crate::bars::W as f64) } else { (0.0, 0.0) };
                let side = left.max(right);
                (self.centre() + room + if side > 0.0 { 2.0 * (side - 8.0).max(0.0) } else { 0.0 }, 0.0, self.idle_r())
            }
            Head::Peek(p @ Peek::Notification { summary, body, .. }) => {
                let depth = match self.app.dock() {
                    Dock::Screen => (p.height() - self.header()).max(0.0),
                    Dock::Bar => p.height(),
                };
                (self.island_width(summary, body) + room, depth, p.radius())
            }
            // The device's name sits in the right wing, which grows to hold it (up to a point; past
            // that it ends in "…"), so it never runs into the clock.
            Head::Peek(Peek::Output { name, .. }) => {
                let wing = (self.text_width(name, 14.0, POINT) + 28.0).clamp(Peek::Output { name: String::new(), icon: "" }.wing(), 260.0);
                (self.centre() + room + 2.0 * wing, 0.0, self.idle_r())
            }
            Head::Peek(p) => (self.centre() + room + 2.0 * p.wing(), 0.0, self.idle_r()),
            Head::Call => {
                // Size for the widest digits of the current length, so the island holds still
                // while the seconds tick (it only grows when a digit is added).
                let time = self.call_time(App::now());
                let widest = ['0', '8'].iter().map(|d| self.text_width(&time.chars().map(|c| if c.is_ascii_digit() { *d } else { c }).collect::<String>(), 15.0, Face::Number(560))).fold(0.0, f64::max);
                let left = 30.0 + widest.max(self.text_width("0:00", 15.0, Face::Number(560))).ceil();
                let right = 12.0 + crate::bars::W as f64;
                (self.centre() + room + 2.0 * (left.max(right) - 8.0).max(0.0), 0.0, self.idle_r())
            }
            Head::Open => (SHEET_W, OPEN_DEPTH + ISLAND_PAD * self.float.target().clamp(0.0, 1.0), 26.0),
        }
    }

    fn retarget(&mut self) -> bool {
        let now = App::now();
        let timed = self.app.media.borrow().timing == Timing::Timed;
        if timed && !self.timed {
            self.fill.set(0.0);
            self.fill.go(1.0, now, 520, motion::EMPHASIZED_DECEL);
        }
        self.timed = timed;
        let clock: Vec<char> = self.app.clock.borrow().chars().collect();
        if clock != self.shown {
            let mut roll = vec![None; clock.len()];
            if clock.len() == self.shown.len() {
                let mut k = 0;
                for i in (0..clock.len()).rev() {
                    if clock[i] != self.shown[i] {
                        roll[i] = Some(now + k * 40_000);
                        k += 1;
                    }
                }
            }
            self.was = std::mem::replace(&mut self.shown, clock);
            self.roll = roll;
        }
        // Before sizing the island: a workspace pill in the notch can widen it.
        {
            let (list, active) = self.spaces();
            let always = self.app.prefs.borrow().bar_spaces;
            let in_notch = self.pill_in_notch();
            let filled: Vec<bool> = list.iter().map(|s| s.filled).collect();
            if let Some(st) = &self.strip {
                st.set_spaces(if in_notch { Vec::new() } else { filled.clone() }, active, always && !in_notch);
            }
            self.pill.update(if in_notch { filled } else { Vec::new() }, active, false, now);
        }
        let want = self.target(now);
        let was_peek = matches!(self.want, Head::Peek(_));
        let (w, depth, r) = self.geometry(&want, now);
        let r = self.rounded(r, depth);
        let (ms, curve) = if self.open {
            (500, motion::DEFAULT_SPATIAL)
        } else if depth > 0.0 || self.depth.target() > 0.0 && matches!(want, Head::Peek(_)) {
            (440, motion::DEFAULT_SPATIAL)
        } else if matches!(want, Head::Peek(_)) {
            (350, motion::FAST_SPATIAL)
        } else {
            (450, motion::EMPHASIZED_DECEL)
        };
        self.w.go(w, now, ms, curve);
        self.depth.go(depth, now, ms, curve);
        self.r.go(r, now, ms, curve);
        if self.open && self.sheet.target() < 1.0 {
            self.sheet.after(1.0, now, ms * 11 / 20, 220, motion::STANDARD);
        } else if !self.open {
            self.sheet.go(0.0, now, 90, motion::FAST_EFFECTS);
        }
        self.back.go(if matches!(want, Head::Peek(_)) { 1.0 } else { 0.0 }, now, 200, motion::STANDARD);
        let hush = if self.app.dock() == Dock::Screen && matches!(want, Head::Peek(Peek::Notification { .. })) { 0.0 } else { 1.0 };
        if self.hush.target() != hush {
            if hush == 0.0 {
                self.hush.go(0.0, now, 120, motion::FAST_EFFECTS);
            } else {
                self.hush.after(1.0, now, 180, 260, motion::STANDARD);
            }
        }
        let gauge = match &want {
            Head::Peek(Peek::Volume { level, .. }) => Some((*level, matches!(self.head, Head::Peek(Peek::Volume { .. })))),
            Head::Peek(Peek::Osd { level: Some(level), .. }) => Some((*level, matches!(self.head, Head::Peek(Peek::Osd { level: Some(_), .. })))),
            _ => None,
        };
        if let Some((level, same)) = gauge {
            if !same {
                self.meter.set(level);
            }
            self.meter.go(level.clamp(0.0, 1.0), now, 200, motion::STANDARD);
        }
        {
            let lights = self.app.lights.borrow();
            self.dots[0].go(if lights.sharing() { 1.0 } else { 0.0 }, now, 320, motion::STANDARD);
            self.dots[1].go(if lights.capture() { 1.0 } else { 0.0 }, now, 320, motion::STANDARD);
            if lights.capture() {
                self.capture.set(if lights.camera.is_empty() { ORANGE } else { GREEN });
            }
        }
        let playing = self.app.media.borrow().playing;
        self.art_scale.go(if playing { 1.0 } else { 0.85 }, now, 400, motion::DEFAULT_SPATIAL);
        if self.head.same(&want) {
            self.head = want.clone();
            if self.fade.target() < 1.0 {
                self.fade.go(1.0, now, 200, motion::STANDARD);
            }
        } else if self.fade.target() > 0.0 {
            self.fade.go(0.0, now, 110, motion::FAST_EFFECTS);
        }
        let now_peek = matches!(want, Head::Peek(_));
        self.want = want;
        was_peek && !now_peek && self.hovered && !self.open && self.app.prefs.borrow().hover
    }

    fn step(&mut self, now: i64) -> bool {
        if self.fade.target() == 0.0 && self.fade.settled(now) {
            self.head = self.want.clone();
            let shape_left = self.w.left_ms(now).max(self.depth.left_ms(now));
            self.fade.after(1.0, now, shape_left * 2 / 5, 200, motion::STANDARD);
        }
        self.update_region(now);
        self.update_frost(now);
        self.place_bars(now);
        if self.grown && !self.app.wide() && !self.app.menubar() && self.widen.target() == 0.0 && self.widen.settled(now) && self.menu.settled(now) {
            self.grow(false);
        }
        let wide = self.widen.value(now).max(self.menu.value(now));
        if !self.widen.settled(now) || !self.menu.settled(now) || !self.glass.settled(now) || !self.float.settled(now) || wide > 0.0 && (!self.w.settled(now) || !self.r.settled(now)) {
            if let Some(st) = &self.strip {
                st.queue_draw();
            }
        }
        let glass = if self.glassy() { 1.0 } else { 0.0 };
        if self.glass.target() != glass {
            self.glass.go(glass, now, 360, motion::STANDARD);
        }
        let tweens = [&self.w, &self.depth, &self.r, &self.fade, &self.sheet, &self.back, &self.meter, &self.art_scale, &self.fill, &self.tune, &self.hush, &self.tab_fade, &self.flip, &self.float, &self.widen, &self.menu, &self.glass];
        let moving = self.feel.busy(now) || tweens.into_iter().chain(self.glow.iter()).chain(self.dots.iter()).any(|t| !t.settled(now));
        let rolling = !motion::reduced() && self.roll.iter().flatten().any(|t| now < t + ROLL);
        let showing = self.pill.showing();
        let pill = self.pill.step(now);
        if showing != self.pill.showing() && !self.open {
            // The pill folded away: the island goes back to the clock's width.
            let (w, _, _) = self.geometry(&self.want.clone(), now);
            self.w.go(w, now, 450, motion::EMPHASIZED_DECEL);
        }
        moving || rolling || pill || self.app.palette_moving(now)
    }

    fn call_time(&self, now: i64) -> String {
        let since = self.app.call.borrow().as_ref().map_or(now, |c| c.since);
        let t = ((now - since).max(0) / 1_000_000) as u64;
        if t >= 3600 { format!("{}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60) } else { format!("{}:{:02}", t / 60, t % 60) }
    }

    fn place_bars(&self, now: i64) {
        let call = matches!(self.head, Head::Call);
        let show = !self.open && (matches!(self.head, Head::Idle(true)) || call);
        if show {
            let (_, _, _, r1) = self.wings(&self.head, now);
            let mid = self.header() / 2.0;
            let x = (r1 - crate::bars::W as f64).round() as i32;
            let y = (mid + self.body_lift(now) - crate::bars::H as f64 / 2.0).round() as i32;
            if self.bars.margin_start() != x {
                self.bars.set_margin_start(x);
            }
            if self.bars.margin_top() != y {
                self.bars.set_margin_top(y);
            }
            let pal = self.pal.get();
            let tint = if call { Some(GREEN) } else { self.app.media.borrow().tint };
            self.bars.ink(tint.map_or(pal.ink_sec, |t| legible(t, &pal)));
            self.bars.set_opacity(self.fade.value(now).clamp(0.0, 1.0));
            if !self.bars.is_visible() {
                self.bars.set(&self.app.spectrum.borrow());
            }
        }
        if self.bars.is_visible() != show {
            self.bars.set_visible(show);
        }
    }

    fn off(&self) -> i32 {
        (self.span - WIDTH as i32).max(0) / 2
    }

    fn grow(&mut self, wide: bool) {
        self.grown = wide;
        let span = if wide { self.monitor.geometry().width().max(WIDTH as i32) } else { WIDTH as i32 };
        let h = (self.header() + ROOM) as i32;
        if let Some(st) = &self.strip {
            st.set_size_request(span, h);
        }
        self.window.set_default_size(span, h);
        self.region = (0, 0, 0, 0);
    }

    fn lift(&self, now: i64) -> f64 {
        self.lift_at(self.float.value(now))
    }

    /// Where the content sits below the shape's top: the float lift, plus the island's top
    /// padding while it is open.
    fn content_lift(&self, now: i64) -> f64 {
        // Peeks (notifications and the like) are laid out in their own height; only the open
        // sheet gets the island's extra top padding.
        let open = if matches!(self.head, Head::Open) || self.open { (self.depth.value(now).max(0.0) / OPEN_DEPTH).clamp(0.0, 1.0) } else { 0.0 };
        self.body_lift(now) + ISLAND_PAD * self.float.value(now).clamp(0.0, 1.0) * open
    }

    fn body_lift(&self, now: i64) -> f64 {
        self.lift_at(self.float.value(now))
    }

    /// The island floats one compositor gap below the screen edge, with or without a menu bar.
    fn lift_at(&self, float: f64) -> f64 {
        self.app.lift() * float.clamp(0.0, 1.0)
    }

    fn band(&self, now: i64) -> f64 {
        let bar = (self.menu.value(now) * (1.0 - self.widen.value(now))).clamp(0.0, 1.0);
        self.header() + self.lift(now) * (1.0 + bar)
    }

    fn idle_r(&self) -> f64 {
        if self.app.floating() { (self.header() / 2.0).floor() } else { 14.0 }
    }

    fn update_region(&mut self, now: i64) {
        let w = self.w.value(now).max(self.w.target()) + 12.0;
        let lift = self.lift_at(self.float.value(now).max(self.float.target()));
        let band = self.band(now).max(self.header() + 2.0 * lift * self.menu.target() * (1.0 - self.widen.target())).ceil() as i32;
        let body = lift;
        let h = self.header() + lift.max(body) + self.depth.value(now).max(self.depth.target());
        let mut rect = ((CX - w / 2.0).floor() as i32 + self.off(), 0, w.ceil() as i32, h.ceil() as i32);
        if self.dropping {
            rect = (0, 0, self.span.max(WIDTH as i32), (self.header() + ROOM) as i32);
        }
        if rect == self.region {
            return;
        }
        if let Some(surface) = self.window.surface() {
            let region = cairo::Region::create_rectangle(&cairo::RectangleInt::new(rect.0, rect.1, rect.2, rect.3));
            if self.app.dock() == Dock::Screen {
                let _ = region.union_rectangle(&cairo::RectangleInt::new(self.off(), 0, WIDTH as i32, (self.header() + lift) as i32));
            }
            if (self.app.wide() || self.app.menubar()) && self.span > WIDTH as i32 {
                let _ = region.union_rectangle(&cairo::RectangleInt::new(0, 0, self.span.max(WIDTH as i32), band));
            }
            surface.set_input_region(Some(&region));
            if let Some(scrim) = self.scrim.as_ref().and_then(|w| w.surface()) {
                let geo = self.monitor.geometry();
                let full = cairo::Region::create_rectangle(&cairo::RectangleInt::new(0, 0, geo.width(), geo.height()));
                let hole = region.copy();
                hole.translate((geo.width() - self.window.width()) / 2, 0);
                let _ = full.subtract(&hole);
                scrim.set_input_region(Some(&full));
            }
            self.region = rect;
        }
    }

    fn frosted(&self) -> bool {
        self.app.palette.get().body.a < 1.0
    }

    fn glassy(&self) -> bool {
        if kit::effects::reduced() || !self.app.menubar() || self.app.wide() {
            return false;
        }
        match self.app.prefs.borrow().menubar_look.as_str() {
            "glass" => true,
            "clear" => false,
            _ => self.app.spaces.borrow().iter().any(|s| s.output == self.output && s.active && s.filled),
        }
    }

    fn update_frost(&mut self, now: i64) {
        // Reduced effects draw the menu bar and panels solid, without the compositor's blur.
        if kit::effects::reduced() {
            self.panes.place(None, (0.0, 0.0));
            if let (Some((effect, _)), Some(blur)) = (self.effect.take(), self.app.blur.as_ref()) {
                blur.detach(effect);
                self.frost.clear();
            }
            return;
        }
        let band = self.glass.value(now) > 0.02 || self.glass.target() > 0.5;
        let Some(blur) = self.app.blur.as_ref() else {
            // GTK owns the blur: lay the pane under the band.
            let bounds = (self.window.width() as f64, self.window.height() as f64);
            let bar = (self.app.dock() == Dock::Screen && band && self.span > WIDTH as i32).then_some((0.0, 0.0, self.span as f64, self.band(now)));
            self.panes.place(bar, bounds);
            return;
        };
        if self.app.dock() != Dock::Screen || (!self.frosted() && !band && self.frost.is_empty()) {
            return;
        }
        let Some(surface) = self.window.surface() else { return };
        let Some(current) = blur::Blur::surface_id(&surface) else { return };
        if self.effect.as_ref().is_some_and(|(_, id)| *id != current) {
            if let Some((old, _)) = self.effect.take() {
                blur.detach(old);
            }
            self.frost.clear();
        }
        if self.effect.is_none() {
            self.effect = blur.attach(&surface);
        }
        let Some((effect, _)) = self.effect.as_ref() else { return };
        let depth = self.header() + self.depth.value(now).max(0.0);
        let f = self.float.value(now).clamp(0.0, 1.0);
        let off = self.off();
        let mut rects = if self.frosted() {
            shape::rows(CX, self.body_lift(now), self.w.value(now), depth, self.r.value(now), 6.0 * (1.0 - f), 0.0).into_iter().map(|(x, y, w, h)| (x + off, y, w, h)).collect()
        } else {
            Vec::new()
        };
        if band && self.span > WIDTH as i32 {
            rects.push((0, 0, self.span, self.band(now).round() as i32));
        }
        if rects != self.frost {
            blur.set(effect, &rects);
            self.frost = rects;
        }
    }

    fn layout(&self, text: &str, px: f64, face: Face) -> pango::Layout {
        draw::shaped(&self.canvas, &self.app.cfg, text, px, face)
    }

    fn text_width(&self, text: &str, px: f64, face: Face) -> f64 {
        draw::width(&self.canvas, &self.app.cfg, text, px, face)
    }

    #[allow(clippy::too_many_arguments)]
    fn text(&self, s: &gtk::Snapshot, text: &str, px: f64, face: Face, color: Rgb, x: f64, mid: f64, width: f64, align: pango::Alignment) {
        draw::text(s, &self.canvas, &self.app.cfg, text, px, face, color, x, mid, width, align);
    }

    fn icon(&self, s: &gtk::Snapshot, name: &str, px: f64, fill: bool, color: Rgb, cx: f64, mid: f64) {
        draw::icon(s, &self.canvas, &self.app.cfg, name, px, fill, color, cx, mid);
    }

    fn draw(&self, canvas: &Canvas, s: &gtk::Snapshot) {
        let now = canvas.frame_clock().map(|c| c.frame_time()).unwrap_or_else(glib::monotonic_time);
        self.pal.set(self.app.palette_at(now));
        if let Some(st) = &self.strip {
            if self.app.palette_moving(now) {
                st.queue_draw();
            }
        }
        let pal = self.pal.get();
        let header = self.header();
        let width = self.w.value(now);
        let sheet = self.depth.value(now).max(0.0);
        let lift = self.body_lift(now);
        let f = self.float.value(now).clamp(0.0, 1.0);
        let wide = self.widen.value(now).clamp(0.0, 1.0);
        let (top, depth) = match self.app.dock() {
            Dock::Bar => (header, sheet),
            Dock::Screen => (lift, header + sheet),
        };
        let radius = self.r.value(now);
        let fillet = 6.0 * (1.0 - f);
        let parts = if pal.body.a >= 1.0 || f > 0.001 { shape::island_parts(CX, top, width, depth, radius, fillet, radius * f) } else { None };
        let path = if parts.is_none() { shape::docked(CX, top, width, depth, radius, fillet) } else { None };

        let clip_to = |s: &gtk::Snapshot| match (&parts, &path) {
            (Some((rr, _)), _) => {
                s.push_rounded_clip(rr);
                true
            }
            (None, Some(p)) => {
                s.push_fill(p, gsk::FillRule::Winding);
                true
            }
            _ => false,
        };
        if let Some(p) = &parts {
            draw::slab(s, p, pal.body);
        }
        if let Some(p) = &path {
            s.append_fill(p, gsk::FillRule::Winding, &pal.body.rgba());
        }
        if wide > 0.01 && sheet > 1.0 && self.app.dock() == Dock::Screen {
            if let Some((_, Some(coves))) = shape::docked_parts(CX, header, width, sheet, radius, 6.0 * wide) {
                s.append_fill(&coves, gsk::FillRule::Winding, &pal.body.rgba());
            }
        }
        let clip = self.app.dock() == Dock::Screen && clip_to(s);
        s.save();
        s.translate(&graphene::Point::new(0.0, self.content_lift(now) as f32));
        let fade = self.fade.value(now).clamp(0.0, 1.0);
        if fade > 0.01 {
            s.push_opacity(fade);
            self.draw_head(s, now);
            s.pop();
        }
        self.draw_clock(s, now);
        self.draw_lights(s, now);
        s.restore();
        if clip {
            s.pop();
        }
        let shown = self.sheet.value(now).clamp(0.0, 1.0);
        if sheet > 1.0 && shown > 0.01 && clip_to(s) {
            s.save();
            s.translate(&graphene::Point::new(0.0, self.content_lift(now) as f32));
            s.push_opacity(shown);
            self.draw_sheet(s, now);
            s.pop();
            s.restore();
            s.pop();
        }
    }

}

mod imp {
    use super::View;
    use gtk::subclass::prelude::*;
    use gtk::{glib, prelude::*};
    use std::cell::RefCell;
    use std::rc::Weak;

    #[derive(Default)]
    pub struct Canvas {
        pub(super) view: RefCell<Weak<RefCell<View>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Canvas {
        const NAME: &'static str = "DigitCanvas";
        type Type = super::Canvas;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Canvas {}

    impl WidgetImpl for Canvas {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let Some(view) = self.view.borrow().upgrade() else { return };
            let Ok(v) = view.try_borrow() else {
                self.obj().queue_draw();
                return;
            };
            v.draw(&self.obj(), snapshot);
            // Something it drew has changed since the last time, and goes on from here.
            let now = self.obj().frame_clock().map(|c| c.frame_time()).unwrap_or_else(glib::monotonic_time);
            if !v.ticking && v.feel.busy(now) {
                let weak = std::rc::Rc::downgrade(&view);
                glib::idle_add_local_once(move || {
                    if let Some(view) = weak.upgrade() {
                        super::Notch(view).kick();
                    }
                });
            }
        }
    }
}

glib::wrapper! {
    pub struct Canvas(ObjectSubclass<imp::Canvas>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for Canvas {
    fn default() -> Self {
        glib::Object::new()
    }
}

fn lifted(v: &Rc<RefCell<View>>, y: f64) -> f64 {
    y - v.try_borrow().map(|v| v.content_lift(App::now())).unwrap_or(0.0)
}

fn legible(tint: Rgb, pal: &kit::palette::Palette) -> Rgb {
    let mut c = tint;
    for k in 1..=6 {
        if kit::palette::apca(c, pal.body).abs() >= 48.0 {
            return c;
        }
        c = tint.mix(pal.ink_main, 1.0 - 0.15 * k as f64);
    }
    c
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phone_call_reserves_first_row() {
        assert_eq!(phone_rows(false, 4).collect::<Vec<_>>(), vec![(0, 0), (1, 1), (2, 2)]);
        assert_eq!(phone_rows(true, 4).collect::<Vec<_>>(), vec![(1, 0), (2, 1)]);
        assert_eq!(phone_rows(true, 0).count(), 0);
    }
}

/// The workspaces in use on `output`, and when the active one is the last of them and has
/// windows, an empty one after it: the place a step to the right goes. niri keeps that empty
/// workspace already; on Hyprland it's the next number no other screen is using.
fn with_next(mut list: Vec<crate::compositor::Space>, all: &[crate::compositor::Space], output: &str) -> Vec<crate::compositor::Space> {
    let Some(last) = list.last().filter(|s| s.active && s.filled).cloned() else { return list };
    let next = all.iter().filter(|s| s.output == output && s.idx > last.idx && !s.filled).min_by_key(|s| s.idx).cloned();
    let next = next.unwrap_or_else(|| {
        let mut idx = last.idx + 1;
        while all.iter().any(|s| s.idx == idx) {
            idx += 1;
        }
        crate::compositor::Space { id: idx, idx, output: output.to_string(), active: false, focused: false, filled: false }
    });
    list.push(next);
    list
}

#[cfg(test)]
mod next_space_tests {
    use super::with_next;
    use crate::compositor::Space;

    fn space(idx: u64, output: &str, active: bool, filled: bool) -> Space {
        Space { id: idx, idx, output: output.into(), active, focused: active, filled }
    }

    fn shown(all: &[Space], output: &str) -> Vec<u64> {
        let list: Vec<Space> = all.iter().filter(|s| s.output == output && (s.filled || s.active)).cloned().collect();
        with_next(list, all, output).iter().map(|s| s.idx).collect()
    }

    #[test]
    fn the_last_workspace_in_use_shows_an_empty_one_after_it() {
        let mut all: Vec<Space> = (1..=4).map(|i| space(i, "DP-1", i == 4, true)).collect();
        assert_eq!(shown(&all, "DP-1"), [1, 2, 3, 4, 5]);
        all[3].active = false;
        all[2].active = true;
        assert_eq!(shown(&all, "DP-1"), [1, 2, 3, 4], "only on the rightmost");
    }

    #[test]
    fn an_empty_active_workspace_adds_nothing_more() {
        let mut all: Vec<Space> = (1..=4).map(|i| space(i, "DP-1", false, true)).collect();
        all.push(space(5, "DP-1", true, false));
        assert_eq!(shown(&all, "DP-1"), [1, 2, 3, 4, 5]);
    }

    #[test]
    fn niris_trailing_empty_workspace_is_the_one_shown() {
        let mut all: Vec<Space> = (1..=3).map(|i| space(i, "eDP-1", i == 3, true)).collect();
        all.push(Space { id: 42, idx: 4, output: "eDP-1".into(), active: false, focused: false, filled: false });
        let list: Vec<Space> = all.iter().filter(|s| s.filled || s.active).cloned().collect();
        assert_eq!(with_next(list, &all, "eDP-1").last().map(|s| s.id), Some(42));
    }

    #[test]
    fn a_number_another_screen_uses_is_skipped() {
        let mut all: Vec<Space> = (1..=2).map(|i| space(i, "DP-1", i == 2, true)).collect();
        all.push(space(3, "HDMI-A-1", true, true));
        assert_eq!(shown(&all, "DP-1"), [1, 2, 4]);
    }
}
