use crate::app::App;
use gtk::{gio, glib, prelude::*};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::Duration;

const APP_ID: &str = "dev.digit.Settings";

/// What gpu-screen-recorder runs as. A regular recording can also run on top of a replay buffer
/// or a stream (`-ro`); `Run::taping` tracks that one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    Record,
    Replay,
    Stream,
}

/// The running recorder. Kept in the runtime dir so a restarted shell picks it up again: the
/// recorder is not our child's child, it outlives the shell.
#[derive(Clone, Serialize, Deserialize)]
struct Run {
    pid: i32,
    mode: Mode,
    /// Wall clock, µs.
    since: i64,
    taping: Option<i64>,
    paused: Option<i64>,
    held: i64,
    replay: u32,
}

#[derive(Default)]
struct Engine {
    app: Weak<App>,
    run: Option<Run>,
    starting: bool,
    stopping: bool,
}

thread_local! {
    static ENGINE: RefCell<Engine> = RefCell::new(Engine::default());
}

#[derive(Clone, Copy, PartialEq)]
pub struct Status {
    pub mode: Mode,
    pub taping: bool,
    pub paused: bool,
    /// Length of the regular recording, or how long the replay buffer or stream has been on.
    pub secs: u64,
    pub replay: u32,
}

fn runtime(name: &str) -> PathBuf {
    glib::user_runtime_dir().join(name)
}

fn state_file() -> PathBuf {
    runtime("digit-rec.json")
}

fn socket() -> PathBuf {
    runtime("digit-rec.sock")
}

fn log_file() -> PathBuf {
    runtime("digit-rec.log")
}

pub fn installed() -> bool {
    glib::find_program_in_path("gpu-screen-recorder").is_some()
}

pub fn folder() -> PathBuf {
    let custom = kit::prefs::text(&kit::prefs::read(), "rec_dir", "");
    if !custom.is_empty() {
        return kit::prefs::home(&custom);
    }
    glib::user_special_dir(glib::UserDirectory::Videos).unwrap_or_else(|| glib::home_dir().join("Videos")).join("Recordings")
}

/// The stream address holds the stream key, so it lives in its own private file, not in config.toml.
pub fn stream_url() -> String {
    std::fs::read_to_string(glib::user_config_dir().join("digit/rec-stream")).map(|s| s.trim().to_string()).unwrap_or_default()
}

pub fn status() -> Option<Status> {
    ENGINE.with(|e| {
        let e = e.borrow();
        let r = e.run.as_ref()?;
        let now = glib::real_time();
        let from = r.taping.unwrap_or(r.since);
        let paused = r.paused.map_or(0, |p| now - p);
        Some(Status { mode: r.mode, taping: r.taping.is_some(), paused: r.paused.is_some(), secs: ((now - from - r.held - paused).max(0) / 1_000_000) as u64, replay: r.replay })
    })
}

pub fn busy() -> bool {
    ENGINE.with(|e| e.borrow().starting)
}

pub fn clock(secs: u64) -> String {
    if secs >= 3600 { format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60) } else { format!("{:02}:{:02}", secs / 60, secs % 60) }
}

pub fn span(secs: u32) -> String {
    match secs {
        s if s >= 60 && s % 60 == 0 => format!("{} min", s / 60),
        s => format!("{s} s"),
    }
}

pub fn notify(summary: &str, body: &str) {
    let (summary, body) = (summary.to_string(), body.to_string());
    glib::spawn_future_local(async move {
        let Ok(conn) = gio::bus_get_future(gio::BusType::Session).await else { return };
        let hints = glib::VariantDict::new(None);
        hints.insert("desktop-entry", APP_ID);
        let actions: Vec<String> = Vec::new();
        let args = ("Recorder", 0u32, APP_ID, summary.as_str(), body.as_str(), actions, hints.end(), 5000i32).to_variant();
        let _ = conn.call_future(Some("org.freedesktop.Notifications"), "/org/freedesktop/Notifications", "org.freedesktop.Notifications", "Notify", Some(&args), None, gio::DBusCallFlags::NONE, 3000).await;
    });
}

fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string())
}

fn changed() {
    let (app, mode) = ENGINE.with(|e| {
        let e = e.borrow();
        let mode = e.run.as_ref().map(|r| if r.mode == Mode::Replay && r.taping.is_some() { Mode::Record } else { r.mode });
        (e.app.upgrade(), mode)
    });
    if let Some(app) = app {
        app.set_lights(|l| l.record = mode);
    }
    crate::gamebar::changed();
}

