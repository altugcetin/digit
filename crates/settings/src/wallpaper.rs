//! Settings > Wallpaper: the picture, the folder it comes from, how it is shared between screens,
//! and, for a picture that fills a screen, which part of it stays in view.
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

/// The picture shown at a screen's shape, which the pointer drags to choose what stays in view.
///
/// The widgets own it through their handlers, and it holds them only weakly, so it lives as long
/// as they do and no longer.
struct Pic {
    area: glib::WeakRef<gtk::DrawingArea>,
    note: glib::WeakRef<gtk::Label>,
    pixels: RefCell<Option<Pixbuf>>,
    path: RefCell<Option<PathBuf>>,
    focus: Cell<Focus>,
    key: String,
    /// Where the screens divide a spanning picture, as fractions of its width and height.
    parts: Vec<Part>,
    asked: Cell<u64>,
}

impl Pic {
    fn new(key: &str, aspect: f64, parts: Vec<Part>, focus: Focus) -> (Rc<Self>, gtk::DrawingArea, gtk::Label) {
        let width = if aspect > 2.4 { 420 } else { 340 };
        let height = ((width as f64 / aspect).round() as i32).clamp(90, 280);
        let area = gtk::DrawingArea::new();
        area.set_content_width(width);
        area.set_content_height(height);
        area.set_halign(gtk::Align::Start);
        area.set_cursor_from_name(Some("grab"));
        let note = ui::text("", "dim");
        note.set_wrap(true);
        note.set_xalign(0.0);
        let pic = Rc::new(Self { area: area.downgrade(), note: note.downgrade(), pixels: RefCell::new(None), path: RefCell::new(None), focus: Cell::new(focus), key: key.into(), parts, asked: Cell::new(0) });
        let owner = pic.clone();
        area.set_draw_func(move |_, cr, w, h| owner.draw(cr, w as f64, h as f64));
        let drag = gtk::GestureDrag::new();
        let start = Rc::new(Cell::new(focus));
        let (pic_b, from) = (pic.clone(), start.clone());
        drag.connect_drag_begin(move |_, _, _| {
            from.set(pic_b.focus.get());
            if let Some(area) = pic_b.area.upgrade() { area.set_cursor_from_name(Some("grabbing")); }
        });
        let (pic_u, from) = (pic.clone(), start);
        drag.connect_drag_update(move |_, dx, dy| {
            let pic = &pic_u;
            let (over_x, over_y) = pic.overflow();
            let from = from.get();
            // Pulling the picture one way shows more of the side it came from.
            let x = if over_x > 1.0 { from.0 - dx / over_x } else { from.0 };
            let y = if over_y > 1.0 { from.1 - dy / over_y } else { from.1 };
            pic.focus.set(wallfocus::clamp((x, y)));
            if let Some(area) = pic.area.upgrade() { area.queue_draw(); }
        });
        let pic_e = pic.clone();
        drag.connect_drag_end(move |_, _, _| {
            if let Some(area) = pic_e.area.upgrade() { area.set_cursor_from_name(Some("grab")); }
            pic_e.store();
        });
        area.add_controller(drag);
        // A picture that has not arrived when the page is shown is asked for again.
        let pic_m = pic.clone();
        area.connect_map(move |_| {
            let path = pic_m.path.borrow().clone();
            if pic_m.pixels.borrow().is_none() && path.is_some() { pic_m.load(path); }
        });
        (pic, area, note)
    }

