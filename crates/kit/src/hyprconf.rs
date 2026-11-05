//! Digit-owned Hyprland session config in ~/.config/digit/hyprland.

use crate::keybinds::{self, Act, Bind, Win};
use std::path::{Path, PathBuf};

pub fn hypr() -> PathBuf {
    gtk::glib::user_config_dir().join("digit/hyprland")
}

pub fn dir() -> PathBuf {
    hypr().join("digit")
}

/// Whether this is the Hyprland with Digit session.
pub fn installed() -> bool {
    std::env::var("DIGIT_SESSION").as_deref() == Ok("hyprland")
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
    if let Ok(t) = std::env::var("TERMINAL")
        && !t.trim().is_empty()
    {
        return t;
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
            Win::StepWindow(i) => format!("hl.dsp.window.cycle_next({{ next = {} }})", *i > 0),
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
         -- Changes here are replaced: change shortcuts in digit Settings, or add your own in user.lua.\n\n\
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
    // A saved override keeps its keys if a new default lands on the same combination.
    let taken: Vec<_> = binds.iter().filter(|b| supports(b) && (keybinds::changed(&b.id) || b.id.starts_with("custom-"))).map(|b| keybinds::normalize(&b.keys)).collect();
    let mut placed = Vec::new();
    let mut group = "";
    for b in binds.iter().filter(|b| supports(b) && !b.keys.trim().is_empty()) {
        let k = keybinds::normalize(&b.keys);
        let own = keybinds::changed(&b.id) || b.id.starts_with("custom-");
        if placed.contains(&k) || (!own && taken.contains(&k)) {
            continue;
        }
        placed.push(k);
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

/// Connector-keyed assignments include unplugged displays. Ignore invalid numbers and
/// resolve duplicate assignments deterministically, so each workspace has one home.
pub fn workspace_monitors(prefs: &toml::Table) -> std::collections::BTreeMap<String, Vec<u8>> {
    let mut monitors = std::collections::BTreeMap::new();
    let mut used = std::collections::BTreeSet::new();
    if let Some(table) = prefs.get("monitor_workspaces").and_then(toml::Value::as_table) {
        for (name, value) in table {
            let mut numbers: Vec<u8> = value.as_array().into_iter().flatten().filter_map(toml::Value::as_integer).filter(|n| (1..=10).contains(n)).map(|n| n as u8).collect();
            numbers.sort_unstable();
            numbers.retain(|n| used.insert(*n));
            monitors.insert(name.clone(), numbers);
        }
    }
    monitors
}

fn workspaces(prefs: &toml::Table) -> String {
    let mut monitors = workspace_monitors(prefs);
    let primary = crate::prefs::text(prefs, "primary_monitor", "");
    // An explicit workspace assignment takes precedence over the primary default.
    if !primary.is_empty() && !monitors.values().any(|numbers| numbers.contains(&1)) {
        monitors.entry(primary).or_default().insert(0, 1);
    }
    let mut rules = String::new();
    for (monitor, numbers) in monitors {
        for (i, number) in numbers.iter().enumerate() {
            let default = if i == 0 { ", default = true" } else { "" };
            rules.push_str(&format!("hl.workspace_rule({{ workspace = \"{number}\", monitor = {}{default} }})\n", lua_str(&monitor)));
        }
    }
    fill(WORKSPACES, &[("rules", &rules)])
}

fn primary(prefs: &toml::Table) -> String {
    let name = crate::prefs::text(prefs, "primary_monitor", "");
    let mut text = String::from("-- Primary XWayland output from Digit Settings. Requires xrandr.\n");
    if prefs.contains_key("primary_monitor") {
        // hl.exec_cmd uses a shell. Quote independently of the Lua string literal.
        let quoted = format!("'{}'", name.replace('\'', "'\\''"));
        let command = if name.is_empty() { "xrandr --noprimary".to_string() } else { format!("xrandr --output {quoted} --primary") };
        let command = lua_str(&command);
        text.push_str(&format!("local function primary()\n    hl.exec_cmd({command})\nend\nhl.on(\"hyprland.start\", primary)\nhl.on(\"config.reloaded\", primary)\nhl.on(\"monitor.layout_changed\", primary)\n"));
    }
    text
}

/// Corner radius for windows, following Settings > Desktop > Corners like the rest of digit.
pub(crate) fn rounding() -> i64 {
    let round = crate::prefs::number(&crate::prefs::read(), "ui_round", 1.0).clamp(0.1, 1.8);
    (14.0 * round).round() as i64
}


/// digit-base.lua: what the config needs to be usable on its own: input, layouts, gestures,
/// environment.
fn base() -> String {
    let gpu = NVIDIA_ENV.iter().filter(|_| nvidia()).map(|(k, v)| format!("hl.env(\"{k}\", \"{v}\")\n")).collect::<String>();
    fill(BASE, &[("settings", &compositor()), ("gpu", &gpu)])
}

/// What the NVIDIA driver needs for hardware video (screen sharing, players) and GLX apps.
pub(crate) const NVIDIA_ENV: [(&str, &str); 3] = [("LIBVA_DRIVER_NAME", "nvidia"), ("__GLX_VENDOR_LIBRARY_NAME", "nvidia"), ("NVD_BACKEND", "direct")];

pub(crate) fn nvidia() -> bool {
    Path::new("/sys/module/nvidia_drm").exists()
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

/// Rewrites digit's folder from digit's settings. Display geometry is only created when
/// missing; workspace assignments are regenerated from connector-keyed preferences.
pub fn sync() -> Result<bool, String> {
    if !installed() {
        return Ok(false);
    }
    generate()
}

fn generate() -> Result<bool, String> {
    // One change can save two settings at once (blur on + its strength), each starting a
    // sync on its own thread; they must not write the same files at the same time.
    static WRITING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    let d = dir();
    let mut changed = crate::setup::entry(&hypr(), "hyprland.lua", MAIN, "user.lua", "--")?;
    let e = |e: std::io::Error| format!("Could not write {}: {e}", d.display());
    let binds = shortcuts();
    changed |= write_if_changed(&d.join("init.lua"), INIT).map_err(e)?;
    changed |= write_if_changed(&d.join("digit-autostart.lua"), &autostart()).map_err(e)?;
    changed |= write_if_changed(&d.join("digit-rules.lua"), RULES).map_err(e)?;
    changed |= write_if_changed(&d.join("digit-keybinds.lua"), &render_keybinds(&binds)).map_err(e)?;
    changed |= write_if_changed(&d.join("digit-base.lua"), &base()).map_err(e)?;
    changed |= write_if_changed(&d.join("digit-decorations.lua"), &decorations()).map_err(e)?;
    let prefs = crate::prefs::read();
    changed |= write_if_changed(&d.join("digit-primary.lua"), &primary(&prefs)).map_err(e)?;
    let workspaces_path = d.join("digit-workspaces.lua");
    let managed = std::fs::read_to_string(&workspaces_path).is_ok_and(|text| text.starts_with(WORKSPACES.lines().next().unwrap_or_default()));
    if prefs.contains_key("monitor_workspaces") || prefs.contains_key("primary_monitor") || managed || !workspaces_path.exists() {
        changed |= write_if_changed(&workspaces_path, &workspaces(&prefs)).map_err(e)?;
    }
    let monitors_path = d.join("digit-monitors.lua");
    if !monitors_path.exists() {
        changed |= write_if_changed(&monitors_path, MONITORS).map_err(e)?;
    }
    Ok(changed)
}

/// Shortcuts in effect, as far as Hyprland can do them.
pub fn shortcuts() -> Vec<Bind> {
    keybinds::effective().into_iter().filter(supports).collect()
}

fn run(argv: &[&str]) -> Option<String> {
    std::process::Command::new(argv[0]).args(&argv[1..]).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
}

/// Prepares the session without a display or compositor reload.
pub fn prepare() -> Result<(), String> {
    generate().map(|_| ())
}

/// Lets every key reach the focused app while Settings records a shortcut (`on`), or goes back
/// to the normal shortcuts.
pub fn capture(on: bool) {
    if !installed() { return; }
    let map = if on { "digit-capture" } else { "reset" };
    let _ = run(&["hyprctl", "eval", &format!("hl.dispatch(hl.dsp.submap(\"{map}\"))")]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_rules_choose_the_lowest_default_and_keep_unplugged_connectors() {
        let prefs = "primary_monitor = 'DP-1'\n[monitor_workspaces]\nDP-1 = [5, 3, 3, 0, 11]\nHDMI-A-1 = [10, 7]\n".parse().unwrap();
        let rendered = workspaces(&prefs);
        assert!(rendered.contains("workspace = \"1\", monitor = \"DP-1\", default = true"));
        assert!(rendered.contains("workspace = \"3\", monitor = \"DP-1\" })"));
        assert!(rendered.contains("workspace = \"7\", monitor = \"HDMI-A-1\", default = true"));
        assert!(rendered.contains("workspace = \"10\", monitor = \"HDMI-A-1\" })"));
        assert_eq!(rendered.matches("hl.workspace_rule(").count(), 5);
        assert_eq!(rendered.matches("default = true").count(), 2);
    }

    #[test]
    fn explicit_workspace_one_overrides_primary_and_duplicates_have_one_home() {
        let prefs = "primary_monitor = 'HDMI-A-1'\n[monitor_workspaces]\nDP-1 = [1, 2]\nHDMI-A-1 = [2, 4]\n".parse().unwrap();
        let rendered = workspaces(&prefs);
        assert_eq!(rendered.matches("workspace = \"1\"").count(), 1);
        assert_eq!(rendered.matches("workspace = \"2\"").count(), 1);
        assert!(rendered.contains("workspace = \"1\", monitor = \"DP-1\", default = true"));
        assert!(rendered.contains("workspace = \"4\", monitor = \"HDMI-A-1\", default = true"));
    }

    #[test]
    fn automatic_primary_generates_no_rules_or_commands() {
        let prefs = toml::Table::new();
        assert!(!workspaces(&prefs).contains("hl.workspace_rule("));
        assert!(!primary(&prefs).contains("hl.exec_cmd("));
    }

    #[test]
    fn clearing_primary_removes_workspace_one_and_resets_xwayland_primary() {
        let prefs = "primary_monitor = ''".parse().unwrap();
        assert!(!workspaces(&prefs).contains("hl.workspace_rule("));
        assert!(primary(&prefs).contains("hl.exec_cmd(\"xrandr --noprimary\")"));
    }

    #[test]
    fn primary_command_quotes_connector_for_both_lua_and_the_shell() {
        let mut prefs = toml::Table::new();
        prefs.insert("primary_monitor".into(), toml::Value::String("DP-'\"$(touch /tmp/never)".into()));
        let rendered = primary(&prefs);
        let command = "xrandr --output 'DP-'\\''\"$(touch /tmp/never)' --primary";
        assert!(rendered.contains(&format!("hl.exec_cmd({})", lua_str(command))));
        for event in ["hyprland.start", "config.reloaded", "monitor.layout_changed"] {
            assert!(rendered.contains(&format!("hl.on(\"{event}\", primary)")));
        }
    }

    #[test]
    fn workspace_connectors_are_lua_strings() {
        let prefs = "[monitor_workspaces]\n'DP-\"1' = [2]\n".parse().unwrap();
        assert!(workspaces(&prefs).contains("monitor = \"DP-\\\"1\", default = true"));
    }

    #[test]
    fn wheel_focus_and_window_defaults_render_to_hyprland_actions() {
        let binds: Vec<_> = keybinds::defaults().into_iter().filter(|b| b.id.starts_with("focus-scroll-") || b.id.starts_with("window-")).collect();
        let rendered = render_keybinds(&binds);
        assert!(rendered.contains("bind(\"SUPER + mouse_up\", hl.dsp.focus({ direction = \"up\" }))"));
        assert!(rendered.contains("bind(\"SUPER + mouse_down\", hl.dsp.focus({ direction = \"down\" }))"));
        assert!(rendered.contains("bind(\"SUPER + SHIFT + mouse_down\", hl.dsp.window.cycle_next({ next = true }))"));
        assert!(rendered.contains("bind(\"SUPER + SHIFT + mouse_up\", hl.dsp.window.cycle_next({ next = false }))"));
    }
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
    if installed() && std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        let _ = run(&["hyprctl", "reload"]);
    }
}
