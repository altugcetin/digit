//! More desktop widgets, each in a corner of the main display: what's playing, the weather and
//! the system. Like the clock and the batteries they sit straight on the wallpaper, their ink
//! following it, or on a card of their own. This also holds what every desktop widget shares:
//! how it looks over the wallpaper, its card, and keeping clear of the dock.

use crate::app::App;
use crate::batterywidget::CardRect;
use crate::draw::{self, Face};
use crate::paint::Paint;
use gtk::{cairo, gdk, glib, pango, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kit::palette::Rgb;
use kit::motion::Tween;
use kit::deskwidget::{Ink, Tone};
use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::time::Duration;

/// The gap to the screen's edges, as the batteries keep.
const AIR: f64 = 28.0;
/// How far a widget's card reaches past what it shows.
pub(crate) const CARD_PAD: f64 = 16.0;
const RED: Rgb = Rgb::new(1.0, 0.27, 0.23);

/// How a desktop widget draws over the wallpaper: its ink, a shadow under that ink when it sits
/// straight on the wallpaper, and the card under it when it has one.
pub(crate) struct Look {
    pub ink: Rgb,
    pub shade: Option<gdk::RGBA>,
    pub card: Option<Rgb>,
}

/// The color set for `widget` (see `kit::deskwidget`).
pub(crate) fn ink_of(app: &App, widget: &str) -> Ink {
    let p = app.prefs.borrow();
    Ink::parse(match widget {
        "clock" => &p.desktop_clock_ink,
        "battery" => &p.desktop_battery_ink,
        "media" => &p.desktop_media_ink,
        "weather" => &p.desktop_weather_ink,
        "system" => &p.desktop_system_ink,
        "screentime" => &p.desktop_screentime_ink,
        _ => "auto",
    })
}

/// The look over the wallpaper of `output`, at its top (`top`) or anywhere on it, in the color
/// set for `widget`.
pub(crate) fn look(app: &App, output: &str, top: bool, card: bool, widget: &str) -> Look {
    let luma = app.wallpapers.top_luma(output, if top { 0.32 } else { 1.0 }).unwrap_or(0.0);
    let tone = app.prefs.borrow().tone_of(widget);
    look_for(luma > 0.42, card, kit::effects::reduced(), ink_of(app, widget), tone, app.palette.get().accent)
}

/// `light`: the wallpaper behind is light. A color that was set holds over it, and the card
/// and the shadow follow that color instead, so it always reads: white on dark, black on light.
/// A card set Light or Dark holds too, and Automatic text on it follows the card.
fn look_for(light: bool, card: bool, reduced: bool, set: Ink, tone: Tone, accent: Rgb) -> Look {
    let mut light = match set {
        Ink::White => false,
        Ink::Black => true,
        Ink::Auto | Ink::Accent => light,
    };
    if card {
        match tone {
            Tone::Light => light = true,
            Tone::Dark => light = false,
            Tone::Auto => {}
        }
    }
    let ink = match set {
        Ink::Accent => accent,
        Ink::White => Rgb::new(1.0, 1.0, 1.0),
        Ink::Black => Rgb::new(0.08, 0.08, 0.1),
        Ink::Auto if light => Rgb::new(0.08, 0.08, 0.1),
        Ink::Auto => Rgb::new(1.0, 1.0, 1.0),
    };
    if card {
        // As macOS's widgets: a soft see-through card, light over a light wallpaper and dark
        // over a dark one, the ink on it needing no shadow.
        let fill = if light { Rgb::new(0.97, 0.97, 0.98).alpha(0.66) } else { Rgb::new(0.1, 0.1, 0.12).alpha(0.56) };
        return Look { ink, shade: None, card: Some(fill) };
    }
    let shade = if light { gdk::RGBA::new(1.0, 1.0, 1.0, 0.28) } else { gdk::RGBA::new(0.0, 0.0, 0.0, 0.32) };
    Look { ink, shade: (!reduced).then_some(shade), card: None }
}

/// A widget's card over `(x, y, w, h)`, round as the apps' cards and lifted by a soft shadow.
pub(crate) fn card(s: &gtk::Snapshot, app: &App, (x, y, w, h): (f64, f64, f64, f64), fill: Rgb) {
    let shape = draw::round(x, y, w, h, crate::metrics::scaled(24.0, app.round(), h));
    if !kit::effects::reduced() {
        s.append_outset_shadow(&shape, &gdk::RGBA::new(0.0, 0.0, 0.0, 0.16), 0.0, 3.0, 0.0, 14.0);
    }
    draw::fill(s, &shape, fill);
}

/// Draws what follows `k` times as big, growing from the point `air` in from the surface's
/// corner: the widget's air to the screen's edges stays as it is (see `kit::deskwidget::SIZES`).
pub(crate) fn sized(s: &gtk::Snapshot, air: f64, k: f64) {
    if k == 1.0 {
        return;
    }
    let at = gtk::graphene::Point::new(air as f32, air as f32);
    s.translate(&at);
    s.scale(k as f32, k as f32);
    s.translate(&gtk::graphene::Point::new(-at.x(), -at.y()));
}

/// The dock's room on the left, right and bottom of `output` (see `dock::room`), and the
/// margins a widget anchored at the `top` or bottom, `left` or right takes from it.
pub(crate) fn dock_margins(output: &str, top: bool, left: bool) -> (i32, i32) {
    let room = crate::dock::room(output);
    (if left { room[0] } else { room[1] }, if top { 0 } else { room[2] })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Media,
    Weather,
    System,
    ScreenTime,
}

impl Kind {
    const ALL: [Kind; 4] = [Kind::Media, Kind::Weather, Kind::System, Kind::ScreenTime];

    /// Whether it has a card.
    fn backed(self, p: &crate::config::Prefs) -> bool {
        match self {
            Kind::Media => p.desktop_media_bg,
            Kind::Weather => p.desktop_weather_bg,
            Kind::System => p.desktop_system_bg,
            Kind::ScreenTime => p.desktop_screentime_bg,
        }
    }

    /// The corner it's in on the screen `output` ("top_left"…), or "off".
    fn place_on(self, app: &App, output: &str) -> String {
        let i = kit::deskwidget::WIDGETS.iter().position(|w| *w == self.name()).unwrap_or(0);
        crate::batterywidget::places(app, output)[i].clone()
    }

    /// The name its settings go under.
    fn name(self) -> &'static str {
        match self {
            Kind::Media => "media",
            Kind::Weather => "weather",
            Kind::System => "system",
            Kind::ScreenTime => "screentime",
        }
    }

    /// How much larger or smaller than as designed it's drawn.
    fn scale(self, p: &crate::config::Prefs) -> f64 {
        p.size_of(self.name())
    }

    /// The size of what it draws, without the air around it, as designed.
    fn size(self) -> (f64, f64) {
        match self {
            Kind::Media => (MEDIA_W, ART),
            Kind::Weather => (300.0, 64.0),
            Kind::System => (4.0 * CELL, 100.0),
            Kind::ScreenTime => (SHOT_W, 150.0),
        }
    }
}

