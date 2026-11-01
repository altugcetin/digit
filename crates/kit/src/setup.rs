//! The compositor config digit manages, whichever compositor is running: Hyprland through
//! `hyprconf`, niri through `niriconf`. Settings and the shell talk to this.

use crate::hyprconf::{self, Mode};
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

pub fn mode() -> Option<Mode> {
    match kind()? {
        Kind::Hyprland => hyprconf::mode(),
        Kind::Niri => niriconf::mode(),
    }
}

pub fn setup(mode: Mode) -> Result<String, String> {
    match kind() {
        Some(Kind::Hyprland) => hyprconf::setup(mode),
        Some(Kind::Niri) => niriconf::setup(mode),
        None => Err("digit can set up Hyprland and niri only".into()),
    }
}

/// Regenerates digit's part of the config after a setting or shortcut changed.
pub fn apply() -> Result<(), String> {
    match kind() {
        Some(Kind::Hyprland) => hyprconf::apply(),
        Some(Kind::Niri) => niriconf::sync().map(|_| ()),
        None => Ok(()),
    }
}

/// Regenerates both compositors' files where they are set up, as the shell does at start.
pub fn sync_all() -> Result<bool, String> {
    let hypr = hyprconf::sync()?;
    let niri = niriconf::sync()?;
    if hypr && kind() == Some(Kind::Hyprland) {
        hyprconf::reload();
    }
    Ok(hypr || niri)
}

/// Shortcuts in effect that the compositor can do, and those the user's config binds too.
pub fn shortcuts() -> (Vec<Bind>, Vec<(String, String)>) {
    match kind() {
        Some(Kind::Niri) => {
            let full = niriconf::mode() == Some(Mode::Full);
            (crate::keybinds::effective(full).into_iter().filter(niriconf::supports).collect(), Vec::new())
        }
        _ => hyprconf::shortcuts(),
    }
}

/// Whether the running compositor can do what this shortcut does.
pub fn supports(b: &Bind) -> bool {
    kind() != Some(Kind::Niri) || niriconf::supports(b)
}

/// Lets keys that are already shortcuts reach Settings while it records one (Hyprland only).
pub fn capture(on: bool) {
    if kind() == Some(Kind::Hyprland) {
        hyprconf::capture(on);
    }
}
