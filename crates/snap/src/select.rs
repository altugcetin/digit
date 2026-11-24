use crate::capture::Shot;
use crate::moving::{Loupe, Moving};
use crate::windows::Rect;
use crate::{draw, out};
use gtk::{cairo, gdk, glib, graphene, gsk, pango, prelude::*, subclass::prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub struct Screen {
    pub name: String,
    /// Where it sits in the compositor's layout of all the screens, in logical pixels.
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub base: draw::Base,
    wins: Vec<Rect>,
    window: gtk::Window,
    area: Canvas,
    layer: gtk::Overlay,
    texture: gdk::Texture,
    label: pango::Layout,
    loupe_label: pango::Layout,
    bar: gtk::Box,
    hint: gtk::Label,
    moving: RefCell<Moving>,
}

mod canvas {
    use super::*;

    #[derive(Default)]
    pub struct Canvas {
        pub(super) overlay: RefCell<std::rc::Weak<Overlay>>,
        pub(super) screen: Cell<usize>,
        pub(super) pending: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Canvas {
        const NAME: &'static str = "DigitSnapSelection";
        type Type = super::Canvas;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Canvas {}

    impl WidgetImpl for Canvas {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            if let Some(o) = self.overlay.borrow().upgrade() {
                o.paint(self.screen.get(), snapshot);
            }
        }
    }
}

glib::wrapper! {
    pub struct Canvas(ObjectSubclass<canvas::Canvas>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

/// A drag on a screen, in the layout's coordinates: across the screens, the pointer stays with
/// the screen it was pressed on, which goes on hearing it beyond its edges.
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
    /// The selection in the layout's coordinates; it can reach across screens.
    sel: Cell<Option<Rect>>,
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

/// Where `a` and `b` overlap, if they do.
fn meet(a: Rect, b: Rect) -> Option<Rect> {
    let (x0, y0) = (a.0.max(b.0), a.1.max(b.1));
    let (x1, y1) = ((a.0 + a.2).min(b.0 + b.2), (a.1 + a.3).min(b.1 + b.3));
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1 - x0, y1 - y0))
}

/// What all of `rects` cover together, gaps between them included.
fn span(rects: &[Rect]) -> Rect {
    let Some(first) = rects.first() else { return (0.0, 0.0, 1.0, 1.0) };
    let (mut x0, mut y0, mut x1, mut y1) = (first.0, first.1, first.0 + first.2, first.1 + first.3);
    for r in rects {
        (x0, y0, x1, y1) = (x0.min(r.0), y0.min(r.1), x1.max(r.0 + r.2), y1.max(r.1 + r.3));
    }
    (x0, y0, x1 - x0, y1 - y0)
}

/// Where each screen named in `names`, `sizes` big, sits in the layout: as the compositor has
/// them, or side by side when it doesn't say.
fn layout(names: &[String], sizes: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut next = 0.0;
    names.iter().zip(sizes).map(|(n, (w, _))| {
        let at = kit::screens::by_name(n).map(|m| m.geometry()).map(|g| (g.x() as f64, g.y() as f64));
        let at = at.unwrap_or((next, 0.0));
        next = next.max(at.0 + w);
        at
    }).collect()
}

fn handles(r: Rect) -> [(f64, f64); 8] {
    let (x, y, w, h) = r;
    [(x, y), (x + w / 2.0, y), (x + w, y), (x + w, y + h / 2.0), (x + w, y + h), (x + w / 2.0, y + h), (x, y + h), (x, y + h / 2.0)]
}

// All arguments are in the owning monitor's local logical coordinates. Positioning
// must never become part of the bar's measured size (as widget margins do).
fn place(bar_size: (f64, f64), selection: Rect, monitor_size: (f64, f64)) -> (f64, f64) {
    let (bw, bh) = bar_size;
    let (x, y, w, h) = selection;
    let (mw, mh) = monitor_size;
    let inset = 8.0;
    let gap = 12.0;
    let below = y + h + gap;
    let above = y - bh - gap;
    let by = if below + bh <= mh - inset {
        below
    } else if above >= inset {
        above
    } else {
        y + h - bh - gap
    };
    // On an output smaller than the bar, keep its origin on screen without
    // panicking on inverted clamp bounds. Normal outputs retain the full inset.
    let clamp = |pos: f64, size: f64, extent: f64| {
        let end = (extent - size - inset).max(0.0);
        pos.clamp(inset.min(end), end)
    };
    (clamp(x + (w - bw) / 2.0, bw, mw), clamp(by, bh, mh))
}

const PICK: &str = "Click to copy a color  ·  Enter picks under the pointer  ·  Esc cancels";

pub fn open(app: &gtk::Application, shots: Vec<Shot>, pick: bool) {
    let rects = if pick { vec![Vec::new(); shots.len()] } else { crate::windows::rects(&shots) };
    let prefs = out::prefs();
    let ov = Rc::new(Overlay { app: app.clone(), screens: RefCell::new(Vec::new()), sel: Cell::new(None), drag: Cell::new(None), hover: Cell::new(None), prefs, closed: Cell::new(false), pick });
    let mut screens = Vec::new();
    let names: Vec<String> = shots.iter().map(|s| s.name.clone()).collect();
    let sizes: Vec<(f64, f64)> = shots.iter().map(|s| (s.w as f64, s.h as f64)).collect();
    let origins = layout(&names, &sizes);
    for ((shot, wins), (ox, oy)) in shots.into_iter().zip(rects).zip(origins) {
        let Some(base) = draw::Base::new(shot.data, shot.pw, shot.ph, shot.stride) else { continue };
        // A failed image must not leave subsequent callbacks addressing the wrong screen.
        let i = screens.len();
        let texture = gdk::MemoryTexture::new(base.w, base.h, gdk::MemoryFormat::B8g8r8a8Premultiplied, &base.pixels, base.stride).upcast();
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
        window.set_monitor(kit::screens::by_name(&shot.name).as_ref());
        let area: Canvas = glib::Object::new();
        *area.imp().overlay.borrow_mut() = Rc::downgrade(&ov);
        area.imp().screen.set(i);
        area.set_hexpand(true);
        area.set_vexpand(true);
        area.set_cursor_from_name(Some("crosshair"));
        let layer = gtk::Overlay::new();
        layer.set_child(Some(&area));
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        bar.add_css_class("snapbar");
        if kit::motion::on(kit::motion::Part::Screenshot) {
            bar.add_css_class("moves");
        }
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
        let weak = Rc::downgrade(&ov);
        let bar_child = bar.clone();
        layer.connect_get_child_position(move |layer, child| {
            if child != bar_child.upcast_ref::<gtk::Widget>() {
                return None;
            }
            let o = weak.upgrade()?;
            if o.bar_screen()? != i {
                return None;
            }
            let r = o.part(i)?;
            // Run during allocation: CSS has resolved and the overlay has its
            // actual monitor size. No physical screenshot pixels or desktop offsets.
            let bw = child.measure(gtk::Orientation::Horizontal, -1).1;
            let bh = child.measure(gtk::Orientation::Vertical, bw).1;
            let (x, y) = place((bw as f64, bh as f64), r, (layer.width() as f64, layer.height() as f64));
            Some(gdk::Rectangle::new(x.round() as i32, y.round() as i32, bw, bh))
        });
        let hint = kit::ui::text(if pick { PICK } else { HINT }, "snaphint");
        hint.set_halign(gtk::Align::Center);
        hint.set_valign(gtk::Align::End);
        hint.set_margin_bottom(40);
        hint.set_can_target(false);
        if kit::motion::on(kit::motion::Part::Screenshot) {
            hint.add_css_class("moves");
        }
        come(&hint);
        layer.add_overlay(&hint);
        window.set_child(Some(&layer));
        let label = area.create_pango_layout(None);
        label.set_font_description(Some(&pango::FontDescription::from_string("Rubik Medium 12px")));
        let loupe_label = area.create_pango_layout(None);
        loupe_label.set_font_description(Some(&pango::FontDescription::from_string("Rubik Medium 11px")));
        wire(&ov, i, &window, &area);
        screens.push(Screen { name: shot.name, x: ox, y: oy, w: shot.w as f64, h: shot.h as f64, base, wins, window, area, layer, texture, label, loupe_label, bar, hint, moving: RefCell::default() });
    }
    *ov.screens.borrow_mut() = screens;
    let windows: Vec<gtk::Window> = ov.screens.borrow().iter().map(|s| s.window.clone()).collect();
    ACTIVE.with(|a| *a.borrow_mut() = Some(ov.clone()));
    for w in windows {
        w.present();
    }
}

/// Shows `w`, coming in from a little below if it wasn't there.
fn come(w: &impl IsA<gtk::Widget>) {
    w.set_visible(true);
    // Once it has been drawn without it, so there's something to come in from.
    let frames = Cell::new(0);
    w.add_tick_callback(move |w, _| {
        frames.set(frames.get() + 1);
        if frames.get() < 3 && w.is_visible() {
            return glib::ControlFlow::Continue;
        }
        if w.is_visible() {
            w.add_css_class("in");
        }
        glib::ControlFlow::Break
    });
}

fn go(w: &impl IsA<gtk::Widget>) {
    w.set_visible(false);
    w.remove_css_class("in");
}

fn wire(ov: &Rc<Overlay>, i: usize, window: &gtk::Window, area: &Canvas) {
    let motion = gtk::EventControllerMotion::new();
    let weak = Rc::downgrade(ov);
    motion.connect_motion(move |_, x, y| {
        if let Some(o) = weak.upgrade() {
            let prev = o.hover.replace(Some((i, x, y)));
            if let Some((p, _, _)) = prev && p != i {
                o.redraw_hover(p);
            }
            o.redraw_hover(i);
        }
    });
    let weak = Rc::downgrade(ov);
    motion.connect_leave(move |_| {
        if let Some(o) = weak.upgrade() && o.hover.get().is_some_and(|h| h.0 == i) {
            o.hover.set(None);
            o.redraw_hover(i);
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
            o.update(i, sx, sy, dx, dy);
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
        } else if n == 2 && o.sel.get().is_some_and(|r| {
            let (x, y) = o.global(i, x, y);
            x >= r.0 && x <= r.0 + r.2 && y >= r.1 && y <= r.1 + r.3
        }) {
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
        if let Ok(s) = self.screens.try_borrow() && let Some(sc) = s.get(i) {
            // Coalesce motion + gesture updates into one redraw at the next
            // frame, and stop ticking immediately when there is no more work.
            if !sc.area.imp().pending.replace(true) {
                sc.area.add_tick_callback(|area, _| {
                    area.imp().pending.set(false);
                    area.queue_draw();
                    glib::ControlFlow::Break
                });
            }
        }
    }

    fn redraw_hover(&self, i: usize) {
        // Hover only affects the window preview/loupe before selection or while
        // creating it. Moving over a completed selection changes no pixels.
        if self.part(i).is_none()
            || self.drag.get().is_some_and(|(di, d)| di == i && matches!(d, Drag::Create(..)))
        {
            self.redraw(i);
        }
    }

    fn select(&self, selection: Option<Rect>) {
        let previous = self.sel.replace(selection);
        if previous == selection {
            return;
        }
        // A selection can reach across screens; there are only a few of them.
        for i in 0..self.screens.borrow().len() {
            self.redraw(i);
        }
    }

    fn size(&self, i: usize) -> (f64, f64) {
        self.screens.borrow().get(i).map(|s| (s.w, s.h)).unwrap_or((1.0, 1.0))
    }

    /// Where the screen `i` sits in the layout.
    fn screen(&self, i: usize) -> Rect {
        self.screens.borrow().get(i).map(|s| (s.x, s.y, s.w, s.h)).unwrap_or((0.0, 0.0, 1.0, 1.0))
    }

    /// The point `x`, `y` on the screen `i`, in the layout.
    fn global(&self, i: usize, x: f64, y: f64) -> (f64, f64) {
        let r = self.screen(i);
        (r.0 + x, r.1 + y)
    }

    /// What all the screens cover, which a selection stays within.
    fn area(&self) -> Rect {
        let rects: Vec<Rect> = self.screens.borrow().iter().map(|s| (s.x, s.y, s.w, s.h)).collect();
        span(&rects)
    }

    /// The part of the selection on the screen `i`, in its own coordinates.
    fn part(&self, i: usize) -> Option<Rect> {
        let screen = self.screen(i);
        let r = meet(self.sel.get()?, screen)?;
        Some((r.0 - screen.0, r.1 - screen.1, r.2, r.3))
    }

    /// The screen with the most of the selection, where its buttons go.
    fn bar_screen(&self) -> Option<usize> {
        let n = self.screens.borrow().len();
        (0..n).filter_map(|i| self.part(i).map(|r| (i, r.2 * r.3))).max_by(|a, b| a.1.total_cmp(&b.1)).map(|(i, _)| i)
    }

    /// Image pixels per logical pixel in the screenshot of the selection: the sharpest of the
    /// screens it covers.
    fn sharpest(&self) -> f64 {
        let s = self.screens.borrow();
        (0..s.len()).filter(|i| self.part(*i).is_some()).map(|i| s[i].base.w as f64 / s[i].w).fold(0.0, f64::max).max(1.0)
    }

    fn window_at(&self, i: usize, x: f64, y: f64) -> Option<Rect> {
        let s = self.screens.borrow();
        let sc = s.get(i)?;
        let r = sc.wins.iter().rev().find(|r| x >= r.0 && x <= r.0 + r.2 && y >= r.1 && y <= r.1 + r.3)?;
        // Windows may extend beyond an output. Keep selection geometry within
        // its monitor, including later move/resize clamp bounds.
        let (x0, y0) = (r.0.clamp(0.0, sc.w), r.1.clamp(0.0, sc.h));
        let (x1, y1) = ((r.0 + r.2).clamp(0.0, sc.w), (r.1 + r.3).clamp(0.0, sc.h));
        Some((x0, y0, x1 - x0, y1 - y0))
    }

    fn begin(&self, i: usize, x: f64, y: f64) {
        self.hide_bars();
        let (x, y) = self.global(i, x, y);
        if let Some(r) = self.sel.get() {
            if let Some(h) = handles(r).iter().position(|p| (p.0 - x).abs() < 10.0 && (p.1 - y).abs() < 10.0) {
                self.drag.set(Some((i, Drag::Resize(h as u8, r))));
                return;
            }
            if x >= r.0 && x <= r.0 + r.2 && y >= r.1 && y <= r.1 + r.3 {
                self.drag.set(Some((i, Drag::Move(r))));
                return;
            }
        }
        self.drag.set(Some((i, Drag::Create(x, y))));
        self.redraw(i);
    }

    fn update(&self, i: usize, sx: f64, sy: f64, dx: f64, dy: f64) {
        let Some((di, d)) = self.drag.get() else { return };
        if di != i {
            return;
        }
        // Past this screen's edges, onto the others: within all of them.
        let (ax, ay, aw, ah) = self.area();
        let (gx, gy) = self.global(i, sx + dx, sy + dy);
        let cur = (gx.clamp(ax, ax + aw), gy.clamp(ay, ay + ah));
        let r = match d {
            Drag::Create(x, y) => {
                if dx.abs() < 3.0 && dy.abs() < 3.0 {
                    return;
                }
                norm((x, y), cur)
            }
            Drag::Move(r) => ((r.0 + dx).clamp(ax, ax + aw - r.2), (r.1 + dy).clamp(ay, ay + ah - r.3), r.2, r.3),
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
        self.select(Some(r));
    }

    fn end(&self, i: usize, x: f64, y: f64) {
        let d = self.drag.take();
        let (gx, gy) = self.global(i, x, y);
        if let Some((_, Drag::Create(sx, sy))) = d && (gx - sx).abs() < 4.0 && (gy - sy).abs() < 4.0 {
            // A click: the window under it, or the whole screen.
            let (w, h) = self.size(i);
            let r = self.window_at(i, x, y).unwrap_or((0.0, 0.0, w, h));
            let (ox, oy) = self.global(i, 0.0, 0.0);
            self.select(Some((r.0 + ox, r.1 + oy, r.2, r.3)));
        }
        self.redraw(i);
        self.show_bar();
    }

    fn clear(&self) {
        self.select(None);
        self.hide_bars();
    }

    fn hide_bars(&self) {
        if let Ok(s) = self.screens.try_borrow() {
            for sc in s.iter() {
                go(&sc.bar);
                if self.sel.get().is_none() { come(&sc.hint) } else { go(&sc.hint) }
            }
        }
    }

    fn show_bar(&self) {
        let Some(i) = self.bar_screen() else { return };
        let s = self.screens.borrow();
        for (k, sc) in s.iter().enumerate() {
            go(&sc.hint);
            if k == i { come(&sc.bar) } else { go(&sc.bar) }
        }
        let Some(sc) = s.get(i) else { return };
        sc.layer.queue_allocate();
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
                let Some(r) = self.sel.get() else { return true };
                let step = if shift { 10.0 } else { 1.0 };
                let (dx, dy) = match k {
                    gdk::Key::Left => (-step, 0.0),
                    gdk::Key::Right => (step, 0.0),
                    gdk::Key::Up => (0.0, -step),
                    _ => (0.0, step),
                };
                let (ax, ay, aw, ah) = self.area();
                let r = if ctrl { (r.0, r.1, (r.2 + dx).clamp(1.0, ax + aw - r.0), (r.3 + dy).clamp(1.0, ay + ah - r.1)) } else { ((r.0 + dx).clamp(ax, ax + aw - r.2), (r.1 + dy).clamp(ay, ay + ah - r.3), r.2, r.3) };
                self.select(Some(r));
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
        self.select(Some(self.screen(i)));
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
        out::copy_text(&hex, &self.app);
        let c = kit::color::Color { r: rgb.0 as f64 / 255.0, g: rgb.1 as f64 / 255.0, b: rgb.2 as f64 / 255.0, a: 1.0 };
        let body: Vec<String> = c.formats().into_iter().skip(1).take(3).map(|(_, v)| v).collect();
        let swatch = out::swatch(rgb);
        out::notify(&format!("{hex} copied"), &match kit::keybinds::shown("clipboard") {
                Some(k) => format!("{}\n{k} shows every format", body.join("  ·  ")),
                None => body.join("  ·  "),
            }, swatch.as_deref());
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
        let Some(r) = self.sel.get() else { return };
        if r.2 < 2.0 || r.3 < 2.0 {
            return;
        }
        let Some(main) = self.bar_screen() else { return };
        let Some(surface) = self.picture(r) else { return };
        // Edited or pinned on the screen with the most of it, where its part of it sits.
        let (name, origin) = {
            let s = self.screens.borrow();
            let Some(sc) = s.get(main) else { return };
            let at = self.part(main).unwrap_or((0.0, 0.0, r.2, r.3));
            (sc.name.clone(), (at.0, at.1, r.2, r.3))
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
                        Act::Text => out::ocr(&tex, &app),
                        Act::Edit => {}
                    }
                }
            }
        });
    }

    /// The screenshot of `r`, in the layout's coordinates: one screen's pixels as they are, or
    /// the parts of each screen it covers put together, at the sharpest one's scale. What lies
    /// between screens stays clear.
    fn picture(&self, r: Rect) -> Option<cairo::ImageSurface> {
        let s = self.screens.borrow();
        let parts: Vec<usize> = (0..s.len()).filter(|i| self.part(*i).is_some()).collect();
        if let [i] = parts[..] {
            let sc = &s[i];
            let local = (r.0 - sc.x, r.1 - sc.y, r.2, r.3);
            let k = sc.base.w as f64 / sc.w;
            let (px, py) = ((local.0 * k).round(), (local.1 * k).round());
            let (pw, ph) = (((local.0 + local.2) * k).round() - px, ((local.1 + local.3) * k).round() - py);
            let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, pw.max(1.0) as i32, ph.max(1.0) as i32).ok()?;
            let cr = cairo::Context::new(&surface).ok()?;
            let _ = cr.set_source_surface(&sc.base.surface, -px, -py);
            let _ = cr.paint();
            drop(cr);
            return Some(surface);
        }
        let k = self.sharpest();
        let (ox, oy) = ((r.0 * k).round(), (r.1 * k).round());
        let (pw, ph) = (((r.0 + r.2) * k).round() - ox, ((r.1 + r.3) * k).round() - oy);
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, pw.max(1.0) as i32, ph.max(1.0) as i32).ok()?;
        let cr = cairo::Context::new(&surface).ok()?;
        for i in parts {
            let sc = &s[i];
            let Some(seen) = meet(r, (sc.x, sc.y, sc.w, sc.h)) else { continue };
            let own = sc.base.w as f64 / sc.w;
            cr.save().ok()?;
            cr.rectangle((seen.0 * k).round() - ox, (seen.1 * k).round() - oy, (seen.2 * k).round(), (seen.3 * k).round());
            cr.clip();
            // Whole pixels where the screen's scale is the picture's, so it stays as sharp.
            cr.translate((sc.x * k).round() - ox, (sc.y * k).round() - oy);
            cr.scale(k / own, k / own);
            let _ = cr.set_source_surface(&sc.base.surface, 0.0, 0.0);
            if (k - own).abs() > 1e-6 {
                cr.source().set_filter(cairo::Filter::Good);
            }
            let _ = cr.paint();
            cr.restore().ok()?;
        }
        drop(cr);
        Some(surface)
    }

    fn paint(&self, i: usize, snapshot: &gtk::Snapshot) {
        let Ok(s) = self.screens.try_borrow() else { return };
        let Some(sc) = s.get(i) else { return };
        let (w, h) = (sc.w, sc.h);
        let k = sc.base.w as f64 / w;
        let now = sc.area.frame_clock().map_or_else(glib::monotonic_time, |c| c.frame_time());
        let mut moving = sc.moving.borrow_mut();
        // The frozen screenshot is uploaded once, then reused by both the
        // background and magnifier. No full-screen Cairo raster/upload per frame.
        snapshot.append_texture(&sc.texture, &bounds((0.0, 0.0, w, h)));
        // The selection where this screen sits: what of it is on here, and all of it, whose
        // edges and handles past this screen's edges fall off it.
        let part = self.sel.get().and_then(|r| meet(r, (sc.x, sc.y, w, h))).map(|r| (r.0 - sc.x, r.1 - sc.y, r.2, r.3));
        let sel = self.sel.get().filter(|_| part.is_some()).map(|r| (r.0 - sc.x, r.1 - sc.y, r.2, r.3));
        let dragging = self.drag.get().is_some_and(|(di, d)| di == i && matches!(d, Drag::Create(..)));
        // The drag may have started on another screen.
        moving.select(part, self.drag.get().is_some(), self.drag.get().is_some_and(|(_, d)| matches!(d, Drag::Create(..))), now);
        if !self.pick {
            let dark = moving.dark(now);
            let shade = |k: f64| gdk::RGBA::new(0.0, 0.0, 0.0, (dark * k) as f32);
            let dim = shade(1.0);
            if let Some((x, y, sw, sh)) = part {
                for r in [(0.0, 0.0, w, y), (0.0, y, x, sh), (x + sw, y, w - x - sw, sh), (0.0, y + sh, w, h - y - sh)] {
                    if r.2 > 0.0 && r.3 > 0.0 {
                        snapshot.append_color(&dim, &bounds(r));
                    }
                }
                let over = moving.over(now);
                if over > 0.01 {
                    snapshot.append_color(&shade(over), &bounds((x, y, sw, sh)));
                }
            } else if let Some(((x, y, sw, sh), back)) = moving.dropped(now) {
                for (r, k) in [((0.0, 0.0, w, y), 1.0), ((0.0, y, x, sh), 1.0), ((x + sw, y, w - x - sw, sh), 1.0), ((0.0, y + sh, w, h - y - sh), 1.0), ((x, y, sw, sh), back)] {
                    if r.2 > 0.0 && r.3 > 0.0 {
                        snapshot.append_color(&shade(k), &bounds(r));
                    }
                }
            } else {
                snapshot.append_color(&dim, &bounds((0.0, 0.0, w, h)));
            }
        }
        let hover = self.hover.get().filter(|(hi, _, _)| *hi == i);
        // It stays lit while it's pressed: until there's a selection, that is a click on it.
        let window = hover.filter(|_| sel.is_none()).and_then(|(_, x, y)| self.window_at(i, x, y));
        let (r, light) = moving.light(window, now);
        if light > 0.01 {
            // GSK draws the preview without a monitor-sized Cairo node.
            let light = light as f32;
            let shape = gsk::RoundedRect::from_rect(bounds((r.0 - 1.0, r.1 - 1.0, r.2 + 2.0, r.3 + 2.0)), 13.0);
            fill(snapshot, &gsk::RoundedRect::from_rect(bounds(r), 12.0), gdk::RGBA::new(1.0, 1.0, 1.0, 0.10 * light));
            snapshot.append_border(&shape, &[2.0; 4], &[gdk::RGBA::new(1.0, 1.0, 1.0, 0.9 * light); 4]);
        }
        if let Some(r) = sel {
            // Cairo strokes are centred on the path; GSK borders are inset.
            let edge = (r.0.round() - 0.5, r.1.round() - 0.5, r.2.round() + 1.0, r.3.round() + 1.0);
            let shape = gsk::RoundedRect::from_rect(bounds(edge), 0.0);
            snapshot.append_border(&shape, &[1.5; 4], &[gdk::RGBA::new(1.0, 1.0, 1.0, 0.95); 4]);
            let grip = moving.grips(now);
            if grip > 0.01 {
                for (hx, hy) in handles(r) {
                    // Small Cairo nodes preserve the exact existing circular
                    // handles without allocating a surface spanning the selection.
                    let cr = snapshot.append_cairo(&bounds((hx - 6.0, hy - 6.0, 12.0, 12.0)));
                    cr.arc(hx, hy, 5.0 * grip, 0.0, std::f64::consts::TAU);
                    cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
                    let _ = cr.fill_preserve();
                    cr.set_source_rgba(0.0, 0.0, 0.0, 0.5);
                    cr.set_line_width(1.0);
                    let _ = cr.stroke();
                }
            }
            // The size, by its top left corner, on the screen that has it; else by the part of
            // it the buttons are on.
            let k = self.sharpest();
            let corner_here = r.0 >= 0.0 && r.0 < w && r.1 >= 0.0 && r.1 < h;
            let corner_nowhere = !s.iter().any(|o| {
                let (cx, cy) = (r.0 + sc.x, r.1 + sc.y);
                cx >= o.x && cx < o.x + o.w && cy >= o.y && cy < o.y + o.h
            });
            if corner_here || (corner_nowhere && self.bar_screen() == Some(i)) {
                let at = if corner_here { r } else { part.unwrap_or(r) };
                let label = format!("{} × {}", (r.2 * k).round(), (r.3 * k).round());
                set_text(&sc.label, &label);
                pill(snapshot, &sc.label, at.0 + 2.0, if at.1 > 34.0 { at.1 - 30.0 } else { at.1 + 6.0 }, w);
            }
        }
        let at = hover.filter(|_| (self.prefs.loupe || self.pick) && (sel.is_none() || dragging)).map(|(_, x, y)| (x, y));
        if let Some(l) = moving.loupe(at, |x, y| loupe_sides(sc, x, y), now) {
            loupe(snapshot, sc, &l, k, self.pick);
        }
        if moving.busy(now) {
            self.redraw(i);
        }
    }
}

