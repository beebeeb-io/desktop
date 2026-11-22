//! `scan` items split out of `watcher.rs`.

use super::*;

/// Task 2229: a file as the directory listing shows it. On Windows the listing carries all of it
/// (`DirEntry::metadata` opens no handle), where a stat opens one: ~40 µs each on a Cloud Files
/// sync root. A file still open for writing may list its old size and time until it is closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiskStamp {
    pub(super) len: u64,
    pub(super) modified: Option<std::time::SystemTime>,
    #[cfg(target_os = "windows")]
    pub(super) attributes: u32,
    /// Task 2241 (Windows): `ChangeTime` and the 128-bit file id, read through a handle opened for no
    /// data ([`DiskStamp::with_identity`]). The listing alone may show an old size and time for a file still open
    /// for writing; the handle shows the live values. `None` when the file could not be opened.
    pub(super) identity: Option<StampIdentity>,
    #[cfg(unix)]
    pub(super) inode_and_change: (u64, i64, i64),
}

impl DiskStamp {
    pub fn of(meta: &std::fs::Metadata) -> Self {
        Self {
            len: meta.len(),
            modified: meta.modified().ok(),
            #[cfg(target_os = "windows")]
            attributes: std::os::windows::fs::MetadataExt::file_attributes(meta),
            identity: None,
            #[cfg(unix)]
            inode_and_change: {
                use std::os::unix::fs::MetadataExt;
                (meta.ino(), meta.ctime(), meta.ctime_nsec())
            },
        }
    }
}

/// `(ChangeTime, volume serial, file id)`; read on Windows only.
pub type StampIdentity = (i64, u64, [u8; 16]);

impl DiskStamp {
    /// The live identity of `path` (Windows: one attribute-only open; elsewhere `None`).
    pub fn default_identity(path: &std::path::Path) -> Option<StampIdentity> {
        identity_at(path)
    }

    /// `Ok(())` when the file is unchanged since `seen`; otherwise the stamp to remember for it.
    /// Task 2241: `read` (the attribute-only open) runs only when the listing (size, time, attributes)
    /// already matches `seen`, a skip candidate; a changed file costs no open and its stamp is the
    /// listing alone. The identity `read` returned is kept in the returned stamp (no second open).
    pub fn check(&self, seen: &DiskStamp, read: impl FnOnce() -> Option<StampIdentity>) -> Result<(), DiskStamp> {
        self.check_on(seen, cfg!(target_os = "windows"), read)
    }

    /// [`DiskStamp::check`] for a platform that does (`has_identity`) or does not supply an identity.
    /// Where it does, a stored stamp without one never counts as unchanged: it is read once and refilled.
    pub(super) fn check_on(
        &self,
        seen: &DiskStamp,
        has_identity: bool,
        read: impl FnOnce() -> Option<StampIdentity>,
    ) -> Result<(), DiskStamp> {
        let listing = |s: &DiskStamp| DiskStamp { identity: None, ..*s };
        if listing(self) != listing(seen) {
            return Err(*self);
        }
        let identity = read();
        if identity == seen.identity && (seen.identity.is_some() || !has_identity) {
            Ok(())
        } else {
            Err(DiskStamp { identity, ..*self })
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn identity_at(_path: &std::path::Path) -> Option<StampIdentity> {
    None
}

#[cfg(target_os = "windows")]
fn identity_at(path: &std::path::Path) -> Option<StampIdentity> {
    windows_identity(path)
}

/// `(ChangeTime, volume serial, file id)` of `path` through a no-data handle that never follows a
/// reparse point; `None` when it cannot be opened or read.
#[cfg(target_os = "windows")]
fn windows_identity(path: &std::path::Path) -> Option<StampIdentity> {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        FILE_BASIC_INFO, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, FileBasicInfo, FileIdInfo, GetFileInformationByHandleEx,
    };
    let file = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES.0)
        .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)
        .ok()?;
    let handle = HANDLE(file.as_raw_handle());
    let mut basic = FILE_BASIC_INFO::default();
    let mut id = FILE_ID_INFO::default();
    // SAFETY: each struct is the one its info class names and its size is passed; `handle` is open for the calls.
    unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileBasicInfo,
            (&mut basic as *mut FILE_BASIC_INFO).cast(),
            std::mem::size_of::<FILE_BASIC_INFO>() as u32,
        )
        .ok()?;
        GetFileInformationByHandleEx(
            handle,
            FileIdInfo,
            (&mut id as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
        .ok()?;
    }
    Some((basic.ChangeTime, id.VolumeSerialNumber, id.FileId.Identifier))
}

