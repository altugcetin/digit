//! The desktop's right-click menu and new folders.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MenuAction {
    NewFolder,
    Wallpaper,
    Terminal,
    Sort,
    OpenFolder,
    Separator,
    DesktopSettings,
    DisplaySettings,
    Open,
    Reveal,
    Cut,
    Copy,
    CopyPath,
    Rename,
    Duplicate,
    Trash,
    Undo,
    Paste,
    SelectAll,
}
impl MenuAction {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::NewFolder => "New Folder…",
            Self::Wallpaper => "Change Wallpaper",
            Self::Terminal => "Open Terminal Here",
            Self::Sort => "Sort by Name",
            Self::OpenFolder => "Open Desktop Folder",
            Self::Separator => "",
            Self::DesktopSettings => "Desktop Settings",
            Self::DisplaySettings => "Display Settings",
            Self::Open => "Open",
            Self::Reveal => "Show in Files",
            Self::Cut => "Cut",
            Self::Copy => "Copy",
            Self::CopyPath => "Copy Path",
            Self::Rename => "Rename…",
            Self::Duplicate => "Duplicate",
            Self::Trash => "Move to Trash",
            Self::Undo => "Undo",
            Self::Paste => "Paste",
            Self::SelectAll => "Select All",
        }
    }

    /// The same glyphs Files uses for these.
    pub(super) fn glyph(self) -> &'static str {
        match self {
            Self::NewFolder => "create_new_folder",
            Self::Wallpaper => "wallpaper",
            Self::Terminal => "terminal",
            Self::Sort => "sort",
            Self::OpenFolder => "folder_open",
            Self::Separator => "",
            Self::DesktopSettings => "settings",
            Self::DisplaySettings => "monitor",
            Self::Open => "open_in_new",
            Self::Reveal => "folder_open",
            Self::Cut => "content_cut",
            Self::Copy => "content_copy",
            Self::CopyPath => "link",
            Self::Rename => "edit",
            Self::Duplicate => "library_add",
            Self::Trash => "delete",
            Self::Undo => "undo",
            Self::Paste => "content_paste",
            Self::SelectAll => "select_all",
        }
    }

    pub(super) fn keys(self) -> &'static str {
        match self {
            Self::Cut => "Ctrl+X",
            Self::Copy => "Ctrl+C",
            Self::Paste => "Ctrl+V",
            Self::SelectAll => "Ctrl+A",
            Self::Undo => "Ctrl+Z",
            Self::Rename => "F2",
            Self::Trash => "Delete",
            Self::Open => "Enter",
            _ => "",
        }
    }
}
pub(super) fn menu_entries(on_icon: bool, selected: usize, undo: bool) -> Vec<MenuAction> {
    use MenuAction::*;
    if !on_icon {
        let mut rows = vec![NewFolder, Paste, SelectAll];
        if undo {
            rows.push(Undo);
        }
        rows.extend([Separator, Terminal, Sort, OpenFolder, Separator, Wallpaper, DesktopSettings, DisplaySettings]);
        rows
    } else if selected > 1 {
        vec![Open, Separator, Cut, Copy, Duplicate, Separator, Trash]
    } else {
        vec![Open, Reveal, Separator, Cut, Copy, CopyPath, Separator, Rename, Duplicate, Separator, Trash]
    }
}
pub(super) fn menu(d: &Rc<RefCell<Desktop>>, on_icon: bool, x: f64, y: f64) {
    release_keyboard();
    let (app, monitor, folder, entries, files, clipboard) = {
        let Ok(state) = d.try_borrow() else { return };
        let Some(app) = state.app.upgrade() else {
            return;
        };
        (
            app,
            state.monitor.clone(),
            state.folder.clone(),
            menu_entries(on_icon, state.selected.len(), can_undo()),
            state
                .items
                .iter()
                .filter(|i| state.selected.contains(&i.data.name))
                .map(|i| i.data.clone())
                .collect::<Vec<_>>(),
            state.paint.clone(),
        )
    };
    let rows = entries
        .into_iter()
        .map(|action| {
            if action == MenuAction::Separator {
                return Row::Sep;
            }
            let weak = Rc::downgrade(d);
            let app = Rc::downgrade(&app);
            let folder = folder.clone();
            let files = files.clone();
            let paint = clipboard.clone();
            let row = Row::item(action.label(), move || match action {
                MenuAction::Cut | MenuAction::Copy => clip(&paint, &files, action == MenuAction::Cut),
                MenuAction::Paste => {
                    if let Some(d) = weak.upgrade() {
                        paste(&d, (x, y));
                    }
                }
                MenuAction::Duplicate => {
                    if let Some(d) = weak.upgrade() {
                        let uris = files.iter().map(|f| gio::File::for_path(&f.path).uri().to_string()).collect();
                        import_drop(&d, uris, (x + 24.0, y + 24.0), true, false, |_| {});
                    }
                }
                MenuAction::SelectAll => {
                    if let Some(d) = weak.upgrade() {
                        select_all(&d);
                    }
                }
                MenuAction::Undo => {
                    if let Some(d) = weak.upgrade() {
                        undo_trash(&d);
                    }
                }
                MenuAction::NewFolder => {
                    let folder = folder.clone();
                    let weak = weak.clone();
                    glib::spawn_future_local(async move {
                        match gio::spawn_blocking(move || new_folder(&folder)).await {
                            Ok(Ok(path)) => {
                                if let Some(d) = weak.upgrade() {
                                    {
                                        let mut state = d.borrow_mut();
                                        if state.closed {
                                            return;
                                        }
                                        state.rename_pending = path.file_name().map(name_key);
                                        if let Some(name) = state.rename_pending.clone() {
                                            let pos =
                                                drop_cascade(state.grid, (x, y), state.page, 1)[0];
                                            state.saved.insert(name, pos);
                                        }
                                    }
                                    save(&d);
                                    refresh(&d);
                                }
                            }
                            other => eprintln!("digit: desktop new folder: {other:?}"),
                        }
                    });
                }
                MenuAction::Wallpaper => {
                    if let Some(app) = app.upgrade() {
                        crate::picker::toggle(&app);
                    }
                }
                MenuAction::Terminal => {
                    let command = kit::terminal::command();
                    let argv: Vec<_> = command
                        .split_whitespace()
                        .map(std::ffi::OsStr::new)
                        .collect();
                    let launcher = gio::SubprocessLauncher::new(gio::SubprocessFlags::NONE);
                    launcher.set_cwd(&folder);
                    if !argv.is_empty()
                        && let Err(e) = launcher.spawn(&argv)
                    {
                        eprintln!("digit: desktop terminal: {e}");
                    }
                }
                MenuAction::Sort => {
                    if let Some(d) = weak.upgrade() {
                        {
                            let Ok(mut state) = d.try_borrow_mut() else {
                                return;
                            };
                            state.sort_by_name();
                        }
                        save(&d);
                        animate(&d);
                    }
                }
                MenuAction::OpenFolder => show_files(&folder),
                MenuAction::DesktopSettings | MenuAction::DisplaySettings => {
                    let page = if action == MenuAction::DesktopSettings {
                        "desktop"
                    } else {
                        "displays"
                    };
                    if let Err(e) = gio::Subprocess::newv(
                        &[
                            std::ffi::OsStr::new("digit-settings"),
                            std::ffi::OsStr::new(page),
                        ],
                        gio::SubprocessFlags::NONE,
                    ) {
                        eprintln!("digit: desktop settings: {e}");
                    }
                }
                MenuAction::Open => files.iter().for_each(open),
                MenuAction::Reveal => {
                    if let Some(file) = files.first() {
                        reveal_file(&file.path);
                    }
                }
                MenuAction::Rename => {
                    if let Some(d) = weak.upgrade() {
                        rename(&d);
                    }
                }
                MenuAction::CopyPath => {
                    if let Some(file) = files.first() {
                        paint.clipboard().set_text(&file.path.to_string_lossy());
                    }
                }
                MenuAction::Trash => {
                    if let Some(d) = weak.upgrade() {
                        trash(&d, files.iter().map(|f| f.path.clone()).collect());
                    }
                }
                MenuAction::Separator => {}
            });
            let row = row.glyph(action.glyph()).hint(action.keys());
            if action == MenuAction::Trash {
                row.danger()
            } else {
                row
            }
        })
        .collect();
    // The shared menu uses an Overlay surface with Exclusive keyboard mode and
    // a Top-layer outside-click scrim: no popup grab from the Bottom desktop.
    popmenu::show_desktop(&app, Some(monitor), x, y, rows);
}
pub(super) fn folder_names() -> impl Iterator<Item = String> {
    (1..10000).map(|n| {
        if n == 1 {
            "New Folder".into()
        } else {
            format!("New Folder {n}")
        }
    })
}
pub(super) fn next_folder_name(mut exists: impl FnMut(&str) -> bool) -> Option<String> {
    folder_names().find(|name| !exists(name))
}
pub(super) fn new_folder(folder: &Path) -> std::io::Result<PathBuf> {
    loop {
        let name = next_folder_name(|name| folder.join(name).symlink_metadata().is_ok())
            .ok_or_else(|| std::io::Error::other("No free folder name"))?;
        let path = folder.join(name);
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            // Another creator can win between the name search and mkdir.
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
}