fn save() {
    let run = ENGINE.with(|e| e.borrow().run.clone());
    match run.and_then(|r| serde_json::to_string(&r).ok()) {
        Some(text) => {
            let _ = std::fs::write(state_file(), text);
        }
        None => {
            let _ = std::fs::remove_file(state_file());
        }
    }
}

fn update(f: impl FnOnce(&mut Run)) {
    ENGINE.with(|e| {
        if let Some(r) = e.borrow_mut().run.as_mut() {
            f(r);
        }
    });
    save();
    changed();
}

fn alive(pid: i32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| {
        let (name, rest) = s.split_once(") ").unwrap_or(("", ""));
        name.contains("gpu-screen-rec") && !rest.starts_with('Z')
    })
}

/// Picks up a recorder a previous shell started.
pub fn restore(app: &Rc<App>) {
    ENGINE.with(|e| e.borrow_mut().app = Rc::downgrade(app));
    let run: Option<Run> = std::fs::read_to_string(state_file()).ok().and_then(|t| serde_json::from_str(&t).ok());
    match run {
        Some(r) if alive(r.pid) => {
            let pid = r.pid;
            ENGINE.with(|e| e.borrow_mut().run = Some(r));
            changed();
            glib::timeout_add_local(Duration::from_secs(2), move || {
                if alive(pid) {
                    return glib::ControlFlow::Continue;
                }
                ended(pid);
                glib::ControlFlow::Break
            });
        }
        _ => {
            let _ = std::fs::remove_file(state_file());
        }
    }
}

fn ended(pid: i32) {
    let (run, asked) = ENGINE.with(|e| {
        let mut e = e.borrow_mut();
        if e.run.as_ref().is_none_or(|r| r.pid != pid) {
            return (None, false);
        }
        (e.run.take(), std::mem::take(&mut e.stopping))
    });
    let Some(run) = run else { return };
    save();
    changed();
    if !asked {
        let log = std::fs::read_to_string(log_file()).unwrap_or_default();
        let last = log.lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or("The recorder stopped").to_string();
        let early = glib::real_time() - run.since < 3_000_000;
        notify(if early { "Recording didn't start" } else { "Recording stopped" }, &last);
    }
}

/// One request to the recorder's control socket; the reply's data is the saved file, if any.
async fn ask(name: &'static str, data: serde_json::Value) -> Result<Option<String>, String> {
    let line = serde_json::json!({ "id": 1, "name": name, "data": data }).to_string();
    let path = socket();
    gio::spawn_blocking(move || -> Result<Option<String>, String> {
        let mut s = UnixStream::connect(&path).map_err(|e| e.to_string())?;
        s.set_read_timeout(Some(Duration::from_secs(90))).map_err(|e| e.to_string())?;
        writeln!(s, "{line}").map_err(|e| e.to_string())?;
        let mut reply = String::new();
        BufReader::new(s).read_line(&mut reply).map_err(|e| e.to_string())?;
        let v: serde_json::Value = serde_json::from_str(&reply).map_err(|e| e.to_string())?;
        let data = v.get("data").and_then(|d| d.as_str()).map(str::to_string);
        if v.get("result").and_then(|r| r.as_str()) == Some("ok") { Ok(data) } else { Err(data.unwrap_or_else(|| "the recorder said no".into())) }
    })
    .await
    .unwrap_or_else(|_| Err("the recorder did not answer".into()))
}

fn signal(sig: i32) -> bool {
    match ENGINE.with(|e| e.borrow().run.as_ref().map(|r| r.pid)) {
        Some(pid) => unsafe { libc::kill(pid, sig) == 0 },
        None => false,
    }
}

async fn region() -> Option<String> {
    let argv: [&OsStr; 3] = ["slurp".as_ref(), "-f".as_ref(), "%wx%h+%x+%y".as_ref()];
    let p = gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_SILENCE).ok()?;
    let (out, _) = p.communicate_utf8_future(None).await.ok()?;
    let out = out?.trim().to_string();
    (p.is_successful() && !out.is_empty()).then_some(out)
}

fn monitors() -> Vec<String> {
    gtk::gdk::Display::default()
        .map(|d| d.monitors().iter::<gtk::gdk::Monitor>().filter_map(Result::ok).filter_map(|m| m.connector().map(|c| c.to_string())).collect())
        .unwrap_or_default()
}

async fn target() -> Option<(String, Option<String>)> {
    let want = kit::prefs::text(&kit::prefs::read(), "rec_target", "monitor");
    let screens = monitors();
    match want.as_str() {
        "region" => Some(("region".into(), Some(region().await?))),
        "portal" => Some(("portal".into(), None)),
        w if screens.iter().any(|s| s == w) => Some((w.to_string(), None)),
        _ => {
            let focused = gio::spawn_blocking(kit::wm::focused_output).await.ok().flatten();
            let screen = focused.filter(|f| screens.contains(f)).or_else(|| screens.first().cloned()).unwrap_or_else(|| "screen".into());
            Some((screen, None))
        }
    }
}

