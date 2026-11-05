//! digit's keyboard shortcuts, kept in digit's own format so they don't depend on a compositor.
//! The defaults live here and change with updates; ~/.config/digit/keybinds.toml holds only what
//! the user changed, by shortcut id:
//!
//! ```toml
//! [keys]
//! launcher = "SUPER + K"   # moved
//! files = ""               # turned off
//!
//! [[custom]]
//! keys = "SUPER + B"
//! command = "firefox"
//! ```
//!
//! A default whose keys change in an update moves for everyone who hadn't moved it themselves.
//! Changing a shortcut's group or label preserves user keys as long as its id stays the same.

use std::path::PathBuf;

/// What a shortcut does.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    /// `digitctl` with these arguments.
    Ctl(String),
    /// One of digit's programs with arguments, such as "digit-snap screen".
    Run(String),
    /// Any command line.
    Shell(String),
    /// The terminal: $TERMINAL, xdg-terminal-exec or kitty.
    Terminal,
    /// A compositor action that has no command, such as closing a window.
    Window(Win),
}

/// Window management actions, bound only by a full setup that owns the whole compositor config.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Win {
    Close,
    Float,
    Maximize,
    Fullscreen,
    Focus(Dir),
    Move(Dir),
    Workspace(u8),
    ToWorkspace(u8),
    /// The next (1) or previous (-1) workspace in use.
    StepWorkspace(i8),
    /// Traverse windows in compositor order (1 forward, -1 backward).
    StepWindow(i8),
    Drag,
    Resize,
    Grow(Dir),
    Pin,
    Group,
    Scratchpad,
    ToScratchpad,
    /// niri: step the column through its preset widths.
    PresetWidth,
    /// niri: zoom out to every workspace at once.
    Overview,
    /// niri: bring the focused column to the middle of the screen.
    Center,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

impl Dir {
    pub fn name(self) -> &'static str {
        match self {
            Dir::Left => "left",
            Dir::Right => "right",
            Dir::Up => "up",
            Dir::Down => "down",
        }
    }
}

/// How a shortcut fires besides a plain press.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Opts {
    /// Works on the lock screen too.
    pub locked: bool,
    /// Repeats while held.
    pub repeating: bool,
    /// A mouse button held with the keys.
    pub mouse: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Bind {
    pub id: String,
    pub group: &'static str,
    pub label: String,
    /// Keys as "SUPER + SHIFT + E"; empty when turned off.
    pub keys: String,
    pub act: Act,
    pub opts: Opts,
    /// Window management, which only a full setup binds.
    pub window: bool,
}

const MEDIA: Opts = Opts { locked: true, repeating: true, mouse: false };
const LOCKED: Opts = Opts { locked: true, repeating: false, mouse: false };
const MOUSE: Opts = Opts { locked: false, repeating: false, mouse: true };

fn bind(id: &str, group: &'static str, label: &str, keys: &str, act: Act, opts: Opts, window: bool) -> Bind {
    Bind { id: id.into(), group, label: label.into(), keys: keys.into(), act, opts, window }
}

