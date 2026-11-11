use crate::config::{Config, Dock};
use kit::palette::{Palette, Rgb, Roles};
use crate::{apps, blur, launcher, media, notch, notify, picker, spectrum, wallpaper};
use gtk::{gdk, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub enum Peek {
    Volume { level: f64, muted: bool },
    Track { title: String, artist: String },
    Notification { id: u32, summary: String, body: String, app: String, urgent: bool, icon: Option<gdk::Paintable>, avatar: Option<gdk::Paintable>, actions: Vec<(String, String)> },
    Battery { level: u8, device: Option<(String, &'static str)> },
    Device { name: String, icon: &'static str, battery: Option<u8>, on: bool },
    Output { name: String, icon: &'static str },
    Note { text: String },
    Mode { icon: &'static str, label: &'static str, on: bool },
    Osd { icon: &'static str, text: String, level: Option<f64> },
    Spaces { idx: u64 },
    Privacy { light: crate::privacy::Light, who: String },
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
        matches!(self, Peek::Volume { .. } | Peek::Mode { .. } | Peek::Osd { .. } | Peek::Spaces { .. })
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
            Peek::Mode { .. } => 70,
            Peek::Osd { .. } => 76,
            Peek::Spaces { .. } => 58,
            Peek::Privacy { .. } => 74,
            Peek::Battery { .. } => 90,
            Peek::Device { .. } => 72,
            Peek::Output { .. } => 71,
        }
    }

    fn hold(&self) -> u64 {
        match self {
            Peek::Volume { .. } => 1300,
            Peek::Track { .. } => 3000,
            Peek::Notification { actions, .. } if !actions.is_empty() => 15000,
            Peek::Notification { urgent: true, .. } => 8000,
            Peek::Notification { .. } => 4000,
            Peek::Note { .. } => 4000,
            Peek::Mode { .. } => 1600,
            Peek::Osd { .. } => 1400,
            Peek::Spaces { .. } => 1100,
            Peek::Privacy { .. } => 2600,
            Peek::Battery { .. } => 4200,
            Peek::Device { .. } => 2600,
            Peek::Output { .. } => 2200,
        }
    }

    pub fn wing(&self) -> f64 {
        match self {
            Peek::Volume { .. } => 108.0,
            Peek::Track { .. } => 140.0,
            Peek::Notification { .. } => 200.0,
            Peek::Note { .. } => 140.0,
            Peek::Mode { .. } => 96.0,
            Peek::Osd { level: Some(_), .. } => 108.0,
            Peek::Osd { .. } => 120.0,
            Peek::Spaces { .. } => 112.0,
            Peek::Privacy { .. } => 150.0,
            Peek::Battery { device: Some(_), .. } => 150.0,
            Peek::Battery { .. } => 132.0,
            Peek::Device { .. } => 118.0,
            Peek::Output { .. } => 150.0,
        }
    }

    pub fn height(&self) -> f64 {
        match self {
            Peek::Notification { body, actions, .. } if body.is_empty() => 60.0 + if actions.is_empty() { 0.0 } else { 46.0 },
            Peek::Notification { actions, .. } => 78.0 + if actions.is_empty() { 0.0 } else { 46.0 },
            _ => 0.0,
        }
    }

    pub fn radius(&self) -> f64 {
        match self {
            Peek::Notification { body, .. } if body.is_empty() => 26.0,
            Peek::Notification { .. } => 32.0,
            _ => 14.0,
        }
    }
}

#[derive(Clone)]
pub struct Notice {
    pub id: u32,
    pub app: String,
    pub summary: String,
    pub body: String,
    pub icon: Option<gdk::Paintable>,
    pub at: i64,
}

#[derive(Clone)]
struct Queued {
    peek: Peek,
    at: i64,
}

/// How the current item exposes time: a normal track, a live stream, or a player (typically a
/// Firefox-family browser) that publishes neither duration nor position.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Timing {
    Timed,
    Live,
    #[default]
    Untimed,
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
    pub tint: Option<Rgb>,
    pub length: f64,
    pub position: f64,
    pub position_at: i64,
    pub counted: bool,
    pub can_seek: bool,
    pub can_next: bool,
    pub can_prev: bool,
    pub timing: Timing,
    /// "YouTube · Zen Browser", "Spotify", …
    pub source: String,
    pub player: String,
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
        let t = (self.position + run).max(0.0);
        if self.length > 0.0 { t.min(self.length) } else { t }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub who: String,
    pub since: i64,
}

pub fn discord_place(title: &str) -> Option<(String, String)> {
    let mut t = title.trim();
    if t.starts_with('(') {
        if let Some(i) = t.find(") ") {
            if t[1..i].chars().all(|c| c.is_ascii_digit() || c == '•') {
                t = &t[i + 2..];
            }
        }
    }
    for tail in [" - Discord", " — Discord", " - Vesktop", " — Vesktop", " - Legcord", " - Equibop"] {
        if let Some(rest) = t.strip_suffix(tail) {
            t = rest;
            break;
        }
    }
    let mut parts: Vec<&str> = t.split(" | ").map(str::trim).filter(|p| !p.is_empty()).collect();
    if parts.first().is_some_and(|p| matches!(*p, "Discord" | "Vesktop")) {
        parts.remove(0);
    }
    let clean = |s: &str| s.chars().filter(|c| !c.is_control()).take(48).collect::<String>();
    match parts.as_slice() {
        [channel, server, ..] => Some((clean(channel), clean(server))),
        [one] if !matches!(*one, "Discord" | "Vesktop" | "Friends" | "Home") => Some((clean(one), String::new())),
        _ => None,
    }
}

/// Space between the screen edge and the floating island.
pub const ISLAND_LIFT: f64 = 4.0;
/// How long a panel in "leave" mode waits after the pointer leaves before it closes.
pub const LEAVE_MS: u64 = 350;

const PALETTE_FADE_US: i64 = 900_000;

pub struct App {
    pub cfg: Config,
    pub blur: Option<blur::Blur>,
    pub palette: Cell<Palette>,
    palette_from: Cell<Palette>,
    palette_since: Cell<i64>,
    motion_reduced: Cell<bool>,
    effects_reduced: Cell<bool>,
    pub clock: RefCell<String>,
    pub date: RefCell<String>,
    pub media: RefCell<Media>,
    pub volume: Cell<Option<(f64, bool)>>,
    pub lights: RefCell<crate::privacy::Lights>,
    pub spaces: RefCell<Vec<crate::compositor::Space>>,
    pub gadgets: RefCell<Vec<crate::devices::Gadget>>,
    pub sni: RefCell<Vec<crate::sni::Item>>,
    pub weather: RefCell<Option<crate::weather::Weather>>,
    pub call_place: RefCell<Option<(String, String)>>,
    pub running: RefCell<Vec<crate::compositor::Running>>,
    pub prefs: RefCell<crate::config::Prefs>,
    roles: Cell<Roles>,
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
    pub retheming: Cell<bool>,
    pub history: RefCell<Vec<Notice>>,
    pub dnd: Cell<bool>,
    pub awake: RefCell<Option<crate::awake::Awake>>,
    pub control: RefCell<Option<crate::control::Control>>,
    pub wallpapers: Rc<wallpaper::Wallpapers>,
    pub apps: Rc<apps::Apps>,
    pub launcher: RefCell<Option<launcher::Launcher>>,
    pub picker: RefCell<Option<picker::Picker>>,
    open_count: Cell<usize>,
    pub call: RefCell<Option<Call>>,
    ticking_call: Cell<bool>,
    pub seen: RefCell<std::collections::HashMap<String, i64>>,
    pub mic_muted: Cell<Option<bool>>,
}

impl App {
    pub fn new(cfg: Config, blur: Option<blur::Blur>) -> Rc<Self> {
        let frosted = blur.as_ref().is_some_and(|b| b.able()) && !kit::effects::reduced();
        let prefs = crate::config::Prefs::from(&cfg);
        let body = Self::body_for(&cfg, &cfg.surface, Roles::default(), frosted, prefs.wide(&cfg));
        let app = Rc::new(Self {
            blur,
            palette: Cell::new(Palette::new(Roles::default(), body)),
            palette_from: Cell::new(Palette::new(Roles::default(), body)),
            palette_since: Cell::new(0),
            motion_reduced: Cell::new(kit::motion::reduced()),
            effects_reduced: Cell::new(kit::effects::reduced()),
            cfg,
            clock: RefCell::new(String::new()),
            date: RefCell::new(String::new()),
            media: RefCell::new(Media::default()),
            volume: Cell::new(None),
            lights: RefCell::new(crate::privacy::Lights::default()),
            spaces: RefCell::new(Vec::new()),
            gadgets: RefCell::new(Vec::new()),
            sni: RefCell::new(Vec::new()),
            weather: RefCell::new(None),
            call_place: RefCell::new(None),
            running: RefCell::new(Vec::new()),
            prefs: RefCell::new(prefs),
            roles: Cell::new(Roles::default()),
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
            retheming: Cell::new(false),
            history: RefCell::new(Vec::new()),
            dnd: Cell::new(kit::theme::state_dir().join("dnd").exists()),
            awake: RefCell::new(None),
            control: RefCell::new(None),
            wallpapers: Rc::new(wallpaper::Wallpapers::default()),
            apps: apps::Apps::new(),
            launcher: RefCell::new(None),
            picker: RefCell::new(None),
            open_count: Cell::new(0),
            call: RefCell::new(None),
            ticking_call: Cell::new(false),
            seen: RefCell::new(std::collections::HashMap::new()),
            mic_muted: Cell::new(None),
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

    fn tinted(cfg: &Config, roles: Roles) -> Rgb {
        roles.surface_high.mix(roles.accent_container, 1.0 - cfg.tint.clamp(0.0, 1.0))
    }

    fn surface(cfg: &Config, surface: &str, roles: Roles) -> Rgb {
        if surface == "tinted" { Self::tinted(cfg, roles) } else { Rgb::new(0.0, 0.0, 0.0) }
    }

    fn body_for(cfg: &Config, surface: &str, roles: Roles, frosted: bool, wide: bool) -> Rgb {
        if wide {
            return Self::surface(cfg, surface, roles);
        }
        match cfg.dock {
            Dock::Bar => cfg
                .bar_color
                .as_deref()
                .and_then(Rgb::hex)
                .unwrap_or_else(|| roles.surface_high.mix(roles.surface, 0.47)),
            Dock::Screen if frosted => Self::surface(cfg, surface, roles).alpha(1.0 - cfg.glass.clamp(0.0, 0.6)),
            Dock::Screen => Self::surface(cfg, surface, roles),
        }
    }

    pub fn set_roles(&self, roles: Roles) {
        let frosted = self.blur.as_ref().is_some_and(|b| b.able()) && !kit::effects::reduced();
        self.roles.set(roles);
        let (surface, wide) = {
            let p = self.prefs.borrow();
            (p.surface.clone(), p.wide(&self.cfg))
        };
        let next = Palette::new(roles, Self::body_for(&self.cfg, &surface, roles, frosted, wide));
        let now = Self::now();
        let shown = if self.notches.borrow().is_empty() { next } else { self.palette_at(now) };
        self.palette_from.set(shown);
        self.palette_since.set(now);
        self.palette.set(next);
        if let Some(launcher) = self.launcher.borrow().as_ref() {
            launcher.refresh_palette();
        }
        crate::gamebar::appearance_changed();
        crate::paint::refresh_palette(if kit::motion::reduced() { now } else { now + PALETTE_FADE_US });
        let notches: Vec<_> = self.notches.borrow().iter().cloned().collect();
        for n in notches {
            n.refresh();
        }
        self.redraw();
    }

    pub fn palette_at(&self, now: i64) -> Palette {
        if kit::motion::reduced() {
            return self.palette.get();
        }
        let t = ((now - self.palette_since.get()) as f64 / PALETTE_FADE_US as f64).clamp(0.0, 1.0);
        self.palette_from.get().lerp(&self.palette.get(), kit::motion::IN_OUT_CUBIC.at(t))
    }

    pub fn palette_moving(&self, now: i64) -> bool {
        !kit::motion::reduced() && now - self.palette_since.get() < PALETTE_FADE_US
    }

    pub fn now() -> i64 {
        glib::monotonic_time()
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

    pub fn trim() {
        glib::timeout_add_local_once(Duration::from_millis(400), || unsafe {
            libc::malloc_trim(0);
        });
    }

    pub fn set_dnd(&self, on: bool) {
        self.dnd.set(on);
        let flag = kit::theme::state_dir().join("dnd");
        if on {
            let _ = std::fs::create_dir_all(kit::theme::state_dir());
            let _ = std::fs::write(flag, "");
        } else {
            let _ = std::fs::remove_file(flag);
        }
    }

    pub fn set_lights(self: &Rc<Self>, change: impl FnOnce(&mut crate::privacy::Lights)) {
        use crate::privacy::Light;
        let before = self.lights.borrow().clone();
        change(&mut self.lights.borrow_mut());
        let after = self.lights.borrow().clone();
        if after == before {
            return;
        }
        self.changed();
        self.history_changed();
        self.spot_call();
        let calling = |n: &String| self.prefs.borrow().calls(n);
        let started = if let Some(who) = after.mic.iter().find(|n| !before.mic.contains(n) && !calling(n)) {
            Some((Light::Mic, who.clone()))
        } else if before.camera.is_empty() && !after.camera.is_empty() {
            Some((Light::Camera, "In use".into()))
        } else if after.record.is_some() && after.record != before.record {
            let who = match after.record {
                Some(crate::gsr::Mode::Replay) => "Instant replay on",
                Some(crate::gsr::Mode::Stream) => "Live",
                _ => "Started",
            };
            Some((Light::Record, who.into()))
        } else if before.screen == 0 && after.screen > 0 && after.record.is_none() {
            Some((Light::Screen, "Started".into()))
        } else {
            None
        };
        if let Some((light, who)) = started {
            self.request(Peek::Privacy { light, who });
        }
    }

    pub fn wide(&self) -> bool {
        self.prefs.borrow().wide(&self.cfg)
    }

    /// Whether the top bar is drawn beside the notch: always on a screen-docked notch. The Menu
    /// bar setting only decides whether the focused app's name and menus are in it.
    pub fn menubar(&self) -> bool {
        self.cfg.dock == Dock::Screen
    }

    pub fn app_menus(&self) -> bool {
        self.menubar() && self.prefs.borrow().menubar
    }

    pub fn floating(&self) -> bool {
        self.cfg.dock == Dock::Screen && self.prefs.borrow().style == "dynamic"
    }

    pub fn dock(&self) -> Dock {
        self.cfg.dock
    }

    /// The Roundness setting as a factor for panel corners (1.0 is the default look).
    pub fn round(&self) -> f64 {
        self.prefs.borrow().ui_round.clamp(0.1, 1.8)
    }

    /// The notch's corner factor: the app roundness, or its own value when overridden.
    pub fn notch_round(&self) -> f64 {
        let (follow, own) = {
            let p = self.prefs.borrow();
            (p.notch_round_follow, p.notch_round)
        };
        if follow { self.round() } else { own.clamp(0.1, 1.8) }
    }

    /// Game Bar is an app surface, so use the Apps accent and card color.
    pub fn gamebar_palette(&self, mut palette: Palette) -> Palette {
        let prefs = self.prefs.borrow();
        if let Some((accent, on_accent)) = kit::style::custom_accent(&prefs.ui_accent) {
            palette.accent = accent;
            palette.on_accent = Rgb::hex(on_accent).expect("valid accent text color");
        }
        palette.body = kit::style::card_color(&prefs.ui_surface, palette.accent);
        palette
    }

    /// Screen edges and corners stay quiet while a fullscreen app is up, unless the user allows them.
    pub fn edges_live(&self, output: &str) -> bool {
        self.prefs.borrow().fullscreen_edges || !kit::wm::fullscreen_on(output)
    }

    pub fn header(&self) -> f64 {
        if self.wide() { self.prefs.borrow().wide_height.clamp(26.0, 64.0).round() } else { self.cfg.header() }
    }

    /// How far the floating island sits below the screen edge. Fixed: the gap settings only
    /// space windows, and the compositor adds its own gap under the island.
    pub fn lift(&self) -> f64 {
        ISLAND_LIFT
    }

    pub fn reserve(&self) -> i32 {
        if self.wide() {
            self.header() as i32
        } else if self.floating() {
            // The island, plus whatever keeps windows at least the Tight gap below it; the
            // compositor's own gap does the rest.
            (self.header() + self.lift() + self.prefs.borrow().under) as i32
        } else {
            self.cfg.reserve()
        }
    }

    fn spot_call(self: &Rc<Self>) {
        let who = {
            let prefs = self.prefs.borrow();
            let lights = self.lights.borrow();
            let pool = if prefs.call == "any" { &lights.mic } else { &lights.talk };
            pool.iter().find(|n| prefs.calls(n)).cloned()
        };
        let before = self.call.borrow().clone();
        let next = match (who, before.clone()) {
            (None, _) => None,
            (Some(who), Some(c)) => Some(Call { who, since: c.since }),
            (Some(who), None) => Some(Call { who, since: Self::now() }),
        };
        if next == before {
            return;
        }
        let live = next.is_some();
        *self.call.borrow_mut() = next;
        let wants = self.media.borrow().wings(Self::now()) || live;
        spectrum::want(self, wants);
        if !live {
            *self.call_place.borrow_mut() = None;
        }
        if live && !self.ticking_call.replace(true) {
            let weak = Rc::downgrade(self);
            let beat = Cell::new(0u32);
            self.find_place();
            glib::timeout_add_local(Duration::from_millis(1000), move || {
                let Some(app) = weak.upgrade() else { return glib::ControlFlow::Break };
                if app.call.borrow().is_none() {
                    app.ticking_call.set(false);
                    return glib::ControlFlow::Break;
                }
                beat.set(beat.get() + 1);
                if beat.get() % 4 == 0 {
                    app.find_place();
                }
                app.redraw();
                glib::ControlFlow::Continue
            });
        }
        self.changed();
    }

    fn find_place(self: &Rc<Self>) {
        let Some(who) = self.call.borrow().as_ref().map(|c| c.who.to_lowercase()) else { return };
        if !["discord", "vesktop", "legcord", "equibop", "webcord", "armcord"].iter().any(|k| who.contains(k)) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let found = gtk::gio::spawn_blocking(|| {
                kit::wm::windows().into_iter().find(|w| {
                    let id = w.app_id.to_lowercase();
                    ["discord", "vesktop", "legcord", "equibop", "webcord", "armcord"].iter().any(|k| id.contains(k))
                }).map(|w| w.title)
            })
            .await
            .ok()
            .flatten();
            let Some(app) = weak.upgrade() else { return };
            let place = found.as_deref().and_then(discord_place);
            if *app.call_place.borrow() != place {
                *app.call_place.borrow_mut() = place;
                app.redraw();
            }
        });
    }

    pub fn apply_prefs(self: &Rc<Self>, next: crate::config::Prefs) {
        let before = self.prefs.replace(next.clone());
        kit::appearance::apply_gtk();
        kit::motion::refresh();
        let motion = kit::motion::reduced();
        let motion_changed = self.motion_reduced.replace(motion) != motion;
        kit::effects::invalidate();
        let effects = kit::effects::reduced();
        let effects_changed = self.effects_reduced.replace(effects) != effects;
        if motion_changed {
            if kit::motion::reduced() {
                self.palette_since.set(0);
            }
            let wants = self.media.borrow().wings(Self::now()) || self.call.borrow().is_some();
            spectrum::want(self, wants);
            self.redraw_wings();
            self.redraw();
        }
        if motion_changed || before.smooth_caret != next.smooth_caret {
            if let Some(launcher) = self.launcher.borrow().as_ref() {
                launcher.refresh_caret();
            }
        }
        let restyle = before.style != next.style || before.under != next.under || before.menubar != next.menubar || before.bar_spaces != next.bar_spaces || (next.wide(&self.cfg) && before.wide_height != next.wide_height);
        if before.surface != next.surface || restyle || effects_changed {
            self.set_roles(self.roles.get());
        }
        // Roundness changes reshape the notch at once (restyle retargets its corner radius) and
        // redraw any open panel, rather than waiting for the next open or close.
        let rounding = before.ui_round != next.ui_round || before.notch_round_follow != next.notch_round_follow || before.notch_round != next.notch_round;
        if restyle || rounding {
            let notches: Vec<notch::Notch> = self.notches.borrow().iter().cloned().collect();
            for n in notches {
                n.restyle();
            }
        }
        if rounding {
            self.redraw();
        }
        if rounding || before.ui_surface != next.ui_surface || before.ui_accent != next.ui_accent {
            crate::gamebar::appearance_changed();
        }
        if before.ui_surface != next.ui_surface || before.ui_accent != next.ui_accent {
            crate::theming::restyle();
        }
        if before.ui_round != next.ui_round {
            // Window corners in a full Hyprland setup follow the same Corners setting.
            glib::spawn_future_local(async {
                if let Ok(Err(e)) = gtk::gio::spawn_blocking(kit::setup::apply).await {
                    eprintln!("digit: {e}");
                }
            });
        }
        if effects_changed {
            crate::desk::changed(self);
            crate::batterywidget::changed();
            crate::lock::appearance_changed();
        }
        if before.menubar_look != next.menubar_look {
            self.changed();
        }
        if (before.idle_lock, before.idle_off, before.idle_sleep) != (next.idle_lock, next.idle_off, next.idle_sleep) {
            crate::idle::apply(self);
        }
        if (&before.night_light, before.night_temp, &before.night_from, &before.night_to) != (&next.night_light, next.night_temp, &next.night_from, &next.night_to) {
            crate::night::apply(self);
        }
        if (&before.weather_city, &before.weather_unit) != (&next.weather_city, &next.weather_unit) {
            crate::weather::start(self);
        }
        if before.desktop_widget != next.desktop_widget {
            crate::desk::apply(self);
        }
        crate::deskicons::apply(self);
        if (&before.desktop_battery, &before.desktop_widget) != (&next.desktop_battery, &next.desktop_widget) {
            crate::batterywidget::apply(self);
        }
        if before.screen_corners != next.screen_corners {
            crate::corners::apply(self);
        }
        if before.mic_guard != next.mic_guard || before.call_apps != next.call_apps {
            crate::audio::ask(crate::audio::Ask::Inputs);
        }
        if before.call != next.call || before.call_apps != next.call_apps {
            self.spot_call();
        }
        if before.icon_theme != next.icon_theme {
            crate::icons::theme(next.icon_theme.as_deref());
        }
        if before.dock != next.dock || before.icon_theme != next.icon_theme || before.corners != next.corners {
            crate::dock::apply(self);
        }
        if before.corners != next.corners {
            for notch in self.notches.borrow().iter() {
                notch.refresh_corners();
            }
        }
        if before.corners != next.corners || restyle || rounding || before.fullscreen_edges != next.fullscreen_edges
            || before.screen_corners != next.screen_corners || before.ui_surface != next.ui_surface || before.ui_accent != next.ui_accent {
            crate::panel_corner::refresh(self);
        }
    }

    pub fn set_running(&self, list: Vec<crate::compositor::Running>) {
        let focus = |l: &[crate::compositor::Running]| l.iter().find(|r| r.focused).map(|r| r.app_id.clone());
        let moved = focus(&self.running.borrow()) != focus(&list);
        let now_focused = focus(&list);
        *self.running.borrow_mut() = list;
        if moved {
            if let Some(id) = now_focused.filter(|i| !i.is_empty()) {
                let stamp = glib::real_time() / 1_000_000;
                let list = self.apps.list();
                let mut names = vec![id.to_lowercase(), self.apps.display_name(&id).to_lowercase()];
                if let Some(e) = self.apps.find(&id).and_then(|i| list.get(i)) {
                    names.push(e.name.to_lowercase());
                }
                let mut seen = self.seen.borrow_mut();
                for n in names {
                    seen.insert(n, stamp);
                }
                drop(seen);
                crate::dock::badges(self);
            }
        }
        crate::dock::changed();
        if moved && (self.wide() || self.menubar()) {
            for n in self.notches.borrow().iter() {
                n.redraw_strip();
            }
        }
    }

    pub fn set_spaces(self: &Rc<Self>, spaces: Vec<crate::compositor::Space>) {
        if *self.spaces.borrow() == spaces {
            return;
        }
        let was = self.spaces.borrow().iter().find(|s| s.focused).map(|s| s.id);
        let now = spaces.iter().find(|s| s.focused).map(|s| (s.id, s.idx));
        *self.spaces.borrow_mut() = spaces;
        if let (Some(before), Some((id, idx))) = (was, now) {
            // With a menu bar the workspace row opens beside the logo instead.
            if before != id && self.call.borrow().is_some() && !(self.wide() || self.menubar()) {
                self.request(Peek::Spaces { idx });
            }
        }
        self.changed();
        crate::dock::settle_all();
        crate::spaces::changed(self);
    }

    pub fn set_gadgets(&self, list: Vec<crate::devices::Gadget>) {
        *self.gadgets.borrow_mut() = list;
        crate::batterywidget::changed();
        if let Some(c) = self.control.borrow().as_ref() {
            c.changed();
        }
        for n in self.notches.borrow().iter() {
            n.restrip();
        }
    }

    pub fn history_changed(&self) {
        crate::dock::badges(self);
        if let Some(c) = self.control.borrow().as_ref() {
            c.changed();
        }
    }

    pub fn redraw_wings(&self) {
        for n in self.notches.borrow().iter() {
            n.redraw_wings();
        }
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
                let ms = match &peek {
                    Peek::Notification { id, .. } => self.notify.borrow().as_ref().and_then(|n| n.timeout(*id)).unwrap_or_else(|| peek.hold()),
                    _ => peek.hold(),
                };
                self.hold(ms);
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
        let ms = match &peek {
            Peek::Notification { id, .. } => self.notify.borrow().as_ref().and_then(|n| n.timeout(*id)).unwrap_or_else(|| peek.hold()),
            _ => peek.hold(),
        };
        self.hold(ms);
        self.changed();
    }

    fn hold(self: &Rc<Self>, ms: u64) {
        let generation = self.shown_gen.get() + 1;
        self.shown_gen.set(generation);
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(ms), move || {
            let Some(app) = weak.upgrade() else { return };
            if app.shown_gen.get() != generation {
                return;
            }
            let reading = matches!(&*app.peek.borrow(), Some(Peek::Notification { .. })) && app.notches.borrow().iter().any(|n| n.hovered());
            if reading {
                app.hold(1200);
            } else {
                app.dismiss(Ending::Expired);
            }
        });
    }

    pub fn dismiss(self: &Rc<Self>, how: Ending) {
        self.shown_gen.set(self.shown_gen.get() + 1);
        let ended = self.peek.borrow_mut().take();
        if let Some(Peek::Notification { id, .. }) = ended {
            let notify = self.notify.borrow().clone();
            if let Some(n) = notify {
                match how {
                    Ending::Expired => n.closed(id, 1),
                    Ending::Clicked => {
                        n.clicked(id);
                        self.history.borrow_mut().retain(|x| x.id != id);
                        self.history_changed();
                    }
                    Ending::Closed => {}
                    Ending::Action(key) => {
                        n.invoke(id, &key);
                        self.history.borrow_mut().retain(|x| x.id != id);
                        self.history_changed();
                    }
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
        if open { crate::deskicons::release_keyboard(); }
        let count = self.open_count.get();
        self.open_count.set(if open { count + 1 } else { count.saturating_sub(1) });
        if let Some(p) = self.players.borrow().as_ref() {
            p.poll_position(self.open_count.get() > 0);
        }
    }

    pub fn media_changed(self: &Rc<Self>) {
        let wants = self.media.borrow().wings(Self::now()) || self.call.borrow().is_some();
        spectrum::want(self, wants);
        if !self.media.borrow().playing && self.media.borrow().bus.is_some() {
            let weak = Rc::downgrade(self);
            glib::timeout_add_local_once(Duration::from_millis(3100), move || {
                if let Some(app) = weak.upgrade() {
                    let wants = app.media.borrow().wings(Self::now()) || app.call.borrow().is_some();
                    spectrum::want(&app, wants);
                    app.changed();
                }
            });
        }
        self.changed();
    }
}

#[derive(Clone)]
pub enum Ending {
    Expired,
    Clicked,
    Closed,
    Action(String),
}
