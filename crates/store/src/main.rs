mod appimage;
mod appstream;
mod arch;
mod aur;
mod catalog;
mod confirm;
mod flatpak;
mod native;
mod pages;
mod sources;
mod xml;

use catalog::Catalog;
use gtk::{gio, glib, prelude::*};
use kit::ui;
use serde::Deserialize;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

const APP_ID: &str = "dev.digit.Store";

#[derive(Deserialize, Clone)]
pub struct Feature {
    pub id: String,
    pub line: String,
}

#[derive(Deserialize, Clone)]
pub struct Job {
    pub name: String,
    pub glyph: String,
    pub hue: String,
    pub apps: Vec<String>,
    pub advice: String,
    pub words: Vec<String>,
}

#[derive(Deserialize, Clone)]
pub struct Editorial {
    pub feature: Feature,
    pub picks: Vec<String>,
    pub jobs: Vec<Job>,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Page {
    Loading,
    Discover,
    Yours,
    Sources,
    Job(usize),
    Search(String),
    App(String),
    Problem(String),
}

pub struct Store {
    pub window: gtk::ApplicationWindow,
    stack: gtk::Stack,
    back: gtk::Button,
    tabs: [gtk::Button; 3],
    search: gtk::SearchEntry,
    /// Stands in for the search field while the window is too narrow for it.
    search_button: gtk::Button,
    /// The folded search field was opened with its button and is still being used.
    search_open: Cell<bool>,
    bar: gtk::Revealer,
    bar_label: gtk::Label,
    bar_progress: gtk::ProgressBar,
    /// The note or system-package line under the Flatpak jobs.
    bar_line: gtk::Box,
    /// One row per Flatpak job, running or waiting.
    task_rows: gtk::Box,
    /// Flatpak jobs in the order they were asked for; the first `PARALLEL` waiting ones run.
    tasks: RefCell<Vec<Rc<Task>>>,
    pub catalog: RefCell<Option<Rc<Catalog>>>,
    /// Arch repository packages, when this is an Arch system.
    pub arch: RefCell<Option<Rc<arch::Index>>>,
    pub arch_installed: RefCell<HashMap<String, String>>,
    pub distro: sources::Distro,
    pub native_metadata: RefCell<HashMap<String, appstream::Component>>,
    /// Search shows every package rather than favouring apps and tools.
    pub all_packages: Cell<bool>,
    pub installed: RefCell<HashMap<String, flatpak::Installed>>,
    pub updates: RefCell<HashSet<String>>,
    pub lists: RefCell<HashMap<String, Vec<String>>>,
    pub editorial: Editorial,
    history: RefCell<Vec<(Page, f64)>>,
    current: RefCell<Page>,
    pub busy: RefCell<Option<String>>,
    pub chosen: RefCell<HashMap<String, (String, bool)>>,
    serial: Cell<u64>,
    pending: RefCell<Option<Page>>,
    /// What the pages shown change with the window's width; each says whether it is still there.
    fits: RefCell<Vec<Fit>>,
}

/// Follows the window's width; false once what it changed is gone.
type Fit = Box<dyn Fn(f64) -> bool>;

/// How many Flatpak jobs run at once. More wait their turn; flatpak itself takes the turns when
/// they reach the same installation, so downloads overlap while installing stays orderly.
const PARALLEL: usize = 2;

/// A Flatpak job: waiting for its turn, or running.
pub struct Task {
    pub id: String,
    name: String,
    verb: &'static str,
    job: RefCell<Option<flatpak::Job>>,
    running: Cell<bool>,
    cancel: gio::Cancellable,
    progress: gtk::ProgressBar,
}

/// Below this width the search field folds into a button, unless it is in use.
const SEARCH_FOLD: f64 = 720.0;

thread_local! {
    static STORE: RefCell<Option<Rc<Store>>> = const { RefCell::new(None) };
}

pub fn store() -> Option<Rc<Store>> {
    STORE.with(|s| s.borrow().clone())
}

impl Store {
    /// The window's width, as it is or as it is about to be.
    pub fn width(&self) -> f64 {
        // The surface is the window as the compositor sized it, tiled or not.
        let w = self.window.surface().map(|s| s.width()).filter(|w| *w > 1).unwrap_or_else(|| self.window.default_size().0);
        if w > 1 { w as f64 } else { 1240.0 }
    }

    /// Runs `f` with the window's width now and whenever it changes, while `owner` lives.
    pub fn on_width<W: IsA<gtk::Widget>>(&self, owner: &W, f: impl Fn(&W, f64) + 'static) {
        f(owner, self.width());
        let weak = owner.downgrade();
        self.fits.borrow_mut().push(Box::new(move |w| match weak.upgrade() {
            Some(o) => {
                f(&o, w);
                true
            }
            None => false,
        }));
    }

    fn fit(&self) {
        let w = self.width();
        // In use, the search field stays; on a narrow window it then takes the tabs' room.
        let focused = self.search.has_focus() || self.search.focus_child().is_some();
        if !focused && self.search.text().is_empty() {
            self.search_open.set(false);
        }
        let used = !self.search.text().is_empty() || self.search_open.get();
        let narrow = w < SEARCH_FOLD;
        self.search.set_visible(!narrow || used);
        self.search_button.set_visible(narrow && !used);
        for t in &self.tabs {
            t.set_visible(!(narrow && used));
        }
        let hooks = std::mem::take(&mut *self.fits.borrow_mut());
        let kept: Vec<_> = hooks.into_iter().filter(|f| f(w)).collect();
        let mut fits = self.fits.borrow_mut();
        let added = std::mem::take(&mut *fits);
        *fits = kept;
        fits.extend(added);
    }

