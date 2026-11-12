//! The notch's pointer, drag and drop handling: hover, clicks, scrubbing, the volume tuner.

use super::*;

impl Notch {
    pub(super) fn hover(&self, inside: bool) {
        let (open, peek, hover) = {
            let mut v = self.0.borrow_mut();
            v.hovered = inside;
            v.leave += 1;
            v.dwell += 1;
            (v.open, v.app.peek.borrow().is_some(), v.app.prefs.borrow().hover)
        };
        if inside {
            if hover && !open && !peek {
                self.dwell();
            }
        } else {
            let generation = self.0.borrow().leave;
            let weak = Rc::downgrade(&self.0);
            glib::timeout_add_local_once(Duration::from_millis(crate::app::LEAVE_MS), move || {
                if let Some(v) = weak.upgrade() {
                    let fold = {
                        let v = v.borrow();
                        v.leave == generation && !v.hovered && v.app.prefs.borrow().notch_close == "leave"
                    };
                    if fold {
                        Notch(v).set_open(false);
                    }
                }
            });
        }
    }

    pub(super) fn dwell(&self) {
        let generation = {
            let mut v = self.0.borrow_mut();
            v.dwell += 1;
            v.dwell
        };
        let weak = Rc::downgrade(&self.0);
        glib::timeout_add_local_once(Duration::from_millis(300), move || {
            if let Some(v) = weak.upgrade() {
                let open = {
                    let v = v.borrow();
                    let now = App::now();
                    let morphing = !v.float.settled(now) || !v.widen.settled(now);
                    v.dwell == generation && v.hovered && !v.open && !morphing && v.app.peek.borrow().is_none()
                };
                if open {
                    Notch(v).set_open(true);
                }
            }
        });
    }

    pub(super) fn point(&self, at: Option<(f64, f64)>) {
        let (canvas, changed, inside, was) = {
            let mut v = self.0.borrow_mut();
            v.pointer = at;
            let now = App::now();
            let inside = at.is_some_and(|(x, y)| v.over_body(x, y, now));
            let was = v.hovered;
            let hot = at.and_then(|(x, y)| v.hot_at(x, y, now));
            let changed = hot != v.hot;
            if changed {
                v.hot = hot;
                for i in 0..v.glow.len() {
                    let on = hot.map(Hot::index) == Some(i);
                    v.glow[i].go(if on { 1.0 } else { 0.0 }, now, if on { 160 } else { 280 }, motion::STANDARD);
                }
            }
            (v.canvas.clone(), changed.then_some(hot.is_some()), inside, was)
        };
        if inside != was {
            self.hover(inside);
        }
        if let Some(pointer) = changed {
            canvas.set_cursor_from_name(pointer.then_some("pointer"));
            self.kick();
        }
    }

    pub(super) fn switch_tab(&self, tab: Tab) {
        let app = {
            let mut v = self.0.borrow_mut();
            if v.tab == tab {
                return;
            }
            v.tab = tab;
            v.tab_fade.set(0.0);
            v.tab_fade.go(1.0, App::now(), 240, motion::STANDARD);
            v.hot = None;
            v.app.clone()
        };
        if tab == Tab::Tray {
            crate::share::forget_senders();
            crate::share::scan(&app);
        }
        if tab == Tab::Phone {
            crate::phone::refresh(&app);
        }
        let at = self.0.borrow().pointer;
        self.point(at);
        self.kick();
    }

    pub(super) fn tool(&self, i: usize) {
        let app = self.0.borrow().app.clone();
        match i {
            0 => {
                self.set_open(false);
                glib::timeout_add_local_once(Duration::from_millis(320), || {
                    glib::spawn_future_local(async {
                        let _ = gtk::gio::spawn_blocking(crate::compositor::screenshot).await;
                    });
                });
            }
            1 => {
                self.set_open(false);
                let weak = Rc::downgrade(&app);
                glib::timeout_add_local_once(Duration::from_millis(320), move || {
                    let argv: [&std::ffi::OsStr; 2] = ["digit-snap".as_ref(), "pick".as_ref()];
                    if gtk::gio::Subprocess::newv(&argv, gtk::gio::SubprocessFlags::STDOUT_SILENCE | gtk::gio::SubprocessFlags::STDERR_SILENCE).is_ok() {
                        return;
                    }
                    glib::spawn_future_local(async move {
                        let Ok(Some([r, g, b])) = gtk::gio::spawn_blocking(crate::compositor::pick_color).await else { return };
                        let hex = format!("#{:02X}{:02X}{:02X}", (r * 255.0).round() as u8, (g * 255.0).round() as u8, (b * 255.0).round() as u8);
                        let _ = crate::child::spawn(&["wl-copy".as_ref(), hex.as_ref()], gtk::gio::SubprocessFlags::NONE);
                        if let Some(app) = weak.upgrade() {
                            app.request(crate::app::Peek::Note { text: format!("{hex} copied") });
                        }
                    });
                });
            }
            2 => {
                app.set_dnd(!app.dnd.get());
                app.history_changed();
            }
            _ => {
                let on = app.awake.borrow().as_ref().is_some_and(|a| a.on());
                let surface = self.surface();
                if let Some(a) = app.awake.borrow().as_ref() {
                    a.set(surface.as_ref(), !on);
                }
                app.history_changed();
            }
        }
        self.0.borrow().canvas.queue_draw();
    }