/// The shortcuts digit ships with.
pub fn defaults() -> Vec<Bind> {
    let ctl = |a: &str| Act::Ctl(a.into());
    let run = |a: &str| Act::Run(a.into());
    let n = Opts::default();
    let mut out = vec![
        bind("notch", "Shell", "Open the notch", "SUPER + SPACE", ctl("key notch"), n, false),
        bind("launcher", "Shell", "Spotlight", "ALT + SPACE", ctl("key launcher"), n, false),
        bind("clipboard", "Shell", "Clipboard history", "ALT + V", ctl("key clipboard"), n, false),
        bind("control", "Shell", "Control Center", "ALT + C", ctl("key control"), n, false),
        bind("gamebar", "Shell", "Game Bar", "ALT + Z", ctl("gamebar toggle"), n, false),
        bind("spaces", "Shell", "Workspaces", "SUPER + TAB", ctl("key spaces"), n, false),
        bind("windows", "Shell", "Switch windows", "ALT + TAB", ctl("key windows"), n, false),
        bind("windows-back", "Shell", "Switch windows backwards", "ALT + SHIFT + TAB", ctl("key windows-back"), n, false),
        bind("dock", "Shell", "Show or hide the dock", "ALT + D", ctl("key dock"), n, false),
        bind("wallpaper", "Shell", "Wallpapers", "SUPER + W", ctl("key wallpaper"), n, false),
        bind("settings", "Shell", "Settings", "ALT + COMMA", ctl("key settings"), n, false),
        bind("files", "Shell", "Files", "SUPER + E", run("digit-files"), n, false),
        // niri: the layout picker; Hyprland: the next tiling layout everywhere.
        bind("layouts", "Shell", "Change the window layout", "SUPER + Z", ctl("key layouts"), n, false),
        bind("workspace-layout", "Shell", "Change this workspace's layout", "SUPER + SHIFT + Z", ctl("key workspace-layout"), n, false),
        // Off until the user picks keys for them.
        bind("dnd", "Shell", "Do not disturb", "", ctl("dnd toggle"), n, false),
        bind("awake", "Shell", "Keep awake", "", ctl("awake toggle"), n, false),
        bind("pick", "Shell", "Color picker", "", ctl("pick"), n, false),
        bind("lock", "Session", "Lock the screen", "SUPER + L", ctl("key lock"), n, false),
        bind("session", "Session", "Lock, sleep, restart or shut down", "SUPER + SHIFT + E", ctl("key session"), n, false),
        bind("snap", "Screenshots", "Screenshot of an area", "PRINT", run("digit-snap"), n, false),
        bind("snap-area", "Screenshots", "Screenshot of an area", "SUPER + SHIFT + S", run("digit-snap"), n, false),
        bind("snap-screen", "Screenshots", "Screenshot of the screen", "CTRL + PRINT", run("digit-snap screen"), n, false),
        bind("snap-window", "Screenshots", "Screenshot of a window", "ALT + PRINT", run("digit-snap window"), n, false),
        bind("snap-delay", "Screenshots", "Screenshot after 3 seconds", "SHIFT + PRINT", run("digit-snap --delay 3"), n, false),
        bind("record-panel", "Recording", "Open the recorder panel", "", ctl("record panel"), n, false),
        bind("record", "Recording", "Start or stop recording", "SUPER + ALT + R", ctl("record toggle"), n, false),
        bind("record-pause", "Recording", "Pause or resume recording", "", ctl("record pause"), n, false),
        bind("replay", "Recording", "Instant replay on or off", "", ctl("record replay toggle"), n, false),
        bind("replay-save", "Recording", "Save the instant replay", "SUPER + ALT + S", ctl("record replay-save"), n, false),
        bind("stream", "Recording", "Start or stop streaming", "", ctl("record stream toggle"), n, false),
        bind("volume-up", "Media keys", "Volume up", "XF86AudioRaiseVolume", ctl("key volume-up"), MEDIA, false),
        bind("volume-down", "Media keys", "Volume down", "XF86AudioLowerVolume", ctl("key volume-down"), MEDIA, false),
        bind("mute", "Media keys", "Mute", "XF86AudioMute", ctl("key mute"), LOCKED, false),
        bind("brightness-up", "Media keys", "Brightness up", "XF86MonBrightnessUp", ctl("key brightness-up"), MEDIA, false),
        bind("brightness-down", "Media keys", "Brightness down", "XF86MonBrightnessDown", ctl("key brightness-down"), MEDIA, false),
        bind("kbd-up", "Media keys", "Keyboard light up", "XF86KbdBrightnessUp", ctl("key kbd-up"), MEDIA, false),
        bind("kbd-down", "Media keys", "Keyboard light down", "XF86KbdBrightnessDown", ctl("key kbd-down"), MEDIA, false),
        bind("terminal", "Windows", "Terminal", "SUPER + RETURN", Act::Terminal, n, true),
        bind("close", "Windows", "Close the window", "SUPER + Q", Act::Window(Win::Close), n, true),
        bind("float", "Windows", "Float or tile the window", "SUPER + V", Act::Window(Win::Float), n, true),
        bind("maximize", "Windows", "Maximize", "SUPER + F", Act::Window(Win::Maximize), n, true),
        bind("fullscreen", "Windows", "Fullscreen", "SUPER + SHIFT + F", Act::Window(Win::Fullscreen), n, true),
        bind("drag", "Windows", "Move a window with the mouse", "SUPER + mouse:272", Act::Window(Win::Drag), MOUSE, true),
        bind("resize", "Windows", "Resize a window with the mouse", "SUPER + mouse:273", Act::Window(Win::Resize), MOUSE, true),
        bind("pin", "Windows", "Keep a floating window on every workspace", "SUPER + P", Act::Window(Win::Pin), n, true),
        bind("group", "Windows", "Group windows into tabs", "SUPER + G", Act::Window(Win::Group), n, true),
        bind("scratchpad", "Workspaces", "Show or hide the scratchpad", "SUPER + GRAVE", Act::Window(Win::Scratchpad), n, true),
        bind("to-scratchpad", "Workspaces", "Move window to the scratchpad", "SUPER + SHIFT + GRAVE", Act::Window(Win::ToScratchpad), n, true),
        bind("column-width", "Windows", "Cycle the column width", "SUPER + R", Act::Window(Win::PresetWidth), n, true),
        bind("overview", "Workspaces", "Overview of all workspaces", "SUPER + O", Act::Window(Win::Overview), n, true),
        bind("center", "Windows", "Center the column", "", Act::Window(Win::Center), n, true),
        // Keep the workspace-only ids for saved overrides; the wheel now follows spatial focus.
        bind("focus-scroll-up", "Windows", "Focus above", "SUPER + mouse_up", Act::Window(Win::Focus(Dir::Up)), n, true),
        bind("focus-scroll-down", "Windows", "Focus below", "SUPER + mouse_down", Act::Window(Win::Focus(Dir::Down)), n, true),
        bind("window-next", "Windows", "Next window", "SUPER + SHIFT + mouse_down", Act::Window(Win::StepWindow(1)), n, true),
        bind("window-prev", "Windows", "Previous window", "SUPER + SHIFT + mouse_up", Act::Window(Win::StepWindow(-1)), n, true),
        bind("workspace-next", "Workspaces", "Next workspace", "", Act::Window(Win::StepWorkspace(1)), n, true),
        bind("workspace-prev", "Workspaces", "Previous workspace", "", Act::Window(Win::StepWorkspace(-1)), n, true),
        bind("play", "Media keys", "Play or pause", "XF86AudioPlay", Act::Shell("playerctl play-pause".into()), LOCKED, true),
        bind("pause", "Media keys", "Play or pause", "XF86AudioPause", Act::Shell("playerctl play-pause".into()), LOCKED, true),
        bind("next", "Media keys", "Next track", "XF86AudioNext", Act::Shell("playerctl next".into()), LOCKED, true),
        bind("previous", "Media keys", "Previous track", "XF86AudioPrev", Act::Shell("playerctl previous".into()), LOCKED, true),
        bind("mic-mute", "Media keys", "Mute the microphone", "XF86AudioMicMute", Act::Shell("wpctl set-mute @DEFAULT_AUDIO_SOURCE@ toggle".into()), LOCKED, true),
    ];
    for (d, key) in [(Dir::Left, "LEFT"), (Dir::Right, "RIGHT"), (Dir::Up, "UP"), (Dir::Down, "DOWN")] {
        out.push(bind(&format!("focus-{}", d.name()), "Windows", &format!("Focus {}", d.name()), &format!("SUPER + {key}"), Act::Window(Win::Focus(d)), n, true));
        out.push(bind(&format!("move-{}", d.name()), "Windows", &format!("Move window {}", d.name()), &format!("SUPER + SHIFT + {key}"), Act::Window(Win::Move(d)), n, true));
        out.push(bind(&format!("grow-{}", d.name()), "Windows", &format!("Resize window {}", d.name()), &format!("SUPER + CTRL + {key}"), Act::Window(Win::Grow(d)), Opts { repeating: true, ..n }, true));
    }
    for i in 1..=10u8 {
        let key = i % 10;
        out.push(bind(&format!("workspace-{i}"), "Workspaces", &format!("Go to workspace {i}"), &format!("SUPER + {key}"), Act::Window(Win::Workspace(i)), n, true));
        out.push(bind(&format!("to-workspace-{i}"), "Workspaces", &format!("Move window to workspace {i}"), &format!("SUPER + SHIFT + {key}"), Act::Window(Win::ToWorkspace(i)), n, true));
    }
    // Keep each group together (lists show one heading per group), in the order groups first appear.
    let mut order: Vec<&'static str> = Vec::new();
    for b in &out {
        if !order.contains(&b.group) {
            order.push(b.group);
        }
    }
    out.sort_by_key(|b| order.iter().position(|g| *g == b.group));
    out
}

