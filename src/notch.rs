use crate::app::{App, Ending, Peek};
use crate::config::Dock;
use crate::motion::{self, Tween};
use crate::palette::Rgb;
use crate::{blur, media, shape};
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{cairo, gdk, glib, graphene, gsk, pango};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::time::Duration;

const WIDTH: f64 = 860.0;
const ROOM: f64 = 420.0;
const CENTRE: f64 = 116.0;
const CX: f64 = WIDTH / 2.0;
const SHEET_W: f64 = 560.0;
const ROLL: i64 = 260_000;

#[derive(Clone, Debug, PartialEq)]
enum Head {
    Idle(bool),
    Open,
    Peek(Peek),
}

impl Head {
    fn same(&self, other: &Head) -> bool {
        match (self, other) {
            (Head::Peek(a), Head::Peek(b)) => a.same_kind(b),
            _ => self == other,
        }
    }
}

#[derive(Clone, Copy)]
enum Action {
    Previous,
    Toggle,
    Next,
}

struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Rect {
    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x <= self.x + self.w && y >= self.y && y <= self.y + self.h
    }

    fn graphene(&self) -> graphene::Rect {
        graphene::Rect::new(self.x as f32, self.y as f32, self.w as f32, self.h as f32)
    }
}

struct Player {
    art: Rect,
    info_x: f64,
    info_w: f64,
    top: f64,
    controls: [(Rect, Action); 3],
}

struct View {
    app: Rc<App>,
    window: gtk::Window,
    canvas: Canvas,
    output: String,
    open: bool,
    hovered: bool,
    dwell: u64,
    leave: u64,
    w: Tween,
    depth: Tween,
    r: Tween,
    fade: Tween,
    sheet: Tween,
    back: Tween,
    meter: Tween,
    art_scale: Tween,
    head: Head,
    want: Head,
    shown: Vec<char>,
    was: Vec<char>,
    roll: Vec<Option<i64>>,
    region: (i32, i32, i32, i32),
    effect: Option<blur::Effect>,
    frost: Vec<(i32, i32, i32, i32)>,
    ticking: bool,
}

#[derive(Clone)]
pub struct Notch(Rc<RefCell<View>>);

impl Notch {
    pub fn new(app: &Rc<App>, monitor: &gdk::Monitor) -> Self {
        let header = app.cfg.header();
        let window = gtk::Window::new();
        window.init_layer_shell();
        window.set_layer(Layer::Top);
        window.set_namespace(Some("digit"));
        window.set_anchor(Edge::Top, true);
        window.set_exclusive_zone(app.cfg.reserve());
        window.set_keyboard_mode(KeyboardMode::None);
        window.set_monitor(Some(monitor));
        window.add_css_class("digit");
        window.set_default_size(WIDTH as i32, (header + ROOM) as i32);
        let canvas = Canvas::default();
        canvas.set_size_request(WIDTH as i32, (header + ROOM) as i32);
        window.set_child(Some(&canvas));
        let clock: Vec<char> = app.clock.borrow().chars().collect();
        let view = Rc::new(RefCell::new(View {
            app: app.clone(),
            window: window.clone(),
            canvas: canvas.clone(),
            output: monitor.connector().map(|c| c.to_string()).unwrap_or_default(),
            open: false,
            hovered: false,
            dwell: 0,
            leave: 0,
            w: Tween::still(CENTRE),
            depth: Tween::still(0.0),
            r: Tween::still(14.0),
            fade: Tween::still(1.0),
            sheet: Tween::still(0.0),
            back: Tween::still(0.0),
            meter: Tween::still(0.0),
            art_scale: Tween::still(1.0),
            head: Head::Idle(false),
            want: Head::Idle(false),
            roll: vec![None; clock.len()],
            was: clock.clone(),
            shown: clock,
            region: (0, 0, 0, 0),
            effect: None,
            frost: Vec::new(),
            ticking: false,
        }));
        *canvas.imp().view.borrow_mut() = Rc::downgrade(&view);
        let notch = Notch(view);
        notch.wire();
        window.present();
        notch.refresh();
        notch
    }

