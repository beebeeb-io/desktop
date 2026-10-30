//! Task 2191 S1: the guard for files of unknown content, synced before task 1640 (spec
//! `docs/superpowers/specs/2026-10-25-2191-windows-unknown-baseline-design.md`, D2, D9, D11,
//! D12). Nothing here makes a network call: the guard's functions take the state database, the
//! sync root and the platform's view of a path, never an API client.
//!
//! - At every engine start, every guarded file whose placeholder holds bytes and reads in sync
//!   is marked NOT in sync, so neither the engine, Explorer's "Free up space" nor Storage Sense
//!   can free it ([`guard_rows`]). Since desktop 0.8.15 this pass runs beside the engine, and the
//!   engine's own freeing work waits for it ([`StartupGuard`]).
//! - Every tick, the marks still to be made are retried: a mark that failed, a row that joined
//!   the guarded set during the session, and placeholders left at another path that may hold
//!   unknown bytes ([`retry_marks`]).
//! - At sign-out, unknown bytes are copied out of the sync folder or deleted, as the person
//!   chose (`windows_cf::signout`), never deleted unasked.
//!
//! The decisions are pure and cross-platform so the macOS suite reaches them; the Windows
//! adapters live in `windows_cf`.

use crate::state_db::{FileStatus, GuardedRow, StateDb};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// At most this many pending marks are retried per tick, oldest first, so a large batch of rows
/// joining the guarded set (a first snapshot) never stalls one tick.
pub(crate) const MARK_RETRIES_PER_TICK: usize = 2000;

/// Desktop 0.8.15: the start-up guard reports its progress (an info line, counts only) every
/// this many rows, and records the empty placeholders it found in one transaction per this many.
pub(crate) const GUARD_PROGRESS_EVERY: usize = 2000;

/// Desktop 0.8.15: the start-up guard's progress, as the UI reads it (`startup_guard` on the
/// `engine-status` event and on `sync_status`). Counts only.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub(crate) struct StartupGuardProgress {
    /// A pass is walking the files of unknown content now.
    pub running: bool,
    /// Rows walked so far.
    pub done: u64,
    /// Rows to walk (the guarded set when the pass started).
    pub total: u64,
}

#[derive(Debug, Default)]
struct StartupGuardInner {
    generation: u64,
    /// A pass is owed for the current engine and has not walked every row yet.
    pending: bool,
    progress: StartupGuardProgress,
}

/// Desktop 0.8.15 (the guard no longer blocks the engine): the start-up pass of spec D9 runs
/// beside the engine instead of before it. Uploads, downloads, deletes and renames start at
/// once; the tick's own freeing work (the in-sync stamp pass, the kept-originals retry, cache
/// eviction) waits until this pass has walked every row, as spec U4 orders it.
///
/// One per process ([`STARTUP_GUARD`]). Each engine start [`begin`](Self::begin)s a pass and gets
/// its generation; a pass of an earlier engine that is still stopping can no longer finish or
/// move the current one.
#[derive(Debug, Default)]
pub(crate) struct StartupGuard {
    inner: Mutex<StartupGuardInner>,
}

impl StartupGuard {
    pub(crate) const fn new() -> Self {
        Self {
            inner: Mutex::new(StartupGuardInner {
                generation: 0,
                pending: false,
                progress: StartupGuardProgress { running: false, done: 0, total: 0 },
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, StartupGuardInner> {
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// An engine start owes a pass: from now until it [`finish`](Self::finish)es, freeing work
    /// waits. Returns the pass's generation.
    pub(crate) fn begin(&self) -> u64 {
        let mut inner = self.lock();
        inner.generation += 1;
        inner.pending = true;
        inner.progress = StartupGuardProgress { running: true, done: 0, total: 0 };
        inner.generation
    }

    /// The pass of `generation` has `total` rows to walk.
    pub(crate) fn set_total(&self, generation: u64, total: u64) {
        let mut inner = self.lock();
        if inner.generation == generation {
            inner.progress.total = total;
        }
    }

    /// The pass of `generation` has walked `done` rows.
    pub(crate) fn advance(&self, generation: u64, done: u64) {
        let mut inner = self.lock();
        if inner.generation == generation {
            inner.progress.done = done;
        }
    }

    /// The pass of `generation` walked every row: freeing work may run.
    pub(crate) fn finish(&self, generation: u64) {
        let mut inner = self.lock();
        if inner.generation == generation {
            inner.pending = false;
            inner.progress.running = false;
        }
    }

    /// The pass of `generation` stopped before the end (the engine is stopping, the database
    /// could not be read, a panic): it no longer runs, and freeing work keeps waiting until the
    /// next engine start's pass finishes. The safe direction.
    pub(crate) fn abandon(&self, generation: u64) {
        let mut inner = self.lock();
        if inner.generation == generation {
            inner.progress.running = false;
        }
    }

    /// No start-up pass is owed: the tick's freeing work may run.
    pub(crate) fn allows_freeing(&self) -> bool {
        !self.lock().pending
    }

    pub(crate) fn progress(&self) -> StartupGuardProgress {
        self.lock().progress
    }

    /// A pass is walking the rows now (the tick's retry of pending marks leaves them to it).
    pub(crate) fn is_running(&self) -> bool {
        self.lock().progress.running
    }
}

/// Desktop 0.8.15: the file name of a process image path (`CF_PROCESS_INFO.ImagePath`), at most
/// 64 characters: which program asked for a file's bytes, for the log, never where it lives.
pub(crate) fn requester_image_name(image_path: &[u16]) -> String {
    let separator = |c: &u16| *c == u16::from(b'\\') || *c == u16::from(b'/');
    let start = image_path.iter().rposition(separator).map_or(0, |at| at + 1);
    String::from_utf16_lossy(&image_path[start..]).chars().take(64).collect()
}

/// The process's start-up guard state (Windows begins a pass at every engine start; elsewhere no
/// pass is ever owed, so freeing work is never held).
pub(crate) static STARTUP_GUARD: StartupGuard = StartupGuard::new();

/// Whether the guard's progress line is due after `done` of `total` rows.
pub(crate) fn guard_progress_due(done: usize, total: usize) -> bool {
    done > 0 && (done % GUARD_PROGRESS_EVERY == 0 || done == total)
}

/// How much of a file's bytes are on this PC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OnDisk {
    Nothing,
    Partly,
    Wholly,
}

/// What the platform says about a path. Read without fetching anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PlatformView {
    /// Nothing at the path.
    Missing,
    /// Something is there, but it could not be read.
    Unreadable,
    /// An ordinary file, not a placeholder.
    PlainFile,
    /// A Cloud Files placeholder: how much is on disk, whether it reads in sync (whatever its
    /// modified size, review-114 F-P1-2), and the file id its identity names.
    Placeholder { on_disk: OnDisk, in_sync: bool, identity: Option<String> },
}

/// Task 2284 A1.1: a metadata error means "missing" only when the path is not there; any other
/// error (a denied folder, a held handle) is something that is there but could not be read.
pub(crate) fn view_of_metadata_error(error: &std::io::Error) -> PlatformView {
    if error.kind() == std::io::ErrorKind::NotFound {
        PlatformView::Missing
    } else {
        PlatformView::Unreadable
    }
}

/// What the guard does for one guarded row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GuardDecision {
    /// Mark the placeholder NOT in sync: it holds bytes and reads in sync (spec D2, rows 3 and 5).
    pub mark_not_in_sync: bool,
    /// Record that it holds nothing (`fetched_min_version` 0), conditional on still `-1`: an
    /// empty placeholder of a `local` or `cloud_only` row (spec row 1).
    pub mark_empty: bool,
}

/// Spec section 5, rows 1-5, 19, 22 and 23. The guard never frees anything (D5 withdrawn).
pub(crate) fn decide_guard(view: &PlatformView, fetched_min_version: i64, in_population: bool) -> GuardDecision {
    match view {
        PlatformView::Placeholder { on_disk, in_sync, .. } => GuardDecision {
            mark_not_in_sync: *on_disk != OnDisk::Nothing && *in_sync,
            mark_empty: *on_disk == OnDisk::Nothing && fetched_min_version == -1 && in_population,
        },
        PlatformView::Missing | PlatformView::Unreadable | PlatformView::PlainFile => GuardDecision {
            mark_not_in_sync: false,
            mark_empty: false,
        },
    }
}

/// What a guard pass did, as counts (logged; never a path or a name).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct GuardSummary {
    pub marked: usize,
    pub emptied: usize,
    pub mark_failed: usize,
    /// Rows looked at (all of them unless the pass was stopped).
    pub walked: usize,
    /// The pass was told to stop before the last row (the engine is stopping).
    pub stopped: bool,
}

/// Desktop 0.8.15: how a guard pass is steered from outside. `stop` is asked before each row (the
/// engine stopping: a lock or a sign-out must not wait for the pass, which holds the callback
/// bridge). `progress` is told how many rows were walked after each one.
pub(crate) struct GuardControl<'a> {
    pub stop: &'a dyn Fn() -> bool,
    pub progress: &'a mut dyn FnMut(usize),
}

