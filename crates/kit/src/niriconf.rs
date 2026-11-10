//! digit's part of the niri config: ~/.config/digit/niri/digit/, included by one line in
//! config.kdl. It is the niri twin of `hyprconf`: the same shortcuts, Compositor settings and
//! Corners, written as KDL. Settings niri has no counterpart for (tiling layouts, sideways
//! workspaces, sliding windows) are left out.

use crate::hyprconf::{bin, fill, rounding, terminal, write_if_changed};
use crate::keybinds::{self, Act, Bind, Dir, Win};
use std::path::PathBuf;

const INIT: &str = include_str!("../../../data/niri/init.kdl");
const MAIN: &str = include_str!("../../../data/niri/config.kdl");
const BASE: &str = include_str!("../../../data/niri/digit-base.kdl");
const DECORATIONS: &str = include_str!("../../../data/niri/digit-decorations.kdl");
const RULES: &str = include_str!("../../../data/niri/digit-rules.kdl");
const AUTOSTART: &str = include_str!("../../../data/niri/digit-autostart.kdl");

pub fn niri() -> PathBuf {
    gtk::glib::user_config_dir().join("digit/niri")
}

pub fn dir() -> PathBuf {
    niri().join("digit")
}

/// Whether this is the niri with Digit session.
pub fn installed() -> bool {
    std::env::var("DIGIT_SESSION").as_deref() == Ok("niri")
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
        // niri has no per-workspace layouts.
        Act::Ctl(args) if args == "key workspace-layout" => return None,
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
            Win::Focus(Dir::Left) => "focus-column-or-monitor-left;".into(),
            Win::Focus(Dir::Right) => "focus-column-or-monitor-right;".into(),
            Win::Focus(Dir::Up) => "focus-window-or-workspace-up;".into(),
            Win::Focus(Dir::Down) => "focus-window-or-workspace-down;".into(),
            Win::StepWindow(i) if *i > 0 => "focus-window-down-or-column-right;".into(),
            Win::StepWindow(_) => "focus-window-up-or-column-left;".into(),
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
            // windows, minimization, pseudotiling or scratchpad.
            Win::Drag | Win::Resize | Win::Snap(_) | Win::ToMonitor(_) | Win::Pin | Win::Minimize | Win::Pseudo | Win::Scratchpad | Win::ToScratchpad => return None,
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
         // Changes here are replaced: change shortcuts in digit Settings, or add your own in user.kdl.\n\n\
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
        if b.keys.trim().is_empty() { continue; }
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
        if b.keys.split('+').any(|k| k.trim().to_ascii_lowercase().starts_with("mouse_")) {
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
    crate::motion::refresh();
    crate::effects::invalidate();
    compositor_with(&crate::prefs::read(), crate::motion::reduced(), crate::effects::reduced())
}

fn compositor_with(t: &toml::Table, motion: bool, reduced: bool) -> String {
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
    let mut cursor = crate::cursors::niri(t);
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
    match if motion { Some("off".to_string()) } else { niri_text("motion", "normal") }.as_deref() {
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

// niri uses the FIRST matching output block, rather than merging duplicate blocks.
// Keep startup focus in the same Settings-owned block as the output's mode and position.
fn primary(prefs: &toml::Table, outputs: &str) -> String {
    let name = crate::prefs::text(prefs, "primary_monitor", "");
    let header = format!("output {} {{", kdl_str(&name));
    let mut text = String::new();
    let mut found = false;
    for line in outputs.lines() {
        if line.trim() == "focus-at-startup" { continue; }
        text.push_str(line);
        text.push('\n');
        if !name.is_empty() && line.trim() == header {
            text.push_str("    focus-at-startup\n");
            found = true;
        }
    }
    if !name.is_empty() && !found {
        text.push_str(&format!("output {} {{\n    focus-at-startup\n}}\n", kdl_str(&name)));
    }
    text
}

/// Rewrites digit's niri folder from digit's settings. niri notices the change by itself.
pub fn sync() -> Result<bool, String> {
    if !installed() {
        return Ok(false);
    }
    generate()
}

fn generate() -> Result<bool, String> {
    static WRITING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    let d = dir();
    let mut changed = crate::setup::entry(&niri(), "config.kdl", MAIN, "user.kdl", "//")?;
    let e = |e: std::io::Error| format!("Could not write {}: {e}", d.display());
    let binds = keybinds::effective();
    changed |= write_if_changed(&d.join("init.kdl"), INIT).map_err(e)?;
    let enabled = crate::hyprconf::nvidia_env();
    let gpu = crate::hyprconf::NVIDIA_ENV.iter().filter(|_| enabled).map(|(k, v)| format!("    {k} \"{v}\"\n")).collect::<String>();
    changed |= write_if_changed(&d.join("digit-base.kdl"), &fill(BASE, &[("gpu", &gpu)])).map_err(e)?;
    // niri wants one layout or input block per file, so the settings get their own.
    let settings = format!("// Settings > Compositor. Written by digit; change them in digit Settings.\n// niri can't switch a setting like tap-to-click off from here; turn those off in user.kdl.\n\n{}", compositor());
    changed |= write_if_changed(&d.join("digit-compositor.kdl"), &settings).map_err(e)?;
    changed |= write_if_changed(&d.join("digit-decorations.kdl"), &decorations()).map_err(e)?;
    let outputs_path = niri().join("digit-outputs.kdl");
    let outputs = match std::fs::read_to_string(&outputs_path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(e(error)),
    };
    changed |= write_if_changed(&outputs_path, &primary(&crate::prefs::read(), &outputs)).map_err(e)?;
    changed |= write_if_changed(&d.join("digit-rules.kdl"), RULES).map_err(e)?;
    changed |= write_if_changed(&d.join("digit-keybinds.kdl"), &render_keybinds(&binds)).map_err(e)?;
    changed |= write_if_changed(&d.join("digit-autostart.kdl"), &fill(AUTOSTART, &[("digit", &kdl_str(&bin().join("digit").display().to_string()))])).map_err(e)?;
    Ok(changed)
}

/// Prepares the session without a display.
pub fn prepare() -> Result<(), String> {
    generate().map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduced_motion_disables_niri_animations_without_changing_stored_choices() {
        for choice in ["normal", "slow", "off"] {
            let t: toml::Table = format!("reduce_motion = true\nniri_motion = '{choice}'").parse().unwrap();
            let stored = toml::to_string(&t).unwrap();
            let rendered = compositor_with(&t, crate::effects::effective(&t, "reduce_motion", false), false);
            assert!(rendered.contains("animations {\n    off\n}"));
            assert!(!rendered.contains("slowdown"));
            let restored = compositor_with(&t, false, false);
            match choice {
                "normal" => assert!(!restored.contains("animations {")),
                "slow" => assert!(restored.contains("animations {\n    slowdown 2.0\n}")),
                _ => assert!(restored.contains("animations {\n    off\n}")),
            }
            assert_eq!(toml::to_string(&t).unwrap(), stored, "config.toml choices stay stored");
        }
    }

    #[test]
    fn effects_never_change_niri_motion() {
        let t = "niri_motion = 'slow'\nhypr_blur = true\nhypr_shadow = true".parse().unwrap();
        for motion in [false, true] {
            let normal = compositor_with(&t, motion, false);
            let reduced = compositor_with(&t, motion, true);
            let animations = if motion { "animations {\n    off\n}" } else { "animations {\n    slowdown 2.0\n}" };
            assert!(normal.contains(animations));
            assert!(reduced.contains(animations));
            assert!(normal.contains("shadow {\n        on"));
            assert!(reduced.contains("shadow {\n        off"));
            assert!(normal.contains("blur true"));
            assert!(!reduced.contains("blur true"));
        }
    }

    #[test]
    fn primary_focus_preserves_geometry_and_never_duplicates_output_blocks() {
        let prefs = "primary_monitor = 'DP-1'".parse().unwrap();
        let outputs = "output \"DP-1\" {\n    mode \"2560x1440@144\"\n    scale 1.25\n    position x=1920 y=0\n}\noutput \"eDP-1\" {\n    focus-at-startup\n    off\n}\n";
        let rendered = primary(&prefs, outputs);
        assert_eq!(rendered.matches("output \"DP-1\"").count(), 1);
        assert_eq!(rendered.matches("focus-at-startup").count(), 1);
        assert!(rendered.starts_with("output \"DP-1\" {\n    focus-at-startup\n    mode"));
        assert!(rendered.contains("scale 1.25\n    position x=1920 y=0"));
        assert!(rendered.contains("output \"eDP-1\" {\n    off\n}"));
        assert_eq!(primary(&prefs, &rendered), rendered);
        assert!(!primary(&toml::Table::new(), &rendered).contains("focus-at-startup"));
    }

    #[test]
    fn primary_focus_can_precede_the_first_display_layout_and_keeps_unplugged_outputs() {
        let prefs = "primary_monitor = 'HDMI-A-1'".parse().unwrap();
        let rendered = primary(&prefs, "output \"DP-1\" {\n    off\n}\n");
        assert!(rendered.contains("output \"HDMI-A-1\" {\n    focus-at-startup\n}"));
        assert!(rendered.contains("output \"DP-1\" {\n    off\n}"));
        assert_eq!(primary(&prefs, &rendered), rendered);
    }

    #[test]
    fn primary_output_names_are_kdl_strings() {
        let mut prefs = toml::Table::new();
        prefs.insert("primary_monitor".into(), toml::Value::String("DP-\"1".into()));
        assert!(primary(&prefs, "").contains("output \"DP-\\\"1\" {"));
    }

    #[test]
    fn wheel_names_convert_to_niri_scroll_triggers() {
        assert_eq!(keys("SUPER + mouse_up"), "Super+WheelScrollUp");
        assert_eq!(keys("SUPER + SHIFT + mouse_down"), "Super+Shift+WheelScrollDown");
    }

    #[test]
    fn wheel_focus_actions_use_native_niri_focus_and_cooldown() {
        let binds: Vec<_> = keybinds::defaults().into_iter().filter(|b| b.id.starts_with("focus-scroll-") || b.id.starts_with("window-")).collect();
        let rendered = render_keybinds(&binds);
        assert!(rendered.contains("Super+WheelScrollUp cooldown-ms=150 hotkey-overlay-title=\"Focus above\" { focus-window-or-workspace-up; }"));
        assert!(rendered.contains("Super+WheelScrollDown cooldown-ms=150 hotkey-overlay-title=\"Focus below\" { focus-window-or-workspace-down; }"));
        assert!(rendered.contains("Super+Shift+WheelScrollDown cooldown-ms=150 hotkey-overlay-title=\"Next window\" { focus-window-down-or-column-right; }"));
        assert!(rendered.contains("Super+Shift+WheelScrollUp cooldown-ms=150 hotkey-overlay-title=\"Previous window\" { focus-window-up-or-column-left; }"));
    }

    #[test]
    fn focus_left_and_right_include_monitor_fallbacks() {
        let binds: Vec<_> = keybinds::defaults().into_iter().filter(|b| b.id == "focus-left" || b.id == "focus-right").collect();
        let rendered = render_keybinds(&binds);
        assert!(rendered.contains("focus-column-or-monitor-left;"));
        assert!(rendered.contains("focus-column-or-monitor-right;"));
    }

}
