use crate::app::App;
use crate::compositor;
use crate::draw::{self, Face};
use crate::gsr::{self, Mode};
use crate::paint::Paint;
use gtk::{cairo, gdk, glib, graphene, pango, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kit::motion::{self, Curve, Tween};
use kit::palette::Rgb;
use kit::shape;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

const TILE_W: f64 = 212.0;
const TILE_H: f64 = 164.0;
const SIDE_W: f64 = 148.0;
const GAP: f64 = 14.0;
const PAD: f64 = 24.0;
const TOP: f64 = 64.0;
const ACTS: f64 = 52.0;
const PILL_H: f64 = 32.0;
const ROW_H: f64 = 40.0;
const CHIP_H: f64 = 30.0;
const LABEL_W: f64 = 118.0;
const RADIUS: f64 = 30.0;
const FILLET: f64 = 16.0;
const W: f64 = 3.0 * TILE_W + SIDE_W + 3.0 * GAP + 2.0 * PAD;
const RED: Rgb = Rgb::new(1.0, 0.27, 0.23);
const OPEN: Curve = Curve::new(0.16, 1.0, 0.3, 1.0);
const CLOSE: Curve = Curve::new(0.4, 0.0, 0.9, 0.6);
/// Long enough for the close motion to finish, so the panel is never in the first frames.
const CLEAR_MS: u64 = 280;
const TILES: [Mode; 3] = [Mode::Replay, Mode::Record, Mode::Stream];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Hit {
    Tile(Mode),
    Videos,
    Settings,
    Save(u32),
    Pause,
    Chip(usize, usize),
}

struct Item {
    hit: Hit,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

struct Row {
    label: &'static str,
    key: &'static str,
    /// Each chip is its own on/off setting instead of one choice among the row.
    toggles: bool,
    chips: Vec<(String, String)>,
}

struct State {
    app: Rc<App>,
    window: gtk::Window,
    paint: Paint,
    open: bool,
    rows: Vec<Row>,
    prefs: toml::Table,
    hot: Option<Hit>,
    keyed: bool,
    rise: Tween,
    fade: Tween,
    ticking: bool,
    clock: Option<glib::SourceId>,
}

type Shared = Rc<RefCell<State>>;

thread_local! {
    static ONE: RefCell<Option<Shared>> = const { RefCell::new(None) };
}

fn get(app: &Rc<App>) -> Shared {
    ONE.with(|one| one.borrow_mut().get_or_insert_with(|| build(app)).clone())
}

fn current() -> Option<Shared> {
    ONE.with(|one| one.borrow().clone())
}

pub fn toggle(app: &Rc<App>) {
    let st = get(app);
    let open = st.borrow().open;
    if open { close(&st) } else { show(&st) }
}

/// The recorder changed state: redraw the open panel and keep its clock running while needed.
pub fn changed() {
    if let Some(st) = current() {
        if st.try_borrow().is_ok_and(|s| s.open) {
            tick_clock(&st);
            st.borrow().paint.queue_draw();
        }
    }
}

/// Closes the panel, then runs `then` once it is off the screen.
pub fn after_close(then: impl FnOnce() + 'static) {
    match current() {
        Some(st) if st.borrow().open => {
            close(&st);
            glib::timeout_add_local_once(Duration::from_millis(CLEAR_MS), then);
        }
        _ => then(),
    }
}

fn rows(prefs: &toml::Table) -> Vec<Row> {
    let screens: Vec<String> = gdk::Display::default()
        .map(|d| d.monitors().iter::<gdk::Monitor>().filter_map(Result::ok).filter_map(|m| m.connector().map(|c| c.to_string())).collect())
        .unwrap_or_default();
    let mut capture = vec![("monitor".to_string(), if screens.len() > 1 { "Focused screen" } else { "Screen" }.to_string())];
    if screens.len() > 1 {
        capture.extend(screens.iter().map(|s| (s.clone(), s.clone())));
    }
    capture.extend([("region".to_string(), "Region".to_string()), ("portal".to_string(), "Window".to_string())]);
    let pairs = |list: &[(&str, &str)]| list.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect::<Vec<_>>();
    let mut fps = pairs(&[("30", "30"), ("60", "60"), ("120", "120"), ("144", "144")]);
    let current = kit::prefs::text(prefs, "rec_fps", "60");
    if !fps.iter().any(|f| f.0 == current) {
        fps.push((current.clone(), current));
    }
    vec![
        Row { label: "Capture", key: "rec_target", toggles: false, chips: capture },
        Row { label: "Sound", key: "", toggles: true, chips: pairs(&[("rec_system_audio", "System sound"), ("rec_mic", "Microphone")]) },
        Row { label: "Quality", key: "rec_quality", toggles: false, chips: pairs(&[("medium", "Medium"), ("high", "High"), ("very_high", "Very high"), ("ultra", "Ultra")]) },
        Row { label: "Frame rate", key: "rec_fps", toggles: false, chips: fps },
        Row { label: "Replay length", key: "rec_replay", toggles: false, chips: pairs(&[("15", "15 s"), ("30", "30 s"), ("60", "1 min"), ("120", "2 min"), ("300", "5 min")]) },
    ]
}

fn build(app: &Rc<App>) -> Shared {
    let window = gtk::Window::new();
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace(Some("digit-record"));
    for e in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(e, true);
    }
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(KeyboardMode::None);
    window.add_css_class("digit");
    let paint = Paint::new(-1, -1);
    window.set_child(Some(&paint));
    let prefs = kit::prefs::read();
    let st = Rc::new(RefCell::new(State {
        app: app.clone(),
        window: window.clone(),
        paint: paint.clone(),
        open: false,
        rows: rows(&prefs),
        prefs,
        hot: None,
        keyed: false,
        rise: Tween::still(0.0),
        fade: Tween::still(0.0),
        ticking: false,
        clock: None,
    }));
    let weak = Rc::downgrade(&st);
    paint.on_draw(move |s| {
        if let Some(st) = weak.upgrade() {
            if let Ok(st) = st.try_borrow() {
                st.draw(s);
            }
        }
    });
    let keys = gtk::EventControllerKey::new();
    let weak = Rc::downgrade(&st);
    keys.connect_key_pressed(move |_, key, _, _| {
        let Some(st) = weak.upgrade() else { return glib::Propagation::Proceed };
        match key {
            gdk::Key::Escape => close(&st),
            gdk::Key::Right | gdk::Key::Tab => step(&st, 1),
            gdk::Key::Left | gdk::Key::ISO_Left_Tab => step(&st, -1),
            gdk::Key::Down => vertical(&st, 1.0),
            gdk::Key::Up => vertical(&st, -1.0),
            gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::space => {
                let hot = st.borrow().hot;
                if let Some(hit) = hot {
                    press(&st, hit);
                }
            }
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    window.add_controller(keys);
    let motion = gtk::EventControllerMotion::new();
    let weak = Rc::downgrade(&st);
    motion.connect_motion(move |_, x, y| {
        let Some(st) = weak.upgrade() else { return };
        let at = st.borrow().hit_at(x, y);
        hover(&st, at, false);
    });
    paint.add_controller(motion);
    let click = gtk::GestureClick::new();
    let weak = Rc::downgrade(&st);
    click.connect_released(move |_, _, x, y| {
        let Some(st) = weak.upgrade() else { return };
        let (at, inside) = {
            let s = st.borrow();
            (s.hit_at(x, y), s.inside(x, y))
        };
        match at {
            Some(hit) => press(&st, hit),
            None if !inside => close(&st),
            None => {}
        }
    });
    paint.add_controller(click);
    st
}

fn show(st: &Shared) {
    {
        let mut s = st.borrow_mut();
        let output = compositor::focused_output();
        let monitors: Vec<gdk::Monitor> = gdk::Display::default().map(|d| d.monitors().iter::<gdk::Monitor>().filter_map(Result::ok).collect()).unwrap_or_default();
        let monitor = monitors.iter().find(|m| output.as_deref().is_some_and(|o| m.connector().as_deref() == Some(o))).or(monitors.first()).cloned();
        if !s.window.is_visible() {
            s.window.set_monitor(monitor.as_ref());
        }
        s.prefs = kit::prefs::read();
        s.rows = rows(&s.prefs);
        s.open = true;
        s.hot = Some(Hit::Tile(Mode::Record));
        s.keyed = false;
        let now = glib::monotonic_time();
        s.rise.go(1.0, now, 420, OPEN);
        s.fade.after(1.0, now, 60, 220, motion::STANDARD);
        s.window.set_keyboard_mode(KeyboardMode::Exclusive);
    }
    let window = st.borrow().window.clone();
    window.present();
    region(st);
    tick_clock(st);
    kick(st);
}

fn close(st: &Shared) {
    {
        let mut s = st.borrow_mut();
        if !s.open {
            return;
        }
        s.open = false;
        let now = glib::monotonic_time();
        s.rise.go(0.0, now, 240, CLOSE);
        s.fade.go(0.0, now, 180, motion::STANDARD);
        s.window.set_keyboard_mode(KeyboardMode::None);
        s.paint.set_cursor_from_name(None);
        if let Some(id) = s.clock.take() {
            id.remove();
        }
    }
    region(st);
    kick(st);
}

/// Redraws once a second while something is counting, so the timers move.
fn tick_clock(st: &Shared) {
    let mut s = st.borrow_mut();
    let counting = s.open && gsr::status().is_some();
    if counting == s.clock.is_some() {
        return;
    }
    if !counting {
        if let Some(id) = s.clock.take() {
            id.remove();
        }
        return;
    }
    let weak = Rc::downgrade(st);
    s.clock = Some(glib::timeout_add_local(Duration::from_millis(1000), move || {
        let Some(st) = weak.upgrade() else { return glib::ControlFlow::Break };
        let Ok(s) = st.try_borrow() else { return glib::ControlFlow::Continue };
        s.paint.queue_draw();
        glib::ControlFlow::Continue
    }));
}

fn press(st: &Shared, hit: Hit) {
    let app = st.borrow().app.clone();
    match hit {
        Hit::Tile(Mode::Record) => {
            if gsr::status().is_some() {
                gsr::toggle_record(&app);
            } else {
                after_close(move || gsr::toggle_record(&app));
            }
        }
        Hit::Tile(Mode::Replay) => {
            let _ = gsr::switch(&app, Mode::Replay, None);
        }
        Hit::Tile(Mode::Stream) => {
            if gsr::status().is_some_and(|s| s.mode == Mode::Stream) {
                gsr::stop();
            } else if gsr::stream_url().is_empty() {
                close(st);
                gsr::settings();
            } else {
                after_close(move || {
                    let _ = gsr::switch(&app, Mode::Stream, Some(true));
                });
            }
        }
        Hit::Save(secs) => {
            after_close(move || {
                gsr::save_replay((secs > 0).then_some(secs));
            });
        }
        Hit::Pause => {
            gsr::pause();
        }
        Hit::Videos => {
            close(st);
            gsr::open_folder();
        }
        Hit::Settings => {
            close(st);
            gsr::settings();
        }
        Hit::Chip(r, c) => {
            {
                let s = st.borrow();
                let Some(row) = s.rows.get(r) else { return };
                let Some((value, _)) = row.chips.get(c) else { return };
                if row.toggles {
                    let on = kit::prefs::flag(&s.prefs, value, value == "rec_system_audio");
                    let _ = kit::prefs::set(value, toml::Value::Boolean(!on));
                } else {
                    let _ = kit::prefs::set(row.key, toml::Value::String(value.clone()));
                }
            }
            let mut s = st.borrow_mut();
            s.prefs = kit::prefs::read();
            s.paint.queue_draw();
        }
    }
    st.borrow().paint.queue_draw();
}

fn hover(st: &Shared, at: Option<Hit>, keyed: bool) {
    let mut s = st.borrow_mut();
    if !s.open || (s.hot == at && s.keyed == keyed) {
        return;
    }
    if at.is_none() && !keyed && s.keyed {
        return;
    }
    s.hot = at;
    s.keyed = keyed;
    s.paint.set_cursor_from_name(at.is_some().then_some("pointer"));
    s.paint.queue_draw();
}

fn step(st: &Shared, by: i32) {
    let next = {
        let s = st.borrow();
        let items = s.items();
        if items.is_empty() {
            return;
        }
        let at = s.hot.and_then(|h| items.iter().position(|i| i.hit == h)).map_or(0, |p| p as i32 + by);
        items[at.rem_euclid(items.len() as i32) as usize].hit
    };
    hover(st, Some(next), true);
}

fn vertical(st: &Shared, dir: f64) {
    let next = {
        let s = st.borrow();
        let items = s.items();
        let Some(cur) = s.hot.and_then(|h| items.iter().find(|i| i.hit == h)) else {
            drop(s);
            return step(st, 0);
        };
        let (cx, cy) = (cur.x + cur.w / 2.0, cur.y + cur.h / 2.0);
        items
            .iter()
            .filter(|i| (i.y + i.h / 2.0 - cy) * dir > 4.0)
            .min_by(|a, b| {
                let score = |i: &Item| ((i.y + i.h / 2.0 - cy).abs() * 3.0 + (i.x + i.w / 2.0 - cx).abs()) as i64;
                score(a).cmp(&score(b))
            })
            .map(|i| i.hit)
    };
    if let Some(n) = next {
        hover(st, Some(n), true);
    }
}

fn region(st: &Shared) {
    let s = st.borrow();
    let Some(surface) = s.window.surface() else { return };
    if s.open {
        surface.set_input_region(None);
    } else {
        surface.set_input_region(Some(&cairo::Region::create()));
    }
}

fn kick(st: &Shared) {
    let paint = {
        let mut s = st.borrow_mut();
        if s.ticking {
            return;
        }
        s.ticking = true;
        s.paint.clone()
    };
    let weak = Rc::downgrade(st);
    paint.add_tick_callback(move |w, clock| {
        let Some(st) = weak.upgrade() else { return glib::ControlFlow::Break };
        let Ok(mut s) = st.try_borrow_mut() else { return glib::ControlFlow::Continue };
        let now = clock.frame_time();
        w.queue_draw();
        if !s.rise.settled(now) || !s.fade.settled(now) {
            return glib::ControlFlow::Continue;
        }
        s.ticking = false;
        if !s.open && s.window.is_visible() {
            let window = s.window.clone();
            draw::forget(&s.paint);
            drop(s);
            window.set_visible(false);
            glib::idle_add_local_once(move || {
                let idle = ONE.with(|one| one.borrow().as_ref().is_some_and(|st| st.try_borrow().is_ok_and(|s| !s.open)));
                if idle {
                    let gone = ONE.with(|one| one.borrow_mut().take());
                    window.destroy();
                    drop(gone);
                    App::trim();
                }
            });
        }
        glib::ControlFlow::Break
    });
}

fn pill_w(s: &State, text: &str) -> f64 {
    draw::width(&s.paint, &s.app.cfg, text, 13.0, Face::Text(560)).ceil() + 30.0
}

impl State {
    fn origin(&self) -> f64 {
        ((self.paint.width() as f64 - W) / 2.0).round()
    }

    fn rows_top(&self) -> f64 {
        TOP + TILE_H + ACTS + 8.0
    }

    fn height(&self) -> f64 {
        self.rows_top() + self.rows.len() as f64 * ROW_H + PAD - 6.0
    }

    fn inside(&self, x: f64, y: f64) -> bool {
        let x0 = self.origin();
        x >= x0 && x <= x0 + W && y <= self.height()
    }

    fn hit_at(&self, x: f64, y: f64) -> Option<Hit> {
        if !self.open {
            return None;
        }
        let (x, y) = (x - self.origin(), y);
        self.items().into_iter().find(|i| x >= i.x && x <= i.x + i.w && y >= i.y && y <= i.y + i.h).map(|i| i.hit)
    }

    fn tile_x(i: usize) -> f64 {
        PAD + i as f64 * (TILE_W + GAP)
    }

    fn enabled(mode: Mode, st: Option<gsr::Status>) -> bool {
        match (mode, st) {
            (Mode::Record, _) | (_, None) => !gsr::busy(),
            (m, Some(s)) => s.mode == m,
        }
    }

    fn pills(&self, mode: Mode, st: Option<gsr::Status>) -> Vec<(Hit, String)> {
        match (mode, st) {
            (Mode::Replay, Some(s)) if s.mode == Mode::Replay => {
                let mut out = vec![(Hit::Save(0), format!("Save {}", gsr::span(s.replay)))];
                if s.replay > 30 {
                    out.push((Hit::Save(30), "Last 30 s".into()));
                }
                out
            }
            (Mode::Record, Some(s)) if s.mode == Mode::Record => vec![(Hit::Pause, if s.paused { "Resume" } else { "Pause" }.into())],
            _ => Vec::new(),
        }
    }

    fn items(&self) -> Vec<Item> {
        let st = gsr::status();
        let mut out = Vec::new();
        for (i, mode) in TILES.iter().enumerate() {
            let x = Self::tile_x(i);
            if Self::enabled(*mode, st) {
                out.push(Item { hit: Hit::Tile(*mode), x, y: TOP, w: TILE_W, h: TILE_H });
            }
            let pills = self.pills(*mode, st);
            let widths: Vec<f64> = pills.iter().map(|(_, t)| pill_w(self, t)).collect();
            let total = widths.iter().sum::<f64>() + 8.0 * (widths.len() as f64 - 1.0).max(0.0);
            let mut px = x + (TILE_W - total) / 2.0;
            for ((hit, _), w) in pills.iter().zip(widths) {
                out.push(Item { hit: *hit, x: px, y: TOP + TILE_H + 10.0, w, h: PILL_H });
                px += w + 8.0;
            }
        }
        let sx = Self::tile_x(3);
        let half = (TILE_H - GAP) / 2.0;
        out.push(Item { hit: Hit::Videos, x: sx, y: TOP, w: SIDE_W, h: half });
        out.push(Item { hit: Hit::Settings, x: sx, y: TOP + half + GAP, w: SIDE_W, h: half });
        let top = self.rows_top();
        for (r, row) in self.rows.iter().enumerate() {
            let mut x = PAD + LABEL_W;
            let y = top + r as f64 * ROW_H + (ROW_H - CHIP_H) / 2.0;
            for (c, (_, label)) in row.chips.iter().enumerate() {
                let w = pill_w(self, label) - 4.0;
                out.push(Item { hit: Hit::Chip(r, c), x, y, w, h: CHIP_H });
                x += w + 8.0;
            }
        }
        out
    }

    fn chosen(&self, r: usize, c: usize) -> bool {
        let Some(row) = self.rows.get(r) else { return false };
        let Some((value, _)) = row.chips.get(c) else { return false };
        if row.toggles {
            return kit::prefs::flag(&self.prefs, value, value == "rec_system_audio");
        }
        let fallback = match row.key {
            "rec_target" => "monitor",
            "rec_quality" => "very_high",
            "rec_fps" | "rec_replay" => "60",
            _ => "",
        };
        let current = kit::prefs::text(&self.prefs, row.key, fallback);
        if row.key == "rec_target" && !row.chips.iter().any(|(v, _)| *v == current) {
            return value == "monitor";
        }
        current == *value
    }

    fn draw(&self, s: &gtk::Snapshot) {
        let now = self.paint.frame_clock().map(|c| c.frame_time()).unwrap_or_else(glib::monotonic_time);
        let fade = self.fade.value(now).clamp(0.0, 1.0);
        let rise = self.rise.value(now);
        if fade < 0.003 && rise < 0.003 {
            return;
        }
        let pal = self.app.palette_at(now);
        let cfg = &self.app.cfg;
        let (sw, sh) = (self.paint.width() as f64, self.paint.height() as f64);
        let dim = rise.clamp(0.0, 1.0);
        s.append_color(&Rgb::new(0.0, 0.0, 0.0).alpha(0.42 * dim).rgba(), &graphene::Rect::new(0.0, 0.0, sw as f32, sh as f32));
        let h = self.height();
        let lift = ((1.0 - rise) * (h + FILLET)).round();
        let x0 = self.origin();
        s.save();
        s.translate(&graphene::Point::new(0.0, -lift as f32));
        if let Some(p) = shape::docked_parts(sw / 2.0, 0.0, W, h, RADIUS, FILLET) {
            draw::slab(s, &p, pal.body.alpha(1.0));
        }
        s.translate(&graphene::Point::new(x0 as f32, 0.0));
        s.push_opacity(fade);
        self.draw_body(s, &pal, cfg);
        s.pop();
        s.restore();
    }

    fn draw_body(&self, s: &gtk::Snapshot, pal: &kit::palette::Palette, cfg: &crate::config::Config) {
        let left = pango::Alignment::Left;
        let st = gsr::status();
        let items = self.items();
        let hot = |hit: Hit| self.hot == Some(hit);
        let ring = |s: &gtk::Snapshot, hit: Hit, x: f64, y: f64, w: f64, h: f64, r: f64| {
            if self.keyed && hot(hit) {
                s.append_border(&draw::round(x - 3.0, y - 3.0, w + 6.0, h + 6.0, r + 3.0), &[2.0; 4], &[pal.accent.rgba(); 4]);
            }
        };

        // Header: the name on the left, what is running on the right.
        let mid = TOP / 2.0 + 2.0;
        draw::icon(s, &self.paint, cfg, "screen_record", 22.0, true, pal.accent, PAD + 11.0, mid);
        draw::text(s, &self.paint, cfg, "Recorder", 17.0, Face::Text(650), pal.ink_main, PAD + 30.0, mid, 200.0, left);
        let (status, ink) = match st {
            None if gsr::busy() => ("Starting…".to_string(), pal.ink_meta),
            None if !gsr::installed() => ("gpu-screen-recorder is not installed".to_string(), RED),
            None => ("Esc to close".to_string(), pal.ink_faint),
            Some(s) => {
                let what = match (s.mode, s.taping, s.paused) {
                    (Mode::Record, _, true) => format!("Paused · {}", gsr::clock(s.secs)),
                    (Mode::Record, _, false) | (Mode::Replay, true, _) => format!("Recording · {}", gsr::clock(s.secs)),
                    (Mode::Replay, false, _) => format!("Instant replay on · last {}", gsr::span(s.replay)),
                    (Mode::Stream, _, _) => format!("Live · {}", gsr::clock(s.secs)),
                };
                (what, if s.mode == Mode::Replay && !s.taping { pal.accent } else { RED })
            }
        };
        let tw = draw::width(&self.paint, cfg, &status, 13.0, Face::Number(560)).ceil();
        if st.is_some() {
            let dot = draw::round(W - PAD - tw - 16.0, mid - 4.0, 8.0, 8.0, 4.0);
            draw::fill(s, &dot, ink);
        }
        draw::text(s, &self.paint, cfg, &status, 13.0, Face::Number(560), ink, W - PAD - tw, mid, tw + 2.0, left);

        // The three big tiles.
        for (i, mode) in TILES.iter().enumerate() {
            let x = Self::tile_x(i);
            let y = TOP;
            let on = match (mode, st) {
                (Mode::Record, Some(s)) => s.mode == Mode::Record || s.taping,
                (m, Some(s)) => s.mode == *m,
                (_, None) => false,
            };
            let live = Self::enabled(*mode, st);
            let color = if *mode == Mode::Replay { pal.accent } else { RED };
            let glow = if hot(Hit::Tile(*mode)) && live { 0.06 } else { 0.0 };
            let back = if on { color.alpha(0.16 + glow) } else { pal.ink_main.alpha(0.07 + glow) };
            draw::fill(s, &draw::round(x, y, TILE_W, TILE_H, 22.0), back);
            ring(s, Hit::Tile(*mode), x, y, TILE_W, TILE_H, 22.0);
            s.push_opacity(if live { 1.0 } else { 0.4 });
            let disc = 50.0;
            draw::fill(s, &draw::round(x + 18.0, y + 18.0, disc, disc, disc / 2.0), if on { color } else { pal.ink_main.alpha(0.1) });
            let (glyph, title) = match mode {
                Mode::Replay => ("history", "Instant Replay"),
                Mode::Record => (if on { "stop" } else { "radio_button_checked" }, "Record"),
                Mode::Stream => ("cell_tower", "Stream"),
            };
            let glyph = if *mode == Mode::Record && st.is_some_and(|s| s.paused) { "pause" } else { glyph };
            draw::icon(s, &self.paint, cfg, glyph, 26.0, true, match (on, mode) {
                (false, _) => pal.ink_main,
                (true, Mode::Replay) => pal.on_accent,
                (true, _) => Rgb::new(1.0, 1.0, 1.0),
            }, x + 18.0 + disc / 2.0, y + 18.0 + disc / 2.0);
            if on {
                let badge = match mode {
                    Mode::Replay => "ON",
                    Mode::Record => "REC",
                    Mode::Stream => "LIVE",
                };
                let bw = draw::width(&self.paint, cfg, badge, 11.0, Face::Text(700)).ceil() + 18.0;
                draw::fill(s, &draw::round(x + TILE_W - 16.0 - bw, y + 18.0, bw, 22.0, 11.0), color.alpha(0.22));
                draw::text(s, &self.paint, cfg, badge, 11.0, Face::Text(700), color, x + TILE_W - 16.0 - bw, y + 29.0, bw, pango::Alignment::Center);
            }
            let sub = match (mode, st) {
                (Mode::Replay, Some(s)) if s.mode == Mode::Replay => format!("Keeping the last {}", gsr::span(s.replay)),
                (Mode::Replay, _) => format!("Off · keeps {}", gsr::span(kit::prefs::text(&self.prefs, "rec_replay", "60").parse().unwrap_or(60))),
                (Mode::Record, Some(s)) if s.mode == Mode::Record || s.taping => {
                    if s.paused { format!("Paused · {}", gsr::clock(s.secs)) } else { gsr::clock(s.secs) }
                }
                (Mode::Record, Some(_)) => "Records on top of the buffer".into(),
                (Mode::Record, None) => "Starts when this closes".into(),
                (Mode::Stream, Some(s)) if s.mode == Mode::Stream => format!("Live · {}", gsr::clock(s.secs)),
                (Mode::Stream, _) if gsr::stream_url().is_empty() => "Add an address in Settings".into(),
                (Mode::Stream, _) => "Off".into(),
            };
            draw::text(s, &self.paint, cfg, title, 17.0, Face::Text(620), pal.ink_main, x + 20.0, y + TILE_H - 48.0, TILE_W - 40.0, left);
            let face = if on && *mode != Mode::Replay { Face::Number(520) } else { Face::Text(450) };
            draw::text(s, &self.paint, cfg, &sub, 13.0, face, if on { color.mix(pal.ink_main, 0.5) } else { pal.ink_meta }, x + 20.0, y + TILE_H - 24.0, TILE_W - 40.0, left);
            s.pop();
        }

        // The pills under the tiles, the two small tiles and the option rows.
        for item in &items {
            let label = match item.hit {
                Hit::Save(0) => st.map(|s| format!("Save {}", gsr::span(s.replay))).unwrap_or_default(),
                Hit::Save(n) => format!("Last {}", gsr::span(n)),
                Hit::Pause => if st.is_some_and(|s| s.paused) { "Resume".into() } else { "Pause".into() },
                _ => continue,
            };
            let glow = if hot(item.hit) { 0.08 } else { 0.0 };
            let primary = matches!(item.hit, Hit::Save(0));
            let back = if primary { pal.accent.alpha(0.9 + glow) } else { pal.ink_main.alpha(0.09 + glow) };
            draw::fill(s, &draw::round(item.x, item.y, item.w, item.h, item.h / 2.0), back);
            ring(s, item.hit, item.x, item.y, item.w, item.h, item.h / 2.0);
            draw::text(s, &self.paint, cfg, &label, 13.0, Face::Text(560), if primary { pal.on_accent } else { pal.ink_main }, item.x, item.y + item.h / 2.0, item.w, pango::Alignment::Center);
        }
        let sx = Self::tile_x(3);
        let half = (TILE_H - GAP) / 2.0;
        for (hit, glyph, label, y) in [(Hit::Videos, "video_library", "Videos", TOP), (Hit::Settings, "tune", "Settings", TOP + half + GAP)] {
            let glow = if hot(hit) { 0.06 } else { 0.0 };
            draw::fill(s, &draw::round(sx, y, SIDE_W, half, 18.0), pal.ink_main.alpha(0.07 + glow));
            ring(s, hit, sx, y, SIDE_W, half, 18.0);
            draw::icon(s, &self.paint, cfg, glyph, 22.0, false, pal.ink_main, sx + 30.0, y + half / 2.0);
            draw::text(s, &self.paint, cfg, label, 14.0, Face::Text(560), pal.ink_main, sx + 52.0, y + half / 2.0, SIDE_W - 60.0, left);
        }
        let top = self.rows_top();
        draw::fill(s, &draw::round(PAD, top - 6.0, W - 2.0 * PAD, 1.0, 0.0), pal.hairline);
        for (r, row) in self.rows.iter().enumerate() {
            let mid = top + r as f64 * ROW_H + ROW_H / 2.0;
            draw::text(s, &self.paint, cfg, row.label, 13.0, Face::Text(500), pal.ink_meta, PAD + 2.0, mid, LABEL_W - 10.0, left);
        }
        for item in &items {
            let Hit::Chip(r, c) = item.hit else { continue };
            let Some((_, label)) = self.rows.get(r).and_then(|row| row.chips.get(c)) else { continue };
            let on = self.chosen(r, c);
            let glow = if hot(item.hit) { 0.07 } else { 0.0 };
            let back = if on { pal.accent.alpha(1.0) } else { pal.ink_main.alpha(0.07 + glow) };
            draw::fill(s, &draw::round(item.x, item.y, item.w, item.h, item.h / 2.0), back);
            ring(s, item.hit, item.x, item.y, item.w, item.h, item.h / 2.0);
            draw::text(s, &self.paint, cfg, label, 13.0, Face::Text(540), if on { pal.on_accent } else { pal.ink_main }, item.x, item.y + item.h / 2.0, item.w, pango::Alignment::Center);
        }
    }
}
