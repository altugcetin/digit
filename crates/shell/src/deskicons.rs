//! Desktop files live on Bottom, above the Background wallpaper and below windows.
//! The wallpaper currently has no pointer controllers. This surface owns empty-desktop
//! clicks too, so rubber-band selection and external file drops work anywhere.
use crate::{
    app::App,
    draw::{self, Face},
    paint::{self, Paint},
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
    /// Where desktop widgets sit (the batteries and the corner widgets), which icons keep out of.
    reserved: [Option<Rect>; 4],
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
            reserved: [None; 4],
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
        self.reserved.iter().flatten().any(|r| r.intersects(p.rect()))
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
#[cfg(test)]
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
type OutputNames = BTreeMap<String, String>;
fn decode_state(bytes: &[u8]) -> Result<(Positions, OutputNames), serde_json::Error> {
    let positions = decode(bytes)?;
    let entries: BTreeMap<String, serde_json::Value> = serde_json::from_slice(bytes)?;
    let outputs = entries
        .into_iter()
        .filter_map(|(name, value)| Some((name, value.get("output")?.as_str()?.to_owned())))
        .collect();
    Ok((positions, outputs))
}
fn encode_state(
    positions: &Positions,
    outputs: &OutputNames,
) -> Result<Vec<u8>, serde_json::Error> {
    let mut entries = serde_json::to_value(positions)?;
    for (name, output) in outputs {
        if let Some(entry) = entries
            .get_mut(name)
            .and_then(serde_json::Value::as_object_mut)
        {
            entry.insert("output".into(), output.clone().into());
        }
    }
    serde_json::to_vec_pretty(&entries)
}
fn write_state(path: &Path, positions: &Positions, outputs: &OutputNames) -> Result<(), String> {
    write_bytes(
        path,
        &encode_state(positions, outputs).map_err(|e| e.to_string())?,
    )
}
fn state_path() -> PathBuf {
    kit::theme::state_dir().join("desktop-icons.json")
}
#[cfg(test)]
fn read_positions(path: &Path) -> Result<Positions, String> {
    // Missing, unreadable or corrupt state always starts from the default layout.
    Ok(std::fs::read(path)
        .ok()
        .and_then(|bytes| decode(&bytes).ok())
        .unwrap_or_default())
}
#[cfg(test)]
fn write_positions(path: &Path, positions: &Positions) -> Result<(), String> {
    write_bytes(path, &encode(positions).map_err(|e| e.to_string())?)
}
fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
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
        file.write_all(bytes)?;
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
    target_output: Option<String>,
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
// Folder metadata, selection and persistence have one owner. GTK resources and
// transient geometry live on each surface (including scale-specific icons/layouts).
#[derive(Default)]
struct Folder {
    saved: Positions,
    outputs: BTreeMap<String, String>,
    selected: BTreeSet<String>,
    files: Vec<FileData>,
    primary: String,
    connected: Vec<String>,
    monitor: Option<gio::FileMonitor>,
    pending: Option<glib::SourceId>,
    loading: bool,
    dirty: bool,
    ready: bool,
    drag_names: BTreeSet<String>,
    moved_out: BTreeSet<String>,
    importing: bool,
}

fn item_output<'a>(saved: Option<&'a str>, connected: &[String], primary: &'a str) -> &'a str {
    saved
        .filter(|name| connected.iter().any(|c| c == name))
        .unwrap_or(primary)
}

// Missing outputs are temporary guests on primary. Never write their fallback
// coordinates over the connector's saved home, even after a geometry change.
fn place_output(
    names: &[String],
    saved: &Positions,
    outputs: &BTreeMap<String, String>,
    connected: &[String],
    primary: &str,
    output: &str,
    grid: Grid,
) -> Positions {
    let names: Vec<_> = names
        .iter()
        .filter(|n| item_output(outputs.get(*n).map(String::as_str), connected, primary) == output)
        .cloned()
        .collect();
    let home: Positions = saved
        .iter()
        .filter(|(n, _)| outputs.get(*n).is_none_or(|o| o == output))
        .map(|(n, p)| (n.clone(), *p))
        .collect();
    place(&names, &home, grid)
}

fn remember_output(
    positions: &Positions,
    saved: &mut Positions,
    outputs: &mut OutputNames,
    output: &str,
) {
    for (name, &pos) in positions {
        if outputs.get(name).is_none_or(|o| o == output) {
            saved.insert(name.clone(), pos);
            outputs.insert(name.clone(), output.to_owned());
        }
    }
}

// GTK may reset its click count when keyboard acquisition replaces the layer.
// Keep the user's click sequence in the surface model across that replacement.
struct ClickRecord {
    name: String,
    time: i64,
    point: (f64, f64),
}
impl ClickRecord {
    fn follows(
        &self,
        name: &str,
        time: i64,
        point: (f64, f64),
        interval: i64,
        distance: f64,
    ) -> bool {
        self.name == name
            && (0..=interval).contains(&(time - self.time))
            && (self.point.0 - point.0).abs() <= distance
            && (self.point.1 - point.1).abs() <= distance
    }
}

#[derive(Clone)]
struct IncomingDrag {
    preview: gdk::Paintable,
    pointer: (f64, f64),
    hotspot: (i32, i32),
}

struct Desktop {
    // The item being renamed: its label is the editor while that lasts.
    renaming: Option<String>,
    shared: Rc<RefCell<Folder>>,
    output: String,
    primary: bool,
    keyboard: bool,
    acquiring: bool,
    pointer_down: bool,
    last_click: Option<ClickRecord>,
    retiring: Option<gtk::Window>,
    theme: Option<(gtk::IconTheme, glib::SignalHandlerId)>,
    // Surface render caches; saved is a working copy for imports/rename/drag.
    baseline: Positions,
    incoming: Option<IncomingDrag>,
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
    importing: bool,
    pending_imports: Positions,
    ticking: bool,
    closed: bool,
    rename_window: Option<gtk::Window>,
    rename_pending: Option<String>,
}
thread_local! {
    static DESKTOPS: RefCell<Vec<Rc<RefCell<Desktop>>>> = const { RefCell::new(Vec::new()) };
    static FOLDER: RefCell<Option<Rc<RefCell<Folder>>>> = const { RefCell::new(None) };
    // One serial writer across surface replacements; the newest queued state wins.
    static FOCUS_GENERATION: Cell<u64> = const { Cell::new(0) };
    static WRITING: Cell<bool> = const { Cell::new(false) };
    static TO_WRITE: RefCell<Option<(Positions, BTreeMap<String, String>)>> = const { RefCell::new(None) };
}

/// Renaming happens in the icon's label: plain text with a thin plate, no field around it.
pub fn style(display: &gdk::Display) {
    let windows = gtk::Window::toplevels();
    windows.connect_items_changed(|windows, at, _, added| {
        for n in at..at + added {
            if let Some(window) = windows.item(n).and_downcast::<gtk::Window>() {
                window.connect_is_active_notify(|window| {
                    if !window.has_css_class("digit-lock")
                        && window.is_active()
                        && window.is_layer_window()
                        && paint::layer_namespace(window).as_deref() != Some("digit-desktop-icons")
                    {
                        defer_release_keyboard();
                    }
                });
                window.connect_map(|window| {
                    if !window.has_css_class("digit-lock")
                        && window.is_layer_window()
                        && paint::layer_namespace(window).as_deref() != Some("digit-desktop-icons")
                        && (matches!(window.layer(), Layer::Top | Layer::Overlay)
                            || window.keyboard_mode() != KeyboardMode::None)
                    {
                        defer_release_keyboard();
                    }
                });
            }
        }
    });
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

#[derive(Clone, Copy)]
enum KeyboardEvent {
    Select,
    Clear,
    Window,
    Overlay,
}
fn keyboard_requested(selected: bool, event: KeyboardEvent) -> bool {
    selected && matches!(event, KeyboardEvent::Select)
}

fn release_surface_for(d: &Rc<RefCell<Desktop>>, event: KeyboardEvent) -> bool {
    let (window, retiring) = {
        let mut d = d.borrow_mut();
        if !d.keyboard {
            return false;
        }
        d.keyboard = keyboard_requested(!d.selected.is_empty() || can_undo(), event);
        d.acquiring = false;
        (d.window.clone(), d.retiring.take())
    };
    window.set_keyboard_mode(KeyboardMode::None);
    if let Some(retiring) = retiring {
        retiring.destroy();
    }
    true
}

fn release_surface(d: &Rc<RefCell<Desktop>>) -> bool {
    release_surface_for(d, KeyboardEvent::Overlay)
}

/// Hyprland 0.57 Window.cpp's LS-focus check marks new windows NO_INITIAL_FOCUS.
/// LayerSurface::onCommit(None) only refocuses the *old* last window. Correct that
/// once for the exact openwindow address, after GTK committed the mode change.
fn needs_focus_correction(opened: u64, previous: Option<u64>, current: Option<u64>) -> bool {
    current != Some(opened) && (current.is_none() || current == previous)
}

pub fn window_focus(opened: Option<u64>) {
    let current = surfaces();
    if !current.iter().any(|d| d.borrow().keyboard) {
        return;
    }
    let previous = opened.and_then(|_| crate::compositor::focused_id());
    let mut released = false;
    for d in current {
        released |= release_surface_for(&d, KeyboardEvent::Window);
    }
    if let (true, Some(id)) = (released, opened) {
        // GTK's configure/paint commit runs before an idle at DEFAULT_IDLE.
        // A display roundtrip then orders the layer commit before IPC focus.
        let generation = FOCUS_GENERATION.get();
        glib::idle_add_local_once(move || {
            if FOCUS_GENERATION.get() != generation {
                return;
            }
            if let Some(display) = gdk::Display::default() {
                display.sync();
            }
            // Preserve any newer user/application focus that won the race.
            let focused = crate::compositor::focused_id();
            if needs_focus_correction(id, previous, focused) {
                crate::compositor::focus(id);
            }
        });
    }
}

/// Called before shell overlays take over and for compositor open/focus events.
/// Selection remains visible, but only another desktop gesture can reacquire keys.
pub fn release_keyboard() -> bool {
    FOCUS_GENERATION.set(FOCUS_GENERATION.get().wrapping_add(1));
    let mut released = false;
    for d in surfaces() {
        released |= release_surface(&d);
    }
    released
}

fn defer_release_keyboard() {
    FOCUS_GENERATION.set(FOCUS_GENERATION.get().wrapping_add(1));
    let generation = FOCUS_GENERATION.get();
    glib::idle_add_local_once(move || {
        if FOCUS_GENERATION.get() == generation {
            release_keyboard();
        }
    });
}

fn focus_selection(d: &Rc<RefCell<Desktop>>) {
    FOCUS_GENERATION.set(FOCUS_GENERATION.get().wrapping_add(1));
    let target = {
        let mut d = d.borrow_mut();
        let wanted = !d.selected.is_empty() || can_undo();
        let event = if wanted { KeyboardEvent::Select } else { KeyboardEvent::Clear };
        let requested = keyboard_requested(wanted, event);
        d.acquiring |= requested && !d.keyboard;
        d.keyboard = requested;
        d.keyboard.then(|| (d.window.clone(), d.paint.clone()))
    };
    if let Some((window, paint)) = target {
        window.set_keyboard_mode(KeyboardMode::OnDemand);
        paint.grab_focus();
        if window.is_active() {
            release_other_keyboards(d);
        }
    }
}
fn release_other_keyboards(d: &Rc<RefCell<Desktop>>) {
    for other in surfaces() {
        if !Rc::ptr_eq(&other, d) {
            release_surface(&other);
        }
    }
}

// None -> OnDemand on an already mapped layer does not grant keyboard focus
// in Hyprland 0.57 (LayerSurface::onCommit), nor niri's layer-shell handler.
// Remap *after* the pointer sequence ends if it did not receive focus. Mapping
// OnDemand requests focus on both compositors, without an Exclusive grab or
// breaking the click/DND implicit pointer grab. External focus cancels acquiring.
fn acquire_after_release(d: &Rc<RefCell<Desktop>>) {
    let weak = Rc::downgrade(d);
    glib::idle_add_local_once(move || {
        let Some(d) = weak.upgrade() else { return };
        let target = {
            let state = d.borrow();
            (!state.closed
                && state.keyboard
                && state.acquiring
                && !state.pointer_down
                && state.drag.is_none()
                && state.rubber.is_none()
                && state.retiring.is_none())
            .then(|| {
                (
                    state.window.clone(),
                    state.paint.clone(),
                    state.monitor.clone(),
                )
            })
        };
        if let Some((old, paint, monitor)) = target {
            // list_toplevels, not toplevels(): that model's item type isn't Window, and a typed
            // iter over it panics.
            let overlay = gtk::Window::list_toplevels()
                .into_iter()
                .filter_map(|w| w.downcast::<gtk::Window>().ok())
                .any(|w| {
                    !w.has_css_class("digit-lock")
                        && w.is_visible()
                        && w.is_layer_window()
                        && paint::layer_namespace(&w).as_deref() != Some("digit-desktop-icons")
                        && w.keyboard_mode() != KeyboardMode::None
                });
            if overlay {
                release_keyboard();
                return;
            }
            // Keep the old buffer mapped until the new OnDemand surface receives
            // keyboard focus. Unmapping first would refocus the old app and send
            // a misleading activewindow event during acquisition.
            let window = desktop_window(&monitor, KeyboardMode::OnDemand);
            old.set_child(gtk::Widget::NONE);
            window.set_child(Some(&paint));
            {
                let mut state = d.borrow_mut();
                state.retiring = Some(old);
                state.window = window.clone();
            }
            watch_focus(&d, &window);
            window.present();
            paint.grab_focus();
        }
    });
}

fn selected_files(d: &Rc<RefCell<Desktop>>) -> Vec<FileData> {
    let shared = { d.borrow().shared.clone() };
    let f = shared.borrow();
    f.files
        .iter()
        .filter(|i| f.selected.contains(&i.name))
        .cloned()
        .collect()
}

fn surfaces() -> Vec<Rc<RefCell<Desktop>>> {
    DESKTOPS.with(|s| s.borrow().clone())
}

fn destroy_surface(d: &Rc<RefCell<Desktop>>) {
    let (window, retiring, theme) = {
        let mut state = d.borrow_mut();
        state.closed = true;
        (
            state.window.clone(),
            state.retiring.take(),
            state.theme.take(),
        )
    };
    cancel_drag(d);
    close_rename(d, None);
    if let Some((theme, handler)) = theme {
        theme.disconnect(handler);
    }
    if let Some(retiring) = retiring {
        retiring.destroy();
    }
    window.destroy();
}

pub fn apply(app: &Rc<App>) {
    let enabled = app.prefs.borrow().desktop_icons;
    let monitors: Vec<_> = gdk::Display::default()
        .map(|d| {
            d.monitors()
                .iter::<gdk::Monitor>()
                .filter_map(Result::ok)
                .collect()
        })
        .unwrap_or_default();
    let preferred = kit::prefs::text(&kit::prefs::read(), "primary_monitor", "");
    let names: Vec<_> = monitors
        .iter()
        .map(|m| m.connector().map(|c| c.to_string()).unwrap_or_default())
        .collect();
    let primary = names
        .iter()
        .find(|n| **n == preferred)
        .or(names.first())
        .cloned()
        .unwrap_or_default();
    let current = surfaces();
    if current.iter().any(|d| d.try_borrow_mut().is_err()) {
        let app = app.clone();
        glib::idle_add_local_once(move || apply(&app));
        return;
    }
    if !enabled {
        DESKTOPS.with(|s| s.borrow_mut().clear());
        let shared = FOLDER.with(|s| s.borrow_mut().take());
        if let Some(shared) = shared {
            let (monitor, pending) = {
                let mut shared = shared.borrow_mut();
                (shared.monitor.take(), shared.pending.take())
            };
            if let Some(m) = monitor {
                m.cancel();
            }
            if let Some(p) = pending {
                p.remove();
            }
        }
        for d in current {
            destroy_surface(&d);
        }
        return;
    }
    let shared = FOLDER.with(|f| f.borrow().clone()).unwrap_or_else(|| {
        let shared = Rc::new(RefCell::new(Folder::default()));
        FOLDER.with(|f| *f.borrow_mut() = Some(shared.clone()));
        shared
    });
    {
        let mut f = shared.borrow_mut();
        f.primary = primary.clone();
        f.connected = names;
    }
    let mut keep = Vec::new();
    for d in current {
        let (old_monitor, output, geometry, paint) = {
            let d = d.borrow();
            (
                d.monitor.clone(),
                d.output.clone(),
                d.geometry,
                d.paint.clone(),
            )
        };
        let fits = monitors
            .iter()
            .find(|m| **m == old_monitor)
            .is_some_and(|m| {
                let g = m.geometry();
                geometry == (g.width(), g.height(), app.reserve())
                    && paint.scale_factor() == m.scale_factor()
            });
        if fits {
            d.borrow_mut().primary = output == primary;
            keep.push(d);
        } else {
            destroy_surface(&d);
        }
    }
    DESKTOPS.with(|s| *s.borrow_mut() = keep);
    for monitor in monitors {
        let output = monitor
            .connector()
            .map(|c| c.to_string())
            .unwrap_or_default();
        if !surfaces().iter().any(|d| d.borrow().output == output) {
            create_surface(app, monitor, shared.clone(), output == primary);
        }
    }
    if let Some(d) = surfaces().first() {
        refresh(d);
    }
}

fn desktop_window(monitor: &gdk::Monitor, mode: KeyboardMode) -> gtk::Window {
    let window = gtk::Window::new();
    window.init_layer_shell();
    window.set_layer(Layer::Bottom);
    window.set_namespace(Some("digit-desktop-icons"));
    window.set_monitor(Some(monitor));
    // Selection can request the keyboard; external focus always revokes that request.
    window.set_keyboard_mode(mode);
    window.set_exclusive_zone(-1);
    for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(edge, true);
    }
    window.add_css_class("digit");
    window
}

fn watch_focus(d: &Rc<RefCell<Desktop>>, window: &gtk::Window) {
    let weak = Rc::downgrade(d);
    window.connect_is_active_notify(move |window| {
        let weak = weak.clone();
        let window = window.downgrade();
        // Other panels can change focus while updating their own RefCell model.
        // Leave the synchronous GTK signal stack before changing any GTK state.
        glib::idle_add_local_once(move || {
            let (Some(d), Some(window)) = (weak.upgrade(), window.upgrade()) else {
                return;
            };
            let active = window.is_active();
            let (release, retiring) = {
                let mut state = d.borrow_mut();
                if state.window != window {
                    return;
                }
                let retiring = if active {
                    state.acquiring = false;
                    state.retiring.take()
                } else {
                    None
                };
                (!active && !state.acquiring, retiring)
            };
            if active {
                release_other_keyboards(&d);
            }
            if let Some(retiring) = retiring {
                retiring.destroy();
            }
            if release {
                release_surface(&d);
            }
        });
    });
}

fn create_surface(
    app: &Rc<App>,
    monitor: gdk::Monitor,
    shared: Rc<RefCell<Folder>>,
    primary: bool,
) {
    let g = monitor.geometry();
    let window = desktop_window(&monitor, KeyboardMode::None);
    let paint = Paint::new(1, 1);
    paint.set_hexpand(true);
    paint.set_vexpand(true);
    paint.set_focusable(true);
    window.set_child(Some(&paint));
    let folder = glib::user_special_dir(glib::UserDirectory::Desktop)
        .unwrap_or_else(|| glib::home_dir().join("Desktop"));
    let output = monitor
        .connector()
        .map(|c| c.to_string())
        .unwrap_or_default();
    let desktop = Rc::new(RefCell::new(Desktop {
        shared,
        output,
        primary,
        keyboard: false,
        acquiring: false,
        pointer_down: false,
        last_click: None,
        retiring: None,
        theme: None,
        baseline: Positions::new(),
        incoming: None,
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
        importing: false,
        pending_imports: Positions::new(),
        ticking: false,
        closed: false,
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
    desktop.borrow_mut().theme = Some((theme, handler));
    DESKTOPS.with(|d| d.borrow_mut().push(desktop.clone()));
    watch_focus(&desktop, &window);
    window.present();
}

impl Desktop {
    fn visible(&self, item: &Item) -> bool {
        if self.shared.borrow().moved_out.contains(&item.data.name) {
            return false;
        }
        if let Some(drag) = &self.drag
            && drag.movement.origins.contains_key(&item.data.name)
        {
            return drag_items_visible(
                drag.system.is_some(),
                drag.local_preview,
                drag.over_desktop,
                drag.end,
            ) && (drag.end != DragEnd::Local
                || self
                    .positions
                    .get(&item.data.name)
                    .is_some_and(|p| p.page == self.page));
        }
        if self.shared.borrow().drag_names.contains(&item.data.name) {
            return false;
        }
        self.positions
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
        {
            let mut shared = self.shared.borrow_mut();
            for name in self.positions.keys() {
                if shared.outputs.get(name) == Some(&self.output) {
                    shared.saved.remove(name);
                }
            }
        }
        self.page = 0;
        self.layout(true);
    }
    fn select(&mut self, selected: BTreeSet<String>) {
        self.shared.borrow_mut().selected = selected.clone();
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
        self.grid.reserved = [None; 4];
        if let Some(app) = self.app.upgrade().filter(|app| self.primary && app.cfg.wallpaper) {
            let screen = (self.geometry.0, self.geometry.1);
            let battery = crate::batterywidget::card_rect(&app.prefs.borrow().desktop_battery, crate::batterywidget::SHOWN, screen, app.reserve(), crate::desk::room(&app), crate::dock::room(&self.output));
            let all = battery.into_iter().chain(crate::deskcards::rects(&app, &self.output, screen));
            for (slot, r) in self.grid.reserved.iter_mut().zip(all) {
                *slot = Some(Rect { x: r.x, y: r.y, w: r.w, h: r.h });
            }
        }
        let mut shared = self.shared.borrow_mut();
        self.positions = place_output(
            &names,
            &shared.saved,
            &shared.outputs,
            &shared.connected,
            &shared.primary,
            &self.output,
            self.grid,
        );
        let Folder { saved, outputs, .. } = &mut *shared;
        remember_output(&self.positions, saved, outputs, &self.output);
        self.saved = shared.saved.clone();
        self.baseline = self.saved.clone();
        drop(shared);
        for (name, &pos) in &self.pending_imports {
            self.saved.insert(name.clone(), pos);
        }
        self.page = self
            .page
            .min(self.positions.values().map(|p| p.page).max().unwrap_or(0));
        let now = glib::monotonic_time();
        for i in &mut self.items {
            let Some(&FreePos { x, y, .. }) = self.positions.get(&i.data.name) else {
                continue;
            };
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
    let shadow = !kit::effects::reduced();
    if shadow { s.push_shadow(&[gtk::gsk::Shadow::new(
        gdk::RGBA::new(0.0, 0.0, 0.0, 0.55 * (1.0 - selected as f32)),
        0.0,
        1.0,
        3.0,
    )]); }
    let ink = pal.on_accent.mix(Rgb::new(1.0, 1.0, 1.0), selected);
    s.append_layout(label, &ink.rgba());
    if shadow { s.pop(); }
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
        &Rgb::new(0.0, 0.0, 0.0).alpha(0.004).rgba(),
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
    let incoming = { d.borrow().incoming.clone() };
    if let Some(IncomingDrag {
        preview,
        pointer,
        hotspot: hot,
    }) = incoming
    {
        s.save();
        s.translate(&graphene::Point::new(
            (pointer.0 - hot.0 as f64) as f32,
            (pointer.1 - hot.1 as f64) as f32,
        ));
        preview.snapshot(
            s,
            preview.intrinsic_width() as f64,
            preview.intrinsic_height() as f64,
        );
        s.restore();
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
    sync_selection();
    animate_surface(d);
}
fn sync_selection() {
    let selected = FOLDER
        .with(|f| f.borrow().as_ref().map(|f| f.borrow().selected.clone()))
        .unwrap_or_default();
    for d in surfaces() {
        let changed = {
            let mut d = d.borrow_mut();
            if d.selected == selected {
                false
            } else {
                d.selected = selected.clone();
                let now = glib::monotonic_time();
                for i in &mut d.items {
                    i.selected.go(
                        f64::from(selected.contains(&i.data.name)),
                        now,
                        160,
                        motion::STANDARD,
                    );
                }
                true
            }
        };
        if selected.is_empty() {
            release_surface(&d);
        }
        if changed {
            animate_surface(&d);
        }
    }
}
fn animate_surface(d: &Rc<RefCell<Desktop>>) {
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
    let snapshot = {
        let mut d = d.borrow_mut();
        if d.closed {
            return;
        }
        let shared = d.shared.clone();
        let mut f = shared.borrow_mut();
        // Commit only this surface's edits, never another surface's cached copy.
        for name in d.baseline.keys().filter(|n| !d.saved.contains_key(*n)) {
            f.saved.remove(name);
            f.outputs.remove(name);
        }
        for (name, pos) in &d.saved {
            if d.baseline.get(name) != Some(pos) {
                f.saved.insert(name.clone(), *pos);
                f.outputs.insert(name.clone(), d.output.clone());
            }
        }
        d.baseline = d.saved.clone();
        (f.saved.clone(), f.outputs.clone())
    };
    TO_WRITE.with(|p| *p.borrow_mut() = Some(snapshot));
    if WRITING.replace(true) {
        return;
    }
    glib::spawn_future_local(async {
        while let Some((positions, outputs)) = TO_WRITE.with(|p| p.borrow_mut().take()) {
            let result =
                gio::spawn_blocking(move || write_state(&state_path(), &positions, &outputs)).await;
            if !matches!(result, Ok(Ok(()))) {
                eprintln!("digit: desktop icon positions could not be saved: {result:?}");
            }
        }
        WRITING.set(false);
    });
}

fn watch_folder(shared: &Rc<RefCell<Folder>>, folder: &Path) {
    if shared.borrow().monitor.is_some() {
        return;
    }
    let Ok(monitor) = gio::File::for_path(folder)
        .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
    else {
        return;
    };
    let weak = Rc::downgrade(shared);
    let folder = folder.to_owned();
    monitor.connect_changed(move |monitor, file, _, event| {
        let Some(shared) = weak.upgrade() else { return };
        let path = file.path();
        let cancel = path.as_ref() == Some(&folder)
            && matches!(
                event,
                gio::FileMonitorEvent::Deleted
                    | gio::FileMonitorEvent::MovedOut
                    | gio::FileMonitorEvent::Unmounted
            );
        if cancel {
            shared.borrow_mut().monitor.take();
            monitor.cancel();
        }
        if shared.borrow().pending.is_some() {
            return;
        }
        let weak = Rc::downgrade(&shared);
        let id = glib::timeout_add_local_once(Duration::from_millis(100), move || {
            if let Some(shared) = weak.upgrade() {
                shared.borrow_mut().pending.take();
                if let Some(d) = surfaces().first() {
                    refresh(d);
                }
            }
        });
        shared.borrow_mut().pending = Some(id);
    });
    shared.borrow_mut().monitor = Some(monitor);
}
/// The accent changed: folders are drawn in it.
pub fn restyle() {
    if let Some(d) = surfaces().first() {
        refresh(d);
    }
}

fn refresh(d: &Rc<RefCell<Desktop>>) {
    let (shared, folder) = {
        let d = d.borrow();
        (d.shared.clone(), d.folder.clone())
    };
    if surfaces().iter().any(|d| {
        let d = d.borrow();
        d.drag.is_some() || d.rubber.is_some() || d.importing
    }) {
        shared.borrow_mut().dirty = true;
        return;
    }
    if shared.borrow().loading {
        shared.borrow_mut().dirty = true;
        return;
    }
    let read_state = !shared.borrow().ready;
    shared.borrow_mut().loading = true;
    let weak = Rc::downgrade(&shared);
    glib::spawn_future_local(async move {
        // One directory scan per refresh, irrespective of the number of outputs.
        let result = gio::spawn_blocking(move || {
            let state = if read_state {
                decode_state(&std::fs::read(state_path()).unwrap_or_default()).unwrap_or_default()
            } else {
                Default::default()
            };
            (scan(&folder), state, folder)
        })
        .await;
        let Some(shared) = weak.upgrade() else { return };
        if !FOLDER.with(|f| f.borrow().as_ref().is_some_and(|f| Rc::ptr_eq(f, &shared))) {
            return;
        }
        // A drag/import may have started while the worker was scanning. Its
        // reservations and selection are newer than that snapshot.
        if surfaces().iter().any(|d| {
            let d = d.borrow();
            d.drag.is_some() || d.rubber.is_some() || d.importing
        }) {
            let mut f = shared.borrow_mut();
            f.loading = false;
            f.dirty = true;
            return;
        }
        if let Ok((Ok(files), (saved, outputs), folder)) = result {
            {
                let mut f = shared.borrow_mut();
                if !f.ready {
                    f.saved = saved;
                    f.outputs = outputs;
                    f.ready = true;
                }
                let names: BTreeSet<_> = files.iter().map(|f| f.name.clone()).collect();
                f.selected.retain(|n| names.contains(n));
                f.saved.retain(|n, _| names.contains(n));
                f.outputs.retain(|n, _| names.contains(n));
                f.files = files.clone();
            }
            let first = shared.borrow().monitor.is_none();
            watch_folder(&shared, &folder);
            for d in surfaces() {
                materialize(d, files.clone()).await;
            }
            // Close the scan/watch race just once when starting the folder watch.
            if first && shared.borrow().monitor.is_some() {
                shared.borrow_mut().dirty = true;
            }
        } else {
            eprintln!("digit: desktop folder could not be read");
        }
        let again = {
            let mut f = shared.borrow_mut();
            f.loading = false;
            std::mem::take(&mut f.dirty)
        };
        if again && let Some(d) = surfaces().first() {
            refresh(d);
        }
    });
}

async fn materialize(d: Rc<RefCell<Desktop>>, files: Vec<FileData>) {
    if d.borrow().closed {
        return;
    }
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
                    folders.insert(glyph, kit::folder::texture(&accent, glyph, ICON * scale)?);
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
            items.sort_by_key(|i| order.get(&i.data.name).copied().unwrap_or(usize::MAX));
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
            let present: BTreeSet<_> = items.iter().map(|i| i.data.name.clone()).collect();
            state
                .shared
                .borrow_mut()
                .moved_out
                .retain(|name| present.contains(name));
            state
                .pending_imports
                .retain(|name, _| !present.contains(name));
            let previous = std::mem::replace(&mut state.items, items);
            let names: BTreeSet<_> = state.items.iter().map(|i| i.data.name.clone()).collect();
            let selected = state
                .shared
                .borrow()
                .selected
                .intersection(&names)
                .cloned()
                .collect();
            state.select(selected);
            state.layout(false);
            previous
        } else {
            state.shared.borrow_mut().dirty = true;
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
/// Something the trash took from the desktop, and where it sat.
#[derive(Clone)]
struct Trashed {
    path: PathBuf,
    position: Option<FreePos>,
}

const UNDO_DEPTH: usize = 20;

thread_local! {
    static UNDO: RefCell<Vec<Vec<Trashed>>> = const { RefCell::new(Vec::new()) };
}

fn can_undo() -> bool {
    UNDO.with(|u| !u.borrow().is_empty())
}

fn trash(d: &Rc<RefCell<Desktop>>, paths: Vec<PathBuf>) {
    // Capture the drawn centres before the folder monitor removes trashed items.
    let flights: Vec<_> = {
        let state = d.borrow();
        let geometry = state.monitor.geometry();
        let now = glib::monotonic_time();
        paths.iter().filter_map(|path| {
            let item = state.items.iter().find(|i| &i.data.path == path)?;
            let at = (geometry.x() as f64 + item.x.value(now).round() + CELL_W as f64 / 2.0,
                geometry.y() as f64 + item.y.value(now).round() + ICON as f64 / 2.0);
            Some((path.clone(), at, item.data.icon.clone(), item.data.directory))
        }).collect()
    };
    let app = d.borrow().app.clone();
    let entries: Vec<Trashed> = {
        let state = d.borrow();
        paths
            .into_iter()
            .map(|path| {
                let position = path.file_name().and_then(|n| state.saved.get(&name_key(n)).copied());
                Trashed { path, position }
            })
            .collect()
    };
    glib::spawn_future_local(async move {
        let mut gone = Vec::new();
        for entry in entries {
            match gio::File::for_path(&entry.path)
                .trash_future(glib::Priority::DEFAULT)
                .await
            {
                Ok(()) => gone.push(entry),
                Err(e) => eprintln!("digit: desktop trash: {e}"),
            }
        }
        if let Some(app) = app.upgrade() {
            for entry in gone.iter().take(3) {
                let Some((_, at, icon, directory)) = flights.iter().find(|(p, _, _, _)| p == &entry.path) else { continue };
                // Folders fly as the accent folder the desktop draws for them.
                let folder = directory.then(|| crate::dock::folder_icon(kit::folder::glyph(&gio::File::for_path(&entry.path)), ICON as f64)).flatten();
                let icon = folder.or_else(|| icon.as_deref().and_then(|i| gio::Icon::for_string(i).ok())).unwrap_or_else(|| {
                    let content_type = if *directory { "inode/directory".into() } else { gio::content_type_guess(Some(&entry.path), None).0 };
                    gio::content_type_get_icon(&content_type)
                });
                crate::dock::fly_to_trash(&app, *at, ICON as f64, Some(icon));
            }
        }
        if !gone.is_empty() {
            UNDO.with(|u| {
                let mut undo = u.borrow_mut();
                undo.push(gone);
                let extra = undo.len().saturating_sub(UNDO_DEPTH);
                undo.drain(..extra);
            });
        }
    });
}

/// The newest copy in the trash of what was at `original` goes back there, or beside it if
/// something took that name since.
fn restore_from_trash(original: &Path) -> Result<PathBuf, String> {
    let trash = gio::File::for_uri("trash:///");
    let list = trash
        .enumerate_children("standard::name,trash::orig-path,trash::deletion-date", gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS, gio::Cancellable::NONE)
        .map_err(|e| e.to_string())?;
    let mut found: Option<(String, gio::File)> = None;
    while let Some(info) = list.next_file(gio::Cancellable::NONE).map_err(|e| e.to_string())? {
        if info.attribute_byte_string("trash::orig-path").is_some_and(|p| Path::new(p.as_str()) == original) {
            let when = info.attribute_string("trash::deletion-date").map(|d| d.to_string()).unwrap_or_default();
            if found.as_ref().is_none_or(|(newest, _)| when >= *newest) {
                found = Some((when, trash.child(info.name())));
            }
        }
    }
    let (_, file) = found.ok_or("It's no longer in the trash")?;
    let name = original.file_name().ok_or("Missing file name")?;
    let directory = file.query_file_type(gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS, gio::Cancellable::NONE) == gio::FileType::Directory;
    let free = collision_name(name, directory, |candidate| original.with_file_name(candidate).symlink_metadata().is_ok());
    let dest = original.with_file_name(free);
    file.move_(&gio::File::for_path(&dest), gio::FileCopyFlags::NONE, gio::Cancellable::NONE, None).map_err(|e| e.to_string())?;
    Ok(dest)
}

/// Ctrl+Z: what went to the trash last comes back to the place it had.
fn undo_trash(d: &Rc<RefCell<Desktop>>) {
    let Some(entries) = UNDO.with(|u| u.borrow_mut().pop()) else { return };
    {
        let mut state = d.borrow_mut();
        for entry in &entries {
            if let (Some(name), Some(pos)) = (entry.path.file_name().map(name_key), entry.position) {
                state.saved.insert(name.clone(), pos);
                state.positions.insert(name.clone(), pos);
                let output = state.output.clone();
                let mut shared = state.shared.borrow_mut();
                shared.saved.insert(name.clone(), pos);
                shared.outputs.insert(name, output);
            }
        }
    }
    save(d);
    let weak = Rc::downgrade(d);
    glib::spawn_future_local(async move {
        let paths: Vec<PathBuf> = entries.into_iter().map(|e| e.path).collect();
        let results = gio::spawn_blocking(move || paths.iter().map(|p| restore_from_trash(p)).collect::<Vec<_>>()).await;
        let Some(d) = weak.upgrade() else { return };
        for error in results.into_iter().flatten().filter_map(Result::err) {
            let app = d.borrow().app.upgrade();
            if let Some(app) = app {
                app.request(crate::app::Peek::Note { text: format!("Couldn't restore: {error}") });
            }
        }
        refresh(&d);
    });
}

const COPIED_FILES: &str = "x-special/gnome-copied-files";

/// The same clipboard formats as Files, so cut and copy work between the two.
fn clip(paint: &Paint, files: &[FileData], cut: bool) {
    if files.is_empty() {
        return;
    }
    let list: Vec<gio::File> = files.iter().map(|f| gio::File::for_path(&f.path)).collect();
    let uris: Vec<String> = list.iter().map(|f| f.uri().to_string()).collect();
    let gnome = format!("{}\n{}", if cut { "cut" } else { "copy" }, uris.join("\n"));
    let text = files.iter().map(|f| f.path.to_string_lossy().into_owned()).collect::<Vec<_>>().join("\n");
    let provider = gdk::ContentProvider::new_union(&[
        gdk::ContentProvider::for_value(&gdk::FileList::from_array(&list).to_value()),
        gdk::ContentProvider::for_bytes(COPIED_FILES, &glib::Bytes::from_owned(gnome.into_bytes())),
        gdk::ContentProvider::for_bytes("text/uri-list", &glib::Bytes::from_owned(format!("{}\r\n", uris.join("\r\n")).into_bytes())),
        gdk::ContentProvider::for_bytes("text/plain;charset=utf-8", &glib::Bytes::from_owned(text.into_bytes())),
    ]);
    let _ = paint.clipboard().set_content(Some(&provider));
}

struct ClipboardWatch {
    clipboard: gdk::Clipboard,
    changed: Rc<Cell<bool>>,
    handler: Option<glib::SignalHandlerId>,
}

impl ClipboardWatch {
    fn new(clipboard: &gdk::Clipboard) -> Self {
        let changed = Rc::new(Cell::new(false));
        let signal = changed.clone();
        let handler = clipboard.connect_changed(move |_| signal.set(true));
        Self { clipboard: clipboard.clone(), changed, handler: Some(handler) }
    }
}

impl Drop for ClipboardWatch {
    fn drop(&mut self) {
        if let Some(handler) = self.handler.take() { self.clipboard.disconnect(handler); }
    }
}

/// Files from the clipboard land at `pointer`: cut ones move here, copied ones are copied.
fn paste(d: &Rc<RefCell<Desktop>>, pointer: (f64, f64)) {
    let paint = d.borrow().paint.clone();
    let clipboard = paint.clipboard();
    let weak = Rc::downgrade(d);
    glib::spawn_future_local(async move {
        let watch = ClipboardWatch::new(&clipboard);
        let formats = clipboard.formats();
        let mut found: Option<(Vec<String>, bool)> = None;
        if formats.contain_mime_type(COPIED_FILES)
            && let Ok((stream, _)) = clipboard.read_future(&[COPIED_FILES], glib::Priority::DEFAULT).await
        {
            let out = gio::MemoryOutputStream::new_resizable();
            let flags = gio::OutputStreamSpliceFlags::CLOSE_SOURCE | gio::OutputStreamSpliceFlags::CLOSE_TARGET;
            if out.splice_future(&stream, flags, glib::Priority::DEFAULT).await.is_ok() {
                let bytes = out.steal_as_bytes();
                let text = String::from_utf8_lossy(&bytes).to_string();
                let mut lines = text.lines();
                let cut = lines.next() == Some("cut");
                found = Some((lines.filter(|l| !l.is_empty()).map(str::to_string).collect(), cut));
            }
        }
        if found.as_ref().is_none_or(|(uris, _)| uris.is_empty())
            && formats.contains_type(gdk::FileList::static_type())
            && let Ok(value) = clipboard.read_value_future(gdk::FileList::static_type(), glib::Priority::DEFAULT).await
            && let Ok(list) = value.get::<gdk::FileList>()
        {
            found = Some((list.files().iter().map(|f| f.uri().to_string()).collect(), false));
        }
        if let (Some((uris, cut)), Some(d)) = (found, weak.upgrade())
            && !uris.is_empty()
        {
            import_drop(&d, uris, pointer, !cut, cut, move |success| {
                let unchanged = !watch.changed.get();
                drop(watch);
                if cut && success && unchanged {
                    let _ = clipboard.set_content(None::<&gdk::ContentProvider>);
                }
            });
        }
    });
}

fn select_all(d: &Rc<RefCell<Desktop>>) {
    let selected = surfaces()
        .iter()
        .flat_map(|d| {
            let d = d.borrow();
            d.items.iter().filter(|i| d.visible(i)).map(|i| i.data.name.clone()).collect::<Vec<_>>()
        })
        .collect();
    d.borrow_mut().select(selected);
    animate(d);
}

/// Where keyboard paste lands: the middle of this screen.
fn screen_middle(d: &Rc<RefCell<Desktop>>) -> (f64, f64) {
    let state = d.borrow();
    (state.grid.width as f64 / 2.0, state.geometry.1 as f64 / 2.0)
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
    release_keyboard();
    // F2 can arrive on another selected output. The editor follows the item.
    let owner = {
        let state = d.borrow();
        if state.selected.len() != 1 {
            return;
        }
        state
            .selected
            .iter()
            .next()
            .filter(|n| !state.positions.contains_key(*n))
            .cloned()
    };
    if let Some(name) = owner {
        if let Some(target) = surfaces()
            .into_iter()
            .find(|d| d.borrow().positions.contains_key(&name))
        {
            rename(&target);
        }
        return;
    }
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
    let y = (r.y + (ICON + 5) as f64)
        .clamp(0.0, (geometry.height() - 24).max(0) as f64)
        .round();
    let screen = geometry.width();
    let place = {
        let (fixed, editor) = (fixed.downgrade(), editor.downgrade());
        move |entry: &gtk::Entry| {
            let (Some(fixed), Some(editor)) = (fixed.upgrade(), editor.upgrade()) else {
                return;
            };
            let text = entry.text();
            let width = (entry.create_pango_layout(Some(&text)).pixel_size().0 + 20)
                .clamp(44, 260)
                .min(screen.max(1));
            entry.set_size_request(width, -1);
            let x = (r.x + CELL_W as f64 / 2.0 - width as f64 / 2.0)
                .clamp(0.0, (screen - width).max(0) as f64)
                .round();
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
        // A name already in use gets a number, the way New Folder and drops do: "test (2)".
        let name = collision_name(std::ffi::OsStr::new(&name), directory, |candidate| {
            path.with_file_name(candidate).symlink_metadata().is_ok()
        })
        .to_string_lossy()
        .into_owned();
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
                        state.shared.borrow().loading && !state.closed
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
    Cut,
    Copy,
    CopyPath,
    Rename,
    Duplicate,
    Trash,
    Undo,
    Paste,
    SelectAll,
}
impl MenuAction {
    fn label(self) -> &'static str {
        match self {
            Self::NewFolder => "New Folder…",
            Self::Wallpaper => "Change Wallpaper",
            Self::Terminal => "Open Terminal Here",
            Self::Sort => "Sort by Name",
            Self::OpenFolder => "Open Desktop Folder",
            Self::Separator => "",
            Self::DesktopSettings => "Desktop Settings",
            Self::DisplaySettings => "Display Settings",
            Self::Open => "Open",
            Self::Reveal => "Show in Files",
            Self::Cut => "Cut",
            Self::Copy => "Copy",
            Self::CopyPath => "Copy Path",
            Self::Rename => "Rename…",
            Self::Duplicate => "Duplicate",
            Self::Trash => "Move to Trash",
            Self::Undo => "Undo",
            Self::Paste => "Paste",
            Self::SelectAll => "Select All",
        }
    }

    /// The same glyphs Files uses for these.
    fn glyph(self) -> &'static str {
        match self {
            Self::NewFolder => "create_new_folder",
            Self::Wallpaper => "wallpaper",
            Self::Terminal => "terminal",
            Self::Sort => "sort",
            Self::OpenFolder => "folder_open",
            Self::Separator => "",
            Self::DesktopSettings => "settings",
            Self::DisplaySettings => "monitor",
            Self::Open => "open_in_new",
            Self::Reveal => "folder_open",
            Self::Cut => "content_cut",
            Self::Copy => "content_copy",
            Self::CopyPath => "link",
            Self::Rename => "edit",
            Self::Duplicate => "library_add",
            Self::Trash => "delete",
            Self::Undo => "undo",
            Self::Paste => "content_paste",
            Self::SelectAll => "select_all",
        }
    }

    fn keys(self) -> &'static str {
        match self {
            Self::Cut => "Ctrl+X",
            Self::Copy => "Ctrl+C",
            Self::Paste => "Ctrl+V",
            Self::SelectAll => "Ctrl+A",
            Self::Undo => "Ctrl+Z",
            Self::Rename => "F2",
            Self::Trash => "Delete",
            Self::Open => "Enter",
            _ => "",
        }
    }
}
fn menu_entries(on_icon: bool, selected: usize, undo: bool) -> Vec<MenuAction> {
    use MenuAction::*;
    if !on_icon {
        let mut rows = vec![NewFolder, Paste, SelectAll];
        if undo {
            rows.push(Undo);
        }
        rows.extend([Separator, Terminal, Sort, OpenFolder, Separator, Wallpaper, DesktopSettings, DisplaySettings]);
        rows
    } else if selected > 1 {
        vec![Open, Separator, Cut, Copy, Duplicate, Separator, Trash]
    } else {
        vec![Open, Reveal, Separator, Cut, Copy, CopyPath, Separator, Rename, Duplicate, Separator, Trash]
    }
}
fn menu(d: &Rc<RefCell<Desktop>>, on_icon: bool, x: f64, y: f64) {
    release_keyboard();
    let (app, monitor, folder, entries, files, clipboard) = {
        let Ok(state) = d.try_borrow() else { return };
        let Some(app) = state.app.upgrade() else {
            return;
        };
        (
            app,
            state.monitor.clone(),
            state.folder.clone(),
            menu_entries(on_icon, state.selected.len(), can_undo()),
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
                MenuAction::Cut | MenuAction::Copy => clip(&paint, &files, action == MenuAction::Cut),
                MenuAction::Paste => {
                    if let Some(d) = weak.upgrade() {
                        paste(&d, (x, y));
                    }
                }
                MenuAction::Duplicate => {
                    if let Some(d) = weak.upgrade() {
                        let uris = files.iter().map(|f| gio::File::for_path(&f.path).uri().to_string()).collect();
                        import_drop(&d, uris, (x + 24.0, y + 24.0), true, false, |_| {});
                    }
                }
                MenuAction::SelectAll => {
                    if let Some(d) = weak.upgrade() {
                        select_all(&d);
                    }
                }
                MenuAction::Undo => {
                    if let Some(d) = weak.upgrade() {
                        undo_trash(&d);
                    }
                }
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
                                        if let Some(name) = state.rename_pending.clone() {
                                            let pos =
                                                drop_cascade(state.grid, (x, y), state.page, 1)[0];
                                            state.saved.insert(name, pos);
                                        }
                                    }
                                    save(&d);
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
                MenuAction::Trash => {
                    if let Some(d) = weak.upgrade() {
                        trash(&d, files.iter().map(|f| f.path.clone()).collect());
                    }
                }
                MenuAction::Separator => {}
            });
            let row = row.glyph(action.glyph()).hint(action.keys());
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
        d.borrow_mut().pointer_down = button == 1;
        let ctrl = g
            .current_event_state()
            .contains(gdk::ModifierType::CONTROL_MASK);
        let (interval, distance) = gtk::Settings::default()
            .map(|s| {
                (
                    s.gtk_double_click_time() as i64 * 1000,
                    s.gtk_double_click_distance() as f64,
                )
            })
            .unwrap_or((400_000, 5.0));
        let now = glib::monotonic_time();
        let (on_icon, open_file) = {
            let Ok(mut state) = d.try_borrow_mut() else {
                return;
            };
            let at = state.hit(x, y);
            let double = button == 1
                && !ctrl
                && at.is_some_and(|i| {
                    n == 2
                        || state.last_click.as_ref().is_some_and(|last| {
                            last.follows(&state.items[i].data.name, now, (x, y), interval, distance)
                        })
                });
            state.last_click = if button == 1 && !ctrl && !double {
                at.map(|i| ClickRecord {
                    name: state.items[i].data.name.clone(),
                    time: now,
                    point: (x, y),
                })
            } else {
                None
            };
            let mut selected = state.selected.clone();
            match at {
                Some(i) => {
                    let name = state.items[i].data.name.clone();
                    if ctrl && button == 1 {
                        if !selected.remove(&name) {
                            selected.insert(name);
                        }
                    } else if !selected.contains(&name) || double {
                        selected.clear();
                        selected.insert(name);
                    }
                }
                None => selected.clear(),
            }
            state.select(selected);
            let open_file = double
                .then(|| at.map(|i| state.items[i].data.clone()))
                .flatten();
            if let Some(at) = at {
                state.raise(at);
            }
            (at.is_some(), open_file)
        };
        animate(&d);
        focus_selection(&d);
        if button == 3 {
            g.set_state(gtk::EventSequenceState::Claimed);
            menu(&d, on_icon, x, y);
        } else if let Some(file) = open_file {
            release_keyboard();
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
            d.borrow_mut().pointer_down = false;
            cancel_pending_drag(&d);
            acquire_after_release(&d);
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
        focus_selection(&d);
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
            let preview = (drag.preview.clone(), drag.hotspot);
            state.last_click = None;
            preview
        };
        // The shared helper also repairs Hyprland's offset after GPU attaches;
        // merely setting GDK's hotspot before the frame cannot survive those.
        let mut picture = Some(kit::drag::set_icon(system, &preview, hotspot));
        let previous = {
            let mut state = d.borrow_mut();
            state
                .drag
                .as_mut()
                .and_then(|drag| std::mem::replace(&mut drag.visual, picture.take()))
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
            && let Some(source) = own_source(target)
        {
            cross_motion(&d, &source, x, y);
            return gdk::DragAction::MOVE;
        }
        gdk::DragAction::COPY
    });
    let weak = Rc::downgrade(d);
    target.connect_motion(move |target, x, y| {
        let Some(d) = weak.upgrade() else {
            return gdk::DragAction::empty();
        };
        if let Some(source) = own_source(target) {
            cross_motion(&d, &source, x, y);
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
            leave_desktop(&d);
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
        if let Some(source) = own_source(target) {
            finish_cross_drag(&d, &source, x, y);
            return true;
        }
        let uris: Vec<_> = list.files().iter().map(|f| f.uri().to_string()).collect();
        if uris.is_empty() {
            return false;
        }
        import_drop(&d, uris, (x, y), false, false, |_| {});
        true
    });
    paint.add_controller(target);

    let keys = gtk::EventControllerKey::new();
    let weak = Rc::downgrade(d);
    keys.connect_key_pressed(move |_, key, _, mods| {
        let Some(d) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        if !d.borrow().keyboard {
            return glib::Propagation::Proceed;
        }
        match key {
            gdk::Key::Return | gdk::Key::KP_Enter => {
                let files = selected_files(&d);
                release_keyboard();
                files.iter().for_each(open);
            }
            gdk::Key::Delete | gdk::Key::KP_Delete => {
                trash(&d, selected_files(&d).into_iter().map(|f| f.path).collect());
            }
            gdk::Key::F2 => rename(&d),
            gdk::Key::z | gdk::Key::Z if mods.contains(gdk::ModifierType::CONTROL_MASK) => undo_trash(&d),
            gdk::Key::c | gdk::Key::C if mods.contains(gdk::ModifierType::CONTROL_MASK) => {
                let paint = d.borrow().paint.clone();
                clip(&paint, &selected_files(&d), false);
            }
            gdk::Key::x | gdk::Key::X if mods.contains(gdk::ModifierType::CONTROL_MASK) => {
                let paint = d.borrow().paint.clone();
                clip(&paint, &selected_files(&d), true);
            }
            gdk::Key::v | gdk::Key::V if mods.contains(gdk::ModifierType::CONTROL_MASK) => {
                let middle = screen_middle(&d);
                paste(&d, middle);
            }
            gdk::Key::Escape => {
                cancel_drag(&d);
                {
                    let mut state = d.borrow_mut();
                    state.rubber = None;
                    state.select(BTreeSet::new());
                }
                animate(&d);
            }
            gdk::Key::a | gdk::Key::A if mods.contains(gdk::ModifierType::CONTROL_MASK) => select_all(&d),
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    paint.add_controller(keys);

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
        let positions = drag.movement.at(drag.pointer.0, drag.pointer.1);
        let frames: Vec<_> = state
            .items
            .iter()
            .filter(|i| drag.movement.origins.contains_key(&i.data.name))
            .map(|i| ItemFrame {
                icon: i.icon.clone(),
                label: i.label.clone(),
                rect: Rect {
                    x: positions[&i.data.name].0,
                    y: positions[&i.data.name].1,
                    w: CELL_W as f64,
                    h: CELL_H as f64,
                },
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
    let monitor = { d.borrow().monitor.clone() };
    let source_geometry = monitor.geometry();
    let mut all_origins = BTreeMap::new();
    let now = glib::monotonic_time();
    for surface in surfaces() {
        let monitor = { surface.borrow().monitor.clone() };
        let geometry = monitor.geometry();
        let surface = surface.borrow();
        for item in surface.items.iter().filter(|i| surface.visible(i)) {
            all_origins.insert(
                item.data.name.clone(),
                (
                    item.x.value(now) + (geometry.x() - source_geometry.x()) as f64,
                    item.y.value(now) + (geometry.y() - source_geometry.y()) as f64,
                ),
            );
        }
    }
    let (movement, anchor, paths, fallback) = {
        let state = d.borrow();
        if state.drag.is_some() || state.closed {
            return;
        }
        let Some(at) = state.hit(x, y) else { return };
        let item = &state.items[at];
        let anchor = item.data.name.clone();
        let selected = drag_selection(&state.selected, &anchor);
        let movement = Drag {
            start: (x, y),
            origins: all_origins
                .into_iter()
                .filter(|(name, _)| selected.contains(name))
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
        target_output: None,
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
        {
            let mut shared = state.shared.borrow_mut();
            shared.drag_names = drag.movement.origins.keys().cloned().collect();
        }
        state.drag = Some(drag);
    }
    for surface in surfaces() {
        animate(&surface);
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

fn own_source(target: &gtk::DropTarget) -> Option<Rc<RefCell<Desktop>>> {
    let system = target.current_drop()?.drag()?;
    surfaces().into_iter().find(|d| {
        d.borrow()
            .drag
            .as_ref()
            .and_then(|drag| drag.system.as_ref())
            == Some(&system)
    })
}

// A frozen group preview moves between output-local coordinate spaces. The
// source owns the GDK drag until its terminal event; the target never imports
// its URIs or creates a second drag. Only one surface draws the preview.
fn cross_motion(target: &Rc<RefCell<Desktop>>, source: &Rc<RefCell<Desktop>>, x: f64, y: f64) {
    let output = { target.borrow().output.clone() };
    let changed = {
        let mut state = source.borrow_mut();
        let Some(drag) = state.drag.as_mut() else {
            return;
        };
        let changed = drag.target_output.as_ref() != Some(&output);
        drag.target_output = Some(output);
        changed
    };
    if !changed {
        if Rc::ptr_eq(target, source) {
            move_drag(source, x, y);
        } else {
            let paint = {
                let mut state = target.borrow_mut();
                if let Some(incoming) = &mut state.incoming {
                    incoming.pointer = (x, y);
                }
                state.paint.clone()
            };
            paint.queue_draw();
        }
        return;
    }
    for surface in surfaces() {
        let (paint, incoming) = {
            let mut state = surface.borrow_mut();
            (state.paint.clone(), state.incoming.take())
        };
        drop(incoming);
        paint.queue_draw();
    }
    if Rc::ptr_eq(target, source) {
        desktop_drag_target(source, true);
        // niri uses the native icon over the source, including on reentry after
        // another output temporarily owned the local preview.
        let native = {
            let state = source.borrow();
            state
                .drag
                .as_ref()
                .filter(|d| !d.local_preview)
                .and_then(|d| Some((d.visual.clone()?, d.preview.clone())))
        };
        if let Some((visual, preview)) = native {
            visual.set_paintable(Some(&preview));
        }
        move_drag(source, x, y);
        return;
    }
    desktop_drag_target(source, false);
    let (preview, hot, visual, paint) = {
        let mut state = source.borrow_mut();
        let Some(drag) = state.drag.as_mut() else {
            return;
        };
        drag.over_desktop = false;
        (
            drag.preview.clone(),
            drag.hotspot,
            drag.visual.clone(),
            state.paint.clone(),
        )
    };
    let target_paint = {
        let mut state = target.borrow_mut();
        state.incoming = Some(IncomingDrag {
            preview,
            pointer: (x, y),
            hotspot: hot,
        });
        state.paint.clone()
    };
    if let Some(visual) = visual {
        hide_native_visual(&visual);
    }
    paint.queue_draw();
    target_paint.queue_draw();
}
fn hide_native_visual(visual: &gtk::Picture) {
    let empty = gdk::MemoryTexture::new(
        1,
        1,
        gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from_static(&[0, 0, 0, 0]),
        4,
    );
    visual.set_paintable(Some(&empty));
}
fn leave_desktop(d: &Rc<RefCell<Desktop>>) {
    let output = { d.borrow().output.clone() };
    let (paint, incoming) = {
        let mut state = d.borrow_mut();
        (state.paint.clone(), state.incoming.take())
    };
    drop(incoming);
    paint.queue_draw();
    for source in surfaces() {
        let leaving = {
            let mut state = source.borrow_mut();
            state.drag.as_mut().is_some_and(|drag| {
                if drag.target_output.as_ref() != Some(&output) {
                    return false;
                }
                drag.target_output = None;
                true
            })
        };
        if !leaving {
            continue;
        }
        desktop_drag_target(&source, false);
        let visual = {
            let state = source.borrow();
            state
                .drag
                .as_ref()
                .filter(|drag| drag.end != DragEnd::Local)
                .and_then(|drag| Some((drag.visual.clone()?, drag.preview.clone())))
        };
        if let Some((visual, preview)) = visual {
            visual.set_paintable(Some(&preview));
        }
    }
}
fn clear_drag_surfaces(d: &Rc<RefCell<Desktop>>) {
    let shared = { d.borrow().shared.clone() };
    shared.borrow_mut().drag_names.clear();
    for surface in surfaces() {
        let (paint, incoming) = {
            let mut s = surface.borrow_mut();
            (s.paint.clone(), s.incoming.take())
        };
        drop(incoming);
        paint.queue_draw();
    }
}
fn finish_cross_drag(target: &Rc<RefCell<Desktop>>, source: &Rc<RefCell<Desktop>>, x: f64, y: f64) {
    if Rc::ptr_eq(target, source) {
        finish_drag(source, x, y);
        return;
    }
    let (grid, page, output, shared) = {
        let t = target.borrow();
        (t.grid, t.page, t.output.clone(), t.shared.clone())
    };
    let dropped = {
        let mut state = source.borrow_mut();
        let Some(drag) = state.drag.as_mut() else {
            return;
        };
        let dropped = drag.movement.dropped(grid, x, y, page);
        drag.end.event(DragEvent::LocalDrop);
        dropped
    };
    {
        let mut f = shared.borrow_mut();
        for (name, pos) in &dropped {
            f.saved.insert(name.clone(), *pos);
            f.outputs.insert(name.clone(), output.clone());
        }
    }
    source.borrow_mut().pointer_down = false;
    target.borrow_mut().pointer_down = false;
    // Relayout all outputs before queuing GTK drawing, so ownership changes once.
    for surface in surfaces() {
        surface.borrow_mut().layout(false);
    }
    clear_drag_surfaces(source);
    for surface in surfaces() {
        animate(&surface);
    }
    focus_selection(target);
    acquire_after_release(target);
    save(target);
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
        let Some(drag) = state.drag.as_mut() else {
            return;
        };
        if !drag.local_preview
            || drag.system.is_none()
            || drag.end == DragEnd::Local
            || drag.over_desktop == over
        {
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
            let empty = gdk::MemoryTexture::new(
                1,
                1,
                gdk::MemoryFormat::R8g8b8a8,
                &glib::Bytes::from_static(&[0, 0, 0, 0]),
                4,
            );
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
        state.shared.borrow().dirty
    };
    animate(d);
    acquire_after_release(d);
    if dirty {
        refresh(d);
    }
}
fn cancel_drag(d: &Rc<RefCell<Desktop>>) {
    let Ok(mut state) = d.try_borrow_mut() else {
        return;
    };
    let Some(drag) = state.drag.take() else {
        return;
    };
    restore_drag_items(&mut state.items, &drag.movement.restored());
    let selected = state.selected.clone();
    state.select(selected);
    let refresh_needed = state.shared.borrow().dirty;
    drop(state);
    // Widgets/paintables may finalize with synchronous callbacks too.
    drop(drag);
    clear_drag_surfaces(d);
    for surface in surfaces() {
        if !Rc::ptr_eq(&surface, d) {
            surface.borrow_mut().layout(false);
            animate(&surface);
        }
    }
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
        state.pointer_down = false;
        match result {
            DragResult::Restore => restore_drag_items(&mut state.items, &drag.movement.restored()),
            DragResult::AwaitRemoval => {
                state
                    .shared
                    .borrow_mut()
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
    clear_drag_surfaces(d);
    animate(d);
    for surface in surfaces() {
        acquire_after_release(&surface);
    }
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
    {
        let mut shared = state.shared.borrow_mut();
        for (name, pos) in &dropped {
            shared.saved.insert(name.clone(), *pos);
            shared.outputs.insert(name.clone(), state.output.clone());
        }
    }
    state.saved.extend(dropped.clone());
    state.positions.extend(dropped);
    let refresh_needed = state.shared.borrow().dirty;
    drop(state);
    d.borrow_mut().pointer_down = false;
    for surface in surfaces() {
        surface.borrow_mut().layout(false);
    }
    clear_drag_surfaces(d);
    for surface in surfaces() {
        animate(&surface);
    }
    focus_selection(d);
    acquire_after_release(d);
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
    cut: bool,
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
    copy: bool,
    cut: bool,
) -> Vec<ImportPlan> {
    uris.into_iter()
        .filter_map(|uri| {
            let file = gio::File::for_uri(&uri);
            let name = file.basename()?;
            let original = folder.join(&name);
            let dest = if !copy && file.equal(&gio::File::for_path(&original)) {
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
            Some(ImportPlan { uri, dest, cut })
        })
        .collect()
}

fn import_drop(d: &Rc<RefCell<Desktop>>, uris: Vec<String>, pointer: (f64, f64), copy: bool, cut: bool, done: impl FnOnce(bool) + 'static) {
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
                if state.shared.borrow().importing {
                    false
                } else {
                    state.shared.borrow_mut().importing = true;
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
            (
                state.folder.clone(),
                state.shared.borrow().saved.keys().cloned().collect(),
            )
        };
        let count = uris.len();
        let plans = gio::spawn_blocking(move || plan_imports(uris, &folder, reserved, copy, cut)).await;
        let Ok(plans) = plans else {
            {
                let mut state = d.borrow_mut();
                state.importing = false;
                state.shared.borrow_mut().importing = false;
            }
            refresh(&d);
            done(false);
            return;
        };
        let complete = plans.len() == count;
        let positions = drop_cascade(grid, pointer, page, plans.len());
        let previous: Vec<_> = {
            let mut state = d.borrow_mut();
            plans
                .iter()
                .zip(positions)
                .map(|(plan, pos)| {
                    let key = name_key(plan.dest.file_name().unwrap());
                    let previous = state.saved.insert(key.clone(), pos);
                    {
                        let mut shared = state.shared.borrow_mut();
                        shared.saved.insert(key.clone(), pos);
                        shared.outputs.insert(key.clone(), state.output.clone());
                    }
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
                .map(|plan| import(&gio::File::for_uri(&plan.uri), &plan.dest, copy, plan.cut))
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
            state.shared.borrow_mut().importing = false;
        }
        save(&d);
        refresh(&d);
        done(complete && imports_succeeded(&results));
    });
}

fn imports_succeeded<E>(results: &Result<Vec<Result<(), String>>, E>) -> bool {
    matches!(results, Ok(items) if items.iter().all(Result::is_ok))
}

fn import(file: &gio::File, dest: &Path, copy: bool, cut: bool) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    import_with_devices(file, dest, copy, cut, |src, folder| {
        Ok(std::fs::symlink_metadata(src)?.dev() == std::fs::metadata(folder)?.dev())
    })
}

fn import_with_devices(file: &gio::File, dest: &Path, copy: bool, cut: bool, same_device: impl FnOnce(&Path, &Path) -> std::io::Result<bool>) -> Result<(), String> {
    let folder = dest.parent().ok_or("Missing destination folder")?;
    let target = gio::File::for_path(dest);
    if file.equal(&target) {
        return Ok(());
    }
    if copy {
        return copy_import(file, dest, false);
    }
    if let Some(path) = file.path() {
        // Compare devices without following the source symlink; a cross-device drop is a
        // copy, including folders. A clipboard cut explicitly removes the source afterward.
        if !same_device(&path, folder).map_err(|e| e.to_string())? {
            return copy_import(file, dest, cut);
        }
    }
    match file.move_(&target, gio::FileCopyFlags::NO_FALLBACK_FOR_MOVE, gio::Cancellable::NONE, None) {
        Ok(()) => Ok(()),
        Err(e) if e.matches(gio::IOErrorEnum::NotSupported) => copy_import(file, dest, cut),
        Err(e) => Err(e.to_string()),
    }
}

fn copy_import(file: &gio::File, dest: &Path, cut: bool) -> Result<(), String> {
    let target = gio::File::for_path(dest);
    match file.path() {
        Some(path) => {
            let copied = copy_tree(&path, dest).map_err(|e| e.to_string())?;
            if cut {
                remove_import_source(&copied).map_err(|e| e.to_string())?;
            }
        }
        None => {
            let flags = if cut { gio::FileCopyFlags::NOFOLLOW_SYMLINKS } else { gio::FileCopyFlags::NONE };
            file.copy(&target, flags, gio::Cancellable::NONE, None).map_err(|e| e.to_string())?;
            if cut {
                file.delete(gio::Cancellable::NONE).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

struct CopiedSource {
    path: PathBuf,
    meta: std::fs::Metadata,
}

fn remove_import_source(copied: &[CopiedSource]) -> std::io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let mut error = None;
    // Children precede their directories; newly created entries keep a directory nonempty.
    for entry in copied.iter().rev() {
        let result = (|| {
            let meta = std::fs::symlink_metadata(&entry.path)?;
            let old = &entry.meta;
            if meta.dev() != old.dev() || meta.ino() != old.ino() || meta.file_type() != old.file_type()
                || (!meta.is_dir() && (meta.len() != old.len() || meta.modified()? != old.modified()?
                    || meta.ctime() != old.ctime() || meta.ctime_nsec() != old.ctime_nsec()))
            {
                return Err(std::io::Error::other("Source changed during copy"));
            }
            if meta.is_dir() { std::fs::remove_dir(&entry.path) } else { std::fs::remove_file(&entry.path) }
        })();
        if let Err(e) = result { error.get_or_insert(e); }
    }
    error.map_or(Ok(()), Err)
}

fn copy_tree(src: &Path, dest: &Path) -> std::io::Result<Vec<CopiedSource>> {
    if std::fs::symlink_metadata(src)?.is_dir() {
        let source = std::fs::canonicalize(src)?;
        let parent = std::fs::canonicalize(dest.parent().ok_or_else(|| std::io::Error::other("Missing destination folder"))?)?;
        let target = parent.join(dest.file_name().ok_or_else(|| std::io::Error::other("Missing destination name"))?);
        if target.starts_with(&source) {
            return Err(std::io::Error::other("Cannot copy a folder into itself"));
        }
    }
    let mut copied = Vec::new();
    copy_tree_entries(src, dest, &mut copied)?;
    Ok(copied)
}

fn copy_tree_entries(src: &Path, dest: &Path, copied: &mut Vec<CopiedSource>) -> std::io::Result<()> {
    use std::io;
    let meta = std::fs::symlink_metadata(src)?;
    copied.push(CopiedSource { path: src.to_path_buf(), meta: meta.clone() });
    if meta.is_symlink() {
        std::os::unix::fs::symlink(std::fs::read_link(src)?, dest)
    } else if meta.is_dir() {
        std::fs::create_dir(dest)?;
        let result = (|| {
            for entry in std::fs::read_dir(src)? {
                let entry = entry?;
                copy_tree_entries(&entry.path(), &dest.join(entry.file_name()), copied)?;
            }
            std::fs::set_permissions(dest, meta.permissions())
        })();
        if result.is_err() { let _ = std::fs::remove_dir_all(dest); }
        result
    } else if meta.is_file() {
        let mut input = std::fs::File::open(src)?;
        // Track the opened file, so replacing the source path cannot delete the replacement.
        copied.last_mut().unwrap().meta = input.metadata()?;
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dest)?;
        let result = io::copy(&mut input, &mut output).and_then(|_| output.set_permissions(meta.permissions()));
        if result.is_err() { let _ = std::fs::remove_file(dest); }
        result
    } else {
        Err(io::Error::other("Unsupported file type"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn double_click_survives_layer_replacement_and_rejects_other_icons_or_distant_presses() {
        let press = ClickRecord {
            name: "file".into(),
            time: 1_000_000,
            point: (100.5, 200.25),
        };
        assert!(press.follows("file", 1_200_000, (102.0, 201.0), 400_000, 5.0));
        assert!(!press.follows("other", 1_200_000, (102.0, 201.0), 400_000, 5.0));
        assert!(!press.follows("file", 1_500_000, (102.0, 201.0), 400_000, 5.0));
        assert!(!press.follows("file", 1_200_000, (110.0, 201.0), 400_000, 5.0));
    }

    #[test]
    fn focus_correction_targets_new_window_without_overriding_a_newer_focus() {
        assert!(needs_focus_correction(20, Some(10), Some(10)));
        assert!(needs_focus_correction(20, Some(10), None));
        assert!(!needs_focus_correction(20, Some(10), Some(20)));
        assert!(!needs_focus_correction(20, Some(10), Some(30)));
    }

    #[test]
    fn keyboard_is_requested_only_by_selection_and_revoked_by_windows_or_overlays() {
        for selected in [false, true] {
            assert_eq!(
                keyboard_requested(selected, KeyboardEvent::Select),
                selected
            );
            for event in [
                KeyboardEvent::Clear,
                KeyboardEvent::Window,
                KeyboardEvent::Overlay,
            ] {
                assert!(!keyboard_requested(selected, event));
            }
        }
        // Visible selection is not permission to reacquire after a window event.
        let selected = true;
        assert!(!keyboard_requested(selected, KeyboardEvent::Window));
        assert!(keyboard_requested(selected, KeyboardEvent::Select));
    }

    #[test]
    fn connected_connector_owns_an_item_and_missing_or_legacy_items_use_primary() {
        let connected = vec!["DP-1".into(), "eDP-1".into()];
        assert_eq!(item_output(Some("DP-1"), &connected, "eDP-1"), "DP-1");
        assert_eq!(item_output(Some("HDMI-A-1"), &connected, "eDP-1"), "eDP-1");
        assert_eq!(item_output(None, &connected, "eDP-1"), "eDP-1");
    }

    #[test]
    fn outputs_place_independently_and_new_items_belong_to_primary() {
        let names = vec!["left".into(), "right".into(), "new".into()];
        let connected = vec!["DP-1".into(), "eDP-1".into()];
        let mut outputs = BTreeMap::from([
            ("left".into(), "DP-1".into()),
            ("right".into(), "eDP-1".into()),
        ]);
        let mut saved = Positions::new();
        let left = place_output(
            &names,
            &saved,
            &outputs,
            &connected,
            "eDP-1",
            "DP-1",
            grid(),
        );
        let right = place_output(
            &names,
            &saved,
            &outputs,
            &connected,
            "eDP-1",
            "eDP-1",
            grid(),
        );
        assert_eq!(left.keys().cloned().collect::<Vec<_>>(), vec!["left"]);
        assert_eq!(right.len(), 2);
        assert_eq!(left["left"], right["right"]); // Same coordinates on different outputs are fine.
        assert_separate(&right);
        remember_output(&right, &mut saved, &mut outputs, "eDP-1");
        assert_eq!(outputs["new"], "eDP-1");
    }

    #[test]
    fn unplugged_icons_get_free_guest_spots_and_replug_restores_exact_home() {
        let names = vec!["home".into(), "guest".into()];
        let mut outputs = BTreeMap::from([
            ("home".into(), "eDP-1".into()),
            ("guest".into(), "DP-1".into()),
        ]);
        let home = FreePos {
            x: 650.5,
            y: 350.25,
            page: 2,
        };
        let mut saved = Positions::from([("guest".into(), home)]);
        let before = saved.clone();
        let fallback = place_output(
            &names,
            &saved,
            &outputs,
            &["eDP-1".into()],
            "eDP-1",
            "eDP-1",
            grid(),
        );
        assert_separate(&fallback);
        assert_eq!(fallback["guest"].page, 0);
        remember_output(&fallback, &mut saved, &mut outputs, "eDP-1");
        assert_eq!(saved["guest"], before["guest"]);
        assert_eq!(outputs["guest"], "DP-1");
        let restored = place_output(
            &names,
            &saved,
            &outputs,
            &["eDP-1".into(), "DP-1".into()],
            "eDP-1",
            "DP-1",
            grid(),
        );
        assert_eq!(restored, Positions::from([("guest".into(), home)]));
    }

    #[test]
    fn output_geometry_clamps_only_its_own_icons_and_preserves_other_homes() {
        let names = vec!["small".into(), "large".into()];
        let connected = vec!["DP-1".into(), "eDP-1".into()];
        let outputs = BTreeMap::from([
            ("small".into(), "eDP-1".into()),
            ("large".into(), "DP-1".into()),
        ]);
        let far = FreePos {
            x: 1700.75,
            y: 900.5,
            page: 0,
        };
        let saved = Positions::from([("small".into(), far), ("large".into(), far)]);
        let small_grid = Grid::new(800, 600, 72);
        let small = place_output(
            &names, &saved, &outputs, &connected, "eDP-1", "eDP-1", small_grid,
        );
        let large = place_output(
            &names,
            &saved,
            &outputs,
            &connected,
            "eDP-1",
            "DP-1",
            Grid::new(2560, 1440, 40),
        );
        assert_eq!(small["small"], small_grid.clamp(far));
        assert_eq!(large["large"], far);
    }

    #[test]
    fn state_keeps_connector_names_and_migrates_positions_without_them() {
        let positions = Positions::from([(
            "file".into(),
            FreePos {
                x: 50.5,
                y: 100.25,
                page: 1,
            },
        )]);
        let outputs = BTreeMap::from([("file".into(), "DP-3".into())]);
        let (restored, connectors) =
            decode_state(&encode_state(&positions, &outputs).unwrap()).unwrap();
        assert_eq!(restored, positions);
        assert_eq!(connectors, outputs);
        let (legacy, outputs) = decode_state(&encode(&positions).unwrap()).unwrap();
        assert_eq!(legacy, positions);
        assert!(outputs.is_empty());
    }

    #[test]
    fn cross_output_group_drop_uses_target_logical_coordinates_and_original_grab() {
        let drag = Drag {
            start: (540.25, 220.5),
            origins: BTreeMap::from([("a".into(), (500.0, 200.0)), ("b".into(), (620.0, 250.0))]),
        };
        let dropped = drag.dropped(Grid::new(1280, 720, 50), 240.75, 160.25, 0);
        assert_eq!(dropped["a"].x, 200.5);
        assert_eq!(dropped["a"].y, 139.75);
        assert_eq!(dropped["b"].x - dropped["a"].x, 120.0);
        assert_eq!(dropped["b"].y - dropped["a"].y, 50.0);
    }

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
                .into_iter()
                .collect(),
        };
        let (_, hot) = preview_bounds(&drag.origins, press);
        for pointer in [(351.0, 237.5), (951.0, 537.5), (401.0, 317.5)] {
            // The same frozen surface follows the pointer on either target;
            // neither leaving nor re-entering may rebase it to the old cell.
            let moved = drag.at(pointer.0, pointer.1);
            for (name, &(x, y)) in &drag.origins {
                let in_preview = (x + hot.0 as f64 - press.0, y + hot.1 as f64 - press.1);
                assert_eq!(
                    moved[name],
                    (
                        pointer.0 - hot.0 as f64 + in_preview.0,
                        pointer.1 - hot.1 as f64 + in_preview.1
                    )
                );
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
        let plans = plan_imports(uris, &desktop, BTreeSet::new(), false, false);
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
            import(&gio::File::for_uri(&plan.uri), &plan.dest, false, false).unwrap();
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
        g.reserved[0] = crate::batterywidget::card_rect(
            placement,
            crate::batterywidget::SHOWN,
            (800, 600),
            26,
            if clock == "left" { 214 } else { 0 },
            [0; 3],
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
    fn copy_rejects_descendants_including_symlinked_parents() {
        let dir = temp();
        let src = dir.join("src");
        std::fs::create_dir_all(src.join("Desktop")).unwrap();
        std::os::unix::fs::symlink(src.join("Desktop"), dir.join("alias")).unwrap();
        for dest in [src.clone(), src.join("Desktop/copy"), dir.join("alias/copy")] {
            assert!(copy_tree(&src, &dest).is_err());
        }
        assert_eq!(std::fs::read_dir(src.join("Desktop")).unwrap().count(), 0);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn cut_cleanup_preserves_new_changed_and_replaced_entries() {
        let dir = temp();
        let src = dir.join("src");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        for name in ["unchanged", "changed", "mtime", "replaced"] {
            std::fs::write(src.join(name), b"original").unwrap();
        }
        std::os::unix::fs::symlink("unchanged", src.join("link")).unwrap();
        let copied = copy_tree(&src, &dir.join("dest")).unwrap();
        std::fs::write(src.join("new"), b"new").unwrap();
        std::fs::write(src.join("changed"), b"changed size").unwrap();
        let file = std::fs::OpenOptions::new().write(true).open(src.join("mtime")).unwrap();
        file.set_modified(std::time::SystemTime::UNIX_EPOCH).unwrap();
        std::fs::rename(src.join("replaced"), dir.join("old")).unwrap();
        std::fs::write(src.join("replaced"), b"original").unwrap();
        assert!(remove_import_source(&copied).is_err());
        for name in ["new", "changed", "mtime", "replaced"] { assert!(src.join(name).exists()); }
        assert!(!src.join("unchanged").exists());
        assert!(src.join("link").symlink_metadata().is_err());
        assert!(!src.join("sub").exists());
        assert_eq!(std::fs::read(dir.join("dest/changed")).unwrap(), b"original");
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
    fn cross_device_cut_removes_only_fully_copied_sources() {
        let dir = temp();
        let desktop = dir.join("Desktop");
        std::fs::create_dir_all(&desktop).unwrap();
        let source = dir.join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("file"), "contents").unwrap();
        // A FIFO is unsupported: a partially copied folder must leave its source intact.
        use std::os::unix::ffi::OsStrExt;
        let fifo = std::ffi::CString::new(source.join("fifo").as_os_str().as_bytes()).unwrap();
        // SAFETY: fifo is a valid, NUL-terminated path for the duration of the call.
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        let file = gio::File::for_path(&source);
        let dest = desktop.join("source");
        assert!(import_with_devices(&file, &dest, false, true, |_, _| Ok(false)).is_err());
        assert!(source.join("file").exists());
        assert!(source.join("fifo").exists());
        assert!(!dest.exists());
        std::fs::remove_file(source.join("fifo")).unwrap();
        import_with_devices(&file, &dest, false, false, |_, _| Ok(false)).unwrap();
        assert!(source.join("file").exists(), "a normal cross-device drop stays a copy");
        assert!(import_with_devices(&file, &dest, false, true, |_, _| Ok(false)).is_err());
        assert!(source.join("file").exists(), "a collision must preserve the source");
        std::fs::remove_dir_all(&dest).unwrap();
        let plans = plan_imports(vec![file.uri().to_string()], &desktop, BTreeSet::new(), false, true);
        assert!(plans[0].cut);
        import_with_devices(&file, &plans[0].dest, false, plans[0].cut, |_, _| Ok(false)).unwrap();
        assert!(!source.exists());
        assert_eq!(std::fs::read_to_string(dest.join("file")).unwrap(), "contents");
        let link = dir.join("link");
        std::os::unix::fs::symlink(dest.join("file"), &link).unwrap();
        import_with_devices(&gio::File::for_path(&link), &desktop.join("link"), false, true, |_, _| Ok(false)).unwrap();
        assert!(link.symlink_metadata().is_err());
        assert!(desktop.join("link").symlink_metadata().unwrap().is_symlink());
        assert!(dest.join("file").exists(), "moving a symlink must preserve its target");
        assert!(imports_succeeded(&Ok::<_, ()>(vec![Ok(()), Ok(())])));
        assert!(!imports_succeeded(&Ok::<_, ()>(vec![Ok(()), Err("Failed".into())])));
        assert!(!imports_succeeded(&Err::<Vec<Result<(), String>>, _>(())));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn import_moves_on_same_filesystem_and_refuses_collisions() {
        let dir = temp();
        std::fs::create_dir_all(dir.join("Desktop")).unwrap();
        let source = dir.join("a");
        let dest = dir.join("Desktop/a");
        std::fs::write(&source, b"original").unwrap();
        import(&gio::File::for_path(&source), &dest, false, false).unwrap();
        assert!(!source.exists());
        std::fs::write(&source, b"replacement").unwrap();
        assert!(import(&gio::File::for_path(&source), &dest, false, false).is_err());
        assert_eq!(std::fs::read(&dest).unwrap(), b"original");
        assert_eq!(std::fs::read(&source).unwrap(), b"replacement");
        import(&gio::File::for_path(&dest), &dest, false, false).unwrap();
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
    fn a_name_in_use_gets_the_next_number_and_copies_leave_the_source() {
        let root = std::env::temp_dir().join(format!("digit-desk-names-{}", std::process::id()));
        std::fs::create_dir_all(root.join("test")).unwrap();
        std::fs::create_dir_all(root.join("test (2)")).unwrap();
        let free = collision_name(std::ffi::OsStr::new("test"), true, |c| root.join(c).symlink_metadata().is_ok());
        assert_eq!(free, "test (3)");
        std::fs::write(root.join("a.txt"), "x").unwrap();
        let uri = gio::File::for_path(root.join("a.txt")).uri().to_string();
        let plans = plan_imports(vec![uri], &root, BTreeSet::new(), true, false);
        assert_eq!(plans[0].dest, root.join("a (2).txt"), "a copy onto itself is a duplicate");
        import(&gio::File::for_uri(&plans[0].uri), &plans[0].dest, true, false).unwrap();
        assert!(root.join("a.txt").exists() && root.join("a (2).txt").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn menus_follow_files_order_and_undo_shows_only_when_possible() {
        use MenuAction::*;
        let empty = menu_entries(false, 0, false);
        assert_eq!(&empty[..3], &[NewFolder, Paste, SelectAll]);
        assert!(!empty.contains(&Undo));
        assert!(menu_entries(false, 3, true).contains(&Undo));
        assert_eq!(menu_entries(true, 1, true), vec![Open, Reveal, Separator, Cut, Copy, CopyPath, Separator, Rename, Duplicate, Separator, Trash]);
        for count in [2, 20] {
            assert_eq!(menu_entries(true, count, false), vec![Open, Separator, Cut, Copy, Duplicate, Separator, Trash]);
        }
        for action in menu_entries(true, 1, false).into_iter().chain(menu_entries(false, 0, true)) {
            assert_eq!(action.glyph().is_empty(), action == Separator);
        }
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
