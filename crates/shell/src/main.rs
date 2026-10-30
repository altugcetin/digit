mod app;
mod appmenu;
mod apps;
mod art;
mod auth;
mod bars;
mod audio;
mod awake;
mod blur;
mod bluetooth;
mod brightness;
mod bus;
mod calendar;
mod child;
mod clip;
mod compositor;
mod config;
mod control;
mod corners;
mod datepop;
mod desk;
mod devices;
mod dock;
mod draw;
mod free;
mod gpu;
mod gsr;
mod hooks;
mod icons;
mod idle;
mod intro;
mod ipc;
mod launcher;
mod layouts;
mod lift;
mod lock;
mod media;
mod menus;
mod night;
mod notch;
mod notify;
mod paint;
mod picker;
mod polkit;
mod menubar;
mod popmenu;
mod power;
mod privacy;
mod providers;
mod quit;
mod record;
mod session;
mod share;
mod sni;
mod spaces;
mod spectrum;
mod stash;
mod strip;
mod switcher;
mod sysmon;
mod theming;
mod tray;
mod wallpaper;
mod weather;

use app::App;
use gtk::{gdk, gio, glib, prelude::*};
use std::rc::Rc;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("msg") => match ipc::send(&args[1..]) {
            Ok(reply) => println!("{reply}"),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        },
        Some("release") => {
            theming::give_back();
            hooks::run("stop", &[]);
        }
        Some("theme") => theme_cli(&args[1..]),
        Some("wallpaper") => wallpaper_cli(&args[1..]),
        Some("key") => key_cli(&args[1..]),
        Some("auth") => auth::helper(),
        None => run(),
        Some(other) => {
            eprintln!("digit: unknown command {other}\nusage: digit [release | key ACTION | theme [--scheme NAME] [IMAGE] | wallpaper IMAGE | msg open|close|toggle|note <words>]");
            std::process::exit(2);
        }
    }
}

fn run() {
    if ipc::running() {
        eprintln!("digit is already running");
        std::process::exit(1);
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())) {
        let path = std::env::var_os("PATH").unwrap_or_default();
        if !std::env::split_paths(&path).any(|p| p == dir) {
            let joined = std::env::join_paths(std::iter::once(dir).chain(std::env::split_paths(&path)));
            if let Ok(joined) = joined {
                unsafe { std::env::set_var("PATH", joined) };
            }
        }
    }
    unsafe {
        libc::mallopt(libc::M_MMAP_THRESHOLD, 256 * 1024);
        libc::mallopt(libc::M_ARENA_MAX, 2);
    }
    gpu::prepare();
    gtk::init().expect("digit: no display");
    let css = gtk::CssProvider::new();
    css.load_from_string("window.digit { background: transparent; } window.digit-wallpaper { background: black; } window.digit:drop(active), window.digit *:drop(active), window.digit *:drop(active):focus { box-shadow: none; outline: none; border-color: transparent; }");
    let display = gdk::Display::default().expect("digit: no display");
    gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    icons::install(&display);

    let cfg = config::load();
    let gtk_owns_blur = (gtk::major_version(), gtk::minor_version(), gtk::micro_version()) >= (4, 23, 3);
    let frost = if cfg.dock == config::Dock::Screen && !gtk_owns_blur { blur::Blur::new(&display) } else { None };
    let app = App::new(cfg, frost);
    *app.awake.borrow_mut() = awake::Awake::new(&display);
    if app.cfg.icon_theme.is_some() {
        icons::theme(app.cfg.icon_theme.as_deref());
    }
    if app.cfg.wallpaper {
        if let Some(path) = wallpaper::current().or_else(wallpaper::found) {
            wallpaper::remember(&path);
            app.wallpapers.set(path);
        }
    }
    theming::refresh(None, &app.cfg.scheme, None, false);
    load_palette(&app);
    watch_palette(&app);
    place(&app, &display);
    intro::play(&app);
    let monitors = display.monitors();
    let weak = Rc::downgrade(&app);
    monitors.connect_items_changed(move |_, _, _, _| {
        if let (Some(app), Some(display)) = (weak.upgrade(), gdk::Display::default()) {
            place(&app, &display);
            let later = std::rc::Rc::downgrade(&app);
            glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
                if let Some(app) = later.upgrade() {
                    night::apply(&app);
                    desk::apply(&app);
                    corners::apply(&app);
                }
            });
        }
    });
    media::Players::start(&app);
    audio::start(&app);
    dock::apply(&app);
    watch_config(&app);
    privacy::start(&app);
    gsr::restore(&app);
    power::start(&app);
    bluetooth::start(&app);
    devices::start(&app);
    idle::start(&app);
    night::start(&app);
    sni::start(&app);
    weather::start(&app);
    desk::apply(&app);
    corners::apply(&app);
    if app.cfg.clipboard {
        clip::start(&app);
    }
    picker::slideshow(&app);
    if !kit::theme::state_dir().join("onboarded").exists() {
        glib::timeout_add_local_once(std::time::Duration::from_millis(2500), || {
            let argv: [&std::ffi::OsStr; 2] = ["digit-settings".as_ref(), "welcome".as_ref()];
            let _ = gtk::gio::Subprocess::newv(&argv, gtk::gio::SubprocessFlags::STDOUT_SILENCE | gtk::gio::SubprocessFlags::STDERR_SILENCE);
        });
    }
    ipc::serve(&app);
    if app.cfg.notifications {
        notify::start(&app);
    }
    if app.cfg.polkit {
        polkit::start(&app);
    }
    theming::take(app.cfg.qt_theme);
    if app.cfg.wallpaper && !app.wallpapers.is_empty() {
        app.wallpapers.when_ready(|| hooks::spawn("start"));
    } else {
        hooks::spawn("start");
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        theming::give_back();
        hooks::run("stop", &[]);
        previous(info);
    }));

    let main_loop = glib::MainLoop::new(None, false);
    for signal in [libc_signal::TERM, libc_signal::INT] {
        let main_loop = main_loop.clone();
        glib_unix::unix_signal_add_local(signal, move || {
            main_loop.quit();
            glib::ControlFlow::Break
        });
    }
    main_loop.run();
    let _ = std::fs::remove_file(ipc::socket());
    if let Some(n) = app.notify.borrow().as_ref() {
        n.release_name();
    }
    polkit::stop();
    theming::give_back();
    hooks::run("stop", &[]);
    for n in app.notches.borrow().iter() {
        n.destroy();
    }
    app.wallpapers.destroy();
}

