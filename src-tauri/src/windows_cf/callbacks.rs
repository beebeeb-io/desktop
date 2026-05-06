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
//!   UUID), the connection key, and the transfer key. The latter two
//!   are needed by the (currently TODO) `CfExecute(TRANSFER_DATA)` call
//!   that streams plaintext bytes back to the Cloud Files runtime.
//! - `CF_CALLBACK_PARAMETERS` carries the `FetchData` view — the byte
//!   range Windows wants. We currently hydrate the whole file via
//!   `EngineBridge::hydrate_file` (which writes plaintext to a temp
//!   path) and rely on the post-MVP transfer-data wiring to splice
//!   the bytes back into the placeholder.
//!
//! ## Threading
//!
//! Cloud Files callbacks are delivered on a worker thread the
//! filter-driver picks. They are *not* on a tokio runtime, so we use
//! the parent runtime handle stashed by `runner::run` to bridge
//! sync→async. Blocking the worker for the full hydration is OK; the
//! Windows runtime will keep the placeholder marked "loading" without
//! UI thread impact.
//!
//! ## Open scaffold work
//!
//! - Wire `CfExecute(CF_OPERATION_TYPE_TRANSFER_DATA, ...)` so the
//!   plaintext we just produced actually reaches the placeholder. This
//!   needs the `connection_key` from the callback info and a buffer
//!   spec covering the requested range. Filed in
//!   `.claude/tasks/decisions/` once Task 6 lands.
//! - Map errors back into `CfExecute(...TRANSFER_DATA, FailureReason)`
//!   so Explorer shows a real error instead of a stuck spinner.

#![cfg(target_os = "windows")]

use windows::Win32::Storage::CloudFilters::*;

/// Windows-installed fetch callback. Registered via
/// `CfConnectSyncRoot` in a follow-up to Task 6 (the registration
/// glue lives in [`super::register_sync_root`] — connecting the
/// callback table is a separate `CfConnectSyncRoot` call we'll wire in
/// once the engine bridge stabilises its hydration error surface).
///
/// SAFETY: `callback_info` and `_callback_parameters` are valid for
/// the duration of this call per Cloud Files API contract. We read
/// them once into owned values before doing any blocking work, so
/// nothing is held across the await/block_on boundary.
pub extern "system" fn fetch_data_callback(
    callback_info: *const CF_CALLBACK_INFO,
    _callback_parameters: *const CF_CALLBACK_PARAMETERS,
) {
    // Defensive: a null pointer here would be an OS bug, but it costs
    // nothing to refuse to dereference.
    if callback_info.is_null() {
        tracing::warn!("fetch_data_callback fired with null CF_CALLBACK_INFO");
        return;
    }
    let info = unsafe { &*callback_info };

    // Decode FileIdentity → UTF-8 file_id. We stored it as raw UTF-8
    // bytes in `placeholders::create_placeholder`, so we don't need
    // the wide-string round trip the plan sketch did.
    let identity_bytes = unsafe {
        std::slice::from_raw_parts(
            info.FileIdentity as *const u8,
            info.FileIdentityLength as usize,
        )
    };
    let file_id = match std::str::from_utf8(identity_bytes) {
        Ok(s) => s.to_owned(),
        Err(e) => {
            tracing::warn!(error = %e, "FileIdentity is not valid UTF-8 — skipping hydrate");
            return;
        }
    };

    // Get the live engine bridge. Set by `runner::run` immediately
    // after building the bridge, before any placeholders exist, so a
    // missing bridge here means the daemon isn't running or a logout
    // raced with a callback. Either way: we can't hydrate.
    let bridge = match super::bridge() {
        Some(b) => b,
        None => {
            tracing::warn!(
                file_id = %file_id,
                "fetch callback fired but no EngineBridge registered"
            );
            return;
        }
    };

    // Hydration writes plaintext to a temp path. Post-MVP, we'll
    // wrap this with `CfExecute(TRANSFER_DATA, ...)` to push the
    // bytes back through Cloud Files instead — for now the temp
    // file is the audit trail when we need to debug a stuck file.
    let dest_path = std::env::temp_dir().join(&file_id);

    // Cloud Files calls us synchronously from a worker thread. We
    // don't have a tokio runtime in scope, so use the daemon's
    // already-running runtime via Handle::try_current. If we're not
    // attached to one (test harness, etc.), bail loudly.
    let handle = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(_) => {
            tracing::error!(
                "fetch_data_callback called outside a tokio runtime; \
                 hydration impossible"
            );
            return;
        }
    };

    let res = handle.block_on(async {
        bridge.hydrate_file(&file_id, &dest_path).await
    });

    if let Err(e) = res {
        tracing::warn!(file_id = %file_id, error = %e, "hydrate_file failed for Cloud Files callback");
        // TODO: surface via CfExecute(TRANSFER_DATA) with FailureReason
        // so Explorer doesn't spin forever.
        return;
    }
    tracing::info!(file_id = %file_id, dest = %dest_path.display(), "hydrated for Cloud Files");
    // TODO: CfExecute(CF_OPERATION_TYPE_TRANSFER_DATA, ...)
    //   — uses info.ConnectionKey + info.TransferKey to splice
    //   plaintext into the placeholder. Captured as a follow-up.
}
