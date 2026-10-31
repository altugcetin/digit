use crate::draw::{self, Mark, Rgba, Tool, TOOLS};
use crate::out;
use crate::windows::Rect;
use gtk::{cairo, gdk, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Clone)]
struct State {
    marks: Vec<Mark>,
    crop: Option<Rect>,
    frame: usize,
}

struct Ed {
    app: gtk::Application,
    window: gtk::ApplicationWindow,
    area: gtk::DrawingArea,
    base: draw::Base,
    marks: RefCell<Vec<Mark>>,
    undo: RefCell<Vec<State>>,
    redo: RefCell<Vec<State>>,
    live: RefCell<Option<Mark>>,
    tool: Cell<Tool>,
    color: Cell<Rgba>,
    width: Cell<f64>,
    fill: Cell<bool>,
    crop: Cell<Option<Rect>>,
    frame: Cell<usize>,
    pad: Cell<f64>,
    picked: Cell<Option<usize>>,
    step: Cell<u32>,
    sticker: RefCell<String>,
    view: Cell<(f64, f64, f64)>,
    start: Cell<(f64, f64)>,
    last: Cell<(f64, f64)>,
    tools: RefCell<Vec<(Tool, gtk::ToggleButton)>>,
    swatches: RefCell<Vec<(Rgba, gtk::Button)>>,
    widths: RefCell<Vec<(f64, gtk::ToggleButton)>>,
    fill_btn: RefCell<Option<gtk::ToggleButton>>,
    toast: gtk::Label,
    dirty: Cell<bool>,
    output: String,
    origin: Rect,
    prefs: out::Prefs,
}

const STICKERS: [&str; 30] = ["👍", "👎", "✅", "❌", "⚠️", "❓", "❗", "💡", "🔥", "⭐", "❤️", "😀", "😂", "😍", "🤔", "😎", "🙏", "👀", "🎉", "📌", "🐛", "🚀", "✨", "💯", "👉", "👈", "👆", "👇", "🔒", "⏱️"];

fn rgba(hex: &str) -> Rgba {
    draw::hex(hex)
}

pub fn open(app: &gtk::Application, mut surface: cairo::ImageSurface, output: String, origin: Rect) {
    surface.flush();
    let (w, h, stride) = (surface.width(), surface.height(), surface.stride());
    let data = surface.data().map(|d| d.to_vec()).unwrap_or_default();
    let Some(base) = draw::Base::new(data, w, h, stride) else { return };
    open_base(app, base, output, origin);
}

pub fn open_file(app: &gtk::Application, path: &std::path::Path) -> bool {
    let Ok(pb) = gtk::gdk_pixbuf::Pixbuf::from_file(path) else { return false };
    let pb = pb.apply_embedded_orientation().unwrap_or(pb);
    let Some(base) = pixbuf_base(&pb) else { return false };
    open_base(app, base, String::new(), (40.0, 40.0, pb.width() as f64, pb.height() as f64));
    true
}

fn pixbuf_base(pb: &gtk::gdk_pixbuf::Pixbuf) -> Option<draw::Base> {
    let (w, h) = (pb.width(), pb.height());
    let n = pb.n_channels() as usize;
    let rs = pb.rowstride() as usize;
    let bytes = pb.read_pixel_bytes();
    let stride = w * 4;
    let mut data = vec![0u8; (stride * h) as usize];
    for y in 0..h as usize {
        for x in 0..w as usize {
            let s = y * rs + x * n;
            let a = if n == 4 { bytes[s + 3] as u32 } else { 255 };
            let d = y * stride as usize + x * 4;
            data[d] = (bytes[s + 2] as u32 * a / 255) as u8;
            data[d + 1] = (bytes[s + 1] as u32 * a / 255) as u8;
            data[d + 2] = (bytes[s] as u32 * a / 255) as u8;
            data[d + 3] = a as u8;
        }
    }
    draw::Base::new(data, w, h, stride)
}

