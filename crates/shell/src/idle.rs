use crate::app::App;
use crate::paint::Paint;
use gdk4_wayland::prelude::*;
use gtk::{cairo, gdk, glib, graphene, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::cell::RefCell;
use std::os::fd::AsRawFd;
use std::rc::{Rc, Weak};
use std::time::Duration;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_registry::WlRegistry, wl_seat::WlSeat};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notification_v1::{self as note, ExtIdleNotificationV1};
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notifier_v1::ExtIdleNotifierV1;

const DIM: u32 = 8;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Step {
    Dim,
    Lock,
    Off,
    Sleep,
    /// Nobody has touched anything for a while; screen time stops counting.
    Away,
}

#[derive(Default)]
struct Events {
    list: Vec<(Step, bool)>,
}

struct Idle {
    conn: Connection,
    queue: EventQueue<Events>,
    qh: QueueHandle<Events>,
    notifier: ExtIdleNotifierV1,
    seat: WlSeat,
    notes: Vec<ExtIdleNotificationV1>,
    plan: (u32, u32, u32),
    dark: Vec<gtk::Window>,
    off: bool,
}

/// How long without input before screen time stops counting.
const AWAY: u32 = 120;

thread_local! {
    static AWAY_NOW: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static IDLE: RefCell<Option<Idle>> = const { RefCell::new(None) };
}

pub fn start(app: &Rc<App>) {
    // A connection of its own. On GTK's, GTK reads the socket and nothing wakes anyone for
    // events left in a queue of ours, so it had to be polled; this one's socket wakes the main
    // loop exactly when the compositor reports idle or activity.
    let Ok(conn) = Connection::connect_to_env() else { return };
    let Ok((globals, queue)) = registry_queue_init::<Events>(&conn) else { return };
    let qh = queue.handle();
    let Ok(notifier) = globals.bind::<ExtIdleNotifierV1, _, _>(&qh, 1..=1, ()) else { return };
    let Ok(seat) = globals.bind::<WlSeat, _, _>(&qh, 1..=1, ()) else { return };
    let fd = conn.backend().poll_fd().as_raw_fd();
    IDLE.with(|i| *i.borrow_mut() = Some(Idle { conn, queue, qh, notifier, seat, notes: Vec::new(), plan: (0, 0, 0), dark: Vec::new(), off: false }));
    apply(app);
    let weak = Rc::downgrade(app);
    glib_unix::unix_fd_add_local(fd, glib::IOCondition::IN | glib::IOCondition::HUP | glib::IOCondition::ERR, move |_, _| {
        let Some(app) = weak.upgrade() else { return glib::ControlFlow::Break };
        if pump(&app) { glib::ControlFlow::Continue } else { glib::ControlFlow::Break }
    });
    sleep_watch(Rc::downgrade(app));
}

/// Nobody is at the computer: it has sat idle a couple of minutes, or it is locked.
pub fn away() -> bool {
    AWAY_NOW.with(|a| a.get()) || crate::lock::active()
}

pub fn plan() -> (u32, u32, u32) {
    IDLE.with(|i| i.borrow().as_ref().map_or((0, 0, 0), |s| s.plan))
}

pub fn apply(app: &App) {
    let prefs = app.prefs.borrow();
    let mins = |m: f64| if m > 0.0 { (m * 60.0).round().max(30.0) as u32 } else { 0 };
    let (off, sleep) = if app.power.borrow().on_battery() { kit::power::battery_idle(&kit::prefs::read()) } else { (prefs.idle_off, prefs.idle_sleep) };
    let plan = (mins(prefs.idle_lock), mins(off), mins(sleep));
    drop(prefs);
    if IDLE.with(|i| i.borrow().as_ref().is_some_and(|s| s.plan != plan)) {
        dim(false);
    }
    IDLE.with(|i| {
        let mut slot = i.borrow_mut();
        let Some(idle) = slot.as_mut() else { return };
        if idle.plan == plan && !idle.notes.is_empty() {
            return;
        }
        for n in idle.notes.drain(..) {
            n.destroy();
        }
        AWAY_NOW.with(|a| a.set(false));
        idle.plan = plan;
        let (lock, off, sleep) = plan;
        let first = [lock, off].into_iter().filter(|t| *t > 0).min();
        let mut wants: Vec<(Step, u32)> = vec![(Step::Away, AWAY)];
        if let Some(f) = first.filter(|f| *f > DIM + 20) {
            wants.push((Step::Dim, f - DIM));
        }
        if lock > 0 {
            wants.push((Step::Lock, lock));
        }
        if off > 0 {
            wants.push((Step::Off, off));
        }
        if sleep > 0 {
            wants.push((Step::Sleep, sleep));
        }
        for (step, secs) in wants {
            let n = idle.notifier.get_idle_notification(secs.saturating_mul(1000), &idle.seat, &idle.qh, step);
            idle.notes.push(n);
        }
        let _ = idle.conn.flush();
    });
}

