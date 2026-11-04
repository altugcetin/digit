//! Local distribution catalog data, shared by Arch and PackageKit results.

use crate::xml::{Event, Reader, attr, unescape};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const ROOTS: &[&str] = &[
    "/usr/share/swcatalog",
    "/var/cache/swcatalog",
    "/var/lib/swcatalog",
    "/usr/share/app-info",
    "/var/cache/app-info",
    "/var/lib/app-info",
];

#[derive(Clone, Default)]
pub struct Component {
    pub kind: String,
    pub name: String,
    pub summary: String,
    pub description: String,
    pub icon: String,
    pub stock: String,
}

pub fn files() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for root in ROOTS {
        for dir in ["xml", "xmls"] {
            let entries = std::fs::read_dir(Path::new(root).join(dir));
            paths.extend(
                entries
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        p.to_str()
                            .is_some_and(|name| name.ends_with(".xml") || name.ends_with(".xml.gz"))
                    }),
            );
        }
    }
    paths.sort();
    paths
}

pub fn load() -> HashMap<String, Component> {
    let mut out = HashMap::new();
    for path in files() {
        let Some(root) = path.parent().and_then(Path::parent) else {
            continue;
        };
        let text = if path.extension().is_some_and(|ext| ext == "gz") {
            Command::new("gzip")
                .arg("-dc")
                .arg(&path)
                .stderr(Stdio::null())
                .output()
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| String::from_utf8(o.stdout).ok())
        } else {
            std::fs::read_to_string(&path).ok()
        };
        if let Some(text) = text {
            components(&text, root, &mut out);
        }
    }
    out
}

fn cached_icon(root: &Path, origin: &str, size: u32, name: &str) -> Option<PathBuf> {
    let safe = |part: &str| {
        matches!(
            Path::new(part).components().collect::<Vec<_>>().as_slice(),
            [std::path::Component::Normal(_)]
        )
    };
    if !safe(origin) || !safe(name) {
        return None;
    }
    let size = if size > 0 { size } else { 64 };
    let size = format!("{size}x{size}");
    std::iter::once(root)
        .chain(ROOTS.iter().map(Path::new))
        .map(|r| r.join("icons").join(origin).join(&size).join(name))
        .find(|p| p.is_file())
}

fn components(xml: &str, root: &Path, out: &mut HashMap<String, Component>) {
    let mut origin = "";
    let mut component = Component::default();
    let mut packages = Vec::new();
    let mut depth = 0;
    let mut grab = None;
    let mut width = 0;
    let mut icon_size = 0;
    let mut description = false;
    let mut translated_depth = None;
    for ev in Reader::new(xml) {
        match ev {
            Event::Start {
                name: "components",
                attrs,
                ..
            } => origin = attr(attrs, "origin").unwrap_or_default(),
            Event::Start {
                name: "component",
                attrs,
                empty: false,
            } => {
                depth = 1;
                component = Component {
                    kind: attr(attrs, "type").unwrap_or_default().into(),
                    ..Default::default()
                };
                packages.clear();
                icon_size = 0;
                description = false;
                translated_depth = None;
            }
            Event::Start {
                name,
                attrs,
                empty: false,
            } if depth > 0 => {
                depth += 1;
                let localized = attr(attrs, "xml:lang").is_some_and(|lang| lang != "C");
                if depth == 2 && name == "description" && !localized {
                    description = true;
                }
                if description && localized && translated_depth.is_none() {
                    translated_depth = Some(depth);
                }
                if depth == 2 {
                    grab = match name {
                        "name" if !localized && component.name.is_empty() => Some("name"),
                        "summary" if !localized && component.summary.is_empty() => Some("summary"),
                        "pkgname" => Some("pkgname"),
                        "icon"
                            if attr(attrs, "type") == Some("stock")
                                && component.stock.is_empty() =>
                        {
                            Some("stock")
                        }
                        "icon" if attr(attrs, "type") == Some("cached") => {
                            width = attr(attrs, "width")
                                .and_then(|w| w.parse().ok())
                                .unwrap_or(0);
                            ((width >= icon_size && width <= 128) || component.icon.is_empty())
                                .then_some("icon")
                        }
                        _ => None,
                    };
                }
            }
            Event::Text(text) if depth > 0 => {
                let text = unescape(text.trim());
                if text.is_empty() {
                    continue;
                }
                match grab.take() {
                    Some("name") => component.name = text.into_owned(),
                    Some("summary") => component.summary = text.into_owned(),
                    Some("pkgname") => packages.push(text.into_owned()),
                    Some("stock") => component.stock = text.into_owned(),
                    Some("icon") => {
                        if let Some(path) = cached_icon(root, origin, width, &text) {
                            component.icon = path.to_string_lossy().into_owned();
                            icon_size = width;
                        }
                    }
                    _ if description && translated_depth.is_none() => {
                        if !component.description.is_empty()
                            && !component.description.ends_with('\n')
                        {
                            component.description.push(' ');
                        }
                        component.description.push_str(&text);
                    }
                    _ => {}
                }
            }
            Event::End("component") if depth > 0 => {
                depth = 0;
                for pkg in packages.drain(..) {
                    let better = out.get(&pkg).is_none_or(|old| {
                        (old.kind != "desktop-application"
                            && component.kind == "desktop-application")
                            || (old.kind == component.kind
                                && old.icon.is_empty()
                                && !component.icon.is_empty())
                    });
                    if better {
                        out.insert(pkg, component.clone());
                    }
                }
            }
            Event::End(name) if depth > 0 => {
                if description
                    && translated_depth.is_none()
                    && matches!(name, "p" | "li")
                    && !component.description.is_empty()
                {
                    component.description.push('\n');
                }
                if translated_depth == Some(depth) {
                    translated_depth = None;
                }
                if depth == 2 {
                    description = false;
                    grab = None;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_native_metadata_without_mixing_translations_or_release_notes() {
        let xml = r#"<components origin="repo"><component type="desktop-application">
            <pkgname>example</pkgname><name>Example</name><summary>A tool</summary>
            <icon type="stock">example-icon</icon>
            <description><p xml:lang="de">Übersetzung</p><p>Open <em>files</em>.</p></description>
            <releases><release><description><p>Release notes</p></description></release></releases>
            </component></components>"#;
        let mut index = HashMap::new();
        components(xml, Path::new("/nonexistent"), &mut index);
        let app = &index["example"];
        assert_eq!(app.summary, "A tool");
        assert_eq!(app.stock, "example-icon");
        assert!(app.description.contains("Open files"));
        assert!(!app.description.contains("Übersetzung"));
        assert!(!app.description.contains("Release notes"));
    }

    #[test]
    fn reads_appstream_package_names_and_types() {
        let xml = r#"<components origin="archlinux-arch-extra"><component type="desktop-application"><id>org.mozilla.firefox</id><name>Firefox</name><name xml:lang="de">Feuerfuchs</name><pkgname>firefox</pkgname><developer><name>Mozilla</name></developer></component><component type="console-application"><id>curl</id><name>curl</name><pkgname>curl</pkgname></component></components>"#;
        let mut out = HashMap::new();
        components(xml, Path::new("/nonexistent"), &mut out);
        assert_eq!(
            out.get("firefox")
                .map(|c| (c.kind.as_str(), c.name.as_str())),
            Some(("desktop-application", "Firefox"))
        );
        assert_eq!(
            out.get("curl").map(|c| c.kind.as_str()),
            Some("console-application")
        );
    }
}
