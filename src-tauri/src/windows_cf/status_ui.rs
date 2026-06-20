//! OneDrive-style Explorer **status flyout** for the Beebeeb sync root.
//!
//! Clicking the "Beebeeb" entry in the Explorer breadcrumb / path-bar opens a
//! flyout that shows "Your files are synced", a storage progress bar
//! ("X GB used of Y (Z%)") and a "Free up space" button. Windows drives that
//! flyout by asking our cloud provider for a **status UI source**: it calls our
//! [`IStorageProviderStatusUISourceFactory`], gets back an
//! [`IStorageProviderStatusUISource`], and calls
//! [`IStorageProviderStatusUISource::GetStatusUI`] to build the
//! [`StorageProviderStatusUI`] it renders.
//!
//! ## How Windows finds this (registration — researched against CloudMirror)
//!
//! The Microsoft `CloudMirror` sample registers its status-UI source factory by
//! (1) implementing an `IClassFactory` whose `CreateInstance` returns the
//! `IStorageProviderStatusUISourceFactory`, and (2) calling
//! [`CoRegisterClassObject`] under a stable CLSID with `CLSCTX_LOCAL_SERVER` +
//! `REGCLS_MULTIPLEUSE` (see `ShellServices.cpp`). That sample is **MSIX-
//! packaged**, so it *additionally* declares the CLSID→provider binding in its
//! appxmanifest:
//! `<cloudfiles2:StorageProviderStatusUISourceFactory Clsid="…"/>` inside the
//! `windows.cloudFiles` extension, plus a `<com:ExeServer>` for the CLSID.
//!
//! Beebeeb's desktop app is **unpackaged** (a plain Tauri `.exe`, no MSIX), so
//! the manifest entries have no effect — the equivalent must be written to the
//! **registry**, exactly as the MS "Build a cloud file sync engine" guide says
//! for the sibling handlers (CopyHook / ShareHandler / SearchHandlerFactory):
//! *"the provider registers their handler by setting the `<Handler>` registry
//! value under their **sync root registry key** to the CLSID of their COM local
//! server object."* So we:
//!
//! 1. `CoRegisterClassObject(CLSID, …, CLSCTX_LOCAL_SERVER, REGCLS_MULTIPLEUSE)`
//!    at engine startup so this running `.exe` *is* the local server for the
//!    CLSID for the process lifetime.
//! 2. Write the CLSID's `LocalServer32` (the exe path) under
//!    `HKCU\Software\Classes\CLSID\{clsid}\LocalServer32`, so the shell can launch
//!    us on demand if we are not already running.
//! 3. Write the value `StorageProviderStatusUISourceFactory = {clsid}` under the
//!    sync-root registry key
//!    `HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\SyncRootManager\<id>`
//!    (the key `StorageProviderSyncRootManager::Register` already created), which
//!    is what binds the factory to our sync root so Windows asks *us* for the
//!    flyout.
//!
//! ## Threading + the cached snapshot (why `GetStatusUI` never blocks)
//!
//! Windows calls `GetStatusUI` on an arbitrary COM thread, on theme/contrast
//! changes and whenever it refreshes the flyout — and it must return fast. So it
//! reads ONLY a process-global [`STATUS_SNAPSHOT`] of plain atomics that the
//! engine refreshes cheaply each tick (state + in-flight/conflict counts) and
//! that the storage IPC refreshes on demand (used/quota bytes). No locks held
//! across the call, no `await`, no DB open, no network. The same data
//! `sync_status` / `desktop_storage_summary` already compute feeds the snapshot,
//! so the flyout never disagrees with the settings page.
//!
//! Purely additive: this module does NOT touch `register_sync_root`,
//! `register_shell_sync_root`, `connect_callbacks`, or the hydration path. A
//! failure to register the status UI only costs the flyout — overlays, the
//! Status column, the nav-pane entry and hydration all keep working.

#![cfg(target_os = "windows")]

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};

