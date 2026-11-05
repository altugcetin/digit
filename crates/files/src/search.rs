use glib::translate::ToGlibPtr;
use gtk::{gio, glib, prelude::*};
use std::ffi::{CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{SyncSender, TrySendError};
use std::time::{Duration, Instant};

pub const DEBOUNCE: Duration = Duration::from_millis(220);
pub const BATCH_SIZE: usize = 200;
pub const BATCH_INTERVAL: Duration = Duration::from_millis(75);
pub const RESULT_LIMIT: usize = 2000;
// Preserve the existing recursive search scope without a wide directory queue.
const MAX_DEPTH: usize = 14;

pub fn normalize(query: &str) -> String {
    query.trim().to_lowercase()
}

pub fn matches(name: &str, query: &str) -> bool {
    name.to_lowercase().contains(query)
}

pub fn narrows(previous: &str, query: &str) -> bool {
    !previous.is_empty() && query.contains(previous)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Complete,
    Limited,
    Cancelled,
}

pub struct Cache {
    pub query: String,
    pub outcome: Outcome,
}

impl Cache {
    pub fn can_narrow(&self, previous: &str, query: &str) -> bool {
        // A partial/capped search cannot prove that all narrower matches are cached.
        self.outcome == Outcome::Complete && narrows(previous, query) && narrows(&self.query, query)
    }
}

/// Iterative traversal, with cancellation, time/size batching and the cap checked
/// per entry (including within a single very large directory). Metadata is read
/// by `map` on the worker, never by the receiver on GTK's main thread.
pub fn scan<T>(
    root: &Path,
    query: &str,
    hidden: bool,
    cancel: &gio::Cancellable,
    mut map: impl FnMut(PathBuf) -> Option<T>,
    mut emit: impl FnMut(Vec<T>) -> bool,
) -> Outcome {
    if cancel.is_cancelled() {
        return Outcome::Cancelled;
    }
    let Ok(read) = std::fs::read_dir(root) else {
        return Outcome::Complete;
    };
    let mut stack = vec![read];
    let mut batch = Vec::with_capacity(BATCH_SIZE);
    let mut total = 0;
    let mut last = Instant::now();
    while let Some(read) = stack.last_mut() {
        if cancel.is_cancelled() {
            return Outcome::Cancelled;
        }
        let Some(entry) = read.next() else {
            stack.pop();
            continue;
        };
        let Ok(entry) = entry else { continue };
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !hidden && (name.starts_with('.') || name.ends_with('~')) {
            continue;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if matches(&name, query)
            && let Some(item) = map(entry.path())
        {
            batch.push(item);
            total += 1;
        }
        if cancel.is_cancelled() {
            return Outcome::Cancelled;
        }
        if !batch.is_empty()
            && (batch.len() >= BATCH_SIZE
                || last.elapsed() >= BATCH_INTERVAL
                || total >= RESULT_LIMIT)
        {
            if !emit(std::mem::replace(
                &mut batch,
                Vec::with_capacity(BATCH_SIZE),
            )) {
                return Outcome::Cancelled;
            }
            last = Instant::now();
        }
        if total >= RESULT_LIMIT {
            return Outcome::Limited;
        }
        // DirEntry::file_type does not follow links; symlink loops stay leaves.
        if kind.is_dir()
            && !kind.is_symlink()
            && stack.len() <= MAX_DEPTH
            && let Ok(read) = std::fs::read_dir(entry.path())
        {
            stack.push(read);
        }
    }
    if cancel.is_cancelled() || (!batch.is_empty() && !emit(batch)) {
        Outcome::Cancelled
    } else {
        Outcome::Complete
    }
}

/// A bounded queue supplies backpressure without trapping a cancelled worker
/// behind a full queue while GTK is busy or the pane is being closed.
pub fn send<T>(tx: &SyncSender<T>, mut value: T, cancel: &gio::Cancellable) -> bool {
    while !cancel.is_cancelled() {
        match tx.try_send(value) {
            Ok(()) => return true,
            Err(TrySendError::Disconnected(_)) => return false,
            Err(TrySendError::Full(pending)) => value = pending,
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

// GFileInfo and GIcon are not Send. Transfer their requested attributes as owned
// data, preserving file names as bytes and icons as serialized GVariants.
enum Attribute {
    String(String),
    Bytes(CString),
    Boolean(bool),
    Uint32(u32),
    Int32(i32),
    Uint64(u64),
    Int64(i64),
    Icon(glib::Variant),
}

fn byte_attribute(info: &gio::FileInfo, name: &str) -> Option<CString> {
    let name = CString::new(name).ok()?;
    // The older gtk-rs byte-string getter constructs a UTF-8 GString, which
    // panics for valid Unix filenames containing non-UTF-8 bytes. Copy the raw
    // value instead. GFileInfo owns this NUL-terminated string, and remains
    // alive and unmodified until the copy is complete.
    let value = unsafe {
        gio::ffi::g_file_info_get_attribute_byte_string(info.to_glib_none().0, name.as_ptr())
    };
    if value.is_null() {
        None
    } else {
        Some(unsafe { CStr::from_ptr(value) }.to_owned())
    }
}

pub struct Hit {
    path: PathBuf,
    attributes: Vec<(&'static str, Attribute)>,
}

impl Hit {
    pub fn read(
        path: PathBuf,
        attrs: &'static str,
        hidden: bool,
        cancel: &gio::Cancellable,
    ) -> Option<Self> {
        let file = gio::File::for_path(&path);
        let info = file
            .query_info(attrs, gio::FileQueryInfoFlags::NONE, Some(cancel))
            .ok()?;
        if cancel.is_cancelled() || (!hidden && (info.is_hidden() || info.is_backup())) {
            return None;
        }
        let attributes = attrs
            .split(',')
            .filter_map(|name| {
                let value = match info.attribute_type(name) {
                    gio::FileAttributeType::String => {
                        Attribute::String(info.attribute_string(name)?.to_string())
                    }
                    gio::FileAttributeType::ByteString => {
                        Attribute::Bytes(byte_attribute(&info, name)?)
                    }
                    gio::FileAttributeType::Boolean => Attribute::Boolean(info.boolean(name)),
                    gio::FileAttributeType::Uint32 => {
                        Attribute::Uint32(info.attribute_uint32(name))
                    }
                    gio::FileAttributeType::Int32 => Attribute::Int32(info.attribute_int32(name)),
                    gio::FileAttributeType::Uint64 => {
                        Attribute::Uint64(info.attribute_uint64(name))
                    }
                    gio::FileAttributeType::Int64 => Attribute::Int64(info.attribute_int64(name)),
                    gio::FileAttributeType::Object if name == "standard::icon" => {
                        Attribute::Icon(info.icon()?.serialize()?)
                    }
                    _ => return None,
                };
                Some((name, value))
            })
            .collect();
        Some(Self { path, attributes })
    }

    /// This only reconstructs data already read by the worker; no filesystem IO.
    pub fn into_info(self) -> gio::FileInfo {
        let info = gio::FileInfo::new();
        for (name, value) in self.attributes {
            match value {
                Attribute::String(value) => info.set_attribute_string(name, &value),
                Attribute::Bytes(value) => info.set_attribute(name, value.as_c_str()),
                Attribute::Boolean(value) => info.set_attribute_boolean(name, value),
                Attribute::Uint32(value) => info.set_attribute_uint32(name, value),
                Attribute::Int32(value) => info.set_attribute_int32(name, value),
                Attribute::Uint64(value) => info.set_attribute_uint64(name, value),
                Attribute::Int64(value) => info.set_attribute_int64(name, value),
                Attribute::Icon(value) => {
                    if let Some(icon) = gio::Icon::deserialize(&value) {
                        info.set_icon(&icon);
                    }
                }
            }
        }
        info.set_attribute_object("standard::file", &gio::File::for_path(self.path));
        info
    }
}

pub enum Message {
    Batch(Vec<Hit>),
    Finished(Outcome),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;

    struct Tree(PathBuf);

    impl Tree {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "digit-search-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn files(&self, count: usize) {
            for i in 0..count {
                std::fs::File::create(self.0.join(format!("match-{i:05}.txt"))).unwrap();
            }
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn collect(tree: &Tree, query: &str, hidden: bool) -> (Outcome, Vec<Vec<PathBuf>>) {
        let mut batches = Vec::new();
        let outcome = scan(
            &tree.0,
            query,
            hidden,
            &gio::Cancellable::new(),
            Some,
            |batch| {
                batches.push(batch);
                true
            },
        );
        (outcome, batches)
    }

    #[test]
    fn matcher_preserves_case_insensitive_literal_substrings() {
        assert_eq!(normalize("  RÉsumÉ  "), "résumé");
        assert!(matches("My RÉSUMÉ.pdf", &normalize("  résumé ")));
        assert!(matches("a[1]*.txt", "[1]*"));
        assert!(!matches("notes.txt", "note "));
        assert!(matches("notes.txt", ""));
    }

    #[test]
    fn narrowing_filters_complete_results_without_losing_matches() {
        let tree = Tree::new();
        for name in ["report.txt", "REPORT.pdf", "reply.txt", "other.txt"] {
            std::fs::File::create(tree.0.join(name)).unwrap();
        }
        let (outcome, batches) = collect(&tree, "re", false);
        let cache = Cache {
            query: "re".into(),
            outcome,
        };
        assert!(cache.can_narrow("re", "report"));
        let mut filtered: Vec<_> = batches
            .into_iter()
            .flatten()
            .filter(|p| matches(&p.file_name().unwrap().to_string_lossy(), "report"))
            .collect();
        let mut rescanned: Vec<_> = collect(&tree, "report", false)
            .1
            .into_iter()
            .flatten()
            .collect();
        filtered.sort();
        rescanned.sort();
        assert_eq!(filtered, rescanned);
        assert_eq!(filtered.len(), 2);
        assert!(
            !cache.can_narrow("report", "repo"),
            "Backspace must restart"
        );
        assert!(!cache.can_narrow("report", "other"));
        for outcome in [Outcome::Limited, Outcome::Cancelled] {
            assert!(
                !Cache {
                    query: "re".into(),
                    outcome
                }
                .can_narrow("re", "report")
            );
        }
    }

    #[test]
    fn one_directory_streams_bounded_batches_and_flushes_the_tail() {
        let tree = Tree::new();
        tree.files(BATCH_SIZE * 2 + 17);
        let (outcome, batches) = collect(&tree, "match", false);
        assert_eq!(outcome, Outcome::Complete);
        assert_eq!(
            batches.iter().map(Vec::len).sum::<usize>(),
            BATCH_SIZE * 2 + 17
        );
        assert!(batches.len() >= 3);
        assert!(
            batches
                .iter()
                .all(|batch| !batch.is_empty() && batch.len() <= BATCH_SIZE)
        );
        assert!(batches.last().unwrap().len() < BATCH_SIZE);
    }

    #[test]
    fn sparse_matches_flush_on_time_before_the_directory_finishes() {
        let tree = Tree::new();
        tree.files(3);
        let visited = Cell::new(0);
        let mut flushed_at = Vec::new();
        let outcome = scan(
            &tree.0,
            "match",
            false,
            &gio::Cancellable::new(),
            |path| {
                visited.set(visited.get() + 1);
                if visited.get() == 2 {
                    std::thread::sleep(BATCH_INTERVAL);
                }
                Some(path)
            },
            |batch| {
                flushed_at.push((visited.get(), batch.len()));
                true
            },
        );
        assert_eq!(outcome, Outcome::Complete);
        assert!(flushed_at[0].0 <= 2);
        assert_eq!(flushed_at.iter().map(|(_, len)| len).sum::<usize>(), 3);
    }

    #[test]
    fn cancellation_is_observed_inside_one_large_directory() {
        let tree = Tree::new();
        tree.files(BATCH_SIZE * 3);
        let cancel = gio::Cancellable::new();
        let mut visited = 0;
        let mut emitted = 0;
        let outcome = scan(
            &tree.0,
            "match",
            false,
            &cancel,
            |path| {
                visited += 1;
                if visited == 13 {
                    cancel.cancel();
                }
                Some(path)
            },
            |_| {
                emitted += 1;
                true
            },
        );
        assert_eq!(outcome, Outcome::Cancelled);
        assert_eq!(visited, 13);
        assert_eq!(emitted, 0, "cancelled pending hits must be discarded");
        assert_eq!(
            scan(
                &tree.0,
                "match",
                false,
                &cancel,
                |_| -> Option<PathBuf> { panic!("cancelled query read metadata") },
                |_| false
            ),
            Outcome::Cancelled
        );
    }

    #[test]
    fn consumer_shutdown_stops_enumeration_at_the_next_batch() {
        let tree = Tree::new();
        tree.files(BATCH_SIZE * 3);
        let mut visited = 0;
        let outcome = scan(
            &tree.0,
            "match",
            false,
            &gio::Cancellable::new(),
            |path| {
                visited += 1;
                Some(path)
            },
            |_| false,
        );
        assert_eq!(outcome, Outcome::Cancelled);
        assert!(visited <= BATCH_SIZE);
    }

    #[test]
    fn cancellation_unblocks_a_full_result_queue_and_rejects_late_batches() {
        let (tx, rx) = mpsc::sync_channel(1);
        tx.send(1).unwrap();
        let cancel = gio::Cancellable::new();
        let worker_cancel = cancel.clone();
        let (ready_tx, ready_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            ready_tx.send(()).unwrap();
            send(&tx, 2, &worker_cancel)
        });
        ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        cancel.cancel();
        assert!(!worker.join().unwrap());
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), [1]);
    }

    #[test]
    fn hidden_setting_controls_descent_and_symlink_loops_stay_leaves() {
        let tree = Tree::new();
        std::fs::create_dir(tree.0.join(".hidden")).unwrap();
        std::fs::create_dir(tree.0.join("backup~")).unwrap();
        std::fs::create_dir(tree.0.join("visible")).unwrap();
        for name in [
            ".hidden/match.txt",
            "backup~/match.txt",
            "visible/match.txt",
            ".match",
            "match~",
        ] {
            std::fs::File::create(tree.0.join(name)).unwrap();
        }
        std::os::unix::fs::symlink(&tree.0, tree.0.join("visible/loop")).unwrap();
        assert_eq!(
            collect(&tree, "match", false)
                .1
                .into_iter()
                .flatten()
                .count(),
            1
        );
        assert_eq!(
            collect(&tree, "match", true)
                .1
                .into_iter()
                .flatten()
                .count(),
            5
        );
    }

    #[test]
    fn metadata_survives_worker_transfer_without_a_second_filesystem_read() {
        use std::os::unix::ffi::OsStrExt;
        let tree = Tree::new();
        let path = tree.0.join(std::ffi::OsStr::from_bytes(b"R\xffport.txt"));
        std::fs::write(&path, "hello").unwrap();
        let cancel = gio::Cancellable::new();
        let expected = gio::File::for_path(&path)
            .query_info(
                crate::pane::ATTRS,
                gio::FileQueryInfoFlags::NONE,
                Some(&cancel),
            )
            .unwrap();
        let worker_path = path.clone();
        let hit = std::thread::spawn(move || {
            Hit::read(worker_path, crate::pane::ATTRS, false, &cancel).unwrap()
        })
        .join()
        .unwrap();
        std::fs::remove_file(&path).unwrap();
        let info = hit.into_info();
        assert_eq!(info.name(), expected.name());
        assert_eq!(info.display_name(), expected.display_name());
        assert_eq!(info.size(), 5);
        assert_eq!(info.file_type(), expected.file_type());
        assert!(info.icon().unwrap().equal(expected.icon().as_ref()));
        assert_eq!(crate::pane::file_of(&info).unwrap().path().unwrap(), path);
        for name in crate::pane::ATTRS
            .split(',')
            .filter(|name| *name != "standard::icon")
        {
            assert_eq!(
                info.attribute_type(name),
                expected.attribute_type(name),
                "{name}"
            );
            // Raw filenames need byte comparisons rather than UTF-8 conversion.
            if expected.attribute_type(name) != gio::FileAttributeType::ByteString {
                assert_eq!(
                    info.attribute_as_string(name),
                    expected.attribute_as_string(name),
                    "{name}"
                );
            }
        }
    }

    #[test]
    fn flat_50k_tree_enforces_the_cap_during_rapid_query_changes() {
        let tree = Tree::new();
        tree.files(50_000);
        let (outcome, batches) = collect(&tree, "match", false);
        assert_eq!(outcome, Outcome::Limited);
        assert_eq!(batches.iter().map(Vec::len).sum::<usize>(), RESULT_LIMIT);
        assert!(batches.iter().all(|batch| batch.len() <= BATCH_SIZE));
        // Simulate replacing a query as soon as its first results arrive. Each
        // obsolete query stops within this same flat directory, with no tail.
        for query in ["match-0", "match-00", "match-0", "match-", "match"] {
            let cancel = gio::Cancellable::new();
            let mut emitted = 0;
            let outcome = scan(&tree.0, query, false, &cancel, Some, |batch| {
                emitted += batch.len();
                cancel.cancel();
                true
            });
            assert_eq!(outcome, Outcome::Cancelled);
            assert!(emitted > 0 && emitted <= BATCH_SIZE);
        }
    }
}
