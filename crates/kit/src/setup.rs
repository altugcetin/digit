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

/// Whether digit has set up the running compositor.
pub fn installed() -> bool {
    match kind() {
        Some(Kind::Hyprland) => hyprconf::installed(),
        Some(Kind::Niri) => niriconf::installed(),
        None => false,
    }
}

pub fn setup() -> Result<String, String> {
    match kind() {
        Some(Kind::Hyprland) => hyprconf::setup(),
        Some(Kind::Niri) => niriconf::setup(),
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
