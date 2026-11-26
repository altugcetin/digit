//! SVG icons with text in them, drawn so GTK's own SVG renderer can't take the shell down.
//!
//! GTK 4.24 draws SVG icons itself. When a `<text>` in one holds no characters (an empty
//! `<tspan/>`, a lone space: org.gnome.Characters' icon has one), its first drawing leaves the
//! text's layouts behind, and the next drawing at another size aborts the whole process
//! (`generate_layouts: assertion failed: (node->characters.chunks == NULL)`; GTK's development
//! branch has rewritten that code). The icon theme hands the same paintable to everyone who
//! asks, and the app grid, the dock's styles and magnification, notifications and the tray all
//! draw icons at more than one size.
//!
//! So an SVG icon with any text in it is drawn here from a copy of its own, always at the one
//! size it was asked for and scaled from there: GTK keeps what it drew for a size and never lays
//! the text out a second time. Every other icon is passed through untouched.

use gtk::{gdk, gio, glib, prelude::*, subclass::prelude::*};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

mod imp {
    use super::*;
    use std::cell::OnceCell;

    #[derive(Default)]
    pub struct Steady {
        pub file: OnceCell<gio::File>,
        pub copy: OnceCell<gtk::IconPaintable>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Steady {
        const NAME: &'static str = "DigitSteadyIcon";
        type Type = super::Steady;
        type Interfaces = (gdk::Paintable,);
    }

    impl ObjectImpl for Steady {}

    impl PaintableImpl for Steady {
        fn flags(&self) -> gdk::PaintableFlags {
            gdk::PaintableFlags::STATIC_SIZE | gdk::PaintableFlags::STATIC_CONTENTS
        }

        fn intrinsic_width(&self) -> i32 {
            self.copy.get().map_or(0, |c| c.intrinsic_width())
        }

        fn intrinsic_height(&self) -> i32 {
            self.copy.get().map_or(0, |c| c.intrinsic_height())
        }

        fn snapshot(&self, snapshot: &gdk::Snapshot, width: f64, height: f64) {
            let (Some(copy), Some(s)) = (self.copy.get(), snapshot.downcast_ref::<gtk::Snapshot>()) else { return };
            if width <= 0.0 || height <= 0.0 {
                return;
            }
            let (w, h) = (copy.intrinsic_width().max(1) as f64, copy.intrinsic_height().max(1) as f64);
            s.save();
            s.scale((width / w) as f32, (height / h) as f32);
            copy.snapshot(s, w, h);
            s.restore();
        }
    }
}

glib::wrapper! {
    pub struct Steady(ObjectSubclass<imp::Steady>) @implements gdk::Paintable;
}

impl Steady {
    fn new(file: &gio::File, size: i32) -> Self {
        let icon: Self = glib::Object::new();
        let _ = icon.imp().file.set(file.clone());
        let _ = icon.imp().copy.set(gtk::IconPaintable::for_file(file, size.max(1), 1));
        icon
    }
}

/// Copies kept, one per file and size: drawing code asks again every frame.
const KEPT: usize = 128;

thread_local! {
    static TEXT: RefCell<HashMap<PathBuf, bool>> = RefCell::new(HashMap::new());
    static COPIES: RefCell<HashMap<(String, i32), Steady>> = RefCell::new(HashMap::new());
}

/// Whether the SVG has a text element in it, with or without a namespace prefix. Matching a
/// little too much only costs the icon a copy of its own.
fn has_text(svg: &[u8]) -> bool {
    svg.windows(6).any(|w| matches!(w[0], b'<' | b':') && &w[1..5] == b"text" && matches!(w[5], b' ' | b'\t' | b'\n' | b'\r' | b'>' | b'/'))
}

fn risky(path: &Path) -> bool {
    if path.extension().is_none_or(|e| e != "svg") {
        return false;
    }
    if let Some(known) = TEXT.with(|t| t.borrow().get(path).copied()) {
        return known;
    }
    let text = std::fs::read(path).is_ok_and(|b| has_text(&b));
    TEXT.with(|t| {
        let mut t = t.borrow_mut();
        if t.len() >= 1024 {
            t.clear();
        }
        t.insert(path.to_owned(), text);
    });
    text
}

/// `icon`, or for an SVG icon file with text in it, the same artwork drawn from a copy of its
/// own at one size, which can then be drawn at any size. Symbolic icons are left as they are, to
/// be recolored as they're drawn.
pub fn steady(icon: &gdk::Paintable) -> gdk::Paintable {
    let Some(file) = icon.downcast_ref::<gtk::IconPaintable>().filter(|p| !p.is_symbolic()).and_then(|p| p.file()) else { return icon.clone() };
    if !file.path().is_some_and(|p| risky(&p)) {
        return icon.clone();
    }
    let key = (file.uri().to_string(), icon.intrinsic_width());
    COPIES.with(|c| {
        let mut c = c.borrow_mut();
        if let Some(s) = c.get(&key) {
            return s.clone().upcast();
        }
        if c.len() >= KEPT {
            c.clear();
        }
        let s = Steady::new(&file, key.1);
        c.insert(key, s.clone());
        s.upcast()
    })
}

/// The file an icon is drawn from, a steady copy's too.
pub fn file(icon: &gdk::Paintable) -> Option<gio::File> {
    match icon.downcast_ref::<Steady>() {
        Some(s) => s.imp().file.get().cloned(),
        None => icon.downcast_ref::<gtk::IconPaintable>().and_then(|p| p.file()),
    }
}

/// An icon like org.gnome.Characters': a `<text>` with nothing in it but an empty `<tspan/>`.
#[cfg(test)]
pub(crate) const EMPTY_TEXT_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><rect width="16" height="16" fill="#3584e4"/><text x="2" y="12"><tspan x="2" y="12"/></text></svg>"##;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alone;

