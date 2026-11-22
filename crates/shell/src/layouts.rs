use crate::app::App;
use crate::compositor;
use crate::draw::{self, Face};
use crate::paint::Paint;
use gtk::{gdk, gio, glib, pango, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kit::motion::{self, Tween};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::rc::Rc;

const COLS: usize = 5;
const TILE_W: f64 = 100.0;
const TILE_H: f64 = 64.0;
use crate::metrics::{GRID_GAP as GAP, PANEL_PAD as PAD};
const HEAD: f64 = 34.0;
const FOOT: f64 = 40.0;
const W: f64 = 2.0 * PAD + COLS as f64 * TILE_W + (COLS - 1) as f64 * GAP;
const H: f64 = PAD + HEAD + 2.0 * TILE_H + GAP + FOOT;

type Region = (f64, f64, f64, f64);

#[derive(Clone, Copy)]
enum Kind {
    Columns(&'static [f64]),
    Stack,
    Centered,
    Float(Region),
}

struct Layout {
    name: &'static str,
    label: &'static str,
    kind: Kind,
    regions: &'static [Region],
}

const LAYOUTS: [Layout; 10] = [
    Layout { name: "fill", label: "Fill", kind: Kind::Columns(&[100.0]), regions: &[(0.0, 0.0, 1.0, 1.0)] },
    Layout { name: "halves", label: "Side by side", kind: Kind::Columns(&[50.0, 50.0]), regions: &[(0.0, 0.0, 0.5, 1.0), (0.5, 0.0, 0.5, 1.0)] },
    Layout { name: "thirds", label: "Three columns", kind: Kind::Columns(&[100.0 / 3.0, 100.0 / 3.0, 100.0 / 3.0]), regions: &[(0.0, 0.0, 1.0 / 3.0, 1.0), (1.0 / 3.0, 0.0, 1.0 / 3.0, 1.0), (2.0 / 3.0, 0.0, 1.0 / 3.0, 1.0)] },
    Layout { name: "focus", label: "Two thirds", kind: Kind::Columns(&[200.0 / 3.0, 100.0 / 3.0]), regions: &[(0.0, 0.0, 2.0 / 3.0, 1.0), (2.0 / 3.0, 0.0, 1.0 / 3.0, 1.0)] },
    Layout { name: "stack", label: "Half and stack", kind: Kind::Stack, regions: &[(0.0, 0.0, 0.5, 1.0), (0.5, 0.0, 0.5, 0.5), (0.5, 0.5, 0.5, 0.5)] },
    Layout { name: "center", label: "Floating center", kind: Kind::Float((0.2, 0.15, 0.6, 0.7)), regions: &[(0.2, 0.15, 0.6, 0.7)] },
    Layout { name: "top-left", label: "Top left", kind: Kind::Float((0.0, 0.0, 0.5, 0.5)), regions: &[(0.0, 0.0, 0.5, 0.5)] },
    Layout { name: "top-right", label: "Top right", kind: Kind::Float((0.5, 0.0, 0.5, 0.5)), regions: &[(0.5, 0.0, 0.5, 0.5)] },
    Layout { name: "bottom-left", label: "Bottom left", kind: Kind::Float((0.0, 0.5, 0.5, 0.5)), regions: &[(0.0, 0.5, 0.5, 0.5)] },
    Layout { name: "centered", label: "Centered column", kind: Kind::Centered, regions: &[(1.0 / 6.0, 0.0, 2.0 / 3.0, 1.0)] },
];

struct State {
    app: Rc<App>,
    window: gtk::Window,
    paint: Paint,
    target: Option<u64>,
    selected: usize,
    glow: Vec<Tween>,
    fade: Tween,
    seen: bool,
    area: (f64, f64),
}

type Shared = Rc<RefCell<State>>;

thread_local! {
    static ONE: RefCell<Option<Shared>> = const { RefCell::new(None) };
}

pub fn names() -> Vec<&'static str> {
    LAYOUTS.iter().map(|l| l.name).collect()
}

pub fn niri_gap() -> f64 {
    let path = kit::niriconf::dir().join("digit-compositor.kdl");
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| t.lines().map(str::trim).find(|l| l.starts_with("gaps ")).and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok()))
        .unwrap_or(16.0)
}

