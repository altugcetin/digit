use super::Target;
use glib::translate::*;
use gtk::{gio, glib, prelude::*};
use std::collections::HashSet;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Component, Path, PathBuf};

pub(super) struct Row {
    pub icon: &'static str,
    pub name: String,
    pub eject: Option<gio::Mount>,
    pub target: Target,
}

// Keep presentation and GIO objects out of the selection policy. Names are not
// identities: two different partitions can have the same label.
#[derive(Default)]
struct Candidate {
    volume: Option<String>,
    root: Option<String>,
    hidden: bool,
    shadowed: bool,
}

struct LocalMount {
    device: PathBuf,
    path: PathBuf,
    name: String,
    fs_type: String,
}

fn local_mounts() -> Vec<LocalMount> {
    // The safe MountEntry binding requires GLib 2.84. Use GIO's older API here
    // to retain the project's GTK 4.18 / GLib 2.80 minimum. Copy all values
    // before freeing each owned entry and then its list container.
    // SAFETY: g_unix_mounts_get returns a list we own; each entry is read only while alive, freed once, and the list after its entries.
    unsafe {
        let list = gio_unix::ffi::g_unix_mounts_get(std::ptr::null_mut());
        let mut link = list;
        let mut mounts = Vec::new();
        while !link.is_null() {
            let entry = (*link).data.cast();
            let name: glib::GString = from_glib_full(gio_unix::ffi::g_unix_mount_guess_name(entry));
            let fs_type: glib::GString =
                from_glib_none(gio_unix::ffi::g_unix_mount_get_fs_type(entry));
            mounts.push(LocalMount {
                device: from_glib_none(gio_unix::ffi::g_unix_mount_get_device_path(entry)),
                path: from_glib_none(gio_unix::ffi::g_unix_mount_get_mount_path(entry)),
                name: name.into(),
                fs_type: fs_type.into(),
            });
            gio_unix::ffi::g_unix_mount_free(entry);
            link = (*link).next;
        }
        glib::ffi::g_list_free(list);
        mounts
    }
}

fn select(candidates: &[Candidate]) -> Vec<usize> {
    let mut volumes = HashSet::new();
    let mut roots = HashSet::new();
    candidates
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            if c.hidden || c.shadowed {
                return None;
            }
            let duplicate = c.volume.as_ref().is_some_and(|v| volumes.contains(v))
                || c.root.as_ref().is_some_and(|r| roots.contains(r));
            // A duplicate root can introduce a second identifier for a volume
            // (e.g. a UUID-only GIO volume and a /dev mount-table entry).
            if let Some(v) = &c.volume {
                volumes.insert(v.clone());
            }
            if let Some(r) = &c.root {
                roots.insert(r.clone());
            }
            (!duplicate).then_some(i)
        })
        .collect()
}

fn hidden_path(path: &Path) -> bool {
    gio_unix::functions::is_mount_path_system_internal(path)
        || path == glib::home_dir()
        || ["/boot", "/efi", "/recovery"]
            .iter()
            .any(|p| path.starts_with(p))
        || path
            .components()
            .any(|c| matches!(c, Component::Normal(s) if s.as_encoded_bytes().starts_with(b".")))
}

fn hidden_options(options: &str) -> bool {
    options
        .split(',')
        .any(|o| matches!(o, "x-gvfs-hide" | "comment=x-gvfs-hide"))
}

fn hidden_properties(properties: &str) -> bool {
    // UDisks' udev rules set this for recovery, EFI, reserved and explicitly
    // hidden partitions. HintSystem / HintAuto do not hide fixed data disks.
    properties
        .lines()
        .any(|line| matches!(line, "E:UDISKS_IGNORE=1" | "E:ID_FS_TYPE=swap"))
}

fn hidden_device(device: &Path) -> bool {
    let Ok(meta) = device.metadata() else {
        return false;
    };
    if !meta.file_type().is_block_device() {
        return false;
    }
    let path = format!(
        "/run/udev/data/b{}:{}",
        libc::major(meta.rdev()),
        libc::minor(meta.rdev())
    );
    std::fs::read_to_string(path).is_ok_and(|p| hidden_properties(&p))
}

fn device_key(device: &Path) -> String {
    // /dev/mapper, /dev/disk/by-* and the kernel name may all refer to one disk.
    format!(
        "device:{}",
        device
            .canonicalize()
            .unwrap_or_else(|_| device.to_path_buf())
            .display()
    )
}

fn volume_key(volume: &gio::Volume) -> String {
    if let Some(device) = volume.identifier("unix-device") {
        device_key(Path::new(device.as_str()))
    } else if let Some(uuid) = volume.uuid() {
        format!("uuid:{uuid}")
    } else {
        format!("volume:{:p}", volume.as_ptr())
    }
}