fn bounds(r: Rect) -> graphene::Rect {
    graphene::Rect::new(r.0 as f32, r.1 as f32, r.2 as f32, r.3 as f32)
}

fn fill(snapshot: &gtk::Snapshot, shape: &gsk::RoundedRect, color: gdk::RGBA) {
    snapshot.push_rounded_clip(shape);
    snapshot.append_color(&color, shape.bounds());
    snapshot.pop();
}

fn set_text(layout: &pango::Layout, text: &str) {
    if layout.text().as_str() != text {
        layout.set_text(text);
    }
}

fn text(snapshot: &gtk::Snapshot, layout: &pango::Layout, x: f64, y: f64) {
    snapshot.save();
    snapshot.translate(&graphene::Point::new(x as f32, y as f32));
    snapshot.append_layout(layout, &gdk::RGBA::WHITE);
    snapshot.restore();
}

fn pill(snapshot: &gtk::Snapshot, layout: &pango::Layout, x: f64, y: f64, maxw: f64) {
    let (tw, th) = layout.pixel_size();
    let (bw, bh) = (tw as f64 + 16.0, th as f64 + 8.0);
    let x = x.min(maxw - bw - 4.0).max(4.0);
    let shape = gsk::RoundedRect::from_rect(bounds((x, y, bw, bh)), (bh / 2.0) as f32);
    fill(snapshot, &shape, gdk::RGBA::new(0.0, 0.0, 0.0, 0.78));
    text(snapshot, layout, x + 8.0, y + 4.0);
}

