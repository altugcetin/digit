use gtk::{cairo, gdk, gio, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::io::Write;
use std::path::PathBuf;

pub struct Prefs {
    pub dir: PathBuf,
    pub name: String,
    pub format: String,
    pub after: String,
    pub save_on_copy: bool,
    pub cursor: bool,
    pub loupe: bool,
    pub notify: bool,
    pub palette: Vec<String>,
}

pub fn prefs() -> Prefs {
    let t = kit::prefs::read();
    let pictures = glib::user_special_dir(glib::UserDirectory::Pictures).unwrap_or_else(|| glib::home_dir().join("Pictures"));
    let dir = kit::prefs::text(&t, "snap_dir", "");
    let mut palette = kit::prefs::list(&t, "snap_palette");
    palette.retain(|c| c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|x| x.is_ascii_hexdigit()));
    if palette.is_empty() {
        palette = ["#ff3b30", "#ff9500", "#ffcc00", "#34c759", "#0a84ff", "#bf5af2", "#ffffff", "#111111"].iter().map(|s| s.to_string()).collect();
    }
    Prefs {
        dir: if dir.is_empty() { pictures.join("Screenshots") } else { kit::prefs::home(&dir) },
        name: kit::prefs::text(&t, "snap_name", "Screenshot %Y-%m-%d at %H.%M.%S"),
        format: kit::prefs::text(&t, "snap_format", "png"),
        after: kit::prefs::text(&t, "snap_after", "copy"),
        save_on_copy: kit::prefs::flag(&t, "snap_save_on_copy", true),
        cursor: kit::prefs::flag(&t, "snap_cursor", false),
        loupe: kit::prefs::flag(&t, "snap_loupe", false),
        notify: kit::prefs::flag(&t, "snap_notify", true),
        palette,
    }
}

pub fn texture(surface: &cairo::ImageSurface) -> Option<gdk::Texture> {
    surface.flush();
    let (w, h) = (surface.width(), surface.height());
    let mut copy = cairo::ImageSurface::create(cairo::Format::ARgb32, w, h).ok()?;
    {
        let cr = cairo::Context::new(&copy).ok()?;
        cr.set_source_surface(surface, 0.0, 0.0).ok()?;
        cr.paint().ok()?;
    }
    copy.flush();
    let data = copy.data().ok()?.to_vec();
    Some(gdk::MemoryTexture::new(w, h, gdk::MemoryFormat::B8g8r8a8Premultiplied, &glib::Bytes::from_owned(data), copy.stride() as usize).upcast())
}

pub fn encode(tex: &gdk::Texture, format: &str) -> (Vec<u8>, &'static str) {
    if format == "jpg" || format == "jpeg" {
        let (w, h) = (tex.width(), tex.height());
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        tex.download(&mut rgba, (w * 4) as usize);
        let mut rgb = Vec::with_capacity((w * h * 3) as usize);
        for px in rgba.chunks_exact(4) {
            rgb.extend_from_slice(&[px[2], px[1], px[0]]);
        }
        let pb = gtk::gdk_pixbuf::Pixbuf::from_bytes(&glib::Bytes::from_owned(rgb), gtk::gdk_pixbuf::Colorspace::Rgb, false, 8, w, h, w * 3);
        if let Ok(bytes) = pb.save_to_bufferv("jpeg", &[("quality", "92")]) {
            return (bytes, "jpg");
        }
    }
    (tex.save_to_png_bytes().to_vec(), "png")
}

pub fn filename(p: &Prefs, ext: &str) -> PathBuf {
    let stamp = glib::DateTime::now_local().ok().and_then(|d| d.format(&p.name).ok()).map(|s| s.to_string()).unwrap_or_else(|| "Screenshot".into());
    let stamp: String = stamp.chars().map(|c| if c == '/' { '-' } else { c }).collect();
    let mut path = p.dir.join(format!("{stamp}.{ext}"));
    let mut n = 2;
    while path.exists() {
        path = p.dir.join(format!("{stamp} ({n}).{ext}"));
        n += 1;
    }
    path
}

pub fn save(tex: &gdk::Texture, p: &Prefs) -> Result<PathBuf, String> {
    std::fs::create_dir_all(&p.dir).map_err(|e| e.to_string())?;
    let (bytes, ext) = encode(tex, &p.format);
    let path = filename(p, ext);
    let tmp = path.with_extension(format!("{ext}.part"));
    std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(path)
}

