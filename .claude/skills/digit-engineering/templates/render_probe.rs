// TEMPORARY render probe: draws a GTK page offscreen to PNGs so its look can be
// checked without touching the live desktop. Paste at the end of the crate's
// test module (Settings: src/tests.rs), adapt `build`, run, look, then DELETE
// before committing.
//
// Run (Broadway backend, no Wayland, isolated config):
//   (broadwayd :7 >/dev/null 2>&1 &); sleep 1
//   S=<scratchpad>; mkdir -p $S/probe/{config,data,cache}
//   GDK_BACKEND=broadway BROADWAY_DISPLAY=:7 WAYLAND_DISPLAY= \
//   XDG_CONFIG_HOME=$S/probe/config XDG_DATA_HOME=$S/probe/data XDG_CACHE_HOME=$S/probe/cache \
//   PROBE=$S cargo test --release -p digit-settings probe -- --nocapture
//   pkill -x broadwayd
#[cfg(test)]
mod render_probe {
    use gtk::prelude::*;

    fn settle(ms: u64) {
        let ctx = gtk::glib::MainContext::default();
        let end = std::time::Instant::now() + std::time::Duration::from_millis(ms);
        while std::time::Instant::now() < end {
            ctx.iteration(false);
        }
    }

    #[test]
    fn probe() {
        let Some(out) = std::env::var_os("PROBE") else { return };
        if gtk::init().is_err() {
            return;
        }
        // The app's own CSS, as the real binary loads it.
        kit::style::live_with(&gtk::gdk::Display::default().unwrap(), crate::style::css, || {});
        for (name, width, height) in [("compositor", 820, 900)] {
            let win = gtk::Window::new();
            win.set_default_size(width, height);
            let page = crate::build_page(name); // ← what to draw
            win.set_child(Some(&page));
            win.present();
            settle(2500);
            // The first snapshots can be empty while the window maps: retry.
            let node = (0..40).find_map(|_| {
                let snap = gtk::Snapshot::new();
                win.snapshot_child(&page, &snap);
                snap.to_node().or_else(|| {
                    settle(100);
                    None
                })
            });
            let node = node.expect("page never drew");
            let texture = win.native().unwrap().renderer().unwrap().render_texture(&node, None);
            let path = std::path::Path::new(&out).join(format!("{name}.png"));
            texture.save_to_png(&path).unwrap();
            println!("probe: {}", path.display());
        }
    }
}