fn in_population(row: &GuardedRow) -> bool {
    matches!(row.status, FileStatus::Local | FileStatus::CloudOnly)
}

/// Guard `rows` (guarded rows): mark each one's placeholder not in sync when it holds bytes and
/// reads in sync, and record an empty one as empty. A mark that fails is left pending for the
/// tick's retry ([`retry_marks`]); one that is made, or not needed, clears it.
///
/// Desktop 0.8.15: `control.stop` is asked before each row and `control.progress` told after it.
/// A row's pending flag is written only when it changes: a row that needs nothing and was not
/// pending when the pass began gets no write (0.8.14 wrote every one of 37,169 rows), and a flag
/// the database's triggers set while the pass ran (the row changed under it) is left for the
/// tick's retry, never cleared unseen. Empty placeholders are recorded every
/// [`GUARD_PROGRESS_EVERY`] rows and when the pass ends or stops.
pub(crate) fn guard_rows(
    db: &StateDb,
    sync_root: &Path,
    rows: &[GuardedRow],
    view: &dyn Fn(&Path) -> PlatformView,
    mark: &mut dyn FnMut(&Path) -> anyhow::Result<()>,
    control: &mut GuardControl<'_>,
) -> anyhow::Result<GuardSummary> {
    let was_pending = db.mark_pending_ids()?;
    let clear = |file_id: &str| -> anyhow::Result<()> {
        if was_pending.contains(file_id) {
            db.set_mark_pending(file_id, false)?;
        }
        Ok(())
    };
    let mut summary = GuardSummary::default();
    let mut empty = Vec::new();
    for row in rows {
        if (control.stop)() {
            summary.stopped = true;
            break;
        }
        guard_row(db, sync_root, row, view, mark, &clear, &mut summary, &mut empty)?;
        summary.walked += 1;
        (control.progress)(summary.walked);
        if empty.len() >= GUARD_PROGRESS_EVERY {
            summary.emptied += db.mark_no_fetched_content_many(&empty)?;
            empty.clear();
        }
    }
    summary.emptied += db.mark_no_fetched_content_many(&empty)?;
    Ok(summary)
}

/// One row of [`guard_rows`].
#[allow(clippy::too_many_arguments)]
fn guard_row(
    db: &StateDb,
    sync_root: &Path,
    row: &GuardedRow,
    view: &dyn Fn(&Path) -> PlatformView,
    mark: &mut dyn FnMut(&Path) -> anyhow::Result<()>,
    clear: &dyn Fn(&str) -> anyhow::Result<()>,
    summary: &mut GuardSummary,
    empty: &mut Vec<String>,
) -> anyhow::Result<()> {
    let Some(path) = crate::engine_bridge::EngineBridge::placeholder_path_under(sync_root, &row.path) else {
        return clear(&row.file_id);
    };
    let seen = view(&path);
    // Review-157 F1 (spec row 22): a file that can't be read now (held by another app) may
    // hold bytes that read in sync; its mark stays pending and is retried every tick.
    if seen == PlatformView::Unreadable {
        db.set_mark_pending(&row.file_id, true)?;
        summary.mark_failed += 1;
        return Ok(());
    }
    let decision = decide_guard(&seen, row.fetched_min_version, in_population(row));
    if decision.mark_empty {
        empty.push(row.file_id.clone());
    }
    if decision.mark_not_in_sync {
        if let Err(e) = mark(&path) {
            tracing::warn!(file_id = %row.file_id, error = %e, "guard: could not mark a file of unknown content not in sync; retried every tick");
            db.set_mark_pending(&row.file_id, true)?;
            summary.mark_failed += 1;
            return Ok(());
        }
        summary.marked += 1;
    }
    clear(&row.file_id)
}