const ART: f64 = 112.0;
const MEDIA_W: f64 = 380.0;
const CELL: f64 = 92.0;

struct Card {
    kind: Kind,
    window: gtk::Window,
    paint: Paint,
}

/// What one widget animates: the glow under the Now playing buttons (previous, play or pause,
/// next), the values on their way to new ones, the track it fades over from, and whether its
/// frames are being drawn until all of that settles.
struct Live {
    glow: [Tween; 3],
    glides: crate::glide::Glides,
    track: Swap,
    ticking: bool,
}

impl Default for Live {
    fn default() -> Self {
        Self { glow: std::array::from_fn(|_| Tween::still(0.0)), glides: Default::default(), track: Swap::default(), ticking: false }
    }
}

impl Live {
    fn moving(&self, now: i64) -> bool {
        self.glow.iter().any(|t| !t.settled(now)) || self.glides.moving(now) || self.track.moving(now)
    }
}

/// A track as Now playing shows it.
#[derive(Clone)]
struct Shown {
    art: Option<gdk::Texture>,
    title: String,
    artist: String,
}

/// The track Now playing fades over from to a new one, and its cover fading in once it comes.
struct Swap {
    key: String,
    last: Option<Shown>,
    from: Option<Shown>,
    t: Tween,
    art: Option<gdk::Texture>,
    art_t: Tween,
}

impl Default for Swap {
    fn default() -> Self {
        Self { key: String::new(), last: None, from: None, t: Tween::still(1.0), art: None, art_t: Tween::still(1.0) }
    }
}

impl Swap {
    /// Notes that `now` shows `shown`: a new track fades over from the one before, a cover that
    /// came for it fades in. `on`: they do at all. How far the track and its cover got.
    fn step(&mut self, shown: &Shown, now: i64, on: bool) -> (Option<f64>, f64) {
        let key = format!("{}\u{1}{}", shown.title, shown.artist);
        if self.key != key {
            if on && !self.key.is_empty() {
                self.from = self.last.take();
                self.t = Tween::still(0.0);
                self.t.go(1.0, now, 460, kit::motion::STANDARD);
            }
            self.key = key;
        }
        if self.art != shown.art {
            if on && shown.art.is_some() {
                self.art_t = Tween::still(0.0);
                self.art_t.go(1.0, now, 320, kit::motion::STANDARD);
            }
            self.art = shown.art.clone();
        }
        self.last = Some(shown.clone());
        if !on || self.t.settled(now) {
            self.from = None;
        }
        (self.from.as_ref().map(|_| self.t.value(now)), if on { self.art_t.value(now) } else { 1.0 })
    }

    fn moving(&self, now: i64) -> bool {
        (self.from.is_some() && !self.t.settled(now)) || !self.art_t.settled(now)
    }
}

/// After a frame of `paint`: keeps drawing frames while anything of `live` is on its way.
fn keep_going(paint: &Paint, live: &Rc<RefCell<Live>>) {
    let start = {
        let Ok(mut l) = live.try_borrow_mut() else { return };
        l.moving(glib::monotonic_time()) && !std::mem::replace(&mut l.ticking, true)
    };
    if !start {
        return;
    }
    let live = live.clone();
    paint.add_tick_callback(move |paint, clock| {
        paint.queue_draw();
        let mut l = live.borrow_mut();
        if l.moving(clock.frame_time()) {
            glib::ControlFlow::Continue
        } else {
            l.ticking = false;
            glib::ControlFlow::Break
        }
    });
}


thread_local! {
    static CARDS: RefCell<Vec<Card>> = const { RefCell::new(Vec::new()) };
    static SHOWN: RefCell<String> = const { RefCell::new(String::new()) };
    static TIMER: RefCell<Option<glib::SourceId>> = const { RefCell::new(None) };
    static APP: RefCell<Weak<App>> = const { RefCell::new(Weak::new()) };
    /// The screens where the Now playing buttons showed when last drawn.
    static BUTTONS_ON: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static STATS: RefCell<(crate::sysmon::Sampler, Option<crate::sysmon::Stats>, Option<f64>)> = RefCell::new(Default::default());
}

/// Glows the Now playing button at `at` (an index into `buttons`) of the widget `paint` and
/// lets its others fade, redrawing it until they settle.
fn hover(paint: &Paint, live: &Rc<RefCell<Live>>, at: Option<usize>) {
    let now = glib::monotonic_time();
    paint.set_cursor_from_name(at.map(|_| "pointer"));
    let moved = {
        let mut l = live.borrow_mut();
        let mut moved = false;
        for (i, t) in l.glow.iter_mut().enumerate() {
            let to = if Some(i) == at { 1.0 } else { 0.0 };
            if t.target() != to {
                t.go(to, now, if to > 0.0 { 160 } else { 260 }, kit::motion::STANDARD);
                moved = true;
            }
        }
        moved
    };
    if moved {
        paint.queue_draw();
        keep_going(paint, live);
    }
}