pub fn path() -> PathBuf {
    gtk::glib::user_config_dir().join("digit/keybinds.toml")
}

fn overrides() -> toml::Table {
    std::fs::read_to_string(path()).ok().and_then(|t| t.parse::<toml::Table>().ok()).unwrap_or_default()
}

/// The shortcuts in effect: the defaults with the user's changes and own shortcuts. Turned-off
/// ones are dropped.
pub fn effective() -> Vec<Bind> {
    let t = overrides();
    let keys = t.get("keys").and_then(|v| v.as_table()).cloned().unwrap_or_default();
    let mut out: Vec<Bind> = defaults()
        .into_iter()
        .map(|mut b| {
            if let Some(k) = keys.get(&b.id).and_then(|v| v.as_str()) {
                b.keys = k.trim().to_string();
            }
            b
        })
        .filter(|b| !b.keys.is_empty())
        .collect();
    let custom = t.get("custom").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    for (i, c) in custom.iter().enumerate() {
        let (Some(k), Some(cmd)) = (c.get("keys").and_then(|v| v.as_str()), c.get("command").and_then(|v| v.as_str())) else { continue };
        if k.trim().is_empty() || cmd.trim().is_empty() {
            continue;
        }
        let label = c.get("label").and_then(|v| v.as_str()).unwrap_or(cmd);
        out.push(bind(&format!("custom-{i}"), "Your shortcuts", label, k.trim(), Act::Shell(cmd.into()), Opts::default(), false));
    }
    out
}