/// Every tick (spec D2, D9): the marks still to be made. The guarded rows whose mark failed or
/// that joined the guarded set during the session ([`MARK_RETRIES_PER_TICK`] at most), then the
/// recorded orphans still to verify ([`verify_orphan`]). Returns how many marks were made.
pub(crate) fn retry_marks(
    db: &StateDb,
    sync_root: &Path,
    view: &dyn Fn(&Path) -> PlatformView,
    mark: &mut dyn FnMut(&Path) -> anyhow::Result<()>,
    stop: &dyn Fn() -> bool,
) -> anyhow::Result<usize> {
    let rows = db.list_mark_pending(MARK_RETRIES_PER_TICK)?;
    let mut control = GuardControl { stop, progress: &mut |_| {} };
    let summary = guard_rows(db, sync_root, &rows, view, mark, &mut control)?;
    let mut marked = summary.marked;
    if summary.stopped {
        return Ok(marked);
    }
    for (path, _, pending) in db.list_unknown_orphans()? {
        if stop() {
            break;
        }
        if pending && verify_orphan(db, sync_root, &path, view, mark)? {
            marked += 1;
        }
    }
    Ok(marked)
}

/// Both stored shapes of a relative path are the same place.
fn same_place(a: &str, b: &str) -> bool {
    a.trim_start_matches('/') == b.trim_start_matches('/')
}

/// Spec D12 (review-114 F-P1-7, R2-P2-2): the placeholder at `rel_path` is an orphan when it
/// holds bytes and its identity names a file of unknown content that lives at another path, or
/// a file with no row at all. A row of ANOTHER file at `rel_path` does not matter: the identity
/// decides. An orphan that reads in sync is marked not in sync and kept on record for the check
/// (S3); anything else is forgotten. `Ok(true)`: a mark was made.
pub(crate) fn verify_orphan(
    db: &StateDb,
    sync_root: &Path,
    rel_path: &str,
    view: &dyn Fn(&Path) -> PlatformView,
    mark: &mut dyn FnMut(&Path) -> anyhow::Result<()>,
) -> anyhow::Result<bool> {
    let Some(path) = crate::engine_bridge::EngineBridge::placeholder_path_under(sync_root, rel_path) else {
        db.forget_unknown_orphan(rel_path)?;
        return Ok(false);
    };
    let (on_disk, in_sync, identity) = match view(&path) {
        PlatformView::Placeholder { on_disk, in_sync, identity: Some(identity) } => (on_disk, in_sync, identity),
        // Review-157 F1: unreadable now (held by another app): kept, and read again next tick.
        PlatformView::Unreadable => return Ok(false),
        _ => {
            db.forget_unknown_orphan(rel_path)?;
            return Ok(false);
        }
    };
    let owner = db.get_file(&identity)?;
    // Review-157 F7: bytes at another path than the owner's are the owner's state's business only
    // when the owner has agreed content; a row merely marked empty (its new placeholder) says
    // nothing about what the old place still holds.
    let orphan = on_disk != OnDisk::Nothing
        && match owner {
            None => true,
            Some(owner) => !db.has_agreed_content(&identity)? && !same_place(&owner.path, rel_path),
        };
    if !orphan {
        db.forget_unknown_orphan(rel_path)?;
        return Ok(false);
    }
    if in_sync {
        if let Err(e) = mark(&path) {
            tracing::warn!(file_id = %identity, error = %e, "guard: could not mark a placeholder left at another path not in sync; retried every tick");
            db.put_unknown_orphan(rel_path, &identity, true)?;
            return Ok(false);
        }
        db.put_unknown_orphan(rel_path, &identity, false)?;
        return Ok(true);
    }
    db.put_unknown_orphan(rel_path, &identity, false)?;
    Ok(false)
}

/// Spec D11: the folder unknown bytes are copied to at sign-out, beside the sync folder
/// (`stamp` is the local date and time, `YYYY-MM-DD HHMM`). A sync folder at a drive root has
/// no "beside": the person's home folder is used, never a place inside the sync folder.
pub(crate) fn kept_files_dir(sync_root: &Path, stamp: &str, home: Option<&Path>) -> Option<PathBuf> {
    let name = format!("Beebeeb kept files {stamp}");
    match sync_root.parent() {
        Some(parent) => Some(parent.join(name)),
        None => home.filter(|home| !home.starts_with(sync_root)).map(|home| home.join(name)),
    }
}

/// What the database says is at a path at sign-out (self-review: never trusted without the
/// placeholder's identity).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ExpectedAt {
    /// The path of this file of unknown content.
    Guarded(String),
    /// A place recorded as possibly holding another file's unknown bytes, not verified yet.
    OrphanCandidate,
}

/// Spec D11: the bytes at `rel_path` are kept (or deleted, as asked) as unknown at sign-out only
/// when the platform agrees with the database: at a guarded file's path, a plain file or a
/// placeholder of that same file; at an orphan candidate, a placeholder whose identity names a
/// file living elsewhere without agreed content ([`verify_orphan`]'s rule), or no file at all.
/// `owner_of_identity` is that file's (path, has agreed content). Anything else is signed out by
/// the ordinary rules, which refuse unsent changes.
pub(crate) fn unknown_at_signout(
    expected: &ExpectedAt,
    rel_path: &str,
    is_placeholder: bool,
    identity: Option<&str>,
    owner_of_identity: Option<(&str, bool)>,
) -> bool {
    match expected {
        ExpectedAt::Guarded(file_id) => !is_placeholder || identity == Some(file_id.as_str()),
        ExpectedAt::OrphanCandidate => {
            is_placeholder
                && identity.is_some()
                && match owner_of_identity {
                    None => true,
                    Some((path, agreed)) => !agreed && !same_place(path, rel_path),
                }
        }
    }
}

