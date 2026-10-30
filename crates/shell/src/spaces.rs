use crate::app::App;
use crate::compositor;
use crate::draw::{self, Face};
use crate::paint::Paint;
use gtk::{cairo, gdk, gio, glib, graphene, pango, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kit::motion::{self, Tween};
use kit::shape;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

const CARD_W: f64 = 196.0;
const CARD_H: f64 = 124.0;
const GAP: f64 = 12.0;
const PAD: f64 = 18.0;
const TOP: f64 = 50.0;
const PER_ROW: usize = 4;
const ROWS: usize = 3;
const FILLET: f64 = 14.0;
const RADIUS: f64 = 28.0;
const ICON: f64 = 30.0;
const SURFACE_W: f64 = PER_ROW as f64 * CARD_W + (PER_ROW - 1) as f64 * GAP + 2.0 * PAD + FILLET;
const CHIP_H: f64 = 32.0;
const CHIP_ICON: f64 = 18.0;
const SURFACE_H: f64 = TOP + ROWS as f64 * CARD_H + (ROWS - 1) as f64 * GAP + GAP + CHIP_H + PAD + FILLET;

struct Card {
    id: u64,
    idx: u64,
    active: bool,
    windows: Vec<(u64, String, Option<gdk::Paintable>)>,
    title: String,
}

/// A special workspace, shown as a small chip under the cards.
struct Chip {
    name: String,
    shown: bool,
    icons: Vec<(String, Option<gdk::Paintable>)>,
}

#[derive(Clone, Copy, PartialEq)]
enum Spot {
    Card(usize),
    Window(usize, usize),
    Special(usize),
}

struct State {
    app: Rc<App>,
    window: gtk::Window,
    paint: Paint,
    output: String,
    open: bool,
    inside: bool,
    leave: u64,
    scrim: Option<gtk::Window>,
    cards: Vec<Card>,
    chips: Vec<Chip>,
    hot: Option<Spot>,
    glow: Vec<Tween>,
    rise: Tween,
    fade: Tween,
    ticking: bool,
}

type Shared = Rc<RefCell<State>>;

thread_local! {
    static ONE: RefCell<Option<Shared>> = const { RefCell::new(None) };
}

fn get(app: &Rc<App>) -> Shared {
    ONE.with(|one| one.borrow_mut().get_or_insert_with(|| build(app)).clone())
}

pub fn toggle(app: &Rc<App>, output: Option<String>) {
    let st = get(app);
    let open = st.borrow().open;
    if open {
        close(&st);
    } else {
        show(&st, output.or_else(compositor::focused_output));
    }
}

pub fn changed(_app: &App) {
    let st = ONE.with(|one| one.borrow().clone());
    if let Some(st) = st {
        let open = st.borrow().open;
        if open {
            load(&st);
            st.borrow().paint.queue_draw();
        }
    }
}

pub fn corner(app: &Rc<App>, monitor: &gdk::Monitor) -> gtk::Window {
    let window = gtk::Window::new();
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace(Some("digit-corner"));
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Left, true);
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(KeyboardMode::None);
    window.set_monitor(Some(monitor));
    window.add_css_class("digit");
    window.set_default_size(2, 2);
    let paint = Paint::new(2, 2);
    paint.present();
    window.set_child(Some(&paint));
    let motion = gtk::EventControllerMotion::new();
    let weak = Rc::downgrade(app);
    let output = monitor.connector().map(|c| c.to_string());
    motion.connect_enter(move |_, _, _| {
        if let Some(app) = weak.upgrade() {
            if app.prefs.borrow().hot_corner && output.as_deref().is_none_or(|o| app.edges_live(o)) {
                toggle(&app, output.clone());
            }
        }
    });
    paint.add_controller(motion);
    window.present();
    window
}

