//! Desktop files live on Bottom, above the Background wallpaper and below windows.
//! The wallpaper currently has no pointer controllers. This surface owns empty-desktop
//! clicks too, so rubber-band selection and external file drops work anywhere.
use crate::{
    app::App,
    draw::{self, Face},
    paint::Paint,
    popmenu::{self, Row},
};
use gtk::{gdk, gio, glib, graphene, pango, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kit::motion::{self, Tween};
use kit::palette::Rgb;
use serde::{Deserialize, Serialize};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

const CELL_W: i32 = 112;
const CELL_H: i32 = 116;
const PAD: i32 = 24;
const ICON: i32 = 64;
const LABEL_INSET: i32 = 6;
type Positions = BTreeMap<String, FreePos>;

/// Logical pixels of the icon cell's top-left on the desktop surface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
struct FreePos {
    x: f64,
    y: f64,
    #[serde(default)]
    page: u32,
}
impl FreePos {
    fn rect(self) -> Rect {
        Rect {
            x: self.x,
            y: self.y,
            w: CELL_W as f64,
            h: CELL_H as f64,
        }
    }
    fn overlaps(self, other: Self) -> bool {
        self.page == other.page && self.rect().intersects(other.rect())
    }
}

#[derive(Clone, Copy)]
struct Grid {
    cols: u32,
    rows: u32,
    width: i32,
    height: i32,
    top: i32,
    reserved: Option<Rect>,
}
impl Grid {
    fn new(width: i32, height: i32, top: i32) -> Self {
        let top = top.max(PAD).min((height - CELL_H).max(0));
        Self {
            cols: ((width - 2 * PAD) / CELL_W).max(1) as u32,
            rows: ((height - top - 88) / CELL_H).max(1) as u32,
            width,
            height,
            top,
            reserved: None,
        }
    }
    fn clamp(self, p: FreePos) -> FreePos {
        FreePos {
            x: p.x.clamp(0.0, (self.width - CELL_W).max(0) as f64),
            y: p.y
                .clamp(self.top as f64, (self.height - CELL_H).max(self.top) as f64),
            page: p.page.min(u32::MAX - 1),
        }
    }
    // The grid is only an automatic-placement policy; manual positions never snap.
    fn nth(self, n: usize) -> FreePos {
        let n = n as u32;
        self.clamp(FreePos {
            x: (self.width - PAD - CELL_W - ((n / self.rows) % self.cols) as i32 * CELL_W) as f64,
            y: (self.top + (n % self.rows) as i32 * CELL_H) as f64,
            page: n / (self.rows * self.cols),
        })
    }
    fn reserved(self, p: FreePos) -> bool {
        self.reserved.is_some_and(|r| r.intersects(p.rect()))
    }
    fn automatic_cells(self) -> Vec<FreePos> {
        (0..(self.cols * self.rows) as usize)
            .map(|n| self.nth(n))
            .filter(|p| !self.reserved(*p))
            .collect()
    }
}

/// Restore free positions, including overlaps, before assigning automatic cells.
/// Persisting the result makes later additions independent of the current name order.
fn place(names: &[String], saved: &Positions, grid: Grid) -> Positions {
    let mut cells = grid.automatic_cells();
    // On very small displays there may be no room beside the card. Keep icons reachable.
    if cells.is_empty() {
        cells.extend((0..(grid.cols * grid.rows) as usize).map(|n| grid.nth(n)));
    }
    let mut out = Positions::new();
    for name in names {
        if let Some(&p) = saved.get(name)
            && p.x.is_finite()
            && p.y.is_finite()
        {
            out.insert(name.clone(), grid.clamp(p));
        }
    }
    let mut next = 0;
    for name in names {
        if !out.contains_key(name) {
            // Rejected cells stay blocked as icons are added. Scan each candidate once.
            // With at most out.len() occupied pages, one additional page is always free.
            let (n, p) = (next..(out.len() + 1) * cells.len())
                .map(|n| {
                    (
                        n,
                        FreePos {
                            page: (n / cells.len()) as u32,
                            ..cells[n % cells.len()]
                        },
                    )
                })
                .find(|(_, p)| !out.values().any(|other| p.overlaps(*other)))
                .expect("one more page than occupied icons");
            next = n + 1;
            out.insert(name.clone(), p);
        }
    }
    out
}

#[derive(Clone, Copy, Debug)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}
impl Rect {
    fn between(a: (f64, f64), b: (f64, f64)) -> Self {
        Self {
            x: a.0.min(b.0),
            y: a.1.min(b.1),
            w: (a.0 - b.0).abs(),
            h: (a.1 - b.1).abs(),
        }
    }
    fn intersects(self, b: Self) -> bool {
        self.x < b.x + b.w && self.x + self.w > b.x && self.y < b.y + b.h && self.y + self.h > b.y
    }
    fn contains(self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}
fn rubber_select(
    rect: Rect,
    items: &[(String, Rect)],
    initial: &BTreeSet<String>,
    additive: bool,
) -> BTreeSet<String> {
    let mut selected = if additive {
        initial.clone()
    } else {
        BTreeSet::new()
    };
    selected.extend(
        items
            .iter()
            .filter(|(_, r)| rect.intersects(*r))
            .map(|(n, _)| n.clone()),
    );
    selected
}
fn encode(positions: &Positions) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec_pretty(positions)
}
fn decode(bytes: &[u8]) -> Result<Positions, serde_json::Error> {
    let entries: BTreeMap<String, serde_json::Value> = serde_json::from_slice(bytes)?;
    Ok(entries
        .into_iter()
        .filter_map(|(name, value)| {
            // Old col/row entries (and malformed entries) have no free position to restore.
            let p: FreePos = serde_json::from_value(value).ok()?;
            (p.x.is_finite() && p.y.is_finite()).then_some((name, p))
        })
        .collect())
}
fn state_path() -> PathBuf {
    kit::theme::state_dir().join("desktop-icons.json")
}
fn read_positions(path: &Path) -> Result<Positions, String> {
    // Missing, unreadable or corrupt state always starts from the default layout.
    Ok(std::fs::read(path)
        .ok()
        .and_then(|bytes| decode(&bytes).ok())
        .unwrap_or_default())
}
fn write_positions(path: &Path, positions: &Positions) -> Result<(), String> {
    use std::io::Write;
    let bytes = encode(positions).map_err(|e| e.to_string())?;
    let parent = path.parent().ok_or("Missing state directory")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let tmp = parent.join(format!(
        ".desktop-icons-{}-{}.tmp",
        std::process::id(),
        glib::random_int()
    ));
    let result = (|| -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map_err(|e| e.to_string())
}

#[derive(Clone)]
struct FileData {
    name: String,
    path: PathBuf,
    label: String,
    icon: Option<String>,
    directory: bool,
    launcher: bool,
}
// Preserve distinct Unix filenames even when their display names contain replacement
// characters. NUL cannot occur in a filename, so this prefix cannot collide with UTF-8.
fn name_key(name: &std::ffi::OsStr) -> String {
    name.to_str().map(str::to_owned).unwrap_or_else(|| {
        let mut key = String::from("\0");
        use std::fmt::Write;
        for byte in name.as_encoded_bytes() {
            let _ = write!(key, "{byte:02x}");
        }
        key
    })
}
fn scan(path: &Path) -> Result<Vec<FileData>, String> {
    std::fs::create_dir_all(path).map_err(|e| e.to_string())?;
    let folder = gio::File::for_path(path);
    let files = folder.enumerate_children("standard::name,standard::display-name,standard::is-hidden,standard::type,standard::icon", gio::FileQueryInfoFlags::NONE, gio::Cancellable::NONE).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for info in files {
        let info = info.map_err(|e| e.to_string())?;
        if info.is_hidden() || info.name().as_os_str().as_encoded_bytes().starts_with(b".") {
            continue;
        }
        let file = path.join(info.name());
        let launcher = file.extension().is_some_and(|e| e == "desktop");
        let app = launcher
            .then(|| gio_unix::DesktopAppInfo::from_filename(&file))
            .flatten();
        out.push(FileData {
            name: name_key(info.name().as_os_str()),
            path: file,
            label: app
                .as_ref()
                .map(|a| a.display_name().to_string())
                .unwrap_or_else(|| info.display_name().to_string()),
            icon: app
                .as_ref()
                .and_then(|a| a.icon())
                .or_else(|| info.icon())
                .and_then(|i| i.to_string())
                .map(|i| i.to_string()),
            directory: info.file_type() == gio::FileType::Directory,
            launcher,
        });
    }
    out.sort_by_cached_key(|f| (f.label.to_lowercase(), f.name.clone()));
    Ok(out)
}
struct Item {
    data: FileData,
    icon: gdk::Paintable,
    label: pango::Layout,
    x: Tween,
    y: Tween,
    selected: Tween,
}
impl Item {
    fn rect(&self, now: i64) -> Rect {
        Rect {
            x: self.x.value(now),
            y: self.y.value(now),
            w: CELL_W as f64,
            h: CELL_H as f64,
        }
    }
}
struct Rubber {
    start: (f64, f64),
    end: (f64, f64),
    initial: BTreeSet<String>,
    additive: bool,
}
struct Drag {
    start: (f64, f64),
    origins: BTreeMap<String, (f64, f64)>,
}
impl Drag {
    /// Always translate from the press, including motions before GTK starts DND.
    fn at(&self, x: f64, y: f64) -> BTreeMap<String, (f64, f64)> {
        let (dx, dy) = (x - self.start.0, y - self.start.1);
        self.origins
            .iter()
            .map(|(name, &(ox, oy))| (name.clone(), (ox + dx, oy + dy)))
            .collect()
    }
    fn restored(&self) -> BTreeMap<String, (f64, f64)> {
        self.at(self.start.0, self.start.1)
    }
    fn dropped(&self, grid: Grid, x: f64, y: f64, page: u32) -> Positions {
        let mut dx = x - self.start.0;
        let mut dy = y - self.start.1;
        let mut min_x = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for &(ox, oy) in self.origins.values() {
            min_x = min_x.min(ox);
            max_x = max_x.max(ox);
            min_y = min_y.min(oy);
            max_y = max_y.max(oy);
        }
        let right = (grid.width - CELL_W).max(0) as f64;
        let bottom = (grid.height - CELL_H).max(grid.top) as f64;
        // One translation preserves relative offsets even when dropped at an edge.
        // If a screen change made the group larger than the output, individual
        // clamping below takes priority over preserving those offsets.
        if max_x - min_x <= right {
            dx = dx.clamp(-min_x, right - max_x);
        }
        if max_y - min_y <= bottom - grid.top as f64 {
            dy = dy.clamp(grid.top as f64 - min_y, bottom - max_y);
        }
        self.origins
            .iter()
            .map(|(name, &(ox, oy))| {
                (
                    name.clone(),
                    grid.clamp(FreePos {
                        x: ox + dx,
                        y: oy + dy,
                        page,
                    }),
                )
            })
            .collect()
    }
}
// Prepared on press; only DragSource may turn this into a system drag. Keeping
// the original geometry here avoids re-hit-testing icons already following us.
struct DesktopDrag {
    movement: Drag,
    anchor: String,
    content: gdk::ContentProvider,
    preview: gdk::Paintable,
    visual: Option<gtk::Picture>,
    local_preview: bool,
    over_desktop: bool,
    hotspot: (i32, i32),
    pointer: (f64, f64),
    end: DragEnd,
    system: Option<gdk::Drag>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum DragEnd {
    #[default]
    Active,
    Performed,
    Local,
    Ended,
}
#[derive(Clone, Copy)]
enum DragEvent {
    Performed,
    LocalDrop,
    Finished { moved: bool },
    Cancel,
    SourceEnd,
}
#[derive(Debug, PartialEq)]
enum DragResult {
    Restore,
    KeepLocal,
    AwaitRemoval,
}
fn drag_items_visible(system: bool, local_preview: bool, over_desktop: bool, end: DragEnd) -> bool {
    !system || end == DragEnd::Local || (local_preview && over_desktop)
}
impl DragEnd {
    fn event(&mut self, event: DragEvent) -> Option<DragResult> {
        if *self == Self::Ended {
            return None;
        }
        let result = match event {
            DragEvent::Performed => {
                if *self != Self::Local {
                    *self = Self::Performed;
                }
                return None;
            }
            DragEvent::LocalDrop => {
                *self = Self::Local;
                return None;
            }
            // GtkDragSource emits this inside GDK's terminal signal. It is not
            // evidence of success: handling drag-cancel also sets delete_data.
            DragEvent::SourceEnd => return None,
            _ if *self == Self::Local => DragResult::KeepLocal,
            DragEvent::Finished { moved: true } => DragResult::AwaitRemoval,
            _ => DragResult::Restore,
        };
        *self = Self::Ended;
        Some(result)
    }
}

/// Include the tile and text shadow, and keep fractional grabs despite GDK's
/// integer hotspot. Contents are translated by hot - pointer when composed.
fn preview_bounds(
    origins: &BTreeMap<String, (f64, f64)>,
    pointer: (f64, f64),
) -> (Rect, (i32, i32)) {
    let min_x = origins
        .values()
        .map(|p| p.0)
        .fold(pointer.0, f64::min)
        .floor()
        - 8.0;
    let min_y = origins
        .values()
        .map(|p| p.1)
        .fold(pointer.1, f64::min)
        .floor()
        - 8.0;
    let max_x = origins
        .values()
        .map(|p| p.0 + CELL_W as f64)
        .fold(pointer.0, f64::max)
        .ceil()
        + 8.0;
    let max_y = origins
        .values()
        .map(|p| p.1 + CELL_H as f64)
        .fold(pointer.1, f64::max)
        .ceil()
        + 8.0;
    (
        Rect {
            x: min_x,
            y: min_y,
            w: max_x - min_x,
            h: max_y - min_y,
        },
        (
            (pointer.0 - min_x).round() as i32,
            (pointer.1 - min_y).round() as i32,
        ),
    )
}
struct Desktop {
    // The item being renamed: its label is the editor while that lasts.
    renaming: Option<String>,
    app: std::rc::Weak<App>,
    window: gtk::Window,
    paint: Paint,
    monitor: gdk::Monitor,
    folder: PathBuf,
    grid: Grid,
    geometry: (i32, i32, i32),
    items: Vec<Item>,
    positions: Positions,
    saved: Positions,
    selected: BTreeSet<String>,
    page: u32,
    rubber: Option<Rubber>,
    drag: Option<DesktopDrag>,
    // Successful outgoing MOVE items stay invisible until a scan sees them gone.
    moved_out: BTreeSet<String>,
    importing: bool,
    pending_imports: Positions,
    ticking: bool,
    loading: bool,
    dirty: bool,
    writable: bool,
    closed: bool,
    monitor_file: Option<gio::FileMonitor>,
    pending: Option<glib::SourceId>,
    rename_window: Option<gtk::Window>,
    rename_pending: Option<String>,
}
thread_local! {
    static DESKTOP: RefCell<Option<Rc<RefCell<Desktop>>>> = const { RefCell::new(None) };
    // One serial writer across surface replacements; the newest queued state wins.
    static WRITING: Cell<bool> = const { Cell::new(false) };
    static TO_WRITE: RefCell<Option<Positions>> = const { RefCell::new(None) };
}

/// Renaming happens in the icon's label: plain text with a thin plate, no field around it.
pub fn style(display: &gdk::Display) {
    let css = gtk::CssProvider::new();
    css.load_from_string(
        "entry.desk-rename { background: rgba(0,0,0,0.58); color: white; caret-color: white; border: none; outline: none; box-shadow: none; \
         border-radius: 6px; min-height: 0; padding: 1px 6px; margin: 0; font-size: 13px; font-weight: 520; } \
         entry.desk-rename text selection { background: rgba(255,255,255,0.32); color: white; } \
         entry.desk-rename.error { box-shadow: inset 0 0 0 1px rgb(255,69,58); } \
         label.desk-rename-error { color: white; font-size: 12px; text-shadow: 0 1px 3px rgba(0,0,0,0.6); }",
    );
    gtk::style_context_add_provider_for_display(
        display,
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

pub fn apply(app: &Rc<App>) {
    let enabled = app.prefs.borrow().desktop_icons;
    let target = gdk::Display::default().and_then(|d| {
        let monitors: Vec<_> = d
            .monitors()
            .iter::<gdk::Monitor>()
            .filter_map(Result::ok)
            .collect();
        let primary = kit::prefs::text(&kit::prefs::read(), "primary_monitor", "");
        monitors
            .iter()
            .find(|m| m.connector().as_deref() == Some(primary.as_str()))
            .cloned()
            .or_else(|| monitors.first().cloned())
    });
    // Never keep the thread-local slot borrowed while operating a surface: GTK may
    // synchronously call back into apply() during focus, unparent or destruction.
    let current = DESKTOP.with(|d| d.borrow().clone());
    if current
        .as_ref()
        .is_some_and(|d| d.try_borrow_mut().is_err())
    {
        let app = app.clone();
        glib::idle_add_local_once(move || apply(&app));
        return;
    }
    let keep = enabled
        && target.as_ref().is_some_and(|m| {
            current.as_ref().is_some_and(|d| {
                let (monitor, geometry, paint) = {
                    let Ok(d) = d.try_borrow() else { return false };
                    (d.monitor.clone(), d.geometry, d.paint.clone())
                };
                let g = m.geometry();
                monitor == *m
                    && geometry == (g.width(), g.height(), app.reserve())
                    && paint.scale_factor() == m.scale_factor()
            })
        });
    if keep {
        if let Some(d) = current {
            {
                let Ok(mut state) = d.try_borrow_mut() else {
                    return;
                };
                if state.drag.is_none() && state.rubber.is_none() && !state.importing {
                    state.layout(true);
                } else {
                    state.dirty = true;
                }
            }
            save(&d);
            animate(&d);
        }
        return;
    }
    let old = DESKTOP.with(|d| d.borrow_mut().take());
    if let Some(old) = old {
        let (monitor, pending, window) = {
            let mut old = old.borrow_mut();
            old.closed = true;
            (
                old.monitor_file.take(),
                old.pending.take(),
                old.window.clone(),
            )
        };
        if let Some(m) = monitor {
            m.cancel();
        }
        if let Some(id) = pending {
            id.remove();
        }
        close_rename(&old, None);
        window.destroy();
    }
    if !enabled {
        return;
    }
    let Some(monitor) = target else { return };
    let g = monitor.geometry();
    let window = gtk::Window::new();
    window.init_layer_shell();
    window.set_layer(Layer::Bottom);
    window.set_namespace(Some("digit-desktop-icons"));
    window.set_monitor(Some(&monitor));
    // Never takes the keyboard: holding it on this layer keeps new windows from getting focus
    // until the pointer moves. Menus and rename open on their own surfaces instead.
    window.set_keyboard_mode(KeyboardMode::None);
    window.set_exclusive_zone(-1);
    for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(edge, true);
    }
    window.add_css_class("digit");
    let paint = Paint::new(1, 1);
    paint.set_hexpand(true);
    paint.set_vexpand(true);
    window.set_child(Some(&paint));
    let folder = glib::user_special_dir(glib::UserDirectory::Desktop)
        .unwrap_or_else(|| glib::home_dir().join("Desktop"));
    let desktop = Rc::new(RefCell::new(Desktop {
        app: Rc::downgrade(app),
        window: window.clone(),
        paint: paint.clone(),
        monitor,
        folder,
        grid: Grid::new(g.width(), g.height(), app.reserve() + 24),
        geometry: (g.width(), g.height(), app.reserve()),
        items: Vec::new(),
        positions: Positions::new(),
        saved: Positions::new(),
        selected: BTreeSet::new(),
        page: 0,
        rubber: None,
        drag: None,
        moved_out: BTreeSet::new(),
        importing: false,
        pending_imports: Positions::new(),
        ticking: false,
        loading: false,
        dirty: false,
        writable: true,
        closed: false,
        monitor_file: None,
        pending: None,
        rename_window: None,
        renaming: None,
        rename_pending: None,
    }));
    let weak = Rc::downgrade(&desktop);
    paint.on_draw(move |s| {
        if let Some(d) = weak.upgrade() {
            draw_desktop(&d, s);
        }
    });
    controllers(&desktop, &paint);
    let theme = gtk::IconTheme::for_display(&paint.display());
    let weak = Rc::downgrade(&desktop);
    let handler = theme.connect_changed(move |_| {
        if let Some(d) = weak.upgrade() {
            refresh(&d);
        }
    });
    let handler = RefCell::new(Some(handler));
    window.connect_destroy(move |_| {
        let id = handler.borrow_mut().take();
        if let Some(id) = id {
            theme.disconnect(id);
        }
    });
    DESKTOP.with(|d| *d.borrow_mut() = Some(desktop.clone()));
    window.present();
    let weak = Rc::downgrade(&desktop);
    glib::spawn_future_local(async move {
        let result = gio::spawn_blocking(|| read_positions(&state_path())).await;
        let Some(d) = weak.upgrade() else { return };
        if d.borrow().closed {
            return;
        }
        match result {
            Ok(Ok(p)) => d.borrow_mut().saved = p,
            other => {
                eprintln!("digit: desktop icon positions could not be read: {other:?}");
                d.borrow_mut().writable = false;
            }
        }
        refresh(&d);
    });
}

impl Desktop {
    fn visible(&self, item: &Item) -> bool {
        !self.moved_out.contains(&item.data.name)
            && !self.drag.as_ref().is_some_and(|drag| {
                !drag_items_visible(drag.system.is_some(), drag.local_preview, drag.over_desktop, drag.end)
                    && drag.movement.origins.contains_key(&item.data.name)
            })
            && self
                .positions
                .get(&item.data.name)
                .is_some_and(|p| p.page == self.page)
    }
    fn hit(&self, x: f64, y: f64) -> Option<usize> {
        let now = glib::monotonic_time();
        self.items
            .iter()
            .rposition(|i| self.visible(i) && i.rect(now).contains(x, y))
    }
    fn raise(&mut self, at: usize) {
        let item = self.items.remove(at);
        self.items.push(item);
    }
    fn sort_by_name(&mut self) {
        self.items
            .sort_by_cached_key(|i| (i.data.label.to_lowercase(), i.data.name.clone()));
        self.saved.clear();
        self.page = 0;
        self.layout(true);
    }
    fn select(&mut self, selected: BTreeSet<String>) {
        self.selected = selected;
        let now = glib::monotonic_time();
        for i in &mut self.items {
            i.selected.go(
                f64::from(self.selected.contains(&i.data.name)),
                now,
                160,
                motion::STANDARD,
            );
        }
    }
    fn layout(&mut self, animate: bool) {
        let mut ordered: Vec<_> = self.items.iter().map(|i| &i.data).collect();
        ordered.sort_by_cached_key(|data| (data.label.to_lowercase(), data.name.clone()));
        let names: Vec<_> = ordered.into_iter().map(|data| data.name.clone()).collect();
        self.grid.reserved = self.app.upgrade().and_then(|app| {
            let prefs = app.prefs.borrow();
            if !app.cfg.wallpaper {
                return None;
            }
            crate::batterywidget::card_rect(
                &prefs.desktop_battery,
                crate::batterywidget::SHOWN,
                (self.geometry.0, self.geometry.1),
                app.reserve(),
                &prefs.desktop_widget,
            )
            .map(|r| Rect {
                x: r.x,
                y: r.y,
                w: r.w,
                h: r.h,
            })
        });
        self.positions = place(&names, &self.saved, self.grid);
        // Save automatic placements too: new files must never reshuffle existing icons.
        self.saved = self.positions.clone();
        for (name, &pos) in &self.pending_imports {
            self.saved.entry(name.clone()).or_insert(pos);
        }
        self.page = self
            .page
            .min(self.positions.values().map(|p| p.page).max().unwrap_or(0));
        let now = glib::monotonic_time();
        for i in &mut self.items {
            let FreePos { x, y, .. } = self.positions[&i.data.name];
            if animate {
                i.x.go(x, now, 180, motion::STANDARD);
                i.y.go(y, now, 180, motion::STANDARD);
            } else {
                i.x.set(x);
                i.y.set(y);
            }
        }
    }
}

struct ItemFrame {
    icon: gdk::Paintable,
    label: pango::Layout,
    rect: Rect,
    selected: f64,
    editing: bool,
}

fn draw_item(s: &gtk::Snapshot, app: &App, pal: &kit::palette::Palette, item: &ItemFrame) {
    let ItemFrame {
        icon,
        label,
        rect: r,
        selected,
        editing,
    } = item;
    let selected = *selected;
    let x = r.x.round();
    let y = r.y.round();
    // Finder's look: a soft tile behind the icon and a pill around the words when selected,
    // white text with a small shadow otherwise, so any wallpaper stays readable.
    let (w, h) = label.pixel_size();
    let (w, h) = (w as f64, h as f64);
    let label_y = y + ICON as f64 + 6.0;
    if *editing {
        draw::paintable(s, icon, x + (CELL_W - ICON) as f64 / 2.0, y, ICON as f64);
        return;
    }
    let plate = 6.0;
    s.push_opacity(selected);
    draw::fill(
        s,
        &draw::round(
            x + (CELL_W - ICON) as f64 / 2.0 - plate,
            y - plate,
            ICON as f64 + 2.0 * plate,
            ICON as f64 + 2.0 * plate,
            12.0 * app.round(),
        ),
        Rgb::new(0.0, 0.0, 0.0).alpha(0.24),
    );
    draw::fill(
        s,
        &draw::round(
            x + (CELL_W as f64 - w) / 2.0 - 6.0,
            label_y - 1.0,
            w + 12.0,
            h + 2.0,
            6.0 * app.round(),
        ),
        pal.accent,
    );
    s.pop();
    draw::paintable(s, icon, x + (CELL_W - ICON) as f64 / 2.0, y, ICON as f64);
    s.save();
    s.translate(&graphene::Point::new(
        (x + LABEL_INSET as f64) as f32,
        label_y as f32,
    ));
    s.push_shadow(&[gtk::gsk::Shadow::new(
        gdk::RGBA::new(0.0, 0.0, 0.0, 0.55 * (1.0 - selected as f32)),
        0.0,
        1.0,
        3.0,
    )]);
    let ink = pal.on_accent.mix(Rgb::new(1.0, 1.0, 1.0), selected);
    s.append_layout(label, &ink.rgba());
    s.pop();
    s.restore();
}

fn draw_desktop(d: &Rc<RefCell<Desktop>>, s: &gtk::Snapshot) {
    let paint = {
        let Ok(state) = d.try_borrow() else { return };
        state.paint.clone()
    };
    let now = paint
        .frame_clock()
        .map(|c| c.frame_time())
        .unwrap_or_else(glib::monotonic_time);
    // Paintables can run their own snapshot code. Clone the visible frame first,
    // so even drawing never calls GTK with the desktop model borrowed.
    let (app, items, rubber, page, pages, grid, geometry) = {
        let Ok(state) = d.try_borrow() else { return };
        let Some(app) = state.app.upgrade() else {
            return;
        };
        let items: Vec<_> = state
            .items
            .iter()
            .filter(|i| state.visible(i))
            .map(|i| ItemFrame {
                icon: i.icon.clone(),
                label: i.label.clone(),
                rect: i.rect(now),
                selected: i.selected.value(now),
                editing: state.renaming.as_ref() == Some(&i.data.name),
            })
            .collect();
        (
            app,
            items,
            state.rubber.as_ref().map(|r| Rect::between(r.start, r.end)),
            state.page,
            state.positions.values().map(|p| p.page).max().unwrap_or(0) + 1,
            state.grid,
            state.geometry,
        )
    };
    let pal = app.palette_at(now);
    // A snapshot without nodes submits no buffer. Keep the full desktop mapped
    // even before the scan finishes, on an empty folder, and on empty pages.
    // Match Paint::present's barely visible fill; alpha zero is optimized away.
    s.append_color(
        &pal.body.alpha(0.004).rgba(),
        &graphene::Rect::new(
            0.0,
            0.0,
            paint.width().max(1) as f32,
            paint.height().max(1) as f32,
        ),
    );
    for item in &items {
        draw_item(s, &app, &pal, item);
    }
    if let Some(r) = rubber {
        draw::fill(
            s,
            &draw::round(r.x, r.y, r.w, r.h, 8.0 * app.round()),
            pal.accent.alpha(0.22),
        );
    }
    if pages > 1 {
        draw::text(
            s,
            &paint,
            &app.cfg,
            &format!("{} / {}", page + 1, pages),
            12.0,
            Face::Number(500),
            pal.ink_main,
            (grid.width - 110) as f64,
            (geometry.1 - 38) as f64,
            80.0,
            pango::Alignment::Center,
        );
    }
}

fn animate(d: &Rc<RefCell<Desktop>>) {
    let (paint, start) = {
        let Ok(mut state) = d.try_borrow_mut() else {
            return;
        };
        if state.closed {
            return;
        }
        let start = !state.ticking;
        state.ticking = true;
        (state.paint.clone(), start)
    };
    paint.queue_draw();
    if !start {
        return;
    }
    let weak = Rc::downgrade(d);
    paint.add_tick_callback(move |paint, clock| {
        let Some(d) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        let now = clock.frame_time();
        let busy = {
            // A nested frame may run while a callback is updating the model.
            let Ok(mut d) = d.try_borrow_mut() else {
                return glib::ControlFlow::Continue;
            };
            let busy = !d.closed
                && d.items
                    .iter()
                    .any(|i| !i.x.settled(now) || !i.y.settled(now) || !i.selected.settled(now));
            if !busy {
                d.ticking = false;
            }
            busy
        };
        paint.queue_draw();
        if busy {
            glib::ControlFlow::Continue
        } else {
            glib::ControlFlow::Break
        }
    });
}
fn save(d: &Rc<RefCell<Desktop>>) {
    let state = d.borrow();
    if !state.writable || state.closed {
        return;
    }
    let saved = state.saved.clone();
    drop(state);
    TO_WRITE.with(|p| *p.borrow_mut() = Some(saved));
    if WRITING.replace(true) {
        return;
    }
    glib::spawn_future_local(async {
        while let Some(positions) = TO_WRITE.with(|p| p.borrow_mut().take()) {
            match gio::spawn_blocking(move || write_positions(&state_path(), &positions)).await {
                Ok(Ok(())) => {}
                other => eprintln!("digit: desktop icon positions could not be saved: {other:?}"),
            }
        }
        WRITING.set(false);
    });
}
fn watch(d: &Rc<RefCell<Desktop>>) {
    let folder = {
        let state = d.borrow();
        if state.closed || state.monitor_file.is_some() {
            return;
        }
        state.folder.clone()
    };
    let Ok(m) = gio::File::for_path(&folder)
        .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
    else {
        return;
    };
    let weak = Rc::downgrade(d);
    m.connect_changed(move |monitor, file, _, event| {
        let Some(d) = weak.upgrade() else { return };
        let path = file.path();
        let (cancel, schedule, previous) = {
            let Ok(mut state) = d.try_borrow_mut() else {
                return;
            };
            if state.closed || state.monitor_file.as_ref() != Some(monitor) {
                return;
            }
            let cancel = path.as_ref() == Some(&state.folder)
                && matches!(
                    event,
                    gio::FileMonitorEvent::Deleted
                        | gio::FileMonitorEvent::MovedOut
                        | gio::FileMonitorEvent::Unmounted
                );
            let previous = if cancel {
                state.monitor_file.take()
            } else {
                None
            };
            (cancel, state.pending.is_none(), previous)
        };
        if cancel {
            monitor.cancel();
        }
        drop(previous);
        if !schedule {
            return;
        }
        let weak = Rc::downgrade(&d);
        let id = glib::timeout_add_local_once(Duration::from_millis(100), move || {
            if let Some(d) = weak.upgrade() {
                d.borrow_mut().pending.take();
                refresh(&d);
            }
        });
        d.borrow_mut().pending = Some(id);
    });
    let cancel = {
        let mut state = d.borrow_mut();
        if state.closed || state.monitor_file.is_some() {
            true
        } else {
            state.monitor_file = Some(m.clone());
            false
        }
    };
    if cancel {
        m.cancel();
    }
}
/// The accent changed: folders are drawn in it.
pub fn restyle() {
    if let Some(d) = DESKTOP.with(|d| d.borrow().clone()) {
        refresh(&d);
    }
}

fn refresh(d: &Rc<RefCell<Desktop>>) {
    let Ok(mut state) = d.try_borrow_mut() else {
        let weak = Rc::downgrade(d);
        glib::idle_add_local_once(move || {
            if let Some(d) = weak.upgrade() {
                refresh(&d);
            }
        });
        return;
    };
    if state.closed {
        return;
    }
    if state.loading || state.drag.is_some() || state.rubber.is_some() || state.importing {
        state.dirty = true;
        return;
    }
    state.loading = true;
    let folder = state.folder.clone();
    let weak = Rc::downgrade(d);
    drop(state);
    glib::spawn_future_local(async move {
        let result = gio::spawn_blocking(move || scan(&folder)).await;
        let Some(d) = weak.upgrade() else { return };
        if d.borrow().closed {
            return;
        }
        // Start watching before materializing icons; a second scan catches changes during the first.
        let first = d.borrow().monitor_file.is_none();
        watch(&d);
        let watched = first && d.borrow().monitor_file.is_some();
        match result {
            Ok(Ok(files)) => {
                let Some(app) = d.borrow().app.upgrade() else {
                    return;
                };
                let paint = d.borrow().paint.clone();
                let scale = paint.scale_factor().max(1);
                let theme = gtk::IconTheme::for_display(&paint.display());
                let accent = kit::style::ink().accent;
                let mut folders: HashMap<&'static str, gdk::Texture> = HashMap::new();
                let mut items = Vec::with_capacity(files.len());
                for (n, data) in files.into_iter().enumerate() {
                    let icon = data
                        .icon
                        .as_deref()
                        .and_then(|i| gio::Icon::for_string(i).ok())
                        .unwrap_or_else(|| {
                            gio::ThemedIcon::new(if data.directory {
                                "folder"
                            } else {
                                "text-x-generic"
                            })
                            .upcast()
                        });
                    // Folders are drawn like the ones in Files, in the accent color.
                    let drawn = data
                        .directory
                        .then(|| kit::folder::glyph(&gio::File::for_path(&data.path)))
                        .and_then(|glyph| {
                            if !folders.contains_key(glyph) {
                                folders.insert(
                                    glyph,
                                    kit::folder::texture(&accent, glyph, ICON * scale)?,
                                );
                            }
                            folders.get(glyph).cloned()
                        });
                    let icon: gdk::Paintable = match drawn {
                        Some(texture) => texture.upcast(),
                        None => theme
                            .lookup_by_gicon(
                                &icon,
                                ICON,
                                scale,
                                gtk::TextDirection::Ltr,
                                gtk::IconLookupFlags::empty(),
                            )
                            .upcast(),
                    };
                    let label = draw::layout(&paint, &app.cfg, &data.label, 13.0, Face::Text(520));
                    label.set_width((CELL_W - 2 * LABEL_INSET) * pango::SCALE);
                    label.set_height(-2);
                    label.set_wrap(pango::WrapMode::WordChar);
                    label.set_ellipsize(pango::EllipsizeMode::End);
                    label.set_alignment(pango::Alignment::Center);
                    items.push(Item {
                        data,
                        icon,
                        label,
                        x: Tween::still(0.0),
                        y: Tween::still(0.0),
                        selected: Tween::still(0.0),
                    });
                    if n % 24 == 23 {
                        glib::timeout_future(Duration::from_millis(1)).await;
                    }
                    if d.borrow().closed {
                        return;
                    }
                }
                let previous = {
                    let mut state = d.borrow_mut();
                    // An interaction begun while metadata was loading must not be overwritten.
                    if state.drag.is_none() && state.rubber.is_none() && !state.importing {
                        let order: BTreeMap<_, _> = state
                            .items
                            .iter()
                            .enumerate()
                            .map(|(n, i)| (i.data.name.clone(), n))
                            .collect();
                        items.sort_by_key(|i| {
                            order.get(&i.data.name).copied().unwrap_or(usize::MAX)
                        });
                        // Metadata refresh must not restart selection from zero
                        // just after replacing the drag preview with real items.
                        let selection: BTreeMap<_, _> = state
                            .items
                            .iter()
                            .map(|i| (i.data.name.clone(), i.selected))
                            .collect();
                        for item in &mut items {
                            if let Some(selected) = selection.get(&item.data.name) {
                                item.selected = *selected;
                            }
                        }
                        // Only confirmed absence releases outgoing MOVE suppression.
                        let present: BTreeSet<_> =
                            items.iter().map(|i| i.data.name.clone()).collect();
                        state.moved_out.retain(|name| present.contains(name));
                        state
                            .pending_imports
                            .retain(|name, _| !present.contains(name));
                        let previous = std::mem::replace(&mut state.items, items);
                        let names: BTreeSet<_> =
                            state.items.iter().map(|i| i.data.name.clone()).collect();
                        let selected = state.selected.intersection(&names).cloned().collect();
                        state.select(selected);
                        state.layout(false);
                        previous
                    } else {
                        state.dirty = true;
                        items
                    }
                };
                drop(previous);
                save(&d);
                animate(&d);
                let rename_ready = {
                    let mut state = d.borrow_mut();
                    let pending = state.rename_pending.clone();
                    if let Some(name) = pending.filter(|name| {
                        state.drag.is_none()
                            && state.rubber.is_none()
                            && state.items.iter().any(|i| &i.data.name == name)
                    }) {
                        state.rename_pending = None;
                        if let Some(pos) = state.positions.get(&name) {
                            state.page = pos.page;
                        }
                        state.select([name].into_iter().collect());
                        true
                    } else {
                        false
                    }
                };
                if rename_ready {
                    rename(&d);
                }
            }
            other => eprintln!(
                "digit: desktop folder could not be read: {}",
                match other {
                    Ok(Err(e)) => e,
                    _ => "worker failed".into(),
                }
            ),
        }
        let again = {
            let mut d = d.borrow_mut();
            d.loading = false;
            let dirty = std::mem::take(&mut d.dirty);
            watched || dirty
        };
        if again {
            refresh(&d);
        }
    });
}

fn open(data: &FileData) {
    if data.directory {
        show_files(&data.path);
    } else if data.launcher {
        let path = data.path.clone();
        glib::spawn_future_local(async move {
            // Desktop-entry parsing can touch disk; launch is done through GIO, never a shell.
            if let Some(info) = gio_unix::DesktopAppInfo::from_filename(path)
                && let Err(e) = info.launch(&[], gio::AppLaunchContext::NONE)
            {
                eprintln!("digit: desktop launcher: {e}");
            }
        });
    } else {
        let uri = gio::File::for_path(&data.path).uri();
        gio::AppInfo::launch_default_for_uri_async(
            &uri,
            gio::AppLaunchContext::NONE,
            gio::Cancellable::NONE,
            |r| {
                if let Err(e) = r {
                    eprintln!("digit: desktop open: {e}");
                }
            },
        );
    }
}
fn show_files(path: &Path) {
    let argv: [&std::ffi::OsStr; 2] = ["digit-files".as_ref(), path.as_os_str()];
    if let Err(e) = gio::Subprocess::newv(&argv, gio::SubprocessFlags::NONE) {
        eprintln!("digit: open Files: {e}");
    }
}
fn reveal_file(path: &Path) {
    let argv: [&std::ffi::OsStr; 3] = [
        "digit-files".as_ref(),
        "--select".as_ref(),
        path.as_os_str(),
    ];
    if let Err(e) = gio::Subprocess::newv(&argv, gio::SubprocessFlags::NONE) {
        eprintln!("digit: show in Files: {e}");
    }
}
fn trash(paths: Vec<PathBuf>) {
    glib::spawn_future_local(async move {
        for path in paths {
            if let Err(e) = gio::File::for_path(path)
                .trash_future(glib::Priority::DEFAULT)
                .await
            {
                eprintln!("digit: desktop trash: {e}");
            }
        }
    });
}
/// True means a filesystem change is needed; empty and unchanged names just dismiss.
fn rename_name_changed(name: &str, original: &str) -> Result<bool, &'static str> {
    if name.is_empty() || name == original {
        return Ok(false);
    }
    if name == "." || name == ".." {
        return Err("Choose a name other than . or ..");
    }
    if name.contains('/') || name.contains('\0') {
        return Err("Names cannot contain / or a null character");
    }
    Ok(true)
}

/// GTK editable offsets count characters, not UTF-8 bytes. Only the last suffix is excluded.
fn rename_selection(name: &str, directory: bool) -> (i32, i32) {
    let stem = if directory {
        name
    } else {
        name.rsplit_once('.')
            .filter(|(stem, _)| !stem.is_empty())
            .map_or(name, |(stem, _)| stem)
    };
    (0, stem.chars().count() as i32)
}

/// Take ownership before destroying: destruction can synchronously re-enter this helper.
/// An expected window prevents a late callback from closing a newer editor.
fn close_rename(d: &Rc<RefCell<Desktop>>, expected: Option<&gtk::Window>) {
    let window = {
        let mut state = d.borrow_mut();
        if expected.is_some_and(|w| state.rename_window.as_ref() != Some(w)) {
            return;
        }
        state.renaming = None;
        (state.rename_window.take(), state.paint.clone())
    };
    let (window, paint) = window;
    if let Some(window) = window {
        window.destroy();
    }
    paint.queue_draw();
}

fn rename_error(d: &Rc<RefCell<Desktop>>, entry: &gtk::Entry, error: &gtk::Label, text: &str) {
    entry.add_css_class("error");
    error.set_text(text);
    error.set_visible(true);
    let app = { d.borrow().app.upgrade() };
    if let Some(app) = app {
        app.request(crate::app::Peek::Note {
            text: format!("Couldn't rename: {text}"),
        });
    }
    entry.grab_focus();
}

fn rename(d: &Rc<RefCell<Desktop>>) {
    close_rename(d, None);
    let target = {
        let state = d.borrow();
        if state.closed {
            return;
        }
        (state.selected.len() == 1)
            .then(|| {
                state
                    .items
                    .iter()
                    .find(|i| state.selected.contains(&i.data.name))
                    .map(|item| {
                        (
                            item.data.path.clone(),
                            item.data.name.clone(),
                            item.data.directory,
                            item.rect(glib::monotonic_time()),
                            state.monitor.clone(),
                            state.positions.get(&item.data.name).copied(),
                        )
                    })
            })
            .flatten()
    };
    let Some((path, old, directory, r, monitor, position)) = target else {
        return;
    };
    let old_name = old.clone();
    // Like popmenu's scrim, cover the whole output without a popup or an xdg_popup grab.
    let window = gtk::Window::new();
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace(Some("digit-desktop-rename"));
    window.set_monitor(Some(&monitor));
    window.set_keyboard_mode(KeyboardMode::Exclusive);
    window.set_exclusive_zone(-1);
    for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(edge, true);
    }
    window.add_css_class("digit");
    let fixed = gtk::Fixed::new();
    fixed.set_hexpand(true);
    fixed.set_vexpand(true);
    window.set_child(Some(&fixed));
    let geometry = monitor.geometry();
    let entry = gtk::Entry::new();
    entry.add_css_class("desk-rename");
    entry.set_has_frame(false);
    gtk::prelude::EntryExt::set_alignment(&entry, 0.5);
    entry.set_width_chars(1);
    entry.set_max_width_chars(1);
    let filename = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    entry.set_text(&filename);
    let selection = rename_selection(&filename, directory);
    let error = gtk::Label::new(None);
    error.add_css_class("desk-rename-error");
    error.set_wrap(true);
    error.set_max_width_chars(22);
    error.set_visible(false);
    let editor = gtk::Box::new(gtk::Orientation::Vertical, 4);
    editor.append(&entry);
    editor.append(&error);
    // Sits exactly on the label and grows from its center with the name, like the label would.
    let y = (r.y + (ICON + 5) as f64).clamp(0.0, (geometry.height() - 24).max(0) as f64).round();
    let screen = geometry.width();
    let place = {
        let (fixed, editor) = (fixed.downgrade(), editor.downgrade());
        move |entry: &gtk::Entry| {
            let (Some(fixed), Some(editor)) = (fixed.upgrade(), editor.upgrade()) else {
                return;
            };
            let text = entry.text();
            let width = (entry.create_pango_layout(Some(&text)).pixel_size().0 + 20).clamp(44, 260).min(screen.max(1));
            entry.set_size_request(width, -1);
            let x = (r.x + CELL_W as f64 / 2.0 - width as f64 / 2.0).clamp(0.0, (screen - width).max(0) as f64).round();
            fixed.move_(&editor, x, y);
        }
    };
    fixed.put(&editor, r.x.round(), y);
    place(&entry);
    entry.connect_changed(place);

    // Weak widget references avoid cycles through the entry and its controllers.
    let weak = Rc::downgrade(d);
    let w = window.downgrade();
    let e = entry.downgrade();
    let err = error.downgrade();
    let busy = Rc::new(Cell::new(false));
    let commit: Rc<dyn Fn()> = Rc::new(move || {
        let (Some(d), Some(window), Some(entry), Some(error)) =
            (weak.upgrade(), w.upgrade(), e.upgrade(), err.upgrade())
        else {
            return;
        };
        let active = {
            let state = d.borrow();
            !state.closed && state.rename_window.as_ref() == Some(&window)
        };
        if !active || busy.get() {
            return;
        }
        let name = entry.text().to_string();
        match rename_name_changed(&name, &filename) {
            Ok(false) => {
                close_rename(&d, Some(&window));
                return;
            }
            Err(text) => {
                rename_error(&d, &entry, &error, text);
                return;
            }
            Ok(true) => {}
        }
        busy.set(true);
        entry.set_editable(false);
        let source_path = path.clone();
        let destination_path = path.with_file_name(&name);
        let weak = weak.clone();
        let w = w.clone();
        let e = e.clone();
        let err = err.clone();
        let busy = busy.clone();
        let old = old.clone();
        glib::spawn_future_local(async move {
            // NONE refuses existing targets, including dangling symlinks, without a racy precheck.
            let target_path = destination_path.clone();
            let result = gio::spawn_blocking(move || {
                gio::File::for_path(source_path)
                    .move_(
                        &gio::File::for_path(target_path),
                        gio::FileCopyFlags::NONE,
                        gio::Cancellable::NONE,
                        None,
                    )
                    .map_err(|e| e.to_string())
            })
            .await
            .unwrap_or_else(|_| Err("Rename worker failed".into()));
            let Some(d) = weak.upgrade() else { return };
            match result {
                Ok(()) => {
                    // Let any pre-rename scan finish before restoring selection and placement.
                    // Otherwise its stale snapshot can discard the new persistence key.
                    while {
                        let state = d.borrow();
                        state.loading && !state.closed
                    } {
                        glib::timeout_future(Duration::from_millis(16)).await;
                    }
                    let new = name_key(std::ffi::OsStr::new(&name));
                    {
                        let mut state = d.borrow_mut();
                        state.saved.remove(&old);
                        if let Some(pos) = position {
                            state.saved.insert(new.clone(), pos);
                            state.positions.remove(&old);
                            state.positions.insert(new.clone(), pos);
                        }
                        // Keep selection valid until the refreshed metadata arrives.
                        if let Some(item) = state.items.iter_mut().find(|i| i.data.name == old) {
                            item.data.name = new.clone();
                            item.data.path = destination_path;
                        }
                        state.select([new].into_iter().collect());
                    }
                    if let Some(window) = w.upgrade() {
                        close_rename(&d, Some(&window));
                    }
                    save(&d);
                    refresh(&d);
                }
                Err(reason) => {
                    busy.set(false);
                    let (Some(window), Some(entry), Some(error)) =
                        (w.upgrade(), e.upgrade(), err.upgrade())
                    else {
                        return;
                    };
                    let active = { d.borrow().rename_window.as_ref() == Some(&window) };
                    if active {
                        entry.set_editable(true);
                        rename_error(&d, &entry, &error, &reason.to_string());
                    }
                }
            }
        });
    });
    let activate = commit.clone();
    entry.connect_activate(move |_| activate());
    let click = gtk::GestureClick::new();
    click.set_button(0);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let e = entry.downgrade();
    click.connect_pressed(move |gesture, _, x, y| {
        let Some(entry) = e.upgrade() else { return };
        let Some(widget) = gesture.widget() else {
            return;
        };
        let inside = entry
            .compute_bounds(&widget)
            .is_some_and(|bounds| bounds.contains_point(&graphene::Point::new(x as f32, y as f32)));
        if !inside {
            commit();
        }
    });
    fixed.add_controller(click);
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(d);
    let w = window.downgrade();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gdk::Key::Escape {
            if let (Some(d), Some(window)) = (weak.upgrade(), w.upgrade()) {
                close_rename(&d, Some(&window));
            }
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    window.add_controller(keys);
    let weak = Rc::downgrade(d);
    window.connect_destroy(move |window| {
        if let Some(d) = weak.upgrade() {
            close_rename(&d, Some(window));
        }
    });
    let paint = {
        let mut state = d.borrow_mut();
        state.rename_window = Some(window.clone());
        state.renaming = Some(old_name);
        state.paint.clone()
    };
    paint.queue_draw();
    let mapped = Cell::new(false);
    let e = entry.downgrade();
    window.connect_map(move |window| {
        if mapped.replace(true) {
            return;
        }
        let e = e.clone();
        let w = window.downgrade();
        glib::idle_add_local_once(move || {
            if let (Some(window), Some(entry)) = (w.upgrade(), e.upgrade())
                && window.is_mapped()
            {
                entry.grab_focus();
                entry.select_region(selection.0, selection.1);
            }
        });
    });
    window.present();
}

#[cfg(test)]
mod rename_tests {
    use super::{rename_name_changed, rename_selection};

    #[test]
    fn validates_names_and_treats_empty_or_unchanged_as_dismissal() {
        for name in ["", "original"] {
            assert_eq!(rename_name_changed(name, "original"), Ok(false));
        }
        for name in [".", "..", "a/b", "a\0b"] {
            assert!(rename_name_changed(name, "original").is_err());
        }
        for name in ["new", ".hidden", "two words", " padded ", "a.tar.gz"] {
            assert_eq!(rename_name_changed(name, "original"), Ok(true));
        }
    }

    #[test]
    fn selects_file_stems_and_complete_directory_names_in_character_offsets() {
        assert_eq!(rename_selection("a.tar.gz", false), (0, 5));
        assert_eq!(rename_selection(".hidden", false), (0, 7));
        assert_eq!(rename_selection("folder.d", true), (0, 8));
        assert_eq!(rename_selection("filename", false), (0, 8));
        assert_eq!(rename_selection("é日.txt", false), (0, 2));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MenuAction {
    NewFolder,
    Wallpaper,
    Terminal,
    Sort,
    OpenFolder,
    Separator,
    DesktopSettings,
    DisplaySettings,
    Open,
    Reveal,
    Rename,
    CopyPath,
    Trash,
}
impl MenuAction {
    fn label(self) -> &'static str {
        match self {
            Self::NewFolder => "New Folder",
            Self::Wallpaper => "Change Wallpaper",
            Self::Terminal => "Open Terminal Here",
            Self::Sort => "Sort by Name",
            Self::OpenFolder => "Open Desktop Folder",
            Self::Separator => "",
            Self::DesktopSettings => "Desktop Settings",
            Self::DisplaySettings => "Display Settings",
            Self::Open => "Open",
            Self::Reveal => "Show in Files",
            Self::Rename => "Rename",
            Self::CopyPath => "Copy Path",
            Self::Trash => "Move to Trash",
        }
    }
}
fn menu_entries(on_icon: bool, selected: usize) -> Vec<MenuAction> {
    use MenuAction::*;
    if !on_icon {
        vec![
            NewFolder,
            Wallpaper,
            Terminal,
            Sort,
            OpenFolder,
            Separator,
            DesktopSettings,
            DisplaySettings,
        ]
    } else if selected > 1 {
        vec![Open, Trash]
    } else {
        vec![Open, Reveal, Rename, CopyPath, Trash]
    }
}
fn menu(d: &Rc<RefCell<Desktop>>, on_icon: bool, x: f64, y: f64) {
    let (app, monitor, folder, entries, files, clipboard) = {
        let Ok(state) = d.try_borrow() else { return };
        let Some(app) = state.app.upgrade() else {
            return;
        };
        (
            app,
            state.monitor.clone(),
            state.folder.clone(),
            menu_entries(on_icon, state.selected.len()),
            state
                .items
                .iter()
                .filter(|i| state.selected.contains(&i.data.name))
                .map(|i| i.data.clone())
                .collect::<Vec<_>>(),
            state.paint.clone(),
        )
    };
    let rows = entries
        .into_iter()
        .map(|action| {
            if action == MenuAction::Separator {
                return Row::Sep;
            }
            let weak = Rc::downgrade(d);
            let app = Rc::downgrade(&app);
            let folder = folder.clone();
            let files = files.clone();
            let paint = clipboard.clone();
            let row = Row::item(action.label(), move || match action {
                MenuAction::NewFolder => {
                    let folder = folder.clone();
                    let weak = weak.clone();
                    glib::spawn_future_local(async move {
                        match gio::spawn_blocking(move || new_folder(&folder)).await {
                            Ok(Ok(path)) => {
                                if let Some(d) = weak.upgrade() {
                                    {
                                        let mut state = d.borrow_mut();
                                        if state.closed {
                                            return;
                                        }
                                        state.rename_pending = path.file_name().map(name_key);
                                    }
                                    refresh(&d);
                                }
                            }
                            other => eprintln!("digit: desktop new folder: {other:?}"),
                        }
                    });
                }
                MenuAction::Wallpaper => {
                    if let Some(app) = app.upgrade() {
                        crate::picker::toggle(&app);
                    }
                }
                MenuAction::Terminal => {
                    let command = kit::terminal::command();
                    let argv: Vec<_> = command
                        .split_whitespace()
                        .map(std::ffi::OsStr::new)
                        .collect();
                    let launcher = gio::SubprocessLauncher::new(gio::SubprocessFlags::NONE);
                    launcher.set_cwd(&folder);
                    if !argv.is_empty()
                        && let Err(e) = launcher.spawn(&argv)
                    {
                        eprintln!("digit: desktop terminal: {e}");
                    }
                }
                MenuAction::Sort => {
                    if let Some(d) = weak.upgrade() {
                        {
                            let Ok(mut state) = d.try_borrow_mut() else {
                                return;
                            };
                            state.sort_by_name();
                        }
                        save(&d);
                        animate(&d);
                    }
                }
                MenuAction::OpenFolder => show_files(&folder),
                MenuAction::DesktopSettings | MenuAction::DisplaySettings => {
                    let page = if action == MenuAction::DesktopSettings {
                        "desktop"
                    } else {
                        "displays"
                    };
                    if let Err(e) = gio::Subprocess::newv(
                        &[
                            std::ffi::OsStr::new("digit-settings"),
                            std::ffi::OsStr::new(page),
                        ],
                        gio::SubprocessFlags::NONE,
                    ) {
                        eprintln!("digit: desktop settings: {e}");
                    }
                }
                MenuAction::Open => files.iter().for_each(open),
                MenuAction::Reveal => {
                    if let Some(file) = files.first() {
                        reveal_file(&file.path);
                    }
                }
                MenuAction::Rename => {
                    if let Some(d) = weak.upgrade() {
                        rename(&d);
                    }
                }
                MenuAction::CopyPath => {
                    if let Some(file) = files.first() {
                        paint.clipboard().set_text(&file.path.to_string_lossy());
                    }
                }
                MenuAction::Trash => trash(files.iter().map(|f| f.path.clone()).collect()),
                MenuAction::Separator => {}
            });
            if action == MenuAction::Trash {
                row.danger()
            } else {
                row
            }
        })
        .collect();
    // The shared menu uses an Overlay surface with Exclusive keyboard mode and
    // a Top-layer outside-click scrim: no popup grab from the Bottom desktop.
    popmenu::show_desktop(&app, Some(monitor), x, y, rows);
}
fn folder_names() -> impl Iterator<Item = String> {
    (1..10000).map(|n| {
        if n == 1 {
            "New Folder".into()
        } else {
            format!("New Folder {n}")
        }
    })
}
fn next_folder_name(mut exists: impl FnMut(&str) -> bool) -> Option<String> {
    folder_names().find(|name| !exists(name))
}
fn new_folder(folder: &Path) -> std::io::Result<PathBuf> {
    loop {
        let name = next_folder_name(|name| folder.join(name).symlink_metadata().is_ok())
            .ok_or_else(|| std::io::Error::other("No free folder name"))?;
        let path = folder.join(name);
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            // Another creator can win between the name search and mkdir.
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
}

fn controllers(d: &Rc<RefCell<Desktop>>, paint: &Paint) {
    let click = gtk::GestureClick::new();
    click.set_button(0);
    // Capture selection and the grab point before DragSource sees the press.
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(d);
    click.connect_pressed(move |g, n, x, y| {
        let Some(d) = weak.upgrade() else { return };
        let button = g.current_button();
        if button != 1 && button != 3 {
            return;
        }
        cancel_pending_drag(&d);
        let ctrl = g
            .current_event_state()
            .contains(gdk::ModifierType::CONTROL_MASK);
        let (on_icon, open_file) = {
            let Ok(mut state) = d.try_borrow_mut() else {
                return;
            };
            let at = state.hit(x, y);
            let mut selected = state.selected.clone();
            match at {
                Some(i) => {
                    let name = state.items[i].data.name.clone();
                    if ctrl && button == 1 {
                        if !selected.remove(&name) {
                            selected.insert(name);
                        }
                    } else if !selected.contains(&name) || n == 2 {
                        selected.clear();
                        selected.insert(name);
                    }
                }
                None if !ctrl || button == 3 => selected.clear(),
                None => {}
            }
            state.select(selected);
            let open_file = (button == 1 && n == 2)
                .then(|| at.map(|i| state.items[i].data.clone()))
                .flatten();
            if let Some(at) = at {
                state.raise(at);
            }
            (at.is_some(), open_file)
        };
        animate(&d);
        if button == 3 {
            g.set_state(gtk::EventSequenceState::Claimed);
            menu(&d, on_icon, x, y);
        } else if let Some(file) = open_file {
            open(&file);
        } else if button == 1 && n == 1 && on_icon {
            arm_drag(&d, x, y);
        }
    });
    paint.add_controller(click);

    // Motion has no threshold and does not claim a gesture. Once Wayland takes
    // the pointer grab, use drop-target motion for the local Hyprland visual.
    let motion = gtk::EventControllerMotion::new();
    motion.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(d);
    motion.connect_motion(move |controller, x, y| {
        let Some(d) = weak.upgrade() else { return };
        if !controller
            .current_event_state()
            .contains(gdk::ModifierType::BUTTON1_MASK)
        {
            cancel_pending_drag(&d);
            return;
        }
        let pending = d
            .borrow()
            .drag
            .as_ref()
            .is_some_and(|drag| drag.system.is_none());
        if pending {
            move_drag(&d, x, y);
        }
    });
    paint.add_controller(motion);

    // GestureClick stops recognizing a click at the threshold, even if the
    // button is released before DragSource actually starts. Observe that release
    // independently so a quick small movement cannot leave icons displaced.
    let release = gtk::EventControllerLegacy::new();
    release.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(d);
    release.connect_event(move |_, event| {
        if event.event_type() == gdk::EventType::ButtonRelease
            && let Some(d) = weak.upgrade()
        {
            cancel_pending_drag(&d);
        }
        glib::Propagation::Proceed
    });
    paint.add_controller(release);

    let band = gtk::GestureDrag::new();
    let weak = Rc::downgrade(d);
    band.connect_drag_begin(move |g, x, y| {
        let Some(d) = weak.upgrade() else { return };
        let primary = g.current_button() == 1;
        let additive = g
            .current_event_state()
            .contains(gdk::ModifierType::CONTROL_MASK);
        let denied = {
            let Ok(mut state) = d.try_borrow_mut() else {
                return;
            };
            state.rubber = begin_band(
                state.hit(x, y).is_some(),
                primary,
                state.drag.is_some(),
                (x, y),
                state.selected.clone(),
                additive,
            );
            state.rubber.is_none()
        };
        if denied {
            // GTK emits drag-end from this call. The model must already be free.
            g.set_state(gtk::EventSequenceState::Denied);
        }
    });
    let weak = Rc::downgrade(d);
    band.connect_drag_update(move |_, dx, dy| {
        let Some(d) = weak.upgrade() else { return };
        let Ok(mut state) = d.try_borrow_mut() else {
            return;
        };
        let Some(r) = state.rubber.as_mut() else {
            return;
        };
        r.end = (r.start.0 + dx, r.start.1 + dy);
        let rect = Rect::between(r.start, r.end);
        let initial = r.initial.clone();
        let additive = r.additive;
        let now = glib::monotonic_time();
        let items: Vec<_> = state
            .items
            .iter()
            .filter(|i| state.visible(i))
            .map(|i| (i.data.name.clone(), i.rect(now)))
            .collect();
        let selected = rubber_select(rect, &items, &initial, additive);
        state.select(selected);
        drop(state);
        animate(&d);
    });
    let weak = Rc::downgrade(d);
    band.connect_drag_end(move |_, _, _| {
        if let Some(d) = weak.upgrade() {
            end_band(&d, false);
        }
    });
    let weak = Rc::downgrade(d);
    band.connect_cancel(move |_, _| {
        if let Some(d) = weak.upgrade() {
            end_band(&d, true);
        }
    });
    paint.add_controller(band);

    let source = gtk::DragSource::new();
    // MOVE must remain offered: Hyprland may select it even for a local drag.
    source.set_actions(gdk::DragAction::COPY | gdk::DragAction::MOVE);
    let weak = Rc::downgrade(d);
    source.connect_prepare(move |_, _, _| {
        let d = weak.upgrade()?;
        let content = {
            let mut state = d.try_borrow_mut().ok()?;
            let drag = state.drag.as_ref()?;
            let name = drag.anchor.clone();
            let content = drag.content.clone();
            if !state.selected.contains(&name) {
                state.select([name.clone()].into_iter().collect());
            }
            let selected = state.selected.clone();
            state.items.sort_by_key(|i| selected.contains(&i.data.name));
            let at = state.items.iter().position(|i| i.data.name == name)?;
            state.raise(at);
            content
        };
        // Capture the current selection tween and pixel phase. Icon textures and
        // Pango layouts were warmed on press; no file IO or texture render here.
        let (preview, hotspot) = make_preview(&d, true)?;
        let previous = {
            let mut state = d.borrow_mut();
            let drag = state.drag.as_mut()?;
            drag.hotspot = hotspot;
            std::mem::replace(&mut drag.preview, preview.clone())
        };
        drop(previous);
        Some(content)
    });
    let weak = Rc::downgrade(d);
    source.connect_drag_begin(move |_, system| {
        let Some(d) = weak.upgrade() else { return };
        let (preview, hotspot) = {
            let mut state = d.borrow_mut();
            let Some(drag) = state.drag.as_mut() else {
                return;
            };
            drag.system = Some(system.clone());
            (drag.preview.clone(), drag.hotspot)
        };
        // The shared helper also repairs Hyprland's offset after GPU attaches;
        // merely setting GDK's hotspot before the frame cannot survive those.
        let mut picture = Some(kit::drag::set_icon(system, &preview, hotspot));
        let previous = {
            let mut state = d.borrow_mut();
            state.drag.as_mut().and_then(|drag| std::mem::replace(&mut drag.visual, picture.take()))
        };
        drop(previous);
        drop(picture);
        let weak = Rc::downgrade(&d);
        system.connect_drop_performed(move |system| {
            if let Some(d) = weak.upgrade() {
                drag_event(&d, system, DragEvent::Performed);
            }
        });
        let weak = Rc::downgrade(&d);
        system.connect_dnd_finished(move |system| {
            let moved = system.selected_action() == gdk::DragAction::MOVE;
            if let Some(d) = weak.upgrade() {
                drag_event(&d, system, DragEvent::Finished { moved });
            }
        });
        let weak = Rc::downgrade(&d);
        system.connect_cancel(move |system, _| {
            if let Some(d) = weak.upgrade() {
                drag_event(&d, system, DragEvent::Cancel);
            }
        });
        animate(&d);
    });
    let weak = Rc::downgrade(d);
    source.connect_drag_end(move |_, system, _| {
        // URI targets own transfers. Never delete on delete_data (also true for
        // local repositioning and for a handled cancellation). GDK decides end.
        if let Some(d) = weak.upgrade() {
            drag_event(&d, system, DragEvent::SourceEnd);
        }
    });
    let weak = Rc::downgrade(d);
    source.connect_drag_cancel(move |_, system, _| {
        if let Some(d) = weak.upgrade() {
            // Also covers GtkDragSource::drag_cancel(), which bypasses GDK cancel.
            drag_event(&d, system, DragEvent::Cancel);
        }
        true // Direct restoration; suppress GTK's separate failure animation.
    });
    paint.add_controller(source);

    let target = gtk::DropTarget::new(
        gdk::FileList::static_type(),
        gdk::DragAction::COPY | gdk::DragAction::MOVE,
    );
    let weak = Rc::downgrade(d);
    target.connect_enter(move |target, x, y| {
        if let Some(d) = weak.upgrade()
            && own_drop(&d, target)
        {
            desktop_drag_target(&d, true);
            move_drag(&d, x, y);
            return gdk::DragAction::MOVE;
        }
        gdk::DragAction::COPY
    });
    let weak = Rc::downgrade(d);
    target.connect_motion(move |target, x, y| {
        let Some(d) = weak.upgrade() else {
            return gdk::DragAction::empty();
        };
        if own_drop(&d, target) {
            desktop_drag_target(&d, true);
            move_drag(&d, x, y);
            gdk::DragAction::MOVE
        } else {
            // We own the transfer; reporting COPY prevents source-side deletion on failure
            // and preserves the original for cross-filesystem copies.
            gdk::DragAction::COPY
        }
    });
    let weak = Rc::downgrade(d);
    target.connect_leave(move |_| {
        if let Some(d) = weak.upgrade() {
            desktop_drag_target(&d, false);
        }
    });
    let weak = Rc::downgrade(d);
    target.connect_drop(move |target, value, x, y| {
        let Some(d) = weak.upgrade() else {
            return false;
        };
        let Ok(list) = value.get::<gdk::FileList>() else {
            return false;
        };
        if own_drop(&d, target) {
            finish_drag(&d, x, y);
            return true;
        }
        let uris: Vec<_> = list.files().iter().map(|f| f.uri().to_string()).collect();
        if uris.is_empty() {
            return false;
        }
        import_drop(&d, uris, (x, y));
        true
    });
    paint.add_controller(target);

    let scroll = gtk::EventControllerScroll::new(
        gtk::EventControllerScrollFlags::VERTICAL | gtk::EventControllerScrollFlags::DISCRETE,
    );
    let weak = Rc::downgrade(d);
    let last = Cell::new(0_i64);
    scroll.connect_scroll(move |_, _, dy| {
        let Some(d) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        let Ok(mut state) = d.try_borrow_mut() else {
            return glib::Propagation::Proceed;
        };
        let max = state.positions.values().map(|p| p.page).max().unwrap_or(0);
        let now = glib::monotonic_time();
        if max == 0 || state.drag.is_some() || state.rubber.is_some() || now - last.get() < 180_000
        {
            return glib::Propagation::Proceed;
        }
        if dy > 0.0 {
            state.page = (state.page + 1).min(max);
        } else if dy < 0.0 {
            state.page = state.page.saturating_sub(1);
        }
        last.set(now);
        state.select(BTreeSet::new());
        drop(state);
        animate(&d);
        glib::Propagation::Stop
    });
    paint.add_controller(scroll);
}
fn make_preview(
    d: &Rc<RefCell<Desktop>>,
    freeze_selection: bool,
) -> Option<(gdk::Paintable, (i32, i32))> {
    let paint = { d.borrow().paint.clone() };
    let now = paint
        .frame_clock()
        .map(|c| c.frame_time())
        .unwrap_or_else(glib::monotonic_time);
    let (app, frames, pointer, origins) = {
        let mut state = d.borrow_mut();
        if freeze_selection {
            let names = state.drag.as_ref()?.movement.origins.clone();
            for item in &mut state.items {
                if names.contains_key(&item.data.name) {
                    // The cached preview and the restored real item must have
                    // identical selection ink, even if the press tween is mid-flight.
                    item.selected.set(item.selected.value(now));
                }
            }
        }
        let drag = state.drag.as_ref()?;
        let frames: Vec<_> = state
            .items
            .iter()
            .filter(|i| drag.movement.origins.contains_key(&i.data.name))
            .map(|i| ItemFrame {
                icon: i.icon.clone(),
                label: i.label.clone(),
                rect: i.rect(now),
                selected: i.selected.value(now),
                editing: false,
            })
            .collect();
        let origins = drag.movement.at(drag.pointer.0, drag.pointer.1);
        (state.app.upgrade()?, frames, drag.pointer, origins)
    };
    let (bounds, hot) = preview_bounds(&origins, pointer);
    let s = gtk::Snapshot::new();
    // Compensate the fractional part GDK's integer hotspot cannot represent.
    // Drawing in desktop coordinates first also preserves its text pixel phase.
    s.translate(&graphene::Point::new(
        hot.0 as f32 - pointer.0 as f32,
        hot.1 as f32 - pointer.1 as f32,
    ));
    let pal = app.palette_at(now);
    for item in &frames {
        draw_item(&s, &app, &pal, item);
    }
    s.to_paintable(Some(&graphene::Size::new(bounds.w as f32, bounds.h as f32)))
        .map(|preview| (preview, hot))
}

fn arm_drag(d: &Rc<RefCell<Desktop>>, x: f64, y: f64) {
    let (movement, anchor, paths, fallback) = {
        let state = d.borrow();
        if state.drag.is_some() || state.closed {
            return;
        }
        let Some(at) = state.hit(x, y) else { return };
        let item = &state.items[at];
        let anchor = item.data.name.clone();
        let selected = drag_selection(&state.selected, &anchor);
        let now = glib::monotonic_time();
        let movement = Drag {
            start: (x, y),
            origins: state
                .items
                .iter()
                .filter(|i| state.visible(i) && selected.contains(&i.data.name))
                .map(|i| (i.data.name.clone(), (i.x.value(now), i.y.value(now))))
                .collect(),
        };
        let paths: Vec<_> = state
            .items
            .iter()
            .filter(|i| movement.origins.contains_key(&i.data.name))
            .map(|i| i.data.path.clone())
            .collect();
        (movement, anchor, paths, item.icon.clone())
    };
    let files: Vec<_> = paths.iter().map(gio::File::for_path).collect();
    let content = gdk::ContentProvider::for_value(&gdk::FileList::from_array(&files).to_value());
    let drag = DesktopDrag {
        movement,
        anchor,
        content,
        preview: fallback,
        visual: None,
        local_preview: kit::wm::which() == kit::wm::Wm::Hyprland,
        over_desktop: false,
        hotspot: (0, 0),
        pointer: (x, y),
        end: DragEnd::Active,
        system: None,
    };
    {
        let mut state = d.borrow_mut();
        if state.closed || state.drag.is_some() {
            return;
        }
        state.drag = Some(drag);
    }
    move_drag(d, x, y);
    // Warm the exact drawing (including theme paintables) before the threshold.
    if let Some((preview, hotspot)) = make_preview(d, false) {
        let previous = {
            let mut state = d.borrow_mut();
            let Some(drag) = state.drag.as_mut() else {
                return;
            };
            drag.hotspot = hotspot;
            std::mem::replace(&mut drag.preview, preview)
        };
        drop(previous);
    }
}

fn own_drop(d: &Rc<RefCell<Desktop>>, target: &gtk::DropTarget) -> bool {
    let system = target.current_drop().and_then(|drop| drop.drag());
    let state = d.borrow();
    system.is_some() && system == state.drag.as_ref().and_then(|drag| drag.system.clone())
}

fn drag_selection(selected: &BTreeSet<String>, anchor: &str) -> BTreeSet<String> {
    if selected.contains(anchor) {
        selected.clone()
    } else {
        [anchor.to_owned()].into_iter().collect()
    }
}

fn desktop_drag_target(d: &Rc<RefCell<Desktop>>, over: bool) {
    let (paint, visual, preview) = {
        let mut state = d.borrow_mut();
        let Some(drag) = state.drag.as_mut() else { return };
        if !drag.local_preview || drag.system.is_none() || drag.end == DragEnd::Local
            || drag.over_desktop == over {
            return;
        }
        drag.over_desktop = over;
        let visual = drag.visual.clone();
        let preview = drag.preview.clone();
        (state.paint.clone(), visual, preview)
    };
    if let Some(visual) = visual {
        if over {
            // A real texture node submits a transparent buffer, clearing the
            // previous native visual. Keep its allocation/hotspot unchanged.
            let empty = gdk::MemoryTexture::new(1, 1, gdk::MemoryFormat::R8g8b8a8,
                &glib::Bytes::from_static(&[0, 0, 0, 0]), 4);
            visual.set_paintable(Some(&empty));
        } else {
            visual.set_paintable(Some(&preview));
        }
    }
    // Leaving only hides these items; restoration is a terminal drag event.
    paint.queue_draw();
}

fn move_drag(d: &Rc<RefCell<Desktop>>, x: f64, y: f64) {
    let paint = {
        let mut state = d.borrow_mut();
        let Some(drag) = state.drag.as_mut() else {
            return;
        };
        if drag.end == DragEnd::Local {
            return;
        }
        if drag.system.is_some() && !(drag.local_preview && drag.over_desktop) {
            return;
        }
        drag.pointer = (x, y);
        let positions = drag.movement.at(x, y);
        for i in &mut state.items {
            if let Some(&(x, y)) = positions.get(&i.data.name) {
                i.x.set(x);
                i.y.set(y);
            }
        }
        state.paint.clone()
    };
    paint.queue_draw();
}

fn restore_drag_items(items: &mut [Item], restored: &BTreeMap<String, (f64, f64)>) {
    for i in items {
        if let Some(&(x, y)) = restored.get(&i.data.name) {
            i.x.set(x);
            i.y.set(y);
        }
    }
}

fn cancel_pending_drag(d: &Rc<RefCell<Desktop>>) {
    let pending = d
        .borrow()
        .drag
        .as_ref()
        .is_some_and(|drag| drag.system.is_none());
    if pending {
        cancel_drag(d);
    }
}
// Gesture arbitration may deny an empty-point drag too (for example a secondary
// click). Both denied and duplicate end/cancel signals leave no rubber band behind.
fn begin_band(
    on_icon: bool,
    primary: bool,
    dragging: bool,
    start: (f64, f64),
    initial: BTreeSet<String>,
    additive: bool,
) -> Option<Rubber> {
    (!on_icon && primary && !dragging).then_some(Rubber {
        start,
        end: start,
        initial,
        additive,
    })
}
fn finish_band(rubber: &mut Option<Rubber>, cancelled: bool) -> Option<BTreeSet<String>> {
    rubber.take().and_then(|r| cancelled.then_some(r.initial))
}
fn end_band(d: &Rc<RefCell<Desktop>>, cancelled: bool) {
    let dirty = {
        let Ok(mut state) = d.try_borrow_mut() else {
            return;
        };
        if let Some(initial) = finish_band(&mut state.rubber, cancelled) {
            state.select(initial);
        }
        state.dirty
    };
    animate(d);
    if dirty {
        refresh(d);
    }
}
fn cancel_drag(d: &Rc<RefCell<Desktop>>) {
    let Ok(mut state) = d.try_borrow_mut() else {
        return;
    };
    let drag = state.drag.take();
    if let Some(drag) = &drag {
        restore_drag_items(&mut state.items, &drag.movement.restored());
    }
    let selected = state.selected.clone();
    state.select(selected);
    let refresh_needed = state.dirty;
    drop(state);
    // Widgets/paintables may finalize with synchronous callbacks too.
    drop(drag);
    animate(d);
    if refresh_needed {
        refresh(d);
    }
}

fn drag_event(d: &Rc<RefCell<Desktop>>, system: &gdk::Drag, event: DragEvent) {
    let drag = {
        let mut state = d.borrow_mut();
        let Some(drag) = state.drag.as_mut() else {
            return;
        };
        if drag.system.as_ref() != Some(system) {
            return;
        }
        let Some(result) = drag.end.event(event) else {
            return;
        };
        let drag = state.drag.take().unwrap();
        match result {
            DragResult::Restore => restore_drag_items(&mut state.items, &drag.movement.restored()),
            DragResult::AwaitRemoval => {
                state
                    .moved_out
                    .extend(drag.movement.origins.keys().cloned());
            }
            DragResult::KeepLocal => {}
        }
        let selected = state.selected.clone();
        state.select(selected);
        drag
    };
    // GDK callbacks may be followed synchronously by GtkDragSource::drag-end.
    // The model has already transitioned once, and GTK objects finalize unborrowed.
    drop(drag);
    animate(d);
    refresh(d);
}

fn finish_drag(d: &Rc<RefCell<Desktop>>, x: f64, y: f64) {
    let Ok(mut state) = d.try_borrow_mut() else {
        return;
    };
    let Some(drag) = state.drag.as_ref() else {
        return;
    };
    if drag.end == DragEnd::Local {
        return;
    }
    let dropped = drag.movement.dropped(state.grid, x, y, state.page);
    state.drag.as_mut().unwrap().end.event(DragEvent::LocalDrop);
    for i in &mut state.items {
        if let Some(p) = dropped.get(&i.data.name) {
            i.x.set(p.x);
            i.y.set(p.y);
        }
    }
    state.saved.extend(dropped.clone());
    state.positions.extend(dropped);
    let refresh_needed = state.dirty;
    drop(state);
    save(d);
    animate(d);
    if refresh_needed {
        refresh(d);
    }
}
/// A compact cascade, centered on the drop rather than the automatic right column.
fn drop_cascade(grid: Grid, pointer: (f64, f64), page: u32, count: usize) -> Vec<FreePos> {
    (0..count)
        .map(|n| {
            grid.clamp(FreePos {
                x: pointer.0 - CELL_W as f64 / 2.0 + n as f64 * 24.0,
                y: pointer.1 - CELL_H as f64 / 2.0 + n as f64 * 24.0,
                page,
            })
        })
        .collect()
}

fn collision_name(
    name: &std::ffi::OsStr,
    directory: bool,
    mut exists: impl FnMut(&std::ffi::OsStr) -> bool,
) -> std::ffi::OsString {
    if !exists(name) {
        return name.to_owned();
    }
    let path = Path::new(name);
    let (stem, extension) = if directory {
        (name, None)
    } else {
        (path.file_stem().unwrap_or(name), path.extension())
    };
    for n in 2_u64.. {
        let mut candidate = stem.to_owned();
        candidate.push(format!(" ({n})"));
        if let Some(extension) = extension {
            candidate.push(".");
            candidate.push(extension);
        }
        if !exists(&candidate) {
            return candidate;
        }
    }
    unreachable!("all filename suffixes exhausted")
}

struct ImportPlan {
    uri: String,
    dest: PathBuf,
}

fn rollback_import(
    saved: &mut Positions,
    positions: &mut Positions,
    pending: &mut Positions,
    key: String,
    seeded: FreePos,
    previous: Option<FreePos>,
) {
    pending.remove(&key);
    // A later manual reposition takes precedence over the failed reservation.
    if saved.get(&key) == Some(&seeded) {
        saved.remove(&key);
        positions.remove(&key);
        if let Some(previous) = previous {
            saved.insert(key.clone(), previous);
            positions.insert(key, previous);
        }
    }
}

fn plan_imports(
    uris: Vec<String>,
    folder: &Path,
    mut reserved: BTreeSet<String>,
) -> Vec<ImportPlan> {
    uris.into_iter()
        .filter_map(|uri| {
            let file = gio::File::for_uri(&uri);
            let name = file.basename()?;
            let original = folder.join(&name);
            let dest = if file.equal(&gio::File::for_path(&original)) {
                original // Already on the desktop: reposition the existing item.
            } else {
                let directory = file
                    .query_file_type(gio::FileQueryInfoFlags::NONE, gio::Cancellable::NONE)
                    == gio::FileType::Directory;
                let name = collision_name(name.as_os_str(), directory, |candidate| {
                    reserved.contains(&name_key(candidate))
                        || folder.join(candidate).symlink_metadata().is_ok()
                });
                folder.join(name)
            };
            reserved.insert(name_key(dest.file_name()?));
            Some(ImportPlan { uri, dest })
        })
        .collect()
}

fn import_drop(d: &Rc<RefCell<Desktop>>, uris: Vec<String>, pointer: (f64, f64)) {
    let (grid, page) = {
        let state = d.borrow();
        (state.grid, state.page)
    };
    let weak = Rc::downgrade(d);
    glib::spawn_future_local(async move {
        let Some(d) = weak.upgrade() else { return };
        // Serialize planning too, so two concurrent drops cannot reserve the same name.
        loop {
            let ready = {
                let mut state = d.borrow_mut();
                if state.closed {
                    return;
                }
                if state.importing {
                    false
                } else {
                    state.importing = true;
                    true
                }
            };
            if ready {
                break;
            }
            glib::timeout_future(Duration::from_millis(16)).await;
        }
        let (folder, reserved) = {
            let state = d.borrow();
            (state.folder.clone(), state.saved.keys().cloned().collect())
        };
        let plans = gio::spawn_blocking(move || plan_imports(uris, &folder, reserved)).await;
        let Ok(plans) = plans else {
            d.borrow_mut().importing = false;
            refresh(&d);
            return;
        };
        let positions = drop_cascade(grid, pointer, page, plans.len());
        let previous: Vec<_> = {
            let mut state = d.borrow_mut();
            plans
                .iter()
                .zip(positions)
                .map(|(plan, pos)| {
                    let key = name_key(plan.dest.file_name().unwrap());
                    let previous = state.saved.insert(key.clone(), pos);
                    state.pending_imports.insert(key.clone(), pos);
                    state.positions.insert(key.clone(), pos);
                    for item in &mut state.items {
                        if item.data.name == key {
                            item.x.set(pos.x);
                            item.y.set(pos.y);
                        }
                    }
                    (key, pos, previous)
                })
                .collect()
        };
        // Seed both the monitor's in-memory layout and persistence before transfer.
        // Pending reservations survive layout; refresh is deferred while importing.
        save(&d);
        animate(&d);
        while WRITING.get() {
            glib::timeout_future(Duration::from_millis(1)).await;
        }
        let results = gio::spawn_blocking(move || {
            plans
                .into_iter()
                .map(|plan| import(&gio::File::for_uri(&plan.uri), &plan.dest))
                .collect::<Vec<_>>()
        })
        .await;
        {
            let mut state = d.borrow_mut();
            for (n, (key, pos, old)) in previous.into_iter().enumerate() {
                let error = match &results {
                    Ok(results) => results[n].as_ref().err().cloned(),
                    Err(_) => Some("Import worker failed".into()),
                };
                if let Some(error) = error {
                    eprintln!("digit: desktop drop: {error}");
                    let Desktop {
                        saved,
                        positions,
                        pending_imports,
                        ..
                    } = &mut *state;
                    rollback_import(saved, positions, pending_imports, key, pos, old);
                }
            }
            state.importing = false;
        }
        save(&d);
        refresh(&d);
    });
}

fn import(file: &gio::File, dest: &Path) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let folder = dest.parent().ok_or("Missing destination folder")?;
    let target = gio::File::for_path(dest);
    if file.equal(&target) {
        return Ok(());
    }
    if let Some(path) = file.path() {
        // Compare devices without following the source symlink; a cross-device drop is a
        // copy, including folders. Neither path ever overwrites an existing destination.
        let src_dev = std::fs::symlink_metadata(&path)
            .map_err(|e| e.to_string())?
            .dev();
        let dest_dev = std::fs::metadata(folder).map_err(|e| e.to_string())?.dev();
        if src_dev != dest_dev {
            return copy_tree(&path, dest).map_err(|e| e.to_string());
        }
    }
    match file.move_(
        &target,
        gio::FileCopyFlags::NO_FALLBACK_FOR_MOVE,
        gio::Cancellable::NONE,
        None,
    ) {
        Ok(()) => Ok(()),
        Err(e) if e.matches(gio::IOErrorEnum::NotSupported) => match file.path() {
            Some(path) => copy_tree(&path, dest).map_err(|e| e.to_string()),
            None => file
                .copy(
                    &target,
                    gio::FileCopyFlags::NONE,
                    gio::Cancellable::NONE,
                    None,
                )
                .map_err(|e| e.to_string()),
        },
        Err(e) => Err(e.to_string()),
    }
}
fn copy_tree(src: &Path, dest: &Path) -> std::io::Result<()> {
    use std::io;
    let meta = std::fs::symlink_metadata(src)?;
    if meta.is_symlink() {
        std::os::unix::fs::symlink(std::fs::read_link(src)?, dest)
    } else if meta.is_dir() {
        std::fs::create_dir(dest)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_tree(&entry.path(), &dest.join(entry.file_name()))?;
        }
        std::fs::set_permissions(dest, meta.permissions())
    } else if meta.is_file() {
        let mut input = std::fs::File::open(src)?;
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dest)?;
        io::copy(&mut input, &mut output)?;
        output.set_permissions(meta.permissions())
    } else {
        Err(io::Error::other("Unsupported file type"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_visual_follows_desktop_reentry_but_never_restores_the_source_on_leave() {
        assert!(drag_items_visible(false, true, false, DragEnd::Active));
        for over in [true, false, true, false] {
            assert_eq!(drag_items_visible(true, true, over, DragEnd::Active), over);
        }
        assert!(!drag_items_visible(true, true, false, DragEnd::Performed));
        assert!(drag_items_visible(true, true, false, DragEnd::Local));
        // On niri the compositor owns the visual even over the desktop.
        assert!(!drag_items_visible(true, false, true, DragEnd::Active));
        // A terminal restoration removes the system drag before showing items.
        assert!(drag_items_visible(false, true, false, DragEnd::Ended));
    }

    #[test]
    fn frozen_preview_keeps_the_press_across_windows_and_desktop_reentry() {
        let press = (337.75, 229.25);
        let drag = Drag {
            start: press,
            origins: [("a".into(), (300.25, 200.5)), ("b".into(), (20.5, 60.25))]
                .into_iter().collect(),
        };
        let (_, hot) = preview_bounds(&drag.origins, press);
        for pointer in [(351.0, 237.5), (951.0, 537.5), (401.0, 317.5)] {
            // The same frozen surface follows the pointer on either target;
            // neither leaving nor re-entering may rebase it to the old cell.
            let moved = drag.at(pointer.0, pointer.1);
            for (name, &(x, y)) in &drag.origins {
                let in_preview = (x + hot.0 as f64 - press.0, y + hot.1 as f64 - press.1);
                assert_eq!(moved[name], (pointer.0 - hot.0 as f64 + in_preview.0,
                                        pointer.1 - hot.1 as f64 + in_preview.1));
            }
        }
    }

    #[test]
    fn preview_hotspot_is_the_press_minus_bounds_origin_for_one_item_and_groups() {
        let cell = (300.0, 200.0);
        for grab in [(0.0, 0.0), (37.0, 29.0), (CELL_W as f64 - 1.0, 80.0)] {
            let press = (cell.0 + grab.0, cell.1 + grab.1);
            for grouped in [false, true] {
                let mut origins = BTreeMap::from([("grabbed".into(), cell)]);
                if grouped {
                    origins.insert("above-left".into(), (20.0, 60.0));
                    origins.insert("below-right".into(), (480.0, 350.0));
                }
                let (bounds, hot) = preview_bounds(&origins, press);
                assert_eq!(hot.0 as f64, press.0 - bounds.x);
                assert_eq!(hot.1 as f64, press.1 - bounds.y);
                assert_eq!(hot.0 as f64 - (cell.0 - bounds.x), grab.0);
                assert_eq!(hot.1 as f64 - (cell.1 - bounds.y), grab.1);

                let drag = Drag {
                    start: press,
                    origins,
                };
                // Crossing the threshold moves both the cells and their bounds;
                // it must not rebase the grab to the threshold event's position.
                let pointer = (press.0 + 19.0, press.1 - 13.0);
                let moved = drag.at(pointer.0, pointer.1);
                let (moved_bounds, moved_hot) = preview_bounds(&moved, pointer);
                assert_eq!(moved_hot, hot);
                assert_eq!(moved_hot.0 as f64, pointer.0 - moved_bounds.x);
                assert_eq!(moved_hot.1 as f64, pointer.1 - moved_bounds.y);
            }
        }
    }
    #[test]
    fn preview_fits_one_icon_and_its_shadow_with_the_exact_fractional_grab() {
        let origins = [("a".into(), (200.25, 100.5))].into_iter().collect();
        let pointer = (237.75, 129.25);
        let (bounds, hot) = preview_bounds(&origins, pointer);
        assert_eq!(
            (bounds.x, bounds.y, bounds.w, bounds.h),
            (192.0, 92.0, 129.0, 133.0)
        );
        assert_eq!(hot, (46, 37));
        assert!(bounds.contains(pointer.0, pointer.1));
        // The surface is positioned at pointer - integer hotspot. Its contents
        // compensate the fractional remainder, so no grab distance is rounded.
        let in_preview = (
            origins["a"].0 + hot.0 as f64 - pointer.0,
            origins["a"].1 + hot.1 as f64 - pointer.1,
        );
        assert_eq!(pointer.0 - hot.0 as f64 + in_preview.0, origins["a"].0);
        assert_eq!(pointer.1 - hot.1 as f64 + in_preview.1, origins["a"].1);
    }
    #[test]
    fn preview_group_includes_icons_left_and_above_the_grab_and_keeps_its_first_frame() {
        let drag = Drag {
            start: (337.75, 229.25),
            origins: [("a".into(), (300.25, 200.5)), ("b".into(), (20.5, 60.25))]
                .into_iter()
                .collect(),
        };
        let pointer = (351.0, 237.5); // Already moved before the DND threshold.
        let moved = drag.at(pointer.0, pointer.1);
        let (bounds, hot) = preview_bounds(&moved, pointer);
        for &(x, y) in moved.values() {
            assert!(bounds.contains(x - 6.0, y - 6.0));
            assert!(bounds.contains(x + CELL_W as f64 + 5.0, y + CELL_H as f64 + 5.0));
            let in_preview = (
                x.round() + hot.0 as f64 - pointer.0,
                y.round() + hot.1 as f64 - pointer.1,
            );
            assert_eq!(pointer.0 - hot.0 as f64 + in_preview.0, x.round());
            assert_eq!(pointer.1 - hot.1 as f64 + in_preview.1, y.round());
        }
        let dropped = drag.dropped(grid(), pointer.0, pointer.1, 0);
        for (name, &(x, y)) in &moved {
            assert_eq!((dropped[name].x, dropped[name].y), (x, y));
        }
    }
    #[test]
    fn external_drop_cascade_is_centered_clamped_and_wins_over_automatic_placement() {
        let positions = drop_cascade(grid(), (250.5, 200.25), 1, 3);
        assert_eq!(
            positions[0],
            FreePos {
                x: 194.5,
                y: 142.25,
                page: 1
            }
        );
        assert_eq!(
            positions[2],
            FreePos {
                x: 242.5,
                y: 190.25,
                page: 1
            }
        );
        for pointer in [(0.0, 0.0), (800.0, 600.0)] {
            assert!(
                drop_cascade(grid(), pointer, 0, 5)
                    .iter()
                    .all(|p| grid().clamp(*p) == *p)
            );
        }
        assert!(drop_cascade(grid(), (0.0, 0.0), 0, 0).is_empty());
        let saved = [("incoming".into(), positions[0])].into_iter().collect();
        let placed = place(&["incoming".into(), "other".into()], &saved, grid());
        assert_eq!(placed["incoming"], positions[0]);
    }
    #[test]
    fn failed_import_removes_its_reservation_but_preserves_existing_or_newer_placements() {
        let seeded = drop_cascade(grid(), (250.0, 200.0), 0, 1)[0];
        let old = grid().nth(0);
        for previous in [None, Some(old)] {
            let mut saved: Positions = [("incoming".into(), seeded)].into_iter().collect();
            let mut positions = saved.clone();
            let mut pending = saved.clone();
            rollback_import(
                &mut saved,
                &mut positions,
                &mut pending,
                "incoming".into(),
                seeded,
                previous,
            );
            assert!(pending.is_empty());
            assert_eq!(saved.get("incoming").copied(), previous);
            assert_eq!(positions, saved);
        }
        let mut saved: Positions = [("incoming".into(), old)].into_iter().collect();
        let mut positions = saved.clone();
        let mut pending = [("incoming".into(), seeded)].into_iter().collect();
        rollback_import(
            &mut saved,
            &mut positions,
            &mut pending,
            "incoming".into(),
            seeded,
            None,
        );
        assert!(pending.is_empty());
        assert_eq!(saved["incoming"], old);
        assert_eq!(positions["incoming"], old);
    }
    #[test]
    fn collision_names_preserve_suffixes_directories_and_non_utf8_names() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        assert_eq!(
            collision_name(OsStr::new("a.txt"), false, |_| false),
            "a.txt"
        );
        assert_eq!(
            collision_name(OsStr::new("a.txt"), false, |n| n == "a.txt"
                || n == "a (2).txt"),
            "a (3).txt"
        );
        assert_eq!(
            collision_name(OsStr::new("folder.ext"), true, |n| n == "folder.ext"),
            "folder.ext (2)"
        );
        assert_eq!(
            collision_name(OsStr::new(".hidden"), false, |n| n == ".hidden"),
            ".hidden (2)"
        );
        let name = OsStr::from_bytes(b"a\xff.txt");
        assert_eq!(
            collision_name(name, false, |n| n == name),
            OsStr::from_bytes(b"a\xff (2).txt")
        );
    }
    #[test]
    fn import_plans_reserve_final_names_before_any_transfer_and_keep_existing_desktop_files() {
        let dir = temp();
        let desktop = dir.join("Desktop");
        std::fs::create_dir_all(&desktop).unwrap();
        std::fs::create_dir(dir.join("first")).unwrap();
        std::fs::create_dir(dir.join("second")).unwrap();
        std::fs::write(desktop.join("a.txt"), b"desktop").unwrap();
        std::fs::write(dir.join("first/a.txt"), b"first").unwrap();
        std::fs::write(dir.join("second/a.txt"), b"second").unwrap();
        // Dangling symlinks also occupy names.
        std::os::unix::fs::symlink("missing", desktop.join("a (2).txt")).unwrap();
        let paths = [
            dir.join("first/a.txt"),
            dir.join("second/a.txt"),
            desktop.join("a.txt"),
        ];
        let uris = paths
            .iter()
            .map(|p| gio::File::for_path(p).uri().to_string())
            .collect();
        let plans = plan_imports(uris, &desktop, BTreeSet::new());
        assert_eq!(
            plans.iter().map(|p| p.dest.clone()).collect::<Vec<_>>(),
            [
                desktop.join("a (3).txt"),
                desktop.join("a (4).txt"),
                desktop.join("a.txt"),
            ]
        );
        assert!(!desktop.join("a (3).txt").exists());
        assert!(!desktop.join("a (4).txt").exists());
        for plan in plans {
            import(&gio::File::for_uri(&plan.uri), &plan.dest).unwrap();
        }
        assert_eq!(std::fs::read(desktop.join("a.txt")).unwrap(), b"desktop");
        assert_eq!(std::fs::read(desktop.join("a (3).txt")).unwrap(), b"first");
        assert_eq!(std::fs::read(desktop.join("a (4).txt")).unwrap(), b"second");
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn drag_end_waits_for_transfer_and_finishes_once_in_either_signal_order() {
        for moved in [false, true] {
            for source_first in [false, true] {
                let mut state = DragEnd::default();
                assert_eq!(state.event(DragEvent::Performed), None);
                if source_first {
                    assert_eq!(state.event(DragEvent::SourceEnd), None);
                }
                assert_eq!(
                    state.event(DragEvent::Finished { moved }),
                    Some(if moved {
                        DragResult::AwaitRemoval
                    } else {
                        DragResult::Restore
                    })
                );
                assert_eq!(state.event(DragEvent::SourceEnd), None);
                assert_eq!(state.event(DragEvent::Cancel), None);
                assert_eq!(state.event(DragEvent::Finished { moved }), None);
            }
        }
        for performed in [false, true] {
            let mut state = DragEnd::default();
            if performed {
                state.event(DragEvent::Performed);
            }
            assert_eq!(state.event(DragEvent::Cancel), Some(DragResult::Restore));
            assert_eq!(state.event(DragEvent::SourceEnd), None);
            assert_eq!(state.event(DragEvent::Cancel), None);
        }
    }
    #[test]
    fn local_drop_can_precede_or_follow_performed_and_is_never_an_external_move() {
        for performed_first in [false, true] {
            for cancelled in [false, true] {
                let mut state = DragEnd::default();
                if performed_first {
                    state.event(DragEvent::Performed);
                }
                assert_eq!(state.event(DragEvent::LocalDrop), None);
                if !performed_first {
                    state.event(DragEvent::Performed);
                }
                assert_eq!(state, DragEnd::Local);
                assert_eq!(
                    state.event(if cancelled {
                        DragEvent::Cancel
                    } else {
                        DragEvent::Finished { moved: true }
                    }),
                    Some(DragResult::KeepLocal)
                );
                assert_eq!(state.event(DragEvent::SourceEnd), None);
            }
        }
    }
    fn grid() -> Grid {
        Grid::new(800, 600, 50)
    }
    fn names(count: usize) -> Vec<String> {
        (0..count).map(|n| format!("file{n:03}")).collect()
    }
    fn temp() -> PathBuf {
        std::env::temp_dir().join(format!(
            "digit-deskicons-{}-{}",
            std::process::id(),
            glib::random_int()
        ))
    }
    fn assert_separate(positions: &Positions) {
        let positions: Vec<_> = positions.values().collect();
        for (n, p) in positions.iter().enumerate() {
            assert!(positions[n + 1..].iter().all(|other| !p.overlaps(**other)));
        }
    }
    #[test]
    fn new_items_fill_down_from_top_right_without_displacing_saved_items() {
        let g = grid();
        let names = names(8);
        let saved = [(names[2].clone(), g.nth(0))].into_iter().collect();
        let p = place(&names, &saved, g);
        assert_eq!(p[&names[2]], g.nth(0));
        assert_eq!(p[&names[0]], g.nth(1));
        assert!(g.nth(0).x > g.nth(g.rows as usize).x);
        assert_separate(&p);
    }
    fn battery_grid(placement: &str, clock: &str) -> Grid {
        let mut g = grid();
        g.reserved = crate::batterywidget::card_rect(
            placement,
            crate::batterywidget::SHOWN,
            (800, 600),
            26,
            clock,
        )
        .map(|r| Rect {
            x: r.x,
            y: r.y,
            w: r.w,
            h: r.h,
        });
        g
    }
    #[test]
    fn automatic_placement_skips_battery_cells_in_every_corner_and_page() {
        let names = names(100);
        for corner in ["top_left", "top_right", "bottom_left", "bottom_right"] {
            for clock in ["off", "left"] {
                let g = battery_grid(corner, clock);
                assert!(
                    !corner.starts_with("top")
                        || (0..(g.cols * g.rows) as usize).any(|n| g.reserved(g.nth(n)))
                );
                let p = place(&names, &Positions::new(), g);
                assert!(p.values().all(|p| !g.reserved(*p)), "{corner}, {clock}");
                assert_separate(&p);
                assert_eq!(place(&names, &p, g), p);
            }
        }
    }
    #[test]
    fn automatic_placement_skips_free_saved_rectangles_and_the_battery_card() {
        let g = battery_grid("top_left", "off");
        let names = names(20);
        let saved_pos = FreePos {
            x: g.nth(0).x - 10.5,
            y: 65.25,
            page: 0,
        };
        let saved = [(names[10].clone(), saved_pos)].into_iter().collect();
        let p = place(&names, &saved, g);
        assert_eq!(p[&names[10]], saved_pos);
        assert!(!p.values().any(|p| g.reserved(*p)));
        assert_separate(&p);
        assert_ne!(p[&names[0]], g.nth(0));
        assert_ne!(p[&names[1]], g.nth(1));
    }
    #[test]
    fn saved_positions_inside_battery_card_and_overlaps_stay_put() {
        let names = names(3);
        let g = battery_grid("top_right", "off");
        let saved = [(names[0].clone(), g.nth(0)), (names[1].clone(), g.nth(0))]
            .into_iter()
            .collect();
        assert!(g.reserved(g.nth(0)));
        let p = place(&names, &saved, g);
        assert_eq!(p[&names[0]], g.nth(0));
        assert_eq!(p[&names[1]], g.nth(0));
        assert!(!g.reserved(p[&names[2]]));
    }
    #[test]
    fn smaller_screen_pulls_restored_positions_to_the_nearest_edge_without_regridding() {
        let names = names(3);
        let old = Grid::new(1920, 1080, 50);
        let edge = FreePos {
            x: 1700.5,
            y: 900.25,
            page: 2,
        };
        let untouched = FreePos {
            x: 223.75,
            y: 101.5,
            page: 0,
        };
        let saved = [
            (names[0].clone(), edge),
            (names[1].clone(), untouched),
            (names[2].clone(), edge),
        ]
        .into_iter()
        .collect();
        assert_eq!(place(&names, &saved, old), saved);
        let p = place(&names, &saved, grid());
        assert_eq!(
            p[&names[0]],
            FreePos {
                x: 688.0,
                y: 484.0,
                page: 2
            }
        );
        assert_eq!(p[&names[1]], untouched);
        assert_eq!(p[&names[2]], p[&names[0]]);
    }
    #[test]
    fn automatic_results_stay_stable_when_earlier_names_arrive_or_card_changes() {
        let old_names = vec!["b".into(), "c".into()];
        let saved = place(&old_names, &Positions::new(), grid());
        let new_names = vec!["a".into(), "b".into(), "c".into()];
        let p = place(&new_names, &saved, battery_grid("top_right", "off"));
        for name in old_names {
            assert_eq!(p[&name], saved[&name]);
        }
        assert_eq!(place(&new_names, &p, grid()), p);
    }
    #[test]
    fn five_hundred_items_get_separate_reachable_pages() {
        let names = names(500);
        let g = grid();
        let p = place(&names, &Positions::new(), g);
        assert_eq!(p.len(), 500);
        assert_separate(&p);
        assert!(p.values().all(|p| g.clamp(*p) == *p));
        assert!(p.values().any(|p| p.page > 0));
        assert_eq!(place(&names, &p, g), p);
    }
    #[test]
    fn drag_follows_first_motion_and_dnd_handoff_without_rebasing_the_grab() {
        let drag = Drag {
            start: (237.25, 129.5),
            origins: [("a".into(), (200.0, 100.0)), ("b".into(), (323.5, 211.25))]
                .into_iter()
                .collect(),
        };
        // Sub-threshold motion is visible immediately. The first DND motion
        // continues the same translation rather than recapturing moved origins.
        for (dx, dy) in [(0.0, 0.0), (0.75, 1.25), (12.0, 3.0), (54.5, 68.25)] {
            let pointer = (drag.start.0 + dx, drag.start.1 + dy);
            let p = drag.at(pointer.0, pointer.1);
            assert_eq!(p["a"], (200.0 + dx, 100.0 + dy));
            assert_eq!(pointer.0 - p["a"].0, 37.25);
            assert_eq!(pointer.1 - p["a"].1, 29.5);
            assert_eq!(p["b"].0 - p["a"].0, 123.5);
            assert_eq!(p["b"].1 - p["a"].1, 111.25);
            let dropped = drag.dropped(grid(), pointer.0, pointer.1, 0);
            assert_eq!((dropped["a"].x, dropped["a"].y), p["a"]);
        }
    }
    #[test]
    fn drag_leave_reentry_and_cancel_keep_the_original_group_immutable() {
        let drag = Drag {
            start: (237.25, 129.5),
            origins: [("a".into(), (200.0, 100.0)), ("b".into(), (323.5, 211.25))]
                .into_iter()
                .collect(),
        };
        let origins = drag.origins.clone();
        let before_leave = drag.at(412.5, 310.25);
        assert_ne!(before_leave, origins);
        assert_eq!(drag.restored(), origins);
        assert_eq!(drag.at(412.5, 310.25), before_leave);
        let _ = drag.dropped(grid(), -9999.0, 9999.0, 0);
        // Cancellation, including a release before DND, never uses the last
        // pointer position or a clamped drop as its return destination.
        assert_eq!(drag.restored(), origins);
    }
    #[test]
    fn dragging_preserves_a_selected_group_and_exports_a_toggled_off_anchor_alone() {
        let selected: BTreeSet<_> = ["a".into(), "b".into()].into_iter().collect();
        assert_eq!(drag_selection(&selected, "b"), selected);
        assert_eq!(
            drag_selection(&selected, "c"),
            ["c".into()].into_iter().collect()
        );
        assert_eq!(selected.len(), 2);
    }
    #[test]
    fn drop_keeps_exact_fractional_coordinates_and_selection_offsets() {
        let drag = Drag {
            start: (230.0, 120.0),
            origins: [("a".into(), (200.0, 100.0)), ("b".into(), (323.5, 211.25))]
                .into_iter()
                .collect(),
        };
        let p = drag.dropped(grid(), 247.25, 149.5, 1);
        assert_eq!(
            p["a"],
            FreePos {
                x: 217.25,
                y: 129.5,
                page: 1
            }
        );
        assert_eq!(
            p["b"],
            FreePos {
                x: 340.75,
                y: 240.75,
                page: 1
            }
        );
        // Dropping onto an existing icon is allowed; restoration keeps both there.
        let mut saved = p.clone();
        saved.insert("c".into(), p["a"]);
        assert_eq!(
            place(&["a".into(), "b".into(), "c".into()], &saved, grid()),
            saved
        );
    }
    #[test]
    fn clamping_keeps_icons_below_reserve_and_group_offsets_at_edges() {
        let g = grid();
        assert_eq!(
            g.clamp(FreePos {
                x: -20.5,
                y: -10.0,
                page: 0
            }),
            FreePos {
                x: 0.0,
                y: 50.0,
                page: 0
            }
        );
        assert_eq!(
            g.clamp(FreePos {
                x: 9999.0,
                y: 9999.0,
                page: 0
            }),
            FreePos {
                x: 688.0,
                y: 484.0,
                page: 0
            }
        );
        let drag = Drag {
            start: (230.0, 120.0),
            origins: [("a".into(), (200.0, 100.0)), ("b".into(), (323.5, 211.25))]
                .into_iter()
                .collect(),
        };
        for pointer in [(-9999.0, -9999.0), (9999.0, 9999.0)] {
            let p = drag.dropped(g, pointer.0, pointer.1, 0);
            assert_eq!(p["b"].x - p["a"].x, 123.5);
            assert_eq!(p["b"].y - p["a"].y, 111.25);
            assert!(p.values().all(|p| g.clamp(*p) == *p));
        }
    }
    #[test]
    fn sort_by_name_discards_free_positions_and_restores_grid_order() {
        let names = names(30);
        let saved = names
            .iter()
            .map(|name| {
                (
                    name.clone(),
                    FreePos {
                        x: 123.25,
                        y: 111.5,
                        page: 5,
                    },
                )
            })
            .collect();
        assert_eq!(place(&names, &saved, grid()), saved);
        // Sort clears saved positions before using the same name-ordered placement.
        let sorted = place(&names, &Positions::new(), grid());
        for (n, name) in names.iter().enumerate() {
            assert_eq!(sorted[name], grid().nth(n));
        }
        assert_eq!(place(&names, &sorted, grid()), sorted);
    }
    #[test]
    fn rubber_band_works_in_both_directions_and_preserves_ctrl_selection() {
        let r = Rect {
            x: 10.0,
            y: 10.0,
            w: 100.0,
            h: 100.0,
        };
        let items = vec![("one".into(), r), ("two".into(), Rect { x: 200.0, ..r })];
        let initial = ["two".into()].into_iter().collect();
        for band in [
            Rect::between((0.0, 0.0), (50.0, 50.0)),
            Rect::between((50.0, 50.0), (0.0, 0.0)),
        ] {
            assert_eq!(
                rubber_select(band, &items, &initial, false),
                ["one".into()].into_iter().collect()
            );
            assert_eq!(rubber_select(band, &items, &initial, true).len(), 2);
        }
        assert!(
            rubber_select(
                Rect::between((0.0, 0.0), (0.0, 0.0)),
                &items,
                &initial,
                false
            )
            .is_empty()
        );
    }
    #[test]
    fn positions_roundtrip_names_and_reject_corrupt_data() {
        let p = [("notes\n\"日本語.txt".into(), grid().nth(3))]
            .into_iter()
            .collect();
        assert_eq!(decode(&encode(&p).unwrap()).unwrap(), p);
        assert!(decode(b"truncated {").is_err());
        assert!(decode(br#"{"bad":{"col":-1,"row":0}}"#).unwrap().is_empty());
    }
    #[test]
    fn old_format_entries_are_ignored_without_losing_new_positions() {
        let old = br#"{"a":{"col":1,"row":2,"page":0},"b":{"col":0,"row":0}}"#;
        assert!(decode(old).unwrap().is_empty());
        let mixed = br#"{"old":{"col":1,"row":2},"new":{"x":123.5,"y":99.25}}"#;
        assert_eq!(
            decode(mixed).unwrap(),
            [(
                "new".into(),
                FreePos {
                    x: 123.5,
                    y: 99.25,
                    page: 0
                }
            )]
            .into_iter()
            .collect()
        );
    }
    #[test]
    fn unreadable_or_corrupt_state_uses_defaults() {
        let dir = temp();
        std::fs::create_dir_all(&dir).unwrap();
        assert!(read_positions(&dir).unwrap().is_empty());
        assert!(read_positions(&dir.join("missing")).unwrap().is_empty());
        std::fs::write(dir.join("broken.json"), b"truncated {").unwrap();
        assert!(read_positions(&dir.join("broken.json")).unwrap().is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn atomic_save_replaces_complete_state_and_reports_failure() {
        let dir = temp();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("icons.json");
        let p = [("a".into(), grid().nth(0))].into_iter().collect();
        write_positions(&path, &p).unwrap();
        assert_eq!(read_positions(&path).unwrap(), p);
        write_positions(&path, &Positions::new()).unwrap();
        assert!(read_positions(&path).unwrap().is_empty());
        assert!(write_positions(&path.join("impossible.json"), &p).is_err());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn copy_preserves_original_symlinks_and_never_overwrites() {
        let dir = temp();
        std::fs::create_dir_all(dir.join("src/sub")).unwrap();
        std::fs::write(dir.join("src/sub/a"), b"original").unwrap();
        std::os::unix::fs::symlink("sub/a", dir.join("src/link")).unwrap();
        copy_tree(&dir.join("src"), &dir.join("dest")).unwrap();
        assert_eq!(std::fs::read(dir.join("dest/link")).unwrap(), b"original");
        assert_eq!(
            std::fs::read_link(dir.join("dest/link")).unwrap(),
            PathBuf::from("sub/a")
        );
        assert!(copy_tree(&dir.join("src/sub/a"), &dir.join("dest/sub/a")).is_err());
        assert!(dir.join("src/sub/a").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn scan_hides_dotfiles_and_reads_desktop_entry_metadata() {
        let dir = temp();
        std::fs::create_dir_all(dir.join("Folder")).unwrap();
        std::fs::write(dir.join(".hidden"), b"hidden").unwrap();
        std::fs::write(
            dir.join("launch.desktop"),
            "[Desktop Entry]\nType=Application\nName=Desktop test\nIcon=folder\nExec=true\n",
        )
        .unwrap();
        let items = scan(&dir).unwrap();
        assert_eq!(items.len(), 2);
        let launcher = items.iter().find(|f| f.launcher).unwrap();
        assert_eq!(launcher.label, "Desktop test");
        assert_eq!(launcher.icon.as_deref(), Some("folder"));
        assert!(items.iter().any(|f| f.directory));
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn import_moves_on_same_filesystem_and_refuses_collisions() {
        let dir = temp();
        std::fs::create_dir_all(dir.join("Desktop")).unwrap();
        let source = dir.join("a");
        let dest = dir.join("Desktop/a");
        std::fs::write(&source, b"original").unwrap();
        import(&gio::File::for_path(&source), &dest).unwrap();
        assert!(!source.exists());
        std::fs::write(&source, b"replacement").unwrap();
        assert!(import(&gio::File::for_path(&source), &dest).is_err());
        assert_eq!(std::fs::read(&dest).unwrap(), b"original");
        assert_eq!(std::fs::read(&source).unwrap(), b"replacement");
        import(&gio::File::for_path(&dest), &dest).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn invalid_utf8_filenames_have_distinct_persistence_keys() {
        use std::os::unix::ffi::OsStrExt;
        let a = name_key(std::ffi::OsStr::from_bytes(b"a\xff"));
        let b = name_key(std::ffi::OsStr::from_bytes(b"a\xfe"));
        assert_ne!(a, b);
        assert_ne!(a, name_key(std::ffi::OsStr::new("a�")));
        let positions = [(a, grid().nth(0)), (b, grid().nth(1))]
            .into_iter()
            .collect();
        assert_eq!(decode(&encode(&positions).unwrap()).unwrap(), positions);
    }
    #[test]
    fn desktop_icons_default_on_with_explicit_off_switch() {
        let default: crate::config::Config = toml::from_str("").unwrap();
        assert!(crate::config::Prefs::from(&default).desktop_icons);
        let disabled: crate::config::Config = toml::from_str("desktop_icons = false").unwrap();
        assert!(!crate::config::Prefs::from(&disabled).desktop_icons);
    }
    #[test]
    fn desktop_menu_entries_follow_context_and_order() {
        use MenuAction::*;
        for count in [0, 1, 3] {
            assert_eq!(
                menu_entries(false, count),
                vec![
                    NewFolder,
                    Wallpaper,
                    Terminal,
                    Sort,
                    OpenFolder,
                    Separator,
                    DesktopSettings,
                    DisplaySettings
                ]
            );
        }
        assert_eq!(
            menu_entries(true, 1),
            vec![Open, Reveal, Rename, CopyPath, Trash]
        );
        assert_eq!(menu_entries(true, 2), vec![Open, Trash]);
        assert_eq!(menu_entries(true, 20), vec![Open, Trash]);
        assert_eq!(
            menu_entries(false, 0)
                .iter()
                .map(|a| a.label())
                .collect::<Vec<_>>(),
            vec![
                "New Folder",
                "Change Wallpaper",
                "Open Terminal Here",
                "Sort by Name",
                "Open Desktop Folder",
                "",
                "Desktop Settings",
                "Display Settings",
            ]
        );
        assert_eq!(
            menu_entries(true, 1)
                .iter()
                .map(|a| a.label())
                .collect::<Vec<_>>(),
            vec![
                "Open",
                "Show in Files",
                "Rename",
                "Copy Path",
                "Move to Trash",
            ]
        );
    }
    #[test]
    fn next_folder_name_starts_at_two_and_fills_gaps() {
        assert_eq!(next_folder_name(|_| false).as_deref(), Some("New Folder"));
        let occupied = ["New Folder", "New Folder 2", "New Folder 4"];
        assert_eq!(
            next_folder_name(|n| occupied.contains(&n)).as_deref(),
            Some("New Folder 3")
        );
        assert_eq!(
            next_folder_name(|n| n == "New Folder").as_deref(),
            Some("New Folder 2")
        );
        assert!(next_folder_name(|_| true).is_none());
    }
    #[test]
    fn denied_empty_point_drag_and_duplicate_end_leave_no_rubber_state() {
        let selected = BTreeSet::from(["kept".to_string()]);
        // Empty space, but a non-primary gesture or an active file drag denies the band.
        for (primary, dragging) in [(false, false), (true, true)] {
            let model = RefCell::new(begin_band(
                false,
                primary,
                dragging,
                (10.0, 20.0),
                selected.clone(),
                false,
            ));
            assert!(model.borrow().is_none());
            // Model the synchronous drag-end followed by cancel after set_state(Denied).
            assert!(finish_band(&mut model.borrow_mut(), false).is_none());
            assert!(finish_band(&mut model.borrow_mut(), true).is_none());
            assert!(model.borrow().is_none());
            assert_eq!(selected, BTreeSet::from(["kept".to_string()]));
        }
        let mut band = begin_band(false, true, false, (10.0, 20.0), selected.clone(), true);
        assert!(band.is_some());
        assert_eq!(finish_band(&mut band, true), Some(selected));
        assert!(band.is_none());
        assert!(finish_band(&mut band, false).is_none());
        assert!(begin_band(true, true, false, (10.0, 20.0), BTreeSet::new(), false).is_none());
    }
    #[test]
    fn new_folder_uses_a_free_name() {
        let dir = temp();
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(new_folder(&dir).unwrap(), dir.join("New Folder"));
        std::fs::write(dir.join("New Folder 2"), b"also occupied").unwrap();
        std::os::unix::fs::symlink("missing", dir.join("New Folder 3")).unwrap();
        assert_eq!(new_folder(&dir).unwrap(), dir.join("New Folder 4"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
