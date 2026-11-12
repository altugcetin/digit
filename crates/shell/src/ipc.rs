use crate::app::{App, Peek};
use gtk::{gio, glib, prelude::*};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::rc::Rc;

pub fn socket() -> PathBuf {
    glib::user_runtime_dir().join("digit.sock")
}

pub fn send(words: &[String]) -> Result<String, String> {
    let mut stream = UnixStream::connect(socket()).map_err(|_| "digit is not running".to_string())?;
    writeln!(stream, "{}", serde_json::to_string(words).unwrap_or_default()).map_err(|e| e.to_string())?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply).map_err(|e| e.to_string())?;
    Ok(reply.trim().to_string())
}

pub fn running() -> bool {
    UnixStream::connect(socket()).is_ok()
}

pub fn serve(app: &Rc<App>) {
    let path = socket();
    let _ = std::fs::remove_file(&path);
    let listener = gio::SocketListener::new();
    let address = gio::UnixSocketAddress::new(&path);
    if let Err(e) = listener.add_address(&address, gio::SocketType::Stream, gio::SocketProtocol::Default, None::<&glib::Object>) {
        eprintln!("digit: {}: {e}", path.display());
        return;
    }
    let weak = Rc::downgrade(app);
    glib::spawn_future_local(async move {
        while let Ok((conn, _)) = listener.accept_future().await {
            let Some(app) = weak.upgrade() else { break };
            glib::spawn_future_local(async move {
                let input = conn.input_stream();
                let read = Box::pin(read_request(&input));
                let timer = Box::pin(glib::timeout_future(std::time::Duration::from_secs(5)));
                let futures_util::future::Either::Left((Some(line), _)) = futures_util::future::select(read, timer).await else { return };
                let words: Vec<String> = serde_json::from_str(&line).unwrap_or_default();
                let reply = handle(&app, &words).await;
                let _ = conn.output_stream().write_all_future(format!("{reply}\n").into_bytes(), glib::Priority::DEFAULT).await;
            });
        }
    });
}

const REQUEST_MAX: usize = 64 * 1024;

fn request_line(bytes: &[u8]) -> Result<Option<String>, ()> {
    if let Some(end) = bytes.iter().position(|b| *b == b'\n') {
        if end > REQUEST_MAX { return Err(()); }
        return std::str::from_utf8(&bytes[..end]).map(|s| Some(s.to_string())).map_err(|_| ());
    }
    if bytes.len() > REQUEST_MAX { Err(()) } else { Ok(None) }
}

async fn read_request(input: &gio::InputStream) -> Option<String> {
    let mut bytes = Vec::new();
    loop {
        let (chunk, count) = input.read_future(vec![0u8; 4096], glib::Priority::DEFAULT).await.ok()?;
        if count == 0 { return None; }
        bytes.extend_from_slice(&chunk[..count]);
        match request_line(&bytes) { Ok(Some(line)) => return Some(line), Ok(None) => {}, Err(()) => return None }
    }
}

async fn focused_output() -> Option<String> {
    gio::spawn_blocking(kit::wm::focused_output).await.ok().flatten()
}

const HELP: &str = "open | close | toggle | tray | phone | note <words> | dnd [on|off|toggle|status] | night [on|off|toggle|status] | awake [on|off|toggle|status] | volume [up|down|mute|<0-150>] | mic | brightness up|down | screenshot | pick | lock [preview] | session | control | spaces | clipboard | settings | launcher [text] | menu <logo|app|window|date|N|traymenu N> | dock show|hide|toggle | trash-fly <pid> <x> <y> <size> [icon...] | layout <name> | wallpaper <image> | demo <peek> | gamebar [toggle|open|close|status] | record [panel|start|stop|toggle|pause|replay [on|off|toggle]|replay-save [seconds]|stream [on|off|toggle]|settings|status] | status | key <action>";

