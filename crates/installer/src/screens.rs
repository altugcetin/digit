use crate::{
    check::{self, Check},
    run::{self, Event},
};
use gtk::{gio, glib, prelude::*};
use kit::ui;
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};

type Wizard = Rc<Installer>;

const HELLO: usize = 0;
const CHECK: usize = 1;
const COMPOSITOR: usize = 2;
const METHOD: usize = 3;
const READY: usize = 4;
const INSTALL: usize = 5;
const DONE: usize = 6;


struct Installer {
    window: gtk::ApplicationWindow,
    stack: gtk::Stack,
    back: gtk::Button,
    next: gtk::Button,
    dots: gtk::Box,
    page: Cell<usize>,
    root: Result<PathBuf, String>,
    check: RefCell<Option<Check>>,
    wm: RefCell<String>,
    nvidia_env: Cell<bool>,
    busy: Cell<bool>,
    process: RefCell<Option<gio::Subprocess>>,
}

fn wm_name(wm: &str) -> &str {
    if wm == "niri" { "niri" } else { "Hyprland" }
}

fn lead(text: &str) -> gtk::Label {
    let l = ui::text(text, "lead");
    l.set_xalign(0.5);
    l.set_justify(gtk::Justification::Center);
    l.set_wrap(true);
    l.set_max_width_chars(54);
    l.set_halign(gtk::Align::Center);
    l
}

/// The top of every step: a colored icon, a title and one sentence.
fn header(body: &gtk::Box, icon: &str, title: &str, text: &str) -> gtk::Label {
    let tile = ui::tile(icon);
    tile.add_css_class("big");
    tile.set_halign(gtk::Align::Center);
    tile.set_margin_bottom(6);
    body.append(&tile);
    let t = ui::text(title, "welcome-title");
    t.set_xalign(0.5);
    body.append(&t);
    let l = lead(text);
    body.append(&l);
    l
}

/// Big picture cards in a row; one is always picked.
fn picks(items: Vec<(&'static str, gtk::Widget, &str, &str)>, current: &str, picked: impl Fn(&'static str) + 'static) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    row.set_halign(gtk::Align::Center);
    let picked = Rc::new(picked);
    let mut first: Option<gtk::ToggleButton> = None;
    for (key, picture, name, sub) in items {
        let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
        picture.set_overflow(gtk::Overflow::Hidden);
        card.append(&picture);
        let n = ui::text(name, "name");
        n.set_xalign(0.5);
        card.append(&n);
        if !sub.is_empty() {
            let s = ui::text(sub, "sub");
            s.set_xalign(0.5);
            card.append(&s);
        }
        let b = gtk::ToggleButton::new();
        b.add_css_class("style-card");
        b.set_child(Some(&card));
        match &first {
            Some(f) => b.set_group(Some(f)),
            None => first = Some(b.clone()),
        }
        b.set_active(current == key);
        let picked = picked.clone();
        b.connect_toggled(move |b| {
            if b.is_active() {
                picked(key);
            }
        });
        row.append(&b);
    }
    row
}

fn state_icon(ok: bool) -> gtk::Label {
    let s = ui::sym(if ok { "check_circle" } else { "error" });
    s.add_css_class(if ok { "state-ok" } else { "state-bad" });
    s
}

fn page_frame() -> (gtk::ScrolledWindow, gtk::Box) {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 14);
    body.add_css_class("setup-page");
    body.set_valign(gtk::Align::Center);
    body.set_halign(gtk::Align::Center);
    body.set_size_request(600, -1);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
    scroll.set_vexpand(true);
    scroll.set_child(Some(&body));
    (scroll, body)
}

impl Installer {
    fn show(self: &Wizard, page: usize) {
        self.page.set(page);
        self.back.set_visible((CHECK..=READY).contains(&page));
        self.next.set_visible(page != INSTALL);
        self.next.set_sensitive(true);
        self.next.set_label(match page {
            HELLO => "Get started",
            READY => "Install",
            DONE => "Close",
            _ => "Continue",
        });
        if page == DONE { self.next.remove_css_class("accent") } else { self.next.add_css_class("accent") }
        self.dots.set_visible(page <= READY);
        let mut dot = self.dots.first_child();
        let mut i = 0;
        while let Some(d) = dot {
            if i == page { d.add_css_class("on") } else { d.remove_css_class("on") }
            dot = d.next_sibling();
            i += 1;
        }
        let (scroll, body) = page_frame();
        // Keep only the current and outgoing pages for the crossfade.
        if let Some(old) = self.stack.child_by_name("previous") {
            self.stack.remove(&old);
        }
        if let Some(old) = self.stack.visible_child() {
            self.stack.page(&old).set_name("previous");
        }
        self.stack.add_named(&scroll, Some("current"));
        self.stack.set_visible_child(&scroll);
        match page {
            HELLO => self.hello(&body),
            CHECK => self.system_check(&body),
            COMPOSITOR => self.compositor(&body),
            METHOD => self.method(&body),
            READY => self.ready(&body),
            INSTALL => self.install(&body),
            _ => self.done(&body),
        }
    }

