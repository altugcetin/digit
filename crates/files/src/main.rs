mod art;
mod info;
mod look;
mod ops;
mod pane;
mod popup;
mod side;
mod thumbs;
mod tools;

use gtk::{gdk, gio, glib, pango, prelude::*};
use pane::{Pane, View};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const APP_ID: &str = "dev.digit.Files";

struct Tab {
    pane: Rc<Pane>,
    title: gtk::Label,
}

/// One side of the window: its own row of tabs. Split view shows two of these.
struct Column {
    notebook: gtk::Notebook,
    tabs: RefCell<Vec<Rc<Tab>>>,
}

struct Files {
    window: gtk::ApplicationWindow,
    split: gtk::Paned,
    columns: RefCell<Vec<Rc<Column>>>,
    current: Cell<usize>,
    crumbs: gtk::Box,
    path_stack: gtk::Stack,
    path_entry: gtk::Entry,
    search: gtk::SearchEntry,
    back: gtk::Button,
    forward: gtk::Button,
    up: gtk::Button,
    view_btn: gtk::Button,
    side: Rc<side::Side>,
    side_rev: gtk::Revealer,
    info: Rc<info::Info>,
    info_rev: gtk::Revealer,
    jobs: gtk::Box,
    jobs_rev: gtk::Revealer,
    toast: gtk::Label,
    toast_rev: gtk::Revealer,
    toast_gen: Cell<u32>,
    overlay: gtk::Overlay,
    trash_bar: gtk::Revealer,
    hold: RefCell<Option<gio::ApplicationHoldGuard>>,
}

thread_local! {
    static FILES: RefCell<Option<Rc<Files>>> = const { RefCell::new(None) };
}

fn files() -> Option<Rc<Files>> {
    FILES.with(|f| f.borrow().clone())
}

pub fn window() -> Option<gtk::Window> {
    files().map(|f| f.window.clone().upcast())
}

fn column() -> Option<Rc<Column>> {
    let f = files()?;
    let columns = f.columns.borrow();
    columns.get(f.current.get()).or_else(|| columns.first()).cloned()
}

impl Column {
    fn tab(&self) -> Option<Rc<Tab>> {
        let i = self.notebook.current_page()? as usize;
        self.tabs.borrow().get(i).cloned()
    }
}

fn tab() -> Option<Rc<Tab>> {
    column()?.tab()
}

pub fn active() -> Option<Rc<Pane>> {
    tab().map(|t| t.pane.clone())
}

fn other() -> Option<Rc<Pane>> {
    let f = files()?;
    let columns = f.columns.borrow();
    if columns.len() < 2 {
        return None;
    }
    columns.get(1 - f.current.get().min(1))?.tab().map(|t| t.pane.clone())
}

fn panes() -> Vec<Rc<Pane>> {
    let Some(f) = files() else { return Vec::new() };
    f.columns.borrow().iter().flat_map(|c| c.tabs.borrow().iter().map(|t| t.pane.clone()).collect::<Vec<_>>()).collect()
}

pub fn toast(msg: &str) {
    let Some(f) = files() else { return };
    f.toast.set_text(msg);
    f.toast_rev.set_reveal_child(true);
    let g = f.toast_gen.get() + 1;
    f.toast_gen.set(g);
    glib::timeout_add_local_once(std::time::Duration::from_millis(3200), move || {
        if let Some(f) = files() {
            if f.toast_gen.get() == g {
                f.toast_rev.set_reveal_child(false);
            }
        }
    });
}

pub fn jobs_row(label: &str, cancel: &gio::Cancellable) -> (gtk::Box, gtk::ProgressBar, gtk::Label) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("job");
    let col = gtk::Box::new(gtk::Orientation::Vertical, 4);
    col.set_hexpand(true);
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    top.append(&kit::ui::text(label, "name"));
    let now = kit::ui::text("", "sub");
    now.set_ellipsize(pango::EllipsizeMode::Middle);
    now.set_hexpand(true);
    top.append(&now);
    col.append(&top);
    let bar = gtk::ProgressBar::new();
    col.append(&bar);
    row.append(&col);
    let stop = kit::app::control("close", "");
    stop.set_tooltip_text(Some("Cancel"));
    let c = cancel.clone();
    stop.connect_clicked(move |_| c.cancel());
    row.append(&stop);
    if let Some(f) = files() {
        f.jobs.append(&row);
        f.jobs_rev.set_reveal_child(true);
    }
    (row, bar, now)
}

pub fn jobs_done(row: &gtk::Box) {
    let Some(f) = files() else { return };
    f.jobs.remove(row);
    if f.jobs.first_child().is_none() {
        f.jobs_rev.set_reveal_child(false);
    }
}

pub fn jobs_busy() -> bool {
    files().is_some_and(|f| f.jobs.first_child().is_some())
}

pub fn refresh_all() {
    for p in panes() {
        if !p.location().is_native() && !ops::is_trash(&p.location()) {
            p.refresh();
        }
    }
}

pub fn set_cut(uris: Vec<String>) {
    for p in panes() {
        *p.cut.borrow_mut() = uris.clone();
        p.redraw_items();
    }
}

pub fn select_later(file: &gio::File) {
    let Some(p) = active() else { return };
    let (Some(parent), Some(name)) = (file.parent(), file.basename()) else { return };
    if parent.equal(&p.location()) {
        if let Some(n) = name.to_str() {
            p.select_name(n);
        }
    }
}

pub fn go(file: &gio::File) {
    if let Some(p) = active() {
        p.go(file, true);
        p.focus();
    }
}

pub fn rebuild_side() {
    if let Some(f) = files() {
        f.side.build();
    }
}

pub fn overlay_add(w: &gtk::Box) {
    if let Some(f) = files() {
        f.overlay.add_overlay(w);
    }
}

pub fn overlay_remove(w: &gtk::Box) {
    if let Some(f) = files() {
        f.overlay.remove_overlay(w);
        if let Some(p) = active() {
            p.focus();
        }
    }
}

pub fn focus_pane(p: &Rc<Pane>) {
    let Some(f) = files() else { return };
    let i = f.columns.borrow().iter().position(|c| c.tabs.borrow().iter().any(|t| Rc::ptr_eq(&t.pane, p)));
    if let Some(i) = i {
        if f.current.get() != i {
            f.current.set(i);
            mark_focus();
            sync();
        }
    }
}

/// In split view the side being worked in gets an outline.
fn mark_focus() {
    let Some(f) = files() else { return };
    let split = f.columns.borrow().len() > 1;
    for (i, c) in f.columns.borrow().iter().enumerate() {
        for t in c.tabs.borrow().iter() {
            if split && i == f.current.get() {
                t.pane.root.add_css_class("focused");
            } else {
                t.pane.root.remove_css_class("focused");
            }
        }
    }
}

pub fn location_changed(p: &Rc<Pane>) {
    let Some(f) = files() else { return };
    for c in f.columns.borrow().iter() {
        for t in c.tabs.borrow().iter().filter(|t| Rc::ptr_eq(&t.pane, p)) {
            t.title.set_text(&title_of(&p.location()));
        }
    }
    if active().is_some_and(|a| Rc::ptr_eq(&a, p)) {
        sync();
    }
}

pub fn selection_changed(p: &Rc<Pane>) {
    if !active().is_some_and(|a| Rc::ptr_eq(&a, p)) {
        return;
    }
    if let Some(f) = files() {
        if f.info_rev.reveals_child() {
            f.info.show(&p.selected(), &p.location());
        }
    }
    look::update();
}

fn title_of(f: &gio::File) -> String {
    if f.path().is_some_and(|p| p == glib::home_dir()) {
        return "Home".into();
    }
    f.query_info("standard::display-name", gio::FileQueryInfoFlags::NONE, None::<&gio::Cancellable>)
        .map(|i| i.display_name().to_string())
        .unwrap_or_else(|_| f.basename().map(|b| b.to_string_lossy().into_owned()).unwrap_or_else(|| f.uri().to_string()))
}