async fn handle(app: &Rc<App>, words: &[String]) -> String {
    let verb = words.first().map(String::as_str).unwrap_or("");
    match verb {
        "open" | "close" | "toggle" | "tray" | "phone" => {
            let output = focused_output().await;
            let notches: Vec<_> = app.notches.borrow().iter().cloned().collect();
            let target = notches.iter().find(|n| Some(n.output()) == output).or(notches.first()).cloned();
            let Some(notch) = target else { return "no notch".into() };
            let open = match verb {
                "open" => true,
                "tray" => {
                    notch.show_tray();
                    return "ok".into();
                }
                "phone" => {
                    notch.show_phone();
                    return "ok".into();
                }
                "close" => false,
                _ => !notch.is_open(),
            };
            for n in notches.iter().filter(|n| n.output() != notch.output()) {
                n.set_open(false);
            }
            notch.set_open(open);
            "ok".into()
        }
        "trash-fly" => {
            let Some((pid, x, y, size)) = trash_fly_args(words) else { return "no".into() };
            let at = gio::spawn_blocking(move || trash_fly_at(&kit::wm::windows(), pid, x, y)).await.ok().flatten();
            let Some(at) = at else { return "no".into() };
            let icon = words.get(5..).filter(|w| !w.is_empty()).and_then(|w| flight_icon(&w.join(" "), size));
            if crate::dock::fly_to_trash(app, at, size, icon) { "ok".into() } else { "no".into() }
        }
        "dock" => {
            let verb = words.get(1).map(String::as_str).unwrap_or("toggle");
            if !matches!(verb, "show" | "hide" | "toggle") {
                return "dock show | hide | toggle".into();
            }
            if crate::dock::command(app, verb) { "ok".into() } else { "dock: could not save".into() }
        }
        "layout" => {
            let name = words.get(1).map(String::as_str).unwrap_or("");
            if crate::layouts::apply_named(app, name) { "ok".into() } else { format!("layouts: {}", crate::layouts::names().join(" | ")) }
        }
        "lock" => match words.get(1).map(String::as_str) {
            Some("preview") => {
                crate::lock::preview(app, words.get(2).map(String::as_str));
                "ok".into()
            }
            _ => {
                if crate::lock::lock(app) { "ok".into() } else { "lock: session lock unavailable".into() }
            }
        },
        "menu" => {
            use crate::strip::Zone;
            let zone = match words.get(1).map(String::as_str).unwrap_or("") {
                "close" => {
                    crate::popmenu::close();
                    crate::datepop::close();
                    return "ok".into();
                }
                "logo" => Zone::Logo,
                "date" | "calendar" => Zone::Date,
                "app" => Zone::App,
                "window" => Zone::Win,
                n if n.starts_with("traymenu") => match n[8..].parse::<u8>() {
                    Ok(i) => Zone::Tray(i),
                    Err(_) => return "menu traymenu<index>".into(),
                },
                n => match n.parse::<u8>() {
                    Ok(i) => Zone::Menu(i),
                    Err(_) => return "menu logo | app | window | date | <index> | traymenu<index> | close".into(),
                },
            };
            let button = if matches!(zone, Zone::Tray(_)) { 3 } else { 1 };
            let output = match words.get(2) {
                Some(o) => Some(o.clone()),
                None => focused_output().await,
            };
            let notches: Vec<_> = app.notches.borrow().iter().cloned().collect();
            let Some(notch) = notches.iter().find(|n| Some(n.output()) == output).or(notches.first()).cloned() else { return "no notch".into() };
            if notch.open_zone(zone, button) { "ok".into() } else { "menu bar is off".into() }
        }
        "demo" => {
            let peek = match words.get(1).map(String::as_str).unwrap_or("") {
                "battery" => Peek::Battery { level: 20, device: None },
                "gadget" => Peek::Battery { level: 10, device: Some(("G305".into(), "mouse")) },
                "device" => Peek::Device { name: "AirPods Pro".into(), icon: "headphones", battery: Some(80), on: true },
                "unplugged" => Peek::Device { name: "AirPods Pro".into(), icon: "headphones", battery: None, on: false },
                "output" => Peek::Output { name: "AirPods Pro".into(), icon: "headphones" },
                "dnd" => Peek::Mode { icon: "do_not_disturb_on", label: "Do not disturb", on: true },
                "keyboard" => Peek::Osd { icon: "keyboard", text: "Turkish".into(), level: None },
                "brightness" => Peek::Osd { icon: "light_mode", text: "Brightness".into(), level: Some(0.6) },
                _ => return "demo battery | gadget | device | unplugged | output | dnd | keyboard | brightness".into(),
            };
            app.request(peek);
            "ok".into()
        }
        "note" => {
            let text = words[1..].join(" ");
            if text.is_empty() {
                return "note needs words".into();
            }
            app.request(Peek::Note { text });
            "ok".into()
        }
        "key" => {
            let action = words.get(1).map(String::as_str).unwrap_or("");
            match action {
                "notch" => Box::pin(handle(app, &["toggle".to_string()])).await,
                "tray" => Box::pin(handle(app, &["tray".to_string()])).await,
                "launcher" => {
                    crate::launcher::toggle(app);
                    "ok".into()
                }
                "control" => {
                    crate::control::toggle(app);
                    "ok".into()
                }
                "clipboard" => {
                    crate::launcher::clipboard(app);
                    "ok".into()
                }
                "wallpaper" if app.cfg.wallpaper => {
                    crate::picker::toggle(app);
                    "ok".into()
                }
                "windows" => {
                    crate::switcher::key(app, 1);
                    "ok".into()
                }
                "windows-back" => {
                    crate::switcher::key(app, -1);
                    "ok".into()
                }
                "settings" => {
                    crate::config::settings("overview", &app.cfg);
                    "ok".into()
                }
                "dock" => {
                    if crate::dock::command(app, "toggle") { "ok".into() } else { "dock: could not save".into() }
                }
                "layouts" => {
                    crate::layouts::toggle(app);
                    "ok".into()
                }
                "workspace-layout" => {
                    crate::layouts::workspace_next(app);
                    "ok".into()
                }
                "snap-max" | "snap-left" | "snap-right" | "snap-restore" => {
                    crate::free::snap(app, &action[5..]);
                    "ok".into()
                }
                "spaces" => {
                    crate::spaces::cycle(app);
                    "ok".into()
                }
                "lock" => {
                    if crate::lock::lock(app) { "ok".into() } else { "unhandled".into() }
                }
                "session" => {
                    crate::session::toggle(app);
                    "ok".into()
                }
                "sleep" => {
                    crate::session::run(app, crate::session::Act::Sleep);
                    "ok".into()
                }
                "night" => {
                    let on = crate::night::active(app);
                    crate::night::force(app, !on);
                    app.request(Peek::Mode { icon: "nightlight", label: "Night Light", on: !on });
                    "ok".into()
                }
                _ if crate::brightness::step(app, action) => "ok".into(),
                _ if crate::audio::press(app, action) => "ok".into(),
                _ => "unhandled".into(),
            }
        }
        "launcher" => {
            crate::launcher::show(app, &words[1..].join(" "));
            "ok".into()
        }
        "wallpaper" => {
            let path = words[1..].join(" ");
            if !crate::wallpaper::remember(&path) {
                return format!("{path} is not a file");
            }
            app.wallpapers.set(path);
            crate::retheme(app);
            "ok".into()
        }
        "dnd" | "night" | "awake" => {
            let current = match verb {
                "dnd" => app.dnd.get(),
                "night" => crate::night::active(app),
                _ => app.awake.borrow().as_ref().is_some_and(|a| a.on()),
            };
            let want = match words.get(1).map(String::as_str).unwrap_or("toggle") {
                "on" => true,
                "off" => false,
                "toggle" => !current,
                "status" => return if current { "on".into() } else { "off".into() },
                _ => return format!("{verb} on | off | toggle | status"),
            };
            match verb {
                "dnd" => {
                    app.set_dnd(want);
                    app.request(Peek::Mode { icon: "do_not_disturb_on", label: "Do not disturb", on: want });
                }
                "night" => {
                    crate::night::force(app, want);
                    app.request(Peek::Mode { icon: "nightlight", label: "Night Light", on: want });
                }
                _ => {
                    let surface = app.notches.borrow().first().and_then(|n| n.surface());
                    if let Some(a) = app.awake.borrow().as_ref() {
                        a.set(surface.as_ref(), want);
                    }
                    app.request(Peek::Mode { icon: "coffee", label: "Keep awake", on: want });
                }
            }
            app.changed();
            if want { "on".into() } else { "off".into() }
        }
        "volume" => match words.get(1).map(String::as_str) {
            Some("up") => Box::pin(handle(app, &["key".to_string(), "volume-up".to_string()])).await,
            Some("down") => Box::pin(handle(app, &["key".to_string(), "volume-down".to_string()])).await,
            Some("mute") => Box::pin(handle(app, &["key".to_string(), "mute".to_string()])).await,
            Some(n) if n.trim_end_matches('%').parse::<f64>().is_ok() => {
                let level = (n.trim_end_matches('%').parse::<f64>().unwrap_or(0.0) / 100.0).clamp(0.0, 1.5);
                let muted = app.volume.get().map(|v| v.1).unwrap_or(false);
                app.volume.set(Some((level, muted)));
                crate::audio::ask(crate::audio::Ask::Level(level));
                app.request(Peek::Volume { level, muted });
                "ok".into()
            }
            None => match app.volume.get() {
                Some((level, muted)) => format!("{}%{}", (level * 100.0).round(), if muted { " muted" } else { "" }),
                None => "unknown".into(),
            },
            _ => "volume up | down | mute | <0-150>".into(),
        },
        "mic" => Box::pin(handle(app, &["key".to_string(), "mic-mute".to_string()])).await,
        "brightness" => match words.get(1).map(String::as_str) {
            Some("up") => if crate::brightness::step(app, "brightness-up") { "ok".into() } else { "no backlight".into() },
            Some("down") => if crate::brightness::step(app, "brightness-down") { "ok".into() } else { "no backlight".into() },
            _ => "brightness up | down".into(),
        },
        "screenshot" => {
            let _ = gio::spawn_blocking(crate::compositor::screenshot).await;
            "ok".into()
        }
        "pick" => match crate::child::spawn(&["digit-snap".as_ref(), "pick".as_ref()], gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE) {
            Ok(_) => "ok".into(),
            Err(_) => "digit-snap is missing".into(),
        },
        "control" | "spaces" | "session" | "clipboard" | "settings" => Box::pin(handle(app, &["key".to_string(), verb.to_string()])).await,
        "renderer" => crate::gpu::describe().to_string(),
        "status" => {
            let (level, muted) = app.volume.get().unwrap_or((0.0, false));
            let call = app.call.borrow().as_ref().map(|c| c.who.clone());
            let lights = app.lights.borrow().clone();
            serde_json::json!({
                "dnd": app.dnd.get(),
                "night_light": crate::night::active(app),
                "keep_awake": app.awake.borrow().as_ref().is_some_and(|a| a.on()),
                "volume": (level * 100.0).round(),
                "muted": muted,
                "mic_muted": app.mic_muted.get(),
                "call": call,
                "microphone": lights.mic,
                "camera": lights.camera,
                "screen_sharing": lights.screen > 0,
                "locked": crate::lock::active(),
                "compositor": kit::wm::name(),
                "recorder": crate::gsr::describe(),
            })
            .to_string()
        }
        "record" => record(app, &words[1..]),
        "gamebar" => match words.get(1).map(String::as_str).unwrap_or("toggle") {
            "toggle" => {
                crate::gamebar::toggle(app);
                "ok".into()
            }
            "open" => {
                crate::gamebar::open(app);
                "ok".into()
            }
            "close" => {
                crate::gamebar::close_now();
                "ok".into()
            }
            "status" => serde_json::json!({ "open": crate::gamebar::is_open() }).to_string(),
            _ => "gamebar [toggle|open|close|status]".into(),
        },
        "help" => HELP.into(),
        _ => HELP.into(),
    }
}