/// Keys in one spelling, so "super+shift + e" and "SHIFT + SUPER + E" compare equal.
pub fn normalize(keys: &str) -> String {
    const MODS: [&str; 6] = ["SUPER", "CTRL", "ALT", "SHIFT", "MOD3", "MOD5"];
    let mut mods: Vec<String> = Vec::new();
    let mut rest: Vec<String> = Vec::new();
    for part in keys.split(['+', ' ']).map(str::trim).filter(|p| !p.is_empty()) {
        let up = part.to_ascii_uppercase();
        let up = match up.as_str() {
            "WIN" | "LOGO" | "MOD4" | "META" => "SUPER".to_string(),
            "CONTROL" => "CTRL".to_string(),
            _ => up,
        };
        if MODS.contains(&up.as_str()) {
            if !mods.contains(&up) {
                mods.push(up);
            }
        } else {
            rest.push(up);
        }
    }
    mods.sort_by_key(|m| MODS.iter().position(|x| x == m));
    mods.extend(rest);
    mods.join(" + ")
}

fn save(t: &toml::Table) -> Result<(), String> {
    let file = path();
    if let Some(d) = file.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let body = format!("# digit's shortcuts that differ from its defaults. Settings > Keyboard writes this file.\n{}", toml::to_string_pretty(t).map_err(|e| e.to_string())?);
    let tmp = file.with_extension("toml.new");
    std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &file).map_err(|e| e.to_string())
}

/// Moves a default shortcut to new keys; empty keys turn it off.
pub fn set_keys(id: &str, keys: &str) -> Result<(), String> {
    let mut t = overrides();
    let table = t.entry("keys").or_insert_with(|| toml::Value::Table(toml::Table::new()));
    if let Some(k) = table.as_table_mut() {
        k.insert(id.to_string(), toml::Value::String(keys.trim().to_string()));
    }
    save(&t)
}