fn sync() {
    let (Some(f), Some(p)) = (files(), active()) else { return };
    f.back.set_sensitive(p.can_back());
    f.forward.set_sensitive(p.can_forward());
    f.up.set_sensitive(p.location().parent().is_some());
    f.view_btn.set_child(Some(&kit::ui::sym(if p.view() == View::Grid { "view_list" } else { "grid_view" })));
    crumbs(&f, &p.location());
    f.side.mark(&p.location());
    f.trash_bar.set_reveal_child(ops::pane_of_trash(&p));
    if f.info_rev.reveals_child() {
        f.info.show(&p.selected(), &p.location());
    }
    if f.search.text().to_lowercase() != p.query() {
        f.search.set_text(&p.query());
    }
    f.window.set_title(Some(&title_of(&p.location())));
    if let Some(a) = f.window.lookup_action("hidden").and_downcast::<gio::SimpleAction>() {
        a.set_state(&p.hidden().to_variant());
    }
}

fn crumbs(f: &Files, loc: &gio::File) {
    kit::ui::clear(&f.crumbs);
    let home = gio::File::for_path(glib::home_dir());
    let mut chain = Vec::new();
    let mut cur = Some(loc.clone());
    while let Some(c) = cur {
        let stop = c.equal(&home);
        chain.push(c.clone());
        if stop {
            break;
        }
        cur = c.parent();
    }
    chain.reverse();
    let last = chain.len().saturating_sub(1);
    for (i, c) in chain.iter().enumerate() {
        if i > 0 {
            let s = kit::ui::sym("chevron_right");
            s.add_css_class("crumb-sep");
            f.crumbs.append(&s);
        }
        let label = if c.equal(&home) {
            "Home".to_string()
        } else if c.path().is_some_and(|p| p == std::path::Path::new("/")) {
            "Computer".to_string()
        } else {
            title_of(c)
        };
        let b = gtk::Button::with_label(&label);
        b.add_css_class("crumb");
        if i == last {
            b.add_css_class("here");
        }
        let target = c.clone();
        b.connect_clicked(move |_| go(&target));
        let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY | gdk::DragAction::MOVE);
        let dest = c.clone();
        drop.connect_drop(move |t, v, _, _| {
            let Ok(list) = v.get::<gdk::FileList>() else { return false };
            ops::dropped(list.files(), &dest, t.current_event_state().contains(gdk::ModifierType::CONTROL_MASK));
            true
        });
        b.add_controller(drop);
        f.crumbs.append(&b);
    }
    f.path_stack.set_visible_child_name("crumbs");
}

fn edit_path() {
    let (Some(f), Some(p)) = (files(), active()) else { return };
    let loc = p.location();
    let text = match loc.path() {
        Some(path) => {
            let s = path.to_string_lossy().into_owned();
            let home = glib::home_dir().to_string_lossy().into_owned();
            if s == home { "~/".to_string() } else if let Some(r) = s.strip_prefix(&format!("{home}/")) { format!("~/{r}/") } else { format!("{s}/").replace("//", "/") }
        }
        None => loc.uri().to_string(),
    };
    f.path_entry.set_text(&text);
    f.path_stack.set_visible_child_name("entry");
    f.path_entry.grab_focus();
    f.path_entry.set_position(-1);
}

fn resolve(text: &str) -> Option<gio::File> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    if t.contains("://") || t.starts_with("trash:") || t.starts_with("recent:") {
        return Some(gio::File::for_uri(t));
    }
    let path = kit::prefs::home(t);
    if path.is_absolute() {
        return Some(gio::File::for_path(path));
    }
    active().map(|p| p.location().resolve_relative_path(t))
}

fn complete() {
    let Some(f) = files() else { return };
    let text = f.path_entry.text().to_string();
    let (dir, part) = match text.rfind('/') {
        Some(i) => (text[..=i].to_string(), text[i + 1..].to_string()),
        None => return,
    };
    let Some(d) = resolve(&dir).and_then(|d| d.path()) else { return };
    let Ok(read) = std::fs::read_dir(d) else { return };
    let mut hits: Vec<String> = read.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with(&part) && (part.starts_with('.') || !n.starts_with('.'))).collect();
    hits.sort();
    let Some(first) = hits.first().cloned() else { return };
    let common = hits.iter().fold(first.clone(), |acc, h| acc.chars().zip(h.chars()).take_while(|(a, b)| a == b).map(|(a, _)| a).collect());
    let done = if hits.len() == 1 { format!("{dir}{first}/") } else { format!("{dir}{common}") };
    f.path_entry.set_text(&done);
    f.path_entry.set_position(-1);
}

pub fn new_tab(file: &gio::File) {
    if let Some(c) = column() {
        add_tab(&c, file);
    }
}

fn add_tab(c: &Rc<Column>, file: &gio::File) {
    let pane = Pane::new(file);
    let label = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let title = kit::ui::text(&title_of(file), "");
    title.set_ellipsize(pango::EllipsizeMode::End);
    title.set_width_chars(10);
    title.set_max_width_chars(24);
    label.append(&title);
    let x = kit::app::control("close", "");
    x.add_css_class("tab-close");
    label.append(&x);
    let tab = Rc::new(Tab { pane: pane.clone(), title });
    let n = c.notebook.append_page(&pane.root, Some(&label));
    c.notebook.set_tab_reorderable(&pane.root, true);
    c.tabs.borrow_mut().insert(n as usize, tab);
    let page = pane.root.clone().upcast::<gtk::Widget>();
    let pg = page.clone();
    x.connect_clicked(move |_| close_tab(&pg));
    let middle = gtk::GestureClick::new();
    middle.set_button(2);
    middle.connect_released(move |g, _, _, _| {
        g.set_state(gtk::EventSequenceState::Claimed);
        close_tab(&page);
    });
    label.add_controller(middle);
    c.notebook.set_show_tabs(c.notebook.n_pages() > 1);
    c.notebook.set_current_page(Some(n));
    pane.focus();
    focus_pane(&pane);
    mark_focus();
    sync();
}

fn close_tab(page: &gtk::Widget) {
    let Some(f) = files() else { return };
    let Some(c) = f.columns.borrow().iter().find(|c| c.notebook.page_num(page).is_some()).cloned() else { return };
    let Some(i) = c.notebook.page_num(page) else { return };
    if c.notebook.n_pages() <= 1 {
        if f.columns.borrow().len() > 1 {
            close_column(&c);
        } else {
            f.window.close();
        }
        return;
    }
    c.notebook.remove_page(Some(i));
    c.tabs.borrow_mut().remove(i as usize);
    c.notebook.set_show_tabs(c.notebook.n_pages() > 1);
    kit::app::trim_later(800);
    sync();
}

fn new_column() -> Rc<Column> {
    let notebook = gtk::Notebook::new();
    notebook.set_show_border(false);
    notebook.set_show_tabs(false);
    notebook.set_scrollable(true);
    notebook.set_hexpand(true);
    notebook.set_vexpand(true);
    let c = Rc::new(Column { notebook: notebook.clone(), tabs: RefCell::new(Vec::new()) });
    let weak = Rc::downgrade(&c);
    notebook.connect_switch_page(move |_, page, _| {
        let Some(c) = weak.upgrade() else { return };
        let picked = c.tabs.borrow().iter().find(|t| t.pane.root.upcast_ref::<gtk::Widget>() == page).map(|t| t.pane.clone());
        glib::idle_add_local_once(move || {
            if let Some(p) = picked {
                focus_pane(&p);
            }
            sync();
        });
    });
    let weak = Rc::downgrade(&c);
    notebook.connect_page_reordered(move |_, child, to| {
        let Some(c) = weak.upgrade() else { return };
        let mut tabs = c.tabs.borrow_mut();
        if let Some(from) = tabs.iter().position(|t| t.pane.root.upcast_ref::<gtk::Widget>() == child) {
            let t = tabs.remove(from);
            let at = (to as usize).min(tabs.len());
            tabs.insert(at, t);
        }
    });
    c
}

