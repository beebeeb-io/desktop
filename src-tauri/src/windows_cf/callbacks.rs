//! Cloud Files callbacks — Windows calls these `extern "system"`
//! functions when the user touches a cloud-only placeholder. We
//! translate the callback shape into a hydration request against the
//! live `EngineBridge`.
//!
//! Spec: `docs/superpowers/plans/2026-05-07-desktop-sync-client.md`
//! Phase 2 Task 6.
//!
//! ## What Windows expects
//!
//! - `CF_CALLBACK_INFO` carries `FileIdentity` (the blob we stored
//!   when minting the placeholder — for us, UTF-8 bytes of the file's
//!   UUID), the `ConnectionKey`, and the `TransferKey`. The latter two
//!   are required by the `CfExecute(TRANSFER_DATA)` call that streams
//!   plaintext bytes back to the Cloud Files runtime.
//! - `CF_CALLBACK_PARAMETERS.FetchData` carries the byte range Windows
//!   wants: `RequiredFileOffset` / `RequiredLength` (we MUST satisfy at
//!   least this) plus an `Optional*` hint we may over-serve. We hydrate
//!   the covering chunk span via `EngineBridge::hydrate_file_range`
//!   (falling back to `hydrate_file_to_memory` when range math is
//!   unavailable), then splice the required range back with
//!   `CfExecute(TRANSFER_DATA)`.
//!
//! ## Threading
//!
//! Cloud Files callbacks are delivered on a worker thread the
//! filter-driver picks. That thread is *not* attached to any tokio
//! runtime, so `Handle::try_current()` returns `Err` there. We instead
//! reach the daemon's runtime handle stashed by [`super::connect_root`]
//! (via [`super::runtime`]) and `block_on` the hydration. Blocking the
//! worker for the full hydration is OK; the Windows runtime keeps the
//! placeholder marked "loading" without UI-thread impact.
//!
//! ## Failure handling
//!
//! Every exit path that cannot deliver bytes calls
//! [`fail_transfer`], which issues `CfExecute(TRANSFER_DATA)` with a
//! non-success `CompletionStatus` on the required range. Without this,
//! Explorer's download spinner hangs forever. A bare `return;` is never
//! correct once we have a valid `ConnectionKey` + `TransferKey`.
//!
//! ## Security
//!
//! Decrypted plaintext is NEVER written to disk. Range and whole-file
//! hydration return [`zeroize::Zeroizing`] buffers that overwrite themselves
//! on drop. We never log plaintext, keys, or tokens.

#![cfg(target_os = "windows")]

use std::os::windows::ffi::OsStrExt;

use windows::Win32::Foundation::{NTSTATUS, STATUS_SUCCESS, STATUS_UNSUCCESSFUL};
use windows::Win32::Storage::CloudFilters::*;
use windows::Win32::System::CorrelationVector::CORRELATION_VECTOR;

use crate::engine_bridge::{
    HydratedFileMemory, HydratedRange, HydrationCoverageEvidence, HydrationSizeEvidence, SizeDecision,
    decide_hydration_size_action, should_mark_in_sync_after_hydration,
};

/// Chunk size for the `CfExecute(TRANSFER_DATA)` loop. Cloud Files
/// requires every non-final transfer's `Offset` and `Length` to be a
/// multiple of the volume sector/page size; 1 MiB is a safe multiple of
/// the 4 KiB page size and keeps a single buffer in memory at a time for
/// large files. The final chunk (the one that reaches the end of the
/// requested range) may be shorter / unaligned.
const TRANSFER_CHUNK: i64 = 1024 * 1024;

enum FetchHydration {
    Range(HydratedRange),
    WholeFile(HydratedFileMemory),
}

impl FetchHydration {
    fn size_evidence(&self, required_offset: i64) -> HydrationSizeEvidence {
        match self {
            FetchHydration::Range(range) => HydrationSizeEvidence::Range {
                server_size_bytes: range.file_size_bytes,
                required_offset,
            },
            FetchHydration::WholeFile(file) => HydrationSizeEvidence::WholeFile {
                server_size_bytes: file.server_size_bytes,
                plaintext_len: file.data.len() as i64,
            },
        }
    }

    fn coverage_evidence(
        &self,
        required_offset: i64,
        required_length: i64,
        file_size: i64,
    ) -> HydrationCoverageEvidence {
        match self {
            FetchHydration::Range(range) => HydrationCoverageEvidence::Range {
                covers_whole_file: range.covers_whole_file,
            },
            FetchHydration::WholeFile(_) => HydrationCoverageEvidence::WholeFileFallback {
                required_offset,
                required_length,
                file_size,
            },
        }
    }
}

