use crate::{guard, pk, system, ui};
use gtk::{glib, prelude::*};
use std::rc::Rc;

fn base(v: &str) -> &str {
    v.split(['-', '_']).next().unwrap_or(v)
}

pub fn build() -> gtk::Widget {
    let (scroll, body) = ui::page("Drivers", "Drivers and their packages. Changes are previewed first, use signed packages only, and take effect after a reboot.");
    let gpus = system::gpus();
    if gpus.is_empty() {
        let card = ui::card();
        card.append(&ui::row(Some("memory"), "No graphics device found", "").0);
        body.append(&card);
    }
    for g in gpus {
        body.append(&ui::heading(&format!("{} {}", g.vendor, g.name)));
        let card = Rc::new(ui::card());
        body.append(&*card);
        let (row, end) = ui::row(Some("memory"), &g.name, &format!("{} · {} · kernel driver {}", g.vendor, g.card, g.driver));
        end.append(&ui::badge(&g.driver, if g.driver == "none" { "warn" } else { "" }));
        card.append(&row);
        match g.kind() {
            "nvidia" => nvidia(card.clone()),
            kind => mesa(card.clone(), kind),
        }
    }
    if system::sound_without_firmware() {
        sound(&body);
    }
    scroll.upcast()
}

const SOUND_FIRMWARE: [&str; 2] = ["sof-firmware", "firmware-sof-signed"];

fn sound(body: &gtk::Box) {
    let heading = ui::heading("Sound");
    let card = ui::card();
    let (row, end) = ui::row(Some("volume_off"), "Sound firmware is missing", "Speakers and microphones stay silent until it's installed");
    let install = ui::button("Install", "accent");
    install.set_valign(gtk::Align::Center);
    install.set_sensitive(false);
    end.append(&install);
    card.append(&row);
    body.append(&heading);
    body.append(&card);
    glib::spawn_future_local(async move {
        if pk::resolve(&SOUND_FIRMWARE, pk::FILTER_INSTALLED).await.is_ok_and(|found| !found.is_empty()) {
            heading.set_visible(false);
            card.set_visible(false);
            return;
        }
        let Some(package) = pk::resolve(&SOUND_FIRMWARE, pk::FILTER_NONE).await.ok().and_then(|found| found.into_iter().find(|p| !p.installed())) else { return };
        install.set_sensitive(true);
        install.connect_clicked(move |b| firmware(ui::root(b), package.clone()));
    });
}

fn firmware(parent: Option<gtk::Window>, package: pk::Pkg) {
    if !guard::package_id(&package.id) {
        ui::fail("Refused: invalid package id");
        return;
    }
    glib::spawn_future_local(async move {
        if !ui::begin() {
            return;
        }
        let ids = vec![package.id.clone()];
        let sim = pk::install(&ids, pk::ONLY_TRUSTED | pk::SIMULATE, ui::progress("Checking the change")).await;
        ui::end();
        let changes = match sim {
            Ok(c) => c,
            Err(e) => {
                ui::fail(&e);
                return;
            }
        };
        let items: Vec<String> = changes.iter().map(|p| format!("{} {}", p.name(), p.version())).collect();
        ui::confirm(parent, &format!("Install {}?", package.name()), "Restart the computer afterwards to hear sound.", &items, "Install", "accent", move || {
            glib::spawn_future_local(async move {
                if !ui::begin() {
                    return;
                }
                let r = pk::install(&ids, pk::ONLY_TRUSTED, ui::progress("Installing the sound firmware")).await;
                ui::end();
                match r {
                    Ok(_) => ui::done("Installed. Restart the computer to hear sound"),
                    Err(e) => ui::fail(&e),
                }
            });
        });
    });
}

fn mesa(card: Rc<gtk::Box>, kind: &str) {
    let names: Vec<&'static str> = match kind {
        "amd" => vec!["Mesa", "Mesa-dri", "libvulkan_radeon", "kernel-firmware-amdgpu"],
        "intel" => vec!["Mesa", "Mesa-dri", "libvulkan_intel", "kernel-firmware-intel", "kernel-firmware-i915"],
        _ => vec!["Mesa", "Mesa-dri"],
    };
    glib::spawn_future_local(async move {
        match pk::resolve(&names, pk::FILTER_INSTALLED).await {
            Ok(found) => {
                for name in names {
                    if let Some(p) = found.iter().find(|p| p.name() == name) {
                        card.append(&ui::row(Some("deployed_code"), p.name(), &format!("{} · {}", p.version(), p.repo())).0);
                    }
                }
                card.append(&ui::row(Some("info"), "Open source driver", "Mesa, Vulkan drivers and firmware are updated together with system updates.").0);
            }
            Err(e) => card.append(&ui::row(Some("error"), "Could not read driver packages", &e).0),
        }
    });
}