fn close_column(c: &Rc<Column>) {
    let Some(f) = files() else { return };
    let Some(i) = f.columns.borrow().iter().position(|x| Rc::ptr_eq(x, c)) else { return };
    f.columns.borrow_mut().remove(i);
    let keep = f.columns.borrow().first().cloned();
    if let Some(keep) = keep {
        f.split.set_end_child(None::<&gtk::Widget>);
        f.split.set_start_child(Some(&keep.notebook));
    }
    f.current.set(0);
    mark_focus();
    if let Some(p) = active() {
        p.focus();
    }
    kit::app::trim_later(800);
    sync();
}

fn toggle_split() {
    let Some(f) = files() else { return };
    let right = f.columns.borrow().get(1).cloned();
    if let Some(right) = right {
        close_column(&right);
        return;
    }
    let loc = active().map(|p| p.location()).unwrap_or_else(|| gio::File::for_path(glib::home_dir()));
    let c = new_column();
    f.split.set_end_child(Some(&c.notebook));
    let w = f.split.width();
    if w > 0 {
        f.split.set_position(w / 2);
    }
    f.columns.borrow_mut().push(c.clone());
    f.current.set(1);
    add_tab(&c, &loc);
}

/// Ctrl+Tab and Ctrl+Shift+Tab walk the tabs of the side being worked in.
fn cycle_tab(by: i32) {
    let Some(c) = column() else { return };
    let n = c.notebook.n_pages() as i32;
    if n < 2 {
        return;
    }
    let at = c.notebook.current_page().unwrap_or(0) as i32;
    c.notebook.set_current_page(Some((at + by).rem_euclid(n) as u32));
    if let Some(t) = c.tab() {
        t.pane.focus();
    }
}

fn toggle_info() {
    let Some(f) = files() else { return };
    let on = !f.info_rev.reveals_child();
    f.info_rev.set_reveal_child(on);
    let _ = kit::prefs::set("files_info", toml::Value::Boolean(on));
    if on {
        if let Some(p) = active() {
            f.info.show(&p.selected(), &p.location());
        }
    }
}

fn in_view() -> bool {
    let Some(f) = files() else { return false };
    let Some(focus) = gtk::prelude::GtkWindowExt::focus(&f.window) else { return true };
    !(focus.is::<gtk::Text>() || focus.ancestor(gtk::Entry::static_type()).is_some() || focus.ancestor(gtk::SearchEntry::static_type()).is_some() || focus.is::<gtk::TextView>())
}

fn action(window: &gtk::ApplicationWindow, name: &str, f: impl Fn() + 'static) {
    let a = gio::SimpleAction::new(name, None);
    a.connect_activate(move |_, _| f());
    window.add_action(&a);
}

fn actions(window: &gtk::ApplicationWindow) {
    action(window, "back", || active().inspect(|p| p.back()).map(|_| ()).unwrap_or(()));
    action(window, "forward", || active().inspect(|p| p.forward()).map(|_| ()).unwrap_or(()));
    action(window, "up", || active().inspect(|p| p.up()).map(|_| ()).unwrap_or(()));
    action(window, "home", || go(&gio::File::for_path(glib::home_dir())));
    action(window, "new-tab", || new_tab(&active().map(|p| p.location()).unwrap_or_else(|| gio::File::for_path(glib::home_dir()))));
    action(window, "new-window", || {
        let here = active().map(|p| p.location()).unwrap_or_else(|| gio::File::for_path(glib::home_dir()));
        let exe = std::path::PathBuf::from("/proc/self/exe");
        let arg = here.path().map(|p| p.into_os_string()).unwrap_or_else(|| here.uri().to_string().into());
        let _ = gio::Subprocess::newv(&[exe.as_os_str(), arg.as_os_str()], gio::SubprocessFlags::NONE);
    });
    action(window, "close-tab", || {
        if let Some(t) = tab() {
            close_tab(t.pane.root.upcast_ref());
        }
    });
    action(window, "split", toggle_split);
    action(window, "info", toggle_info);
    action(window, "sidebar", || {
        if let Some(f) = files() {
            f.side_rev.set_reveal_child(!f.side_rev.reveals_child());
        }
    });
    action(window, "location", edit_path);
    action(window, "search", || {
        if let Some(f) = files() {
            f.search.grab_focus();
        }
    });
    action(window, "view-grid", || set_view(View::Grid));
    action(window, "view-list", || set_view(View::List));
    action(window, "view-toggle", || set_view(if active().is_some_and(|p| p.view() == View::Grid) { View::List } else { View::Grid }));
    action(window, "zoom-in", || active().inspect(|p| p.zoom_by(1)).map(|_| ()).unwrap_or(()));
    action(window, "zoom-out", || active().inspect(|p| p.zoom_by(-1)).map(|_| ()).unwrap_or(()));
    action(window, "reload", || active().inspect(|p| p.refresh()).map(|_| ()).unwrap_or(()));
    action(window, "open", || {
        if in_view() {
            if let Some(p) = active() {
                p.open_selected();
            }
        }
    });
    action(window, "open-with", || {
        if let Some(p) = active() {
            ops::open_with(p.selected().into_iter().filter(|i| !pane::is_dir(i)).collect());
        }
    });
    action(window, "open-tab", || {
        if let Some(p) = active() {
            for i in p.selected().iter().filter(|i| pane::is_dir(i)) {
                if let Some(f) = pane::target_of(i) {
                    new_tab(&f);
                }
            }
        }
    });
    action(window, "copy", || {
        if in_view() {
            if let Some(p) = active() {
                ops::clip(&p.selected_files(), false);
            }
        }
    });
    action(window, "cut", || {
        if in_view() {
            if let Some(p) = active() {
                ops::clip(&p.selected_files(), true);
            }
        }
    });
    action(window, "paste", || {
        if in_view() {
            if let Some(p) = active() {
                ops::paste(p.location());
            }
        }
    });
    action(window, "paste-into", || {
        if let Some(p) = active() {
            let dest = match p.selected().as_slice() {
                [one] if pane::is_dir(one) => pane::target_of(one).unwrap_or(p.location()),
                _ => p.location(),
            };
            ops::paste(dest);
        }
    });
    action(window, "copy-path", || {
        if let Some(p) = active() {
            let files = p.selected_files();
            ops::copy_paths(if files.is_empty() { std::slice::from_ref(&*Box::leak(Box::new(p.location()))) } else { &files });
        }
    });
    action(window, "select-all", || {
        if in_view() {
            if let Some(p) = active() {
                p.sel.select_all();
            }
        }
    });
    action(window, "trash", || {
        if in_view() {
            if let Some(p) = active() {
                if ops::pane_of_trash(&p) {
                    ops::delete(p.selected_files());
                } else {
                    ops::trash(p.selected_files());
                }
            }
        }
    });
    action(window, "delete", || {
        if in_view() {
            if let Some(p) = active() {
                ops::delete(p.selected_files());
            }
        }
    });
    action(window, "restore", || {
        if let Some(p) = active() {
            ops::restore(p.selected());
        }
    });
    action(window, "empty-trash", ops::empty_trash);
    action(window, "rename", || {
        if !in_view() {
            return;
        }
        let Some(p) = active() else { return };
        let picked = p.selected();
        match picked.len() {
            0 => {}
            1 => ops::rename(picked[0].clone()),
            _ => ops::batch_rename(picked),
        }
    });
    action(window, "duplicate", || {
        if let Some(p) = active() {
            ops::run("Duplicating", ops::Plan::Copy(p.selected_files(), p.location()));
        }
    });
    action(window, "new-folder", || {
        if let Some(p) = active() {
            ops::new_folder(p.location());
        }
    });
    action(window, "compress", || {
        if let Some(p) = active() {
            ops::compress(p.selected_files());
        }
    });
    action(window, "extract", || {
        if let Some(p) = active() {
            ops::extract(p.selected().iter().filter(|i| ops::archive(i)).filter_map(pane::file_of).collect());
        }
    });
    action(window, "undo", || {
        if in_view() {
            ops::undo();
        }
    });
    action(window, "terminal", || {
        if let Some(p) = active() {
            let here = match p.selected().as_slice() {
                [one] if pane::is_dir(one) => pane::target_of(one).unwrap_or(p.location()),
                _ => p.location(),
            };
            ops::terminal(&here);
        }
    });
    action(window, "bookmark", || {
        if let Some(p) = active() {
            let target = match p.selected().as_slice() {
                [one] if pane::is_dir(one) => pane::target_of(one).unwrap_or(p.location()),
                _ => p.location(),
            };
            side::toggle_bookmark(&target);
        }
    });
    action(window, "look", look::toggle);
    action(window, "send", || {
        if let Some(p) = active() {
            tools::send(p.selected_files());
        }
    });
    action(window, "shrink", || {
        if let Some(p) = active() {
            tools::shrink(p.selected());
        }
    });
    action(window, "cutout", || {
        if let Some(p) = active() {
            tools::cutout(p.selected());
        }
    });
    let convert = gio::SimpleAction::new("convert", Some(glib::VariantTy::STRING));
    convert.connect_activate(|_, v| {
        let (Some(p), Some(ext)) = (active(), v.and_then(|v| v.get::<String>())) else { return };
        tools::convert(p.selected(), &ext);
    });
    window.add_action(&convert);
    action(window, "copy-other", || transfer_other(false));
    action(window, "move-other", || transfer_other(true));
    action(window, "quit", || {
        if let Some(f) = files() {
            f.window.close();
        }
    });
    let hidden = gio::SimpleAction::new_stateful("hidden", None, &kit::prefs::flag(&kit::prefs::read(), "files_hidden", false).to_variant());
    hidden.connect_activate(|a, _| {
        let on = !a.state().and_then(|s| s.get::<bool>()).unwrap_or(false);
        a.set_state(&on.to_variant());
        let _ = kit::prefs::set("files_hidden", toml::Value::Boolean(on));
        for p in panes() {
            p.set_hidden(on);
        }
    });
    window.add_action(&hidden);
    let sort = gio::SimpleAction::new_stateful("sort", Some(glib::VariantTy::STRING), &kit::prefs::text(&kit::prefs::read(), "files_sort", "name").to_variant());
    sort.connect_activate(|a, v| {
        let Some(v) = v.and_then(|v| v.get::<String>()) else { return };
        a.set_state(&v.to_variant());
        let _ = kit::prefs::set("files_sort", toml::Value::String(v.clone()));
        apply_sort(&v);
    });
    window.add_action(&sort);
    let thumbs = gio::SimpleAction::new_stateful("thumbs", Some(glib::VariantTy::STRING), &kit::prefs::text(&kit::prefs::read(), "files_thumbnails", "local").to_variant());
    thumbs.connect_activate(|a, v| {
        let Some(v) = v.and_then(|v| v.get::<String>()) else { return };
        a.set_state(&v.to_variant());
        let _ = kit::prefs::set("files_thumbnails", toml::Value::String(v));
        toast("Thumbnail setting saved");
    });
    window.add_action(&thumbs);
    let newfile = gio::SimpleAction::new("new-file", Some(glib::VariantTy::STRING));
    newfile.connect_activate(|_, v| {
        let Some(p) = active() else { return };
        let t = v.and_then(|v| v.get::<String>()).unwrap_or_default();
        ops::new_file(p.location(), (!t.is_empty()).then(|| std::path::PathBuf::from(t)));
    });
    window.add_action(&newfile);
}

