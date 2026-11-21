//! digit's keyboard shortcuts, kept in digit's own format so they don't depend on a compositor.
//! The defaults live here and change with updates; ~/.config/digit/keybinds.toml holds only what
//! the user changed, by shortcut id:
//!
//! ```toml
//! [keys]
//! launcher = "SUPER + K"                 # moved
//! files = ""                             # turned off
//! snap = ["PRINT", "SUPER + SHIFT + S"]  # on more than one combination
//!
//! [workspaces]                           # going to and moving windows to workspaces
//! go = "SUPER"
//! move = "SUPER + SHIFT"
//! keys = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"]
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
    Minimize,
    Float,
    /// Hyprland: toggle pseudotiling in dwindle.
    Pseudo,
    Maximize,
    Fullscreen,
    Focus(Dir),
    Move(Dir),
    Workspace(u8),
    ToWorkspace(u8),
    /// The next (1) or previous (-1) workspace in use; past the last one, an empty one.
    StepWorkspace(i8),
    /// Moves the window to the next (1) or previous (-1) workspace, as `StepWorkspace` goes.
    MoveStepWorkspace(i8),
    /// Traverse windows in compositor order (1 forward, -1 backward).
    StepWindow(i8),
    Drag,
    Resize,
    /// Hyprland floating-window zones, including restore.
    Snap(&'static str),
    /// Hyprland: to the next or previous monitor, keeping a floating window's place.
    ToMonitor(i8),
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

/// Whether focusing or moving `d` carries on to a workspace at the edge: up and down, or left
/// and right while workspaces lie side by side (see [`spaces_across`]).
pub fn crosses(d: Dir, across: bool) -> bool {
    matches!(d, Dir::Left | Dir::Right) == across
}

/// Whether workspaces lie side by side: Hyprland with workspaces switching horizontally. They
/// stack otherwise, as niri's always do.
pub fn spaces_across() -> bool {
    crate::wm::which() == crate::wm::Wm::Hyprland && crate::prefs::text(&crate::prefs::read(), "hypr_workspace_anim", "vertical").starts_with("horizontal")
}

/// "Focus up", or "Focus up or the previous workspace" when it carries on to one.
fn directed(what: &str, d: Dir, across: bool) -> String {
    if !crosses(d, across) {
        return format!("{what} {}", d.name());
    }
    let to = if what == "Focus" { "" } else { " to" };
    let side = if matches!(d, Dir::Up | Dir::Left) { "previous" } else { "next" };
    format!("{what} {} or{to} the {side} workspace", d.name())
}

/// What a shortcut does that its name can't say, shown under it in Settings.
pub fn note(b: &Bind, across: bool) -> Option<String> {
    let place = |d: Dir| match d {
        Dir::Up => "above",
        Dir::Down => "below",
        Dir::Left => "to the left",
        Dir::Right => "to the right",
    };
    let side = |d: Dir| if matches!(d, Dir::Up | Dir::Left) { "previous" } else { "next" };
    match b.act {
        Act::Window(Win::Focus(d)) if crosses(d, across) => Some(format!("The window {}; with none there, the {} workspace", place(d), side(d))),
        Act::Window(Win::Move(d)) if crosses(d, across) => Some(format!("Moves the window {}; at the edge, to the {} workspace", match d { Dir::Up => "up", Dir::Down => "down", Dir::Left => "left", Dir::Right => "right" }, side(d))),
        Act::Window(Win::StepWorkspace(1) | Win::MoveStepWorkspace(1)) => Some("The next one in use; after the last, a new empty one".into()),
        Act::Window(Win::StepWorkspace(_) | Win::MoveStepWorkspace(_)) => Some("The previous one in use".into()),
        _ => None,
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
    /// Keys as "SUPER + SHIFT + E", several combinations joined by ", " (see [`combos`]);
    /// empty when turned off.
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
        bind("appgrid", "Shell", "All apps", "SUPER + A", ctl("key appgrid"), n, false),
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
        bind("snap", "Screenshots", "Screenshot of an area", "PRINT, SUPER + SHIFT + S", run("digit-snap"), n, false),
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
        bind("minimize", "Windows", "Minimize window", "SUPER + M", Act::Window(Win::Minimize), n, true),
        bind("float", "Windows", "Float or tile the window", "SUPER + V", Act::Window(Win::Float), n, true),
        bind("pseudo", "Windows", "Pseudotile the window", "SUPER + SHIFT + V", Act::Window(Win::Pseudo), n, true),
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
        bind("window-next", "Windows", "Next window", "SUPER + SHIFT + mouse_down", Act::Window(Win::StepWindow(1)), n, true),
        bind("window-prev", "Windows", "Previous window", "SUPER + SHIFT + mouse_up", Act::Window(Win::StepWindow(-1)), n, true),
        bind("workspace-next", "Workspaces", "Next workspace", "SUPER + PAGE_DOWN, SUPER + mouse_down", Act::Window(Win::StepWorkspace(1)), n, true),
        bind("workspace-prev", "Workspaces", "Previous workspace", "SUPER + PAGE_UP, SUPER + mouse_up", Act::Window(Win::StepWorkspace(-1)), n, true),
        bind("to-workspace-next", "Workspaces", "Move window to the next workspace", "SUPER + SHIFT + PAGE_DOWN", Act::Window(Win::MoveStepWorkspace(1)), n, true),
        bind("to-workspace-prev", "Workspaces", "Move window to the previous workspace", "SUPER + SHIFT + PAGE_UP", Act::Window(Win::MoveStepWorkspace(-1)), n, true),
        bind("play", "Media keys", "Play or pause", "XF86AudioPlay, XF86AudioPause", Act::Ctl("key play".into()), LOCKED, true),
        bind("next", "Media keys", "Next track", "XF86AudioNext", Act::Ctl("key next".into()), LOCKED, true),
        bind("previous", "Media keys", "Previous track", "XF86AudioPrev", Act::Ctl("key previous".into()), LOCKED, true),
        bind("mic-mute", "Media keys", "Mute the microphone", "XF86AudioMicMute", Act::Shell("wpctl set-mute @DEFAULT_AUDIO_SOURCE@ toggle".into()), LOCKED, true),
    ];
    for (d, key) in [(Dir::Left, "LEFT"), (Dir::Right, "RIGHT"), (Dir::Up, "UP"), (Dir::Down, "DOWN")] {
        out.push(bind(&format!("focus-{}", d.name()), "Windows", &directed("Focus", d, false), &format!("SUPER + {key}"), Act::Window(Win::Focus(d)), n, true));
        out.push(bind(&format!("move-{}", d.name()), "Windows", &directed("Move window", d, false), &format!("SUPER + SHIFT + {key}"), Act::Window(Win::Move(d)), n, true));
        out.push(bind(&format!("grow-{}", d.name()), "Windows", &format!("Resize window {}", d.name()), &format!("SUPER + CTRL + {key}"), Act::Window(Win::Grow(d)), Opts { repeating: true, ..n }, true));
    }
    for (zone, key, label) in [("left", "LEFT", "Snap left"), ("right", "RIGHT", "Snap right"), ("max", "UP", "Maximize floating window"), ("restore", "DOWN", "Restore floating window")] {
        out.push(bind(&format!("hypr-snap-{zone}"), "Windows", label, &format!("SUPER + ALT + {key}"), Act::Window(Win::Snap(zone)), n, true));
    }
    for (zone, key, label) in [("center", "C", "Center floating window"), ("grow", "EQUAL", "Grow floating window"), ("shrink", "MINUS", "Shrink floating window")] {
        out.push(bind(&format!("hypr-snap-{zone}"), "Windows", label, &format!("SUPER + ALT + {key}"), Act::Window(Win::Snap(zone)), n, true));
    }
    out.push(bind("hypr-monitor-next", "Windows", "Move window to the next monitor", "SUPER + ALT + PERIOD", Act::Window(Win::ToMonitor(1)), n, true));
    out.push(bind("hypr-monitor-prev", "Windows", "Move window to the previous monitor", "SUPER + ALT + COMMA", Act::Window(Win::ToMonitor(-1)), n, true));
    // Settings shows these as two modifiers and a key per workspace (see [`Spaces`]).
    let spaces = Spaces::default();
    for i in 1..=SPACES as u8 {
        out.push(bind(&format!("workspace-{i}"), "Workspaces", &format!("Go to workspace {i}"), &spaces.go_keys(i), Act::Window(Win::Workspace(i)), n, true));
        out.push(bind(&format!("to-workspace-{i}"), "Workspaces", &format!("Move window to workspace {i}"), &spaces.move_keys(i), Act::Window(Win::ToWorkspace(i)), n, true));
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

fn for_writing(file: &std::path::Path) -> Result<toml::Table, String> {
    match std::fs::read_to_string(file) {
        Ok(text) => text.parse::<toml::Table>().map_err(|e| format!("{} has an error, so nothing was saved: {e}", file.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(toml::Table::new()),
        Err(e) => Err(format!("{} could not be read, so nothing was saved: {e}", file.display())),
    }
}

/// Separates the combinations of a shortcut on more than one.
pub const SEP: &str = ", ";

/// The key combinations in `keys`, each as "SUPER + SHIFT + E".
pub fn combos(keys: &str) -> Vec<String> {
    keys.split(',').map(str::trim).filter(|k| !k.is_empty()).map(str::to_string).collect()
}

/// One shortcut per key combination, for the compositor configs that bind each on its own.
pub fn expand(binds: &[Bind]) -> Vec<Bind> {
    binds.iter().flat_map(|b| combos(&b.keys).into_iter().map(move |keys| Bind { keys, ..b.clone() })).collect()
}

/// Shortcuts that became a second combination of another one: (old id, the one it joined, which
/// combination it was). A saved override of the old id still counts.
const JOINED: [(&str, &str, usize); 2] = [("snap-area", "snap", 1), ("pause", "play", 1)];

/// How many workspaces have shortcuts.
pub const SPACES: usize = 10;

/// Workspace shortcuts: a modifier to go to a workspace, one to move the window there, and each
/// workspace's own key (keybinds.toml's [workspaces]).
#[derive(Clone, Debug, PartialEq)]
pub struct Spaces {
    pub go: String,
    pub to: String,
    pub keys: [String; SPACES],
}

impl Default for Spaces {
    fn default() -> Self {
        Self { go: "SUPER".into(), to: "SUPER + SHIFT".into(), keys: std::array::from_fn(|i| ((i + 1) % 10).to_string()) }
    }
}

impl Spaces {
    fn read(t: &toml::Table) -> Self {
        let mut s = Self::default();
        let Some(w) = t.get("workspaces").and_then(|v| v.as_table()) else { return s };
        let text = |k: &str| w.get(k).and_then(|v| v.as_str()).map(|v| v.trim().to_string());
        if let Some(go) = text("go") { s.go = go; }
        if let Some(to) = text("move") { s.to = to; }
        if let Some(keys) = w.get("keys").and_then(|v| v.as_array()) {
            for (slot, k) in s.keys.iter_mut().zip(keys) {
                if let Some(k) = k.as_str() { *slot = k.trim().to_string(); }
            }
        }
        s
    }

    fn join(modifier: &str, key: &str) -> String {
        if modifier.is_empty() || key.is_empty() { String::new() } else { format!("{modifier} + {key}") }
    }

    /// The keys that go to workspace `i` (1-based); empty when off.
    pub fn go_keys(&self, i: u8) -> String {
        self.keys.get(usize::from(i).wrapping_sub(1)).map_or(String::new(), |k| Self::join(&self.go, k))
    }

    /// The keys that move the window to workspace `i` (1-based); empty when off.
    pub fn move_keys(&self, i: u8) -> String {
        self.keys.get(usize::from(i).wrapping_sub(1)).map_or(String::new(), |k| Self::join(&self.to, k))
    }
}

/// The workspace shortcuts in effect.
pub fn spaces() -> Spaces {
    Spaces::read(&overrides())
}

/// Saves the workspace shortcuts; digit's defaults leave nothing behind.
pub fn set_spaces(s: &Spaces) -> Result<(), String> {
    let mut t = for_writing(&path())?;
    if *s == Spaces::default() {
        t.remove("workspaces");
    } else {
        let mut w = toml::Table::new();
        w.insert("go".into(), toml::Value::String(s.go.clone()));
        w.insert("move".into(), toml::Value::String(s.to.clone()));
        w.insert("keys".into(), toml::Value::Array(s.keys.iter().map(|k| toml::Value::String(k.clone())).collect()));
        t.insert("workspaces".into(), toml::Value::Table(w));
    }
    // Workspaces are set as a whole now; older per-workspace overrides would only confuse.
    if let Some(k) = t.get_mut("keys").and_then(|v| v.as_table_mut()) {
        k.retain(|id, _| !is_space(id));
    }
    save(&t)
}

/// Whether `id` is one of the go-to or move-to workspace shortcuts.
fn is_space(id: &str) -> bool {
    id.strip_prefix("to-").unwrap_or(id).strip_prefix("workspace-").is_some_and(|n| n.parse::<u8>().is_ok())
}

/// Keys saved for a shortcut: one combination as a string, several as a list.
fn saved(v: &toml::Value) -> Option<String> {
    match v {
        toml::Value::String(k) => Some(combos(k).join(SEP)),
        toml::Value::Array(a) => Some(a.iter().filter_map(|k| k.as_str()).flat_map(combos).collect::<Vec<_>>().join(SEP)),
        _ => None,
    }
}

/// The defaults with the user's changes, one per shortcut, turned-off ones included. `across`:
/// workspaces lie side by side (see [`spaces_across`]), which names the directions that reach them.
fn with_overrides(t: &toml::Table, across: bool) -> Vec<Bind> {
    let keys = t.get("keys").and_then(|v| v.as_table()).cloned().unwrap_or_default();
    let spaces = Spaces::read(t);
    defaults()
        .into_iter()
        .map(|mut b| {
            match b.act {
                Act::Window(Win::Workspace(i)) => b.keys = spaces.go_keys(i),
                Act::Window(Win::ToWorkspace(i)) => b.keys = spaces.move_keys(i),
                Act::Window(Win::Focus(d)) => b.label = directed("Focus", d, across),
                Act::Window(Win::Move(d)) => b.label = directed("Move window", d, across),
                _ => {}
            }
            if is_space(&b.id) {
                return b;
            }
            let joined = JOINED.iter().any(|j| j.1 == b.id);
            match keys.get(&b.id) {
                // Saved as a list, or a shortcut that was never more than one: just that.
                Some(v) if !joined || v.is_array() => b.keys = saved(v).unwrap_or_default(),
                v => {
                    // Saved before other shortcuts joined it: that was its first combination,
                    // and the joined ones keep theirs, moved or turned off.
                    let mut list = combos(&b.keys);
                    if let (Some(k), Some(slot)) = (v.and_then(|v| v.as_str()), list.first_mut()) {
                        *slot = k.trim().to_string();
                    }
                    for (old, _, at) in JOINED.iter().filter(|j| j.1 == b.id) {
                        if let (Some(k), Some(slot)) = (keys.get(*old).and_then(|v| v.as_str()), list.get_mut(*at)) {
                            *slot = k.trim().to_string();
                        }
                    }
                    b.keys = list.into_iter().filter(|k| !k.is_empty()).collect::<Vec<_>>().join(SEP);
                }
            }
            b
        })
        .collect()
}

/// The shortcuts in effect, one per key combination: the defaults with the user's changes and
/// own shortcuts. Turned-off ones are dropped.
pub fn effective() -> Vec<Bind> {
    let t = overrides();
    let mut out: Vec<Bind> = expand(&with_overrides(&t, spaces_across()));
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
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let (tmp, mut out) = loop {
        let tmp = file.with_extension(format!("toml.{}.{}.new", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(out) => break (tmp, out),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    };
    let result = out.write_all(body.as_bytes()).and_then(|()| std::fs::rename(&tmp, &file));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map_err(|e| e.to_string())
}

/// Moves a default shortcut to new keys: one or more combinations joined by ", ", or empty to
/// turn it off.
pub fn set_keys(id: &str, keys: &str) -> Result<(), String> {
    let mut t = for_writing(&path())?;
    let list = combos(keys);
    let table = t.entry("keys").or_insert_with(|| toml::Value::Table(toml::Table::new()));
    if let Some(k) = table.as_table_mut() {
        // A shortcut others joined is saved as a list, which tells it from one saved before.
        let listed = list.len() > 1 || JOINED.iter().any(|j| j.1 == id);
        let value = if listed { toml::Value::Array(list.into_iter().map(toml::Value::String).collect()) } else { toml::Value::String(list.join(SEP)) };
        k.insert(id.to_string(), value);
        for (old, _, _) in JOINED.iter().filter(|j| j.1 == id) {
            k.remove(*old);
        }
    }
    save(&t)
}

/// Puts a default shortcut back on its default keys.
pub fn reset(id: &str) -> Result<(), String> {
    let mut t = for_writing(&path())?;
    if let Some(k) = t.get_mut("keys").and_then(|v| v.as_table_mut()) {
        k.remove(id);
        for (old, _, _) in JOINED.iter().filter(|j| j.1 == id) {
            k.remove(*old);
        }
    }
    save(&t)
}

/// Whether the user moved or turned off this default shortcut.
pub fn changed(id: &str) -> bool {
    let t = overrides();
    if is_space(id) {
        return t.contains_key("workspaces");
    }
    t.get("keys").and_then(|v| v.as_table()).is_some_and(|k| k.contains_key(id) || JOINED.iter().any(|j| j.1 == id && k.contains_key(j.0)))
}

/// Default shortcuts including turned-off ones, one per shortcut with all its keys in effect
/// ("" when off).
pub fn all() -> Vec<Bind> {
    with_overrides(&overrides(), spaces_across())
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
    combos(&b.keys).first().map(|k| label(k))
}

/// Like [`shown`], with every combination: "Print or Super+Shift+S".
pub fn shown_all(id: &str) -> Option<String> {
    let b = all().into_iter().find(|b| b.id == id)?;
    let list: Vec<String> = combos(&b.keys).iter().map(|k| label(k)).collect();
    (!list.is_empty()).then(|| list.join(" or "))
}

/// A shortcut written for people, including mouse wheel directions. Several combinations read
/// "Print or Super+Shift+S".
pub fn label(keys: &str) -> String {
    if keys.contains(',') {
        return combos(keys).iter().map(|k| label(k)).collect::<Vec<_>>().join(" or ");
    }
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
    let mut t = for_writing(&path())?;
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
    fn minimize_is_a_window_action_with_a_unique_default_key() {
        let binds = defaults();
        let minimize = binds.iter().find(|b| b.id == "minimize").unwrap();
        assert_eq!(minimize.group, "Windows");
        assert_eq!(minimize.label, "Minimize window");
        assert_eq!(minimize.keys, "SUPER + M");
        assert_eq!(minimize.act, Act::Window(Win::Minimize));
        assert!(minimize.window);
        assert_eq!(binds.iter().filter(|b| normalize(&b.keys) == normalize(&minimize.keys)).count(), 1);
        assert_eq!(binds.iter().filter(|b| b.id == minimize.id).count(), 1);
    }

    #[test]
    fn edits_preserve_invalid_shortcut_files() {
        const CHILD: &str = "DIGIT_KEYBINDS_WRITE_TEST";
        if std::env::var_os(CHILD).is_some() {
            let file = path();
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            for body in [b"[keys".as_slice(), b"invalid = \"unterminated", b"\xff"] {
                std::fs::write(&file, body).unwrap();
                assert!(set_keys("launcher", "SUPER + A").is_err());
                assert!(reset("launcher").is_err());
                assert!(set_custom(&[]).is_err());
                assert_eq!(std::fs::read(&file).unwrap(), body);
            }
            std::fs::remove_file(&file).unwrap();
            set_keys("launcher", "SUPER + A").unwrap();
            assert!(for_writing(&file).unwrap().contains_key("keys"));
            assert_eq!(std::fs::read_dir(file.parent().unwrap()).unwrap().count(), 1);
            std::fs::remove_file(&file).unwrap();
            std::fs::create_dir(&file).unwrap();
            assert!(set_custom(&[]).is_err());
            return;
        }
        let root = std::env::temp_dir().join(format!("digit-keybinds-xdg-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "keybinds::tests::edits_preserve_invalid_shortcut_files"])
            .env(CHILD, "1").env("XDG_CONFIG_HOME", &root).output().unwrap();
        let _ = std::fs::remove_dir_all(root);
        assert!(result.status.success(), "{}{}", String::from_utf8_lossy(&result.stdout), String::from_utf8_lossy(&result.stderr));
    }

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
        // The arrows focus; the wheel steps through workspaces.
        assert_eq!(get("focus-up").keys, "SUPER + UP");
        assert_eq!(get("focus-down").keys, "SUPER + DOWN");
        assert!(!binds.iter().any(|b| b.id.starts_with("focus-scroll-")));
        assert_eq!(get("window-next").keys, "SUPER + SHIFT + mouse_down");
        assert_eq!(get("window-next").act, Act::Window(Win::StepWindow(1)));
        assert_eq!(get("window-prev").keys, "SUPER + SHIFT + mouse_up");
        assert_eq!(get("window-prev").act, Act::Window(Win::StepWindow(-1)));
        assert_eq!(get("workspace-next").keys, "SUPER + PAGE_DOWN, SUPER + mouse_down");
        assert_eq!(get("workspace-prev").keys, "SUPER + PAGE_UP, SUPER + mouse_up");
        assert_eq!(get("to-workspace-next").act, Act::Window(Win::MoveStepWorkspace(1)));
        assert_eq!(get("to-workspace-prev").keys, "SUPER + SHIFT + PAGE_UP");
        let mut seen = std::collections::HashSet::new();
        for k in expand(&binds).iter().map(|b| normalize(&b.keys)) {
            assert!(seen.insert(k.clone()), "{k} is on two defaults");
        }
    }

    #[test]
    fn a_shortcut_can_hold_several_combinations() {
        let binds = defaults();
        let snap = binds.iter().find(|b| b.id == "snap").unwrap();
        assert_eq!(combos(&snap.keys), ["PRINT", "SUPER + SHIFT + S"]);
        assert!(!binds.iter().any(|b| b.id == "snap-area" || b.id == "pause"));
        let each: Vec<_> = expand(std::slice::from_ref(snap));
        assert_eq!(each.iter().map(|b| b.keys.as_str()).collect::<Vec<_>>(), ["PRINT", "SUPER + SHIFT + S"]);
        assert!(each.iter().all(|b| b.id == "snap" && b.label == snap.label));
        assert_eq!(label(&snap.keys), "Print or Super+Shift+S");
        for render in [crate::hyprconf::render_keybinds(&binds), crate::niriconf::render_keybinds(&binds)] {
            assert!(render.contains("PRINT") || render.contains("Print"));
            assert!(render.contains("SUPER + SHIFT + S") || render.contains("Super+Shift+S"));
            assert!(render.contains("XF86AudioPause"));
        }
    }

    #[test]
    fn saved_keys_read_as_text_or_list_and_old_ids_still_count() {
        let get = |t: &str, id: &str| with_overrides(&t.parse().unwrap(), false).into_iter().find(|b| b.id == id).unwrap().keys;
        assert_eq!(get("[keys]\nsnap = ['SUPER + A', 'SUPER + B']", "snap"), "SUPER + A, SUPER + B");
        assert_eq!(get("[keys]\nsnap = ['SUPER + A']", "snap"), "SUPER + A");
        assert_eq!(get("[keys]\nsnap = []", "snap"), "");
        assert_eq!(get("[keys]\nfiles = 'SUPER + A'", "files"), "SUPER + A");
        assert_eq!(get("[keys]\nfiles = ['SUPER + A', 'F2']", "files"), "SUPER + A, F2");
        // Saved before others joined it: only its own combination changes.
        assert_eq!(get("[keys]\nsnap = 'SUPER + A'", "snap"), "SUPER + A, SUPER + SHIFT + S");
        assert_eq!(get("[keys]\nsnap = ''", "snap"), "SUPER + SHIFT + S");
        assert_eq!(get("[keys]\nfocus-up = 'SUPER + K'", "focus-up"), "SUPER + K");
        // Moved or turned off when it was a shortcut of its own.
        assert_eq!(get("[keys]\nsnap-area = 'SUPER + A'", "snap"), "PRINT, SUPER + A");
        assert_eq!(get("[keys]\nsnap-area = ''", "snap"), "PRINT");
        assert_eq!(get("[keys]\npause = ''", "play"), "XF86AudioPlay");
        assert_eq!(get("[keys]\nsnap = 'F9'\nsnap-area = 'SUPER + A'", "snap"), "F9, SUPER + A");
    }

    #[test]
    fn workspaces_take_two_modifiers_and_a_key_each() {
        let d = Spaces::default();
        assert_eq!((d.go_keys(1), d.move_keys(1)), ("SUPER + 1".to_string(), "SUPER + SHIFT + 1".to_string()));
        assert_eq!(d.go_keys(10), "SUPER + 0");
        assert_eq!(d.go_keys(11), "");
        let binds = defaults();
        assert_eq!(binds.iter().find(|b| b.id == "workspace-3").unwrap().keys, "SUPER + 3");
        assert_eq!(binds.iter().find(|b| b.id == "to-workspace-10").unwrap().keys, "SUPER + SHIFT + 0");
        let t: toml::Table = "[workspaces]\ngo = 'ALT'\nmove = ''\nkeys = ['Q', '', 'E']\n[keys]\nworkspace-1 = 'F1'".parse().unwrap();
        let got = with_overrides(&t, false);
        let keys = |id: &str| got.iter().find(|b| b.id == id).unwrap().keys.clone();
        assert_eq!(keys("workspace-1"), "ALT + Q");
        assert_eq!(keys("workspace-2"), "");
        assert_eq!(keys("workspace-4"), "ALT + 4");
        assert_eq!(keys("to-workspace-1"), "");
        for (id, space) in [("workspace-1", true), ("to-workspace-10", true), ("workspace-next", false), ("workspace-layout", false), ("to-scratchpad", false)] {
            assert_eq!(is_space(id), space, "{id}");
        }
    }

    #[test]
    fn edits_write_lists_and_drop_what_they_replace() {
        const CHILD: &str = "DIGIT_KEYBINDS_LIST_TEST";
        if std::env::var_os(CHILD).is_some() {
            std::fs::create_dir_all(path().parent().unwrap()).unwrap();
            std::fs::write(path(), "[keys]\nsnap-area = 'SUPER + A'\nworkspace-2 = 'F2'\n").unwrap();
            assert!(changed("snap"));
            set_keys("snap", "PRINT, SUPER + Y").unwrap();
            let t = overrides();
            let keys = t["keys"].as_table().unwrap();
            assert_eq!(keys["snap"].as_array().unwrap().len(), 2);
            assert!(!keys.contains_key("snap-area"));
            assert_eq!(all().into_iter().find(|b| b.id == "snap").unwrap().keys, "PRINT, SUPER + Y");
            assert_eq!(effective().iter().filter(|b| b.id == "snap").count(), 2);
            set_keys("snap", "F9").unwrap();
            assert_eq!(overrides()["keys"]["snap"].as_array().map(Vec::len), Some(1));
            assert_eq!(all().into_iter().find(|b| b.id == "snap").unwrap().keys, "F9");
            set_keys("files", "F9").unwrap();
            assert_eq!(overrides()["keys"]["files"].as_str(), Some("F9"));
            assert!(!changed("workspace-1"));
            let mut s = spaces();
            s.go = "ALT".into();
            set_spaces(&s).unwrap();
            assert!(changed("workspace-1") && changed("to-workspace-5"));
            assert!(!overrides()["keys"].as_table().unwrap().contains_key("workspace-2"));
            assert_eq!(spaces(), s);
            set_spaces(&Spaces::default()).unwrap();
            assert!(!overrides().contains_key("workspaces"));
            return;
        }
        let root = std::env::temp_dir().join(format!("digit-keybinds-list-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "keybinds::tests::edits_write_lists_and_drop_what_they_replace"])
            .env(CHILD, "1").env("XDG_CONFIG_HOME", &root).output().unwrap();
        let _ = std::fs::remove_dir_all(root);
        assert!(result.status.success(), "{}{}", String::from_utf8_lossy(&result.stdout), String::from_utf8_lossy(&result.stderr));
    }

    #[test]
    fn the_directions_that_reach_workspaces_say_so() {
        let t = toml::Table::new();
        let label = |across: bool, id: &str| with_overrides(&t, across).into_iter().find(|b| b.id == id).unwrap().label;
        assert_eq!(label(false, "focus-up"), "Focus up or the previous workspace");
        assert_eq!(label(false, "move-down"), "Move window down or to the next workspace");
        assert_eq!(label(false, "focus-left"), "Focus left");
        assert_eq!(label(true, "focus-up"), "Focus up");
        assert_eq!(label(true, "focus-right"), "Focus right or the next workspace");
        assert_eq!(label(true, "move-left"), "Move window left or to the previous workspace");
        let binds = with_overrides(&t, false);
        let note_of = |id: &str| note(binds.iter().find(|b| b.id == id).unwrap(), false);
        assert_eq!(note_of("focus-up").as_deref(), Some("The window above; with none there, the previous workspace"));
        assert_eq!(note_of("focus-left"), None);
        assert!(note_of("workspace-next").unwrap().contains("empty"));
    }

    #[test]
    fn wheel_labels_use_readable_directions() {
        assert_eq!(label("SUPER + SHIFT + mouse_up"), "Super+Shift+Scroll up");
        assert_eq!(label("CTRL + mouse_down"), "Ctrl+Scroll down");
    }
}