const LOUPE_R: f64 = 62.0;

/// Whether the magnifier goes to the pointer's left and above it, for want of room.
fn loupe_sides(sc: &Screen, x: f64, y: f64) -> (bool, bool) {
    (x + 26.0 + LOUPE_R * 2.0 > sc.w - 4.0, y + 26.0 + LOUPE_R * 2.0 + 30.0 > sc.h - 4.0)
}

fn loupe(snapshot: &gtk::Snapshot, sc: &Screen, l: &Loupe, k: f64, color: bool) {
    let r = LOUPE_R;
    let zoom = 9.0;
    let (x, y) = l.at;
    let lx = x + 26.0 - l.side.0 * (52.0 + r * 2.0);
    let ly = y + 26.0 - l.side.1 * (52.0 + r * 2.0 + 30.0);
    let (mx, my) = ((lx + r) as f32, (ly + r) as f32);
    let size = (0.85 + 0.15 * l.shown) as f32;
    snapshot.save();
    snapshot.translate(&graphene::Point::new(mx, my));
    snapshot.scale(size, size);
    snapshot.translate(&graphene::Point::new(-mx, -my));
    snapshot.push_opacity(l.shown);
    let (px, py) = ((x * k).floor(), (y * k).floor());
    let circle = gsk::RoundedRect::from_rect(bounds((lx, ly, r * 2.0, r * 2.0)), r as f32);
    snapshot.push_rounded_clip(&circle);
    snapshot.append_color(&gdk::RGBA::BLACK, circle.bounds());
    // Only the 124px circle is sampled, directly from the shared texture. Nearest
    // filtering preserves the original nine-logical-pixels-per-image-pixel loupe.
    snapshot.append_scaled_texture(&sc.texture, gsk::ScalingFilter::Nearest,
        &bounds((lx + r - (px + 0.5) * zoom, ly + r - (py + 0.5) * zoom, sc.base.w as f64 * zoom, sc.base.h as f64 * zoom)));
    snapshot.pop();
    let cr = snapshot.append_cairo(&bounds((lx - 1.0, ly - 1.0, r * 2.0 + 2.0, r * 2.0 + 2.0)));
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.9);
    cr.set_line_width(1.0);
    cr.rectangle(lx + r - zoom / 2.0, ly + r - zoom / 2.0, zoom, zoom);
    let _ = cr.stroke();
    cr.arc(lx + r, ly + r, r, 0.0, std::f64::consts::TAU);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.85);
    cr.set_line_width(2.0);
    let _ = cr.stroke();
    let (rr, gg, bb) = sc.base.pixel(px as i32, py as i32);
    let caption = if color { format!("#{rr:02x}{gg:02x}{bb:02x}   {} , {}", px as i32, py as i32) } else { format!("{} , {}", px as i32, py as i32) };
    set_text(&sc.loupe_label, &caption);
    let (tw, th) = sc.loupe_label.pixel_size();
    let (bw, bh) = (tw as f64 + 30.0, th as f64 + 8.0);
    let bx = lx + r - bw / 2.0;
    let by = ly + r * 2.0 + 6.0;
    fill(snapshot, &gsk::RoundedRect::from_rect(bounds((bx, by, bw, bh)), (bh / 2.0) as f32), gdk::RGBA::new(0.0, 0.0, 0.0, 0.8));
    if color {
        let cr = snapshot.append_cairo(&bounds((bx + 6.0, by + bh / 2.0 - 6.0, 12.0, 12.0)));
        cr.arc(bx + 12.0, by + bh / 2.0, 5.0, 0.0, std::f64::consts::TAU);
        cr.set_source_rgb(rr as f64 / 255.0, gg as f64 / 255.0, bb as f64 / 255.0);
        let _ = cr.fill_preserve();
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.7);
        cr.set_line_width(1.0);
        let _ = cr.stroke();
    }
    text(snapshot, &sc.loupe_label, if color { bx + 22.0 } else { bx + 15.0 }, by + 4.0);
    snapshot.pop();
    snapshot.restore();
}

