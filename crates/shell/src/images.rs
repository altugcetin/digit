use gtk::{gdk, prelude::*};

pub fn scale() -> i32 {
    gdk::Display::default().map(|d| d.monitors().iter::<gdk::Monitor>().filter_map(Result::ok).map(|m| m.scale_factor()).max().unwrap_or(1)).unwrap_or(1).max(1)
}

pub fn fit(w: i32, h: i32, px: i32) -> Option<(i32, i32)> {
    if w <= 0 || h <= 0 || px <= 0 {
        return None;
    }
    let k = (f64::from(px) / f64::from(w.max(h))).min(1.0);
    Some(((f64::from(w) * k).round().max(1.0) as i32, (f64::from(h) * k).round().max(1.0) as i32))
}

pub fn decode(bytes: &[u8], px: i32) -> Option<gdk::Texture> {
    let loader = gtk::gdk_pixbuf::PixbufLoader::new();
    loader.connect_size_prepared(move |l, w, h| {
        if let Some((w, h)) = fit(w, h, px) {
            l.set_size(w, h);
        }
    });
    let written = loader.write(bytes).is_ok();
    let closed = loader.close().is_ok();
    if !written || !closed {
        return None;
    }
    let pb = loader.pixbuf()?;
    let (w, h) = fit(pb.width(), pb.height(), px)?;
    let pb = if (w, h) != (pb.width(), pb.height()) { pb.scale_simple(w, h, gtk::gdk_pixbuf::InterpType::Bilinear)? } else { pb };
    Some(gdk::Texture::for_pixbuf(&pb))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoding_large_images_retains_only_a_scaled_texture() {
        let pb = gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, 1000, 500).unwrap();
        pb.fill(0x406080ff);
        let bytes = pb.save_to_bufferv("png", &[]).unwrap();
        let texture = decode(&bytes, 208).unwrap();
        assert_eq!((texture.width(), texture.height()), (208, 104));
        let texture = decode(&bytes, 416).unwrap();
        assert_eq!((texture.width(), texture.height()), (416, 208));
    }

    #[test]
    fn caps_preserve_aspect_without_upscaling() {
        assert_eq!(fit(4000, 2000, 208), Some((208, 104)));
        assert_eq!(fit(20, 10, 208), Some((20, 10)));
        assert_eq!(fit(1, 10000, 60), Some((1, 60)));
        assert_eq!(fit(0, 100, 60), None);
        assert_eq!(fit(i32::MAX, i32::MAX, 96), Some((96, 96)));
    }
}
