use crate::{ops, search, thumbs};
use gtk::{gdk, gio, glib, pango, prelude::*};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

pub const ATTRS: &str = "standard::name,standard::display-name,standard::edit-name,standard::type,standard::size,standard::icon,standard::fast-content-type,standard::is-hidden,standard::is-backup,standard::is-symlink,standard::target-uri,time::modified,access::can-write,access::can-trash,access::can-rename,trash::orig-path,trash::deletion-date,unix::mode";
pub const SIZES: [i32; 6] = [40, 56, 72, 96, 128, 176];
// Match the desktop cell, regardless of Files zoom or grid/list allocation.
const DRAG_ICON: f64 = 64.0;
const DRAG_CELL: (f64, f64) = (112.0, 116.0);
const DRAG_NAME_Y: f64 = DRAG_ICON + 6.0;
type DragRect = (f64, f64, f64, f64);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum View {
    Grid,
    List,
}

pub struct Pane {
    pub root: gtk::Box,
    rename_generation: Cell<u64>,
    rename_layer: gtk::Overlay,
    editor: gtk::Entry,
    renaming: RefCell<Option<gio::FileInfo>>,
    rename_label: RefCell<Option<glib::WeakRef<gtk::Widget>>>,
    pub dir: Rc<crate::listing::Listing>,
    filter: gtk::CustomFilter,
    filtered: gtk::FilterListModel,
    pub sorted: gtk::SortListModel,
    pub sel: gtk::MultiSelection,
    grid: gtk::GridView,
    list: gtk::ColumnView,
    columns: RefCell<Vec<gtk::ColumnViewColumn>>,
    stack: gtk::Stack,
    empty: gtk::Box,
    status: gtk::Label,
    location: RefCell<gio::File>,
    history: RefCell<Vec<gio::File>>,
    future: RefCell<Vec<gio::File>>,
    query: Rc<RefCell<String>>,
    hidden: Rc<Cell<bool>>,
    typed: RefCell<(String, Instant)>,
    zoom: Cell<usize>,
    /// Bumped by each zoom, so an older zoom animation steps aside.
    zoom_gen: Cell<u64>,
    view: Cell<View>,
    found: gio::ListStore,
    hunt: RefCell<Option<gio::Cancellable>>,
    hunt_source: RefCell<Option<glib::SourceId>>,
    hunt_debounce: RefCell<Option<glib::SourceId>>,
    hunt_generation: Cell<u64>,
    search_cache: RefCell<Option<search::Cache>>,
    pub cut: Rc<RefCell<Vec<String>>>,
    pending: RefCell<Option<String>>,
    /// A just-created item to rename in place as soon as it shows up in the view.
    rename_new: RefCell<Option<String>>,
    /// A model change is waiting for the views to have taken it in (see `after_change`).
    changing: Cell<bool>,
    dragging: RefCell<Vec<gio::FileInfo>>,
}

fn position(item: &glib::WeakRef<glib::Object>) -> Option<u32> {
    let o = item.upgrade()?;
    let pos = if let Some(li) = o.downcast_ref::<gtk::ListItem>() { li.position() } else { o.downcast_ref::<gtk::ColumnViewCell>()?.position() };
    (pos != gtk::INVALID_LIST_POSITION).then_some(pos)
}

pub fn file_of(info: &gio::FileInfo) -> Option<gio::File> {
    info.attribute_object("standard::file").and_downcast::<gio::File>()
}

pub fn target_of(info: &gio::FileInfo) -> Option<gio::File> {
    match info.attribute_string("standard::target-uri") {
        Some(t) if !t.is_empty() => Some(gio::File::for_uri(&t)),
        _ => file_of(info),
    }
}

pub fn is_dir(info: &gio::FileInfo) -> bool {
    matches!(info.file_type(), gio::FileType::Directory | gio::FileType::Mountable)
}

pub fn kind(info: &gio::FileInfo) -> String {
    if is_dir(info) {
        return "Folder".into();
    }
    let ct = info.attribute_string("standard::fast-content-type").map(|s| s.to_string()).unwrap_or_default();
    if ct.is_empty() {
        return "File".into();
    }
    gio::content_type_get_description(&ct).to_string()
}

pub fn size_text(bytes: u64) -> String {
    glib::format_size(bytes).to_string()
}

pub fn when(t: &glib::DateTime) -> String {
    let Ok(now) = glib::DateTime::now_local() else { return String::new() };
    let Ok(t) = t.to_local() else { return String::new() };
    let fmt = if t.ymd() == now.ymd() {
        "Today %H:%M"
    } else if t.year() == now.year() {
        "%e %b %H:%M"
    } else {
        "%e %b %Y"
    };
    t.format(fmt).map(|s| s.trim().to_string()).unwrap_or_default()
}

fn key(info: &gio::FileInfo) -> String {
    if let Some(k) = info.attribute_string("digit::key") {
        return k.to_string();
    }
    let k = natural(&info.display_name());
    info.set_attribute_string("digit::key", &k);
    k
}

pub fn natural(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 16);
    let mut digits = String::new();
    let flush = |digits: &mut String, out: &mut String| {
        if !digits.is_empty() {
            let trimmed = digits.trim_start_matches('0');
            out.push_str(&format!("{:0>20}", if trimmed.is_empty() { "0" } else { trimmed }));
            digits.clear();
        }
    };
    for c in name.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
        } else {
            flush(&mut digits, &mut out);
            out.extend(c.to_lowercase());
        }
    }
    flush(&mut digits, &mut out);
    out
}

fn modified(info: &gio::FileInfo) -> i64 {
    info.modification_date_time().map(|d| d.to_unix()).unwrap_or(0)
}

fn order<T: Ord>(a: T, b: T) -> gtk::Ordering {
    a.cmp(&b).into()
}

fn infos(a: &glib::Object, b: &glib::Object) -> Option<(gio::FileInfo, gio::FileInfo)> {
    Some((a.clone().downcast::<gio::FileInfo>().ok()?, b.clone().downcast::<gio::FileInfo>().ok()?))
}

fn sorter(f: impl Fn(&gio::FileInfo, &gio::FileInfo) -> gtk::Ordering + 'static) -> gtk::CustomSorter {
    gtk::CustomSorter::new(move |a, b| match infos(a, b) {
        Some((a, b)) => f(&a, &b),
        None => gtk::Ordering::Equal,
    })
}

fn icon_for(info: &gio::FileInfo) -> gio::Icon {
    info.icon().unwrap_or_else(|| gio::ThemedIcon::new(if is_dir(info) { "folder" } else { "text-x-generic" }).upcast())
}

// Factory children are tagged in attach(); walk only realized widgets, never the
// model's offscreen items. A list tag covers its icon + name cell, not metadata.
fn item_widgets(view: &gtk::Widget) -> Vec<(u32, gtk::Widget)> {
    fn visit(widget: &gtk::Widget, out: &mut Vec<(u32, gtk::Widget)>) {
        if let Some(item) = kit::tag::get::<glib::WeakRef<glib::Object>>(widget, "digit-item") {
            if let Some(pos) = position(&item) {
                out.push((pos, widget.clone()));
            }
            return;
        }
        let mut child = widget.first_child();
        while let Some(w) = child {
            child = w.next_sibling();
            visit(&w, out);
        }
    }
    let mut out = Vec::new();
    visit(view, &mut out);
    out
}

fn stack_offset(index: usize) -> f64 {
    index.min(2) as f64 * 8.0
}

fn preview_regions(artwork: (f64, f64), name_height: f64) -> [DragRect; 2] {
    [((DRAG_CELL.0 - artwork.0) / 2.0, (DRAG_ICON - artwork.1) / 2.0, artwork.0, artwork.1),
     (6.0, DRAG_NAME_Y, DRAG_CELL.0 - 12.0, name_height)]
}

fn artwork_size(aspect: f64, size: f64) -> (f64, f64) {
    if !aspect.is_finite() || aspect <= 0.0 {
        (size, size)
    } else if aspect > 1.0 {
        (size, size / aspect)
    } else {
        (size * aspect, size)
    }
}

/// Preserve the relative press inside the artwork or name. Padding and list
/// metadata have no counterpart on the card: use the nearest content edge.
fn preview_grab(pointer: (f64, f64), source: [DragRect; 2], target: [DragRect; 2]) -> (f64, f64) {
    let nearest = |r: DragRect| (pointer.0.clamp(r.0, r.0 + r.2), pointer.1.clamp(r.1, r.1 + r.3));
    let distance = |r| {
        let p = nearest(r);
        (pointer.0 - p.0).powi(2) + (pointer.1 - p.1).powi(2)
    };
    let index = usize::from(distance(source[1]) < distance(source[0]));
    let (x, y, w, h) = source[index];
    let (tx, ty, tw, th) = target[index];
    let p = nearest(source[index]);
    (tx + (p.0 - x) / w.max(1.0) * tw, ty + (p.1 - y) / h.max(1.0) * th)
}

fn item_image(widget: &gtk::Widget) -> Option<gtk::Image> {
    let first = widget.first_child()?;
    first.clone().downcast::<gtk::Image>().ok()
        .or_else(|| first.first_child().and_downcast::<gtk::Image>())
}

#[derive(Debug)]
struct DragGeometry {
    size: (f64, f64),
    hotspot: (i32, i32),
    shift: (f64, f64),
}