/// The icon a trashed item flies as. Files sends `digit-folder:<glyph>` for folders so they fly as
/// the accent folder it draws, not the icon theme's.
fn flight_icon(name: &str, size: f64) -> Option<gio::Icon> {
    match name.strip_prefix("digit-folder:") {
        Some(glyph) => crate::dock::folder_icon(glyph, size),
        None => gio::Icon::for_string(name).ok(),
    }
}

fn trash_fly_args(words: &[String]) -> Option<(u32, f64, f64, f64)> {
    let pid = words.get(1)?.parse::<u32>().ok().filter(|p| *p > 0)?;
    let number = |i: usize| words.get(i)?.parse::<f64>().ok().filter(|v| v.is_finite());
    let (x, y, size) = (number(2)?, number(3)?, number(4)?);
    (size > 0.0).then_some((pid, x, y, size))
}

fn trash_fly_at(windows: &[kit::wm::Win], pid: u32, x: f64, y: f64) -> Option<(f64, f64)> {
    let window = windows.iter().filter(|w| w.pid == Some(pid)).max_by_key(|w| (w.focused, w.stamp))?;
    let (wx, wy, _, _) = window.rect?;
    // Hyprland clients.at is the window geometry origin, excluding external borders
    // and decorations (HyprCtl.cpp: getWindowData). Files is undecorated, with no
    // client-side shadow inset: its window-local GTK origin matches that geometry.
    // Do not add a border/titlebar offset or multiply these logical units by scale.
    // Backends without a global rect (currently niri) cannot place the flight.
    let at = (wx + x, wy + y);
    (at.0.is_finite() && at.1.is_finite()).then_some(at)
}