use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Com::{
    CLSCTX_LOCAL_SERVER, CoRegisterClassObject, CoRevokeClassObject, IClassFactory,
    IClassFactory_Impl, REGCLS_MULTIPLEUSE,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW,
};
use windows::Foundation::Uri;
use windows::Storage::Provider::{
    IStorageProviderStatusUISource, IStorageProviderStatusUISourceFactory,
    IStorageProviderStatusUISource_Impl, IStorageProviderStatusUISourceFactory_Impl,
    IStorageProviderUICommand, IStorageProviderUICommand_Impl, StorageProviderQuotaUI,
    StorageProviderState, StorageProviderStatusUI, StorageProviderUICommandState,
};
use windows::core::{GUID, HSTRING, Interface, PCWSTR, Result as WinResult, implement, w};

// ── Stable CLSID for the Beebeeb status-UI source factory ──────────────────
//
// Minted once (UUIDv4) and hardcoded so the registry binding written at
// registration and the `CoRegisterClassObject` at startup always agree, and so
// `unregister` can find and remove the same key. Distinct from
// `BEEBEEB_PROVIDER_GUID` (the sync-root identity GUID) — this is the COM class
// id of the *factory*, not the provider.
//
// `{c0b33b33-5217-4c0a-9e3f-2f3b8d5e9fa1}`
pub const STATUS_UI_FACTORY_CLSID: GUID = GUID::from_u128(0xc0b3_3b33_5217_4c0a_9e3f_2f3b8d5e9fa1);

// ── Cached, lock-light status snapshot read by the COM `GetStatusUI` ────────
//
// Plain atomics so the COM call reads them with no lock and no chance of
// blocking the shell. The engine writes `state` each tick (it derives the value
// from the same paused/in-flight/conflict signals `sync_status` uses); the
// storage IPC writes `used`/`quota` when it fetches billing usage. `quota_known`
// gates whether the flyout shows a quota bar at all (we must not render
// "0 of 0" before the first usage fetch).
struct StatusSnapshot {
    /// `StorageProviderState` discriminant (InSync=0, Syncing=1, Paused=2,
    /// Error=3…). Defaults to `InSync` so a flyout opened before the first tick
    /// reads "synced" rather than a scary error.
    state: AtomicI32,
    /// Account usage / quota (bytes). Only meaningful once `quota_known`.
    used_bytes: AtomicU64,
    quota_bytes: AtomicU64,
    quota_known: AtomicBool,
}

static STATUS_SNAPSHOT: StatusSnapshot = StatusSnapshot {
    state: AtomicI32::new(0), // StorageProviderState::InSync
    used_bytes: AtomicU64::new(0),
    quota_bytes: AtomicU64::new(0),
    quota_known: AtomicBool::new(false),
};

/// Update the cached sync-state portion of the flyout snapshot. Called from the
/// engine runner each tick (and on pause/resume) with the same counts
/// `sync_status` exposes. Lock-free and cheap — safe to call every tick.
///
/// The derived `StorageProviderState` precedence mirrors the settings pill:
/// paused > conflicts(error) > in-flight(syncing) > in-sync.
pub fn set_engine_state(paused: bool, syncing: u32, conflicts: u32, engine_error: bool) {
    let state = if paused {
        StorageProviderState::Paused
    } else if conflicts > 0 || engine_error {
        StorageProviderState::Error
    } else if syncing > 0 {
        StorageProviderState::Syncing
    } else {
        StorageProviderState::InSync
    };
    STATUS_SNAPSHOT.state.store(state.0, Ordering::Relaxed);
}

/// Update the cached quota portion of the flyout snapshot. Called when the
/// storage IPC (`desktop_storage_summary`) successfully fetches billing usage,
/// so the flyout's progress bar reflects the same numbers the control center
/// shows. Marks the quota as known so the flyout begins rendering the bar.
pub fn set_quota(used_bytes: u64, quota_bytes: u64) {
    STATUS_SNAPSHOT.used_bytes.store(used_bytes, Ordering::Relaxed);
    STATUS_SNAPSHOT.quota_bytes.store(quota_bytes, Ordering::Relaxed);
    STATUS_SNAPSHOT.quota_known.store(true, Ordering::Relaxed);
}

