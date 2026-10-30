//! Arch packages in the store: one index built from pacman's sync databases (with their file
//! lists when `pacman -Fy` has run), the local database and Arch's AppStream data.
//!
//! Every package gets a [`Kind`] with a [`Confidence`]. [`classify`] weighs its evidence in
//! this order and stops at the first that settles it:
//!
//! 1. AppStream: a desktop-application component is an Application, a console-application a
//!    CLI Tool, a driver or firmware a System Component.
//! 2. Desktop entries: a visible `.desktop` file next to an executable is an Application.
//! 3. The file list: no real files but dependencies is a Meta-package; firmware or kernel
//!    modules a System Component; executables with section-1 man pages a CLI Tool; shared
//!    libraries a Library; headers, pkg-config or CMake files Development.
//! 4. The description, only when no file list is known: words like "library", "headers" or
//!    "firmware" give a low-confidence guess.
//!
//! A lone executable is not enough to call something a CLI Tool, and anything without real
//! evidence is `Unknown`: the store shows low-confidence guesses as a plain "Package" rather
//! than a confident wrong label. Packages can carry secondary kinds (a CLI Tool that also ships
//! its library). Repository order follows `pacman-conf`, so a package from a higher-priority
//! repository hides its namesake further down, as pacman does.

use crate::xml::{Event, Reader, attr, unescape};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SYNC: &str = "/var/lib/pacman/sync";
const LOCAL: &str = "/var/lib/pacman/local";
const SWCATALOG: &str = "/usr/share/swcatalog";

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Hash, Default)]
pub enum Kind {
    Application,
    Cli,
    Library,
    Development,
    System,
    Meta,
    #[default]
    Unknown,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Application => "Application",
            Kind::Cli => "CLI Tool",
            Kind::Library => "Library",
            Kind::Development => "Development",
            Kind::System => "System Component",
            Kind::Meta => "Meta-package",
            Kind::Unknown => "Package",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Kind::Application => "apps",
            Kind::Cli => "terminal",
            Kind::Library => "library_books",
            Kind::Development => "code",
            Kind::System => "memory",
            Kind::Meta => "inventory_2",
            Kind::Unknown => "package_2",
        }
    }

    /// How far up a search result moves; "Apps & Tools" pushes the non-user-facing kinds down harder.
    fn weight(self, apps_only: bool) -> i32 {
        match (self, apps_only) {
            (Kind::Application, _) => 600,
            (Kind::Cli, _) => 400,
            (Kind::Unknown, _) => 0,
            (Kind::Library | Kind::Development, false) => -250,
            (Kind::System | Kind::Meta, false) => -400,
            (Kind::Library | Kind::Development, true) => -900,
            (Kind::System | Kind::Meta, true) => -1100,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub enum Confidence {
    #[default]
    Low,
    Medium,
    High,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub desc: String,
    pub repo: String,
    pub url: String,
    pub kind: Kind,
    pub also: Vec<Kind>,
    pub confidence: Confidence,
    /// The AppStream name, when it says more than the package name.
    pub title: String,
    /// AppStream's cached icon; Arch ships these as JPEG XL, which the store converts to show.
    pub icon: String,
    /// The icon-theme name AppStream gives, shown while (or instead of) the cached icon.
    #[serde(default)]
    pub stock: String,
}

impl Package {
    /// The kind to show; guesses without real evidence read as a plain "Package".
    pub fn shown(&self) -> Kind {
        if self.confidence >= Confidence::Medium { self.kind } else { Kind::Unknown }
    }
}

/// What a package's file list says, reduced to the facts classification uses.
#[derive(Default, Debug, Clone, PartialEq)]
pub struct Files {
    pub bins: usize,
    pub man1: bool,
    pub libs: usize,
    pub headers: bool,
    pub devel: bool,
    pub firmware: bool,
    pub modules: bool,
    pub units: bool,
    pub desktop: usize,
    /// Files that are not documentation or licences.
    pub content: usize,
}

impl Files {
    pub fn add(&mut self, path: &str) {
        let path = path.trim_start_matches('/');
        if path.is_empty() || path.ends_with('/') {
            return;
        }
        if !path.starts_with("usr/share/doc/") && !path.starts_with("usr/share/licenses/") {
            self.content += 1;
        }
        let file = path.rsplit('/').next().unwrap_or(path);
        if let Some(rest) = path.strip_prefix("usr/bin/") {
            if !rest.contains('/') {
                self.bins += 1;
            }
        } else if path.starts_with("usr/share/man/man1/") {
            self.man1 = true;
        } else if path.starts_with("usr/include/") {
            self.headers = true;
        } else if path.starts_with("usr/lib/firmware/") {
            self.firmware = true;
        } else if path.starts_with("usr/lib/modules/") {
            self.modules = true;
        } else if path.starts_with("usr/lib/systemd/system/") || path.starts_with("usr/lib/udev/rules.d/") {
            self.units = true;
        } else if path.starts_with("usr/share/applications/") && file.ends_with(".desktop") {
            self.desktop += 1;
        } else if path.starts_with("usr/lib/pkgconfig/") || path.starts_with("usr/share/pkgconfig/") || path.starts_with("usr/lib/cmake/") {
            self.devel = true;
        } else if let Some(rest) = path.strip_prefix("usr/lib/") {
            if !rest.contains('/') && file.starts_with("lib") {
                if file.contains(".so") {
                    self.libs += 1;
                } else if file.ends_with(".a") {
                    self.devel = true;
                }
            }
        }
    }
}

/// Everything known about one package before it is classified.
#[derive(Default, Debug)]
pub struct Signals {
    pub name: String,
    pub desc: String,
    pub depends: usize,
    /// The AppStream component type, when the package has a component.
    pub appstream: Option<String>,
    pub files: Option<Files>,
    /// For installed packages: false when every desktop entry is hidden.
    pub entries_shown: Option<bool>,
}

fn words(desc: &str, list: &[&str]) -> bool {
    let d = desc.to_lowercase();
    list.iter().any(|w| d.contains(w))
}

const CLI_WORDS: [&str; 4] = ["command line", "command-line", "commandline", "cli "];
const LIB_WORDS: [&str; 6] = ["library", "libraries", "toolkit", "framework", "bindings", "module for"];

pub fn classify(s: &Signals) -> (Kind, Vec<Kind>, Confidence) {
    use Confidence::*;
    let mut also = Vec::new();
    if let Some(f) = &s.files {
        if f.libs > 0 {
            also.push(Kind::Library);
        }
        if f.headers || f.devel {
            also.push(Kind::Development);
        }
    }
    let primary = |k: Kind, also: Vec<Kind>, c| (k, also.into_iter().filter(|a| *a != k).collect(), c);
    match s.appstream.as_deref() {
        Some("desktop-application" | "desktop") => return primary(Kind::Application, also, High),
        Some("console-application") => return primary(Kind::Cli, also, High),
        Some("driver" | "firmware") => return primary(Kind::System, also, High),
        _ => {}
    }
    let Some(f) = &s.files else {
        // No file list: only the description and name are left, so every answer is a guess.
        let n = s.name.as_str();
        let kind = if n.ends_with("-headers") || words(&s.desc, &["header files", "headers for", "development files"]) {
            Kind::Development
        } else if words(&s.desc, &["firmware", "driver"]) {
            Kind::System
        } else if words(&s.desc, &["meta package", "metapackage", "meta-package"]) && s.depends > 0 {
            Kind::Meta
        } else if words(&s.desc, &CLI_WORDS) {
            Kind::Cli
        } else if n.starts_with("lib") || ["python-", "perl-", "ruby-", "lua-", "haskell-", "nodejs-", "rust-"].iter().any(|p| n.starts_with(p)) || words(&s.desc, &LIB_WORDS) {
            Kind::Library
        } else {
            Kind::Unknown
        };
        return (kind, Vec::new(), Low);
    };
    if f.desktop > 0 && s.entries_shown != Some(false) {
        return primary(Kind::Application, also, if f.bins > 0 { High } else { Medium });
    }
    if f.content == 0 {
        return if s.depends > 0 { primary(Kind::Meta, also, High) } else { (Kind::Unknown, also, Low) };
    }
    if (f.firmware || f.modules) && f.bins == 0 {
        return primary(Kind::System, also, High);
    }
    if f.bins > 0 && f.man1 {
        // A command with its library: the description decides which one the package is for.
        if f.libs > 0 && words(&s.desc, &LIB_WORDS) && !words(&s.desc, &CLI_WORDS) {
            let mut rest = also;
            rest.push(Kind::Cli);
            return primary(Kind::Library, rest, High);
        }
        return primary(Kind::Cli, also, High);
    }
    if f.libs > 0 {
        return primary(Kind::Library, also, High);
    }
    if f.headers || f.devel {
        return primary(Kind::Development, also, High);
    }
    if f.firmware || f.modules || f.units {
        return primary(Kind::System, also, Medium);
    }
    if f.bins > 0 {
        return primary(Kind::Cli, also, Low);
    }
    (Kind::Unknown, also, Low)
}

/// One `%SECTION%` record from a pacman database stream.
#[derive(Default)]
struct Record {
    name: String,
    version: String,
    desc: String,
    url: String,
    depends: usize,
    files: Option<Files>,
}

/// Reads `desc` (and `files`, for a `.files` database) entries from `bsdtar -xO` output.
/// A package's desc starts with `%FILENAME%` and its file list with `%FILES%`; the two come
/// together but in either order (Arch writes desc first, CachyOS files first), so a record
/// ends when the section it is about to start again has already been seen.
fn records(input: impl BufRead, mut each: impl FnMut(Record)) {
    let mut cur = Record::default();
    let mut section = String::new();
    let mut described = false;
    for line in input.lines() {
        let Ok(line) = line else { break };
        if line.starts_with('%') && line.ends_with('%') && line.len() > 2 {
            let again = (line == "%FILENAME%" && described) || (line == "%FILES%" && cur.files.is_some());
            if again {
                if !cur.name.is_empty() {
                    each(std::mem::take(&mut cur));
                } else {
                    cur = Record::default();
                }
                described = false;
            }
            if line == "%FILENAME%" {
                described = true;
            }
            if line == "%FILES%" {
                cur.files = Some(Files::default());
            }
            section = line;
            continue;
        }
        if line.is_empty() {
            continue;
        }
        match section.as_str() {
            "%NAME%" => cur.name = line,
            "%VERSION%" => cur.version = line,
            "%DESC%" => cur.desc = line,
            "%URL%" => cur.url = line,
            "%DEPENDS%" => cur.depends += 1,
            "%FILES%" => {
                if let Some(f) = cur.files.as_mut() {
                    f.add(&line);
                }
            }
            _ => {}
        }
    }
    if !cur.name.is_empty() {
        each(cur);
    }
}

fn repos() -> Vec<String> {
    Command::new("pacman-conf")
        .arg("--repo-list")
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect())
        .filter(|v: &Vec<String>| !v.is_empty())
        .unwrap_or_else(|| ["core", "extra", "multilib"].map(String::from).to_vec())
}

/// The database to read for a repository: its file list when synced, else the plain one.
fn database(repo: &str) -> Option<PathBuf> {
    let files = Path::new(SYNC).join(format!("{repo}.files"));
    let db = Path::new(SYNC).join(format!("{repo}.db"));
    [files, db].into_iter().find(|p| p.exists())
}

pub fn available() -> bool {
    Path::new(SYNC).is_dir() && gtk::glib::find_program_in_path("bsdtar").is_some()
}

fn stamp_of(path: &Path) -> String {
    std::fs::metadata(path).ok().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or_else(|| "0".into(), |d| d.as_secs().to_string())
}

fn appstream_files() -> Vec<PathBuf> {
    std::fs::read_dir(Path::new(SWCATALOG).join("xml"))
        .map(|d| d.flatten().map(|e| e.path()).filter(|p| p.to_string_lossy().ends_with(".xml.gz") || p.extension().is_some_and(|e| e == "xml")).collect())
        .unwrap_or_default()
}

/// AppStream facts per package name: component type, display name and cached icon.
struct Component {
    kind: String,
    name: String,
    icon: String,
    stock: String,
}

fn appstream() -> HashMap<String, Component> {
    let mut out = HashMap::new();
    for path in appstream_files() {
        let text = if path.to_string_lossy().ends_with(".gz") {
            Command::new("gzip").arg("-dc").arg(&path).stderr(Stdio::null()).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        } else {
            std::fs::read_to_string(&path).ok()
        };
        if let Some(text) = text {
            components(&text, &mut out);
        }
    }
    out
}

fn components(xml: &str, out: &mut HashMap<String, Component>) {
    let mut origin = String::new();
    let (mut kind, mut name, mut pkg, mut icon, mut stock) = (String::new(), String::new(), String::new(), String::new(), String::new());
    let mut depth = 0usize;
    let mut grab: Option<&str> = None;
    let mut icon_size = 0u32;
    for ev in Reader::new(xml) {
        match ev {
            Event::Start { name: "components", attrs, .. } => origin = attr(attrs, "origin").unwrap_or_default().to_string(),
            Event::Start { name: "component", attrs, empty } => {
                if !empty {
                    depth = 1;
                    kind = attr(attrs, "type").unwrap_or_default().to_string();
                    name.clear();
                    pkg.clear();
                    icon.clear();
                    stock.clear();
                    icon_size = 0;
                }
            }
            Event::Start { name: tag, attrs, empty } if depth > 0 => {
                if !empty {
                    depth += 1;
                }
                // Only the component's own <name> (depth 2), not names inside releases or developers.
                grab = match tag {
                    "name" if depth == 2 && attr(attrs, "xml:lang").is_none() && name.is_empty() => Some("name"),
                    "pkgname" if depth == 2 => Some("pkgname"),
                    "icon" if depth == 2 && attr(attrs, "type") == Some("stock") && stock.is_empty() => Some("stock"),
                    "icon" if depth == 2 && attr(attrs, "type") == Some("cached") => {
                        let w = attr(attrs, "width").and_then(|w| w.parse().ok()).unwrap_or(0);
                        if (w >= icon_size && w <= 128) || icon.is_empty() {
                            icon_size = w;
                            Some("icon")
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if empty {
                    grab = None;
                }
            }
            Event::Text(t) if grab.is_some() => {
                let t = unescape(t.trim()).into_owned();
                match grab {
                    Some("name") => name = t,
                    Some("pkgname") => pkg = t,
                    Some("stock") => stock = t,
                    Some("icon") => {
                        let size = if icon_size > 0 { format!("{icon_size}x{icon_size}") } else { "64x64".into() };
                        let p = Path::new(SWCATALOG).join("icons").join(&origin).join(size).join(&t);
                        if p.exists() {
                            icon = p.to_string_lossy().into_owned();
                        }
                    }
                    _ => {}
                }
                grab = None;
            }
            Event::End("component") => {
                depth = 0;
                if !pkg.is_empty() {
                    let c = Component { kind: kind.clone(), name: name.clone(), icon: icon.clone(), stock: stock.clone() };
                    // A package can have several components; a desktop application describes it best.
                    let better = out.get(&pkg).is_none_or(|old: &Component| old.kind != "desktop-application" && c.kind == "desktop-application");
                    if better {
                        out.insert(pkg.clone(), c);
                    }
                }
            }
            Event::End(_) if depth > 0 => {
                depth -= 1;
                grab = None;
            }
            _ => {}
        }
    }
}

/// Installed packages and their versions, from the local database.
pub fn installed() -> HashMap<String, String> {
    let mut out = HashMap::new();
    for entry in std::fs::read_dir(LOCAL).into_iter().flatten().flatten() {
        let Ok(text) = std::fs::read_to_string(entry.path().join("desc")) else { continue };
        records(BufReader::new(text.as_bytes()), |r| {
            out.insert(r.name, r.version);
        });
    }
    out
}

/// File lists of installed packages, and whether their desktop entries are shown.
fn local_files() -> HashMap<String, (Files, Option<bool>)> {
    let mut out = HashMap::new();
    for entry in std::fs::read_dir(LOCAL).into_iter().flatten().flatten() {
        let dir = entry.path();
        let Ok(desc) = std::fs::read_to_string(dir.join("desc")) else { continue };
        let mut name = String::new();
        records(BufReader::new(desc.as_bytes()), |r| name = r.name);
        let Ok(list) = std::fs::read_to_string(dir.join("files")) else { continue };
        let mut files = Files::default();
        let mut entries = Vec::new();
        for line in list.lines().skip_while(|l| *l != "%FILES%").skip(1).take_while(|l| !l.is_empty()) {
            files.add(line);
            if line.starts_with("usr/share/applications/") && line.ends_with(".desktop") {
                entries.push(format!("/{line}"));
            }
        }
        let shown = (!entries.is_empty()).then(|| entries.iter().any(|e| entry_shown(Path::new(e))));
        out.insert(name, (files, shown));
    }
    out
}

fn entry_shown(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else { return true };
    let mut in_entry = false;
    let mut app = true;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        match line.split_once('=').map(|(k, v)| (k.trim(), v.trim())) {
            Some(("NoDisplay" | "Hidden", "true")) => return false,
            Some(("Type", t)) => app = t == "Application",
            _ => {}
        }
    }
    app
}

#[derive(Serialize, Deserialize)]
struct Cached {
    stamp: String,
    packages: Vec<Package>,
}

fn cache_file() -> PathBuf {
    gtk::glib::user_cache_dir().join("digit/store/arch.json")
}

pub struct Index {
    pub packages: Vec<Package>,
    folded: Vec<(String, String, String)>,
    /// Whether file lists were available; without them classification is mostly guesses.
    pub detailed: bool,
}

impl Index {
    fn new(packages: Vec<Package>, detailed: bool) -> Self {
        let folded = packages.iter().map(|p| (kit::fuzzy::fold(&p.name), kit::fuzzy::fold(&p.title), kit::fuzzy::fold(&p.desc))).collect();
        Self { packages, folded, detailed }
    }

    /// Exact names first, then applications and tools, then description matches, with
    /// libraries, system parts and meta-packages pushed down (harder in "Apps & Tools").
    /// In "Apps & Tools" only what is known to be an application or a command-line tool shows.
    pub fn search(&self, query: &str, apps_only: bool, limit: usize) -> Vec<&Package> {
        let q = kit::fuzzy::fold(query.trim());
        if q.is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<(i32, usize)> = self
            .folded
            .iter()
            .enumerate()
            .filter_map(|(i, (name, title, desc))| {
                let exact = (*name == q).then_some(10_000);
                let prefix = name.starts_with(&q).then_some(1_200);
                let fuzzy = [kit::fuzzy::score(&q, name), kit::fuzzy::score(&q, title).map(|s| s - 50)].into_iter().flatten().max();
                let described = (q.len() > 2 && desc.contains(&q)).then_some(150);
                let best = [exact, prefix, fuzzy, described].into_iter().flatten().max()?;
                let p = &self.packages[i];
                let kind = p.shown();
                if apps_only && !matches!(kind, Kind::Application | Kind::Cli) {
                    return None;
                }
                Some((best + kind.weight(apps_only), i))
            })
            .collect();
        hits.sort_by(|a, b| b.0.cmp(&a.0).then(self.packages[a.1].name.len().cmp(&self.packages[b.1].name.len())));
        hits.into_iter().take(limit).map(|(_, i)| &self.packages[i]).collect()
    }
}

/// Builds the index, reusing the cached one while the databases and AppStream data are unchanged.
pub fn load() -> Option<Index> {
    if !available() {
        return None;
    }
    let repos = repos();
    let dbs: Vec<(String, PathBuf)> = repos.iter().filter_map(|r| database(r).map(|p| (r.clone(), p))).collect();
    if dbs.is_empty() {
        return None;
    }
    let detailed = dbs.iter().any(|(_, p)| p.extension().is_some_and(|e| e == "files"));
    let mut stamp = String::from("v3");
    for (_, p) in &dbs {
        stamp.push_str(&format!("|{}:{}", p.display(), stamp_of(p)));
    }
    for p in appstream_files() {
        stamp.push_str(&format!("|{}:{}", p.display(), stamp_of(&p)));
    }
    stamp.push_str(&format!("|local:{}", stamp_of(Path::new(LOCAL))));
    if let Ok(text) = std::fs::read_to_string(cache_file()) {
        if let Ok(c) = serde_json::from_str::<Cached>(&text) {
            if c.stamp == stamp {
                return Some(Index::new(c.packages, detailed));
            }
        }
    }
    let components = appstream();
    let local = local_files();
    let mut seen = HashSet::new();
    let mut packages = Vec::new();
    for (repo, path) in &dbs {
        let Ok(mut child) = Command::new("bsdtar").arg("-xOf").arg(path).stdout(Stdio::piped()).stderr(Stdio::null()).spawn() else { continue };
        let Some(out) = child.stdout.take() else { continue };
        records(BufReader::with_capacity(1 << 16, out), |r| {
            if !seen.insert(r.name.clone()) {
                return;
            }
            let comp = components.get(&r.name);
            let (files, shown) = match (r.files, local.get(&r.name)) {
                (Some(f), l) => (Some(f), l.and_then(|l| l.1)),
                (None, Some((f, s))) => (Some(f.clone()), *s),
                (None, None) => (None, None),
            };
            let signals = Signals { name: r.name.clone(), desc: r.desc.clone(), depends: r.depends, appstream: comp.map(|c| c.kind.clone()), files, entries_shown: shown };
            let (kind, also, confidence) = classify(&signals);
            let title = comp.map(|c| c.name.clone()).filter(|t| !t.eq_ignore_ascii_case(&r.name)).unwrap_or_default();
            packages.push(Package { name: r.name, version: r.version, desc: r.desc, repo: repo.clone(), url: r.url, kind, also, confidence, title, icon: comp.map(|c| c.icon.clone()).unwrap_or_default(), stock: comp.map(|c| c.stock.clone()).unwrap_or_default() });
        });
        let _ = child.wait();
    }
    if let Some(dir) = cache_file().parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string(&Cached { stamp, packages: packages.clone() }) {
        let tmp = cache_file().with_extension("new");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(tmp, cache_file());
        }
    }
    Some(Index::new(packages, detailed))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(paths: &[&str]) -> Files {
        let mut f = Files::default();
        for p in paths {
            f.add(p);
        }
        f
    }

    fn sig(name: &str, desc: &str, paths: Option<&[&str]>) -> Signals {
        Signals { name: name.into(), desc: desc.into(), depends: 3, appstream: None, files: paths.map(files), entries_shown: None }
    }

    #[test]
    fn appstream_desktop_component_is_an_application() {
        let mut s = sig("firefox", "Fast, Private & Safe Web Browser", None);
        s.appstream = Some("desktop-application".into());
        assert_eq!(classify(&s).0, Kind::Application);
        assert_eq!(classify(&s).2, Confidence::High);
    }

    #[test]
    fn desktop_entry_with_executable_is_an_application() {
        let s = sig("kate", "Advanced text editor", Some(&["usr/bin/kate", "usr/share/applications/org.kde.kate.desktop"]));
        assert_eq!(classify(&s), (Kind::Application, vec![], Confidence::High));
    }

    #[test]
    fn hidden_desktop_entries_do_not_make_an_application() {
        let mut s = sig("helper", "A helper", Some(&["usr/bin/helper", "usr/share/man/man1/helper.1.gz", "usr/share/applications/helper.desktop"]));
        s.entries_shown = Some(false);
        assert_eq!(classify(&s).0, Kind::Cli);
    }

    #[test]
    fn command_with_its_library_follows_the_description() {
        let paths: &[&str] = &["usr/bin/curl", "usr/share/man/man1/curl.1.gz", "usr/lib/libcurl.so.4", "usr/include/curl/curl.h"];
        let curl = sig("curl", "command line tool and library for transferring data with URLs", Some(paths));
        assert_eq!(classify(&curl), (Kind::Cli, vec![Kind::Library, Kind::Development], Confidence::High));
        let gtk = sig("gtk4", "GObject-based multi-platform GUI toolkit", Some(&["usr/bin/gtk4-builder-tool", "usr/share/man/man1/gtk4-builder-tool.1.gz", "usr/lib/libgtk-4.so.1", "usr/include/gtk-4.0/gtk/gtk.h"]));
        assert_eq!(classify(&gtk).0, Kind::Library);
        assert!(classify(&gtk).1.contains(&Kind::Cli));
    }

    #[test]
    fn libraries_headers_firmware_and_meta() {
        assert_eq!(classify(&sig("libfoo", "", Some(&["usr/lib/libfoo.so.1", "usr/lib/pkgconfig/foo.pc"]))).0, Kind::Library);
        assert_eq!(classify(&sig("foo-headers", "", Some(&["usr/include/foo.h"]))).0, Kind::Development);
        assert_eq!(classify(&sig("linux-firmware", "Firmware files for Linux", Some(&["usr/lib/firmware/amd/a.bin"]))).0, Kind::System);
        assert_eq!(classify(&sig("base-devel", "Basic tools to build Arch Linux packages", Some(&["usr/share/licenses/base-devel/LICENSE"]))).0, Kind::Meta);
    }

    #[test]
    fn a_lone_executable_is_only_a_guess() {
        let s = sig("thing", "Does a thing", Some(&["usr/bin/thing"]));
        assert_eq!(classify(&s).2, Confidence::Low);
        assert_eq!(Package { kind: Kind::Cli, confidence: Confidence::Low, ..Default::default() }.shown(), Kind::Unknown);
    }

    #[test]
    fn without_files_only_low_confidence_guesses() {
        assert_eq!(classify(&sig("libbar", "Bar parsing library", None)), (Kind::Library, vec![], Confidence::Low));
        assert_eq!(classify(&sig("mystery", "Something", None)).0, Kind::Unknown);
    }

    #[test]
    fn reads_records_from_a_files_database_stream() {
        let text = "%FILENAME%\na-1-1-x86_64.pkg.tar.zst\n\n%NAME%\na\n\n%VERSION%\n1-1\n\n%DESC%\nThe a\n\n%DEPENDS%\nx\ny\n\n%FILES%\nusr/\nusr/bin/\nusr/bin/a\n\n%FILENAME%\nb-2-1-any.pkg.tar.zst\n\n%NAME%\nb\n\n%VERSION%\n2-1\n\n";
        let mut got = Vec::new();
        records(BufReader::new(text.as_bytes()), |r| got.push(r));
        assert_eq!(got.len(), 2);
        assert_eq!((got[0].name.as_str(), got[0].version.as_str(), got[0].depends), ("a", "1-1", 2));
        assert_eq!(got[0].files.as_ref().map(|f| f.bins), Some(1));
        assert!(got[1].files.is_none());
    }

    #[test]
    fn reads_file_lists_that_come_before_their_desc() {
        let text = "%FILES%\nusr/lib/libc.so.6\n\n%FILENAME%\nglibc-2-1.pkg\n\n%NAME%\nglibc\n\n%FILES%\nusr/bin/x\n\n%FILENAME%\nx-1-1.pkg\n\n%NAME%\nx\n\n";
        let mut got = Vec::new();
        records(BufReader::new(text.as_bytes()), |r| got.push(r));
        assert_eq!(got.len(), 2);
        assert_eq!((got[0].name.as_str(), got[0].files.as_ref().map(|f| f.libs)), ("glibc", Some(1)));
        assert_eq!((got[1].name.as_str(), got[1].files.as_ref().map(|f| f.bins)), ("x", Some(1)));
    }

    #[test]
    fn reads_appstream_package_names_and_types() {
        let xml = r#"<components origin="archlinux-arch-extra"><component type="desktop-application"><id>org.mozilla.firefox</id><name>Firefox</name><name xml:lang="de">Feuerfuchs</name><pkgname>firefox</pkgname><developer><name>Mozilla</name></developer></component><component type="console-application"><id>curl</id><name>curl</name><pkgname>curl</pkgname></component></components>"#;
        let mut out = HashMap::new();
        components(xml, &mut out);
        assert_eq!(out.get("firefox").map(|c| (c.kind.as_str(), c.name.as_str())), Some(("desktop-application", "Firefox")));
        assert_eq!(out.get("curl").map(|c| c.kind.as_str()), Some("console-application"));
    }
}

#[cfg(test)]
mod real {
    /// Prints how this machine's packages classify: `cargo test -p digit-store real -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn print_sample() {
        let Some(index) = super::load() else { return };
        println!("detailed: {}, packages: {}", index.detailed, index.packages.len());
        for name in ["firefox", "kate", "curl", "git", "gtk4", "qt6-base", "linux-firmware", "base-devel", "python", "htop", "mpv", "openssh", "zen-browser", "discord", "obs-studio", "ffmpeg", "glibc", "noto-fonts", "networkmanager", "pipewire"] {
            if let Some(p) = index.packages.iter().find(|p| p.name == name) {
                println!("{:16} {:12} {:?} {:?} also={:?}", p.name, p.repo, p.kind, p.confidence, p.also);
            }
        }
        let t = std::time::Instant::now();
        let hits: Vec<&str> = index.search("firefox", true, 6).iter().map(|p| p.name.as_str()).collect();
        println!("search firefox: {hits:?} in {:?}", t.elapsed());
        let hits: Vec<&str> = index.search("gtk", false, 6).iter().map(|p| p.name.as_str()).collect();
        println!("search gtk (all): {hits:?}");
        let hits: Vec<&str> = index.search("video editor", true, 6).iter().map(|p| p.name.as_str()).collect();
        println!("search video editor: {hits:?}");
    }
}