/// Align outer bounds to device pixels; keep drawing and the hotspot logical.
fn drag_geometry(sizes: &[(f64, f64)], grab: (f64, f64), badge: Option<(f64, f64)>, scale: i32) -> DragGeometry {
    let scale = scale.max(1) as f64;
    let mut min = (0.0_f64.min(grab.0), 0.0_f64.min(grab.1));
    let mut max = (grab.0, grab.1);
    for (i, &(w, h)) in sizes.iter().take(3).enumerate() {
        let offset = stack_offset(i);
        max.0 = max.0.max(w + offset);
        max.1 = max.1.max(h + offset);
    }
    if let (Some(&(w, _)), Some((bw, bh))) = (sizes.first(), badge) {
        min.0 = min.0.min(w - bw);
        max.1 = max.1.max(bh);
    }
    min = ((min.0 * scale).floor() / scale, (min.1 * scale).floor() / scale);
    max = ((max.0 * scale).ceil() / scale, (max.1 * scale).ceil() / scale);
    let hotspot = ((grab.0 - min.0).round() as i32, (grab.1 - min.1).round() as i32);
    let shift = (hotspot.0 as f64 - grab.0, hotspot.1 as f64 - grab.1);
    // Fractional compensation can extend either edge by up to half a logical px.
    // Reserve a pixel around all contents so nothing is clipped.
    DragGeometry {
        size: ((max.0 - min.0 + 2.0).ceil(), (max.1 - min.1 + 2.0).ceil()),
        hotspot: (hotspot.0 + 1, hotspot.1 + 1),
        shift: (shift.0 + 1.0, shift.1 + 1.0),
    }
}

impl Pane {
    pub fn new(start: &gio::File) -> Rc<Pane> {
        let dir = crate::listing::Listing::new(ATTRS, start);
        let query: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
        let hidden = Rc::new(Cell::new(kit::prefs::flag(&kit::prefs::read(), "files_hidden", false)));
        let (q, h) = (query.clone(), hidden.clone());
        let filter = gtk::CustomFilter::new(move |o| {
            let Some(info) = o.downcast_ref::<gio::FileInfo>() else { return false };
            if !h.get() && (info.is_hidden() || info.is_backup()) {
                return false;
            }
            let q = q.borrow();
            q.is_empty() || search::matches(&info.display_name(), &q)
        });
        let filtered = gtk::FilterListModel::new(Some(dir.model().clone()), Some(filter.clone()));
        filtered.set_incremental(true);
        let folders = sorter(|a, b| order(!is_dir(a), !is_dir(b)));
        let multi = gtk::MultiSorter::new();
        multi.append(folders);
        let sorted = gtk::SortListModel::new(Some(filtered.clone()), Some(multi.clone()));
        sorted.set_incremental(true);
        let sel = gtk::MultiSelection::new(Some(sorted.clone()));
        let grid = gtk::GridView::new(Some(sel.clone()), None::<gtk::ListItemFactory>);
        grid.set_enable_rubberband(true);
        grid.set_max_columns(64);
        grid.set_min_columns(1);
        grid.add_css_class("grid");
        let list = gtk::ColumnView::new(Some(sel.clone()));
        list.set_enable_rubberband(true);
        list.set_show_column_separators(false);
        list.set_reorderable(false);
        list.add_css_class("list");
        let stack = gtk::Stack::new();
        stack.set_vexpand(true);
        let gs = gtk::ScrolledWindow::new();
        gs.set_child(Some(&grid));
        let ls = gtk::ScrolledWindow::new();
        ls.set_child(Some(&list));
        stack.add_named(&gs, Some("grid"));
        stack.add_named(&ls, Some("list"));
        let empty = gtk::Box::new(gtk::Orientation::Vertical, 8);
        empty.set_valign(gtk::Align::Center);
        empty.set_halign(gtk::Align::Center);
        empty.set_can_target(false);
        let et = kit::ui::sym("folder_open");
        et.add_css_class("empty-icon");
        empty.append(&et);
        let el = kit::ui::text("This folder is empty", "dim");
        el.set_halign(gtk::Align::Center);
        empty.append(&el);
        empty.set_visible(false);
        let over = gtk::Overlay::new();
        over.set_child(Some(&stack));
        over.add_overlay(&empty);
        let editor = gtk::Entry::new();
        editor.add_css_class("inline-rename");
        editor.set_halign(gtk::Align::Start);
        editor.set_valign(gtk::Align::Start);
        editor.set_width_chars(1);
        editor.set_visible(false);
        over.add_overlay(&editor);
        over.set_measure_overlay(&editor, false);
        over.set_clip_overlay(&editor, true);
        let status = kit::ui::text("", "status");
        status.set_ellipsize(pango::EllipsizeMode::End);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("pane");
        root.append(&over);
        root.append(&status);
        let p = Rc::new(Pane {
            root,
            rename_generation: Cell::new(0),
            rename_layer: over,
            editor,
            renaming: RefCell::new(None),
            rename_label: RefCell::new(None),
            dir,
            filter,
            filtered,
            sorted,
            sel,
            grid,
            list,
            columns: RefCell::new(Vec::new()),
            stack,
            empty,
            status,
            location: RefCell::new(start.clone()),
            history: RefCell::new(Vec::new()),
            future: RefCell::new(Vec::new()),
            query,
            hidden,
            typed: RefCell::new((String::new(), Instant::now())),
            zoom_gen: Cell::new(0),
            zoom: Cell::new(kit::prefs::number(&kit::prefs::read(), "files_zoom", 3.0).clamp(0.0, (SIZES.len() - 1) as f64) as usize),
            view: Cell::new(View::Grid),
            found: gio::ListStore::new::<gio::FileInfo>(),
            hunt: RefCell::new(None),
            hunt_source: RefCell::new(None),
            hunt_debounce: RefCell::new(None),
            hunt_generation: Cell::new(0),
            search_cache: RefCell::new(None),
            cut: Rc::new(RefCell::new(Vec::new())),
            pending: RefCell::new(None),
            rename_new: RefCell::new(None),
            changing: Cell::new(false),
            dragging: RefCell::new(Vec::new()),
        });
        let columns = p.build_columns();
        if let Some(cs) = p.list.sorter() {
            multi.append(cs);
        }
        if let Some(c) = columns.first() {
            p.list.sort_by_column(Some(c), gtk::SortType::Ascending);
        }
        *p.columns.borrow_mut() = columns;
        p.grid.set_factory(Some(&p.grid_factory()));
        p.drag_from(p.grid.upcast_ref());
        p.drag_from(p.list.upcast_ref());
        p.wire();
        let v = if kit::prefs::text(&kit::prefs::read(), "files_view", "grid") == "list" { View::List } else { View::Grid };
        p.set_view(v);
        p
    }

    fn weak(self: &Rc<Self>) -> Weak<Pane> {
        Rc::downgrade(self)
    }

    pub fn start_rename(self: &Rc<Self>, info: gio::FileInfo) {
        for pane in crate::panes() { pane.cancel_rename(); }
        let Some(file) = file_of(&info) else { return };
        let Some(pos) = (0..self.sorted.n_items()).find(|&i| self.sorted.item(i).and_downcast::<gio::FileInfo>().and_then(|i| file_of(&i)).is_some_and(|f| f.equal(&file))) else { return };
        // Reveal without a deferred row-focus request stealing the editor's focus.
        match self.view.get() {
            View::Grid => self.grid.scroll_to(pos, gtk::ListScrollFlags::empty(), None),
            View::List => self.list.scroll_to(pos, None::<&gtk::ColumnViewColumn>, gtk::ListScrollFlags::empty(), None),
        }
        self.editor.remove_css_class("error");
        self.editor.set_text(&info.display_name());
        *self.renaming.borrow_mut() = Some(info.clone());
        self.editor.set_visible(true);
        self.position_editor();
        self.editor.grab_focus();
        let (a, b) = rename_selection(&info.display_name(), is_dir(&info));
        self.editor.select_region(a, b);
        let me = self.weak();
        let generation = self.rename_generation.get();
        self.editor.add_tick_callback(move |_, _| {
            let Some(p) = me.upgrade() else { return glib::ControlFlow::Break };
            if p.rename_generation.get() != generation || p.renaming.borrow().is_none() { return glib::ControlFlow::Break; }
            p.position_editor();
            glib::ControlFlow::Continue
        });
    }

    fn cancel_rename(&self) {
        self.rename_generation.set(self.rename_generation.get().wrapping_add(1));
        self.renaming.borrow_mut().take();
        if let Some(label) = self.rename_label.borrow_mut().take().and_then(|w| w.upgrade()) { label.set_opacity(1.0); }
        self.editor.set_visible(false);
    }

    fn commit_rename(&self) {
        let info = self.renaming.borrow().clone();
        let Some(info) = info else { return };
        if ops::rename(&info, self.editor.text().as_str()) {
            self.cancel_rename();
        } else {
            self.editor.add_css_class("error");
            self.editor.grab_focus();
        }
    }

