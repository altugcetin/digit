use super::{Aim, Kind, State, HEAD, PAD, RED};
use crate::audio::{self, Mix};
use crate::draw::{self, Face};
use crate::gsr::{self, Mode};
use crate::sysmon::{Sampler, Stats};
use gtk::{gsk, pango, prelude::*};
use kit::bluez;
use kit::palette::{Palette, Rgb};

const TAB_H: f64 = 30.0;
const PILL_H: f64 = 32.0;
const ROW: f64 = 54.0;
const BT_ROW: f64 = 50.0;
const AIM_ROW: f64 = 42.0;
/// Width of the label column in the crosshair card; the choices start after it.
const AIM_LABEL: f64 = 62.0;
const AIM_SWATCH: f64 = 44.0;
/// Media card: cover size, and the column the title, artist and controls share.
const ART: f64 = 84.0;
const MEDIA_TEXT: f64 = PAD + ART + 16.0;
const MAX_ROWS: usize = 6;
const GRAPH_H: f64 = 86.0;
const HISTORY: usize = 60;
const AIM_COLORS: [Rgb; 3] = [Rgb::new(0.25, 1.0, 0.45), Rgb::new(1.0, 0.25, 0.3), Rgb::new(1.0, 1.0, 1.0)];

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Rec {
    Region,
    Screen,
    Record,
    Save,
    Replay,
    Pause,
    Folder,
    Settings,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    Bar(Kind),
    /// A part of the toolbar that does nothing, so a click there doesn't close the overlay.
    Bare,
    Head(Kind),
    Body(Kind),
    /// A card's edge or corner: bits 1 left, 2 right, 4 top, 8 bottom.
    Resize(Kind, u8),
    Pin(Kind),
    Close(Kind),
    Rec(Rec),
    ResTab(usize),
    MixTab(usize),
    Level(usize),
    Mute(usize),
    Media(u8),
    BtPower,
    BtDevice(usize),
    AimOn,
    AimStyle(u8),
    AimSize(u8),
    AimColor(u8),
}

pub struct Item {
    pub hit: Hit,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

fn item(hit: Hit, x: f64, y: f64, w: f64, h: f64) -> Item {
    Item { hit, x, y, w, h }
}

#[derive(Default)]
pub struct Data {
    pub beat: u64,
    pub sampler: Sampler,
    pub stats: Stats,
    pub nvidia: Option<f64>,
    /// CPU, RAM, swap and GPU use, oldest first.
    pub history: [Vec<f64>; 4],
    pub res_tab: usize,
    pub mix_tab: usize,
    pub mix: Option<Mix>,
    pub mixing: bool,
    /// The row under the pointer's slider and the level it shows until the server agrees.
    pub sliding: Option<(usize, f64)>,
    pub sent: Option<f64>,
    pub held: bool,
    pub pushing: bool,
    pub bt: bluez::Snapshot,
    pub bt_reading: bool,
    pub bt_busy: Vec<String>,
}

impl Data {
    pub fn push(&mut self, stats: Stats) {
        self.stats = stats;
        let swap = if stats.swap_total > 0.0 { stats.swap_used / stats.swap_total } else { 0.0 };
        for (h, v) in self.history.iter_mut().zip([stats.cpu, stats.mem, swap, stats.gpu.unwrap_or(0.0)]) {
            h.push(v);
            if h.len() > HISTORY {
                h.remove(0);
            }
        }
    }

    /// Row 0 is the device, the rest are app streams.
    pub fn channel(&self, row: usize) -> Option<audio::Channel> {
        let mix = self.mix.as_ref()?;
        if row == 0 { mix.device.clone() } else { mix.apps.get(row - 1).cloned() }
    }

    fn tabs(&self) -> Vec<&'static str> {
        let mut t = vec!["CPU", "RAM", "Swap"];
        if self.stats.gpu.is_some() {
            t.push("GPU");
        }
        t
    }
}

/// Paired devices on the chosen adapter, connected ones first.
pub fn devices(snap: &bluez::Snapshot) -> Vec<bluez::Device> {
    let Some(a) = snap.chosen() else { return Vec::new() };
    let mut list: Vec<bluez::Device> = snap.on(&a.path).filter(|d| d.paired).cloned().collect();
    list.sort_by_key(|d| (!d.connected, d.name.to_lowercase()));
    list.truncate(MAX_ROWS);
    list
}

