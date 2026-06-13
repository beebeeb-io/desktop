//! Windows Cloud Files API integration — registers the Beebeeb sync
//! root with the OS and creates placeholders for cloud-only files. The
//! companion [`callbacks`] module installs the on-open hydration hook
//! Windows fires when a user touches a cloud-only entry in Explorer.
//!
//! Phase 2 Task 6 of `docs/superpowers/plans/2026-05-07-desktop-sync-client.md`.
//!
//! ## Why a separate module
//!
//! The macOS counterpart lives in a Swift `BeebeebFileProvider` extension
//! target that talks to the Rust daemon over a Unix socket
//! ([`crate::ipc_socket`]). Windows is the opposite: there's no
//! second process — the desktop binary itself registers as the cloud
//! sync provider, and Windows calls back into our running tokio
//! runtime through `extern "system"` callbacks. So everything in this
//! module is in-process Rust code, not an FFI shim.
//!
//! ## Lifecycle
//!
//! 1. On first launch, after the user picks a sync root,
//!    [`register_sync_root`] is called once. Idempotent — re-registering
//!    the same path is a no-op per Windows docs.
//! 2. Each time the engine bridge learns of a new remote file, it calls
//!    [`placeholders::create_placeholder`] to drop a 0-byte placeholder
//!    in Explorer. The placeholder carries the file_id as its identity
//!    blob so the fetch callback can recover it later.
//! 3. When the user opens the file, Windows fires
//!    [`callbacks::fetch_data_callback`], which invokes
//!    `EngineBridge::hydrate_file` to download + decrypt and writes the
//!    plaintext into the placeholder.
//!
//! ## Why no IPC client here
//!
//! The plan doc sketched an `IpcClient::connect` from inside the
//! callback. That made sense when the callback was assumed to live in
//! a separate process. In practice it doesn't — the desktop binary is
//! both the daemon and the provider — so we take a static handle to
//! the live `EngineBridge` (set by [`set_bridge`] from
//! `runner::run`) and call it directly. Removes a serialisation round
//! trip and removes the need for a Windows-named-pipe IPC variant.
//!
//! ## Build gating
//!
//! Every file in this module starts with `#![cfg(target_os = "windows")]`
//! and `windows_cf` itself is only registered in `lib.rs` under the
//! same gate. macOS and Linux builds skip everything here so the
//! `windows` crate never resolves on those platforms.

#![cfg(target_os = "windows")]

pub mod callbacks;
pub mod placeholders;

use std::sync::Arc;
use std::sync::OnceLock;

use windows::Win32::Storage::CloudFilters::*;
use windows::core::*;

use crate::engine_bridge::EngineBridge;

/// Identifier shown to Windows for our sync provider. Surfaced in the
/// Sync settings UI ("Files synced by Beebeeb"). Keep stable —
/// changing this between releases would orphan existing placeholders.
pub const PROVIDER_ID: &str = "Beebeeb";

/// Provider version reported alongside `PROVIDER_ID`. Bumping this is
/// how we tell Windows "schema for the FileIdentity blob may have
/// changed; recreate placeholders" — for now we keep it pinned to
/// "1.0.0" and rely on the `FileIdentity` being a UTF-8 file_id.
pub const PROVIDER_VERSION: &str = "1.0.0";

/// Live handle to the engine bridge, set once at runner startup and
/// read by the Cloud Files fetch callback. We can't capture the
/// bridge into the `extern "system"` function pointer Windows expects,
/// so a OnceLock is the simplest hop.
///
/// Set from `runner::run` immediately after building the bridge,
/// before placeholders are created (so a callback never fires before
/// the bridge exists).
static BRIDGE: OnceLock<Arc<EngineBridge>> = OnceLock::new();

/// Set once the first time we successfully `CfConnectSyncRoot`. Used to
/// make callback registration idempotent across engine respawns (re-login,
/// sync-root change) — Windows rejects a second connect on a root that's
/// already connected by this process, so we connect exactly once per
/// process lifetime.
static CONNECTED: OnceLock<()> = OnceLock::new();

/// Stash the engine bridge so [`callbacks::fetch_data_callback`] can
/// reach it when Windows demands hydration. Idempotent — once set the
/// pointer doesn't change for the lifetime of the process; on
/// re-login the runner respawns inside the same process and reuses the
/// existing OnceLock value (the master key + token are session-scoped
/// and live inside the bridge's `ApiClient`, not here).
pub fn set_bridge(bridge: Arc<EngineBridge>) {
    let _ = BRIDGE.set(bridge);
}

/// Internal accessor for the callback module.
pub(crate) fn bridge() -> Option<Arc<EngineBridge>> {
    BRIDGE.get().cloned()
}