const RECORD: &str = "record panel | start | stop | toggle | pause | replay [on|off|toggle] | replay-save [seconds] | stream [on|off|toggle] | settings | status";

fn record(app: &Rc<App>, words: &[String]) -> String {
    use crate::gsr::{self, Mode};
    let want = |w: Option<&String>| match w.map(String::as_str) {
        None | Some("toggle") => Ok(None),
        Some("on") => Ok(Some(true)),
        Some("off") => Ok(Some(false)),
        Some(_) => Err(()),
    };
    match words.first().map(String::as_str).unwrap_or("panel") {
        "panel" => {
            crate::gamebar::toggle(app);
            "ok".into()
        }
        "toggle" => {
            let app = app.clone();
            crate::gamebar::after_close(move || gsr::toggle_record(&app));
            "ok".into()
        }
        "start" => {
            let app = app.clone();
            crate::gamebar::after_close(move || gsr::record(&app, true));
            "ok".into()
        }
        "stop" => {
            if gsr::stop() { "ok".into() } else { "not recording".into() }
        }
        "pause" => {
            if gsr::pause() { "ok".into() } else { "no recording to pause".into() }
        }
        "replay-save" => {
            let secs = match words.get(1).map(|w| w.trim_end_matches('s').parse::<u32>()) {
                None => None,
                Some(Ok(n)) => Some(n),
                Some(Err(_)) => return "record replay-save [seconds]".into(),
            };
            if gsr::save_replay(secs) { "ok".into() } else { "instant replay is off".into() }
        }
        verb @ ("replay" | "stream") => {
            let Ok(want) = want(words.get(1)) else { return format!("record {verb} on | off | toggle") };
            let mode = if verb == "replay" { Mode::Replay } else { Mode::Stream };
            let app = app.clone();
            let starting = want.unwrap_or(gsr::status().is_none_or(|s| s.mode != mode));
            if starting && mode == Mode::Stream {
                crate::gamebar::after_close(move || {
                    let _ = gsr::switch(&app, mode, want);
                });
                return "ok".into();
            }
            match gsr::switch(&app, mode, want) {
                Ok(()) => "ok".into(),
                Err(e) => e,
            }
        }
        "settings" => {
            gsr::settings();
            "ok".into()
        }
        "status" => gsr::describe().to_string(),
        _ => RECORD.into(),
    }
}