pub fn head_items(kind: Kind, w: f64) -> Vec<Item> {
    let mid = HEAD / 2.0 + 2.0;
    vec![item(Hit::Pin(kind), w - PAD - 66.0, mid - 15.0, 30.0, 30.0), item(Hit::Close(kind), w - PAD - 30.0, mid - 15.0, 30.0, 30.0)]
}

/// The mixer slider's left edge and width inside the card.
pub fn slider(w: f64) -> (f64, f64) {
    (PAD + 44.0, w - 2.0 * PAD - 44.0 - 44.0)
}

/// The field the notes are typed into, inside the card.
pub fn notes_field(w: f64, h: f64) -> (f64, f64, f64, f64) {
    (PAD, HEAD + 2.0, w - 2.0 * PAD, h - HEAD - 2.0 - PAD)
}

fn mix_rows(d: &Data) -> usize {
    d.mix.as_ref().map_or(0, |m| m.device.is_some() as usize + m.apps.len().min(MAX_ROWS))
}

pub fn height(st: &State, kind: Kind) -> f64 {
    let d = &st.data;
    match kind {
        Kind::Recorder => HEAD + 200.0,
        Kind::Resources => HEAD + 2.0 + TAB_H + 14.0 + GRAPH_H + PAD,
        Kind::Mixer => {
            let apps = d.mix.as_ref().map_or(0, |m| m.apps.len());
            let rows = mix_rows(d) as f64 * ROW + if apps == 0 { 40.0 } else { 0.0 };
            HEAD + 2.0 + TAB_H + 10.0 + rows.max(ROW) + 8.0
        }
        Kind::Media => {
            if st.app.media.borrow().bus.is_some() {
                // Cover, then the progress bar and its times, then the card's own padding.
                HEAD + 2.0 + ART + 18.0 + 4.0 + 22.0 + PAD - 4.0
            } else {
                HEAD + 110.0
            }
        }
        Kind::Bluetooth => {
            let n = devices(&d.bt).len().max(1) as f64;
            HEAD + 2.0 + 44.0 + 6.0 + n * BT_ROW + 10.0
        }
        Kind::Notes => 280.0,
        Kind::Crosshair => HEAD + 2.0 + 44.0 + 3.0 * AIM_ROW + 12.0,
    }
}

fn pill_w(st: &State, text: &str) -> f64 {
    draw::width(&st.paint, &st.app.cfg, text, 13.0, Face::Text(560)).ceil() + 30.0
}

fn tab_items(st: &State, labels: &[&str], make: fn(usize) -> Hit) -> Vec<Item> {
    let mut x = PAD;
    labels
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let w = pill_w(st, l) - 4.0;
            let it = item(make(i), x, HEAD + 2.0, w, TAB_H);
            x += w + 6.0;
            it
        })
        .collect()
}

fn replay_label(st: Option<gsr::Status>) -> &'static str {
    if st.is_some_and(|s| s.mode == Mode::Replay) { "Instant replay on" } else { "Instant replay off" }
}

fn pause_label(st: Option<gsr::Status>) -> Option<&'static str> {
    st.filter(|s| s.mode == Mode::Record || s.taping).map(|s| if s.paused { "Resume" } else { "Pause" })
}