fn open_base(app: &gtk::Application, base: draw::Base, output: String, origin: Rect) {
    let prefs = out::prefs();
    let window = gtk::ApplicationWindow::builder().application(app).title("Screenshot").build();
    window.add_css_class("digit-dialog");
    window.add_css_class("snap-editor");
    // A full-screen overlay rather than a window; Esc, Ctrl+W or a click beside it closes it.
    window.init_layer_shell();
    window.set_namespace(Some("digit-snap-editor"));
    window.set_layer(Layer::Overlay);
    window.set_keyboard_mode(KeyboardMode::Exclusive);
    for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(edge, true);
    }
    window.set_exclusive_zone(-1);
    let area = gtk::DrawingArea::new();
    area.set_hexpand(true);
    area.set_vexpand(true);
    area.set_focusable(true);
    let toast = kit::ui::text("", "toast");
    toast.set_halign(gtk::Align::Center);
    toast.set_valign(gtk::Align::End);
    toast.set_margin_bottom(18);
    toast.set_visible(false);
    toast.set_can_target(false);
    let color = prefs.palette.first().map(|c| rgba(c)).unwrap_or([1.0, 0.23, 0.19, 1.0]);
    let ed = Rc::new(Ed {
        app: app.clone(),
        window: window.clone(),
        area: area.clone(),
        base,
        marks: RefCell::new(Vec::new()),
        undo: RefCell::new(Vec::new()),
        redo: RefCell::new(Vec::new()),
        live: RefCell::new(None),
        tool: Cell::new(Tool::Arrow),
        color: Cell::new(color),
        width: Cell::new(kit::prefs::number(&kit::prefs::read(), "snap_width", 4.0)),
        fill: Cell::new(false),
        crop: Cell::new(None),
        frame: Cell::new(prefs.frame),
        pad: Cell::new(kit::prefs::number(&kit::prefs::read(), "snap_pad", 56.0)),
        picked: Cell::new(None),
        step: Cell::new(1),
        sticker: RefCell::new("👍".into()),
        view: Cell::new((1.0, 0.0, 0.0)),
        start: Cell::new((0.0, 0.0)),
        last: Cell::new((0.0, 0.0)),
        tools: RefCell::new(Vec::new()),
        swatches: RefCell::new(Vec::new()),
        widths: RefCell::new(Vec::new()),
        fill_btn: RefCell::new(None),
        toast: toast.clone(),
        dirty: Cell::new(false),
        output,
        origin,
        prefs,
    });
    let top = toolbar(&ed);
    let bottom = footer(&ed);
    let stage = gtk::Overlay::new();
    stage.add_css_class("stage");
    stage.set_child(Some(&area));
    stage.add_overlay(&toast);
    unfocus(&top);
    unfocus(&bottom);
    let col = gtk::Box::new(gtk::Orientation::Vertical, 0);
    col.append(&top);
    col.append(&stage);
    col.append(&bottom);
    let (layer, backdrop) = kit::ui::overlay_card(&col);
    window.set_child(Some(&layer));
    kit::ui::overlay_handling(window.upcast_ref(), &backdrop, &col);
    let weak = Rc::downgrade(&ed);
    area.set_draw_func(move |_, cr, w, h| {
        if let Some(e) = weak.upgrade() {
            e.paint(cr, w as f64, h as f64);
        }
    });
    wire(&ed);
    let mon = gdk::Display::default().and_then(|d| d.monitors().item(0)).and_downcast::<gdk::Monitor>().map(|m| m.geometry());
    let (mw, mh) = mon.map(|g| (g.width() as f64, g.height() as f64)).unwrap_or((1600.0, 900.0));
    let k = ed.base.w as f64 / origin.2.max(1.0);
    let (iw, ih) = (ed.base.w as f64 / k.max(1.0), ed.base.h as f64 / k.max(1.0));
    col.set_size_request((iw + 96.0).clamp(1040.0, mw * 0.86) as i32, (ih + 200.0).clamp(600.0, mh * 0.86) as i32);
    ed.select_tool(Tool::Arrow);
    ed.sync_style();
    backdrop.set_opacity(0.0);
    window.present();
    kit::ui::settle(&backdrop, &col, 0, true, || {});
    area.grab_focus();
    let weak = Rc::downgrade(&ed);
    let closing = Rc::new(Cell::new(false));
    window.connect_close_request(move |w| {
        let Some(e) = weak.upgrade() else { return glib::Propagation::Proceed };
        if closing.get() {
            return glib::Propagation::Proceed;
        }
        if e.dirty.get() {
            let w = e.window.clone();
            let e2 = e.clone();
            kit::ui::ask(Some(w.upcast_ref()), "Discard your markup?", "The screenshot and its markup haven't been copied or saved.", "Keep Editing", "Discard", "danger", move || {
                e2.dirty.set(false);
                e2.window.close();
            });
            return glib::Propagation::Stop;
        }
        closing.set(true);
        let w = w.clone();
        kit::ui::settle(&backdrop, &col, 0, false, move || w.close());
        glib::Propagation::Stop
    });
    let keep = ed.clone();
    window.connect_destroy(move |_| {
        keep.marks.borrow_mut().clear();
    });
}

fn unfocus(w: &impl IsA<gtk::Widget>) {
    let mut c = w.as_ref().first_child();
    while let Some(x) = c {
        if x.is::<gtk::Button>() || x.is::<gtk::MenuButton>() {
            x.set_focus_on_click(false);
            x.set_focusable(false);
        }
        unfocus(&x);
        c = x.next_sibling();
    }
}

fn icon_toggle(icon: &str, tip: &str) -> gtk::ToggleButton {
    let b = gtk::ToggleButton::new();
    b.set_child(Some(&kit::ui::sym(icon)));
    b.add_css_class("toolbtn");
    b.set_tooltip_text(Some(tip));
    b.set_valign(gtk::Align::Center);
    b
}

fn sep() -> gtk::Separator {
    let s = gtk::Separator::new(gtk::Orientation::Vertical);
    s.add_css_class("bar-sep");
    s
}

