use gtk::{gio, glib, prelude::*};

pub fn control(icon: &str, kind: &str) -> gtk::Button {
    let b = gtk::Button::new();
    b.set_child(Some(&crate::ui::sym(icon)));
    b.add_css_class("ctl");
    if !kind.is_empty() {
        b.add_css_class(kind);
    }
    b.set_valign(gtk::Align::Center);
    b
}

thread_local! {
    static CONTROLS: std::cell::RefCell<Vec<(glib::WeakRef<gtk::Button>, &'static str)>> = const { std::cell::RefCell::new(Vec::new()) };
}

pub(crate) fn refresh_controls(prefs: &toml::Table) {
    let all = CONTROLS.with(|all| std::mem::take(&mut *all.borrow_mut()));
    let buttons: Vec<_> = all.into_iter().filter_map(|(button, key)| button.upgrade().map(|b| (b, key))).collect();
    let live: Vec<_> = buttons.iter().map(|(button, key)| (button.downgrade(), *key)).collect();
    CONTROLS.with(|all| all.borrow_mut().extend(live));
    for (button, key) in buttons {
        button.set_visible(crate::appearance::button_shown(prefs, key));
    }
}

pub fn controls(window: &impl IsA<gtk::Window>, app_id: &'static str) -> gtk::Box {
    let window = window.as_ref().clone();
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    bar.add_css_class("controls");
    bar.set_valign(gtk::Align::Center);
    let min = control("remove", "");
    let max = control(if window.is_maximized() { "filter_none" } else { "crop_square" }, "");
    let close = control("close", "close");
    min.set_tooltip_text(Some("Minimize"));
    max.set_tooltip_text(Some("Maximize"));
    close.set_tooltip_text(Some("Close"));
    min.connect_clicked(move |_| {
        glib::spawn_future_local(async move {
            let _ = gio::spawn_blocking(move || crate::wm::minimize(app_id)).await;
        });
    });
    let w = window.clone();
    max.connect_clicked(move |_| if w.is_maximized() { w.unmaximize() } else { w.maximize() });
    let m = max.clone();
    window.connect_maximized_notify(move |w| m.set_child(Some(&crate::ui::sym(if w.is_maximized() { "filter_none" } else { "crop_square" }))));
    let w = window.clone();
    close.connect_clicked(move |_| w.close());
    bar.append(&min);
    bar.append(&max);
    bar.append(&close);
    let prefs = crate::prefs::read();
    for (button, key) in [(&min, "button_minimize"), (&max, "button_maximize"), (&close, "button_close")] {
        button.set_visible(crate::appearance::button_shown(&prefs, key));
        let weak = button.downgrade();
        CONTROLS.with(|all| all.borrow_mut().push((weak, key)));
    }
    bar
}

/// Hands memory freed by a big teardown back to the system now; glibc keeps it otherwise.
pub fn trim() {
    // SAFETY: malloc_trim only walks the allocator's own free lists, under its own locks, and
    // may be called from any thread at any time.
    unsafe {
        libc::malloc_trim(0);
    }
}

/// [`trim`], once what is being torn down has settled.
pub fn trim_later(ms: u64) {
    glib::timeout_add_local_once(std::time::Duration::from_millis(ms), trim);
}

/// Keeps the allocator lean: large blocks go straight back to the system and at most two
/// arenas are kept. Call it first thing in `main`.
pub fn tune_malloc() {
    // SAFETY: mallopt takes plain integers, both parameters are ones glibc knows, and it is
    // called at start, before GTK or any other thread allocates.
    unsafe {
        libc::mallopt(libc::M_MMAP_THRESHOLD, 256 * 1024);
        libc::mallopt(libc::M_ARENA_MAX, 2);
    }
}

/// The renderer `env` picks, and [`tune_malloc`]: the start of every Digit app.
pub fn lean(env: &str) {
    crate::effects::select(env);
    tune_malloc();
}

/// The D-Bus interface other apps call to show files in a folder ("Show in folder").
pub const FILE_MANAGER: &str = "org.freedesktop.FileManager1";
pub const FILE_MANAGER_XML: &str = r#"<node><interface name="org.freedesktop.FileManager1">
<method name="ShowFolders"><arg type="as" name="URIs" direction="in"/><arg type="s" name="StartupId" direction="in"/></method>
<method name="ShowItems"><arg type="as" name="URIs" direction="in"/><arg type="s" name="StartupId" direction="in"/></method>
<method name="ShowItemProperties"><arg type="as" name="URIs" direction="in"/><arg type="s" name="StartupId" direction="in"/></method>
</interface></node>"#;
pub const FILES_DESKTOP: &str = "dev.digit.Files.desktop";

/// The app other than Digit Files the user picked to open folders, if they picked one.
pub fn folders_elsewhere() -> Option<gio::AppInfo> {
    gio::AppInfo::default_for_type("inode/directory", false).filter(|a| a.id().is_some_and(|id| id != FILES_DESKTOP))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    /// Digit's apps' icons in `data/`: a letter on a gradient, top color first.
    const ICONS: [(&str, &str, (&str, &str)); 4] = [
        ("dev.digit.Settings", "d", ("#6a8dff", "#2f55e0")),
        ("dev.digit.Store", "s", ("#4aa3ff", "#0a64e0")),
        ("dev.digit.Files", "f", ("#5ec8ff", "#1673e6")),
        ("dev.digit.Snap", "c", ("#ff6a88", "#e8355b")),
    ];

    fn icon(letter: &str, (top, bottom): (&str, &str)) -> String {
        let (_, w, h) = crate::pixel::cells(letter);
        let cell = 12.0;
        let (ox, oy) = ((128.0 - w as f64 * cell) / 2.0, (128.0 - h as f64 * cell) / 2.0);
        let (d, _, _) = crate::pixel::path(letter, cell, ox, oy);
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128" viewBox="0 0 128 128"><defs><linearGradient id="g" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{top}"/><stop offset="1" stop-color="{bottom}"/></linearGradient></defs><rect width="128" height="128" rx="30" fill="url(#g)"/><path d="{d}" fill="#ffffff"/></svg>
"##
        )
    }

    fn data() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")
    }

    /// `DIGIT_WRITE_ICONS=1 cargo test -p kit app_icons` writes them again after a change here.
    #[test]
    fn the_app_icons_in_data_are_drawn_from_their_letters() {
        for (id, letter, colors) in ICONS {
            let path = data().join(format!("icons/hicolor/scalable/apps/{id}.svg"));
            let svg = icon(letter, colors);
            if std::env::var_os("DIGIT_WRITE_ICONS").is_some() {
                std::fs::write(&path, &svg).unwrap();
            }
            assert_eq!(std::fs::read_to_string(&path).unwrap_or_default(), svg, "{id}'s icon; DIGIT_WRITE_ICONS=1 writes it");
        }
    }

    #[test]
    fn every_app_entry_in_data_has_its_icon_and_a_plain_command() {
        for (id, _, _) in ICONS {
            let text = std::fs::read_to_string(data().join(format!("applications/{id}.desktop"))).unwrap();
            assert!(text.lines().any(|l| l == format!("Icon={id}")), "{id}");
            // install.sh puts its own folder before the command; a package leaves it on PATH.
            assert!(text.lines().filter(|l| l.starts_with("Exec=")).all(|l| l.starts_with("Exec=digit-")), "{id}");
        }
    }
}
