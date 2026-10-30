use super::{Control, Hit, State, Tile, fill, rounded};
use crate::draw::{self, Face};
use futures_util::{FutureExt, StreamExt};
use gtk::{gdk, glib, graphene, pango, prelude::*};
use kit::motion::Curve;
use kit::palette::Palette;
use kit::{bluez, nm};
use std::rc::Rc;
use std::time::Duration;

const OPEN: Curve = Curve::new(0.16, 1.0, 0.3, 1.0);
const SHUT: Curve = Curve::new(0.4, 0.0, 0.9, 0.6);
const TOP: f64 = 10.0;
const INSET: f64 = 8.0;
const ROW: f64 = 48.0;
const SUB: f64 = 28.0;
const NOTE: f64 = 40.0;
const INFO: f64 = 18.0;
const FIELD: f64 = 48.0;
const FOOT: f64 = 42.0;
const NEARBY: usize = 6;
const NETS: usize = 8;

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum D {
    Expand(Tile),
    Visible,
    Scan,
    WifiScan,
    Device(usize),
    Trust(usize),
    Remove(usize),
    Net(usize),
    Forget(usize),
    Disconnect,
    Field,
    Reveal,
    Join,
    More,
}

/// The password being typed for a network, right under its row.
pub(super) struct Pass {
    ssid: String,
    text: String,
    shown: bool,
}

#[derive(Clone, Copy)]
enum Item {
    BtNote,
    Device(usize),
    Nearby,
    Searching,
    WifiNote,
    Current,
    Info(u8),
    Net(usize),
    Field,
    Looking,
    Foot,
}

fn height(i: Item) -> f64 {
    match i {
        Item::BtNote | Item::WifiNote | Item::Searching | Item::Looking => NOTE,
        Item::Device(_) | Item::Net(_) | Item::Current => ROW,
        Item::Info(_) => INFO,
        Item::Nearby => SUB,
        Item::Field => FIELD,
        Item::Foot => FOOT,
    }
}

impl State {
    /// Devices of the chosen adapter: paired ones first, then a few named ones nearby.
    fn bt_devices(&self) -> (Vec<bluez::Device>, usize) {
        let Some(a) = self.bt.chosen() else { return (Vec::new(), 0) };
        let mut list: Vec<bluez::Device> = self.bt.on(&a.path).filter(|d| d.paired || d.connected).cloned().collect();
        let paired = list.len();
        list.extend(self.bt.on(&a.path).filter(|d| !d.paired && !d.connected && d.named).take(NEARBY).cloned());
        (list, paired)
    }

    fn nets(&self) -> Vec<nm::Network> {
        self.wifi.networks.iter().filter(|n| !n.active).take(NETS).cloned().collect()
    }

    fn items(&self, t: Tile) -> Vec<Item> {
        let mut v = Vec::new();
        if t == Tile::Bluetooth {
            match self.bt.chosen() {
                None => v.push(Item::BtNote),
                Some(a) => {
                    let (list, paired) = self.bt_devices();
                    if !a.powered || (paired == 0 && !a.discovering && list.is_empty()) {
                        v.push(Item::BtNote);
                    } else {
                        v.extend((0..paired).map(Item::Device));
                        if a.discovering || list.len() > paired {
                            v.push(Item::Nearby);
                            if list.len() == paired {
                                v.push(Item::Searching);
                            }
                            v.extend((paired..list.len()).map(Item::Device));
                        }
                    }
                }
            }
        } else {
            if self.wifi.device.is_none() || !self.wifi.enabled {
                v.push(Item::WifiNote);
            } else {
                if self.wifi.current.is_some() {
                    v.extend([Item::Current, Item::Info(0), Item::Info(1)]);
                }
                let nets = self.nets();
                if nets.is_empty() {
                    v.push(Item::Looking);
                }
                for (i, n) in nets.iter().enumerate() {
                    v.push(Item::Net(i));
                    if self.pass.as_ref().is_some_and(|p| p.ssid == n.ssid) {
                        v.push(Item::Field);
                    }
                }
            }
        }
        v.push(Item::Foot);
        v
    }

