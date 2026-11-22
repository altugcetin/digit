use crate::app::App;
use crate::draw::{self, Face};
use crate::paint::Paint;
use crate::session::{self, Act};
use gtk::glib::translate::{FromGlibPtrFull, ToGlibPtr};
use gtk::{gdk, gio, glib, graphene, pango, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use kit::palette::Rgb;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;
use kit::secret::wipe;

mod ffi {
    use gtk::glib::ffi::gboolean;
    use gtk::glib::gobject_ffi::GObject;

    #[link(name = "gtk4-layer-shell")]
    unsafe extern "C" {
        pub fn gtk_session_lock_is_supported() -> gboolean;
        pub fn gtk_session_lock_instance_new() -> *mut GObject;
        pub fn gtk_session_lock_instance_lock(instance: *mut GObject) -> gboolean;
        pub fn gtk_session_lock_instance_unlock(instance: *mut GObject);
        pub fn gtk_session_lock_instance_assign_window_to_monitor(instance: *mut GObject, window: *mut gtk::ffi::GtkWindow, monitor: *mut gtk::gdk::ffi::GdkMonitor);
    }
}

enum Mode {
    Real(glib::Object),
    Preview,
}

#[derive(Clone)]
struct Sheet {
    monitor: gdk::Monitor,
    window: gtk::Window,
    paint: Paint,
    column: gtk::Box,
    actions: gtk::Box,
    entry: gtk::PasswordEntry,
    status: gtk::Label,
    /// The main screen's sheet holds the clock, the password and the buttons; the others show
    /// only the wallpaper and pass what is typed on them to it.
    main: Rc<Cell<bool>>,
    /// When unlocking started to clear this screen into the desktop (see [`reveal`]).
    reveal: Rc<Cell<Option<i64>>>,
}

/// How long unlocking takes to clear the wallpaper before the screen unlocks, and then for the
/// clock to settle onto the desktop.
const CLEAR_MS: i64 = 240;
const SETTLE_MS: i64 = 600;
/// How long the rest of the desktop takes to fade in once unlocked.
const FADE_MS: i64 = 460;

/// `x` eased in and out, 0 to 1.
fn smooth(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// How far something `ms` long that started at `start` got at `now`, 0 to 1.
fn part(start: i64, now: i64, ms: i64) -> f64 {
    ((now - start) as f64 / (ms * 1000) as f64).clamp(0.0, 1.0)
}

/// The middles and sizes of the lock screen's date and time on a screen `w` by `h`, `fade` of
/// the way in.
fn lock_lines(w: f64, h: f64, fade: f64) -> ((f64, f64, f64), (f64, f64, f64)) {
    let top = h * 0.14 + (1.0 - fade) * 10.0;
    ((w / 2.0, top, 22.0), (w / 2.0, top + 22.0 + (h * 0.06).clamp(42.0, 88.0), (h * 0.105).clamp(72.0, 150.0)))
}

/// The lock screen's date and time, white, their middles and sizes `date_at` and `time_at`; with
/// a soft shadow unless `flat`.
#[allow(clippy::too_many_arguments)]
fn lines(s: &gtk::Snapshot, p: &Paint, cfg: &crate::config::Config, w: f64, (date_at, time_at): ((f64, f64, f64), (f64, f64, f64)), date_shown: f64, flat: bool) {
    let dt = glib::DateTime::now_local().ok();
    let date = dt.as_ref().and_then(|d| d.format("%A, %-d %B").ok()).map(|s| s.to_string()).unwrap_or_default();
    let time = dt.as_ref().and_then(|d| d.format(&cfg.clock).ok()).map(|s| s.to_string()).unwrap_or_default();
    let white = Rgb::new(1.0, 1.0, 1.0);
    if !flat { s.push_shadow(&[gtk::gsk::Shadow::new(gdk::RGBA::new(0.0, 0.0, 0.0, 0.25), 0.0, 1.0, 12.0)]); }
    draw::text(s, p, cfg, &date, date_at.2, Face::Text(560), white.alpha(0.9 * date_shown), date_at.0 - w, date_at.1, 2.0 * w, pango::Alignment::Center);
    draw::text(s, p, cfg, &time, time_at.2, Face::Text(420), white.alpha(0.96), time_at.0 - w, time_at.1, 2.0 * w, pango::Alignment::Center);
    if !flat { s.pop(); }
}

/// Unlocking a bare screen: a surface of its own above the whole desktop shows the wallpaper as
/// the lock screen left it, cleared, with its clock (`clock`: on the main screen). It waits so
/// until `go` holds when it may start; then the wallpaper fades for the desktop to come in from
/// under it, the bar and the widgets as they are, and the clock settles where the desktop's
/// sits, which shows once it's there. Made before the screen unlocks, so it's there the moment
/// it does.
fn settle(app: &Rc<App>, monitor: &gdk::Monitor, go: Rc<Cell<Option<i64>>>, clock: bool) {
    let output = monitor.connector().map(|c| c.to_string()).unwrap_or_default();
    let window = gtk::Window::new();
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace(Some("digit-unlock"));
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(KeyboardMode::None);
    window.set_monitor(Some(monitor));
    for e in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(e, true);
    }
    window.add_css_class("digit");
    let paint = Paint::new(-1, -1);
    paint.set_hexpand(true);
    paint.set_vexpand(true);
    let held = clock && crate::desk::hold(&output, true);
    let spot: Rc<RefCell<Option<Option<crate::desk::Spot>>>> = Rc::default();
    let sharp: RefCell<Option<Option<(gdk::Texture, graphene::Rect)>>> = RefCell::new(None);
    {
        let (weak, a, output, go, spot) = (paint.downgrade(), Rc::downgrade(app), output.clone(), go.clone(), spot.clone());
        paint.on_draw(move |s| {
            let (Some(p), Some(app)) = (weak.upgrade(), a.upgrade()) else { return };
            let (w, h) = (p.width() as f64, p.height() as f64);
            if w < 2.0 || h < 2.0 {
                return;
            }
            let now = p.frame_clock().map(|c| c.frame_time()).unwrap_or_else(glib::monotonic_time);
            let (b, veil) = match go.get() {
                Some(start) if start != i64::MAX => (part(start, now, SETTLE_MS), 1.0 - smooth(part(start, now, FADE_MS))),
                _ => (0.0, 1.0),
            };
            // The wallpaper the lock screen cleared into, over the desktop until it fades.
            if veil > 0.0 {
                let full = graphene::Rect::new(0.0, 0.0, w as f32, h as f32);
                if let Some((tex, rect)) = sharp.borrow_mut().get_or_insert_with(|| app.wallpapers.sharp(&output, w, h)).clone() {
                    s.push_clip(&full);
                    s.push_opacity(veil);
                    s.append_color(&gdk::RGBA::BLACK, &full);
                    s.append_texture(&tex, &rect);
                    s.pop();
                    s.pop();
                }
            }
            if !clock {
                return;
            }
            let mut slot = spot.borrow_mut();
            let spot = slot.get_or_insert_with(|| if held { crate::desk::spot(&app, &p, &output, (w as i32, h as i32)) } else { None });
            let (mut date_at, mut time_at) = lock_lines(w, h, 1.0);
            let m = kit::motion::STANDARD.at(b);
            let mut date_shown = 1.0;
            let shown = match spot.as_ref() {
                Some(spot) => {
                    time_at = toward(time_at, spot.time, m);
                    match spot.date {
                        Some(d) => date_at = toward(date_at, d, m),
                        None => {
                            date_shown = 1.0 - smooth(b / 0.4);
                            date_at.1 -= 24.0 * m;
                        }
                    }
                    1.0 - smooth((b - 0.3) / 0.55)
                }
                // No clock on the desktop: this one rises away.
                None => {
                    time_at.1 -= 40.0 * m;
                    date_at.1 -= 40.0 * m;
                    1.0 - smooth(b / 0.55)
                }
            };
            if shown > 0.0 {
                s.push_opacity(shown);
                lines(s, &p, &app.cfg, w, (date_at, time_at), date_shown, kit::effects::reduced());
                s.pop();
            }
            // The desktop's clock, as its own surface draws it, comes in where it sits.
            if let Some(spot) = spot.as_ref() {
                let desk = smooth((b - 0.25) / 0.6);
                if desk > 0.0 {
                    s.push_opacity(desk);
                    crate::desk::paint_at(s, &p, &app, &output, spot);
                    s.pop();
                }
            }
        });
    }
    window.set_child(Some(&paint));
    window.connect_realize(|w| {
        if let Some(surface) = w.surface() {
            surface.set_input_region(Some(&gtk::cairo::Region::create()));
        }
    });
    window.present();
    // Until it may start, then on its way; once there, the desktop's own clock shows again and
    // this goes a moment later, the two drawing the same meanwhile.
    let (w, o) = (window.clone(), output.clone());
    let done = Rc::new(Cell::new(false));
    paint.add_tick_callback(move |p, frames| {
        p.queue_draw();
        let now = frames.frame_time();
        let start = match go.get() {
            None => return glib::ControlFlow::Continue,
            Some(i64::MAX) => {
                go.set(Some(now));
                now
            }
            Some(t) => t,
        };
        let over = if clock { part(start, now, SETTLE_MS.max(FADE_MS)) } else { part(start, now, FADE_MS) };
        if over < 1.0 || done.replace(true) {
            return glib::ControlFlow::Continue;
        }
        if clock {
            crate::desk::hold(&o, false);
        }
        let w = w.clone();
        glib::timeout_add_local_once(Duration::from_millis(160), move || w.destroy());
        glib::ControlFlow::Break
    });
    // Whatever happens to unlocking, nothing stays behind.
    let (w, o) = (window.downgrade(), output);
    glib::timeout_add_local_once(Duration::from_secs(4), move || {
        if let Some(w) = w.upgrade() {
            crate::desk::hold(&o, false);
            w.destroy();
        }
    });
}

/// `a` on its way to `b`, `t` of the way.
fn toward(a: (f64, f64, f64), b: (f64, f64, f64), t: f64) -> (f64, f64, f64) {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, a.2 + (b.2 - a.2) * t)
}

