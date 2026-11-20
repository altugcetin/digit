use crate::app::App;
use crate::draw::{self, Face};
use crate::paint::Paint;
use gtk::{gdk, gio, glib, graphene, gsk, pango, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kit::motion::{self, Tween};
use kit::palette::Rgb;
use std::cell::RefCell;
use std::rc::Rc;

const CELL: f64 = 118.0;
const DISC: f64 = 76.0;
const COUNT: i64 = 3_000_000;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Act {
    Lock,
    Sleep,
    Restart,
    PowerOff,
    LogOut,
}

const ACTS: [(Act, &str, &str); 5] = [
    (Act::Lock, "lock", "Lock"),
    (Act::Sleep, "bedtime", "Sleep"),
    (Act::Restart, "restart_alt", "Restart"),
    (Act::PowerOff, "power_settings_new", "Shut Down"),
    (Act::LogOut, "logout", "Log Out"),
];

impl Act {
    fn careful(self) -> bool {
        matches!(self, Act::Restart | Act::PowerOff | Act::LogOut)
    }
}

async fn logind(method: &'static str) {
    let Ok(conn) = crate::bus::system().await else { return };
    let Ok(m) = zbus::Proxy::new_owned(conn, "org.freedesktop.login1", "/org/freedesktop/login1", "org.freedesktop.login1.Manager").await else { return };
    let _ = m.call_method(method, &(true,)).await;
}

pub fn run(app: &Rc<App>, act: Act) {
    match act {
        Act::Lock => {
            crate::lock::lock(app);
        }
        Act::Sleep => {
            glib::spawn_future_local(logind("Suspend"));
        }
        Act::Restart => {
            glib::spawn_future_local(logind("Reboot"));
        }
        Act::PowerOff => {
            glib::spawn_future_local(logind("PowerOff"));
        }
        Act::LogOut => {
            glib::spawn_future_local(async {
                let _ = gio::spawn_blocking(kit::wm::quit).await;
            });
        }
    }
}

struct Menu {
    app: Rc<App>,
    window: gtk::Window,
    paint: Paint,
    hover: Option<usize>,
    pick: Option<usize>,
    pending: Option<(usize, i64)>,
    fade: Tween,
    closing: bool,
    who: String,
}

thread_local! {
    static OPEN: RefCell<Option<Rc<RefCell<Menu>>>> = const { RefCell::new(None) };
}

fn uptime() -> String {
    let secs = std::fs::read_to_string("/proc/uptime").ok().and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok()).unwrap_or(0.0) as u64;
    let (d, h, m) = (secs / 86400, secs / 3600 % 24, secs / 60 % 60);
    if d > 0 {
        format!("up {d} d {h} h")
    } else if h > 0 {
        format!("up {h} h {m} min")
    } else {
        format!("up {m} min")
    }
}

/// The user's full name from the account, or the login name when the account has none. GLib
/// reads it once with the thread-safe calls (unlike `getpwuid`), and names a missing one
/// "Unknown".
pub fn real_name() -> String {
    let real = glib::real_name().to_string_lossy().trim().to_string();
    if real.is_empty() || real == "Unknown" { glib::user_name().to_string_lossy().into_owned() } else { real }
}

impl Menu {
    fn origin(&self) -> (f64, f64) {
        let w = self.paint.width() as f64;
        let h = self.paint.height() as f64;
        ((w - CELL * ACTS.len() as f64) / 2.0, h / 2.0 - 24.0)
    }

    fn at(&self, x: f64, y: f64) -> Option<usize> {
        let (x0, cy) = self.origin();
        let i = ((x - x0) / CELL).floor();
        if i < 0.0 || i >= ACTS.len() as f64 {
            return None;
        }
        let cx = x0 + (i + 0.5) * CELL;
        ((x - cx).powi(2) + (y - cy).powi(2) <= (DISC / 2.0 + 10.0).powi(2) || (y > cy && y < cy + DISC / 2.0 + 34.0 && (x - cx).abs() < CELL / 2.0 - 6.0)).then_some(i as usize)
    }

