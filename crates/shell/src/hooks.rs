use gtk::{gio, glib};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

const PATIENCE: std::time::Duration = std::time::Duration::from_secs(2);

fn path(name: &str) -> Option<PathBuf> {
    let file = crate::config::dir().join("hooks").join(name);
    let meta = std::fs::metadata(&file).ok()?;
    (meta.is_file() && meta.permissions().mode() & 0o111 != 0).then_some(file)
}

/// True if the hook finished and succeeded in time. A slow hook keeps running, unattended.
pub fn run(name: &str, args: &[&str]) -> bool {
    let Some(file) = path(name) else { return false };
    let Ok(mut child) = std::process::Command::new(file).args(args).spawn() else { return false };
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(child.wait().is_ok_and(|s| s.success()));
    });
    finished.recv_timeout(PATIENCE).unwrap_or(false)
}

pub fn spawn(name: &str) {
    let Some(file) = path(name) else { return };
    let argv = [file.as_os_str()];
    let Ok(process) = gio::Subprocess::newv(&argv, gio::SubprocessFlags::NONE) else { return };
    glib::spawn_future_local(async move {
        let _ = process.wait_future().await;
    });
}
