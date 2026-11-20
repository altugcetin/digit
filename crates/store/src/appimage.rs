//! AppImages installed from a file: kept in ~/Applications, with a launcher and an icon so the app
//! grid and Spotlight find them. Each launcher Digit writes carries `X-Digit-AppImage`, which is how
//! the Store lists and removes them later.
use gtk::{gio, glib, prelude::*};
use std::path::{Path, PathBuf};

const KEY: &str = "X-Digit-AppImage";

/// Where installed AppImages live.
pub fn dir() -> PathBuf {
    glib::home_dir().join("Applications")
}

fn launchers() -> PathBuf {
    glib::user_data_dir().join("applications")
}

fn icons() -> PathBuf {
    glib::user_data_dir().join("icons/digit-appimage")
}

/// An installed AppImage, as its launcher describes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Installed {
    pub name: String,
    pub comment: String,
    pub icon: Option<PathBuf>,
    pub file: PathBuf,
    pub launcher: PathBuf,
}

/// What the file shows before it is installed.
pub struct Peek {
    pub name: String,
    /// The file is installed already, at this launcher.
    pub installed: Option<Installed>,
}

/// A file name turned into an app name: "Foo-1.2.3-x86_64.AppImage" reads "Foo".
fn name_of(path: &Path) -> String {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    // The first "-1.2", "_v2" or "-x86_64" starts the part that isn't the name.
    let cut = stem.char_indices().find(|&(i, c)| (c == '-' || c == '_') && stem[i + 1..].starts_with(|c: char| c.is_ascii_digit() || c == 'v' || c == 'x')).map_or(stem.len(), |(i, _)| i);
    let name = stem[..cut].replace(['_', '-'], " ").trim().to_string();
    if name.is_empty() { stem } else { name }
}

/// Lowercase letters, digits and dashes, for file names Digit writes.
fn slug(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let s = s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    if s.is_empty() { "app".into() } else { s.chars().take(48).collect() }
}

/// Where an AppImage's filesystem starts: right after its ELF runtime, at the end of the section
/// headers. `None` when the file isn't a 64-bit type 2 AppImage.
fn payload_offset(path: &Path) -> Option<u64> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let mut h = [0u8; 64];
    f.read_exact(&mut h).ok()?;
    // ELF, 64-bit, little-endian, and the AppImage type 2 mark at byte 8.
    if &h[..4] != b"\x7fELF" || h[4] != 2 || h[5] != 1 || &h[8..11] != b"AI\x02" {
        return None;
    }
    let shoff = u64::from_le_bytes(h[0x28..0x30].try_into().ok()?);
    let shentsize = u16::from_le_bytes(h[0x3a..0x3c].try_into().ok()?) as u64;
    let shnum = u16::from_le_bytes(h[0x3c..0x3e].try_into().ok()?) as u64;
    let offset = shoff.checked_add(shentsize.checked_mul(shnum)?)?;
    let mut magic = [0u8; 4];
    f.seek(SeekFrom::Start(offset)).ok()?;
    f.read_exact(&mut magic).ok()?;
    (&magic == b"hsqs").then_some(offset)
}

pub fn valid(path: &Path) -> bool {
    payload_offset(path).is_some()
}

pub fn peek(path: &Path) -> Result<Peek, String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_file() || !valid(path) {
        return Err("This isn't an AppImage Digit can install. Only 64-bit AppImages of the current kind are supported.".into());
    }
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let installed = list().into_iter().find(|i| std::fs::canonicalize(&i.file).is_ok_and(|f| f == canonical) || i.file.file_name() == path.file_name());
    Ok(Peek { name: installed.as_ref().map(|i| i.name.clone()).unwrap_or_else(|| name_of(path)), installed })
}

async fn run(argv: &[&std::ffi::OsStr], cwd: &Path) -> Result<(), String> {
    let launcher = gio::SubprocessLauncher::new(gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_PIPE);
    launcher.set_cwd(cwd);
    let p = launcher.spawn(argv).map_err(|e| e.to_string())?;
    let (_, err) = p.communicate_utf8_future(None).await.map_err(|e| e.to_string())?;
    if p.is_successful() { Ok(()) } else { Err(err.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "extracting failed".into())) }
}

/// Copies the launcher and the icons at the top of the AppImage into `into`. With unsquashfs the
/// file is only read; without it, the AppImage's own runtime extracts them, which runs the file
/// the user has just chosen to install.
async fn extract(file: &Path, into: &Path) -> Result<PathBuf, String> {
    let root = into.join("squashfs-root");
    let patterns = ["*.desktop", "*.png", "*.svg", ".DirIcon"];
    if glib::find_program_in_path("unsquashfs").is_some() {
        let offset = payload_offset(file).ok_or("not an AppImage")?.to_string();
        let mut argv: Vec<&std::ffi::OsStr> = vec!["unsquashfs".as_ref(), "-q".as_ref(), "-n".as_ref(), "-o".as_ref(), offset.as_ref(), "-d".as_ref(), root.as_os_str(), file.as_os_str()];
        argv.extend(patterns.iter().map(std::ffi::OsStr::new));
        run(&argv, into).await?;
    } else {
        for p in patterns {
            let argv: [&std::ffi::OsStr; 3] = [file.as_os_str(), "--appimage-extract".as_ref(), p.as_ref()];
            run(&argv, into).await?;
        }
    }
    Ok(root)
}