pub fn items(st: &State, kind: Kind, w: f64) -> Vec<Item> {
    let d = &st.data;
    let mut out = Vec::new();
    match kind {
        Kind::Recorder => {
            let bw = (w - 2.0 * PAD - 30.0) / 4.0;
            for (i, r) in [Rec::Region, Rec::Screen, Rec::Record, Rec::Save].into_iter().enumerate() {
                out.push(item(Hit::Rec(r), PAD + i as f64 * (bw + 10.0), HEAD + 24.0, bw, 60.0));
            }
            let status = gsr::status();
            let mut x = PAD;
            for (r, label) in [(Rec::Replay, Some(replay_label(status))), (Rec::Pause, pause_label(status))] {
                let Some(label) = label else { continue };
                let pw = pill_w(st, label) + if r == Rec::Replay { 22.0 } else { 0.0 };
                out.push(item(Hit::Rec(r), x, HEAD + 110.0, pw, PILL_H));
                x += pw + 8.0;
            }
            let fw = pill_w(st, "Open recordings folder") + 22.0;
            out.push(item(Hit::Rec(Rec::Folder), PAD, HEAD + 152.0, fw, PILL_H));
            out.push(item(Hit::Rec(Rec::Settings), PAD + fw + 8.0, HEAD + 152.0, PILL_H + 8.0, PILL_H));
        }
        Kind::Resources => out = tab_items(st, &d.tabs(), Hit::ResTab),
        Kind::Mixer => {
            out = tab_items(st, &["Output", "Input"], Hit::MixTab);
            let (sx, sw) = slider(w);
            for row in 0..mix_rows(d) {
                let y = HEAD + 2.0 + TAB_H + 10.0 + row as f64 * ROW;
                out.push(item(Hit::Level(row), sx - 6.0, y + 26.0, sw + 12.0, 22.0));
                out.push(item(Hit::Mute(row), w - PAD - 36.0, y + 9.0, 36.0, 36.0));
            }
        }
        Kind::Media => {
            if st.app.media.borrow().bus.is_some() {
                // Bottom-aligned with the cover; the first button's glyph lines up with the text above.
                for i in 0..3u8 {
                    out.push(item(Hit::Media(i), MEDIA_TEXT - 8.0 + i as f64 * 46.0, HEAD + 2.0 + ART - 40.0, 40.0, 40.0));
                }
            }
        }
        Kind::Bluetooth => {
            if d.bt.chosen().is_some() {
                out.push(item(Hit::BtPower, w - PAD - 52.0, HEAD + 2.0 + 10.0, 52.0, 26.0));
                let top = HEAD + 2.0 + 44.0 + 6.0;
                for i in 0..devices(&d.bt).len() {
                    out.push(item(Hit::BtDevice(i), PAD - 6.0, top + i as f64 * BT_ROW, w - 2.0 * PAD + 12.0, BT_ROW - 4.0));
                }
            }
        }
        Kind::Notes => {}
        Kind::Crosshair => {
            out.push(item(Hit::AimOn, w - PAD - 52.0, HEAD + 2.0 + 10.0, 52.0, 26.0));
            let top = HEAD + 2.0 + 44.0;
            let chips = |out: &mut Vec<Item>, row: f64, labels: &[&str], make: fn(u8) -> Hit| {
                let mut x = PAD + AIM_LABEL;
                for (i, l) in labels.iter().enumerate() {
                    let cw = pill_w(st, l) - 4.0;
                    out.push(item(make(i as u8), x, top + row * AIM_ROW + 6.0, cw, AIM_ROW - 12.0));
                    x += cw + 6.0;
                }
            };
            chips(&mut out, 0.0, &["Dot", "Cross"], Hit::AimStyle);
            chips(&mut out, 1.0, &["Small", "Medium", "Large"], Hit::AimSize);
            for i in 0..AIM_COLORS.len() {
                out.push(item(Hit::AimColor(i as u8), PAD + AIM_LABEL + 4.0 + i as f64 * AIM_SWATCH, top + 2.0 * AIM_ROW + 6.0, AIM_ROW - 12.0, AIM_ROW - 12.0));
            }
        }
    }
    out
}

struct Ink<'a> {
    st: &'a State,
    s: &'a gtk::Snapshot,
    pal: &'a Palette,
}

impl Ink<'_> {
    fn text(&self, text: &str, px: f64, face: Face, color: Rgb, x: f64, mid: f64, w: f64, align: pango::Alignment) {
        draw::text(self.s, &self.st.paint, &self.st.app.cfg, text, px, face, color, x, mid, w, align);
    }

    fn icon(&self, name: &str, px: f64, fill: bool, color: Rgb, cx: f64, mid: f64) {
        draw::icon(self.s, &self.st.paint, &self.st.app.cfg, name, px, fill, color, cx, mid);
    }

    fn hot(&self, hit: Hit) -> bool {
        self.st.hot == Some(hit)
    }

    /// A rounded label button; `on` fills it with the accent.
    fn pill(&self, it: &Item, label: &str, on: bool) {
        let hot = self.hot(it.hit);
        let back = if on { self.pal.accent } else { self.pal.tile(if hot { 0.14 } else { 0.08 }) };
        draw::fill(self.s, &draw::round(it.x, it.y, it.w, it.h, it.h / 2.0), back);
        self.text(label, 13.0, Face::Text(500), if on { self.pal.on_accent } else { self.pal.ink_main }, it.x, it.y + it.h / 2.0, it.w, pango::Alignment::Center);
    }

    fn switch(&self, it: &Item, on: bool) {
        let (x, y, w, h) = (it.x + 6.0, it.y + 2.0, it.w - 12.0, it.h - 4.0);
        // The same switch as Control Center's device rows.
        let track = if on { self.pal.accent } else { self.pal.ink_main.alpha(if self.hot(it.hit) { 0.24 } else { 0.18 }) };
        draw::fill(self.s, &draw::round(x, y, w, h, h / 2.0), track);
        let k = h - 6.0;
        let kx = if on { x + w - k - 3.0 } else { x + 3.0 };
        draw::fill(self.s, &draw::round(kx, y + 3.0, k, k, k / 2.0), if on { self.pal.on_accent } else { self.pal.ink_main });
    }

    fn tabs(&self, items: &[Item], labels: &[&str], chosen: usize) {
        for (i, (it, l)) in items.iter().zip(labels).enumerate() {
            let on = i == chosen;
            let hot = self.hot(it.hit);
            let back = if on { self.pal.accent } else if hot { self.pal.tile(0.12) } else { self.pal.body.alpha(0.0) };
            draw::fill(self.s, &draw::round(it.x, it.y, it.w, it.h, it.h / 2.0), back);
            self.text(l, 13.0, Face::Text(if on { 600 } else { 500 }), if on { self.pal.on_accent } else { self.pal.ink_meta }, it.x, it.y + it.h / 2.0, it.w, pango::Alignment::Center);
        }
    }

    fn empty(&self, glyph: &str, text: &str, w: f64, top: f64, h: f64) {
        let mid = top + h / 2.0;
        self.icon(glyph, 34.0, false, self.pal.ink_faint, w / 2.0, mid - 14.0);
        self.text(text, 13.0, Face::Text(520), self.pal.ink_meta, 0.0, mid + 20.0, w, pango::Alignment::Center);
    }
}