    #[test]
    fn finds_text_elements_with_or_without_a_prefix() {
        assert!(has_text(br#"<svg><text x="2"> </text></svg>"#));
        assert!(has_text(b"<svg><text>A</text></svg>"));
        assert!(has_text(b"<svg:svg><svg:text\n x='1'/></svg:svg>"));
        assert!(!has_text(br#"<svg><path style="text-anchor:start" d="M0 0"/><textPath/></svg>"#));
        assert!(!has_text(br#"<svg><rect width="16" height="16"/></svg>"#));
    }

    #[test]
    fn icon_with_empty_text_draws_at_many_sizes_without_aborting() {
        if !alone("svgicon::tests::icon_with_empty_text_draws_at_many_sizes_without_aborting") || gtk::init().is_err() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("digit-svgicon-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("empty-text.svg");
        std::fs::write(&path, EMPTY_TEXT_SVG).unwrap();
        let theme_icon: gdk::Paintable = gtk::IconPaintable::for_file(&gio::File::for_path(&path), 48, 1).upcast();
        let icon = steady(&theme_icon);
        assert!(icon.is::<Steady>());
        assert_eq!(icon.intrinsic_width(), 48);
        assert_eq!(file(&icon).and_then(|f| f.path()), Some(path.clone()));
        assert!(steady(&theme_icon) == icon, "asked again, the same copy comes back");
        // Each size here is a fresh layout of the text for GTK's renderer; the second one is
        // where GTK 4.24 aborted.
        for size in [64.0, 48.0, 71.5, 30.0, 64.0] {
            let s = gtk::Snapshot::new();
            icon.snapshot(&s, size, size);
            let node = s.to_node().expect("the icon draws something");
            let b = node.bounds();
            assert!((b.width() as f64 - size).abs() < 0.5 && (b.height() as f64 - size).abs() < 0.5, "{size}: {b:?}");
        }
        let plain: gdk::Paintable = gtk::IconPaintable::for_file(&gio::File::for_path(dir.join("plain.png")), 48, 1).upcast();
        assert!(steady(&plain) == plain, "an icon that isn't an SVG with text is passed through");
        let symbolic_path = dir.join("empty-text-symbolic.svg");
        std::fs::write(&symbolic_path, EMPTY_TEXT_SVG).unwrap();
        let symbolic: gdk::Paintable = gtk::IconPaintable::for_file(&gio::File::for_path(&symbolic_path), 48, 1).upcast();
        assert!(steady(&symbolic) == symbolic, "a symbolic icon stays recolorable");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
