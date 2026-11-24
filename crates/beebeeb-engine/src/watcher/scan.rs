//! `scan` items split out of `watcher.rs`.

use super::*;

/// Task 2229: a file as the directory listing shows it. On Windows the listing carries all of it
/// ([`list_dir`] opens the folder, never a file), where a stat opens one: ~40 µs each on a Cloud Files
/// sync root. A file still open for writing may list its old size and times until it is closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiskStamp {
    pub(super) len: u64,
    pub(super) modified: Option<std::time::SystemTime>,
    #[cfg(target_os = "windows")]
    pub(super) attributes: u32,
    /// Task 2241 (Windows): `ChangeTime` and the file id, so a replacement that keeps size and time, and an
    /// equal-length rewrite with the time set back, are another stamp. RC-d: they come with the folder's
    /// listing ([`list_dir`]), which opens no file; 113 read them through one open per unchanged file on
    /// every pass (12,300-14,564 file operations a second at 41,510 files). Only a path named outside a
    /// listing (the change journal's) is opened for them ([`DiskStamp::of_path`]). `None`: not read.
    pub(super) identity: Option<StampIdentity>,
    #[cfg(unix)]
    pub(super) inode_and_change: (u64, i64, i64),
}

/// `(ChangeTime, file id)`; read on Windows only.
pub type StampIdentity = (i64, u64);

impl DiskStamp {
    /// The stamp `meta` alone gives: on Windows without the identity.
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

    /// The stamp of one path that no listing was read for (a path the change journal named): `meta` plus,
    /// on Windows, one attribute-only open for the identity. Never called by the walk.
    pub fn of_path(path: &std::path::Path, meta: &std::fs::Metadata) -> Self {
        Self {
            identity: identity_at(path),
            ..Self::of(meta)
        }
    }

    /// The file is as `seen` recorded it. On Windows a stamp without its identity is never unchanged.
    pub fn unchanged_since(&self, seen: &DiskStamp) -> bool {
        self.unchanged_on(seen, cfg!(target_os = "windows"))
    }

    /// [`DiskStamp::unchanged_since`] for a platform that does (`has_identity`) or does not supply an identity.
    pub(super) fn unchanged_on(&self, seen: &DiskStamp, has_identity: bool) -> bool {
        self == seen && (self.identity.is_some() || !has_identity)
    }
}

#[cfg(not(target_os = "windows"))]
fn identity_at(_path: &std::path::Path) -> Option<StampIdentity> {
    None
}

/// `(ChangeTime, file id)` of `path` through a no-data handle that never follows a reparse point, the same
/// two values its folder's listing carries; `None` when it cannot be opened or read.
#[cfg(target_os = "windows")]
fn identity_at(path: &std::path::Path) -> Option<StampIdentity> {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        FileBasicInfo, GetFileInformationByHandle, GetFileInformationByHandleEx, BY_HANDLE_FILE_INFORMATION,
        FILE_BASIC_INFO, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE,
    };
    let file = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES.0)
        .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)
        .ok()?;
    let handle = HANDLE(file.as_raw_handle());
    let mut basic = FILE_BASIC_INFO::default();
    let mut index = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: each struct is the one its call fills and `basic`'s size is passed; `handle` is open for the calls.
    unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileBasicInfo,
            (&mut basic as *mut FILE_BASIC_INFO).cast(),
            std::mem::size_of::<FILE_BASIC_INFO>() as u32,
        )
        .ok()?;
        GetFileInformationByHandle(handle, &mut index).ok()?;
    }
    Some((
        basic.ChangeTime,
        (u64::from(index.nFileIndexHigh) << 32) | u64::from(index.nFileIndexLow),
    ))
}

/// RE-e (11-24: 12 POST /files/folder on prod right after the first load): whether the folder at `path`, as its
/// parent's listing (or, for a path the change journal named, its own metadata) shows it in `stamp`, is a Cloud Files
/// placeholder. Only the engine mints one, from a server row; a folder made on this PC is a plain folder
/// (`a_folder_made_on_this_pc_gets_no_cloud_files_identity`), and a link is listed as one and never reaches here.
/// Read from the listing: no folder is opened for it.
pub(super) fn is_placeholder_dir(path: &std::path::Path, stamp: &DiskStamp) -> bool {
    #[cfg(test)]
    if placeholder_dirs_for_tests::marked(path) {
        return true;
    }
    #[cfg(target_os = "windows")]
    {
        let _ = path;
        const REPARSE_POINT: u32 = 0x400;
        stamp.attributes & REPARSE_POINT != 0
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (path, stamp);
        false
    }
}