    fn hello(self: &Wizard, body: &gtk::Box) {
        body.set_spacing(26);
        let brand = kit::pictures::brand(6.0, 64.0);
        brand.set_halign(gtk::Align::Center);
        body.append(&brand);
        let show = kit::pictures::showcase(470, 280);
        show.set_halign(gtk::Align::Center);
        body.append(&show);
        body.append(&lead(&format!("A calm, fast desktop for {}.", kit::distro::name())));
        if let Err(message) = &self.root {
            let error = lead(message);
            error.add_css_class("state-bad");
            body.append(&error);
            self.next.set_sensitive(false);
        }
    }

    fn system_check(self: &Wizard, body: &gtk::Box) {
        let lead = header(body, "computer", "Your computer", "Making sure Digit has what it needs…");
        self.next.set_sensitive(false);
        let spinner = gtk::Spinner::new();
        spinner.start();
        spinner.set_size_request(28, 28);
        body.append(&spinner);
        let (this, body) = (self.clone(), body.clone());
        glib::MainContext::default().spawn_local(async move {
            let result = match &this.root { Ok(root) => check::probe(root).await, Err(e) => Err(e.clone()) };
            body.remove(&spinner);
            let check = match result {
                Ok(check) => check,
                Err(error) => {
                    lead.set_text(&error);
                    let retry = ui::button("Try again", "");
                    retry.set_halign(gtk::Align::Center);
                    let wizard = this.clone();
                    retry.connect_clicked(move |_| wizard.show(CHECK));
                    body.append(&retry);
                    return;
                }
            };
            let card = ui::card();
            let add = |icon: &str, name: &str, sub: &str, ok: bool| {
                let (row, end) = ui::row((!icon.is_empty()).then_some(icon), name, sub);
                if icon.is_empty() {
                    // The distribution shows its own logo.
                    let logo = ui::distro_logo(28, false);
                    logo.set_valign(gtk::Align::Center);
                    row.prepend(&logo);
                }
                end.append(&state_icon(ok));
                card.append(&row);
            };
            let supported = check.supported();
            add("", check.get("distro"), if supported { "Supported" } else { "Not supported yet" }, supported);
            let gpu = match check.get("gpu") { "nvidia" => "NVIDIA", "amd" => "AMD", "intel" => "Intel", _ => "Ready" };
            add("developer_board", "Graphics", gpu, true);
            let disk = check.get("disk_free_mb").parse::<u64>().ok();
            let roomy = disk.is_none_or(|mb| mb >= 3000);
            let space = disk.map(|mb| if mb >= 10_240 { format!("{} GB free", mb / 1024) } else { format!("{mb} MB free") });
            add("hard_drive", "Storage", &if roomy { space.unwrap_or_else(|| "Enough space".into()) } else { format!("{}, Digit needs about 3 GB", space.unwrap_or_default()) }, roomy);
            if check.get("rust") != "ok" || check.get("gtk") == "none" {
                add("terminal", "Build tools", "Installed during setup", true);
            }
            body.append(&card);
            lead.set_text(if supported { "Everything Digit needs is here or will be installed." } else { "Digit runs on Arch, Fedora and openSUSE Tumbleweed or Slowroll." });
            if this.check.borrow().is_none() { *this.wm.borrow_mut() = check.preferred_wm().into(); }
            if this.page.get() == CHECK { this.next.set_sensitive(supported); }
            *this.check.borrow_mut() = Some(check);
        });
    }

