use crate::capture::Shot;
use crate::windows::Rect;
use crate::{draw, out};
use gtk::{cairo, gdk, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub struct Screen {
    pub name: String,
    pub w: f64,
    pub h: f64,
    pub base: draw::Base,
    wins: Vec<Rect>,
    window: gtk::Window,
    area: gtk::DrawingArea,
    bar: gtk::Box,
    hint: gtk::Label,
}

#[derive(Clone, Copy)]
enum Drag {
    Create(f64, f64),
    Move(Rect),
    Resize(u8, Rect),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Act {
    Copy,
    Save,
    Edit,
    Pin,
    Text,
}

pub struct Overlay {
    app: gtk::Application,
    screens: RefCell<Vec<Screen>>,
    sel: Cell<Option<(usize, Rect)>>,
    drag: Cell<Option<(usize, Drag)>>,
    hover: Cell<Option<(usize, f64, f64)>>,
    prefs: out::Prefs,
    closed: Cell<bool>,
    pick: bool,
}

thread_local! {
    static ACTIVE: RefCell<Option<Rc<Overlay>>> = const { RefCell::new(None) };
}

pub fn busy() -> bool {
    ACTIVE.with(|a| a.borrow().is_some())
}

const HINT: &str = "Drag to select  ·  Click a window, or Space for the whole screen  ·  Esc cancels";

fn norm(a: (f64, f64), b: (f64, f64)) -> Rect {
    (a.0.min(b.0), a.1.min(b.1), (a.0 - b.0).abs(), (a.1 - b.1).abs())
}

fn handles(r: Rect) -> [(f64, f64); 8] {
    let (x, y, w, h) = r;
    [(x, y), (x + w / 2.0, y), (x + w, y), (x + w, y + h / 2.0), (x + w, y + h), (x + w / 2.0, y + h), (x, y + h), (x, y + h / 2.0)]
}

const PICK: &str = "Click to copy a color  ·  Enter picks under the pointer  ·  Esc cancels";

pub fn open(app: &gtk::Application, shots: Vec<Shot>, pick: bool) {
    let rects = if pick { vec![Vec::new(); shots.len()] } else { crate::windows::rects(&shots) };
    let prefs = out::prefs();
    let ov = Rc::new(Overlay { app: app.clone(), screens: RefCell::new(Vec::new()), sel: Cell::new(None), drag: Cell::new(None), hover: Cell::new(None), prefs, closed: Cell::new(false), pick });
    let mut screens = Vec::new();
    for (i, (shot, wins)) in shots.into_iter().zip(rects).enumerate() {
        let Some(base) = draw::Base::new(shot.data, shot.pw, shot.ph, shot.stride) else { continue };
        let window = gtk::Window::new();
        window.set_application(Some(app));
        window.add_css_class("snap");
        window.init_layer_shell();
        window.set_namespace(Some("digit-snap"));
        window.set_layer(Layer::Overlay);
        for e in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            window.set_anchor(e, true);
        }
        window.set_exclusive_zone(-1);
        window.set_keyboard_mode(KeyboardMode::Exclusive);
        if let Some(d) = gdk::Display::default() {
            let m = d.monitors().iter::<gdk::Monitor>().filter_map(Result::ok).find(|m| m.connector().is_some_and(|c| c == shot.name));
            window.set_monitor(m.as_ref());
        }
        let area = gtk::DrawingArea::new();
        area.set_hexpand(true);
        area.set_vexpand(true);
        area.set_cursor_from_name(Some("crosshair"));
        let layer = gtk::Overlay::new();
        layer.set_child(Some(&area));
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        bar.add_css_class("snapbar");
        bar.set_halign(gtk::Align::Start);
        bar.set_valign(gtk::Align::Start);
        bar.set_visible(false);
        for (icon, tip, act) in [("edit", "Annotate (E)", Act::Edit), ("content_copy", "Copy (Ctrl+C)", Act::Copy), ("download", "Save (Ctrl+S)", Act::Save), ("push_pin", "Pin to screen (P)", Act::Pin), ("text_fields", "Copy text (T)", Act::Text)] {
            if act == Act::Text && !out::ocr_available() {
                continue;
            }
            let b = kit::app::control(icon, "");
            b.set_tooltip_text(Some(tip));
            let weak = Rc::downgrade(&ov);
            b.connect_clicked(move |_| {
                if let Some(o) = weak.upgrade() {
                    o.act(act);
                }
            });
            bar.append(&b);
        }
        let x = kit::app::control("close", "close");
        x.set_tooltip_text(Some("Cancel (Esc)"));
        let weak = Rc::downgrade(&ov);
        x.connect_clicked(move |_| {
            if let Some(o) = weak.upgrade() {
                o.close();
            }
        });
        bar.append(&x);
        layer.add_overlay(&bar);
        let hint = kit::ui::text(if pick { PICK } else { HINT }, "snaphint");
        hint.set_halign(gtk::Align::Center);
        hint.set_valign(gtk::Align::End);
        hint.set_margin_bottom(40);
        hint.set_can_target(false);
        layer.add_overlay(&hint);
        window.set_child(Some(&layer));
        let weak = Rc::downgrade(&ov);
        area.set_draw_func(move |_, cr, _, _| {
            if let Some(o) = weak.upgrade() {
                o.paint(i, cr);
            }
        });
        wire(&ov, i, &window, &area);
        screens.push(Screen { name: shot.name, w: shot.w as f64, h: shot.h as f64, base, wins, window, area, bar, hint });
    }
    *ov.screens.borrow_mut() = screens;
    let windows: Vec<gtk::Window> = ov.screens.borrow().iter().map(|s| s.window.clone()).collect();
    ACTIVE.with(|a| *a.borrow_mut() = Some(ov.clone()));
    for w in windows {
        w.present();
    }
}

fn wire(ov: &Rc<Overlay>, i: usize, window: &gtk::Window, area: &gtk::DrawingArea) {
    let motion = gtk::EventControllerMotion::new();
    let weak = Rc::downgrade(ov);
    motion.connect_motion(move |_, x, y| {
        if let Some(o) = weak.upgrade() {
            let prev = o.hover.replace(Some((i, x, y)));
            if let Some((p, _, _)) = prev {
                if p != i {
                    o.redraw(p);
                }
            }
            o.redraw(i);
        }
    });
    let weak = Rc::downgrade(ov);
    motion.connect_leave(move |_| {
        if let Some(o) = weak.upgrade() {
            if o.hover.get().is_some_and(|h| h.0 == i) {
                o.hover.set(None);
                o.redraw(i);
            }
        }
    });
    area.add_controller(motion);
    let drag = gtk::GestureDrag::new();
    if ov.pick {
        drag.set_propagation_phase(gtk::PropagationPhase::None);
    }
    let weak = Rc::downgrade(ov);
    drag.connect_drag_begin(move |_, x, y| {
        if let Some(o) = weak.upgrade() {
            o.begin(i, x, y);
        }
    });
    let weak = Rc::downgrade(ov);
    drag.connect_drag_update(move |g, dx, dy| {
        if let (Some(o), Some((sx, sy))) = (weak.upgrade(), g.start_point()) {
            o.update(i, sx, sy, dx, dy);
        }
    });
    let weak = Rc::downgrade(ov);
    drag.connect_drag_end(move |g, dx, dy| {
        if let (Some(o), Some((sx, sy))) = (weak.upgrade(), g.start_point()) {
            o.end(i, sx + dx, sy + dy);
        }
    });
    area.add_controller(drag);
    let click = gtk::GestureClick::new();
    click.set_button(0);
    let weak = Rc::downgrade(ov);
    click.connect_pressed(move |g, n, x, y| {
        let Some(o) = weak.upgrade() else { return };
        if o.pick {
            if g.current_button() == 1 {
                o.picked(i, x, y);
            } else {
                o.close();
            }
            return;
        }
        if g.current_button() == 3 {
            if o.sel.get().is_some() {
                o.clear();
            } else {
                o.close();
            }
        } else if n == 2 && o.sel.get().is_some_and(|(s, r)| s == i && x >= r.0 && x <= r.0 + r.2 && y >= r.1 && y <= r.1 + r.3) {
            o.act(o.default_act());
        }
    });
    area.add_controller(click);
    let keys = gtk::EventControllerKey::new();
    let weak = Rc::downgrade(ov);
    keys.connect_key_pressed(move |_, k, _, m| {
        let Some(o) = weak.upgrade() else { return glib::Propagation::Proceed };
        if o.key(k, m) { glib::Propagation::Stop } else { glib::Propagation::Proceed }
    });
    window.add_controller(keys);
}

impl Overlay {
    fn redraw(&self, i: usize) {
        if let Ok(s) = self.screens.try_borrow() {
            if let Some(sc) = s.get(i) {
                sc.area.queue_draw();
            }
        }
    }

    fn redraw_all(&self) {
        if let Ok(s) = self.screens.try_borrow() {
            for sc in s.iter() {
                sc.area.queue_draw();
            }
        }
    }

    fn size(&self, i: usize) -> (f64, f64) {
        self.screens.borrow().get(i).map(|s| (s.w, s.h)).unwrap_or((1.0, 1.0))
    }

    fn window_at(&self, i: usize, x: f64, y: f64) -> Option<Rect> {
        let s = self.screens.borrow();
        let wins = &s.get(i)?.wins;
        wins.iter().rev().find(|r| x >= r.0 && x <= r.0 + r.2 && y >= r.1 && y <= r.1 + r.3).copied()
    }

    fn begin(&self, i: usize, x: f64, y: f64) {
        self.hide_bars();
        if let Some((s, r)) = self.sel.get() {
            if s == i {
                if let Some(h) = handles(r).iter().position(|p| (p.0 - x).abs() < 10.0 && (p.1 - y).abs() < 10.0) {
                    self.drag.set(Some((i, Drag::Resize(h as u8, r))));
                    return;
                }
                if x >= r.0 && x <= r.0 + r.2 && y >= r.1 && y <= r.1 + r.3 {
                    self.drag.set(Some((i, Drag::Move(r))));
                    return;
                }
            }
        }
        self.drag.set(Some((i, Drag::Create(x, y))));
    }

    fn update(&self, i: usize, sx: f64, sy: f64, dx: f64, dy: f64) {
        let Some((di, d)) = self.drag.get() else { return };
        if di != i {
            return;
        }
        let (w, h) = self.size(i);
        let cur = ((sx + dx).clamp(0.0, w), (sy + dy).clamp(0.0, h));
        let r = match d {
            Drag::Create(x, y) => {
                if dx.abs() < 3.0 && dy.abs() < 3.0 {
                    return;
                }
                norm((x, y), cur)
            }
            Drag::Move(r) => ((r.0 + dx).clamp(0.0, w - r.2), (r.1 + dy).clamp(0.0, h - r.3), r.2, r.3),
            Drag::Resize(hd, r) => {
                let (mut x0, mut y0, mut x1, mut y1) = (r.0, r.1, r.0 + r.2, r.1 + r.3);
                match hd {
                    0 => (x0, y0) = cur,
                    1 => y0 = cur.1,
                    2 => (x1, y0) = cur,
                    3 => x1 = cur.0,
                    4 => (x1, y1) = cur,
                    5 => y1 = cur.1,
                    6 => (x0, y1) = cur,
                    _ => x0 = cur.0,
                }
                norm((x0, y0), (x1, y1))
            }
        };
        self.sel.set(Some((i, r)));
        self.redraw_all();
    }

    fn end(&self, i: usize, x: f64, y: f64) {
        let d = self.drag.take();
        if let Some((_, Drag::Create(sx, sy))) = d {
            if (x - sx).abs() < 4.0 && (y - sy).abs() < 4.0 {
                let (w, h) = self.size(i);
                let r = self.window_at(i, x, y).unwrap_or((0.0, 0.0, w, h));
                self.sel.set(Some((i, r)));
            }
        }
        self.redraw_all();
        self.show_bar();
    }

    fn clear(&self) {
        self.sel.set(None);
        self.hide_bars();
        self.redraw_all();
    }

    fn hide_bars(&self) {
        if let Ok(s) = self.screens.try_borrow() {
            for sc in s.iter() {
                sc.bar.set_visible(false);
                sc.hint.set_visible(self.sel.get().is_none());
            }
        }
    }

    fn show_bar(&self) {
        let Some((i, r)) = self.sel.get() else { return };
        let s = self.screens.borrow();
        for (k, sc) in s.iter().enumerate() {
            sc.hint.set_visible(false);
            sc.bar.set_visible(k == i);
        }
        let Some(sc) = s.get(i) else { return };
        let (bw, bh) = (sc.bar.measure(gtk::Orientation::Horizontal, -1).1, sc.bar.measure(gtk::Orientation::Vertical, -1).1);
        let x = (r.0 + r.2 / 2.0 - bw as f64 / 2.0).clamp(8.0, sc.w - bw as f64 - 8.0);
        let below = r.1 + r.3 + 12.0;
        let y = if below + bh as f64 + 8.0 < sc.h { below } else if r.1 - bh as f64 - 12.0 > 8.0 { r.1 - bh as f64 - 12.0 } else { r.1 + r.3 - bh as f64 - 12.0 };
        sc.bar.set_margin_start(x.round() as i32);
        sc.bar.set_margin_top(y.round() as i32);
    }

    fn default_act(&self) -> Act {
        match self.prefs.after.as_str() {
            "save" => Act::Save,
            "edit" => Act::Edit,
            "pin" => Act::Pin,
            _ => Act::Copy,
        }
    }

    fn key(&self, k: gdk::Key, m: gdk::ModifierType) -> bool {
        let ctrl = m.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = m.contains(gdk::ModifierType::SHIFT_MASK);
        if self.pick {
            match k {
                gdk::Key::Escape => self.close(),
                gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::space => {
                    if let Some((i, x, y)) = self.hover.get() {
                        self.picked(i, x, y);
                    }
                }
                _ => return false,
            }
            return true;
        }
        match k {
            gdk::Key::Escape => {
                if self.sel.get().is_some() && self.drag.get().is_none() {
                    self.clear();
                } else {
                    self.close();
                }
            }
            gdk::Key::Return | gdk::Key::KP_Enter => {
                if self.sel.get().is_none() {
                    self.whole();
                }
                self.act(self.default_act());
            }
            gdk::Key::c if ctrl => self.act_or_whole(Act::Copy),
            gdk::Key::s if ctrl => self.act_or_whole(Act::Save),
            gdk::Key::e => self.act_or_whole(Act::Edit),
            gdk::Key::p => self.act_or_whole(Act::Pin),
            gdk::Key::t if out::ocr_available() => self.act_or_whole(Act::Text),
            gdk::Key::space => {
                self.whole();
                self.show_bar();
            }
            gdk::Key::Left | gdk::Key::Right | gdk::Key::Up | gdk::Key::Down => {
                let Some((i, r)) = self.sel.get() else { return true };
                let step = if shift { 10.0 } else { 1.0 };
                let (dx, dy) = match k {
                    gdk::Key::Left => (-step, 0.0),
                    gdk::Key::Right => (step, 0.0),
                    gdk::Key::Up => (0.0, -step),
                    _ => (0.0, step),
                };
                let (w, h) = self.size(i);
                let r = if ctrl { (r.0, r.1, (r.2 + dx).clamp(1.0, w - r.0), (r.3 + dy).clamp(1.0, h - r.1)) } else { ((r.0 + dx).clamp(0.0, w - r.2), (r.1 + dy).clamp(0.0, h - r.3), r.2, r.3) };
                self.sel.set(Some((i, r)));
                self.redraw_all();
                self.show_bar();
            }
            _ => return false,
        }
        true
    }

    fn act_or_whole(&self, a: Act) {
        if self.sel.get().is_none() {
            self.whole();
        }
        self.act(a);
    }

    fn whole(&self) {
        let i = self.hover.get().map(|h| h.0).unwrap_or(0);
        let (w, h) = self.size(i);
        self.sel.set(Some((i, (0.0, 0.0, w, h))));
        self.redraw_all();
    }

    fn picked(&self, i: usize, x: f64, y: f64) {
        let rgb = {
            let s = self.screens.borrow();
            let Some(sc) = s.get(i) else { return };
            let k = sc.base.w as f64 / sc.w;
            sc.base.pixel((x * k) as i32, (y * k) as i32)
        };
        self.close();
        let hex = format!("#{:02x}{:02x}{:02x}", rgb.0, rgb.1, rgb.2);
        out::copy_text(&hex);
        let c = kit::color::Color { r: rgb.0 as f64 / 255.0, g: rgb.1 as f64 / 255.0, b: rgb.2 as f64 / 255.0, a: 1.0 };
        let body: Vec<String> = c.formats().into_iter().skip(1).take(3).map(|(_, v)| v).collect();
        let swatch = out::swatch(rgb);
        out::notify(&format!("{hex} copied"), &format!("{}\nAlt+V shows every format", body.join("  ·  ")), swatch.as_deref());
    }

    pub fn close(&self) {
        if self.closed.replace(true) {
            return;
        }
        let windows: Vec<gtk::Window> = self.screens.borrow().iter().map(|s| s.window.clone()).collect();
        ACTIVE.with(|a| a.borrow_mut().take());
        for w in windows {
            w.close();
        }
    }

    fn act(&self, a: Act) {
        let Some((i, r)) = self.sel.get() else { return };
        if r.2 < 2.0 || r.3 < 2.0 {
            return;
        }
        let (surface, name, origin) = {
            let s = self.screens.borrow();
            let Some(sc) = s.get(i) else { return };
            let k = sc.base.w as f64 / sc.w;
            let (px, py) = ((r.0 * k).round(), (r.1 * k).round());
            let (pw, ph) = (((r.0 + r.2) * k).round() - px, ((r.1 + r.3) * k).round() - py);
            let Ok(surface) = cairo::ImageSurface::create(cairo::Format::ARgb32, pw.max(1.0) as i32, ph.max(1.0) as i32) else { return };
            if let Ok(cr) = cairo::Context::new(&surface) {
                let _ = cr.set_source_surface(&sc.base.surface, -px, -py);
                let _ = cr.paint();
            }
            (surface, sc.name.clone(), (r.0, r.1, r.2, r.3))
        };
        let app = self.app.clone();
        let hold = app.hold();
        self.close();
        let prefs = out::prefs();
        glib::idle_add_local_once(move || {
            let _hold = hold;
            match a {
                Act::Edit => crate::editor::open(&app, surface, name, origin),
                _ => {
                    let Some(tex) = out::texture(&surface) else { return };
                    match a {
                        Act::Copy => {
                            out::copy(&tex, &app);
                            let saved = prefs.save_on_copy.then(|| out::save(&tex, &prefs).ok()).flatten();
                            if prefs.notify {
                                out::notify("Screenshot copied", &saved.as_ref().and_then(|p| p.file_name()).map(|n| format!("Also saved as {}", n.to_string_lossy())).unwrap_or_default(), saved.as_deref());
                            }
                        }
                        Act::Save => match out::save(&tex, &prefs) {
                            Ok(p) => {
                                if prefs.notify {
                                    out::notify("Screenshot saved", &p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), Some(&p));
                                }
                            }
                            Err(e) => out::notify("Couldn't save the screenshot", &e, None),
                        },
                        Act::Pin => out::pin(&app, tex, &name, origin.0 as i32, origin.1 as i32, surface.width(), surface.height()),
                        Act::Text => out::ocr(&tex),
                        Act::Edit => {}
                    }
                }
            }
        });
    }

    fn paint(&self, i: usize, cr: &cairo::Context) {
        let Ok(s) = self.screens.try_borrow() else { return };
        let Some(sc) = s.get(i) else { return };
        let (w, h) = (sc.w, sc.h);
        let k = sc.base.w as f64 / w;
        cr.save().ok();
        cr.scale(1.0 / k, 1.0 / k);
        let _ = cr.set_source_surface(&sc.base.surface, 0.0, 0.0);
        cr.source().set_filter(cairo::Filter::Good);
        let _ = cr.paint();
        cr.restore().ok();
        let sel = self.sel.get().filter(|(si, _)| *si == i).map(|(_, r)| r);
        cr.set_fill_rule(cairo::FillRule::EvenOdd);
        cr.rectangle(0.0, 0.0, w, h);
        if let Some(r) = sel {
            cr.rectangle(r.0, r.1, r.2, r.3);
        }
        cr.set_source_rgba(0.0, 0.0, 0.0, if self.pick { 0.0 } else { 0.45 });
        let _ = cr.fill();
        cr.set_fill_rule(cairo::FillRule::Winding);
        let hover = self.hover.get().filter(|(hi, _, _)| *hi == i);
        let dragging = self.drag.get().is_some_and(|(di, d)| di == i && matches!(d, Drag::Create(..)));
        if sel.is_none() && !dragging {
            if let Some((_, x, y)) = hover {
                if let Some(r) = sc.wins.iter().rev().find(|r| x >= r.0 && x <= r.0 + r.2 && y >= r.1 && y <= r.1 + r.3) {
                    draw::rounded(cr, r.0, r.1, r.2, r.3, 12.0);
                    cr.set_source_rgba(1.0, 1.0, 1.0, 0.10);
                    let _ = cr.fill_preserve();
                    cr.set_source_rgba(1.0, 1.0, 1.0, 0.9);
                    cr.set_line_width(2.0);
                    let _ = cr.stroke();
                }
            }
        }
        if let Some(r) = sel {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.95);
            cr.set_line_width(1.5);
            cr.rectangle(r.0.round() + 0.25, r.1.round() + 0.25, r.2.round() - 0.5, r.3.round() - 0.5);
            let _ = cr.stroke();
            if !dragging {
                for (hx, hy) in handles(r) {
                    cr.arc(hx, hy, 5.0, 0.0, std::f64::consts::TAU);
                    cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
                    let _ = cr.fill_preserve();
                    cr.set_source_rgba(0.0, 0.0, 0.0, 0.5);
                    cr.set_line_width(1.0);
                    let _ = cr.stroke();
                }
            }
            let label = format!("{} × {}", (r.2 * k).round(), (r.3 * k).round());
            pill(cr, &label, r.0 + 2.0, if r.1 > 34.0 { r.1 - 30.0 } else { r.1 + 6.0 }, w);
        }
        if (self.prefs.loupe || self.pick) && (sel.is_none() || dragging) {
            if let Some((_, x, y)) = hover {
                loupe(cr, &sc.base, x, y, k, w, h, self.pick);
            }
        }
    }
}

