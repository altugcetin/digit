use crate::pane::{file_of, is_dir};
use gtk::{gdk, gio, glib, prelude::*};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

const KEEP: usize = 320;
const WORKERS: usize = 3;

struct Job {
    key: String,
    uri: String,
    path: Option<PathBuf>,
    ct: String,
    mtime: i64,
    size: i64,
    big: bool,
    img: glib::WeakRef<gtk::Image>,
}

thread_local! {
    static CACHE: RefCell<(HashMap<String, gdk::Texture>, VecDeque<String>)> = RefCell::new((HashMap::new(), VecDeque::new()));
    static QUEUE: RefCell<VecDeque<Job>> = const { RefCell::new(VecDeque::new()) };
    static BUSY: Cell<usize> = const { Cell::new(0) };
    static MISS: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    static MARK: RefCell<HashMap<usize, String>> = RefCell::new(HashMap::new());
}

pub struct Thumbnailer {
    exec: Vec<String>,
    mimes: Vec<String>,
}

fn thumbnailers() -> &'static Vec<Thumbnailer> {
    static ALL: OnceLock<Vec<Thumbnailer>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut out = Vec::new();
        if glib::find_program_in_path("bwrap").is_none() {
            return out;
        }
        let mut dirs = vec![glib::user_data_dir().join("thumbnailers")];
        dirs.extend(glib::system_data_dirs().iter().map(|d| d.join("thumbnailers")));
        for d in dirs {
            let Ok(read) = std::fs::read_dir(&d) else { continue };
            for e in read.flatten() {
                let kf = glib::KeyFile::new();
                if kf.load_from_file(e.path(), glib::KeyFileFlags::NONE).is_err() {
                    continue;
                }
                let g = "Thumbnailer Entry";
                let (Ok(exec), Ok(mimes)) = (kf.string(g, "Exec"), kf.string_list(g, "MimeType")) else { continue };
                if let Ok(t) = kf.string(g, "TryExec") {
                    if glib::find_program_in_path(t.as_str()).is_none() {
                        continue;
                    }
                }
                let Ok(argv) = glib::shell_parse_argv(exec.as_str()) else { continue };
                let argv: Vec<String> = argv.iter().map(|a| a.to_string_lossy().into_owned()).collect();
                if argv.first().is_none_or(|b| glib::find_program_in_path(b).is_none()) {
                    continue;
                }
                out.push(Thumbnailer { exec: argv, mimes: mimes.iter().map(|m| m.to_string()).collect() });
            }
        }
        out
    })
}

fn wanted(ct: &str) -> bool {
    ct.starts_with("image/") || thumbnailers().iter().any(|t| t.mimes.iter().any(|m| m == ct))
}

fn policy() -> String {
    thread_local! {
        static P: RefCell<Option<(String, Instant)>> = const { RefCell::new(None) };
    }
    P.with(|p| {
        let mut p = p.borrow_mut();
        if let Some((v, t)) = p.as_ref() {
            if t.elapsed() < Duration::from_secs(5) {
                return v.clone();
            }
        }
        let v = kit::prefs::text(&kit::prefs::read(), "files_thumbnails", "local");
        *p = Some((v.clone(), Instant::now()));
        v
    })
}

fn id(img: &gtk::Image) -> usize {
    img.as_ptr() as usize
}

pub fn forget(img: &gtk::Image) {
    MARK.with(|m| m.borrow_mut().remove(&id(img)));
}

pub fn load(img: &gtk::Image, info: &gio::FileInfo, px: i32) {
    forget(img);
    if is_dir(info) {
        return;
    }
    let mode = policy();
    if mode == "off" {
        return;
    }
    let Some(file) = file_of(info) else { return };
    if mode == "local" && !file.is_native() {
        return;
    }
    let ct = info.attribute_string("standard::fast-content-type").map(|s| s.to_string()).unwrap_or_default();
    if !wanted(&ct) {
        return;
    }
    let big = px * img.scale_factor() > 128;
    let uri = file.uri().to_string();
    let key = format!("{}{uri}", if big { "L" } else { "N" });
    if let Some(tex) = CACHE.with(|c| c.borrow().0.get(&key).cloned()) {
        img.set_paintable(Some(&tex));
        return;
    }
    if MISS.with(|m| m.borrow().contains(&key)) {
        return;
    }
    MARK.with(|m| m.borrow_mut().insert(id(img), key.clone()));
    let job = Job {
        key,
        uri,
        path: file.path(),
        ct,
        mtime: info.modification_date_time().map(|d| d.to_unix()).unwrap_or(0),
        size: info.size(),
        big,
        img: img.downgrade(),
    };
    QUEUE.with(|q| {
        let mut q = q.borrow_mut();
        q.push_front(job);
        q.truncate(400);
    });
    pump();
}