    /// Shows the search field, unfolding it on a narrow window, and puts the caret in it.
    fn open_search(&self) {
        self.search_open.set(true);
        self.search.set_visible(true);
        self.search.grab_focus();
        self.fit();
    }

    fn fit_later(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(s) = weak.upgrade() {
                s.fit();
            }
        });
    }

    fn can_search(&self) -> bool {
        self.catalog.borrow().is_some() || self.distro != sources::Distro::None
    }

    pub fn open(self: &Rc<Self>, page: Page) {
        let from = self.current.replace(page.clone());
        let replace_search = matches!((&from, &page), (Page::Search(_), Page::Search(_)));
        if from != page && !replace_search && !matches!(from, Page::Loading | Page::Problem(_)) {
            let y = self.offset();
            let mut h = self.history.borrow_mut();
            h.push((from, y));
            if h.len() > 32 {
                h.remove(0);
            }
        }
        self.show(&page, 0.0);
    }

    fn offset(&self) -> f64 {
        self.stack.visible_child().and_downcast::<gtk::ScrolledWindow>().map(|s| s.vadjustment().value()).unwrap_or(0.0)
    }

    fn show(self: &Rc<Self>, page: &Page, y: f64) {
        let n = self.serial.get() + 1;
        self.serial.set(n);
        let name = format!("p{n}");
        let widget = pages::build(self, page);
        self.stack.add_named(&widget, Some(&name));
        if y > 0.0 {
            if let Some(sw) = widget.downcast_ref::<gtk::ScrolledWindow>() {
                let tries = Cell::new(0);
                sw.add_tick_callback(move |sw, _| {
                    let adj = sw.vadjustment();
                    tries.set(tries.get() + 1);
                    if adj.upper() - adj.page_size() >= y || tries.get() > 40 {
                        adj.set_value(y);
                        return glib::ControlFlow::Break;
                    }
                    glib::ControlFlow::Continue
                });
            }
        }
        self.stack.set_visible_child_name(&name);
        let stack = self.stack.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(320), move || {
            let shown = stack.visible_child();
            let mut child = stack.first_child();
            while let Some(c) = child {
                child = c.next_sibling();
                let older = stack.page(&c).name().and_then(|x| x.trim_start_matches('p').parse::<u64>().ok()).is_some_and(|k| k < n);
                if older && shown.as_ref() != Some(&c) {
                    stack.remove(&c);
                }
            }
            glib::timeout_add_local_once(std::time::Duration::from_millis(400), || unsafe {
                libc::malloc_trim(0);
            });
        });
        self.back.set_sensitive(!self.history.borrow().is_empty());
        for (i, t) in self.tabs.iter().enumerate() {
            let on = matches!((i, page), (0, Page::Discover) | (1, Page::Yours) | (2, Page::Sources));
            if on {
                t.add_css_class("on");
            } else {
                t.remove_css_class("on");
            }
        }
        if !matches!(page, Page::Search(_)) && !self.search.text().is_empty() {
            self.search.set_text("");
        }
    }

    pub fn back(self: &Rc<Self>) {
        let prev = self.history.borrow_mut().pop();
        if let Some((p, y)) = prev {
            *self.current.borrow_mut() = p.clone();
            self.show(&p, y);
        }
    }

    pub fn refresh(self: &Rc<Self>) {
        let page = self.current.borrow().clone();
        let y = self.offset();
        self.show(&page, y);
    }

    pub fn action(&self, id: &str) -> (&'static str, &'static str) {
        if self.task(id).is_some() {
            return ("Cancel", "");
        }
        match (self.installed.borrow().contains_key(id), self.updates.borrow().contains(id)) {
            (true, true) => ("Update", "accent"),
            (true, false) => ("Open", ""),
            _ => ("Get", "accent"),
        }
    }

    pub fn press(self: &Rc<Self>, id: &str) {
        let (label, _) = self.action(id);
        match label {
            "Cancel" => self.cancel(id),
            "Open" => launch(id),
            "Update" => {
                let system = self.installed.borrow().get(id).is_some_and(|i| i.system);
                self.work(id, flatpak::Job::Update(system), "Updating");
            }
            "Get" => {
                let (remote, system) = self.chosen.borrow().get(id).cloned().unwrap_or_else(|| ("flathub".into(), sources::scope_system()));
                self.work(id, flatpak::Job::Install(system, remote), "Installing");
            }
            _ => {}
        }
    }

    pub fn remove(self: &Rc<Self>, id: &str) {
        let system = self.installed.borrow().get(id).is_some_and(|i| i.system);
        self.work(id, flatpak::Job::Remove(system), "Removing");
    }

    pub fn task(&self, id: &str) -> Option<Rc<Task>> {
        self.tasks.borrow().iter().find(|t| t.id == id).cloned()
    }

    /// Queues a Flatpak job; it starts as soon as fewer than `PARALLEL` are running.
    fn work(self: &Rc<Self>, id: &str, job: flatpak::Job, verb: &'static str) {
        if self.task(id).is_some() {
            return;
        }
        let name = self.catalog.borrow().as_ref().and_then(|c| c.get(id).map(|a| a.name.clone())).unwrap_or_else(|| id.to_string());
        let task = Rc::new(Task { id: id.to_string(), name, verb, job: RefCell::new(Some(job)), running: Cell::new(false), cancel: gio::Cancellable::new(), progress: gtk::ProgressBar::new() });
        self.tasks.borrow_mut().push(task);
        self.pump();
        self.show_tasks();
        self.refresh();
    }

    /// Stops a running job, or takes a waiting one off the queue.
    pub fn cancel(self: &Rc<Self>, id: &str) {
        let Some(task) = self.task(id) else { return };
        if task.running.get() {
            task.cancel.cancel();
        } else {
            self.tasks.borrow_mut().retain(|t| t.id != id);
            self.show_tasks();
            self.refresh();
        }
    }

    /// Starts waiting jobs while there is room.
    fn pump(self: &Rc<Self>) {
        loop {
            let next = {
                let tasks = self.tasks.borrow();
                if tasks.iter().filter(|t| t.running.get()).count() >= PARALLEL {
                    return;
                }
                tasks.iter().find(|t| !t.running.get()).cloned()
            };
            let Some(task) = next else { return };
            self.start(task);
        }
    }

    fn start(self: &Rc<Self>, task: Rc<Task>) {
        let Some(job) = task.job.borrow_mut().take() else { return };
        task.running.set(true);
        task.progress.set_fraction(0.0);
        let me = self.clone();
        glib::spawn_future_local(async move {
            let bar = task.progress.clone();
            let pulse = glib::timeout_add_local(std::time::Duration::from_millis(400), {
                let bar = bar.clone();
                move || {
                    if bar.fraction() < 0.001 {
                        bar.pulse();
                    }
                    glib::ControlFlow::Continue
                }
            });
            let result = flatpak::run(&task.id, job, move |p| {
                if let Some(f) = p {
                    bar.set_fraction(f.max(0.01));
                }
            }, &task.cancel)
            .await;
            pulse.remove();
            me.tasks.borrow_mut().retain(|t| !Rc::ptr_eq(t, &task));
            me.pump();
            *me.installed.borrow_mut() = flatpak::installed().await;
            me.updates.borrow_mut().remove(&task.id);
            match result {
                Ok(()) => me.note(&format!("{}: done", task.name), false),
                Err(e) if e == flatpak::CANCELLED => me.note(&format!("{}: cancelled", task.name), false),
                Err(e) => me.note(&format!("{}: {e}", task.name), true),
            }
            me.show_tasks();
            me.refresh();
        });
    }

    /// Rebuilds the job rows and shows or hides the activity panel.
    fn show_tasks(&self) {
        while let Some(c) = self.task_rows.first_child() {
            self.task_rows.remove(&c);
        }
        for task in self.tasks.borrow().iter() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            let col = gtk::Box::new(gtk::Orientation::Vertical, 6);
            col.set_hexpand(true);
            col.set_valign(gtk::Align::Center);
            let label = ui::text(&if task.running.get() { format!("{} {}", task.verb, task.name) } else { format!("Waiting · {}", task.name) }, "name");
            label.set_xalign(0.0);
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            col.append(&label);
            if let Some(parent) = task.progress.parent().and_downcast::<gtk::Box>() {
                parent.remove(&task.progress);
            }
            task.progress.set_visible(task.running.get());
            col.append(&task.progress);
            row.append(&col);
            let stop = kit::app::control("close", "close");
            stop.set_tooltip_text(Some(if task.running.get() { "Cancel" } else { "Take off the queue" }));
            let id = task.id.clone();
            stop.connect_clicked(move |_| {
                if let Some(s) = store() {
                    s.cancel(&id);
                }
            });
            row.append(&stop);
            self.task_rows.append(&row);
        }
        self.task_rows.set_visible(!self.tasks.borrow().is_empty());
        self.update_bar();
    }

    /// The panel shows while there are jobs or a line to read.
    fn update_bar(&self) {
        self.bar.set_reveal_child(!self.tasks.borrow().is_empty() || self.bar_line.is_visible());
    }

    /// Installs or removes an Arch package through pkexec, so polkit asks for the password.
    pub fn pacman(self: &Rc<Self>, name: &str) {
        if self.busy.borrow().is_some() {
            self.note("Another app is still being installed", true);
            return;
        }
        if name.starts_with('-') || name.is_empty() {
            self.note("Invalid package name", true);
            return;
        }
        let remove = self.arch_installed.borrow().contains_key(name);
        if remove && kit::guard::protected(name) {
            self.note("Protected system package", true);
            return;
        }
        let key = format!("arch:{name}");
        *self.busy.borrow_mut() = Some(key);
        self.bar_label.set_text(&format!("{} {name}", if remove { "Removing" } else { "Installing" }));
        self.bar_label.remove_css_class("warn");
        self.bar_progress.set_visible(true);
        self.bar_progress.set_fraction(0.0);
        self.bar_line.set_visible(true);
        self.update_bar();
        self.refresh();
        let me = self.clone();
        let name = name.to_string();
        glib::spawn_future_local(async move {
            let bar = me.bar_progress.clone();
            let pulse = glib::timeout_add_local(std::time::Duration::from_millis(300), move || {
                bar.pulse();
                glib::ControlFlow::Continue
            });
            let args: Vec<&std::ffi::OsStr> = if remove {
                vec!["pkexec".as_ref(), "pacman".as_ref(), "-R".as_ref(), "--noconfirm".as_ref(), "--".as_ref(), name.as_ref()]
            } else {
                vec!["pkexec".as_ref(), "pacman".as_ref(), "-S".as_ref(), "--needed".as_ref(), "--noconfirm".as_ref(), "--".as_ref(), name.as_ref()]
            };
            let result = match gio::Subprocess::newv(&args, gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_PIPE) {
                Ok(p) => match p.communicate_utf8_future(None).await {
                    Ok(_) if p.is_successful() => Ok(()),
                    Ok((_, err)) => Err(match p.exit_status() {
                        126 | 127 => "Cancelled".to_string(),
                        _ => err.map(|e| e.lines().rev().map(str::trim).find(|l| l.starts_with("error:")).unwrap_or("pacman failed").to_string()).unwrap_or_else(|| "pacman failed".into()),
                    }),
                    Err(e) => Err(e.to_string()),
                },
                Err(e) => Err(e.to_string()),
            };
            pulse.remove();
            *me.busy.borrow_mut() = None;
            *me.arch_installed.borrow_mut() = gio::spawn_blocking(arch::installed).await.unwrap_or_default();
            match result {
                Ok(()) => me.note(&format!("{name}: done"), false),
                Err(e) => me.note(&format!("{name}: {e}"), true),
            }
            me.refresh();
        });
    }

    pub fn note(&self, text: &str, bad: bool) {
        self.bar_label.set_text(text);
        if bad {
            self.bar_label.add_css_class("warn");
        } else {
            self.bar_label.remove_css_class("warn");
        }
        self.bar_progress.set_visible(false);
        self.bar_line.set_visible(!text.is_empty());
        self.update_bar();
        let label = self.bar_label.clone();
        let stamp = text.to_string();
        glib::timeout_add_local_once(std::time::Duration::from_millis(if bad { 8000 } else { 3500 }), move || {
            if label.text() == stamp
                && let Some(s) = store()
                && s.busy.borrow().is_none()
            {
                s.bar_line.set_visible(false);
                s.update_bar();
            }
        });
    }
}

