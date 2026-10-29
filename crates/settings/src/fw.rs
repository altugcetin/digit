use std::collections::HashMap;
use zbus::zvariant::OwnedValue;

pub struct Device {
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub updatable: bool,
}

pub async fn devices() -> Result<Vec<Device>, String> {
    let conn = crate::pk::system().await?;
    let proxy = zbus::Proxy::new(&conn, "org.freedesktop.fwupd", "/", "org.freedesktop.fwupd").await.map_err(|e| e.to_string())?;
    let list: Vec<HashMap<String, OwnedValue>> = proxy.call("GetDevices", &()).await.map_err(|e| e.to_string())?;
    let text = |m: &HashMap<String, OwnedValue>, k: &str| m.get(k).and_then(|v| String::try_from(v.try_clone().ok()?).ok()).unwrap_or_default();
    let mut out: Vec<Device> = list
        .iter()
        .map(|m| {
            let flags = m.get("Flags").and_then(|v| u64::try_from(v.try_clone().ok()?).ok()).unwrap_or(0);
            Device { name: text(m, "Name"), vendor: text(m, "Vendor"), version: text(m, "Version"), updatable: flags & (1 << 1) != 0 }
        })
        .filter(|d| !d.name.is_empty() && !d.version.is_empty())
        .collect();
    out.sort_by(|a, b| b.updatable.cmp(&a.updatable).then(a.name.cmp(&b.name)));
    Ok(out)
}
