use crate::ui;
use gtk::{glib, prelude::*};
use kit::keybinds::{self, Bind};
use std::cell::Cell;
use std::rc::Rc;

pub fn build() -> gtk::Widget {
    let (scroll, body) = ui::page("Keyboard", "Typing and digit's shortcuts. Click a shortcut to record new keys, use the scroll wheel or turn it off; changes apply right away.");
    let Some(kind) = kit::setup::kind() else {
        let card = ui::card();
        let (row, _) = ui::row(Some("warning"), "digit can't set shortcuts on this compositor", "digit manages its shortcuts on Hyprland and niri. Bind digit's commands, such as \"digitctl key launcher\", in your compositor's own config");
        card.append(&row);
        body.append(&card);
        return scroll.upcast();
    };
    if !kit::setup::installed() {
        crate::desktop::session_notice(&body, kind);
        return scroll.upcast();
    }
    crate::compositor::typing(&body);
    let live = kit::setup::shortcuts();
    // Only what the compositor can do: niri has no scratchpad or pinned windows, for one.
    // Going to and moving to workspaces are set as a whole, at the top of their group.
    let all: Vec<Bind> = keybinds::all().into_iter().filter(kit::setup::supports).filter(|b| !numbered(b)).collect();
    let across = keybinds::spaces_across();
    let mut group = "";
    let mut card = ui::card();
    for b in &all {
        if b.group != group {
            if !group.is_empty() {
                body.append(&card);
            }
            group = b.group;
            body.append(&ui::heading(group));
            card = ui::card();
            if group == "Workspaces" {
                spaces_rows(&card);
            }
        }
        card.append(&default_row(b, across));
    }
    if !group.is_empty() {
        body.append(&card);
    }
    body.append(&ui::heading("Your shortcuts"));
    body.append(&custom_card(&live));
    // Start at the top even when a button deep in the page takes focus.
    // Remember how far down the page is, so redrawing it after a change keeps the place;
    // a fresh visit starts at the top even when a button deep in the page takes focus.
    let adj = scroll.vadjustment();
    adj.connect_value_changed(|a| SCROLLED.with(|c| c.set(a.value())));
    scroll.connect_map(|s| {
        let adj = s.vadjustment();
        let to = if KEEP.with(|k| k.replace(false)) { SCROLLED.with(Cell::get) } else { 0.0 };
        // The page's height settles over the first frames; keep placing it until it fits.
        let tries = Rc::new(Cell::new(0));
        glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
            adj.set_value(to);
            tries.set(tries.get() + 1);
            if (adj.value() - to).abs() < 1.0 || tries.get() > 20 { glib::ControlFlow::Break } else { glib::ControlFlow::Continue }
        });
    });
    scroll.upcast()
}

thread_local! {
    static SCROLLED: Cell<f64> = const { Cell::new(0.0) };
    static KEEP: Cell<bool> = const { Cell::new(false) };
}

/// Lets a row's subtitle wrap instead of being cut off.
fn key_button(keys: &str) -> gtk::Button {
    let shown = if keys.is_empty() { "Off".into() } else { keybinds::label(keys) };
    let b = ui::button(&shown, "");
    b.add_css_class("keycap");
    b.set_tooltip_text(Some("Change this shortcut"));
    b
}

/// Going to or moving the window to one numbered workspace, which [`spaces_rows`] sets for all.
fn numbered(b: &Bind) -> bool {
    matches!(b.act, keybinds::Act::Window(keybinds::Win::Workspace(_) | keybinds::Win::ToWorkspace(_)))
}

/// A shortcut with each of its key combinations, + to add another, and back to the default.
fn default_row(b: &Bind, across: bool) -> gtk::Box {
    let note = keybinds::note(b, across).unwrap_or_default();
    let (row, end) = ui::row(None, &b.label, &note);
    let reset = kit::app::control("restart_alt", "on-hover");
    reset.set_tooltip_text(Some("Back to the default"));
    let keys = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    keys.set_valign(gtk::Align::Center);
    {
        let (id, keys) = (b.id.clone(), keys.downgrade());
        reset.connect_clicked(move |r| {
            if save(keybinds::reset(&id)) && let Some(keys) = keys.upgrade() {
                fill_keys(&keys, r, &id);
            }
        });
    }
    end.append(&reset);
    end.append(&keys);
    fill_keys(&keys, &reset, &b.id);
    row
}