/// Windows-installed fetch callback. Registered via `CfConnectSyncRoot`
/// in [`super::connect_callbacks`]. Windows fires this when a user opens
/// a cloud-only placeholder and the requested range isn't hydrated yet.
///
/// SAFETY: `callback_info` and `callback_parameters` are valid for the
/// duration of this call per the Cloud Files API contract. We copy the
/// scalar fields we need into owned values before doing any blocking
/// work, so nothing borrowed from Windows is held across the
/// `block_on` boundary.
pub unsafe extern "system" fn fetch_data_callback(
    callback_info: *const CF_CALLBACK_INFO,
    callback_parameters: *const CF_CALLBACK_PARAMETERS,
) {
    // Defensive: null pointers here would be an OS bug, but it costs
    // nothing to refuse to dereference. With no info we have no transfer
    // key, so there's nothing to fail against — just bail.
    if callback_info.is_null() || callback_parameters.is_null() {
        tracing::warn!("fetch_data_callback fired with null CF_CALLBACK_INFO/PARAMETERS");
        return;
    }
    let info = unsafe { &*callback_info };
    let params = unsafe { &*callback_parameters };

    // The connection + transfer keys identify the in-flight hydration to
    // the Cloud Files runtime. Copy them out so we can satisfy or fail
    // the transfer regardless of which exit path we take below.
    let connection_key = info.ConnectionKey;
    let transfer_key = info.TransferKey;
    let request_key = info.RequestKey;
    let file_size = info.FileSize;

    // The fetch range. We must satisfy at least the *required* span; the
    // optional span is a read-ahead hint we may over-serve. Copy scalars
    // out of the union immediately.
    let fetch = unsafe { params.Anonymous.FetchData };
    let required_offset = fetch.RequiredFileOffset;
    let required_length = fetch.RequiredLength;

    // Decode FileIdentity → UTF-8 file_id. Stored as raw UTF-8 bytes in
    // `placeholders::create_placeholder`, so no wide-string round trip.
    let identity_bytes =
        unsafe { std::slice::from_raw_parts(info.FileIdentity as *const u8, info.FileIdentityLength as usize) };
    let file_id = match std::str::from_utf8(identity_bytes) {
        Ok(s) => s.to_owned(),
        Err(e) => {
            tracing::warn!(error = %e, "FileIdentity is not valid UTF-8 — failing hydrate");
            unsafe {
                fail_transfer(
                    connection_key,
                    transfer_key,
                    request_key,
                    required_offset,
                    required_length,
                )
            };
            return;
        }
    };

    // Get the live engine bridge. Set by `runner::run` before any
    // placeholder exists, so a missing bridge means the daemon isn't
    // running or a logout raced this callback. Either way we can't
    // hydrate — fail the transfer so Explorer stops spinning.
    let bridge = match super::bridge() {
        Some(b) => b,
        None => {
            tracing::warn!(file_id = %file_id, "fetch callback fired but no EngineBridge registered");
            unsafe {
                fail_transfer(
                    connection_key,
                    transfer_key,
                    request_key,
                    required_offset,
                    required_length,
                )
            };
            return;
        }
    };

    // Reach the daemon's tokio runtime. The filter-driver worker thread
    // we're on has no runtime attached, so `Handle::try_current()` would
    // fail — we use the handle stashed in `connect_root()` instead.
    let handle = match super::runtime() {
        Some(h) => h,
        None => {
            tracing::error!(
                file_id = %file_id,
                "no tokio runtime handle registered; cannot hydrate Cloud Files request"
            );
            unsafe {
                fail_transfer(
                    connection_key,
                    transfer_key,
                    request_key,
                    required_offset,
                    required_length,
                )
            };
            return;
        }
    };

    let range_offset = match u64::try_from(required_offset) {
        Ok(offset) => offset,
        Err(_) => {
            tracing::warn!(file_id = %file_id, required_offset, "negative Cloud Files fetch offset");
            unsafe {
                fail_transfer(
                    connection_key,
                    transfer_key,
                    request_key,
                    required_offset,
                    required_length,
                )
            };
            return;
        }
    };
    let range_length = match u64::try_from(required_length) {
        Ok(length) => length,
        Err(_) => {
            tracing::warn!(file_id = %file_id, required_length, "negative Cloud Files fetch length");
            unsafe {
                fail_transfer(
                    connection_key,
                    transfer_key,
                    request_key,
                    required_offset,
                    required_length,
                )
            };
            return;
        }
    };

    // Decrypt only the chunk span that covers the requested range — plaintext
    // is NEVER written to disk. If the range metadata is insufficient for
    // bounded hydration, fall back to the existing whole-file in-memory path.
    // Every success path carries a Zeroizing buffer that wipes itself on drop.
    let hydration = handle.block_on(async {
        match bridge.hydrate_file_range(&file_id, range_offset, range_length).await {
            Ok(Some(range)) => Ok(FetchHydration::Range(range)),
            Ok(None) => bridge
                .hydrate_file_to_memory_with_metadata(&file_id)
                .await
                .map(FetchHydration::WholeFile),
            Err(e) => Err(e),
        }
    });
    let hydration = match hydration {
        Ok(hydration) => hydration,
        Err(e) => {
            // Do NOT log the error's inner content at info+ if it could carry
            // decrypted bytes; hydration errors are status strings only.
            tracing::warn!(file_id = %file_id, error = %e, "Cloud Files range hydration failed");
            unsafe {
                fail_transfer(
                    connection_key,
                    transfer_key,
                    request_key,
                    required_offset,
                    required_length,
                )
            };
            return;
        }
    };

    // Size sanity: the server's fresh file size must match the size Windows
    // recorded on the placeholder (info.FileSize). For range hydration the
    // decrypted buffer is intentionally smaller than the file, so comparing
    // buffer length to file_size would make every ordinary partial fetch look
    // like the production 0783 mismatch. Whole-file fallback uses the same
    // server metadata when available and only falls back to plaintext length
    // when the metadata lacks `size_bytes`.
    //
    // RESOLVE-OR-ERROR (task 0783) — never silently fail:
    //
    //   Tier-1 (resolve): hydration fetched authenticated bytes and fresh file
    //   metadata; `corrected_size` is the trusted logical plaintext size.
    //   We cannot resize the placeholder DURING this fetch (the file is in-use
    //   by the active transfer; CfUpdatePlaceholder would return
    //   ERROR_CLOUD_FILE_IN_USE), so we fail THIS fetch cleanly, then correct
    //   the size out-of-band — resize the CF placeholder to the corrected
    //   logical length AND rewrite the state_db size_bytes to it. The next open fires
    //   a fresh, now-aligned FETCH_DATA and hydrates successfully, so the file
    //   becomes openable at its true size.
    //
    //   Tier-2 (visible error): if the out-of-band correction itself can't be
    //   applied (the resize failed — the placeholder is gone, or some other CF
    //   error), we still must not be silent: flip the row to Error and mark the
    //   placeholder NOT in-sync so Explorer shows a non-synced/error overlay
    //   rather than a green check on a file that won't open. The failed
    //   transfer (below) already surfaces an error on this open.
    if let SizeDecision::ResolveTo { corrected_size } =
        decide_hydration_size_action(hydration.size_evidence(required_offset), file_size)
    {
        tracing::warn!(
            file_id = %file_id,
            corrected_size,
            file_size,
            "server size != placeholder size; resolving placeholder to the authenticated logical length"
        );

        // Drop plaintext here — the resolution path works from the size only,
        // never the bytes. The Zeroizing drop wipes the allocation.
        drop(hydration);

        // Fail THIS fetch cleanly first: the file must not be mid-hydration
        // when we resize, and Explorer must stop spinning on this open.
        unsafe {
            fail_transfer(
                connection_key,
                transfer_key,
                request_key,
                required_offset,
                required_length,
            )
        };

        // Tier-1: correct the placeholder + DB to the trusted logical length so
        // the NEXT open hydrates against an aligned size. `&*bridge` derefs the
        // Arc to a plain `&EngineBridge` (a `&Arc<_>` would not coerce here).
        unsafe { resolve_size_mismatch(info, &*bridge, &file_id, corrected_size) };
        return;
    }

    // Splice the required range back into the placeholder. The hydration buffer
    // is kept alive here so the data is valid for every CfExecute call inside
    // transfer_range.
    let transfer_ok = match &hydration {
        FetchHydration::Range(range) => {
            let range_start = match i64::try_from(range.range_start_bytes) {
                Ok(range_start) => range_start,
                Err(_) => {
                    tracing::warn!(
                        file_id = %file_id,
                        range_start_bytes = range.range_start_bytes,
                        "range start exceeds Cloud Files i64 offset range"
                    );
                    unsafe {
                        fail_transfer(
                            connection_key,
                            transfer_key,
                            request_key,
                            required_offset,
                            required_length,
                        )
                    };
                    return;
                }
            };
            unsafe {
                transfer_range(
                    connection_key,
                    transfer_key,
                    request_key,
                    &range.data,
                    range_start,
                    required_offset,
                    required_length,
                )
            }
        }
        FetchHydration::WholeFile(file) => unsafe {
            transfer_range(
                connection_key,
                transfer_key,
                request_key,
                &file.data,
                0,
                required_offset,
                required_length,
            )
        },
    };

    let should_mark_in_sync =
        should_mark_in_sync_after_hydration(hydration.coverage_evidence(required_offset, required_length, file_size));

    // Drop plaintext now — transfer_range has consumed all bytes it needs.
    // The Zeroizing wrapper wipes the plaintext allocation on drop.
    drop(hydration);

    match transfer_ok {
        Ok(()) => {
            tracing::info!(file_id = %file_id, "hydrated and transferred Cloud Files range");
            // Best-effort: mark the placeholder IN_SYNC only when the
            // hydration result actually covered every chunk in the file.
            // Requested span shape is no longer enough once bounded range
            // hydration is live.
            if should_mark_in_sync {
                if let Some(path) = normalized_path(info) {
                    unsafe { mark_in_sync(&path) };
                }
            }
        }
        Err(e) => {
            tracing::warn!(file_id = %file_id, error = %e, "CfExecute(TRANSFER_DATA) failed");
            // The transfer call itself failed; tell Explorer to stop
            // spinning. If this also fails there's nothing more we can do.
            unsafe {
                fail_transfer(
                    connection_key,
                    transfer_key,
                    request_key,
                    required_offset,
                    required_length,
                )
            };
        }
    }
}

