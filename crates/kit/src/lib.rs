pub mod appearance;
pub mod cursors;
pub mod app;
pub mod bluez;
pub mod calc;
pub mod color;
pub mod corners;
pub mod distro;
pub mod drag;
pub mod effects;
pub mod folder;
pub mod fuzzy;
pub mod hyprconf;
pub mod niriconf;
pub mod setup;
pub mod keybinds;
pub mod motion;
pub mod optional;
pub mod paint;
pub mod palette;
pub mod pictures;
pub mod pixel;
pub mod pk;
pub mod prefs;
pub mod scroll;
pub mod search;
pub mod send;
pub mod shape;
pub mod style;
pub mod ui;
pub mod niri;
pub mod nm;
pub mod wm;
pub mod terminal;
pub mod theme;

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