#[cfg(test)]
mod request_tests {
    use super::*;
    #[test]
    fn trash_fly_rejects_invalid_numbers_and_selects_the_process_window() {
        let args = |s: &str| s.split_whitespace().map(str::to_string).collect::<Vec<_>>();
        assert_eq!(trash_fly_args(&args("trash-fly 42 -3 4.5 48 icon with spaces")), Some((42, -3.0, 4.5, 48.0)));
        for bad in ["trash-fly", "trash-fly 0 1 2 48", "trash-fly 42 NaN 2 48", "trash-fly 42 1 inf 48", "trash-fly 42 1 2 0", "trash-fly 42 1 2 -48"] {
            assert!(trash_fly_args(&args(bad)).is_none());
        }
        let mut windows = vec![
            kit::wm::Win { pid: Some(42), stamp: 10, rect: Some((-1920.0, 100.0, 800.0, 600.0)), ..Default::default() },
            kit::wm::Win { pid: Some(42), stamp: 5, focused: true, rect: Some((100.0, 200.0, 800.0, 600.0)), ..Default::default() },
            kit::wm::Win { pid: Some(99), stamp: 20, focused: true, ..Default::default() },
        ];
        assert_eq!(trash_fly_at(&windows, 42, 12.5, 24.0), Some((112.5, 224.0)));
        windows[1].focused = false;
        assert_eq!(trash_fly_at(&windows, 42, 12.5, 24.0), Some((-1907.5, 124.0)));
        windows[0].rect = None;
        assert_eq!(trash_fly_at(&windows, 42, 12.5, 24.0), None);
        assert_eq!(trash_fly_at(&windows, 1, 12.5, 24.0), None);
    }

    #[test]
    fn rejects_oversize_and_invalid_requests() {
        assert_eq!(request_line(b"[]\nignored"), Ok(Some("[]".into())));
        assert_eq!(request_line(b"["), Ok(None));
        assert_eq!(request_line(&vec![b'x'; REQUEST_MAX + 1]), Err(()));
        assert_eq!(request_line(b"\xff\n"), Err(()));
    }
}
