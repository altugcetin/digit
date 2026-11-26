use gdk4_wayland::prelude::*;
use gtk::{gdk, glib};
use std::cell::{Cell, RefCell};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::wp::idle_inhibit::zv1::client::zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1;
use wayland_protocols::wp::idle_inhibit::zv1::client::zwp_idle_inhibitor_v1::ZwpIdleInhibitorV1;

struct State;

pub struct Awake {
    conn: Connection,
    qh: QueueHandle<State>,
    manager: ZwpIdleInhibitManagerV1,
    /// Whether the user asked for it, kept apart from the inhibitor: that one lives on a notch's
    /// surface, which goes when the monitors change (waking up, a lid, a new screen).
    want: Cell<bool>,
    held: RefCell<Option<(ZwpIdleInhibitorV1, glib::WeakRef<gdk::Surface>)>>,
    _queue: wayland_client::EventQueue<State>,
}

/// There while the computer is kept awake, so it still is after a restart or a new boot.
fn flag() -> std::path::PathBuf {
    kit::theme::state_dir().join("awake")
}

impl Awake {
    pub fn new(display: &gdk::Display) -> Option<Self> {
        let wl = display.downcast_ref::<gdk4_wayland::WaylandDisplay>()?.wl_display()?;
        let conn = Connection::from_backend(wl.backend().upgrade()?);
        let (globals, queue) = registry_queue_init::<State>(&conn).ok()?;
        let qh = queue.handle();
        let manager = globals.bind::<ZwpIdleInhibitManagerV1, _, _>(&qh, 1..=1, ()).ok()?;
        Some(Self { conn, qh, manager, want: Cell::new(flag().exists()), held: RefCell::new(None), _queue: queue })
    }

    pub fn on(&self) -> bool {
        self.want.get()
    }

    /// Keeps the computer awake or lets it sleep again, and remembers which.
    pub fn set(&self, surface: Option<&gdk::Surface>, on: bool) {
        self.want.set(on);
        if on {
            let _ = kit::atomic::write(&flag(), "");
        } else {
            let _ = std::fs::remove_file(flag());
        }
        self.hold(surface);
    }

    /// Holds the inhibitor on `surface` while it's wanted: again when the surface it was on has
    /// gone, and not at all once it isn't wanted.
    pub fn hold(&self, surface: Option<&gdk::Surface>) {
        let gone = self.held.borrow().as_ref().is_some_and(|(_, s)| s.upgrade().is_none_or(|s| s.is_destroyed()));
        if (gone || !self.want.get()) && let Some((inhibitor, _)) = self.held.borrow_mut().take() {
            inhibitor.destroy();
        }
        if self.want.get() && self.held.borrow().is_none() {
            let Some(surface) = surface else { return };
            let Some(wl) = surface.downcast_ref::<gdk4_wayland::WaylandSurface>().and_then(|s| s.wl_surface()) else { return };
            *self.held.borrow_mut() = Some((self.manager.create_inhibitor(&wl, &self.qh, ()), surface.downgrade()));
        }
        let _ = self.conn.flush();
    }
}

/// Holds keeping awake on the first notch, as it was asked for: at start and whenever the
/// notches are made again.
pub fn keep(app: &crate::app::App) {
    let surface = app.notches.borrow().first().and_then(|n| n.surface());
    if let Some(a) = app.awake.borrow().as_ref() {
        a.hold(surface.as_ref());
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &WlRegistry, _: <WlRegistry as Proxy>::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ZwpIdleInhibitManagerV1, ()> for State {
    fn event(_: &mut Self, _: &ZwpIdleInhibitManagerV1, _: <ZwpIdleInhibitManagerV1 as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ZwpIdleInhibitorV1, ()> for State {
    fn event(_: &mut Self, _: &ZwpIdleInhibitorV1, _: <ZwpIdleInhibitorV1 as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
