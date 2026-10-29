use gtk::{gdk, gio, glib, prelude::*};
use kit::ui;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

const APP_ID: &str = "dev.digit.Rec";
const FLATPAK: &str = "com.dec05eba.gpu_screen_recorder";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Record,
    Replay,
    Stream,
}

#[derive(Clone, PartialEq)]
enum Backend {
    Native,
    Flatpak,
    Missing,
}

struct Rec {
    window: RefCell<Option<gtk::ApplicationWindow>>,
    child: RefCell<Option<std::process::Child>>,
    started: Cell<Option<Instant>>,
    paused: Cell<bool>,
    mode: Cell<Mode>,
    output: RefCell<PathBuf>,
    status: RefCell<Option<gtk::Label>>,
    main_button: RefCell<Option<gtk::Button>>,
    side: RefCell<Option<gtk::Box>>,
    hold: RefCell<Option<gio::ApplicationHoldGuard>>,
}

thread_local! {
    static REC: Rc<Rec> = Rc::new(Rec {
        window: RefCell::new(None),
        child: RefCell::new(None),
        started: Cell::new(None),
        paused: Cell::new(false),
        mode: Cell::new(Mode::Record),
        output: RefCell::new(PathBuf::new()),
        status: RefCell::new(None),
        main_button: RefCell::new(None),
        side: RefCell::new(None),
        hold: RefCell::new(None),
    });
}

fn rec() -> Rc<Rec> {
    REC.with(|r| r.clone())
}

