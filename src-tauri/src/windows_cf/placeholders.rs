//! Placeholder creation — drops 0-byte cloud-only stubs into the sync
//! root so files appear in Explorer before any bytes have been
//! downloaded.
//!
//! Spec: `docs/superpowers/plans/2026-05-07-desktop-sync-client.md`
//! Phase 2 Task 6.
//!
//! ## When this is called
//!
//! `engine_bridge::sync_tick` walks the server's file list each tick.
//! For each remote file the local DB has marked `CloudOnly`, the bridge
//! calls [`create_placeholder`] with the parent directory and the
//! file's plaintext name. Subsequent ticks are no-ops because Windows
//! returns ERROR_ALREADY_EXISTS, which we map to `Ok(())`.
//!
//! ## File identity
//!
//! The `FileIdentity` blob is the file's UUID stored as UTF-8 bytes.
//! On the way back in (when Windows fires the fetch callback), we
//! decode the same bytes back into a UUID string and pass it to
//! `EngineBridge::hydrate_file`.
//!
//! ## What gets stored on disk
//!
//! - A reparse point at `<parent>/<name>` — looks like a normal file in
//!   Explorer, has the right size + modified-at metadata, but contains
//!   no real bytes until the first hydration.
//! - The reparse point carries our `FileIdentity` blob, which Windows
//!   passes back to our fetch callback verbatim.

#![cfg(target_os = "windows")]

use windows::Win32::Storage::CloudFilters::*;
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT, FILE_BASIC_INFO,
    GetFileAttributesW, INVALID_FILE_ATTRIBUTES,
};
use windows::core::PCWSTR;

