use crate::pane::{Pane, file_of, is_dir, target_of};
use gtk::{gdk, gio, glib, pango, prelude::*};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Replace,
    Both,
    Skip,
    Stop,
}

#[derive(Clone)]
pub enum Undo {
    Created(Vec<gio::File>),
    Moved(Vec<(gio::File, gio::File)>),
    Trashed(Vec<PathBuf>),
    Renamed(gio::File, String),
}

#[derive(Default)]
struct Shared {
    done: u64,
    total: u64,
    now: String,
    finished: bool,
    errors: Vec<String>,
    ask: Option<String>,
    answer: Option<(Answer, bool)>,
    undo: Option<Undo>,
    refused: Vec<gio::File>,
}

type State = Arc<(Mutex<Shared>, Condvar)>;

pub enum Plan {
    Copy(Vec<gio::File>, gio::File),
    Move(Vec<gio::File>, gio::File),
    Pairs(Vec<(gio::File, gio::File)>),
    Trash(Vec<gio::File>),
    Delete(Vec<gio::File>),
    Run(Vec<String>, PathBuf, Option<PathBuf>),
    Native(Box<dyn FnOnce() -> Result<(), String> + Send>),
}

thread_local! {
    static UNDO: RefCell<Vec<Undo>> = const { RefCell::new(Vec::new()) };
    static ASKING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn push_undo(u: Undo) {
    UNDO.with(|s| {
        let mut s = s.borrow_mut();
        s.push(u);
        if s.len() > 30 {
            s.remove(0);
        }
    });
}

/// Opens each file with its default app; a kind of file with no default asks with Digit's own Open With.
pub fn open(files: &[gio::File]) {
    let ctx = gdk::Display::default().map(|d| d.app_launch_context());
    for f in files {
        let ct = f
            .query_info("standard::content-type", gio::FileQueryInfoFlags::NONE, None::<&gio::Cancellable>)
            .ok()
            .and_then(|i| i.content_type())
            .map(|c| c.to_string())
            .unwrap_or_else(|| "application/octet-stream".into());
        match gio::AppInfo::default_for_type(&ct, !f.is_native()) {
            Some(app) => {
                if let Err(e) = app.launch(std::slice::from_ref(f), ctx.as_ref()) {
                    crate::toast(&format!("Couldn't open: {}", e.message()));
                }
            }
            None => choose_app(&ct, vec![f.clone()]),
        }
    }
}

pub fn dropped(files: Vec<gio::File>, dest: &gio::File, copy: bool) {
    let dest = dest.clone();
    glib::spawn_future_local(async move {
        let target = dest.clone();
        let Ok((moving, copying)) = gio::spawn_blocking(move || files.into_iter().partition::<Vec<_>, _>(|f| !copy && same_fs(f, &target))).await else { return };
        if !copying.is_empty() {
            run("Copying", Plan::Copy(copying, dest.clone()));
        }
        if !moving.is_empty() {
            run("Moving", Plan::Move(moving, dest));
        }
    });
}

fn same_fs(a: &gio::File, b: &gio::File) -> bool {
    let id = |f: &gio::File| f.query_info("id::filesystem", gio::FileQueryInfoFlags::NONE, None::<&gio::Cancellable>).ok().and_then(|i| i.attribute_string("id::filesystem")).map(|s| s.to_string());
    match (id(a), id(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

pub fn run(label: &str, plan: Plan) {
    let state: State = Arc::new((Mutex::new(Shared::default()), Condvar::new()));
    let cancel = gio::Cancellable::new();
    let row = crate::jobs_row(label, &cancel);
    let st = state.clone();
    let c = cancel.clone();
    std::thread::spawn(move || {
        work(plan, &st, &c);
        let (m, _) = &*st;
        if let Ok(mut s) = m.lock() {
            s.finished = true;
        }
    });
    glib::timeout_add_local(Duration::from_millis(100), move || {
        let (m, cv) = &*state;
        let Ok(mut s) = m.lock() else { return glib::ControlFlow::Break };
        if s.total > 0 {
            row.1.set_fraction((s.done as f64 / s.total as f64).clamp(0.0, 1.0));
        } else if !s.finished {
            row.1.pulse();
        }
        if !s.now.is_empty() {
            row.2.set_text(&s.now);
        }
        if let Some(q) = s.ask.take() {
            drop(s);
            let st = state.clone();
            ASKING.with(|a| a.set(true));
            conflict(&q, move |answer, all| {
                ASKING.with(|a| a.set(false));
                let (m, cv) = &*st;
                if let Ok(mut s) = m.lock() {
                    s.answer = Some((answer, all));
                }
                cv.notify_all();
            });
            let _ = cv;
            return glib::ControlFlow::Continue;
        }
        if !s.finished {
            return glib::ControlFlow::Continue;
        }
        let errors = std::mem::take(&mut s.errors);
        let undo = s.undo.take();
        let refused = std::mem::take(&mut s.refused);
        drop(s);
        crate::jobs_done(&row.0);
        if let Some(u) = undo {
            push_undo(u);
        }
        if !errors.is_empty() {
            crate::toast(&if errors.len() == 1 { errors[0].clone() } else { format!("{} (+{} more problems)", errors[0], errors.len() - 1) });
        }
        if !refused.is_empty() {
            offer_delete(refused);
        }
        crate::refresh_all();
        glib::ControlFlow::Break
    });
}

fn ask(st: &State, question: String) -> (Answer, bool) {
    let (m, cv) = &**st;
    let Ok(mut s) = m.lock() else { return (Answer::Stop, true) };
    s.answer = None;
    s.ask = Some(question);
    loop {
        if let Some(a) = s.answer.take() {
            return a;
        }
        s = match cv.wait_timeout(s, Duration::from_millis(250)) {
            Ok((g, _)) => g,
            Err(_) => return (Answer::Stop, true),
        };
    }
}

fn note(st: &State, f: impl FnOnce(&mut Shared)) {
    if let Ok(mut s) = st.0.lock() {
        f(&mut s);
    }
}

fn name(f: &gio::File) -> String {
    f.basename().map(|b| b.to_string_lossy().into_owned()).unwrap_or_else(|| f.uri().to_string())
}

pub fn unique(dir: &gio::File, wanted: &str) -> gio::File {
    let first = dir.child(wanted);
    if !exists(&first, None) {
        return first;
    }
    let (stem, ext) = split_ext(wanted);
    for n in 2..10_000 {
        let candidate = dir.child(format!("{stem} ({n}){ext}"));
        if !exists(&candidate, None) {
            return candidate;
        }
    }
    first
}

async fn unique_off_thread(dir: &gio::File, wanted: String) -> gio::File {
    let (folder, name) = (dir.clone(), wanted.clone());
    gio::spawn_blocking(move || unique(&folder, &name)).await.unwrap_or_else(|_| dir.child(wanted))
}

pub fn split_ext(name: &str) -> (String, String) {
    for multi in [".tar.gz", ".tar.xz", ".tar.zst", ".tar.bz2"] {
        if name.len() > multi.len() && name.to_lowercase().ends_with(multi) {
            let at = name.len() - multi.len();
            return (name[..at].to_string(), name[at..].to_string());
        }
    }
    match name.rfind('.') {
        Some(i) if i > 0 => (name[..i].to_string(), name[i..].to_string()),
        _ => (name.to_string(), String::new()),
    }
}

fn measure(f: &gio::File, cancel: &gio::Cancellable) -> u64 {
    let Ok(info) = f.query_info("standard::type,standard::size", gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS, Some(cancel)) else { return 0 };
    if info.file_type() != gio::FileType::Directory {
        return info.size().max(0) as u64;
    }
    let mut sum = 0;
    if let Ok(children) = f.enumerate_children("standard::name,standard::type,standard::size", gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS, Some(cancel)) {
        while let Ok(Some(c)) = children.next_file(Some(cancel)) {
            let child = f.child(c.name());
            sum += if c.file_type() == gio::FileType::Directory { measure(&child, cancel) } else { c.size().max(0) as u64 };
        }
    }
    sum
}

/// Also true for a symlink whose target is gone.
fn exists(f: &gio::File, cancel: Option<&gio::Cancellable>) -> bool {
    f.query_info("standard::name", gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS, cancel).is_ok()
}

/// The same file reached by two names: a symlink to it, or a hard link.
fn same_item(a: &gio::File, b: &gio::File, cancel: &gio::Cancellable) -> bool {
    let id = |f: &gio::File| f.query_info("id::file", gio::FileQueryInfoFlags::NONE, Some(cancel)).ok().and_then(|i| i.attribute_string("id::file"));
    matches!((id(a), id(b)), (Some(x), Some(y)) if x == y)
}

/// Every name in a folder, or the error: a partial list must never look complete.
fn children(dir: &gio::File, cancel: &gio::Cancellable) -> Result<Vec<PathBuf>, String> {
    let list = dir.enumerate_children("standard::name", gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS, Some(cancel)).map_err(|e| format!("{}: {}", name(dir), e.message()))?;
    let mut names = Vec::new();
    loop {
        match list.next_file(Some(cancel)) {
            Ok(Some(child)) => names.push(child.name()),
            Ok(None) => return Ok(names),
            Err(e) => return Err(format!("{}: {}", name(dir), e.message())),
        }
    }
}

/// Moves `dst` out of the way, so a failed replace can put it back.
fn set_aside(dst: &gio::File, cancel: &gio::Cancellable) -> Result<Option<gio::File>, String> {
    let aside = dst.parent().map(|p| p.child(format!(".digit-replacing-{}", glib::random_int())));
    let Some(aside) = aside else { return Err(format!("{}: can't be replaced", name(dst))) };
    let flags = gio::FileCopyFlags::NOFOLLOW_SYMLINKS | gio::FileCopyFlags::NO_FALLBACK_FOR_MOVE;
    match dst.move_(&aside, flags, Some(cancel), None) {
        Ok(()) => Ok(Some(aside)),
        Err(e) if e.matches(gio::IOErrorEnum::NotSupported) => remove_tree(dst, cancel).map(|()| None).map_err(|e| format!("{}: {e}", name(dst))),
        Err(e) => Err(format!("{}: {}", name(dst), e.message())),
    }
}

/// The outermost items of a depth-first list, so nothing is undone twice.
fn tops_of(made: &[gio::File]) -> Vec<gio::File> {
    let mut tops: Vec<gio::File> = Vec::new();
    for file in made {
        if !tops.last().is_some_and(|top| file.has_prefix(top)) {
            tops.push(file.clone());
        }
    }
    tops
}

fn is_real_dir(f: &gio::File, cancel: &gio::Cancellable) -> bool {
    f.query_file_type(gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS, Some(cancel)) == gio::FileType::Directory
}

struct Ctx<'a> {
    st: &'a State,
    cancel: &'a gio::Cancellable,
    policy: Option<Answer>,
    made: Vec<gio::File>,
    moved: Vec<(gio::File, gio::File)>,
    base: u64,
}

impl Ctx<'_> {
    fn decide(&mut self, dst: &gio::File) -> Answer {
        if let Some(p) = self.policy {
            return p;
        }
        let place = dst.parent().map(|p| name(&p)).unwrap_or_default();
        let (a, all) = ask(self.st, format!("“{}” already exists in “{place}”", name(dst)));
        if all || a == Answer::Stop {
            self.policy = Some(a);
        }
        a
    }

    fn copy_tree(&mut self, src: &gio::File, dst: &gio::File) -> Result<(), String> {
        if self.cancel.is_cancelled() {
            return Err("Cancelled".into());
        }
        if is_real_dir(src, self.cancel) {
            match dst.make_directory(Some(self.cancel)) {
                Ok(()) => self.made.push(dst.clone()),
                Err(e) if e.matches(gio::IOErrorEnum::Exists) && is_real_dir(dst, self.cancel) => {}
                Err(e) => return Err(format!("{}: {}", name(dst), e.message())),
            }
            for child in children(src, self.cancel)? {
                self.place(&src.child(&child), dst.child(&child), false)?;
            }
            return Ok(());
        }
        note(self.st, |s| s.now = name(src));
        let base = self.base;
        let st = self.st.clone();
        let mut progress = |cur: i64, _: i64| note(&st, |s| s.done = base + cur.max(0) as u64);
        let flags = gio::FileCopyFlags::NOFOLLOW_SYMLINKS | gio::FileCopyFlags::ALL_METADATA | gio::FileCopyFlags::OVERWRITE;
        src.copy(dst, flags, Some(self.cancel), Some(&mut progress)).map_err(|e| format!("{}: {}", name(src), e.message()))?;
        let size = src.query_info("standard::size", gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS, None::<&gio::Cancellable>).map(|i| i.size().max(0) as u64).unwrap_or(0);
        self.base += size;
        note(self.st, |s| s.done = self.base);
        self.made.push(dst.clone());
        Ok(())
    }

    fn place(&mut self, src: &gio::File, mut dst: gio::File, moving: bool) -> Result<(), String> {
        let mut aside = None;
        if exists(&dst, Some(self.cancel)) {
            let merge = is_real_dir(src, self.cancel) && is_real_dir(&dst, self.cancel);
            match self.decide(&dst) {
                Answer::Skip => return Ok(()),
                Answer::Stop => return Err("Cancelled".into()),
                Answer::Both => dst = unique(&dst.parent().unwrap_or_else(|| dst.clone()), &name(&dst)),
                Answer::Replace if merge && moving => return self.merge(src, &dst),
                Answer::Replace if merge => return self.copy_tree(src, &dst),
                Answer::Replace if inside(src, &dst) || same_item(src, &dst, self.cancel) => return Err(format!("“{}” can't replace itself or a folder it's in", name(src))),
                Answer::Replace => aside = set_aside(&dst, self.cancel)?,
            }
        }
        let before = self.made.len();
        let result = self.transfer(src, &dst, moving);
        let Some(aside) = aside else { return result };
        let cleanup = gio::Cancellable::new();
        if result.is_ok() {
            return remove_tree(&aside, &cleanup).map_err(|e| format!("{}: {e}", name(&dst)));
        }
        self.made.truncate(before);
        let _ = remove_tree(&dst, &cleanup);
        let flags = gio::FileCopyFlags::NOFOLLOW_SYMLINKS | gio::FileCopyFlags::NO_FALLBACK_FOR_MOVE;
        if let Err(e) = aside.move_(&dst, flags, Some(&cleanup), None) {
            return Err(format!("{}: the old item is kept as {}: {}", name(&dst), name(&aside), e.message()));
        }
        result
    }

    fn transfer(&mut self, src: &gio::File, dst: &gio::File, moving: bool) -> Result<(), String> {
        if !moving {
            return self.copy_tree(src, dst);
        }
        let flags = gio::FileCopyFlags::NOFOLLOW_SYMLINKS | gio::FileCopyFlags::ALL_METADATA | gio::FileCopyFlags::NO_FALLBACK_FOR_MOVE;
        match src.move_(dst, flags, Some(self.cancel), None) {
            Ok(()) => {
                self.moved.push((src.clone(), dst.clone()));
                self.base += measure(dst, self.cancel);
                note(self.st, |s| s.done = self.base);
                return Ok(());
            }
            Err(e) if e.matches(gio::IOErrorEnum::NotSupported) || e.matches(gio::IOErrorEnum::WouldRecurse) || e.matches(gio::IOErrorEnum::WouldMerge) => {}
            Err(e) => return Err(format!("{}: {}", name(src), e.message())),
        }
        let before = self.made.len();
        self.copy_tree(src, dst)?;
        self.made.truncate(before);
        remove_tree(src, self.cancel).map_err(|e| format!("{}: {e}", name(src)))?;
        self.moved.push((src.clone(), dst.clone()));
        Ok(())
    }

    /// Moves a folder's items one by one; whatever was skipped stays where it was.
    fn merge(&mut self, src: &gio::File, dst: &gio::File) -> Result<(), String> {
        for child in children(src, self.cancel)? {
            self.place(&src.child(&child), dst.child(&child), true)?;
        }
        match src.delete(Some(self.cancel)) {
            Err(e) if !e.matches(gio::IOErrorEnum::NotEmpty) => Err(format!("{}: {}", name(src), e.message())),
            _ => Ok(()),
        }
    }
}

fn remove_tree(f: &gio::File, cancel: &gio::Cancellable) -> Result<(), String> {
    if is_real_dir(f, cancel) {
        for child in children(f, cancel)? {
            remove_tree(&f.child(child), cancel)?;
        }
    }
    f.delete(Some(cancel)).map_err(|e| e.message().to_string())
}

fn inside(child: &gio::File, parent: &gio::File) -> bool {
    child.equal(parent) || child.has_prefix(parent)
}

fn work(plan: Plan, st: &State, cancel: &gio::Cancellable) {
    let mut ctx = Ctx { st, cancel, policy: None, made: Vec::new(), moved: Vec::new(), base: 0 };
    let mut errors = Vec::new();
    match plan {
        Plan::Copy(files, dest) | Plan::Move(files, dest) if files.is_empty() => {
            let _ = dest;
        }
        Plan::Copy(files, dest) => {
            note(st, |s| s.now = "Preparing…".into());
            let total: u64 = files.iter().map(|f| measure(f, cancel)).sum();
            note(st, |s| s.total = total.max(1));
            let mut tops = Vec::new();
            for f in files {
                if inside(&dest, &f) && !f.parent().is_some_and(|p| p.equal(&dest)) {
                    errors.push(format!("Can't copy “{}” into itself", name(&f)));
                    continue;
                }
                let dst = if f.parent().is_some_and(|p| p.equal(&dest)) { unique(&dest, &name(&f)) } else { dest.child(name(&f)) };
                let before = ctx.made.len();
                if let Err(e) = ctx.place(&f, dst, false) {
                    errors.push(e);
                    if cancel.is_cancelled() || ctx.policy == Some(Answer::Stop) {
                        break;
                    }
                }
                tops.extend(tops_of(&ctx.made[before..]));
            }
            note(st, |s| s.undo = (!tops.is_empty()).then(|| Undo::Created(tops)));
        }
        Plan::Move(files, dest) => {
            note(st, |s| s.now = "Preparing…".into());
            let total: u64 = files.iter().map(|f| measure(f, cancel)).sum();
            note(st, |s| s.total = total.max(1));
            for f in files {
                if inside(&dest, &f) {
                    errors.push(format!("Can't move “{}” into itself", name(&f)));
                    continue;
                }
                if f.parent().is_some_and(|p| p.equal(&dest)) {
                    continue;
                }
                if let Err(e) = ctx.place(&f, dest.child(name(&f)), true) {
                    errors.push(e);
                    if cancel.is_cancelled() || ctx.policy == Some(Answer::Stop) {
                        break;
                    }
                }
            }
            let moved = std::mem::take(&mut ctx.moved);
            note(st, |s| s.undo = (!moved.is_empty()).then(|| Undo::Moved(moved)));
        }
        Plan::Pairs(pairs) => {
            note(st, |s| s.total = pairs.len() as u64);
            for (i, (src, dst)) in pairs.into_iter().enumerate() {
                if let Some(p) = dst.parent() {
                    let _ = p.make_directory_with_parents(Some(cancel));
                }
                if let Err(e) = ctx.place(&src, dst, true) {
                    errors.push(e);
                }
                note(st, |s| s.done = i as u64 + 1);
            }
        }
        Plan::Trash(files) => {
            note(st, |s| s.total = files.len() as u64);
            let mut gone = Vec::new();
            let mut refused = Vec::new();
            for (i, f) in files.iter().enumerate() {
                note(st, |s| s.now = name(f));
                match f.trash(Some(cancel)) {
                    Ok(()) => {
                        if let Some(p) = f.path() {
                            gone.push(p);
                        }
                    }
                    Err(e) if e.matches(gio::IOErrorEnum::NotSupported) || e.matches(gio::IOErrorEnum::PermissionDenied) => refused.push(f.clone()),
                    Err(e) => errors.push(format!("{}: {}", name(f), e.message())),
                }
                note(st, |s| s.done = i as u64 + 1);
            }
            note(st, |s| {
                s.undo = (!gone.is_empty()).then(|| Undo::Trashed(gone));
                s.refused = refused;
            });
        }
        Plan::Delete(files) => {
            note(st, |s| s.total = files.len() as u64);
            for (i, f) in files.iter().enumerate() {
                note(st, |s| s.now = name(f));
                if let Err(e) = remove_tree(f, cancel) {
                    errors.push(format!("{}: {e}", name(f)));
                }
                note(st, |s| s.done = i as u64 + 1);
            }
        }
        Plan::Native(job) => {
            note(st, |s| s.now = "Working…".into());
            if let Err(e) = job() {
                errors.push(e);
            }
        }
        Plan::Run(argv, cwd, made) => {
            note(st, |s| s.now = argv.first().cloned().unwrap_or_default());
            match std::process::Command::new(&argv[0]).args(&argv[1..]).current_dir(&cwd).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped()).spawn() {
                Ok(mut child) => {
                    // Read while it runs: a full pipe would stop the tool for good.
                    let complaint = child.stderr.take().map(|e| std::thread::spawn(move || last_line(e)));
                    loop {
                        if cancel.is_cancelled() {
                            let _ = child.kill();
                            let _ = child.wait();
                            if let Some(m) = &made {
                                let _ = remove_tree(&gio::File::for_path(m), &gio::Cancellable::new());
                            }
                            break;
                        }
                        match child.try_wait() {
                            Ok(Some(status)) => {
                                if !status.success() {
                                    let err = complaint.and_then(|c| c.join().ok()).flatten();
                                    errors.push(err.unwrap_or_else(|| format!("{} failed", argv[0])));
                                } else if let Some(m) = made {
                                    note(st, |s| s.undo = Some(Undo::Created(vec![gio::File::for_path(m)])));
                                }
                                break;
                            }
                            Ok(None) => std::thread::sleep(Duration::from_millis(80)),
                            Err(e) => {
                                errors.push(e.to_string());
                                break;
                            }
                        }
                    }
                }
                Err(e) => errors.push(format!("{}: {e}", argv[0])),
            }
        }
    }
    if cancel.is_cancelled() {
        errors.retain(|e| e != "Cancelled");
    }
    note(st, |s| s.errors = errors);
}

fn offer_delete(files: Vec<gio::File>) {
    let n = files.len();
    let title = if n == 1 { format!("“{}” can't go to the trash", name(&files[0])) } else { format!("{n} items can't go to the trash") };
    kit::ui::ask(crate::window().as_ref(), &title, "This location has no trash. Delete permanently instead? This can't be undone.", "Cancel", "Delete Permanently", "danger", move || run("Deleting", Plan::Delete(files)));
}

pub fn trash(files: Vec<gio::File>) {
    if files.is_empty() {
        return;
    }
    run("Moving to trash", Plan::Trash(files));
}

pub fn delete(files: Vec<gio::File>) {
    if files.is_empty() {
        return;
    }
    let n = files.len();
    let title = if n == 1 { format!("Delete “{}” permanently?", name(&files[0])) } else { format!("Delete {n} items permanently?") };
    kit::ui::ask(crate::window().as_ref(), &title, "They won't go to the trash and can't be recovered.", "Cancel", "Delete", "danger", move || run("Deleting", Plan::Delete(files)));
}

pub fn restore(infos: Vec<gio::FileInfo>) {
    let pairs: Vec<(gio::File, gio::File)> = infos
        .iter()
        .filter_map(|i| {
            let src = file_of(i)?;
            let orig = i.attribute_byte_string("trash::orig-path")?;
            Some((src, gio::File::for_path(orig.as_str())))
        })
        .collect();
    if !pairs.is_empty() {
        run("Restoring", Plan::Pairs(pairs));
    }
}

pub fn empty_trash() {
    kit::ui::ask(crate::window().as_ref(), "Empty the trash?", "Everything in the trash will be deleted permanently.", "Cancel", "Empty Trash", "danger", || {
        let trash = gio::File::for_uri("trash:///");
        let mut items = Vec::new();
        if let Ok(e) = trash.enumerate_children("standard::name", gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS, None::<&gio::Cancellable>) {
            while let Ok(Some(c)) = e.next_file(None::<&gio::Cancellable>) {
                items.push(trash.child(c.name()));
            }
        }
        if !items.is_empty() {
            run("Emptying trash", Plan::Delete(items));
        }
    });
}

fn last_line(stream: impl std::io::Read) -> Option<String> {
    use std::io::BufRead;
    let lines = std::io::BufReader::new(stream).split(b'\n').map_while(Result::ok);
    lines.map(|l| String::from_utf8_lossy(&l).trim().to_string()).filter(|l| !l.is_empty()).last()
}

pub fn undo() {
    let Some(u) = UNDO.with(|s| s.borrow_mut().pop()) else {
        crate::toast("Nothing to undo");
        return;
    };
    match u {
        Undo::Created(files) => run("Undoing", Plan::Trash(files)),
        Undo::Moved(pairs) => run("Undoing", Plan::Pairs(pairs.into_iter().map(|(a, b)| (b, a)).collect())),
        Undo::Renamed(file, old) => match file.set_display_name(&old, None::<&gio::Cancellable>) {
            Ok(_) => crate::refresh_all(),
            Err(e) => crate::toast(&e.message().to_string()),
        },
        Undo::Trashed(paths) => {
            let trash = gio::File::for_uri("trash:///");
            let mut pairs = Vec::new();
            if let Ok(e) = trash.enumerate_children("standard::name,trash::orig-path,trash::deletion-date", gio::FileQueryInfoFlags::NONE, None::<&gio::Cancellable>) {
                let mut found: Vec<(gio::FileInfo, PathBuf)> = Vec::new();
                while let Ok(Some(c)) = e.next_file(None::<&gio::Cancellable>) {
                    if let Some(orig) = c.attribute_byte_string("trash::orig-path") {
                        let orig = PathBuf::from(orig.as_str());
                        if paths.contains(&orig) {
                            found.push((c, orig));
                        }
                    }
                }
                found.sort_by_key(|(c, _)| std::cmp::Reverse(c.deletion_date().map(|d| d.to_unix()).unwrap_or(0)));
                let mut seen: Vec<PathBuf> = Vec::new();
                for (c, orig) in found {
                    if seen.contains(&orig) {
                        continue;
                    }
                    seen.push(orig.clone());
                    pairs.push((trash.child(c.name()), gio::File::for_path(orig)));
                }
            }
            if pairs.is_empty() {
                crate::toast("Those items are no longer in the trash");
            } else {
                run("Undoing", Plan::Pairs(pairs));
            }
        }
    }
}

const GNOME: &str = "x-special/gnome-copied-files";

pub fn clip(files: &[gio::File], cut: bool) {
    let Some(display) = gdk::Display::default() else { return };
    if files.is_empty() {
        return;
    }
    let uris: Vec<String> = files.iter().map(|f| f.uri().to_string()).collect();
    let gnome = format!("{}\n{}", if cut { "cut" } else { "copy" }, uris.join("\n"));
    let text = files.iter().map(|f| f.path().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| f.uri().to_string())).collect::<Vec<_>>().join("\n");
    let provider = gdk::ContentProvider::new_union(&[
        gdk::ContentProvider::for_value(&gdk::FileList::from_array(files).to_value()),
        gdk::ContentProvider::for_bytes(GNOME, &glib::Bytes::from_owned(gnome.into_bytes())),
        gdk::ContentProvider::for_bytes("text/uri-list", &glib::Bytes::from_owned(format!("{}\r\n", uris.join("\r\n")).into_bytes())),
        gdk::ContentProvider::for_bytes("text/plain;charset=utf-8", &glib::Bytes::from_owned(text.into_bytes())),
    ]);
    let _ = display.clipboard().set_content(Some(&provider));
    crate::set_cut(if cut { uris } else { Vec::new() });
}

