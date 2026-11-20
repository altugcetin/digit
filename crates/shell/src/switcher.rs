use crate::app::App;
use crate::compositor;
use crate::draw::{self, Face};
use crate::paint::Paint;
use gtk::{cairo, gdk, gio, glib, pango, prelude::*};
use gtk4_layer_shell::{KeyboardMode, Layer, LayerShell};
use kit::motion::{self, Tween};
use std::cell::RefCell;
use std::rc::Rc;

const CELL: f64 = 96.0;
const ICON: f64 = 60.0;
const PAD: f64 = 14.0;
const LABEL: f64 = 54.0;
const MAX: usize = 9;
const MARGIN: f64 = 24.0;
const W: f64 = MAX as f64 * CELL + 2.0 * PAD + 2.0 * MARGIN;
const H: f64 = PAD + CELL + LABEL + 2.0 * MARGIN;

struct Item {
    id: u64,
    name: String,
    title: String,
    icon: Option<gdk::Paintable>,
}

struct State {
    app: Rc<App>,
    window: gtk::Window,
    paint: Paint,
    open: bool,
    items: Vec<Item>,
    selected: usize,
    fade: Tween,
    pick: Tween,
    shift: Tween,
    ticking: bool,
}

type Shared = Rc<RefCell<State>>;

thread_local! {
    static ONE: RefCell<Option<Shared>> = const { RefCell::new(None) };
}

pub fn key(app: &Rc<App>, by: i32) {
    let st = ONE.with(|one| one.borrow_mut().get_or_insert_with(|| build(app)).clone());
    let open = st.borrow().open;
    if open {
        step(&st, by);
    } else {
        show(&st, by);
    }
}