/// Tests off Windows, where no folder is a placeholder: the folders a test marks as minted from a server row. Per
/// thread: a scan pass runs on the test's own thread.
#[cfg(test)]
pub(crate) mod placeholder_dirs_for_tests {
    use std::cell::RefCell;
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};

    thread_local! {
        static MARKED: RefCell<HashSet<PathBuf>> = RefCell::new(HashSet::new());
    }

    /// Mark `path` as a placeholder folder for this thread's scans.
    pub(crate) fn mark(path: &Path) {
        MARKED.with(|m| m.borrow_mut().insert(path.to_path_buf()));
    }

    pub(super) fn marked(path: &Path) -> bool {
        MARKED.with(|m| m.borrow().contains(path))
    }
}

/// What a folder's listing says an entry is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ListedKind {
    Dir,
    File,
    /// A symlink or a junction: never followed, never sent.
    Link,
    /// A socket, a device.
    #[cfg_attr(target_os = "windows", allow(dead_code))] // the Windows listing has no such kind
    Other,
}

/// One entry of a folder, from its listing alone.
#[derive(Debug)]
pub(super) struct Listed {
    pub(super) name: std::ffi::OsString,
    pub(super) kind: ListedKind,
    pub(super) stamp: DiskStamp,
}

/// The entries of `dir` with each one's [`DiskStamp`], reading the folder only.
#[cfg(not(target_os = "windows"))]
pub(super) fn list_dir(dir: &std::path::Path) -> std::io::Result<Vec<Listed>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        // One bad entry (a racing delete) never ends the listing; the next pass sees it again.
        let Ok(entry) = entry else {
            tracing::debug!("enumeration scan: dir entry error; skipping");
            continue;
        };
        // The entry's own metadata never follows a link, so a symlink is listed as one.
        let Ok(meta) = entry.metadata() else {
            tracing::debug!("enumeration scan: stat failed; skipping entry");
            continue;
        };
        let ft = meta.file_type();
        let kind = if ft.is_symlink() {
            ListedKind::Link
        } else if ft.is_dir() {
            ListedKind::Dir
        } else if ft.is_file() {
            ListedKind::File
        } else {
            ListedKind::Other
        };
        out.push(Listed {
            name: entry.file_name(),
            kind,
            stamp: DiskStamp::of(&meta),
        });
    }
    Ok(out)
}

