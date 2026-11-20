use crate::ops::{self, Plan};
use crate::pane::{file_of, is_dir};
use gtk::{gdk, gio, glib, pango, prelude::*};
use std::path::{Path, PathBuf};

fn has(bin: &str) -> bool {
    glib::find_program_in_path(bin).is_some()
}

fn ct(info: &gio::FileInfo) -> String {
    info.attribute_string("standard::fast-content-type").map(|s| s.to_string()).unwrap_or_default()
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Image,
    Video,
    Audio,
    Document,
    Pdf,
    Other,
}

pub fn family(info: &gio::FileInfo) -> Family {
    if is_dir(info) {
        return Family::Other;
    }
    let c = ct(info);
    let name = info.name().to_string_lossy().to_lowercase();
    if c == "application/pdf" {
        Family::Pdf
    } else if c.starts_with("image/") && c != "image/svg+xml" {
        Family::Image
    } else if c.starts_with("video/") {
        Family::Video
    } else if c.starts_with("audio/") {
        Family::Audio
    } else if c.starts_with("application/vnd.oasis.opendocument") || c.starts_with("application/vnd.openxmlformats-officedocument") || c == "application/msword" || c == "application/vnd.ms-excel" || c == "application/vnd.ms-powerpoint" || c == "application/rtf" || name.ends_with(".rtf") {
        Family::Document
    } else {
        Family::Other
    }
}

pub fn targets(infos: &[gio::FileInfo]) -> Vec<(&'static str, &'static str)> {
    let fams: Vec<Family> = infos.iter().map(family).collect();
    let Some(first) = fams.first().copied() else { return Vec::new() };
    if fams.iter().any(|f| *f != first) {
        return Vec::new();
    }
    // The format they are all in already isn't offered: that would only make a copy.
    let same = |ext: &str| infos.iter().all(|i| same_format(&i.name().to_string_lossy(), ext));
    let all = match first {
        Family::Image if has("magick") || has("ffmpeg") => vec![("PNG", "png"), ("JPEG", "jpg"), ("WebP", "webp"), ("AVIF", "avif"), ("JPEG XL", "jxl"), ("Icon (ICO)", "ico"), ("PDF", "pdf")],
        Family::Video if has("ffmpeg") => vec![("MP4 (H.264)", "mp4"), ("WebM (VP9)", "webm"), ("MKV (AV1)", "mkv"), ("GIF", "gif"), ("MP3 (audio only)", "mp3")],
        Family::Audio if has("ffmpeg") => vec![("MP3", "mp3"), ("Opus", "opus"), ("FLAC", "flac"), ("WAV", "wav"), ("M4A (AAC)", "m4a")],
        Family::Document if has("soffice") => vec![("PDF", "pdf"), ("Word (DOCX)", "docx"), ("OpenDocument (ODT)", "odt")],
        Family::Pdf if has("pdftoppm") => return vec![("PNG pages", "png"), ("JPEG pages", "jpg")],
        _ => Vec::new(),
    };
    all.into_iter().filter(|(_, ext)| !same(ext)).collect()
}

/// Whether the file `name` is already in the format `ext` names (`.jpeg` is `jpg`).
fn same_format(name: &str, ext: &str) -> bool {
    let own = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let canon = |e: &str| match e {
        "jpeg" | "jpe" | "jfif" => "jpg".to_string(),
        e => e.to_string(),
    };
    canon(&own) == canon(ext)
}