pub fn paste(dest: gio::File) {
    let Some(display) = gdk::Display::default() else { return };
    let clipboard = display.clipboard();
    let formats = clipboard.formats();
    glib::spawn_future_local(async move {
        if formats.contain_mime_type(GNOME) {
            if let Ok((stream, _)) = clipboard.read_future(&[GNOME], glib::Priority::DEFAULT).await {
                let out = gio::MemoryOutputStream::new_resizable();
                if out.splice_future(&stream, gio::OutputStreamSpliceFlags::CLOSE_SOURCE | gio::OutputStreamSpliceFlags::CLOSE_TARGET, glib::Priority::DEFAULT).await.is_ok() {
                    let bytes = out.steal_as_bytes();
                    let text = String::from_utf8_lossy(&bytes).to_string();
                    let mut lines = text.lines();
                    let cut = lines.next() == Some("cut");
                    let files: Vec<gio::File> = lines.filter(|l| !l.is_empty()).map(gio::File::for_uri).collect();
                    if !files.is_empty() {
                        if cut {
                            crate::set_cut(Vec::new());
                            let _ = clipboard.set_content(None::<&gdk::ContentProvider>);
                            run("Moving", Plan::Move(files, dest));
                        } else {
                            run("Copying", Plan::Copy(files, dest));
                        }
                        return;
                    }
                }
            }
        }
        if formats.contains_type(gdk::FileList::static_type()) {
            if let Ok(v) = clipboard.read_value_future(gdk::FileList::static_type(), glib::Priority::DEFAULT).await {
                if let Ok(list) = v.get::<gdk::FileList>() {
                    let files = list.files();
                    if !files.is_empty() {
                        run("Copying", Plan::Copy(files, dest));
                        return;
                    }
                }
            }
        }
        let stamp = glib::DateTime::now_local().ok().and_then(|d| d.format("%Y-%m-%d %H.%M.%S").ok()).map(|s| s.to_string()).unwrap_or_default();
        if formats.contains_type(gdk::Texture::static_type()) {
            if let Ok(Some(tex)) = clipboard.read_texture_future().await {
                let target = unique_off_thread(&dest, format!("Pasted image {stamp}.png")).await;
                match target.path().map(|p| tex.save_to_png(p)) {
                    Some(Ok(())) => {
                        push_undo(Undo::Created(vec![target.clone()]));
                        crate::select_later(&target);
                    }
                    _ => crate::toast("Couldn't save the pasted image here"),
                }
                return;
            }
        }
        if let Ok(Some(text)) = clipboard.read_text_future().await {
            let target = unique_off_thread(&dest, format!("Pasted text {stamp}.txt")).await;
            match target.replace_contents(text.as_bytes(), None, false, gio::FileCreateFlags::NONE, None::<&gio::Cancellable>) {
                Ok(_) => {
                    push_undo(Undo::Created(vec![target.clone()]));
                    crate::select_later(&target);
                }
                Err(e) => crate::toast(&e.message().to_string()),
            }
            return;
        }
        crate::toast("Nothing to paste");
    });
}