/// Resolve a hydration size mismatch out-of-band (task 0783, Tier-1) with a
/// Tier-2 visible-error fallback. Called AFTER the offending fetch has been
/// failed cleanly (so the placeholder is no longer mid-hydration and a resize
/// is API-legal).
///
/// `true_size` is the AES-256-GCM-authenticated decrypted plaintext length —
/// ground truth. We:
///
///   1. Rewrite the state_db `size_bytes` to `true_size` so the row (and every
///      other client reading the local mirror) reflects the real size, and so
///      a future placeholder (re)creation mints the aligned size.
///   2. Resolve the on-disk placeholder path (see below), resize the CF
///      placeholder to `true_size` and re-mark it in-sync, so the NEXT open
///      fires an aligned FETCH_DATA that succeeds.
///
/// ## Path resolution (the 0783 follow-up fix)
///
/// The FETCH_DATA callback's `CF_CALLBACK_INFO.NormalizedPath` is only
/// populated when the root is connected with
/// `CF_CONNECT_FLAG_REQUIRE_FULL_FILE_PATH`, which we do NOT set — so on a real
/// fetch `normalized_path(info)` returns `None` and the resize was previously
/// never attempted (Tier-1 was dead; only Tier-2 fired). We now fall back to
/// reconstructing the path from the state DB: `<sync_root>/<entry.path>`, the
/// SAME on-disk layout `windows_cf::populate_placeholders` mints. A path-safety
/// guard (see [`db_placeholder_path`]) rejects any relative path that escapes
/// the sync root before we touch it.
///
/// If the path cannot be resolved by EITHER route, or the resize genuinely
/// fails, we DO NOT leave the file silently broken: flip the row to `Error` and
/// (if we have a wide path) mark the placeholder NOT in-sync so Explorer
/// surfaces a non-synced/error overlay (Tier-2). The earlier `fail_transfer`
/// already surfaced an error on the open that triggered this.
///
/// Best-effort throughout: every DB / CF failure is logged (without sensitive
/// content) and swallowed — there is no caller to propagate to (this runs at
/// the tail of a fire-and-forget `extern "system"` callback).
///
/// SAFETY: `info` is the live `CF_CALLBACK_INFO` for this callback; we only
/// read its path fields, which are valid for the duration of the call.
unsafe fn resolve_size_mismatch(
    info: &CF_CALLBACK_INFO,
    bridge: &crate::engine_bridge::EngineBridge,
    file_id: &str,
    true_size: i64,
) {
    // Step 1 — correct the recorded size to the authenticated length. A failure
    // here is non-fatal: the placeholder resize below is what makes the next
    // open succeed; the DB value is for cross-surface consistency + reseeding.
    match bridge.db().set_size_bytes(file_id, true_size) {
        Ok(0) => {
            tracing::warn!(file_id = %file_id, "size correction matched no row (file_id absent from state DB)");
        }
        Ok(_) => {}
        Err(e) => {
            tracing::warn!(file_id = %file_id, error = %e, "could not correct state DB size_bytes");
        }
    }

    // Resolve the on-disk placeholder path. PRIMARY: the callback's
    // NormalizedPath (a NUL-terminated wide string we strip below). FALLBACK
    // (the common case — we connect without REQUIRE_FULL_FILE_PATH so this is
    // usually empty): reconstruct `<sync_root>/<state-DB rel path>` by file_id.
    // We keep the wide form (when available) so Tier-2's mark_not_in_sync can
    // reuse the exact openable path the OS gave us.
    let mut wide_path: Option<Vec<u16>> = normalized_path(info);
    let os_path: Option<std::path::PathBuf> = match &wide_path {
        Some(path) => {
            let path_no_nul: Vec<u16> = path.iter().copied().take_while(|&c| c != 0).collect();
            Some(std::path::PathBuf::from(String::from_utf16_lossy(&path_no_nul)))
        }
        None => db_placeholder_path(bridge, file_id),
    };

    let Some(os_path) = os_path else {
        // Neither route yielded a path — cannot resize. Tier-2: visible Error.
        // (No wide path either, so no overlay to flip beyond the row status.)
        tracing::warn!(file_id = %file_id, "no placeholder path (callback + state DB both unresolved); marking Error");
        let _ = bridge.db().set_status(file_id, crate::state_db::FileStatus::Error);
        return;
    };

    // If we resolved via the DB (no wide path from the callback), build a
    // NUL-terminated wide form too so a Tier-2 fallback can mark_not_in_sync the
    // same on-disk file.
    if wide_path.is_none() {
        let mut w: Vec<u16> = os_path.as_os_str().encode_wide().collect();
        w.push(0);
        wide_path = Some(w);
    }

    // Step 2 (Tier-1) — resize the placeholder to the authenticated length. On
    // success the next open hydrates against the aligned size and the file is
    // openable.
    match super::placeholders::resize_placeholder(&os_path, true_size) {
        Ok(()) => {
            // The placeholder is now an in-sync, cloud-only stub at the correct
            // size — no plaintext bytes were delivered to it (this fetch was
            // failed). `hydrate_file_to_memory` had optimistically flipped the row
            // to `Local`, so flip it back to `CloudOnly` to match on-disk reality.
            // The next open fires a fresh, aligned FETCH_DATA that hydrates
            // successfully.
            let _ = bridge.db().set_status(file_id, crate::state_db::FileStatus::CloudOnly);
            tracing::info!(file_id = %file_id, "resolved size mismatch; placeholder resized to decrypted length");
        }
        Err(e) => {
            // Tier-2: the resize could not be applied — never leave it silent.
            tracing::warn!(file_id = %file_id, error = %e, "placeholder resize failed; marking Error + not-in-sync");
            let _ = bridge.db().set_status(file_id, crate::state_db::FileStatus::Error);
            if let Some(path) = wide_path.as_deref() {
                unsafe { mark_not_in_sync(path) };
            }
        }
    }
}

