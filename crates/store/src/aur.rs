//! Optional Arch User Repository search and interactive helper operations.

use gtk::{gio, glib};
use serde::Deserialize;
use std::cmp::Reverse;
use std::collections::HashSet;
use std::path::PathBuf;

pub const PREF: &str = "store_aur";

pub struct Helper {
    pub name: &'static str,
    path: PathBuf,
}

pub fn helper() -> Option<Helper> {
    find_helper(|name| glib::find_program_in_path(name))
}

fn find_helper(find: impl Fn(&str) -> Option<PathBuf>) -> Option<Helper> {
    ["yay", "paru"]
        .into_iter()
        .find_map(|name| find(name).map(|path| Helper { name, path }))
}

pub fn requested() -> bool {
    kit::prefs::flag(&kit::prefs::read(), PREF, false)
}

pub fn enabled() -> bool {
    requested() && helper().is_some()
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Package {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    #[serde(default)]
    num_votes: u32,
}

#[derive(Deserialize)]
struct Response {
    #[serde(default)]
    results: Vec<Package>,
    error: Option<String>,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"@._+-".contains(&b))
}

fn parse(text: &str) -> Result<Vec<Package>, String> {
    let response: Response =
        serde_json::from_str(text).map_err(|_| "AUR returned an invalid response".to_string())?;
    if let Some(error) = response.error {
        return Err(error);
    }
    let mut names = HashSet::new();
    Ok(response
        .results
        .into_iter()
        .filter(|p| valid_name(&p.name) && names.insert(p.name.clone()))
        .collect())
}

async fn request(url: String) -> Result<Vec<Package>, String> {
    if !enabled() {
        return Ok(Vec::new());
    }
    let args = [
        "curl",
        "-q",
        "-fsS",
        "--max-time",
        "12",
        "--max-filesize",
        "2097152",
        "--url",
        &url,
    ];
    let args: Vec<_> = args.iter().map(std::ffi::OsStr::new).collect();
    let process = gio::Subprocess::newv(
        &args,
        gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_SILENCE,
    )
    .map_err(|_| "Couldn't start the AUR request".to_string())?;
    let (out, _) = process
        .communicate_utf8_future(None)
        .await
        .map_err(|e| e.to_string())?;
    if !process.is_successful() {
        return Err("Couldn't reach the AUR. Try again in a moment.".into());
    }
    parse(out.as_deref().unwrap_or_default())
}

pub fn searchable(query: &str) -> bool {
    (2..=80).contains(&query.chars().count()) && !query.chars().any(char::is_control)
}

pub async fn search(query: &str) -> Result<Vec<Package>, String> {
    if !searchable(query) {
        return Ok(Vec::new());
    }
    let query = query.to_ascii_lowercase();
    let term = glib::Uri::escape_string(&query, None, false);
    let mut packages = request(format!(
        "https://aur.archlinux.org/rpc/v5/search/{term}?by=name-desc"
    ))
    .await?;
    packages.sort_by_cached_key(|p| {
        let name = p.name.to_ascii_lowercase();
        (
            name != query,
            !name.starts_with(&query),
            Reverse(p.num_votes),
            name,
        )
    });
    packages.truncate(24);
    Ok(packages)
}

pub async fn lookup(names: &[&str]) -> Result<Vec<Package>, String> {
    let args: Vec<_> = names
        .iter()
        .filter(|name| valid_name(name))
        .map(|name| format!("arg%5B%5D={}", glib::Uri::escape_string(name, None, false)))
        .collect();
    if args.is_empty() {
        return Ok(Vec::new());
    }
    let mut found = request(format!(
        "https://aur.archlinux.org/rpc/v5/info?{}",
        args.join("&")
    ))
    .await?;
    found.retain(|p| names.contains(&p.name.as_str()));
    Ok(found)
}

fn with_helper(run: impl FnOnce(&str) -> Result<(), String>) -> Result<(), String> {
    if !requested() {
        return Err("Enable the AUR in Sources first".into());
    }
    let helper = helper().ok_or("Install yay or paru and make sure it is on PATH")?;
    let path = helper
        .path
        .to_str()
        .ok_or("The AUR helper path is not valid UTF-8")?;
    run(path)
}

pub fn install(package: &str) -> Result<(), String> {
    if !valid_name(package) {
        return Err("Invalid AUR package name".into());
    }
    with_helper(|path| crate::sources::terminal(&[path, "-S", "--aur", "--needed", "--", package]))
}

pub fn update() -> Result<(), String> {
    with_helper(|path| crate::sources::terminal(&[path, "-Sua"]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_yay_and_falls_back_to_paru() {
        assert_eq!(
            find_helper(|name| Some(PathBuf::from(name))).unwrap().name,
            "yay"
        );
        assert_eq!(
            find_helper(|name| (name == "paru").then(|| PathBuf::from(name)))
                .unwrap()
                .name,
            "paru"
        );
        assert!(find_helper(|_| None).is_none());
    }

    #[test]
    fn filters_invalid_rpc_targets_and_reports_rpc_errors() {
        let response = r#"{"results":[{"Name":"code-bin","Version":"1","Description":null},
            {"Name":"--flag","Version":"1"},{"Name":"../../other","Version":"1"},
            {"Name":"code-bin","Version":"1"}]}"#;
        let packages = parse(response).unwrap();
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].name, "code-bin");
        assert_eq!(
            parse(r#"{"error":"Too many results"}"#).err().as_deref(),
            Some("Too many results")
        );
        assert!(!searchable("a"));
    }
}
