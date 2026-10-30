//! The recorder itself lives in the shell (`digitctl record …`): it owns gpu-screen-recorder,
//! draws the overlay panel and the notch light. This binary forwards commands to it and holds
//! the settings that don't fit on the panel.

use gtk::{gdk, gio, glib, prelude::*};
use kit::ui;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

const APP_ID: &str = "dev.digit.Rec";

fn ask(words: &[&str]) -> Option<String> {
    let mut stream = UnixStream::connect(glib::user_runtime_dir().join("digit.sock")).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
    let line: Vec<String> = words.iter().map(|w| format!("\"{}\"", w.replace('\\', "\\\\").replace('"', "\\\""))).collect();
    writeln!(stream, "[{}]", line.join(",")).ok()?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply).ok()?;
    Some(reply.trim().to_string())
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

fn targets() -> Vec<(String, String)> {
    let mut out = vec![("monitor".to_string(), "The focused screen".to_string())];
    if let Some(display) = gdk::Display::default() {
        for m in display.monitors().iter::<gdk::Monitor>().filter_map(Result::ok) {
            if let Some(c) = m.connector() {
                let g = m.geometry();
                out.push((c.to_string(), format!("Screen {c} · {}×{}", g.width() * m.scale_factor(), g.height() * m.scale_factor())));
            }
        }
    }
    out.push(("region".into(), "A region you draw".into()));
    out.push(("portal".into(), "A window, chosen in the system picker".into()));
    out
}