/// Create a single cloud-only placeholder under `parent_dir`. The file
/// shows up in Explorer immediately with the cloud-icon overlay; bytes
/// are fetched lazily by [`super::callbacks::fetch_data_callback`].
///
/// # Parameters
///
/// - `parent_dir`: directory inside the registered sync root. Must
///   already exist.
/// - `file_id`: the server's UUID for this file. Round-trips through
///   the FileIdentity blob and resurfaces in the fetch callback.
/// - `name`: plaintext filename Explorer should show. The encrypted
///   filename never reaches Windows.
/// - `size_bytes`: plaintext size — what the user expects to see in
///   the size column.
/// - `modified_at`: unix timestamp (seconds; millis tolerated — see
///   [`unix_to_filetime`]) of the file's last modification. Converted to
///   a Windows `FILETIME` tick count and written to the placeholder's
///   `LastWriteTime` / `ChangeTime` / `CreationTime` so Explorer shows a
///   real modified date instead of a blank/epoch one.
/// - `is_dir`: when `true`, mint a **directory** placeholder
///   (`FILE_ATTRIBUTE_DIRECTORY`) that is a real, openable folder in
///   Explorer — children land inside it on disk. When `false`, mint a file
///   placeholder (`FILE_ATTRIBUTE_NORMAL`) whose bytes are hydrated on open.
pub fn create_placeholder(
    parent_dir: &std::path::Path,
    file_id: &str,
    name: &str,
    size_bytes: i64,
    modified_at: i64,
    is_dir: bool,
) -> anyhow::Result<()> {
    let parent_wide: Vec<u16> = parent_dir
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let name_wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    // FileIdentity is opaque to Windows — we use raw UTF-8 bytes of
    // the file_id so the fetch callback can decode it directly.
    let identity: Vec<u8> = file_id.as_bytes().to_vec();

    // Convert the file's unix modified timestamp into the Windows
    // FILETIME tick count (100-ns intervals since 1601-01-01) that
    // FILE_BASIC_INFO stores as an i64.
    let modified_ticks = unix_to_filetime(modified_at);

    // Directory vs file placeholder.
    //  - DIRECTORY: a real, openable folder. Its children are the on-disk
    //    placeholders we seed inside it; a directory has no data stream so
    //    `FileSize` is 0.
    //  - FILE (NORMAL): a hydrate-on-open stub carrying the plaintext size.
    let file_attributes = if is_dir {
        FILE_ATTRIBUTE_DIRECTORY.0
    } else {
        FILE_ATTRIBUTE_NORMAL.0
    };
    let file_size = if is_dir { 0 } else { size_bytes };

    // Placeholder-create flags:
    //  - MARK_IN_SYNC: this placeholder reflects the latest server state.
    //    Without it Windows treats a new placeholder as locally-modified
    //    relative to the cloud and starts a spurious upload conflict.
    //  - DISABLE_ON_DEMAND_POPULATION (directories only): the per-directory
    //    analog of the root's CF_REGISTER_FLAG_DISABLE_ON_DEMAND_POPULATION_ON_ROOT
    //    + CF_POPULATION_POLICY_ALWAYS_FULL. It tells Windows this directory is
    //    ALREADY fully enumerated by its on-disk children, so Explorer must NOT
    //    block on a FETCH_PLACEHOLDERS callback (which this in-process provider
    //    never registers — see mod.rs). This is what keeps nested folders from
    //    reintroducing the enumeration hang while staying in the eager model.
    let mut create_flags = CF_PLACEHOLDER_CREATE_FLAG_MARK_IN_SYNC;
    if is_dir {
        create_flags |= CF_PLACEHOLDER_CREATE_FLAG_DISABLE_ON_DEMAND_POPULATION;
    }

    // SAFETY: `entry`, `parent_wide`, `name_wide`, `identity` all live
    // on this stack frame for the duration of the FFI call. The
    // `CfCreatePlaceholders` API copies what it needs before returning
    // and does not retain raw pointers afterwards.
    unsafe {
        let mut entry = CF_PLACEHOLDER_CREATE_INFO {
            RelativeFileName: PCWSTR(name_wide.as_ptr()),
            FsMetadata: CF_FS_METADATA {
                BasicInfo: FILE_BASIC_INFO {
                    // FILE_ATTRIBUTE_DIRECTORY (0x10) for folders, else
                    // FILE_ATTRIBUTE_NORMAL (0x80). A non-zero attribute is
                    // required or Cloud Files refuses the placeholder. (The
                    // previous file path hardcoded 0x20 / ARCHIVE, which also
                    // worked; NORMAL is the semantically-correct file attr.) We
                    // don't carry over hidden/system bits; we have no use for them.
                    FileAttributes: file_attributes,
                    // Real timestamps so Explorer's Date Modified column
                    // is populated. We only have a single modified time
                    // from the server, so we mirror it across
                    // creation/write/change. LastAccessTime is left at 0
                    // (Windows treats 0 as "don't change / unknown").
                    CreationTime: modified_ticks,
                    LastWriteTime: modified_ticks,
                    ChangeTime: modified_ticks,
                    ..Default::default()
                },
                FileSize: file_size,
            },
            FileIdentity: identity.as_ptr() as *const _,
            FileIdentityLength: identity.len() as u32,
            Flags: create_flags,
            // Result is filled in by Windows on return. Initialised to
            // S_OK so a no-op (already exists) reads sensibly.
            Result: windows::core::HRESULT(0),
            CreateUsn: 0,
        };

        // The 1-element slice + count == 1 is the canonical "create one
        // placeholder" call shape. Batch creation is possible by
        // passing a longer slice, but we pace creation per-file as the
        // engine bridge discovers them, so single-entry is fine.
        // windows 0.58 takes the placeholder array as a `&mut [..]` slice
        // (the length is derived from it) plus an optional
        // `entriesprocessed` out-pointer — four args total, not five.
        let entries = std::slice::from_mut(&mut entry);
        if let Err(e) = CfCreatePlaceholders(PCWSTR(parent_wide.as_ptr()), entries, CF_CREATE_FLAG_NONE, None) {
            // A placeholder that already exists is the steady state on every
            // tick after the first — `HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS)`
            // (0x800700B7). Treat it as success: the desired end state holds.
            const ALREADY_EXISTS: windows::core::HRESULT = windows::core::HRESULT(0x800700B7u32 as i32);
            if e.code() == ALREADY_EXISTS {
                return Ok(());
            }
            return Err(anyhow::anyhow!("CfCreatePlaceholders: {e}"));
        }
    }

    // Zero-knowledge: log the file_id + kind only. The decrypted plaintext
    // filename (and the parent path, which can carry decrypted folder names)
    // must never reach the logs.
    tracing::debug!(file_id = %file_id, is_dir, "cloud placeholder created");
    Ok(())
}

