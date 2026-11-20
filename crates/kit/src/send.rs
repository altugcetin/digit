use gtk::{gio, glib, prelude::*};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, UdpSocket};
use std::os::fd::FromRawFd;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 167);
const PORT: u16 = 53317;

#[derive(Clone, PartialEq, Debug)]
pub enum Via {
    LocalSend { ip: String, port: u16, https: bool },
    KdeConnect { id: String },
}

#[derive(Clone, Debug)]
pub struct Peer {
    pub name: String,
    pub kind: String,
    pub via: Via,
}

impl Peer {
    pub fn icon(&self) -> &'static str {
        match self.kind.as_str() {
            "mobile" | "phone" | "smartphone" => "smartphone",
            "tablet" => "tablet",
            "web" => "language",
            _ => "computer",
        }
    }

    pub fn service(&self) -> &'static str {
        match self.via {
            Via::LocalSend { .. } => "LocalSend",
            Via::KdeConnect { .. } => "KDE Connect",
        }
    }
}

pub fn identity() -> String {
    let path = crate::theme::state_dir().join("localsend-id");
    if let Ok(id) = std::fs::read_to_string(&path) {
        if id.trim().len() >= 16 {
            return id.trim().to_string();
        }
    }
    let mut bytes = [0u8; 16];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        let _ = std::io::Read::read_exact(&mut f, &mut bytes);
    }
    let id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let _ = crate::atomic::write(&path, &id);
    id
}

fn alias() -> String {
    let host = std::fs::read_to_string("/etc/hostname").ok().map(|h| h.trim().to_string()).filter(|h| !h.is_empty());
    format!("digit · {}", host.unwrap_or_else(|| "linux".into()))
}

pub fn info(fingerprint: &str) -> Value {
    json!({
        "alias": alias(),
        "version": "2.1",
        "deviceModel": "Linux",
        "deviceType": "desktop",
        "fingerprint": fingerprint,
        "port": PORT,
        "protocol": "https",
        "download": false,
    })
}

fn bind() -> Option<UdpSocket> {
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0);
        if fd < 0 {
            return None;
        }
        let one: libc::c_int = 1;
        let len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_REUSEADDR, (&one as *const libc::c_int).cast(), len);
        libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_REUSEPORT, (&one as *const libc::c_int).cast(), len);
        let addr = libc::sockaddr_in { sin_family: libc::AF_INET as libc::sa_family_t, sin_port: PORT.to_be(), sin_addr: libc::in_addr { s_addr: 0 }, sin_zero: [0; 8] };
        if libc::bind(fd, (&addr as *const libc::sockaddr_in).cast(), std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t) < 0 {
            libc::close(fd);
            return None;
        }
        Some(UdpSocket::from_raw_fd(fd))
    }
}

pub fn discover() -> Vec<Peer> {
    let me = identity();
    let Some(sock) = bind() else { return Vec::new() };
    if sock.join_multicast_v4(&GROUP, &Ipv4Addr::UNSPECIFIED).is_err() {
        return Vec::new();
    }
    let _ = sock.set_read_timeout(Some(Duration::from_millis(250)));
    let mut hello = info(&me);
    hello["announce"] = json!(true);
    let hello = hello.to_string();
    let mut found: Vec<Peer> = Vec::new();
    let start = Instant::now();
    let mut buf = [0u8; 4096];
    let mut sent = 0;
    while start.elapsed() < Duration::from_millis(2600) {
        if sent < 3 && start.elapsed() >= Duration::from_millis(sent * 700) {
            let _ = sock.send_to(hello.as_bytes(), (GROUP, PORT));
            sent += 1;
        }
        let Ok((n, from)) = sock.recv_from(&mut buf) else { continue };
        let Ok(v) = serde_json::from_slice::<Value>(&buf[..n]) else { continue };
        if v.get("fingerprint").and_then(Value::as_str) == Some(me.as_str()) {
            continue;
        }
        let ip = from.ip().to_string();
        let port = v.get("port").and_then(Value::as_u64).unwrap_or(PORT as u64) as u16;
        let https = v.get("protocol").and_then(Value::as_str) != Some("http");
        let via = Via::LocalSend { ip, port, https };
        if found.iter().any(|d| d.via == via) {
            continue;
        }
        found.push(Peer {
            name: v.get("alias").and_then(Value::as_str).unwrap_or("LocalSend").to_string(),
            kind: v.get("deviceType").and_then(Value::as_str).unwrap_or("desktop").to_string(),
            via,
        });
    }
    found
}