/// Redraws the widgets, after the weather or the palette changed.
pub fn changed() {
    CARDS.with(|c| c.borrow().iter().for_each(|c| c.paint.queue_draw()));
}

/// Redraws what's playing, after a track or its state changed.
pub fn media_changed() {
    CARDS.with(|c| c.borrow().iter().filter(|c| c.kind == Kind::Media).for_each(|c| c.paint.queue_draw()));
}

/// Redraws the Screen Time widget, after time was counted.
pub fn screentime_changed() {
    CARDS.with(|c| c.borrow().iter().filter(|c| c.kind == Kind::ScreenTime).for_each(|c| c.paint.queue_draw()));
}

/// The dock started or stopped keeping room: widgets in its corners move.
pub fn dock_moved() {
    let Some(app) = APP.with(|a| a.borrow().upgrade()) else { return };
    crate::batterywidget::apply(&app);
    apply(&app);
}

fn clear() {
    for c in CARDS.with(|c| std::mem::take(&mut *c.borrow_mut())) {
        c.window.destroy();
    }
    BUTTONS_ON.with(|b| b.borrow_mut().clear());
    if let Some(id) = TIMER.with(|t| t.borrow_mut().take()) {
        id.remove();
    }
}

/// How far from the top a widget in a top corner of the screen `output` starts: under the bar,
/// and under the clock when both are at the top left.
fn top_margin(app: &App, output: &str, left: bool) -> i32 {
    app.reserve() + if left { crate::desk::room(app, output) } else { 0 }
}

/// Where the widgets that are out sit on a screen `screen` big, in its coordinates: their
/// cards, or just what they draw when they have none (a card's padding would reach into the
/// next row of icons and empty it). Desktop icons keep out of these.
pub(crate) fn rects(app: &App, output: &str, screen: (i32, i32)) -> Vec<CardRect> {
    let prefs = app.prefs.borrow().clone();
    Kind::ALL
        .iter()
        .filter_map(|k| {
            let (place, backed) = (k.place_on(app, output), k.backed(&prefs));
            if matches!(place.as_str(), "" | "off") {
                return None;
            }
            let (top, left) = (place.starts_with("top"), place.ends_with("left"));
            let scale = k.scale(&prefs);
            let (w, h) = (k.size().0 * scale, k.size().1 * scale);
            let (side, bottom) = dock_margins(output, top, left);
            let x = if left { side as f64 + AIR } else { screen.0 as f64 - side as f64 - AIR - w };
            let y = if top { top_margin(app, output, left) as f64 + AIR } else { screen.1 as f64 - bottom as f64 - AIR - h };
            let pad = if backed { CARD_PAD * scale } else { 0.0 };
            Some(CardRect { x: x - pad, y: y - pad, w: w + 2.0 * pad, h: h + 2.0 * pad })
        })
        .collect()
}

/// Where the Now playing buttons sit on a screen `screen` big, in its coordinates, while the
/// widget shows there with its controls on. The desktop icons' surface, which covers the whole
/// screen in the same layer and can end up above the widget, leaves these to it.
pub(crate) fn media_buttons(app: &App, output: &str, screen: (i32, i32)) -> Vec<CardRect> {
    // As the widget last drew them, so the two never disagree.
    if !BUTTONS_ON.with(|b| b.borrow().iter().any(|o| o == output)) {
        return Vec::new();
    }
    let kind = Kind::Media;
    let place = kind.place_on(app, output);
    if matches!(place.as_str(), "" | "off") {
        return Vec::new();
    }
    let k = kind.scale(&app.prefs.borrow());
    let (top, left) = (place.starts_with("top"), place.ends_with("left"));
    let (side, bottom) = dock_margins(output, top, left);
    let top_at = if top { top_margin(app, output, left) as f64 } else { 0.0 };
    media_buttons_at(screen, (top, left), k, (side, bottom), top_at)
}

/// [`media_buttons`] for a widget `k` times as big in the corner `(top, left)`, `margins` (side,
/// bottom) from the screen's edges and `top_at` from its top.
fn media_buttons_at(screen: (i32, i32), (top, left): (bool, bool), k: f64, (side, bottom): (i32, i32), top_at: f64) -> Vec<CardRect> {
    let (w, h) = (MEDIA_W * k, Kind::Media.size().1 * k);
    // The drawn area starts AIR in from the surface, as in `rects`.
    let x = if left { side as f64 + AIR } else { screen.0 as f64 - side as f64 - AIR - w };
    let y = if top { top_at + AIR } else { screen.1 as f64 - bottom as f64 - AIR - h };
    buttons().into_iter().map(|(_, (bx, by, bw, bh))| CardRect { x: x + (bx - AIR) * k, y: y + (by - AIR) * k, w: bw * k, h: bh * k }).collect()
}