pub fn save_as(tex: gdk::Texture, parent: Option<&gtk::Window>, done: impl FnOnce(Option<PathBuf>) + 'static) {
    let p = prefs();
    let d = gtk::FileDialog::new();
    d.set_title("Save Screenshot");
    let _ = std::fs::create_dir_all(&p.dir);
    d.set_initial_folder(Some(&gio::File::for_path(&p.dir)));
    d.set_initial_name(Some(&filename(&p, if p.format == "jpg" { "jpg" } else { "png" }).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
    d.save(parent, None::<&gio::Cancellable>, move |r| {
        let Ok(file) = r else { return done(None) };
        let Some(path) = file.path() else { return done(None) };
        let fmt = if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg")) { "jpg" } else { "png" };
        let (bytes, _) = encode(&tex, fmt);
        done(std::fs::write(&path, bytes).ok().map(|_| path));
    });
}

pub fn copy(tex: &gdk::Texture, app: &gtk::Application) {
    let png = tex.save_to_png_bytes();
    if glib::find_program_in_path("wl-copy").is_some() {
        if let Ok(mut child) = std::process::Command::new("wl-copy").args(["--type", "image/png"]).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn() {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(&png);
            }
            let _ = child.wait();
            return;
        }
    }
    let Some(display) = gdk::Display::default() else { return };
    let clip = display.clipboard();
    clip.set_texture(tex);
    let guard = std::rc::Rc::new(std::cell::RefCell::new(Some(app.hold())));
    let g = guard.clone();
    clip.connect_changed(move |c| {
        if !c.is_local() {
            g.borrow_mut().take();
        }
    });
    glib::timeout_add_local_once(std::time::Duration::from_secs(600), move || {
        guard.borrow_mut().take();
    });
}

pub fn copy_text(text: &str) {
    if glib::find_program_in_path("wl-copy").is_some() {
        if let Ok(mut child) = std::process::Command::new("wl-copy").stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn() {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            let _ = child.wait();
            return;
        }
    }
    if let Some(d) = gdk::Display::default() {
        d.clipboard().set_text(text);
    }
}

pub fn notify(summary: &str, body: &str, image: Option<&std::path::Path>) {
    let summary = summary.to_string();
    let body = body.to_string();
    let image = image.map(|p| p.to_string_lossy().into_owned());
    glib::spawn_future_local(async move {
        let Ok(conn) = gio::bus_get_future(gio::BusType::Session).await else { return };
        let hints = glib::VariantDict::new(None);
        hints.insert("desktop-entry", crate::APP_ID);
        if let Some(i) = &image {
            hints.insert("image-path", i.as_str());
        }
        let actions: Vec<String> = Vec::new();
        let args = ("Screenshot", 0u32, crate::APP_ID, summary.as_str(), body.as_str(), actions, hints.end(), 4000i32).to_variant();
        let _ = conn.call_future(Some("org.freedesktop.Notifications"), "/org/freedesktop/Notifications", "org.freedesktop.Notifications", "Notify", Some(&args), None, gio::DBusCallFlags::NONE, 3000).await;
    });
}

pub fn swatch(rgb: (u8, u8, u8)) -> Option<PathBuf> {
    let size = 96;
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, size, size).ok()?;
    {
        let cr = cairo::Context::new(&surface).ok()?;
        kit::paint::rounded(&cr, 0.0, 0.0, size as f64, size as f64, 22.0);
        cr.set_source_rgb(rgb.0 as f64 / 255.0, rgb.1 as f64 / 255.0, rgb.2 as f64 / 255.0);
        cr.fill().ok()?;
    }
    let tex = texture(&surface)?;
    let path = glib::user_runtime_dir().join(format!("digit-color-{:02x}{:02x}{:02x}.png", rgb.0, rgb.1, rgb.2));
    tex.save_to_png(&path).ok()?;
    Some(path)
}

pub fn ocr_available() -> bool {
    glib::find_program_in_path("tesseract").is_some()
}

pub fn ocr(tex: &gdk::Texture) {
    let png = tex.save_to_png_bytes();
    let tmp = glib::user_runtime_dir().join(format!("digit-ocr-{}.png", std::process::id()));
    if std::fs::write(&tmp, &png).is_err() {
        return;
    }
    let langs = kit::prefs::text(&kit::prefs::read(), "snap_ocr_langs", "eng+tur");
    let out = std::process::Command::new("tesseract").arg(&tmp).arg("-").args(["-l", &langs]).stderr(std::process::Stdio::null()).output();
    let _ = std::fs::remove_file(&tmp);
    match out {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if text.is_empty() {
                notify("No text found", "The selection has no readable text", None);
            } else {
                copy_text(&text);
                notify("Text copied", &text.chars().take(120).collect::<String>(), None);
            }
        }
        _ => notify("Couldn't read text", "tesseract failed; check the installed languages", None),
    }
}