pub async fn kde() -> Vec<Peer> {
    let Ok(conn) = gio::bus_get_future(gio::BusType::Session).await else { return Vec::new() };
    let call = |path: String, iface: &str, method: &str, args: Option<glib::Variant>| {
        let conn = conn.clone();
        let iface = iface.to_string();
        let method = method.to_string();
        async move { conn.call_future(Some("org.kde.kdeconnect"), &path, &iface, &method, args.as_ref(), None, gio::DBusCallFlags::NONE, 3000).await.ok() }
    };
    let Some(ids) = call("/modules/kdeconnect".into(), "org.kde.kdeconnect.daemon", "devices", Some((true, true).to_variant())).await else { return Vec::new() };
    let ids: Vec<String> = ids.child_value(0).get().unwrap_or_default();
    let mut out = Vec::new();
    for id in ids {
        let path = format!("/modules/kdeconnect/devices/{id}");
        let prop = |name: &'static str| call(path.clone(), "org.freedesktop.DBus.Properties", "Get", Some(("org.kde.kdeconnect.device", name).to_variant()));
        let get = |v: Option<glib::Variant>| v.and_then(|v| v.child_value(0).as_variant()).and_then(|v| v.get::<String>());
        let name = get(prop("name").await).unwrap_or_else(|| id.clone());
        let kind = get(prop("type").await).unwrap_or_else(|| "phone".into());
        out.push(Peer { name, kind, via: Via::KdeConnect { id } });
    }
    out
}

pub async fn kde_send(id: &str, files: &[PathBuf]) -> bool {
    let Ok(conn) = gio::bus_get_future(gio::BusType::Session).await else { return false };
    let path = format!("/modules/kdeconnect/devices/{id}/share");
    for f in files {
        let uri = gio::File::for_path(f).uri().to_string();
        let r = conn.call_future(Some("org.kde.kdeconnect"), &path, "org.kde.kdeconnect.device.share", "shareUrl", Some(&(uri,).to_variant()), None, gio::DBusCallFlags::NONE, 10_000).await;
        if r.is_err() {
            return false;
        }
    }
    true
}