/// Task 2229: the files a pass found settled or held ([`ScanDispatch::Settled`]), with the disk stamp
/// and the row stamp seen before classifying them. A later pass skips a file whose two stamps
/// are both unchanged. The scan used to classify every file on every pass (about five handle
/// opens each), which kept a core busy on a 37k-file sync root. Cleared every
/// [`FULL_RECHECK_INTERVAL`].
#[derive(Default)]
pub struct ScanMemo {
    pub(super) settled: HashMap<PathBuf, (DiskStamp, u64)>,
    /// R3: untracked placeholders recorded once for the mark retry, with the listing seen then; a later
    /// pass with the same listing does not classify them again.
    pub(super) untracked: HashMap<PathBuf, DiskStamp>,
    pub(super) since: Option<Instant>,
    /// Files classified by the last pass.
    pub classified: usize,
    /// Files the last pass skipped as settled and unchanged.
    pub skipped: usize,
    /// dc-166: folders and files the last pass looked at (0: it opened nothing).
    pub visited: usize,
    /// dc-166: files the last pass queued nothing for and must ask again (a change feed would not name them).
    pub(super) retry: std::collections::HashSet<PathBuf>,
    /// dc-166: the last pass left work only a full walk finds (the per-pass upload cap, a folder not scaffolded yet).
    pub(super) owe_full: bool,
    /// dc-166: the enabled backup folders the last pass saw; a change (a folder enabled again) walks everything.
    pub(super) backup_keys: Option<std::collections::HashSet<String>>,
}

#[cfg(any(test, feature = "test-support"))]
impl ScanMemo {
    /// As if the last full pass ran `by` earlier.
    pub fn age_by(&mut self, by: Duration) {
        self.since = self.since.and_then(|since| since.checked_sub(by));
    }
}

