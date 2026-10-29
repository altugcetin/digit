use crate::app::App;
use crate::audio::{self, Ask};
use crate::compositor;
use crate::draw::{self, Face};
use gtk::subclass::prelude::*;
use gtk::{cairo, gdk, glib, graphene, gsk, pango, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kit::motion::{self, Tween};
use kit::palette::Rgb;
use kit::shape;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

const W: f64 = 380.0;
const FILLET: f64 = 12.0;
const RADIUS: f64 = 26.0;
const PAD: f64 = 16.0;
const TILE_H: f64 = 64.0;
const TILE_GAP: f64 = 10.0;
const SOUND: f64 = 44.0;
const HEAD: f64 = 34.0;
const ROW: f64 = 64.0;
const EMPTY: f64 = 56.0;

#[derive(Clone, Copy, PartialEq)]
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

fn ring(s: &gtk::Snapshot, cx: f64, cy: f64, r: f64, frac: f64, width: f32, track: Rgb, tint: Rgb) {
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
    Device(usize),
}

#[derive(Clone, Default)]
struct Net {
    wifi: bool,
    wifi_on: bool,
    name: String,
    kind: String,
    online: bool,
}

#[derive(Clone)]
struct Device {
    path: String,
    name: String,
    icon: String,
    connected: bool,
    battery: Option<u8>,
}

#[derive(Clone, Default)]
struct Blue {
    adapter: Option<OwnedObjectPath>,
    on: bool,
    connected: Vec<String>,
    devices: Vec<Device>,
    busy: Vec<String>,
}

const DEVICE: f64 = 52.0;
const GREEN: Rgb = Rgb::new(0.19, 0.82, 0.35);
const RED: Rgb = Rgb::new(1.0, 0.27, 0.23);
const CELL: f64 = 96.0;
const PER_ROW: usize = 4;

fn device_symbol(icon: &str) -> &'static str {
    match icon {
        i if i.contains("headset") || i.contains("headphone") => "headphones",
        i if i.contains("audio") || i.contains("speaker") => "speaker",
        i if i.contains("mouse") => "mouse",
        i if i.contains("keyboard") => "keyboard",
        i if i.contains("gaming") || i.contains("joystick") => "sports_esports",
        i if i.contains("phone") => "smartphone",
        i if i.contains("computer") => "computer",
        i if i.contains("watch") => "watch",
        _ => "bluetooth",
    }
}

struct State {
    app: Rc<App>,
    window: gtk::Window,
    canvas: Canvas,
    open: bool,
    max_h: f64,
    net: Net,
    blue: Blue,
    rise: Tween,
    fade: Tween,
    height: Tween,
    scroll: f64,
    hover: Option<Hit>,
    dragging: bool,
    hits: Vec<(Hit, f64, f64, f64, f64)>,
    slider: (f64, f64),
    region: (i32, i32, i32, i32),
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
        control.open_on(output);
    }
}