pub fn apply(app: &Rc<App>) {
    APP.with(|a| *a.borrow_mut() = Rc::downgrade(app));
    let monitors = kit::screens::all();
    // Every widget that is out, on each screen whose places have it.
    let mut places: Vec<(gdk::Monitor, String, Kind, String)> = Vec::new();
    let mut key = format!("{}", app.reserve());
    for m in monitors {
        let Some(output) = m.connector().map(|c| c.to_string()) else { continue };
        key.push_str(&format!(" {output}:{}:{:?}", crate::desk::room(app, &output), crate::dock::room(&output)));
        for kind in Kind::ALL {
            let place = kind.place_on(app, &output);
            if !matches!(place.as_str(), "" | "off") {
                key.push_str(&format!(":{kind:?}={place}@{}", kind.scale(&app.prefs.borrow())));
                places.push((m.clone(), output.clone(), kind, place));
            }
        }
    }
    // The widgets may have moved, or their buttons come or gone: the icons make way again.
    crate::deskicons::widget_buttons_moved();
    if SHOWN.with(|s| *s.borrow() == key) && CARDS.with(|c| c.borrow().len()) == places.len() {
        // Same places: options (cards, controls) only need a redraw.
        changed();
        return;
    }
    clear();
    SHOWN.with(|s| *s.borrow_mut() = key);
    if !app.cfg.wallpaper {
        return;
    }
    for (m, output, kind, place) in places {
        let (top, left) = (place.starts_with("top"), place.ends_with("left"));
        let (w, h) = kind.size();
        let k = kind.scale(&app.prefs.borrow());
        let window = gtk::Window::new();
        window.init_layer_shell();
        window.set_layer(Layer::Bottom);
        window.set_namespace(Some("digit-desktop"));
        window.set_keyboard_mode(KeyboardMode::None);
        window.set_exclusive_zone(-1);
        window.set_monitor(Some(&m));
        window.add_css_class("digit");
        window.set_anchor(if top { Edge::Top } else { Edge::Bottom }, true);
        window.set_anchor(if left { Edge::Left } else { Edge::Right }, true);
        let (side, bottom) = dock_margins(&output, top, left);
        window.set_margin(if left { Edge::Left } else { Edge::Right }, side);
        window.set_margin(if top { Edge::Top } else { Edge::Bottom }, if top { top_margin(app, &output, left) } else { bottom });
        let paint = Paint::new((w * k + 2.0 * AIR).ceil() as i32, (h * k + 2.0 * AIR).ceil() as i32);
        let live = Rc::new(RefCell::new(Live::default()));
        let (weak, a, out, l) = (paint.downgrade(), Rc::downgrade(app), output.clone(), live.clone());
        paint.on_draw(move |s| {
            let (Some(p), Some(app)) = (weak.upgrade(), a.upgrade()) else { return };
            sized(s, AIR, k);
            let (font, on) = {
                let prefs = app.prefs.borrow();
                (prefs.font_of(kind.name()), prefs.desktop_motion)
            };
            {
                let mut live = l.borrow_mut();
                live.glides.begin(on);
                crate::draw::zoomed(k, || crate::draw::in_font(&font, || crate::draw::rasterized(|| draw(s, &p, &app, kind, &out, top, k, &mut live, on))));
            }
            keep_going(&p, &l);
        });
        if kind == Kind::Media {
            let click = gtk::GestureClick::new();
            let a = Rc::downgrade(app);
            click.connect_released(move |_, _, x, y| {
                let Some(app) = a.upgrade() else { return };
                if !app.prefs.borrow().desktop_media_controls {
                    return;
                }
                let (x, y) = (AIR + (x - AIR) / k, AIR + (y - AIR) / k);
                let Some(cmd) = buttons().into_iter().find(|(_, r)| contains(*r, x, y)).map(|(c, _)| c) else { return };
                let players = app.players.borrow().clone();
                if let Some(p) = players {
                    p.press(cmd);
                }
            });
            paint.add_controller(click);
            // The buttons glow under the pointer, as the shell's other round buttons do.
            let motion = gtk::EventControllerMotion::new();
            let (weak, l) = (paint.downgrade(), live.clone());
            motion.connect_motion(move |_, x, y| {
                let Some(p) = weak.upgrade() else { return };
                let (x, y) = (AIR + (x - AIR) / k, AIR + (y - AIR) / k);
                hover(&p, &l, buttons().iter().position(|(_, r)| contains(*r, x, y)));
            });
            let weak = paint.downgrade();
            motion.connect_leave(move |_| {
                if let Some(p) = weak.upgrade() {
                    hover(&p, &live, None);
                }
            });
            paint.add_controller(motion);
        }
        window.set_child(Some(&paint));
        window.connect_realize(|w| {
            if let Some(surface) = w.surface() {
                surface.set_input_region(Some(&cairo::Region::create()));
            }
        });
        window.present();
        CARDS.with(|c| c.borrow_mut().push(Card { kind, window, paint }));
    }
    if CARDS.with(|c| !c.borrow().is_empty()) {
        tick(Rc::downgrade(app), 0);
    }
}

/// Once a second: what's playing moves on, and every other second the system is sampled.
fn tick(app: Weak<App>, beat: u32) {
    let id = glib::timeout_add_local_once(Duration::from_millis(if beat == 0 { 400 } else { 1000 }), move || {
        TIMER.with(|t| t.borrow_mut().take());
        let Some(app) = app.upgrade() else { return };
        let kinds: Vec<Kind> = CARDS.with(|c| c.borrow().iter().map(|c| c.kind).collect());
        if kinds.is_empty() {
            return;
        }
        if kinds.contains(&Kind::Media) && app.media.borrow().playing {
            media_changed();
        }
        if kinds.contains(&Kind::System) && beat.is_multiple_of(2) {
            let nvidia = STATS.with(|st| {
                let mut st = st.borrow_mut();
                let mut stats = st.0.sample();
                if stats.gpu.is_none() {
                    stats.gpu = st.2;
                }
                let changed = st.1 != Some(stats);
                st.1 = Some(stats);
                if changed {
                    CARDS.with(|c| c.borrow().iter().filter(|c| c.kind == Kind::System).for_each(|c| c.paint.queue_draw()));
                }
                st.0.wants_nvidia()
            });
            if nvidia && beat.is_multiple_of(4) {
                glib::spawn_future_local(async {
                    let gpu = crate::sysmon::nvidia().await;
                    if gpu.is_some() {
                        STATS.with(|st| st.borrow_mut().2 = gpu);
                    }
                });
            }
        }
        tick(Rc::downgrade(&app), beat.wrapping_add(1));
    });
    TIMER.with(|t| *t.borrow_mut() = Some(id));
}

