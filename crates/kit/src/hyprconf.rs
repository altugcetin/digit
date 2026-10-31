//! digit's part of the Hyprland config: ~/.config/hypr/digit/, loaded by one `require("digit")`
//! in hyprland.lua. digit owns everything in that folder and regenerates it from its own
//! settings; hyprland.lua stays the user's.
//!
//! A full setup moves the old ~/.config/hypr aside and starts from digit's config, window
//! shortcuts included. A minimal setup only adds the folder and the require line, so the
//! user's own config keeps working and digit's shortcuts win where they overlap.

use crate::keybinds::{self, Act, Bind, Win};
use std::path::{Path, PathBuf};

pub const REQUIRE: &str = "require(\"digit\")";
const MARK: &str = "-- digit: its part of the config lives in ~/.config/hypr/digit (added by digit setup)";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    Full,
    Minimal,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Full => "full",
            Mode::Minimal => "minimal",
        }
    }

    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "full" => Some(Mode::Full),
            "minimal" => Some(Mode::Minimal),
            _ => None,
        }
    }
}

pub fn hypr() -> PathBuf {
    gtk::glib::user_config_dir().join("hypr")
}

pub fn dir() -> PathBuf {
    hypr().join("digit")
}

/// The setup chosen last, if digit has been set up on Hyprland.
pub fn mode() -> Option<Mode> {
    Mode::parse(&crate::prefs::text(&crate::prefs::read(), "hypr_setup", ""))
}

/// Where digit's programs live, so the config can start them by full path.
fn bin() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_else(|| gtk::glib::home_dir().join(".local/bin"))
}

fn lua_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
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

fn terminal() -> String {
    if let Ok(t) = std::env::var("TERMINAL") {
        if !t.trim().is_empty() {
            return t;
        }
    }
    for t in ["xdg-terminal-exec", "kitty", "foot", "alacritty", "wezterm", "ghostty", "konsole", "gnome-terminal"] {
        if gtk::glib::find_program_in_path(t).is_some() {
            return t.into();
        }
    }
    "kitty".into()
}

/// Left and right mean columns in the scrolling layout: the same keys focus or swap whole
/// columns there, and act on windows everywhere else. Up and down stay as they are.
fn scrolling(d: keybinds::Dir, tiled: String, column: &str) -> String {
    let side = match d {
        keybinds::Dir::Left => "l",
        keybinds::Dir::Right => "r",
        _ => return tiled,
    };
    format!("per_layout({{ default = {tiled}, scrolling = hl.dsp.layout(\"{column} {side}\") }})")
}

fn action(b: &Bind) -> String {
    let bin = bin();
    let exec = |cmd: String| format!("hl.dsp.exec_cmd({})", lua_str(&cmd));
    match &b.act {
        Act::Ctl(args) => exec(format!("{} {args}", bin.join("digitctl").display())),
        Act::Run(cmd) => exec(format!("{}", bin.join(cmd).display())),
        Act::Shell(cmd) => exec(cmd.clone()),
        Act::Terminal => exec(terminal()),
        Act::Window(w) => match w {
            Win::Close => "hl.dsp.window.close()".into(),
            Win::Float => "hl.dsp.window.float({ action = \"toggle\" })".into(),
            Win::Maximize => "hl.dsp.window.fullscreen({ mode = 1 })".into(),
            Win::Fullscreen => "hl.dsp.window.fullscreen({ mode = 0 })".into(),
            Win::Focus(d) => scrolling(*d, format!("hl.dsp.focus({{ direction = \"{}\" }})", d.name()), "focus"),
            Win::Move(d) => scrolling(*d, format!("hl.dsp.window.move({{ direction = \"{}\" }})", d.name()), "swapcol"),
            Win::Workspace(i) => format!("hl.dsp.focus({{ workspace = {i} }})"),
            Win::ToWorkspace(i) => format!("hl.dsp.window.move({{ workspace = {i} }})"),
            Win::StepWorkspace(i) => format!("hl.dsp.focus({{ workspace = \"e{i:+}\" }})"),
            Win::Drag => "hl.dsp.window.drag()".into(),
            Win::Resize => "hl.dsp.window.resize()".into(),
            Win::Grow(d) => {
                let (x, y) = match d {
                    keybinds::Dir::Left => (-40, 0),
                    keybinds::Dir::Right => (40, 0),
                    keybinds::Dir::Up => (0, -40),
                    keybinds::Dir::Down => (0, 40),
                };
                let tiled = format!("hl.dsp.window.resize({{ x = {x}, y = {y}, relative = true }})");
                match d {
                    // Scrolling columns grow and shrink by a share of the screen instead.
                    keybinds::Dir::Left => format!("per_layout({{ default = {tiled}, scrolling = hl.dsp.layout(\"colresize -0.1\") }})"),
                    keybinds::Dir::Right => format!("per_layout({{ default = {tiled}, scrolling = hl.dsp.layout(\"colresize +0.1\") }})"),
                    _ => tiled,
                }
            }
            Win::Pin => "hl.dsp.window.pin()".into(),
            Win::Group => "hl.dsp.group.toggle()".into(),
            Win::Scratchpad => "hl.dsp.workspace.toggle_special(\"scratchpad\")".into(),
            Win::ToScratchpad => "hl.dsp.window.move({ workspace = \"special:scratchpad\" })".into(),
        },
    }
}