fn live(job: &Job) -> bool {
    job.img.upgrade().is_some_and(|i| MARK.with(|m| m.borrow().get(&id(&i)) == Some(&job.key)))
}

fn pump() {
    while BUSY.with(|b| b.get()) < WORKERS {
        let Some(job) = QUEUE.with(|q| {
            let mut q = q.borrow_mut();
            while let Some(j) = q.pop_front() {
                if live(&j) {
                    return Some(j);
                }
            }
            None
        }) else {
            return;
        };
        BUSY.with(|b| b.set(b.get() + 1));
        let (uri, path, ct, mtime, size, big) = (job.uri.clone(), job.path.clone(), job.ct.clone(), job.mtime, job.size, job.big);
        glib::spawn_future_local(async move {
            let tex = gio::spawn_blocking(move || make(&uri, path.as_deref(), &ct, mtime, size, big)).await.ok().flatten();
            BUSY.with(|b| b.set(b.get() - 1));
            match tex {
                Some(t) => {
                    CACHE.with(|c| {
                        let mut c = c.borrow_mut();
                        c.0.insert(job.key.clone(), t.clone());
                        c.1.push_back(job.key.clone());
                        while c.1.len() > KEEP {
                            if let Some(old) = c.1.pop_front() {
                                c.0.remove(&old);
                            }
                        }
                    });
                    if live(&job) {
                        if let Some(i) = job.img.upgrade() {
                            i.set_paintable(Some(&t));
                        }
                    }
                }
                None => {
                    MISS.with(|m| m.borrow_mut().insert(job.key.clone()));
                }
            }
            pump();
        });
    }
}

fn cache_dir(big: bool) -> PathBuf {
    glib::user_cache_dir().join("thumbnails").join(if big { "large" } else { "normal" })
}

fn fresh(thumb: &Path, mtime: i64) -> bool {
    std::fs::metadata(thumb).ok().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).is_some_and(|t| t.as_secs() as i64 >= mtime)
}

fn make(uri: &str, path: Option<&Path>, ct: &str, mtime: i64, size: i64, big: bool) -> Option<gdk::Texture> {
    let md5 = glib::compute_checksum_for_string(glib::ChecksumType::Md5, uri)?.to_string();
    let n = if big { 256 } else { 128 };
    for (dir, ok) in [(cache_dir(big), true), (cache_dir(true), !big)] {
        let candidate = dir.join(format!("{md5}.png"));
        if ok && fresh(&candidate, mtime) {
            if let Ok(t) = gdk::Texture::from_filename(&candidate) {
                return Some(t);
            }
        }
    }
    let fail = glib::user_cache_dir().join("digit/files/fail").join(&md5);
    if fresh(&fail, mtime) {
        return None;
    }
    let dir = cache_dir(big);
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o700));
    let out = dir.join(format!("{md5}.png"));
    let tmp = dir.join(format!(".{md5}.{}.tmp.png", std::process::id()));
    let made = match path {
        Some(p) if ct.starts_with("image/") && size < 96 * 1024 * 1024 => scale(p, n, uri, mtime, &tmp) || external(p, uri, ct, n, &tmp),
        Some(p) => external(p, uri, ct, n, &tmp),
        None => false,
    };
    if made && std::fs::rename(&tmp, &out).is_ok() {
        return gdk::Texture::from_filename(&out).ok();
    }
    let _ = std::fs::remove_file(&tmp);
    if let Some(d) = fail.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(&fail, b"");
    None
}

fn scale(path: &Path, n: i32, uri: &str, mtime: i64, out: &Path) -> bool {
    let Ok(pb) = gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(path, n, n, true) else { return false };
    let pb = pb.apply_embedded_orientation().unwrap_or(pb);
    let m = mtime.to_string();
    pb.savev(out, "png", &[("tEXt::Thumb::URI", uri), ("tEXt::Thumb::MTime", m.as_str()), ("tEXt::Software", "Digit Files")]).is_ok()
}