    fn position_editor(&self) {
        let Some(file) = self.renaming.borrow().as_ref().and_then(file_of) else { return };
        if let Some(label) = self.rename_label.borrow_mut().take().and_then(|w| w.upgrade()) { label.set_opacity(1.0); }
        let view: &gtk::Widget = match self.view.get() { View::Grid => self.grid.upcast_ref(), View::List => self.list.upcast_ref() };
        self.editor.set_opacity(0.0);
        self.editor.set_can_target(false);
        for (pos, widget) in item_widgets(view) {
            let matches = self.sorted.item(pos).and_downcast::<gio::FileInfo>().and_then(|i| file_of(&i)).is_some_and(|f| f.equal(&file));
            if !matches { continue; }
            let Some(label) = widget.last_child() else { continue };
            let Some(bounds) = label.compute_bounds(&self.rename_layer) else { continue };
            let Some(scroll) = view.ancestor(gtk::ScrolledWindow::static_type()) else { continue };
            let Some(clip) = scroll.compute_bounds(&self.rename_layer) else { continue };
            // Keep the persistent editor focused offscreen, without painting over
            // adjacent rows or the status line. Recycling never owns its text.
            if bounds.y() < clip.y() || bounds.y() + bounds.height() > clip.y() + clip.height()
                || bounds.x() < clip.x() || bounds.x() + bounds.width() > clip.x() + clip.width() { break; }
            self.editor.set_margin_start(bounds.x().round().max(0.0) as i32);
            self.editor.set_margin_top(bounds.y().round().max(0.0) as i32);
            self.editor.set_size_request(bounds.width().round() as i32, bounds.height().round() as i32);
            gtk::prelude::EntryExt::set_alignment(&self.editor, if self.view.get() == View::Grid { 0.5 } else { 0.0 });
            if self.view.get() == View::Grid { self.editor.add_css_class("grid-name"); } else { self.editor.remove_css_class("grid-name"); }
            self.editor.set_opacity(1.0);
            self.editor.set_can_target(true);
            label.set_opacity(0.0);
            *self.rename_label.borrow_mut() = Some(label.downgrade());
            break;
        }
    }

    /// Where the first selected item's picture is, in `target`'s coordinates, while it shows in
    /// the view: for Quick Look to grow out of it and back into it.
    pub fn selected_spot(&self, target: &impl IsA<gtk::Widget>) -> Option<gtk::graphene::Rect> {
        if !self.root.is_mapped() { return None; }
        let first = self.selected_position()?;
        let view: &gtk::Widget = match self.view.get() { View::Grid => self.grid.upcast_ref(), View::List => self.list.upcast_ref() };
        let scroll = view.ancestor(gtk::ScrolledWindow::static_type())?;
        let (_, widget) = item_widgets(view).into_iter().find(|(pos, _)| *pos == first)?;
        let image = item_image(&widget).filter(|i| i.is_mapped())?;
        // Only while it shows: scrolled out of the view, there's nothing to grow from.
        let seen = image.compute_bounds(&scroll)?;
        if seen.x() + seen.width() <= 0.0 || seen.y() + seen.height() <= 0.0 || seen.x() >= scroll.width() as f32 || seen.y() >= scroll.height() as f32 { return None; }
        image.compute_bounds(target)
    }

    /// Where the first selected item is in the view's order.
    pub fn selected_position(&self) -> Option<u32> {
        gtk::BitsetIter::init_first(&self.sel.selection()).map(|(_, first)| first)
    }

    pub fn trash_flights(&self, files: &[gio::File]) -> Vec<TrashFlight> {
        if !self.root.is_mapped() { return Vec::new(); }
        let Some(window) = self.root.root().and_downcast::<gtk::Window>() else { return Vec::new() };
        let view: &gtk::Widget = match self.view.get() { View::Grid => self.grid.upcast_ref(), View::List => self.list.upcast_ref() };
        let mut out = Vec::new();
        for (pos, widget) in item_widgets(view) {
            let Some(info) = self.sorted.item(pos).and_downcast::<gio::FileInfo>() else { continue };
            let Some(file) = file_of(&info) else { continue };
            if !files.iter().any(|f| f.equal(&file)) { continue; }
            let Some(image) = item_image(&widget) else { continue };
            let Some(scroll) = view.ancestor(gtk::ScrolledWindow::static_type()) else { continue };
            let center = gtk::graphene::Point::new(image.width() as f32 / 2.0, image.height() as f32 / 2.0);
            let Some(visible) = image.compute_point(&scroll, &center) else { continue };
            if !image.is_mapped() || visible.x() < 0.0 || visible.y() < 0.0 || visible.x() >= scroll.width() as f32 || visible.y() >= scroll.height() as f32 { continue; }
            let Some(point) = image.compute_point(&window, &center) else { continue };
            // Folders show Digit's own folder art; tell the shell which one so it flies as drawn.
            let icon = if info.file_type() == gio::FileType::Directory {
                format!("digit-folder:{}", kit::folder::glyph(&file))
            } else {
                let icon = thumbs::icon(&image).or_else(|| image.gicon()).unwrap_or_else(|| icon_for(&info));
                let Some(icon) = icon.to_string() else { continue };
                icon.to_string()
            };
            out.push(TrashFlight { uri: file.uri().to_string(), x: point.x(), y: point.y(), size: image.pixel_size().max(1), icon });
            if out.len() == 3 { break; }
        }
        out
    }

    fn grid_factory(self: &Rc<Self>) -> gtk::SignalListItemFactory {
        let f = gtk::SignalListItemFactory::new();
        let me = self.weak();
        f.connect_setup(move |_, o| {
            let Some(item) = o.downcast_ref::<gtk::ListItem>() else { return };
            let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
            b.add_css_class("cell");
            b.set_valign(gtk::Align::Start);
            let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
            frame.add_css_class("thumbbox");
            frame.set_halign(gtk::Align::Center);
            let img = gtk::Image::new();
            img.add_css_class("thumb");
            img.set_halign(gtk::Align::Center);
            img.set_valign(gtk::Align::Center);
            img.set_vexpand(true);
            frame.append(&img);
            let l = gtk::Label::new(None);
            l.add_css_class("fname");
            l.set_halign(gtk::Align::Center);
            l.set_wrap(true);
            l.set_wrap_mode(pango::WrapMode::WordChar);
            l.set_lines(2);
            l.set_ellipsize(pango::EllipsizeMode::End);
            l.set_justify(gtk::Justification::Center);
            l.set_max_width_chars(16);
            l.set_width_chars(10);
            b.append(&frame);
            b.append(&l);
            item.set_child(Some(&b));
            if let Some(p) = me.upgrade() {
                p.attach(&b, item.clone().upcast::<glib::Object>().downgrade());
            }
        });
        let me = self.weak();
        f.connect_bind(move |_, o| {
            let Some(item) = o.downcast_ref::<gtk::ListItem>() else { return };
            let (Some(b), Some(info)) = (item.child(), item.item().and_downcast::<gio::FileInfo>()) else { return };
            let Some(frame) = b.first_child() else { return };
            let Some(img) = frame.first_child().and_downcast::<gtk::Image>() else { return };
            let Some(l) = b.last_child().and_downcast::<gtk::Label>() else { return };
            let px = me.upgrade().map(|p| SIZES[p.zoom.get()]).unwrap_or(72);
            let room = px + (px / 4).max(12);
            frame.set_size_request(room, room);
            match is_dir(&info).then(|| target_of(&info)).flatten() {
                Some(dir) => crate::art::set(&img, &dir, px),
                None => {
                    img.set_pixel_size(px);
                    img.set_from_gicon(&icon_for(&info));
                }
            }
            l.set_text(&info.display_name());
            let opacity = me.upgrade().map_or(1.0, |p| p.item_opacity(&info));
            b.set_opacity(opacity);
            if px >= 56 {
                thumbs::load(&img, &info, px);
            }
        });
        f.connect_unbind(|_, o| {
            if let Some(img) = o.downcast_ref::<gtk::ListItem>().and_then(|i| i.child()).and_then(|b| b.first_child()).and_then(|f| f.first_child()).and_downcast::<gtk::Image>() {
                thumbs::forget(&img);
            }
        });
        f
    }

    fn build_columns(self: &Rc<Self>) -> Vec<gtk::ColumnViewColumn> {
        let mut out = Vec::new();
        let specs: [(&str, i32); 4] = [("Name", 0), ("Size", 1), ("Modified", 2), ("Kind", 3)];
        for (title, which) in specs {
            let f = gtk::SignalListItemFactory::new();
            let me = self.weak();
            f.connect_setup(move |_, o| {
                let Some(cell) = o.downcast_ref::<gtk::ColumnViewCell>() else { return };
                let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                if which == 0 {
                    b.add_css_class("namecell");
                    let img = gtk::Image::new();
                    img.set_pixel_size(22);
                    b.append(&img);
                }
                let l = gtk::Label::new(None);
                l.set_xalign(0.0);
                l.set_ellipsize(pango::EllipsizeMode::End);
                if which != 0 {
                    l.add_css_class("dim");
                }
                if which == 1 {
                    l.add_css_class("mono");
                }
                b.append(&l);
                cell.set_child(Some(&b));
                if which == 0 {
                    if let Some(p) = me.upgrade() {
                        p.attach(&b, cell.clone().upcast::<glib::Object>().downgrade());
                    }
                }
            });
            let me = self.weak();
            f.connect_bind(move |_, o| {
                let Some(cell) = o.downcast_ref::<gtk::ColumnViewCell>() else { return };
                let (Some(b), Some(info)) = (cell.child(), cell.item().and_downcast::<gio::FileInfo>()) else { return };
                let Some(l) = b.last_child().and_downcast::<gtk::Label>() else { return };
                match which {
                    0 => {
                        if let Some(img) = b.first_child().and_downcast::<gtk::Image>() {
                            match is_dir(&info).then(|| target_of(&info)).flatten() {
                                Some(dir) => crate::art::set(&img, &dir, 22),
                                None => {
                                    img.set_from_gicon(&icon_for(&info));
                                    img.set_pixel_size(22);
                                }
                            }
                        }
                        l.set_text(&info.display_name());
                        let opacity = me.upgrade().map_or(1.0, |p| p.item_opacity(&info));
                        b.set_opacity(opacity);
                    }
                    1 => l.set_text(&if is_dir(&info) { "—".to_string() } else { size_text(info.size().max(0) as u64) }),
                    2 => l.set_text(&info.modification_date_time().map(|d| when(&d)).unwrap_or_default()),
                    _ => l.set_text(&kind(&info)),
                }
            });
            let col = gtk::ColumnViewColumn::new(Some(title), Some(f));
            col.set_resizable(true);
            match which {
                0 => {
                    col.set_expand(true);
                    col.set_sorter(Some(&sorter(|a, b| order(key(a), key(b)))));
                }
                1 => {
                    col.set_fixed_width(110);
                    col.set_sorter(Some(&sorter(|a, b| order(a.size(), b.size()))));
                }
                2 => {
                    col.set_fixed_width(170);
                    col.set_sorter(Some(&sorter(|a, b| order(modified(a), modified(b)))));
                }
                _ => {
                    col.set_fixed_width(190);
                    col.set_sorter(Some(&sorter(|a, b| order(kind(a), kind(b)))));
                }
            }
            self.list.append_column(&col);
            out.push(col);
        }
        out
    }

