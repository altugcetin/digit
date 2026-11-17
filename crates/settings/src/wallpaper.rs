//! The Desktop page's wallpaper part: the folder and slideshow, how the screens share a picture,
//! a picture for each screen and, for a picture that fills a screen, which part of it stays in
//! view. The small screens are also where desktop widgets are placed (see `Overlay`).
use crate::desktop::{choice, save, section, tiles};
use crate::{prefs, ui};
use gtk::gdk_pixbuf::Pixbuf;
use gtk::{gio, glib, prelude::*};
use kit::wallfocus::{self, Focus};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

const LAYOUTS: [(&str, &str); 3] = [("same", "Same on each"), ("span", "Span across"), ("separate", "One per screen")];

/// A screen as the picture sees it: its connector, its size and where it sits in the layout.
#[derive(Clone, Debug, PartialEq)]
struct Screen {
    output: String,
    w: f64,
    h: f64,
    x: f64,
    y: f64,
}

fn screens() -> Vec<Screen> {
    let Some(display) = gtk::gdk::Display::default() else { return Vec::new() };
    let places = kit::wm::outputs();
    display
        .monitors()
        .iter::<gtk::gdk::Monitor>()
        .filter_map(Result::ok)
        .map(|m| {
            let (g, output) = (m.geometry(), m.connector().map(|c| c.to_string()).unwrap_or_default());
            let at = places.iter().find(|p| p.name == output);
            Screen { w: g.width().max(1) as f64, h: g.height().max(1) as f64, x: at.map_or(g.x() as f64, |p| p.x), y: at.map_or(g.y() as f64, |p| p.y), output }
        })
        .collect()
}

/// A screen's part of a spanning picture: x, y, width and height as fractions of it.
type Part = (f64, f64, f64, f64);

/// What all the screens span: its size, and where each screen's part of it is.
fn span_of(screens: &[Screen]) -> (f64, f64, Vec<Part>) {
    let x0 = screens.iter().map(|s| s.x).fold(f64::MAX, f64::min);
    let y0 = screens.iter().map(|s| s.y).fold(f64::MAX, f64::min);
    let x1 = screens.iter().map(|s| s.x + s.w).fold(f64::MIN, f64::max);
    let y1 = screens.iter().map(|s| s.y + s.h).fold(f64::MIN, f64::max);
    let (w, h) = ((x1 - x0).max(1.0), (y1 - y0).max(1.0));
    (w, h, screens.iter().map(|s| ((s.x - x0) / w, (s.y - y0) / h, s.w / w, s.h / h)).collect())
}

fn layout_of(t: &toml::Table) -> &'static str {
    match prefs::get_str(t, "wallpaper_layout", "same").as_str() {
        "span" => "span",
        "separate" => "separate",
        _ => "same",
    }
}

/// The picture used everywhere that has none of its own: the one the shell remembers.
fn global() -> Option<PathBuf> {
    let path = std::fs::read_to_string(kit::theme::state_dir().join("wallpaper")).ok()?;
    let path = PathBuf::from(path.trim());
    path.is_file().then_some(path)
}

/// The picture a screen shows: its own in "one per screen", else the shared one.
fn picture(t: &toml::Table, layout: &str, output: &str) -> Option<PathBuf> {
    let own = (layout == "separate").then(|| t.get("wallpapers")?.as_table()?.get(output)?.as_str().map(PathBuf::from)).flatten();
    own.filter(|p| p.is_file()).or_else(global)
}

fn name_of(path: Option<&Path>) -> String {
    path.and_then(|p| p.file_stem()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "None yet".into())
}

/// Where the chooser opens: the wallpaper folder, else Pictures.
fn start_dir(t: &toml::Table) -> PathBuf {
    let custom = prefs::get_str(t, "wall_dir", "");
    if !custom.is_empty() && kit::prefs::home(&custom).is_dir() {
        return kit::prefs::home(&custom);
    }
    glib::user_special_dir(glib::UserDirectory::Pictures).unwrap_or_else(glib::home_dir)
}

/// Sets the picture of `output` ("one per screen"), keeping the choices of screens that are not
/// plugged in now.
fn set_own(output: &str, path: &Path) -> bool {
    let mut table = kit::prefs::read().get("wallpapers").and_then(toml::Value::as_table).cloned().unwrap_or_default();
    table.insert(output.into(), toml::Value::String(path.to_string_lossy().into_owned()));
    save("wallpapers", toml::Value::Table(table))
}