pub fn working_area(app: &App) -> (f64, f64) {
    let output = compositor::focused_output();
    let m = crate::screens::for_output(output.as_deref());
    let (w, h) = m.map_or((1920.0, 1080.0), |m| (m.geometry().width() as f64, m.geometry().height() as f64));
    let top = app.reserve().max(0) as f64;
    let dock = app.prefs.borrow().dock.clone();
    let on_dock = crate::dock::mode(app) == "on" && (dock.output.is_none() || dock.output == output);
    let reserve = if on_dock { 8.0 + dock.size.clamp(28.0, 96.0).round() * 1.36 } else { 0.0 };
    match dock.side.as_str() {
        "left" | "right" => (w - reserve, h - top),
        _ => (w, h - top - reserve),
    }
}

pub fn apply_named(app: &App, name: &str) -> bool {
    let Some(i) = LAYOUTS.iter().position(|l| l.name == name) else { return false };
    run(i, None, working_area(app));
    true
}

/// Hyprland's tiling layouts, in the order the shortcuts step through them.
const TILINGS: [(&str, &str); 3] = kit::hyprconf::LAYOUTS;

fn next_tiling_with_prefs(now: &str, t: &toml::Table) -> (&'static str, &'static str) {
    let off: Vec<&str> = t.get("hypr_tiling_cycle_off").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|v| v.as_str()).collect()).unwrap_or_default();
    let on: Vec<(&'static str, &'static str)> = TILINGS.iter().copied().filter(|l| !off.contains(&l.0)).collect();
    let on = if on.len() < 2 { TILINGS.to_vec() } else { on };
    let now = kit::hyprconf::selected_layout(t, now);
    let start = TILINGS.iter().position(|l| l.0 == now).unwrap_or(0);
    (1..=TILINGS.len()).map(|k| TILINGS[(start + k) % TILINGS.len()]).find(|l| on.contains(l)).unwrap_or(on[0])
}

/// A short notch flash for a layout change, like the keyboard layout's.
fn flash(app: &Rc<App>, key: &str, text: String) {
    let icon = match key {
        "master" => "view_quilt",
        "scrolling" => "view_column",
        _ => "auto_awesome_mosaic",
    };
    app.request(crate::app::Peek::Osd { icon, text, level: None });
}

/// Hyprland: every workspace moves to the next tiling layout, saved like Settings > Windows
/// > Tiling. A global change clears workspace overrides.
fn hypr_next(app: &Rc<App>) {
    let prefs = kit::prefs::read();
    let (key, name) = next_tiling_with_prefs(kit::hyprconf::tiled_layout(&prefs), &prefs);
    let app = app.clone();
    glib::spawn_future_local(async move {
        let saved = gio::spawn_blocking(move || {
            let values = [
                ("hypr_tiling", toml::Value::String(key.into())),
                ("hypr_workspace_layouts", toml::Value::Table(toml::Table::new())),
            ];
            kit::prefs::set_many(&values)?;
            kit::setup::apply()
        }).await;
        match saved {
            Ok(Ok(())) => flash(&app, key, name.into()),
            Ok(Err(e)) => app.request(crate::app::Peek::Note { text: e }),
            Err(_) => app.request(crate::app::Peek::Note { text: "Could not change the layout".into() }),
        }
    });
}

/// Hyprland only: the workspace in front moves to the next tiling layout; the others stay.
pub fn workspace_next(app: &Rc<App>) {
    if kit::wm::which() != kit::wm::Wm::Hyprland {
        return;
    }
    let app = app.clone();
    glib::spawn_future_local(async move {
        let done = gio::spawn_blocking(|| {
            let out = std::process::Command::new("hyprctl").args(["-j", "activeworkspace"]).output().ok()?;
            let ws: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
            let id = ws.get("id")?.as_i64()?;
            // Named and special workspaces do not take part in workspace modes.
            if id < 1 { return None; }
            // The next mode is worked out from the stored one in the same locked change.
            let mut picked = None;
            kit::prefs::edit(|prefs| {
                let mut modes = prefs.get("hypr_workspace_layouts").and_then(toml::Value::as_table).cloned().unwrap_or_default();
                let now = modes.get(&id.to_string()).and_then(toml::Value::as_str)
                    .or_else(|| ws.get("tiledLayout").and_then(Value::as_str)).unwrap_or(kit::hyprconf::tiled_layout(prefs));
                let (key, name) = next_tiling_with_prefs(now, prefs);
                modes.insert(id.to_string(), toml::Value::String(key.into()));
                prefs.insert("hypr_workspace_layouts".into(), toml::Value::Table(modes));
                picked = Some((key, name));
                true
            }).ok()?;
            let (key, name) = picked?;
            kit::setup::apply().ok()?;
            Some((key, name))
        })
        .await;
        match done.ok().flatten() {
            Some((key, name)) => flash(&app, key, format!("{name} here")),
            None => app.request(crate::app::Peek::Note { text: "Could not change this workspace's layout".into() }),
        }
    });
}

