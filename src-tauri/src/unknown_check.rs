//! Task 2191: the guard for files of unknown content, synced before task 1640 (spec
//! `docs/superpowers/specs/2026-10-25-2191-windows-unknown-baseline-design.md`, D2, D9, D11,
//! D12). The guard functions make no network calls: they take the state database, the
//! sync root and the platform's view of a path, never an API client.
//!
//! - At every engine start, every guarded file whose placeholder holds bytes and reads in sync
//!   is marked NOT in sync, so neither the engine, Explorer's "Free up space" nor Storage Sense
//!   can free it ([`guard_rows`]).
//! - Every tick, the marks still to be made are retried: a mark that failed, a row that joined
//!   the guarded set during the session, and placeholders left at another path that may hold
//!   unknown bytes ([`retry_marks`]).
//! - At sign-out, unknown bytes are copied out of the sync folder or deleted, as the person
//!   chose (`windows_cf::signout`), never deleted unasked.
//!
//! The decisions are pure and cross-platform so the macOS suite reaches them; the Windows
//! adapters live in `windows_cf`. S2 recording, comparison and recovery live in the child modules;
//! `job::Job` owns their Windows-only, account-scoped lifetime.

pub(crate) mod budget;
mod record;
pub(crate) mod compare;
mod resolve;
pub(crate) mod job;
mod cost;
mod source;

use crate::state_db::{FileStatus, GuardedRow, StateDb};
use std::path::{Path, PathBuf};

/// At most this many pending marks are retried per tick, oldest first, so a large batch of rows
/// joining the guarded set (a first snapshot) never stalls one tick.
pub(crate) const MARK_RETRIES_PER_TICK: usize = 2000;

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
}

fn in_population(row: &GuardedRow) -> bool {
    matches!(row.status, FileStatus::Local | FileStatus::CloudOnly)
}

/// Guard `rows` (guarded rows): mark each one's placeholder not in sync when it holds bytes and
/// reads in sync, and record an empty one as empty. A mark that fails is left pending for the
/// tick's retry ([`retry_marks`]); one that is made, or not needed, clears it.
pub(crate) fn guard_rows(
    db: &StateDb,
    sync_root: &Path,
    rows: &[GuardedRow],
    view: &dyn Fn(&Path) -> PlatformView,
    mark: &mut dyn FnMut(&Path) -> anyhow::Result<()>,
) -> anyhow::Result<GuardSummary> {
    let mut summary = GuardSummary::default();
    let mut empty = Vec::new();
    for row in rows {
        let Some(path) = crate::engine_bridge::EngineBridge::placeholder_path_under(sync_root, &row.path) else {
            db.set_mark_pending(&row.file_id, false)?;
            continue;
        };
        let seen = view(&path);
        // Review-157 F1 (spec row 22): a file that can't be read now (held by another app) may
        // hold bytes that read in sync; its mark stays pending and is retried every tick.
        if seen == PlatformView::Unreadable {
            db.set_mark_pending(&row.file_id, true)?;
            summary.mark_failed += 1;
            continue;
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
                continue;
            }
            summary.marked += 1;
        }
        db.set_mark_pending(&row.file_id, false)?;
    }
    summary.emptied = db.mark_no_fetched_content_many(&empty)?;
    Ok(summary)
}

