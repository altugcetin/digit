//! digit's part of the niri config: ~/.config/niri/digit/, included by one line in
//! config.kdl. It is the niri twin of `hyprconf`: the same shortcuts, Compositor settings and
//! Corners, written as KDL. Settings niri has no counterpart for (tiling layouts, sideways
//! workspaces, sliding windows) are left out.

use crate::hyprconf::{bin, copy_dir, fill, rounding, stamp, terminal, write_if_changed};
use crate::keybinds::{self, Act, Bind, Dir, Win};
use std::path::PathBuf;

pub const INCLUDE: &str = "include \"digit/init.kdl\"";
/// The line older setups added above the include; removing digit takes it out too.
const MARK: &str = "// digit: its part of the config lives in ~/.config/niri/digit (added by digit setup)";

const INIT: &str = include_str!("../../../data/niri/init.kdl");
const MAIN: &str = include_str!("../../../data/niri/config.kdl");
const BASE: &str = include_str!("../../../data/niri/digit-base.kdl");
const DECORATIONS: &str = include_str!("../../../data/niri/digit-decorations.kdl");
const RULES: &str = include_str!("../../../data/niri/digit-rules.kdl");
const AUTOSTART: &str = include_str!("../../../data/niri/digit-autostart.kdl");

pub fn niri() -> PathBuf {
    gtk::glib::user_config_dir().join("niri")
}

pub fn dir() -> PathBuf {
    niri().join("digit")
}

/// Whether digit has set up niri. Older "minimal" setups count as not set up.
pub fn installed() -> bool {
    crate::prefs::text(&crate::prefs::read(), "niri_setup", "") == "full"
}