/// Whether `monitor` is the main screen (see `kit::screens::primary`).
fn is_main(app: &App, monitor: &gdk::Monitor) -> bool {
    app.primary().is_none_or(|m| m == *monitor)
}

/// Makes `face` the one with the clock and the password, as when the main screen goes away.
fn promote(face: &Sheet) {
    face.main.set(true);
    face.column.set_visible(true);
    face.actions.set_visible(true);
    face.paint.queue_draw();
    face.entry.grab_focus();
}

/// The main screen's sheet.
fn main_face() -> Option<Sheet> {
    STATE.with(|s| s.borrow().as_ref().and_then(|st| st.faces.iter().find(|f| f.main.get()).cloned()))
}

struct State {
    generation: u64,
    app: std::rc::Weak<App>,
    mode: Mode,
    faces: Vec<Sheet>,
    busy: bool,
    locked: bool,
    clock: Option<glib::SourceId>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    static GENERATION: Cell<u64> = const { Cell::new(0) };
    static FALLBACK: RefCell<Option<std::sync::Arc<std::sync::atomic::AtomicBool>>> = const { RefCell::new(None) };
    static STYLED: Cell<bool> = const { Cell::new(false) };
}

pub fn active() -> bool {
    STATE.with(|s| s.borrow().as_ref().is_some_and(|st| matches!(st.mode, Mode::Real(_))))
}

