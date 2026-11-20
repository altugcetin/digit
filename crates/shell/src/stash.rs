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
    use std::sync::{OnceLock, mpsc::{SyncSender, sync_channel}};
    static WRITER: OnceLock<SyncSender<(PathBuf, gdk::Texture)>> = OnceLock::new();
    let sender = WRITER.get_or_init(|| {
        let (tx, rx) = sync_channel::<(PathBuf, gdk::Texture)>(2);
        let _ = std::thread::Builder::new().name("digit-stash".into()).spawn(move || {
            use std::os::unix::fs::DirBuilderExt;
            for (f, texture) in rx {
                if std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir()).is_err() { continue; }
                let tmp = f.with_extension(format!("{}.part", SEQ.fetch_add(1, Ordering::Relaxed)));
                if texture.save_to_png(&tmp).is_ok() {
                    let _ = std::fs::rename(&tmp, &f);
                } else {
                    let _ = std::fs::remove_file(&tmp);
                }
            }
        });
        tx
    });
    let Some(f) = file(path, tag) else { return };
    let _ = sender.try_send((f, texture));
}

pub fn master(path: &str, need: &[(i32, i32)]) -> Option<Pixbuf> {
    let size = imagesize::size(path).ok()?;
    let (iw, ih) = (size.width.max(1) as f64, size.height.max(1) as f64);
    let k = need.iter().map(|&(w, h)| (w as f64 / iw).max(h as f64 / ih)).fold(0.0f64, f64::max).max(1e-3);
    let long = (iw.max(ih) * k).ceil().clamp(16.0, 8192.0) as i32;
    Pixbuf::from_file_at_scale(path, long, long, true).ok()
}

pub fn cover(pb: &Pixbuf, w: i32, h: i32) -> Option<Pixbuf> {
    cover_at(pb, w, h, kit::wallfocus::CENTER)
}

/// As [`cover`], keeping the part of the image `focus` points at (see `kit::wallfocus`).
pub fn cover_at(pb: &Pixbuf, w: i32, h: i32, focus: kit::wallfocus::Focus) -> Option<Pixbuf> {
    if w < 1 || h < 1 { return None; }
    let (x, y, cw, ch) = crop_at(pb.width(), pb.height(), w, h, focus);
    pb.new_subpixbuf(x, y, cw, ch).scale_simple(w, h, InterpType::Bilinear)
}

#[cfg(test)]
fn crop_size(pw: i32, ph: i32, w: i32, h: i32) -> (i32, i32, i32, i32) {
    crop_at(pw, ph, w, h, kit::wallfocus::CENTER)
}

fn crop_at(pw: i32, ph: i32, w: i32, h: i32, focus: kit::wallfocus::Focus) -> (i32, i32, i32, i32) {
    let focus = kit::wallfocus::clamp(focus);
    // The part that covers the screen, and of that, zoomed in, a smaller part in the same shape.
    let cw = ((ph as f64 * w as f64 / h as f64).round() as i32).clamp(1, pw);
    let ch = ((pw as f64 * h as f64 / w as f64).round() as i32).clamp(1, ph);
    let (cw, ch) = (((cw as f64 / focus.2).round() as i32).max(1), ((ch as f64 / focus.2).round() as i32).max(1));
    (((pw - cw) as f64 * focus.0).floor() as i32, ((ph - ch) as f64 * focus.1).floor() as i32, cw, ch)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_retains_only_target_pixels() {
        let pb = Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, 128, 128).unwrap();
        pb.fill(0xffffffff);
        let cropped = cover(&pb, 128, 32).unwrap();
        assert_eq!((cropped.width(), cropped.height()), (128, 32));
        assert_eq!(cropped.read_pixel_bytes().len(), 128 * 32 * 4);
        assert!(cover(&pb, 0, 32).is_none());
    }

    #[test]
    fn cover_crops_extreme_sources_before_scaling() {
        assert_eq!(crop_size(82, 8192, 1920, 1080), (0, 4073, 82, 46));
        assert_eq!(crop_size(8192, 82, 1080, 1920), (4073, 0, 46, 82));
        assert_eq!(crop_size(7680, 7680, 7680, 2160), (0, 2760, 7680, 2160));
    }

    #[test]
    fn cover_keeps_the_part_the_focus_points_at() {
        // A square image on a wide screen overflows up and down: 0 keeps the top, 1 the bottom.
        assert_eq!(crop_at(1000, 1000, 1000, 500, (0.5, 0.0, 1.0)), (0, 0, 1000, 500));
        assert_eq!(crop_at(1000, 1000, 1000, 500, (0.5, 1.0, 1.0)), (0, 500, 1000, 500));
        assert_eq!(crop_at(1000, 1000, 1000, 500, (0.5, 0.5, 1.0)), (0, 250, 1000, 500));
        // Nothing overflows sideways, so the horizontal focus changes nothing.
        assert_eq!(crop_at(1000, 1000, 1000, 500, (0.0, 0.5, 1.0)), crop_at(1000, 1000, 1000, 500, (1.0, 0.5, 1.0)));
        // Zoomed in twice: half of it each way, moved along both by the focus.
        assert_eq!(crop_at(1000, 1000, 1000, 500, (0.5, 0.5, 2.0)), (250, 375, 500, 250));
        assert_eq!(crop_at(1000, 1000, 1000, 500, (1.0, 0.0, 2.0)), (500, 0, 500, 250));
    }
}
