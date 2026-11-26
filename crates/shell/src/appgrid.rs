//! The app grid: every app on pages over the blurred wallpaper, opened from the dock's Apps
//! button or its shortcut. Apps are found by typing, launched with a click or Enter, rearranged
//! by dragging, and gathered into folders by dropping one app onto another. The arrangement is
//! kept in the state directory; new apps join at the end and removed ones leave quietly.

use crate::app::App;
use crate::draw::{self, Face};
use crate::feel::{self, Feel};
use crate::paint::Paint;
use gtk::{gdk, glib, graphene, gsk, pango, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kit::motion::{self, Tween};
use kit::palette::Rgb;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// One place on the grid: an app by id, or a named folder of apps.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub enum Slot {
    App(String),
    Folder { name: String, apps: Vec<String> },
}

impl Slot {
    fn key(&self) -> String {
        match self {
            Slot::App(id) => id.clone(),
            Slot::Folder { apps, .. } => format!("folder:{}", apps.first().map_or("", String::as_str)),
        }
    }
}

fn path() -> std::path::PathBuf {
    kit::theme::state_dir().join("appgrid.json")
}

fn load() -> Vec<Slot> {
    std::fs::read_to_string(path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save(slots: &[Slot]) {
    let Ok(text) = serde_json::to_string(slots) else { return };
    let _ = kit::atomic::write(&path(), text);
}

/// The saved arrangement brought up to date with what's installed (`installed`, in name order):
/// gone apps leave, a folder left with one app becomes that app, and new apps join at the end.
fn sync(mut slots: Vec<Slot>, installed: &[String]) -> Vec<Slot> {
    let has = |id: &String| installed.contains(id);
    let mut seen: Vec<String> = Vec::new();
    slots = slots
        .into_iter()
        .filter_map(|slot| match slot {
            Slot::App(id) if has(&id) && !seen.contains(&id) => {
                seen.push(id.clone());
                Some(Slot::App(id))
            }
            Slot::Folder { name, apps } => {
                let apps: Vec<String> = apps.into_iter().filter(|a| has(a) && !seen.contains(a)).collect();
                seen.extend(apps.iter().cloned());
                match apps.len() {
                    0 => None,
                    1 => Some(Slot::App(apps[0].clone())),
                    _ => Some(Slot::Folder { name, apps }),
                }
            }
            _ => None,
        })
        .collect();
    slots.extend(installed.iter().filter(|id| !seen.contains(id)).map(|id| Slot::App(id.clone())));
    slots
}

/// A name for a new folder of these apps: the main category they share, or "Folder".
fn folder_name(app: &App, ids: &[String]) -> String {
    const NAMES: [(&str, &str); 9] = [
        ("Game", "Games"),
        ("Development", "Development"),
        ("Graphics", "Creativity"),
        ("AudioVideo", "Media"),
        ("Office", "Productivity"),
        ("Network", "Internet"),
        ("Settings", "Settings"),
        ("System", "Utilities"),
        ("Utility", "Utilities"),
    ];
    let list = app.apps.list();
    let cats: Vec<String> = ids
        .iter()
        .filter_map(|id| list.iter().find(|e| e.id == *id))
        .filter_map(|e| e.info.downcast_ref::<gio_unix::DesktopAppInfo>().and_then(|d| d.categories()).map(|c| c.to_string()))
        .collect();
    NAMES.iter().find(|(c, _)| !cats.is_empty() && cats.iter().all(|all| all.split(';').any(|x| x == *c))).map_or("Folder", |(_, n)| n).to_string()
}

/// How long a page takes to turn.
const PAGE_MS: i64 = 460;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Hit {
    /// An item of the grid (or of the open folder), by its index there.
    Item(usize),
    /// A page dot.
    Dot(usize),
    /// The open folder's name.
    Title,
    /// Inside the open folder, away from its apps.
    Panel,
    Search,
    Nothing,
}

struct Drag {
    /// The cell it waits over and since when: after a moment the others make way there.
    pending: Option<(usize, i64)>,
    /// The slot being moved, taken out of the grid (or the folder) while it moves.
    slot: Slot,
    /// From an open folder (its index in the grid).
    from_folder: Option<usize>,
    at: (f64, f64),
    /// The item it rests on and since when, to make a folder after a moment.
    over: Option<(usize, i64)>,
    /// Since when it waits at a side of the screen, to turn the page.
    edge: Option<(i32, i64)>,
}

struct Folder {
    index: usize,
    open: Tween,
    renaming: bool,
}

struct State {
    app: Rc<App>,
    window: gtk::Window,
    paint: Paint,
    size: (f64, f64),
    open: bool,
    fade: Tween,
    page: Tween,
    slots: Vec<Slot>,
    query: String,
    selected: Option<usize>,
    folder: Option<Folder>,
    drag: Option<Drag>,
    /// Where each item is drawn now, easing toward where it belongs.
    shown: HashMap<String, (f64, f64)>,
    /// When items that came in with the search appeared, to grow them in.
    born: HashMap<String, i64>,
    /// Apps' names by id, read once per opening rather than every frame.
    names: HashMap<String, String>,
    /// An idle is already drawing icons.
    art_pending: bool,
    /// When the search last changed.
    searched: i64,
    art: HashMap<String, gdk::Texture>,
    /// What the icons in `art` were drawn for (style, theme, size), to redraw them when it changes.
    art_key: String,
    wanted: bool,
    /// The wallpaper this screen shows, blurred (see `Wallpapers::blurred`); drawn as it is every frame.
    backdrop: Option<gdk::Texture>,
    /// Items still easing to their places, which keeps frames coming.
    moving: bool,
    search: gtk::Text,
    press: Option<(f64, f64, Hit)>,
    feel: Feel<Hit>,
    hover: Option<Hit>,
    /// The dock as seen when the grid opened: how far it reaches in at the bottom, and its apps'
    /// icon centres, which apps found by the search fly in from.
    dock_room: f64,
    dock_apps: HashMap<String, (f64, f64)>,
    /// An app being dragged out of the dock over the grid, and where.
    ghost: Option<(String, (f64, f64))>,
    scroll: f64,
    turned: i64,
    ticking: bool,
    last: i64,
}

type Shared = Rc<RefCell<State>>;

thread_local! {
    static ONE: RefCell<Option<Shared>> = const { RefCell::new(None) };
    /// Kept between openings: the icons (by what they were drawn for), so the grid opens at
    /// once, its icons already there.
    static ICONS: RefCell<(String, HashMap<String, gdk::Texture>)> = RefCell::new(Default::default());
}

/// The soft copy of the wallpaper the grid's screen shows, as that screen shows it (so a
/// wallpaper stretched over two screens is blurred in the part each has). It is the same
/// texture the lock screen uses, made once with the wallpaper.
fn blurred(app: &App) -> Option<gdk::Texture> {
    app.wallpapers.blurred(&app.primary_name())
}

/// The grid's measures on a screen `w` by `h`.
#[derive(Clone, Copy)]
struct Grid {
    cols: usize,
    rows: usize,
    cell_w: f64,
    cell_h: f64,
    x0: f64,
    y0: f64,
    icon: f64,
}

const SEARCH_Y: f64 = 34.0;
const SEARCH_H: f64 = 36.0;
const SEARCH_W: f64 = 300.0;
const DOTS: f64 = 44.0;

fn grid(w: f64, h: f64) -> Grid {
    let icon = (h * 0.072).clamp(52.0, 92.0).round();
    let cell_w = (icon * 1.95).round();
    let cell_h = (icon + 48.0).round();
    let top = SEARCH_Y + SEARCH_H + 36.0;
    let room = h - top - DOTS - 40.0;
    let cols = ((w * 0.82 / cell_w).floor() as usize).clamp(4, 8);
    let rows = ((room / cell_h).floor() as usize).clamp(2, 6);
    Grid { cols, rows, cell_w, cell_h, x0: ((w - cols as f64 * cell_w) / 2.0).round(), y0: (top + (room - rows as f64 * cell_h) / 2.0).round(), icon }
}

impl Grid {
    fn per_page(&self) -> usize {
        self.cols * self.rows
    }

    /// The top left of the icon of item `n` (on its own page, whatever page that is).
    fn spot(&self, n: usize) -> (f64, f64) {
        let k = n % self.per_page();
        let (c, r) = (k % self.cols, k / self.cols);
        (self.x0 + c as f64 * self.cell_w + (self.cell_w - self.icon) / 2.0, self.y0 + r as f64 * self.cell_h)
    }

    /// The cell of the page under (x, y), counted on that page.
    fn cell_at(&self, x: f64, y: f64) -> Option<usize> {
        let (c, r) = ((x - self.x0) / self.cell_w, (y - self.y0) / self.cell_h);
        (c >= 0.0 && r >= 0.0 && (c as usize) < self.cols && (r as usize) < self.rows).then(|| r as usize * self.cols + c as usize)
    }
}

/// Opens the grid, or closes it when it's open.
pub fn toggle(app: &Rc<App>) {
    let open = ONE.with(|one| one.borrow().as_ref().is_some_and(|st| st.borrow().open));
    if open {
        if let Some(st) = ONE.with(|one| one.borrow().clone()) {
            close(&st);
        }
    } else {
        show(app);
    }
}

fn show(app: &Rc<App>) {
    let st = ONE.with(|one| one.borrow_mut().get_or_insert_with(|| build(app)).clone());
    // Apps in the dock or hidden (see `kit::hidden`) stay out of the arrangement; the dock's
    // still turn up in the search.
    let docked = crate::dock::pinned();
    let installed: Vec<String> = {
        let list = app.apps.list();
        let mut v: Vec<(String, String)> = list.iter().filter(|e| !app.hidden(&e.id) && !docked.contains(&e.id)).map(|e| (e.name.to_lowercase(), e.id.clone())).collect();
        v.sort();
        v.into_iter().map(|(_, id)| id).collect()
    };
    {
        let mut s = st.borrow_mut();
        let slots = sync(load(), &installed);
        s.slots = slots;
        s.names = app.apps.list().iter().map(|e| (e.id.clone(), e.name.clone())).collect();
        s.query.clear();
        s.selected = None;
        s.folder = None;
        s.drag = None;
        s.shown.clear();
        s.page.set(0.0);
        s.open = true;
        s.backdrop = None;
        let (key, art) = ICONS.with(|i| std::mem::take(&mut *i.borrow_mut()));
        s.art = art;
        s.art_key = key;
        s.search.set_text("");
        let now = glib::monotonic_time();
        s.fade.go(1.0, now, if motion::reduced() { 160 } else { 420 }, motion::EMPHASIZED_DECEL);
        s.window.set_keyboard_mode(KeyboardMode::Exclusive);
        s.window.set_visible(true);
        s.window.present();
        let size = s.size;
        let seen = crate::dock::seen(size);
        s.dock_room = seen.as_ref().filter(|(side, _, _)| *side == "bottom").map_or(0.0, |(_, reach, _)| *reach);
        s.dock_apps = seen.map(|(_, _, apps)| apps.into_iter().collect()).unwrap_or_default();
        s.ghost = None;
        s.search.grab_focus();
    }
    // The dock comes up over the grid, to drag apps between them.
    crate::dock::lift(true);
    save(&st.borrow().slots);
    kick(&st);
}

fn close(st: &Shared) {
    let mut s = st.borrow_mut();
    if !s.open {
        return;
    }
    s.open = false;
    s.folder = None;
    s.drag = None;
    let now = glib::monotonic_time();
    s.fade.go(0.0, now, if motion::reduced() { 140 } else { 300 }, motion::STANDARD);
    s.window.set_keyboard_mode(KeyboardMode::None);
    drop(s);
    crate::dock::lift(false);
    kick(st);
}

fn build(app: &Rc<App>) -> Shared {
    let window = gtk::Window::new();
    window.init_layer_shell();
    // A layer under the dock's, which rises over it while the grid is open (see `dock::lift`).
    window.set_layer(Layer::Top);
    window.set_namespace(Some("digit-appgrid"));
    window.set_exclusive_zone(-1);
    for e in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(e, true);
    }
    window.add_css_class("digit");
    let monitor = app.primary();
    if let Some(m) = &monitor {
        window.set_monitor(Some(m));
    }
    let size = monitor.map_or((1920.0, 1080.0), |m| (m.geometry().width() as f64, m.geometry().height() as f64));
    let paint = Paint::new(size.0 as i32, size.1 as i32);
    // The search field's text is a real one (selection, Ctrl+A, Ctrl+Backspace, input methods)
    // laid over the field the grid draws.
    let search = gtk::Text::new();
    search.set_placeholder_text(Some("Search Applications"));
    search.add_css_class("appgrid-search");
    search.set_halign(gtk::Align::Center);
    search.set_valign(gtk::Align::Start);
    search.set_margin_top(SEARCH_Y as i32);
    search.set_margin_start(24);
    search.set_size_request((SEARCH_W - 56.0) as i32, SEARCH_H as i32);
    let css = gtk::CssProvider::new();
    css.load_from_string("text.appgrid-search { background: none; color: rgba(255,255,255,0.95); caret-color: white; font-size: 14px; } text.appgrid-search placeholder { color: rgba(255,255,255,0.55); } text.appgrid-search selection { background: rgba(255,255,255,0.3); }");
    #[allow(deprecated)]
    search.style_context().add_provider(&css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    let layers = gtk::Overlay::new();
    layers.set_child(Some(&paint));
    layers.add_overlay(&search);
    window.set_child(Some(&layers));
    let st = Rc::new(RefCell::new(State {
        app: app.clone(),
        window: window.clone(),
        paint: paint.clone(),
        size,
        open: false,
        fade: Tween::still(0.0),
        page: Tween::still(0.0),
        slots: Vec::new(),
        query: String::new(),
        selected: None,
        folder: None,
        drag: None,
        shown: HashMap::new(),
        born: HashMap::new(),
        names: HashMap::new(),
        art_pending: false,
        searched: 0,
        art: HashMap::new(),
        art_key: String::new(),
        wanted: false,
        backdrop: None,
        moving: false,
        search: search.clone(),
        press: None,
        feel: Feel::default(),
        hover: None,
        dock_room: 0.0,
        dock_apps: HashMap::new(),
        ghost: None,
        scroll: 0.0,
        turned: 0,
        ticking: false,
        last: 0,
    }));
    let weak = Rc::downgrade(&st);
    paint.on_draw(move |s| {
        if let Some(st) = weak.upgrade() {
            if let Ok(mut st) = st.try_borrow_mut() {
                st.draw(s);
            }
            fill_art(&st);
        }
    });
    let keys = gtk::EventControllerKey::new();
    let weak = Rc::downgrade(&st);
    // Before the search field: the grid's keys, and everything while a folder is renamed.
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed(move |_, key, _, _| {
        let Some(st) = weak.upgrade() else { return glib::Propagation::Proceed };
        if press_key(&st, key) { glib::Propagation::Stop } else { glib::Propagation::Proceed }
    });
    window.add_controller(keys);
    let weak = Rc::downgrade(&st);
    search.connect_changed(move |t| {
        let Some(st) = weak.upgrade() else { return };
        let Ok(mut s) = st.try_borrow_mut() else { return };
        s.query = t.text().to_string();
        s.selected = (!s.query.trim().is_empty()).then_some(0);
        // Apps that stay slide to their new places; the ones that leave go, new ones grow in.
        let keep: Vec<String> = visible(&s).iter().map(Slot::key).collect();
        s.shown.retain(|k, _| keep.contains(k));
        s.searched = glib::monotonic_time();
        s.moving = true;
        s.page.set(0.0);
        drop(s);
        kick(&st);
    });
    let drag = gtk::GestureDrag::new();
    let weak = Rc::downgrade(&st);
    drag.connect_drag_begin(move |_, x, y| {
        if let Some(st) = weak.upgrade() {
            let hit = st.borrow().hit(x, y);
            st.borrow_mut().press = Some((x, y, hit));
            if matches!(hit, Hit::Item(_) | Hit::Dot(_)) {
                st.borrow().feel.press(hit, glib::monotonic_time());
                kick(&st);
            }
        }
    });
    let weak = Rc::downgrade(&st);
    drag.connect_drag_update(move |g, dx, dy| {
        let Some(st) = weak.upgrade() else { return };
        let Some((x, y)) = g.start_point() else { return };
        moved(&st, x + dx, y + dy);
    });
    let weak = Rc::downgrade(&st);
    drag.connect_drag_end(move |g, dx, dy| {
        let Some(st) = weak.upgrade() else { return };
        st.borrow().feel.release(glib::monotonic_time());
        kick(&st);
        let Some((x, y)) = g.start_point() else { return };
        released(&st, x + dx, y + dy, dx.hypot(dy) < 8.0);
    });
    paint.add_controller(drag);
    let motion = gtk::EventControllerMotion::new();
    let weak = Rc::downgrade(&st);
    motion.connect_motion(move |_, x, y| {
        if let Some(st) = weak.upgrade() {
            let hit = st.borrow().hit(x, y);
            point(&st, Some(hit));
        }
    });
    let weak = Rc::downgrade(&st);
    motion.connect_leave(move |_| {
        if let Some(st) = weak.upgrade() {
            point(&st, None);
        }
    });
    paint.add_controller(motion);
    // A right click on an app or a folder offers what can be done with it.
    let menu = gtk::GestureClick::new();
    menu.set_button(3);
    let weak = Rc::downgrade(&st);
    menu.connect_pressed(move |_, _, x, y| {
        if let Some(st) = weak.upgrade() {
            context_menu(&st, x, y);
        }
    });
    paint.add_controller(menu);
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
    let weak = Rc::downgrade(&st);
    scroll.connect_scroll(move |c, dx, dy| {
        let Some(st) = weak.upgrade() else { return glib::Propagation::Proceed };
        let smooth = c.unit() == gdk::ScrollUnit::Surface;
        let step = if dx.abs() > dy.abs() { dx } else { dy };
        let mut s = st.borrow_mut();
        let now = glib::monotonic_time();
        if s.folder.is_some() || now - s.turned < 320_000 {
            return glib::Propagation::Stop;
        }
        s.scroll += if smooth { step / 60.0 } else { step };
        if s.scroll.abs() >= 1.0 {
            let by = s.scroll.signum() as i32;
            s.scroll = 0.0;
            s.turned = now;
            drop(s);
            turn(&st, by);
        }
        glib::Propagation::Stop
    });
    paint.add_controller(scroll);
    st
}

/// What's shown: the arrangement, or the apps matching the search, best first.
fn visible(s: &State) -> std::borrow::Cow<'_, [Slot]> {
    let q = s.query.trim().to_lowercase();
    if q.is_empty() {
        return std::borrow::Cow::Borrowed(&s.slots);
    }
    let list = s.app.apps.list();
    let mut hits: Vec<(i32, String)> = list
        .iter()
        .filter(|e| !s.app.hidden(&e.id))
        .filter_map(|e| {
            let name = e.name.to_lowercase();
            let score = if name.starts_with(&q) {
                3
            } else if name.split_whitespace().any(|w| w.starts_with(&q)) {
                2
            } else if name.contains(&q) || e.keys.iter().any(|(k, _)| k.to_lowercase().contains(&q)) {
                1
            } else {
                return None;
            };
            Some((score * 1000 + s.app.apps.boost(&e.id).min(999), e.id.clone()))
        })
        .collect();
    hits.sort_by_key(|h| std::cmp::Reverse(h.0));
    std::borrow::Cow::Owned(hits.into_iter().map(|(_, id)| Slot::App(id)).collect())
}

impl State {
    fn grid(&self) -> Grid {
        grid(self.size.0, self.size.1 - self.dock_room)
    }

    fn pages(&self) -> usize {
        visible(self).len().div_ceil(self.grid().per_page()).max(1)
    }

    fn current_page(&self) -> usize {
        self.page.target().round().max(0.0) as usize
    }

    /// The open folder's panel: where it is, and the grid of its apps.
    fn panel(&self) -> Option<((f64, f64, f64, f64), Grid)> {
        let f = self.folder.as_ref()?;
        let Some(Slot::Folder { apps, .. }) = self.slots.get(f.index) else { return None };
        let g = self.grid();
        let cols = apps.len().clamp(3, 5);
        let rows = apps.len().div_ceil(cols).clamp(1, 4);
        let w = cols as f64 * g.cell_w + 48.0;
        let h = rows as f64 * g.cell_h + 48.0;
        let (x, y) = ((self.size.0 - w) / 2.0, ((self.size.1 - h) / 2.0).max(g.y0));
        Some(((x, y, w, h), Grid { cols, rows, x0: x + 24.0, y0: y + 30.0, ..g }))
    }

    fn folder_apps(&self) -> Vec<String> {
        match self.folder.as_ref().and_then(|f| self.slots.get(f.index)) {
            Some(Slot::Folder { apps, .. }) => apps.clone(),
            _ => Vec::new(),
        }
    }

    fn hit(&self, x: f64, y: f64) -> Hit {
        if let Some(((px, py, pw, ph), g)) = self.panel() {
            if y >= py - 46.0 && y < py && (x - self.size.0 / 2.0).abs() < 160.0 {
                return Hit::Title;
            }
            if x < px || x > px + pw || y < py || y > py + ph {
                return Hit::Nothing;
            }
            return match g.cell_at(x, y) {
                Some(n) if n < self.folder_apps().len() => Hit::Item(n),
                _ => Hit::Panel,
            };
        }
        if (SEARCH_Y..=SEARCH_Y + SEARCH_H).contains(&y) && (x - self.size.0 / 2.0).abs() <= SEARCH_W / 2.0 {
            return Hit::Search;
        }
        let pages = self.pages();
        let dots_y = self.size.1 - self.dock_room - DOTS;
        if (y - dots_y).abs() < 12.0 && pages > 1 {
            let x0 = self.size.0 / 2.0 - (pages - 1) as f64 * 9.0;
            if let Some(p) = (0..pages).find(|p| (x - (x0 + *p as f64 * 18.0)).abs() < 9.0) {
                return Hit::Dot(p);
            }
        }
        let g = self.grid();
        match g.cell_at(x, y) {
            Some(n) => {
                let i = self.current_page() * g.per_page() + n;
                if i < visible(self).len() { Hit::Item(i) } else { Hit::Nothing }
            }
            None => Hit::Nothing,
        }
    }

    fn icon_of(&self, s: &gtk::Snapshot, id: &str, x: f64, y: f64, size: f64) {
        let rect = graphene::Rect::new(x as f32, y as f32, size as f32, size as f32);
        match self.art.get(id) {
            Some(t) => s.append_scaled_texture(t, gsk::ScalingFilter::Linear, &rect),
            None => draw::fill(s, &draw::round(x, y, size, size, size * 0.24), Rgb::new(1.0, 1.0, 1.0).alpha(0.12)),
        }
    }

    /// A folder's face: a frosted tile with its first apps in small.
    fn folder_face(&self, s: &gtk::Snapshot, apps: &[String], x: f64, y: f64, size: f64, lit: f64) {
        draw::fill(s, &draw::round(x, y, size, size, size * 0.24), Rgb::new(1.0, 1.0, 1.0).alpha(0.2 + 0.14 * lit));
        let mini = size * 0.24;
        let gap = (size - 3.0 * mini) / 4.0;
        for (k, id) in apps.iter().take(9).enumerate() {
            let (c, r) = ((k % 3) as f64, (k / 3) as f64);
            self.icon_of(s, id, x + gap + c * (mini + gap), y + gap + r * (mini + gap), mini);
        }
    }

    /// A name under an icon, lifted by a dark copy a pixel below: a shadow node per label
    /// costs the GPU an offscreen pass each, every frame.
    fn label(&self, s: &gtk::Snapshot, text: &str, cx: f64, mid: f64, w: f64, alpha: f64) {
        let (cfg, center) = (&self.app.cfg, pango::Alignment::Center);
        draw::text(s, &self.paint, cfg, text, 13.0, Face::Text(500), Rgb::new(0.0, 0.0, 0.0).alpha(0.4 * alpha), cx - w / 2.0, mid + 1.0, w, center);
        draw::text(s, &self.paint, cfg, text, 13.0, Face::Text(500), Rgb::new(1.0, 1.0, 1.0).alpha(0.95 * alpha), cx - w / 2.0, mid, w, center);
    }

    /// Eases each drawn position toward its place; returns whether anything still moves.
    fn ease(&mut self, targets: &[(String, (f64, f64))], now: i64) -> bool {
        let dt = ((now - self.last) as f64 / 1e6).clamp(0.0, 0.05);
        let k = if motion::reduced() { 1.0 } else { 1.0 - (-dt * 16.0).exp() };
        let mut moving = false;
        for (key, to) in targets {
            if !self.shown.contains_key(key) && now - self.searched < 400_000 && !motion::reduced() {
                self.born.insert(key.clone(), now);
            }
            // An app the search found in the dock flies out of it.
            let from_dock = (!self.query.trim().is_empty()).then(|| self.dock_apps.get(key)).flatten().map(|c| {
                let icon = grid(self.size.0, self.size.1 - self.dock_room).icon;
                (c.0 - icon / 2.0, c.1 - icon / 2.0)
            });
            if from_dock.is_some() {
                self.born.remove(key);
            }
            let at = self.shown.entry(key.clone()).or_insert(from_dock.unwrap_or(*to));
            at.0 += (to.0 - at.0) * k;
            at.1 += (to.1 - at.1) * k;
            if (to.0 - at.0).abs() > 0.5 || (to.1 - at.1).abs() > 0.5 {
                moving = true;
            } else {
                *at = *to;
            }
        }
        self.born.retain(|_, b| now - *b < 220_000);
        moving || !self.born.is_empty()
    }

    fn draw(&mut self, s: &gtk::Snapshot) {
        let now = self.paint.frame_clock().map_or_else(glib::monotonic_time, |c| c.frame_time());
        let t = self.fade.value(now).clamp(0.0, 1.0);
        let (w, h) = self.size;
        if t < 0.003 {
            return;
        }
        // The wallpaper, blurred and dimmed, fading in over everything.
        let faded = fade(s, t);
        if let Some(bg) = &self.backdrop {
            s.append_scaled_texture(bg, gsk::ScalingFilter::Linear, &graphene::Rect::new(0.0, 0.0, w as f32, h as f32));
        }
        let dim = if self.backdrop.is_some() { 0.28 } else { 0.62 };
        s.append_color(&gdk::RGBA::new(0.0, 0.0, 0.0, dim), &graphene::Rect::new(0.0, 0.0, w as f32, h as f32));
        if faded {
            s.pop();
        }
        // The grid settles in from a little larger, as macOS's does.
        // Only the icons' places and sizes grow: scaling the whole grid would make the GPU draw
        // every label's glyphs again at each frame's new scale.
        let zoom = if motion::reduced() { 1.0 } else { 1.0 + 0.09 * (1.0 - t) };
        let (zx, zy) = (w / 2.0, h / 2.0);
        s.save();
        let grid_faded = fade(s, t);
        let cfg = self.app.cfg.clone();
        let white = Rgb::new(1.0, 1.0, 1.0);
        // The search field.
        let sx = (w - SEARCH_W) / 2.0;
        let field = draw::round(sx, SEARCH_Y, SEARCH_W, SEARCH_H, SEARCH_H / 2.0);
        draw::fill(s, &field, white.alpha(0.16));
        s.append_border(&field, &[1.0; 4], &[white.alpha(0.18).rgba(); 4]);
        draw::icon(s, &self.paint, &cfg, "search", 18.0, false, white.alpha(0.8), sx + 22.0, SEARCH_Y + SEARCH_H / 2.0);
        let g = self.grid();
        let list = visible(self).into_owned();
        let folder_t = self.folder.as_ref().map_or(0.0, |f| f.open.value(now).clamp(0.0, 1.0));
        // Each item's place: its cell on its page, the pages side by side.
        let page = self.page.value(now);
        let dragged = self.drag.as_ref().map(|d| d.slot.key());
        let targets: Vec<(String, (f64, f64))> = list
            .iter()
            .enumerate()
            .filter(|(_, slot)| Some(slot.key()) != dragged || self.drag.as_ref().is_some_and(|d| d.from_folder.is_some()))
            .map(|(i, slot)| {
                let (x, y) = g.spot(i);
                (slot.key(), (x + (i / g.per_page()) as f64 * w, y))
            })
            .collect();
        let moving = self.ease(&targets, now);
        let names = &self.names;
        let over = self.drag.as_ref().and_then(|d| d.over.filter(|(_, at)| now - at > 380_000).map(|(i, _)| i));
        let behind = fade(s, 1.0 - 0.75 * folder_t);
        for (i, slot) in list.iter().enumerate() {
            let key = slot.key();
            let Some(&(x, y)) = self.shown.get(&key) else { continue };
            let x = x - page * w;
            // Grown out from the middle while it opens.
            let (cx, cy) = (zx + (x + g.icon / 2.0 - zx) * zoom, zy + (y + g.icon / 2.0 - zy) * zoom);
            let born = self.born.get(&key).map_or(1.0, |b| ((now - b) as f64 / 220_000.0).clamp(0.0, 1.0));
            let size = g.icon * zoom * (0.7 + 0.3 * born);
            let (x, y) = (cx - size / 2.0, cy - size / 2.0);
            if x < -g.cell_w * 2.0 || x > w + g.cell_w || (dragged.as_ref() == Some(&key) && self.drag.as_ref().is_some_and(|d| d.from_folder.is_none())) {
                continue;
            }
            let lit = over == Some(i);
            if self.selected == Some(i) {
                draw::fill(s, &draw::round(x - 10.0, y - 10.0, g.icon + 20.0, g.icon + 20.0, (g.icon + 20.0) * 0.24), white.alpha(0.16));
            }
            let hit = Hit::Item(i);
            // The open folder's apps are the ones under the pointer then.
            let (glow, give) = if self.folder.is_some() { (0.0, 1.0) } else { (self.feel.glow(hit, now), self.feel.scale(hit, feel::SMALL, now)) };
            let over = self.feel.on(Hit::Dot(usize::MAX - i), lit, now);
            let grow = g.icon * (0.08 * over + 0.02 * glow) + (give - 1.0) * size / 2.0;
            // Apps that just came in with the search grow and fade in.
            let young = born < 1.0 && fade(s, born);
            let name = match slot {
                Slot::App(id) => {
                    self.icon_of(s, id, x - grow, y - grow, size + 2.0 * grow);
                    names.get(id).cloned().unwrap_or_default()
                }
                Slot::Folder { name, apps } => {
                    self.folder_face(s, apps, x - grow, y - grow, size + 2.0 * grow, over.max(0.4 * glow));
                    name.clone()
                }
            };
            self.label(s, &name, cx, cy + size / 2.0 + 18.0, g.cell_w - 10.0, 1.0);
            if young {
                s.pop();
            }
        }
        if behind {
            s.pop();
        }
        // Page dots.
        let pages = self.pages();
        if pages > 1 {
            let x0 = w / 2.0 - (pages - 1) as f64 * 9.0;
            for p in 0..pages {
                let near = 1.0 - (page - p as f64).abs().min(1.0);
                let dot = Hit::Dot(p);
                let r = 3.5 * self.feel.scale(dot, feel::SMALL, now) + self.feel.glow(dot, now);
                draw::fill(s, &draw::round(x0 + p as f64 * 18.0 - r, h - self.dock_room - DOTS - r, 2.0 * r, 2.0 * r, r), white.alpha(0.35 + 0.55 * near));
            }
        }
        // The open folder: its name over a panel of its apps, growing out of the grid.
        if let (Some(((px, py, pw, ph), fg)), Some(f)) = (self.panel(), self.folder.as_ref()) {
            let k = 0.92 + 0.08 * folder_t;
            s.save();
            s.translate(&graphene::Point::new((px + pw / 2.0) as f32, (py + ph / 2.0) as f32));
            s.scale(k as f32, k as f32);
            s.translate(&graphene::Point::new((-(px + pw / 2.0)) as f32, (-(py + ph / 2.0)) as f32));
            s.push_opacity(folder_t);
            let shape = draw::round(px, py, pw, ph, 30.0 * self.app.round());
            draw::fill(s, &shape, white.alpha(0.16));
            s.append_border(&shape, &[1.0; 4], &[white.alpha(0.16).rgba(); 4]);
            let name = match self.slots.get(f.index) {
                Some(Slot::Folder { name, .. }) => name.clone(),
                _ => String::new(),
            };
            let title = if f.renaming { format!("{name}│") } else { name };
            s.push_shadow(&[gsk::Shadow::new(gdk::RGBA::new(0.0, 0.0, 0.0, 0.4), 0.0, 1.0, 6.0)]);
            draw::grown(k, || draw::text(s, &self.paint, &cfg, &title, 26.0, Face::Text(600), white.alpha(0.96), 0.0, py - 24.0, w, pango::Alignment::Center));
            s.pop();
            for (n, id) in self.folder_apps().iter().enumerate() {
                if dragged.as_deref() == Some(id.as_str()) {
                    continue;
                }
                let (x, y) = fg.spot(n);
                let hit = Hit::Item(n);
                let grow = g.icon * (0.02 * self.feel.glow(hit, now) + (self.feel.scale(hit, feel::SMALL, now) - 1.0) / 2.0);
                self.icon_of(s, id, x - grow, y - grow, g.icon + 2.0 * grow);
                draw::grown(k, || self.label(s, names.get(id).map_or("", String::as_str), x + g.icon / 2.0, y + g.icon + 18.0, g.cell_w - 10.0, 1.0));
            }
            s.pop();
            s.restore();
        }
        // An app dragged out of the dock, under the pointer.
        if let Some((id, at)) = &self.ghost {
            let size = g.icon * 1.1;
            s.push_shadow(&[gsk::Shadow::new(gdk::RGBA::new(0.0, 0.0, 0.0, 0.35), 0.0, 6.0, 14.0)]);
            self.icon_of(s, id, at.0 - size / 2.0, at.1 - size / 2.0, size);
            s.pop();
        }
        // What's being dragged, a little larger, under the pointer (the dock draws it over itself).
        if let Some(d) = self.drag.as_ref().filter(|d| !crate::dock::over(d.at, self.size)) {
            let size = g.icon * 1.1;
            let (x, y) = (d.at.0 - size / 2.0, d.at.1 - size / 2.0);
            s.push_shadow(&[gsk::Shadow::new(gdk::RGBA::new(0.0, 0.0, 0.0, 0.35), 0.0, 6.0, 14.0)]);
            match &d.slot {
                Slot::App(id) => self.icon_of(s, id, x, y, size),
                Slot::Folder { apps, .. } => self.folder_face(s, apps, x, y, size, 0.0),
            }
            s.pop();
        }
        if grid_faded {
            s.pop();
        }
        s.restore();
        self.last = now;
        // Icons still to be drawn for the pages in view.
        let around = (page.floor().max(0.0) as usize * g.per_page()).saturating_sub(g.per_page());
        self.wanted = list.iter().skip(around).take(g.per_page() * 3).any(|slot| match slot {
            Slot::App(id) => !self.art.contains_key(id),
            Slot::Folder { apps, .. } => apps.iter().take(9).any(|a| !self.art.contains_key(a)),
        }) || self.folder_apps().iter().any(|a| !self.art.contains_key(a));
        self.moving = moving;
    }

    /// While dragging: after a moment over a cell the others make way there; held over the
    /// middle of another app instead, it opens to take this one (see `released`).
    fn drag_step(&mut self, now: i64) {
        if self.folder.is_some() {
            return;
        }
        let Some(d) = self.drag.as_ref() else { return };
        let (x, y) = d.at;
        // Over the dock it's going there: the grid stays as it is.
        if crate::dock::over(d.at, self.size) {
            return;
        }
        let key = d.slot.key();
        let is_app = matches!(d.slot, Slot::App(_)) && d.from_folder.is_none();
        let g = self.grid();
        let Some(cell) = g.cell_at(x, y) else {
            if let Some(d) = self.drag.as_mut() {
                d.over = None;
                d.pending = None;
            }
            return;
        };
        let target = (self.current_page() * g.per_page() + cell).min(self.slots.len().saturating_sub(1));
        let from = self.slots.iter().position(|slot| slot.key() == key);
        let (ix, iy) = g.spot(cell);
        let on_icon = (x - (ix + g.icon / 2.0)).abs() < g.icon * 0.42 && (y - (iy + g.icon / 2.0)).abs() < g.icon * 0.42;
        let other = from.is_some_and(|f| f != target);
        let Some(d) = self.drag.as_mut() else { return };
        if is_app && on_icon && other {
            d.pending = None;
            if d.over.map(|o| o.0) != Some(target) {
                d.over = Some((target, now));
            }
            return;
        }
        d.over = None;
        match d.pending {
            Some((t, since)) if t == target => {
                if now - since < 180_000 {
                    return;
                }
            }
            _ => {
                d.pending = Some((target, now));
                return;
            }
        }
        let slot = d.slot.clone();
        match from {
            Some(f) if f != target => {
                let moved = self.slots.remove(f);
                let at = target.min(self.slots.len());
                self.slots.insert(at, moved);
            }
            None => {
                let at = target.min(self.slots.len());
                self.slots.insert(at, slot);
            }
            _ => {}
        }
    }
}

/// Pushes an opacity node for `a`, unless it is fully opaque (an opacity node draws everything
/// under it offscreen first). Returns whether it pushed one, to pop it.
fn fade(s: &gtk::Snapshot, a: f64) -> bool {
    let push = a < 0.999;
    if push {
        s.push_opacity(a);
    }
    push
}

/// Draws the missing icons of the pages in view, a few at a time between frames.
fn fill_art(st: &Shared) {
    let Ok(mut s) = st.try_borrow_mut() else { return };
    if !s.wanted || !s.open || s.art_pending {
        return;
    }
    s.art_pending = true;
    drop(s);
    let st = st.clone();
    glib::idle_add_local_once(move || {
        if let Ok(mut s) = st.try_borrow_mut() {
            s.art_pending = false;
        }
        // What the icons are drawn for: the style, the icon theme and the screen's scale, and not
        // the size, which moves with the dock and would draw them again at every opening. When it
        // has changed since they were drawn, they are all drawn again: this comes before looking
        // for the missing ones, or icons of the old style, none of them missing, would stay.
        let (paint, app, px) = {
            let Ok(s) = st.try_borrow() else { return };
            (s.paint.clone(), s.app.clone(), s.grid().icon.round() as i32)
        };
        let style = crate::art::Style::parse(&app.prefs.borrow().dock.icons);
        let scale = paint.scale_factor().max(1);
        let active = gtk::Settings::default().and_then(|s| s.gtk_icon_theme_name()).unwrap_or_else(|| "Adwaita".into());
        let art_key = format!("{style:?} {active} {scale}");
        let ids = {
            let Ok(mut s) = st.try_borrow_mut() else { return };
            if s.art_key != art_key {
                s.art.clear();
                s.art_key = art_key;
            }
            let g = s.grid();
            let page = s.page.target().max(0.0) as usize;
            let around = (page * g.per_page()).saturating_sub(g.per_page());
            let mut ids: Vec<String> = s.folder_apps();
            for slot in visible(&s).iter().skip(around).take(g.per_page() * 3) {
                match slot {
                    Slot::App(id) => ids.push(id.clone()),
                    Slot::Folder { apps, .. } => ids.extend(apps.iter().take(9).cloned()),
                }
            }
            ids.retain(|id| !s.art.contains_key(id));
            ids.dedup();
            // A few at a time between frames, so the opening keeps moving while they're drawn.
            ids.truncate(8);
            ids
        };
        if ids.is_empty() || paint.native().and_then(|n| n.renderer()).is_none() {
            return;
        }
        let key = crate::icons::DockKey::new(style, &active, px, scale);
        let Some(display) = gdk::Display::default() else { return };
        let theme = crate::icons::dock_theme(&display, &key);
        let list = app.apps.list();
        let accent = app.palette.get().accent;
        for id in ids {
            let entry = list.iter().find(|e| e.id == id);
            let icon = crate::icons::dock_icon(&theme, entry.and_then(|e| e.icon.as_ref()), &id, &key);
            let name = entry.map_or(id.as_str(), |e| e.name.as_str());
            // The original icons are drawn as they are; a missing one gets a plain tile.
            let tex = match (style, icon.as_ref()) {
                (crate::art::Style::Native, Some(p)) => kit::art::plain(&paint, &id, p, px as f64),
                (crate::art::Style::Native, None) => crate::art::texture(&paint, &id, None, name, px as f64, crate::art::Style::Graphite, accent, &app.cfg.font),
                _ => crate::art::texture(&paint, &id, icon.as_ref(), name, px as f64, style, accent, &app.cfg.font),
            };
            if let (Some(tex), Ok(mut s)) = (tex, st.try_borrow_mut()) {
                s.art.insert(id, tex);
            }
        }
        paint.queue_draw();
    });
}

fn kick(st: &Shared) {
    let paint = {
        let mut s = st.borrow_mut();
        s.paint.queue_draw();
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
        if s.open && s.backdrop.is_none() {
            s.backdrop = blurred(&s.app);
        }
        s.drag_step(now);
        let t = s.fade.value(now).clamp(0.0, 1.0);
        s.search.set_opacity(t);
        let folder = s.folder.as_ref().is_some_and(|f| !f.open.settled(now));
        if !s.fade.settled(now) || !s.page.settled(now) || folder || s.drag.is_some() || s.moving || s.feel.busy(now) {
            return glib::ControlFlow::Continue;
        }
        s.ticking = false;
        if !s.open {
            // Closed: keep the icons for next time, let go of the window.
            let window = s.window.clone();
            let art = (std::mem::take(&mut s.art_key), std::mem::take(&mut s.art));
            ICONS.with(|i| *i.borrow_mut() = art);
            s.backdrop = None;
            s.shown.clear();
            draw::forget(&s.paint);
            drop(s);
            window.set_visible(false);
            ONE.with(|one| one.borrow_mut().take());
            window.destroy();
            kit::app::trim_later(1500);
        }
        glib::ControlFlow::Break
    });
}