    /// How far the picture, scaled to cover the frame, runs past it sideways and up and down.
    fn overflow(&self) -> (f64, f64) {
        let Some(area) = self.area.upgrade() else { return (0.0, 0.0) };
        let (Some(pb), w, h) = (self.pixels.borrow().clone(), area.content_width() as f64, area.content_height() as f64) else { return (0.0, 0.0) };
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
        let pic = self.clone();
        glib::spawn_future_local(async move {
            let read = gio::spawn_blocking(move || {
                let pb = Pixbuf::from_file_at_scale(&path, 900, 900, true).ok()?;
                Some((pb.width(), pb.height(), pb.rowstride(), pb.has_alpha(), pb.read_pixel_bytes().to_vec()))
            })
            .await
            .ok()
            .flatten();
            if pic.asked.get() != ask { return; }
            *pic.pixels.borrow_mut() = read.map(|(w, h, stride, alpha, bytes)| Pixbuf::from_bytes(&glib::Bytes::from_owned(bytes), gtk::gdk_pixbuf::Colorspace::Rgb, alpha, 8, w, h, stride));
            pic.refresh();
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
        let r = 12.0;
        cr.new_sub_path();
        cr.arc(w - r, r, r, -std::f64::consts::FRAC_PI_2, 0.0);
        cr.arc(w - r, h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
        cr.arc(r, h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
        cr.arc(r, r, r, std::f64::consts::PI, 3.0 * std::f64::consts::FRAC_PI_2);
        cr.close_path();
        cr.save().ok();
        cr.clip_preserve();
        cr.set_source_rgba(0.5, 0.5, 0.5, 0.18);
        cr.paint().ok();
        if let Some(pb) = self.pixels.borrow().as_ref() {
            let (iw, ih) = (pb.width() as f64, pb.height() as f64);
            let (x, y, cw, ch) = wallfocus::crop((iw, ih), (w, h), self.focus.get());
            cr.save().ok();
            cr.scale(w / (cw * iw), h / (ch * ih));
            cr.set_source_pixbuf(pb, -x * iw, -y * ih);
            cr.source().set_filter(gtk::cairo::Filter::Good);
            cr.paint().ok();
            cr.restore().ok();
            // The seams between screens, where a spanning picture divides.
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.7);
            cr.set_line_width(1.5);
            cr.set_dash(&[5.0, 4.0], 0.0);
            for (px, py, pw, ph) in self.parts.iter().filter(|_| self.parts.len() > 1) {
                cr.rectangle(px * w + 0.75, py * h + 0.75, pw * w - 1.5, ph * h - 1.5);
                cr.stroke().ok();
            }
        }
        cr.restore().ok();
        cr.set_source_rgba(0.5, 0.5, 0.5, 0.35);
        cr.set_line_width(1.0);
        cr.new_sub_path();
        cr.arc(w - r - 0.5, r + 0.5, r, -std::f64::consts::FRAC_PI_2, 0.0);
        cr.arc(w - r - 0.5, h - r - 0.5, r, 0.0, std::f64::consts::FRAC_PI_2);
        cr.arc(r + 0.5, h - r - 0.5, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
        cr.arc(r + 0.5, r + 0.5, r, std::f64::consts::PI, 3.0 * std::f64::consts::FRAC_PI_2);
        cr.close_path();
        cr.stroke().ok();
    }
}

/// A row with the picture to drag, a note when there is nothing to drag, and a Reset that shows
/// on hover.
fn position(card: &gtk::Box, pic: &Rc<Pic>, area: &gtk::DrawingArea, note: &gtk::Label) {
    let (row, end) = ui::row(Some("open_with"), "Position", "Drag the picture to choose what stays in view when it fills the screen");
    let reset = kit::app::control("restart_alt", "on-hover");
    reset.set_tooltip_text(Some("Centre the picture"));
    let pic = pic.clone();
    reset.connect_clicked(move |_| pic.reset());
    end.append(&reset);
    card.append(&row);
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 8);
    holder.set_margin_start(16);
    holder.set_margin_end(16);
    holder.set_margin_bottom(14);
    holder.append(area);
    holder.append(note);
    card.append(&holder);
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
pub(crate) fn source(body: &gtk::Box, t: &toml::Table) {
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

/// The picture of each screen, how the screens share it, and where it is cut.
pub(crate) fn screens_section(body: &gtk::Box, t: &toml::Table) {
    let found = screens();
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 6);
    if found.len() > 1 {
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
    let focus = |key: &str| wallfocus::read(&t, key);
    let again = |holder: &gtk::Box| {
        let weak = holder.downgrade();
        move || {
            if let Some(holder) = weak.upgrade() {
                glib::idle_add_local_once(move || fill(&holder));
            }
        }
    };
    match layout {
        "span" => {
            let (w, h, parts) = span_of(&found);
            let card = section(holder, "Picture");
            let shown = global();
            let refill = again(holder);
            let refill = Rc::new(refill);
            choose(&card, "Picture on all screens", shown.as_deref(), start, move |p| {
                let refill = refill.clone();
                set_shared(p, move || refill());
            });
            let (pic, area, note) = Pic::new(wallfocus::SPAN, w / h, parts, focus(wallfocus::SPAN));
            pic.load(shown);
            position(&card, &pic, &area, &note);
        }
        "separate" => {
            for screen in &found {
                let card = section(holder, &format!("{} · {}×{}", screen.output, screen.w as i64, screen.h as i64));
                let shown = picture(&t, layout, &screen.output);
                let (refill, output) = (again(holder), screen.output.clone());
                choose(&card, "Picture", shown.as_deref(), start.clone(), move |p| {
                    if set_own(&output, &p) { refill(); }
                });
                let (pic, area, note) = Pic::new(&screen.output, screen.w / screen.h, Vec::new(), focus(&screen.output));
                pic.load(shown);
                position(&card, &pic, &area, &note);
            }
        }
        _ => {
            let card = section(holder, "Picture");
            let shown = global();
            let refill = again(holder);
            let refill = Rc::new(refill);
            choose(&card, if found.len() > 1 { "Picture on all screens" } else { "Picture" }, shown.as_deref(), start, move |p| {
                let refill = refill.clone();
                set_shared(p, move || refill());
            });
            for screen in &found {
                let card = section(holder, &if found.len() > 1 { format!("{} · {}×{}", screen.output, screen.w as i64, screen.h as i64) } else { "Position".to_string() });
                let (pic, area, note) = Pic::new(&screen.output, screen.w / screen.h, Vec::new(), focus(&screen.output));
                pic.load(shown.clone());
                position(&card, &pic, &area, &note);
            }
        }
    }
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