    pub(super) fn detail_height(&self, t: Tile) -> f64 {
        TOP + 2.0 * INSET + self.items(t).into_iter().map(height).sum::<f64>()
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_detail(&self, s: &gtk::Snapshot, now: i64, t: Tile, x0: f64, y: f64, inner: f64, shift: f64, hits: &mut Vec<(Hit, f64, f64, f64, f64)>) -> f64 {
        let reveal = self.reveal.value(now).clamp(0.0, 1.0);
        let full = self.detail_height(t);
        let vis = full * reveal;
        if vis < 0.5 {
            return 0.0;
        }
        let pal = self.app.palette_at(now);
        s.push_clip(&graphene::Rect::new((x0 - 8.0) as f32, y as f32, (inner + 16.0) as f32, vis as f32));
        s.push_opacity(reveal.powf(0.6));
        let top = y + TOP;
        fill(s, &rounded(x0, top, inner, full - TOP, 20.0), pal.ink_main.mix(pal.body, 0.045));
        let (cx, cw) = (x0 + INSET, inner - 2.0 * INSET);
        let mut iy = top + INSET;
        let mut local: Vec<(D, f64, f64, f64, f64)> = Vec::new();
        for item in self.items(t) {
            let h = height(item);
            self.item(s, &pal, t, item, cx, iy, cw, h, &mut local);
            iy += h;
        }
        s.pop();
        s.pop();
        hits.extend(local.into_iter().filter(|(_, _, hy, _, hh)| hy + hh <= y + vis + 0.5).map(|(d, hx, hy, hw, hh)| (Hit::Detail(d), hx, hy + shift, hw, hh)));
        vis
    }

    #[allow(clippy::too_many_arguments)]
    fn small(&self, s: &gtk::Snapshot, pal: &Palette, cx: f64, cy: f64, icon: &str, tint: Option<kit::palette::Rgb>, d: D, hits: &mut Vec<(D, f64, f64, f64, f64)>) {
        let hot = self.hover == Some(Hit::Detail(d));
        let bg = match tint {
            Some(t) => t.alpha(if hot { 0.30 } else { 0.20 }),
            None => pal.ink_main.alpha(if hot { 0.14 } else { 0.07 }),
        };
        fill(s, &rounded(cx - 14.0, cy - 14.0, 28.0, 28.0, 14.0), bg);
        draw::icon(s, &self.canvas, &self.app.cfg, icon, 16.0, tint.is_some(), tint.unwrap_or(pal.ink_sec), cx, cy);
        hits.push((d, cx - 14.0, cy - 14.0, 28.0, 28.0));
    }

    #[allow(clippy::too_many_arguments)]
    fn pill(&self, s: &gtk::Snapshot, pal: &Palette, right: f64, cy: f64, label: &str, accent: bool, d: D, hits: &mut Vec<(D, f64, f64, f64, f64)>) {
        let (canvas, cfg) = (&self.canvas, &self.app.cfg);
        let w = draw::width(canvas, cfg, label, 12.5, Face::Text(560)).ceil() + 24.0;
        let x = right - w;
        let hot = self.hover == Some(Hit::Detail(d));
        let bg = if accent { if hot { pal.accent.mix(pal.on_accent, 0.88) } else { pal.accent } } else { pal.ink_main.alpha(if hot { 0.16 } else { 0.10 }) };
        fill(s, &rounded(x, cy - 14.0, w, 28.0, 14.0), bg);
        draw::text(s, canvas, cfg, label, 12.5, Face::Text(560), if accent { pal.on_accent } else { pal.ink_main }, x, cy, w, pango::Alignment::Center);
        hits.push((d, x, cy - 14.0, w, 28.0));
    }

    fn note(&self, s: &gtk::Snapshot, pal: &Palette, text: &str, x: f64, y: f64, w: f64, h: f64) {
        draw::text(s, &self.canvas, &self.app.cfg, text, 13.0, Face::Text(440), pal.ink_meta, x, y + h / 2.0, w, pango::Alignment::Center);
    }

    #[allow(clippy::too_many_arguments)]
    fn item(&self, s: &gtk::Snapshot, pal: &Palette, t: Tile, item: Item, x: f64, y: f64, w: f64, h: f64, hits: &mut Vec<(D, f64, f64, f64, f64)>) {
        let (canvas, cfg) = (&self.canvas, &self.app.cfg);
        let mid = y + h / 2.0;
        match item {
            Item::BtNote => {
                let text = match self.bt.chosen() {
                    None => "No Bluetooth adapter".to_string(),
                    Some(a) if !a.powered => format!("Bluetooth is off on {}", a.name),
                    Some(_) => "No paired devices yet. Scan to find some.".into(),
                };
                self.note(s, pal, &text, x, y, w, h);
            }
            Item::Nearby => {
                draw::text(s, canvas, cfg, "Nearby devices", 12.5, Face::Text(600), pal.ink_meta, x + 6.0, mid + 2.0, w - 12.0, pango::Alignment::Left);
            }
            Item::Searching => self.note(s, pal, "Searching for devices…", x, y, w, h),
            Item::Device(i) => {
                let (list, _) = self.bt_devices();
                let Some(d) = list.get(i) else { return };
                let paired = d.paired || d.connected;
                let busy = self.busy.contains(&d.path);
                let armed = self.armed.as_ref() == Some(&d.path);
                let hovered = matches!(self.hover, Some(Hit::Detail(D::Device(j) | D::Trust(j) | D::Remove(j))) if j == i);
                if hovered {
                    fill(s, &rounded(x, y + 2.0, w, h - 4.0, 14.0), pal.ink_main.mix(pal.body, 0.07));
                }
                hits.push((D::Device(i), x, y + 2.0, w, h - 4.0));
                fill(s, &rounded(x + 6.0, mid - 16.0, 32.0, 32.0, 16.0), if d.connected { pal.accent.alpha(0.22) } else { pal.raised });
                draw::icon(s, canvas, cfg, bluez::symbol(&d.icon), 18.0, d.connected, if d.connected { pal.accent } else { pal.ink_meta }, x + 22.0, mid);
                let sub = if armed {
                    "Click again to remove".to_string()
                } else if busy {
                    (if !paired { "Pairing…" } else if d.connected { "Disconnecting…" } else { "Connecting…" }).to_string()
                } else if paired {
                    let mut p = vec![if d.connected { "Connected".to_string() } else { "Not connected".to_string() }];
                    if let Some(b) = d.battery {
                        p.push(format!("{b}%"));
                    }
                    if d.trusted {
                        p.push("Trusted".into());
                    }
                    p.join(" · ")
                } else {
                    match d.rssi {
                        Some(r) if r >= -60 => "Strong signal".into(),
                        Some(r) if r >= -75 => "Good signal".into(),
                        Some(_) => "Weak signal".into(),
                        None => d.address.clone(),
                    }
                };
                let right = if !paired { 64.0 } else if hovered || armed { 120.0 } else { 52.0 };
                let tw = w - 50.0 - right;
                draw::text(s, canvas, cfg, &d.name, 14.0, Face::Text(540), pal.ink_main, x + 48.0, y + 17.0, tw, pango::Alignment::Left);
                draw::text(s, canvas, cfg, &sub, 12.5, Face::Text(420), if armed { pal.risk } else { pal.ink_meta }, x + 48.0, y + 33.0, tw, pango::Alignment::Left);
                if !paired {
                    if !busy {
                        self.pill(s, pal, x + w - 6.0, mid, "Pair", true, D::Device(i), hits);
                    }
                    return;
                }
                let (sw_w, sw_h) = (40.0, 24.0);
                let sx = x + w - sw_w - 8.0;
                let sy = mid - sw_h / 2.0;
                let on = d.connected != busy;
                fill(s, &rounded(sx, sy, sw_w, sw_h, sw_h / 2.0), if on { pal.accent } else { pal.ink_main.alpha(0.18) });
                let knob = sw_h - 6.0;
                let kx = if on { sx + sw_w - knob - 3.0 } else { sx + 3.0 };
                fill(s, &rounded(kx, sy + 3.0, knob, knob, knob / 2.0), if on { pal.on_accent } else { pal.ink_main });
                if hovered || armed {
                    self.small(s, pal, sx - 22.0, mid, "delete", armed.then_some(pal.risk), D::Remove(i), hits);
                    self.small(s, pal, sx - 56.0, mid, "verified_user", d.trusted.then_some(pal.accent), D::Trust(i), hits);
                }
            }
            Item::WifiNote => {
                let text = if self.wifi.device.is_none() { "No Wi-Fi hardware" } else if !self.wifi.hardware { "Turned off by a hardware switch" } else { "Wi-Fi is off" };
                self.note(s, pal, text, x, y, w, h);
            }
            Item::Current => {
                let Some(c) = &self.wifi.current else { return };
                fill(s, &rounded(x + 6.0, mid - 16.0, 32.0, 32.0, 16.0), pal.accent.alpha(0.22));
                draw::icon(s, canvas, cfg, nm::bars(c.strength), 18.0, true, pal.accent, x + 22.0, mid);
                let mut meta = vec![c.security.label().to_string(), c.band()];
                if c.bitrate > 0 {
                    meta.push(format!("{} Mb/s", c.bitrate / 1000));
                }
                let tw = w - 50.0 - 110.0;
                draw::text(s, canvas, cfg, &c.ssid, 14.0, Face::Text(600), pal.ink_main, x + 48.0, y + 17.0, tw, pango::Alignment::Left);
                draw::text(s, canvas, cfg, &meta.join(" · "), 12.5, Face::Text(420), pal.ink_meta, x + 48.0, y + 33.0, tw, pango::Alignment::Left);
                self.pill(s, pal, x + w - 6.0, mid, "Disconnect", false, D::Disconnect, hits);
            }
            Item::Info(n) => {
                let Some(c) = &self.wifi.current else { return };
                let text = if n == 0 {
                    let mut p = vec![format!("IP {}", if c.address.is_empty() { "–" } else { &c.address })];
                    if !c.gateway.is_empty() {
                        p.push(format!("Router {}", c.gateway));
                    }
                    p.join("  ·  ")
                } else {
                    let mut p = Vec::new();
                    if !c.dns.is_empty() {
                        p.push(format!("DNS {}", c.dns.iter().take(2).cloned().collect::<Vec<_>>().join(", ")));
                    }
                    p.push(format!("Signal {}%", c.strength));
                    p.join("  ·  ")
                };
                draw::text(s, canvas, cfg, &text, 12.0, Face::Number(440), pal.ink_meta, x + 48.0, mid - 2.0, w - 54.0, pango::Alignment::Left);
            }
            Item::Looking => self.note(s, pal, "Looking for networks…", x, y, w, h),
            Item::Net(i) => {
                let nets = self.nets();
                let Some(n) = nets.get(i) else { return };
                let joining = self.joining.as_ref() == Some(&n.ssid);
                let armed = n.saved.is_some() && self.armed == n.saved;
                let hovered = matches!(self.hover, Some(Hit::Detail(D::Net(j) | D::Forget(j))) if j == i);
                if hovered || self.pass.as_ref().is_some_and(|p| p.ssid == n.ssid) {
                    fill(s, &rounded(x, y + 2.0, w, h - 4.0, 14.0), pal.ink_main.mix(pal.body, 0.07));
                }
                hits.push((D::Net(i), x, y + 2.0, w, h - 4.0));
                draw::icon(s, canvas, cfg, nm::bars(n.strength), 20.0, false, pal.ink_main, x + 22.0, mid);
                let sub = if armed {
                    "Click again to forget".to_string()
                } else if joining {
                    "Connecting…".to_string()
                } else {
                    let mut p = Vec::new();
                    if n.saved.is_some() {
                        p.push("Saved".to_string());
                    }
                    p.push(n.security.label().to_string());
                    p.push(nm::band(n.frequency));
                    p.join(" · ")
                };
                let right = if hovered && n.saved.is_some() || armed { 64.0 } else { 30.0 };
                let tw = w - 50.0 - right;
                draw::text(s, canvas, cfg, &n.ssid, 14.0, Face::Text(520), pal.ink_main, x + 48.0, y + 17.0, tw, pango::Alignment::Left);
                draw::text(s, canvas, cfg, &sub, 12.5, Face::Text(420), if armed { pal.risk } else { pal.ink_meta }, x + 48.0, y + 33.0, tw, pango::Alignment::Left);
                if n.security.locked() {
                    draw::icon(s, canvas, cfg, "lock", 15.0, false, pal.ink_meta, x + w - 16.0, mid);
                }
                if (hovered && n.saved.is_some()) || armed {
                    self.small(s, pal, x + w - 48.0, mid, "delete", armed.then_some(pal.risk), D::Forget(i), hits);
                }
            }
            Item::Field => {
                let Some(p) = &self.pass else { return };
                let fx = x + 44.0;
                let join_w = 60.0;
                let fw = w - 44.0 - join_w - 14.0;
                let fy = mid - 17.0;
                fill(s, &rounded(fx, fy, fw, 34.0, 12.0), pal.ink_main.alpha(0.08));
                let r = rounded(fx, fy, fw, 34.0, 12.0);
                s.push_rounded_clip(&r);
                let shown: String = if p.shown { p.text.clone() } else { "•".repeat(p.text.chars().count()) };
                let room = fw - 12.0 - 36.0;
                if shown.is_empty() {
                    draw::text(s, canvas, cfg, "Password", 13.5, Face::Text(420), pal.ink_faint, fx + 12.0, mid, room, pango::Alignment::Left);
                    fill(s, &rounded(fx + 12.0, mid - 9.0, 2.0, 18.0, 1.0), pal.accent);
                } else {
                    let tw = draw::width(canvas, cfg, &shown, 13.5, Face::Text(480)).ceil();
                    let off = (tw - room).max(0.0);
                    draw::text(s, canvas, cfg, &shown, 13.5, Face::Text(480), pal.ink_main, fx + 12.0 - off, mid, tw + 4.0, pango::Alignment::Left);
                    fill(s, &rounded(fx + 13.0 + tw - off, mid - 9.0, 2.0, 18.0, 1.0), pal.accent);
                }
                s.pop();
                hits.push((D::Field, fx, fy, fw, 34.0));
                let hot = self.hover == Some(Hit::Detail(D::Reveal));
                draw::icon(s, canvas, cfg, if p.shown { "visibility_off" } else { "visibility" }, 17.0, false, if hot { pal.ink_main } else { pal.ink_meta }, fx + fw - 18.0, mid);
                hits.push((D::Reveal, fx + fw - 34.0, fy, 34.0, 34.0));
                self.pill(s, pal, x + w - 6.0, mid, "Join", !p.text.is_empty(), D::Join, hits);
            }
            Item::Foot => {
                fill(s, &rounded(x + 6.0, y + 3.0, w - 12.0, 1.0, 0.5), pal.hairline);
                // Settings sits first; the tile's own actions follow as glyphs.
                let fm = y + 5.0 + (h - 5.0) / 2.0;
                let step = 36.0;
                let mut gx = x + 18.0;
                self.small(s, pal, gx, fm, "settings", None, D::More, hits);
                if t == Tile::Bluetooth {
                    if let Some(a) = self.bt.chosen().filter(|a| a.powered) {
                        gx += step;
                        self.small(s, pal, gx, fm, if a.discoverable { "visibility" } else { "visibility_off" }, a.discoverable.then_some(pal.accent), D::Visible, hits);
                        gx += step;
                        self.small(s, pal, gx, fm, "bluetooth_searching", a.discovering.then_some(pal.accent), D::Scan, hits);
                    }
                } else if self.wifi.enabled && self.wifi.device.is_some() {
                    gx += step;
                    self.small(s, pal, gx, fm, "sync", None, D::WifiScan, hits);
                }
            }
        }
    }
}

impl Control {
    pub(super) fn expand(&self, t: Tile) {
        let now = glib::monotonic_time();
        let opening = {
            let mut s = self.0.borrow_mut();
            let opening = !(s.expand == Some(t) && s.reveal.target() > 0.5);
            s.pass = None;
            s.armed = None;
            s.watch_gen += 1;
            if opening {
                s.expand = Some(t);
                s.reveal.go(1.0, now, 420, OPEN);
            } else {
                s.reveal.go(0.0, now, 240, SHUT);
            }
            opening
        };
        if opening {
            self.resize_with(420, OPEN);
            self.watch(t);
            match t {
                Tile::Bluetooth => crate::bluetooth::agent(&self.0.borrow().app),
                _ => self.wifi_scan(),
            }
        } else {
            self.resize_with(240, SHUT);
            self.stop_scan();
            let weak = Rc::downgrade(&self.0);
            glib::timeout_add_local_once(Duration::from_millis(260), move || {
                let Some(s) = weak.upgrade() else { return };
                let gone = {
                    let mut st = s.borrow_mut();
                    let gone = st.reveal.target() < 0.5 && st.expand.is_some();
                    if gone {
                        st.expand = None;
                    }
                    gone
                };
                if gone {
                    Control(s).changed();
                }
            });
        }
        self.kick();
    }