pub fn launch(id: &str) {
    if !flatpak::valid(id) {
        return;
    }
    match gio_unix::DesktopAppInfo::new(&format!("{id}.desktop")) {
        Some(info) => {
            let _ = info.launch(&[], gio::AppLaunchContext::NONE);
        }
        None => {
            let argv: [&std::ffi::OsStr; 3] = ["flatpak".as_ref(), "run".as_ref(), id.as_ref()];
            let _ = gio::Subprocess::newv(&argv, gio::SubprocessFlags::NONE);
        }
    }
}

fn header(window: &gtk::ApplicationWindow) -> (gtk::Widget, gtk::Button, [gtk::Button; 3], gtk::SearchEntry, gtk::Button) {
    let bar = gtk::CenterBox::new();
    bar.add_css_class("topbar");
    let left = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let back = kit::app::control("arrow_back", "");
    back.set_sensitive(false);
    let tabs = [ui::button("Discover", "tab"), ui::button("Yours", "tab"), ui::button("Sources", "tab")];
    left.append(&back);
    left.append(&tabs[0]);
    left.append(&tabs[1]);
    left.append(&tabs[2]);
    let search = gtk::SearchEntry::new();
    search.add_css_class("search");
    search.add_css_class("big");
    search.set_placeholder_text(Some("What do you want to do?"));
    // Wide when there is room, narrower rather than cut off when there isn't.
    search.set_width_chars(12);
    search.set_max_width_chars(44);
    search.set_margin_start(12);
    search.set_margin_end(12);
    let right = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let search_button = kit::app::control("search", "");
    search_button.set_tooltip_text(Some("Search"));
    search_button.set_visible(false);
    right.append(&search_button);
    right.append(&kit::app::controls(window, APP_ID));
    bar.set_start_widget(Some(&left));
    bar.set_center_widget(Some(&search));
    bar.set_end_widget(Some(&right));
    let handle = gtk::WindowHandle::new();
    handle.set_child(Some(&bar));
    (handle.upcast(), back, tabs, search, search_button)
}