    fn wire(&self) {
        let canvas = self.0.borrow().canvas.clone();
        let motion = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(&self.0);
        motion.connect_enter(move |_, _, _| {
            if let Some(v) = weak.upgrade() {
                Notch(v).hover(true);
            }
        });
        let weak = Rc::downgrade(&self.0);
        motion.connect_leave(move |_| {
            if let Some(v) = weak.upgrade() {
                Notch(v).hover(false);
            }
        });
        canvas.add_controller(motion);
        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(&self.0);
        click.connect_released(move |_, _, x, y| {
            if let Some(v) = weak.upgrade() {
                Notch(v).click(x, y);
            }
        });
        canvas.add_controller(click);
        let weak = Rc::downgrade(&self.0);
        self.0.borrow().window.connect_realize(move |_| {
            if let Some(v) = weak.upgrade() {
                Notch(v).kick();
            }
        });
    }

    pub fn output(&self) -> String {
        self.0.borrow().output.clone()
    }

    pub fn destroy(&self) {
        let (window, open, app) = {
            let v = self.0.borrow();
            (v.window.clone(), v.open, v.app.clone())
        };
        if open {
            app.set_open(false);
        }
        window.destroy();
    }

    pub fn redraw(&self) {
        self.0.borrow().canvas.queue_draw();
    }

    pub fn is_open(&self) -> bool {
        self.0.borrow().open
    }

    pub fn set_open(&self, open: bool) {
        let app = {
            let mut v = self.0.borrow_mut();
            if v.open == open {
                return;
            }
            v.open = open;
            v.dwell += 1;
            v.leave += 1;
            v.app.clone()
        };
        app.set_open(open);
        self.refresh();
    }

    pub fn refresh(&self) {
        let start_dwell = {
            let Ok(mut v) = self.0.try_borrow_mut() else {
                let weak = Rc::downgrade(&self.0);
                glib::idle_add_local_once(move || {
                    if let Some(v) = weak.upgrade() {
                        Notch(v).refresh();
                    }
                });
                return;
            };
            v.retarget()
        };
        if start_dwell {
            self.dwell();
        }
        self.kick();
    }

    fn hover(&self, inside: bool) {
        let (open, peek, hover) = {
            let mut v = self.0.borrow_mut();
            v.hovered = inside;
            v.leave += 1;
            v.dwell += 1;
            (v.open, v.app.peek.borrow().is_some(), v.app.cfg.hover)
        };
        if inside {
            if hover && !open && !peek {
                self.dwell();
            }
        } else {
            let generation = self.0.borrow().leave;
            let weak = Rc::downgrade(&self.0);
            glib::timeout_add_local_once(Duration::from_millis(100), move || {
                if let Some(v) = weak.upgrade() {
                    let fold = {
                        let v = v.borrow();
                        v.leave == generation && !v.hovered
                    };
                    if fold {
                        Notch(v).set_open(false);
                    }
                }
            });
        }
    }

    fn dwell(&self) {
        let generation = {
            let mut v = self.0.borrow_mut();
            v.dwell += 1;
            v.dwell
        };
        let weak = Rc::downgrade(&self.0);
        glib::timeout_add_local_once(Duration::from_millis(300), move || {
            if let Some(v) = weak.upgrade() {
                let open = {
                    let v = v.borrow();
                    v.dwell == generation && v.hovered && !v.open && v.app.peek.borrow().is_none()
                };
                if open {
                    Notch(v).set_open(true);
                }
            }
        });
    }

    fn click(&self, x: f64, y: f64) {
        let (app, open, action) = {
            let v = self.0.borrow();
            let action = if v.open {
                v.player(App::now()).and_then(|p| p.controls.into_iter().find(|(r, _)| r.contains(x, y)).map(|(_, a)| a))
            } else {
                None
            };
            (v.app.clone(), v.open, action)
        };
        if let Some(action) = action {
            let cmd = match action {
                Action::Previous => media::Command::Previous,
                Action::Toggle => media::Command::PlayPause,
                Action::Next => media::Command::Next,
            };
            if let Some(p) = app.players.borrow().as_ref() {
                p.command(cmd);
            }
            return;
        }
        if open {
            if y <= app.cfg.header() {
                self.set_open(false);
            }
            return;
        }
        let peeking = app.peek.borrow().is_some();
        if peeking {
            app.dismiss(Ending::Clicked);
        } else {
            self.set_open(true);
        }
    }