/// Convert a freshly-uploaded **plain local file** at `path` into an
/// in-sync Cloud Files placeholder, so Explorer shows it with the synced
/// (green check) overlay rather than as an ordinary always-local file.
///
/// Called by the upload watcher path (task 0780) right after a NEW
/// user-created file under the sync root has been encrypted + uploaded and
/// the server has assigned it a `file_id`. The point is two-fold:
///
/// 1. **Identity** — stamp the same `FileIdentity` blob (UTF-8 `file_id`)
///    the placeholder seeder uses, so a later `free_up_space` /
///    on-demand-hydrate round-trips through the same fetch callback. Without
///    this the file would be a local-only file with no CF identity and could
///    never be dehydrated or re-hydrated.
///
/// 2. **In-sync state** — `CF_CONVERT_FLAG_MARK_IN_SYNC` marks the new
///    placeholder as reflecting the latest server state, so Windows does NOT
///    immediately treat it as locally-modified-relative-to-cloud (which would
///    kick off a spurious re-upload — the very feedback loop we are avoiding).
///    We do NOT pass `CF_CONVERT_FLAG_DEHYDRATE`: the user just created this
///    file locally, so the happy path keeps the bytes on disk and merely
///    re-labels them as an in-sync placeholder. Dehydration (reclaiming the
///    bytes) is the separate `free_up_space` / smart-cache path.
///
/// Best-effort and idempotent: a file that is ALREADY a placeholder returns
/// `ERROR_NOT_A_CLOUD_FILE`-adjacent failures from `CfConvertToPlaceholder`,
/// which we map to `Ok(())` — converting a placeholder again is a no-op.
///
/// SAFETY: opens a Cloud Files handle via `CfOpenFileWithOplock` (exclusive,
/// write access — conversion mutates the file's reparse data) and always
/// closes it via `CfCloseHandle` before returning.
pub fn convert_to_in_sync_placeholder(path: &std::path::Path, file_id: &str) -> anyhow::Result<()> {
    let path_wide: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    // FileIdentity is opaque to Windows — raw UTF-8 bytes of the file_id, the
    // exact same encoding `create_placeholder` uses, so the fetch callback can
    // decode it identically on a later hydration.
    let identity: Vec<u8> = file_id.as_bytes().to_vec();

    // SAFETY: `path_wide` / `identity` live on this stack frame for the whole
    // call. The opened handle is closed before return on every path.
    unsafe {
        // Exclusive + write access: converting a file to a placeholder
        // rewrites its reparse data, so we need a writable, exclusive handle.
        let handle = CfOpenFileWithOplock(
            PCWSTR(path_wide.as_ptr()),
            CF_OPEN_FILE_FLAG_EXCLUSIVE | CF_OPEN_FILE_FLAG_WRITE_ACCESS,
        )
        .map_err(|e| anyhow::anyhow!("CfOpenFileWithOplock (convert): {e}"))?;

        let result = CfConvertToPlaceholder(
            handle,
            Some(identity.as_ptr() as *const core::ffi::c_void),
            identity.len() as u32,
            // MARK_IN_SYNC, no DEHYDRATE — keep the user's bytes on disk but
            // label the file as a synced placeholder. ENABLE_ON_DEMAND_POPULATION
            // is for directories; this is a file, so it's omitted.
            CF_CONVERT_FLAG_MARK_IN_SYNC,
            None,
            None,
        );

        CfCloseHandle(handle);

        if let Err(e) = result {
            // 0x8007017C = HRESULT_FROM_WIN32(ERROR_CLOUD_FILE_ALREADY_CONNECTED)
            // and 0x80070178 = ERROR_NOT_A_CLOUD_FILE are the "already a
            // placeholder / nothing to convert" steady states — treat as the
            // desired end state. Anything else is a real failure.
            const ALREADY_A_PLACEHOLDER: windows::core::HRESULT =
                windows::core::HRESULT(0x8007017Cu32 as i32);
            if e.code() == ALREADY_A_PLACEHOLDER {
                return Ok(());
            }
            return Err(anyhow::anyhow!("CfConvertToPlaceholder: {e}"));
        }
    }

    // Zero-knowledge: log the file_id only, never the decrypted plaintext path.
    tracing::debug!(file_id = %file_id, "converted local file to in-sync placeholder");
    Ok(())
}

