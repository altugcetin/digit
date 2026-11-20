//! Files dropped on the desktop from elsewhere: where they land, copying or moving them in, and undoing that.

use super::*;

/// A compact cascade, centered on the drop rather than the automatic right column.
pub(super) fn drop_cascade(grid: Grid, pointer: (f64, f64), page: u32, count: usize) -> Vec<FreePos> {
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

pub(super) fn collision_name(
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

pub(super) struct ImportPlan {
    pub(super) uri: String,
    pub(super) dest: PathBuf,
    pub(super) cut: bool,
}

pub(super) fn rollback_import(
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

pub(super) fn plan_imports(
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

pub(super) fn import_drop(d: &Rc<RefCell<Desktop>>, uris: Vec<String>, pointer: (f64, f64), copy: bool, cut: bool, done: impl FnOnce(bool) + 'static) {
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

pub(super) fn imports_succeeded<E>(results: &Result<Vec<Result<(), String>>, E>) -> bool {
    matches!(results, Ok(items) if items.iter().all(Result::is_ok))
}

pub(super) fn import(file: &gio::File, dest: &Path, copy: bool, cut: bool) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    import_with_devices(file, dest, copy, cut, |src, folder| {
        Ok(std::fs::symlink_metadata(src)?.dev() == std::fs::metadata(folder)?.dev())
    })
}

pub(super) fn import_with_devices(file: &gio::File, dest: &Path, copy: bool, cut: bool, same_device: impl FnOnce(&Path, &Path) -> std::io::Result<bool>) -> Result<(), String> {
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

pub(super) fn copy_import(file: &gio::File, dest: &Path, cut: bool) -> Result<(), String> {
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

pub(super) struct CopiedSource {
    pub(super) path: PathBuf,
    pub(super) meta: std::fs::Metadata,
}

pub(super) fn remove_import_source(copied: &[CopiedSource]) -> std::io::Result<()> {
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

pub(super) fn copy_tree(src: &Path, dest: &Path) -> std::io::Result<Vec<CopiedSource>> {
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

pub(super) fn copy_tree_entries(src: &Path, dest: &Path, copied: &mut Vec<CopiedSource>) -> std::io::Result<()> {
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
