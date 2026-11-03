use crate::app::App;
use crate::calendar;
use crate::draw::{self, Face};
use crate::paint::Paint;
use gtk::{cairo, gdk, glib, graphene, pango, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kit::motion::{self, Tween};
use kit::shape;
use std::cell::RefCell;
use std::rc::Rc;

const W: f64 = 360.0;
const PAD: f64 = 20.0;
const HEAD: f64 = 52.0;
const NAMES: f64 = 26.0;
const CELL: f64 = 44.0;
const CELL_W: f64 = (W - 2.0 * PAD) / 7.0;
const ROWS: usize = 6;
const LINE: f64 = 26.0;
const LINES: usize = 3;
const FILLET: f64 = 14.0;
const RADIUS: f64 = 28.0;

#[derive(Clone, Copy, PartialEq)]
enum Spot {
    Back,
    Ahead,
    Title,
    Day(usize),
}

struct Pop {
    app: Rc<App>,
    window: gtk::Window,
    paint: Paint,
    year: i32,
    month: i32,
    pick: i64,
    hover: Option<Spot>,
    slide: Tween,
    fade: Tween,
    open: bool,
    scrim: Option<gtk::Window>,
    agenda: bool,
}

type Hook = Box<dyn FnOnce()>;

thread_local! {
    static OPEN: RefCell<Option<Rc<RefCell<Pop>>>> = const { RefCell::new(None) };
    static DONE: RefCell<Option<Hook>> = const { RefCell::new(None) };
}

pub fn is_open() -> bool {
    OPEN.with(|o| o.borrow().is_some())
}

/// Slides the panel back out to the right; the window goes once the motion settles.
pub fn close() {
    let Some(p) = OPEN.with(|o| o.borrow_mut().take()) else { return };
    {
        let mut pp = p.borrow_mut();
        pp.open = false;
        if let Some(scrim) = pp.scrim.take() {
            scrim.destroy();
        }
        let now = glib::monotonic_time();
        pp.slide.go(0.0, now, 240, motion::Curve::new(0.4, 0.0, 0.9, 0.6));
        pp.fade.go(0.0, now, 180, motion::STANDARD);
        pp.window.set_keyboard_mode(KeyboardMode::None);
        if let Some(surface) = pp.window.surface() {
            surface.set_input_region(Some(&cairo::Region::create()));
        }
    }
    animate(&p);
    if let Some(h) = DONE.with(|d| d.borrow_mut().take()) {
        h();
    }
}

fn animate(p: &Rc<RefCell<Pop>>) {
    let paint = p.borrow().paint.clone();
    let p = p.clone();
    paint.add_tick_callback(move |w, clock| {
        w.queue_draw();
        let Ok(pp) = p.try_borrow() else { return glib::ControlFlow::Continue };
        let now = clock.frame_time();
        if !pp.slide.settled(now) || !pp.fade.settled(now) {
            return glib::ControlFlow::Continue;
        }
        if !pp.open {
            let window = pp.window.clone();
            glib::idle_add_local_once(move || {
                draw::forget(&window);
                window.destroy();
            });
        }
        glib::ControlFlow::Break
    });
}

fn midnight(y: i32, m: i32, d: i32) -> Option<glib::DateTime> {
    glib::DateTime::from_local(y, m, d, 0, 0, 0.0).ok()
}

fn today() -> Option<glib::DateTime> {
    let now = glib::DateTime::now_local().ok()?;
    midnight(now.year(), now.month(), now.day_of_month())
}

fn grid_top() -> f64 {
    HEAD + NAMES
}

fn list_top() -> f64 {
    grid_top() + CELL * ROWS as f64 + 12.0
}

/// The panel hangs off the right screen edge under the bar; fillets above and below it meet the edge.
fn surface(agenda: bool) -> (f64, f64) {
    (W, height(agenda) + 2.0 * FILLET)
}

fn height(agenda: bool) -> f64 {
    if agenda {
        list_top() + 24.0 + LINE * LINES as f64 + PAD - 6.0
    } else {
        grid_top() + CELL * ROWS as f64 + PAD - 3.0
    }
}

impl Pop {
    fn cells(&self) -> Vec<glib::DateTime> {
        let Some(first) = midnight(self.year, self.month, 1) else { return Vec::new() };
        let Ok(start) = first.add_days(-(first.day_of_week() - 1)) else { return Vec::new() };
        (0..(ROWS * 7) as i32).filter_map(|i| start.add_days(i).ok()).collect()
    }

    fn spot_at(&self, x: f64, y: f64) -> Option<Spot> {
        let y = y - FILLET;
        if (0.0..HEAD).contains(&y) {
            return if x >= W - PAD - 30.0 {
                Some(Spot::Ahead)
            } else if x >= W - PAD - 62.0 {
                Some(Spot::Back)
            } else if x < W - PAD - 70.0 {
                Some(Spot::Title)
            } else {
                None
            };
        }
        let top = grid_top();
        if y >= top && y < top + CELL * ROWS as f64 && x >= PAD && x < W - PAD {
            let col = (((x - PAD) / CELL_W).floor() as usize).min(6);
            let row = ((y - top) / CELL).floor() as usize;
            return Some(Spot::Day(row * 7 + col));
        }
        None
    }

    fn shift(&mut self, by: i32) {
        let m = self.month - 1 + by;
        self.year += m.div_euclid(12);
        self.month = m.rem_euclid(12) + 1;
    }

    fn home(&mut self) {
        if let Some(t) = today() {
            self.year = t.year();
            self.month = t.month();
            self.pick = t.to_unix();
        }
    }

    fn choose(&mut self, d: &glib::DateTime) {
        self.pick = d.to_unix();
        self.year = d.year();
        self.month = d.month();
    }

    fn step(&mut self, days: i32) {
        let Some(d) = glib::DateTime::from_unix_local(self.pick).ok().and_then(|d| d.add_days(days).ok()) else { return };
        self.choose(&d);
    }

    fn draw(&self, s: &gtk::Snapshot) {
        let now = self.paint.frame_clock().map(|c| c.frame_time()).unwrap_or_else(glib::monotonic_time);
        let fade = self.fade.value(now).clamp(0.0, 1.0);
        let slide = self.slide.value(now);
        if fade < 0.003 && slide < 0.003 {
            return;
        }
        let pal = self.app.palette_at(now);
        let cfg = &self.app.cfg;
        let h = height(self.agenda);
        let (sw, sh) = surface(self.agenda);
        let hot = |spot: Spot| self.hover == Some(spot);
        s.save();
        s.translate(&graphene::Point::new(((1.0 - slide) * sw).round() as f32, 0.0));
        s.save();
        // A notch hanging from the right edge: the top-edge shape turned a quarter.
        s.translate(&graphene::Point::new(sw as f32, 0.0));
        s.rotate(90.0);
        let radius = RADIUS * self.app.round();
        if let Some(p) = shape::docked_parts(sh / 2.0, 0.0, h, W, radius, FILLET) {
            draw::slab(s, &p, pal.body.alpha(1.0));
        }
        s.restore();
        s.translate(&graphene::Point::new(0.0, FILLET as f32));
        let (z, r) = (graphene::Size::zero(), graphene::Size::new(radius as f32, radius as f32));
        s.push_rounded_clip(&gtk::gsk::RoundedRect::new(graphene::Rect::new(0.0, 0.0, W as f32, h as f32), r, z, z, r));
        s.push_opacity(fade);
        let mid = 28.0;
        let title = midnight(self.year, self.month, 1).and_then(|d| d.format("%B %Y").ok()).map(|t| t.to_string()).unwrap_or_default();
        draw::text(s, &self.paint, cfg, &title, 17.0, Face::Text(650), if hot(Spot::Title) { pal.accent } else { pal.ink_main }, PAD + 4.0, mid, W - 2.0 * PAD - 74.0, pango::Alignment::Left);
        for (spot, glyph, cx) in [(Spot::Back, "chevron_left", W - PAD - 46.0), (Spot::Ahead, "chevron_right", W - PAD - 15.0)] {
            if hot(spot) {
                draw::fill(s, &draw::round(cx - 14.0, mid - 14.0, 28.0, 28.0, 14.0), pal.ink_main.alpha(0.08));
            }
            draw::icon(s, &self.paint, cfg, glyph, 22.0, false, pal.ink_sec, cx, mid);
        }
        let cells = self.cells();
        for (i, d) in cells.iter().take(7).enumerate() {
            let name: String = d.format("%a").map(|t| t.chars().take(2).collect()).unwrap_or_default();
            draw::text(s, &self.paint, cfg, &name, 12.5, Face::Text(600), pal.ink_meta, PAD + i as f64 * CELL_W, HEAD + NAMES / 2.0, CELL_W, pango::Alignment::Center);
        }
        let today = today().map(|t| t.to_unix());
        let marks = self.agenda;
        for (i, d) in cells.iter().enumerate() {
            let (col, row) = (i % 7, i / 7);
            let x = PAD + col as f64 * CELL_W;
            let cy = grid_top() + row as f64 * CELL + CELL / 2.0;
            let at = d.to_unix();
            let inside = d.month() == self.month;
            let now_day = Some(at) == today;
            let dot = draw::round(x + CELL_W / 2.0 - 18.0, cy - 18.0, 36.0, 36.0, 18.0);
            if now_day {
                draw::fill(s, &dot, pal.accent);
            } else if at == self.pick {
                draw::fill(s, &dot, pal.ink_main.alpha(0.13));
            } else if hot(Spot::Day(i)) {
                draw::fill(s, &dot, pal.ink_main.alpha(0.07));
            }
            let ink = if now_day {
                pal.on_accent
            } else if !inside {
                pal.ink_faint
            } else if col >= 5 {
                pal.ink_meta
            } else {
                pal.ink_main
            };
            draw::text(s, &self.paint, cfg, &d.day_of_month().to_string(), 15.0, Face::Number(if now_day { 650 } else { 500 }), ink, x, cy, CELL_W, pango::Alignment::Center);
            if marks && inside && !calendar::at(at).is_empty() {
                let tint = if now_day { pal.on_accent } else { pal.accent };
                draw::fill(s, &draw::round(x + CELL_W / 2.0 - 2.0, cy + 11.0, 4.0, 4.0, 2.0), tint);
            }
        }
        if self.agenda {
            let top = list_top();
            draw::fill(s, &draw::round(PAD, top - 6.0, W - 2.0 * PAD, 1.0, 0.0), pal.ink_main.alpha(0.08));
            let picked = glib::DateTime::from_unix_local(self.pick).ok();
            let head = picked.and_then(|d| d.format("%A, %-d %B").ok()).map(|t| t.to_string()).unwrap_or_default();
            draw::text(s, &self.paint, cfg, &head, 12.5, Face::Text(600), pal.ink_meta, PAD + 4.0, top + 10.0, W - 2.0 * PAD - 60.0, pango::Alignment::Left);
            if Some(self.pick) == today {
                draw::text(s, &self.paint, cfg, "Today", 12.5, Face::Text(600), pal.accent, PAD, top + 10.0, W - 2.0 * PAD - 4.0, pango::Alignment::Right);
            }
            let events = calendar::at(self.pick);
            let y0 = top + 24.0;
            if events.is_empty() {
                draw::text(s, &self.paint, cfg, "No events", 13.5, Face::Text(450), pal.ink_faint, PAD + 4.0, y0 + LINE / 2.0, W - 2.0 * PAD, pango::Alignment::Left);
            }
            for (i, ev) in events.iter().take(LINES).enumerate() {
                let y = y0 + i as f64 * LINE + LINE / 2.0;
                if i == LINES - 1 && events.len() > LINES {
                    let more = format!("{} more", events.len() - LINES + 1);
                    draw::text(s, &self.paint, cfg, &more, 12.5, Face::Text(500), pal.ink_meta, PAD + 14.0, y, W - 2.0 * PAD - 14.0, pango::Alignment::Left);
                    break;
                }
                draw::fill(s, &draw::round(PAD + 4.0, y - 8.0, 3.0, 16.0, 1.5), pal.accent);
                let when = if ev.all_day { "All day".to_string() } else { calendar::clock(ev.start) };
                draw::text(s, &self.paint, cfg, &when, 12.5, Face::Number(500), pal.ink_meta, PAD + 14.0, y, 52.0, pango::Alignment::Left);
                draw::text(s, &self.paint, cfg, &ev.title, 13.5, Face::Text(500), pal.ink_main, PAD + 70.0, y, W - 2.0 * PAD - 70.0, pango::Alignment::Left);
            }
        }
        s.pop();
        s.pop();
        s.restore();
    }
}

fn changed(p: &Rc<RefCell<Pop>>) {
    if let Ok(p) = p.try_borrow() {
        p.paint.queue_draw();
    }
}

/// Opens the calendar as a panel sliding in from the right edge, under the bar.
pub fn show(app: &Rc<App>, monitor: Option<gdk::Monitor>, _right: f64, _y: f64) {
    crate::popmenu::close();
    close();
    let hook = crate::popmenu::take_then();
    let Some(now) = today() else {
        if let Some(h) = hook {
            h();
        }
        return;
    };
    crate::calendar::refresh(app);
    let agenda = !app.cfg.calendars.is_empty();
    let window = gtk::Window::new();
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace(Some("digit-menu"));
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(KeyboardMode::Exclusive);
    window.set_monitor(monitor.as_ref());
    window.add_css_class("digit");
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Right, true);
    window.set_margin(Edge::Top, (app.reserve() - FILLET as i32).max(0));
    let paint = Paint::new(1, 1);
    let now_us = glib::monotonic_time();
    let mut slide = Tween::still(0.0);
    slide.go(1.0, now_us, 420, motion::Curve::new(0.16, 1.0, 0.3, 1.0));
    let mut fade = Tween::still(0.0);
    fade.after(1.0, now_us, 60, 220, motion::STANDARD);
    let (sw, sh) = surface(agenda);
    let pop = Rc::new(RefCell::new(Pop {
        app: app.clone(),
        window: window.clone(),
        paint: paint.clone(),
        year: now.year(),
        month: now.month(),
        pick: now.to_unix(),
        hover: None,
        slide,
        fade,
        open: true,
        scrim: None,
        agenda,
    }));
    window.set_default_size(sw as i32, sh as i32);
    paint.set_size_request(sw as i32, sh as i32);
    window.set_child(Some(&paint));
    let weak = Rc::downgrade(&pop);
    paint.on_draw(move |s| {
        if let Some(p) = weak.upgrade() {
            if let Ok(p) = p.try_borrow() {
                p.draw(s);
            }
        }
    });
    let motion = gtk::EventControllerMotion::new();
    let weak = Rc::downgrade(&pop);
    motion.connect_motion(move |_, x, y| {
        let Some(p) = weak.upgrade() else { return };
        let Ok(mut p) = p.try_borrow_mut() else { return };
        let at = p.spot_at(x, y);
        if at != p.hover {
            p.hover = at;
            p.paint.set_cursor_from_name(at.map(|_| "pointer"));
            p.paint.queue_draw();
        }
    });
    let weak = Rc::downgrade(&pop);
    motion.connect_leave(move |_| {
        let Some(p) = weak.upgrade() else { return };
        let Ok(mut p) = p.try_borrow_mut() else { return };
        p.hover = None;
        p.paint.queue_draw();
    });
    paint.add_controller(motion);
    // "leave" mode: the panel slides away shortly after the pointer leaves it.
    let away = gtk::EventControllerMotion::new();
    let gone = Rc::new(std::cell::Cell::new(0u64));
    let g = gone.clone();
    away.connect_enter(move |_, _, _| g.set(g.get() + 1));
    let weak = Rc::downgrade(&pop);
    away.connect_leave(move |_| {
        let generation = gone.get() + 1;
        gone.set(generation);
        let (weak, gone) = (weak.clone(), gone.clone());
        glib::timeout_add_local_once(std::time::Duration::from_millis(crate::app::LEAVE_MS), move || {
            let Some(p) = weak.upgrade() else { return };
            let fold = gone.get() == generation && p.try_borrow().is_ok_and(|p| p.open && p.app.prefs.borrow().calendar_close == "leave");
            if fold {
                close();
            }
        });
    });
    paint.add_controller(away);
    let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL | gtk::EventControllerScrollFlags::DISCRETE);
    let weak = Rc::downgrade(&pop);
    wheel.connect_scroll(move |_, _, dy| {
        let Some(p) = weak.upgrade() else { return glib::Propagation::Proceed };
        if dy != 0.0 {
            p.borrow_mut().shift(if dy > 0.0 { 1 } else { -1 });
            changed(&p);
        }
        glib::Propagation::Stop
    });
    paint.add_controller(wheel);
    let click = gtk::GestureClick::new();
    let weak = Rc::downgrade(&pop);
    click.connect_released(move |_, _, x, y| {
        let Some(p) = weak.upgrade() else { return };
        {
            let mut pp = p.borrow_mut();
            match pp.spot_at(x, y) {
                Some(Spot::Back) => pp.shift(-1),
                Some(Spot::Ahead) => pp.shift(1),
                Some(Spot::Title) => pp.home(),
                Some(Spot::Day(i)) => {
                    if let Some(d) = pp.cells().get(i).cloned() {
                        pp.choose(&d);
                    }
                }
                None => return,
            }
        }
        changed(&p);
    });
    paint.add_controller(click);
    let keys = gtk::EventControllerKey::new();
    let weak = Rc::downgrade(&pop);
    keys.connect_key_pressed(move |_, key, _, _| {
        let Some(p) = weak.upgrade() else { return glib::Propagation::Proceed };
        match key {
            gdk::Key::Escape => {
                close();
                return glib::Propagation::Stop;
            }
            gdk::Key::Left => p.borrow_mut().step(-1),
            gdk::Key::Right => p.borrow_mut().step(1),
            gdk::Key::Up => p.borrow_mut().step(-7),
            gdk::Key::Down => p.borrow_mut().step(7),
            gdk::Key::Page_Up => p.borrow_mut().shift(-1),
            gdk::Key::Page_Down => p.borrow_mut().shift(1),
            gdk::Key::Home | gdk::Key::t => p.borrow_mut().home(),
            _ => return glib::Propagation::Proceed,
        }
        changed(&p);
        glib::Propagation::Stop
    });
    window.add_controller(keys);
    pop.borrow_mut().scrim = Some(crate::paint::scrim(monitor.as_ref(), close));
    OPEN.with(|o| *o.borrow_mut() = Some(pop.clone()));
    DONE.with(|d| *d.borrow_mut() = hook);
    window.present();
    if let Some(surface) = window.surface() {
        let body = cairo::RectangleInt::new(0, FILLET as i32, W as i32, height(agenda) as i32);
        surface.set_input_region(Some(&cairo::Region::create_rectangle(&body)));
    }
    animate(&pop);
}
