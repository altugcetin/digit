//! Dragging icons: on the desktop, between screens and out to other apps, and the rubber band.

use super::*;

pub(super) fn make_preview(
    d: &Rc<RefCell<Desktop>>,
    freeze_selection: bool,
) -> Option<(gdk::Paintable, (i32, i32))> {
    let paint = { d.borrow().paint.clone() };
    let now = paint
        .frame_clock()
        .map(|c| c.frame_time())
        .unwrap_or_else(glib::monotonic_time);
    let (app, frames, pointer, origins) = {
        let mut state = d.borrow_mut();
        if freeze_selection {
            let names = state.drag.as_ref()?.movement.origins.clone();
            for item in &mut state.items {
                if names.contains_key(&item.data.name) {
                    // The cached preview and the restored real item must have
                    // identical selection ink, even if the press tween is mid-flight.
                    item.selected.set(item.selected.value(now));
                }
            }
        }
        let drag = state.drag.as_ref()?;
        let positions = drag.movement.at(drag.pointer.0, drag.pointer.1);
        let frames: Vec<_> = state
            .items
            .iter()
            .filter(|i| drag.movement.origins.contains_key(&i.data.name))
            .map(|i| ItemFrame {
                icon: i.icon.clone(),
                label: i.label.clone(),
                rect: Rect {
                    x: positions[&i.data.name].0,
                    y: positions[&i.data.name].1,
                    w: CELL_W as f64,
                    h: CELL_H as f64,
                },
                selected: i.selected.value(now),
                editing: false,
            })
            .collect();
        let origins = drag.movement.at(drag.pointer.0, drag.pointer.1);
        (state.app.upgrade()?, frames, drag.pointer, origins)
    };
    let (bounds, hot) = preview_bounds(&origins, pointer);
    let s = gtk::Snapshot::new();
    // Compensate the fractional part GDK's integer hotspot cannot represent.
    // Drawing in desktop coordinates first also preserves its text pixel phase.
    s.translate(&graphene::Point::new(
        hot.0 as f32 - pointer.0 as f32,
        hot.1 as f32 - pointer.1 as f32,
    ));
    let pal = app.palette_at(now);
    for item in &frames {
        draw_item(&s, &app, item);
    }
    draw_labels(&s, &app, &pal, &frames);
    s.to_paintable(Some(&graphene::Size::new(bounds.w as f32, bounds.h as f32)))
        .map(|preview| (preview, hot))
}

pub(super) fn arm_drag(d: &Rc<RefCell<Desktop>>, x: f64, y: f64) {
    let monitor = { d.borrow().monitor.clone() };
    let source_geometry = monitor.geometry();
    let mut all_origins = BTreeMap::new();
    let now = glib::monotonic_time();
    for surface in surfaces() {
        let monitor = { surface.borrow().monitor.clone() };
        let geometry = monitor.geometry();
        let surface = surface.borrow();
        for item in surface.items.iter().filter(|i| surface.visible(i)) {
            all_origins.insert(
                item.data.name.clone(),
                (
                    item.x.value(now) + (geometry.x() - source_geometry.x()) as f64,
                    item.y.value(now) + (geometry.y() - source_geometry.y()) as f64,
                ),
            );
        }
    }
    let (movement, anchor, paths, fallback) = {
        let state = d.borrow();
        if state.drag.is_some() || state.closed {
            return;
        }
        let Some(at) = state.hit(x, y) else { return };
        let item = &state.items[at];
        let anchor = item.data.name.clone();
        let selected = drag_selection(&state.selected, &anchor);
        let movement = Drag {
            start: (x, y),
            origins: all_origins
                .into_iter()
                .filter(|(name, _)| selected.contains(name))
                .collect(),
        };
        let paths: Vec<_> = state
            .items
            .iter()
            .filter(|i| movement.origins.contains_key(&i.data.name))
            .map(|i| i.data.path.clone())
            .collect();
        (movement, anchor, paths, item.icon.clone())
    };
    let files: Vec<_> = paths.iter().map(gio::File::for_path).collect();
    let content = gdk::ContentProvider::for_value(&gdk::FileList::from_array(&files).to_value());
    let drag = DesktopDrag {
        movement,
        anchor,
        content,
        preview: fallback,
        visual: None,
        local_preview: kit::wm::which() == kit::wm::Wm::Hyprland,
        over_desktop: false,
        target_output: None,
        hotspot: (0, 0),
        pointer: (x, y),
        end: DragEnd::Active,
        system: None,
    };
    {
        let mut state = d.borrow_mut();
        if state.closed || state.drag.is_some() {
            return;
        }
        {
            let mut shared = state.shared.borrow_mut();
            shared.drag_names = drag.movement.origins.keys().cloned().collect();
        }
        state.drag = Some(drag);
    }
    for surface in surfaces() {
        animate(&surface);
    }
    move_drag(d, x, y);
    // Warm the exact drawing (including theme paintables) before the threshold.
    if let Some((preview, hotspot)) = make_preview(d, false) {
        let previous = {
            let mut state = d.borrow_mut();
            let Some(drag) = state.drag.as_mut() else {
                return;
            };
            drag.hotspot = hotspot;
            std::mem::replace(&mut drag.preview, preview)
        };
        drop(previous);
    }
}