/// The value of `key` in the [Desktop Entry] group.
fn field(text: &str, key: &str) -> Option<String> {
    let mut main = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            main = line == "[Desktop Entry]";
        } else if main && let Some(v) = line.strip_prefix(key).and_then(|r| r.trim_start().strip_prefix('=')) {
            return Some(v.trim().to_string());
        }
    }
    None
}

/// The AppImage's launcher, rewritten to start the installed file: `Exec` and `TryExec` point at
/// it, D-Bus activation goes (the app isn't installed as a service), and the icon is a full path.
fn launcher_text(original: &str, file: &Path, icon: Option<&Path>, fallback_name: &str) -> String {
    let quoted = format!("\"{}\"", file.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\""));
    let mut out = Vec::new();
    let mut main = false;
    let mut seen_main = false;
    for line in original.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            if main {
                out.push(format!("{KEY}={}", file.to_string_lossy()));
            }
            main = t == "[Desktop Entry]";
            seen_main |= main;
            out.push(line.to_string());
            continue;
        }
        if main {
            let key = t.split('=').next().unwrap_or("").trim();
            match key {
                "Exec" => {
                    // Keep the field codes and arguments; replace only the program.
                    let rest = t.split_once('=').map(|(_, v)| v.trim()).unwrap_or("");
                    let args = if let Some(r) = rest.strip_prefix('"') { r.split_once('"').map(|(_, a)| a).unwrap_or("") } else { rest.split_once(' ').map(|(_, a)| a).unwrap_or("") };
                    out.push(format!("Exec={quoted} {}", args.trim()).trim_end().to_string());
                    continue;
                }
                "TryExec" | "DBusActivatable" | "Icon" | KEY => continue,
                _ if key.starts_with("X-AppImage") => continue,
                _ => {}
            }
        }
        out.push(line.to_string());
    }
    if !seen_main {
        out = vec!["[Desktop Entry]".into(), "Type=Application".into(), format!("Name={fallback_name}"), format!("Exec={quoted} %U")];
        main = true;
    }
    if main {
        out.push(format!("{KEY}={}", file.to_string_lossy()));
    }
    // TryExec and Icon go right under the group header, where readers look first.
    let at = out.iter().position(|l| l.trim() == "[Desktop Entry]").map_or(0, |i| i + 1);
    let mut extra = vec![format!("TryExec={}", file.to_string_lossy())];
    if let Some(icon) = icon {
        extra.push(format!("Icon={}", icon.to_string_lossy()));
    }
    for (i, l) in extra.into_iter().enumerate() {
        out.insert(at + i, l);
    }
    out.join("\n") + "\n"
}

/// The icon the launcher names, found at the top of the AppImage, else `.DirIcon`.
fn pick_icon(root: &Path, icon: Option<&str>) -> Option<PathBuf> {
    let named = icon.into_iter().flat_map(|n| ["png", "svg"].map(|e| root.join(format!("{n}.{e}"))));
    named.chain([root.join(".DirIcon")]).find(|p| std::fs::canonicalize(p).is_ok_and(|c| c.starts_with(root) && c.is_file()))
}

