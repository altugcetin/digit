use crate::app::App;
use crate::compositor;
use crate::draw::{self, Face};
use crate::paint::Paint;
use crate::panel_corner;
use gtk::{cairo, gdk, gio, glib, graphene, pango, prelude::*};
use gtk4_layer_shell::{KeyboardMode, Layer, LayerShell};
use kit::corners::{Corner, Panel};
use kit::motion::{self, Tween};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

const CARD_W: f64 = 196.0;
const CARD_H: f64 = 124.0;
use crate::metrics::{FILLET, GRID_GAP as GAP, PANEL_PAD as PAD, PANEL_RADIUS as RADIUS};
const TOP: f64 = 50.0;
const PER_ROW: usize = 4;
const ROWS: usize = 3;
const ICON: f64 = 30.0;
const SURFACE_W: f64 = PER_ROW as f64 * CARD_W + (PER_ROW - 1) as f64 * GAP + 2.0 * PAD + FILLET;
const CHIP_H: f64 = 32.0;
const CHIP_ICON: f64 = 18.0;
const SURFACE_H: f64 = TOP + ROWS as f64 * CARD_H + (ROWS - 1) as f64 * GAP + GAP + CHIP_H + PAD + FILLET;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Id(u64),
    Index(u64),
}

#[derive(Debug, PartialEq)]
struct CardPlan {
    target: Target,
    idx: u64,
    active: bool,
}

// Select by priority before sorting for display: used/active, trailing, then pins.
fn card_list(spaces: &[compositor::Space], output: &str, niri: bool, always: i64, cap: usize) -> Vec<CardPlan> {
    let mut local: Vec<_> = spaces.iter().filter(|s| s.output == output).collect();
    local.sort_by_key(|s| s.idx);
    let mut cards: Vec<_> = local.iter().filter(|s| s.filled || s.active).map(|s| CardPlan { target: Target::Id(s.id), idx: s.idx, active: s.active }).collect();
    let mut trailing = local.iter().filter(|s| s.filled || s.active).map(|s| s.idx).max().unwrap_or(0).saturating_add(1);
    if niri {
        // The last existing workspace is niri's own empty slot, even when active.
        if let Some(s) = local.last().filter(|s| !s.filled) { trailing = s.idx; }
    } else {
        // Hyprland numbers are global: never borrow a number owned by another output.
        while spaces.iter().any(|s| s.idx == trailing && s.output != output) {
            let Some(next) = trailing.checked_add(1) else { break };
            trailing = next;
        }
    }
    for idx in std::iter::once(trailing).chain(1..=always.clamp(0, 10) as u64) {
        if cards.iter().any(|c| c.idx == idx) || (!niri && spaces.iter().any(|s| s.idx == idx && s.output != output)) { continue; }
        let existing = local.iter().find(|s| s.idx == idx);
        cards.push(CardPlan { target: existing.map_or(Target::Index(idx), |s| Target::Id(s.id)), idx, active: existing.is_some_and(|s| s.active) });
    }
    cards.truncate(cap);
    cards.sort_by_key(|c| c.idx);
    cards
}

struct Card {
    target: Target,
    idx: u64,
    active: bool,
    windows: Vec<(u64, String, Option<gdk::Paintable>)>,
    title: String,
}

/// A window icon being dragged onto another workspace card.
struct Drag {
    space: Target,
    window: u64,
    at: (f64, f64),
    moving: bool,
}

fn drag_window<'a>(cards: &'a [Card], drag: &Drag) -> Option<&'a (u64, String, Option<gdk::Paintable>)> {
    cards.iter().find(|c| c.target == drag.space)?.windows.iter().find(|w| w.0 == drag.window)
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
    frost: crate::blur::Frost,
    output: String,
    corner: Corner,
    margins: [i32; 4],
    open: bool,
    inside: bool,
    leave: u64,
    scrim: Option<gtk::Window>,
    cards: Vec<Card>,
    chips: Vec<Chip>,
    drag: Option<Drag>,
    /// Set when a drag ends so the click that follows it does not also pick.
    dropped: bool,
    hot: Option<Spot>,
    glow: Vec<Tween>,
    rise: Tween,
    fade: Tween,
    ticking: bool,
    /// The workspace Super+Tab moved to last while the panel is open.
    cursor: Option<u64>,
}

