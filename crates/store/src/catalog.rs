use crate::xml::{Event, Reader, attr, unescape};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct App {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub developer: String,
    pub license: String,
    pub categories: Vec<String>,
    pub keywords: Vec<String>,
    pub icon: String,
    pub shots: Vec<String>,
    pub version: String,
    pub released: u64,
    pub homepage: String,
    pub verified: bool,
    pub about: Vec<String>,
    #[serde(default)]
    pub brand: String,
}

#[derive(Serialize, Deserialize)]
struct Cached {
    stamp: String,
    apps: Vec<App>,
}

pub struct Catalog {
    pub apps: Vec<App>,
    index: HashMap<String, usize>,
    folded: Vec<(String, String, String)>,
}

impl Catalog {
    pub fn get(&self, id: &str) -> Option<&App> {
        self.index.get(id).map(|&i| &self.apps[i])
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<&App> {
        let q = kit::fuzzy::fold(query.trim());
        if q.is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<(i32, usize)> = self
            .folded
            .iter()
            .enumerate()
            .filter_map(|(i, (name, summary, keys))| {
                let best = [kit::fuzzy::score(&q, name).map(|s| s + 200), kit::fuzzy::score(&q, keys).map(|s| s - 150), (q.len() > 2).then(|| summary.contains(&q)).flatten_bool().map(|_| 250)].into_iter().flatten().max()?;
                let boost = if self.apps[i].verified { 40 } else { 0 };
                Some((best + boost, i))
            })
            .collect();
        hits.sort_by(|a, b| b.0.cmp(&a.0).then(self.apps[a.1].name.len().cmp(&self.apps[b.1].name.len())));
        hits.into_iter().take(limit).map(|(_, i)| &self.apps[i]).collect()
    }

    fn new(mut apps: Vec<App>) -> Self {
        apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        let index = apps.iter().enumerate().map(|(i, a)| (a.id.clone(), i)).collect();
        let folded = apps.iter().map(|a| (kit::fuzzy::fold(&a.name), kit::fuzzy::fold(&a.summary), kit::fuzzy::fold(&a.keywords.join(" ")))).collect();
        Self { apps, index, folded }
    }
}

trait FlattenBool {
    fn flatten_bool(self) -> Option<()>;
}

impl FlattenBool for Option<bool> {
    fn flatten_bool(self) -> Option<()> {
        self.filter(|b| *b).map(|_| ())
    }
}

fn roots() -> Vec<PathBuf> {
    let home = gtk::glib::home_dir();
    vec![home.join(".local/share/flatpak/appstream/flathub/x86_64/active"), PathBuf::from("/var/lib/flatpak/appstream/flathub/x86_64/active")]
}

fn source() -> Option<(PathBuf, String)> {
    roots()
        .into_iter()
        .filter_map(|r| {
            let meta = std::fs::metadata(r.join("appstream.xml")).ok()?;
            let stamp = format!("v2-{}-{}", meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs(), meta.len());
            Some((meta.modified().ok()?, r, stamp))
        })
        .max_by_key(|x| x.0)
        .map(|(_, r, s)| (r, s))
}

fn cache_file() -> PathBuf {
    gtk::glib::user_cache_dir().join("digit/store/index.json")
}

pub fn load() -> Option<Catalog> {
    let (root, stamp) = source()?;
    if let Ok(text) = std::fs::read_to_string(cache_file()) {
        if let Ok(c) = serde_json::from_str::<Cached>(&text) {
            if c.stamp == stamp {
                return Some(Catalog::new(c.apps));
            }
        }
    }
    let xml = std::fs::read_to_string(root.join("appstream.xml")).ok()?;
    let apps = parse(&xml, &root.join("icons"));
    drop(xml);
    if let Some(dir) = cache_file().parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string(&Cached { stamp, apps: apps.clone() }) {
        let tmp = cache_file().with_extension("new");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(tmp, cache_file());
        }
    }
    Some(Catalog::new(apps))
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(PartialEq)]
enum Grab {
    None,
    Name,
    Summary,
    Developer,
    License,
    Category,
    Keyword,
    Icon(u32),
    Shot(u32),
    Homepage,
    Bundle,
    Verified,
    Para,
    Brand(bool),
}

