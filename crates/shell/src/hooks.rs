use gtk::{gio, glib};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

fn path(name: &str) -> Option<PathBuf> {
    let file = crate::config::dir().join("hooks").join(name);
    let meta = std::fs::metadata(&file).ok()?;
    (meta.is_file() && meta.permissions().mode() & 0o111 != 0).then_some(file)
}

pub fn run(name: &str, args: &[&str]) -> bool {
    path(name).is_some_and(|file| std::process::Command::new(file).args(args).status().is_ok_and(|s| s.success()))
}

pub fn spawn(name: &str) {
    let Some(file) = path(name) else { return };
    let argv = [file.as_os_str()];
    let Ok(process) = gio::Subprocess::newv(&argv, gio::SubprocessFlags::NONE) else { return };
    glib::spawn_future_local(async move {
        let _ = process.wait_future().await;
    });
}
