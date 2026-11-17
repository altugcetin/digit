//! The install sheet: a file opened from Files (an AppImage, a Flatpak bundle or this system's
//! own package format) shows what it is over a dimmed screen and installs only after Install.
use crate::appimage;
use gtk::{gio, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kit::ui;
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    AppImage,
    Bundle,
    Arch,
    Rpm,
    Deb,
}

impl Kind {
    pub fn of(path: &Path) -> Option<Self> {
        let n = path.file_name()?.to_string_lossy().to_lowercase();
        if n.ends_with(".appimage") {
            Some(Self::AppImage)
        } else if n.ends_with(".flatpak") {
            Some(Self::Bundle)
        } else if n.ends_with(".pkg.tar.zst") || n.ends_with(".pkg.tar.xz") {
            Some(Self::Arch)
        } else if n.ends_with(".rpm") {
            Some(Self::Rpm)
        } else if n.ends_with(".deb") {
            Some(Self::Deb)
        } else {
            None
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::AppImage => "AppImage",
            Self::Bundle => "Flatpak bundle",
            Self::Arch => "Arch package",
            Self::Rpm => "RPM package",
            Self::Deb => "Debian package",
        }
    }

    fn system(self) -> bool {
        matches!(self, Self::Arch | Self::Rpm | Self::Deb)
    }

    fn glyph(self) -> &'static str {
        if self.system() { "inventory_2" } else if self == Self::Bundle { "package_2" } else { "deployed_code" }
    }
}

/// What the sheet shows about the file.
#[derive(Default)]
struct Info {
    name: String,
    version: String,
    summary: String,
    depends: Vec<String>,
    size: u64,
    /// Why it can't be installed here; no Install button then.
    problem: Option<String>,
    installed: Option<appimage::Installed>,
}

async fn output(argv: &[&std::ffi::OsStr]) -> Option<String> {
    let p = gio::Subprocess::newv(argv, gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_SILENCE).ok()?;
    let (out, _) = p.communicate_utf8_future(None).await.ok()?;
    p.is_successful().then(|| out.map(|s| s.to_string()).unwrap_or_default())
}

/// `key = value` lines of an Arch package's .PKGINFO.
fn pkginfo(text: &str, info: &mut Info) {
    for line in text.lines() {
        let Some((k, v)) = line.split_once(" = ") else { continue };
        let v = v.trim().to_string();
        match k.trim() {
            "pkgname" => info.name = v,
            "pkgver" => info.version = v,
            "pkgdesc" => info.summary = v,
            "depend" => info.depends.push(v),
            _ => {}
        }
    }
}

/// Reads the file's name, version and description without running anything from it.
async fn inspect(path: &Path, kind: Kind) -> Info {
    let mut info = Info { size: std::fs::metadata(path).map(|m| m.len()).unwrap_or(0), ..Default::default() };
    let distro = crate::sources::distro();
    let os = crate::sources::os_name();
    let file = path.as_os_str();
    match kind {
        Kind::AppImage => match appimage::peek(path) {
            Ok(p) => {
                info.name = p.name;
                info.installed = p.installed;
            }
            Err(e) => info.problem = Some(e),
        },
        Kind::Bundle => {
            info.name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            if glib::find_program_in_path("flatpak").is_none() {
                info.problem = Some("Flatpak isn't installed on this computer".into());
            }
        }
        Kind::Arch => {
            if distro != crate::sources::Distro::Arch {
                info.problem = Some(format!("This package is for Arch-based systems; this computer runs {os}."));
            } else if let Some(text) = output(&["bsdtar".as_ref(), "-xOf".as_ref(), file, ".PKGINFO".as_ref()]).await {
                pkginfo(&text, &mut info);
            } else {
                info.problem = Some("The package can't be read".into());
            }
        }
        Kind::Rpm => {
            if distro != crate::sources::Distro::PackageKit || glib::find_program_in_path("rpm").is_none() {
                info.problem = Some(format!("This package is for RPM-based systems such as Fedora; this computer runs {os}."));
            } else if let Some(text) = output(&["rpm".as_ref(), "-qp".as_ref(), "--qf".as_ref(), "%{NAME}\n%{VERSION}-%{RELEASE}\n%{SUMMARY}\n".as_ref(), file]).await {
                let mut l = text.lines();
                (info.name, info.version, info.summary) = (l.next().unwrap_or("").into(), l.next().unwrap_or("").into(), l.next().unwrap_or("").into());
            } else {
                info.problem = Some("The package can't be read".into());
            }
        }
        Kind::Deb => {
            if distro != crate::sources::Distro::PackageKit || glib::find_program_in_path("dpkg-deb").is_none() {
                info.problem = Some(format!("This package is for Debian-based systems; this computer runs {os}."));
            } else if let Some(text) = output(&["dpkg-deb".as_ref(), "-f".as_ref(), file, "Package".as_ref(), "Version".as_ref(), "Description".as_ref()]).await {
                for line in text.lines() {
                    if let Some((k, v)) = line.split_once(": ") {
                        match k {
                            "Package" => info.name = v.into(),
                            "Version" => info.version = v.into(),
                            "Description" => info.summary = v.into(),
                            _ => {}
                        }
                    }
                }
            } else {
                info.problem = Some("The package can't be read".into());
            }
        }
    }
    if info.name.is_empty() {
        info.name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    }
    info
}