fn kdl_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// "SUPER + SHIFT + E" in niri's spelling, "Super+Shift+E".
pub fn keys(hypr: &str) -> String {
    hypr.split('+')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| match p.to_ascii_uppercase().as_str() {
            "SUPER" => "Super".to_string(),
            "CTRL" | "CONTROL" => "Ctrl".into(),
            "ALT" => "Alt".into(),
            "SHIFT" => "Shift".into(),
            "MOUSE_DOWN" => "WheelScrollDown".into(),
            "MOUSE_UP" => "WheelScrollUp".into(),
            "GRAVE" => "grave".into(),
            _ if p.starts_with("XF86") || p.len() == 1 => p.to_string(),
            // Space, Return, Print, Left, Page_Up…: capitalized parts, as xkb spells them.
            up => up
                .split('_')
                .map(|w| {
                    let mut c = w.chars();
                    c.next().map(|f| f.to_string() + &c.as_str().to_ascii_lowercase()).unwrap_or_default()
                })
                .collect::<Vec<_>>()
                .join("_"),
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// The niri action for a shortcut, or `None` where niri has nothing like it.
fn action(b: &Bind) -> Option<String> {
    let bin = bin();
    let spawn = |argv: Vec<String>| format!("spawn {};", argv.iter().map(|a| kdl_str(a)).collect::<Vec<_>>().join(" "));
    let program = |name: &str, rest: &str| {
        let mut argv = vec![bin.join(name).display().to_string()];
        argv.extend(rest.split_whitespace().map(str::to_string));
        argv
    };
    Some(match &b.act {
        Act::Ctl(args) => spawn(program("digitctl", args)),
        Act::Run(cmd) => {
            let (name, rest) = cmd.split_once(' ').unwrap_or((cmd, ""));
            spawn(program(name, rest))
        }
        Act::Shell(cmd) => spawn(vec!["sh".into(), "-c".into(), cmd.clone()]),
        Act::Terminal => spawn(terminal().split_whitespace().map(str::to_string).collect()),
        Act::Window(w) => match w {
            Win::Close => "close-window;".into(),
            Win::Float => "toggle-window-floating;".into(),
            Win::Maximize => "maximize-column;".into(),
            Win::Fullscreen => "fullscreen-window;".into(),
            Win::Focus(Dir::Left) => "focus-column-left;".into(),
            Win::Focus(Dir::Right) => "focus-column-right;".into(),
            Win::Focus(Dir::Up) => "focus-window-up;".into(),
            Win::Focus(Dir::Down) => "focus-window-down;".into(),
            // At the edge of the screen a move carries on to the next monitor or workspace.
            Win::Move(Dir::Left) => "move-column-left-or-to-monitor-left;".into(),
            Win::Move(Dir::Right) => "move-column-right-or-to-monitor-right;".into(),
            Win::Move(Dir::Up) => "move-window-up-or-to-workspace-up;".into(),
            Win::Move(Dir::Down) => "move-window-down-or-to-workspace-down;".into(),
            Win::Grow(Dir::Left) => "set-column-width \"-10%\";".into(),
            Win::Grow(Dir::Right) => "set-column-width \"+10%\";".into(),
            Win::Grow(Dir::Up) => "set-window-height \"-10%\";".into(),
            Win::Grow(Dir::Down) => "set-window-height \"+10%\";".into(),
            Win::Workspace(i) => format!("focus-workspace {i};"),
            Win::ToWorkspace(i) => format!("move-column-to-workspace {i};"),
            Win::StepWorkspace(i) if *i > 0 => "focus-workspace-down;".into(),
            Win::StepWorkspace(_) => "focus-workspace-up;".into(),
            Win::Group => "toggle-column-tabbed-display;".into(),
            Win::PresetWidth => "switch-preset-column-width;".into(),
            Win::Overview => "toggle-overview;".into(),
            Win::Center => "center-column;".into(),
            // Moving and resizing with Super and the mouse is built into niri; it has no pinned
            // windows or scratchpad.
            Win::Drag | Win::Resize | Win::Pin | Win::Scratchpad | Win::ToScratchpad => return None,
        },
    })
}

/// Whether niri can do what a shortcut does, for the Keyboard page.
pub fn supports(b: &Bind) -> bool {
    action(b).is_some()
}

/// digit-keybinds.kdl: every shortcut in effect that niri can do.
pub fn render_keybinds(binds: &[Bind]) -> String {
    let mut out = String::from(
        "// Generated by digit from its defaults and ~/.config/digit/keybinds.toml.\n\
         // Changes here are replaced: change shortcuts in digit Settings, or add your own in config.kdl.\n\n\
         binds {\n",
    );
    // niri refuses two binds on the same keys in one file. When a default lands on keys the
    // user gave to something else (or two defaults meet), the user's choice keeps them.
    let mut taken: Vec<String> = binds
        .iter()
        .filter(|b| keybinds::changed(&b.id) || b.id.starts_with("custom-"))
        .filter(|b| action(b).is_some())
        .map(|b| keybinds::normalize(&b.keys))
        .collect();
    let mut placed: Vec<String> = Vec::new();
    let mut group = "";
    for b in binds {
        let Some(act) = action(b) else { continue };
        let k = keybinds::normalize(&b.keys);
        let own = keybinds::changed(&b.id) || b.id.starts_with("custom-");
        if placed.contains(&k) || (!own && taken.contains(&k)) {
            continue;
        }
        placed.push(k.clone());
        if !own {
            taken.push(k);
        }
        if b.group != group {
            group = b.group;
            out.push_str(&format!("    // {group}\n"));
        }
        let mut props = Vec::new();
        let spawns = act.starts_with("spawn");
        if b.opts.locked {
            props.push("allow-when-locked=true".to_string());
        }
        if spawns && !b.opts.repeating {
            props.push("repeat=false".into());
        }
        if matches!(b.act, Act::Window(Win::StepWorkspace(_))) {
            props.push("cooldown-ms=150".into());
        }
        // digit's own panels keep working even when an app inhibits shortcuts.
        if !b.window && !b.opts.locked {
            props.push("allow-inhibiting=false".into());
        }
        let props = if props.is_empty() { String::new() } else { format!(" {}", props.join(" ")) };
        out.push_str(&format!("    {}{props} hotkey-overlay-title={} {{ {act} }}\n", keys(&b.keys), kdl_str(&b.label)));
    }
    out.push_str("}\n");
    out
}

/// Settings > Compositor in KDL: every value, with digit's defaults for the ones never changed.
/// niri can't switch a flag off from an included file, so an off flag is just left out.
fn compositor() -> String {
    let t = crate::prefs::read();
    let pick = |key: &str| t.get(&format!("hypr_{key}")).cloned();
    let text = |key: &str, def: &str| pick(key).and_then(|v| v.as_str().map(str::to_string)).or_else(|| Some(def.to_string()));
    let num = |key: &str, def: f64| pick(key).and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64))).or(Some(def));
    let flag = |key: &str, def: bool| pick(key).and_then(|v| v.as_bool()).or(Some(def));
    // niri's own settings, with no Hyprland counterpart.
    let niri_text = |key: &str, def: &str| t.get(&format!("niri_{key}")).and_then(|v| v.as_str().map(str::to_string)).or_else(|| Some(def.to_string()));
    let niri_flag = |key: &str, def: bool| t.get(&format!("niri_{key}")).and_then(|v| v.as_bool()).or(Some(def));
    let mut out = String::new();
    let mut xkb = Vec::new();
    if let Some(v) = text("kb_layout", "us") {
        xkb.push(format!("layout {}", kdl_str(&v)));
    }
    if let Some(v) = text("kb_variant", "").filter(|v| !v.is_empty()) {
        xkb.push(format!("variant {}", kdl_str(&v)));
    }
    let (switch, caps) = (text("layout_switch", "none"), text("caps", "normal"));
    if switch.is_some() || caps.is_some() {
        let mut opts = Vec::new();
        match caps.as_deref() {
            Some("escape") => opts.push("caps:escape"),
            Some("ctrl") => opts.push("ctrl:nocaps"),
            Some("swapescape") => opts.push("caps:swapescape"),
            _ => {}
        }
        match switch.as_deref() {
            Some("alt_shift") => opts.push("grp:alt_shift_toggle"),
            Some("ctrl_shift") => opts.push("grp:ctrl_shift_toggle"),
            Some("win_space") => opts.push("grp:win_space_toggle"),
            Some("caps") => opts.push("grp:caps_toggle"),
            _ => {}
        }
        xkb.push(format!("options {}", kdl_str(&opts.join(","))));
    }
    let mut keyboard = Vec::new();
    if !xkb.is_empty() {
        keyboard.push(format!("xkb {{\n{}\n        }}", xkb.iter().map(|l| format!("            {l}")).collect::<Vec<_>>().join("\n")));
    }
    if let Some(v) = num("repeat_delay", 300.0) {
        keyboard.push(format!("repeat-delay {}", v.round() as i64));
    }
    if let Some(v) = num("repeat_rate", 30.0) {
        keyboard.push(format!("repeat-rate {}", v.round() as i64));
    }
    let speed = num("sensitivity", 0.0).map(|v| format!("accel-speed {:.2}", v.clamp(-1.0, 1.0)));
    let profile = text("accel", "adaptive").map(|v| format!("accel-profile {}", kdl_str(&v)));
    let mut touchpad: Vec<String> = Vec::new();
    if flag("tap", true) == Some(true) {
        touchpad.push("tap".into());
    }
    if niri_flag("dwt", true) == Some(true) {
        touchpad.push("dwt".into());
    }
    if let Some(sp) = niri_text("scroll", "normal") {
        touchpad.push(format!("scroll-factor {}", match sp.as_str() { "slow" => "0.5", "fast" => "1.5", _ => "1.0" }));
    }
    if flag("touch_natural", true) == Some(true) {
        touchpad.push("natural-scroll".into());
    }
    let mut mouse: Vec<String> = speed.iter().chain(profile.iter()).cloned().collect();
    if flag("mouse_natural", false) == Some(true) {
        mouse.push("natural-scroll".into());
    }
    touchpad.extend(speed.iter().chain(profile.iter()).cloned());
    let block = |name: &str, lines: &[String], indent: &str| {
        format!("{indent}{name} {{\n{}\n{indent}}}", lines.iter().map(|l| format!("{indent}    {l}")).collect::<Vec<_>>().join("\n"))
    };
    let mut input = Vec::new();
    if !keyboard.is_empty() {
        input.push(block("keyboard", &keyboard, "    "));
    }
    if !touchpad.is_empty() {
        input.push(block("touchpad", &touchpad, "    "));
    }
    if !mouse.is_empty() {
        input.push(block("mouse", &mouse, "    "));
    }
    if flag("follow_mouse", true) == Some(true) {
        input.push("    focus-follows-mouse".into());
    }
    if !input.is_empty() {
        out.push_str(&format!("input {{\n{}\n}}\n\n", input.join("\n")));
    }
    let mut layout = Vec::new();
    // niri has one gap, between windows and at the edges alike: twice Hyprland's inner gap.
    if let Some(v) = num("gaps_in", 4.0) {
        layout.push(format!("    gaps {}", (v * 2.0).round() as i64));
    }
    if let Some(w) = niri_text("column_width", "half") {
        let p = match w.as_str() {
            "third" => "0.33333",
            "two_thirds" => "0.66667",
            "full" => "1.0",
            _ => "0.5",
        };
        layout.push(format!("    default-column-width {{ proportion {p}; }}"));
    }
    if let Some(c) = niri_text("center", "never") {
        layout.push(format!("    center-focused-column {}", kdl_str(&c)));
    }
    if let Some(p) = niri_text("presets", "thirds") {
        let widths: &[&str] = if p == "halves" { &["0.5", "1.0"] } else { &["0.33333", "0.5", "0.66667"] };
        layout.push(format!("    preset-column-widths {{\n{}\n    }}", widths.iter().map(|w| format!("        proportion {w}")).collect::<Vec<_>>().join("\n")));
    }
    // The focused window is marked by a border or a ring around it, in digit's accent.
    let highlight = niri_text("highlight", "border");
    let width = num("border", 2.0);
    if highlight.is_some() || width.is_some() {
        let w = width.unwrap_or(2.0).round().max(1.0) as i64;
        let (active, inactive) = border_colors();
        // niri's border is off until a file turns it on, so say "on" explicitly.
        let on = |name: &str| format!("    {name} {{\n        on\n        width {w}\n        active-color {}\n        inactive-color {}\n    }}", kdl_str(&active), kdl_str(&inactive));
        let off = |name: &str| format!("    {name} {{\n        off\n    }}");
        match highlight.as_deref().unwrap_or("border") {
            _ if width.is_some_and(|v| v < 0.5) => {
                layout.push(off("border"));
                layout.push(off("focus-ring"));
            }
            "ring" => {
                layout.push(off("border"));
                layout.push(on("focus-ring"));
            }
            "none" => {
                layout.push(off("border"));
                layout.push(off("focus-ring"));
            }
            _ => {
                layout.push(on("border"));
                layout.push(off("focus-ring"));
            }
        }
    }
    // Reduced effects switch the compositor's shadows and blur off too.
    let reduced = crate::effects::reduced();
    let flag = |key: &str, def: bool| if reduced && (key == "shadow" || key == "blur") { Some(false) } else { flag(key, def) };
    if let Some(on) = flag("shadow", true) {
        let softness = match text("shadow_size", "medium").as_deref() {
            Some("soft") => 16,
            Some("large") => 48,
            _ => 30,
        };
        layout.push(if on {
            format!("    shadow {{\n        on\n        softness {softness}\n        spread 4\n        offset x=0 y=6\n        color \"#00000066\"\n    }}")
        } else {
            "    shadow {\n        off\n    }".into()
        });
    }
    if !layout.is_empty() {
        out.push_str(&format!("layout {{\n{}\n}}\n\n", layout.join("\n")));
    }
    let mut cursor = Vec::new();
    if let Some(size) = niri_text("cursor_size", "small") {
        cursor.push(format!("    xcursor-size {}", match size.as_str() { "large" => 48, "medium" => 32, _ => 24 }));
    }
    if niri_flag("hide_cursor", false) == Some(true) {
        cursor.push("    hide-when-typing".into());
    }
    if !cursor.is_empty() {
        out.push_str(&format!("cursor {{\n{}\n}}\n\n", cursor.join("\n")));
    }
    if let Some(z) = niri_text("overview_zoom", "medium") {
        let zoom = match z.as_str() { "small" => "0.35", "large" => "0.65", _ => "0.5" };
        out.push_str(&format!("overview {{\n    zoom {zoom}\n}}\n\n"));
    }
    match niri_text("motion", "normal").as_deref() {
        Some("slow") => out.push_str("animations {\n    slowdown 2.0\n}\n\n"),
        Some("off") => out.push_str("animations {\n    off\n}\n\n"),
        _ => {}
    }
    if flag("blur", false) == Some(true) {
        let (passes, offset) = match text("blur_strength", "medium").as_deref() {
            Some("light") => (2, 2),
            Some("strong") => (4, 5),
            _ => (3, 3),
        };
        out.push_str(&format!("blur {{\n    passes {passes}\n    offset {offset}\n}}\n\nwindow-rule {{\n    background-effect {{\n        blur true\n    }}\n}}\n"));
    }
    out
}