/// The CREATE trigger. Every [`SCAN_INTERVAL`] (debounced by awaiting each walk
/// before scheduling the next), recursively walk `sync_root`, run the shared
/// [`EngineBridge::classify_local_path`] on every regular file not found settled
/// with the same listing and row before ([`ScanMemo`]), and upload the
/// survivors (genuinely-new plain files). This is what catches a file a foreign
/// process drops into the sync root — the case the CF NOTIFY callbacks miss
/// entirely (see module docs).
///
/// Why a scan and not just the callbacks: reading the disk always works; it does
/// not depend on any filter callback firing. The cost is a recursive `read_dir`
/// every 8s plus one indexed row read per file: a file found settled is skipped
/// while its listing and its row are unchanged ([`ScanMemo`], task 2229 — classifying
/// all 37k files of one sync root on every pass kept a core busy). The heavy lifting
/// (the walk + classify + the synchronous `queue_finder_create`, which touches the
/// SQLite DB and stages bytes) runs inside [`tokio::task::spawn_blocking`] so it
/// never stalls the async runtime.
///
/// Dedupe: `queue_finder_create` writes the file's `Uploading` row
/// synchronously before it returns, so a file queued by scan N is a tracked
/// file for scan N+1: `classify_local_path` takes it to `classify_tracked_edit`,
/// which queues nothing while its upload is queued and, once it landed, only a
/// change of its bytes (task 1640). Task 1640 (follow-up): there used to be an
/// `in_flight` set of the paths a scan had queued, skipped until they left the
/// disk. Since the row is written before the queue call returns, it guarded no
/// window, but it kept a just-uploaded file from ever being classified again, so
/// an atomic save of it (no change notification) waited for a restart. Removed.
pub(super) async fn scan_loop(
    bridge: Arc<EngineBridge>,
    sync_root: PathBuf,
    mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
) {
    let mut memo = ScanMemo::default();
    // dc-166: opened on the first pass (blocking I/O), kept across passes.
    let mut feed: Option<Box<dyn super::change_feed::ChangeFeed>> = None;
    loop {
        // Wait one interval OR a shutdown signal. We sleep FIRST so the very
        // first scan happens one interval after startup — by then the initial
        // `seed_placeholders` + first sync tick have run, so the DB rows that
        // filter-3 needs to suppress just-downloaded files are already present.
        tokio::select! {
            biased;
            _ = &mut shutdown_rx => {
                tracing::debug!("enumeration scan loop shutting down");
                break;
            }
            _ = tokio::time::sleep(SCAN_INTERVAL) => {}
        }

        // Run the blocking walk + classify + queue off the async runtime.
        // `bridge`/`sync_root` are cheap to clone (Arc + PathBuf).
        let bridge_for_scan = bridge.clone();
        let root_for_scan = sync_root.clone();
        let mut memo_for_scan = std::mem::take(&mut memo);
        let feed_for_scan = feed.take();
        match tokio::task::spawn_blocking(move || {
            let mut feed = feed_for_scan.unwrap_or_else(|| super::change_feed::open(&root_for_scan));
            scan_tick(&bridge_for_scan, &root_for_scan, &mut memo_for_scan, feed.as_mut());
            (memo_for_scan, feed)
        })
        .await
        {
            Ok((kept, kept_feed)) => {
                memo = kept;
                feed = Some(kept_feed);
            }
            // The blocking task panicked — log and continue. Never let one bad
            // scan kill the loop. The memo and the feed are lost: the next pass classifies every file.
            Err(e) => tracing::warn!(error = %e, "enumeration scan task panicked; continuing"),
        }
    }
}

/// dc-166: one tick of [`scan_loop`]. A full walk when the backstop is due ([`FULL_RECHECK_INTERVAL`]),
/// when the last pass owes one, when the enabled backup folders changed, or when `feed` cannot say what
/// changed; otherwise only what `feed` named plus the files left to ask again, and, when that is nothing,
/// no pass at all: an idle tick opens no file under the sync root.
pub fn scan_tick(
    bridge: &EngineBridge,
    sync_root: &std::path::Path,
    memo: &mut ScanMemo,
    feed: &mut dyn super::change_feed::ChangeFeed,
) {
    use super::change_feed::Changes;
    // Taken before the walk, so a change made while it runs is named next tick.
    let changes = feed.take();
    let backstop = memo.since.is_none_or(|since| since.elapsed() >= FULL_RECHECK_INTERVAL);
    let full = backstop || memo.owe_full || changes == Changes::Unknown || backup_keys_changed(bridge, memo);
    if full {
        return run_scan(bridge, sync_root, memo, None);
    }
    let mut paths = match changes {
        Changes::Paths(paths) => paths,
        _ => std::collections::BTreeSet::new(),
    };
    paths.extend(memo.retry.drain());
    if paths.is_empty() {
        memo.classified = 0;
        memo.skipped = 0;
        memo.visited = 0;
        return;
    }
    run_scan(bridge, sync_root, memo, Some(&paths));
}

/// The enabled backup folders differ from the ones the last pass saw (one config read, outside the sync root).
fn backup_keys_changed(bridge: &EngineBridge, memo: &ScanMemo) -> bool {
    let now: Option<std::collections::HashSet<String>> = crate::config::DesktopConfig::load_for(bridge.identity)
        .ok()
        .map(|cfg| cfg.known_folder_backup.iter().cloned().collect());
    now != memo.backup_keys
}

