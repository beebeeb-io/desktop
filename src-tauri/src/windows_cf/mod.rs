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
pub mod status_ui;

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

/// Provider version reported alongside `PROVIDER_ID`. It is written into the
/// Explorer SHELL registration (`StorageProviderSyncRootInfo::SetVersion`).
///
/// Bumping this is ALSO how we force a re-registration of the shell sync root
/// when its policies change: `register_shell_sync_root` skips the
/// `StorageProviderSyncRootManager::Register` call when an entry already exists
/// for the (stable) Id. A first-launch entry registered with the old policy set
/// would otherwise never pick up a policy change. The version is part of the
/// shell metadata, so a re-register IS still required to apply the new policies
/// — see `register_shell_sync_root`, which now force-unregisters a stale entry
/// whose recorded version differs from this constant before re-registering.
///
/// `1.0.0` → `1.1.0`: dropped `AutoDehydrationAllowed` from the hydration
/// modifier (pinned/opened files must stay resident — see line ~486). Existing
/// installs registered at 1.0.0 must re-register to apply the new modifier.
pub const PROVIDER_VERSION: &str = "1.1.0";

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

/// Stable, deterministic 64-bit FNV-1a hash of the sync-root path bytes.
///
/// Used only to mint the WinRT shell-registration `Id` (see
/// [`shell_sync_root_id`]). FNV-1a is chosen because it is tiny, dependency
/// free, and — critically — STABLE across releases and machines (a fixed
/// algorithm with fixed constants), which is exactly what the shell `Id`
/// requires: the same sync root must hash to the same Id on every launch so
/// re-registration is idempotent and [`unregister_shell_sync_root`] can
/// reconstruct the Id from the persisted sync-root path alone.
#[cfg(target_os = "windows")]
fn fnv1a64(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// The stable WinRT shell-registration `Id` for a given sync root.
///
/// ## Id scheme — `Beebeeb!<fnv1a64(sync_root)>` (chosen + why)
///
/// `StorageProviderSyncRootManager::Register` keys every shell entry by this
/// string Id; `Unregister` takes the same Id. So the Id MUST be:
///
/// 1. **Stable across releases/launches** — a re-register of the same root must
///    hit the same entry (idempotent), and sign-out must be able to unregister
///    it. We derive it purely from the sync-root path, which is persisted in
///    `desktop.toml` (`DesktopConfig.sync_root`) and is the one piece of
///    identity reachable from BOTH `register_shell_sync_root(sync_root)` and the
///    sign-out path (`clear_session`, which has only the config, not the running
///    engine). A fixed FNV-1a of the path is fully deterministic.
/// 2. **Forward-compatible with multi-account** — each account maps to its own
///    sync-root folder, so distinct accounts naturally hash to distinct Ids
///    without any schema change here. When real multi-account lands, the same
///    function keeps working per-root; if a SID/account component is later wanted
///    for nav-pane grouping it can be appended as a third `!`-segment
///    (`Beebeeb!<sid>!<account>`) without disturbing existing single-account Ids.
///
/// We deliberately do NOT embed the Windows user SID here (the OneDrive
/// `OneDrive!<sid>!Business1` form): pulling the SID needs extra Win32 token +
/// authorization features and an unsafe `GetTokenInformation`/`ConvertSidToStringSidW`
/// dance, and `Register` accepts a plain `provider!stableid` Id for a
/// single-account provider. The path hash gives the same stability with zero
/// extra deps and trivial reconstruction at sign-out.
///
/// The `Beebeeb!` prefix namespaces the provider (mirrors OneDrive/ProtonDrive's
/// `<Provider>!…` convention) and matches the Win32 [`PROVIDER_ID`] name so the
/// two registrations read as the same provider in tooling.
#[cfg(target_os = "windows")]
fn shell_sync_root_id(sync_root: &std::path::Path) -> String {
    // Normalise the path string the same way `register_sync_root` does (lossy
    // UTF-8 of the OS path) so the Id is computed from a canonical byte form and
    // both register + unregister agree.
    let canonical = sync_root.to_string_lossy();
    format!("{}!{:016x}", PROVIDER_ID, fnv1a64(canonical.as_bytes()))
}

/// Write the Explorer SHELL registration for the Beebeeb sync root via WinRT
/// `StorageProviderSyncRootManager::Register`.
///
/// ## What this adds (and why it's separate from [`register_sync_root`])
///
/// [`register_sync_root`] registers the root with the Cloud Files **platform**
/// (Win32 `CfRegisterSyncRoot`) — that is what drives placeholders + on-demand
/// hydration, and it is verified working. But it writes **no Explorer shell
/// metadata**, so Explorer shows no Status column, no cloud/check overlays, no
/// "Beebeeb" sidebar entry, and no "Free up space" context menu. This function
/// writes that shell metadata — the `…\Explorer\SyncRootManager\<id>` shell
/// entry, the `CLSID\{…}` namespace tree, and the nav-pane mount — in one WinRT
/// call. The per-file in-sync / pin state is already stamped by
/// `create_placeholder` (MARK_IN_SYNC) and `mark_in_sync`/`mark_not_in_sync`, so
/// once the provider is shell-registered the overlays, Status column, sidebar
/// entry, and menu all render from existing state — no per-file rework.
///
/// **Purely additive.** It does NOT touch the Win32 registration, the FETCH_DATA
/// callback, or the ALWAYS_FULL/PARTIAL population/hydration contract. The two
/// registrations are kept in agreement: the WinRT policies below mirror the
/// Win32 `CF_SYNC_POLICIES` — `HydrationPolicy = Partial` ↔ `CF_HYDRATION_POLICY_PARTIAL`,
/// `PopulationPolicy = AlwaysFull` ↔ `CF_POPULATION_POLICY_ALWAYS_FULL`.
///
/// Best-effort: every failure is returned as `Err` for the caller to log-and-
/// continue. A shell-registration failure must never break login / engine /
/// hydration — those run off the Win32 path, which is independent of this.
///
/// Idempotent: if a sync root is already shell-registered for this folder we
/// skip the re-register (a duplicate `Register` for the same Id throws), and a
/// "already registered" error is swallowed as success.
#[cfg(target_os = "windows")]
pub fn register_shell_sync_root(sync_root: &std::path::Path) -> anyhow::Result<()> {
    use windows::Foundation::Collections::IVector;
    use windows::Security::Cryptography::CryptographicBuffer;
    use windows::Storage::Provider::{
        StorageProviderHardlinkPolicy, StorageProviderHydrationPolicy,
        StorageProviderHydrationPolicyModifier, StorageProviderInSyncPolicy,
        StorageProviderItemPropertyDefinition, StorageProviderPopulationPolicy,
        StorageProviderProtectionMode, StorageProviderSyncRootInfo, StorageProviderSyncRootManager,
    };
    use windows::Storage::{IStorageFolder, StorageFolder};
    use windows::core::{HSTRING, Interface};

    let id = shell_sync_root_id(sync_root);
    let id_h = HSTRING::from(id.as_str());

    // Resolve the sync-root folder as a WinRT StorageFolder (the `Path` property
    // is a StorageFolder, not a string). `GetFolderFromPathAsync` is async; we are
    // on a normal (non-callback) thread here so blocking on `.get()` is fine.
    let path_h = HSTRING::from(&*sync_root.to_string_lossy());
    let folder: StorageFolder = StorageFolder::GetFolderFromPathAsync(&path_h)
        .map_err(|e| anyhow::anyhow!("GetFolderFromPathAsync queue failed: {e}"))?
        .get()
        .map_err(|e| anyhow::anyhow!("GetFolderFromPathAsync failed for sync root: {e}"))?;

    // Idempotency + policy-version guard. The shell Id is derived purely from the
    // sync-root PATH (stable across releases) — it does NOT encode the provider
    // version — so a plain "Id already exists" check would skip re-registration
    // forever, and a policy change (e.g. dropping AutoDehydrationAllowed in
    // PROVIDER_VERSION 1.1.0) would never reach an existing install. So we probe
    // by Id and then compare the REGISTERED version against the current
    // `PROVIDER_VERSION`:
    //   - versions match  → genuine no-op, skip the re-register (a second Register
    //     with the same Id throws ALREADY_EXISTS).
    //   - versions differ (or the version can't be read) → the registration is
    //     stale; Unregister it and fall through to Register with the new policies.
    // This is what actually makes a PROVIDER_VERSION bump take effect on upgrade.
    if let Ok(existing) = StorageProviderSyncRootManager::GetSyncRootInformationForId(&id_h) {
        let registered_version = existing.Version().map(|h| h.to_string_lossy()).unwrap_or_default();
        if registered_version == PROVIDER_VERSION {
            tracing::info!(
                sync_root = %sync_root.display(),
                version = %PROVIDER_VERSION,
                "Explorer shell sync root already registered at current version; skipping re-register"
            );
            return Ok(());
        }
        // Stale registration from an older provider version — tear it down so the
        // Register below applies the new policy set (e.g. the new hydration
        // modifier). Best-effort: if the Unregister fails the Register below will
        // surface ALREADY_EXISTS, which we swallow, so the worst case is the old
        // policies linger until the next clean sign-out — never a crash.
        tracing::info!(
            sync_root = %sync_root.display(),
            from_version = %registered_version,
            to_version = %PROVIDER_VERSION,
            "Explorer shell sync root registered at an older version; re-registering with new policies"
        );
        if let Err(e) = StorageProviderSyncRootManager::Unregister(&id_h) {
            tracing::warn!(error = %e, "Unregister of stale shell sync root failed; will attempt Register anyway");
        }
    }

    let info = StorageProviderSyncRootInfo::new()
        .map_err(|e| anyhow::anyhow!("StorageProviderSyncRootInfo::new failed: {e}"))?;

    info.SetId(&id_h)
        .map_err(|e| anyhow::anyhow!("SetId failed: {e}"))?;
    // `SetPath` takes `Param<IStorageFolder>` (the interface), and the
    // `Param<…, InterfaceType> for &U` blanket impl only fires when `U:
    // CanInto<IStorageFolder>`. `StorageFolder` (the concrete runtime class) does
    // NOT auto-declare `CanInto<IStorageFolder>` in windows-0.58 — its
    // `interface_hierarchy!` lists only IUnknown/IInspectable — so `&folder`
    // fails to satisfy the bound. QueryInterface the class to the interface it
    // implements, then pass that owned `IStorageFolder` (which trivially
    // satisfies `Param<IStorageFolder>`).
    let folder_iface: IStorageFolder = folder
        .cast::<IStorageFolder>()
        .map_err(|e| anyhow::anyhow!("cast StorageFolder→IStorageFolder failed: {e}"))?;
    info.SetPath(&folder_iface)
        .map_err(|e| anyhow::anyhow!("SetPath failed: {e}"))?;
    info.SetDisplayNameResource(&HSTRING::from("Beebeeb"))
        .map_err(|e| anyhow::anyhow!("SetDisplayNameResource failed: {e}"))?;

    // Icon: the running exe's first icon resource, the same pattern OneDrive /
    // ProtonDrive use (`"<exe>,0"`). Best-effort — if the exe path can't be
    // resolved we fall back to just the index-0 reference, which Explorer renders
    // as a generic icon rather than failing the whole registration.
    let icon_resource = std::env::current_exe()
        .ok()
        .map(|exe| format!("{},0", exe.to_string_lossy()))
        .unwrap_or_else(|| ",0".to_string());
    info.SetIconResource(&HSTRING::from(icon_resource.as_str()))
        .map_err(|e| anyhow::anyhow!("SetIconResource failed: {e}"))?;

    // ── Policies — MUST mirror the Win32 CF_SYNC_POLICIES so the platform and
    //    shell registrations agree (see register_sync_root) ──
    //
    // Partial hydration ↔ CF_HYDRATION_POLICY_PARTIAL: ranges hydrate on open via
    // the FETCH_DATA callback rather than dragging the whole file.
    info.SetHydrationPolicy(StorageProviderHydrationPolicy::Partial)
        .map_err(|e| anyhow::anyhow!("SetHydrationPolicy failed: {e}"))?;
    // StreamingAllowed ONLY — stream bytes during hydration. We deliberately
    // DROP `AutoDehydrationAllowed`: with it set, Windows freely auto-dehydrates
    // an UNPINNED file the moment it is idle after a read, so a just-opened file
    // never stays present — the root cause of "every file is perpetually
    // cloud-only". Without it, an opened/hydrated file stays resident until the
    // user (or our app via `free_up_space`) explicitly frees it, and "Always keep
    // on this device" pins (CfSetPinState) are honoured rather than silently
    // reclaimed. The native "Free up space" menu still works — it dehydrates on
    // explicit user action regardless of this modifier.
    info.SetHydrationPolicyModifier(StorageProviderHydrationPolicyModifier::StreamingAllowed)
        .map_err(|e| anyhow::anyhow!("SetHydrationPolicyModifier failed: {e}"))?;
    // AlwaysFull ↔ CF_POPULATION_POLICY_ALWAYS_FULL: the namespace is always fully
    // present locally (we eagerly mint a placeholder per cloud-only row), so
    // Explorer never asks the provider to enumerate.
    info.SetPopulationPolicy(StorageProviderPopulationPolicy::AlwaysFull)
        .map_err(|e| anyhow::anyhow!("SetPopulationPolicy failed: {e}"))?;
    // InSync `Default`: consistent with the per-file in-sync state we stamp
    // ourselves (create_placeholder MARK_IN_SYNC; mark_in_sync→CfSetInSyncState).
    // We manage in-sync explicitly, so the shell needs no extra attribute-based
    // in-sync inference.
    info.SetInSyncPolicy(StorageProviderInSyncPolicy::Default)
        .map_err(|e| anyhow::anyhow!("SetInSyncPolicy failed: {e}"))?;
    info.SetHardlinkPolicy(StorageProviderHardlinkPolicy::None)
        .map_err(|e| anyhow::anyhow!("SetHardlinkPolicy failed: {e}"))?;
    info.SetShowSiblingsAsGroup(false)
        .map_err(|e| anyhow::anyhow!("SetShowSiblingsAsGroup failed: {e}"))?;
    info.SetProtectionMode(StorageProviderProtectionMode::Unknown)
        .map_err(|e| anyhow::anyhow!("SetProtectionMode failed: {e}"))?;
    info.SetVersion(&HSTRING::from(PROVIDER_VERSION))
        .map_err(|e| anyhow::anyhow!("SetVersion failed: {e}"))?;

    // Context is an opaque per-provider blob Windows hands back to us; encode the
    // Id bytes so it is non-empty and self-describing. Built via CryptographicBuffer
    // to avoid hand-rolling a DataWriter.
    let context = CryptographicBuffer::CreateFromByteArray(id.as_bytes())
        .map_err(|e| anyhow::anyhow!("CryptographicBuffer::CreateFromByteArray failed: {e}"))?;
    info.SetContext(&context)
        .map_err(|e| anyhow::anyhow!("SetContext failed: {e}"))?;

    // Declare a Status column so Explorer renders the sync-state column for this
    // provider. Property id 0 is the canonical per-item sync-status property; the
    // per-file value is supplied through the in-sync/pin state we already stamp.
    let props: IVector<StorageProviderItemPropertyDefinition> = info
        .StorageProviderItemPropertyDefinitions()
        .map_err(|e| anyhow::anyhow!("StorageProviderItemPropertyDefinitions failed: {e}"))?;
    let status_def = StorageProviderItemPropertyDefinition::new()
        .map_err(|e| anyhow::anyhow!("StorageProviderItemPropertyDefinition::new failed: {e}"))?;
    status_def
        .SetId(0)
        .map_err(|e| anyhow::anyhow!("property SetId failed: {e}"))?;
    status_def
        .SetDisplayNameResource(&HSTRING::from("Status"))
        .map_err(|e| anyhow::anyhow!("property SetDisplayNameResource failed: {e}"))?;
    props
        .Append(&status_def)
        .map_err(|e| anyhow::anyhow!("append Status property definition failed: {e}"))?;

    // Register. Swallow an "already registered" race as success (idempotent) —
    // the guard above usually catches it, but a concurrent register could still
    // collide.
    match StorageProviderSyncRootManager::Register(&info) {
        Ok(()) => {
            tracing::info!(
                sync_root = %sync_root.display(),
                shell_id = %id,
                "Explorer shell sync root registered (Status column + overlays + sidebar)"
            );
            Ok(())
        }
        Err(e) => {
            // HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS) == 0x800700B7. Treat an
            // already-registered entry as success so a re-login reconverges
            // without surfacing a spurious error.
            if e.code().0 as u32 == 0x8007_00B7 {
                tracing::info!(
                    sync_root = %sync_root.display(),
                    "Explorer shell sync root already registered (Register returned ALREADY_EXISTS)"
                );
                Ok(())
            } else {
                Err(anyhow::anyhow!("StorageProviderSyncRootManager::Register failed: {e}"))
            }
        }
    }
}

/// Remove the Explorer SHELL registration for `sync_root` (the inverse of
/// [`register_shell_sync_root`]).
///
/// Called on sign-out / reset so a logout does not leave a dead "Beebeeb"
/// nav-pane entry, Status column, or overlays pointing at a folder the user is
/// no longer signed into. Best-effort: a failure (including "not registered")
/// is returned for the caller to log-and-continue and must never break the
/// sign-out flow.
///
/// The Id is reconstructed from the sync-root path via the same stable
/// [`shell_sync_root_id`] scheme used to register, so the caller only needs the
/// persisted `DesktopConfig.sync_root` — not the running engine.
#[cfg(target_os = "windows")]
pub fn unregister_shell_sync_root(sync_root: &std::path::Path) -> anyhow::Result<()> {
    use windows::Storage::Provider::StorageProviderSyncRootManager;
    use windows::core::HSTRING;

    let id = shell_sync_root_id(sync_root);
    let id_h = HSTRING::from(id.as_str());

    // Tear down the breadcrumb status-flyout registration FIRST (revoke the COM
    // class object + remove the registry binding under the sync-root key), while
    // the sync-root key still exists. Best-effort and self-contained; it logs its
    // own failures and never returns an error, so it cannot block the shell
    // unregister below. Inverse of `register_status_ui_for_sync_root`.
    status_ui::unregister_status_ui_source(&id);

    StorageProviderSyncRootManager::Unregister(&id_h)
        .map_err(|e| anyhow::anyhow!("StorageProviderSyncRootManager::Unregister failed: {e}"))?;
    tracing::info!(
        sync_root = %sync_root.display(),
        shell_id = %id,
        "Explorer shell sync root unregistered"
    );
    Ok(())
}

/// Guard so the dedicated status-UI COM thread is spawned at most once per
/// process. A re-login re-runs [`connect_root`], but the class object + its STA
/// apartment must persist for the whole process lifetime, so we never respawn.
static STATUS_UI_THREAD: OnceLock<()> = OnceLock::new();

/// Stand up the OneDrive-style Explorer breadcrumb **status flyout** for
/// `sync_root` (clicking "Beebeeb" in the path-bar → synced state + storage bar
/// + "Free up space"). PURELY ADDITIVE on top of [`register_shell_sync_root`].
///
/// Why a dedicated thread: Windows asks our COM `IClassFactory` for the status-UI
/// source on demand, and `CoRegisterClassObject` keeps the class object alive
/// only as long as the registering thread's COM apartment lives. The Microsoft
/// CloudMirror sample does exactly this — registers its status-UI factory on a
/// dedicated single-threaded-apartment thread that then parks on a wait. A tokio
/// worker is the wrong home: it is transient and may have its apartment torn
/// down. So we spawn one long-lived STA thread that initializes COM, registers
/// the class object + the registry binding, and parks for the process lifetime.
///
/// Spawned at most once per process (guarded by [`STATUS_UI_THREAD`]); a re-login
/// reuses the existing registration. Best-effort: any failure is logged inside
/// the thread and never propagated — the flyout is a nicety; overlays, the
/// Status column, the nav-pane entry and hydration are unaffected.
pub fn register_status_ui_for_sync_root(sync_root: &std::path::Path) {
    if STATUS_UI_THREAD.get().is_some() {
        return;
    }
    let shell_id = shell_sync_root_id(sync_root);
    let _ = STATUS_UI_THREAD.set(());

    let spawned = std::thread::Builder::new()
        .name("beebeeb-status-ui".to_string())
        .spawn(move || {
            use windows::Win32::System::Com::{
                COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize,
            };
            // SAFETY: standard COM init for this dedicated thread. Returns an
            // HRESULT; S_FALSE (already initialized) is non-error. We pair it with
            // a CoUninitialize only on the (failure) early-exit paths.
            let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
            if hr.is_err() {
                tracing::warn!(hr = ?hr, "status-UI thread: CoInitializeEx failed; flyout disabled");
                return;
            }

            if let Err(e) = status_ui::register_status_ui_source(&shell_id) {
                tracing::warn!(error = %e, "status-UI source registration failed; breadcrumb flyout absent (overlays/column/hydration unaffected)");
                // SAFETY: balance the CoInitializeEx above before exiting — no
                // point holding an apartment for a failed registration.
                unsafe { CoUninitialize() };
                return;
            }

            tracing::info!("status-UI source registered; parking COM thread for process lifetime");
            // Park forever: keep the apartment + class object alive. The class
            // object must live exactly as long as the process; process exit tears
            // it down. `unregister_status_ui_source` revokes the cookie on
            // sign-out; this parked thread simply keeps the apartment resident.
            loop {
                std::thread::park();
            }
        });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "could not spawn status-UI thread; breadcrumb flyout disabled");
    }
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

    // Write the Explorer SHELL registration (Status column + overlays + nav-pane
    // entry + "Free up space" menu) via WinRT. PURELY ADDITIVE on top of the
    // verified Win32 `register_sync_root` above. Best-effort: a failure here must
    // NOT abort the connect/hydration path (which runs entirely off the Win32
    // registration), so we log-and-continue exactly like a Cloud Files hiccup.
    // Re-running on every engine spawn makes a re-login reconverge the shell entry.
    if let Err(e) = register_shell_sync_root(sync_root) {
        tracing::warn!(error = %e, "Explorer shell sync-root registration failed; overlays/column/sidebar may be absent (hydration unaffected)");
    }

    // Stand up the OneDrive-style breadcrumb STATUS FLYOUT (clicking "Beebeeb" in
    // the Explorer path-bar → synced state + storage bar + "Free up space").
    // PURELY ADDITIVE: it spawns a dedicated COM thread that registers our
    // status-UI source factory; it never touches the Win32 registration, the
    // shell registration above, the callbacks, or hydration. Idempotent across
    // re-logins (spawns at most once per process). Failures are logged inside the
    // thread, so this call is infallible from connect_root's perspective.
    register_status_ui_for_sync_root(sync_root);

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