    /// Re-reads Bluetooth or NetworkManager whenever they signal a change, while the section is open.
    fn watch(&self, t: Tile) {
        let generation = self.0.borrow().watch_gen;
        let weak = Rc::downgrade(&self.0);
        glib::spawn_future_local(async move {
            let Ok(conn) = crate::bus::system().await else { return };
            let stream = if t == Tile::Bluetooth { bluez::changes(&conn).await } else { nm::changes(&conn).await };
            let Some(mut stream) = stream else { return };
            loop {
                let next = stream.next();
                let tick = glib::timeout_future(Duration::from_secs(1));
                let got = match futures_util::future::select(next, Box::pin(tick)).await {
                    futures_util::future::Either::Left((None, _)) => return,
                    futures_util::future::Either::Left((Some(_), _)) => true,
                    futures_util::future::Either::Right(_) => false,
                };
                {
                    let Some(s) = weak.upgrade() else { return };
                    let st = s.borrow();
                    if st.watch_gen != generation || !st.open {
                        return;
                    }
                }
                if !got {
                    continue;
                }
                glib::timeout_future(Duration::from_millis(250)).await;
                while let Some(Some(_)) = stream.next().now_or_never() {}
                let Some(s) = weak.upgrade() else { return };
                Control(s).refresh();
            }
        });
    }