fn toolbar(ed: &Rc<Ed>) -> gtk::Widget {
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    bar.add_css_class("editbar");
    for (tool, icon, tip, key) in TOOLS {
        let b = icon_toggle(icon, &format!("{tip} ({})", key.to_ascii_uppercase()));
        let weak = Rc::downgrade(ed);
        b.connect_clicked(move |_| {
            if let Some(e) = weak.upgrade() {
                e.select_tool(tool);
            }
        });
        if tool == Tool::Sticker {
            let pop = sticker_popover(ed);
            pop.set_parent(&b);
            let p = pop.clone();
            let weak = Rc::downgrade(ed);
            let r = gtk::GestureClick::new();
            r.set_button(3);
            r.connect_pressed(move |_, _, _, _| p.popup());
            b.add_controller(r);
            let p = pop.clone();
            b.connect_clicked(move |b| {
                if b.is_active() {
                    p.popup();
                }
                let _ = weak.upgrade();
            });
        }
        bar.append(&b);
        ed.tools.borrow_mut().push((tool, b));
        if matches!(tool, Tool::Select | Tool::Marker | Tool::Sticker) {
            bar.append(&sep());
        }
    }
    bar.append(&sep());
    for (icon, tip, f) in [("undo", "Undo (Ctrl+Z)", 0), ("redo", "Redo (Ctrl+Shift+Z)", 1)] {
        let b = kit::app::control(icon, "");
        b.set_tooltip_text(Some(tip));
        let weak = Rc::downgrade(ed);
        b.connect_clicked(move |_| {
            if let Some(e) = weak.upgrade() {
                if f == 0 { e.undo() } else { e.redo() }
            }
        });
        bar.append(&b);
    }
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    bar.append(&spacer);
    let close = kit::app::control("close", "close");
    close.set_tooltip_text(Some("Close (Esc)"));
    let w = ed.window.clone();
    close.connect_clicked(move |_| w.close());
    bar.append(&close);
    bar.upcast()
}

fn sticker_popover(ed: &Rc<Ed>) -> gtk::Popover {
    let pop = gtk::Popover::new();
    kit::ui::menu_popover(&pop);
    let grid = gtk::FlowBox::new();
    grid.set_max_children_per_line(6);
    grid.set_selection_mode(gtk::SelectionMode::None);
    for s in STICKERS {
        let b = gtk::Button::with_label(s);
        b.add_css_class("emoji");
        let weak = Rc::downgrade(ed);
        let p = pop.clone();
        b.connect_clicked(move |_| {
            if let Some(e) = weak.upgrade() {
                *e.sticker.borrow_mut() = s.to_string();
                e.select_tool(Tool::Sticker);
            }
            p.popdown();
        });
        grid.insert(&b, -1);
    }
    let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
    col.append(&kit::ui::text("Click the picture to place it", "sub"));
    col.append(&grid);
    pop.set_child(Some(&col));
    pop
}