fn pill(cr: &cairo::Context, text: &str, x: f64, y: f64, maxw: f64) {
    let l = draw::layout(cr, text, "Rubik Medium 12px");
    let (tw, th) = l.pixel_size();
    let (bw, bh) = (tw as f64 + 16.0, th as f64 + 8.0);
    let x = x.min(maxw - bw - 4.0).max(4.0);
    draw::rounded(cr, x, y, bw, bh, bh / 2.0);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.78);
    let _ = cr.fill();
    cr.set_source_rgb(1.0, 1.0, 1.0);
    cr.move_to(x + 8.0, y + 4.0);
    draw::show(cr, &l);
}

#[allow(clippy::too_many_arguments)]
fn loupe(cr: &cairo::Context, base: &draw::Base, x: f64, y: f64, k: f64, w: f64, h: f64, color: bool) {
    let r = 62.0;
    let zoom = 9.0;
    let mut lx = x + 26.0;
    let mut ly = y + 26.0;
    if lx + r * 2.0 > w - 4.0 {
        lx = x - 26.0 - r * 2.0;
    }
    if ly + r * 2.0 + 30.0 > h - 4.0 {
        ly = y - 26.0 - r * 2.0 - 30.0;
    }
    let (px, py) = ((x * k).floor(), (y * k).floor());
    cr.save().ok();
    cr.arc(lx + r, ly + r, r, 0.0, std::f64::consts::TAU);
    cr.clip();
    cr.set_source_rgb(0.0, 0.0, 0.0);
    let _ = cr.paint();
    cr.translate(lx + r, ly + r);
    cr.scale(zoom, zoom);
    cr.translate(-(px + 0.5), -(py + 0.5));
    let _ = cr.set_source_surface(&base.surface, 0.0, 0.0);
    cr.source().set_filter(cairo::Filter::Nearest);
    let _ = cr.paint();
    cr.restore().ok();
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.9);
    cr.set_line_width(1.0);
    cr.rectangle(lx + r - zoom / 2.0, ly + r - zoom / 2.0, zoom, zoom);
    let _ = cr.stroke();
    cr.arc(lx + r, ly + r, r, 0.0, std::f64::consts::TAU);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.85);
    cr.set_line_width(2.0);
    let _ = cr.stroke();
    let (rr, gg, bb) = base.pixel(px as i32, py as i32);
    let text = if color { format!("#{rr:02x}{gg:02x}{bb:02x}   {} , {}", px as i32, py as i32) } else { format!("{} , {}", px as i32, py as i32) };
    let l = draw::layout(cr, &text, "Rubik Medium 11px");
    let (tw, th) = l.pixel_size();
    let (bw, bh) = (tw as f64 + 30.0, th as f64 + 8.0);
    let bx = lx + r - bw / 2.0;
    let by = ly + r * 2.0 + 6.0;
    draw::rounded(cr, bx, by, bw, bh, bh / 2.0);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.8);
    let _ = cr.fill();
    if color {
        cr.arc(bx + 12.0, by + bh / 2.0, 5.0, 0.0, std::f64::consts::TAU);
        cr.set_source_rgb(rr as f64 / 255.0, gg as f64 / 255.0, bb as f64 / 255.0);
        let _ = cr.fill_preserve();
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.7);
        cr.set_line_width(1.0);
        let _ = cr.stroke();
    }
    cr.set_source_rgb(1.0, 1.0, 1.0);
    cr.move_to(if color { bx + 22.0 } else { bx + 15.0 }, by + 4.0);
    draw::show(cr, &l);
}