fn backend() -> Backend {
    if glib::find_program_in_path("gpu-screen-recorder").is_some() {
        return Backend::Native;
    }
    let out = std::process::Command::new("flatpak").args(["info", FLATPAK]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
    if out.is_ok_and(|s| s.success()) { Backend::Flatpak } else { Backend::Missing }
}

fn base(b: &Backend) -> Vec<String> {
    match b {
        Backend::Flatpak => vec!["flatpak".into(), "run".into(), "--command=gpu-screen-recorder".into(), FLATPAK.into()],
        _ => vec!["gpu-screen-recorder".into()],
    }
}

fn pref(key: &str, fallback: &str) -> String {
    kit::prefs::text(&kit::prefs::read(), key, fallback)
}

fn set(key: &str, v: &str) {
    let _ = kit::prefs::set(key, toml::Value::String(v.to_string()));
}

fn secret_path() -> PathBuf {
    glib::user_config_dir().join("digit/rec-stream")
}

fn stream_url() -> String {
    std::fs::read_to_string(secret_path()).map(|s| s.trim().to_string()).unwrap_or_default()
}

fn set_stream_url(url: &str) {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let path = secret_path();
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if url.is_empty() {
        let _ = std::fs::remove_file(&path);
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&path) {
        let _ = f.write_all(url.as_bytes());
    }
}

fn folder() -> PathBuf {
    let custom = pref("rec_dir", "");
    if !custom.is_empty() {
        return kit::prefs::home(&custom);
    }
    glib::user_special_dir(glib::UserDirectory::Videos).unwrap_or_else(|| glib::home_dir().join("Videos")).join("Recordings")
}

fn targets(b: &Backend) -> Vec<(String, String)> {
    let mut out = vec![("portal".to_string(), "Choose with the system picker".to_string())];
    if *b == Backend::Missing {
        return out;
    }
    let mut argv = base(b);
    argv.push("--list-capture-options".into());
    if let Ok(o) = std::process::Command::new(&argv[0]).args(&argv[1..]).stderr(std::process::Stdio::null()).output() {
        for line in String::from_utf8_lossy(&o.stdout).lines() {
            let mut parts = line.splitn(2, '|');
            let id = parts.next().unwrap_or("").trim().to_string();
            if id.is_empty() || id == "portal" || id == "window" || id == "focused" {
                continue;
            }
            let label = match parts.next() {
                Some(size) => format!("Screen {id} · {size}"),
                None if id == "screen" => "Every screen".to_string(),
                None => id.clone(),
            };
            out.push((id, label));
        }
    }
    out
}

fn argv(b: &Backend, mode: Mode) -> Result<(Vec<String>, PathBuf), String> {
    let mut a = base(b);
    let target = pref("rec_target", "portal");
    a.extend(["-w".into(), target.clone()]);
    if target == "portal" {
        a.extend(["-restore-portal-session".into(), "yes".into()]);
    }
    a.extend(["-f".into(), pref("rec_fps", "60")]);
    a.extend(["-q".into(), pref("rec_quality", "very_high")]);
    let codec = pref("rec_codec", "auto");
    if codec != "auto" {
        a.extend(["-k".into(), codec]);
    }
    let container = pref("rec_container", "mp4");
    a.extend(["-c".into(), container.clone()]);
    a.extend(["-cursor".into(), if kit::prefs::flag(&kit::prefs::read(), "rec_cursor", true) { "yes".into() } else { "no".into() }]);
    let t = kit::prefs::read();
    let mut audio = Vec::new();
    if kit::prefs::flag(&t, "rec_system_audio", true) {
        audio.push("default_output");
    }
    if kit::prefs::flag(&t, "rec_mic", false) {
        audio.push("default_input");
    }
    if !audio.is_empty() {
        a.extend(["-a".into(), audio.join("|")]);
    }
    let dir = folder();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let out = match mode {
        Mode::Record => {
            let stamp = glib::DateTime::now_local().ok().and_then(|d| d.format("Recording %Y-%m-%d at %H.%M.%S").ok()).map(|s| s.to_string()).unwrap_or_else(|| "Recording".into());
            dir.join(format!("{stamp}.{container}"))
        }
        Mode::Replay => {
            a.extend(["-r".into(), pref("rec_replay", "60")]);
            dir.clone()
        }
        Mode::Stream => {
            let url = stream_url();
            if !(url.starts_with("rtmp://") || url.starts_with("rtmps://") || url.starts_with("srt://")) {
                return Err("Add an rtmp://, rtmps:// or srt:// address first".into());
            }
            a.retain(|x| x != &container);
            if let Some(i) = a.iter().position(|x| x == "-c") {
                a.remove(i);
            }
            a.extend(["-c".into(), "flv".into()]);
            PathBuf::from(url)
        }
    };
    a.extend(["-o".into(), out.to_string_lossy().into_owned()]);
    Ok((a, out))
}

fn notify(summary: &str, body: &str) {
    let (summary, body) = (summary.to_string(), body.to_string());
    glib::spawn_future_local(async move {
        let Ok(conn) = gio::bus_get_future(gio::BusType::Session).await else { return };
        let hints = glib::VariantDict::new(None);
        hints.insert("desktop-entry", APP_ID);
        let actions: Vec<String> = Vec::new();
        let args = ("Recorder", 0u32, APP_ID, summary.as_str(), body.as_str(), actions, hints.end(), 4000i32).to_variant();
        let _ = conn.call_future(Some("org.freedesktop.Notifications"), "/org/freedesktop/Notifications", "org.freedesktop.Notifications", "Notify", Some(&args), None, gio::DBusCallFlags::NONE, 3000).await;
    });
}

fn signal(sig: i32) -> bool {
    let r = rec();
    let pid = r.child.borrow().as_ref().map(|c| c.id() as i32);
    match pid {
        Some(p) => unsafe { libc::kill(p, sig) == 0 },
        None => false,
    }
}

fn running() -> bool {
    let r = rec();
    let mut c = r.child.borrow_mut();
    match c.as_mut().map(|c| c.try_wait()) {
        Some(Ok(None)) => true,
        Some(_) => {
            *c = None;
            false
        }
        None => false,
    }
}

fn start(app: &gtk::Application, mode: Mode) {
    if running() {
        return;
    }
    let b = backend();
    if b == Backend::Missing {
        notify("GPU Screen Recorder isn't installed", "Open the recorder to install it from Flathub");
        present(app);
        return;
    }
    let (a, out) = match argv(&b, mode) {
        Ok(x) => x,
        Err(e) => {
            notify("Can't start recording", &e);
            return;
        }
    };
    let log = glib::user_runtime_dir().join("digit-rec.log");
    let err = std::fs::File::create(&log).map(std::process::Stdio::from).unwrap_or_else(|_| std::process::Stdio::null());
    match std::process::Command::new(&a[0]).args(&a[1..]).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(err).spawn() {
        Ok(child) => {
            let r = rec();
            *r.child.borrow_mut() = Some(child);
            r.started.set(Some(Instant::now()));
            r.paused.set(false);
            r.mode.set(mode);
            *r.output.borrow_mut() = out;
            *r.hold.borrow_mut() = Some(app.hold());
            watch(app);
            refresh();
        }
        Err(e) => notify("Can't start recording", &e.to_string()),
    }
}

fn stop() {
    if !running() {
        return;
    }
    signal(libc::SIGINT);
}

fn watch(app: &gtk::Application) {
    let app = app.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
        if running() {
            refresh();
            return glib::ControlFlow::Continue;
        }
        let r = rec();
        let mode = r.mode.get();
        let out = r.output.borrow().clone();
        let short = r.started.take().is_some_and(|t| t.elapsed().as_secs() < 2);
        r.hold.borrow_mut().take();
        if mode == Mode::Record && out.is_file() {
            notify("Recording saved", &out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        } else if short {
            let log = std::fs::read_to_string(glib::user_runtime_dir().join("digit-rec.log")).unwrap_or_default();
            let last = log.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("The recorder stopped right away").to_string();
            notify("Recording didn't start", &last);
        }
        refresh();
        let _ = &app;
        glib::ControlFlow::Break
    });
}