fn footer(ed: &Rc<Ed>) -> gtk::Widget {
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    bar.add_css_class("editfoot");
    let style = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    for (i, c) in ed.prefs.palette.iter().enumerate().take(8) {
        let col = rgba(c);
        let b = gtk::Button::new();
        b.add_css_class("swatch");
        b.set_tooltip_text(Some(&format!("{c} ({})", i + 1)));
        let dot = gtk::DrawingArea::new();
        dot.set_content_width(18);
        dot.set_content_height(18);
        dot.set_draw_func(move |_, cr, w, h| {
            cr.arc(w as f64 / 2.0, h as f64 / 2.0, 8.0, 0.0, std::f64::consts::TAU);
            cr.set_source_rgba(col[0], col[1], col[2], 1.0);
            let _ = cr.fill_preserve();
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.25);
            cr.set_line_width(1.0);
            let _ = cr.stroke();
        });
        b.set_child(Some(&dot));
        let weak = Rc::downgrade(ed);
        b.connect_clicked(move |_| {
            if let Some(e) = weak.upgrade() {
                e.set_color(col);
            }
        });
        style.append(&b);
        ed.swatches.borrow_mut().push((col, b));
    }
    // A popover, not the color dialog: a separate window would open underneath the overlay.
    let custom = gtk::MenuButton::new();
    custom.add_css_class("swatch");
    custom.set_tooltip_text(Some("Any color"));
    custom.set_child(Some(&kit::ui::sym("palette")));
    let pop = gtk::Popover::new();
    kit::ui::menu_popover(&pop);
    #[allow(deprecated)]
    {
        let chooser = gtk::ColorChooserWidget::new();
        chooser.set_use_alpha(false);
        let weak = Rc::downgrade(ed);
        chooser.connect_rgba_notify(move |c| {
            if let Some(e) = weak.upgrade() {
                let c = c.rgba();
                e.set_color([c.red() as f64, c.green() as f64, c.blue() as f64, 1.0]);
            }
        });
        pop.set_child(Some(&chooser));
    }
    custom.set_popover(Some(&pop));
    style.append(&custom);
    style.append(&sep());
    for (wv, label) in [(2.0, "Thin"), (4.0, "Medium"), (8.0, "Thick")] {
        let b = gtk::ToggleButton::new();
        b.add_css_class("toolbtn");
        b.set_tooltip_text(Some(&format!("{label} ([ and ])")));
        let dot = gtk::DrawingArea::new();
        dot.set_content_width(20);
        dot.set_content_height(20);
        dot.set_draw_func(move |_, cr, w, h| {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.85);
            cr.set_line_width(wv * 0.9 + 0.6);
            cr.set_line_cap(cairo::LineCap::Round);
            cr.move_to(4.0, h as f64 / 2.0);
            cr.line_to(w as f64 - 4.0, h as f64 / 2.0);
            let _ = cr.stroke();
        });
        b.set_child(Some(&dot));
        let weak = Rc::downgrade(ed);
        b.connect_clicked(move |_| {
            if let Some(e) = weak.upgrade() {
                e.set_width(wv);
            }
        });
        style.append(&b);
        ed.widths.borrow_mut().push((wv, b));
    }
    let fill = icon_toggle("format_color_fill", "Filled shapes and text background (F)");
    let weak = Rc::downgrade(ed);
    fill.connect_toggled(move |b| {
        if let Some(e) = weak.upgrade() {
            e.fill.set(b.is_active());
            e.restyle_picked();
        }
    });
    style.append(&fill);
    *ed.fill_btn.borrow_mut() = Some(fill);
    bar.append(&style);
    bar.append(&sep());
    let frame = gtk::MenuButton::new();
    frame.set_label("Background");
    frame.add_css_class("pillmenu");
    let pop = gtk::Popover::new();
    kit::ui::menu_popover(&pop);
    let col = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let flow = gtk::FlowBox::new();
    flow.set_max_children_per_line(4);
    flow.set_selection_mode(gtk::SelectionMode::None);
    for (i, (name, a, b)) in draw::FRAMES.iter().enumerate() {
        let btn = gtk::Button::new();
        btn.add_css_class("framebtn");
        let (ca, cb) = if a.is_empty() { ([0.1, 0.1, 0.1, 1.0], [0.1, 0.1, 0.1, 1.0]) } else { (rgba(a), rgba(b)) };
        let sw = gtk::DrawingArea::new();
        sw.set_content_width(64);
        sw.set_content_height(40);
        let none = a.is_empty();
        sw.set_draw_func(move |_, cr, w, h| {
            draw::rounded(cr, 0.0, 0.0, w as f64, h as f64, 8.0);
            let g = cairo::LinearGradient::new(0.0, 0.0, w as f64, h as f64);
            g.add_color_stop_rgb(0.0, ca[0], ca[1], ca[2]);
            g.add_color_stop_rgb(1.0, cb[0], cb[1], cb[2]);
            let _ = cr.set_source(&g);
            let _ = cr.fill();
            if none {
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.4);
                cr.set_line_width(1.5);
                cr.move_to(10.0, h as f64 - 10.0);
                cr.line_to(w as f64 - 10.0, 10.0);
                let _ = cr.stroke();
            }
        });
        let v = gtk::Box::new(gtk::Orientation::Vertical, 4);
        v.append(&sw);
        v.append(&kit::ui::text(name, "sub"));
        btn.set_child(Some(&v));
        let weak = Rc::downgrade(ed);
        btn.connect_clicked(move |_| {
            if let Some(e) = weak.upgrade() {
                e.snapshot();
                e.frame.set(i);
                let _ = kit::prefs::set("snap_frame", toml::Value::String(name.to_string()));
                e.changed();
            }
        });
        flow.insert(&btn, -1);
    }
    col.append(&flow);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.append(&kit::ui::text("Padding", "sub"));
    let pad = gtk::Scale::with_range(gtk::Orientation::Horizontal, 16.0, 160.0, 4.0);
    pad.set_value(ed.pad.get());
    pad.set_hexpand(true);
    let weak = Rc::downgrade(ed);
    pad.connect_value_changed(move |s| {
        if let Some(e) = weak.upgrade() {
            e.pad.set(s.value());
            e.area.queue_draw();
        }
    });
    pad.connect_unrealize(|s| {
        let _ = kit::prefs::set("snap_pad", toml::Value::Float(s.value().round()));
    });
    row.append(&pad);
    col.append(&row);
    pop.set_child(Some(&col));
    let weak = Rc::downgrade(ed);
    pop.connect_closed(move |_| {
        if let Some(e) = weak.upgrade() {
            e.area.grab_focus();
        }
    });
    frame.set_popover(Some(&pop));
    bar.append(&frame);
    let size = kit::ui::text(&format!("{} × {}", ed.base.w, ed.base.h), "sub mono");
    size.set_margin_start(6);
    bar.append(&size);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    bar.append(&spacer);
    let mut actions: Vec<(&str, &str, &str, u8)> = vec![("content_copy", "Copy", "Copy (Ctrl+C)", 0), ("download", "Save", "Save (Ctrl+S)", 1), ("save_as", "Save As…", "Save as (Ctrl+Shift+S)", 2), ("push_pin", "Pin", "Pin to screen", 3)];
    if out::ocr_available() {
        actions.push(("text_fields", "Copy Text", "Copy the text in the image", 4));
    }
    for (icon, _label, tip, id) in actions {
        let b = kit::app::control(icon, "");
        b.add_css_class("act");
        b.set_tooltip_text(Some(tip));
        let weak = Rc::downgrade(ed);
        b.connect_clicked(move |_| {
            if let Some(e) = weak.upgrade() {
                e.action(id);
            }
        });
        bar.append(&b);
    }
    let done = kit::ui::button("Done", "accent");
    done.set_tooltip_text(Some("Copy, save and close (Enter)"));
    let weak = Rc::downgrade(ed);
    done.connect_clicked(move |_| {
        if let Some(e) = weak.upgrade() {
            e.action(9);
        }
    });
    bar.append(&done);
    bar.upcast()
}

