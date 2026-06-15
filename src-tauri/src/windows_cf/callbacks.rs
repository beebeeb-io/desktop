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
//!   the whole file via `EngineBridge::hydrate_file` (decrypt to a temp
//!   path) and then splice the required range back with
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
//! `hydrate_file` writes *decrypted user plaintext* to a temp path. We
//! read it back, transfer it, then remove the temp file promptly so
//! decrypted bytes don't linger in `%TEMP%`. We never log plaintext,
//! keys, or tokens.

#![cfg(target_os = "windows")]

use windows::Win32::Foundation::{NTSTATUS, STATUS_SUCCESS, STATUS_UNSUCCESSFUL};
use windows::Win32::Storage::CloudFilters::*;
use windows::Win32::System::CorrelationVector::CORRELATION_VECTOR;

/// Chunk size for the `CfExecute(TRANSFER_DATA)` loop. Cloud Files
/// requires every non-final transfer's `Offset` and `Length` to be a
/// multiple of the volume sector/page size; 1 MiB is a safe multiple of
/// the 4 KiB page size and keeps a single buffer in memory at a time for
/// large files. The final chunk (the one that reaches the end of the
/// requested range) may be shorter / unaligned.
const TRANSFER_CHUNK: i64 = 1024 * 1024;

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
            unsafe { fail_transfer(connection_key, transfer_key, request_key, required_offset, required_length) };
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
            unsafe { fail_transfer(connection_key, transfer_key, request_key, required_offset, required_length) };
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
            unsafe { fail_transfer(connection_key, transfer_key, request_key, required_offset, required_length) };
            return;
        }
    };

    // Hydration writes the whole decrypted plaintext to a temp path. We
    // splice the required range back into the placeholder below, then
    // remove the temp file so decrypted user data doesn't linger.
    let dest_path = std::env::temp_dir().join(&file_id);

    let res = handle.block_on(async { bridge.hydrate_file(&file_id, &dest_path).await });
    if let Err(e) = res {
        // Do NOT log the error's inner content at info+ if it could carry
        // decrypted bytes; hydrate_file errors are status strings only.
        tracing::warn!(file_id = %file_id, error = %e, "hydrate_file failed for Cloud Files callback");
        secure_remove_temp(&dest_path);
        unsafe { fail_transfer(connection_key, transfer_key, request_key, required_offset, required_length) };
        return;
    }

    // Read the plaintext back into memory. (See REPORT: this round-trips
    // through %TEMP%; a future improvement is to stream from hydrate_file
    // in-memory and skip the temp file entirely.)
    let mut plaintext = match std::fs::read(&dest_path) {
        Ok(buf) => buf,
        Err(e) => {
            tracing::warn!(file_id = %file_id, error = %e, "could not read hydrated temp file");
            secure_remove_temp(&dest_path);
            unsafe { fail_transfer(connection_key, transfer_key, request_key, required_offset, required_length) };
            return;
        }
    };

    // Size sanity: the decrypted plaintext length must match the size
    // Windows recorded on the placeholder (info.FileSize). A mismatch
    // means our view of the file diverged from the OS's (re-upload with a
    // new layout, truncation, a stale placeholder…). Transferring anyway
    // risks an unaligned, non-EOF final chunk → STATUS_INVALID_PARAMETER
    // on the last CfExecute. Fail cleanly on the required range instead.
    if plaintext.len() as i64 != file_size {
        tracing::warn!(
            file_id = %file_id,
            plaintext_len = plaintext.len(),
            file_size,
            "hydrated size != placeholder size; failing transfer to avoid a misaligned chunk"
        );
        zero_bytes(&mut plaintext);
        secure_remove_temp(&dest_path);
        unsafe { fail_transfer(connection_key, transfer_key, request_key, required_offset, required_length) };
        return;
    }

    // Splice the required range back into the placeholder.
    let transfer_ok = unsafe {
        transfer_range(
            connection_key,
            transfer_key,
            request_key,
            &plaintext,
            required_offset,
            required_length,
        )
    };

    // Zero the in-memory plaintext copy before dropping (it's decrypted
    // user data) and remove the temp file.
    zero_bytes(&mut plaintext);
    secure_remove_temp(&dest_path);

    match transfer_ok {
        Ok(()) => {
            tracing::info!(file_id = %file_id, "hydrated and transferred Cloud Files range");
            // Best-effort: mark the placeholder IN_SYNC so Explorer flips
            // the overlay from "downloading" to the synced check — but
            // ONLY when this fetch delivered the whole file. With
            // CF_HYDRATION_POLICY_PARTIAL a request may cover just a
            // range; marking the file fully in-sync after a partial
            // transfer would be a lie and could suppress later fetches.
            // (hydrate_file always decrypts the whole file to disk, so a
            // [0, file_size) required range means the file is now fully
            // present.) A failure here doesn't undo the transfer, so we
            // only log.
            let whole_file = required_offset == 0 && required_length >= file_size;
            if whole_file {
                if let Some(path) = normalized_path(info) {
                    unsafe { mark_in_sync(&path) };
                }
            }
        }
        Err(e) => {
            tracing::warn!(file_id = %file_id, error = %e, "CfExecute(TRANSFER_DATA) failed");
            // The transfer call itself failed; tell Explorer to stop
            // spinning. If this also fails there's nothing more we can do.
            unsafe { fail_transfer(connection_key, transfer_key, request_key, required_offset, required_length) };
        }
    }
}

/// Stream `[offset, offset+length)` of `plaintext` back into the
/// placeholder via `CfExecute(CF_OPERATION_TYPE_TRANSFER_DATA)`,
/// chunked so each non-final transfer is page-aligned. Satisfies the
/// caller's *required* range. If the requested range runs past the end
/// of the decrypted data (shouldn't happen — the placeholder size and
/// plaintext come from the same source) we transfer what we have.
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
    required_offset: i64,
    required_length: i64,
) -> windows::core::Result<()> {
    let total = plaintext.len() as i64;
    // Clamp the requested range to what we actually have.
    let start = required_offset.clamp(0, total);
    let end = (required_offset + required_length).clamp(0, total);
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
        let buf_ptr = unsafe { plaintext.as_ptr().add(off as usize) } as *const core::ffi::c_void;
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

/// Best-effort overwrite of a byte buffer with zeros. `plaintext` is
/// decrypted user data; we scrub it before the Vec is freed. Uses a
/// volatile write so the compiler can't elide it.
fn zero_bytes(buf: &mut [u8]) {
    for b in buf.iter_mut() {
        // SAFETY: `b` is a valid, aligned, writable u8.
        unsafe { std::ptr::write_volatile(b, 0) };
    }
}

/// Remove the decrypted-plaintext temp file. Best-effort: a leftover
/// decrypted file in %TEMP% is a privacy risk, so we always attempt
/// removal and only debug-log a failure (e.g. another handle still open).
fn secure_remove_temp(path: &std::path::Path) {
    if let Err(e) = std::fs::remove_file(path) {
        // Don't surface the path content beyond debug; it's a temp file
        // name (the file_id UUID), not sensitive on its own, but keep
        // noise low.
        tracing::debug!(error = %e, "could not remove hydrated temp file");
    }
}
