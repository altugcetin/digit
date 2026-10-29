use crate::app::App;
use crate::compositor;
use crate::draw::{self, Face};
use crate::providers;
use gtk::subclass::prelude::*;
use gtk::{cairo, gdk, gio, glib, graphene, gsk, pango, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kit::calc;
use kit::fuzzy::{fold, score};
use kit::motion::{self, Tween};
use kit::palette::{Palette, Rgb};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

const PAD: f64 = 40.0;
const WIDTH: f64 = 680.0;
const BAR: f64 = 56.0;
const BUBBLE: f64 = 56.0;
const BUBBLE_GAP: f64 = 10.0;
const GAP: f64 = 10.0;
const INSET: f64 = 8.0;
const PANEL_R: f64 = 24.0;
const ROW: f64 = 44.0;
const HEAD: f64 = 30.0;
const MAX_PANEL: f64 = 448.0;
const SURFACE_W: f64 = WIDTH + 2.0 * PAD;
const SURFACE_H: f64 = PAD + BAR + GAP + MAX_PANEL + PAD;
const PANEL_TOP: f64 = PAD + BAR + GAP;
const FIELD_X: f64 = PAD + 54.0;
const NARROW: f64 = WIDTH - MODES.len() as f64 * (BUBBLE + BUBBLE_GAP);
const SETTLE: motion::Curve = motion::Curve::new(0.16, 1.0, 0.3, 1.0);
const LEAVE: motion::Curve = motion::Curve::new(0.4, 0.0, 0.9, 0.6);

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    All,
    Apps,
    Windows,
    Calc,
    Web,
    Clip,
}

const MODES: [(Mode, &str, &str, &str); 5] = [
    (Mode::Apps, "apps", "Search applications", "Applications"),
    (Mode::Windows, "select_window", "Search windows", "Windows"),
    (Mode::Calc, "calculate", "Calculate", "Calculator"),
    (Mode::Clip, "content_paste", "Search your clipboard", "Clipboard"),
    (Mode::Web, "travel_explore", "Search the web", "Web search"),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Window,
    App,
    Calc,
    Run,
    Web,
    Clip,
    Color,
    Found(u16),
    Scope,
    Settings,
}

impl Kind {
    fn title(self) -> &'static str {
        match self {
            Kind::Window => "Windows",
            Kind::App => "Applications",
            Kind::Calc => "Calculator",
            Kind::Run => "Command",
            Kind::Web => "Web",
            Kind::Clip => "Clipboard",
            Kind::Color => "Color",
            Kind::Found(_) => "Results",
            Kind::Scope => "Search in apps",
            Kind::Settings => "Settings",
        }
    }
}

enum Act {
    App(usize),
    Window(u64),
    Copy(String),
    Open(String),
    Run(String),
    Image(gdk::Texture),
    Clear,
    Found(usize, String),
    More(usize),
    Scope(usize, String),
    Spawn(Vec<String>),
}

const PAGES: [(&str, &str, &str, &str, &str); 6] = [
    ("welcome", "Welcome tour", "Set up Digit", "waving_hand", "welcome tour onboarding setup start intro getting started wizard"),
    ("overview", "About this computer", "System overview", "info", "about system computer hardware memory disk processor gpu overview"),
    ("updates", "Software updates", "Settings", "system_update_alt", "update updates upgrade security patch packages"),
    ("apps", "Default and startup apps", "Settings", "apps", "apps applications default startup autostart browser"),
    ("drivers", "Drivers and firmware", "Settings", "memory", "driver drivers nvidia amd intel gpu firmware graphics"),
    ("desktop", "Desktop and dock", "Settings", "palette", "desktop dock notch bar island call wallpaper theme icons screenshot clipboard window placement tiling free"),
];

const POWER: [(&str, &str, &str, &str); 6] = [
    ("lock", "Lock Screen", "lock", "lock screen away secure"),
    ("sleep", "Sleep", "bedtime", "sleep suspend"),
    ("session", "Restart…", "restart_alt", "restart reboot"),
    ("session", "Shut Down…", "power_settings_new", "shut down shutdown power off poweroff"),
    ("session", "Log Out…", "logout", "log out logout sign out exit quit session"),
    ("night", "Night Light", "nightlight", "night light shift warm blue light"),
];

