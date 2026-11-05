use crate::app::{App, Ending, Peek, Timing};
use crate::config::Dock;
use kit::motion::{self, Tween};
use kit::palette::Rgb;
use crate::draw::{self, Face};
use crate::audio::{self, Ask};
use crate::{blur, media};
use kit::shape;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{cairo, gdk, glib, graphene, gsk, pango};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

const WIDTH: f64 = 860.0;
const ROOM: f64 = 420.0;
const CENTRE: f64 = 116.0;
const CX: f64 = WIDTH / 2.0;
const SHEET_W: f64 = 780.0;
const OPEN_DEPTH: f64 = 156.0;
/// Extra room above an open Dynamic Island's content, so its top edge matches the sides.
const ISLAND_PAD: f64 = 9.0;
const LEFT: f64 = CX - SHEET_W / 2.0 + 22.0;
const RIGHT: f64 = CX + SHEET_W / 2.0 - 22.0;
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
const DOT: f64 = 5.0;
const PILL: f64 = 16.0;
const GAP: f64 = 5.0;
const GREEN: Rgb = Rgb::new(0.19, 0.82, 0.35);
const MORPH_MS: i64 = 680;
const MORPH: motion::Curve = motion::Curve::new(0.33, 0.0, 0.15, 1.0);
const ORANGE: Rgb = Rgb::new(1.0, 0.62, 0.04);
const PURPLE: Rgb = Rgb::new(0.75, 0.35, 0.95);
const ACTION_BLUE: Rgb = Rgb::new(0.04, 0.52, 1.0);
const RED: Rgb = Rgb::new(1.0, 0.27, 0.23);

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
    ticking: bool,
    /// Fraction of the track under the pointer while the progress bar is being dragged.
    scrub: Option<f64>,
    fill: Tween,
    timed: bool,
    pointer: Option<(f64, f64)>,
    hot: Option<Hot>,
    glow: [Tween; 12],
    /// The open notch shows the player instead of the call it would normally show; cleared when the call ends.
    media_over_call: std::cell::Cell<bool>,
    /// Fades the media spot back in after flipping between the call and the player.
    flip: Tween,
    tuning: bool,
    tune: Tween,
    sliding: bool,
    dots: [Tween; 2],
    capture: std::cell::Cell<Rgb>,
    hush: Tween,
    spot: Tween,
    spots: usize,
    corner: Option<gtk::Window>,
    control_corner: Option<gtk::Window>,
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
        match &strip {
            Some(st) => {
                st.set_size_request(span, (header + ROOM) as i32);
                st.set_hexpand(true);
                layer.set_halign(gtk::Align::Center);
                layer.set_valign(gtk::Align::Start);
                let root = gtk::Overlay::new();
                root.set_child(Some(st));
                root.add_overlay(&layer);
                window.set_child(Some(&root));
            }
            None => window.set_child(Some(&layer)),
        }
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
            ticking: false,
            scrub: None,
            fill: Tween::still(1.0),
            timed: true,
            pointer: None,
            hot: None,
            glow: [Tween::still(0.0); 12],
            media_over_call: std::cell::Cell::new(false),
            flip: Tween::still(1.0),
            tuning: false,
            tune: Tween::still(0.0),
            sliding: false,
            dots: [Tween::still(0.0); 2],
            capture: std::cell::Cell::new(ORANGE),
            hush: Tween::still(1.0),
            spot: Tween::still(0.0),
            spots: 0,
            corner: None,
            control_corner: None,
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
            v.corner = Some(crate::spaces::corner(app, monitor));
            v.control_corner = Some(crate::control::corner(app, monitor));
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
            if weak.upgrade().is_some_and(|v| Notch(v).wheel(dy)) {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        canvas.add_controller(scroll);
        // Hyprland offers MOVE as the only action before any negotiation; files are still only read.
        let target = gtk::DropTarget::new(glib::Type::INVALID, gdk::DragAction::COPY | gdk::DragAction::MOVE);
        target.set_types(&[gdk::FileList::static_type(), gdk::Texture::static_type(), String::static_type()]);
        let host: gtk::Widget = self.0.borrow().window.child().unwrap_or_else(|| canvas.clone().upcast());
        let to_canvas = {
            let (host, canvas) = (host.clone(), canvas.clone());
            move |x: f64, y: f64| host.compute_point(&canvas, &graphene::Point::new(x as f32, y as f32)).map(|p| (p.x() as f64, p.y() as f64)).unwrap_or((x, y))
        };
        let weak = Rc::downgrade(&self.0);
        target.connect_enter(move |_, _, _| {
            if let Some(v) = weak.upgrade() {
                Notch(v).drag_enter();
            }
            gdk::DragAction::COPY
        });
        let weak = Rc::downgrade(&self.0);
        let map = to_canvas.clone();
        target.connect_motion(move |_, x, y| {
            if let Some(v) = weak.upgrade() {
                let (x, y) = map(x, y);
                let y = lifted(&v, y);
                Notch(v).drag_motion(x, y);
            }
            gdk::DragAction::COPY
        });
        let weak = Rc::downgrade(&self.0);
        target.connect_leave(move |_| {
            if let Some(v) = weak.upgrade() {
                Notch(v).drag_leave();
            }
        });
        let weak = Rc::downgrade(&self.0);
        let map = to_canvas.clone();
        target.connect_drop(move |_, value, x, y| {
            let Some(v) = weak.upgrade() else { return false };
            let (x, y) = map(x, y);
            let paths: Vec<std::path::PathBuf> = if let Ok(list) = value.get::<gdk::FileList>() {
                list.files().iter().filter_map(|f| f.path()).collect()
            } else if let Ok(texture) = value.get::<gdk::Texture>() {
                crate::tray::keep_texture(&texture).into_iter().collect()
            } else if let Ok(text) = value.get::<String>() {
                crate::tray::keep_text(&text)
            } else {
                Vec::new()
            };
            let y = lifted(&v, y);
            Notch(v).dropped(paths, x, y)
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
            let target = st.clone();
            let open_menu: Rc<dyn Fn(Zone, u32)> = Rc::new(move |zone: Zone, button: u32| {
                let Some(v) = weak.upgrade() else { return };
                let (app, below, monitor) = {
                    let v = v.borrow();
                    (v.app.clone(), v.band(App::now()) + 5.0, Some(v.monitor.clone()))
                };
                let Some(left) = target.zone_x(zone) else { return };
                target.hold(Some(zone));
                let held = target.clone();
                crate::popmenu::then(move || {
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
                        crate::datepop::show(&app, monitor, target.zone_end(zone).unwrap_or(left), below);
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
            let hover = st.clone();
            let slide = open_menu.clone();
            let cornered = Rc::new(Cell::new(false));
            let weak = Rc::downgrade(&self.0);
            motion.connect_motion(move |_, x, y| {
                let edge = x <= 1.5 && y <= 1.5 && hover.width() as f64 >= hover.root().map(|r| r.width() as f64).unwrap_or(0.0) - 1.0;
                if edge && !cornered.replace(true) {
                    if let Some(v) = weak.upgrade() {
                        let (app, output) = { let v = v.borrow(); (v.app.clone(), v.output.clone()) };
                        if app.prefs.borrow().hot_corner && app.edges_live(&output) {
                            crate::spaces::toggle(&app, Some(output));
                        }
                    }
                } else if !edge {
                    cornered.set(false);
                }
                let zone = hover.zone_at(x, y);
                hover.hover(zone);
                if menu_zone(zone) && hover.held().is_some() && hover.held() != zone && (crate::popmenu::is_open() || crate::datepop::is_open()) {
                    if let Some(z) = zone {
                        slide(z, 1);
                    }
                }
            });
            let hover = st.clone();
            motion.connect_leave(move |_| hover.hover(None));
            st.add_controller(motion);
            let click = gtk::GestureClick::new();
            click.set_button(0);
            let pressed: Rc<Cell<Option<Zone>>> = Rc::new(Cell::new(None));
            let was = pressed.clone();
            let target = st.clone();
            click.connect_pressed(move |_, _, _, _| {
                was.set(if crate::popmenu::is_open() || crate::datepop::is_open() { target.held() } else { None });
            });
            let weak = Rc::downgrade(&self.0);
            let target = st.clone();
            click.connect_released(move |g, _, x, y| {
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
                    Some(z) if menu_zone(Some(z)) || matches!(z, Zone::Tray(_) | Zone::Date) => open_menu(z, button),
                    Some(Zone::Status) => crate::control::toggle(&app),
                    _ => {}
                }
            });
            st.add_controller(click);
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
        if let Some(corner) = self.0.borrow_mut().corner.take() {
            corner.destroy();
        }
        if let Some(c) = self.0.borrow_mut().control_corner.take() {
            c.destroy();
        }
        if let Some(c) = self.0.borrow_mut().scrim.take() {
            c.destroy();
        }
        let effect = self.0.borrow_mut().effect.take();
        if let (Some((effect, _)), Some(blur)) = (effect, app.blur.as_ref()) {
            blur.detach(effect);
        }
        window.destroy();
    }

    pub fn surface(&self) -> Option<gdk::Surface> {
        self.0.borrow().window.surface()
    }

    pub fn redraw(&self) {
        let v = self.0.borrow();
        v.canvas.queue_draw();
        if let Some(st) = &v.strip {
            st.queue_draw();
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
        let app = {
            let mut v = self.0.borrow_mut();
            if v.open == open {
                return;
            }
            v.open = open;
            v.dwell += 1;
            v.leave += 1;
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
                if let Some(scrim) = v.scrim.take() {
                    scrim.destroy();
                }
            }
            v.app.clone()
        };
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

    fn hover(&self, inside: bool) {
        let (open, peek, hover) = {
            let mut v = self.0.borrow_mut();
            v.hovered = inside;
            v.leave += 1;
            v.dwell += 1;
            (v.open, v.app.peek.borrow().is_some(), v.app.prefs.borrow().hover)
        };
        if inside {
            if hover && !open && !peek {
                self.dwell();
            }
        } else {
            let generation = self.0.borrow().leave;
            let weak = Rc::downgrade(&self.0);
            glib::timeout_add_local_once(Duration::from_millis(crate::app::LEAVE_MS), move || {
                if let Some(v) = weak.upgrade() {
                    let fold = {
                        let v = v.borrow();
                        v.leave == generation && !v.hovered && v.app.prefs.borrow().notch_close == "leave"
                    };
                    if fold {
                        Notch(v).set_open(false);
                    }
                }
            });
        }
    }

    fn dwell(&self) {
        let generation = {
            let mut v = self.0.borrow_mut();
            v.dwell += 1;
            v.dwell
        };
        let weak = Rc::downgrade(&self.0);
        glib::timeout_add_local_once(Duration::from_millis(300), move || {
            if let Some(v) = weak.upgrade() {
                let open = {
                    let v = v.borrow();
                    let now = App::now();
                    let morphing = !v.float.settled(now) || !v.widen.settled(now);
                    v.dwell == generation && v.hovered && !v.open && !morphing && v.app.peek.borrow().is_none()
                };
                if open {
                    Notch(v).set_open(true);
                }
            }
        });
    }

    fn point(&self, at: Option<(f64, f64)>) {
        let (canvas, changed, inside, was) = {
            let mut v = self.0.borrow_mut();
            v.pointer = at;
            let now = App::now();
            let inside = at.is_some_and(|(x, y)| v.over_body(x, y, now));
            let was = v.hovered;
            let hot = at.and_then(|(x, y)| v.hot_at(x, y, now));
            let changed = hot != v.hot;
            if changed {
                v.hot = hot;
                for i in 0..v.glow.len() {
                    let on = hot.map(Hot::index) == Some(i);
                    v.glow[i].go(if on { 1.0 } else { 0.0 }, now, if on { 160 } else { 280 }, motion::STANDARD);
                }
            }
            (v.canvas.clone(), changed.then_some(hot.is_some()), inside, was)
        };
        if inside != was {
            self.hover(inside);
        }
        if let Some(pointer) = changed {
            canvas.set_cursor_from_name(pointer.then_some("pointer"));
            self.kick();
        }
    }

    fn switch_tab(&self, tab: Tab) {
        let app = {
            let mut v = self.0.borrow_mut();
            if v.tab == tab {
                return;
            }
            v.tab = tab;
            v.tab_fade.set(0.0);
            v.tab_fade.go(1.0, App::now(), 240, motion::STANDARD);
            v.hot = None;
            v.app.clone()
        };
        if tab == Tab::Tray {
            crate::share::forget_senders();
            crate::share::scan(&app);
        }
        if tab == Tab::Phone {
            crate::phone::refresh(&app);
        }
        let at = self.0.borrow().pointer;
        self.point(at);
        self.kick();
    }

    fn tool(&self, i: usize) {
        let app = self.0.borrow().app.clone();
        match i {
            0 => {
                self.set_open(false);
                glib::timeout_add_local_once(Duration::from_millis(320), || {
                    glib::spawn_future_local(async {
                        let _ = gtk::gio::spawn_blocking(crate::compositor::screenshot).await;
                    });
                });
            }
            1 => {
                self.set_open(false);
                let weak = Rc::downgrade(&app);
                glib::timeout_add_local_once(Duration::from_millis(320), move || {
                    let argv: [&std::ffi::OsStr; 2] = ["digit-snap".as_ref(), "pick".as_ref()];
                    if gtk::gio::Subprocess::newv(&argv, gtk::gio::SubprocessFlags::STDOUT_SILENCE | gtk::gio::SubprocessFlags::STDERR_SILENCE).is_ok() {
                        return;
                    }
                    glib::spawn_future_local(async move {
                        let Ok(Some([r, g, b])) = gtk::gio::spawn_blocking(crate::compositor::pick_color).await else { return };
                        let hex = format!("#{:02X}{:02X}{:02X}", (r * 255.0).round() as u8, (g * 255.0).round() as u8, (b * 255.0).round() as u8);
                        let _ = crate::child::spawn(&["wl-copy".as_ref(), hex.as_ref()], gtk::gio::SubprocessFlags::NONE);
                        if let Some(app) = weak.upgrade() {
                            app.request(crate::app::Peek::Note { text: format!("{hex} copied") });
                        }
                    });
                });
            }
            2 => {
                app.set_dnd(!app.dnd.get());
                app.history_changed();
            }
            _ => {
                let on = app.awake.borrow().as_ref().is_some_and(|a| a.on());
                let surface = self.surface();
                if let Some(a) = app.awake.borrow().as_ref() {
                    a.set(surface.as_ref(), !on);
                }
                app.history_changed();
            }
        }
        self.0.borrow().canvas.queue_draw();
    }

    fn drag_enter(&self) {
        self.0.borrow_mut().dropping = true;
        if !self.is_open() {
            self.0.borrow_mut().tab = Tab::Tray;
            self.set_open(true);
        } else {
            self.switch_tab(Tab::Tray);
        }
        let app = self.0.borrow().app.clone();
        crate::share::scan(&app);
        self.kick();
    }

    fn drag_motion(&self, x: f64, y: f64) {
        let changed = {
            let mut v = self.0.borrow_mut();
            let at = v.landing_at(x, y);
            let changed = at != v.landing;
            v.landing = at;
            changed
        };
        if changed {
            self.0.borrow().canvas.queue_draw();
        }
    }

    fn drag_leave(&self) {
        {
            let mut v = self.0.borrow_mut();
            v.dropping = false;
            v.landing = None;
            v.canvas.queue_draw();
        }
        self.kick();
        let weak = Rc::downgrade(&self.0);
        glib::timeout_add_local_once(Duration::from_millis(500), move || {
            if let Some(v) = weak.upgrade() {
                let idle = {
                    let v = v.borrow();
                    !v.hovered && !v.dropping
                };
                if idle {
                    Notch(v).set_open(false);
                }
            }
        });
    }

    fn dropped(&self, paths: Vec<std::path::PathBuf>, x: f64, y: f64) -> bool {
        let (app, landing) = {
            let mut v = self.0.borrow_mut();
            v.dropping = false;
            let at = v.landing_at(x, y).unwrap_or(Landing::Files);
            v.landing = None;
            (v.app.clone(), at)
        };
        if paths.is_empty() {
            return false;
        }
        let devices = crate::share::devices();
        let senders = crate::share::senders();
        match landing {
            Landing::Device(i) if i < devices.len() => crate::share::send(&app, &devices[i], paths),
            Landing::Sender(i) if i < senders.len() => crate::share::send_with(&app, &senders[i], paths),
            Landing::Share if devices.len() == 1 => crate::share::send(&app, &devices[0], paths),
            _ => crate::tray::add(paths),
        }
        self.0.borrow().canvas.queue_draw();
        true
    }

    fn tile_at(&self, x: f64, y: f64) -> Option<usize> {
        let v = self.0.borrow();
        if !v.open || v.tab != Tab::Tray {
            return None;
        }
        (0..crate::tray::len().min(TILES)).find(|&i| v.tile_rect(i).contains(x, y) && !v.remove_rect(i).contains(x, y))
    }

    fn set_level(&self, level: f64) {
        let app = self.0.borrow().app.clone();
        let muted = app.volume.get().is_some_and(|v| v.1);
        let level = level.clamp(0.0, 1.0);
        app.volume.set(Some((level, muted)));
        audio::ask(Ask::Level(level));
        app.history_changed();
        self.0.borrow().canvas.queue_draw();
    }

    fn tune(&self, on: bool) {
        {
            let mut v = self.0.borrow_mut();
            if v.tuning == on {
                return;
            }
            v.tuning = on;
            let now = App::now();
            if on {
                v.tune.go(1.0, now, 380, motion::DEFAULT_SPATIAL);
            } else {
                v.tune.go(0.0, now, 200, motion::FAST_EFFECTS);
            }
        }
        let at = self.0.borrow().pointer;
        self.0.borrow_mut().hot = None;
        self.point(at);
        self.kick();
    }

    fn wheel(&self, dy: f64) -> bool {
        let inside = {
            let v = self.0.borrow();
            let now = App::now();
            v.open && v.pointer.is_some_and(|(x, y)| v.volume_zone(now).is_some_and(|z| z.contains(x, y)))
        };
        if !inside {
            return false;
        }
        let Some((level, _)) = self.0.borrow().app.volume.get() else { return false };
        self.tune(true);
        self.set_level(level - dy * 0.02);
        true
    }

    fn begin_drag(&self, x: f64, y: f64) -> bool {
        let level = {
            let mut v = self.0.borrow_mut();
            let now = App::now();
            match v.tuner(now) {
                Some(t) if v.tuning && x >= t.track.x - 8.0 && x <= t.track.x + t.track.w + 8.0 && (y - (t.track.y + t.track.h / 2.0)).abs() <= 12.0 => {
                    v.sliding = true;
                    Some(((x - t.track.x) / t.track.w).clamp(0.0, 1.0))
                }
                _ => None,
            }
        };
        match level {
            Some(l) => {
                self.set_level(l);
                true
            }
            None => self.begin_scrub(x, y),
        }
    }

    fn move_drag(&self, x: f64) {
        let level = {
            let v = self.0.borrow();
            if !v.sliding {
                None
            } else {
                v.tuner(App::now()).map(|t| ((x - t.track.x) / t.track.w).clamp(0.0, 1.0))
            }
        };
        match level {
            Some(l) => self.set_level(l),
            None => self.move_scrub(x),
        }
    }

    fn end_drag(&self, x: f64) {
        let sliding = std::mem::replace(&mut self.0.borrow_mut().sliding, false);
        if sliding {
            self.move_drag_end(x);
        } else {
            self.end_scrub(x);
        }
    }

    fn move_drag_end(&self, x: f64) {
        let level = self.0.borrow().tuner(App::now()).map(|t| ((x - t.track.x) / t.track.w).clamp(0.0, 1.0));
        if let Some(l) = level {
            self.set_level(l);
        }
        self.0.borrow().canvas.queue_draw();
    }

    fn seek_fraction(v: &View, x: f64) -> Option<f64> {
        let p = v.player(App::now())?;
        Some(((x - p.seek.x) / p.seek.w).clamp(0.0, 1.0))
    }

    fn begin_scrub(&self, x: f64, y: f64) -> bool {
        let mut v = self.0.borrow_mut();
        let seekable = {
            let m = v.app.media.borrow();
            m.timing == Timing::Timed && m.can_seek
        };
        let Some(p) = v.player(App::now()) else { return false };
        if !v.open || !seekable || !p.seek.contains(x, y) {
            return false;
        }
        v.scrub = Self::seek_fraction(&v, x);
        v.canvas.queue_draw();
        true
    }

    fn move_scrub(&self, x: f64) {
        let mut v = self.0.borrow_mut();
        if v.scrub.is_some() {
            v.scrub = Self::seek_fraction(&v, x);
            v.canvas.queue_draw();
        }
    }

    fn end_scrub(&self, x: f64) {
        let (app, fraction) = {
            let mut v = self.0.borrow_mut();
            if v.scrub.take().is_none() {
                return;
            }
            (v.app.clone(), Self::seek_fraction(&v, x))
        };
        if let (Some(f), Some(p)) = (fraction, app.players.borrow().as_ref()) {
            p.seek_to(f);
        }
        self.0.borrow().canvas.queue_draw();
    }

    fn click(&self, x: f64, y: f64) {
        let outside = {
            let v = self.0.borrow();
            !v.open && !v.over_body(x, y, App::now())
        };
        if outside {
            return;
        }
        let (zone, tuner, tuning) = {
            let v = self.0.borrow();
            let now = App::now();
            (if v.open { v.volume_zone(now) } else { None }, v.tuner(now), v.tuning)
        };
        if let Some(zone) = zone.filter(|z| z.contains(x, y)) {
            if !tuning {
                self.tune(true);
                return;
            }
            if tuner.is_some_and(|t| t.icon.contains(x, y)) {
                let app = self.0.borrow().app.clone();
                if let Some((level, muted)) = app.volume.get() {
                    app.volume.set(Some((level, !muted)));
                    audio::ask(Ask::Mute(!muted));
                    app.history_changed();
                    self.0.borrow().canvas.queue_draw();
                }
            }
            let _ = zone;
            return;
        }
        if tuning && y <= self.0.borrow().header() {
            self.tune(false);
            return;
        }
        let hot = {
            let v = self.0.borrow();
            v.hot_at(x, y, App::now())
        };
        let app = self.0.borrow().app.clone();
        match hot {
            Some(Hot::CallMute) => {
                audio::ask(Ask::MicFlip);
                return;
            }
            Some(Hot::CallSwitch) => {
                {
                    let mut v = self.0.borrow_mut();
                    v.media_over_call.set(!v.media_over_call.get());
                    v.flip.set(0.0);
                    v.flip.go(1.0, App::now(), 240, motion::STANDARD);
                }
                let at = self.0.borrow().pointer;
                self.point(at);
                self.kick();
                return;
            }
            Some(Hot::CallOpen) => {
                let who = app.call.borrow().as_ref().map(|c| c.who.to_lowercase());
                self.set_open(false);
                if let Some(who) = who {
                    let apps = app.apps.clone();
                    let target = crate::compositor::windows().into_iter().find(|w| apps.display_name(&w.app_id).to_lowercase() == who || w.app_id.to_lowercase().contains(&who)).map(|w| w.id);
                    if let Some(id) = target {
                        glib::spawn_future_local(async move {
                            let _ = gtk::gio::spawn_blocking(move || crate::compositor::focus(id)).await;
                        });
                    }
                }
                return;
            }
            Some(Hot::Gear) => {
                self.set_open(false);
                crate::config::settings("notch", &app.cfg);
                return;
            }
            Some(Hot::Tab(i)) => {
                self.switch_tab(TABS[i].0);
                return;
            }
            Some(Hot::PhoneAction(i)) => {
                crate::phone::act(&app, crate::phone::ACTIONS[i].0);
                return;
            }
            Some(Hot::PhoneSwitch) => {
                crate::phone::switch(&app);
                return;
            }
            Some(Hot::PhoneDismiss(i)) => {
                if let Some(n) = crate::phone::phone().and_then(|p| p.notes.get(i).cloned()) {
                    crate::phone::dismiss(&n);
                }
                return;
            }
            Some(Hot::Tool(i)) => {
                self.tool(i);
                return;
            }
            Some(Hot::Day(i)) => {
                self.0.borrow_mut().day = i as i32 - 3;
                self.0.borrow().canvas.queue_draw();
                return;
            }
            Some(Hot::Remove(i)) => {
                crate::tray::remove(i);
                let at = self.0.borrow().pointer;
                self.0.borrow_mut().hot = None;
                self.point(at);
                self.0.borrow().canvas.queue_draw();
                return;
            }
            Some(Hot::Tile(i)) => {
                if let Some(path) = crate::tray::path(i) {
                    let uri = gtk::gio::File::for_path(path).uri();
                    let context = gtk::gio::AppLaunchContext::new();
                    crate::apps::prepare(&context);
                    let _ = gtk::gio::AppInfo::launch_default_for_uri(&uri, Some(&context));
                    self.set_open(false);
                }
                return;
            }
            Some(Hot::Device(i)) => {
                let files: Vec<std::path::PathBuf> = (0..crate::tray::len()).filter_map(crate::tray::path).collect();
                if let Some(d) = crate::share::devices().get(i) {
                    crate::share::send(&app, d, files);
                }
                return;
            }
            Some(Hot::Sender(i)) => {
                let files: Vec<std::path::PathBuf> = (0..crate::tray::len()).filter_map(crate::tray::path).collect();
                if let Some(sender) = crate::share::senders().get(i) {
                    crate::share::send_with(&app, sender, files);
                }
                return;
            }
            _ => {}
        }
        let target = {
            let v = self.0.borrow();
            v.space_at(x, y)
        };
        if let Some(id) = target {
            glib::spawn_future_local(async move {
                let _ = gtk::gio::spawn_blocking(move || crate::compositor::focus_space(id)).await;
            });
            return;
        }
        let (app, open, action) = {
            let v = self.0.borrow();
            let action = if v.open && v.tab == Tab::Nook {
                v.player(App::now()).and_then(|p| p.controls.into_iter().find(|(r, _)| r.contains(x, y)).map(|(_, a)| a))
            } else {
                None
            };
            (v.app.clone(), v.open, action)
        };
        if let Some(action) = action {
            let cmd = match action {
                Action::Previous => media::Command::Previous,
                Action::Toggle => media::Command::PlayPause,
                Action::Next => media::Command::Next,
            };
            if let Some(p) = app.players.borrow().as_ref() {
                p.command(cmd);
            }
            return;
        }
        if open {
            if y <= app.cfg.header() {
                self.set_open(false);
            }
            return;
        }
        let peeking = app.peek.borrow().is_some();
        let action = {
            let v = self.0.borrow();
            match (&v.head, v.hot_at(x, y, App::now())) {
                (Head::Peek(Peek::Notification { actions, .. }), Some(Hot::Action(i))) => actions.get(i).map(|a| a.0.clone()),
                _ => None,
            }
        };
        if let Some(key) = action {
            app.dismiss(Ending::Action(key));
        } else if peeking {
            app.dismiss(Ending::Clicked);
        } else {
            self.set_open(true);
        }
    }

    fn kick(&self) {
        let (canvas, already) = {
            let mut v = self.0.borrow_mut();
            let already = v.ticking;
            v.ticking = true;
            (v.canvas.clone(), already)
        };
        if already {
            return;
        }
        let weak: Weak<RefCell<View>> = Rc::downgrade(&self.0);
        canvas.add_tick_callback(move |w, clock| {
            let Some(view) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let Ok(mut v) = view.try_borrow_mut() else {
                return glib::ControlFlow::Continue;
            };
            let busy = v.step(clock.frame_time());
            w.queue_draw();
            if busy {
                glib::ControlFlow::Continue
            } else {
                v.ticking = false;
                glib::ControlFlow::Break
            }
        });
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
        CENTRE + (self.clock_width() - self.text_width("00:00", 16.0, Face::Number(500))).max(0.0)
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
        let (x0, x1) = (CX - half + 18.0, CX + half - 18.0);
        let gap = 10.0;
        let n = actions.len() as f64;
        let bw = ((x1 - x0) - gap * (n - 1.0)) / n;
        let y = top + peek.height() - 46.0 + 2.0;
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
                (0..phone.notes.len().min(NOTES)).find(|&i| phone.notes[i].dismissable && self.note_rect(i).contains(x, y)).map(Hot::PhoneDismiss)
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

    fn day_rects(&self) -> Vec<(Rect, crate::calendar::Day)> {
        let x0 = CAL_X + 64.0;
        let col = (CAL_W - 64.0) / 7.0;
        let top = self.body_top();
        crate::calendar::week(0).into_iter().enumerate().map(|(i, d)| (Rect { x: x0 + i as f64 * col, y: top, w: col, h: 46.0 }, d)).collect()
    }

    fn tool_rect(&self, i: usize) -> Rect {
        let x0 = TOOLS_X + ((RIGHT - TOOLS_X) - 96.0) / 2.0;
        let y0 = self.body_top() + 4.0;
        Rect { x: x0 + (i % 2) as f64 * 52.0, y: y0 + (i / 2) as f64 * 52.0, w: 44.0, h: 44.0 }
    }

    fn phone_rect(&self) -> Rect {
        Rect { x: LEFT, y: self.body_top(), w: PHONE_W, h: 120.0 }
    }

    fn phone_switch_rect(&self) -> Rect {
        let p = self.phone_rect();
        Rect { x: p.x + p.w - 42.0, y: p.y + 14.0, w: 30.0, h: 30.0 }
    }

    fn phone_action_rect(&self, i: usize) -> Rect {
        let p = self.phone_rect();
        Rect { x: p.x + 16.0 + i as f64 * 42.0, y: p.y + p.h - 46.0, w: 36.0, h: 36.0 }
    }

    fn note_rect(&self, i: usize) -> Rect {
        let x = LEFT + PHONE_W + 14.0;
        Rect { x, y: self.body_top() + 4.0 + i as f64 * 38.0, w: RIGHT - x, h: 36.0 }
    }

    fn files_rect(&self) -> Rect {
        Rect { x: LEFT, y: self.body_top(), w: FILES_W, h: 120.0 }
    }

    fn share_rect(&self) -> Rect {
        Rect { x: LEFT + FILES_W + 14.0, y: self.body_top(), w: RIGHT - LEFT - FILES_W - 14.0, h: 120.0 }
    }

    fn tile_rect(&self, i: usize) -> Rect {
        let f = self.files_rect();
        Rect { x: f.x + 14.0 + i as f64 * TILE, y: f.y + 12.0, w: TILE - 6.0, h: 96.0 }
    }

    fn remove_rect(&self, i: usize) -> Rect {
        let t = self.tile_rect(i);
        Rect { x: t.x + t.w - 22.0, y: t.y + 2.0, w: 20.0, h: 20.0 }
    }

    fn device_rect(&self, i: usize) -> Rect {
        let z = self.share_rect();
        Rect { x: z.x + 8.0, y: z.y + 38.0 + i as f64 * 26.0, w: z.w - 16.0, h: 24.0 }
    }

    fn chip_rect(&self, i: usize) -> Rect {
        let z = self.share_rect();
        Rect { x: z.x + 12.0 + i as f64 * (CHIP + 6.0), y: z.y + z.h - CHIP - 8.0, w: CHIP, h: CHIP }
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

    fn spaces(&self) -> (Vec<crate::compositor::Space>, usize) {
        let all = self.app.spaces.borrow();
        let list: Vec<_> = all.iter().filter(|s| s.output == self.output && (s.filled || s.active)).cloned().collect();
        let active = list.iter().position(|s| s.active).unwrap_or(0);
        (list, active)
    }

    fn spaces_start(&self, now: i64) -> Option<(f64, f64)> {
        let n = self.spaces().0.len();
        if n == 0 {
            return None;
        }
        let total = dots_width(n);
        match &self.head {
            Head::Open => None,
            Head::Idle(_) => {
                let (l0, l1, _, _) = self.wings(&self.head, now);
                Some((((l0 + l1) / 2.0 - total / 2.0).round(), total))
            }
            Head::Peek(Peek::Spaces { .. }) => {
                let (l0, l1, _, _) = self.wings(&self.head, now);
                Some((((l0 + l1) / 2.0 - total / 2.0).round(), total))
            }
            Head::Peek(_) | Head::Call => None,
        }
    }

    fn space_at(&self, x: f64, y: f64) -> Option<u64> {
        let (start, total) = self.spaces_start(App::now())?;
        if y > self.header() || x < start - 6.0 || x > start + total + 6.0 {
            return None;
        }
        let (list, active) = self.spaces();
        let mut left = start;
        for (i, sp) in list.iter().enumerate() {
            let w = if i == active { PILL } else { DOT };
            if x <= left + w + GAP / 2.0 || i + 1 == list.len() {
                return Some(sp.id);
            }
            left += w + GAP;
        }
        None
    }

    fn draw_spaces(&self, s: &gtk::Snapshot, now: i64, mid: f64) {
        let Some((start, _)) = self.spaces_start(now) else { return };
        let pal = self.pal.get();
        let (list, _) = self.spaces();
        let at = self.spot.value(now);
        let mut x = start;
        for (i, sp) in list.iter().enumerate() {
            let near = (1.0 - (i as f64 - at).abs()).clamp(0.0, 1.0);
            let w = DOT + (PILL - DOT) * near;
            let base = if sp.filled { pal.ink_meta } else { pal.ink_faint.alpha(0.55) };
            let r = graphene::Rect::new(x as f32, (mid - DOT / 2.0) as f32, w as f32, DOT as f32);
            s.push_rounded_clip(&gsk::RoundedRect::from_rect(r, (DOT / 2.0) as f32));
            s.append_color(&pal.ink_main.mix(base, near).rgba(), &r);
            s.pop();
            x += w + GAP;
        }
    }

    fn room(&self) -> f64 {
        let n = self.app.lights.borrow().count();
        if n == 0 { 0.0 } else { 4.0 + 12.0 * n as f64 }
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
                let left = dots_width(self.spaces().0.len());
                let right = if *media { 28.0 + crate::bars::W as f64 } else { 0.0 };
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
            // During a call the switch peek shows only the dots and the call's waveform, so it is
            // sized like the idle notch rather than with room for "Desktop N".
            Head::Peek(Peek::Spaces { .. }) if self.app.call.borrow().is_some() => {
                let left = dots_width(self.spaces().0.len());
                let right = 28.0 + crate::bars::W as f64;
                (self.centre() + room + 2.0 * (left.max(right) - 8.0).max(0.0), 0.0, self.idle_r())
            }
            Head::Peek(p) => (self.centre() + room + 2.0 * p.wing(), 0.0, self.idle_r()),
            Head::Call => {
                let left = 30.0 + self.text_width(&self.call_time(App::now()), 15.0, Face::Number(560)).max(self.text_width("0:00", 15.0, Face::Number(560))).ceil();
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
        {
            let (list, active) = self.spaces();
            if list.len() != self.spots {
                self.spots = list.len();
                self.spot.set(active as f64);
            } else {
                self.spot.go(active as f64, now, 380, motion::EMPHASIZED_DECEL);
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
        let tweens = [&self.w, &self.depth, &self.r, &self.fade, &self.sheet, &self.back, &self.meter, &self.art_scale, &self.fill, &self.tune, &self.hush, &self.spot, &self.tab_fade, &self.flip, &self.float, &self.widen, &self.menu, &self.glass];
        let moving = tweens.into_iter().chain(self.glow.iter()).chain(self.dots.iter()).any(|t| !t.settled(now));
        let rolling = !motion::reduced() && self.roll.iter().flatten().any(|t| now < t + ROLL);
        moving || rolling || self.app.palette_moving(now)
    }

    fn call_time(&self, now: i64) -> String {
        let since = self.app.call.borrow().as_ref().map_or(now, |c| c.since);
        let t = ((now - since).max(0) / 1_000_000) as u64;
        if t >= 3600 { format!("{}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60) } else { format!("{}:{:02}", t / 60, t % 60) }
    }

    fn place_bars(&self, now: i64) {
        // During a call the workspace switch peek keeps the call's waveform on the right.
        let call = matches!(self.head, Head::Call) || (matches!(self.head, Head::Peek(Peek::Spaces { .. })) && self.app.call.borrow().is_some());
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
        Self::lift_at(self.float.value(now), self.menu.value(now) * (1.0 - self.widen.value(now)))
    }

    /// Where the content sits below the shape's top: the float lift, plus the island's top
    /// padding while it is open.
    fn content_lift(&self, now: i64) -> f64 {
        let open = (self.depth.value(now).max(0.0) / OPEN_DEPTH).clamp(0.0, 1.0);
        self.body_lift(now) + ISLAND_PAD * self.float.value(now).clamp(0.0, 1.0) * open
    }

    fn body_lift(&self, now: i64) -> f64 {
        let open = (self.depth.value(now).max(0.0) / 24.0).clamp(0.0, 1.0);
        Self::lift_at(self.float.value(now), self.menu.value(now) * (1.0 - self.widen.value(now)) * (1.0 - open))
    }

    fn lift_at(float: f64, bar: f64) -> f64 {
        let bar = bar.clamp(0.0, 1.0);
        (crate::app::LIFT + (crate::app::MENU_LIFT - crate::app::LIFT) * bar) * float.clamp(0.0, 1.0)
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
        let lift = Self::lift_at(self.float.value(now).max(self.float.target()), self.menu.value(now).max(self.menu.target()) * (1.0 - self.widen.target()));
        let band = self.band(now).max(self.header() + 2.0 * lift * self.menu.target() * (1.0 - self.widen.target())).ceil() as i32;
        let body = crate::app::LIFT * self.float.value(now).max(self.float.target()).clamp(0.0, 1.0);
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
        if !self.app.menubar() || self.app.wide() {
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
            if let (Some((effect, _)), Some(blur)) = (self.effect.take(), self.app.blur.as_ref()) {
                blur.detach(effect);
                self.frost.clear();
            }
            return;
        }
        let band = self.glass.value(now) > 0.02 || self.glass.target() > 0.5;
        if self.app.dock() != Dock::Screen || (!self.frosted() && !band && self.frost.is_empty()) {
            return;
        }
        let Some(blur) = self.app.blur.as_ref() else { return };
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

    fn draw_clock(&self, s: &gtk::Snapshot, now: i64) {
        let pal = self.pal.get();
        let hush = self.hush.value(now).clamp(0.0, 1.0);
        if hush < 0.01 {
            return;
        }
        let color = pal.ink_main.mix(pal.ink_meta, 1.0 - self.back.value(now).clamp(0.0, 1.0)).alpha(hush);
        let mid = self.header() / 2.0;
        let face = Face::Number(500);
        let cells: Vec<(pango::Layout, f64)> = self
            .shown
            .iter()
            .map(|c| {
                let l = self.layout(&c.to_string(), 16.0, face);
                let w = l.pixel_extents().1.width() as f64;
                (l, w)
            })
            .collect();
        let total: f64 = cells.iter().map(|c| c.1).sum();
        let mut x = (CX - total / 2.0).round();
        let line = cells.first().map(|c| c.0.pixel_extents().1.height() as f64).unwrap_or(20.0);
        let y = (mid - line / 2.0).round();
        for (i, (l, w)) in cells.iter().enumerate() {
            let e = match self.roll.get(i).copied().flatten() {
                Some(t) if !motion::reduced() && now < t + ROLL => motion::IN_OUT_CUBIC.at(((now - t).max(0)) as f64 / ROLL as f64),
                _ => 1.0,
            };
            if e < 1.0 {
                s.push_clip(&graphene::Rect::new(x as f32, y as f32, *w as f32, line as f32));
                if let Some(old) = self.was.get(i) {
                    let ol = self.layout(&old.to_string(), 16.0, face);
                    s.save();
                    s.translate(&graphene::Point::new(x as f32, (y - line * e) as f32));
                    s.append_layout(&ol, &color.rgba());
                    s.restore();
                }
                s.save();
                s.translate(&graphene::Point::new(x as f32, (y + line * (1.0 - e)) as f32));
                s.append_layout(l, &color.rgba());
                s.restore();
                s.pop();
            } else {
                s.save();
                s.translate(&graphene::Point::new(x as f32, y as f32));
                s.append_layout(l, &color.rgba());
                s.restore();
            }
            x += w;
        }
    }

    fn wings(&self, head: &Head, now: i64) -> (f64, f64, f64, f64) {
        let (w, _, _) = self.geometry(head, now);
        let half = self.clock_width() / 2.0;
        (CX - w / 2.0 + 12.0, CX - half - 10.0, CX + half + 10.0, CX + w / 2.0 - 12.0 - self.room())
    }

    fn draw_lights(&self, s: &gtk::Snapshot, now: i64) {
        let (screen, capture) = (self.dots[0].value(now).clamp(0.0, 1.0), self.dots[1].value(now).clamp(0.0, 1.0));
        if screen < 0.01 && capture < 0.01 {
            return;
        }
        let edge = CX + self.w.value(now) / 2.0 - 14.0;
        let mid = self.header() / 2.0;
        let mut x = edge;
        for (alpha, color) in [(capture, self.capture.get()), (screen, RED)] {
            if alpha < 0.01 {
                continue;
            }
            for (r, a) in [(6.0 * (0.6 + 0.4 * alpha), 0.22 * alpha), (3.5 * (0.5 + 0.5 * alpha), alpha)] {
                let dot = graphene::Rect::new((x - r) as f32, (mid - r) as f32, (2.0 * r) as f32, (2.0 * r) as f32);
                s.push_rounded_clip(&gsk::RoundedRect::from_rect(dot, r as f32));
                s.append_color(&color.alpha(a).rgba(), &dot);
                s.pop();
            }
            x -= 12.0 * alpha;
        }
    }

    fn vol_right(&self, now: i64) -> f64 {
        self.wings(&Head::Open, now).3 - 38.0
    }

    fn volume_zone(&self, now: i64) -> Option<Rect> {
        let (level, muted) = self.app.volume.get()?;
        let (_, _, r0, _) = self.wings(&Head::Open, now);
        let r1 = self.vol_right(now);
        let h = self.header();
        if self.tuning {
            return Some(Rect { x: r0 - 4.0, y: 0.0, w: r1 - r0 + 10.0, h });
        }
        let text = if muted { String::new() } else { format!("{}%", (level * 100.0).round()) };
        let tw = self.text_width(&text, 15.0, Face::Number(450));
        let x = r1 - tw - 30.0;
        Some(Rect { x, y: 4.0, w: r1 + 8.0 - x, h: h - 8.0 })
    }

    fn tuner(&self, now: i64) -> Option<Tuner> {
        if !self.open {
            return None;
        }
        let (_, _, r0, _) = self.wings(&Head::Open, now);
        let r1 = self.vol_right(now);
        let mid = self.header() / 2.0;
        let label = self.text_width("100%", 15.0, Face::Number(560)).ceil() + 2.0;
        let x = r0 + 28.0;
        Some(Tuner {
            icon: Rect { x: r0 - 2.0, y: mid - 13.0, w: 26.0, h: 26.0 },
            track: Rect { x, y: mid - 2.0, w: (r1 - label - 10.0 - x).max(20.0), h: 4.0 },
            label,
        })
    }

    fn draw_volume(&self, s: &gtk::Snapshot, now: i64) {
        let Some((level, muted)) = self.app.volume.get() else { return };
        let pal = self.pal.get();
        let mid = self.header() / 2.0;
        let r1 = self.vol_right(now);
        let t = self.tune.value(now).clamp(0.0, 1.0);
        let glow = self.glow[4].value(now).clamp(0.0, 1.0);
        let icon = if muted { "volume_off" } else if level < 0.5 { "volume_down" } else { "volume_up" };
        if t < 0.99 {
            s.push_opacity(1.0 - t);
            if let Some(z) = self.volume_zone(now).filter(|_| !self.tuning) {
                if glow > 0.01 {
                    let pill = graphene::Rect::new(z.x as f32, (mid - 13.0) as f32, z.w as f32, 26.0);
                    s.push_rounded_clip(&gsk::RoundedRect::from_rect(pill, 13.0));
                    s.append_color(&pal.ink_main.alpha(0.09 * glow).rgba(), &pill);
                    s.pop();
                }
            }
            let text = if muted { String::new() } else { format!("{}%", (level * 100.0).round()) };
            let tw = self.text_width(&text, 15.0, Face::Number(450));
            let ink = pal.ink_meta.mix(pal.ink_main, 1.0 - glow);
            self.text(s, &text, 15.0, Face::Number(450), ink, r1 - tw - 2.0, mid, tw + 2.0, pango::Alignment::Right);
            self.icon(s, icon, 18.0, false, ink, r1 - tw - 14.0, mid);
            s.pop();
        }
        let Some(tuner) = self.tuner(now) else { return };
        if t > 0.01 {
            s.push_opacity(t);
            let ink = if muted { pal.ink_meta } else { pal.ink_main };
            self.icon(s, icon, 18.0, false, ink, tuner.icon.x + tuner.icon.w / 2.0, mid);
            let end = tuner.track.x + tuner.track.w;
            let w = tuner.track.w * (0.3 + 0.7 * t);
            let track = Rect { x: end - w, w, ..tuner.track };
            let value = level.clamp(0.0, 1.0);
            self.bar(s, &track, value, pal.ink_main.alpha(0.15), ink);
            let r = if self.sliding { 6.5 } else { 5.0 + glow };
            let knob = graphene::Rect::new((track.x + track.w * value - r) as f32, (mid - r) as f32, (2.0 * r) as f32, (2.0 * r) as f32);
            s.push_rounded_clip(&gsk::RoundedRect::from_rect(knob, r as f32));
            s.append_color(&ink.rgba(), &knob);
            s.pop();
            let percent = format!("{}%", (value * 100.0).round());
            self.text(s, &percent, 15.0, Face::Number(560), ink, r1 - tuner.label, mid, tuner.label, pango::Alignment::Right);
            s.pop();
        }
    }

    fn draw_head(&self, s: &gtk::Snapshot, now: i64) {
        let pal = self.pal.get();
        let mid = self.header() / 2.0;
        let (l0, l1, r0, r1) = self.wings(&self.head, now);
        let (left, right) = (l1 - l0, r1 - r0);
        let start = pango::Alignment::Left;
        let end = pango::Alignment::Right;
        match &self.head {
            Head::Idle(media) => {
                self.draw_spaces(s, now, mid);
                if *media {
                    self.draw_art(s, &Rect { x: r1 - crate::bars::W as f64 - 28.0, y: mid - 10.0, w: 20.0, h: 20.0 }, 5.0, 1.0);
                }
            }
            Head::Call => {
                let disc = graphene::Rect::new(l0 as f32, (mid - 11.0) as f32, 22.0, 22.0);
                s.push_rounded_clip(&gsk::RoundedRect::from_rect(disc, 11.0));
                s.append_color(&GREEN.alpha(0.2).rgba(), &disc);
                s.pop();
                self.icon(s, "call", 14.0, true, GREEN, l0 + 11.0, mid);
                self.text(s, &self.call_time(now), 15.0, Face::Number(560), GREEN, l0 + 30.0, mid, left - 30.0, start);
            }
            Head::Open => {
                let _ = (l0, left, r0, right);
                for (i, (r, icon, label)) in self.tab_rects().iter().enumerate() {
                    let on = TABS[i].0 == self.tab;
                    let glow = self.glow[Hot::Tab(i).index()].value(now).clamp(0.0, 1.0);
                    let fill = if on { 0.14 } else { 0.0 } + 0.07 * glow;
                    if fill > 0.005 {
                        s.push_rounded_clip(&gsk::RoundedRect::from_rect(r.graphene(), 13.0));
                        s.append_color(&pal.ink_main.alpha(fill).rgba(), &r.graphene());
                        s.pop();
                    }
                    let ink = if on { pal.ink_main } else { pal.ink_meta.mix(pal.ink_main, 1.0 - glow) };
                    self.icon(s, icon, 17.0, on, ink, r.x + 21.0, mid);
                    self.text(s, label, 14.0, POINT, ink, r.x + 34.0, mid, r.w - 38.0, start);
                }
                let g = self.gear_rect(now);
                let glow = self.glow[7].value(now).clamp(0.0, 1.0);
                if glow > 0.01 {
                    s.push_rounded_clip(&gsk::RoundedRect::from_rect(g.graphene(), 13.0));
                    s.append_color(&pal.ink_main.alpha(0.1 * glow).rgba(), &g.graphene());
                    s.pop();
                }
                self.icon(s, "settings", 18.0, true, pal.ink_meta.mix(pal.ink_main, 1.0 - glow), g.x + g.w / 2.0, mid);
                self.draw_volume(s, now);
            }
            Head::Peek(Peek::Volume { level, muted }) => {
                let icon = if *muted { "volume_off" } else if *level < 0.5 { "volume_down" } else { "volume_up" };
                self.icon(s, icon, 18.0, false, pal.ink_main, l0 + 9.0, mid);
                self.text(s, if *muted { "Muted" } else { "Volume" }, 15.0, REST, pal.ink_meta, l0 + 24.0, mid, left - 24.0, start);
                let value = self.meter.value(now).clamp(0.0, 1.0);
                let percent = format!("{}%", (value * 100.0).round());
                // Size the label for the widest value so "100%" never ellipsizes and the meter
                // does not change length while the number animates.
                let label = self.text_width("100%", 15.0, Face::Number(560)).ceil() + 2.0;
                self.text(s, &percent, 15.0, Face::Number(560), pal.ink_main, r1 - label, mid, label, end);
                let track = Rect { x: r0, y: mid - 1.5, w: (right - label - 8.0).max(0.0), h: 3.0 };
                self.bar(s, &track, value, pal.ink_main.alpha(0.15), pal.ink_main);
            }
            Head::Peek(Peek::Track { title, artist }) => {
                self.draw_art(s, &Rect { x: l0, y: mid - 10.0, w: 20.0, h: 20.0 }, 5.0, 1.0);
                self.text(s, title, 15.0, POINT, pal.ink_main, l0 + 28.0, mid, left - 28.0, start);
                self.text(s, artist, 15.0, REST, pal.ink_meta, r0, mid, right, end);
            }
            Head::Peek(peek @ Peek::Notification { summary, body, app, icon, avatar, actions, .. }) => {
                let (top, height) = match self.app.dock() {
                    Dock::Screen => (0.0, peek.height()),
                    Dock::Bar => (self.header(), peek.height()),
                };
                let content = if actions.is_empty() { height } else { height - 46.0 };
                self.draw_note(s, top + content / 2.0, summary, body, app, icon.as_ref(), avatar.as_ref());
                for (i, (r, label)) in self.note_buttons(peek).into_iter().enumerate() {
                    let last = i + 1 == actions.len();
                    let glow = if self.hot == Some(Hot::Action(i)) { 1.0 } else { 0.0 };
                    let fill = if last { ACTION_BLUE.alpha(0.9 + 0.1 * glow) } else { pal.ink_main.alpha(0.14 + 0.08 * glow) };
                    s.push_rounded_clip(&gsk::RoundedRect::from_rect(r.graphene(), (r.h / 2.0) as f32));
                    s.append_color(&fill.rgba(), &r.graphene());
                    s.pop();
                    self.text(s, &label, 14.0, POINT, Rgb::new(1.0, 1.0, 1.0), r.x, r.y + r.h / 2.0, r.w, pango::Alignment::Center);
                }
            }
            Head::Peek(Peek::Spaces { idx }) => {
                self.draw_spaces(s, now, mid);
                if self.app.call.borrow().is_none() {
                    self.text(s, &format!("Desktop {idx}"), 14.0, POINT, pal.ink_main, r0 - 40.0, mid, right + 40.0, end);
                }
            }
            Head::Peek(Peek::Osd { icon, text, level }) => {
                self.icon(s, icon, 18.0, false, pal.ink_main, l0 + 9.0, mid);
                match level {
                    Some(_) => {
                        self.text(s, text, 15.0, REST, pal.ink_meta, l0 + 24.0, mid, left - 24.0, start);
                        let value = self.meter.value(now).clamp(0.0, 1.0);
                        let percent = format!("{}%", (value * 100.0).round());
                        let label = self.text_width("100%", 15.0, Face::Number(560)).ceil() + 2.0;
                        self.text(s, &percent, 15.0, Face::Number(560), pal.ink_main, r1 - label, mid, label, end);
                        let track = Rect { x: r0, y: mid - 1.5, w: (right - label - 8.0).max(0.0), h: 3.0 };
                        self.bar(s, &track, value, pal.ink_main.alpha(0.15), pal.ink_main);
                    }
                    None => self.text(s, text, 15.0, POINT, pal.ink_main, r0 - 60.0, mid, right + 60.0, end),
                }
            }
            Head::Peek(Peek::Mode { icon, label, on }) => {
                let (glyph, tint) = match *label {
                    "Do not disturb" => ("bedtime", PURPLE),
                    "Night Light" => ("nightlight", ORANGE),
                    "Keep awake" => ("coffee", ORANGE),
                    _ => (*icon, pal.accent),
                };
                let tint = if *on { tint } else { pal.ink_meta };
                self.icon(s, glyph, 20.0, true, tint, l0 + 11.0, mid);
                self.text(s, if *on { "On" } else { "Off" }, 15.0, Face::Text(620), tint, r0, mid, right, end);
            }
            Head::Peek(Peek::Battery { level, device }) => {
                match device {
                    Some((name, glyph)) => {
                        self.icon(s, glyph, 20.0, true, pal.ink_main, l0 + 11.0, mid);
                        self.text(s, name, 14.0, REST, pal.ink_meta, l0 + 28.0, mid, left - 28.0, start);
                    }
                    None => self.text(s, "Low Battery", 15.0, Face::Text(620), pal.ink_main, l0, mid, left, start),
                }
                let bw = 30.0;
                let percent = format!("{level}%");
                let pw = self.text_width(&percent, 15.0, Face::Number(600)).ceil();
                let bx = r1 - bw - 4.0;
                self.text(s, &percent, 15.0, Face::Number(600), RED, bx - pw - 8.0, mid, pw + 2.0, start);
                let shell = Rect { x: bx, y: mid - 7.0, w: bw, h: 14.0 };
                s.push_rounded_clip(&gsk::RoundedRect::from_rect(shell.graphene(), 4.0));
                s.append_color(&RED.alpha(0.35).rgba(), &shell.graphene());
                let fill = Rect { x: bx, y: mid - 7.0, w: bw * (*level as f64 / 100.0).clamp(0.08, 1.0), h: 14.0 };
                s.append_color(&RED.rgba(), &fill.graphene());
                s.pop();
                let nub = Rect { x: bx + bw + 1.5, y: mid - 3.0, w: 2.5, h: 6.0 };
                s.push_rounded_clip(&gsk::RoundedRect::from_rect(nub.graphene(), 1.2));
                s.append_color(&RED.alpha(0.6).rgba(), &nub.graphene());
                s.pop();
            }
            Head::Peek(Peek::Device { name, icon, battery, on }) => {
                self.icon(s, icon, 20.0, true, pal.ink_main, l0 + 11.0, mid);
                self.text(s, name, 14.0, REST, pal.ink_meta, l0 + 28.0, mid, left - 28.0, start);
                match (battery, on) {
                    (Some(b), true) => {
                        let ring = if *b <= 20 { RED } else { GREEN };
                        let cx = r1 - 11.0;
                        let path = |frac: f64| {
                            let pb = gsk::PathBuilder::new();
                            let r = 9.0f32;
                            let start = -std::f64::consts::FRAC_PI_2;
                            let steps = (48.0 * frac).ceil().max(1.0) as usize;
                            for k in 0..=steps {
                                let a = start + frac * std::f64::consts::TAU * k as f64 / steps as f64;
                                let (x, y) = ((cx + r as f64 * a.cos()) as f32, (mid + r as f64 * a.sin()) as f32);
                                if k == 0 { pb.move_to(x, y) } else { pb.line_to(x, y) }
                            }
                            pb.to_path()
                        };
                        let stroke = gsk::Stroke::new(3.0);
                        stroke.set_line_cap(gsk::LineCap::Round);
                        s.append_stroke(&path(1.0), &stroke, &ring.alpha(0.25).rgba());
                        s.append_stroke(&path((*b as f64 / 100.0).clamp(0.02, 1.0)), &stroke, &ring.rgba());
                    }
                    (_, true) => self.text(s, "Connected", 14.0, POINT, GREEN, r0, mid, right, end),
                    (_, false) => self.text(s, "Disconnected", 14.0, REST, pal.ink_faint, r0, mid, right, end),
                }
            }
            Head::Peek(Peek::Output { name, icon }) => {
                self.icon(s, icon, 20.0, true, ACTION_BLUE, l0 + 11.0, mid);
                self.text(s, name, 14.0, POINT, pal.ink_main, r0 - 60.0, mid, right + 60.0, end);
            }
            Head::Peek(Peek::Privacy { light, who }) => {
                let color = match light {
                    crate::privacy::Light::Camera => GREEN,
                    crate::privacy::Light::Mic => ORANGE,
                    crate::privacy::Light::Screen | crate::privacy::Light::Record => RED,
                };
                let disc = graphene::Rect::new(l0 as f32, (mid - 12.0) as f32, 24.0, 24.0);
                s.push_rounded_clip(&gsk::RoundedRect::from_rect(disc, 12.0));
                s.append_color(&color.alpha(0.2).rgba(), &disc);
                s.pop();
                self.icon(s, light.icon(), 16.0, true, color, l0 + 12.0, mid);
                self.text(s, light.label(), 15.0, REST, pal.ink_main, l0 + 32.0, mid, left - 32.0, start);
                self.text(s, who, 15.0, POINT, color.mix(pal.ink_main, 0.55), r0, mid, right, end);
            }
            Head::Peek(Peek::Note { text }) => {
                self.text(s, text, 15.0, POINT, pal.ink_main, l0, mid, left, start);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_note(&self, s: &gtk::Snapshot, mid: f64, summary: &str, body: &str, app: &str, logo: Option<&gdk::Paintable>, face: Option<&gdk::Paintable>) {
        let pal = self.pal.get();
        let half = self.w.target() / 2.0;
        let left = CX - half + 18.0;
        let right = CX + half - 22.0 - self.room();
        let rich = !body.is_empty();
        let size = if rich { 46.0 } else { 36.0 };
        let (ax, ay) = (left, mid - size / 2.0);
        match (face, logo) {
            (Some(f), _) => self.draw_paintable(s, f, ax, ay, size, size / 2.0),
            (None, Some(l)) => self.draw_paintable(s, l, ax, ay, size, size * 0.26),
            (None, None) => self.draw_monogram(s, app, ax, mid, size),
        }
        if let (Some(_), Some(l)) = (face, logo) {
            let b = if rich { 20.0 } else { 16.0 };
            let (bx, by) = (ax + size - b + 4.0, ay + size - b + 4.0);
            let ring = graphene::Rect::new((bx - 2.0) as f32, (by - 2.0) as f32, (b + 4.0) as f32, (b + 4.0) as f32);
            s.push_rounded_clip(&gsk::RoundedRect::from_rect(ring, ((b + 4.0) * 0.3) as f32));
            s.append_color(&pal.body.alpha(1.0).rgba(), &ring);
            s.pop();
            self.draw_paintable(s, l, bx, by, b, b * 0.26);
        }
        let start = pango::Alignment::Left;
        let tx = ax + size + 14.0;
        let stamp = self.text_width("now", 14.0, REST);
        let line = if rich { mid - 11.0 } else { mid };
        self.text(s, summary, 16.0, Face::Text(600), pal.ink_main, tx, line, (right - stamp - 12.0 - tx).max(0.0), start);
        self.text(s, "now", 14.0, REST, pal.ink_faint, right - stamp, line, stamp + 2.0, start);
        if rich {
            self.text(s, body, 15.0, REST, pal.ink_meta, tx, mid + 11.0, (right - tx).max(0.0), start);
        }
    }

    /// Fallback logo for apps that ship no icon: their initial on a quiet rounded square.
    fn draw_monogram(&self, s: &gtk::Snapshot, app: &str, x: f64, mid: f64, size: f64) {
        let pal = self.pal.get();
        let rect = graphene::Rect::new(x as f32, (mid - size / 2.0) as f32, size as f32, size as f32);
        s.push_rounded_clip(&gsk::RoundedRect::from_rect(rect, (size * 0.24) as f32));
        s.append_color(&pal.ink_meta.alpha(0.22).rgba(), &rect);
        s.pop();
        match app.chars().find(|c| c.is_alphanumeric()) {
            Some(c) => {
                let letter: String = c.to_uppercase().collect();
                self.text(s, &letter, (size * 0.5).round(), Face::Text(650), pal.ink_main, x, mid, size, pango::Alignment::Center);
            }
            None => self.icon(s, "notifications", (size * 0.7).round(), false, pal.ink_main, x + size / 2.0, mid),
        }
    }

    /// Draws a square, slightly rounded paintable; non-square images are centre-cropped.
    #[allow(clippy::too_many_arguments)]
    fn draw_paintable(&self, s: &gtk::Snapshot, p: &gdk::Paintable, x: f64, y: f64, size: f64, radius: f64) {
        let (iw, ih) = (p.intrinsic_width().max(1) as f64, p.intrinsic_height().max(1) as f64);
        let k = (size / iw).max(size / ih);
        let (w, h) = if p.intrinsic_width() > 0 && p.intrinsic_height() > 0 { (iw * k, ih * k) } else { (size, size) };
        let clip = graphene::Rect::new(x as f32, y as f32, size as f32, size as f32);
        s.push_rounded_clip(&gsk::RoundedRect::from_rect(clip, radius as f32));
        s.save();
        s.translate(&graphene::Point::new((x + (size - w) / 2.0) as f32, (y + (size - h) / 2.0) as f32));
        p.snapshot(s, w, h);
        s.restore();
        s.pop();
    }

    fn bar(&self, s: &gtk::Snapshot, r: &Rect, value: f64, track: Rgb, fill: Rgb) {
        if r.w <= 0.0 {
            return;
        }
        let radius = (r.h / 2.0) as f32;
        s.push_rounded_clip(&gsk::RoundedRect::from_rect(r.graphene(), radius));
        s.append_color(&track.rgba(), &r.graphene());
        let filled = Rect { w: r.w * value.clamp(0.0, 1.0), ..*r };
        s.append_color(&fill.rgba(), &filled.graphene());
        s.pop();
    }

    fn draw_art(&self, s: &gtk::Snapshot, r: &Rect, radius: f64, scale: f64) {
        let pal = self.pal.get();
        let (w, h) = (r.w * scale, r.h * scale);
        let rect = graphene::Rect::new((r.x + (r.w - w) / 2.0) as f32, (r.y + (r.h - h) / 2.0) as f32, w as f32, h as f32);
        s.push_rounded_clip(&gsk::RoundedRect::from_rect(rect, (radius * scale) as f32));
        s.append_color(&pal.raised.rgba(), &rect);
        let art = self.app.media.borrow().art.clone();
        match art {
            Some(texture) => {
                let (tw, th) = (texture.width() as f64, texture.height() as f64);
                let k = (w / tw).max(h / th);
                let (dw, dh) = (tw * k, th * k);
                let bounds = graphene::Rect::new(
                    (rect.x() as f64 + (w - dw) / 2.0) as f32,
                    (rect.y() as f64 + (h - dh) / 2.0) as f32,
                    dw as f32,
                    dh as f32,
                );
                s.append_scaled_texture(&texture, gsk::ScalingFilter::Trilinear, &bounds);
            }
            None => {
                let px = if r.w > 40.0 { 36.0 } else { 14.0 };
                self.icon(s, "music_note", px * scale, true, pal.ink_meta, r.x + r.w / 2.0, r.y + r.h / 2.0);
            }
        }
        s.pop();
    }

    /// Whether the open notch shows the call card rather than the player.
    fn showing_call(&self, now: i64) -> bool {
        if self.app.call.borrow().is_none() {
            self.media_over_call.set(false);
            return false;
        }
        !(self.media_over_call.get() && self.app.media.borrow().present(now))
    }

    /// A call and a player are both around, so the media spot can flip between them.
    fn can_switch(&self, now: i64) -> bool {
        self.app.call.borrow().is_some() && self.app.media.borrow().present(now)
    }

    fn switch_rect(&self) -> Rect {
        Rect { x: LEFT + MEDIA_W - 34.0, y: self.body_top() + 6.0, w: 34.0, h: 34.0 }
    }

    fn draw_switch(&self, s: &gtk::Snapshot, now: i64) {
        if !self.can_switch(now) {
            return;
        }
        let pal = self.pal.get();
        let r = self.switch_rect();
        let glow = self.glow[10].value(now).clamp(0.0, 1.0);
        s.push_rounded_clip(&gsk::RoundedRect::from_rect(r.graphene(), (r.h / 2.0) as f32));
        s.append_color(&pal.ink_main.alpha(0.09 + 0.07 * glow).rgba(), &r.graphene());
        s.pop();
        let icon = if self.showing_call(now) { "music_note" } else { "call" };
        self.icon(s, icon, 18.0, true, pal.ink_main, r.x + r.w / 2.0, r.y + r.h / 2.0);
    }

    fn call_buttons(&self) -> (Rect, Rect) {
        let y = self.body_top() + 76.0;
        (Rect { x: LEFT + 82.0, y, w: 104.0, h: 34.0 }, Rect { x: LEFT + 194.0, y, w: 104.0, h: 34.0 })
    }

    fn draw_call(&self, s: &gtk::Snapshot, now: i64) {
        let pal = self.pal.get();
        let start = pango::Alignment::Left;
        let Some((who, since)) = self.app.call.borrow().as_ref().map(|c| (c.who.clone(), c.since)) else { return };
        let top = self.body_top();
        let icon_rect = Rect { x: LEFT, y: top + 4.0, w: 68.0, h: 68.0 };
        let gicon = {
            let list = self.app.apps.list();
            list.iter().find(|e| e.name.eq_ignore_ascii_case(&who)).and_then(|e| e.icon.clone())
        };
        let scale = self.window.scale_factor().max(1);
        let paint: Option<gdk::Paintable> = gicon.and_then(|g| {
            let theme = gtk::IconTheme::for_display(&WidgetExt::display(&self.window));
            Some(theme.lookup_by_gicon(&g, 68, scale, gtk::TextDirection::Ltr, gtk::IconLookupFlags::empty()).upcast())
        });
        match paint {
            Some(p) => self.draw_paintable(s, &p, icon_rect.x, icon_rect.y, 68.0, 16.0),
            None => self.draw_monogram(s, &who, icon_rect.x, icon_rect.y + 34.0, 68.0),
        }
        let dot = graphene::Rect::new((LEFT + 82.0) as f32, (top + 12.0) as f32, 8.0, 8.0);
        let pulse = if motion::reduced() {
            1.0
        } else {
            0.55 + 0.45 * ((now as f64 / 1_000_000.0) * std::f64::consts::PI).sin().abs()
        };
        s.push_rounded_clip(&gsk::RoundedRect::from_rect(dot, 4.0));
        s.append_color(&GREEN.alpha(pulse).rgba(), &dot);
        s.pop();
        let t = ((now - since).max(0) / 1_000_000) as u64;
        let clock = if t >= 3600 { format!("{}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60) } else { format!("{}:{:02}", t / 60, t % 60) };
        let text_w = MEDIA_W - 82.0 - if self.can_switch(now) { 42.0 } else { 0.0 };
        let place = self.app.call_place.borrow().clone();
        match place {
            Some((channel, server)) => {
                self.text(s, &format!("On a call · {who}"), 13.0, REST, GREEN, LEFT + 96.0, top + 16.0, text_w - 14.0, start);
                self.text(s, &channel, 17.0, Face::Text(620), pal.ink_main, LEFT + 82.0, top + 38.0, text_w, start);
                let sub = if server.is_empty() { clock.clone() } else { format!("{server} · {clock}") };
                self.text(s, &sub, 14.0, Face::Text(480), pal.ink_meta, LEFT + 82.0, top + 60.0, text_w, start);
            }
            None => {
                self.text(s, "On a call", 13.0, REST, GREEN, LEFT + 96.0, top + 16.0, text_w - 14.0, start);
                self.text(s, &who, 17.0, Face::Text(620), pal.ink_main, LEFT + 82.0, top + 38.0, text_w, start);
                self.text(s, &clock, 15.0, Face::Number(560), pal.ink_meta, LEFT + 82.0, top + 60.0, text_w, start);
            }
        }
        let muted = self.app.mic_muted.get().unwrap_or(false);
        let (mute, open) = self.call_buttons();
        for (i, (r, icon, label, tint)) in [(mute, if muted { "mic_off" } else { "mic" }, if muted { "Unmute" } else { "Mute" }, if muted { RED } else { pal.ink_main }), (open, "open_in_new", "Show", pal.ink_main)].into_iter().enumerate() {
            let glow = self.glow[i].value(now).clamp(0.0, 1.0);
            let fill = if i == 0 && muted { RED.alpha(0.18 + 0.08 * glow) } else { pal.ink_main.alpha(0.09 + 0.07 * glow) };
            s.push_rounded_clip(&gsk::RoundedRect::from_rect(r.graphene(), 17.0));
            s.append_color(&fill.rgba(), &r.graphene());
            s.pop();
            self.icon(s, icon, 17.0, true, tint, r.x + 22.0, r.y + r.h / 2.0);
            self.text(s, label, 13.0, POINT, tint, r.x + 36.0, r.y + r.h / 2.0, r.w - 42.0, start);
        }
    }

    fn player(&self, now: i64) -> Option<Player> {
        if self.showing_call(now) || !self.app.media.borrow().present(now) {
            return None;
        }
        let top = self.body_top();
        let art = Rect { x: LEFT, y: top + 4.0, w: 104.0, h: 104.0 };
        let info_x = LEFT + 122.0;
        let info_w = MEDIA_W - 122.0 - if self.can_switch(now) { 42.0 } else { 0.0 };
        let ccx = info_x + 52.0;
        let button = |cx: f64| Rect { x: cx - 18.0, y: top + 96.0 - 18.0, w: 36.0, h: 36.0 };
        Some(Player {
            art,
            info_x,
            info_w,
            top,
            seek: Rect { x: info_x, y: top + 54.0, w: info_w, h: 16.0 },
            controls: [(button(ccx - 44.0), Action::Previous), (button(ccx), Action::Toggle), (button(ccx + 44.0), Action::Next)],
        })
    }

    fn draw_sheet(&self, s: &gtk::Snapshot, now: i64) {
        let fade = self.tab_fade.value(now).clamp(0.0, 1.0);
        if fade < 0.01 {
            return;
        }
        s.push_opacity(fade);
        match self.tab {
            Tab::Nook => self.draw_nook(s, now),
            Tab::Tray => self.draw_tray(s, now),
            Tab::Phone => self.draw_phone(s, now),
        }
        s.pop();
    }

    fn divider(&self, s: &gtk::Snapshot, x: f64) {
        let top = self.body_top();
        let line = graphene::Rect::new(x as f32, (top + 8.0) as f32, 1.0, 96.0);
        s.append_color(&self.pal.get().ink_main.alpha(0.08).rgba(), &line);
    }

    fn draw_nook(&self, s: &gtk::Snapshot, now: i64) {
        self.draw_media(s, now);
        self.divider(s, LEFT + MEDIA_W + 14.0);
        self.draw_calendar(s, now);
        self.divider(s, CAL_X + CAL_W + 14.0);
        self.draw_tools(s, now);
    }

    fn draw_media(&self, s: &gtk::Snapshot, now: i64) {
        self.draw_switch(s, now);
        s.push_opacity(self.flip.value(now).clamp(0.0, 1.0));
        self.draw_media_body(s, now);
        s.pop();
    }

    fn draw_media_body(&self, s: &gtk::Snapshot, now: i64) {
        if self.showing_call(now) {
            self.draw_call(s, now);
            return;
        }
        let pal = self.pal.get();
        let start = pango::Alignment::Left;
        let Some(p) = self.player(now) else {
            let mid = self.body_top() + 56.0;
            self.icon(s, "music_note", 28.0, true, pal.ink_faint, LEFT + MEDIA_W / 2.0, mid - 12.0);
            self.text(s, "Nothing playing", 14.0, REST, pal.ink_meta, LEFT, mid + 18.0, MEDIA_W, pango::Alignment::Center);
            return;
        };
        let media = self.app.media.borrow().clone();
        self.draw_art(s, &p.art, 20.0, self.art_scale.value(now));
        if let Some(badge) = self.badge_icon() {
            let b = 26.0;
            let (bx, by) = (p.art.x + p.art.w - b + 5.0, p.art.y + p.art.h - b + 5.0);
            let ring = graphene::Rect::new((bx - 3.0) as f32, (by - 3.0) as f32, (b + 6.0) as f32, (b + 6.0) as f32);
            s.push_rounded_clip(&gsk::RoundedRect::from_rect(ring, 10.0));
            s.append_color(&pal.body.alpha(1.0).rgba(), &ring);
            s.pop();
            self.draw_paintable(s, &badge, bx, by, b, 7.0);
        }
        self.text(s, &media.title, 16.0, Face::Text(600), pal.ink_main, p.info_x, p.top + 13.0, p.info_w, start);
        self.text(s, &media.artist, 14.0, REST, pal.ink_meta, p.info_x, p.top + 34.0, p.info_w, start);
        let bar_y = p.seek.y + p.seek.h / 2.0;
        let small = |t: f64| {
            let t = t.max(0.0) as u64;
            if t >= 3600 { format!("{}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60) } else { format!("{}:{:02}", t / 60, t % 60) }
        };
        let face = Face::Number(450);
        let half = p.info_w / 2.0;
        match media.timing {
            Timing::Timed => {
                let elapsed = self.scrub.map_or_else(|| media.elapsed(now), |f| f * media.length);
                let progress = (elapsed / media.length).clamp(0.0, 1.0) * self.fill.value(now).clamp(0.0, 1.0);
                let lift = if media.can_seek { self.glow[3].value(now).clamp(0.0, 1.0) } else { 0.0 };
                let thick = 4.0 + 2.0 * lift;
                let track = Rect { x: p.info_x, y: bar_y - thick / 2.0, w: p.info_w, h: thick };
                self.bar(s, &track, progress, pal.ink_main.alpha(0.15 + 0.05 * lift), pal.ink_main);
                if media.can_seek && (lift > 0.01 || self.scrub.is_some()) {
                    let r = if self.scrub.is_some() { 6.5 } else { 3.0 + 3.0 * lift };
                    let knob = graphene::Rect::new((track.x + track.w * progress - r) as f32, (bar_y - r) as f32, (2.0 * r) as f32, (2.0 * r) as f32);
                    s.push_rounded_clip(&gsk::RoundedRect::from_rect(knob, r as f32));
                    s.append_color(&pal.ink_main.rgba(), &knob);
                    s.pop();
                }
                self.text(s, &small(elapsed), 11.0, face, pal.ink_faint, p.info_x, bar_y + 14.0, half, start);
                self.text(s, &small(media.length), 11.0, face, pal.ink_faint, p.info_x + half, bar_y + 14.0, half, pango::Alignment::Right);
            }
            Timing::Live => {
                let pill = Rect { x: p.info_x, y: bar_y - 9.0, w: 48.0, h: 18.0 };
                s.push_rounded_clip(&gsk::RoundedRect::from_rect(pill.graphene(), 9.0));
                s.append_color(&pal.risk.rgba(), &pill.graphene());
                s.pop();
                self.text(s, "LIVE", 11.0, Face::Text(700), Rgb::new(1.0, 1.0, 1.0), pill.x, pill.y + pill.h / 2.0, pill.w, pango::Alignment::Center);
                self.text(s, &media.source, 12.0, REST, pal.ink_meta, pill.x + pill.w + 10.0, bar_y, p.info_w - pill.w - 10.0, start);
            }
            Timing::Untimed => {
                let track = Rect { x: p.info_x, y: bar_y - 2.0, w: p.info_w, h: 4.0 };
                self.bar(s, &track, 0.0, pal.ink_main.alpha(0.15), pal.ink_main);
                if media.counted {
                    self.text(s, &small(media.elapsed(now)), 11.0, face, pal.ink_faint, p.info_x, bar_y + 14.0, half, start);
                }
            }
        }
        for (i, (r, action)) in p.controls.iter().enumerate() {
            let (name, px, enabled) = match action {
                Action::Previous => ("skip_previous", 22.0, media.can_prev),
                Action::Toggle => (if media.playing { "pause" } else { "play_arrow" }, 28.0, true),
                Action::Next => ("skip_next", 22.0, media.can_next),
            };
            let glow = if enabled { self.glow[i].value(now).clamp(0.0, 1.0) } else { 0.0 };
            let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
            if glow > 0.01 {
                let rad = r.w / 2.0 + 3.0 * glow;
                let halo = graphene::Rect::new((cx - rad) as f32, (cy - rad) as f32, (2.0 * rad) as f32, (2.0 * rad) as f32);
                s.push_rounded_clip(&gsk::RoundedRect::from_rect(halo, rad as f32));
                s.append_color(&pal.ink_main.alpha(0.1 * glow).rgba(), &halo);
                s.pop();
            }
            let ink = if enabled { pal.ink_main } else { pal.ink_main.alpha(0.28) };
            self.icon(s, name, px, true, ink, cx, cy);
        }
    }

    fn draw_calendar(&self, s: &gtk::Snapshot, now: i64) {
        let pal = self.pal.get();
        let top = self.body_top();
        let days = self.day_rects();
        if days.len() < 7 {
            return;
        }
        let sel = (3 + self.day).clamp(0, 6) as usize;
        let center = pango::Alignment::Center;
        self.text(s, &crate::calendar::month(&days[sel].1), 24.0, Face::Text(650), pal.ink_main, CAL_X, top + 23.0, 64.0, pango::Alignment::Left);
        let glow = self.glow[9].value(now).clamp(0.0, 1.0);
        for (i, (r, d)) in days.iter().enumerate() {
            let hot = self.hot == Some(Hot::Day(i));
            let back = if i == sel && !d.today { 0.11 } else { 0.0 } + if hot && !d.today { 0.06 * glow } else { 0.0 };
            if back > 0.005 {
                let pill = graphene::Rect::new((r.x + 1.0) as f32, (r.y - 2.0) as f32, (r.w - 2.0) as f32, (r.h + 4.0) as f32);
                s.push_rounded_clip(&gsk::RoundedRect::from_rect(pill, 10.0));
                s.append_color(&pal.ink_main.alpha(back).rgba(), &pill);
                s.pop();
            }
            let weekend = d.weekday >= 6;
            let letter = if d.today { crate::calendar::label(d) } else { d.letter.clone() };
            let quiet = if weekend { pal.risk.mix(pal.ink_meta, 0.55) } else { pal.ink_meta };
            self.text(s, &letter, 11.5, Face::Text(650), if d.today { pal.accent } else { quiet }, r.x - 8.0, r.y + 9.0, r.w + 16.0, center);
            let (px, face) = if d.today { (21.0, Face::Number(650)) } else { (15.0, Face::Number(500)) };
            let ink = if d.today { pal.accent } else if weekend { pal.risk.mix(pal.ink_meta, 0.5) } else { pal.ink_meta };
            self.text(s, &format!("{:02}", d.number), px, face, ink, r.x - 6.0, r.y + 31.0, r.w + 12.0, center);
        }
        let day = &days[sel].1;
        let events = crate::calendar::on(day);
        let ey = top + 66.0;
        if events.is_empty() {
            let msg = if day.today { "Nothing for today".to_string() } else { format!("Nothing on {}", crate::calendar::name(day)) };
            let tw = self.text_width(&msg, 13.0, REST);
            let x = CAL_X + (CAL_W - tw - 22.0) / 2.0;
            self.icon(s, "event_available", 16.0, false, pal.ink_faint, x + 8.0, ey + 16.0);
            self.text(s, &msg, 13.0, REST, pal.ink_meta, x + 22.0, ey + 16.0, tw + 2.0, pango::Alignment::Left);
            return;
        }
        for (k, e) in events.iter().take(2).enumerate() {
            let y = ey + 6.0 + k as f64 * 22.0;
            let mark = graphene::Rect::new(CAL_X as f32, (y - 8.0) as f32, 3.0, 16.0);
            s.push_rounded_clip(&gsk::RoundedRect::from_rect(mark, 1.5));
            s.append_color(&pal.accent.rgba(), &mark);
            s.pop();
            let when = if e.all_day { "All day".to_string() } else { crate::calendar::clock(e.start) };
            self.text(s, &when, 12.0, Face::Number(500), pal.ink_meta, CAL_X + 10.0, y, 52.0, pango::Alignment::Left);
            self.text(s, &e.title, 13.0, REST, pal.ink_main, CAL_X + 64.0, y, CAL_W - 64.0, pango::Alignment::Left);
        }
        if events.len() > 2 {
            self.text(s, &format!("+{} more", events.len() - 2), 11.0, REST, pal.ink_faint, CAL_X + 10.0, ey + 50.0, CAL_W - 10.0, pango::Alignment::Left);
        }
    }

    fn draw_tools(&self, s: &gtk::Snapshot, now: i64) {
        let pal = self.pal.get();
        let glow = self.glow[9].value(now).clamp(0.0, 1.0);
        let awake = self.app.awake.borrow().as_ref().is_some_and(|a| a.on());
        let mut label = None;
        for (i, (icon, name)) in TOOLS.iter().enumerate() {
            let r = self.tool_rect(i);
            let on = match i {
                2 => self.app.dnd.get(),
                3 => awake,
                _ => false,
            };
            let hot = self.hot == Some(Hot::Tool(i));
            if hot {
                label = Some(*name);
            }
            let fill = if on { pal.accent } else { pal.ink_main.alpha(0.1 + if hot { 0.08 * glow } else { 0.0 }) };
            s.push_rounded_clip(&gsk::RoundedRect::from_rect(r.graphene(), 15.0));
            s.append_color(&fill.rgba(), &r.graphene());
            s.pop();
            let ink = if on { pal.on_accent } else { pal.ink_main };
            self.icon(s, icon, 20.0, on, ink, r.x + r.w / 2.0, r.y + r.h / 2.0);
        }
        if let Some(name) = label {
            let r = self.tool_rect(2);
            self.text(s, name, 11.0, REST, pal.ink_meta.alpha(glow), TOOLS_X - 6.0, r.y + r.h + 13.0, RIGHT - TOOLS_X + 12.0, pango::Alignment::Center);
        }
    }

    fn draw_phone(&self, s: &gtk::Snapshot, now: i64) {
        let pal = self.pal.get();
        let start = pango::Alignment::Left;
        let center = pango::Alignment::Center;
        let top = self.body_top();
        let Some(phone) = crate::phone::phone() else {
            let (title, sub) = if crate::phone::daemon() {
                ("No phone paired", "Pair your phone in the KDE Connect app to see it here")
            } else {
                ("KDE Connect isn't running", "Install kdeconnect and pair your phone to see it here")
            };
            let mid = top + 56.0;
            self.icon(s, "smartphone", 30.0, true, pal.ink_faint, CX, mid - 22.0);
            self.text(s, title, 15.0, POINT, pal.ink_main, LEFT, mid + 12.0, RIGHT - LEFT, center);
            self.text(s, sub, 12.0, REST, pal.ink_meta, LEFT, mid + 32.0, RIGHT - LEFT, center);
            return;
        };
        let p = self.phone_rect();
        s.push_rounded_clip(&gsk::RoundedRect::from_rect(p.graphene(), 20.0));
        s.append_color(&pal.ink_main.alpha(0.04).rgba(), &p.graphene());
        s.pop();
        let disc = graphene::Rect::new((p.x + 14.0) as f32, (p.y + 14.0) as f32, 40.0, 40.0);
        s.push_rounded_clip(&gsk::RoundedRect::from_rect(disc, 20.0));
        s.append_color(&(if phone.reachable { pal.accent.alpha(0.18) } else { pal.ink_main.alpha(0.08) }).rgba(), &disc);
        s.pop();
        self.icon(s, phone.glyph(), 22.0, true, if phone.reachable { pal.accent } else { pal.ink_meta }, p.x + 34.0, p.y + 34.0);
        let next = crate::phone::next();
        let name_w = p.w - 80.0 - if next.is_some() { 36.0 } else { 0.0 };
        self.text(s, &phone.name, 16.0, Face::Text(620), pal.ink_main, p.x + 66.0, p.y + 25.0, name_w, start);
        let glow = self.glow[9].value(now).clamp(0.0, 1.0);
        if next.is_some() {
            let r = self.phone_switch_rect();
            let hot = self.hot == Some(Hot::PhoneSwitch);
            s.push_rounded_clip(&gsk::RoundedRect::from_rect(r.graphene(), 15.0));
            s.append_color(&pal.ink_main.alpha(if hot { 0.09 + 0.08 * glow } else { 0.07 }).rgba(), &r.graphene());
            s.pop();
            self.icon(s, "swap_horiz", 17.0, false, if hot { pal.ink_main } else { pal.ink_meta }, r.x + r.w / 2.0, r.y + r.h / 2.0);
        }
        let mut status = Vec::new();
        if !phone.reachable {
            status.push("Not reachable".to_string());
        }
        if let Some((charge, charging)) = phone.battery {
            status.push(format!("{charge}%{}", if charging { " · Charging" } else { "" }));
        }
        if let Some((kind, _)) = &phone.signal {
            status.push(kind.clone());
        }
        // The status line gives way to the hovered button's name, then to an action's outcome.
        let hovered = match self.hot {
            Some(Hot::PhoneAction(i)) if phone.reachable => crate::phone::ACTIONS.get(i).map(|a| a.2.to_string()),
            Some(Hot::PhoneSwitch) => next.as_ref().map(|n| format!("Switch to {}", n.name)),
            _ => None,
        };
        let line = hovered.or_else(crate::phone::note).unwrap_or_else(|| if status.is_empty() { "Connected".to_string() } else { status.join(" · ") });
        self.text(s, &line, 12.0, REST, if phone.reachable { pal.ink_meta } else { pal.ink_faint }, p.x + 66.0, p.y + 45.0, p.w - 80.0, start);
        for (i, (_, icon, _)) in crate::phone::ACTIONS.iter().enumerate() {
            let r = self.phone_action_rect(i);
            let hot = self.hot == Some(Hot::PhoneAction(i));
            let fill = if !phone.reachable { 0.04 } else if hot { 0.09 + 0.08 * glow } else { 0.09 };
            s.push_rounded_clip(&gsk::RoundedRect::from_rect(r.graphene(), 18.0));
            s.append_color(&pal.ink_main.alpha(fill).rgba(), &r.graphene());
            s.pop();
            let ink = if phone.reachable { pal.ink_main } else { pal.ink_faint };
            self.icon(s, icon, 18.0, true, ink, r.x + r.w / 2.0, r.y + r.h / 2.0);
        }
        let x = LEFT + PHONE_W + 14.0;
        let w = RIGHT - x;
        let call = crate::phone::call();
        if phone.notes.is_empty() && call.is_none() {
            let msg = if phone.reachable { "No notifications from your phone" } else { "Notifications show up while the phone is reachable" };
            self.icon(s, "notifications", 24.0, false, pal.ink_faint, x + w / 2.0, top + 44.0);
            self.text(s, msg, 13.0, REST, pal.ink_meta, x, top + 76.0, w, center);
            return;
        }
        let mut row = 0;
        if let Some(c) = &call {
            let r = self.note_rect(row);
            let mid = r.y + r.h / 2.0;
            self.icon(s, "call", 18.0, true, GREEN, r.x + 16.0, mid);
            self.text(s, c, 13.0, Face::Text(560), pal.ink_main, r.x + 36.0, mid, r.w - 44.0, start);
            row += 1;
        }
        for (i, n) in phone.notes.iter().enumerate().take(NOTES.saturating_sub(row)) {
            let r = self.note_rect(row);
            row += 1;
            let hot = self.hot == Some(Hot::PhoneDismiss(i));
            let mid = r.y + r.h / 2.0;
            match &n.icon {
                Some(t) => self.draw_paintable(s, t.upcast_ref(), r.x + 4.0, mid - 12.0, 24.0, 6.0),
                None => self.icon(s, "notifications", 18.0, false, pal.ink_meta, r.x + 16.0, mid),
            }
            let head = if n.title.is_empty() { n.app.clone() } else if n.app.is_empty() || n.app == n.title { n.title.clone() } else { format!("{} · {}", n.app, n.title) };
            let room = r.w - 44.0 - if n.dismissable { 30.0 } else { 0.0 };
            self.text(s, &head, 13.0, Face::Text(560), pal.ink_main, r.x + 36.0, mid - 8.0, room, start);
            self.text(s, &n.text, 12.0, REST, pal.ink_meta, r.x + 36.0, mid + 9.0, room, start);
            if n.dismissable {
                let cx = r.x + r.w - 14.0;
                if hot {
                    let d = graphene::Rect::new((cx - 11.0) as f32, (mid - 11.0) as f32, 22.0, 22.0);
                    s.push_rounded_clip(&gsk::RoundedRect::from_rect(d, 11.0));
                    s.append_color(&pal.ink_main.alpha(0.1 + 0.06 * glow).rgba(), &d);
                    s.pop();
                }
                self.icon(s, "close", 14.0, false, if hot { pal.ink_main } else { pal.ink_faint }, cx, mid);
            }
        }
        let more = phone.notes.len() + call.is_some() as usize;
        if more > NOTES {
            self.text(s, &format!("+{} more", more - NOTES), 11.0, REST, pal.ink_faint, x, top + 4.0 + NOTES as f64 * 38.0 + 6.0, w, pango::Alignment::Right);
        }
    }

    fn draw_tray(&self, s: &gtk::Snapshot, now: i64) {
        let pal = self.pal.get();
        let start = pango::Alignment::Left;
        let center = pango::Alignment::Center;
        let f = self.files_rect();
        let over_files = self.landing == Some(Landing::Files);
        s.push_rounded_clip(&gsk::RoundedRect::from_rect(f.graphene(), 20.0));
        s.append_color(&pal.ink_main.alpha(if over_files { 0.1 } else { 0.04 }).rgba(), &f.graphene());
        s.pop();
        let outline = gsk::PathBuilder::new();
        outline.add_rounded_rect(&gsk::RoundedRect::from_rect(graphene::Rect::new((f.x + 1.0) as f32, (f.y + 1.0) as f32, (f.w - 2.0) as f32, (f.h - 2.0) as f32), 19.0));
        let stroke = gsk::Stroke::new(1.5);
        stroke.set_dash(&[7.0, 6.0]);
        s.append_stroke(&outline.to_path(), &stroke, &(if over_files { pal.accent } else { pal.ink_meta.alpha(0.45) }).rgba());
        let count = crate::tray::len();
        if count == 0 {
            self.icon(s, "inbox", 28.0, true, pal.accent, f.x + 38.0, f.y + 44.0);
            self.text(s, "Files Tray", 15.0, POINT, pal.accent, f.x + 24.0, f.y + 78.0, 200.0, start);
            self.text(s, "Drop files here to keep them close", 12.0, REST, pal.ink_faint, f.x + 24.0, f.y + 98.0, f.w - 48.0, start);
        } else {
            let glow = self.glow[9].value(now).clamp(0.0, 1.0);
            crate::tray::each(|i, item| {
                if i >= TILES {
                    return;
                }
                let t = self.tile_rect(i);
                let hot = matches!(self.hot, Some(Hot::Tile(j) | Hot::Remove(j)) if j == i);
                if hot {
                    s.push_rounded_clip(&gsk::RoundedRect::from_rect(t.graphene(), 14.0));
                    s.append_color(&pal.ink_main.alpha(0.08 * glow).rgba(), &t.graphene());
                    s.pop();
                }
                match &item.icon {
                    Some(p) => self.draw_paintable(s, p, t.x + (t.w - 48.0) / 2.0, t.y + 10.0, 48.0, 10.0),
                    None => self.icon(s, "draft", 40.0, false, pal.ink_meta, t.x + t.w / 2.0, t.y + 34.0),
                }
                self.text(s, &item.name, 11.0, REST, pal.ink_main, t.x + 4.0, t.y + 76.0, t.w - 8.0, center);
                if hot {
                    let x = self.remove_rect(i);
                    let on = self.hot == Some(Hot::Remove(i));
                    s.push_rounded_clip(&gsk::RoundedRect::from_rect(x.graphene(), 10.0));
                    s.append_color(&(if on { pal.risk } else { pal.ink_main.alpha(0.85) }).rgba(), &x.graphene());
                    s.pop();
                    self.icon(s, "close", 13.0, false, pal.body.alpha(1.0), x.x + x.w / 2.0, x.y + x.h / 2.0);
                }
            });
            if count > TILES {
                let t = self.tile_rect(TILES);
                self.text(s, &format!("+{}", count - TILES), 13.0, Face::Number(500), pal.ink_meta, t.x - 8.0, t.y + 34.0, 40.0, start);
            }
        }
        let z = self.share_rect();
        let over_share = matches!(self.landing, Some(Landing::Share | Landing::Device(_) | Landing::Sender(_)));
        s.push_rounded_clip(&gsk::RoundedRect::from_rect(z.graphene(), 20.0));
        s.append_color(&pal.accent.alpha(if over_share { 0.26 } else { 0.15 }).rgba(), &z.graphene());
        self.icon(s, "share", 18.0, true, pal.accent, z.x + 24.0, z.y + 21.0);
        self.text(s, "Share", 15.0, POINT, pal.ink_main, z.x + 40.0, z.y + 21.0, 80.0, start);
        let senders = crate::share::senders();
        let named = match (self.hot, self.landing) {
            (Some(Hot::Sender(i)), _) | (_, Some(Landing::Sender(i))) => senders.get(i).map(|s| format!("Send with {}", s.name)),
            _ => None,
        };
        let status = crate::share::status().map(|status| match status {
            crate::share::Status::Asking(n) => (format!("Waiting for {n}…"), pal.ink_meta),
            crate::share::Status::Sending(f) => (format!("Sending {f}…"), pal.ink_meta),
            crate::share::Status::Sent(n) => (format!("Sent to {n}"), GREEN),
            crate::share::Status::Failed(e) => (e, pal.risk),
        });
        let (label, ink) = match (named, status) {
            (Some(n), _) => (n, pal.ink_main),
            (None, Some(st)) => st,
            (None, None) => ("LocalSend · KDE Connect".to_string(), pal.ink_faint),
        };
        self.text(s, &label, 11.0, REST, ink, z.x + 100.0, z.y + 21.0, z.w - 116.0, pango::Alignment::Right);
        let devices = crate::share::devices();
        if devices.is_empty() {
            let msg = if crate::share::scanning() { "Looking for devices…" } else { "No devices nearby" };
            self.text(s, msg, 13.0, REST, pal.ink_meta, z.x, z.y + 54.0, z.w, center);
        }
        for (i, sender) in senders.iter().take(CHIPS).enumerate() {
            let r = self.chip_rect(i);
            let on = self.hot == Some(Hot::Sender(i)) || self.landing == Some(Landing::Sender(i));
            s.push_rounded_clip(&gsk::RoundedRect::from_rect(r.graphene(), 9.0));
            s.append_color(&pal.ink_main.alpha(if on { 0.2 } else { 0.08 }).rgba(), &r.graphene());
            s.pop();
            match &sender.icon {
                Some(p) => self.draw_paintable(s, p, r.x + 4.0, r.y + 4.0, r.w - 8.0, 4.0),
                None => self.icon(s, "language", 17.0, false, pal.ink_main, r.x + r.w / 2.0, r.y + r.h / 2.0),
            }
        }
        let glow = self.glow[9].value(now).clamp(0.0, 1.0);
        for (i, d) in devices.iter().take(DEVICES).enumerate() {
            let r = self.device_rect(i);
            let fill = if self.landing == Some(Landing::Device(i)) { 0.3 } else if self.hot == Some(Hot::Device(i)) { 0.12 * glow } else { 0.0 };
            if fill > 0.005 {
                s.push_rounded_clip(&gsk::RoundedRect::from_rect(r.graphene(), 10.0));
                s.append_color(&pal.accent.alpha(fill).rgba(), &r.graphene());
                s.pop();
            }
            let mid = r.y + r.h / 2.0;
            self.icon(s, d.icon(), 16.0, false, pal.ink_main, r.x + 16.0, mid);
            self.text(s, &d.name, 13.0, REST, pal.ink_main, r.x + 30.0, mid, r.w - 120.0, start);
            self.text(s, d.service(), 11.0, REST, pal.ink_faint, r.x + r.w - 92.0, mid, 84.0, pango::Alignment::Right);
        }
        s.pop();
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

fn dots_width(n: usize) -> f64 {
    if n == 0 { 0.0 } else { (n - 1) as f64 * (DOT + GAP) + PILL }
}