fn choice(card: &gtk::Box, icon: &str, title: &str, key: &'static str, options: &[(&str, &str)], fallback: &str) {
    let (row, end) = ui::row(Some(icon), title, "");
    let labels: Vec<&str> = options.iter().map(|o| o.1).collect();
    let pick = ui::dropdown(&labels, 0);
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

fn shortcut(card: &gtk::Box, icon: &str, title: &str, command: &str) {
    let (row, end) = ui::row(Some(icon), title, "");
    let cmd = ui::text(command, "dim mono");
    cmd.set_selectable(true);
    end.append(&cmd);
    card.append(&row);
}

fn build(app: &gtk::Application) -> gtk::ApplicationWindow {
    let window = gtk::ApplicationWindow::builder().application(app).title("Recorder Settings").default_width(640).default_height(780).build();
    window.set_decorated(false);
    window.add_css_class("digit-settings");
    let col = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    top.add_css_class("topbar");
    let handle = gtk::WindowHandle::new();
    let title = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let dot = ui::tile("radio_button_checked");
    dot.add_css_class("red");
    title.append(&dot);
    title.append(&ui::text("Recorder Settings", "name big"));
    handle.set_child(Some(&title));
    handle.set_hexpand(true);
    top.append(&handle);
    top.append(&kit::app::controls(&window, APP_ID));
    col.append(&top);
    let (scroll, body) = ui::page("", "");
    if let Some(first) = body.first_child() {
        body.remove(&first);
    }
    if glib::find_program_in_path("gpu-screen-recorder").is_none() {
        let card = ui::card();
        let (row, _) = ui::row(Some("download"), "Install GPU Screen Recorder", "Recording needs the gpu-screen-recorder package from your distribution");
        card.append(&row);
        body.append(&card);
    }
    let lead = ui::text("Quick choices are on the recorder panel too. Changes apply the next time a recording starts.", "dim");
    lead.set_wrap(true);
    lead.set_xalign(0.0);
    lead.set_margin_top(6);
    body.append(&lead);

    body.append(&ui::heading("Capture"));
    let cap = ui::card();
    let opts = targets();
    let pairs: Vec<(&str, &str)> = opts.iter().map(|(a, l)| (a.as_str(), l.as_str())).collect();
    choice(&cap, "screenshot_monitor", "What to record", "rec_target", &pairs, "monitor");
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

    body.append(&ui::heading("Instant Replay"));
    let rep = ui::card();
    choice(&rep, "history", "Keep the last", "rec_replay", &[("15", "15 seconds"), ("30", "30 seconds"), ("60", "1 minute"), ("120", "2 minutes"), ("300", "5 minutes"), ("600", "10 minutes")], "60");
    body.append(&rep);

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
    body.append(&save);

    body.append(&ui::heading("Streaming"));
    let live = ui::card();
    let (row, end) = ui::row(Some("cell_tower"), "Stream address", "rtmp://, rtmps:// or srt:// with your stream key; kept in a private file only you can read");
    let url = gtk::PasswordEntry::new();
    url.set_show_peek_icon(true);
    url.set_text(&stream_url());
    url.set_width_chars(22);
    url.add_css_class("search");
    url.connect_changed(|e| set_stream_url(e.text().trim()));
    end.append(&url);
    live.append(&row);
    body.append(&live);

    body.append(&ui::heading("Shortcuts"));
    let keys = ui::card();
    shortcut(&keys, "dashboard", "Open the recorder panel", "digitctl record panel");
    shortcut(&keys, "radio_button_checked", "Start or stop recording", "digitctl record toggle");
    shortcut(&keys, "history", "Instant replay on or off", "digitctl record replay");
    shortcut(&keys, "save", "Save the replay", "digitctl record replay-save");
    shortcut(&keys, "pause", "Pause or resume", "digitctl record pause");
    body.append(&keys);
    let hint = ui::text("Bind these in your compositor's config, like the other digitctl commands.", "dim");
    hint.set_xalign(0.0);
    hint.set_margin_top(6);
    body.append(&hint);

    col.append(&scroll);
    window.set_child(Some(&col));
    let keys = gtk::EventControllerKey::new();
    let w = window.clone();
    keys.connect_key_pressed(move |_, k, _, m| {
        if k == gdk::Key::Escape || ((k == gdk::Key::w || k == gdk::Key::q) && m.contains(gdk::ModifierType::CONTROL_MASK)) {
            w.close();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(keys);
    window
}

fn css() -> String {
    format!(
        r#"{base}
.topbar {{ padding: 10px 12px 6px 18px; }}
.name.big {{ font-size: 17px; font-weight: 600; }}
.mono {{ font-family: monospace; font-size: 12px; }}
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
        extra: "Categories=AudioVideo;Recorder;Utility;\nKeywords=record;screen;video;capture;replay;stream;shadowplay;\nActions=toggle;replay;settings;\n\n[Desktop Action toggle]\nName=Start or Stop Recording\nExec=digit-rec toggle\n\n[Desktop Action replay]\nName=Save Replay\nExec=digit-rec save-replay\n\n[Desktop Action settings]\nName=Recorder Settings\nExec=digit-rec settings\n",
    });
}

/// Old spellings from before the recorder moved into the shell.
fn forward(verb: &str) -> Option<Vec<&str>> {
    Some(match verb {
        "panel" => vec!["record", "panel"],
        "toggle" => vec!["record", "toggle"],
        "start" => vec!["record", "start"],
        "stop" => vec!["record", "stop"],
        "pause" => vec!["record", "pause"],
        "replay" => vec!["record", "replay", "toggle"],
        "save-replay" | "replay-save" => vec!["record", "replay-save"],
        "stream" => vec!["record", "stream", "toggle"],
        _ => return None,
    })
}

fn main() -> glib::ExitCode {
    let first = std::env::args().nth(1);
    match first.as_deref() {
        Some("--register") => {
            install();
            return glib::ExitCode::SUCCESS;
        }
        Some("settings") => {}
        verb => {
            let words = forward(verb.unwrap_or("panel"));
            match words.as_deref().and_then(ask) {
                Some(reply) if reply == "ok" => return glib::ExitCode::SUCCESS,
                Some(reply) => {
                    eprintln!("digit-rec: {reply}");
                    return glib::ExitCode::FAILURE;
                }
                None if verb.is_some() && words.is_some() => {
                    eprintln!("digit-rec: digit is not running");
                    return glib::ExitCode::FAILURE;
                }
                None if words.is_none() => {
                    eprintln!("usage: digit-rec [panel | toggle | start | stop | pause | replay | save-replay | stream | settings]");
                    return glib::ExitCode::FAILURE;
                }
                None => {}
            }
        }
    }
    kit::app::lean("DIGIT_REC_RENDERER");
    let app = gtk::Application::new(Some(APP_ID), gio::ApplicationFlags::default());
    app.connect_startup(|_| {
        gtk::Window::set_default_icon_name(APP_ID);
        install();
        if let Some(display) = gdk::Display::default() {
            kit::style::live(&display, css);
        }
    });
    app.connect_activate(|app| {
        if let Some(w) = app.active_window() {
            w.present();
            return;
        }
        build(app).present();
    });
    app.run_with_args::<&str>(&[])
}