/// Per-tick reconcile of on-disk Cloud Files placeholder state back into the
/// state DB. Runs right after [`refresh_placeholders`] on every successful sync
/// tick (see `runner::run`).
///
/// ## Why this exists
///
/// The in-app pin / free-up path writes the DB **and** the OS placeholder
/// together, so the app's view always matches. But a user can pin or "Free up
/// space" a file **natively** in Explorer's right-click menu — that path NEVER
/// touches our DB. Without this pass, a native pin/free-up would be invisible to
/// the app (wrong status badge, stale pin) and `free_up_space` would ignore a
/// file the user pinned in Explorer. This pass reads each placeholder's actual
/// on-disk attributes and writes back any DELTA, so native changes converge.
///
/// ## Idempotent + delta-only
///
/// We compute the desired status/pin from the on-disk attribute mask
/// ([`crate::state_db::decode_os_state`]) and push to the batch **only** when it
/// differs from the live row. In steady state — and for every file changed via
/// the in-app path (DB + OS written together) — there is no delta, so nothing is
/// written. Engine-owned statuses (`Uploading`/`Downloading`/`Conflict`/`Error`)
/// are never overridden — `decode_os_state` returns `None` for them.
///
/// ## Zero-knowledge
///
/// Never logs a path or filename — only the aggregate update count.
#[cfg(target_os = "windows")]
pub fn reconcile_placeholder_state(sync_root: &std::path::Path) {
    use crate::state_db::{decode_os_state, FileStatus, PinState};

    let Some(bridge) = bridge() else {
        tracing::debug!("reconcile_placeholder_state called before bridge was set; skipping");
        return;
    };

    // `file_overview_rows` pairs each row with its EFFECTIVE-pinned flag, computed
    // with the same predicate the in-app "Files" tab uses — exactly the current
    // pin truth we must diff the OS-reported pin against.
    let rows = match bridge.db().file_overview_rows() {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "reconcile_placeholder_state: could not list files");
            return;
        }
    };

    let mut deltas: Vec<(String, Option<FileStatus>, Option<PinState>)> = Vec::new();
    for (entry, pinned) in rows {
        // Directory placeholders carry their own pin/hydration semantics handled
        // by the engine's recursive pin path; reconcile only leaf files, the same
        // surface `populate_placeholders` mints file placeholders for.
        if entry.is_dir() {
            continue;
        }

        // Map the server-relative '/'-joined key to the on-disk placeholder path
        // the SAME way `populate_placeholders` does.
        let rel = entry.path.trim_matches('/');
        if rel.is_empty() {
            continue;
        }
        let local_path = sync_root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));

        let Some(attrs) = placeholders::read_os_placeholder_state(&local_path) else {
            // Placeholder gone / unreadable this tick — skip; a later tick retries.
            continue;
        };

        let (desired_status, desired_pin) = decode_os_state(attrs, entry.status.clone());

        // Status delta: only when decode produced a candidate that differs.
        let status_delta = match desired_status {
            Some(s) if s != entry.status => Some(s),
            _ => None,
        };

        // Pin delta: decode reports the EXPLICIT per-file pin the OS carries
        // (Pinned/Unpinned) or None. Compare its boolean meaning to the row's
        // current effective-pinned flag so we don't churn on equivalent states.
        let pin_delta = match desired_pin {
            Some(PinState::Pinned) if !pinned => Some(PinState::Pinned),
            Some(PinState::Unpinned) if pinned => Some(PinState::Unpinned),
            _ => None,
        };

        if status_delta.is_some() || pin_delta.is_some() {
            deltas.push((entry.file_id, status_delta, pin_delta));
        }
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    match bridge.db().reconcile_os_state(&deltas, now) {
        Ok(updated) => {
            if updated > 0 {
                // Zero-knowledge: count only, never a path.
                tracing::debug!(updates = updated, "reconcile_os_state");
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "reconcile_os_state failed");
        }
    }
}

