//! Drawing the icons, their labels and the desktop's selection.

use super::*;

pub(super) struct ItemFrame {
    pub(super) icon: gdk::Paintable,
    pub(super) label: pango::Layout,
    pub(super) rect: Rect,
    pub(super) selected: f64,
    pub(super) editing: bool,
}

pub(super) fn draw_item(s: &gtk::Snapshot, app: &App, item: &ItemFrame) {
    let ItemFrame {
        icon,
        rect: r,
        selected,
        editing,
        ..
    } = item;
    let selected = *selected;
    let x = r.x.round();
    let y = r.y.round();
    // A soft tile behind the icon when selected; the label's pill is drawn with the labels.
    if *editing {
        draw::paintable(s, icon, x + (CELL_W - ICON) as f64 / 2.0, y, ICON as f64);
        return;
    }
    let plate = 6.0;
    if selected <= 0.0 {
        draw::paintable(s, icon, x + (CELL_W - ICON) as f64 / 2.0, y, ICON as f64);
        return;
    }
    s.push_opacity(selected);
    draw::fill(
        s,
        &draw::round(
            x + (CELL_W - ICON) as f64 / 2.0 - plate,
            y - plate,
            ICON as f64 + 2.0 * plate,
            ICON as f64 + 2.0 * plate,
            12.0 * app.round(),
        ),
        Rgb::new(0.0, 0.0, 0.0).alpha(0.24),
    );
    s.pop();
    draw::paintable(s, icon, x + (CELL_W - ICON) as f64 / 2.0, y, ICON as f64);
}

/// Draws every item's label: white with a small shadow, so any wallpaper stays readable, and
/// on an accent pill when selected.
///
/// Each shadow node is its own offscreen blur on the GPU, so all labels share one: with many
/// icons, a redraw (a selection, a drag, the rubber band) stays one blur instead of one per
/// icon. The group keeps the same labels while one gets selected, whose white copy only fades
/// out under it, so the other labels' shadows don't shift as the group changes.
pub(super) fn draw_labels(s: &gtk::Snapshot, app: &App, pal: &kit::palette::Palette, items: &[ItemFrame]) {
    let place = |s: &gtk::Snapshot, item: &ItemFrame, ink: Rgb| {
        s.save();
        s.translate(&graphene::Point::new(
            (item.rect.x.round() + LABEL_INSET as f64) as f32,
            (item.rect.y.round() + ICON as f64 + 6.0) as f32,
        ));
        s.append_layout(&item.label, &ink.rgba());
        s.restore();
    };
    let shown: Vec<&ItemFrame> = items.iter().filter(|i| !i.editing).collect();
    let shadow = !kit::effects::reduced() && !shown.is_empty();
    if shadow {
        s.push_shadow(&[gtk::gsk::Shadow::new(gdk::RGBA::new(0.0, 0.0, 0.0, 0.55), 0.0, 1.0, 3.0)]);
    }
    for item in &shown {
        // Never fully transparent: GTK drops a clear text node, and the group would change.
        place(s, item, Rgb::new(1.0, 1.0, 1.0).alpha((1.0 - item.selected).max(0.004)));
    }
    if shadow {
        s.pop();
    }
    for item in shown.iter().filter(|i| i.selected > 0.0) {
        let (w, h) = item.label.pixel_size();
        let (w, h) = (w as f64, h as f64);
        let (x, y) = (item.rect.x.round(), item.rect.y.round() + ICON as f64 + 6.0);
        s.push_opacity(item.selected);
        draw::fill(s, &draw::round(x + (CELL_W as f64 - w) / 2.0 - 6.0, y - 1.0, w + 12.0, h + 2.0, 6.0 * app.round()), pal.accent);
        place(s, item, pal.on_accent);
        s.pop();
    }
}

pub(super) fn draw_desktop(d: &Rc<RefCell<Desktop>>, s: &gtk::Snapshot) {
    let paint = {
        let Ok(state) = d.try_borrow() else { return };
        state.paint.clone()
    };
    let now = paint
        .frame_clock()
        .map(|c| c.frame_time())
        .unwrap_or_else(glib::monotonic_time);
    // Paintables can run their own snapshot code. Clone the visible frame first,
    // so even drawing never calls GTK with the desktop model borrowed.
    let (app, items, rubber, page, pages, grid, geometry) = {
        let Ok(state) = d.try_borrow() else { return };
        let Some(app) = state.app.upgrade() else {
            return;
        };
        let items: Vec<_> = state
            .items
            .iter()
            .filter(|i| state.visible(i))
            .map(|i| ItemFrame {
                icon: i.icon.clone(),
                label: i.label.clone(),
                rect: i.rect(now),
                selected: i.selected.value(now),
                editing: state.renaming.as_ref() == Some(&i.data.name),
            })
            .collect();
        (
            app,
            items,
            state.rubber.as_ref().map(|r| Rect::between(r.start, r.end)),
            state.page,
            state.positions.values().map(|p| p.page).max().unwrap_or(0) + 1,
            state.grid,
            state.geometry,
        )
    };
    let pal = app.palette_at(now);
    // A snapshot without nodes submits no buffer. Keep the full desktop mapped
    // even before the scan finishes, on an empty folder, and on empty pages.
    // Match Paint::present's barely visible fill; alpha zero is optimized away.
    s.append_color(
        &Rgb::new(0.0, 0.0, 0.0).alpha(0.004).rgba(),
        &graphene::Rect::new(
            0.0,
            0.0,
            paint.width().max(1) as f32,
            paint.height().max(1) as f32,
        ),
    );
    for item in &items {
        draw_item(s, &app, item);
    }
    draw_labels(s, &app, &pal, &items);
    let incoming = { d.borrow().incoming.clone() };
    if let Some(IncomingDrag {
        preview,
        pointer,
        hotspot: hot,
    }) = incoming
    {
        s.save();
        s.translate(&graphene::Point::new(
            (pointer.0 - hot.0 as f64) as f32,
            (pointer.1 - hot.1 as f64) as f32,
        ));
        preview.snapshot(
            s,
            preview.intrinsic_width() as f64,
            preview.intrinsic_height() as f64,
        );
        s.restore();
    }
    if let Some(r) = rubber {
        draw::fill(
            s,
            &draw::round(r.x, r.y, r.w, r.h, 8.0 * app.round()),
            pal.accent.alpha(0.22),
        );
    }
    if pages > 1 {
        draw::text(
            s,
            &paint,
            &app.cfg,
            &format!("{} / {}", page + 1, pages),
            12.0,
            Face::Number(500),
            pal.ink_main,
            (grid.width - 110) as f64,
            (geometry.1 - 38) as f64,
            80.0,
            pango::Alignment::Center,
        );
    }
}
