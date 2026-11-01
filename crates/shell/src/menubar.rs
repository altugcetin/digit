use crate::app::App;
use crate::popmenu::{self, Row};
use gio_unix::DesktopAppInfo;
use gtk::{gdk, gio, glib, prelude::*};
use std::rc::Rc;

fn spawn(argv: &[&str]) {
    let argv: Vec<&std::ffi::OsStr> = argv.iter().map(std::ffi::OsStr::new).collect();
    let _ = gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE);
}

fn blocking(f: impl FnOnce() + Send + 'static) {
    glib::spawn_future_local(async move {
        let _ = gio::spawn_blocking(f).await;
    });
}

pub fn system(app: &Rc<App>, monitor: Option<gdk::Monitor>, x: f64, y: f64) {
    // The keys each one is on now, as set in Settings > Keyboard.
    let keys = |id: &str| kit::keybinds::shown(id).unwrap_or_default();
    let rows = vec![
        Row::item("About This Computer", || spawn(&["digit-settings", "overview"])),
        Row::Sep,
        Row::item("System Settings…", || spawn(&["digit-settings"])).hint(&keys("settings")),
        Row::item("App Store…", || spawn(&["digit-store"])),
        Row::item("Files", || spawn(&["digit-files"])).hint(&keys("files")),
        Row::item("Show All Windows", || blocking(crate::compositor::overview)).hint(&keys("overview")).off(kit::wm::which() == kit::wm::Wm::Hyprland),
        Row::Sep,
        Row::item("Welcome Tour…", || spawn(&["digit-settings", "welcome"])),
        Row::Sep,
        Row::item("Lock Screen", || spawn(&["digitctl", "key", "lock"])).hint(&keys("lock")),
        Row::item("Log Out…", || spawn(&["digitctl", "key", "session"])).hint(&keys("session")),
    ];
    popmenu::show(app, monitor, x, y, &kit::distro::name(), rows);
}

pub fn focused(app: &Rc<App>) -> Option<(kit::wm::Win, String, Option<usize>)> {
    let win = kit::wm::focused_window().or_else(|| kit::wm::windows().into_iter().max_by_key(|w| w.stamp))?;
    let entry = app.apps.find(&win.app_id);
    let name = app.apps.display_name(&win.app_id);
    Some((win, name, entry))
}

pub fn app_menu(app: &Rc<App>, monitor: Option<gdk::Monitor>, x: f64, y: f64) -> bool {
    let Some((win, name, entry)) = focused(app) else { return false };
    let app = app.clone();
    glib::spawn_future_local(async move {
        let gtk = crate::appmenu::gtk_actions(&win.app_id).await;
        app_rows(&app, monitor, x, y, win, name, entry, gtk);
    });
    true
}