/// Maximum number of `CfSetInSyncState` calls issued in a single
/// [`stamp_local_files_in_sync`] pass. Mirrors the style of
/// `watcher::MAX_NEW_UPLOADS_PER_SCAN` (256) and the runner comment for
/// `known_folder::MAX_MIRROR_FILES_PER_PASS` — bounded so a vault with many
/// Local files doesn't block the tick loop. Remaining files are stamped on
/// the next tick (the call is idempotent: `CfSetInSyncState` on an already-
/// in-sync placeholder is a Windows no-op).
const MAX_IN_SYNC_STAMPS_PER_PASS: usize = 512;

/// Stamp every DB-`Local` file whose on-disk Cloud Files placeholder is NOT
/// yet marked in-sync, so Explorer shows the green ✓ overlay for those files.
///
/// ## Why this is needed
///
/// `CfSetInSyncState(IN_SYNC)` currently fires only from `fetch_data_callback`
/// when a single FETCH_DATA request covers the WHOLE file
/// (`required_offset == 0 && required_length >= file_size`). Under
/// `CF_HYDRATION_POLICY_PARTIAL` that condition rarely holds — Windows often
/// issues partial-range requests — so files that ARE fully local (status=Local
/// in the DB) may never receive the ✓ overlay. Additionally, the upload path
/// (`upload_version` / modify) transitions the DB row to `Local` but does not
/// call `CfSetInSyncState`, so freshly-uploaded files also miss the overlay.
///
/// This pass drives the built-in CF overlay from DB truth: if a row is `Local`
/// the file IS fully synced, so we stamp it in-sync regardless of how it got
/// there. `CfSetInSyncState` on an already-in-sync placeholder is a documented
/// Windows no-op, making the pass idempotent and safe to run every tick.
///
/// ## Zero-knowledge
///
/// Never logs a path or filename — only aggregate counts. Paths may appear at
/// `debug` level (matching the `mark_in_sync` sibling in `callbacks.rs`).
///
/// ## Bounded
///
/// At most [`MAX_IN_SYNC_STAMPS_PER_PASS`] files are stamped per call. Any
/// remaining Local files are stamped on subsequent ticks.
#[cfg(target_os = "windows")]
pub fn stamp_local_files_in_sync(sync_root: &std::path::Path) {
    // All CF symbols (CfOpenFileWithOplock, CfSetInSyncState, CfCloseHandle,
    // CF_IN_SYNC_STATE_IN_SYNC, CF_OPEN_FILE_FLAG_NONE, CF_SET_IN_SYNC_FLAG_NONE)
    // and PCWSTR are in scope via the module-level
    //   use windows::Win32::Storage::CloudFilters::*;
    //   use windows::core::*;
    // globs — no local re-import needed.

    let Some(bridge) = bridge() else {
        tracing::debug!("stamp_local_files_in_sync called before bridge was set; skipping");
        return;
    };

    let entries = match bridge.db().list_by_status(crate::state_db::FileStatus::Local) {
        Ok(entries) => entries,
        Err(e) => {
            tracing::warn!(error = %e, "stamp_local_files_in_sync: could not list Local files");
            return;
        }
    };

    let mut stamped: usize = 0;
    let mut errors: usize = 0;

    for entry in entries.iter().take(MAX_IN_SYNC_STAMPS_PER_PASS) {
        // Directory placeholders are managed separately (pin/hydration semantics
        // for folders differ); stamp only leaf files, consistent with how
        // `populate_placeholders` mints file placeholders.
        if entry.is_dir() {
            continue;
        }

        // Map the server-relative '/'-joined path onto the on-disk placeholder
        // path — the same safe-join that `populate_placeholders` and
        // `reconcile_placeholder_state` use.
        let rel = entry.path.trim_matches('/');
        if rel.is_empty() {
            continue;
        }
        let local_path = sync_root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));

        // Skip placeholders that don't exist on disk yet. They will be
        // minted by `refresh_placeholders` (which runs before this pass) and
        // will be stamped on the next tick once they appear.
        if !local_path.exists() {
            continue;
        }

        // Build the NUL-terminated UTF-16 path string that `CfOpenFileWithOplock`
        // expects — identical to `db_placeholder_path` in callbacks.rs.
        let path_wide: Vec<u16> = local_path
            .to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0u16))
            .collect();

        // Mirror the `mark_in_sync` pattern from callbacks.rs exactly:
        //   CfOpenFileWithOplock → CfSetInSyncState(IN_SYNC) → CfCloseHandle.
        // Handle is ALWAYS closed before the next iteration — via the Ok and
        // Err branches — so there is no handle leak on error.
        //
        // SAFETY: `path_wide` is a NUL-terminated wide string kept alive for
        // the duration of the unsafe block. The CF handle returned by
        // CfOpenFileWithOplock is closed by CfCloseHandle before return.
        // CfSetInSyncState on an already-in-sync placeholder is a no-op.
        let handle = match unsafe {
            CfOpenFileWithOplock(PCWSTR(path_wide.as_ptr()), CF_OPEN_FILE_FLAG_NONE)
        } {
            Ok(h) => h,
            Err(e) => {
                tracing::debug!(error = %e, "stamp_local_files_in_sync: CfOpenFileWithOplock failed");
                errors += 1;
                continue;
            }
        };

        if let Err(e) = unsafe {
            CfSetInSyncState(handle, CF_IN_SYNC_STATE_IN_SYNC, CF_SET_IN_SYNC_FLAG_NONE, None)
        } {
            tracing::debug!(error = %e, "stamp_local_files_in_sync: CfSetInSyncState failed");
            errors += 1;
        } else {
            stamped += 1;
        }

        // Close the handle on ALL paths (including the CfSetInSyncState error
        // branch above — the handle is still open and must be released).
        unsafe { CfCloseHandle(handle) };
    }

    if stamped > 0 || errors > 0 {
        // Zero-knowledge: counts only, never a path.
        tracing::debug!(stamped, errors, "stamp_local_files_in_sync");
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