// NOTE: `zero_bytes` and `secure_remove_temp` have been removed. The hydration
// path now uses `Zeroizing<Vec<u8>>` (automatic wipe on drop) and never writes
// plaintext to disk, so neither function is needed.

/// Reconstruct the on-disk placeholder path for `file_id` from the state DB:
/// `<sync_root>/<entry.path>` using the SAME mapping `populate_placeholders`
/// uses (`trim_matches('/')` → `replace('/', MAIN_SEPARATOR)`). Used as the
/// fallback when the FETCH_DATA callback's `NormalizedPath` is unavailable
/// (task 0783).
///
/// PATH SAFETY: the relative path comes from server-controlled metadata, so we
/// reject any component that could escape the sync root — an empty rel path, an
/// absolute/root-prefixed component, or any `..` / `.` / non-normal component.
/// Only when the rel path is a clean sequence of normal components do we join
/// it onto the sync root. Returns `None` (→ Tier-2) on any failure: missing
/// sync root, absent row, empty/illegal rel path.
fn db_placeholder_path(bridge: &crate::engine_bridge::EngineBridge, file_id: &str) -> Option<std::path::PathBuf> {
    let sync_root = super::sync_root()?;

    let entry = match bridge.db().get_file(file_id) {
        Ok(Some(e)) => e,
        Ok(None) => {
            tracing::warn!(file_id = %file_id, "size-resolve fallback: file_id absent from state DB");
            return None;
        }
        Err(e) => {
            tracing::warn!(file_id = %file_id, error = %e, "size-resolve fallback: state DB read failed");
            return None;
        }
    };

    match safe_join_under_root(&sync_root, &entry.path) {
        Some(p) => Some(p),
        None => {
            tracing::warn!(file_id = %file_id, "size-resolve fallback: rejecting unsafe/empty relative path");
            None
        }
    }
}