    fn compositor(self: &Wizard, body: &gtk::Box) {
        header(body, "view_quilt", "How should windows behave?", "Pick the one you like. Your own setup stays as it is.");
        let check = self.check.borrow();
        let state = |wm: &str| match check.as_ref().map(|c| c.get(wm)).unwrap_or("") {
            "running" => "In use now",
            "installed" => "Installed",
            _ => "Will be installed",
        };
        let this = self.clone();
        let items = vec![
            ("hyprland", kit::pictures::compositor("hyprland").upcast(), "Hyprland", state("hyprland")),
            ("niri", kit::pictures::compositor("niri").upcast(), "niri", state("niri")),
        ];
        let current = self.wm.borrow().clone();
        body.append(&picks(items, &current, move |k| *this.wm.borrow_mut() = k.into()));
        let hint = lead("Hyprland tiles windows into the screen. niri lines them up in columns you scroll through.");
        hint.add_css_class("sub");
        body.append(&hint);
    }

    fn method(self: &Wizard, body: &gtk::Box) {
        header(body, "download", "How to install", "Pick how Digit gets onto this computer.");
        let card = |icon: &str, title: &str, sub: &str| {
            let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
            col.set_size_request(220, -1);
            let tile = ui::tile(icon);
            tile.add_css_class("big");
            tile.set_halign(gtk::Align::Center);
            tile.set_margin_top(8);
            col.append(&tile);
            let n = ui::text(title, "name");
            n.set_xalign(0.5);
            col.append(&n);
            let s = ui::text(sub, "sub");
            s.set_xalign(0.5);
            s.set_justify(gtk::Justification::Center);
            s.set_wrap(true);
            s.set_max_width_chars(26);
            col.append(&s);
            let b = gtk::ToggleButton::new();
            b.add_css_class("style-card");
            b.set_child(Some(&col));
            (b, col)
        };
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        row.set_halign(gtk::Align::Center);
        let (source, _) = card("construction", "Build on this computer", "Made for your system. Takes a few minutes.");
        source.set_active(true);
        source.connect_toggled(|b| if !b.is_active() { b.set_active(true) });
        let (package, col) = card("package_2", "Ready-made package", "Quick to install and update.");
        let soon = ui::badge("Coming soon", "");
        soon.set_halign(gtk::Align::Center);
        col.append(&soon);
        package.set_sensitive(false);
        row.append(&source);
        row.append(&package);
        body.append(&row);
    }

    fn ready(self: &Wizard, body: &gtk::Box) {
        let wm = self.wm.borrow().clone();
        let name = wm_name(&wm);
        header(body, "check_circle", "Ready to install", "Here is what happens next. It takes a few minutes and asks for your password once.");
        let card = ui::card();
        let (row, _) = ui::row(Some("login"), &format!("{name} with Digit"), "A new choice on your login screen");
        card.append(&row);
        let packages = self.check.borrow().as_ref().map(|c| c.get("packages").to_string()).unwrap_or_default();
        let count = packages.split_whitespace().count();
        let (row, end) = ui::row(Some("package_2"), "System packages", &match count {
            0 => "Already installed".into(),
            1 => "1 package to install".into(),
            n => format!("{n} packages to install"),
        });
        if count > 0 {
            row.set_tooltip_text(Some(&packages));
            end.append(&ui::badge(&count.to_string(), ""));
        }
        card.append(&row);
        let (row, _) = ui::row(Some("verified_user"), &format!("Your {name} settings"), "Stay exactly as they are");
        card.append(&row);
        body.append(&card);
        if self.check.borrow().as_ref().is_some_and(|c| c.get("gpu") == "nvidia") {
            let graphics = ui::card();
            let (row, end) = ui::row(Some("developer_board"), "Add NVIDIA variables", "For hardware video and older apps. You can change this in Settings");
            let sw = gtk::Switch::new();
            sw.set_valign(gtk::Align::Center);
            sw.set_active(self.nvidia_env.get());
            let this = self.clone();
            sw.connect_active_notify(move |s| this.nvidia_env.set(s.is_active()));
            end.append(&sw);
            graphics.append(&row);
            body.append(&graphics);
        }
    }

