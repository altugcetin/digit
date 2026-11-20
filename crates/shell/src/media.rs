use crate::app::{App, Peek, Timing};
use futures_util::StreamExt;
use gtk::{gdk, gio, glib, prelude::*};
use kit::palette::Rgb;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue};

const PREFIX: &str = "org.mpris.MediaPlayer2.";
/// A second player must keep playing this long before it takes the notch from one that is
/// already playing. Filters browser hover previews, ads and buffering blips.
const STEAL_US: i64 = 1_500_000;

#[zbus::proxy(interface = "org.mpris.MediaPlayer2.Player", default_path = "/org/mpris/MediaPlayer2")]
trait Player {
    fn play_pause(&self) -> zbus::Result<()>;
    fn next(&self) -> zbus::Result<()>;
    fn previous(&self) -> zbus::Result<()>;
    fn seek(&self, offset: i64) -> zbus::Result<()>;
    fn set_position(&self, track_id: &ObjectPath<'_>, position: i64) -> zbus::Result<()>;
    #[zbus(signal)]
    fn seeked(&self, position: i64) -> zbus::Result<()>;
    #[zbus(property)]
    fn playback_status(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn metadata(&self) -> zbus::Result<HashMap<String, OwnedValue>>;
    #[zbus(property)]
    fn position(&self) -> zbus::Result<i64>;
    #[zbus(property)]
    fn can_seek(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn can_go_next(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn can_go_previous(&self) -> zbus::Result<bool>;
}

#[zbus::proxy(interface = "org.mpris.MediaPlayer2", default_path = "/org/mpris/MediaPlayer2")]
trait Root {
    #[zbus(property)]
    fn identity(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn desktop_entry(&self) -> zbus::Result<String>;
}

#[derive(Clone, Copy)]
pub enum Command {
    PlayPause,
    Next,
    Previous,
}

#[derive(Clone, Default)]
struct Track {
    status: String,
    title: String,
    artist: String,
    art: String,
    url: String,
    trackid: Option<OwnedObjectPath>,
    length: f64,
    can_seek: bool,
    can_next: bool,
    can_prev: bool,
    /// Browsers never publish durations from Firefox-family engines, so their missing length
    /// says nothing about whether the content is live.
    browser: bool,
    since: i64,
}

struct Entry {
    proxy: PlayerProxy<'static>,
    /// Same player without the property cache: every read is a real D-Bus call, so a duration
    /// that arrives without a change signal is still picked up.
    fresh: PlayerProxy<'static>,
    track: Track,
    app_name: String,
    desktop: String,
    position: f64,
    position_at: i64,
    counted: bool,
    task: glib::JoinHandle<()>,
    generation: u64,
    refreshing: bool,
    refresh_again: bool,
    attempts: u8,
}

struct Inner {
    app: std::rc::Weak<App>,
    entries: HashMap<String, Entry>,
    active: Option<String>,
    pending: HashMap<String, u64>,
    generation: u64,
    art_task: Option<glib::JoinHandle<()>>,
    art_track: Option<(Option<String>, u8, String)>,
    art_cache: Vec<(String, i32, gdk::Texture, Option<Rgb>)>,
    polling: Option<glib::SourceId>,
    recheck: Option<glib::SourceId>,
    refresh: Option<(String, glib::SourceId)>,
    lengths: Vec<(String, f64)>,
    ticks: u32,
}

#[derive(Clone)]
pub struct Players(Rc<RefCell<Inner>>);

impl Players {
    pub fn start(app: &Rc<App>) {
        let players = Players(Rc::new(RefCell::new(Inner {
            app: Rc::downgrade(app),
            entries: HashMap::new(),
            active: None,
            pending: HashMap::new(),
            generation: 0,
            art_task: None,
            art_track: None,
            art_cache: Vec::new(),
            polling: None,
            recheck: None,
            refresh: None,
            lengths: Vec::new(),
            ticks: 0,
        })));
        *app.players.borrow_mut() = Some(players.clone());
        glib::spawn_future_local(async move {
            let mut wait = 1;
            loop {
                if players.0.borrow().app.upgrade().is_none() { break; }
                if let Err(e) = players.run().await {
                    eprintln!("digit: media: {e}");
                }
                let names: Vec<_> = players.0.borrow().entries.keys().cloned().collect();
                players.0.borrow_mut().pending.clear();
                for name in names { players.remove(&name); }
                glib::timeout_future(Duration::from_secs(wait)).await;
                wait = (wait * 2).min(30);
            }
        });
    }

    async fn run(&self) -> zbus::Result<()> {
        let conn = crate::bus::session().await?;
        let dbus = zbus::fdo::DBusProxy::new(&conn).await?;
        let mut owners = dbus.receive_name_owner_changed().await?;
        for name in dbus.list_names().await? {
            if is_player(name.as_str()) {
                self.add(&conn, name.to_string());
            }
        }
        while let Some(signal) = owners.next().await {
            let Ok(args) = signal.args() else { continue };
            let name = args.name().to_string();
            if !is_player(&name) {
                continue;
            }
            if args.new_owner().is_none() {
                self.remove(&name);
            } else {
                self.remove(&name);
                self.add(&conn, name);
            }
        }
        Ok(())
    }

    fn add(&self, conn: &zbus::Connection, name: String) {
        let generation = {
            let mut inner = self.0.borrow_mut();
            inner.generation += 1;
            let generation = inner.generation;
            inner.pending.insert(name.clone(), generation);
            generation
        };
        let this = self.clone();
        let conn = conn.clone();
        glib::spawn_future_local(async move {
            if let Err(e) = this.watch(conn, name.clone(), generation).await {
                eprintln!("digit: {name}: {e}");
            }
        });
    }

    fn remove(&self, name: &str) {
        let gone = {
            let mut inner = self.0.borrow_mut();
            inner.pending.remove(name);
            inner.entries.remove(name)
        };
        if let Some(entry) = gone {
            entry.task.abort();
        }
        self.pick();
    }

    async fn watch(&self, conn: zbus::Connection, name: String, generation: u64) -> zbus::Result<()> {
        let proxy = PlayerProxy::builder(&conn)
            .destination(name.clone())?
            .uncached_properties(&["Position"])
            .build()
            .await?;
        let fresh = PlayerProxy::builder(&conn)
            .destination(name.clone())?
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await?;
        let root = RootProxy::builder(&conn).destination(name.clone())?.build().await?;
        let identity = root.identity().await.unwrap_or_default();
        let desktop = root.desktop_entry().await.unwrap_or_default();
        let app_name = self.0.borrow().app.upgrade().map(|app| player_name(&app, &name, &desktop, &identity)).unwrap_or_default();
        let browser = is_browser(&name, &desktop);
        let mut track = read(&fresh).await;
        track.browser = browser;
        if self.0.borrow().pending.get(&name) != Some(&generation) { return Ok(()); }
        self.0.borrow_mut().pending.remove(&name);
        let this = self.clone();
        let listener = proxy.clone();
        let key = name.clone();
        let task = glib::spawn_future_local(async move {
            enum Change {
                Props,
                Seeked(i64),
            }
            let props = futures_util::stream::select_all([
                listener.receive_playback_status_changed().await.map(|_| ()).boxed_local(),
                listener.receive_metadata_changed().await.map(|_| ()).boxed_local(),
                listener.receive_can_seek_changed().await.map(|_| ()).boxed_local(),
                listener.receive_can_go_next_changed().await.map(|_| ()).boxed_local(),
                listener.receive_can_go_previous_changed().await.map(|_| ()).boxed_local(),
            ]);
            let seeked = match listener.receive_seeked().await {
                Ok(stream) => stream.filter_map(|s| async move { s.args().ok().map(|a| a.position) }).boxed_local(),
                Err(_) => futures_util::stream::empty().boxed_local(),
            };
            let mut all = futures_util::stream::select(props.map(|_| Change::Props), seeked.map(Change::Seeked));
            while let Some(change) = all.next().await {
                match change {
                    Change::Props => {
                        this.refresh(&key, false).await;
                    }
                    Change::Seeked(position) => this.set_position(&key, position as f64 / 1e6, true),
                }
            }
        });
        let desktop = if desktop.is_empty() { name.trim_start_matches(PREFIX).split(".instance").next().unwrap_or_default().to_string() } else { desktop };
        let entry = Entry { proxy, fresh, track: Track::default(), app_name, desktop, position: 0.0, position_at: App::now(), counted: false, task, generation, refreshing: false, refresh_again: false, attempts: 0 };
        if let Some(old) = self.0.borrow_mut().entries.insert(name.clone(), entry) {
            old.task.abort();
        }
        self.update(&name, track);
        Ok(())
    }

    fn update(&self, name: &str, mut track: Track) {
        let now = App::now();
        let resample = {
            let mut inner = self.0.borrow_mut();
            let key = format!("{name}\u{1}{}", if track.url.is_empty() { &track.title } else { &track.url });
            if track.length > 0.0 {
                inner.lengths.retain(|(k, _)| *k != key);
                inner.lengths.push((key, track.length));
                if inner.lengths.len() > 64 {
                    inner.lengths.remove(0);
                }
            } else if let Some((_, known)) = inner.lengths.iter().find(|(k, _)| *k == key) {
                track.length = *known;
            }
            let Some(entry) = inner.entries.get_mut(name) else { return };
            track.since = if track.status == entry.track.status && entry.track.since != 0 { entry.track.since } else { now };
            let new_track = track_key(&track) != track_key(&entry.track);
            let toggled = track.status != entry.track.status;
            if new_track {
                entry.attempts = 0;
                let seen = !entry.track.title.is_empty() && track.status == "Playing";
                entry.position = 0.0;
                entry.position_at = now;
                entry.counted = seen;
            } else if toggled && entry.track.status == "Playing" {
                entry.position += (now - entry.position_at).max(0) as f64 / 1e6;
                entry.position_at = now;
            } else if toggled {
                entry.position_at = now;
            }
            entry.track = track;
            new_track || toggled
        };
        if resample {
            self.sample_position_of(name);
        }
        self.pick();
    }

    /// Which player the notch shows, and how long to wait before re-checking a pending takeover.
    fn choose(inner: &Inner, now: i64, filter: &kit::players::Filter) -> (Option<String>, Option<i64>) {
        // A player the user keeps out of the notch is still watched, just never chosen.
        let shown = |n: &str, e: &Entry| filter.shows(&[n.trim_start_matches(PREFIX), &e.desktop, &e.app_name]);
        let entries = || inner.entries.iter().filter(move |(n, e)| shown(n, e));
        let titled = |e: &Entry| !e.track.title.is_empty();
        let is_playing = |e: &Entry| e.track.status == "Playing";
        let current = inner.active.clone().filter(|n| inner.entries.get(n).is_some_and(|e| shown(n, e)));
        let current_playing = current.as_ref().and_then(|n| inner.entries.get(n).map(|e| (n, e))).filter(|(_, e)| is_playing(e));
        if let Some((cur_name, cur_entry)) = current_playing {
            let challenger = entries()
                .filter(|(n, e)| *n != cur_name && is_playing(e) && titled(e) && e.track.since > cur_entry.track.since)
                .max_by_key(|(_, e)| e.track.since);
            return match challenger {
                Some((n, e)) if now - e.track.since >= STEAL_US => (Some(n.clone()), None),
                Some((_, e)) => (current.clone(), Some(STEAL_US - (now - e.track.since))),
                None => (current.clone(), None),
            };
        }
        let newest_playing = entries()
            .filter(|(_, e)| is_playing(e) && titled(e))
            .max_by_key(|(_, e)| e.track.since)
            .map(|(n, _)| n.clone());
        let current_titled = current.filter(|n| inner.entries.get(n).is_some_and(titled));
        let paused_recent = || {
            entries()
                .filter(|(_, e)| e.track.status == "Paused" && titled(e))
                .max_by_key(|(_, e)| e.track.since)
                .map(|(n, _)| n.clone())
        };
        (newest_playing.or(current_titled).or_else(paused_recent), None)
    }

    /// Chooses the player the notch shows again, after a change of which players may show.
    pub fn pick(&self) {
        let Some(app) = self.0.borrow().app.upgrade() else { return };
        let now = App::now();
        let (active, track, app_name, desktop, position, position_at, counted, wait) = {
            let mut inner = self.0.borrow_mut();
            let filter = app.prefs.borrow().media.clone();
            let (active, wait) = Self::choose(&inner, now, &filter);
            inner.active = active.clone();
            let entry = active.as_ref().and_then(|n| inner.entries.get(n));
            let track = entry.map(|e| e.track.clone()).unwrap_or_default();
            let app_name = entry.map(|e| e.app_name.clone()).unwrap_or_default();
            let desktop = entry.map(|e| e.desktop.clone()).unwrap_or_default();
            let (position, position_at, counted) = entry.map(|e| (e.position, e.position_at, e.counted)).unwrap_or((0.0, now, false));
            (active, track, app_name, desktop, position, position_at, counted, wait)
        };
        if let Some(us) = wait {
            let this = self.clone();
            let id = glib::timeout_add_local_once(Duration::from_micros(us.max(0) as u64 + 20_000), move || {
                this.0.borrow_mut().recheck = None;
                this.pick();
            });
            if let Some(old) = self.0.borrow_mut().recheck.replace(id) {
                old.remove();
            }
        }
        let playing = track.status == "Playing";
        // Some players publish a new track before its duration and never signal the duration
        // afterwards; poll until it shows up so the progress bar does not vanish.
        self.keep_refreshing(active.as_deref().filter(|_| playing && track.length <= 0.0 && !track.browser));
        let track_changed = {
            let (kind, key) = track_key(&track);
            let identity = (active.clone(), kind, key.to_string());
            self.0.borrow_mut().art_track.replace(identity.clone()).as_ref() != Some(&identity)
        };
        let (announce, art_changed) = {
            let mut m = app.media.borrow_mut();
            let was_playing = m.playing;
            let switched = m.bus != active;
            let announce = playing && !track.title.is_empty() && (track.title != m.title || !was_playing && switched);
            let art_changed = track_changed || track.art != m.art_url;
            if was_playing && !playing {
                m.paused_at = Some(now);
            }
            if playing {
                m.paused_at = None;
            }
            m.bus = active.clone();
            m.playing = playing;
            m.title = track.title.clone();
            m.artist = track.artist.clone();
            m.length = track.length;
            m.position = position;
            m.position_at = position_at;
            m.counted = counted || track.length > 0.0;
            m.can_seek = track.can_seek && track.length > 0.0;
            m.can_next = track.can_next;
            m.can_prev = track.can_prev;
            m.timing = timing(&track);
            m.source = source_label(&track.url, &app_name);
            m.player = desktop;
            if art_changed {
                m.art_url = track.art.clone();
                m.art = None;
                m.tint = None;
            }
            if active.is_none() {
                *m = Default::default();
            }
            (announce, art_changed)
        };
        if art_changed {
            self.load_art(&app, track.art.clone());
        }
        if announce {
            app.request(Peek::Track { title: track.title, artist: track.artist });
        }
        app.media_changed();
    }

    async fn refresh(&self, name: &str, poll: bool) {
        let (fresh, browser, generation) = {
            let mut inner = self.0.borrow_mut();
            let Some(e) = inner.entries.get_mut(name) else { return };
            if e.refreshing {
                if !poll { e.refresh_again = true; }
                return;
            }
            if poll {
                if !duration_retry(&e.track, e.attempts) { return; }
                e.attempts += 1;
            }
            e.refreshing = true;
            (e.fresh.clone(), e.track.browser, e.generation)
        };
        loop {
            let mut track = read(&fresh).await;
            track.browser = browser;
            let again = {
                let mut inner = self.0.borrow_mut();
                let Some(e) = inner.entries.get_mut(name).filter(|e| e.generation == generation) else { return };
                std::mem::take(&mut e.refresh_again)
            };
            self.update(name, track);
            if !again { break; }
        }
        if let Some(e) = self.0.borrow_mut().entries.get_mut(name).filter(|e| e.generation == generation) {
            e.refreshing = false;
        }
    }

    fn keep_refreshing(&self, name: Option<&str>) {
        let mut inner = self.0.borrow_mut();
        let name = name.filter(|n| inner.entries.get(*n).is_some_and(|e| duration_retry(&e.track, e.attempts)));
        if inner.refresh.as_ref().map(|(n, _)| n.as_str()) == name {
            return;
        }
        if let Some((_, id)) = inner.refresh.take() {
            id.remove();
        }
        let Some(name) = name.map(str::to_string) else { return };
        let this = self.clone();
        let key = name.clone();
        let id = glib::timeout_add_local(Duration::from_millis(1200), move || {
            let this = this.clone();
            let key = key.clone();
            glib::spawn_future_local(async move { this.refresh(&key, true).await; });
            glib::ControlFlow::Continue
        });
        inner.refresh = Some((name, id));
    }

    fn load_art(&self, app: &Rc<App>, url: String) {
        let old = self.0.borrow_mut().art_task.take();
        if let Some(task) = old { task.abort(); }
        if url.is_empty() {
            return;
        }
        // All art consumers are in notch.rs: 104 logical px at most, plus 20 px miniatures.
        let px = 208 * crate::images::scale();
        let cached = self.0.borrow().art_cache.iter().find(|(u, n, _, _)| *u == url && *n >= px).map(|(_, _, t, c)| (t.clone(), *c));
        if let Some((texture, tint)) = cached {
            let mut m = app.media.borrow_mut();
            m.art = Some(texture);
            m.tint = tint;
            drop(m);
            app.redraw();
            return;
        }
        let this = self.clone();
        let weak = Rc::downgrade(app);
        let task = glib::spawn_future_local(async move {
            let file = gio::File::for_uri(&url);
            let Ok(stream) = file.read_future(glib::Priority::DEFAULT).await else { return };
            let mut data = Vec::new();
            loop {
                let Ok(bytes) = stream.read_bytes_future(64 * 1024, glib::Priority::DEFAULT).await else { return };
                if bytes.is_empty() { break; }
                if !append_art(&mut data, &bytes) { return; }
            }
            let Ok(Some(texture)) = gio::spawn_blocking(move || crate::images::decode(&glib::Bytes::from_owned(data), px)).await else { return };
            let Some(app) = weak.upgrade() else { return };
            if app.media.borrow().art_url != url { return; }
            let tint = tint_of(&texture);
            {
                let mut inner = this.0.borrow_mut();
                inner.art_cache.push((url.clone(), px, texture.clone(), tint));
                if inner.art_cache.len() > 12 {
                    inner.art_cache.remove(0);
                }
            }
            let fits = app.media.borrow().art_url == url;
            if fits {
                let mut m = app.media.borrow_mut();
                m.art = Some(texture);
                m.tint = tint;
                drop(m);
                app.redraw();
            }
        });
        self.0.borrow_mut().art_task = Some(task);
    }

    fn active_entry(&self) -> Option<(String, PlayerProxy<'static>, Track)> {
        let inner = self.0.borrow();
        let name = inner.active.clone()?;
        let e = inner.entries.get(&name)?;
        Some((name, e.proxy.clone(), e.track.clone()))
    }

    pub fn command(&self, cmd: Command) {
        let Some((_, proxy, track)) = self.active_entry() else { return };
        let allowed = match cmd {
            Command::PlayPause => true,
            Command::Next => track.can_next,
            Command::Previous => track.can_prev,
        };
        if !allowed {
            return;
        }
        glib::spawn_future_local(async move {
            let r = match cmd {
                Command::PlayPause => proxy.play_pause().await,
                Command::Next => proxy.next().await,
                Command::Previous => proxy.previous().await,
            };
            if let Err(e) = r {
                eprintln!("digit: media command: {e}");
            }
        });
    }

    /// Jumps the shown player to `fraction` of the current track.
    pub fn seek_to(&self, fraction: f64) {
        let Some((name, proxy, track)) = self.active_entry() else { return };
        if !track.can_seek || track.length <= 0.0 {
            return;
        }
        let target = fraction.clamp(0.0, 1.0) * track.length;
        self.set_position(&name, target, true);
        let target_us = (target * 1e6) as i64;
        let trackid = track.trackid.clone().filter(|t| !t.as_str().ends_with("/NoTrack"));
        glib::spawn_future_local(async move {
            let absolute = match &trackid {
                Some(id) => proxy.set_position(&id.as_ref(), target_us).await,
                None => Err(zbus::Error::Unsupported),
            };
            if absolute.is_err() {
                let current = proxy.position().await.unwrap_or(0);
                if let Err(e) = proxy.seek(target_us - current).await {
                    eprintln!("digit: media seek: {e}");
                }
            }
        });
    }

    fn set_position(&self, name: &str, seconds: f64, real: bool) {
        let now = App::now();
        let (is_active, app, counted) = {
            let mut inner = self.0.borrow_mut();
            let is_active = inner.active.as_deref() == Some(name);
            let mut counted = false;
            if let Some(e) = inner.entries.get_mut(name) {
                e.position = seconds.max(0.0);
                e.position_at = now;
                e.counted |= real;
                counted = e.counted;
            }
            (is_active, inner.app.upgrade(), counted)
        };
        let Some(app) = app else { return };
        if is_active && app.media.borrow().bus.as_deref() == Some(name) {
            {
                let mut m = app.media.borrow_mut();
                m.position = seconds.max(0.0);
                m.position_at = now;
                m.counted = counted || m.length > 0.0;
            }
            app.redraw();
        }
    }

    fn sample_position_of(&self, name: &str) {
        let entry = self.0.borrow().entries.get(name).map(|e| (e.fresh.clone(), e.track.browser));
        let Some((proxy, browser)) = entry else { return };
        let this = self.clone();
        let name = name.to_string();
        glib::spawn_future_local(async move {
            let Ok(position) = proxy.position().await else { return };
            if position <= 0 && browser {
                return;
            }
            this.set_position(&name, position as f64 / 1e6, true);
        });
    }

    pub fn poll_position(&self, on: bool) {
        let mut inner = self.0.borrow_mut();
        if on && inner.polling.is_none() {
            let this = self.clone();
            inner.ticks = 0;
            let this_now = this.clone();
            glib::idle_add_local_once(move || {
                let active = this_now.0.borrow().active.clone();
                if let Some(name) = active {
                    this_now.sample_position_of(&name);
                }
            });
            inner.polling = Some(glib::timeout_add_local(Duration::from_millis(1000), move || {
                let (active, sample) = {
                    let mut inner = this.0.borrow_mut();
                    inner.ticks += 1;
                    (inner.active.clone(), inner.ticks % 5 == 0)
                };
                if let (Some(name), true) = (active, sample) {
                    this.sample_position_of(&name);
                }
                let app = this.0.borrow().app.upgrade();
                if let Some(app) = app { app.redraw(); }
                glib::ControlFlow::Continue
            }));
        } else if !on {
            if let Some(id) = inner.polling.take() {
                id.remove();
            }
        }
    }
}

fn is_browser(bus: &str, desktop: &str) -> bool {
    let id = format!("{bus} {desktop}").to_lowercase();
    ["firefox", "chromium", "chrome", "zen", "brave", "vivaldi", "msedge", "opera", "librewolf", "floorp", "waterfox", "epiphany", "falkon", "qutebrowser"]
        .iter()
        .any(|b| id.contains(b))
}

fn is_player(name: &str) -> bool {
    name.starts_with(PREFIX) && !name.contains("playerctld")
}

/// Human name for a player: its installed app name when we can find it, else a tidy version of
/// its desktop entry, identity or bus name ("firefox.instance_1_501" → "Firefox").
fn player_name(app: &App, bus: &str, desktop: &str, identity: &str) -> String {
    let short = bus.trim_start_matches(PREFIX).split(".instance").next().unwrap_or_default();
    for key in [desktop, short] {
        if key.is_empty() {
            continue;
        }
        if let Some(i) = app.apps.find(key) {
            if let Some(e) = app.apps.list().get(i) {
                return e.name.clone();
            }
        }
    }
    if !desktop.is_empty() {
        return crate::apps::pretty(desktop);
    }
    if !identity.is_empty() {
        return identity.to_string();
    }
    crate::apps::pretty(short)
}

fn host(url: &str) -> Option<&str> {
    let rest = url.split_once("://")?.1;
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.rsplit('@').next()?.split(':').next()?;
    (!host.is_empty()).then(|| host.trim_start_matches("www.").trim_start_matches("m."))
}

fn site_name(url: &str) -> Option<String> {
    let host = host(url)?;
    let known = [
        ("music.youtube.com", "YouTube Music"),
        ("youtube.com", "YouTube"),
        ("youtu.be", "YouTube"),
        ("twitch.tv", "Twitch"),
        ("kick.com", "Kick"),
        ("soundcloud.com", "SoundCloud"),
        ("open.spotify.com", "Spotify"),
        ("music.apple.com", "Apple Music"),
        ("deezer.com", "Deezer"),
        ("tidal.com", "TIDAL"),
        ("bandcamp.com", "Bandcamp"),
        ("vimeo.com", "Vimeo"),
        ("netflix.com", "Netflix"),
    ];
    let named = known.iter().find(|(h, _)| host == *h || host.ends_with(&format!(".{h}"))).map(|(_, n)| n.to_string());
    named.or_else(|| url.starts_with("http").then(|| host.to_string()))
}

fn source_label(url: &str, app_name: &str) -> String {
    match site_name(url) {
        Some(site) if !app_name.is_empty() && site != app_name => format!("{site} · {app_name}"),
        Some(site) => site,
        None => app_name.to_string(),
    }
}

fn track_key(track: &Track) -> (u8, &str) {
    if let Some(id) = track.trackid.as_ref().filter(|id| !id.as_str().ends_with("/NoTrack")) {
        (0, id.as_str())
    } else if !track.url.is_empty() {
        (1, &track.url)
    } else {
        (2, &track.title)
    }
}

fn append_art(data: &mut Vec<u8>, bytes: &[u8]) -> bool {
    if bytes.len() > (16 * 1024 * 1024usize).saturating_sub(data.len()) { return false; }
    data.extend_from_slice(bytes);
    true
}

fn duration_retry(track: &Track, attempts: u8) -> bool {
    attempts < 5 && track.length <= 0.0 && !track.browser
}

/// Live streams report no duration; their title, URL or lack of seeking gives them away.
fn timing(track: &Track) -> Timing {
    if track.length > 0.0 {
        return Timing::Timed;
    }
    let title = track.title.to_lowercase();
    let live_word = title.split(|c: char| !c.is_alphanumeric()).any(|w| matches!(w, "live" | "canlı" | "livestream")) || track.title.contains('🔴');
    let url = track.url.to_lowercase();
    let live_url = ["twitch.tv", "kick.com", "/live", ".m3u8", "icecast", "shoutcast"].iter().any(|k| url.contains(k));
    // A media player (not a browser) with an http(s) source and no duration is a stream.
    let native_stream = !track.browser && (url.starts_with("http://") || url.starts_with("https://"));
    if live_word || live_url || native_stream || (!track.can_seek && !track.title.is_empty()) { Timing::Live } else { Timing::Untimed }
}

async fn read(proxy: &PlayerProxy<'static>) -> Track {
    let status = proxy.playback_status().await.unwrap_or_default();
    let meta = proxy.metadata().await.unwrap_or_default();
    let text = |k: &str| meta.get(k).and_then(|v| String::try_from(v.try_clone().ok()?).ok()).unwrap_or_default();
    let artist = meta
        .get("xesam:artist")
        .and_then(|v| Vec::<String>::try_from(v.try_clone().ok()?).ok())
        .map(|a| a.join(", "))
        .unwrap_or_else(|| text("xesam:artist"));
    let length = meta
        .get("mpris:length")
        .and_then(|v| {
            i64::try_from(v.try_clone().ok()?)
                .ok()
                .or_else(|| u64::try_from(v.try_clone().ok()?).ok().map(|x| x as i64))
                .or_else(|| f64::try_from(v.try_clone().ok()?).ok().map(|x| x as i64))
        })
        .unwrap_or(0);
    let trackid = meta.get("mpris:trackid").and_then(|v| {
        OwnedObjectPath::try_from(v.try_clone().ok()?)
            .ok()
            .or_else(|| String::try_from(v.try_clone().ok()?).ok().and_then(|s| OwnedObjectPath::try_from(s).ok()))
    });
    Track {
        status,
        title: text("xesam:title"),
        artist,
        art: text("mpris:artUrl"),
        url: text("xesam:url"),
        trackid,
        length: length.max(0) as f64 / 1e6,
        can_seek: proxy.can_seek().await.unwrap_or(false),
        can_next: proxy.can_go_next().await.unwrap_or(true),
        can_prev: proxy.can_go_previous().await.unwrap_or(true),
        browser: false,
        since: 0,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn album_art_limit_rejects_extra_bytes_without_growing() {
        let mut data = vec![0; 16 * 1024 * 1024 - 1];
        assert!(super::append_art(&mut data, &[1]));
        assert!(!super::append_art(&mut data, &[2]));
        assert_eq!(data.len(), 16 * 1024 * 1024);
        assert_eq!(data.last(), Some(&1));
    }

    #[test]
    fn duration_retries_are_bounded() {
        let mut track = super::Track::default();
        assert!(super::duration_retry(&track, 0));
        assert!(super::duration_retry(&track, 4));
        assert!(!super::duration_retry(&track, 5));
        track.length = 10.0;
        assert!(!super::duration_retry(&track, 0));
        track.length = 0.0;
        track.browser = true;
        assert!(!super::duration_retry(&track, 0));
    }

    use super::*;

    fn track(title: &str, url: &str, length: f64, can_seek: bool) -> Track {
        Track { title: title.into(), url: url.into(), length, can_seek, ..Default::default() }
    }

    #[test]
    fn track_identity_prefers_id_then_url_then_title() {
        let mut a = track("Same", "file:///one", 1.0, true);
        let mut b = track("Same", "file:///two", 1.0, true);
        assert_ne!(track_key(&a), track_key(&b));
        a.trackid = Some(OwnedObjectPath::try_from("/one").unwrap());
        b.trackid = Some(OwnedObjectPath::try_from("/two").unwrap());
        assert_ne!(track_key(&a), track_key(&b));
        b.trackid = a.trackid.clone();
        assert_eq!(track_key(&a), track_key(&b));
        a.trackid = None;
        a.url.clear();
        assert_eq!(track_key(&a), (2, "Same"));
    }

    #[test]
    fn timing_classification() {
        assert!(matches!(timing(&track("Song", "", 200.0, true)), Timing::Timed));
        let youtube_vod = Track { browser: true, ..track("Mix", "https://www.youtube.com/watch?v=x", 0.0, true) };
        assert!(matches!(timing(&youtube_vod), Timing::Untimed));
        assert!(matches!(timing(&track("🔴 LIVE: lofi radio", "https://www.youtube.com/watch?v=x", 0.0, true)), Timing::Live));
        assert!(matches!(timing(&track("Just chatting", "https://www.twitch.tv/someone", 0.0, true)), Timing::Live));
        assert!(matches!(timing(&track("Radio", "", 0.0, false)), Timing::Live));
        assert!(matches!(timing(&track("Delivery", "", 0.0, true)), Timing::Untimed));
        let radio = Track { browser: false, ..track("Lo-fi radio", "https://stream.example.org/lofi.mp3", 0.0, true) };
        assert!(matches!(timing(&radio), Timing::Live));
        let page = Track { browser: true, ..track("Some clip", "https://news.example.org/clip", 0.0, true) };
        assert!(matches!(timing(&page), Timing::Untimed));
        assert!(is_browser("org.mpris.MediaPlayer2.firefox.instance_1_501", "zen"));
        assert!(!is_browser("org.mpris.MediaPlayer2.spotify", "spotify"));
    }

    #[test]
    fn sources() {
        assert_eq!(source_label("https://www.youtube.com/watch?v=x", "Zen Browser"), "YouTube · Zen Browser");
        assert_eq!(source_label("https://music.youtube.com/watch?v=x", "Firefox"), "YouTube Music · Firefox");
        assert_eq!(source_label("", "Spotify"), "Spotify");
        assert_eq!(source_label("https://open.spotify.com/track/1", "Spotify"), "Spotify");
        assert_eq!(source_label("https://example.org/a", "Chromium"), "example.org · Chromium");
        assert_eq!(host("https://user@m.youtube.com:443/x"), Some("youtube.com"));
    }
}

fn tint_of(texture: &gdk::Texture) -> Option<Rgb> {
    let (w, h) = (texture.width().max(0) as usize, texture.height().max(0) as usize);
    if w == 0 || h == 0 {
        return None;
    }
    let stride = w * 4;
    let mut buf = vec![0u8; stride * h];
    texture.download(&mut buf, stride);
    let step = ((w * h) as f64 / 2500.0).sqrt().max(1.0) as usize;
    let mut bins = [(0.0f64, 0.0f64, 0.0f64, 0.0f64); 12];
    let mut seen = 0.0;
    for y in (0..h).step_by(step) {
        for x in (0..w).step_by(step) {
            let i = y * stride + x * 4;
            let (b, g, r) = (buf[i] as f64 / 255.0, buf[i + 1] as f64 / 255.0, buf[i + 2] as f64 / 255.0);
            let (max, min) = (r.max(g).max(b), r.min(g).min(b));
            seen += 1.0;
            if max < 0.12 || max - min < 0.08 {
                continue;
            }
            let d = max - min;
            let hue = if max == r { ((g - b) / d).rem_euclid(6.0) } else if max == g { (b - r) / d + 2.0 } else { (r - g) / d + 4.0 };
            let weight = (d / max).powi(2) * max;
            let bin = &mut bins[((hue * 2.0) as usize).min(11)];
            bin.0 += r * weight;
            bin.1 += g * weight;
            bin.2 += b * weight;
            bin.3 += weight;
        }
    }
    let best = bins.iter().copied().fold((0.0, 0.0, 0.0, 0.0), |a, b| if b.3 > a.3 { b } else { a });
    (best.3 > seen * 0.015).then(|| Rgb::new(best.0 / best.3, best.1 / best.3, best.2 / best.3))
}