/// digit-keybinds.lua: every shortcut in effect, each unbinding its keys first so digit's
/// binding replaces whatever the user's config put there.
pub fn render_keybinds(binds: &[Bind], conflicts: &[(String, String)]) -> String {
    let mut out = String::from(
        "-- Generated by digit from its defaults and ~/.config/digit/keybinds.toml.\n\
         -- Changes here are replaced: change shortcuts in digit Settings, or add your own in hyprland.lua.\n\n\
         local function bind(keys, action, opts)\n    hl.unbind(keys)\n    hl.bind(keys, action, opts)\nend\n\n\
         -- Picks the action for the layout of the workspace in front when the keys are pressed,\n\
         -- so one shortcut can mean different things in dwindle and scrolling.\n\
         local function per_layout(actions)\n    return function()\n        local ws = hl.get_active_special_workspace() or hl.get_active_workspace()\n        local action = ws and actions[ws.tiled_layout] or actions.default\n        if action then\n            hl.dispatch(action)\n        end\n    end\nend\n\n\
         -- Settings lets keys through to itself while you record a new shortcut; Escape leaves.\n\
         hl.define_submap(\"digit-capture\", function()\n    hl.bind(\"ESCAPE\", hl.dsp.submap(\"reset\"))\nend)\n\n",
    );
    if !conflicts.is_empty() {
        out.push_str("-- Also bound in your own config; digit's binding below wins:\n");
        for (k, f) in conflicts {
            out.push_str(&format!("--   {k} ({f})\n"));
        }
        out.push('\n');
    }
    let mut group = "";
    for b in binds {
        if b.group != group {
            group = b.group;
            out.push_str(&format!("-- {group}\n"));
        }
        let mut opts = Vec::new();
        if b.opts.locked {
            opts.push("locked = true");
        }
        if b.opts.repeating {
            opts.push("repeating = true");
        }
        if b.opts.mouse {
            opts.push("mouse = true");
        }
        let opts = if opts.is_empty() { String::new() } else { format!(", {{ {} }}", opts.join(", ")) };
        out.push_str(&format!("bind({}, {}{opts}) -- {}\n", lua_str(&b.keys), action(b), b.label));
    }
    out
}

/// The default files, kept as plain Lua in data/hypr. `{{name}}` marks a value filled in when
/// the file is written.
const INIT: &str = include_str!("../../../data/hypr/init.lua");
const AUTOSTART: &str = include_str!("../../../data/hypr/digit-autostart.lua");
const RULES: &str = include_str!("../../../data/hypr/digit-rules.lua");
const BASE: &str = include_str!("../../../data/hypr/digit-base.lua");
const DECORATIONS: &str = include_str!("../../../data/hypr/digit-decorations.lua");
const WORKSPACES: &str = include_str!("../../../data/hypr/digit-workspaces.lua");
const MONITORS: &str = include_str!("../../../data/hypr/digit-monitors.lua");
const MAIN: &str = include_str!("../../../data/hypr/hyprland.lua");
const BASE_MINIMAL: &str = include_str!("../../../data/hypr/minimal/digit-base.lua");
const DECORATIONS_MINIMAL: &str = include_str!("../../../data/hypr/minimal/digit-decorations.lua");

fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (k, v) in values {
        out = out.replace(&format!("{{{{{k}}}}}"), v);
    }
    out
}

fn autostart() -> String {
    fill(AUTOSTART, &[("digit", &lua_str(&bin().join("digit").display().to_string()))])
}

/// Corner radius for windows, following Settings > Desktop > Corners like the rest of digit.
fn rounding() -> i64 {
    let round = crate::prefs::number(&crate::prefs::read(), "ui_round", 1.0).clamp(0.1, 1.8);
    (14.0 * round).round() as i64
}


