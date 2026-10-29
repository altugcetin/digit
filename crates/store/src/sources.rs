use crate::{Store, flatpak, store};
use gtk::{gio, glib, pango, prelude::*};
use kit::ui;
use std::rc::Rc;

#[derive(Clone, PartialEq)]
pub enum Distro {
    PackageKit,
    Arch { shelly: bool },
    None,
}

pub fn os_name() -> String {
    let text = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let field = |k: &str| text.lines().find_map(|l| l.strip_prefix(&format!("{k}="))).map(|v| v.trim_matches('"').to_string());
    field("NAME").or_else(|| field("PRETTY_NAME")).unwrap_or_else(|| "Linux".into())
}

pub fn distro() -> Distro {
    let text = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let field = |k: &str| text.lines().find_map(|l| l.strip_prefix(&format!("{k}="))).map(|v| v.trim_matches('"').to_lowercase()).unwrap_or_default();
    let (id, like) = (field("ID"), field("ID_LIKE"));
    if id == "arch" || like.split_whitespace().any(|l| l == "arch") || glib::find_program_in_path("pacman").is_some() && glib::find_program_in_path("zypper").is_none() {
        return Distro::Arch { shelly: glib::find_program_in_path("shelly").is_some() };
    }
    if std::path::Path::new("/usr/share/dbus-1/system-services/org.freedesktop.PackageKit.service").exists() || glib::find_program_in_path("pkcon").is_some() {
        return Distro::PackageKit;
    }
    Distro::None
}

