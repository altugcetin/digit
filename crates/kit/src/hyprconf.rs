//! digit's part of the Hyprland config: ~/.config/hypr/digit/, loaded by one `require("digit")`
//! in hyprland.lua. digit owns everything in that folder and regenerates it from its own
//! settings; hyprland.lua stays the user's.
//!
//! Setting up moves the old ~/.config/hypr to a backup and starts from digit's config, window
//! shortcuts included.

use crate::keybinds::{self, Act, Bind, Win};
use std::path::{Path, PathBuf};

pub const REQUIRE: &str = "require(\"digit\")";
/// The line older setups added above the require line; removing digit takes it out too.
const MARK: &str = "-- digit: its part of the config lives in ~/.config/hypr/digit (added by digit setup)";

pub fn hypr() -> PathBuf {
    gtk::glib::user_config_dir().join("hypr")
}

pub fn dir() -> PathBuf {
    hypr().join("digit")
}

/// Whether digit has set up Hyprland. Setups from before there was only one kind
/// ("minimal") count as not set up, so Settings offers to set it up properly.
pub fn installed() -> bool {
    crate::prefs::text(&crate::prefs::read(), "hypr_setup", "") == "full"
}

/// Where digit's programs live, so the config can start them by full path.
pub(crate) fn bin() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_else(|| gtk::glib::home_dir().join(".local/bin"))
}

pub(crate) fn lua_str(s: &str) -> String {
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

pub(crate) fn terminal() -> String {
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

/// Whether Hyprland can do what this shortcut does.
pub fn supports(b: &Bind) -> bool {
    !matches!(b.act, Act::Window(Win::PresetWidth | Win::Overview | Win::Center))
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
            Win::Maximize => "per_layout({ default = hl.dsp.window.fullscreen({ mode = 1 }), scrolling = column_maximize })".into(),
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
            // niri only; `supports` keeps these out of Hyprland's file.
            Win::PresetWidth | Win::Overview | Win::Center => String::new(),
        },
    }
}