fn build(app: &Rc<App>) -> Shared {
    let window = gtk::Window::new();
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace(Some("digit-switcher"));
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(KeyboardMode::None);
    window.add_css_class("digit");
    window.set_default_size(W as i32, H as i32);
    let paint = Paint::new(W as i32, H as i32);
    window.set_child(Some(&paint));
    let st = Rc::new(RefCell::new(State {
        app: app.clone(),
        window: window.clone(),
        paint: paint.clone(),
        open: false,
        items: Vec::new(),
        selected: 0,
        fade: Tween::still(0.0),
        pick: Tween::still(0.0),
        shift: Tween::still(0.0),
        ticking: false,
    }));
    let weak = Rc::downgrade(&st);
    paint.on_draw(move |s| {
        if let Some(st) = weak.upgrade() {
            if let Ok(st) = st.try_borrow() {
                st.draw(s);
            }
        }
    });
    let keys = gtk::EventControllerKey::new();
    let weak = Rc::downgrade(&st);
    keys.connect_key_pressed(move |_, key, _, mods| {
        let Some(st) = weak.upgrade() else { return glib::Propagation::Proceed };
        let back = mods.contains(gdk::ModifierType::SHIFT_MASK) || key == gdk::Key::ISO_Left_Tab;
        match key {
            gdk::Key::Tab | gdk::Key::ISO_Left_Tab => step(&st, if back { -1 } else { 1 }),
            gdk::Key::Right | gdk::Key::Down => step(&st, 1),
            gdk::Key::Left | gdk::Key::Up => step(&st, -1),
            gdk::Key::Escape => close(&st),
            gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::space => commit(&st),
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    let weak = Rc::downgrade(&st);
    keys.connect_key_released(move |_, key, _, _| {
        if matches!(key, gdk::Key::Alt_L | gdk::Key::Alt_R | gdk::Key::Meta_L | gdk::Key::Meta_R) {
            if let Some(st) = weak.upgrade() {
                commit(&st);
            }
        }
    });
    window.add_controller(keys);
    let weak = Rc::downgrade(&st);
    window.connect_is_active_notify(move |w| {
        if !w.is_active() {
            return;
        }
        let weak = weak.clone();
        glib::idle_add_local_once(move || {
            if let (false, Some(st)) = (alt_held(), weak.upgrade()) {
                commit(&st);
            }
        });
    });
    let motion = gtk::EventControllerMotion::new();
    let weak = Rc::downgrade(&st);
    motion.connect_motion(move |_, x, y| {
        if let Some(st) = weak.upgrade() {
            let at = st.borrow().cell_at(x, y);
            if let Some(i) = at {
                select(&st, i);
            }
        }
    });
    paint.add_controller(motion);
    let click = gtk::GestureClick::new();
    let weak = Rc::downgrade(&st);
    click.connect_released(move |_, _, x, y| {
        if let Some(st) = weak.upgrade() {
            let at = st.borrow().cell_at(x, y);
            match at {
                Some(i) => {
                    select(&st, i);
                    commit(&st);
                }
                None => close(&st),
            }
        }
    });
    paint.add_controller(click);
    st
}

fn show(st: &Shared, by: i32) {
    let windows = compositor::windows();
    if windows.is_empty() {
        return;
    }
    let output = compositor::focused_output();
    {
        let mut s = st.borrow_mut();
        let scale = s.window.scale_factor().max(1);
        let apps = s.app.apps.clone();
        s.items = windows
            .into_iter()
            .map(|w| Item { id: w.id, name: apps.display_name(&w.app_id), icon: draw::app_icon(&apps, &w.app_id, ICON as i32, scale), title: w.title })
            .collect();
        let n = s.items.len();
        s.selected = if n < 2 { 0 } else if by < 0 { n - 1 } else { 1 };
        let selected = s.selected;
        s.pick.set(selected as f64);
        s.shift.set(first_for(selected, n));
        s.open = true;
        s.fade.go(1.0, glib::monotonic_time(), 160, motion::STANDARD);
        if !s.window.is_visible() {
            {
                let monitor = crate::screens::for_output(output.as_deref());
                s.window.set_monitor(monitor.as_ref());
            }
        }
        s.window.set_keyboard_mode(KeyboardMode::Exclusive);
    }
    let window = st.borrow().window.clone();
    window.present();
    region(st);
    kick(st);
    let weak = Rc::downgrade(st);
    glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
        let Some(st) = weak.upgrade() else { return glib::ControlFlow::Break };
        let (open, active) = {
            let s = st.borrow();
            (s.open, s.window.is_active())
        };
        if !open {
            return glib::ControlFlow::Break;
        }
        if active && !alt_held() {
            commit(&st);
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
}

fn alt_held() -> bool {
    gdk::Display::default()
        .and_then(|d| d.default_seat())
        .and_then(|s| s.keyboard())
        .is_some_and(|k| k.modifier_state().contains(gdk::ModifierType::ALT_MASK))
}

fn first_for(selected: usize, n: usize) -> f64 {
    if n <= MAX {
        return 0.0;
    }
    (selected as f64 - (MAX as f64 - 1.0) / 2.0).clamp(0.0, (n - MAX) as f64).round()
}

fn select(st: &Shared, i: usize) {
    {
        let mut s = st.borrow_mut();
        if !s.open || i == s.selected || i >= s.items.len() {
            return;
        }
        s.selected = i;
        let now = glib::monotonic_time();
        let first = first_for(i, s.items.len());
        s.pick.go(i as f64, now, 260, motion::EMPHASIZED_DECEL);
        s.shift.go(first, now, 260, motion::EMPHASIZED_DECEL);
    }
    kick(st);
}

fn step(st: &Shared, by: i32) {
    let next = {
        let s = st.borrow();
        let n = s.items.len() as i32;
        if n == 0 {
            return;
        }
        (s.selected as i32 + by).rem_euclid(n) as usize
    };
    select(st, next);
}

fn commit(st: &Shared) {
    let id = {
        let s = st.borrow();
        if !s.open {
            return;
        }
        s.items.get(s.selected).map(|i| i.id)
    };
    if id.is_some() {
        compositor::hold_focus();
    }
    close(st);
    if let Some(id) = id {
        glib::spawn_future_local(async move {
            let _ = gio::spawn_blocking(move || compositor::focus_after_panel(id)).await;
        });
    }
}

fn close(st: &Shared) {
    {
        let mut s = st.borrow_mut();
        if !s.open {
            return;
        }
        s.open = false;
        s.fade.go(0.0, glib::monotonic_time(), 130, motion::FAST_EFFECTS);
        s.window.set_keyboard_mode(KeyboardMode::None);
    }
    region(st);
    kick(st);
}

fn region(st: &Shared) {
    let s = st.borrow();
    let Some(surface) = s.window.surface() else { return };
    let r = if s.open {
        let (x, y, w, h) = s.panel();
        cairo::Region::create_rectangle(&cairo::RectangleInt::new(x as i32, y as i32, w.ceil() as i32, h.ceil() as i32))
    } else {
        cairo::Region::create()
    };
    surface.set_input_region(Some(&r));
}

fn kick(st: &Shared) {
    let paint = {
        let mut s = st.borrow_mut();
        if s.ticking {
            return;
        }
        s.ticking = true;
        s.paint.clone()
    };
    let weak = Rc::downgrade(st);
    paint.add_tick_callback(move |w, clock| {
        let Some(st) = weak.upgrade() else { return glib::ControlFlow::Break };
        let Ok(mut s) = st.try_borrow_mut() else { return glib::ControlFlow::Continue };
        let now = clock.frame_time();
        w.queue_draw();
        let busy = [&s.fade, &s.pick, &s.shift].iter().any(|t| !t.settled(now));
        if busy {
            return glib::ControlFlow::Continue;
        }
        s.ticking = false;
        if !s.open && s.window.is_visible() {
            let window = s.window.clone();
            s.items.clear();
            draw::forget(&s.paint);
            drop(s);
            window.set_visible(false);
            glib::idle_add_local_once(move || {
                let idle = ONE.with(|one| one.borrow().as_ref().is_some_and(|st| st.try_borrow().is_ok_and(|s| !s.open)));
                if idle {
                    let gone = ONE.with(|one| one.borrow_mut().take());
                    window.destroy();
                    drop(gone);
                    App::trim();
                }
            });
            App::trim();
        }
        glib::ControlFlow::Break
    });
}

impl State {
    fn panel(&self) -> (f64, f64, f64, f64) {
        let shown = self.items.len().clamp(1, MAX) as f64;
        let w = (shown * CELL + 2.0 * PAD).max(300.0);
        ((W - w) / 2.0, MARGIN, w, PAD + CELL + LABEL)
    }

    fn cell_at(&self, x: f64, y: f64) -> Option<usize> {
        let (px, py, pw, _) = self.panel();
        if y < py + PAD || y > py + PAD + CELL {
            return None;
        }
        let row = self.items.len().min(MAX) as f64 * CELL;
        let left = px + (pw - row) / 2.0;
        if x < left || x >= left + row {
            return None;
        }
        let i = ((x - left) / CELL + self.shift.target()).floor() as usize;
        (i < self.items.len()).then_some(i)
    }

    fn draw(&self, s: &gtk::Snapshot) {
        let now = self.paint.frame_clock().map(|c| c.frame_time()).unwrap_or_else(glib::monotonic_time);
        let fade = self.fade.value(now).clamp(0.0, 1.0);
        if fade < 0.003 || self.items.is_empty() {
            return;
        }
        let pal = self.app.palette_at(now);
        let cfg = &self.app.cfg;
        let (px, py, pw, ph) = self.panel();
        let shape = draw::round(px, py, pw, ph, 28.0);
        s.push_opacity(fade);
        draw::fill(s, &shape, pal.body);
        s.push_rounded_clip(&shape);
        let row = self.items.len().min(MAX) as f64 * CELL;
        let left = px + (pw - row) / 2.0;
        let shift = self.shift.value(now);
        let hx = left + (self.pick.value(now) - shift) * CELL;
        draw::fill(s, &draw::round(hx + 6.0, py + PAD + 6.0, CELL - 12.0, CELL - 12.0, 22.0), pal.ink_main.alpha(0.13));
        for (i, item) in self.items.iter().enumerate() {
            let cx = left + (i as f64 - shift) * CELL;
            if cx + CELL < px || cx > px + pw {
                continue;
            }
            let (ix, iy) = ((cx + (CELL - ICON) / 2.0).round(), (py + PAD + (CELL - ICON) / 2.0).round());
            match &item.icon {
                Some(p) => draw::paintable(s, p, ix, iy, ICON),
                None => draw::monogram(s, &self.paint, cfg, &item.name, ix, iy, ICON, pal.raised, pal.ink_main),
            }
        }
        if let Some(item) = self.items.get(self.selected) {
            let ly = py + PAD + CELL + 14.0;
            let center = pango::Alignment::Center;
            draw::text(s, &self.paint, cfg, &item.name, 15.0, Face::Text(600), pal.ink_main, px + PAD, ly, pw - 2.0 * PAD, center);
            draw::text(s, &self.paint, cfg, &item.title, 13.0, Face::Text(450), pal.ink_meta, px + 2.0 * PAD, ly + 21.0, pw - 4.0 * PAD, center);
        }
        s.pop();
        s.pop();
    }
}
