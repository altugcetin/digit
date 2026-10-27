mod app;
mod audio;
mod blur;
mod config;
mod ipc;
mod media;
mod motion;
mod notch;
mod notify;
mod palette;
mod shape;
mod spectrum;
mod takeover;

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
        Some("release") => takeover::release(),
        None => run(),
        Some(other) => {
            eprintln!("digit: unknown command {other}\nusage: digit [release | msg open|close|toggle|note <words>]");
            std::process::exit(2);
        }
    }
}

fn run() {
    if ipc::running() {
        eprintln!("digit is already running");
        std::process::exit(1);
    }
    gtk::init().expect("digit: no display");
    let css = gtk::CssProvider::new();
    css.load_from_string("window.digit { background: transparent; }");
    let display = gdk::Display::default().expect("digit: no display");
    gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);

    let cfg = config::load();
    let frost = if cfg.dock == config::Dock::Screen { blur::Blur::new(&display) } else { None };
    let app = App::new(cfg, frost);
    load_palette(&app);
    watch_palette(&app);
    place(&app, &display);
    let monitors = display.monitors();
    let weak = Rc::downgrade(&app);
    monitors.connect_items_changed(move |_, _, _, _| {
        if let (Some(app), Some(display)) = (weak.upgrade(), gdk::Display::default()) {
            place(&app, &display);
        }
    });
    media::Players::start(&app);
    audio::start(&app);
    ipc::serve(&app);
    if app.cfg.takeover {
        takeover::begin();
    }

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
    if app.cfg.takeover {
        takeover::release();
    }
    for n in app.notches.borrow().iter() {
        n.destroy();
    }
}

mod libc_signal {
    pub const INT: i32 = 2;
    pub const TERM: i32 = 15;
}

fn place(app: &Rc<App>, display: &gdk::Display) {
    let monitors: Vec<gdk::Monitor> = display.monitors().iter::<gdk::Monitor>().filter_map(Result::ok).collect();
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

fn load_palette(app: &Rc<App>) {
    if let Some(roles) = palette::gtk_roles() {
        app.set_roles(roles);
    }
}

fn watch_palette(app: &Rc<App>) {
    let dir = gio::File::for_path(palette::gtk_dir());
    let Ok(monitor) = dir.monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE) else { return };
    let weak = Rc::downgrade(app);
    monitor.connect_changed(move |_, _, _, _| {
        if let Some(app) = weak.upgrade() {
            load_palette(&app);
        }
    });
    app.kept.borrow_mut().push(monitor.upcast());
}
