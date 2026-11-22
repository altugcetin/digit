use crate::moved::Moved;
use crate::pane::{is_dir, kind, size_text, target_of};
use gtk::{gdk, gio, glib, pango, prelude::*};
use kit::motion::{Curve, Tween};
use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

pub const DROP: f64 = kit::ui::SETTLE_DROP;
const MARGIN: i32 = 36;

struct Look {
    root: gtk::Box,
    card: gtk::Box,
    /// The card, grown out of the file it shows as it opens and back into it as it closes.
    zoom: Moved,
    stage: gtk::Box,
    /// The preview, coming in from the side of the file it moved to.
    shift: Moved,
    /// Where the file shown is in the view's order, and which move of the preview is the last.
    at: Cell<Option<u32>>,
    moves: std::rc::Rc<Cell<u64>>,
    /// When the card stops moving, for what's heavy to start (see `later`).
    calm: Cell<i64>,
    title: gtk::Label,
    sub: gtk::Label,
    shown: RefCell<Option<String>>,
    busy: RefCell<Option<Arc<AtomicBool>>>,
}

thread_local! {
    static LOOK: RefCell<Option<std::rc::Rc<Look>>> = const { RefCell::new(None) };
}

pub fn open() -> bool {
    LOOK.with(|l| l.borrow().is_some())
}

pub fn toggle() {
    if open() {
        close();
    } else {
        show();
    }
}

