use crate::{flatpak, guard, pk, prefs, ui};
use gtk::{gio, glib, prelude::*};
use std::rc::Rc;

#[derive(Clone)]
enum Source {
    System,
    Flatpak { id: String, system: bool },
    Local,
}

#[derive(Clone)]
struct Entry {
    name: String,
    path: String,
    icon: Option<gio::Icon>,
    source: Source,
    version: String,
}

fn collect(flatpaks: &[flatpak::App]) -> Vec<Entry> {
    let home = glib::home_dir().to_string_lossy().to_string();
    let mut out: Vec<Entry> = gio::AppInfo::all()
        .into_iter()
        .filter(|a| a.should_show())
        .filter_map(|a| {
            let d = a.downcast_ref::<gio_unix::DesktopAppInfo>()?;
            let path = d.filename()?.to_string_lossy().to_string();
            let source = if path.contains("/flatpak/exports/share/applications/") {
                let id = std::path::Path::new(&path).file_stem()?.to_string_lossy().to_string();
                let system = !path.starts_with(&home);
                Source::Flatpak { id, system }
            } else if path.starts_with(&home) {
                Source::Local
            } else {
                Source::System
            };
            let version = match &source {
                Source::Flatpak { id, .. } => flatpaks.iter().find(|f| f.id == *id).map(|f| f.version.clone()).unwrap_or_default(),
                _ => String::new(),
            };
            Some(Entry { name: a.display_name().to_string(), path, icon: a.icon(), source, version })
        })
        .collect();
    out.sort_by_key(|e| e.name.to_lowercase());
    out.dedup_by(|a, b| a.path == b.path);
    out
}