    fn wifi_scan(&self) {
        let Some(device) = self.0.borrow().wifi.device.clone() else { return };
        glib::spawn_future_local(async move {
            if let Ok(conn) = crate::bus::system().await {
                let _ = nm::scan(&conn, &device).await;
            }
        });
    }

    pub(super) fn stop_scan(&self) {
        let Some(path) = self.0.borrow_mut().scanning.take() else { return };
        glib::spawn_future_local(async move {
            if let Ok(conn) = crate::bus::system().await {
                let _ = bluez::scan(&conn, &path, false).await;
            }
        });
    }

    fn note(&self, text: String) {
        let app = self.0.borrow().app.clone();
        app.request(crate::app::Peek::Note { text });
    }

    /// Runs `work` against the system bus, says what went wrong, and re-reads afterwards.
    fn run(&self, what: &'static str, work: impl AsyncFnOnce(zbus::Connection) -> Result<(), String> + 'static) {
        let weak = Rc::downgrade(&self.0);
        glib::spawn_future_local(async move {
            let r = match crate::bus::system().await {
                Ok(conn) => work(conn).await,
                Err(e) => Err(e.to_string()),
            };
            let Some(s) = weak.upgrade() else { return };
            let me = Control(s);
            if let Err(e) = r {
                me.note(format!("{what}: {e}"));
            }
            me.refresh();
        });
    }