pub(super) fn own_source(target: &gtk::DropTarget) -> Option<Rc<RefCell<Desktop>>> {
    let system = target.current_drop()?.drag()?;
    surfaces().into_iter().find(|d| {
        d.borrow()
            .drag
            .as_ref()
            .and_then(|drag| drag.system.as_ref())
            == Some(&system)
    })
}

// A frozen group preview moves between output-local coordinate spaces. The
// source owns the GDK drag until its terminal event; the target never imports
// its URIs or creates a second drag. Only one surface draws the preview.
pub(super) fn cross_motion(target: &Rc<RefCell<Desktop>>, source: &Rc<RefCell<Desktop>>, x: f64, y: f64) {
    let output = { target.borrow().output.clone() };
    let changed = {
        let mut state = source.borrow_mut();
        let Some(drag) = state.drag.as_mut() else {
            return;
        };
        let changed = drag.target_output.as_ref() != Some(&output);
        drag.target_output = Some(output);
        changed
    };
    if !changed {
        if Rc::ptr_eq(target, source) {
            move_drag(source, x, y);
        } else {
            let paint = {
                let mut state = target.borrow_mut();
                if let Some(incoming) = &mut state.incoming {
                    incoming.pointer = (x, y);
                }
                state.paint.clone()
            };
            paint.queue_draw();
        }
        return;
    }
    for surface in surfaces() {
        let (paint, incoming) = {
            let mut state = surface.borrow_mut();
            (state.paint.clone(), state.incoming.take())
        };
        drop(incoming);
        paint.queue_draw();
    }
    if Rc::ptr_eq(target, source) {
        desktop_drag_target(source, true);
        // niri uses the native icon over the source, including on reentry after
        // another output temporarily owned the local preview.
        let native = {
            let state = source.borrow();
            state
                .drag
                .as_ref()
                .filter(|d| !d.local_preview)
                .and_then(|d| Some((d.visual.clone()?, d.preview.clone())))
        };
        if let Some((visual, preview)) = native {
            visual.set_paintable(Some(&preview));
        }
        move_drag(source, x, y);
        return;
    }
    desktop_drag_target(source, false);
    let (preview, hot, visual, paint) = {
        let mut state = source.borrow_mut();
        let Some(drag) = state.drag.as_mut() else {
            return;
        };
        drag.over_desktop = false;
        (
            drag.preview.clone(),
            drag.hotspot,
            drag.visual.clone(),
            state.paint.clone(),
        )
    };
    let target_paint = {
        let mut state = target.borrow_mut();
        state.incoming = Some(IncomingDrag {
            preview,
            pointer: (x, y),
            hotspot: hot,
        });
        state.paint.clone()
    };
    if let Some(visual) = visual {
        hide_native_visual(&visual);
    }
    paint.queue_draw();
    target_paint.queue_draw();
}
pub(super) fn hide_native_visual(visual: &gtk::Picture) {
    let empty = gdk::MemoryTexture::new(
        1,
        1,
        gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from_static(&[0, 0, 0, 0]),
        4,
    );
    visual.set_paintable(Some(&empty));
}
pub(super) fn leave_desktop(d: &Rc<RefCell<Desktop>>) {
    let output = { d.borrow().output.clone() };
    let (paint, incoming) = {
        let mut state = d.borrow_mut();
        (state.paint.clone(), state.incoming.take())
    };
    drop(incoming);
    paint.queue_draw();
    for source in surfaces() {
        let leaving = {
            let mut state = source.borrow_mut();
            state.drag.as_mut().is_some_and(|drag| {
                if drag.target_output.as_ref() != Some(&output) {
                    return false;
                }
                drag.target_output = None;
                true
            })
        };
        if !leaving {
            continue;
        }
        desktop_drag_target(&source, false);
        let visual = {
            let state = source.borrow();
            state
                .drag
                .as_ref()
                .filter(|drag| drag.end != DragEnd::Local)
                .and_then(|drag| Some((drag.visual.clone()?, drag.preview.clone())))
        };
        if let Some((visual, preview)) = visual {
            visual.set_paintable(Some(&preview));
        }
    }
}
pub(super) fn clear_drag_surfaces(d: &Rc<RefCell<Desktop>>) {
    let shared = { d.borrow().shared.clone() };
    shared.borrow_mut().drag_names.clear();
    for surface in surfaces() {
        let (paint, incoming) = {
            let mut s = surface.borrow_mut();
            (s.paint.clone(), s.incoming.take())
        };
        drop(incoming);
        paint.queue_draw();
    }
}
pub(super) fn finish_cross_drag(target: &Rc<RefCell<Desktop>>, source: &Rc<RefCell<Desktop>>, x: f64, y: f64) {
    if Rc::ptr_eq(target, source) {
        finish_drag(source, x, y);
        return;
    }
    let (grid, page, output, shared) = {
        let t = target.borrow();
        (t.grid, t.page, t.output.clone(), t.shared.clone())
    };
    let dropped = {
        let mut state = source.borrow_mut();
        let Some(drag) = state.drag.as_mut() else {
            return;
        };
        let dropped = drag.movement.dropped(grid, x, y, page);
        drag.end.event(DragEvent::LocalDrop);
        dropped
    };
    {
        let mut f = shared.borrow_mut();
        for (name, pos) in &dropped {
            f.saved.insert(name.clone(), *pos);
            f.outputs.insert(name.clone(), output.clone());
        }
    }
    source.borrow_mut().pointer_down = false;
    target.borrow_mut().pointer_down = false;
    // Relayout all outputs before queuing GTK drawing, so ownership changes once. Animated:
    // an icon dropped on a widget glides out beside it instead of jumping there.
    for surface in surfaces() {
        surface.borrow_mut().layout(true);
    }
    clear_drag_surfaces(source);
    for surface in surfaces() {
        animate(&surface);
    }
    focus_selection(target);
    acquire_after_release(target);
    save(target);
}