pub fn pin(app: &gtk::Application, tex: gdk::Texture, output: &str, x: i32, y: i32, w: i32, h: i32) {
    let win = gtk::Window::new();
    win.set_application(Some(app));
    win.add_css_class("pin");
    win.init_layer_shell();
    win.set_namespace(Some("digit-pin"));
    win.set_layer(Layer::Top);
    win.set_keyboard_mode(KeyboardMode::OnDemand);
    win.set_anchor(Edge::Top, true);
    win.set_anchor(Edge::Left, true);
    win.set_exclusive_zone(-1);
    if let Some(d) = gdk::Display::default() {
        let m = d.monitors().iter::<gdk::Monitor>().filter_map(Result::ok).find(|m| m.connector().is_some_and(|c| c == output));
        win.set_monitor(m.as_ref());
    }
    let max = 560.0f64;
    let k = (max / w.max(h) as f64).min(1.0);
    let (pw, ph) = ((w as f64 * k).round() as i32, (h as f64 * k).round() as i32);
    win.set_margin(Edge::Left, x);
    win.set_margin(Edge::Top, y);
    let pic = gtk::Picture::for_paintable(&tex);
    pic.set_content_fit(gtk::ContentFit::Fill);
    pic.set_size_request(pw, ph);
    pic.set_can_shrink(true);
    let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
    frame.add_css_class("pinned");
    frame.set_overflow(gtk::Overflow::Hidden);
    frame.append(&pic);
    win.set_child(Some(&frame));
    let size = std::rc::Rc::new(std::cell::Cell::new((pw, ph)));
    let drag = gtk::GestureDrag::new();
    let start = std::rc::Rc::new(std::cell::Cell::new((x, y)));
    let w2 = win.clone();
    let s2 = start.clone();
    drag.connect_drag_begin(move |_, _, _| s2.set((w2.margin(Edge::Left), w2.margin(Edge::Top))));
    let w2 = win.clone();
    drag.connect_drag_update(move |_, dx, dy| {
        let (sx, sy) = start.get();
        w2.set_margin(Edge::Left, (sx + dx as i32).max(0));
        w2.set_margin(Edge::Top, (sy + dy as i32).max(0));
    });
    win.add_controller(drag);
    let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    let p2 = pic.clone();
    let ratio = h as f64 / w as f64;
    wheel.connect_scroll(move |_, _, dy| {
        let (cw, _) = size.get();
        let nw = ((cw as f64) * if dy < 0.0 { 1.1 } else { 1.0 / 1.1 }).clamp(80.0, 2400.0).round() as i32;
        let nh = (nw as f64 * ratio).round() as i32;
        size.set((nw, nh));
        p2.set_size_request(nw, nh);
        glib::Propagation::Stop
    });
    win.add_controller(wheel);
    let click = gtk::GestureClick::new();
    click.set_button(0);
    let w2 = win.clone();
    let t2 = tex.clone();
    let a2 = app.clone();
    click.connect_pressed(move |g, n, _, _| {
        if g.current_button() == 3 {
            let menu = gio::Menu::new();
            menu.append(Some("Copy"), Some("pin.copy"));
            menu.append(Some("Save"), Some("pin.save"));
            menu.append(Some("Close"), Some("pin.close"));
            let pop = gtk::PopoverMenu::from_model(Some(&menu));
            pop.set_parent(&w2);
            pop.set_has_arrow(false);
            pop.connect_closed(|p| {
                let p = p.clone();
                glib::idle_add_local_once(move || p.unparent());
            });
            pop.popup();
        } else if n == 2 {
            w2.close();
        }
        let _ = (&t2, &a2);
    });
    win.add_controller(click);
    let group = gio::SimpleActionGroup::new();
    let a = gio::SimpleAction::new("copy", None);
    let (t2, a2) = (tex.clone(), app.clone());
    a.connect_activate(move |_, _| copy(&t2, &a2));
    group.add_action(&a);
    let a = gio::SimpleAction::new("save", None);
    let t2 = tex.clone();
    a.connect_activate(move |_, _| {
        let p = prefs();
        if let Ok(path) = save(&t2, &p) {
            notify("Screenshot saved", &path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), Some(&path));
        }
    });
    group.add_action(&a);
    let a = gio::SimpleAction::new("close", None);
    let w2 = win.clone();
    a.connect_activate(move |_, _| w2.close());
    group.add_action(&a);
    win.insert_action_group("pin", Some(&group));
    let keys = gtk::EventControllerKey::new();
    let w2 = win.clone();
    let (t2, a2) = (tex, app.clone());
    keys.connect_key_pressed(move |_, k, _, m| {
        match k {
            gdk::Key::Escape => w2.close(),
            gdk::Key::c if m.contains(gdk::ModifierType::CONTROL_MASK) => copy(&t2, &a2),
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    win.add_controller(keys);
    win.set_tooltip_text(Some("Drag to move · scroll to resize · double-click to close"));
    win.present();
}