fn out_path(src: &Path, ext: &str, suffix: &str) -> Option<PathBuf> {
    let dir = src.parent()?;
    let stem = src.file_stem()?.to_string_lossy().into_owned();
    let wanted = format!("{stem}{suffix}.{ext}");
    ops::unique(&gio::File::for_path(dir), &wanted).path()
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

pub fn convert(infos: Vec<gio::FileInfo>, ext: &str) {
    for info in infos {
        let Some(src) = file_of(&info).and_then(|f| f.path()) else { continue };
        let Some(dir) = src.parent().map(Path::to_path_buf) else { continue };
        let fam = family(&info);
        let ext = ext.to_string();
        let argv: Vec<String> = match fam {
            Family::Image => {
                let Some(out) = out_path(&src, &ext, "") else { continue };
                // A moving GIF to a still format: its first frame, not a numbered file per frame.
                let still = !matches!(ext.as_str(), "gif" | "webp" | "avif" | "pdf");
                let input = if still && has("magick") { format!("{}[0]", s(&src)) } else { s(&src) };
                let mut a: Vec<String> = if has("magick") { vec!["magick".into(), input, "-auto-orient".into()] } else { vec!["ffmpeg".into(), "-nostdin".into(), "-y".into(), "-i".into(), s(&src)] };
                if a[0] == "magick" {
                    match ext.as_str() {
                        "jpg" => a.extend(["-background".into(), "white".into(), "-flatten".into(), "-quality".into(), "90".into()]),
                        "webp" | "avif" | "jxl" => a.extend(["-quality".into(), "85".into()]),
                        "ico" => a.extend(["-resize".into(), "256x256>".into(), "-define".into(), "icon:auto-resize=256,128,64,48,32,16".into()]),
                        _ => {}
                    }
                }
                a.push(s(&out));
                run(&format!("Converting to {}", ext.to_uppercase()), a, dir, out);
                continue;
            }
            Family::Video | Family::Audio => {
                let Some(out) = out_path(&src, &ext, "") else { continue };
                let mut a: Vec<String> = vec!["ffmpeg".into(), "-nostdin".into(), "-hide_banner".into(), "-loglevel".into(), "error".into(), "-y".into(), "-i".into(), s(&src)];
                let extra: &[&str] = match ext.as_str() {
                    "mp4" => &["-c:v", "libx264", "-preset", "medium", "-crf", "22", "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "160k", "-movflags", "+faststart"],
                    "webm" => &["-c:v", "libvpx-vp9", "-crf", "32", "-b:v", "0", "-row-mt", "1", "-c:a", "libopus", "-b:a", "128k"],
                    "mkv" => &["-c:v", "libsvtav1", "-crf", "34", "-preset", "8", "-c:a", "libopus", "-b:a", "128k"],
                    "gif" => &["-vf", "fps=15,scale='min(720,iw)':-2:flags=lanczos,split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=bayer", "-loop", "0"],
                    "mp3" => &["-vn", "-c:a", "libmp3lame", "-q:a", "2"],
                    "opus" => &["-vn", "-c:a", "libopus", "-b:a", "128k"],
                    "flac" => &["-vn", "-c:a", "flac"],
                    "wav" => &["-vn", "-c:a", "pcm_s16le"],
                    "m4a" => &["-vn", "-c:a", "aac", "-b:a", "192k"],
                    _ => &[],
                };
                a.extend(extra.iter().map(|x| x.to_string()));
                a.push(s(&out));
                a
            }
            Family::Document => {
                let Some(out) = out_path(&src, &ext, "") else { continue };
                let work = dir.join(format!(".digit-convert-{}", glib::random_int()));
                if std::fs::create_dir(&work).is_err() {
                    continue;
                }
                let filter = match ext.as_str() {
                    "docx" => "docx:MS Word 2007 XML",
                    "odt" => "odt",
                    _ => "pdf",
                };
                let produced = work.join(format!("{}.{ext}", src.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
                let argv = vec!["soffice".into(), "--headless".into(), "--norestore".into(), "--convert-to".into(), filter.into(), "--outdir".into(), s(&work), s(&src)];
                ops::run_then("Converting document", Plan::Run(argv, dir.clone(), None), move || {
                    if std::fs::rename(&produced, &out).is_ok() {
                        crate::select_later(&gio::File::for_path(&out));
                    } else {
                        crate::toast("LibreOffice couldn't convert this document");
                    }
                    let _ = std::fs::remove_dir_all(&work);
                });
                continue;
            }
            Family::Pdf => {
                let stem = src.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let folder = ops::unique(&gio::File::for_path(&dir), &format!("{stem} pages"));
                let Some(fp) = folder.path() else { continue };
                if std::fs::create_dir(&fp).is_err() {
                    continue;
                }
                let flag = if ext == "jpg" { "-jpeg" } else { "-png" };
                let argv = vec!["pdftoppm".into(), flag.into(), "-r".into(), "150".into(), s(&src), s(&fp.join("page"))];
                run("Exporting pages", argv, dir, fp);
                continue;
            }
            Family::Other => continue,
        };
        let out = PathBuf::from(argv.last().cloned().unwrap_or_default());
        run(&format!("Converting to {}", ext.to_uppercase()), argv, dir, out);
    }
}

fn run(label: &str, argv: Vec<String>, cwd: PathBuf, out: PathBuf) {
    let o = out.clone();
    ops::run_then(label, Plan::Run(argv, cwd, Some(out)), move || {
        if o.exists() {
            crate::select_later(&gio::File::for_path(&o));
        }
    });
}

pub fn shrinkable(infos: &[gio::FileInfo]) -> bool {
    !infos.is_empty() && infos.iter().all(|i| match family(i) {
        Family::Image => has("magick"),
        Family::Video => has("ffmpeg"),
        Family::Pdf => has("gs"),
        _ => false,
    })
}

pub fn shrink(infos: Vec<gio::FileInfo>) {
    for info in infos {
        let Some(src) = file_of(&info).and_then(|f| f.path()) else { continue };
        let Some(dir) = src.parent().map(Path::to_path_buf) else { continue };
        let ext = src.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        // The tool's arguments and the new file, named here rather than found again in them: a
        // source already called "… (smaller)" would be taken for it.
        let (argv, out): (Vec<String>, PathBuf) = match family(&info) {
            Family::Image => {
                let target = if matches!(ext.as_str(), "png" | "webp" | "avif" | "jxl" | "gif") { ext.clone() } else { "jpg".into() };
                let Some(out) = out_path(&src, &target, " (smaller)") else { continue };
                let mut a = vec!["magick".into(), s(&src)];
                if target == "gif" {
                    // Every frame, resized whole and then stored as what changed: a GIF stays
                    // moving instead of turning into a JPEG per frame.
                    a.extend(["-coalesce".into(), "-resize".into(), "1280x1280>".into(), "-layers".into(), "Optimize".into()]);
                } else {
                    a.extend(["-auto-orient".into(), "-strip".into(), "-resize".into(), "2560x2560>".into()]);
                    if target == "png" {
                        a.extend(["-define".into(), "png:compression-level=9".into(), "-define".into(), "png:compression-filter=5".into()]);
                    } else {
                        a.extend(["-quality".into(), "78".into()]);
                    }
                }
                a.push(s(&out));
                (a, out)
            }
            Family::Video => {
                let Some(out) = out_path(&src, "mp4", " (smaller)") else { continue };
                let a = ["ffmpeg", "-nostdin", "-hide_banner", "-loglevel", "error", "-y", "-i"].iter().map(|x| x.to_string()).chain([s(&src)]).chain(["-vf", "scale='min(1920,iw)':-2", "-c:v", "libx264", "-preset", "slow", "-crf", "28", "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "128k", "-movflags", "+faststart"].iter().map(|x| x.to_string())).chain([s(&out)]).collect();
                (a, out)
            }
            Family::Pdf => {
                let Some(out) = out_path(&src, "pdf", " (smaller)") else { continue };
                let a = vec!["gs".into(), "-sDEVICE=pdfwrite".into(), "-dCompatibilityLevel=1.6".into(), "-dPDFSETTINGS=/ebook".into(), "-dNOPAUSE".into(), "-dQUIET".into(), "-dBATCH".into(), "-dSAFER".into(), format!("-sOutputFile={}", s(&out)), s(&src)];
                (a, out)
            }
            _ => continue,
        };
        let before = std::fs::metadata(&src).map(|m| m.len()).unwrap_or(0);
        let o = out.clone();
        ops::run_then("Making smaller", Plan::Run(argv, dir, Some(out)), move || {
            let Ok(meta) = std::fs::metadata(&o) else { return };
            let after = meta.len();
            if after == 0 {
                // Finished without writing anything: no empty file left behind.
                let _ = std::fs::remove_file(&o);
                ops::forget_made(&o);
                crate::toast("Couldn't make it smaller");
                return;
            }
            if after >= before {
                // No smaller than what it came from: not worth a second copy.
                let _ = std::fs::remove_file(&o);
                ops::forget_made(&o);
                crate::toast("It was already as small as it gets");
                return;
            }
            crate::toast(&format!("{} → {} ({}% smaller)", glib::format_size(before), glib::format_size(after), 100 - after * 100 / before.max(1)));
            crate::select_later(&gio::File::for_path(&o));
        });
    }
}

pub fn cutout(infos: Vec<gio::FileInfo>) {
    for info in infos.into_iter().filter(|i| family(i) == Family::Image) {
        let Some(src) = file_of(&info).and_then(|f| f.path()) else { continue };
        let Some(dir) = src.parent().map(Path::to_path_buf) else { continue };
        let Some(out) = out_path(&src, "png", " (no background)") else { continue };
        if has("rembg") {
            run("Removing background", vec!["rembg".into(), "i".into(), s(&src), s(&out)], dir, out);
            continue;
        }
        let o = out.clone();
        ops::run_then("Removing background", Plan::Native(Box::new(move || remove_background(&src, &o))), move || {
            if out.exists() {
                crate::select_later(&gio::File::for_path(&out));
            }
        });
    }
}

fn remove_background(src: &Path, out: &Path) -> Result<(), String> {
    let pb = gtk::gdk_pixbuf::Pixbuf::from_file(src).map_err(|e| e.to_string())?;
    let pb = pb.apply_embedded_orientation().unwrap_or(pb);
    let pb = pb.add_alpha(false, 0, 0, 0).map_err(|e| e.to_string())?;
    let (w, h) = (pb.width() as usize, pb.height() as usize);
    if w < 3 || h < 3 || w * h > 60_000_000 {
        return Err("This image is too small or too large".into());
    }
    let rs = pb.rowstride() as usize;
    let mut px = pb.read_pixel_bytes().to_vec();
    let at = |x: usize, y: usize| y * rs + x * 4;
    let mut border: Vec<[f64; 3]> = Vec::new();
    for x in (0..w).step_by((w / 200).max(1)) {
        for y in [0, h - 1] {
            let i = at(x, y);
            border.push([px[i] as f64, px[i + 1] as f64, px[i + 2] as f64]);
        }
    }
    for y in (0..h).step_by((h / 200).max(1)) {
        for x in [0, w - 1] {
            let i = at(x, y);
            border.push([px[i] as f64, px[i + 1] as f64, px[i + 2] as f64]);
        }
    }
    let median = |k: usize| {
        let mut v: Vec<f64> = border.iter().map(|c| c[k]).collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        v[v.len() / 2]
    };
    let bg = [median(0), median(1), median(2)];
    let dist = |i: usize, px: &[u8]| ((px[i] as f64 - bg[0]).powi(2) + (px[i + 1] as f64 - bg[1]).powi(2) + (px[i + 2] as f64 - bg[2]).powi(2)).sqrt();
    let spread = border.iter().map(|c| ((c[0] - bg[0]).powi(2) + (c[1] - bg[1]).powi(2) + (c[2] - bg[2]).powi(2)).sqrt()).sum::<f64>() / border.len() as f64;
    if spread > 60.0 {
        return Err("The background is too busy to remove on this computer (install rembg for photos)".into());
    }
    let t = (spread * 2.2 + 26.0).clamp(26.0, 90.0);
    let mut seen = vec![false; w * h];
    let mut queue = std::collections::VecDeque::new();
    for x in 0..w {
        queue.push_back((x, 0));
        queue.push_back((x, h - 1));
    }
    for y in 0..h {
        queue.push_back((0, y));
        queue.push_back((w - 1, y));
    }
    while let Some((x, y)) = queue.pop_front() {
        let k = y * w + x;
        if seen[k] || dist(at(x, y), &px) > t {
            continue;
        }
        seen[k] = true;
        if x > 0 {
            queue.push_back((x - 1, y));
        }
        if x + 1 < w {
            queue.push_back((x + 1, y));
        }
        if y > 0 {
            queue.push_back((x, y - 1));
        }
        if y + 1 < h {
            queue.push_back((x, y + 1));
        }
    }
    let gone = seen.iter().filter(|s| **s).count();
    if gone == 0 || gone == w * h {
        return Err("Couldn't find a background to remove".into());
    }
    for y in 0..h {
        for x in 0..w {
            let k = y * w + x;
            let i = at(x, y);
            if seen[k] {
                px[i + 3] = 0;
                continue;
            }
            let near = (x > 0 && seen[k - 1]) || (x + 1 < w && seen[k + 1]) || (y > 0 && seen[k - w]) || (y + 1 < h && seen[k + w]);
            if near {
                let d = dist(i, &px);
                let a = ((d - t * 0.5) / (t * 1.2)).clamp(0.25, 1.0);
                px[i + 3] = (px[i + 3] as f64 * a).round() as u8;
            }
        }
    }
    let result = gtk::gdk_pixbuf::Pixbuf::from_bytes(&glib::Bytes::from_owned(px), gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, w as i32, h as i32, rs as i32);
    let tmp = out.with_extension("png.part");
    result.savev(&tmp, "png", &[]).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, out).map_err(|e| e.to_string())
}

pub fn send(files: Vec<gio::File>) {
    let paths: Vec<PathBuf> = files.iter().filter_map(|f| f.path()).filter(|p| p.is_file()).collect();
    if paths.is_empty() {
        crate::toast("Pick files on this computer to send (folders can be compressed first)");
        return;
    }
    let n = paths.len();
    let (w, body, buttons) = ops::dialog(&format!("Send {}", if n == 1 { paths[0].file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default() } else { format!("{n} files") }), "Nearby devices with LocalSend or KDE Connect, and apps on this computer");
    let list = gtk::ListBox::new();
    list.add_css_class("picker");
    list.set_selection_mode(gtk::SelectionMode::None);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_min_content_height(240);
    scroll.set_max_content_height(380);
    scroll.set_propagate_natural_height(true);
    scroll.set_child(Some(&list));
    body.append(&scroll);
    let searching = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    searching.add_css_class("row");
    let spin = gtk::Spinner::new();
    spin.start();
    searching.append(&spin);
    searching.append(&kit::ui::text("Looking for nearby devices…", "sub"));
    list.append(&searching);
    let row = |icon: &str, name: &str, sub: &str| -> gtk::Box {
        let r = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        r.add_css_class("row");
        let t = kit::ui::tile(icon);
        r.append(&t);
        let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
        col.set_hexpand(true);
        let l = kit::ui::text(name, "name");
        l.set_ellipsize(pango::EllipsizeMode::End);
        col.append(&l);
        col.append(&kit::ui::text(sub, "sub"));
        r.append(&col);
        r.append(&kit::ui::sym("send"));
        r
    };
    let apps = kit::send::apps();
    let apps_head = kit::ui::text("Apps", "side-head");
    if !apps.is_empty() {
        list.append(&apps_head);
    }
    for (label, info) in &apps {
        let b = gtk::Button::new();
        b.add_css_class("flat-row");
        b.set_child(Some(&row("apps", label, "Open with these files")));
        let (info, paths, ww) = (info.clone(), paths.clone(), w.clone());
        b.connect_clicked(move |_| {
            let ctx = gdk::Display::default().map(|d| d.app_launch_context());
            match kit::send::launch_with(&info, &paths, ctx.as_ref().map(|c| c.upcast_ref())) {
                Ok(()) => crate::toast(&format!("Opened {}", info.display_name())),
                Err(e) => crate::toast(&e),
            }
            ww.close();
        });
        list.append(&b);
    }
    let close = kit::ui::button("Cancel", "");
    let ww = w.clone();
    close.connect_clicked(move |_| ww.close());
    buttons.append(&close);
    w.present();
    let held = w.clone();
    glib::spawn_future_local(async move {
        let (local, kde) = (gio::spawn_blocking(kit::send::discover).await.unwrap_or_default(), kit::send::kde().await);
        let w = held;
        if !w.is_shown() {
            return;
        }
        list.remove(&searching);
        let peers: Vec<kit::send::Peer> = kde.into_iter().chain(local).collect();
        if peers.is_empty() {
            let hint = gtk::Box::new(gtk::Orientation::Vertical, 4);
            hint.add_css_class("row");
            hint.append(&kit::ui::text("No devices found", "name"));
            let t = kit::ui::text("Open LocalSend or KDE Connect on the other device. Both need to be on the same network.", "sub");
            t.set_wrap(true);
            hint.append(&t);
            list.prepend(&hint);
            return;
        }
        let head = kit::ui::text("Nearby", "side-head");
        let mut at = 0;
        list.insert(&head, at);
        for p in peers {
            at += 1;
            let b = gtk::Button::new();
            b.add_css_class("flat-row");
            b.set_child(Some(&row(p.icon(), &p.name, p.service())));
            let (paths, ww) = (paths.clone(), w.clone());
            b.connect_clicked(move |_| {
                ww.close();
                deliver(p.clone(), paths.clone());
            });
            list.insert(&b, at);
        }
    });
}

fn deliver(peer: kit::send::Peer, paths: Vec<PathBuf>) {
    let cancel = gio::Cancellable::new();
    let row = crate::jobs_row(&format!("Sending to {}", peer.name), &cancel);
    match peer.via.clone() {
        kit::send::Via::KdeConnect { id } => {
            glib::spawn_future_local(async move {
                let ok = kit::send::kde_send(&id, &paths).await;
                crate::jobs_done(&row.0);
                crate::toast(&if ok { format!("Sent to {}", peer.name) } else { "KDE Connect couldn't send the files".to_string() });
            });
        }
        kit::send::Via::LocalSend { ip, port, https } => {
            let (tx, rx) = std::sync::mpsc::channel::<Result<String, String>>();
            std::thread::spawn(move || {
                let t2 = tx.clone();
                let progress = move |f: String| {
                    let _ = t2.send(Ok(f));
                };
                let r = kit::send::localsend(&ip, port, https, &paths, &progress);
                let _ = tx.send(r.map(|_| String::new()).map_err(|e| e.to_string()).and_then(|_| Err(String::new())));
            });
            let name = peer.name.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(150), move || {
                row.1.pulse();
                loop {
                    match rx.try_recv() {
                        Ok(Ok(f)) => row.2.set_text(&f),
                        Ok(Err(e)) => {
                            crate::jobs_done(&row.0);
                            crate::toast(&if e.is_empty() { format!("Sent to {name}") } else { format!("{name}: {e}") });
                            return glib::ControlFlow::Break;
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                        Err(_) => {
                            crate::jobs_done(&row.0);
                            return glib::ControlFlow::Break;
                        }
                    }
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_is_not_offered_its_own_format() {
        assert!(same_format("photo.JPEG", "jpg"));
        assert!(same_format("clip.mkv", "mkv"));
        assert!(!same_format("photo.png", "jpg"));
        assert!(!same_format("README", "png"));
    }
}
