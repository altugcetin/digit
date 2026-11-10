use gdk4_wayland::prelude::*;
use gtk::{gdk, glib};
use std::cell::RefCell;
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
    held: RefCell<Option<(ZwpIdleInhibitorV1, glib::WeakRef<gdk::Surface>)>>,
    _queue: wayland_client::EventQueue<State>,
}

impl Awake {
    pub fn new(display: &gdk::Display) -> Option<Self> {
        let wl = display.downcast_ref::<gdk4_wayland::WaylandDisplay>()?.wl_display()?;
        let conn = Connection::from_backend(wl.backend().upgrade()?);
        let (globals, queue) = registry_queue_init::<State>(&conn).ok()?;
        let qh = queue.handle();
        let manager = globals.bind::<ZwpIdleInhibitManagerV1, _, _>(&qh, 1..=1, ()).ok()?;
        Some(Self { conn, qh, manager, held: RefCell::new(None), _queue: queue })
    }

    pub fn on(&self) -> bool {
        let surface = self.held.borrow().as_ref().map(|(_, s)| s.clone());
        let dead = surface.is_some_and(|s| s.upgrade().is_none_or(|s| s.is_destroyed()));
        if dead { self.set(None, false); }
        self.held.borrow().is_some()
    }

    pub fn set(&self, surface: Option<&gdk::Surface>, on: bool) {
        if on { let _ = self.on(); }
        if !on {
            let held = self.held.borrow_mut().take();
            if let Some((i, _)) = held {
                i.destroy();
            }
        } else if self.held.borrow().is_none() {
            let Some(wl) = surface.and_then(|s| s.downcast_ref::<gdk4_wayland::WaylandSurface>()).and_then(|s| s.wl_surface()) else { return };
            *self.held.borrow_mut() = Some((self.manager.create_inhibitor(&wl, &self.qh, ()), surface.unwrap().downgrade()));
        }
        let _ = self.conn.flush();
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