    pub(super) fn drag_enter(&self) {
        self.0.borrow_mut().dropping = true;
        if !self.is_open() {
            self.0.borrow_mut().tab = Tab::Tray;
            self.set_open(true);
        } else {
            self.switch_tab(Tab::Tray);
        }
        let app = self.0.borrow().app.clone();
        crate::share::scan(&app);
        self.kick();
    }

    pub(super) fn drag_motion(&self, x: f64, y: f64) {
        let changed = {
            let mut v = self.0.borrow_mut();
            let at = v.landing_at(x, y);
            let changed = at != v.landing;
            v.landing = at;
            changed
        };
        if changed {
            self.0.borrow().canvas.queue_draw();
        }
    }

    pub(super) fn drag_leave(&self) {
        {
            let mut v = self.0.borrow_mut();
            v.dropping = false;
            v.landing = None;
            v.canvas.queue_draw();
        }
        self.kick();
        let weak = Rc::downgrade(&self.0);
        glib::timeout_add_local_once(Duration::from_millis(500), move || {
            if let Some(v) = weak.upgrade() {
                let idle = {
                    let v = v.borrow();
                    !v.hovered && !v.dropping
                };
                if idle {
                    Notch(v).set_open(false);
                }
            }
        });
    }

    pub(super) fn dropped(&self, paths: Vec<std::path::PathBuf>, landing: DropLanding) -> bool {
        let app = self.0.borrow().app.clone();
        if paths.is_empty() {
            crate::share::failed(&app, "Couldn't add image");
            if !self.is_open() {
                app.request(crate::app::Peek::Note { text: "Couldn't add image".into() });
            }
            return false;
        }
        match landing {
            DropLanding::Device(device) => crate::share::send(&app, &device, paths),
            DropLanding::Sender(sender) => crate::share::send_with(&app, &sender, paths),
            DropLanding::Files => crate::tray::add(paths),
        }
        self.0.borrow().canvas.queue_draw();
        true
    }

    pub(super) fn tile_at(&self, x: f64, y: f64) -> Option<usize> {
        let v = self.0.borrow();
        if !v.open || v.tab != Tab::Tray {
            return None;
        }
        (0..crate::tray::len().min(TILES)).find(|&i| v.tile_rect(i).contains(x, y) && !v.remove_rect(i).contains(x, y))
    }

    pub(super) fn set_level(&self, level: f64) {
        let app = self.0.borrow().app.clone();
        let muted = app.volume.get().is_some_and(|v| v.1);
        let level = level.clamp(0.0, 1.0);
        app.volume.set(Some((level, muted)));
        audio::ask(Ask::Level(level));
        app.history_changed();
        self.0.borrow().canvas.queue_draw();
    }

    pub(super) fn tune(&self, on: bool) {
        {
            let mut v = self.0.borrow_mut();
            if v.tuning == on {
                return;
            }
            v.tuning = on;
            let now = App::now();
            if on {
                v.tune.go(1.0, now, 380, motion::DEFAULT_SPATIAL);
            } else {
                v.tune.go(0.0, now, 200, motion::FAST_EFFECTS);
            }
        }
        let at = self.0.borrow().pointer;
        self.0.borrow_mut().hot = None;
        self.point(at);
        self.kick();
    }

    pub(super) fn wheel(&self, dy: f64) -> bool {
        let inside = {
            let v = self.0.borrow();
            let now = App::now();
            v.open && v.pointer.is_some_and(|(x, y)| v.volume_zone(now).is_some_and(|z| z.contains(x, y)))
        };
        if !inside {
            return false;
        }
        let Some((level, _)) = self.0.borrow().app.volume.get() else { return false };
        self.tune(true);
        self.set_level(level - dy * 0.02);
        true
    }

    pub(super) fn begin_drag(&self, x: f64, y: f64) -> bool {
        let level = {
            let mut v = self.0.borrow_mut();
            let now = App::now();
            match v.tuner(now) {
                Some(t) if v.tuning && x >= t.track.x - 8.0 && x <= t.track.x + t.track.w + 8.0 && (y - (t.track.y + t.track.h / 2.0)).abs() <= 12.0 => {
                    v.sliding = true;
                    Some(((x - t.track.x) / t.track.w).clamp(0.0, 1.0))
                }
                _ => None,
            }
        };
        match level {
            Some(l) => {
                self.set_level(l);
                true
            }
            None => self.begin_scrub(x, y),
        }
    }

