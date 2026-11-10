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
    entry: gtk::PasswordEntry,
    status: gtk::Label,
}

struct State {
    generation: u64,
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
    let ptr = unsafe { ffi::gtk_session_lock_instance_new() };
    if ptr.is_null() {
        fallback();
        return false;
    }
    let instance: glib::Object = unsafe { glib::Object::from_glib_full(ptr) };
    let generation = next_generation();
    STATE.with(|s| *s.borrow_mut() = Some(State { generation, mode: Mode::Real(instance.clone()), faces: Vec::new(), busy: false, locked: false, clock: None }));
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
            if let Some(face) = removed { face.window.destroy(); }
        });
        let face = build(&a, &monitor, false, generation);
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
    let Some(display) = gdk::Display::default() else { return };
    let generation = next_generation();
    STATE.with(|s| *s.borrow_mut() = Some(State { generation, mode: Mode::Preview, faces: Vec::new(), busy: false, locked: true, clock: None }));
    for m in display.monitors().iter::<gdk::Monitor>().filter_map(Result::ok) {
        if output.is_some_and(|o| m.connector().as_deref() != Some(o)) {
            continue;
        }
        let face = build(app, &m, true, generation);
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

fn unlock(generation: u64) {
    if !current(generation) { return; }
    let real = STATE.with(|s| match s.borrow().as_ref().map(|st| &st.mode) {
        Some(Mode::Real(i)) => Some(i.clone()),
        _ => None,
    });
    match real {
        Some(instance) => unsafe { ffi::gtk_session_lock_instance_unlock(instance.as_ptr()) },
        None => finish(generation),
    }
}

fn wipe(buf: &mut [u8]) {
    for b in buf.iter_mut() {
        unsafe { std::ptr::write_volatile(b, 0) };
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
            unlock(generation);
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
    let user = unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if pw.is_null() || (*pw).pw_name.is_null() { String::new() } else { std::ffi::CStr::from_ptr((*pw).pw_name).to_string_lossy().into_owned() }
    };
    let home = glib::home_dir();
    [home.join(".face"), home.join(".face.icon"), std::path::PathBuf::from(format!("/var/lib/AccountsService/icons/{user}"))]
        .into_iter()
        .filter(|p| p.is_file())
        .find_map(|p| gdk::Texture::from_filename(p).ok())
}

fn blurred(paint: &Paint, texture: &gdk::Texture, w: f64, h: f64) -> Option<gdk::Texture> {
    let renderer = paint.native()?.renderer()?;
    let (qw, qh) = ((w / 4.0).ceil().max(1.0), (h / 4.0).ceil().max(1.0));
    let (tw, th) = (texture.width() as f64, texture.height() as f64);
    let scale = (qw / tw).max(qh / th);
    let (dw, dh) = (tw * scale, th * scale);
    let pad = 20.0;
    let snap = gtk::Snapshot::new();
    snap.push_blur(9.0);
    snap.append_texture(texture, &graphene::Rect::new(((qw - dw) / 2.0 - pad) as f32, ((qh - dh) / 2.0 - pad) as f32, (dw + 2.0 * pad) as f32, (dh + 2.0 * pad) as f32));
    snap.pop();
    let node = snap.to_node()?;
    Some(renderer.render_texture(&node, Some(&graphene::Rect::new(0.0, 0.0, qw as f32, qh as f32))))
}

fn build(app: &Rc<App>, monitor: &gdk::Monitor, preview: bool, generation: u64) -> Sheet {
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
    let source = app.wallpapers.texture(&output);
    let back: Rc<RefCell<Option<gdk::Texture>>> = Rc::new(RefCell::new(None));
    let start = glib::monotonic_time();
    let paint = Paint::new(-1, -1);
    paint.set_hexpand(true);
    paint.set_vexpand(true);
    {
        let weak = paint.downgrade();
        let back = back.clone();
        let cfg = app.cfg.clone();
        paint.on_draw(move |s| {
            let Some(p) = weak.upgrade() else { return };
            let (w, h) = (p.width() as f64, p.height() as f64);
            if w < 2.0 || h < 2.0 {
                return;
            }
            let full = graphene::Rect::new(0.0, 0.0, w as f32, h as f32);
            s.append_color(&gdk::RGBA::BLACK, &full);
            let effects = kit::effects::reduced();
            if !effects && back.borrow().is_none() {
                if let Some(t) = source.as_ref() {
                    *back.borrow_mut() = blurred(&p, t, w, h);
                }
            }
            let now = p.frame_clock().map(|c| c.frame_time()).unwrap_or_else(glib::monotonic_time);
            let fade = if kit::motion::reduced() { 1.0 } else { ((now - start) as f64 / 380_000.0).clamp(0.0, 1.0) };
            let fade = fade * fade * (3.0 - 2.0 * fade);
            let back = back.borrow();
            if let Some(t) = if effects { source.as_ref() } else { back.as_ref() } {
                s.push_opacity(fade);
                s.append_texture(t, &full);
                s.pop();
            }
            s.append_color(&gdk::RGBA::new(0.0, 0.0, 0.0, 0.26), &full);
            s.append_linear_gradient(
                &graphene::Rect::new(0.0, (h * 0.55) as f32, w as f32, (h * 0.45) as f32),
                &graphene::Point::new(0.0, (h * 0.55) as f32),
                &graphene::Point::new(0.0, h as f32),
                &[gtk::gsk::ColorStop::new(0.0, gdk::RGBA::new(0.0, 0.0, 0.0, 0.0)), gtk::gsk::ColorStop::new(1.0, gdk::RGBA::new(0.0, 0.0, 0.0, 0.38))],
            );
            let dt = glib::DateTime::now_local().ok();
            let date = dt.as_ref().and_then(|d| d.format("%A, %-d %B").ok()).map(|s| s.to_string()).unwrap_or_default();
            let time = dt.as_ref().and_then(|d| d.format(&cfg.clock).ok()).map(|s| s.to_string()).unwrap_or_default();
            let white = Rgb::new(1.0, 1.0, 1.0);
            let top = h * 0.14 + (1.0 - fade) * 10.0;
            s.push_opacity(fade);
            if !effects { s.push_shadow(&[gtk::gsk::Shadow::new(gdk::RGBA::new(0.0, 0.0, 0.0, 0.25), 0.0, 1.0, 12.0)]); }
            draw::text(s, &p, &cfg, &date, 22.0, Face::Text(560), white.alpha(0.9), 0.0, top, w, pango::Alignment::Center);
            draw::text(s, &p, &cfg, &time, (h * 0.105).clamp(72.0, 150.0), Face::Text(420), white.alpha(0.96), 0.0, top + 22.0 + (h * 0.06).clamp(42.0, 88.0), w, pango::Alignment::Center);
            if !effects { s.pop(); }
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
    window.set_child(Some(&overlay));
    entry.connect_activate(move |e| submit(e, generation));
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let e = entry.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        if !current(generation) { return glib::Propagation::Proceed; }
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
    window.connect_map(move |_| {
        let e = e.clone();
        glib::timeout_add_local_once(Duration::from_millis(60), move || {
            e.grab_focus();
        });
    });
    let e = entry.clone();
    window.connect_is_active_notify(move |w| {
        if w.is_active() {
            e.grab_focus();
        }
    });
    Sheet { monitor: monitor.clone(), window, paint, column, entry, status }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(mode: Mode) -> u64 {
        let generation = next_generation();
        STATE.with(|s| *s.borrow_mut() = Some(State { generation, mode, faces: Vec::new(), busy: false, locked: false, clock: None }));
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
