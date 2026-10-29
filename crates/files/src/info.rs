use crate::pane::{file_of, is_dir, kind, size_text, target_of, when};
use gtk::{gio, glib, pango, prelude::*};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

pub struct Info {
    pub root: gtk::Box,
    body: gtk::Box,
    busy: RefCell<Option<Arc<AtomicBool>>>,
}

fn line(body: &gtk::Box, label: &str, value: &str) -> gtk::Label {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 2);
    b.append(&kit::ui::text(label, "sub"));
    let v = kit::ui::text(value, "");
    v.set_wrap(true);
    v.set_wrap_mode(pango::WrapMode::WordChar);
    v.set_selectable(true);
    v.set_xalign(0.0);
    b.append(&v);
    body.append(&b);
    v
}

pub fn mode(m: u32) -> String {
    let bits = [(0o400, 'r'), (0o200, 'w'), (0o100, 'x'), (0o040, 'r'), (0o020, 'w'), (0o010, 'x'), (0o004, 'r'), (0o002, 'w'), (0o001, 'x')];
    let s: String = bits.iter().map(|(b, c)| if m & b != 0 { *c } else { '-' }).collect();
    format!("{s}  ({:o})", m & 0o7777)
}

impl Info {
    pub fn new() -> Rc<Info> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("info");
        root.set_size_request(300, -1);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
        scroll.set_vexpand(true);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 14);
        body.add_css_class("info-body");
        scroll.set_child(Some(&body));
        root.append(&scroll);
        Rc::new(Info { root, body, busy: RefCell::new(None) })
    }

    pub fn show(&self, infos: &[gio::FileInfo], here: &gio::File) {
        if let Some(b) = self.busy.borrow_mut().take() {
            b.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        kit::ui::clear(&self.body);
        match infos {
            [] => {
                let f = here.clone();
                let picture = gtk::Image::new();
                crate::art::set(&picture, &f, 128);
                picture.add_css_class("info-art");
                self.body.append(&picture);
                let name = f.query_info("standard::display-name", gio::FileQueryInfoFlags::NONE, None::<&gio::Cancellable>).map(|i| i.display_name().to_string()).unwrap_or_default();
                let t = kit::ui::text(&name, "name big");
                t.set_wrap(true);
                self.body.append(&t);
                if let Ok(fs) = f.query_filesystem_info("filesystem::free,filesystem::size,filesystem::type", None::<&gio::Cancellable>) {
                    let free = fs.attribute_uint64("filesystem::free");
                    let size = fs.attribute_uint64("filesystem::size");
                    if size > 0 {
                        line(&self.body, "Free space", &format!("{} of {}", size_text(free), size_text(size)));
                        let bar = gtk::ProgressBar::new();
                        bar.set_fraction(1.0 - free as f64 / size as f64);
                        self.body.append(&bar);
                    }
                    if let Some(t) = fs.attribute_string("filesystem::type") {
                        line(&self.body, "Filesystem", &t);
                    }
                }
                line(&self.body, "Location", &f.path().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| f.uri().to_string()));
            }
            [one] => self.single(one),
            many => {
                let picture = gtk::Image::from_icon_name("edit-select-all");
                picture.set_pixel_size(96);
                picture.add_css_class("info-art");
                self.body.append(&picture);
                self.body.append(&kit::ui::text(&format!("{} items", many.len()), "name big"));
                let bytes: i64 = many.iter().filter(|i| !is_dir(i)).map(|i| i.size()).sum();
                let dirs = many.iter().filter(|i| is_dir(i)).count();
                line(&self.body, "Files", &format!("{} · {}", many.len() - dirs, size_text(bytes.max(0) as u64)));
                if dirs > 0 {
                    line(&self.body, "Folders", &dirs.to_string());
                }
            }
        }
    }

    fn single(&self, info: &gio::FileInfo) {
        let Some(file) = file_of(info) else { return };
        let img = gtk::Image::from_gicon(&info.icon().unwrap_or_else(|| gio::ThemedIcon::new("text-x-generic").upcast()));
        img.set_pixel_size(160);
        img.add_css_class("info-art");
        self.body.append(&img);
        let target = target_of(info).unwrap_or(file.clone());
        if is_dir(info) {
            crate::art::set(&img, &target, 160);
        }
        let im = img.clone();
        crate::thumbs::texture_for(&target, info, move |t| {
            if let Some(t) = t {
                im.set_paintable(Some(&t));
            }
        });
        let t = kit::ui::text(&info.display_name(), "name big");
        t.set_wrap(true);
        t.set_wrap_mode(pango::WrapMode::WordChar);
        t.set_selectable(true);
        self.body.append(&t);
        self.body.append(&kit::ui::text(&kind(info), "sub"));
        if is_dir(info) {
            let v = line(&self.body, "Size", "Calculating…");
            let stop = crate::ops::sizes(target.clone(), move |bytes, count| v.set_text(&format!("{} · {count} items", size_text(bytes))));
            *self.busy.borrow_mut() = Some(stop);
        } else {
            line(&self.body, "Size", &format!("{} ({} bytes)", size_text(info.size().max(0) as u64), info.size()));
        }
        if let Some(d) = info.modification_date_time() {
            line(&self.body, "Modified", &when(&d));
        }
        if let Ok(full) = target.query_info("time::created,time::access,owner::user,owner::group", gio::FileQueryInfoFlags::NONE, None::<&gio::Cancellable>) {
            if full.has_attribute("time::created") {
                if let Ok(d) = glib::DateTime::from_unix_local(full.attribute_uint64("time::created") as i64) {
                    line(&self.body, "Created", &when(&d));
                }
            }
            if let Some(u) = full.attribute_string("owner::user") {
                line(&self.body, "Owner", &format!("{u}{}", full.attribute_string("owner::group").map(|g| format!(" · {g}")).unwrap_or_default()));
            }
        }
        if info.has_attribute("unix::mode") {
            line(&self.body, "Permissions", &mode(info.attribute_uint32("unix::mode")));
        }
        let ct = info.attribute_string("standard::fast-content-type").map(|s| s.to_string()).unwrap_or_default();
        if ct.starts_with("image/") {
            if let Some(p) = target.path() {
                if let Some((_, w, h)) = gtk::gdk_pixbuf::Pixbuf::file_info(&p) {
                    line(&self.body, "Dimensions", &format!("{w} × {h}"));
                }
            }
        }
        if let Some(orig) = info.attribute_byte_string("trash::orig-path") {
            line(&self.body, "Was in", &orig);
        }
        if let Some(app) = (!is_dir(info)).then(|| gio::AppInfo::default_for_type(&ct, false)).flatten() {
            line(&self.body, "Opens with", &app.display_name());
        }
        line(&self.body, "Location", &target.path().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| target.uri().to_string()));
        let _ = glib::Priority::DEFAULT;
    }
}
