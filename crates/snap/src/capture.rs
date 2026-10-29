use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use wayland_client::protocol::{wl_buffer, wl_output, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols::xdg::xdg_output::zv1::client::{zxdg_output_manager_v1 as xmgr, zxdg_output_v1 as xout};
use wayland_protocols_wlr::screencopy::v1::client::{zwlr_screencopy_frame_v1 as frame, zwlr_screencopy_manager_v1 as mgr};

pub struct Shot {
    pub name: String,
    pub w: i32,
    pub h: i32,
    pub pw: i32,
    pub ph: i32,
    pub stride: i32,
    pub data: Vec<u8>,
}

#[derive(Default)]
struct Out {
    wl: Option<wl_output::WlOutput>,
    name: String,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    mode: (i32, i32),
    scale: i32,
}

#[derive(Default, Clone)]
struct Frame {
    format: Option<(wl_shm::Format, u32, u32, u32)>,
    listed: bool,
    yinv: bool,
    ready: bool,
    failed: bool,
}

#[derive(Default)]
struct St {
    shm: Option<wl_shm::WlShm>,
    mgr: Option<mgr::ZwlrScreencopyManagerV1>,
    xdg: Option<xmgr::ZxdgOutputManagerV1>,
    outs: Vec<Out>,
    frames: Vec<Frame>,
}

fn usable(f: wl_shm::Format) -> bool {
    matches!(f, wl_shm::Format::Xrgb8888 | wl_shm::Format::Argb8888 | wl_shm::Format::Xbgr8888 | wl_shm::Format::Abgr8888)
}

impl Dispatch<wl_registry::WlRegistry, ()> for St {
    fn event(s: &mut Self, r: &wl_registry::WlRegistry, e: wl_registry::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_registry::Event::Global { name, interface, version } = e {
            match interface.as_str() {
                "wl_shm" => s.shm = Some(r.bind(name, 1, qh, ())),
                "zwlr_screencopy_manager_v1" => s.mgr = Some(r.bind(name, version.min(3), qh, ())),
                "zxdg_output_manager_v1" => s.xdg = Some(r.bind(name, version.min(3), qh, ())),
                "wl_output" => {
                    let i = s.outs.len();
                    s.outs.push(Out { wl: Some(r.bind(name, version.min(4), qh, i)), scale: 1, ..Default::default() });
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_output::WlOutput, usize> for St {
    fn event(s: &mut Self, _: &wl_output::WlOutput, e: wl_output::Event, i: &usize, _: &Connection, _: &QueueHandle<Self>) {
        let Some(o) = s.outs.get_mut(*i) else { return };
        match e {
            wl_output::Event::Mode { flags, width, height, .. } => {
                if let WEnum::Value(f) = flags {
                    if f.contains(wl_output::Mode::Current) {
                        o.mode = (width, height);
                    }
                }
            }
            wl_output::Event::Scale { factor } => o.scale = factor.max(1),
            wl_output::Event::Name { name } => o.name = name,
            _ => {}
        }
    }
}

impl Dispatch<xout::ZxdgOutputV1, usize> for St {
    fn event(s: &mut Self, _: &xout::ZxdgOutputV1, e: xout::Event, i: &usize, _: &Connection, _: &QueueHandle<Self>) {
        let Some(o) = s.outs.get_mut(*i) else { return };
        match e {
            xout::Event::LogicalPosition { x, y } => {
                o.x = x;
                o.y = y;
            }
            xout::Event::LogicalSize { width, height } => {
                o.w = width;
                o.h = height;
            }
            xout::Event::Name { name } => {
                if o.name.is_empty() {
                    o.name = name;
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<frame::ZwlrScreencopyFrameV1, usize> for St {
    fn event(s: &mut Self, _: &frame::ZwlrScreencopyFrameV1, e: frame::Event, i: &usize, _: &Connection, _: &QueueHandle<Self>) {
        let Some(f) = s.frames.get_mut(*i) else { return };
        match e {
            frame::Event::Buffer { format: WEnum::Value(fmt), width, height, stride } if usable(fmt) => {
                if f.format.is_none() || fmt == wl_shm::Format::Xrgb8888 {
                    f.format = Some((fmt, width, height, stride));
                }
            }
            frame::Event::BufferDone => f.listed = true,
            frame::Event::Flags { flags: WEnum::Value(fl) } => f.yinv = fl.contains(frame::Flags::YInvert),
            frame::Event::Ready { .. } => f.ready = true,
            frame::Event::Failed => f.failed = true,
            _ => {}
        }
    }
}

macro_rules! ignore {
    ($t:ty, $u:ty) => {
        impl Dispatch<$t, $u> for St {
            fn event(_: &mut Self, _: &$t, _: <$t as wayland_client::Proxy>::Event, _: &$u, _: &Connection, _: &QueueHandle<Self>) {}
        }
    };
}
ignore!(wl_shm::WlShm, ());
ignore!(wl_shm_pool::WlShmPool, ());
ignore!(wl_buffer::WlBuffer, ());
ignore!(mgr::ZwlrScreencopyManagerV1, ());
ignore!(xmgr::ZxdgOutputManagerV1, ());

struct Map {
    ptr: *mut libc::c_void,
    len: usize,
    fd: OwnedFd,
}

impl Map {
    fn new(len: usize) -> Option<Map> {
        unsafe {
            let raw = libc::memfd_create(c"digit-snap".as_ptr(), libc::MFD_CLOEXEC);
            if raw < 0 {
                return None;
            }
            let fd = OwnedFd::from_raw_fd(raw);
            if libc::ftruncate(raw, len as libc::off_t) != 0 {
                return None;
            }
            let ptr = libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, raw, 0);
            if ptr == libc::MAP_FAILED {
                return None;
            }
            Some(Map { ptr, len, fd })
        }
    }

    fn bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr as *const u8, self.len) }
    }
}

impl Drop for Map {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.ptr, self.len);
        }
    }
}

pub fn grab(cursor: bool) -> Result<Vec<Shot>, String> {
    let conn = Connection::connect_to_env().map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let _registry = conn.display().get_registry(&qh, ());
    let mut st = St::default();
    queue.roundtrip(&mut st).map_err(|e| e.to_string())?;
    queue.roundtrip(&mut st).map_err(|e| e.to_string())?;
    let (Some(shm), Some(m)) = (st.shm.clone(), st.mgr.clone()) else { return Err("no screencopy".into()) };
    if let Some(x) = st.xdg.clone() {
        let outs: Vec<wl_output::WlOutput> = st.outs.iter().filter_map(|o| o.wl.clone()).collect();
        let _xouts: Vec<xout::ZxdgOutputV1> = outs.iter().enumerate().map(|(i, o)| x.get_xdg_output(o, &qh, i)).collect();
        queue.roundtrip(&mut st).map_err(|e| e.to_string())?;
    }
    for o in st.outs.iter_mut() {
        if o.w == 0 || o.h == 0 {
            o.w = o.mode.0 / o.scale.max(1);
            o.h = o.mode.1 / o.scale.max(1);
        }
    }
    st.frames = vec![Frame::default(); st.outs.len()];
    let frames: Vec<frame::ZwlrScreencopyFrameV1> = st.outs.iter().enumerate().filter_map(|(i, o)| Some(m.capture_output(cursor as i32, o.wl.as_ref()?, &qh, i))).collect();
    let started = std::time::Instant::now();
    while st.frames.iter().any(|f| f.format.is_none() && !f.failed && !f.listed) {
        queue.roundtrip(&mut st).map_err(|e| e.to_string())?;
        if started.elapsed().as_secs() > 2 {
            break;
        }
    }
    let mut maps: Vec<Option<(Map, wl_buffer::WlBuffer, wl_shm_pool::WlShmPool)>> = Vec::new();
    for (i, f) in frames.iter().enumerate() {
        let Some((fmt, w, h, stride)) = st.frames[i].format else {
            maps.push(None);
            continue;
        };
        let len = (stride * h) as usize;
        let Some(map) = Map::new(len) else {
            maps.push(None);
            continue;
        };
        let pool = shm.create_pool(map.fd.as_fd(), len as i32, &qh, ());
        let buffer = pool.create_buffer(0, w as i32, h as i32, stride as i32, fmt, &qh, ());
        f.copy(&buffer);
        maps.push(Some((map, buffer, pool)));
    }
    while st.frames.iter().enumerate().any(|(i, f)| maps[i].is_some() && !f.ready && !f.failed) {
        queue.blocking_dispatch(&mut st).map_err(|e| e.to_string())?;
        if started.elapsed().as_secs() > 4 {
            return Err("capture timed out".into());
        }
    }
    let mut shots = Vec::new();
    for (i, slot) in maps.into_iter().enumerate() {
        let Some((map, buffer, pool)) = slot else { continue };
        let f = &st.frames[i];
        let o = &st.outs[i];
        if f.ready {
            let (fmt, w, h, stride) = f.format.unwrap_or((wl_shm::Format::Xrgb8888, 0, 0, 0));
            let mut data = map.bytes().to_vec();
            if f.yinv {
                let s = stride as usize;
                let rows: Vec<Vec<u8>> = data.chunks(s).map(|r| r.to_vec()).collect();
                data = rows.into_iter().rev().flatten().collect();
            }
            if matches!(fmt, wl_shm::Format::Xbgr8888 | wl_shm::Format::Abgr8888) {
                for px in data.chunks_exact_mut(4) {
                    px.swap(0, 2);
                }
            }
            if matches!(fmt, wl_shm::Format::Xrgb8888 | wl_shm::Format::Xbgr8888) {
                for px in data.chunks_exact_mut(4) {
                    px[3] = 255;
                }
            }
            shots.push(Shot { name: o.name.clone(), w: o.w, h: o.h, pw: w as i32, ph: h as i32, stride: stride as i32, data });
        }
        buffer.destroy();
        pool.destroy();
    }
    for f in frames {
        f.destroy();
    }
    let _ = conn.flush();
    if shots.is_empty() {
        return Err("nothing captured".into());
    }
    Ok(shots)
}

pub async fn portal() -> Result<Vec<Shot>, String> {
    use gtk::{gio, glib, prelude::*};
    let conn = gio::bus_get_future(gio::BusType::Session).await.map_err(|e| e.to_string())?;
    let token = format!("digit{}", glib::random_int());
    let sender = conn.unique_name().map(|n| n.trim_start_matches(':').replace('.', "_")).unwrap_or_default();
    let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
    let (tx, rx) = futures_channel();
    let tx = std::cell::RefCell::new(Some(tx));
    let sub = conn.subscribe_to_signal(Some("org.freedesktop.portal.Desktop"), Some("org.freedesktop.portal.Request"), Some("Response"), Some(&path), None, gio::DBusSignalFlags::NONE, move |sig| {
        if let Some(t) = tx.borrow_mut().take() {
            let _ = t.send(sig.parameters.clone());
        }
    });
    let opts = glib::VariantDict::new(None);
    opts.insert("handle_token", &token);
    opts.insert("interactive", false);
    let args = ("", opts.end()).to_variant();
    let r = conn.call_future(Some("org.freedesktop.portal.Desktop"), "/org/freedesktop/portal/desktop", "org.freedesktop.portal.Screenshot", "Screenshot", Some(&args), None, gio::DBusCallFlags::NONE, 30000).await;
    if let Err(e) = r {
        drop(sub);
        return Err(e.to_string());
    }
    let params = rx.await.map_err(|_| "portal closed".to_string())?;
    drop(sub);
    let code = params.child_value(0).get::<u32>().unwrap_or(2);
    if code != 0 {
        return Err("cancelled".into());
    }
    let results = params.child_value(1);
    let uri = glib::VariantDict::new(Some(&results)).lookup::<String>("uri").ok().flatten().ok_or("no image")?;
    let file = gio::File::for_uri(&uri);
    let path = file.path().ok_or("not a local file")?;
    let pb = gtk::gdk_pixbuf::Pixbuf::from_file(&path).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&path);
    let (w, h) = (pb.width(), pb.height());
    let n = pb.n_channels() as usize;
    let rs = pb.rowstride() as usize;
    let bytes = pb.read_pixel_bytes();
    let mut data = vec![0u8; (w * h * 4) as usize];
    for y in 0..h as usize {
        for x in 0..w as usize {
            let s = y * rs + x * n;
            let d = (y * w as usize + x) * 4;
            data[d] = bytes[s + 2];
            data[d + 1] = bytes[s + 1];
            data[d + 2] = bytes[s];
            data[d + 3] = 255;
        }
    }
    let monitor = gtk::gdk::Display::default().and_then(|d| d.monitors().item(0)).and_downcast::<gtk::gdk::Monitor>();
    let (name, lw, lh) = monitor.map(|m| (m.connector().map(|c| c.to_string()).unwrap_or_default(), m.geometry().width(), m.geometry().height())).unwrap_or_default();
    let (lw, lh) = if lw > 0 { (lw, lh) } else { (w, h) };
    Ok(vec![Shot { name, w: lw, h: lh, pw: w, ph: h, stride: w * 4, data }])
}

fn futures_channel() -> (Sender, Receiver) {
    let slot = std::rc::Rc::new(std::cell::RefCell::new((None::<gtk::glib::Variant>, None::<std::task::Waker>)));
    (Sender(slot.clone()), Receiver(slot))
}

struct Sender(std::rc::Rc<std::cell::RefCell<(Option<gtk::glib::Variant>, Option<std::task::Waker>)>>);
struct Receiver(std::rc::Rc<std::cell::RefCell<(Option<gtk::glib::Variant>, Option<std::task::Waker>)>>);

impl Sender {
    fn send(self, v: gtk::glib::Variant) -> Result<(), ()> {
        let mut s = self.0.borrow_mut();
        s.0 = Some(v);
        if let Some(w) = s.1.take() {
            w.wake();
        }
        Ok(())
    }
}

impl std::future::Future for Receiver {
    type Output = Result<gtk::glib::Variant, ()>;
    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
        let mut s = self.0.borrow_mut();
        match s.0.take() {
            Some(v) => std::task::Poll::Ready(Ok(v)),
            None => {
                s.1 = Some(cx.waker().clone());
                std::task::Poll::Pending
            }
        }
    }
}