    pub(super) fn act(&self, d: D) {
        let armed = self.0.borrow_mut().armed.take();
        let adapter = self.0.borrow().bt.chosen().cloned();
        match d {
            D::Expand(t) => self.expand(t),
            D::Visible => {
                let Some(a) = adapter else { return };
                self.run("Bluetooth", async move |c| bluez::set_adapter(&c, &a.path, "Discoverable", !a.discoverable).await);
            }
            D::Scan => {
                let Some(a) = adapter else { return };
                if a.discovering {
                    self.0.borrow_mut().scanning = Some(a.path.clone());
                    self.stop_scan();
                    return;
                }
                let round = {
                    let mut s = self.0.borrow_mut();
                    s.scanning = Some(a.path.clone());
                    s.scan_round += 1;
                    s.scan_round
                };
                self.run("Couldn't scan", async move |c| bluez::scan(&c, &a.path, true).await);
                let weak = Rc::downgrade(&self.0);
                glib::timeout_add_local_once(Duration::from_secs(30), move || {
                    if let Some(s) = weak.upgrade() {
                        if s.borrow().scan_round == round {
                            Control(s).stop_scan();
                        }
                    }
                });
            }
            D::Device(i) => {
                let Some(dev) = self.0.borrow().bt_devices().0.get(i).cloned() else { return };
                if self.0.borrow().busy.contains(&dev.path) {
                    return;
                }
                self.0.borrow_mut().busy.push(dev.path.clone());
                self.changed();
                let weak = Rc::downgrade(&self.0);
                glib::spawn_future_local(async move {
                    let paired = dev.paired || dev.connected;
                    let r = match crate::bus::system().await {
                        Ok(c) if !paired => bluez::pair(&c, &dev.path).await,
                        Ok(c) => bluez::connect(&c, &dev.path, !dev.connected).await,
                        Err(e) => Err(e.to_string()),
                    };
                    let Some(s) = weak.upgrade() else { return };
                    s.borrow_mut().busy.retain(|b| *b != dev.path);
                    let me = Control(s);
                    if let Err(e) = r {
                        let verb = if !paired { "pair" } else if dev.connected { "disconnect" } else { "connect" };
                        me.note(format!("{} couldn't {verb}: {e}", dev.name));
                    }
                    me.refresh();
                });
            }
            D::Trust(i) => {
                let Some(dev) = self.0.borrow().bt_devices().0.get(i).cloned() else { return };
                self.run("Bluetooth", async move |c| bluez::set_device(&c, &dev.path, "Trusted", !dev.trusted).await);
            }
            D::Remove(i) => {
                let Some(dev) = self.0.borrow().bt_devices().0.get(i).cloned() else { return };
                if armed.as_ref() != Some(&dev.path) {
                    self.0.borrow_mut().armed = Some(dev.path);
                    self.0.borrow().canvas.queue_draw();
                    return;
                }
                let Some(a) = adapter else { return };
                self.run("Couldn't remove", async move |c| bluez::remove(&c, &a.path, &dev.path).await);
            }
            D::WifiScan => self.wifi_scan(),
            D::Disconnect => {
                let Some(dev) = self.0.borrow().wifi.device.clone() else { return };
                self.run("Couldn't disconnect", async move |c| nm::disconnect(&c, &dev).await);
            }
            D::Net(i) => {
                let Some(n) = self.0.borrow().nets().get(i).cloned() else { return };
                if self.0.borrow().joining.is_some() {
                    return;
                }
                if n.saved.is_some() || !n.security.locked() {
                    self.join(n, None);
                } else if n.security == nm::Security::Enterprise {
                    self.note("Enterprise networks need nmtui or nm-connection-editor".into());
                } else {
                    let same = self.0.borrow().pass.as_ref().is_some_and(|p| p.ssid == n.ssid);
                    self.0.borrow_mut().pass = if same { None } else { Some(Pass { ssid: n.ssid, text: String::new(), shown: false }) };
                    self.changed();
                }
            }
            D::Forget(i) => {
                let Some(path) = self.0.borrow().nets().get(i).and_then(|n| n.saved.clone()) else { return };
                if armed.as_ref() != Some(&path) {
                    self.0.borrow_mut().armed = Some(path);
                    self.0.borrow().canvas.queue_draw();
                    return;
                }
                self.run("Couldn't forget", async move |c| nm::forget(&c, &path).await);
            }
            D::Field => {}
            D::Reveal => {
                if let Some(p) = self.0.borrow_mut().pass.as_mut() {
                    p.shown = !p.shown;
                }
                self.0.borrow().canvas.queue_draw();
            }
            D::Join => self.submit(),
            D::More => {
                let (app, t) = {
                    let s = self.0.borrow();
                    (s.app.clone(), s.expand)
                };
                crate::config::settings(if t == Some(Tile::Bluetooth) { "bluetooth" } else { "wifi" }, &app.cfg);
                self.close();
            }
        }
    }