fn turn_to(page: &mut Tween, to: usize, now: i64) {
    if motion::on(motion::Part::AppGrid) {
        page.go(to as f64, now, PAGE_MS, motion::GENTLE);
    } else {
        page.set(to as f64);
    }
}

/// The pointer is on `hit`.
fn point(st: &Shared, hit: Option<Hit>) {
    let changed = {
        let Ok(mut s) = st.try_borrow_mut() else { return };
        let hit = hit.filter(|h| s.drag.is_none() && matches!(h, Hit::Item(_) | Hit::Dot(_)));
        let changed = s.hover != hit;
        if changed {
            s.hover = hit;
            let keys: Vec<Hit> = hit.into_iter().collect();
            s.feel.hover(&keys, glib::monotonic_time());
        }
        changed
    };
    if changed {
        kick(st);
    }
}

fn turn(st: &Shared, by: i32) {
    let mut s = st.borrow_mut();
    let pages = s.pages() as i32;
    let to = (s.current_page() as i32 + by).clamp(0, pages - 1);
    let now = glib::monotonic_time();
    turn_to(&mut s.page, to as usize, now);
    // Other items are under the pointer now; they start as they are.
    s.feel.retain(|h| !matches!(h, Hit::Item(_)));
    s.hover = None;
    drop(s);
    kick(st);
}

