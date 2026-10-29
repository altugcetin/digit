use crate::{ops, thumbs};
use gtk::{gdk, gio, glib, pango, prelude::*};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::time::{Duration, Instant};

pub const ATTRS: &str = "standard::name,standard::display-name,standard::edit-name,standard::type,standard::size,standard::icon,standard::fast-content-type,standard::is-hidden,standard::is-backup,standard::is-symlink,standard::target-uri,time::modified,access::can-write,access::can-trash,access::can-rename,trash::orig-path,trash::deletion-date,unix::mode";
pub const SIZES: [i32; 6] = [40, 56, 72, 96, 128, 176];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum View {
    Grid,
    List,
}

pub struct Pane {
    pub root: gtk::Box,
    pub dir: gtk::DirectoryList,
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
    view: Cell<View>,
    found: gio::ListStore,
    hunt: RefCell<Option<Arc<AtomicBool>>>,
    pub cut: Rc<RefCell<Vec<String>>>,
    pending: RefCell<Option<String>>,
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

impl Pane {
    pub fn new(start: &gio::File) -> Rc<Pane> {
        let dir = gtk::DirectoryList::new(Some(ATTRS), Some(start));
        dir.set_monitored(true);
        let query: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
        let hidden = Rc::new(Cell::new(kit::prefs::flag(&kit::prefs::read(), "files_hidden", false)));
        let (q, h) = (query.clone(), hidden.clone());
        let filter = gtk::CustomFilter::new(move |o| {
            let Some(info) = o.downcast_ref::<gio::FileInfo>() else { return false };
            if !h.get() && (info.is_hidden() || info.is_backup()) {
                return false;
            }
            let q = q.borrow();
            q.is_empty() || info.display_name().to_lowercase().contains(q.as_str())
        });
        let filtered = gtk::FilterListModel::new(Some(dir.clone()), Some(filter.clone()));
        let folders = sorter(|a, b| order(!is_dir(a), !is_dir(b)));
        let multi = gtk::MultiSorter::new();
        multi.append(folders);
        let sorted = gtk::SortListModel::new(Some(filtered.clone()), Some(multi.clone()));
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
        let status = kit::ui::text("", "status");
        status.set_ellipsize(pango::EllipsizeMode::End);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("pane");
        root.append(&over);
        root.append(&status);
        let p = Rc::new(Pane {
            root,
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
            zoom: Cell::new(kit::prefs::number(&kit::prefs::read(), "files_zoom", 3.0).clamp(0.0, (SIZES.len() - 1) as f64) as usize),
            view: Cell::new(View::Grid),
            found: gio::ListStore::new::<gio::FileInfo>(),
            hunt: RefCell::new(None),
            cut: Rc::new(RefCell::new(Vec::new())),
            pending: RefCell::new(None),
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
            let cut = me.upgrade().is_some_and(|p| file_of(&info).is_some_and(|f| p.cut.borrow().contains(&f.uri().to_string())));
            b.set_opacity(if info.is_hidden() || cut { 0.55 } else { 1.0 });
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
                        let cut = me.upgrade().is_some_and(|p| file_of(&info).is_some_and(|f| p.cut.borrow().contains(&f.uri().to_string())));
                        b.set_opacity(if info.is_hidden() || cut { 0.55 } else { 1.0 });
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
            let item = unsafe { w.data::<glib::WeakRef<glib::Object>>("digit-item") }?;
            position(unsafe { item.as_ref() })
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
        let me = self.weak();
        let v = view.clone();
        drag.connect_prepare(move |d, x, y| {
            let p = me.upgrade()?;
            let pos = Pane::item_at(&v, x, y)?;
            if !p.sel.is_selected(pos) {
                p.sel.select_item(pos, true);
            }
            let files = p.selected_files();
            if files.is_empty() {
                return None;
            }
            if let Some(info) = p.sorted.item(pos).and_downcast::<gio::FileInfo>() {
                let theme = gtk::IconTheme::for_display(&v.display());
                let icon = theme.lookup_by_gicon(&icon_for(&info), 48, v.scale_factor().max(1), gtk::TextDirection::None, gtk::IconLookupFlags::empty());
                d.set_icon(Some(&icon), 24, 24);
            }
            Some(gdk::ContentProvider::for_value(&gdk::FileList::from_array(&files).to_value()))
        });
        view.add_controller(drag);
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
        unsafe {
            w.set_data("digit-item", item.clone());
        }
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
        let me = self.weak();
        self.dir.connect_loading_notify(move |d| {
            if let Some(p) = me.upgrade() {
                if !d.is_loading() {
                    p.settled();
                }
                p.sync_empty();
            }
        });
        let me = self.weak();
        self.sorted.connect_items_changed(move |_, _, _, _| {
            if let Some(p) = me.upgrade() {
                p.sync_empty();
                p.sync_status();
            }
        });
        let me = self.weak();
        self.sel.connect_selection_changed(move |_, _, _| {
            if let Some(p) = me.upgrade() {
                p.sync_status();
                crate::selection_changed(&p);
            }
        });
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

    pub fn select_name(&self, name: &str) {
        let n = self.sorted.n_items();
        match (0..n).find(|i| self.sorted.item(*i).and_downcast::<gio::FileInfo>().is_some_and(|f| f.name().to_str() == Some(name))) {
            Some(pos) => self.reveal(pos),
            None => *self.pending.borrow_mut() = Some(name.to_string()),
        }
    }

    fn settled(&self) {
        if let Some(name) = self.pending.borrow_mut().take() {
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
        let from = self.location.replace(file.clone());
        if record {
            self.history.borrow_mut().push(from.clone());
            self.future.borrow_mut().clear();
        }
        self.stop_hunt();
        self.query.borrow_mut().clear();
        self.filtered.set_model(Some(&self.dir));
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

    pub fn refresh(&self) {
        let f = self.location();
        self.dir.set_file(None::<&gio::File>);
        self.dir.set_file(Some(&f));
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
        self.zoom.set(z);
        let _ = kit::prefs::set("files_zoom", toml::Value::Integer(z as i64));
        self.grid.set_factory(None::<&gtk::ListItemFactory>);
        self.grid.set_factory(Some(&self.grid_factory()));
    }

    pub fn hidden(&self) -> bool {
        self.hidden.get()
    }

    pub fn set_hidden(&self, on: bool) {
        self.hidden.set(on);
        self.filter.changed(gtk::FilterChange::Different);
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
        let q = q.trim().to_lowercase();
        self.stop_hunt();
        *self.query.borrow_mut() = q.clone();
        if q.is_empty() || !deep || !self.location().is_native() {
            self.filtered.set_model(Some(&self.dir));
            self.filter.changed(gtk::FilterChange::Different);
            self.sync_status();
            return;
        }
        self.found.remove_all();
        self.filtered.set_model(Some(&self.found));
        self.filter.changed(gtk::FilterChange::Different);
        let Some(root) = self.location().path() else { return };
        let stop = Arc::new(AtomicBool::new(false));
        *self.hunt.borrow_mut() = Some(stop.clone());
        let show_hidden = self.hidden.get();
        let (tx, rx) = std::sync::mpsc::channel::<Vec<std::path::PathBuf>>();
        std::thread::spawn(move || hunt(&root, &q, show_hidden, &stop, &tx));
        let me = self.weak();
        glib::timeout_add_local(Duration::from_millis(120), move || {
            let Some(p) = me.upgrade() else { return glib::ControlFlow::Break };
            loop {
                match rx.try_recv() {
                    Ok(batch) => p.add_found(batch),
                    Err(std::sync::mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        p.sync_status();
                        return glib::ControlFlow::Break;
                    }
                }
            }
        });
    }

    fn add_found(&self, batch: Vec<std::path::PathBuf>) {
        let items: Vec<glib::Object> = batch
            .iter()
            .filter_map(|path| {
                let f = gio::File::for_path(path);
                let info = f.query_info(ATTRS, gio::FileQueryInfoFlags::NONE, None::<&gio::Cancellable>).ok()?;
                info.set_attribute_object("standard::file", &f);
                Some(info.upcast())
            })
            .collect();
        self.found.extend_from_slice(&items);
    }

    fn stop_hunt(&self) {
        if let Some(s) = self.hunt.borrow_mut().take() {
            s.store(true, AtomicOrdering::Relaxed);
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
        let text = if picked.is_empty() {
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

fn hunt(root: &std::path::Path, q: &str, hidden: bool, stop: &AtomicBool, tx: &std::sync::mpsc::Sender<Vec<std::path::PathBuf>>) {
    let mut queue = std::collections::VecDeque::from([(root.to_path_buf(), 0usize)]);
    let mut batch = Vec::new();
    let mut total = 0usize;
    let mut last = Instant::now();
    while let Some((dir, depth)) = queue.pop_front() {
        if stop.load(AtomicOrdering::Relaxed) || total >= 3000 {
            return;
        }
        let Ok(read) = std::fs::read_dir(&dir) else { continue };
        for e in read.flatten() {
            let name = e.file_name();
            let name = name.to_string_lossy();
            if !hidden && name.starts_with('.') {
                continue;
            }
            let Ok(ft) = e.file_type() else { continue };
            if name.to_lowercase().contains(q) {
                batch.push(e.path());
                total += 1;
            }
            if ft.is_dir() && !ft.is_symlink() && depth < 14 {
                queue.push_back((e.path(), depth + 1));
            }
        }
        if !batch.is_empty() && (batch.len() >= 64 || last.elapsed() > Duration::from_millis(150)) {
            if tx.send(std::mem::take(&mut batch)).is_err() {
                return;
            }
            last = Instant::now();
        }
    }
    if !batch.is_empty() {
        let _ = tx.send(batch);
    }
}