    fn submit(&self) {
        let Some(p) = self.0.borrow_mut().pass.take() else { return };
        if p.text.is_empty() {
            self.0.borrow_mut().pass = Some(p);
            return;
        }
        let Some(n) = self.0.borrow().wifi.networks.iter().find(|n| n.ssid == p.ssid).cloned() else { return };
        self.join(n, Some(p.text));
    }

    fn join(&self, n: nm::Network, password: Option<String>) {
        let Some(device) = self.0.borrow().wifi.device.clone() else { return };
        self.0.borrow_mut().joining = Some(n.ssid.clone());
        self.changed();
        let weak = Rc::downgrade(&self.0);
        glib::spawn_future_local(async move {
            let r = match crate::bus::system().await {
                Ok(c) => nm::join(&c, &device, &n, password.as_deref()).await,
                Err(e) => Err(e.to_string()),
            };
            let Some(s) = weak.upgrade() else { return };
            s.borrow_mut().joining = None;
            let me = Control(s);
            match r {
                Ok(()) => {}
                Err(e) => {
                    me.note(e);
                    if password.is_none() && n.security.locked() && me.0.borrow().open {
                        me.0.borrow_mut().pass = Some(Pass { ssid: n.ssid.clone(), text: String::new(), shown: false });
                    }
                }
            }
            me.refresh();
        });
    }