/// Convert a brand-new plain local file at `path` into a Cloud Files
/// placeholder WITHOUT marking it in-sync, called the moment we decide to upload
/// it (task 0780) and BEFORE the upload completes.
///
/// Why convert pre-upload at all: a plain file the user just dropped into a
/// CF-connected root is NOT a placeholder, so it has no Cloud Files identity and
/// the CF filter has no reason to leave it alone while we read its bytes for the
/// encrypted upload. Converting it to a placeholder hands it to the filter under
/// our control, so it is not reclaimed/dehydrated mid-upload.
///
/// Why NOT `MARK_IN_SYNC` here (unlike [`convert_to_in_sync_placeholder`]): the
/// file is NOT yet on the server — the upload is only just being queued. Marking
/// it in-sync now would be a lie (and could let "Free up space" dehydrate bytes
/// that were never uploaded). We pass `CF_CONVERT_FLAG_NONE`: the file becomes a
/// placeholder that keeps its bytes on disk and is understood to be locally
/// ahead of the cloud. Once the real `file_id` lands,
/// [`super::super::engine_bridge::EngineBridge::finalize_local_upload_placeholder`]
/// → [`convert_to_in_sync_placeholder`] re-stamps it in-sync with the server id.
///
/// We use a throwaway identity here (the local op uuid would do, but we have no
/// server id yet) — pass `local_id`, which `finalize` overwrites with the real
/// server `file_id` on the in-sync re-stamp. The identity is opaque to Windows.
///
/// Best-effort + idempotent: a file that is already a placeholder returns the
/// "already a cloud file" HRESULT, which we map to `Ok(())`. A failure here
/// leaves a working plain file that still uploads — the convert is an
/// anti-reclaim guard, not a correctness requirement — so the caller only logs.
///
/// SAFETY: opens an exclusive, write CF handle via `CfOpenFileWithOplock`
/// (conversion rewrites reparse data) and always closes it before returning.
pub fn convert_to_unsynced_placeholder(path: &std::path::Path, local_id: &str) -> anyhow::Result<()> {
    let path_wide: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let identity: Vec<u8> = local_id.as_bytes().to_vec();

    // SAFETY: `path_wide` / `identity` live for the whole call; the handle is
    // closed on every exit path.
    unsafe {
        let handle = CfOpenFileWithOplock(
            PCWSTR(path_wide.as_ptr()),
            CF_OPEN_FILE_FLAG_EXCLUSIVE | CF_OPEN_FILE_FLAG_WRITE_ACCESS,
        )
        .map_err(|e| anyhow::anyhow!("CfOpenFileWithOplock (convert-unsynced): {e}"))?;

        // NONE: become a placeholder, keep bytes on disk, do NOT claim in-sync.
        let result = CfConvertToPlaceholder(
            handle,
            Some(identity.as_ptr() as *const core::ffi::c_void),
            identity.len() as u32,
            CF_CONVERT_FLAG_NONE,
            None,
            None,
        );

        CfCloseHandle(handle);

        if let Err(e) = result {
            // Already a placeholder / already a cloud file → desired end state.
            const ALREADY_A_PLACEHOLDER: windows::core::HRESULT =
                windows::core::HRESULT(0x8007017Cu32 as i32);
            const NOT_A_CLOUD_FILE: windows::core::HRESULT = windows::core::HRESULT(0x80070178u32 as i32);
            if e.code() == ALREADY_A_PLACEHOLDER || e.code() == NOT_A_CLOUD_FILE {
                return Ok(());
            }
            return Err(anyhow::anyhow!("CfConvertToPlaceholder (unsynced): {e}"));
        }
    }

    // Zero-knowledge: log the local id only, never the decrypted path/filename.
    tracing::debug!(local_id = %local_id, "converted new local file to unsynced placeholder (pre-upload)");
    Ok(())
}

