//! Trash and undo, copy and paste, and selecting everything.

use super::*;

/// Something the trash took from the desktop, and where it sat.
#[derive(Clone)]
pub(super) struct Trashed {
    pub(super) path: PathBuf,
    pub(super) position: Option<FreePos>,
}

pub(super) const UNDO_DEPTH: usize = 20;

thread_local! {
    pub(super) static UNDO: RefCell<Vec<Vec<Trashed>>> = const { RefCell::new(Vec::new()) };
}

pub(super) fn can_undo() -> bool {
    UNDO.with(|u| !u.borrow().is_empty())
}

pub(super) fn trash(d: &Rc<RefCell<Desktop>>, paths: Vec<PathBuf>) {
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
    let desktop = Rc::downgrade(d);
    glib::spawn_future_local(async move {
        let mut gone = Vec::new();
        for entry in entries {
            // The folder icon to fly, made before the folder is gone.
            let folder = flights.iter().find(|(p, ..)| p == &entry.path).filter(|(.., directory)| *directory)
                .and_then(|_| crate::dock::folder_icon(kit::folder::glyph(&gio::File::for_path(&entry.path)), ICON as f64));
            match gio::File::for_path(&entry.path)
                .trash_future(glib::Priority::DEFAULT)
                .await
            {
                Ok(()) => {
                    // Seamless: the icon leaves the desktop in the same frame its flight
                    // starts there, rather than whenever the folder monitor notices.
                    if let Some(d) = desktop.upgrade() {
                        let paint = {
                            let mut state = d.borrow_mut();
                            state.items.retain(|i| i.data.path != entry.path);
                            state.paint.clone()
                        };
                        paint.queue_draw();
                    }
                    if gone.len() < 3
                        && let Some(app) = app.upgrade()
                        && let Some((_, at, icon, directory)) = flights.iter().find(|(p, ..)| p == &entry.path)
                    {
                        // Folders fly as the accent folder the desktop draws for them.
                        let icon = folder.or_else(|| icon.as_deref().and_then(|i| gio::Icon::for_string(i).ok())).unwrap_or_else(|| {
                            let content_type = if *directory { "inode/directory".into() } else { gio::content_type_guess(Some(&entry.path), None).0 };
                            gio::content_type_get_icon(&content_type)
                        });
                        crate::dock::fly_to_trash(&app, *at, ICON as f64, Some(icon));
                    }
                    gone.push(entry);
                }
                Err(e) => eprintln!("digit: desktop trash: {e}"),
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
pub(super) fn restore_from_trash(original: &Path) -> Result<PathBuf, String> {
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
pub(super) fn undo_trash(d: &Rc<RefCell<Desktop>>) {
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

pub(super) const COPIED_FILES: &str = "x-special/gnome-copied-files";

/// The same clipboard formats as Files, so cut and copy work between the two.
pub(super) fn clip(paint: &Paint, files: &[FileData], cut: bool) {
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

pub(super) struct ClipboardWatch {
    pub(super) clipboard: gdk::Clipboard,
    pub(super) changed: Rc<Cell<bool>>,
    pub(super) handler: Option<glib::SignalHandlerId>,
}

impl ClipboardWatch {
    pub(super) fn new(clipboard: &gdk::Clipboard) -> Self {
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
pub(super) fn paste(d: &Rc<RefCell<Desktop>>, pointer: (f64, f64)) {
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

pub(super) fn select_all(d: &Rc<RefCell<Desktop>>) {
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
pub(super) fn screen_middle(d: &Rc<RefCell<Desktop>>) -> (f64, f64) {
    let state = d.borrow();
    (state.grid.width as f64 / 2.0, state.geometry.1 as f64 / 2.0)
}