/// digit's accent as a color: the one picked in Settings, else the wallpaper's.
pub(crate) fn accent() -> String {
    let picked = crate::prefs::text(&crate::prefs::read(), "ui_accent", "wallpaper");
    if picked.starts_with('#') && picked.len() == 7 {
        return picked;
    }
    crate::theme::load().map(|t| t.hex("primary")).filter(|h| h.starts_with('#')).unwrap_or_else(|| "#d0bcff".into())
}

/// The focused window's border: digit's accent, or the plain grey of digit's default look.
fn border_colors() -> (String, String) {
    if crate::prefs::text(&crate::prefs::read(), "border_color", "accent") == "default" {
        ("#595959".into(), "#5959591a".into())
    } else {
        (accent(), "#ffffff14".into())
    }
}

fn decorations() -> String {
    fill(DECORATIONS, &[("rounding", &rounding().to_string())])
}

/// Rewrites digit's niri folder from digit's settings. niri notices the change by itself.
pub fn sync() -> Result<bool, String> {
    static WRITING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    if !installed() {
        return Ok(false);
    }
    let d = dir();
    let e = |e: std::io::Error| format!("Could not write {}: {e}", d.display());
    let binds = keybinds::effective();
    let mut changed = false;
    changed |= write_if_changed(&d.join("init.kdl"), INIT).map_err(e)?;
    changed |= write_if_changed(&d.join("digit-base.kdl"), BASE).map_err(e)?;
    // niri wants one layout or input block per file, so the settings get their own.
    let settings = format!("// Settings > Compositor. Written by digit; change them in digit Settings.\n// niri can't switch a setting like tap-to-click off from here; turn those off in config.kdl.\n\n{}", compositor());
    changed |= write_if_changed(&d.join("digit-compositor.kdl"), &settings).map_err(e)?;
    changed |= write_if_changed(&d.join("digit-decorations.kdl"), &decorations()).map_err(e)?;
    changed |= write_if_changed(&d.join("digit-rules.kdl"), RULES).map_err(e)?;
    changed |= write_if_changed(&d.join("digit-keybinds.kdl"), &render_keybinds(&binds)).map_err(e)?;
    changed |= write_if_changed(&d.join("digit-autostart.kdl"), &fill(AUTOSTART, &[("digit", &kdl_str(&bin().join("digit").display().to_string()))])).map_err(e)?;
    Ok(changed)
}