fn build(app: &Rc<App>) -> Shared {
    let window = gtk::Window::new();
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace(Some("digit-spaces"));
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Left, true);
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(KeyboardMode::None);
    window.add_css_class("digit");
    window.set_default_size(SURFACE_W as i32, SURFACE_H as i32);
    let paint = Paint::new(SURFACE_W as i32, SURFACE_H as i32);
    window.set_child(Some(&paint));
    let st = Rc::new(RefCell::new(State {
        app: app.clone(),
        window: window.clone(),
        paint: paint.clone(),
        output: String::new(),
        open: false,
        inside: false,
        leave: 0,
        scrim: None,
        cards: Vec::new(),
        chips: Vec::new(),
        hot: None,
        glow: Vec::new(),
        rise: Tween::still(0.0),
        fade: Tween::still(0.0),
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
    keys.connect_key_pressed(move |_, key, _, _| {
        let Some(st) = weak.upgrade() else { return glib::Propagation::Proceed };
        match key {
            gdk::Key::Escape => close(&st),
            gdk::Key::Right | gdk::Key::Tab => nudge(&st, 1),
            gdk::Key::Left | gdk::Key::ISO_Left_Tab => nudge(&st, -1),
            gdk::Key::Down => nudge(&st, PER_ROW as i32),
            gdk::Key::Up => nudge(&st, -(PER_ROW as i32)),
            gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::space => {
                let hot = st.borrow().hot;
                if let Some(spot) = hot {
                    pick(&st, spot);
                }
            }
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    window.add_controller(keys);
    let weak = Rc::downgrade(&st);
    window.connect_is_active_notify(move |w| {
        let leave = weak.upgrade().is_some_and(|st| st.try_borrow().is_ok_and(|s| s.app.prefs.borrow().spaces_close == "leave"));
        if !w.is_active() && leave {
            let weak = weak.clone();
            glib::idle_add_local_once(move || {
                if let Some(st) = weak.upgrade() {
                    close(&st);
                }
            });
        }
    });
    let motion = gtk::EventControllerMotion::new();
    let weak = Rc::downgrade(&st);
    motion.connect_motion(move |_, x, y| {
        let Some(st) = weak.upgrade() else { return };
        let Ok(mut s) = st.try_borrow_mut() else { return };
        let at = s.spot_at(x, y);
        s.inside = true;
        drop(s);
        hover(&st, at);
    });
    let weak = Rc::downgrade(&st);
    motion.connect_leave(move |_| {
        let Some(st) = weak.upgrade() else { return };
        let generation = {
            let Ok(mut s) = st.try_borrow_mut() else { return };
            s.leave += 1;
            s.inside = false;
            s.leave
        };
        hover(&st, None);
        let weak = Rc::downgrade(&st);
        glib::timeout_add_local_once(Duration::from_millis(crate::app::LEAVE_MS), move || {
            if let Some(st) = weak.upgrade() {
                let gone = {
                    let s = st.borrow();
                    s.leave == generation && !s.inside && s.app.prefs.borrow().spaces_close == "leave"
                };
                if gone {
                    close(&st);
                }
            }
        });
    });
    paint.add_controller(motion);
    let click = gtk::GestureClick::new();
    let weak = Rc::downgrade(&st);
    click.connect_released(move |_, _, x, y| {
        if let Some(st) = weak.upgrade() {
            let at = st.borrow().spot_at(x, y);
            match at {
                Some(spot) => pick(&st, spot),
                None => close(&st),
            }
        }
    });
    paint.add_controller(click);
    st
}

fn load(st: &Shared) {
    let windows = compositor::windows();
    let specials = kit::wm::specials();
    let mut s = st.borrow_mut();
    let scale = s.window.scale_factor().max(1);
    let apps = s.app.apps.clone();
    let spaces: Vec<compositor::Space> = s.app.spaces.borrow().iter().filter(|sp| sp.output == s.output).cloned().collect();
    let last = spaces.iter().rposition(|sp| sp.filled || sp.active);
    s.cards = spaces
        .iter()
        .enumerate()
        .filter(|(i, sp)| sp.filled || sp.active || Some(*i) == last.map(|l| l + 1))
        .take(PER_ROW * ROWS)
        .map(|(_, sp)| {
            let mine: Vec<&compositor::Window> = windows.iter().filter(|w| w.space == sp.id).collect();
            Card {
                id: sp.id,
                idx: sp.idx,
                active: sp.active,
                title: mine.first().map(|w| w.title.clone()).unwrap_or_default(),
                windows: mine.iter().map(|w| (w.id, w.app_id.clone(), draw::app_icon(&apps, &w.app_id, ICON as i32, scale))).collect(),
            }
        })
        .collect();
    let output = s.output.clone();
    s.chips = specials
        .into_iter()
        .filter(|sp| sp.output == output)
        .take(PER_ROW)
        .map(|sp| Chip { name: sp.name, shown: sp.shown, icons: sp.apps.iter().take(3).map(|a| (a.clone(), draw::app_icon(&apps, a, CHIP_ICON as i32, scale))).collect() })
        .collect();
    if s.hot.is_some_and(|h| matches!(h, Spot::Special(k) if k >= s.chips.len())) {
        s.hot = None;
    }
    let n = s.cards.len();
    s.glow.resize(n, Tween::still(0.0));
    if s.hot.is_some_and(|h| matches!(h, Spot::Card(i) | Spot::Window(i, _) if i >= n)) {
        s.hot = None;
    }
}

fn show(st: &Shared, output: Option<String>) {
    {
        let mut s = st.borrow_mut();
        let monitors: Vec<gdk::Monitor> = gdk::Display::default().map(|d| d.monitors().iter::<gdk::Monitor>().filter_map(Result::ok).collect()).unwrap_or_default();
        let monitor = monitors.iter().find(|m| output.as_deref().is_some_and(|o| m.connector().as_deref() == Some(o))).or(monitors.first()).cloned();
        s.output = monitor.as_ref().and_then(|m| m.connector()).map(|c| c.to_string()).unwrap_or_default();
        if !s.window.is_visible() {
            s.window.set_monitor(monitor.as_ref());
        }
        s.open = true;
        s.inside = false;
        s.hot = None;
        let now = glib::monotonic_time();
        s.rise.go(1.0, now, 420, motion::Curve::new(0.16, 1.0, 0.3, 1.0));
        s.fade.after(1.0, now, 60, 220, motion::STANDARD);
        s.window.set_keyboard_mode(KeyboardMode::OnDemand);
        if s.app.prefs.borrow().spaces_close == "click" && s.scrim.is_none() {
            let weak = Rc::downgrade(st);
            s.scrim = Some(crate::paint::scrim(monitor.as_ref(), move || {
                if let Some(st) = weak.upgrade() {
                    close(&st);
                }
            }));
        }
    }
    load(st);
    {
        let mut s = st.borrow_mut();
        let active = s.cards.iter().position(|c| c.active);
        s.hot = active.map(Spot::Card);
        for (i, g) in s.glow.iter_mut().enumerate() {
            g.set(if Some(i) == active { 1.0 } else { 0.0 });
        }
    }
    let window = st.borrow().window.clone();
    window.present();
    region(st);
    kick(st);
}

fn close(st: &Shared) {
    {
        let mut s = st.borrow_mut();
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
    region(st);
    kick(st);
}

fn pick(st: &Shared, spot: Spot) {
    let target = {
        let s = st.borrow();
        match spot {
            Spot::Card(i) => s.cards.get(i).map(|c| (true, c.id)),
            Spot::Window(i, j) => s.cards.get(i).and_then(|c| c.windows.get(j)).map(|w| (false, w.0)),
            Spot::Special(k) => {
                let name = s.chips.get(k).map(|c| c.name.clone());
                drop(s);
                close(st);
                if let Some(name) = name {
                    glib::spawn_future_local(async move {
                        let _ = gio::spawn_blocking(move || kit::wm::toggle_special(&name)).await;
                    });
                }
                return;
            }
        }
    };
    close(st);
    if let Some((space, id)) = target {
        glib::spawn_future_local(async move {
            let _ = gio::spawn_blocking(move || if space { compositor::focus_space(id) } else { compositor::focus(id) }).await;
        });
    }
}

fn hover(st: &Shared, at: Option<Spot>) {
    {
        let mut s = st.borrow_mut();
        if s.hot == at {
            return;
        }
        s.hot = at;
        let card = at.and_then(|a| match a {
            Spot::Card(i) | Spot::Window(i, _) => Some(i),
            Spot::Special(_) => None,
        });
        let now = glib::monotonic_time();
        for (i, g) in s.glow.iter_mut().enumerate() {
            let on = card == Some(i);
            g.go(if on { 1.0 } else { 0.0 }, now, if on { 160 } else { 260 }, motion::STANDARD);
        }
        let pointer = matches!(at, Some(_));
        s.paint.set_cursor_from_name(pointer.then_some("pointer"));
    }
    st.borrow().paint.queue_draw();
    kick(st);
}

fn nudge(st: &Shared, by: i32) {
    let next = {
        let s = st.borrow();
        let n = s.cards.len() as i32;
        if n == 0 {
            return;
        }
        let at = match s.hot {
            Some(Spot::Card(i)) | Some(Spot::Window(i, _)) => i as i32,
            Some(Spot::Special(_)) | None => 0,
        };
        (at + by).clamp(0, n - 1) as usize
    };
    hover(st, Some(Spot::Card(next)));
}

fn region(st: &Shared) {
    let s = st.borrow();
    let Some(surface) = s.window.surface() else { return };
    let r = if s.open {
        let (w, h) = s.size();
        cairo::Region::create_rectangle(&cairo::RectangleInt::new(0, 0, (w + FILLET).ceil() as i32, (h + FILLET).ceil() as i32))
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
        let busy = [&s.rise, &s.fade].into_iter().chain(s.glow.iter()).any(|t| !t.settled(now));
        if busy {
            return glib::ControlFlow::Continue;
        }
        s.ticking = false;
        if !s.open && s.window.is_visible() {
            let window = s.window.clone();
            s.cards.clear();
            s.glow.clear();
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
    fn size(&self) -> (f64, f64) {
        let n = self.cards.len().max(1);
        let cols = n.min(PER_ROW) as f64;
        let rows = n.div_ceil(PER_ROW) as f64;
        let chips = if self.chips.is_empty() { 0.0 } else { GAP + CHIP_H };
        (cols * CARD_W + (cols - 1.0) * GAP + 2.0 * PAD, TOP + rows * CARD_H + (rows - 1.0) * GAP + chips + PAD)
    }

    fn chip_rect(&self, k: usize) -> (f64, f64, f64) {
        let (w, _) = self.size();
        let n = self.chips.len().max(1) as f64;
        let cw = ((w - 2.0 * PAD - (n - 1.0) * GAP) / n).min(CARD_W);
        let rows = self.cards.len().max(1).div_ceil(PER_ROW) as f64;
        (PAD + k as f64 * (cw + GAP), TOP + rows * CARD_H + (rows - 1.0) * GAP + GAP, cw)
    }

    fn card_rect(&self, i: usize) -> (f64, f64) {
        let (c, r) = ((i % PER_ROW) as f64, (i / PER_ROW) as f64);
        (PAD + c * (CARD_W + GAP), TOP + r * (CARD_H + GAP))
    }

    fn icon_rect(&self, i: usize, j: usize) -> (f64, f64) {
        let (x, y) = self.card_rect(i);
        (x + 14.0 + j as f64 * (ICON + 8.0), y + 44.0)
    }

    fn spot_at(&self, x: f64, y: f64) -> Option<Spot> {
        for k in 0..self.chips.len() {
            let (cx, cy, cw) = self.chip_rect(k);
            if x >= cx && x <= cx + cw && y >= cy && y <= cy + CHIP_H {
                return Some(Spot::Special(k));
            }
        }
        for (i, card) in self.cards.iter().enumerate() {
            let (cx, cy) = self.card_rect(i);
            if x < cx || x > cx + CARD_W || y < cy || y > cy + CARD_H {
                continue;
            }
            for j in 0..card.windows.len().min(4) {
                let (ix, iy) = self.icon_rect(i, j);
                if x >= ix - 3.0 && x <= ix + ICON + 3.0 && y >= iy - 3.0 && y <= iy + ICON + 3.0 {
                    return Some(Spot::Window(i, j));
                }
            }
            return Some(Spot::Card(i));
        }
        None
    }

    fn draw(&self, s: &gtk::Snapshot) {
        let now = self.paint.frame_clock().map(|c| c.frame_time()).unwrap_or_else(glib::monotonic_time);
        let fade = self.fade.value(now).clamp(0.0, 1.0);
        let rise = self.rise.value(now);
        if fade < 0.003 && rise < 0.003 {
            return;
        }
        let pal = self.app.palette_at(now);
        let cfg = &self.app.cfg;
        let (w, h) = self.size();
        let lift = ((1.0 - rise) * (h + FILLET)).round();
        s.save();
        s.translate(&graphene::Point::new(0.0, -lift as f32));
        s.save();
        s.translate(&graphene::Point::new((w + FILLET) as f32, (h + FILLET) as f32));
        s.scale(-1.0, -1.0);
        let parts = shape::cornered_parts(w + FILLET, h + FILLET, w, h, RADIUS, FILLET);
        if let Some(p) = &parts {
            draw::slab(s, p, pal.body.alpha(1.0));
        }
        s.restore();
        s.push_rounded_clip(&gtk::gsk::RoundedRect::new(
            graphene::Rect::new(0.0, 0.0, w as f32, h as f32),
            graphene::Size::zero(),
            graphene::Size::zero(),
            graphene::Size::new(RADIUS as f32, RADIUS as f32),
            graphene::Size::zero(),
        ));
        s.push_opacity(fade);
        let left = pango::Alignment::Left;
        draw::text(s, &self.paint, cfg, "Workspaces", 16.0, Face::Text(600), pal.ink_main, PAD + 2.0, 27.0, 200.0, left);
        draw::text(s, &self.paint, cfg, &self.output, 13.0, Face::Text(450), pal.ink_faint, w - PAD - 200.0, 27.0, 200.0, pango::Alignment::Right);
        for (i, card) in self.cards.iter().enumerate() {
            let (x, y) = self.card_rect(i);
            let glow = self.glow.get(i).map_or(0.0, |g| g.value(now).clamp(0.0, 1.0));
            let base = if card.active { 0.15 } else { 0.08 };
            draw::fill(s, &draw::round(x, y, CARD_W, CARD_H, 20.0), pal.ink_main.alpha(base + 0.06 * glow));
            let ink = if card.active { pal.accent } else { pal.ink_main };
            draw::text(s, &self.paint, cfg, &card.idx.to_string(), 17.0, Face::Number(600), ink, x + 14.0, y + 22.0, 40.0, left);
            let count = match card.windows.len() {
                0 => "Empty".to_string(),
                1 => "1 window".to_string(),
                n => format!("{n} windows"),
            };
            draw::text(s, &self.paint, cfg, &count, 12.0, Face::Text(450), pal.ink_meta, x + CARD_W - 114.0, y + 22.0, 100.0, pango::Alignment::Right);
            if card.windows.is_empty() {
                draw::icon(s, &self.paint, cfg, "add", 26.0, false, pal.ink_faint, x + CARD_W / 2.0, y + CARD_H / 2.0 + 10.0);
                continue;
            }
            for (j, (_, app_id, icon)) in card.windows.iter().take(4).enumerate() {
                let (ix, iy) = self.icon_rect(i, j);
                if self.hot == Some(Spot::Window(i, j)) {
                    draw::fill(s, &draw::round(ix - 4.0, iy - 4.0, ICON + 8.0, ICON + 8.0, 10.0), pal.ink_main.alpha(0.12));
                }
                match icon {
                    Some(p) => draw::paintable(s, p, ix, iy, ICON),
                    None => draw::monogram(s, &self.paint, cfg, &crate::apps::pretty(app_id), ix, iy, ICON, pal.raised, pal.ink_main),
                }
            }
            if card.windows.len() > 4 {
                let (ix, iy) = self.icon_rect(i, 4);
                draw::text(s, &self.paint, cfg, &format!("+{}", card.windows.len() - 4), 13.0, Face::Number(500), pal.ink_meta, ix, iy + ICON / 2.0, 30.0, left);
            }
            draw::text(s, &self.paint, cfg, &card.title, 12.0, Face::Text(450), pal.ink_meta, x + 14.0, y + CARD_H - 20.0, CARD_W - 28.0, left);
        }
        for (k, chip) in self.chips.iter().enumerate() {
            let (x, y, cw) = self.chip_rect(k);
            let hot = self.hot == Some(Spot::Special(k));
            let base = if chip.shown { 0.15 } else { 0.08 };
            draw::fill(s, &draw::round(x, y, cw, CHIP_H, CHIP_H / 2.0), pal.ink_main.alpha(base + if hot { 0.06 } else { 0.0 }));
            let ink = if chip.shown { pal.accent } else { pal.ink_main };
            draw::icon(s, &self.paint, cfg, "stacks", 16.0, false, ink, x + 22.0, y + CHIP_H / 2.0);
            let room = cw - 40.0 - chip.icons.len() as f64 * (CHIP_ICON + 4.0) - 12.0;
            draw::text(s, &self.paint, cfg, &chip.name, 12.5, Face::Text(550), ink, x + 36.0, y + CHIP_H / 2.0, room.max(0.0), left);
            for (j, (app_id, icon)) in chip.icons.iter().enumerate() {
                let ix = x + cw - 12.0 - (chip.icons.len() - j) as f64 * (CHIP_ICON + 4.0) + 4.0;
                let iy = y + (CHIP_H - CHIP_ICON) / 2.0;
                match icon {
                    Some(p) => draw::paintable(s, p, ix, iy, CHIP_ICON),
                    None => draw::monogram(s, &self.paint, cfg, &crate::apps::pretty(app_id), ix, iy, CHIP_ICON, pal.raised, pal.ink_main),
                }
            }
        }
        s.pop();
        s.pop();
        s.restore();
    }
}
