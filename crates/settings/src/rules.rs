//! Settings > Window rules: how chosen apps' windows open, for Hyprland and niri alike.

use crate::desktop::save;
use crate::{prefs, ui};
use gtk::{glib, prelude::*};
use kit::winrules::{self, Rule};
use std::cell::RefCell;
use std::rc::Rc;

const OPACITY: [(f64, &str); 6] = [(1.0, "Opaque"), (0.95, "95%"), (0.9, "90%"), (0.85, "85%"), (0.8, "80%"), (0.7, "70%")];

type Rules = Rc<RefCell<Vec<Rule>>>;

pub fn build() -> gtk::Widget {
    let (scroll, body) = ui::page("Window rules", "How chosen apps' windows open. A rule matches an app, and optionally words in the window's title.");
    if kit::setup::kind().is_none() || !kit::setup::installed() {
        let card = ui::card();
        let (row, _) = ui::row(Some("warning"), "digit can't change this compositor's rules", "Window rules are written to Hyprland's or niri's config. Add them in your compositor's own config instead");
        card.append(&row);
        body.append(&card);
        return scroll.upcast();
    }
    let rules: Rules = Rc::new(RefCell::new(winrules::read(&prefs::read())));
    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.append(&list);
    fill(&list, &rules);

    body.append(&ui::heading("New rule"));
    let card = ui::card();
    let (row, end) = ui::row(Some("add"), "Add a rule", "Start empty, or from a window that's open now");
    let windows = open_apps();
    let mut names: Vec<&str> = vec!["Empty rule"];
    names.extend(windows.iter().map(String::as_str));
    let pick = kit::ui::dropdown(&names, 0);
    pick.set_valign(gtk::Align::Center);
    let add = ui::button("Add", "accent");
    add.set_valign(gtk::Align::Center);
    end.append(&pick);
    end.append(&add);
    card.append(&row);
    body.append(&card);
    let (list_for_add, rules_for_add) = (list.clone(), rules.clone());
    add.connect_clicked(move |_| {
        let app = pick.selected().checked_sub(1).and_then(|i| windows.get(i as usize)).cloned().unwrap_or_default();
        rules_for_add.borrow_mut().push(Rule { app, float: true, ..Rule::default() });
        store(&rules_for_add);
        fill(&list_for_add, &rules_for_add);
    });
    scroll.upcast()
}

/// App IDs of the windows open now, each once.
fn open_apps() -> Vec<String> {
    let mut apps: Vec<String> = kit::wm::windows().into_iter().map(|w| w.app_id).filter(|a| !a.is_empty()).collect();
    apps.sort();
    apps.dedup();
    apps
}

fn store(rules: &Rules) {
    save("window_rules", winrules::value(&rules.borrow()));
}

/// Rebuilds the cards, one per rule.
fn fill(list: &gtk::Box, rules: &Rules) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let count = rules.borrow().len();
    if count == 0 {
        let card = ui::card();
        let (row, _) = ui::row(Some("rule"), "No rules yet", "Apps' windows open the way the compositor decides");
        card.append(&row);
        list.append(&ui::heading("Rules"));
        list.append(&card);
        return;
    }
    for i in 0..count {
        list.append(&ui::heading(&format!("Rule {}", i + 1)));
        list.append(&card(list, rules, i));
    }
}

fn card(list: &gtk::Box, rules: &Rules, i: usize) -> gtk::Box {
    let rule = rules.borrow()[i].clone();
    let card = ui::card();
    let edit = |f: Box<dyn Fn(&mut Rule)>| {
        let rules = rules.clone();
        move || {
            if let Some(r) = rules.borrow_mut().get_mut(i) {
                f(r);
            }
            store(&rules);
        }
    };

    let entry = |icon: &str, title: &str, sub: &str, value: &str, hint: &str| {
        let (row, end) = ui::row(Some(icon), title, sub);
        let e = gtk::Entry::new();
        e.set_text(value);
        e.set_placeholder_text(Some(hint));
        e.set_valign(gtk::Align::Center);
        e.set_width_chars(18);
        e.set_max_length(128);
        end.append(&e);
        card.append(&row);
        e
    };
    let app = entry("apps", "App", "Its app ID, exactly: firefox, org.gnome.Nautilus…", &rule.app, "app ID");
    let save_app = edit(Box::new({
        let app = app.clone();
        move |r| r.app = app.text().trim().to_string()
    }));
    ui::bind_entry(&app, |_| Ok(()), move |_| {
        save_app();
        true
    });
    let title = entry("title", "Title contains", "Leave empty for every window of the app", &rule.title, "any title");
    let save_title = edit(Box::new({
        let title = title.clone();
        move |r| r.title = title.text().trim().to_string()
    }));
    ui::bind_entry(&title, |_| Ok(()), move |_| {
        save_title();
        true
    });

    let switch = |icon: &str, name: &str, sub: &str, on: bool, set: fn(&mut Rule, bool)| {
        let (row, end) = ui::row(Some(icon), name, sub);
        let sw = gtk::Switch::new();
        sw.set_valign(gtk::Align::Center);
        let rules = rules.clone();
        ui::bind_switch(&sw, on, move |on| {
            if let Some(r) = rules.borrow_mut().get_mut(i) {
                set(r, on);
            }
            store(&rules);
            true
        });
        end.append(&sw);
        card.append(&row);
    };
    switch("picture_in_picture", "Float", "Opens floating, above the tiled windows", rule.float, |r, on| r.float = on);
    switch("fullscreen", "Open maximized", "", rule.maximize, |r, on| r.maximize = on);
    switch("fit_screen", "Open fullscreen", "", rule.fullscreen, |r, on| r.fullscreen = on);

    let (row, end) = ui::row(Some("opacity"), "Opacity", "See-through windows show the blur behind them when it's on");
    let labels: Vec<&str> = OPACITY.iter().map(|o| o.1).collect();
    let at = OPACITY.iter().position(|o| (o.0 - rule.opacity).abs() < 0.01).unwrap_or(0);
    let pick = kit::ui::dropdown(&labels, at as u32);
    pick.set_valign(gtk::Align::Center);
    {
        let rules = rules.clone();
        pick.connect_selected_notify(move |p| {
            let Some((value, _)) = OPACITY.get(p.selected() as usize) else { return };
            if let Some(r) = rules.borrow_mut().get_mut(i) {
                r.opacity = *value;
            }
            store(&rules);
        });
    }
    end.append(&pick);
    card.append(&row);

    let (row, end) = ui::row(Some("delete"), "Remove this rule", "");
    let remove = ui::button("Remove", "danger");
    remove.set_valign(gtk::Align::Center);
    let (list, rules) = (list.clone(), rules.clone());
    remove.connect_clicked(move |_| {
        if i < rules.borrow().len() {
            rules.borrow_mut().remove(i);
        }
        store(&rules);
        // After the click finishes: the button is inside the card being rebuilt.
        let (list, rules) = (list.clone(), rules.clone());
        glib::idle_add_local_once(move || fill(&list, &rules));
    });
    end.append(&remove);
    card.append(&row);
    card
}
