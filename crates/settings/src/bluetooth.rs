use crate::ui;
use futures_util::StreamExt;
use gtk::{glib, prelude::*};
use kit::bluez::{self, Adapter, Ask, Device, Snapshot};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

struct Page {
    root: glib::WeakRef<gtk::Widget>,
    adapter: gtk::Box,
    devices: gtk::Box,
    nearby_head: gtk::Label,
    nearby: gtk::Box,
    scan: gtk::Button,
    snap: RefCell<Option<Snapshot>>,
    shape: RefCell<String>,
    busy: RefCell<Vec<String>>,
    scanning: RefCell<Option<String>>,
    round: Cell<u32>,
    pending: Cell<bool>,
}

thread_local! {
    static AGENT: Cell<bool> = const { Cell::new(false) };
    static DIALOG: RefCell<Option<gtk::Window>> = const { RefCell::new(None) };
}

pub fn build() -> gtk::Widget {
    let (scroll, body) = ui::page("Bluetooth", "Choose an adapter, then find, pair and connect devices with it.");
    body.append(&ui::heading("Adapter"));
    let adapter = ui::card();
    body.append(&adapter);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let title = ui::heading("My devices");
    title.set_hexpand(true);
    head.append(&title);
    let scan = ui::button("Scan", "");
    scan.set_valign(gtk::Align::End);
    head.append(&scan);
    body.append(&head);
    let devices = ui::card();
    body.append(&devices);
    let nearby_head = ui::heading("Nearby devices");
    let nearby = ui::card();
    body.append(&nearby_head);
    body.append(&nearby);
    let page = Rc::new(Page {
        root: scroll.upcast_ref::<gtk::Widget>().downgrade(),
        adapter,
        devices,
        nearby_head,
        nearby,
        scan: scan.clone(),
        snap: RefCell::new(None),
        shape: RefCell::new(String::new()),
        busy: RefCell::new(Vec::new()),
        scanning: RefCell::new(None),
        round: Cell::new(0),
        pending: Cell::new(false),
    });
    let weak = Rc::downgrade(&page);
    scan.connect_clicked(move |_| {
        if let Some(p) = weak.upgrade() {
            toggle_scan(&p);
        }
    });
    let weak = Rc::downgrade(&page);
    scroll.connect_unrealize(move |_| {
        if let Some(path) = weak.upgrade().and_then(|p| p.scanning.borrow_mut().take()) {
            glib::spawn_future_local(async move {
                if let Ok(conn) = kit::pk::system().await {
                    let _ = bluez::scan(&conn, &path, false).await;
                }
            });
        }
    });
    unsafe { scroll.set_data("digit-bluetooth", page.clone()) };
    agent();
    watch(Rc::downgrade(&page));
    refresh(&page);
    scroll.upcast()
}

fn refresh(page: &Rc<Page>) {
    let weak = Rc::downgrade(page);
    glib::spawn_future_local(async move {
        let Ok(conn) = kit::pk::system().await else { return };
        let snap = bluez::read(&conn).await;
        if let Some(p) = weak.upgrade() {
            render(&p, snap);
        }
    });
}

fn watch(weak: Weak<Page>) {
    glib::spawn_future_local(async move {
        let Ok(conn) = kit::pk::system().await else { return };
        let Some(mut stream) = bluez::changes(&conn).await else { return };
        while stream.next().await.is_some() {
            let Some(p) = weak.upgrade() else { return };
            if p.root.upgrade().and_then(|r| r.root()).is_none() {
                return;
            }
            if p.pending.replace(true) {
                continue;
            }
            let w = Rc::downgrade(&p);
            glib::timeout_add_local_once(Duration::from_millis(250), move || {
                if let Some(p) = w.upgrade() {
                    p.pending.set(false);
                    refresh(&p);
                }
            });
        }
    });
}

fn chosen(page: &Page) -> Option<Adapter> {
    page.snap.borrow().as_ref().and_then(|s| s.chosen().cloned())
}