pub(super) fn drag_selection(selected: &BTreeSet<String>, anchor: &str) -> BTreeSet<String> {
    if selected.contains(anchor) {
        selected.clone()
    } else {
        [anchor.to_owned()].into_iter().collect()
    }
}

/// A one-pixel transparent texture: a real node, so the drag surface still submits a buffer.
pub(super) fn clear_texture() -> gdk::MemoryTexture {
    gdk::MemoryTexture::new(1, 1, gdk::MemoryFormat::R8g8b8a8, &glib::Bytes::from_static(&[0, 0, 0, 0]), 4)
}

pub(super) fn desktop_drag_target(d: &Rc<RefCell<Desktop>>, over: bool) {
    let (paint, visual, preview) = {
        let mut state = d.borrow_mut();
        let Some(drag) = state.drag.as_mut() else {
            return;
        };
        if !drag.local_preview
            || drag.system.is_none()
            || drag.end == DragEnd::Local
            || drag.over_desktop == over
        {
            return;
        }
        drag.over_desktop = over;
        let visual = drag.visual.clone();
        let preview = drag.preview.clone();
        (state.paint.clone(), visual, preview)
    };
    if let Some(visual) = visual {
        if over {
            // A real texture node submits a transparent buffer, clearing the
            // previous native visual. Keep its allocation/hotspot unchanged.
            visual.set_paintable(Some(&clear_texture()));
        } else {
            visual.set_paintable(Some(&preview));
        }
    }
    // Leaving only hides these items; restoration is a terminal drag event.
    paint.queue_draw();
}

pub(super) fn move_drag(d: &Rc<RefCell<Desktop>>, x: f64, y: f64) {
    let paint = {
        let mut state = d.borrow_mut();
        let Some(drag) = state.drag.as_mut() else {
            return;
        };
        if drag.end == DragEnd::Local {
            return;
        }
        if drag.system.is_some() && !(drag.local_preview && drag.over_desktop) {
            return;
        }
        drag.pointer = (x, y);
        let positions = drag.movement.at(x, y);
        for i in &mut state.items {
            if let Some(&(x, y)) = positions.get(&i.data.name) {
                i.x.set(x);
                i.y.set(y);
            }
        }
        state.paint.clone()
    };
    paint.queue_draw();
}

pub(super) fn restore_drag_items(items: &mut [Item], restored: &BTreeMap<String, (f64, f64)>) {
    for i in items {
        if let Some(&(x, y)) = restored.get(&i.data.name) {
            i.x.set(x);
            i.y.set(y);
        }
    }
}

