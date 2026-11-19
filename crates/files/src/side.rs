mod devices;

use gtk::{gdk, gio, glib, pango, prelude::*};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;

#[derive(Clone)]
enum Target {
    File(gio::File),
    Mount(gio::Mount),
    Volume(gio::Volume),
    Connect,
}

pub struct Side {
    pub root: gtk::ScrolledWindow,
    list: gtk::ListBox,
    targets: RefCell<Vec<Option<Target>>>,
    monitor: gio::VolumeMonitor,
    mount_monitor: gio_unix::MountMonitor,
    device_start: Cell<usize>,
    device_count: Cell<usize>,
    refresh_pending: Cell<bool>,
    mounting: Rc<RefCell<HashSet<gio::Volume>>>,
    marking: Cell<bool>,
}

fn bookmarks_file() -> PathBuf {
    glib::user_config_dir().join("gtk-3.0/bookmarks")
}

pub fn bookmarks() -> Vec<(gio::File, String)> {
    std::fs::read_to_string(bookmarks_file())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let mut parts = l.splitn(2, ' ');
            let uri = parts.next()?.trim();
            if uri.is_empty() {
                return None;
            }
            let f = gio::File::for_uri(uri);
            let label = parts.next().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| f.basename().map(|b| b.to_string_lossy().into_owned()).unwrap_or_else(|| uri.to_string()));
            Some((f, label))
        })
        .collect()
}

pub fn toggle_bookmark(file: &gio::File) {
    let mut list = bookmarks();
    let had = list.iter().any(|(f, _)| f.equal(file));
    if had {
        list.retain(|(f, _)| !f.equal(file));
    } else {
        let label = file.basename().map(|b| b.to_string_lossy().into_owned()).unwrap_or_default();
        list.push((file.clone(), label));
    }
    let body: String = list.iter().map(|(f, l)| format!("{} {l}\n", f.uri())).collect();
    let path = bookmarks_file();
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let tmp = path.with_extension("new");
    if std::fs::write(&tmp, body).is_ok() && std::fs::rename(&tmp, &path).is_ok() {
        crate::toast(if had { "Removed from the sidebar" } else { "Added to the sidebar" });
        crate::rebuild_side();
    }
}

fn special(d: glib::UserDirectory) -> Option<gio::File> {
    glib::user_special_dir(d).filter(|p| *p != glib::home_dir() && p.is_dir()).map(gio::File::for_path)
}

