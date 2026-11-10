use futures_util::StreamExt;
use std::cell::RefCell;
use zbus::zvariant::{DynamicType, OwnedObjectPath, Value};

const NAME: &str = "org.freedesktop.PackageKit";

pub const FILTER_NONE: u64 = 1 << 1;
pub const FILTER_INSTALLED: u64 = 1 << 2;
pub const INFO_INSTALLED: u32 = 1;
pub const INFO_AVAILABLE: u32 = 2;
pub const ONLY_TRUSTED: u64 = 1 << 1;
pub const SIMULATE: u64 = 1 << 2;
pub const ALLOW_DOWNGRADE: u64 = 1 << 6;

pub const INFO_IMPORTANT: u32 = 7;
pub const INFO_SECURITY: u32 = 8;
pub const INFO_CRITICAL: u32 = 26;

#[derive(Clone, Debug)]
pub struct Pkg {
    pub info: u32,
    pub id: String,
    pub summary: String,
}

impl Pkg {
    fn part(&self, i: usize) -> &str {
        self.id.split(';').nth(i).unwrap_or("")
    }

    pub fn name(&self) -> &str {
        self.part(0)
    }

    pub fn version(&self) -> &str {
        self.part(1)
    }

    pub fn arch(&self) -> &str {
        self.part(2)
    }

    pub fn installed(&self) -> bool {
        self.info == INFO_INSTALLED
    }