/// Sets digit up on niri. Returns a line saying what happened, or why it couldn't.
/// Asks niri whether the config parses, when niri is installed.
fn check(main: &std::path::Path) -> Result<(), String> {
    if gtk::glib::find_program_in_path("niri").is_none() {
        return Ok(());
    }
    match crate::hyprconf::run_for(&["niri", "validate", "-c", &main.display().to_string()], 30) {
        Some((false, out)) => Err(format!("niri didn't accept the new config, so nothing was changed:\n{}", out.lines().filter(|l| !l.contains("DEBUG") && !l.trim().is_empty()).take(8).collect::<Vec<_>>().join("\n"))),
        _ => Ok(()),
    }
}

pub fn setup() -> Result<String, String> {
    let main = niri().join("config.kdl");
    if installed() && dir().exists() {
        sync()?;
        check(&main)?;
        return Ok("Already set up; digit's files are up to date".into());
    }
    crate::hyprconf::guarded(&niri(), "niri_setup", setup_now, || check(&main))
}

fn setup_now() -> Result<String, String> {
    let niri = niri();
    let main = niri.join("config.kdl");
    let mut kept = None;
    if niri.exists() {
        let mut backup = niri.with_file_name(format!("niri.backup-{}", stamp()));
        let mut n = 1;
        while backup.exists() {
            backup = niri.with_file_name(format!("niri.backup-{}-{n}", stamp()));
            n += 1;
        }
        copy_dir(&niri, &backup).map_err(|e| format!("Could not back up {}: {e}", niri.display()))?;
        std::fs::remove_dir_all(&niri).map_err(|e| format!("Could not clear {}: {e}", niri.display()))?;
        kept = Some(backup);
    }
    std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    std::fs::write(&main, MAIN).map_err(|e| e.to_string())?;
    crate::prefs::set("niri_setup", toml::Value::String("full".into()))?;
    sync()?;
    Ok(match kept {
        Some(b) => format!("Set up; your old config is in {}", b.display()),
        None => "Set up".into(),
    })
}

/// Removes digit's part: the folder and the include line. The rest of the config stays.
pub fn remove() -> Result<(), String> {
    let main = niri().join("config.kdl");
    if let Ok(text) = std::fs::read_to_string(&main) {
        let kept: Vec<&str> = text.lines().filter(|l| l.trim() != INCLUDE && l.trim() != MARK).collect();
        std::fs::write(&main, format!("{}\n", kept.join("\n").trim_end())).map_err(|e| e.to_string())?;
    }
    let _ = std::fs::remove_dir_all(dir());
    Ok(())
}