impl Side {
    pub fn new() -> Rc<Side> {
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::Single);
        list.add_css_class("places");
        let root = gtk::ScrolledWindow::new();
        root.set_hscrollbar_policy(gtk::PolicyType::Never);
        root.set_vexpand(true);
        root.set_child(Some(&list));
        let side = Rc::new(Side {
            root, list, targets: RefCell::new(Vec::new()), monitor: gio::VolumeMonitor::get(), mount_monitor: gio_unix::MountMonitor::get(),
            device_start: Cell::new(0), device_count: Cell::new(0), refresh_pending: Cell::new(false), mounting: Rc::new(RefCell::new(HashSet::new())), marking: Cell::new(false),
        });
        let me = Rc::downgrade(&side);
        side.list.connect_row_activated(move |_, row| {
            if let Some(s) = me.upgrade() {
                s.activate(row.index() as usize);
            }
        });
        let me = Rc::downgrade(&side);
        side.list.connect_row_selected(move |_, row| {
            let Some(s) = me.upgrade() else { return };
            if s.marking.get() {
                return;
            }
            if let Some(r) = row {
                s.activate(r.index() as usize);
            }
        });
        for sig in ["mount-added", "mount-removed", "mount-changed", "volume-added", "volume-removed", "volume-changed", "drive-connected", "drive-disconnected", "drive-changed"] {
            let me = Rc::downgrade(&side);
            side.monitor.connect_local(sig, false, move |_| {
                if let Some(s) = me.upgrade() {
                    s.queue_devices();
                }
                None
            });
        }
        for sig in ["mounts-changed", "mountpoints-changed"] {
            let me = Rc::downgrade(&side);
            side.mount_monitor.connect_local(sig, false, move |_| {
                if let Some(s) = me.upgrade() {
                    s.queue_devices();
                }
                None
            });
        }
        side.build();
        side
    }

    pub fn build(&self) {
        self.marking.set(true);
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        let mut targets: Vec<Option<Target>> = Vec::new();
        let home = gio::File::for_path(glib::home_dir());
        let mut places: Vec<(String, String, gio::File)> = vec![("home".into(), "Home".into(), home.clone())];
        for (icon, label, d) in [
            ("desktop_windows", "Desktop", glib::UserDirectory::Desktop),
            ("description", "Documents", glib::UserDirectory::Documents),
            ("download", "Downloads", glib::UserDirectory::Downloads),
            ("music_note", "Music", glib::UserDirectory::Music),
            ("image", "Pictures", glib::UserDirectory::Pictures),
            ("movie", "Videos", glib::UserDirectory::Videos),
        ] {
            if let Some(f) = special(d) {
                places.push((icon.into(), label.into(), f));
            }
        }
        places.push(("schedule".into(), "Recent".into(), gio::File::for_uri("recent:///")));
        places.push(("delete".into(), "Trash".into(), gio::File::for_uri("trash:///")));
        self.header("Places", &mut targets);
        for (icon, label, f) in &places {
            self.row(icon, label, None, &mut targets, Target::File(f.clone()));
        }
        let marks: Vec<(gio::File, String)> = bookmarks().into_iter().filter(|(f, _)| !places.iter().any(|(_, _, p)| p.equal(f))).collect();
        if !marks.is_empty() {
            self.header("Bookmarks", &mut targets);
            for (f, label) in marks {
                let icon = if f.is_native() { "folder" } else { "folder_shared" };
                let at = self.list.observe_children().n_items();
                self.row(icon, &label, None, &mut targets, Target::File(f.clone()));
                if let Some(row) = self.list.row_at_index(at as i32) {
                    removable(&row, &f);
                }
            }
        }
        self.header("Devices", &mut targets);
        let row = self.row_widget("hard_drive", "Computer", None, &Target::File(gio::File::for_path("/")), true);
        self.list.append(&row);
        targets.push(Some(Target::File(gio::File::for_path("/"))));
        self.device_start.set(targets.len());
        for device in devices::rows(&self.monitor) {
            let row = self.row_widget(device.icon, &device.name, device.eject, &device.target, true);
            self.list.append(&row);
            targets.push(Some(device.target));
        }
        self.device_count.set(targets.len() - self.device_start.get());
        self.header("Network", &mut targets);
        self.row("lan", "Connect to Server…", None, &mut targets, Target::Connect);
        *self.targets.borrow_mut() = targets;
        self.marking.set(false);
        if let Some(p) = crate::active() {
            self.mark(&p.location());
        }
    }

    fn queue_devices(self: &Rc<Self>) {
        if self.refresh_pending.replace(true) {
            return;
        }
        let me = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(s) = me.upgrade() {
                s.refresh_pending.set(false);
                s.refresh_devices();
            }
        });
    }

    fn refresh_devices(&self) {
        let devices = devices::rows(&self.monitor);
        let start = self.device_start.get();
        self.marking.set(true);
        for _ in 0..self.device_count.get() {
            if let Some(row) = self.list.row_at_index(start as i32) {
                self.list.remove(&row);
            }
        }
        let mut targets = Vec::with_capacity(devices.len());
        for (i, device) in devices.into_iter().enumerate() {
            let row = self.row_widget(device.icon, &device.name, device.eject, &device.target, true);
            self.list.insert(&row, (start + i) as i32);
            targets.push(Some(device.target));
        }
        let count = targets.len();
        self.targets.borrow_mut().splice(start..start + self.device_count.get(), targets);
        self.device_count.set(count);
        self.marking.set(false);
        if let Some(p) = crate::active() {
            self.mark(&p.location());
        }
    }

    fn header(&self, title: &str, targets: &mut Vec<Option<Target>>) {
        let l = kit::ui::text(title, "side-head");
        let row = gtk::ListBoxRow::new();
        row.add_css_class("head");
        row.set_child(Some(&l));
        row.set_activatable(false);
        row.set_selectable(false);
        row.set_focusable(false);
        self.list.append(&row);
        targets.push(None);
    }

    fn row(&self, icon: &str, label: &str, eject: Option<gio::Mount>, targets: &mut Vec<Option<Target>>, target: Target) {
        let row = self.row_widget(icon, label, eject, &target, false);
        self.list.append(&row);
        targets.push(Some(target));
    }

    /// A row of the sidebar. A drive (`usage`) shows how full it is under its name.
    fn row_widget(&self, icon: &str, label: &str, eject: Option<gio::Mount>, target: &Target, usage: bool) -> gtk::ListBoxRow {
        let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let s = kit::ui::sym(icon);
        s.add_css_class("place-icon");
        b.append(&s);
        let l = kit::ui::text(label, "");
        l.set_hexpand(true);
        l.set_ellipsize(pango::EllipsizeMode::End);
        l.set_xalign(0.0);
        // Mounted drives show how full they are, whichever way they were listed.
        let place = match target {
            Target::File(f) => Some(f.clone()),
            Target::Mount(m) => Some(m.root()),
            Target::Volume(v) => v.get_mount().map(|m| m.root()),
            Target::Connect => None,
        };
        match place.filter(|f| usage && f.is_native()) {
            Some(f) => {
                let col = gtk::Box::new(gtk::Orientation::Vertical, 3);
                col.set_hexpand(true);
                col.append(&l);
                col.append(&fullness(&f));
                b.append(&col);
            }
            _ => b.append(&l),
        }
        if let Some(m) = eject {
            let e = kit::app::control("eject", "");
            e.set_tooltip_text(Some(if m.can_eject() { "Eject" } else { "Unmount" }));
            e.connect_clicked(move |_| {
                let op = gtk::MountOperation::new(crate::window().as_ref());
                let done = |r: Result<(), glib::Error>| {
                    if let Err(e) = r {
                        crate::toast(e.message());
                    }
                };
                if m.can_eject() {
                    m.eject_with_operation(gio::MountUnmountFlags::NONE, Some(&op), None::<&gio::Cancellable>, done);
                } else {
                    m.unmount_with_operation(gio::MountUnmountFlags::NONE, Some(&op), None::<&gio::Cancellable>, done);
                }
            });
            b.append(&e);
        }
        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&b));
        if let Target::File(f) = target {
            let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY | gdk::DragAction::MOVE);
            let dest = f.clone();
            drop.connect_drop(move |t, v, _, _| {
                let Ok(list) = v.get::<gdk::FileList>() else { return false };
                if dest.uri_scheme().is_some_and(|s| s == "trash") {
                    crate::ops::trash(list.files());
                } else {
                    crate::ops::dropped(list.files(), &dest, t.current_event_state().contains(gdk::ModifierType::CONTROL_MASK));
                }
                true
            });
            row.add_controller(drop);
        }
        row
    }

    fn activate(&self, i: usize) {
        let Some(Some(t)) = self.targets.borrow().get(i).cloned() else { return };
        match t {
            Target::File(f) => crate::go(&f),
            Target::Mount(m) => crate::go(&m.default_location()),
            Target::Volume(v) => {
                if let Some(m) = v.get_mount() {
                    crate::go(&m.root());
                    return;
                }
                // Selection and row activation can both fire for one click.
                if !self.mounting.borrow_mut().insert(v.clone()) {
                    return;
                }
                let op = gtk::MountOperation::new(crate::window().as_ref());
                let vv = v.clone();
                let mounting = self.mounting.clone();
                v.mount(gio::MountMountFlags::NONE, Some(&op), None::<&gio::Cancellable>, move |r| {
                    mounting.borrow_mut().remove(&vv);
                    match r {
                        Ok(()) => {
                            if let Some(m) = vv.get_mount() {
                                crate::go(&m.root());
                            }
                        }
                        Err(e) if e.matches(gio::IOErrorEnum::AlreadyMounted) => {
                            if let Some(m) = vv.get_mount() {
                                crate::go(&m.root());
                            } else {
                                crate::toast(e.message());
                            }
                        }
                        Err(e) => crate::toast(e.message()),
                    }
                });
            }
            Target::Connect => connect(),
        }
    }

    pub fn mark(&self, loc: &gio::File) {
        let hit = self.targets.borrow().iter().position(|t| match t {
            Some(Target::File(f)) => f.equal(loc),
            Some(Target::Mount(m)) => m.root().equal(loc),
            _ => false,
        });
        self.marking.set(true);
        match hit.and_then(|i| self.list.row_at_index(i as i32)) {
            Some(r) => self.list.select_row(Some(&r)),
            None => self.list.unselect_all(),
        }
        self.marking.set(false);
    }
}