    pub(super) fn move_drag(&self, x: f64) {
        let level = {
            let v = self.0.borrow();
            if !v.sliding {
                None
            } else {
                v.tuner(App::now()).map(|t| ((x - t.track.x) / t.track.w).clamp(0.0, 1.0))
            }
        };
        match level {
            Some(l) => self.set_level(l),
            None => self.move_scrub(x),
        }
    }

    pub(super) fn end_drag(&self, x: f64) {
        let sliding = std::mem::replace(&mut self.0.borrow_mut().sliding, false);
        if sliding {
            self.move_drag_end(x);
        } else {
            self.end_scrub(x);
        }
    }

    pub(super) fn move_drag_end(&self, x: f64) {
        let level = self.0.borrow().tuner(App::now()).map(|t| ((x - t.track.x) / t.track.w).clamp(0.0, 1.0));
        if let Some(l) = level {
            self.set_level(l);
        }
        self.0.borrow().canvas.queue_draw();
    }

    pub(super) fn seek_fraction(v: &View, x: f64) -> Option<f64> {
        let p = v.player(App::now())?;
        Some(((x - p.seek.x) / p.seek.w).clamp(0.0, 1.0))
    }

    pub(super) fn begin_scrub(&self, x: f64, y: f64) -> bool {
        let mut v = self.0.borrow_mut();
        let seekable = {
            let m = v.app.media.borrow();
            m.timing == Timing::Timed && m.can_seek
        };
        let Some(p) = v.player(App::now()) else { return false };
        if !v.open || !seekable || !p.seek.contains(x, y) {
            return false;
        }
        v.scrub = Self::seek_fraction(&v, x);
        v.canvas.queue_draw();
        true
    }

    pub(super) fn move_scrub(&self, x: f64) {
        let mut v = self.0.borrow_mut();
        if v.scrub.is_some() {
            v.scrub = Self::seek_fraction(&v, x);
            v.canvas.queue_draw();
        }
    }

    pub(super) fn end_scrub(&self, x: f64) {
        let (app, fraction) = {
            let mut v = self.0.borrow_mut();
            if v.scrub.take().is_none() {
                return;
            }
            (v.app.clone(), Self::seek_fraction(&v, x))
        };
        if let (Some(f), Some(p)) = (fraction, app.players.borrow().as_ref()) {
            p.seek_to(f);
        }
        self.0.borrow().canvas.queue_draw();
    }