    fn install(self: &Wizard, body: &gtk::Box) {
        self.busy.set(true);
        let lead = header(body, "downloading", "Installing Digit", "Getting started…");
        let progress = gtk::ProgressBar::new();
        progress.add_css_class("setup-progress");
        body.append(&progress);
        let steps = ui::card();
        let mut indicators = Vec::new();
        for (_, title) in run::STEPS {
            let (row, end) = ui::row(None, title, "");
            row.add_css_class("step-row");
            let indicator = gtk::Stack::new();
            indicator.set_transition_type(gtk::StackTransitionType::Crossfade);
            let pending = ui::sym("radio_button_unchecked");
            pending.add_css_class("state-wait");
            indicator.add_named(&pending, Some("pending"));
            let spinner = gtk::Spinner::new();
            spinner.start();
            indicator.add_named(&spinner, Some("active"));
            indicator.add_named(&state_icon(true), Some("done"));
            indicator.add_named(&state_icon(false), Some("failed"));
            indicator.set_visible_child_name("pending");
            end.append(&indicator);
            steps.append(&row);
            indicators.push(indicator);
        }
        body.append(&steps);
        let log = gtk::TextView::new();
        log.set_editable(false);
        log.set_cursor_visible(false);
        log.set_monospace(true);
        log.add_css_class("installer-log");
        log.set_wrap_mode(gtk::WrapMode::WordChar);
        let log_scroll = gtk::ScrolledWindow::new();
        log_scroll.set_min_content_height(140);
        log_scroll.set_child(Some(&log));
        let details = gtk::Expander::new(Some("Details"));
        details.set_child(Some(&log_scroll));
        body.append(&details);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.set_halign(gtk::Align::Center);
        body.append(&actions);
        let (this, lead) = (self.clone(), lead.clone());
        glib::MainContext::default().spawn_local(async move {
            let active = Cell::new(None::<usize>);
            let wm = this.wm.borrow().clone();
            let result = match &this.root {
                Err(e) => Err(e.clone()),
                Ok(root) => match run::start(root, &wm, this.nvidia_env.get()) {
                    Ok(process) => {
                        *this.process.borrow_mut() = Some(process.clone());
                        run::follow(&process, |event| match event {
                            Event::Step(index, title) => {
                                if let Some(previous) = active.replace(Some(index)) {
                                    indicators[previous].set_visible_child_name("done");
                                }
                                indicators[index].set_visible_child_name("active");
                                lead.set_text(&format!("{title}…"));
                                progress.set_fraction(index as f64 / run::STEPS.len() as f64);
                            }
                            Event::Warn(text) | Event::Fail(text) => lead.set_text(&text),
                            Event::Done => {}
                            Event::Log(text) => {
                                let buffer = log.buffer();
                                buffer.insert(&mut buffer.end_iter(), &format!("{text}\n"));
                                let mark = buffer.create_mark(None, &buffer.end_iter(), false);
                                log.scroll_mark_onscreen(&mark);
                                buffer.delete_mark(&mark);
                            }
                        })
                        .await
                    }
                    Err(e) => Err(e),
                },
            };
            this.busy.set(false);
            this.process.borrow_mut().take();
            match result {
                Ok(()) => {
                    for indicator in &indicators {
                        indicator.set_visible_child_name("done");
                    }
                    progress.set_fraction(1.0);
                    this.show(DONE);
                }
                Err(error) => {
                    if let Some(index) = active.get() {
                        indicators[index].set_visible_child_name("failed");
                    }
                    lead.set_text(&error);
                    lead.add_css_class("state-bad");
                    let show = ui::button("Show log", "");
                    let message = lead.clone();
                    show.connect_clicked(move |_| {
                        let path = glib::home_dir().join(".cache/digit/install.log");
                        if let Err(e) = gio::AppInfo::launch_default_for_uri(&gio::File::for_path(path).uri(), gio::AppLaunchContext::NONE) {
                            message.set_text(&e.to_string());
                        }
                    });
                    let retry = ui::button("Try again", "accent");
                    let wizard = this.clone();
                    retry.connect_clicked(move |_| wizard.show(INSTALL));
                    actions.append(&show);
                    actions.append(&retry);
                }
            }
        });
    }

    fn done(self: &Wizard, body: &gtk::Box) {
        let name = wm_name(&self.wm.borrow()).to_string();
        let lead = header(body, "check_circle", "Digit is ready", &format!("Log out, then choose “{name} with Digit” on the login screen. A short tour starts on your first login."));
        let logout = ui::button("Log out now", "accent");
        logout.set_halign(gtk::Align::Center);
        let window = self.window.clone();
        logout.connect_clicked(move |_| {
            let lead = lead.clone();
            ui::ask(Some(window.upcast_ref()), "Log out now?", "Save your work in open apps first.", "Cancel", "Log out", "accent", move || {
                let target = std::env::var("XDG_SESSION_ID").ok().filter(|s| !s.is_empty()).map(|id| ("terminate-session", id))
                    .or_else(|| std::env::var("USER").ok().filter(|s| !s.is_empty()).map(|user| ("terminate-user", user)));
                let Some((action, value)) = target else {
                    lead.set_text("Log out from your desktop's menu.");
                    return;
                };
                glib::MainContext::default().spawn_local(async move {
                    let ok = match gio::Subprocess::newv(&["loginctl".as_ref(), action.as_ref(), value.as_ref()], gio::SubprocessFlags::STDERR_PIPE) {
                        Ok(p) => p.communicate_utf8_future(None).await.is_ok() && p.is_successful(),
                        Err(_) => false,
                    };
                    if !ok { lead.set_text("Couldn't log out. Use your desktop's menu."); }
                });
            });
        });
        body.append(&logout);
    }
}

