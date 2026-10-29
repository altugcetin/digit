use crate::{flatpak, fw, pk, system, ui};
use gtk::{glib, prelude::*};
use std::rc::Rc;

fn rolling() -> bool {
    let id = system::os_id();
    id.contains("tumbleweed") || id.contains("slowroll") || id.contains("microos")
}

pub fn build() -> gtk::Widget {
    let (scroll, body) = ui::page("Updates", "Nothing is installed without your confirmation. Packages must be signed by a trusted repository.");
    body.append(&ui::heading("System packages"));
    let sys = Rc::new(ui::card());
    body.append(&*sys);
    body.append(&ui::heading("Flatpak apps"));
    let apps = Rc::new(ui::card());
    body.append(&*apps);
    body.append(&ui::heading("Firmware"));
    let firm = Rc::new(ui::card());
    body.append(&*firm);
    load_system(sys.clone(), false);
    load_flatpak(apps.clone());
    load_firmware(firm.clone());
    scroll.upcast()
}

fn load_system(card: Rc<gtk::Box>, refresh: bool) {
    ui::clear(&card);
    let (row, _) = ui::row(Some("deployed_code"), if refresh { "Refreshing repositories…" } else { "Checking for updates…" }, "");
    card.append(&row);
    glib::spawn_future_local(async move {
        if refresh {
            if !ui::begin() {
                return;
            }
            let r = pk::refresh(ui::progress("Refreshing")).await;
            ui::end();
            match r {
                Ok(()) => ui::done("Repositories refreshed"),
                Err(e) => ui::fail(&e),
            }
        }
        let result = pk::updates().await;
        ui::clear(&card);
        let list = match result {
            Ok(l) => l,
            Err(e) => {
                let (row, _) = ui::row(Some("error"), "Could not check for updates", &e);
                card.append(&row);
                return;
            }
        };
        let security = list.iter().filter(|p| p.info == pk::INFO_SECURITY).count();
        let important = list.iter().filter(|p| matches!(p.info, pk::INFO_IMPORTANT | pk::INFO_CRITICAL)).count();
        let (title, sub) = if list.is_empty() {
            ("Your system is up to date".to_string(), String::new())
        } else {
            (format!("{} updates available", list.len()), [(security, "security"), (important, "important")].iter().filter(|(n, _)| *n > 0).map(|(n, l)| format!("{n} {l}")).collect::<Vec<_>>().join(" · "))
        };
        let (row, end) = ui::row(Some(if list.is_empty() { "check_circle" } else { "system_update_alt" }), &title, &sub);
        let check = ui::button("Check", "");
        end.append(&check);
        let c = card.clone();
        check.connect_clicked(move |_| load_system(c.clone(), true));
        if !list.is_empty() {
            let update = ui::button("Update", "accent");
            end.append(&update);
            let ids: Vec<String> = list.iter().map(|p| p.id.clone()).collect();
            let c = card.clone();
            update.connect_clicked(move |b| update_system(b, ids.clone(), c.clone()));
        }
        card.append(&row);
        if !list.is_empty() {
            let mut sorted = list.clone();
            sorted.sort_by_key(|p| (p.info != pk::INFO_SECURITY, p.name().to_lowercase()));
            let exp = gtk::Expander::new(Some("Show packages"));
            exp.set_margin_start(14);
            exp.set_margin_bottom(6);
            let pending = std::cell::RefCell::new(Some(sorted));
            exp.connect_expanded_notify(move |e| {
                if !e.is_expanded() {
                    return;
                }
                let Some(sorted) = pending.borrow_mut().take() else { return };
                let inner = gtk::Box::new(gtk::Orientation::Vertical, 2);
                for p in sorted.iter().take(300) {
                    let (r, end) = ui::row(None, p.name(), &format!("{} · {}", p.version(), p.repo()));
                    if p.info == pk::INFO_SECURITY {
                        end.append(&ui::badge("Security", "warn"));
                    }
                    inner.append(&r);
                }
                e.set_child(Some(&inner));
            });
            card.append(&exp);
        }
    });
}