/// digit-base.lua: what a full setup needs to be usable on its own: input, layouts, gestures,
/// environment. A minimal setup leaves all of it to the user's config.
fn base(mode: Mode) -> String {
    let template = if mode == Mode::Minimal { BASE_MINIMAL } else { BASE };
    fill(template, &[("settings", &compositor(mode))])
}

/// Settings > Compositor, as Lua. A full setup writes every value, with digit's defaults for
/// the ones never changed; a minimal one writes only what was changed, so the rest of the
/// user's own config keeps deciding.
fn compositor(mode: Mode) -> String {
    let t = crate::prefs::read();
    let full = mode == Mode::Full;
    let pick = |key: &str| t.get(&format!("hypr_{key}")).cloned();
    let text = |key: &str, def: &str| pick(key).and_then(|v| v.as_str().map(str::to_string)).or_else(|| full.then(|| def.to_string()));
    let num = |key: &str, def: f64| pick(key).and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64))).or_else(|| full.then_some(def));
    let flag = |key: &str, def: bool| pick(key).and_then(|v| v.as_bool()).or_else(|| full.then_some(def));
    let int = |v: f64| format!("{}", v.round() as i64);
    let mut input = Vec::new();
    let mut touch = Vec::new();
    let mut general = Vec::new();
    if let Some(v) = text("kb_layout", "us") {
        input.push(format!("kb_layout = {}", lua_str(&v)));
    }
    if let Some(v) = text("kb_variant", "") {
        input.push(format!("kb_variant = {}", lua_str(&v)));
    }
    let switch = text("layout_switch", "none");
    let caps = text("caps", "normal");
    if switch.is_some() || caps.is_some() {
        // A minimal setup keeps the user's other options; only the part changed is replaced.
        let mut opts: Vec<String> = if full {
            Vec::new()
        } else {
            live_option("input:kb_options")
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|o| !o.is_empty())
                .filter(|o| !(caps.is_some() && (o.starts_with("caps:") || *o == "ctrl:nocaps")) && !(switch.is_some() && o.starts_with("grp:")))
                .map(str::to_string)
                .collect()
        };
        let caps = match caps.as_deref() {
            Some("escape") => Some("caps:escape"),
            Some("ctrl") => Some("ctrl:nocaps"),
            Some("swapescape") => Some("caps:swapescape"),
            _ => None,
        };
        let switch = match switch.as_deref() {
            Some("alt_shift") => Some("grp:alt_shift_toggle"),
            Some("ctrl_shift") => Some("grp:ctrl_shift_toggle"),
            Some("win_space") => Some("grp:win_space_toggle"),
            Some("caps") => Some("grp:caps_toggle"),
            _ => None,
        };
        opts.extend(caps.into_iter().chain(switch).map(str::to_string));
        input.push(format!("kb_options = {}", lua_str(&opts.join(","))));
    }
    if let Some(v) = num("repeat_delay", 300.0) {
        input.push(format!("repeat_delay = {}", int(v)));
    }
    if let Some(v) = num("repeat_rate", 30.0) {
        input.push(format!("repeat_rate = {}", int(v)));
    }
    if let Some(v) = num("sensitivity", 0.0) {
        input.push(format!("sensitivity = {:.2}", v.clamp(-1.0, 1.0)));
    }
    if let Some(v) = text("accel", "adaptive") {
        input.push(format!("accel_profile = {}", lua_str(&v)));
    }
    if let Some(v) = flag("mouse_natural", false) {
        input.push(format!("natural_scroll = {v}"));
    }
    if let Some(v) = flag("follow_mouse", true) {
        input.push(format!("follow_mouse = {}", if v { 1 } else { 0 }));
    }
    if let Some(v) = flag("touch_natural", true) {
        touch.push(format!("natural_scroll = {v}"));
    }
    if let Some(v) = flag("tap", true) {
        touch.push(format!("tap_to_click = {v}"));
    }
    if !touch.is_empty() {
        input.push(format!("touchpad = {{ {} }}", touch.join(", ")));
    }
    if let Some(v) = text("tiling", "dwindle") {
        general.push(format!("layout = {}", lua_str(&v)));
    }
    if let Some(v) = num("gaps_in", 4.0) {
        general.push(format!("gaps_in = {}", int(v)));
    }
    if let Some(v) = num("gaps_out", 8.0) {
        general.push(format!("gaps_out = {}", int(v)));
    }
    if let Some(v) = num("border", 2.0) {
        general.push(format!("border_size = {}", int(v)));
    }
    let mut out = String::from("-- Settings > Compositor\n");
    let mut sections = Vec::new();
    if !input.is_empty() {
        sections.push(format!("    input = {{\n{}\n    }},", input.iter().map(|l| format!("        {l},")).collect::<Vec<_>>().join("\n")));
    }
    if !general.is_empty() {
        sections.push(format!("    general = {{\n{}\n    }},", general.iter().map(|l| format!("        {l},")).collect::<Vec<_>>().join("\n")));
    }
    if !sections.is_empty() {
        out.push_str(&format!("hl.config({{\n{}\n}})\n", sections.join("\n")));
    }
    if flag("gesture", true) == Some(true) {
        out.push_str("-- Three fingers sideways switch workspaces.\nhl.gesture({ fingers = 3, direction = \"horizontal\", action = \"workspace\" })\n");
    }
    out
}