type Ask = Rc<dyn Fn(&Sheet) -> glib::Propagation>;

#[derive(Clone)]
pub struct Sheet {
    root: gtk::Box,
    card: gtk::Box,
    shown: Rc<std::cell::Cell<bool>>,
    asks: Rc<RefCell<Vec<Ask>>>,
}

impl Sheet {
    pub fn present(&self) {
        if self.shown.replace(true) {
            return;
        }
        self.root.set_opacity(0.0);
        self.root.set_can_target(true);
        crate::overlay_add(&self.root);
        crate::look::play(&self.root, &self.card, SHEET_MARGIN, true, || {});
        let root = self.root.clone();
        glib::idle_add_local_once(move || {
            root.child_focus(gtk::DirectionType::TabForward);
        });
    }

    pub fn close(&self) {
        if !self.shown.get() {
            return;
        }
        let asks: Vec<Ask> = self.asks.borrow().clone();
        for ask in asks {
            if ask(self) == glib::Propagation::Stop {
                return;
            }
        }
        self.shown.set(false);
        self.root.set_can_target(false);
        let root = self.root.clone();
        crate::look::play(&self.root, &self.card, SHEET_MARGIN, false, move || crate::overlay_remove(&root));
    }

    pub fn is_shown(&self) -> bool {
        self.shown.get()
    }

