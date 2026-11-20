//! Renaming an icon in place.

use super::*;

/// True means a filesystem change is needed; empty and unchanged names just dismiss.
pub(super) fn rename_name_changed(name: &str, original: &str) -> Result<bool, &'static str> {
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
pub(super) fn rename_selection(name: &str, directory: bool) -> (i32, i32) {
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
pub(super) fn close_rename(d: &Rc<RefCell<Desktop>>, expected: Option<&gtk::Window>) {
    let window = {
        let mut state = d.borrow_mut();
        if expected.is_some_and(|w| state.rename_window.as_ref() != Some(w)) {
            return;
        }
        state.renaming = None;
        (state.rename_window.take(), state.paint.clone())
    };
    let (window, paint) = window;
    if let Some(window) = window {
        window.destroy();
    }
    paint.queue_draw();
}

pub(super) fn rename_error(d: &Rc<RefCell<Desktop>>, entry: &gtk::Entry, error: &gtk::Label, text: &str) {
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

pub(super) fn rename(d: &Rc<RefCell<Desktop>>) {
    release_keyboard();
    // F2 can arrive on another selected output. The editor follows the item.
    let owner = {
        let state = d.borrow();
        if state.selected.len() != 1 {
            return;
        }
        state
            .selected
            .iter()
            .next()
            .filter(|n| !state.positions.contains_key(*n))
            .cloned()
    };
    if let Some(name) = owner {
        if let Some(target) = surfaces()
            .into_iter()
            .find(|d| d.borrow().positions.contains_key(&name))
        {
            rename(&target);
        }
        return;
    }
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
    let old_name = old.clone();
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
    let entry = gtk::Entry::new();
    entry.add_css_class("desk-rename");
    entry.set_has_frame(false);
    gtk::prelude::EntryExt::set_alignment(&entry, 0.5);
    entry.set_width_chars(1);
    entry.set_max_width_chars(1);
    let filename = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    entry.set_text(&filename);
    let selection = rename_selection(&filename, directory);
    let error = gtk::Label::new(None);
    error.add_css_class("desk-rename-error");
    error.set_wrap(true);
    error.set_max_width_chars(22);
    error.set_visible(false);
    let editor = gtk::Box::new(gtk::Orientation::Vertical, 4);
    editor.append(&entry);
    editor.append(&error);
    // Sits exactly on the label and grows from its center with the name, like the label would.
    let y = (r.y + (ICON + 5) as f64)
        .clamp(0.0, (geometry.height() - 24).max(0) as f64)
        .round();
    let screen = geometry.width();
    let place = {
        let (fixed, editor) = (fixed.downgrade(), editor.downgrade());
        move |entry: &gtk::Entry| {
            let (Some(fixed), Some(editor)) = (fixed.upgrade(), editor.upgrade()) else {
                return;
            };
            let text = entry.text();
            let width = (entry.create_pango_layout(Some(&text)).pixel_size().0 + 20)
                .clamp(44, 260)
                .min(screen.max(1));
            entry.set_size_request(width, -1);
            let x = (r.x + CELL_W as f64 / 2.0 - width as f64 / 2.0)
                .clamp(0.0, (screen - width).max(0) as f64)
                .round();
            fixed.move_(&editor, x, y);
        }
    };
    fixed.put(&editor, r.x.round(), y);
    place(&entry);
    entry.connect_changed(place);

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
        // A name already in use gets a number, the way New Folder and drops do: "test (2)".
        let name = collision_name(std::ffi::OsStr::new(&name), directory, |candidate| {
            path.with_file_name(candidate).symlink_metadata().is_ok()
        })
        .to_string_lossy()
        .into_owned();
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
                        state.shared.borrow().loading && !state.closed
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
    let paint = {
        let mut state = d.borrow_mut();
        state.rename_window = Some(window.clone());
        state.renaming = Some(old_name);
        state.paint.clone()
    };
    paint.queue_draw();
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
