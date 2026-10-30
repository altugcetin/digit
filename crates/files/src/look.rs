use crate::pane::{is_dir, kind, size_text, target_of};
use gtk::{gdk, gio, glib, pango, prelude::*};
use kit::motion::{self, Tween};
use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

const OPEN_MS: i64 = 260;
const CLOSE_MS: i64 = 150;
pub const DROP: f64 = 18.0;
const MARGIN: i32 = 36;

struct Look {
    root: gtk::Box,
    card: gtk::Box,
    stage: gtk::Box,
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
        play(&l.root, &l.card, MARGIN, false, move || {
            kit::ui::clear(&stage);
            crate::overlay_remove(&root);
            kit::app::trim_later(600);
        });
    }
}

/// Fades the backdrop and settles the card from slightly below, like the shell's panels; closing runs it back, quicker.
/// Quick Look and every dialog share it; `margin` is the card's resting top and bottom margin.
pub fn play(root: &gtk::Box, card: &gtk::Box, margin: i32, opening: bool, done: impl FnOnce() + 'static) {
    let now = glib::monotonic_time();
    let mut t = Tween::still(root.opacity());
    if opening {
        t.go(1.0, now, OPEN_MS, motion::Curve::new(0.16, 1.0, 0.3, 1.0));
    } else {
        t.go(0.0, now, CLOSE_MS, motion::Curve::new(0.4, 0.0, 0.9, 0.6));
    }
    let card = card.clone();
    let done = std::cell::Cell::new(Some(done));
    root.add_tick_callback(move |w, clock| {
        let now = clock.frame_time();
        let v = t.value(now).clamp(0.0, 1.0);
        w.set_opacity(v);
        let off = ((1.0 - v) * DROP).round() as i32;
        card.set_margin_top(margin + off);
        card.set_margin_bottom(margin - off);
        if !t.settled(now) {
            return glib::ControlFlow::Continue;
        }
        if let Some(f) = done.take() {
            f();
        }
        glib::ControlFlow::Break
    });
}

fn show() {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("look-backdrop");
    root.set_hexpand(true);
    root.set_vexpand(true);
    let card = gtk::Box::new(gtk::Orientation::Vertical, 12);
    card.add_css_class("look");
    card.set_halign(gtk::Align::Center);
    card.set_valign(gtk::Align::Center);
    card.set_margin_top(MARGIN + DROP as i32);
    card.set_margin_bottom(MARGIN - DROP as i32);
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
    card.append(&stage);
    root.append(&card);
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
    let look = std::rc::Rc::new(Look { root: root.clone(), card: card.clone(), stage, title, sub, shown: RefCell::new(None), busy: RefCell::new(None) });
    LOOK.with(|l| *l.borrow_mut() = Some(look));
    crate::overlay_add(&root);
    play(&root, &card, MARGIN, true, || {});
    update();
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
    *look.shown.borrow_mut() = Some(uri);
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
    if ct.starts_with("video/") || ct.starts_with("audio/") {
        let v = gtk::Video::for_file(Some(&file));
        v.set_autoplay(true);
        v.set_size_request(maxw.min(960), if ct.starts_with("audio/") { 120 } else { maxh.min(560) });
        look.stage.append(&v);
        return;
    }
    let texty = ct.starts_with("text/") || ["application/json", "application/xml", "application/javascript", "application/x-shellscript", "application/toml", "application/x-yaml", "application/x-desktop", "application/sql", "application/x-php", "application/x-perl", "application/x-ruby", "application/x-python"].contains(&ct.as_str()) || ct.ends_with("+xml") || ct.ends_with("+json");
    if texty {
        let view = gtk::TextView::new();
        view.set_editable(false);
        view.set_cursor_visible(false);
        view.set_monospace(true);
        view.add_css_class("look-text");
        view.set_wrap_mode(gtk::WrapMode::WordChar);
        let sc = gtk::ScrolledWindow::new();
        sc.set_size_request(maxw.min(900), maxh.min(620));
        sc.set_child(Some(&view));
        look.stage.append(&sc);
        let f = file.clone();
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