/// The `digit` program: beside this one first, since a desktop launch may not have
/// ~/.local/bin on its PATH.
fn digit() -> Option<PathBuf> {
    let beside = std::fs::read_link("/proc/self/exe").ok().and_then(|e| Some(e.parent()?.join("digit"))).filter(|p| p.is_file());
    beside.or_else(|| glib::find_program_in_path("digit"))
}

/// Sets the shared picture through the shell, which remembers it and recolours Digit from it,
/// and runs `then` once the shell has taken it (the page reads what the shell remembers).
fn set_shared(path: PathBuf, then: impl FnOnce() + 'static) {
    let Some(digit) = digit() else {
        ui::fail("Digit isn't installed, so the wallpaper can't be changed");
        return;
    };
    let argv: [&std::ffi::OsStr; 3] = [digit.as_os_str(), "wallpaper".as_ref(), path.as_os_str()];
    let Ok(p) = gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE) else {
        ui::fail("Could not reach digit to change the wallpaper");
        return;
    };
    glib::spawn_future_local(async move {
        if p.wait_check_future().await.is_err() {
            ui::fail("Digit could not change the wallpaper");
        }
        then();
    });
}

/// What draws over the wallpaper on the small screen of the primary display, and takes its clicks:
/// the desktop widgets (see `desktop::widget_overlay`).
pub(crate) struct Overlay {
    /// Draws in a box of the given size, with the pointer's spot highlighted.
    pub draw: DrawFn,
    /// The pointer moved to a place in the box (or left it): whether to redraw, and whether it is
    /// over something to click.
    pub moved: MovedFn,
    /// A click at a place in the box of the given size, whose corner sits at `origin` in the area.
    pub click: ClickFn,
    /// Called once with the area, to give it the actions its menus use.
    pub install: Box<dyn Fn(&gtk::DrawingArea)>,
}

pub(crate) type DrawFn = Box<dyn Fn(&gtk::cairo::Context, f64, f64)>;
pub(crate) type MovedFn = Box<dyn Fn(Option<(f64, f64)>, f64, f64) -> (bool, bool)>;
/// Arguments: the area, the box's corner in it, the box's size, and the click in the box.
pub(crate) type ClickFn = Box<dyn Fn(&gtk::DrawingArea, (f64, f64), (f64, f64), (f64, f64))>;

/// A screen's own picture: the file it shows now, where the chooser opens, and what to do with a pick.
struct Own {
    current: Option<PathBuf>,
    start: PathBuf,
    picked: Rc<dyn Fn(PathBuf)>,
}

/// A small screen: the picture as the screen shows it, which the pointer drags to choose what
/// stays in view, with the widgets over it on the primary screen.
///
/// The widgets own it through their handlers, and it holds them only weakly, so it lives as long
/// as they do and no longer.
struct Map {
    area: glib::WeakRef<gtk::DrawingArea>,
    note: glib::WeakRef<gtk::Label>,
    pixels: RefCell<Option<Pixbuf>>,
    /// The picture scaled to cover the map at its current size, made once: drawing is then a
    /// plain copy, which keeps scrolling the page light.
    covered: RefCell<Option<(i32, i32, gtk::cairo::ImageSurface)>>,
    path: RefCell<Option<PathBuf>>,
    focus: Cell<Focus>,
    key: String,
    /// Where the screens divide a spanning picture, as fractions of its width and height.
    parts: Vec<Part>,
    /// Where the overlay's box sits on the map, as fractions.
    spot: Part,
    overlay: Option<Overlay>,
    asked: Cell<u64>,
    moved: Cell<bool>,
}