    fn kick(&self) {
        let (canvas, already) = {
            let mut v = self.0.borrow_mut();
            let already = v.ticking;
            v.ticking = true;
            (v.canvas.clone(), already)
        };
        if already {
            return;
        }
        let weak: Weak<RefCell<View>> = Rc::downgrade(&self.0);
        canvas.add_tick_callback(move |w, clock| {
            let Some(view) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let Ok(mut v) = view.try_borrow_mut() else {
                return glib::ControlFlow::Continue;
            };
            let busy = v.step(clock.frame_time());
            w.queue_draw();
            if busy {
                glib::ControlFlow::Continue
            } else {
                v.ticking = false;
                glib::ControlFlow::Break
            }
        });
    }
}

#[derive(Clone, Copy)]
enum Face {
    Text(i32),
    Number(i32),
    Icon(bool),
}

const POINT: Face = Face::Text(560);
const REST: Face = Face::Text(450);

impl View {
    fn header(&self) -> f64 {
        self.app.cfg.header()
    }

    fn target(&self, now: i64) -> Head {
        if self.open {
            return Head::Open;
        }
        if let Some(p) = self.app.peek.borrow().clone() {
            return Head::Peek(p);
        }
        Head::Idle(self.app.media.borrow().wings(now))
    }

    fn clock_width(&self) -> f64 {
        let text: String = self.shown.iter().collect();
        self.text_width(&text, 16.0, Face::Number(500)).max(self.text_width("00:00", 16.0, Face::Number(500)))
    }

    fn centre(&self) -> f64 {
        CENTRE + (self.clock_width() - self.text_width("00:00", 16.0, Face::Number(500))).max(0.0)
    }

    fn geometry(&self, head: &Head, now: i64) -> (f64, f64, f64) {
        match head {
            Head::Idle(media) => (self.centre() + if *media { 64.0 } else { 0.0 }, 0.0, 14.0),
            Head::Peek(p) => (self.centre() + 2.0 * p.wing(), 0.0, 14.0),
            Head::Open => (SHEET_W, if self.app.media.borrow().present(now) { 150.0 } else { 72.0 }, 24.0),
        }
    }

    fn retarget(&mut self) -> bool {
        let now = App::now();
        let clock: Vec<char> = self.app.clock.borrow().chars().collect();
        if clock != self.shown {
            let mut roll = vec![None; clock.len()];
            if clock.len() == self.shown.len() {
                let mut k = 0;
                for i in (0..clock.len()).rev() {
                    if clock[i] != self.shown[i] {
                        roll[i] = Some(now + k * 40_000);
                        k += 1;
                    }
                }
            }
            self.was = std::mem::replace(&mut self.shown, clock);
            self.roll = roll;
        }
        let want = self.target(now);
        let was_peek = matches!(self.want, Head::Peek(_));
        let (w, depth, r) = self.geometry(&want, now);
        let (ms, curve) = if self.open {
            (500, motion::DEFAULT_SPATIAL)
        } else if matches!(want, Head::Peek(_)) {
            (350, motion::FAST_SPATIAL)
        } else {
            (450, motion::EMPHASIZED_DECEL)
        };
        self.w.go(w, now, ms, curve);
        self.depth.go(depth, now, ms, curve);
        self.r.go(r, now, ms, curve);
        if self.open && self.sheet.target() < 1.0 {
            self.sheet.after(1.0, now, ms * 11 / 20, 220, motion::STANDARD);
        } else if !self.open {
            self.sheet.go(0.0, now, 90, motion::FAST_EFFECTS);
        }
        self.back.go(if matches!(want, Head::Peek(_)) { 1.0 } else { 0.0 }, now, 200, motion::STANDARD);
        if let Head::Peek(Peek::Volume { level, .. }) = &want {
            if !matches!(self.head, Head::Peek(Peek::Volume { .. })) {
                self.meter.set(*level);
            }
            self.meter.go(level.clamp(0.0, 1.0), now, 200, motion::STANDARD);
        }
        let playing = self.app.media.borrow().playing;
        self.art_scale.go(if playing { 1.0 } else { 0.85 }, now, 400, motion::DEFAULT_SPATIAL);
        if self.head.same(&want) {
            self.head = want.clone();
            if self.fade.target() < 1.0 {
                self.fade.go(1.0, now, 200, motion::STANDARD);
            }
        } else if self.fade.target() > 0.0 {
            self.fade.go(0.0, now, 110, motion::FAST_EFFECTS);
        }
        let now_peek = matches!(want, Head::Peek(_));
        self.want = want;
        was_peek && !now_peek && self.hovered && !self.open && self.app.cfg.hover
    }