    fn item_at(view: &gtk::Widget, x: f64, y: f64) -> Option<u32> {
        fn tagged(w: &gtk::Widget) -> Option<u32> {
            let item = kit::tag::get::<glib::WeakRef<glib::Object>>(w, "digit-item")?;
            position(&item)
        }
        fn search(w: &gtk::Widget, depth: u32) -> Option<u32> {
            if let Some(pos) = tagged(w) {
                return Some(pos);
            }
            if depth > 6 {
                return None;
            }
            let mut c = w.first_child();
            while let Some(child) = c {
                if let Some(pos) = search(&child, depth + 1) {
                    return Some(pos);
                }
                c = child.next_sibling();
            }
            None
        }
        let mut cur = view.pick(x, y, gtk::PickFlags::DEFAULT);
        while let Some(w) = cur {
            if &w == view {
                return None;
            }
            if let Some(pos) = tagged(&w) {
                return Some(pos);
            }
            if matches!(w.css_name().as_str(), "child" | "row") && w.parent().is_some_and(|p| matches!(p.css_name().as_str(), "gridview" | "listview")) {
                return search(&w, 0);
            }
            cur = w.parent();
        }
        None
    }

    fn drag_from(self: &Rc<Self>, view: &gtk::Widget) {
        let held = gtk::GestureDrag::new();
        held.set_button(gdk::BUTTON_PRIMARY);
        held.set_propagation_phase(gtk::PropagationPhase::Capture);
        let on_item = Rc::new(std::cell::Cell::new(false));
        let v = view.clone();
        let flag = on_item.clone();
        held.connect_drag_begin(move |_, x, y| flag.set(Pane::item_at(&v, x, y).is_some()));
        let v = view.clone();
        let flag = on_item.clone();
        held.connect_drag_update(move |g, _, _| {
            if !flag.replace(false) {
                return;
            }
            let mut hosts = vec![v.clone()];
            let mut c = v.first_child();
            while let Some(w) = c {
                if w.css_name() == "listview" {
                    hosts.push(w.clone());
                }
                c = w.next_sibling();
            }
            for host in hosts {
                let controllers = host.observe_controllers();
                for i in 0..controllers.n_items() {
                    if let Some(rubber) = controllers.item(i).and_downcast::<gtk::GestureDrag>() {
                        if rubber != *g {
                            rubber.set_state(gtk::EventSequenceState::Denied);
                        }
                    }
                }
            }
        });
        view.add_controller(held);
        let drag = gtk::DragSource::new();
        drag.set_propagation_phase(gtk::PropagationPhase::Capture);
        drag.set_actions(gdk::DragAction::COPY | gdk::DragAction::MOVE | gdk::DragAction::LINK);
        let prepared = Rc::new(Cell::new(None));
        let preview = prepared.clone();
        let me = self.weak();
        let v = view.clone();
        drag.connect_prepare(move |_, x, y| {
            let p = me.upgrade()?;
            let pos = Pane::item_at(&v, x, y)?;
            if !p.sel.is_selected(pos) {
                p.sel.select_item(pos, true);
            }
            let files = p.selected_files();
            if files.is_empty() {
                return None;
            }
            // Freeze artwork and text at desktop size before dimming the source.
            preview.set(Some(p.drag_preview(&v, pos, (x, y))?));
            Some(gdk::ContentProvider::for_value(&gdk::FileList::from_array(&files).to_value()))
        });
        let me = self.weak();
        drag.connect_drag_begin(move |_, system| {
            let Some(p) = me.upgrade() else { return };
            if let Some((preview, hot)) = prepared.take() {
                let _ = kit::drag::set_icon(system, &preview, hot);
            }
            let selected = p.selected();
            *p.dragging.borrow_mut() = selected;
            p.sync_drag_opacity();
        });
        let me = self.weak();
        drag.connect_drag_end(move |_, _, _| {
            let Some(p) = me.upgrade() else { return };
            p.dragging.borrow_mut().clear();
            p.sync_drag_opacity();
        });
        view.add_controller(drag);
    }

    fn item_opacity(&self, info: &gio::FileInfo) -> f64 {
        let uri = file_of(info).map(|f| f.uri().to_string());
        let cut = uri.is_some_and(|uri| self.cut.borrow().contains(&uri));
        let lifted = self.dragging.borrow().contains(info);
        let base = if info.is_hidden() || cut { 0.55 } else { 1.0 };
        base * if lifted { 0.4 } else { 1.0 }
    }

    fn sync_drag_opacity(&self) {
        for view in [self.grid.upcast_ref(), self.list.upcast_ref()] {
            for (pos, widget) in item_widgets(view) {
                if let Some(info) = self.sorted.item(pos).and_downcast::<gio::FileInfo>() {
                    let opacity = self.item_opacity(&info);
                    widget.set_opacity(opacity);
                }
            }
        }
    }

    fn drag_preview(&self, view: &gtk::Widget, anchor: u32, pointer: (f64, f64)) -> Option<(gdk::Paintable, (i32, i32))> {
        let widgets = item_widgets(view);
        let (_, front) = widgets.iter().find(|(pos, _)| *pos == anchor)?;
        let image = item_image(front)?;
        let bounds = image.compute_bounds(view)?;
        let px = image.pixel_size().max(1) as f64;
        let name = front.last_child()?.compute_bounds(view)?;
        let mut entries = vec![(anchor, front.clone())];
        entries.extend(widgets.iter().filter(|(pos, w)| *pos != anchor && self.sel.is_selected(*pos) && w.is_mapped()).take(2).cloned());
        let theme = gtk::IconTheme::for_display(&view.display());
        let mut cards = Vec::new();
        for (pos, widget) in entries {
            let info = self.sorted.item(pos).and_downcast::<gio::FileInfo>()?;
            let image = item_image(&widget)?;
            // Reuse loaded thumbnails. Request folders and themed icons at 64 px
            // as list artwork may only have been loaded at 22 px.
            let folder = is_dir(&info).then(|| target_of(&info)).flatten();
            let icon = folder.as_ref()
                .and_then(|f| crate::art::folder(kit::folder::glyph(f), DRAG_ICON as i32 * view.scale_factor()))
                .map(|t| t.upcast::<gdk::Paintable>())
                .or_else(|| folder.is_none().then(|| image.paintable()).flatten())
                .unwrap_or_else(|| theme.lookup_by_gicon(&icon_for(&info), DRAG_ICON as i32,
                    view.scale_factor(), view.direction(), gtk::IconLookupFlags::empty()).upcast());
            let label = view.create_pango_layout(Some(&info.display_name()));
            let mut font = view.pango_context().font_description().unwrap_or_default();
            font.set_absolute_size(13.0 * pango::SCALE as f64);
            font.set_weight(pango::Weight::Medium);
            font.set_variations(Some("wght=520"));
            label.set_font_description(Some(&font));
            label.set_width((DRAG_CELL.0 as i32 - 12) * pango::SCALE);
            label.set_height(-2);
            label.set_wrap(pango::WrapMode::WordChar);
            label.set_ellipsize(pango::EllipsizeMode::End);
            label.set_alignment(pango::Alignment::Center);
            cards.push((icon.current_image(), label));
        }
        let aspect = cards[0].0.intrinsic_aspect_ratio();
        let (w, h) = artwork_size(aspect, px);
        let artwork = (bounds.x() as f64 + (bounds.width() as f64 - w) / 2.0,
                       bounds.y() as f64 + (bounds.height() as f64 - h) / 2.0, w, h);
        let target = preview_regions(artwork_size(aspect, DRAG_ICON), cards[0].1.pixel_size().1 as f64);
        let grab = preview_grab(pointer, [artwork, (name.x() as f64, name.y() as f64,
            name.width() as f64, name.height() as f64)], target);
        let count = self.sel.selection().size();
        let sizes = vec![DRAG_CELL; cards.len()];
        let badge = (count > 1).then(|| {
            let layout = view.create_pango_layout(Some(&count.to_string()));
            layout.set_font_description(Some(&pango::FontDescription::from_string("Readex Pro Medium 10")));
            layout
        });
        let badge_size = badge.as_ref().map(|l| {
            let (w, h) = l.pixel_size();
            ((w + 12) as f64, (h + 6) as f64)
        });
        let geometry = drag_geometry(&sizes, grab, badge_size, view.scale_factor());
        let snapshot = gtk::Snapshot::new();
        // Resolve the same installed selection colors as the source cells.
        #[allow(deprecated)]
        let color = |name| view.style_context().lookup_color(name).unwrap_or_else(|| view.color());
        let bg = color("accent_bg_color");
        let fg = color("accent_fg_color");
        let round = kit::style::look().round;
        // GDK hotspots use logical, integer coordinates, even on scaled outputs.
        // Compensate the fractional grab in the contents, rather than rounding it
        // away. GTK/GDK scales the vector snapshot and its loaded textures.
        snapshot.translate(&gtk::graphene::Point::new(geometry.shift.0 as f32, geometry.shift.1 as f32));
        for (i, (icon, label)) in cards.iter().enumerate().rev() {
            let offset = stack_offset(i);
            snapshot.save();
            snapshot.translate(&gtk::graphene::Point::new(offset as f32, offset as f32));
            // The artwork and name keep their own sizes; never scale a whole
            // grid cell or a wide list row into the drag surface.
            snapshot.save();
            let (w, h) = artwork_size(icon.intrinsic_aspect_ratio(), DRAG_ICON);
            let art = preview_regions((w, h), 0.0)[0];
            snapshot.translate(&gtk::graphene::Point::new(art.0 as f32, art.1 as f32));
            icon.snapshot(&snapshot, w, h);
            snapshot.restore();
            let (_, text) = label.pixel_extents();
            let rect = gtk::graphene::Rect::new(text.x() as f32, DRAG_NAME_Y as f32 - 1.0,
                text.width() as f32 + 12.0, text.height() as f32 + 2.0);
            snapshot.push_rounded_clip(&gtk::gsk::RoundedRect::from_rect(rect, (6.0 * round) as f32));
            snapshot.append_color(&bg, &rect);
            snapshot.pop();
            snapshot.translate(&gtk::graphene::Point::new(6.0, DRAG_NAME_Y as f32));
            snapshot.append_layout(label, &fg);
            snapshot.restore();
        }
        if let (Some(layout), Some((w, h))) = (badge, badge_size) {
            let rect = gtk::graphene::Rect::new((sizes[0].0 - w) as f32, 0.0, w as f32, h as f32);
            snapshot.push_rounded_clip(&gtk::gsk::RoundedRect::from_rect(rect, h as f32 / 2.0));
            snapshot.append_color(&bg, &rect);
            snapshot.pop();
            snapshot.translate(&gtk::graphene::Point::new(rect.x() + 6.0, 3.0));
            snapshot.append_layout(&layout, &fg);
        }
        snapshot.to_paintable(Some(&gtk::graphene::Size::new(geometry.size.0 as f32, geometry.size.1 as f32)))
            .map(|image| (image, geometry.hotspot))
    }

