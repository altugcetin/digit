//! Settings > Window rules: how chosen apps' windows open, for Hyprland and niri alike.

use crate::desktop::save;
use crate::{prefs, ui};
use gtk::prelude::*;
use kit::winrules::{self, Rule};
use std::cell::RefCell;
use std::rc::Rc;

const OPACITY: [(f64, &str); 6] = [(1.0, "Opaque"), (0.95, "95%"), (0.9, "90%"), (0.85, "85%"), (0.8, "80%"), (0.7, "70%")];

type Rules = Rc<RefCell<Vec<Rule>>>;
/// Which rules' cards are open, in the rules' order.
type Open = Rc<RefCell<Vec<bool>>>;

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
    // The rules there already start folded, so the page shows them all at a glance.
    let open: Open = Rc::new(RefCell::new(vec![false; rules.borrow().len()]));
    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.append(&list);
    fill(&list, &rules, &open);

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
        // Nothing is chosen for it yet: the user picks what it does.
        rules_for_add.borrow_mut().push(Rule { app, ..Rule::default() });
        open.borrow_mut().push(true);
        store(&rules_for_add);
        fill(&list_for_add, &rules_for_add, &open);
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

/// What a rule's folded card says: the windows it matches, and what it does to them.
fn heading(r: &Rule) -> (String, String) {
    let (app, title) = (r.app.trim(), r.title.trim());
    let name = match (app.is_empty(), title.is_empty()) {
        (true, true) => "New rule".to_string(),
        (false, true) => app.to_string(),
        (true, false) => format!("Windows titled \u{201c}{title}\u{201d}"),
        (false, false) => format!("{app}, titled \u{201c}{title}\u{201d}"),
    };
    let mut does = Vec::new();
    if r.float {
        does.push("Float".to_string());
    }
    if r.maximize {
        does.push("Maximized".to_string());
    }
    if r.fullscreen {
        does.push("Fullscreen".to_string());
    }
    if r.opacity < 0.995 {
        does.push(format!("{}% opacity", (r.opacity * 100.0).round()));
    }
    if r.no_share {
        does.push("Hidden from sharing".to_string());
    }
    if r.no_focus {
        does.push("Without focus".to_string());
    }
    if r.workspace > 0 {
        does.push(format!("Workspace {}", r.workspace));
    }
    if r.tearing {
        does.push("Tearing".to_string());
    }
    let does = if does.is_empty() { "Changes nothing yet".to_string() } else { does.join(" \u{b7} ") };
    (name, does)
}

/// Rebuilds the cards, one per rule.
fn fill(list: &gtk::Box, rules: &Rules, open: &Open) {
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
    open.borrow_mut().resize(count, false);
    for i in 0..count {
        list.append(&ui::heading(&format!("Rule {}", i + 1)));
        list.append(&card(list, rules, open, i));
    }
}