fn launch(st: &Shared, id: &str) {
    let app = st.borrow().app.clone();
    if let Some(e) = app.apps.list().iter().find(|e| e.id == id) {
        app.apps.launch(e);
    }
    close(st);
}

fn open_folder(st: &Shared, index: usize) {
    let mut s = st.borrow_mut();
    let mut open = Tween::still(0.0);
    open.go(1.0, glib::monotonic_time(), if motion::reduced() { 1 } else { 260 }, motion::EMPHASIZED_DECEL);
    s.folder = Some(Folder { index, open, renaming: false });
    drop(s);
    kick(st);
}

fn close_folder(st: &Shared) {
    let mut s = st.borrow_mut();
    s.folder = None;
    drop(s);
    save(&st.borrow().slots);
    kick(st);
}

fn moved(st: &Shared, x: f64, y: f64) {
    let mut s = st.borrow_mut();
    let now = glib::monotonic_time();
    if s.drag.is_none() {
        let Some((px, py, Hit::Item(i))) = s.press else { return };
        if (x - px).hypot(y - py) < 8.0 || !s.query.trim().is_empty() {
            return;
        }
        // Lift the item: out of the open folder, or out of the grid.
        let drag = if let Some(f) = s.folder.as_ref().map(|f| f.index) {
            let id = s.folder_apps().get(i).cloned();
            id.map(|id| Drag { slot: Slot::App(id), from_folder: Some(f), at: (x, y), over: None, edge: None, pending: None })
        } else {
            s.slots.get(i).cloned().map(|slot| Drag { slot, from_folder: None, at: (x, y), over: None, edge: None, pending: None })
        };
        s.drag = drag;
        drop(s);
        kick(st);
        return;
    }
    // Leaving the open folder takes the app out of it.
    if s.drag.as_ref().is_some_and(|d| d.from_folder.is_some())
        && let Some(((px, py, pw, ph), _)) = s.panel()
        && (x < px - 20.0 || x > px + pw + 20.0 || y < py - 20.0 || y > py + ph + 20.0)
    {
                let f = s.drag.as_ref().and_then(|d| d.from_folder).unwrap_or(0);
                let id = s.drag.as_ref().map(|d| d.slot.key()).unwrap_or_default();
                if let Some(Slot::Folder { apps, .. }) = s.slots.get_mut(f) {
                    apps.retain(|a| *a != id);
                }
                let at = (f + 1).min(s.slots.len());
                s.slots.insert(at, Slot::App(id));
                if let Some(Slot::Folder { apps, .. }) = s.slots.get(f)
                    && apps.len() == 1
                {
                    let only = apps[0].clone();
                    s.slots[f] = Slot::App(only);
                }
                s.folder = None;
                if let Some(d) = s.drag.as_mut() {
                    d.from_folder = None;
                }
    }
    if let Some(d) = s.drag.as_mut() {
        d.at = (x, y);
    }
    // Over the dock, the dock shows it where it would land, drawn over the grid.
    let hand = match s.drag.as_ref() {
        Some(Drag { slot: Slot::App(id), .. }) if crate::dock::over((x, y), s.size) => Some((id.clone(), s.size)),
        _ => None,
    };
    drop(s);
    match &hand {
        Some((id, screen)) => crate::dock::incoming(Some((id, (x, y), *screen))),
        None => crate::dock::incoming(None),
    }
    let mut s = st.borrow_mut();
    if s.folder.is_some() {
        return;
    }
    // Waiting at a side of the screen turns the page.
    let side = if x < 48.0 { -1 } else if x > s.size.0 - 48.0 { 1 } else { 0 };
    let mut turn_to = None;
    if let Some(d) = s.drag.as_mut() {
        match d.edge {
            Some((dir, since)) if dir == side && side != 0 && now - since > 600_000 => {
                d.edge = Some((side, now));
                turn_to = Some(side);
            }
            Some((dir, _)) if dir == side => {}
            _ => d.edge = (side != 0).then_some((side, now)),
        }
    }
    drop(s);
    if let Some(by) = turn_to {
        turn(st, by);
    }
}