fn valid_remote(name: &str) -> bool {
    !name.is_empty() && name.len() < 64 && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

pub fn scope_system() -> bool {
    kit::prefs::text(&kit::prefs::read(), "store_scope", "user") == "system"
}

fn terminal(argv: &[&str]) -> Result<(), String> {
    let chosen = kit::prefs::text(&kit::prefs::read(), "files_terminal", "");
    let candidates = ["ghostty", "kitty", "foot", "alacritty", "wezterm", "konsole", "gnome-terminal", "ptyxis", "xterm"];
    let term = if !chosen.is_empty() && glib::find_program_in_path(&chosen).is_some() { Some(chosen) } else { candidates.iter().find(|c| glib::find_program_in_path(c).is_some()).map(|c| c.to_string()) };
    let term = term.ok_or("No terminal found")?;
    let mut full: Vec<String> = vec![term.clone()];
    match term.as_str() {
        "kitty" | "foot" => full.push("--hold".into()),
        "gnome-terminal" | "ptyxis" => full.push("--".into()),
        _ => full.push("-e".into()),
    }
    full.extend(argv.iter().map(|s| s.to_string()));
    let os: Vec<&std::ffi::OsStr> = full.iter().map(std::ffi::OsStr::new).collect();
    gio::Subprocess::newv(&os, gio::SubprocessFlags::NONE).map(|_| ()).map_err(|e| e.to_string())
}

fn section(title: &str) -> (gtk::Box, gtk::Box) {
    let wrap = gtk::Box::new(gtk::Orientation::Vertical, 4);
    wrap.append(&ui::heading(title));
    let card = ui::card();
    wrap.append(&card);
    (wrap, card)
}

pub fn page(s: &Rc<Store>) -> gtk::Widget {
    let (scroll, body) = ui::page("Sources", "Where apps come from. Flatpak remotes need a signed .flatpakrepo; system changes ask for your password.");
    let (fwrap, fcard) = section("Flatpak");
    fcard.append(&loading());
    body.append(&fwrap);
    let add = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    add.set_margin_top(8);
    let addb = ui::button("Add a Source…", "");
    addb.connect_clicked(|_| add_remote());
    add.append(&addb);
    body.append(&add);
    let me = s.clone();
    let fc = fcard.clone();
    glib::spawn_future_local(async move {
        let list = flatpak::remotes().await;
        fill_remotes(&me, &fc, list);
    });
    let (dwrap, dcard) = section(&format!("{} repositories", os_name()));
    if let Some(h) = dwrap.first_child() {
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        head.append(&kit::ui::distro_logo(18, false));
        dwrap.remove(&h);
        head.append(&h);
        dwrap.prepend(&head);
    }
    body.append(&dwrap);
    match distro() {
        Distro::PackageKit => {
            dcard.append(&loading());
            let dc = dcard.clone();
            glib::spawn_future_local(async move {
                match kit::pk::repos().await {
                    Ok(repos) => fill_repos(&dc, repos),
                    Err(e) => {
                        ui::clear(&dc);
                        dc.append(&ui::row(Some("error"), "Couldn't read the repositories", &e).0);
                    }
                }
            });
        }
        Distro::Arch { shelly } => {
            for r in pacman_repos() {
                dcard.append(&ui::row(Some("deployed_code"), &r, "Enabled in /etc/pacman.conf").0);
            }
            let (row, end) = ui::row(Some("terminal"), "Shelly", if shelly { "Installs, AUR builds and upgrades run in a terminal so you can review them" } else { "Install Shelly (Seafoam-Labs/Shelly-ALPM) for one-click Arch packages and the AUR" });
            if shelly {
                let up = ui::button("Upgrade Everything", "");
                up.connect_clicked(|_| {
                    if let Err(e) = terminal(&["shelly", "upgrade", "all"]) {
                        if let Some(s) = store() {
                            s.note(&e, true);
                        }
                    }
                });
                end.append(&up);
            }
            dcard.append(&row);
            dcard.append(&ui::row(Some("info"), "Changing repositories", "Shelly can't edit repositories yet; edit /etc/pacman.conf as root").0);
        }
        Distro::None => {
            dcard.append(&ui::row(Some("info"), "No supported package manager", "PackageKit isn't available, so only Flatpak apps can be managed here").0);
        }
    }
    let (swrap, scard) = section("Installing apps");
    let (row, end) = ui::row(Some("person"), "Install apps for", "Just you needs no password; everyone shares one copy and asks for your password");
    let pick = kit::ui::dropdown(&["Just me", "Everyone on this computer"], 0);
    pick.set_selected(if scope_system() { 1 } else { 0 });
    pick.connect_selected_notify(|p| {
        let _ = kit::prefs::set("store_scope", toml::Value::String(if p.selected() == 1 { "system".into() } else { "user".into() }));
    });
    end.append(&pick);
    scard.append(&row);
    body.append(&swrap);
    scroll.upcast()
}

fn loading() -> gtk::Widget {
    let r = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    r.add_css_class("row");
    let sp = gtk::Spinner::new();
    sp.start();
    r.append(&sp);
    r.append(&ui::text("Reading…", "sub"));
    r.upcast()
}

fn pacman_repos() -> Vec<String> {
    std::fs::read_to_string("/etc/pacman.conf")
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.trim().strip_prefix('[').and_then(|r| r.strip_suffix(']')).map(str::to_string))
        .filter(|r| r != "options")
        .collect()
}

fn fill_remotes(s: &Rc<Store>, card: &gtk::Box, list: Vec<flatpak::Remote>) {
    ui::clear(card);
    if list.is_empty() {
        card.append(&ui::row(Some("info"), "No Flatpak sources", "Add Flathub to get apps").0);
        return;
    }
    for r in list {
        let sub = format!("{} · {}", if r.system { "Everyone" } else { "Just you" }, r.url);
        let (row, end) = ui::row(Some(if r.disabled { "cloud_off" } else { "cloud_done" }), &format!("{}{}", if r.title.is_empty() { &r.name } else { &r.title }, if r.disabled { " (off)" } else { "" }), &sub);
        let sw = gtk::Switch::new();
        sw.set_active(!r.disabled);
        sw.set_valign(gtk::Align::Center);
        let (name, system) = (r.name.clone(), r.system);
        let me = s.clone();
        sw.connect_state_set(move |_, on| {
            let (name, me) = (name.clone(), me.clone());
            glib::spawn_future_local(async move {
                let args = vec!["remote-modify".to_string(), if on { "--enable".into() } else { "--disable".into() }, if system { "--system".into() } else { "--user".into() }, name.clone()];
                match flatpak::command(&args).await {
                    Ok(()) => me.note(&format!("{name} {}", if on { "turned on" } else { "turned off" }), false),
                    Err(e) => me.note(&e, true),
                }
                me.refresh();
            });
            glib::Propagation::Proceed
        });
        end.append(&sw);
        let del = kit::app::control("delete", "close");
        del.set_tooltip_text(Some("Remove this source"));
        let (name, system) = (r.name.clone(), r.system);
        let me = s.clone();
        del.connect_clicked(move |b| {
            let (name, me) = (name.clone(), me.clone());
            let detail = "Apps installed from it must be removed first. You can add it again later.";
            kit::ui::ask(ui::root(b).as_ref(), &format!("Remove “{name}”?"), detail, "Cancel", "Remove", "danger", move || {
                glib::spawn_future_local(async move {
                    let args = vec!["remote-delete".to_string(), if system { "--system".into() } else { "--user".into() }, name.clone()];
                    match flatpak::command(&args).await {
                        Ok(()) => me.note(&format!("Removed {name}"), false),
                        Err(e) => me.note(&e, true),
                    }
                    me.refresh();
                });
            });
        });
        end.append(&del);
        card.append(&row);
    }
}