fn render(page: &Rc<Page>, snap: Snapshot) {
    if page.snap.borrow().as_ref() == Some(&snap) {
        return;
    }
    let pick = snap.chosen().cloned();
    let shape = format!("{:?}", snap.adapters.iter().map(|a| (&a.id, &a.name, a.powered, a.discoverable)).collect::<Vec<_>>()) + &format!("{:?}", pick.as_ref().map(|a| &a.id));
    *page.snap.borrow_mut() = Some(snap.clone());
    if *page.shape.borrow() != shape {
        *page.shape.borrow_mut() = shape;
        adapter_card(page, &snap, pick.as_ref());
    }
    let Some(a) = pick else {
        page.scan.set_visible(false);
        for w in [page.devices.upcast_ref::<gtk::Widget>(), page.nearby.upcast_ref(), page.nearby_head.upcast_ref()] {
            w.set_visible(false);
        }
        return;
    };
    page.scan.set_visible(true);
    page.scan.set_sensitive(a.powered);
    page.scan.set_label(if a.discovering { "Stop scanning" } else { "Scan" });
    if a.discovering { page.scan.add_css_class("accent") } else { page.scan.remove_css_class("accent") }
    page.devices.set_visible(true);
    ui::clear(&page.devices);
    let paired: Vec<&Device> = snap.on(&a.path).filter(|d| d.paired).collect();
    if paired.is_empty() {
        let (row, _) = ui::row(Some("bluetooth_searching"), "No paired devices", if a.powered { "Scan to find devices nearby" } else { "Turn the adapter on to use Bluetooth" });
        page.devices.append(&row);
    }
    for d in paired {
        page.devices.append(&device_row(page, &a, d));
    }
    let others: Vec<&Device> = snap.on(&a.path).filter(|d| !d.paired).take(40).collect();
    let show = a.powered && (a.discovering || !others.is_empty());
    page.nearby_head.set_visible(show);
    page.nearby.set_visible(show);
    ui::clear(&page.nearby);
    if show && others.is_empty() {
        let (row, end) = ui::row(Some("bluetooth_searching"), "Looking for devices…", "Put the device in pairing mode");
        end.append(&gtk::Spinner::builder().spinning(true).build());
        page.nearby.append(&row);
    }
    for d in others {
        page.nearby.append(&device_row(page, &a, d));
    }
}

fn adapter_card(page: &Rc<Page>, snap: &Snapshot, pick: Option<&Adapter>) {
    ui::clear(&page.adapter);
    let Some(a) = pick else {
        let (row, _) = ui::row(Some("bluetooth_disabled"), "No Bluetooth adapter", "Plug one in, or start bluetooth.service");
        page.adapter.append(&row);
        return;
    };
    let (row, end) = ui::row(Some("bluetooth"), &a.name, &format!("{} · {}", a.id, a.address));
    if snap.adapters.len() > 1 {
        let labels: Vec<String> = snap.adapters.iter().map(|x| format!("{} ({})", x.name, x.id)).collect();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let at = snap.adapters.iter().position(|x| x.id == a.id).unwrap_or(0);
        let list = ui::dropdown(&refs, at as u32);
        let ids: Vec<String> = snap.adapters.iter().map(|x| x.id.clone()).collect();
        let weak = Rc::downgrade(page);
        list.connect_selected_notify(move |l| {
            let Some(id) = ids.get(l.selected() as usize) else { return };
            if let Err(e) = bluez::choose(id) {
                ui::fail(&format!("Could not save: {e}"));
            }
            if let Some(p) = weak.upgrade() {
                stop_scan(&p);
                p.snap.borrow_mut().take();
                refresh(&p);
            }
        });
        end.append(&list);
    } else {
        end.append(&ui::badge(&a.id, ""));
    }
    page.adapter.append(&row);
    let path = a.path.clone();
    switch(&page.adapter, "power_settings_new", "Power", "Turn this adapter on or off", a.powered, true, move |on| {
        let path = path.clone();
        glib::spawn_future_local(async move {
            let Ok(conn) = kit::pk::system().await else { return };
            if let Err(e) = bluez::set_adapter(&conn, &path, "Powered", on).await {
                ui::fail(&format!("Bluetooth: {e}"));
            }
        });
    });
    let path = a.path.clone();
    switch(&page.adapter, "visibility", "Visible", "Let other devices find this computer", a.discoverable, a.powered, move |on| {
        let path = path.clone();
        glib::spawn_future_local(async move {
            let Ok(conn) = kit::pk::system().await else { return };
            if let Err(e) = bluez::set_adapter(&conn, &path, "Discoverable", on).await {
                ui::fail(&format!("Bluetooth: {e}"));
            }
        });
    });
}