fn released(st: &Shared, x: f64, y: f64, click: bool) {
    let press = st.borrow_mut().press.take();
    let dragging = st.borrow().drag.is_some();
    if dragging {
        let mut s = st.borrow_mut();
        let d = s.drag.take();
        let now = glib::monotonic_time();
        // Let go over the dock: it's kept there, and leaves the grid.
        if let Some(Drag { slot: Slot::App(id), from_folder, at, .. }) = d.as_ref().filter(|d| crate::dock::over(d.at, s.size)) {
            let (id, at, size) = (id.clone(), *at, s.size);
            match from_folder {
                Some(f) => {
                    let f = *f;
                    if let Some(Slot::Folder { apps, .. }) = s.slots.get_mut(f) {
                        apps.retain(|a| *a != id);
                    }
                    // A folder left with one app is that app again.
                    if let Some(Slot::Folder { apps, .. }) = s.slots.get(f)
                        && apps.len() == 1
                    {
                        let only = apps[0].clone();
                        s.slots[f] = Slot::App(only);
                    }
                }
                None => s.slots.retain(|slot| *slot != Slot::App(id.clone())),
            }
            s.folder = None;
            s.moving = true;
            drop(s);
            if !crate::dock::keep_incoming() {
                crate::dock::pin_at(&id, at, size);
            }
            save(&st.borrow().slots);
            kick(st);
            return;
        }
        if let Some(d) = d {
            let key = d.slot.key();
            // It settles into its place from where it was let go.
            let g = s.grid();
            let page = s.page.value(now) * s.size.0;
            s.shown.insert(key.clone(), (d.at.0 - g.icon / 2.0 + page, d.at.1 - g.icon / 2.0));
            s.moving = true;
            if let (Some((target, since)), Slot::App(id)) = (d.over, &d.slot) {
                if now - since > 380_000 {
                    // Dropped onto another item: into its folder, or a new folder of the two.
                    let from = s.slots.iter().position(|slot| slot.key() == key);
                    let app = s.app.clone();
                    if let Some(target_slot) = s.slots.get(target).cloned() {
                        let merged = match target_slot {
                            Slot::Folder { name, mut apps } => {
                                apps.push(id.clone());
                                Slot::Folder { name, apps }
                            }
                            Slot::App(other) => {
                                let apps = vec![other, id.clone()];
                                Slot::Folder { name: folder_name(&app, &apps), apps }
                            }
                        };
                        s.slots[target] = merged;
                        if let Some(f) = from {
                            s.slots.remove(f);
                        }
                    }
                }
            } else if d.from_folder.is_some() {
                // Let go inside the folder: it stays where it was.
            }
        }
        drop(s);
        save(&st.borrow().slots);
        kick(st);
        return;
    }
    if !click {
        return;
    }
    let hit = press.map(|p| p.2).unwrap_or(Hit::Nothing);
    if st.borrow().hit(x, y) != hit {
        return;
    }
    let in_folder = st.borrow().folder.is_some();
    match hit {
        Hit::Item(i) if in_folder => {
            let id = st.borrow().folder_apps().get(i).cloned();
            if let Some(id) = id {
                launch(st, &id);
            }
        }
        Hit::Item(i) => {
            let slot = visible(&st.borrow()).get(i).cloned();
            match slot {
                Some(Slot::App(id)) => launch(st, &id),
                Some(Slot::Folder { .. }) => open_folder(st, i),
                None => {}
            }
        }
        Hit::Title => {
            if let Some(f) = st.borrow_mut().folder.as_mut() {
                f.renaming = true;
            }
            kick(st);
        }
        Hit::Dot(p) => {
            let by = p as i32 - st.borrow().current_page() as i32;
            turn(st, by);
        }
        Hit::Panel | Hit::Search => {}
        Hit::Nothing if in_folder => close_folder(st),
        Hit::Nothing => close(st),
    }
}