impl Map {
    fn new(key: &str, aspect: f64, parts: Vec<Part>, overlay: Option<(Overlay, Part)>, focus: Focus) -> (Rc<Self>, gtk::Widget, gtk::Label) {
        let area = gtk::DrawingArea::new();
        area.add_css_class("desk-map");
        area.set_overflow(gtk::Overflow::Hidden);
        area.set_hexpand(true);
        area.set_content_width(80);
        area.set_content_height(45);
        area.set_cursor_from_name(Some("grab"));
        // Whatever width the page gives it, the map takes the shape of the screen.
        area.connect_resize(move |a, w, _| {
            let h = ((w as f64 / aspect).round() as i32).max(40);
            if a.content_height() != h { a.set_content_height(h); }
        });
        let note = ui::text("", "dim");
        note.set_wrap(true);
        note.set_xalign(0.0);
        let (overlay, spot) = match overlay {
            Some((o, p)) => (Some(o), p),
            None => (None, (0.0, 0.0, 1.0, 1.0)),
        };
        if let Some(o) = &overlay { (o.install)(&area); }
        let map = Rc::new(Self { area: area.downgrade(), note: note.downgrade(), pixels: RefCell::new(None), covered: RefCell::new(None), path: RefCell::new(None), focus: Cell::new(focus), key: key.into(), parts, spot, overlay, asked: Cell::new(0), moved: Cell::new(false) });
        let owner = map.clone();
        area.set_draw_func(move |_, cr, w, h| owner.draw(cr, w as f64, h as f64));
        let drag = gtk::GestureDrag::new();
        let start = Rc::new(Cell::new(focus));
        let (m, from) = (map.clone(), start.clone());
        drag.connect_drag_begin(move |_, _, _| {
            from.set(m.focus.get());
            m.moved.set(false);
        });
        let (m, from) = (map.clone(), start);
        drag.connect_drag_update(move |_, dx, dy| {
            // A click wobbles a little; only a real drag moves the picture.
            if dx.abs() + dy.abs() < 4.0 && !m.moved.get() { return; }
            m.moved.set(true);
            let (over_x, over_y) = m.overflow();
            let from = from.get();
            // Pulling the picture one way shows more of the side it came from.
            let x = if over_x > 1.0 { from.0 - dx / over_x } else { from.0 };
            let y = if over_y > 1.0 { from.1 - dy / over_y } else { from.1 };
            m.focus.set(wallfocus::clamp((x, y)));
            if let Some(area) = m.area.upgrade() {
                area.set_cursor_from_name(Some("grabbing"));
                area.queue_draw();
            }
        });
        let m = map.clone();
        drag.connect_drag_end(move |_, _, _| {
            if m.moved.get() { m.store(); }
            m.refresh();
        });
        area.add_controller(drag);
        let click = gtk::GestureClick::new();
        let m = map.clone();
        click.connect_released(move |g, _, x, y| {
            if m.moved.get() { return; }
            let (Some(o), Some(area)) = (m.overlay.as_ref(), g.widget().and_downcast::<gtk::DrawingArea>()) else { return };
            let (w, h) = (area.width() as f64, area.height() as f64);
            let (bx, by, bw, bh) = (m.spot.0 * w, m.spot.1 * h, m.spot.2 * w, m.spot.3 * h);
            if x >= bx && x <= bx + bw && y >= by && y <= by + bh { (o.click)(&area, (bx, by), (bw, bh), (x - bx, y - by)); }
        });
        area.add_controller(click);
        let motion = gtk::EventControllerMotion::new();
        let m = map.clone();
        motion.connect_motion(move |c, x, y| m.pointer(c, Some((x, y))));
        let m = map.clone();
        motion.connect_leave(move |c| m.pointer(c, None));
        area.add_controller(motion);
        // A picture that has not arrived when the page is shown is asked for again.
        let m = map.clone();
        area.connect_map(move |_| {
            let path = m.path.borrow().clone();
            if m.pixels.borrow().is_none() && path.is_some() { m.load(path); }
        });
        (map, area.upcast(), note)
    }

    fn pointer(&self, c: &gtk::EventControllerMotion, at: Option<(f64, f64)>) {
        let (Some(o), Some(area)) = (self.overlay.as_ref(), c.widget().and_downcast::<gtk::DrawingArea>()) else { return };
        let (w, h) = (area.width() as f64, area.height() as f64);
        let (bx, by, bw, bh) = (self.spot.0 * w, self.spot.1 * h, self.spot.2 * w, self.spot.3 * h);
        let local = at.filter(|(x, y)| *x >= bx && *x <= bx + bw && *y >= by && *y <= by + bh).map(|(x, y)| (x - bx, y - by));
        let (redraw, over) = (o.moved)(local, bw, bh);
        if redraw { area.queue_draw(); }
        let (over_x, over_y) = self.overflow();
        area.set_cursor_from_name(Some(if over { "pointer" } else if over_x >= 1.0 || over_y >= 1.0 { "grab" } else { "default" }));
    }

