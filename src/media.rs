use crate::app::{App, Peek};
use futures_util::StreamExt;
use gtk::{gdk, gio, glib, prelude::*};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;
use zbus::zvariant::OwnedValue;

const PREFIX: &str = "org.mpris.MediaPlayer2.";

#[zbus::proxy(interface = "org.mpris.MediaPlayer2.Player", default_path = "/org/mpris/MediaPlayer2")]
trait Player {
    fn play_pause(&self) -> zbus::Result<()>;
    fn next(&self) -> zbus::Result<()>;
    fn previous(&self) -> zbus::Result<()>;
    #[zbus(property)]
    fn playback_status(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn metadata(&self) -> zbus::Result<HashMap<String, OwnedValue>>;
    #[zbus(property)]
    fn position(&self) -> zbus::Result<i64>;
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
    length: f64,
    since: i64,
}

struct Entry {
    proxy: PlayerProxy<'static>,
    track: Track,
    task: glib::JoinHandle<()>,
}

struct Inner {
    app: std::rc::Weak<App>,
    entries: HashMap<String, Entry>,
    active: Option<String>,
    art_cache: Vec<(String, gdk::Texture)>,
    polling: Option<glib::SourceId>,
}

#[derive(Clone)]
pub struct Players(Rc<RefCell<Inner>>);

impl Players {
    pub fn start(app: &Rc<App>) {
        let players = Players(Rc::new(RefCell::new(Inner {
            app: Rc::downgrade(app),
            entries: HashMap::new(),
            active: None,
            art_cache: Vec::new(),
            polling: None,
        })));
        *app.players.borrow_mut() = Some(players.clone());
        glib::spawn_future_local(async move {
            if let Err(e) = players.run().await {
                eprintln!("digit: media: {e}");
            }
        });
    }