/// Dehydrate a Cloud Files placeholder at `path`, freeing the on-disk bytes
/// that live INSIDE the placeholder while keeping it visible in Explorer as a
/// cloud-only file. Returns the number of bytes actually reclaimed from disk.
///
/// This is the Windows-correct replacement for the Unix `remove_file` in
/// `free_up_space` (task 0781): on Windows CF the hydrated bytes are stored in
/// the placeholder's primary data stream, NOT in a separate cache copy — so a
/// `remove_file` would delete the user's file outright (and the old code's
/// `remove_file` on a still-present placeholder simply failed/no-op'd, leaving
/// the file fully hydrated while the DB lied that it was cloud-only).
///
/// We dehydrate the WHOLE file (`startingoffset = 0`, `length = -1` meaning
/// "to EOF" per the Cloud Files contract). `bytes_freed` is computed from the
/// real on-disk allocation BEFORE dehydration (the logical file size, which
/// for a fully-hydrated placeholder equals the bytes on disk) and only
/// returned on a SUCCESSFUL dehydrate — a failed dehydrate reclaims nothing,
/// so the caller must not flip the DB row to cloud_only for it.
///
/// `Ok(0)` (no error) is returned when the file is already dehydrated /
/// partial — there is nothing to reclaim and the end state already holds.
///
/// SAFETY: opens a CF handle via `CfOpenFileWithOplock` and always closes it
/// via `CfCloseHandle` before returning.
pub fn dehydrate_placeholder(path: &std::path::Path) -> anyhow::Result<u64> {
    // Logical size = bytes currently materialised on disk for a fully-hydrated
    // placeholder. Read it before we dehydrate so we can report what we freed.
    let size_before = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

    let path_wide: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    // SAFETY: `path_wide` is alive for the whole call; the handle is closed on
    // every exit path.
    unsafe {
        let handle = CfOpenFileWithOplock(
            PCWSTR(path_wide.as_ptr()),
            CF_OPEN_FILE_FLAG_EXCLUSIVE | CF_OPEN_FILE_FLAG_WRITE_ACCESS,
        )
        .map_err(|e| anyhow::anyhow!("CfOpenFileWithOplock (dehydrate): {e}"))?;

        // length = -1 → dehydrate from `startingoffset` to EOF. NONE flags =
        // synchronous foreground dehydration (we want the bytes gone now so
        // the freed figure is accurate when we return).
        let result = CfDehydratePlaceholder(handle, 0, -1, CF_DEHYDRATE_FLAG_NONE, None);

        CfCloseHandle(handle);

        if let Err(e) = result {
            // 0x80070179 = HRESULT_FROM_WIN32(ERROR_CLOUD_FILE_NOT_IN_SYNC):
            // the placeholder isn't in-sync (e.g. locally-modified) so it can't
            // be safely dehydrated — skip it, reclaim nothing, don't error the
            // whole sweep. Anything else is a real failure for THIS file.
            const NOT_IN_SYNC: windows::core::HRESULT = windows::core::HRESULT(0x80070179u32 as i32);
            // 0x80070178 = ERROR_NOT_A_CLOUD_FILE — already a plain file or
            // already dehydrated; nothing to reclaim.
            const NOT_A_CLOUD_FILE: windows::core::HRESULT = windows::core::HRESULT(0x80070178u32 as i32);
            if e.code() == NOT_IN_SYNC || e.code() == NOT_A_CLOUD_FILE {
                return Ok(0);
            }
            return Err(anyhow::anyhow!("CfDehydratePlaceholder: {e}"));
        }
    }

    Ok(size_before)
}