    fn step(&mut self, now: i64) -> bool {
        if self.fade.target() == 0.0 && self.fade.settled(now) {
            self.head = self.want.clone();
            let shape_left = self.w.left_ms(now).max(self.depth.left_ms(now));
            self.fade.after(1.0, now, shape_left * 2 / 5, 200, motion::STANDARD);
        }
        self.update_region(now);
        self.update_frost(now);
        let tweens = [&self.w, &self.depth, &self.r, &self.fade, &self.sheet, &self.back, &self.meter, &self.art_scale];
        let moving = tweens.iter().any(|t| !t.settled(now));
        let rolling = self.roll.iter().flatten().any(|t| now < t + ROLL);
        moving || rolling
    }

    fn update_region(&mut self, now: i64) {
        let w = self.w.value(now).max(self.w.target()) + 12.0;
        let h = self.header() + self.depth.value(now).max(self.depth.target());
        let rect = ((CX - w / 2.0).floor() as i32, 0, w.ceil() as i32, h.ceil() as i32);
        if rect == self.region {
            return;
        }
        if let Some(surface) = self.window.surface() {
            let region = cairo::Region::create_rectangle(&cairo::RectangleInt::new(rect.0, rect.1, rect.2, rect.3));
            surface.set_input_region(Some(&region));
            self.region = rect;
        }
    }

    fn frosted(&self) -> bool {
        self.app.palette.get().body.a < 1.0
    }

    fn update_frost(&mut self, now: i64) {
        if self.app.cfg.dock != Dock::Screen || !self.frosted() {
            return;
        }
        let Some(blur) = self.app.blur.as_ref() else { return };
        if self.effect.is_none() {
            let Some(surface) = self.window.surface() else { return };
            self.effect = blur.attach(&surface);
        }
        let Some(effect) = self.effect.as_ref() else { return };
        let depth = self.header() + self.depth.value(now).max(0.0);
        let rects = shape::rows(CX, 0.0, self.w.value(now), depth, self.r.value(now), 6.0, 1.5);
        if rects != self.frost {
            blur.set(effect, &rects);
            self.frost = rects;
        }
    }

    fn layout(&self, text: &str, px: f64, face: Face) -> pango::Layout {
        let l = self.canvas.create_pango_layout(Some(text));
        let mut fd = pango::FontDescription::new();
        fd.set_absolute_size(px * pango::SCALE as f64);
        match face {
            Face::Text(w) => {
                fd.set_family(&self.app.cfg.font);
                fd.set_variations(Some(&format!("wght={w}")));
                fd.set_weight(if w >= 550 { pango::Weight::Semibold } else if w >= 480 { pango::Weight::Medium } else { pango::Weight::Normal });
            }
            Face::Number(w) => {
                fd.set_family(&self.app.cfg.numbers);
                fd.set_variations(Some(&format!("wght={w}")));
                fd.set_weight(if w >= 550 { pango::Weight::Semibold } else if w >= 480 { pango::Weight::Medium } else { pango::Weight::Normal });
            }
            Face::Icon(fill) => {
                fd.set_family(&self.app.cfg.icon_font);
                fd.set_variations(Some(&format!("FILL={},wght=400,opsz={}", fill as i32, px.clamp(20.0, 48.0))));
            }
        }
        l.set_font_description(Some(&fd));
        if matches!(face, Face::Number(_)) {
            let attrs = pango::AttrList::new();
            attrs.insert(pango::AttrFontFeatures::new("tnum 1"));
            l.set_attributes(Some(&attrs));
        }
        l
    }