    pub(super) fn click(&self, x: f64, y: f64) {
        let outside = {
            let v = self.0.borrow();
            !v.open && !v.over_body(x, y, App::now())
        };
        if outside {
            return;
        }
        let (zone, tuner, tuning) = {
            let v = self.0.borrow();
            let now = App::now();
            (if v.open { v.volume_zone(now) } else { None }, v.tuner(now), v.tuning)
        };
        if let Some(zone) = zone.filter(|z| z.contains(x, y)) {
            if !tuning {
                self.tune(true);
                return;
            }
            if tuner.is_some_and(|t| t.icon.contains(x, y)) {
                let app = self.0.borrow().app.clone();
                if let Some((level, muted)) = app.volume.get() {
                    app.volume.set(Some((level, !muted)));
                    audio::ask(Ask::Mute(!muted));
                    app.history_changed();
                    self.0.borrow().canvas.queue_draw();
                }
            }
            let _ = zone;
            return;
        }
        if tuning && y <= self.0.borrow().header() {
            self.tune(false);
            return;
        }
        let hot = {
            let v = self.0.borrow();
            v.hot_at(x, y, App::now())
        };
        let app = self.0.borrow().app.clone();
        match hot {
            Some(Hot::CallMute) => {
                audio::ask(Ask::MicFlip);
                return;
            }
            Some(Hot::CallSwitch) => {
                {
                    let mut v = self.0.borrow_mut();
                    v.media_over_call.set(!v.media_over_call.get());
                    v.flip.set(0.0);
                    v.flip.go(1.0, App::now(), 240, motion::STANDARD);
                }
                let at = self.0.borrow().pointer;
                self.point(at);
                self.kick();
                return;
            }
            Some(Hot::CallOpen) => {
                let who = app.call.borrow().as_ref().map(|c| c.who.to_lowercase());
                self.set_open(false);
                if let Some(who) = who {
                    let apps = app.apps.clone();
                    let target = crate::compositor::windows().into_iter().find(|w| apps.display_name(&w.app_id).to_lowercase() == who || w.app_id.to_lowercase().contains(&who)).map(|w| w.id);
                    if let Some(id) = target {
                        glib::spawn_future_local(async move {
                            let _ = gtk::gio::spawn_blocking(move || crate::compositor::focus(id)).await;
                        });
                    }
                }
                return;
            }
            Some(Hot::Gear) => {
                self.set_open(false);
                crate::config::settings("notch", &app.cfg);
                return;
            }
            Some(Hot::Tab(i)) => {
                self.switch_tab(TABS[i].0);
                return;
            }
            Some(Hot::PhoneAction(i)) => {
                crate::phone::act(&app, crate::phone::ACTIONS[i].0);
                return;
            }
            Some(Hot::PhoneSwitch) => {
                crate::phone::switch(&app);
                return;
            }
            Some(Hot::PhoneDismiss(i)) => {
                if let Some(n) = crate::phone::phone().and_then(|p| p.notes.get(i).cloned()) {
                    crate::phone::dismiss(&n);
                }
                return;
            }
            Some(Hot::Tool(i)) => {
                self.tool(i);
                return;
            }
            Some(Hot::Day(i)) => {
                self.0.borrow_mut().day = i as i32 - 3;
                self.0.borrow().canvas.queue_draw();
                return;
            }
            Some(Hot::Remove(i)) => {
                crate::tray::remove(i);
                let at = self.0.borrow().pointer;
                self.0.borrow_mut().hot = None;
                self.point(at);
                self.0.borrow().canvas.queue_draw();
                return;
            }
            Some(Hot::Tile(i)) => {
                if let Some(path) = crate::tray::path(i) {
                    let uri = gtk::gio::File::for_path(path).uri();
                    let context = gtk::gio::AppLaunchContext::new();
                    crate::apps::prepare(&context);
                    let _ = gtk::gio::AppInfo::launch_default_for_uri(&uri, Some(&context));
                    self.set_open(false);
                }
                return;
            }
            Some(Hot::Device(i)) => {
                let files: Vec<std::path::PathBuf> = (0..crate::tray::len()).filter_map(crate::tray::path).collect();
                if let Some(d) = crate::share::devices().get(i) {
                    crate::share::send(&app, d, files);
                }
                return;
            }
            Some(Hot::Sender(i)) => {
                let files: Vec<std::path::PathBuf> = (0..crate::tray::len()).filter_map(crate::tray::path).collect();
                if let Some(sender) = crate::share::senders().get(i) {
                    crate::share::send_with(&app, sender, files);
                }
                return;
            }
            _ => {}
        }
        let target = {
            let v = self.0.borrow();
            v.space_at(x, y)
        };
        if let Some(id) = target {
            glib::spawn_future_local(async move {
                let _ = gtk::gio::spawn_blocking(move || crate::compositor::focus_space(id)).await;
            });
            return;
        }
        let (app, open, action) = {
            let v = self.0.borrow();
            let action = if v.open && v.tab == Tab::Nook {
                v.player(App::now()).and_then(|p| p.controls.into_iter().find(|(r, _)| r.contains(x, y)).map(|(_, a)| a))
            } else {
                None
            };
            (v.app.clone(), v.open, action)
        };
        if let Some(action) = action {
            let cmd = match action {
                Action::Previous => media::Command::Previous,
                Action::Toggle => media::Command::PlayPause,
                Action::Next => media::Command::Next,
            };
            if let Some(p) = app.players.borrow().as_ref() {
                p.command(cmd);
            }
            return;
        }
        if open {
            if y <= app.cfg.header() {
                self.set_open(false);
            }
            return;
        }
        let peeking = app.peek.borrow().is_some();
        let action = {
            let v = self.0.borrow();
            match (&v.head, v.hot_at(x, y, App::now())) {
                (Head::Peek(Peek::Notification { actions, .. }), Some(Hot::Action(i))) => actions.get(i).map(|a| a.0.clone()),
                _ => None,
            }
        };
        if let Some(key) = action {
            app.dismiss(Ending::Action(key));
        } else if peeking {
            app.dismiss(Ending::Clicked);
        } else {
            self.set_open(true);
        }
    }

    pub(super) fn kick(&self) {
        let (canvas, already) = {
            let mut v = self.0.borrow_mut();
            let already = v.ticking;
            v.ticking = true;
            (v.canvas.clone(), already)
        };
        if already {
            return;
        }
        let weak: Weak<RefCell<View>> = Rc::downgrade(&self.0);
        canvas.add_tick_callback(move |w, clock| {
            let Some(view) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let Ok(mut v) = view.try_borrow_mut() else {
                return glib::ControlFlow::Continue;
            };
            let busy = v.step(clock.frame_time());
            w.queue_draw();
            if busy {
                glib::ControlFlow::Continue
            } else {
                v.ticking = false;
                glib::ControlFlow::Break
            }
        });
    }
}
