//! How digit draws: which GTK renderer its programs use, and whether effects are reduced for
//! slower graphics (no shadows, blur or frosted panels). Settings > Desktop sets both.

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};

static RENDERER: AtomicBool = AtomicBool::new(false);
static SAVING: AtomicBool = AtomicBool::new(false);

thread_local! {
    static SOFTWARE: Cell<(bool, i64)> = const { Cell::new((false, i64::MIN)) };
    static REDUCED: Cell<(bool, i64)> = const { Cell::new((false, i64::MIN)) };
}

/// Whether effects are reduced, re-read at most every two seconds so the switch applies live;
/// also while the computer saves power, when the shell says so.
pub fn reduced() -> bool {
    let now = gtk::glib::monotonic_time();
    let chosen = REDUCED.with(|r| {
        let (on, at) = r.get();
        if now.saturating_sub(at) < 2_000_000 {
            return on;
        }
        let on = effective(&crate::prefs::read(), "reduced_effects", software());
        r.set((on, now));
        on
    });
    chosen || saving()
}

/// Draws lighter while the computer saves power (the shell sets this in Power Saver), without
/// touching the Reduce effects setting; whether that changed anything.
pub fn set_saving(on: bool) -> bool {
    SAVING.swap(on, Ordering::Relaxed) != on
}

pub fn saving() -> bool {
    SAVING.load(Ordering::Relaxed)
}

/// Drop the short-lived preference cache before rebuilding styles.
pub fn invalidate() {
    REDUCED.with(|r| r.set((false, i64::MIN)));
}

/// Both masters default to software detection only when no explicit boolean is stored.
pub(crate) fn effective(t: &toml::Table, key: &str, software: bool) -> bool {
    crate::prefs::flag(t, key, software)
}

/// Whether drawing uses software, independent of the Reduce effects preference.
/// Cache detection for two seconds so draw paths avoid filesystem and environment reads.
pub fn software() -> bool {
    let now = gtk::glib::monotonic_time();
    SOFTWARE.with(|s| {
        let (on, at) = s.get();
        if now.saturating_sub(at) < 2_000_000 {
            return on;
        }
        let on = software_in(has_gpu(), RENDERER.load(Ordering::Relaxed), std::env::var("GSK_RENDERER").ok().as_deref(), std::env::var_os("LIBGL_ALWAYS_SOFTWARE").is_some());
        s.set((on, now));
        on
    })
}

fn software_in(gpu: bool, cairo: bool, gsk: Option<&str>, libgl: bool) -> bool {
    !gpu || cairo || gsk == Some("cairo") || libgl
}

/// True when effects are reduced by detection rather than an explicit preference.
pub fn automatic() -> bool {
    let t = crate::prefs::read();
    t.get("reduced_effects").and_then(toml::Value::as_bool).is_none() && software()
}

/// Whether the computer has a GPU that GTK can draw with.
fn has_gpu() -> bool {
    std::fs::read_dir("/dev/dri").is_ok_and(|d| d.flatten().any(|e| e.file_name().to_string_lossy().starts_with("renderD")))
}

/// The GSK renderer for a digit program: `env` (such as DIGIT_SETTINGS_RENDERER) wins, then the
/// Rendering setting. Automatic draws on the GPU when there is one and in software otherwise.
pub fn renderer(env: &str) -> String {
    let r = resolve_renderer(env);
    RENDERER.store(r == "cairo", Ordering::Relaxed);
    SOFTWARE.with(|s| s.set((false, i64::MIN)));
    invalidate();
    r
}

fn resolve_renderer(env: &str) -> String {
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

/// Picks the renderer for a digit program (see [`renderer`]) and sets it up. Call it first thing
/// in `main`, before GTK starts or any thread does.
///
/// On NVIDIA, GTK's OpenGL renderer is kept on desktop OpenGL rather than its GLES API: glyphs
/// went missing on rarely redrawn surfaces there. (GTK issue 6216 was the original GLES glyph
/// bug and is fixed since 4.14; the desktop surfaces also avoid the glyph cache, see
/// `draw::rasterized` in the shell.)
pub fn select(env: &str) -> String {
    let r = renderer(env);
    let gles_off = matches!(r.as_str(), "gl" | "ngl") && drivers() == ["nvidia"];
    // SAFETY: called at startup, before GTK or any other thread reads the environment.
    unsafe {
        std::env::set_var("GSK_RENDERER", &r);
        if gles_off {
            std::env::set_var("GDK_DISABLE", without_gles(std::env::var("GDK_DISABLE").ok().as_deref()));
        }
    }
    r
}

/// `GDK_DISABLE`'s value with GLES added to what it turns off.
fn without_gles(current: Option<&str>) -> String {
    match current.map(str::trim).filter(|c| !c.is_empty()) {
        Some(c) if c.split(',').any(|f| f.trim() == "gles-api") => c.to_string(),
        Some(c) => format!("{c},gles-api"),
        None => "gles-api".into(),
    }
}

/// The kernel drivers of the display GPUs.
pub fn drivers() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else { return Vec::new() };
    let mut out: Vec<String> = entries
        .flatten()
        .filter(|e| e.file_name().to_str().and_then(|n| n.strip_prefix("card")).is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())))
        .filter_map(|e| std::fs::read_link(e.path().join("device/driver")).ok())
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_effects_override_software_detection() {
        for software in [false, true] {
            assert_eq!(effective(&toml::Table::new(), "reduced_effects", software), software);
            for on in [false, true] {
                let mut t = toml::Table::new();
                t.insert("reduced_effects".into(), on.into());
                assert_eq!(effective(&t, "reduced_effects", software), on);
            }
        }
    }

    #[test]
    fn detects_each_software_rendering_signal() {
        assert!(!software_in(true, false, None, false));
        assert!(!software_in(true, false, Some("gl"), false));
        assert!(software_in(false, false, None, false));
        assert!(software_in(true, true, None, false));
        assert!(software_in(true, false, Some("cairo"), false));
        assert!(software_in(true, false, None, true));
    }

    #[test]
    fn gles_is_added_to_what_gdk_turns_off() {
        assert_eq!(without_gles(None), "gles-api");
        assert_eq!(without_gles(Some("")), "gles-api");
        assert_eq!(without_gles(Some("vulkan")), "vulkan,gles-api");
        assert_eq!(without_gles(Some("gles-api,vulkan")), "gles-api,vulkan");
    }

    #[test]
    fn invalidation_expires_cached_effects() {
        REDUCED.with(|r| r.set((true, gtk::glib::monotonic_time())));
        invalidate();
        REDUCED.with(|r| assert_eq!(r.get().1, i64::MIN));
    }
}
