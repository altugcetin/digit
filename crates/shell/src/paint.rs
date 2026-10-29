use gtk::glib;
use gtk::subclass::prelude::*;

type Draw = Box<dyn Fn(&gtk::Snapshot)>;

mod imp {
    use gtk::glib;
    use gtk::subclass::prelude::*;
    use std::cell::RefCell;

    #[derive(Default)]
    pub struct Paint {
        pub draw: RefCell<Option<super::Draw>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Paint {
        const NAME: &'static str = "DigitPaint";
        type Type = super::Paint;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Paint {}

    impl WidgetImpl for Paint {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            if let Some(draw) = self.draw.borrow().as_ref() {
                draw(snapshot);
            }
        }
    }
}

glib::wrapper! {
    pub struct Paint(ObjectSubclass<imp::Paint>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Paint {
    pub fn new(w: i32, h: i32) -> Self {
        let paint: Self = glib::Object::new();
        gtk::prelude::WidgetExt::set_size_request(&paint, w, h);
        paint
    }

    pub fn present(&self) {
        self.on_draw(|s| gtk::prelude::SnapshotExt::append_color(s, &gtk::gdk::RGBA::new(0.0, 0.0, 0.0, 0.004), &gtk::graphene::Rect::new(0.0, 0.0, 8192.0, 8192.0)));
    }

    pub fn on_draw(&self, draw: impl Fn(&gtk::Snapshot) + 'static) {
        *self.imp().draw.borrow_mut() = Some(Box::new(draw));
    }
}