fn removable(row: &gtk::ListBoxRow, file: &gio::File) {
    let Some(line) = row.child().and_then(|c| c.downcast::<gtk::Box>().ok()) else { return };
    let x = kit::app::control("close", "");
    x.add_css_class("place-remove");
    x.set_tooltip_text(Some("Remove from Sidebar"));
    let f = file.clone();
    x.connect_clicked(move |_| crate::side::toggle_bookmark(&f));
    line.append(&x);
    let click = gtk::GestureClick::new();
    click.set_button(3);
    let (f, r) = (file.clone(), row.clone());
    click.connect_pressed(move |g, _, x, y| {
        g.set_state(gtk::EventSequenceState::Claimed);
        let (open, gone) = (f.clone(), f.clone());
        crate::popup::show(
            &r,
            x,
            y,
            vec![
                crate::popup::run("tab", "Open in New Tab", move || crate::new_tab(&open)),
                crate::popup::Item::Sep,
                crate::popup::run("bookmark_remove", "Remove from Sidebar", move || crate::side::toggle_bookmark(&gone)),
            ],
        );
    });
    row.add_controller(click);
}

fn connect() {
    let hint = kit::ui::text("sftp://user@host/path · smb://server/share · ftp://host · dav://host", "sub");
    hint.set_wrap(true);
    crate::ops::ask_name("Connect to Server", "sftp://", "Connect", false, Some(hint.upcast()), |uri, _| {
        let uri = uri.trim().to_string();
        let scheme = uri.split("://").next().unwrap_or_default().to_lowercase();
        if !["sftp", "ssh", "smb", "ftp", "ftps", "dav", "davs", "afp", "nfs", "webdav", "webdavs"].contains(&scheme.as_str()) || !uri.contains("://") {
            crate::toast("Use an address like sftp://user@host");
            return false;
        }
        let f = gio::File::for_uri(&uri);
        let op = gtk::MountOperation::new(crate::window().as_ref());
        let target = f.clone();
        f.mount_enclosing_volume(gio::MountMountFlags::NONE, Some(&op), None::<&gio::Cancellable>, move |r| match r {
            Ok(()) => crate::go(&target),
            Err(e) if e.matches(gio::IOErrorEnum::AlreadyMounted) => crate::go(&target),
            Err(e) => crate::toast(&format!("Couldn't connect: {}", e.message())),
        });
        true
    });
}

