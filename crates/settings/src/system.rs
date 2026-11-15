use std::collections::HashMap;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct Gpu {
    pub card: String,
    pub vendor: String,
    pub name: String,
    pub driver: String,
    pub pci: (u32, u32),
}

impl Gpu {
    pub fn kind(&self) -> &'static str {
        match self.pci.0 {
            0x10de => "nvidia",
            0x1002 => "amd",
            0x8086 => "intel",
            _ => "other",
        }
    }
}

fn read(path: impl AsRef<Path>) -> String {
    std::fs::read_to_string(path).map(|s| s.trim().to_string()).unwrap_or_default()
}

pub fn os_release() -> HashMap<String, String> {
    read("/etc/os-release")
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.to_string(), v.trim_matches('"').to_string()))
        .collect()
}

pub fn os_name() -> String {
    let os = os_release();
    os.get("PRETTY_NAME").or(os.get("NAME")).cloned().unwrap_or_else(|| "Linux".into())
}

pub fn os_id() -> String {
    os_release().get("ID").cloned().unwrap_or_default()
}

pub fn kernel() -> String {
    read("/proc/sys/kernel/osrelease")
}

pub fn hostname() -> String {
    let h = read("/etc/hostname");
    if h.is_empty() { read("/proc/sys/kernel/hostname") } else { h }
}

pub fn cpu() -> String {
    let info = read("/proc/cpuinfo");
    let model = info.lines().find(|l| l.starts_with("model name")).and_then(|l| l.split_once(':')).map(|(_, v)| v.trim().to_string()).unwrap_or_default();
    let threads = info.lines().filter(|l| l.starts_with("processor")).count();
    if threads > 0 { format!("{model} · {threads} threads") } else { model }
}

pub fn memory() -> (u64, u64) {
    let info = read("/proc/meminfo");
    let kb = |key: &str| info.lines().find(|l| l.starts_with(key)).and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok()).unwrap_or(0);
    (kb("MemTotal:") * 1024, kb("MemAvailable:") * 1024)
}

pub fn uptime() -> u64 {
    read("/proc/uptime").split_whitespace().next().and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) as u64
}

pub fn disk(path: &str) -> Option<(u64, u64)> {
    use gtk::gio::{self, prelude::FileExt};
    let info = gio::File::for_path(path).query_filesystem_info("filesystem::size,filesystem::free", gio::Cancellable::NONE).ok()?;
    Some((info.attribute_uint64("filesystem::size"), info.attribute_uint64("filesystem::free")))
}

pub fn bytes(n: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i + 1 < units.len() {
        v /= 1024.0;
        i += 1;
    }
    if i >= 3 { format!("{v:.1} {}", units[i]) } else { format!("{v:.0} {}", units[i]) }
}

pub fn duration(secs: u64) -> String {
    let (d, h, m) = (secs / 86_400, secs / 3600 % 24, secs / 60 % 60);
    if d > 0 { format!("{d}d {h}h") } else if h > 0 { format!("{h}h {m}m") } else { format!("{m}m") }
}

fn pci_names(vendor: u32, device: u32) -> (String, String) {
    let text = std::fs::read_to_string("/usr/share/hwdata/pci.ids").unwrap_or_default();
    let v = format!("{vendor:04x}");
    let d = format!("\t{device:04x}");
    let mut vname = String::new();
    let mut inside = false;
    for line in text.lines() {
        if !line.starts_with('\t') && !line.starts_with('#') {
            if inside {
                break;
            }
            if line.starts_with(&v) {
                inside = true;
                vname = line[v.len()..].trim().to_string();
            }
        } else if inside && line.starts_with(&d) && !line.starts_with("\t\t") {
            return (vname, line[d.len()..].trim().to_string());
        }
    }
    (vname, String::new())
}

pub fn gpus() -> Vec<Gpu> {
    let Ok(dir) = std::fs::read_dir("/sys/class/drm") else { return Vec::new() };
    let mut out: Vec<Gpu> = Vec::new();
    for e in dir.flatten() {
        let card = e.file_name().to_string_lossy().to_string();
        if !card.starts_with("card") || card.contains('-') {
            continue;
        }
        let dev = e.path().join("device");
        let hex = |f: &str| u32::from_str_radix(read(dev.join(f)).trim_start_matches("0x"), 16).unwrap_or(0);
        let pci = (hex("vendor"), hex("device"));
        if pci.0 == 0 {
            continue;
        }
        let driver = std::fs::read_link(dev.join("driver")).ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string())).unwrap_or_else(|| "none".into());
        let (vendor, model) = pci_names(pci.0, pci.1);
        let short = model.split_once('[').and_then(|(_, r)| r.split_once(']')).map(|(m, _)| m.to_string()).unwrap_or(model);
        let vendor = vendor.replace(" Corporation", "").replace(", Inc.", "").replace(" [AMD/ATI]", "");
        out.push(Gpu { card, name: if short.is_empty() { format!("{:04x}:{:04x}", pci.0, pci.1) } else { short }, vendor, driver, pci });
    }
    out.sort_by(|a, b| a.card.cmp(&b.card));
    out
}

pub fn nvidia_loaded() -> Option<(String, bool)> {
    let version = read("/sys/module/nvidia/version");
    if version.is_empty() {
        return None;
    }
    let open = read("/proc/driver/nvidia/version").contains("Open Kernel Module");
    Some((version, open))
}