fn fill_repos(card: &gtk::Box, repos: Vec<kit::pk::Repo>) {
    ui::clear(card);
    if repos.is_empty() {
        card.append(&ui::row(Some("info"), "No repositories reported", "").0);
        return;
    }
    let mut repos = repos;
    repos.sort_by(|a, b| b.enabled.cmp(&a.enabled).then(a.id.cmp(&b.id)));
    for r in repos {
        let (row, end) = ui::row(Some(if r.enabled { "inventory_2" } else { "block" }), if r.description.is_empty() { &r.id } else { &r.description }, &r.id);
        let sw = gtk::Switch::new();
        sw.set_active(r.enabled);
        sw.set_valign(gtk::Align::Center);
        let id = r.id.clone();
        sw.connect_state_set(move |sw, on| {
            let (id, sw) = (id.clone(), sw.clone());
            glib::spawn_future_local(async move {
                if let Err(e) = kit::pk::repo_enable(&id, on).await {
                    sw.set_active(!on);
                    if let Some(s) = store() {
                        s.note(&e, true);
                    }
                } else if let Some(s) = store() {
                    s.note(&format!("{id} {}", if on { "enabled" } else { "disabled" }), false);
                }
            });
            glib::Propagation::Proceed
        });
        end.append(&sw);
        card.append(&row);
    }
}

const PRESETS: [(&str, &str, &str); 3] = [
    ("flathub", "Flathub", "https://dl.flathub.org/repo/flathub.flatpakrepo"),
    ("flathub-beta", "Flathub Beta", "https://flathub.org/beta-repo/flathub-beta.flatpakrepo"),
    ("gnome-nightly", "GNOME Nightly", "https://nightly.gnome.org/gnome-nightly.flatpakrepo"),
];

fn add_remote() {
    let Some(s) = store() else { return };
    let w = gtk::Window::builder().modal(true).decorated(false).resizable(false).build();
    w.set_transient_for(Some(&s.window));
    w.add_css_class("digit-dialog");
    let card = gtk::Box::new(gtk::Orientation::Vertical, 10);
    card.add_css_class("dialog");
    card.set_size_request(460, -1);
    let t = ui::text("Add a Flatpak Source", "name dialog-title");
    card.append(&t);
    let d = ui::text("Paste a .flatpakrepo link from a project you trust. Its signing key is checked before anything installs.", "dim");
    d.set_wrap(true);
    d.set_max_width_chars(52);
    card.append(&d);
    let presets = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let entry = gtk::Entry::new();
    entry.add_css_class("search");
    entry.set_placeholder_text(Some("https://example.org/repo.flatpakrepo"));
    for (_, label, url) in PRESETS {
        let b = ui::button(label, "");
        let e = entry.clone();
        b.connect_clicked(move |_| e.set_text(url));
        presets.append(&b);
    }
    card.append(&presets);
    card.append(&entry);
    let everyone = gtk::CheckButton::with_label("For everyone on this computer (asks for your password)");
    card.append(&everyone);
    let error = ui::text("", "warn");
    error.set_visible(false);
    error.set_wrap(true);
    card.append(&error);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    buttons.set_halign(gtk::Align::End);
    let cancel = ui::button("Cancel", "");
    let ok = ui::button("Add", "accent");
    buttons.append(&cancel);
    buttons.append(&ok);
    card.append(&buttons);
    w.set_child(Some(&card));
    let ww = w.clone();
    cancel.connect_clicked(move |_| ww.close());
    let ww = w.clone();
    ok.connect_clicked(move |_| {
        let url = entry.text().trim().to_string();
        if !url.starts_with("https://") || !url.ends_with(".flatpakrepo") || url.contains(char::is_whitespace) {
            error.set_text("Use an https link that ends in .flatpakrepo");
            error.set_visible(true);
            return;
        }
        let name = PRESETS.iter().find(|p| p.2 == url).map(|p| p.0.to_string()).unwrap_or_else(|| url.rsplit('/').next().unwrap_or("remote").trim_end_matches(".flatpakrepo").to_lowercase());
        if !valid_remote(&name) {
            error.set_text("That link doesn't give a usable name");
            error.set_visible(true);
            return;
        }
        let system = everyone.is_active();
        ww.close();
        glib::spawn_future_local(async move {
            let Some(s) = store() else { return };
            s.note(&format!("Adding {name}…"), false);
            let args = vec!["remote-add".to_string(), "--if-not-exists".into(), if system { "--system".into() } else { "--user".into() }, name.clone(), url];
            match flatpak::command(&args).await {
                Ok(()) => s.note(&format!("Added {name}"), false),
                Err(e) => s.note(&e, true),
            }
            s.refresh();
        });
    });
    w.present();
}

