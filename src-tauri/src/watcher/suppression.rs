//! `suppression` items split out of `watcher.rs`.

use super::*;

// ── Engine-delete suppression set (task 0806) ──────────────────────────────────
//
// The REMOTE→local deletion reconcile (`engine_bridge` → `windows_cf::delete_placeholder`)
// removes an on-disk CF placeholder when a file/folder was trashed/deleted on
// another client. Removing that placeholder fires `NOTIFY_DELETE_COMPLETION`,
// which lands here as `NotifyEvent::Delete` → `handle_delete` → a redundant server
// `TrashFile` op. That would be wrong (the server ALREADY trashed it) and is a
// feedback loop.
//
// The PRIMARY guard is ordering: the engine removes the DB ROW *before* the
// placeholder, so `handle_delete`'s `get_file_by_path` returns `Ok(None)` and
// queues nothing. This set is the DETERMINISTIC second guard for the window where
// the row may not be observable yet (and to drop the event before `handle_delete`
// runs at all): the engine registers each path it is about to delete via
// [`suppress_engine_delete`], and the debounce loop consults [`take_engine_delete_suppressed`]
// on every `Delete` event — a hit means "engine-originated, do not propagate".
//
// Mirrors the `windows_cf::NOTIFY_TX` static: an `extern "system"` callback / a deep engine
// call site can't thread a handle, so a process-global is the simplest hop. The
// set is small (only paths mid-delete) and self-draining (entries are consumed on
// the matching Delete event, or swept by [`prune_stale_engine_suppressions`]).
pub(super) static ENGINE_DELETE_SUPPRESS: OnceLock<Mutex<HashMap<PathBuf, Instant>>> = OnceLock::new();

pub(super) fn engine_delete_suppress() -> &'static Mutex<HashMap<PathBuf, Instant>> {
    ENGINE_DELETE_SUPPRESS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Register `path` as an ENGINE-ORIGINATED delete so the watcher drops the
/// `NOTIFY_DELETE_COMPLETION` it will fire instead of queuing a redundant server
/// trash. Call this IMMEDIATELY BEFORE `windows_cf::delete_placeholder` on the
/// remote-deletion reconcile path (task 0806). Idempotent; refreshes the timer.
pub fn suppress_engine_delete(path: &Path) {
    if let Ok(mut set) = engine_delete_suppress().lock() {
        set.insert(path.to_path_buf(), Instant::now());
    }
}

/// If `path` was registered by [`suppress_engine_delete`], CONSUME the entry and
/// return `true` (the delete is engine-originated → the watcher must NOT queue a
/// server trash). Returns `false` for a genuine user delete. Consuming on read
/// keeps the set self-draining so a later user delete of the same path is honoured.
pub(super) fn take_engine_delete_suppressed(path: &Path) -> bool {
    match engine_delete_suppress().lock() {
        Ok(mut set) => set.remove(path).is_some(),
        Err(_) => false,
    }
}

/// Task 1640 (review-151d P3-1): the engine's own remove registered for `path` did not happen (it
/// failed): drop its registration, so a later delete by the person is honoured.
pub fn cancel_engine_delete_suppression(path: &Path) {
    let _ = take_engine_delete_suppressed(path);
}

/// Drop suppression entries older than this. A registered engine delete whose
/// NOTIFY never arrives (revert/remove failed, or the OS coalesced the event)
/// must not linger and swallow a genuine LATER user delete of the same path.
pub(super) const ENGINE_SUPPRESS_TTL: Duration = Duration::from_secs(30);

/// Evict suppression entries older than [`ENGINE_SUPPRESS_TTL`]. Called from the
/// debounce loop's periodic tick so the set can never grow unbounded or shadow a
/// real user delete indefinitely.
pub(super) fn prune_stale_engine_suppressions() {
    if let Ok(mut set) = engine_delete_suppress().lock() {
        let now = Instant::now();
        set.retain(|_, registered| now.duration_since(*registered) < ENGINE_SUPPRESS_TTL);
    }
}
