use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::glib;

mod imp {
    use gtk::subclass::prelude::*;
    use gtk::{glib, graphene, prelude::*};
    use std::cell::Cell;

    #[derive(Default)]
    pub struct Lift {
        pub scale: Cell<f64>,
        pub dy: Cell<f64>,
        pub origin: Cell<(f64, f64)>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Lift {
        const NAME: &'static str = "DigitLift";
        type Type = super::Lift;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BinLayout>();
        }
    }

    impl ObjectImpl for Lift {
        fn constructed(&self) {
            self.parent_constructed();
            self.scale.set(1.0);
        }

        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for Lift {
        fn snapshot(&self, s: &gtk::Snapshot) {
            let (k, dy) = (self.scale.get(), self.dy.get().round());
            if (k - 1.0).abs() < 1e-4 && dy.abs() < 0.01 {
                self.parent_snapshot(s);
                return;
            }
            let (ox, oy) = self.origin.get();
            s.save();
            s.translate(&graphene::Point::new(ox as f32, (oy + dy) as f32));
            s.scale(k as f32, k as f32);
            s.translate(&graphene::Point::new(-ox as f32, -oy as f32));
            self.parent_snapshot(s);
            s.restore();
        }
    }
}

glib::wrapper! {
    pub struct Lift(ObjectSubclass<imp::Lift>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Lift {
    pub fn new(child: &impl IsA<gtk::Widget>, origin: (f64, f64)) -> Self {
        let lift: Self = glib::Object::new();
        child.set_parent(&lift);
        lift.imp().origin.set(origin);
        lift
    }

    pub fn set(&self, scale: f64, dy: f64) {
        let imp = self.imp();
        if (imp.scale.get() - scale).abs() < 1e-5 && (imp.dy.get() - dy).abs() < 1e-3 {
            return;
        }
        imp.scale.set(scale);
        imp.dy.set(dy);
        self.queue_draw();
    }
}
