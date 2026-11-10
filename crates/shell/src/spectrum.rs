use crate::app::App;
use gtk::{gio, glib, prelude::*};
use std::rc::Rc;

const CONFIG: &str = "[general]
bars = 5
framerate = 30
sleep_timer = 1
[input]
method = pipewire
source = auto
[output]
channels = mono
mono_option = average
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

thread_local! {
    static BARS: std::cell::RefCell<Vec<glib::WeakRef<crate::bars::Bars>>> = const { std::cell::RefCell::new(Vec::new()) };
}

pub fn watch(app: &Rc<App>, bars: &crate::bars::Bars) {
    BARS.with(|b| b.borrow_mut().push(bars.downgrade()));
    for mapped in [true, false] {
        let weak = Rc::downgrade(app);
        let changed = move |_: &crate::bars::Bars| {
            let weak = weak.clone();
            // Mapping can change while the notch View is borrowed.
            glib::idle_add_local_once(move || {
                let Some(app) = weak.upgrade() else { return };
                let on = app.media.borrow().wings(App::now()) || app.call.borrow().is_some();
                want(&app, on);
            });
        };
        if mapped { bars.connect_map(changed); } else { bars.connect_unmap(changed); }
    }
}

fn visible() -> bool {
    BARS.with(|b| {
        let mut b = b.borrow_mut();
        b.retain(|w| w.upgrade().is_some());
        b.iter().filter_map(|w| w.upgrade()).any(|b| b.is_mapped())
    })
}

pub fn want(app: &Rc<App>, on: bool) {
    if !on || kit::motion::reduced() || kit::effects::software() || !visible() {
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
    let Ok(process) = crate::child::spawn(
        &["cava".as_ref(), "-p".as_ref(), path.as_os_str()],
        gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_SILENCE,
    ) else {
        return;
    };
    let Some(stdout) = process.stdout_pipe() else { return };
    *app.cava.borrow_mut() = Some(Cava { process: process.clone() });
    let lines = gio::DataInputStream::new(&stdout);
    let weak = Rc::downgrade(app);
    glib::spawn_future_local(async move {
        while let Ok(Some(line)) = lines.read_line_utf8_future(glib::Priority::DEFAULT).await {
            let Some(app) = weak.upgrade() else { break };
            if app.cava.borrow().as_ref().is_none_or(|c| c.process != process) {
                break;
            }
            if kit::effects::software() || kit::motion::reduced() || !visible() {
                want(&app, false);
                app.redraw_wings();
                break;
            }
            let bars: Vec<f64> = line.split(';').filter_map(|v| v.trim().parse::<f64>().ok()).map(|v| v / 1000.0).collect();
            *app.spectrum.borrow_mut() = bars;
            app.redraw_wings();
        }
        // cava exited on its own (bad config, audio server restart): forget it so the next
        // `want(true)` can start a fresh one instead of keeping a dead handle forever.
        let Some(app) = weak.upgrade() else { return };
        let ours = app.cava.borrow().as_ref().is_some_and(|c| c.process == process);
        if ours {
            app.cava.borrow_mut().take();
            app.spectrum.borrow_mut().clear();
            app.redraw();
        }
    });
}