/// True if `path` is a reparse point — i.e. a Cloud Files placeholder this
/// provider created. We never create any other kind of reparse point under
/// the sync root, so a reparse-point attribute is a reliable, cheap signal
/// that the engine (not the user) owns this on-disk entry.
///
/// Used as a SECONDARY feedback-loop guard by the upload watcher: the PRIMARY
/// guard is the state DB (a path already tracked as a server file is never
/// re-uploaded), but a placeholder that the seeder just minted may race ahead
/// of its DB row in some orderings, so the attribute check catches it too.
///
/// Returns `false` (treat as a normal file → eligible for upload) on any
/// attribute-query error: failing open here only risks a redundant upload
/// attempt that the DB guard will already have suppressed, never data loss.
pub fn is_cloud_placeholder(path: &std::path::Path) -> bool {
    let path_wide: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: `path_wide` is a valid NUL-terminated wide string alive for the
    // duration of the call. `GetFileAttributesW` does not retain the pointer.
    let attrs = unsafe { GetFileAttributesW(PCWSTR(path_wide.as_ptr())) };
    if attrs == INVALID_FILE_ATTRIBUTES {
        return false;
    }
    (attrs & FILE_ATTRIBUTE_REPARSE_POINT.0) != 0
}

/// Resize the Cloud Files placeholder at `path` so its `FileSize` matches
/// the AES-256-GCM-authenticated decrypted plaintext length, and re-stamp
/// it in-sync. This is **Tier-1 of the hydration resolve-or-error path**
/// (task 0783): when `hydrate_file` decrypts successfully but the decrypted
/// length disagrees with the placeholder's recorded size (the server stored
/// the *encrypted* size on some upload paths — see the 0783 decision file),
/// the decrypted length is ground truth, so we correct the placeholder to it.
///
/// ## Why this runs AFTER the fetch fails, not during it
///
/// `CfUpdatePlaceholder` cannot legally resize a placeholder that has an
/// in-flight hydration: the file is in-use by the active `FETCH_DATA`
/// transfer, and the call returns `ERROR_CLOUD_FILE_IN_USE`. The documented
/// way to abort a hydration mid-callback is `CfExecute(TRANSFER_DATA)` with a
/// non-success status (our `fail_transfer`). So the callback fails THIS fetch
/// cleanly first, THEN calls this to correct the size out-of-band. The file
/// stays openable: the very next open fires a fresh `FETCH_DATA` against the
/// now-aligned size and hydrates successfully. The end state — an openable
/// file whose size equals the decrypted length — is reached, just one open
/// later, which is the only point at which the resize is API-legal.
///
/// ## What we change
///
/// We pass a `CF_FS_METADATA` carrying ONLY the corrected `FileSize` plus
/// `FILE_ATTRIBUTE_NORMAL`; the `FILE_BASIC_INFO` timestamp fields are left at
/// 0, which Windows treats as "do not change" so the file keeps its real
/// Date Modified. `CF_UPDATE_FLAG_MARK_IN_SYNC` re-anchors the placeholder to
/// the corrected metadata so Windows does not treat the resize as a local
/// edit needing re-upload. We pass `None` for the file identity (keep the
/// existing UTF-8 `file_id` blob) and no dehydrate range (the whole file is
/// already cloud-only after the failed fetch).
///
/// Best-effort: opens an exclusive, write CF handle via `CfOpenFileWithOplock`
/// and always closes it before returning. A failure here leaves the
/// placeholder at the stale size (the file is no worse off than before this
/// path existed — Tier-2 still surfaced a real error on the failed fetch), so
/// the caller only logs.
///
/// SAFETY: `path_wide` lives for the whole call; the handle is closed on every
/// exit path.
pub fn resize_placeholder(path: &std::path::Path, true_size_bytes: i64) -> anyhow::Result<()> {
    let path_wide: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    // SAFETY: `path_wide` is alive for the whole call; `metadata` lives on this
    // stack frame and `CfUpdatePlaceholder` copies what it needs before return.
    unsafe {
        let handle = CfOpenFileWithOplock(
            PCWSTR(path_wide.as_ptr()),
            CF_OPEN_FILE_FLAG_EXCLUSIVE | CF_OPEN_FILE_FLAG_WRITE_ACCESS,
        )
        .map_err(|e| anyhow::anyhow!("CfOpenFileWithOplock (resize): {e}"))?;

        let metadata = CF_FS_METADATA {
            BasicInfo: FILE_BASIC_INFO {
                // 0 timestamps = "leave unchanged" so the placeholder keeps its
                // real Date Modified; only the size is being corrected here.
                CreationTime: 0,
                LastAccessTime: 0,
                LastWriteTime: 0,
                ChangeTime: 0,
                // A non-zero attribute is required; NORMAL matches what
                // create_placeholder mints for a file.
                FileAttributes: FILE_ATTRIBUTE_NORMAL.0,
            },
            // The correction: the placeholder's logical size becomes the
            // decrypted plaintext length (clamped non-negative).
            FileSize: true_size_bytes.max(0),
        };

        let result = CfUpdatePlaceholder(
            handle,
            Some(&metadata as *const CF_FS_METADATA),
            // Keep the existing FileIdentity blob (the UTF-8 file_id) — passing
            // None means "do not touch the identity".
            None,
            0,
            // No dehydrate range: the file is fully cloud-only after the failed
            // fetch, so there is no hydrated range to invalidate.
            None,
            // MARK_IN_SYNC: the corrected metadata IS the latest truth, so don't
            // let Windows read the resize as a local modification.
            CF_UPDATE_FLAG_MARK_IN_SYNC,
            None,
            None,
        );

        CfCloseHandle(handle);

        if let Err(e) = result {
            // ERROR_NOT_A_CLOUD_FILE (0x80070178) — the path isn't a placeholder
            // (e.g. it was hydrated to a plain file or removed). Nothing to
            // resize; treat as the end state rather than erroring the sweep.
            const NOT_A_CLOUD_FILE: windows::core::HRESULT = windows::core::HRESULT(0x80070178u32 as i32);
            if e.code() == NOT_A_CLOUD_FILE {
                return Ok(());
            }
            return Err(anyhow::anyhow!("CfUpdatePlaceholder (resize): {e}"));
        }
    }

    // Zero-knowledge: log the corrected size only, never the decrypted path.
    tracing::debug!(true_size_bytes, "resized cloud placeholder to decrypted length");
    Ok(())
}