/// The grid's keys; false leaves the key to the search field.
fn press_key(st: &Shared, key: gdk::Key) -> bool {
    let renaming = st.borrow().folder.as_ref().is_some_and(|f| f.renaming);
    let text = key.to_unicode().filter(|c| !c.is_control());
    if renaming {
        let mut s = st.borrow_mut();
        let index = s.folder.as_ref().map_or(0, |f| f.index);
        if let Some(Slot::Folder { name, .. }) = s.slots.get_mut(index) {
            match key {
                gdk::Key::BackSpace => {
                    name.pop();
                }
                gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::Escape => {
                    if name.trim().is_empty() {
                        *name = "Folder".into();
                    }
                    if let Some(f) = s.folder.as_mut() {
                        f.renaming = false;
                    }
                }
                _ => {
                    if let Some(c) = text
                        && name.chars().count() < 32
                    {
                        name.push(c);
                    }
                }
            }
        }
        drop(s);
        save(&st.borrow().slots);
        kick(st);
        return true;
    }
    let g = st.borrow().grid();
    let (folder, query) = {
        let s = st.borrow();
        (s.folder.is_some(), !s.query.is_empty())
    };
    match key {
        gdk::Key::Escape => {
            if folder {
                close_folder(st);
            } else if query {
                let search = st.borrow().search.clone();
                search.set_text("");
            } else {
                close(st);
            }
        }
        gdk::Key::Return | gdk::Key::KP_Enter => {
            let (slot, folder) = {
                let s = st.borrow();
                let list = visible(&s);
                let Some(i) = s.selected else { return true };
                (list.get(i).cloned(), s.query.is_empty().then_some(i))
            };
            match slot {
                Some(Slot::App(id)) => launch(st, &id),
                Some(Slot::Folder { .. }) => open_folder(st, folder.unwrap_or(0)),
                None => {}
            }
        }
        gdk::Key::Page_Down => turn(st, 1),
        gdk::Key::Page_Up => turn(st, -1),
        // While searching, left and right move in the text.
        gdk::Key::Left | gdk::Key::Right if query => return false,
        gdk::Key::Left | gdk::Key::Right | gdk::Key::Up | gdk::Key::Down => {
            let mut s = st.borrow_mut();
            let len = visible(&s).len();
            if len == 0 || s.folder.is_some() {
                return true;
            }
            let at = s.selected.unwrap_or(s.current_page() * g.per_page()) as i64;
            let step = match key {
                gdk::Key::Left => -1,
                gdk::Key::Right => 1,
                gdk::Key::Up => -(g.cols as i64),
                _ => g.cols as i64,
            };
            let next = if s.selected.is_none() { at } else { (at + step).clamp(0, len as i64 - 1) } as usize;
            s.selected = Some(next);
            let page = next / g.per_page();
            let now = glib::monotonic_time();
            if page != s.current_page() {
                turn_to(&mut s.page, page, now);
            }
            drop(s);
            kick(st);
        }
        // A folder takes no typing (but its name, above); the search field takes the rest.
        _ => return folder && text.is_some(),
    }
    true
}