#[cfg(test)]
mod tests {
    use super::{meet, place, span};

    #[test]
    fn a_selection_across_two_screens_has_a_part_on_each() {
        let (left, right) = ((0.0, 0.0, 1920.0, 1080.0), (1920.0, 0.0, 2560.0, 1440.0));
        let sel = (1800.0, 100.0, 400.0, 300.0);
        assert_eq!(meet(sel, left), Some((1800.0, 100.0, 120.0, 300.0)));
        assert_eq!(meet(sel, right), Some((1920.0, 100.0, 280.0, 300.0)));
        assert_eq!(meet((0.0, 0.0, 10.0, 10.0), right), None);
        // Both screens together, the space under the shorter one included.
        assert_eq!(span(&[left, right]), (0.0, 0.0, 4480.0, 1440.0));
    }

    const BAR: (f64, f64) = (300.0, 46.0);
    const MONITOR: (f64, f64) = (1920.0, 1080.0);

    #[test]
    fn bar_is_centered_below_selection() {
        assert_eq!(
            place(BAR, (400.0, 200.0, 500.0, 300.0), MONITOR),
            (500.0, 512.0)
        );
    }

    #[test]
    fn bar_goes_above_when_below_is_full() {
        assert_eq!(
            place(BAR, (400.0, 800.0, 500.0, 260.0), MONITOR),
            (500.0, 742.0)
        );
    }