    /// How far the picture, scaled to cover the frame, runs past it sideways and up and down.
    fn overflow(&self) -> (f64, f64) {
        let Some(area) = self.area.upgrade() else { return (0.0, 0.0) };
        let (w, h) = (area.width().max(area.content_width()) as f64, area.height().max(area.content_height()) as f64);
        let Some(pb) = self.pixels.borrow().clone() else { return (0.0, 0.0) };
        let k = (w / pb.width() as f64).max(h / pb.height() as f64);
        (pb.width() as f64 * k - w, pb.height() as f64 * k - h)
    }

    fn store(&self) {
        if let Err(e) = wallfocus::save(&self.key, self.focus.get()) {
            ui::fail(&format!("Could not save: {e}"));
        }
    }

    fn reset(&self) {
        self.focus.set(wallfocus::CENTER);
        if let Some(area) = self.area.upgrade() { area.queue_draw(); }
        self.store();
    }

    /// Shows `path`, reading it off the main thread.
    fn load(self: &Rc<Self>, path: Option<PathBuf>) {
        *self.path.borrow_mut() = path.clone();
        let ask = self.asked.get() + 1;
        self.asked.set(ask);
        let Some(path) = path else {
            *self.pixels.borrow_mut() = None;
            self.refresh();
            return;
        };
        let map = self.clone();
        glib::spawn_future_local(async move {
            let read = gio::spawn_blocking(move || {
                let pb = Pixbuf::from_file_at_scale(&path, 1200, 1200, true).ok()?;
                Some((pb.width(), pb.height(), pb.rowstride(), pb.has_alpha(), pb.read_pixel_bytes().to_vec()))
            })
            .await
            .ok()
            .flatten();
            if map.asked.get() != ask { return; }
            *map.covered.borrow_mut() = None;
            *map.pixels.borrow_mut() = read.map(|(w, h, stride, alpha, bytes)| Pixbuf::from_bytes(&glib::Bytes::from_owned(bytes), gtk::gdk_pixbuf::Colorspace::Rgb, alpha, 8, w, h, stride));
            map.refresh();
        });
    }

    fn refresh(&self) {
        let (over_x, over_y) = self.overflow();
        let loaded = self.pixels.borrow().is_some();
        let text = if !loaded { "This picture can't be shown." } else if over_x < 1.0 && over_y < 1.0 { "This picture is the shape of the screen, so all of it shows." } else { "" };
        if let Some(note) = self.note.upgrade() {
            note.set_text(text);
            note.set_visible(!text.is_empty());
        }
        if let Some(area) = self.area.upgrade() {
            area.set_cursor_from_name(Some(if loaded && (over_x >= 1.0 || over_y >= 1.0) { "grab" } else { "default" }));
            area.queue_draw();
        }
    }

