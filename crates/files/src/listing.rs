//! A folder's contents as a list model that stays up to date.
//!
//! GTK's DirectoryList watches its folder with moves on but drops move events: a file saved the
//! usual way (written under a temporary name, then renamed into place) never appears, and one
//! moved to the trash never leaves, until the folder is reloaded. This list handles every kind
//! of change, so the view follows the folder without F5.

use gtk::{gio, glib, prelude::*};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// How many entries are read before they're shown, so big folders fill in as they load.
const BATCH: i32 = 128;

pub struct Listing {
    store: gio::ListStore,
    /// The names listed, to tell at once whether an entry is already there.
    names: RefCell<std::collections::HashSet<String>>,
    attrs: String,
    file: RefCell<Option<gio::File>>,
    loading: Cell<bool>,
    error: RefCell<Option<glib::Error>>,
    monitor: RefCell<Option<gio::FileMonitor>>,
    cancel: RefCell<Option<gio::Cancellable>>,
    /// Bumped whenever the folder changes, so late answers for an old folder are dropped.
    generation: Cell<u64>,
    loading_changed: RefCell<Vec<Box<dyn Fn()>>>,
}

impl Listing {
    pub fn new(attrs: &str, start: &gio::File) -> Rc<Self> {
        let l = Rc::new(Self {
            store: gio::ListStore::new::<gio::FileInfo>(),
            names: RefCell::new(std::collections::HashSet::new()),
            attrs: attrs.to_string(),
            file: RefCell::new(None),
            loading: Cell::new(false),
            error: RefCell::new(None),
            monitor: RefCell::new(None),
            cancel: RefCell::new(None),
            generation: Cell::new(0),
            loading_changed: RefCell::new(Vec::new()),
        });
        l.set_file(Some(start));
        l
    }

    /// The model to show: one `gio::FileInfo` per entry, each carrying its file under
    /// "standard::file" as GTK's DirectoryList does.
    pub fn model(&self) -> &gio::ListStore {
        &self.store
    }

    pub fn file(&self) -> Option<gio::File> {
        self.file.borrow().clone()
    }

    pub fn is_loading(&self) -> bool {
        self.loading.get()
    }

    pub fn error(&self) -> Option<glib::Error> {
        self.error.borrow().clone()
    }

    pub fn n_items(&self) -> u32 {
        self.store.n_items()
    }

    pub fn connect_loading_notify(&self, f: impl Fn() + 'static) {
        self.loading_changed.borrow_mut().push(Box::new(f));
    }

    fn set_loading(&self, on: bool) {
        if self.loading.replace(on) != on {
            for f in self.loading_changed.borrow().iter() {
                f();
            }
        }
    }