/// Puts a default shortcut back on its default keys.
pub fn reset(id: &str) -> Result<(), String> {
    let mut t = overrides();
    if let Some(k) = t.get_mut("keys").and_then(|v| v.as_table_mut()) {
        k.remove(id);
    }
    save(&t)
}

/// Whether the user moved or turned off this default shortcut.
pub fn changed(id: &str) -> bool {
    overrides().get("keys").and_then(|v| v.as_table()).is_some_and(|k| k.contains_key(id))
}

/// Default shortcuts including turned-off ones, with the keys in effect ("" when off).
pub fn all() -> Vec<Bind> {
    let t = overrides();
    let keys = t.get("keys").and_then(|v| v.as_table()).cloned().unwrap_or_default();
    defaults()
        .into_iter()
        .map(|mut b| {
            if let Some(k) = keys.get(&b.id).and_then(|v| v.as_str()) {
                b.keys = k.trim().to_string();
            }
            b
        })
        .collect()
}

/// The keys a shortcut is on now, written for people: "Super+Shift+E". None when it is
/// turned off. Shown wherever digit mentions a shortcut, so it follows what the user set.
pub fn shown(id: &str) -> Option<String> {
    // Panels ask while drawing; read keybinds.toml at most every two seconds.
    static CACHE: std::sync::Mutex<Option<(std::time::Instant, Vec<Bind>)>> = std::sync::Mutex::new(None);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if cache.as_ref().is_none_or(|(at, _)| at.elapsed() > std::time::Duration::from_secs(2)) {
        *cache = Some((std::time::Instant::now(), all()));
    }
    let b = cache.as_ref()?.1.iter().find(|b| b.id == id)?.clone();
    drop(cache);
    if b.keys.trim().is_empty() {
        return None;
    }
    Some(label(&b.keys))
}

/// A shortcut written for people, including mouse wheel directions.
pub fn label(keys: &str) -> String {
    let parts: Vec<String> = keys
        .split('+')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| match p.to_ascii_uppercase().as_str() {
            "SUPER" => "Super".into(),
            "CTRL" | "CONTROL" => "Ctrl".into(),
            "ALT" => "Alt".into(),
            "SHIFT" => "Shift".into(),
            "MOUSE_UP" => "Scroll up".into(),
            "MOUSE_DOWN" => "Scroll down".into(),
            "COMMA" => ",".into(),
            "PERIOD" => ".".into(),
            "RETURN" | "ENTER" => "Enter".into(),
            "GRAVE" => "`".into(),
            "MINUS" => "-".into(),
            "EQUAL" => "=".into(),
            "SLASH" => "/".into(),
            "BACKSLASH" => "\\".into(),
            "SEMICOLON" => ";".into(),
            "APOSTROPHE" => "'".into(),
            "BRACKETLEFT" => "[".into(),
            "BRACKETRIGHT" => "]".into(),
            k if k.len() == 1 => k.into(),
            _ => {
                let mut c = p.chars();
                c.next().map(|f| f.to_uppercase().chain(c.flat_map(char::to_lowercase)).collect()).unwrap_or_default()
            }
        })
        .collect();
    parts.join("+")
}

/// `shown`, or a word saying the shortcut is off.
pub fn shown_or(id: &str, off: &str) -> String {
    shown(id).unwrap_or_else(|| off.to_string())
}