fn showing() -> bool {
    STATE.with(|s| s.borrow().is_some())
}

fn next_generation() -> u64 {
    GENERATION.with(|g| {
        let next = g.get().checked_add(1).expect("lock generation exhausted");
        g.set(next);
        next
    })
}

fn current(generation: u64) -> bool {
    STATE.with(|s| s.borrow().as_ref().is_some_and(|st| st.generation == generation))
}

pub fn supported() -> bool {
    // SAFETY: takes nothing and only asks the compositor connection GTK already has.
    unsafe { ffi::gtk_session_lock_is_supported() != 0 }
}

fn style(app: &App) {
    if STYLED.with(|s| s.replace(true)) {
        return;
    }
    let Some(display) = gdk::Display::default() else { return };
    let font = &app.cfg.font;
    let icons = &app.cfg.icon_font;
    let css = gtk::CssProvider::new();
    css.load_from_string(&format!(
        "window.digit-lock {{ background: black; }}
        .lock-name {{ font-family: \"{font}\"; font-size: 18px; font-weight: 600; color: white; }}
        .lock-entry {{ font-family: \"{font}\"; font-size: 15px; min-height: 40px; min-width: 260px; border-radius: 20px; padding: 0 16px; color: white; background: rgba(255,255,255,0.16); border: 1px solid rgba(255,255,255,0.16); box-shadow: none; outline: none; caret-color: white; }}
        .lock-entry:focus-within {{ background: rgba(255,255,255,0.22); border-color: rgba(255,255,255,0.32); }}
        .lock-entry text placeholder {{ color: rgba(255,255,255,0.6); }}
        .lock-entry image {{ color: rgba(255,255,255,0.8); }}
        .lock-status {{ font-family: \"{font}\"; font-size: 13px; color: rgba(255,255,255,0.72); min-height: 18px; }}
        .lock-status.error {{ color: #ffb4ab; }}
        .lock-action {{ min-width: 44px; min-height: 44px; padding: 0; border-radius: 22px; border: none; box-shadow: none; outline: none; background: rgba(255,255,255,0.13); color: white; }}
        .lock-action:hover {{ background: rgba(255,255,255,0.24); }}
        .lock-action:active {{ background: rgba(255,255,255,0.32); }}
        .lock-glyph {{ font-family: \"{icons}\"; font-size: 21px; color: white; }}
        .lock-caption {{ font-family: \"{font}\"; font-size: 12px; color: rgba(255,255,255,0.78); }}"
    ));
    gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 2);
}

async fn locked_hint(on: bool) {
    let Ok(conn) = crate::bus::system().await else { return };
    let Ok(s) = zbus::Proxy::new_owned(conn, "org.freedesktop.login1", "/org/freedesktop/login1/session/auto", "org.freedesktop.login1.Session").await else { return };
    let _ = s.call_method("SetLockedHint", &(on,)).await;
}

pub fn confirmed() -> bool {
    STATE.with(|s| s.borrow().as_ref().is_some_and(|st| matches!(st.mode, Mode::Real(_)) && st.locked))
        || FALLBACK.with(|f| f.borrow().as_ref().is_some_and(|f| f.load(std::sync::atomic::Ordering::SeqCst)))
}

fn fallback() {
    let started = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    FALLBACK.with(|f| *f.borrow_mut() = Some(started.clone()));
    glib::spawn_future_local(async move {
        let hooked = gio::spawn_blocking(|| crate::hooks::run("key", &["lock"])).await.unwrap_or(false);
        started.store(hooked || swaylock(), std::sync::atomic::Ordering::SeqCst);
    });
}

fn swaylock() -> bool {
    let Ok(process) = gio::Subprocess::newv(&[std::ffi::OsStr::new("swaylock"), std::ffi::OsStr::new("-f")], gio::SubprocessFlags::NONE) else { return false };
    glib::spawn_future_local(async move { let _ = process.wait_future().await; });
    true
}

pub fn lock(app: &Rc<App>) -> bool {
    if active() {
        return true;
    }
    let preview = STATE.with(|s| s.borrow().as_ref().map(|st| st.generation));
    if let Some(generation) = preview {
        finish(generation);
    }
    if !supported() {
        fallback();
        return false;
    }
    FALLBACK.with(|f| f.borrow_mut().take());
    style(app);
    // SAFETY: takes nothing and returns a new object, or null, which is checked below.
    let ptr = unsafe { ffi::gtk_session_lock_instance_new() };
    if ptr.is_null() {
        fallback();
        return false;
    }
    // SAFETY: `ptr` is the non-null new object above, whose one reference this takes over.
    let instance: glib::Object = unsafe { glib::Object::from_glib_full(ptr) };
    let generation = next_generation();
    STATE.with(|s| *s.borrow_mut() = Some(State { generation, app: Rc::downgrade(app), mode: Mode::Real(instance.clone()), faces: Vec::new(), busy: false, locked: false, clock: None }));
    let a = app.clone();
    instance.connect_local("monitor", false, move |args| {
        if !current(generation) { return None; }
        let inst = args.first()?.get::<glib::Object>().ok()?;
        let monitor = args.get(1)?.get::<gdk::Monitor>().ok()?;
        if STATE.with(|s| s.borrow().as_ref().is_some_and(|st| st.faces.iter().any(|f| f.monitor == monitor))) { return None; }
        monitor.connect_invalidate(move |monitor| {
            if !current(generation) { return; }
            let removed = STATE.with(|s| {
                let mut slot = s.borrow_mut();
                let st = slot.as_mut()?;
                let index = st.faces.iter().position(|f| f.monitor == *monitor)?;
                Some(st.faces.remove(index))
            });
            if let Some(face) = removed {
                face.window.destroy();
                // The password must stay somewhere: another screen takes it.
                let left = STATE.with(|s| s.borrow().as_ref().and_then(|st| (!st.faces.iter().any(|f| f.main.get())).then(|| st.faces.first().cloned()).flatten()));
                if let Some(next) = left { promote(&next); }
            }
        });
        // One screen holds the password: the main one, unless another took it while it was gone.
        let main = is_main(&a, &monitor) && main_face().is_none();
        let face = build(&a, &monitor, false, generation, main);
        // SAFETY: the instance, window and monitor are live objects borrowed for the call; the lock keeps its own references.
        unsafe {
            ffi::gtk_session_lock_instance_assign_window_to_monitor(inst.as_ptr(), face.window.to_glib_none().0, monitor.to_glib_none().0);
        }
        STATE.with(|s| {
            if let Some(st) = s.borrow_mut().as_mut() {
                st.faces.push(face);
            }
        });
        None
    });
    instance.connect_local("locked", false, move |_| {
        if !current(generation) { return None; }
        STATE.with(|s| {
            if let Some(st) = s.borrow_mut().as_mut() {
                st.locked = true;
            }
        });
        glib::spawn_future_local(locked_hint(true));
        None
    });
    instance.connect_local("failed", false, move |_| {
        if !current(generation) { return None; }
        let was = STATE.with(|s| s.borrow_mut().take());
        if let Some(st) = was {
            if let Some(id) = st.clock {
                id.remove();
            }
        }
        glib::idle_add_local_once(fallback);
        None
    });
    instance.connect_local("unlocked", false, move |_| {
        finish(generation);
        None
    });
    start_clock();
    // SAFETY: `ptr` is the live instance, kept alive by `instance` in STATE.
    let ok = unsafe { ffi::gtk_session_lock_instance_lock(ptr) } != 0;
    if !ok && current(generation) && !STATE.with(|s| s.borrow().as_ref().is_some_and(|st| st.locked)) {
        let was = STATE.with(|s| s.borrow_mut().take());
        if let Some(st) = was {
            if let Some(id) = st.clock { id.remove(); }
        }
        fallback();
        return false;
    }
    true
}

pub fn preview(app: &Rc<App>, output: Option<&str>) {
    if showing() {
        return;
    }
    style(app);
    let generation = next_generation();
    STATE.with(|s| *s.borrow_mut() = Some(State { generation, app: Rc::downgrade(app), mode: Mode::Preview, faces: Vec::new(), busy: false, locked: true, clock: None }));
    for m in kit::screens::all() {
        if output.is_some_and(|o| m.connector().as_deref() != Some(o)) {
            continue;
        }
        // A preview of one screen shows that screen as the main one would look.
        let face = build(app, &m, true, generation, output.is_some() || is_main(app, &m));
        face.window.present();
        STATE.with(|s| {
            if let Some(st) = s.borrow_mut().as_mut() {
                st.faces.push(face);
            }
        });
    }
    if STATE.with(|s| s.borrow().as_ref().is_some_and(|st| st.faces.is_empty())) {
        finish(generation);
        return;
    }
    start_clock();
}

fn start_clock() {
    let last = Rc::new(RefCell::new(String::new()));
    let id = glib::timeout_add_seconds_local(1, move || {
        let now = glib::DateTime::now_local().ok().and_then(|d| d.format("%H:%M").ok()).map(|s| s.to_string()).unwrap_or_default();
        if *last.borrow() != now {
            *last.borrow_mut() = now;
            STATE.with(|s| {
                if let Some(st) = s.borrow().as_ref() {
                    st.faces.iter().for_each(|f| f.paint.queue_draw());
                }
            });
        }
        glib::ControlFlow::Continue
    });
    STATE.with(|s| {
        if let Some(st) = s.borrow_mut().as_mut() {
            st.clock = Some(id);
        }
    });
}

fn finish(generation: u64) {
    if !current(generation) { return; }
    let was = STATE.with(|s| s.borrow_mut().take());
    let Some(st) = was else { return };
    if let Some(id) = st.clock {
        id.remove();
    }
    if matches!(st.mode, Mode::Preview) {
        for f in st.faces {
            f.window.destroy();
        }
    } else {
        glib::spawn_future_local(locked_hint(false));
    }
}

/// Whether the screen `monitor` unlocks into windows: its workspace in view has some.
fn busy(app: &App, monitor: &gdk::Monitor) -> bool {
    let output = monitor.connector().map(|c| c.to_string()).unwrap_or_default();
    app.spaces.borrow().iter().any(|sp| sp.output == output && sp.active && sp.filled)
}

/// The password was right. On each screen that unlocks into its bare desktop the wallpaper
/// clears and the password fades, then the screen unlocks, and the clock settles onto the
/// desktop above the bar and the widgets, which show as they are (see [`settle`]). At once
/// without animations, or when the main screen unlocks into windows, which would cover it.
fn reveal(generation: u64) {
    let app = STATE.with(|s| s.borrow().as_ref().and_then(|st| st.app.upgrade()));
    let Some(app) = app.filter(|a| a.prefs.borrow().lock_motion && !kit::motion::reduced()) else {
        unlock(generation);
        return;
    };
    if main_face().is_none_or(|f| busy(&app, &f.monitor)) {
        unlock(generation);
        return;
    }
    let go: Rc<Cell<Option<i64>>> = Rc::default();
    let start = glib::monotonic_time();
    each(|f| {
        // A screen with windows waits as it is.
        if busy(&app, &f.monitor) {
            return;
        }
        settle(&app, &f.monitor, go.clone(), f.main.get());
        f.reveal.set(Some(start));
        f.paint.queue_draw();
        let (column, actions) = (f.column.clone(), f.actions.clone());
        f.paint.add_tick_callback(move |p, clock| {
            p.queue_draw();
            let r = part(start, clock.frame_time(), CLEAR_MS);
            column.set_opacity(1.0 - smooth(r));
            actions.set_opacity(1.0 - smooth(r));
            if r >= 1.0 { glib::ControlFlow::Break } else { glib::ControlFlow::Continue }
        });
    });
    // A frame after the clear one; the clock starts settling with the first frame unlocked.
    glib::timeout_add_local_once(Duration::from_millis((CLEAR_MS + 30) as u64), move || {
        go.set(Some(i64::MAX));
        unlock(generation);
    });
}

fn unlock(generation: u64) {
    if !current(generation) { return; }
    let real = STATE.with(|s| match s.borrow().as_ref().map(|st| &st.mode) {
        Some(Mode::Real(i)) => Some(i.clone()),
        _ => None,
    });
    match real {
        // SAFETY: `instance` is a live, owned reference to the lock instance.
        Some(instance) => unsafe { ffi::gtk_session_lock_instance_unlock(instance.as_ptr()) },
        None => finish(generation),
    }
}


async fn verify(bytes: Vec<u8>) -> Option<bool> {
    let argv = [std::ffi::OsStr::new("/proc/self/exe"), std::ffi::OsStr::new("auth")];
    let Ok(p) = gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDIN_PIPE | gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE) else {
        let mut b = bytes;
        wipe(&mut b);
        return Some(false);
    };
    let Some(stdin) = p.stdin_pipe() else {
        let mut b = bytes;
        wipe(&mut b);
        p.force_exit();
        let _ = p.wait_future().await;
        return Some(false);
    };
    let work = Box::pin(async {
        match stdin.write_all_future(bytes, glib::Priority::DEFAULT).await {
            Ok((mut b, _, _)) => wipe(&mut b),
            Err((mut b, _)) => wipe(&mut b),
        }
        let _ = stdin.close_future(glib::Priority::DEFAULT).await;
        p.wait_future().await.is_ok() && p.is_successful()
    });
    match futures_util::future::select(work, Box::pin(glib::timeout_future(Duration::from_secs(30)))).await {
        futures_util::future::Either::Left((ok, _)) => Some(ok),
        futures_util::future::Either::Right((_, work)) => {
            p.force_exit();
            // Complete the pending write so its password buffer is wiped, then reap.
            let _ = work.await;
            let _ = p.wait_future().await;
            None
        }
    }
}

fn each(f: impl Fn(&Sheet)) {
    let faces = STATE.with(|s| s.borrow().as_ref().map(|st| st.faces.clone()).unwrap_or_default());
    faces.iter().for_each(f);
}

pub fn appearance_changed() {
    each(|f| f.paint.queue_draw());
}

fn shake(column: &gtk::Box) {
    if kit::motion::reduced() {
        return;
    }
    let start = glib::monotonic_time();
    column.add_tick_callback(move |c, clock| {
        let t = ((clock.frame_time() - start) as f64 / 420_000.0).clamp(0.0, 1.0);
        let dx = ((t * std::f64::consts::PI * 6.0).sin() * 12.0 * (1.0 - t)).round() as i32;
        c.set_margin_start((dx * 2).max(0));
        c.set_margin_end((-dx * 2).max(0));
        if t >= 1.0 || kit::motion::reduced() {
            c.set_margin_start(0);
            c.set_margin_end(0);
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

fn submit(entry: &gtk::PasswordEntry, generation: u64) {
    if !current(generation) { return; }
    let busy = STATE.with(|s| s.borrow().as_ref().is_none_or(|st| st.busy));
    if busy {
        return;
    }
    let bytes = {
        let text = entry.text();
        if text.is_empty() {
            return;
        }
        text.as_bytes().to_vec()
    };
    entry.set_text("");
    STATE.with(|s| {
        if let Some(st) = s.borrow_mut().as_mut() {
            st.busy = true;
        }
    });
    each(|f| {
        f.entry.set_sensitive(false);
        f.status.remove_css_class("error");
        f.status.set_text("Checking…");
    });
    glib::spawn_future_local(async move {
        let ok = verify(bytes).await;
        if !current(generation) { return; }
        if ok == Some(true) {
            each(|f| f.status.set_text(""));
            reveal(generation);
            return;
        }
        STATE.with(|s| {
            if let Some(st) = s.borrow_mut().as_mut() {
                st.busy = false;
            }
        });
        each(|f| {
            f.entry.set_sensitive(true);
            f.status.add_css_class("error");
            f.status.set_text(if ok.is_none() { "Authentication timed out" } else { "Incorrect password" });
            shake(&f.column);
        });
        each(|f| {
            if f.window.is_active() {
                f.entry.grab_focus();
            }
        });
    });
}

fn avatar() -> Option<gdk::Texture> {
    let user = glib::user_name().to_string_lossy().into_owned();
    let home = glib::home_dir();
    [home.join(".face"), home.join(".face.icon"), std::path::PathBuf::from(format!("/var/lib/AccountsService/icons/{user}"))]
        .into_iter()
        .filter(|p| p.is_file())
        .find_map(|p| gdk::Texture::from_filename(p).ok())
}

fn build(app: &Rc<App>, monitor: &gdk::Monitor, preview: bool, generation: u64, main: bool) -> Sheet {
    let main = Rc::new(Cell::new(main));
    let window = gtk::Window::new();
    window.add_css_class("digit-lock");
    if preview {
        window.init_layer_shell();
        window.set_layer(Layer::Overlay);
        window.set_namespace(Some("digit-lock-preview"));
        window.set_exclusive_zone(-1);
        window.set_keyboard_mode(KeyboardMode::Exclusive);
        window.set_monitor(Some(monitor));
        for e in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            window.set_anchor(e, true);
        }
    }
    let output = monitor.connector().map(|c| c.to_string()).unwrap_or_default();
    // With effects on, the wallpaper's soft copy that this screen shows, the one the app grid
    // lies on too (see `Wallpapers::blurred`); with them off, the sharp wallpaper as it is.
    let effects_off = kit::effects::reduced();
    let source = if effects_off { app.wallpapers.texture(&output) } else { app.wallpapers.blurred(&output) };
    let start = glib::monotonic_time();
    let paint = Paint::new(-1, -1);
    paint.set_hexpand(true);
    paint.set_vexpand(true);
    let reveal: Rc<Cell<Option<i64>>> = Rc::default();
    {
        let weak = paint.downgrade();
        let cfg = app.cfg.clone();
        let main = main.clone();
        let (a, reveal, output) = (Rc::downgrade(app), reveal.clone(), output.clone());
        // The sharp wallpaper, found once unlocking starts.
        let sharp: RefCell<Option<Option<(gdk::Texture, graphene::Rect)>>> = RefCell::new(None);
        paint.on_draw(move |s| {
            let Some(p) = weak.upgrade() else { return };
            let (w, h) = (p.width() as f64, p.height() as f64);
            if w < 2.0 || h < 2.0 {
                return;
            }
            let full = graphene::Rect::new(0.0, 0.0, w as f32, h as f32);
            s.append_color(&gdk::RGBA::BLACK, &full);
            let effects = effects_off;
            let now = p.frame_clock().map(|c| c.frame_time()).unwrap_or_else(glib::monotonic_time);
            let fade = if kit::motion::reduced() { 1.0 } else { ((now - start) as f64 / 380_000.0).clamp(0.0, 1.0) };
            let fade = fade * fade * (3.0 - 2.0 * fade);
            if let Some(t) = source.as_ref() {
                s.push_opacity(fade);
                s.append_texture(t, &full);
                s.pop();
            }
            // Unlocking: the soft wallpaper clears into the sharp one the desktop shows, and the
            // dark over it lifts.
            let r = reveal.get().map(|start| part(start, now, CLEAR_MS));
            let clear = r.map_or(0.0, smooth);
            if let (Some(_), Some(app), false) = (r, a.upgrade(), effects) {
                let sharp = sharp.borrow_mut().get_or_insert_with(|| app.wallpapers.sharp(&output, w, h)).clone();
                if let Some((tex, rect)) = sharp {
                    s.push_clip(&full);
                    s.push_opacity(clear);
                    s.append_texture(&tex, &rect);
                    s.pop();
                    s.pop();
                }
            }
            s.append_color(&gdk::RGBA::new(0.0, 0.0, 0.0, 0.26 * (1.0 - clear) as f32), &full);
            s.append_linear_gradient(
                &graphene::Rect::new(0.0, (h * 0.55) as f32, w as f32, (h * 0.45) as f32),
                &graphene::Point::new(0.0, (h * 0.55) as f32),
                &graphene::Point::new(0.0, h as f32),
                &[gtk::gsk::ColorStop::new(0.0, gdk::RGBA::new(0.0, 0.0, 0.0, 0.0)), gtk::gsk::ColorStop::new(1.0, gdk::RGBA::new(0.0, 0.0, 0.0, 0.38 * (1.0 - clear) as f32))],
            );
            // The other screens show only the wallpaper.
            if !main.get() {
                return;
            }
            // Unlocking, the clock stays where it is: its own surface takes it on (see `settle`).
            s.push_opacity(fade);
            lines(s, &p, &cfg, w, lock_lines(w, h, fade), 1.0, effects);
            s.pop();
        });
    }
    if !kit::motion::reduced() {
        paint.add_tick_callback(move |p, clock| {
            p.queue_draw();
            if kit::motion::reduced() || clock.frame_time() - start > 420_000 {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&paint));
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    column.set_halign(gtk::Align::Center);
    column.set_valign(gtk::Align::End);
    let geo = monitor.geometry();
    column.set_margin_bottom(((geo.height() as f64) * 0.13).round() as i32);
    let face = Paint::new(88, 88);
    face.set_halign(gtk::Align::Center);
    {
        let pic = avatar();
        let name = session::real_name();
        let weak = face.downgrade();
        let cfg = app.cfg.clone();
        face.on_draw(move |s| {
            let Some(p) = weak.upgrade() else { return };
            let circle = draw::round(0.0, 0.0, 88.0, 88.0, 44.0);
            match pic.as_ref() {
                Some(t) => {
                    s.push_rounded_clip(&circle);
                    s.append_texture(t, circle.bounds());
                    s.pop();
                }
                None => {
                    draw::fill(s, &circle, Rgb::new(1.0, 1.0, 1.0).alpha(0.2));
                    let letter: String = name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_default();
                    if letter.is_empty() {
                        draw::icon(s, &p, &cfg, "person", 44.0, true, Rgb::new(1.0, 1.0, 1.0), 44.0, 44.0);
                    } else {
                        draw::text(s, &p, &cfg, &letter, 36.0, Face::Text(560), Rgb::new(1.0, 1.0, 1.0), 0.0, 44.0, 88.0, pango::Alignment::Center);
                    }
                }
            }
            let c = gdk::RGBA::new(1.0, 1.0, 1.0, 0.22);
            s.append_border(&circle, &[1.0; 4], &[c, c, c, c]);
        });
    }
    column.append(&face);
    let name = gtk::Label::new(Some(&session::real_name()));
    name.add_css_class("lock-name");
    name.set_margin_top(14);
    column.append(&name);
    let entry = gtk::PasswordEntry::new();
    entry.set_show_peek_icon(false);
    entry.set_property("placeholder-text", "Enter Password");
    entry.add_css_class("lock-entry");
    entry.set_halign(gtk::Align::Center);
    entry.set_margin_top(16);
    column.append(&entry);
    let status = gtk::Label::new(None);
    status.add_css_class("lock-status");
    status.set_margin_top(10);
    column.append(&status);
    overlay.add_overlay(&column);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 22);
    actions.set_halign(gtk::Align::End);
    actions.set_valign(gtk::Align::End);
    actions.set_margin_end(36);
    actions.set_margin_bottom(32);
    for (act, glyph, label) in [(Act::Sleep, "bedtime", "Sleep"), (Act::Restart, "restart_alt", "Restart"), (Act::PowerOff, "power_settings_new", "Shut Down")] {
        let cell = gtk::Box::new(gtk::Orientation::Vertical, 7);
        let button = gtk::Button::new();
        button.add_css_class("lock-action");
        button.set_halign(gtk::Align::Center);
        button.set_focus_on_click(false);
        let g = gtk::Label::new(Some(glyph));
        g.add_css_class("lock-glyph");
        button.set_child(Some(&g));
        let a = app.clone();
        button.connect_clicked(move |_| session::run(&a, act));
        let caption = gtk::Label::new(Some(label));
        caption.add_css_class("lock-caption");
        cell.append(&button);
        cell.append(&caption);
        actions.append(&cell);
    }
    overlay.add_overlay(&actions);
    column.set_visible(main.get());
    actions.set_visible(main.get());
    window.set_child(Some(&overlay));
    entry.connect_activate(move |e| submit(e, generation));
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let e = entry.clone();
    let mine = main.clone();
    keys.connect_key_pressed(move |_, key, _, mods| {
        if !current(generation) { return glib::Propagation::Proceed; }
        // Typed on a screen without the password: it goes into the main screen's.
        if !mine.get() {
            if let Some(m) = main_face() {
                forward(&m.entry, key, mods, generation);
            }
            return glib::Propagation::Stop;
        }
        if key == gdk::Key::Escape {
            let preview = STATE.with(|s| s.borrow().as_ref().is_some_and(|st| matches!(st.mode, Mode::Preview)));
            if e.text().is_empty() && preview {
                glib::idle_add_local_once(move || finish(generation));
            } else {
                e.set_text("");
            }
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(keys);
    let e = entry.clone();
    let mine = main.clone();
    window.connect_map(move |_| {
        if !mine.get() {
            return;
        }
        let e = e.clone();
        glib::timeout_add_local_once(Duration::from_millis(60), move || {
            e.grab_focus();
        });
    });
    let e = entry.clone();
    let mine = main.clone();
    window.connect_is_active_notify(move |w| {
        if w.is_active() && mine.get() {
            e.grab_focus();
        }
    });
    Sheet { monitor: monitor.clone(), window, paint, column, actions, entry, status, main, reveal }
}

/// A key pressed on a screen without the password box, put into `entry` on the main screen:
/// letters go in, Backspace takes the last one off, Enter tries it and Escape clears it.
fn forward(entry: &gtk::PasswordEntry, key: gdk::Key, mods: gdk::ModifierType, generation: u64) {
    if !entry.is_sensitive() {
        return;
    }
    match key {
        gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::ISO_Enter => submit(entry, generation),
        gdk::Key::Escape => entry.set_text(""),
        gdk::Key::BackSpace => {
            let text = entry.text();
            let mut chars = text.chars();
            chars.next_back();
            entry.set_text(chars.as_str());
        }
        _ if !mods.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK | gdk::ModifierType::SUPER_MASK) => {
            if let Some(c) = key.to_unicode().filter(|c| !c.is_control()) {
                let mut text = entry.text().to_string();
                text.push(c);
                entry.set_text(&text);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(mode: Mode) -> u64 {
        let generation = next_generation();
        STATE.with(|s| *s.borrow_mut() = Some(State { generation, app: std::rc::Weak::new(), mode, faces: Vec::new(), busy: false, locked: false, clock: None }));
        generation
    }

    #[test]
    fn preview_is_not_a_real_lock_and_stale_generation_cannot_finish_replacement() {
        let preview = state(Mode::Preview);
        assert!(showing());
        assert!(!active());
        assert!(!confirmed());
        let real = state(Mode::Real(glib::Object::new()));
        assert!(active());
        assert!(!confirmed());
        STATE.with(|s| s.borrow_mut().as_mut().unwrap().locked = true);
        assert!(confirmed());
        assert!(!current(preview));
        finish(preview);
        assert!(current(real));
        STATE.with(|s| s.borrow_mut().take());
    }

    #[test]
    fn finishing_empty_preview_clears_state() {
        let preview = state(Mode::Preview);
        finish(preview);
        assert!(!showing());
        assert!(!current(preview));
    }
}