    fn attach(self: &Rc<Self>, w: &gtk::Box, item: glib::WeakRef<glib::Object>) {
        let click = gtk::GestureClick::new();
        click.set_button(0);
        let me = self.weak();
        let it = item.clone();
        click.connect_pressed(move |g, n, x, y| {
            let Some(p) = me.upgrade() else { return };
            let Some(pos) = position(&it) else { return };
            let button = g.current_button();
            if button == 3 {
                g.set_state(gtk::EventSequenceState::Claimed);
                if !p.sel.is_selected(pos) {
                    p.sel.select_item(pos, true);
                }
                if let Some(w) = g.widget() {
                    let (px, py) = w.compute_point(&p.root, &gtk::graphene::Point::new(x as f32, y as f32)).map(|pt| (pt.x() as f64, pt.y() as f64)).unwrap_or((x, y));
                    crate::menu(&p, Some((px, py)), true);
                }
            } else if button == 2 && n == 1 {
                g.set_state(gtk::EventSequenceState::Claimed);
                if let Some(info) = p.sorted.item(pos).and_downcast::<gio::FileInfo>() {
                    if is_dir(&info) {
                        if let Some(f) = target_of(&info) {
                            crate::new_tab(&f);
                        }
                    }
                }
            }
        });
        w.add_controller(click);
        kit::tag::set(w, "digit-item", item.clone());
        let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY | gdk::DragAction::MOVE);
        let me = self.weak();
        let it = item.clone();
        drop.connect_accept(move |_, _| {
            let Some(p) = me.upgrade() else { return false };
            position(&it).and_then(|pos| p.sorted.item(pos).and_downcast::<gio::FileInfo>()).is_some_and(|i| is_dir(&i))
        });
        let me = self.weak();
        drop.connect_drop(move |t, value, _, _| {
            let Some(p) = me.upgrade() else { return false };
            let Ok(list) = value.get::<gdk::FileList>() else { return false };
            let Some(dest) = position(&item).and_then(|pos| p.sorted.item(pos).and_downcast::<gio::FileInfo>()).filter(is_dir).and_then(|i| target_of(&i)) else { return false };
            let files: Vec<gio::File> = list.files().into_iter().filter(|f| !f.equal(&dest)).collect();
            if files.is_empty() {
                return false;
            }
            ops::dropped(files, &dest, t.current_event_state().contains(gdk::ModifierType::CONTROL_MASK));
            true
        });
        w.add_controller(drop);
    }

    fn wire(self: &Rc<Self>) {
        let me = self.weak();
        self.editor.connect_changed(|entry| entry.remove_css_class("error"));
        self.editor.connect_activate(move |_| {
            if let Some(p) = me.upgrade() { p.commit_rename(); }
        });
        let keys = gtk::EventControllerKey::new();
        let me = self.weak();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                if let Some(p) = me.upgrade() { p.cancel_rename(); }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.editor.add_controller(keys);
        let focus = gtk::EventControllerFocus::new();
        let me = self.weak();
        focus.connect_leave(move |_| {
            let Some(p) = me.upgrade() else { return };
            let generation = p.rename_generation.get();
            let me = me.clone();
            glib::idle_add_local_once(move || {
                if let Some(p) = me.upgrade() && p.rename_generation.get() == generation { p.commit_rename(); }
            });
        });
        self.editor.add_controller(focus);
        let me = self.weak();
        self.sorted.connect_items_changed(move |_, _, _, _| {
            let me = me.clone();
            glib::idle_add_local_once(move || {
                let Some(p) = me.upgrade() else { return };
                let file = p.renaming.borrow().as_ref().and_then(file_of);
                let generation = p.rename_generation.get();
                if let Some(file) = file {
                    glib::spawn_future_local(async move {
                        if let Err(e) = file.query_info_future("standard::name", gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS, glib::Priority::DEFAULT).await {
                            if e.matches(gio::IOErrorEnum::NotFound) && p.rename_generation.get() == generation && p.renaming.borrow().as_ref().and_then(file_of).is_some_and(|f| f.equal(&file)) { p.cancel_rename(); }
                        }
                    });
                }
            });
        });
        let me = self.weak();
        self.grid.connect_activate(move |_, pos| {
            if let Some(p) = me.upgrade() {
                p.activate(pos);
            }
        });
        let me = self.weak();
        self.list.connect_activate(move |_, pos| {
            if let Some(p) = me.upgrade() {
                p.activate(pos);
            }
        });
        // Incremental filtering/sorting can settle after DirectoryList finishes.
        // Keep a pending reveal until both models have processed the directory.
        let me = self.weak();
        self.dir.connect_loading_notify(move || {
            if let Some(p) = me.upgrade() {
                p.after_change();
            }
        });
        let me = self.weak();
        self.sorted.connect_items_changed(move |_, _, _, _| {
            if let Some(p) = me.upgrade() {
                p.after_change();
            }
        });
        let me = self.weak();
        self.filtered.connect_pending_notify(move |_| {
            if let Some(p) = me.upgrade() {
                p.after_change();
            }
        });
        let me = self.weak();
        self.sorted.connect_pending_notify(move |_| {
            if let Some(p) = me.upgrade() {
                p.after_change();
            }
        });
        let me = self.weak();
        self.sel.connect_selection_changed(move |_, _, _| {
            if let Some(p) = me.upgrade() {
                p.sync_status();
                crate::selection_changed(&p);
            }
        });
        // In split view the side under the pointer becomes the one being worked in.
        let hover = gtk::EventControllerMotion::new();
        let me = self.weak();
        hover.connect_enter(move |c, _, _| {
            let Some(p) = me.upgrade() else { return };
            crate::focus_pane(&p);
            // Take keyboard focus too, unless something is being typed into.
            let typing = c.widget().and_then(|w| w.root()).and_then(|r| r.focus()).is_some_and(|f| f.is::<gtk::Text>() || f.ancestor(gtk::Text::static_type()).is_some());
            if !typing {
                p.focus();
            }
        });
        self.root.add_controller(hover);
        for w in [self.grid.clone().upcast::<gtk::Widget>(), self.list.clone().upcast::<gtk::Widget>()] {
            let click = gtk::GestureClick::new();
            click.set_button(0);
            let me = self.weak();
            click.connect_pressed(move |g, _, x, y| {
                let Some(p) = me.upgrade() else { return };
                crate::focus_pane(&p);
                match g.current_button() {
                    3 => {
                        p.sel.unselect_all();
                        let (px, py) = g.widget().and_then(|w| w.compute_point(&p.root, &gtk::graphene::Point::new(x as f32, y as f32))).map(|pt| (pt.x() as f64, pt.y() as f64)).unwrap_or((x, y));
                        crate::menu(&p, Some((px, py)), false);
                    }
                    1 => {
                        let held = g.current_event_state().intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK);
                        if !held && g.widget().is_some_and(|w| Pane::item_at(&w, x, y).is_none()) {
                            p.sel.unselect_all();
                        }
                    }
                    8 => p.back(),
                    9 => p.forward(),
                    _ => {}
                }
            });
            w.add_controller(click);
            let keys = gtk::EventControllerKey::new();
            let me = self.weak();
            keys.connect_key_pressed(move |_, key, _, mods| {
                let Some(p) = me.upgrade() else { return glib::Propagation::Proceed };
                if p.key(key, mods) { glib::Propagation::Stop } else { glib::Propagation::Proceed }
            });
            w.add_controller(keys);
            let look = gtk::EventControllerKey::new();
            look.set_propagation_phase(gtk::PropagationPhase::Capture);
            let me = self.weak();
            look.connect_key_pressed(move |_, key, _, mods| {
                let Some(p) = me.upgrade() else { return glib::Propagation::Proceed };
                if p.renaming.borrow().is_some() { return glib::Propagation::Proceed; }
                let plain = !mods.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK | gdk::ModifierType::SHIFT_MASK);
                if key == gdk::Key::space && plain && p.typed.borrow().0.is_empty() {
                    crate::look::toggle();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            w.add_controller(look);
            let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY | gdk::DragAction::MOVE);
            let me = self.weak();
            drop.connect_drop(move |t, value, _, _| {
                let Some(p) = me.upgrade() else { return false };
                let Ok(list) = value.get::<gdk::FileList>() else { return false };
                let dest = p.location();
                let files: Vec<gio::File> = list.files().into_iter().filter(|f| f.parent().is_none_or(|d| !d.equal(&dest))).collect();
                if files.is_empty() {
                    return false;
                }
                ops::dropped(files, &dest, t.current_event_state().contains(gdk::ModifierType::CONTROL_MASK));
                true
            });
            w.add_controller(drop);
            let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
            wheel.set_propagation_phase(gtk::PropagationPhase::Capture);
            let me = self.weak();
            wheel.connect_scroll(move |c, _, dy| {
                let Some(p) = me.upgrade() else { return glib::Propagation::Proceed };
                if !c.current_event_state().contains(gdk::ModifierType::CONTROL_MASK) {
                    return glib::Propagation::Proceed;
                }
                p.zoom_by(if dy < 0.0 { 1 } else { -1 });
                glib::Propagation::Stop
            });
            w.add_controller(wheel);
            let focus = gtk::EventControllerFocus::new();
            let me = self.weak();
            focus.connect_enter(move |_| {
                if let Some(p) = me.upgrade() {
                    crate::focus_pane(&p);
                }
            });
            w.add_controller(focus);
        }
    }

    fn key(self: &Rc<Self>, key: gdk::Key, mods: gdk::ModifierType) -> bool {
        if self.renaming.borrow().is_some() { return false; }
        let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
        let alt = mods.contains(gdk::ModifierType::ALT_MASK);
        match key {
            gdk::Key::BackSpace if !ctrl => {
                if !self.typed.borrow().0.is_empty() && self.typed.borrow().1.elapsed() < Duration::from_millis(1200) {
                    let mut t = self.typed.borrow_mut();
                    t.0.pop();
                    t.1 = Instant::now();
                    return true;
                }
                self.up();
                true
            }
            gdk::Key::space if !ctrl && !alt && self.typed.borrow().0.is_empty() => {
                crate::look::toggle();
                true
            }
            gdk::Key::Menu => {
                crate::menu(self, None, self.sel.selection().size() > 0);
                true
            }
            _ => {
                if ctrl || alt {
                    return false;
                }
                let Some(c) = key.to_unicode().filter(|c| !c.is_control()) else { return false };
                if c == ' ' && self.typed.borrow().0.is_empty() {
                    return false;
                }
                let prefix = {
                    let mut t = self.typed.borrow_mut();
                    if t.1.elapsed() > Duration::from_millis(1200) {
                        t.0.clear();
                    }
                    t.0.extend(c.to_lowercase());
                    t.1 = Instant::now();
                    t.0.clone()
                };
                self.jump(&prefix);
                true
            }
        }
    }

    fn jump(&self, prefix: &str) {
        let n = self.sorted.n_items();
        let hit = (0..n).find(|i| self.sorted.item(*i).and_downcast::<gio::FileInfo>().is_some_and(|f| f.display_name().to_lowercase().starts_with(prefix)));
        let hit = hit.or_else(|| (0..n).find(|i| self.sorted.item(*i).and_downcast::<gio::FileInfo>().is_some_and(|f| f.display_name().to_lowercase().contains(prefix))));
        if let Some(pos) = hit {
            self.reveal(pos);
        }
    }

    pub fn reveal(&self, pos: u32) {
        let flags = gtk::ListScrollFlags::FOCUS | gtk::ListScrollFlags::SELECT;
        match self.view.get() {
            View::Grid => self.grid.scroll_to(pos, flags, None),
            View::List => self.list.scroll_to(pos, None::<&gtk::ColumnViewColumn>, flags, None),
        }
    }

    /// Selects the item called `name` and starts renaming it in place, once the directory
    /// monitor has listed it (a folder just made by New Folder).
    pub fn rename_when_listed(self: &Rc<Self>, name: &str) {
        *self.rename_new.borrow_mut() = Some(name.to_string());
        self.try_rename_new();
    }

    fn try_rename_new(self: &Rc<Self>) {
        let Some(name) = self.rename_new.borrow().clone() else { return };
        let n = self.sorted.n_items();
        let Some(pos) = (0..n).find(|i| self.sorted.item(*i).and_downcast::<gio::FileInfo>().is_some_and(|f| f.name().to_str() == Some(name.as_str()))) else { return };
        self.rename_new.borrow_mut().take();
        self.reveal(pos);
        let Some(info) = self.sorted.item(pos).and_downcast::<gio::FileInfo>() else { return };
        // After the view has made and focused the new row, so the editor can sit over its name.
        let me = self.weak();
        glib::idle_add_local_once(move || {
            if let Some(p) = me.upgrade() {
                p.start_rename(info);
            }
        });
    }

    pub fn select_name(&self, name: &str) {
        let n = self.sorted.n_items();
        match (0..n).find(|i| self.sorted.item(*i).and_downcast::<gio::FileInfo>().is_some_and(|f| f.name().to_str() == Some(name))) {
            Some(pos) => self.reveal(pos),
            None => *self.pending.borrow_mut() = Some(name.to_string()),
        }
    }

    /// Follows a change of the listing or its models, once the views have taken it in: never from
    /// inside the models' signals, where scrolling a view, selecting in it or swapping it for the
    /// empty folder's page upsets GTK's lists, which aborts.
    fn after_change(self: &Rc<Self>) {
        if self.changing.replace(true) {
            return;
        }
        let me = self.weak();
        glib::idle_add_local_full(glib::Priority::HIGH_IDLE, move || {
            let Some(p) = me.upgrade() else { return glib::ControlFlow::Break };
            p.changing.set(false);
            p.sync_empty();
            p.sync_status();
            if p.rename_new.borrow().is_some() {
                p.try_rename_new();
            }
            if !p.dir.is_loading() {
                p.settled();
            }
            glib::ControlFlow::Break
        });
    }

    fn settled(&self) {
        if self.filtered.pending() > 0 || self.sorted.pending() > 0 {
            return;
        }
        let pending = self.pending.borrow_mut().take();
        if let Some(name) = pending {
            let n = self.sorted.n_items();
            if let Some(pos) = (0..n).find(|i| self.sorted.item(*i).and_downcast::<gio::FileInfo>().is_some_and(|f| f.name().to_str() == Some(name.as_str()))) {
                self.reveal(pos);
            }
        }
        if let Some(e) = self.dir.error() {
            self.status.set_text(&format!("Can't open this folder: {}", e.message()));
        }
    }

    fn activate(self: &Rc<Self>, pos: u32) {
        let Some(info) = self.sorted.item(pos).and_downcast::<gio::FileInfo>() else { return };
        let Some(file) = target_of(&info) else { return };
        if is_dir(&info) {
            self.go(&file, true);
        } else {
            ops::open(&[file]);
        }
    }

    pub fn open_selected(self: &Rc<Self>) {
        let infos = self.selected();
        if let [one] = infos.as_slice() {
            if is_dir(one) {
                if let Some(f) = target_of(one) {
                    self.go(&f, true);
                }
                return;
            }
        }
        let files: Vec<gio::File> = infos.iter().filter(|i| !is_dir(i)).filter_map(target_of).collect();
        ops::open(&files);
        for i in infos.iter().filter(|i| is_dir(i)) {
            if let Some(f) = target_of(i) {
                crate::new_tab(&f);
            }
        }
    }

    pub fn location(&self) -> gio::File {
        self.location.borrow().clone()
    }

    pub fn go(self: &Rc<Self>, file: &gio::File, record: bool) {
        if file.equal(&self.location()) && self.query.borrow().is_empty() {
            return;
        }
        self.cancel_rename();
        let from = self.location.replace(file.clone());
        if record {
            self.history.borrow_mut().push(from.clone());
            self.future.borrow_mut().clear();
        }
        self.stop_hunt();
        self.query.borrow_mut().clear();
        self.filtered.set_model(Some(self.dir.model()));
        self.search_cache.borrow_mut().take();
        self.found.remove_all();
        self.filter.changed(gtk::FilterChange::LessStrict);
        self.typed.borrow_mut().0.clear();
        self.dir.set_file(Some(file));
        *self.pending.borrow_mut() = from.parent().filter(|p| p.equal(file)).and_then(|_| from.basename()).and_then(|n| n.to_str().map(str::to_string));
        crate::location_changed(self);
        kit::app::trim_later(1200);
    }

    pub fn back(self: &Rc<Self>) {
        let prev = self.history.borrow_mut().pop();
        if let Some(f) = prev {
            self.future.borrow_mut().push(self.location());
            self.go(&f, false);
        }
    }

    pub fn forward(self: &Rc<Self>) {
        let next = self.future.borrow_mut().pop();
        if let Some(f) = next {
            self.history.borrow_mut().push(self.location());
            self.go(&f, false);
        }
    }

    pub fn up(self: &Rc<Self>) {
        if let Some(p) = self.location().parent() {
            self.go(&p, true);
        }
    }

    pub fn can_back(&self) -> bool {
        !self.history.borrow().is_empty()
    }

    pub fn can_forward(&self) -> bool {
        !self.future.borrow().is_empty()
    }

    pub fn refresh(self: &Rc<Self>) {
        let f = self.location();
        self.dir.set_file(Some(&f));
        self.stop_hunt();
        self.search_cache.borrow_mut().take();
        if self.filtered.model().is_some_and(|model| model == self.found) {
            self.queue_hunt();
        }
    }

    pub fn selected(&self) -> Vec<gio::FileInfo> {
        let set = self.sel.selection();
        let mut out = Vec::new();
        if let Some((mut iter, first)) = gtk::BitsetIter::init_first(&set) {
            let mut cur = Some(first);
            while let Some(pos) = cur {
                if let Some(info) = self.sorted.item(pos).and_downcast::<gio::FileInfo>() {
                    out.push(info);
                }
                cur = iter.next();
            }
        }
        out
    }

    pub fn selected_files(&self) -> Vec<gio::File> {
        self.selected().iter().filter_map(file_of).collect()
    }

    pub fn all(&self) -> Vec<gio::FileInfo> {
        (0..self.sorted.n_items()).filter_map(|i| self.sorted.item(i).and_downcast::<gio::FileInfo>()).collect()
    }

    pub fn view(&self) -> View {
        self.view.get()
    }

    pub fn set_view(&self, v: View) {
        self.view.set(v);
        self.stack.set_visible_child_name(if v == View::Grid { "grid" } else { "list" });
    }

    pub fn zoom_by(self: &Rc<Self>, d: i32) {
        let z = (self.zoom.get() as i32 + d).clamp(0, SIZES.len() as i32 - 1) as usize;
        if z == self.zoom.get() {
            return;
        }
        let from = SIZES[self.zoom.get()] as f64;
        self.zoom.set(z);
        let _ = kit::prefs::set("files_zoom", toml::Value::Integer(z as i64));
        if self.view.get() != View::Grid || !self.grid.is_mapped() {
            self.rebind();
            return;
        }
        // Grow or shrink the icons on screen smoothly, then rebind once at the new size so
        // thumbnails load at the right resolution.
        use kit::motion::{Curve, Tween};
        let mut t = Tween::still(from);
        t.go(SIZES[z] as f64, glib::monotonic_time(), 200, Curve::new(0.2, 0.0, 0.0, 1.0));
        let generation = self.zoom_gen.get() + 1;
        self.zoom_gen.set(generation);
        let me = self.weak();
        self.grid.add_tick_callback(move |grid, clock| {
            let Some(p) = me.upgrade() else { return glib::ControlFlow::Break };
            if p.zoom_gen.get() != generation {
                return glib::ControlFlow::Break;
            }
            let now = clock.frame_time();
            let px = t.value(now).round() as i32;
            let room = px + (px / 4).max(12);
            let mut cell = grid.first_child();
            while let Some(c) = cell {
                if let Some(frame) = c.first_child().and_then(|b| b.first_child()) {
                    frame.set_size_request(room, room);
                    if let Some(img) = frame.first_child().and_downcast::<gtk::Image>() {
                        img.set_pixel_size(px);
                    }
                }
                cell = c.next_sibling();
            }
            if !t.settled(now) {
                return glib::ControlFlow::Continue;
            }
            p.rebind();
            glib::ControlFlow::Break
        });
    }

    fn rebind(self: &Rc<Self>) {
        self.grid.set_factory(None::<&gtk::ListItemFactory>);
        self.grid.set_factory(Some(&self.grid_factory()));
    }

    pub fn hidden(&self) -> bool {
        self.hidden.get()
    }

    pub fn set_hidden(self: &Rc<Self>, on: bool) {
        if self.hidden.replace(on) == on {
            return;
        }
        self.stop_hunt();
        self.search_cache.borrow_mut().take();
        self.filter.changed(gtk::FilterChange::Different);
        if self.filtered.model().is_some_and(|model| model == self.found) {
            self.queue_hunt();
        }
    }

    /// Shows the list columns that fit in `width`: Kind gives way first, then Modified, then
    /// Size, so a narrow window keeps the names readable instead of cutting the view off.
    pub fn fit_columns(&self, width: f64) {
        for (i, c) in self.columns.borrow().iter().enumerate() {
            let room = match i {
                1 => 380.0,
                2 => 540.0,
                3 => 720.0,
                _ => 0.0,
            };
            if c.is_visible() != (width >= room) {
                c.set_visible(width >= room);
            }
        }
    }

    pub fn sort_by(&self, column: usize, descending: bool) {
        if let Some(c) = self.columns.borrow().get(column) {
            self.list.sort_by_column(Some(c), if descending { gtk::SortType::Descending } else { gtk::SortType::Ascending });
        }
    }

    pub fn redraw_items(&self) {
        let n = self.sorted.n_items();
        if n > 0 {
            self.sorted.items_changed(0, n, n);
        }
    }

    pub fn focus(&self) {
        match self.view.get() {
            View::Grid => self.grid.grab_focus(),
            View::List => self.list.grab_focus(),
        };
    }

    pub fn query(&self) -> String {
        self.query.borrow().clone()
    }

    pub fn search(self: &Rc<Self>, q: &str, deep: bool) {
        let q = search::normalize(q);
        let previous = self.query();
        let recursive = !q.is_empty() && deep && self.location().is_native();
        let showing_found = self.filtered.model().is_some_and(|model| model == self.found);
        if q == previous && recursive == showing_found {
            return;
        }
        let reuse = self.search_cache.borrow().as_ref().is_some_and(|cache| cache.can_narrow(&previous, &q));
        self.stop_hunt();
        *self.query.borrow_mut() = q.clone();
        if !recursive {
            self.filtered.set_model(Some(self.dir.model()));
            self.filter.changed(gtk::FilterChange::Different);
            self.sync_status();
            return;
        }
        self.filtered.set_model(Some(&self.found));
        self.filter.changed(gtk::FilterChange::Different);
        self.sync_status();
        if !reuse {
            self.queue_hunt();
        }
    }

    fn queue_hunt(self: &Rc<Self>) {
        let me = self.weak();
        let generation = self.hunt_generation.get();
        let source = glib::timeout_add_local_once(search::DEBOUNCE, move || {
            let Some(p) = me.upgrade() else { return };
            if p.hunt_generation.get() != generation {
                return;
            }
            p.hunt_debounce.borrow_mut().take();
            p.start_hunt(generation);
        });
        *self.hunt_debounce.borrow_mut() = Some(source);
    }

    fn start_hunt(self: &Rc<Self>, generation: u64) {
        let Some(root) = self.location().path() else { return };
        self.search_cache.borrow_mut().take();
        self.found.remove_all();
        let q = self.query();
        let searched = q.clone();
        let cancel = gio::Cancellable::new();
        *self.hunt.borrow_mut() = Some(cancel.clone());
        let show_hidden = self.hidden.get();
        let (tx, rx) = std::sync::mpsc::sync_channel(2);
        std::thread::spawn(move || {
            let outcome = search::scan(&root, &q, show_hidden, &cancel,
                |path| search::Hit::read(path, ATTRS, show_hidden, &cancel),
                |batch| search::send(&tx, search::Message::Batch(batch), &cancel));
            search::send(&tx, search::Message::Finished(outcome), &cancel);
        });
        let me = self.weak();
        let source = glib::timeout_add_local(search::BATCH_INTERVAL, move || {
            let Some(p) = me.upgrade() else { return glib::ControlFlow::Break };
            if p.hunt_generation.get() != generation {
                return glib::ControlFlow::Break;
            }
            // One bounded batch per tick: never drain a backlog in one frame.
            match rx.try_recv() {
                Ok(search::Message::Batch(batch)) => {
                    p.add_found(batch);
                    return glib::ControlFlow::Continue;
                }
                Ok(search::Message::Finished(outcome)) => {
                    *p.search_cache.borrow_mut() = Some(search::Cache { query: searched.clone(), outcome });
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {}
            }
            p.hunt_source.borrow_mut().take();
            p.hunt.borrow_mut().take();
            p.sync_status();
            glib::ControlFlow::Break
        });
        *self.hunt_source.borrow_mut() = Some(source);
    }

    fn add_found(&self, batch: Vec<search::Hit>) {
        let room = search::RESULT_LIMIT.saturating_sub(self.found.n_items() as usize);
        let items: Vec<gio::FileInfo> = batch.into_iter().take(room).map(search::Hit::into_info).collect();
        self.found.extend_from_slice(&items);
    }

    fn stop_hunt(&self) {
        self.hunt_generation.set(self.hunt_generation.get().wrapping_add(1));
        let cancel = self.hunt.borrow_mut().take();
        if let Some(cancel) = cancel {
            cancel.cancel();
        }
        let debounce = self.hunt_debounce.borrow_mut().take();
        if let Some(source) = debounce {
            source.remove();
        }
        let pending = self.hunt_source.borrow_mut().take();
        if let Some(source) = pending {
            source.remove();
        }
    }

    fn sync_empty(&self) {
        let empty = self.sorted.n_items() == 0 && !self.dir.is_loading();
        self.empty.set_visible(empty);
        if let Some(l) = self.empty.last_child().and_downcast::<gtk::Label>() {
            l.set_text(if !self.query.borrow().is_empty() {
                "Nothing matches"
            } else if self.dir.error().is_some() {
                "Can't open this folder"
            } else {
                "This folder is empty"
            });
        }
    }

    fn sync_status(&self) {
        let n = self.sorted.n_items();
        let picked = self.selected();
        let capped = !self.query.borrow().is_empty() && self.found.n_items() as usize >= search::RESULT_LIMIT;
        let text = if picked.is_empty() && capped {
            format!("Showing the first {} results · Type more to narrow them", search::RESULT_LIMIT)
        } else if picked.is_empty() {
            format!("{n} item{}", if n == 1 { "" } else { "s" })
        } else {
            let bytes: i64 = picked.iter().filter(|i| !is_dir(i)).map(|i| i.size()).sum();
            let dirs = picked.iter().filter(|i| is_dir(i)).count();
            let mut t = format!("{} of {n} selected", picked.len());
            if bytes > 0 {
                t.push_str(&format!(" · {}", size_text(bytes as u64)));
            }
            if dirs > 0 {
                t.push_str(&format!(" · {dirs} folder{}", if dirs == 1 { "" } else { "s" }));
            }
            t
        };
        self.status.set_text(&text);
    }
}