fn elapsed() -> String {
    let secs = rec().started.get().map(|t| t.elapsed().as_secs()).unwrap_or(0);
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

fn refresh() {
    let r = rec();
    let on = running();
    if let Some(l) = r.status.borrow().as_ref() {
        let text = match (on, r.mode.get(), r.paused.get()) {
            (false, _, _) => "Ready".to_string(),
            (true, Mode::Record, true) => format!("Paused · {}", elapsed()),
            (true, Mode::Record, false) => format!("Recording · {}", elapsed()),
            (true, Mode::Replay, _) => format!("Replay buffer on · last {} s kept", pref("rec_replay", "60")),
            (true, Mode::Stream, _) => format!("Live · {}", elapsed()),
        };
        l.set_text(&text);
        if on { l.add_css_class("live") } else { l.remove_css_class("live") }
    }
    if let Some(b) = r.main_button.borrow().as_ref() {
        let label = match (on, r.mode.get()) {
            (true, _) => "Stop",
            (false, Mode::Record) => "Start Recording",
            (false, Mode::Replay) => "Start Replay Buffer",
            (false, Mode::Stream) => "Go Live",
        };
        b.set_label(label);
        if on {
            b.remove_css_class("accent");
            b.add_css_class("danger");
        } else {
            b.remove_css_class("danger");
            b.add_css_class("accent");
        }
    }
    if let Some(side) = r.side.borrow().as_ref() {
        side.set_visible(on && r.mode.get() != Mode::Stream);
        if let Some(first) = side.first_child().and_downcast::<gtk::Button>() {
            first.set_label(if r.mode.get() == Mode::Replay { "Save Replay" } else if r.paused.get() { "Resume" } else { "Pause" });
        }
    }
}

fn choice(card: &gtk::Box, icon: &str, title: &str, key: &'static str, options: &[(&str, &str)], fallback: &str) {
    let (row, end) = ui::row(Some(icon), title, "");
    let labels: Vec<&str> = options.iter().map(|o| o.1).collect();
    let pick = kit::ui::dropdown(&labels, 0);
    let current = pref(key, fallback);
    pick.set_selected(options.iter().position(|o| o.0 == current).unwrap_or(0) as u32);
    let values: Vec<String> = options.iter().map(|o| o.0.to_string()).collect();
    pick.connect_selected_notify(move |p| {
        if let Some(v) = values.get(p.selected() as usize) {
            set(key, v);
        }
    });
    end.append(&pick);
    card.append(&row);
}

fn toggle(card: &gtk::Box, icon: &str, title: &str, sub: &str, key: &'static str, fallback: bool) {
    let (row, end) = ui::row(Some(icon), title, sub);
    let sw = gtk::Switch::new();
    sw.set_active(kit::prefs::flag(&kit::prefs::read(), key, fallback));
    sw.set_valign(gtk::Align::Center);
    sw.connect_active_notify(move |s| {
        let _ = kit::prefs::set(key, toml::Value::Boolean(s.is_active()));
    });
    end.append(&sw);
    card.append(&row);
}

fn build(app: &gtk::Application) -> gtk::ApplicationWindow {
    let window = gtk::ApplicationWindow::builder().application(app).title("Recorder").default_width(620).default_height(760).build();
    window.set_decorated(false);
    window.add_css_class("digit-settings");
    let b = backend();
    let col = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    top.add_css_class("topbar");
    let handle = gtk::WindowHandle::new();
    let title = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let dot = ui::tile("radio_button_checked");
    dot.add_css_class("red");
    title.append(&dot);
    title.append(&ui::text("Recorder", "name big"));
    handle.set_child(Some(&title));
    handle.set_hexpand(true);
    top.append(&handle);
    top.append(&kit::app::controls(&window, APP_ID));
    col.append(&top);
    let (scroll, body) = ui::page("", "");
    if let Some(first) = body.first_child() {
        body.remove(&first);
    }
    if b == Backend::Missing {
        let card = ui::card();
        let (row, end) = ui::row(Some("download"), "Install GPU Screen Recorder", "The recording engine comes from Flathub and installs for you without a password");
        let get = ui::button("Get", "accent");
        get.connect_clicked(|_| {
            let argv: [&std::ffi::OsStr; 2] = ["digit-store".as_ref(), FLATPAK.as_ref()];
            if gio::Subprocess::newv(&argv, gio::SubprocessFlags::NONE).is_err() {
                let _ = gio::AppInfo::launch_default_for_uri(&format!("appstream://{FLATPAK}"), gio::AppLaunchContext::NONE);
            }
        });
        end.append(&get);
        card.append(&row);
        body.append(&card);
        let note = ui::text("Open the recorder again after it's installed.", "dim");
        note.set_margin_top(8);
        body.append(&note);
    }
    let modes = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    modes.add_css_class("modes");
    let mode_buttons: Rc<RefCell<Vec<gtk::Button>>> = Rc::new(RefCell::new(Vec::new()));
    for (m, label) in [(Mode::Record, "Record"), (Mode::Replay, "Replay"), (Mode::Stream, "Stream")] {
        let btn = ui::button(label, "tab");
        if rec().mode.get() == m {
            btn.add_css_class("on");
        }
        let mb = mode_buttons.clone();
        btn.connect_clicked(move |me| {
            if running() {
                return;
            }
            rec().mode.set(m);
            for x in mb.borrow().iter() {
                x.remove_css_class("on");
            }
            me.add_css_class("on");
            refresh();
        });
        mode_buttons.borrow_mut().push(btn.clone());
        modes.append(&btn);
    }
    body.append(&modes);
    let status = ui::text("Ready", "status-big");
    status.set_halign(gtk::Align::Center);
    status.set_margin_top(18);
    body.append(&status);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    actions.set_halign(gtk::Align::Center);
    actions.set_margin_top(12);
    actions.set_margin_bottom(6);
    let main = ui::button("Start Recording", "accent");
    main.add_css_class("big-action");
    let a2 = app.clone();
    main.connect_clicked(move |_| if running() { stop() } else { start(&a2, rec().mode.get()) });
    let side = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let second = ui::button("Pause", "");
    second.connect_clicked(|_| {
        let r = rec();
        if r.mode.get() == Mode::Replay {
            if signal(libc::SIGUSR1) {
                notify("Replay saved", &format!("In {}", folder().display()));
            }
        } else if signal(libc::SIGUSR2) {
            r.paused.set(!r.paused.get());
            refresh();
        }
    });
    side.append(&second);
    side.set_visible(false);
    actions.append(&main);
    actions.append(&side);
    body.append(&actions);
    let hint = ui::text("Super+Alt+R starts or stops · Super+Alt+S saves a replay", "dim");
    hint.set_halign(gtk::Align::Center);
    body.append(&hint);
    body.append(&ui::heading("Capture"));
    let cap = ui::card();
    let opts = targets(&b);
    let pairs: Vec<(&str, &str)> = opts.iter().map(|(a, l)| (a.as_str(), l.as_str())).collect();
    choice(&cap, "screenshot_monitor", "What to record", "rec_target", &pairs, "portal");
    choice(&cap, "speed", "Frame rate", "rec_fps", &[("30", "30 fps"), ("60", "60 fps"), ("120", "120 fps"), ("144", "144 fps"), ("165", "165 fps"), ("240", "240 fps")], "60");
    choice(&cap, "high_quality", "Quality", "rec_quality", &[("medium", "Medium"), ("high", "High"), ("very_high", "Very high"), ("ultra", "Ultra")], "very_high");
    choice(&cap, "movie", "Codec", "rec_codec", &[("auto", "Automatic (H.264)"), ("hevc", "HEVC"), ("av1", "AV1"), ("vp9", "VP9")], "auto");
    choice(&cap, "video_file", "File type", "rec_container", &[("mp4", "MP4"), ("mkv", "MKV"), ("mov", "MOV")], "mp4");
    toggle(&cap, "arrow_selector_tool", "Show the pointer", "", "rec_cursor", true);
    body.append(&cap);
    body.append(&ui::heading("Sound"));
    let snd = ui::card();
    toggle(&snd, "volume_up", "System sound", "What you hear", "rec_system_audio", true);
    toggle(&snd, "mic", "Microphone", "Mixed into the same track", "rec_mic", false);
    body.append(&snd);
    body.append(&ui::heading("Saving"));
    let save = ui::card();
    let (row, end) = ui::row(Some("folder"), "Save to", &folder().display().to_string());
    let pick = ui::button("Choose…", "");
    let sub = row.first_child().and_then(|t| t.next_sibling()).and_then(|c| c.last_child()).and_downcast::<gtk::Label>();
    pick.connect_clicked(move |b| {
        let d = gtk::FileDialog::new();
        let sub = sub.clone();
        d.select_folder(ui::root(b).as_ref(), gio::Cancellable::NONE, move |r| {
            if let Ok(Some(p)) = r.map(|f| f.path()) {
                set("rec_dir", &p.display().to_string());
                if let Some(l) = &sub {
                    l.set_text(&p.display().to_string());
                }
            }
        });
    });
    end.append(&pick);
    save.append(&row);
    let (row, end) = ui::row(Some("folder_open"), "Open the folder", "");
    let open = ui::button("Open", "");
    open.connect_clicked(|_| {
        let dir = folder();
        let _ = std::fs::create_dir_all(&dir);
        let _ = gio::AppInfo::launch_default_for_uri(&gio::File::for_path(dir).uri(), gio::AppLaunchContext::NONE);
    });
    end.append(&open);
    save.append(&row);
    choice(&save, "history", "Replay length", "rec_replay", &[("15", "15 seconds"), ("30", "30 seconds"), ("60", "1 minute"), ("120", "2 minutes"), ("300", "5 minutes")], "60");
    let (row, end) = ui::row(Some("cell_tower"), "Stream address", "rtmp://… with your stream key; kept in a private file only you can read");
    let url = gtk::PasswordEntry::new();
    url.set_show_peek_icon(true);
    url.set_text(&stream_url());
    url.set_width_chars(22);
    url.add_css_class("search");
    url.connect_changed(|e| set_stream_url(e.text().trim()));
    end.append(&url);
    save.append(&row);
    body.append(&save);
    col.append(&scroll);
    window.set_child(Some(&col));
    let r = rec();
    *r.status.borrow_mut() = Some(status);
    *r.main_button.borrow_mut() = Some(main);
    *r.side.borrow_mut() = Some(side);
    let keys = gtk::EventControllerKey::new();
    let w = window.clone();
    keys.connect_key_pressed(move |_, k, _, m| {
        if (k == gdk::Key::w || k == gdk::Key::q) && m.contains(gdk::ModifierType::CONTROL_MASK) {
            w.close();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(keys);
    window.connect_close_request(|_| {
        let r = rec();
        r.status.borrow_mut().take();
        r.main_button.borrow_mut().take();
        r.side.borrow_mut().take();
        r.window.borrow_mut().take();
        glib::Propagation::Proceed
    });
    refresh();
    window
}

fn present(app: &gtk::Application) {
    let r = rec();
    let existing = r.window.borrow().clone();
    let w = existing.unwrap_or_else(|| {
        let w = build(app);
        *r.window.borrow_mut() = Some(w.clone());
        w
    });
    w.present();
}

fn css() -> String {
    let i = kit::style::ink();
    let (main, meta, accent) = (&i.main, &i.meta, &i.accent);
    format!(
        r#"{base}
.topbar {{ padding: 10px 12px 6px 18px; }}
.name.big {{ font-size: 17px; font-weight: 600; }}
.modes {{ background: @d_card; border-radius: 999px; padding: 4px; margin-top: 6px; }}
.modes button.pill.tab {{ background: transparent; color: {meta}; }}
.modes button.pill.tab.on {{ background: alpha({main}, 0.12); color: {main}; }}
.status-big {{ font-family: "Rubik", sans-serif; font-feature-settings: "tnum"; font-size: 30px; font-weight: 600; color: {main}; }}
.status-big.live {{ color: #ff453a; }}
button.pill.big-action {{ padding: 10px 26px; font-size: 15px; }}
button.pill.accent {{ background: {accent}; }}
"#,
        base = kit::style::css()
    )
}

fn install() {
    kit::app::install(&kit::app::Entry {
        id: APP_ID,
        name: "Digit Recorder",
        generic: "Screen Recorder",
        comment: "Record, replay and stream your screen with GPU Screen Recorder",
        exec: "digit-rec",
        letter: "r",
        colors: ("#ff6b61", "#d1281f"),
        extra: "Categories=AudioVideo;Recorder;Utility;\nKeywords=record;screen;video;capture;replay;stream;obs;\nActions=toggle;replay;\n\n[Desktop Action toggle]\nName=Start or Stop Recording\nExec=digit-rec toggle\n\n[Desktop Action replay]\nName=Save Replay\nExec=digit-rec save-replay\n",
    });
}

fn main() -> glib::ExitCode {
    kit::app::lean("DIGIT_REC_RENDERER");
    let app = gtk::Application::new(Some(APP_ID), gio::ApplicationFlags::HANDLES_COMMAND_LINE);
    app.connect_startup(|_| {
        gtk::Window::set_default_icon_name(APP_ID);
        install();
        if let Some(display) = gdk::Display::default() {
            kit::style::live(&display, css);
        }
    });
    app.connect_command_line(|app, cmd| {
        let args: Vec<String> = cmd.arguments().iter().skip(1).filter_map(|a| a.to_str().map(str::to_string)).collect();
        match args.first().map(String::as_str) {
            Some("toggle") => {
                if running() { stop() } else { start(app, Mode::Record) }
            }
            Some("stop") => stop(),
            Some("replay") => {
                if running() { stop() } else { start(app, Mode::Replay) }
            }
            Some("save-replay") => {
                if rec().mode.get() == Mode::Replay && signal(libc::SIGUSR1) {
                    notify("Replay saved", &format!("In {}", folder().display()));
                } else {
                    notify("No replay buffer running", "Start one in the recorder or with Super+Alt+R in Replay mode");
                }
            }
            Some("pause") => {
                if signal(libc::SIGUSR2) {
                    rec().paused.set(!rec().paused.get());
                    refresh();
                }
            }
            _ => present(app),
        }
        glib::ExitCode::SUCCESS
    });
    app.run()
}