/// Join a server-relative, '/'-separated `rel_path` onto `sync_root`, returning
/// the on-disk path ONLY if it is provably contained under the root. Pure +
/// unit-testable (no CF / DB state). Mirrors `populate_placeholders`'
/// `trim_matches('/')` → `replace('/', MAIN_SEPARATOR)` mapping, but rejects:
///   - an empty rel path (would resolve to the root itself),
///   - any segment that is empty, `.`, or `..` (traversal),
/// then re-checks lexical containment as belt-and-suspenders. Returns `None` on
/// any rejection so the caller falls through to Tier-2 (visible error).
fn safe_join_under_root(sync_root: &std::path::Path, rel_path: &str) -> Option<std::path::PathBuf> {
    let rel = rel_path.trim_matches('/');
    if rel.is_empty() {
        return None;
    }
    if rel.split('/').any(|seg| seg.is_empty() || seg == "." || seg == "..") {
        return None;
    }
    let native_rel = rel.replace('/', std::path::MAIN_SEPARATOR_STR);
    let candidate = sync_root.join(&native_rel);
    // The joined path must still be under the sync root. We compare lexically
    // (we avoid canonicalize() — it would resolve the placeholder's reparse
    // point; the per-segment check above already blocks traversal).
    if !candidate.starts_with(sync_root) {
        return None;
    }
    Some(candidate)
}

// ── Upload triggers — Cloud Files NOTIFY callbacks (task 0780) ───────────────
//
// These three `NOTIFY_*_COMPLETION` callbacks are the Windows UPLOAD path. They
// fire AFTER the corresponding local operation completes on a CfConnectSyncRoot'd
// root (where a `notify`/ReadDirectoryChanges watcher is starved by the CF
// filter and never fires). Each one extracts the path(s) from `CF_CALLBACK_INFO`
// (and, for rename, the params arm), then pushes a `crate::watcher::NotifyEvent`
// into the upload driver's debounce loop. ALL feedback-loop filtering +
// parent_id resolution + the actual `queue_finder_*` enqueue happen there, in
// the one shared `EngineBridge::classify_local_path` — these callbacks only
// translate the CF shape into a channel message and return fast.
//
// We do NOT `block_on` here: unlike `fetch_data_callback` (which must deliver
// bytes synchronously or Explorer hangs), a NOTIFY completion has no transfer to
// satisfy — dropping the work onto the async debounce loop keeps the
// filter-driver worker thread free and lets us debounce chatty close bursts.

/// `NOTIFY_FILE_CLOSE_COMPLETION` — fires after a handle that may have written a
/// new/modified file in the sync root closes. The create/modify upload trigger.
///
/// We skip closes flagged `CF_CALLBACK_CLOSE_COMPLETION_FLAG_DELETED` (the file
/// was deleted, not written — the DELETE_COMPLETION callback handles that), then
/// push the absolute path as a debounced `CloseCompletion` event. The pre-upload
/// `CfConvertToPlaceholder` (no MARK_IN_SYNC) and the `queue_finder_create`
/// happen in the debounce loop's `handle_settled_path`, immediately before
/// queueing — so a path that gets filtered out is never converted.
///
/// SAFETY: `callback_info`/`callback_parameters` are valid for the duration of
/// this call per the Cloud Files contract. We copy the path out before
/// returning; nothing borrowed from Windows is retained.
pub unsafe extern "system" fn notify_file_close_completion_callback(
    callback_info: *const CF_CALLBACK_INFO,
    callback_parameters: *const CF_CALLBACK_PARAMETERS,
) {
    if callback_info.is_null() {
        return;
    }
    let info = unsafe { &*callback_info };

    // The close-completion params arm carries a `Flags` field; a DELETED close
    // is not a write we should upload. Defensive null-check on params.
    if !callback_parameters.is_null() {
        let params = unsafe { &*callback_parameters };
        let close = unsafe { params.Anonymous.CloseCompletion };
        if (close.Flags & CF_CALLBACK_CLOSE_COMPLETION_FLAG_DELETED)
            != CF_CALLBACK_CLOSE_COMPLETION_FLAG_NONE
        {
            // Deleted on close — leave it to the delete-completion callback.
            return;
        }
    }

    let Some(path) = notify_full_path(info) else {
        return;
    };
    push_notify_event(crate::watcher::NotifyEvent::CloseCompletion(path));
}

/// `NOTIFY_DELETE_COMPLETION` — fires after a local delete completes. Pushes a
/// `Delete` event; the debounce loop resolves the path to a known server file
/// and enqueues the trash op (no-op if the path is untracked / engine-internal).
///
/// SAFETY: as [`notify_file_close_completion_callback`].
pub unsafe extern "system" fn notify_delete_completion_callback(
    callback_info: *const CF_CALLBACK_INFO,
    _callback_parameters: *const CF_CALLBACK_PARAMETERS,
) {
    if callback_info.is_null() {
        return;
    }
    let info = unsafe { &*callback_info };
    let Some(path) = notify_full_path(info) else {
        return;
    };
    push_notify_event(crate::watcher::NotifyEvent::Delete(path));
}

/// `NOTIFY_RENAME_COMPLETION` — fires after a local rename/move completes. The
/// OLD (source) path is in the params arm's `SourcePath`; the NEW (target) path
/// is `CF_CALLBACK_INFO.NormalizedPath`. Pushes a `Rename { source, target }`
/// event; the debounce loop resolves the source to a known server file and
/// enqueues a move (changed parent) or rename (name-only) via `queue_finder_modify`.
///
/// SAFETY: as above. `SourcePath` is a NUL-terminated PCWSTR valid for the
/// duration of the call; we copy it into an owned path before returning.
pub unsafe extern "system" fn notify_rename_completion_callback(
    callback_info: *const CF_CALLBACK_INFO,
    callback_parameters: *const CF_CALLBACK_PARAMETERS,
) {
    if callback_info.is_null() || callback_parameters.is_null() {
        return;
    }
    let info = unsafe { &*callback_info };
    let params = unsafe { &*callback_parameters };

    // NEW path: NormalizedPath (volume-relative) joined with the volume DOS name.
    let Some(target) = notify_full_path(info) else {
        return;
    };

    // OLD path: the rename-completion arm's SourcePath. Like NormalizedPath it is
    // VOLUME-RELATIVE (no drive letter), so prefix the same VolumeDosName to make
    // it openable / strippable against the sync root.
    let rename = unsafe { params.Anonymous.RenameCompletion };
    let Some(source) = volume_relative_to_full(info, rename.SourcePath.0) else {
        return;
    };

    push_notify_event(crate::watcher::NotifyEvent::Rename { source, target });
}