/// Number of seconds between the Windows FILETIME epoch (1601-01-01) and
/// the Unix epoch (1970-01-01).
const FILETIME_UNIX_EPOCH_OFFSET_SECS: i64 = 11_644_473_600;

/// Convert a Unix timestamp into a Windows FILETIME tick count: the
/// number of 100-nanosecond intervals since 1601-01-01, as stored in
/// `FILE_BASIC_INFO`'s i64 timestamp fields.
///
/// The desktop DB stores `modified_at` in **seconds** (see
/// `engine_bridge::now_secs`). As a defensive measure we also accept
/// millisecond values: anything implausibly large to be seconds (beyond
/// ~year 5000) is treated as milliseconds. A non-positive or zero input
/// yields 0, which Windows renders as "no date".
fn unix_to_filetime(modified_at: i64) -> i64 {
    if modified_at <= 0 {
        return 0;
    }

    // Year ~5000 in unix seconds. A value larger than this is almost
    // certainly milliseconds, not seconds.
    const SECONDS_SANITY_CEILING: i64 = 95_617_584_000;
    let unix_secs = if modified_at > SECONDS_SANITY_CEILING {
        modified_at / 1000
    } else {
        modified_at
    };

    // Shift to the 1601 epoch, then scale seconds → 100-ns ticks.
    // saturating_* keeps a pathological value from overflowing into a
    // negative (which Explorer would render as a bogus 1601 date).
    unix_secs
        .saturating_add(FILETIME_UNIX_EPOCH_OFFSET_SECS)
        .saturating_mul(10_000_000)
}
