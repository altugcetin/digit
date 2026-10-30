use crate::ui;
use futures_util::StreamExt;
use gtk::{glib, prelude::*};
use kit::nm::{self, Network, Wifi};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

struct Page {
    root: glib::WeakRef<gtk::Widget>,
    power: gtk::Box,
    current_head: gtk::Label,
    current: gtk::Box,
    networks: gtk::Box,
    saved_head: gtk::Label,
    saved: gtk::Box,
    scan: gtk::Button,
    wifi: RefCell<Option<Wifi>>,
    shape: RefCell<String>,
    joining: RefCell<Option<String>>,
    pending: Cell<bool>,
}

pub fn build() -> gtk::Widget {
    let (scroll, body) = ui::page("Wi-Fi", "Join wireless networks and manage the ones this computer remembers.");
    let power = ui::card();
    body.append(&power);
    let current_head = ui::heading("Connected");
    let current = ui::card();
    body.append(&current_head);
    body.append(&current);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let title = ui::heading("Networks");
    title.set_hexpand(true);
    head.append(&title);
    let scan = ui::button("Scan", "");
    scan.set_valign(gtk::Align::End);
    head.append(&scan);
    body.append(&head);
    let networks = ui::card();
    body.append(&networks);
    let saved_head = ui::heading("Saved networks");
    let saved = ui::card();
    body.append(&saved_head);
    body.append(&saved);
    let page = Rc::new(Page {
        root: scroll.upcast_ref::<gtk::Widget>().downgrade(),
        power,
        current_head,
        current,
        networks,
        saved_head,
        saved,
        scan: scan.clone(),
        wifi: RefCell::new(None),
        shape: RefCell::new(String::new()),
        joining: RefCell::new(None),
        pending: Cell::new(false),
    });
    let weak = Rc::downgrade(&page);
    scan.connect_clicked(move |b| {
        let Some(device) = weak.upgrade().and_then(|p| p.wifi.borrow().as_ref().and_then(|w| w.device.clone())) else { return };
        b.set_sensitive(false);
        let b = b.clone();
        glib::spawn_future_local(async move {
            if let Ok(conn) = kit::pk::system().await {
                if let Err(e) = nm::scan(&conn, &device).await {
                    ui::fail(&format!("Couldn't scan: {e}"));
                }
            }
            glib::timeout_future(Duration::from_secs(3)).await;
            b.set_sensitive(true);
        });
    });
    unsafe { scroll.set_data("digit-wifi", page.clone()) };
    watch(Rc::downgrade(&page));
    refresh(&page);
    scroll.upcast()
}

fn refresh(page: &Rc<Page>) {
    let weak = Rc::downgrade(page);
    glib::spawn_future_local(async move {
        let Ok(conn) = kit::pk::system().await else { return };
        let wifi = nm::read(&conn).await;
        if let Some(p) = weak.upgrade() {
            render(&p, wifi);
        }
    });
}

fn watch(weak: Weak<Page>) {
    glib::spawn_future_local(async move {
        let Ok(conn) = kit::pk::system().await else { return };
        let Some(mut stream) = nm::changes(&conn).await else { return };
        while stream.next().await.is_some() {
            let Some(p) = weak.upgrade() else { return };
            if p.root.upgrade().and_then(|r| r.root()).is_none() {
                return;
            }
            if p.pending.replace(true) {
                continue;
            }
            let w = Rc::downgrade(&p);
            glib::timeout_add_local_once(Duration::from_millis(700), move || {
                if let Some(p) = w.upgrade() {
                    p.pending.set(false);
                    refresh(&p);
                }
            });
        }
    });
}

/// What the page shows, without signal noise, so a changing strength only redraws when a bar changes.
fn shape(w: &Wifi, joining: &Option<String>) -> String {
    let nets: Vec<_> = w.networks.iter().map(|n| (&n.ssid, nm::bars(n.strength), n.saved.is_some(), n.active, n.security)).collect();
    let saved: Vec<_> = w.saved.iter().map(|s| &s.path).collect();
    let cur = w.current.as_ref().map(|c| (&c.ssid, &c.address, &c.gateway, &c.dns, &c.ipv6, c.bitrate, nm::bars(c.strength)));
    format!("{} {} {:?} {} {:?} {:?} {:?} {:?}", w.enabled, w.hardware, w.device, w.state, nets, saved, cur, joining)
}

