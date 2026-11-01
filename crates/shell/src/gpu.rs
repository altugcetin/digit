use std::path::Path;

const VENDOR: &str = "__EGL_VENDOR_LIBRARY_FILENAMES";

fn drivers() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else { return Vec::new() };
    let mut out: Vec<String> = entries
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.starts_with("card") && name[4..].chars().all(|c| c.is_ascii_digit())
        })
        .filter_map(|e| std::fs::read_link(e.path().join("device/driver")).ok())
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    out.sort();
    out.dedup();
    out
}

fn nvidia_vendor() -> Option<String> {
    ["/etc/glvnd/egl_vendor.d", "/usr/share/glvnd/egl_vendor.d"]
        .iter()
        .map(|d| Path::new(d).join("10_nvidia.json"))
        .find(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
}

pub fn prepare() {
    let renderer = kit::effects::renderer("DIGIT_RENDERER");
    let inherited = [("GSK_RENDERER", std::env::var("GSK_RENDERER").ok()), (VENDOR, std::env::var(VENDOR).ok())];
    unsafe {
        for (key, value) in &inherited {
            std::env::set_var(format!("DIGIT_CHILD_{key}"), value.as_deref().unwrap_or("\u{1}"));
        }
        std::env::set_var("GSK_RENDERER", renderer);
        if std::env::var_os(VENDOR).is_none() && drivers() == ["nvidia"] {
            if let Some(file) = nvidia_vendor() {
                std::env::set_var(VENDOR, file);
            }
        }
    }
}

pub fn child_env() -> Vec<(String, Option<String>)> {
    ["GSK_RENDERER", VENDOR]
        .iter()
        .map(|key| {
            let value = std::env::var(format!("DIGIT_CHILD_{key}")).ok().filter(|v| v != "\u{1}");
            (key.to_string(), value)
        })
        .collect()
}