    async fn run(&self) -> zbus::Result<()> {
        let conn = zbus::Connection::session().await?;
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
            } else if args.old_owner().is_none() {
                self.add(&conn, name);
            }
        }
        Ok(())
    }

    fn add(&self, conn: &zbus::Connection, name: String) {
        let this = self.clone();
        let conn = conn.clone();
        glib::spawn_future_local(async move {
            if let Err(e) = this.watch(conn, name.clone()).await {
                eprintln!("digit: {name}: {e}");
            }
        });
    }

    fn remove(&self, name: &str) {
        let gone = self.0.borrow_mut().entries.remove(name);
        if let Some(entry) = gone {
            entry.task.abort();
        }
        self.pick();
    }

    async fn watch(&self, conn: zbus::Connection, name: String) -> zbus::Result<()> {
        let proxy = PlayerProxy::builder(&conn)
            .destination(name.clone())?
            .uncached_properties(&["Position"])
            .build()
            .await?;
        let track = read(&proxy).await;
        let this = self.clone();
        let listener = proxy.clone();
        let key = name.clone();
        let task = glib::spawn_future_local(async move {
            let status = listener.receive_playback_status_changed().await.map(|_| ());
            let meta = listener.receive_metadata_changed().await.map(|_| ());
            let mut changes = futures_util::stream::select(status, meta);
            while changes.next().await.is_some() {
                let track = read(&listener).await;
                this.update(&key, track);
            }
        });
        if let Some(old) = self.0.borrow_mut().entries.insert(name.clone(), Entry { proxy, track: Track::default(), task }) {
            old.task.abort();
        }
        self.update(&name, track);
        Ok(())
    }

    fn update(&self, name: &str, mut track: Track) {
        {
            let mut inner = self.0.borrow_mut();
            let Some(entry) = inner.entries.get_mut(name) else { return };
            track.since = if track.status == entry.track.status && entry.track.since != 0 { entry.track.since } else { App::now() };
            entry.track = track;
        }
        self.pick();
    }

    fn pick(&self) {
        let Some(app) = self.0.borrow().app.upgrade() else { return };
        let (active, track) = {
            let mut inner = self.0.borrow_mut();
            let playing = inner
                .entries
                .iter()
                .filter(|(_, e)| e.track.status == "Playing")
                .max_by_key(|(_, e)| e.track.since)
                .map(|(n, _)| n.clone());
            let keep = inner.active.clone().filter(|n| inner.entries.contains_key(n));
            let fallback = inner
                .entries
                .iter()
                .filter(|(_, e)| e.track.status == "Paused")
                .max_by_key(|(_, e)| e.track.since)
                .map(|(n, _)| n.clone());
            let active = playing.or(keep).or(fallback);
            inner.active = active.clone();
            let track = active.as_ref().and_then(|n| inner.entries.get(n)).map(|e| e.track.clone());
            (active, track)
        };
        let track = track.unwrap_or_default();
        let now = App::now();
        let playing = track.status == "Playing";
        let (announce, art_changed) = {
            let mut m = app.media.borrow_mut();
            let was_playing = m.playing;
            let announce = playing && !track.title.is_empty() && (track.title != m.title || !was_playing && m.bus != active);
            let art_changed = track.art != m.art_url;
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
            if art_changed {
                m.art_url = track.art.clone();
                m.art = None;
            }
            if active.is_none() {
                *m = Default::default();
            }
            (announce, art_changed)
        };
        if art_changed {
            self.load_art(&app, track.art.clone());
        }
        if active.is_some() {
            self.sample_position();
        }
        if announce {
            app.request(Peek::Track { title: track.title, artist: track.artist });
        }
        app.media_changed();
    }

    fn load_art(&self, app: &Rc<App>, url: String) {
        if url.is_empty() {
            return;
        }
        let cached = self.0.borrow().art_cache.iter().find(|(u, _)| *u == url).map(|(_, t)| t.clone());
        if let Some(texture) = cached {
            app.media.borrow_mut().art = Some(texture);
            app.redraw();
            return;
        }
        let this = self.clone();
        let weak = Rc::downgrade(app);
        glib::spawn_future_local(async move {
            let file = gio::File::for_uri(&url);
            let Ok((bytes, _)) = file.load_bytes_future().await else { return };
            let Ok(texture) = gdk::Texture::from_bytes(&bytes) else { return };
            let Some(app) = weak.upgrade() else { return };
            {
                let mut inner = this.0.borrow_mut();
                inner.art_cache.push((url.clone(), texture.clone()));
                if inner.art_cache.len() > 12 {
                    inner.art_cache.remove(0);
                }
            }
            let fits = app.media.borrow().art_url == url;
            if fits {
                app.media.borrow_mut().art = Some(texture);
                app.redraw();
            }
        });
    }

    fn active_proxy(&self) -> Option<PlayerProxy<'static>> {
        let inner = self.0.borrow();
        inner.active.as_ref().and_then(|n| inner.entries.get(n)).map(|e| e.proxy.clone())
    }

    pub fn command(&self, cmd: Command) {
        let Some(proxy) = self.active_proxy() else { return };
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

    fn sample_position(&self) {
        let Some(proxy) = self.active_proxy() else { return };
        let weak = self.0.borrow().app.clone();
        glib::spawn_future_local(async move {
            let position = proxy.position().await.unwrap_or(0);
            if let Some(app) = weak.upgrade() {
                let mut m = app.media.borrow_mut();
                m.position = position as f64 / 1e6;
                m.position_at = App::now();
            }
        });
    }

    pub fn poll_position(&self, on: bool) {
        let mut inner = self.0.borrow_mut();
        if on && inner.polling.is_none() {
            let this = self.clone();
            inner.polling = Some(glib::timeout_add_local(Duration::from_millis(1000), move || {
                this.sample_position();
                if let Some(app) = this.0.borrow().app.upgrade() {
                    app.redraw();
                }
                glib::ControlFlow::Continue
            }));
        } else if !on {
            if let Some(id) = inner.polling.take() {
                id.remove();
            }
        }
    }
}

fn is_player(name: &str) -> bool {
    name.starts_with(PREFIX) && !name.contains("playerctld")
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
        .and_then(|v| i64::try_from(v.try_clone().ok()?).ok().or_else(|| u64::try_from(v.try_clone().ok()?).ok().map(|x| x as i64)))
        .unwrap_or(0);
    Track {
        status,
        title: text("xesam:title"),
        artist,
        art: text("mpris:artUrl"),
        length: length as f64 / 1e6,
        since: 0,
    }
}