/// Human-readable byte size, e.g. `30.7 GB` / `1 TB`. Kept dependency-free and
/// tiny so it is callable from the COM thread.
fn human_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    const TB: f64 = GB * 1024.0;
    let b = bytes as f64;
    if b >= TB {
        let v = b / TB;
        // Whole terabytes render as "1 TB", fractional as "1.5 TB".
        if (v - v.round()).abs() < 0.05 {
            format!("{} TB", v.round() as u64)
        } else {
            format!("{v:.1} TB")
        }
    } else if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

/// The exe's first icon resource as a `file:` URI, for the flyout's provider/
/// state/command icons. Best-effort: an unresolved exe path yields `None` and we
/// simply omit the icon (Explorer renders a generic one) rather than fail the
/// whole flyout.
fn provider_icon_uri() -> Option<Uri> {
    let exe = std::env::current_exe().ok()?;
    // `ms-appx:` is for packaged apps; an unpackaged exe must use a plain file
    // URI. `Uri::CreateUri` wants an absolute URI string as an `HSTRING`.
    let uri_str = format!("file:///{}", exe.to_string_lossy().replace('\\', "/"));
    Uri::CreateUri(&HSTRING::from(uri_str)).ok()
}

/// A `Uri` to use for a command's icon: the provider exe icon when resolvable,
/// otherwise a harmless absolute `file:///` placeholder. The
/// [`IStorageProviderUICommand::Icon`] contract requires a non-error `Uri`
/// return; a `Uri` is always producible here so the command still renders even
/// without a bespoke icon asset.
fn command_icon_uri() -> WinResult<Uri> {
    match provider_icon_uri() {
        Some(uri) => Ok(uri),
        None => Uri::CreateUri(&HSTRING::from("file:///")),
    }
}

// ── The "Free up space" command shown in the flyout ─────────────────────────
//
// `Invoke` runs the SAME reclaim path as the `free_up_space` IPC
// (`crate::free_up_space_blocking`) — dehydrate every unpinned, fully-local
// placeholder back to cloud-only. It is a synchronous blocking fn (DB +
// filesystem), so it runs directly on the COM thread Windows delivers `Invoke`
// on — no tokio runtime required (and the engine's runtime may not even be
// reachable from this thread). Bounded, best-effort: a reclaim error is logged,
// never surfaced as a COM failure that would make the flyout look broken.
#[implement(IStorageProviderUICommand)]
struct FreeUpSpaceCommand;

// windows-rs 0.58 convention: `#[implement(IFoo)] struct Bar;` generates a
// `Bar_Impl` wrapper, and the `IFoo_Impl` trait must be implemented on the
// WRAPPER (`Bar_Impl`), not the base struct. The wrapper `Deref`s to the base
// (`Deref::Target = Bar`, via `&self.this`), so a field-less command needs no
// field access changes; a struct with fields reaches them via `self.this.<field>`.
impl IStorageProviderUICommand_Impl for FreeUpSpaceCommand_Impl {
    fn Label(&self) -> WinResult<HSTRING> {
        Ok(HSTRING::from("Free up space"))
    }
    fn Description(&self) -> WinResult<HSTRING> {
        Ok(HSTRING::from(
            "Make files online-only to reclaim local disk space. Files stay safe in Beebeeb.",
        ))
    }
    fn Icon(&self) -> WinResult<Uri> {
        // No bespoke icon asset yet — reuse the provider exe icon; if even that
        // is unresolved, a placeholder `file:///` URI is returned so the command
        // still renders (Explorer falls back to a default glyph).
        command_icon_uri()
    }
    fn State(&self) -> WinResult<StorageProviderUICommandState> {
        Ok(StorageProviderUICommandState::Enabled)
    }
    fn Invoke(&self) -> WinResult<()> {
        match crate::free_up_space_blocking() {
            Ok(result) => {
                tracing::info!(
                    bytes_freed = result.bytes_freed,
                    "Explorer status flyout: Free up space invoked"
                );
            }
            Err(e) => {
                // Don't fail the COM call — the user asked to free space, a
                // partial/failed reclaim is logged and the flyout closes cleanly.
                tracing::warn!(error = %e, "Explorer status flyout: Free up space failed");
            }
        }
        Ok(())
    }
}