type Shared = Rc<RefCell<State>>;

thread_local! {
    static ONE: RefCell<Option<Shared>> = const { RefCell::new(None) };
}

fn get(app: &Rc<App>) -> Shared {
    ONE.with(|one| one.borrow_mut().get_or_insert_with(|| build(app)).clone())
}

/// Super+Tab: the first press opens the panel; each press while it is open goes to the next
/// workspace that is in use, wrapping around to the first.
pub fn cycle(app: &Rc<App>) {
    let st = get(app);
    if !st.borrow().open {
        show(&st, compositor::focused_output());
        return;
    }
    let next = {
        let mut s = st.borrow_mut();
        let used: Vec<(u64, u64)> = s.cards.iter().filter_map(|c| match c.target {
            Target::Id(id) if c.active || !c.windows.is_empty() => Some((c.idx, id)),
            _ => None,
        }).collect();
        // Presses can come faster than the compositor reports the switch, so step from the
        // card picked last rather than from the one it still calls active.
        let at = s.cursor.or_else(|| s.cards.iter().find(|c| c.active).map(|c| c.idx));
        let next = next_used(&used, at);
        if let Some((idx, _)) = next {
            s.cursor = Some(idx);
            for c in s.cards.iter_mut() {
                c.active = c.idx == idx;
            }
        }
        next
    };
    if let Some((_, id)) = next {
        compositor::focus_space(id);
        st.borrow().paint.queue_draw();
    }
}

/// The workspace after `at` among `used` (idx, id) pairs sorted by idx, wrapping around.
fn next_used(used: &[(u64, u64)], at: Option<u64>) -> Option<(u64, u64)> {
    let at = at?;
    used.iter().copied().find(|&(idx, _)| idx > at).or_else(|| used.first().copied()).filter(|&(idx, _)| idx != at)
}

pub fn changed(_app: &App) {
    let st = ONE.with(|one| one.borrow().clone());
    if let Some(st) = st {
        let open = st.borrow().open;
        if open {
            load(&st);
            region(&st);
            st.borrow().paint.queue_draw();
        }
    }
}

pub fn show_on(app: &Rc<App>, output: Option<String>) {
    let st = get(app);
    if !st.borrow().open {
        show(&st, output);
    }
}

pub fn reposition() {
    if let Some(st) = ONE.with(|one| one.borrow().clone()) {
        {
            let mut s = st.borrow_mut();
            s.reposition();
        }
        region(&st);
        st.borrow().paint.queue_draw();
    }
}

