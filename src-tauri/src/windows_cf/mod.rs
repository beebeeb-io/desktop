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
use std::sync::RwLock;

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

/// Live handle to the tokio runtime that owns the daemon.
///
/// Cloud Files delivers the fetch callback on an OS-managed
/// filter-driver worker thread that is *not* attached to any tokio
/// runtime, so `Handle::try_current()` returns `Err` there and the
/// callback can't drive `hydrate_file` (an async fn). We stash the
/// runtime handle here from [`connect_root`] (which runs on a tokio
/// worker, so `Handle::current()` is valid) and the callback reaches
/// it through [`runtime`] to `block_on` the hydration.
static RUNTIME: OnceLock<tokio::runtime::Handle> = OnceLock::new();

/// Absolute path of the connected Cloud Files sync root.
///
/// The FETCH_DATA callback only receives a `CF_CALLBACK_INFO`, which carries
/// the file's volume-relative `NormalizedPath` ONLY when the root was connected
/// with `CF_CONNECT_FLAG_REQUIRE_FULL_FILE_PATH` — we connect WITHOUT it (task
/// 0783 finding), so `NormalizedPath` can be empty on a fetch. When the
/// callback needs an on-disk placeholder path it can't get from the callback
/// info (e.g. the resolve-or-error resize in [`callbacks::fetch_data_callback`]),
/// it falls back to `<SYNC_ROOT>/<state_db rel path>` — the same on-disk layout
/// `populate_placeholders` mints. Stashed here from [`connect_root`] (which
/// already owns the path); set-once for the process lifetime (a re-login reuses
/// the same root).
static SYNC_ROOT: OnceLock<std::path::PathBuf> = OnceLock::new();

/// Internal accessor for the callback module: the connected sync-root path, or
/// `None` if [`connect_root`] hasn't run yet. Used to reconstruct an on-disk
/// placeholder path from a state-DB relative path when the callback's
/// `NormalizedPath` is unavailable.
pub(crate) fn sync_root() -> Option<std::path::PathBuf> {
    SYNC_ROOT.get().cloned()
}

/// Stable provider GUID for the Beebeeb sync root.
///
/// Minted once (UUIDv4 `b33b33b0-5217-4c0a-9e3f-1f2a7c4d8e90`) and
/// hardcoded so the sync-root identity is stable across releases and
/// machines. Previously `GUID::zeroed()`, a dev shortcut that let
/// Windows pick a per-machine default — fine for local testing but it
/// would orphan placeholders if the default ever changed. A fixed GUID
/// keeps the provider identity deterministic.
///
/// `GUID::from_u128` lays the value out in the canonical
/// big-endian-string order, so the literal here matches the dashed
/// form above.
pub const BEEBEEB_PROVIDER_GUID: GUID = GUID::from_u128(0xb33b33b0_5217_4c0a_9e3f_1f2a7c4d8e90);

/// Set once the first time we successfully `CfConnectSyncRoot`. Used to
/// make callback registration idempotent across engine respawns (re-login,
/// sync-root change) — Windows rejects a second connect on a root that's
/// already connected by this process, so we connect exactly once per
/// process lifetime.
static CONNECTED: OnceLock<()> = OnceLock::new();

/// Sender into the upload driver's debounce loop (`crate::watcher`). The
/// `extern "system"` NOTIFY callbacks can't capture state, so they reach the
/// live channel through this slot. Overwritten by [`set_notify_sender`] from
/// `watcher::spawn`; read by [`notify_sender`] in the callback module.
///
/// **Must be RE-POINTABLE, not set-once.** On a re-login or sync-root change the
/// runner DROPS the old runner and re-spawns IN THE SAME PROCESS (see
/// `runner::run` lifecycle), so `watcher::spawn` runs again and creates a BRAND
/// NEW channel. The previous debounce loop has ended, so the old channel's
/// receiver is gone and every `send` into it errors. A set-once `OnceLock` would
/// keep the dead first sender forever → local uploads would silently stop after
/// the first logout→login. So we store the sender in a `RwLock<Option<…>>` and
/// OVERWRITE it on every (re)spawn, always pointing at the live debounce loop.
///
/// (This differs from the `BRIDGE`/`RUNTIME` `OnceLock`s, which are genuinely
/// set-once: a respawn reuses the same runtime + rebuilds the bridge value, and
/// neither has a receiver end that gets torn down. A channel does.)
static NOTIFY_TX: RwLock<Option<tokio::sync::mpsc::UnboundedSender<crate::watcher::NotifyEvent>>> =
    RwLock::new(None);