/// digit-decorations.lua: gaps, borders, corners, shadows and motion of windows. The corner
/// radius follows digit's Corners setting.
fn decorations(mode: Mode) -> String {
    if mode == Mode::Minimal {
        return DECORATIONS_MINIMAL.into();
    }
    fill(DECORATIONS, &[("rounding", &rounding().to_string())])
}

fn write_if_changed(path: &Path, body: &str) -> std::io::Result<bool> {
    if std::fs::read_to_string(path).is_ok_and(|old| old == body) {
        return Ok(false);
    }
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("lua.new");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path)?;
    Ok(true)
}

/// Rewrites digit's folder from digit's settings. Displays and workspaces, which Settings
/// writes itself, are only created when missing. Returns whether anything changed.
pub fn sync() -> Result<bool, String> {
    let Some(mode) = mode() else { return Ok(false) };
    let d = dir();
    let e = |e: std::io::Error| format!("Could not write {}: {e}", d.display());
    let binds = keybinds::effective(mode == Mode::Full);
    let conflicts = if mode == Mode::Minimal { keybinds::conflicts(&binds) } else { Vec::new() };
    let mut changed = false;
    changed |= write_if_changed(&d.join("init.lua"), INIT).map_err(e)?;
    changed |= write_if_changed(&d.join("digit-autostart.lua"), &autostart()).map_err(e)?;
    changed |= write_if_changed(&d.join("digit-rules.lua"), RULES).map_err(e)?;
    changed |= write_if_changed(&d.join("digit-keybinds.lua"), &render_keybinds(&binds, &conflicts)).map_err(e)?;
    changed |= write_if_changed(&d.join("digit-base.lua"), &base(mode)).map_err(e)?;
    changed |= write_if_changed(&d.join("digit-decorations.lua"), &decorations(mode)).map_err(e)?;
    for (name, body) in [
        ("digit-workspaces.lua", WORKSPACES),
        ("digit-monitors.lua", MONITORS),
    ] {
        let p = d.join(name);
        if !p.exists() {
            changed |= write_if_changed(&p, body).map_err(e)?;
        }
    }
    Ok(changed)
}

/// Shortcuts in effect for the current setup and the ones the user's config binds too.
pub fn shortcuts() -> (Vec<Bind>, Vec<(String, String)>) {
    let mode = mode();
    let binds = keybinds::effective(mode == Some(Mode::Full));
    let conflicts = if mode == Some(Mode::Minimal) { keybinds::conflicts(&binds) } else { Vec::new() };
    (binds, conflicts)
}

fn stamp() -> String {
    gtk::glib::DateTime::now_local().ok().and_then(|t| t.format("%Y%m%d-%H%M%S").ok()).map(|s| s.to_string()).unwrap_or_else(|| "backup".into())
}

fn run(argv: &[&str]) -> Option<String> {
    std::process::Command::new(argv[0]).args(&argv[1..]).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
}

/// A text option as Hyprland runs it now.
fn live_option(name: &str) -> Option<String> {
    run(&["hyprctl", "-j", "getoption", name])
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("str").and_then(|s| s.as_str()).map(str::to_string))
        .filter(|s| s != "[[EMPTY]]")
}