/// The entries of `dir` with each one's [`DiskStamp`], identity included, reading the folder only: one
/// folder handle and `FileIdBothDirectoryInfo` (what `FindNextFile` costs; task 2241's own plan). The
/// kinds are the ones `std` tells from the same listing: a reparse point with a name-surrogate tag is a
/// link, a Cloud Files placeholder is a file or a folder.
#[cfg(target_os = "windows")]
pub(super) fn list_dir(dir: &std::path::Path) -> std::io::Result<Vec<Listed>> {
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::{ERROR_NO_MORE_FILES, HANDLE};
    use windows::Win32::Storage::FileSystem::{
        FileIdBothDirectoryInfo, FileIdBothDirectoryRestartInfo, GetFileInformationByHandleEx,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_LIST_DIRECTORY, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    const DIRECTORY: u32 = 0x10;
    const REPARSE_POINT: u32 = 0x400;
    /// The reparse tag bit of a symlink or a junction (what `std` calls a symlink).
    const NAME_SURROGATE: u32 = 0x2000_0000;
    /// `FILE_ID_BOTH_DIR_INFO` up to `FileName`: NextEntryOffset 0, LastWriteTime 24, ChangeTime 32,
    /// EndOfFile 40, FileAttributes 56, FileNameLength 60, EaSize (the reparse tag of a reparse point) 64,
    /// FileId 96.
    const HEADER: usize = 104;
    let folder = std::fs::OpenOptions::new()
        .access_mode(FILE_LIST_DIRECTORY.0)
        .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0)
        .open(dir)?;
    // 64 KB, 8-byte aligned as the records are.
    let mut words = vec![0u64; 8 * 1024];
    let size = words.len() * 8;
    let mut out = Vec::new();
    let mut class = FileIdBothDirectoryRestartInfo;
    loop {
        // SAFETY: `words` is `size` writable bytes and outlives the synchronous call; `folder` is open.
        let read = unsafe {
            GetFileInformationByHandleEx(
                HANDLE(folder.as_raw_handle()),
                class,
                words.as_mut_ptr().cast(),
                size as u32,
            )
        };
        if let Err(e) = read {
            if e.code() == ERROR_NO_MORE_FILES.to_hresult() {
                return Ok(out);
            }
            return Err(e.into());
        }
        class = FileIdBothDirectoryInfo;
        // SAFETY: the same live allocation of `size` bytes, read as bytes.
        let buf = unsafe { std::slice::from_raw_parts(words.as_ptr().cast::<u8>(), size) };
        let mut at = 0usize;
        while let Some(rec) = buf.get(at..).filter(|rec| rec.len() >= HEADER) {
            let u32_at = |o: usize| u32::from_le_bytes(rec[o..o + 4].try_into().unwrap());
            let i64_at = |o: usize| i64::from_le_bytes(rec[o..o + 8].try_into().unwrap());
            let Some(name) = rec.get(HEADER..HEADER + u32_at(60) as usize) else {
                break;
            };
            let name: Vec<u16> = name.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
            const DOT: u16 = b'.' as u16;
            if name != [DOT] && name != [DOT, DOT] {
                let attributes = u32_at(56);
                let kind = if attributes & REPARSE_POINT != 0 && u32_at(64) & NAME_SURROGATE != 0 {
                    ListedKind::Link
                } else if attributes & DIRECTORY != 0 {
                    ListedKind::Dir
                } else {
                    ListedKind::File
                };
                out.push(Listed {
                    name: std::ffi::OsString::from_wide(&name),
                    kind,
                    stamp: DiskStamp {
                        len: i64_at(40) as u64,
                        modified: filetime(i64_at(24)),
                        attributes,
                        identity: Some((i64_at(32), i64_at(96) as u64)),
                    },
                });
            }
            let next = u32_at(0) as usize;
            if next == 0 {
                break;
            }
            at += next;
        }
    }
}