/// Whether app `id` can be removed from the system: a system package or a Flatpak, which
/// Settings knows how to take out (with its checks).
fn removable(app: &App, id: &str) -> bool {
    let list = app.apps.list();
    let Some(path) = list.iter().find(|e| e.id == id).and_then(|e| e.info.downcast_ref::<gio_unix::DesktopAppInfo>().and_then(|d| d.filename())) else { return false };
    let path = path.to_string_lossy();
    path.contains("/flatpak/exports/share/applications/") || path.starts_with("/usr/share/applications/")
}

fn context_menu(st: &Shared, x: f64, y: f64) {
    use crate::popmenu::Row;
    let (hit, app, in_folder) = {
        let s = st.borrow();
        (s.hit(x, y), s.app.clone(), s.folder.is_some())
    };
    let Hit::Item(i) = hit else { return };
    let slot = {
        let s = st.borrow();
        if in_folder { s.folder_apps().get(i).cloned().map(Slot::App) } else { visible(&s).get(i).cloned() }
    };
    let weak = Rc::downgrade(st);
    let with = move |f: fn(&Shared, &str), arg: String| {
        let weak = weak.clone();
        move || {
            if let Some(st) = weak.upgrade() {
                f(&st, &arg);
            }
        }
    };
    let rows = match slot {
        Some(Slot::App(id)) => {
            let docked = crate::dock::pinned().contains(&id);
            let mut rows = vec![
                Row::item("Open", with(launch, id.clone())).glyph("open_in_new"),
                if docked { Row::item("Remove from Dock", with(|_, id| crate::dock::unpin(id), id.clone())).glyph("keep_off") } else { Row::item("Keep in Dock", with(keep_in_dock, id.clone())).glyph("keep") },
                Row::item("Hide", with(hide, id.clone())).glyph("visibility_off").hint("Settings › Apps shows it again"),
            ];
            if removable(&app, &id) {
                rows.push(Row::Sep);
                rows.push(Row::item("Uninstall…", with(uninstall, id.clone())).glyph("delete").danger());
            }
            rows
        }
        Some(Slot::Folder { .. }) => {
            let index = i.to_string();
            vec![
                Row::item("Rename", with(rename_folder, index.clone())).glyph("edit"),
                Row::item("Ungroup", with(ungroup, index)).glyph("folder_off"),
            ]
        }
        None => return,
    };
    crate::popmenu::show(&app, app.primary(), x, y, "", rows);
}

