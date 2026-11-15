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
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::Duration;

const PAD: f64 = 40.0;
const WIDTH: f64 = 680.0;
const BAR: f64 = 56.0;
const BUBBLE: f64 = 56.0;
const BUBBLE_GAP: f64 = 10.0;
use crate::metrics::{GAP, INSET};
const PANEL_R: f64 = 24.0;
const ROW: f64 = 44.0;
const HEAD: f64 = 30.0;
const MAX_PANEL: f64 = 448.0;
const CARD_H: f64 = 56.0;
/// The small round button under the clipboard list that clears it.
const CLEAR: f64 = BUBBLE;
const SURFACE_W: f64 = WIDTH + 2.0 * PAD;
const SURFACE_H: f64 = PAD + BAR + GAP + MAX_PANEL + GAP + CLEAR + PAD;
const PANEL_TOP: f64 = PAD + BAR + GAP;
const FIELD_X: f64 = PAD + 54.0;
const NARROW: f64 = WIDTH - MODES.len() as f64 * (BUBBLE + BUBBLE_GAP);
const SETTLE: motion::Curve = motion::Curve::new(0.16, 1.0, 0.3, 1.0);
const LEAVE: motion::Curve = motion::Curve::new(0.4, 0.0, 0.9, 0.6);
/// How the drawn caret glides to its new spot, quick to start and soft to land.
const CARET: motion::Curve = motion::Curve::new(0.2, 0.0, 0.0, 1.0);

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
    Prism,
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
            Kind::Prism => "Minecraft",
        }
    }
}

enum Act {
    App(usize),
    Window(u64),
    Copy(String),
    Open(String),
    Run(String),
    Image(Rc<crate::clip::Clip>),
    Clip(Rc<crate::clip::Clip>),
    Clear,
    Found(usize, String),
    More(usize),
    Scope(usize, String),
    Spawn(Vec<String>),
}

const PAGES: [(&str, &str, &str, &str, &str); 18] = [
    ("wifi", "Wi-Fi", "Settings", "wifi", "wifi wi-fi wireless wlan network internet hotspot password ssid"),
    ("bluetooth", "Bluetooth", "Settings", "bluetooth", "bluetooth pair pairing headphones headset mouse keyboard controller adapter blueman"),
    ("displays", "Displays", "Settings", "monitor", "display displays monitor monitors screen resolution refresh rate hz scale rotation arrange"),
    ("welcome", "Welcome tour", "Set up Digit", "waving_hand", "welcome tour onboarding setup start intro getting started wizard"),
    ("overview", "About this computer", "System overview", "info", "about system computer hardware memory disk processor gpu overview"),
    ("updates", "Software updates", "Settings", "system_update_alt", "update updates upgrade security patch packages"),
    ("apps", "Default and startup apps", "Settings", "apps", "apps applications default startup autostart browser"),
    ("drivers", "Drivers and firmware", "Settings", "memory", "driver drivers nvidia amd intel gpu firmware graphics"),
    ("appearance", "Appearance", "Settings", "palette", "appearance theme accent color colors scheme icons corners roundness surfaces dark density effects blur shadow rendering"),
    ("dock", "Dock", "Settings", "dock_to_bottom", "dock icons magnification autohide badges"),
    ("notch", "Notch and menu bar", "Settings", "toolbar", "notch island bar menubar menu tray call panels hot corner"),
    ("desktop", "Desktop", "Settings", "wallpaper", "desktop clock widget weather city screen corners"),
    ("spotlight", "Spotlight", "Settings", "search", "spotlight search launcher caret clipboard history minecraft prism"),
    ("compositor", "Windows", "Settings", "select_window", "windows compositor hyprland niri setup gaps layout tiling floating placement mouse touchpad animations"),
    ("keyboard", "Keyboard shortcuts", "Settings", "keyboard", "keyboard shortcuts keybinds hotkeys keys"),
    ("power", "Lock and power", "Settings", "power_settings_new", "lock power idle sleep suspend screen off"),
    ("screenshots", "Screenshots", "Settings", "screenshot_region", "screenshot screenshots capture print folder format"),
    ("recording", "Recording", "Settings", "radio_button_checked", "record recording video screen capture replay stream gpu"),
];

/// An instance's name as typed: folder-style names use _ and - between words.
fn prism_key(name: &str) -> String {
    fold(&name.replace(['_', '-'], " "))
}

/// A PrismLauncher instance as a result that starts it.
fn prism_item(ins: &crate::prism::Instance, score: i32) -> Option<Item> {
    let argv = crate::prism::launch(&ins.id)?;
    let look = ins.icon.as_ref().map(|p| Look::Icon(gio::FileIcon::new(&gio::File::for_path(p)).upcast())).unwrap_or(Look::Symbol("deployed_code"));
    let detail = if ins.group.is_empty() { "Minecraft instance".to_string() } else { ins.group.clone() };
    Some(Item { kind: Kind::Prism, act: Act::Spawn(argv), title: ins.name.clone(), detail, look, score })
}

/// A page or row in digit Settings, from the index Settings writes for Spotlight.
#[derive(Clone, serde::Deserialize)]
struct Setting {
    title: String,
    place: String,
    words: String,
    icon: String,
    link: String,
    /// A whole page, rather than a row on it.
    page: bool,
    #[serde(skip)]
    symbol: &'static str,
}

impl Setting {
    fn item(&self, score: i32) -> Item {
        Item { kind: Kind::Settings, act: Act::Spawn(vec!["digit-settings".into(), self.link.clone()]), title: self.title.clone(), detail: if self.page { "Settings".into() } else { self.place.clone() }, look: Look::Symbol(self.symbol), score }
    }
}

/// The settings index, asking Settings to write it again when it is missing or older than
/// the installed Settings. A fresh one shows up the next time Spotlight opens.
fn settings_index() -> Vec<Setting> {
    let path = gtk::glib::user_cache_dir().join("digit/settings-index.json");
    let made = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    let built = gtk::glib::find_program_in_path("digit-settings").and_then(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
    if made.is_none() || built.is_some_and(|b| made.is_some_and(|m| b > m)) {
        let argv: [&std::ffi::OsStr; 2] = ["digit-settings".as_ref(), "--index".as_ref()];
        let _ = gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE);
    }
    let list: Vec<Setting> = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    list.into_iter()
        .map(|mut s| {
            s.symbol = symbol(&s.icon);
            s
        })
        .collect()
}

/// Icon names live as long as the shell; each distinct one is kept once.
fn symbol(name: &str) -> &'static str {
    thread_local! {
        static NAMES: RefCell<HashMap<String, &'static str>> = RefCell::new(HashMap::new());
    }
    NAMES.with(|n| *n.borrow_mut().entry(name.to_string()).or_insert_with(|| Box::leak(name.to_string().into_boxed_str())))
}

