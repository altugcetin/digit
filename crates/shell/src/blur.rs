use gdk4_wayland::prelude::*;
use gtk::gdk;
use std::cell::{Cell, RefCell};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_compositor::WlCompositor, wl_region::WlRegion, wl_registry::WlRegistry};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols::ext::background_effect::v1::client::ext_background_effect_manager_v1::{
    self as manager, ExtBackgroundEffectManagerV1,
};
use wayland_protocols::ext::background_effect::v1::client::ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1;

pub type Effect = ExtBackgroundEffectSurfaceV1;

#[derive(Default)]
struct State {
    blur: bool,
}

pub struct Blur {
    conn: Connection,
    queue: RefCell<EventQueue<State>>,
    qh: QueueHandle<State>,
    compositor: WlCompositor,
    manager: ExtBackgroundEffectManagerV1,
    able: Cell<bool>,
}

impl Blur {
    pub fn new(display: &gdk::Display) -> Option<Self> {
        let wl = display.downcast_ref::<gdk4_wayland::WaylandDisplay>()?.wl_display()?;
        let conn = Connection::from_backend(wl.backend().upgrade()?);
        let (globals, mut queue) = registry_queue_init::<State>(&conn).ok()?;
        let qh = queue.handle();
        let compositor = globals.bind::<WlCompositor, _, _>(&qh, 1..=6, ()).ok()?;
        let manager = globals.bind::<ExtBackgroundEffectManagerV1, _, _>(&qh, 1..=1, ()).ok()?;
        let mut state = State::default();
        queue.roundtrip(&mut state).ok()?;
        Some(Self { conn, qh, compositor, manager, able: Cell::new(state.blur), queue: RefCell::new(queue) })
    }

    pub fn able(&self) -> bool {
        let mut state = State { blur: self.able.get() };
        if let Ok(mut q) = self.queue.try_borrow_mut() {
            let _ = q.dispatch_pending(&mut state);
        }
        self.able.set(state.blur);
        state.blur
    }

    pub fn surface_id(surface: &gdk::Surface) -> Option<wayland_client::backend::ObjectId> {
        surface.downcast_ref::<gdk4_wayland::WaylandSurface>()?.wl_surface().map(|s| s.id())
    }

    pub fn attach(&self, surface: &gdk::Surface) -> Option<(Effect, wayland_client::backend::ObjectId)> {
        let wl = surface.downcast_ref::<gdk4_wayland::WaylandSurface>()?.wl_surface()?;
        if !wl.is_alive() {
            return None;
        }
        Some((self.manager.get_background_effect(&wl, &self.qh, ()), wl.id()))
    }

    pub fn detach(&self, effect: Effect) {
        effect.destroy();
        let _ = self.conn.flush();
    }

    pub fn set(&self, effect: &Effect, rects: &[(i32, i32, i32, i32)]) {
        if !effect.is_alive() {
            return;
        }
        let region = self.compositor.create_region(&self.qh, ());
        for &(x, y, w, h) in rects {
            region.add(x, y, w, h);
        }
        effect.set_blur_region(Some(&region));
        region.destroy();
        let _ = self.conn.flush();
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &WlRegistry, _: <WlRegistry as Proxy>::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ExtBackgroundEffectManagerV1, ()> for State {
    fn event(state: &mut Self, _: &ExtBackgroundEffectManagerV1, event: manager::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let manager::Event::Capabilities { flags } = event {
            state.blur = matches!(flags, WEnum::Value(f) if f.contains(manager::Capability::Blur));
        }
    }
}

macro_rules! quiet {
    ($t:ty) => {
        impl Dispatch<$t, ()> for State {
            fn event(_: &mut Self, _: &$t, _: <$t as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
        }
    };
}

quiet!(WlCompositor);
quiet!(WlRegion);
quiet!(Effect);