// ── A generic informational command (the sync-status banner line) ───────────
#[implement(IStorageProviderUICommand)]
struct InfoCommand {
    label: HSTRING,
    description: HSTRING,
}

impl IStorageProviderUICommand_Impl for InfoCommand_Impl {
    fn Label(&self) -> WinResult<HSTRING> {
        // `self` is the `InfoCommand_Impl` wrapper; reach the base struct's
        // fields through `self.this` (the original `InfoCommand`).
        Ok(self.this.label.clone())
    }
    fn Description(&self) -> WinResult<HSTRING> {
        Ok(self.this.description.clone())
    }
    fn Icon(&self) -> WinResult<Uri> {
        command_icon_uri()
    }
    fn State(&self) -> WinResult<StorageProviderUICommandState> {
        Ok(StorageProviderUICommandState::Enabled)
    }
    fn Invoke(&self) -> WinResult<()> {
        // Banner line is informational; nothing to do on click.
        Ok(())
    }
}

// ── The status UI source: builds the flyout's StorageProviderStatusUI ───────
#[implement(IStorageProviderStatusUISource)]
struct StatusUiSource;

impl IStorageProviderStatusUISource_Impl for StatusUiSource_Impl {
    fn GetStatusUI(&self) -> WinResult<StorageProviderStatusUI> {
        let ui = StorageProviderStatusUI::new()?;

        // ── Provider state (read the cached snapshot — no lock, no await) ──
        let state = StorageProviderState(STATUS_SNAPSHOT.state.load(Ordering::Relaxed));
        ui.SetProviderState(state)?;

        let (state_label, banner_label, banner_desc) = match state {
            StorageProviderState::Syncing => (
                "Syncing",
                "Your files are syncing",
                "Beebeeb is syncing your files.",
            ),
            StorageProviderState::Paused => (
                "Paused",
                "Sync is paused",
                "Resume Beebeeb to keep your files up to date.",
            ),
            StorageProviderState::Error => (
                "Attention needed",
                "Some files need attention",
                "Open Beebeeb to resolve a sync conflict or error.",
            ),
            _ => (
                "Up to date",
                "Your files are synced",
                "All your files are synced with Beebeeb.",
            ),
        };
        ui.SetProviderStateLabel(&HSTRING::from(state_label))?;
        if let Some(icon) = provider_icon_uri() {
            // Best-effort: a missing icon must not fail the whole flyout.
            let _ = ui.SetProviderStateIcon(&icon);
        }

        // ── Sync-status banner ("Your files are synced") ──
        let banner: IStorageProviderUICommand = InfoCommand {
            label: HSTRING::from(banner_label),
            description: HSTRING::from(banner_desc),
        }
        .into();
        ui.SetSyncStatusCommand(&banner)?;

        // ── Quota progress bar ("30.7 GB used of 1 TB (2%)") ──
        // Only when we have real numbers — never render "0 of 0".
        if STATUS_SNAPSHOT.quota_known.load(Ordering::Relaxed) {
            let used = STATUS_SNAPSHOT.used_bytes.load(Ordering::Relaxed);
            let total = STATUS_SNAPSHOT.quota_bytes.load(Ordering::Relaxed);
            if total > 0 {
                let quota = StorageProviderQuotaUI::new()?;
                quota.SetQuotaTotalInBytes(total)?;
                quota.SetQuotaUsedInBytes(used.min(total))?;
                let pct = ((used as f64 / total as f64) * 100.0).round() as u64;
                quota.SetQuotaUsedLabel(&HSTRING::from(format!(
                    "{} used of {} ({}%)",
                    human_bytes(used),
                    human_bytes(total),
                    pct
                )))?;
                ui.SetQuotaUI(&quota)?;
            }
        }

        // ── Primary command: "Free up space" (the textual button) ──
        let free_up: IStorageProviderUICommand = FreeUpSpaceCommand.into();
        ui.SetProviderPrimaryCommand(&free_up)?;

        Ok(ui)
    }