enum Look {
    Glyph(String),
    Icon(gio::Icon),
    Symbol(&'static str),
    Swatch(kit::color::Color),
    Picture(gdk::Texture),
}

struct Item {
    kind: Kind,
    act: Act,
    title: String,
    detail: String,
    look: Look,
    score: i32,
}

enum Line {
    Head(String),
    Item(usize),
}

impl Line {
    fn height(&self) -> f64 {
        match self {
            Line::Head(_) => HEAD,
            Line::Item(_) => ROW,
        }
    }
}

struct State {
    app: Rc<App>,
    window: gtk::Window,
    canvas: Canvas,
    field: gtk::Text,
    lift: crate::lift::Lift,
    rise: Tween,
    css: gtk::CssProvider,
    open: bool,
    mode: Mode,
    shown: Mode,
    items: Vec<Item>,
    lines: Vec<Line>,
    selected: usize,
    windows: Vec<compositor::Window>,
    fade: Tween,
    panel: Tween,
    bar: Tween,
    bubbles: Tween,
    scroll: Tween,
    glow: [Tween; 5],
    hovered: Option<usize>,
    tip: Tween,
    tip_at: usize,
    tip_wait: u64,
    field_w: i32,
    icons: HashMap<String, gtk::IconPaintable>,
    region: (i32, i32, i32, i32),
    ticking: bool,
    providers: Vec<providers::Provider>,
    scopes: Vec<providers::Scope>,
    found: Vec<providers::Found>,
    found_q: String,
    asking: u64,
    terms: Vec<String>,
    moved: bool,
}

#[derive(Clone)]
pub struct Launcher(Rc<RefCell<State>>);

fn get(app: &Rc<App>) -> Launcher {
    let existing = app.launcher.borrow().clone();
    existing.unwrap_or_else(|| {
        let l = Launcher::new(app);
        *app.launcher.borrow_mut() = Some(l.clone());
        l
    })
}

pub fn toggle(app: &Rc<App>) {
    get(app).toggle();
}

pub fn clipboard(app: &Rc<App>) {
    let l = get(app);
    let open = l.0.borrow().open;
    let mode = l.0.borrow().mode;
    if open && mode == Mode::Clip {
        l.close();
        return;
    }
    if !open {
        l.open();
    }
    let field = l.0.borrow().field.clone();
    field.set_text("");
    l.set_mode(Mode::Clip);
}

pub fn clips_changed(app: &Rc<App>) {
    let Some(l) = app.launcher.borrow().clone() else { return };
    let Ok(s) = l.0.try_borrow() else { return };
    let refresh = s.open && s.shown == Mode::Clip;
    drop(s);
    if refresh {
        l.search();
    }
}

fn ago(at: i64) -> String {
    let secs = (glib::real_time() - at).max(0) / 1_000_000;
    match secs {
        0..=59 => "now".into(),
        60..=3599 => format!("{} min", secs / 60),
        3600..=86_399 => format!("{} h", secs / 3600),
        _ => format!("{} d", secs / 86_400),
    }
}

fn color_items(items: &mut Vec<Item>, c: kit::color::Color, original: &str, top: i32) {
    for (i, (label, value)) in c.formats().into_iter().enumerate() {
        if value.eq_ignore_ascii_case(original.trim()) && i > 0 {
            continue;
        }
        items.push(Item { kind: Kind::Color, act: Act::Copy(value.clone()), title: value, detail: format!("Copy {label}"), look: Look::Swatch(c), score: top - i as i32 });
    }
}

fn clip_items(items: &mut Vec<Item>, query: &str) {
    let q = fold(query);
    let all = crate::clip::history();
    let mut shown_colors = 0;
    for (n, c) in all.iter().enumerate() {
        let score = 10_000 - n as i32 * 10;
        match &c.data {
            crate::clip::Data::Text(text) => {
                if !q.is_empty() && !fold(text).contains(&q) {
                    continue;
                }
                let line: String = text.trim().lines().next().unwrap_or("").chars().take(90).collect();
                let lines = text.lines().count();
                if let Some(color) = kit::color::parse(text) {
                    items.push(Item { kind: Kind::Clip, act: Act::Copy(text.clone()), title: line.clone(), detail: format!("Color · {}", ago(c.at)), look: Look::Swatch(color), score });
                    if shown_colors < 2 {
                        shown_colors += 1;
                        let mut extra = Vec::new();
                        color_items(&mut extra, color, text, score - 1);
                        for mut e in extra.into_iter().skip(1) {
                            e.kind = Kind::Clip;
                            e.score = score - 1;
                            items.push(e);
                        }
                    }
                    continue;
                }
                let detail = if lines > 1 { format!("{lines} lines · {}", ago(c.at)) } else { format!("{} chars · {}", text.chars().count(), ago(c.at)) };
                items.push(Item { kind: Kind::Clip, act: Act::Copy(text.clone()), title: line, detail, look: Look::Symbol("content_paste"), score });
            }
            crate::clip::Data::Image(_) => {
                if !q.is_empty() && !"image picture screenshot".contains(q.as_str()) {
                    continue;
                }
                if let Some(t) = c.texture() {
                    let title = format!("Image {} × {}", t.width(), t.height());
                    items.push(Item { kind: Kind::Clip, act: Act::Image(t.clone()), title, detail: ago(c.at), look: Look::Picture(t), score });
                }
            }
        }
    }
    if all.is_empty() {
        items.push(Item { kind: Kind::Clip, act: Act::Clear, title: "Nothing copied yet".into(), detail: "Copies appear here; kept only in memory".into(), look: Look::Symbol("content_paste_off"), score: 0 });
    } else if q.is_empty() {
        items.push(Item { kind: Kind::Clip, act: Act::Clear, title: "Clear clipboard history".into(), detail: format!("{} items, kept only in memory", all.len()), look: Look::Symbol("delete_sweep"), score: -1 });
    }
}

pub fn show(app: &Rc<App>, text: &str) {
    let l = get(app);
    if !l.0.borrow().open {
        l.open();
    }
    let field = l.0.borrow().field.clone();
    field.set_text(text);
    field.set_position(-1);
}

fn rounded(x: f64, y: f64, w: f64, h: f64, r: f64) -> gsk::RoundedRect {
    gsk::RoundedRect::from_rect(graphene::Rect::new(x as f32, y as f32, w as f32, h as f32), r as f32)
}

fn slab(s: &gtk::Snapshot, shape: &gsk::RoundedRect, fill: Rgb) {
    s.append_outset_shadow(shape, &gdk::RGBA::new(0.0, 0.0, 0.0, 0.34), 0.0, 10.0, 0.0, 30.0);
    s.push_rounded_clip(shape);
    s.append_color(&fill.rgba(), shape.bounds());
    s.pop();
}

impl Launcher {
    fn new(app: &Rc<App>) -> Self {
        let window = gtk::Window::new();
        window.init_layer_shell();
        window.set_layer(Layer::Overlay);
        window.set_namespace(Some("digit-launcher"));
        window.set_anchor(Edge::Top, true);
        window.set_exclusive_zone(-1);
        window.set_keyboard_mode(KeyboardMode::None);
        window.add_css_class("digit");
        window.set_default_size(SURFACE_W as i32, SURFACE_H as i32);
        let canvas: Canvas = glib::Object::new();
        canvas.set_size_request(SURFACE_W as i32, SURFACE_H as i32);
        let field = gtk::Text::new();
        field.add_css_class("digit-field");
        field.set_valign(gtk::Align::Start);
        field.set_halign(gtk::Align::Start);
        field.set_margin_top((PAD + (BAR - 34.0) / 2.0) as i32);
        field.set_margin_start(FIELD_X as i32);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&canvas));
        overlay.add_overlay(&field);
        let lift = crate::lift::Lift::new(&overlay, (SURFACE_W / 2.0, PAD + BAR / 2.0));
        window.set_child(Some(&lift));
        let css = gtk::CssProvider::new();
        if let Some(display) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
        }
        let state = Rc::new(RefCell::new(State {
            app: app.clone(),
            window,
            canvas: canvas.clone(),
            field,
            lift,
            rise: Tween::still(0.0),
            css,
            open: false,
            mode: Mode::All,
            shown: Mode::All,
            items: Vec::new(),
            lines: Vec::new(),
            selected: 0,
            windows: Vec::new(),
            fade: Tween::still(0.0),
            panel: Tween::still(0.0),
            bar: Tween::still(NARROW),
            bubbles: Tween::still(1.0),
            scroll: Tween::still(0.0),
            glow: [Tween::still(0.0); 5],
            hovered: None,
            tip: Tween::still(0.0),
            tip_at: 0,
            tip_wait: 0,
            field_w: 0,
            icons: HashMap::new(),
            region: (0, 0, 0, 0),
            ticking: false,
            providers: Vec::new(),
            scopes: Vec::new(),
            found: Vec::new(),
            found_q: String::new(),
            asking: 0,
            terms: Vec::new(),
            moved: false,
        }));
        *canvas.imp().state.borrow_mut() = Rc::downgrade(&state);
        let launcher = Launcher(state);
        launcher.wire();
        launcher
    }

    fn wire(&self) {
        let (window, field, canvas) = {
            let s = self.0.borrow();
            (s.window.clone(), s.field.clone(), s.canvas.clone())
        };
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(&self.0);
        keys.connect_key_pressed(move |_, key, _, mods| {
            let Some(s) = weak.upgrade() else { return glib::Propagation::Proceed };
            let me = Launcher(s);
            let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
            let empty = me.0.borrow().field.text().is_empty();
            let mode = me.0.borrow().mode;
            match key {
                gdk::Key::Escape if !empty => {
                    let field = me.0.borrow().field.clone();
                    field.set_text("");
                }
                gdk::Key::Escape if mode != Mode::All => me.set_mode(Mode::All),
                gdk::Key::Escape => me.close(),
                gdk::Key::BackSpace if empty && mode != Mode::All => me.set_mode(Mode::All),
                gdk::Key::Return | gdk::Key::KP_Enter => me.activate(None),
                gdk::Key::Down | gdk::Key::Tab => me.step_selection(1),
                gdk::Key::Up | gdk::Key::ISO_Left_Tab => me.step_selection(-1),
                gdk::Key::Page_Down => me.step_selection(6),
                gdk::Key::Page_Up => me.step_selection(-6),
                gdk::Key::n if ctrl => me.step_selection(1),
                gdk::Key::p if ctrl => me.step_selection(-1),
                gdk::Key::_1 | gdk::Key::_2 | gdk::Key::_3 | gdk::Key::_4 | gdk::Key::_5 if ctrl => {
                    let i = (key.to_unicode().and_then(|c| c.to_digit(10)).unwrap_or(1) - 1) as usize;
                    let m = MODES[i].0;
                    me.set_mode(if mode == m { Mode::All } else { m });
                }
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        window.add_controller(keys);
        let weak = Rc::downgrade(&self.0);
        field.connect_changed(move |_| {
            if let Some(s) = weak.upgrade() {
                Launcher(s).search();
            }
        });
        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(&self.0);
        click.connect_released(move |_, _, x, y| {
            let Some(s) = weak.upgrade() else { return };
            let me = Launcher(s);
            if let Some(m) = me.bubble_at(x, y) {
                let mode = me.0.borrow().mode;
                me.set_mode(if mode == m { Mode::All } else { m });
            } else if let Some(line) = me.line_at(x, y) {
                me.activate(Some(line));
            } else if !me.inside(x, y) {
                me.close();
            }
        });
        canvas.add_controller(click);
        let hover = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(&self.0);
        hover.connect_motion(move |_, x, y| {
            let Some(s) = weak.upgrade() else { return };
            let me = Launcher(s);
            let bubble = me.bubble_at(x, y).and_then(|m| MODES.iter().position(|e| e.0 == m));
            me.hover_bubble(bubble);
            if let Some(line) = me.line_at(x, y) {
                let changed = {
                    let mut s = me.0.borrow_mut();
                    let changed = s.selected != line;
                    s.selected = line;
                    changed
                };
                if changed {
                    me.0.borrow().canvas.queue_draw();
                }
            }
        });
        let weak = Rc::downgrade(&self.0);
        hover.connect_leave(move |_| {
            if let Some(s) = weak.upgrade() {
                Launcher(s).hover_bubble(None);
            }
        });
        canvas.add_controller(hover);
        let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        let weak = Rc::downgrade(&self.0);
        wheel.connect_scroll(move |_, _, dy| {
            if let Some(s) = weak.upgrade() {
                Launcher(s).scroll_by(dy * ROW);
            }
            glib::Propagation::Stop
        });
        canvas.add_controller(wheel);
        let weak = Rc::downgrade(&self.0);
        window.connect_is_active_notify(move |w| {
            if !w.is_active() {
                if let Some(s) = weak.upgrade() {
                    let Ok(open) = s.try_borrow().map(|s| s.open) else { return };
                    if open {
                        Launcher(s).close();
                    }
                }
            }
        });
    }

    pub fn toggle(&self) {
        let open = self.0.borrow().open;
        if open { self.close() } else { self.open() }
    }

    fn open(&self) {
        let output = compositor::focused_output();
        let windows = compositor::windows();
        let app = self.0.borrow().app.clone();
        let table = kit::prefs::read();
        let found_in = providers::discover(&app, &table);
        let scopes = providers::scopes(&app, &table);
        {
            let mut s = self.0.borrow_mut();
            s.providers = found_in;
            s.scopes = scopes;
            s.found.clear();
            s.found_q.clear();
            if !s.window.is_visible() {
                if let Some(display) = gdk::Display::default() {
                    let monitors: Vec<gdk::Monitor> = display.monitors().iter::<gdk::Monitor>().filter_map(Result::ok).collect();
                    let monitor = monitors.iter().find(|m| output.as_deref().is_some_and(|o| m.connector().as_deref() == Some(o))).or(monitors.first()).cloned();
                    if let Some(m) = &monitor {
                        let top = (m.geometry().height() as f64 * 0.22 - PAD).max(0.0) as i32;
                        s.window.set_margin(Edge::Top, top);
                    }
                    s.window.set_monitor(monitor.as_ref());
                }
            }
            s.windows = windows;
            s.open = true;
            s.mode = Mode::All;
            let now = glib::monotonic_time();
            s.fade.go(1.0, now, 240, motion::STANDARD);
            s.rise.go(1.0, now, 560, SETTLE);
            s.window.set_keyboard_mode(KeyboardMode::OnDemand);
        }
        self.style();
        let (window, field) = {
            let s = self.0.borrow();
            (s.window.clone(), s.field.clone())
        };
        field.set_text("");
        self.search();
        window.present();
        field.grab_focus();
        self.kick();
    }

    fn close(&self) {
        {
            let mut s = self.0.borrow_mut();
            if !s.open {
                return;
            }
            s.open = false;
            let now = glib::monotonic_time();
            s.fade.go(0.0, now, 200, motion::STANDARD);
            s.rise.go(0.0, now, 240, LEAVE);
            s.window.set_keyboard_mode(KeyboardMode::None);
        }
        self.kick();
    }

    fn hover_bubble(&self, at: Option<usize>) {
        let wait = {
            let mut s = self.0.borrow_mut();
            if s.hovered == at {
                return;
            }
            let now = glib::monotonic_time();
            s.hovered = at;
            for (i, glow) in s.glow.iter_mut().enumerate() {
                if Some(i) == at {
                    glow.go(1.0, now, 160, motion::STANDARD);
                } else {
                    glow.go(0.0, now, 240, motion::STANDARD);
                }
            }
            s.tip_wait += 1;
            match at {
                Some(i) if s.tip.target() > 0.5 => {
                    s.tip_at = i;
                    None
                }
                Some(_) => Some(s.tip_wait),
                None => {
                    s.tip.go(0.0, now, 120, motion::FAST_EFFECTS);
                    None
                }
            }
        };
        if let Some(generation) = wait {
            let weak = Rc::downgrade(&self.0);
            glib::timeout_add_local_once(std::time::Duration::from_millis(380), move || {
                let Some(s) = weak.upgrade() else { return };
                let show = {
                    let mut st = s.borrow_mut();
                    match (st.tip_wait == generation, st.hovered) {
                        (true, Some(i)) => {
                            st.tip_at = i;
                            st.tip.go(1.0, glib::monotonic_time(), 160, motion::STANDARD);
                            true
                        }
                        _ => false,
                    }
                };
                if show {
                    Launcher(s).kick();
                }
            });
        }
        self.kick();
    }

    fn set_mode(&self, mode: Mode) {
        self.0.borrow_mut().mode = mode;
        self.search();
    }

    fn style(&self) {
        let s = self.0.borrow();
        let pal = s.app.palette.get();
        s.css.load_from_string(&format!(
            "text.digit-field {{ background: none; border: none; box-shadow: none; padding: 0; min-height: 34px; font-family: \"{}\"; font-size: 22px; font-weight: 420; color: {}; caret-color: {}; }}
             text.digit-field > placeholder {{ color: {}; }}
             text.digit-field > selection {{ background-color: {}; color: {}; }}",
            s.app.cfg.font,
            draw::css(pal.ink_main),
            draw::css(pal.accent),
            draw::css(pal.ink_meta),
            draw::css(pal.accent.alpha(0.32)),
            draw::css(pal.ink_main),
        ));
    }

    fn step_selection(&self, by: i32) {
        let mut s = self.0.borrow_mut();
        let picks: Vec<usize> = s.lines.iter().enumerate().filter(|(_, l)| matches!(l, Line::Item(_))).map(|(i, _)| i).collect();
        if picks.is_empty() {
            return;
        }
        let at = picks.iter().position(|&i| i == s.selected).unwrap_or(0) as i32;
        let next = (at + by).clamp(0, picks.len() as i32 - 1) as usize;
        s.selected = picks[next];
        s.moved = true;
        s.reveal();
        drop(s);
        self.kick();
    }

    fn scroll_by(&self, dy: f64) {
        let mut s = self.0.borrow_mut();
        let now = glib::monotonic_time();
        let limit = (s.content() - s.panel.target() + 2.0 * INSET).max(0.0);
        let to = (s.scroll.target() + dy).clamp(0.0, limit);
        s.scroll.go(to, now, 160, motion::STANDARD);
        drop(s);
        self.kick();
    }

    fn bubble_at(&self, x: f64, y: f64) -> Option<Mode> {
        let s = self.0.borrow();
        if !s.open || s.bubbles.target() < 0.5 || !(PAD..=PAD + BAR).contains(&y) {
            return None;
        }
        (0..4)
            .find(|&i| {
                let n = MODES.len();
                let bx = PAD + WIDTH - (n - i) as f64 * BUBBLE - (n - 1 - i) as f64 * BUBBLE_GAP;
                (bx..=bx + BUBBLE).contains(&x)
            })
            .map(|i| MODES[i].0)
    }

    fn line_at(&self, x: f64, y: f64) -> Option<usize> {
        let s = self.0.borrow();
        if !s.open || !(PAD..=PAD + WIDTH).contains(&x) || y < PANEL_TOP || y > PANEL_TOP + s.panel.target() {
            return None;
        }
        let mut top = PANEL_TOP + INSET - s.scroll.target();
        for (i, line) in s.lines.iter().enumerate() {
            let h = line.height();
            if y >= top && y < top + h {
                return matches!(line, Line::Item(_)).then_some(i);
            }
            top += h;
        }
        None
    }

    fn inside(&self, x: f64, y: f64) -> bool {
        let s = self.0.borrow();
        let bar = (PAD..=PAD + s.bar.target()).contains(&x) && (PAD..=PAD + BAR).contains(&y);
        let panel = (PAD..=PAD + WIDTH).contains(&x) && (PANEL_TOP..=PANEL_TOP + s.panel.target()).contains(&y);
        bar || panel
    }

    fn search(&self) {
        let mut s = self.0.borrow_mut();
        let raw = s.field.text().to_string();
        let mut mode = s.mode;
        let mut query = raw.trim().to_string();
        if mode == Mode::All {
            if let Some(c) = raw.chars().next() {
                let prefixed = match c {
                    '>' => Some(Mode::Apps),
                    '@' => Some(Mode::Windows),
                    '=' => Some(Mode::Calc),
                    '?' => Some(Mode::Web),
                    '"' => Some(Mode::Clip),
                    _ => None,
                };
                if let Some(m) = prefixed {
                    mode = m;
                    query = raw[c.len_utf8()..].trim().to_string();
                }
            }
        }
        let run = mode == Mode::All && raw.starts_with('$');
        if run {
            query = raw[1..].trim().to_string();
        }
        let q = fold(&query);
        let apps = s.app.apps.list();
        let mut items: Vec<Item> = Vec::new();
        if run {
            if !query.is_empty() {
                items.push(Item { kind: Kind::Run, act: Act::Run(query.clone()), title: query.clone(), detail: "Run".into(), look: Look::Symbol("terminal"), score: 2000 });
            }
        } else {
            if matches!(mode, Mode::All | Mode::Windows) && (!q.is_empty() || mode == Mode::Windows) {
                for w in &s.windows {
                    let hit = [score(&q, &fold(&w.title)), score(&q, &fold(&w.app_id))].into_iter().flatten().max();
                    let Some(hit) = hit else { continue };
                    let entry = s.app.apps.find(&w.app_id).and_then(|i| apps.get(i));
                    let name = entry.map(|e| e.name.clone()).unwrap_or_else(|| crate::apps::pretty(&w.app_id));
                    let title = if w.title.is_empty() { name.clone() } else { w.title.clone() };
                    let detail = if fold(&title) == fold(&name) { "Window".to_string() } else { name };
                    items.push(Item {
                        kind: Kind::Window,
                        act: Act::Window(w.id),
                        title,
                        detail,
                        look: entry.and_then(|e| e.icon.clone()).map(Look::Icon).unwrap_or(Look::Symbol("select_window")),
                        score: hit + 60 - if w.focused { 40 } else { 0 },
                    });
                }
            }
            if matches!(mode, Mode::All | Mode::Apps) && (!q.is_empty() || mode == Mode::Apps) {
                for (i, e) in apps.iter().enumerate() {
                    let hit = e.keys.iter().filter_map(|(k, bias)| score(&q, k).map(|v| v + bias)).max();
                    if let Some(hit) = hit {
                        let bonus = if q.is_empty() { 0 } else { s.app.apps.boost(&e.id) };
                        items.push(Item { kind: Kind::App, act: Act::App(i), title: e.name.clone(), detail: e.detail.clone(), look: look(e), score: hit + bonus });
                    }
                }
            }
            if mode == Mode::Calc || (mode == Mode::All && calc::looks_like_math(&query)) {
                if let Some(v) = calc::eval(&query) {
                    let result = calc::format(v);
                    items.push(Item { kind: Kind::Calc, act: Act::Copy(result.clone()), title: result, detail: "Copy".into(), look: Look::Symbol("calculate"), score: 2000 });
                }
            }
            if mode == Mode::All && !query.is_empty() {
                let keyed = query.split_once(' ').and_then(|(k, rest)| s.scopes.iter().position(|sc| sc.key.eq_ignore_ascii_case(k)).map(|i| (i, rest.trim().to_string())));
                if let Some((i, rest)) = keyed.filter(|(_, r)| !r.is_empty()) {
                    let sc = &s.scopes[i];
                    let look = sc.icon.clone().map(Look::Icon).unwrap_or(Look::Symbol(sc.symbol));
                    items.push(Item { kind: Kind::Scope, act: Act::Scope(i, rest.clone()), title: format!("Search {} for “{}”", sc.name, rest), detail: sc.key.clone(), look, score: 2600 });
                } else if query.chars().count() >= 2 {
                    for (i, sc) in s.scopes.iter().enumerate().filter(|(_, sc)| sc.app || matches!(sc.reach, providers::Reach::Store)) {
                        let look = sc.icon.clone().map(Look::Icon).unwrap_or(Look::Symbol(sc.symbol));
                        items.push(Item { kind: Kind::Scope, act: Act::Scope(i, query.clone()), title: format!("Search {} for “{}”", sc.name, query), detail: format!("{} …", sc.key), look, score: -800 - i as i32 });
                    }
                }
                for (key, title, symbol, keys) in POWER {
                    let hit = keys.split_whitespace().filter_map(|k| score(&q, k)).max().or_else(|| score(&q, &fold(title)));
                    if let Some(hit) = hit.filter(|_| q.chars().count() >= 3) {
                        items.push(Item { kind: Kind::Settings, act: Act::Spawn(vec!["digitctl".into(), "key".into(), key.into()]), title: title.into(), detail: "System".into(), look: Look::Symbol(symbol), score: hit - 180 });
                    }
                }
                for (page, title, detail, symbol, keys) in PAGES {
                    let hit = keys.split_whitespace().filter_map(|k| score(&q, k)).max().or_else(|| score(&q, &fold(title)));
                    if let Some(hit) = hit.filter(|_| q.chars().count() >= 3) {
                        items.push(Item { kind: Kind::Settings, act: Act::Spawn(vec!["digit-settings".into(), page.into()]), title: title.into(), detail: detail.into(), look: Look::Symbol(symbol), score: hit - 200 });
                    }
                }
                if s.found_q == query {
                    for (rank, f) in s.found.iter().enumerate() {
                        let look = match &f.glyph {
                            Some(g) => Look::Glyph(g.clone()),
                            None => f.icon.clone().map(Look::Icon).unwrap_or(Look::Symbol("search")),
                        };
                        items.push(Item { kind: Kind::Found(f.provider as u16), act: Act::Found(f.provider, f.id.clone()), title: f.name.clone(), detail: f.detail.clone(), look, score: 300 - rank as i32 });
                    }
                    let mut seen: Vec<usize> = Vec::new();
                    for f in &s.found {
                        if !seen.contains(&f.provider) {
                            seen.push(f.provider);
                        }
                    }
                    for p in seen {
                        let name = s.providers.get(p).map(|x| x.name.clone()).unwrap_or_default();
                        items.push(Item { kind: Kind::Found(p as u16), act: Act::More(p), title: format!("More in {name}"), detail: String::new(), look: Look::Symbol("arrow_forward"), score: -5000 });
                    }
                }
            }
            if matches!(mode, Mode::All | Mode::Web) && !query.is_empty() {
                let url = s.app.cfg.search_url.replace("{}", &encode(&query));
                let top = if mode == Mode::Web { 2000 } else { -1000 };
                items.push(Item { kind: Kind::Web, act: Act::Open(url), title: query.clone(), detail: "Search the web".into(), look: Look::Symbol("travel_explore"), score: top });
            }
        }
        if mode == Mode::Clip {
            clip_items(&mut items, &query);
        } else if !query.is_empty() && matches!(mode, Mode::All | Mode::Calc) {
            if let Some(c) = kit::color::parse(&query) {
                color_items(&mut items, c, &query, 2100);
            }
        }
        if mode == Mode::Clip {
        } else if q.is_empty() && mode == Mode::Apps {
            items.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
        } else {
            items.sort_by(|a, b| b.score.cmp(&a.score));
        }
        let mut lines = Vec::new();
        if mode == Mode::All && items.len() > 1 {
            lines.push(Line::Head("Top hit".into()));
            lines.push(Line::Item(0));
            let mut kinds: Vec<Kind> = Vec::new();
            for it in items.iter().skip(1) {
                if !kinds.contains(&it.kind) {
                    kinds.push(it.kind);
                }
            }
            for k in kinds {
                let title = match k {
                    Kind::Found(p) => s.providers.get(p as usize).map(|x| x.name.clone()).unwrap_or_else(|| k.title().into()),
                    _ => k.title().into(),
                };
                lines.push(Line::Head(title));
                let cap = if k == Kind::Scope { 4 } else { 6 };
                lines.extend(items.iter().enumerate().skip(1).filter(|(_, it)| it.kind == k).take(cap).map(|(i, _)| Line::Item(i)));
            }
        } else {
            lines.extend((0..items.len().min(200)).map(Line::Item));
        }
        let placeholder = MODES.iter().find(|m| m.0 == mode).map(|m| m.2).unwrap_or("Search");
        s.field.set_placeholder_text(Some(placeholder));
        let ask = (mode == Mode::All && !run && query.chars().count() >= 2 && !s.providers.is_empty() && s.found_q != query).then(|| query.clone());
        if s.found_q != query {
            s.found.clear();
            s.found_q.clear();
            s.moved = false;
        }
        s.terms = query.split_whitespace().map(str::to_string).collect();
        s.shown = mode;
        s.items = items;
        s.lines = lines;
        s.selected = s.lines.iter().position(|l| matches!(l, Line::Item(_))).unwrap_or(0);
        let now = glib::monotonic_time();
        let height = if s.lines.is_empty() { 0.0 } else { (s.content() + 2.0 * INSET).min(MAX_PANEL) };
        let first = s.panel.target() < 1.0;
        if s.open {
            if first {
                s.panel.go(height, now, 280, motion::EMPHASIZED_DECEL);
            } else {
                s.panel.go(height, now, 180, motion::STANDARD);
            }
            let bubbles = raw.is_empty();
            s.bubbles.go(if bubbles { 1.0 } else { 0.0 }, now, 180, motion::STANDARD);
            s.bar.go(if bubbles { NARROW } else { WIDTH }, now, 260, motion::DEFAULT_SPATIAL);
        }
        s.scroll.set(0.0);
        drop(s);
        if let Some(q) = ask {
            self.ask(q);
        }
        self.kick();
    }

    fn ask(&self, query: String) {
        let (generation, list) = {
            let mut s = self.0.borrow_mut();
            s.asking += 1;
            (s.asking, s.providers.clone())
        };
        let weak = Rc::downgrade(&self.0);
        glib::spawn_future_local(async move {
            glib::timeout_future(std::time::Duration::from_millis(170)).await;
            let live = |w: &Weak<RefCell<State>>| w.upgrade().filter(|st| st.borrow().asking == generation && st.borrow().open);
            if live(&weak).is_none() {
                return;
            }
            if let Some(st) = live(&weak) {
                let mut s = st.borrow_mut();
                s.found.clear();
                s.found_q = query.clone();
            }
            let terms: Vec<String> = query.split_whitespace().map(str::to_string).collect();
            let mut calls: futures_util::stream::FuturesUnordered<_> = list.into_iter().enumerate().map(|(i, p)| providers::query(p, i, terms.clone())).collect();
            use futures_util::StreamExt;
            while let Some(found) = calls.next().await {
                let Some(st) = live(&weak) else { return };
                if found.is_empty() {
                    continue;
                }
                st.borrow_mut().found.extend(found);
                Launcher(st).research();
            }
        });
    }

    fn research(&self) {
        let keep = {
            let s = self.0.borrow();
            match s.lines.get(s.selected).filter(|_| s.moved) {
                Some(Line::Item(i)) => s.items.get(*i).map(|it| (it.title.clone(), it.detail.clone())),
                _ => None,
            }
        };
        self.search();
        if let Some((title, detail)) = keep {
            let mut s = self.0.borrow_mut();
            let at = s.lines.iter().position(|l| matches!(l, Line::Item(i) if s.items.get(*i).is_some_and(|it| it.title == title && it.detail == detail)));
            if let Some(at) = at {
                s.selected = at;
            }
        }
    }

    fn activate(&self, line: Option<usize>) {
        let (act, app) = {
            let mut s = self.0.borrow_mut();
            let at = line.unwrap_or(s.selected);
            let Some(Line::Item(i)) = s.lines.get(at) else { return };
            let i = *i;
            if i >= s.items.len() {
                return;
            }
            let item = s.items.swap_remove(i);
            s.lines.clear();
            (item.act, s.app.clone())
        };
        let (list, scopes, terms) = {
            let s = self.0.borrow();
            (s.providers.clone(), s.scopes.clone(), s.terms.clone())
        };
        self.close();
        match act {
            Act::Found(p, id) => {
                if let Some(p) = list.get(p) {
                    providers::activate(p.clone(), id, terms);
                }
            }
            Act::More(p) => {
                if let Some(p) = list.get(p) {
                    providers::launch_search(p.clone(), terms);
                }
            }
            Act::Scope(i, query) => match scopes.get(i).map(|sc| sc.reach.clone()) {
                Some(providers::Reach::Url(template)) => {
                    let url = template.replace("{}", &encode(&query));
                    if let Some(display) = gdk::Display::default() {
                        let context = display.app_launch_context();
                        crate::apps::prepare(context.upcast_ref());
                        let _ = gio::AppInfo::launch_default_for_uri(&url, Some(&context));
                    }
                }
                Some(providers::Reach::Switcher { ids, entry }) => providers::switcher(app.clone(), ids, entry, query),
                Some(providers::Reach::Store) => {
                    let arg = format!("search:{}", query.chars().filter(|c| !c.is_control()).take(80).collect::<String>());
                    let argv: [&std::ffi::OsStr; 2] = ["digit-store".as_ref(), arg.as_ref()];
                    let _ = gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE);
                }
                None => {}
            },
            Act::Spawn(argv) => {
                let argv: Vec<&std::ffi::OsStr> = argv.iter().map(std::ffi::OsStr::new).collect();
                let _ = gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE);
            }
            Act::App(i) => {
                let list = app.apps.list();
                if let Some(e) = list.get(i) {
                    app.apps.launch(e);
                }
            }
            Act::Window(id) => {
                glib::spawn_future_local(async move {
                    let _ = gio::spawn_blocking(move || compositor::focus(id)).await;
                });
            }
            Act::Copy(text) => {
                if let Some(display) = gdk::Display::default() {
                    display.clipboard().set_text(&text);
                }
            }
            Act::Image(t) => {
                if let Some(display) = gdk::Display::default() {
                    display.clipboard().set_texture(&t);
                }
            }
            Act::Clear => crate::clip::clear(),
            Act::Open(url) => {
                if let Some(display) = gdk::Display::default() {
                    let context = display.app_launch_context();
                    crate::apps::prepare(context.upcast_ref());
                    let _ = gio::AppInfo::launch_default_for_uri(&url, Some(&context));
                }
            }
            Act::Run(command) => {
                let mut cmd = std::process::Command::new("sh");
                cmd.arg("-c").arg(&command);
                for (k, v) in crate::gpu::child_env() {
                    match v {
                        Some(v) => cmd.env(k, v),
                        None => cmd.env_remove(k),
                    };
                }
                if let Ok(mut child) = cmd.spawn() {
                    std::thread::spawn(move || child.wait());
                }
            }
        }
    }

    fn kick(&self) {
        let (canvas, already) = {
            let mut s = self.0.borrow_mut();
            let already = s.ticking;
            s.ticking = true;
            (s.canvas.clone(), already)
        };
        if already {
            return;
        }
        let weak: Weak<RefCell<State>> = Rc::downgrade(&self.0);
        canvas.add_tick_callback(move |w, clock| {
            let Some(state) = weak.upgrade() else { return glib::ControlFlow::Break };
            let Ok(mut s) = state.try_borrow_mut() else { return glib::ControlFlow::Continue };
            let busy = s.step(clock.frame_time());
            w.queue_draw();
            if busy {
                glib::ControlFlow::Continue
            } else {
                s.ticking = false;
                glib::ControlFlow::Break
            }
        });
    }
}