    fn draw(&self, s: &gtk::Snapshot) {
        let now = self.paint.frame_clock().map(|c| c.frame_time()).unwrap_or_else(glib::monotonic_time);
        let fade = self.fade.value(now).clamp(0.0, 1.0);
        let pal = self.app.palette_at(now);
        let cfg = &self.app.cfg;
        let w = self.paint.width() as f64;
        let h = self.paint.height() as f64;
        s.append_color(&Rgb::new(0.0, 0.0, 0.0).alpha(0.55 * fade).rgba(), &graphene::Rect::new(0.0, 0.0, w as f32, h as f32));
        s.push_opacity(fade);
        let lift = (1.0 - fade) * 14.0;
        s.save();
        s.translate(&graphene::Point::new(0.0, lift as f32));
        let (x0, cy) = self.origin();
        let white = Rgb::new(1.0, 1.0, 1.0);
        draw::text(s, &self.paint, cfg, &self.who, 22.0, Face::Text(600), white, x0, cy - DISC / 2.0 - 70.0, CELL * ACTS.len() as f64, pango::Alignment::Center);
        draw::text(s, &self.paint, cfg, &uptime(), 14.0, Face::Text(450), white.alpha(0.6), x0, cy - DISC / 2.0 - 42.0, CELL * ACTS.len() as f64, pango::Alignment::Center);
        for (i, (act, glyph, label)) in ACTS.iter().enumerate() {
            let cx = x0 + (i as f64 + 0.5) * CELL;
            let on = self.hover == Some(i) || self.pick == Some(i);
            let pending = self.pending.filter(|(p, _)| *p == i);
            let danger = matches!(act, Act::PowerOff | Act::Restart);
            let disc = if pending.is_some() {
                if danger { pal.risk.alpha(0.9) } else { pal.accent }
            } else if on {
                white.alpha(0.26)
            } else {
                white.alpha(0.13)
            };
            draw::fill(s, &draw::round(cx - DISC / 2.0, cy - DISC / 2.0, DISC, DISC, DISC / 2.0), disc);
            if self.pick == Some(i) && self.hover != Some(i) {
                let ring = draw::round(cx - DISC / 2.0 - 4.0, cy - DISC / 2.0 - 4.0, DISC + 8.0, DISC + 8.0, DISC / 2.0 + 4.0);
                let c = white.alpha(0.5).rgba();
                s.append_border(&ring, &[2.0; 4], &[c, c, c, c]);
            }
            let ink = if pending.is_some() { if danger { white } else { pal.on_accent } } else { white };
            draw::icon(s, &self.paint, cfg, glyph, 30.0, false, ink, cx, cy);
            let text = match pending {
                Some((_, at)) => {
                    let left = ((COUNT - (now - at)).max(0) as f64 / 1_000_000.0).ceil() as i64;
                    let progress = ((now - at) as f64 / COUNT as f64).clamp(0.0, 1.0);
                    let pb = gsk::PathBuilder::new();
                    let r = DISC / 2.0 + 6.0;
                    let steps = (72.0 * progress).ceil().max(1.0) as usize;
                    for k in 0..=steps {
                        let a = -std::f64::consts::FRAC_PI_2 + progress * std::f64::consts::TAU * k as f64 / steps as f64;
                        let (px, py) = ((cx + r * a.cos()) as f32, (cy + r * a.sin()) as f32);
                        if k == 0 { pb.move_to(px, py) } else { pb.line_to(px, py) }
                    }
                    let stroke = gsk::Stroke::new(3.0);
                    stroke.set_line_cap(gsk::LineCap::Round);
                    s.append_stroke(&pb.to_path(), &stroke, &white.alpha(0.9).rgba());
                    format!("{label} in {left}")
                }
                None => label.to_string(),
            };
            draw::text(s, &self.paint, cfg, &text, 14.5, Face::Text(if on { 600 } else { 500 }), white.alpha(if on { 1.0 } else { 0.85 }), cx - CELL / 2.0, cy + DISC / 2.0 + 22.0, CELL, pango::Alignment::Center);
        }
        let hint = if self.pending.is_some() { "Esc to cancel · Enter to go now" } else { "Esc to close" };
        draw::text(s, &self.paint, cfg, hint, 13.0, Face::Text(450), white.alpha(0.5), 0.0, cy + DISC / 2.0 + 86.0, w, pango::Alignment::Center);
        s.restore();
        s.pop();
    }
}

pub fn close() {
    let Some(m) = OPEN.with(|o| o.borrow().clone()) else { return };
    let mut mm = m.borrow_mut();
    if mm.closing {
        return;
    }
    mm.closing = true;
    mm.pending = None;
    let now = glib::monotonic_time();
    mm.fade.go(0.0, now, 160, motion::FAST_EFFECTS);
    mm.window.set_keyboard_mode(KeyboardMode::None);
    drop(mm);
    tick(&m);
}

fn tick(m: &Rc<RefCell<Menu>>) {
    let weak = Rc::downgrade(m);
    let paint = m.borrow().paint.clone();
    paint.add_tick_callback(move |p, clock| {
        let Some(m) = weak.upgrade() else { return glib::ControlFlow::Break };
        p.queue_draw();
        let now = clock.frame_time();
        let (busy, done, fire) = {
            let mm = m.borrow();
            let fire = mm.pending.filter(|(_, at)| now - at >= COUNT).map(|(i, _)| i);
            (!mm.fade.settled(now) || mm.pending.is_some(), mm.closing && mm.fade.settled(now), fire)
        };
        if let Some(i) = fire {
            let app = m.borrow().app.clone();
            m.borrow_mut().pending = None;
            close();
            run(&app, ACTS[i].0);
        }
        if done {
            let window = m.borrow().window.clone();
            OPEN.with(|o| o.borrow_mut().take());
            glib::idle_add_local_once(move || window.destroy());
            return glib::ControlFlow::Break;
        }
        if busy || fire.is_some() { glib::ControlFlow::Continue } else { glib::ControlFlow::Break }
    });
}