impl Drop for Pane {
    fn drop(&mut self) {
        self.stop_hunt();
    }
}

#[cfg(test)]
mod drag_tests {
    use super::*;

    fn assert_grab(g: &DragGeometry, grab: (f64, f64)) {
        assert!((grab.0 + g.shift.0 - g.hotspot.0 as f64).abs() < 1e-9);
        assert!((grab.1 + g.shift.1 - g.hotspot.1 as f64).abs() < 1e-9);
        assert!(g.hotspot.0 >= 0 && (g.hotspot.0 as f64) < g.size.0);
        assert!(g.hotspot.1 >= 0 && (g.hotspot.1 as f64) < g.size.1);
    }

    fn assert_contains(g: &DragGeometry, rect: (f64, f64, f64, f64)) {
        assert!(rect.0 + g.shift.0 >= 0.0);
        assert!(rect.1 + g.shift.1 >= 0.0);
        assert!(rect.0 + rect.2 + g.shift.0 <= g.size.0);
        assert!(rect.1 + rect.3 + g.shift.1 <= g.size.1);
    }

    #[test]
    fn fractional_grabs_stay_under_pointer_at_every_scale() {
        for scale in [0, 1, 2, 3] {
            for grab in [(0.0, 0.0), (13.25, 42.75), (97.99, 130.01)] {
                let g = drag_geometry(&[(120.5, 144.25)], grab, None, scale);
                assert_grab(&g, grab);
                assert_contains(&g, (0.0, 0.0, 120.5, 144.25));
                // Moving the pointer preserves the same world-space grab, without
                // multiplying the logical hotspot by the output scale.
                let pointer = (901.5, 210.25);
                let world = (pointer.0 - g.hotspot.0 as f64 + g.shift.0 + grab.0,
                             pointer.1 - g.hotspot.1 as f64 + g.shift.1 + grab.1);
                assert_eq!(world, pointer);
            }
        }
    }