pub fn open(app: &gtk::Application, root: Result<PathBuf, String>) {
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Install Digit")
        .default_width(820)
        .default_height(640)
        .decorated(false)
        .build();
    window.add_css_class("digit-installer");
    kit::scroll::smooth(&window);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    top.set_halign(gtk::Align::End);
    top.set_margin_top(10);
    top.set_margin_end(10);
    let close = gtk::Button::new();
    close.set_child(Some(&ui::sym("close")));
    close.add_css_class("ctl");
    close.add_css_class("close");
    close.set_tooltip_text(Some("Close"));
    top.append(&close);
    content.append(&top);
    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_transition_duration(if kit::motion::reduced() { 0 } else { 220 });
    stack.set_vexpand(true);
    content.append(&stack);
    let bottom = gtk::CenterBox::new();
    bottom.set_margin_start(28);
    bottom.set_margin_end(28);
    bottom.set_margin_bottom(24);
    bottom.set_margin_top(12);
    let back = ui::button("Back", "");
    let next = ui::button("Continue", "accent");
    let dots = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    dots.set_valign(gtk::Align::Center);
    for _ in HELLO..=READY {
        let d = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        d.add_css_class("step-dot");
        dots.append(&d);
    }
    bottom.set_start_widget(Some(&back));
    bottom.set_center_widget(Some(&dots));
    bottom.set_end_widget(Some(&next));
    content.append(&bottom);
    // Undecorated, so the whole window is the handle to move it by.
    let handle = gtk::WindowHandle::new();
    handle.set_child(Some(&content));
    window.set_child(Some(&handle));
    let this = Rc::new(Installer {
        window: window.clone(),
        stack,
        back,
        next,
        dots,
        page: Cell::new(HELLO),
        root,
        check: RefCell::new(None),
        wm: RefCell::new("hyprland".into()),
        nvidia_env: Cell::new(std::env::var("DIGIT_INSTALLER_NVIDIA_ENV").as_deref() == Ok("true")),
        busy: Cell::new(false),
        process: RefCell::new(None),
    });
    let w = window.clone();
    close.connect_clicked(move |_| w.close());
    let wizard = this.clone();
    this.back.connect_clicked(move |_| {
        if !wizard.busy.get() {
            wizard.show(wizard.page.get().saturating_sub(1));
        }
    });
    let wizard = this.clone();
    this.next.connect_clicked(move |_| {
        if wizard.busy.get() {
            return;
        }
        if wizard.page.get() == DONE {
            wizard.window.close();
        } else {
            wizard.show(wizard.page.get() + 1);
        }
    });
    let wizard = this.clone();
    window.connect_close_request(move |window| {
        if !wizard.busy.get() { return glib::Propagation::Proceed; }
        let this = wizard.clone();
        ui::ask(Some(window.upcast_ref()), "Stop watching the install?", "It keeps going in the background. The log is in ~/.cache/digit/install.log.", "Keep open", "Close", "danger", move || {
            // Keep the main loop alive to drain the engine pipes and serve askpass until it exits.
            this.window.set_visible(false);
            let wizard = this.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
                if wizard.busy.get() { return glib::ControlFlow::Continue; }
                wizard.window.close();
                glib::ControlFlow::Break
            });
        });
        glib::Propagation::Stop
    });
    // Debug builds can open on any step to check its look: DIGIT_INSTALLER_PAGE=3.
    let first = if cfg!(debug_assertions) { std::env::var("DIGIT_INSTALLER_PAGE").ok().and_then(|p| p.parse().ok()).unwrap_or(HELLO) } else { HELLO };
    // Never the install step itself: that would start installing.
    this.show(if first == INSTALL { READY } else { first.min(DONE) });
    window.present();
}
