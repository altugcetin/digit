use crate::ui;
use gtk::{glib, prelude::*};
use kit::keybinds::{self, Bind};
use std::cell::Cell;
use std::rc::Rc;

pub fn build() -> gtk::Widget {
    let (scroll, body) = ui::page("Keyboard", "digit's shortcuts. Click a shortcut to record new keys; changes apply right away.");
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
    let live = kit::setup::shortcuts();
    // Only what the compositor can do: niri has no scratchpad or pinned windows, for one.
    let all: Vec<Bind> = keybinds::all().into_iter().filter(kit::setup::supports).collect();
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
        }
        card.append(&default_row(b, &live));
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
    let b = ui::button(if keys.is_empty() { "Off" } else { keys }, "");
    b.add_css_class("keycap");
    b.set_tooltip_text(Some("Click, then press the new keys"));
    b
}

fn default_row(b: &Bind, _live: &[Bind]) -> gtk::Box {
    let (row, end) = ui::row(None, &b.label, "");
    let keys = key_button(&b.keys);
    // A change updates this row in place; redrawing the whole page would flicker.
    let reset = kit::app::control("restart_alt", "");
    reset.set_tooltip_text(Some("Back to the default"));
    reset.set_visible(keybinds::changed(&b.id));
    {
        let (id, keys) = (b.id.clone(), keys.clone());
        reset.connect_clicked(move |r| {
            if save(keybinds::reset(&id)) {
                let def = keybinds::defaults().into_iter().find(|d| d.id == id).map(|d| d.keys).unwrap_or_default();
                keys.set_label(if def.is_empty() { "Off" } else { &def });
                r.set_visible(false);
            }
        });
    }
    end.append(&reset);
    let (id, reset_btn) = (b.id.clone(), reset.clone());
    keys.connect_clicked(move |btn| {
        let (id, btn2, reset_btn) = (id.clone(), btn.clone(), reset_btn.clone());
        record(btn.upcast_ref(), move |picked| {
            if !picked.is_empty() {
                if let Some(o) = taken(&kit::setup::shortcuts(), &picked, &id) {
                    ui::fail(&format!("{picked} already opens {o}"));
                    return;
                }
            }
            if save(keybinds::set_keys(&id, &picked)) {
                btn2.set_label(if picked.is_empty() { "Off" } else { &picked });
                reset_btn.set_visible(keybinds::changed(&id));
            }
        });
    });
    end.append(&keys);
    row
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
            record(btn.upcast_ref(), move |picked| {
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
            record(btn.upcast_ref(), move |picked| {
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

/// Records the next key combination pressed: `done` gets the keys, or "" when Backspace turns
/// the shortcut off. Escape cancels. Hyprland hands every key to Settings meanwhile, so keys
/// that are already shortcuts can be recorded too.
fn record(anchor: &gtk::Widget, done: impl Fn(String) + 'static) {
    let pop = gtk::Popover::new();
    kit::ui::menu_popover(&pop);
    pop.set_parent(anchor);
    let col = gtk::Box::new(gtk::Orientation::Vertical, 4);
    col.set_margin_top(6);
    col.set_margin_bottom(6);
    col.set_margin_start(10);
    col.set_margin_end(10);
    col.set_focusable(true);
    col.append(&ui::text("Press the new shortcut", "name"));
    col.append(&ui::text("Esc cancels · Backspace turns it off", "sub"));
    pop.set_child(Some(&col));
    let finished = Rc::new(Cell::new(false));
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let (pop, finished) = (pop.clone(), finished.clone());
        let done = Rc::new(done);
        keys.connect_key_pressed(move |_, key, _, mods| {
            let plain = !mods.intersects(gtk::gdk::ModifierType::SUPER_MASK | gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::ALT_MASK | gtk::gdk::ModifierType::SHIFT_MASK);
            let picked = match key {
                gtk::gdk::Key::Escape if plain => None,
                gtk::gdk::Key::BackSpace if plain => Some(String::new()),
                _ => match keybinds::from_press(key, mods) {
                    Some(k) => Some(k),
                    None => return glib::Propagation::Stop,
                },
            };
            finished.set(true);
            pop.popdown();
            if let Some(k) = picked {
                let done = done.clone();
                // Let the popover close and the normal shortcuts come back first.
                glib::idle_add_local_once(move || done(k));
            }
            glib::Propagation::Stop
        });
    }
    pop.add_controller(keys);
    pop.connect_closed(move |p| {
        kit::setup::capture(false);
        p.unparent();
    });
    kit::setup::capture(true);
    pop.popup();
    col.grab_focus();
    // Never leave the shortcuts off for long if something goes wrong.
    let weak = pop.downgrade();
    glib::timeout_add_local_once(std::time::Duration::from_secs(20), move || {
        if let Some(p) = weak.upgrade() {
            if !finished.get() {
                p.popdown();
            }
        }
    });
}