fn wire(ed: &Rc<Ed>) {
    let drag = gtk::GestureDrag::new();
    let weak = Rc::downgrade(ed);
    drag.connect_drag_begin(move |_, x, y| {
        if let Some(e) = weak.upgrade() {
            e.begin(x, y);
        }
    });
    let weak = Rc::downgrade(ed);
    drag.connect_drag_update(move |g, dx, dy| {
        if let (Some(e), Some((sx, sy))) = (weak.upgrade(), g.start_point()) {
            let shift = g.current_event_state().contains(gdk::ModifierType::SHIFT_MASK);
            e.update(sx + dx, sy + dy, shift);
        }
    });
    let weak = Rc::downgrade(ed);
    drag.connect_drag_end(move |g, dx, dy| {
        if let (Some(e), Some((sx, sy))) = (weak.upgrade(), g.start_point()) {
            e.end(sx + dx, sy + dy, dx.abs() < 3.0 && dy.abs() < 3.0);
        }
    });
    ed.area.add_controller(drag);
    let click = gtk::GestureClick::new();
    let weak = Rc::downgrade(ed);
    click.connect_pressed(move |_, n, x, y| {
        let Some(e) = weak.upgrade() else { return };
        if n == 2 {
            let p = e.to_img(x, y);
            let hit = e.hit(p);
            if let Some(i) = hit {
                let m = e.marks.borrow()[i].clone();
                if m.tool == Tool::Text {
                    e.text_entry(x, y, p, Some(i), &m.text);
                }
            }
        }
    });
    ed.area.add_controller(click);
    let motion = gtk::EventControllerMotion::new();
    let weak = Rc::downgrade(ed);
    motion.connect_motion(move |_, x, y| {
        if let Some(e) = weak.upgrade() {
            let p = e.to_img(x, y);
            let cursor = match e.tool.get() {
                Tool::Select => if e.hit(p).is_some() { "move" } else { "default" },
                Tool::Text => "text",
                _ => "crosshair",
            };
            e.area.set_cursor_from_name(Some(cursor));
        }
    });
    ed.area.add_controller(motion);
    let keys = gtk::EventControllerKey::new();
    let weak = Rc::downgrade(ed);
    keys.connect_key_pressed(move |_, k, _, m| {
        let Some(e) = weak.upgrade() else { return glib::Propagation::Proceed };
        if e.key(k, m) { glib::Propagation::Stop } else { glib::Propagation::Proceed }
    });
    ed.window.add_controller(keys);
}

impl Ed {
    fn region(&self) -> Rect {
        let full = (0.0, 0.0, self.base.w as f64, self.base.h as f64);
        if self.tool.get() == Tool::Crop { full } else { self.crop.get().unwrap_or(full) }
    }

    fn pad_now(&self) -> f64 {
        if self.frame.get() > 0 && self.tool.get() != Tool::Crop { self.pad.get() } else { 0.0 }
    }

    fn compose(&self, cr: &cairo::Context, live: bool) {
        let (cx, cy, cw, ch) = self.region();
        let pad = self.pad_now();
        if pad > 0.0 {
            draw::frame(cr, self.frame.get(), cw, ch, pad);
            cr.translate(pad, pad);
            draw::rounded(cr, 0.0, 0.0, cw, ch, 12.0);
            cr.clip();
        } else {
            cr.rectangle(0.0, 0.0, cw, ch);
            cr.clip();
        }
        cr.translate(-cx, -cy);
        let live_mark = if live { self.live.borrow().clone() } else { None };
        draw::scene(cr, &self.base, &self.marks.borrow(), live_mark.as_ref());
    }

