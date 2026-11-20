//! Writing a file whole or not at all: the bytes go to a new file beside it, which is synced and
//! then renamed over it, so a crash or a full disk leaves the old file or the new one, never
//! part of either. Every state and settings file Digit keeps is written this way.

use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// Replaces `path` with `bytes`, making its folder when it is missing.
pub fn write(path: &Path, bytes: impl AsRef<[u8]>) -> io::Result<()> {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let name = path.file_name().ok_or_else(|| io::Error::other("no file name"))?.to_string_lossy();
    static NEXT: AtomicU64 = AtomicU64::new(0);
    // A name nobody else is writing to, even another Digit program saving the same file.
    let (tmp, mut file) = loop {
        let tmp = parent.join(format!(".{name}.{}.{}.new", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(file) => break (tmp, file),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    };
    let result = file.write_all(bytes.as_ref()).and_then(|()| file.sync_all()).and_then(|()| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn a_write_replaces_the_file_whole_and_a_failed_one_keeps_the_old() {
        let dir = std::env::temp_dir().join(format!("digit-atomic-{}", std::process::id()));
        let file = dir.join("nested/state.json");
        write(&file, "original").unwrap();
        write(&file, "replacement").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "replacement");
        // A folder it can't write to: the old file stays, and no temporary file is left.
        let folder = file.parent().unwrap();
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o555)).unwrap();
        let failed = write(&file, "lost");
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o755)).unwrap();
        if failed.is_ok() {
            // Running as root: permissions don't stop anything, so there is nothing to check.
            std::fs::remove_dir_all(&dir).unwrap();
            return;
        }
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "replacement");
        assert_eq!(std::fs::read_dir(folder).unwrap().count(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
