use gtk::{gio, glib};
use std::path::PathBuf;

#[derive(Clone, Copy, Default, PartialEq)]
pub struct Stats {
    pub cpu: f64,
    pub mem: f64,
    pub used: f64,
    pub total: f64,
    pub swap_used: f64,
    pub swap_total: f64,
    pub temp: Option<f64>,
    pub gpu: Option<f64>,
}

#[derive(Default)]
pub struct Sampler {
    last: Option<(u64, u64)>,
    hwmon: Option<Option<PathBuf>>,
    amd: Option<Option<PathBuf>>,
}

fn cpu_times() -> Option<(u64, u64)> {
    let stat = std::fs::read_to_string("/proc/stat").ok()?;
    let line = stat.lines().next()?;
    let v: Vec<u64> = line.split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
    if v.len() < 5 {
        return None;
    }
    let idle = v[3] + v.get(4).copied().unwrap_or(0);
    Some((idle, v.iter().take(8).sum()))
}

/// Used and total RAM, then used and total swap, in GiB.
fn memory() -> Option<(f64, f64, f64, f64)> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let get = |k: &str| info.lines().find(|l| l.starts_with(k)).and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok());
    let total = get("MemTotal:")?;
    let avail = get("MemAvailable:")?;
    let swap = get("SwapTotal:").unwrap_or(0.0);
    let free = get("SwapFree:").unwrap_or(0.0);
    Some(((total - avail) / 1048576.0, total / 1048576.0, (swap - free) / 1048576.0, swap / 1048576.0))
}

fn find_hwmon() -> Option<PathBuf> {
    let dir = std::fs::read_dir("/sys/class/hwmon").ok()?;
    let mut found: Vec<(usize, PathBuf)> = dir
        .flatten()
        .filter_map(|e| {
            let name = std::fs::read_to_string(e.path().join("name")).ok()?;
            let rank = ["k10temp", "zenpower", "coretemp", "cpu_thermal", "acpitz"].iter().position(|n| *n == name.trim())?;
            e.path().join("temp1_input").exists().then(|| (rank, e.path()))
        })
        .collect();
    found.sort_by_key(|f| f.0);
    found.into_iter().next().map(|f| f.1)
}

fn find_amd() -> Option<PathBuf> {
    let dir = std::fs::read_dir("/sys/class/drm").ok()?;
    dir.flatten().map(|e| e.path().join("device/gpu_busy_percent")).find(|p| p.exists())
}

impl Sampler {
    pub fn sample(&mut self) -> Stats {
        let mut s = Stats::default();
        if let Some(now) = cpu_times() {
            if let Some((idle, total)) = self.last {
                let dt = now.1.saturating_sub(total) as f64;
                if dt > 0.0 {
                    s.cpu = (1.0 - now.0.saturating_sub(idle) as f64 / dt).clamp(0.0, 1.0);
                }
            }
            self.last = Some(now);
        }
        if let Some((used, total, swap_used, swap_total)) = memory() {
            s.used = used;
            s.total = total;
            s.swap_used = swap_used;
            s.swap_total = swap_total;
            s.mem = (used / total.max(0.001)).clamp(0.0, 1.0);
        }
        let hw = self.hwmon.get_or_insert_with(find_hwmon).clone();
        s.temp = hw.and_then(|p| std::fs::read_to_string(p.join("temp1_input")).ok()).and_then(|t| t.trim().parse::<f64>().ok()).map(|t| t / 1000.0);
        let amd = self.amd.get_or_insert_with(find_amd).clone();
        s.gpu = amd.and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| t.trim().parse::<f64>().ok()).map(|g| (g / 100.0).clamp(0.0, 1.0));
        s
    }

    pub fn wants_nvidia(&mut self) -> bool {
        self.amd.get_or_insert_with(find_amd).is_none() && glib::find_program_in_path("nvidia-smi").is_some()
    }
}

static NVIDIA: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn gpu_usage(out: &str) -> Option<f64> {
    out.lines().next()?.trim().parse::<f64>().ok().map(|g| (g / 100.0).clamp(0.0, 1.0))
}

pub async fn nvidia() -> Option<f64> {
    use std::sync::atomic::Ordering;
    if NVIDIA.swap(true, Ordering::AcqRel) {
        return None;
    }
    gio::spawn_blocking(|| {
        struct Busy;
        impl Drop for Busy {
            fn drop(&mut self) { NVIDIA.store(false, Ordering::Release); }
        }
        let _busy = Busy;
        let mut child = std::process::Command::new("nvidia-smi")
            .args(["--query-gpu=utilization.gpu", "--format=csv,noheader,nounits"])
            .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().ok()?;
        let start = std::time::Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    if !status.success() { return None; }
                    use std::io::Read;
                    let mut out = String::new();
                    child.stdout.take()?.take(4096).read_to_string(&mut out).ok()?;
                    return gpu_usage(&out);
                }
                Ok(None) if start.elapsed() < std::time::Duration::from_secs(3) => {
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            }
        }
    }).await.ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_usage_reads_first_device_and_rejects_missing_data() {
        assert_eq!(gpu_usage(" 42\n80\n"), Some(0.42));
        assert_eq!(gpu_usage("120"), Some(1.0));
        assert_eq!(gpu_usage("N/A"), None);
        assert_eq!(gpu_usage(""), None);
    }
}
