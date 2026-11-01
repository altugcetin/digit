//! How digit draws: which GTK renderer its programs use, and whether effects are reduced for
//! slower graphics (no shadows, blur or frosted panels). Settings > Desktop sets both.

use std::cell::Cell;

thread_local! {
    static REDUCED: Cell<(bool, i64)> = const { Cell::new((false, i64::MIN)) };
}

/// Whether effects are reduced, re-read at most every two seconds so the switch applies live.
pub fn reduced() -> bool {
    let now = gtk::glib::monotonic_time();
    REDUCED.with(|r| {
        let (on, at) = r.get();
        if now.saturating_sub(at) < 2_000_000 {
            return on;
        }
        let on = crate::prefs::flag(&crate::prefs::read(), "reduced_effects", false);
        r.set((on, now));
        on
    })
}

/// Whether the computer has a GPU that GTK can draw with.
fn has_gpu() -> bool {
    std::fs::read_dir("/dev/dri").is_ok_and(|d| d.flatten().any(|e| e.file_name().to_string_lossy().starts_with("renderD")))
}

/// The GSK renderer for a digit program: `env` (such as DIGIT_SETTINGS_RENDERER) wins, then the
/// Rendering setting. Automatic draws on the GPU when there is one and in software otherwise.
pub fn renderer(env: &str) -> String {
    let valid = |r: &str| matches!(r, "cairo" | "gl" | "ngl" | "vulkan");
    if let Some(r) = std::env::var(env).ok().filter(|r| valid(r)) {
        return r;
    }
    match crate::prefs::text(&crate::prefs::read(), "renderer", "auto").as_str() {
        r @ ("gl" | "vulkan" | "cairo") => r.to_string(),
        _ if has_gpu() => "gl".into(),
        _ => "cairo".into(),
    }
}