/// Spec D11: copy `source` (of `size` bytes) to `dest`, wholly or only the `ranges` that are on
/// this PC (a partly downloaded placeholder: the rest is left as holes, and the name gets
/// " (partial)"). Returns the copy's hash ([`hash_source`]'s definition) and where it went.
pub(crate) fn copy_out(
    source: &mut std::fs::File,
    size: u64,
    ranges: Option<&[(u64, u64)]>,
    dest: &Path,
) -> std::io::Result<(String, PathBuf)> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Seek, SeekFrom, Write};
    let dest = match ranges {
        Some(_) => partial_name(dest),
        None => dest.to_path_buf(),
    };
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut out = std::fs::OpenOptions::new().write(true).create_new(true).open(&dest)?;
    let mut hasher = Sha256::new();
    let whole = [(0, size)];
    let spans: &[(u64, u64)] = ranges.unwrap_or(&whole);
    let mut buffer = vec![0u8; 64 * 1024];
    for &(offset, length) in spans {
        source.seek(SeekFrom::Start(offset))?;
        out.seek(SeekFrom::Start(offset))?;
        let mut left = length;
        while left > 0 {
            let want = buffer.len().min(usize::try_from(left).unwrap_or(usize::MAX));
            let read = source.read(&mut buffer[..want])?;
            if read == 0 {
                break;
            }
            out.write_all(&buffer[..read])?;
            hasher.update(&buffer[..read]);
            left -= read as u64;
        }
    }
    out.set_len(size)?;
    out.sync_all()?;
    Ok((format!("{:x}", hasher.finalize()), dest))
}

