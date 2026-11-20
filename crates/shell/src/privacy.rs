use crate::app::App;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString};
use std::os::unix::ffi::OsStrExt;
use std::rc::Rc;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Light {
    Camera,
    Mic,
    Screen,
    Record,
}

impl Light {
    pub fn icon(self) -> &'static str {
        match self {
            Light::Camera => "videocam",
            Light::Mic => "mic",
            Light::Screen => "screen_share",
            Light::Record => "radio_button_checked",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Light::Camera => "Camera",
            Light::Mic => "Microphone",
            Light::Screen => "Screen sharing",
            Light::Record => "Recording",
        }
    }
}

#[derive(Clone, Default, PartialEq)]
pub struct Lights {
    pub camera: Vec<String>,
    pub mic: Vec<String>,
    pub talk: Vec<String>,
    pub screen: usize,
    /// What digit's own recorder is doing; it stands in for the screen sharing light.
    pub record: Option<crate::gsr::Mode>,
}

impl Lights {
    pub fn capture(&self) -> bool {
        !self.camera.is_empty() || !self.mic.is_empty()
    }

    pub fn count(&self) -> usize {
        usize::from(self.capture()) + usize::from(self.sharing())
    }

    pub fn sharing(&self) -> bool {
        self.screen > 0 || self.record.is_some()
    }
}

pub fn start(app: &Rc<App>) {
    camera(app);
    crate::compositor::watch(app);
}

fn holders() -> (i64, Vec<u32>) {
    let mut count = 0;
    let mut pids = Vec::new();
    let Ok(procs) = std::fs::read_dir("/proc") else { return (0, pids) };
    for p in procs.flatten() {
        let name = p.file_name();
        let Some(pid) = name.to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
        let Ok(fds) = std::fs::read_dir(p.path().join("fd")) else { continue };
        let n = fds.flatten().filter(|f| std::fs::read_link(f.path()).is_ok_and(|l| l.as_os_str().as_bytes().starts_with(b"/dev/video"))).count();
        if n > 0 {
            count += n as i64;
            pids.push(pid);
        }
    }
    (count, pids)
}

fn watch(fd: i32, name: &[u8]) -> Option<i32> {
    let path = CString::new([b"/dev/".as_slice(), name].concat()).ok()?;
    // SAFETY: `path` is a NUL-terminated string that outlives the call.
    let wd = unsafe { libc::inotify_add_watch(fd, path.as_ptr(), libc::IN_OPEN | libc::IN_CLOSE_WRITE | libc::IN_CLOSE_NOWRITE) };
    (wd >= 0).then_some(wd)
}

struct Camera {
    open: Cell<i64>,
    pids: RefCell<Vec<u32>>,
    pending: Cell<bool>,
    polling: Cell<bool>,
    seen: Cell<u32>,
}

fn awake() -> bool {
    let Ok(dir) = std::fs::read_dir("/sys/class/video4linux") else { return false };
    dir.flatten().any(|e| {
        ["device/../power/runtime_status", "device/power/runtime_status"]
            .iter()
            .find_map(|rel| std::fs::read_to_string(e.path().join(rel)).ok())
            .is_some_and(|s| s.trim() == "active")
    })
}

fn mapped(pids: &[u32]) -> bool {
    pids.iter().any(|p| std::fs::read(format!("/proc/{p}/maps")).is_ok_and(|b| b.windows(10).any(|w| w == b"/dev/video")))
}

fn light(app: &Rc<App>, on: bool) {
    app.set_lights(|l| l.camera = if on { vec!["camera".into()] } else { Vec::new() });
}

fn check(app: &Rc<App>, cam: &Rc<Camera>) {
    let weak = Rc::downgrade(app);
    let cam = cam.clone();
    let pids = cam.pids.borrow().clone();
    glib::spawn_future_local(async move {
        let streaming = gio::spawn_blocking(move || awake() && mapped(&pids)).await.unwrap_or(false);
        let Some(app) = weak.upgrade() else { return };
        let raw = cam.open.get() > 0 && streaming;
        cam.seen.set(if raw { cam.seen.get() + 1 } else { 0 });
        light(&app, cam.seen.get() >= 2);
    });
}