    pub fn connect_close_request(&self, f: impl Fn(&Sheet) -> glib::Propagation + 'static) {
        self.asks.borrow_mut().push(Rc::new(f));
    }
}

const SHEET_MARGIN: i32 = 24;

pub fn dialog(title: &str, detail: &str) -> (Sheet, gtk::Box, gtk::Box) {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("sheet-backdrop");
    root.set_hexpand(true);
    root.set_vexpand(true);
    let card = gtk::Box::new(gtk::Orientation::Vertical, 12);
    card.add_css_class("dialog");
    card.set_size_request(420, -1);
    card.set_halign(gtk::Align::Center);
    card.set_valign(gtk::Align::Center);
    card.set_vexpand(true);
    card.set_margin_start(24);
    card.set_margin_end(24);
    card.set_margin_top(SHEET_MARGIN + crate::look::DROP as i32);
    card.set_margin_bottom(SHEET_MARGIN - crate::look::DROP as i32);
    card.append(&kit::ui::text(title, "name big"));
    if !detail.is_empty() {
        let d = kit::ui::text(detail, "sub");
        d.set_wrap(true);
        d.set_max_width_chars(52);
        card.append(&d);
    }
    let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
    card.append(&body);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    buttons.set_margin_top(6);
    card.append(&buttons);
    root.append(&card);
    let sheet = Sheet { root: root.clone(), card: card.clone(), shown: Rc::new(std::cell::Cell::new(false)), asks: Rc::new(RefCell::new(Vec::new())) };
    let keys = gtk::EventControllerKey::new();
    let me = sheet.clone();
    keys.connect_key_pressed(move |_, k, _, _| {
        if k == gdk::Key::Escape {
            me.close();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    root.add_controller(keys);
    let click = gtk::GestureClick::new();
    let (me, inner) = (sheet.clone(), card.clone());
    click.connect_released(move |g, _, x, y| {
        let Some(w) = g.widget() else { return };
        if !w.pick(x, y, gtk::PickFlags::DEFAULT).is_some_and(|p| p == inner || p.is_ancestor(&inner)) {
            me.close();
        }
    });
    root.add_controller(click);
    (sheet, body, buttons)
}

fn conflict(question: &str, done: impl Fn(Answer, bool) + 'static) {
    let (w, body, buttons) = dialog(question, "Replacing it overwrites its contents. Folders are merged.");
    let all = gtk::CheckButton::with_label("Do this for every conflict");
    body.append(&all);
    let done = Rc::new(done);
    let answered = Rc::new(std::cell::Cell::new(false));
    for (label, answer, kind) in [("Stop", Answer::Stop, ""), ("Skip", Answer::Skip, ""), ("Keep Both", Answer::Both, ""), ("Replace", Answer::Replace, "accent")] {
        let b = kit::ui::button(label, kind);
        let (w2, all2, done2, answered2) = (w.clone(), all.clone(), done.clone(), answered.clone());
        b.connect_clicked(move |_| {
            answered2.set(true);
            done2(answer, all2.is_active());
            w2.close();
        });
        buttons.append(&b);
    }
    w.connect_close_request(move |_| {
        if !answered.get() {
            answered.set(true);
            done(Answer::Stop, true);
        }
        glib::Propagation::Proceed
    });
    w.present();
}

pub fn ask_name(title: &str, initial: &str, action: &str, select_stem: bool, extra: Option<gtk::Widget>, done: impl Fn(String, &Sheet) -> bool + 'static) {
    let (w, body, buttons) = dialog(title, "");
    let entry = gtk::Entry::new();
    entry.add_css_class("search");
    entry.set_text(initial);
    body.append(&entry);
    if let Some(x) = extra {
        body.append(&x);
    }
    let error = kit::ui::text("", "warn");
    error.set_visible(false);
    body.append(&error);
    let cancel = kit::ui::button("Cancel", "");
    let ok = kit::ui::button(action, "accent");
    let ww = w.clone();
    cancel.connect_clicked(move |_| ww.close());
    buttons.append(&cancel);
    buttons.append(&ok);
    let done = Rc::new(done);
    let commit = {
        let (w, entry, error, done) = (w.clone(), entry.clone(), error.clone(), done.clone());
        move || {
            let text = entry.text().trim().to_string();
            if text.is_empty() || text == "." || text == ".." || text.contains('/') {
                error.set_text("Names can't be empty or contain “/”");
                error.set_visible(true);
                return;
            }
            if done(text, &w) {
                w.close();
            }
        }
    };
    let c = commit.clone();
    ok.connect_clicked(move |_| c());
    entry.connect_activate(move |_| commit());
    w.present();
    entry.grab_focus();
    if select_stem {
        let (stem, _) = split_ext(initial);
        entry.select_region(0, stem.chars().count() as i32);
    } else {
        entry.select_region(0, -1);
    }
}

pub fn rename(info: gio::FileInfo) {
    let Some(file) = file_of(&info) else { return };
    let old = info.display_name().to_string();
    let dir = is_dir(&info);
    let initial = old.clone();
    ask_name("Rename", &initial, "Rename", !dir, None, move |new, _| {
        if new == old {
            return true;
        }
        if file.parent().is_some_and(|p| p.child(&new).query_exists(None::<&gio::Cancellable>)) && !new.eq_ignore_ascii_case(&old) {
            crate::toast(&format!("“{new}” already exists"));
            return false;
        }
        match file.set_display_name(&new, None::<&gio::Cancellable>) {
            Ok(renamed) => {
                push_undo(Undo::Renamed(renamed.clone(), old.clone()));
                crate::select_later(&renamed);
                true
            }
            Err(e) => {
                crate::toast(&e.message().to_string());
                false
            }
        }
    });
}

pub fn new_folder(dest: gio::File) {
    glib::spawn_future_local(async move {
        let initial = name(&unique_off_thread(&dest, "New Folder".into()).await);
        ask_new_folder(dest, &initial);
    });
}

fn ask_new_folder(dest: gio::File, initial: &str) {
    ask_name("New Folder", initial, "Create", false, None, move |n, _| {
        let f = dest.child(&n);
        match f.make_directory(None::<&gio::Cancellable>) {
            Ok(()) => {
                push_undo(Undo::Created(vec![f.clone()]));
                crate::select_later(&f);
                true
            }
            Err(e) => {
                crate::toast(&e.message().to_string());
                false
            }
        }
    });
}

pub fn templates() -> Vec<PathBuf> {
    let Some(dir) = glib::user_special_dir(glib::UserDirectory::Templates) else { return Vec::new() };
    if dir == glib::home_dir() {
        return Vec::new();
    }
    let mut out: Vec<PathBuf> = std::fs::read_dir(&dir).map(|r| r.flatten().map(|e| e.path()).filter(|p| p.is_file() && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))).collect()).unwrap_or_default();
    out.sort();
    out
}

pub fn new_file(dest: gio::File, template: Option<PathBuf>) {
    let wanted = template.as_ref().and_then(|t| t.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "New File.txt".into());
    glib::spawn_future_local(async move {
        let initial = name(&unique_off_thread(&dest, wanted).await);
        ask_new_file(dest, template, &initial);
    });
}

fn ask_new_file(dest: gio::File, template: Option<PathBuf>, initial: &str) {
    ask_name("New File", initial, "Create", true, None, move |n, _| {
        let f = dest.child(&n);
        if f.query_exists(None::<&gio::Cancellable>) {
            crate::toast(&format!("“{n}” already exists"));
            return false;
        }
        let made = match &template {
            Some(t) => gio::File::for_path(t).copy(&f, gio::FileCopyFlags::NONE, None::<&gio::Cancellable>, None).map(|_| ()),
            None => f.create(gio::FileCreateFlags::NONE, None::<&gio::Cancellable>).map(|_| ()),
        };
        match made {
            Ok(()) => {
                push_undo(Undo::Created(vec![f.clone()]));
                crate::select_later(&f);
                true
            }
            Err(e) => {
                crate::toast(&e.message().to_string());
                false
            }
        }
    });
}

fn has(bin: &str) -> bool {
    glib::find_program_in_path(bin).is_some()
}

pub fn compress(files: Vec<gio::File>) {
    let Some(dir) = files.first().and_then(|f| f.parent()) else { return };
    let (Some(cwd), true) = (dir.path(), files.iter().all(|f| f.parent().is_some_and(|p| p.equal(&dir)) && f.is_native())) else {
        crate::toast("Compressing works on local files in one folder");
        return;
    };
    let mut formats: Vec<(&str, &str)> = Vec::new();
    if has("zip") {
        formats.push((".zip", "zip"));
    }
    if has("tar") {
        formats.extend([(".tar.gz", "tar"), (".tar.xz", "tar"), (".tar.zst", "tar")]);
    }
    if has("7z") {
        formats.push((".7z", "7z"));
    }
    if formats.is_empty() {
        crate::toast("No archive tool found (zip, tar or 7z)");
        return;
    }
    let base = if files.len() == 1 { split_ext(&name(&files[0])).0 } else { "Archive".to_string() };
    let labels: Vec<&str> = formats.iter().map(|f| f.0).collect();
    let pick = kit::ui::dropdown(&labels, 0);
    let names: Vec<String> = files.iter().map(name).collect();
    let extra = gtk::Box::new(gtk::Orientation::Vertical, 8);
    extra.append(&pick);
    let each = gtk::CheckButton::with_label("Each item in its own archive");
    if names.len() > 1 {
        extra.append(&each);
    }
    let p2 = pick.clone();
    ask_name("Compress", &base, "Create", false, Some(extra.upcast()), move |stem, _| {
        let (ext, tool) = formats[p2.selected() as usize];
        let groups: Vec<(String, Vec<String>)> = if each.is_active() {
            names.iter().map(|n| (split_ext(n).0, vec![n.clone()])).collect()
        } else {
            vec![(stem, names.clone())]
        };
        for (stem, members) in groups {
            let out = unique(&dir, &format!("{stem}{ext}"));
            let Some(outp) = out.path() else { continue };
            let mut argv: Vec<String> = match tool {
                "zip" => vec!["zip".into(), "-r".into(), "-q".into(), "-y".into(), outp.to_string_lossy().into_owned(), "--".into()],
                "7z" => vec!["7z".into(), "a".into(), "-bd".into(), "-y".into(), outp.to_string_lossy().into_owned(), "--".into()],
                _ => vec!["tar".into(), "-c".into(), "-a".into(), "-f".into(), outp.to_string_lossy().into_owned(), "--".into()],
            };
            argv.extend(members);
            run("Compressing", Plan::Run(argv, cwd.clone(), Some(outp)));
        }
        true
    });
}

pub fn archive(info: &gio::FileInfo) -> bool {
    let n = info.name().to_string_lossy().to_lowercase();
    [".zip", ".tar", ".tar.gz", ".tgz", ".tar.xz", ".txz", ".tar.zst", ".tar.bz2", ".tbz2", ".7z", ".rar", ".gz", ".xz", ".zst", ".bz2", ".iso", ".jar", ".apk", ".deb", ".rpm", ".cab"].iter().any(|e| n.ends_with(e))
}

pub fn extract(files: Vec<gio::File>) {
    for f in files {
        let (Some(path), Some(dir)) = (f.path(), f.parent().and_then(|p| p.path())) else { continue };
        let n = name(&f);
        let lower = n.to_lowercase();
        let stem = split_ext(&n).0;
        let target = unique(&gio::File::for_path(&dir), &stem);
        let Some(out) = target.path() else { continue };
        let p = path.to_string_lossy().into_owned();
        let o = out.to_string_lossy().into_owned();
        let argv: Vec<String> = if lower.contains(".tar") || lower.ends_with(".tgz") || lower.ends_with(".txz") || lower.ends_with(".tbz2") {
            if !has("tar") {
                crate::toast("tar isn't installed");
                continue;
            }
            vec!["sh_mkdir".into(), o.clone(), "tar".into(), "-x".into(), "-f".into(), p, "-C".into(), o.clone()]
        } else if lower.ends_with(".zip") && has("unzip") {
            vec!["unzip".into(), "-q".into(), "-n".into(), p, "-d".into(), o.clone()]
        } else if has("7z") {
            vec!["7z".into(), "x".into(), "-bd".into(), "-y".into(), format!("-o{o}"), "--".into(), p]
        } else {
            crate::toast("No tool can open this archive (install 7-Zip)");
            continue;
        };
        let argv = if argv[0] == "sh_mkdir" {
            if std::fs::create_dir_all(&argv[1]).is_err() {
                crate::toast("Couldn't create the folder");
                continue;
            }
            argv[2..].to_vec()
        } else {
            argv
        };
        run_then("Extracting", Plan::Run(argv, dir.clone(), Some(out.clone())), move || flatten(&out));
    }
}

pub fn run_then(label: &str, plan: Plan, after: impl FnOnce() + 'static) {
    run(label, plan);
    let after = RefCell::new(Some(after));
    glib::timeout_add_local(Duration::from_millis(300), move || {
        if crate::jobs_busy() {
            return glib::ControlFlow::Continue;
        }
        let then = after.borrow_mut().take();
        if let Some(a) = then {
            a();
        }
        glib::ControlFlow::Break
    });
}

fn flatten(out: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(out).map(|r| r.flatten().collect::<Vec<_>>()) else { return };
    if let [only] = entries.as_slice() {
        let Some(parent) = out.parent() else { return };
        let dest = parent.join(only.file_name());
        if !dest.exists() && std::fs::rename(only.path(), &dest).is_ok() {
            let _ = std::fs::remove_dir(out);
            crate::select_later(&gio::File::for_path(dest));
            return;
        }
    }
    crate::select_later(&gio::File::for_path(out));
}

pub fn terminal(dir: &gio::File) {
    let Some(path) = dir.path() else {
        crate::toast("Terminals open only in local folders");
        return;
    };
    let line = kit::terminal::command();
    let argv: Vec<&std::ffi::OsStr> = line.split_whitespace().map(std::ffi::OsStr::new).collect();
    let bin = argv.first().map(|b| b.to_string_lossy().into_owned()).unwrap_or_default();
    if argv.is_empty() || !has(&bin) {
        crate::toast("No terminal found");
        return;
    }
    let launcher = gio::SubprocessLauncher::new(gio::SubprocessFlags::NONE);
    launcher.set_cwd(&path);
    if launcher.spawn(&argv).is_err() {
        crate::toast(&format!("Couldn't start {bin}"));
    }
}

pub fn open_with(infos: Vec<gio::FileInfo>) {
    let Some(first) = infos.first() else { return };
    let ct = first.attribute_string("standard::fast-content-type").map(|s| s.to_string()).unwrap_or_else(|| "application/octet-stream".into());
    choose_app(&ct, infos.iter().filter_map(target_of).collect());
}

fn choose_app(ct: &str, files: Vec<gio::File>) {
    let ct = ct.to_string();
    let (w, body, buttons) = dialog("Open With", &gio::content_type_get_description(&ct));
    let mut apps = gio::AppInfo::recommended_for_type(&ct);
    for a in gio::AppInfo::all_for_type(&ct) {
        if !apps.iter().any(|b| b.id() == a.id()) {
            apps.push(a);
        }
    }
    let default = gio::AppInfo::default_for_type(&ct, false).and_then(|a| a.id());
    let always = gtk::CheckButton::with_label("Always use for this kind of file");
    let list = gtk::ListBox::new();
    list.add_css_class("picker");
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_min_content_height(260.min(56 * apps.len().max(1) as i32));
    scroll.set_max_content_height(360);
    scroll.set_propagate_natural_height(true);
    scroll.set_child(Some(&list));
    for a in &apps {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.add_css_class("row");
        let img = a.icon().map(|i| gtk::Image::from_gicon(&i)).unwrap_or_else(|| gtk::Image::from_icon_name("application-x-executable"));
        img.set_pixel_size(32);
        row.append(&img);
        let l = kit::ui::text(&a.display_name(), "name");
        l.set_hexpand(true);
        l.set_ellipsize(pango::EllipsizeMode::End);
        row.append(&l);
        if a.id() == default {
            row.append(&kit::ui::badge("Default", "accent"));
        }
        list.append(&row);
    }
    if apps.is_empty() {
        body.append(&kit::ui::text("No installed app opens this kind of file.", "dim"));
    } else {
        body.append(&scroll);
        body.append(&always);
    }
    let close = kit::ui::button("Cancel", "");
    let ww = w.clone();
    close.connect_clicked(move |_| ww.close());
    buttons.append(&close);
    let ww = w.clone();
    list.connect_row_activated(move |_, row| {
        let Some(app) = apps.get(row.index() as usize) else { return };
        if always.is_active() {
            let _ = app.set_as_default_for_type(&ct);
        }
        let ctx = gdk::Display::default().map(|d| d.app_launch_context());
        if let Err(e) = app.launch(&files, ctx.as_ref()) {
            crate::toast(&e.message().to_string());
        }
        ww.close();
    });
    w.present();
}

pub fn copy_paths(files: &[gio::File]) {
    let text = files.iter().map(|f| f.path().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| f.uri().to_string())).collect::<Vec<_>>().join("\n");
    if let Some(d) = gdk::Display::default() {
        d.clipboard().set_text(&text);
        crate::toast(if files.len() == 1 { "Path copied" } else { "Paths copied" });
    }
}

