//! PrismLauncher's Minecraft instances for Spotlight: `/pl` lists them and starts the one picked.
//! Off unless Settings > Spotlight turns it on. Only reads Prism's files, never changes them.

use std::path::{Path, PathBuf};

const FLATPAK: &str = "org.prismlauncher.PrismLauncher";

/// One instance, as Prism shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Instance {
    /// The folder name, which `--launch` takes.
    pub id: String,
    pub name: String,
    pub group: String,
    pub icon: Option<PathBuf>,
    /// When it was last played (ms since the epoch); 0 if never.
    pub last: i64,
}

/// Where Prism keeps its data and the command that starts it: the installed `prismlauncher`, or
/// the Flatpak.
struct Install {
    data: PathBuf,
    command: Vec<String>,
}

fn install() -> Option<Install> {
    let home = gtk::glib::home_dir();
    let native = gtk::glib::user_data_dir().join("PrismLauncher");
    if native.is_dir() && gtk::glib::find_program_in_path("prismlauncher").is_some() {
        return Some(Install { data: native, command: vec!["prismlauncher".into()] });
    }
    let flatpak = home.join(".var/app").join(FLATPAK).join("data/PrismLauncher");
    if flatpak.is_dir() && gtk::glib::find_program_in_path("flatpak").is_some() {
        return Some(Install { data: flatpak, command: vec!["flatpak".into(), "run".into(), FLATPAK.into()] });
    }
    None
}

/// A value from an INI file's top level (Prism's .cfg files), unquoted.
fn value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (k, v) = line.split_once('=')?;
        (k.trim() == key).then(|| v.trim().trim_matches('"').to_string())
    })
}

fn instances_dir(data: &Path) -> PathBuf {
    let configured = std::fs::read_to_string(data.join("prismlauncher.cfg")).ok().and_then(|t| value(&t, "InstanceDir")).filter(|d| !d.is_empty());
    match configured {
        Some(dir) if Path::new(&dir).is_absolute() => PathBuf::from(dir),
        Some(dir) => data.join(dir),
        None => data.join("instances"),
    }
}

/// Prism's chosen icon (`iconKey`) from its icons folder, or the one the instance keeps.
fn icon(data: &Path, dir: &Path, key: Option<&str>) -> Option<PathBuf> {
    const KINDS: [&str; 7] = ["svg", "png", "ico", "gif", "jpg", "jpeg", "webp"];
    if let Some(key) = key.filter(|k| !k.is_empty() && *k != "default") {
        let found = std::fs::read_dir(data.join("icons")).ok()?.flatten().map(|e| e.path()).find(|p| {
            let stem = p.file_stem().and_then(|s| s.to_str());
            let kind = p.extension().and_then(|e| e.to_str()).map(str::to_lowercase);
            (stem == Some(key) || p.file_name().and_then(|n| n.to_str()) == Some(key)) && kind.is_some_and(|k| KINDS.contains(&k.as_str()))
        });
        if found.is_some() {
            return found;
        }
    }
    let own = dir.join("minecraft/icon.png");
    own.is_file().then_some(own)
}

/// Groups instances were sorted into, from Prism's instgroups.json.
fn groups(dir: &Path) -> Vec<(String, String)> {
    let Ok(text) = std::fs::read_to_string(dir.join("instgroups.json")) else { return Vec::new() };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else { return Vec::new() };
    let Some(groups) = json.get("groups").and_then(serde_json::Value::as_object) else { return Vec::new() };
    groups
        .iter()
        .flat_map(|(group, g)| g.get("instances").and_then(serde_json::Value::as_array).into_iter().flatten().filter_map(|i| i.as_str()).map(move |i| (i.to_string(), group.clone())))
        .collect()
}

/// The instances, most recently played first. Reads files; run it off the main thread.
pub fn instances() -> Vec<Instance> {
    install().map(|i| scan(&i.data)).unwrap_or_default()
}

fn scan(data: &Path) -> Vec<Instance> {
    let dir = instances_dir(data);
    let grouped = groups(&dir);
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut out: Vec<Instance> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            // As Prism does: only folders with an instance.cfg are instances.
            let cfg = std::fs::read_to_string(path.join("instance.cfg")).ok()?;
            let id = e.file_name().to_str()?.to_string();
            let name = value(&cfg, "name").filter(|n| !n.is_empty()).unwrap_or_else(|| id.clone());
            let last = value(&cfg, "lastLaunchTime").and_then(|v| v.parse().ok()).unwrap_or(0);
            let group = grouped.iter().find(|(i, _)| *i == id).map(|(_, g)| g.clone()).unwrap_or_default();
            Some(Instance { icon: icon(data, &path, value(&cfg, "iconKey").as_deref()), id, name, group, last })
        })
        .collect();
    out.sort_by(|a, b| b.last.cmp(&a.last).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    out
}

/// The command that opens Prism itself.
pub fn open() -> Option<Vec<String>> {
    install().map(|i| i.command)
}

/// The command that starts instance `id`.
pub fn launch(id: &str) -> Option<Vec<String>> {
    let mut command = install()?.command;
    command.push("--launch".into());
    command.push(id.into());
    Some(command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ini_values_are_read_from_the_top_level_and_unquoted() {
        let cfg = "[General]\nname=Fabulously Optimized\niconKey=\"fabric\"\nlastLaunchTime=1731500000000\n";
        assert_eq!(value(cfg, "name").as_deref(), Some("Fabulously Optimized"));
        assert_eq!(value(cfg, "iconKey").as_deref(), Some("fabric"));
        assert_eq!(value(cfg, "missing"), None);
    }

    #[test]
    fn instances_are_found_named_grouped_and_sorted_by_last_play() {
        let root = std::env::temp_dir().join(format!("digit-prism-{}", std::process::id()));
        let dir = root.join("instances");
        for (id, cfg) in [("a", "name=Alpha\nlastLaunchTime=10\n"), ("b", "name=Beta\nlastLaunchTime=20\n"), ("c", "lastLaunchTime=0\n")] {
            std::fs::create_dir_all(dir.join(id)).unwrap();
            std::fs::write(dir.join(id).join("instance.cfg"), cfg).unwrap();
        }
        std::fs::create_dir_all(dir.join("not-an-instance")).unwrap();
        std::fs::write(dir.join("instgroups.json"), r#"{"formatVersion":"1","groups":{"Modded":{"hidden":false,"instances":["a"]}}}"#).unwrap();
        assert_eq!(instances_dir(&root), dir);
        let found = scan(&root);
        let names: Vec<&str> = found.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["Beta", "Alpha", "c"]);
        assert_eq!(found[1].group, "Modded");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