fn render(page: &Rc<Page>, w: Wifi) {
    let key = shape(&w, &page.joining.borrow());
    *page.wifi.borrow_mut() = Some(w.clone());
    if *page.shape.borrow() == key {
        return;
    }
    *page.shape.borrow_mut() = key;
    ui::clear(&page.power);
    let sub = match (&w.device, w.hardware) {
        (None, _) => "No Wi-Fi hardware found".to_string(),
        (_, false) => "Turned off by a hardware switch".to_string(),
        (Some(_), true) if w.enabled => format!("On · {}", w.interface),
        _ => format!("Off · {}", w.interface),
    };
    let (row, end) = ui::row(Some(if w.enabled { "wifi" } else { "wifi_off" }), "Wi-Fi", &sub);
    let sw = gtk::Switch::new();
    sw.set_active(w.enabled);
    sw.set_sensitive(w.device.is_some() && w.hardware);
    sw.set_valign(gtk::Align::Center);
    sw.connect_active_notify(|s| {
        let on = s.is_active();
        glib::spawn_future_local(async move {
            let Ok(conn) = kit::pk::system().await else { return };
            if let Err(e) = nm::set_enabled(&conn, on).await {
                ui::fail(&format!("Wi-Fi: {e}"));
            }
        });
    });
    end.append(&sw);
    page.power.append(&row);
    let Some(device) = w.device.clone().filter(|_| w.enabled) else {
        for x in [page.current_head.upcast_ref::<gtk::Widget>(), page.current.upcast_ref(), page.networks.upcast_ref(), page.scan.upcast_ref()] {
            x.set_visible(false);
        }
        saved_list(page, &w);
        return;
    };
    page.scan.set_visible(true);
    page.networks.set_visible(true);
    page.current_head.set_visible(w.current.is_some());
    page.current.set_visible(w.current.is_some());
    ui::clear(&page.current);
    if let Some(c) = &w.current {
        let mut meta = vec![c.security.label().to_string(), c.band()];
        if c.bitrate > 0 {
            meta.push(format!("{} Mb/s", c.bitrate / 1000));
        }
        let (row, end) = ui::row(Some(nm::bars(c.strength)), &c.ssid, &meta.join(" · "));
        let off = ui::button("Disconnect", "");
        let dev = device.clone();
        off.connect_clicked(move |_| {
            let dev = dev.clone();
            glib::spawn_future_local(async move {
                let Ok(conn) = kit::pk::system().await else { return };
                if let Err(e) = nm::disconnect(&conn, &dev).await {
                    ui::fail(&format!("Couldn't disconnect: {e}"));
                }
            });
        });
        end.append(&off);
        page.current.append(&row);
        let facts = [
            ("Signal", format!("{}%", c.strength)),
            ("IP address", c.address.clone()),
            ("Router", c.gateway.clone()),
            ("DNS", c.dns.join(", ")),
            ("IPv6", c.ipv6.clone()),
            ("Hardware address", c.mac.clone()),
            ("Access point", c.bssid.clone()),
        ];
        for (label, value) in facts {
            if value.is_empty() {
                continue;
            }
            let (row, end) = ui::row(None, label, "");
            let v = ui::text(&value, "dim");
            v.set_selectable(true);
            end.append(&v);
            page.current.append(&row);
        }
    }
    ui::clear(&page.networks);
    let others: Vec<&Network> = w.networks.iter().filter(|n| !n.active).collect();
    if others.is_empty() {
        let (row, end) = ui::row(Some("wifi_find"), "Looking for networks…", "");
        end.append(&gtk::Spinner::builder().spinning(true).build());
        page.networks.append(&row);
    }
    let joining = page.joining.borrow().clone();
    for n in others {
        let mut meta: Vec<String> = Vec::new();
        if n.saved.is_some() {
            meta.push("Saved".into());
        }
        meta.push(n.security.label().into());
        meta.push(nm::band(n.frequency));
        let (row, end) = ui::row(Some(nm::bars(n.strength)), &n.ssid, &meta.join(" · "));
        if n.security.locked() {
            let lock = ui::sym("lock");
            lock.add_css_class("dim");
            end.append(&lock);
        }
        if joining.as_deref() == Some(n.ssid.as_str()) {
            end.append(&gtk::Spinner::builder().spinning(true).build());
        } else {
            let b = ui::button("Join", if n.saved.is_some() { "accent" } else { "" });
            b.set_sensitive(joining.is_none());
            let (weak, n, dev) = (Rc::downgrade(page), n.clone(), device.clone());
            b.connect_clicked(move |b| {
                if let Some(p) = weak.upgrade() {
                    ask_join(&p, b, &dev, &n);
                }
            });
            end.append(&b);
        }
        page.networks.append(&row);
    }
    saved_list(page, &w);
}