/// Shows shortcut `id`'s combinations in `holder`, each a key to change or remove. Changes
/// update the row in place; redrawing the whole page would flicker.
fn fill_keys(holder: &gtk::Box, reset: &gtk::Button, id: &str) {
    while let Some(c) = holder.first_child() {
        holder.remove(&c);
    }
    reset.set_visible(keybinds::changed(id));
    let current = keybinds::all().into_iter().find(|b| b.id == id).map(|b| b.keys).unwrap_or_default();
    let list = keybinds::combos(&current);
    let refresh = {
        let (holder, reset, id) = (holder.downgrade(), reset.downgrade(), id.to_string());
        Rc::new(move || if let (Some(h), Some(r)) = (holder.upgrade(), reset.upgrade()) { fill_keys(&h, &r, &id) })
    };
    // Puts `picked` in place `at` (a new one past the end), or removes that one when empty.
    let change = {
        let (list, id, refresh) = (list.clone(), id.to_string(), refresh.clone());
        Rc::new(move |at: usize, picked: String| {
            let mut next = list.clone();
            if picked.is_empty() {
                if at < next.len() { next.remove(at); }
            } else {
                let k = keybinds::normalize(&picked);
                if next.iter().enumerate().any(|(i, c)| i != at && keybinds::normalize(c) == k) {
                    return;
                }
                if let Some(o) = taken(&kit::setup::shortcuts(), &picked, &id) {
                    ui::fail(&format!("{} already opens {o}", keybinds::label(&picked)));
                    return;
                }
                if at < next.len() { next[at] = picked } else { next.push(picked) }
            }
            if save(keybinds::set_keys(&id, &next.join(keybinds::SEP))) {
                refresh();
            }
        })
    };
    let several = list.len() > 1;
    for (i, k) in list.iter().enumerate() {
        let chip = key_button(k);
        let (k, change) = (k.clone(), change.clone());
        chip.connect_clicked(move |btn| {
            let change = change.clone();
            record(btn.upcast_ref(), &k, Some(if several { "Remove these keys" } else { "Turn off" }), move |picked| change(i, picked));
        });
        holder.append(&chip);
    }
    if list.is_empty() {
        let off = key_button("");
        let change = change.clone();
        off.connect_clicked(move |btn| {
            let change = change.clone();
            record(btn.upcast_ref(), "", None, move |picked| change(0, picked));
        });
        holder.append(&off);
        return;
    }
    // Before the keys, so the keys line up at the row's end with or without it.
    let add = kit::app::control("add", "on-hover");
    add.set_tooltip_text(Some("Add other keys for this"));
    let at = list.len();
    add.connect_clicked(move |btn| {
        let change = change.clone();
        record(btn.upcast_ref(), "", None, move |picked| change(at, picked));
    });
    holder.prepend(&add);
}

/// The modifiers offered for going to and moving to workspaces.
const MODIFIERS: [(&str, &str); 10] = [
    ("SUPER", "Super"),
    ("SUPER + SHIFT", "Super+Shift"),
    ("SUPER + CTRL", "Super+Ctrl"),
    ("SUPER + ALT", "Super+Alt"),
    ("ALT", "Alt"),
    ("ALT + SHIFT", "Alt+Shift"),
    ("CTRL", "Ctrl"),
    ("CTRL + SHIFT", "Ctrl+Shift"),
    ("CTRL + ALT", "Ctrl+Alt"),
    ("", "Off"),
];

/// Another shortcut on the keys `next` would give the workspaces, by its keys and label.
fn spaces_clash(next: &keybinds::Spaces) -> Option<String> {
    let live: Vec<Bind> = kit::setup::shortcuts().into_iter().filter(|b| !numbered(b)).collect();
    let mut mine: Vec<String> = Vec::new();
    for i in 1..=keybinds::SPACES as u8 {
        for keys in [next.go_keys(i), next.move_keys(i)] {
            if keys.is_empty() { continue; }
            let k = keybinds::normalize(&keys);
            if mine.contains(&k) {
                return Some(format!("{} would be on two workspaces", keybinds::label(&keys)));
            }
            if let Some(o) = taken(&live, &keys, "") {
                return Some(format!("{} already opens {o}", keybinds::label(&keys)));
            }
            mine.push(k);
        }
    }
    None
}