/// The user's own shortcuts: keys, command and label.
pub fn custom() -> Vec<(String, String, String)> {
    let t = overrides();
    t.get("custom")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .map(|c| {
                    let get = |k: &str| c.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
                    (get("keys"), get("command"), get("label"))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Replaces the user's own shortcuts.
pub fn set_custom(list: &[(String, String, String)]) -> Result<(), String> {
    let mut t = overrides();
    let arr: Vec<toml::Value> = list
        .iter()
        .map(|(k, c, l)| {
            let mut e = toml::Table::new();
            e.insert("keys".into(), toml::Value::String(k.clone()));
            e.insert("command".into(), toml::Value::String(c.clone()));
            if !l.is_empty() {
                e.insert("label".into(), toml::Value::String(l.clone()));
            }
            toml::Value::Table(e)
        })
        .collect();
    if arr.is_empty() {
        t.remove("custom");
    } else {
        t.insert("custom".into(), toml::Value::Array(arr));
    }
    save(&t)
}

/// The keys a GTK key press stands for, in Hyprland's spelling ("SUPER + SHIFT + E"); `None`
/// for a modifier pressed on its own.
pub fn from_press(key: gtk::gdk::Key, mods: gtk::gdk::ModifierType) -> Option<String> {
    let name = key.to_lower().name()?.to_string();
    if ["Shift_L", "Shift_R", "Control_L", "Control_R", "Alt_L", "Alt_R", "Super_L", "Super_R", "Meta_L", "Meta_R", "ISO_Level3_Shift", "Hyper_L", "Hyper_R"].contains(&name.as_str()) {
        return None;
    }
    Some(with_modifiers(if name.starts_with("XF86") { name } else { name.to_ascii_uppercase() }, mods))
}

fn with_modifiers(trigger: String, mods: gtk::gdk::ModifierType) -> String {
    use gtk::gdk::ModifierType as M;
    let mut parts = Vec::new();
    if mods.intersects(M::SUPER_MASK | M::META_MASK | M::HYPER_MASK) {
        parts.push("SUPER".to_string());
    }
    if mods.contains(M::CONTROL_MASK) {
        parts.push("CTRL".into());
    }
    if mods.contains(M::ALT_MASK) {
        parts.push("ALT".into());
    }
    if mods.contains(M::SHIFT_MASK) {
        parts.push("SHIFT".into());
    }
    parts.push(trigger);
    parts.join(" + ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_group_contains_only_recording_actions() {
        let binds = defaults();
        let recording: Vec<_> = binds.iter().filter(|b| b.group == "Recording").collect();
        assert!(!recording.is_empty());
        for bind in recording {
            assert!(matches!(&bind.act, Act::Ctl(action) if action.starts_with("record")), "{} is not a recording action", bind.id);
        }
    }

    #[test]
    fn gamebar_is_a_shell_panel_after_control_center() {
        let binds = defaults();
        let control = binds.iter().position(|b| b.id == "control").unwrap();
        let gamebar = &binds[control + 1];
        assert_eq!(gamebar.id, "gamebar");
        assert_eq!(gamebar.group, "Shell");
        assert_eq!(gamebar.keys, "ALT + Z");
        assert_eq!(gamebar.act, Act::Ctl("gamebar toggle".into()));
    }

    #[test]
    fn wheel_focus_and_window_defaults_replace_workspace_only_wheel_actions() {
        let binds = defaults();
        let get = |id: &str| binds.iter().find(|b| b.id == id).unwrap();
        assert_eq!(get("focus-scroll-up").keys, "SUPER + mouse_up");
        assert_eq!(get("focus-scroll-up").act, Act::Window(Win::Focus(Dir::Up)));
        assert_eq!(get("focus-scroll-down").keys, "SUPER + mouse_down");
        assert_eq!(get("focus-scroll-down").act, Act::Window(Win::Focus(Dir::Down)));
        assert_eq!(get("window-next").keys, "SUPER + SHIFT + mouse_down");
        assert_eq!(get("window-next").act, Act::Window(Win::StepWindow(1)));
        assert_eq!(get("window-prev").keys, "SUPER + SHIFT + mouse_up");
        assert_eq!(get("window-prev").act, Act::Window(Win::StepWindow(-1)));
        assert!(get("workspace-next").keys.is_empty());
        assert!(get("workspace-prev").keys.is_empty());
    }

    #[test]
    fn wheel_labels_use_readable_directions() {
        assert_eq!(label("SUPER + SHIFT + mouse_up"), "Super+Shift+Scroll up");
        assert_eq!(label("CTRL + mouse_down"), "Ctrl+Scroll down");
    }
}