/// The best `limit` settings for a folded query: every word must start a word of the title,
/// or of where it lives and what it says.
fn find_settings<'a>(list: impl IntoIterator<Item = &'a Setting>, q: &str, limit: usize) -> Vec<(i32, &'a Setting)> {
    let words: Vec<&str> = q.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<(i32, &Setting)> = list
        .into_iter()
        .filter_map(|s| {
            let title = fold(&s.title);
            let rest = fold(&format!("{} {}", s.place, s.words));
            let starts = |text: &str, w: &str| text.split(|c: char| !c.is_alphanumeric()).any(|t| t.starts_with(w));
            let mut total = 0;
            for w in &words {
                total += if title.starts_with(w) {
                    300
                } else if starts(&title, w) {
                    220
                } else if starts(&rest, w) {
                    80
                } else {
                    return None;
                };
            }
            // Prefer a category when it matches as well as one of its controls.
            Some((total / words.len() as i32 + if s.page { 20 } else { 0 }, s))
        })
        .collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0));
    let mut seen = std::collections::HashSet::new();
    hits.retain(|(_, s)| seen.insert((s.page, s.title.clone(), s.place.clone())));
    hits.truncate(limit);
    hits
}

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
    /// A copied color, with chips that copy it in other formats.
    Palette(usize),
}

impl Line {
    fn height(&self) -> f64 {
        match self {
            Line::Head(_) => HEAD,
            Line::Item(_) => ROW,
            Line::Palette(_) => CARD_H,
        }
    }

    /// Lines the keyboard selection can land on.
    fn pickable(&self) -> bool {
        matches!(self, Line::Item(_) | Line::Palette(_))
    }
}

/// A part of the color card under the pointer.
#[derive(Clone, Copy, PartialEq)]
enum CardHit {
    Format(usize),
    Body,
}

/// A copied color: the color, the text that was copied, and when.
#[derive(Clone)]
struct Copied {
    color: kit::color::Color,
    text: String,
    at: i64,
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
    /// Copied colors, newest first, and the color row part under the pointer.
    colors: Vec<Copied>,
    card_hot: Option<(usize, CardHit)>,
    clear_hot: bool,
    /// The clear button shows under the clipboard list while nothing is typed.
    clear_on: bool,
    clear_glow: Tween,
    windows: Vec<compositor::Window>,
    fade: Tween,
    panel: Tween,
    bar: Tween,
    bubbles: Tween,
    scroll: Tween,
    // The selection pill, measured from the top of the list so it scrolls with the rows.
    pill_y: Tween,
    pill_h: Tween,
    glow: [Tween; 5],
    hovered: Option<usize>,
    tip: Tween,
    tip_at: usize,
    /// The drawn caret's position, and when it last moved (it stays solid while typing).
    caret: Tween,
    caret_moved: i64,
    caret_shown: bool,
    caret_paint: crate::paint::Paint,
    caret_ticking: bool,
    caret_blink: Option<glib::SourceId>,
    caret_measure: Option<(glib::GString, i32, f64)>,
    tip_wait: u64,
    field_w: i32,
    icons: HashMap<String, gtk::IconPaintable>,
    region: (i32, i32, i32, i32),
    ticking: bool,
    scrim: Option<gtk::Window>,
    providers: Vec<providers::Provider>,
    scopes: Vec<providers::Scope>,
    files_focus: bool,
    files_folder: Option<String>,
    /// PrismLauncher's instances, read as Spotlight opens while Settings has `/pl` on.
    prism: Vec<crate::prism::Instance>,
    prism_on: bool,
    opening: u64,
    settings: Vec<Setting>,
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

/// Fills the clipboard list. Colors go to `colors` instead, each with the item index it comes
/// before, so they keep their place in the history.
fn clip_items(items: &mut Vec<Item>, colors: &mut Vec<(usize, Copied)>, query: &str) {
    let q = fold(query);
    let all = crate::clip::history();
    let scale = crate::images::scale();
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
                    if !colors.iter().any(|(_, k)| k.color.hex() == color.hex()) {
                        colors.push((items.len(), Copied { color, text: text.trim().to_string(), at: c.at }));
                    }
                    continue;
                }
                let detail = if lines > 1 { format!("{lines} lines · {}", ago(c.at)) } else { format!("{} chars · {}", text.chars().count(), ago(c.at)) };
                items.push(Item { kind: Kind::Clip, act: Act::Clip(c.clone()), title: line, detail, look: Look::Symbol("content_paste"), score });
            }
            crate::clip::Data::Image(_) => {
                if !q.is_empty() && !"image picture screenshot".contains(q.as_str()) {
                    continue;
                }
                if let Some(t) = c.preview(scale) {
                    let (w, h) = c.dimensions().unwrap_or((t.width() as usize, t.height() as usize));
                    let title = format!("Image {w} × {h}");
                    items.push(Item { kind: Kind::Clip, act: Act::Image(c.clone()), title, detail: ago(c.at), look: Look::Picture(t), score });
                }
            }
        }
    }
    if all.is_empty() {
        items.push(Item { kind: Kind::Clip, act: Act::Clear, title: "Nothing copied yet".into(), detail: "Copies appear here; kept only in memory".into(), look: Look::Symbol("content_paste_off"), score: 0 });
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
    shadow(s, shape);
    face(s, shape, fill);
}

fn shadow(s: &gtk::Snapshot, shape: &gsk::RoundedRect) {
    if !kit::effects::reduced() {
        s.append_outset_shadow(shape, &gdk::RGBA::new(0.0, 0.0, 0.0, 0.34), 0.0, 10.0, 0.0, 30.0);
    }
}

