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

mod drag;
mod grid;
mod import;
mod menu;
mod ops;
mod rename;
mod render;
mod state;
use self::{drag::*, grid::*, import::*, menu::*, ops::*, rename::*, render::*, state::*};

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
    /// The [`ICON_LOOK`] its items' icons were made under.
    icon_look: u64,
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
    /// The user clicked the desktop itself (or dragged a band on it): it becomes the
    /// active place, like a file manager window, so Ctrl+A and paste work with nothing
    /// selected yet.
    Clicked,
    Window,
    Overlay,
}
fn keyboard_requested(selected: bool, event: KeyboardEvent) -> bool {
    (selected && matches!(event, KeyboardEvent::Select)) || matches!(event, KeyboardEvent::Clicked)
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
    focus_desktop(d, false);
}

/// Takes the keyboard for the selection, or for any click on the desktop (`clicked`).
fn focus_desktop(d: &Rc<RefCell<Desktop>>, clicked: bool) {
    FOCUS_GENERATION.set(FOCUS_GENERATION.get().wrapping_add(1));
    let target = {
        let mut d = d.borrow_mut();
        let wanted = !d.selected.is_empty() || can_undo();
        let event = if wanted {
            KeyboardEvent::Select
        } else if clicked {
            KeyboardEvent::Clicked
        } else {
            KeyboardEvent::Clear
        };
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
            // The old surface shows a still copy of the icons until the new one has drawn:
            // emptying it at once made every icon blink. It empties right after that first
            // frame, since two surfaces on top of each other double the labels' shadows.
            let still = gtk::Picture::for_paintable(&gtk::WidgetPaintable::new(Some(&paint)).current_image());
            still.set_can_shrink(true);
            still.set_content_fit(gtk::ContentFit::Fill);
            old.set_child(Some(&still));
            window.set_child(Some(&paint));
            {
                let mut state = d.borrow_mut();
                state.retiring = Some(old.clone());
                state.window = window.clone();
            }
            watch_focus(&d, &window);
            window.present();
            paint.grab_focus();
            // Tick callbacks run before a frame is drawn: the second tick comes after the
            // first frame went out.
            let (old, frames) = (old.downgrade(), std::cell::Cell::new(0));
            paint.add_tick_callback(move |_, _| {
                frames.set(frames.get() + 1);
                if frames.get() < 2 {
                    return glib::ControlFlow::Continue;
                }
                if let Some(old) = old.upgrade() {
                    old.set_child(gtk::Widget::NONE);
                }
                glib::ControlFlow::Break
            });
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
    let monitors = kit::screens::all();
    let names: Vec<_> = monitors.iter().map(kit::screens::name).collect();
    let primary = app.primary_name();
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
    let mut made = false;
    for monitor in monitors {
        let output = monitor
            .connector()
            .map(|c| c.to_string())
            .unwrap_or_default();
        if !surfaces().iter().any(|d| d.borrow().output == output) {
            create_surface(app, monitor, shared.clone(), output == primary);
            made = true;
        }
    }
    // Every settings change lands here. Only a new surface needs the folder read again;
    // otherwise the icons just make room for widgets that moved, without rebuilding them.
    if made || !shared.borrow().ready {
        if let Some(d) = surfaces().first() {
            refresh(d);
        }
        return;
    }
    for d in surfaces() {
        let busy = {
            let state = d.borrow();
            state.drag.is_some() || state.rubber.is_some() || state.importing
        };
        if !busy {
            d.borrow_mut().layout(true);
            animate(&d);
        }
    }
}

/// The Now playing buttons moved, came or went: each surface leaves their place again, with its
/// next frame.
pub fn widget_buttons_moved() {
    for d in surfaces() {
        if let Ok(d) = d.try_borrow() {
            d.paint.queue_draw();
        }
    }
}

/// Takes every click on the screen but the Now playing buttons. The widgets share this surface's
/// layer, and the one mapped last is on top: this surface is mapped again whenever it asks for
/// the keyboard, and would then cover the buttons. Called on each draw, so a new surface takes
/// it with its first frame.
fn leave_widget_buttons(d: &Rc<RefCell<Desktop>>) {
    let Ok(d) = d.try_borrow() else { return };
    let (Some(app), Some(surface)) = (d.app.upgrade(), d.window.surface()) else { return };
    let (w, h) = (d.geometry.0, d.geometry.1);
    let region = gtk::cairo::Region::create_rectangle(&gtk::cairo::RectangleInt::new(0, 0, w, h));
    for r in crate::deskcards::media_buttons(&app, &d.output, (w, h)) {
        let _ = region.subtract_rectangle(&gtk::cairo::RectangleInt::new(r.x.floor() as i32, r.y.floor() as i32, r.w.ceil() as i32 + 1, r.h.ceil() as i32 + 1));
    }
    surface.set_input_region(Some(&region));
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
        icon_look: ICON_LOOK.get(),
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
            leave_widget_buttons(&d);
            draw_desktop(&d, s);
        }
    });
    controllers(&desktop, &paint);
    let theme = gtk::IconTheme::for_display(&paint.display());
    let weak = Rc::downgrade(&desktop);
    let handler = theme.connect_changed(move |_| {
        ICON_LOOK.set(ICON_LOOK.get() + 1);
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
        self.grid.reserved = [None; 6];
        // Every screen keeps its icons clear of the widgets its own places put there.
        if let Some(app) = self.app.upgrade().filter(|app| app.cfg.wallpaper) {
            let screen = (self.geometry.0, self.geometry.1);
            let places = crate::batterywidget::places(&app, &self.output);
            let battery = crate::batterywidget::card_rect(&places[1], crate::batterywidget::SHOWN, screen, app.reserve(), crate::desk::room(&app, &self.output), crate::dock::room(&self.output), app.prefs.borrow().size_of("battery"));
            let clock = crate::desk::rect(&app, &self.output, screen);
            let all = clock.into_iter().chain(battery).chain(crate::deskcards::rects(&app, &self.output, screen));
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
            } else if (i.x.target(), i.y.target()) != (x, y) {
                // A glide already heading here keeps going.
                i.x.set(x);
                i.y.set(y);
            }
        }
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
        // A selection emptied elsewhere lets this surface's keyboard go. An empty selection
        // alone doesn't: a click on the bare desktop takes the keyboard (Ctrl+A, paste).
        if changed && selected.is_empty() {
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
thread_local! {
    /// Counts icon theme and accent changes: icons made before one aren't kept.
    static ICON_LOOK: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// The accent changed: folders are drawn in it.
pub fn restyle() {
    ICON_LOOK.set(ICON_LOOK.get() + 1);
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
            // Positions carry over too, with any glide in flight: the rescan that follows a
            // drop must not cut the icon's move beside a widget short.
            let motion: BTreeMap<_, _> = state.items.iter().map(|i| (i.data.name.clone(), (i.x, i.y))).collect();
            for item in &mut items {
                if let Some(selected) = selection.get(&item.data.name) {
                    item.selected = *selected;
                }
                if let Some(&(x, y)) = motion.get(&item.data.name) {
                    (item.x, item.y) = (x, y);
                }
            }
            // Keep the icon an unchanged item already shows: a fresh themed icon loads on
            // its first frame, and every file icon blinked whenever the folder changed.
            let shown: BTreeMap<_, _> = state
                .items
                .iter()
                .map(|i| (i.data.name.clone(), (i.data.icon.clone(), i.data.directory, i.icon.clone())))
                .collect();
            let same_look = std::mem::replace(&mut state.icon_look, ICON_LOOK.get()) == ICON_LOOK.get();
            for item in items.iter_mut().filter(|_| same_look) {
                if let Some((icon, directory, paintable)) = shown.get(&item.data.name)
                    && *icon == item.data.icon
                    && *directory == item.data.directory
                    && !item.data.directory
                {
                    item.icon = paintable.clone();
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
        focus_desktop(&d, true);
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
        focus_desktop(&d, true);
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
        let (preview, hotspot, local) = {
            let mut state = d.borrow_mut();
            let Some(drag) = state.drag.as_mut() else {
                return;
            };
            drag.system = Some(system.clone());
            // A drag starts over the desktop: keep drawing the items here from the first
            // frame, instead of hiding them until the drop target's enter arrives a frame
            // or two later and showing them again, which blinked every dragged icon.
            if drag.local_preview {
                drag.over_desktop = true;
            }
            let preview = (drag.preview.clone(), drag.hotspot, drag.local_preview);
            state.last_click = None;
            preview
        };
        // The shared helper also repairs Hyprland's offset after GPU attaches;
        // merely setting GDK's hotspot before the frame cannot survive those.
        let mut picture = Some(kit::drag::set_icon(system, &preview, hotspot));
        if local && let Some(picture) = &picture {
            // Over the desktop the native visual stays empty (see desktop_drag_target).
            picture.set_paintable(Some(&clear_texture()));
        }
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

#[cfg(test)]
mod tests;
