use crate::app::App;
use gtk::{gdk, glib};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::rc::Rc;
use std::time::{Duration, Instant};
use wayland_client::backend::ObjectId;
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, event_created_child};
use wayland_protocols::ext::data_control::v1::client::{ext_data_control_device_v1 as dev, ext_data_control_manager_v1 as mgr, ext_data_control_offer_v1 as offer};

const KEEP: usize = 60;
const IMAGES: usize = 8;
const TEXT_MAX: usize = 512 * 1024;
const IMAGE_MAX: usize = 24 * 1024 * 1024;
const TEXT: [&str; 4] = ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain", "STRING"];

pub enum Data {
    Text(String),
    Image(Vec<u8>),
}

pub struct Clip {
    pub id: u64,
    pub data: Data,
    pub at: i64,
    texture: RefCell<Option<(i32, gdk::Texture)>>,
}

impl Clip {
    pub fn preview(&self, scale: i32) -> Option<gdk::Texture> {
        let Data::Image(png) = &self.data else { return None };
        let px = 60 * scale;
        let cached = self.texture.borrow().clone();
        if let Some((_, t)) = cached.filter(|(n, _)| *n == px) { return Some(t); }
        let t = crate::images::decode(png, px);
        *self.texture.borrow_mut() = t.clone().map(|t| (px, t));
        t
    }

    pub fn dimensions(&self) -> Option<(usize, usize)> {
        let Data::Image(png) = &self.data else { return None };
        let size = imagesize::blob_size(png).ok()?;
        Some((size.width, size.height))
    }

    pub fn original(&self) -> Option<gdk::Texture> {
        let Data::Image(png) = &self.data else { return None };
        gdk::Texture::from_bytes(&glib::Bytes::from(png)).ok()
    }

}

thread_local! {
    static HISTORY: RefCell<VecDeque<Rc<Clip>>> = const { RefCell::new(VecDeque::new()) };
    static NEXT: Cell<u64> = const { Cell::new(1) };
}

pub fn history() -> Vec<Rc<Clip>> {
    HISTORY.with(|h| h.borrow().iter().cloned().collect())
}

pub fn clear() {
    HISTORY.with(|h| h.borrow_mut().clear());
}

fn push(data: Data) {
    HISTORY.with(|h| {
        let mut h = h.borrow_mut();
        if let Data::Text(t) = &data {
            if let Some(i) = h.iter().position(|c| matches!(&c.data, Data::Text(x) if x == t)) {
                if let Some(c) = h.remove(i) {
                    h.push_front(Rc::new(Clip { id: c.id, data, at: glib::real_time(), texture: RefCell::new(None) }));
                }
                return;
            }
        }
        if let Data::Image(png) = &data {
            if h.iter().any(|c| matches!(&c.data, Data::Image(x) if x == png)) {
                return;
            }
        }
        let id = NEXT.with(|n| {
            let v = n.get();
            n.set(v + 1);
            v
        });
        h.push_front(Rc::new(Clip { id, data, at: glib::real_time(), texture: RefCell::new(None) }));
        let mut images = 0;
        h.retain(|c| {
            if matches!(c.data, Data::Image(_)) {
                images += 1;
                images <= IMAGES
            } else {
                true
            }
        });
        h.truncate(KEEP);
    });
}

pub fn start(app: &Rc<App>) {
    let (tx, rx) = async_channel::bounded::<Data>(16);
    let spawned = std::thread::Builder::new().name("digit-clip".into()).spawn(move || {
        let mut delay = 2u64;
        loop {
            let started = Instant::now();
            let _ = watch(&tx);
            if tx.is_closed() {
                return;
            }
            if started.elapsed() > Duration::from_secs(60) {
                delay = 2;
            }
            std::thread::sleep(Duration::from_secs(delay));
            delay = (delay * 2).min(60);
        }
    });
    if spawned.is_err() {
        return;
    }
    let weak = Rc::downgrade(app);
    glib::spawn_future_local(async move {
        while let Ok(data) = rx.recv().await {
            push(data);
            if let Some(app) = weak.upgrade() {
                crate::launcher::clips_changed(&app);
            }
        }
    });
}

#[derive(Default)]
struct Watch {
    seat: Option<wl_seat::WlSeat>,
    mgr: Option<mgr::ExtDataControlManagerV1>,
    mimes: HashMap<ObjectId, Vec<String>>,
    pending: Option<offer::ExtDataControlOfferV1>,
    gone: bool,
}

