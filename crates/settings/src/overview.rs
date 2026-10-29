use crate::{pk, system, ui};
use gtk::{glib, prelude::*};

pub fn build() -> gtk::Widget {
    let (scroll, body) = ui::page("Overview", "");
    let hero = gtk::Box::new(gtk::Orientation::Horizontal, 20);
    hero.add_css_class("hero-card");
    let mark = gtk::Box::new(gtk::Orientation::Vertical, 0);
    mark.add_css_class("hero-mark");
    mark.set_valign(gtk::Align::Center);
    mark.append(&kit::ui::distro_logo(64, false));
    hero.append(&mark);
    let col = gtk::Box::new(gtk::Orientation::Vertical, 4);
    col.set_hexpand(true);
    col.set_valign(gtk::Align::Center);
    col.append(&ui::text(&system::os_name(), "hero"));
    col.append(&ui::text(&format!("{} · up {}", system::hostname(), system::duration(system::uptime())), "sub"));
    let chips = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    chips.set_margin_top(8);
    let status = ui::badge("Checking updates…", "");
    chips.append(&status);
    chips.append(&ui::badge(&format!("Linux {}", system::kernel()), ""));
    chips.append(&ui::badge(&format!("digit {}", env!("CARGO_PKG_VERSION")), ""));
    col.append(&chips);
    let again = ui::button("Set up Digit…", "");
    again.set_halign(gtk::Align::Start);
    again.set_margin_top(10);
    again.connect_clicked(|_| {
        let argv: [&std::ffi::OsStr; 2] = ["digit-settings".as_ref(), "welcome".as_ref()];
        let _ = gtk::gio::Subprocess::newv(&argv, gtk::gio::SubprocessFlags::NONE);
    });
    col.append(&again);
    hero.append(&col);
    body.append(&hero);
    body.append(&ui::heading("Hardware"));
    let hw = ui::card();
    hw.append(&ui::row(Some("developer_board"), "Processor", &system::cpu()).0);
    let (total, free) = system::memory();
    hw.append(&ui::row(Some("memory_alt"), "Memory", &format!("{} total · {} available", system::bytes(total), system::bytes(free))).0);
    for g in system::gpus() {
        hw.append(&ui::row(Some("memory"), "Graphics", &format!("{} {} · driver {}", g.vendor, g.name, g.driver)).0);
    }
    for (mount, label) in [("/", "System disk"), ("/home", "Home")] {
        if let Some((size, avail)) = system::disk(mount) {
            let used = size.saturating_sub(avail);
            let (row, end) = ui::row(Some("hard_drive"), label, &format!("{} used of {} · {} free", system::bytes(used), system::bytes(size), system::bytes(avail)));
            let bar = gtk::ProgressBar::new();
            bar.set_fraction(if size > 0 { used as f64 / size as f64 } else { 0.0 });
            bar.set_size_request(160, -1);
            end.append(&bar);
            hw.append(&row);
        }
    }
    body.append(&hw);
    body.append(&ui::heading("Software"));
    let sw = ui::card();
    sw.append(&ui::row(Some("terminal"), "Kernel", &system::kernel()).0);
    let (niri_row, _) = ui::row(Some("view_column"), "Compositor", kit::wm::name());
    sw.append(&niri_row);
    sw.append(&ui::row(Some("deployed_code"), "Shell", &format!("digit {}", env!("CARGO_PKG_VERSION"))).0);
    body.append(&sw);
    let sub = niri_row.first_child().and_then(|c| c.next_sibling()).and_then(|col| col.last_child()).and_then(|l| l.downcast::<gtk::Label>().ok());
    glib::spawn_future_local(async move {
        if let Ok(Some(v)) = gtk::gio::spawn_blocking(kit::wm::version).await {
            if let Some(l) = &sub {
                l.set_text(&v);
            }
        }
    });
    glib::spawn_future_local(async move {
        match pk::updates().await {
            Ok(list) if list.is_empty() => {
                status.set_text("Up to date");
                status.add_css_class("good");
            }
            Ok(list) => {
                let security = list.iter().filter(|p| p.info == pk::INFO_SECURITY).count();
                status.set_text(&if security > 0 { format!("{} updates · {security} security", list.len()) } else { format!("{} updates", list.len()) });
                status.add_css_class(if security > 0 { "warn" } else { "accent" });
            }
            Err(_) => status.set_text("Updates unknown"),
        }
    });
    scroll.upcast()
}
