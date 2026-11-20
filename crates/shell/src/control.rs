use crate::app::App;
use crate::audio::{self, Ask};
use crate::compositor;
use crate::draw::{self, Face};
use crate::panel_corner;
use gtk::subclass::prelude::*;
use gtk::{cairo, gdk, glib, graphene, gsk, pango, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kit::bluez;
use kit::corners::{Corner, Panel};
use kit::motion::{self, Tween};
use kit::nm;
use kit::palette::Rgb;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

mod detail;
mod sound;

const W: f64 = 380.0;
use crate::metrics::{FILLET, PANEL_PAD as PAD, PANEL_RADIUS as RADIUS};
const TILE_H: f64 = 64.0;
const TILE_GAP: f64 = crate::metrics::GAP;
const SOUND: f64 = 44.0;
const HEAD: f64 = 34.0;
const ROW: f64 = 64.0;
const EMPTY: f64 = 56.0;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Tile {
    Network,
    Bluetooth,
    Quiet,
    Awake,
    Night,
    Power,
}

#[derive(Clone, Default)]
struct Power {
    bus: String,
    path: String,
    active: String,
    list: Vec<String>,
}

async fn read_power() -> Option<Power> {
    let conn = crate::bus::system().await.ok()?;
    for (bus, path) in [("org.freedesktop.UPower.PowerProfiles", "/org/freedesktop/UPower/PowerProfiles"), ("net.hadess.PowerProfiles", "/net/hadess/PowerProfiles")] {
        let Ok(p) = zbus::Proxy::new_owned(conn.clone(), bus, path, bus).await else { continue };
        let call = p.get_property::<String>("ActiveProfile");
        let t = glib::timeout_future(std::time::Duration::from_millis(700));
        let active = match futures_util::future::select(Box::pin(call), Box::pin(t)).await {
            futures_util::future::Either::Left((Ok(a), _)) => a,
            _ => continue,
        };
        let list: Vec<String> = p
            .get_property::<Vec<HashMap<String, OwnedValue>>>("Profiles")
            .await
            .map(|v| v.iter().filter_map(|m| m.get("Profile").and_then(|x| x.downcast_ref::<String>().ok())).collect())
            .unwrap_or_default();
        return Some(Power { bus: bus.into(), path: path.into(), active, list });
    }
    None
}

fn profile_name(p: &str) -> &'static str {
    match p {
        "performance" => "Performance",
        "power-saver" => "Power Saver",
        _ => "Balanced",
    }
}

pub(crate) fn ring(s: &gtk::Snapshot, cx: f64, cy: f64, r: f64, frac: f64, width: f32, track: Rgb, tint: Rgb) {
    let path = |frac: f64| {
        let pb = gsk::PathBuilder::new();
        let start = -std::f64::consts::FRAC_PI_2;
        let steps = (64.0 * frac).ceil().max(1.0) as usize;
        for k in 0..=steps {
            let a = start + frac * std::f64::consts::TAU * k as f64 / steps as f64;
            let (px, py) = ((cx + r * a.cos()) as f32, (cy + r * a.sin()) as f32);
            if k == 0 { pb.move_to(px, py) } else { pb.line_to(px, py) }
        }
        pb.to_path()
    };
    let stroke = gsk::Stroke::new(width);
    stroke.set_line_cap(gsk::LineCap::Round);
    s.append_stroke(&path(1.0), &stroke, &track.rgba());
    s.append_stroke(&path(frac.clamp(0.02, 1.0)), &stroke, &tint.rgba());
}

#[derive(Clone, Copy, PartialEq)]
enum Hit {
    Tile(Tile),
    Mute,
    Slider,
    Clear,
    Row(usize),
    Close(usize),
    Detail(detail::D),
    /// The sound details: the chevron, an output or input device, an app's mute and slider.
    SoundMore,
    Out(usize),
    In(usize),
    AppMute(usize),
    AppLevel(usize),
}

#[derive(Clone, Default)]
struct Net {
    wifi: bool,
    wifi_on: bool,
    name: String,
    kind: String,
    online: bool,
}

const RED: Rgb = Rgb::new(1.0, 0.27, 0.23);
/// How quickly a wheel glide closes in on its target (as `kit::scroll`), and how long a touchpad
/// fling coasts.
const WHEEL_TAU: f64 = 70_000.0;
const FLING_TAU: f64 = 325_000.0;

/// Scrolling still to travel; each frame takes a share of it, so it eases out.
#[derive(Default)]
struct Glide {
    left: f64,
    tau: f64,
    at: i64,
}

impl Glide {
    fn push(&mut self, dy: f64, tau: f64, now: i64) {
        // Turning back drops what was left in the old direction.
        if self.left * dy < 0.0 {
            self.left = 0.0;
        }
        if self.left == 0.0 {
            self.at = now;
        }
        self.left += dy;
        self.tau = tau;
    }

    /// The part to move by at `now`.
    fn take(&mut self, now: i64) -> f64 {
        let dt = (now - self.at).max(0) as f64;
        self.at = now;
        let mut part = self.left * (1.0 - (-dt / self.tau.max(1.0)).exp());
        if (self.left - part).abs() < 0.5 {
            part = self.left;
        }
        self.left -= part;
        part
    }

    fn stop(&mut self) {
        self.left = 0.0;
    }
}
const CELL: f64 = 96.0;
const PER_ROW: usize = 4;

struct State {
    app: Rc<App>,
    window: gtk::Window,
    canvas: Canvas,
    open: bool,
    max_h: f64,
    screen: (f64, f64),
    corner: Corner,
    /// Where the panel opens: its hot corner, or beside the bar button that opened it.
    mount: Mount,
    /// Runs once when the panel closes, so the bar button lets go of its held look.
    on_close: Option<Box<dyn FnOnce()>>,
    margins: [i32; 4],
    net: Net,
    bt: bluez::Snapshot,
    busy: Vec<String>,
    wifi: nm::Wifi,
    expand: Option<Tile>,
    reveal: Tween,
    joining: Option<String>,
    pass: Option<detail::Pass>,
    armed: Option<String>,
    scanning: Option<String>,
    scan_round: u32,
    watch_gen: u32,
    rise: Tween,
    fade: Tween,
    height: Tween,
    scroll: f64,
    /// Scrolling still to travel and how fast it closes in, as in the apps (`kit::scroll`): a
    /// wheel notch adds a step that glides in; a touchpad fling coasts out more slowly.
    glide: Glide,
    hover: Option<Hit>,
    dragging: bool,
    hits: Vec<(Hit, f64, f64, f64, f64)>,
    slider: (f64, f64),
    region: (i32, i32, i32, i32),
    sound: sound::Sound,
    ticking: bool,
    polling: bool,
    scrim: Option<gtk::Window>,
    power: Option<Power>,
    stats: Option<crate::sysmon::Stats>,
    sampler: crate::sysmon::Sampler,
    sampling: bool,
}

#[derive(Clone)]
pub struct Control(Rc<RefCell<State>>);

pub fn show_on(app: &Rc<App>, output: Option<String>) {
    let existing = app.control.borrow().clone();
    let control = existing.unwrap_or_else(|| {
        let c = Control::new(app);
        *app.control.borrow_mut() = Some(c.clone());
        c
    });
    if !control.0.borrow().open {
        control.0.borrow_mut().mount = Mount::Corner;
        control.open_on(output);
    }
}