/// Every tick (spec D2, D9): the marks still to be made. The guarded rows whose mark failed or
/// that joined the guarded set during the session ([`MARK_RETRIES_PER_TICK`] at most), then the
/// recorded orphans still to verify ([`verify_orphan`]). Returns how many marks were made.
pub(crate) fn retry_marks(
    db: &StateDb,
    sync_root: &Path,
    view: &dyn Fn(&Path) -> PlatformView,
    mark: &mut dyn FnMut(&Path) -> anyhow::Result<()>,
) -> anyhow::Result<usize> {
    let rows = db.list_mark_pending(MARK_RETRIES_PER_TICK)?;
    let mut marked = guard_rows(db, sync_root, &rows, view, mark)?.marked;
    for (path, _, pending) in db.list_unknown_orphans()? {
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
        let summary = guard_rows(&db, &root, &rows, &view, &mut |path: &Path| {
            calls.push(path.file_name().unwrap().to_string_lossy().into_owned());
            if fail_l && path.ends_with("l.txt") {
                fail_l = false;
                anyhow::bail!("in use")
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(summary, GuardSummary { marked: 2, emptied: 0, mark_failed: 1 });
        // The next tick retries only the failed one.
        let mut retried = Vec::new();
        let marked = retry_marks(&db, &root, &view, &mut |path: &Path| {
            retried.push(path.file_name().unwrap().to_string_lossy().into_owned());
            Ok(())
        })
        .unwrap();
        assert_eq!((marked, retried), (1, vec!["l.txt".to_string()]));
        let again = retry_marks(&db, &root, &view, &mut |_: &Path| -> anyhow::Result<()> { panic!("nothing is pending") }).unwrap();
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
        let marked = retry_marks(&db, &root, &|_: &Path| placeholder(OnDisk::Wholly, true), &mut |_: &Path| {
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
        retry_marks(&db, &root, &view, &mut |_: &Path| {
            marks += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(marks, 1, "marked not in sync");
        assert_eq!(db.list_unknown_orphans().unwrap(), vec![("u.txt".into(), "u".into(), false)], "kept on record, named by its identity");
        // A placeholder whose identity is the file at that path is no orphan.
        db.put_unknown_orphan("u2.txt", "u", true).unwrap();
        retry_marks(&db, &root, &|_: &Path| PlatformView::Placeholder { on_disk: OnDisk::Wholly, in_sync: true, identity: Some("u".into()) }, &mut |_: &Path| Ok(())).unwrap();
        assert!(db.list_unknown_orphans().unwrap().iter().all(|(path, _, _)| path != "u2.txt"), "forgotten");
    }

    /// Review-157 F1 (spec row 22): a file held by another app can't be read now; its mark stays
    /// pending and is tried again at the next tick, and an unreadable orphan stays on record.
    #[test]
    fn task_2191_an_unreadable_file_keeps_its_mark_pending() {
        let (dir, db) = db_with(&[("u", "u.pst", FileStatus::Local)]);
        let root = dir.path().join("root");
        let rows = db.list_guarded().unwrap();
        let summary = guard_rows(&db, &root, &rows, &|_: &Path| PlatformView::Unreadable, &mut |_: &Path| -> anyhow::Result<()> { panic!("no mark") }).unwrap();
        assert_eq!(summary.mark_failed, 1);
        let mut marks = 0;
        retry_marks(&db, &root, &|_: &Path| placeholder(OnDisk::Wholly, true), &mut |_: &Path| {
            marks += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(marks, 1, "readable again: marked at the next tick");
        db.put_unknown_orphan("old.pst", "u", true).unwrap();
        retry_marks(&db, &root, &|_: &Path| PlatformView::Unreadable, &mut |_: &Path| Ok(())).unwrap();
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
        retry_marks(&db, &root, &view, &mut |_: &Path| {
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
        let marked = retry_marks(&db, &root, &view, &mut |_: &Path| -> anyhow::Result<()> { panic!("no mark") }).unwrap();
        assert_eq!(marked, 0);
        assert!(db.list_unknown_orphans().unwrap().is_empty());
    }

    /// D9 / U4: the guard runs before the first tick and is awaited; every tick retries the
    /// pending marks before the stamp pass (a source pin: the runner calls them on Windows only).
    #[test]
    fn task_2191_the_guard_runs_before_the_first_tick_and_every_tick_before_the_stamp() {
        let runner = include_str!("runner.rs");
        let production = runner.split("\nmod tests {").next().unwrap();
        let at = |needle: &str| production.find(needle).unwrap_or_else(|| panic!("{needle} not found"));
        let seed = at("crate::windows_cf::seed_placeholders(bridge.clone(), &sync_root);");
        let guard = at("crate::windows_cf::guard_unknown_placeholders(&guard_root)).await");
        let tick = at("let mut tick = tokio::time::interval(TICK_INTERVAL);");
        let after = |from: usize, needle: &str| from + production[from..].find(needle).unwrap_or_else(|| panic!("{needle} not found"));
        const PAUSED: &str = "if sync_paused.load(Ordering::Relaxed) {";
        let retry = after(tick, "crate::windows_cf::retry_pending_marks(&mark_root)).await");
        let paused = after(retry, PAUSED);
        let stamp = after(paused, "crate::windows_cf::stamp_local_files_in_sync(&stamp_root);");
        assert!(seed < guard && guard < tick, "the guard runs after seeding and before the tick loop, awaited");
        assert!(
            !production[tick..retry].contains(PAUSED),
            "every tick, paused included: the pending marks come before the loop's pause check (review-157 F2)"
        );
        assert!(paused < stamp);
        assert_eq!(production.matches("retry_pending_marks(").count(), 1, "one call, on every tick");
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
