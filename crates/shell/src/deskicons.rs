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
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

const CELL_W: i32 = 112;
const CELL_H: i32 = 116;
const PAD: i32 = 24;
const ICON: i32 = 64;
const LABEL_INSET: i32 = 6;
type Positions = BTreeMap<String, CellPos>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
struct CellPos {
    col: u32,
    row: u32,
    #[serde(default)]
    page: u32,
}

#[derive(Clone, Copy)]
struct Grid {
    cols: u32,
    rows: u32,
    width: i32,
    top: i32,
    reserved: Option<Rect>,
}
impl Grid {
    fn new(width: i32, height: i32, top: i32) -> Self {
        let top = top.max(PAD).min((height - CELL_H - PAD).max(0));
        Self {
            cols: ((width - 2 * PAD) / CELL_W).max(1) as u32,
            rows: ((height - top - 88) / CELL_H).max(1) as u32,
            width,
            top,
            reserved: None,
        }
    }
    fn point(self, p: CellPos) -> (f64, f64) {
        (
            (self.width - PAD - CELL_W - p.col as i32 * CELL_W).max(0) as f64,
            (self.top + p.row as i32 * CELL_H) as f64,
        )
    }
    fn snap(self, x: f64, y: f64, page: u32) -> CellPos {
        CellPos {
            col: (((self.width - PAD - CELL_W) as f64 - x) / CELL_W as f64)
                .round()
                .clamp(0.0, (self.cols - 1) as f64) as u32,
            row: ((y - self.top as f64) / CELL_H as f64)
                .round()
                .clamp(0.0, (self.rows - 1) as f64) as u32,
            page,
        }
    }
    fn nth(self, n: usize) -> CellPos {
        let n = n as u32;
        CellPos {
            row: n % self.rows,
            col: (n / self.rows) % self.cols,
            page: n / (self.rows * self.cols),
        }
    }
    fn reserved(self, p: CellPos) -> bool {
        let (x, y) = self.point(p);
        self.reserved.is_some_and(|r| {
            r.intersects(Rect {
                x,
                y,
                w: CELL_W as f64,
                h: CELL_H as f64,
            })
        })
    }
    fn automatic_cells(self) -> Vec<CellPos> {
        (0..(self.cols * self.rows) as usize)
            .map(|n| self.nth(n))
            .filter(|p| !self.reserved(*p))
            .collect()
    }
    fn free(self, occupied: &BTreeSet<CellPos>) -> CellPos {
        (0..=occupied.len())
            .map(|n| self.nth(n))
            .find(|p| !occupied.contains(p))
            .unwrap_or_default()
    }
}