fn warning(kind: Kind) -> &'static str {
    match kind {
        Kind::AppImage => "Copied to Applications in your home folder and added to your apps. It runs with your rights, like any program you start.",
        Kind::Bundle => "Installed for you only, with no password.",
        _ => "Installed for the whole system, with your password. It comes from outside your system's repositories, so install it only if you trust where it came from.",
    }
}

/// Installs `path`; the pulse keeps the bar moving while the tools give no percentage.
async fn install(path: &Path, kind: Kind) -> Result<Option<appimage::Installed>, String> {
    match kind {
        Kind::AppImage => appimage::install(path.to_path_buf()).await.map(Some),
        Kind::Bundle => crate::flatpak::install_bundle(path, |_| {}, &gio::Cancellable::new()).await.map(|_| None),
        Kind::Rpm | Kind::Deb => kit::pk::install_files(&[path.to_string_lossy().into_owned()], |_| {}).await.map(|_| None),
        Kind::Arch => {
            // As the Store installs Arch packages: pacman through pkexec, so polkit asks.
            let argv: [&std::ffi::OsStr; 6] = ["pkexec".as_ref(), "pacman".as_ref(), "-U".as_ref(), "--noconfirm".as_ref(), "--".as_ref(), path.as_os_str()];
            let p = gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_PIPE).map_err(|e| e.to_string())?;
            let (_, err) = p.communicate_utf8_future(None).await.map_err(|e| e.to_string())?;
            if p.is_successful() {
                Ok(None)
            } else if matches!(p.exit_status(), 126 | 127) {
                Err("Cancelled".into())
            } else {
                let err = err.map(|s| s.to_string()).unwrap_or_default();
                Err(err.lines().rev().map(str::trim).find(|l| l.starts_with("error:")).map(|l| l.trim_start_matches("error:").trim().to_string()).unwrap_or_else(|| "pacman failed".into()))
            }
        }
    }
}

