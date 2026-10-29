use gtk::glib;
use std::path::PathBuf;

pub const QUIET: [&str; 9] = [
    "org.gnome.Software",
    "org.gnome.Settings",
    "org.gnome.Calculator",
    "org.gnome.Seahorse",
    "seahorse",
    "org.gnome.Photos",
    "org.gnome.Contacts",
    "org.gnome.Weather",
    "org.gnome.clocks",
];

pub struct Listed {
    pub id: String,
    pub bus: String,
    pub path: String,
    pub default_on: bool,
}

fn dirs() -> Vec<PathBuf> {
    let mut out = vec![glib::user_data_dir()];
    out.extend(glib::system_data_dirs());
    out.push(PathBuf::from("/var/lib/flatpak/exports/share"));
    out.push(glib::user_data_dir().join("flatpak/exports/share"));
    let mut seen = Vec::new();
    out.retain(|d| {
        let fresh = !seen.contains(d);
        seen.push(d.clone());
        fresh
    });
    out
}

fn bus_ok(name: &str) -> bool {
    let parts: Vec<&str> = name.split('.').collect();
    name.len() <= 255
        && parts.len() >= 2
        && parts.iter().all(|p| !p.is_empty() && !p.starts_with(|c: char| c.is_ascii_digit()) && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'))
}

fn path_ok(path: &str) -> bool {
    path == "/" || (path.starts_with('/') && !path.ends_with('/') && path[1..].split('/').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')))
}

pub fn listed() -> Vec<Listed> {
    let mut out: Vec<Listed> = Vec::new();
    for dir in dirs() {
        let Ok(read) = std::fs::read_dir(dir.join("gnome-shell/search-providers")) else { continue };
        let mut files: Vec<PathBuf> = read.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "ini")).collect();
        files.sort();
        for path in files {
            let kf = glib::KeyFile::new();
            if kf.load_from_file(&path, glib::KeyFileFlags::NONE).is_err() {
                continue;
            }
            let group = "Shell Search Provider";
            let (Ok(desktop), Ok(bus), Ok(object)) = (kf.string(group, "DesktopId"), kf.string(group, "BusName"), kf.string(group, "ObjectPath")) else { continue };
            if kf.integer(group, "Version").unwrap_or(0) != 2 || !bus_ok(&bus) || !path_ok(&object) {
                continue;
            }
            let id = desktop.trim_end_matches(".desktop").to_string();
            if out.iter().any(|l| l.bus == bus.as_str() || l.id == id) {
                continue;
            }
            let default_on = !kf.boolean(group, "DefaultDisabled").unwrap_or(false) && !QUIET.contains(&id.as_str());
            out.push(Listed { id, bus: bus.to_string(), path: object.to_string(), default_on });
        }
    }
    out
}

pub fn off_list(t: &toml::Table) -> Option<Vec<String>> {
    t.get("search_providers_off").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
}

pub fn enabled(l: &Listed, off: &Option<Vec<String>>) -> bool {
    match off {
        Some(off) => !off.contains(&l.id),
        None => l.default_on,
    }
}
