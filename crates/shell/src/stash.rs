use gtk::gdk_pixbuf::{InterpType, Pixbuf};
use gtk::{gdk, glib, prelude::*};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const BUDGET: u64 = 320 << 20;
const KEEP: u64 = 240 << 20;

static SEQ: AtomicU64 = AtomicU64::new(0);

pub fn dir() -> PathBuf {
    glib::user_cache_dir().join("digit").join("stash")
}

fn file(path: &str, tag: &str) -> Option<PathBuf> {
    let meta = std::fs::metadata(path).ok()?;
    let stamp = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos();
    let key = format!("v2\0{path}\0{stamp}\0{}\0{tag}", meta.len());
    let name = glib::compute_checksum_for_string(glib::ChecksumType::Sha256, &key)?;
    Some(dir().join(format!("{name}.png")))
}

pub fn has(path: &str, tag: &str) -> bool {
    file(path, tag).is_some_and(|f| f.is_file())
}

pub fn get(path: &str, tag: &str) -> Option<gdk::Texture> {
    let f = file(path, tag)?;
    if !f.is_file() {
        return None;
    }
    gdk::Texture::from_filename(&f).ok()
}

pub fn put(path: &str, tag: &str, texture: gdk::Texture) {
    let Some(f) = file(path, tag) else { return };
    let _ = std::thread::Builder::new().name("digit-stash".into()).spawn(move || {
        use std::os::unix::fs::DirBuilderExt;
        if std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir()).is_err() {
            return;
        }
        let tmp = f.with_extension(format!("{}.part", SEQ.fetch_add(1, Ordering::Relaxed)));
        if texture.save_to_png(&tmp).is_ok() {
            let _ = std::fs::rename(&tmp, &f);
        } else {
            let _ = std::fs::remove_file(&tmp);
        }
    });
}

pub fn master(path: &str, need: &[(i32, i32)]) -> Option<Pixbuf> {
    let size = imagesize::size(path).ok()?;
    let (iw, ih) = (size.width.max(1) as f64, size.height.max(1) as f64);
    let k = need.iter().map(|&(w, h)| (w as f64 / iw).max(h as f64 / ih)).fold(0.0f64, f64::max).max(1e-3);
    let long = (iw.max(ih) * k).ceil().clamp(16.0, 8192.0) as i32;
    Pixbuf::from_file_at_scale(path, long, long, true).ok()
}

pub fn cover(pb: &Pixbuf, w: i32, h: i32) -> Option<Pixbuf> {
    let (pw, ph) = (pb.width().max(1) as f64, pb.height().max(1) as f64);
    let k = (w as f64 / pw).max(h as f64 / ph);
    let (cw, ch) = (((pw * k).ceil() as i32).max(w), ((ph * k).ceil() as i32).max(h));
    let scaled = if cw == pb.width() && ch == pb.height() { pb.clone() } else { pb.scale_simple(cw, ch, InterpType::Bilinear)? };
    scaled.new_subpixbuf((cw - w) / 2, (ch - h) / 2, w, h).copy()
}

pub fn texture(pb: &Pixbuf) -> gdk::Texture {
    let format = if pb.has_alpha() { gdk::MemoryFormat::R8g8b8a8 } else { gdk::MemoryFormat::R8g8b8 };
    gdk::MemoryTexture::new(pb.width(), pb.height(), format, &pb.read_pixel_bytes(), pb.rowstride() as usize).upcast()
}

pub fn prune() {
    let Ok(read) = std::fs::read_dir(dir()) else { return };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = read
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            m.is_file().then(|| Some((m.modified().ok()?, m.len(), e.path())))?
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    if total <= BUDGET {
        return;
    }
    files.sort();
    for (_, len, p) in files {
        if total <= KEEP {
            break;
        }
        if std::fs::remove_file(&p).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}