/// Join `CF_CALLBACK_INFO.NormalizedPath` with the volume DOS name into an
/// absolute [`PathBuf`] (e.g. `C:\Users\...\file`). Mirrors [`normalized_path`]
/// but yields a `PathBuf` (for the upload driver) instead of a NUL-terminated
/// wide string (for `CfOpenFileWithOplock`). Returns `None` if the relative path
/// is missing/empty.
fn notify_full_path(info: &CF_CALLBACK_INFO) -> Option<std::path::PathBuf> {
    volume_relative_to_full(info, info.NormalizedPath.0)
}

/// Join an arbitrary VOLUME-RELATIVE wide path (`\Users\...\file`, no drive)
/// with `info.VolumeDosName` (`C:`) into an absolute [`PathBuf`]. Shared by
/// [`notify_full_path`] and the rename source-path decode. Returns `None` for a
/// null/empty relative path.
fn volume_relative_to_full(info: &CF_CALLBACK_INFO, rel_ptr: *const u16) -> Option<std::path::PathBuf> {
    let rel = wide_to_vec(rel_ptr)?;
    if rel.is_empty() {
        return None;
    }
    let vol = wide_to_vec(info.VolumeDosName.0).unwrap_or_default();
    let mut full: Vec<u16> = Vec::with_capacity(vol.len() + rel.len());
    full.extend_from_slice(&vol);
    full.extend_from_slice(&rel);
    Some(std::path::PathBuf::from(String::from_utf16_lossy(&full)))
}

/// Push a NOTIFY event into the upload driver's debounce loop. No-op (with a
/// debug log) if `watcher::spawn` hasn't registered a sender yet, or if the
/// receiver was torn down (logout/shutdown) — a dropped event is reconciled by
/// the next `sync_tick`. Never blocks the filter-driver thread.
fn push_notify_event(event: crate::watcher::NotifyEvent) {
    match super::notify_sender() {
        Some(tx) => {
            if tx.send(event).is_err() {
                tracing::debug!("NOTIFY event dropped: upload driver receiver is gone");
            }
        }
        None => {
            tracing::debug!("NOTIFY event dropped: no upload driver sender registered yet");
        }
    }
}

/// Stream `[offset, offset+length)` of `plaintext` back into the placeholder
/// via `CfExecute(CF_OPERATION_TYPE_TRANSFER_DATA)`, chunked so each non-final
/// transfer is page-aligned. `plaintext_base_offset` is the absolute file
/// offset corresponding to `plaintext[0]`; it is `0` for whole-file fallback and
/// the first covering chunk's offset for range hydration. If the requested range
/// runs past the end of the decrypted data, we transfer what we have.
///
/// SAFETY: `connection_key` / `transfer_key` come from a live
/// `CF_CALLBACK_INFO`. `plaintext` outlives every `CfExecute` call (it's
/// borrowed for the whole function). Each `CfExecute` copies the buffer
/// synchronously before returning.
unsafe fn transfer_range(
    connection_key: CF_CONNECTION_KEY,
    transfer_key: i64,
    request_key: i64,
    plaintext: &[u8],
    plaintext_base_offset: i64,
    required_offset: i64,
    required_length: i64,
) -> windows::core::Result<()> {
    let total_start = plaintext_base_offset.max(0);
    let total_end = total_start.saturating_add(plaintext.len() as i64);
    // Clamp the requested range to what we actually have.
    let start = required_offset.clamp(total_start, total_end);
    let end = required_offset
        .saturating_add(required_length)
        .clamp(total_start, total_end);
    if end <= start {
        // Nothing to send for this range; report a zero-length success so
        // Cloud Files marks the request satisfied rather than aborted.
        return unsafe {
            transfer_one(
                connection_key,
                transfer_key,
                request_key,
                std::ptr::null(),
                start,
                0,
                STATUS_SUCCESS,
            )
        };
    }

    let mut off = start;
    while off < end {
        let remaining = end - off;
        let len = remaining.min(TRANSFER_CHUNK);
        let buf_index = (off - total_start) as usize;
        let buf_ptr = unsafe { plaintext.as_ptr().add(buf_index) } as *const core::ffi::c_void;
        unsafe {
            transfer_one(
                connection_key,
                transfer_key,
                request_key,
                buf_ptr,
                off,
                len,
                STATUS_SUCCESS,
            )
        }?;
        off += len;
    }
    Ok(())
}

/// Issue a non-success `CfExecute(TRANSFER_DATA)` so Explorer stops
/// spinning and surfaces an error for the requested range. Best-effort:
/// logs (without sensitive content) and swallows any error from the call
/// itself — there's no further recovery once hydration has failed.
///
/// SAFETY: keys come from a live `CF_CALLBACK_INFO`.
unsafe fn fail_transfer(
    connection_key: CF_CONNECTION_KEY,
    transfer_key: i64,
    request_key: i64,
    required_offset: i64,
    required_length: i64,
) {
    // Length is required by the API on a transfer-data op; a non-success
    // status on the required span tells Cloud Files to abort this fetch.
    let len = required_length.max(0);
    if let Err(e) = unsafe {
        transfer_one(
            connection_key,
            transfer_key,
            request_key,
            std::ptr::null(),
            required_offset,
            len,
            STATUS_UNSUCCESSFUL,
        )
    } {
        tracing::warn!(error = %e, "CfExecute(TRANSFER_DATA) failure-report itself failed");
    }
}