    #[test]
    fn grid_zoom_and_list_artwork_keep_the_relative_press_at_desktop_size() {
        let target = preview_regions((64.0, 64.0), 32.0);
        for px in SIZES.into_iter().chain([22]) {
            let px = px as f64;
            let source = [(20.0, 12.0, px, px), (6.0, px + 30.0, 200.0, 32.0)];
            for (rx, ry) in [(0.0, 0.0), (0.25, 0.75), (0.5, 0.5), (1.0, 1.0)] {
                let grab = preview_grab((20.0 + px * rx, 12.0 + px * ry), source, target);
                assert_eq!(grab, (24.0 + 64.0 * rx, 64.0 * ry));
                let g = drag_geometry(&[DRAG_CELL], grab, None, 2);
                assert_grab(&g, grab);
                assert_eq!(g.size, (114.0, 118.0));
            }
        }
    }

    #[test]
    fn names_and_list_metadata_map_into_the_compact_name_instead_of_a_wide_surface() {
        let target = preview_regions((64.0, 64.0), 32.0);
        let source = [(0.0, 0.0, 22.0, 22.0), (32.0, 0.0, 300.0, 22.0)];
        for (pointer, expected) in [((107.0, 16.5), (31.0, 94.0)), ((840.75, 11.0), (106.0, 86.0))] {
            let grab = preview_grab(pointer, source, target);
            assert_eq!(grab, expected);
            let g = drag_geometry(&[DRAG_CELL], grab, None, 2);
            assert_grab(&g, grab);
            assert_eq!(g.size, (114.0, 118.0));
        }
    }