fn poll(app: &Rc<App>, cam: &Rc<Camera>) {
    if cam.polling.replace(true) {
        return;
    }
    let weak = Rc::downgrade(app);
    let cam = cam.clone();
    glib::timeout_add_local(Duration::from_millis(1500), move || {
        let Some(app) = weak.upgrade() else { return glib::ControlFlow::Break };
        if cam.open.get() <= 0 {
            cam.polling.set(false);
            return glib::ControlFlow::Break;
        }
        check(&app, &cam);
        glib::ControlFlow::Continue
    });
}

fn settle(app: &Rc<App>, cam: &Rc<Camera>) {
    if cam.pending.replace(true) {
        return;
    }
    let weak = Rc::downgrade(app);
    let cam = cam.clone();
    glib::timeout_add_local_once(Duration::from_millis(350), move || {
        glib::spawn_future_local(async move {
            let (n, pids) = gio::spawn_blocking(holders).await.unwrap_or((0, Vec::new()));
            cam.pending.set(false);
            let Some(app) = weak.upgrade() else { return };
            cam.open.set(n);
            *cam.pids.borrow_mut() = pids;
            if n > 0 {
                check(&app, &cam);
                poll(&app, &cam);
            } else {
                cam.seen.set(0);
                light(&app, false);
            }
        });
    });
}

fn camera(app: &Rc<App>) {
    // SAFETY: takes only flags and returns a new descriptor or -1.
    let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
    if fd < 0 {
        return;
    }
    // SAFETY: a static NUL-terminated path on the descriptor just created.
    let dev = unsafe { libc::inotify_add_watch(fd, c"/dev".as_ptr(), libc::IN_CREATE) };
    let mut any = false;
    if let Ok(entries) = std::fs::read_dir("/dev") {
        for e in entries.flatten() {
            let name = e.file_name();
            if name.as_bytes().starts_with(b"video") {
                any |= watch(fd, name.as_bytes()).is_some();
            }
        }
    }
    let cam = Rc::new(Camera { open: Cell::new(0), pids: RefCell::new(Vec::new()), pending: Cell::new(false), polling: Cell::new(false), seen: Cell::new(0) });
    if any {
        settle(app, &cam);
    }
    let weak = Rc::downgrade(app);
    let head = std::mem::size_of::<libc::inotify_event>();
    glib_unix::unix_fd_add_local(fd, glib::IOCondition::IN, move |fd, _| {
        let Some(app) = weak.upgrade() else { return glib::ControlFlow::Break };
        let mut buf = [0u8; 4096];
        let mut touched = false;
        loop {
            // SAFETY: reads at most `buf.len()` bytes into `buf`, which is writable and that long.
            let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
            if n <= 0 {
                break;
            }
            let n = n as usize;
            let mut off = 0;
            while off + head <= n {
                // SAFETY: `off + head <= n` was checked, so a whole event header lies in the bytes read; read_unaligned copes with its alignment.
                let ev: libc::inotify_event = unsafe { std::ptr::read_unaligned(buf.as_ptr().add(off).cast()) };
                let name = &buf[(off + head).min(n)..(off + head + ev.len as usize).min(n)];
                off += head + ev.len as usize;
                if ev.mask & libc::IN_Q_OVERFLOW != 0 {
                    touched = true;
                } else if ev.wd == dev {
                    let name = CStr::from_bytes_until_nul(name).map(CStr::to_bytes).unwrap_or(name);
                    if ev.mask & libc::IN_CREATE != 0 && name.starts_with(b"video") {
                        watch(fd, name);
                    }
                } else if ev.mask & (libc::IN_OPEN | libc::IN_CLOSE_WRITE | libc::IN_CLOSE_NOWRITE) != 0 {
                    touched = true;
                }
            }
        }
        if touched {
            settle(&app, &cam);
        }
        glib::ControlFlow::Continue
    });
}