    #[test]
    fn full_screen_bar_is_inside_near_bottom() {
        assert_eq!(
            place(BAR, (0.0, 0.0, 1920.0, 1080.0), MONITOR),
            (810.0, 1022.0)
        );
    }

    #[test]
    fn exact_gap_and_inset_fit_below_or_above() {
        assert_eq!(
            place(BAR, (400.0, 200.0, 500.0, 814.0), MONITOR),
            (500.0, 1026.0)
        );
        assert_eq!(
            place(BAR, (400.0, 66.0, 500.0, 1014.0), MONITOR),
            (500.0, 8.0)
        );
    }

    #[test]
    fn bar_is_clamped_to_left_right_and_vertical_insets() {
        assert_eq!(place(BAR, (0.0, 100.0, 20.0, 20.0), MONITOR), (8.0, 132.0));
        assert_eq!(
            place(BAR, (1900.0, 100.0, 20.0, 20.0), MONITOR),
            (1612.0, 132.0)
        );
        assert_eq!(
            place(BAR, (0.0, 0.0, 100.0, 50.0), (320.0, 62.0)),
            (8.0, 8.0)
        );
    }

    #[test]
    fn narrow_outputs_do_not_panic_on_inverted_bounds() {
        assert_eq!(
            place(BAR, (0.0, 0.0, 200.0, 40.0), (200.0, 40.0)),
            (0.0, 0.0)
        );
    }