    #[test]
    fn nonsquare_thumbnails_fit_64_pixels_and_keep_the_same_press() {
        for (aspect, expected) in [(2.0, (64.0, 32.0)), (0.5, (32.0, 64.0)), (0.0, (64.0, 64.0))] {
            let size = artwork_size(aspect, DRAG_ICON);
            assert_eq!(size, expected);
            let source = preview_regions(artwork_size(aspect, 176.0), 32.0);
            let target = preview_regions(size, 32.0);
            let art = source[0];
            let grab = preview_grab((art.0 + art.2 * 0.25, art.1 + art.3 * 0.75), source, target);
            assert_eq!(grab, (target[0].0 + size.0 * 0.25, target[0].1 + size.1 * 0.75));
        }
    }

    #[test]
    fn stack_bounds_include_different_card_sizes_and_count_badge() {
        let sizes = [(80.0, 100.0), (160.25, 40.5), (110.75, 120.25)];
        let g = drag_geometry(&sizes, (12.75, 24.25), Some((96.0, 28.0)), 2);
        assert_grab(&g, (12.75, 24.25));
        for (i, (w, h)) in sizes.into_iter().enumerate() {
            assert_contains(&g, (stack_offset(i), stack_offset(i), w, h));
        }
        assert_contains(&g, (-16.0, 0.0, 96.0, 28.0));
    }

    #[test]
    fn stack_is_capped_at_three_and_single_item_keeps_its_size() {
        let sizes = [DRAG_CELL; 4];
        let single = drag_geometry(&sizes[..1], (15.0, 20.0), None, 1);
        assert_eq!(single.size, (114.0, 118.0));
        let three = drag_geometry(&sizes[..3], (15.0, 20.0), None, 1);
        let four = drag_geometry(&sizes, (15.0, 20.0), None, 1);
        assert_eq!(three.size, (130.0, 134.0));
        assert_eq!(four.size, three.size);
        assert_eq!(four.hotspot, three.hotspot);
    }
}

// GTK editable offsets count characters, not UTF-8 bytes.
pub fn rename_selection(name: &str, folder: bool) -> (i32, i32) {
    let end = if folder || name.starts_with('.') { name.len() } else {
        name.rfind('.').filter(|&i| i > 0 && i + 1 < name.len()).unwrap_or(name.len())
    };
    (0, name[..end].chars().count() as i32)
}

pub fn validate_name(name: &str, exists: bool) -> Result<(), &'static str> {
    if name.is_empty() { Err("Enter a name") }
    else if name.contains('/') || name.contains('\0') || name == "." || name == ".." { Err("Invalid name") }
    else if exists { Err("That name already exists") }
    else { Ok(()) }
}

pub struct TrashFlight {
    pub uri: String,
    x: f32,
    y: f32,
    size: i32,
    icon: String,
}

impl TrashFlight {
    // Same socket and JSON word-array protocol as digitctl. Called by the
    // operation worker after a successful trash; never blocks the GTK thread.
    pub fn send(&self) {
        use std::io::Write;
        use std::os::unix::net::UnixStream;
        let dir = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let Ok(mut stream) = UnixStream::connect(dir.join("digit.sock")) else { return };
        let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
        let words = ["trash-fly".to_string(), std::process::id().to_string(), self.x.to_string(), self.y.to_string(), self.size.to_string(), self.icon.clone()];
        let quoted: Vec<String> = words.iter().map(|word| {
            let mut out = String::from("\"");
            for c in word.chars() {
                match c {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                    c => out.push(c),
                }
            }
            out.push('"');
            out
        }).collect();
        let _ = writeln!(stream, "[{}]", quoted.join(","));
    }
}

#[cfg(test)]
mod rename_tests {
    use super::{rename_selection, validate_name};

    #[test]
    fn selects_stem_using_character_offsets() {
        assert_eq!(rename_selection("résumé.tar.gz", false), (0, 10));
        assert_eq!(rename_selection("photo.png", false), (0, 5));
    }

    #[test]
    fn selects_whole_folders_dotfiles_and_extensionless_names() {
        for (name, folder) in [("folder.ext", true), ("README", false), (".config.json", false), ("name.", false), ("", false)] {
            assert_eq!(rename_selection(name, folder), (0, name.chars().count() as i32));
        }
    }

    #[test]
    fn rejects_empty_invalid_and_existing_names() {
        for name in ["", ".", "..", "a/b", "bad\0name"] { assert!(validate_name(name, false).is_err()); }
        assert!(validate_name("sibling", true).is_err());
        for name in ["file.txt", ".hidden", " spaced ", "日本語"] { assert!(validate_name(name, false).is_ok()); }
    }
}