/// dc-166: visit each of `paths` (sorted, so a folder comes before what is in it): a folder is
/// scaffolded and walked, a file handed to `on_file`; a path gone, outside the root or engine-internal
/// is passed over (deletes ride the Cloud Files callbacks).
pub(super) fn visit_paths(
    sync_root: &std::path::Path,
    paths: &std::collections::BTreeSet<PathBuf>,
    on_dir: &mut dyn FnMut(&std::path::Path),
    on_file: &mut dyn FnMut(&std::path::Path, &std::fs::Metadata),
) {
    let mut walked: Option<&std::path::Path> = None;
    for path in paths {
        if walked.is_some_and(|dir| path.starts_with(dir)) {
            continue;
        }
        let Ok(rel) = path.strip_prefix(sync_root) else {
            continue;
        };
        if rel.as_os_str().is_empty() || crate::engine_bridge::path_is_engine_internal(sync_root, path) {
            continue;
        }
        if path
            .ancestors()
            .skip(1)
            .take_while(|a| *a != sync_root)
            .any(|a| crate::engine_bridge::path_is_engine_internal(sync_root, a))
        {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(path) else {
            continue;
        };
        let ft = meta.file_type();
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            on_dir(path);
            walk_dir(sync_root, path, rel.components().count(), on_dir, on_file);
            walked = Some(path);
        } else if ft.is_file() {
            on_file(path, &meta);
        }
    }
}