    fn StatusUIChanged(
        &self,
        _handler: Option<
            &windows::Foundation::TypedEventHandler<
                IStorageProviderStatusUISource,
                windows::core::IInspectable,
            >,
        >,
    ) -> WinResult<windows::Foundation::EventRegistrationToken> {
        // We do not push change notifications; Windows re-polls `GetStatusUI`
        // when the flyout is (re)opened, which reads the live snapshot. Return a
        // zero token (no subscription) — this is a valid "registered nothing"
        // response and avoids holding a handler ref across threads.
        Ok(windows::Foundation::EventRegistrationToken::default())
    }

    fn RemoveStatusUIChanged(
        &self,
        _token: &windows::Foundation::EventRegistrationToken,
    ) -> WinResult<()> {
        Ok(())
    }
}

// ── The factory Windows asks for the source, keyed by sync-root id ──────────
#[implement(IStorageProviderStatusUISourceFactory)]
struct StatusUiSourceFactory;

impl IStorageProviderStatusUISourceFactory_Impl for StatusUiSourceFactory_Impl {
    fn GetStatusUISource(
        &self,
        _syncrootid: &HSTRING,
    ) -> WinResult<IStorageProviderStatusUISource> {
        // Single-account today: every sync-root id maps to the one source that
        // reads the shared snapshot. (A future multi-account build would branch
        // on `_syncrootid` here, mirroring CloudMirror's note.)
        Ok(StatusUiSource.into())
    }
}

// ── IClassFactory so `CoRegisterClassObject` can hand out the WinRT factory ──
//
// `CreateInstance` returns our `IStorageProviderStatusUISourceFactory`
// (Queried to the requested `riid`). We ignore aggregation (`punkouter`), as the
// shell never aggregates these.
#[implement(IClassFactory)]
struct StatusUiClassFactory;

impl IClassFactory_Impl for StatusUiClassFactory_Impl {
    fn CreateInstance(
        &self,
        punkouter: Option<&windows::core::IUnknown>,
        riid: *const GUID,
        ppvobject: *mut *mut core::ffi::c_void,
    ) -> WinResult<()> {
        use windows::Win32::Foundation::{CLASS_E_NOAGGREGATION, E_POINTER};
        if ppvobject.is_null() {
            return Err(E_POINTER.into());
        }
        // SAFETY: the shell passes a valid out-pointer; null it first per COM
        // contract so a failure leaves no dangling interface pointer.
        unsafe { *ppvobject = std::ptr::null_mut() };
        if punkouter.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let factory: IStorageProviderStatusUISourceFactory = StatusUiSourceFactory.into();
        // SAFETY: `riid`/`ppvobject` come from the shell's QueryInterface-style
        // call; `query` writes the requested interface (or an error) into
        // `ppvobject` and returns the HRESULT, which `.ok()` maps to Result.
        unsafe { factory.query(riid, ppvobject).ok() }
    }

    fn LockServer(&self, _flock: windows::Win32::Foundation::BOOL) -> WinResult<()> {
        // We are an in-process resident server for the lifetime of the engine;
        // no separate lock count needed.
        Ok(())
    }
}

/// COM registration cookie from `CoRegisterClassObject`, kept so
/// [`unregister_status_ui_source`] can `CoRevokeClassObject` it. Set once per
/// process; a re-login reuses the existing registration.
static CLASS_OBJECT_COOKIE: OnceLock<u32> = OnceLock::new();