/// Saves the workspace shortcuts unless they would land on other shortcuts.
fn save_spaces(next: &keybinds::Spaces) -> bool {
    if let Some(why) = spaces_clash(next) {
        ui::fail(&why);
        return false;
    }
    save(keybinds::set_spaces(next))
}

/// A dropdown of [`MODIFIERS`] (and `current`, if it is none of them) for one way of reaching
/// workspaces; `set` puts the pick into the workspace shortcuts.
fn modifier_row(card: &gtk::Box, title: &str, sub: &str, current: &str, set: fn(&mut keybinds::Spaces, String)) {
    let (row, end) = ui::row(None, title, sub);
    let mut options: Vec<(String, String)> = MODIFIERS.iter().map(|(v, l)| (v.to_string(), l.to_string())).collect();
    let at = match options.iter().position(|(v, _)| keybinds::normalize(v) == keybinds::normalize(current)) {
        Some(at) => at,
        None => {
            options.insert(0, (current.to_string(), keybinds::label(current)));
            0
        }
    };
    let labels: Vec<&str> = options.iter().map(|(_, l)| l.as_str()).collect();
    let pick = ui::dropdown(&labels, at as u32);
    let last = Rc::new(Cell::new(at as u32));
    pick.connect_selected_notify(move |p| {
        let i = p.selected();
        if i == last.get() { return; }
        let mut next = keybinds::spaces();
        set(&mut next, options.get(i as usize).map(|o| o.0.clone()).unwrap_or_default());
        if save_spaces(&next) { last.set(i) } else { p.set_selected(last.get()) }
    });
    end.append(&pick);
    card.append(&row);
}

