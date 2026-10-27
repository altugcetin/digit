use crate::app::{App, Peek};
use gtk::{gio, glib, prelude::*};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

pub fn start(app: &Rc<App>) {
    let weak = Rc::downgrade(app);
    glib::spawn_future_local(async move {
        let pending = Rc::new(Cell::new(false));
        loop {
            if let Some(app) = weak.upgrade() {
                read(&app, false).await;
            }
            let Ok(process) = gio::Subprocess::newv(
                &["pactl".as_ref(), "subscribe".as_ref()],
                gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_SILENCE,
            ) else {
                return;
            };
            let Some(stdout) = process.stdout_pipe() else { return };
            let lines = gio::DataInputStream::new(&stdout);
            while let Ok(Some(line)) = lines.read_line_utf8_future(glib::Priority::DEFAULT).await {
                if !(line.contains(" sink ") || line.contains(" server ")) || pending.replace(true) {
                    continue;
                }
                let weak = weak.clone();
                let pending = pending.clone();
                glib::timeout_add_local_once(Duration::from_millis(30), move || {
                    pending.set(false);
                    if let Some(app) = weak.upgrade() {
                        glib::spawn_future_local(async move { read(&app, true).await });
                    }
                });
            }
            glib::timeout_future(Duration::from_secs(2)).await;
            if weak.upgrade().is_none() {
                return;
            }
        }
    });
}

async fn read(app: &Rc<App>, announce: bool) {
    let Ok(process) = gio::Subprocess::newv(
        &["wpctl".as_ref(), "get-volume".as_ref(), "@DEFAULT_AUDIO_SINK@".as_ref()],
        gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_SILENCE,
    ) else {
        return;
    };
    let Ok((Some(out), _)) = process.communicate_utf8_future(None).await else { return };
    let Some(level) = out.split_whitespace().nth(1).and_then(|v| v.parse::<f64>().ok()) else { return };
    let muted = out.contains("[MUTED]");
    let next = Some((level, muted));
    if app.volume.replace(next) == next {
        return;
    }
    if announce {
        app.request(Peek::Volume { level, muted });
    } else {
        app.changed();
    }
}