fn curl() -> Command {
    let mut c = Command::new("curl");
    c.args(["-sk", "--connect-timeout", "5", "--speed-limit", "1", "--speed-time", "30"]).stderr(Stdio::null());
    unsafe {
        c.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
    c
}

fn mime(path: &Path) -> String {
    let (kind, _) = gio::content_type_guess(Some(path), None::<&[u8]>);
    gio::content_type_get_mime_type(&kind).map(|m| m.to_string()).unwrap_or_else(|| "application/octet-stream".into())
}

pub fn localsend(ip: &str, port: u16, https: bool, files: &[PathBuf], progress: &dyn Fn(String)) -> Result<(), String> {
    let base = format!("{}://{ip}:{port}/api/localsend/v2", if https { "https" } else { "http" });
    let mut list = serde_json::Map::new();
    for (i, f) in files.iter().enumerate() {
        let size = std::fs::metadata(f).map(|m| m.len()).map_err(|e| e.to_string())?;
        let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        list.insert(format!("f{i}"), json!({ "id": format!("f{i}"), "fileName": name, "size": size, "fileType": mime(f) }));
    }
    let body = json!({ "info": info(&identity()), "files": list }).to_string();
    let mut child = curl()
        .args(["--max-time", "180", "-w", "\n%{http_code}", "-X", "POST", "-H", "Content-Type: application/json", "--data-binary", "@-", &format!("{base}/prepare-upload")])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(body.as_bytes());
    }
    let mut out = Vec::new();
    let result = child.stdout.take().ok_or("Bad reply")?.take(1024 * 1024 + 1).read_to_end(&mut out);
    if result.is_err() || out.len() > 1024 * 1024 {
        let _ = child.kill();
        let _ = child.wait();
        return Err("Bad reply".into());
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() { return Err("Transfer failed".into()); }
    let text = String::from_utf8_lossy(&out).to_string();
    let (payload, code) = text.rsplit_once('\n').unwrap_or(("", text.as_str()));
    match code.trim() {
        "200" => {}
        "204" => return Ok(()),
        "403" => return Err("Declined".into()),
        "409" | "429" => return Err("Busy".into()),
        "" | "000" => return Err("Unreachable".into()),
        other => return Err(format!("Error {other}")),
    }
    let reply: Value = serde_json::from_str(payload).map_err(|_| "Bad reply".to_string())?;
    let session = reply.get("sessionId").and_then(Value::as_str).ok_or("Bad reply")?.to_string();
    let mut sent = 0;
    for (i, f) in files.iter().enumerate() {
        let id = format!("f{i}");
        let Some(token) = reply.get("files").and_then(|m| m.get(&id)).and_then(Value::as_str) else { continue };
        progress(f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        let url = format!("{base}/upload?sessionId={session}&fileId={id}&token={token}");
        let status = curl()
            .args(["-o", "/dev/null", "-w", "%{http_code}", "-X", "POST", "-H", "Content-Type: application/octet-stream", "--data-binary"])
            .arg(format!("@{}", f.display()))
            .arg(&url)
            .output()
            .map_err(|e| e.to_string())?;
        if !status.status.success() || String::from_utf8_lossy(&status.stdout).trim() != "200" {
            return Err("Transfer failed".into());
        }
        sent += 1;
    }
    send_result(sent, files.len())
}

fn send_result(sent: usize, total: usize) -> Result<(), String> {
    if sent == total { Ok(()) } else { Err(format!("Sent {sent} of {total}")) }
}

pub const APPS: &[(&str, &str)] = &[
    ("localsend", "LocalSend"),
    ("blip", "Blip"),
    ("warpinator", "Warpinator"),
    ("nozwock.packet", "Packet"),
    ("rquickshare", "RQuickShare"),
    ("quickshare", "Quick Share"),
    ("drey.warp", "Warp"),
    ("kdeconnect.handler", "KDE Connect"),
    ("kdeconnect-handler", "KDE Connect"),
    ("blueman-sendto", "Bluetooth"),
    ("bluetooth-sendto", "Bluetooth"),
    ("telegram", "Telegram"),
];

pub fn apps() -> Vec<(String, gio::AppInfo)> {
    let mut list: Vec<(String, gio::AppInfo)> = Vec::new();
    for info in gio::AppInfo::all() {
        let id = info.id().map(|i| i.to_lowercase()).unwrap_or_default();
        let Some((_, label)) = APPS.iter().find(|(k, _)| id.contains(k)) else { continue };
        if list.iter().any(|(n, _)| n == label) {
            continue;
        }
        list.push((label.to_string(), info));
    }
    list
}

pub fn launch_with(info: &gio::AppInfo, files: &[PathBuf], ctx: Option<&gio::AppLaunchContext>) -> Result<(), String> {
    let gfiles: Vec<gio::File> = files.iter().map(gio::File::for_path).collect();
    if !gfiles.is_empty() && info.supports_files() {
        info.launch(&gfiles, ctx).map_err(|e| e.to_string())
    } else if !gfiles.is_empty() && info.supports_uris() {
        let uris: Vec<String> = gfiles.iter().map(|f| f.uri().to_string()).collect();
        let refs: Vec<&str> = uris.iter().map(String::as_str).collect();
        info.launch_uris(&refs, ctx).map_err(|e| e.to_string())
    } else {
        info.launch(&[], ctx).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn reports_partial_sends() {
        assert_eq!(super::send_result(2, 2), Ok(()));
        assert_eq!(super::send_result(1, 2), Err("Sent 1 of 2".into()));
        assert_eq!(super::send_result(0, 2), Err("Sent 0 of 2".into()));
    }
}
