use gtk::subclass::prelude::*;
use gtk::{glib, graphene, gsk, prelude::*};
use kit::palette::Rgb;
use std::cell::{Cell, RefCell};

pub const W: i32 = 24;
pub const H: i32 = 16;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Bars {
        pub(super) values: RefCell<Vec<f64>>,
        pub(super) ink: Cell<Option<Rgb>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Bars {
        const NAME: &'static str = "DigitBars";
        type Type = super::Bars;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Bars {}

    impl WidgetImpl for Bars {
        fn snapshot(&self, s: &gtk::Snapshot) {
            let Some(ink) = self.ink.get() else { return };
            let values = self.values.borrow();
            let n = 5usize;
            let mid = H as f64 / 2.0;
            for i in 0..n {
                let v = values.get(i).copied().unwrap_or(0.0).clamp(0.0, 1.0);
                let h = (3.0 + 11.0 * v).round();
                let x = W as f64 - (n - i) as f64 * 5.0 + 2.0;
                let rect = graphene::Rect::new(x as f32, (mid - h / 2.0) as f32, 3.0, h as f32);
                s.push_rounded_clip(&gsk::RoundedRect::from_rect(rect, 1.5));
                s.append_color(&ink.rgba(), &rect);
                s.pop();
            }
        }
    }
}

glib::wrapper! {
    pub struct Bars(ObjectSubclass<imp::Bars>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for Bars {
    fn default() -> Self {
        let b: Self = glib::Object::new();
        b.set_size_request(W, H);
        b.set_halign(gtk::Align::Start);
        b.set_valign(gtk::Align::Start);
        b.set_can_target(false);
        b
    }
}

impl Bars {
    pub fn set(&self, values: &[f64]) {
        let same = {
            let mut v = self.imp().values.borrow_mut();
            let same = v.len() == values.len() && v.iter().zip(values).all(|(a, b)| (a - b).abs() < 0.004);
            if !same {
                v.clear();
                v.extend_from_slice(values);
            }
            same
        };
        if !same && self.is_visible() {
            self.queue_draw();
        }
    }

    pub fn ink(&self, ink: Rgb) {
        if self.imp().ink.get() != Some(ink) {
            self.imp().ink.set(Some(ink));
            self.queue_draw();
        }
    }
}