    fn text_width(&self, text: &str, px: f64, face: Face) -> f64 {
        self.layout(text, px, face).pixel_extents().1.width() as f64
    }

    #[allow(clippy::too_many_arguments)]
    fn text(&self, s: &gtk::Snapshot, text: &str, px: f64, face: Face, color: Rgb, x: f64, mid: f64, width: f64, align: pango::Alignment) {
        if width <= 4.0 || text.is_empty() {
            return;
        }
        let l = self.layout(text, px, face);
        l.set_width((width * pango::SCALE as f64) as i32);
        l.set_ellipsize(pango::EllipsizeMode::End);
        l.set_single_paragraph_mode(true);
        l.set_alignment(align);
        let h = l.pixel_extents().1.height() as f64;
        s.save();
        s.translate(&graphene::Point::new(x.round() as f32, (mid - h / 2.0).round() as f32));
        s.append_layout(&l, &color.rgba());
        s.restore();
    }

    fn icon(&self, s: &gtk::Snapshot, name: &str, px: f64, fill: bool, color: Rgb, cx: f64, mid: f64) {
        let l = self.layout(name, px, Face::Icon(fill));
        let (ink, _) = l.pixel_extents();
        s.save();
        s.translate(&graphene::Point::new(
            (cx - ink.x() as f64 - ink.width() as f64 / 2.0).round() as f32,
            (mid - ink.y() as f64 - ink.height() as f64 / 2.0).round() as f32,
        ));
        s.append_layout(&l, &color.rgba());
        s.restore();
    }

    fn draw(&self, canvas: &Canvas, s: &gtk::Snapshot) {
        let now = canvas.frame_clock().map(|c| c.frame_time()).unwrap_or_else(glib::monotonic_time);
        let pal = self.app.palette.get();
        let header = self.header();
        let width = self.w.value(now);
        let sheet = self.depth.value(now).max(0.0);
        let (top, depth) = match self.app.cfg.dock {
            Dock::Bar => (header, sheet),
            Dock::Screen => (0.0, header + sheet),
        };
        let body = shape::docked(CX, top, width, depth, self.r.value(now), 6.0);
        if let Some(path) = &body {
            s.append_fill(path, gsk::FillRule::Winding, &pal.body.rgba());
            if let (true, Some(rim)) = (pal.body.a < 1.0, shape::rim(CX, top, width, depth, self.r.value(now), 6.0)) {
                s.push_fill(path, gsk::FillRule::Winding);
                for (width, alpha) in [(6.0, 0.18), (4.0, 0.25), (2.0, 0.35)] {
                    s.append_stroke(&rim, &gsk::Stroke::new(width), &pal.body.alpha(alpha).rgba());
                }
                s.pop();
            }
        }
        let clip = match (&body, self.app.cfg.dock) {
            (Some(path), Dock::Screen) => {
                s.push_fill(path, gsk::FillRule::Winding);
                true
            }
            _ => false,
        };
        let fade = self.fade.value(now).clamp(0.0, 1.0);
        if fade > 0.01 {
            s.push_opacity(fade);
            self.draw_head(s, now);
            s.pop();
        }
        self.draw_clock(s, now);
        if clip {
            s.pop();
        }
        let shown = self.sheet.value(now).clamp(0.0, 1.0);
        if let (Some(path), true) = (&body, sheet > 1.0 && shown > 0.01) {
            s.push_fill(path, gsk::FillRule::Winding);
            s.push_opacity(shown);
            self.draw_sheet(s, now);
            s.pop();
            s.pop();
        }
    }