impl Dispatch<wl_registry::WlRegistry, ()> for Watch {
    fn event(w: &mut Self, r: &wl_registry::WlRegistry, e: wl_registry::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_registry::Event::Global { name, interface, version } = e {
            match interface.as_str() {
                "wl_seat" if w.seat.is_none() => w.seat = Some(r.bind(name, version.min(7), qh, ())),
                "ext_data_control_manager_v1" => w.mgr = Some(r.bind(name, 1, qh, ())),
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for Watch {
    fn event(_: &mut Self, _: &wl_seat::WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<mgr::ExtDataControlManagerV1, ()> for Watch {
    fn event(_: &mut Self, _: &mgr::ExtDataControlManagerV1, _: mgr::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<dev::ExtDataControlDeviceV1, ()> for Watch {
    fn event(w: &mut Self, _: &dev::ExtDataControlDeviceV1, e: dev::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match e {
            dev::Event::DataOffer { id } => {
                w.mimes.insert(id.id(), Vec::new());
            }
            dev::Event::Selection { id } => {
                if let Some(old) = w.pending.take() {
                    w.mimes.remove(&old.id());
                    old.destroy();
                }
                w.pending = id;
            }
            dev::Event::PrimarySelection { id: Some(o) } => {
                w.mimes.remove(&o.id());
                o.destroy();
            }
            dev::Event::Finished => w.gone = true,
            _ => {}
        }
    }

    event_created_child!(Watch, dev::ExtDataControlDeviceV1, [dev::EVT_DATA_OFFER_OPCODE => (offer::ExtDataControlOfferV1, ())]);
}

impl Dispatch<offer::ExtDataControlOfferV1, ()> for Watch {
    fn event(w: &mut Self, o: &offer::ExtDataControlOfferV1, e: offer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let offer::Event::Offer { mime_type } = e {
            w.mimes.entry(o.id()).or_default().push(mime_type);
        }
    }
}

fn watch(tx: &async_channel::Sender<Data>) -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let _registry = conn.display().get_registry(&qh, ());
    let mut w = Watch::default();
    queue.roundtrip(&mut w).map_err(|e| e.to_string())?;
    let (Some(seat), Some(m)) = (w.seat.clone(), w.mgr.clone()) else { return Err("no data control".into()) };
    let device = m.get_data_device(&seat, &qh, ());
    loop {
        queue.blocking_dispatch(&mut w).map_err(|e| e.to_string())?;
        if w.gone || tx.is_closed() {
            device.destroy();
            return Ok(());
        }
        if let Some(o) = w.pending.take() {
            let mimes = w.mimes.remove(&o.id()).unwrap_or_default();
            if let Some(data) = take(&conn, &o, &mimes) {
                if tx.send_blocking(data).is_err() {
                    o.destroy();
                    return Ok(());
                }
            }
            o.destroy();
            let _ = conn.flush();
        }
    }
}

fn take(conn: &Connection, o: &offer::ExtDataControlOfferV1, mimes: &[String]) -> Option<Data> {
    if mimes.iter().any(|m| m == "x-kde-passwordManagerHint") {
        return None;
    }
    let (mime, image) = match TEXT.iter().find(|t| mimes.iter().any(|m| m == *t)) {
        Some(t) => (t.to_string(), false),
        None => (mimes.iter().find(|m| *m == "image/png")?.clone(), true),
    };
    let mut fds = [0; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return None;
    }
    let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    o.receive(mime, write.as_fd());
    conn.flush().ok()?;
    drop(write);
    let limit = if image { IMAGE_MAX } else { TEXT_MAX };
    let bytes = slurp(read, limit)?;
    if bytes.is_empty() {
        return None;
    }
    if image {
        return bytes.starts_with(b"\x89PNG").then_some(Data::Image(bytes));
    }
    let text = String::from_utf8(bytes).ok()?;
    (!text.trim().is_empty()).then_some(Data::Text(text))
}

fn slurp(fd: OwnedFd, limit: usize) -> Option<Vec<u8>> {
    let raw = fd.as_raw_fd();
    unsafe {
        let flags = libc::fcntl(raw, libc::F_GETFL);
        libc::fcntl(raw, libc::F_SETFL, flags | libc::O_NONBLOCK);
    }
    let mut file = std::fs::File::from(fd);
    let mut out = Vec::new();
    let mut buf = [0u8; 65536];
    let deadline = Instant::now() + Duration::from_millis(1500);
    loop {
        match file.read(&mut buf) {
            Ok(0) => return Some(out),
            Ok(n) => {
                out.extend_from_slice(&buf[..n]);
                if out.len() > limit {
                    return None;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return None;
                }
                let mut p = libc::pollfd { fd: raw, events: libc::POLLIN, revents: 0 };
                unsafe { libc::poll(&mut p, 1, left.as_millis().min(i32::MAX as u128) as i32) };
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
}

#[cfg(test)]
mod preview_tests {
    use super::*;
    use gtk::prelude::*;

    #[test]
    fn previews_are_scale_aware_and_copy_keeps_original_pixels() {
        let pb = gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, 800, 400).unwrap();
        pb.fill(0x123456ff);
        let bytes = pb.save_to_bufferv("png", &[]).unwrap();
        let clip = Clip { id: 1, data: Data::Image(bytes.clone()), at: 0, texture: RefCell::new(None) };
        let preview = clip.preview(1).unwrap();
        assert_eq!((preview.width(), preview.height()), (60, 30));
        let preview = clip.preview(2).unwrap();
        assert_eq!((preview.width(), preview.height()), (120, 60));
        let original = clip.original().unwrap();
        assert_eq!((original.width(), original.height()), (800, 400));
        assert_eq!(clip.dimensions(), Some((800, 400)));
        assert!(matches!(clip.data, Data::Image(ref png) if *png == bytes));
    }
}
