use gtk::{gio, glib, prelude::*};
use std::collections::{HashMap, HashSet};

pub fn valid(id: &str) -> bool {
    id.split('.').count() >= 3 && id.len() < 256 && id.starts_with(|c: char| c.is_ascii_alphabetic()) && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

#[derive(Clone)]
pub struct Installed {
    pub version: String,
    pub system: bool,
    pub name: String,
}

async fn output(args: &[&str]) -> Result<String, String> {
    let mut argv: Vec<&std::ffi::OsStr> = vec!["flatpak".as_ref()];
    argv.extend(args.iter().map(|a| std::ffi::OsStr::new(*a)));
    let p = gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_PIPE).map_err(|e| e.to_string())?;
    let (out, err) = p.communicate_utf8_future(None).await.map_err(|e| e.to_string())?;
    if p.is_successful() {
        Ok(out.map(|s| s.to_string()).unwrap_or_default())
    } else {
        Err(err.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "flatpak failed".into()))
    }
}

pub async fn installed() -> HashMap<String, Installed> {
    let text = output(&["list", "--app", "--columns=application,version,installation,name"]).await.unwrap_or_default();
    text.lines()
        .filter_map(|l| {
            let c: Vec<&str> = l.split('\t').map(str::trim).collect();
            let id = *c.first()?;
            valid(id).then(|| (id.to_string(), Installed { version: c.get(1).unwrap_or(&"").to_string(), system: c.get(2) == Some(&"system"), name: c.get(3).unwrap_or(&"").to_string() }))
        })
        .collect()
}

pub async fn updates() -> HashSet<String> {
    let text = output(&["remote-ls", "--updates", "--app", "--columns=application"]).await.unwrap_or_default();
    text.lines().map(str::trim).filter(|l| valid(l)).map(str::to_string).collect()
}

#[derive(Clone)]
pub struct Remote {
    pub name: String,
    pub title: String,
    pub url: String,
    pub system: bool,
    pub disabled: bool,
}

pub async fn remotes() -> Vec<Remote> {
    let text = output(&["remotes", "--show-disabled", "--columns=name,title,url,options"]).await.unwrap_or_default();
    text.lines()
        .filter_map(|l| {
            let c: Vec<&str> = l.split('\t').map(str::trim).collect();
            let name = c.first()?.to_string();
            let opts = c.get(3).copied().unwrap_or("");
            Some(Remote { name, title: c.get(1).unwrap_or(&"").to_string(), url: c.get(2).unwrap_or(&"").to_string(), system: opts.contains("system"), disabled: opts.contains("disabled") })
        })
        .collect()
}

pub async fn remotes_with(id: &str) -> Vec<(String, bool)> {
    if !valid(id) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for r in remotes().await.into_iter().filter(|r| !r.disabled) {
        let scope = if r.system { "--system" } else { "--user" };
        if output(&["remote-info", scope, "--cached", &r.name, id]).await.is_ok() {
            out.push((r.name, r.system));
        }
    }
    out
}

pub async fn command(args: &[String]) -> Result<(), String> {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    output(&refs).await.map(|_| ())
}

pub enum Job {
    Install(bool, String),
    Remove(bool),
    Update(bool),
}

pub async fn run(id: &str, job: Job, progress: impl Fn(Option<f64>)) -> Result<(), String> {
    if !valid(id) {
        return Err("Refused: not a valid application id".into());
    }
    let scope = |system: bool| if system { "--system" } else { "--user" };
    let args: Vec<&str> = match job {
        Job::Install(system, ref remote) => {
            if remote.is_empty() || !remote.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) {
                return Err("Refused: not a valid source name".into());
            }
            vec!["install", scope(system), "--noninteractive", "-y", remote.as_str(), id]
        }
        Job::Remove(system) => vec!["uninstall", "--noninteractive", "-y", scope(system), id],
        Job::Update(system) => vec!["update", "--noninteractive", "-y", scope(system), id],
    };
    let mut argv: Vec<&std::ffi::OsStr> = vec!["flatpak".as_ref()];
    argv.extend(args.iter().map(|a| std::ffi::OsStr::new(*a)));
    let p = gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_MERGE).map_err(|e| e.to_string())?;
    let stream = p.stdout_pipe().ok_or("no output")?;
    let mut tail = String::new();
    loop {
        let chunk = stream.read_bytes_future(4096, glib::Priority::DEFAULT).await.map_err(|e| e.to_string())?;
        if chunk.is_empty() {
            break;
        }
        let text = String::from_utf8_lossy(&chunk);
        let mut last = None;
        for piece in text.split(['\r', '\n']) {
            if let Some(i) = piece.rfind('%') {
                let digits: String = piece[..i].chars().rev().take_while(|c| c.is_ascii_digit()).collect::<Vec<_>>().into_iter().rev().collect();
                if let Ok(v) = digits.parse::<u32>() {
                    if v <= 100 {
                        last = Some(v as f64 / 100.0);
                    }
                }
            }
        }
        if last.is_some() {
            progress(last);
        }
        tail.push_str(&text);
        if tail.len() > 4096 {
            let cut = tail.floor_char_boundary(tail.len() - 2048);
            tail = tail[cut..].to_string();
        }
    }
    p.wait_future().await.map_err(|e| e.to_string())?;
    if p.is_successful() {
        Ok(())
    } else {
        let line = tail.lines().rev().map(str::trim).find(|l| l.starts_with("error:") || l.starts_with("Error:")).unwrap_or("flatpak failed").to_string();
        Err(line)
    }
}