    fn draw_clock(&self, s: &gtk::Snapshot, now: i64) {
        let pal = self.app.palette.get();
        let color = pal.ink_main.mix(pal.ink_meta, 1.0 - self.back.value(now).clamp(0.0, 1.0));
        let mid = self.header() / 2.0;
        let face = Face::Number(500);
        let cells: Vec<(pango::Layout, f64)> = self
            .shown
            .iter()
            .map(|c| {
                let l = self.layout(&c.to_string(), 16.0, face);
                let w = l.pixel_extents().1.width() as f64;
                (l, w)
            })
            .collect();
        let total: f64 = cells.iter().map(|c| c.1).sum();
        let mut x = (CX - total / 2.0).round();
        let line = cells.first().map(|c| c.0.pixel_extents().1.height() as f64).unwrap_or(20.0);
        let y = (mid - line / 2.0).round();
        for (i, (l, w)) in cells.iter().enumerate() {
            let e = match self.roll.get(i).copied().flatten() {
                Some(t) if now < t + ROLL => motion::IN_OUT_CUBIC.at(((now - t).max(0)) as f64 / ROLL as f64),
                _ => 1.0,
            };
            if e < 1.0 {
                s.push_clip(&graphene::Rect::new(x as f32, y as f32, *w as f32, line as f32));
                if let Some(old) = self.was.get(i) {
                    let ol = self.layout(&old.to_string(), 16.0, face);
                    s.save();
                    s.translate(&graphene::Point::new(x as f32, (y - line * e) as f32));
                    s.append_layout(&ol, &color.rgba());
                    s.restore();
                }
                s.save();
                s.translate(&graphene::Point::new(x as f32, (y + line * (1.0 - e)) as f32));
                s.append_layout(l, &color.rgba());
                s.restore();
                s.pop();
            } else {
                s.save();
                s.translate(&graphene::Point::new(x as f32, y as f32));
                s.append_layout(l, &color.rgba());
                s.restore();
            }
            x += w;
        }
    }

    fn wings(&self, head: &Head, now: i64) -> (f64, f64, f64, f64) {
        let (w, _, _) = self.geometry(head, now);
        let half = self.clock_width() / 2.0;
        (CX - w / 2.0 + 12.0, CX - half - 10.0, CX + half + 10.0, CX + w / 2.0 - 12.0)
    }