/// The displays as Hyprland runs them now, so a full setup keeps the screen arrangement.
fn current_monitors() -> Option<String> {
    let text = run(&["hyprctl", "-j", "monitors"])?;
    let list: Vec<serde_json::Value> = serde_json::from_str(&text).ok()?;
    let mut out = String::from(MONITORS);
    for m in list {
        let name = m.get("name")?.as_str()?;
        let (w, h) = (m.get("width")?.as_i64()?, m.get("height")?.as_i64()?);
        let rate = m.get("refreshRate").and_then(|r| r.as_f64()).unwrap_or(60.0);
        let (x, y) = (m.get("x")?.as_i64()?, m.get("y")?.as_i64()?);
        let scale = m.get("scale").and_then(|s| s.as_f64()).unwrap_or(1.0);
        out.push_str(&format!("hl.monitor({{\n    output = {},\n    mode = \"{w}x{h}@{rate:.3}\",\n    position = \"{x}x{y}\",\n    scale = {scale}\n}})\n", lua_str(name)));
    }
    Some(out)
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let target = to.join(e.file_name());
        let kind = e.file_type()?;
        if kind.is_dir() {
            copy_dir(&e.path(), &target)?;
        } else if kind.is_symlink() {
            let link = std::fs::read_link(e.path())?;
            std::os::unix::fs::symlink(link, target)?;
        } else {
            std::fs::copy(e.path(), target)?;
        }
    }
    Ok(())
}

/// Sets digit up on Hyprland. Returns a line saying what happened, or why it couldn't.
pub fn setup(mode: Mode) -> Result<String, String> {
    let hypr = hypr();
    let main = hypr.join("hyprland.lua");
    let note = match mode {
        Mode::Full => {
            // Keep a copy of the old config, never over an earlier one.
            let mut kept = None;
            if hypr.exists() {
                let mut backup = hypr.with_file_name(format!("hypr.backup-{}", stamp()));
                let mut n = 1;
                while backup.exists() {
                    backup = hypr.with_file_name(format!("hypr.backup-{}-{n}", stamp()));
                    n += 1;
                }
                copy_dir(&hypr, &backup).map_err(|e| format!("Could not back up {}: {e}", hypr.display()))?;
                kept = Some(backup);
            }
            let monitors = current_monitors();
            if hypr.exists() {
                std::fs::remove_dir_all(&hypr).map_err(|e| format!("Could not clear {}: {e}", hypr.display()))?;
            }
            std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
            let body = MAIN.to_string();
            std::fs::write(&main, body).map_err(|e| e.to_string())?;
            if let Some(m) = monitors {
                std::fs::write(dir().join("digit-monitors.lua"), m).map_err(|e| e.to_string())?;
            }
            match kept {
                Some(b) => format!("Full setup done; your old config is in {}", b.display()),
                None => "Full setup done".into(),
            }
        }
        Mode::Minimal => {
            if !main.exists() {
                if hypr.join("hyprland.conf").exists() {
                    return Err("Your Hyprland config is hyprland.conf; a minimal setup needs hyprland.lua. A full setup backs up your config and starts fresh.".into());
                }
                std::fs::create_dir_all(&hypr).map_err(|e| e.to_string())?;
                std::fs::write(&main, "").map_err(|e| e.to_string())?;
            }
            let current = std::fs::read_to_string(&main).map_err(|e| e.to_string())?;
            if !current.lines().any(|l| l.trim() == REQUIRE) {
                std::fs::copy(&main, hypr.join(format!("hyprland.lua.bak-{}", stamp()))).map_err(|e| format!("Could not back up hyprland.lua: {e}"))?;
                let body = format!("{}\n\n{MARK}\n{REQUIRE}\n", current.trim_end());
                std::fs::write(&main, body.trim_start()).map_err(|e| e.to_string())?;
            }
            "Minimal setup done; digit's part loads at the end of hyprland.lua".into()
        }
    };
    crate::prefs::set("hypr_setup", toml::Value::String(mode.name().into()))?;
    sync()?;
    reload();
    Ok(note)
}

/// Removes digit's part: the folder and the require line. The rest of the config stays.
pub fn remove() -> Result<(), String> {
    let main = hypr().join("hyprland.lua");
    if let Ok(text) = std::fs::read_to_string(&main) {
        let kept: Vec<&str> = text.lines().filter(|l| l.trim() != REQUIRE && l.trim() != MARK).collect();
        std::fs::write(&main, format!("{}\n", kept.join("\n").trim_end())).map_err(|e| e.to_string())?;
    }
    let _ = std::fs::remove_dir_all(dir());
    Ok(())
}

/// Lets every key reach the focused app while Settings records a shortcut (`on`), or goes back
/// to the normal shortcuts.
pub fn capture(on: bool) {
    let map = if on { "digit-capture" } else { "reset" };
    let _ = run(&["hyprctl", "eval", &format!("hl.dispatch(hl.dsp.submap(\"{map}\"))")]);
}

/// Regenerates digit's files after a shortcut changed and has Hyprland pick them up.
pub fn apply() -> Result<(), String> {
    if sync()? {
        reload();
    }
    Ok(())
}

/// Asks a running Hyprland to read its config again.
pub fn reload() {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        let _ = run(&["hyprctl", "reload"]);
    }
}