/// Shows the sheet for `path` over the whole screen.
pub fn show(app: &gtk::Application, path: PathBuf) {
    let Some(kind) = Kind::of(&path) else { return };
    let window = gtk::Window::new();
    window.set_application(Some(app));
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace(Some("digit-install"));
    for e in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(e, true);
    }
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(KeyboardMode::Exclusive);
    window.add_css_class("digit");
    window.add_css_class("install-sheet");
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("sheet-backdrop");
    let card = gtk::Box::new(gtk::Orientation::Vertical, 12);
    card.add_css_class("dialog");
    card.set_size_request(460, -1);
    card.set_halign(gtk::Align::Center);
    card.set_valign(gtk::Align::Center);
    card.set_vexpand(true);
    root.append(&card);
    window.set_child(Some(&root));
    root.set_opacity(0.0);

    // Head: picture, name, and what kind of file it is.
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    let picture = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let tile = ui::tile(kind.glyph());
    tile.add_css_class("app");
    picture.append(&tile);
    head.append(&picture);
    let titles = gtk::Box::new(gtk::Orientation::Vertical, 2);
    titles.set_valign(gtk::Align::Center);
    let name = ui::text(&path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), "name");
    name.add_css_class("dialog-title");
    name.set_xalign(0.0);
    name.set_wrap(true);
    name.set_max_width_chars(36);
    let sub = ui::text(kind.label(), "dim");
    sub.set_xalign(0.0);
    titles.append(&name);
    titles.append(&sub);
    head.append(&titles);
    card.append(&head);
    let summary = ui::text("", "");
    summary.set_xalign(0.0);
    summary.set_wrap(true);
    summary.set_max_width_chars(52);
    summary.set_visible(false);
    card.append(&summary);
    let note = ui::text(warning(kind), "dim");
    note.set_xalign(0.0);
    note.set_wrap(true);
    note.set_max_width_chars(52);
    card.append(&note);
    let bar = gtk::ProgressBar::new();
    bar.set_visible(false);
    card.append(&bar);
    let status = ui::text("", "dim");
    status.set_xalign(0.0);
    status.set_wrap(true);
    status.set_max_width_chars(52);
    status.set_visible(false);
    card.append(&status);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    buttons.set_halign(gtk::Align::End);
    buttons.set_margin_top(6);
    let cancel = ui::button("Cancel", "");
    let go = ui::button("Install", "accent");
    let open = ui::button("Open", "accent");
    open.set_visible(false);
    go.set_sensitive(false);
    buttons.append(&cancel);
    buttons.append(&go);
    buttons.append(&open);
    card.append(&buttons);

    let busy = Rc::new(Cell::new(false));
    let closing = Rc::new(Cell::new(false));
    let close: Rc<dyn Fn()> = {
        let (window, root, card, busy) = (window.downgrade(), root.downgrade(), card.downgrade(), busy.clone());
        Rc::new(move || {
            if busy.get() || closing.replace(true) {
                return;
            }
            let (Some(root), Some(card)) = (root.upgrade(), card.upgrade()) else { return };
            let window = window.clone();
            ui::settle(&root, &card, 0, false, move || {
                if let Some(w) = window.upgrade() {
                    w.destroy();
                }
            });
        })
    };
    let c = close.clone();
    cancel.connect_clicked(move |_| c());
    // A click on the dimmed screen, outside the card, cancels too.
    let outside = gtk::GestureClick::new();
    let (c, card_w) = (close.clone(), card.downgrade());
    outside.connect_pressed(move |g, _, x, y| {
        let Some(card) = card_w.upgrade() else { return };
        let Some(root) = g.widget() else { return };
        let inside = root.compute_point(&card, &gtk::graphene::Point::new(x as f32, y as f32)).is_some_and(|p| card.contains(p.x() as f64, p.y() as f64));
        if !inside {
            c();
        }
    });
    root.add_controller(outside);
    let keys = gtk::EventControllerKey::new();
    let c = close.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            c();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(keys);

    let installed: Rc<std::cell::RefCell<Option<appimage::Installed>>> = Rc::default();
    let shown_name: Rc<std::cell::RefCell<String>> = Rc::default();
    {
        let (path, go, name, sub, summary, status, tile, picture, installed, shown_name, open) = (path.clone(), go.clone(), name.clone(), sub.clone(), summary.clone(), status.clone(), tile.clone(), picture.clone(), installed.clone(), shown_name.clone(), open.clone());
        glib::spawn_future_local(async move {
            let info = inspect(&path, kind).await;
            name.set_text(&info.name);
            *shown_name.borrow_mut() = info.name.clone();
            let mut facts = vec![kind.label().to_string()];
            if !info.version.is_empty() {
                facts.push(info.version.clone());
            }
            if info.size > 0 {
                facts.push(glib::format_size(info.size).to_string());
            }
            sub.set_text(&facts.join(" · "));
            let mut text = info.summary.clone();
            if !info.depends.is_empty() {
                let deps: Vec<&str> = info.depends.iter().map(|d| d.split(['<', '>', '=']).next().unwrap_or(d)).take(8).collect();
                let more = info.depends.len().saturating_sub(8);
                let list = if more > 0 { format!("{} and {more} more", deps.join(", ")) } else { deps.join(", ") };
                text = if text.is_empty() { format!("Needs {list}") } else { format!("{text}\nNeeds {list}") };
            }
            summary.set_text(&text);
            summary.set_visible(!text.is_empty());
            if let Some(icon) = info.installed.as_ref().and_then(|i| i.icon.clone()) {
                let image = gtk::Image::from_file(icon);
                image.set_pixel_size(52);
                picture.remove(&tile);
                picture.append(&image);
            }
            if let Some(problem) = info.problem {
                status.set_text(&problem);
                status.add_css_class("warn");
                status.set_visible(true);
                go.set_visible(false);
            } else if info.installed.is_some() {
                status.set_text("Already installed");
                status.set_visible(true);
                go.set_visible(false);
                open.set_visible(true);
                *installed.borrow_mut() = info.installed;
            } else {
                go.set_sensitive(true);
                go.grab_focus();
            }
        });
    }
    {
        let (path, bar, status, cancel, open, busy, installed, shown_name) = (path.clone(), bar.clone(), status.clone(), cancel.clone(), open.clone(), busy.clone(), installed.clone(), shown_name.clone());
        go.connect_clicked(move |go| {
            busy.set(true);
            go.set_sensitive(false);
            cancel.set_sensitive(false);
            go.set_label("Installing…");
            bar.set_visible(true);
            status.remove_css_class("warn");
            status.set_text(if kind.system() { "Waiting for your password…" } else { "Installing…" });
            status.set_visible(true);
            let pulse = glib::timeout_add_local(std::time::Duration::from_millis(120), {
                let bar = bar.clone();
                move || {
                    bar.pulse();
                    glib::ControlFlow::Continue
                }
            });
            let (path, bar, status, cancel, open, busy, installed, shown_name, go) = (path.clone(), bar.clone(), status.clone(), cancel.clone(), open.clone(), busy.clone(), installed.clone(), shown_name.clone(), go.clone());
            glib::spawn_future_local(async move {
                let result = install(&path, kind).await;
                pulse.remove();
                bar.set_visible(false);
                busy.set(false);
                cancel.set_sensitive(true);
                match result {
                    Ok(app) => {
                        status.set_text(&format!("{} is installed", shown_name.borrow()));
                        go.set_visible(false);
                        cancel.set_label("Done");
                        open.set_visible(app.is_some());
                        *installed.borrow_mut() = app;
                        if let Some(s) = crate::store() {
                            s.refresh();
                        }
                    }
                    Err(e) => {
                        status.set_text(&e);
                        status.add_css_class("warn");
                        go.set_label("Try Again");
                        go.set_sensitive(true);
                    }
                }
            });
        });
    }
    let c = close.clone();
    open.connect_clicked(move |_| {
        if let Some(app) = installed.borrow().as_ref() {
            appimage::launch(app);
        }
        c();
    });
    window.present();
    ui::settle(&root, &card, 0, true, || {});
}