fn transfer_other(moving: bool) {
    let (Some(p), Some(o)) = (active(), other()) else {
        toast("Open a split view first (F3)");
        return;
    };
    let files = p.selected_files();
    if files.is_empty() {
        return;
    }
    if moving {
        ops::run("Moving", ops::Plan::Move(files, o.location()));
    } else {
        ops::run("Copying", ops::Plan::Copy(files, o.location()));
    }
}

fn apply_sort(v: &str) {
    let (col, desc) = match v {
        "size" => (1, true),
        "modified" => (2, true),
        "kind" => (3, false),
        "name-desc" => (0, true),
        _ => (0, false),
    };
    for p in panes() {
        p.sort_by(col, desc);
    }
}

fn set_view(v: View) {
    if let Some(p) = active() {
        p.set_view(v);
        let _ = kit::prefs::set("files_view", toml::Value::String(if v == View::Grid { "grid".into() } else { "list".into() }));
        p.focus();
        sync();
    }
}

fn accels(app: &gtk::Application) {
    for (a, keys) in [
        ("win.back", &["<Alt>Left", "<Alt>KP_Left"][..]),
        ("win.forward", &["<Alt>Right"]),
        ("win.up", &["<Alt>Up"]),
        ("win.home", &["<Alt>Home"]),
        ("win.new-tab", &["<Control>t"]),
        ("win.new-window", &["<Control>n"]),
        ("win.close-tab", &["<Control>w"]),
        ("win.split", &["F3"]),
        ("win.info", &["<Control>i", "F9"]),
        ("win.sidebar", &["<Control>b"]),
        ("win.location", &["<Control>l"]),
        ("win.search", &["<Control>f"]),
        ("win.view-grid", &["<Control>1"]),
        ("win.view-list", &["<Control>2"]),
        ("win.zoom-in", &["<Control>plus", "<Control>equal", "<Control>KP_Add"]),
        ("win.zoom-out", &["<Control>minus", "<Control>KP_Subtract"]),
        ("win.reload", &["F5", "<Control>r"]),
        ("win.copy", &["<Control>c"]),
        ("win.cut", &["<Control>x"]),
        ("win.paste", &["<Control>v"]),
        ("win.copy-path", &["<Control><Shift>c"]),
        ("win.trash", &["Delete"]),
        ("win.delete", &["<Shift>Delete"]),
        ("win.rename", &["F2"]),
        ("win.new-folder", &["<Control><Shift>n"]),
        ("win.undo", &["<Control>z"]),
        ("win.terminal", &["F4"]),
        ("win.bookmark", &["<Control>d"]),
        ("win.hidden", &["<Control>h"]),
        ("win.open", &["<Control>o"]),
        ("win.copy-other", &["F7"]),
        ("win.move-other", &["F8"]),
        ("win.quit", &["<Control>q"]),
    ] {
        app.set_accels_for_action(a, keys);
    }
}

