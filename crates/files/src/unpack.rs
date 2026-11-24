//! Extracting archives, and asking for the password of one that has it.

use crate::ops::{self, Plan, Secret};
use gtk::{gio, glib, prelude::*};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

/// The command that extracts the archive `name` at `path` into `out`, and whether `out` has to
/// be there already; or why nothing can. Only 7-Zip is given passwords: unzip and bsdtar take
/// them from a terminal, or in their arguments for everyone to see.
fn command(name: &str, path: &str, out: &str, locked: bool, has: impl Fn(&str) -> bool) -> Result<(Vec<String>, bool), &'static str> {
    let argv = |words: &[&str]| words.iter().map(|w| w.to_string()).collect::<Vec<_>>();
    let into = format!("-o{out}");
    let seven = argv(&["7z", "x", "-bd", "-y", &into, "--", path]);
    if tarball(name) {
        if has("tar") { Ok((argv(&["tar", "-x", "-f", path, "-C", out]), true)) } else { Err("tar isn't installed") }
    } else if locked {
        Ok((seven, false))
    } else if name.to_lowercase().ends_with(".zip") && has("unzip") {
        Ok((argv(&["unzip", "-q", "-n", path, "-d", out]), false))
    } else if has("7z") {
        Ok((seven, false))
    } else {
        Err("No tool can open this archive (install 7-Zip)")
    }
}

fn tarball(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains(".tar") || [".tgz", ".txz", ".tbz2"].iter().any(|e| lower.ends_with(e))
}

/// Whether 7-Zip says the archive, or something in it, needs a password.
fn locked(path: &Path) -> bool {
    let mut cmd = std::process::Command::new("7z");
    cmd.args(["l", "-slt", "--"]).arg(path).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
    ops::detach(&mut cmd);
    let Ok(mut child) = cmd.spawn() else { return false };
    let found = child.stdout.take().is_some_and(says_locked);
    // A long list is cut short once it has said so.
    let _ = child.kill();
    let _ = child.wait();
    found
}

/// Reads 7-Zip's list. A list that itself needs the password is only a question for it.
fn says_locked(list: impl std::io::Read) -> bool {
    use std::io::BufRead;
    std::io::BufReader::new(list).split(b'\n').map_while(Result::ok).any(|l| l.starts_with(b"Encrypted = +") || l.starts_with(b"Enter password"))
}

struct Archive {
    path: PathBuf,
    dir: PathBuf,
    name: String,
}

pub fn extract(files: Vec<gio::File>) {
    let archives = files.into_iter().filter_map(|f| {
        let (path, dir) = (f.path()?, f.parent()?.path()?);
        Some(Archive { name: path.file_name()?.to_string_lossy().into_owned(), path, dir })
    });
    glib::spawn_future_local(next(archives.collect()));
}

/// Starts each archive's job. One that needs a password asks for it, and the rest go on once
/// that is answered.
async fn next(mut queue: VecDeque<Archive>) {
    while let Some(a) = queue.pop_front() {
        let path = a.path.clone();
        let check = !tarball(&a.name) && glib::find_program_in_path("7z").is_some();
        if check && gio::spawn_blocking(move || locked(&path)).await.unwrap_or(false) {
            ask(a, false, move || {
                glib::spawn_future_local(next(queue));
            });
            return;
        }
        start(a, None);
    }
}

fn start(a: Archive, password: Option<Secret>) {
    let target = ops::unique(&gio::File::for_path(&a.dir), ops::split_ext(&a.name).0);
    let Some(out) = target.path() else { return };
    let (path, into) = (a.path.to_string_lossy(), out.to_string_lossy());
    let (argv, make) = match command(&a.name, &path, &into, password.is_some(), |bin| glib::find_program_in_path(bin).is_some()) {
        Ok(found) => found,
        Err(why) => return crate::toast(why),
    };
    if make && std::fs::create_dir_all(&out).is_err() {
        return crate::toast("Couldn't create the folder");
    }
    let plan = Plan::Run(argv, a.dir.clone(), Some(out.clone()));
    match password {
        None => ops::run_then("Extracting", plan, move || ops::flatten(&out)),
        Some(password) => ops::run_unlocking("Extracting", plan, password, move |wrong| {
            if wrong {
                ask(a, true, || {});
            } else {
                ops::flatten(&out);
            }
        }),
    }
}