/// Low-level single `CfExecute(CF_OPERATION_TYPE_TRANSFER_DATA)`. A
/// success status with a real buffer delivers bytes; a non-success
/// status aborts the fetch. `buffer` may be null when `length == 0` or
/// when reporting a failure.
///
/// SAFETY: `connection_key` / `transfer_key` identify a live transfer;
/// `buffer` (if non-null) is valid for `length` bytes for the duration
/// of the call. `CfExecute` consumes the buffer synchronously.
unsafe fn transfer_one(
    connection_key: CF_CONNECTION_KEY,
    transfer_key: i64,
    request_key: i64,
    buffer: *const core::ffi::c_void,
    offset: i64,
    length: i64,
    status: NTSTATUS,
) -> windows::core::Result<()> {
    let op_info = CF_OPERATION_INFO {
        StructSize: std::mem::size_of::<CF_OPERATION_INFO>() as u32,
        Type: CF_OPERATION_TYPE_TRANSFER_DATA,
        ConnectionKey: connection_key,
        TransferKey: transfer_key,
        CorrelationVector: std::ptr::null::<CORRELATION_VECTOR>(),
        SyncStatus: std::ptr::null::<CF_SYNC_STATUS>(),
        // Plumb the callback's RequestKey through so CfExecute correlates
        // this transfer with the originating fetch request. TransferKey is
        // the primary correlator, but a meaningful RequestKey shouldn't be
        // discarded — a 0 here is the first suspect if Explorer hangs
        // despite a STATUS_SUCCESS transfer.
        RequestKey: request_key,
    };

    let mut op_params = CF_OPERATION_PARAMETERS {
        // Per the Cloud Files contract, ParamSize is the size of the
        // specific operation arm in use (TransferData), NOT the full
        // union. CF_OPERATION_PARAMETERS_0_6 is not the largest arm, so
        // size_of::<CF_OPERATION_PARAMETERS>() would be oversized; compute
        // the arm size exactly: offset of the union + size of the arm.
        ParamSize: (std::mem::offset_of!(CF_OPERATION_PARAMETERS, Anonymous)
            + std::mem::size_of::<CF_OPERATION_PARAMETERS_0_6>()) as u32,
        Anonymous: CF_OPERATION_PARAMETERS_0 {
            TransferData: CF_OPERATION_PARAMETERS_0_6 {
                Flags: CF_OPERATION_TRANSFER_DATA_FLAG_NONE,
                CompletionStatus: status,
                Buffer: buffer,
                Offset: offset,
                Length: length,
            },
        },
    };

    unsafe { CfExecute(&op_info, &mut op_params) }
}

/// Mark the file at `path` IN_SYNC so Explorer flips the overlay from
/// the downloading spinner to the synced check after a successful
/// hydration. Best-effort: opens a Cloud Files handle via
/// `CfOpenFileWithOplock`, calls `CfSetInSyncState`, logs and swallows
/// any error. Does NOT attempt to manage the full upload-side overlay
/// state machine — that lives in the sync runtime (runner.rs /
/// engine_bridge), out of this module's scope. See REPORT.
///
/// We use the Cloud Files–native `CfOpenFileWithOplock` rather than
/// `CreateFileW`: the latter is gated behind the `Win32_Security`
/// feature (for `SECURITY_ATTRIBUTES`) which this crate doesn't enable,
/// and the CF API is the documented way to obtain a handle for
/// `CfSetInSyncState`.
///
/// SAFETY: `path` is a NUL-terminated wide string; the opened CF handle
/// is closed via `CfCloseHandle` before return.
unsafe fn mark_in_sync(path: &[u16]) {
    use windows::core::PCWSTR;

    let handle = match unsafe { CfOpenFileWithOplock(PCWSTR(path.as_ptr()), CF_OPEN_FILE_FLAG_NONE) } {
        Ok(h) => h,
        Err(e) => {
            tracing::debug!(error = %e, "CfSetInSyncState: CfOpenFileWithOplock failed (overlay not updated)");
            return;
        }
    };

    if let Err(e) =
        unsafe { CfSetInSyncState(handle, CF_IN_SYNC_STATE_IN_SYNC, CF_SET_IN_SYNC_FLAG_NONE, None) }
    {
        tracing::debug!(error = %e, "CfSetInSyncState failed (overlay not updated)");
    }

    unsafe { CfCloseHandle(handle) };
}

/// Mark the file at `path` NOT in-sync so Explorer shows a non-synced (pending/
/// error) cloud overlay rather than the green synced check. This is the
/// Tier-2 visible-error signal (task 0783): when a hydration cannot be resolved
/// (the placeholder resize failed), the file must not silently masquerade as
/// synced. Paired with flipping the state_db row to `Error`, this keeps the
/// Explorer overlay honest. Best-effort: opens a CF handle, calls
/// `CfSetInSyncState(NOT_IN_SYNC)`, logs and swallows any error.
///
/// SAFETY: `path` is a NUL-terminated wide string; the opened CF handle is
/// closed via `CfCloseHandle` before return.
unsafe fn mark_not_in_sync(path: &[u16]) {
    use windows::core::PCWSTR;

    let handle = match unsafe { CfOpenFileWithOplock(PCWSTR(path.as_ptr()), CF_OPEN_FILE_FLAG_NONE) } {
        Ok(h) => h,
        Err(e) => {
            tracing::debug!(error = %e, "mark_not_in_sync: CfOpenFileWithOplock failed (overlay not updated)");
            return;
        }
    };

    if let Err(e) =
        unsafe { CfSetInSyncState(handle, CF_IN_SYNC_STATE_NOT_IN_SYNC, CF_SET_IN_SYNC_FLAG_NONE, None) }
    {
        tracing::debug!(error = %e, "CfSetInSyncState(NOT_IN_SYNC) failed (overlay not updated)");
    }

    unsafe { CfCloseHandle(handle) };
}