fn look(e: &crate::apps::Entry) -> Look {
    e.icon.clone().map(Look::Icon).unwrap_or(Look::Symbol("apps"))
}

fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            b' ' => "+".into(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

impl State {
    fn content(&self) -> f64 {
        self.lines.iter().map(Line::height).sum()
    }

    fn reveal(&mut self) {
        let mut top = 0.0;
        for (i, line) in self.lines.iter().enumerate() {
            if i == self.selected {
                break;
            }
            top += line.height();
        }
        let view = self.panel.target() - 2.0 * INSET;
        let mut to = self.scroll.target();
        let header = matches!(self.selected.checked_sub(1).and_then(|p| self.lines.get(p)), Some(Line::Head(_)));
        let lead = if header { HEAD } else { 0.0 };
        if top - lead < to {
            to = top - lead;
        } else if top + ROW > to + view {
            to = top + ROW - view;
        }
        self.scroll.go(to.max(0.0), glib::monotonic_time(), 140, motion::STANDARD);
    }

    fn step(&mut self, now: i64) -> bool {
        let fade = self.fade.value(now).clamp(0.0, 1.0);
        self.field.set_opacity(fade);
        let rise = self.rise.value(now);
        let dy = if self.open { -10.0 } else { -6.0 } * (1.0 - rise);
        self.lift.set(1.0, dy);
        let want = (self.bar.value(now) - 54.0 - 22.0).max(40.0) as i32;
        if want != self.field_w {
            self.field.set_size_request(want, 34);
            self.field_w = want;
        }
        let rect = if self.open { (0, 0, SURFACE_W as i32, SURFACE_H as i32) } else { (0, 0, 1, 1) };
        if rect != self.region {
            if let Some(surface) = self.window.surface() {
                surface.set_input_region(Some(&cairo::Region::create_rectangle(&cairo::RectangleInt::new(rect.0, rect.1, rect.2, rect.3))));
                self.region = rect;
            }
        }
        let moving = [&self.fade, &self.rise, &self.panel, &self.bar, &self.bubbles, &self.scroll, &self.tip].iter().any(|t| !t.settled(now))
            || self.glow.iter().any(|t| !t.settled(now))
            || self.app.palette_moving(now);
        if !moving && !self.open && self.window.is_visible() {
            let window = self.window.clone();
            glib::idle_add_local_once(move || window.set_visible(false));
            self.panel.set(0.0);
            self.lines.clear();
            self.items.clear();
            self.icons.clear();
            self.region = (0, 0, 0, 0);
            draw::forget(&self.canvas);
            App::trim();
        }
        moving
    }

    fn paintable(&mut self, icon: &gio::Icon, size: i32) -> Option<gtk::IconPaintable> {
        let key = icon.to_string()?.to_string();
        if let Some(p) = self.icons.get(&key) {
            return Some(p.clone());
        }
        let display = gdk::Display::default()?;
        let p = gtk::IconTheme::for_display(&display).lookup_by_gicon(icon, size, self.window.scale_factor(), gtk::TextDirection::Ltr, gtk::IconLookupFlags::empty());
        if self.icons.len() > 256 {
            self.icons.clear();
        }
        self.icons.insert(key, p.clone());
        Some(p)
    }

    fn draw(&mut self, s: &gtk::Snapshot, now: i64) {
        let fade = self.fade.value(now).clamp(0.0, 1.0);
        if fade < 0.003 {
            return;
        }
        let pal = self.app.palette_at(now);
        let cfg = self.app.cfg.clone();
        let canvas = self.canvas.clone();
        s.push_opacity(fade);
        let bar_w = self.bar.value(now);
        let mid = PAD + BAR / 2.0;
        slab(s, &rounded(PAD, PAD, bar_w, BAR, BAR / 2.0), pal.body);
        match MODES.iter().find(|m| m.0 == self.shown) {
            Some(m) => draw::icon(s, &canvas, &cfg, m.1, 24.0, true, pal.accent, PAD + 30.0, mid),
            None => draw::icon(s, &canvas, &cfg, "search", 24.0, false, pal.ink_meta, PAD + 30.0, mid),
        }
        let shown = self.bubbles.value(now).clamp(0.0, 1.0);
        if shown > 0.01 {
            s.push_opacity(shown);
            let rise = if self.open { self.rise.value(now) } else { 1.0 };
            for (i, (mode, icon, _, _)) in MODES.iter().enumerate() {
                let enter = ((rise - 0.1 * i as f64) / 0.6).clamp(0.0, 1.0);
                if enter < 0.01 {
                    continue;
                }
                s.push_opacity(enter);
                let n = MODES.len();
                let x = (PAD + WIDTH - (n - i) as f64 * BUBBLE - (n - 1 - i) as f64 * BUBBLE_GAP + (1.0 - shown) * 18.0 + (1.0 - enter) * 14.0).round();
                let on = *mode == self.mode;
                let glow = self.glow[i].value(now).clamp(0.0, 1.0);
                let grow = 1.5 * glow;
                let fill = if on { pal.accent } else { pal.ink_main.mix(pal.body, 0.09 * glow) };
                slab(s, &rounded(x - grow, PAD - grow, BUBBLE + 2.0 * grow, BUBBLE + 2.0 * grow, BUBBLE / 2.0 + grow), fill);
                let ink = if on { pal.on_accent } else { pal.ink_main.mix(pal.ink_sec, glow) };
                draw::icon(s, &canvas, &cfg, icon, 24.0, on, ink, x + BUBBLE / 2.0, mid);
                s.pop();
            }
            s.pop();
        }
        let height = self.panel.value(now);
        if height > 1.0 && !self.lines.is_empty() {
            let panel = rounded(PAD, PANEL_TOP, WIDTH, height, PANEL_R);
            slab(s, &panel, pal.body);
            s.push_rounded_clip(&panel);
            self.draw_lines(s, &pal, now, height);
            s.pop();
        }
        let tip = self.tip.value(now).clamp(0.0, 1.0) * shown;
        if tip > 0.01 {
            let (_, _, _, label) = MODES[self.tip_at];
            let hint = format!("Ctrl {}", self.tip_at + 1);
            let lw = draw::width(&canvas, &cfg, label, 13.0, Face::Text(520));
            let hw = draw::width(&canvas, &cfg, &hint, 12.0, Face::Text(420));
            let w = lw + hw + 32.0;
            let n = MODES.len();
            let cx = PAD + WIDTH - (n - self.tip_at) as f64 * BUBBLE - (n - 1 - self.tip_at) as f64 * BUBBLE_GAP + BUBBLE / 2.0;
            let x = (cx - w / 2.0).min(PAD + WIDTH - w).max(PAD);
            let y = (PAD + BAR + 10.0 - 4.0 * (1.0 - tip)).round();
            s.push_opacity(tip);
            slab(s, &rounded(x, y, w, 28.0, 14.0), pal.body.mix(pal.ink_main, 0.94));
            draw::text(s, &canvas, &cfg, label, 13.0, Face::Text(520), pal.ink_main, x + 12.0, y + 14.0, lw + 2.0, pango::Alignment::Left);
            draw::text(s, &canvas, &cfg, &hint, 12.0, Face::Text(420), pal.ink_meta, x + 20.0 + lw, y + 14.0, hw + 2.0, pango::Alignment::Left);
            s.pop();
        }
        s.pop();
    }

    fn draw_lines(&mut self, s: &gtk::Snapshot, pal: &Palette, now: i64, height: f64) {
        let cfg = self.app.cfg.clone();
        let canvas = self.canvas.clone();
        let mut top = PANEL_TOP + INSET - self.scroll.value(now).round();
        let bottom = PANEL_TOP + height;
        for li in 0..self.lines.len() {
            let h = self.lines[li].height();
            if top + h < PANEL_TOP || top > bottom {
                top += h;
                continue;
            }
            match self.lines[li] {
                Line::Head(ref title) => {
                    let title = title.clone();
                    draw::text(s, &canvas, &cfg, &title, 12.5, Face::Text(560), pal.ink_meta, PAD + INSET + 14.0, top + HEAD / 2.0 + 3.0, WIDTH - 60.0, pango::Alignment::Left);
                }
                Line::Item(i) => {
                    let on = li == self.selected;
                    let mid = top + ROW / 2.0;
                    if on {
                        let row = rounded(PAD + INSET, top, WIDTH - 2.0 * INSET, ROW, PANEL_R - INSET);
                        s.push_rounded_clip(&row);
                        s.append_color(&pal.accent.rgba(), row.bounds());
                        s.pop();
                    }
                    let (title_ink, detail_ink, symbol_ink) = if on {
                        (pal.on_accent, pal.on_accent.mix(pal.accent, 0.72), pal.on_accent)
                    } else {
                        (pal.ink_main, pal.ink_meta, pal.ink_sec)
                    };
                    let (title, detail) = (self.items[i].title.clone(), self.items[i].detail.clone());
                    let icon = match &self.items[i].look {
                        Look::Glyph(g) => {
                            let g = g.clone();
                            draw::text(s, &canvas, &cfg, &g, 22.0, Face::Text(400), title_ink, PAD + INSET + 8.0, mid, 36.0, pango::Alignment::Center);
                            None
                        }
                        Look::Icon(icon) => Some(icon.clone()),
                        Look::Symbol(name) => {
                            draw::icon(s, &canvas, &cfg, name, 22.0, false, symbol_ink, PAD + INSET + 26.0, mid);
                            None
                        }
                        Look::Swatch(c) => {
                            let r = rounded(PAD + INSET + 13.0, mid - 13.0, 26.0, 26.0, 8.0);
                            s.push_rounded_clip(&r);
                            s.append_color(&gdk::RGBA::new(c.r as f32, c.g as f32, c.b as f32, c.a as f32), r.bounds());
                            s.pop();
                            let edge = gdk::RGBA::new(1.0, 1.0, 1.0, 0.22);
                            s.append_border(&r, &[1.0; 4], &[edge, edge, edge, edge]);
                            None
                        }
                        Look::Picture(t) => {
                            let r = rounded(PAD + INSET + 11.0, mid - 15.0, 30.0, 30.0, 7.0);
                            s.push_rounded_clip(&r);
                            let (tw, th) = (t.width().max(1) as f32, t.height().max(1) as f32);
                            let k = (30.0 / tw).max(30.0 / th);
                            let (w, h) = (tw * k, th * k);
                            let x = (PAD + INSET + 11.0) as f32 + (30.0 - w) / 2.0;
                            let y = (mid - 15.0) as f32 + (30.0 - h) / 2.0;
                            s.append_texture(t, &graphene::Rect::new(x, y, w, h));
                            s.pop();
                            None
                        }
                    };
                    if let Some(icon) = icon {
                        if let Some(p) = self.paintable(&icon, 28) {
                            s.save();
                            s.translate(&graphene::Point::new((PAD + INSET + 12.0) as f32, (mid - 14.0) as f32));
                            p.snapshot(s, 28.0, 28.0);
                            s.restore();
                        }
                    }
                    let detail_w = 220.0;
                    let x = PAD + INSET + 52.0;
                    draw::text(s, &canvas, &cfg, &title, 15.0, Face::Text(if on { 560 } else { 480 }), title_ink, x, mid, WIDTH - 2.0 * INSET - 52.0 - 20.0 - detail_w, pango::Alignment::Left);
                    draw::text(s, &canvas, &cfg, &detail, 13.0, Face::Text(420), detail_ink, PAD + WIDTH - INSET - 16.0 - detail_w, mid, detail_w, pango::Alignment::Right);
                }
            }
            top += h;
        }
    }
}

mod imp {
    use super::State;
    use gtk::subclass::prelude::*;
    use gtk::{glib, prelude::*};
    use std::cell::RefCell;
    use std::rc::Weak;

    #[derive(Default)]
    pub struct Canvas {
        pub(super) state: RefCell<Weak<RefCell<State>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Canvas {
        const NAME: &'static str = "DigitLauncher";
        type Type = super::Canvas;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Canvas {}

    impl WidgetImpl for Canvas {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let Some(state) = self.state.borrow().upgrade() else { return };
            let Ok(mut s) = state.try_borrow_mut() else {
                self.obj().queue_draw();
                return;
            };
            let now = self.obj().frame_clock().map(|c| c.frame_time()).unwrap_or_else(glib::monotonic_time);
            s.draw(snapshot, now);
        }
    }
}

glib::wrapper! {
    pub struct Canvas(ObjectSubclass<imp::Canvas>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}
