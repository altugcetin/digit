use gtk::gdk;
use std::collections::HashMap;

pub fn release() -> HashMap<String, String> {
    let text = std::fs::read_to_string("/etc/os-release").or_else(|_| std::fs::read_to_string("/usr/lib/os-release")).unwrap_or_default();
    text.lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().trim_matches('"').to_string()))
        .collect()
}

pub fn name() -> String {
    let os = release();
    os.get("NAME").or(os.get("PRETTY_NAME")).cloned().unwrap_or_else(|| "Linux".into())
}

fn names(symbolic: bool) -> Vec<String> {
    let os = release();
    let mut bases = Vec::new();
    if let Some(l) = os.get("LOGO").filter(|l| !l.is_empty() && !l.contains('/')) {
        bases.push(l.clone());
    }
    if let Some(id) = os.get("ID").filter(|i| !i.is_empty()) {
        bases.push(format!("distributor-logo-{id}"));
        if let Some(first) = id.split('-').next().filter(|f| *f != id) {
            bases.push(format!("distributor-logo-{first}"));
        }
    }
    for like in os.get("ID_LIKE").map(|l| l.split_whitespace().map(str::to_string).collect::<Vec<_>>()).unwrap_or_default() {
        bases.push(format!("distributor-logo-{like}"));
    }
    bases.push("distributor-logo".into());
    let mut out = Vec::new();
    for b in &bases {
        if symbolic {
            out.push(format!("{b}-symbolic"));
        }
        out.push(b.clone());
    }
    out
}

pub fn logo_name(symbolic: bool) -> Option<String> {
    let display = gdk::Display::default()?;
    let theme = gtk::IconTheme::for_display(&display);
    names(symbolic).into_iter().find(|n| theme.has_icon(n))
}

pub fn logo(px: i32, scale: i32, symbolic: bool) -> Option<gtk::IconPaintable> {
    let display = gdk::Display::default()?;
    let theme = gtk::IconTheme::for_display(&display);
    let found = names(symbolic).into_iter().find(|n| theme.has_icon(n))?;
    Some(theme.lookup_icon(&found, &[], px, scale, gtk::TextDirection::Ltr, gtk::IconLookupFlags::empty()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QtPlatform { Missing, Qt6ct, Kde }
impl QtPlatform {
    pub fn name(self) -> &'static str { match self { Self::Missing => "missing", Self::Qt6ct => "qt6ct", Self::Kde => "qt6ct-kde" } }
}

/// Package probes and PATH scans belong on a worker. Cache for the process lifetime.
pub fn qt_platform() -> QtPlatform {
    static FOUND: std::sync::OnceLock<QtPlatform> = std::sync::OnceLock::new();
    *FOUND.get_or_init(|| {
        let os = release();
        let family = format!("{} {}", os.get("ID").map_or("", String::as_str), os.get("ID_LIKE").map_or("", String::as_str));
        let installed = |package: &str| {
            let (cmd, args) = if family.split_whitespace().any(|id| matches!(id, "arch" | "manjaro")) { ("pacman", vec!["-Qq", package]) }
                else if family.split_whitespace().any(|id| matches!(id, "debian" | "ubuntu")) { ("dpkg-query", vec!["-W", "-f=${Status}", package]) }
                else { ("rpm", vec!["-q", package]) };
            std::process::Command::new(cmd).args(args).output().is_ok_and(|o| o.status.success() && (cmd != "dpkg-query" || String::from_utf8_lossy(&o.stdout).trim() == "install ok installed"))
        };
        if installed("qt6ct-kde") { QtPlatform::Kde }
        else if installed("qt6ct") || std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|p| {
            use std::os::unix::fs::PermissionsExt;
            std::fs::metadata(p.join("qt6ct")).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })) { QtPlatform::Qt6ct }
        else { QtPlatform::Missing }
    })
}