fn keep_in_dock(st: &Shared, id: &str) {
    crate::dock::pin(id);
    let mut s = st.borrow_mut();
    // Kept in the dock, it leaves the grid (the search still finds it).
    for slot in s.slots.iter_mut() {
        if let Slot::Folder { apps, .. } = slot {
            apps.retain(|a| a != id);
        }
    }
    s.slots.retain(|slot| *slot != Slot::App(id.to_string()));
    let slots = std::mem::take(&mut s.slots);
    s.slots = slots.into_iter().map(|slot| match slot {
        Slot::Folder { apps, .. } if apps.len() == 1 => Slot::App(apps[0].clone()),
        other => other,
    }).collect();
    s.moving = true;
    drop(s);
    save(&st.borrow().slots);
    kick(st);
}

fn hide(st: &Shared, id: &str) {
    if let Err(e) = kit::hidden::save(id, true) {
        eprintln!("digit: {e}");
        return;
    }
    keep_out(st, id);
}

/// Takes app `id` out of the grid, its folders included.
fn keep_out(st: &Shared, id: &str) {
    let mut s = st.borrow_mut();
    let slots = std::mem::take(&mut s.slots);
    s.slots = slots
        .into_iter()
        .filter_map(|slot| match slot {
            Slot::App(a) if a == id => None,
            Slot::Folder { name, apps } => {
                let apps: Vec<String> = apps.into_iter().filter(|a| a != id).collect();
                match apps.len() {
                    0 => None,
                    1 => Some(Slot::App(apps[0].clone())),
                    _ => Some(Slot::Folder { name, apps }),
                }
            }
            other => Some(other),
        })
        .collect();
    s.moving = true;
    drop(s);
    save(&st.borrow().slots);
    kick(st);
}