/// Reads what the compositor sent and acts on it; false once the connection is gone.
fn pump(app: &Rc<App>) -> bool {
    let events = IDLE.with(|i| {
        let mut slot = i.borrow_mut();
        let idle = slot.as_mut()?;
        // None: events are queued already, and only need dispatching.
        if let Some(guard) = idle.conn.prepare_read() {
            match guard.read() {
                Ok(_) => {}
                Err(wayland_client::backend::WaylandError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => return None,
            }
        }
        let mut ev = Events::default();
        idle.queue.dispatch_pending(&mut ev).ok()?;
        Some(ev.list)
    });
    let Some(events) = events else { return false };
    for (step, idled) in events {
        act(app, step, idled);
    }
    true
}

fn act(app: &Rc<App>, step: Step, idled: bool) {
    match (step, idled) {
        (Step::Dim, true) => dim(true),
        (Step::Dim, false) => {
            dim(false);
            let was_off = IDLE.with(|i| i.borrow_mut().as_mut().map(|s| std::mem::replace(&mut s.off, false)).unwrap_or(false));
            if was_off && kit::wm::which() != kit::wm::Wm::Niri {
                glib::spawn_future_local(async {
                    let _ = gtk::gio::spawn_blocking(|| kit::wm::screens(true)).await;
                });
            }
        }
        (Step::Lock, true) => {
            dim(false);
            crate::lock::lock(app);
        }
        (Step::Off, true) => {
            dim(false);
            IDLE.with(|i| {
                if let Some(s) = i.borrow_mut().as_mut() {
                    s.off = true;
                }
            });
            glib::spawn_future_local(async {
                let _ = gtk::gio::spawn_blocking(|| kit::wm::screens(false)).await;
            });
        }
        (Step::Sleep, true) => {
            dim(false);
            glib::spawn_future_local(async {
                let Ok(conn) = crate::bus::system().await else { return };
                let Ok(m) = zbus::Proxy::new_owned(conn, "org.freedesktop.login1", "/org/freedesktop/login1", "org.freedesktop.login1.Manager").await else { return };
                let _ = m.call_method("Suspend", &(false,)).await;
            });
        }
        (Step::Away, idled) => AWAY_NOW.with(|a| a.set(idled)),
        (Step::Off, false) => {
            let was_off = IDLE.with(|i| i.borrow_mut().as_mut().map(|s| std::mem::replace(&mut s.off, false)).unwrap_or(false));
            if was_off && kit::wm::which() != kit::wm::Wm::Niri {
                glib::spawn_future_local(async {
                    let _ = gtk::gio::spawn_blocking(|| kit::wm::screens(true)).await;
                });
            }
        }
        _ => {}
    }
}

fn dim(on: bool) {
    if !on {
        let windows = IDLE.with(|i| i.borrow_mut().as_mut().map(|s| std::mem::take(&mut s.dark)).unwrap_or_default());
        for w in windows {
            w.destroy();
        }
        return;
    }
    if crate::lock::active() {
        return;
    }
    let mut made = Vec::new();
    for m in kit::screens::all() {
        let window = gtk::Window::new();
        window.init_layer_shell();
        window.set_layer(Layer::Overlay);
        window.set_namespace(Some("digit-dim"));
        window.set_exclusive_zone(-1);
        window.set_keyboard_mode(KeyboardMode::None);
        window.set_monitor(Some(&m));
        for e in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            window.set_anchor(e, true);
        }
        window.add_css_class("digit");
        let paint = Paint::new(-1, -1);
        let start = glib::monotonic_time();
        let weak = paint.downgrade();
        paint.on_draw(move |s| {
            let Some(p) = weak.upgrade() else { return };
            let now = p.frame_clock().map(|c| c.frame_time()).unwrap_or_else(glib::monotonic_time);
            let t = if kit::motion::reduced() {
                1.0
            } else {
                ((now - start) as f64 / (DIM as f64 * 1_000_000.0)).clamp(0.0, 1.0)
            };
            s.append_color(&gdk::RGBA::new(0.0, 0.0, 0.0, (0.6 * t * t * (3.0 - 2.0 * t)) as f32), &graphene::Rect::new(0.0, 0.0, p.width() as f32, p.height() as f32));
        });
        if !kit::motion::reduced() {
            paint.add_tick_callback(move |p, clock| {
                p.queue_draw();
                if kit::motion::reduced() || clock.frame_time() - start > DIM as i64 * 1_000_000 {
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            });
        }
        window.set_child(Some(&paint));
        window.connect_realize(|w| {
            if let Some(surface) = w.surface() {
                surface.set_input_region(Some(&cairo::Region::create()));
            }
        });
        window.present();
        made.push(window);
    }
    IDLE.with(|i| {
        if let Some(s) = i.borrow_mut().as_mut() {
            s.dark.extend(made);
        }
    });
}

async fn inhibit(conn: &zbus::Connection) -> Option<zbus::zvariant::OwnedFd> {
    let m = zbus::Proxy::new_owned(conn.clone(), "org.freedesktop.login1", "/org/freedesktop/login1", "org.freedesktop.login1.Manager").await.ok()?;
    m.call::<_, _, zbus::zvariant::OwnedFd>("Inhibit", &("sleep", "Digit", "Lock the screen before sleep", "delay")).await.ok()
}

fn sleep_watch(weak: Weak<App>) {
    glib::spawn_future_local(async move {
        let Ok(conn) = crate::bus::system().await else { return };
        let mut held = inhibit(&conn).await;
        let Ok(rule) = zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).sender("org.freedesktop.login1").map(|b| b.build()) else { return };
        let Ok(stream) = zbus::MessageStream::for_match_rule(rule, &conn, Some(16)).await else { return };
        let stream = kit::signals::drained(stream);
        let ours: Option<String> = match zbus::Proxy::new_owned(conn.clone(), "org.freedesktop.login1", "/org/freedesktop/login1", "org.freedesktop.login1.Manager").await {
            Ok(m) => m.call::<_, _, zbus::zvariant::OwnedObjectPath>("GetSession", &("auto",)).await.ok().map(|p| p.to_string()),
            Err(_) => None,
        };
        while let Ok(msg) = stream.recv().await {
            let Ok(msg) = msg else { continue };
            let header = msg.header();
            let member = header.member().map(|m| m.to_string()).unwrap_or_default();
            let path = header.path().map(|p| p.to_string()).unwrap_or_default();
            let Some(app) = weak.upgrade() else { break };
            match member.as_str() {
                "PrepareForSleep" => {
                    let Ok(going) = msg.body().deserialize::<bool>() else { continue };
                    if going {
                        crate::brightness::keep_keyboard().await;
                        if app.prefs.borrow().lock_on_sleep {
                            crate::lock::lock(&app);
                            let deadline = std::time::Instant::now() + Duration::from_secs(3);
                            while !crate::lock::confirmed() && std::time::Instant::now() < deadline {
                                glib::timeout_future(Duration::from_millis(25)).await;
                            }
                        }
                        held = None;
                    } else {
                        // Firmware often turns the keyboard light off as the computer wakes,
                        // sometimes a moment after.
                        glib::spawn_future_local(async {
                            for wait in [700, 2500] {
                                glib::timeout_future(Duration::from_millis(wait)).await;
                                crate::brightness::restore_keyboard(true).await;
                            }
                        });
                        if held.is_none() {
                            held = inhibit(&conn).await;
                        }
                    }
                }
                "Lock" if ours.as_deref().is_some_and(|o| o == path) => {
                    crate::lock::lock(&app);
                }
                _ => {}
            }
        }
        drop(held);
    });
}

impl Dispatch<WlRegistry, GlobalListContents> for Events {
    fn event(_: &mut Self, _: &WlRegistry, _: <WlRegistry as Proxy>::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WlSeat, ()> for Events {
    fn event(_: &mut Self, _: &WlSeat, _: <WlSeat as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ExtIdleNotifierV1, ()> for Events {
    fn event(_: &mut Self, _: &ExtIdleNotifierV1, _: <ExtIdleNotifierV1 as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ExtIdleNotificationV1, Step> for Events {
    fn event(state: &mut Self, _: &ExtIdleNotificationV1, event: note::Event, step: &Step, _: &Connection, _: &QueueHandle<Self>) {
        match event {
            note::Event::Idled => state.list.push((*step, true)),
            note::Event::Resumed => state.list.push((*step, false)),
            _ => {}
        }
    }
}
