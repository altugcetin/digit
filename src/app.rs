use crate::config::{Config, Dock};
use crate::palette::{Palette, Rgb, Roles};
use crate::{blur, media, notch, notify, spectrum};
use gtk::{gdk, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub enum Peek {
    Volume { level: f64, muted: bool },
    Track { title: String, artist: String },
    Notification { id: u32, summary: String, app: String, urgent: bool },
    Note { text: String },
}

impl Peek {
    pub fn same_kind(&self, other: &Peek) -> bool {
        match (self, other) {
            (Peek::Notification { id: a, .. }, Peek::Notification { id: b, .. }) => a == b,
            _ => std::mem::discriminant(self) == std::mem::discriminant(other),
        }
    }

    fn stacks(&self) -> bool {
        matches!(self, Peek::Notification { .. })
    }

    fn stale(&self) -> bool {
        matches!(self, Peek::Volume { .. })
    }

    fn waits(&self) -> bool {
        matches!(self, Peek::Notification { .. })
    }

    pub fn priority(&self) -> i32 {
        match self {
            Peek::Volume { .. } => 76,
            Peek::Track { .. } => 45,
            Peek::Notification { urgent: true, .. } => 80,
            Peek::Notification { .. } => 60,
            Peek::Note { .. } => 65,
        }
    }

    fn hold(&self) -> u64 {
        match self {
            Peek::Volume { .. } => 1300,
            Peek::Track { .. } => 3000,
            Peek::Notification { urgent: true, .. } => 8000,
            Peek::Notification { .. } => 4000,
            Peek::Note { .. } => 4000,
        }
    }

    pub fn wing(&self) -> f64 {
        match self {
            Peek::Volume { .. } => 108.0,
            Peek::Track { .. } => 140.0,
            Peek::Notification { .. } => 172.0,
            Peek::Note { .. } => 140.0,
        }
    }
}

#[derive(Clone)]
struct Queued {
    peek: Peek,
    at: i64,
}

#[derive(Clone, Default)]
pub struct Media {
    pub bus: Option<String>,
    pub playing: bool,
    pub paused_at: Option<i64>,
    pub title: String,
    pub artist: String,
    pub art_url: String,
    pub art: Option<gdk::Texture>,
    pub length: f64,
    pub position: f64,
    pub position_at: i64,
}

impl Media {
    pub fn present(&self, now: i64) -> bool {
        self.bus.is_some() && (self.playing || self.paused_at.is_some_and(|t| now - t < 600_000_000))
    }

    pub fn wings(&self, now: i64) -> bool {
        self.bus.is_some() && (self.playing || self.paused_at.is_some_and(|t| now - t < 3_000_000))
    }

    pub fn elapsed(&self, now: i64) -> f64 {
        let run = if self.playing { (now - self.position_at) as f64 / 1e6 } else { 0.0 };
        (self.position + run).clamp(0.0, self.length.max(0.0))
    }
}

pub struct App {
    pub cfg: Config,
    pub blur: Option<blur::Blur>,
    pub palette: Cell<Palette>,
    pub clock: RefCell<String>,
    pub date: RefCell<String>,
    pub media: RefCell<Media>,
    pub volume: Cell<Option<(f64, bool)>>,
    pub spectrum: RefCell<Vec<f64>>,
    pub peek: RefCell<Option<Peek>>,
    queue: RefCell<Vec<Queued>>,
    shown_gen: Cell<u64>,
    warm: Cell<bool>,
    pub notches: RefCell<Vec<notch::Notch>>,
    pub notify: RefCell<Option<notify::Handle>>,
    pub players: RefCell<Option<media::Players>>,
    pub cava: RefCell<Option<spectrum::Cava>>,
    pub kept: RefCell<Vec<glib::Object>>,
    open_count: Cell<usize>,
}

impl App {
    pub fn new(cfg: Config, blur: Option<blur::Blur>) -> Rc<Self> {
        let frosted = blur.as_ref().is_some_and(|b| b.able());
        let body = Self::body_for(&cfg, Roles::default(), frosted);
        let app = Rc::new(Self {
            blur,
            palette: Cell::new(Palette::new(Roles::default(), body)),
            cfg,
            clock: RefCell::new(String::new()),
            date: RefCell::new(String::new()),
            media: RefCell::new(Media::default()),
            volume: Cell::new(None),
            spectrum: RefCell::new(Vec::new()),
            peek: RefCell::new(None),
            queue: RefCell::new(Vec::new()),
            shown_gen: Cell::new(0),
            warm: Cell::new(false),
            notches: RefCell::new(Vec::new()),
            notify: RefCell::new(None),
            players: RefCell::new(None),
            cava: RefCell::new(None),
            kept: RefCell::new(Vec::new()),
            open_count: Cell::new(0),
        });
        let weak = Rc::downgrade(&app);
        glib::timeout_add_local_once(Duration::from_millis(1500), move || {
            if let Some(app) = weak.upgrade() {
                app.warm.set(true);
            }
        });
        app.tick_clock();
        app
    }

    fn body_for(cfg: &Config, roles: Roles, frosted: bool) -> Rgb {
        match cfg.dock {
            Dock::Bar => cfg
                .bar_color
                .as_deref()
                .and_then(Rgb::hex)
                .unwrap_or_else(|| roles.surface_high.mix(roles.surface, 0.47)),
            Dock::Screen if frosted => roles.surface_high.alpha(1.0 - cfg.glass.clamp(0.0, 0.6)),
            Dock::Screen => roles.surface_high,
        }
    }

    pub fn set_roles(&self, roles: Roles) {
        let frosted = self.blur.as_ref().is_some_and(|b| b.able());
        self.palette.set(Palette::new(roles, Self::body_for(&self.cfg, roles, frosted)));
        self.redraw();
    }

    pub fn now() -> i64 {
        glib::monotonic_time()
    }

    pub fn warm(&self) -> bool {
        self.warm.get()
    }

    fn tick_clock(self: &Rc<Self>) {
        let now = glib::DateTime::now_local().ok();
        let fmt = |f: &str| now.as_ref().and_then(|d| d.format(f).ok()).map(|s| s.to_string()).unwrap_or_default();
        let clock = fmt(&self.cfg.clock);
        let date = fmt(&self.cfg.date);
        let changed = *self.clock.borrow() != clock || *self.date.borrow() != date;
        *self.clock.borrow_mut() = clock;
        *self.date.borrow_mut() = date;
        if changed {
            self.changed();
        }
        let into = now.map(|d| d.seconds() % 60.0).unwrap_or(0.0);
        let wait = ((60.0 - into) * 1000.0).max(50.0) as u64 + 15;
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(wait), move || {
            if let Some(app) = weak.upgrade() {
                app.tick_clock();
            }
        });
    }

    pub fn redraw(&self) {
        for n in self.notches.borrow().iter() {
            n.redraw();
        }
    }

    pub fn changed(&self) {
        let notches: Vec<_> = self.notches.borrow().iter().cloned().collect();
        for n in notches {
            n.refresh();
        }
    }

    pub fn request(self: &Rc<Self>, peek: Peek) {
        if !self.warm.get() {
            return;
        }
        let current = self.peek.borrow().clone();
        if let Some(cur) = current {
            if cur.same_kind(&peek) {
                *self.peek.borrow_mut() = Some(peek.clone());
                self.hold(&peek);
                self.changed();
                return;
            }
            if cur.stacks() && peek.stacks() {
                self.enqueue(peek);
                return;
            }
            if peek.priority() < cur.priority() {
                if !peek.stale() {
                    self.enqueue(peek);
                }
                return;
            }
            if cur.waits() {
                self.queue.borrow_mut().insert(0, Queued { peek: cur, at: 0 });
            }
        }
        self.activate(peek);
    }

    fn enqueue(&self, peek: Peek) {
        let mut q = self.queue.borrow_mut();
        q.retain(|x| !x.peek.same_kind(&peek));
        q.push(Queued { peek, at: Self::now() });
        while q.len() > 6 {
            q.remove(0);
        }
    }

    fn activate(self: &Rc<Self>, peek: Peek) {
        *self.peek.borrow_mut() = Some(peek.clone());
        self.hold(&peek);
        self.changed();
    }

    fn hold(self: &Rc<Self>, peek: &Peek) {
        let generation = self.shown_gen.get() + 1;
        self.shown_gen.set(generation);
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(peek.hold()), move || {
            if let Some(app) = weak.upgrade() {
                if app.shown_gen.get() == generation {
                    app.dismiss(Ending::Expired);
                }
            }
        });
    }

    pub fn dismiss(self: &Rc<Self>, how: Ending) {
        self.shown_gen.set(self.shown_gen.get() + 1);
        let ended = self.peek.borrow_mut().take();
        if let Some(Peek::Notification { id, .. }) = ended {
            if let Some(n) = self.notify.borrow().as_ref() {
                match how {
                    Ending::Expired => n.closed(id, 1),
                    Ending::Clicked => n.clicked(id),
                    Ending::Closed => {}
                }
            }
        }
        let now = Self::now();
        let next = {
            let mut q = self.queue.borrow_mut();
            q.retain(|x| x.peek.waits() || x.at == 0 || now - x.at <= 10_000_000);
            if q.is_empty() { None } else { Some(q.remove(0).peek) }
        };
        match next {
            Some(peek) => self.activate(peek),
            None => self.changed(),
        }
    }

    pub fn cancel_notification(self: &Rc<Self>, id: u32) {
        self.queue.borrow_mut().retain(|x| !matches!(x.peek, Peek::Notification { id: i, .. } if i == id));
        let showing = matches!(&*self.peek.borrow(), Some(Peek::Notification { id: i, .. }) if *i == id);
        if showing {
            self.dismiss(Ending::Closed);
        }
    }

    pub fn set_open(&self, open: bool) {
        let count = self.open_count.get();
        self.open_count.set(if open { count + 1 } else { count.saturating_sub(1) });
        if let Some(p) = self.players.borrow().as_ref() {
            p.poll_position(self.open_count.get() > 0);
        }
    }

    pub fn media_changed(self: &Rc<Self>) {
        let wants = self.media.borrow().wings(Self::now());
        spectrum::want(self, wants);
        if !self.media.borrow().playing && self.media.borrow().bus.is_some() {
            let weak = Rc::downgrade(self);
            glib::timeout_add_local_once(Duration::from_millis(3100), move || {
                if let Some(app) = weak.upgrade() {
                    let wants = app.media.borrow().wings(Self::now());
                    spectrum::want(&app, wants);
                    app.changed();
                }
            });
        }
        self.changed();
    }
}

#[derive(Clone, Copy)]
pub enum Ending {
    Expired,
    Clicked,
    Closed,
}