fn card(list: &gtk::Box, rules: &Rules, open: &Open, i: usize) -> gtk::Box {
    let rule = rules.borrow()[i].clone();
    let outer = ui::card();
    // The head says what the rule does and folds the rest away.
    let (name, does) = heading(&rule);
    let (head, end) = ui::row(Some("rule"), &name, &does);
    let remove = kit::app::control("delete", "danger");
    remove.set_tooltip_text(Some("Remove this rule"));
    {
        let (list, rules, open) = (list.clone(), rules.clone(), open.clone());
        remove.connect_clicked(move |b| {
            let name = rules.borrow().get(i).map(|r| heading(r).0).unwrap_or_default();
            let (list, rules, open) = (list.clone(), rules.clone(), open.clone());
            ui::ask(ui::root(b).as_ref(), &format!("Remove {name}?"), "Its windows open the way the compositor decides again.", "Cancel", "Remove", "danger", move || {
                if i < rules.borrow().len() {
                    rules.borrow_mut().remove(i);
                }
                if i < open.borrow().len() {
                    open.borrow_mut().remove(i);
                }
                store(&rules);
                fill(&list, &rules, &open);
            });
        });
    }
    end.append(&remove);
    let arrow = ui::sym("expand_more");
    end.append(&arrow);
    head.set_cursor_from_name(Some("pointer"));
    outer.append(&head);
    let reveal = gtk::Revealer::new();
    reveal.set_transition_type(gtk::RevealerTransitionType::SlideDown);
    reveal.set_transition_duration(200);
    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    reveal.set_child(Some(&card));
    outer.append(&reveal);
    let shown = open.borrow().get(i).copied().unwrap_or(false);
    reveal.set_reveal_child(shown);
    arrow.set_label(if shown { "expand_less" } else { "expand_more" });
    let click = gtk::GestureClick::new();
    {
        let (reveal, arrow, open) = (reveal.clone(), arrow.clone(), open.clone());
        let trash = remove.downgrade();
        click.connect_released(move |g, _, x, y| {
            // The trash button removes the rule; it doesn't fold it too.
            let on_trash = g.widget().and_then(|w| w.pick(x, y, gtk::PickFlags::DEFAULT)).zip(trash.upgrade()).is_some_and(|(hit, trash)| hit == trash || hit.is_ancestor(&trash));
            if on_trash {
                return;
            }
            let now = !reveal.reveals_child();
            reveal.set_reveal_child(now);
            arrow.set_label(if now { "expand_less" } else { "expand_more" });
            if let Some(o) = open.borrow_mut().get_mut(i) {
                *o = now;
            }
        });
    }
    head.add_controller(click);
    // The head follows every change made below it.
    let said: Rc<dyn Fn()> = {
        let (rules, head) = (rules.clone(), head.downgrade());
        Rc::new(move || {
            let (Some(head), Some(r)) = (head.upgrade(), rules.borrow().get(i).cloned()) else { return };
            let (name, does) = heading(&r);
            if let Some(l) = crate::desktop::name_label(head.upcast_ref()) {
                l.set_text(&name);
            }
            if let Some(l) = crate::desktop::sub_label(head.upcast_ref()) {
                l.set_text(&does);
            }
        })
    };
    let edit = |f: Box<dyn Fn(&mut Rule)>| {
        let (rules, said) = (rules.clone(), said.clone());
        move || {
            if let Some(r) = rules.borrow_mut().get_mut(i) {
                f(r);
            }
            store(&rules);
            said();
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
        let (rules, said) = (rules.clone(), said.clone());
        ui::bind_switch(&sw, on, move |on| {
            if let Some(r) = rules.borrow_mut().get_mut(i) {
                set(r, on);
            }
            store(&rules);
            said();
            true
        });
        end.append(&sw);
        card.append(&row);
    };
    switch("picture_in_picture", "Float", "Opens floating, above the tiled windows", rule.float, |r, on| r.float = on);
    switch("fullscreen", "Open maximized", "", rule.maximize, |r, on| r.maximize = on);
    switch("fit_screen", "Open fullscreen", "", rule.fullscreen, |r, on| r.fullscreen = on);
    switch("visibility_off", "Hide from screen sharing", "Shows blank in screen shares and recordings", rule.no_share, |r, on| r.no_share = on);
    switch("do_not_touch", "Open without focus", "Keeps typing in the window you're using", rule.no_focus, |r, on| r.no_focus = on);
    // niri's workspaces have no numbers to open on, and niri doesn't tear.
    if kit::setup::kind() != Some(kit::setup::Kind::Niri) {
        switch("speed", "Allow tearing", "In fullscreen it shows each frame at once, for less input lag. Needs Allow tearing in Windows", rule.tearing, |r, on| r.tearing = on);
        let (row, end) = ui::row(Some("workspaces"), "Open on workspace", "Without switching to it");
        let labels: Vec<String> = std::iter::once("Where it opens".to_string()).chain((1..=10).map(|n| n.to_string())).collect();
        let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
        let pick = kit::ui::dropdown(&labels, rule.workspace.min(10));
        pick.set_valign(gtk::Align::Center);
        let (rules, said) = (rules.clone(), said.clone());
        pick.connect_selected_notify(move |p| {
            if let Some(r) = rules.borrow_mut().get_mut(i) {
                r.workspace = p.selected();
            }
            store(&rules);
            said();
        });
        end.append(&pick);
        card.append(&row);
    }

    let (row, end) = ui::row(Some("opacity"), "Opacity", "See-through windows show the blur behind them when it's on");
    let labels: Vec<&str> = OPACITY.iter().map(|o| o.1).collect();
    let at = OPACITY.iter().position(|o| (o.0 - rule.opacity).abs() < 0.01).unwrap_or(0);
    let pick = kit::ui::dropdown(&labels, at as u32);
    pick.set_valign(gtk::Align::Center);
    {
        let (rules, said) = (rules.clone(), said.clone());
        pick.connect_selected_notify(move |p| {
            let Some((value, _)) = OPACITY.get(p.selected() as usize) else { return };
            if let Some(r) = rules.borrow_mut().get_mut(i) {
                r.opacity = *value;
            }
            store(&rules);
            said();
        });
    }
    end.append(&pick);
    card.append(&row);

    outer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folded_rule_says_what_it_matches_and_does() {
        assert_eq!(heading(&Rule::default()), ("New rule".to_string(), "Changes nothing yet".to_string()));
        let r = Rule { app: "mpv".into(), float: true, opacity: 0.9, workspace: 3, ..Rule::default() };
        assert_eq!(heading(&r), ("mpv".to_string(), "Float \u{b7} 90% opacity \u{b7} Workspace 3".to_string()));
        let r = Rule { title: "Picture-in-Picture".into(), tearing: true, ..Rule::default() };
        assert_eq!(heading(&r).0, "Windows titled \u{201c}Picture-in-Picture\u{201d}");
        assert_eq!(heading(&r).1, "Tearing");
    }
}
