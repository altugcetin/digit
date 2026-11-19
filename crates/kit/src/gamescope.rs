//! Gamescope puts a game in a window of its own, so the compositor sees every game it runs as
//! "gamescope". This finds the game inside: the Steam app the process was started for, or else
//! the program after `--` on its command line, and names it the way the game's own window would
//! be named (`steam_app_730`), so the dock, the switcher and the rest treat it as that game.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const WRAPPERS: &[&str] = &["env", "mangohud", "gamemoderun", "gamemode", "prime-run", "obs-gamecapture", "taskset", "nice", "ionice", "strangle", "dxvk-run", "steam-run", "proton", "umu-run", "wine", "wine64", "stdbuf", "nohup", "setsid", "sh", "bash"];
/// How long a "found nothing" stays: the game may not have been started yet.
const MISS: Duration = Duration::from_secs(2);
const LIMIT: usize = 64;

static SEEN: Mutex<BTreeMap<u32, (Instant, Option<String>)>> = Mutex::new(BTreeMap::new());

pub fn is_gamescope(app_id: &str) -> bool {
    app_id.eq_ignore_ascii_case("gamescope")
}

/// The id a window should go by: the game inside a gamescope window, the given id for any other.
pub fn real_id(app_id: &str, pid: Option<u32>) -> String {
    match pid.filter(|_| is_gamescope(app_id)).and_then(resolve) {
        Some(id) => id,
        None => app_id.to_string(),
    }
}

/// The game running under the gamescope process `pid`.
pub fn resolve(pid: u32) -> Option<String> {
    {
        let seen = SEEN.lock().ok()?;
        if let Some((at, found)) = seen.get(&pid)
            && (found.is_some() || at.elapsed() < MISS)
        {
            return found.clone();
        }
    }
    let found = find(pid);
    if let Ok(mut seen) = SEEN.lock() {
        if seen.len() > 128 {
            seen.retain(|_, (at, found)| found.is_none() && at.elapsed() < MISS);
        }
        seen.insert(pid, (Instant::now(), found.clone()));
    }
    found
}

fn find(pid: u32) -> Option<String> {
    for p in family(pid) {
        if let Some(app) = steam_app(&read(&format!("/proc/{p}/environ"))) {
            return Some(format!("steam_app_{app}"));
        }
    }
    program(&read(&format!("/proc/{pid}/cmdline")))
}

/// The process and what it started, itself first, a bounded number of them.
fn family(pid: u32) -> Vec<u32> {
    let mut all = vec![pid];
    let mut at = 0;
    while at < all.len() && all.len() < LIMIT {
        let p = all[at];
        at += 1;
        let Ok(tasks) = std::fs::read_dir(format!("/proc/{p}/task")) else { continue };
        for t in tasks.flatten() {
            let kids = std::fs::read_to_string(t.path().join("children")).unwrap_or_default();
            for k in kids.split_whitespace().filter_map(|k| k.parse::<u32>().ok()) {
                if !all.contains(&k) {
                    all.push(k);
                }
            }
        }
    }
    all.truncate(LIMIT);
    all
}

fn read(path: &str) -> Vec<u8> {
    std::fs::read(path).unwrap_or_default()
}

/// The Steam app in a process environment (NUL-separated), if it was started by Steam for one.
pub fn steam_app(environ: &[u8]) -> Option<u64> {
    let get = |name: &str| {
        environ.split(|b| *b == 0).find_map(|kv| {
            let kv = std::str::from_utf8(kv).ok()?;
            kv.strip_prefix(name)?.strip_prefix('=')?.parse::<u64>().ok().filter(|n| *n > 0)
        })
    };
    get("SteamAppId").or_else(|| get("SteamGameId"))
}