fn uninstall(st: &Shared, id: &str) {
    let _ = crate::child::spawn(&["digit-settings".as_ref(), format!("uninstall:{id}").as_ref()], gtk::gio::SubprocessFlags::NONE);
    close(st);
}

fn rename_folder(st: &Shared, index: &str) {
    let Ok(i) = index.parse::<usize>() else { return };
    open_folder(st, i);
    if let Some(f) = st.borrow_mut().folder.as_mut() {
        f.renaming = true;
    }
    kick(st);
}

fn ungroup(st: &Shared, index: &str) {
    let Ok(i) = index.parse::<usize>() else { return };
    let mut s = st.borrow_mut();
    if let Some(Slot::Folder { apps, .. }) = s.slots.get(i).cloned() {
        s.slots.splice(i..=i, apps.into_iter().map(Slot::App));
        s.moving = true;
    }
    drop(s);
    save(&st.borrow().slots);
    kick(st);
}

/// An app being dragged out of the dock (`Some`: its id, and where it is on a screen of a given
/// size), drawn over the open grid; `None` once it's let go or back over the dock.
pub type Place = Box<dyn Fn((f64, f64)) -> (f64, f64)>;

pub fn ghost(item: Option<(String, Place)>) {
    let Some(st) = ONE.with(|one| one.borrow().clone()) else { return };
    let Ok(mut s) = st.try_borrow_mut() else { return };
    if !s.open {
        return;
    }
    let size = s.size;
    s.ghost = item.map(|(id, at)| (id, at(size)));
    s.paint.queue_draw();
}

/// An app dragged out of the dock was let go over the grid: it goes in where it was let go.
/// Returns whether the grid was open to take it. `at` gives the point on a screen of a size.
pub fn take_from_dock(id: &str, at: impl Fn((f64, f64)) -> (f64, f64)) -> bool {
    let Some(st) = ONE.with(|one| one.borrow().clone()) else { return false };
    let Ok(mut s) = st.try_borrow_mut() else { return false };
    if !s.open || s.app.hidden(id) {
        s.ghost = None;
        return false;
    }
    s.ghost = None;
    let (x, y) = at(s.size);
    let g = s.grid();
    let page = s.current_page();
    let index = g.cell_at(x, y).map_or(s.slots.len(), |c| (page * g.per_page() + c).min(s.slots.len()));
    s.slots.retain(|slot| *slot != Slot::App(id.to_string()));
    let index = index.min(s.slots.len());
    s.slots.insert(index, Slot::App(id.to_string()));
    // It settles into its place from where it was let go.
    let offset = s.page.value(glib::monotonic_time()) * s.size.0;
    s.shown.insert(id.to_string(), (x - g.icon / 2.0 + offset, y - g.icon / 2.0));
    s.moving = true;
    let slots = s.slots.clone();
    drop(s);
    save(&slots);
    kick(&st);
    true
}

/// Closes the grid if it's open, as launching from the dock does.
pub fn close_if_open() {
    if let Some(st) = ONE.with(|one| one.borrow().clone()) {
        close(&st);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_arrangement_follows_installed_apps() {
        let installed: Vec<String> = ["a", "b", "c", "d"].iter().map(|s| s.to_string()).collect();
        let saved = vec![
            Slot::App("c".into()),
            Slot::Folder { name: "Tools".into(), apps: vec!["a".into(), "gone".into()] },
            Slot::App("gone".into()),
            Slot::Folder { name: "Two".into(), apps: vec!["b".into(), "d".into()] },
        ];
        let out = sync(saved, &installed);
        assert_eq!(out, vec![Slot::App("c".into()), Slot::App("a".into()), Slot::Folder { name: "Two".into(), apps: vec!["b".into(), "d".into()] }]);
        // New apps join at the end, in the order given.
        let fresh = sync(Vec::new(), &installed);
        assert_eq!(fresh.len(), 4);
        assert_eq!(fresh[0], Slot::App("a".into()));
        // Saved as JSON, read back the same.
        let text = serde_json::to_string(&out).unwrap();
        assert_eq!(serde_json::from_str::<Vec<Slot>>(&text).unwrap(), out);
    }

    #[test]
    fn grid_fits_the_screen_and_finds_its_cells() {
        for (w, h) in [(1920.0, 1080.0), (1366.0, 768.0), (2560.0, 1440.0)] {
            let g = grid(w, h);
            assert!(g.x0 >= 0.0 && g.x0 + g.cols as f64 * g.cell_w <= w);
            assert!(g.y0 + g.rows as f64 * g.cell_h <= h - DOTS);
            let (x, y) = g.spot(g.cols + 1);
            assert_eq!(g.cell_at(x + g.icon / 2.0, y + g.icon / 2.0), Some(g.cols + 1));
            assert_eq!(g.spot(g.per_page()), g.spot(0), "pages repeat the same cells");
        }
    }
}
