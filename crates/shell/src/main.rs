mod app;
mod appgrid;
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
mod confignote;
mod control;
mod corners;
mod datepop;
mod batteries;
mod batterywidget;
mod desk;
mod deskcards;
mod deskicons;
mod devices;
mod dock;
mod draw;
mod emoji;
mod feel;
mod filemanager;
mod free;
mod gamebar;
mod glide;
mod gpu;
mod gsr;
mod hooks;
mod icons;
mod images;
mod idle;
mod intro;
mod ipc;
mod launcher;
mod layouts;
mod lift;
mod lock;
mod media;
mod menus;
mod metrics;
mod night;
mod notch;
mod notify;
mod paint;
mod panel_corner;
mod phone;
mod picker;
mod polkit;
mod menubar;
mod popmenu;
mod power;
mod privacy;
mod prism;
mod providers;
mod quit;
mod screens;
mod screentime;
mod session;
mod share;
mod sni;
mod snapzone;
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

kit::layer_shell_first!();

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

/// When digit was started from a terminal that later closes, stdout/stderr point at a dead
/// pty. Apps we launch inherit them, and Rust apps (LACT, …) panic on their first log line,
/// so once the terminal hangs up both go to ~/.cache/digit/shell.log instead.
fn outlive_terminal() {
    use std::io::IsTerminal;
    for (fd, tty) in [(1, std::io::stdout().is_terminal()), (2, std::io::stderr().is_terminal())] {
        if !tty {
            continue;
        }
        glib_unix::unix_fd_add_local(fd, glib::IOCondition::HUP | glib::IOCondition::ERR, move |fd, _| {
            let dir = glib::user_cache_dir().join("digit");
            let _ = std::fs::create_dir_all(&dir);
            let file = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("shell.log"));
            let file = file.or_else(|_| std::fs::OpenOptions::new().write(true).open("/dev/null"));
            if let Ok(file) = file {
                // SAFETY: both are open descriptors; dup2 only makes `fd` (stdout or stderr) point where `file` does.
                unsafe { libc::dup2(std::os::fd::AsRawFd::as_raw_fd(&file), fd) };
            }
            glib::ControlFlow::Break
        });
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
                // SAFETY: the first thing `run` does, before GTK or any thread that could read the environment.
                unsafe { std::env::set_var("PATH", joined) };
            }
        }
    }
    kit::app::tune_malloc();
    gpu::prepare();
    gtk::init().expect("digit: no display");
    let css = gtk::CssProvider::new();
    css.load_from_string("window.digit { background: transparent; } window.digit-wallpaper { background: black; } window.digit:drop(active), window.digit *:drop(active), window.digit *:drop(active):focus { box-shadow: none; outline: none; border-color: transparent; }");
    let display = gdk::Display::default().expect("digit: no display");
    gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    deskicons::style(&display);
    icons::install(&display);
    kit::style::live_glyphs(&display, draw::glyphs_changed);

    let cfg = config::load();
    kit::motion::refresh();
    let gtk_owns_blur = (gtk::major_version(), gtk::minor_version(), gtk::micro_version()) >= (4, 23, 3);
    let frost = if cfg.dock == config::Dock::Screen && !gtk_owns_blur { blur::Blur::new(&display) } else { None };
    let app = App::new(cfg, frost);
    *app.awake.borrow_mut() = awake::Awake::new(&display);
    if app.cfg.icon_theme.is_some() {
        icons::theme(app.cfg.icon_theme.as_deref());
    }
    if app.cfg.wallpaper {
        if let Some(path) = wallpaper::current().or_else(wallpaper::found).or_else(|| app.cfg.wallpapers.values().find(|p| std::path::Path::new(p).is_file()).cloned()) {
            wallpaper::remember(&path);
            app.wallpapers.set(path);
        }
    }
    place(&app);
    let (scheme, image) = (app.cfg.scheme.clone(), app.wallpapers.theme_image());
    glib::spawn_future_local(async move {
        let _ = gio::spawn_blocking(move || theming::refresh(None, &scheme, image, false)).await;
    });
    load_palette(&app);
    watch_palette(&app);
    intro::play(&app);
    let monitors = display.monitors();
    let weak = Rc::downgrade(&app);
    monitors.connect_items_changed(move |_, _, _, _| {
        if let Some(app) = weak.upgrade() {
            place(&app);
            let later = std::rc::Rc::downgrade(&app);
            glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
                if let Some(app) = later.upgrade() {
                    night::apply(&app);
                    desk::apply(&app);
                    deskicons::apply(&app);
                    batterywidget::apply(&app);
                    deskcards::apply(&app);
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
    gamebar::restore(&app);
    power::start(&app);
    bluetooth::start(&app);
    devices::start(&app);
    phone::refresh(&app);
    // An update may bring new default shortcuts; regenerate digit's part of the Hyprland
    // config and have Hyprland read it again if anything changed.
    glib::spawn_future_local(async {
        if let Ok(Err(e)) = gtk::gio::spawn_blocking(kit::setup::sync_all).await {
            eprintln!("digit: {e}");
        }
    });
    idle::start(&app);
    brightness::watch_keyboard();
    glib::spawn_future_local(brightness::restore_keyboard(false));
    screentime::start(&app);
    night::start(&app);
    sni::start(&app);
    weather::start(&app);
    draw::set_raster(app.prefs.borrow().desktop_raster);
    desk::apply(&app);
    deskicons::apply(&app);
    batterywidget::apply(&app);
    deskcards::apply(&app);
    corners::apply(&app);
    if app.cfg.clipboard {
        clip::start(&app);
    }
    picker::slideshow(&app);
    if app.cfg.wallpaper { wallpaper::watch_layout(&app); }
    if !kit::theme::state_dir().join("onboarded").exists() {
        glib::timeout_add_local_once(std::time::Duration::from_millis(2500), || {
            let argv: [&std::ffi::OsStr; 2] = ["digit-settings".as_ref(), "welcome".as_ref()];
            let _ = gtk::gio::Subprocess::newv(&argv, gtk::gio::SubprocessFlags::STDOUT_SILENCE | gtk::gio::SubprocessFlags::STDERR_SILENCE);
        });
    }
    ipc::serve(&app);
    filemanager::start();
    if app.cfg.notifications {
        notify::start(&app);
    }
    if app.cfg.polkit {
        polkit::start(&app);
    }
    theming::changed(false);
    if app.cfg.wallpaper && !app.wallpapers.is_empty() {
        app.wallpapers.when_ready(|| hooks::spawn("start"));
    } else {
        hooks::spawn("start");
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        hooks::run("stop", &[]);
        previous(info);
    }));

    outlive_terminal();
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
    kit::wm::release_pointer_focus();
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

/// Scale or mode changes keep the monitor but change its logical size; the notch, bar and
/// wallpaper are sized when built, so that monitor's surfaces are rebuilt.
fn watch_monitor(app: &Rc<App>, monitor: &gdk::Monitor) {
    if kit::tag::has(monitor, "digit-watched") {
        return;
    }
    kit::tag::set(monitor, "digit-watched", true);
    let pending = Rc::new(std::cell::Cell::new(0u32));
    for prop in ["geometry", "scale-factor", "scale"] {
        let (weak, pending) = (Rc::downgrade(app), pending.clone());
        monitor.connect_notify_local(Some(prop), move |m, _| {
            let generation = pending.get() + 1;
            pending.set(generation);
            let (weak, pending, output) = (weak.clone(), pending.clone(), m.connector().map(|c| c.to_string()).unwrap_or_default());
            glib::timeout_add_local_once(std::time::Duration::from_millis(250), move || {
                if pending.get() == generation {
                    if let Some(app) = weak.upgrade() {
                        reshape(&app, &output);
                    }
                }
            });
        });
    }
}

fn reshape(app: &Rc<App>, output: &str) {
    let stale: Vec<notch::Notch> = {
        let mut list = app.notches.borrow_mut();
        let (drop, keep): (Vec<_>, Vec<_>) = list.drain(..).partition(|n| n.output() == output);
        *list = keep;
        drop
    };
    for n in stale {
        n.destroy();
    }
    place(app);
    dock::apply(app);
    night::apply(app);
    desk::apply(app);
    deskicons::apply(app);
    batterywidget::apply(app);
    deskcards::apply(app);
    corners::apply(app);
}

fn place(app: &Rc<App>) {
    let monitors = kit::screens::all();
    for m in &monitors {
        watch_monitor(app, m);
    }
    if app.cfg.wallpaper {
        let had_monitors = !app.wallpapers.monitors().is_empty();
        let before = app.wallpapers.theme_image();
        app.wallpapers.place(&monitors);
        if had_monitors && before != app.wallpapers.theme_image() { retheme(app); }
        let picker = app.picker.borrow().clone();
        if let Some(picker) = picker { picker.refresh_layout(); }
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
    awake::keep(app);
}

fn theme_preferences() -> Vec<Option<toml::Value>> {
    let prefs = kit::prefs::read();
    ["theme_gtk", "theme_qt", "gtk_theme", "qt_theme", "qt_colors", "icon_theme", "cursor_theme", "cursor_size"].iter().map(|key| prefs.get(*key).cloned()).collect()
}

fn watch_config(app: &Rc<App>) {
    let themes = Rc::new(std::cell::RefCell::new(theme_preferences()));
    let _ = std::fs::create_dir_all(config::dir());
    glib::timeout_add_seconds_local_once(3, confignote::check);
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
        let themes = themes.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(90), move || {
            pending.set(false);
            if let Some(app) = weak.upgrade() {
                confignote::check();
                let next = theme_preferences();
                let before = themes.replace(next.clone());
                if before[..6] != next[..6] { theming::changed(before[5] != next[5]); }
                if before[6..] != next[6..] { theming::cursor_changed(); }
                app.apply_prefs(config::Prefs::from(&config::load()));
                picker::prefs_changed(&app);
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
    // A media key the shell didn't take: playerctl's most recent player, as before.
    if let Some(cmd) = media::Command::from_key(&action) {
        if let Err(e) = std::process::Command::new("playerctl").arg(cmd.playerctl()).status() {
            eprintln!("digit: playerctl: {e}");
        }
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
    let image = app.wallpapers.theme_image();
    app.retheming.set(true);
    let weak = Rc::downgrade(app);
    glib::spawn_future_local(async move {
        let fresh = gio::spawn_blocking(move || theming::refresh(None, &scheme, image, false)).await.ok().flatten();
        let Some(app) = weak.upgrade() else { return };
        if fresh.is_none() {
            app.retheming.set(false);
            return;
        }
        theming::changed(false);
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
    let _ = std::fs::create_dir_all(kit::theme::state_dir());
    watch(app, kit::theme::state_dir(), Some("theme.json"), |app| {
        if !app.retheming.get() {
            load_palette(app);
            theming::restyle();
        }
    });
    watch(app, kit::palette::gtk_dir(), None, |app| {
        if kit::theme::load().is_none() {
            load_palette(app);
        }
    });
}