/// The file `arg` names, when it is one the sheet installs.
pub fn file(arg: &str) -> Option<PathBuf> {
    let path = if arg.starts_with("file://") { gio::File::for_uri(arg).path()? } else if arg.starts_with('/') { PathBuf::from(arg) } else { return None };
    (Kind::of(&path).is_some() && path.is_file()).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_follow_the_file_name() {
        assert_eq!(Kind::of(Path::new("/a/Foo-1.AppImage")), Some(Kind::AppImage));
        assert_eq!(Kind::of(Path::new("/a/x.flatpak")), Some(Kind::Bundle));
        assert_eq!(Kind::of(Path::new("/a/yay-12-1-x86_64.pkg.tar.zst")), Some(Kind::Arch));
        assert_eq!(Kind::of(Path::new("/a/x.rpm")), Some(Kind::Rpm));
        assert_eq!(Kind::of(Path::new("/a/x.deb")), Some(Kind::Deb));
        assert_eq!(Kind::of(Path::new("/a/x.tar.zst")), None);
    }

    #[test]
    fn pkginfo_gives_name_version_and_needs() {
        let mut info = Info::default();
        pkginfo("# generated\npkgname = yay\npkgver = 12.4-1\npkgdesc = An AUR helper\ndepend = pacman>6\ndepend = git\nsize = 9\n", &mut info);
        assert_eq!((info.name.as_str(), info.version.as_str(), info.summary.as_str()), ("yay", "12.4-1", "An AUR helper"));
        assert_eq!(info.depends, ["pacman>6", "git"]);
    }
}
