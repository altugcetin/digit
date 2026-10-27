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
    let process = gio::Subprocess::newv(
        &["niri".as_ref(), "msg".as_ref(), "--json".as_ref(), "focused-output".as_ref()],
        gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_SILENCE,
    )
    .ok()?;
    let (out, _) = process.communicate_utf8_future(None).await.ok()?;
    let json: serde_json::Value = serde_json::from_str(out?.as_str()).ok()?;
    json.get("name")?.as_str().map(str::to_string)
}

async fn handle(app: &Rc<App>, words: &[String]) -> String {
    let verb = words.first().map(String::as_str).unwrap_or("");
    match verb {
        "open" | "close" | "toggle" => {
            let output = focused_output().await;
            let notches: Vec<_> = app.notches.borrow().iter().cloned().collect();
            let target = notches.iter().find(|n| Some(n.output()) == output).or(notches.first()).cloned();
            let Some(notch) = target else { return "no notch".into() };
            let open = match verb {
                "open" => true,
                "close" => false,
                _ => !notch.is_open(),
            };
            for n in notches.iter().filter(|n| n.output() != notch.output()) {
                n.set_open(false);
            }
            notch.set_open(open);
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
        _ => "open | close | toggle | note <words>".into(),
    }
}
