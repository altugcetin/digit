use crate::guard;
use gtk::gio;

#[derive(Clone, Debug)]
pub struct App {
    pub id: String,
    pub name: String,
    pub version: String,
}

pub fn available() -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join("flatpak").is_file()))
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

fn rows(text: &str) -> Vec<Vec<String>> {
    text.lines().filter(|l| !l.trim().is_empty()).map(|l| l.split('\t').map(|c| c.trim().to_string()).collect()).collect()
}

pub async fn installed() -> Result<Vec<App>, String> {
    let text = output(&["list", "--app", "--columns=application,name,version"]).await?;
    Ok(rows(&text)
        .into_iter()
        .filter(|r| !r.is_empty() && guard::flatpak_id(&r[0]))
        .map(|r| App { id: r[0].clone(), name: r.get(1).cloned().unwrap_or_default(), version: r.get(2).cloned().unwrap_or_default() })
        .collect())
}

pub async fn updates() -> Result<Vec<(String, String)>, String> {
    let text = output(&["remote-ls", "--updates", "--app", "--columns=application,version"]).await?;
    Ok(rows(&text).into_iter().filter(|r| !r.is_empty() && guard::flatpak_id(&r[0])).map(|r| (r[0].clone(), r.get(1).cloned().unwrap_or_default())).collect())
}

pub async fn uninstall(id: &str, system: bool) -> Result<(), String> {
    if !guard::flatpak_id(id) {
        return Err("Refused: not a valid Flatpak application id".into());
    }
    let scope = if system { "--system" } else { "--user" };
    output(&["uninstall", "--noninteractive", "-y", scope, id]).await.map(|_| ())
}

pub async fn update_all() -> Result<(), String> {
    output(&["update", "--noninteractive", "-y"]).await.map(|_| ())
}