fn argv(mode: Mode, target: &str, region: Option<&str>) -> Result<(Vec<String>, u32), String> {
    let t = kit::prefs::read();
    let text = |k: &str, f: &str| kit::prefs::text(&t, k, f);
    let mut a: Vec<String> = vec!["gpu-screen-recorder".into(), "-w".into(), target.into()];
    if let Some(r) = region {
        a.extend(["-region".into(), r.into()]);
    }
    if target == "portal" {
        a.extend(["-restore-portal-session".into(), "yes".into()]);
    }
    a.extend(["-f".into(), text("rec_fps", "60"), "-q".into(), text("rec_quality", "very_high")]);
    let codec = text("rec_codec", "auto");
    if codec != "auto" {
        a.extend(["-k".into(), codec]);
    }
    a.extend(["-cursor".into(), if kit::prefs::flag(&t, "rec_cursor", true) { "yes" } else { "no" }.into()]);
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
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let dir_s = dir.to_string_lossy().into_owned();
    let container = text("rec_container", "mp4");
    let replay = text("rec_replay", "60").parse::<u32>().unwrap_or(60).clamp(5, 1800);
    match mode {
        Mode::Record => {
            let stamp = glib::DateTime::now_local().ok().and_then(|d| d.format("Recording %Y-%m-%d at %H.%M.%S").ok()).map(|s| s.to_string()).unwrap_or_else(|| "Recording".into());
            a.extend(["-c".into(), container.clone(), "-o".into(), dir.join(format!("{stamp}.{container}")).to_string_lossy().into_owned()]);
        }
        Mode::Replay => {
            a.extend(["-c".into(), container, "-r".into(), replay.to_string(), "-replay-storage".into(), "ram".into(), "-ro".into(), dir_s.clone(), "-o".into(), dir_s]);
        }
        Mode::Stream => {
            let url = stream_url();
            let kind = if url.starts_with("srt://") {
                "mpegts"
            } else if url.starts_with("rtmp://") || url.starts_with("rtmps://") {
                "flv"
            } else {
                return Err("Add an rtmp://, rtmps:// or srt:// address in the recorder settings first".into());
            };
            a.extend(["-c".into(), kind.into(), "-ro".into(), dir_s, "-o".into(), url]);
        }
    }
    a.extend(["-ipc".into(), socket().to_string_lossy().into_owned()]);
    Ok((a, replay))
}

pub fn start(app: &Rc<App>, mode: Mode) {
    let blocked = ENGINE.with(|e| {
        let mut e = e.borrow_mut();
        e.app = Rc::downgrade(app);
        let blocked = e.starting || e.run.is_some();
        if !blocked {
            e.starting = true;
        }
        blocked
    });
    if blocked {
        return;
    }
    if !installed() {
        ENGINE.with(|e| e.borrow_mut().starting = false);
        notify("GPU Screen Recorder isn't installed", "Install the gpu-screen-recorder package to record the screen");
        return;
    }
    crate::gamebar::changed();
    glib::spawn_future_local(async move {
        let built = match target().await {
            Some((target, region)) => argv(mode, &target, region.as_deref()),
            None => Err(String::new()),
        };
        let spawned = built.and_then(|(a, replay)| {
            let launcher = gio::SubprocessLauncher::new(gio::SubprocessFlags::STDIN_PIPE | gio::SubprocessFlags::STDOUT_SILENCE);
            launcher.set_stderr_file_path(Some(log_file()));
            let refs: Vec<&OsStr> = a.iter().map(OsStr::new).collect();
            launcher.spawn(&refs).map(|p| (p, replay)).map_err(|e| e.to_string())
        });
        ENGINE.with(|e| e.borrow_mut().starting = false);
        let (p, replay) = match spawned {
            Ok(x) => x,
            Err(e) => {
                if !e.is_empty() {
                    notify("Can't start recording", &e);
                }
                crate::gamebar::changed();
                return;
            }
        };
        let Some(pid) = p.identifier().and_then(|i| i.parse::<i32>().ok()) else { return };
        let now = glib::real_time();
        let run = Run { pid, mode, since: now, taping: (mode == Mode::Record).then_some(now), paused: None, held: 0, replay };
        ENGINE.with(|e| e.borrow_mut().run = Some(run));
        save();
        changed();
        let _ = p.wait_future().await;
        ended(pid);
    });
}

fn saved(what: &str, path: &str) {
    notify(&format!("{what} saved"), &file_name(path));
}