/// The program a gamescope command line runs: what follows `--`, past launch wrappers and
/// `VAR=value` words, named without its folder and `.exe`.
pub fn program(cmdline: &[u8]) -> Option<String> {
    let args: Vec<&str> = cmdline.split(|b| *b == 0).filter_map(|a| std::str::from_utf8(a).ok()).collect();
    let rest = &args[args.iter().position(|a| *a == "--")? + 1..];
    rest.iter().filter(|a| !a.is_empty() && !a.starts_with('-') && !is_assignment(a)).find_map(|a| {
        let base = a.rsplit(['/', '\\']).next().unwrap_or(a);
        let name = base.strip_suffix(".exe").or_else(|| base.strip_suffix(".EXE")).unwrap_or(base);
        (!name.is_empty() && !WRAPPERS.contains(&name)).then(|| name.to_string())
    })
}

fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(k, _)| !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
}

/// The id of a Steam game window class (`steam_app_730` → 730).
pub fn steam_id(app_id: &str) -> Option<u64> {
    app_id.strip_prefix("steam_app_")?.parse().ok()
}

/// The installed Steam game's name, read from its manifest in any library folder.
pub fn steam_name(app: u64) -> Option<String> {
    static NAMES: Mutex<BTreeMap<u64, Option<String>>> = Mutex::new(BTreeMap::new());
    if let Some(found) = NAMES.lock().ok()?.get(&app) {
        return found.clone();
    }
    let found = steam_roots().into_iter().find_map(|root| {
        let text = std::fs::read_to_string(root.join(format!("steamapps/appmanifest_{app}.acf"))).ok()?;
        text.lines().find_map(|l| {
            let mut quoted = l.split('"').skip(1).step_by(2);
            (quoted.next()? == "name").then(|| quoted.next().map(str::to_string)).flatten()
        })
    });
    if let Ok(mut names) = NAMES.lock() {
        names.insert(app, found.clone());
    }
    found
}

fn steam_roots() -> Vec<std::path::PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else { return Vec::new() };
    let mut roots = Vec::new();
    for base in [home.join(".local/share/Steam"), home.join(".steam/steam"), home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam")] {
        let libs = std::fs::read_to_string(base.join("steamapps/libraryfolders.vdf")).unwrap_or_default();
        for l in libs.lines() {
            let mut quoted = l.split('"').skip(1).step_by(2);
            if quoted.next() == Some("path") {
                roots.extend(quoted.next().map(std::path::PathBuf::from));
            }
        }
        roots.push(base);
    }
    roots
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steam_games_are_found_in_the_environment() {
        assert_eq!(steam_app(b"HOME=/h\0SteamAppId=730\0SteamGameId=999\0"), Some(730));
        assert_eq!(steam_app(b"SteamGameId=1091500\0"), Some(1091500));
        assert_eq!(steam_app(b"SteamAppId=0\0PATH=/bin\0"), None);
        assert_eq!(steam_app(b"MySteamAppId=5\0"), None);
    }

    #[test]
    fn the_program_after_the_dashes_is_the_game() {
        assert_eq!(program(b"gamescope\x00-W\x001920\x00--\x00/games/Hades/Hades\x00"), Some("Hades".into()));
        assert_eq!(program(b"gamescope\0--\0mangohud\0env\0A=1\0/g/Game.exe\0-fullscreen\0"), Some("Game".into()));
        assert_eq!(program(b"gamescope\0-f\0"), None);
        assert_eq!(program(b"gamescope\0--\0gamemoderun\0"), None);
    }

    #[test]
    fn a_running_process_is_read_from_proc() {
        let mut child = std::process::Command::new("sleep").arg("5").env("SteamAppId", "4242").spawn().unwrap();
        let found = resolve(child.id());
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(found.as_deref(), Some("steam_app_4242"));
    }

    #[test]
    fn only_gamescope_windows_are_renamed() {
        assert_eq!(real_id("firefox", Some(1)), "firefox");
        assert_eq!(real_id("gamescope", None), "gamescope");
        assert_eq!(steam_id("steam_app_730"), Some(730));
        assert_eq!(steam_id("steam_app_x"), None);
    }
}