fn saved_list(page: &Rc<Page>, w: &Wifi) {
    ui::clear(&page.saved);
    page.saved_head.set_visible(!w.saved.is_empty());
    page.saved.set_visible(!w.saved.is_empty());
    for s in &w.saved {
        let here = w.current.as_ref().is_some_and(|c| c.ssid == s.ssid);
        let near = w.networks.iter().any(|n| n.ssid == s.ssid);
        let (row, end) = ui::row(Some("wifi_lock"), &s.id, if here { "Connected" } else if near { "In range" } else { "Not in range" });
        let b = ui::button("Forget", "danger");
        let (path, name) = (s.path.clone(), s.id.clone());
        b.connect_clicked(move |b| {
            let (path, name2) = (path.clone(), name.clone());
            ui::ask(ui::root(b).as_ref(), &format!("Forget {name}?"), "Its password is removed and it will not join by itself any more.", "Cancel", "Forget", "danger", move || {
                glib::spawn_future_local(async move {
                    let Ok(conn) = kit::pk::system().await else { return };
                    match nm::forget(&conn, &path).await {
                        Ok(()) => ui::done(&format!("{name2} forgotten")),
                        Err(e) => ui::fail(&format!("Couldn't forget {name2}: {e}")),
                    }
                });
            });
        });
        end.append(&b);
        page.saved.append(&row);
    }
}

fn ask_join(page: &Rc<Page>, from: &gtk::Button, device: &str, n: &Network) {
    if n.saved.is_some() || !n.security.locked() {
        join(page, device, n, None);
        return;
    }
    let (weak, dev, net) = (Rc::downgrade(page), device.to_string(), n.clone());
    ui::prompt(ui::root(from), &format!("Join {}", n.ssid), &format!("{} needs a password.", n.security.label()), Some(("Password", true)), "Join", move |pw| {
        if let (Some(pw), Some(p)) = (pw, weak.upgrade()) {
            join(&p, &dev, &net, Some(pw));
        }
    });
}

fn join(page: &Rc<Page>, device: &str, n: &Network, password: Option<String>) {
    *page.joining.borrow_mut() = Some(n.ssid.clone());
    page.shape.borrow_mut().clear();
    refresh(page);
    ui::busy(&format!("Joining {}…", n.ssid), None);
    let (weak, dev, net) = (Rc::downgrade(page), device.to_string(), n.clone());
    glib::spawn_future_local(async move {
        let r = match kit::pk::system().await {
            Ok(conn) => nm::join(&conn, &dev, &net, password.as_deref()).await,
            Err(e) => Err(e),
        };
        let retry = r.is_err() && password.is_none() && net.saved.is_some() && net.security.locked();
        match r {
            Ok(()) => ui::done(&format!("Connected to {}", net.ssid)),
            Err(e) => ui::fail(&e),
        }
        let Some(p) = weak.upgrade() else { return };
        p.joining.borrow_mut().take();
        p.shape.borrow_mut().clear();
        refresh(&p);
        if retry {
            let weak = Rc::downgrade(&p);
            let parent = p.root.upgrade().as_ref().and_then(ui::root);
            ui::prompt(parent, &format!("Join {}", net.ssid), "The saved password did not work. Type it again.", Some(("Password", true)), "Join", move |pw| {
                if let (Some(pw), Some(p)) = (pw, weak.upgrade()) {
                    join(&p, &dev, &net, Some(pw));
                }
            });
        }
    });
}