/// Opens Control Center with its sound details shown (`digit msg control sound`).
pub fn show_sound(app: &Rc<App>) {
    show_on(app, compositor::focused_output());
    if let Some(c) = app.control.borrow().clone() && !c.0.borrow().sound.open {
        c.toggle_sound();
    }
}

/// Where Control Center opens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mount {
    /// In its hot corner, against both screen edges.
    Corner,
    /// Hanging from the top bar against the screen's side edge, like the calendar
    /// on a notch. `left` is the side; `top` is where the bar ends.
    Hang { left: bool, top: f64 },
    /// A free card under the island, its right edge at `right` and its top at `top`.
    Card { right: f64, top: f64 },
}

/// The gap a card keeps from the screen's edges.
const CARD_AIR: f64 = 8.0;
use crate::metrics::CARD_DROP;

/// Opens or closes Control Center from a bar button whose right edge is at `right` and whose
/// bar ends at `below` (both in screen pixels). When the setting allows, the panel opens beside
/// that button: a card under the island, or hanging from a notch's bar like the calendar.
/// `on_close` runs once when the panel goes, so the button can stop looking pressed.
pub fn toggle_from(app: &Rc<App>, right: f64, below: f64, screen_w: f64, on_close: impl FnOnce() + 'static) {
    let mount = if !app.prefs.borrow().control_follows_button {
        Mount::Corner
    } else if app.floating() {
        Mount::Card { right, top: below }
    } else {
        Mount::Hang { left: right < screen_w / 2.0, top: if app.wide() { app.reserve() as f64 } else { below } }
    };
    let existing = app.control.borrow().clone();
    let control = existing.unwrap_or_else(|| {
        let c = Control::new(app);
        *app.control.borrow_mut() = Some(c.clone());
        c
    });
    if control.0.borrow().open {
        control.close();
    } else {
        {
            let mut s = control.0.borrow_mut();
            s.mount = mount;
            s.on_close = Some(Box::new(on_close));
        }
        control.open_on(compositor::focused_output());
    }
}

pub fn toggle(app: &Rc<App>) {
    let existing = app.control.borrow().clone();
    let control = existing.unwrap_or_else(|| {
        let c = Control::new(app);
        *app.control.borrow_mut() = Some(c.clone());
        c
    });
    control.toggle();
}

fn rounded(x: f64, y: f64, w: f64, h: f64, r: f64) -> gsk::RoundedRect {
    gsk::RoundedRect::from_rect(graphene::Rect::new(x as f32, y as f32, w as f32, h as f32), r as f32)
}

fn fill(s: &gtk::Snapshot, r: &gsk::RoundedRect, c: Rgb) {
    s.push_rounded_clip(r);
    s.append_color(&c.rgba(), r.bounds());
    s.pop();
}

fn ago(at: i64) -> String {
    let d = (glib::real_time() / 1_000_000 - at).max(0);
    match d {
        0..=59 => "now".into(),
        60..=3599 => format!("{}m", d / 60),
        3600..=86399 => format!("{}h", d / 3600),
        _ => format!("{}d", d / 86400),
    }
}

async fn read_net() -> Net {
    let mut net = Net::default();
    let Ok(conn) = crate::bus::system().await else { return net };
    let nm = |path: &str, iface: &'static str| zbus::Proxy::new_owned(conn.clone(), "org.freedesktop.NetworkManager", path.to_string(), iface);
    let Ok(root) = nm("/org/freedesktop/NetworkManager", "org.freedesktop.NetworkManager").await else { return net };
    net.wifi_on = root.get_property::<bool>("WirelessEnabled").await.unwrap_or(false);
    for dev in root.get_property::<Vec<OwnedObjectPath>>("Devices").await.unwrap_or_default() {
        if let Ok(d) = nm(dev.as_str(), "org.freedesktop.NetworkManager.Device").await {
            if d.get_property::<u32>("DeviceType").await.unwrap_or(0) == 2 {
                net.wifi = true;
            }
        }
    }
    net.online = root.get_property::<u32>("State").await.unwrap_or(0) >= 60;
    if let Ok(primary) = root.get_property::<OwnedObjectPath>("PrimaryConnection").await {
        if primary.as_str() != "/" {
            if let Ok(active) = nm(primary.as_str(), "org.freedesktop.NetworkManager.Connection.Active").await {
                net.name = active.get_property::<String>("Id").await.unwrap_or_default();
                net.kind = active.get_property::<String>("Type").await.unwrap_or_default();
            }
        }
    }
    net
}

async fn read_wifi() -> nm::Wifi {
    match crate::bus::system().await {
        Ok(conn) => nm::read(&conn).await,
        Err(_) => nm::Wifi::default(),
    }
}

async fn read_bt() -> bluez::Snapshot {
    match crate::bus::system().await {
        Ok(conn) => bluez::read(&conn).await,
        Err(_) => bluez::Snapshot::default(),
    }
}