/// Stops everything the recorder runs, saving a recording in progress.
pub fn stop() -> bool {
    let Some(st) = status() else { return false };
    ENGINE.with(|e| e.borrow_mut().stopping = true);
    glib::spawn_future_local(async move {
        if st.mode != Mode::Record && st.taping {
            if let Ok(Some(path)) = ask("stop-replay-recording", serde_json::Value::Null).await {
                saved("Recording", &path);
            }
        }
        match ask("stop", serde_json::Value::Null).await {
            Ok(Some(path)) => saved("Recording", &path),
            Ok(None) => {}
            Err(_) => {
                signal(libc::SIGINT);
            }
        }
    });
    true
}

/// Starts or stops a regular recording. With a replay buffer or stream running, the recording
/// runs on top of it instead of starting a second capture.
pub fn toggle_record(app: &Rc<App>) {
    match status() {
        None => start(app, Mode::Record),
        Some(st) if st.mode == Mode::Record => {
            stop();
        }
        Some(st) => {
            let taping = st.taping;
            glib::spawn_future_local(async move {
                if taping {
                    match ask("stop-replay-recording", serde_json::Value::Null).await {
                        Ok(path) => {
                            update(|r| r.taping = None);
                            if let Some(path) = path {
                                saved("Recording", &path);
                            }
                        }
                        Err(e) => notify("Can't stop the recording", &e),
                    }
                } else {
                    match ask("start-replay-recording", serde_json::Value::Null).await {
                        Ok(_) => update(|r| r.taping = Some(glib::real_time())),
                        Err(e) => notify("Can't start recording", &e),
                    }
                }
            });
        }
    }
}

pub fn record(app: &Rc<App>, on: bool) {
    let taping = status().is_some_and(|s| s.taping);
    if on != taping {
        toggle_record(app);
    }
}

pub fn pause() -> bool {
    if status().is_none_or(|s| s.mode != Mode::Record) {
        return false;
    }
    glib::spawn_future_local(async move {
        let ok = ask("toggle-pause", serde_json::Value::Null).await.is_ok() || signal(libc::SIGUSR2);
        if ok {
            update(|r| {
                let now = glib::real_time();
                match r.paused.take() {
                    Some(at) => r.held += now - at,
                    None => r.paused = Some(now),
                }
            });
        }
    });
    true
}

/// Saves the replay buffer, all of it or the last `secs` seconds.
pub fn save_replay(secs: Option<u32>) -> bool {
    let Some(st) = status().filter(|s| s.mode == Mode::Replay) else { return false };
    let data = match secs {
        Some(s) if s > 0 && s < st.replay => serde_json::json!({ "seconds": s }),
        _ => serde_json::Value::Null,
    };
    let length = secs.filter(|s| *s > 0 && *s < st.replay).unwrap_or(st.replay);
    glib::spawn_future_local(async move {
        match ask("save-replay", data).await {
            Ok(path) => notify(&format!("Replay saved · last {}", span(length)), &path.as_deref().map(file_name).unwrap_or_default()),
            Err(_) if signal(libc::SIGUSR1) => notify("Replay saved", &folder().display().to_string()),
            Err(e) => notify("Can't save the replay", &e),
        }
    });
    true
}

/// Turns the replay buffer or the stream on or off; `None` flips it.
pub fn switch(app: &Rc<App>, mode: Mode, want: Option<bool>) -> Result<(), String> {
    let st = status();
    let on = st.is_some_and(|s| s.mode == mode);
    let want = want.unwrap_or(!on);
    if want == on {
        return Ok(());
    }
    if !want {
        stop();
        return Ok(());
    }
    if st.is_some() {
        return Err("the recorder is busy; stop it first".into());
    }
    if mode == Mode::Stream && stream_url().is_empty() {
        settings();
        return Err("add a stream address first".into());
    }
    start(app, mode);
    Ok(())
}

pub fn settings() {
    let argv: [&OsStr; 2] = ["digit-settings".as_ref(), "recording".as_ref()];
    let _ = gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE);
}

pub fn open_folder() {
    let dir = folder();
    let _ = std::fs::create_dir_all(&dir);
    let _ = gio::AppInfo::launch_default_for_uri(&gio::File::for_path(dir).uri(), gio::AppLaunchContext::NONE);
}

pub fn describe() -> serde_json::Value {
    match status() {
        None => serde_json::json!({ "running": false }),
        Some(s) => serde_json::json!({
            "running": true,
            "mode": match s.mode { Mode::Record => "record", Mode::Replay => "replay", Mode::Stream => "stream" },
            "recording": s.taping,
            "paused": s.paused,
            "seconds": s.secs,
            "replay_seconds": s.replay,
        }),
    }
}