/// digit-keybinds.lua: every shortcut in effect, each unbinding its keys first so digit's
/// binding replaces whatever the user's config put there.
pub fn render_keybinds(binds: &[Bind]) -> String {
    let mut out = String::from(
        "-- Generated by digit from its defaults and ~/.config/digit/keybinds.toml.\n\
         -- Changes here are replaced: change shortcuts in digit Settings, or add your own in hyprland.lua.\n\n\
         local function bind(keys, action, opts)\n    hl.unbind(keys)\n    hl.bind(keys, action, opts)\nend\n\n\
         -- Picks the action for the layout of the workspace in front when the keys are pressed,\n\
         -- so one shortcut can mean different things in dwindle and scrolling.\n\
         local function per_layout(actions)\n    return function()\n        local ws = hl.get_active_special_workspace() or hl.get_active_workspace()\n        local action = ws and actions[ws.tiled_layout] or actions.default\n        if type(action) == \"function\" then\n            action()\n        elseif action then\n            hl.dispatch(action)\n        end\n    end\nend\n\n\
         -- In scrolling, maximize widens the column to the whole screen and back to the width it had.\n\
         local widths = {}\n\
         local function column_maximize()\n    local w = hl.get_active_window()\n    local m = hl.get_active_monitor()\n    if not w or not m then\n        return\n    end\n    local share = w.size.x / (m.width / m.scale)\n    if share < 0.9 then\n        -- Remember the width to the nearest 5%, as the window itself is a little narrower than its column.\n        widths[w.address] = string.format(\"%.2f\", math.floor(share * 20 + 0.5) / 20)\n        hl.dispatch(hl.dsp.layout(\"colresize 1.0\"))\n    else\n        local back = widths[w.address] or tostring(hl.get_config(\"scrolling.column_width\") or 0.5)\n        widths[w.address] = nil\n        hl.dispatch(hl.dsp.layout(\"colresize \" .. back))\n    end\nend\n\n\
         -- Settings lets keys through to itself while you record a new shortcut; Escape leaves.\n\
         hl.define_submap(\"digit-capture\", function()\n    hl.bind(\"ESCAPE\", hl.dsp.submap(\"reset\"))\nend)\n\n",
    );
    let mut group = "";
    for b in binds.iter().filter(|b| supports(b)) {
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

pub(crate) fn fill(template: &str, values: &[(&str, &str)]) -> String {
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
pub(crate) fn rounding() -> i64 {
    let round = crate::prefs::number(&crate::prefs::read(), "ui_round", 1.0).clamp(0.1, 1.8);
    (14.0 * round).round() as i64
}


/// digit-base.lua: what the config needs to be usable on its own: input, layouts, gestures,
/// environment.
fn base() -> String {
    fill(BASE, &[("settings", &compositor())])
}

/// Settings > Compositor, as Lua: every value, with digit's defaults for the ones never changed.
fn compositor() -> String {
    let t = crate::prefs::read();
    let pick = |key: &str| t.get(&format!("hypr_{key}")).cloned();
    let text = |key: &str, def: &str| pick(key).and_then(|v| v.as_str().map(str::to_string)).or_else(|| Some(def.to_string()));
    let num = |key: &str, def: f64| pick(key).and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64))).or(Some(def));
    let flag = |key: &str, def: bool| pick(key).and_then(|v| v.as_bool()).or(Some(def));
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
        let mut opts: Vec<String> = Vec::new();
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
    // The focused window's border: digit's accent, or the plain grey of digit's default look.
    let colors = t.get("border_color").and_then(|v| v.as_str().map(str::to_string)).or_else(|| Some("accent".to_string()));
    if let Some(c) = colors {
        let (active, inactive) = if c == "default" {
            ("rgba(595959ff)".to_string(), "rgba(5959591a)".to_string())
        } else {
            (format!("rgba({}ff)", crate::niriconf::accent().trim_start_matches('#')), "rgba(ffffff14)".to_string())
        };
        general.push(format!("col = {{ active_border = {}, inactive_border = {} }}", lua_str(&active), lua_str(&inactive)));
    }
    let mut decoration = Vec::new();
    // Reduced effects switch the compositor's shadows and blur off too.
    let reduced = crate::effects::reduced();
    let flag = |key: &str, def: bool| if reduced && (key == "shadow" || key == "blur") { Some(false) } else { flag(key, def) };
    if let Some(on) = flag("shadow", true) {
        let range = match text("shadow_size", "medium").as_deref() {
            Some("soft") => 8,
            Some("large") => 24,
            _ => 14,
        };
        decoration.push(format!("shadow = {{ enabled = {on}, range = {range}, render_power = 3, color = 0x66000000 }}"));
    }
    if let Some(on) = flag("blur", false) {
        let (size, passes) = match text("blur_strength", "medium").as_deref() {
            Some("light") => (4, 2),
            Some("strong") => (12, 4),
            _ => (8, 3),
        };
        decoration.push(format!("blur = {{ enabled = {on}, size = {size}, passes = {passes} }}"));
    }
    let mut out = String::from("-- Settings > Compositor\n");
    let mut sections = Vec::new();
    if !input.is_empty() {
        sections.push(format!("    input = {{\n{}\n    }},", input.iter().map(|l| format!("        {l},")).collect::<Vec<_>>().join("\n")));
    }
    if !general.is_empty() {
        sections.push(format!("    general = {{\n{}\n    }},", general.iter().map(|l| format!("        {l},")).collect::<Vec<_>>().join("\n")));
    }
    if !decoration.is_empty() {
        sections.push(format!("    decoration = {{\n{}\n    }},", decoration.iter().map(|l| format!("        {l},")).collect::<Vec<_>>().join("\n")));
    }
    if !sections.is_empty() {
        out.push_str(&format!("hl.config({{\n{}\n}})\n", sections.join("\n")));
    }
    // Motion: how windows open and close, and which way workspaces slide.
    let windows = text("window_anim", "popin");
    let spaces = text("workspace_anim", "vertical");
    if windows.is_some() || spaces.is_some() {
        out.push_str("hl.curve(\"md3_standard\", { type = \"bezier\", points = { { 0.2, 0 }, { 0, 1.0 } } })\n");
        out.push_str("hl.curve(\"md3_decel\", { type = \"bezier\", points = { { 0.05, 0.7 }, { 0.1, 1 } } })\n");
        out.push_str("hl.curve(\"md3_accel\", { type = \"bezier\", points = { { 0.3, 0 }, { 0.8, 0.15 } } })\n");
    }
    if let Some(w) = windows {
        let style = if w == "slide" { "slide" } else { "popin 85%" };
        out.push_str(&format!("hl.animation({{ leaf = \"windowsIn\", enabled = true, speed = 3, bezier = \"md3_decel\", style = \"{style}\" }})\n"));
        out.push_str(&format!("hl.animation({{ leaf = \"windowsOut\", enabled = true, speed = 3, bezier = \"md3_accel\", style = \"{style}\" }})\n"));
    }
    if let Some(sp) = spaces {
        let style = if sp == "horizontal" { "slidefade 20%" } else { "slidefadevert 20%" };
        out.push_str(&format!("hl.animation({{ leaf = \"workspaces\", enabled = true, speed = 3, bezier = \"md3_standard\", style = \"{style}\" }})\n"));
    }
    if flag("gesture", true) == Some(true) {
        out.push_str("-- Three fingers sideways switch workspaces.\nhl.gesture({ fingers = 3, direction = \"horizontal\", action = \"workspace\" })\n");
    }
    out
}

/// digit-decorations.lua: gaps, borders, corners, shadows and motion of windows. The corner
/// radius follows digit's Corners setting.
fn decorations() -> String {
    fill(DECORATIONS, &[("rounding", &rounding().to_string())])
}

