use crate::app::App;
use gdk4_wayland::prelude::*;
use gtk::{gdk, glib};
use std::cell::{Cell, RefCell};
use std::io::Write;
use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::rc::Rc;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols_wlr::gamma_control::v1::client::zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1;
use wayland_protocols_wlr::gamma_control::v1::client::zwlr_gamma_control_v1::{self as control, ZwlrGammaControlV1};

const DAY: f64 = 6500.0;
const FADE: f64 = 40.0;

#[derive(Default)]
struct Sizes {
    list: Vec<(usize, u32)>,
    failed: Vec<usize>,
}

struct Night {
    conn: Connection,
    queue: EventQueue<Sizes>,
    qh: QueueHandle<Sizes>,
    manager: ZwlrGammaControlManagerV1,
    controls: Vec<(String, ZwlrGammaControlV1, u32)>,
    shown: f64,
}

thread_local! {
    static NIGHT: RefCell<Option<Night>> = const { RefCell::new(None) };
    static FORCE: Cell<Option<(bool, bool)>> = const { Cell::new(None) };
    static SUN: Cell<Option<(f64, f64)>> = const { Cell::new(None) };
}

fn white(kelvin: f64) -> (f64, f64, f64) {
    let t = kelvin.clamp(1000.0, 10000.0) / 100.0;
    let r = if t <= 66.0 { 255.0 } else { 329.698_727_446 * (t - 60.0).powf(-0.133_204_759_2) };
    let g = if t <= 66.0 { 99.470_802_586_1 * t.ln() - 161.119_568_166_1 } else { 288.122_169_528_3 * (t - 60.0).powf(-0.075_514_849_2) };
    let b = if t >= 66.0 { 255.0 } else if t <= 19.0 { 0.0 } else { 138.517_731_223_1 * (t - 10.0).ln() - 305.044_792_730_7 };
    ((r / 255.0).clamp(0.0, 1.0), (g / 255.0).clamp(0.0, 1.0), (b / 255.0).clamp(0.0, 1.0))
}

fn ramp(size: u32, kelvin: f64) -> Option<OwnedFd> {
    let n = size as usize;
    if n < 2 {
        return None;
    }
    let (r, g, b) = white(kelvin);
    let mut bytes = Vec::with_capacity(n * 6);
    for k in [r, g, b] {
        for i in 0..n {
            let v = (i as f64 / (n - 1) as f64 * k * 65535.0).round().clamp(0.0, 65535.0) as u16;
            bytes.extend_from_slice(&v.to_ne_bytes());
        }
    }
    // SAFETY: a static NUL-terminated name and flags; returns a new descriptor or -1.
    let fd = unsafe { libc::memfd_create(c"digit-gamma".as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return None;
    }
    // SAFETY: `fd` is the new descriptor just created, owned by nothing else.
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };
    let mut file = std::fs::File::from(owned);
    file.write_all(&bytes).ok()?;
    use std::io::Seek;
    file.seek(std::io::SeekFrom::Start(0)).ok()?;
    Some(file.into())
}

fn minutes(s: &str) -> Option<f64> {
    let (h, m) = s.trim().split_once(':')?;
    Some(h.parse::<f64>().ok()? * 60.0 + m.parse::<f64>().ok()?)
}

pub fn set_sun(rise: f64, set: f64) {
    SUN.with(|s| s.set(Some((rise, set))));
}

fn clock() -> f64 {
    glib::DateTime::now_local().map(|d| d.hour() as f64 * 60.0 + d.minute() as f64 + d.second() as f64 / 60.0).unwrap_or(0.0)
}

fn amount(app: &App, now: f64) -> f64 {
    if let Some((on, _)) = FORCE.with(Cell::get) {
        return if on { 1.0 } else { 0.0 };
    }
    natural(app, now)
}

fn natural(app: &App, now: f64) -> f64 {
    let prefs = app.prefs.borrow();
    let (start, end) = match prefs.night_light.as_str() {
        "always" => return 1.0,
        "sunset" => match SUN.with(Cell::get) {
            Some((rise, set)) => (set, rise),
            None => (minutes(&prefs.night_from).unwrap_or(1200.0), minutes(&prefs.night_to).unwrap_or(420.0)),
        },
        "schedule" => (minutes(&prefs.night_from).unwrap_or(1200.0), minutes(&prefs.night_to).unwrap_or(420.0)),
        _ => return 0.0,
    };
    let after = |a: f64, b: f64| (a - b).rem_euclid(1440.0);
    let into = after(now, start);
    let span = after(end, start);
    if into < span {
        let rise = (into / FADE).min(1.0);
        let fall = ((span - into) / FADE).min(1.0);
        rise.min(fall)
    } else {
        0.0
    }
}

