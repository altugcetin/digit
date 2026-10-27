use crate::app::App;
use gtk::{gio, glib, prelude::*};
use std::rc::Rc;

const CONFIG: &str = "[general]
bars = 5
framerate = 40
sleep_timer = 1
[input]
method = pipewire
source = auto
[output]
method = raw
raw_target = /dev/stdout
data_format = ascii
ascii_max_range = 1000
bar_delimiter = 59
frame_delimiter = 10
[smoothing]
noise_reduction = 0.77
";

pub struct Cava {
    process: gio::Subprocess,
}

impl Drop for Cava {
    fn drop(&mut self) {
        self.process.force_exit();
    }
}

pub fn want(app: &Rc<App>, on: bool) {
    if !on {
        if app.cava.borrow_mut().take().is_some() {
            app.spectrum.borrow_mut().clear();
        }
        return;
    }
    if app.cava.borrow().is_some() {
        return;
    }
    let path = glib::user_runtime_dir().join("digit-cava.conf");
    if std::fs::write(&path, CONFIG).is_err() {
        return;
    }
    let Ok(process) = gio::Subprocess::newv(
        &["cava".as_ref(), "-p".as_ref(), path.as_os_str()],
        gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_SILENCE,
    ) else {
        return;
    };
    let Some(stdout) = process.stdout_pipe() else { return };
    *app.cava.borrow_mut() = Some(Cava { process });
    let lines = gio::DataInputStream::new(&stdout);
    let weak = Rc::downgrade(app);
    glib::spawn_future_local(async move {
        while let Ok(Some(line)) = lines.read_line_utf8_future(glib::Priority::DEFAULT).await {
            let Some(app) = weak.upgrade() else { break };
            let bars: Vec<f64> = line.split(';').filter_map(|v| v.trim().parse::<f64>().ok()).map(|v| v / 1000.0).collect();
            *app.spectrum.borrow_mut() = bars;
            app.redraw();
        }
    });
}