impl Control {
    fn new(app: &Rc<App>) -> Self {
        let window = gtk::Window::new();
        window.init_layer_shell();
        window.set_layer(Layer::Overlay);
        window.set_namespace(Some("digit-control"));
        panel_corner::anchor(&window, panel_corner::selected(app, Panel::Control, ""));
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
            max_h: 600.0,
            screen: (1920.0, 1080.0),
            corner: panel_corner::selected(app, Panel::Control, ""),
            mount: Mount::Corner,
            on_close: None,
            margins: [0; 4],
            net: Net::default(),
            bt: bluez::Snapshot::default(),
            busy: Vec::new(),
            wifi: nm::Wifi::default(),
            expand: None,
            reveal: Tween::still(0.0),
            joining: None,
            pass: None,
            armed: None,
            scanning: None,
            scan_round: 0,
            watch_gen: 0,
            rise: Tween::still(0.0),
            fade: Tween::still(0.0),
            height: Tween::still(0.0),
            scroll: 0.0,
            glide: Glide::default(),
            hover: None,
            dragging: false,
            hits: Vec::new(),
            slider: (0.0, 0.0),
            region: (0, 0, 0, 0),
            sound: sound::Sound::default(),
            ticking: false,
            polling: false,
            scrim: None,
            power: None,
            stats: None,
            sampler: crate::sysmon::Sampler::default(),
            sampling: false,
        }));
        *canvas.imp().state.borrow_mut() = Rc::downgrade(&state);
        let control = Control(state);
        control.wire();
        control
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
            let me = Control(s);
            if me.key(key, mods) {
                return glib::Propagation::Stop;
            }
            if matches!(key, gdk::Key::Escape | gdk::Key::space) {
                me.close();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(keys);
        let drag = gtk::GestureDrag::new();
        let weak = Rc::downgrade(&self.0);
        drag.connect_drag_begin(move |_, x, y| {
            if let Some(s) = weak.upgrade() {
                let me = Control(s);
                match me.hit(x, y) {
                    Some(Hit::Slider) => {
                        me.0.borrow_mut().dragging = true;
                        me.slide(x);
                    }
                    Some(Hit::AppLevel(i)) => me.grab_app(i, x),
                    _ => {}
                }
            }
        });
        let weak = Rc::downgrade(&self.0);
        drag.connect_drag_update(move |g, dx, _| {
            if let (Some(s), Some((x, _))) = (weak.upgrade(), g.start_point()) {
                let me = Control(s);
                if me.0.borrow().dragging {
                    me.slide(x + dx);
                } else if me.sliding_app() {
                    me.slide_app(x + dx);
                }
            }
        });
        let weak = Rc::downgrade(&self.0);
        drag.connect_drag_end(move |g, dx, dy| {
            let Some(s) = weak.upgrade() else { return };
            let me = Control(s);
            let was = std::mem::replace(&mut me.0.borrow_mut().dragging, false);
            if was {
                return;
            }
            if me.sliding_app() {
                me.drop_app();
                return;
            }
            if dx.abs() > 6.0 || dy.abs() > 6.0 {
                return;
            }
            if let Some((x, y)) = g.start_point() {
                me.click(x, y);
            }
        });
        canvas.add_controller(drag);
        let menu = gtk::GestureClick::new();
        menu.set_button(gdk::BUTTON_SECONDARY);
        let weak = Rc::downgrade(&self.0);
        menu.connect_pressed(move |_, _, x, y| {
            let Some(s) = weak.upgrade() else { return };
            let me = Control(s);
            match me.hit(x, y) {
                Some(Hit::Tile(t @ (Tile::Network | Tile::Bluetooth))) | Some(Hit::Detail(detail::D::Expand(t))) => me.expand(t),
                _ => {}
            }
        });
        canvas.add_controller(menu);
        let motion = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(&self.0);
        motion.connect_motion(move |_, x, y| {
            if let Some(s) = weak.upgrade() {
                let me = Control(s);
                let h = me.hit(x, y);
                let changed = {
                    let mut st = me.0.borrow_mut();
                    let changed = st.hover != h;
                    st.hover = h;
                    changed
                };
                if changed {
                    me.0.borrow().canvas.queue_draw();
                }
            }
        });
        let weak = Rc::downgrade(&self.0);
        motion.connect_leave(move |_| {
            if let Some(s) = weak.upgrade() {
                if let Ok(mut st) = s.try_borrow_mut() {
                    st.hover = None;
                    st.canvas.queue_draw();
                }
            }
        });
        canvas.add_controller(motion);
        // "leave" mode: the panel folds shortly after the pointer leaves it.
        let away = gtk::EventControllerMotion::new();
        let gone = Rc::new(std::cell::Cell::new(0u64));
        let g = gone.clone();
        away.connect_enter(move |_, _, _| g.set(g.get() + 1));
        let weak = Rc::downgrade(&self.0);
        away.connect_leave(move |_| {
            let generation = gone.get() + 1;
            gone.set(generation);
            let (weak, gone) = (weak.clone(), gone.clone());
            glib::timeout_add_local_once(std::time::Duration::from_millis(crate::app::LEAVE_MS), move || {
                let Some(s) = weak.upgrade() else { return };
                let fold = gone.get() == generation && s.try_borrow().is_ok_and(|s| s.open && s.app.prefs.borrow().control_close == "leave");
                if fold {
                    Control(s).close();
                }
            });
        });
        canvas.add_controller(away);
        let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL | gtk::EventControllerScrollFlags::KINETIC);
        let weak = Rc::downgrade(&self.0);
        wheel.connect_scroll(move |c, _, dy| {
            if let Some(s) = weak.upgrade() {
                let smooth = kit::scroll::enabled();
                {
                    let mut st = s.borrow_mut();
                    let now = glib::monotonic_time();
                    if c.unit() == gdk::ScrollUnit::Wheel && smooth {
                        // A notch adds GTK's own wheel step to what is still to travel.
                        let step = st.max_h.powf(2.0 / 3.0);
                        st.glide.push(dy * step, WHEEL_TAU, now);
                    } else {
                        // Touchpads (or no smooth scrolling): follow the fingers exactly.
                        let step = if c.unit() == gdk::ScrollUnit::Wheel { st.max_h.powf(2.0 / 3.0) } else { 1.0 };
                        st.glide.stop();
                        st.scroll_by(dy * step);
                    }
                }
                Control(s).kick();
            }
            glib::Propagation::Stop
        });
        let weak = Rc::downgrade(&self.0);
        wheel.connect_decelerate(move |_, _, vy| {
            // Fingers lifted mid-swipe: coast on with the swipe's speed, slowing down.
            let Some(s) = weak.upgrade() else { return };
            if kit::scroll::enabled() {
                s.borrow_mut().glide.push(vy * FLING_TAU / 1e6, FLING_TAU, glib::monotonic_time());
                Control(s).kick();
            }
        });
        canvas.add_controller(wheel);
    }

    pub fn changed(&self) {
        let open = self.0.borrow().open;
        if open {
            self.reposition();
        }
    }

    pub fn reposition(&self) {
        self.0.borrow_mut().reposition();
        self.resize();
        self.kick();
    }

    pub fn is_open(&self) -> bool {
        self.0.borrow().open
    }

    fn toggle(&self) {
        let open = self.0.borrow().open;
        if open { self.close() } else { self.open() }
    }

    fn open(&self) {
        self.0.borrow_mut().mount = Mount::Corner;
        self.open_on(compositor::focused_output());
    }

    fn open_on(&self, output: Option<String>) {
        {
            let mut s = self.0.borrow_mut();
            let display = gdk::Display::default();
            let monitors: Vec<gdk::Monitor> = display.map(|d| d.monitors().iter::<gdk::Monitor>().filter_map(Result::ok).collect()).unwrap_or_default();
            let monitor = monitors.iter().find(|m| output.as_deref().is_some_and(|o| m.connector().as_deref() == Some(o))).or(monitors.first()).cloned();
            s.screen = monitor.as_ref().map(|m| (m.geometry().width() as f64, m.geometry().height() as f64)).unwrap_or((1920.0, 1080.0));
            if !s.window.is_visible() {
                s.window.set_monitor(monitor.as_ref());
            }
            s.reposition();
            s.open = true;
            s.scroll = 0.0;
            s.glide.stop();
            s.expand = None;
            s.pass = None;
            s.reveal.set(0.0);
            s.sound.open = false;
            s.sound.reveal.set(0.0);
            let now = glib::monotonic_time();
            s.rise.go(1.0, now, 420, motion::Curve::new(0.16, 1.0, 0.3, 1.0));
            s.fade.after(1.0, now, 60, 220, motion::STANDARD);
            s.window.set_keyboard_mode(KeyboardMode::Exclusive);
            if s.scrim.is_none() {
                let weak = Rc::downgrade(&self.0);
                s.scrim = Some(crate::paint::scrim(monitor.as_ref(), move || {
                    if let Some(s) = weak.upgrade() {
                        Control(s).close();
                    }
                }));
            }
        }
        self.resize();
        let window = self.0.borrow().window.clone();
        window.present();
        audio::ask(Ask::Read);
        // Read the sound devices and apps now, so the sound details open without a hitch.
        self.refresh_sound();
        self.poll();
        self.refresh();
        self.sample();
        self.kick();
    }

    fn sample(&self) {
        if std::mem::replace(&mut self.0.borrow_mut().sampling, true) {
            return;
        }
        let weak = Rc::downgrade(&self.0);
        glib::spawn_future_local(async move {
            let mut gpu: Option<f64> = None;
            let mut beat = 0u32;
            loop {
                let Some(s) = weak.upgrade() else { return };
                let (open, nvidia) = {
                    let mut st = s.borrow_mut();
                    if !st.open {
                        st.sampling = false;
                        return;
                    }
                    let mut stats = st.sampler.sample();
                    if stats.gpu.is_none() {
                        stats.gpu = gpu;
                    }
                    let changed = st.stats != Some(stats);
                    st.stats = Some(stats);
                    if changed && beat > 0 {
                        st.canvas.queue_draw();
                    }
                    (st.open, st.sampler.wants_nvidia())
                };
                drop(s);
                if !open {
                    return;
                }
                if nvidia && beat % 2 == 0 {
                    gpu = crate::sysmon::nvidia().await;
                }
                beat += 1;
                glib::timeout_future(std::time::Duration::from_millis(if beat == 1 { 400 } else { 2000 })).await;
            }
        });
    }

    fn poll(&self) {
        if audio::live() || std::mem::replace(&mut self.0.borrow_mut().polling, true) {
            return;
        }
        let weak = Rc::downgrade(&self.0);
        glib::timeout_add_local(std::time::Duration::from_secs(1), move || {
            let Some(s) = weak.upgrade() else { return glib::ControlFlow::Break };
            let mut st = s.borrow_mut();
            if !st.open || audio::live() {
                st.polling = false;
                return glib::ControlFlow::Break;
            }
            audio::ask(Ask::Heard);
            glib::ControlFlow::Continue
        });
    }

    fn close(&self) {
        {
            let mut s = self.0.borrow_mut();
            if !s.open {
                return;
            }
            s.open = false;
            s.pass = None;
            s.watch_gen += 1;
            let now = glib::monotonic_time();
            s.rise.go(0.0, now, 240, motion::Curve::new(0.4, 0.0, 0.9, 0.6));
            s.fade.go(0.0, now, 180, motion::STANDARD);
            s.window.set_keyboard_mode(KeyboardMode::None);
            if let Some(scrim) = s.scrim.take() {
                scrim.destroy();
            }
            if let Some(f) = s.on_close.take() {
                glib::idle_add_local_once(f);
            }
        }
        self.stop_scan();
        self.kick();
    }

    fn refresh(&self) {
        let weak = Rc::downgrade(&self.0);
        glib::spawn_future_local(async move {
            let (net, bt, wifi, power) = futures_util::join!(read_net(), read_bt(), read_wifi(), read_power());
            let Some(s) = weak.upgrade() else { return };
            {
                let mut st = s.borrow_mut();
                st.net = net;
                st.bt = bt;
                st.wifi = wifi;
                st.power = power;
            }
            Control(s).changed();
        });
    }

    fn resize(&self) {
        self.resize_with(260, motion::STANDARD);
    }

    fn resize_with(&self, ms: i64, curve: motion::Curve) {
        let mut s = self.0.borrow_mut();
        let target = s.content().min(s.max_h);
        let now = glib::monotonic_time();
        if s.height.target() < 1.0 {
            s.height.set(target);
        } else if (s.height.target() - target).abs() > 0.5 {
            s.height.go(target, now, ms, curve);
        }
        let limit = (s.content() - s.max_h).max(0.0);
        s.scroll = s.scroll.min(limit);

    }

    fn hit(&self, x: f64, y: f64) -> Option<Hit> {
        let s = self.0.borrow();
        if !s.placement(glib::monotonic_time()).contains(x, y) { return None; }
        s.hits.iter().rev().find(|(_, hx, hy, hw, hh)| x >= *hx && x <= hx + hw && y >= *hy && y <= hy + hh).map(|h| h.0)
    }

    fn slide(&self, x: f64) {
        let (x0, w) = self.0.borrow().slider;
        if w <= 0.0 {
            return;
        }
        let level = ((x - x0) / w).clamp(0.0, 1.0);
        let app = self.0.borrow().app.clone();
        let muted = app.volume.get().map(|v| v.1).unwrap_or(false);
        app.volume.set(Some((level, muted)));
        audio::ask(Ask::Level(level));
        app.request(crate::app::Peek::Volume { level, muted });
        self.0.borrow().canvas.queue_draw();
    }

    fn click(&self, x: f64, y: f64) {
        let app = self.0.borrow().app.clone();
        match self.hit(x, y) {
            Some(Hit::Tile(Tile::Quiet)) => {
                app.set_dnd(!app.dnd.get());
                app.request(crate::app::Peek::Mode { icon: "do_not_disturb_on", label: "Do not disturb", on: app.dnd.get() });
                self.0.borrow().canvas.queue_draw();
            }
            Some(Hit::Tile(Tile::Awake)) => {
                let on = app.awake.borrow().as_ref().is_some_and(|a| a.on());
                let surface = app.notches.borrow().first().and_then(|n| n.surface());
                if let Some(a) = app.awake.borrow().as_ref() {
                    a.set(surface.as_ref(), !on);
                }
                app.request(crate::app::Peek::Mode { icon: "coffee", label: "Keep awake", on: !on });
                self.0.borrow().canvas.queue_draw();
            }
            Some(Hit::Tile(Tile::Night)) => {
                let on = crate::night::active(&app);
                crate::night::force(&app, !on);
                app.request(crate::app::Peek::Mode { icon: "nightlight", label: "Night Light", on: !on });
                self.0.borrow().canvas.queue_draw();
            }
            Some(Hit::Tile(Tile::Power)) => {
                let Some(p) = self.0.borrow().power.clone() else { return };
                let at = p.list.iter().position(|x| *x == p.active).unwrap_or(0);
                let Some(next) = p.list.get((at + 1) % p.list.len().max(1)).cloned() else { return };
                if let Some(pw) = self.0.borrow_mut().power.as_mut() {
                    pw.active = next.clone();
                }
                self.0.borrow().canvas.queue_draw();
                glib::spawn_future_local(async move {
                    let Ok(conn) = crate::bus::system().await else { return };
                    let Ok(props) = zbus::fdo::PropertiesProxy::builder(&conn).destination(p.bus.clone()).and_then(|b| b.path(p.path.clone())).map(|b| b.cache_properties(zbus::proxy::CacheProperties::No)) else { return };
                    let Ok(props) = props.build().await else { return };
                    let Ok(iface) = zbus::names::InterfaceName::try_from(p.bus.clone()) else { return };
                    let _ = props.set(iface, "ActiveProfile", zbus::zvariant::Value::from(next.as_str())).await;
                });
            }
            Some(Hit::Tile(Tile::Network)) => {
                let net = self.0.borrow().net.clone();
                if net.wifi {
                    self.set_property("org.freedesktop.NetworkManager", "/org/freedesktop/NetworkManager", "org.freedesktop.NetworkManager", "WirelessEnabled", !net.wifi_on);
                }
            }
            Some(Hit::Tile(Tile::Bluetooth)) => {
                let adapter = self.0.borrow().bt.chosen().cloned();
                if let Some(a) = adapter {
                    self.set_property("org.bluez", &a.path, "org.bluez.Adapter1", "Powered", !a.powered);
                }
            }
            Some(Hit::Mute) => {
                let (level, muted) = app.volume.get().unwrap_or((0.0, false));
                app.volume.set(Some((level, !muted)));
                audio::ask(Ask::Mute(!muted));
                app.request(crate::app::Peek::Volume { level, muted: !muted });
                self.0.borrow().canvas.queue_draw();
            }
            Some(Hit::Clear) => {
                let ids: Vec<u32> = app.history.borrow().iter().map(|n| n.id).collect();
                app.history.borrow_mut().clear();
                if let Some(n) = app.notify.borrow().as_ref() {
                    for id in ids {
                        n.closed(id, 2);
                    }
                }
                self.changed();
            }
            Some(Hit::Detail(d)) => self.act(d),
            Some(h @ (Hit::SoundMore | Hit::Out(_) | Hit::In(_) | Hit::AppMute(_) | Hit::AppLevel(_))) => self.sound_act(h),
            Some(Hit::Close(i)) => self.forget(i, false),
            Some(Hit::Row(i)) => self.forget(i, true),
            Some(Hit::Slider) | None => {
                if !self.inside(x, y) {
                    self.close();
                }
            }
        }
    }

    fn forget(&self, i: usize, activate: bool) {
        let app = self.0.borrow().app.clone();
        let Some(notice) = app.history.borrow().get(i).cloned() else { return };
        app.history.borrow_mut().remove(i);
        if let Some(n) = app.notify.borrow().as_ref() {
            if activate && n.has_default(notice.id) {
                n.clicked(notice.id);
            } else {
                n.closed(notice.id, 2);
            }
        }
        if activate {
            self.close();
        } else {
            self.changed();
        }
    }

    fn inside(&self, x: f64, y: f64) -> bool {
        let s = self.0.borrow();
        s.placement(glib::monotonic_time()).contains(x, y)
    }

    fn set_property(&self, dest: &'static str, path: &str, iface: &'static str, name: &'static str, value: bool) {
        let path = path.to_string();
        let weak = Rc::downgrade(&self.0);
        glib::spawn_future_local(async move {
            if let Ok(conn) = crate::bus::system().await {
                if let Ok(p) = zbus::Proxy::new_owned(conn, dest, path, iface).await {
                    if let Err(e) = p.set_property(name, value).await {
                        eprintln!("digit: {iface}.{name}: {e}");
                    }
                }
            }
            if let Some(s) = weak.upgrade() {
                Control(s).refresh();
            }
        });
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

// Keep HEAD's height budget even for bottom corners, which have no top margin.
fn max_height(screen_h: f64) -> f64 {
    (screen_h - 38.0 - 2.0 * FILLET - 12.0).min(820.0)
}

impl State {
    fn reposition(&mut self) {
        match self.mount {
            Mount::Corner => {
                // The corner it has on the screen it opens on.
                let output = self.window.monitor().and_then(|m| m.connector()).map(|c| c.to_string()).unwrap_or_default();
                self.corner = panel_corner::selected(&self.app, Panel::Control, &output);
                self.margins = panel_corner::position(&self.window, self.corner);
                self.max_h = max_height(self.screen.1);
            }
            Mount::Hang { left, top } => {
                self.corner = if left { Corner::TopLeft } else { Corner::TopRight };
                panel_corner::anchor(&self.window, self.corner);
                let top = (top - FILLET).max(0.0);
                self.margins = [top as i32, 0, 0, 0];
                self.max_h = (self.screen.1 - top - 3.0 * FILLET - 12.0).clamp(200.0, 820.0);
            }
            Mount::Card { right, top } => {
                self.corner = Corner::TopLeft;
                panel_corner::anchor(&self.window, self.corner);
                let left = (right - W).clamp(CARD_AIR, (self.screen.0 - W - CARD_AIR).max(CARD_AIR));
                // The surface starts a drop higher, so the card falls in without being cut off.
                self.margins = [(top - CARD_DROP).max(0.0).round() as i32, left.round() as i32, 0, 0];
                self.max_h = (self.screen.1 - top - 2.0 * CARD_AIR).clamp(200.0, 820.0);
            }
        }
        if self.mount != Mount::Corner {
            for (edge, margin) in [Edge::Top, Edge::Left, Edge::Right, Edge::Bottom].into_iter().zip(self.margins) {
                self.window.set_margin(edge, margin);
            }
        }
        let (w, h) = self.wanted();
        let (w, h) = (w as i32, h as i32);
        self.window.set_default_size(w, h);
        self.canvas.set_size_request(w, h);
        self.hits.clear();
    }

    /// Scrolls by `dy`, within the content.
    fn scroll_by(&mut self, dy: f64) {
        let limit = (self.content() - self.max_h).max(0.0);
        self.scroll = (self.scroll + dy).clamp(0.0, limit);
    }

    /// The surface each mount needs: room for the fillets it draws, and no more.
    fn wanted(&self) -> (f64, f64) {
        match self.mount {
            Mount::Corner => (W + FILLET, self.max_h + FILLET),
            Mount::Hang { .. } => (W, self.max_h + 2.0 * FILLET),
            Mount::Card { .. } => (W, self.max_h + CARD_DROP),
        }
    }

    fn surface_size(&self) -> (f64, f64) {
        let (w, h) = (self.canvas.width(), self.canvas.height());
        let wanted = self.wanted();
        (if w > 1 { w as f64 } else { wanted.0 }, if h > 1 { h as f64 } else { wanted.1 })
    }

    fn placement(&self, now: i64) -> panel_corner::Rect {
        self.placement_at(self.rise.value(now), self.height.value(now).max(1.0))
    }

    fn placement_at(&self, rise: f64, h: f64) -> panel_corner::Rect {
        let surface = self.surface_size();
        match self.mount {
            Mount::Corner => panel_corner::placement(self.corner, surface, (W, h), rise, FILLET),
            Mount::Hang { left, .. } => {
                // Slides in from the side edge it hangs on, as the calendar does.
                let mut r = panel_corner::placement(self.corner, surface, (W, h), 1.0, FILLET);
                r.y += FILLET;
                r.x += ((1.0 - rise) * W).round() * if left { -1.0 } else { 1.0 };
                r
            }
            Mount::Card { .. } => panel_corner::Rect { x: 0.0, y: (rise.clamp(0.0, 1.0) * CARD_DROP).round(), w: W, h },
        }
    }

    /// The input region: the panel and the fillets it draws.
    fn envelope(&self, now: i64) -> panel_corner::Rect {
        let r = self.placement(now);
        match self.mount {
            Mount::Corner => panel_corner::envelope(self.corner, r, self.surface_size(), FILLET),
            Mount::Hang { .. } => {
                let y = (r.y - FILLET).max(0.0);
                panel_corner::Rect { x: r.x, y, w: r.w, h: (r.y + r.h + FILLET - y).max(0.0) }
            }
            Mount::Card { .. } => r,
        }
    }

    fn bt_on(&self) -> bool {
        self.bt.chosen().is_some_and(|a| a.powered)
    }

    fn tiles(&self) -> Vec<Tile> {
        let mut t = vec![Tile::Network];
        if !self.bt.adapters.is_empty() {
            t.push(Tile::Bluetooth);
        }
        t.push(Tile::Quiet);
        if self.app.awake.borrow().is_some() {
            t.push(Tile::Awake);
        }
        t.push(Tile::Night);
        if self.power.as_ref().is_some_and(|p| p.list.len() > 1) {
            t.push(Tile::Power);
        }
        t
    }

    fn content(&self) -> f64 {
        let rows = self.tiles().len().div_ceil(2) as f64;
        let tiles = rows * TILE_H + (rows - 1.0).max(0.0) * TILE_GAP;
        let n = self.app.history.borrow().len();
        let list = if n == 0 { EMPTY } else { n as f64 * ROW };
        let detail = self.expand.map(|t| self.detail_height(t) * self.reveal.target()).unwrap_or(0.0);
        let cells = crate::batteries::list(&self.app).len();
        let batteries = if cells == 0 { 0.0 } else { HEAD + cells.div_ceil(PER_ROW) as f64 * CELL + 6.0 };
        let system = HEAD + CELL + 6.0;
        let weather = if self.app.weather.borrow().is_some() { 58.0 } else { 0.0 };
        let sound = self.sound_height() * self.sound.reveal.target();
        PAD + weather + tiles + detail + 14.0 + batteries + system + SOUND + sound + 16.0 + HEAD + list + PAD
    }

    fn step(&mut self, now: i64) -> bool {
        if self.reveal.target() == 0.0 && self.reveal.settled(now) {
            self.expand = None;
        }
        let r = if self.open {
            self.envelope(now).region()
        } else {
            cairo::RectangleInt::new(0, 0, 0, 0)
        };
        let rect = (r.x(), r.y(), r.width(), r.height());
        if rect != self.region && let Some(surface) = self.window.surface() {
            surface.set_input_region(Some(&cairo::Region::create_rectangle(&r)));
            self.region = rect;
        }
        if self.glide.left != 0.0 {
            let part = self.glide.take(now);
            let before = self.scroll;
            self.scroll_by(part);
            // Ran into the top or the bottom: nothing left to travel.
            if (self.scroll - before).abs() < 0.01 && part.abs() >= 0.01 {
                self.glide.stop();
            }
        }
        let moving = [&self.rise, &self.fade, &self.height, &self.reveal, &self.sound.reveal].iter().any(|t| !t.settled(now)) || self.glide.left != 0.0 || self.app.palette_moving(now);
        if !moving && !self.open && self.window.is_visible() {
            let window = self.window.clone();
            glib::idle_add_local_once(move || window.set_visible(false));
            self.region = (0, 0, 0, 0);
            self.hits.clear();
            draw::forget(&self.canvas);
            App::trim();
        }
        moving
    }

    fn tile_look(&self, tile: Tile) -> (&'static str, String, String, bool) {
        match tile {
            Tile::Network => {
                let n = &self.net;
                let icon = if n.kind.contains("wireless") { "wifi" } else if n.online { "lan" } else if n.wifi && n.wifi_on { "wifi_find" } else { "wifi_off" };
                let title = if n.wifi { "Wi-Fi" } else { "Network" };
                let state = if !n.name.is_empty() { n.name.clone() } else if n.wifi && !n.wifi_on { "Off".into() } else { "Not connected".into() };
                (icon, title.into(), state, if n.wifi { n.wifi_on } else { n.online })
            }
            Tile::Bluetooth => {
                let on = self.bt_on();
                let connected: Vec<String> = self.bt.devices.iter().filter(|d| d.connected).map(|d| d.name.clone()).collect();
                let state = if !on { "Off".into() } else if connected.is_empty() { "On".into() } else { connected.join(", ") };
                (if on { "bluetooth" } else { "bluetooth_disabled" }, "Bluetooth".into(), state, on)
            }
            Tile::Quiet => {
                let on = self.app.dnd.get();
                ("do_not_disturb_on", "Do not disturb".into(), if on { "On".into() } else { "Off".into() }, on)
            }
            Tile::Awake => {
                let on = self.app.awake.borrow().as_ref().is_some_and(|a| a.on());
                ("coffee", "Keep awake".into(), if on { "On".into() } else { "Off".into() }, on)
            }
            Tile::Night => {
                let on = crate::night::active(&self.app);
                let prefs = self.app.prefs.borrow();
                let state = match (on, prefs.night_light.as_str()) {
                    (true, "always") => "On".to_string(),
                    (true, "off") => "On until tomorrow".to_string(),
                    (true, _) => format!("On until {}", prefs.night_to),
                    (false, "schedule") => format!("From {}", prefs.night_from),
                    (false, "sunset") => "From sunset".to_string(),
                    _ => "Off".to_string(),
                };
                ("nightlight", "Night Light".into(), state, on)
            }
            Tile::Power => {
                let p = self.power.clone().unwrap_or_default();
                let icon = match p.active.as_str() {
                    "performance" => "speed",
                    "power-saver" => "eco",
                    _ => "balance",
                };
                (icon, "Power Mode".into(), profile_name(&p.active).into(), p.active != "balanced")
            }
        }
    }

    fn draw(&mut self, s: &gtk::Snapshot, now: i64) {
        let rise = self.rise.value(now);
        if rise <= 0.001 {
            return;
        }
        let pal = self.app.palette_at(now);
        let cfg = self.app.cfg.clone();
        let canvas = self.canvas.clone();
        let h = self.height.value(now).max(1.0);
        let placed = self.placement(now);
        let shift = placed.y - self.placement_at(1.0, h).y;
        let rect = panel_corner::Rect { x: placed.x, y: placed.y - shift, w: W, h };
        let right = rect.x + W;
        let bottom = rect.y + h;
        s.save();
        s.translate(&graphene::Point::new(0.0, shift as f32));
        let radius = RADIUS * self.app.round();
        let card = matches!(self.mount, Mount::Card { .. });
        if card {
            s.push_opacity(rise.clamp(0.0, 1.0));
        }
        match self.mount {
            Mount::Corner => {
                panel_corner::slab(s, self.corner, rect, radius, FILLET, pal.body);
                s.push_rounded_clip(&panel_corner::clip(self.corner, rect, radius, FILLET));
            }
            Mount::Hang { left, .. } => {
                let bar = self.app.wide();
                panel_corner::hang(s, rect, left, bar, radius, FILLET, pal.body);
                s.push_rounded_clip(&panel_corner::hang_clip(rect, left, bar, radius));
            }
            Mount::Card { .. } => {
                let body = draw::round(rect.x, rect.y, W, h, radius);
                fill(s, &body, pal.body);
                s.push_rounded_clip(&body);
            }
        }
        s.push_opacity(self.fade.value(now).clamp(0.0, 1.0));
        let left = right - W;
        let x0 = left + PAD;
        let inner = W - 2.0 * PAD;
        // Whole pixels, so text stays crisp while it scrolls.
        let mut y = bottom - h + PAD - self.scroll.round();
        let mut hits: Vec<(Hit, f64, f64, f64, f64)> = Vec::new();
        let weather = self.app.weather.borrow().clone();
        if let Some(wx) = weather {
            let mid = y + 22.0;
            draw::icon(s, &canvas, &cfg, crate::weather::glyph(wx.code, wx.day), 30.0, true, pal.ink_main, x0 + 20.0, mid);
            let temp = format!("{}°", wx.temp.round());
            let tw = draw::width(&canvas, &cfg, &temp, 28.0, Face::Number(500)).ceil();
            draw::text(s, &canvas, &cfg, &temp, 28.0, Face::Number(500), pal.ink_main, x0 + 44.0, mid, tw + 2.0, pango::Alignment::Left);
            let tx = x0 + 52.0 + tw;
            draw::text(s, &canvas, &cfg, crate::weather::describe(wx.code), 14.0, Face::Text(560), pal.ink_main, tx, mid - 9.0, inner - (tx - x0), pango::Alignment::Left);
            let sub = format!("{}  ·  H {}°  L {}°", wx.place, wx.hi.round(), wx.lo.round());
            draw::text(s, &canvas, &cfg, &sub, 12.5, Face::Text(420), pal.ink_meta, tx, mid + 10.0, inner - (tx - x0), pango::Alignment::Left);
            y += 58.0;
        }
        let tiles = self.tiles();
        let tile_w = (inner - TILE_GAP) / 2.0;
        let opened = match self.expand {
            Some(t) => self.draw_detail(s, now, t, x0, y + TILE_H, inner, shift, &mut hits),
            None => 0.0,
        };
        for (i, tile) in tiles.iter().enumerate() {
            let tx = x0 + (i % 2) as f64 * (tile_w + TILE_GAP);
            let ty = y + (i / 2) as f64 * (TILE_H + TILE_GAP) + if i >= 2 { opened } else { 0.0 };
            let (icon, title, state, on) = self.tile_look(*tile);
            let hovered = self.hover == Some(Hit::Tile(*tile));
            let base = if on { pal.accent } else { pal.tile(if hovered { 0.11 } else { 0.06 }) };
            fill(s, &rounded(tx, ty, tile_w, TILE_H, (18.0 * self.app.round()).min(TILE_H / 2.0)), base);
            let (ink, sub) = if on { (pal.on_accent, pal.on_accent.mix(pal.accent, 0.72)) } else { (pal.ink_main, pal.ink_meta) };
            draw::icon(s, &canvas, &cfg, icon, 22.0, on, ink, tx + 30.0, ty + TILE_H / 2.0);
            let more = matches!(tile, Tile::Network | Tile::Bluetooth);
            let text_w = tile_w - 62.0 - if more { 22.0 } else { 0.0 };
            draw::text(s, &canvas, &cfg, &title, 14.0, Face::Text(560), ink, tx + 54.0, ty + 24.0, text_w, pango::Alignment::Left);
            draw::text(s, &canvas, &cfg, &state, 12.5, Face::Text(420), sub, tx + 54.0, ty + 42.0, text_w, pango::Alignment::Left);
            hits.push((Hit::Tile(*tile), tx, ty + shift, tile_w, TILE_H));
            if more {
                let (cx, cy) = (tx + tile_w - 20.0, ty + TILE_H / 2.0);
                let hot = self.hover == Some(Hit::Detail(detail::D::Expand(*tile)));
                if hot {
                    fill(s, &rounded(cx - 13.0, cy - 13.0, 26.0, 26.0, 13.0), if on { pal.on_accent.alpha(0.18) } else { pal.ink_main.alpha(0.10) });
                }
                let open = self.expand == Some(*tile) && self.reveal.target() > 0.5;
                draw::icon(s, &canvas, &cfg, if open { "expand_less" } else { "expand_more" }, 18.0, false, sub, cx, cy);
                hits.push((Hit::Detail(detail::D::Expand(*tile)), cx - 15.0, cy - 15.0 + shift, 30.0, 30.0));
            }
        }
        let rows = tiles.len().div_ceil(2) as f64;
        y += rows * TILE_H + (rows - 1.0).max(0.0) * TILE_GAP + opened + 14.0;
        {
            draw::text(s, &canvas, &cfg, "System", 14.0, Face::Text(600), pal.ink_main, x0, y + HEAD / 2.0, 220.0, pango::Alignment::Left);
            y += HEAD;
            let cw = inner / PER_ROW as f64;
            let st = self.stats.unwrap_or_default();
            let hot = |v: f64, warn: f64| if v >= warn { RED } else { pal.ink_main };
            let cells: [(&str, &str, f64, String, Rgb); 4] = [
                ("memory", "CPU", st.cpu, format!("{}%", (st.cpu * 100.0).round()), hot(st.cpu, 0.9)),
                ("memory_alt", "Memory", st.mem, if st.total > 0.0 { format!("{:.1} GB", st.used) } else { "–".into() }, hot(st.mem, 0.9)),
                ("developer_board", "GPU", st.gpu.unwrap_or(0.0), st.gpu.map(|g| format!("{}%", (g * 100.0).round())).unwrap_or_else(|| "–".into()), hot(st.gpu.unwrap_or(0.0), 0.95)),
                ("device_thermostat", "Temp", (st.temp.unwrap_or(0.0) / 100.0).clamp(0.0, 1.0), st.temp.map(|t| format!("{}°", t.round())).unwrap_or_else(|| "–".into()), hot(st.temp.unwrap_or(0.0) / 100.0, 0.85)),
            ];
            for (i, (glyph, name, frac, value, tint)) in cells.iter().enumerate() {
                let cx = x0 + cw * i as f64 + cw / 2.0;
                let cy = y + 28.0;
                fill(s, &rounded(cx - 26.0, cy - 26.0, 52.0, 52.0, 26.0), pal.tile(0.05));
                ring(s, cx, cy, 22.0, *frac, 3.5, pal.ink_main.alpha(0.12), *tint);
                draw::icon(s, &canvas, &cfg, glyph, 19.0, false, pal.ink_main, cx, cy);
                draw::text(s, &canvas, &cfg, value, 13.0, Face::Number(560), pal.ink_main, cx - cw / 2.0 + 3.0, cy + 39.0, cw - 6.0, pango::Alignment::Center);
                draw::text(s, &canvas, &cfg, name, 11.5, Face::Text(450), pal.ink_meta, cx - cw / 2.0 + 3.0, cy + 56.0, cw - 6.0, pango::Alignment::Center);
            }
            y += CELL + 6.0;
        }
        let gadgets = crate::batteries::list(&self.app);
        if !gadgets.is_empty() {
            draw::text(s, &canvas, &cfg, "Batteries", 14.0, Face::Text(600), pal.ink_main, x0, y + HEAD / 2.0, 220.0, pango::Alignment::Left);
            y += HEAD;
            let cw = inner / PER_ROW as f64;
            for (i, g) in gadgets.iter().enumerate() {
                let cx = x0 + cw * (i % PER_ROW) as f64 + cw / 2.0;
                let cy = y + (i / PER_ROW) as f64 * CELL + 28.0;
                crate::batteries::cell(s, &canvas, &cfg, &pal, g, cx, cy, cw, true, true);
            }
            y += gadgets.len().div_ceil(PER_ROW) as f64 * CELL + 6.0;
        }
        let (level, muted) = self.app.volume.get().unwrap_or((0.0, false));
        let mid = y + SOUND / 2.0;
        let mute_hover = self.hover == Some(Hit::Mute);
        fill(s, &rounded(x0, mid - 18.0, 36.0, 36.0, 18.0), pal.tile(if mute_hover { 0.12 } else { 0.06 }));
        let speaker = if muted { "volume_off" } else if level < 0.01 { "volume_mute" } else if level < 0.5 { "volume_down" } else { "volume_up" };
        draw::icon(s, &canvas, &cfg, speaker, 20.0, true, if muted { pal.ink_meta } else { pal.ink_main }, x0 + 18.0, mid);
        hits.push((Hit::Mute, x0, mid - 18.0 + shift, 36.0, 36.0));
        let sx = x0 + 50.0;
        let sw = inner - 50.0 - 48.0 - 30.0;
        let shown = level.clamp(0.0, 1.0);
        let track = rounded(sx, mid - 3.0, sw, 6.0, 3.0);
        fill(s, &track, pal.ink_main.alpha(0.15));
        let fill_ink = if muted { pal.ink_meta } else { pal.accent };
        fill(s, &rounded(sx, mid - 3.0, (sw * shown).max(6.0), 6.0, 3.0), fill_ink);
        let knob_r = if self.dragging || self.hover == Some(Hit::Slider) { 9.0 } else { 7.0 };
        fill(s, &rounded(sx + sw * shown - knob_r, mid - knob_r, 2.0 * knob_r, 2.0 * knob_r, knob_r), fill_ink);
        draw::text(s, &canvas, &cfg, &format!("{}%", (level * 100.0).round()), 13.0, Face::Number(500), pal.ink_meta, sx + sw + 8.0, mid, 40.0, pango::Alignment::Right);
        hits.push((Hit::Slider, sx - 10.0, mid - 14.0 + shift, sw + 20.0, 28.0));
        self.slider = (sx, sw);
        self.draw_sound_more(s, x0 + inner - 15.0, mid, shift, &mut hits);
        y += SOUND;
        y += self.draw_sound(s, now, x0, y, inner, shift, &mut hits);
        y += 16.0;
        let history = self.app.history.borrow().clone();
        draw::text(s, &canvas, &cfg, "Notifications", 14.0, Face::Text(600), pal.ink_main, x0, y + HEAD / 2.0, 200.0, pango::Alignment::Left);
        if !history.is_empty() {
            let cw = draw::width(&canvas, &cfg, "Clear", 13.0, Face::Text(500)) + 20.0;
            let cx = x0 + inner - cw;
            if self.hover == Some(Hit::Clear) {
                fill(s, &rounded(cx, y + 4.0, cw, HEAD - 8.0, (HEAD - 8.0) / 2.0), pal.tile(0.08));
            }
            draw::text(s, &canvas, &cfg, "Clear", 13.0, Face::Text(500), pal.ink_meta, cx + 10.0, y + HEAD / 2.0, cw - 20.0, pango::Alignment::Left);
            hits.push((Hit::Clear, cx, y + 4.0 + shift, cw, HEAD - 8.0));
        }
        y += HEAD;
        if history.is_empty() {
            draw::text(s, &canvas, &cfg, "No notifications", 13.5, Face::Text(420), pal.ink_faint, x0, y + EMPTY / 2.0, inner, pango::Alignment::Center);
        }
        for (i, n) in history.iter().enumerate() {
            let ry = y + i as f64 * ROW;
            if ry > bottom || ry + ROW < bottom - h {
                continue;
            }
            let hovered = matches!(self.hover, Some(Hit::Row(j)) | Some(Hit::Close(j)) if j == i);
            if hovered {
                fill(s, &rounded(x0 - 6.0, ry + 2.0, inner + 12.0, ROW - 4.0, 14.0), pal.tile(0.07));
            }
            let icon_box = rounded(x0 + 2.0, ry + ROW / 2.0 - 16.0, 32.0, 32.0, 9.0);
            match &n.icon {
                Some(p) => {
                    s.push_rounded_clip(&icon_box);
                    s.save();
                    s.translate(&graphene::Point::new(icon_box.bounds().x(), icon_box.bounds().y()));
                    p.snapshot(s, 32.0, 32.0);
                    s.restore();
                    s.pop();
                }
                None => {
                    fill(s, &icon_box, pal.raised);
                    draw::icon(s, &canvas, &cfg, "notifications", 18.0, true, pal.ink_meta, x0 + 18.0, ry + ROW / 2.0);
                }
            }
            let tx = x0 + 46.0;
            let tw = inner - 46.0 - 44.0;
            let line2 = if n.body.is_empty() { n.app.clone() } else { n.body.clone() };
            draw::text(s, &canvas, &cfg, &n.summary, 14.0, Face::Text(560), pal.ink_main, tx, ry + 22.0, tw, pango::Alignment::Left);
            draw::text(s, &canvas, &cfg, &line2, 12.5, Face::Text(420), pal.ink_meta, tx, ry + 42.0, tw, pango::Alignment::Left);
            let rx = x0 + inner - 36.0;
            if hovered {
                let close_hover = self.hover == Some(Hit::Close(i));
                fill(s, &rounded(rx + 4.0, ry + ROW / 2.0 - 14.0, 28.0, 28.0, 14.0), pal.tile(if close_hover { 0.14 } else { 0.08 }));
                draw::icon(s, &canvas, &cfg, "close", 16.0, false, pal.ink_sec, rx + 18.0, ry + ROW / 2.0);
            } else {
                draw::text(s, &canvas, &cfg, &ago(n.at), 12.0, Face::Number(450), pal.ink_faint, rx - 4.0, ry + 22.0, 40.0, pango::Alignment::Right);
            }
            hits.push((Hit::Row(i), x0 - 6.0, ry + 2.0 + shift, inner + 12.0, ROW - 4.0));
            hits.push((Hit::Close(i), rx + 4.0, ry + ROW / 2.0 - 14.0 + shift, 28.0, 28.0));
        }
        s.pop();
        s.pop();
        if card {
            s.pop();
        }
        s.restore();
        let floor = bottom - h + shift;
        self.hits = hits.into_iter().filter(|(_, _, hy, _, hh)| hy + hh > floor && *hy < bottom + shift).collect();
        panel_corner::screen_mask(s, &self.app, self.corner, self.surface_size(), self.margins);
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
        const NAME: &'static str = "DigitControl";
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_corner_keeps_heads_unscaled_size_and_height_limit() {
        assert_eq!(W + FILLET, 394.0);
        for (screen_h, expected_max, expected_surface_h) in [(1080.0, 820.0, 834.0), (894.0, 816.0, 830.0), (720.0, 642.0, 656.0), (600.0, 522.0, 536.0)] {
            let max_h = max_height(screen_h);
            assert_eq!(max_h, expected_max);
            let surface = (W + FILLET, max_h + FILLET);
            assert_eq!(surface, (394.0, expected_surface_h));
            assert_eq!(panel_corner::margins(Corner::BottomRight), [0; 4]);
            // HEAD drew at right = W + FILLET, bottom = max_h + FILLET.
            assert_eq!(panel_corner::placement(Corner::BottomRight, surface, (W, max_h), 1.0, FILLET),
                panel_corner::Rect { x: FILLET, y: FILLET, w: 380.0, h: expected_max });
            for corner in Corner::ALL {
                let rect = panel_corner::placement(corner, surface, (W, max_h), 1.0, FILLET);
                assert_eq!(rect.x, if corner.is_left() { 0.0 } else { FILLET });
                assert_eq!(rect.y, if corner.is_top() { 0.0 } else { FILLET });
            }
        }
    }
}