/// Register `sync_root_path` as a Cloud Files sync root. After this
/// call returns Ok, Explorer shows the folder with our provider name
/// and the `placeholders::create_placeholder` calls start having
/// effect. Failures here are fatal for the Windows hydration story —
/// log and surface to the engine-status stream so the WebView can
/// degrade gracefully.
///
/// Idempotent at the OS layer: re-registering the same path with the
/// same provider id returns success.
pub fn register_sync_root(sync_root_path: &std::path::Path) -> anyhow::Result<()> {
    let path_wide: Vec<u16> = sync_root_path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let provider_version: Vec<u16> = PROVIDER_VERSION.encode_utf16().chain(std::iter::once(0)).collect();
    let display_name: Vec<u16> = PROVIDER_ID.encode_utf16().chain(std::iter::once(0)).collect();

    // SAFETY: `info` and `policies` are stack-allocated structs whose
    // lifetimes outlive the `CfRegisterSyncRoot` call. The wide-string
    // buffers are kept alive on the same stack frame for the same
    // reason. `CfRegisterSyncRoot` does not retain pointers past
    // return.
    unsafe {
        let info = CF_SYNC_REGISTRATION {
            StructSize: std::mem::size_of::<CF_SYNC_REGISTRATION>() as u32,
            ProviderName: PCWSTR(display_name.as_ptr()),
            ProviderVersion: PCWSTR(provider_version.as_ptr()),
            SyncRootIdentity: std::ptr::null(),
            SyncRootIdentityLength: 0,
            FileIdentity: std::ptr::null(),
            FileIdentityLength: 0,
            // GUID::zeroed() picks up the per-machine default. For
            // production we'll mint a stable provider GUID once and
            // hardcode it; the all-zero GUID still works for dev.
            ProviderId: GUID::zeroed(),
        };
        // Hydration policy `PARTIAL` = Windows hydrates ranges on demand
        // rather than dragging down the whole file (large vault
        // friendly). Population `PARTIAL` = directory listings expand
        // on demand instead of forcing us to enumerate the whole
        // server tree at registration.
        let policies = CF_SYNC_POLICIES {
            StructSize: std::mem::size_of::<CF_SYNC_POLICIES>() as u32,
            Hydration: CF_HYDRATION_POLICY {
                // In windows 0.58 the policy fields are strongly-typed
                // newtypes (CF_HYDRATION_POLICY_PRIMARY(u16) etc.), and the
                // `CF_*_PARTIAL` constants are already those newtypes — assign
                // them directly rather than unwrapping `.0` into a raw int.
                Primary: CF_HYDRATION_POLICY_PARTIAL,
                Modifier: CF_HYDRATION_POLICY_MODIFIER_NONE,
            },
            Population: CF_POPULATION_POLICY {
                Primary: CF_POPULATION_POLICY_PARTIAL,
                Modifier: CF_POPULATION_POLICY_MODIFIER_NONE,
            },
            InSync: CF_INSYNC_POLICY_NONE,
            HardLink: CF_HARDLINK_POLICY_NONE,
            PlaceholderManagement: CF_PLACEHOLDER_MANAGEMENT_POLICY_DEFAULT,
        };

        CfRegisterSyncRoot(PCWSTR(path_wide.as_ptr()), &info, &policies, CF_REGISTER_FLAG_NONE)
            .map_err(|e| anyhow::anyhow!("CfRegisterSyncRoot failed: {e}"))?;
    }

    tracing::info!(
        sync_root = %sync_root_path.display(),
        "Cloud Files sync root registered"
    );
    Ok(())
}

/// Connect the Cloud Files callback table for `sync_root_path` so Windows
/// fires [`callbacks::fetch_data_callback`] when a user opens a cloud-only
/// placeholder. Must be called after [`register_sync_root`]. Returns the
/// error from `CfConnectSyncRoot` on failure (e.g. already connected).
///
/// The callback table is a static array terminated by a
/// `CF_CALLBACK_TYPE_NONE` sentinel, per the Cloud Files contract. We only
/// register `FETCH_DATA` — the bytes-on-open hook — which is all on-demand
/// hydration needs.
fn connect_callbacks(sync_root_path: &std::path::Path) -> anyhow::Result<()> {
    let path_wide: Vec<u16> = sync_root_path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    // The table must outlive the call; it lives on this stack frame and
    // `CfConnectSyncRoot` copies what it needs before returning.
    let table = [
        CF_CALLBACK_REGISTRATION {
            Type: CF_CALLBACK_TYPE_FETCH_DATA,
            Callback: Some(callbacks::fetch_data_callback),
        },
        // Sentinel terminator — required by the API.
        CF_CALLBACK_REGISTRATION {
            Type: CF_CALLBACK_TYPE_NONE,
            Callback: None,
        },
    ];

    // SAFETY: `path_wide` and `table` are kept alive on this stack frame for
    // the duration of the call. The callback function pointer has static
    // lifetime. We ignore the returned connection key: hydration only needs
    // the connection key passed back inside `CF_CALLBACK_INFO`, and we keep
    // the root connected for the whole process lifetime.
    unsafe {
        CfConnectSyncRoot(
            PCWSTR(path_wide.as_ptr()),
            table.as_ptr(),
            None,
            CF_CONNECT_FLAG_REQUIRE_PROCESS_INFO,
        )
        .map_err(|e| anyhow::anyhow!("CfConnectSyncRoot failed: {e}"))?;
    }
    Ok(())
}