/// Preserve valid cells before assigning new or displaced items, in name order.
fn place(names: &[String], saved: &Positions, grid: Grid) -> Positions {
    let mut cells = grid.automatic_cells();
    // On very small displays there may be no room beside the card. Keep icons reachable.
    if cells.is_empty() {
        cells.extend((0..(grid.cols * grid.rows) as usize).map(|n| grid.nth(n)));
    }
    let mut out = Positions::new();
    let mut occupied = BTreeSet::new();
    for name in names {
        if let Some(&p) = saved.get(name)
            && p.col < grid.cols
            && p.row < grid.rows
            && p.page <= (names.len() / cells.len()) as u32
            && occupied.insert(p)
        {
            out.insert(name.clone(), p);
        }
    }
    for name in names {
        if !out.contains_key(name) {
            let p = (0..=occupied.len())
                .map(|n| CellPos {
                    page: (n / cells.len()) as u32,
                    ..cells[n % cells.len()]
                })
                .find(|p| !occupied.contains(p))
                .expect("one more candidate than occupied cells");
            occupied.insert(p);
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
    serde_json::from_slice(bytes)
}
fn state_path() -> PathBuf {
    kit::theme::state_dir().join("desktop-icons.json")
}
fn read_positions(path: &Path) -> Result<Positions, String> {
    match std::fs::read(path) {
        Ok(bytes) => decode(&bytes).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Positions::new()),
        Err(e) => Err(e.to_string()),
    }
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
struct Desktop {
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
    drag: Option<Drag>,
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
                let Ok(d) = d.try_borrow() else { return false };
                let g = m.geometry();
                d.monitor == *m
                    && d.geometry == (g.width(), g.height(), app.reserve())
                    && d.paint.scale_factor() == m.scale_factor()
            })
        });
    if keep {
        if let Some(d) = current {
            {
                let Ok(mut state) = d.try_borrow_mut() else {
                    return;
                };
                if state.drag.is_none() && state.rubber.is_none() {
                    state.layout(true);
                } else {
                    state.dirty = true;
                }
            }
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
    window.set_keyboard_mode(KeyboardMode::OnDemand);
    window.set_exclusive_zone(-1);
    for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(edge, true);
    }
    window.add_css_class("digit");
    let paint = Paint::new(1, 1);
    paint.set_hexpand(true);
    paint.set_vexpand(true);
    paint.set_focusable(true);
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
        ticking: false,
        loading: false,
        dirty: false,
        writable: true,
        closed: false,
        monitor_file: None,
        pending: None,
        rename_window: None,
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
        self.positions
            .get(&item.data.name)
            .is_some_and(|p| p.page == self.page)
    }
    fn hit(&self, x: f64, y: f64) -> Option<usize> {
        let now = glib::monotonic_time();
        self.items
            .iter()
            .position(|i| self.visible(i) && i.rect(now).contains(x, y))
    }
    fn paths(&self) -> Vec<PathBuf> {
        self.items
            .iter()
            .filter(|i| self.selected.contains(&i.data.name))
            .map(|i| i.data.path.clone())
            .collect()
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
        let names: Vec<_> = self.items.iter().map(|i| i.data.name.clone()).collect();
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
        // Recovered saved positions remain saved; automatic positions remain free to reflow.
        self.saved.retain(|name, p| {
            if let Some(&placed) = self.positions.get(name) {
                *p = placed;
                true
            } else {
                false
            }
        });
        self.page = self
            .page
            .min(self.positions.values().map(|p| p.page).max().unwrap_or(0));
        let now = glib::monotonic_time();
        for i in &mut self.items {
            let (x, y) = self.grid.point(self.positions[&i.data.name]);
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
            .map(|i| {
                (
                    i.icon.clone(),
                    i.label.clone(),
                    i.rect(now),
                    i.selected.value(now),
                )
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
    for (icon, label, r, selected) in items {
        let x = r.x.round();
        let y = r.y.round();
        // Finder's look: a soft tile behind the icon and a pill around the words when selected,
        // white text with a small shadow otherwise, so any wallpaper stays readable.
        let (w, h) = label.pixel_size();
        let (w, h) = (w as f64, h as f64);
        let label_y = y + ICON as f64 + 6.0;
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
        draw::paintable(s, &icon, x + (CELL_W - ICON) as f64 / 2.0, y, ICON as f64);
        s.save();
        s.translate(&graphene::Point::new((x + LABEL_INSET as f64) as f32, label_y as f32));
        s.push_shadow(&[gtk::gsk::Shadow::new(
            gdk::RGBA::new(0.0, 0.0, 0.0, 0.55 * (1.0 - selected as f32)),
            0.0,
            1.0,
            3.0,
        )]);
        let ink = pal.on_accent.mix(Rgb::new(1.0, 1.0, 1.0), selected);
        s.append_layout(&label, &ink.rgba());
        s.pop();
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
        let busy = {
            // A nested frame may run while a callback is updating the model.
            let Ok(mut d) = d.try_borrow_mut() else {
                return glib::ControlFlow::Continue;
            };
            let now = clock.frame_time();
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
    if state.loading || state.drag.is_some() || state.rubber.is_some() {
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
                    let icon = theme
                        .lookup_by_gicon(
                            &icon,
                            ICON,
                            scale,
                            gtk::TextDirection::Ltr,
                            gtk::IconLookupFlags::empty(),
                        )
                        .upcast();
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
                {
                    let mut state = d.borrow_mut();
                    // An interaction begun while metadata was loading must not be overwritten.
                    if state.drag.is_none() && state.rubber.is_none() {
                        state.items = items;
                        let names: BTreeSet<_> =
                            state.items.iter().map(|i| i.data.name.clone()).collect();
                        let selected = state.selected.intersection(&names).cloned().collect();
                        state.select(selected);
                        state.layout(false);
                    } else {
                        state.dirty = true;
                    }
                }
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
        state.rename_window.take()
    };
    if let Some(window) = window {
        window.destroy();
    }
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
    let width = 180.min(geometry.width().max(1));
    let entry = gtk::Entry::new();
    entry.add_css_class("search");
    gtk::prelude::EntryExt::set_alignment(&entry, 0.5);
    entry.set_width_chars(1);
    entry.set_max_width_chars(1);
    entry.set_size_request(width, 38);
    let filename = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    entry.set_text(&filename);
    let selection = rename_selection(&filename, directory);
    let error = gtk::Label::new(None);
    error.add_css_class("error");
    error.set_wrap(true);
    error.set_max_width_chars(22);
    error.set_visible(false);
    let editor = gtk::Box::new(gtk::Orientation::Vertical, 4);
    editor.append(&entry);
    editor.append(&error);
    let x = (r.x + CELL_W as f64 / 2.0 - width as f64 / 2.0)
        .clamp(0.0, (geometry.width() - width).max(0) as f64);
    let y = (r.y + 70.0).clamp(0.0, (geometry.height() - 38).max(0) as f64);
    fixed.put(&editor, x.round(), y.round());

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
    d.borrow_mut().rename_window = Some(window.clone());
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
fn selected_files(d: &Rc<RefCell<Desktop>>) -> Vec<FileData> {
    let Ok(state) = d.try_borrow() else {
        return Vec::new();
    };
    state
        .items
        .iter()
        .filter(|i| state.selected.contains(&i.data.name))
        .map(|i| i.data.clone())
        .collect()
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
                            state.saved.clear();
                            state.layout(true);
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
    let weak = Rc::downgrade(d);
    click.connect_pressed(move |g, n, x, y| {
        let Some(d) = weak.upgrade() else { return };
        let button = g.current_button();
        if button != 1 && button != 3 {
            return;
        }
        let ctrl = g
            .current_event_state()
            .contains(gdk::ModifierType::CONTROL_MASK);
        let (on_icon, paint, open_file) = {
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
            (at.is_some(), state.paint.clone(), open_file)
        };
        // Focus and gesture arbitration can synchronously emit cancel/drag-end.
        paint.grab_focus();
        animate(&d);
        if button == 3 {
            g.set_state(gtk::EventSequenceState::Claimed);
            menu(&d, on_icon, x, y);
        } else if let Some(file) = open_file {
            open(&file);
        }
    });
    paint.add_controller(click);

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
    source.set_actions(gdk::DragAction::COPY | gdk::DragAction::MOVE);
    let weak = Rc::downgrade(d);
    source.connect_prepare(move |source, x, y| {
        let d = weak.upgrade()?;
        let (icon, paths) = {
            let mut state = d.try_borrow_mut().ok()?;
            let at = state.hit(x, y)?;
            let name = state.items[at].data.name.clone();
            // Ctrl-click toggles selection; dragging an unselected icon still exports it.
            if !state.selected.contains(&name) {
                state.select([name].into_iter().collect());
            }
            let now = glib::monotonic_time();
            state.drag = Some(Drag {
                start: (x, y),
                origins: state
                    .items
                    .iter()
                    .filter(|i| state.selected.contains(&i.data.name))
                    .map(|i| (i.data.name.clone(), (i.x.value(now), i.y.value(now))))
                    .collect(),
            });
            (state.items[at].icon.clone(), state.paths())
        };
        source.set_icon(Some(&icon), 32, 32);
        let files: Vec<_> = paths.iter().map(gio::File::for_path).collect();
        Some(gdk::ContentProvider::for_value(
            &gdk::FileList::from_array(&files).to_value(),
        ))
    });
    let weak = Rc::downgrade(d);
    source.connect_drag_end(move |_, _, _| {
        if let Some(d) = weak.upgrade() {
            cancel_drag(&d);
        }
    });
    let weak = Rc::downgrade(d);
    source.connect_drag_cancel(move |_, _, _| {
        if let Some(d) = weak.upgrade() {
            cancel_drag(&d);
        }
        false
    });
    paint.add_controller(source);

    let target = gtk::DropTarget::new(
        gdk::FileList::static_type(),
        gdk::DragAction::COPY | gdk::DragAction::MOVE,
    );
    let weak = Rc::downgrade(d);
    target.connect_enter(move |_, _, _| {
        if weak
            .upgrade()
            .is_some_and(|d| d.try_borrow().is_ok_and(|d| d.drag.is_some()))
        {
            gdk::DragAction::MOVE
        } else {
            gdk::DragAction::COPY
        }
    });
    let weak = Rc::downgrade(d);
    target.connect_motion(move |_, x, y| {
        let Some(d) = weak.upgrade() else {
            return gdk::DragAction::empty();
        };
        let Ok(mut state) = d.try_borrow_mut() else {
            return gdk::DragAction::empty();
        };
        if let Some(drag) = &state.drag {
            let dx = x - drag.start.0;
            let dy = y - drag.start.1;
            let origins = drag.origins.clone();
            for i in &mut state.items {
                if let Some(&(ox, oy)) = origins.get(&i.data.name) {
                    i.x.set(ox + dx);
                    i.y.set(oy + dy);
                }
            }
            let paint = state.paint.clone();
            drop(state);
            paint.queue_draw();
            gdk::DragAction::MOVE
        } else {
            // We own the transfer; reporting COPY prevents source-side deletion on failure
            // and preserves the original for cross-filesystem copies.
            gdk::DragAction::COPY
        }
    });
    let weak = Rc::downgrade(d);
    target.connect_drop(move |_, value, x, y| {
        let Some(d) = weak.upgrade() else {
            return false;
        };
        let Ok(list) = value.get::<gdk::FileList>() else {
            return false;
        };
        let (internal, folder) = {
            let Ok(state) = d.try_borrow() else {
                return false;
            };
            (state.drag.is_some(), state.folder.clone())
        };
        if internal {
            finish_drag(&d, x, y);
            return true;
        }
        let uris: Vec<_> = list.files().iter().map(|f| f.uri().to_string()).collect();
        if uris.is_empty() {
            return false;
        }
        glib::spawn_future_local(async move {
            let _ = gio::spawn_blocking(move || {
                for uri in uris {
                    if let Err(e) = import(&gio::File::for_uri(&uri), &folder) {
                        eprintln!("digit: desktop drop: {e}");
                    }
                }
            })
            .await;
        });
        true
    });
    paint.add_controller(target);

    let keys = gtk::EventControllerKey::new();
    let weak = Rc::downgrade(d);
    keys.connect_key_pressed(move |_, key, _, mods| {
        let Some(d) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        match key {
            gdk::Key::Return | gdk::Key::KP_Enter => {
                let files = selected_files(&d);
                files.iter().for_each(open);
            }
            gdk::Key::Delete | gdk::Key::KP_Delete => {
                let paths = {
                    let Ok(state) = d.try_borrow() else {
                        return glib::Propagation::Proceed;
                    };
                    state.paths()
                };
                trash(paths);
            }
            gdk::Key::F2 => rename(&d),
            gdk::Key::Escape => {
                cancel_drag(&d);
                {
                    let Ok(mut state) = d.try_borrow_mut() else {
                        return glib::Propagation::Proceed;
                    };
                    state.rubber = None;
                    state.select(BTreeSet::new());
                }
                animate(&d);
            }
            gdk::Key::a if mods.contains(gdk::ModifierType::CONTROL_MASK) => {
                {
                    let Ok(mut state) = d.try_borrow_mut() else {
                        return glib::Propagation::Proceed;
                    };
                    let selected = state
                        .items
                        .iter()
                        .filter(|i| state.visible(i))
                        .map(|i| i.data.name.clone())
                        .collect();
                    state.select(selected);
                }
                animate(&d);
            }
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
    if state.drag.take().is_some() {
        state.layout(true);
    }
    let refresh_needed = state.dirty;
    drop(state);
    animate(d);
    if refresh_needed {
        refresh(d);
    }
}
fn finish_drag(d: &Rc<RefCell<Desktop>>, x: f64, y: f64) {
    let Ok(mut state) = d.try_borrow_mut() else {
        return;
    };
    let Some(drag) = state.drag.take() else {
        return;
    };
    let grid = state.grid;
    let page = state.page;
    let mut occupied: BTreeSet<_> = state
        .positions
        .iter()
        .filter(|(n, _)| !drag.origins.contains_key(*n))
        .map(|(_, p)| *p)
        .collect();
    for (name, (ox, oy)) in drag.origins {
        let desired = grid.snap(ox + x - drag.start.0, oy + y - drag.start.1, page);
        let cell = nearest_free(grid, desired, &occupied);
        occupied.insert(cell);
        state.saved.insert(name, cell);
    }
    state.layout(true);
    let refresh_needed = state.dirty;
    drop(state);
    save(d);
    animate(d);
    if refresh_needed {
        refresh(d);
    }
}
fn nearest_free(grid: Grid, desired: CellPos, occupied: &BTreeSet<CellPos>) -> CellPos {
    if !occupied.contains(&desired) {
        return desired;
    }
    (0..grid.cols as usize * grid.rows as usize)
        .map(|n| CellPos {
            page: desired.page,
            ..grid.nth(n)
        })
        .filter(|p| !occupied.contains(p))
        .min_by_key(|p| p.col.abs_diff(desired.col) + p.row.abs_diff(desired.row))
        .unwrap_or_else(|| grid.free(occupied))
}
fn import(file: &gio::File, folder: &Path) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let name = file.basename().ok_or("Missing file name")?;
    let dest = folder.join(name);
    let target = gio::File::for_path(&dest);
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
            return copy_tree(&path, &dest).map_err(|e| e.to_string());
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
            Some(path) => copy_tree(&path, &dest).map_err(|e| e.to_string()),
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
    #[test]
    fn new_items_fill_down_from_top_right_without_displacing_saved_items() {
        let g = grid();
        let names = names(8);
        let saved = [(names[2].clone(), g.nth(0))].into_iter().collect();
        let p = place(&names, &saved, g);
        assert_eq!(p[&names[2]], g.nth(0));
        assert_eq!(p[&names[0]], g.nth(1));
        let (right, _) = g.point(g.nth(0));
        let (left, _) = g.point(g.nth(g.rows as usize));
        assert!(right > left);
        assert_eq!(
            p.values().copied().collect::<BTreeSet<_>>().len(),
            names.len()
        );
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
                // A bottom card can sit below the last row, leaving nothing to skip.
                assert!(
                    !corner.starts_with("top")
                        || (0..(g.cols * g.rows) as usize).any(|n| g.reserved(g.nth(n)))
                );
                let p = place(&names, &Positions::new(), g);
                assert!(p.values().all(|p| !g.reserved(*p)), "{corner}, {clock}");
                assert_eq!(
                    p.values().copied().collect::<BTreeSet<_>>().len(),
                    names.len()
                );
                assert_eq!(place(&names, &p, g), p);
            }
        }
    }
    #[test]
    fn saved_positions_inside_battery_card_stay_while_unsaved_icons_reflow() {
        let names = names(2);
        let g = battery_grid("top_right", "off");
        let saved = [(names[0].clone(), g.nth(0))].into_iter().collect();
        assert!(g.reserved(g.nth(0)));
        let p = place(&names, &saved, g);
        assert_eq!(p[&names[0]], g.nth(0));
        assert!(!g.reserved(p[&names[1]]));
        let off = place(&names, &saved, grid());
        assert_eq!(off[&names[0]], g.nth(0));
        assert_eq!(off[&names[1]], g.nth(1));
        assert_ne!(p[&names[1]], off[&names[1]]);
    }
    #[test]
    fn offscreen_saved_positions_recover_outside_battery_card() {
        let names = names(1);
        let g = battery_grid("top_right", "off");
        let saved = [(
            names[0].clone(),
            CellPos {
                col: 100,
                row: 100,
                page: 0,
            },
        )]
        .into_iter()
        .collect();
        let p = place(&names, &saved, g);
        assert!(!g.reserved(p[&names[0]]));
    }
    #[test]
    fn resolution_change_recovers_invalid_and_colliding_cells() {
        let g = grid();
        let names = names(3);
        let saved = [
            (
                names[0].clone(),
                CellPos {
                    col: 100,
                    row: 99,
                    page: 0,
                },
            ),
            (names[1].clone(), g.nth(0)),
            (names[2].clone(), g.nth(0)),
        ]
        .into_iter()
        .collect();
        let p = place(&names, &saved, g);
        assert_eq!(p[&names[1]], g.nth(0));
        assert!(p.values().all(|p| p.col < g.cols && p.row < g.rows));
        assert_eq!(p.values().copied().collect::<BTreeSet<_>>().len(), 3);
    }
    #[test]
    fn five_hundred_items_get_unique_reachable_pages() {
        let names = names(500);
        let p = place(&names, &Positions::new(), grid());
        assert_eq!(p.len(), 500);
        assert_eq!(p.values().copied().collect::<BTreeSet<_>>().len(), 500);
        assert_eq!(place(&names, &p, grid()), p);
    }
    #[test]
    fn snap_clamps_and_nearest_cell_avoids_other_icons() {
        let g = grid();
        let pos = g.snap(-9999.0, 9999.0, 0);
        assert_eq!(pos.col, g.cols - 1);
        assert_eq!(pos.row, g.rows - 1);
        let occupied = [pos].into_iter().collect();
        assert_ne!(nearest_free(g, pos, &occupied), pos);
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
        assert!(decode(br#"{"bad":{"col":-1,"row":0}}"#).is_err());
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
        import(&gio::File::for_path(&source), &dir.join("Desktop")).unwrap();
        assert!(!source.exists());
        std::fs::write(&source, b"replacement").unwrap();
        assert!(import(&gio::File::for_path(&source), &dir.join("Desktop")).is_err());
        assert_eq!(std::fs::read(&dest).unwrap(), b"original");
        assert_eq!(std::fs::read(&source).unwrap(), b"replacement");
        import(&gio::File::for_path(&dest), &dir.join("Desktop")).unwrap();
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