pub(crate) fn write_if_changed(path: &Path, body: &str) -> std::io::Result<bool> {
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
    // One change can save two settings at once (blur on + its strength), each starting a
    // sync on its own thread; they must not write the same files at the same time.
    static WRITING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    if !installed() {
        return Ok(false);
    }
    let d = dir();
    let e = |e: std::io::Error| format!("Could not write {}: {e}", d.display());
    let binds = shortcuts();
    let mut changed = false;
    changed |= write_if_changed(&d.join("init.lua"), INIT).map_err(e)?;
    changed |= write_if_changed(&d.join("digit-autostart.lua"), &autostart()).map_err(e)?;
    changed |= write_if_changed(&d.join("digit-rules.lua"), RULES).map_err(e)?;
    changed |= write_if_changed(&d.join("digit-keybinds.lua"), &render_keybinds(&binds)).map_err(e)?;
    changed |= write_if_changed(&d.join("digit-base.lua"), &base()).map_err(e)?;
    changed |= write_if_changed(&d.join("digit-decorations.lua"), &decorations()).map_err(e)?;
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

/// Shortcuts in effect, as far as Hyprland can do them.
pub fn shortcuts() -> Vec<Bind> {
    keybinds::effective().into_iter().filter(supports).collect()
}

pub(crate) fn stamp() -> String {
    gtk::glib::DateTime::now_local().ok().and_then(|t| t.format("%Y%m%d-%H%M%S").ok()).map(|s| s.to_string()).unwrap_or_else(|| "backup".into())
}

fn run(argv: &[&str]) -> Option<String> {
    std::process::Command::new(argv[0]).args(&argv[1..]).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
}

/// The displays as Hyprland runs them now, so setting up keeps the screen arrangement.
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

pub(crate) fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
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
/// Runs a command for at most `secs`, giving whether it succeeded and what it printed.
pub(crate) fn run_for(argv: &[&str], secs: u64) -> Option<(bool, String)> {
    let mut child = std::process::Command::new(argv[0])
        .args(&argv[1..])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()?;
    let start = std::time::Instant::now();
    loop {
        if let Ok(Some(_)) = child.try_wait() {
            let out = child.wait_with_output().ok()?;
            let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
            return Some((out.status.success(), text));
        }
        if start.elapsed().as_secs() >= secs {
            let _ = child.kill();
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// Changes a compositor config folder with a way back: if `change` or the check after it
/// fails, the folder and the setup choice go back to how they were.
pub(crate) fn guarded(root: &Path, pref: &str, change: impl FnOnce() -> Result<String, String>, check: impl FnOnce() -> Result<(), String>) -> Result<String, String> {
    let before = crate::prefs::read().get(pref).cloned();
    let keep = root.with_file_name(format!(".digit-undo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&keep);
    let had = root.exists();
    if had {
        copy_dir(root, &keep).map_err(|e| format!("Could not keep a copy of {}: {e}", root.display()))?;
    }
    let result = change().and_then(|note| check().map(|_| note));
    if result.is_err() {
        let _ = std::fs::remove_dir_all(root);
        if had {
            let _ = copy_dir(&keep, root);
        }
        let _ = match before {
            Some(v) => crate::prefs::set(pref, v),
            None => crate::prefs::unset(pref),
        };
    }
    let _ = std::fs::remove_dir_all(&keep);
    result
}

/// Asks Hyprland whether the config parses, when Hyprland is installed.
fn check(main: &Path) -> Result<(), String> {
    if gtk::glib::find_program_in_path("Hyprland").is_none() {
        return Ok(());
    }
    match run_for(&["Hyprland", "--verify-config", "-c", &main.display().to_string()], 30) {
        Some((_, out)) if out.contains("config ok") => Ok(()),
        Some((_, out)) => Err(format!("Hyprland didn't accept the new config, so nothing was changed:\n{}", out.lines().filter(|l| !l.contains("DEBUG")).rev().take(6).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n"))),
        None => Ok(()),
    }
}

pub fn setup() -> Result<String, String> {
    let hypr = hypr();
    let main = hypr.join("hyprland.lua");
    // Running the installer again refreshes digit's files instead of starting over, so the
    // user's own additions in hyprland.lua stay.
    if installed() && dir().exists() {
        sync()?;
        check(&main)?;
        reload();
        return Ok("Already set up; digit's files are up to date".into());
    }
    let note = guarded(&hypr, "hypr_setup", setup_now, || check(&main))?;
    reload();
    Ok(note)
}

fn setup_now() -> Result<String, String> {
    let hypr = hypr();
    let main = hypr.join("hyprland.lua");
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
    std::fs::write(&main, MAIN).map_err(|e| e.to_string())?;
    if let Some(m) = monitors {
        std::fs::write(dir().join("digit-monitors.lua"), m).map_err(|e| e.to_string())?;
    }
    crate::prefs::set("hypr_setup", toml::Value::String("full".into()))?;
    sync()?;
    Ok(match kept {
        Some(b) => format!("Set up; your old config is in {}", b.display()),
        None => "Set up".into(),
    })
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