pub fn app_sources(s: &Rc<Store>, id: &str, name: &str, card: &gtk::Box) {
    let installed = s.installed.borrow().get(id).cloned();
    if installed.is_none() {
        let (row, end) = ui::row(Some("person"), "Install for", "Just you needs no password");
        let pick = kit::ui::dropdown(&["Just me", "Everyone"], 0);
        pick.set_selected(if scope_system() { 1 } else { 0 });
        end.append(&pick);
        card.append(&row);
        let (row, end) = ui::row(Some("cloud_done"), "From", "Flatpak sources that have this app");
        let remotes = kit::ui::dropdown(&["flathub"], 0);
        remotes.set_sensitive(false);
        end.append(&remotes);
        card.append(&row);
        let (id2, me) = (id.to_string(), s.clone());
        me.chosen.borrow_mut().insert(id2.clone(), ("flathub".into(), scope_system()));
        let p2 = pick.clone();
        glib::spawn_future_local(async move {
            let found = Rc::new(flatpak::remotes_with(&id2).await);
            let fill = {
                let (found, remotes, me, id2) = (found.clone(), remotes.clone(), me.clone(), id2.clone());
                move |system: bool| {
                    let fit: Vec<String> = found.iter().filter(|(_, sys)| *sys == system).map(|(n, _)| n.clone()).collect();
                    let refs: Vec<&str> = fit.iter().map(String::as_str).collect();
                    remotes.set_model(Some(&gtk::StringList::new(if refs.is_empty() { &["not available"][..] } else { &refs })));
                    remotes.set_sensitive(fit.len() > 1);
                    let pick = fit.iter().position(|n| n == "flathub").unwrap_or(0);
                    remotes.set_selected(pick as u32);
                    if let Some(n) = fit.get(pick) {
                        me.chosen.borrow_mut().insert(id2.clone(), (n.clone(), system));
                    }
                }
            };
            fill(scope_system());
            let f2 = fill.clone();
            p2.connect_selected_notify(move |p| {
                let system = p.selected() == 1;
                let _ = kit::prefs::set("store_scope", toml::Value::String(if system { "system".into() } else { "user".into() }));
                f2(system);
            });
            let (me2, found2, p3) = (me.clone(), found.clone(), p2.clone());
            remotes.connect_selected_notify(move |r| {
                let system = p3.selected() == 1;
                let fit: Vec<&(String, bool)> = found2.iter().filter(|(_, sys)| *sys == system).collect();
                if let Some((n, _)) = fit.get(r.selected() as usize) {
                    me2.chosen.borrow_mut().insert(id2.clone(), (n.clone(), system));
                }
            });
        });
    }
    match distro() {
        Distro::PackageKit => {
            let candidates = candidates(id, name);
            let (row, end) = ui::row(None, &format!("{} package", os_name()), "Looking…");
            row.prepend(&kit::ui::distro_logo(28, false));
            card.append(&row);
            let me = s.clone();
            glib::spawn_future_local(async move {
                let refs: Vec<&str> = candidates.iter().map(String::as_str).collect();
                let found = kit::pk::resolve(&refs, kit::pk::FILTER_NONE).await.unwrap_or_default();
                let sub = row.first_child().and_then(|t| t.next_sibling()).and_then(|c| c.last_child()).and_downcast::<gtk::Label>();
                let Some(pkg) = found.iter().find(|p| p.repo() != "installed").or(found.first()).cloned() else {
                    if let Some(l) = sub {
                        l.set_text("Not packaged by your distribution");
                    }
                    return;
                };
                let installed = found.iter().any(|p| p.name() == pkg.name() && p.id.contains(";installed"));
                if let Some(l) = sub {
                    l.set_text(&format!("{} {} · {}", pkg.name(), pkg.version(), if installed { "installed".to_string() } else { pkg.repo().to_string() }));
                }
                if !installed {
                    let b = ui::button("Install", "");
                    let me2 = me.clone();
                    b.connect_clicked(move |b| distro_install(&me2, b, pkg.clone()));
                    end.append(&b);
                }
            });
        }
        Distro::Arch { shelly } => {
            let pkg = candidates(id, name).into_iter().next().unwrap_or_default();
            let (row, end) = ui::row(Some("inventory_2"), "Arch package", if shelly { "Install with Shelly in a terminal" } else { "Install Shelly to use Arch repositories and the AUR" });
            if shelly && !pkg.is_empty() {
                let repo = ui::button("Repositories", "");
                let p1 = pkg.clone();
                repo.connect_clicked(move |_| {
                    let _ = terminal(&["shelly", "-Is", &p1]);
                });
                let aur = ui::button("AUR", "");
                aur.set_tooltip_text(Some("AUR packages are built from user-submitted scripts; review them before building"));
                aur.connect_clicked(move |_| {
                    let _ = terminal(&["shelly", "install", "aur", &pkg]);
                });
                end.append(&repo);
                end.append(&aur);
            }
            card.append(&row);
        }
        Distro::None => {}
    }
}