/// Pull the placeholder's normalized path out of `CF_CALLBACK_INFO` as a
/// NUL-terminated wide string suitable for `CfOpenFileWithOplock`. The
/// Cloud Files `NormalizedPath` is volume-relative (it lacks the drive),
/// so we prefix the volume DOS name (e.g. `C:`) to form an openable path.
/// Returns `None` if the relative path is missing/empty.
fn normalized_path(info: &CF_CALLBACK_INFO) -> Option<Vec<u16>> {
    let rel = wide_to_vec(info.NormalizedPath.0)?;
    let vol = wide_to_vec(info.VolumeDosName.0).unwrap_or_default();
    if rel.is_empty() {
        return None;
    }
    // Join `<VolumeDosName><NormalizedPath>`; NormalizedPath starts with a
    // backslash, so concatenation yields e.g. `C:\Users\...\file`.
    let mut full: Vec<u16> = Vec::with_capacity(vol.len() + rel.len() + 1);
    full.extend_from_slice(&vol);
    full.extend_from_slice(&rel);
    full.push(0);
    Some(full)
}

/// Copy a Windows `*const u16` NUL-terminated string into an owned Vec
/// WITHOUT the trailing NUL. Returns `None` for a null pointer.
fn wide_to_vec(ptr: *const u16) -> Option<Vec<u16>> {
    if ptr.is_null() {
        return None;
    }
    let mut out = Vec::new();
    // SAFETY: Cloud Files guarantees these PCWSTRs are NUL-terminated and
    // valid for the duration of the callback.
    unsafe {
        let mut p = ptr;
        while *p != 0 {
            out.push(*p);
            p = p.add(1);
        }
    }
    Some(out)
}


#[cfg(test)]
mod tests {
    use super::*;

    /// Sizes agree → transfer the bytes as-is (the steady-state path).
    #[test]
    fn size_match_transfers() {
        assert_eq!(
            decide_hydration_size_action(
                HydrationSizeEvidence::WholeFile {
                    server_size_bytes: Some(20),
                    plaintext_len: 20,
                },
                20,
            ),
            SizeDecision::Transfer
        );
        assert_eq!(
            decide_hydration_size_action(
                HydrationSizeEvidence::WholeFile {
                    server_size_bytes: Some(0),
                    plaintext_len: 0,
                },
                0,
            ),
            SizeDecision::Transfer
        );
    }

    /// The exact production bad-row shape: a single-chunk file whose recorded
    /// placeholder size is stale/wrong. The fresh server size is trusted ground
    /// truth for range hydration, so the resolution path must be chosen and the
    /// corrected size must be the server size, NOT the recorded 48.
    #[test]
    fn single_chunk_gcm_overhead_resolves_to_server_size() {
        let server_size = 20;
        let recorded = server_size + 28; // 48 — stale encrypted size on the placeholder
        assert_eq!(
            decide_hydration_size_action(
                HydrationSizeEvidence::Range {
                    server_size_bytes: server_size,
                    required_offset: 10,
                },
                recorded,
            ),
            SizeDecision::ResolveTo {
                corrected_size: server_size
            }
        );
    }

    /// Multi-chunk overhead (28 * chunk_count) resolves the same way: the
    /// corrected size is always the server's logical size regardless of the
    /// delta.
    #[test]
    fn multi_chunk_overhead_resolves_to_server_size() {
        let server_size = 8 * 1024 * 1024; // spans 2 chunks at 4 MiB
        let recorded = server_size + 28 * 2;
        assert_eq!(
            decide_hydration_size_action(
                HydrationSizeEvidence::Range {
                    server_size_bytes: server_size,
                    required_offset: 4 * 1024 * 1024,
                },
                recorded,
            ),
            SizeDecision::ResolveTo {
                corrected_size: server_size
            }
        );
    }

    /// A recorded size SMALLER than the server size (e.g. truncated/stale
    /// placeholder) still resolves to the server size.
    #[test]
    fn smaller_recorded_size_resolves_to_server_size() {
        assert_eq!(
            decide_hydration_size_action(
                HydrationSizeEvidence::Range {
                    server_size_bytes: 100,
                    required_offset: 40,
                },
                40,
            ),
            SizeDecision::ResolveTo { corrected_size: 100 }
        );
    }

    /// The DB-fallback path builder (task 0783 follow-up): a clean relative
    /// path joins onto the sync root with native separators, tolerating a
    /// leading/trailing slash exactly as populate_placeholders does.
    #[test]
    fn safe_join_builds_path_under_root() {
        let root = std::path::Path::new("C:\\Users\\me\\Beebeeb");
        // Leading slash + nested path → joined under the root.
        assert_eq!(
            safe_join_under_root(root, "/docs/report.txt"),
            Some(root.join("docs").join("report.txt"))
        );
        // Bare leaf, no slash.
        assert_eq!(
            safe_join_under_root(root, "bb-test-upload.txt"),
            Some(root.join("bb-test-upload.txt"))
        );
    }

    /// Path-safety guard: empty, root-only, and traversal rel paths are all
    /// rejected so a server-controlled path can never escape the sync root.
    #[test]
    fn safe_join_rejects_unsafe_paths() {
        let root = std::path::Path::new("C:\\Users\\me\\Beebeeb");
        assert_eq!(safe_join_under_root(root, ""), None);
        assert_eq!(safe_join_under_root(root, "/"), None);
        assert_eq!(safe_join_under_root(root, "///"), None);
        // `..` traversal anywhere in the path is refused.
        assert_eq!(safe_join_under_root(root, "../escape.txt"), None);
        assert_eq!(safe_join_under_root(root, "docs/../../escape.txt"), None);
        assert_eq!(safe_join_under_root(root, "docs/./report.txt"), None);
        // Empty interior segment (double slash) is refused.
        assert_eq!(safe_join_under_root(root, "docs//report.txt"), None);
    }
}