mod libc_signal {
    pub const INT: i32 = 2;
    pub const TERM: i32 = 15;
}

fn place(app: &Rc<App>, display: &gdk::Display) {
    let monitors: Vec<gdk::Monitor> = display.monitors().iter::<gdk::Monitor>().filter_map(Result::ok).collect();
    if app.cfg.wallpaper {
        app.wallpapers.place(&monitors);
    }
    if !app.notches.borrow().is_empty() {
        dock::apply(app);
    }
    let wanted: Vec<gdk::Monitor> = monitors
        .into_iter()
        .filter(|m| {
            let name = m.connector().map(|c| c.to_string()).unwrap_or_default();
            app.cfg.outputs.is_empty() || app.cfg.outputs.contains(&name)
        })
        .collect();
    let names: Vec<String> = wanted.iter().map(|m| m.connector().map(|c| c.to_string()).unwrap_or_default()).collect();
    let stale: Vec<notch::Notch> = {
        let mut list = app.notches.borrow_mut();
        let (keep, drop): (Vec<_>, Vec<_>) = list.drain(..).partition(|n| names.contains(&n.output()));
        *list = keep;
        drop
    };
    for n in stale {
        n.destroy();
    }
    for monitor in wanted {
        let name = monitor.connector().map(|c| c.to_string()).unwrap_or_default();
        if app.notches.borrow().iter().any(|n| n.output() == name) {
            continue;
        }
        let notch = notch::Notch::new(app, &monitor);
        app.notches.borrow_mut().push(notch);
    }
}

fn watch_config(app: &Rc<App>) {
    let _ = std::fs::create_dir_all(config::dir());
    let dir = gio::File::for_path(config::dir());
    let Ok(monitor) = dir.monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE) else { return };
    let weak = Rc::downgrade(app);
    let pending = Rc::new(std::cell::Cell::new(false));
    monitor.connect_changed(move |_, file, other, _| {
        let ours = |f: &gio::File| f.basename().is_some_and(|b| b.as_os_str() == "config.toml");
        if !ours(file) && !other.is_some_and(ours) {
            return;
        }
        if pending.replace(true) {
            return;
        }
        let weak = weak.clone();
        let pending = pending.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(90), move || {
            pending.set(false);
            if let Some(app) = weak.upgrade() {
                app.apply_prefs(config::Prefs::from(&config::load()));
            }
        });
    });
    app.kept.borrow_mut().push(monitor.upcast());
}