/// One-shot Windows Cloud Files activation, called from `runner::run` after
/// the engine bridge is built and before the sync loop starts ticking.
///
/// Idempotent end-to-end — safe to call on every (re)spawn:
///
/// 1. Stash the live [`EngineBridge`] so the in-process fetch callback
///    ([`callbacks::fetch_data_callback`]) can reach it on hydration.
/// 2. Register the sync root with Windows ([`register_sync_root`]).
///    Re-registering the same path is a no-op at the OS layer.
/// 3. Seed Explorer with cloud-only placeholders from the current DB file
///    list ([`populate_placeholders`]). Existing placeholders return
///    `ERROR_ALREADY_EXISTS`, which we swallow.
///
/// All failures are logged rather than propagated: a Cloud Files hiccup
/// must not take down the sync loop, which still works headlessly.
pub fn activate(_app: &tauri::AppHandle, bridge: Arc<EngineBridge>, sync_root: &std::path::Path) {
    // The callback can fire as soon as the sync root is registered, so the
    // bridge must be reachable first.
    set_bridge(bridge.clone());

    if let Err(e) = register_sync_root(sync_root) {
        tracing::error!(error = %e, "Cloud Files sync root registration failed; placeholders skipped");
        return;
    }

    // Connect the in-process fetch callback exactly once per process. Without
    // this, Windows shows placeholders but never asks us to hydrate them.
    if CONNECTED.get().is_none() {
        match connect_callbacks(sync_root) {
            Ok(()) => {
                let _ = CONNECTED.set(());
                tracing::info!("Cloud Files callbacks connected");
            }
            Err(e) => {
                tracing::error!(error = %e, "CfConnectSyncRoot failed; on-demand hydration disabled");
            }
        }
    }

    let created = populate_placeholders(&bridge, sync_root);
    tracing::info!(
        placeholders = created,
        sync_root = %sync_root.display(),
        "Cloud Files placeholders seeded"
    );
}

/// Walk the engine bridge's DB file list and drop a cloud-only placeholder
/// into Explorer for every file currently marked `CloudOnly`. Returns the
/// number of placeholders successfully created (already-existing ones are
/// counted as created since the end state is identical).
///
/// The placeholder is created under `sync_root joined with the file's
/// parent directory` — we mint the parent path from the stored relative
/// path and ensure it exists first, so nested cloud files land in the right
/// Explorer subfolder.
fn populate_placeholders(bridge: &EngineBridge, sync_root: &std::path::Path) -> usize {
    let entries = match bridge.db().list_by_status(crate::state_db::FileStatus::CloudOnly) {
        Ok(entries) => entries,
        Err(e) => {
            tracing::warn!(error = %e, "could not list cloud-only files for placeholder seeding");
            return 0;
        }
    };

    let mut created = 0usize;
    for entry in entries {
        // Stored paths are server-relative, '/'-separated, possibly with a
        // leading slash. Map onto the local sync root with native
        // separators.
        let rel = entry.path.trim_matches('/');
        if rel.is_empty() {
            continue;
        }
        let local_path = sync_root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        let (parent_dir, name) = match (local_path.parent(), local_path.file_name()) {
            (Some(parent), Some(name)) => (parent.to_path_buf(), name.to_string_lossy().into_owned()),
            _ => {
                tracing::warn!(path = %entry.path, "skipping placeholder for path with no file name");
                continue;
            }
        };

        if let Err(e) = crate::config::ensure_directory(&parent_dir) {
            tracing::warn!(error = %e, parent = %parent_dir.display(), "could not create placeholder parent dir");
            continue;
        }

        match placeholders::create_placeholder(&parent_dir, &entry.file_id, &name, entry.size_bytes, entry.modified_at)
        {
            Ok(()) => created += 1,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    file_id = %entry.file_id,
                    name = %name,
                    "placeholder creation failed"
                );
            }
        }
    }
    created
}
