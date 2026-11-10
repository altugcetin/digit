//! The compositor config digit manages, whichever compositor is running: Hyprland through
//! `hyprconf`, niri through `niriconf`. Settings and the shell talk to this.

use crate::hyprconf;
use crate::keybinds::Bind;
use crate::niriconf;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Hyprland,
    Niri,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Hyprland => "Hyprland",
            Kind::Niri => "niri",
        }
    }
}

/// The running compositor, when digit can manage its config.
pub fn kind() -> Option<Kind> {
    match crate::wm::which() {
        crate::wm::Wm::Hyprland => Some(Kind::Hyprland),
        crate::wm::Wm::Niri => Some(Kind::Niri),
        _ => None,
    }
}

/// Whether the running compositor belongs to the Digit session.
pub fn installed() -> bool {
    match kind() {
        Some(Kind::Hyprland) => hyprconf::installed(),
        Some(Kind::Niri) => niriconf::installed(),
        None => false,
    }
}

/// Creates Digit's config before the compositor starts. No display is needed.
pub fn prepare(wm: &str) -> Result<(), String> {
    match wm {
        "hyprland" => hyprconf::prepare(),
        "niri" => niriconf::prepare(),
        _ => Err("Usage: digit-settings --prepare-session hyprland|niri".into()),
    }
}

/// Refreshes the owned entry and creates the user's file exactly once.
pub(crate) fn entry(root: &std::path::Path, main: &str, body: &str, user: &str, comment: &str) -> Result<bool, String> {
    use std::io::Write;
    let result = (|| -> std::io::Result<bool> {
        std::fs::create_dir_all(root)?;
        match std::fs::OpenOptions::new().write(true).create_new(true).open(root.join(user)) {
            Ok(mut file) => writeln!(file, "{comment} Your settings load last. Digit never changes this file.")?,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        crate::hyprconf::write_if_changed(&root.join(main), body)
    })();
    result.map_err(|e| format!("Could not prepare {}: {e}", root.display()))
}

/// Regenerates digit's part of the config after a setting or shortcut changed.
pub fn apply() -> Result<(), String> {
    let result = match kind() {
        Some(Kind::Hyprland) => hyprconf::apply(),
        Some(Kind::Niri) => niriconf::sync().map(|_| ()),
        None => Ok(()),
    };
    if result.is_ok() { crate::cursors::apply_live(); }
    result
}

/// Regenerates the active Digit session config, as the shell does at start.
pub fn sync_all() -> Result<bool, String> {
    let hypr = hyprconf::sync()?;
    let niri = niriconf::sync()?;
    if hypr && kind() == Some(Kind::Hyprland) {
        hyprconf::reload();
    }
    Ok(hypr || niri)
}

/// Shortcuts in effect that the compositor can do.
pub fn shortcuts() -> Vec<Bind> {
    match kind() {
        Some(Kind::Niri) => crate::keybinds::effective().into_iter().filter(niriconf::supports).collect(),
        _ => hyprconf::shortcuts(),
    }
}

/// Whether the running compositor can do what this shortcut does.
pub fn supports(b: &Bind) -> bool {
    match kind() {
        Some(Kind::Niri) => niriconf::supports(b),
        _ => hyprconf::supports(b),
    }
}

/// Lets keys that are already shortcuts reach Settings while it records one (Hyprland only).
pub fn capture(on: bool) {
    if kind() == Some(Kind::Hyprland) {
        hyprconf::capture(on);
    }
}

#[cfg(test)]
mod tests;