/// Depth-first recursive walk of `dir`, invoking `on_file` for every regular
/// FILE entry. Skips engine-internal paths (`.beebeeb/`, the lock file) via the
/// shared [`crate::engine_bridge::fs`]'s `path_is_engine_internal` guard, never follows
/// symlinks, and is bounded by [`MAX_SCAN_DEPTH`].
///
/// Errors (an unreadable directory, a racing delete) are logged at trace and
/// skipped — a scan must never abort the whole walk for one bad entry; the next
/// scan retries. Pure `std::fs`, so it works on every platform (the scan task is
/// only spawned on Windows, but keeping the walk platform-agnostic means it
/// type-checks and unit-tests everywhere).
pub(super) fn walk_dir(
    sync_root: &std::path::Path,
    dir: &std::path::Path,
    depth: usize,
    on_dir: &mut dyn FnMut(&std::path::Path),
    on_file: &mut dyn FnMut(&std::path::Path, &std::fs::Metadata),
) {
    if depth > MAX_SCAN_DEPTH {
        tracing::warn!(depth, "enumeration scan hit max depth; not descending further");
        return;
    }

    // Skip the whole engine-internal subtree up front (`.beebeeb/` etc.) so we
    // never even read its children — cheaper and impossible to misclassify.
    if crate::engine_bridge::path_is_engine_internal(sync_root, dir) {
        tracing::debug!("enumeration scan: skipped engine-internal directory");
        return;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::debug!(
                error = %e,
                "enumeration scan: read_dir failed; skipping directory"
            );
            return;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                tracing::debug!(
                    error = %e,
                    "enumeration scan: dir entry error; skipping"
                );
                continue;
            }
        };
        let path = entry.path();

        // Engine-internal guard per entry too (the lock file at the root, a
        // nested `.beebeeb`). Cheap and defensive.
        if crate::engine_bridge::path_is_engine_internal(sync_root, &path) {
            tracing::debug!("enumeration scan: skipped engine-internal entry");
            continue;
        }

        // The listing's own metadata, which never follows a link, so a symlink is
        // classified as a symlink (we skip it) rather than followed — no
        // symlink-cycle risk, and we never upload through a link to outside the
        // vault. Task 2229: on Windows it opens no handle, where a stat opened one
        // per entry on every pass.
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(e) => {
                tracing::debug!(
                    error = %e,
                    "enumeration scan: stat failed; skipping entry"
                );
                continue;
            }
        };
        let ft = meta.file_type();
        if ft.is_symlink() {
            tracing::debug!("enumeration scan: skipped symlink");
            continue;
        }
        if ft.is_dir() {
            // PRE-ORDER: ensure this directory's vault folder exists BEFORE we
            // descend, so a nested child resolves its parent (task 0811).
            on_dir(&path);
            walk_dir(sync_root, &path, depth + 1, on_dir, on_file);
        } else if ft.is_file() {
            on_file(&path, &meta);
        } else {
            tracing::debug!("enumeration scan: skipped non-file entry");
        }
        // Other entry kinds (sockets, devices) are ignored.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Zero-knowledge: the non-test text of the scan, the watcher and the classifier logs no path or name.
    #[test]
    fn scan_and_classify_logs_carry_no_paths() {
        for (name, src) in [
            ("watcher.rs", include_str!("../watcher.rs")),
            ("watcher/scan.rs", include_str!("scan.rs")),
            (
                "engine_bridge/local_classify.rs",
                include_str!("../engine_bridge/local_classify.rs"),
            ),
        ] {
            let production = src.split("#[cfg(test)]").next().unwrap();
            for (n, line) in production.lines().enumerate() {
                let t = line.trim();
                if t.starts_with("//") {
                    continue;
                }
                for bad in [
                    "path = %",
                    "path = ?",
                    "path=%",
                    "filename = %",
                    "dir.display()",
                    "path.display()",
                ] {
                    assert!(!t.contains(bad), "{name}:{}: log carries a path: {t}", n + 1);
                }
            }
        }
    }

    fn stamp_of(path: &std::path::Path) -> DiskStamp {
        DiskStamp::of(&std::fs::metadata(path).unwrap())
    }

    /// Task 2241: a changed file costs 0 opens (listing-only stamp); an unchanged candidate exactly 1,
    /// whose value is kept; a Windows-style platform never skips a stamp stored without identity.
    #[test]
    fn identity_is_read_only_for_skip_candidates() {
        let dir = std::env::temp_dir().join(format!("bb-stamp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("a.bin");
        std::fs::write(&f, b"one").unwrap();
        let id = Some((7, 8, [9u8; 16]));
        let mut seen = stamp_of(&f);
        seen.identity = id;
        let reads = std::cell::Cell::new(0);
        let read = |v| {
            reads.set(reads.get() + 1);
            v
        };
        // Unchanged candidate: exactly one open, skipped.
        assert_eq!(stamp_of(&f).check_on(&seen, true, || read(id)), Ok(()));
        assert_eq!(reads.get(), 1);
        // Candidate with another identity: one open, its value kept.
        let other = Some((8, 8, [9u8; 16]));
        let got = stamp_of(&f).check_on(&seen, true, || read(other));
        assert_eq!(got.unwrap_err().identity, other);
        assert_eq!(reads.get(), 2);
        // Stored without identity: on an identity platform never unchanged, refilled once; elsewhere skipped.
        let mut bare = seen;
        bare.identity = None;
        assert_eq!(
            stamp_of(&f).check_on(&bare, true, || read(id)).unwrap_err().identity,
            id
        );
        assert_eq!(stamp_of(&f).check_on(&bare, false, || read(None)), Ok(()));
        assert_eq!(reads.get(), 4);
        // Changed file: no open, listing-only stamp.
        std::fs::write(&f, b"longer bytes").unwrap();
        let got = stamp_of(&f).check_on(&seen, true, || read(id));
        assert_eq!(got.unwrap_err().identity, None);
        assert_eq!(reads.get(), 4, "a changed file must not be opened");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// macOS/Linux supply no identity: an unchanged file is still skipped there.
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn unchanged_file_is_skipped_without_identity() {
        let dir = std::env::temp_dir().join(format!("bb-stamp2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("a.bin");
        std::fs::write(&f, b"one").unwrap();
        let seen = stamp_of(&f);
        assert_eq!(stamp_of(&f).check(&seen, || DiskStamp::default_identity(&f)), Ok(()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