pub fn parse(xml: &str, icons: &Path) -> Vec<App> {
    let mut out = Vec::new();
    let mut stack: Vec<&str> = Vec::new();
    let mut app: Option<App> = None;
    let mut is_app = false;
    let mut grab = Grab::None;
    let mut buf = String::new();
    let mut icon_size = 0u32;
    let mut shot_best: Option<(u32, String)> = None;
    let mut in_release = false;
    let mut desc_foreign = false;
    let mut released = false;
    for ev in Reader::new(xml) {
        match ev {
            Event::Start { name, attrs, empty } => {
                let parent = stack.last().copied().unwrap_or("");
                let foreign = attr(attrs, "xml:lang").is_some_and(|l| !l.is_empty() && l != "C" && l != "en");
                if name == "component" {
                    let kind = attr(attrs, "type").unwrap_or("");
                    app = matches!(kind, "desktop-application" | "desktop").then(App::default);
                    is_app = false;
                    icon_size = 0;
                    released = false;
                    in_release = false;
                } else if let Some(a) = app.as_mut() {
                    if grab == Grab::Para && matches!(name, "em" | "code" | "strong" | "b" | "i" | "a" | "span") {
                        if !empty {
                            stack.push(name);
                        }
                        continue;
                    }
                    grab = Grab::None;
                    match (name, parent) {
                        ("id", "component") => {
                            buf.clear();
                            grab = Grab::Name;
                            stack.push("id");
                            if empty {
                                stack.pop();
                            }
                            continue;
                        }
                        ("name", "component") if !foreign => grab = Grab::Name,
                        ("summary", "component") if !foreign => grab = Grab::Summary,
                        ("name", "developer") | ("developer_name", "component") if !foreign => grab = Grab::Developer,
                        ("project_license", "component") => grab = Grab::License,
                        ("category", "categories") => grab = Grab::Category,
                        ("keyword", "keywords") if !foreign => grab = Grab::Keyword,
                        ("icon", "component") if attr(attrs, "type") == Some("cached") => {
                            let h: u32 = attr(attrs, "height").and_then(|v| v.parse().ok()).unwrap_or(0);
                            let scale: u32 = attr(attrs, "scale").and_then(|v| v.parse().ok()).unwrap_or(1);
                            grab = Grab::Icon(h * scale);
                        }
                        ("screenshot", "screenshots") => shot_best = None,
                        ("image", "screenshot") => {
                            let w: u32 = attr(attrs, "width").and_then(|v| v.parse().ok()).unwrap_or(0);
                            let thumb = attr(attrs, "type") == Some("thumbnail");
                            grab = Grab::Shot(if thumb && (500..=1300).contains(&w) { 2000 - w.abs_diff(900) } else if thumb { w.min(400) } else { 100 });
                        }
                        ("release", "releases") => {
                            in_release = !empty;
                            if !released {
                                released = true;
                                a.version = attr(attrs, "version").unwrap_or("").to_string();
                                a.released = attr(attrs, "timestamp").and_then(|v| v.parse().ok()).unwrap_or(0);
                            }
                        }
                        ("description", "component") => desc_foreign = foreign,
                        ("url", "component") if attr(attrs, "type") == Some("homepage") => grab = Grab::Homepage,
                        ("bundle", "component") if attr(attrs, "type") == Some("flatpak") => grab = Grab::Bundle,
                        ("value", "custom") if attr(attrs, "key") == Some("flathub::verification::verified") => grab = Grab::Verified,
                        ("color", "branding") if attr(attrs, "type") == Some("primary") => grab = Grab::Brand(attr(attrs, "scheme_preference") == Some("dark")),
                        ("p", "description") | ("li", "ul") | ("li", "ol") if !in_release && !foreign && !desc_foreign && stack.contains(&"description") && !stack.contains(&"release") => {
                            grab = Grab::Para;
                            buf.clear();
                            if name == "li" {
                                buf.push_str("• ");
                            }
                        }
                        _ => {}
                    }
                    if grab != Grab::None && grab != Grab::Para {
                        buf.clear();
                    }
                    let _ = a;
                }
                if !empty {
                    stack.push(name);
                }
            }
            Event::Text(t) => {
                if grab != Grab::None || stack.last() == Some(&"id") {
                    buf.push_str(&unescape(t));
                }
            }
            Event::End(name) => {
                stack.pop();
                if name == "component" {
                    if let Some(a) = app.take() {
                        if is_app && !a.id.is_empty() && !a.name.is_empty() {
                            out.push(a);
                        }
                    }
                    grab = Grab::None;
                    continue;
                }
                if name == "release" {
                    in_release = false;
                }
                let Some(a) = app.as_mut() else { continue };
                if name == "id" && stack.last() == Some(&"component") {
                    if a.id.is_empty() {
                        a.id = squash(&buf);
                    }
                    buf.clear();
                    grab = Grab::None;
                    continue;
                }
                if name == "screenshot" {
                    if let Some((_, url)) = shot_best.take() {
                        if a.shots.len() < 5 && url.starts_with("https://") {
                            a.shots.push(url);
                        }
                    }
                }
                let text = squash(&buf);
                match grab {
                    Grab::Name if name == "name" => a.name = text,
                    Grab::Summary if name == "summary" => a.summary = text,
                    Grab::Developer if name == "name" || name == "developer_name" => {
                        if a.developer.is_empty() {
                            a.developer = text;
                        }
                    }
                    Grab::License if name == "project_license" => a.license = text,
                    Grab::Category if name == "category" => a.categories.push(text),
                    Grab::Keyword if name == "keyword" && a.keywords.len() < 24 => a.keywords.push(text),
                    Grab::Icon(size) if name == "icon" => {
                        let dir = if size >= 128 { "128x128" } else { "64x64" };
                        let path = icons.join(dir).join(&text);
                        if size > icon_size && !text.contains('/') && path.exists() {
                            icon_size = size;
                            a.icon = path.to_string_lossy().to_string();
                        }
                    }
                    Grab::Shot(rank) if name == "image" => {
                        if shot_best.as_ref().is_none_or(|(r, _)| rank > *r) {
                            shot_best = Some((rank, text));
                        }
                    }
                    Grab::Homepage if name == "url" => a.homepage = text,
                    Grab::Bundle if name == "bundle" => {
                        is_app = text.starts_with("app/");
                        if let Some(id) = text.split('/').nth(1).filter(|i| crate::flatpak::valid(i)) {
                            a.id = id.to_string();
                        }
                    }
                    Grab::Verified if name == "value" => a.verified = text == "true",
                    Grab::Brand(dark) if name == "color" => {
                        let ok = text.len() == 7 && text.starts_with('#') && text[1..].chars().all(|c| c.is_ascii_hexdigit());
                        if ok && (dark || a.brand.is_empty()) {
                            a.brand = text.to_lowercase();
                        }
                    }
                    Grab::Para if name == "p" || name == "li" => {
                        if !text.is_empty() && a.about.len() < 24 {
                            a.about.push(text);
                        }
                        buf.clear();
                    }
                    _ => {}
                }
                if !matches!(grab, Grab::Para) || name == "p" || name == "li" {
                    grab = Grab::None;
                    buf.clear();
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn reads_a_component() {
        let xml = r#"<components><component type="desktop-application"><id>org.x.App</id><name xml:lang="de">Ding</name><name>Thing &amp; Co</name>
<summary>Do things</summary><developer><name>Some One</name></developer><description><p>First <em>bold</em> line.</p><ul><li>One</li></ul></description>
<categories><category>Graphics</category></categories><releases><release version="2.1" timestamp="100"><description><p>Notes</p></description></release><release version="2.0" timestamp="50"/></releases>
<screenshots><screenshot type="default"><image type="source" width="1920">https://a/src.png</image><image type="thumbnail" width="752">https://a/t.png</image></screenshot></screenshots>
<branding><color type="primary" scheme_preference="light">#BFDBFE</color><color type="primary" scheme_preference="dark">#1E3A8A</color></branding><bundle type="flatpak">app/org.x.App/x86_64/stable</bundle><custom><value key="flathub::verification::verified">true</value></custom></component>
<component type="runtime"><id>org.rt</id><name>RT</name></component>
<component type="desktop-application"><id>org.y.Two</id><name>Two</name><releases><release version="1" timestamp="1"/></releases><description xml:lang="de"><p>Nach</p></description><description><p>After release</p></description><bundle type="flatpak">app/org.y.Two/x86_64/stable</bundle></component></components>"#;
        let apps = parse(xml, std::path::Path::new("/nonexistent"));
        assert_eq!(apps.len(), 2);
        assert_eq!(apps[1].about, vec!["After release".to_string()]);
        let a = &apps[0];
        assert_eq!(a.id, "org.x.App");
        assert_eq!(a.name, "Thing & Co");
        assert_eq!(a.developer, "Some One");
        assert_eq!(a.about, vec!["First bold line.".to_string(), "• One".to_string()]);
        assert_eq!(a.version, "2.1");
        assert_eq!(a.shots, vec!["https://a/t.png".to_string()]);
        assert!(a.verified);
        assert_eq!(a.brand, "#1e3a8a");
        assert_eq!(a.categories, vec!["Graphics".to_string()]);
    }
}