pub fn batch_rename(infos: Vec<gio::FileInfo>) {
    let (w, body, buttons) = dialog(&format!("Rename {} Items", infos.len()), "");
    let mode = kit::ui::dropdown(&["Replace text", "Number in order", "Add text", "Change case"], 0);
    body.append(&mode);
    let a = gtk::Entry::new();
    let b = gtk::Entry::new();
    let start = gtk::SpinButton::with_range(0.0, 100_000.0, 1.0);
    start.set_value(1.0);
    let pad = gtk::SpinButton::with_range(1.0, 8.0, 1.0);
    pad.set_value(2.0);
    let case = kit::ui::dropdown(&["lowercase", "UPPERCASE", "Title Case"], 0);
    let at_end = gtk::CheckButton::with_label("Add at the end");
    for e in [&a, &b] {
        e.add_css_class("search");
    }
    let fields = gtk::Box::new(gtk::Orientation::Vertical, 8);
    body.append(&fields);
    let preview = gtk::ListBox::new();
    preview.add_css_class("picker");
    preview.set_selection_mode(gtk::SelectionMode::None);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_min_content_height(200);
    scroll.set_max_content_height(320);
    scroll.set_child(Some(&preview));
    body.append(&scroll);
    let names: Vec<String> = infos.iter().map(|i| i.display_name().to_string()).collect();
    let compute: Rc<dyn Fn() -> Vec<String>> = {
        let (mode, a, b, start, pad, case, at_end, names) = (mode.clone(), a.clone(), b.clone(), start.clone(), pad.clone(), case.clone(), at_end.clone(), names.clone());
        Rc::new(move || {
            names
                .iter()
                .enumerate()
                .map(|(i, n)| {
                    let (stem, ext) = split_ext(n);
                    match mode.selected() {
                        0 => {
                            let find = a.text();
                            if find.is_empty() { n.clone() } else { format!("{}{ext}", stem.replace(find.as_str(), b.text().as_str())) }
                        }
                        1 => {
                            let base = a.text();
                            let num = format!("{:0width$}", start.value() as i64 + i as i64, width = pad.value() as usize);
                            let base = if base.is_empty() { stem.clone() } else { base.to_string() };
                            format!("{base} {num}{ext}")
                        }
                        2 => {
                            if at_end.is_active() { format!("{stem}{}{ext}", a.text()) } else { format!("{}{stem}{ext}", a.text()) }
                        }
                        _ => {
                            let s = match case.selected() {
                                0 => stem.to_lowercase(),
                                1 => stem.to_uppercase(),
                                _ => stem.split(' ').map(|w| {
                                    let mut c = w.chars();
                                    c.next().map(|f| f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase()).unwrap_or_default()
                                }).collect::<Vec<_>>().join(" "),
                            };
                            format!("{s}{ext}")
                        }
                    }
                })
                .collect()
        })
    };
    let refresh: Rc<dyn Fn()> = {
        let (mode, fields, a, b, start, pad, case, at_end, preview, compute, names) = (mode.clone(), fields.clone(), a.clone(), b.clone(), start.clone(), pad.clone(), case.clone(), at_end.clone(), preview.clone(), compute.clone(), names.clone());
        Rc::new(move || {
            kit::ui::clear(&fields);
            let field = |label: &str, w: &gtk::Widget| {
                let r = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                let l = kit::ui::text(label, "sub");
                l.set_size_request(90, -1);
                r.append(&l);
                w.set_hexpand(true);
                r.append(w);
                r
            };
            match mode.selected() {
                0 => {
                    fields.append(&field("Find", a.upcast_ref()));
                    fields.append(&field("Replace with", b.upcast_ref()));
                }
                1 => {
                    fields.append(&field("Name", a.upcast_ref()));
                    fields.append(&field("Start at", start.upcast_ref()));
                    fields.append(&field("Digits", pad.upcast_ref()));
                }
                2 => {
                    fields.append(&field("Text", a.upcast_ref()));
                    fields.append(&at_end);
                }
                _ => fields.append(&field("Case", case.upcast_ref())),
            }
            while let Some(c) = preview.first_child() {
                preview.remove(&c);
            }
            let new = compute();
            let mut seen = std::collections::HashSet::new();
            for (old, n) in names.iter().zip(new.iter()).take(200) {
                let r = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                r.add_css_class("row");
                let o = kit::ui::text(old, "dim");
                o.set_ellipsize(pango::EllipsizeMode::Middle);
                o.set_hexpand(true);
                o.set_width_chars(16);
                let arrow = kit::ui::sym("arrow_forward");
                let nl = kit::ui::text(n, if seen.insert(n.clone()) { "name" } else { "warn" });
                nl.set_ellipsize(pango::EllipsizeMode::Middle);
                nl.set_hexpand(true);
                nl.set_width_chars(16);
                r.append(&o);
                r.append(&arrow);
                r.append(&nl);
                preview.append(&r);
            }
        })
    };
    refresh();
    let r = refresh.clone();
    mode.connect_selected_notify(move |_| r());
    for e in [&a, &b] {
        let r = refresh.clone();
        e.connect_changed(move |_| r());
    }
    for s in [&start, &pad] {
        let r = refresh.clone();
        s.connect_value_changed(move |_| r());
    }
    let r = refresh.clone();
    case.connect_selected_notify(move |_| r());
    let r = refresh.clone();
    at_end.connect_toggled(move |_| r());
    let cancel = kit::ui::button("Cancel", "");
    let ok = kit::ui::button("Rename", "accent");
    let ww = w.clone();
    cancel.connect_clicked(move |_| ww.close());
    let ww = w.clone();
    ok.connect_clicked(move |_| {
        let new = compute();
        let unique: std::collections::HashSet<&String> = new.iter().collect();
        if unique.len() != new.len() || new.iter().any(|n| n.is_empty() || n.contains('/')) {
            crate::toast("Some new names collide or are invalid");
            return;
        }
        let files: Vec<gio::File> = infos.iter().filter_map(file_of).collect();
        let mut temps = Vec::new();
        for (i, f) in files.iter().enumerate() {
            if names[i] == new[i] {
                continue;
            }
            match f.set_display_name(&format!(".digit-rename-{i}-{}", glib::random_int()), None::<&gio::Cancellable>) {
                Ok(t) => temps.push((t, i)),
                Err(e) => crate::toast(&e.message().to_string()),
            }
        }
        let mut failed = 0;
        for (t, i) in temps {
            let target = t.parent().map(|p| p.child(&new[i]));
            let taken = target.as_ref().is_some_and(|x| x.query_exists(None::<&gio::Cancellable>));
            let name = if taken { names[i].clone() } else { new[i].clone() };
            match t.set_display_name(&name, None::<&gio::Cancellable>) {
                Ok(done) => {
                    if !taken {
                        push_undo(Undo::Renamed(done, names[i].clone()));
                    } else {
                        failed += 1;
                    }
                }
                Err(_) => failed += 1,
            }
        }
        if failed > 0 {
            crate::toast(&format!("{failed} item(s) kept their names because the new name was taken"));
        }
        crate::refresh_all();
        ww.close();
    });
    buttons.append(&cancel);
    buttons.append(&ok);
    w.present();
}