fn nvidia(card: Rc<gtk::Box>) {
    let loaded = system::nvidia_loaded();
    let running = loaded.as_ref().map(|(v, _)| v.clone()).unwrap_or_default();
    let (row, end) = match &loaded {
        Some((v, open)) => ui::row(Some("bolt"), "Running driver", &format!("{} kernel module {v}", if *open { "Open" } else { "Proprietary" })),
        None => ui::row(Some("bolt"), "Running driver", "The NVIDIA kernel module is not loaded"),
    };
    let status = ui::badge("Checking…", "");
    end.append(&status);
    card.append(&row);
    glib::spawn_future_local(async move {
        let installed = match pk::search_names(&["nvidia"], pk::FILTER_INSTALLED).await {
            Ok(i) => i,
            Err(e) => {
                card.append(&ui::row(Some("error"), "Could not read driver packages", &e).0);
                status.set_text("Unknown");
                return;
            }
        };
        let find = |f: &dyn Fn(&str) -> bool| installed.iter().find(|p| f(p.name())).cloned();
        let kmp_meta = find(&|n| n.contains("kmp-meta"));
        let user_meta = find(&|n| n.contains("userspace-meta"));
        let kmp = find(&|n| n.contains("-kmp-") && !n.contains("meta"));
        let gl = find(&|n| n.starts_with("nvidia-gl-") && !n.contains("32bit"));
        for (label, p) in [("Kernel module package", &kmp), ("User space libraries", &gl), ("Driver series", &kmp_meta)] {
            if let Some(p) = p {
                card.append(&ui::row(Some("deployed_code"), label, &format!("{} {} · {}", p.name(), p.version(), p.repo())).0);
            }
        }
        let run = base(&running).to_string();
        let (text, kind, note) = match (&kmp, &gl, &kmp_meta) {
            _ if running.is_empty() => ("Not loaded", "warn", "Reboot, or check Secure Boot enrolment, so the kernel module can load.".to_string()),
            (Some(k), _, _) if base(k.version()) != run => ("Reboot required", "warn", format!("Driver {} is installed but {run} is still running. Reboot to load it.", base(k.version()))),
            (_, Some(g), _) if base(g.version()) != run => ("Mismatch", "warn", format!("User space {} does not match the running module {run}. Run a system update.", base(g.version()))),
            (_, _, Some(m)) if pk::compare(base(m.version()), &run).is_gt() => ("Update incomplete", "warn", format!("Series {} is selected but driver {run} is installed. Run a system update, then reboot.", base(m.version()))),
            _ => ("Consistent", "good", String::new()),
        };
        status.set_text(text);
        status.add_css_class(kind);
        if !note.is_empty() {
            card.append(&ui::row(Some("info"), text, &note).0);
        }
        let (Some(kmeta), Some(umeta)) = (kmp_meta, user_meta) else { return };
        let names = [kmeta.name().to_string(), umeta.name().to_string()];
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let all = match pk::resolve(&refs, pk::FILTER_NONE).await {
            Ok(a) => a,
            Err(e) => {
                card.append(&ui::row(Some("error"), "Could not list driver versions", &e).0);
                return;
            }
        };
        let arch = kmeta.id.split(';').nth(2).unwrap_or("x86_64").to_string();
        let native = |p: &&pk::Pkg| p.id.split(';').nth(2) == Some(arch.as_str()) && guard::package_id(&p.id);
        let mut kv: Vec<pk::Pkg> = all.iter().filter(|p| p.name() == kmeta.name()).filter(native).cloned().collect();
        kv.sort_by(|a, b| pk::compare(b.version(), a.version()));
        kv.dedup_by(|a, b| a.version() == b.version());
        let uv: Vec<pk::Pkg> = all.iter().filter(|p| p.name() == umeta.name()).filter(native).cloned().collect();
        if kv.len() < 2 {
            return;
        }
        let labels: Vec<String> = kv.iter().map(|p| if p.version() == kmeta.version() { format!("{} (selected)", p.version()) } else { p.version().to_string() }).collect();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let pick = kit::ui::dropdown(&refs, 0);
        pick.set_selected(kv.iter().position(|p| p.version() == kmeta.version()).unwrap_or(0) as u32);
        let (row, end) = ui::row(Some("swap_vert"), "Driver version", &format!("{} versions available from trusted repositories", kv.len()));
        let go = ui::button("Switch", "");
        end.append(&pick);
        end.append(&go);
        card.append(&row);
        let current = kmeta.version().to_string();
        go.connect_clicked(move |b| {
            let Some(target) = kv.get(pick.selected() as usize).cloned() else { return };
            if target.version() == current {
                ui::done("That version is already selected");
                return;
            }
            let mut matching: Vec<&pk::Pkg> = uv.iter().filter(|u| base(u.version()) == base(target.version())).collect();
            matching.sort_by(|a, b| pk::compare(b.version(), a.version()));
            let Some(user) = matching.first().map(|p| (*p).clone()) else {
                ui::fail(&format!("No matching user space package for {}", base(target.version())));
                return;
            };
            switch(ui::root(b), vec![target.id.clone(), user.id.clone()], base(target.version()).to_string());
        });
    });
}

fn switch(parent: Option<gtk::Window>, ids: Vec<String>, version: String) {
    if !ids.iter().all(|i| guard::package_id(i)) {
        ui::fail("Refused: invalid package id");
        return;
    }
    glib::spawn_future_local(async move {
        if !ui::begin() {
            return;
        }
        let flags = pk::ONLY_TRUSTED | pk::ALLOW_DOWNGRADE;
        let sim = pk::install(&ids, flags | pk::SIMULATE, ui::progress("Checking the change")).await;
        ui::end();
        let changes = match sim {
            Ok(c) => c,
            Err(e) => {
                ui::fail(&e);
                return;
            }
        };
        ui::done("Check complete, nothing was changed");
        let items: Vec<String> = changes.iter().map(|p| format!("{} {}", p.name(), p.version())).collect();
        ui::confirm(
            parent,
            &format!("Switch to driver {version}?"),
            &format!("{} packages will change. Keep this window open until it finishes, then reboot. If anything goes wrong, switch back to the previous version here.", changes.len()),
            &items,
            "Switch driver",
            "accent",
            move || {
                glib::spawn_future_local(async move {
                    if !ui::begin() {
                        return;
                    }
                    let r = pk::install(&ids, flags, ui::progress("Switching driver")).await;
                    ui::end();
                    match r {
                        Ok(_) => ui::done(&format!("Driver {version} installed. Reboot to start using it")),
                        Err(e) => ui::fail(&e),
                    }
                });
            },
        );
    });
}
