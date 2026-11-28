pub mod app_themes;
pub mod hidden;
pub mod players;
pub mod qt_colors;
pub mod appearance;
pub mod cursors;
pub mod app;
pub mod atomic;
pub mod bluez;
pub mod calc;
pub mod color;
pub mod corners;
pub mod distro;
pub mod drag;
pub mod effects;
pub mod folder;
pub mod gamescope;
pub mod fuzzy;
pub mod guard;
pub mod hyprconf;
pub mod niriconf;
pub mod setup;
pub mod keybinds;
pub mod motion;
pub mod optional;
pub mod paint;
pub mod art;
pub mod palette;
pub mod pictures;
pub mod pixel;
pub mod pk;
pub mod power;
pub mod prefs;
pub mod wallfocus;
pub mod recent;
pub mod trash;
pub mod screentime;
pub mod screens;
pub mod secret;
pub mod deskwidget;
pub mod scroll;
pub mod search;
pub mod send;
pub mod shape;
pub mod signals;
pub mod style;
pub mod svgicon;
pub mod tag;
pub mod ui;
pub mod niri;
pub mod nm;
pub mod wm;
pub mod winrules;
pub mod terminal;
pub mod theme;

/// Runs the test `name` in a process of its own, and says whether this is that process: GTK
/// belongs to one thread, and an abort there fails this test rather than every other one.
#[cfg(test)]
fn alone(name: &str) -> bool {
    const MARK: &str = "DIGIT_TEST_ALONE";
    if std::env::var(MARK).is_ok_and(|v| v == name) {
        return true;
    }
    let out = std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact", name, "--nocapture"]).env(MARK, name).output().unwrap();
    assert!(out.status.success(), "{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    false
}

/// Goes in every binary, with a build.rs naming gtk4-layer-shell: the reference has to sit in
/// the binary's own object, or `--as-needed` drops that early library again.
#[macro_export]
macro_rules! layer_shell_first {
    () => {
        unsafe extern "C" {
            fn gtk_layer_get_protocol_version() -> u32;
        }
        #[used]
        static LAYER_SHELL_FIRST: unsafe extern "C" fn() -> u32 = gtk_layer_get_protocol_version;
    };
}