/// Register the Beebeeb status-UI source factory so the Explorer breadcrumb
/// flyout for our sync root shows synced state + storage + "Free up space".
///
/// Three steps (see module docs): `CoRegisterClassObject`, write the CLSID's
/// `LocalServer32`, and bind the CLSID to the sync root via the
/// `StorageProviderStatusUISourceFactory` value under the SyncRootManager key.
///
/// PURELY ADDITIVE + best-effort. Idempotent: the `CoRegisterClassObject` is
/// guarded by `CLASS_OBJECT_COOKIE` (once per process); the registry writes are
/// create-or-overwrite. `shell_id` is the same `Beebeeb!<hash>` id
/// [`super::shell_sync_root_id`] mints, so the binding lands under the exact key
/// `register_shell_sync_root` created.
///
/// `com_initialized_sta` MUST be true: `CoRegisterClassObject` requires the
/// calling thread to have initialized COM. The caller
/// ([`super::register_status_ui_for_sync_root`]) handles that.
pub fn register_status_ui_source(shell_id: &str) -> anyhow::Result<()> {
    // 1. Register the class object so this running exe is the local server for
    //    the CLSID. Once per process (Windows rejects a duplicate).
    if CLASS_OBJECT_COOKIE.get().is_none() {
        let factory: IClassFactory = StatusUiClassFactory.into();
        // SAFETY: `STATUS_UI_FACTORY_CLSID` is a valid CLSID; `factory` outlives
        // the call (it is moved into COM's keeping via the cookie). We pass
        // `CLSCTX_LOCAL_SERVER | REGCLS_MULTIPLEUSE` exactly as CloudMirror does.
        let cookie = unsafe {
            CoRegisterClassObject(
                &STATUS_UI_FACTORY_CLSID,
                &factory,
                CLSCTX_LOCAL_SERVER,
                REGCLS_MULTIPLEUSE,
            )
        }
        .map_err(|e| anyhow::anyhow!("CoRegisterClassObject(status-ui factory) failed: {e}"))?;
        let _ = CLASS_OBJECT_COOKIE.set(cookie);
        tracing::info!(cookie, "status-UI source factory class object registered");
    }

    // 2. Write HKCU\Software\Classes\CLSID\{clsid}\LocalServer32 = "<exe>" so the
    //    shell can launch us on demand if we are not already running.
    let clsid_str = format!("{{{:?}}}", STATUS_UI_FACTORY_CLSID);
    let exe = std::env::current_exe()
        .map_err(|e| anyhow::anyhow!("resolve current exe for LocalServer32: {e}"))?;
    let exe_str = exe.to_string_lossy().into_owned();
    let local_server_key = format!("Software\\Classes\\CLSID\\{clsid_str}\\LocalServer32");
    write_string_value(HKEY_CURRENT_USER, &local_server_key, None, &exe_str)
        .map_err(|e| anyhow::anyhow!("write CLSID LocalServer32: {e}"))?;

    // 3. Bind the CLSID to the sync root: set
    //    StorageProviderStatusUISourceFactory = {clsid} under the SyncRootManager
    //    key the WinRT Register created. This is what tells the shell to ask our
    //    factory for THIS sync root's flyout.
    let sync_root_key = format!(
        "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Explorer\\SyncRootManager\\{shell_id}"
    );
    write_string_value(
        HKEY_CURRENT_USER,
        &sync_root_key,
        Some("StorageProviderStatusUISourceFactory"),
        &clsid_str,
    )
    .map_err(|e| anyhow::anyhow!("bind StatusUISourceFactory under sync-root key: {e}"))?;

    tracing::info!(
        shell_id,
        clsid = %clsid_str,
        "Explorer status-flyout source registered for sync root"
    );
    Ok(())
}