    #[test]
    fn placement_is_stable_across_repeated_handle_adjustments() {
        let selection = (400.0, 200.0, 500.0, 300.0);
        let first = place(BAR, selection, MONITOR);
        for _ in 0..10 {
            assert_eq!(place(BAR, selection, MONITOR), first);
        }
        assert_eq!(
            place(BAR, (400.0, 200.0, 502.0, 301.0), MONITOR),
            (501.0, 513.0)
        );
        assert_eq!(place(BAR, selection, MONITOR), first);
    }

    #[test]
    fn monitors_with_offsets_and_different_scales_use_local_logical_pixels() {
        // Model the boundary contract: pointer/selection and GTK bar dimensions
        // are logical and local, while only the frozen image is physical pixels.
        // A monitor's desktop origin must have no effect on its bar placement.
        for (origin, pixels, scale) in [
            ((0.0, 0.0), (1920.0, 1080.0), 1.0),
            ((1920.0, -200.0), (3840.0, 2160.0), 2.0),
            ((-1920.0, 300.0), (2400.0, 1350.0), 1.25),
        ] {
            let desktop_selection = (origin.0 + 400.0, origin.1 + 200.0, 500.0, 300.0);
            let local = (
                desktop_selection.0 - origin.0,
                desktop_selection.1 - origin.1,
                desktop_selection.2,
                desktop_selection.3,
            );
            let monitor = (pixels.0 / scale, pixels.1 / scale);
            assert_eq!(place(BAR, local, monitor), (500.0, 512.0));
        }
        // A smaller logical second output clamps to its own right edge.
        assert_eq!(
            place(BAR, (1200.0, 600.0, 80.0, 120.0), (1280.0, 720.0)),
            (972.0, 542.0)
        );
    }
}