fn key_cli(args: &[String]) {
    let action = args.first().cloned().unwrap_or_default();
    if ipc::running() && ipc::send(&["key".to_string(), action.clone()]).is_ok_and(|r| r == "ok") {
        return;
    }
    if audio::Ask::parse(&action).is_some_and(audio::once) {
        return;
    }
    hooks::run("key", &[&action]);
}

fn wallpaper_cli(args: &[String]) {
    let Some(path) = args.first().map(|p| std::fs::canonicalize(p).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| p.clone())) else {
        eprintln!("usage: digit wallpaper IMAGE");
        std::process::exit(2);
    };
    if ipc::running() {
        match ipc::send(&["wallpaper".to_string(), path]) {
            Ok(reply) => println!("{reply}"),
            Err(e) => eprintln!("{e}"),
        }
        return;
    }
    if !wallpaper::remember(&path) {
        eprintln!("digit: {path} is not a file");
        std::process::exit(1);
    }
    theme_cli(&[]);
}

fn theme_cli(args: &[String]) {
    let cfg = config::load();
    let mut scheme: Option<String> = None;
    let mut image = None;
    let mut words = args.iter();
    while let Some(w) = words.next() {
        match w.as_str() {
            "--scheme" => scheme = words.next().cloned(),
            other => image = Some(other.to_string()),
        }
    }
    if scheme.as_deref().is_some_and(|s| !kit::theme::SCHEMES.contains(&s)) {
        eprintln!("digit: schemes: {}", kit::theme::SCHEMES.join(", "));
        std::process::exit(2);
    }
    match theming::refresh(scheme.as_deref(), &cfg.scheme, image, true) {
        Some(t) => println!("{} {} {}", t.scheme, t.source, t.wallpaper),
        None => {
            eprintln!("digit: no wallpaper to read");
            std::process::exit(1);
        }
    }
}

fn load_palette(app: &Rc<App>) {
    let roles = kit::theme::load().and_then(|t| t.shell_roles()).or_else(kit::palette::gtk_roles);
    if let Some(roles) = roles {
        app.set_roles(roles);
    }
}

pub(crate) fn retheme(app: &Rc<App>) {
    let scheme = app.cfg.scheme.clone();
    app.retheming.set(true);
    let weak = Rc::downgrade(app);
    glib::spawn_future_local(async move {
        let fresh = gio::spawn_blocking(move || theming::refresh(None, &scheme, None, false)).await.ok().flatten();
        let Some(app) = weak.upgrade() else { return };
        if fresh.is_none() {
            app.retheming.set(false);
            return;
        }
        let wait = (app.wallpapers.remaining_ms() as f64 * 0.45) as u64;
        drop(app);
        glib::timeout_future(std::time::Duration::from_millis(wait)).await;
        if let Some(app) = weak.upgrade() {
            load_palette(&app);
            glib::timeout_future(std::time::Duration::from_millis(400)).await;
            app.retheming.set(false);
            App::trim();
        }
    });
}

fn watch(app: &Rc<App>, dir: std::path::PathBuf, name: Option<&'static str>, on: fn(&Rc<App>)) {
    let _ = std::fs::create_dir_all(&dir);
    let Ok(monitor) = gio::File::for_path(&dir).monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE) else { return };
    let weak = Rc::downgrade(app);
    monitor.connect_changed(move |_, file, other, _| {
        let hit = |f: Option<&gio::File>| f.and_then(|f| f.basename()).is_some_and(|b| name.is_none_or(|n| b.to_string_lossy() == n));
        if hit(Some(file)) || hit(other) {
            if let Some(app) = weak.upgrade() {
                on(&app);
            }
        }
    });
    app.kept.borrow_mut().push(monitor.upcast());
}

fn watch_palette(app: &Rc<App>) {
    watch(app, kit::theme::state_dir(), Some("theme.json"), |app| {
        if !app.retheming.get() {
            load_palette(app);
        }
    });
    watch(app, kit::palette::gtk_dir(), None, |app| {
        if kit::theme::load().is_none() {
            load_palette(app);
        }
    });
}
