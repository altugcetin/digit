//! A box that draws its one child grown out of a spot, or shifted across: Quick Look as it opens
//! out of a file and closes back into it, and its preview as it comes in from the side.

use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene, gsk, prelude::*};
use std::cell::{Cell, RefCell};

/// A child drawn into a picture: the picture, where it reaches, and the size it was drawn at.
type Still = (gdk::Texture, graphene::Rect, (i32, i32));

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Moved {
        /// The spot it grows out of, in `of`'s coordinates, and how far grown it is, 0 to 1.
        pub(super) from: RefCell<Option<(graphene::Rect, glib::WeakRef<gtk::Widget>)>>,
        pub(super) grown: Cell<f64>,
        /// How far right of its place it's drawn.
        pub(super) dx: Cell<f64>,
        /// The child drawn once into a picture, its bounds and the size it was drawn at: while it
        /// grows, the picture is scaled, which the graphics card does in no time, instead of
        /// the child being drawn again at every size, its text and its shadow with it.
        pub(super) still: RefCell<Option<Still>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Moved {
        const NAME: &'static str = "DigitFilesMoved";
        type Type = super::Moved;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Moved {
        fn constructed(&self) {
            self.parent_constructed();
            self.grown.set(1.0);
        }

        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for Moved {
        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            self.obj().first_child().map_or((0, 0, -1, -1), |c| c.measure(orientation, for_size))
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            if let Some(child) = self.obj().first_child() {
                child.allocate(width, height, baseline, None);
            }
        }

        fn snapshot(&self, s: &gtk::Snapshot) {
            let obj = self.obj();
            let Some(child) = obj.first_child() else { return };
            let (w, h) = (obj.width() as f64, obj.height() as f64);
            let grown = self.grown.get();
            let spot = self.from.borrow().as_ref().and_then(|(r, of)| {
                let of = of.upgrade()?;
                let p = of.compute_point(&*obj, &graphene::Point::new(r.x(), r.y()))?;
                Some((p.x() as f64, p.y() as f64, r.width() as f64, r.height() as f64))
            });
            s.save();
            s.translate(&graphene::Point::new(self.dx.get() as f32, 0.0));
            if let (Some((x, y, rw, rh)), true) = (spot, grown < 1.0 && w > 0.0 && h > 0.0) {
                // Grown from the spot's middle, as large as fits the spot, to its own place.
                let k0 = (rw / w).max(rh / h).clamp(0.02, 1.0);
                let k = k0 + (1.0 - k0) * grown;
                let (cx, cy) = (x + rw / 2.0 + (w / 2.0 - x - rw / 2.0) * grown, y + rh / 2.0 + (h / 2.0 - y - rh / 2.0) * grown);
                s.translate(&graphene::Point::new((cx - w / 2.0 * k) as f32, (cy - h / 2.0 * k) as f32));
                s.scale(k as f32, k as f32);
                if let Some((tex, bounds)) = self.picture(&child) {
                    s.append_scaled_texture(&tex, gsk::ScalingFilter::Trilinear, &bounds);
                    s.restore();
                    return;
                }
            }
            // In its place: drawn as it is, and drawn again into a picture the next time it grows.
            self.still.replace(None);
            obj.snapshot_child(&child, s);
            s.restore();
        }
    }
}

impl imp::Moved {
    /// The child as a picture at full size, drawn once for as long as its size stays.
    fn picture(&self, child: &gtk::Widget) -> Option<(gdk::Texture, graphene::Rect)> {
        let obj = self.obj();
        let size = (obj.width(), obj.height());
        if let Some((tex, bounds, at)) = self.still.borrow().as_ref()
            && *at == size
        {
            return Some((tex.clone(), *bounds));
        }
        let renderer = obj.native()?.renderer()?;
        let scale = obj.scale_factor().max(1) as f32;
        let snap = gtk::Snapshot::new();
        snap.scale(scale, scale);
        obj.snapshot_child(child, &snap);
        let node = snap.to_node()?;
        let b = node.bounds();
        let tex = renderer.render_texture(&node, Some(&b));
        let bounds = graphene::Rect::new(b.x() / scale, b.y() / scale, b.width() / scale, b.height() / scale);
        self.still.replace(Some((tex.clone(), bounds, size)));
        Some((tex, bounds))
    }
}

glib::wrapper! {
    pub struct Moved(ObjectSubclass<imp::Moved>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Moved {
    pub fn new(child: &impl IsA<gtk::Widget>) -> Self {
        let moved: Self = glib::Object::new();
        child.set_parent(&moved);
        moved
    }

    /// Grows it out of `spot`, in `of`'s coordinates; None draws it in its place.
    pub fn grow_from(&self, spot: Option<(graphene::Rect, &gtk::Widget)>) {
        *self.imp().from.borrow_mut() = spot.map(|(r, of)| (r, of.downgrade()));
        self.queue_draw();
    }

    /// How far grown out of its spot it's drawn, 0 to 1.
    pub fn set_grown(&self, grown: f64) {
        self.imp().grown.set(grown.clamp(0.0, 1.0));
        self.queue_draw();
    }

    /// How far right of its place it's drawn.
    pub fn set_dx(&self, dx: f64) {
        self.imp().dx.set(dx.round());
        self.queue_draw();
    }
}
