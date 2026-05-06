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

use windows::core::*;
use windows::Win32::Storage::CloudFilters::*;

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
    let provider_version: Vec<u16> = PROVIDER_VERSION
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let display_name: Vec<u16> = PROVIDER_ID
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

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
                Primary: CF_HYDRATION_POLICY_PARTIAL.0 as i16,
                Modifier: 0,
            },
            Population: CF_POPULATION_POLICY {
                Primary: CF_POPULATION_POLICY_PARTIAL.0 as i16,
                Modifier: 0,
            },
            InSync: CF_INSYNC_POLICY_NONE,
            HardLink: CF_HARDLINK_POLICY_NONE,
            PlaceholderManagement: CF_PLACEHOLDER_MANAGEMENT_POLICY_DEFAULT,
        };

        CfRegisterSyncRoot(
            PCWSTR(path_wide.as_ptr()),
            &info,
            &policies,
            CF_REGISTER_FLAG_NONE,
        )
        .map_err(|e| anyhow::anyhow!("CfRegisterSyncRoot failed: {e}"))?;
    }

    tracing::info!(
        sync_root = %sync_root_path.display(),
        "Cloud Files sync root registered"
    );
    Ok(())
}