/// The hash [`copy_out`] returns, read again from `source` over the same `ranges` (the whole
/// file when `None`), so sign-out deletes the original only if it still holds what was copied.
/// For a partly downloaded file only the ranges on this PC are read, so nothing is fetched.
pub(crate) fn hash_source(source: &mut std::fs::File, size: u64, ranges: Option<&[(u64, u64)]>) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Seek, SeekFrom};
    let mut hasher = Sha256::new();
    let whole = [(0, size)];
    let spans: &[(u64, u64)] = ranges.unwrap_or(&whole);
    let mut buffer = vec![0u8; 64 * 1024];
    for &(offset, length) in spans {
        source.seek(SeekFrom::Start(offset))?;
        let mut left = length;
        while left > 0 {
            let want = buffer.len().min(usize::try_from(left).unwrap_or(usize::MAX));
            let read = source.read(&mut buffer[..want])?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            left -= read as u64;
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Where [`copy_out`] writes a copy of `dest` (a partial copy gets " (partial)" in its name).
pub(crate) fn copy_dest(dest: &Path, partial: bool) -> PathBuf {
    if partial { partial_name(dest) } else { dest.to_path_buf() }
}

/// `a/b.docx` -> `a/b (partial).docx`.
fn partial_name(dest: &Path) -> PathBuf {
    let stem = dest.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let name = match dest.extension().and_then(|s| s.to_str()) {
        Some(ext) => format!("{stem} (partial).{ext}"),
        None => format!("{stem} (partial)"),
    };
    dest.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_db::{FileEntry, ItemKind};
    use std::collections::HashSet;

    /// The guard's pass with nobody stopping it and no progress wanted (the 2191 tests' shape).
    fn guard(
        db: &StateDb,
        root: &Path,
        rows: &[GuardedRow],
        view: &dyn Fn(&Path) -> PlatformView,
        mark: &mut dyn FnMut(&Path) -> anyhow::Result<()>,
    ) -> anyhow::Result<GuardSummary> {
        guard_rows(db, root, rows, view, mark, &mut GuardControl { stop: &|| false, progress: &mut |_| {} })
    }

    fn retry(
        db: &StateDb,
        root: &Path,
        view: &dyn Fn(&Path) -> PlatformView,
        mark: &mut dyn FnMut(&Path) -> anyhow::Result<()>,
    ) -> anyhow::Result<usize> {
        retry_marks(db, root, view, mark, &|| false)
    }

    fn placeholder(on_disk: OnDisk, in_sync: bool) -> PlatformView {
        PlatformView::Placeholder { on_disk, in_sync, identity: None }
    }

    #[test]
    fn task_2191_the_guard_decides_from_the_platform_view() {
        use OnDisk::*;
        let cases: [(PlatformView, i64, bool, bool, bool); 12] = [
            // (view, fetched_min_version, in population) -> (mark not in sync, mark empty)
            (placeholder(Nothing, true), -1, true, false, true),
            (placeholder(Nothing, true), -2, true, false, false),
            (placeholder(Nothing, true), -1, false, false, false),
            (placeholder(Partly, true), -1, true, true, false),
            (placeholder(Partly, true), -2, true, true, false),
            (placeholder(Partly, false), -1, true, false, false),
            (placeholder(Wholly, true), -1, true, true, false),
            (placeholder(Wholly, false), -1, true, false, false),
            (placeholder(Wholly, true), -1, false, true, false),
            (PlatformView::Missing, -1, true, false, false),
            (PlatformView::Unreadable, -1, true, false, false),
            (PlatformView::PlainFile, -1, true, false, false),
        ];
        for (view, fmv, population, mark, empty) in cases {
            assert_eq!(
                decide_guard(&view, fmv, population),
                GuardDecision { mark_not_in_sync: mark, mark_empty: empty },
                "{view:?} fmv {fmv} population {population}"
            );
        }
    }

    fn db_with(rows: &[(&str, &str, FileStatus)]) -> (tempfile::TempDir, StateDb) {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        for (id, path, status) in rows {
            db.upsert_file(&FileEntry {
                file_id: (*id).into(),
                path: (*path).into(),
                status: status.clone(),
                size_bytes: 1,
                modified_at: 0,
                content_hash: None,
                remote_updated_at: 0,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
        }
        (dir, db)
    }

    #[test]
    fn task_2191_conflict_and_error_rows_are_marked_and_a_failed_mark_is_retried_on_the_next_tick() {
        let (dir, db) = db_with(&[
            ("c", "c.txt", FileStatus::Conflict),
            ("e", "e.txt", FileStatus::Error),
            ("l", "l.txt", FileStatus::Local),
        ]);
        let root = dir.path().join("root");
        let view = |_: &Path| placeholder(OnDisk::Wholly, true);
        let mut calls = Vec::new();
        let mut fail_l = true;
        let rows = db.list_guarded().unwrap();
        assert_eq!(rows.len(), 3, "every status is guarded");
        let summary = guard(&db, &root, &rows, &view, &mut |path: &Path| {
            calls.push(path.file_name().unwrap().to_string_lossy().into_owned());
            if fail_l && path.ends_with("l.txt") {
                fail_l = false;
                anyhow::bail!("in use")
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(summary, GuardSummary { marked: 2, emptied: 0, mark_failed: 1, walked: 3, stopped: false });
        // The next tick retries only the failed one.
        let mut retried = Vec::new();
        let marked = retry(&db, &root, &view, &mut |path: &Path| {
            retried.push(path.file_name().unwrap().to_string_lossy().into_owned());
            Ok(())
        })
        .unwrap();
        assert_eq!((marked, retried), (1, vec!["l.txt".to_string()]));
        let again = retry(&db, &root, &view, &mut |_: &Path| -> anyhow::Result<()> { panic!("nothing is pending") }).unwrap();
        assert_eq!(again, 0);
    }

    #[test]
    fn task_2191_a_row_that_joins_the_guarded_set_mid_session_is_marked_at_the_next_tick() {
        let (dir, db) = db_with(&[]);
        let root = dir.path().join("root");
        // A pull inserts a cloud_only row whose 0.8.12 placeholder already holds an edit.
        db.upsert_file(&FileEntry {
            file_id: "late".into(),
            path: "late.txt".into(),
            status: FileStatus::CloudOnly,
            size_bytes: 1,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
        // ...and one inserted as a conflict (any status is guarded, self-review).
        db.upsert_file(&FileEntry {
            file_id: "late-conflict".into(),
            path: "late (conflict).txt".into(),
            status: FileStatus::Conflict,
            size_bytes: 1,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
        let mut marks = 0;
        let marked = retry(&db, &root, &|_: &Path| placeholder(OnDisk::Wholly, true), &mut |_: &Path| {
            marks += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!((marked, marks), (2, 2), "marked at the first tick, not at the next start");
    }

    #[test]
    fn task_2191_an_orphan_is_found_by_its_identity_even_under_another_file_s_row() {
        // `u` (unknown content) was renamed on the web to `u2.txt`; its old placeholder at
        // `u.txt` holds its bytes. A known file `k` was then moved to `u.txt`: its row sits there.
        let (dir, db) = db_with(&[("u", "u2.txt", FileStatus::CloudOnly), ("k", "u.txt", FileStatus::Local)]);
        db.record_local_baseline("k", 1, "hash").unwrap();
        let root = dir.path().join("root");
        db.set_mark_pending("u", false).unwrap(); // its own place is marked already
        db.put_unknown_orphan("u.txt", "k", true).unwrap();
        let view = |_: &Path| PlatformView::Placeholder { on_disk: OnDisk::Wholly, in_sync: true, identity: Some("u".into()) };
        let mut marks = 0;
        retry(&db, &root, &view, &mut |_: &Path| {
            marks += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(marks, 1, "marked not in sync");
        assert_eq!(db.list_unknown_orphans().unwrap(), vec![("u.txt".into(), "u".into(), false)], "kept on record, named by its identity");
        // A placeholder whose identity is the file at that path is no orphan.
        db.put_unknown_orphan("u2.txt", "u", true).unwrap();
        retry(&db, &root, &|_: &Path| PlatformView::Placeholder { on_disk: OnDisk::Wholly, in_sync: true, identity: Some("u".into()) }, &mut |_: &Path| Ok(())).unwrap();
        assert!(db.list_unknown_orphans().unwrap().iter().all(|(path, _, _)| path != "u2.txt"), "forgotten");
    }

    /// Review-157 F1 (spec row 22): a file held by another app can't be read now; its mark stays
    /// pending and is tried again at the next tick, and an unreadable orphan stays on record.
    #[test]
    fn task_2191_an_unreadable_file_keeps_its_mark_pending() {
        let (dir, db) = db_with(&[("u", "u.pst", FileStatus::Local)]);
        let root = dir.path().join("root");
        let rows = db.list_guarded().unwrap();
        let summary = guard(&db, &root, &rows, &|_: &Path| PlatformView::Unreadable, &mut |_: &Path| -> anyhow::Result<()> { panic!("no mark") }).unwrap();
        assert_eq!(summary.mark_failed, 1);
        let mut marks = 0;
        retry(&db, &root, &|_: &Path| placeholder(OnDisk::Wholly, true), &mut |_: &Path| {
            marks += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(marks, 1, "readable again: marked at the next tick");
        db.put_unknown_orphan("old.pst", "u", true).unwrap();
        retry(&db, &root, &|_: &Path| PlatformView::Unreadable, &mut |_: &Path| Ok(())).unwrap();
        assert_eq!(db.list_unknown_orphans().unwrap(), vec![("old.pst".into(), "u".into(), true)], "kept for the next tick");
    }

    /// Review-157 F7: a renamed file's new placeholder marked its row empty (fetched 0, no agreed
    /// content); the old place, still holding the edit, stays an orphan.
    #[test]
    fn task_2191_an_orphan_stays_when_its_owner_was_only_marked_empty() {
        let (dir, db) = db_with(&[("u", "new.docx", FileStatus::CloudOnly)]);
        assert!(db.mark_no_fetched_content("u").unwrap(), "precondition: marked empty");
        let root = dir.path().join("root");
        db.set_mark_pending("u", false).unwrap();
        db.put_unknown_orphan("old.docx", "u", true).unwrap();
        let view = |_: &Path| PlatformView::Placeholder { on_disk: OnDisk::Wholly, in_sync: true, identity: Some("u".into()) };
        let mut marks = 0;
        retry(&db, &root, &view, &mut |_: &Path| {
            marks += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(marks, 1, "marked not in sync");
        assert_eq!(db.list_unknown_orphans().unwrap(), vec![("old.docx".into(), "u".into(), false)]);
    }

    #[test]
    fn task_2191_an_orphan_of_a_known_file_or_holding_nothing_is_forgotten() {
        let (dir, db) = db_with(&[("k", "k2.txt", FileStatus::Local)]);
        db.record_local_baseline("k", 1, "hash").unwrap();
        let root = dir.path().join("root");
        db.put_unknown_orphan("k.txt", "k", true).unwrap();
        db.put_unknown_orphan("gone.txt", "x", true).unwrap();
        let view = |path: &Path| {
            if path.ends_with("gone.txt") {
                PlatformView::Placeholder { on_disk: OnDisk::Nothing, in_sync: true, identity: Some("x".into()) }
            } else {
                PlatformView::Placeholder { on_disk: OnDisk::Wholly, in_sync: true, identity: Some("k".into()) }
            }
        };
        let marked = retry(&db, &root, &view, &mut |_: &Path| -> anyhow::Result<()> { panic!("no mark") }).unwrap();
        assert_eq!(marked, 0);
        assert!(db.list_unknown_orphans().unwrap().is_empty());
    }

    /// D9 / U4, as desktop 0.8.15 revised them: the start-up pass is begun after seeding and
    /// before the tick loop; every tick retries the pending marks before the pause check, and the
    /// stamp pass comes after it (a source pin: the runner calls them on Windows only).
    #[test]
    fn task_2191_the_guard_runs_before_the_first_tick_and_every_tick_before_the_stamp() {
        let runner = include_str!("runner.rs").replace("\r\n", "\n");
        let production = runner.split("\nmod tests {").next().unwrap();
        let at = |needle: &str| production.find(needle).unwrap_or_else(|| panic!("{needle} not found"));
        let seed = at("crate::windows_cf::seed_placeholders(bridge.clone(), &sync_root);");
        let guard = at("spawn_startup_guard(sync_root.clone());");
        let tick = at("let mut tick = tokio::time::interval(TICK_INTERVAL);");
        let after = |from: usize, needle: &str| from + production[from..].find(needle).unwrap_or_else(|| panic!("{needle} not found"));
        const PAUSED: &str = "if sync_paused.load(Ordering::Relaxed) {";
        let retry = after(tick, "crate::windows_cf::retry_pending_marks(&mark_root)).await");
        let paused = after(retry, PAUSED);
        let stamp = after(paused, "crate::windows_cf::stamp_local_files_in_sync(&stamp_root);");
        assert!(seed < guard && guard < tick, "the guard starts after seeding and before the tick loop");
        assert!(
            !production[tick..retry].contains(PAUSED),
            "every tick, paused included: the pending marks come before the loop's pause check (review-157 F2)"
        );
        assert!(paused < stamp);
        assert_eq!(production.matches("retry_pending_marks(").count(), 1, "one call, on every tick");
    }

    /// Desktop 0.8.15 (deliverable 1): uploads must not wait for the guard. On Guus's PC the
    /// 0.8.14 pass ran 2 h 12 min, and nothing uploaded while it ran, because the runner awaited
    /// it before it spawned the upload watcher. Now the pass is spawned (off the async workers)
    /// and nothing between seeding and the tick loop is awaited.
    #[test]
    fn desktop_0815_uploads_start_without_waiting_for_the_guard() {
        let runner = include_str!("runner.rs").replace("\r\n", "\n");
        let production = runner.split("\nmod tests {").next().unwrap();
        let at = |needle: &str| production.find(needle).unwrap_or_else(|| panic!("{needle} not found"));
        let seed = at("crate::windows_cf::seed_placeholders(bridge.clone(), &sync_root);");
        let guard = at("spawn_startup_guard(sync_root.clone());");
        let watcher = at("let _upload_watcher = crate::watcher::spawn(bridge.clone(), sync_root.clone());");
        let tick = at("let mut tick = tokio::time::interval(TICK_INTERVAL);");
        assert!(seed < guard && guard < watcher && watcher < tick, "seed, start the pass, watcher, ticks");
        assert!(
            !production[seed..tick].contains(".await"),
            "nothing between seeding and the tick loop is awaited: the watcher and the first tick never wait for the pass"
        );
        let start = at("fn spawn_startup_guard(");
        let body = &production[start..start + production[start..].find("\n}\n").unwrap()];
        let within = |needle: &str| body.find(needle).unwrap_or_else(|| panic!("{needle} not in spawn_startup_guard"));
        let begin = within("let generation = crate::unknown_check::STARTUP_GUARD.begin();");
        let task = within("tokio::spawn(async move {");
        let blocking = within("tokio::task::spawn_blocking(move ||");
        let pass = within("crate::windows_cf::guard_unknown_placeholders(&guard_root, generation)");
        let panicked = within("crate::unknown_check::STARTUP_GUARD.abandon(generation);");
        assert!(
            begin < task && task < blocking && blocking < pass && pass < panicked,
            "begun before the first tick, then walked in its own task off the async workers; a pass that panicked leaves freeing held"
        );
    }

    /// Desktop 0.8.15 (deliverable 1): the tick's own freeing work, the in-sync stamp pass and
    /// the kept-originals retry (and cache eviction, pinned in `engine_bridge`), waits until the
    /// start-up pass has walked every row (spec U4: the guard comes before the first stamp).
    #[test]
    fn desktop_0815_the_tick_stamps_nothing_until_the_start_up_pass_is_done() {
        use crate::source_pin::squeeze;
        let runner = include_str!("runner.rs").replace("\r\n", "\n");
        let production = runner.split("\nmod tests {").next().unwrap();
        let stamp = production.find("crate::windows_cf::stamp_local_files_in_sync(&stamp_root);").unwrap();
        let kept = production.find("crate::windows_cf::retry_kept_originals(&stamp_root);").unwrap();
        let block = production[..stamp].rfind("let stamp_root = sync_root.clone();").unwrap();
        let gate = production[..block].rfind("if ").unwrap();
        assert_eq!(
            squeeze(&production[gate..block]),
            squeeze("if crate::unknown_check::STARTUP_GUARD.allows_freeing() {"),
            "the stamp block opens under the gate"
        );
        assert!(stamp < kept && !production[stamp..kept].contains("})"), "the kept-originals retry sits in the same gated pass");
        assert_eq!(production.matches("stamp_local_files_in_sync(").count(), 1);
        assert_eq!(production.matches("retry_kept_originals(").count(), 1);
    }

    /// Desktop 0.8.15 (deliverables 2, 3): the UI reads the start-up guard's progress as
    /// `startup_guard` (`{running, done, total}`) on every `engine-status` event and on the
    /// `sync_status` poll, and the changes held back as `changes_on_hold` on `sync_status`.
    #[test]
    fn desktop_0815_the_ui_reads_the_guard_s_progress_and_the_changes_on_hold() {
        let runner = include_str!("runner.rs").replace("\r\n", "\n");
        let production = runner.split("\nmod tests {").next().unwrap();
        let emit = &production[production.find("fn emit_payload<").unwrap()..];
        let emit = &emit[..emit.find("\n}\n").unwrap()];
        assert!(emit.contains("app.emit(crate::engine_status::EVENT, with_startup_guard(payload))"), "{emit}");
        assert_eq!(production.matches("crate::engine_status::EVENT").count(), 1, "every engine-status event goes through emit_payload");
        let lib = include_str!("lib.rs").replace("\r\n", "\n");
        let status = &lib[lib.find("async fn sync_status(").unwrap()..];
        let status = &status[..status.find("\n}\n").unwrap()];
        let literal = status.rfind("Ok(serde_json::json!({").unwrap();
        assert!(status[literal..].contains("\"startup_guard\": crate::unknown_check::STARTUP_GUARD.progress(),"), "{status}");
        assert!(status[literal..].contains("\"changes_on_hold\": counts.3,"), "{status}");
        // Task 2243: the counts come from `sync_status_counts`, which counts through the indexes.
        assert!(status[..literal].contains(".map(|db| sync_status_counts(&db))"), "{status}");
        let counts = &lib[lib.find("fn sync_status_counts(").unwrap()..];
        let counts = &counts[..counts.find("\n}\n").unwrap()];
        assert!(counts.contains("db.count_operations_on_hold().unwrap_or(0)"), "{counts}");
        // No engine began a pass in this process: the shape the UI reads, at rest.
        let value = crate::runner::with_startup_guard(serde_json::json!({ "state": "running" }));
        assert_eq!(value["startup_guard"], serde_json::json!({ "running": false, "done": 0, "total": 0 }));
        assert_eq!(value["state"], "running");
    }

    /// Desktop 0.8.15: freeing waits for the pass begun at the latest engine start; a pass of an
    /// earlier engine can neither open the gate nor move the progress, and a pass that stopped
    /// before the end keeps it shut.
    #[test]
    fn desktop_0815_freeing_waits_for_the_start_up_pass() {
        let guard = StartupGuard::new();
        assert!(guard.allows_freeing(), "no engine started a pass: nothing is held (macOS, Linux)");
        assert_eq!(guard.progress(), StartupGuardProgress::default());
        let first = guard.begin();
        assert!(!guard.allows_freeing(), "held from the engine start");
        assert_eq!(guard.progress(), StartupGuardProgress { running: true, done: 0, total: 0 });
        guard.set_total(first, 3);
        guard.advance(first, 2);
        assert_eq!(guard.progress(), StartupGuardProgress { running: true, done: 2, total: 3 });
        let second = guard.begin();
        guard.advance(first, 3);
        guard.finish(first);
        assert!(!guard.allows_freeing(), "an earlier engine's pass does not open the gate");
        assert_eq!(guard.progress(), StartupGuardProgress { running: true, done: 0, total: 0 });
        assert!(guard.is_running());
        guard.abandon(second);
        assert!(!guard.allows_freeing(), "a pass that stopped keeps freeing held");
        assert!(!guard.is_running());
        let third = guard.begin();
        guard.set_total(third, 1);
        guard.advance(third, 1);
        guard.finish(third);
        assert!(guard.allows_freeing());
        assert_eq!(guard.progress(), StartupGuardProgress { running: false, done: 1, total: 1 });
        assert!(guard_progress_due(GUARD_PROGRESS_EVERY, 37_169) && guard_progress_due(37_169, 37_169));
        assert!(!guard_progress_due(1, 37_169) && !guard_progress_due(0, 0));
    }

    fn empty_rows(n: usize) -> (tempfile::TempDir, StateDb) {
        let ids: Vec<(String, String)> = (0..n).map(|i| (format!("f{i}"), format!("f{i}.txt"))).collect();
        let rows: Vec<(&str, &str, FileStatus)> = ids.iter().map(|(id, path)| (id.as_str(), path.as_str(), FileStatus::CloudOnly)).collect();
        db_with(&rows)
    }

    /// Desktop 0.8.15: a lock or a sign-out stops the pass between rows (it holds the callback
    /// bridge, whose drain waits 3 s), and what it found so far is kept.
    #[test]
    fn desktop_0815_the_guard_stops_between_rows_and_keeps_what_it_found() {
        let (dir, db) = empty_rows(3);
        let root = dir.path().join("root");
        let rows = db.list_guarded().unwrap();
        let walked = std::cell::Cell::new(0);
        let mut seen = Vec::new();
        let summary = guard_rows(
            &db,
            &root,
            &rows,
            &|_: &Path| placeholder(OnDisk::Nothing, true),
            &mut |_: &Path| -> anyhow::Result<()> { panic!("an empty placeholder is never marked") },
            &mut GuardControl {
                stop: &|| walked.get() >= 1,
                progress: &mut |done| {
                    walked.set(done);
                    seen.push(done);
                },
            },
        )
        .unwrap();
        assert_eq!(summary, GuardSummary { marked: 0, emptied: 1, mark_failed: 0, walked: 1, stopped: true });
        assert_eq!(seen, vec![1]);
        assert_eq!(db.list_guarded().unwrap().len(), 2, "the rows not walked stay guarded for the next start");
    }

    /// Desktop 0.8.15: the UI's progress moves after every row.
    #[test]
    fn desktop_0815_the_guard_reports_progress_after_every_row() {
        let (dir, db) = empty_rows(3);
        let root = dir.path().join("root");
        let rows = db.list_guarded().unwrap();
        let mut seen = Vec::new();
        let summary = guard_rows(
            &db,
            &root,
            &rows,
            &|_: &Path| placeholder(OnDisk::Wholly, false),
            &mut |_: &Path| -> anyhow::Result<()> { panic!("not in sync already: no mark") },
            &mut GuardControl { stop: &|| false, progress: &mut |done| seen.push(done) },
        )
        .unwrap();
        assert_eq!(summary, GuardSummary { walked: 3, ..GuardSummary::default() });
        assert_eq!(seen, vec![1, 2, 3]);
    }

    /// Desktop 0.8.15 (deliverable 2): a row that needs nothing gets no write. The pass writes a
    /// row's pending flag only when it was pending when the pass began, so a flag the database's
    /// triggers set while the pass ran (the row changed under it) is never cleared unseen.
    #[test]
    fn desktop_0815_the_guard_never_clears_a_flag_set_while_it_ran() {
        let (dir, db) = db_with(&[("a", "a.txt", FileStatus::Local), ("b", "b.txt", FileStatus::Local)]);
        // Both joined the guarded set on insert (the trigger flags them); `a` was marked since.
        db.set_mark_pending("a", false).unwrap();
        assert_eq!(db.mark_pending_ids().unwrap(), HashSet::from(["b".to_string()]), "precondition");
        let root = dir.path().join("root");
        let rows = db.list_guarded().unwrap();
        let view = |path: &Path| {
            if path.ends_with("a.txt") {
                // The row changed while the pass looked at it: its trigger flagged it again.
                db.set_mark_pending("a", true).unwrap();
            }
            placeholder(OnDisk::Wholly, false)
        };
        guard(&db, &root, &rows, &view, &mut |_: &Path| -> anyhow::Result<()> { panic!("no mark") }).unwrap();
        let pending: Vec<String> = db.list_mark_pending(10).unwrap().into_iter().map(|row| row.file_id).collect();
        assert_eq!(pending, vec!["a".to_string()], "a's new flag stays for the tick; b's, seen and needing nothing, is cleared");
    }

    /// Desktop 0.8.15 (deliverable 2): the requesting process of a Cloud Files fetch is logged by
    /// its image's file name, never its path.
    #[test]
    fn desktop_0815_a_requester_is_named_by_its_image_file_name_only() {
        let wide = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
        assert_eq!(requester_image_name(&wide(r"C:\ProgramData\Microsoft\Windows Defender\Platform\4.18\MsMpEng.exe")), "MsMpEng.exe");
        assert_eq!(requester_image_name(&wide(r"\Device\HarddiskVolume3\Windows\explorer.exe")), "explorer.exe");
        assert_eq!(requester_image_name(&wide("beebeeb-desktop.exe")), "beebeeb-desktop.exe");
        assert_eq!(requester_image_name(&wide("")), "");
        assert_eq!(requester_image_name(&wide(&format!(r"C:\x\{}", "a".repeat(300)))).chars().count(), 64, "capped");
    }

    /// Self-review: at a drive root the copies go to the home folder, never into the sync folder.
    #[test]
    fn task_2191_a_sync_folder_at_a_drive_root_keeps_copies_in_the_home_folder() {
        let root = Path::new("/");
        let home = Path::new("/Users/someone");
        assert_eq!(
            kept_files_dir(Path::new("/Beebeeb"), "s", Some(home)),
            Some(PathBuf::from("/Beebeeb kept files s"))
        );
        assert_eq!(kept_files_dir(root, "s", None), None, "no home: no place to keep, sign-out refuses Keep");
        assert_eq!(kept_files_dir(root, "s", Some(home)), None, "a home inside the sync folder is no outside place");
    }

    /// Self-review: sign-out trusts the database only when the placeholder's identity agrees.
    #[test]
    fn task_2191_sign_out_treats_bytes_as_unknown_only_when_the_identity_agrees() {
        let guarded = ExpectedAt::Guarded("u".into());
        assert!(unknown_at_signout(&guarded, "a.txt", false, None, None), "a plain file at a guarded path");
        assert!(unknown_at_signout(&guarded, "a.txt", true, Some("u"), None), "its own placeholder");
        assert!(!unknown_at_signout(&guarded, "a.txt", true, Some("k"), Some(("a.txt", true))), "another file's: ordinary rules");
        let candidate = ExpectedAt::OrphanCandidate;
        assert!(unknown_at_signout(&candidate, "a.txt", true, Some("u"), Some(("b.txt", false))), "no agreed content, lives elsewhere");
        assert!(unknown_at_signout(&candidate, "a.txt", true, Some("gone"), None), "no file at all");
        assert!(!unknown_at_signout(&candidate, "a.txt", true, Some("k"), Some(("b.txt", true))), "a file with agreed content: ordinary rules");
        assert!(!unknown_at_signout(&candidate, "a.txt", true, Some("u"), Some(("a.txt", false))), "its own place");
        assert!(!unknown_at_signout(&candidate, "a.txt", false, None, None), "a plain file: ordinary rules");
    }

    /// Task 2284 A1.1: a metadata error says "missing" only when it is NotFound.
    #[test]
    fn a_metadata_error_other_than_not_found_is_unreadable() {
        use std::io;
        assert_eq!(
            crate::unknown_check::view_of_metadata_error(&io::Error::from(io::ErrorKind::NotFound)),
            PlatformView::Missing
        );
        assert_eq!(
            crate::unknown_check::view_of_metadata_error(&io::Error::from(io::ErrorKind::PermissionDenied)),
            PlatformView::Unreadable
        );
        assert_eq!(
            crate::unknown_check::view_of_metadata_error(&io::Error::other("x")),
            PlatformView::Unreadable
        );
    }

    #[test]
    fn task_2191_sign_out_copies_whole_and_partial_files_and_rehashes_what_it_copied() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("a.docx");
        std::fs::write(&source_path, b"0123456789abcdef").unwrap();
        let mut source = std::fs::File::open(&source_path).unwrap();
        let kept = kept_files_dir(&dir.path().join("Beebeeb"), "2026-10-26 1200", None).unwrap();
        assert_eq!(kept, dir.path().join("Beebeeb kept files 2026-10-26 1200"));
        let (whole_hash, whole) = copy_out(&mut source, 16, None, &kept.join("docs/a.docx")).unwrap();
        assert_eq!(std::fs::read(&whole).unwrap(), b"0123456789abcdef");
        assert_eq!(hash_source(&mut source, 16, None).unwrap(), whole_hash);
        let ranges = [(0, 4), (10, 2)];
        let (partial_hash, partial) = copy_out(&mut source, 16, Some(&ranges), &kept.join("docs/b.docx")).unwrap();
        assert_eq!(partial, kept.join("docs/b (partial).docx"));
        assert_eq!(std::fs::read(&partial).unwrap(), b"0123\0\0\0\0\0\0ab\0\0\0\0", "holes where nothing was on this PC");
        assert_eq!(hash_source(&mut source, 16, Some(&ranges)).unwrap(), partial_hash);
        std::fs::write(&source_path, b"0123456789XXcdef").unwrap();
        let mut changed = std::fs::File::open(&source_path).unwrap();
        assert_ne!(hash_source(&mut changed, 16, Some(&ranges)).unwrap(), partial_hash, "a change is seen");
    }
}