    fn paint(&self, cr: &cairo::Context, aw: f64, ah: f64) {
        let (cx, cy, cw, ch) = self.region();
        let pad = self.pad_now();
        let (tw, th) = (cw + pad * 2.0, ch + pad * 2.0);
        let scale = ((aw - 48.0) / tw).min((ah - 48.0) / th).min(1.0).max(0.05);
        let (ox, oy) = ((aw - tw * scale) / 2.0, (ah - th * scale) / 2.0);
        self.view.set((scale, ox + (pad - cx) * scale, oy + (pad - cy) * scale));
        cr.save().ok();
        cr.translate(ox, oy);
        cr.scale(scale, scale);
        if pad == 0.0 {
            cr.rectangle(0.0, 0.0, cw, ch);
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
            let _ = cr.fill();
        }
        self.compose(cr, true);
        cr.restore().ok();
        let (s, tx, ty) = self.view.get();
        if self.tool.get() == Tool::Crop {
            if let Some(r) = self.crop.get() {
                cr.set_fill_rule(cairo::FillRule::EvenOdd);
                cr.rectangle(tx, ty, self.base.w as f64 * s, self.base.h as f64 * s);
                cr.rectangle(tx + r.0 * s, ty + r.1 * s, r.2 * s, r.3 * s);
                cr.set_source_rgba(0.0, 0.0, 0.0, 0.55);
                let _ = cr.fill();
                cr.set_fill_rule(cairo::FillRule::Winding);
                cr.rectangle(tx + r.0 * s + 0.5, ty + r.1 * s + 0.5, r.2 * s - 1.0, r.3 * s - 1.0);
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.95);
                cr.set_line_width(1.5);
                let _ = cr.stroke();
            }
        }
        if let Some(i) = self.picked.get() {
            if let Some(m) = self.marks.borrow().get(i) {
                let (x, y, w, h) = m.bounds(Some(cr));
                cr.rectangle(tx + x * s, ty + y * s, w * s, h * s);
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.9);
                cr.set_line_width(1.0);
                cr.set_dash(&[5.0, 4.0], 0.0);
                let _ = cr.stroke();
                cr.set_dash(&[], 0.0);
            }
        }
    }

    fn to_img(&self, x: f64, y: f64) -> (f64, f64) {
        let (s, tx, ty) = self.view.get();
        ((x - tx) / s, (y - ty) / s)
    }

    fn hit(&self, p: (f64, f64)) -> Option<usize> {
        let marks = self.marks.borrow();
        let surf = cairo::ImageSurface::create(cairo::Format::ARgb32, 1, 1).ok();
        let cr = surf.as_ref().and_then(|s| cairo::Context::new(s).ok());
        (0..marks.len()).rev().find(|&i| {
            let (x, y, w, h) = marks[i].bounds(cr.as_ref());
            p.0 >= x && p.0 <= x + w && p.1 >= y && p.1 <= y + h
        })
    }

    fn state(&self) -> State {
        State { marks: self.marks.borrow().clone(), crop: self.crop.get(), frame: self.frame.get() }
    }

    fn snapshot(&self) {
        let s = self.state();
        let mut u = self.undo.borrow_mut();
        u.push(s);
        if u.len() > 100 {
            u.remove(0);
        }
        self.redo.borrow_mut().clear();
    }

    fn restore(&self, s: State) {
        *self.marks.borrow_mut() = s.marks;
        self.crop.set(s.crop);
        self.frame.set(s.frame);
        self.picked.set(None);
        self.changed();
    }

    fn undo(&self) {
        let prev = self.undo.borrow_mut().pop();
        if let Some(s) = prev {
            self.redo.borrow_mut().push(self.state());
            self.restore(s);
        }
    }

    fn redo(&self) {
        let next = self.redo.borrow_mut().pop();
        if let Some(s) = next {
            self.undo.borrow_mut().push(self.state());
            self.restore(s);
        }
    }

    fn changed(&self) {
        self.dirty.set(true);
        self.area.queue_draw();
    }

    fn select_tool(&self, t: Tool) {
        if self.tool.get() == Tool::Crop && t != Tool::Crop {
            self.tool.set(t);
        }
        self.tool.set(t);
        if t != Tool::Select {
            self.picked.set(None);
        }
        for (tool, b) in self.tools.borrow().iter() {
            if b.is_active() != (*tool == t) {
                b.set_active(*tool == t);
            }
        }
        self.area.queue_draw();
    }

    fn set_color(&self, c: Rgba) {
        self.color.set(c);
        self.sync_style();
        self.restyle_picked();
        if !matches!(self.tool.get(), Tool::Select | Tool::Pen | Tool::Marker | Tool::Line | Tool::Arrow | Tool::Rect | Tool::Ellipse | Tool::Text | Tool::Step) {
            self.select_tool(Tool::Arrow);
        }
    }

    fn set_width(&self, w: f64) {
        self.width.set(w);
        let _ = kit::prefs::set("snap_width", toml::Value::Float(w));
        self.sync_style();
        self.restyle_picked();
    }

    fn sync_style(&self) {
        let c = self.color.get();
        for (col, b) in self.swatches.borrow().iter() {
            if *col == c {
                b.add_css_class("on");
            } else {
                b.remove_css_class("on");
            }
        }
        for (w, b) in self.widths.borrow().iter() {
            if b.is_active() != (*w == self.width.get()) {
                b.set_active(*w == self.width.get());
            }
        }
    }

    fn restyle_picked(&self) {
        let Some(i) = self.picked.get() else { return };
        self.snapshot();
        if let Some(m) = self.marks.borrow_mut().get_mut(i) {
            if !matches!(m.tool, Tool::Pixelate | Tool::Blur | Tool::Spotlight | Tool::Sticker) {
                m.color = self.color.get();
            }
            m.width = self.width.get();
            m.fill = self.fill.get();
        }
        self.changed();
    }

    fn begin(&self, x: f64, y: f64) {
        let p = self.to_img(x, y);
        self.start.set(p);
        self.last.set(p);
        let tool = self.tool.get();
        match tool {
            Tool::Select => {
                self.picked.set(self.hit(p));
                if self.picked.get().is_some() {
                    self.snapshot();
                }
            }
            Tool::Crop => {
                self.snapshot();
                self.crop.set(Some((p.0, p.1, 0.0, 0.0)));
            }
            Tool::Text => {}
            Tool::Step => {
                self.snapshot();
                let mut m = Mark::new(Tool::Step, p, self.color.get(), self.width.get(), true);
                m.n = self.step.get();
                self.step.set(m.n + 1);
                self.marks.borrow_mut().push(m);
                self.changed();
            }
            Tool::Sticker => {
                self.snapshot();
                let mut m = Mark::new(Tool::Sticker, p, self.color.get(), self.width.get(), false);
                m.text = self.sticker.borrow().clone();
                let size = m.size() * 2.4;
                m.pts = vec![(p.0 - size * 0.6, p.1 - size * 0.7)];
                self.marks.borrow_mut().push(m);
                self.changed();
            }
            _ => {
                let mut m = Mark::new(tool, p, self.color.get(), self.width.get(), self.fill.get());
                if tool == Tool::Pen || tool == Tool::Marker {
                    m.pts = vec![p];
                }
                *self.live.borrow_mut() = Some(m);
            }
        }
    }

    fn update(&self, x: f64, y: f64, shift: bool) {
        let mut p = self.to_img(x, y);
        let s = self.start.get();
        let tool = self.tool.get();
        match tool {
            Tool::Select => {
                let l = self.last.get();
                if let Some(i) = self.picked.get() {
                    if let Some(m) = self.marks.borrow_mut().get_mut(i) {
                        m.shift(p.0 - l.0, p.1 - l.1);
                    }
                    self.last.set(p);
                    self.changed();
                }
            }
            Tool::Crop => {
                let (w, h) = (self.base.w as f64, self.base.h as f64);
                p = (p.0.clamp(0.0, w), p.1.clamp(0.0, h));
                let r = (s.0.min(p.0), s.1.min(p.1), (s.0 - p.0).abs(), (s.1 - p.1).abs());
                self.crop.set(Some(r));
                self.area.queue_draw();
            }
            _ => {
                let mut live = self.live.borrow_mut();
                let Some(m) = live.as_mut() else { return };
                match tool {
                    Tool::Pen | Tool::Marker => {
                        if shift {
                            m.pts.truncate(1);
                            m.pts.push(p);
                        } else if m.pts.last().is_none_or(|l| (l.0 - p.0).hypot(l.1 - p.1) > 1.2) {
                            m.pts.push(p);
                        }
                    }
                    Tool::Line | Tool::Arrow => {
                        if shift {
                            let (dx, dy) = (p.0 - s.0, p.1 - s.1);
                            let len = dx.hypot(dy);
                            let ang = (dy.atan2(dx) / (std::f64::consts::PI / 12.0)).round() * (std::f64::consts::PI / 12.0);
                            p = (s.0 + len * ang.cos(), s.1 + len * ang.sin());
                        }
                        m.pts = vec![s, p];
                    }
                    _ => {
                        if shift {
                            let d = (p.0 - s.0).abs().max((p.1 - s.1).abs());
                            p = (s.0 + d * (p.0 - s.0).signum(), s.1 + d * (p.1 - s.1).signum());
                        }
                        m.pts = vec![s, p];
                    }
                }
                drop(live);
                self.area.queue_draw();
            }
        }
    }

    fn end(self: &Rc<Self>, x: f64, y: f64, click: bool) {
        let p = self.to_img(x, y);
        match self.tool.get() {
            Tool::Text if click => self.text_entry(x, y, p, None, ""),
            Tool::Crop => {
                if self.crop.get().is_some_and(|r| r.2 < 8.0 || r.3 < 8.0) {
                    self.crop.set(None);
                }
                self.changed();
            }
            Tool::Select => {
                if click {
                    self.picked.set(self.hit(p));
                    if let Some(i) = self.picked.get() {
                        let (c, w, f) = self.marks.borrow().get(i).map(|m| (m.color, m.width, m.fill)).unwrap_or((self.color.get(), self.width.get(), false));
                        self.color.set(c);
                        self.width.set(w);
                        self.fill.set(f);
                        self.sync_style();
                    }
                }
                self.area.queue_draw();
            }
            _ => {
                let live = self.live.borrow_mut().take();
                if let Some(m) = live {
                    if !m.trivial() {
                        self.snapshot();
                        self.marks.borrow_mut().push(m);
                    }
                }
                self.changed();
            }
        }
    }

    fn text_entry(self: &Rc<Self>, x: f64, y: f64, p: (f64, f64), edit: Option<usize>, initial: &str) {
        let pop = gtk::Popover::new();
        pop.set_parent(&self.area);
        pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        pop.set_position(gtk::PositionType::Bottom);
        let entry = gtk::Entry::new();
        entry.add_css_class("search");
        entry.set_width_chars(28);
        entry.set_text(initial);
        entry.set_placeholder_text(Some("Type, then Enter"));
        pop.set_child(Some(&entry));
        let area = self.area.clone();
        let weak = Rc::downgrade(self);
        let p2 = pop.clone();
        entry.connect_activate(move |e| {
            let text = e.text().to_string();
            if let Some(ed) = weak.upgrade() {
                if !text.trim().is_empty() {
                    ed.snapshot();
                    match edit {
                        Some(i) => {
                            if let Some(m) = ed.marks.borrow_mut().get_mut(i) {
                                m.text = text;
                            }
                        }
                        None => {
                            let mut m = Mark::new(Tool::Text, p, ed.color.get(), ed.width.get(), ed.fill.get());
                            let size = m.size();
                            m.pts = vec![(p.0, p.1 - size * 0.6)];
                            m.text = text;
                            ed.marks.borrow_mut().push(m);
                        }
                    }
                    ed.changed();
                }
            }
            p2.popdown();
        });
        pop.connect_closed(move |p| {
            let p = p.clone();
            let a = area.clone();
            glib::idle_add_local_once(move || {
                p.unparent();
                a.grab_focus();
            });
        });
        pop.popup();
        entry.grab_focus();
    }

    fn key(&self, k: gdk::Key, m: gdk::ModifierType) -> bool {
        let ctrl = m.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = m.contains(gdk::ModifierType::SHIFT_MASK);
        if gtk::prelude::GtkWindowExt::focus(&self.window).is_some_and(|f| f.is::<gtk::Text>()) {
            return false;
        }
        match k {
            gdk::Key::z if ctrl && shift => self.redo(),
            gdk::Key::Z if ctrl => self.redo(),
            gdk::Key::y if ctrl => self.redo(),
            gdk::Key::z if ctrl => self.undo(),
            gdk::Key::c if ctrl => self.action(0),
            gdk::Key::S if ctrl => self.action(2),
            gdk::Key::s if ctrl && shift => self.action(2),
            gdk::Key::s if ctrl => self.action(1),
            gdk::Key::w | gdk::Key::q if ctrl => {
                self.window.close();
            }
            gdk::Key::Return | gdk::Key::KP_Enter => self.action(9),
            gdk::Key::Escape => {
                if self.picked.get().is_some() {
                    self.picked.set(None);
                    self.area.queue_draw();
                } else if self.tool.get() == Tool::Crop {
                    self.select_tool(Tool::Arrow);
                } else {
                    self.window.close();
                }
            }
            gdk::Key::Delete | gdk::Key::BackSpace => {
                if let Some(i) = self.picked.take() {
                    self.snapshot();
                    let mut marks = self.marks.borrow_mut();
                    if i < marks.len() {
                        marks.remove(i);
                    }
                    drop(marks);
                    self.changed();
                }
            }
            gdk::Key::bracketleft => self.set_width((self.width.get() / 2.0).max(1.0)),
            gdk::Key::bracketright => self.set_width((self.width.get() * 2.0).min(16.0)),
            gdk::Key::f if !ctrl => {
                if let Some(b) = self.fill_btn.borrow().as_ref() {
                    b.set_active(!b.is_active());
                }
            }
            _ => {
                if ctrl {
                    return false;
                }
                let Some(c) = k.to_unicode() else { return false };
                if let Some(d) = c.to_digit(10).filter(|d| (1..=8).contains(d)) {
                    let col = self.swatches.borrow().get(d as usize - 1).map(|s| s.0);
                    if let Some(col) = col {
                        self.set_color(col);
                    }
                    return true;
                }
                match TOOLS.iter().find(|t| t.3 == c.to_ascii_lowercase()) {
                    Some(t) => self.select_tool(t.0),
                    None => return false,
                }
            }
        }
        true
    }

    fn export(&self) -> Option<gdk::Texture> {
        let (_, _, cw, ch) = {
            let full = (0.0, 0.0, self.base.w as f64, self.base.h as f64);
            self.crop.get().unwrap_or(full)
        };
        let pad = if self.frame.get() > 0 { self.pad.get() } else { 0.0 };
        let (w, h) = ((cw + pad * 2.0).round() as i32, (ch + pad * 2.0).round() as i32);
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, w.max(1), h.max(1)).ok()?;
        {
            let cr = cairo::Context::new(&surface).ok()?;
            let tool = self.tool.replace(Tool::Select);
            self.compose(&cr, false);
            self.tool.set(tool);
        }
        out::texture(&surface)
    }

    fn flash(&self, msg: &str) {
        self.toast.set_text(msg);
        self.toast.set_visible(true);
        let t = self.toast.clone();
        let m = msg.to_string();
        glib::timeout_add_local_once(std::time::Duration::from_millis(2200), move || {
            if t.text() == m {
                t.set_visible(false);
            }
        });
    }

    fn action(&self, id: u8) {
        let Some(tex) = self.export() else { return };
        match id {
            0 => {
                out::copy(&tex, &self.app);
                self.dirty.set(false);
                self.flash("Copied");
            }
            1 => match out::save(&tex, &self.prefs) {
                Ok(p) => {
                    self.dirty.set(false);
                    self.flash(&format!("Saved to {}", p.display()));
                }
                Err(e) => self.flash(&format!("Couldn't save: {e}")),
            },
            2 => {
                let toast = self.toast.clone();
                let w = self.window.clone();
                // The file chooser is an ordinary window, so step below it until it closes.
                w.set_layer(Layer::Bottom);
                w.set_keyboard_mode(KeyboardMode::None);
                let back = w.clone();
                out::save_as(tex, Some(w.upcast_ref()), move |p| {
                    back.set_layer(Layer::Overlay);
                    back.set_keyboard_mode(KeyboardMode::Exclusive);
                    if let Some(p) = p {
                        toast.set_text(&format!("Saved to {}", p.display()));
                        toast.set_visible(true);
                    }
                });
            }
            3 => {
                out::pin(&self.app, tex, &self.output, self.origin.0 as i32, self.origin.1 as i32, self.base.w, self.base.h);
                self.dirty.set(false);
                self.window.close();
            }
            4 => {
                out::ocr(&tex);
                self.flash("Reading text…");
            }
            _ => {
                out::copy(&tex, &self.app);
                let saved = self.prefs.save_on_copy.then(|| out::save(&tex, &self.prefs).ok()).flatten();
                if self.prefs.notify {
                    out::notify("Screenshot copied", &saved.as_ref().and_then(|p| p.file_name()).map(|n| format!("Also saved as {}", n.to_string_lossy())).unwrap_or_default(), saved.as_deref());
                }
                self.dirty.set(false);
                self.window.close();
            }
        }
    }
}
