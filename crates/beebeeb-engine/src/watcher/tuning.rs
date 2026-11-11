//! `tuning` items split out of `watcher.rs`.

use super::*;

/// Debounce window: coalesce a burst of writes to the same path (an editor
/// saving in several `write` syscalls, a copy streaming in) into a single
/// upload, and give the writer time to finish before we stage the file. The
/// repo's documented file-watcher debounce is 100ms; we use a slightly larger
/// settle window so a large file finishes landing before we stage it.
///
/// `CF_CALLBACK_TYPE_NOTIFY_FILE_CLOSE_COMPLETION` is far less chatty than
/// ReadDirectoryChanges (one event per handle-close, not per write syscall),
/// but a single logical save can still close several handles, so we still
/// path-key debounce.
pub(super) const DEBOUNCE: Duration = Duration::from_millis(400);

/// How often the debounce loop wakes to flush paths whose settle window has
/// elapsed. Small relative to [`DEBOUNCE`] so the effective latency is close to
/// the debounce window itself.
pub(super) const DEBOUNCE_TICK: Duration = Duration::from_millis(100);

/// How often the enumeration scan walks the whole sync root looking for new
/// local files (the CREATE trigger — see the module docs on why CF NOTIFY
/// callbacks don't fire for brand-new plain files). 8s is the sweet spot: short
/// enough that a dropped file uploads within roughly one interval, long enough
/// that the recursive `read_dir` walk of a (small) vault is negligible overhead.
/// Each scan is awaited to completion before the next is scheduled (the loop
/// ticks AFTER the previous walk returns), so scans can never overlap or pile up
/// even if a walk ever runs long.
pub(super) const SCAN_INTERVAL: Duration = Duration::from_secs(8);

/// Hard cap on directory recursion depth, so a pathological deep tree (or a
/// symlink cycle that slipped past the symlink skip) can never make a scan run
/// unbounded. The vault nests a handful of levels in practice; 64 is far beyond
/// any real layout while still finite.
pub(super) const MAX_SCAN_DEPTH: usize = 64;

/// Max NEW file uploads a single scan pass may enqueue (task 0811 throttle).
///
/// A first enable of a large known folder (e.g. Music, 5,344 files) would
/// otherwise enqueue thousands of `upload_version` ops in ONE pass, flooding the
/// queue and the upload workers and pinning the machine — exactly the overload
/// the founder hit. Bounding new enqueues per pass spreads the work across scans
/// (one pass every `SCAN_INTERVAL`), so backup ramps up gradually instead of
/// thundering. The on-disk mirror already caps its copy volume per pass; this is
/// the matching cap on the upload side. Files not enqueued this pass are simply
/// picked up by the next one (the walk re-sees them — nothing is lost). Folder
/// scaffolding is NOT counted against this cap: folders are cheap metadata ops
/// and must keep pace with their files so parents always exist first.
pub(super) const MAX_NEW_UPLOADS_PER_SCAN: usize = 256;

/// Task 2229: how long the scan trusts a settled verdict before it classifies every file again.
/// The listing misses bytes rewritten with the old size and modification time on Windows, and a
/// file still open for writing; this bounds both. The full pass compares the bytes by a fingerprint
/// a restored time cannot fool ([`crate::engine_bridge::ContentFingerprint`], task 2232).
pub(super) const FULL_RECHECK_INTERVAL: Duration = Duration::from_secs(10 * 60);
