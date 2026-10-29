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
                let input = gio::DataInputStream::new(&conn.input_stream());
                let Ok(Some(line)) = input.read_line_utf8_future(glib::Priority::DEFAULT).await else { return };
                let words: Vec<String> = serde_json::from_str(&line).unwrap_or_default();
                let reply = handle(&app, &words).await;
                let _ = conn.output_stream().write_all_future(format!("{reply}\n").into_bytes(), glib::Priority::DEFAULT).await;
            });
        }
    });
}

async fn focused_output() -> Option<String> {
    gio::spawn_blocking(kit::wm::focused_output).await.ok().flatten()
}

const HELP: &str = "open | close | toggle | tray | note <words> | dnd [on|off|toggle|status] | night [on|off|toggle|status] | awake [on|off|toggle|status] | volume [up|down|mute|<0-150>] | mic | brightness up|down | lock [preview] | session | control | spaces | clipboard | settings | launcher [text] | menu <logo|app|window|N|traymenu N> | dock show|hide|toggle | layout <name> | wallpaper <image> | demo <peek> | status | key <action>";

async fn handle(app: &Rc<App>, words: &[String]) -> String {
    let verb = words.first().map(String::as_str).unwrap_or("");
    match verb {
        "open" | "close" | "toggle" | "tray" => {
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
                "close" => false,
                _ => !notch.is_open(),
            };
            for n in notches.iter().filter(|n| n.output() != notch.output()) {
                n.set_open(false);
            }
            notch.set_open(open);
            "ok".into()
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
                    return "ok".into();
                }
                "logo" => Zone::Logo,
                "app" => Zone::App,
                "window" => Zone::Win,
                n if n.starts_with("traymenu") => match n[8..].parse::<u8>() {
                    Ok(i) => Zone::Tray(i),
                    Err(_) => return "menu traymenu<index>".into(),
                },
                n => match n.parse::<u8>() {
                    Ok(i) => Zone::Menu(i),
                    Err(_) => return "menu logo | app | window | <index> | traymenu<index> | close".into(),
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
                "spaces" => Peek::Spaces { idx: 2 },
                "keyboard" => Peek::Osd { icon: "keyboard", text: "Turkish".into(), level: None },
                "brightness" => Peek::Osd { icon: "light_mode", text: "Brightness".into(), level: Some(0.6) },
                _ => return "demo battery | gadget | device | unplugged | output | dnd | keyboard | brightness | spaces".into(),
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
                "snap-max" | "snap-left" | "snap-right" | "snap-restore" => {
                    crate::free::snap(app, &action[5..]);
                    "ok".into()
                }
                "spaces" => {
                    crate::spaces::toggle(app, None);
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
        "control" | "spaces" | "session" | "clipboard" | "settings" => Box::pin(handle(app, &["key".to_string(), verb.to_string()])).await,
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
            })
            .to_string()
        }
        "help" => HELP.into(),
        _ => HELP.into(),
    }
}