    fn draw_head(&self, s: &gtk::Snapshot, now: i64) {
        let pal = self.app.palette.get();
        let mid = self.header() / 2.0;
        let (l0, l1, r0, r1) = self.wings(&self.head, now);
        let (left, right) = (l1 - l0, r1 - r0);
        let start = pango::Alignment::Left;
        let end = pango::Alignment::Right;
        match &self.head {
            Head::Idle(false) => {}
            Head::Idle(true) => {
                self.draw_art(s, &Rect { x: l0, y: mid - 10.0, w: 20.0, h: 20.0 }, 5.0, 1.0);
                let bars = self.app.spectrum.borrow();
                let n = 5usize;
                for i in 0..n {
                    let v = bars.get(i).copied().unwrap_or(0.0).clamp(0.0, 1.0);
                    let h = (3.0 + 11.0 * v).round();
                    let x = r1 - (n - i) as f64 * 5.0 + 2.0;
                    let rect = graphene::Rect::new(x as f32, (mid - h / 2.0) as f32, 3.0, h as f32);
                    s.push_rounded_clip(&gsk::RoundedRect::from_rect(rect, 1.5));
                    s.append_color(&pal.ink_sec.rgba(), &rect);
                    s.pop();
                }
            }
            Head::Open => {
                let date = self.app.date.borrow().clone();
                self.text(s, &date, 15.0, REST, pal.ink_meta, l0, mid, left, start);
                if let Some((level, muted)) = self.app.volume.get() {
                    let icon = if muted { "volume_off" } else if level < 0.5 { "volume_down" } else { "volume_up" };
                    let text = if muted { String::new() } else { format!("{}%", (level * 100.0).round()) };
                    let tw = self.text_width(&text, 15.0, Face::Number(450));
                    self.text(s, &text, 15.0, Face::Number(450), pal.ink_meta, r0, mid, right, end);
                    self.icon(s, icon, 18.0, false, pal.ink_meta, r1 - tw - 14.0, mid);
                }
            }
            Head::Peek(Peek::Volume { level, muted }) => {
                let icon = if *muted { "volume_off" } else if *level < 0.5 { "volume_down" } else { "volume_up" };
                self.icon(s, icon, 18.0, false, pal.ink_main, l0 + 9.0, mid);
                self.text(s, if *muted { "Muted" } else { "Volume" }, 15.0, REST, pal.ink_meta, l0 + 24.0, mid, left - 24.0, start);
                let value = self.meter.value(now).clamp(0.0, 1.0);
                let percent = format!("{}%", (value * 100.0).round());
                self.text(s, &percent, 15.0, Face::Number(560), pal.ink_main, r1 - 40.0, mid, 40.0, end);
                let track = Rect { x: r0, y: mid - 1.5, w: (right - 48.0).max(0.0), h: 3.0 };
                self.bar(s, &track, value, pal.ink_main.alpha(0.15), pal.ink_main);
            }
            Head::Peek(Peek::Track { title, artist }) => {
                self.draw_art(s, &Rect { x: l0, y: mid - 10.0, w: 20.0, h: 20.0 }, 5.0, 1.0);
                self.text(s, title, 15.0, POINT, pal.ink_main, l0 + 28.0, mid, left - 28.0, start);
                self.text(s, artist, 15.0, REST, pal.ink_meta, r0, mid, right, end);
            }
            Head::Peek(Peek::Notification { summary, app, .. }) => {
                self.text(s, summary, 15.0, POINT, pal.ink_main, l0, mid, left, start);
                self.text(s, app, 15.0, REST, pal.ink_meta, r0, mid, right, end);
            }
            Head::Peek(Peek::Note { text }) => {
                self.text(s, text, 15.0, POINT, pal.ink_main, l0, mid, left, start);
            }
        }
    }

    fn bar(&self, s: &gtk::Snapshot, r: &Rect, value: f64, track: Rgb, fill: Rgb) {
        if r.w <= 0.0 {
            return;
        }
        let radius = (r.h / 2.0) as f32;
        s.push_rounded_clip(&gsk::RoundedRect::from_rect(r.graphene(), radius));
        s.append_color(&track.rgba(), &r.graphene());
        let filled = Rect { w: r.w * value.clamp(0.0, 1.0), ..*r };
        s.append_color(&fill.rgba(), &filled.graphene());
        s.pop();
    }

    fn draw_art(&self, s: &gtk::Snapshot, r: &Rect, radius: f64, scale: f64) {
        let pal = self.app.palette.get();
        let (w, h) = (r.w * scale, r.h * scale);
        let rect = graphene::Rect::new((r.x + (r.w - w) / 2.0) as f32, (r.y + (r.h - h) / 2.0) as f32, w as f32, h as f32);
        s.push_rounded_clip(&gsk::RoundedRect::from_rect(rect, (radius * scale) as f32));
        s.append_color(&pal.raised.rgba(), &rect);
        let art = self.app.media.borrow().art.clone();
        match art {
            Some(texture) => {
                let (tw, th) = (texture.width() as f64, texture.height() as f64);
                let k = (w / tw).max(h / th);
                let (dw, dh) = (tw * k, th * k);
                let bounds = graphene::Rect::new(
                    (rect.x() as f64 + (w - dw) / 2.0) as f32,
                    (rect.y() as f64 + (h - dh) / 2.0) as f32,
                    dw as f32,
                    dh as f32,
                );
                s.append_scaled_texture(&texture, gsk::ScalingFilter::Trilinear, &bounds);
            }
            None => {
                let px = if r.w > 40.0 { 36.0 } else { 14.0 };
                self.icon(s, "music_note", px * scale, true, pal.ink_meta, r.x + r.w / 2.0, r.y + r.h / 2.0);
            }
        }
        s.pop();
    }