/// Asks for the archive's password and extracts it; `done` runs once that is answered.
fn ask(a: Archive, again: bool, done: impl FnOnce() + 'static) {
    let detail = if again { "That password didn't work. Type it again." } else { "It's protected. Type its password to extract it." };
    let (w, body, buttons) = ops::dialog(&format!("Extract “{}”", a.name), detail);
    let entry = gtk::PasswordEntry::new();
    entry.add_css_class("search");
    entry.set_show_peek_icon(true);
    body.append(&entry);
    let cancel = kit::ui::button("Cancel", "");
    let ok = kit::ui::button("Extract", "accent");
    ok.set_sensitive(false);
    buttons.append(&cancel);
    buttons.append(&ok);
    let ww = w.clone();
    cancel.connect_clicked(move |_| ww.close());
    let button = ok.clone();
    entry.connect_changed(move |e| button.set_sensitive(!e.text().is_empty()));
    let (archive, done) = (std::cell::RefCell::new(Some(a)), std::cell::RefCell::new(Some(done)));
    w.connect_close_request(move |_| {
        if let Some(done) = done.take() {
            done();
        }
        glib::Propagation::Proceed
    });
    let (ww, typed) = (w.clone(), entry.clone());
    let commit = std::rc::Rc::new(move || {
        let text = typed.text();
        if text.is_empty() {
            return;
        }
        if let Some(a) = archive.take() {
            start(a, Some(Secret::new(&text)));
        }
        typed.set_text("");
        ww.close();
    });
    let c = commit.clone();
    ok.connect_clicked(move |_| c());
    entry.connect_activate(move |_| commit());
    w.present();
    // After the sheet has put the focus on its first control.
    glib::idle_add_local_once(move || {
        entry.grab_focus();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_seven_zip_is_given_a_password() {
        let run = |name: &str, locked: bool, tools: &[&str]| command(name, "/d/a", "/d/o", locked, |b| tools.contains(&b)).map(|(argv, make)| (argv[0].clone(), make));
        assert_eq!(run("a.zip", false, &["unzip", "7z"]), Ok(("unzip".into(), false)));
        assert_eq!(run("a.zip", true, &["unzip", "7z"]), Ok(("7z".into(), false)));
        assert_eq!(run("a.ZIP", false, &["7z"]), Ok(("7z".into(), false)));
        assert_eq!(run("a.tar.gz", false, &["tar", "7z"]), Ok(("tar".into(), true)));
        assert_eq!(run("a.tgz", false, &["7z"]), Err("tar isn't installed"));
        assert!(run("a.rar", false, &["unzip", "tar"]).is_err());
        assert!(!command("a.7z", "/d/a", "/d/o", true, |_| true).unwrap().0.iter().any(|w| w.starts_with("-p")));
    }

    #[test]
    fn a_list_says_whether_a_password_is_needed() {
        assert!(says_locked(&b"Path = a.txt\nEncrypted = -\nPath = b.txt\nEncrypted = +\n"[..]));
        assert!(says_locked(&b"\nListing archive: a.7z\n\nEnter password:"[..]));
        assert!(!says_locked(&b"Path = Encrypted = +\nEncrypted = -\n"[..]));
        assert!(!says_locked(&b""[..]));
    }

    #[test]
    fn a_protected_archive_opens_with_its_password_and_leaves_nothing_behind_without_it() {
        if glib::find_program_in_path("7z").is_none() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("digit-unpack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "hello").unwrap();
        let quiet = || std::process::Stdio::null();
        for (name, password, extra) in [("open.zip", "", &["-tzip"][..]), ("old.zip", "pass word", &["-tzip"]), ("aes.zip", "pass word", &["-tzip", "-mem=AES256"]), ("body.7z", "paß word", &[]), ("names.7z", "paß word", &["-mhe=on"])] {
            assert!(std::process::Command::new("7z").args(["a", "-bd", "-y"]).args(extra).args((!password.is_empty()).then(|| format!("-p{password}"))).args([name, "a.txt"]).current_dir(&dir).stdout(quiet()).stderr(quiet()).status().unwrap().success());
            assert_eq!(locked(&dir.join(name)), !password.is_empty(), "{name}");
            if password.is_empty() {
                continue;
            }
            let out = dir.join(format!("{name}.out"));
            let (argv, _) = command(name, &dir.join(name).to_string_lossy(), &out.to_string_lossy(), true, |_| true).unwrap();
            let go = |password: &str| ops::unlocked(Plan::Run(argv.clone(), dir.clone(), Some(out.clone())), Secret::new(password));
            assert_eq!(go("wrong"), (true, 0), "{name}");
            assert!(!out.exists(), "{name}");
            assert_eq!(go(password), (false, 0), "{name}");
            assert_eq!(std::fs::read_to_string(out.join("a.txt")).unwrap(), "hello", "{name}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