fn choose(m: &Rc<RefCell<Menu>>, i: usize) {
    let act = ACTS[i].0;
    let app = m.borrow().app.clone();
    let pending = m.borrow().pending;
    if !act.careful() || pending.is_some_and(|(p, _)| p == i) {
        m.borrow_mut().pending = None;
        close();
        let app = app.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(180), move || run(&app, act));
        return;
    }
    m.borrow_mut().pending = Some((i, glib::monotonic_time()));
    tick(m);
}

pub fn toggle(app: &Rc<App>) {
    if OPEN.with(|o| o.borrow().is_some()) {
        close();
        return;
    }
    let output = kit::wm::focused_output();
    let monitor = crate::screens::for_output(output.as_deref());
    let window = gtk::Window::new();
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace(Some("digit-session"));
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(KeyboardMode::Exclusive);
    window.set_monitor(monitor.as_ref());
    for e in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(e, true);
    }
    window.add_css_class("digit");
    let paint = Paint::new(-1, -1);
    paint.set_hexpand(true);
    paint.set_vexpand(true);
    window.set_child(Some(&paint));
    let mut fade = Tween::still(0.0);
    fade.go(1.0, glib::monotonic_time(), 220, motion::STANDARD);
    let menu = Rc::new(RefCell::new(Menu { app: app.clone(), window: window.clone(), paint: paint.clone(), hover: None, pick: None, pending: None, fade, closing: false, who: real_name() }));
    let weak = Rc::downgrade(&menu);
    paint.on_draw(move |s| {
        if let Some(m) = weak.upgrade() {
            if let Ok(m) = m.try_borrow() {
                m.draw(s);
            }
        }
    });
    let motion = gtk::EventControllerMotion::new();
    let weak = Rc::downgrade(&menu);
    motion.connect_motion(move |_, x, y| {
        let Some(m) = weak.upgrade() else { return };
        let Ok(mut mm) = m.try_borrow_mut() else { return };
        let at = mm.at(x, y);
        if at != mm.hover {
            mm.hover = at;
            mm.paint.set_cursor_from_name(at.map(|_| "pointer"));
            mm.paint.queue_draw();
        }
    });
    paint.add_controller(motion);
    let click = gtk::GestureClick::new();
    let weak = Rc::downgrade(&menu);
    click.connect_released(move |_, _, x, y| {
        let Some(m) = weak.upgrade() else { return };
        let at = m.borrow().at(x, y);
        match at {
            Some(i) => choose(&m, i),
            None => close(),
        }
    });
    paint.add_controller(click);
    let keys = gtk::EventControllerKey::new();
    let weak = Rc::downgrade(&menu);
    keys.connect_key_pressed(move |_, key, _, _| {
        let Some(m) = weak.upgrade() else { return glib::Propagation::Proceed };
        match key {
            gdk::Key::Escape => {
                if m.borrow().pending.is_some() {
                    m.borrow_mut().pending = None;
                    m.borrow().paint.queue_draw();
                } else {
                    close();
                }
            }
            gdk::Key::Left | gdk::Key::Right | gdk::Key::Tab | gdk::Key::ISO_Left_Tab => {
                let mut mm = m.borrow_mut();
                let n = ACTS.len();
                let back = matches!(key, gdk::Key::Left | gdk::Key::ISO_Left_Tab);
                mm.pick = Some(match mm.pick {
                    None => if back { n - 1 } else { 0 },
                    Some(p) => if back { (p + n - 1) % n } else { (p + 1) % n },
                });
                mm.paint.queue_draw();
            }
            gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::space => {
                let target = { let mm = m.borrow(); mm.pending.map(|(i, _)| i).or(mm.pick) };
                if let Some(i) = target {
                    choose(&m, i);
                }
            }
            gdk::Key::l | gdk::Key::L => choose(&m, 0),
            gdk::Key::s | gdk::Key::S => choose(&m, 1),
            gdk::Key::r | gdk::Key::R => choose(&m, 2),
            gdk::Key::p | gdk::Key::P => choose(&m, 3),
            gdk::Key::e | gdk::Key::E => choose(&m, 4),
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    window.add_controller(keys);
    OPEN.with(|o| *o.borrow_mut() = Some(menu.clone()));
    window.present();
    tick(&menu);
}
