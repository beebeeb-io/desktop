//! `scan` items split out of `watcher.rs`.

use super::*;

/// Task 2229: a file as the directory listing shows it. On Windows the listing carries all of it
/// (`DirEntry::metadata` opens no handle), where a stat opens one: ~40 µs each on a Cloud Files
/// sync root. A file still open for writing may list its old size and time until it is closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DiskStamp {
    pub(super) len: u64,
    pub(super) modified: Option<std::time::SystemTime>,
    #[cfg(target_os = "windows")]
    pub(super) attributes: u32,
    #[cfg(unix)]
    pub(super) inode_and_change: (u64, i64, i64),
}

impl DiskStamp {
    pub(crate) fn of(meta: &std::fs::Metadata) -> Self {
        Self {
            len: meta.len(),
            modified: meta.modified().ok(),
            #[cfg(target_os = "windows")]
            attributes: std::os::windows::fs::MetadataExt::file_attributes(meta),
            #[cfg(unix)]
            inode_and_change: {
                use std::os::unix::fs::MetadataExt;
                (meta.ino(), meta.ctime(), meta.ctime_nsec())
            },
        }
    }
}

/// Task 2229: the files a pass found settled or held ([`ScanDispatch::Settled`]), with the disk stamp
/// and the row stamp seen before classifying them. A later pass skips a file whose two stamps
/// are both unchanged. The scan used to classify every file on every pass (about five handle
/// opens each), which kept a core busy on a 37k-file sync root. Cleared every
/// [`FULL_RECHECK_INTERVAL`].
#[derive(Default)]
pub(crate) struct ScanMemo {
    pub(super) settled: HashMap<PathBuf, (DiskStamp, u64)>,
    pub(super) since: Option<Instant>,
    /// Files classified by the last pass.
    pub(crate) classified: usize,
    /// Files the last pass skipped as settled and unchanged.
    pub(crate) skipped: usize,
}

#[cfg(test)]
impl ScanMemo {
    /// As if the last full pass ran `by` earlier.
    pub(crate) fn age_by(&mut self, by: Duration) {
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
        match tokio::task::spawn_blocking(move || {
            run_one_scan_with(&bridge_for_scan, &root_for_scan, &mut memo_for_scan);
            memo_for_scan
        })
        .await
        {
            Ok(kept) => memo = kept,
            // The blocking task panicked — log and continue. Never let one bad
            // scan kill the loop. The memo is lost: the next pass classifies every file.
            Err(e) => tracing::warn!(error = %e, "enumeration scan task panicked; continuing"),
        }
    }
}

/// Depth-first recursive walk of `dir`, invoking `on_file` for every regular
/// FILE entry. Skips engine-internal paths (`.beebeeb/`, the lock file) via the
/// shared [`crate::engine_bridge::path_is_engine_internal`] guard, never follows
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
        tracing::debug!(
            path = %dir.display(),
            "enumeration scan: skipped engine-internal directory"
        );
        return;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::debug!(
                path = %dir.display(),
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
                    path = %dir.display(),
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
            tracing::debug!(
                path = %path.display(),
                "enumeration scan: skipped engine-internal entry"
            );
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
                    path = %path.display(),
                    error = %e,
                    "enumeration scan: stat failed; skipping entry"
                );
                continue;
            }
        };
        let ft = meta.file_type();
        if ft.is_symlink() {
            tracing::debug!(
                path = %path.display(),
                "enumeration scan: skipped symlink"
            );
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
            tracing::debug!(
                path = %path.display(),
                "enumeration scan: skipped non-file entry"
            );
        }
        // Other entry kinds (sockets, devices) are ignored.
    }
}