fn build(app: &Rc<App>) -> Shared {
    let window = gtk::Window::new();
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace(Some("digit-spaces"));
    panel_corner::anchor(&window, panel_corner::selected(app, Panel::Workspaces));
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(KeyboardMode::None);
    window.add_css_class("digit");
    window.set_default_size(SURFACE_W as i32, SURFACE_H as i32);
    let paint = Paint::new(SURFACE_W as i32, SURFACE_H as i32);
    let (frosted, frost) = crate::blur::Frost::wrap(&paint);
    window.set_child(Some(&frosted));
    let st = Rc::new(RefCell::new(State {
        app: app.clone(),
        window: window.clone(),
        paint: paint.clone(),
        frost,
        output: String::new(),
        corner: panel_corner::selected(app, Panel::Workspaces),
        margins: [0; 4],
        open: false,
        cursor: None,
        inside: false,
        leave: 0,
        scrim: None,
        cards: Vec::new(),
        chips: Vec::new(),
        drag: None,
        dropped: false,
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
            if std::mem::take(&mut st.borrow_mut().dropped) {
                return;
            }
            let at = st.borrow().spot_at(x, y);
            match at {
                Some(spot) => pick(&st, spot),
                None => close(&st),
            }
        }
    });
    paint.add_controller(click);
    let drag = gtk::GestureDrag::new();
    let weak = Rc::downgrade(&st);
    drag.connect_drag_begin(move |_, x, y| {
        let Some(st) = weak.upgrade() else { return };
        let mut s = st.borrow_mut();
        s.drag = match s.spot_at(x, y) {
            Some(Spot::Window(card, win)) => Some(Drag { space: s.cards[card].target, window: s.cards[card].windows[win].0, at: (x, y), moving: false }),
            _ => None,
        };
    });
    let weak = Rc::downgrade(&st);
    drag.connect_drag_update(move |g, dx, dy| {
        let (Some(st), Some((x, y))) = (weak.upgrade(), g.start_point()) else { return };
        let target = {
            let mut s = st.borrow_mut();
            let Some(d) = s.drag.as_mut() else { return };
            d.at = (x + dx, y + dy);
            if !d.moving && dx.hypot(dy) < 6.0 {
                return;
            }
            d.moving = true;
            let at = d.at;
            s.spot_at(at.0, at.1).and_then(|sp| match sp {
                Spot::Card(i) | Spot::Window(i, _) => Some(Spot::Card(i)),
                Spot::Special(_) => None,
            })
        };
        hover(&st, target);
        st.borrow().paint.queue_draw();
    });
    let weak = Rc::downgrade(&st);
    drag.connect_drag_end(move |_, _, _| {
        let Some(st) = weak.upgrade() else { return };
        let moved = {
            let mut s = st.borrow_mut();
            let Some(d) = s.drag.take() else { return };
            if !d.moving {
                return;
            }
            s.dropped = true;
            let to = s.spot_at(d.at.0, d.at.1).and_then(|sp| match sp {
                Spot::Card(i) | Spot::Window(i, _) => Some(i),
                Spot::Special(_) => None,
            });
            let win = drag_window(&s.cards, &d).map(|w| w.0);
            match (to, win) {
                (Some(to), Some(win)) if s.cards[to].target != d.space => s.cards.get(to).map(|c| (win, c.target)),
                _ => None,
            }
        };
        st.borrow().paint.queue_draw();
        if let Some((win, space)) = moved {
            let output = st.borrow().output.clone();
            let weak = Rc::downgrade(&st);
            glib::spawn_future_local(async move {
                let _ = gio::spawn_blocking(move || match space {
                    Target::Id(id) => kit::wm::move_to_space(win, id),
                    Target::Index(idx) => kit::wm::move_to_space_index(win, idx, &output),
                }).await;
                glib::timeout_add_local_once(Duration::from_millis(120), move || {
                    if let Some(st) = weak.upgrade() {
                        load(&st);
                        st.borrow().paint.queue_draw();
                    }
                });
            });
        }
    });
    paint.add_controller(drag);
    st
}