    fn draw(&self, cr: &gtk::cairo::Context, w: f64, h: f64) {
        use gtk::gdk::prelude::GdkCairoContextExt;
        let drawn = self.pixels.borrow().as_ref().and_then(|pb| {
            let (iw, ih) = (pb.width() as f64, pb.height() as f64);
            let k = (w / iw).max(h / ih);
            let (dw, dh) = ((iw * k).ceil().max(w.ceil()) as i32, (ih * k).ceil().max(h.ceil()) as i32);
            let mut covered = self.covered.borrow_mut();
            if covered.as_ref().is_none_or(|(cw, ch, _)| (*cw, *ch) != (dw, dh)) {
                let scaled = pb.scale_simple(dw, dh, gtk::gdk_pixbuf::InterpType::Bilinear)?;
                let surface = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, dw, dh).ok()?;
                let c = gtk::cairo::Context::new(&surface).ok()?;
                c.set_source_pixbuf(&scaled, 0.0, 0.0);
                c.paint().ok()?;
                *covered = Some((dw, dh, surface));
            }
            let (_, _, surface) = covered.as_ref()?;
            let f = wallfocus::clamp(self.focus.get());
            cr.set_source_surface(surface, -((dw as f64 - w) * f.0).round(), -((dh as f64 - h) * f.1).round()).ok()?;
            cr.paint().ok()
        });
        if drawn.is_none() {
            // Until the picture is there: the sky the widgets used to be drawn on.
            let g = gtk::cairo::LinearGradient::new(0.0, 0.0, w, h);
            g.add_color_stop_rgb(0.0, 0.34, 0.43, 0.62);
            g.add_color_stop_rgb(1.0, 0.86, 0.55, 0.42);
            cr.rectangle(0.0, 0.0, w, h);
            let _ = cr.set_source(&g);
            let _ = cr.fill();
        }
        // The seams between screens, where a spanning picture divides.
        if self.parts.len() > 1 {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.7);
            cr.set_line_width(1.5);
            cr.set_dash(&[5.0, 4.0], 0.0);
            for (px, py, pw, ph) in &self.parts {
                cr.rectangle(px * w + 0.75, py * h + 0.75, pw * w - 1.5, ph * h - 1.5);
                cr.stroke().ok();
            }
            cr.set_dash(&[], 0.0);
        }
        if let Some(o) = &self.overlay {
            cr.save().ok();
            let (bx, by, bw, bh) = (self.spot.0 * w, self.spot.1 * h, self.spot.2 * w, self.spot.3 * h);
            cr.rectangle(bx, by, bw, bh);
            cr.clip();
            cr.translate(bx, by);
            (o.draw)(cr, bw, bh);
            cr.restore().ok();
        }
    }
}

/// A row with the file's name and Choose…, which hands the picture on to `picked`.
fn choose(card: &gtk::Box, title: &str, current: Option<&Path>, start: PathBuf, picked: impl Fn(PathBuf) + 'static) {
    let (row, end) = ui::row(Some("image"), title, &name_of(current));
    let button = ui::button("Choose…", "");
    let picked = Rc::new(picked);
    button.connect_clicked(move |b| {
        let picked = picked.clone();
        ui::choose_image(ui::root(b), &start, move |path| picked(path));
    });
    end.append(&button);
    card.append(&row);
}

/// The folder the Wallpaper picker looks in, and the slideshow.
fn source(body: &gtk::Box, t: &toml::Table) {
    let card = section(body, "Wallpaper");
    let default = glib::user_special_dir(glib::UserDirectory::Pictures).unwrap_or_else(|| glib::home_dir().join("Pictures")).join("Wallpapers");
    let (row, end) = ui::row(Some("folder"), "Wallpaper folder", "Leave empty to use Pictures and its wallpaper folders");
    let entry = gtk::Entry::new();
    entry.set_text(&prefs::get_str(t, "wall_dir", ""));
    entry.set_width_chars(22);
    entry.set_hexpand(true);
    entry.set_placeholder_text(Some(&default.display().to_string()));
    ui::bind_entry(&entry, |text| {
        crate::desktop::screenshot_folder(text)?;
        if !text.is_empty() && std::fs::read_dir(kit::prefs::home(text)).is_err() {
            return Err("Choose a readable folder".into());
        }
        Ok(())
    }, |text| save("wall_dir", toml::Value::String(text.into())));
    end.append(&entry);
    let pick = ui::button("Choose…", "");
    let target = entry.downgrade();
    pick.connect_clicked(move |b| {
        let Some(entry) = target.upgrade() else { return };
        let path = if entry.text().trim().is_empty() { default.clone() } else { kit::prefs::home(entry.text().trim()) };
        let target = target.clone();
        ui::choose_folder(ui::root(b), &path, move |path| {
            if let Some(entry) = target.upgrade() {
                entry.set_text(&path.display().to_string());
                entry.emit_activate();
            }
        });
    });
    end.append(&pick);
    card.append(&row);
    let every = prefs::get_f64(t, "wallpaper_every", 0.0);
    let every = if every.is_finite() && every >= 1.0 { format!("{}", every.round() as i64) } else { "0".into() };
    choice(&card, "timer", "Change automatically", "Takes the next picture from the folder", "wallpaper_every", &[("0", "Off"), ("15", "Every 15 minutes"), ("30", "Every 30 minutes"), ("60", "Every hour"), ("180", "Every 3 hours"), ("1440", "Every day")], &every);
}

