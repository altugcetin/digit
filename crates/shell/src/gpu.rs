use gtk::prelude::*;
use std::collections::BTreeSet;
use std::path::Path;

const VENDOR: &str = "__EGL_VENDOR_LIBRARY_FILENAMES";
/// What the shell changes in its own environment, restored for the apps it starts.
const INHERITED: [&str; 3] = ["GSK_RENDERER", "GDK_DISABLE", VENDOR];

fn nvidia_vendor() -> Option<String> {
    ["/etc/glvnd/egl_vendor.d", "/usr/share/glvnd/egl_vendor.d"]
        .iter()
        .map(|d| Path::new(d).join("10_nvidia.json"))
        .find(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
}

pub fn prepare() {
    let inherited = INHERITED.map(|key| (key, std::env::var(key).ok()));
    kit::effects::select("DIGIT_RENDERER");
    // SAFETY: called from main before GTK starts or any thread exists, so nothing reads the environment meanwhile.
    unsafe {
        for (key, value) in &inherited {
            std::env::set_var(format!("DIGIT_CHILD_{key}"), value.as_deref().unwrap_or("\u{1}"));
        }
        if std::env::var_os(VENDOR).is_none() && kit::effects::drivers() == ["nvidia"] {
            if let Some(file) = nvidia_vendor() {
                std::env::set_var(VENDOR, file);
            }
        }
    }
}

pub fn child_env() -> Vec<(String, Option<String>)> {
    INHERITED
        .iter()
        .map(|key| {
            let value = std::env::var(format!("DIGIT_CHILD_{key}")).ok().filter(|v| v != "\u{1}");
            (key.to_string(), value)
        })
        .collect()
}

pub fn describe() -> serde_json::Value {
    let active: BTreeSet<_> = gtk::Window::list_toplevels()
        .into_iter()
        .filter(|w| w.is_mapped())
        .filter_map(|w| w.native()?.renderer())
        .map(|r| r.type_().name().to_string())
        .collect();

    // These are the nodes opened by the shell process. GTK does not expose a
    // portable mapping from each GSK renderer to its hardware device.
    let nodes: BTreeSet<_> = std::fs::read_dir("/proc/self/fd")
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|fd| std::fs::read_link(fd.path()).ok())
        .filter(|path| {
            path.parent() == Some(Path::new("/dev/dri"))
                && path.file_name().is_some_and(|name| name.to_string_lossy().starts_with("renderD"))
        })
        .collect();
    let devices: Vec<_> = nodes.into_iter().map(|path| {
        let sys = Path::new("/sys/class/drm").join(path.file_name().unwrap_or_default()).join("device");
        let read = |name| std::fs::read_to_string(sys.join(name)).ok().map(|s| s.trim().to_string());
        let driver = std::fs::read_link(sys.join("driver")).ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
        serde_json::json!({ "node": path, "driver": driver, "vendor_id": read("vendor"), "device_id": read("device") })
    }).collect();

    serde_json::json!({
        "requested": std::env::var("GSK_RENDERER").ok(),
        "gdk_disable": std::env::var("GDK_DISABLE").ok(),
        "active": active,
        "open_render_devices": devices,
    })
}