    fn player(&self, now: i64) -> Option<Player> {
        if !self.app.media.borrow().present(now) {
            return None;
        }
        let top = self.header();
        let x0 = CX - SHEET_W / 2.0 + 20.0;
        let x1 = CX + SHEET_W / 2.0 - 20.0;
        let mid = top + 6.0 + 64.0;
        let art = Rect { x: x0, y: mid - 45.0, w: 90.0, h: 90.0 };
        let info_x = x0 + 106.0;
        let info_w = x1 - info_x;
        let block = mid - 57.0;
        let controls_mid = block + 20.0 + 18.0 + 36.0 + 20.0;
        let ccx = info_x + info_w / 2.0;
        let button = |cx: f64| Rect { x: cx - 18.0, y: controls_mid - 18.0, w: 36.0, h: 36.0 };
        Some(Player {
            art,
            info_x,
            info_w,
            top: block,
            controls: [
                (button(ccx - 44.0), Action::Previous),
                (button(ccx), Action::Toggle),
                (button(ccx + 44.0), Action::Next),
            ],
        })
    }

    fn draw_sheet(&self, s: &gtk::Snapshot, now: i64) {
        let pal = self.app.palette.get();
        let Some(p) = self.player(now) else {
            self.text(s, "Nothing playing", 15.0, REST, pal.ink_meta, CX - 200.0, self.header() + 36.0, 400.0, pango::Alignment::Center);
            return;
        };
        let media = self.app.media.borrow().clone();
        self.draw_art(s, &p.art, 13.0, self.art_scale.value(now));
        let start = pango::Alignment::Left;
        self.text(s, &media.title, 16.0, Face::Text(600), pal.ink_main, p.info_x, p.top + 10.0, p.info_w, start);
        self.text(s, &media.artist, 14.0, REST, pal.ink_meta, p.info_x, p.top + 29.0, p.info_w, start);
        let slider = p.top + 38.0;
        let elapsed = media.elapsed(now);
        let progress = if media.length > 0.0 { elapsed / media.length } else { 0.0 };
        self.bar(s, &Rect { x: p.info_x, y: slider + 9.0, w: p.info_w, h: 4.0 }, progress, pal.ink_main.alpha(0.15), pal.ink_main);
        if media.length > 0.0 {
            let small = |t: f64| format!("{}:{:02}", (t / 60.0) as u64, (t % 60.0) as u64);
            let face = Face::Number(450);
            self.text(s, &small(elapsed), 12.0, face, pal.ink_meta, p.info_x, slider + 25.0, p.info_w / 2.0, start);
            self.text(s, &small(media.length), 12.0, face, pal.ink_meta, p.info_x + p.info_w / 2.0, slider + 25.0, p.info_w / 2.0, pango::Alignment::Right);
        }
        for (r, action) in &p.controls {
            let (name, px) = match action {
                Action::Previous => ("skip_previous", 22.0),
                Action::Toggle => (if media.playing { "pause" } else { "play_arrow" }, 30.0),
                Action::Next => ("skip_next", 22.0),
            };
            self.icon(s, name, px, true, pal.ink_main, r.x + r.w / 2.0, r.y + r.h / 2.0);
        }
    }
}

mod imp {
    use super::View;
    use gtk::subclass::prelude::*;
    use gtk::{glib, prelude::*};
    use std::cell::RefCell;
    use std::rc::Weak;

    #[derive(Default)]
    pub struct Canvas {
        pub(super) view: RefCell<Weak<RefCell<View>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Canvas {
        const NAME: &'static str = "DigitCanvas";
        type Type = super::Canvas;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Canvas {}

    impl WidgetImpl for Canvas {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let Some(view) = self.view.borrow().upgrade() else { return };
            let Ok(v) = view.try_borrow() else {
                self.obj().queue_draw();
                return;
            };
            v.draw(&self.obj(), snapshot);
        }
    }
}

glib::wrapper! {
    pub struct Canvas(ObjectSubclass<imp::Canvas>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for Canvas {
    fn default() -> Self {
        glib::Object::new()
    }
}
