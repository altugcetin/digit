//! The shell's side of `kit::screens`: the main screen as the settings now choose it, and the
//! screen a panel opens on.

use gtk::gdk;
use std::cell::RefCell;

thread_local! {
    /// The `primary_monitor` setting, kept as the settings are applied.
    static CHOSEN: RefCell<String> = const { RefCell::new(String::new()) };
}

pub fn set_chosen(name: &str) {
    CHOSEN.with(|c| *c.borrow_mut() = name.to_string());
}

/// The main screen (see `kit::screens::primary`).
pub fn main() -> Option<gdk::Monitor> {
    CHOSEN.with(|c| kit::screens::primary(&c.borrow()))
}

/// The main screen's connector, or "" with no screen.
pub fn main_name() -> String {
    main().map(|m| kit::screens::name(&m)).unwrap_or_default()
}

/// The screen `output` when it is plugged in, else the main one: where a panel asked for on a
/// screen opens.
pub fn for_output(output: Option<&str>) -> Option<gdk::Monitor> {
    output.and_then(kit::screens::by_name).or_else(main)
}