fn external(path: &Path, uri: &str, ct: &str, n: i32, out: &Path) -> bool {
    let Some(t) = thumbnailers().iter().find(|t| t.mimes.iter().any(|m| m == ct)) else { return false };
    let Some(outdir) = out.parent() else { return false };
    let work = glib::user_cache_dir().join("digit/files/work").join(format!("{}-{}", std::process::id(), glib::random_int()));
    if std::fs::create_dir_all(&work).is_err() {
        return false;
    }
    let target = work.join("thumb.png");
    let input = path.to_string_lossy().into_owned();
    let size = n.to_string();
    let mut argv: Vec<String> = Vec::new();
    for a in &t.exec {
        argv.push(a.replace("%%", "\u{1}").replace("%i", &input).replace("%u", uri).replace("%o", &target.to_string_lossy()).replace("%s", &size).replace('\u{1}', "%"));
    }
    let mut cmd = std::process::Command::new("bwrap");
    cmd.args(["--ro-bind", "/usr", "/usr", "--ro-bind", "/etc", "/etc", "--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp"]);
    for top in ["/lib", "/lib64", "/bin", "/sbin"] {
        match std::fs::read_link(top) {
            Ok(t) => {
                cmd.args(["--symlink", &t.to_string_lossy(), top]);
            }
            Err(_) if Path::new(top).is_dir() => {
                cmd.args(["--ro-bind", top, top]);
            }
            Err(_) => {}
        }
    }
    if Path::new("/var/cache/fontconfig").is_dir() {
        cmd.args(["--ro-bind", "/var/cache/fontconfig", "/var/cache/fontconfig"]);
    }
    let registry = glib::user_cache_dir().join("gstreamer-1.0/registry.x86_64.bin");
    if registry.is_file() {
        cmd.args(["--ro-bind", &registry.to_string_lossy(), "/tmp/gst-registry.bin", "--setenv", "GST_REGISTRY", "/tmp/gst-registry.bin", "--setenv", "GST_REGISTRY_UPDATE", "no"]);
    }
    cmd.args(["--ro-bind", &input, &input, "--bind", &work.to_string_lossy(), &work.to_string_lossy()]);
    cmd.args(["--unshare-all", "--die-with-parent", "--new-session", "--clearenv", "--setenv", "HOME", "/tmp", "--setenv", "PATH", "/usr/bin:/bin", "--setenv", "LANG", "C.UTF-8", "--"]);
    cmd.args(&argv);
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    let ok = match cmd.spawn() {
        Ok(mut child) => {
            let start = Instant::now();
            loop {
                match child.try_wait() {
                    Ok(Some(s)) => break s.success(),
                    Ok(None) if start.elapsed() > Duration::from_secs(12) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break false;
                    }
                    Ok(None) => std::thread::sleep(Duration::from_millis(40)),
                    Err(_) => break false,
                }
            }
        }
        Err(_) => false,
    };
    let done = ok && target.is_file() && {
        match gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(&target, n, n, true) {
            Ok(pb) => {
                let m = std::fs::metadata(path).ok().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0).to_string();
                pb.savev(out, "png", &[("tEXt::Thumb::URI", uri), ("tEXt::Thumb::MTime", m.as_str()), ("tEXt::Software", "Digit Files")]).is_ok()
            }
            Err(_) => false,
        }
    };
    let _ = outdir;
    let _ = std::fs::remove_dir_all(&work);
    done
}

pub fn texture_for(file: &gio::File, info: &gio::FileInfo, done: impl FnOnce(Option<gdk::Texture>) + 'static) {
    let ct = info.attribute_string("standard::fast-content-type").map(|s| s.to_string()).unwrap_or_default();
    if is_dir(info) || !wanted(&ct) {
        done(None);
        return;
    }
    let (uri, path, mtime, size) = (file.uri().to_string(), file.path(), info.modification_date_time().map(|d| d.to_unix()).unwrap_or(0), info.size());
    glib::spawn_future_local(async move {
        let t = gio::spawn_blocking(move || make(&uri, path.as_deref(), &ct, mtime, size, true)).await.ok().flatten();
        done(t);
    });
}