/// The wallpaper half of the Desktop page: the folder and slideshow, how several screens share a
/// picture, then each screen with its picture, its widgets and where the picture is cut.
pub(crate) fn section_for(body: &gtk::Box, t: &toml::Table) {
    source(body, t);
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 6);
    if screens().len() > 1 {
        let card = section(body, "Screens");
        let current = layout_of(t);
        let items = LAYOUTS.iter().map(|(v, name)| (*name, kit::pictures::wallpapers(v).upcast())).collect();
        let weak = holder.downgrade();
        tiles(&card, "wallpaper", "Wallpaper on several screens", "Each screen can have its own picture, or one picture can stretch across them", items, LAYOUTS.iter().position(|l| l.0 == current).unwrap_or(0), move |i| {
            if !save("wallpaper_layout", toml::Value::String(LAYOUTS[i].0.into())) { return false; }
            if let Some(holder) = weak.upgrade() {
                glib::idle_add_local_once(move || fill(&holder));
            }
            true
        });
    }
    body.append(&holder);
    fill(&holder);
}

/// A screen's small map, with a line under it: its name, a hover Reset and, when it has a picture
/// of its own, Choose….
///
/// The map takes `share` of the width it is given, centred, so a single screen doesn't fill the page.
fn block(map: &Rc<Map>, frame: &gtk::Widget, note: &gtk::Label, name: &str, share: f64, own: Option<Own>) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
    b.set_hexpand(true);
    frame.set_hexpand(false);
    frame.set_halign(gtk::Align::Center);
    b.append(frame);
    // A hairline as wide as the block tells the map how much room there is, only when it changes.
    let ruler = gtk::DrawingArea::new();
    ruler.set_hexpand(true);
    ruler.set_content_height(1);
    ruler.set_content_width(1);
    let area = frame.downgrade();
    ruler.connect_resize(move |_, width, _| {
        if let Some(area) = area.upgrade().and_then(|a| a.downcast::<gtk::DrawingArea>().ok()) {
            let w = ((width as f64 * share).round() as i32).max(80);
            if area.content_width() != w { area.set_content_width(w); }
        }
    });
    b.append(&ruler);
    b.append(note);
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let label = ui::text(name, "dim");
    label.set_hexpand(true);
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    line.append(&label);
    let reset = kit::app::control("restart_alt", "on-hover");
    reset.set_tooltip_text(Some("Centre the picture"));
    let m = map.clone();
    reset.connect_clicked(move |_| m.reset());
    line.append(&reset);
    if let Some(Own { current, start, picked }) = own {
        label.set_text(&format!("{name} · {}", name_of(current.as_deref())));
        let button = ui::button("Choose…", "");
        button.connect_clicked(move |b| {
            let picked = picked.clone();
            ui::choose_image(ui::root(b), &start, move |path| picked(path));
        });
        line.append(&button);
    }
    b.append(&line);
    b
}