/// Remove the status-UI registration (inverse of [`register_status_ui_source`]).
///
/// Revokes the COM class object (if registered this process) and deletes the
/// CLSID + the sync-root binding value. Best-effort: every step is independent
/// and a failure is logged, never propagated past the caller's log-and-continue.
pub fn unregister_status_ui_source(shell_id: &str) {
    if let Some(&cookie) = CLASS_OBJECT_COOKIE.get() {
        // SAFETY: `cookie` is the value returned by our own CoRegisterClassObject.
        if let Err(e) = unsafe { CoRevokeClassObject(cookie) } {
            tracing::warn!(error = %e, "CoRevokeClassObject(status-ui factory) failed");
        }
    }

    // Remove the binding under the sync-root key (leave the rest of the key,
    // which `unregister_shell_sync_root` owns).
    let sync_root_key = format!(
        "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Explorer\\SyncRootManager\\{shell_id}"
    );
    if let Err(e) = delete_value(HKEY_CURRENT_USER, &sync_root_key, w!("StorageProviderStatusUISourceFactory")) {
        tracing::debug!(error = %e, "remove StatusUISourceFactory binding (may already be gone)");
    }

    // Remove the CLSID subtree.
    let clsid_str = format!("{{{:?}}}", STATUS_UI_FACTORY_CLSID);
    let clsid_key = HSTRING::from(format!("Software\\Classes\\CLSID\\{clsid_str}"));
    // SAFETY: deletes our own CLSID subtree under HKCU.
    let _ = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &clsid_key) };

    tracing::info!(shell_id, "Explorer status-flyout source unregistered");
}

/// Create-or-open `subkey` under `root` and set a REG_SZ value (`value_name`
/// `None` ⇒ the key's default value) to `data`. Closes the key before returning.
fn write_string_value(
    root: HKEY,
    subkey: &str,
    value_name: Option<&str>,
    data: &str,
) -> anyhow::Result<()> {
    let subkey_h = HSTRING::from(subkey);
    let mut hkey = HKEY::default();
    // SAFETY: out-params are stack locals; `subkey_h` outlives the call.
    let rc = unsafe {
        RegCreateKeyExW(
            root,
            &subkey_h,
            0,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut hkey,
            None,
        )
    };
    if rc != ERROR_SUCCESS {
        return Err(anyhow::anyhow!("RegCreateKeyExW({subkey}) failed: {:?}", rc));
    }

    // REG_SZ data is a NUL-terminated wide string; pass the bytes including the
    // trailing NUL.
    let wide: Vec<u16> = data.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes: &[u8] =
        unsafe { std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2) };
    let name_h = value_name.map(HSTRING::from);
    let name_pcwstr = name_h.as_ref().map_or(PCWSTR::null(), |h| PCWSTR(h.as_ptr()));
    // SAFETY: `hkey` is open; `bytes` outlives the call; `name_pcwstr` is valid
    // for the call's duration (HSTRING kept alive in `name_h`).
    let set_rc = unsafe { RegSetValueExW(hkey, name_pcwstr, 0, REG_SZ, Some(bytes)) };
    // SAFETY: close the key we opened regardless of the set result.
    let _ = unsafe { RegCloseKey(hkey) };
    if set_rc != ERROR_SUCCESS {
        return Err(anyhow::anyhow!("RegSetValueExW failed: {:?}", set_rc));
    }
    Ok(())
}

/// Delete a single named value under `root\subkey` (used to remove the
/// `StorageProviderStatusUISourceFactory` binding on sign-out). `RegDeleteKeyValueW`
/// opens, deletes the value, and closes in one call — no separate open needed.
fn delete_value(root: HKEY, subkey: &str, value_name: PCWSTR) -> anyhow::Result<()> {
    use windows::Win32::System::Registry::RegDeleteKeyValueW;
    let subkey_h = HSTRING::from(subkey);
    // SAFETY: `subkey_h`/`value_name` outlive the call.
    let rc = unsafe { RegDeleteKeyValueW(root, &subkey_h, value_name) };
    if rc != ERROR_SUCCESS {
        return Err(anyhow::anyhow!("RegDeleteKeyValueW failed: {:?}", rc));
    }
    Ok(())
}