    /// Shows `file`'s contents (or nothing), reading them afresh.
    pub fn set_file(self: &Rc<Self>, file: Option<&gio::File>) {
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        if let Some(c) = self.cancel.borrow_mut().take() {
            c.cancel();
        }
        if let Some(m) = self.monitor.borrow_mut().take() {
            m.cancel();
        }
        self.error.borrow_mut().take();
        self.store.remove_all();
        self.names.borrow_mut().clear();
        *self.file.borrow_mut() = file.cloned();
        let Some(dir) = file.cloned() else {
            self.set_loading(false);
            return;
        };
        let cancel = gio::Cancellable::new();
        *self.cancel.borrow_mut() = Some(cancel.clone());
        // Watch before reading, so nothing that changes meanwhile is missed.
        if let Ok(m) = dir.monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, Some(&cancel)) {
            let weak = Rc::downgrade(self);
            m.connect_changed(move |_, file, other, event| {
                if let Some(l) = weak.upgrade() && l.generation.get() == generation {
                    l.changed(file, other, event);
                }
            });
            *self.monitor.borrow_mut() = Some(m);
        }
        self.set_loading(true);
        let me = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let attrs = match me.upgrade() {
                Some(l) => l.attrs.clone(),
                None => return,
            };
            let result = async {
                let e = dir.enumerate_children_future(&attrs, gio::FileQueryInfoFlags::NONE, glib::Priority::DEFAULT).await?;
                loop {
                    let batch = e.next_files_future(BATCH, glib::Priority::DEFAULT).await?;
                    if batch.is_empty() {
                        return Ok::<(), glib::Error>(());
                    }
                    let Some(l) = me.upgrade() else { return Ok(()) };
                    if l.generation.get() != generation {
                        return Ok(());
                    }
                    let infos: Vec<gio::FileInfo> = {
                        let mut names = l.names.borrow_mut();
                        batch
                            .into_iter()
                            // Something the watch already added while this was reading.
                            .filter(|i| names.insert(i.name().to_string_lossy().into_owned()))
                            .map(|i| {
                                i.set_attribute_object("standard::file", &dir.child(i.name()));
                                i
                            })
                            .collect()
                    };
                    l.store.extend_from_slice(&infos);
                }
            }
            .await;
            let Some(l) = me.upgrade() else { return };
            if l.generation.get() != generation {
                return;
            }
            if let Err(e) = result {
                if !e.matches(gio::IOErrorEnum::Cancelled) {
                    *l.error.borrow_mut() = Some(e);
                }
            }
            l.set_loading(false);
        });
    }

    fn position(&self, name: &str) -> Option<u32> {
        if !self.names.borrow().contains(name) {
            return None;
        }
        (0..self.store.n_items()).find(|&i| self.store.item(i).and_downcast::<gio::FileInfo>().is_some_and(|f| f.name().to_str() == Some(name)))
    }

    fn inside(&self, file: &gio::File) -> bool {
        let dir = self.file.borrow();
        file.parent().is_some_and(|p| dir.as_ref().is_some_and(|d| d.equal(&p)))
    }

    fn changed(self: &Rc<Self>, file: &gio::File, other: Option<&gio::File>, event: gio::FileMonitorEvent) {
        use gio::FileMonitorEvent as E;
        match event {
            E::Created | E::MovedIn => self.add(file),
            E::Deleted | E::MovedOut => self.remove(file),
            E::Renamed => {
                self.remove(file);
                if let Some(to) = other.filter(|o| self.inside(o)) {
                    self.add(to);
                }
            }
            // A finished write or a new mode, owner or time: read it again.
            E::ChangesDoneHint | E::AttributeChanged => self.add(file),
            _ => {}
        }
    }

    fn remove(&self, file: &gio::File) {
        if let Some(name) = file.basename().and_then(|n| n.to_str().map(str::to_string)) && let Some(at) = self.position(&name) {
            self.names.borrow_mut().remove(&name);
            self.store.remove(at);
        }
    }

    /// Reads `file` and puts it in the list, replacing what was there under its name.
    fn add(self: &Rc<Self>, file: &gio::File) {
        if !self.inside(file) {
            return;
        }
        let generation = self.generation.get();
        let me = Rc::downgrade(self);
        let file = file.clone();
        glib::spawn_future_local(async move {
            let Some(attrs) = me.upgrade().map(|l| l.attrs.clone()) else { return };
            let Ok(info) = file.query_info_future(&attrs, gio::FileQueryInfoFlags::NONE, glib::Priority::DEFAULT).await else { return };
            let Some(l) = me.upgrade() else { return };
            if l.generation.get() != generation {
                return;
            }
            info.set_attribute_object("standard::file", &file);
            let name = info.name().to_string_lossy().into_owned();
            match l.position(&name) {
                Some(at) => l.store.splice(at, 1, &[info]),
                None => {
                    l.names.borrow_mut().insert(name);
                    l.store.append(&info);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs this thread's main loop until `done` holds or two seconds pass.
    fn wait(done: impl Fn() -> bool) -> bool {
        let ctx = glib::MainContext::thread_default().expect("a thread-default context");
        let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !done() && std::time::Instant::now() < until {
            ctx.iteration(false);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        done()
    }

    #[test]
    fn files_saved_by_rename_or_moved_away_follow_the_folder() {
        // A context of its own: other tests use the default one on their threads.
        let ctx = glib::MainContext::new();
        ctx.with_thread_default(saved_and_moved).expect("thread-default context");
    }

    fn saved_and_moved() {
        let dir = std::env::temp_dir().join(format!("digit-listing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("kept.txt"), "x").unwrap();
        let away = dir.with_extension("away");
        std::fs::create_dir_all(&away).unwrap();
        let l = Listing::new("standard::name", &gio::File::for_path(&dir));
        assert!(wait(|| !l.is_loading() && l.n_items() == 1));
        // Saved the usual way: written under a temporary name, then renamed into place.
        std::fs::write(dir.join(".saved.tmp"), "x").unwrap();
        std::fs::rename(dir.join(".saved.tmp"), dir.join("saved.txt")).unwrap();
        assert!(wait(|| l.position("saved.txt").is_some() && l.position(".saved.tmp").is_none()), "renamed-in file missing");
        // Moved out of the folder, as the trash does.
        std::fs::rename(dir.join("kept.txt"), away.join("kept.txt")).unwrap();
        assert!(wait(|| l.position("kept.txt").is_none()), "moved-out file still listed");
        let names: Vec<String> = (0..l.n_items()).filter_map(|i| l.model().item(i).and_downcast::<gio::FileInfo>()).map(|f| f.name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["saved.txt"]);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&away);
    }
}