fn update_system(button: &gtk::Button, ids: Vec<String>, card: Rc<gtk::Box>) {
    let parent = ui::root(button);
    if rolling() {
        ui::confirm(
            parent,
            "Update with zypper dup",
            "Rolling releases are updated with a full distribution upgrade. It can ask about conflicts or vendor changes, so it opens in a terminal where you see every step and answer each question. Your password goes to sudo in the terminal, never to this app.",
            &[],
            "Open terminal",
            "accent",
            || {
                if let Err(e) = ui::terminal(&["sudo", "zypper", "dup"]) {
                    ui::fail(&format!("Could not open a terminal: {e}"));
                }
            },
        );
        return;
    }
    let ids: Vec<String> = ids.into_iter().filter(|i| crate::guard::package_id(i)).collect();
    glib::spawn_future_local(async move {
        if !ui::begin() {
            return;
        }
        let sim = pk::update(&ids, true, ui::progress("Checking")).await;
        ui::end();
        match sim {
            Ok(changes) => {
                ui::done("Ready to update");
                let items: Vec<String> = changes.iter().map(|p| format!("{} {}", p.name(), p.version())).collect();
                ui::confirm(parent, "Install updates?", &format!("{} packages will change.", changes.len()), &items, "Update", "accent", move || {
                    glib::spawn_future_local(async move {
                        if !ui::begin() {
                            return;
                        }
                        let r = pk::update(&ids, false, ui::progress("Updating")).await;
                        ui::end();
                        match r {
                            Ok(_) => ui::done("Updates installed"),
                            Err(e) => ui::fail(&e),
                        }
                        load_system(card, false);
                    });
                });
            }
            Err(e) => ui::fail(&e),
        }
    });
}

fn load_flatpak(card: Rc<gtk::Box>) {
    ui::clear(&card);
    if !flatpak::available() {
        card.append(&ui::row(Some("package_2"), "Flatpak is not installed", "").0);
        return;
    }
    card.append(&ui::row(Some("package_2"), "Checking Flatpak apps…", "").0);
    glib::spawn_future_local(async move {
        let result = flatpak::updates().await;
        let installed = flatpak::installed().await.unwrap_or_default();
        ui::clear(&card);
        match result {
            Ok(list) => {
                let title = if list.is_empty() { "All Flatpak apps are up to date".to_string() } else { format!("{} app updates", list.len()) };
                let (row, end) = ui::row(Some("package_2"), &title, if list.is_empty() { "" } else { "Updated from Flathub without touching the system" });
                if !list.is_empty() {
                    let b = ui::button("Update apps", "accent");
                    end.append(&b);
                    let c = card.clone();
                    b.connect_clicked(move |_| {
                        let c = c.clone();
                        glib::spawn_future_local(async move {
                            if !ui::begin() {
                                return;
                            }
                            ui::busy("Updating Flatpak apps", None);
                            let r = flatpak::update_all().await;
                            ui::end();
                            match r {
                                Ok(()) => ui::done("Flatpak apps updated"),
                                Err(e) => ui::fail(&e),
                            }
                            load_flatpak(c);
                        });
                    });
                }
                card.append(&row);
                for (id, version) in &list {
                    let app = installed.iter().find(|a| a.id == *id);
                    let desktop = gio_unix::DesktopAppInfo::new(&format!("{id}.desktop"));
                    let name = app.map(|a| a.name.clone()).filter(|n| !n.is_empty()).or_else(|| desktop.as_ref().map(|d| d.display_name().to_string())).unwrap_or_else(|| id.rsplit('.').next().unwrap_or(id).to_string());
                    let from = app.map(|a| a.version.clone()).unwrap_or_default();
                    let sub = match (from.is_empty(), version.is_empty()) {
                        (false, false) if from != *version => format!("{from} → {version}"),
                        (_, false) => version.clone(),
                        _ => "New build available".to_string(),
                    };
                    let (r, _) = ui::row(None, &name, &sub);
                    let image = match desktop.as_ref().and_then(|d| d.icon()) {
                        Some(i) => gtk::Image::from_gicon(&i),
                        None => gtk::Image::from_icon_name("application-x-executable"),
                    };
                    image.set_pixel_size(28);
                    r.prepend(&image);
                    card.append(&r);
                }
            }
            Err(e) => card.append(&ui::row(Some("error"), "Could not check Flatpak apps", &e).0),
        }
    });
}

fn load_firmware(card: Rc<gtk::Box>) {
    ui::clear(&card);
    card.append(&ui::row(Some("memory"), "Reading devices…", "").0);
    glib::spawn_future_local(async move {
        let result = fw::devices().await;
        ui::clear(&card);
        match result {
            Ok(devices) => {
                let updatable = devices.iter().filter(|d| d.updatable).count();
                let (row, end) = ui::row(Some("memory"), &format!("{} devices", devices.len()), &format!("{updatable} support firmware updates through fwupd"));
                let b = ui::button("Check firmware", "");
                end.append(&b);
                b.connect_clicked(|_| {
                    if let Err(e) = ui::terminal(&["fwupdmgr", "update"]) {
                        ui::fail(&e);
                    }
                });
                card.append(&row);
                for d in devices.iter().take(12) {
                    let (r, end) = ui::row(None, &d.name, &format!("{} · {}", d.vendor, d.version));
                    if d.updatable {
                        end.append(&ui::badge("Updatable", ""));
                    }
                    card.append(&r);
                }
            }
            Err(e) => card.append(&ui::row(Some("error"), "Firmware service unavailable", &e).0),
        }
    });
}