type Rect = (f64, f64, f64, f64);

fn contains((x, y, w, h): Rect, px: f64, py: f64) -> bool {
    px >= x && px <= x + w && py >= y && py <= y + h
}

/// Previous, play or pause, and next, under the track, in surface coordinates.
fn buttons() -> [(crate::media::Command, Rect); 3] {
    use crate::media::Command;
    let (tx, mid) = (AIR + ART + 18.0, AIR + ART - 18.0);
    let cx = tx + (MEDIA_W - ART - 18.0) / 2.0;
    let r = |c: f64| (c - 18.0, mid - 18.0, 36.0, 36.0);
    [(Command::Previous, r(cx - 46.0)), (Command::PlayPause, r(cx)), (Command::Next, r(cx + 46.0))]
}

fn mmss(seconds: f64) -> String {
    let s = seconds.max(0.0).round() as i64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// `k`: the size it's drawn at (see [`sized`]), which the buttons' input region follows; `live`
/// what it animates, `on` whether it does.
#[allow(clippy::too_many_arguments)]
fn draw(s: &gtk::Snapshot, p: &Paint, app: &App, kind: Kind, output: &str, top: bool, k: f64, live: &mut Live, on: bool) {
    let (with_card, controls) = {
        let prefs = app.prefs.borrow();
        (kind.backed(&prefs), prefs.desktop_media_controls)
    };
    let look = look(app, output, top, with_card, kind.name());
    let drawn = match kind {
        Kind::Media => app.media.borrow().present(glib::monotonic_time()),
        Kind::Weather => app.weather.borrow().is_some(),
        Kind::System | Kind::ScreenTime => true,
    };
    if kind == Kind::Media {
        buttons_shown(output, drawn && controls);
    }
    // Only the media buttons take clicks, and only while they're there.
    if let Some(surface) = p.root().and_then(|r| r.surface()) {
        let region = cairo::Region::create();
        if kind == Kind::Media && drawn && controls {
            for (_, (x, y, w, h)) in buttons() {
                let (x, y) = (AIR + (x - AIR) * k, AIR + (y - AIR) * k);
                let _ = region.union_rectangle(&cairo::RectangleInt::new(x as i32, y as i32, (w * k).ceil() as i32, (h * k).ceil() as i32));
            }
        }
        surface.set_input_region(Some(&region));
    }
    if !drawn {
        return;
    }
    if let Some(fill) = look.card {
        let (w, h) = kind.size();
        card(s, app, (AIR - CARD_PAD, AIR - CARD_PAD, w + 2.0 * CARD_PAD, h + 2.0 * CARD_PAD), fill);
    }
    // Under the shadow, which must keep still while the glow comes and goes.
    if kind == Kind::Media && controls {
        media_glow(s, app, look.ink, &live.glow);
    }
    if let Some(shade) = look.shade {
        s.push_shadow(&[gtk::gsk::Shadow::new(shade, 0.0, 1.0, 12.0)]);
    }
    match kind {
        Kind::Media => media(s, p, app, look.ink, controls, &mut live.track, on),
        Kind::Weather => weather(s, p, app, look.ink),
        Kind::System => system(s, p, app, look.ink, &mut live.glides),
        Kind::ScreenTime => screentime(s, p, app, look.ink, look.card.is_some(), k),
    }
    if look.shade.is_some() {
        s.pop();
    }
}

/// The cover, the title and the artist of `track`, its cover `art` of the way faded in.
fn track(s: &gtk::Snapshot, p: &Paint, app: &App, ink: Rgb, top: f64, track: &Shown, art: f64) {
    let cfg = &app.cfg;
    let cover = draw::round(AIR, AIR, ART, ART, crate::metrics::scaled(16.0, app.round(), ART));
    if track.art.is_none() || art < 1.0 {
        draw::fill(s, &cover, ink.alpha(0.14));
        draw::icon(s, p, cfg, "music_note", 40.0, true, ink.alpha(0.85), AIR + ART / 2.0, AIR + ART / 2.0);
    }
    if let Some(tex) = &track.art {
        s.push_opacity(art.clamp(0.0, 1.0));
        s.push_rounded_clip(&cover);
        s.append_texture(tex, cover.bounds());
        s.pop();
        s.pop();
    }
    let (tx, tw) = (AIR + ART + 18.0, MEDIA_W - ART - 18.0);
    let left = pango::Alignment::Left;
    draw::text(s, p, cfg, &track.title, 18.0, Face::Text(600), ink.alpha(0.96), tx, top + 14.0, tw, left);
    draw::text(s, p, cfg, &track.artist, 14.0, Face::Text(450), ink.alpha(0.72), tx, top + 37.0, tw, left);
}

/// `swap`: the track it fades over from, `on` whether it does.
fn media(s: &gtk::Snapshot, p: &Paint, app: &App, ink: Rgb, controls: bool, swap: &mut Swap, on: bool) {
    let cfg = &app.cfg;
    let m = app.media.borrow();
    let now = glib::monotonic_time();
    let (tx, tw) = (AIR + ART + 18.0, MEDIA_W - ART - 18.0);
    let left = pango::Alignment::Left;
    // Without the controls, the track sits in the middle of the cover's height.
    let top = if controls { AIR } else { AIR + 20.0 };
    let shown = Shown { art: m.art.clone(), title: if m.title.is_empty() { m.player.clone() } else { m.title.clone() }, artist: m.artist.clone() };
    match swap.step(&shown, now, on) {
        (Some(e), art) => {
            // The track before rises out as the new one comes up into its place.
            let lift = |s: &gtk::Snapshot, dy: f64, a: f64, t: &Shown, art: f64| {
                s.save();
                s.translate(&gtk::graphene::Point::new(0.0, dy.round() as f32));
                s.push_opacity(a.clamp(0.0, 1.0));
                track(s, p, app, ink, top, t, art);
                s.pop();
                s.restore();
            };
            if let Some(from) = swap.from.as_ref() {
                lift(s, -10.0 * e, 1.0 - e, from, 1.0);
            }
            lift(s, 10.0 * (1.0 - e), e, &shown, art);
        }
        (None, art) => track(s, p, app, ink, top, &shown, art),
    }
    if m.length > 0.0 {
        let at = m.elapsed(now);
        let y = top + 58.0;
        draw::fill(s, &draw::round(tx, y, tw, 4.0, 2.0), ink.alpha(0.22));
        draw::fill(s, &draw::round(tx, y, (tw * (at / m.length).clamp(0.0, 1.0)).max(4.0), 4.0, 2.0), ink.alpha(0.9));
        draw::text(s, p, cfg, &mmss(at), 11.5, Face::Number(500), ink.alpha(0.66), tx, y + 15.0, tw / 2.0, left);
        draw::text(s, p, cfg, &mmss(m.length), 11.5, Face::Number(500), ink.alpha(0.66), tx + tw / 2.0, y + 15.0, tw / 2.0, pango::Alignment::Right);
    }
    if !controls {
        return;
    }
    for (cmd, (x, y, w, h)) in buttons() {
        use crate::media::Command;
        let (glyph, size, on) = match cmd {
            Command::Previous => ("skip_previous", 24.0, m.can_prev),
            Command::Next => ("skip_next", 24.0, m.can_next),
            Command::PlayPause => (if m.playing { "pause" } else { "play_arrow" }, 30.0, true),
        };
        draw::icon(s, p, cfg, glyph, size, true, ink.alpha(if on { 0.92 } else { 0.35 }), x + w / 2.0, y + h / 2.0);
    }
}

/// Notes whether the Now playing buttons show on the screen `output`; when that changes, the
/// desktop icons leave their place to them, or take it back.
fn buttons_shown(output: &str, on: bool) {
    if BUTTONS_ON.with(|b| note_shown(&mut b.borrow_mut(), output, on)) {
        crate::deskicons::widget_buttons_moved();
    }
}

/// Puts `output` in `shown` or takes it out; whether that changed anything.
fn note_shown(shown: &mut Vec<String>, output: &str, on: bool) -> bool {
    let was = shown.iter().any(|o| o == output);
    if on && !was {
        shown.push(output.to_string());
    } else if !on && was {
        shown.retain(|o| o != output);
    }
    on != was
}

/// The glow under the Now playing buttons that can be pressed, as the pointer left it.
fn media_glow(s: &gtk::Snapshot, app: &App, ink: Rgb, glow: &[Tween; 3]) {
    let m = app.media.borrow();
    let now = glib::monotonic_time();
    for ((cmd, (x, y, w, h)), t) in buttons().into_iter().zip(glow) {
        use crate::media::Command;
        let on = match cmd {
            Command::Previous => m.can_prev,
            Command::Next => m.can_next,
            Command::PlayPause => true,
        };
        let v = t.value(now);
        if on && v > 0.0 {
            let r = w / 2.0 + 1.5 * v;
            draw::fill(s, &draw::round(x + w / 2.0 - r, y + h / 2.0 - r, 2.0 * r, 2.0 * r, r), ink.alpha(0.14 * v));
        }
    }
}

fn weather(s: &gtk::Snapshot, p: &Paint, app: &App, ink: Rgb) {
    let cfg = &app.cfg;
    let Some(wx) = app.weather.borrow().clone() else { return };
    let left = pango::Alignment::Left;
    let (x, y) = (AIR, AIR);
    let (w, _) = Kind::Weather.size();
    draw::icon(s, p, cfg, crate::weather::glyph(wx.code, wx.day), 46.0, true, ink.alpha(0.94), x + 24.0, y + 32.0);
    let temp = format!("{}°", wx.temp.round());
    let tw = draw::width(p, cfg, &temp, 54.0, Face::Text(330)).ceil();
    draw::text(s, p, cfg, &temp, 54.0, Face::Text(330), ink.alpha(0.96), x + 58.0, y + 32.0, tw + 4.0, left);
    let rx = x + 58.0 + tw + 18.0;
    let rw = w - (rx - x);
    draw::text(s, p, cfg, crate::weather::describe(wx.code), 17.0, Face::Text(600), ink.alpha(0.94), rx, y + 14.0, rw, left);
    draw::text(s, p, cfg, &format!("H {}°  L {}°", wx.hi.round(), wx.lo.round()), 13.5, Face::Text(500), ink.alpha(0.78), rx, y + 34.0, rw, left);
    draw::text(s, p, cfg, &wx.place, 13.0, Face::Text(450), ink.alpha(0.64), rx, y + 52.0, rw, left);
}

/// `glides`: the values on their way to the last sample's.
fn system(s: &gtk::Snapshot, p: &Paint, app: &App, ink: Rgb, glides: &mut crate::glide::Glides) {
    let cfg = &app.cfg;
    let mut st = STATS.with(|st| st.borrow().1).unwrap_or_default();
    let now = glib::monotonic_time();
    // Only a change of a tenth or more glides; the small ones of every sample just show.
    st.cpu = glides.at("cpu", st.cpu, 0.1, now);
    st.mem = glides.at("mem", st.mem, 0.1, now);
    st.used = glides.at("used", st.used, 0.1 * st.total, now);
    st.gpu = st.gpu.map(|g| glides.at("gpu", g, 0.1, now));
    st.temp = st.temp.map(|t| glides.at("temp", t, 10.0, now));
    let hot = |v: f64, warn: f64| if v >= warn { RED } else { ink };
    let cells: [(&str, &str, f64, String, Rgb); 4] = [
        ("memory", "CPU", st.cpu, format!("{}%", (st.cpu * 100.0).round()), hot(st.cpu, 0.9)),
        ("memory_alt", "Memory", st.mem, if st.total > 0.0 { format!("{:.1} GB", st.used) } else { "–".into() }, hot(st.mem, 0.9)),
        ("developer_board", "GPU", st.gpu.unwrap_or(0.0), st.gpu.map(|g| format!("{}%", (g * 100.0).round())).unwrap_or_else(|| "–".into()), hot(st.gpu.unwrap_or(0.0), 0.95)),
        ("device_thermostat", "Temp", (st.temp.unwrap_or(0.0) / 100.0).clamp(0.0, 1.0), st.temp.map(|t| format!("{}°", t.round())).unwrap_or_else(|| "–".into()), hot(st.temp.unwrap_or(0.0) / 100.0, 0.85)),
    ];
    for (i, (glyph, name, frac, value, tint)) in cells.iter().enumerate() {
        let cx = AIR + CELL * (i as f64 + 0.5);
        let cy = AIR + 26.0;
        crate::control::ring(s, cx, cy, 22.0, *frac, 3.5, ink.alpha(0.2), *tint);
        draw::icon(s, p, cfg, glyph, 19.0, false, ink, cx, cy);
        draw::text(s, p, cfg, value, 13.0, Face::Number(560), ink, cx - CELL / 2.0 + 3.0, cy + 39.0, CELL - 6.0, pango::Alignment::Center);
        draw::text(s, p, cfg, name, 11.5, Face::Text(450), ink.alpha(0.72), cx - CELL / 2.0 + 3.0, cy + 56.0, CELL - 6.0, pango::Alignment::Center);
    }
}

/// The widget's width, and how much of it the list of apps takes at the right.
const SHOT_W: f64 = 440.0;
const SHOT_SIDE: f64 = 132.0;

/// "29m", "1h 5m".
fn short(secs: u64) -> String {
    let m = secs / 60;
    if m >= 60 { format!("{}h {}m", m / 60, m % 60) } else { format!("{m}m") }
}

/// Screen Time as macOS draws it: today's total over the hours of the day, each hour a column
/// stacked by app, and at the right a panel with the apps that had the most time.
fn screentime(s: &gtk::Snapshot, p: &Paint, app: &App, ink: Rgb, with_card: bool, k: f64) {
    let cfg = &app.cfg;
    let sum = crate::screentime::summary();
    let left = pango::Alignment::Left;
    let (x, y) = (AIR, AIR);
    let (w, h) = Kind::ScreenTime.size();
    let main_w = w - SHOT_SIDE;
    // The panel at the right reaches the card's edges.
    if with_card {
        let edge = draw::round(x - CARD_PAD, y - CARD_PAD, w + 2.0 * CARD_PAD, h + 2.0 * CARD_PAD, crate::metrics::scaled(24.0, app.round(), h + 2.0 * CARD_PAD));
        s.push_rounded_clip(&edge);
        draw::fill(s, &draw::round(x + main_w, y - CARD_PAD, SHOT_SIDE + CARD_PAD, h + 2.0 * CARD_PAD, 0.0), ink.alpha(0.09));
        s.pop();
    }
    // The header: a round badge and the total.
    draw::fill(s, &draw::round(x, y, 40.0, 40.0, 20.0), ink.alpha(0.14));
    draw::icon(s, p, cfg, "hourglass_top", 22.0, true, ink.alpha(0.9), x + 20.0, y + 20.0);
    draw::text(s, p, cfg, &short(sum.total), 30.0, Face::Text(500), ink.alpha(0.96), x + 52.0, y + 20.0, main_w - 52.0, left);
    // The chart: 24 columns with lines at 0, 30 and 60 minutes and at every sixth hour.
    let (cx0, cx1) = (x, x + main_w - 42.0);
    let (top, base) = (y + 56.0, y + h - 24.0);
    let tall = base - top;
    let line = ink.alpha(0.22);
    for (i, label) in ["60m", "30m", "0m"].iter().enumerate() {
        let ly = top + tall * i as f64 / 2.0;
        draw::fill(s, &draw::round(cx0, ly - 0.5, cx1 - cx0, 1.0, 0.0), line);
        draw::text(s, p, cfg, label, 11.5, Face::Number(450), ink.alpha(0.6), cx1 + 8.0, ly, 34.0, left);
    }
    let col = (cx1 - cx0) / 24.0;
    for (i, label) in ["12 AM", "6 AM", "12 PM", "6 PM"].iter().enumerate() {
        let vx = cx0 + col * 6.0 * i as f64;
        let mut dy = top;
        while dy < base {
            draw::fill(s, &draw::round(vx - 0.5, dy, 1.0, 3.0, 0.0), line);
            dy += 6.0;
        }
        draw::text(s, p, cfg, label, 11.5, Face::Text(450), ink.alpha(0.6), vx + 4.0, base + 13.0, 60.0, left);
    }
    let tints = [Rgb::new(0.04, 0.52, 1.0), Rgb::new(0.35, 0.68, 0.8), Rgb::new(1.0, 0.62, 0.04), ink.alpha(0.4)];
    for hour in 0..24 {
        let mut at = base;
        for (series, tint) in sum.hours.iter().zip(&tints) {
            if series[hour] == 0 {
                continue;
            }
            // Even a minute shows: a sliver under two pixels would vanish.
            let part = (tall * series[hour] as f64 / 3600.0).max(2.0).min(at - top);
            draw::fill(s, &draw::round(cx0 + col * hour as f64 + col * 0.2, at - part, col * 0.6, part, 1.5), *tint);
            at -= part;
        }
    }
    // The apps with the most time.
    for (i, (id, name, secs)) in sum.top.iter().enumerate() {
        let row = y + (h / 4.0) * (i as f64 + 0.5);
        let ix = x + main_w + 18.0;
        let id = id.trim_end_matches(".desktop");
        // Loaded at the size it's drawn at, so a larger widget's icons stay sharp.
        let scale = p.scale_factor().max(1);
        match draw::app_icon(&app.apps, id, (28.0 * scale as f64 * k.max(1.0)).ceil() as i32, scale) {
            Some(icon) => draw::paintable(s, &icon, ix, row - 14.0, 28.0),
            None => draw::monogram(s, p, cfg, name, ix, row - 14.0, 28.0, ink.alpha(0.14), ink),
        }
        draw::text(s, p, cfg, &short(*secs), 13.5, Face::Number(450), ink.alpha(0.62), ix + 38.0, row, SHOT_SIDE - 60.0, left);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_buttons_sit_inside_the_widget_without_overlapping() {
        let (w, h) = Kind::Media.size();
        let b = buttons();
        for (_, (x, y, bw, bh)) in b {
            assert!(x >= AIR + ART && x + bw <= AIR + w && y >= AIR && y + bh <= AIR + h);
        }
        for pair in b.windows(2) {
            assert!(pair[0].1.0 + pair[0].1.2 <= pair[1].1.0);
        }
        assert_eq!(mmss(65.4), "1:05");
        assert_eq!(mmss(-3.0), "0:00");
    }

    #[test]
    fn a_new_track_fades_over_from_the_one_before() {
        let track = |t: &str| Shown { art: None, title: t.into(), artist: "Artist".into() };
        let mut swap = Swap::default();
        assert_eq!(swap.step(&track("One"), 0, true).0, None, "the first shows at once");
        assert_eq!(swap.step(&track("Two"), 1_000_000, true).0, Some(0.0));
        assert_eq!(swap.from.as_ref().map(|f| f.title.as_str()), Some("One"));
        assert!(swap.moving(1_200_000));
        assert_eq!(swap.step(&track("Two"), 3_000_000, true).0, None, "and settles");
        assert!(swap.from.is_none());
        // Off, it just shows.
        assert_eq!(swap.step(&track("Three"), 4_000_000, false).0, None);
    }

    #[test]
    fn the_icons_make_way_only_when_the_buttons_come_or_go_on_a_screen() {
        let mut shown = Vec::new();
        assert!(note_shown(&mut shown, "DP-2", true));
        assert!(!note_shown(&mut shown, "DP-2", true), "a redraw that changes nothing");
        assert!(note_shown(&mut shown, "HDMI-A-1", true));
        assert!(note_shown(&mut shown, "DP-2", false));
        assert_eq!(shown, ["HDMI-A-1"]);
        assert!(!note_shown(&mut shown, "DP-2", false));
    }

    #[test]
    fn media_buttons_on_screen_sit_where_the_widget_draws_them() {
        let screen = (1920, 1080);
        for (corner, k) in [((true, true), 1.0), ((false, false), 1.5), ((true, false), 0.6)] {
            let (top, left) = corner;
            let at = media_buttons_at(screen, corner, k, (10, 20), 40.0);
            assert_eq!(at.len(), 3);
            // The drawn area, as `rects` places it.
            let (w, h) = (MEDIA_W * k, ART * k);
            let x = if left { 10.0 + AIR } else { 1920.0 - 10.0 - AIR - w };
            let y = if top { 40.0 + AIR } else { 1080.0 - 20.0 - AIR - h };
            for r in &at {
                assert!(r.x >= x && r.y >= y && r.x + r.w <= x + w + 0.01 && r.y + r.h <= y + h + 0.01, "{corner:?} {k}");
                assert!((r.w - 36.0 * k).abs() < 1e-9);
            }
            assert!(at[0].x + at[0].w <= at[1].x && at[1].x + at[1].w <= at[2].x, "previous, play, next in a row");
        }
    }

    #[test]
    fn cards_fit_in_the_air_around_each_widget() {
        const { assert!(CARD_PAD < AIR) };
        for light in [false, true] {
            let on = look_for(light, true, false, Ink::Auto, Tone::Auto, RED);
            assert!(on.card.is_some() && on.shade.is_none(), "a card's ink needs no shadow");
            let off = look_for(light, false, false, Ink::Auto, Tone::Auto, RED);
            assert!(off.card.is_none() && off.shade.is_some());
            assert!(look_for(light, false, true, Ink::Auto, Tone::Auto, RED).shade.is_none(), "reduced effects drop the shadow");
            // Dark ink over light, light over dark.
            assert_eq!(on.ink.r > 0.5, !light);
            // A color that was set holds over any wallpaper, its card the other way round.
            let white = look_for(light, true, false, Ink::White, Tone::Auto, RED);
            assert!(white.ink.r > 0.9 && white.card.is_some_and(|c| c.r < 0.5));
            let black = look_for(light, true, false, Ink::Black, Tone::Auto, RED);
            assert!(black.ink.r < 0.1 && black.card.is_some_and(|c| c.r > 0.5));
            assert_eq!(look_for(light, false, false, Ink::Accent, Tone::Auto, RED).ink, RED);
            // A card tone that was set holds over any wallpaper, and Automatic text follows it.
            let pale = look_for(light, true, false, Ink::Auto, Tone::Light, RED);
            assert!(pale.card.is_some_and(|c| c.r > 0.5) && pale.ink.r < 0.1);
            let deep = look_for(light, true, false, Ink::Auto, Tone::Dark, RED);
            assert!(deep.card.is_some_and(|c| c.r < 0.5) && deep.ink.r > 0.9);
            // Without a card the tone changes nothing.
            assert_eq!(look_for(light, false, false, Ink::Auto, Tone::Light, RED).ink.r, off.ink.r);
        }
    }
}