/// A Windows file time (100 ns since 1601) as the `SystemTime` `std` makes of it.
#[cfg(target_os = "windows")]
fn filetime(ticks: i64) -> Option<std::time::SystemTime> {
    const UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;
    let since = ticks - UNIX_EPOCH_TICKS;
    let span = Duration::new(
        since.unsigned_abs() / 10_000_000,
        (since.unsigned_abs() % 10_000_000) as u32 * 100,
    );
    if since >= 0 {
        std::time::UNIX_EPOCH.checked_add(span)
    } else {
        std::time::UNIX_EPOCH.checked_sub(span)
    }
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
    use super::change_feed::{set_changes_pending, Gathered, GATHER_INTERVAL};
    let mut memo = ScanMemo::default();
    // dc-166: opened on the first pass (blocking I/O), kept across passes.
    let mut feed: Option<Gathered> = None;
    let mut last_tick = std::time::SystemTime::now();
    set_changes_pending(false);
    loop {
        // Wait one interval OR a shutdown signal. We sleep FIRST so the very
        // first scan happens one interval after startup — by then the initial
        // `seed_placeholders` + first sync tick have run, so the DB rows that
        // filter-3 needs to suppress just-downloaded files are already present.
        // RC5-1: meanwhile the feed is read every GATHER_INTERVAL and what it named is held for the scan, so the
        // status says a change is waiting from the moment the journal has it; the scan keeps its own interval
        // (a copied file has those seconds to finish landing).
        let scan_due = tokio::time::Instant::now() + SCAN_INTERVAL;
        loop {
            let wake = scan_due.min(tokio::time::Instant::now() + GATHER_INTERVAL);
            tokio::select! {
                biased;
                _ = &mut shutdown_rx => {
                    tracing::debug!("enumeration scan loop shutting down");
                    set_changes_pending(false);
                    return;
                }
                _ = tokio::time::sleep_until(wake) => {}
            }
            if tokio::time::Instant::now() >= scan_due {
                break;
            }
            let Some(mut gathering) = feed.take() else { continue };
            match tokio::task::spawn_blocking(move || {
                gathering.gather();
                gathering
            })
            .await
            {
                Ok(gathered) => {
                    set_changes_pending(gathered.named());
                    feed = Some(gathered);
                }
                // As for a scan: the feed is lost, the next pass opens a new one and walks everything.
                Err(e) => tracing::warn!(error = %e, "change feed read panicked; continuing"),
            }
        }

        // dc-174: a tick far later than the interval means the machine slept; the journal may have
        // wrapped meanwhile, so walk everything now rather than at the next hourly backstop.
        let now = std::time::SystemTime::now();
        if now.duration_since(last_tick).is_ok_and(|gap| gap >= SCAN_INTERVAL * 4) {
            memo.owe_full = true;
        }
        last_tick = now;

        // Run the blocking walk + classify + queue off the async runtime.
        // `bridge`/`sync_root` are cheap to clone (Arc + PathBuf).
        let bridge_for_scan = bridge.clone();
        let root_for_scan = sync_root.clone();
        let mut memo_for_scan = std::mem::take(&mut memo);
        let feed_for_scan = feed.take();
        match tokio::task::spawn_blocking(move || {
            let mut feed = feed_for_scan.unwrap_or_else(|| Gathered::new(super::change_feed::open(&root_for_scan)));
            scan_tick(&bridge_for_scan, &root_for_scan, &mut memo_for_scan, &mut feed);
            (memo_for_scan, feed)
        })
        .await
        {
            Ok((kept, kept_feed)) => {
                memo = kept;
                // RC5-1: what was waiting has been looked at (and queued) now; only what came in since still waits.
                set_changes_pending(kept_feed.named());
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
/// is passed over (deletes ride the Cloud Files callbacks). RC2-1: the sync root itself named means
/// "something in its top level" (a journal record with no name whose file is gone): its top-level files
/// are handed over from one listing, and no folder is walked for it.
pub(super) fn visit_paths(
    sync_root: &std::path::Path,
    paths: &std::collections::BTreeSet<PathBuf>,
    on_dir: &mut dyn FnMut(&std::path::Path, &DiskStamp),
    on_file: &mut dyn FnMut(&std::path::Path, &DiskStamp),
) {
    let mut walked: Option<&std::path::Path> = None;
    let mut top_level_listed = false;
    for path in paths {
        if walked.is_some_and(|dir| path.starts_with(dir)) {
            continue;
        }
        let Ok(rel) = path.strip_prefix(sync_root) else {
            continue;
        };
        if rel.as_os_str().is_empty() {
            if !top_level_listed {
                top_level_listed = true;
                list_top_level(sync_root, on_file);
            }
            continue;
        }
        if crate::engine_bridge::path_is_engine_internal(sync_root, path) {
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
            on_dir(path, &DiskStamp::of(&meta));
            walk_dir(sync_root, path, rel.components().count(), on_dir, on_file);
            walked = Some(path);
        } else if ft.is_file() {
            // The root sorts before everything in it: its listing handed this file over already.
            if top_level_listed && path.parent() == Some(sync_root) {
                continue;
            }
            // Named, so something happened to it: the one place a file is opened for its stamp.
            on_file(path, &DiskStamp::of_path(path, &meta));
        }
    }
}

/// RC2-1: the regular files directly in the sync root, each with its listing's stamp (no file is opened;
/// one the scan found settled and unchanged is skipped by its memo). Folders are left alone: a new or
/// changed one is named by its own journal record.
fn list_top_level(sync_root: &std::path::Path, on_file: &mut dyn FnMut(&std::path::Path, &DiskStamp)) {
    let Ok(entries) = list_dir(sync_root) else {
        tracing::debug!("enumeration scan: top-level listing failed");
        return;
    };
    for entry in entries {
        let path = sync_root.join(&entry.name);
        if entry.kind == ListedKind::File && !crate::engine_bridge::path_is_engine_internal(sync_root, &path) {
            on_file(&path, &entry.stamp);
        }
    }
}

/// Depth-first recursive walk of `dir`, invoking `on_file` for every regular
/// FILE entry with the stamp its folder's listing gave ([`list_dir`]: RC-d, no file is opened
/// here). Skips engine-internal paths (`.beebeeb/`, the lock file) via the
/// shared [`crate::engine_bridge::fs`]'s `path_is_engine_internal` guard, never follows
/// symlinks, and is bounded by [`MAX_SCAN_DEPTH`].
///
/// Errors (an unreadable directory, a racing delete) are logged at trace and
/// skipped — a scan must never abort the whole walk for one bad entry; the next
/// scan retries. It type-checks and unit-tests on every platform (the scan task is
/// only spawned on Windows).
pub(super) fn walk_dir(
    sync_root: &std::path::Path,
    dir: &std::path::Path,
    depth: usize,
    on_dir: &mut dyn FnMut(&std::path::Path, &DiskStamp),
    on_file: &mut dyn FnMut(&std::path::Path, &DiskStamp),
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

    let entries = match list_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::debug!(
                error = %e,
                "enumeration scan: listing failed; skipping directory"
            );
            return;
        }
    };

    for entry in entries {
        let path = dir.join(&entry.name);

        // Engine-internal guard per entry too (the lock file at the root, a
        // nested `.beebeeb`). Cheap and defensive.
        if crate::engine_bridge::path_is_engine_internal(sync_root, &path) {
            tracing::debug!("enumeration scan: skipped engine-internal entry");
            continue;
        }

        // The listing never follows a link, so a symlink is listed as one (we skip it)
        // rather than followed — no symlink-cycle risk, and we never upload through a
        // link to outside the vault.
        match entry.kind {
            ListedKind::Link => tracing::debug!("enumeration scan: skipped symlink"),
            ListedKind::Dir => {
                // PRE-ORDER: ensure this directory's vault folder exists BEFORE we
                // descend, so a nested child resolves its parent (task 0811).
                on_dir(&path, &entry.stamp);
                walk_dir(sync_root, &path, depth + 1, on_dir, on_file);
            }
            ListedKind::File => on_file(&path, &entry.stamp),
            // Other entry kinds (sockets, devices) are ignored.
            ListedKind::Other => tracing::debug!("enumeration scan: skipped non-file entry"),
        }
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

    /// RC-d: the walk decides "unchanged" from the folder's listing and never opens a file for it; only a
    /// path named outside a listing is opened. Pinned in the source, as an open cannot be counted here.
    #[test]
    fn the_walk_opens_no_file_for_its_stamp() {
        let source = include_str!("scan.rs").replace("\r\n", "\n");
        let production = source.split("#[cfg(test)]").next().unwrap();
        let walk = crate::source_pin::body_between(production, "pub(super) fn walk_dir(", "\n}\n");
        for opens in ["of_path(", "identity_at(", "metadata(", "OpenOptions", "File::open"] {
            assert!(!walk.contains(opens), "the walk must not call {opens}");
        }
        assert!(walk.contains("list_dir(dir)") && walk.contains("on_file(&path, &entry.stamp)"));
        let on_file = include_str!("../watcher.rs").replace("\r\n", "\n");
        let on_file = crate::source_pin::body_between(&on_file, "let mut on_file = |", "// Throttle (task 0811)");
        for opens in ["of_path(", "metadata(", "OpenOptions", "File::open"] {
            assert!(!on_file.contains(opens), "the skip decision must not call {opens}");
        }
        assert_eq!(
            production.matches("identity_at(path)").count(),
            1,
            "one caller: of_path"
        );
        assert_eq!(
            production.matches("DiskStamp::of_path(").count(),
            1,
            "one caller: a named path"
        );
    }

    fn listed_stamps(dir: &std::path::Path) -> HashMap<String, (ListedKind, DiskStamp)> {
        list_dir(dir)
            .unwrap()
            .into_iter()
            .map(|e| (e.name.to_string_lossy().into_owned(), (e.kind, e.stamp)))
            .collect()
    }

    /// Where the platform supplies an identity, a stamp without one is never unchanged; elsewhere equal
    /// stamps are.
    #[test]
    fn a_stamp_without_identity_is_never_unchanged_where_there_is_one() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.bin");
        std::fs::write(&f, b"one").unwrap();
        let bare = DiskStamp::of(&std::fs::metadata(&f).unwrap());
        let full = DiskStamp {
            identity: Some((7, 8)),
            ..bare
        };
        assert!(full.unchanged_on(&full, true));
        assert!(!bare.unchanged_on(&bare, true), "no identity on an identity platform");
        assert!(bare.unchanged_on(&bare, false));
        let other_time = DiskStamp {
            identity: Some((9, 8)),
            ..bare
        };
        let other_file = DiskStamp {
            identity: Some((7, 9)),
            ..bare
        };
        assert!(!other_time.unchanged_on(&full, true), "another ChangeTime");
        assert!(!other_file.unchanged_on(&full, true), "another file at the name");
    }

    /// The listing's stamp of a file left alone is the same on the next listing, and what the platform
    /// asks of a stamp (an identity on Windows) the listing supplies: such a file is skipped unopened.
    #[test]
    fn a_listed_file_left_alone_is_unchanged_on_the_next_listing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("a.bin"), b"one").unwrap();
        std::fs::write(dir.path().join("b.bin"), b"three").unwrap();
        let first = listed_stamps(dir.path());
        let second = listed_stamps(dir.path());
        assert_eq!(first.len(), 3);
        assert_eq!(first["sub"].0, ListedKind::Dir);
        for name in ["a.bin", "b.bin"] {
            assert_eq!(first[name].0, ListedKind::File);
            assert!(second[name].1.unchanged_since(&first[name].1), "{name}");
        }
        assert!(!first["a.bin"].1.unchanged_since(&first["b.bin"].1));
        // A change of the bytes is another stamp.
        std::fs::write(dir.path().join("a.bin"), b"longer bytes").unwrap();
        assert!(!listed_stamps(dir.path())["a.bin"].1.unchanged_since(&first["a.bin"].1));
    }

    /// RC-d on Windows: the raw listing says what `std` says of every entry (name, kind, size, time,
    /// attributes) over more than one buffer of entries, and its identity is the one a handle reads.
    #[cfg(target_os = "windows")]
    #[test]
    fn the_windows_listing_agrees_with_std_and_with_a_handle() {
        use std::os::windows::fs::MetadataExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join("Père's Mac mini")).unwrap();
        std::fs::write(root.join("ünïcødé 写真.jpg"), b"picture").unwrap();
        std::fs::write(root.join("empty.txt"), b"").unwrap();
        // 1,500 long names: well over one 64 KB buffer.
        for n in 0..1500 {
            let name = format!("{n:04}-{}.bin", "a-long-name-to-fill-the-listing-buffer".repeat(3));
            std::fs::write(root.join(name), vec![b'x'; n % 7]).unwrap();
        }
        std::os::windows::fs::symlink_dir(root.join("Père's Mac mini"), root.join("junction-like")).ok();
        let listed = listed_stamps(root);
        let mut by_std = 0;
        for entry in std::fs::read_dir(root).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().into_owned();
            let meta = entry.metadata().unwrap();
            let (kind, stamp) = listed.get(&name).unwrap_or_else(|| panic!("{name} is not listed"));
            let ft = meta.file_type();
            let want = if ft.is_symlink() {
                ListedKind::Link
            } else if ft.is_dir() {
                ListedKind::Dir
            } else {
                ListedKind::File
            };
            assert_eq!(*kind, want, "{name}");
            assert_eq!(stamp.attributes, meta.file_attributes(), "{name}");
            if want == ListedKind::File {
                assert_eq!(stamp.len, meta.len(), "{name}");
                assert_eq!(stamp.modified, meta.modified().ok(), "{name}");
                assert_eq!(stamp.identity, identity_at(&entry.path()), "{name}");
                // A named path's stamp is the listing's: a file the journal names and nothing changed is skipped.
                let named = DiskStamp::of_path(&entry.path(), &std::fs::symlink_metadata(entry.path()).unwrap());
                assert!(named.unchanged_since(stamp), "{name}");
            }
            by_std += 1;
        }
        assert_eq!(listed.len(), by_std, "no entry more, none less (no `.` or `..`)");
        assert!(by_std >= 1503);
    }

    /// Task 2241 on Windows, from the listing alone: a replacement that keeps size and modification time,
    /// and an equal-length rewrite with the time set back, are each another stamp on the next listing.
    #[cfg(target_os = "windows")]
    #[test]
    fn the_windows_listing_shows_what_size_and_time_hide() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let f = root.join("a.bin");
        std::fs::write(&f, b"first bytes").unwrap();
        let time = std::fs::metadata(&f).unwrap().modified().unwrap();
        let before = listed_stamps(root)["a.bin"].1;

        // Equal-length rewrite, modification time set back.
        std::thread::sleep(Duration::from_millis(30));
        let file = std::fs::OpenOptions::new().write(true).open(&f).unwrap();
        std::io::Write::write_all(&mut &file, b"other bytes").unwrap();
        file.set_modified(time).unwrap();
        drop(file);
        let rewritten = listed_stamps(root)["a.bin"].1;
        assert_eq!((rewritten.len, rewritten.modified), (before.len, before.modified));
        assert!(!rewritten.unchanged_since(&before), "the rewrite shows in ChangeTime");

        // Another file with the same size and time renamed over it.
        let other = root.join("b.bin");
        std::fs::write(&other, b"third bytes").unwrap();
        let file = std::fs::OpenOptions::new().write(true).open(&other).unwrap();
        file.set_modified(time).unwrap();
        drop(file);
        std::fs::rename(&other, &f).unwrap();
        let replaced = listed_stamps(root)["a.bin"].1;
        assert_eq!((replaced.len, replaced.modified), (before.len, before.modified));
        assert_ne!(
            replaced.identity.unwrap().1,
            rewritten.identity.unwrap().1,
            "another file id"
        );
        assert!(!replaced.unchanged_since(&rewritten));
    }

    /// RC-d red test (Windows; elsewhere it held): with no change feed every pass is a walk. 113 opened
    /// each unchanged file on every one of them and classified all of them a second time on the second
    /// (their stamps had no identity yet). Now the second walk skips them all, from the listing.
    #[test]
    fn the_second_walk_skips_every_unchanged_file() {
        use crate::state_db::{FileEntry, FileStatus, ItemKind, StateDb};
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("sync-root");
        let db = Arc::new(StateDb::open(temp.path().join("state.db")).unwrap());
        db.set_sync_cursor(1).unwrap();
        for d in 0..3 {
            std::fs::create_dir_all(root.join(format!("d{d}"))).unwrap();
            for f in 0..4 {
                std::fs::write(root.join(format!("d{d}/f{f}.bin")), [b'x'; 64]).unwrap();
                db.upsert_file(&FileEntry {
                    file_id: format!("00000000-0000-4000-8000-{:012}", d * 100 + f),
                    path: format!("d{d}/f{f}.bin"),
                    status: FileStatus::CloudOnly,
                    size_bytes: 64,
                    modified_at: 0,
                    content_hash: None,
                    remote_updated_at: 0,
                    parent_id: None,
                    item_kind: ItemKind::File,
                })
                .unwrap();
            }
        }
        let api = Arc::new(crate::api_client::ApiClient::new(
            "https://api.beebeeb.io".into(),
            "token".into(),
            [17u8; 32],
        ));
        let bridge = EngineBridge::new(&crate::identity::TEST, db.clone(), api);
        let mut memo = ScanMemo::default();
        scan_tick(&bridge, &root, &mut memo, &mut crate::watcher::NoFeed);
        assert_eq!(
            (memo.classified, memo.skipped),
            (12, 0),
            "the first walk classifies each file"
        );
        for pass in 2..=3 {
            scan_tick(&bridge, &root, &mut memo, &mut crate::watcher::NoFeed);
            assert_eq!((memo.classified, memo.skipped), (0, 12), "walk {pass}");
        }
        // A changed file is the only one classified.
        std::fs::write(root.join("d1/f2.bin"), [b'y'; 65]).unwrap();
        scan_tick(&bridge, &root, &mut memo, &mut crate::watcher::NoFeed);
        assert_eq!((memo.classified, memo.skipped), (1, 11));
        // A named path whose file did not change is skipped too (its stamp is the listing's).
        let named: std::collections::BTreeSet<PathBuf> = [root.join("d0/f0.bin")].into();
        let mut feed_named = Named(Some(crate::watcher::Changes::Paths(named)));
        scan_tick(&bridge, &root, &mut memo, &mut feed_named);
        assert_eq!((memo.visited, memo.classified, memo.skipped), (1, 0, 1));
    }

    /// A feed that names what the test told it, once.
    struct Named(Option<crate::watcher::Changes>);

    impl crate::watcher::ChangeFeed for Named {
        fn take(&mut self) -> crate::watcher::Changes {
            self.0.take().unwrap_or(crate::watcher::Changes::Nothing)
        }
    }

    /// RC-d measure, by hand on Windows (a number, not a check):
    /// `cargo test --locked --lib rc_d_measure -- --ignored --nocapture --test-threads 1`.
    /// The process's file operations per scan pass over a tenth of the 41,510-item account (4,110
    /// online-only files in 274 folders, rows in the state DB), with the change journal and without.
    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "a measurement: prints file operations per pass"]
    fn rc_d_measure_file_operations_per_pass() {
        use crate::state_db::{FileEntry, FileStatus, ItemKind, StateDb};
        const DIRS: usize = 274;
        const PER_DIR: usize = 15;
        fn io_ops() -> [u64; 3] {
            use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessIoCounters, IO_COUNTERS};
            let mut c = IO_COUNTERS::default();
            // SAFETY: `c` is the struct the call fills; the pseudo handle of this process needs no close.
            unsafe { GetProcessIoCounters(GetCurrentProcess(), &mut c).unwrap() };
            [c.ReadOperationCount, c.WriteOperationCount, c.OtherOperationCount]
        }
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("sync-root");
        let db = Arc::new(StateDb::open(temp.path().join("state.db")).unwrap());
        db.set_sync_cursor(1).unwrap();
        for d in 0..DIRS {
            let dir = root.join(format!("d{d:04}"));
            std::fs::create_dir_all(&dir).unwrap();
            for f in 0..PER_DIR {
                std::fs::write(dir.join(format!("f{f:02}.bin")), [b'x'; 64]).unwrap();
                db.upsert_file(&FileEntry {
                    file_id: format!("00000000-0000-4000-8000-{:012}", d * 100 + f),
                    path: format!("d{d:04}/f{f:02}.bin"),
                    status: FileStatus::CloudOnly,
                    size_bytes: 64,
                    modified_at: 0,
                    content_hash: None,
                    remote_updated_at: 0,
                    parent_id: None,
                    item_kind: ItemKind::File,
                })
                .unwrap();
            }
        }
        let api = Arc::new(crate::api_client::ApiClient::new(
            "https://api.beebeeb.io".into(),
            "token".into(),
            [17u8; 32],
        ));
        let bridge = EngineBridge::new(&crate::identity::TEST, db.clone(), api);
        let files = (DIRS * PER_DIR) as f64;
        let pass = |what: &str, memo: &mut ScanMemo, feed: &mut dyn crate::watcher::ChangeFeed| {
            let (before, at) = (io_ops(), Instant::now());
            scan_tick(&bridge, &root, memo, feed);
            let after = io_ops();
            let [r, w, o] = [0, 1, 2].map(|i| after[i] - before[i]);
            println!(
                "RC-d {what}: read {r} write {w} other {o} = {:.2} a file | visited {} classified {} skipped {} | {} ms",
                (r + w + o) as f64 / files,
                memo.visited,
                memo.classified,
                memo.skipped,
                at.elapsed().as_millis()
            );
        };
        // The tip: the change journal (a tick that visits 0 shows it is followed).
        let mut memo = ScanMemo::default();
        let mut feed = crate::watcher::change_feed::open(&root);
        pass("journal, first walk (folders scaffolded)", &mut memo, feed.as_mut());
        for n in 1..=3 {
            pass(&format!("journal, idle tick {n}"), &mut memo, feed.as_mut());
        }
        memo.age_by(FULL_RECHECK_INTERVAL);
        pass("journal, hourly backstop walk", &mut memo, feed.as_mut());
        pass("journal, idle tick after it", &mut memo, feed.as_mut());
        memo.age_by(FULL_RECHECK_INTERVAL);
        pass("journal, second backstop walk", &mut memo, feed.as_mut());
        // No journal (build c54280089, or a volume without one): every pass walks.
        let mut memo = ScanMemo::default();
        for n in 1..=4 {
            pass(&format!("no journal, walk {n}"), &mut memo, &mut crate::watcher::NoFeed);
        }
        memo.age_by(FULL_RECHECK_INTERVAL);
        pass("no journal, hourly walk", &mut memo, &mut crate::watcher::NoFeed);
        pass("no journal, walk after it", &mut memo, &mut crate::watcher::NoFeed);
    }
}