fn size(device: &Path) -> Option<u64> {
    let meta = device.metadata().ok()?;
    if !meta.file_type().is_block_device() {
        return None;
    }
    let path = format!(
        "/sys/dev/block/{}:{}/size",
        libc::major(meta.rdev()),
        libc::minor(meta.rdev())
    );
    std::fs::read_to_string(path)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?
        .checked_mul(512)
}

/// The icon for what GVfs reaches at `root` beyond this computer: a phone, a camera, or a server.
fn remote_icon(root: &gio::File) -> &'static str {
    match root.uri_scheme().as_deref() {
        Some("mtp" | "afc") => "smartphone",
        Some("gphoto2") => "photo_camera",
        _ => "lan",
    }
}

fn name(label: &str, bytes: Option<u64>) -> String {
    if !label.trim().is_empty() {
        label.to_string()
    } else if let Some(bytes) = bytes.filter(|b| *b > 0) {
        format!("{} Volume", glib::format_size(bytes))
    } else {
        "Volume".into()
    }
}

pub(super) fn rows(monitor: &gio::VolumeMonitor) -> Vec<Row> {
    let mounts = local_mounts();
    let (points, _) = gio_unix::MountPoint::mount_points();
    let hidden_root = |root: &gio::File| {
        root.path().is_some_and(|path| {
            hidden_path(&path)
                || points.iter().any(|p| {
                    p.mount_path() == path
                        && (p.fs_type() == "swap"
                            || hidden_options(p.options().as_deref().unwrap_or_default()))
                })
                || mounts.iter().any(|m| m.path == path && m.fs_type == "swap")
        })
    };
    let hidden_volume = |v: &gio::Volume| {
        // GVfs applies UDisks' ignore/recovery hints before exposing volumes.
        // should_automount() is deliberately not a visibility filter: fixed
        // disks and volumes configured with noauto still belong in Devices.
        v.activation_root().as_ref().is_some_and(hidden_root)
            || v.identifier("unix-device")
                .is_some_and(|d| hidden_device(Path::new(d.as_str())))
            || points.iter().any(|p| {
                let matches = v.identifier("unix-device").is_some_and(|d| {
                    device_key(Path::new(d.as_str())) == device_key(&p.device_path())
                }) || v
                    .uuid()
                    .is_some_and(|u| p.device_path() == Path::new(&format!("UUID={u}")));
                matches
                    && (hidden_path(&p.mount_path())
                        || p.fs_type() == "swap"
                        || hidden_options(p.options().as_deref().unwrap_or_default()))
            })
    };
    let mut volumes = monitor.volumes();
    for drive in monitor.connected_drives() {
        volumes.extend(drive.volumes());
    }
    let mut candidates = Vec::new();
    let mut rows = Vec::new();
    for v in volumes {
        let mount = v.get_mount();
        let root = mount.as_ref().map(|m| m.root());
        candidates.push(Candidate {
            volume: Some(volume_key(&v)),
            root: root.as_ref().map(|r| r.uri().to_string()),
            hidden: hidden_volume(&v) || root.as_ref().is_some_and(hidden_root),
            shadowed: mount.as_ref().is_some_and(|m| m.is_shadowed()),
        });
        let device = v.identifier("unix-device");
        // A phone not mounted yet says what it is by where it would be.
        let at = root.clone().or_else(|| v.activation_root());
        rows.push(Row {
            icon: if let Some(r) = at.as_ref().filter(|r| !r.is_native()) {
                remote_icon(r)
            } else if mount
                .as_ref()
                .map_or_else(|| v.can_eject(), |m| m.can_eject())
            {
                "usb"
            } else {
                "hard_drive"
            },
            name: name(
                &v.name(),
                device.as_deref().and_then(|d| size(Path::new(d))),
            ),
            eject: mount
                .as_ref()
                .filter(|m| m.can_eject() || m.can_unmount())
                .cloned(),
            target: mount
                .map(Target::Mount)
                .unwrap_or_else(|| Target::Volume(v)),
        });
    }
    let monitored_mounts = monitor.mounts();
    for m in &monitored_mounts {
        let root = m.root();
        let volume = m.volume();
        candidates.push(Candidate {
            volume: volume.as_ref().map(volume_key),
            root: Some(root.uri().to_string()),
            hidden: hidden_root(&root) || volume.as_ref().is_some_and(hidden_volume),
            shadowed: m.is_shadowed(),
        });
        rows.push(Row {
            icon: if !root.is_native() {
                remote_icon(&root)
            } else if m.can_eject() {
                "usb"
            } else {
                "hard_drive"
            },
            name: name(&m.name(), None),
            eject: (m.can_eject() || m.can_unmount()).then(|| m.clone()),
            target: Target::Mount(m.clone()),
        });
    }
    // GIO's desktop monitor can omit mounted fixed disks outside /media and
    // /run/media. Supplement it with actual local block-device mounts, rather
    // than applying guess_should_display() and losing those disks again.
    let root_devices: HashSet<_> = mounts
        .iter()
        .filter(|m| m.path == Path::new("/"))
        .map(|m| device_key(&m.device))
        .collect();
    for m in &mounts {
        let device = &m.device;
        if !device.starts_with("/dev")
            || root_devices.contains(&device_key(device))
            || gio_unix::functions::is_system_fs_type(&m.fs_type)
        {
            continue;
        }
        let root = gio::File::for_path(&m.path);
        candidates.push(Candidate {
            volume: Some(device_key(device)),
            root: Some(root.uri().to_string()),
            hidden: hidden_root(&root) || hidden_device(device),
            shadowed: monitored_mounts
                .iter()
                .any(|m| m.is_shadowed() && m.root().equal(&root)),
        });
        rows.push(Row {
            icon: "hard_drive",
            name: name(&m.name, size(device)),
            eject: None,
            target: Target::File(root),
        });
    }
    let selected: HashSet<_> = select(&candidates).into_iter().collect();
    rows.into_iter()
        .enumerate()
        .filter_map(|(i, row)| selected.contains(&i).then_some(row))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume(id: &str, root: Option<&str>) -> Candidate {
        Candidate {
            volume: Some(id.into()),
            root: root.map(String::from),
            ..Candidate::default()
        }
    }

    #[test]
    fn includes_unmounted_fixed_volumes_and_mounts_outside_media() {
        let candidates = [
            volume("internal", None),
            volume("data", Some("file:///mnt/data")),
        ];
        assert_eq!(select(&candidates), [0, 1]);
    }

    #[test]
    fn deduplicates_monitor_drive_and_mount_views_of_a_volume() {
        let candidates = [
            volume("data", Some("file:///mnt/data")),
            volume("data", None),
            volume("data", Some("file:///mnt/data")),
        ];
        assert_eq!(select(&candidates), [0]);
    }

    #[test]
    fn deduplicates_mounts_by_root_without_losing_other_partitions() {
        let candidates = [
            volume("a", Some("file:///mnt/a")),
            Candidate {
                root: Some("file:///mnt/a".into()),
                ..Candidate::default()
            },
            volume("b", Some("file:///mnt/b")),
        ];
        assert_eq!(select(&candidates), [0, 2]);
    }

    #[test]
    fn a_duplicate_root_registers_device_aliases_for_other_mount_points() {
        let candidates = [
            volume("uuid:data", Some("file:///mnt/data")),
            volume("device:/dev/sdb1", Some("file:///mnt/data")),
            volume("device:/dev/sdb1", Some("file:///mnt/alias")),
        ];
        assert_eq!(select(&candidates), [0]);
    }

    #[test]
    fn hidden_and_shadowed_mounts_do_not_consume_a_visible_identity() {
        let candidates = [
            Candidate {
                hidden: true,
                ..volume("data", None)
            },
            Candidate {
                shadowed: true,
                ..volume("data", Some("file:///mnt/data"))
            },
            volume("data", Some("file:///mnt/data")),
        ];
        assert_eq!(select(&candidates), [2]);
    }

    #[test]
    fn keeps_system_and_explicitly_hidden_mounts_out() {
        for path in [
            "/",
            "/boot",
            "/boot/efi",
            "/efi",
            "/recovery",
            "/mnt/.recovery",
        ] {
            assert!(hidden_path(Path::new(path)), "{path}");
        }
        for path in ["/mnt/data", "/data", "/run/media/user/disk"] {
            assert!(!hidden_path(Path::new(path)), "{path}");
        }
        assert!(hidden_options("defaults,noauto,x-gvfs-hide"));
        assert!(hidden_options("defaults,comment=x-gvfs-hide"));
        assert!(!hidden_options("defaults,noauto"));
    }

    #[test]
    fn respects_udisks_ignore_and_swap_but_keeps_fixed_noauto_disks() {
        assert!(hidden_properties("E:UDISKS_IGNORE=1\nE:ID_FS_TYPE=ntfs\n"));
        assert!(hidden_properties("E:ID_FS_TYPE=swap\n"));
        assert!(!hidden_properties(
            "E:UDISKS_IGNORE=0\nE:UDISKS_SYSTEM=1\nE:UDISKS_AUTO=0\n"
        ));
    }

    #[test]
    fn phones_and_cameras_have_their_own_icons() {
        assert_eq!(remote_icon(&gio::File::for_uri("mtp://Pixel_8/")), "smartphone");
        assert_eq!(remote_icon(&gio::File::for_uri("afc://00008110/")), "smartphone");
        assert_eq!(remote_icon(&gio::File::for_uri("gphoto2://Canon/")), "photo_camera");
        assert_eq!(remote_icon(&gio::File::for_uri("sftp://host/")), "lan");
    }

    #[test]
    fn uses_volume_names_and_capacity_fallbacks() {
        assert_eq!(name("Data", Some(500_000_000_000)), "Data");
        assert_eq!(
            name(" ", Some(500_000_000_000)),
            format!("{} Volume", glib::format_size(500_000_000_000))
        );
        assert_eq!(name("", None), "Volume");
    }
}