pub fn active(app: &App) -> bool {
    amount(app, clock()) > 0.5
}

pub fn force(app: &App, on: bool) {
    let natural = natural(app, clock()) > 0.5;
    FORCE.with(|f| f.set(if on == natural { None } else { Some((on, natural)) }));
    apply(app);
}

fn connect() -> Option<Night> {
    let display = gdk::Display::default()?;
    let wl = display.downcast_ref::<gdk4_wayland::WaylandDisplay>()?.wl_display()?;
    let conn = Connection::from_backend(wl.backend().upgrade()?);
    let (globals, queue) = registry_queue_init::<Sizes>(&conn).ok()?;
    let qh = queue.handle();
    let manager = globals.bind::<ZwlrGammaControlManagerV1, _, _>(&qh, 1..=1, ()).ok()?;
    Some(Night { conn, queue, qh, manager, controls: Vec::new(), shown: -1.0 })
}

pub fn apply(app: &App) {
    let t = amount(app, clock());
    let kelvin = DAY + (app.prefs.borrow().night_temp.clamp(2500.0, 6500.0) - DAY) * t;
    NIGHT.with(|n| {
        let mut slot = n.borrow_mut();
        if t <= 0.0 {
            if let Some(night) = slot.take() {
                for (_, c, _) in night.controls {
                    c.destroy();
                }
                let _ = night.conn.flush();
            }
            return;
        }
        if slot.is_none() {
            *slot = connect();
        }
        let Some(night) = slot.as_mut() else { return };
        let monitors = kit::screens::all();
        let names: Vec<String> = monitors.iter().filter_map(|m| m.connector().map(|c| c.to_string())).collect();
        night.controls.retain(|(name, c, _)| {
            let keep = names.contains(name);
            if !keep {
                c.destroy();
            }
            keep
        });
        let mut fresh = false;
        for (i, m) in monitors.iter().enumerate() {
            let Some(name) = m.connector().map(|c| c.to_string()) else { continue };
            if night.controls.iter().any(|(n, _, _)| *n == name) {
                continue;
            }
            let Some(output) = m.downcast_ref::<gdk4_wayland::WaylandMonitor>().and_then(|w| w.wl_output()) else { continue };
            let c = night.manager.get_gamma_control(&output, &night.qh, i);
            night.controls.push((name, c, 0));
            fresh = true;
        }
        if fresh {
            let mut sizes = Sizes::default();
            let _ = night.queue.roundtrip(&mut sizes);
            for (i, size) in sizes.list {
                if let Some(name) = monitors.get(i).and_then(|m| m.connector()).map(|c| c.to_string()) {
                    if let Some(entry) = night.controls.iter_mut().find(|(n, _, _)| *n == name) {
                        entry.2 = size;
                    }
                }
            }
            for i in sizes.failed {
                if let Some(name) = monitors.get(i).and_then(|m| m.connector()).map(|c| c.to_string()) {
                    night.controls.retain(|(n, _, _)| *n != name);
                }
            }
            night.shown = -1.0;
        }
        if (night.shown - kelvin).abs() < 15.0 {
            return;
        }
        for (_, c, size) in &night.controls {
            if let Some(fd) = ramp(*size, kelvin) {
                c.set_gamma(fd.as_fd());
            }
        }
        night.shown = kelvin;
        let _ = night.conn.flush();
    });
}

pub fn start(app: &Rc<App>) {
    apply(app);
    let weak = Rc::downgrade(app);
    glib::timeout_add_seconds_local(60, move || {
        let Some(app) = weak.upgrade() else { return glib::ControlFlow::Break };
        if let Some((_, was)) = FORCE.with(Cell::get) {
            if (natural(&app, clock()) > 0.5) != was {
                FORCE.with(|f| f.set(None));
            }
        }
        apply(&app);
        glib::ControlFlow::Continue
    });
}

impl Dispatch<WlRegistry, GlobalListContents> for Sizes {
    fn event(_: &mut Self, _: &WlRegistry, _: <WlRegistry as Proxy>::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ZwlrGammaControlManagerV1, ()> for Sizes {
    fn event(_: &mut Self, _: &ZwlrGammaControlManagerV1, _: <ZwlrGammaControlManagerV1 as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ZwlrGammaControlV1, usize> for Sizes {
    fn event(state: &mut Self, _: &ZwlrGammaControlV1, event: control::Event, index: &usize, _: &Connection, _: &QueueHandle<Self>) {
        match event {
            control::Event::GammaSize { size } => state.list.push((*index, size)),
            control::Event::Failed => state.failed.push(*index),
            _ => {}
        }
    }
}
