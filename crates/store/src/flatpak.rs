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

/// What a cancelled job reports.
pub const CANCELLED: &str = "Cancelled";

/// Runs `job` for app `id`, reporting progress. Cancelling `cancel` stops flatpak the way Ctrl+C
/// would, so it cleans up after itself.
pub async fn run(id: &str, job: Job, progress: impl Fn(Option<f64>), cancel: &gio::Cancellable) -> Result<(), String> {
    if !valid(id) {
        return Err("Refused: not a valid application id".into());
    }
    let scope = |system: bool| if system { "--system" } else { "--user" };
    // Distributions like Arch set Flathub up for the whole system only, so an install for the
    // user alone would find no "flathub" there. Add it for the user too; that needs no password.
    if cancel.is_cancelled() {
        return Err(CANCELLED.into());
    }
    if let Job::Install(false, ref remote) = job
        && remote == "flathub"
    {
        let _ = output(&["remote-add", "--user", "--if-not-exists", "flathub", "https://dl.flathub.org/repo/flathub.flatpakrepo"]).await;
    }
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
    stream(args.iter().map(std::ffi::OsStr::new).collect(), progress, cancel).await
}

/// Installs a `.flatpak` bundle file for the user, which needs no password.
pub async fn install_bundle(path: &std::path::Path, progress: impl Fn(Option<f64>), cancel: &gio::Cancellable) -> Result<(), String> {
    let args: Vec<&std::ffi::OsStr> = vec!["install".as_ref(), "--user".as_ref(), "--noninteractive".as_ref(), "-y".as_ref(), "--bundle".as_ref(), path.as_os_str()];
    stream(args, progress, cancel).await
}

/// Output split into lines at `\r` (progress redrawn in place) or `\n`, across reads: a line a
/// read cut in two waits for the rest, so "4" then "5%" reads 45%, not 5%.
#[derive(Default)]
struct Lines(Vec<u8>);

impl Lines {
    fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.0.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(end) = self.0.iter().position(|b| matches!(b, b'\r' | b'\n')) {
            let line: Vec<u8> = self.0.drain(..=end).collect();
            out.push(String::from_utf8_lossy(&line[..end]).into_owned());
        }
        out
    }

    /// What is left at the end, when the output stops without a last line break.
    fn rest(&mut self) -> Vec<String> {
        let rest = std::mem::take(&mut self.0);
        if rest.is_empty() { Vec::new() } else { vec![String::from_utf8_lossy(&rest).into_owned()] }
    }
}

/// The percentage a progress line ends with ("Installing 3/7… 45%"), as a fraction.
fn percent(line: &str) -> Option<f64> {
    let i = line.rfind('%')?;
    let digits: String = line[..i].chars().rev().take_while(|c| c.is_ascii_digit()).collect::<Vec<_>>().into_iter().rev().collect();
    digits.parse::<u32>().ok().filter(|v| *v <= 100).map(|v| v as f64 / 100.0)
}

/// Runs flatpak with `args`, reading percentages from its output, and stops it when cancelled.
async fn stream(args: Vec<&std::ffi::OsStr>, progress: impl Fn(Option<f64>), cancel: &gio::Cancellable) -> Result<(), String> {
    let mut argv: Vec<&std::ffi::OsStr> = vec!["flatpak".as_ref()];
    argv.extend(args);
    let p = gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_MERGE).map_err(|e| e.to_string())?;
    let stop = cancel.connect_cancelled_local({
        let p = p.clone();
        move |_| {
            p.send_signal(libc::SIGINT);
            // Still there after a while: it won't stop by itself.
            let p = p.clone();
            glib::timeout_add_local_once(std::time::Duration::from_secs(4), move || p.force_exit());
        }
    });
    let stream = p.stdout_pipe().ok_or("no output")?;
    let mut tail = String::new();
    let mut lines = Lines::default();
    loop {
        let chunk = stream.read_bytes_future(4096, glib::Priority::DEFAULT).await.map_err(|e| e.to_string())?;
        let done = chunk.is_empty();
        let read = if done { lines.rest() } else { lines.push(&chunk) };
        if let Some(last) = read.iter().filter_map(|l| percent(l)).next_back() {
            progress(Some(last));
        }
        for line in read {
            tail.push_str(&line);
            tail.push('\n');
        }
        if tail.len() > 4096 {
            let cut = tail.floor_char_boundary(tail.len() - 2048);
            tail = tail[cut..].to_string();
        }
        if done {
            break;
        }
    }
    p.wait_future().await.map_err(|e| e.to_string())?;
    if let Some(stop) = stop {
        cancel.disconnect_cancelled(stop);
    }
    if cancel.is_cancelled() {
        return Err(CANCELLED.into());
    }
    if p.is_successful() {
        Ok(())
    } else {
        let line = tail.lines().rev().map(str::trim).find(|l| l.starts_with("error:") || l.starts_with("Error:")).unwrap_or("flatpak failed").to_string();
        Err(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_split_across_reads_reads_whole() {
        let mut lines = Lines::default();
        assert!(lines.push("Installing… 4".as_bytes()).is_empty());
        let read = lines.push("5%\rInstalling… 6".as_bytes());
        assert_eq!(read.iter().filter_map(|l| percent(l)).next_back(), Some(0.45));
        // A character cut between reads comes out whole.
        let snow = "☃".as_bytes();
        assert!(lines.push(&snow[..1]).is_empty());
        let read = lines.push(&[&snow[1..], b" 70%\n"].concat());
        assert!(read[0].ends_with("☃ 70%"), "{read:?}");
        assert!(lines.push(b"error: no space").is_empty());
        assert_eq!(lines.rest(), ["error: no space"]);
        assert_eq!(percent("100%"), Some(1.0));
        assert_eq!(percent("150%"), None);
        assert_eq!(percent("no number"), None);
    }
}