pub fn close() {
    let look = LOOK.with(|l| l.borrow_mut().take());
    if let Some(l) = look {
        if let Some(b) = l.busy.borrow_mut().take() {
            b.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        l.root.set_can_target(false);
        let (root, stage) = (l.root.clone(), l.stage.clone());
        let done = move || {
            kit::ui::clear(&stage);
            crate::overlay_remove(&root);
            kit::app::trim_later(600);
        };
        // Back into the file it shows, while that shows in the view; else as every dialog goes.
        match spot() {
            Some(spot) if !kit::motion::reduced() => {
                l.zoom.grow_from(Some((spot.0, &spot.1)));
                grow(&l, false, done);
            }
            _ => play(&l.root, &l.card, MARGIN, false, done),
        }
    }
}

/// Where the selected file's picture is in the window, while it shows in the view.
fn spot() -> Option<(gtk::graphene::Rect, gtk::Widget)> {
    let window = crate::window()?;
    let rect = crate::active()?.selected_spot(&window)?;
    Some((rect, window.upcast()))
}

/// Quick Look growing out of the file it shows (`opening`), or back into it, then `done`.
fn grow(look: &Look, opening: bool, done: impl FnOnce() + 'static) {
    let now = glib::monotonic_time();
    look.calm.set(now + 360_000);
    let mut t = Tween::still(if opening { 0.0 } else { 1.0 });
    if opening {
        t.go(1.0, now, 360, Curve::new(0.16, 1.0, 0.3, 1.0));
    } else {
        t.go(0.0, now, 240, Curve::new(0.4, 0.0, 0.9, 0.6));
    }
    let (zoom, done) = (look.zoom.downgrade(), Cell::new(Some(done)));
    let step = move |root: &gtk::Widget, now: i64| {
        let v = t.value(now);
        if let Some(zoom) = zoom.upgrade() {
            zoom.set_grown(v);
        }
        // It shows as it starts to grow, and goes as it's nearly back.
        root.set_opacity((v * 1.6).min(1.0));
        t.settled(now)
    };
    step(look.root.upcast_ref(), now);
    look.root.add_tick_callback(move |root, clock| {
        if !step(root.upcast_ref(), clock.frame_time()) {
            return glib::ControlFlow::Continue;
        }
        if let Some(f) = done.take() {
            f();
        }
        glib::ControlFlow::Break
    });
}

/// The preview of the file the selection moved to comes in from its side: `ahead`, from the
/// right.
fn slide_in(look: &Look, ahead: bool) {
    let n = look.moves.get() + 1;
    look.moves.set(n);
    let now = glib::monotonic_time();
    look.calm.set(look.calm.get().max(now + 220_000));
    let mut t = Tween::still(0.0);
    t.go(1.0, now, 220, kit::motion::STANDARD);
    let reach = if ahead { 28.0 } else { -28.0 };
    let (stage, shift) = (look.stage.downgrade(), look.shift.downgrade());
    let mine = look.moves.clone();
    let step = move |now: i64| {
        let v = t.value(now);
        if let (Some(stage), Some(shift)) = (stage.upgrade(), shift.upgrade()) {
            shift.set_dx(reach * (1.0 - v));
            stage.set_opacity(v);
        }
        t.settled(now)
    };
    step(now);
    look.shift.add_tick_callback(move |_, clock| {
        // A newer move takes over.
        if mine.get() != n || step(clock.frame_time()) { glib::ControlFlow::Break } else { glib::ControlFlow::Continue }
    });
}

/// Quick Look and every Files dialog share the shell-like settle; `margin` is the card's resting margin.
pub fn play(root: &gtk::Box, card: &gtk::Box, margin: i32, opening: bool, done: impl FnOnce() + 'static) {
    kit::ui::settle(root, card, margin, opening, done);
}

fn show() {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("look-backdrop");
    root.set_hexpand(true);
    root.set_vexpand(true);
    let card = gtk::Box::new(gtk::Orientation::Vertical, 12);
    card.add_css_class("look");
    // Out of the selected file, while it shows; else as every dialog comes.
    let from = spot().filter(|_| !kit::motion::reduced());
    let drop = if from.is_some() { 0 } else { DROP as i32 };
    card.set_margin_top(MARGIN + drop);
    card.set_margin_bottom(MARGIN - drop);
    card.set_margin_start(48);
    card.set_margin_end(48);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
    col.set_hexpand(true);
    let title = kit::ui::text("", "name");
    title.set_ellipsize(pango::EllipsizeMode::Middle);
    let sub = kit::ui::text("", "sub");
    col.append(&title);
    col.append(&sub);
    head.append(&col);
    let openb = kit::ui::button("Open", "accent");
    openb.connect_clicked(|_| {
        if let Some(p) = crate::active() {
            p.open_selected();
        }
        close();
    });
    head.append(&openb);
    let x = kit::app::control("close", "");
    x.connect_clicked(|_| close());
    head.append(&x);
    card.append(&head);
    let stage = gtk::Box::new(gtk::Orientation::Vertical, 0);
    stage.add_css_class("stage");
    stage.set_vexpand(true);
    stage.set_hexpand(true);
    let shift = Moved::new(&stage);
    card.append(&shift);
    let zoom = Moved::new(&card);
    // In the middle of the window, as the card was before it grew.
    zoom.set_halign(gtk::Align::Center);
    zoom.set_valign(gtk::Align::Center);
    zoom.set_hexpand(true);
    zoom.set_vexpand(true);
    root.append(&zoom);
    let click = gtk::GestureClick::new();
    let c2 = card.clone();
    click.connect_released(move |g, _, x, y| {
        let Some(w) = g.widget() else { return };
        let inside = w.pick(x, y, gtk::PickFlags::DEFAULT).is_some_and(|p| p == c2 || p.is_ancestor(&c2));
        if !inside {
            close();
        }
    });
    root.add_controller(click);
    root.set_opacity(0.0);
    let look = std::rc::Rc::new(Look { root: root.clone(), card: card.clone(), zoom, stage, shift, at: Cell::new(None), moves: std::rc::Rc::default(), calm: Cell::new(0), title, sub, shown: RefCell::new(None), busy: RefCell::new(None) });
    LOOK.with(|l| *l.borrow_mut() = Some(look.clone()));
    crate::overlay_add(&root);
    match from {
        Some((rect, of)) => {
            look.zoom.grow_from(Some((rect, &of)));
            grow(&look, true, || {});
        }
        None => play(&root, &card, MARGIN, true, || {}),
    }
    update();
}

/// Whether text of the type `ct` is prose, which wraps, rather than code or data, which doesn't.
fn prose(ct: &str) -> bool {
    matches!(ct, "text/plain" | "text/markdown" | "text/x-markdown" | "text/x-log" | "text/rtf" | "text/x-readme")
}

/// Runs `f` once the card stopped moving, a frame after.
fn later(look: &Look, f: impl FnOnce() + 'static) {
    let wait = (look.calm.get() - glib::monotonic_time()).max(0) / 1000 + 30;
    glib::timeout_add_local_once(std::time::Duration::from_millis(wait as u64), f);
}

fn showing(stage: &gtk::Box, uri: &str) -> bool {
    LOOK.with(|l| l.borrow().as_ref().is_some_and(|look| look.stage == *stage && look.shown.borrow().as_deref() == Some(uri)))
}

pub fn update() {
    let Some(look) = LOOK.with(|l| l.borrow().clone()) else { return };
    let Some(p) = crate::active() else { return };
    let infos = p.selected();
    let Some(info) = infos.first().cloned() else {
        close();
        return;
    };
    let Some(file) = target_of(&info) else { return };
    let uri = file.uri().to_string();
    if look.shown.borrow().as_deref() == Some(uri.as_str()) {
        return;
    }
    let before = look.shown.replace(Some(uri.clone()));
    // Moved to another file while open: its preview comes in from that side.
    let now_at = p.selected_position();
    if let (Some(_), Some(was), Some(at)) = (before, look.at.replace(now_at), now_at)
        && was != at
        && !kit::motion::reduced()
    {
        slide_in(&look, at > was);
    }
    if let Some(b) = look.busy.borrow_mut().take() {
        b.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    kit::ui::clear(&look.stage);
    look.title.set_text(&info.display_name());
    let size = if is_dir(&info) { String::new() } else { format!(" · {}", size_text(info.size().max(0) as u64)) };
    look.sub.set_text(&format!("{}{size}", kind(&info)));
    let ct = info.attribute_string("standard::fast-content-type").map(|s| s.to_string()).unwrap_or_default();
    let (w, h) = crate::window().map(|w| (w.width(), w.height())).unwrap_or((1000, 700));
    let (maxw, maxh) = ((w as f64 * 0.78) as i32, (h as f64 * 0.70) as i32);
    if is_dir(&info) {
        let img = gtk::Image::new();
        crate::art::set(&img, &file, 196);
        look.stage.append(&img);
        let l = kit::ui::text("Counting…", "sub");
        l.set_halign(gtk::Align::Center);
        look.stage.append(&l);
        let stop = crate::ops::sizes(file, move |bytes, count| l.set_text(&format!("{count} items · {}", size_text(bytes))));
        *look.busy.borrow_mut() = Some(stop);
        return;
    }
    if ct.starts_with("image/") && file.is_native() {
        let pic = gtk::Picture::new();
        pic.set_size_request(maxw.min(900), maxh.min(640));
        pic.set_content_fit(gtk::ContentFit::Contain);
        look.stage.append(&pic);
        let path = file.path();
        let (bw, bh) = (maxw * pic.scale_factor(), maxh * pic.scale_factor());
        glib::spawn_future_local(async move {
            let tex = gio::spawn_blocking(move || {
                let pb = gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(path?, bw.max(64), bh.max(64), true).ok()?;
                let pb = pb.apply_embedded_orientation().unwrap_or(pb);
                let format = if pb.has_alpha() { gdk::MemoryFormat::R8g8b8a8 } else { gdk::MemoryFormat::R8g8b8 };
                Some(gdk::MemoryTexture::new(pb.width(), pb.height(), format, &pb.read_pixel_bytes(), pb.rowstride() as usize).upcast::<gdk::Texture>())
            })
            .await
            .ok()
            .flatten();
            if let Some(t) = tex {
                pic.set_paintable(Some(&t));
            }
        });
        return;
    }
    if ct.starts_with("video/") {
        // The player starts once the card stopped moving: setting it up holds everything up a
        // moment, which would stutter its growing. Meanwhile, and until its first frame is
        // ready, the video's picture shows where it plays.
        let size = (maxw.min(960), maxh.min(560));
        let held = gtk::Overlay::new();
        held.set_size_request(size.0, size.1);
        let still = gtk::Picture::new();
        still.set_content_fit(gtk::ContentFit::Contain);
        held.set_child(Some(&still));
        let (pic, icon) = (still.clone(), info.icon());
        crate::thumbs::texture_for(&file, &info, move |t| match t {
            Some(t) => pic.set_paintable(Some(&t)),
            // No picture of it: its icon, as large as the shown file's.
            None => {
                let theme = gtk::IconTheme::for_display(&pic.display());
                let icon = icon.unwrap_or_else(|| gio::ThemedIcon::new("video-x-generic").upcast());
                pic.set_paintable(Some(&theme.lookup_by_gicon(&icon, 256, pic.scale_factor(), gtk::TextDirection::None, gtk::IconLookupFlags::empty())));
                pic.set_content_fit(gtk::ContentFit::ScaleDown);
            }
        });
        look.stage.append(&held);
        let (stage, uri) = (look.stage.clone(), uri.clone());
        later(&look, move || {
            if !showing(&stage, &uri) {
                return;
            }
            let v = gtk::Video::for_file(Some(&file));
            v.set_autoplay(true);
            v.set_size_request(size.0, size.1);
            v.set_opacity(0.0);
            held.add_overlay(&v);
            let shown = v.downgrade();
            let reveal = move || {
                if let Some(v) = shown.upgrade() {
                    v.set_opacity(1.0);
                    still.set_visible(false);
                }
            };
            match v.media_stream() {
                Some(m) if !m.is_prepared() => {
                    let once = Cell::new(Some(reveal));
                    m.connect_prepared_notify(move |m| {
                        if m.is_prepared() && let Some(f) = once.take() {
                            f();
                        }
                    });
                }
                _ => reveal(),
            }
        });
        return;
    }
    if let Some(path) = file.path().filter(|_| ct.starts_with("audio/")) {
        let stage = look.stage.clone();
        let opened_for = uri.clone();
        let calm = look.calm.get();
        glib::spawn_future_local(async move {
            let opened = gio::spawn_blocking(move || crate::audio::open(&path)).await.ok().and_then(Result::ok);
            if !showing(&stage, &opened_for) {
                return;
            }
            let Some((source, audio)) = opened else {
                stage.append(&kit::ui::text("Can't read this file", "sub"));
                return;
            };
            // As for a video: the player starts once the card stopped moving.
            let wait = (calm - glib::monotonic_time()).max(0) / 1000;
            glib::timeout_future(std::time::Duration::from_millis(wait as u64)).await;
            if !showing(&stage, &opened_for) {
                return;
            }
            let media = gtk::MediaFile::for_input_stream(&crate::audio::Tagless::new(source, audio));
            let v = gtk::Video::for_media_stream(Some(&media));
            v.set_autoplay(true);
            v.set_size_request(maxw.min(960), 120);
            stage.append(&v);
        });
        return;
    }
    let texty = ct.starts_with("text/") || ["application/json", "application/xml", "application/javascript", "application/x-shellscript", "application/toml", "application/x-yaml", "application/x-desktop", "application/sql", "application/x-php", "application/x-perl", "application/x-ruby", "application/x-python"].contains(&ct.as_str()) || ct.ends_with("+xml") || ct.ends_with("+json");
    if texty {
        let view = gtk::TextView::new();
        view.set_editable(false);
        view.set_cursor_visible(false);
        view.set_monospace(true);
        view.add_css_class("look-text");
        // Room around the text, and a little between its lines.
        view.set_left_margin(16);
        view.set_right_margin(16);
        view.set_top_margin(14);
        view.set_bottom_margin(14);
        view.set_pixels_below_lines(2);
        view.set_pixels_inside_wrap(2);
        let tab = view.create_pango_layout(Some("    ")).pixel_size().0;
        let mut tabs = pango::TabArray::new(1, true);
        tabs.set_tab(0, pango::TabAlign::Left, tab.max(1));
        view.set_tabs(&tabs);
        // Prose wraps at the edge; code keeps its lines and scrolls across.
        let prose = prose(&ct);
        view.set_wrap_mode(if prose { gtk::WrapMode::WordChar } else { gtk::WrapMode::None });
        let sc = gtk::ScrolledWindow::new();
        sc.add_css_class("look-scroll");
        sc.set_overflow(gtk::Overflow::Hidden);
        sc.set_hscrollbar_policy(if prose { gtk::PolicyType::Never } else { gtk::PolicyType::Automatic });
        sc.set_size_request(maxw.min(900), maxh.min(620));
        sc.set_child(Some(&view));
        look.stage.append(&sc);
        let (f, stage, calm) = (file.clone(), look.stage.clone(), look.calm.get());
        glib::spawn_future_local(async move {
            let text = gio::spawn_blocking(move || {
                use std::io::Read;
                let path = f.path()?;
                let mut buf = Vec::new();
                std::fs::File::open(path).ok()?.take(256 * 1024).read_to_end(&mut buf).ok()?;
                Some(String::from_utf8_lossy(&buf).into_owned())
            })
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| "Can't read this file".into());
            // In once the card stopped moving: laying a long text out holds everything up.
            let wait = (calm - glib::monotonic_time()).max(0) / 1000;
            glib::timeout_future(std::time::Duration::from_millis(wait as u64)).await;
            if !showing(&stage, &uri) {
                return;
            }
            view.buffer().set_text(&text);
        });
        return;
    }
    let img = gtk::Image::from_gicon(&info.icon().unwrap_or_else(|| gio::ThemedIcon::new("text-x-generic").upcast()));
    img.set_pixel_size(256);
    look.stage.append(&img);
    let im = img.clone();
    crate::thumbs::texture_for(&file, &info, move |t| {
        if let Some(t) = t {
            im.set_paintable(Some(&t));
        }
    });
    if let Some(app) = gio::AppInfo::default_for_type(&ct, false) {
        let l = kit::ui::text(&format!("Opens with {}", app.display_name()), "sub");
        l.set_halign(gtk::Align::Center);
        look.stage.append(&l);
    }
}