/// Register (or RE-register) the upload driver's event sender so the Cloud Files
/// NOTIFY callbacks ([`callbacks::notify_file_close_completion_callback`] and
/// friends) push create/modify/delete/rename events into the LIVE debounce loop.
/// Called from `watcher::spawn` on every (re)spawn — it OVERWRITES any prior
/// sender so a re-login installs the new channel and uploads keep working.
pub fn set_notify_sender(tx: tokio::sync::mpsc::UnboundedSender<crate::watcher::NotifyEvent>) {
    if let Ok(mut slot) = NOTIFY_TX.write() {
        *slot = Some(tx);
    }
}

/// Internal accessor for the callback module: a clone of the CURRENT NOTIFY-event
/// sender, or `None` if `watcher::spawn` hasn't registered one yet (e.g. a NOTIFY
/// callback raced engine startup, or a logout cleared it). The callback drops the
/// event in that case — a missed startup event is reconciled by the next
/// `sync_tick`.
pub(crate) fn notify_sender() -> Option<tokio::sync::mpsc::UnboundedSender<crate::watcher::NotifyEvent>> {
    NOTIFY_TX.read().ok().and_then(|slot| slot.clone())
}

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

/// Internal accessor for the daemon's tokio runtime handle, used by the
/// fetch callback to bridge sync→async. `None` if [`connect_root`] never ran
/// (e.g. registration failed before the handle was stashed) — the
/// callback must then fail the hydration rather than hang Explorer.
pub(crate) fn runtime() -> Option<tokio::runtime::Handle> {
    RUNTIME.get().cloned()
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
            // Stable, hardcoded provider GUID (see BEEBEEB_PROVIDER_GUID)
            // so the sync-root identity is deterministic across releases
            // and machines, rather than the previous GUID::zeroed() dev
            // shortcut that let Windows pick a per-machine default.
            ProviderId: BEEBEEB_PROVIDER_GUID,
        };
        // Hydration policy `PARTIAL` = Windows hydrates ranges on demand
        // rather than dragging down the whole file (large vault
        // friendly) — this stays, the FETCH_DATA callback serves bytes
        // lazily on open.
        //
        // Population policy `ALWAYS_FULL` (was `FULL`, originally `PARTIAL`):
        // the provider PRE-POPULATES the whole namespace by eagerly minting a
        // placeholder for every cloud-only row (`populate_placeholders`,
        // re-run on every tick via `refresh_placeholders` so newly discovered
        // rows are added). `ALWAYS_FULL` is the "the namespace is ALWAYS
        // present locally in full — never forward an enumeration request to
        // the provider" contract: Windows treats every directory as fully
        // enumerated by the on-disk placeholders and NEVER asks us for more,
        // so no `FETCH_PLACEHOLDERS` callback is ever needed.
        //
        // Why not `FULL`: `FULL` only marks a directory fully-populated AFTER
        // the provider explicitly stamps it `CF_PLACEHOLDER_CREATE_FLAG_DISABLE_ON_DEMAND_POPULATION`
        // (or sets pin state); any directory NOT yet marked is still treated
        // as "not fully populated" and Windows BLOCKS its first Explorer
        // listing waiting for a `FETCH_PLACEHOLDERS` callback — which this
        // in-process provider never registered, so enumeration hung/timed out.
        // `ALWAYS_FULL` removes that per-directory requirement entirely: the
        // namespace is unconditionally complete, so enumeration is served from
        // the placeholders with no callback round-trip.
        //
        // The vault is small and (today) flat — `sync_tick` lists only the
        // root, non-recursively — so eager full population is cheap and
        // correct. If deep recursive listing is added later and the vault
        // grows large, the alternative is to switch back to `PARTIAL` and
        // register a real `CF_CALLBACK_TYPE_FETCH_PLACEHOLDERS` that serves a
        // directory's children from the DB via `CfExecute(TRANSFER_PLACEHOLDERS)`.
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
                // `CF_POPULATION_POLICY_ALWAYS_FULL` (windows 0.58:
                // `CF_POPULATION_POLICY_PRIMARY(3)`) — verified present in the
                // vendored bindings. The namespace is ALWAYS fully present
                // locally; Windows never forwards an enumeration request, so no
                // FETCH_PLACEHOLDERS callback is required.
                Primary: CF_POPULATION_POLICY_ALWAYS_FULL,
                Modifier: CF_POPULATION_POLICY_MODIFIER_NONE,
            },
            InSync: CF_INSYNC_POLICY_NONE,
            HardLink: CF_HARDLINK_POLICY_NONE,
            PlaceholderManagement: CF_PLACEHOLDER_MANAGEMENT_POLICY_DEFAULT,
        };

        // Flags:
        //  - `UPDATE` so a root previously registered with an older policy
        //    (`PARTIAL`, or `FULL`) — e.g. an onboarding that ran before this
        //    fix — is re-registered in place with the new `ALWAYS_FULL` policy
        //    rather than the call no-opping on the stale registration.
        //  - `DISABLE_ON_DEMAND_POPULATION_ON_ROOT` is the explicit
        //    "this provider fully pre-populates the namespace; do NOT
        //    expect an on-demand FETCH_PLACEHOLDERS callback" contract
        //    that pairs with `ALWAYS_FULL` — it reinforces, at the root, that
        //    Explorer must not block on a placeholder fetch that never comes.
        let register_flags = CF_REGISTER_FLAG_UPDATE | CF_REGISTER_FLAG_DISABLE_ON_DEMAND_POPULATION_ON_ROOT;
        CfRegisterSyncRoot(PCWSTR(path_wide.as_ptr()), &info, &policies, register_flags)
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
/// `CF_CALLBACK_TYPE_NONE` sentinel, per the Cloud Files contract. We register:
///
/// - `FETCH_DATA` — the bytes-on-open (download) hook. Directory *enumeration*
///   is served from the eagerly pre-created on-disk placeholders (the root is
///   registered `CF_POPULATION_POLICY_ALWAYS_FULL` +
///   `DISABLE_ON_DEMAND_POPULATION_ON_ROOT`), so Windows never asks us for
///   placeholders on demand and no `FETCH_PLACEHOLDERS` callback is required.
/// - `NOTIFY_FILE_CLOSE_COMPLETION` — the **UPLOAD** trigger (task 0780). Fires
///   AFTER a handle that wrote a new/modified file in the connected root closes.
///   Unlike a `notify`/ReadDirectoryChanges watcher (which the CF filter starves
///   on a connected root), this DOES fire, so it is the create/modify hook for
///   local uploads.
/// - `NOTIFY_DELETE_COMPLETION` — fires after a local delete; propagates a
///   server trash for a known file.
/// - `NOTIFY_RENAME_COMPLETION` — fires after a local rename/move; propagates a
///   server move/rename. Its params arm carries the OLD path (`SourcePath`); the
///   NEW path is `CF_CALLBACK_INFO.NormalizedPath`.
///
/// No special connect flag is needed for NOTIFY delivery: the
/// `NOTIFY_*_COMPLETION` callbacks are delivered to any registered handler
/// post-operation. `CF_CONNECT_FLAG_REQUIRE_PROCESS_INFO` (kept) only asks
/// Windows to populate `CF_CALLBACK_INFO.ProcessInfo`; it does not gate notify
/// delivery. [LEAD native build: confirm the three NOTIFY callbacks actually
/// fire on a real CfConnectSyncRoot'd root — the bindings + table shape are
/// verified against windows-0.58, but only a native run proves delivery.]
fn connect_callbacks(sync_root_path: &std::path::Path) -> anyhow::Result<()> {
    let path_wide: Vec<u16> = sync_root_path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    // The table must outlive the call; it lives on this stack frame and
    // `CfConnectSyncRoot` copies what it needs before returning. The
    // CF_CALLBACK_TYPE_NONE sentinel MUST stay last.
    let table = [
        CF_CALLBACK_REGISTRATION {
            Type: CF_CALLBACK_TYPE_FETCH_DATA,
            Callback: Some(callbacks::fetch_data_callback),
        },
        // Upload trigger: a handle that wrote a new/modified file has closed.
        CF_CALLBACK_REGISTRATION {
            Type: CF_CALLBACK_TYPE_NOTIFY_FILE_CLOSE_COMPLETION,
            Callback: Some(callbacks::notify_file_close_completion_callback),
        },
        // Delete trigger: a local delete completed.
        CF_CALLBACK_REGISTRATION {
            Type: CF_CALLBACK_TYPE_NOTIFY_DELETE_COMPLETION,
            Callback: Some(callbacks::notify_delete_completion_callback),
        },
        // Rename/move trigger: a local rename completed.
        CF_CALLBACK_REGISTRATION {
            Type: CF_CALLBACK_TYPE_NOTIFY_RENAME_COMPLETION,
            Callback: Some(callbacks::notify_rename_completion_callback),
        },
        // Sentinel terminator — required by the API; MUST be last.
        CF_CALLBACK_REGISTRATION {
            Type: CF_CALLBACK_TYPE_NONE,
            Callback: None,
        },
    ];

    // SAFETY: `path_wide` and `table` are kept alive on this stack frame for
    // the duration of the call. The callback function pointers have static
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

/// **Phase 1 of activation — run EARLY in `runner::run`, BEFORE the engine
/// writes anything into the sync root** (the `.beebeeb-sync.lock` file and the
/// `.beebeeb/state.db` database both live inside the root).
///
/// This is the fix for the bootstrap-ordering deadlock: `CfRegisterSyncRoot`
/// alone puts the folder under Cloud Files control but leaves it
/// *disconnected*, and a registered-but-disconnected root rejects **all**
/// writes with `0x801F0005 ERROR_CLOUD_FILE_PROVIDER_NOT_RUNNING`. If the
/// engine tries to acquire the lock or open the state DB before the provider
/// connects, those writes fail and `run()` bails out — so the connect step
/// below never runs and the root is permanently non-functional. By connecting
/// here, before the first write, the root is live and writable when the lock
/// and DB are created.
///
/// Steps (no DB / bridge required — connecting only needs the registered root):
///
/// 1. Stash the current tokio runtime handle so the Cloud Files fetch callback
///    (which fires on an OS filter-driver thread with no runtime attached) can
///    `block_on` the async hydration. `run` calls this on a tokio worker, so
///    `Handle::current()` is valid here. Idempotent.
/// 2. Register the sync root with Windows ([`register_sync_root`]).
///    Re-registering the same path is a no-op at the OS layer (and
///    `install_windows_shell_integration` already registered it during
///    onboarding — this makes the engine path self-contained for re-logins).
/// 3. Connect the in-process fetch callback table via `CfConnectSyncRoot`,
///    exactly once per process (guarded by the `CONNECTED` OnceLock — Windows
///    rejects a second connect on an already-connected root).
///
/// The fetch callback reaches the bridge through the `BRIDGE` OnceLock, which
/// is set later in [`seed_placeholders`]. That ordering is safe: no
/// placeholders exist yet (they are minted in `seed_placeholders`), so Windows
/// has nothing to fetch and the callback cannot fire before the bridge is set.
///
/// All failures are logged rather than propagated: a Cloud Files hiccup must
/// not take down the sync loop, which still works headlessly.
pub fn connect_root(sync_root: &std::path::Path) {
    // Stash the current tokio runtime handle so the Cloud Files fetch
    // callback (which fires on an OS filter-driver thread with no runtime
    // attached) can `block_on` the async hydration. This runs on a tokio
    // worker, so `Handle::current()` is valid here. Idempotent.
    let _ = RUNTIME.set(tokio::runtime::Handle::current());

    // Stash the sync-root path so the fetch callback can reconstruct an on-disk
    // placeholder path from a state-DB relative path when `CF_CALLBACK_INFO`'s
    // `NormalizedPath` is unavailable (task 0783 resolve-or-error fallback).
    let _ = SYNC_ROOT.set(sync_root.to_path_buf());

    if let Err(e) = register_sync_root(sync_root) {
        tracing::error!(error = %e, "Cloud Files sync root registration failed; connect skipped");
        return;
    }

    // Connect the in-process fetch callback exactly once per process. Without
    // this, Windows shows placeholders but never asks us to hydrate them — and,
    // critically, the root stays DISCONNECTED and rejects the engine's lock +
    // state.db writes with ERROR_CLOUD_FILE_PROVIDER_NOT_RUNNING.
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
}

/// **Phase 2 of activation — run LATE in `runner::run`, AFTER the state DB is
/// opened and the engine bridge is built.**
///
/// 1. Stash the live [`EngineBridge`] so the in-process fetch callback
///    ([`callbacks::fetch_data_callback`]) can reach it on hydration. Done here
///    (not in [`connect_root`]) because the bridge needs the open DB, and the
///    callback cannot fire until a placeholder exists — which only happens in
///    step 2 below — so setting the bridge just before populating is in time.
/// 2. Seed Explorer with cloud-only placeholders from the current DB file list
///    ([`populate_placeholders`]). Existing placeholders return
///    `ERROR_ALREADY_EXISTS`, which we swallow.
///
/// Requires [`connect_root`] to have run first so the root is connected and the
/// placeholder writes land in a live (writable) Cloud Files root.
pub fn seed_placeholders(bridge: Arc<EngineBridge>, sync_root: &std::path::Path) {
    // The callback can fire as soon as a placeholder exists, so the bridge must
    // be reachable before we create any placeholder below.
    set_bridge(bridge.clone());

    let created = populate_placeholders(&bridge, sync_root);
    tracing::info!(
        placeholders = created,
        sync_root = %sync_root.display(),
        "Cloud Files placeholders seeded"
    );
}

/// Re-run placeholder population from the current DB so cloud-only rows
/// discovered by LATER `sync_tick`s (after the one-shot [`seed_placeholders`]
/// at engine spawn) also appear in Explorer.
///
/// This is the fix for the "placeholders only seeded once on an empty DB" bug:
/// `seed_placeholders` runs a single time at startup, when the DB is typically
/// empty (placeholders=0), so the first metadata pull's rows — and every row
/// added on subsequent ticks — never got a placeholder. The runner now calls
/// this after each successful sync tick.
///
/// Idempotent and cheap to repeat: [`populate_placeholders`] only walks rows
/// currently in `CloudOnly` status and skips any with an empty `path` (so a
/// row whose name hasn't decrypted yet is simply retried next tick once the
/// path is populated). `create_placeholder` maps the steady-state
/// `ERROR_ALREADY_EXISTS` to `Ok`, so re-creating an existing placeholder is a
/// no-op rather than an error — calling this every tick is safe.
///
/// No-op (logs at debug) until [`seed_placeholders`] has stashed the bridge;
/// the runner only calls this on ticks that run after the bridge is built, so
/// in practice the bridge is always present here.
pub fn refresh_placeholders(sync_root: &std::path::Path) {
    let Some(bridge) = bridge() else {
        tracing::debug!("refresh_placeholders called before bridge was set; skipping");
        return;
    };
    let ensured = populate_placeholders(&bridge, sync_root);
    if ensured > 0 {
        // `ensured` counts every cloud-only row whose placeholder is now present,
        // INCLUDING ones that already existed (re-creating an existing placeholder
        // maps ERROR_ALREADY_EXISTS → Ok). So this is "placeholders present after
        // refresh", not "newly added rows" — keep the wording neutral so a
        // steady-state tick doesn't read as if new files appeared every time.
        tracing::debug!(
            placeholders = ensured,
            sync_root = %sync_root.display(),
            "Cloud Files placeholders refreshed"
        );
    }
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
    let mut entries = match bridge.db().list_by_status(crate::state_db::FileStatus::CloudOnly) {
        Ok(entries) => entries,
        Err(e) => {
            tracing::warn!(error = %e, "could not list cloud-only files for placeholder seeding");
            return 0;
        }
    };

    // PARENTS-BEFORE-CHILDREN. A nested file's placeholder must land INSIDE its
    // parent directory's placeholder, and a directory placeholder must exist
    // before anything is created under it. Sorting by path depth (number of
    // '/'-separated segments, ascending) guarantees every ancestor directory
    // row is processed before its descendants. Folders and files at the same
    // depth are independent, so their relative order is irrelevant.
    //
    // Why this matters with directory placeholders: a folder row is minted as a
    // real CF directory placeholder (DISABLE_ON_DEMAND_POPULATION) so Explorer
    // treats it as fully enumerated by its on-disk children. If we instead
    // processed a child first, `ensure_directory` would `mkdir` a PLAIN OS
    // directory at the parent path, and the later folder-row `create_placeholder`
    // would no-op on ERROR_ALREADY_EXISTS — leaving a plain dir where a CF
    // directory placeholder was wanted. Depth-ordering avoids that race.
    entries.sort_by_key(|e| e.path.trim_matches('/').split('/').count());

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
                // Zero-knowledge: log the file_id only, never the decrypted
                // plaintext path/name.
                tracing::warn!(file_id = %entry.file_id, "skipping placeholder for path with no file name");
                continue;
            }
        };

        // Ensure the immediate parent directory exists on disk. For a nested
        // row, depth-ordering means the parent FOLDER row was already minted as
        // a CF directory placeholder above, so this is a no-op for it; this call
        // only materialises the sync root itself (depth-1 rows) or backfills any
        // intermediate dir the server didn't surface as its own row.
        if let Err(e) = crate::config::ensure_directory(&parent_dir) {
            // Zero-knowledge: the parent dir is built from the decrypted
            // relative path, so it can carry plaintext folder names — log the
            // file_id only, not the path.
            tracing::warn!(error = %e, file_id = %entry.file_id, "could not create placeholder parent dir");
            continue;
        }

        match placeholders::create_placeholder(
            &parent_dir,
            &entry.file_id,
            &name,
            entry.size_bytes,
            entry.modified_at,
            entry.is_dir(),
        ) {
            Ok(()) => created += 1,
            Err(e) => {
                // Zero-knowledge: log the file_id only — never the decrypted
                // plaintext filename.
                tracing::warn!(
                    error = %e,
                    file_id = %entry.file_id,
                    "placeholder creation failed"
                );
            }
        }
    }
    created
}