pub(super) fn cancel_pending_drag(d: &Rc<RefCell<Desktop>>) {
    let pending = d
        .borrow()
        .drag
        .as_ref()
        .is_some_and(|drag| drag.system.is_none());
    if pending {
        cancel_drag(d);
    }
}
// Gesture arbitration may deny an empty-point drag too (for example a secondary
// click). Both denied and duplicate end/cancel signals leave no rubber band behind.
pub(super) fn begin_band(
    on_icon: bool,
    primary: bool,
    dragging: bool,
    start: (f64, f64),
    initial: BTreeSet<String>,
    additive: bool,
) -> Option<Rubber> {
    (!on_icon && primary && !dragging).then_some(Rubber {
        start,
        end: start,
        initial,
        additive,
    })
}
pub(super) fn finish_band(rubber: &mut Option<Rubber>, cancelled: bool) -> Option<BTreeSet<String>> {
    rubber.take().and_then(|r| cancelled.then_some(r.initial))
}
pub(super) fn end_band(d: &Rc<RefCell<Desktop>>, cancelled: bool) {
    let dirty = {
        let Ok(mut state) = d.try_borrow_mut() else {
            return;
        };
        if let Some(initial) = finish_band(&mut state.rubber, cancelled) {
            state.select(initial);
        }
        state.shared.borrow().dirty
    };
    animate(d);
    acquire_after_release(d);
    if dirty {
        refresh(d);
    }
}
pub(super) fn cancel_drag(d: &Rc<RefCell<Desktop>>) {
    let Ok(mut state) = d.try_borrow_mut() else {
        return;
    };
    let Some(drag) = state.drag.take() else {
        return;
    };
    restore_drag_items(&mut state.items, &drag.movement.restored());
    let selected = state.selected.clone();
    state.select(selected);
    let refresh_needed = state.shared.borrow().dirty;
    drop(state);
    // Widgets/paintables may finalize with synchronous callbacks too.
    drop(drag);
    clear_drag_surfaces(d);
    for surface in surfaces() {
        if !Rc::ptr_eq(&surface, d) {
            surface.borrow_mut().layout(false);
            animate(&surface);
        }
    }
    animate(d);
    if refresh_needed {
        refresh(d);
    }
}

pub(super) fn drag_event(d: &Rc<RefCell<Desktop>>, system: &gdk::Drag, event: DragEvent) {
    let drag = {
        let mut state = d.borrow_mut();
        let Some(drag) = state.drag.as_mut() else {
            return;
        };
        if drag.system.as_ref() != Some(system) {
            return;
        }
        let Some(result) = drag.end.event(event) else {
            return;
        };
        let drag = state.drag.take().unwrap();
        state.pointer_down = false;
        match result {
            DragResult::Restore => restore_drag_items(&mut state.items, &drag.movement.restored()),
            DragResult::AwaitRemoval => {
                state
                    .shared
                    .borrow_mut()
                    .moved_out
                    .extend(drag.movement.origins.keys().cloned());
            }
            DragResult::KeepLocal => {}
        }
        let selected = state.selected.clone();
        state.select(selected);
        drag
    };
    // GDK callbacks may be followed synchronously by GtkDragSource::drag-end.
    // The model has already transitioned once, and GTK objects finalize unborrowed.
    drop(drag);
    clear_drag_surfaces(d);
    animate(d);
    for surface in surfaces() {
        acquire_after_release(&surface);
    }
    refresh(d);
}

pub(super) fn finish_drag(d: &Rc<RefCell<Desktop>>, x: f64, y: f64) {
    let Ok(mut state) = d.try_borrow_mut() else {
        return;
    };
    let Some(drag) = state.drag.as_ref() else {
        return;
    };
    if drag.end == DragEnd::Local {
        return;
    }
    let dropped = drag.movement.dropped(state.grid, x, y, state.page);
    state.drag.as_mut().unwrap().end.event(DragEvent::LocalDrop);
    for i in &mut state.items {
        if let Some(p) = dropped.get(&i.data.name) {
            i.x.set(p.x);
            i.y.set(p.y);
        }
    }
    {
        let mut shared = state.shared.borrow_mut();
        for (name, pos) in &dropped {
            shared.saved.insert(name.clone(), *pos);
            shared.outputs.insert(name.clone(), state.output.clone());
        }
    }
    state.saved.extend(dropped.clone());
    state.positions.extend(dropped);
    let refresh_needed = state.shared.borrow().dirty;
    drop(state);
    d.borrow_mut().pointer_down = false;
    // Animated: an icon dropped on a widget glides out beside it instead of jumping there.
    for surface in surfaces() {
        surface.borrow_mut().layout(true);
    }
    clear_drag_surfaces(d);
    for surface in surfaces() {
        animate(&surface);
    }
    focus_selection(d);
    acquire_after_release(d);
    save(d);
    animate(d);
    if refresh_needed {
        refresh(d);
    }
}
