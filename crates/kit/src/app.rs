use gtk::{gio, glib, prelude::*};

pub struct Entry<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub generic: &'a str,
    pub comment: &'a str,
    pub exec: &'a str,
    pub letter: &'a str,
    pub colors: (&'a str, &'a str),
    pub extra: &'a str,
}

pub fn install(e: &Entry) -> bool {
    let (_, w, h) = crate::pixel::cells(e.letter);
    let cell = 12.0;
    let (ox, oy) = ((128.0 - w as f64 * cell) / 2.0, (128.0 - h as f64 * cell) / 2.0);
    let (d, _, _) = crate::pixel::path(e.letter, cell, ox, oy);
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128" viewBox="0 0 128 128"><defs><linearGradient id="g" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{}"/><stop offset="1" stop-color="{}"/></linearGradient></defs><rect width="128" height="128" rx="30" fill="url(#g)"/><path d="{d}" fill="#ffffff"/></svg>"##,
        e.colors.0, e.colors.1
    );
    let desktop = format!(
        "[Desktop Entry]\nType=Application\nName={}\nGenericName={}\nComment={}\nExec={}\nIcon={}\nStartupNotify=true\n{}",
        e.name, e.generic, e.comment, e.exec, e.id, e.extra
    );
    let data = glib::user_data_dir();
    let mut changed = false;
    let mut icon = false;
    for (path, body) in [(data.join(format!("icons/hicolor/scalable/apps/{}.svg", e.id)), svg), (data.join(format!("applications/{}.desktop", e.id)), desktop)] {
        if std::fs::read_to_string(&path).ok().as_deref() == Some(body.as_str()) {
            continue;
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if crate::atomic::write(&path, &body).is_ok() {
            changed = true;
            icon |= path.extension().is_some_and(|x| x == "svg");
        }
    }
    if icon {
        let theme = data.join("icons/hicolor");
        let argv: [&std::ffi::OsStr; 5] = ["gtk4-update-icon-cache".as_ref(), "-q".as_ref(), "-t".as_ref(), "-f".as_ref(), theme.as_os_str()];
        let _ = gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE);
    }
    changed
}

pub fn claim(desktop: &str, types: &[&str]) {
    let Some(info) = gio_unix::DesktopAppInfo::new(desktop) else { return };
    for t in types {
        let current = gio::AppInfo::default_for_type(t, false).and_then(|a| a.id()).map(|s| s.to_string());
        if current.as_deref() != Some(desktop) {
            let _ = info.set_as_default_for_type(t);
        }
    }
}

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

pub fn trim_later(ms: u64) {
    glib::timeout_add_local_once(std::time::Duration::from_millis(ms), || {
        unsafe extern "C" {
            fn malloc_trim(pad: usize) -> i32;
        }
        unsafe {
            malloc_trim(0);
        }
    });
}

pub fn lean(env: &str) {
    crate::effects::select(env);
    unsafe extern "C" {
        fn mallopt(param: i32, value: i32) -> i32;
    }
    unsafe {
        mallopt(-3, 256 * 1024);
        mallopt(-8, 2);
    }
}

pub fn replace(ids: &[&str], by: &str, marker: &str) {
    let state = std::env::var_os("XDG_STATE_HOME").map(std::path::PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| glib::home_dir().join(".local/state"));
    let marker = state.join("digit").join(marker);
    let done = std::fs::read_to_string(&marker).unwrap_or_default();
    let mut record = done.clone();
    let apps = glib::user_data_dir().join("applications");
    for id in ids {
        if done.lines().any(|l| l == *id) {
            continue;
        }
        let local = apps.join(id);
        if local.exists() {
            continue;
        }
        let Some(source) = gio_unix::DesktopAppInfo::new(id).and_then(|i| i.filename()) else { continue };
        let Ok(text) = std::fs::read_to_string(&source) else { continue };
        let mut out = String::with_capacity(text.len() + 64);
        let mut group = String::new();
        for line in text.lines() {
            if line.starts_with('[') {
                group = line.to_string();
                out.push_str(line);
                out.push('\n');
                if group == "[Desktop Entry]" {
                    out.push_str(&format!("NoDisplay=true\nX-Digit-Replaced-By={by}\n"));
                }
                continue;
            }
            if group == "[Desktop Entry]" && (line.starts_with("NoDisplay=") || line.starts_with("Hidden=")) {
                continue;
            }
            out.push_str(line);
            out.push('\n');
        }
        let _ = std::fs::create_dir_all(&apps);
        if crate::atomic::write(&local, &out).is_ok() {
            record.push_str(id);
            record.push('\n');
        }
    }
    if record != done {
        let _ = crate::atomic::write(&marker, record);
    }
}