pub fn menu(p: &Rc<Pane>, at: Option<(f64, f64)>, items: bool) {
    use popup::{act, act_with, danger, sub, Item};
    let mut m: Vec<Item> = Vec::new();
    let trash = ops::pane_of_trash(p);
    let picked = p.selected();
    if items && !picked.is_empty() {
        if trash {
            m.push(act("restore_from_trash", "Restore", "win.restore"));
            m.push(danger("delete_forever", "Delete Permanently", "win.delete"));
        } else {
            m.push(act("open_in_new", "Open", "win.open"));
            if picked.iter().any(|i| !pane::is_dir(i)) {
                m.push(act("apps", "Open With…", "win.open-with"));
            }
            if picked.iter().any(pane::is_dir) {
                m.push(act("tab", "Open in New Tab", "win.open-tab"));
            }
            m.push(act("visibility", "Quick Look", "win.look"));
            m.push(Item::Sep);
            m.push(act("content_cut", "Cut", "win.cut"));
            m.push(act("content_copy", "Copy", "win.copy"));
            if picked.len() == 1 && pane::is_dir(&picked[0]) {
                m.push(act("content_paste_go", "Paste Into Folder", "win.paste-into"));
            }
            m.push(act("link", "Copy Path", "win.copy-path"));
            if picked.iter().any(|i| !pane::is_dir(i)) {
                m.push(act("share", "Send to Device…", "win.send"));
            }
            if other().is_some() {
                m.push(act("file_copy", "Copy to Other Pane", "win.copy-other"));
                m.push(act("drive_file_move", "Move to Other Pane", "win.move-other"));
            }
            m.push(Item::Sep);
            m.push(act("edit", if picked.len() > 1 { "Rename Items…" } else { "Rename…" }, "win.rename"));
            m.push(act("library_add", "Duplicate", "win.duplicate"));
            m.push(act("folder_zip", "Compress…", "win.compress"));
            let conv = tools::targets(&picked);
            if !conv.is_empty() {
                let items = conv.into_iter().map(|(label, ext)| act_with("transform", label, "win.convert", ext.to_variant())).collect();
                m.push(sub("transform", "Convert To", items));
            }
            if tools::shrinkable(&picked) {
                m.push(act("compress", "Make Smaller", "win.shrink"));
            }
            if picked.iter().all(|i| tools::family(i) == tools::Family::Image) {
                m.push(act("auto_fix_high", "Remove Background", "win.cutout"));
            }
            if picked.iter().any(ops::archive) {
                m.push(act("unarchive", "Extract Here", "win.extract"));
            }
            if picked.len() == 1 && pane::is_dir(&picked[0]) {
                m.push(act("bookmark_add", "Add to Sidebar", "win.bookmark"));
                m.push(act("terminal", "Open Terminal Here", "win.terminal"));
            }
            m.push(Item::Sep);
            m.push(danger("delete", "Move to Trash", "win.trash"));
            m.push(act("info", "Get Info", "win.info"));
        }
    } else if trash {
        m.push(danger("delete_sweep", "Empty Trash", "win.empty-trash"));
        m.push(act("select_all", "Select All", "win.select-all"));
    } else {
        m.push(act("create_new_folder", "New Folder…", "win.new-folder"));
        let mut files = vec![act_with("note_add", "Empty File", "win.new-file", "".to_variant())];
        for t in ops::templates() {
            let label = t.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            files.push(act_with("description", &label, "win.new-file", t.to_string_lossy().to_variant()));
        }
        m.push(sub("note_add", "New File", files));
        m.push(act("content_paste", "Paste", "win.paste"));
        m.push(act("select_all", "Select All", "win.select-all"));
        m.push(Item::Sep);
        m.push(act("terminal", "Open Terminal Here", "win.terminal"));
        m.push(act("bookmark_add", "Add to Sidebar", "win.bookmark"));
        m.push(act("visibility", "Show Hidden Files", "win.hidden"));
        let sorts = [("Name", "name"), ("Name, Z to A", "name-desc"), ("Size", "size"), ("Modified", "modified"), ("Kind", "kind")]
            .into_iter()
            .map(|(label, key)| act_with("sort", label, "win.sort", key.to_variant()))
            .collect();
        m.push(sub("sort", "Sort By", sorts));
        m.push(Item::Sep);
        m.push(act("info", "Get Info", "win.info"));
    }
    let (x, y) = at.unwrap_or((p.root.width() as f64 / 2.0, 80.0));
    popup::show(&p.root, x, y, m);
}

fn app_menu() -> Vec<popup::Item> {
    use popup::{Item, act, act_with, sub};
    let sorts = [("sort_by_alpha", "Name", "name"), ("sort_by_alpha", "Name, Z to A", "name-desc"), ("straighten", "Size", "size"), ("schedule", "Modified", "modified"), ("category", "Kind", "kind")];
    let thumbs = [("public", "Everywhere", "all"), ("computer", "This Computer Only", "local"), ("hide_image", "Never", "off")];
    vec![
        act("open_in_new", "New Window", "win.new-window"),
        act("tab", "New Tab", "win.new-tab"),
        act("vertical_split", "Split View", "win.split"),
        act("info", "Info Panel", "win.info"),
        act("side_navigation", "Sidebar", "win.sidebar"),
        Item::Sep,
        act("visibility", "Show Hidden Files", "win.hidden"),
        sub("sort", "Sort By", sorts.iter().map(|(g, l, k)| act_with(g, l, "win.sort", k.to_variant())).collect()),
        sub("image", "Thumbnails", thumbs.iter().map(|(g, l, k)| act_with(g, l, "win.thumbs", k.to_variant())).collect()),
        Item::Sep,
        act("undo", "Undo", "win.undo"),
        act("terminal", "Open Terminal Here", "win.terminal"),
        act("keyboard", "Keyboard Shortcuts", "win.shortcuts"),
    ]
}

fn shortcuts() {
    let (w, body, buttons) = ops::dialog("Keyboard Shortcuts", "");
    let grid = gtk::Grid::new();
    grid.set_column_spacing(24);
    grid.set_row_spacing(6);
    let list = [
        ("Space", "Quick Look"),
        ("Type letters", "Jump to a file"),
        ("Ctrl+F", "Search (subfolders too)"),
        ("Ctrl+L", "Type a path · Tab completes"),
        ("Backspace / Alt+↑", "Parent folder"),
        ("Alt+← / Alt+→", "Back / forward"),
        ("Ctrl+T · Ctrl+W", "New tab · close tab"),
        ("Ctrl+Tab · Ctrl+Shift+Tab", "Next tab · previous tab"),
        ("F3", "Split view"),
        ("F7 · F8", "Copy · move to other pane"),
        ("F2", "Rename (many items: batch)"),
        ("Ctrl+Shift+N", "New folder"),
        ("Ctrl+C · X · V", "Copy · cut · paste (images and text too)"),
        ("Ctrl+Z", "Undo"),
        ("Delete · Shift+Delete", "Trash · delete permanently"),
        ("Ctrl+H", "Hidden files"),
        ("Ctrl+1 · Ctrl+2", "Grid · list"),
        ("Ctrl+scroll · Ctrl+±", "Icon size"),
        ("F4", "Terminal here"),
        ("Ctrl+I", "Info panel"),
        ("Ctrl+D", "Add to sidebar"),
    ];
    for (i, (k, v)) in list.iter().enumerate() {
        let kl = kit::ui::text(k, "badge");
        kl.set_halign(gtk::Align::Start);
        grid.attach(&kl, 0, i as i32, 1, 1);
        grid.attach(&kit::ui::text(v, ""), 1, i as i32, 1, 1);
    }
    body.append(&grid);
    let ok = kit::ui::button("Done", "accent");
    let ww = w.clone();
    ok.connect_clicked(move |_| ww.close());
    buttons.append(&ok);
    w.present();
}