fn load(st: &Shared) {
    let windows = compositor::windows();
    let specials = kit::wm::specials();
    let mut s = st.borrow_mut();
    let scale = s.window.scale_factor().max(1);
    let apps = s.app.apps.clone();
    let always = s.app.prefs.borrow().spaces_always;
    let plans = card_list(&s.app.spaces.borrow(), &s.output, kit::wm::which() == kit::wm::Wm::Niri, always, PER_ROW * ROWS);
    s.cards = plans.into_iter().map(|plan| {
        let mine: Vec<&compositor::Window> = windows.iter().filter(|w| plan.target == Target::Id(w.space)).collect();
        Card {
            target: plan.target,
            idx: plan.idx,
            active: plan.active,
            title: mine.first().map(|w| w.title.clone()).unwrap_or_default(),
            windows: mine.iter().map(|w| (w.id, w.app_id.clone(), draw::app_icon(&apps, &w.app_id, ICON as i32, scale))).collect(),
        }
    }).collect();
    if s.drag.as_ref().is_some_and(|d| drag_window(&s.cards, d).is_none()) {
        s.drag = None;
        s.dropped = true;
    }
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
        s.reposition();
        s.open = true;
        s.cursor = None;
        s.inside = false;
        s.leave += 1;
        s.hot = None;
        let now = glib::monotonic_time();
        s.rise.go(1.0, now, 420, motion::Curve::new(0.16, 1.0, 0.3, 1.0));
        s.fade.after(1.0, now, 60, 220, motion::STANDARD);
        // As with the menu bar's menus, keep the keyboard here: Hyprland's
        // focus-follows-mouse must not dismiss a panel opened by a shortcut.
        s.window.set_keyboard_mode(KeyboardMode::Exclusive);
        if s.scrim.is_none() {
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
    let output = st.borrow().output.clone();
    let target = {
        let s = st.borrow();
        match spot {
            Spot::Card(i) => s.cards.get(i).map(|c| (Some(c.target), 0)),
            Spot::Window(i, j) => s.cards.get(i).and_then(|c| c.windows.get(j)).map(|w| (None, w.0)),
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
            let _ = gio::spawn_blocking(move || match space {
                Some(Target::Id(id)) => compositor::focus_space(id),
                Some(Target::Index(idx)) => { kit::wm::focus_space_index(idx, &output); },
                None => compositor::focus_after_panel(id),
            }).await;
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
    surface.set_input_region(Some(&s.input_region(glib::monotonic_time())));
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
        if let Some(surface) = s.window.surface() {
            surface.set_input_region(Some(&s.input_region(now)));
        }
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
    fn reposition(&mut self) {
        self.corner = panel_corner::selected(&self.app, Panel::Workspaces);
        self.margins = panel_corner::position(&self.window, self.corner);
        let (w, h) = (SURFACE_W as i32, SURFACE_H as i32);
        self.window.set_default_size(w, h);
        self.paint.set_size_request(w, h);
    }

    fn surface_size(&self) -> (f64, f64) {
        let (w, h) = (self.paint.width(), self.paint.height());
        (if w > 1 { w as f64 } else { SURFACE_W }, if h > 1 { h as f64 } else { SURFACE_H })
    }

    fn placement(&self, now: i64) -> panel_corner::Rect {
        let (w, h) = self.size();
        panel_corner::placement(self.corner, self.surface_size(), (w, h), self.rise.value(now), FILLET)
    }

    fn input_region(&self, now: i64) -> cairo::Region {
        if self.open {
            cairo::Region::create_rectangle(&panel_corner::envelope(self.corner, self.placement(now), self.surface_size(), FILLET).region())
        } else {
            cairo::Region::create()
        }
    }

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
        let rect = self.placement(glib::monotonic_time());
        if !rect.contains(x, y) {
            return None;
        }
        let (x, y) = (x - rect.x, y - rect.y);
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
            self.frost.set(None, 0.0, self.surface_size());
            return;
        }
        let pal = self.app.palette_at(now);
        let cfg = &self.app.cfg;
        let (w, _) = self.size();
        let rect = self.placement(now);
        let radius = RADIUS * self.app.round();
        self.frost.set_corners((fade > 0.02).then_some((rect.x, rect.y, rect.w, rect.h)), panel_corner::radii(self.corner, radius), self.surface_size());
        s.save();
        panel_corner::slab(s, self.corner, rect, radius, FILLET, crate::app::panel(pal.body));
        s.push_rounded_clip(&panel_corner::clip(self.corner, rect, radius, FILLET));
        s.translate(&graphene::Point::new(rect.x as f32, rect.y as f32));
        s.push_opacity(fade);
        let left = pango::Alignment::Left;
        draw::text(s, &self.paint, cfg, "Workspaces", 16.0, Face::Text(600), pal.ink_main, PAD + 2.0, 27.0, 200.0, left);
        draw::text(s, &self.paint, cfg, &self.output, 13.0, Face::Text(450), pal.ink_faint, w - PAD - 200.0, 27.0, 200.0, pango::Alignment::Right);
        for (i, card) in self.cards.iter().enumerate() {
            let (x, y) = self.card_rect(i);
            let glow = self.glow.get(i).map_or(0.0, |g| g.value(now).clamp(0.0, 1.0));
            let base = if card.active { 0.15 } else { 0.08 };
            draw::fill(s, &draw::round(x, y, CARD_W, CARD_H, 20.0 * self.app.round()), pal.ink_main.alpha(base + 0.06 * glow));
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
            for (j, (id, app_id, icon)) in card.windows.iter().take(4).enumerate() {
                let (ix, iy) = self.icon_rect(i, j);
                if self.hot == Some(Spot::Window(i, j)) {
                    draw::fill(s, &draw::round(ix - 4.0, iy - 4.0, ICON + 8.0, ICON + 8.0, 10.0), pal.ink_main.alpha(0.12));
                }
                let lifted = self.drag.as_ref().is_some_and(|d| d.moving && d.space == card.target && d.window == *id);
                if lifted {
                    s.push_opacity(0.3);
                }
                match icon {
                    Some(p) => draw::paintable(s, p, ix, iy, ICON),
                    None => draw::monogram(s, &self.paint, cfg, &crate::apps::pretty(app_id), ix, iy, ICON, pal.raised, pal.ink_main),
                }
                if lifted {
                    s.pop();
                }
            }
            if card.windows.len() > 4 {
                let (ix, iy) = self.icon_rect(i, 4);
                draw::text(s, &self.paint, cfg, &format!("+{}", card.windows.len() - 4), 13.0, Face::Number(500), pal.ink_meta, ix, iy + ICON / 2.0, 30.0, left);
            }
            draw::text(s, &self.paint, cfg, &card.title, 12.0, Face::Text(450), pal.ink_meta, x + 14.0, y + CARD_H - 20.0, CARD_W - 28.0, left);
        }
        // The dragged window rides under the pointer, a touch larger.
        if let Some(d) = self.drag.as_ref().filter(|d| d.moving)
            && let Some((_, app_id, icon)) = drag_window(&self.cards, d) {
            let size = ICON + 8.0;
            let (ix, iy) = (d.at.0 - rect.x - size / 2.0, d.at.1 - rect.y - size / 2.0);
            draw::fill(s, &draw::round(ix - 6.0, iy - 6.0, size + 12.0, size + 12.0, 14.0), pal.body.mix(pal.ink_main, 0.12).alpha(0.92));
            match icon {
                Some(p) => draw::paintable(s, p, ix, iy, size),
                None => draw::monogram(s, &self.paint, cfg, &crate::apps::pretty(app_id), ix, iy, size, pal.raised, pal.ink_main),
            }
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
        panel_corner::screen_mask(s, &self.app, self.corner, self.surface_size(), self.margins);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn super_tab_steps_through_used_workspaces_and_wraps() {
        let used = [(1, 11), (2, 12), (3, 13), (4, 14)];
        assert_eq!(next_used(&used, Some(2)), Some((3, 13)));
        assert_eq!(next_used(&used, Some(4)), Some((1, 11)));
        assert_eq!(next_used(&[(2, 12)], Some(2)), None);
        assert_eq!(next_used(&used, None), None);
    }

    fn space(id: u64, idx: u64, output: &str, filled: bool, active: bool) -> compositor::Space {
        compositor::Space { id, idx, output: output.into(), filled, active, focused: active }
    }

    fn numbers(cards: &[CardPlan]) -> Vec<u64> { cards.iter().map(|c| c.idx).collect() }

    #[test]
    fn hyprland_synthesizes_trailing_and_pins_in_number_order() {
        let spaces = [space(5, 5, "A", true, true), space(2, 2, "A", true, false), space(3, 3, "A", false, false)];
        let cards = card_list(&spaces, "A", false, 4, 12);
        assert_eq!(numbers(&cards), [1, 2, 3, 4, 5, 6]);
        assert_eq!(cards[0].target, Target::Index(1));
        assert_eq!(cards[2].target, Target::Id(3));
        assert_eq!(cards[5].target, Target::Index(6));
        assert_eq!(numbers(&card_list(&spaces, "A", false, 0, 12)), [2, 5, 6]);
        assert_eq!(numbers(&card_list(&[], "A", false, 0, 12)), [1]);
    }

    #[test]
    fn hyprland_global_numbers_keep_their_output() {
        let spaces = [space(2, 2, "A", true, true), space(1, 1, "B", false, false), space(3, 3, "B", true, true)];
        assert_eq!(numbers(&card_list(&spaces, "A", false, 3, 12)), [2, 4]);
        assert_eq!(numbers(&card_list(&spaces, "B", false, 3, 12)), [1, 3, 4]);
    }

    #[test]
    fn niri_reuses_its_empty_tail_even_when_active_and_pins_local_indexes() {
        let spaces = [space(101, 1, "A", true, false), space(102, 2, "A", false, true), space(201, 1, "B", true, true)];
        let cards = card_list(&spaces, "A", true, 4, 12);
        assert_eq!(numbers(&cards), [1, 2, 3, 4]);
        assert_eq!(cards[1].target, Target::Id(102));
        assert_eq!(cards[2].target, Target::Index(3));
        assert_eq!(numbers(&card_list(&spaces, "A", true, 0, 12)), [1, 2]);
        assert_eq!(numbers(&card_list(&spaces, "B", true, 2, 12)), [1, 2]);
    }

    #[test]
    fn cap_keeps_used_then_tail_then_lowest_pins_and_clamps_pref() {
        let spaces = [space(7, 7, "A", true, false), space(9, 9, "A", true, true)];
        assert_eq!(numbers(&card_list(&spaces, "A", false, 10, 4)), [1, 7, 9, 10]);
        assert_eq!(numbers(&card_list(&spaces, "A", true, 10, 4)), [1, 7, 9, 10]);
        let full: Vec<_> = (1..=13).map(|idx| space(idx, idx, "A", true, idx == 1)).collect();
        assert_eq!(numbers(&card_list(&full, "A", false, 10, PER_ROW * ROWS)), (1..=12).collect::<Vec<_>>());
        assert_eq!(numbers(&card_list(&spaces, "A", false, 10, 3)), [7, 9, 10]);
        assert_eq!(numbers(&card_list(&spaces, "A", false, 10, 2)), [7, 9]);
        assert_eq!(numbers(&card_list(&spaces, "A", false, 10, 1)), [7]);
        assert!(card_list(&spaces, "A", false, 10, 0).is_empty());
        assert_eq!(numbers(&card_list(&[], "A", false, 99, 12)), (1..=10).collect::<Vec<_>>());
        assert_eq!(numbers(&card_list(&[], "A", false, -1, 12)), [1]);
    }

    #[test]
    fn default_corner_keeps_heads_fixed_surface_and_unscaled_placement() {
        assert_eq!((SURFACE_W, SURFACE_H), (870.0, 522.0));
        assert_eq!(panel_corner::margins(Corner::TopLeft), [0; 4]);
        for (w, h) in [(232.0, 192.0), (856.0, 508.0)] {
            let rect = panel_corner::placement(Corner::TopLeft, (SURFACE_W, SURFACE_H), (w, h), 1.0, FILLET);
            assert_eq!(rect, panel_corner::Rect { x: 0.0, y: 0.0, w, h });
            assert_eq!(panel_corner::envelope(Corner::TopLeft, rect, (SURFACE_W, SURFACE_H), FILLET),
                panel_corner::Rect { x: 0.0, y: 0.0, w: w + 14.0, h: h + 14.0 });
        }
    }

    #[test]
    fn drag_follows_ids_and_cancels_missing_source() {
        let card = |id, ids: &[u64]| Card { target: Target::Id(id), idx: id, active: false, title: String::new(), windows: ids.iter().map(|id| (*id, String::new(), None)).collect() };
        let drag = Drag { space: Target::Id(2), window: 9, at: (0.0, 0.0), moving: true };
        assert_eq!(drag_window(&[card(1, &[9]), card(2, &[8, 9])], &drag).map(|w| w.0), Some(9));
        assert!(drag_window(&[card(2, &[8])], &drag).is_none());
        assert!(drag_window(&[card(1, &[9])], &drag).is_none());
    }
}