    pub fn repo(&self) -> &str {
        let r = self.part(3);
        r.strip_prefix("installed:").unwrap_or(r)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Progress {
    pub percent: Option<u32>,
    pub status: u32,
}

impl Progress {
    pub fn label(&self) -> &'static str {
        match self.status {
            1 => "Waiting",
            2 | 3 => "Working",
            4 | 5 => "Querying",
            6 => "Removing",
            7 | 20 | 21 | 22 | 25 => "Refreshing repositories",
            8 => "Downloading",
            9 => "Installing",
            10 => "Updating",
            11 => "Cleaning up",
            13 => "Resolving dependencies",
            14 => "Checking signatures",
            15 => "Testing changes",
            16 => "Applying changes",
            27 => "Loading cache",
            30 => "Waiting for the package manager",
            31 => "Waiting for authorization",
            _ => "Working",
        }
    }
}

thread_local! {
    static BUS: RefCell<Option<zbus::Connection>> = const { RefCell::new(None) };
}

pub async fn system() -> Result<zbus::Connection, String> {
    if let Some(c) = BUS.with(|b| b.borrow().clone()) {
        return Ok(c);
    }
    let c = zbus::Connection::system().await.map_err(|e| e.to_string())?;
    BUS.with(|b| *b.borrow_mut() = Some(c.clone()));
    Ok(c)
}

fn failure(code: u32, details: &str) -> String {
    match code {
        2 => "No network connection".into(),
        13 => format!("Dependencies could not be resolved: {details}"),
        20 => "This is a system package and cannot be removed".into(),
        26 => "Another package manager is running. Try again when it finishes".into(),
        30 | 31 | 50 | 51 => format!("Signature check failed, nothing was changed: {details}"),
        46 => "Not enough disk space".into(),
        48 => "Authorization was not granted, nothing was changed".into(),
        _ if details.is_empty() => format!("PackageKit error {code}"),
        _ => details.to_string(),
    }
}

enum Event {
    OwnerLost,
    Signal(zbus::Message),
    Props(Option<u32>, Option<u32>),
}

#[derive(Clone, Debug)]
pub struct Repo {
    pub id: String,
    pub description: String,
    pub enabled: bool,
}

pub async fn run<B>(method: &str, body: &B, progress: impl Fn(Progress)) -> Result<Vec<Pkg>, String>
where
    B: serde::Serialize + DynamicType,
{
    full(method, body, progress).await.map(|r| r.0)
}

pub async fn full<B>(method: &str, body: &B, progress: impl Fn(Progress)) -> Result<(Vec<Pkg>, Vec<Repo>), String>
where
    B: serde::Serialize + DynamicType,
{
    let conn = system().await?;
    let dbus = zbus::fdo::DBusProxy::new(&conn).await.map_err(|e| e.to_string())?;
    let owners = dbus.receive_name_owner_changed_with_args(&[(0, NAME)]).await.map_err(|e| e.to_string())?;
    let lost = owners.filter_map(|signal| async move {
        signal.args().ok().filter(|a| a.old_owner().is_some()).map(|_| Event::OwnerLost)
    }).chain(futures_util::stream::once(async { Event::OwnerLost })).boxed_local();
    let pk = zbus::Proxy::new(&conn, NAME, "/org/freedesktop/PackageKit", "org.freedesktop.PackageKit").await.map_err(|e| e.to_string())?;
    let path: OwnedObjectPath = pk.call("CreateTransaction", &()).await.map_err(|e| e.to_string())?;
    let tx = zbus::Proxy::new(&conn, NAME, path.clone().into_inner(), "org.freedesktop.PackageKit.Transaction").await.map_err(|e| e.to_string())?;
    let props = zbus::fdo::PropertiesProxy::builder(&conn).destination(NAME).map_err(|e| e.to_string())?.path(path.into_inner()).map_err(|e| e.to_string())?.build().await.map_err(|e| e.to_string())?;
    let locale = std::env::var("LANG").unwrap_or_else(|_| "C.UTF-8".into());
    let hints = vec![format!("locale={locale}"), "interactive=true".to_string(), "background=false".to_string()];
    tx.call_method("SetHints", &(hints,)).await.map_err(|e| e.to_string())?;
    let signals = tx.receive_all_signals().await.map_err(|e| e.to_string())?.map(Event::Signal);
    let changes = props.receive_properties_changed().await.map_err(|e| e.to_string())?.map(|c| {
        let read = |args: &zbus::fdo::PropertiesChangedArgs, key: &str| args.changed_properties().get(key).and_then(|v| match v {
            Value::U32(n) => Some(*n),
            _ => None,
        });
        match c.args() {
            Ok(a) => Event::Props(read(&a, "Percentage"), read(&a, "Status")),
            Err(_) => Event::Props(None, None),
        }
    });
    let mut events = futures_util::stream::select(futures_util::stream::select(signals, changes), lost);
    tx.call_method(method, body).await.map_err(|e| {
        let text = e.to_string();
        if text.contains("NotAuthorized") || text.contains("not authorized") { "Authorization was not granted, nothing was changed".to_string() } else { text }
    })?;
    let mut found = Vec::new();
    let mut repos = Vec::new();
    let mut error: Option<String> = None;
    let mut now = Progress::default();
    while let Some(ev) = events.next().await {
        match ev {
            Event::OwnerLost => return Err("Package manager disconnected".into()),
            Event::Props(percent, status) => {
                if let Some(p) = percent {
                    now.percent = (p <= 100).then_some(p);
                }
                if let Some(s) = status {
                    now.status = s;
                }
                progress(now);
            }
            Event::Signal(msg) => {
                let header = msg.header();
                let member = header.member().map(|m| m.as_str().to_string()).unwrap_or_default();
                match member.as_str() {
                    "Package" => {
                        if let Ok((info, id, summary)) = msg.body().deserialize::<(u32, String, String)>() {
                            found.push(Pkg { info, id, summary });
                        }
                    }
                    "RepoDetail" => {
                        if let Ok((id, description, enabled)) = msg.body().deserialize::<(String, String, bool)>() {
                            repos.push(Repo { id, description, enabled });
                        }
                    }
                    "ErrorCode" => {
                        if let Ok((code, details)) = msg.body().deserialize::<(u32, String)>() {
                            error = Some(failure(code, &details));
                        }
                    }
                    "Finished" => {
                        let exit = msg.body().deserialize::<(u32, u32)>().map(|b| b.0).unwrap_or(0);
                        return match (exit, error) {
                            (1, _) => Ok((found, repos)),
                            (3, _) => Err("Cancelled, nothing was changed".into()),
                            (_, Some(e)) => Err(e),
                            (_, None) => Err("The package manager reported a failure".into()),
                        };
                    }
                    "Destroy" => break,
                    _ => {}
                }
            }
        }
    }
    Err(error.unwrap_or_else(|| "The transaction ended unexpectedly".into()))
}

pub async fn resolve(names: &[&str], filter: u64) -> Result<Vec<Pkg>, String> {
    let names: Vec<String> = names.iter().map(|s| s.to_string()).collect();
    run("Resolve", &(filter, names), |_| {}).await
}

pub async fn search_names(terms: &[&str], filter: u64) -> Result<Vec<Pkg>, String> {
    let terms: Vec<String> = terms.iter().map(|s| s.to_string()).collect();
    run("SearchNames", &(filter, terms), |_| {}).await
}

pub async fn owner(path: &str) -> Result<Option<Pkg>, String> {
    let found = run("SearchFiles", &(FILTER_INSTALLED, vec![path.to_string()]), |_| {}).await?;
    Ok(found.into_iter().next())
}

pub async fn updates() -> Result<Vec<Pkg>, String> {
    run("GetUpdates", &(FILTER_NONE,), |_| {}).await
}

pub async fn refresh(progress: impl Fn(Progress)) -> Result<(), String> {
    run("RefreshCache", &(false,), progress).await.map(|_| ())
}

pub async fn remove(ids: &[String], simulate: bool, progress: impl Fn(Progress)) -> Result<Vec<Pkg>, String> {
    let flags = if simulate { SIMULATE } else { 0 };
    run("RemovePackages", &(flags, ids.to_vec(), true, false), progress).await
}

pub async fn install(ids: &[String], flags: u64, progress: impl Fn(Progress)) -> Result<Vec<Pkg>, String> {
    run("InstallPackages", &(flags, ids.to_vec()), progress).await
}

pub async fn update(ids: &[String], simulate: bool, progress: impl Fn(Progress)) -> Result<Vec<Pkg>, String> {
    let flags = ONLY_TRUSTED | if simulate { SIMULATE } else { 0 };
    run("UpdatePackages", &(flags, ids.to_vec()), progress).await
}

pub async fn repos() -> Result<Vec<Repo>, String> {
    full("GetRepoList", &(FILTER_NONE,), |_| {}).await.map(|r| r.1)
}

pub async fn repo_enable(id: &str, on: bool) -> Result<(), String> {
    run("RepoEnable", &(id.to_string(), on), |_| {}).await.map(|_| ())
}

pub async fn repo_remove(id: &str) -> Result<(), String> {
    run("RepoRemove", &(0u64, id.to_string(), false), |_| {}).await.map(|_| ())
}

pub fn compare(a: &str, b: &str) -> std::cmp::Ordering {
    let split = |s: &str| -> Vec<u64> { s.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).map(|p| p.parse().unwrap_or(0)).collect() };
    split(a).cmp(&split(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_package_ids_and_versions() {
        let p = Pkg { info: 1, id: "nvidia-gl-G07;595.104.02-11.1;x86_64;installed:NVIDIA".into(), summary: String::new() };
        assert_eq!(p.name(), "nvidia-gl-G07");
        assert_eq!(p.version(), "595.104.02-11.1");
        assert_eq!(p.repo(), "NVIDIA");
        assert_eq!(compare("595.112.07-42.1", "595.104.02-39.1"), std::cmp::Ordering::Greater);
        assert_eq!(compare("595.99.02-36.1", "595.104.02-39.1"), std::cmp::Ordering::Less);
    }
}