/// Builds what belongs to the layout in force.
fn fill(holder: &gtk::Box) {
    while let Some(child) = holder.first_child() { holder.remove(&child); }
    let t = prefs::read();
    let found = screens();
    if found.is_empty() { return; }
    // What the shell reads, even with one screen plugged in: a saved "span" or "one per
    // screen" still decides which key and which picture it uses.
    let layout = layout_of(&t);
    let start = start_dir(&t);
    let primary = {
        let wanted = prefs::get_str(&t, "primary_monitor", "");
        found.iter().position(|s| s.output == wanted).unwrap_or(0)
    };
    let again = |holder: &gtk::Box| {
        let weak = holder.downgrade();
        Rc::new(move || {
            if let Some(holder) = weak.upgrade() {
                glib::idle_add_local_once(move || fill(&holder));
            }
        })
    };
    let card = section(holder, "Screens");
    let (hint, _) = ui::row(Some("dashboard_customize"), "Wallpaper and widgets", "Click a spot to choose what sits there. Drag the picture to choose what stays in view when it fills the screen");
    let shared = |card: &gtk::Box, title: &str| {
        let shown = global();
        let refill = again(holder);
        let start = start.clone();
        choose(card, title, shown.as_deref(), start, move |p| {
            let refill = refill.clone();
            set_shared(p, move || refill());
        });
        shown
    };
    let overlay = || crate::desktop::widget_overlay(&t);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    row.set_homogeneous(true);
    row.set_margin_start(16);
    row.set_margin_end(16);
    row.set_margin_bottom(14);
    match layout {
        "span" => {
            let (w, h, parts) = span_of(&found);
            let shown = shared(&card, "Picture on all screens");
            card.append(&hint);
            let (map, frame, note) = Map::new(wallfocus::SPAN, w / h, parts.clone(), Some((overlay(), parts[primary])), wallfocus::read(&t, wallfocus::SPAN));
            map.load(shown);
            row.append(&block(&map, &frame, &note, &format!("{} screens", found.len()), if found.len() > 1 { 0.7 } else { 0.5 }, None));
        }
        "separate" => {
            card.append(&hint);
            for (i, screen) in found.iter().enumerate() {
                let shown = picture(&t, layout, &screen.output);
                let (refill, output) = (again(holder), screen.output.clone());
                let own: Rc<dyn Fn(PathBuf)> = Rc::new(move |p| {
                    if set_own(&output, &p) { refill(); }
                });
                let (map, frame, note) = Map::new(&screen.output, screen.w / screen.h, Vec::new(), (i == primary).then(|| (overlay(), (0.0, 0.0, 1.0, 1.0))), wallfocus::read(&t, &screen.output));
                map.load(shown.clone());
                row.append(&block(&map, &frame, &note, &screen.output, if found.len() > 1 { 0.9 } else { 0.5 }, Some(Own { current: shown, start: start.clone(), picked: own })));
            }
        }
        _ => {
            let shown = shared(&card, if found.len() > 1 { "Picture on all screens" } else { "Picture" });
            card.append(&hint);
            for (i, screen) in found.iter().enumerate() {
                let (map, frame, note) = Map::new(&screen.output, screen.w / screen.h, Vec::new(), (i == primary).then(|| (overlay(), (0.0, 0.0, 1.0, 1.0))), wallfocus::read(&t, &screen.output));
                map.load(shown.clone());
                row.append(&block(&map, &frame, &note, &screen.output, if found.len() > 1 { 0.9 } else { 0.5 }, None));
            }
        }
    }
    card.append(&row);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(output: &str, x: f64, y: f64, w: f64, h: f64) -> Screen {
        Screen { output: output.into(), w, h, x, y }
    }

    #[test]
    fn a_span_divides_where_the_screens_sit() {
        let (w, h, parts) = span_of(&[screen("a", 0.0, 0.0, 1920.0, 1080.0), screen("b", 1920.0, 0.0, 1920.0, 1080.0)]);
        assert_eq!((w, h), (3840.0, 1080.0));
        assert_eq!(parts, [(0.0, 0.0, 0.5, 1.0), (0.5, 0.0, 0.5, 1.0)]);
        // A screen to the left of the origin, and a smaller one lower down.
        let (w, h, parts) = span_of(&[screen("a", -1000.0, 0.0, 1000.0, 1000.0), screen("b", 0.0, 500.0, 1000.0, 500.0)]);
        assert_eq!((w, h), (2000.0, 1000.0));
        assert_eq!(parts, [(0.0, 0.0, 0.5, 1.0), (0.5, 0.5, 0.5, 0.5)]);
    }

    #[test]
    fn a_screen_without_its_own_picture_shows_the_shared_one() {
        let t: toml::Table = "wallpaper_layout = 'separate'\n[wallpapers]\nDP-1 = '/no/such/file.png'\n".parse().unwrap();
        assert_eq!(layout_of(&t), "separate");
        assert_eq!(picture(&t, "separate", "DP-1"), global(), "a missing file falls back to the shared picture");
        assert_eq!(picture(&t, "same", "DP-1"), global());
        assert_eq!(layout_of(&"wallpaper_layout = 'bogus'".parse().unwrap()), "same");
    }

    #[test]
    fn names_show_the_file_without_its_folder_and_extension() {
        assert_eq!(name_of(Some(Path::new("/a/b/Forest Dawn.jpg"))), "Forest Dawn");
        assert_eq!(name_of(None), "None yet");
    }
}