pub fn pane_of_trash(p: &Pane) -> bool {
    p.location().uri_scheme().is_some_and(|s| s == "trash")
}

pub fn sizes(file: gio::File, done: impl FnOnce(u64, u64) + 'static) -> Arc<std::sync::atomic::AtomicBool> {
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let s = stop.clone();
    glib::spawn_future_local(async move {
        let r = gio::spawn_blocking(move || {
            let Some(root) = file.path() else { return (0, 0) };
            let mut bytes = 0u64;
            let mut count = 0u64;
            let mut stack = vec![root];
            while let Some(d) = stack.pop() {
                if s.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                let Ok(read) = std::fs::read_dir(&d) else { continue };
                for e in read.flatten() {
                    let Ok(m) = e.metadata() else { continue };
                    count += 1;
                    if m.is_dir() {
                        stack.push(e.path());
                    } else {
                        bytes += m.len();
                    }
                }
            }
            (bytes, count)
        })
        .await
        .unwrap_or((0, 0));
        done(r.0, r.1);
    });
    stop
}

pub fn is_trash(f: &gio::File) -> bool {
    f.uri_scheme().is_some_and(|s| s == "trash")
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("digit-ops-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn write(&self, path: &str, text: &str) -> PathBuf {
            let file = self.0.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, text).unwrap();
            file
        }

        fn names(&self, path: &str) -> Vec<String> {
            let mut names: Vec<_> = fs::read_dir(self.0.join(path)).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
            names.sort();
            names
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn state() -> State {
        Arc::new((Mutex::new(Shared::default()), Condvar::new()))
    }

    fn file(path: &std::path::Path) -> gio::File {
        gio::File::for_path(path)
    }

    fn place(policy: Answer, src: &std::path::Path, dst: &std::path::Path, moving: bool) -> Result<(), String> {
        let (st, cancel) = (state(), gio::Cancellable::new());
        let mut ctx = Ctx { st: &st, cancel: &cancel, policy: Some(policy), made: Vec::new(), moved: Vec::new(), base: 0 };
        ctx.place(&file(src), file(dst), moving)
    }

    #[test]
    fn a_skipped_file_stays_in_the_folder_being_merged() {
        let t = Scratch::new("skip");
        t.write("a/box/same.txt", "source");
        t.write("a/box/new.txt", "new");
        t.write("b/box/same.txt", "other");
        let (st, cancel) = (state(), gio::Cancellable::new());
        let mut ctx = Ctx { st: &st, cancel: &cancel, policy: Some(Answer::Skip), made: Vec::new(), moved: Vec::new(), base: 0 };
        ctx.merge(&file(&t.0.join("a/box")), &file(&t.0.join("b/box"))).unwrap();
        assert_eq!(fs::read_to_string(t.0.join("a/box/same.txt")).unwrap(), "source");
        assert_eq!(fs::read_to_string(t.0.join("b/box/same.txt")).unwrap(), "other");
        assert_eq!(t.names("b/box"), ["new.txt", "same.txt"]);
    }

    #[test]
    fn undoing_a_merge_only_touches_what_was_moved() {
        let t = Scratch::new("undo");
        t.write("a/box/new.txt", "new");
        t.write("b/box/old.txt", "old");
        let (st, cancel) = (state(), gio::Cancellable::new());
        let mut ctx = Ctx { st: &st, cancel: &cancel, policy: Some(Answer::Replace), made: Vec::new(), moved: Vec::new(), base: 0 };
        ctx.place(&file(&t.0.join("a/box")), file(&t.0.join("b/box")), true).unwrap();
        assert!(!t.0.join("a/box").exists());
        assert_eq!(ctx.moved.len(), 1);
        assert_eq!(ctx.moved[0].1.path().unwrap(), t.0.join("b/box/new.txt"));
    }

    #[test]
    fn a_file_cant_replace_the_folder_it_is_in() {
        let t = Scratch::new("parent");
        let src = t.write("b/b", "precious");
        assert!(place(Answer::Replace, &src, &t.0.join("b"), true).is_err());
        assert!(place(Answer::Replace, &src, &t.0.join("b"), false).is_err());
        assert_eq!(fs::read_to_string(src).unwrap(), "precious");
    }

    #[test]
    fn a_link_cant_replace_its_own_target() {
        let t = Scratch::new("link");
        let target = t.write("b/file", "precious");
        fs::create_dir(t.0.join("a")).unwrap();
        std::os::unix::fs::symlink(&target, t.0.join("a/file")).unwrap();
        assert!(place(Answer::Replace, &t.0.join("a/file"), &target, false).is_err());
        assert_eq!(fs::read_to_string(target).unwrap(), "precious");
    }

    #[test]
    fn a_failed_replace_puts_the_old_item_back() {
        let t = Scratch::new("restore");
        let dst = t.write("b/file", "old");
        assert!(place(Answer::Replace, &t.0.join("missing"), &dst, true).is_err());
        assert_eq!(fs::read_to_string(&dst).unwrap(), "old");
        assert_eq!(t.names("b"), ["file"]);
    }

    #[test]
    fn replacing_leaves_only_the_new_item() {
        let t = Scratch::new("replace");
        let src = t.write("a/file", "new");
        let dst = t.write("b/file", "old");
        place(Answer::Replace, &src, &dst, true).unwrap();
        assert_eq!(fs::read_to_string(&dst).unwrap(), "new");
        assert_eq!(t.names("b"), ["file"]);
        assert!(!src.exists());
    }

    #[test]
    fn a_dangling_link_counts_as_taken() {
        let t = Scratch::new("dangling");
        std::os::unix::fs::symlink(t.0.join("gone"), t.0.join("name")).unwrap();
        assert_eq!(unique(&file(&t.0), "name").path().unwrap(), t.0.join("name (2)"));
    }

    #[test]
    fn undo_keeps_only_the_outermost_items() {
        let made: Vec<_> = ["/x/a", "/x/a/1", "/x/a/1/2", "/x/b", "/x/c/1"].map(gio::File::for_path).to_vec();
        let tops: Vec<_> = tops_of(&made).iter().map(|f| f.path().unwrap()).collect();
        assert_eq!(tops, ["/x/a", "/x/b", "/x/c/1"].map(PathBuf::from));
    }

    #[test]
    fn a_noisy_tool_finishes_and_reports_its_last_line() {
        let st = state();
        let script = "yes noise | head -c 300000 >&2; echo broken >&2; exit 3";
        let plan = Plan::Run(["sh", "-c", script].map(String::from).to_vec(), std::env::temp_dir(), None);
        let (done, finished) = std::sync::mpsc::channel();
        let job = st.clone();
        std::thread::spawn(move || {
            work(plan, &job, &gio::Cancellable::new());
            let _ = done.send(());
        });
        finished.recv_timeout(Duration::from_secs(10)).expect("the tool hung on its own output");
        assert_eq!(st.0.lock().unwrap().errors, ["broken"]);
    }
}