fn css() -> String {
    let i = kit::style::ink();
    let (main, meta, accent) = (&i.main, &i.meta, &i.accent);
    format!(
        r#"{base}
.topbar {{ padding: 12px 14px 10px 14px; }}
.topbar .controls {{ margin: 0; }}
button.pill.tab {{ background: transparent; color: {meta}; }}
button.pill.tab.on {{ background: alpha({main}, 0.12); color: {main}; }}
entry.search.big {{ min-height: 34px; border-radius: 999px; padding: 4px 16px; font-size: 15px; background: @d_raised; }}
/* Three radii, all following Roundness: hero surfaces, cards and the rows in them, screenshots. */
.banner {{ border-radius: var(--r-hero); }}
.banner.brandless, .spot.brandless {{ background: linear-gradient(135deg, alpha({accent}, 0.45), @d_card); }}
.lifted {{ border-radius: var(--r-ctl); box-shadow: 0 14px 40px rgba(0,0,0,0.45); }}
button.dot {{ min-width: 8px; min-height: 8px; padding: 0; border-radius: 999px; background: alpha({main}, 0.25); border: none; box-shadow: none; }}
button.dot.on {{ background: {main}; min-width: 22px; }}
button.arrow {{ min-width: 38px; min-height: 38px; padding: 0; border-radius: 999px; background: rgba(0,0,0,0.42); color: #ffffff; border: none; box-shadow: none; opacity: 0; }}
.carousel:hover button.arrow {{ opacity: 1; }}
button.arrow:hover {{ background: rgba(0,0,0,0.62); }}
.light {{ color: #ffffff; }}
.lead.light, .sub.light {{ color: alpha(#ffffff, 0.82); }}
.eyebrow.light {{ color: alpha(#ffffff, 0.75); }}
.spot {{ border-radius: var(--r-hero); padding: 24px; }}
.spot .title {{ color: #ffffff; }}
/* The glyph sits as far from the pill's edge on the left as above and below, so its circle
   shares the pill's rounded end. */
button.chip {{ background: @d_field; border-radius: 999px; padding: 6px 16px 6px 6px; border: none; }}
button.chip .tile {{ border-radius: 999px; }}
.story {{ border-radius: var(--r-hero); background: @d_field; }}
.scrim {{ padding: 40px 18px 16px 18px; background: linear-gradient(rgba(0,0,0,0), rgba(0,0,0,0.88) 55%); }}
.feature {{ border-radius: var(--r-hero); padding: 30px 34px; background: linear-gradient(120deg, alpha({accent}, 0.30), @d_card 62%); }}
.eyebrow {{ color: {accent}; font-size: 12px; font-weight: 650; letter-spacing: 1px; }}
.jobtile {{ background: @d_card; border-radius: var(--r-card); padding: 16px; border: none; min-width: 168px; }}
.pick {{ background: @d_card; border-radius: var(--r-card); padding: 18px; min-width: 220px; }}
.lockup {{ padding: 10px 12px; border-radius: max(4px, calc(var(--r-card) - 6px)); }}
/* One hover for everything that opens something: the same faint ink over it, eased in and out. */
.lockup, button.chip, .jobtile {{ transition: background-color 200ms ease-out; }}
.lockup:hover {{ background-color: alpha({main}, 0.06); }}
.lockup:active {{ background-color: alpha({main}, 0.10); }}
button.chip:hover {{ background: mix(@d_field, {main}, 0.06); }}
button.chip:active {{ background: mix(@d_field, {main}, 0.10); }}
.jobtile:hover {{ background: mix(@d_card, {main}, 0.06); }}
.jobtile:active {{ background: mix(@d_card, {main}, 0.10); }}
/* Rows sit 6px inside their card whether or not a grid holds them, so the hover is concentric. */
flowbox > flowboxchild {{ padding: 0; outline: none; }}
/* The theme tints a grid cell under the pointer, in the cell's own shape, around the pill or
   card in it. What is in the cell shows the hover instead. */
flowbox > flowboxchild:hover, flowbox > flowboxchild:selected, flowbox > flowboxchild:active {{ background: none; }}
/* Stands in for a missing app icon at the icons' own size. */
.tile.app {{ min-width: 52px; min-height: 52px; font-size: 30px; border-radius: calc(var(--r-ctl) + 2px); }}
.shot {{ border-radius: var(--r-ctl); background: @d_field; }}
.about {{ font-size: 14px; color: {main}; }}
.verified {{ color: {accent}; }}
window.install-sheet {{ background: transparent; }}
"#,
        base = kit::style::css()
    )
}

fn install_files() {
    let (_, w, h) = kit::pixel::cells("s");
    let cell = 12.0;
    let (ox, oy) = ((128.0 - w as f64 * cell) / 2.0, (128.0 - h as f64 * cell) / 2.0);
    let (d, _, _) = kit::pixel::path("s", cell, ox, oy);
    let svg = format!(r##"<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128" viewBox="0 0 128 128"><defs><linearGradient id="g" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#4aa3ff"/><stop offset="1" stop-color="#0a64e0"/></linearGradient></defs><rect width="128" height="128" rx="30" fill="url(#g)"/><path d="{d}" fill="#ffffff"/></svg>"##);
    let data = glib::user_data_dir();
    // The full path, as Files' service file has: an app started by D-Bus may not have
    // ~/.local/bin on its PATH. A build run from the source tree keeps the plain name.
    let exe = std::fs::read_link("/proc/self/exe").ok().filter(|e| e.file_name().is_some_and(|n| n == "digit-store") && !e.components().any(|c| c.as_os_str() == "target"));
    let exec = exe.map_or_else(|| "digit-store".to_string(), |e| format!("\"{}\"", e.to_string_lossy()));
    let desktop = format!(
        "[Desktop Entry]\nType=Application\nName=Digit Store\nGenericName=Software Store\nComment=Find, get and update apps from Flathub\nKeywords=Software;Store;App Store;Apps;Flathub;Flatpak;Install;Uninstall;Update;Updates;Program;\nExec={exec} %U\nIcon={APP_ID}\nCategories=System;PackageManager;\nMimeType=x-scheme-handler/appstream;application/vnd.flatpak.ref;application/vnd.flatpak;application/vnd.appimage;application/x-iso9660-appimage;\nStartupNotify=true\n"
    );
    let mut icon_changed = false;
    for (path, body) in [(data.join(format!("icons/hicolor/scalable/apps/{APP_ID}.svg")), svg), (data.join(format!("applications/{APP_ID}.desktop")), desktop)] {
        if std::fs::read_to_string(&path).ok().as_deref() == Some(body.as_str()) {
            continue;
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if kit::atomic::write(&path, &body).is_ok() && path.extension().is_some_and(|e| e == "svg") {
            icon_changed = true;
        }
    }
    if icon_changed {
        let theme = data.join("icons/hicolor");
        let argv: [&std::ffi::OsStr; 5] = ["gtk4-update-icon-cache".as_ref(), "-q".as_ref(), "-t".as_ref(), "-f".as_ref(), theme.as_os_str()];
        let _ = gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE);
    }
    for mime in ["x-scheme-handler/appstream", "application/vnd.flatpak.ref", "application/vnd.flatpak", "application/vnd.appimage", "application/x-iso9660-appimage"] {
        let current = gio::AppInfo::default_for_type(mime, false).and_then(|a| a.id()).map(|s| s.to_string());
        if current.as_deref() != Some("dev.digit.Store.desktop") {
            if let Some(info) = gio_unix::DesktopAppInfo::new("dev.digit.Store.desktop") {
                let _ = info.set_as_default_for_type(mime);
            }
        }
    }
    kit::app::replace(&REPLACES, "dev.digit.Store.desktop", "store-replaced");
}

const REPLACES: [&str; 2] = ["org.gnome.Software.desktop", "org.kde.discover.desktop"];

fn target(arg: &str) -> Page {
    if let Some(rest) = arg.strip_prefix("appstream://").or_else(|| arg.strip_prefix("appstream:")) {
        let id = rest.trim_matches('/').trim_end_matches(".desktop");
        return if flatpak::valid(id) { Page::App(id.to_string()) } else { Page::Problem("That link doesn't point to an app".into()) };
    }
    if arg.ends_with(".flatpakref") {
        let path = if let Some(p) = arg.strip_prefix("file://") { p.to_string() } else { arg.to_string() };
        let text = std::fs::metadata(&path).ok().filter(|m| m.len() < 65536).and_then(|_| std::fs::read_to_string(&path).ok()).unwrap_or_default();
        let field = |k: &str| text.lines().find_map(|l| l.strip_prefix(&format!("{k}="))).map(str::trim).unwrap_or("");
        let (name, url) = (field("Name"), field("Url"));
        if !url.starts_with("https://dl.flathub.org/repo") {
            return Page::Problem("This file installs from a source other than Flathub, so Digit Store won't open it.".into());
        }
        return if flatpak::valid(name) { Page::App(name.to_string()) } else { Page::Problem("The file doesn't name a valid app".into()) };
    }
    if let Some(q) = arg.strip_prefix("search:") {
        let q: String = q.chars().filter(|c| !c.is_control()).take(80).collect();
        return if q.trim().is_empty() { Page::Discover } else { Page::Search(q.trim().to_string()) };
    }
    match arg {
        "yours" => Page::Yours,
        "sources" => Page::Sources,
        _ if flatpak::valid(arg) => Page::App(arg.to_string()),
        _ => Page::Discover,
    }
}

fn build(app: &gtk::Application) -> Rc<Store> {
    let window = gtk::ApplicationWindow::builder().application(app).title("Store").default_width(1240).default_height(820).build();
    kit::scroll::smooth(&window);
    window.set_decorated(false);
    window.add_css_class("digit-settings");
    let (head, back, tabs, search, search_button) = header(&window);
    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_transition_duration(160);
    stack.set_vexpand(true);
    let bar = gtk::Revealer::new();
    bar.set_transition_type(gtk::RevealerTransitionType::SlideUp);
    let inner = gtk::Box::new(gtk::Orientation::Vertical, 12);
    inner.add_css_class("activity");
    let task_rows = gtk::Box::new(gtk::Orientation::Vertical, 12);
    task_rows.set_visible(false);
    let bar_line = gtk::Box::new(gtk::Orientation::Vertical, 8);
    bar_line.set_visible(false);
    let bar_label = ui::text("", "name");
    bar_label.set_xalign(0.0);
    bar_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let bar_progress = gtk::ProgressBar::new();
    bar_line.append(&bar_label);
    bar_line.append(&bar_progress);
    inner.append(&task_rows);
    inner.append(&bar_line);
    bar.set_child(Some(&inner));
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&head);
    root.append(&stack);
    root.append(&bar);
    window.set_child(Some(&root));
    let editorial: Editorial = serde_json::from_str(include_str!("../../../data/store/editorial.json")).expect("editorial");
    let me = Rc::new(Store {
        window,
        stack,
        back: back.clone(),
        tabs: tabs.clone(),
        search: search.clone(),
        search_button: search_button.clone(),
        search_open: Cell::new(false),
        bar,
        bar_label,
        bar_progress,
        bar_line,
        task_rows,
        tasks: RefCell::new(Vec::new()),
        catalog: RefCell::new(None),
        arch: RefCell::new(None),
        arch_installed: RefCell::new(HashMap::new()),
        distro: sources::distro(),
        native_metadata: RefCell::new(HashMap::new()),
        all_packages: Cell::new(false),
        installed: RefCell::new(HashMap::new()),
        updates: RefCell::new(HashSet::new()),
        lists: RefCell::new(HashMap::new()),
        editorial,
        history: RefCell::new(Vec::new()),
        current: RefCell::new(Page::Loading),
        busy: RefCell::new(None),
        chosen: RefCell::new(HashMap::new()),
        serial: Cell::new(0),
        pending: RefCell::new(None),
        fits: RefCell::new(Vec::new()),
    });
    search_button.connect_clicked(|_| {
        if let Some(s) = store() {
            s.open_search();
        }
    });
    let focus = gtk::EventControllerFocus::new();
    focus.connect_leave(|_| {
        if let Some(s) = store() {
            s.fit_later();
        }
    });
    search.add_controller(focus);
    me.window.connect_default_width_notify(|_| {
        if let Some(s) = store() {
            s.fit_later();
        }
    });
    me.window.connect_realize(|w| {
        if let Some(surface) = w.surface() {
            surface.connect_width_notify(|_| {
                if let Some(s) = store() {
                    s.fit_later();
                }
            });
        }
        if let Some(s) = store() {
            s.fit_later();
        }
    });

    back.connect_clicked(|_| {
        if let Some(s) = store() {
            s.back();
        }
    });
    tabs[0].connect_clicked(|_| {
        if let Some(s) = store() {
            s.open(Page::Discover);
        }
    });
    tabs[1].connect_clicked(|_| {
        if let Some(s) = store() {
            s.open(Page::Yours);
        }
    });
    tabs[2].connect_clicked(|_| {
        if let Some(s) = store() {
            s.open(Page::Sources);
        }
    });
    search.set_search_delay(300);
    search.connect_search_changed(|e| {
        let Some(s) = store() else { return };
        let q = e.text().to_string();
        if q.trim().is_empty() {
            if matches!(*s.current.borrow(), Page::Search(_)) {
                s.back();
            }
        } else if s.can_search() && *s.current.borrow() != Page::Search(q.clone()) {
            s.open(Page::Search(q));
        }
    });
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(|_, key, _, mods| {
        let Some(s) = store() else { return glib::Propagation::Proceed };
        let ctrl = mods.contains(gtk::gdk::ModifierType::CONTROL_MASK);
        match key {
            gtk::gdk::Key::f if ctrl => {
                s.open_search();
            }
            gtk::gdk::Key::w | gtk::gdk::Key::q if ctrl => s.window.close(),
            gtk::gdk::Key::Escape if !s.history.borrow().is_empty() => s.back(),
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    me.window.add_controller(keys);
    STORE.with(|s| *s.borrow_mut() = Some(me.clone()));
    me.show(&Page::Loading, 0.0);
    load(&me);
    me
}

fn load(store: &Rc<Store>) {
    load_with(store, true);
    match store.distro {
        sources::Distro::Arch => {
            watch_arch_installed(store);
            load_arch(store);
        }
        sources::Distro::PackageKit => {
            let me = store.clone();
            glib::spawn_future_local(async move {
                let data = gio::spawn_blocking(appstream::load).await.unwrap_or_default();
                if !data.is_empty() {
                    *me.native_metadata.borrow_mut() = data;
                    if matches!(*me.current.borrow(), Page::Search(_)) {
                        me.refresh();
                    }
                }
            });
        }
        sources::Distro::None => {}
    }
}

/// Builds the Arch package index in the background; search picks it up once it is there.
fn load_arch(store: &Rc<Store>) {
    if !arch::available() {
        return;
    }
    let me = store.clone();
    glib::spawn_future_local(async move {
        let (index, installed) = gio::spawn_blocking(|| (arch::load(), arch::installed())).await.unwrap_or((None, HashMap::new()));
        let Some(index) = index else { return };
        *me.arch.borrow_mut() = Some(Rc::new(index));
        *me.arch_installed.borrow_mut() = installed;
        if matches!(*me.current.borrow(), Page::Search(_) | Page::App(_) | Page::Yours) {
            me.refresh();
        }
    });
}

/// Pacman drops its database lock after writing the local database, including helper installs.
fn watch_arch_installed(store: &Rc<Store>) {
    let Some(dir) = std::path::Path::new(arch::LOCAL).parent() else { return };
    let lock = dir.join("db.lck");
    let Ok(monitor) = gio::File::for_path(dir).monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE) else { return };
    let weak = Rc::downgrade(store);
    monitor.connect_changed(move |_, file, other, _| {
        let hit = |f: &gio::File| f.basename().is_some_and(|b| b.as_os_str() == "db.lck");
        if !(hit(file) || other.is_some_and(hit)) || lock.exists() {
            return;
        }
        let weak = weak.clone();
        glib::spawn_future_local(async move {
            let Ok(installed) = gio::spawn_blocking(arch::installed).await else { return };
            let Some(s) = weak.upgrade() else { return };
            if *s.arch_installed.borrow() != installed {
                *s.arch_installed.borrow_mut() = installed;
                if matches!(*s.current.borrow(), Page::Search(_) | Page::App(_) | Page::Yours) {
                    s.refresh();
                }
            }
        });
    });
    kit::tag::set(&store.window, "digit-pacman-watch", monitor);
}

async fn set_up_flathub() {
    let add: Vec<String> = ["remote-add", "--user", "--if-not-exists", "flathub", "https://dl.flathub.org/repo/flathub.flatpakrepo"].iter().map(|s| s.to_string()).collect();
    let _ = flatpak::command(&add).await;
    let fetch: Vec<String> = ["update", "--user", "--appstream", "--noninteractive", "flathub"].iter().map(|s| s.to_string()).collect();
    let _ = flatpak::command(&fetch).await;
}

fn load_with(store: &Rc<Store>, retry: bool) {
    let me = store.clone();
    glib::spawn_future_local(async move {
        let (catalog, installed, updates) = futures(&me).await;
        if catalog.is_none() && retry && glib::find_program_in_path("flatpak").is_some() {
            me.note("Setting up Flathub for the first time…", false);
            set_up_flathub().await;
            me.note("", false);
            load_with(&me, false);
            return;
        }
        let waiting = matches!(*me.current.borrow(), Page::Loading | Page::Problem(_));
        *me.installed.borrow_mut() = installed;
        *me.updates.borrow_mut() = updates;
        match catalog {
            Some(c) => {
                *me.catalog.borrow_mut() = Some(Rc::new(c));
                collections(&me);
                if waiting {
                    let page = me.pending.borrow_mut().take().unwrap_or(Page::Discover);
                    me.open(page);
                } else {
                    me.refresh();
                }
            }
            None if me.distro != sources::Distro::None => {
                if waiting {
                    let page = me.pending.borrow_mut().take().unwrap_or(Page::Discover);
                    me.open(page);
                } else if matches!(*me.current.borrow(), Page::Yours) {
                    me.refresh();
                }
            }
            None if glib::find_program_in_path("flatpak").is_none() => me.open(Page::Problem("Digit Store gets apps from Flathub and needs Flatpak. Install the “flatpak” package with your package manager, then open the store again.".into())),
            None => me.open(Page::Problem("Flathub's app catalogue could not be downloaded. Check your internet connection, run “flatpak update --appstream” once, then open the store again.".into())),
        }
        glib::timeout_add_local_once(std::time::Duration::from_secs(2), || unsafe {
            libc::malloc_trim(0);
        });
    });
}

async fn futures(_: &Rc<Store>) -> (Option<Catalog>, HashMap<String, flatpak::Installed>, HashSet<String>) {
    let catalog = gio::spawn_blocking(catalog::load).await.ok().flatten();
    let installed = flatpak::installed().await;
    let updates = flatpak::updates().await;
    (catalog, installed, updates)
}

thread_local! {
    static REFRESH: Cell<bool> = const { Cell::new(false) };
}

pub const LISTS: [&str; 9] = ["trending", "popular", "recently-added", "recently-updated", "category/Game", "category/Graphics", "category/AudioVideo", "category/Office", "category/Development"];

fn collections(store: &Rc<Store>) {
    let dir = glib::user_cache_dir().join("digit/store");
    let parse = |text: &str| -> Vec<String> {
        serde_json::from_str::<serde_json::Value>(text)
            .ok()
            .and_then(|v| v.get("hits")?.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|h| h.get("app_id")?.as_str().map(str::to_string))
            .filter(|id| flatpak::valid(id))
            .collect()
    };
    for name in LISTS {
        let file = dir.join(format!("{}.json", name.replace('/', "-")));
        let fresh = std::fs::metadata(&file).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|e| e.as_secs() < 86_400);
        if let Ok(text) = std::fs::read_to_string(&file) {
            store.lists.borrow_mut().insert(name.to_string(), parse(&text));
        }
        if fresh {
            continue;
        }
        let me = store.clone();
        glib::spawn_future_local(async move {
            let url = format!("https://flathub.org/api/v2/collection/{name}?page=1&per_page=40");
            let argv: [&std::ffi::OsStr; 5] = ["curl".as_ref(), "-fsSL".as_ref(), "--max-time".as_ref(), "15".as_ref(), url.as_ref()];
            let Ok(p) = gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_SILENCE) else { return };
            let Ok((Some(out), _)) = p.communicate_utf8_future(None).await else { return };
            let list = parse(&out);
            if list.is_empty() {
                return;
            }
            let _ = kit::atomic::write(&file, out.as_str());
            me.lists.borrow_mut().insert(name.to_string(), list);
            if !REFRESH.with(|r| r.replace(true)) {
                let me = me.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(900), move || {
                    REFRESH.with(|r| r.set(false));
                    if matches!(*me.current.borrow(), Page::Discover) {
                        me.refresh();
                    }
                });
            }
        });
    }
}

kit::layer_shell_first!();

fn main() -> glib::ExitCode {
    if std::env::args().nth(1).as_deref() == Some("--register") {
        install_files();
        return glib::ExitCode::SUCCESS;
    }
    kit::effects::select("DIGIT_STORE_RENDERER");
    unsafe {
        libc::mallopt(libc::M_MMAP_THRESHOLD, 256 * 1024);
        libc::mallopt(libc::M_ARENA_MAX, 2);
    }
    let app = gtk::Application::new(Some(APP_ID), gio::ApplicationFlags::HANDLES_COMMAND_LINE | gio::ApplicationFlags::HANDLES_OPEN);
    app.connect_startup(|_| {
        gtk::Window::set_default_icon_name(APP_ID);
        install_files();
        if let Some(display) = gtk::gdk::Display::default() {
            kit::style::live(&display, css);
        }
    });
    app.connect_command_line(|app, cmd| {
        let arg = cmd.arguments().get(1).and_then(|a| a.to_str().map(str::to_string));
        // A file to install opens the install sheet over the screen, not the Store window.
        if let Some(path) = arg.as_deref().and_then(confirm::file) {
            confirm::show(app, path);
            return glib::ExitCode::SUCCESS;
        }
        let page = arg.as_deref().map(target).unwrap_or(Page::Discover);
        let s = store().unwrap_or_else(|| build(app));
        if let Page::Search(q) = &page {
            s.search.set_text(q);
        }
        if s.catalog.borrow().is_some() || matches!(page, Page::Problem(_)) || matches!(page, Page::Search(_) | Page::Yours | Page::Sources) && s.can_search() {
            s.open(page);
        } else {
            *s.pending.borrow_mut() = Some(page);
        }
        s.window.present();
        glib::ExitCode::SUCCESS
    });
    app.run()
}