pub fn draw(st: &State, s: &gtk::Snapshot, kind: Kind, pal: &Palette, w: f64, h: f64) {
    let ink = Ink { st, s, pal };
    let left = pango::Alignment::Left;
    let items = items(st, kind, w);
    let d = &st.data;
    match kind {
        Kind::Recorder => {
            let status = gsr::status();
            let (line, color) = match status {
                None if gsr::busy() => ("Starting…".to_string(), pal.ink_meta),
                None if !gsr::installed() => ("gpu-screen-recorder is not installed".to_string(), RED),
                None => ("Ready".to_string(), pal.ink_meta),
                Some(s) => match (s.mode, s.taping, s.paused) {
                    (Mode::Record, _, true) => (format!("Paused · {}", gsr::clock(s.secs)), RED),
                    (Mode::Record, _, false) | (Mode::Replay, true, _) => (format!("Recording · {}", gsr::clock(s.secs)), RED),
                    (Mode::Replay, false, _) => (format!("Keeping the last {}", gsr::span(s.replay)), pal.accent),
                    (Mode::Stream, _, _) => (format!("Live · {}", gsr::clock(s.secs)), RED),
                },
            };
            ink.text(&line, 13.0, Face::Number(540), color, PAD + 2.0, HEAD + 6.0, w - 2.0 * PAD, left);
            let replay = status.is_some_and(|s| s.mode == Mode::Replay);
            let taping = status.is_some_and(|s| s.mode == Mode::Record || s.taping);
            let running = status.is_some();
            for it in &items {
                let Hit::Rec(r) = it.hit else { continue };
                match r {
                    Rec::Region | Rec::Screen | Rec::Record | Rec::Save => {
                        // Region and Screen start a recording, so they rest while one runs.
                        let live = match r { Rec::Save => replay, Rec::Region | Rec::Screen => !running, _ => true };
                        let on = r == Rec::Record && taping;
                        let glow = if live && ink.hot(it.hit) { 0.07 } else { 0.0 };
                        let back = if on { RED.alpha(0.2 + glow) } else { pal.tile(0.07 + glow) };
                        draw::fill(s, &draw::round(it.x, it.y, it.w, it.h, 20.0 * st.app.round()), back);
                        let (glyph, label) = match r {
                            Rec::Region => ("crop_free", "Region"),
                            Rec::Screen => ("desktop_windows", "Screen"),
                            Rec::Record if on => ("stop", "Stop"),
                            Rec::Record => ("radio_button_checked", "Record"),
                            _ => ("fast_rewind", "Save clip"),
                        };
                        let tint = if r == Rec::Record { RED } else { pal.ink_main };
                        s.push_opacity(if live { 1.0 } else { 0.38 });
                        ink.icon(glyph, 26.0, on, tint, it.x + it.w / 2.0, it.y + it.h / 2.0);
                        ink.text(label, 12.0, Face::Text(520), pal.ink_meta, it.x - 6.0, it.y + it.h + 13.0, it.w + 12.0, pango::Alignment::Center);
                        s.pop();
                    }
                    Rec::Replay => {
                        ink.pill(it, "", replay);
                        let label = replay_label(status);
                        ink.icon("replay", 16.0, false, if replay { pal.on_accent } else { pal.ink_main }, it.x + 18.0, it.y + it.h / 2.0);
                        ink.text(label, 13.0, Face::Text(560), if replay { pal.on_accent } else { pal.ink_main }, it.x + 30.0, it.y + it.h / 2.0, it.w - 34.0, left);
                    }
                    Rec::Pause => ink.pill(it, pause_label(status).unwrap_or(""), false),
                    Rec::Folder => {
                        ink.pill(it, "", false);
                        ink.icon("folder_open", 17.0, false, pal.ink_main, it.x + 19.0, it.y + it.h / 2.0);
                        ink.text("Open recordings folder", 13.0, Face::Text(560), pal.ink_main, it.x + 32.0, it.y + it.h / 2.0, it.w - 36.0, left);
                    }
                    Rec::Settings => {
                        ink.pill(it, "", false);
                        ink.icon("settings", 18.0, false, pal.ink_main, it.x + it.w / 2.0, it.y + it.h / 2.0);
                    }
                }
            }
        }
        Kind::Resources => {
            let tabs = d.tabs();
            let tab = d.res_tab.min(tabs.len() - 1);
            ink.tabs(&items, &tabs, tab);
            let st_ = d.stats;
            let (value, sub) = match tab {
                0 => (st_.cpu, st_.temp.map(|t| format!("{t:.0}°C")).unwrap_or_default()),
                1 => (st_.mem, format!("{:.1} of {:.0} GB", st_.used, st_.total)),
                2 if st_.swap_total > 0.0 => (st_.swap_used / st_.swap_total, format!("{:.1} of {:.0} GB", st_.swap_used, st_.swap_total)),
                2 => (0.0, "No swap".to_string()),
                _ => (st_.gpu.unwrap_or(0.0), String::new()),
            };
            let top = HEAD + 2.0 + TAB_H + 14.0;
            ink.text(&format!("{:.0}%", value * 100.0), 30.0, Face::Number(620), pal.ink_main, PAD + 2.0, top + 24.0, 120.0, left);
            ink.text(&sub, 12.0, Face::Number(500), pal.ink_meta, PAD + 2.0, top + 56.0, 120.0, left);
            let (gx, gw) = (PAD + 128.0, w - 2.0 * PAD - 128.0);
            // The graph takes whatever height the card was given.
            let gh = (h - top - PAD).max(GRAPH_H);
            let frame = draw::round(gx, top, gw, gh, 14.0 * st.app.round());
            draw::fill(s, &frame, pal.ink_main.alpha(0.05));
            let hist = &d.history[tab.min(3)];
            if hist.len() >= 2 {
                s.push_rounded_clip(&frame);
                let step = gw / (HISTORY - 1) as f64;
                let x0 = gx + gw - (hist.len() - 1) as f64 * step;
                let at = |i: usize, v: f64| (x0 + i as f64 * step, top + gh - 6.0 - v.clamp(0.0, 1.0) * (gh - 14.0));
                let line = gsk::PathBuilder::new();
                let area = gsk::PathBuilder::new();
                area.move_to(x0 as f32, (top + gh) as f32);
                for (i, v) in hist.iter().enumerate() {
                    let (x, y) = at(i, *v);
                    if i == 0 {
                        line.move_to(x as f32, y as f32);
                    } else {
                        line.line_to(x as f32, y as f32);
                    }
                    area.line_to(x as f32, y as f32);
                }
                area.line_to((gx + gw) as f32, (top + gh) as f32);
                area.close();
                s.append_fill(&area.to_path(), gsk::FillRule::Winding, &pal.accent.alpha(0.22).rgba());
                s.append_stroke(&line.to_path(), &gsk::Stroke::new(2.0), &pal.accent.rgba());
                s.pop();
            }
        }
        Kind::Mixer => {
            ink.tabs(&items, &["Output", "Input"], d.mix_tab);
            let top = HEAD + 2.0 + TAB_H + 10.0;
            let Some(mix) = d.mix.as_ref() else {
                ink.text(if crate::audio::live() { "Loading…" } else { "Sound isn't available" }, 13.0, Face::Text(500), pal.ink_meta, 0.0, top + ROW / 2.0, w, pango::Alignment::Center);
                return;
            };
            let input = d.mix_tab == 1;
            let (sx, sw) = slider(w);
            let scale = st.paint.scale_factor();
            for row in 0..mix_rows(d) {
                let Some(ch) = d.channel(row) else { continue };
                let y = top + row as f64 * ROW;
                let name = if row == 0 { ch.name.clone() } else { audio::who(&st.app, &ch) };
                if row == 0 {
                    draw::fill(s, &draw::round(PAD, y + 9.0, 34.0, 34.0, 17.0), pal.accent.alpha(0.18));
                    ink.icon(if input { "mic" } else { "speaker" }, 19.0, true, pal.accent, PAD + 17.0, y + 26.0);
                } else {
                    let icon = [ch.name.to_lowercase(), ch.binary.clone()].into_iter().filter(|id| !id.is_empty()).find_map(|id| draw::app_icon(&st.app.apps, &id, 32, scale));
                    match icon {
                        Some(p) => draw::paintable(s, &p, PAD + 1.0, y + 10.0, 32.0),
                        None => draw::monogram(s, &st.paint, &st.app.cfg, &name, PAD + 1.0, y + 10.0, 32.0, pal.ink_main.alpha(0.1), pal.ink_main),
                    }
                }
                let level = match d.sliding {
                    Some((r, v)) if r == row => v,
                    _ => ch.level,
                };
                ink.text(&name, 13.0, Face::Text(if row == 0 { 600 } else { 540 }), pal.ink_main, sx, y + 16.0, sw - 44.0, left);
                ink.text(&format!("{:.0}%", level * 100.0), 12.0, Face::Number(520), pal.ink_meta, sx + sw - 44.0, y + 16.0, 44.0, pango::Alignment::Right);
                let ty = y + 36.0;
                let frac = level.clamp(0.0, 1.0);
                draw::fill(s, &draw::round(sx, ty - 3.0, sw, 6.0, 3.0), pal.ink_main.alpha(0.12));
                let tint = if ch.muted { pal.ink_faint } else { pal.accent };
                draw::fill(s, &draw::round(sx, ty - 3.0, (sw * frac).max(6.0), 6.0, 3.0), tint);
                let knob = if ink.hot(Hit::Level(row)) || matches!(d.sliding, Some((r, _)) if r == row) { 16.0 } else { 14.0 };
                draw::fill(s, &draw::round(sx + sw * frac - knob / 2.0, ty - knob / 2.0, knob, knob, knob / 2.0), tint);
                let mute = Hit::Mute(row);
                if ink.hot(mute) {
                    draw::fill(s, &draw::round(w - PAD - 36.0, y + 9.0, 36.0, 36.0, 18.0), pal.ink_main.alpha(0.09));
                }
                let glyph = match (input, ch.muted) {
                    (false, false) => "volume_up",
                    (false, true) => "volume_off",
                    (true, false) => "mic",
                    (true, true) => "mic_off",
                };
                ink.icon(glyph, 20.0, false, if ch.muted { RED } else { pal.ink_meta }, w - PAD - 18.0, y + 27.0);
            }
            if mix.apps.is_empty() {
                let y = top + mix_rows(d) as f64 * ROW;
                ink.text(if input { "No apps are listening" } else { "No apps are playing" }, 13.0, Face::Text(500), pal.ink_meta, 0.0, y + 18.0, w, pango::Alignment::Center);
            }
        }
        Kind::Media => {
            let m = st.app.media.borrow();
            if m.bus.is_none() {
                ink.empty("music_off", "Nothing playing", w, HEAD, h - HEAD - PAD);
                return;
            }
            let art = draw::round(PAD, HEAD + 2.0, ART, ART, 16.0 * st.app.round());
            match &m.art {
                Some(tex) => {
                    s.push_rounded_clip(&art);
                    s.append_texture(tex, art.bounds());
                    s.pop();
                }
                None => {
                    draw::fill(s, &art, m.tint.unwrap_or(pal.accent).alpha(0.2));
                    ink.icon("music_note", 34.0, true, m.tint.unwrap_or(pal.accent), PAD + ART / 2.0, HEAD + 2.0 + ART / 2.0);
                }
            }
            let tx = MEDIA_TEXT;
            let title = if m.title.is_empty() { m.player.as_str() } else { m.title.as_str() };
            ink.text(title, 15.0, Face::Text(600), pal.ink_main, tx, HEAD + 14.0, w - tx - PAD, left);
            ink.text(&m.artist, 13.0, Face::Text(450), pal.ink_meta, tx, HEAD + 34.0, w - tx - PAD, left);
            for it in &items {
                let Hit::Media(i) = it.hit else { continue };
                let live = match i {
                    0 => m.can_prev,
                    2 => m.can_next,
                    _ => true,
                };
                let main = i == 1;
                let back = if main { pal.accent.alpha(if ink.hot(it.hit) { 0.9 } else { 1.0 }) } else { pal.ink_main.alpha(if ink.hot(it.hit) && live { 0.1 } else { 0.0 }) };
                draw::fill(s, &draw::round(it.x, it.y, it.w, it.h, it.h / 2.0), back);
                let glyph = match i {
                    0 => "skip_previous",
                    1 if m.playing => "pause",
                    1 => "play_arrow",
                    _ => "skip_next",
                };
                let tint = if main { pal.on_accent } else if live { pal.ink_main } else { pal.ink_faint };
                ink.icon(glyph, 24.0, true, tint, it.x + it.w / 2.0, it.y + it.h / 2.0);
            }
            if m.length > 0.0 {
                let pos = if m.playing { m.position + (crate::app::App::now() - m.position_at) as f64 / 1e6 } else { m.position };
                let frac = (pos / m.length).clamp(0.0, 1.0);
                let y = HEAD + 2.0 + ART + 18.0;
                draw::fill(s, &draw::round(PAD, y, w - 2.0 * PAD, 4.0, 2.0), pal.ink_main.alpha(0.12));
                draw::fill(s, &draw::round(PAD, y, ((w - 2.0 * PAD) * frac).max(4.0), 4.0, 2.0), pal.accent);
                let t = |secs: f64| gsr::clock(secs.max(0.0) as u64);
                ink.text(&t(pos), 11.5, Face::Number(500), pal.ink_meta, PAD, y + 17.0, 80.0, left);
                ink.text(&t(m.length), 11.5, Face::Number(500), pal.ink_meta, w - PAD - 80.0, y + 17.0, 80.0, pango::Alignment::Right);
            }
        }
        Kind::Bluetooth => {
            let Some(adapter) = d.bt.chosen() else {
                ink.empty("bluetooth_disabled", if d.bt_reading { "Loading…" } else { "No Bluetooth adapter" }, w, HEAD, h - HEAD - PAD);
                return;
            };
            let mid = HEAD + 2.0 + 22.0;
            ink.text(if adapter.powered { "On" } else { "Off" }, 14.0, Face::Text(600), pal.ink_main, PAD + 2.0, mid - 8.0, 160.0, left);
            let connected = d.bt.on(&adapter.path).filter(|x| x.connected).count();
            let sub = if !adapter.powered { "Turn on to use devices".to_string() } else if connected == 0 { "Not connected".to_string() } else { format!("{connected} connected") };
            ink.text(&sub, 12.0, Face::Text(480), pal.ink_meta, PAD + 2.0, mid + 10.0, w - 2.0 * PAD - 60.0, left);
            if let Some(it) = items.iter().find(|i| i.hit == Hit::BtPower) {
                ink.switch(it, adapter.powered);
            }
            draw::fill(s, &draw::round(PAD, HEAD + 2.0 + 44.0, w - 2.0 * PAD, 1.0, 0.0), pal.hairline);
            let list = devices(&d.bt);
            let top = HEAD + 2.0 + 44.0 + 6.0;
            if list.is_empty() {
                ink.text("No paired devices", 13.0, Face::Text(500), pal.ink_meta, 0.0, top + BT_ROW / 2.0, w, pango::Alignment::Center);
            }
            s.push_opacity(if adapter.powered { 1.0 } else { 0.45 });
            for (i, dev) in list.iter().enumerate() {
                let it = &items.iter().find(|x| x.hit == Hit::BtDevice(i));
                let y = top + i as f64 * BT_ROW;
                if let Some(it) = it.filter(|it| ink.hot(it.hit) && adapter.powered) {
                    draw::fill(s, &draw::round(it.x, it.y, it.w, it.h, 14.0 * st.app.round()), pal.ink_main.alpha(0.07));
                }
                let busy = d.bt_busy.contains(&dev.path);
                draw::fill(s, &draw::round(PAD, y + 6.0, 34.0, 34.0, 17.0), if dev.connected { pal.accent } else { pal.ink_main.alpha(0.1) });
                ink.icon(bluez::symbol(&dev.icon), 18.0, true, if dev.connected { pal.on_accent } else { pal.ink_main }, PAD + 17.0, y + 23.0);
                ink.text(&dev.name, 13.5, Face::Text(560), pal.ink_main, PAD + 46.0, y + 15.0, w - 2.0 * PAD - 50.0, left);
                let state = match (busy, dev.connected, dev.battery) {
                    (true, true, _) => "Disconnecting…".to_string(),
                    (true, false, _) => "Connecting…".to_string(),
                    (false, true, Some(b)) => format!("Connected · {b}%"),
                    (false, true, None) => "Connected".to_string(),
                    (false, false, _) => "Paired".to_string(),
                };
                ink.text(&state, 12.0, Face::Text(480), if dev.connected { pal.accent } else { pal.ink_meta }, PAD + 46.0, y + 33.0, w - 2.0 * PAD - 50.0, left);
            }
            s.pop();
        }
        Kind::Notes => {
            let (fx, fy, fw, fh) = notes_field(w, h);
            draw::fill(s, &draw::round(fx, fy, fw, fh, 16.0 * st.app.round()), pal.ink_main.alpha(0.05));
            let b = st.notes.buffer();
            if b.char_count() == 0 {
                ink.text("Write something here…", 14.0, Face::Text(450), pal.ink_faint, fx + 12.0, fy + 18.0, fw - 24.0, left);
            }
        }
        Kind::Crosshair => {
            let aim = &st.aim;
            let mid = HEAD + 2.0 + 22.0;
            ink.text("Show crosshair", 14.0, Face::Text(600), pal.ink_main, PAD + 2.0, mid, 180.0, left);
            if let Some(it) = items.iter().find(|i| i.hit == Hit::AimOn) {
                ink.switch(it, aim.on);
            }
            let top = HEAD + 2.0 + 44.0;
            for (row, label) in ["Style", "Size", "Color"].iter().enumerate() {
                ink.text(label, 13.0, Face::Text(500), pal.ink_meta, PAD + 2.0, top + row as f64 * AIM_ROW + AIM_ROW / 2.0, AIM_LABEL - 4.0, left);
            }
            let labels = ["Dot", "Cross", "Small", "Medium", "Large"];
            for it in &items {
                match it.hit {
                    Hit::AimStyle(v) => ink.pill(it, labels[v as usize], aim.style == v),
                    Hit::AimSize(v) => ink.pill(it, labels[2 + v as usize], aim.size == v),
                    Hit::AimColor(v) => {
                        let on = aim.color == v;
                        // The chosen colour sits on an accent disc, like a selected pill.
                        if on || ink.hot(it.hit) {
                            draw::fill(s, &draw::round(it.x - 4.0, it.y - 4.0, it.w + 8.0, it.h + 8.0, it.h / 2.0 + 4.0), if on { pal.accent } else { pal.tile(0.14) });
                        }
                        draw::fill(s, &draw::round(it.x, it.y, it.w, it.h, it.h / 2.0), AIM_COLORS[v as usize]);
                    }
                    _ => {}
                }
            }
        }
    }
}