fn face(s: &gtk::Snapshot, shape: &gsk::RoundedRect, fill: Rgb) {
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
        let caret_paint = crate::paint::Paint::new(SURFACE_W as i32, 26);
        caret_paint.set_valign(gtk::Align::Start);
        caret_paint.set_margin_top((PAD + BAR / 2.0 - 13.0) as i32);
        caret_paint.set_can_target(false);
        overlay.add_overlay(&caret_paint);
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
            colors: Vec::new(),
            card_hot: None,
            clear_hot: false,
            clear_on: false,
            clear_glow: Tween::still(0.0),
            windows: Vec::new(),
            fade: Tween::still(0.0),
            panel: Tween::still(0.0),
            bar: Tween::still(NARROW),
            bubbles: Tween::still(1.0),
            scroll: Tween::still(0.0),
            pill_y: Tween::still(0.0),
            pill_h: Tween::still(ROW),
            glow: [Tween::still(0.0); 5],
            hovered: None,
            tip: Tween::still(0.0),
            tip_at: 0,
            caret: Tween::still(0.0),
            caret_moved: 0,
            caret_shown: false,
            caret_paint: caret_paint.clone(),
            caret_ticking: false,
            caret_blink: None,
            caret_measure: None,
            tip_wait: 0,
            field_w: 0,
            icons: HashMap::new(),
            region: (0, 0, 0, 0),
            ticking: false,
            scrim: None,
            providers: Vec::new(),
            scopes: Vec::new(),
            files_focus: false,
            files_folder: None,
            prism: Vec::new(),
            prism_on: false,
            opening: 0,
            settings: Vec::new(),
            found: Vec::new(),
            found_q: String::new(),
            asking: 0,
            terms: Vec::new(),
            moved: false,
        }));
        *canvas.imp().state.borrow_mut() = Rc::downgrade(&state);
        let weak = Rc::downgrade(&state);
        caret_paint.on_draw(move |snapshot| {
            if let Some(state) = weak.upgrade() {
                state.borrow().draw_caret(snapshot, glib::monotonic_time());
            }
        });
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
        // Text edits can notify both properties. Coalesce them after GTK finishes the edit.
        let pending = Rc::new(Cell::new(false));
        for property in ["text", "cursor-position", "has-focus"] {
            let pending = pending.clone();
            let weak = Rc::downgrade(&self.0);
            field.connect_notify_local(Some(property), move |_, _| {
                if pending.replace(true) {
                    return;
                }
                let (pending, weak) = (pending.clone(), weak.clone());
                glib::idle_add_local_once(move || {
                    pending.set(false);
                    if let Some(state) = weak.upgrade() {
                        Launcher(state).update_caret();
                    }
                });
            });
        }
        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(&self.0);
        click.connect_released(move |_, _, x, y| {
            let Some(s) = weak.upgrade() else { return };
            let me = Launcher(s);
            if let Some(m) = me.bubble_at(x, y) {
                let mode = me.0.borrow().mode;
                me.set_mode(if mode == m { Mode::All } else { m });
            } else if let Some(hit) = me.card_hit(x, y) {
                me.click_card(hit);
            } else if me.clear_hit(x, y) {
                me.0.borrow_mut().lines.clear();
                me.close();
                crate::clip::clear();
            } else if let Some(line) = me.line_at(x, y) {
                me.activate(Some(line));
            } else if !me.inside(x, y) {
                me.close();
            }
        });
        canvas.add_controller(click);
        let hover = gtk::EventControllerMotion::new();
        // Results moving under a still pointer also raise motion events; only a pointer that
        // really moved may take the selection, so Enter right after typing picks the best match.
        let last: Rc<std::cell::Cell<Option<(f64, f64)>>> = Rc::new(std::cell::Cell::new(None));
        let seen = last.clone();
        hover.connect_enter(move |_, x, y| seen.set(Some((x, y))));
        let weak = Rc::downgrade(&self.0);
        hover.connect_motion(move |_, x, y| {
            let Some(s) = weak.upgrade() else { return };
            let me = Launcher(s);
            let bubble = me.bubble_at(x, y).and_then(|m| MODES.iter().position(|e| e.0 == m));
            me.hover_bubble(bubble);
            let (card, clear) = (me.card_hit(x, y), me.clear_hit(x, y));
            let redraw = {
                let mut s = me.0.borrow_mut();
                let changed = s.card_hot != card || s.clear_hot != clear;
                if s.clear_hot != clear {
                    // Same hover as the mode bubbles beside the search bar.
                    let now = glib::monotonic_time();
                    if clear {
                        s.clear_glow.go(1.0, now, 160, motion::STANDARD);
                    } else {
                        s.clear_glow.go(0.0, now, 240, motion::STANDARD);
                    }
                }
                s.card_hot = card;
                s.clear_hot = clear;
                changed
            };
            if redraw {
                me.kick();
                let canvas = me.0.borrow().canvas.clone();
                canvas.set_cursor_from_name(matches!(card, Some((_, CardHit::Format(_)))).then_some("pointer").or(clear.then_some("pointer")));
                canvas.queue_draw();
            }
            let moved = last.replace(Some((x, y))).is_none_or(|(px, py)| (px - x).abs() + (py - y).abs() > 0.5);
            if !moved {
                return;
            }
            if let Some(line) = me.line_at(x, y) {
                let changed = {
                    let mut s = me.0.borrow_mut();
                    let changed = s.selected != line;
                    s.selected = line;
                    if changed {
                        s.glide_pill();
                    }
                    changed
                };
                if changed {
                    me.kick();
                }
            }
        });
        let weak = Rc::downgrade(&self.0);
        hover.connect_leave(move |_| {
            if let Some(s) = weak.upgrade() {
                {
                    let mut st = s.borrow_mut();
                    st.card_hot = None;
                    if st.clear_hot {
                        st.clear_glow.go(0.0, glib::monotonic_time(), 240, motion::STANDARD);
                    }
                    st.clear_hot = false;
                }
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
    }

    pub fn toggle(&self) {
        let open = self.0.borrow().open;
        if open { self.close() } else { self.open() }
    }

    fn open(&self) {
        let app = self.0.borrow().app.clone();
        // Snapshot the compositor event cache before taking keyboard focus. The IPC
        // lookup below runs on a worker; panel focus must not change its target.
        let focused = app.running.borrow().iter().find(|w| w.focused).map(|w| (w.id, w.app_id.clone()));
        let snapshot = focused.clone();
        let prism = kit::prefs::flag(&kit::prefs::read(), "spotlight_prism", false);
        self.0.borrow_mut().prism_on = prism;
        let pending = gio::spawn_blocking(move || {
            let instances = if prism { crate::prism::instances() } else { Vec::new() };
            let windows = compositor::windows();
            let focused = match snapshot {
                Some((id, app_id)) => Some((app_id, windows.iter().find(|w| w.id == id).map(|w| w.title.clone()))),
                None => windows.iter().find(|w| w.focused).map(|w| (w.app_id.clone(), Some(w.title.clone()))),
            };
            (compositor::focused_output(), windows, focused, instances)
        });
        let table = kit::prefs::read();
        let found_in = providers::discover(&app, &table);
        let scopes = providers::scopes(&app, &table);
        let settings = settings_index();
        {
            let mut s = self.0.borrow_mut();
            s.providers = found_in;
            s.scopes = scopes;
            s.settings = settings;
            s.found.clear();
            s.found_q.clear();
            s.opening = s.opening.wrapping_add(1);
            s.files_focus = focused.as_ref().is_some_and(|(_, id)| id.eq_ignore_ascii_case("dev.digit.Files"));
            s.files_folder = None;
            if s.scrim.is_none() {
                let monitor = s.window.monitor();
                let weak = Rc::downgrade(&self.0);
                s.scrim = Some(crate::paint::scrim(monitor.as_ref(), move || {
                    if let Some(s) = weak.upgrade() {
                        Launcher(s).close();
                    }
                }));
            }
            s.windows.clear();
            s.open = true;
            s.mode = Mode::All;
            let now = glib::monotonic_time();
            s.fade.go(1.0, now, 240, motion::STANDARD);
            s.rise.go(1.0, now, 560, SETTLE);
            s.window.set_keyboard_mode(KeyboardMode::Exclusive);
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
        self.update_caret();
        self.kick();
        let weak = Rc::downgrade(&self.0);
        let opening = self.0.borrow().opening;
        glib::spawn_future_local(async move {
            let Ok((output, windows, focused, instances)) = pending.await else { return };
            let Some(state) = weak.upgrade() else { return };
            {
                let mut s = state.borrow_mut();
                if !s.open || s.opening != opening { return; }
                s.files_focus = focused.as_ref().is_some_and(|(id, _)| id.eq_ignore_ascii_case("dev.digit.Files"));
                s.files_folder = focused.and_then(|(_, title)| title).filter(|t| !t.is_empty());
                s.windows = windows;
                s.prism = instances;
                if let Some(display) = gdk::Display::default() {
                    let monitors: Vec<gdk::Monitor> = display.monitors().iter::<gdk::Monitor>().filter_map(Result::ok).collect();
                    let monitor = monitors.iter().find(|m| output.as_deref().is_some_and(|o| m.connector().as_deref() == Some(o))).or(monitors.first());
                    if let Some(m) = monitor {
                        let top = (m.geometry().height() as f64 * 0.22 - PAD).max(0.0) as i32;
                        s.window.set_margin(Edge::Top, top);
                        s.window.set_monitor(Some(m));
                        if let Some(scrim) = &s.scrim { scrim.set_monitor(Some(m)); }
                    }
                }
            }
            Launcher(state).search();
        });
    }

    fn close(&self) {
        let (window, caret, scrim) = {
            let mut s = self.0.borrow_mut();
            if !s.open { return; }
            s.open = false;
            let now = glib::monotonic_time();
            s.fade.go(0.0, now, 200, motion::STANDARD);
            s.rise.go(0.0, now, 240, LEAVE);
            s.caret_shown = false;
            if let Some(id) = s.caret_blink.take() { id.remove(); }
            (s.window.clone(), s.caret_paint.clone(), s.scrim.take())
        };
        window.set_keyboard_mode(KeyboardMode::None);
        caret.queue_draw();
        if let Some(scrim) = scrim { scrim.destroy(); }
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
        // With the smooth caret on, GTK's own caret hides and the launcher draws one that glides.
        let caret = if s.smooth_caret() { "transparent".to_string() } else { draw::css(pal.accent) };
        s.css.load_from_string(&format!(
            "text.digit-field {{ background: none; border: none; box-shadow: none; padding: 0; min-height: 34px; font-family: \"{}\"; font-size: 22px; font-weight: 420; color: {}; caret-color: {}; }}
             text.digit-field > placeholder {{ color: {}; }}
             text.digit-field > selection {{ background-color: {}; color: {}; }}",
            s.app.cfg.font,
            draw::css(pal.ink_main),
            caret,
            draw::css(pal.ink_meta),
            draw::css(pal.accent.alpha(0.32)),
            draw::css(pal.ink_main),
        ));
    }

    pub fn refresh_caret(&self) {
        self.style();
        self.update_caret();
    }

    pub fn refresh_palette(&self) {
        if self.0.borrow().open {
            self.style();
            self.kick();
        }
    }

    fn update_caret(&self) {
        let paint = {
            let mut s = self.0.borrow_mut();
            let visible = s.open && s.smooth_caret() && s.field.has_focus();
            if !visible {
                s.caret_shown = false;
                if let Some(id) = s.caret_blink.take() { id.remove(); }
                let paint = s.caret_paint.clone();
                drop(s);
                paint.queue_draw();
                return;
            }
            let text = s.field.text();
            let position = s.field.position().max(0);
            if s.caret_measure.as_ref().is_none_or(|(t, p, _)| *t != text || *p != position) {
                let prefix: String = text.chars().take(position as usize).collect();
                let width = s.field.create_pango_layout(Some(&prefix)).pixel_size().0 as f64;
                s.caret_measure = Some((text, position, width));
                s.caret_moved = glib::monotonic_time();
            }
            s.retarget_caret(glib::monotonic_time());
            if s.caret_blink.is_none() {
                let weak = Rc::downgrade(&self.0);
                s.caret_blink = Some(glib::timeout_add_local(Duration::from_millis(50), move || {
                    let Some(state) = weak.upgrade() else { return glib::ControlFlow::Break };
                    let paint = {
                        let mut s = state.borrow_mut();
                        if !s.caret_shown || !s.open || !s.smooth_caret() || !s.field.has_focus() {
                            s.caret_blink = None;
                            return glib::ControlFlow::Break;
                        }
                        s.caret_paint.clone()
                    };
                    paint.queue_draw();
                    glib::ControlFlow::Continue
                }));
            }
            if std::mem::replace(&mut s.caret_ticking, true) {
                return;
            }
            s.caret_paint.clone()
        };
        let weak = Rc::downgrade(&self.0);
        paint.add_tick_callback(move |paint, _| {
            let Some(state) = weak.upgrade() else { return glib::ControlFlow::Break };
            let mut s = state.borrow_mut();
            let moving = s.caret_shown && s.open && s.smooth_caret() && s.field.has_focus() && !s.caret.settled(glib::monotonic_time());
            s.caret_ticking = moving;
            drop(s);
            paint.queue_draw();
            if moving { glib::ControlFlow::Continue } else { glib::ControlFlow::Break }
        });
    }

    fn step_selection(&self, by: i32) {
        let mut s = self.0.borrow_mut();
        let picks: Vec<usize> = s.lines.iter().enumerate().filter(|(_, l)| l.pickable()).map(|(i, _)| i).collect();
        if picks.is_empty() {
            return;
        }
        let at = picks.iter().position(|&i| i == s.selected).unwrap_or(0) as i32;
        let next = (at + by).clamp(0, picks.len() as i32 - 1) as usize;
        s.selected = picks[next];
        s.moved = true;
        s.reveal();
        s.glide_pill();
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
        (0..MODES.len())
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
                return line.pickable().then_some(i);
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
        let (field, mut mode) = { let s = self.0.borrow(); (s.field.clone(), s.mode) };
        let raw = field.text().to_string();
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
        // "pl …" lists PrismLauncher's instances, like the "set …" keyword lists settings; off
        // unless Settings has it on.
        let pl = raw.trim_start().strip_prefix("pl ").filter(|_| mode == Mode::All && !run && self.0.borrow().prism_on).map(|rest| rest.trim().to_string());
        let prism = pl.is_some();
        if let Some(rest) = pl {
            query = rest;
        }
        let q = fold(&query);
        let mut clips = Vec::new();
        let mut colors = Vec::new();
        if mode == Mode::Clip { clip_items(&mut clips, &mut colors, &query); }
        let mut s = self.0.borrow_mut();
        let apps = s.app.apps.list();
        let mut items: Vec<Item> = Vec::new();
        if prism {
            for (rank, ins) in s.prism.iter().enumerate() {
                if !q.is_empty() && score(&q, &prism_key(&ins.name)).is_none() {
                    continue;
                }
                if let Some(item) = prism_item(ins, 3000 - rank as i32) {
                    items.push(item);
                }
            }
            if items.is_empty() {
                let title = if s.prism.is_empty() { "No instances found".to_string() } else { format!("No instances match “{query}”") };
                items.push(Item { kind: Kind::Prism, act: Act::Spawn(crate::prism::open().unwrap_or_default()), title, detail: "Open Prism Launcher".into(), look: Look::Symbol("deployed_code"), score: 0 });
            }
        } else if run {
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
                let mut hits: Vec<_> = apps.iter().enumerate().filter(|(_, e)| !s.app.hidden(&e.id)).filter_map(|(i, e)| {
                    let hit = e.keys.iter().filter_map(|(k, bias)| score(&q, k).map(|v| v + bias)).max()?;
                    let bonus = if q.is_empty() { 0 } else { s.app.apps.boost(&e.id) };
                    Some((i, hit + bonus))
                }).collect();
                let cap = if mode == Mode::All { 7 } else { 200 };
                limit_hits(&mut hits, cap, |a, b| {
                    let order = if q.is_empty() && mode == Mode::Apps {
                        apps[a.0].name.to_lowercase().cmp(&apps[b.0].name.to_lowercase())
                    } else { b.1.cmp(&a.1) };
                    order.then(a.0.cmp(&b.0))
                });
                for (i, hit) in hits {
                    let e = &apps[i];
                    items.push(Item { kind: Kind::App, act: Act::App(i), title: e.name.clone(), detail: e.detail.clone(), look: look(e), score: hit });
                }
            }
            // With PrismLauncher on in Settings, instances matching what's typed show among the
            // apps too, not only after "pl".
            if mode == Mode::All && s.prism_on && !q.is_empty() {
                let mut hits: Vec<(usize, i32)> = s.prism.iter().enumerate().filter_map(|(i, ins)| score(&q, &prism_key(&ins.name)).map(|h| (i, h))).collect();
                hits.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                for (i, hit) in hits.into_iter().take(5) {
                    if let Some(item) = prism_item(&s.prism[i], hit) {
                        items.push(item);
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
                    // "set …" lists matching categories and controls above the search in Settings.
                    if matches!(sc.reach, providers::Reach::Settings) {
                        for (rank, (_, set)) in find_settings(&s.settings, &fold(&rest), 12).into_iter().enumerate() {
                            items.push(set.item(2700 - rank as i32));
                        }
                    }
                    items.push(Item {
                        kind: Kind::Scope,
                        act: Act::Scope(i, rest.clone()),
                        title: format!("Search {} for “{}”", sc.name, rest),
                        detail: if matches!(sc.reach, providers::Reach::Files) { scope_detail(sc, s.files_focus, s.files_folder.as_deref()) } else { sc.key.clone() },
                        look,
                        score: scope_score(matches!(sc.reach, providers::Reach::Files), s.files_focus, true, i),
                    });
                } else if query.chars().count() >= 2 {
                    for (i, sc) in s.scopes.iter().enumerate().filter(|(_, sc)| sc.app || matches!(sc.reach, providers::Reach::Store)) {
                        let look = sc.icon.clone().map(Look::Icon).unwrap_or(Look::Symbol(sc.symbol));
                        items.push(Item {
                            kind: Kind::Scope,
                            act: Act::Scope(i, query.clone()),
                            title: format!("Search {} for “{}”", sc.name, query),
                            detail: scope_detail(sc, s.files_focus, s.files_folder.as_deref()),
                            look,
                            score: scope_score(matches!(sc.reach, providers::Reach::Files), s.files_focus, false, i),
                        });
                    }
                }
                for (key, title, symbol, keys) in POWER {
                    let hit = keys.split_whitespace().filter_map(|k| score(&q, k)).max().or_else(|| score(&q, &fold(title)));
                    if let Some(hit) = hit.filter(|_| q.chars().count() >= 3) {
                        items.push(Item { kind: Kind::Settings, act: Act::Spawn(vec!["digitctl".into(), "key".into(), key.into()]), title: title.into(), detail: "System".into(), look: Look::Symbol(symbol), score: hit - 180 });
                    }
                }
                // Single settings too, under the actions and apps they share words with.
                if q.chars().count() >= 3 {
                    for (hit, set) in find_settings(s.settings.iter().filter(|s| !s.page), &q, 3) {
                        items.push(set.item(hit - 350));
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
            items = clips;
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
        } else if mode == Mode::Clip {
            // Each color takes its own row, at its place among the other copies.
            let n = items.len().min(200);
            let mut next = colors.iter().enumerate().peekable();
            for i in 0..=n {
                while let Some((c, _)) = next.next_if(|(_, (at, _))| *at <= i) {
                    lines.push(Line::Palette(c));
                }
                if i < n {
                    lines.push(Line::Item(i));
                }
            }
        } else {
            lines.extend((0..items.len().min(200)).map(Line::Item));
        }
        let placeholder = MODES.iter().find(|m| m.0 == mode).map(|m| m.2).unwrap_or("Search");
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
        s.colors = colors.into_iter().map(|(_, c)| c).collect();
        s.card_hot = None;
        s.clear_on = mode == Mode::Clip && q.is_empty() && !crate::clip::history().is_empty();
        s.selected = s.lines.iter().position(Line::pickable).unwrap_or(0);
        s.place_pill();
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
        field.set_placeholder_text(Some(placeholder));
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
                s.place_pill();
            }
        }
    }

    fn copy_and_close(&self, text: &str) {
        self.0.borrow_mut().lines.clear();
        self.close();
        if let Some(display) = gdk::Display::default() {
            display.clipboard().set_text(text);
        }
    }

    /// The color row under the pointer, and which part of it.
    fn card_hit(&self, x: f64, y: f64) -> Option<(usize, CardHit)> {
        let s = self.0.borrow();
        if !s.open || y < PANEL_TOP || y > PANEL_TOP + s.panel.target() {
            return None;
        }
        let mut top = PANEL_TOP + INSET - s.scroll.target();
        for line in &s.lines {
            if let Line::Palette(c) = line {
                let g = CardGeometry::new(top);
                if g.card.contains(x, y) {
                    let part = g.formats.iter().position(|r| r.contains(x, y)).map_or(CardHit::Body, CardHit::Format);
                    return Some((*c, part));
                }
            }
            top += line.height();
        }
        None
    }

    fn clear_center(panel: f64) -> (f64, f64) {
        (PAD + CLEAR / 2.0, PANEL_TOP + panel + GAP + CLEAR / 2.0)
    }

    fn clear_hit(&self, x: f64, y: f64) -> bool {
        let s = self.0.borrow();
        let (cx, cy) = Self::clear_center(s.panel.target());
        s.open && s.clear_on && (x - cx).hypot(y - cy) <= CLEAR / 2.0 + 2.0
    }

    /// A click on a color row copies the color, or the format whose chip was hit.
    fn click_card(&self, (at, hit): (usize, CardHit)) {
        let text = {
            let s = self.0.borrow();
            let Some(c) = s.colors.get(at) else { return };
            match hit {
                CardHit::Format(i) => c.color.formats().get(i).map(|f| f.1.clone()),
                CardHit::Body => Some(c.text.clone()),
            }
        };
        if let Some(t) = text {
            self.copy_and_close(&t);
        }
    }

    fn activate(&self, line: Option<usize>) {
        let (act, app) = {
            let mut s = self.0.borrow_mut();
            let at = line.unwrap_or(s.selected);
            if let Some(Line::Palette(c)) = s.lines.get(at) {
                let Some(c) = s.colors.get(*c) else { return };
                let text = c.text.clone();
                drop(s);
                self.copy_and_close(&text);
                return;
            }
            let Some(Line::Item(i)) = s.lines.get(at) else { return };
            let i = *i;
            if i >= s.items.len() {
                return;
            }
            let item = s.items.swap_remove(i);
            s.lines.clear();
            (item.act, s.app.clone())
        };
        let (list, scopes, terms, files_focus) = {
            let s = self.0.borrow();
            (s.providers.clone(), s.scopes.clone(), s.terms.clone(), s.files_focus)
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
                Some(providers::Reach::Switcher { ids, entry }) => {
                    providers::switcher(app.clone(), ids, entry, query)
                }
                Some(providers::Reach::Files) => {
                    let flag = if files_focus { "--search" } else { "--search-home" };
                    let argv: [&std::ffi::OsStr; 3] = ["digit-files".as_ref(), flag.as_ref(), query.as_ref()];
                    let _ = gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE);
                }
                Some(providers::Reach::Settings) => {
                    let arg = format!("?{}", query.chars().filter(|c| !c.is_control()).take(80).collect::<String>());
                    let argv: [&std::ffi::OsStr; 2] = ["digit-settings".as_ref(), arg.as_ref()];
                    let _ = gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE);
                }
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
                    let _ = gio::spawn_blocking(move || compositor::focus_after_panel(id)).await;
                });
            }
            Act::Copy(text) => {
                if let Some(display) = gdk::Display::default() {
                    display.clipboard().set_text(&text);
                }
            }
            Act::Clip(c) => {
                if let crate::clip::Data::Text(text) = &c.data && let Some(display) = gdk::Display::default() {
                    display.clipboard().set_text(text);
                }
            }
            Act::Image(c) => {
                if let crate::clip::Data::Image(png) = &c.data && let Some(display) = gdk::Display::default() {
                    let bytes = glib::Bytes::from_owned(png.clone());
                    let provider = gdk::ContentProvider::for_bytes("image/png", &bytes);
                    let _ = display.clipboard().set_content(Some(&provider));
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

    /// Where the selected line sits in the list, and how tall it is.
    fn pill_target(&self) -> (f64, f64) {
        let top = self.lines.iter().take(self.selected).map(Line::height).sum();
        (top, self.lines.get(self.selected).map_or(ROW, Line::height))
    }

    /// The selection moved within the same results: the pill slides there.
    fn glide_pill(&mut self) {
        let ((y, h), now) = (self.pill_target(), glib::monotonic_time());
        self.pill_y.go(y, now, 150, motion::STANDARD);
        self.pill_h.go(h, now, 150, motion::STANDARD);
    }

    /// New results: nothing to slide from, so the pill starts on the selected line.
    fn place_pill(&mut self) {
        let (y, h) = self.pill_target();
        self.pill_y.set(y);
        self.pill_h.set(h);
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
        } else {
            let h = self.lines.get(self.selected).map_or(ROW, Line::height);
            if top + h > to + view {
                to = top + h - view;
            }
        }
        self.scroll.go(to.max(0.0), glib::monotonic_time(), 140, motion::STANDARD);
    }

    fn smooth_caret(&self) -> bool {
        motion::enabled(self.app.prefs.borrow().smooth_caret)
    }

    fn retarget_caret(&mut self, now: i64) {
        let Some((_, _, width)) = &self.caret_measure else { return };
        // Long text scrolls inside the field; the caret then rides its right edge.
        let x = FIELD_X + width.min((self.field_w as f64 - 2.0).max(0.0));
        if !self.caret_shown {
            self.caret.set(x);
            self.caret_shown = true;
            self.caret_moved = now;
        } else if (x - self.caret.target()).abs() > 0.1 {
            self.caret.go(x, now, 110, CARET);
            self.caret_moved = now;
        }
    }

    fn draw_caret(&self, snapshot: &gtk::Snapshot, now: i64) {
        if !self.caret_shown || !self.open || !self.smooth_caret() || !self.field.has_focus() {
            return;
        }
        let idle = (now - self.caret_moved) as f64 / 1e6 - 0.5;
        let alpha = if idle <= 0.0 { 1.0 } else { 0.5 + 0.5 * (idle * std::f64::consts::TAU / 1.1).cos() };
        let bounds = rounded(self.caret.value(now) - 1.0, 0.0, 2.0, 26.0, 1.0);
        draw::fill(snapshot, &bounds, self.app.palette_at(now).accent.alpha(alpha));
    }

    fn step(&mut self, now: i64) -> bool {
        let fade = self.fade.value(now).clamp(0.0, 1.0);
        self.field.set_opacity(fade);
        self.caret_paint.set_opacity(fade);
        let rise = self.rise.value(now);
        let dy = if self.open { -10.0 } else { -6.0 } * (1.0 - rise);
        self.lift.set(1.0, dy);
        let want = (self.bar.value(now) - 54.0 - 22.0).max(40.0) as i32;
        if want != self.field_w {
            self.field.set_size_request(want, 34);
            self.field_w = want;
            if self.caret_shown {
                self.retarget_caret(now);
            }
        }
        let rect = if self.open { (0, 0, SURFACE_W as i32, SURFACE_H as i32) } else { (0, 0, 1, 1) };
        if rect != self.region {
            if let Some(surface) = self.window.surface() {
                surface.set_input_region(Some(&cairo::Region::create_rectangle(&cairo::RectangleInt::new(rect.0, rect.1, rect.2, rect.3))));
                self.region = rect;
            }
        }
        if self.caret_shown && !self.caret.settled(now) {
            self.caret_paint.queue_draw();
        }
        let moving = [&self.fade, &self.rise, &self.panel, &self.bar, &self.bubbles, &self.scroll, &self.pill_y, &self.pill_h, &self.tip].iter().any(|t| !t.settled(now))
            || self.glow.iter().any(|t| !t.settled(now))
            || (self.caret_shown && !self.caret.settled(now))
            || !self.clear_glow.settled(now)
            || self.app.palette_moving(now);
        if !moving && !self.open && self.window.is_visible() {
            let window = self.window.clone();
            glib::idle_add_local_once(move || {
                if window.keyboard_mode() == KeyboardMode::None {
                    window.set_visible(false);
                }
            });
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
                let grow = if motion::reduced() { 0.0 } else { 1.5 * glow };
                let fill = if on { pal.accent } else { pal.tile(0.09 * glow) };
                // The shadow keeps the resting size: a blurred shadow whose size changes every frame
                // gets re-rendered by the GL renderer and flickers, so only the face grows on hover.
                shadow(s, &rounded(x, PAD, BUBBLE, BUBBLE, BUBBLE / 2.0));
                face(s, &rounded(x - grow, PAD - grow, BUBBLE + 2.0 * grow, BUBBLE + 2.0 * grow, BUBBLE / 2.0 + grow), fill);
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
            if self.clear_on {
                // Drawn like the mode bubbles: same size, shadow, hover growth and tint.
                let (cx, cy) = Launcher::clear_center(height);
                let glow = self.clear_glow.value(now).clamp(0.0, 1.0);
                let grow = if motion::reduced() { 0.0 } else { 1.5 * glow };
                let (x, y) = (cx - BUBBLE / 2.0, cy - BUBBLE / 2.0);
                shadow(s, &rounded(x, y, BUBBLE, BUBBLE, BUBBLE / 2.0));
                face(s, &rounded(x - grow, y - grow, BUBBLE + 2.0 * grow, BUBBLE + 2.0 * grow, BUBBLE / 2.0 + grow), pal.tile(0.09 * glow));
                draw::icon(s, &canvas, &cfg, "delete_sweep", 24.0, false, pal.ink_main.mix(pal.ink_sec, glow), cx, cy);
            }
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

    fn draw_card(&mut self, s: &gtk::Snapshot, pal: &Palette, top: f64, at: usize, covered: f64) {
        let cfg = self.app.cfg.clone();
        let canvas = self.canvas.clone();
        let Some(c) = self.colors.get(at).cloned() else { return };
        let g = CardGeometry::new(top);
        let hot = self.card_hot.filter(|(i, _)| *i == at).map(|(_, h)| h);
        // Selected like any other row: the pill sits under it, with its own ink on top.
        let on = covered > 0.5;
        let title_ink = pal.on_accent.mix(pal.ink_main, covered);
        let detail_ink = pal.on_accent.mix(pal.accent, 0.72).mix(pal.ink_meta, covered);
        let chip_ink = title_ink;
        let sw = rounded(g.swatch.x, g.swatch.y, g.swatch.w, g.swatch.h, 8.0);
        s.push_rounded_clip(&sw);
        s.append_color(&gdk::RGBA::new(c.color.r as f32, c.color.g as f32, c.color.b as f32, c.color.a as f32), sw.bounds());
        s.pop();
        let edge = if on { pal.on_accent.alpha(0.3).rgba() } else { gdk::RGBA::new(1.0, 1.0, 1.0, 0.22) };
        s.append_border(&sw, &[1.0; 4], &[edge, edge, edge, edge]);
        let mid = g.card.y + g.card.h / 2.0;
        let x = PAD + INSET + 52.0;
        let text_w = g.formats[0].x - x - 12.0;
        draw::text(s, &canvas, &cfg, &c.color.hex(), 15.0, Face::Text(if on { 560 } else { 480 }), title_ink, x, mid - 9.0, text_w, pango::Alignment::Left);
        // The second line shows what a chip would copy, else what was copied and when.
        let formats = c.color.formats();
        let line = match hot {
            Some(CardHit::Format(i)) => formats.get(i).map(|f| f.1.clone()).unwrap_or_default(),
            _ => format!("{} · {}", c.text, ago(c.at)),
        };
        draw::text(s, &canvas, &cfg, &line, 12.5, Face::Text(420), detail_ink, x, mid + 10.0, text_w, pango::Alignment::Left);
        for (i, r) in g.formats.iter().enumerate() {
            let lit = hot == Some(CardHit::Format(i));
            let chip = rounded(r.x, r.y, r.w, r.h, r.h / 2.0);
            s.push_rounded_clip(&chip);
            s.append_color(&chip_ink.alpha(if lit { 0.2 } else { 0.08 }).rgba(), chip.bounds());
            s.pop();
            let label = formats.get(i).map(|f| f.0).unwrap_or("");
            draw::text(s, &canvas, &cfg, label, 12.0, Face::Text(520), chip_ink, r.x, r.y + r.h / 2.0, r.w, pango::Alignment::Center);
        }
    }

    fn draw_lines(&mut self, s: &gtk::Snapshot, pal: &Palette, now: i64, height: f64) {
        let cfg = self.app.cfg.clone();
        let canvas = self.canvas.clone();
        let mut top = PANEL_TOP + INSET - self.scroll.value(now).round();
        let bottom = PANEL_TOP + height;
        // One pill for the selection, under the rows, so it can slide between them.
        let picked = self.lines.get(self.selected).is_some_and(Line::pickable);
        let (pill_top, pill_h) = (top + self.pill_y.value(now).round(), self.pill_h.value(now).round());
        if picked {
            let pill = rounded(PAD + INSET, pill_top, WIDTH - 2.0 * INSET, pill_h, PANEL_R - INSET);
            s.push_rounded_clip(&pill);
            s.append_color(&pal.accent.rgba(), pill.bounds());
            s.pop();
        }
        // How much of a line the pill covers: its ink follows, so text stays readable mid-slide.
        let covered = |top: f64, h: f64| if picked { (((top + h).min(pill_top + pill_h) - top.max(pill_top)) / h.min(pill_h).max(1.0)).clamp(0.0, 1.0) } else { 0.0 };
        for li in 0..self.lines.len() {
            let h = self.lines[li].height();
            if top + h < PANEL_TOP || top > bottom {
                top += h;
                continue;
            }
            match self.lines[li] {
                Line::Palette(c) => self.draw_card(s, pal, top, c, covered(top, h)),
                Line::Head(ref title) => {
                    let title = title.clone();
                    draw::text(s, &canvas, &cfg, &title, 12.5, Face::Text(560), pal.ink_meta, PAD + INSET + 14.0, top + HEAD / 2.0 + 3.0, WIDTH - 60.0, pango::Alignment::Left);
                }
                Line::Item(i) => {
                    let lit = covered(top, ROW);
                    let on = lit > 0.5;
                    let mid = top + ROW / 2.0;
                    let title_ink = pal.on_accent.mix(pal.ink_main, lit);
                    let detail_ink = pal.on_accent.mix(pal.accent, 0.72).mix(pal.ink_meta, lit);
                    let symbol_ink = pal.on_accent.mix(pal.ink_sec, lit);
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

#[derive(Clone, Copy)]
struct Area {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Area {
    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// Where everything on a color row goes, shared by drawing and clicking.
struct CardGeometry {
    card: Area,
    swatch: Area,
    formats: Vec<Area>,
}

impl CardGeometry {
    fn new(top: f64) -> Self {
        let card = Area { x: PAD + INSET, y: top, w: WIDTH - 2.0 * INSET, h: CARD_H };
        let mid = card.y + card.h / 2.0;
        // The swatch sits where other rows keep their icon.
        let swatch = Area { x: PAD + INSET + 12.0, y: mid - 14.0, w: 28.0, h: 28.0 };
        // Format chips line up on the right, where other rows keep their detail.
        let widths = [44.0, 44.0, 44.0, 44.0, 58.0, 56.0];
        let total: f64 = widths.iter().sum::<f64>() + 6.0 * (widths.len() - 1) as f64;
        let mut x = card.x + card.w - 12.0 - total;
        let formats = widths
            .iter()
            .map(|w| {
                let r = Area { x, y: mid - 12.0, w: *w, h: 24.0 };
                x += w + 6.0;
                r
            })
            .collect();
        CardGeometry { card, swatch, formats }
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

#[cfg(test)]
mod tests {
    use super::{Act, Setting, find_settings};

    fn dock_settings() -> Vec<Setting> {
        serde_json::from_value(serde_json::json!([
            { "title": "Dock", "place": "Dock", "words": "Show or hide the dock", "icon": "dock_to_bottom", "link": "dock/dock/dock", "page": false },
            { "title": "Icon size", "place": "Dock", "words": "", "icon": "apps", "link": "dock/icon-size/dock", "page": false },
            { "title": "Dock", "place": "Dock", "words": "", "icon": "dock_to_bottom", "link": "dock", "page": true }
        ])).unwrap()
    }

    #[test]
    fn category_precedes_its_matching_controls() {
        let settings = dock_settings();
        let hits = find_settings(&settings, "dock", 12);
        assert_eq!(hits.len(), 3);
        assert!(hits[0].1.page);
        assert!(hits.iter().skip(1).all(|(_, s)| !s.page));
        let item = hits[0].1.item(0);
        assert_eq!(item.detail, "Settings");
        match item.act {
            Act::Spawn(args) => assert_eq!(args, ["digit-settings", "dock"]),
            _ => panic!("a settings category should open its page"),
        }
    }

    #[test]
    fn specific_control_query_still_finds_the_control() {
        let settings = dock_settings();
        let hits = find_settings(&settings, "dock icon", 12);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].1.link, "dock/icon-size/dock");
    }
}

// Keep source order for equal-ranked items in the final stable sort.
fn limit_hits(hits: &mut Vec<(usize, i32)>, cap: usize, compare: impl FnMut(&(usize, i32), &(usize, i32)) -> std::cmp::Ordering) {
    if hits.len() > cap {
        hits.select_nth_unstable_by(cap, compare);
        hits.truncate(cap);
        hits.sort_unstable_by_key(|h| h.0);
    }
}

#[cfg(test)]
mod search_limit_tests {
    use super::*;

    #[test]
    fn selects_best_before_building_items_and_preserves_ties() {
        let mut hits = vec![(0, 4), (1, 9), (2, 9), (3, 1)];
        limit_hits(&mut hits, 2, |a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        assert_eq!(hits, vec![(1, 9), (2, 9)]);
        limit_hits(&mut hits, 0, |a, b| b.1.cmp(&a.1));
        assert!(hits.is_empty());
    }
}

// Exact app names score 1000; substring/weak fuzzy matches score at most 600.
fn scope_score(files: bool, focused: bool, keyed: bool, index: usize) -> i32 {
    if keyed { 2600 } else if files && focused { 850 } else { -800 - index as i32 }
}

fn scope_detail(scope: &providers::Scope, focused: bool, folder: Option<&str>) -> String {
    if matches!(scope.reach, providers::Reach::Files) {
        if focused {
            if let Some(folder) = folder { return format!("Search in {folder}"); }
        } else { return "Search in Home".into(); }
    }
    format!("{} …", scope.key)
}

#[cfg(test)]
mod files_scope_tests {
    use super::*;

    #[test]
    fn focused_files_ranks_between_exact_names_and_poor_matches() {
        let rank = scope_score(true, true, false, 3);
        assert!(rank < score("files", "files").unwrap());
        assert!(rank > score("edit", "credits").unwrap());
        assert!(rank > score("ffx", "firefox").unwrap());
        assert_eq!(rank, 850);
    }

    #[test]
    fn unfocused_files_stays_low_until_keyed() {
        assert_eq!(scope_score(true, false, false, 3), -803);
        assert_eq!(scope_score(true, false, true, 3), 2600);
        assert_eq!(scope_score(true, true, true, 3), 2600);
        assert_eq!(scope_score(false, true, false, 3), -803);
    }
}