/// How full the drive holding `file` is: a thin bar and the space left, filled in once the
/// filesystem answers, so a slow network drive never holds up the sidebar.
fn fullness(file: &gio::File) -> gtk::Box {
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    line.add_css_class("usage");
    line.set_visible(false);
    let bar = gtk::LevelBar::for_interval(0.0, 1.0);
    bar.set_hexpand(true);
    bar.set_valign(gtk::Align::Center);
    // No low/high marks: it is a measure, not a warning, until the drive is nearly full.
    bar.remove_offset_value(Some(gtk::LEVEL_BAR_OFFSET_LOW));
    bar.remove_offset_value(Some(gtk::LEVEL_BAR_OFFSET_HIGH));
    bar.remove_offset_value(Some(gtk::LEVEL_BAR_OFFSET_FULL));
    bar.add_offset_value("nearly-full", 0.9);
    let free = kit::ui::text("", "usage-free");
    line.append(&bar);
    line.append(&free);
    let (file, weak) = (file.clone(), line.downgrade());
    glib::spawn_future_local(async move {
        let Ok(info) = file.query_filesystem_info_future("filesystem::size,filesystem::free", glib::Priority::LOW).await else { return };
        let (size, left) = (info.attribute_uint64("filesystem::size"), info.attribute_uint64("filesystem::free"));
        let Some(line) = weak.upgrade().filter(|_| size > 0) else { return };
        bar.set_value(1.0 - left as f64 / size as f64);
        free.set_text(&format!("{} free", glib::format_size(left)));
        line.set_tooltip_text(Some(&format!("{} free of {}", glib::format_size(left), glib::format_size(size))));
        line.set_visible(true);
    });
    line
}