/// The crosshair at the screen's centre, outlined so it reads on light and dark scenes.
pub fn crosshair(s: &gtk::Snapshot, aim: &Aim, cx: f64, cy: f64) {
    let color = AIM_COLORS[(aim.color as usize).min(AIM_COLORS.len() - 1)];
    let edge = Rgb::new(0.0, 0.0, 0.0).alpha(0.7);
    let size = aim.size.min(2) as f64;
    let rect = |x: f64, y: f64, w: f64, h: f64, r: f64, c: Rgb| draw::fill(s, &draw::round(x, y, w, h, r), c);
    if aim.style == 0 {
        let d = 4.0 + size * 2.0;
        rect(cx - d / 2.0 - 1.0, cy - d / 2.0 - 1.0, d + 2.0, d + 2.0, d / 2.0 + 1.0, edge);
        rect(cx - d / 2.0, cy - d / 2.0, d, d, d / 2.0, color);
        return;
    }
    let (arm, gap, t) = (6.0 + size * 4.0, 3.0 + size, 2.0);
    let arms = [(cx - t / 2.0, cy - gap - arm, t, arm), (cx - t / 2.0, cy + gap, t, arm), (cx - gap - arm, cy - t / 2.0, arm, t), (cx + gap, cy - t / 2.0, arm, t)];
    for (x, y, w, h) in arms {
        rect(x - 1.0, y - 1.0, w + 2.0, h + 2.0, 1.5, edge);
    }
    for (x, y, w, h) in arms {
        rect(x, y, w, h, 1.0, color);
    }
}
