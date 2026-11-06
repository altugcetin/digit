use crate::app::App;
use crate::draw::{self, Face};
use crate::paint::Paint;
use gtk::{gdk, glib, pango, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kit::motion::{self, Tween};
use std::cell::RefCell;
use std::rc::Rc;

const ROW: f64 = 32.0;
const SEP: f64 = 11.0;
const PAD: f64 = 6.0;

pub enum Row {
    Item { label: String, hint: String, danger: bool, enabled: bool, act: Rc<dyn Fn()> },
    Sep,
}

impl Row {
    pub fn item(label: impl Into<String>, act: impl Fn() + 'static) -> Row {
        Row::Item { label: label.into(), hint: String::new(), danger: false, enabled: true, act: Rc::new(act) }
    }

    pub fn hint(mut self, h: &str) -> Row {
        if let Row::Item { hint, .. } = &mut self {
            *hint = h.to_string();
        }
        self
    }

    pub fn danger(mut self) -> Row {
        if let Row::Item { danger, .. } = &mut self {
            *danger = true;
        }
        self
    }

    pub fn off(mut self, disabled: bool) -> Row {
        if let Row::Item { enabled, .. } = &mut self {
            *enabled = !disabled;
        }
        self
    }

    fn height(&self) -> f64 {
        match self {
            Row::Item { .. } => ROW,
            Row::Sep => SEP,
        }
    }
}

struct Menu {
    app: Rc<App>,
    window: gtk::Window,
    paint: Paint,
    title: String,
    rows: Vec<Row>,
    hover: Option<usize>,
    width: f64,
    fade: Tween,
    limit: f64,
    scroll: f64,
}

type Hook = Box<dyn FnOnce()>;

thread_local! {
    static OPEN: RefCell<Option<Rc<RefCell<Menu>>>> = const { RefCell::new(None) };
    static NEXT: RefCell<Option<Hook>> = const { RefCell::new(None) };
    static HOOK: RefCell<Option<Hook>> = const { RefCell::new(None) };
    /// Catches a click beside the open menu, which closes it.
    static SCRIM: RefCell<Option<gtk::Window>> = const { RefCell::new(None) };
}

pub fn then(f: impl FnOnce() + 'static) {
    NEXT.with(|n| *n.borrow_mut() = Some(Box::new(f)));
}

pub fn take_then() -> Option<Box<dyn FnOnce()>> {
    NEXT.with(|n| n.borrow_mut().take())
}

pub fn settle() {
    if let Some(h) = NEXT.with(|n| n.borrow_mut().take()) {
        h();
    }
}

pub fn is_open() -> bool {
    OPEN.with(|o| o.borrow().is_some())
}

pub fn close() {
    if let Some(s) = SCRIM.with(|s| s.borrow_mut().take()) {
        s.destroy();
    }
    let Some(m) = OPEN.with(|o| o.borrow_mut().take()) else { return };
    let window = m.borrow().window.clone();
    glib::idle_add_local_once(move || window.destroy());
    if let Some(h) = HOOK.with(|h| h.borrow_mut().take()) {
        h();
    }
}

impl Menu {
    fn top(&self) -> f64 {
        PAD + if self.title.is_empty() { 0.0 } else { 26.0 }
    }

    fn height(&self) -> f64 {
        self.top() + self.rows.iter().map(Row::height).sum::<f64>() + PAD
    }

    fn shown(&self) -> f64 {
        self.height().min(self.limit)
    }

    fn reveal(&mut self, i: usize) {
        let top = self.top() + self.rows.iter().take(i).map(Row::height).sum::<f64>();
        let view = self.shown();
        if top - self.scroll < PAD {
            self.scroll = (top - PAD).max(0.0);
        } else if top + ROW - self.scroll > view - PAD {
            self.scroll = (top + ROW - view + PAD).min((self.height() - self.limit).max(0.0));
        }
    }

    fn row_at(&self, y: f64) -> Option<usize> {
        let y = y + self.scroll;
        let mut top = self.top();
        for (i, r) in self.rows.iter().enumerate() {
            let h = r.height();
            if y >= top && y < top + h {
                return matches!(r, Row::Item { enabled: true, .. }).then_some(i);
            }
            top += h;
        }
        None
    }

    fn draw(&self, s: &gtk::Snapshot) {
        let now = self.paint.frame_clock().map(|c| c.frame_time()).unwrap_or_else(glib::monotonic_time);
        let fade = self.fade.value(now).clamp(0.0, 1.0);
        let pal = self.app.palette_at(now);
        let cfg = &self.app.cfg;
        let (w, h) = (self.width, self.shown());
        s.push_opacity(fade);
        draw::fill(s, &draw::round(0.0, 0.0, w, h, 14.0), pal.body.alpha(1.0));
        let edge = gdk::RGBA::new(1.0, 1.0, 1.0, 0.07);
        s.append_border(&draw::round(0.0, 0.0, w, h, 14.0), &[1.0; 4], &[edge, edge, edge, edge]);
        s.push_rounded_clip(&draw::round(0.0, 0.0, w, h, 14.0));
        s.save();
        s.translate(&gtk::graphene::Point::new(0.0, -self.scroll as f32));
        if !self.title.is_empty() {
            draw::text(s, &self.paint, cfg, &self.title, 12.5, Face::Text(600), pal.ink_meta, 14.0, PAD + 13.0, w - 28.0, pango::Alignment::Left);
        }
        let mut y = self.top();
        for (i, r) in self.rows.iter().enumerate() {
            match r {
                Row::Sep => {
                    draw::fill(s, &draw::round(12.0, y + SEP / 2.0, w - 24.0, 1.0, 0.0), pal.ink_main.alpha(0.08));
                }
                Row::Item { label, hint, danger, enabled, .. } => {
                    let on = self.hover == Some(i);
                    if on {
                        draw::fill(s, &draw::round(5.0, y + 1.0, w - 10.0, ROW - 2.0, 9.0), if *danger { pal.risk.alpha(0.85) } else { pal.accent });
                    }
                    let ink = if !enabled {
                        pal.ink_faint
                    } else if on {
                        pal.on_accent
                    } else if *danger {
                        pal.risk
                    } else {
                        pal.ink_main
                    };
                    draw::text(s, &self.paint, cfg, label, 14.5, Face::Text(if on { 520 } else { 450 }), ink, 14.0, y + ROW / 2.0, w - 28.0, pango::Alignment::Left);
                    if !hint.is_empty() {
                        let hink = if on { pal.on_accent.alpha(0.75) } else { pal.ink_meta };
                        draw::text(s, &self.paint, cfg, hint, 13.0, Face::Text(450), hink, 14.0, y + ROW / 2.0, w - 28.0, pango::Alignment::Right);
                    }
                }
            }
            y += r.height();
        }
        s.restore();
        s.pop();
        s.pop();
    }
}

pub fn show(app: &Rc<App>, monitor: Option<gdk::Monitor>, x: f64, y: f64, title: &str, rows: Vec<Row>) {
    show_inner(app, monitor, x, y, title, rows, true);
}

/// Desktop clicks near the top edge still need an outside-click scrim over the entire output.
pub fn show_desktop(app: &Rc<App>, monitor: Option<gdk::Monitor>, x: f64, y: f64, rows: Vec<Row>) {
    show_inner(app, monitor, x, y, "", rows, false);
}

fn show_inner(app: &Rc<App>, monitor: Option<gdk::Monitor>, x: f64, y: f64, title: &str, rows: Vec<Row>, leave_bar: bool) {
    close();
    crate::datepop::close();
    let hook = NEXT.with(|n| n.borrow_mut().take());
    if rows.is_empty() {
        if let Some(h) = hook {
            h();
        }
        return;
    }
    let window = gtk::Window::new();
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace(Some("digit-menu"));
    window.set_exclusive_zone(-1);
    // The keyboard stays with the menu while it is open, so focus-follows-mouse can't move
    // focus to a window under the pointer on the way to the menu; a click beside it closes it.
    window.set_keyboard_mode(KeyboardMode::Exclusive);
    window.set_monitor(monitor.as_ref());
    window.add_css_class("digit");
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Left, true);
    let scrim = crate::paint::scrim(monitor.as_ref(), close);
    // A menu from the menu bar leaves the bar uncovered, so moving to the next menu still works.
    if leave_bar && y < 80.0 {
        scrim.set_margin(Edge::Top, y.max(0.0) as i32);
    }
    SCRIM.with(|s| *s.borrow_mut() = Some(scrim));
    let paint = Paint::new(1, 1);
    let mut width: f64 = 200.0;
    for r in &rows {
        if let Row::Item { label, hint, .. } = r {
            let lw = draw::width(&paint, &app.cfg, label, 14.5, Face::Text(520));
            let hw = if hint.is_empty() { 0.0 } else { 28.0 + draw::width(&paint, &app.cfg, hint, 13.0, Face::Text(450)) };
            width = width.max(lw + hw + 40.0);
        }
    }
    let width = width.min(420.0).ceil();
    let mut fade = Tween::still(0.0);
    fade.go(1.0, glib::monotonic_time(), 140, motion::STANDARD);
    let screen_w = monitor.as_ref().map(|m| m.geometry().width() as f64).unwrap_or(1920.0);
    let screen_h = monitor.as_ref().map(|m| m.geometry().height() as f64).unwrap_or(1080.0);
    let limit = (screen_h - y - 16.0).max(160.0);
    let menu = Rc::new(RefCell::new(Menu { app: app.clone(), window: window.clone(), paint: paint.clone(), title: title.to_string(), rows, hover: None, width, fade, limit, scroll: 0.0 }));
    let h = menu.borrow().shown();
    window.set_margin(Edge::Left, x.clamp(6.0, (screen_w - width - 6.0).max(6.0)) as i32);
    window.set_margin(Edge::Top, y as i32);
    window.set_default_size(width as i32, h as i32);
    paint.set_size_request(width as i32, h as i32);
    window.set_child(Some(&paint));
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
    motion.connect_motion(move |_, _, y| {
        let Some(m) = weak.upgrade() else { return };
        let Ok(mut m) = m.try_borrow_mut() else { return };
        let at = m.row_at(y);
        if at != m.hover {
            m.hover = at;
            m.paint.set_cursor_from_name(at.map(|_| "pointer"));
            m.paint.queue_draw();
        }
    });
    let weak = Rc::downgrade(&menu);
    motion.connect_leave(move |_| {
        let Some(m) = weak.upgrade() else { return };
        let Ok(mut m) = m.try_borrow_mut() else { return };
        m.hover = None;
        m.paint.queue_draw();
    });
    paint.add_controller(motion);
    let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    let weak = Rc::downgrade(&menu);
    wheel.connect_scroll(move |_, _, dy| {
        let Some(m) = weak.upgrade() else { return glib::Propagation::Proceed };
        let Ok(mut m) = m.try_borrow_mut() else { return glib::Propagation::Proceed };
        let max = (m.height() - m.limit).max(0.0);
        m.scroll = (m.scroll + dy * 36.0).clamp(0.0, max);
        m.hover = None;
        m.paint.queue_draw();
        glib::Propagation::Stop
    });
    paint.add_controller(wheel);
    let run = |m: &Rc<RefCell<Menu>>, i: usize| {
        let act = match m.borrow().rows.get(i) {
            Some(Row::Item { act, enabled: true, .. }) => Some(act.clone()),
            _ => None,
        };
        close();
        if let Some(act) = act {
            glib::idle_add_local_once(move || act());
        }
    };
    let click = gtk::GestureClick::new();
    let weak = Rc::downgrade(&menu);
    click.connect_released(move |_, _, _, y| {
        let Some(m) = weak.upgrade() else { return };
        let at = m.borrow().row_at(y);
        if let Some(i) = at {
            run(&m, i);
        }
    });
    paint.add_controller(click);
    let keys = gtk::EventControllerKey::new();
    let weak = Rc::downgrade(&menu);
    keys.connect_key_pressed(move |_, key, _, _| {
        let Some(m) = weak.upgrade() else { return glib::Propagation::Proceed };
        match key {
            gdk::Key::Escape => close(),
            gdk::Key::Down | gdk::Key::Up => {
                let mut mm = m.borrow_mut();
                let picks: Vec<usize> = mm.rows.iter().enumerate().filter(|(_, r)| matches!(r, Row::Item { enabled: true, .. })).map(|(i, _)| i).collect();
                if picks.is_empty() {
                    return glib::Propagation::Stop;
                }
                let at = mm.hover.and_then(|h| picks.iter().position(|p| *p == h));
                let next = match (at, key == gdk::Key::Down) {
                    (None, true) => 0,
                    (None, false) => picks.len() - 1,
                    (Some(a), true) => (a + 1) % picks.len(),
                    (Some(a), false) => (a + picks.len() - 1) % picks.len(),
                };
                mm.hover = Some(picks[next]);
                mm.reveal(picks[next]);
                mm.paint.queue_draw();
            }
            gdk::Key::Return | gdk::Key::KP_Enter => {
                let at = m.borrow().hover;
                if let Some(i) = at {
                    run(&m, i);
                }
            }
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    window.add_controller(keys);
    OPEN.with(|o| *o.borrow_mut() = Some(menu.clone()));
    HOOK.with(|h| *h.borrow_mut() = hook);
    window.present();
    let weak = Rc::downgrade(&menu);
    paint.add_tick_callback(move |p, clock| {
        p.queue_draw();
        let busy = weak.upgrade().and_then(|m| m.try_borrow().ok().map(|m| !m.fade.settled(clock.frame_time()))).unwrap_or(false);
        if busy { glib::ControlFlow::Continue } else { glib::ControlFlow::Break }
    });
}
