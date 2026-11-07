//! Frozen drag icons, including Hyprland 0.57's non-accumulating surface offset.
use gtk::{gdk, glib, prelude::*};
use std::{cell::Cell, ffi::c_void, rc::Rc};

/// Install before the first frame. No caller may hold a model borrow here.
pub fn set_icon(drag: &gdk::Drag, paintable: &gdk::Paintable, hotspot: (i32, i32)) -> gtk::Picture {
    let icon = gtk::DragIcon::for_drag(drag);
    let picture = gtk::Picture::for_paintable(paintable);
    picture.set_can_shrink(false);
    picture.set_size_request(
        paintable.intrinsic_width().max(1),
        paintable.intrinsic_height().max(1),
    );
    icon.set_child(Some(&picture));
    drag.set_hotspot(hotspot.0, hotspot.1);
    repair_hotspot(drag, hotspot);
    picture
}

fn repair_hotspot(drag: &gdk::Drag, hotspot: (i32, i32)) {
    if crate::wm::which() != crate::wm::Wm::Hyprland {
        return;
    }
    let Some(surface) = drag.drag_surface() else {
        return;
    };
    let Some(wayland) = glib::Type::from_name("GdkWaylandSurface") else {
        return;
    };
    if !surface.type_().is_a(wayland) {
        return;
    }
    let clock = surface.frame_clock();
    let weak = surface.downgrade();
    let handler = clock.connect_after_paint(move |_| {
        let Some(surface) = weak.upgrade() else {
            return;
        };
        if !surface.is_destroyed() && surface.is_mapped() {
            commit_offset(&surface, surface_offset(hotspot));
        }
    });
    // Do not keep repairing the surface during GTK's end/cancel teardown.
    let handler = Rc::new(Cell::new(Some(handler)));
    let (c, h) = (clock.clone(), handler.clone());
    drag.connect_cancel(move |_, _| {
        if let Some(id) = h.take() {
            c.disconnect(id);
        }
    });
    drag.connect_dnd_finished(move |_| {
        if let Some(id) = handler.take() {
            clock.disconnect(id);
        }
    });
}

fn surface_offset(hotspot: (i32, i32)) -> (i32, i32) {
    (-hotspot.0, -hotspot.1)
}

// The three ABI declarations below mirror GTK's public Wayland accessor and
// wayland-client-protocol.h's wl_surface_offset/commit inline wrappers. Keep
// this here rather than adding Wayland crate dependencies to every companion.
#[link(name = "gtk-4")]
unsafe extern "C" {
    fn gdk_wayland_surface_get_wl_surface(surface: *mut gdk::ffi::GdkSurface) -> *mut c_void;
}
#[link(name = "wayland-client")]
unsafe extern "C" {
    fn wl_proxy_get_version(proxy: *mut c_void) -> u32;
    fn wl_proxy_marshal_flags(
        proxy: *mut c_void,
        opcode: u32,
        interface: *const c_void,
        version: u32,
        flags: u32,
        ...
    ) -> *mut c_void;
}

fn commit_offset(surface: &gdk::Surface, offset: (i32, i32)) {
    use glib::translate::ToGlibPtr;
    // SAFETY: set_icon checked the Wayland GType; this live, mapped GDK surface
    // owns the borrowed wl_surface. Requests run on GTK's thread, create no new
    // objects, and neither destroy the proxy nor install an event dispatcher.
    unsafe {
        let wl = gdk_wayland_surface_get_wl_surface(surface.to_glib_none().0);
        if wl.is_null() {
            return;
        }
        let version = wl_proxy_get_version(wl);
        if version < 5 {
            return;
        }
        // Hyprland core/Compositor.cpp's attach(0, 0) overwrites pending.offset,
        // even at v5+. Its SurfaceState::updateFrom assigns rather than adds it;
        // core/DataDevice.cpp::renderDND uses that value as an absolute offset.
        // GTK's GL/Vulkan end_frame queues offset BEFORE presenting the buffer,
        // so both the initial attach and later frames can erase the hotspot.
        // After paint, send the absolute offset in its own buffer-free commit.
        // A driver's deferred presentation can still commit later; the desktop
        // uses its own target-motion visual so it never depends on that timing.
        // No invalidation, tick, polling, buffer attach or extra window needed.
        wl_proxy_marshal_flags(wl, 10, std::ptr::null(), version, 0, offset.0, offset.1);
        wl_proxy_marshal_flags(wl, 6, std::ptr::null(), version, 0);
    }
    surface.display().flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hyprland_gpu_attaches_and_repeat_frames_need_an_offset_after_presentation() {
        let hot = (46, 37);
        for gtk_delta in [surface_offset(hot), (0, 0), (0, 0)] {
            // Model core/Compositor.cpp: offset followed by GPU attach(0, 0)
            // loses GTK's pending delta, even on the initial frame.
            let attach = |pending: &mut (i32, i32), x, y| *pending = (x, y);
            let mut pending = gtk_delta;
            attach(&mut pending, 0, 0);
            let mut current = pending;
            assert_eq!(current, (0, 0));
            // Our after-paint commit must assign the full offset, not a delta.
            pending = surface_offset(hot);
            current = pending;
            assert_eq!(
                (901 + current.0 + hot.0, 210 + current.1 + hot.1),
                (901, 210)
            );
        }
    }

    #[test]
    fn niri_accumulates_the_initial_hotspot_and_zero_deltas_without_repairs() {
        let hot = (46, 37);
        let mut offset = (0, 0);
        for delta in [surface_offset(hot), (0, 0), (0, 0)] {
            offset = (offset.0 + delta.0, offset.1 + delta.1);
            assert_eq!(offset, (-46, -37));
        }
        // Sending Hyprland's absolute repair on niri would accumulate and drift.
        assert_ne!((offset.0 - hot.0, offset.1 - hot.1), offset);
    }
}