pub fn corner(app: &Rc<App>, monitor: &gdk::Monitor) -> gtk::Window {
    let window = gtk::Window::new();
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace(Some("digit-corner"));
    window.set_anchor(Edge::Bottom, true);
    window.set_anchor(Edge::Right, true);
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(KeyboardMode::None);
    window.set_monitor(Some(monitor));
    window.add_css_class("digit");
    window.set_default_size(2, 2);
    let paint = crate::paint::Paint::new(2, 2);
    paint.present();
    window.set_child(Some(&paint));
    let motion = gtk::EventControllerMotion::new();
    let weak = Rc::downgrade(app);
    let output = monitor.connector().map(|c| c.to_string());
    motion.connect_enter(move |_, _, _| {
        if let Some(app) = weak.upgrade() {
            if app.prefs.borrow().control_corner {
                show_on(&app, output.clone());
            }
        }
    });
    paint.add_controller(motion);
    window.present();
    window
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

async fn read_blue() -> Blue {
    let mut blue = Blue::default();
    let Ok(conn) = crate::bus::system().await else { return blue };
    let Ok(om) = zbus::fdo::ObjectManagerProxy::builder(&conn).destination("org.bluez").and_then(|b| b.path("/")) else { return blue };
    let Ok(om) = om.build().await else { return blue };
    let Ok(objects) = om.get_managed_objects().await else { return blue };
    let flag = |props: &HashMap<String, OwnedValue>, key: &str| props.get(key).and_then(|v| bool::try_from(v.try_clone().ok()?).ok()).unwrap_or(false);
    for (path, ifaces) in &objects {
        for (iface, props) in ifaces {
            match iface.as_str() {
                "org.bluez.Adapter1" if blue.adapter.is_none() => {
                    blue.adapter = Some(path.clone());
                    blue.on = flag(props, "Powered");
                }
                "org.bluez.Device1" if flag(props, "Paired") || flag(props, "Connected") => {
                    let text = |key: &str| props.get(key).and_then(|v| String::try_from(v.try_clone().ok()?).ok());
                    let name = text("Alias").or_else(|| text("Name")).unwrap_or_else(|| "Device".into());
                    let connected = flag(props, "Connected");
                    if connected {
                        blue.connected.push(name.clone());
                    }
                    let battery = ifaces.get("org.bluez.Battery1").and_then(|b| b.get("Percentage")).and_then(|v| u8::try_from(v.try_clone().ok()?).ok());
                    blue.devices.push(Device { path: path.to_string(), name, icon: text("Icon").unwrap_or_default(), connected, battery });
                }
                _ => {}
            }
        }
    }
    blue.devices.sort_by(|a, b| b.connected.cmp(&a.connected).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    blue
}

impl Control {
    fn new(app: &Rc<App>) -> Self {
        let window = gtk::Window::new();
        window.init_layer_shell();
        window.set_layer(Layer::Overlay);
        window.set_namespace(Some("digit-control"));
        window.set_anchor(Edge::Bottom, true);
        window.set_anchor(Edge::Right, true);
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
            net: Net::default(),
            blue: Blue::default(),
            rise: Tween::still(0.0),
            fade: Tween::still(0.0),
            height: Tween::still(0.0),
            scroll: 0.0,
            hover: None,
            dragging: false,
            hits: Vec::new(),
            slider: (0.0, 0.0),
            region: (0, 0, 0, 0),
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
        keys.connect_key_pressed(move |_, key, _, _| {
            if matches!(key, gdk::Key::Escape | gdk::Key::space) {
                if let Some(s) = weak.upgrade() {
                    Control(s).close();
                }
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
                if me.hit(x, y) == Some(Hit::Slider) {
                    me.0.borrow_mut().dragging = true;
                    me.slide(x);
                }
            }
        });
        let weak = Rc::downgrade(&self.0);
        drag.connect_drag_update(move |g, dx, _| {
            if let (Some(s), Some((x, _))) = (weak.upgrade(), g.start_point()) {
                let me = Control(s);
                if me.0.borrow().dragging {
                    me.slide(x + dx);
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
            if dx.abs() > 6.0 || dy.abs() > 6.0 {
                return;
            }
            if let Some((x, y)) = g.start_point() {
                me.click(x, y);
            }
        });
        canvas.add_controller(drag);
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
        let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        let weak = Rc::downgrade(&self.0);
        wheel.connect_scroll(move |_, _, dy| {
            if let Some(s) = weak.upgrade() {
                let mut st = s.borrow_mut();
                let limit = (st.content() - st.max_h).max(0.0);
                st.scroll = (st.scroll + dy * 48.0).clamp(0.0, limit);
                st.canvas.queue_draw();
            }
            glib::Propagation::Stop
        });
        canvas.add_controller(wheel);
        let weak = Rc::downgrade(&self.0);
        window.connect_is_active_notify(move |w| {
            if !w.is_active() {
                if let Some(s) = weak.upgrade() {
                    let Ok(open) = s.try_borrow().map(|s| s.open) else { return };
                    if open {
                        Control(s).close();
                    }
                }
            }
        });
    }

    pub fn changed(&self) {
        let open = self.0.borrow().open;
        if open {
            self.resize();
            self.kick();
        }
    }

    fn toggle(&self) {
        let open = self.0.borrow().open;
        if open { self.close() } else { self.open() }
    }

    fn open(&self) {
        self.open_on(compositor::focused_output());
    }

    fn open_on(&self, output: Option<String>) {
        {
            let mut s = self.0.borrow_mut();
            let display = gdk::Display::default();
            let monitors: Vec<gdk::Monitor> = display.map(|d| d.monitors().iter::<gdk::Monitor>().filter_map(Result::ok).collect()).unwrap_or_default();
            let monitor = monitors.iter().find(|m| output.as_deref().is_some_and(|o| m.connector().as_deref() == Some(o))).or(monitors.first()).cloned();
            let screen_h = monitor.as_ref().map(|m| m.geometry().height() as f64).unwrap_or(1080.0);
            s.max_h = (screen_h - 38.0 - 2.0 * FILLET - 12.0).min(820.0);
            let (w, h) = ((W + FILLET) as i32, (s.max_h + FILLET) as i32);
            s.window.set_default_size(w, h);
            s.canvas.set_size_request(w, h);
            if !s.window.is_visible() {
                s.window.set_monitor(monitor.as_ref());
            }
            s.open = true;
            s.scroll = 0.0;
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
            let now = glib::monotonic_time();
            s.rise.go(0.0, now, 240, motion::Curve::new(0.4, 0.0, 0.9, 0.6));
            s.fade.go(0.0, now, 180, motion::STANDARD);
            s.window.set_keyboard_mode(KeyboardMode::None);
            if let Some(scrim) = s.scrim.take() {
                scrim.destroy();
            }
        }
        self.kick();
    }

    fn refresh(&self) {
        let weak = Rc::downgrade(&self.0);
        glib::spawn_future_local(async move {
            let (net, blue, power) = futures_util::join!(read_net(), read_blue(), read_power());
            let Some(s) = weak.upgrade() else { return };
            {
                let mut st = s.borrow_mut();
                st.net = net;
                st.blue = blue;
                st.power = power;
            }
            Control(s).changed();
        });
    }

    fn resize(&self) {
        let mut s = self.0.borrow_mut();
        let target = s.content().min(s.max_h);
        let now = glib::monotonic_time();
        if s.height.target() < 1.0 {
            s.height.set(target);
        } else {
            s.height.go(target, now, 260, motion::STANDARD);
        }
        let limit = (s.content() - s.max_h).max(0.0);
        s.scroll = s.scroll.min(limit);
    }

    fn hit(&self, x: f64, y: f64) -> Option<Hit> {
        let s = self.0.borrow();
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
                let blue = self.0.borrow().blue.clone();
                if let Some(path) = blue.adapter {
                    self.set_property("org.bluez", path.as_str(), "org.bluez.Adapter1", "Powered", !blue.on);
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
            Some(Hit::Device(i)) => self.toggle_device(i),
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
        let h = s.height.target();
        let bottom = s.max_h + FILLET;
        x >= FILLET && y >= bottom - h
    }

    fn toggle_device(&self, i: usize) {
        let Some(dev) = self.0.borrow().blue.devices.get(i).cloned() else { return };
        if self.0.borrow().blue.busy.contains(&dev.path) {
            return;
        }
        self.0.borrow_mut().blue.busy.push(dev.path.clone());
        self.0.borrow().canvas.queue_draw();
        let weak = Rc::downgrade(&self.0);
        glib::spawn_future_local(async move {
            let method = if dev.connected { "Disconnect" } else { "Connect" };
            let mut failed = None;
            if let Ok(conn) = crate::bus::system().await {
                if let Ok(p) = zbus::Proxy::new_owned(conn, "org.bluez", dev.path.clone(), "org.bluez.Device1").await {
                    if let Err(e) = p.call_method(method, &()).await {
                        failed = Some(e.to_string());
                    }
                }
            }
            let Some(s) = weak.upgrade() else { return };
            s.borrow_mut().blue.busy.retain(|b| *b != dev.path);
            if failed.is_some() {
                let app = s.borrow().app.clone();
                app.request(crate::app::Peek::Note { text: format!("{} couldn't {}", dev.name, if dev.connected { "disconnect" } else { "connect" }) });
            }
            Control(s).refresh();
        });
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

impl State {
    fn tiles(&self) -> Vec<Tile> {
        let mut t = vec![Tile::Network];
        if self.blue.adapter.is_some() {
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
        let devices = if self.blue.on && !self.blue.devices.is_empty() { HEAD + self.blue.devices.len() as f64 * DEVICE + 8.0 } else { 0.0 };
        let cells = self.app.gadgets.borrow().len();
        let batteries = if cells == 0 { 0.0 } else { HEAD + cells.div_ceil(PER_ROW) as f64 * CELL + 6.0 };
        let system = HEAD + CELL + 6.0;
        let weather = if self.app.weather.borrow().is_some() { 58.0 } else { 0.0 };
        PAD + weather + tiles + 14.0 + batteries + system + devices + SOUND + 16.0 + HEAD + list + PAD
    }

    fn step(&mut self, now: i64) -> bool {
        let rect = if self.open { (0, 0, (W + FILLET) as i32, (self.max_h + FILLET) as i32) } else { (0, 0, 1, 1) };
        if rect != self.region {
            if let Some(surface) = self.window.surface() {
                surface.set_input_region(Some(&cairo::Region::create_rectangle(&cairo::RectangleInt::new(rect.0, rect.1, rect.2, rect.3))));
                self.region = rect;
            }
        }
        let moving = [&self.rise, &self.fade, &self.height].iter().any(|t| !t.settled(now)) || self.app.palette_moving(now);
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
                let b = &self.blue;
                let state = if !b.on { "Off".into() } else if b.connected.is_empty() { "On".into() } else { b.connected.join(", ") };
                (if b.on { "bluetooth" } else { "bluetooth_disabled" }, "Bluetooth".into(), state, b.on)
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
        let right = W + FILLET;
        let bottom = self.max_h + FILLET;
        let h = self.height.value(now).max(1.0);
        let shift = ((1.0 - rise) * (h + FILLET)).round();
        s.save();
        s.translate(&graphene::Point::new(0.0, shift as f32));
        let solid = pal.body.a >= 1.0;
        let parts = if solid { shape::cornered_parts(right, bottom, W, h, RADIUS, FILLET) } else { None };
        let path = if solid { None } else { shape::cornered(right, bottom, W, h, RADIUS, FILLET) };
        if let Some(p) = &parts {
            draw::slab(s, p, pal.body);
            s.push_rounded_clip(&p.0);
        } else if let Some(p) = &path {
            s.append_fill(p, gsk::FillRule::Winding, &pal.body.rgba());
            s.push_fill(p, gsk::FillRule::Winding);
        } else {
            s.restore();
            return;
        }
        s.push_opacity(self.fade.value(now).clamp(0.0, 1.0));
        let left = right - W;
        let x0 = left + PAD;
        let inner = W - 2.0 * PAD;
        let mut y = bottom - h + PAD - self.scroll;
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
        for (i, tile) in tiles.iter().enumerate() {
            let tx = x0 + (i % 2) as f64 * (tile_w + TILE_GAP);
            let ty = y + (i / 2) as f64 * (TILE_H + TILE_GAP);
            let (icon, title, state, on) = self.tile_look(*tile);
            let hovered = self.hover == Some(Hit::Tile(*tile));
            let base = if on { pal.accent } else { pal.ink_main.mix(pal.body, if hovered { 0.11 } else { 0.06 }) };
            fill(s, &rounded(tx, ty, tile_w, TILE_H, 18.0), base);
            let (ink, sub) = if on { (pal.on_accent, pal.on_accent.mix(pal.accent, 0.72)) } else { (pal.ink_main, pal.ink_meta) };
            draw::icon(s, &canvas, &cfg, icon, 22.0, on, ink, tx + 30.0, ty + TILE_H / 2.0);
            draw::text(s, &canvas, &cfg, &title, 14.0, Face::Text(560), ink, tx + 54.0, ty + 24.0, tile_w - 62.0, pango::Alignment::Left);
            draw::text(s, &canvas, &cfg, &state, 12.5, Face::Text(420), sub, tx + 54.0, ty + 42.0, tile_w - 62.0, pango::Alignment::Left);
            hits.push((Hit::Tile(*tile), tx, ty + shift, tile_w, TILE_H));
        }
        let rows = tiles.len().div_ceil(2) as f64;
        y += rows * TILE_H + (rows - 1.0).max(0.0) * TILE_GAP + 14.0;
        {
            draw::text(s, &canvas, &cfg, "System", 14.0, Face::Text(600), pal.ink_main, x0 + 4.0, y + HEAD / 2.0, 220.0, pango::Alignment::Left);
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
                fill(s, &rounded(cx - 26.0, cy - 26.0, 52.0, 52.0, 26.0), pal.ink_main.mix(pal.body, 0.05));
                ring(s, cx, cy, 22.0, *frac, 3.5, pal.ink_main.alpha(0.12), *tint);
                draw::icon(s, &canvas, &cfg, glyph, 19.0, false, pal.ink_main, cx, cy);
                draw::text(s, &canvas, &cfg, value, 13.0, Face::Number(560), pal.ink_main, cx - cw / 2.0 + 3.0, cy + 39.0, cw - 6.0, pango::Alignment::Center);
                draw::text(s, &canvas, &cfg, name, 11.5, Face::Text(450), pal.ink_meta, cx - cw / 2.0 + 3.0, cy + 56.0, cw - 6.0, pango::Alignment::Center);
            }
            y += CELL + 6.0;
        }
        let gadgets = self.app.gadgets.borrow().clone();
        if !gadgets.is_empty() {
            draw::text(s, &canvas, &cfg, "Batteries", 14.0, Face::Text(600), pal.ink_main, x0 + 4.0, y + HEAD / 2.0, 220.0, pango::Alignment::Left);
            y += HEAD;
            let cw = inner / PER_ROW as f64;
            for (i, g) in gadgets.iter().enumerate() {
                let cx = x0 + cw * (i % PER_ROW) as f64 + cw / 2.0;
                let cy = y + (i / PER_ROW) as f64 * CELL + 28.0;
                let low = !g.charging && g.level <= 20;
                let tint = if g.charging { GREEN } else if low { RED } else { pal.ink_main };
                fill(s, &rounded(cx - 26.0, cy - 26.0, 52.0, 52.0, 26.0), pal.ink_main.mix(pal.body, 0.05));
                let ring = |frac: f64| {
                    let pb = gsk::PathBuilder::new();
                    let r = 22.0f64;
                    let start = -std::f64::consts::FRAC_PI_2;
                    let steps = (64.0 * frac).ceil().max(1.0) as usize;
                    for k in 0..=steps {
                        let a = start + frac * std::f64::consts::TAU * k as f64 / steps as f64;
                        let (px, py) = ((cx + r * a.cos()) as f32, (cy + r * a.sin()) as f32);
                        if k == 0 { pb.move_to(px, py) } else { pb.line_to(px, py) }
                    }
                    pb.to_path()
                };
                let stroke = gsk::Stroke::new(3.5);
                stroke.set_line_cap(gsk::LineCap::Round);
                s.append_stroke(&ring(1.0), &stroke, &pal.ink_main.alpha(0.12).rgba());
                s.append_stroke(&ring((g.level as f64 / 100.0).clamp(0.02, 1.0)), &stroke, &tint.rgba());
                draw::icon(s, &canvas, &cfg, g.glyph, 20.0, true, pal.ink_main, cx, cy);
                let value = match g.coarse {
                    Some(l) => l.to_string(),
                    None => format!("{}%", g.level),
                };
                draw::text(s, &canvas, &cfg, &value, 13.0, Face::Number(560), if low { RED } else { pal.ink_main }, cx - cw / 2.0 + 3.0, cy + 39.0, cw - 6.0, pango::Alignment::Center);
                draw::text(s, &canvas, &cfg, &g.name, 11.5, Face::Text(450), pal.ink_meta, cx - cw / 2.0 + 3.0, cy + 56.0, cw - 6.0, pango::Alignment::Center);
            }
            y += gadgets.len().div_ceil(PER_ROW) as f64 * CELL + 6.0;
        }
        if self.blue.on && !self.blue.devices.is_empty() {
            draw::text(s, &canvas, &cfg, "Bluetooth devices", 14.0, Face::Text(600), pal.ink_main, x0 + 4.0, y + HEAD / 2.0, 220.0, pango::Alignment::Left);
            y += HEAD;
            for (i, d) in self.blue.devices.iter().enumerate() {
                let ry = y + i as f64 * DEVICE;
                let hovered = self.hover == Some(Hit::Device(i));
                if hovered {
                    fill(s, &rounded(x0 - 6.0, ry + 2.0, inner + 12.0, DEVICE - 4.0, 14.0), pal.ink_main.mix(pal.body, 0.07));
                }
                let busy = self.blue.busy.contains(&d.path);
                let badge = rounded(x0 + 2.0, ry + DEVICE / 2.0 - 16.0, 32.0, 32.0, 16.0);
                fill(s, &badge, if d.connected { pal.accent.alpha(0.22) } else { pal.raised });
                draw::icon(s, &canvas, &cfg, device_symbol(&d.icon), 18.0, d.connected, if d.connected { pal.accent } else { pal.ink_meta }, x0 + 18.0, ry + DEVICE / 2.0);
                let sub = if busy {
                    if d.connected { "Disconnecting…".to_string() } else { "Connecting…".to_string() }
                } else {
                    match (d.connected, d.battery) {
                        (true, Some(b)) => format!("Connected · {b}%"),
                        (true, None) => "Connected".into(),
                        (false, _) => "Not connected".into(),
                    }
                };
                let tx = x0 + 46.0;
                let tw = inner - 46.0 - 60.0;
                draw::text(s, &canvas, &cfg, &d.name, 14.0, Face::Text(540), pal.ink_main, tx, ry + 18.0, tw, pango::Alignment::Left);
                draw::text(s, &canvas, &cfg, &sub, 12.5, Face::Text(420), pal.ink_meta, tx, ry + 36.0, tw, pango::Alignment::Left);
                let (sw_w, sw_h) = (40.0, 24.0);
                let sx = x0 + inner - sw_w - 4.0;
                let sy = ry + DEVICE / 2.0 - sw_h / 2.0;
                let on = d.connected != busy;
                fill(s, &rounded(sx, sy, sw_w, sw_h, sw_h / 2.0), if on { pal.accent } else { pal.ink_main.alpha(0.18) });
                let knob = sw_h - 6.0;
                let kx = if on { sx + sw_w - knob - 3.0 } else { sx + 3.0 };
                fill(s, &rounded(kx, sy + 3.0, knob, knob, knob / 2.0), if on { pal.on_accent } else { pal.ink_main });
                hits.push((Hit::Device(i), x0 - 6.0, ry + 2.0 + shift, inner + 12.0, DEVICE - 4.0));
            }
            y += self.blue.devices.len() as f64 * DEVICE + 8.0;
        }
        let (level, muted) = self.app.volume.get().unwrap_or((0.0, false));
        let mid = y + SOUND / 2.0;
        let mute_hover = self.hover == Some(Hit::Mute);
        fill(s, &rounded(x0, mid - 18.0, 36.0, 36.0, 18.0), pal.ink_main.mix(pal.body, if mute_hover { 0.12 } else { 0.06 }));
        let speaker = if muted { "volume_off" } else if level < 0.01 { "volume_mute" } else if level < 0.5 { "volume_down" } else { "volume_up" };
        draw::icon(s, &canvas, &cfg, speaker, 20.0, true, if muted { pal.ink_meta } else { pal.ink_main }, x0 + 18.0, mid);
        hits.push((Hit::Mute, x0, mid - 18.0 + shift, 36.0, 36.0));
        let sx = x0 + 50.0;
        let sw = inner - 50.0 - 48.0;
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
        y += SOUND + 16.0;
        let history = self.app.history.borrow().clone();
        draw::text(s, &canvas, &cfg, "Notifications", 14.0, Face::Text(600), pal.ink_main, x0 + 4.0, y + HEAD / 2.0, 200.0, pango::Alignment::Left);
        if !history.is_empty() {
            let cw = draw::width(&canvas, &cfg, "Clear", 13.0, Face::Text(500)) + 20.0;
            let cx = x0 + inner - cw;
            if self.hover == Some(Hit::Clear) {
                fill(s, &rounded(cx, y + 4.0, cw, HEAD - 8.0, (HEAD - 8.0) / 2.0), pal.ink_main.mix(pal.body, 0.08));
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
                fill(s, &rounded(x0 - 6.0, ry + 2.0, inner + 12.0, ROW - 4.0, 14.0), pal.ink_main.mix(pal.body, 0.07));
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
                fill(s, &rounded(rx + 4.0, ry + ROW / 2.0 - 14.0, 28.0, 28.0, 14.0), pal.ink_main.mix(pal.body, if close_hover { 0.14 } else { 0.08 }));
                draw::icon(s, &canvas, &cfg, "close", 16.0, false, pal.ink_sec, rx + 18.0, ry + ROW / 2.0);
            } else {
                draw::text(s, &canvas, &cfg, &ago(n.at), 12.0, Face::Number(450), pal.ink_faint, rx - 4.0, ry + 22.0, 40.0, pango::Alignment::Right);
            }
            hits.push((Hit::Row(i), x0 - 6.0, ry + 2.0 + shift, inner + 12.0, ROW - 4.0));
            hits.push((Hit::Close(i), rx + 4.0, ry + ROW / 2.0 - 14.0 + shift, 28.0, 28.0));
        }
        s.pop();
        s.pop();
        s.restore();
        let floor = bottom - h;
        self.hits = hits.into_iter().filter(|(_, _, hy, _, hh)| hy + hh > floor && *hy < bottom).collect();
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
