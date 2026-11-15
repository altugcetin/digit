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

/// The menu bar's frosted band. Since 4.23.3 GTK owns each surface's background effect and fills
/// its blur region from widgets with a CSS `backdrop-filter`, so the frost is an empty pane laid
/// under the drawing, kept in step with where the band is drawn.
#[derive(Clone)]
pub struct Frost(std::rc::Rc<Pane>);

pub struct Pane {
    pane: gtk::Box,
    /// The pane as last asked for: x, y, width and height.
    want: Cell<Option<[i32; 4]>>,
    shown: Cell<Option<[i32; 4]>>,
    pending: Cell<bool>,
}

impl Frost {
    /// Wraps the drawing in an overlay that also holds the pane, under the drawing.
    pub fn wrap(content: &impl gtk::prelude::IsA<gtk::Widget>) -> (gtk::Overlay, Self) {
        use gtk::prelude::*;
        // The pane sits under the drawing: GTK renders a backdrop filter too, and over the
        // drawing it would blur the drawing itself.
        let overlay = gtk::Overlay::new();
        let under = gtk::Fixed::new();
        overlay.set_child(Some(&under));
        overlay.add_overlay(content);
        overlay.set_measure_overlay(content, true);
        overlay.set_hexpand(content.hexpands());
        overlay.set_vexpand(content.vexpands());
        let pane = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        pane.set_can_target(false);
        pane.set_visible(false);
        pane.add_css_class("digit-frost");
        let css = gtk::CssProvider::new();
        css.load_from_string(".digit-frost { backdrop-filter: blur(20px); }");
        #[allow(deprecated)]
        pane.style_context().add_provider(&css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        under.put(&pane, 0.0, 0.0);
        (overlay, Self(std::rc::Rc::new(Pane { pane, want: Cell::new(None), shown: Cell::new(None), pending: Cell::new(false) })))
    }

    /// Frosts `rect` (surface coordinates), or nothing. The pane stays inside `bounds` (the
    /// surface's size): one reaching past it would make GTK grow the surface. Safe to call while
    /// drawing: the pane moves on the next idle.
    pub fn place(&self, rect: Option<(f64, f64, f64, f64)>, bounds: (f64, f64)) {
        let clip = |(x, y, w, h): (f64, f64, f64, f64)| {
            let (x0, y0) = (x.max(0.0), y.max(0.0));
            let (x1, y1) = ((x + w).min(bounds.0), (y + h).min(bounds.1));
            (x0, y0, x1 - x0, y1 - y0)
        };
        let want = rect.filter(|_| !kit::effects::reduced()).map(clip).filter(|r| r.2 >= 1.0 && r.3 >= 1.0).map(|(x, y, w, h)| [x.round() as i32, y.round() as i32, w.round() as i32, h.round() as i32]);
        if self.0.want.replace(want) == want || self.0.pending.replace(true) {
            return;
        }
        let me = self.clone();
        gtk::glib::idle_add_local_once(move || me.apply());
    }

    fn apply(&self) {
        use gtk::prelude::*;
        let p = &self.0;
        p.pending.set(false);
        let want = p.want.get();
        if p.shown.replace(want) == want {
            return;
        }
        let Some([x, y, w, h]) = want else {
            p.pane.set_visible(false);
            return;
        };
        if let Some(fixed) = p.pane.parent().and_downcast::<gtk::Fixed>() {
            fixed.move_(&p.pane, x as f64, y as f64);
        }
        p.pane.set_size_request(w, h);
        p.pane.set_visible(true);
    }
}