pub fn toggle(app: &Rc<App>) {
    if !crate::compositor::niri_running() {
        if kit::wm::which() == kit::wm::Wm::Hyprland {
            hypr_next(app);
        }
        return;
    }
    if let Some(st) = ONE.with(|one| one.borrow().clone()) {
        close(&st);
        return;
    }
    let target = compositor::focused_id();
    let window = gtk::Window::new();
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace(Some("digit-layouts"));
    window.set_anchor(Edge::Top, true);
    window.set_margin(Edge::Top, app.reserve().max(0) + 14);
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(KeyboardMode::OnDemand);
    window.add_css_class("digit");
    window.set_default_size(W as i32, H as i32);
    let output = compositor::focused_output();
    window.set_monitor(crate::screens::for_output(output.as_deref()).as_ref());
    let paint = Paint::new(W as i32, H as i32);
    window.set_child(Some(&paint));
    let mut fade = Tween::still(0.0);
    fade.go(1.0, glib::monotonic_time(), 200, motion::STANDARD);
    let st = Rc::new(RefCell::new(State {
        app: app.clone(),
        window: window.clone(),
        paint: paint.clone(),
        target,
        selected: 0,
        glow: (0..LAYOUTS.len()).map(|i| Tween::still(if i == 0 { 1.0 } else { 0.0 })).collect(),
        fade,
        seen: false,
        area: working_area(app),
    }));
    let weak = Rc::downgrade(&st);
    paint.on_draw(move |s| {
        if let Some(st) = weak.upgrade() {
            if let Ok(st) = st.try_borrow() {
                st.draw(s);
            }
        }
    });
    let motion = gtk::EventControllerMotion::new();
    let weak = Rc::downgrade(&st);
    motion.connect_motion(move |_, x, y| {
        if let Some(st) = weak.upgrade() {
            let at = tile_at(x, y);
            if let Some(i) = at {
                select(&st, i);
            }
            st.borrow().paint.set_cursor_from_name(at.map(|_| "pointer"));
        }
    });
    paint.add_controller(motion);
    let click = gtk::GestureClick::new();
    let weak = Rc::downgrade(&st);
    click.connect_released(move |_, _, x, y| {
        if let Some(st) = weak.upgrade() {
            match tile_at(x, y) {
                Some(i) => {
                    select(&st, i);
                    commit(&st);
                }
                None => close(&st),
            }
        }
    });
    paint.add_controller(click);
    let keys = gtk::EventControllerKey::new();
    let weak = Rc::downgrade(&st);
    keys.connect_key_pressed(move |_, key, _, _| {
        let Some(st) = weak.upgrade() else { return glib::Propagation::Proceed };
        let cur = st.borrow().selected as i32;
        let n = LAYOUTS.len() as i32;
        let digit = key.to_unicode().and_then(|c| c.to_digit(10));
        match key {
            gdk::Key::Escape => close(&st),
            gdk::Key::Right | gdk::Key::Tab => select(&st, ((cur + 1).rem_euclid(n)) as usize),
            gdk::Key::Left | gdk::Key::ISO_Left_Tab => select(&st, ((cur - 1).rem_euclid(n)) as usize),
            gdk::Key::Down | gdk::Key::Up => select(&st, ((cur + COLS as i32).rem_euclid(n)) as usize),
            gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::space => commit(&st),
            _ if digit.is_some() => {
                let d = digit.unwrap_or(1) as usize;
                select(&st, if d == 0 { 9 } else { d - 1 });
                commit(&st);
            }
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    window.add_controller(keys);
    let weak = Rc::downgrade(&st);
    window.connect_is_active_notify(move |w| {
        let Some(st) = weak.upgrade() else { return };
        let Ok(mut s) = st.try_borrow_mut() else { return };
        if w.is_active() {
            s.seen = true;
        } else if s.seen {
            drop(s);
            glib::idle_add_local_once(move || close(&st));
        }
    });
    ONE.with(|one| *one.borrow_mut() = Some(st.clone()));
    window.present();
    kick(&st);
}

fn tile_rect(i: usize) -> (f64, f64) {
    let (c, r) = ((i % COLS) as f64, (i / COLS) as f64);
    (PAD + c * (TILE_W + GAP), PAD + HEAD + r * (TILE_H + GAP))
}

fn tile_at(x: f64, y: f64) -> Option<usize> {
    (0..LAYOUTS.len()).find(|&i| {
        let (tx, ty) = tile_rect(i);
        x >= tx - GAP / 2.0 && x <= tx + TILE_W + GAP / 2.0 && y >= ty - GAP / 2.0 && y <= ty + TILE_H + GAP / 2.0
    })
}

fn select(st: &Shared, i: usize) {
    {
        let mut s = st.borrow_mut();
        if s.selected == i {
            return;
        }
        s.selected = i;
        let now = glib::monotonic_time();
        for (k, g) in s.glow.iter_mut().enumerate() {
            let on = k == i;
            g.go(if on { 1.0 } else { 0.0 }, now, if on { 150 } else { 260 }, motion::STANDARD);
        }
    }
    kick(st);
}

fn close(st: &Shared) {
    ONE.with(|one| one.borrow_mut().take());
    let window = st.borrow().window.clone();
    draw::forget(&st.borrow().paint);
    window.destroy();
    App::trim();
}

fn commit(st: &Shared) {
    let (i, target, area) = {
        let s = st.borrow();
        (s.selected, s.target, s.area)
    };
    close(st);
    glib::timeout_add_local_once(std::time::Duration::from_millis(60), move || run(i, target, area));
}

fn kick(st: &Shared) {
    let weak = Rc::downgrade(st);
    let paint = st.borrow().paint.clone();
    paint.add_tick_callback(move |w, clock| {
        let Some(st) = weak.upgrade() else { return glib::ControlFlow::Break };
        let Ok(s) = st.try_borrow() else { return glib::ControlFlow::Continue };
        w.queue_draw();
        let now = clock.frame_time();
        if s.fade.settled(now) && s.glow.iter().all(|g| g.settled(now)) { glib::ControlFlow::Break } else { glib::ControlFlow::Continue }
    });
}

fn act(a: Value) {
    compositor::act(a);
}

fn neighbour(id: u64, right: bool) -> Option<u64> {
    act(json!(if right { "FocusColumnRight" } else { "FocusColumnLeft" }));
    let now = compositor::focused_id()?;
    if now == id {
        return None;
    }
    Some(now)
}

fn columns(id: u64, widths: &[f64]) {
    act(json!({ "SetColumnWidth": { "change": { "SetProportion": widths[0] } } }));
    let mut placed = 1;
    let mut right = true;
    let mut from = id;
    while placed < widths.len() {
        match neighbour(from, right) {
            Some(next) => {
                act(json!({ "SetColumnWidth": { "change": { "SetProportion": widths[placed] } } }));
                from = next;
                placed += 1;
            }
            None if right => {
                right = false;
                act(json!({ "FocusWindow": { "id": id } }));
                from = id;
            }
            None => break,
        }
    }
    act(json!({ "FocusWindow": { "id": id } }));
    act(json!("CenterVisibleColumns"));
}

fn run(i: usize, target: Option<u64>, area: (f64, f64)) {
    let Some(layout) = LAYOUTS.get(i) else { return };
    let kind = layout.kind;
    glib::spawn_future_local(async move {
        let _ = gio::spawn_blocking(move || {
            let Some(id) = target.or_else(compositor::focused_id) else { return };
            let floating = compositor::windows().iter().find(|w| w.id == id).is_some_and(|w| w.floating);
            act(json!({ "FocusWindow": { "id": id } }));
            match kind {
                Kind::Float((x, y, w, h)) => {
                    if !floating {
                        act(json!({ "MoveWindowToFloating": { "id": id } }));
                    }
                    let gap = niri_gap();
                    let (aw, ah) = area;
                    let pw = (aw * w - gap * 1.5).round();
                    let ph = (ah * h - gap * 1.5).round();
                    act(json!({ "SetWindowWidth": { "id": id, "change": { "SetFixed": pw as i32 } } }));
                    act(json!({ "SetWindowHeight": { "id": id, "change": { "SetFixed": ph as i32 } } }));
                    let px = (aw * x + if x > 0.0 { gap / 2.0 } else { gap }).round();
                    let py = (ah * y + if y > 0.0 { gap / 2.0 } else { gap }).round();
                    act(json!({ "MoveFloatingWindow": { "id": id, "x": { "SetFixed": px }, "y": { "SetFixed": py } } }));
                }
                _ => {
                    if floating {
                        act(json!({ "MoveWindowToTiling": { "id": id } }));
                    }
                    act(json!({ "ResetWindowHeight": { "id": id } }));
                    match kind {
                        Kind::Columns(widths) => columns(id, widths),
                        Kind::Centered => {
                            act(json!({ "SetColumnWidth": { "change": { "SetProportion": 200.0 / 3.0 } } }));
                            act(json!("CenterColumn"));
                        }
                        Kind::Stack => {
                            act(json!({ "SetColumnWidth": { "change": { "SetProportion": 50.0 } } }));
                            if neighbour(id, true).is_some() {
                                act(json!({ "SetColumnWidth": { "change": { "SetProportion": 50.0 } } }));
                                act(json!("ConsumeWindowIntoColumn"));
                            }
                            act(json!({ "FocusWindow": { "id": id } }));
                            act(json!("CenterVisibleColumns"));
                        }
                        Kind::Float(_) => {}
                    }
                }
            }
        })
        .await;
    });
}

impl State {
    fn draw(&self, s: &gtk::Snapshot) {
        let now = self.paint.frame_clock().map(|c| c.frame_time()).unwrap_or_else(glib::monotonic_time);
        let fade = self.fade.value(now).clamp(0.0, 1.0);
        let pal = self.app.palette_at(now);
        let cfg = &self.app.cfg;
        let rise = (6.0 * (1.0 - fade)).round();
        s.save();
        s.translate(&gtk::graphene::Point::new(0.0, -rise as f32));
        s.push_opacity(fade);
        draw::fill(s, &draw::round(0.0, 0.0, W, H, 26.0), pal.body.alpha(1.0));
        draw::text(s, &self.paint, cfg, "Window layout", 15.0, Face::Text(600), pal.ink_main, PAD + 2.0, PAD + 12.0, 240.0, pango::Alignment::Left);
        draw::text(s, &self.paint, cfg, &kit::keybinds::shown("layouts").unwrap_or_default(), 12.0, Face::Text(450), pal.ink_faint, W - PAD - 120.0, PAD + 12.0, 120.0, pango::Alignment::Right);
        for (i, l) in LAYOUTS.iter().enumerate() {
            let (x, y) = tile_rect(i);
            let glow = self.glow[i].value(now).clamp(0.0, 1.0);
            draw::fill(s, &draw::round(x - 4.0, y - 4.0, TILE_W + 8.0, TILE_H + 8.0, 16.0), pal.ink_main.alpha(0.05 + 0.09 * glow));
            let (sx, sy, sw, sh) = (x + 6.0, y + 6.0, TILE_W - 12.0, TILE_H - 12.0);
            for (k, (rx, ry, rw, rh)) in l.regions.iter().enumerate() {
                let ink = if k == 0 { pal.accent.mix(pal.ink_meta, 0.35 + 0.65 * glow) } else { pal.ink_main.alpha(0.2 + 0.08 * glow) };
                draw::fill(s, &draw::round(sx + rx * sw + 1.5, sy + ry * sh + 1.5, rw * sw - 3.0, rh * sh - 3.0, 5.0), ink);
            }
        }
        let l = &LAYOUTS[self.selected];
        let key = if self.selected == 9 { "0".to_string() } else { (self.selected + 1).to_string() };
        let fy = H - FOOT / 2.0 - 4.0;
        draw::text(s, &self.paint, cfg, l.label, 14.0, Face::Text(560), pal.ink_main, PAD + 2.0, fy, 260.0, pango::Alignment::Left);
        let hint = if self.target.is_some() { format!("{key} · Enter to apply") } else { "No focused window".to_string() };
        draw::text(s, &self.paint, cfg, &hint, 12.0, Face::Text(450), pal.ink_faint, W - PAD - 220.0, fy, 220.0, pango::Alignment::Right);
        s.pop();
        s.restore();
    }
}

#[cfg(test)]
mod mode_tests {
    use super::*;

    #[test]
    fn hyprland_cycles_native_layouts_and_respects_disabled_modes() {
        let prefs = toml::Table::new();
        assert_eq!(next_tiling_with_prefs("scrolling", &prefs).0, "dwindle");
        assert_eq!(next_tiling_with_prefs("floating", &prefs).0, "master");
        let prefs = "hypr_tiled_layout = 'master'".parse().unwrap();
        assert_eq!(next_tiling_with_prefs("floating", &prefs).0, "scrolling");
        let prefs = "hypr_tiling_cycle_off = ['master']".parse().unwrap();
        assert_eq!(next_tiling_with_prefs("scrolling", &prefs).0, "dwindle");
        assert_eq!(next_tiling_with_prefs("dwindle", &prefs).0, "scrolling");
        // The niri layout picker retains exactly its existing ten entries.
        assert_eq!(LAYOUTS.len(), 10);
        assert!(!names().contains(&"floating"));
    }
}