fn switch(card: &gtk::Box, icon: &str, title: &str, sub: &str, on: bool, sensitive: bool, changed: impl Fn(bool) + 'static) {
    let (row, end) = ui::row(Some(icon), title, sub);
    let sw = gtk::Switch::new();
    sw.set_active(on);
    sw.set_sensitive(sensitive);
    sw.set_valign(gtk::Align::Center);
    sw.connect_active_notify(move |s| changed(s.is_active()));
    end.append(&sw);
    card.append(&row);
}

fn icon_button(icon: &str, tip: &str, kind: &str) -> gtk::Button {
    let b = gtk::Button::new();
    b.set_child(Some(&ui::sym(icon)));
    b.add_css_class("pill");
    if !kind.is_empty() {
        b.add_css_class(kind);
    }
    b.set_tooltip_text(Some(tip));
    b.set_valign(gtk::Align::Center);
    b
}

fn describe(d: &Device) -> String {
    let mut parts: Vec<String> = Vec::new();
    if d.connected {
        parts.push("Connected".into());
    } else if d.paired {
        parts.push("Not connected".into());
    }
    if let Some(b) = d.battery {
        parts.push(format!("Battery {b}%"));
    }
    if d.trusted && d.paired {
        parts.push("Trusted".into());
    }
    if !d.paired {
        if let Some(r) = d.rssi {
            parts.push(match r {
                r if r >= -60 => "Strong signal".into(),
                r if r >= -75 => "Good signal".into(),
                _ => "Weak signal".into(),
            });
        }
    }
    parts.push(d.address.clone());
    parts.join(" · ")
}

fn device_row(page: &Rc<Page>, a: &Adapter, d: &Device) -> gtk::Box {
    let (row, end) = ui::row(Some(bluez::symbol(&d.icon)), &d.name, &describe(d));
    if page.busy.borrow().contains(&d.path) {
        end.append(&gtk::Spinner::builder().spinning(true).build());
        return row;
    }
    if !a.powered {
        return row;
    }
    if !d.paired {
        let b = ui::button("Pair", "accent");
        let (path, name) = (d.path.clone(), d.name.clone());
        let weak = Rc::downgrade(page);
        b.connect_clicked(move |_| {
            let Some(p) = weak.upgrade() else { return };
            let path2 = path.clone();
            act(&p, &path, format!("Pairing with {name}…"), format!("{name} is paired"), async move {
                let conn = kit::pk::system().await?;
                bluez::pair(&conn, &path2).await
            });
        });
        end.append(&b);
        return row;
    }
    let trust = icon_button("verified_user", if d.trusted { "Trusted: connects without asking. Click to stop trusting." } else { "Trust: let it connect without asking" }, if d.trusted { "accent" } else { "" });
    let (path, on) = (d.path.clone(), !d.trusted);
    trust.connect_clicked(move |_| {
        let path = path.clone();
        glib::spawn_future_local(async move {
            let Ok(conn) = kit::pk::system().await else { return };
            if let Err(e) = bluez::set_device(&conn, &path, "Trusted", on).await {
                ui::fail(&format!("Bluetooth: {e}"));
            }
        });
    });
    end.append(&trust);
    let forget = icon_button("delete", "Remove this device", "");
    let (path, adapter, name) = (d.path.clone(), a.path.clone(), d.name.clone());
    forget.connect_clicked(move |b| {
        let (path, adapter) = (path.clone(), adapter.clone());
        let name2 = name.clone();
        ui::ask(ui::root(b).as_ref(), &format!("Remove {name}?"), "You will need to pair it again to use it.", "Cancel", "Remove", "danger", move || {
            glib::spawn_future_local(async move {
                let Ok(conn) = kit::pk::system().await else { return };
                match bluez::remove(&conn, &adapter, &path).await {
                    Ok(()) => ui::done(&format!("{name2} removed")),
                    Err(e) => ui::fail(&format!("Couldn't remove {name2}: {e}")),
                }
            });
        });
    });
    end.append(&forget);
    let go = ui::button(if d.connected { "Disconnect" } else { "Connect" }, if d.connected { "" } else { "accent" });
    let (path, name, on) = (d.path.clone(), d.name.clone(), !d.connected);
    let weak = Rc::downgrade(page);
    go.connect_clicked(move |_| {
        let Some(p) = weak.upgrade() else { return };
        let path2 = path.clone();
        let (doing, did) = if on { (format!("Connecting to {name}…"), format!("{name} connected")) } else { (format!("Disconnecting {name}…"), format!("{name} disconnected")) };
        act(&p, &path, doing, did, async move {
            let conn = kit::pk::system().await?;
            bluez::connect(&conn, &path2, on).await
        });
    });
    end.append(&go);
    row
}

