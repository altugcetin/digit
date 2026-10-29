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
        _ => "open | close | toggle | note <words> | wallpaper <image>".into(),
    }
}