#[allow(clippy::too_many_arguments)]
fn app_rows(app: &Rc<App>, monitor: Option<gdk::Monitor>, x: f64, y: f64, win: kit::wm::Win, name: String, entry: Option<usize>, gtk: Option<crate::appmenu::Gtk>) {
    let mut rows = Vec::new();
    let has = |a: &str| gtk.as_ref().is_some_and(|g| g.actions.iter().any(|x| x == a));
    let act = |a: &str| {
        let g = gtk.as_ref().map(|g| (g.name.clone(), g.path.clone()));
        let a = a.to_string();
        move || {
            if let Some((n, p)) = g.clone() {
                crate::appmenu::activate(n, p, a.clone());
            }
        }
    };
    let profile = crate::appmenu::profile(&win.app_id);
    let send = crate::appmenu::can_send();
    if has("about") {
        rows.push(Row::item(format!("About {name}"), act("about")));
        rows.push(Row::Sep);
    }
    let pref = ["preferences", "prefs", "settings"].into_iter().find(|a| has(a));
    if let Some(p) = pref {
        rows.push(Row::item("Settings…", act(p)).hint("Ctrl+,"));
    } else if let Some(keys) = profile.settings.filter(|_| send) {
        let id = win.id;
        rows.push(Row::item("Settings…", move || crate::appmenu::send(id, keys)).hint(&crate::appmenu::hint(keys)));
    }
    if has("shortcuts") {
        rows.push(Row::item("Keyboard Shortcuts", act("shortcuts")).hint("Ctrl+?"));
    }
    if has("help") {
        rows.push(Row::item("Help", act("help")).hint("F1"));
    }
    if !rows.is_empty() && !matches!(rows.last(), Some(Row::Sep)) {
        rows.push(Row::Sep);
    }
    if let Some(i) = entry {
        let a = app.clone();
        rows.push(Row::item("New Window", move || {
            let list = a.apps.list();
            if let Some(e) = list.get(i) {
                a.apps.launch(e);
            }
        }));
        let list = app.apps.list();
        if let Some(d) = list.get(i).and_then(|e| e.info.downcast_ref::<DesktopAppInfo>().cloned()) {
            let actions: Vec<String> = d.list_actions().iter().map(|s| s.to_string()).collect();
            if !actions.is_empty() {
                rows.push(Row::Sep);
                for action in actions.into_iter().take(8) {
                    let label = d.action_name(&action).to_string();
                    let d = d.clone();
                    rows.push(Row::item(label, move || {
                        if let Some(display) = gdk::Display::default() {
                            let context = display.app_launch_context();
                            crate::apps::prepare(context.upcast_ref());
                            d.launch_action(&action, Some(&context));
                        }
                    }));
                }
            }
        }
        rows.push(Row::Sep);
    }
    let id = win.app_id.clone();
    rows.push(Row::item(format!("Hide {name}"), move || {
        let id = id.clone();
        blocking(move || {
            kit::wm::minimize(&id);
        });
    }));
    let a = app.clone();
    let key = entry.and_then(|i| app.apps.list().get(i).map(|e| e.id.clone())).unwrap_or_else(|| win.app_id.clone());
    let app_id = win.app_id.clone();
    let graceful = has("quit").then(|| act("quit"));
    rows.push(
        Row::item(format!("Quit {name}"), move || {
            if let Some(q) = &graceful {
                q();
                return;
            }
            let ids: Vec<u64> = kit::wm::windows().into_iter().filter(|w| w.app_id == app_id).map(|w| w.id).collect();
            crate::quit::quit(&a, &key, ids);
        })
        .danger()
        .hint("Ctrl+Q"),
    );
    popmenu::show(app, monitor, x, y, "", rows);
}

pub fn section(app: &Rc<App>, monitor: Option<gdk::Monitor>, x: f64, y: f64, index: usize) {
    let Some((win, _, _)) = focused(app) else { return };
    let profile = crate::appmenu::profile(&win.app_id);
    let Some((_, items)) = profile.menus.get(index) else { return };
    let send = crate::appmenu::can_send();
    let id = win.id;
    let rows = items
        .iter()
        .map(|(label, keys)| {
            if label.is_empty() {
                Row::Sep
            } else {
                let keys: &'static str = keys;
                Row::item(*label, move || crate::appmenu::send(id, keys)).hint(&crate::appmenu::hint(keys)).off(!send)
            }
        })
        .collect();
    popmenu::show(app, monitor, x, y, "", rows);
}

pub fn window_menu(app: &Rc<App>, monitor: Option<gdk::Monitor>, x: f64, y: f64) {
    let Some((win, _, _)) = focused(app) else { return };
    let id = win.id;
    let act = |a: kit::wm::WinAct| move || blocking(move || {
        kit::wm::window_act(id, a);
    });
    let multi = gdk::Display::default().is_some_and(|d| d.monitors().n_items() > 1);
    let app_id = win.app_id.clone();
    let rows = vec![
        Row::item("Fullscreen", act(kit::wm::WinAct::Fullscreen)),
        Row::item("Maximize", act(kit::wm::WinAct::Maximize)),
        Row::item(if win.floating { "Tile" } else { "Float" }, act(kit::wm::WinAct::Float)),
        Row::item("Center", act(kit::wm::WinAct::Center)),
        Row::item("Move to Next Display", act(kit::wm::WinAct::NextOutput)).off(!multi),
        Row::Sep,
        Row::item("Minimize", move || {
            let id = app_id.clone();
            blocking(move || {
                kit::wm::minimize(&id);
            });
        }),
        Row::item("Close Window", move || blocking(move || {
            kit::wm::close(id);
        }))
        .danger(),
    ];
    popmenu::show(app, monitor, x, y, if win.title.is_empty() { "Window" } else { win.title.as_str() }, rows);
}
