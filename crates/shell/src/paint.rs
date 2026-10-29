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

pub fn scrim(monitor: Option<&gtk::gdk::Monitor>, on_click: impl Fn() + 'static) -> gtk::Window {
    use gtk::prelude::*;
    use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
    let window = gtk::Window::new();
    window.init_layer_shell();
    window.set_layer(Layer::Top);
    window.set_namespace(Some("digit-scrim"));
    for e in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(e, true);
    }
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(KeyboardMode::None);
    window.set_monitor(monitor);
    window.add_css_class("digit");
    let paint = Paint::new(-1, -1);
    paint.present();
    let click = gtk::GestureClick::new();
    click.set_button(0);
    let on_click = std::rc::Rc::new(on_click);
    click.connect_pressed(move |_, _, _, _| {
        let f = on_click.clone();
        glib::idle_add_local_once(move || f());
    });
    paint.add_controller(click);
    window.set_child(Some(&paint));
    window.present();
    window
}