fn header(window: &gtk::ApplicationWindow) -> (gtk::Widget, gtk::Box, gtk::Stack, gtk::Entry, gtk::SearchEntry, [gtk::Button; 4]) {
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    bar.add_css_class("topbar");
    let side = kit::app::control("side_navigation", "");
    side.set_action_name(Some("win.sidebar"));
    side.set_tooltip_text(Some("Sidebar (Ctrl+B)"));
    let back = kit::app::control("arrow_back", "");
    back.set_action_name(Some("win.back"));
    let forward = kit::app::control("arrow_forward", "");
    forward.set_action_name(Some("win.forward"));
    let up = kit::app::control("arrow_upward", "");
    up.set_action_name(Some("win.up"));
    back.set_tooltip_text(Some("Back (Alt+←)"));
    forward.set_tooltip_text(Some("Forward (Alt+→)"));
    up.set_tooltip_text(Some("Parent folder (Backspace)"));
    for b in [&side, &back, &forward, &up] {
        bar.append(b);
    }
    let crumbs = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    crumbs.add_css_class("crumbs");
    let cs = gtk::ScrolledWindow::new();
    cs.set_vscrollbar_policy(gtk::PolicyType::Never);
    cs.set_hscrollbar_policy(gtk::PolicyType::External);
    cs.set_child(Some(&crumbs));
    cs.set_hexpand(true);
    let cadj = cs.hadjustment();
    cadj.connect_changed(|a| a.set_value(a.upper() - a.page_size()));
    let click = gtk::GestureClick::new();
    click.connect_released(|g, n, x, y| {
        let Some(w) = g.widget() else { return };
        let on_button = w.pick(x, y, gtk::PickFlags::DEFAULT).is_some_and(|p| p.is::<gtk::Button>() || p.ancestor(gtk::Button::static_type()).is_some());
        if !on_button || n == 2 {
            edit_path();
        }
    });
    cs.add_controller(click);
    let entry = gtk::Entry::new();
    entry.add_css_class("search");
    entry.set_hexpand(true);
    entry.set_placeholder_text(Some("Type a path, ~/, or sftp://host"));
    entry.connect_activate(|e| {
        let text = e.text().to_string();
        let Some(f) = resolve(&text) else { return };
        let ft = f.query_file_type(gio::FileQueryInfoFlags::NONE, None::<&gio::Cancellable>);
        match ft {
            gio::FileType::Directory | gio::FileType::Mountable => go(&f),
            gio::FileType::Unknown if f.uri_scheme().is_some_and(|s| s != "file") => {
                let op = gtk::MountOperation::new(window_of(e).as_ref());
                let t = f.clone();
                f.mount_enclosing_volume(gio::MountMountFlags::NONE, Some(&op), None::<&gio::Cancellable>, move |r| match r {
                    Ok(()) => go(&t),
                    Err(e) if e.matches(gio::IOErrorEnum::AlreadyMounted) => go(&t),
                    Err(e) => toast(&e.message().to_string()),
                });
            }
            gio::FileType::Unknown => {
                toast("That folder doesn't exist");
                return;
            }
            _ => {
                if let Some(parent) = f.parent() {
                    go(&parent);
                    select_later(&f);
                }
            }
        }
        if let Some(f) = files() {
            f.path_stack.set_visible_child_name("crumbs");
        }
    });
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed(|_, k, _, _| match k {
        gdk::Key::Escape => {
            if let (Some(f), Some(p)) = (files(), active()) {
                f.path_stack.set_visible_child_name("crumbs");
                p.focus();
            }
            glib::Propagation::Stop
        }
        gdk::Key::Tab => {
            complete();
            glib::Propagation::Stop
        }
        _ => glib::Propagation::Proceed,
    });
    entry.add_controller(keys);
    let focus = gtk::EventControllerFocus::new();
    focus.connect_leave(|_| {
        glib::idle_add_local_once(|| {
            if let Some(f) = files() {
                if !f.path_entry.has_focus() && !f.path_entry.is_focus() {
                    f.path_stack.set_visible_child_name("crumbs");
                }
            }
        });
    });
    entry.add_controller(focus);
    let stack = gtk::Stack::new();
    stack.set_hexpand(true);
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_transition_duration(120);
    stack.add_named(&cs, Some("crumbs"));
    stack.add_named(&entry, Some("entry"));
    let handle = gtk::WindowHandle::new();
    handle.set_child(Some(&stack));
    handle.set_hexpand(true);
    bar.append(&handle);
    let search = gtk::SearchEntry::new();
    search.add_css_class("search");
    search.set_placeholder_text(Some("Search"));
    search.set_width_chars(18);
    search.set_search_delay(220);
    search.connect_search_changed(|s| {
        if let Some(p) = active() {
            let q = s.text().to_string();
            p.search(&q, q.chars().count() >= 2);
        }
    });
    search.connect_stop_search(|s| {
        s.set_text("");
        if let Some(p) = active() {
            p.search("", false);
            p.focus();
        }
    });
    search.connect_activate(|_| {
        if let Some(p) = active() {
            p.focus();
            if p.sorted.n_items() > 0 {
                p.reveal(0);
            }
        }
    });
    bar.append(&search);
    let view = kit::app::control("view_list", "");
    view.set_action_name(Some("win.view-toggle"));
    view.set_tooltip_text(Some("Grid or list (Ctrl+1 / Ctrl+2)"));
    bar.append(&view);
    let split = kit::app::control("vertical_split", "");
    split.set_action_name(Some("win.split"));
    split.set_tooltip_text(Some("Split view (F3)"));
    bar.append(&split);
    let infob = kit::app::control("info", "");
    infob.set_action_name(Some("win.info"));
    infob.set_tooltip_text(Some("Info (Ctrl+I)"));
    bar.append(&infob);
    let more = kit::app::control("more_horiz", "");
    more.add_css_class("ctl-menu");
    more.connect_clicked(|b| popup::show_under(b, app_menu()));
    bar.append(&more);
    bar.append(&kit::app::controls(window, APP_ID));
    (bar.upcast(), crumbs, stack, entry, search, [back, forward, up, view])
}

fn window_of(w: &impl IsA<gtk::Widget>) -> Option<gtk::Window> {
    w.root().and_downcast::<gtk::Window>()
}