fn act(page: &Rc<Page>, path: &str, doing: String, did: String, work: impl std::future::Future<Output = Result<(), String>> + 'static) {
    page.busy.borrow_mut().push(path.to_string());
    page.snap.borrow_mut().take();
    refresh(page);
    ui::busy(&doing, None);
    let weak = Rc::downgrade(page);
    let path = path.to_string();
    glib::spawn_future_local(async move {
        let r = work.await;
        match r {
            Ok(()) => ui::done(&did),
            Err(e) => ui::fail(&format!("{}: {e}", doing.trim_end_matches('…'))),
        }
        if let Some(p) = weak.upgrade() {
            p.busy.borrow_mut().retain(|b| *b != path);
            p.snap.borrow_mut().take();
            refresh(&p);
        }
    });
}

fn stop_scan(page: &Rc<Page>) {
    if let Some(path) = page.scanning.borrow_mut().take() {
        glib::spawn_future_local(async move {
            if let Ok(conn) = kit::pk::system().await {
                let _ = bluez::scan(&conn, &path, false).await;
            }
        });
    }
}

fn toggle_scan(page: &Rc<Page>) {
    let Some(a) = chosen(page) else { return };
    if a.discovering {
        let path = a.path.clone();
        page.scanning.borrow_mut().take();
        glib::spawn_future_local(async move {
            if let Ok(conn) = kit::pk::system().await {
                let _ = bluez::scan(&conn, &path, false).await;
            }
        });
        return;
    }
    *page.scanning.borrow_mut() = Some(a.path.clone());
    let round = page.round.get() + 1;
    page.round.set(round);
    let path = a.path.clone();
    glib::spawn_future_local(async move {
        let Ok(conn) = kit::pk::system().await else { return };
        if let Err(e) = bluez::scan(&conn, &path, true).await {
            ui::fail(&format!("Couldn't scan: {e}"));
        }
    });
    let weak = Rc::downgrade(page);
    glib::timeout_add_local_once(Duration::from_secs(60), move || {
        if let Some(p) = weak.upgrade() {
            if p.round.get() == round {
                stop_scan(&p);
            }
        }
    });
}

fn parent() -> Option<gtk::Window> {
    gtk::gio::Application::default().and_downcast::<gtk::Application>().and_then(|a| a.active_window())
}

/// Answers BlueZ's pairing questions for the whole app, with a dialog for each.
fn agent() {
    if AGENT.with(|a| a.replace(true)) {
        return;
    }
    glib::spawn_future_local(async move {
        let Ok(conn) = kit::pk::system().await else { return };
        let Some(rx) = bluez::agent(&conn).await else {
            AGENT.with(|a| a.set(false));
            return;
        };
        while let Ok(req) = rx.recv().await {
            let reply = req.reply.clone();
            let answer = move |v: Option<String>| {
                let _ = reply.try_send(v);
            };
            let dialog = match req.ask {
                Ask::Cancel => {
                    if let Some(d) = DIALOG.with(|d| d.borrow_mut().take()) {
                        d.close();
                    }
                    continue;
                }
                Ask::Confirm(name, code) if code.is_empty() => ui::prompt(parent(), &format!("Allow {name} to pair?"), "", None, "Allow", answer),
                Ask::Confirm(name, code) => ui::prompt(parent(), &format!("Pair with {name}?"), &format!("Check that {name} shows the code {code}, then confirm on both."), None, "Pair", answer),
                Ask::Pin(name) => ui::prompt(parent(), &format!("PIN for {name}"), "Type the PIN the device expects. Many use 0000 or 1234.", Some(("PIN", false)), "Pair", answer),
                Ask::Passkey(name) => ui::prompt(parent(), &format!("Passkey for {name}"), "Type the six-digit code the device shows.", Some(("Passkey", false)), "Pair", answer),
                Ask::Show(name, code) => ui::prompt(parent(), &format!("Type {code} on {name}"), "Then press Enter on the device to finish pairing.", None, "", answer),
            };
            DIALOG.with(|d| *d.borrow_mut() = Some(dialog));
        }
        AGENT.with(|a| a.set(false));
    });
}