thread_local! {
    /// An app to remove once the list is in, asked for by the app grid ("firefox.desktop").
    static PENDING: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Removes app `id` (its desktop file's name) as its Uninstall button would, once the Apps
/// page has listed it.
pub fn request_uninstall(id: &str) {
    PENDING.with(|p| *p.borrow_mut() = Some(id.to_string()));
}

pub fn build() -> gtk::Widget {
    let (scroll, body) = ui::page("Apps", "Choose which apps open your links and files, which appear in the app grid and Spotlight, and remove apps you no longer need. Protected system components can't be removed.");
    crate::defaults::section(&body);
    body.append(&ui::heading("Installed apps"));
    let search = gtk::SearchEntry::new();
    search.add_css_class("search");
    search.set_placeholder_text(Some("Search apps"));
    search.set_margin_top(10);
    search.set_margin_bottom(6);
    body.append(&search);
    let list = Rc::new(ui::card());
    body.append(&*list);
    load(list.clone(), search.clone());
    let l = list.clone();
    search.connect_search_changed(move |s| filter(&l, &s.text()));
    scroll.upcast()
}

fn filter(list: &gtk::Box, query: &str) {
    let q = query.to_lowercase();
    let mut child = list.first_child();
    while let Some(c) = child {
        let name = c.widget_name().to_string();
        c.set_visible(q.is_empty() || name.contains(&q));
        child = c.next_sibling();
    }
}

fn load(list: Rc<gtk::Box>, search: gtk::SearchEntry) {
    ui::clear(&list);
    list.append(&ui::row(Some("apps"), "Loading apps…", "").0);
    glib::spawn_future_local(async move {
        let flatpaks = if flatpak::available() { flatpak::installed().await.unwrap_or_default() } else { Vec::new() };
        let entries = collect(&flatpaks);
        ui::clear(&list);
        for e in entries {
            let (sub, kind) = match &e.source {
                Source::System => ("System package".to_string(), ""),
                Source::Flatpak { system, .. } => (format!("Flatpak · {}{}", if *system { "system" } else { "user" }, if e.version.is_empty() { String::new() } else { format!(" · {}", e.version) }), "accent"),
                Source::Local => ("Installed in your home folder".to_string(), ""),
            };
            let (row, end) = ui::row(None, &e.name, &sub);
            let image = match &e.icon {
                Some(i) => gtk::Image::from_gicon(i),
                None => gtk::Image::from_icon_name("application-x-executable"),
            };
            image.set_pixel_size(32);
            row.prepend(&image);
            row.set_widget_name(&e.name.to_lowercase());
            end.append(&ui::badge(
                match e.source {
                    Source::System => "System",
                    Source::Flatpak { .. } => "Flatpak",
                    Source::Local => "Local",
                },
                kind,
            ));
            let id = std::path::Path::new(&e.path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if !matches!(e.source, Source::Local) {
                let b = ui::button("Uninstall", "danger");
                end.append(&b);
                let entry = e.clone();
                let l = list.clone();
                let s = search.clone();
                b.connect_clicked(move |b| uninstall(b, entry.clone(), l.clone(), s.clone()));
                if PENDING.with(|p| p.borrow().as_deref() == Some(id.as_str())) {
                    PENDING.with(|p| p.borrow_mut().take());
                    let b = b.clone();
                    glib::idle_add_local_once(move || b.emit_clicked());
                }
            }
            // Shown in the app grid and Spotlight; tools that come with other software start off.
            let t = prefs::read();
            let shown = !kit::hidden::hidden(&id, &kit::hidden::list(&t, "apps_hidden"), &kit::hidden::list(&t, "apps_shown"));
            let sw = gtk::Switch::new();
            sw.set_valign(gtk::Align::Center);
            sw.set_tooltip_text(Some("Show in the app grid and Spotlight"));
            ui::bind_switch(&sw, shown, move |on| crate::desktop::saved(kit::hidden::save(&id, !on)));
            end.append(&sw);
            list.append(&row);
        }
        filter(&list, &search.text());
    });
}

fn uninstall(button: &gtk::Button, e: Entry, list: Rc<gtk::Box>, search: gtk::SearchEntry) {
    let parent = ui::root(button);
    match e.source.clone() {
        Source::Flatpak { id, system } => {
            let name = e.name.clone();
            ui::confirm(
                parent,
                &format!("Uninstall {name}?"),
                &format!("The Flatpak {id} will be removed from the {} installation. Its data in your home folder is kept.", if system { "system" } else { "user" }),
                &[],
                "Uninstall",
                "danger",
                move || {
                    glib::spawn_future_local(async move {
                        if !ui::begin() {
                            return;
                        }
                        ui::busy(&format!("Removing {name}"), None);
                        let r = flatpak::uninstall(&id, system).await;
                        ui::end();
                        match r {
                            Ok(()) => ui::done(&format!("{name} removed")),
                            Err(e) => ui::fail(&e),
                        }
                        load(list, search);
                    });
                },
            );
        }
        Source::System => {
            glib::spawn_future_local(async move {
                if !ui::begin() {
                    return;
                }
                ui::busy(&format!("Finding the package for {}", e.name), None);
                let owner = pk::owner(&e.path).await;
                let pkg = match owner {
                    Ok(Some(p)) if guard::package_id(&p.id) => p,
                    Ok(_) => {
                        ui::end();
                        ui::fail("No installed package owns this app");
                        return;
                    }
                    Err(err) => {
                        ui::end();
                        ui::fail(&err);
                        return;
                    }
                };
                if guard::protected(pkg.name()) {
                    ui::end();
                    ui::confirm(parent, "This app is part of the system", &format!("{} belongs to {}, a protected system component. digit won't remove it.", e.name, pkg.name()), &[], "", "", || {});
                    return;
                }
                let sim = pk::remove(std::slice::from_ref(&pkg.id), true, ui::progress("Checking what would be removed")).await;
                ui::end();
                let changes = match sim {
                    Ok(c) => c,
                    Err(err) => {
                        ui::fail(&err);
                        return;
                    }
                };
                ui::done("Check complete, nothing was changed");
                let items: Vec<String> = changes.iter().map(|p| format!("{} {}", p.name(), p.version())).collect();
                let blocked: Vec<String> = changes.iter().filter(|p| guard::protected(p.name())).map(|p| p.name().to_string()).collect();
                if !blocked.is_empty() {
                    ui::confirm(parent, "Removal blocked", &format!("Removing {} would also remove protected system packages: {}. Nothing was changed.", pkg.name(), blocked.join(", ")), &items, "", "", || {});
                    return;
                }
                if changes.len() > guard::MAX_REMOVALS {
                    ui::confirm(parent, "Too many dependent packages", &format!("Removing {} would remove {} packages. That is unusually many, so digit stops here. Nothing was changed.", pkg.name(), changes.len()), &items, "", "", || {});
                    return;
                }
                let id = pkg.id.clone();
                let name = e.name.clone();
                ui::confirm(parent, &format!("Uninstall {name}?"), &format!("{} package{} will be removed. Your personal files are kept.", changes.len(), if changes.len() == 1 { "" } else { "s" }), &items, "Uninstall", "danger", move || {
                    glib::spawn_future_local(async move {
                        if !ui::begin() {
                            return;
                        }
                        let r = pk::remove(&[id], false, ui::progress(&format!("Removing {name}"))).await;
                        ui::end();
                        match r {
                            Ok(_) => ui::done(&format!("{name} removed")),
                            Err(err) => ui::fail(&err),
                        }
                        load(list, search);
                    });
                });
            });
        }
        Source::Local => {}
    }
}