fn css() -> String {
    let i = kit::style::ink();
    let (main, meta, accent, on_accent) = (&i.main, &i.meta, &i.accent, &i.on_accent);
    format!(
        r#"{base}
.topbar {{ padding: 8px 10px 8px 10px; }}
button.crumb {{ background: transparent; border: none; box-shadow: none; color: {meta}; padding: 4px 8px; border-radius: 10px; min-height: 22px; font-weight: 520; }}
button.crumb:hover {{ background: alpha({main}, 0.08); color: {main}; }}
button.crumb.here {{ color: {main}; }}
.crumb-sep {{ font-size: 16px; color: alpha({meta}, 0.6); }}
.crumbs {{ padding: 0 4px; }}
menubutton.ctl-menu > button {{ background: transparent; border: none; box-shadow: none; border-radius: 999px; min-width: 32px; min-height: 32px; padding: 0; color: {meta}; }}
menubutton.ctl-menu > button:hover {{ background: alpha({main}, 0.10); color: {main}; }}
.side {{ background: @d_base; padding: 6px 8px; }}
list.places {{ background: transparent; }}
list.places row {{ border-radius: 12px; padding: 7px 10px; margin: 1px 0; color: {meta}; background: transparent; }}
list.places row:hover {{ background: alpha({main}, 0.06); }}
list.places row:selected {{ background: alpha({main}, 0.12); color: {main}; }}
.side-head {{ font-size: 12px; font-weight: 600; color: alpha({meta}, 0.75); margin: 12px 4px 2px 4px; }}
.place-icon {{ font-size: 18px; color: {accent}; }}
list.places row button.ctl {{ min-width: 24px; min-height: 24px; }}
.pane {{ background: #050505; border-radius: 16px; margin: 0 8px 8px 0; }}
.pane.focused {{ box-shadow: inset 0 0 0 1px alpha({accent}, 0.35); }}
gridview.grid, columnview.list, columnview.list listview {{ background: transparent; color: {main}; }}
gridview.grid {{ padding: 12px; }}
gridview.grid > child {{ border-radius: 0; padding: 8px 4px; margin: 2px; background: transparent; outline: none; box-shadow: none; }}
gridview.grid > child:hover, gridview.grid > child:selected, gridview.grid > child:focus {{ background: transparent; }}
.thumbbox {{ border-radius: 14px; }}
gridview.grid > child:hover .thumbbox {{ background: alpha({main}, 0.05); }}
gridview.grid > child:selected .thumbbox, gridview.grid > child:active .thumbbox {{ background: alpha({main}, 0.13); }}
gridview.grid > child:active label.fname {{ background: alpha({accent}, 0.75); color: {on_accent}; }}
gridview.grid > child:focus-visible .thumbbox {{ box-shadow: inset 0 0 0 1.5px alpha({accent}, 0.7); }}
label.fname {{ font-size: 13px; padding: 1px 7px; border-radius: 7px; }}
gridview.grid > child:selected label.fname {{ background: {accent}; color: {on_accent}; }}
columnview.list > header > button {{ background: transparent; border: none; box-shadow: none; color: {meta}; font-size: 12px; font-weight: 600; padding: 6px 10px; }}
columnview.list > listview > row {{ border-radius: 10px; margin: 1px 6px; }}
popover.pm > contents {{ padding: 5px; border-radius: 14px; background: @d_pop; box-shadow: 0 10px 30px rgba(0,0,0,0.35), 0 0 0 1px alpha({main}, 0.07); }}
button.pm-row {{ min-height: 30px; padding: 0 10px 0 8px; border-radius: 8px; border: none; box-shadow: none; background: transparent; color: {main}; }}
button.pm-row {{ outline: none; }}
button.pm-row:hover, button.pm-row:focus-visible {{ background: alpha({main}, 0.08); outline: none; box-shadow: none; }}
button.pm-row:active {{ background: alpha({accent}, 0.25); }}
button.pm-row:disabled {{ opacity: 0.4; }}
button.pm-row .pm-glyph {{ font-size: 17px; color: {meta}; min-width: 20px; }}
button.pm-row .pm-label {{ font-size: 13.5px; }}
button.pm-row .pm-accel {{ font-size: 12px; color: {meta}; margin-left: 20px; }}
button.pm-row .pm-trail {{ font-size: 17px; color: {meta}; margin-left: 8px; }}
button.pm-row.pm-danger .pm-label, button.pm-row.pm-danger .pm-glyph {{ color: #ff6b6b; }}
button.pm-row.pm-back .pm-label {{ font-weight: 600; }}
separator.pm-sep {{ margin: 4px 8px; background: alpha({main}, 0.08); min-height: 1px; }}
.place-remove {{ opacity: 0; min-width: 22px; min-height: 22px; padding: 0; }}
list.places row:hover .place-remove {{ opacity: 1; }}
columnview.list > listview > row:hover {{ background: alpha({main}, 0.05); }}
columnview.list > listview > row:selected, columnview.list > listview > row:active {{ background: alpha({accent}, 0.22); color: {main}; }}
columnview.list > listview > row > cell {{ padding: 5px 10px; }}
.cell:drop(active) .thumbbox {{ background: alpha({accent}, 0.20); box-shadow: inset 0 0 0 1.5px alpha({accent}, 0.70); }}
.cell:drop(active) label.fname {{ background: alpha({accent}, 0.80); color: {on_accent}; }}
.namecell:drop(active) {{ background: alpha({accent}, 0.18); border-radius: 8px; box-shadow: inset 0 0 0 1.5px alpha({accent}, 0.55); }}
list.places row:drop(active) {{ background: alpha({accent}, 0.18); color: {main}; }}
button.crumb:drop(active) {{ background: alpha({accent}, 0.20); color: {main}; }}
.pane:drop(active), scrolledwindow:drop(active) {{ box-shadow: inset 0 0 0 1.5px alpha({accent}, 0.40); }}
rubberband {{ background: alpha({accent}, 0.18); border: 1px solid alpha({accent}, 0.6); border-radius: 6px; }}
.status {{ color: {meta}; font-size: 12px; padding: 6px 14px 8px 14px; }}
.empty-icon {{ font-size: 64px; color: alpha({meta}, 0.5); }}
.info {{ background: #050505; border-radius: 16px; margin: 0 8px 8px 0; }}
.info-body {{ padding: 20px 18px; }}
.info-art {{ margin: 6px 0; }}
.name.big {{ font-size: 17px; font-weight: 600; }}
.job {{ padding: 6px 4px; }}
.jobs {{ background: #0d0d0d; border-radius: 16px; padding: 8px 14px; margin: 0 8px 8px 8px; }}
.toast {{ background: @d_raised; color: {main}; border-radius: 999px; padding: 9px 18px; margin-bottom: 26px; box-shadow: 0 8px 24px rgba(0,0,0,0.5); }}
.trash-bar {{ padding: 0 8px 8px 8px; }}
.look-backdrop {{ background: rgba(0,0,0,0.62); }}
.look {{ background: #0e0e0e; border-radius: 26px; padding: 16px 18px 18px 18px; box-shadow: 0 24px 60px rgba(0,0,0,0.6); }}
.stage {{ border-radius: 16px; }}
textview.look-text, textview.look-text text {{ background: #070707; color: {main}; font-size: 13px; }}
notebook > header {{ background: transparent; border: none; box-shadow: none; padding: 0 8px; }}
notebook > header > tabs > tab {{ background: transparent; border-radius: 12px; padding: 4px 10px; margin: 0 2px 6px 0; color: {meta}; border: none; box-shadow: none; outline: none; min-height: 24px; }}
notebook > header > tabs > tab:checked {{ background: alpha({main}, 0.10); color: {main}; }}
notebook > stack {{ background: transparent; }}
notebook {{ background: transparent; }}
button.tab-close {{ min-width: 20px; min-height: 20px; }}
button.tab-close .sym {{ font-size: 14px; }}
paned > separator {{ background: transparent; min-width: 4px; }}
list.picker {{ background: transparent; }}
list.picker row {{ border-radius: 12px; }}
button.flat-row {{ background: transparent; border: none; box-shadow: none; padding: 0; border-radius: 14px; }}
button.flat-row:hover {{ background: alpha({main}, 0.06); }}
spinbutton {{ background: alpha({main}, 0.08); border-radius: 10px; border: none; box-shadow: none; color: {main}; }}
checkbutton check {{ border-radius: 6px; }}
checkbutton check:checked {{ background: {accent}; color: {on_accent}; }}
"#,
        base = kit::style::css()
    )
}

fn build(app: &gtk::Application, start: &gio::File) -> Rc<Files> {
    let window = gtk::ApplicationWindow::builder().application(app).title("Files").default_width(1180).default_height(760).build();
    window.set_decorated(false);
    window.add_css_class("digit-settings");
    window.add_css_class("digit-files");
    actions(&window);
    let (head, crumbs, path_stack, path_entry, search, [back, forward, up, view_btn]) = header(&window);
    let side = side::Side::new();
    let side_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    side_box.add_css_class("side");
    side_box.set_size_request(208, -1);
    side_box.append(&side.root);
    let side_rev = gtk::Revealer::new();
    side_rev.set_transition_type(gtk::RevealerTransitionType::SlideRight);
    side_rev.set_transition_duration(160);
    side_rev.set_child(Some(&side_box));
    side_rev.set_reveal_child(true);
    let first = new_column();
    let split = gtk::Paned::new(gtk::Orientation::Horizontal);
    split.set_wide_handle(false);
    split.set_start_child(Some(&first.notebook));
    split.set_resize_start_child(true);
    split.set_shrink_start_child(false);
    split.set_resize_end_child(true);
    split.set_shrink_end_child(false);
    let info = info::Info::new();
    let info_rev = gtk::Revealer::new();
    info_rev.set_transition_type(gtk::RevealerTransitionType::SlideLeft);
    info_rev.set_transition_duration(160);
    info_rev.set_child(Some(&info.root));
    info_rev.set_reveal_child(kit::prefs::flag(&kit::prefs::read(), "files_info", false));
    let trash_bar_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    trash_bar_box.add_css_class("trash-bar");
    trash_bar_box.append(&kit::ui::text("Items in the trash can be restored to where they were.", "sub"));
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    trash_bar_box.append(&spacer);
    let rb = kit::ui::button("Restore", "");
    rb.set_action_name(Some("win.restore"));
    let eb = kit::ui::button("Empty Trash", "danger");
    eb.set_action_name(Some("win.empty-trash"));
    trash_bar_box.append(&rb);
    trash_bar_box.append(&eb);
    let trash_bar = gtk::Revealer::new();
    trash_bar.set_child(Some(&trash_bar_box));
    let center = gtk::Box::new(gtk::Orientation::Vertical, 0);
    center.append(&trash_bar);
    center.append(&split);
    let jobs = gtk::Box::new(gtk::Orientation::Vertical, 4);
    jobs.add_css_class("jobs");
    let jobs_rev = gtk::Revealer::new();
    jobs_rev.set_transition_type(gtk::RevealerTransitionType::SlideUp);
    jobs_rev.set_child(Some(&jobs));
    center.append(&jobs_rev);
    let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    body.append(&side_rev);
    body.append(&center);
    body.append(&info_rev);
    let main = gtk::Box::new(gtk::Orientation::Vertical, 0);
    main.append(&head);
    main.append(&body);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&main));
    let toast_label = kit::ui::text("", "toast");
    let toast_rev = gtk::Revealer::new();
    toast_rev.set_transition_type(gtk::RevealerTransitionType::Crossfade);
    toast_rev.set_child(Some(&toast_label));
    toast_rev.set_halign(gtk::Align::Center);
    toast_rev.set_valign(gtk::Align::End);
    toast_rev.set_can_target(false);
    overlay.add_overlay(&toast_rev);
    window.set_child(Some(&overlay));
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed(|_, k, _, m| {
        if k == gdk::Key::Escape && look::open() {
            look::close();
            return glib::Propagation::Stop;
        }
        if m.contains(gdk::ModifierType::CONTROL_MASK) && matches!(k, gdk::Key::Tab | gdk::Key::ISO_Left_Tab | gdk::Key::KP_Tab) {
            cycle_tab(if m.contains(gdk::ModifierType::SHIFT_MASK) || k == gdk::Key::ISO_Left_Tab { -1 } else { 1 });
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(keys);
    action(&window, "shortcuts", shortcuts);
    let f = Rc::new(Files {
        window: window.clone(),
        split,
        columns: RefCell::new(vec![first]),
        current: Cell::new(0),
        crumbs,
        path_stack,
        path_entry,
        search,
        back,
        forward,
        up,
        view_btn,
        side,
        side_rev,
        info,
        info_rev,
        jobs,
        jobs_rev,
        toast: toast_label,
        toast_rev,
        toast_gen: Cell::new(0),
        overlay,
        trash_bar,
        hold: RefCell::new(None),
    });
    FILES.with(|s| *s.borrow_mut() = Some(f.clone()));
    new_tab(start);
    apply_sort(&kit::prefs::text(&kit::prefs::read(), "files_sort", "name"));
    window.connect_close_request(|_| {
        look::close();
        glib::Propagation::Proceed
    });
    window.connect_destroy(|_| {
        FILES.with(|s| s.borrow_mut().take());
    });
    f
}

fn open_paths(app: &gtk::Application, targets: Vec<gio::File>, select: bool) {
    let targets = if targets.is_empty() { vec![gio::File::for_path(glib::home_dir())] } else { targets };
    let mut first = true;
    for t in targets {
        let (dir, pick) = match t.query_file_type(gio::FileQueryInfoFlags::NONE, None::<&gio::Cancellable>) {
            gio::FileType::Directory | gio::FileType::Mountable if !select => (t.clone(), None),
            gio::FileType::Unknown if t.uri_scheme().is_some_and(|s| s != "file") => (t.clone(), None),
            _ => (t.parent().unwrap_or_else(|| t.clone()), Some(t.clone())),
        };
        match files() {
            None => {
                build(app, &dir);
            }
            Some(_) if first => {
                if active().is_some_and(|p| p.location().equal(&dir)) {
                } else {
                    new_tab(&dir);
                }
            }
            Some(_) => new_tab(&dir),
        }
        first = false;
        if let Some(p) = pick {
            select_later(&p);
        }
    }
    if let Some(f) = files() {
        f.window.present();
        f.hold.borrow_mut().take();
    }
}

const BUS_XML: &str = r#"<node><interface name="org.freedesktop.FileManager1">
<method name="ShowFolders"><arg type="as" name="URIs" direction="in"/><arg type="s" name="StartupId" direction="in"/></method>
<method name="ShowItems"><arg type="as" name="URIs" direction="in"/><arg type="s" name="StartupId" direction="in"/></method>
<method name="ShowItemProperties"><arg type="as" name="URIs" direction="in"/><arg type="s" name="StartupId" direction="in"/></method>
</interface></node>"#;

fn serve(app: &gtk::Application) {
    let app = app.clone();
    let id = gio::bus_own_name(
        gio::BusType::Session,
        "org.freedesktop.FileManager1",
        gio::BusNameOwnerFlags::NONE,
        move |conn, _| {
            let Ok(node) = gio::DBusNodeInfo::for_xml(BUS_XML) else { return };
            let Some(iface) = node.lookup_interface("org.freedesktop.FileManager1") else { return };
            let app = app.clone();
            let _ = conn
                .register_object("/org/freedesktop/FileManager1", &iface)
                .method_call(move |_, _, _, _, method, params, invocation| {
                    let uris: Vec<String> = params.child_value(0).get::<Vec<String>>().unwrap_or_default();
                    let files: Vec<gio::File> = uris.iter().filter(|u| !u.is_empty()).map(|u| gio::File::for_commandline_arg(u)).collect();
                    match method {
                        "ShowFolders" => open_paths(&app, files, false),
                        "ShowItems" => open_paths(&app, files, true),
                        _ => {
                            open_paths(&app, files, true);
                            if let Some(f) = files_rc() {
                                if !f.info_rev.reveals_child() {
                                    toggle_info();
                                }
                            }
                        }
                    }
                    invocation.return_value(None);
                })
                .build();
        },
        |_, _| {},
        |_, _| {},
    );
    std::mem::forget(id);
}

fn files_rc() -> Option<Rc<Files>> {
    files()
}

fn install() {
    kit::app::install(&kit::app::Entry {
        id: APP_ID,
        name: "Digit Files",
        generic: "Files",
        comment: "Browse, find and manage your files",
        exec: "digit-files %U",
        letter: "f",
        colors: ("#5ec8ff", "#1673e6"),
        extra: "Categories=System;FileTools;FileManager;Utility;Core;\nMimeType=inode/directory;\nKeywords=files;folders;finder;explorer;directory;browser;nautilus;\nActions=new-window;\n\n[Desktop Action new-window]\nName=New Window\nExec=digit-files\n",
    });
    kit::app::claim(&format!("{APP_ID}.desktop"), &["inode/directory"]);
    kit::app::replace(&["org.gnome.Nautilus.desktop", "org.kde.dolphin.desktop", "nemo.desktop", "thunar.desktop"], &format!("{APP_ID}.desktop"), "files-replaced");
    let services = glib::user_data_dir().join("dbus-1/services");
    let file = services.join("org.freedesktop.FileManager1.service");
    let exe = std::env::current_exe().ok().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| "digit-files".into());
    let exe = if exe.contains("/target/") { glib::home_dir().join(".local/bin/digit-files").to_string_lossy().into_owned() } else { exe };
    let body = format!("[D-BUS Service]\nName=org.freedesktop.FileManager1\nExec={exe} --service\n");
    if std::fs::read_to_string(&file).ok().as_deref() != Some(body.as_str()) {
        let _ = std::fs::create_dir_all(&services);
        let tmp = file.with_extension("new");
        if std::fs::write(&tmp, &body).is_ok() {
            let _ = std::fs::rename(&tmp, &file);
        }
    }
}

fn main() -> glib::ExitCode {
    if std::env::args().nth(1).as_deref() == Some("--register") {
        install();
        return glib::ExitCode::SUCCESS;
    }
    kit::app::lean("DIGIT_FILES_RENDERER");
    let app = gtk::Application::new(Some(APP_ID), gio::ApplicationFlags::HANDLES_COMMAND_LINE | gio::ApplicationFlags::NON_UNIQUE);
    app.connect_startup(|app| {
        gtk::Window::set_default_icon_name(APP_ID);
        install();
        if let Some(display) = gdk::Display::default() {
            kit::style::live(&display, css);
        }
        accels(app);
        serve(app);
    });
    app.connect_command_line(|app, cmd| {
        let args: Vec<String> = cmd.arguments().iter().skip(1).filter_map(|a| a.to_str().map(str::to_string)).collect();
        if args.iter().any(|a| a == "--service") {
            if files().is_none() {
                let guard = app.hold();
                glib::timeout_add_local_once(std::time::Duration::from_secs(20), move || drop(guard));
            }
            return glib::ExitCode::SUCCESS;
        }
        let select = args.iter().any(|a| a == "--select");
        let targets: Vec<gio::File> = args.iter().filter(|a| !a.starts_with("--")).map(|a| cmd.create_file_for_arg(a)).collect();
        open_paths(app, targets, select);
        glib::ExitCode::SUCCESS
    });
    app.run()
}
