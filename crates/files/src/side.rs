use gtk::{gdk, gio, glib, pango, prelude::*};
use std::cell::RefCell;
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
    marking: std::cell::Cell<bool>,
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
        let side = Rc::new(Side { root, list, targets: RefCell::new(Vec::new()), monitor: gio::VolumeMonitor::get(), marking: std::cell::Cell::new(false) });
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
        for sig in ["mount-added", "mount-removed", "mount-changed", "volume-added", "volume-removed", "volume-changed"] {
            let me = Rc::downgrade(&side);
            side.monitor.connect_local(sig, false, move |_| {
                if let Some(s) = me.upgrade() {
                    let s2 = s.clone();
                    glib::idle_add_local_once(move || s2.build());
                }
                None
            });
        }
        side.build();
        side
    }

    pub fn build(&self) {
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
                self.row(icon, &label, None, &mut targets, Target::File(f));
            }
        }
        self.header("Devices", &mut targets);
        self.row("hard_drive", "Computer", None, &mut targets, Target::File(gio::File::for_path("/")));
        let mut seen_roots: Vec<gio::File> = Vec::new();
        for m in self.monitor.mounts() {
            if m.is_shadowed() {
                continue;
            }
            let root = m.root();
            if root.path().is_some_and(|p| p == std::path::Path::new("/") || p == glib::home_dir()) {
                continue;
            }
            seen_roots.push(root.clone());
            let icon = if root.is_native() { if m.can_eject() { "usb" } else { "hard_drive" } } else { "lan" };
            let eject = (m.can_eject() || m.can_unmount()).then(|| m.clone());
            self.row(icon, &m.name(), eject, &mut targets, Target::Mount(m));
        }
        for v in self.monitor.volumes() {
            if v.get_mount().is_some() || !v.can_mount() {
                continue;
            }
            let icon = if v.can_eject() { "usb" } else { "hard_drive" };
            self.row(icon, &v.name(), None, &mut targets, Target::Volume(v));
        }
        self.header("Network", &mut targets);
        self.row("lan", "Connect to Server…", None, &mut targets, Target::Connect);
        *self.targets.borrow_mut() = targets;
        if let Some(p) = crate::active() {
            self.mark(&p.location());
        }
    }

    fn header(&self, title: &str, targets: &mut Vec<Option<Target>>) {
        let l = kit::ui::text(title, "side-head");
        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&l));
        row.set_activatable(false);
        row.set_selectable(false);
        row.set_focusable(false);
        self.list.append(&row);
        targets.push(None);
    }

    fn row(&self, icon: &str, label: &str, eject: Option<gio::Mount>, targets: &mut Vec<Option<Target>>, target: Target) {
        let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let s = kit::ui::sym(icon);
        s.add_css_class("place-icon");
        b.append(&s);
        let l = kit::ui::text(label, "");
        l.set_hexpand(true);
        l.set_ellipsize(pango::EllipsizeMode::End);
        b.append(&l);
        if let Some(m) = eject {
            let e = kit::app::control("eject", "");
            e.set_tooltip_text(Some(if m.can_eject() { "Eject" } else { "Unmount" }));
            e.connect_clicked(move |_| {
                let op = gtk::MountOperation::new(crate::window().as_ref());
                let done = |r: Result<(), glib::Error>| {
                    if let Err(e) = r {
                        crate::toast(&e.message().to_string());
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
        if let Target::File(f) = &target {
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
        self.list.append(&row);
        targets.push(Some(target));
    }

    fn activate(&self, i: usize) {
        let Some(Some(t)) = self.targets.borrow().get(i).cloned() else { return };
        match t {
            Target::File(f) => crate::go(&f),
            Target::Mount(m) => crate::go(&m.default_location()),
            Target::Volume(v) => {
                let op = gtk::MountOperation::new(crate::window().as_ref());
                let vv = v.clone();
                v.mount(gio::MountMountFlags::NONE, Some(&op), None::<&gio::Cancellable>, move |r| match r {
                    Ok(()) => {
                        if let Some(m) = vv.get_mount() {
                            crate::go(&m.default_location());
                        }
                    }
                    Err(e) => crate::toast(&e.message().to_string()),
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