    /// Keys for the open section: typing into the password field, Escape to fold it back up.
    pub(super) fn key(&self, key: gdk::Key, mods: gdk::ModifierType) -> bool {
        let editing = self.0.borrow().pass.is_some();
        if !editing {
            let open = self.0.borrow().expand.filter(|_| self.0.borrow().reveal.target() > 0.5);
            if key == gdk::Key::Escape {
                if let Some(t) = open {
                    self.expand(t);
                    return true;
                }
            }
            return false;
        }
        match key {
            gdk::Key::Escape => {
                self.0.borrow_mut().pass = None;
                self.changed();
            }
            gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::ISO_Enter => self.submit(),
            gdk::Key::BackSpace => {
                if let Some(p) = self.0.borrow_mut().pass.as_mut() {
                    if mods.contains(gdk::ModifierType::CONTROL_MASK) {
                        p.text.clear();
                    } else {
                        p.text.pop();
                    }
                }
            }
            gdk::Key::v | gdk::Key::V if mods.contains(gdk::ModifierType::CONTROL_MASK) => {
                let Some(display) = gdk::Display::default() else { return true };
                let weak = Rc::downgrade(&self.0);
                glib::spawn_future_local(async move {
                    let Ok(Some(text)) = display.clipboard().read_text_future().await else { return };
                    let Some(s) = weak.upgrade() else { return };
                    if let Some(p) = s.borrow_mut().pass.as_mut() {
                        p.text.push_str(text.lines().next().unwrap_or_default());
                    }
                    s.borrow().canvas.queue_draw();
                });
            }
            _ => {
                let typed = key.to_unicode().filter(|c| !c.is_control() && !mods.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK | gdk::ModifierType::SUPER_MASK));
                if let (Some(c), Some(p)) = (typed, self.0.borrow_mut().pass.as_mut()) {
                    p.text.push(c);
                }
            }
        }
        self.0.borrow().canvas.queue_draw();
        true
    }
}
