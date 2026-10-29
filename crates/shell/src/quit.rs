use crate::app::App;
use gio_unix::DesktopAppInfo;
use gtk::{gio, glib, prelude::*};
use std::os::unix::fs::MetadataExt;
use std::rc::Rc;
use std::time::Duration;

const KEEP: [&str; 17] = [
    "xwayland-satellite", "xwayland", "niri", "hyprland", "sway", "systemd", "dbus-daemon", "dbus-broker", "pipewire", "pipewire-pulse", "wireplumber", "digit", "noctalia", "qs", "quickshell", "flatpak", "bwrap",
];

const GENERIC: [&str; 26] = [
    "python", "python3", "python2", "java", "node", "nodejs", "electron", "ruby", "perl", "php", "wine", "wine64", "mono", "dotnet", "gjs", "lua", "env", "sh", "bash", "zsh", "fish", "snap", "appimagerun", "qemu-system-x86_64", "toolbox", "distrobox",
];

#[derive(Clone)]
struct Target {
    flatpak: Option<String>,
    exe: Option<String>,
    ids: Vec<String>,
}

fn safe_name(s: &str) -> bool {
    !s.is_empty() && s.len() < 128 && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn target(app: &App, key: &str) -> Target {
    let list = app.apps.list();
    let entry = app.apps.find(key).and_then(|i| list.get(i));
    let flatpak = entry.and_then(|e| e.info.downcast_ref::<DesktopAppInfo>()).and_then(|d| d.string("X-Flatpak")).map(|s| s.to_string()).filter(|s| safe_name(s));
    let exe = entry.map(|e| e.exec.clone()).filter(|x| safe_name(x) && !matches!(x.as_str(), "flatpak" | "gtk-launch" | "gio") && !KEEP.contains(&x.as_str()) && !GENERIC.contains(&x.as_str()) && !x.starts_with("python"));
    let mut ids = vec![key.trim_end_matches(".desktop").to_lowercase()];
    if let Some(e) = entry {
        ids.push(e.id.trim_end_matches(".desktop").to_lowercase());
        if !e.wm_class.is_empty() {
            ids.push(e.wm_class.to_lowercase());
        }
    }
    if let Some(f) = &flatpak {
        ids.push(f.to_lowercase());
    }
    ids.dedup();
    Target { flatpak, exe, ids }
}

fn mine(pid: u32) -> bool {
    let me = std::process::id();
    pid > 1 && pid != me && std::fs::metadata(format!("/proc/{pid}")).is_ok_and(|m| m.uid() == unsafe { libc::getuid() })
}

fn exe_of(pid: u32) -> Option<String> {
    let path = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    Some(path.file_name()?.to_string_lossy().trim_end_matches(" (deleted)").to_lowercase())
}

fn protected(pid: u32) -> bool {
    exe_of(pid).is_none_or(|x| KEEP.contains(&x.as_str()))
}

fn pids() -> impl Iterator<Item = u32> {
    std::fs::read_dir("/proc").into_iter().flatten().flatten().filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok()).filter(|p| mine(*p))
}

fn named(exe: &str) -> Vec<u32> {
    pids().filter(|p| exe_of(*p).is_some_and(|x| x == exe) && !protected(*p)).collect()
}

fn sandboxed(id: &str) -> Vec<u32> {
    let mark = format!("app-flatpak-{id}-");
    pids().filter(|p| std::fs::read_to_string(format!("/proc/{p}/cgroup")).is_ok_and(|c| c.contains(&mark))).collect()
}

pub fn running(app: &App, key: &str) -> bool {
    let t = target(app, key);
    match (&t.flatpak, &t.exe) {
        (Some(f), _) => !sandboxed(f).is_empty(),
        (None, Some(x)) => !named(x).is_empty(),
        _ => false,
    }
}

pub fn quit(app: &Rc<App>, key: &str, windows: Vec<u64>) {
    let t = target(app, key);
    glib::spawn_future_local(async move {
        let ids = windows.clone();
        let owners: Vec<u32> = gio::spawn_blocking(move || {
            let all = kit::wm::windows();
            let owners: Vec<u32> = all.iter().filter(|w| ids.contains(&w.id)).filter_map(|w| w.pid).collect();
            for id in ids {
                kit::wm::close(id);
            }
            owners
        })
        .await
        .unwrap_or_default();
        glib::timeout_future(Duration::from_millis(1200)).await;
        let probe = t.clone();
        let _ = gio::spawn_blocking(move || {
            let left = kit::wm::windows().iter().any(|w| probe.ids.contains(&w.app_id.to_lowercase()) || w.pid.is_some_and(|p| owners.contains(&p)));
            if left {
                return;
            }
            if let Some(f) = &probe.flatpak {
                if !sandboxed(f).is_empty() {
                    let _ = std::process::Command::new("flatpak").args(["kill", f.as_str()]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
                }
                return;
            }
            let mut victims: Vec<u32> = owners.into_iter().filter(|p| mine(*p) && !protected(*p)).collect();
            if victims.is_empty() {
                if let Some(x) = &probe.exe {
                    victims = named(x);
                }
            }
            for pid in victims {
                unsafe {
                    libc::kill(pid as i32, libc::SIGTERM);
                }
            }
        })
        .await;
    });
}