/// Copies `source` into ~/Applications, makes it runnable and adds its launcher and icon.
pub async fn install(source: PathBuf) -> Result<Installed, String> {
    if !valid(&source) {
        return Err("This isn't an AppImage Digit can install".into());
    }
    let dir = dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file_name = source.file_name().ok_or("no file name")?.to_owned();
    let mut dest = dir.join(&file_name);
    let same = std::fs::canonicalize(&source).ok() == std::fs::canonicalize(&dest).ok();
    if !same {
        let mut n = 2;
        while dest.exists() {
            let stem = Path::new(&file_name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            dest = dir.join(format!("{stem} ({n}).AppImage"));
            n += 1;
        }
        let part = dest.with_extension("part");
        gio::File::for_path(&source)
            .copy_future(&gio::File::for_path(&part), gio::FileCopyFlags::OVERWRITE, glib::Priority::DEFAULT)
            .0
            .await
            .map_err(|e| e.to_string())?;
        std::fs::rename(&part, &dest).map_err(|e| e.to_string())?;
    }
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    let work = glib::user_cache_dir().join(format!("digit-appimage-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let done = finish(&dest, &work).await;
    let _ = std::fs::remove_dir_all(&work);
    if done.is_err() && !same {
        let _ = std::fs::remove_file(&dest);
    }
    done
}

async fn finish(dest: &Path, work: &Path) -> Result<Installed, String> {
    let fallback = name_of(dest);
    // A missing launcher or icon still installs: the file runs, under its own name.
    let root = extract(dest, work).await.ok();
    let desktop = root.as_ref().and_then(|r| std::fs::read_dir(r).ok()?.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|e| e == "desktop") && p.is_file()));
    let original = desktop.and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    let name = field(&original, "Name").filter(|n| !n.is_empty()).unwrap_or(fallback);
    let slug = slug(&name);
    let icon_src = root.as_ref().and_then(|r| pick_icon(r, field(&original, "Icon").as_deref()));
    let icon = icon_src.and_then(|src| {
        let real = std::fs::canonicalize(&src).ok()?;
        let ext = real.extension().map(|e| e.to_string_lossy().into_owned()).filter(|e| e == "svg").unwrap_or_else(|| "png".into());
        let to = icons().join(format!("{slug}.{ext}"));
        std::fs::create_dir_all(icons()).ok()?;
        std::fs::copy(&real, &to).ok()?;
        Some(to)
    });
    let text = launcher_text(&original, dest, icon.as_deref(), &name);
    std::fs::create_dir_all(launchers()).map_err(|e| e.to_string())?;
    let launcher = launchers().join(format!("appimage-{slug}.desktop"));
    kit::atomic::write(&launcher, &text).map_err(|e| e.to_string())?;
    Ok(Installed { name, comment: field(&text, "Comment").unwrap_or_default(), icon, file: dest.to_path_buf(), launcher })
}

/// AppImages Digit installed whose file is still there.
pub fn list() -> Vec<Installed> {
    let Ok(entries) = std::fs::read_dir(launchers()) else { return Vec::new() };
    let mut out: Vec<Installed> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "desktop"))
        .filter_map(|launcher| {
            let text = std::fs::read_to_string(&launcher).ok()?;
            let file = PathBuf::from(field(&text, KEY)?);
            file.is_file().then(|| Installed {
                name: field(&text, "Name").unwrap_or_else(|| name_of(&file)),
                comment: field(&text, "Comment").unwrap_or_default(),
                icon: field(&text, "Icon").map(PathBuf::from).filter(|p| p.is_absolute() && p.is_file()),
                file,
                launcher,
            })
        })
        .collect();
    out.sort_by_key(|i| i.name.to_lowercase());
    out
}

/// Removes the file, its launcher and its icon. Only paths Digit wrote are touched.
pub fn remove(app: &Installed) -> Result<(), String> {
    if !app.launcher.starts_with(launchers()) || !app.file.starts_with(dir()) {
        return Err("Refused: not an AppImage Digit installed".into());
    }
    std::fs::remove_file(&app.file).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) }).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&app.launcher);
    if let Some(icon) = &app.icon && icon.starts_with(icons()) {
        let _ = std::fs::remove_file(icon);
    }
    Ok(())
}

/// Starts an installed AppImage.
pub fn launch(app: &Installed) {
    if let Some(info) = gio_unix::DesktopAppInfo::from_filename(&app.launcher) {
        let _ = info.launch(&[], gio::AppLaunchContext::NONE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_drop_versions_and_architectures() {
        assert_eq!(name_of(Path::new("/x/Foo-1.2.3-x86_64.AppImage")), "Foo");
        assert_eq!(name_of(Path::new("/x/Bar_Baz-v2.AppImage")), "Bar Baz");
        assert_eq!(name_of(Path::new("/x/Cool-App.AppImage")), "Cool App");
        assert_eq!(slug("Cool App! 2"), "cool-app-2");
        assert_eq!(slug("…"), "app");
    }

    #[test]
    fn launcher_points_at_the_installed_file_and_keeps_arguments() {
        let original = "[Desktop Entry]\nName=Foo\nExec=AppRun --flag %U\nTryExec=AppRun\nIcon=foo\nDBusActivatable=true\nX-AppImage-Version=1\n\n[Desktop Action New]\nName=New\nExec=AppRun --new\n";
        let text = launcher_text(original, Path::new("/home/u/Applications/Foo.AppImage"), Some(Path::new("/i/foo.png")), "Foo");
        assert_eq!(field(&text, "Exec").as_deref(), Some("\"/home/u/Applications/Foo.AppImage\" --flag %U"));
        assert_eq!(field(&text, "TryExec").as_deref(), Some("/home/u/Applications/Foo.AppImage"));
        assert_eq!(field(&text, "Icon").as_deref(), Some("/i/foo.png"));
        assert_eq!(field(&text, KEY).as_deref(), Some("/home/u/Applications/Foo.AppImage"));
        assert!(!text.contains("DBusActivatable") && !text.contains("X-AppImage-Version"));
        // Actions keep their own Exec, under their own group.
        assert!(text.contains("[Desktop Action New]\nName=New\nExec=AppRun --new"));
    }

    #[test]
    fn a_missing_launcher_gets_a_plain_one() {
        let text = launcher_text("", Path::new("/a/B.AppImage"), None, "B");
        assert_eq!(field(&text, "Name").as_deref(), Some("B"));
        assert_eq!(field(&text, "Exec").as_deref(), Some("\"/a/B.AppImage\" %U"));
        assert_eq!(field(&text, "Type").as_deref(), Some("Application"));
    }

    #[test]
    fn only_appimages_are_valid() {
        let dir = std::env::temp_dir().join(format!("digit-appimage-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("x.AppImage");
        std::fs::write(&p, b"#!/bin/sh\necho hi\n").unwrap();
        assert!(!valid(&p));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