fn candidates(id: &str, name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let last = id.rsplit('.').next().unwrap_or(id).to_lowercase();
    let simple = name.to_lowercase().replace(' ', "-");
    for c in [last, simple] {
        if !c.is_empty() && c.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '+' | '.')) && !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

fn distro_install(s: &Rc<Store>, button: &gtk::Button, pkg: kit::pk::Pkg) {
    button.set_sensitive(false);
    let me = s.clone();
    let b = button.clone();
    glib::spawn_future_local(async move {
        me.note(&format!("Checking {}…", pkg.name()), false);
        let ids = vec![pkg.id.clone()];
        let plan = match kit::pk::install(&ids, kit::pk::ONLY_TRUSTED | kit::pk::SIMULATE, |_| {}).await {
            Ok(p) => p,
            Err(e) => {
                me.note(&e, true);
                b.set_sensitive(true);
                return;
            }
        };
        let lines: Vec<String> = plan.iter().map(|p| format!("{} {}", p.name(), p.version())).collect();
        let detail = format!("{} package{} will be installed from {}:\n{}", lines.len().max(1), if lines.len() == 1 { "" } else { "s" }, os_name(), lines.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
        let me2 = me.clone();
        let b2 = b.clone();
        let w = me.window.clone();
        kit::ui::ask(Some(w.upcast_ref()), &format!("Install {}?", pkg.name()), &detail, "Cancel", "Install", "accent", move || {
            glib::spawn_future_local(async move {
                me2.note(&format!("Installing {}…", pkg.name()), false);
                match kit::pk::install(&[pkg.id.clone()], kit::pk::ONLY_TRUSTED, |_| {}).await {
                    Ok(_) => me2.note(&format!("{} installed", pkg.name()), false),
                    Err(e) => {
                        me2.note(&e, true);
                        b2.set_sensitive(true);
                    }
                }
            });
        });
        b.set_sensitive(true);
    });
    let _ = pango::EllipsizeMode::End;
}
