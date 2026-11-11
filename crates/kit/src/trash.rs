//! GIO trash operations. Queries and mutations must stay off the GTK main thread.
use gtk::{gio, glib, prelude::*};
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

/// Blocking query; call from a worker.
pub fn is_full() -> bool {
    gio::File::for_uri("trash:///").query_info("trash::item-count", gio::FileQueryInfoFlags::NONE, gio::Cancellable::NONE)
        .map(|info| info.attribute_uint32("trash::item-count") > 0).unwrap_or(false)
}

/// Main-thread monitor with coalesced worker queries. Retain the returned monitor.
pub fn watch(f: impl Fn(bool) + 'static) -> Option<gio::FileMonitor> {
    let monitor = gio::File::for_uri("trash:///").monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE).ok()?;
    let f = Rc::new(f);
    let generation = Rc::new(Cell::new(0u64));
    let running = Rc::new(Cell::new(false));
    monitor.connect_changed(move |_, _, _, _| {
        generation.set(generation.get().wrapping_add(1));
        if running.replace(true) { return; }
        let (f, generation, running) = (f.clone(), generation.clone(), running.clone());
        glib::spawn_future_local(async move {
            loop {
                glib::timeout_future(Duration::from_millis(150)).await;
                let version = generation.get();
                let full = gio::spawn_blocking(is_full).await.unwrap_or(false);
                if generation.get() != version { continue; }
                running.set(false);
                f(full);
                break;
            }
        });
    });
    Some(monitor)
}

pub fn put(paths: Vec<PathBuf>) {
    gio::spawn_blocking(move || {
        for path in paths {
            if let Err(e) = gio::File::for_path(path).trash(gio::Cancellable::NONE) { eprintln!("digit: trash: {e}"); }
        }
    });
}

fn delete(file: &gio::File) -> Result<(), glib::Error> {
    let info = file.query_info("standard::type", gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS, gio::Cancellable::NONE)?;
    if info.file_type() == gio::FileType::Directory {
        let children = file.enumerate_children("standard::name", gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS, gio::Cancellable::NONE)?;
        while let Some(info) = children.next_file(gio::Cancellable::NONE)? { delete(&children.child(&info))?; }
    }
    file.delete(gio::Cancellable::NONE)
}

pub fn empty() {
    gio::spawn_blocking(|| {
        let trash = gio::File::for_uri("trash:///");
        let result = (|| -> Result<(), glib::Error> {
            let children = trash.enumerate_children("standard::name", gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS, gio::Cancellable::NONE)?;
            while let Some(info) = children.next_file(gio::Cancellable::NONE)? {
                if let Err(e) = delete(&children.child(&info)) { eprintln!("digit: empty trash: {e}"); }
            }
            Ok(())
        })();
        if let Err(e) = result { eprintln!("digit: empty trash: {e}"); }
    });
}

pub fn open() {
    let argv: [&std::ffi::OsStr; 2] = ["digit-files".as_ref(), "trash:///".as_ref()];
    if let Err(e) = gio::Subprocess::newv(&argv, gio::SubprocessFlags::NONE) { eprintln!("digit: open Trash: {e}"); }
}