/// Going to and moving the window to workspaces: a modifier for each, and each workspace's key.
fn spaces_rows(card: &gtk::Box) {
    let now = keybinds::spaces();
    modifier_row(card, "Go to a workspace", "Held with the workspace's key", &now.go, |s, m| s.go = m);
    modifier_row(card, "Move the window to a workspace", "Held with the workspace's key", &now.to, |s, m| s.to = m);
    let (row, end) = ui::row(None, "Workspace keys", "Pick a workspace, then click its key to change it");
    let names: Vec<String> = (1..=keybinds::SPACES).map(|i| format!("Workspace {i}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let which = ui::dropdown(&refs, 0);
    let key = key_button(&now.keys[0]);
    {
        let key = key.clone();
        which.connect_selected_notify(move |w| {
            let k = keybinds::spaces().keys.get(w.selected() as usize).cloned().unwrap_or_default();
            key.set_label(&if k.is_empty() { "Off".into() } else { keybinds::label(&k) });
        });
    }
    {
        let which = which.clone();
        key.connect_clicked(move |btn| {
            let at = which.selected() as usize;
            let current = keybinds::spaces().keys.get(at).cloned().unwrap_or_default();
            let btn2 = btn.clone();
            record(btn.upcast_ref(), &current, Some("Turn off"), move |picked| {
                // Only the key counts: the modifiers come from the rows above.
                let k = picked.rsplit('+').next().map(str::trim).unwrap_or("").to_string();
                let mut next = keybinds::spaces();
                let Some(slot) = next.keys.get_mut(at) else { return };
                *slot = k.clone();
                if save_spaces(&next) {
                    btn2.set_label(&if k.is_empty() { "Off".into() } else { keybinds::label(&k) });
                }
            });
        });
    }
    end.append(&which);
    end.append(&key);
    card.append(&row);
}

/// Another shortcut in effect on these keys, by its label.
fn taken(live: &[Bind], keys: &str, except: &str) -> Option<String> {
    let k = keybinds::normalize(keys);
    live.iter().find(|b| b.id != except && keybinds::normalize(&b.keys) == k).map(|b| b.label.clone())
}

fn custom_card(live: &[Bind]) -> gtk::Box {
    let card = ui::card();
    let list = keybinds::custom();
    for (i, (keys, command, label)) in list.iter().enumerate() {
        let title = if label.is_empty() { command.as_str() } else { label.as_str() };
        let (row, end) = ui::row(Some("terminal"), title, if label.is_empty() { "" } else { command });
        let remove = kit::app::control("delete", "");
        remove.set_tooltip_text(Some("Remove"));
        remove.connect_clicked(move |_| {
            let mut list = keybinds::custom();
            if i < list.len() {
                list.remove(i);
            }
            commit(keybinds::set_custom(&list));
        });
        end.append(&remove);
        let keys = key_button(keys);
        let others = live.to_vec();
        keys.connect_clicked(move |btn| {
            let others = others.clone();
            let current = keybinds::custom().get(i).map(|b| b.0.clone()).unwrap_or_default();
            record(btn.upcast_ref(), &current, None, move |picked| {
                if picked.is_empty() {
                    return;
                }
                if let Some(o) = taken(&others, &picked, &format!("custom-{i}")) {
                    ui::fail(&format!("{picked} already opens {o}"));
                    return;
                }
                let mut list = keybinds::custom();
                if let Some(e) = list.get_mut(i) {
                    e.0 = picked;
                }
                commit(keybinds::set_custom(&list));
            });
        });
        end.append(&keys);
        card.append(&row);
    }
    let (row, end) = ui::row(Some("add"), "Add a shortcut", "Run any command, such as \"firefox\" or \"kitty -e btop\"");
    let add = ui::button("Add", "accent");
    let others = live.to_vec();
    add.connect_clicked(move |btn| {
        let (btn, others) = (btn.clone(), others.clone());
        ui::prompt(ui::root(&btn), "New shortcut", "The command it runs. Next, press the keys for it.", Some(("Command", false)), "Next", move |cmd| {
            let Some(cmd) = cmd.map(|c| c.trim().to_string()).filter(|c| !c.is_empty()) else { return };
            record(btn.upcast_ref(), "", None, move |picked| {
                if picked.is_empty() {
                    return;
                }
                if let Some(o) = taken(&others, &picked, "") {
                    ui::fail(&format!("{picked} already opens {o}"));
                    return;
                }
                let mut list = keybinds::custom();
                list.push((picked, cmd.clone(), String::new()));
                commit(keybinds::set_custom(&list));
            });
        });
    });
    end.append(&add);
    card.append(&row);
    card
}

/// Saves a change, regenerates digit's Hyprland files and redraws the page.
/// Saves a change and regenerates digit's Hyprland files, without redrawing the page.
fn save(r: Result<(), String>) -> bool {
    match r.and_then(|_| kit::setup::apply()) {
        Ok(()) => true,
        Err(e) => {
            ui::fail(&e);
            false
        }
    }
}

fn commit(r: Result<(), String>) {
    if let Err(e) = r.and_then(|_| kit::setup::apply()) {
        ui::fail(&e);
        return;
    }
    KEEP.with(|k| k.set(true));
    crate::reload("keyboard");
}

const WHEEL_MODIFIERS: [&str; 4] = ["SUPER", "CTRL", "ALT", "SHIFT"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WheelDirection {
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WheelShortcut {
    modifiers: [bool; 4],
    direction: WheelDirection,
}

impl WheelShortcut {
    fn keys(self) -> Option<String> {
        let mut parts: Vec<_> = WHEEL_MODIFIERS.into_iter().zip(self.modifiers).filter_map(|(name, on)| on.then_some(name)).collect();
        if parts.is_empty() { return None; }
        parts.push(match self.direction { WheelDirection::Up => "mouse_up", WheelDirection::Down => "mouse_down" });
        Some(parts.join(" + "))
    }

    fn parse(keys: &str) -> Option<Self> {
        let mut modifiers = [false; 4];
        let mut direction = None;
        for part in keys.split('+').map(|p| p.trim().to_ascii_uppercase()) {
            match part.as_str() {
                "SUPER" => modifiers[0] = true,
                "CTRL" | "CONTROL" => modifiers[1] = true,
                "ALT" => modifiers[2] = true,
                "SHIFT" => modifiers[3] = true,
                "MOUSE_UP" | "MOUSE_DOWN" if direction.is_none() => {
                    direction = Some(if part == "MOUSE_UP" { WheelDirection::Up } else { WheelDirection::Down });
                }
                _ => return None,
            }
        }
        Some(Self { modifiers, direction: direction? })
    }
}

/// A choice in the shortcut menu: an icon and a label, styled like the other popover menus.
fn item(icon: &str, label: &str) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("pop-item");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.append(&ui::sym(icon));
    row.append(&ui::text(label, ""));
    b.set_child(Some(&row));
    b
}

/// Changes a shortcut. A small menu offers recording new keys, a scroll-wheel combination and,
/// with a `clear` label and keys to clear, removing them. `done` gets the new keys ("" = off).
/// Keys are only captured once "Record keys" is chosen; Hyprland hands every key to Settings
/// meanwhile, so keys that are already shortcuts can be recorded too.
fn record(anchor: &gtk::Widget, current: &str, clear: Option<&str>, done: impl Fn(String) + 'static) {
    let pop = gtk::Popover::new();
    kit::ui::menu_popover(&pop);
    pop.set_parent(anchor);
    pop.set_position(gtk::PositionType::Bottom);
    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_transition_duration(160);
    stack.set_hhomogeneous(false);
    stack.set_vhomogeneous(false);
    stack.set_interpolate_size(true);
    stack.set_focusable(true);
    pop.set_child(Some(&stack));
    let done = Rc::new(done);
    // Set once the popover closes with a result or by a choice, so the timeout leaves it alone.
    let finished = Rc::new(Cell::new(false));
    let capturing = Rc::new(Cell::new(false));
    let pick = {
        let (pop, finished, done) = (pop.clone(), finished.clone(), done.clone());
        Rc::new(move |keys: String| {
            finished.set(true);
            pop.popdown();
            let done = done.clone();
            // Let the popover close and the normal shortcuts come back first.
            glib::idle_add_local_once(move || done(keys));
        })
    };

    let menu = gtk::Box::new(gtk::Orientation::Vertical, 2);
    menu.add_css_class("pop-menu");
    let keys_item = item("keyboard", "Record keys");
    let wheel_item = item("mouse", "Scroll combination…");
    menu.append(&keys_item);
    menu.append(&wheel_item);
    if let Some(clear) = clear.filter(|_| !current.is_empty()) {
        let off = item("block", clear);
        let pick = pick.clone();
        off.connect_clicked(move |_| pick(String::new()));
        menu.append(&off);
    }
    stack.add_named(&menu, Some("menu"));

    let listen = gtk::Box::new(gtk::Orientation::Vertical, 4);
    listen.add_css_class("pop-panel");
    listen.append(&ui::text("Press the new keys", "name"));
    listen.append(&ui::text("Esc cancels", "sub"));
    stack.add_named(&listen, Some("keys"));

    let form = gtk::Box::new(gtk::Orientation::Vertical, 10);
    form.add_css_class("pop-panel");
    form.append(&ui::text("Scroll combination", "name"));
    let state = Rc::new(Cell::new(WheelShortcut::parse(current).unwrap_or(WheelShortcut { modifiers: [true, false, false, false], direction: WheelDirection::Up })));
    let save = ui::button("Save", "accent");
    save.set_sensitive(state.get().keys().is_some());
    let modifiers = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    modifiers.add_css_class("seg");
    for (i, label) in ["Super", "Ctrl", "Alt", "Shift"].into_iter().enumerate() {
        let chip = gtk::ToggleButton::with_label(label);
        chip.set_active(state.get().modifiers[i]);
        let (state, save) = (state.clone(), save.clone());
        chip.connect_toggled(move |b| {
            let mut next = state.get();
            next.modifiers[i] = b.is_active();
            state.set(next);
            save.set_sensitive(next.keys().is_some());
        });
        modifiers.append(&chip);
    }
    modifiers.set_homogeneous(true);
    form.append(&modifiers);
    let direction = {
        let state = state.clone();
        ui::segmented(&["Scroll up", "Scroll down"], usize::from(state.get().direction == WheelDirection::Down), move |i| {
            let mut next = state.get();
            next.direction = if i == 0 { WheelDirection::Up } else { WheelDirection::Down };
            state.set(next);
            true
        })
    };
    // Both rows fill the popover evenly instead of leaving the buttons bunched at one side.
    direction.set_homogeneous(true);
    for bar in [&modifiers, &direction] {
        let mut child = bar.first_child();
        while let Some(c) = child {
            c.set_hexpand(true);
            child = c.next_sibling();
        }
    }
    form.append(&direction);
    let hint = ui::text("Hold the modifiers and turn the wheel", "sub");
    hint.set_xalign(0.0);
    form.append(&hint);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_halign(gtk::Align::End);
    let cancel = ui::button("Cancel", "");
    {
        let pop = pop.clone();
        cancel.connect_clicked(move |_| pop.popdown());
    }
    {
        let pick = pick.clone();
        save.connect_clicked(move |_| if let Some(keys) = state.get().keys() { pick(keys) });
    }
    actions.append(&cancel);
    actions.append(&save);
    form.append(&actions);
    stack.add_named(&form, Some("wheel"));
    stack.set_visible_child_name("menu");

    {
        let (stack, capturing, pop, finished) = (stack.clone(), capturing.clone(), pop.downgrade(), finished.clone());
        keys_item.connect_clicked(move |_| {
            stack.set_visible_child_name("keys");
            stack.grab_focus();
            if !capturing.replace(true) { kit::setup::capture(true); }
            // Never leave the shortcuts off for long if something goes wrong.
            let (pop, finished) = (pop.clone(), finished.clone());
            glib::timeout_add_local_once(std::time::Duration::from_secs(20), move || {
                if let Some(p) = pop.upgrade() && !finished.get() { p.popdown(); }
            });
        });
    }
    {
        let (stack, modifiers) = (stack.clone(), modifiers.downgrade());
        wheel_item.connect_clicked(move |_| {
            stack.set_visible_child_name("wheel");
            if let Some(chip) = modifiers.upgrade().and_then(|m| m.first_child()) { chip.grab_focus(); }
        });
    }
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let (stack, pop, pick) = (stack.clone(), pop.clone(), pick.clone());
        keys.connect_key_pressed(move |_, key, _, mods| {
            // Only the "Record keys" step listens; the menu and the form keep normal keyboard use.
            if stack.visible_child_name().as_deref() != Some("keys") { return glib::Propagation::Proceed; }
            let plain = !mods.intersects(gtk::gdk::ModifierType::SUPER_MASK | gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::ALT_MASK | gtk::gdk::ModifierType::SHIFT_MASK);
            if key == gtk::gdk::Key::Escape && plain {
                pop.popdown();
                return glib::Propagation::Stop;
            }
            if let Some(k) = keybinds::from_press(key, mods) { pick(k); }
            glib::Propagation::Stop
        });
    }
    pop.add_controller(keys);
    pop.connect_closed(move |p| {
        if capturing.replace(false) { kit::setup::capture(false); }
        p.unparent();
    });
    pop.popup();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wheel_keys_use_canonical_modifier_order_and_direction() {
        let shortcut = WheelShortcut { modifiers: [true; 4], direction: WheelDirection::Up };
        assert_eq!(shortcut.keys().as_deref(), Some("SUPER + CTRL + ALT + SHIFT + mouse_up"));
        assert_eq!(WheelShortcut { modifiers: [true, false, false, true], direction: WheelDirection::Down }.keys().as_deref(), Some("SUPER + SHIFT + mouse_down"));
        for direction in [WheelDirection::Up, WheelDirection::Down] {
            assert_eq!(WheelShortcut { modifiers: [false; 4], direction }.keys(), None);
        }
    }

    #[test]
    fn existing_wheel_keys_prefill_modifiers_and_direction() {
        assert_eq!(WheelShortcut::parse("SUPER + SHIFT + mouse_down"), Some(WheelShortcut {
            modifiers: [true, false, false, true], direction: WheelDirection::Down,
        }));
        let shortcut = WheelShortcut::parse(" shift + alt + Control + super + MOUSE_UP ").unwrap();
        assert_eq!(shortcut.modifiers, [true; 4]);
        assert_eq!(shortcut.direction, WheelDirection::Up);
        // Existing unmodified wheel bindings can be edited, but cannot be saved as-is.
        assert_eq!(WheelShortcut::parse("mouse_up").unwrap().keys(), None);
        for invalid in ["", "SUPER + A", "SUPER + mouse:272", "SUPER", "SUPER + mouse_up + mouse_down", "SUPER + mouse_up + A"] {
            assert_eq!(WheelShortcut::parse(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn every_wheel_modifier_combination_round_trips() {
        for mask in 1..16 {
            for direction in [WheelDirection::Up, WheelDirection::Down] {
                let shortcut = WheelShortcut { modifiers: std::array::from_fn(|i| mask & (1 << i) != 0), direction };
                assert_eq!(WheelShortcut::parse(&shortcut.keys().unwrap()), Some(shortcut));
            }
        }
    }

    #[test]
    fn wheel_choices_use_the_existing_conflict_check() {
        let shortcut = WheelShortcut { modifiers: [true, false, false, true], direction: WheelDirection::Up };
        let keys = shortcut.keys().unwrap();
        let defaults = keybinds::defaults();
        assert_eq!(taken(&defaults, &keys, ""), Some("Previous window".into()));
        assert_eq!(taken(&defaults, &keys, "window-prev"), None);
    }
}
