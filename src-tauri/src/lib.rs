use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use sysinfo::{CpuRefreshKind, ProcessRefreshKind, ProcessesToUpdate, System};
use tauri::{
    Emitter, Manager, State,
    menu::{AboutMetadata, CheckMenuItemBuilder, Menu, MenuItem, MenuItemBuilder, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIconEvent},
};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_autostart::ManagerExt as AutostartManagerExt;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use tracing_subscriber::EnvFilter;

mod account;
mod account_binding;
mod account_dto;
mod api_client;
mod browser_login;
#[cfg(any(target_os = "windows", test))]
mod callback_gate;
mod config;
mod conflict;
mod desktop_capabilities;
mod desktop_search;
mod diagnostic_redaction;
mod direct_download;
mod download_pin;
mod engine_bridge;
mod engine_status;
mod owner_keys;
mod shared_folder_index;
mod finder_removal;
// Task 1885: how the bridge's answer to "open this in Finder" becomes a command's result, and the pins that keep
// the macOS code from spawning `open` again. Only macOS calls it.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod finder_open;
// Spec 2026-10-06 (macOS Finder setup reconciler). Only macOS runs the reconciler; on
// Windows/Linux these types exist for `AppState` and the commands.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod finder_setup;
// Spec 2026-10-06 §8, the lifecycle log. Only macOS writes it (the same lifetime as
// `finder_setup`).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod lifecycle_log;
#[cfg(test)]
#[path = "../tests/support/bridge.rs"]
mod native_parity_tests;
// Unix-domain-socket IPC (macOS File Provider extension + Linux FUSE). Unix
// sockets don't exist on Windows; the Windows Cloud Files callback runs
// in-process (see `windows_cf`), so this module is `unix`-only.
#[cfg(unix)]
mod ipc_frame;
#[cfg(unix)]
mod ipc_socket;
// Idempotency table for Finder write-queue requests (task 1684).
#[cfg(unix)]
mod ipc_write_dedup;
mod keychain;
// Known-folder backup ("Manage backup", task 0797 / Model 2). The catalog +
// pure copy-diff classifier compile everywhere (unit-tested cross-platform);
// the `SHGetKnownFolderPath` resolver + the source→vault mirror loop are
// Windows-only (gated inside the module). The runner calls
// `mirror_enabled_known_folders` on Windows; non-Windows builds only use the
// catalog for the IPC source-path listing.
mod known_folder;
#[cfg(target_os = "linux")]
mod linux_fuse;
#[cfg(target_os = "linux")]
mod linux_thumbnail;
#[cfg(all(test, not(target_os = "linux")))]
mod linux_fuse {
    #[path = "inode_map.rs"]
    pub mod inode_map;
}
mod link_health;
mod lockfile;
#[cfg(target_os = "macos")]
mod macos_file_provider;
// Task 1885: NSWorkspace for the URLs that are not Finder items (System Settings), with no child `open`.
#[cfg(target_os = "macos")]
mod macos_workspace;
// Task 2267: a Windows start refused over unowned local data proves the data is the signed-in account's own before it
// is adopted (Windows never resets; macOS and Linux reset instead, so they never need it).
#[cfg(any(target_os = "windows", test))]
mod own_data_proof;
// Task 2267 (fix round 2): what the person decides about the changes the proof held for review.
#[cfg(any(target_os = "windows", test))]
mod found_while_paused;
// Tasks 1888 + 2226: the reconcile pass over the queue (ghost folder creates folded into their server twins), planned in
// memory. Windows and Linux run it; macOS keeps its own resume.
mod held_queue;
mod popover_data;
mod macos_popover;
// Ruling R8 (spec 2026-10-06): the pure decision between a first sign-in, the same account signing in again and
// an account switch. Windows keeps its refusal while a session exists and never asks.
#[cfg_attr(target_os = "windows", allow(dead_code))]
mod reauth;
mod runner;
mod sign_out;
mod staged_payload;
mod state_db;
mod state_paths;
// 0.8.15: the read Settings › Sync and the Windows tray poll to say what sync is doing (counts only).
mod sync_activity;
mod transfer_progress;
// 0.8.15: WebView2's browser shortcuts (print, reload, find, ...) are off in every window; no-op off Windows.
mod webview_browser_keys;
mod write_token;
// Task 1683 slice 1: pure macOS-popover surface logic, compiled and tested on every
// platform. Slices 2-6 wire the rest of it; until then only `policy` has callers,
// so dead-code is allowed for the module (remove the allow when slice 6 lands).
#[allow(dead_code)]
mod surfaces;
// Per-process scratch dirs so unit tests never touch the person's real config or cache.
#[cfg(test)]
mod test_sandbox;
// Layout-independent comparison of source text, for the tests that pin the code by reading it.
#[cfg(test)]
mod source_pin;
// The File Provider extension logs through its one logger and makes only closed values public.
#[cfg(test)]
mod extension_log_pin;
// Sync-root filesystem watcher — the local-create UPLOAD trigger (task 0780).
// Primarily for Windows, where there is no OS extension / IPC socket to fire
// `QueueFinderCreate`; the macOS File Provider extension drives that over the
// Unix socket instead, and the Linux FUSE prototype does not use the socket at
// all (it calls the in-process `EngineBridge` directly). Compiled everywhere (cheap, cross-platform via
// `notify`) but the runner only spawns it on Windows (see `runner::run`).
mod watcher;
// Task 2191: the guard for Windows files of unknown content (synced before task 1640). Its
// decisions are cross-platform (tested everywhere); the runner calls it on Windows only.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
mod unknown_check;
// Task 2240 slice 2: the placeholder reconcile decided from folder listings. Cross-platform so it is
// tested everywhere; only the Windows tick calls it.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
mod os_state_reconcile;
// Tasks 2270 / 2280: what to do with an upload finalization journal row. Decided on every platform (tested
// everywhere); applied on Windows only (`windows_cf::upload_finalization`).
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
mod upload_finalization_plan;
// Task 2284: the no-fetch scope a finalization, stamp or promote holds around its data open.
mod no_fetch_scope;
// Windows Cloud Files API — Phase 2 Task 6. Gated to Windows only;
// the module's own files start with `#![cfg(target_os = "windows")]`
// so this `mod` declaration plus the conditional are belt-and-braces.
#[cfg(target_os = "windows")]
mod windows_cf;
#[cfg(target_os = "windows")]
static SESSION_TRANSITION: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
#[cfg(any(target_os = "windows", test))]
mod session_commands;
#[cfg(target_os = "windows")]
static SESSION_COMMANDS: LazyLock<session_commands::SessionCommands> =
    LazyLock::new(session_commands::SessionCommands::new);

#[cfg(any(target_os = "windows", test))]
mod auth_attempts;
#[cfg(target_os = "windows")]
static AUTH_ATTEMPTS: LazyLock<auth_attempts::AuthAttempts> = LazyLock::new(auth_attempts::AuthAttempts::new);

// Windows only: cancel the entire command, including independently built clients.
// Other platforms poll the original command body directly.
macro_rules! session_command {
    ($work:expr) => {{
        #[cfg(target_os = "windows")]
        {
            SESSION_COMMANDS.run($work).await
        }
        #[cfg(not(target_os = "windows"))]
        {
            $work.await
        }
    }};
}

/// Work a command starts as a task of its own, which may outlive the command (the popover's storage fetch), is bound to
/// the session when it is created: on Windows a lock cancels it and waits for what it holds to drop, and once the
/// session is closed it never starts (#139 review P1; tests/windows_session_wiring.rs guards the callers).
fn session_bound<T>(
    work: impl std::future::Future<Output = Result<T, String>> + Send + 'static,
) -> impl std::future::Future<Output = Result<T, String>> + Send + 'static
where
    T: Send + 'static,
{
    #[cfg(target_os = "windows")]
    {
        let bound = SESSION_COMMANDS.bind(work);
        async move { bound?.await }
    }
    #[cfg(not(target_os = "windows"))]
    {
        work
    }
}

#[cfg(target_os = "windows")]
async fn close_session_commands() -> Result<(), String> {
    let revoked = SESSION_COMMANDS.close();
    let auth_revoked = AUTH_ATTEMPTS.close();
    let worker = tokio::task::spawn_blocking(move || {
        if let Some(auth) = auth_revoked {
            auth.drain(std::time::Duration::from_secs(3))?;
        }
        SESSION_COMMANDS.drain(revoked, std::time::Duration::from_secs(3))
    });
    tokio::time::timeout(std::time::Duration::from_secs(4), worker)
        .await
        .map_err(|_| {
            "Vault lock failed: commands are still stopping. The vault is not locked. Retry locking.".to_string()
        })?
        .map_err(|e| format!("Vault lock failed: {e}"))?
}

use config::{DesktopConfig, HeldConfig, ReleaseChannel};
// `platform_keychain_store_for(id)` resolves to the macOS Keychain store on
// macOS and the Windows Credential Manager store on Windows (Linux keeps the
// fail-closed stub), segmenting every secret under the per-account id
// (`io.beebeeb.app/<id>/<leaf>`). `legacy_platform_keychain_store()` is the
// un-segmented pre-Phase-1 layout, used read-only by the migration + the
// anti-lockout legacy fallback. All session/vault persistence below routes
// through these so secrets land in the OS-native credential vault.
use keychain::{AuthVault, SecretBytes, SessionToken, platform_keychain_store_for};
use runner::EngineRunner;

use account::{AccountId, AccountRuntime, synthesize_single_account};

// ── Session bridge (web ↔ rust) ───────────────────────────────────────────────

/// Authenticated session pushed in from the WebView after login.
///
/// `master_key` is the user's 32-byte vault root key, derived from the
/// OPAQUE export_key (or, for legacy accounts, from Argon2id over the
/// password). It is the input to per-file `derive_file_key()` calls
/// inside `beebeeb-sync` — the engine cannot do anything without it.
///
/// The struct is intentionally NOT `Clone`: there should be exactly one
/// authoritative copy of the master key in this process at any moment.
/// `take()` removes it from the registry on logout; everything else
/// reads it through a borrow under the `Mutex`.
///
/// `ZeroizeOnDrop` wipes the secret bytes when the `Session` is dropped — on
/// `lock_vault` / `clear_session` (both `take()` the `Option<Session>`, dropping
/// it) and on process teardown — so the master key and the session token never
/// linger in freed memory. All three fields are `Zeroize`: `[u8; 32]`,
/// `String` (token), and `Option<String>` (email) each implement it in
/// `zeroize` 1.x, so no field needs `#[zeroize(skip)]`.
#[derive(zeroize::ZeroizeOnDrop)]
pub struct Session {
    pub token: String,
    pub master_key: [u8; 32],
    pub email: Option<String>,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("token", &"<redacted>")
            .field("master_key", &"<redacted>")
            .field("email", &self.email.as_deref().map(|_| "<set>").unwrap_or("<unset>"))
            .finish()
    }
}

/// A login that proved the password (OPAQUE) but still needs a second factor.
///
/// Held in [`AppState::pending_2fa`] between `desktop_login` (which receives the
/// server's `requires_2fa` + `partial_token`) and `desktop_login_2fa` (which
/// carries the partial token plus the user's TOTP code to `/auth/2fa/verify` to
/// mint the real session). The `partial_token` is a server-issued, short-lived
/// (≈5-minute) partial session token — it is NOT key material and is validated
/// server-side, so it is never logged. `email` is the lowercased address from
/// `desktop_login`, reused for the same post-login setup the success path runs.
///
/// `ZeroizeOnDrop` wipes the partial token (a live, short-lived server
/// credential) from freed heap whenever `pending_2fa` is replaced or cleared —
/// mirroring how `Session` zeroizes its token. Both `String` fields implement
/// `Zeroize` in `zeroize` 1.x, so the derive applies without `#[zeroize(skip)]`.
#[derive(zeroize::ZeroizeOnDrop)]
struct Pending2fa {
    partial_token: String,
    email: String,
}

/// Tauri-managed shared state. Held behind a `Mutex` because the web
/// thread (IPC handlers) and the sync-engine task both need access.
/// Cheap to lock — the registry and these flags change rarely.
///
/// Phase 0 multi-account refactor (decision 0800): the per-account live
/// state (`session`, `engine`, `engine_state`, `sync_paused`,
/// `cached_profile`, `auth_email`) moved off `AppState` onto
/// [`AccountRuntime`]; handlers reach it through [`AppState::active_account`].
/// At N=1 this is invisible — exactly one account is synthesized at startup.
/// What stays on `AppState` is the genuinely process-global / login-in-flight
/// state: `auth_present` (a process-wide "is anyone signed in" seed) and
/// `pending_2fa` (a login challenge that has no account yet).
pub struct AppState {
    /// The account registry. Exactly one entry in Phase 0 (synthesized by
    /// `synthesize_single_account` at startup). `Arc<AccountRuntime>` so
    /// `active_account()` can hand out a shared handle that callers lock,
    /// without ever cloning the `!Clone` `Session` inside it.
    pub accounts: Mutex<Vec<Arc<AccountRuntime>>>,
    /// Id of the currently-active account. `active_account()` resolves this,
    /// falling back to the first registered account.
    pub active_account_id: Mutex<Option<AccountId>>,
    /// Process-wide "is a session present in the credential store" seed,
    /// set from `keychain_session_present()` at startup and on login/logout.
    /// Stays on `AppState` (not per-account) — it gates onboarding routing
    /// before any account is unlocked.
    pub auth_present: Mutex<bool>,
    /// A password-verified login awaiting its TOTP second factor. Set by
    /// `desktop_login` when the OPAQUE finish returns `requires_2fa: true`;
    /// consumed (and cleared on success) by `desktop_login_2fa`. `None`
    /// whenever no 2FA challenge is in flight. Stays on `AppState`: during the
    /// challenge there is no account yet to attach it to.
    pending_2fa: Mutex<Option<Pending2fa>>,
    /// Last native-menu zoom scale applied to WebViews. Tauri exposes
    /// `set_zoom` but not a readback API, so the menu owns this small bit of
    /// process state.
    pub menu_zoom_scale: Mutex<f64>,
    /// The macOS Finder reconciler (spec 2026-10-06). Set once in `setup()` on macOS; empty on
    /// Windows/Linux and in tests that install none.
    pub finder_setup: std::sync::OnceLock<finder_setup::driver::FinderSetupHandle>,
    /// The sign-out flow's state (spec 2026-10-10 §5.1): the newest ask snapshot, which the gate compares with.
    pub(crate) sign_out: sign_out::SignOutState,
    /// The set-up step a password sign-in without a key left open, for the life of the process (review P2-3 on the Mac
    /// sign-in PR): see [`PendingSetup`].
    pending_setup: Mutex<Option<HeldSetup>>,
    /// The account's storage summary for `popover_snapshot` (task 2179). Here, not on the popover's runtime, so
    /// sign-out clears it with the rest of the account (#139 review P2-3).
    pub(crate) popover_storage: Arc<Mutex<popover_data::StorageCache>>,
}

impl AppState {
    /// Resolve the active account runtime.
    ///
    /// Returns the account whose id matches `active_account_id`, falling back
    /// to the first registered account when the id is unset or stale. Returns
    /// `Err` only when the registry is empty (i.e. before
    /// `synthesize_single_account` has run) — at N=1 it always resolves after
    /// startup.
    ///
    /// Hands out an `Arc<AccountRuntime>`; callers lock the inner mutexes
    /// (`acct.session.lock()`, `acct.engine.lock().await`, …). There is
    /// deliberately NO `active_account_mut()`: interior mutability keeps the
    /// 1:1 locking discipline and never moves the `!Clone` `Session`.
    pub fn active_account(&self) -> Result<Arc<AccountRuntime>, String> {
        let accounts = self
            .accounts
            .lock()
            .map_err(|_| "accounts registry mutex poisoned".to_string())?;
        if accounts.is_empty() {
            return Err("no active account".to_string());
        }
        let active_id = self.active_account_id.lock().ok().and_then(|g| g.clone());
        if let Some(id) = active_id
            && let Some(found) = accounts.iter().find(|a| a.id == id)
        {
            return Ok(found.clone());
        }
        // Fallback: first registered account (the only one in Phase 0).
        Ok(accounts[0].clone())
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            accounts: Mutex::new(Vec::new()),
            active_account_id: Mutex::new(None),
            auth_present: Mutex::new(false),
            pending_2fa: Mutex::new(None),
            menu_zoom_scale: Mutex::new(1.0),
            finder_setup: std::sync::OnceLock::new(),
            sign_out: sign_out::SignOutState::default(),
            pending_setup: Mutex::new(None),
            popover_storage: Arc::new(Mutex::new(popover_data::StorageCache::default())),
        }
    }
}

/// The step a password sign-in on a Mac with no vault key leaves open: “Set up this Mac”, the recovery phrase. The Beebeeb
/// window keeps its sign-in flow in a module store, and ⌘W destroys the window and the store with it, while the session
/// stays and the key was never delivered; a window opened again then showed Home, whose locked card had nothing to unlock
/// from (review P2-3). The window tells Rust when it enters the step (`set_pending_setup`) and asks once when it opens
/// signed in and locked (`pending_setup`). Held for the life of the process: a relaunch is the Keychain signal of task
/// 1747's Part 2.
///
/// It is forgotten the moment keys arrive on this Mac ([`keys_arrived`]: every unlock and every sign-in that puts a session
/// in memory), and it is answered only while it is still true (an account signed in, its vault locked), so a sign-out or
/// any other way the account leaves makes it inert without touching the sign-out code; the next sign-in sets it again or
/// brings keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
struct PendingSetup {
    /// The kept key was replaced (spec §5.6), so the card says why the phrase is asked.
    key_replaced: bool,
}

/// A pending step and the account it was set for. The slot outlives a sign-out (the sign-out code is untouched), so a step
/// of account A would otherwise be answered for account B who signed in after A and is signed in and locked; it is
/// answered only for the account it was set for (review P3-4 of the delta review).
#[derive(Debug, Clone, PartialEq, Eq)]
struct HeldSetup {
    step: PendingSetup,
    account: account_binding::Identity,
}

fn remember_pending_setup(slot: &Mutex<Option<HeldSetup>>, key_replaced: bool, account: account_binding::Identity) {
    *slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(HeldSetup {
        step: PendingSetup { key_replaced },
        account,
    });
}

fn forget_pending_setup(slot: &Mutex<Option<HeldSetup>>) {
    *slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
}

/// The pending step, if it is still true: an account is signed in, its vault is locked, and it is the account the step was
/// set for (`account_binding::same_account`: an identity that cannot be compared resumes nothing). Reading it does not use
/// it up.
fn resume_pending_setup(
    slot: &Mutex<Option<HeldSetup>>,
    signed_in: bool,
    vault_unlocked: bool,
    current: &account_binding::Identity,
) -> Option<PendingSetup> {
    if !signed_in || vault_unlocked {
        return None;
    }
    let held = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()?;
    (account_binding::same_account(&held.account, current) == Some(true)).then_some(held.step)
}

/// Who this Mac's credentials name, for a pending step: the cached profile's user id and email, else the session's or the
/// Keychain's email (`credentials_identity`). Windows has no Beebeeb window to resume, so it names nobody there.
fn pending_setup_identity(acct: &AccountRuntime) -> account_binding::Identity {
    #[cfg(not(target_os = "windows"))]
    {
        credentials_identity(acct)
    }
    #[cfg(target_os = "windows")]
    {
        let _ = acct;
        account_binding::Identity::default()
    }
}

/// The window enters “Set up this Mac”: Rust keeps the step for the process, for the account whose credentials this Mac holds
/// (see [`PendingSetup`]).
#[tauri::command]
fn set_pending_setup(state: State<'_, AppState>, key_replaced: bool) -> Result<(), String> {
    let acct = state.active_account()?;
    remember_pending_setup(&state.pending_setup, key_replaced, pending_setup_identity(&acct));
    Ok(())
}

/// A window that opens signed in and locked asks whether “Set up this Mac” is still owed. `logged_in` is `sync_status`'s own
/// reading: a session in memory, or a token in the credential store.
#[tauri::command]
fn pending_setup(state: State<'_, AppState>) -> Result<Option<PendingSetup>, String> {
    let acct = state.active_account()?;
    let vault_unlocked = acct.session.lock().map(|guard| guard.is_some()).unwrap_or(false);
    let auth_present = state.auth_present.lock().map(|guard| *guard).unwrap_or(false);
    Ok(resume_pending_setup(
        &state.pending_setup,
        vault_unlocked || auth_present,
        vault_unlocked,
        &pending_setup_identity(&acct),
    ))
}

fn keychain_error(context: &str, error: impl fmt::Display) -> String {
    format!("{context}: {error}")
}

/// Drop the cached `/auth/me` profile from the active account.
///
/// Called on logout (`clear_session`) and lock (`lock_vault`) so the
/// `account_profile` IPC can never cache-hit a stale (logged-out / locked)
/// identity. A subsequent login / unlock re-fetches and re-caches. Poisoned
/// mutex is swallowed — clearing must never fail a logout/lock from the
/// user's POV. Takes `&AppState` (not `State<'_, _>`) so it's unit-testable
/// against a synthesized `AppState`. A missing active account (registry not yet
/// synthesized) is also a silent no-op.
fn clear_cached_profile(state: &AppState) {
    if let Ok(acct) = state.active_account()
        && let Ok(mut guard) = acct.cached_profile.lock()
    {
        *guard = None;
    }
}

/// One lock for everything that writes or clears a session (Task 11; Task 12 lead ruling 1): the Keychain items, the
/// session in memory with the flags that mirror it, and the owner record a session writes. Every session transition
/// takes its turn under it: [`claim_session_write`] checks, under this lock, that no other transition of the account
/// happened since the writer's own transition began, and moves the account's [`account::SessionGeneration`] on; Lock and
/// Sign-out move it on under it too ([`end_sessions_in_flight`], [`finish_sign_out_turn`]). Held only across synchronous
/// work, never across an `.await` (clippy's `await_holding_lock` checks it). While a turn writes `state.db` or the
/// Keychain, another transition that needs the lock waits for that I/O (up to the database's busy timeout). A poisoned
/// lock is recovered: it guards no data, only the order.
static SESSION_KEYCHAIN_WRITES: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock_session_keychain_writes() -> std::sync::MutexGuard<'static, ()> {
    SESSION_KEYCHAIN_WRITES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Proof that the holder has the session-write lock for a session transition that is still current. In production only
/// [`claim_session_write`], [`check_session_turn`], [`end_sessions_in_flight`] and [`finish_sign_out_turn`] make one,
/// and every writer of a session (the Keychain items, and the functions that install, replace or name the session in
/// memory) takes one, so none of them can write outside a checked turn. Dropping it ends the turn. The one clear that
/// takes the engine-slot guard instead, [`clear_session_holding_slot`] (Lock and Sign-out), ends every transition in
/// flight first and then clears outside this lock on purpose: `bump_vault_epoch` must never wait under it.
struct SessionWrite {
    _writes: std::sync::MutexGuard<'static, ()>,
}

impl fmt::Debug for SessionWrite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SessionWrite")
    }
}

#[cfg(test)]
impl SessionWrite {
    /// Tests only: the lock without a generation check, to seed the in-memory Keychain the way an earlier sign-in left
    /// it. Used as a temporary, so the lock is released at the end of the statement.
    fn for_test() -> Self {
        Self {
            _writes: lock_session_keychain_writes(),
        }
    }
}

/// What a sign-in says when another session transition (a Sign-out, another sign-in) happened while it worked, so it
/// saved nothing.
const SESSION_CHANGED_WHILE_WAITING: &str =
    "Something changed on this computer while Beebeeb was working; nothing was saved. Try again.";

/// What a sign-in says when a Lock happened while it worked (Task 12 fix round 1, item 8).
const SIGN_IN_LOCKED_MEANWHILE: &str = "Beebeeb was locked while signing in. Sign in again.";

/// What the recovery-phrase unlock says when a Lock happened while the phrase was being checked (a Lock keeps the person
/// signed in, so the phrase is all that is asked again).
const RECOVERY_UNLOCK_LOCKED_MEANWHILE: &str =
    "Beebeeb was locked while the recovery phrase was being checked, so nothing was saved. Enter the phrase once more.";

/// What the Keychain unlock says when a Lock or a Sign-out happened while it unlocked (after a Sign-out there is nothing
/// left to unlock, so the advice is the neutral one; Task 12 fix round 2, item 5).
const KEYCHAIN_UNLOCK_INTERRUPTED: &str = "Beebeeb was signed out or locked while unlocking. Try again.";

/// A transition's turn was refused: another session transition happened since it began. Each path says why in its own
/// words (one distinct sentence per path, so the frontend can tell them apart).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SessionChanged {
    by_lock: bool,
}

impl SessionChanged {
    /// What moved `acct`'s generation last, as a refusal.
    fn of(acct: &AccountRuntime) -> Self {
        Self {
            by_lock: acct.last_session_transition() == account::SessionTransition::Lock,
        }
    }

    fn sign_in_sentence(self) -> &'static str {
        if self.by_lock {
            SIGN_IN_LOCKED_MEANWHILE
        } else {
            SESSION_CHANGED_WHILE_WAITING
        }
    }

    fn recovery_unlock_sentence(self) -> &'static str {
        if self.by_lock {
            RECOVERY_UNLOCK_LOCKED_MEANWHILE
        } else {
            UNLOCK_ACCOUNT_CHANGED
        }
    }

    fn keychain_unlock_sentence(self) -> &'static str {
        KEYCHAIN_UNLOCK_INTERRUPTED
    }
}

/// A sign-in's refusal, by default.
impl From<SessionChanged> for String {
    fn from(changed: SessionChanged) -> Self {
        changed.sign_in_sentence().to_string()
    }
}

/// Take the turn of the session transition that captured `turn` (Task 12, lead ruling 1): the session-write lock and,
/// on macOS and Linux, a refusal when any other transition of `acct` happened since (a Lock or a Sign-out, or another
/// sign-in that wrote first). On success the generation moves on and `turn` follows it, so the same transition's next
/// write passes and every OLDER transition is refused from now on. Windows orders its transitions with
/// `SESSION_TRANSITION` and its attempt fences and never refuses here; its generation still moves.
fn claim_session_write(
    acct: &AccountRuntime,
    turn: &mut account::SessionGeneration,
) -> Result<SessionWrite, SessionChanged> {
    let write = check_session_turn(acct, turn)?;
    *turn = acct.advance_session_generation_holding_session_write_lock(account::SessionTransition::Write);
    Ok(write)
}

/// The same check as [`claim_session_write`] without moving the generation on: for a write that changes nothing a
/// sign-in decides on (a cached profile), or that moves it only when it turns out to change something (the naming of a
/// session, Task 12 fix round 1 item 5). The caller moves it on, holding the returned turn, when it does.
fn check_session_turn(
    acct: &AccountRuntime,
    turn: &account::SessionGeneration,
) -> Result<SessionWrite, SessionChanged> {
    let writes = lock_session_keychain_writes();
    #[cfg(not(target_os = "windows"))]
    if !acct.session_unchanged_since(*turn) {
        tracing::info!("a session transition wrote nothing: another one happened since it began");
        return Err(SessionChanged::of(acct));
    }
    #[cfg(target_os = "windows")]
    let _ = turn;
    Ok(SessionWrite { _writes: writes })
}

/// Lock and Sign-out (`by`): every session transition that began before this call is refused from now on (it can no
/// longer pass [`claim_session_write`]). Always succeeds. The caller clears under the returned turn, or drops it at once.
fn end_sessions_in_flight(acct: &AccountRuntime, by: account::SessionTransition) -> SessionWrite {
    let writes = lock_session_keychain_writes();
    acct.advance_session_generation_holding_session_write_lock(by);
    SessionWrite { _writes: writes }
}

/// Sign-out's last turn on macOS and Linux (Task 12 fix round 1, item 2): under the session-write lock, clear the
/// Keychain (both layouts), check that nothing of the account is left, clear memory once more (an unlock that began after
/// the sign-out's earlier memory clear may have put a session there), and only THEN move the generation on, before the
/// lock is released. So a transition that began before this point is refused at its next check, and one that begins
/// after it finds nothing to unlock. The caller still holds the engine slot (`_engine_slot`), so no start can read keys
/// in between. `best_effort`: the already-signed-out path, where a store that cannot answer is not a failure. The
/// generation moves whatever the result. `dropped` gets every session token this turn took (task 1886): what each
/// Keychain layout held before its clear, and a session put in memory while the sign-out ran, also when the turn fails.
#[cfg(not(target_os = "windows"))]
fn finish_sign_out_turn(
    acct: &AccountRuntime,
    _engine_slot: &tokio::sync::MutexGuard<'_, Option<EngineRunner>>,
    best_effort: bool,
    dropped: &mut DroppedTokens,
) -> Result<(), String> {
    let (result, cleared_keys) = {
        let write = SessionWrite {
            _writes: lock_session_keychain_writes(),
        };
        let result = match clear_keychain_session_holding(&write, acct.id.as_str(), dropped) {
            Err(error) if best_effort => {
                tracing::warn!(%error, "already-signed-out sign-out: keychain session clear failed (nothing should be left); continuing");
                Ok(())
            }
            cleared => cleared,
        };
        let result = result.and_then(|()| ensure_keychain_holds_no_account(acct.id.as_str()));
        let taken = match acct.session.lock() {
            Ok(mut guard) => guard.take(),
            Err(poisoned) => {
                let taken = poisoned.into_inner().take();
                acct.session.clear_poison();
                taken
            }
        };
        let cleared_keys = taken.is_some();
        dropped.add(taken.map(|mut session| zeroize::Zeroizing::new(std::mem::take(&mut session.token))));
        acct.advance_session_generation_holding_session_write_lock(account::SessionTransition::SignOut);
        drop(write);
        (result, cleared_keys)
    };
    if cleared_keys {
        tracing::warn!("sign-out cleared a session that was put in memory while it ran");
        bump_vault_epoch();
    }
    result
}

/// Store a new session's token and vault key, with the email that goes with them (see
/// [`forget_account_email_in_keychain`]: the email kept for the session before is removed first). Only inside a checked
/// turn (`_write`).
fn persist_session_to_keychain(
    _write: &SessionWrite,
    account_id: &str,
    token: &str,
    master_key: &[u8; 32],
    email: Option<&str>,
) -> Result<(), String> {
    let vault = AuthVault::new(platform_keychain_store_for(account_id));
    let token = SessionToken::new(token.to_string()).map_err(|e| keychain_error("session token", e))?;
    forget_account_email_in_keychain(account_id)?;
    vault
        .install_session(token)
        .map_err(|e| keychain_error("store session in Keychain", e))?;
    vault
        .store_wrapped_master_key(SecretBytes::new_master_key(master_key))
        .map_err(|e| keychain_error("store vault key in Keychain", e))?;
    // The session's own email, after its secrets: the Account page shows it after an auto-unlock on relaunch, and
    // the restore knows the stored session by it. None/empty (a sign-in that could not fetch the account record)
    // writes nothing, so the stored session has no email and stays unidentified until a fetch names it.
    persist_account_email_to_keychain(&vault, email)
}

/// Store a new session's token (its vault key comes later, from the recovery phrase), with the email that goes with
/// it (see [`forget_account_email_in_keychain`]). Only inside a checked turn (`_write`).
fn persist_session_token_to_keychain(
    _write: &SessionWrite,
    account_id: &str,
    token: &str,
    email: Option<&str>,
) -> Result<(), String> {
    let vault = AuthVault::new(platform_keychain_store_for(account_id));
    let token = SessionToken::new(token.to_string()).map_err(|e| keychain_error("session token", e))?;
    forget_account_email_in_keychain(account_id)?;
    vault
        .install_session(token)
        .map_err(|e| keychain_error("store session in Keychain", e))?;
    persist_account_email_to_keychain(&vault, email)
}

/// Remove the account email the Keychain kept for the session stored before. Every write of a new session token
/// calls this BEFORE it stores the token, and writes the new session's own email (if it has one) after: the restore
/// knows a stored session by that email, so the Keychain must never hold one account's token or key next to another
/// account's email, not even when a later step of the write fails. A removal that fails fails the write, as a token
/// that cannot be stored does; an email that is not there is no failure.
fn forget_account_email_in_keychain(account_id: &str) -> Result<(), String> {
    use keychain::AuthSecretStore as _;
    platform_keychain_store_for(account_id)
        .delete_account_email()
        .map_err(|e| keychain_error("remove the previous account email from Keychain", e))
}

/// Store the account email in the credential vault if one is provided.
///
/// Always returns `Ok(())`: a store that can't hold the email must never fail an otherwise-successful login, so a
/// persist error is logged and swallowed rather than surfaced. That is safe because the writers of a new session
/// remove the previous email first ([`forget_account_email_in_keychain`]): a failed write leaves NO email, and the
/// restore then treats the session as unidentified (no engine until a fetch names it) rather than as another
/// account. The email shows on the Account page after an auto-unlock and is what the restore knows the session by.
/// `None`/empty writes nothing.
fn persist_account_email_to_keychain(
    vault: &AuthVault<keychain::PlatformKeychainStore>,
    email: Option<&str>,
) -> Result<(), String> {
    let Some(email) = email else {
        return Ok(());
    };
    if let Err(e) = vault.store_account_email(email) {
        tracing::warn!(error = %e, "could not persist account email to credential store (non-fatal)");
    }
    Ok(())
}

/// Store the vault key the recovery phrase proved. Only inside a checked turn (`_write`). The key is borrowed: no copy of
/// it is made here but the one the store wipes (`SecretBytes`).
fn persist_vault_key_to_keychain(_write: &SessionWrite, account_id: &str, master_key: &[u8; 32]) -> Result<(), String> {
    AuthVault::new(platform_keychain_store_for(account_id))
        .store_wrapped_master_key(SecretBytes::new_master_key(master_key))
        .map_err(|e| keychain_error("store vault key in Keychain", e))
}

fn load_session_token_from_keychain(account_id: &str) -> Result<Option<String>, String> {
    let id_keyed = AuthVault::new(platform_keychain_store_for(account_id))
        .session_token()
        .map_err(|e| keychain_error("load session from Keychain", e))?;
    if let Some(token) = id_keyed {
        return Ok(Some(token.expose_for_request().to_string()));
    }
    // LEGACY READ FALLBACK (anti-lockout): the id-keyed lookup found nothing, so
    // fall back to the legacy flat store before concluding "no session." This
    // makes auto-unlock robust even if the migration no-op'd or failed.
    let legacy = AuthVault::new(keychain::legacy_platform_keychain_store())
        .session_token()
        .map_err(|e| keychain_error("load legacy session from Keychain", e))?;
    Ok(legacy.map(|t| t.expose_for_request().to_string()))
}

fn load_session_from_keychain(account_id: &str, email: Option<String>) -> Result<Option<Session>, String> {
    // Try the id-segmented store first; if it has no session token, fall back to
    // the legacy flat store (anti-lockout READ FALLBACK §4D). The fallback is
    // read-only — it never writes — so it can never re-create orphan-able flat
    // entries; it only keeps a not-yet-migrated install auto-unlocking.
    let vault = AuthVault::new(platform_keychain_store_for(account_id));
    if vault
        .session_token()
        .map_err(|e| keychain_error("load session from Keychain", e))?
        .is_some()
    {
        return load_session_from_vault(vault, email);
    }
    // Id-keyed lookup empty → legacy fallback.
    let legacy = AuthVault::new(keychain::legacy_platform_keychain_store());
    if legacy
        .session_token()
        .map_err(|e| keychain_error("load legacy session from Keychain", e))?
        .is_some()
    {
        return load_session_from_vault(legacy, email);
    }
    Ok(None)
}

/// Shared tail of `load_session_from_keychain` for whichever store (id-keyed or
/// legacy) actually holds the session. Loads the token, unlocks the vault
/// (validating the 32-byte key), and recovers the email. Returns `Ok(None)` only
/// if the token vanished between the presence check and here (a race the caller
/// already filtered out); any unlock failure (incl. genuine `NotFound`) is an
/// `Err` so the startup restore can distinguish "no key on this PC" from success.
fn load_session_from_vault<S: keychain::AuthSecretStore>(
    mut vault: AuthVault<S>,
    email: Option<String>,
) -> Result<Option<Session>, String> {
    let Some(token) = vault
        .session_token()
        .map_err(|e| keychain_error("load session from Keychain", e))?
    else {
        return Ok(None);
    };
    vault
        .unlock()
        .map_err(|e| keychain_error("unlock vault key from Keychain", e))?;
    // Wiped when this function returns (Task 12 fix round 1, item 10): the one copy that outlives it is the session's.
    let mut master_key = zeroize::Zeroizing::new([0u8; 32]);
    master_key.copy_from_slice(vault.master_key().map_err(|e| keychain_error("read vault key", e))?);
    // Prefer an email already known in memory (e.g. carried through from a live
    // unlock). Otherwise recover the one persisted in the credential vault so the
    // Account page can show it after an auto-unlock on relaunch. A session stored
    // before email persistence existed simply yields `None` here (no error) —
    // backward-compatible by construction.
    let email = match email {
        Some(email) => Some(email),
        None => vault
            .account_email()
            .map_err(|e| keychain_error("read account email", e))?,
    };
    Ok(Some(Session {
        token: token.expose_for_request().to_string(),
        master_key: *master_key,
        email,
    }))
}

fn keychain_session_present(account_id: &str) -> bool {
    let id_keyed = AuthVault::new(platform_keychain_store_for(account_id))
        .session_token()
        .map(|token| token.is_some())
        .unwrap_or(false);
    if id_keyed {
        return true;
    }
    // LEGACY READ FALLBACK (anti-lockout): report present if a legacy flat
    // session still exists (e.g. migration hasn't run yet), so onboarding routing
    // sees the signed-in state rather than falling back to the sign-in prompt.
    AuthVault::new(keychain::legacy_platform_keychain_store())
        .session_token()
        .map(|token| token.is_some())
        .unwrap_or(false)
}

// Windows' startup discard; on macOS and Linux the sign-out clears inside its last turn
// (`clear_keychain_session_holding`), and only tests call this.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn clear_keychain_session(account_id: &str) -> Result<(), String> {
    clear_keychain_session_into(account_id, &mut DroppedTokens::default())
}

/// [`clear_keychain_session`] that hands every session token it clears to `dropped` (Windows' sign-out, task 1886).
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn clear_keychain_session_into(account_id: &str, dropped: &mut DroppedTokens) -> Result<(), String> {
    let writes = SessionWrite {
        _writes: lock_session_keychain_writes(),
    };
    clear_keychain_session_holding(&writes, account_id, dropped)
}

/// [`clear_keychain_session`] inside a turn the caller already holds (the sign-out's last turn). Task 1886: `dropped`
/// gets the token each layout holds, read in this same turn just before that layout's clear, so a sign-out ends on the
/// server what it removes here (also when a later step fails: the caller ends only what is gone).
fn clear_keychain_session_holding(
    _write: &SessionWrite,
    account_id: &str,
    dropped: &mut DroppedTokens,
) -> Result<(), String> {
    // Clear the id-segmented trio (the live layout post-migration).
    let mut vault = AuthVault::new(platform_keychain_store_for(account_id));
    dropped.add(stored_session_token(&vault));
    vault
        .clear_session()
        .map_err(|e| keychain_error("clear Keychain session", e))?;
    // Belt-and-braces: also clear any lingering legacy flat trio so a logout
    // leaves no recoverable secrets behind even if migration never ran (or the
    // legacy fallback had been serving the session). `clear_session` is
    // idempotent (NotFound → Ok), so this is safe when nothing legacy exists.
    let mut legacy = AuthVault::new(keychain::legacy_platform_keychain_store());
    dropped.add(stored_session_token(&legacy));
    legacy
        .clear_session()
        .map_err(|e| keychain_error("clear legacy Keychain session", e))
}

/// R8: a startup 401 drops only the rejected token, in the id-keyed and the legacy store. The vault key and the email
/// stay, so the same account signs in again without its recovery phrase. Only inside a checked turn (`_write`), so a
/// newer session's token is never the one removed.
#[cfg(not(target_os = "windows"))]
fn clear_keychain_session_token(_write: &SessionWrite, account_id: &str) -> Result<(), String> {
    AuthVault::new(platform_keychain_store_for(account_id))
        .clear_session_token()
        .map_err(|e| keychain_error("clear Keychain session token", e))?;
    AuthVault::new(keychain::legacy_platform_keychain_store())
        .clear_session_token()
        .map_err(|e| keychain_error("clear legacy Keychain session token", e))
}

/// A startup 401 keeps the vault key and the email (R9), so a sign-out (and with it "Sign out and switch") must leave
/// none of the account's items for the next one: no token, no vault key, no email, in either layout. An item that cannot
/// be read counts as still there (fail closed); a store that cannot hold secrets has nothing to leave.
#[cfg(not(target_os = "windows"))]
fn ensure_keychain_holds_no_account(account_id: &str) -> Result<(), String> {
    if keychain_session_trace(account_id) || keychain_vault_key_present(account_id) || keychain_email_trace(account_id)
    {
        return Err(SIGN_OUT_KEYCHAIN_NOT_CLEARED.to_string());
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
const SIGN_OUT_KEYCHAIN_NOT_CLEARED: &str = "Sign-out paused: Beebeeb couldn’t remove the previous account’s keys from this computer. Try again; if it keeps happening, restart Beebeeb.";

/// Read the segmented keychain account-email credential WITHOUT unlocking
/// (email is metadata, not key material — no lock-state gate). `None` on
/// absence or on a store that cannot answer (the Linux fail-closed stub).
fn keychain_account_email(account_id: &str) -> Option<String> {
    AuthVault::new(platform_keychain_store_for(account_id))
        .account_email()
        .ok()
        .flatten()
}

/// The email to remember for the sign-in prefill at sign-out time, in
/// precedence order: the in-memory session's email, then the mirrored
/// per-account `auth_email`, then the keychain account-email credential.
/// MUST be called before `clear_keychain_session` erases the keychain copy.
fn signout_email_to_preserve(acct: &crate::account::AccountRuntime) -> Option<String> {
    acct.session
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().and_then(|s| s.email.clone()))
        .or_else(|| acct.auth_email.lock().ok().and_then(|guard| guard.clone()))
        .or_else(|| keychain_account_email(acct.id.as_str()))
}

/// Pure core of [`persist_last_signed_in_email`]: stage the prefill email
/// onto a config. `None` (nothing signed in) and unchanged values are
/// no-ops, so sign-out never rewrites `desktop.toml` just for the prefill.
/// Returns whether the config changed and needs saving.
fn stage_last_signed_in_email(cfg: &mut DesktopConfig, email: Option<&str>) -> bool {
    match email {
        None => false,
        Some(email) if cfg.last_signed_in_email.as_deref() == Some(email) => false,
        Some(email) => {
            cfg.last_signed_in_email = Some(email.to_string());
            true
        }
    }
}

/// Best-effort: remember the last signed-in email so the onboarding sign-in
/// form can prefill it after a sign-out / startup-401 auto sign-out (the
/// keychain account-email credential is erased by `clear_keychain_session`,
/// so this desktop.toml field is the only surviving copy). A config failure
/// must never fail sign-out — log and continue.
fn persist_last_signed_in_email(email: Option<String>) {
    let Some(email) = email else { return };
    // Task 1889: one load-change-save under the config-write lock (the sign-out runs beside the window's saves).
    let result = DesktopConfig::update(|cfg| (stage_last_signed_in_email(cfg, Some(&email)), ()));
    if let Err(error) = result {
        tracing::warn!(
            %error,
            "could not persist the last signed-in email for sign-in prefill (best-effort)"
        );
    }
}

/// What a sign-out that is an account switch says when it could not forget the previous account's email. Nothing was
/// torn down, so trying again is safe.
const SIGN_OUT_EMAIL_NOT_FORGOTTEN: &str = "Sign-out paused: Beebeeb couldn’t remove the previous account’s email from this computer. Try again; if it keeps happening, restart Beebeeb.";

/// An account switch's sign-out (spec §5.6: "The switch leaves no vault key and no account email behind, or it
/// stops"): forget the sign-in prefill (`last_signed_in_email`, which a sign-out and a startup 401 keep) before anything
/// is torn down. Unlike [`persist_last_signed_in_email`] this is not best-effort: a `desktop.toml` that cannot be read
/// or written stops the switch. An email that is not there is written nothing.
fn forget_last_signed_in_email() -> Result<(), String> {
    let path = DesktopConfig::path().map_err(|error| {
        tracing::warn!(%error, "account switch: desktop.toml could not be read to forget the previous email");
        SIGN_OUT_EMAIL_NOT_FORGOTTEN.to_string()
    })?;
    // Task 1889: one load-change-save under the config-write lock. The two log lines stay apart: a read that
    // failed and a write that failed are different things to look for.
    DesktopConfig::try_update_at(&path, |cfg| (cfg.last_signed_in_email.take().is_some(), ())).map_err(|error| {
        match error {
            config::UpdateError::Read(error) => {
                tracing::warn!(%error, "account switch: desktop.toml could not be read to forget the previous email")
            }
            config::UpdateError::Write(error) => {
                tracing::warn!(%error, "account switch: desktop.toml could not be written to forget the previous email")
            }
        }
        SIGN_OUT_EMAIL_NOT_FORGOTTEN.to_string()
    })
}

/// Result of the one bounded startup session probe (`GET /api/v1/auth/me`).
///
/// The classification is deliberately three-valued: only a DEFINITIVE HTTP
/// 401 proves the stored token is dead. Everything else — network error,
/// timeout, 5xx, any other status — is [`StartupSessionCheck::Inconclusive`]
/// and must fail OPEN (offline users must never be logged out by startup).
#[derive(Debug, Clone, PartialEq, Eq)]
enum StartupSessionCheck {
    /// `GET /auth/me` answered 2xx — the stored token is valid. `profile` is the account the server named in that
    /// answer (`None` when the body could not be read: still authorized, only the profile is missing).
    Authorized {
        profile: Option<account_dto::AccountProfile>,
    },
    /// `GET /auth/me` answered exactly 401 — the stored token is definitively
    /// rejected; the session may be discarded (auto sign-out).
    Unauthorized,
    /// Anything else (network error, timeout, 5xx, other status). Never
    /// authorizes discarding the session.
    Inconclusive(String),
}

/// One bounded `GET /api/v1/auth/me` with the stored session token (the same
/// endpoint+auth shape [`ApiClient::account_profile`] uses, via the ad-hoc
/// reqwest pattern the logout revocation uses). `base_url` is a parameter so
/// tests can point this at a loopback mock; production passes
/// [`runner::api_base_url`]. Total cost is capped at 5 seconds — this runs
/// during startup.
///
/// The bearer token is only sent over the wire, never logged.
async fn probe_startup_session(base_url: &str, token: &str) -> StartupSessionCheck {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .default_headers(api_client::provenance_headers())
        .build()
    {
        Ok(client) => client,
        Err(error) => return StartupSessionCheck::Inconclusive(format!("client build: {error}")),
    };
    let request = client.get(format!("{base_url}/api/v1/auth/me")).bearer_auth(token);
    let response = match tokio::time::timeout(std::time::Duration::from_secs(5), request.send()).await {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => return StartupSessionCheck::Inconclusive(format!("network error: {error}")),
        Err(_) => return StartupSessionCheck::Inconclusive("probe timed out after 5s".to_string()),
    };
    match response.status() {
        reqwest::StatusCode::UNAUTHORIZED => StartupSessionCheck::Unauthorized,
        status if status.is_success() => {
            let profile = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                response.json::<account_dto::AccountProfile>(),
            )
            .await
            .ok()
            .and_then(Result::ok);
            StartupSessionCheck::Authorized { profile }
        }
        status => StartupSessionCheck::Inconclusive(format!("HTTP {status}")),
    }
}

/// Apply the auto sign-out for a startup session the server definitively
/// rejected with 401: clear the keychain session trio (exactly like a real
/// sign-out's credential step — wrapped-master-key semantics untouched,
/// `clear_keychain_session` is the same call) and drop the auth flags so the
/// app boots signed-out and the SignedOutGate routes to onboarding/sign-in.
/// The in-memory session is never installed and the engine never started on
/// this path (the caller checks before doing either).
///
/// `email` is the rejected session's email — persisted for the sign-in
/// prefill BEFORE the keychain clear erases the keychain copy (best-effort).
///
/// Best-effort on the keychain clear: a store that cannot answer (the Linux
/// fail-closed stub) must not panic startup — log and continue, the flags
/// still route the app to signed-out.
#[cfg(target_os = "windows")]
fn discard_unusable_startup_session(state: &AppState, acct: &crate::account::AccountRuntime, email: Option<String>) {
    tracing::info!(
        account_id = acct.id.as_str(),
        "stored session token was rejected by the server (HTTP 401) at startup; \
         clearing the stored session so the app boots signed out"
    );
    // The session was never installed in memory, so the caller passes the
    // restored session's email directly; fall back to the per-account mirror
    // / keychain credential while they still exist (short-circuits when the
    // session email is known).
    persist_last_signed_in_email(email.or_else(|| signout_email_to_preserve(acct)));
    if let Err(error) = clear_keychain_session(acct.id.as_str()) {
        tracing::warn!(
            %error,
            "could not clear the server-rejected keychain session trio at startup (best-effort)"
        );
    }
    set_auth_present(state, false);
    set_auth_email(state, None);
}

/// R8 (spec 2026-10-06): the server rejected the stored token at startup. Drop only that token, keep the vault key and
/// the email, and boot signed out. The next sign-in as the same account swaps in a new token and needs no recovery
/// phrase (`reauth_in_place`). Beebeeb stays in Finder (this is not a sign-out by choice). Another account goes through
/// the switch: full sign-out first. Inside the restore's own turn (`turn`): when another session transition happened
/// while the probe waited (a sign-in may have stored a new token since), nothing is removed and no flag changes.
#[cfg(not(target_os = "windows"))]
fn discard_unusable_startup_session(
    state: &AppState,
    acct: &crate::account::AccountRuntime,
    email: Option<String>,
    turn: &mut account::SessionGeneration,
) {
    let Ok(write) = claim_session_write(acct, turn) else {
        tracing::info!("the server rejected the stored token at startup, but the session changed since; left as it is");
        return;
    };
    tracing::info!(
        account_id = acct.id.as_str(),
        "stored session token was rejected by the server (HTTP 401) at startup; dropping the token, keeping the keys (R8)"
    );
    persist_last_signed_in_email(email.or_else(|| signout_email_to_preserve(acct)));
    if let Err(error) = clear_keychain_session_token(&write, acct.id.as_str()) {
        tracing::warn!(%error, "could not clear the server-rejected session token at startup (best-effort)");
    }
    set_auth_present(state, false);
    set_auth_email(state, None);
}

/// On launch, resume a fully signed-in **and unlocked** session when the
/// platform credential store still holds BOTH the session token and the
/// vault master key.
///
/// This is the persistence guarantee desktop users expect from OneDrive /
/// Dropbox: quit + relaunch returns to a working, unlocked state with no
/// recovery-phrase prompt. The stored `wrapped-master-key` is the raw
/// 32-byte vault root key, protected at rest by the per-user OS credential
/// vault (DPAPI on Windows, Keychain on macOS) — reading it back is all that
/// is required to unlock; there is no passphrase to enter.
///
/// `load_session_from_keychain` is the same path `unlock_vault` uses: it
/// loads the token, calls `AuthVault::unlock()` (which validates the key is
/// 32 bytes and flips the vault to Unlocked), and returns the in-memory
/// `Session`. We then perform exactly the side-effects `apply_session` /
/// `unlock_vault` perform — stash the session in memory, mark auth present,
/// and start the engine for the configured sync root — so the resumed state
/// is indistinguishable from a fresh in-session unlock.
///
/// This function deliberately does NOTHING (no error surfaced) when:
///   - there is no stored session token (never signed in / logged out) —
///     `load_session_from_keychain` returns `Ok(None)`; or
///   - the token IS present but the master key is genuinely absent —
///     `AuthVault::unlock()` returns `AuthStoreError::NotFound`, which
///     `load_session_from_keychain` surfaces as `Err(..)` (NOT `Ok(None)`),
///     logged at `debug!`.
/// The second case is the legitimate "this PC doesn't have your keys" state
/// where onboarding's recovery-phrase step must still appear. `auth_present`
/// is already seeded from `keychain_session_present()` in `setup()`, so a
/// token-only restore still reports `logged_in: true` and routes to unlock.
/// A genuine credential-store/backend failure (anything other than
/// `NotFound`) is logged at `warn!` instead — it is unexpected but must still
/// not block startup, since onboarding can recover.
async fn restore_session_on_startup(app: &tauri::AppHandle) {
    restore_session_inner(app).await;
    // Whichever way the restore returned (refused on Windows, no account, a session already installed, nothing in
    // the Keychain, a revoked token, ...), the upgrade's one-time adoption window is over. The restore adopts on
    // what it loaded before it gets here; this closes the window on every path that did not.
    adopt_unbound_local_data_at_startup(None);
}

/// The startup restore proper (see [`restore_session_on_startup`], which shuts the adoption window after it).
async fn restore_session_inner(app: &tauri::AppHandle) {
    // Spec 2026-10-10 §15 question 3: a sign-out quit between its keep step and its purge left Finder accepts closed on
    // disk. Reopen them before this restore can start an engine.
    #[cfg(target_os = "macos")]
    reopen_finder_accepts_at_startup();
    #[cfg(target_os = "windows")]
    let _transition = SESSION_TRANSITION.lock().await;
    #[cfg(target_os = "windows")]
    if !SESSION_COMMANDS.startup_restore_allowed() || windows_cf::ensure_reactivation_allowed().is_err() {
        return;
    }
    let state = app.state::<AppState>();
    // `synthesize_single_account` runs in `setup()` before this task is
    // spawned, so the active account always resolves; bail defensively if not.
    let Ok(acct) = state.active_account() else {
        tracing::warn!("no active account during startup restore");
        return;
    };
    if !restore_stored_session(&state, &acct, &runner::api_base_url()).await {
        return;
    }
    // Windows keeps today's rule: a restored session with no email is named by the server before the engine start.
    #[cfg(target_os = "windows")]
    if acct
        .session
        .lock()
        .map(|guard| guard.as_ref().is_some_and(|session| session.email.is_none()))
        .unwrap_or(false)
    {
        establish_session_identity(&state, &acct, &runner::api_base_url(), None).await;
    }
    if let Err(error) = start_engine_if_possible(
        app.clone(),
        &state,
        #[cfg(target_os = "windows")]
        &_transition,
    )
    .await
    {
        tracing::warn!(%error, "startup engine refused");
    }
    // macOS and Linux (fix round 1, item 7): a restored session the probe could not name asks the server once more in
    // the background, through the account's single flight, and its engine start follows a successful naming. The
    // start above never waits for it.
    #[cfg(not(target_os = "windows"))]
    tauri::async_runtime::spawn(identify_then_start(app.clone(), IdentifyPace::Now));
}

/// Everything the startup restore does before its engine start, against the server at `base_url` (production passes
/// [`runner::api_base_url`]; a test passes a loopback stand-in): read the Keychain, offer the upgrade's adoption what
/// it read, ask the server whether the stored token still works, and put the session in memory. Returns whether a
/// session is in memory now, so the caller starts an engine for it.
async fn restore_stored_session(state: &AppState, acct: &AccountRuntime, base_url: &str) -> bool {
    // This restore is one session transition (lead ruling 1, Task 12): its generation is captured before anything is
    // read, and every write below happens in its turn. A Lock, a Sign-out or a sign-in that completes while the probe
    // waits ends the turn, and the restore then writes nothing, installs nothing and asks for no engine.
    let mut turn = acct.session_generation();
    // Don't clobber a session already installed this run (e.g. a fresh login
    // that landed before this startup task ran). The upgrade's adoption window is over either way.
    if acct.session.lock().map(|g| g.is_some()).unwrap_or(false) {
        adopt_unbound_local_data_at_startup(None);
        return false;
    }

    let email = acct.auth_email.lock().ok().and_then(|guard| guard.clone());
    let loaded = load_session_from_keychain(acct.id.as_str(), email);
    // R10: local data from before the account binding is adopted, once, by the account whose vault key this
    // computer holds, before the probe and before anything can start. With no vault key nobody is adopted and
    // the window shuts all the same. In this restore's turn: what was read is still what is stored.
    {
        let Ok(_write) = claim_session_write(acct, &mut turn) else {
            adopt_unbound_local_data_at_startup(None);
            return false;
        };
        adopt_unbound_local_data_at_startup(startup_adoption_candidate(&loaded).as_ref());
    }
    let session = match loaded {
        // Both token + master key present → vault is unlocked; resume.
        Ok(Some(session)) => session,
        // No stored session TOKEN at all (never signed in / logged out).
        // Leave onboarding to prompt for sign-in. Not an error.
        Ok(None) => return false,
        Err(error) => {
            // The token IS present but `unlock()` failed. Two very different cases:
            //   1. Key genuinely ABSENT — `AuthStoreError::NotFound` (Display:
            //      "secret not found"). This is the legitimate "this PC has your
            //      session but not your vault key" state; onboarding's recovery-
            //      phrase step must appear. Expected → log at debug, not warn.
            //   2. A real credential-store / backend failure (locked keychain,
            //      DPAPI error, deserialisation). Worth a warning so the user can
            //      see why auto-unlock didn't happen — but must NOT block startup.
            if error.contains("secret not found") {
                tracing::debug!(%error, "no vault key to auto-unlock at startup; recovery-phrase unlock required");
            } else {
                tracing::warn!(%error, "credential-store error while attempting auto-unlock at startup");
            }
            return false;
        }
    };

    // Definitive-401 auto sign-out (the dead-session trap fix): the restore
    // used to reinstall a stored token with NO server validation, while every
    // login path refuses while a session is installed — a revoked/expired
    // token trapped the user. ONE bounded probe decides: HTTP 401 → the
    // stored session is definitively dead → clear the keychain trio and boot
    // signed-out. ANYTHING else (network error, timeout, 5xx, other status)
    // fails OPEN — keep the session and current behavior; an offline user
    // must never be logged out by startup. Still inside the spawned startup
    // task: setup() is never blocked by this.
    let probe = probe_startup_session(base_url, &session.token).await;
    // Lead ruling, F9 review I-2, on every OS since spec D11 (task 1888): a token the server confirmed resumes the
    // queue below.
    let token_confirmed = matches!(probe, StartupSessionCheck::Authorized { .. });
    let probed = match probe {
        StartupSessionCheck::Unauthorized => {
            #[cfg(not(target_os = "windows"))]
            discard_unusable_startup_session(state, acct, session.email.clone(), &mut turn);
            #[cfg(target_os = "windows")]
            discard_unusable_startup_session(state, acct, session.email.clone());
            return false;
        }
        StartupSessionCheck::Authorized { profile } => profile,
        StartupSessionCheck::Inconclusive(reason) => {
            tracing::info!(
                %reason,
                "startup session probe inconclusive; failing open and keeping the stored session"
            );
            None
        }
    };

    let token = zeroize::Zeroizing::new(session.token.clone());
    let email = session.email.clone();
    {
        // Everything the restore puts in memory, in its turn: the session, the flags that mirror it, and what the
        // probe learned of the account.
        let Ok(write) = claim_session_write(acct, &mut turn) else {
            tracing::info!("the session changed while the startup probe waited; nothing restored");
            return false;
        };
        match acct.session.lock() {
            Ok(mut guard) => *guard = Some(session),
            Err(_) => {
                tracing::warn!("session mutex poisoned during startup restore");
                return false;
            }
        }
        set_auth_present(state, true);
        // Mirror `apply_session` / the live-unlock path: keep `auth_email` in step
        // with the restored session so the Account page shows the signed-in email
        // after an auto-unlock, instead of going blank until the next login. Only
        // overwrite when the restored session actually carries an email — never
        // clobber an already-known address with `None`.
        if email.is_some() {
            set_auth_email(state, email);
        }
        // R10 (lead rulings 4 and 5): the account the server named while the session still works. The session is
        // named after it (its user id is known from now on, so the binding compares ids), and an owner record of this
        // account written before ids were known is completed. The profile is cached either way.
        if let Some(profile) = probed {
            name_session_after_profile(&write, state, acct, &token, &profile, owner_record_dir().as_deref());
            if let Ok(mut cached) = acct.cached_profile.lock() {
                *cached = Some(profile);
            }
        }
        // Lead ruling, F9 review I-2: the server confirmed the stored token and the Keychain unlocked the vault, so
        // sync resumes as after a sign-in (the relaunch "Quit and reopen Beebeeb" asks for). A Mac makes the edits
        // paused for `auth` due before the engine's first tick; Windows and Linux after the engine's reconcile pass and
        // key check (spec D11, task 1888). Those held as `key_replaced` stay held.
        if token_confirmed {
            ask_to_resume_auth_paused(&write, acct);
        }
    }
    tracing::info!("vault auto-unlocked from credential store on startup");
    // A restored session the probe could not name (offline, or an unreadable answer) asks the server once more, but not
    // here (fix round 1, item 7): the caller does that without holding up the engine start.
    true
}

// UI cache epoch, independent of the credential-bearing command lease epoch.
static UI_SESSION_REVISION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Bumped at every place the in-memory session is installed by a sign-in or recovery-phrase
/// unlock, and at every place it is cleared (lock, sign-out). The password unlock from the
/// Keychain (`install_unlocked_session`) does NOT bump: it is correct only because the lock that
/// came before it did. A new path that clears the session (an auth-expiry auto-lock, say) must
/// call `bump_vault_epoch()` itself, or the popover serves the pre-clear storage figure for up to
/// five minutes after the next unlock. `lock_then_unlock_refetches_the_storage_summary` runs the
/// real lock and the real unlock install and fails if the lock stops bumping.
///
/// Keys the popover's cached storage summary, so an unlock refetches it even when
/// `UI_SESSION_REVISION` does not move (a lock keeps `auth_present` true, and the email is the
/// same on the way back). Kept separate from `UI_SESSION_REVISION` on purpose: bumping that one
/// makes the existing UI refetch, and slice 2 changes nothing a user sees.
static VAULT_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Unit tests only: a test that reads the epoch (the popover's storage cache is keyed by it) holds this gate
/// exclusively while it runs, and every other test's `bump_vault_epoch()` waits for it. The epoch is one static
/// for the whole test process, and any Lock, Sign-out or sign-in test that ran in parallel moved it under the
/// popover tests' feet (a fetch count that was 1 became 2, in about one full run in ten).
#[cfg(test)]
static EPOCH_GATE: std::sync::RwLock<()> = std::sync::RwLock::new(());

// Set on a thread that holds `EPOCH_GATE` exclusively, so its own `lock_vault` bump does not wait for itself.
#[cfg(test)]
thread_local! {
    static HOLDS_EPOCH_GATE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn bump_vault_epoch() {
    #[cfg(test)]
    let _shared =
        (!HOLDS_EPOCH_GATE.with(std::cell::Cell::get)).then(|| EPOCH_GATE.read().unwrap_or_else(|e| e.into_inner()));
    VAULT_EPOCH.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

fn set_auth_present(state: &AppState, present: bool) {
    if let Ok(mut guard) = state.auth_present.lock() {
        if *guard != present {
            UI_SESSION_REVISION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        *guard = present;
    }
}

fn set_auth_email(state: &AppState, email: Option<String>) {
    // Per-account (decision 0800): the signed-in address belongs to the active
    // account, not the process. No active account (registry not synthesized) is
    // a silent no-op, mirroring the old poisoned-mutex swallow.
    if let Ok(acct) = state.active_account()
        && let Ok(mut guard) = acct.auth_email.lock()
    {
        if *guard != email {
            UI_SESSION_REVISION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        *guard = email;
    }
}

/// Starts the sync engine for the configured sync root, if there is one. It takes no keys: the session
/// is read under the engine slot by `spawn_bound_engine` (lead ruling T9-starts), so a Lock that got
/// there first leaves nothing to start (`Ok`, with the vault left locked).
async fn start_engine_if_possible(
    app: tauri::AppHandle,
    state: &State<'_, AppState>,
    #[cfg(target_os = "windows")] _transition: &tokio::sync::MutexGuard<'_, ()>,
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        windows_cf::ensure_reactivation_allowed()?;
        SESSION_COMMANDS.open()?;
    }
    #[cfg(target_os = "windows")]
    let generation = SESSION_COMMANDS.generation()?;
    if let Ok(cfg) = DesktopConfig::load() {
        let Some(root) = cfg.sync_root else { return Ok(()) };
        // The engine + pause flag are per-account (decision 0800). No active
        // account → nothing to start.
        let Ok(acct) = state.active_account() else {
            return Ok(());
        };
        // Rehydrate the persisted pause state before spawning so a
        // restart-while-paused stays paused (the in-memory AtomicBool
        // always defaults to false; desktop.toml is the source of truth).
        rehydrate_sync_paused(&acct.sync_paused, cfg.pause_sync);
        let mut engine_slot = acct.engine.lock().await;
        engine_start_refusal(&acct)?;
        if let Some(prev) = engine_slot.take() {
            // Task 1538 Codex P1: this is a re-login/sync-root-change
            // respawn, not sign-out — there's no purge to gate here, but an
            // unconfirmed stop is still worth knowing about (a not-really-
            // gone previous task could still be touching the same state.db
            // the freshly spawned runner is about to open).
            if !stop_engine_in_slot(&acct, prev).await.is_stopped() {
                #[cfg(target_os = "windows")]
                return Err("Could not stop the previous sync engine; retry locking before restarting sync.".into());
                tracing::warn!("previous engine did not confirm termination before respawning a new one");
            }
        }
        // Fix round 1: an unconfirmed stop of the old engine has set the flag; refuse the new one.
        engine_start_refusal(&acct)?;
        #[cfg(target_os = "windows")]
        {
            windows_cf::ensure_reactivation_allowed()?;
            SESSION_COMMANDS.validate_start(generation)?;
        }
        // A vault locked in the meantime is not an error here: nothing to start, and Lock has said so. A start
        // deferred for a Finder removal is not one either: the reconciler removes the domain and its next check
        // starts the engine.
        match spawn_bound_engine(app, state, &acct, &mut engine_slot, root)? {
            EngineStart::Started => {}
            EngineStart::NoSession => {
                tracing::info!("engine start skipped: the vault was locked before it held the engine slot");
            }
            EngineStart::FinderRemovalOwed => {
                tracing::info!("engine start deferred: the previous account's Finder domain is removed first");
            }
        }
    }
    Ok(())
}

// ── IPC commands: first-launch onboarding ────────────────────────────────────

/// Open the sign-in surface (`surfaces::policy::onboarding_surface`).
///
/// A Mac signs in in the Beebeeb window (task 1683 slice 2, two-surface spec §7.4): the signed-out window shows the
/// sign-in screens itself, so this only shows `main` on Home, and the `onboarding` window is never built there.
/// Windows keeps its wider two-column wizard (`WindowsFirstRun.tsx`, label `windows-onboarding`) and Linux the compact
/// `Onboarding.tsx` (label `onboarding`). Re-calling when the window already exists is a no-op (focus only).
pub(crate) fn open_onboarding_window_impl(app: &tauri::AppHandle) -> Result<(), String> {
    match surfaces::policy::onboarding_surface(surfaces::policy::Platform::current()) {
        surfaces::policy::OnboardingSurface::BeebeebWindow => show_sign_in_in_beebeeb_window(app),
        #[cfg(not(target_os = "macos"))]
        surfaces::policy::OnboardingSurface::Window(label) => open_onboarding_window_labelled(app, label),
        #[cfg(target_os = "macos")]
        surfaces::policy::OnboardingSurface::Window(_) => Err("A Mac signs in in the Beebeeb window.".to_string()),
    }
}

/// A Mac's sign-in: the Beebeeb window on Home, which shows sign-in by itself when there is no account, or "Sign in
/// again" for an ended one. Nothing is cleared first; the window decides what to show from the account it reads.
#[cfg(target_os = "macos")]
fn show_sign_in_in_beebeeb_window(app: &tauri::AppHandle) -> Result<(), String> {
    show_main_window(app, MainNav::Home);
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn show_sign_in_in_beebeeb_window(_app: &tauri::AppHandle) -> Result<(), String> {
    Err("Only the macOS app signs in in the Beebeeb window.".to_string())
}

/// R8: “Sign in again” on a Mac. Unlike the sign-in above, which lets the window decide from the account it reads, this
/// asks the window for the card: a click on “Sign in again” always lands on it, even when the session was marked ended and
/// has since recovered (review P3-1). Nothing is cleared first.
#[cfg(target_os = "macos")]
fn show_reauth_in_beebeeb_window(app: &tauri::AppHandle) -> Result<(), String> {
    show_main_window(app, MainNav::Reauth);
    Ok(())
}

/// The first-run window of Windows and Linux. It is not compiled on a Mac, which never builds it.
///
/// The window is wide enough for the two-column shell and centred. It mounts `Onboarding.tsx` (`?window=onboarding`)
/// on Linux or the Windows wizard (`platform=windows`) selected by the label, matching the `windows-onboarding` entry
/// in tauri.conf.json.
#[cfg(not(target_os = "macos"))]
fn open_onboarding_window_labelled(app: &tauri::AppHandle, label: &'static str) -> Result<(), String> {
    let (url, width, height) = if label == "windows-onboarding" {
        ("index.html?window=onboarding&platform=windows", 860.0_f64, 640.0_f64)
    } else {
        ("index.html?window=onboarding", 860.0_f64, 640.0_f64)
    };

    if let Some(existing) = app.get_webview_window(label) {
        let _ = existing.show();
        let _ = existing.set_focus();
        return Ok(());
    }

    let win = tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::App(url.into()))
        .title("Welcome to Beebeeb")
        .inner_size(width, height)
        .min_inner_size(780.0, 560.0)
        .resizable(true)
        .center()
        .build()
        .map_err(|e| e.to_string())?;
    // Explicit show + focus: tauri.conf.json declares the window
    // `visible: false`; without these calls the window is created but
    // never appears on screen.
    let _ = win.show();
    let _ = win.set_focus();

    Ok(())
}

/// IPC wrapper so the frontend can also open the sign-in surface
/// (e.g., from Account page → "Switch account").
#[tauri::command]
async fn open_onboarding_window(app: tauri::AppHandle) -> Result<(), String> {
    open_onboarding_window_impl(&app)
}

/// R8: "Sign in again" opens sign-in in place. Nothing is cleared first. On a Mac that is the Beebeeb window, which
/// shows "Sign in again" for an ended account (two-surface spec §7.4); on Linux the onboarding window starts at the
/// sign-in step whatever `sync_status` says (`mode=reauth`); Windows keeps its own flow.
#[tauri::command]
async fn open_reauth_window(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let _ = app;
        Err("Only available on macOS and Linux.".to_string())
    }
    #[cfg(target_os = "macos")]
    {
        show_reauth_in_beebeeb_window(&app)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        const URL: &str = "index.html?window=onboarding&mode=reauth";
        if let Some(existing) = app.get_webview_window("onboarding") {
            existing
                .eval("window.location.search = '?window=onboarding&mode=reauth'")
                .map_err(|e| e.to_string())?;
            let _ = existing.show();
            let _ = existing.set_focus();
            return Ok(());
        }
        let window = tauri::WebviewWindowBuilder::new(&app, "onboarding", tauri::WebviewUrl::App(URL.into()))
            .title("Welcome to Beebeeb")
            .inner_size(860.0, 640.0)
            .min_inner_size(780.0, 560.0)
            .resizable(true)
            .center()
            .build()
            .map_err(|e| e.to_string())?;
        let _ = window.show();
        let _ = window.set_focus();
        Ok(())
    }
}

/// The sign-in card's "Quit Beebeeb" (spec C §7.1): the same exit as the menu's Quit and ⌘Q. It carries an exit code,
/// so `keep_running_on_exit_request` does not turn it down.
#[tauri::command]
fn quit_beebeeb(app: tauri::AppHandle) {
    app.exit(0)
}

/// Result of an OPAQUE sign-in attempt handed back to the frontend.
///
/// `requires_2fa == false` means the session is fully installed and onboarding
/// can proceed to the recovery-phrase step. `requires_2fa == true` means the
/// password was accepted but a TOTP code is still needed: `desktop_login`
/// stashed a [`Pending2fa`] in `AppState`, and the frontend must collect the
/// 6-digit code and call `desktop_login_2fa` to complete sign-in.
///
/// R8 adds `reauthenticated` (the account already on this Mac signed in again in place; with `vault_unlocked` its
/// keys are in memory, so no recovery phrase) and `account_mismatch` (another account: nothing changed on this Mac,
/// and the frontend shows the warning with the number of changes that have not uploaded). `key_replaced` is true only
/// for a re-sign-in whose kept vault key the server no longer accepts (spec §5.6): the key was removed, so the recovery
/// phrase step follows and says why. All five fields are ALWAYS serialized
/// (`{"requires_2fa":false,"reauthenticated":false,"vault_unlocked":false,"key_replaced":false,"account_mismatch":null}`
/// for a plain sign-in): the frontend's `settledFrom` reads them strictly, and
/// `login_outcome_json_is_the_frontends_contract` pins every shape.
#[derive(serde::Serialize)]
struct LoginOutcome {
    requires_2fa: bool,
    reauthenticated: bool,
    vault_unlocked: bool,
    key_replaced: bool,
    account_mismatch: Option<AccountMismatchDto>,
}

/// What the frontend needs to warn before an account switch: how many changes on this Mac have not uploaded yet.
/// On a Mac (spec 2026-10-10 W11) also the ask snapshot the switch step renders (`unsent`), or, when the count could
/// not be read, the coded E3 the step shows as a line (`unsent_error`); each is left out when it is `None`.
#[derive(serde::Serialize)]
struct AccountMismatchDto {
    pending_changes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    unsent: Option<sign_out::snapshot::AskCounts>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unsent_error: Option<String>,
}

impl LoginOutcome {
    fn signed_in() -> Self {
        Self {
            requires_2fa: false,
            reauthenticated: false,
            vault_unlocked: false,
            key_replaced: false,
            account_mismatch: None,
        }
    }

    fn needs_2fa() -> Self {
        Self {
            requires_2fa: true,
            ..Self::signed_in()
        }
    }

    /// A browser sign-in that installed its session: the handoff carried the vault key, so the vault is unlocked (spec C
    /// C-S1). The password paths cannot say that: they sign in without a key and the recovery phrase follows.
    fn browser_signed_in() -> Self {
        Self {
            vault_unlocked: true,
            ..Self::signed_in()
        }
    }

    #[cfg_attr(target_os = "windows", allow(dead_code))]
    fn reauthenticated(vault_unlocked: bool, key_replaced: bool) -> Self {
        Self {
            reauthenticated: true,
            vault_unlocked,
            key_replaced,
            ..Self::signed_in()
        }
    }

    #[cfg_attr(target_os = "windows", allow(dead_code))]
    fn account_mismatch(
        pending_changes: u64,
        unsent: Option<sign_out::snapshot::AskCounts>,
        unsent_error: Option<String>,
    ) -> Self {
        Self {
            account_mismatch: Some(AccountMismatchDto {
                pending_changes,
                unsent,
                unsent_error,
            }),
            ..Self::signed_in()
        }
    }
}

/// Authenticate with the current OPAQUE login endpoints.
///
/// This intentionally mirrors web and iOS: sign-in proves account access with
/// email/password only. If this Mac does not already have the vault key in
/// Keychain, onboarding moves to the separate recovery-phrase unlock step.
///
/// When the account has 2FA enabled, the OPAQUE finish returns
/// `requires_2fa: true` plus a short-lived `partial_token`. Rather than
/// erroring, we stash that token (with the email) in `AppState::pending_2fa`
/// and return `LoginOutcome { requires_2fa: true }`; the frontend then calls
/// `desktop_login_2fa` with the user's TOTP code to mint the real session.
///
/// Returns `Err` on HTTP errors, network failures, or OPAQUE failures.
///
/// Spec: docs/superpowers/plans/2026-05-07-desktop-sync-client.md (onboarding §1)
#[tauri::command]
async fn desktop_login(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    email: String,
    password: String,
) -> Result<LoginOutcome, String> {
    #[cfg(target_os = "windows")]
    let attempt = AUTH_ATTEMPTS.begin()?;
    let work = async {
        // This sign-in is one session transition (lead ruling 1, Task 12): captured before it looks at anything, so a
        // Lock, a Sign-out or another sign-in that completes while it waits for the server is never followed by its
        // writes.
        let mut turn = state.active_account()?.session_generation();
        // Windows refuses a sign-in while a session exists and never asks which account it is (R8).
        #[cfg(target_os = "windows")]
        let _ = &app;
        #[cfg(target_os = "windows")]
        let _transition = SESSION_TRANSITION.lock().await;
        #[cfg(target_os = "windows")]
        AUTH_ATTEMPTS.validate(&attempt)?;
        #[cfg(target_os = "windows")]
        {
            SESSION_COMMANDS.ensure_reactivation_allowed()?;
            windows_cf::ensure_reactivation_allowed()?;
        }
        #[cfg(target_os = "windows")]
        if state.auth_present.lock().map(|present| *present).unwrap_or(true) {
            return Err("Sign out of the current account before signing in again.".into());
        }

        #[cfg(target_os = "windows")]
        drop(_transition);
        let base_url = runner::api_base_url();
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .default_headers(api_client::provenance_headers())
            .build()
            .map_err(|e| format!("reqwest build: {e}"))?;

        let email = account_binding::canonical_email(&email);
        if email.is_empty() || password.is_empty() {
            return Err("Email and password are required.".to_string());
        }

        let login_start = beebeeb_core::opaque_protocol::client_login_start(password.as_bytes())
            .map_err(|e| format!("opaque login start: {e}"))?;
        let client_message = encode_base64(&login_start.message);

        let start_resp = client
            .post(format!("{base_url}/api/v1/opaque/login-start"))
            .json(&serde_json::json!({ "email": email, "client_message": client_message }))
            .send()
            .await
            .map_err(|e| format!("network error: {e}"))?;
        if start_resp.status() == reqwest::StatusCode::UNAUTHORIZED
            || start_resp.status() == reqwest::StatusCode::NOT_FOUND
        {
            return Err("Invalid email or password".to_string());
        }
        if !start_resp.status().is_success() {
            let status = start_resp.status();
            let body = start_resp.text().await.unwrap_or_default();
            return Err(format!("Login start failed ({status}): {body}"));
        }

        let start_body: serde_json::Value = start_resp
            .json()
            .await
            .map_err(|e| format!("parse login start response: {e}"))?;
        let server_message = start_body
            .get("server_message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "No server_message in login start response".to_string())?;
        let server_state = start_body
            .get("server_state")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "No server_state in login start response".to_string())?;
        // Dual-KSF login (task 0815): the server forwards the account's OPAQUE KSF
        // version (0 = legacy Identity, 1 = Argon2id) in the login-start response.
        // The client must finish under the matching CipherSuite, so thread it into
        // client_login_finish — mirrors web/iOS and the server's own call shape in
        // beebeeb-api/tests/api_integration.rs (`as u32` from the JSON number).
        let ksf_version = start_body
            .get("ksf_version")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| "No ksf_version in login start response".to_string())? as u32;
        let server_message_bytes =
            decode_base64(server_message).map_err(|e| format!("invalid OPAQUE server message: {e}"))?;
        let login_finish = beebeeb_core::opaque_protocol::client_login_finish(
            &login_start.state,
            password.as_bytes(),
            &server_message_bytes,
            ksf_version,
        )
        .map_err(|e| format!("Invalid email or password ({e})"))?;

        let finish_resp = client
            .post(format!("{base_url}/api/v1/opaque/login-finish"))
            .json(&serde_json::json!({
                "email": email,
                "client_message": encode_base64(&login_finish.message),
                "server_state": server_state,
            }))
            .send()
            .await
            .map_err(|e| format!("network error: {e}"))?;
        if finish_resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err("Invalid email or password".to_string());
        }
        if !finish_resp.status().is_success() {
            let status = finish_resp.status();
            let body = finish_resp.text().await.unwrap_or_default();
            return Err(format!("Login finish failed ({status}): {body}"));
        }
        let finish_body: serde_json::Value = finish_resp
            .json()
            .await
            .map_err(|e| format!("parse login finish response: {e}"))?;
        if finish_body
            .get("requires_2fa")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            // Password proven, but the account has 2FA. Capture the short-lived
            // partial session token (+ email) so `desktop_login_2fa` can complete
            // the sign-in once the user enters their TOTP code. The partial token is
            // server-validated and short-lived — never log it.
            #[cfg(target_os = "windows")]
            let _transition = SESSION_TRANSITION.lock().await;
            #[cfg(target_os = "windows")]
            AUTH_ATTEMPTS.validate(&attempt)?;
            let partial_token = finish_body
                .get("partial_token")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "No partial_token in login finish response".to_string())?
                .to_string();
            // A Lock or a Sign-out that completed while the password was checked clears the challenge; this one is not
            // set after it. macOS and Linux only: Windows orders this step with `SESSION_TRANSITION` and its attempt
            // fence, as before (Task 12 fix round 1, item 3).
            #[cfg(not(target_os = "windows"))]
            {
                let acct = state.active_account()?;
                if !acct.session_unchanged_since(turn) {
                    return Err(SessionChanged::of(&acct).into());
                }
            }
            {
                let mut guard = state
                    .pending_2fa
                    .lock()
                    .map_err(|_| "pending 2FA mutex poisoned".to_string())?;
                *guard = Some(Pending2fa {
                    partial_token,
                    email: email.clone(),
                });
            }
            tracing::info!("desktop login requires 2FA; awaiting TOTP code");
            return Ok(LoginOutcome::needs_2fa());
        }
        // Wiped when this login is done with it (it lives only as long as the sign-in handshake).
        let session_token = zeroize::Zeroizing::new(
            finish_body
                .get("session_token")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "No session_token in login finish response".to_string())?
                .to_string(),
        );

        let profile = fetch_session_profile(&client, &base_url, &session_token).await?;
        #[cfg(target_os = "windows")]
        let _transition = SESSION_TRANSITION.lock().await;
        #[cfg(target_os = "windows")]
        AUTH_ATTEMPTS.validate(&attempt)?;
        #[cfg(target_os = "windows")]
        if state.auth_present.lock().map(|present| *present).unwrap_or(true) {
            return Err("Sign out of the current account before signing in again.".into());
        }

        // R8 (spec 2026-10-06): the account already on this Mac signs in again in place; another account is an
        // account switch. Decided before anything is stored.
        #[cfg(not(target_os = "windows"))]
        {
            let settlement = match settle_sign_in(app.clone(), &state, &session_token, &profile, None, &mut turn).await
            {
                Ok(settlement) => settlement,
                Err(failure) => {
                    // A re-sign-in that failed before its token reached the Keychain stored nothing of the new
                    // session, so it is revoked, as a first sign-in whose token could not be stored is.
                    if !failure.token_stored {
                        let _ = revoke_desktop_session(&client, &base_url, &session_token).await;
                    }
                    return Err(failure.message);
                }
            };
            match settlement {
                SignInSettlement::Fresh => {}
                SignInSettlement::Reauthenticated {
                    vault_unlocked,
                    key_replaced,
                } => {
                    return Ok(LoginOutcome::reauthenticated(vault_unlocked, key_replaced));
                }
                SignInSettlement::AccountMismatch {
                    pending_changes,
                    unsent,
                    unsent_error,
                } => {
                    // Nothing on this Mac changed. The session just minted for the other account is revoked;
                    // after the switch the person signs in again.
                    let _ = revoke_desktop_session(&client, &base_url, &session_token).await;
                    return Ok(LoginOutcome::account_mismatch(pending_changes, unsent, unsent_error));
                }
                SignInSettlement::Unconfirmed => {
                    // The server could not say whose key this Mac holds: nothing changed, the new session is
                    // revoked, and trying again is safe.
                    let _ = revoke_desktop_session(&client, &base_url, &session_token).await;
                    return Err(SIGN_IN_ACCOUNT_UNKNOWN.to_string());
                }
            }
        }

        // Resolve the active account up front so we can both cache the profile and
        // segment the keychain write under its id (task 0800). The cache is
        // per-account (decision 0800); `pending_2fa` above stays on `AppState`.
        let acct = state.active_account()?;
        // The session is known by the server's spelling of the email, not by what was typed (R2).
        let email = session_email(Some(&profile)).unwrap_or(email);
        let stored = store_first_sign_in(&state, &acct, &mut turn, &session_token, &email, profile);
        if let Err(e) = stored {
            #[cfg(target_os = "windows")]
            drop(_transition);
            let _ = revoke_desktop_session(&client, &base_url, &session_token).await;
            return Err(e);
        }
        tracing::info!("desktop account session installed");

        Ok(LoginOutcome::signed_in())
    };
    #[cfg(target_os = "windows")]
    {
        attempt.run(work).await
    }
    #[cfg(not(target_os = "windows"))]
    {
        work.await
    }
}

/// A first sign-in by password (with or without a second factor): its token, the profile and the flags, written in the
/// sign-in's turn (lead ruling 1, Task 12). Refused, with nothing written, when any session transition happened since
/// the sign-in began (a Lock, a Sign-out, another sign-in that stored first); the caller then revokes the new session.
fn store_first_sign_in(
    state: &AppState,
    acct: &AccountRuntime,
    turn: &mut account::SessionGeneration,
    token: &str,
    email: &str,
    profile: account_dto::AccountProfile,
) -> Result<(), String> {
    let write = claim_session_write(acct, turn)?;
    persist_session_token_to_keychain(&write, acct.id.as_str(), token, Some(email))?;
    if let Ok(mut guard) = acct.cached_profile.lock() {
        *guard = Some(profile);
    }
    set_auth_present(state, true);
    set_auth_email(state, Some(email.to_string()));
    Ok(())
}

/// Complete a 2FA-gated sign-in: trade the held partial token + the user's TOTP
/// code for a real session, then run the same post-login setup as the no-2FA
/// path in `desktop_login`.
///
/// Requires a prior `desktop_login` call that returned
/// `requires_2fa: true` (which stashed a [`Pending2fa`] in `AppState`). On an
/// invalid/expired code the server replies 401 → we surface a clear, retryable
/// error and DELIBERATELY keep the pending state so the user can retry within
/// the partial token's ~5-minute window. On success we clear the pending state.
///
/// Mirrors the web client's `verify2fa(partialToken, code)`:
/// `POST /api/v1/auth/2fa/verify` with body `{ partial_token, code }` →
/// `{ user_id, session_token }`. The TOTP code is never logged.
#[tauri::command]
async fn desktop_login_2fa(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    code: String,
) -> Result<LoginOutcome, String> {
    #[cfg(target_os = "windows")]
    let attempt = AUTH_ATTEMPTS.begin()?;
    let work = async {
        // This sign-in is one session transition (lead ruling 1, Task 12): captured before it looks at anything, so a
        // Lock, a Sign-out or another sign-in that completes while it waits for the server is never followed by its
        // writes.
        let mut turn = state.active_account()?.session_generation();
        // Windows refuses a sign-in while a session exists and never asks which account it is (R8).
        #[cfg(target_os = "windows")]
        let _ = &app;
        #[cfg(target_os = "windows")]
        let _transition = SESSION_TRANSITION.lock().await;
        #[cfg(target_os = "windows")]
        AUTH_ATTEMPTS.validate(&attempt)?;
        #[cfg(target_os = "windows")]
        {
            SESSION_COMMANDS.ensure_reactivation_allowed()?;
            windows_cf::ensure_reactivation_allowed()?;
        }
        #[cfg(target_os = "windows")]
        drop(_transition);
        let base_url = runner::api_base_url();
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .default_headers(api_client::provenance_headers())
            .build()
            .map_err(|e| format!("reqwest build: {e}"))?;

        // Read (don't yet consume) the pending challenge: we only clear it on
        // success so an invalid code stays retryable within the 5-minute window.
        let (partial_token, email) = {
            let guard = state
                .pending_2fa
                .lock()
                .map_err(|_| "pending 2FA mutex poisoned".to_string())?;
            let pending = guard
                .as_ref()
                .ok_or_else(|| "No pending two-factor sign-in. Start sign-in again.".to_string())?;
            (pending.partial_token.clone(), pending.email.clone())
        };

        let code = code.trim().to_string();
        if code.is_empty() {
            return Err("Enter your authentication code.".to_string());
        }

        let verify_resp = client
            .post(format!("{base_url}/api/v1/auth/2fa/verify"))
            .json(&serde_json::json!({ "partial_token": partial_token, "code": code }))
            .send()
            .await
            .map_err(|e| format!("network error: {e}"))?;
        if verify_resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            // Invalid/expired code. Keep `pending_2fa` so the user can retry while
            // the partial token is still valid.
            return Err("Invalid authentication code".to_string());
        }
        if !verify_resp.status().is_success() {
            let status = verify_resp.status();
            let body = verify_resp.text().await.unwrap_or_default();
            return Err(format!("Two-factor verification failed ({status}): {body}"));
        }
        let verify_body: serde_json::Value = verify_resp
            .json()
            .await
            .map_err(|e| format!("parse 2FA verify response: {e}"))?;
        // Wiped when this login is done with it (it lives only as long as the sign-in handshake).
        let session_token = zeroize::Zeroizing::new(
            verify_body
                .get("session_token")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "No session_token in 2FA verify response".to_string())?
                .to_string(),
        );

        // Same post-finish setup as `desktop_login`'s no-2FA success path. The
        // cache is per-account (decision 0800); `pending_2fa` stays on `AppState`.
        let profile = fetch_session_profile(&client, &base_url, &session_token).await?;
        #[cfg(target_os = "windows")]
        let _transition = SESSION_TRANSITION.lock().await;
        #[cfg(target_os = "windows")]
        AUTH_ATTEMPTS.validate(&attempt)?;
        #[cfg(target_os = "windows")]
        if state.auth_present.lock().map(|present| *present).unwrap_or(true) {
            return Err("Sign out of the current account before signing in again.".into());
        }

        // R8 (spec 2026-10-06): the same check as `desktop_login`, before anything is stored. Whatever it finds, the
        // second factor is spent, so the held challenge is dropped on every way out but a first sign-in (which drops it
        // at the end).
        #[cfg(not(target_os = "windows"))]
        {
            let settlement = match settle_sign_in(app.clone(), &state, &session_token, &profile, None, &mut turn).await
            {
                Ok(settlement) => settlement,
                Err(failure) => {
                    if !failure.token_stored {
                        let _ = revoke_desktop_session(&client, &base_url, &session_token).await;
                    }
                    if let Ok(mut guard) = state.pending_2fa.lock() {
                        *guard = None;
                    }
                    return Err(failure.message);
                }
            };
            match settlement {
                SignInSettlement::Fresh => {}
                SignInSettlement::Reauthenticated {
                    vault_unlocked,
                    key_replaced,
                } => {
                    if let Ok(mut guard) = state.pending_2fa.lock() {
                        *guard = None;
                    }
                    return Ok(LoginOutcome::reauthenticated(vault_unlocked, key_replaced));
                }
                SignInSettlement::AccountMismatch {
                    pending_changes,
                    unsent,
                    unsent_error,
                } => {
                    // Nothing on this Mac changed. The session just minted for the other account is revoked.
                    let _ = revoke_desktop_session(&client, &base_url, &session_token).await;
                    if let Ok(mut guard) = state.pending_2fa.lock() {
                        *guard = None;
                    }
                    return Ok(LoginOutcome::account_mismatch(pending_changes, unsent, unsent_error));
                }
                SignInSettlement::Unconfirmed => {
                    let _ = revoke_desktop_session(&client, &base_url, &session_token).await;
                    if let Ok(mut guard) = state.pending_2fa.lock() {
                        *guard = None;
                    }
                    return Err(SIGN_IN_ACCOUNT_UNKNOWN.to_string());
                }
            }
        }

        let acct = state.active_account()?;
        // The session is known by the server's spelling of the email, not by what was typed (R2).
        let email = session_email(Some(&profile)).unwrap_or(email);
        let stored = store_first_sign_in(&state, &acct, &mut turn, &session_token, &email, profile);
        if let Err(e) = stored {
            #[cfg(target_os = "windows")]
            drop(_transition);
            let _ = revoke_desktop_session(&client, &base_url, &session_token).await;
            return Err(e);
        }
        // Challenge satisfied — drop the pending state so a stale partial token
        // can't be reused.
        if let Ok(mut guard) = state.pending_2fa.lock() {
            *guard = None;
        }
        tracing::info!("desktop account session installed after 2FA");

        Ok(LoginOutcome::signed_in())
    };
    #[cfg(target_os = "windows")]
    {
        attempt.run(work).await
    }
    #[cfg(not(target_os = "windows"))]
    {
        work.await
    }
}

/// Provision this Mac with the user's vault key after account sign-in.
///
/// A recovery phrase is only needed when the Mac does not already have a
/// Keychain-stored vault key. The account session was established by
/// `desktop_login`; this command derives the local vault key with
/// `beebeeb-core`, stores it in Keychain, and starts the sync runner.
#[tauri::command]
async fn desktop_unlock_with_recovery_phrase(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    recovery_phrase: String,
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let attempt = AUTH_ATTEMPTS.begin()?;
    let work = async {
        #[cfg(target_os = "windows")]
        let _transition = SESSION_TRANSITION.lock().await;
        #[cfg(target_os = "windows")]
        AUTH_ATTEMPTS.validate(&attempt)?;
        #[cfg(target_os = "windows")]
        {
            SESSION_COMMANDS.ensure_reactivation_allowed()?;
            windows_cf::ensure_reactivation_allowed()?;
        }
        let acct = state.active_account()?;
        // The unlock is one session transition (lead ruling 1, Task 12): captured before it reads anything.
        let mut turn = acct.session_generation();
        let session_installed = acct
            .session
            .lock()
            .map_err(|_| "session mutex poisoned".to_string())?
            .is_some();
        if session_installed {
            let started = start_engine_if_possible(
                app,
                &state,
                #[cfg(target_os = "windows")]
                &_transition,
            )
            .await;
            keys_arrived(&state, KeysFrom::SignIn);
            return started;
        }

        let account_id = acct.id.as_str().to_string();
        let token = zeroize::Zeroizing::new(
            load_session_token_from_keychain(&account_id)?
                .ok_or_else(|| "Sign in before unlocking the vault.".to_string())?,
        );
        let email = acct.auth_email.lock().ok().and_then(|guard| guard.clone());
        let base_url = runner::api_base_url();
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .default_headers(api_client::provenance_headers())
            .build()
            .map_err(|e| format!("reqwest build: {e}"))?;
        // Verification is cancellable network work. Teardown must be able to
        // acquire the transition and revoke this attempt while it is pending.
        #[cfg(target_os = "windows")]
        drop(_transition);
        let verified_key = verify_vault_key_from_phrase(&client, &base_url, &token, &recovery_phrase).await?;
        #[cfg(target_os = "windows")]
        let _transition = SESSION_TRANSITION.lock().await;
        #[cfg(target_os = "windows")]
        AUTH_ATTEMPTS.validate(&attempt)?;
        install_recovered_session(&state, &acct, &mut turn, &token, email, &verified_key)?;
        // Both copies are wiped here, not at the end of the function: the engine start below awaits, and the
        // session in memory is the only holder of the keys from now on.
        drop(token);
        drop(verified_key);
        tracing::info!("vault provisioned from recovery phrase");
        let started = start_engine_if_possible(
            app,
            &state,
            #[cfg(target_os = "windows")]
            &_transition,
        )
        .await;
        keys_arrived(&state, KeysFrom::SignIn);
        started
    };
    #[cfg(target_os = "windows")]
    {
        attempt.run(work).await
    }
    #[cfg(not(target_os = "windows"))]
    {
        work.await
    }
}

/// The recovery-phrase unlock's writes, after the server confirmed the phrase: the vault key and the email are written,
/// and the session put in memory, in ONE turn of the unlock (lead ruling 1, Task 12), and only while the account it
/// started with is still the one stored (a sign-in, a Lock or a Sign-out may have run while the phrase was being
/// checked). Nothing can land between the write and the install.
fn install_recovered_session(
    state: &AppState,
    acct: &AccountRuntime,
    turn: &mut account::SessionGeneration,
    token: &str,
    email: Option<String>,
    verified_key: &[u8; 32],
) -> Result<(), String> {
    {
        let write = store_recovered_vault_key(acct, turn, token, email.as_deref(), verified_key)?;
        let mut guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
        *guard = Some(Session {
            token: token.to_string(),
            master_key: *verified_key,
            email,
        });
        drop(guard);
        set_auth_present(state, true);
        // Ruling F9: the vault is unlocked now (after a replaced key too), so sync resumes; on every OS since spec D11.
        ask_to_resume_auth_paused(&write, acct);
        drop(write);
    }
    bump_vault_epoch();
    Ok(())
}

const INCORRECT_RECOVERY_PHRASE: &str = "Incorrect recovery phrase. Check your words and try again.";

/// What the recovery-phrase unlock says when a Sign-out, or another account's sign-in, happened while it waited for the
/// server (Task 12 fix round 2, item 5).
const UNLOCK_ACCOUNT_CHANGED: &str =
    "You were signed out, or another account signed in, while Beebeeb was unlocking. Try again.";

/// The recovery-phrase unlock's write of the verified vault key (and the account email), for the session it started
/// with: `started_token` is the token it read from the Keychain, `started_email` the identity it knew. Between that
/// read and this write the unlock waited for the server, and a sign-in or sign-out can have run: another account's
/// token may be stored now, or none, or the same account's replacement. In the unlock's turn (`turn`, see
/// [`claim_session_write`]) any session transition since it began stops the write, and the stored token, the identity
/// and the empty memory are checked again: one account's key is never written next to another account's token or
/// email. Returns the turn, still held, so the caller installs the session in memory before anything else can run.
fn store_recovered_vault_key(
    acct: &AccountRuntime,
    turn: &mut account::SessionGeneration,
    started_token: &str,
    started_email: Option<&str>,
    master_key: &[u8; 32],
) -> Result<SessionWrite, String> {
    let account_id = acct.id.as_str();
    // The unlock's turn (lead ruling 1, Task 12): any session transition since the unlock began stops it here. Then,
    // in the same turn, the stored token, the identity and the empty memory are checked against what it started with.
    let write = claim_session_write(acct, turn).map_err(|changed| changed.recovery_unlock_sentence().to_string())?;
    let stored = load_session_token_from_keychain(account_id)?.map(zeroize::Zeroizing::new);
    let session_installed = acct
        .session
        .lock()
        .map_err(|_| "session mutex poisoned".to_string())?
        .is_some();
    let identity = acct
        .auth_email
        .lock()
        .map_err(|_| "auth email mutex poisoned".to_string())?
        .clone();
    if stored.as_ref().map(|token| token.as_str()) != Some(started_token)
        || session_installed
        || identity.as_deref() != started_email
    {
        return Err(UNLOCK_ACCOUNT_CHANGED.to_string());
    }
    persist_vault_key_to_keychain(&write, account_id, master_key)?;
    // `desktop_login` already persisted the email when it stored the token, but persist again here (idempotent) so
    // the invariant "a fully-provisioned session has its email in the store" holds even if memory and store drift.
    if let Some(email) = started_email {
        let vault = AuthVault::new(platform_keychain_store_for(account_id));
        if let Err(e) = vault.store_account_email(email) {
            // Non-fatal: the email is display metadata, not required to unlock.
            tracing::warn!(error = %e, "could not persist account email during recovery-phrase unlock");
        }
    }
    // The caller puts the session in memory before it lets this turn go.
    Ok(write)
}

/// Derive the vault key from a recovery phrase, prove it belongs to the
/// signed-in account, and only then hand it to `persist`.
///
/// `recover_from_phrase` derives *a* key from ANY checksum-valid BIP39 phrase —
/// it does not prove the phrase is this account's. Persisting an unverified key
/// keys the sync engine with a foreign key, so every upload from this device is
/// undecryptable on every other device (flow 7 P0). Same gate as web's
/// device-provision (`recoveredKeyMatchesAccount`, task 0874).
#[cfg(test)]
async fn provision_vault_key_from_phrase<P>(
    client: &reqwest::Client,
    base_url: &str,
    session_token: &str,
    recovery_phrase: &str,
    persist: P,
) -> Result<[u8; 32], String>
where
    P: FnOnce([u8; 32]) -> Result<(), String>,
{
    let master_key = verify_vault_key_from_phrase(client, base_url, session_token, recovery_phrase).await?;
    persist(*master_key)?;
    Ok(*master_key)
}

/// Verification has no persistent side effects; callers fence installation.
async fn verify_vault_key_from_phrase(
    client: &reqwest::Client,
    base_url: &str,
    session_token: &str,
    recovery_phrase: &str,
) -> Result<zeroize::Zeroizing<[u8; 32]>, String> {
    let recovery_phrase = normalize_recovery_phrase_input(recovery_phrase)?;
    let master_key_struct = beebeeb_core::recovery::recover_from_phrase(&recovery_phrase)
        .map_err(|_| "Recovery phrase does not match a valid 12-word Beebeeb phrase.".to_string())?;
    if !recovered_key_matches_account(client, base_url, session_token, &master_key_struct).await? {
        // `master_key_struct` zeroizes on drop.
        return Err(INCORRECT_RECOVERY_PHRASE.to_string());
    }
    Ok(zeroize::Zeroizing::new(master_key_struct.to_bytes()))
}

/// The server's answer to "is this key the signed-in account's?" (`POST /api/v1/auth/verify-recovery-check`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecoveryCheckAnswer {
    Matches,
    /// A `400` whose code is exactly [`INVALID_RECOVERY_PHRASE_CODE`]: the key is not the account's. A server that has
    /// no distinct code for it sends the same answer for an account with no recovery check on file (spec §5.6).
    Mismatch,
    /// A `400` whose code is exactly [`RECOVERY_CHECK_MISSING`]: the account has no recovery check on file.
    NoCheckOnFile,
}

/// The `error` code the server sends when the key's check is not the account's. Matched exactly; nothing else is.
const INVALID_RECOVERY_PHRASE_CODE: &str = "invalid_recovery_phrase";

/// The `error` code the server sends when the account has no recovery check on file. Matched exactly; nothing else is.
const RECOVERY_CHECK_MISSING: &str = "recovery_check_missing";

/// Ask the server whether `master_key` is the signed-in account's key by comparing its `recovery_check` (HKDF of the
/// key; never the key itself) against the stored one. Only a completed answer is `Ok`: a `200` that says valid, or a
/// `400` with one of the two exact codes above. A network error, a 401, a 5xx, an unreadable success body, and any other
/// `400` (no JSON, another code, a body the server's extractor rejected, an intermediary's page) are `Err`: they say
/// nothing about the key, and an unreachable verifier is never read as "valid" (the caller fails closed).
async fn recovery_check_answer(
    client: &reqwest::Client,
    base_url: &str,
    session_token: &str,
    master_key: &beebeeb_core::kdf::MasterKey,
) -> Result<RecoveryCheckAnswer, String> {
    // The check stands in for the key at the server, so its text is wiped after the request (fix round 1, item 10),
    // and the body is serialized from a borrow of it (no second copy in a JSON value).
    let recovery_check = zeroize::Zeroizing::new(encode_base64(&*beebeeb_core::opaque::compute_recovery_check(
        master_key,
    )));
    #[derive(serde::Serialize)]
    struct RecoveryCheckBody<'a> {
        recovery_check: &'a str,
    }
    let resp = client
        .post(format!("{base_url}/api/v1/auth/verify-recovery-check"))
        .bearer_auth(session_token)
        .json(&RecoveryCheckBody {
            recovery_check: &recovery_check,
        })
        .send()
        .await
        .map_err(|e| format!("Could not verify the recovery phrase (network error: {e}). Try again."))?;
    let status = resp.status();
    if status == reqwest::StatusCode::BAD_REQUEST {
        let body = resp.text().await.unwrap_or_default();
        let code = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|value| value.get("error").and_then(|code| code.as_str()).map(str::to_string));
        return match code.as_deref() {
            Some(INVALID_RECOVERY_PHRASE_CODE) => Ok(RecoveryCheckAnswer::Mismatch),
            Some(RECOVERY_CHECK_MISSING) => Ok(RecoveryCheckAnswer::NoCheckOnFile),
            // Any other 400 says nothing about the key: answered like a server error, never as a mismatch.
            _ => Err(format!(
                "Could not verify the recovery phrase (server returned {status}). Try again."
            )),
        };
    }
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err("Your session expired. Sign in again before unlocking the vault.".to_string());
    }
    if !status.is_success() {
        return Err(format!(
            "Could not verify the recovery phrase (server returned {status}). Try again."
        ));
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Could not verify the recovery phrase (unreadable response: {e}). Try again."))?;
    if body.get("valid").and_then(|v| v.as_bool()) == Some(true) {
        Ok(RecoveryCheckAnswer::Matches)
    } else {
        Err("Could not verify the recovery phrase (unexpected response). Try again.".to_string())
    }
}

/// The recovery-phrase unlock's question: is the key from this phrase the signed-in account's? `Ok(true)` only on a
/// match. An account with no check on file proves nothing here either, so it stays `Ok(false)` (web applies the same
/// policy); see [`recovery_check_answer`] for what is `Err`.
async fn recovered_key_matches_account(
    client: &reqwest::Client,
    base_url: &str,
    session_token: &str,
    master_key: &beebeeb_core::kdf::MasterKey,
) -> Result<bool, String> {
    recovery_check_answer(client, base_url, session_token, master_key)
        .await
        .map(|answer| answer == RecoveryCheckAnswer::Matches)
}

fn normalize_recovery_phrase_input(input: &str) -> Result<String, String> {
    let words = input
        .split_whitespace()
        .map(|word| {
            word.trim()
                .to_ascii_lowercase()
                .chars()
                .filter(|c| c.is_ascii_alphabetic())
                .collect::<String>()
        })
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();

    if words.len() != 12 {
        return Err("Recovery phrase must contain exactly 12 words.".to_string());
    }

    Ok(words.join(" "))
}

/// Decode a base64 string (standard alphabet, with or without padding).
fn decode_base64(s: &str) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(s.trim())
        .map_err(|e| e.to_string())
}

fn encode_base64(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Verify a freshly-minted session by fetching `/api/v1/auth/me`, returning
/// the parsed [`account_dto::AccountProfile`].
///
/// This used to discard the payload after reading only `totp_enabled`. We
/// now return the whole profile so the caller can both gate on 2FA AND cache
/// the profile into `AppState` — that's what powers the `account_profile`
/// IPC without a second round-trip (the "all pages empty" data-layer fix).
/// The email a session is known by (R2, fix round 2 of Task 10; fix round 3): the SERVER's own spelling of it, from
/// the account's profile, and nothing else. Typed text is never an identity: a password login lowercases what was
/// typed client-side and a browser login does not, so the same account would have two, and signing in the other way
/// would look like another account. The profile's spelling and user id are what name an account here. Without a profile a session
/// has no email and is unidentified: no engine starts for it (see `authorize_engine_start`).
fn session_email(profile: Option<&account_dto::AccountProfile>) -> Option<String> {
    profile
        .map(|profile| profile.email.trim())
        .filter(|email| !email.is_empty())
        .map(str::to_string)
}

/// The server's record of the account behind `session_token`, for a sign-in that only has the credentials a browser
/// handed over. `None` when the server cannot be reached or does not answer with a profile: the sign-in then has no
/// identity, and no engine starts until [`establish_session_identity`] gets one.
async fn fetch_canonical_profile(session_token: &str) -> Option<account_dto::AccountProfile> {
    fetch_canonical_profile_at(&runner::api_base_url(), session_token).await
}

async fn fetch_canonical_profile_at(base_url: &str, session_token: &str) -> Option<account_dto::AccountProfile> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .default_headers(api_client::provenance_headers())
        .build()
        .ok()?;
    fetch_session_profile(&client, base_url, session_token).await.ok()
}

/// A session whose account is not known by its user id (its sign-in could not fetch the account record, the Keychain
/// kept no email, or the startup probe could not name it) asks the server again, and ONE fetch that succeeds names it
/// (lead ruling 5, Task 12): see [`name_session_after_profile`]. The startup restore calls this before it starts an
/// engine; app activation and "Try again" call it too ([`identify_unidentified_session`]), so recovering from a failed
/// fetch needs no relaunch. A failed fetch changes nothing (`false`), so the engine start keeps refusing; a session that
/// is identified already, that was replaced while the fetch ran, or whose turn ended (a Lock, a Sign-out) is left
/// alone. `owner_dir`: where the owner record a naming completes lives (`None`: no owner record is touched).
async fn establish_session_identity(
    state: &AppState,
    acct: &account::AccountRuntime,
    base_url: &str,
    owner_dir: Option<&Path>,
) -> bool {
    // One session transition: captured before the session is read, and the write below happens only in its turn.
    let turn = acct.session_generation();
    let (token, email) = match acct.session.lock() {
        Ok(guard) => match guard.as_ref() {
            Some(session) => (zeroize::Zeroizing::new(session.token.clone()), session.email.clone()),
            None => return false,
        },
        Err(_) => return false,
    };
    if session_knows_its_user_id(acct, email.as_deref()) {
        return false;
    }
    let Some(profile) = fetch_canonical_profile_at(base_url, &token).await else {
        return false;
    };
    // Checked, not yet moved on (fix round 1, item 5): a fetch that names nothing must not refuse a sign-in in flight.
    let Ok(write) = check_session_turn(acct, &turn) else {
        return false;
    };
    let named = name_session_after_profile(&write, state, acct, &token, &profile, owner_dir);
    if named {
        // Who the session is known as changed: a transition that began before this one is refused from now on.
        acct.advance_session_generation_holding_session_write_lock(account::SessionTransition::Write);
    }
    drop(write);
    named
}

/// Does the session know its account's user id: is a profile cached whose email is this session's own (see
/// [`identity_of_session`])? A session that does not is compared by email alone at an engine start.
fn session_knows_its_user_id(acct: &account::AccountRuntime, email: Option<&str>) -> bool {
    let profile = acct.cached_profile.lock().ok().and_then(|guard| guard.clone());
    identity_of_session(email, profile.as_ref()).user_id.is_some()
}

/// Name the session in memory after the server's own record of its account (R2; lead rulings 4 and 5, Task 12): its
/// email becomes the server's spelling, the profile is cached (from now on its user id is known), and the Account page
/// and the Keychain follow. Only the session whose token is `token`, and only when it has no email yet or the same
/// address in its canonical form (`account_binding::same_email`): an answer never names a session it was not fetched for, nor one another name was
/// given meanwhile. Inside a checked turn (`write`). `true` when it changed the session.
///
/// The owner record comes FIRST (fix round 1, item 1). With an owner record (`owner_dir`, macOS and Linux) the session is
/// named only after the record has been completed with the account's id and the server's spelling
/// ([`backfill_local_data_owner`]); a backfill that fails names nothing, so the stored email stays exactly as it was and
/// the next engine start still knows the account's own data (a later fetch tries again). Without one (Windows) a
/// session that has an email is never renamed to another spelling: only an email-less session is named, as before.
fn name_session_after_profile(
    write: &SessionWrite,
    state: &AppState,
    acct: &account::AccountRuntime,
    token: &str,
    profile: &account_dto::AccountProfile,
    owner_dir: Option<&Path>,
) -> bool {
    let Some(email) = session_email(Some(profile)) else {
        return false;
    };
    let known = {
        let Ok(guard) = acct.session.lock() else {
            return false;
        };
        match guard.as_ref() {
            Some(session)
                if session.token == token
                    && session
                        .email
                        .as_deref()
                        .is_none_or(|known| account_binding::same_email(known, &email)) =>
            {
                session.email.clone()
            }
            _ => return false,
        }
    };
    let renames = known.as_deref().is_some_and(|known| known != email);
    match owner_dir {
        Some(dir) => match backfill_local_data_owner(write, dir, profile) {
            Ok(true) => tracing::info!("R10: the owner record of the local data now names its account by user id"),
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(%error, "R10: the owner record could not be completed, so the session is not renamed; a later fetch tries again");
                return false;
            }
        },
        // No owner record to complete (Windows): another spelling of a known email is never taken over.
        None if renames => return false,
        None => {}
    }
    {
        let Ok(mut guard) = acct.session.lock() else {
            return false;
        };
        match guard.as_mut() {
            Some(session) if session.token == token => session.email = Some(email.clone()),
            _ => return false,
        }
    }
    if let Ok(mut cached) = acct.cached_profile.lock() {
        *cached = Some(profile.clone());
    }
    set_auth_email(state, Some(email.clone()));
    write_session_email_to_keychain(write, acct, &email);
    true
}

/// The Keychain copy of the session's email, written in the naming's turn (Task 12 fix round 2, item 3). A write that
/// fails is not ignored: it is logged as `keychain_email_write_failed` with the kind of store error only (never its
/// text or the address) and marked on the account, so the next naming trigger (app activation, "Try again", the next
/// launch's background request) writes it again. A store that cannot hold secrets (Linux) has nothing to retry.
/// `true` when the Keychain holds it now.
fn write_session_email_to_keychain(_write: &SessionWrite, acct: &account::AccountRuntime, email: &str) -> bool {
    match AuthVault::new(platform_keychain_store_for(acct.id.as_str())).store_account_email(email) {
        Ok(()) | Err(keychain::AuthStoreError::Unsupported(_)) => {
            acct.keychain_email_pending.store(false, Ordering::SeqCst);
            true
        }
        Err(error) => {
            acct.keychain_email_pending.store(true, Ordering::SeqCst);
            let kind = match error {
                keychain::AuthStoreError::Unsupported(_) => "unsupported",
                keychain::AuthStoreError::NotFound => "not_found",
                keychain::AuthStoreError::InvalidSecret(_) => "invalid_secret",
                keychain::AuthStoreError::Backend(_) => "backend",
            };
            tracing::warn!(
                event = "keychain_email_write_failed",
                kind,
                "the session's email is written to the Keychain again at the next naming"
            );
            false
        }
    }
}

/// Where the owner record of this computer's local data lives, for the identity steps that complete it (lead ruling 4).
/// `None` on Windows, which keeps today's binding (it refuses on an owner that differs instead).
fn owner_record_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        None
    }
    #[cfg(not(target_os = "windows"))]
    {
        state_paths::beebeeb_state_dir().ok()
    }
}

/// R10 (lead ruling 4, Task 12): while the session still works, the server's record of its account completes the owner
/// record of this computer's local data. An owner recorded before user ids were known (an email only, possibly spelled
/// with other letter case) gains the account's user id and the server's spelling of the email, so a later sign-in by
/// another method compares ids and never takes the account for another. Only an owner that IS this account is
/// completed: the same user id, or (with no id recorded) the same address in its canonical form (the engine start's
/// own comparison, `account_binding::same_account`). Any other owner is left to the binding at the next engine start. No database yet, or no owner: nothing to complete. Inside a checked
/// turn (`_write`).
fn backfill_local_data_owner(
    _write: &SessionWrite,
    state_dir: &Path,
    profile: &account_dto::AccountProfile,
) -> Result<bool, String> {
    let (Some(email), false) = (session_email(Some(profile)), profile.user_id.trim().is_empty()) else {
        return Ok(false);
    };
    let Some(db) = state_db_from_state_dir(state_dir)? else {
        return Ok(false);
    };
    let Some(owner) = db
        .owner()
        .map_err(|e| format!("read the local data owner: {e}"))?
        .filter(account_binding::Identity::is_known)
    else {
        return Ok(false);
    };
    let complete = account_binding::Identity::new(Some(&profile.user_id), Some(&email));
    // The engine start's own comparison (ruling A″): the ids when the owner has one, else the canonical email.
    let this_account = account_binding::same_account(&owner, &complete) == Some(true);
    if !this_account || owner == complete {
        return Ok(false);
    }
    db.set_owner(&complete)
        .map_err(|e| format!("record the local data owner: {e}"))?;
    Ok(true)
}

async fn fetch_session_profile(
    client: &reqwest::Client,
    base_url: &str,
    session_token: &str,
) -> Result<account_dto::AccountProfile, String> {
    let resp = client
        .get(format!("{base_url}/api/v1/auth/me"))
        .bearer_auth(session_token)
        .send()
        .await
        .map_err(|e| format!("session verification failed: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Session verification failed ({status}): {body}"));
    }
    resp.json::<account_dto::AccountProfile>()
        .await
        .map_err(|e| format!("parse session verification response: {e}"))
}

/// How `POST /api/v1/auth/logout` for one session ended (task 1886). Closed: logged by its code, never with the token,
/// the request or reqwest's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LogoutOutcome {
    /// The server answered 2xx: the session is ended there too.
    Revoked,
    /// The server answered 401: it had already ended the session.
    AlreadyEnded,
    /// The server answered with this other status.
    Refused(u16),
    /// The request did not reach the server, or no answer came back (offline, refused, DNS, TLS).
    Unreachable,
    /// The request could not be built (the API address is not a URL).
    Misconfigured,
    /// No answer within [`SIGN_OUT_REVOKE_TIMEOUT`] (a sign-out's revoke only).
    TimedOut,
    /// The HTTP client could not be built (a sign-out's revoke only).
    NoClient,
}

impl LogoutOutcome {
    fn code(self) -> &'static str {
        match self {
            Self::Revoked => "revoked",
            Self::AlreadyEnded => "already_ended",
            Self::Refused(_) => "refused",
            Self::Unreachable => "unreachable",
            Self::Misconfigured => "misconfigured",
            Self::TimedOut => "timed_out",
            Self::NoClient => "no_client",
        }
    }
}

/// End `session_token`'s session on the server. Never logs; the token goes only into the request's header.
async fn revoke_desktop_session(client: &reqwest::Client, base_url: &str, session_token: &str) -> LogoutOutcome {
    let response = match client
        .post(format!("{base_url}/api/v1/auth/logout"))
        .bearer_auth(session_token)
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) if error.is_builder() => return LogoutOutcome::Misconfigured,
        Err(_) => return LogoutOutcome::Unreachable,
    };
    match response.status() {
        status if status.is_success() => LogoutOutcome::Revoked,
        reqwest::StatusCode::UNAUTHORIZED => LogoutOutcome::AlreadyEnded,
        status => LogoutOutcome::Refused(status.as_u16()),
    }
}

/// How long a sign-out waits for the server to end its sessions, all of them together (task 1886, as Windows always
/// waited): after this the sign-out goes on without the answers.
const SIGN_OUT_REVOKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// The session tokens a sign-out took off this computer (task 1886), each kept once: the one it began with, and every one
/// its clears removed (a session written while it ran included). Each is ended on the server once the local clear has
/// run ([`end_dropped_sessions`]). Wiped on drop; never printed (no `Debug`).
#[derive(Default)]
struct DroppedTokens(Vec<zeroize::Zeroizing<String>>);

impl DroppedTokens {
    fn add(&mut self, token: Option<zeroize::Zeroizing<String>>) {
        if let Some(token) = token
            && !self.0.iter().any(|kept| **kept == *token)
        {
            self.0.push(token);
        }
    }
}

/// The token a sign-out begins with: the session's in memory, else the one the Keychain holds (a locked computer keeps
/// only that). Read once, before the sign-out takes the engine slot: from the slot on, nothing in the sign-out touches
/// the session until its clear (`sign_out_holds_the_engine_slot_until_the_session_is_cleared`).
fn session_token_for_revoke(acct: &AccountRuntime) -> Option<zeroize::Zeroizing<String>> {
    acct.session
        .lock()
        .ok()
        .and_then(|session| session.as_ref().map(|session| zeroize::Zeroizing::new(session.token.clone())))
        .or_else(|| {
            load_session_token_from_keychain(acct.id.as_str())
                .ok()
                .flatten()
                .map(zeroize::Zeroizing::new)
        })
}

/// The session token one Keychain layout holds, if it can say (task 1886: a clear reads what it is about to remove).
fn stored_session_token<S: keychain::AuthSecretStore>(vault: &AuthVault<S>) -> Option<zeroize::Zeroizing<String>> {
    vault
        .session_token()
        .ok()
        .flatten()
        .map(|token| zeroize::Zeroizing::new(token.expose_for_request().to_string()))
}

/// Every session token this computer still holds for `acct`: in memory and in both Keychain layouts. `None` when a
/// store cannot say (one that holds no secrets, `Unsupported`, or none, `NotFound`, holds none).
fn session_tokens_held_here(acct: &AccountRuntime) -> Option<Vec<zeroize::Zeroizing<String>>> {
    let mut held: Vec<zeroize::Zeroizing<String>> = acct
        .session
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_ref()
        .map(|session| zeroize::Zeroizing::new(session.token.clone()))
        .into_iter()
        .collect();
    for vault in [
        AuthVault::new(platform_keychain_store_for(acct.id.as_str())),
        AuthVault::new(keychain::legacy_platform_keychain_store()),
    ] {
        match vault.session_token() {
            Ok(token) => held.extend(token.map(|token| zeroize::Zeroizing::new(token.expose_for_request().to_string()))),
            Err(keychain::AuthStoreError::NotFound | keychain::AuthStoreError::Unsupported(_)) => {}
            Err(_) => return None,
        }
    }
    Some(held)
}

/// Task 1886 (P0): end these sessions on the server, all at once and within one [`SIGN_OUT_REVOKE_TIMEOUT`]. Best
/// effort: each outcome is logged by its closed code, never with a token. A 401 means the server had already ended the
/// session, which is no warning.
async fn revoke_sessions_for_sign_out(tokens: &[&str], base_url: &str) -> Vec<LogoutOutcome> {
    let deadline = tokio::time::Instant::now() + SIGN_OUT_REVOKE_TIMEOUT;
    let outcomes = match reqwest::Client::builder()
        .default_headers(api_client::provenance_headers())
        .build()
    {
        Err(_) => vec![LogoutOutcome::NoClient; tokens.len()],
        Ok(client) => {
            futures_util::future::join_all(tokens.iter().map(|token| {
                let client = &client;
                async move {
                    tokio::time::timeout_at(deadline, revoke_desktop_session(client, base_url, token))
                        .await
                        .unwrap_or(LogoutOutcome::TimedOut)
                }
            }))
            .await
        }
    };
    for &outcome in &outcomes {
        match outcome {
            LogoutOutcome::Revoked => tracing::info!(outcome = outcome.code(), "sign-out: the server ended the session"),
            LogoutOutcome::AlreadyEnded => {
                tracing::info!(outcome = outcome.code(), "sign-out: the server had already ended the session")
            }
            LogoutOutcome::Refused(status) => tracing::warn!(
                outcome = outcome.code(),
                status,
                "sign-out: the server did not end the session; signed out on this computer anyway"
            ),
            _ => tracing::warn!(
                outcome = outcome.code(),
                "sign-out: the server could not be asked to end the session; signed out on this computer anyway"
            ),
        }
    }
    outcomes
}

/// Task 1886 (P0): a sign-out ends on the server every session it took off this computer. Before, only Windows did, and
/// only for the token it began with, so a Mac's or a Linux computer's signed-out token stayed valid on the server until
/// it expired (30 days after the sign-in). Runs once the local clear has run (`cleared`: it completed) and the engine
/// slot is free, so `sync_status`, Lock, Repair and an engine start never wait for the server. After a clear that
/// completed nothing of the account is left here, so every dropped token is ended. After one that failed, only the
/// tokens this computer no longer holds are: one it still holds keeps working here, and trying the sign-out again ends
/// it; when a store cannot say what it holds, none is ended.
async fn end_dropped_sessions(acct: &AccountRuntime, dropped: DroppedTokens, cleared: bool) {
    let mut tokens = dropped.0;
    if !cleared {
        match session_tokens_held_here(acct) {
            Some(held) => tokens.retain(|token| !held.iter().any(|kept| **kept == **token)),
            None => {
                tracing::warn!(
                    dropped = tokens.len(),
                    "sign-out paused: this computer could not say which sessions it still holds; none was ended on the server"
                );
                return;
            }
        }
    }
    if tokens.is_empty() {
        return;
    }
    let tokens: Vec<&str> = tokens.iter().map(|token| token.as_str()).collect();
    revoke_sessions_for_sign_out(&tokens, &sign_out_api_base()).await;
}

#[cfg(test)]
tokio::task_local! {
    /// Set by [`with_sign_out_api_for_test`]: the API this task's sign-out revokes its session at.
    static SIGN_OUT_API_FOR_TEST: String;
}

/// Test builds: run `body` with `base` as the API a sign-out's revoke goes to (task 1886), so a test drives the real
/// sign-out against a server it started. Only this task sees it. (No Windows test drives a signed-in sign-out.)
#[cfg(all(test, not(target_os = "windows")))]
async fn with_sign_out_api_for_test<T>(base: String, body: impl std::future::Future<Output = T>) -> T {
    SIGN_OUT_API_FOR_TEST.scope(base, body).await
}

/// The API a sign-out's revoke goes to: [`runner::api_base_url`]. A test build never reads `BB_API_BASE` here (a test
/// running beside it may point that at its own server): it uses the server its own test started
/// ([`with_sign_out_api_for_test`]), else the closed loopback port every test build defaults to.
fn sign_out_api_base() -> String {
    #[cfg(test)]
    return SIGN_OUT_API_FOR_TEST
        .try_with(Clone::clone)
        .unwrap_or_else(|_| runner::TEST_API_BASE.to_string());
    #[cfg(not(test))]
    runner::api_base_url()
}

// ── IPC commands: session ─────────────────────────────────────────────────────

/// Persist a freshly-authenticated session and start the engine.
///
/// This is the shared core used by `start_browser_login` (the live browser-
/// handoff flow): it arrives at a session token + 32-byte master key + email
/// and needs write-to-credential-store + stash-in-memory + spawn-the-engine
/// side-effects. (Task 1479: this used to also be the core of a `set_session`
/// IPC command invoked by the WebView after an in-webview login — removed
/// because no desktop window ever loads the web client, so nothing could
/// call it. `apply_session` itself is untouched; it's still live.)
pub(crate) async fn apply_session(
    app: tauri::AppHandle,
    state: &State<'_, AppState>,
    token: String,
    master_key: &[u8; 32],
    profile: Option<account_dto::AccountProfile>,
    turn: &mut account::SessionGeneration,
    #[cfg(target_os = "windows")] attempt: &auth_attempts::Attempt,
) -> Result<(), String> {
    let token = zeroize::Zeroizing::new(token);
    // The session is known by the server's spelling of the email and by nothing the browser handed over (R2). With
    // no profile (the fetch failed) it has no email: it is installed, and no engine starts for it until a fetch
    // names it.
    let email = session_email(profile.as_ref());
    #[cfg(target_os = "windows")]
    let _transition = SESSION_TRANSITION.lock().await;
    #[cfg(target_os = "windows")]
    {
        SESSION_COMMANDS.ensure_reactivation_allowed()?;
        windows_cf::ensure_reactivation_allowed()?;
    }
    #[cfg(target_os = "windows")]
    if state.auth_present.lock().map(|present| *present).unwrap_or(true) {
        return Err("Sign out of the current account before signing in again.".into());
    }

    #[cfg(target_os = "windows")]
    AUTH_ATTEMPTS.validate(attempt)?;

    let acct = state.active_account()?;
    install_new_session_or_revoke(
        state,
        &acct,
        turn,
        &token,
        master_key,
        email,
        profile,
        &runner::api_base_url(),
    )
    .await?;
    bump_vault_epoch();
    tracing::info!("session installed");

    // If we already know the sync_root, kick off the engine. Otherwise
    // it'll start when the first-launch picker resolves.
    let started = start_engine_if_possible(
        app,
        state,
        #[cfg(target_os = "windows")]
        &_transition,
    )
    .await;
    keys_arrived(state, KeysFrom::SignIn);
    started
}

/// What [`install_new_session`] did.
#[derive(Debug, PartialEq, Eq)]
enum NewSession {
    Installed,
    /// Another session transition happened since the sign-in began: nothing was stored (the caller revokes it).
    Refused(String),
}

/// The browser sign-in's writes, in its turn (lead ruling 1, Task 12): its generation was captured before it looked at
/// this computer, so a Lock, a Sign-out or another sign-in that completed meanwhile stops it here, before anything is
/// stored. The Keychain (token, vault key, email), the cached profile, the session in memory and the flags are written
/// in the same turn. `Err`: a write failed after the turn was taken.
fn install_new_session(
    state: &AppState,
    acct: &AccountRuntime,
    turn: &mut account::SessionGeneration,
    token: &str,
    master_key: &[u8; 32],
    email: Option<String>,
    profile: Option<account_dto::AccountProfile>,
) -> Result<NewSession, String> {
    let write = match claim_session_write(acct, turn) {
        Ok(write) => write,
        Err(changed) => return Ok(NewSession::Refused(changed.sign_in_sentence().to_string())),
    };
    persist_session_to_keychain(&write, acct.id.as_str(), token, master_key, email.as_deref())?;
    if let Some(profile) = profile
        && let Ok(mut cached) = acct.cached_profile.lock()
    {
        *cached = Some(profile);
    }
    let mut guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
    *guard = Some(Session {
        token: token.to_string(),
        master_key: *master_key,
        email: email.clone(),
    });
    drop(guard);
    set_auth_present(state, true);
    set_auth_email(state, email);
    // Ruling F9: sync resumes; on every OS since spec D11 (task 1888).
    ask_to_resume_auth_paused(&write, acct);
    drop(write);
    Ok(NewSession::Installed)
}

/// [`install_new_session`], and when its turn is refused, the revoke of the session it would have stored (nothing of it
/// is on this computer), against the server at `base_url`, like a sign-in that could not store its token.
#[allow(clippy::too_many_arguments)]
async fn install_new_session_or_revoke(
    state: &AppState,
    acct: &AccountRuntime,
    turn: &mut account::SessionGeneration,
    token: &str,
    master_key: &[u8; 32],
    email: Option<String>,
    profile: Option<account_dto::AccountProfile>,
    base_url: &str,
) -> Result<(), String> {
    if let NewSession::Refused(error) = install_new_session(state, acct, turn, token, master_key, email, profile)? {
        if let Ok(client) = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .default_headers(api_client::provenance_headers())
            .build()
        {
            let _ = revoke_desktop_session(&client, base_url, token).await;
        }
        return Err(error);
    }
    Ok(())
}

/// What [`clear_session_impl`] actually did, so the native menu can tell
/// "you were already signed out" apart from a completed teardown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SignOutOutcome {
    /// Full teardown ran (or completed idempotently).
    Completed,
    /// Nothing was signed in — engine stop and Cloud Files teardown were
    /// skipped; only the idempotent cleanups (keychain clear, local purge)
    /// ran.
    NotSignedIn,
}

/// What a sign-out or a Lock that HAPPENED could not confirm (FB-24): a closed code and its constant sentence,
/// serialized as `{"code": …, "sentence": …}` like `sync_status.engine_refusal`. An `Err` from either command keeps
/// meaning "it did not happen". The variant names mirror the wire codes (`finder_removal_unconfirmed`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
enum ActionWarning {
    /// Signed out, keys cleared, but the removal from Finder was not confirmed (macOS).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    FinderRemovalUnconfirmed,
    /// Locked, keys cleared, but the reconciler did not confirm it holds (macOS).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    FinderLockUnconfirmed,
    /// Locked, keys cleared, but an engine stop (this one or an earlier one) was never confirmed (macOS and Linux).
    #[cfg_attr(target_os = "windows", allow(dead_code))]
    EngineStopUnconfirmed,
}

impl ActionWarning {
    fn code(self) -> &'static str {
        match self {
            Self::FinderRemovalUnconfirmed => "finder_removal_unconfirmed",
            Self::FinderLockUnconfirmed => "finder_lock_unconfirmed",
            Self::EngineStopUnconfirmed => "engine_stop_unconfirmed",
        }
    }

    fn sentence(self) -> &'static str {
        match self {
            Self::FinderRemovalUnconfirmed => FINDER_SIGN_OUT_UNCONFIRMED_WARNING,
            Self::FinderLockUnconfirmed => FINDER_LOCK_UNCONFIRMED_WARNING,
            Self::EngineStopUnconfirmed => LOCK_ENGINE_UNCONFIRMED_WARNING,
        }
    }
}

impl serde::Serialize for ActionWarning {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct as _;
        let mut warning = serializer.serialize_struct("ActionWarning", 2)?;
        warning.serialize_field("code", self.code())?;
        warning.serialize_field("sentence", self.sentence())?;
        warning.end()
    }
}

/// What `clear_session` and `lock_vault` return when the action happened. `warning` is always serialized (null when
/// every step was confirmed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
struct SessionActionOutcome {
    warning: Option<ActionWarning>,
}

/// Spec 2026-10-10 §5.2 step 10: the folder the keep step made and its file count. Never logged.
#[derive(Debug, Clone, PartialEq, Eq)]
struct KeptUnsent {
    folder: String,
    files: usize,
}

/// Task 2191 (review thread on #157): a sign-out failure after the Windows purge, carrying the
/// folder it kept copies of unknown files in (or the macOS keep step's), so the alert can name it.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn sign_out_failure_after_purge(
    message: String,
    preserved_location: &Option<String>,
    kept_unsent: &Option<KeptUnsent>,
    windows_kept: &Option<KeptUnsent>,
) -> SignOutFailure {
    SignOutFailure {
        message,
        preserved_location: preserved_location.clone(),
        kept_unsent: kept_unsent.clone().or_else(|| windows_kept.clone()),
    }
}

/// What [`clear_session_impl`] did, and what of it could not be confirmed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SignedOut {
    outcome: SignOutOutcome,
    warning: Option<ActionWarning>,
    /// Task 1882: the folder where macOS kept the Finder files that had not reached the server when the
    /// sign-out removed the Finder location. Shown to the person in the sign-out's alert
    /// (`surface_sign_out_folders`), never logged.
    preserved_location: Option<String>,
    /// Task 1887: the folder the sign-out's keep step copied the waiting Finder saves into (macOS). Never logged.
    kept_unsent: Option<KeptUnsent>,
}

#[cfg(test)]
impl SignedOut {
    /// A sign-out whose every step was confirmed.
    fn confirmed(outcome: SignOutOutcome) -> Self {
        Self {
            outcome,
            warning: None,
            preserved_location: None,
            kept_unsent: None,
        }
    }
}

/// [`clear_session_impl`]'s failure. Review I1 (task 1882 round 2): a sign-out can still fail AFTER it
/// removed the Finder location, and the folder macOS kept must not be lost with it: the error carries it,
/// and the alert is raised before the error is returned.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SignOutFailure {
    message: String,
    /// The folder macOS kept, when the failure came after the removal. Never logged.
    preserved_location: Option<String>,
    /// Task 1887: the folder the keep step made, when the failure came after the keep step. Never logged.
    kept_unsent: Option<KeptUnsent>,
}

impl From<String> for SignOutFailure {
    fn from(message: String) -> Self {
        Self {
            message,
            preserved_location: None,
            kept_unsent: None,
        }
    }
}

impl From<&str> for SignOutFailure {
    fn from(message: &str) -> Self {
        message.to_string().into()
    }
}

impl std::fmt::Display for SignOutFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// The unconfirmed-stop refusal (Bug A / task 1538 Codex P1). Shared by the
/// fresh-abort path and the Bug-A2 retry gate so a retry cannot be
/// distinguished from a first refusal by its message.
const UNCONFIRMED_ENGINE_STOP_ERROR: &str = "Could not stop the sync engine. Please try signing out again; if this keeps \
     happening, restart Beebeeb before signing in with a different account.";

/// Drop any cached session and abort the engine if running.
///
/// Shared by the WebView IPC command and the native menu "Sign out" item so
/// both routes have the exact same security side-effects.
async fn clear_session_impl(
    state: &AppState,
    forget_email: bool,
    keep_unsent: Option<&str>,
    unknown_files: UnknownFilesArg,
    kept_folder: Option<std::path::PathBuf>,
) -> Result<SignedOut, SignOutFailure> {
    #[cfg(target_os = "windows")]
    let _transition = SESSION_TRANSITION.lock().await;
    #[cfg(target_os = "windows")]
    let mut _auth_reopen = None;
    // Spec 2026-10-10 W5: the gate runs first, before anything changes. A refusal changes nothing.
    #[cfg(target_os = "macos")]
    let accepted = gate_sign_out(state, keep_unsent)?;
    #[cfg(not(target_os = "macos"))]
    let _ = keep_unsent;
    let acct = state.active_account()?;
    // An account switch (spec §5.6) leaves no account email behind, or it stops: the prefill is forgotten before
    // anything is torn down, and it is never saved again below.
    if forget_email {
        forget_last_signed_in_email()?;
    }
    // Task 12, lead ruling 1: no session transition that began before this sign-out writes anything from here on (the
    // session clear below ends any that begins while it runs).
    drop(end_sessions_in_flight(&acct, account::SessionTransition::SignOut));

    // Spec 2026-10-06 §5.3 (5): a sign-out by choice removes Beebeeb from Finder. The reconciler
    // cancels any check first and holds, so no check can start an engine after the stop below (plan
    // "Spec issues" 8). A removal it cannot confirm (not running, no answer in time, or it failed) does
    // not stop the sign-out: everything below still runs, and the result says it was unconfirmed
    // (lead rulings T8-⚠lock 1, T7-⚠2 5).
    #[cfg(target_os = "macos")]
    let finder_cleanup = finder_remove_for(state, finder_setup::core::Trigger::SignOut).await;
    // Task 1882 (P0): the removal keeps the files that never reached the server
    // (`NSFileProviderDomainRemovalModePreserveDirtyUserData`), and the folder macOS kept them in rides the
    // result to the alert. A removal that was not confirmed is logged here; one that failed but kept a folder
    // still names it (review M2).
    #[cfg(target_os = "macos")]
    let preserved_location = finder_removal::sign_out_kept_location(finder_cleanup.clone());
    #[cfg(not(target_os = "macos"))]
    let preserved_location: Option<String> = None;
    // Review I1: the Finder location is gone from here on, so every failure below carries the folder.
    let kept_with = |message: String| SignOutFailure {
        message,
        preserved_location: preserved_location.clone(),
        kept_unsent: None,
    };
    // What the result says when the sign-out happens: a removal the reconciler did not confirm (FB-24).
    #[cfg(target_os = "macos")]
    let removal_warning = finder_cleanup
        .is_err()
        .then_some(ActionWarning::FinderRemovalUnconfirmed);
    #[cfg(not(target_os = "macos"))]
    let removal_warning: Option<ActionWarning> = None;
    // Held to the end of the function: a sign-out that stops short with the keys still in memory puts
    // Finder back (lead ruling T3-⚠2).
    #[cfg(target_os = "macos")]
    let _finder_restore = FinderRestoreOnAbort { state, acct: &acct };

    // Bug B guard: already signed out everywhere (no auth flag, no in-memory
    // session, no keychain session). The old flow ran the full teardown anyway and
    // could FAIL (CFAPI/purge preflight) — trapping the user in a
    // "can't sign out because not signed in" loop. On Windows this path skips the
    // engine stop and the Cloud Files teardown entirely. On macOS and Linux it
    // still takes the engine slot and stops any engine it finds (Task 12 fix round
    // 2, item 1), so it also refuses after an unconfirmed engine stop. What it does
    // skip there, against a signed-in sign-out: saving the email for the next
    // sign-in; the `SignedOut` lifecycle event (it returns `NotSignedIn`, not
    // `Completed`); and failing on a Keychain clear that errors (that error is
    // logged and the sign-out goes on). What still runs below, on BOTH paths:
    // the idempotent keychain clear, the local-state purge (the queue purge is a
    // cross-account safety invariant — never skipped), and the auth-flag resets.
    let already_signed_out = {
        let auth_present = state.auth_present.lock().map(|present| *present).unwrap_or(false);
        let session_installed = acct.session.lock().map(|guard| guard.is_some()).unwrap_or(false);
        !auth_present && !session_installed && !keychain_session_present(acct.id.as_str())
    };
    // The engine slot, once taken below, is held until the session is cleared (lead ruling T8-⚠lock 2):
    // every start reads its keys under the slot, so releasing it first would let a start that was waiting
    // for it see keys this sign-out is about to clear. `clear_session_holding_slot` makes that order the
    // only one that compiles. Session installs are ordered against a sign-out separately (Task 12).
    //
    // Task 1886: every session token this sign-out takes off this computer, each ended on the server once the local
    // clear has run and the slot is free (`end_dropped_sessions`, at the end). It begins with the token in memory or the
    // Keychain, read now, before the slot is taken: from the slot on, nothing here touches the session until its clear.
    // The clears below add what they remove, a session written while the sign-out runs included. An already-signed-out
    // sign-out begins with none.
    let mut dropped = DroppedTokens::default();
    if !already_signed_out {
        dropped.add(session_token_for_revoke(&acct));
    }
    let mut held_engine_slot = if already_signed_out {
        #[cfg(target_os = "windows")]
        tracing::info!("sign-out requested while already signed out; skipping engine stop and Cloud Files teardown");
        #[cfg(not(target_os = "windows"))]
        tracing::info!("sign-out requested while already signed out; stopping any engine still in the slot");
        // macOS and Linux (Task 12 fix round 2, item 1): this path holds the engine slot from here too, BEFORE the
        // purge, and stops any engine it finds there (one a sign-in that completed after this sign-out began has
        // started), through the same helper and with the same refusals as the signed-in path. A start that comes during
        // the purge waits for the slot and then finds no keys.
        #[cfg(not(target_os = "windows"))]
        {
            Some(
                take_slot_and_stop_engine_for_sign_out(&acct)
                    .await
                    .map_err(&kept_with)?,
            )
        }
        #[cfg(target_os = "windows")]
        {
            None
        }
    } else {
        // Remember the signed-in email for the sign-in prefill BEFORE any
        // credential erasure: `clear_keychain_session` deletes the keychain
        // account-email credential, so this capture is the last chance to
        // keep the address the sign-in form should open with. Best-effort —
        // never gates sign-out. On the already-signed-out path the capture
        // finds nothing and no write happens. An account switch saves nothing (it forgot the email above).
        if !forget_email {
            persist_last_signed_in_email(signout_email_to_preserve(&acct));
        }
        #[cfg(target_os = "windows")]
        close_session_commands().await?;
        // Stop the engine before dropping memory so the IPC listener cannot accept
        // new File Provider operations with a cloned master key (see `take_slot_and_stop_engine_for_sign_out`).
        let engine_slot = take_slot_and_stop_engine_for_sign_out(&acct)
            .await
            .map_err(&kept_with)?;

        #[cfg(target_os = "windows")]
        {
            windows_cf::revoke_callbacks()
                .await
                .map_err(|e| format!("Could not disconnect Cloud Files: {e}"))?;
            stop_own_data_proofs();
            windows_cf::wait_for_credential_release().await?;
            // Native/credential shutdown is now confirmed. Cleanup may refuse dirty
            // files; allow an explicit unlock so the user can sync and retry.
            {
                SESSION_COMMANDS.finish_close();
                _auth_reopen = Some(auth_attempts::ReopenOnDrop(&AUTH_ATTEMPTS));
            }
        }
        Some(engine_slot)
    };
    // Spec 2026-10-10 §5.2 step 7 (W6): after the confirmed stop of either arm above, before the purge. Nothing between
    // here and the purge deletes a file, writes the queue, clears a session or starts an engine (U10).
    #[cfg(test)]
    sign_out_seam("before_keep").map_err(&kept_with)?;
    #[cfg(target_os = "macos")]
    let kept = keep_for_sign_out(accepted.as_ref()).map_err(&kept_with)?;
    // Spec 2026-10-10 §15 question 3: the keep step left Finder accepts closed for the purge, which clears them with
    // the rows. Any return before the purge has done so reopens them.
    #[cfg(target_os = "macos")]
    let mut reopen_accepts = AcceptsReopenOnAbort { armed: true };
    #[cfg(not(target_os = "macos"))]
    let kept: Option<sign_out::kept_set::KeptSet> = None;
    let kept_unsent = kept.as_ref().and_then(|kept| {
        kept.folder.as_ref().map(|folder| KeptUnsent {
            folder: folder.to_string_lossy().into_owned(),
            files: kept.files,
        })
    });
    // §5.2 step 10: from here on every failure carries our folder too.
    let kept_with = |message: String| SignOutFailure {
        message,
        preserved_location: preserved_location.clone(),
        kept_unsent: kept_unsent.clone(),
    };
    #[cfg(test)]
    sign_out_seam("after_keep").map_err(&kept_with)?;
    // Task 2191 (D11): the folder a Windows sign-out kept unknown bytes in, for the alert.
    #[cfg(target_os = "windows")]
    let mut windows_kept: Option<KeptUnsent> = None;
    #[cfg(not(target_os = "windows"))]
    let _ = (unknown_files, kept_folder);

    #[cfg(target_os = "windows")]
    {
        // Local teardown is authoritative even offline. The server revocation
        // runs after this block, on every OS (task 1886: it used to be here,
        // Windows only), best effort and bounded.
        //
        // The PURGE below runs on BOTH paths (already-signed-out included):
        // it is the cross-account safety invariant and must never be skipped.
        // Only the Cloud Files unregister (and, after this block, the server
        // revocation) is gated on actually having been signed in — when
        // already signed out there is no live registration of ours to remove
        // and no session to revoke.
        let root = if already_signed_out {
            DesktopConfig::load().ok().and_then(|cfg| cfg.sync_root)
        } else {
            DesktopConfig::load()?.sync_root
        };
        let db = state_db_from_app_local_state_dir()?;
        if let Some(db) = db {
            // Task 2191 (D11): files of unknown content are kept as copies or deleted, as the
            // person chose in the confirmation.
            let choice = match (unknown_files, root.as_deref()) {
                (UnknownFilesArg::Keep, Some(root)) => windows_cf::signout::UnknownFiles::Keep {
                    // Review-157 F4: the folder the confirmation named, when it is one of ours and
                    // outside the sync folder; else a fresh one. A sync folder at a drive root with
                    // no home folder has nowhere outside it, so the sign-out stops.
                    dir: kept_folder
                        .filter(|dir| {
                            !dir.starts_with(root)
                                && dir.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with("Beebeeb kept files "))
                        })
                        .or_else(|| kept_unknown_files_dir(root))
                        .ok_or_else(|| "Sign-out paused: there is no folder outside the sync folder to keep copies in.".to_string())?,
                },
                (UnknownFilesArg::Keep, None) | (UnknownFilesArg::Delete, _) => windows_cf::signout::UnknownFiles::Delete,
            };
            let kept = windows_cf::signout::purge(&db, root.as_deref(), &choice).map_err(|e| format!("Sign-out paused: {e}"))?;
            windows_kept = kept.dir.map(|dir| KeptUnsent {
                folder: dir.to_string_lossy().into_owned(),
                files: kept.files,
            });
        } else if let Some(root) = root.as_ref() {
            if std::fs::read_dir(root)
                .map_err(|e| format!("Cannot inspect sync folder: {e}"))?
                .next()
                .is_some()
            {
                return Err("Cannot verify sync-folder ownership without its database. Restore the local state before signing out.".into());
            }
        }
        // Task 2191 (review thread on #157): from here on the purge may have kept copies and
        // deleted their originals, so every failure names their folder too.
        let failed_after_purge =
            |message: String| sign_out_failure_after_purge(message, &preserved_location, &kept_unsent, &windows_kept);
        if !already_signed_out {
            if let Some(root) = root {
                // Cloud Files must release the root before WinRT removes its shell registration.
                windows_cf::unregister_sync_root(&root)
                    .map_err(|e| failed_after_purge(format!("Could not unregister Cloud Files: {e}")))?;
                windows_cf::unregister_shell_sync_root(&root)
                    .map_err(|e| failed_after_purge(format!("Could not remove Explorer registration: {e}")))?;
            }
        }
    }
    // Task 2191 (review thread on #157): every later failure carries the folder the purge kept
    // copies in, as the success does.
    #[cfg(target_os = "windows")]
    let kept_with = |message: String| sign_out_failure_after_purge(message, &preserved_location, &kept_unsent, &windows_kept);

    // Task 1538 findings 1+2: purge every queued operation (so a later
    // account's engine can never drain THIS account's still-queued uploads —
    // `operation_queue` carries no account scoping, see
    // `state_db::StateDb::purge_all_local_state`) and every decrypted cache
    // file this device holds for this account (so it can't be permanently
    // orphaned on disk once the next account's first sync prunes the DB row
    // that pointed at it). Per file it stays best-effort (the DB rows are
    // cleared regardless). A failure of the purge itself stops the sign-out
    // (R10, spec 2026-10-06 §5.6).
    //
    // Task 1538 Codex P1 (lib.rs:1085 thread): `db` is resolved from the
    // app-local state dir DIRECTLY — `state_db_from_app_local_state_dir`,
    // not the old `DesktopConfig::load().ok().and_then(state_db_for_config)`
    // chain — because `state.db` lives there independently of
    // `desktop.toml`/`sync_root`. A missing, corrupt, or unreadable config
    // (or one whose `sync_root` was rejected on load) must never silently
    // skip this security purge. `sync_root` itself is still read from
    // `DesktopConfig` best-effort, separately — it is used ONLY to resolve
    // Windows placeholder paths below, never to gate whether the purge runs
    // at all.
    #[cfg(not(target_os = "windows"))]
    {
        let cfg_sync_root = DesktopConfig::load().ok().and_then(|cfg| cfg.sync_root);
        // On a Mac a removal the reconciler could not confirm leaves the domain registered: that is a debt no
        // engine may start before.
        #[cfg(target_os = "macos")]
        let owe_removal = finder_cleanup.is_err();
        #[cfg(not(target_os = "macos"))]
        let owe_removal = false;
        // Test builds only: every sign-out test shares the one scratch state directory the purge below works on, so
        // two purges at once can find the database held ("database is locked"). They take turns here (a test that
        // already holds the gate for its whole body says so, and does not wait for itself).
        #[cfg(test)]
        let _one_purge_at_a_time = if PURGE_GATE_HELD.try_with(|_| ()).is_ok() {
            None
        } else {
            Some(SIGN_OUT_TEST_PURGE_GATE.lock().await)
        };
        purge_local_data_for_sign_out(
            state_paths::beebeeb_state_dir(),
            cfg_sync_root.as_deref(),
            owe_removal,
            kept.as_ref(),
        )
        .map_err(&kept_with)?;
        // The purge cleared the rows, and the closed accepts with them.
        #[cfg(target_os = "macos")]
        reopen_accepts.disarm();
        #[cfg(test)]
        sign_out_seam("after_purge").map_err(&kept_with)?;
    }

    // (macOS: the Finder removal is the reconciler's, at the start of this function: spec 2026-10-06
    // §5.3 (5). A logged-out Mac has no File Provider domain pointing at a folder the person is no
    // longer signed into, and no second, direct removal runs here.)
    // Task 1670 round 2: also the account-switch boundary — this codebase's
    // sign-out IS its account-switch mechanism (single active-account slot,
    // see `AppState::active_account`'s own "Phase 0" comment), so there is no
    // separate switch-account hook to add this to.
    purge_macos_hydrate_cache("sign-out");

    // Every path holds the engine slot by now (it took the slot to stop the engine), except the Windows
    // already-signed-out path, which stopped nothing and holds none: it takes the slot here, only to clear under it.
    // Task 1886: the local clear is one step (`cleared`), so the sessions it took are ended on the server only after
    // it, with its result known and the engine slot released.
    let cleared: Result<SignedOut, SignOutFailure> = async {
        let engine_slot = match held_engine_slot.take() {
            Some(slot) => slot,
            None => acct.engine.lock().await,
        };
        let in_memory =
            clear_session_holding_slot(&acct, &engine_slot, account::SessionTransition::SignOut).map_err(&kept_with)?;
        dropped.add(in_memory);
        // Windows: only now may a start waiting for the engine slot get it, and it finds no keys. On macOS and Linux the
        // slot is held through the last turn below (`finish_sign_out_turn`).
        #[cfg(target_os = "windows")]
        drop(engine_slot);
        // Drop the cached account profile so a subsequent `account_profile` IPC
        // can't cache-hit a stale (logged-out) identity. A fresh login repopulates it.
        clear_cached_profile(state);
        // And the storage summary (task 1887's rule: nothing of the account survives sign-out). The session is gone by
        // now, so no new fetch can start with its token; one still running answers into nothing (#139 review P2-3).
        popover_data::clear_storage(&state.popover_storage);
        // Sign out = clean slate: drop any in-flight 2FA challenge so an abandoned
        // partial token can't linger (and is zeroized) past a logout.
        if let Ok(mut guard) = state.pending_2fa.lock() {
            *guard = None;
        }
        if let Ok(mut guard) = acct.engine_state.lock() {
            *guard = "stopped".to_string();
        }
        // Sign out = clean slate for the auth-health streak too (task 1546
        // finding 5): the engine that was feeding it just stopped, and a fresh
        // sign-in must not inherit a stale `auth_expired: true` banner.
        acct.auth_health.note_result(None);
        if already_signed_out {
            // Already signed out: after a startup 401 (R9) the vault key and the email can still be here. The clear is
            // idempotent, but on a store that cannot answer (e.g. the Linux
            // fail-closed stub) an error must not turn an already-signed-out
            // no-op into a failure — log it and go on to the check (macOS/Linux: inside the last turn).
            #[cfg(not(target_os = "windows"))]
            {
                finish_sign_out_turn(&acct, &engine_slot, true, &mut dropped).map_err(&kept_with)?;
                // Only now may a start waiting for the engine slot get it, and it finds no keys.
                drop(engine_slot);
            }
            #[cfg(target_os = "windows")]
            if let Err(error) = clear_keychain_session_into(acct.id.as_str(), &mut dropped) {
                tracing::warn!(
                    %error,
                    "already-signed-out sign-out: keychain session clear failed (nothing should be left); continuing"
                );
            }
            set_auth_present(state, false);
            set_auth_email(state, None);
            state.sign_out.snapshots.clear();
            return Ok(SignedOut {
                outcome: SignOutOutcome::NotSignedIn,
                warning: removal_warning,
                preserved_location: preserved_location.clone(),
                #[cfg(target_os = "windows")]
                kept_unsent: kept_unsent.clone().or_else(|| windows_kept.clone()),
                #[cfg(not(target_os = "windows"))]
                kept_unsent: kept_unsent.clone(),
            });
        }
        // macOS/Linux: the Keychain clear and its check in the sign-out's last turn, which moves the generation on after them.
        #[cfg(not(target_os = "windows"))]
        {
            finish_sign_out_turn(&acct, &engine_slot, false, &mut dropped).map_err(&kept_with)?;
            // Only now may a start waiting for the engine slot get it, and it finds no keys.
            drop(engine_slot);
        }
        #[cfg(target_os = "windows")]
        clear_keychain_session_into(acct.id.as_str(), &mut dropped).map_err(&kept_with)?;
        set_auth_present(state, false);
        set_auth_email(state, None);
        lifecycle_log::event(lifecycle_log::LifecycleEvent::SignedOut);
        // Everything this sign-out can clear is cleared, so it happened; a removal from Finder it could not confirm is a
        // warning on that success, not an error (FB-24; lead rulings T8-⚠lock 1 and T7-⚠2 5).
        state.sign_out.snapshots.clear();
        Ok(SignedOut {
            outcome: SignOutOutcome::Completed,
            warning: removal_warning,
            preserved_location: preserved_location.clone(),
            #[cfg(target_os = "windows")]
            kept_unsent: kept_unsent.clone().or_else(|| windows_kept.clone()),
            #[cfg(not(target_os = "windows"))]
            kept_unsent: kept_unsent.clone(),
        })
    }
    .await;

    // Task 1886 (P0): the sessions this sign-out took off this computer end on the server too, on every OS (it was
    // Windows only, and only for the token it began with, so a Mac's or a Linux computer's signed-out token kept
    // working on the server for up to 30 days). Here: after the local clear, whatever its result (a clear that failed
    // ends only what this computer no longer holds), with the engine slot released. Best effort and bounded: whatever
    // the server answers, or if it does not, the sign-out's result stands and no token is logged.
    end_dropped_sessions(&acct, dropped, cleared.is_ok()).await;
    cleared
}

/// Sign-out's engine stop: take the engine slot and stop whatever engine holds it, through the one stop helper
/// (`stop_engine_in_slot`, called by [`stop_engine_for_sign_out`]). Returns the slot, still held, so the session is
/// cleared before any start can take it. Refuses (with the slot released and nothing cleared) when an earlier stop was
/// never confirmed, when this stop cannot be confirmed, or when the Windows Cloud Files revocation failed. Both
/// sign-out paths use it (Task 12 fix round 2).
async fn take_slot_and_stop_engine_for_sign_out(
    acct: &AccountRuntime,
) -> Result<tokio::sync::MutexGuard<'_, Option<EngineRunner>>, String> {
    let mut engine_slot = acct.engine.lock().await;
    #[cfg(all(test, target_os = "macos"))]
    signal_sign_out_holds_the_engine_slot();
    if let Err(refusal) = stop_engine_for_sign_out(
        acct,
        &mut engine_slot,
        upload_staging_dir_for_session_purge().as_deref(),
    )
    .await
    {
        drop(engine_slot);
        return Err(refusal);
    }
    Ok(engine_slot)
}

/// Sign-out's engine stop on the held slot: the gate in front of every purge sign-out runs.
/// Refuses (Bug A / A2) unless the engine task is CONFIRMED terminated, or no
/// engine is running and no earlier attempt left an unconfirmed one behind.
///
/// Then upload staging: every handed-over copy is removed. A request that arrives later reaches no daemon and fails
/// transiently. One Finder write or change may still be running: a save, a rename, a move or a folder create in
/// `spawn_blocking`, or a delete's synchronous enqueue, which an abort cannot stop once it started. The keep step
/// closes accepts before it reads the queue, each of them reads that in the transaction that queues it, and a
/// sign-out that stops reopens them (spec 2026-10-10 §5.2 step 6, §15 question 3). The caller holds the engine slot,
/// so no new engine can start in between: no engine start reopens accepts between the keep step and the purge.
async fn stop_engine_for_sign_out(
    acct: &crate::account::AccountRuntime,
    engine_slot: &mut Option<runner::EngineRunner>,
    upload_staging: Option<&std::path::Path>,
) -> Result<(), String> {
    // Stop the engine before dropping memory so the IPC listener cannot accept
    // new File Provider operations with a cloned master key.
    //
    // Task 1538 Codex P1 (PR #49, lib.rs:1087 thread): `abort()` reports
    // whether the engine's task is CONFIRMED terminated, not just "we asked
    // and waited a bit". The purge below is a cross-account data-
    // exfiltration control (findings 1+2) — it is only safe to run once the
    // OLD engine (and everything nested in its single task: the IPC socket
    // server, the Windows upload watcher) is genuinely gone and can no
    // longer drain or enqueue operations behind its back. If we can't
    // confirm that, refuse to complete sign-out rather than purge anyway
    // and hand the next account's engine a false sense of a clean slate.
    //
    // Bug A2: a refused attempt consumes the engine handle, leaving the
    // slot empty — without the `engine_stop_unconfirmed` flag below, a
    // RETRY would see an idle slot, skip this gate entirely, and purge
    // while the old engine may still be running. The flag (per-account,
    // in-memory) keeps the gate closed until the process restarts, which
    // is exactly what the error message tells the user to do.
    if acct.engine_stop_unconfirmed.load(std::sync::atomic::Ordering::SeqCst) {
        tracing::error!(
            "sign-out refused: a previous attempt could not confirm the sync engine \
             stopped; restart Beebeeb before signing in with a different account"
        );
        return Err(UNCONFIRMED_ENGINE_STOP_ERROR.to_string());
    }
    if let Some(prev) = engine_slot.take() {
        match stop_engine_in_slot(acct, prev).await {
            runner::AbortOutcome::Stopped => {
                tracing::info!("engine aborted on logout");
            }
            runner::AbortOutcome::RevokeFailed { stage, source } => {
                // Bug A misattribution fix: the engine task itself IS
                // confirmed stopped here — what failed is the Windows
                // Cloud Files callback revocation. Say so, instead of
                // blaming the sync engine.
                tracing::error!(
                    stage,
                    %source,
                    "sign-out refused: Cloud Files revocation failed after the engine task itself stopped"
                );
                return Err(format!(
                    "Cloud Files revocation failed ({stage}); the sync engine itself stopped. \
                     Please try signing out again; if this keeps happening, restart Beebeeb \
                     before signing in with a different account. ({source})"
                ));
            }
            runner::AbortOutcome::TaskUnconfirmed => {
                // (`stop_engine_in_slot` has set `engine_stop_unconfirmed`, before the slot is released.)
                tracing::error!(
                    "sign-out refused: could not confirm the sync engine stopped; \
                     refusing to purge local state or clear credentials while it may still be running"
                );
                return Err(UNCONFIRMED_ENGINE_STOP_ERROR.to_string());
            }
        }
    }
    // Every unconfirmed or failed stop returned above: no engine, and so no
    // IPC listener, is left.
    purge_upload_staging_after_engine_stop("sign-out", true, upload_staging);
    Ok(())
}

/// The turn-taking for the purge in the sign-out tests (see `clear_session_impl`). Test builds only.
#[cfg(all(test, not(target_os = "windows")))]
static SIGN_OUT_TEST_PURGE_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[cfg(all(test, not(target_os = "windows")))]
tokio::task_local! {
    /// Set inside [`with_the_shared_state_dir`]: this task already holds the gate, so its own sign-out must not wait for it.
    static PURGE_GATE_HELD: ();
}

/// Run `body` holding the gate the sign-out purge takes. For a test that works in the shared scratch state directory
/// itself (it seeds the database before it signs out, and checks it after): from its first write to its last check, no
/// other test's purge runs. Its own sign-out sees the marker and does not wait for the gate it holds.
#[cfg(all(test, not(target_os = "windows")))]
async fn with_the_shared_state_dir<T>(body: impl std::future::Future<Output = T>) -> T {
    let _gate = SIGN_OUT_TEST_PURGE_GATE.lock().await;
    PURGE_GATE_HELD.scope((), body).await
}

/// The local-data purge of a sign-out on macOS and Linux, with the state dir passed in (a unit test points it at a
/// throwaway one; `clear_session_impl` passes the app's). Purge the queue, the staged payloads and the cache files,
/// then clear EVERY account row and the owner, so the next sign-in finds a clean database instead of leftovers
/// (fix round 1 of Task 10). `owe_finder_removal`: the sign-out's Finder removal was not confirmed, so a removal is
/// owed before any engine starts. Any failure stops the sign-out: a purge that cannot run (state dir unresolved,
/// database unopenable), one that fails (a file that cannot be removed), a failed clear. A state.db that does not
/// exist yet is a fresh install: nothing to purge.
///
/// `kept` (macOS, spec 2026-10-10 §5.2 step 8): what the keep step kept. The purge refuses before it deletes anything
/// unless every waiting change is in it, and that refusal (`unsent_copy_failed: …`) is returned as it is, so the caller
/// can match its code. Once the rows are cleared, the unfinished sign-out marker is cleared too (W13).
#[cfg(not(target_os = "windows"))]
fn purge_local_data_for_sign_out(
    state_dir: Result<PathBuf, String>,
    sync_root: Option<&Path>,
    owe_finder_removal: bool,
    kept: Option<&sign_out::kept_set::KeptSet>,
) -> Result<(), String> {
    match state_dir.and_then(|dir| state_db_from_state_dir(&dir)) {
        Ok(Some(db)) => match purge_local_state_files(&db, sync_root, kept) {
            Ok(summary) => {
                tracing::info!(
                    queued_ops_purged = summary.queued_ops_purged,
                    files_removed = summary.files_removed,
                    files_skipped = summary.files_skipped,
                    windows_placeholders_dehydrated = summary.windows_placeholders_dehydrated,
                    "purged operation queue and decrypted cache on sign-out"
                );
                // Spec 2026-10-10 W13: the rows are cleared, so no write can look missing any more. Best effort
                // (§7.3). The warning names the step that failed, `path`, `read` or `write`, and never an error's
                // text: a read error quotes a line of the file, and the list's lines are kept folders' paths (W10).
                #[cfg(target_os = "macos")]
                if let Err(step) = DesktopConfig::path()
                    .map_err(|_| "path".to_string())
                    .and_then(|path| sign_out::kept_record::clear_unfinished_at(&path))
                {
                    tracing::warn!(step = %step, "could not clear the unfinished sign-out marker");
                }
                if let Err(error) = db.clear_account_data(owe_finder_removal) {
                    tracing::error!(error = %error, "sign-out stopped: the account rows could not be cleared");
                    return Err(format!("{SIGN_OUT_PURGE_FAILED} ({error})"));
                }
                Ok(())
            }
            // The guard's refusal is E2 with its code, so the caller can match it (spec §11).
            Err(error) if error.starts_with(&format!("{}: ", sign_out::copy::UNSENT_COPY_FAILED_CODE)) => Err(error),
            Err(error) => {
                tracing::error!(error = %error, "sign-out stopped: the local data could not be purged");
                Err(format!("{SIGN_OUT_PURGE_FAILED} ({error})"))
            }
        },
        // No state.db yet: a fresh install that never synced anything. Nothing to purge; not an error.
        Ok(None) => Ok(()),
        Err(error) => {
            tracing::error!(error = %error, "sign-out stopped: the local state database could not be opened");
            Err(format!("{SIGN_OUT_PURGE_FAILED} ({error})"))
        }
    }
}

/// Spec 2026-10-10 §5.1 step 1 without the send conditions (the send phase adds them): one read; the earlier folders
/// only when the ask shows a missing write. A failed read of the queue is E3, never a 0. A failed read of the
/// unfinished list leaves the earlier folders out and the ask still counts; its warning names the step that failed,
/// never the error: a read error quotes a line of the file, and the list's lines are kept folders' paths (W10).
#[cfg(target_os = "macos")]
fn count_ask(owner: &str) -> Result<sign_out::snapshot::AskSnapshot, String> {
    let e3 = || {
        sign_out::copy::coded(
            sign_out::copy::UNSENT_COUNT_FAILED_CODE,
            sign_out::copy::E3_UNSENT_COUNT_FAILED,
        )
    };
    let waiting = read_waiting_changes().map_err(|_| e3())?;
    let earlier = if waiting.counts().missing_files > 0 {
        DesktopConfig::path()
            .map_err(|_| "path")
            .and_then(|path| sign_out::kept_record::unfinished_at(&path).map_err(|_| "read"))
            .unwrap_or_else(|step| {
                tracing::warn!(
                    step = %step,
                    "the ask names no earlier folder: the unfinished sign-out list could not be read"
                );
                Vec::new()
            })
    } else {
        Vec::new()
    };
    Ok(sign_out::snapshot::AskSnapshot::of(&waiting, owner, earlier))
}

/// The queue as the sign-out counts it, from the app's state dir. No `state.db` yet: nothing waits.
#[cfg(target_os = "macos")]
fn read_waiting_changes() -> Result<sign_out::waiting::WaitingChanges, String> {
    match state_db_from_state_dir(&state_paths::beebeeb_state_dir()?)? {
        Some(db) => sign_out::waiting::WaitingChanges::read(&db),
        None => Ok(sign_out::waiting::WaitingChanges::default()),
    }
}

/// §5.1, the contract: `sign_out_count`'s answer. Built on a Mac only; elsewhere the command's twin answers an error.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SignOutCount {
    #[serde(flatten)]
    counts: sign_out::snapshot::AskCounts,
    can_send: bool,
    not_sending: Option<sign_out::waiting::NotSending>,
}

/// §6.1 conditions 1-4 for the active account, and the running engine's phase link.
#[cfg(target_os = "macos")]
async fn send_conditions(state: &AppState) -> (sign_out::waiting::SendConditions, Option<Arc<runner::SendPhaseLink>>) {
    let Ok(acct) = state.active_account() else {
        return (
            sign_out::waiting::SendConditions {
                engine_running: false,
                vault_unlocked: false,
                session_expired: true,
                link_failed: false,
                sync_paused: false,
            },
            None,
        );
    };
    // Running, not merely present: a runner whose task ended at startup leaves its handle in the slot, and that must
    // not advertise a send nobody would run (the count then goes straight to the engine-stopped ask).
    let link = acct
        .engine
        .lock()
        .await
        .as_ref()
        .filter(|engine| engine.is_running())
        .map(|engine| engine.send_phase());
    let conditions = sign_out::waiting::SendConditions {
        engine_running: link.is_some(),
        vault_unlocked: acct.session.lock().map(|session| session.is_some()).unwrap_or(false),
        session_expired: acct.auth_health.is_expired(),
        link_failed: link.as_ref().is_some_and(|link| link.link_failed()),
        sync_paused: acct.sync_paused.load(Ordering::Relaxed),
    };
    (conditions, link)
}

/// §5.1 step 1: one read, a snapshot owned by the calling window, and one `info` line with counts only (review I8).
#[cfg(target_os = "macos")]
#[tauri::command]
async fn sign_out_count(window: tauri::WebviewWindow, state: State<'_, AppState>) -> Result<SignOutCount, String> {
    let snapshot = count_ask(window.label())?;
    let (conditions, _) = send_conditions(&state).await;
    let not_sending = sign_out::waiting::readiness(conditions, snapshot.is_empty(), snapshot.any_live);
    let can_send = !snapshot.is_empty() && not_sending.is_none();
    tracing::info!(
        files = snapshot.counts.files,
        copied_files = snapshot.counts.copied_files,
        others = snapshot.counts.others,
        missing_files = snapshot.counts.missing_files,
        can_send,
        not_sending = not_sending.map(|reason| format!("{reason:?}")).unwrap_or_default(),
        earlier_folder = !snapshot.counts.earlier_folders.is_empty(),
        "sign-out flow counted"
    );
    let counts = snapshot.counts.clone();
    state.sign_out.snapshots.record(snapshot);
    Ok(SignOutCount {
        counts,
        can_send,
        not_sending,
    })
}

/// Windows and Linux keep today's sign-out (spec 2026-10-10 W5): the flow's commands are registered everywhere,
/// because `generate_handler!` takes no `cfg`, and say so off a Mac.
#[cfg(not(target_os = "macos"))]
#[tauri::command]
async fn sign_out_count(window: tauri::WebviewWindow, state: State<'_, AppState>) -> Result<SignOutCount, String> {
    let _ = (window, state);
    Err("This works on a Mac only.".into())
}

/// W11: on macOS the switch step renders a snapshot, owned by the window the step runs in: the Beebeeb window (`main`),
/// which the app destroys on a close, so a close drops it (task 1747 Part 1, review P3-5 on #143; it was owned by the label
/// `onboarding`, a window a Mac no longer builds). Its one caller, `settle_sign_in`, is not built on Windows.
#[cfg_attr(target_os = "windows", allow(dead_code))]
fn switch_snapshot(state: &AppState) -> Result<Option<sign_out::snapshot::AskCounts>, String> {
    #[cfg(target_os = "macos")]
    {
        let snapshot = count_ask(surfaces::policy::main_window_label(surfaces::policy::Platform::current()))?;
        // §6.4 (review I8): the switch's snapshot logs the same line as `sign_out_count`; it never sends (W11).
        tracing::info!(
            files = snapshot.counts.files,
            copied_files = snapshot.counts.copied_files,
            others = snapshot.counts.others,
            missing_files = snapshot.counts.missing_files,
            can_send = false,
            not_sending = "",
            earlier_folder = !snapshot.counts.earlier_folders.is_empty(),
            "sign-out flow counted"
        );
        let counts = snapshot.counts.clone();
        state.sign_out.snapshots.record(snapshot);
        Ok(Some(counts))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = state;
        Ok(None)
    }
}

/// What the driver needs of a started phase.
#[cfg(target_os = "macos")]
struct PhaseRun {
    owner: String,
    link: Option<Arc<runner::SendPhaseLink>>,
    stop: Arc<AtomicBool>,
    closed: Arc<AtomicBool>,
    started: std::time::Instant,
    changes_at_start: usize,
}

/// §6.2: a stale ask refuses; a running phase starts nothing; otherwise make live changes due, arm, and register.
#[cfg(target_os = "macos")]
fn start_send_phase(
    sign_out: &sign_out::SignOutState,
    ask_id: &str,
    owner: &str,
    link: Option<Arc<runner::SendPhaseLink>>,
) -> Result<Option<PhaseRun>, String> {
    if sign_out.snapshots.newest_ask_id().as_deref() != Some(ask_id) {
        return Err(sign_out::copy::coded(
            sign_out::copy::UNSENT_CHANGES_CODE,
            sign_out::copy::E1_UNSENT_CHANGES,
        ));
    }
    let mut phase = sign_out.phase.lock().unwrap_or_else(|p| p.into_inner());
    if phase.is_some() {
        return Ok(None);
    }
    let changes_at_start = read_waiting_changes().map(|waiting| waiting.changes()).unwrap_or(0);
    if let Ok(Some(db)) = state_paths::beebeeb_state_dir().and_then(|dir| state_db_from_state_dir(&dir)) {
        let _ = db.make_due_for_send_phase(now_unix_seconds());
    }
    if let Some(link) = &link {
        link.arm();
    }
    let (stop, closed) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
    *phase = Some(sign_out::send_phase::RunningPhase {
        ask_id: ask_id.to_string(),
        owner: owner.to_string(),
        stop: stop.clone(),
        closed: closed.clone(),
    });
    Ok(Some(PhaseRun {
        owner: owner.to_string(),
        link,
        stop,
        closed,
        started: std::time::Instant::now(),
        changes_at_start,
    }))
}

/// The phase is over, for any reason: disarm, so the runner is back on its tick within a second.
#[cfg(target_os = "macos")]
fn end_send_phase(sign_out: &sign_out::SignOutState, link: Option<&runner::SendPhaseLink>) {
    if let Some(link) = link {
        link.disarm();
    }
    *sign_out.phase.lock().unwrap_or_else(|p| p.into_inner()) = None;
}

#[cfg(target_os = "macos")]
#[tauri::command]
async fn sign_out_send_start(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    ask_id: String,
) -> Result<(), String> {
    let (_, link) = send_conditions(&state).await;
    if let Some(run) = start_send_phase(&state.sign_out, &ask_id, window.label(), link)? {
        tauri::async_runtime::spawn(drive_send_phase(app, run));
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
async fn sign_out_send_start(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    ask_id: String,
) -> Result<(), String> {
    let _ = (app, window, state, ask_id);
    Err("This works on a Mac only.".into())
}

/// "Stop waiting" (and Esc): the phase ends `stopped`. Nothing happens when no phase runs.
#[cfg(target_os = "macos")]
#[tauri::command]
fn sign_out_send_stop(state: State<'_, AppState>) -> Result<(), String> {
    if let Some(phase) = state.sign_out.phase.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
        phase.stop.store(true, Ordering::SeqCst);
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
fn sign_out_send_stop(state: State<'_, AppState>) -> Result<(), String> {
    let _ = state;
    Err("This works on a Mac only.".into())
}

/// One observation of the world for the phase (§6.4): the queue as read, the account's conditions, and what the
/// phase's attempts met on its link. The driver's own step, split out so a test can run it with no window.
#[cfg(target_os = "macos")]
fn observe_phase(
    run: &PhaseRun,
    waiting: &sign_out::waiting::WaitingChanges,
    conditions: sign_out::waiting::SendConditions,
) -> sign_out::send_phase::PhaseObservation {
    sign_out::send_phase::PhaseObservation {
        elapsed: run.started.elapsed(),
        changes_left: waiting.changes(),
        any_live: waiting.any_live(),
        engine_running: run.link.is_some() && conditions.engine_running,
        session_expired: conditions.session_expired,
        link_failed: conditions.link_failed,
        attempt: run.link.as_ref().map(|link| link.signals()).unwrap_or_default(),
        stop_requested: run.stop.load(Ordering::SeqCst),
        window_closed: run.closed.load(Ordering::SeqCst),
    }
}

/// The driver: once a second, wake the runner's queue step, read the world, and end or report progress.
#[cfg(target_os = "macos")]
async fn drive_send_phase(app: tauri::AppHandle, run: PhaseRun) {
    let state = app.state::<AppState>();
    let mut phase = sign_out::send_phase::SendPhase::default();
    let mut changes_left = run.changes_at_start;
    let end = loop {
        if let Some(link) = &run.link {
            link.wake();
        }
        // `std::time::Duration`: `lib.rs` imports none at the crate level.
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let Ok(waiting) = read_waiting_changes() else {
            break sign_out::send_phase::PhaseEnd::Stopped;
        };
        changes_left = waiting.changes();
        let (conditions, _) = send_conditions(&state).await;
        let observation = observe_phase(&run, &waiting, conditions);
        match phase.observe(&observation) {
            sign_out::send_phase::PhaseStep::Continue => {}
            sign_out::send_phase::PhaseStep::Progress(left) => {
                let _ = app.emit_to(
                    run.owner.as_str(),
                    "sign-out-send-progress",
                    serde_json::json!({ "changesLeft": left }),
                );
            }
            sign_out::send_phase::PhaseStep::End(end) => break end,
        }
    };
    end_send_phase(&state.sign_out, run.link.as_deref());
    use sign_out::send_phase::PhaseEnd;
    let ask = match end {
        PhaseEnd::AllSent | PhaseEnd::Closed => None,
        _ => count_ask(&run.owner).ok().map(|snapshot| {
            let counts = snapshot.counts.clone();
            state.sign_out.snapshots.record(snapshot);
            counts
        }),
    };
    tracing::info!(
        reason = end.as_str(),
        changes_at_start = run.changes_at_start,
        changes_left,
        elapsed_ms = run.started.elapsed().as_millis() as u64,
        "sign-out send phase ended"
    );
    let _ = app.emit_to(
        run.owner.as_str(),
        "sign-out-send-ended",
        serde_json::json!({ "reason": end, "ask": ask }),
    );
}

/// W5, the gate: one read compared with the snapshot `keep_unsent` names.
#[cfg(target_os = "macos")]
fn gate_sign_out(
    state: &AppState,
    keep_unsent: Option<&str>,
) -> Result<Option<sign_out::snapshot::AskSnapshot>, SignOutFailure> {
    let waiting = read_waiting_changes().map_err(|_| {
        SignOutFailure::from(sign_out::copy::coded(
            sign_out::copy::UNSENT_COUNT_FAILED_CODE,
            sign_out::copy::E3_UNSENT_COUNT_FAILED,
        ))
    })?;
    state.sign_out.snapshots.gate(&waiting, keep_unsent).map_err(|_| {
        SignOutFailure::from(sign_out::copy::coded(
            sign_out::copy::UNSENT_CHANGES_CODE,
            sign_out::copy::E1_UNSENT_CHANGES,
        ))
    })
}

/// Where the kept folder goes: the container's `Documents/Changes not sent` (§7.1). A test build writes under its
/// per-process sandbox, never the person's folders.
#[cfg(target_os = "macos")]
fn sign_out_kept_root() -> Result<PathBuf, String> {
    #[cfg(test)]
    {
        crate::test_sandbox::dir("kept")
            .map(|dir| dir.join("Changes not sent"))
            .map_err(|e| e.to_string())
    }
    #[cfg(not(test))]
    {
        sign_out::kept_names::kept_root_for_this_app().ok_or_else(|| "no folder for kept changes".to_string())
    }
}

/// Who runs the keep step. A failed copy's log line names it: an R10 reset is not a sign-out (W12).
#[cfg(target_os = "macos")]
#[derive(Clone, Copy)]
enum KeepCaller {
    /// §5.2 step 7: a sign-out by choice.
    SignOut,
    /// W12: spec A's R10 reset before an engine starts.
    Reset,
}

/// The keep step on `db` (§5.2 step 7, W12): the kept root, the config, the clone and the durable save are the app's.
/// A failed copy is logged by its category, never a path (W10), in the caller's own line.
#[cfg(target_os = "macos")]
fn keep_in(
    db: &state_db::StateDb,
    acknowledge: sign_out::keep::Acknowledge<'_>,
    caller: KeepCaller,
) -> Result<sign_out::kept_set::KeptSet, String> {
    let failed = || {
        sign_out::copy::coded(
            sign_out::copy::UNSENT_COPY_FAILED_CODE,
            sign_out::copy::E2_UNSENT_COPY_FAILED,
        )
    };
    let root = sign_out_kept_root().map_err(|_| failed())?;
    let config = DesktopConfig::path().map_err(|_| failed())?;
    sign_out::keep::keep_unsent(sign_out::keep::KeepInputs {
        db,
        acknowledge,
        root: &root,
        event_name: sign_out::kept_names::event_folder_name(chrono::Local::now()),
        config_path: &config,
        clone: &sign_out::kept_writer::SystemClone,
        config_fs: &config::RealConfigFs,
    })
    .map_err(|failure| {
        if let sign_out::keep::KeepFailure::Copy(reason) = &failure {
            match caller {
                KeepCaller::SignOut => tracing::error!(
                    reason,
                    "sign-out stopped: the waiting changes could not be kept; nothing was purged"
                ),
                KeepCaller::Reset => tracing::error!(
                    reason,
                    "R10: local data reset stopped: the waiting changes could not be kept; nothing was purged"
                ),
            }
        }
        failure.message()
    })
}

/// §5.2 step 7: the keep step on the app's state dir. `None`: no `state.db` yet, nothing to keep. A state dir that does
/// not resolve, or a `state.db` that does not open, means the queue cannot be read: E3 with its code, as a failed read
/// inside the keep step is. Its log names the step, never the error, whose text names the database's path (W10).
#[cfg(target_os = "macos")]
fn keep_for_sign_out(
    accepted: Option<&sign_out::snapshot::AskSnapshot>,
) -> Result<Option<sign_out::kept_set::KeptSet>, String> {
    use sign_out::keep::Acknowledge;
    let opened = state_paths::beebeeb_state_dir()
        .map_err(|_| "state_dir")
        .and_then(|dir| state_db_from_state_dir(&dir).map_err(|_| "state_db"));
    let db = match opened {
        Ok(Some(db)) => db,
        Ok(None) => return Ok(None),
        Err(step) => {
            tracing::error!(
                step = %step,
                "sign-out stopped: the waiting changes could not be read; nothing was purged"
            );
            return Err(sign_out::copy::coded(
                sign_out::copy::UNSENT_COUNT_FAILED_CODE,
                sign_out::copy::E3_UNSENT_COUNT_FAILED,
            ));
        }
    };
    let acknowledge = match accepted {
        Some(snapshot) => Acknowledge::Asked {
            others: &snapshot.others,
            missing: &snapshot.missing,
        },
        None => Acknowledge::Nothing,
    };
    keep_in(&db, acknowledge, KeepCaller::SignOut).map(Some)
}

/// Spec 2026-10-10 §15 question 3: reopen Finder accepts in the app's state database. Best effort: an engine start
/// reopens them too (`engine_start_repair`). No `state.db` yet: nothing was closed. One that does not open is logged
/// by its step, never by the error, whose text names the database's path (W10).
#[cfg(target_os = "macos")]
fn reopen_finder_accepts_in(state_dir: &Path) {
    let db = match state_db_from_state_dir(state_dir) {
        Ok(Some(db)) => db,
        Ok(None) => return,
        Err(_) => {
            accepts_not_reopened("state_db");
            return;
        }
    };
    if let Err(error) = db.reopen_finder_accepts() {
        tracing::warn!(%error, "could not reopen Finder accepts; the next engine start reopens them");
    }
}

/// A sign-out quit between its keep step and its purge left accepts closed on disk: the launch reopens them first.
#[cfg(target_os = "macos")]
fn reopen_finder_accepts_at_startup() {
    match state_paths::beebeeb_state_dir() {
        Ok(state_dir) => reopen_finder_accepts_in(&state_dir),
        Err(_) => accepts_not_reopened("state_dir"),
    }
}

/// A reopen that could not reach the state database: the step only (W10).
#[cfg(target_os = "macos")]
fn accepts_not_reopened(step: &'static str) {
    tracing::warn!(
        step = %step,
        "could not reopen Finder accepts; the next engine start reopens them"
    );
}

/// From a completed keep step until the purge has cleared the rows (and the key with them), any return reopens Finder
/// accepts at once: a refused or failed purge, a test seam, a panic.
#[cfg(target_os = "macos")]
struct AcceptsReopenOnAbort {
    armed: bool,
}

#[cfg(target_os = "macos")]
impl AcceptsReopenOnAbort {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

#[cfg(target_os = "macos")]
impl Drop for AcceptsReopenOnAbort {
    fn drop(&mut self) {
        if self.armed
            && let Ok(state_dir) = state_paths::beebeeb_state_dir()
        {
            reopen_finder_accepts_in(&state_dir);
        }
    }
}

/// Test builds only: a hook between the steps of `clear_session_impl`, per test task (`SIGN_OUT_SEAM.scope`). An
/// `Err` stops the sign-out at that point with that message.
#[cfg(test)]
type SignOutSeam = Arc<dyn Fn(&'static str) -> Result<(), String> + Send + Sync>;

#[cfg(test)]
tokio::task_local! {
    static SIGN_OUT_SEAM: SignOutSeam;
}

#[cfg(test)]
fn sign_out_seam(point: &'static str) -> Result<(), String> {
    SIGN_OUT_SEAM.try_with(|seam| seam(point)).unwrap_or(Ok(()))
}

/// R10: on macOS and Linux a sign-out whose local-data purge fails stops instead of completing, as
/// Windows already does. The engine is already stopped, so trying again is safe.
#[cfg(not(target_os = "windows"))]
const SIGN_OUT_PURGE_FAILED: &str = "Sign-out paused: Beebeeb couldn’t clear this computer’s local data. Try again; if it keeps happening, restart Beebeeb.";

/// `clear_session`'s `keepUnsent: { askId }` (spec 2026-10-10 §5.1, the contract). Accepted and ignored off macOS (W5).
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct KeepUnsentArg {
    ask_id: String,
}

/// Task 2191 (Decision (lead, 2026-10-26)): what the person chose in the Windows sign-out
/// confirmation for files of unknown content that hold bytes on this PC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum UnknownFilesArg {
    /// Keep a copy of each in a folder beside the sync folder (the default).
    Keep,
    /// Delete them with the rest.
    Delete,
}

/// Task 2191: what the Windows sign-out confirmation shows: how many files of unknown content
/// hold bytes on this PC, and the folder "Keep copies" would use.
#[derive(Debug, Clone, serde::Serialize)]
struct UnknownFilesAtSignOut {
    count: usize,
    folder: Option<String>,
}

/// Task 2191: the folder a Windows sign-out keeps unknown bytes in: beside the sync folder, or in
/// the home folder for a sync folder at a drive root. `None`: there is no place outside it.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn kept_unknown_files_dir(root: &std::path::Path) -> Option<std::path::PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(std::path::PathBuf::from);
    unknown_check::kept_files_dir(root, &chrono::Local::now().format("%Y-%m-%d %H%M").to_string(), home.as_deref())
}

/// Task 2191 (D11): the files the Windows sign-out would keep or delete as unknown: by the same
/// rule as the purge (`unknown_check::unknown_at_signout`), holding bytes on this PC.
#[cfg(target_os = "windows")]
fn count_unknown_files_at_sign_out() -> Result<UnknownFilesAtSignOut, String> {
    use unknown_check::{ExpectedAt, OnDisk, PlatformView};
    let root = DesktopConfig::load().ok().and_then(|cfg| cfg.sync_root);
    let (Some(db), Some(root)) = (state_db_from_app_local_state_dir()?, root) else {
        return Ok(UnknownFilesAtSignOut { count: 0, folder: None });
    };
    let mut places: Vec<(String, ExpectedAt)> = db
        .list_unknown_orphans()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|(path, _, _)| (path, ExpectedAt::OrphanCandidate))
        .collect();
    places.extend(db.list_guarded().map_err(|e| e.to_string())?.into_iter().map(|row| (row.path, ExpectedAt::Guarded(row.file_id))));
    let mut count = 0;
    for (rel, expected) in places {
        let Some(path) = engine_bridge::EngineBridge::placeholder_path_under(&root, &rel) else {
            continue;
        };
        let (placeholder, identity, holds_bytes) = match windows_cf::placeholders::platform_view(&path) {
            PlatformView::Missing | PlatformView::Unreadable => continue,
            PlatformView::PlainFile => (false, None, true),
            PlatformView::Placeholder { on_disk, identity, .. } => (true, identity, on_disk != OnDisk::Nothing),
        };
        let owner = match identity.as_deref() {
            Some(id) => db
                .get_file(id)
                .map_err(|e| e.to_string())?
                .map(|row| (row.path, db.has_agreed_content(id).unwrap_or(true))),
            None => None,
        };
        let unknown = unknown_check::unknown_at_signout(
            &expected,
            &rel,
            placeholder,
            identity.as_deref(),
            owner.as_ref().map(|(path, guarded)| (path.as_str(), *guarded)),
        );
        if unknown && holds_bytes {
            count += 1;
        }
    }
    let folder = kept_unknown_files_dir(&root).filter(|_| count > 0).map(|dir| dir.to_string_lossy().into_owned());
    Ok(UnknownFilesAtSignOut { count, folder })
}

/// Task 2191: how many files of unknown content hold bytes on this PC (Windows), for the
/// sign-out confirmation. Elsewhere: none.
#[tauri::command]
async fn sign_out_unknown_files() -> Result<UnknownFilesAtSignOut, String> {
    #[cfg(target_os = "windows")]
    {
        // Self-review: one platform read per file; off the async workers.
        tokio::task::spawn_blocking(count_unknown_files_at_sign_out)
            .await
            .map_err(|e| e.to_string())?
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(UnknownFilesAtSignOut { count: 0, folder: None })
    }
}

/// Sign out through the shared native-menu/WebView teardown. Windows returns
/// an error while work, plaintext cleanup or root unregistration is incomplete;
/// the UI must retain the account and display that error for recovery/retry.
/// `forget_email` (`forgetEmail` from the frontend): this sign-out is an account switch, so the sign-in prefill is
/// forgotten too (spec §5.6). Absent or false: the prefill is kept, as before.
/// `Ok` means the sign-out happened; its `warning` says what of it could not be confirmed (FB-24).
///
/// Task 1882: when the sign-out kept Finder files that had not reached the
/// server, the app's alert names the folder. Every frontend sign-out (Settings,
/// the compact Account page, "Sign in again", the account switch) ends here; the menu's sign-out
/// raises the same alert in its own handler.
///
/// Spec 2026-10-10 §5.1 "A late result" (macOS): Tauri passes the calling window. A result that needs showing stays
/// pending until that window acknowledges it (`sign_out_result_ack`); if the window closes, is destroyed or reloads
/// first, Rust shows it in the menu's dialog, and so it does when the call returns after such a close.
#[tauri::command]
async fn clear_session(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    forget_email: Option<bool>,
    keep_unsent: Option<KeepUnsentArg>,
    unknown_files: Option<UnknownFilesArg>,
    kept_folder: Option<String>,
) -> Result<SessionActionOutcome, String> {
    #[cfg(target_os = "macos")]
    state.sign_out.late_results.begin(window.label());
    let result = clear_session_impl(
        &state,
        forget_email.unwrap_or(false),
        keep_unsent.as_ref().map(|keep| keep.ask_id.as_str()),
        // Task 2191: without an answer (the menu, an older window) the copies are kept.
        unknown_files.unwrap_or(UnknownFilesArg::Keep),
        // Review-157 F4: the folder the confirmation named, so the copies go where it said.
        kept_folder.map(std::path::PathBuf::from),
    )
    .await;
    // Review I1: the alert comes first, also when the sign-out failed after the removal.
    // Review I2: and the folder is saved for the Settings › Sync row.
    surface_sign_out_folders(&app, &result);
    #[cfg(target_os = "macos")]
    if let Some(shown) = state
        .sign_out
        .late_results
        .complete(window.label(), late_result_of(&result))
    {
        show_late_result(&app, shown);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = &window;
    result
        .map(|done| SessionActionOutcome { warning: done.warning })
        .map_err(|failure| failure.message)
}

/// Spec 2026-10-10 §5.1 "A late result": what needs showing. Not `unsent_changes` (nothing changed, and a hidden window
/// cannot show the ask), and not a completed sign-out without a warning. Read on macOS and in tests only.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn late_result_of(result: &Result<SignedOut, SignOutFailure>) -> Option<sign_out::flow_window::Shown> {
    use sign_out::flow_window::Shown;
    match result {
        Err(failure)
            if failure
                .message
                .starts_with(&format!("{}: ", sign_out::copy::UNSENT_CHANGES_CODE)) =>
        {
            None
        }
        Err(failure) => Some(Shown::Failure(failure.message.clone())),
        Ok(SignedOut {
            warning: Some(warning), ..
        }) => Some(Shown::Warning(warning.sentence().to_string())),
        Ok(_) => None,
    }
}

/// The menu's own dialogs (its sign-out's "Sign-out paused" and "Signed out"), in the callback form, which does not
/// wait for the person, so a window-event handler can raise one. With no parent window macOS draws it in its own
/// UserNotificationCenter process, so it shows while Settings is hidden.
#[cfg(target_os = "macos")]
fn show_late_result(app: &tauri::AppHandle, shown: sign_out::flow_window::Shown) {
    use sign_out::flow_window::Shown;
    let (title, message, kind) = match shown {
        Shown::Failure(message) => (
            "Sign-out paused",
            message,
            tauri_plugin_dialog::MessageDialogKind::Error,
        ),
        Shown::Warning(sentence) => ("Signed out", sentence, tauri_plugin_dialog::MessageDialogKind::Info),
    };
    app.dialog().message(message).title(title).kind(kind).show(|_| {});
}

/// What a close of `label` did (U34): the four steps of §5.1's hook, on the flow state alone. Read on macOS and in
/// tests only.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
struct FlowClosed {
    // The hook acts on `show` alone; these two say what the close ended, for the tests (U34).
    #[cfg_attr(not(test), allow(dead_code))]
    phase_closed: bool,
    #[cfg_attr(not(test), allow(dead_code))]
    snapshot_dropped: bool,
    show: Option<sign_out::flow_window::Shown>,
}

#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn flow_window_closed(sign_out: &sign_out::SignOutState, label: &str) -> FlowClosed {
    let phase_closed = match sign_out.phase.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
        Some(phase) if phase.owner == label => {
            phase.closed.store(true, Ordering::SeqCst);
            true
        }
        _ => false,
    };
    FlowClosed {
        phase_closed,
        snapshot_dropped: sign_out.snapshots.drop_owned_by(label),
        show: sign_out.late_results.window_closed(label),
    }
}

/// Spec 2026-10-10 §5.1 "A close ends the flow": every `CloseRequested` and `Destroyed`, and every page load `Started`,
/// of any window.
#[cfg(target_os = "macos")]
fn sign_out_flow_window_closed(app: &tauri::AppHandle, label: &str) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let closed = flow_window_closed(&state.sign_out, label);
    let _ = app.emit_to(label, "sign-out-flow-closed", ());
    if let Some(shown) = closed.show {
        show_late_result(app, shown);
    }
}

/// The webview displayed its `clear_session`'s outcome (spec 2026-10-10 F12): remove that window's pending result.
#[cfg(target_os = "macos")]
#[tauri::command]
fn sign_out_result_ack(window: tauri::WebviewWindow, state: State<'_, AppState>) -> Result<(), String> {
    state.sign_out.late_results.acknowledge(window.label());
    Ok(())
}

/// Windows and Linux keep today's sign-out (spec 2026-10-10 W5): nothing is ever pending there.
#[cfg(not(target_os = "macos"))]
#[tauri::command]
fn sign_out_result_ack(window: tauri::WebviewWindow, state: State<'_, AppState>) -> Result<(), String> {
    let _ = (window, state);
    Ok(())
}

/// §5.1 "A flow request survives a window that has not loaded yet": one pending request. Read on macOS and in tests
/// only.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn take_sign_out_flow_request(sign_out: &sign_out::SignOutState) -> bool {
    sign_out.flow_request.swap(false, Ordering::SeqCst)
}

/// Entry points 2 and 5: store the request, open the Beebeeb window on Settings › Account (task 1683 slice 1,
/// two-surface spec §8.1), and tell it (it also consumes on mount).
#[cfg(target_os = "macos")]
fn open_sign_out_flow(app: &tauri::AppHandle) {
    app.state::<AppState>()
        .sign_out
        .flow_request
        .store(true, Ordering::SeqCst);
    show_main_window(app, MainNav::settings(Some("account")));
    let _ = app.emit_to("main", "menu:sign-out-flow", ());
}

#[cfg(target_os = "macos")]
#[tauri::command]
fn request_sign_out_flow(app: tauri::AppHandle) -> Result<(), String> {
    open_sign_out_flow(&app);
    Ok(())
}

#[cfg(target_os = "macos")]
#[tauri::command]
fn consume_sign_out_flow_request(state: State<'_, AppState>) -> bool {
    take_sign_out_flow_request(&state.sign_out)
}

/// Windows and Linux keep today's sign-out (spec 2026-10-10 W5): there is no flow to open there.
#[cfg(not(target_os = "macos"))]
#[tauri::command]
fn request_sign_out_flow(app: tauri::AppHandle) -> Result<(), String> {
    let _ = app;
    Err("This works on a Mac only.".into())
}

/// Windows and Linux: no request is ever pending.
#[cfg(not(target_os = "macos"))]
#[tauri::command]
fn consume_sign_out_flow_request(state: State<'_, AppState>) -> bool {
    let _ = state;
    false
}

/// Task 1882 (spec `docs/specs/2026-10-09-macos-removal-keeps-unsynced-files.md` §5): the app's
/// own alert after a removal that kept files — the sentence, a blank line, then the folder. The app-start sweep and a
/// reconciler removal nobody waits for (an owed removal, the account binding's Repair, or one that finished after the
/// reconciler's time limit) raise it through `surface_kept_folder`, which first saves the folder for the Settings ›
/// Sync row. A sign-out puts the same body in its own alert, after its kept folder's (`surface_sign_out_folders`, task
/// 1887). `install_finder_location`'s rollback calls it too, but only Windows and Linux have that path, and nothing is
/// kept there. Logs never carry the folder. Nothing kept → no alert.
///
/// Device K-F1 (round 2): with no parent window, `tauri-plugin-dialog` 2.7 hands this to `rfd`
/// 0.16, which calls `CFUserNotificationDisplayAlert` on a background thread. macOS draws that
/// alert in its own UserNotificationCenter process, so it is never a Beebeeb window. The two
/// debug lines let a device check prove it was raised and closed, without the path.
fn show_preserved_files_alert(app: &tauri::AppHandle, preserved_location: Option<&str>) {
    let Some((title, message)) = finder_removal::kept_folder_alert(preserved_location) else {
        return;
    };
    app.dialog()
        .message(message)
        .title(title)
        .kind(tauri_plugin_dialog::MessageDialogKind::Info)
        .show(|_| {
            tracing::debug!("kept-folder alert closed");
        });
    tracing::debug!("kept-folder alert shown");
}

/// Review I2 (lead ruling, round 2): hands a kept folder to the person on the paths that have no
/// config of their own (the app-start sweep, a reconciler removal): saved for the Settings › Sync row
/// first, with the windows told (rebase re-review I2: [`save_kept_folder_for_row`]), then the alert. Nothing kept →
/// nothing. A sign-out surfaces its folders through [`surface_sign_out_folders`].
fn surface_kept_folder(app: &tauri::AppHandle, preserved_location: Option<&str>) {
    surface_kept_folder_with(
        preserved_location,
        |location| save_kept_folder_for_row(app, location),
        |location| show_preserved_files_alert(app, Some(location)),
    );
}

/// [`surface_kept_folder`] with its steps passed in, so a test can run the real order on its own config file and
/// record the event and the alert: `save_for_row` (the save and the telling) first, then the alert. Nothing kept →
/// none runs.
fn surface_kept_folder_with(
    preserved_location: Option<&str>,
    save_for_row: impl FnOnce(&str),
    alert: impl FnOnce(&str),
) {
    let Some(location) = preserved_location else {
        return;
    };
    save_for_row(location);
    alert(location);
}

/// Spec 2026-10-10 W9 and §9: after a sign-out, ONE alert names our folder, then macOS's. Our folder was recorded by
/// the keep step already; macOS's is saved for its row here, as before (the one route that tells the windows). Neither
/// path is logged.
fn surface_sign_out_folders(app: &tauri::AppHandle, result: &Result<SignedOut, SignOutFailure>) {
    let (macos, ours) = sign_out_folders(result);
    match macos {
        Some(location) => save_kept_folder_for_row(app, location),
        None if ours.is_some() => emit_kept_folder_changed(app),
        None => {}
    }
    // Task 2191 (D11): on Windows "ours" is the folder files of unknown content were kept in.
    #[cfg(target_os = "windows")]
    let alert = match ours {
        Some(kept) => sign_out::copy::windows_kept_files_alert(&kept.folder, kept.files),
        None => sign_out::copy::sign_out_alert(None, macos),
    };
    #[cfg(not(target_os = "windows"))]
    let alert = sign_out::copy::sign_out_alert(ours.map(|kept| (kept.folder.as_str(), kept.files)), macos);
    let Some((title, message)) = alert else {
        return;
    };
    app.dialog()
        .message(message)
        .title(title)
        .kind(tauri_plugin_dialog::MessageDialogKind::Info)
        .show(|_| {
            tracing::debug!("kept-folder alert closed");
        });
    tracing::debug!("kept-folder alert shown");
}

/// The two folders a sign-out's result carries, whether it succeeded or failed after the removal (task 1882):
/// macOS's, then ours. It replaces the helper that named macOS's folder only.
fn sign_out_folders(result: &Result<SignedOut, SignOutFailure>) -> (Option<&str>, Option<&KeptUnsent>) {
    match result {
        Ok(done) => (done.preserved_location.as_deref(), done.kept_unsent.as_ref()),
        Err(failure) => (failure.preserved_location.as_deref(), failure.kept_unsent.as_ref()),
    }
}

/// U26: "Show in Finder" reveals only the folder saved for the row.
fn reveal_target<'a>(saved: Option<&str>, path: &'a str) -> Result<&'a str, String> {
    if saved == Some(path) {
        Ok(path)
    } else {
        Err(sign_out::copy::K3_TOAST.to_string())
    }
}

/// The row's folder (spec 2026-10-10 §9), or none.
#[tauri::command]
fn kept_unsent_folder() -> Result<Option<String>, String> {
    sign_out::kept_record::saved_kept_unsent_at(&DesktopConfig::path()?)
}

/// The row's "Dismiss": forgets the folder it showed, never deletes it (§7.3).
#[tauri::command]
fn dismiss_kept_unsent_folder(app: tauri::AppHandle, path: String) -> Result<finder_removal::DismissOutcome, String> {
    let outcome = sign_out::kept_record::dismiss_saved_kept_unsent_at(&DesktopConfig::path()?, &path)?;
    emit_kept_folder_changed(&app);
    Ok(outcome)
}

/// "Show in Finder" (spec 2026-10-10 §7.1): the opener's reveal, never `/usr/bin/open`. On macOS it fails only when
/// the folder is gone, which is what K3 says.
#[tauri::command]
fn show_kept_unsent_folder(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let saved = sign_out::kept_record::saved_kept_unsent_at(&DesktopConfig::path()?)?;
    let target = reveal_target(saved.as_deref(), &path)?;
    tracing::info!("kept folder reveal requested");
    app.opener()
        .reveal_item_in_dir(target)
        .map_err(|_| sign_out::copy::K3_TOAST.to_string())
}

/// Rebase re-review I2: told to every window once a kept folder was saved for the Settings › Sync row, so a Sync tab
/// that is open reads the folder again. A removal the reconciler runs in the background (after "Try again", or one
/// that finished after its time limit) saves the folder long after the tab's own read, and so does a Repair that the
/// compact window's Finder page started (device RB3-F1). No payload: the tab reads the folder through
/// `kept_unsynced_folder`, so the path is never sent to a window that did not ask for it. The frontend listens for
/// this name as `KEPT_FOLDER_CHANGED_EVENT`.
const KEPT_FOLDER_CHANGED_EVENT: &str = "kept-folder-changed";

fn emit_kept_folder_changed<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if let Err(error) = app.emit(KEPT_FOLDER_CHANGED_EVENT, ()) {
        tracing::warn!(%error, "could not tell the windows that a kept folder was saved");
    }
}

/// Device RB3-F1 (lead ruling, 2026-10-10): the ONE place a kept folder is saved for the Settings › Sync row, and
/// every save of it tells the windows once it is saved, so a Sync tab that is open follows a Repair that another
/// window started (the compact window's Finder page), as it follows a sign-out or a late reconciler removal. The
/// callers are `surface_kept_folder` (the app-start sweep, the reconciler), `surface_sign_out_folders` (sign-out) and
/// macOS Repair, which has no alert of its own (1882 spec §5: the page's result line carries the folder). A save that
/// failed is logged without the path and the windows are told anyway; the tab reads the row again and finds what is
/// there. A source pin (`rb3_f1_every_save_of_the_rows_folder_goes_through_the_one_function_that_tells_the_windows`)
/// fails on any other route to `remember_kept_folder`.
fn save_kept_folder_for_row<R: tauri::Runtime>(app: &tauri::AppHandle<R>, location: &str) {
    save_kept_folder_for_row_with(location, remember_kept_folder, || emit_kept_folder_changed(app));
}

/// [`save_kept_folder_for_row`] with its two steps passed in: `save` first, then `changed`, once each.
fn save_kept_folder_for_row_with(location: &str, save: impl FnOnce(&str), changed: impl FnOnce()) {
    save(location);
    changed();
}

/// Saves the latest kept folder in `desktop.toml` (spec §5, "The kept-folder row"). Best-effort:
/// a failure is logged without the path, and the alert still names the folder.
///
/// Round 5: one load-change-save under the config-write lock (`DesktopConfig::update_at`), because
/// the app-start sweep runs this while the window may be saving a setting; two unserialized
/// saves share one `desktop.toml.tmp` and the later could drop the earlier one's change.
fn remember_kept_folder(location: &str) {
    if let Err(error) = finder_removal::remember_kept_folder(location) {
        tracing::warn!(error = %error, "could not save the kept folder for Settings › Sync");
    }
}

/// Review I2: the folder Settings › Sync shows until the person dismisses it. `None` = no row.
#[tauri::command]
fn kept_unsynced_folder() -> Result<Option<String>, String> {
    Ok(DesktopConfig::load()?.kept_unsynced_folder)
}

/// Review I2: the row's "Dismiss". Clears the saved folder only if it is still `path`, the one
/// the row showed. Round 5: it says whether it cleared it and which folder is saved now, so a row
/// that showed an older folder shows the newer one instead of vanishing. Under the config-write
/// lock, like the save above.
#[tauri::command]
fn dismiss_kept_unsynced_folder(path: String) -> Result<finder_removal::DismissOutcome, String> {
    finder_removal::dismiss_saved_kept_folder(&path)
}

/// Put a session restored from the Keychain into memory. Deliberately no `bump_vault_epoch()`
/// (see its doc): the lock that preceded this already moved the epoch, and a cold start has no
/// cached storage summary to go stale. Split out of `unlock_vault` so a test can run the real
/// install after the real `lock_vault`; the Keychain read before it has no Linux backend.
fn install_unlocked_session(_write: &SessionWrite, acct: &AccountRuntime, session: Session) -> Result<(), String> {
    let mut guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
    *guard = Some(session);
    Ok(())
}

/// The Keychain unlock's install, in its turn (lead ruling 1, Task 12): `turn` was captured before the Keychain was
/// read, and the session and `auth_present` go into memory only while no other session transition happened since.
fn install_keychain_session(
    state: &AppState,
    acct: &AccountRuntime,
    turn: &mut account::SessionGeneration,
    session: Session,
) -> Result<(), String> {
    let write = claim_session_write(acct, turn).map_err(|changed| changed.keychain_unlock_sentence().to_string())?;
    install_unlocked_session(&write, acct, session)?;
    set_auth_present(state, true);
    Ok(())
}

/// What `unlock_vault` says on a Mac that holds a session token and no vault key (the first password sign-in that was never
/// finished with the recovery phrase): the approved Set up this Mac sentence, which the Beebeeb window recognises to offer the
/// phrase (`isNoVaultKey`, pinned equal in `tests/signInModel.test.ts`). Without it the person read the Keychain's own
/// words, "unlock vault key from Keychain: secret not found". Windows and Linux keep what they showed.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const NO_VAULT_KEY_UNLOCK_ERROR: &str = "This Mac does not have your encryption keys yet. Restore them to continue.";

/// The same test the startup restore applies to this failure (`error.contains("secret not found")`, in
/// `restore_session_inner`): the token is there and the key is not.
fn unlock_failure(error: String) -> String {
    #[cfg(target_os = "macos")]
    if error.contains("secret not found") {
        return NO_VAULT_KEY_UNLOCK_ERROR.to_string();
    }
    error
}

/// Restore a Keychain-backed session into memory and start the engine for the
/// configured sync root. Keychain may show the OS unlock prompt depending on
/// the user's security settings. Until the user calls this command, hydration
/// and upload commands stay unavailable because there is no in-memory key.
#[tauri::command]
async fn unlock_vault(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let attempt = AUTH_ATTEMPTS.begin()?;
    let work = async {
        #[cfg(target_os = "windows")]
        let _transition = SESSION_TRANSITION.lock().await;
        #[cfg(target_os = "windows")]
        AUTH_ATTEMPTS.validate(&attempt)?;
        #[cfg(target_os = "windows")]
        {
            SESSION_COMMANDS.ensure_reactivation_allowed()?;
            windows_cf::ensure_reactivation_allowed()?;
        }
        let acct = state.active_account()?;
        let session_installed = acct
            .session
            .lock()
            .map_err(|_| "session mutex poisoned".to_string())?
            .is_some();
        if session_installed {
            let started = start_engine_if_possible(
                app,
                &state,
                #[cfg(target_os = "windows")]
                &_transition,
            )
            .await;
            keys_arrived(&state, KeysFrom::Unlock);
            return started;
        }

        // The unlock is one session transition (lead ruling 1, Task 12): captured before the Keychain is read, and the
        // session goes into memory in its turn only.
        let mut turn = acct.session_generation();
        let email = acct.auth_email.lock().ok().and_then(|guard| guard.clone());
        let session = load_session_from_keychain(acct.id.as_str(), email)
            .map_err(unlock_failure)?
            .ok_or_else(|| "Sign in before unlocking the vault.".to_string())?;
        install_keychain_session(&state, &acct, &mut turn, session)?;
        tracing::info!("vault unlocked from Keychain");
        let started = start_engine_if_possible(
            app,
            &state,
            #[cfg(target_os = "windows")]
            &_transition,
        )
        .await;
        keys_arrived(&state, KeysFrom::Unlock);
        started
    };
    #[cfg(target_os = "windows")]
    {
        attempt.run(work).await
    }
    #[cfg(not(target_os = "windows"))]
    {
        work.await
    }
}

/// Lock's engine stop, then the upload-staging purge it allows. Returns whether an engine stop on this account is
/// unconfirmed after it: this one, or an earlier one that left the slot empty (lead ruling T8-lockflag). Windows
/// refuses the lock instead, so there it is always `false`.
///
/// Task 1538 Codex P1: lock, like sign-out, clears the in-memory
/// session/master key right after this — an unconfirmed stop means the old
/// engine could still be alive and using it. No general local-file purge is
/// gated on this (lock keeps the account's Keychain session AND the regular
/// local file cache, so re-unlocking stays fast — there's nothing
/// cross-account to protect here), but it's still worth a loud warning rather
/// than a silent "we waited 3s and moved on". Windows refuses the lock.
///
/// Upload staging is emptied only when no engine can still be running: none
/// was, or its stop is confirmed, and no earlier stop is unconfirmed. The IPC
/// listener runs inside the engine's task; one that might still be serving
/// could take a request whose copy the purge just removed and refuse it as
/// `missing`, a definitive failure for the user's file. On an unconfirmed stop
/// the lock still goes ahead and the copies are left for the age-bound purge.
/// The caller holds the engine slot, so no new engine can start in between.
async fn stop_engine_for_lock(
    acct: &AccountRuntime,
    engine_slot: &mut Option<runner::EngineRunner>,
    upload_staging: Option<&std::path::Path>,
) -> Result<bool, String> {
    // Lead ruling T8-lockflag: like sign-out, lock honours an earlier stop that was never confirmed.
    // It still clears the session, and it says to restart instead of reporting success.
    #[cfg(not(target_os = "windows"))]
    let mut engine_unconfirmed = acct.engine_stop_unconfirmed.load(Ordering::SeqCst);
    #[cfg(target_os = "windows")]
    let engine_unconfirmed = false;
    #[cfg(not(target_os = "windows"))]
    if engine_unconfirmed {
        tracing::error!("vault lock: an earlier sync engine stop was never confirmed; restart Beebeeb");
    }
    if let Some(prev) = engine_slot.take() {
        if !stop_engine_in_slot(acct, prev).await.is_stopped() {
            #[cfg(target_os = "windows")]
            return Err("Vault lock failed: sync is still stopping. The vault is not locked. Retry locking; if it persists, restart Beebeeb.".into());
            // Lead ruling T8-lockflag: the consumed handle leaves an empty slot, so the flag (set by
            // `stop_engine_in_slot`) is what keeps every engine start refused (and sign-out) until a restart.
            #[cfg(not(target_os = "windows"))]
            {
                engine_unconfirmed = true;
                tracing::error!(
                    "engine did not confirm termination before vault lock cleared the in-memory session; restart Beebeeb"
                );
            }
        } else {
            tracing::info!("engine aborted on vault lock");
        }
    }
    purge_upload_staging_after_engine_stop("lock", !engine_unconfirmed, upload_staging);
    Ok(engine_unconfirmed)
}

/// The App Group upload-staging directory that sign-out and Lock empty.
/// `None` off macOS. Under `cargo test` it is never the real App Group
/// container: `None`, unless the test runs inside
/// [`with_upload_staging_for_test`] (a folder the test created).
fn upload_staging_dir_for_session_purge() -> Option<std::path::PathBuf> {
    #[cfg(all(target_os = "macos", not(test)))]
    return Some(crate::ipc_socket::macos_upload_staging_dir());
    #[cfg(test)]
    return UPLOAD_STAGING_FOR_TEST.try_with(Clone::clone).ok();
    #[cfg(not(any(target_os = "macos", test)))]
    None
}

#[cfg(test)]
tokio::task_local! {
    /// Set by [`with_upload_staging_for_test`]: the upload-staging folder this task's sign-out and Lock empty.
    static UPLOAD_STAGING_FOR_TEST: std::path::PathBuf;
}

/// Test builds: run `body` with `staging` as the upload-staging folder the session purge empties, so a test drives
/// the real sign-out or Lock path against a folder it created. Only this task sees it.
#[cfg(all(test, unix))]
async fn with_upload_staging_for_test<T>(staging: std::path::PathBuf, body: impl std::future::Future<Output = T>) -> T {
    UPLOAD_STAGING_FOR_TEST.scope(staging, body).await
}

/// Empty upload staging at sign-out or Lock, but only after a confirmed
/// engine stop (`engine_stopped`); otherwise leave it, logged, for the
/// age-bound purge. Best-effort: a failure is logged, never surfaced.
#[cfg(all(unix, any(target_os = "macos", test)))]
fn purge_upload_staging_after_engine_stop(
    context: &'static str,
    engine_stopped: bool,
    upload_staging: Option<&std::path::Path>,
) {
    let Some(dir) = upload_staging else {
        return;
    };
    if !engine_stopped {
        tracing::warn!(
            context,
            "engine stop unconfirmed: upload-staging copies are left for the age-bound purge"
        );
        return;
    }
    crate::ipc_socket::purge_all_upload_staging_at(dir, context);
}

#[cfg(not(all(unix, any(target_os = "macos", test))))]
fn purge_upload_staging_after_engine_stop(
    _context: &'static str,
    _engine_stopped: bool,
    _upload_staging: Option<&std::path::Path>,
) {
}

/// Lock clears all runtime key material and stops the sync daemon, but keeps
/// the Keychain session so the user can unlock again without re-entering their
/// recovery phrase. `Ok` means the lock happened; its `warning` says what of it could not be confirmed (FB-24).
#[tauri::command]
async fn lock_vault(state: State<'_, AppState>) -> Result<SessionActionOutcome, String> {
    #[cfg(target_os = "windows")]
    let _transition = SESSION_TRANSITION.lock().await;
    let acct = state.active_account()?;
    // Task 12, lead ruling 1: no session transition that began before this lock writes anything from here on (the
    // session clear below ends any that begins while it runs).
    drop(end_sessions_in_flight(&acct, account::SessionTransition::Lock));
    #[cfg(target_os = "windows")]
    close_session_commands().await?;
    // Spec A (plan "Spec issues" 8): cancel any Finder check and hold BEFORE the engine stops, so the
    // reconciler cannot start an engine with keys this lock is about to clear. A reconciler that does
    // not acknowledge (it is stuck, or it has stopped) does not stop the lock: everything below still
    // runs, and the result says Finder could not be confirmed (lead rulings T8-⚠lock 1 and T7-⚠2 5).
    #[cfg(target_os = "macos")]
    let finder_lock = finder_lock_for(&state).await;
    #[cfg(target_os = "macos")]
    if let Err(error) = &finder_lock {
        tracing::warn!(%error, "the Finder reconciler did not acknowledge the lock; locking anyway");
    }
    let mut engine_slot = acct.engine.lock().await;
    #[cfg(all(test, target_os = "macos"))]
    signal_lock_holds_the_engine_slot();
    // The stop (and the upload-staging purge it allows) is `stop_engine_for_lock`'s; it says whether a stop on this
    // account is unconfirmed (lead ruling T8-lockflag), for the warning below.
    #[cfg_attr(target_os = "windows", allow(unused_variables))]
    let engine_unconfirmed = stop_engine_for_lock(
        &acct,
        &mut engine_slot,
        upload_staging_dir_for_session_purge().as_deref(),
    )
    .await?;
    // Task 1670 round 2: UNLIKE the general local-file cache above, the macOS
    // hydrate-cache holds nothing but ephemeral per-Finder-open staging
    // copies (never the user's regular offline files), so purging it on
    // every lock is always safe and always right — it does not touch
    // anything `unlock_vault` needs to stay fast.
    purge_macos_hydrate_cache("lock");

    #[cfg(target_os = "windows")]
    {
        windows_cf::revoke_callbacks()
            .await
            .map_err(|e| format!("Could not disconnect Cloud Files: {e}"))?;
        stop_own_data_proofs();
        windows_cf::wait_for_credential_release().await?;
    }

    // The session is cleared while this function still holds the engine slot (it cannot be written otherwise:
    // `clear_session_holding_slot` borrows the guard). Only after that is the slot released, and a start that
    // was waiting for it reads its keys under the slot and finds none.
    clear_session_holding_slot(&acct, &engine_slot, account::SessionTransition::Lock)?;
    #[cfg(not(target_os = "windows"))]
    drop(engine_slot);
    // Drop the cached account profile on lock too, so "locked == no profile"
    // stays honest; the next `unlock_vault` re-fetches.
    clear_cached_profile(&state);
    // Lock = clean slate: drop any in-flight 2FA challenge so an abandoned
    // partial token can't linger (and is zeroized) past a lock.
    if let Ok(mut guard) = state.pending_2fa.lock() {
        *guard = None;
    }
    if let Ok(mut guard) = acct.engine_state.lock() {
        *guard = "stopped".to_string();
    }
    #[cfg(target_os = "windows")]
    {
        SESSION_COMMANDS.finish_close();
        AUTH_ATTEMPTS.finish_close();
    }
    set_auth_present(&state, keychain_session_present(acct.id.as_str()));
    // Everything this lock can clear is cleared, so it happened; what it could not confirm is a warning on that
    // success (FB-24). An engine that may still hold the key comes first: its fix is the more urgent one.
    #[cfg(not(target_os = "windows"))]
    if engine_unconfirmed {
        return Ok(SessionActionOutcome {
            warning: Some(ActionWarning::EngineStopUnconfirmed),
        });
    }
    #[cfg(target_os = "macos")]
    if finder_lock.is_err() {
        return Ok(SessionActionOutcome {
            warning: Some(ActionWarning::FinderLockUnconfirmed),
        });
    }
    Ok(SessionActionOutcome { warning: None })
}

#[derive(Debug, Clone, serde::Serialize)]
struct FinderInstallState {
    installed: bool,
    path: Option<String>,
    status: String,
    last_error: Option<String>,
    last_attempt_at: Option<i64>,
    reason_category: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct MacosIntegrationResetResult {
    removed_file_provider_domain: bool,
    /// Task 1882: the folder where macOS kept un-synced Finder files when the
    /// repair removed the Finder location; `None` = nothing kept.
    preserved_location: Option<String>,
    disabled_autostart: bool,
    removed_socket: bool,
    removed_cache_files: usize,
    skipped_cache_files: usize,
    pending_operations_preserved: i64,
    sync_root_preserved: Option<String>,
    /// An engine stop on this account is unconfirmed after Repair's own stop (this one, or an earlier one): its task
    /// may still run with the keys, so sync starts again only after Beebeeb is quit and reopened (spec §5.6). On macOS
    /// the surfaces render one fixed sentence for it, and `warnings` does not also carry the engine-stop text. On
    /// Windows and Linux `warnings` carries that text, as their surfaces show it.
    engine_stop_unconfirmed: bool,
    warnings: Vec<String>,
}

fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

/// What `wait_for_file_provider_ipc_ready` fails with when the local sync socket does not come up in time. It is the
/// sentence a person reads, and it is also what a Linux or Windows install saves as `finder_install_last_error`, so the
/// classifier below recognises this exact text as a timeout: it holds neither "timed out" nor "timeout" (two-surface spec
/// §10.2: no "daemon", no "Timed out waiting for"), and a reworded sentence must not turn the saved category into "unknown"
/// (PR #140 review, P2-8).
const FINDER_IPC_NOT_READY: &str = "Beebeeb wasn’t ready in time to add its Finder location.";

/// Whether a Finder-install error is a wait that ran out: the typed sentence above, or an OS error that says so.
#[cfg(any(test, not(target_os = "macos")))]
fn finder_install_error_is_timeout(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("timed out") || lower.contains("timeout") || error.contains(FINDER_IPC_NOT_READY)
}

// The install-era Finder state (`finder_install_*` in desktop.toml and the helpers below) is for
// Windows and Linux only. On macOS the reconciler owns the truth (spec 2026-10-06 §6.1).
#[cfg(not(target_os = "macos"))]
fn classify_finder_install_error(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if lower.contains("turned off in system settings") {
        // Issue 4 (task 1524): a user-disabled File Provider domain, detected BEFORE
        // any wait/timeout is attempted -- distinct from the generic "disabled" NSError
        // case below, which comes from the OS itself mid-operation.
        "user_disabled".to_string()
    } else if lower.contains("provision") || lower.contains("entitlement") || lower.contains("app-group") {
        "provisioning".to_string()
    } else if lower.contains("disabled") || lower.contains("-2011") || lower.contains("sync is not enabled") {
        "disabled".to_string()
    } else if finder_install_error_is_timeout(error) {
        "timeout".to_string()
    } else if lower.contains("not available") || lower.contains("only available") || lower.contains("unsupported") {
        "unsupported".to_string()
    } else {
        "unknown".to_string()
    }
}

#[cfg(not(target_os = "macos"))]
fn finder_install_state_from_config(
    cfg: &DesktopConfig,
    installed: bool,
    runtime_error: Option<String>,
) -> FinderInstallState {
    let status = if installed {
        "installed".to_string()
    } else if runtime_error.is_some() || cfg.finder_install_status.as_deref() == Some("error") {
        "error".to_string()
    } else {
        cfg.finder_install_status
            .clone()
            .unwrap_or_else(|| "missing".to_string())
    };
    let last_error = if installed {
        None
    } else {
        runtime_error.or_else(|| cfg.finder_install_last_error.clone())
    };
    let reason_category = if installed {
        None
    } else if let Some(error) = &last_error {
        Some(classify_finder_install_error(error))
    } else {
        cfg.finder_install_reason_category.clone()
    };

    FinderInstallState {
        installed,
        path: finder_state_path(cfg, installed),
        status,
        last_error,
        last_attempt_at: cfg.finder_install_last_attempt_at,
        reason_category,
    }
}

#[cfg(not(target_os = "macos"))]
fn finder_state_path(cfg: &DesktopConfig, installed: bool) -> Option<String> {
    let _ = installed;
    cfg.sync_root.as_ref().map(|p| p.to_string_lossy().into_owned())
}

/// The end of Repair, once the Finder location is gone (re-review P1, as review I1 is for
/// sign-out): saves the config, and if that save fails, shows the folder macOS kept BEFORE the
/// error goes back. The removal cannot be undone and a retry finds no domain, so without this the
/// folder would never be named: the error only says Repair failed, and Repair raises no alert of
/// its own. A save that works shows nothing here: the result carries the folder to the Sync tab.
/// `surface` is `surface_kept_folder` in the app; the tests pass a recorder.
///
/// Windows and Linux only in the app: on macOS Repair saves no config after the removal (spec 2026-10-06 §6.1),
/// so there is no save that can fail after it.
///
/// Rebase re-review Minor 4 (lead ruling, 2026-10-10): the error is the save's own. 1882 r4 marked it with a
/// `repair_failed_after_removal` code when the removal had worked, but that can happen on no platform now: a Mac
/// saves nothing after its removal, and Windows and Linux have no File Provider, so their removal never works. The
/// code and both surfaces' notes for it are gone.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn finish_repair_after_removal(
    cfg: &mut HeldConfig,
    preserved_location: Option<&str>,
    save: impl FnOnce(&mut HeldConfig) -> Result<(), String>,
    surface: impl FnOnce(Option<&str>),
) -> Result<(), String> {
    match save(cfg) {
        Ok(()) => Ok(()),
        Err(error) => {
            surface(preserved_location);
            Err(error)
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn persist_finder_install_result(cfg: &mut HeldConfig, installed: bool, error: Option<String>) -> Result<(), String> {
    // Task 1889: the result is written onto the file as it is now (`update_held`), not by saving the copy this
    // flow loaded before its slow work, which would put back what another writer saved since.
    cfg.update_held(|held| {
        record_finder_install_result(held, installed, error);
        (true, ())
    })
}

/// The in-memory half of [`persist_finder_install_result`]: stamp the attempt time and the
/// outcome fields on `cfg` without touching disk, so the state a failed install returns can be
/// built (and unit-tested) from exactly what gets saved.
#[cfg(not(target_os = "macos"))]
fn record_finder_install_result(cfg: &mut DesktopConfig, installed: bool, error: Option<String>) {
    cfg.finder_install_last_attempt_at = Some(now_unix_seconds());
    if installed {
        cfg.finder_install_status = Some("installed".to_string());
        cfg.finder_install_last_error = None;
        cfg.finder_install_reason_category = None;
    } else if let Some(error) = error {
        cfg.finder_install_status = Some("error".to_string());
        cfg.finder_install_reason_category = Some(classify_finder_install_error(&error));
        cfg.finder_install_last_error = Some(error);
    } else {
        cfg.finder_install_status = Some("missing".to_string());
        cfg.finder_install_last_error = None;
        cfg.finder_install_reason_category = None;
    }
}

/// Task 1683 slice 5 / decision D1: a failed Finder install is SAVED or RETURNED AS AN ERROR,
/// not both. Before this, `install_finder_location` persisted the failure (read back by
/// `finder_location_state` and painted as a banner) and ALSO returned `Err`, which the pane
/// turned into a toast: the same string twice (screenshot 1). A failed install gates "Open in
/// Finder", so it is saved and returned as a state (`installed: false`, `status: "error"`,
/// `last_error` set); the UI renders that one inline and never toasts it. This builds that state
/// from the freshly recorded result, so what is returned is what is saved.
#[cfg(not(target_os = "macos"))]
fn finder_install_failure_state(cfg: &mut DesktopConfig, error: String) -> FinderInstallState {
    record_finder_install_result(cfg, false, Some(error));
    finder_install_state_from_config(cfg, false, None)
}

/// Spec section 7: the saved install error "is cleared when a new attempt starts". An attempt can
/// wait many seconds on the File Provider domain; until it ends, the previous attempt's error is
/// no longer true of anything, yet the status page and the popover read it from this config.
/// Clears the failure fields (an `error` status becomes `missing`, like the frontend's
/// `finderInstallStateWhileAttempting`); an `installed` status and the last attempt time are
/// left alone. Returns whether anything changed, so an attempt with nothing to clear writes nothing.
#[cfg(not(target_os = "macos"))]
fn clear_finder_install_failure(cfg: &mut DesktopConfig) -> bool {
    let had_failure = cfg.finder_install_status.as_deref() == Some("error")
        || cfg.finder_install_last_error.is_some()
        || cfg.finder_install_reason_category.is_some();
    if cfg.finder_install_status.as_deref() == Some("error") {
        cfg.finder_install_status = Some("missing".to_string());
    }
    cfg.finder_install_last_error = None;
    cfg.finder_install_reason_category = None;
    had_failure
}

/// Start of an install attempt: drop the previous attempt's failure from the saved state.
#[cfg(not(target_os = "macos"))]
fn begin_finder_install_attempt(cfg: &mut HeldConfig) -> Result<(), String> {
    cfg.update_held(|held| (clear_finder_install_failure(held), ()))
}

/// Save a failed Finder install and return it as an `Ok` state (see
/// [`finder_install_failure_state`]). Only a failure to save the config itself is an `Err`.
#[cfg(not(target_os = "macos"))]
fn finder_install_failed(cfg: &mut HeldConfig, error: String) -> Result<FinderInstallState, String> {
    // Task 1889: recorded and saved onto the file as it is now; the state is built from what was saved.
    cfg.update_held(|held| (true, finder_install_failure_state(held, error)))
}

fn state_db_for_config(cfg: &DesktopConfig) -> Result<Option<state_db::StateDb>, String> {
    let Some(_) = &cfg.sync_root else {
        return Ok(None);
    };
    let db_path = state_paths::beebeeb_state_dir()?.join(state_paths::STATE_DB_FILENAME);
    if !db_path.exists() {
        return Ok(None);
    }
    state_db::StateDb::open(&db_path)
        .map(Some)
        .map_err(|e| format!("open state.db: {e}"))
}

/// Resolve the local `state.db` from the app-local data dir alone — task
/// 1538 Codex P1 (PR #49, lib.rs:1085 thread).
///
/// `state.db` lives in `state_paths::beebeeb_state_dir()`, which resolves
/// purely from Tauri's `app_local_data_dir()`; it has NEVER depended on
/// `DesktopConfig`/`desktop.toml` (see that function's own doc comment).
/// `state_db_for_config` above nonetheless GATES on a successfully-loaded
/// `DesktopConfig` with a non-`None`, accepted `sync_root` — so a missing,
/// corrupt, unreadable `desktop.toml`, or one whose `sync_root` was rejected
/// on load, makes the caller's `.ok()` chain silently skip the ENTIRE
/// sign-out purge. That purge is a cross-account data-exfiltration control
/// (task 1538 findings 1+2); it must not be able to depend on config health
/// it needs no part of.
///
/// This resolves + opens `state.db` directly against the app-local state
/// dir, with no `DesktopConfig` involved at all. `Ok(None)` only when the
/// file genuinely doesn't exist yet (a fresh install that has never synced
/// anything — nothing to purge); any OTHER failure (state dir not
/// initialized, DB open/migration error) is returned as `Err` so the caller
/// logs it loudly instead of treating it the same as "nothing to do".
///
/// Thin wrapper around [`state_db_from_state_dir`] — the ONLY thing this
/// adds is resolving Tauri's global app-local-data path; that split exists
/// so the actual decision logic is unit-testable without a live Tauri
/// `AppHandle` having called `state_paths::init_from_app` first.
fn state_db_from_app_local_state_dir() -> Result<Option<state_db::StateDb>, String> {
    state_db_from_state_dir(&state_paths::beebeeb_state_dir()?)
}

/// Pure core of [`state_db_from_app_local_state_dir`]: open `state.db`
/// directly under `state_dir`, given explicitly — no Tauri global, no
/// `DesktopConfig`. Task 1538 Codex P1 (PR #49, lib.rs:1085 thread): this
/// signature is the proof the sign-out purge's DB resolution cannot depend
/// on `desktop.toml` health, because there is no `DesktopConfig` parameter
/// for it to depend on.
fn state_db_from_state_dir(state_dir: &std::path::Path) -> Result<Option<state_db::StateDb>, String> {
    let db_path = state_dir.join(state_paths::STATE_DB_FILENAME);
    // Only a database that is CERTAINLY not there is "none yet". A stat that fails for any other reason (a permission,
    // a path component that is not a directory) is unreadable, not absent (lead ruling 13, Task 12): every caller then
    // fails closed (a sign-in decides nothing, a sign-out stops, an engine start refuses).
    match db_path.try_exists() {
        Ok(true) => {}
        Ok(false) => return Ok(None),
        Err(error) => return Err(format!("read state.db: {error}")),
    }
    state_db::StateDb::open(&db_path)
        .map(Some)
        .map_err(|e| format!("open state.db: {e}"))
}

fn disposable_cache_roots() -> Vec<PathBuf> {
    disposable_cache_roots_in(
        std::env::temp_dir(),
        dirs::cache_dir(),
        macos_hydrate_cache_root(),
        &engine_bridge::FinderStagingBases::current(),
    )
}

/// The hydrate cache's root on a Mac (task 1670); none elsewhere.
fn macos_hydrate_cache_root() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        Some(crate::ipc_socket::macos_hydrate_cache_dir())
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// Spec 2026-10-10 §7.1 (U12): the roots `is_disposable_cache_path` accepts, from directories passed in, so a test can
/// check a release layout. The temp dir, the cache dir, the macOS hydrate cache, then every staging folder (spec
/// 2026-10-09 §8.4), never the whole data dir.
fn disposable_cache_roots_in(
    temp_dir: PathBuf,
    cache_dir: Option<PathBuf>,
    hydrate_cache_dir: Option<PathBuf>,
    staging: &engine_bridge::FinderStagingBases,
) -> Vec<PathBuf> {
    let mut roots = vec![temp_dir];
    roots.extend(cache_dir);
    // Task 1670 round 2, Codex P1 (PR #75): `hydrate_file` records a macOS
    // hydrate-cache destination as `cache_path` via `mark_cached`, same as
    // any other cache entry. Without this, `is_disposable_cache_path` (the
    // gate `purge_local_state_files` — sign-out's purge — checks every
    // candidate path against) would refuse to remove it, since neither
    // `std::env::temp_dir()` nor `dirs::cache_dir()` contains it — a
    // sign-out purge would silently skip a staged plaintext file. In
    // practice `fetchContents` (`FileProviderExtension.swift`) already
    // deletes its own staged file unconditionally right after every
    // `completionHandler` call, and the dedicated
    // `macos_purge_hydrate_cache_dir` sweep (`purge_macos_hydrate_cache`)
    // independently wipes this whole directory on sign-out/lock/startup —
    // so this is defense in depth for the general-purpose per-path cleanup
    // machinery, not the primary mechanism, but it closes the gap Codex
    // named precisely (this general machinery must never treat a path
    // holding decrypted plaintext as "not ours to remove").
    roots.extend(hydrate_cache_dir);
    // Every folder a Finder write's plaintext copy is staged in, or was staged in by an earlier build (spec
    // 2026-10-09 §8.4): on macOS the data dir's `beebeeb/finder-writes`, which is under neither root above, so
    // without it a sign-out would skip those copies and leave plaintext on disk. The folders themselves, never the
    // whole data dir.
    roots.extend(staging.candidates());
    roots
}

/// `…/Logs/Beebeeb` anywhere in `path`: the lifecycle log's directory, in the home or in the app's
/// container (spec 2026-10-06 §8). It survives sign-out, Repair and the account reset (lead ruling
/// T5-⚠c), so no purge may treat anything under it as disposable, whatever root it sits in.
fn is_lifecycle_log_path(path: &std::path::Path) -> bool {
    let names: Vec<&str> = path.components().filter_map(|part| part.as_os_str().to_str()).collect();
    names
        .windows(2)
        .any(|pair| pair[0].eq_ignore_ascii_case("Logs") && pair[1].eq_ignore_ascii_case("Beebeeb"))
}

fn is_disposable_cache_path(path: &std::path::Path) -> bool {
    if is_lifecycle_log_path(path) {
        return false;
    }
    let canonical_path = match path.canonicalize() {
        Ok(path) => path,
        Err(_) => {
            let Some(parent) = path.parent() else {
                return false;
            };
            let Ok(parent) = parent.canonicalize() else {
                return false;
            };
            match path.file_name() {
                Some(name) => parent.join(name),
                None => parent,
            }
        }
    };
    // A symlink into the log directory is the same directory.
    if is_lifecycle_log_path(&canonical_path) {
        return false;
    }
    disposable_cache_roots()
        .into_iter()
        .filter_map(|root| root.canonicalize().ok())
        .any(|root| canonical_path.starts_with(root))
}

/// Counts from [`purge_local_state_files`] (task 1538), logged by the
/// sign-out caller.
struct LocalStatePurgeSummary {
    queued_ops_purged: usize,
    files_removed: usize,
    files_skipped: usize,
    /// Windows Cloud Files placeholders dehydrated (or, failing that,
    /// removed outright) — task 1538 Codex P1, PR #49 state_db.rs:1785
    /// thread. Always `0` on macOS/Linux.
    windows_placeholders_dehydrated: usize,
}

/// Task 1538 findings 1+2: delete every queued operation's staged plaintext
/// payload and every cached plaintext file from disk, on top of
/// `state_db::StateDb::purge_all_local_state` clearing the DB rows that
/// pointed at them.
///
/// Pure w.r.t. Tauri/OS credential state (takes an already-opened `db`) so
/// it is directly unit-testable without a live `AppState`/Keychain — see
/// `clear_session_impl`, the only real caller, which resolves `db`
/// independently of `DesktopConfig` (task 1538 Codex P1) and never touches
/// this function's internals. `sync_root`, by contrast, is ONLY needed to
/// resolve Windows Cloud Files placeholder paths below — best-effort and
/// `None`-able; every other step here is sync-root-independent.
///
/// Each candidate path is gated by `is_disposable_cache_path` — the same
/// safety check `reset_macos_integration` already uses — before removal, so
/// a bug that fed this function an unexpected path can never turn a sign-out
/// into an arbitrary-file-delete. A gated-out or already-missing path is
/// logged, not fatal, per file. Since R10 (task 1834) a failure of the purge
/// ITSELF is fatal to the sign-out (macOS and Linux) and to an engine start
/// (the account reset), which both call this one function.
fn purge_local_state_files(
    db: &state_db::StateDb,
    sync_root: Option<&std::path::Path>,
    kept: Option<&sign_out::kept_set::KeptSet>,
) -> Result<LocalStatePurgeSummary, String> {
    purge_local_state_files_in(db, sync_root, &production_staging_dirs(), kept)
}

/// The directories whose plaintext copies the purge sweeps as a whole. A unit test never sweeps the shared
/// per-process sandbox that the engine-bridge tests stage into (a sweep from one test would delete a copy another
/// is about to read); the sweep itself is tested with directories the test creates.
fn production_staging_dirs() -> Vec<PathBuf> {
    #[cfg(test)]
    {
        STAGING_DIRS_OVERRIDE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .unwrap_or_default()
    }
    #[cfg(not(test))]
    {
        release_staging_dirs()
    }
}

/// What a release build sweeps: every place the engine stages a plaintext copy. A function of its own, compiled in
/// every build, so a test can look at the list a release build would use (`production_staging_dirs` is the test
/// override in a test build).
fn release_staging_dirs() -> Vec<PathBuf> {
    engine_bridge::finder_staging_candidates()
}

/// Unit tests only: empty (nothing is swept) unless a test sets it, so the production call sites can be driven with
/// directories the test created (`account_binding_tests::StagingOverride`).
#[cfg(test)]
static STAGING_DIRS_OVERRIDE: std::sync::Mutex<Option<Vec<PathBuf>>> = std::sync::Mutex::new(None);

/// Every regular file under `dir`, not following symlinks (a link is never a plaintext copy of ours, and
/// following one could leave the staging directory). A directory that does not exist has none.
fn files_under(dir: &std::path::Path, depth: u8, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_file() {
            out.push(entry.path());
        } else if kind.is_dir() && depth > 0 {
            files_under(&entry.path(), depth - 1, out);
        }
    }
}

/// The current user's id, where there is one to compare (Unix). A Windows temp dir is already per-user.
fn current_uid() -> Option<u32> {
    #[cfg(unix)]
    {
        // SAFETY: `geteuid` has no preconditions and cannot fail.
        Some(unsafe { libc::geteuid() })
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// A staging directory the sweep may enter (R5, fix round 2): a real directory, not a link, that `uid` owns, under
/// a parent (`.../beebeeb`) that is a real directory `uid` owns too. On a shared `/tmp` another local user's
/// `beebeeb/finder-writes` is neither ours to read nor ours to delete from, and a link could lead out of the
/// staging area. Without a user id to compare (Windows) a real directory is enough.
fn is_owned_staging_dir(dir: &std::path::Path, uid: Option<u32>) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let ours = |path: &std::path::Path| {
            std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir() && uid.is_none_or(|uid| meta.uid() == uid))
        };
        ours(dir) && dir.parent().is_none_or(ours)
    }
    #[cfg(not(unix))]
    {
        let _ = uid;
        std::fs::symlink_metadata(dir).is_ok_and(|meta| meta.is_dir())
    }
}

/// Every orphan regular file in the staging directories this user owns; any other directory is left alone.
fn orphan_staged_files(staging_dirs: &[PathBuf], uid: Option<u32>) -> Vec<PathBuf> {
    let mut orphans = Vec::new();
    for dir in staging_dirs {
        if is_owned_staging_dir(dir, uid) {
            files_under(dir, 8, &mut orphans);
        } else if dir.exists() {
            tracing::warn!(dir = %dir.display(), "purge did not sweep a staging directory that is not this user's");
        }
    }
    orphans
}

/// [`purge_local_state_files`] with the staging directories to sweep. In this order: delete the files the rows
/// point at, and every orphan in the staging directories this user owns, each behind `is_disposable_cache_path`;
/// if ANY removal fails (a missing file is fine) return an error and clear no row, so a retry finds everything and
/// nothing is forgotten while a plaintext copy is still on disk; only then clear the rows. Given a `KeptSet` (a
/// macOS sign-out), the guard of [`purge_local_state_files_with`] runs before all of it, and the waiting writes'
/// staged copies are deleted last.
fn purge_local_state_files_in(
    db: &state_db::StateDb,
    sync_root: Option<&std::path::Path>,
    staging_dirs: &[PathBuf],
    kept: Option<&sign_out::kept_set::KeptSet>,
) -> Result<LocalStatePurgeSummary, String> {
    purge_local_state_files_as(db, sync_root, staging_dirs, current_uid(), kept)
}

/// [`purge_local_state_files_in`] as the user `uid` (a test pretends to be another user).
fn purge_local_state_files_as(
    db: &state_db::StateDb,
    sync_root: Option<&std::path::Path>,
    staging_dirs: &[PathBuf],
    uid: Option<u32>,
    kept: Option<&sign_out::kept_set::KeptSet>,
) -> Result<LocalStatePurgeSummary, String> {
    purge_local_state_files_with(db, sync_root, staging_dirs, uid, kept, &|path| {
        std::fs::remove_file(path)
    })
}

/// [`purge_local_state_files_as`] with the removal passed in (a test makes one removal fail). Spec 2026-10-10 §5.2
/// step 8 and W13: given a `KeptSet`, the guard comes first and refuses with `unsent_copy_failed: …` before any path
/// is collected or deleted, unless every waiting change is kept or acknowledged. Then every other file is removed, and
/// only after all of them the waiting writes' staged copies.
fn purge_local_state_files_with(
    db: &state_db::StateDb,
    sync_root: Option<&std::path::Path>,
    staging_dirs: &[PathBuf],
    uid: Option<u32>,
    kept: Option<&sign_out::kept_set::KeptSet>,
    remove: &dyn Fn(&std::path::Path) -> std::io::Result<()>,
) -> Result<LocalStatePurgeSummary, String> {
    // Spec 2026-10-10 §5.2 step 8: given a `KeptSet` (macOS), the guard is the FIRST statement: one read, and a
    // refusal before any path is collected or deleted. Without one there is no guard, and no copy is held back.
    let waiting_copies: std::collections::HashSet<PathBuf> = match kept {
        Some(kept) => {
            let guard = db
                .unkept_waiting_changes(kept)
                .map_err(|e| format!("check what the sign-out kept: {e}"))?;
            if !guard.unkept.is_empty() {
                tracing::error!(
                    unkept = guard.unkept.len(),
                    "sign-out stopped: a waiting change was not kept; nothing was deleted"
                );
                return Err(sign_out::copy::coded(
                    sign_out::copy::UNSENT_COPY_FAILED_CODE,
                    sign_out::copy::E2_UNSENT_COPY_FAILED,
                ));
            }
            guard.waiting_payloads.into_iter().map(PathBuf::from).collect()
        }
        None => std::collections::HashSet::new(),
    };
    let mut paths: Vec<PathBuf> = db
        .local_state_file_paths()
        .map_err(|e| format!("list the local files: {e}"))?
        .into_iter()
        .map(PathBuf::from)
        .collect();
    let owned: std::collections::HashSet<PathBuf> = paths.iter().cloned().collect();
    paths.extend(
        orphan_staged_files(staging_dirs, uid)
            .into_iter()
            .filter(|path| !owned.contains(path)),
    );
    // W13: every other file first, the waiting writes' staged copies last, so a failure on any other file leaves every
    // waiting write as it was.
    let (waiting_last, others_first): (Vec<PathBuf>, Vec<PathBuf>) =
        paths.drain(..).partition(|path| waiting_copies.contains(path));
    let mut files_removed = 0usize;
    let mut files_skipped = 0usize;
    let (removed, skipped) = remove_purge_paths(&others_first, remove)?;
    files_removed += removed;
    files_skipped += skipped;
    #[cfg(test)]
    sign_out::crash_point("AfterFirstGroup");
    // The waiting writes' staged copies one at a time: a removal that fails stops the purge before the next copy, and a
    // test (spec 2026-10-10 U42) can stop the process between two of them.
    for (index, path) in waiting_last.iter().enumerate() {
        if index == 1 {
            #[cfg(test)]
            sign_out::crash_point("BetweenWaitingCopies");
        }
        let (removed, skipped) = remove_purge_paths(std::slice::from_ref(path), remove)?;
        files_removed += removed;
        files_skipped += skipped;
    }
    #[cfg(test)]
    sign_out::crash_point("BeforeRowClear");

    let purge = db
        .purge_all_local_state()
        .map_err(|e| format!("purge operation queue and cache metadata: {e}"))?;

    // Task 1538 Codex P1 (state_db.rs:1785 thread): a Windows Cloud Files
    // placeholder's plaintext lives in the sync root, not at `cache_path` —
    // the loop above can never find it. `local_placeholder_paths` is the
    // cross-platform candidate list `purge_all_local_state` captured before
    // flipping those rows' status. `resolve_purge_placeholder_paths` (the
    // "which files, which safe paths" decision) runs on every platform so
    // it stays exercised and unit-testable without a Windows host; only the
    // actual `CfDehydratePlaceholder` FFI call is Windows-only. macOS/Linux
    // both store hydrated bytes as a separate cache copy already covered by
    // `cache_paths` above, and macOS's File Provider domain removal
    // (`clear_session_impl`, right after this function returns) handles the
    // analogous cleanup there — so this list is a deliberate no-op on those
    // platforms, not an oversight.
    let windows_placeholders_dehydrated: usize = if purge.local_placeholder_paths.is_empty() {
        0
    } else {
        match sync_root {
            Some(root) => {
                let resolved = resolve_purge_placeholder_paths(&purge.local_placeholder_paths, root);
                #[cfg(target_os = "windows")]
                {
                    // Windows production cleanup runs before metadata removal.
                    let _ = resolved;
                    0
                }
                #[cfg(not(target_os = "windows"))]
                {
                    let _ = resolved;
                    0
                }
            }
            None => {
                #[cfg(target_os = "windows")]
                tracing::warn!(
                    candidate_count = purge.local_placeholder_paths.len(),
                    "sign-out purge: no sync root known; Windows placeholders could not be dehydrated/removed"
                );
                0
            }
        }
    };

    Ok(LocalStatePurgeSummary {
        queued_ops_purged: purge.queued_ops_purged,
        files_removed,
        files_skipped,
        windows_placeholders_dehydrated,
    })
}

/// The purge's removal loop over one group: each path behind `is_disposable_cache_path`; a missing file is fine; any
/// other failure returns an error with no path in it. `(removed, skipped)`.
fn remove_purge_paths(
    paths: &[PathBuf],
    remove: &dyn Fn(&std::path::Path) -> std::io::Result<()>,
) -> Result<(usize, usize), String> {
    let (mut removed, mut skipped, mut failed) = (0usize, 0usize, 0usize);
    let mut first_failure: Option<std::io::ErrorKind> = None;
    for path in paths {
        if !is_disposable_cache_path(path) {
            skipped += 1;
            tracing::warn!(path = %path.display(), "purge skipped a path outside the known cache/staging roots");
            continue;
        }
        match remove(path) {
            Ok(()) => removed += 1,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                failed += 1;
                first_failure.get_or_insert(error.kind());
                tracing::warn!(path = %path.display(), error = %error, "failed to remove a local file in the purge");
            }
        }
    }
    if failed > 0 {
        // No path in the message: it reaches the sign-out error and the engine-start error.
        return Err(format!(
            "{failed} local file(s) could not be removed ({})",
            first_failure.map_or_else(|| "unknown".to_string(), |kind| kind.to_string())
        ));
    }
    Ok((removed, skipped))
}

/// Resolve every `(file_id, server_relative_path)` candidate from
/// [`state_db::LocalStatePurge::local_placeholder_paths`] to an absolute,
/// safety-checked on-disk path under `sync_root` — the same canonicalize +
/// allowed-roots invariant [`free_up_space_windows`] already enforces for
/// the analogous eviction sweep, reused here (task 1538 Codex P1, PR #49
/// state_db.rs:1785 thread).
///
/// Pure and platform-independent on purpose: the actual
/// `CfDehydratePlaceholder` FFI call that ACTS on this list is Windows-only
/// (`windows_cf::signout::purge`); this is only the "which files,
/// which paths" decision, so it is unit-testable on any host. A candidate is
/// silently dropped (not an error — sign-out must still succeed) when it
/// doesn't resolve to a real file under an allowed root: an
/// already-cleaned-up or corrupt row has nothing for the caller to act on.
fn resolve_purge_placeholder_paths(
    candidates: &[(String, String)],
    sync_root: &std::path::Path,
) -> Vec<(String, PathBuf)> {
    let allowed_roots = free_up_space_allowed_roots(sync_root);
    candidates
        .iter()
        .filter_map(|(file_id, rel_path)| {
            let rel = rel_path.trim_matches('/');
            if rel.is_empty() {
                return None;
            }
            let local_path = sync_root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
            let canonical = local_path.canonicalize().ok()?;
            if !allowed_roots.iter().any(|root| canonical.starts_with(root)) {
                return None;
            }
            Some((file_id.clone(), canonical))
        })
        .collect()
}

#[cfg(unix)]
fn remove_stale_ipc_socket() -> Result<bool, String> {
    let path = ipc_socket::ipc_socket_path();
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("remove IPC socket {}: {error}", path.display())),
    }
}

// Windows has no Unix-socket daemon endpoint — the Cloud Files provider runs
// in-process — so there is never a stale socket to remove.
#[cfg(not(unix))]
fn remove_stale_ipc_socket() -> Result<bool, String> {
    Ok(false)
}

// Windows and Linux only: on macOS the reconciler's `ensure_sync_root_and_engine` replaces it.
#[cfg(not(target_os = "macos"))]
async fn persist_sync_root_and_start_engine(
    app: tauri::AppHandle,
    state: &State<'_, AppState>,
    cfg: &mut HeldConfig,
    root: PathBuf,
    #[cfg(target_os = "windows")] _transition: &tokio::sync::MutexGuard<'_, ()>,
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let generation = {
        windows_cf::ensure_reactivation_allowed()?;
        SESSION_COMMANDS.generation()?
    };
    // Task 1889: the sync root is written onto the file as it is now, not by saving the copy loaded earlier; `cfg`
    // afterwards holds what was saved, so the pause state below is the persisted one.
    cfg.update_held(|held| {
        held.sync_root = Some(root.clone());
        (true, ())
    })?;

    let acct = state.active_account()?;
    // Rehydrate the persisted pause state before spawning.
    rehydrate_sync_paused(&acct.sync_paused, cfg.pause_sync);
    {
        let mut engine_slot = acct.engine.lock().await;
        engine_start_refusal(&acct)?;
        if let Some(prev) = engine_slot.take() {
            // Task 1538 Codex P1: this is a re-login/sync-root-change
            // respawn, not sign-out — there's no purge to gate here, but an
            // unconfirmed stop is still worth knowing about (a not-really-
            // gone previous task could still be touching the same state.db
            // the freshly spawned runner is about to open).
            if !stop_engine_in_slot(&acct, prev).await.is_stopped() {
                #[cfg(target_os = "windows")]
                return Err("Could not stop the previous sync engine; retry locking before restarting sync.".into());
                tracing::warn!("previous engine did not confirm termination before respawning a new one");
            }
        }
        // Fix round 1: an unconfirmed stop of the old engine has set the flag; refuse the new one.
        engine_start_refusal(&acct)?;
        #[cfg(target_os = "windows")]
        {
            windows_cf::ensure_reactivation_allowed()?;
            SESSION_COMMANDS.validate_start(generation)?;
        }
        // No session means nothing to start (the root is saved either way); the keys are read under the slot.
        spawn_bound_engine(app, state, &acct, &mut engine_slot, root)?;
    }

    Ok(())
}

// Windows and Linux only: on macOS the reconciler's `start_check_engine` replaces it, with its limit
// on the engine-slot wait alone.
#[cfg(not(target_os = "macos"))]
async fn start_engine_for_pending_finder_install(
    app: tauri::AppHandle,
    state: &State<'_, AppState>,
    root: PathBuf,
    #[cfg(target_os = "windows")] _transition: &tokio::sync::MutexGuard<'_, ()>,
) -> Result<bool, String> {
    #[cfg(target_os = "windows")]
    let generation = {
        windows_cf::ensure_reactivation_allowed()?;
        SESSION_COMMANDS.generation()?
    };
    let acct = state.active_account()?;

    // Rehydrate the persisted pause state before spawning. Best-effort —
    // a missing/unreadable config defaults to not-paused.
    rehydrate_sync_paused(&acct.sync_paused, false);

    #[cfg_attr(not(unix), allow(unused_variables))]
    let (started, ipc_bind_error) = {
        let mut engine_slot = acct.engine.lock().await;
        engine_start_refusal(&acct)?;
        if let Some(existing) = engine_slot.as_ref() {
            // Already running (e.g. a retry after a transient failure) —
            // reuse its bind-status handle so we still see a REAL bind
            // error instead of just re-timing-out silently.
            (false, existing.ipc_bind_error_handle())
        } else {
            #[cfg(target_os = "windows")]
            {
                windows_cf::ensure_reactivation_allowed()?;
                SESSION_COMMANDS.validate_start(generation)?;
            }
            // The keys are read under the slot: a vault that is locked by now is the same refusal as before.
            if spawn_bound_engine(app, state, &acct, &mut engine_slot, root)? == EngineStart::NoSession {
                return Err("Unlock the vault before installing the Finder location.".to_string());
            }
            let bind_error = engine_slot
                .as_ref()
                .map(|runner| runner.ipc_bind_error_handle())
                .ok_or_else(|| "the sync engine is not in its slot after it started".to_string())?;
            (true, bind_error)
        }
    };

    // The macOS File Provider extension talks to the daemon over a Unix
    // socket, so we wait for that endpoint before claiming the install is
    // ready. Windows has no such socket (the Cloud Files provider is
    // in-process), so the readiness probe is unix-only.
    #[cfg(unix)]
    if let Err(error) = wait_for_file_provider_ipc_ready(ipc_bind_error).await {
        stop_pending_finder_install_engine(state, started).await;
        return Err(error);
    }

    Ok(started)
}

// Windows and Linux only: macOS has `stop_check_engine`.
#[cfg(not(target_os = "macos"))]
async fn stop_pending_finder_install_engine(state: &State<'_, AppState>, started: bool) {
    if !started {
        return;
    }
    let Ok(acct) = state.active_account() else { return };
    let mut engine_slot = acct.engine.lock().await;
    if let Some(prev) = engine_slot.take() {
        // Task 1538 Codex P1 — see `start_engine_if_possible`'s identical
        // respawn guard.
        if !stop_engine_in_slot(&acct, prev).await.is_stopped() {
            tracing::warn!("pending-finder-install engine did not confirm termination on stop");
        }
    }
}

/// Poll for the daemon's IPC socket to come up, OR return immediately with
/// the real bind error if the accept loop already failed to bind (task
/// 1524) — e.g. the macOS sandbox refusing a socket path outside the app's
/// container. Before this, a bind failure panicked the fire-and-forget IPC
/// task silently and this function always burned the full 3-second timeout
/// before returning a generic "Timed out…" message with no indication of
/// the real cause.
#[cfg(unix)]
async fn wait_for_file_provider_ipc_ready(
    ipc_bind_error: std::sync::Arc<std::sync::Mutex<Option<String>>>,
) -> Result<(), String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;
    use tokio::time::{Duration, Instant, sleep, timeout};

    let path = ipc_socket::ipc_socket_path();
    let deadline = Instant::now() + Duration::from_secs(3);
    // One `\n`-terminated frame (task 1670 issue 3; see `docs/IPC_PROTOCOL.md`).
    let mut request = serde_json::to_vec(&ipc_socket::IpcRequest::GetSyncSummary)
        .map_err(|e| format!("encode IPC readiness probe: {e}"))?;
    request.push(b'\n');

    loop {
        if let Ok(guard) = ipc_bind_error.lock()
            && let Some(bind_error) = guard.as_ref()
        {
            return Err(format!("Could not start the local Beebeeb sync socket: {bind_error}"));
        }

        if let Ok(Ok(mut stream)) = timeout(Duration::from_millis(300), UnixStream::connect(&path)).await {
            let mut response = vec![0u8; 4096];
            if stream.write_all(&request).await.is_ok()
                && let Ok(Ok(bytes_read)) = timeout(Duration::from_millis(300), stream.read(&mut response)).await
                && bytes_read > 0
                && serde_json::from_slice::<ipc_socket::IpcResponse>(&response[..bytes_read]).is_ok()
            {
                return Ok(());
            }
        }

        if Instant::now() >= deadline {
            return Err(FINDER_IPC_NOT_READY.to_string());
        }
        sleep(Duration::from_millis(100)).await;
    }
}

/// Non-macOS mirror of what `macos_file_provider` reported before the reconciler (spec
/// 2026-10-06 §6.1 retired both on macOS). `finder_location_state` matches on this directly.
/// Windows and Linux only ever get the error arm from `file_provider_installed`.
#[cfg(not(target_os = "macos"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileProviderStatusOutcome {
    Installed,
    NotInstalled,
    UserDisabled,
}

#[cfg(not(target_os = "macos"))]
fn file_provider_installed() -> Result<FileProviderStatusOutcome, String> {
    Err("File Provider is only available on macOS.".to_string())
}

/// Non-macOS mirror of what `macos_file_provider::install` reported (retired on macOS).
/// `install_finder_location` matches on this directly.
#[cfg(not(target_os = "macos"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileProviderInstallOutcome {
    Installed,
    UserDisabled,
}

/// Shown to the user (and persisted as `finder_install_last_error`) when Issue 4's
/// condition is detected: the Beebeeb File Provider domain exists but the user (or
/// macOS) has turned it off in System Settings, so waiting for it to come up would
/// never succeed. `classify_finder_install_error` recognizes this exact copy and
/// maps it to the `"user_disabled"` reason category the frontend switches on.
#[cfg(not(target_os = "macos"))]
const FINDER_USER_DISABLED_MESSAGE: &str = "Beebeeb is turned off in System Settings. Open Login \
    Items & Extensions, turn on Beebeeb under File Providers, then try again.";

#[cfg(not(target_os = "macos"))]
fn install_file_provider_domain() -> Result<FileProviderInstallOutcome, finder_removal::InstallFailure> {
    Err("File Provider is only available on macOS.".to_string().into())
}

// On macOS the reconciler removes the domain (`finder_remove_for`); nothing calls the bridge's
// `remove` directly, because every bridge call goes through the one gate (lead ruling T8-gate-all).
#[cfg(not(target_os = "macos"))]
fn remove_file_provider_domain() -> Result<finder_removal::DomainRemoval, finder_removal::RemovalFailure> {
    Err("File Provider is only available on macOS.".to_string().into())
}

/// Runs a blocking File Provider call on the blocking pool, so it never holds the main thread or
/// an async runtime worker (1882 round 4). The removal blocks on the system's completion handler,
/// and when a kept folder reads as missing it then sleeps about a second (`settle_kept_state`).
/// The failure is the pool's own (a panicked or cancelled job); it carries no path.
///
/// Windows and Linux only in the app: on macOS every File Provider call goes through the reconciler's bridge
/// gate, which runs it on the blocking pool itself (`finder_setup::macos_ports::BridgeGate`).
#[cfg_attr(target_os = "macos", allow(dead_code))]
async fn on_blocking_pool<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| format!("A File Provider call did not finish: {error}"))
}

/// The removal, off the runtime (round 4).
#[cfg(not(target_os = "macos"))]
async fn remove_file_provider_domain_blocking() -> Result<finder_removal::DomainRemoval, finder_removal::RemovalFailure>
{
    match on_blocking_pool(remove_file_provider_domain).await {
        Ok(result) => result,
        Err(message) => Err(message.into()),
    }
}

/// The install, off the runtime (round 4): its cleanup is a removal.
#[cfg(not(target_os = "macos"))]
async fn install_file_provider_domain_blocking() -> Result<FileProviderInstallOutcome, finder_removal::InstallFailure> {
    match on_blocking_pool(install_file_provider_domain).await {
        Ok(result) => result,
        Err(message) => Err(message.into()),
    }
}

/// Task 1670 round 2: wipe the macOS hydrate-cache staging directory at every
/// account-security boundary (sign-out, lock, daemon startup) so a decrypted
/// plaintext copy staged for a Finder open can never outlive the boundary
/// that's supposed to end its access — independent of, and in addition to,
/// the per-request cleanup `fetchContents` itself does. Distinct from the
/// general local-file/smart-cache purge `purge_local_state_files` runs on
/// sign-out: THAT purge intentionally does NOT run on lock (lock keeps the
/// regular local file cache so re-unlocking stays fast), but the hydrate
/// cache holds nothing but ephemeral per-open staging copies, so it is always
/// safe — and always right — to empty here too. Best-effort and non-fatal:
/// logout/lock must always appear to succeed, and daemon startup must never
/// block on this.
#[cfg(target_os = "macos")]
pub(crate) fn purge_macos_hydrate_cache(context: &str) {
    let dir = crate::ipc_socket::macos_hydrate_cache_dir();
    match crate::ipc_socket::macos_purge_hydrate_cache_dir(&dir) {
        Ok(removed) if removed > 0 => {
            tracing::info!(removed, context, "purged macOS hydrate-cache staged plaintext");
        }
        Ok(_) => {}
        Err(error) => {
            tracing::warn!(error = %error, context, "failed to purge macOS hydrate-cache dir (best-effort)");
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn purge_macos_hydrate_cache(_context: &str) {}

/// "Open in Finder" on a Mac (task 1885): the bridge opens the Beebeeb folder through `NSWorkspace` and answers
/// when LaunchServices has. `Ok` only if it opened; an `Err` is what the frontend turns into the toast.
#[cfg(target_os = "macos")]
fn file_provider_open_location() -> Result<(), String> {
    finder_open::finder_open_outcome(finder_setup::macos_ports::open_location(), finder_open::NOTHING_TO_OPEN)
}

/// "Show in Finder" for one item on a Mac (task 1885): the item's File Provider identifier is its file id.
#[cfg(target_os = "macos")]
fn file_provider_reveal_item(item_id: &str) -> Result<(), String> {
    finder_open::finder_open_outcome(
        finder_setup::macos_ports::reveal_item(item_id),
        finder_open::NOTHING_TO_SHOW,
    )
}

/// The one conversion from a bridge error to text that can leave the bridge's callers: it
/// reaches Repair's `warnings` and `finder_domain_user_enabled`'s returned error, both shown to a
/// person. Domain and code only, never the OS's message (lead ruling T1-4). `tracing` sites log
/// the `FpError` itself, which keeps the message for stdout. (`finder_open::finder_open_outcome`
/// is the same conversion for the open and the reveal.)
#[cfg(target_os = "macos")]
fn user_facing_fp<T>(result: Result<T, crate::finder_setup::error::FpError>) -> Result<T, String> {
    result.map_err(|e| e.redacted())
}

/// `Some(true)` = the Beebeeb domain is user-enabled, `Some(false)` = the user (or
/// macOS) has disabled it in System Settings, `None` = the domain isn't registered
/// yet. Backs the `finder_domain_user_enabled` poll the "turned off in System
/// Settings" onboarding card uses to notice when the user flips it back on.
#[cfg(target_os = "macos")]
fn file_provider_domain_user_enabled() -> Result<Option<bool>, String> {
    // Through `user_facing_fp`, never an `FpError`'s `Display`: the error text reaches the frontend,
    // and `Display` carries the OS's message (lead ruling T1-4).
    match user_facing_fp(finder_setup::macos_ports::domain_state())? {
        finder_setup::core::DomainState::Enabled => Ok(Some(true)),
        finder_setup::core::DomainState::Disabled => Ok(Some(false)),
        finder_setup::core::DomainState::NotRegistered => Ok(None),
    }
}

#[cfg(not(target_os = "macos"))]
fn file_provider_domain_user_enabled() -> Result<Option<bool>, String> {
    Err("File Provider is only available on macOS.".to_string())
}

/// Spec 2026-10-06 §5.3: tell the macOS Finder reconciler something happened. A no-op where no
/// reconciler runs (Windows, Linux, and tests that install none). A reconciler that has stopped
/// (it panicked; it starts again at the next launch) cannot be told, and that is logged, never
/// silent (lead ruling 3).
fn notify_finder(state: &AppState, trigger: finder_setup::core::Trigger) {
    if let Some(handle) = state.finder_setup.get()
        && let Err(error) = handle.trigger(trigger)
    {
        tracing::warn!(%error, trigger = trigger.as_str(), "finder setup: could not deliver a trigger");
    }
}

/// Where keys arrived from, for the lifecycle log (§8): a sign-in that completed is logged, an unlock after a Lock is
/// not (a Lock is not logged either).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeysFrom {
    /// A sign-in, a re-sign-in or the recovery-phrase step that completes one.
    SignIn,
    /// The Keychain unlock after a Lock.
    Unlock,
}

/// Keys arrived on this Mac (spec 2026-10-06 §5.3 (2)). The functions that put a `Session` in
/// memory call this right after the engine start, whatever its outcome.
/// `finder_setup_wiring_tests` keeps that list complete.
fn keys_arrived(state: &AppState, from: KeysFrom) {
    // Keys are here: no recovery phrase is owed any more (review P2-3).
    forget_pending_setup(&state.pending_setup);
    if from == KeysFrom::SignIn {
        lifecycle_log::event(lifecycle_log::LifecycleEvent::SignedIn);
    }
    notify_finder(state, finder_setup::core::Trigger::KeysArrived);
}

// ── R8: signing in on a Mac that already holds an account (spec 2026-10-06 §3) ─────────────────────────────────────

/// Where a sign-in looks for what an earlier account left on this computer: the state directory that holds `state.db`.
/// Production resolves the app's own; a test hands in a throwaway one, so it never reads the real one. A state
/// directory that cannot be resolved is carried as the error it is: it counts as a trace.
#[cfg(not(target_os = "windows"))]
struct LocalSources {
    state_dir: Result<PathBuf, String>,
}

#[cfg(not(target_os = "windows"))]
impl LocalSources {
    fn production() -> Self {
        Self {
            state_dir: state_paths::beebeeb_state_dir(),
        }
    }

    #[cfg(test)]
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn for_test(state_dir: &Path) -> Self {
        Self {
            state_dir: Ok(state_dir.to_path_buf()),
        }
    }

    /// `state.db` when the directory has one: `Ok(None)` for none yet, `Err` for a directory or a database that
    /// cannot be read.
    fn state_db(&self) -> Result<Option<state_db::StateDb>, String> {
        match &self.state_dir {
            Ok(dir) => state_db_from_state_dir(dir),
            Err(error) => Err(error.clone()),
        }
    }
}

/// R8 + R10: the recorded owner of this computer's local data: the only account R8 compares a sign-in with.
#[cfg(not(target_os = "windows"))]
fn local_data_owner(sources: &LocalSources) -> Result<Option<account_binding::Identity>, String> {
    match sources.state_db()? {
        Some(db) => db.owner().map_err(|e| format!("read the local data owner: {e}")),
        None => Ok(None),
    }
}

/// Does the Keychain still hold a vault key for this account slot, in the id-keyed or the legacy store? It never
/// unlocks and never reads the key (the real Keychain answers from the item's attributes). Fails closed.
#[cfg(not(target_os = "windows"))]
fn keychain_vault_key_present(account_id: &str) -> bool {
    vault_key_in_either_layout(
        &platform_keychain_store_for(account_id),
        &keychain::legacy_platform_keychain_store(),
    )
}

/// The question over any two stores (the id-keyed layout, then the legacy one), so a test can hold a fake in each. A store that
/// cannot answer counts as holding a key (`keychain::holds_vault_key`'s fail-closed rule): never a reason to say "no key".
#[cfg(not(target_os = "windows"))]
fn vault_key_in_either_layout<A: keychain::AuthSecretStore, B: keychain::AuthSecretStore>(
    id_keyed: &A,
    legacy: &B,
) -> bool {
    keychain::holds_vault_key(id_keyed) || keychain::holds_vault_key(legacy)
}

/// Where "unlock" goes on this Mac (task 1747 Part 2): `Keychain` when the vault key is in the Keychain, `RecoveryPhrase` when
/// neither layout holds one (a session without keys: the first password sign-in that was never finished with the phrase). The
/// window shows Set up this Mac for the second and Home's locked card for the first (plan D4). Serialized snake_case, which is
/// what `vaultUnlockRoute` in `src/desktopApi.ts` parses (pinned equal in `tests/vaultUnlockRoute.test.ts`).
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum VaultUnlockRoute {
    Keychain,
    RecoveryPhrase,
}

/// `key_present` is [`keychain_vault_key_present`]'s answer, which fails closed: a Keychain that cannot be read routes to the
/// Keychain unlock, so the existing unlock error reaches the person, rather than to a path that would overwrite a key.
#[cfg(target_os = "macos")]
fn unlock_route(key_present: bool) -> VaultUnlockRoute {
    if key_present {
        VaultUnlockRoute::Keychain
    } else {
        VaultUnlockRoute::RecoveryPhrase
    }
}

/// The route for one account slot. It reaches the Keychain only through [`keychain_vault_key_present`], which reads the item's
/// attributes and never the key.
#[cfg(target_os = "macos")]
fn vault_unlock_route_for(account_id: &str) -> VaultUnlockRoute {
    unlock_route(keychain_vault_key_present(account_id))
}

/// The Beebeeb window asks this once, when it opens signed in and locked, and again at the click on “Unlock vault” (and
/// Settings › Account's Unlock). An answer about the ACTIVE account, so the window keeps its revision guard on it, and it is
/// not part of `sync_status`, which is polled every second. The Keychain is asked off the async workers; a worker that failed
/// to answer is the Keychain unlock, the fail-closed side.
#[cfg(target_os = "macos")]
#[tauri::command]
async fn vault_unlock_route(state: State<'_, AppState>) -> Result<VaultUnlockRoute, String> {
    let account_id = state.active_account()?.id.as_str().to_string();
    Ok(tokio::task::spawn_blocking(move || vault_unlock_route_for(&account_id))
        .await
        .unwrap_or(VaultUnlockRoute::Keychain))
}

/// Does the Keychain still hold a session token for this account slot (id-keyed or legacy)? A read error counts as
/// present: an unreadable item is a trace, never an absence. (`keychain_session_present` answers the routing
/// question and reads an error as "no session"; this one answers "is anything left?".)
#[cfg(not(target_os = "windows"))]
fn keychain_session_trace(account_id: &str) -> bool {
    keychain::holds_session_token(&platform_keychain_store_for(account_id))
        || keychain::holds_session_token(&keychain::legacy_platform_keychain_store())
}

/// Does the Keychain still hold an account email for this account slot (id-keyed or legacy)? Fails closed like
/// [`keychain_session_trace`].
#[cfg(not(target_os = "windows"))]
fn keychain_email_trace(account_id: &str) -> bool {
    keychain::holds_account_email(&platform_keychain_store_for(account_id))
        || keychain::holds_account_email(&keychain::legacy_platform_keychain_store())
}

/// Changes waiting to upload, or staged copies of them, in the local data. An unreadable `state.db` counts as
/// present (fail closed).
#[cfg(not(target_os = "windows"))]
fn queued_or_staged_present(sources: &LocalSources) -> bool {
    match sources.state_db() {
        Ok(Some(db)) => db.queued_or_staged_count().map(|n| n > 0).unwrap_or(true),
        Ok(None) => false,
        Err(_) => true,
    }
}

/// Is there local data that no account is recorded as owning? Rows of `state.db` (the queue, staged payloads, file
/// rows, cursors, activity: everything `has_account_data` counts, and the cache they point at) in a database whose owner
/// is not recorded. No database yet is no data. A database or a state directory that cannot be read is an error, not an
/// answer: the caller settles nothing (`reauth::SignInKind::Unconfirmed`). Credentials are not data: this is about what
/// a switch would have to purge.
#[cfg(not(target_os = "windows"))]
fn unowned_local_data(sources: &LocalSources) -> Result<bool, String> {
    match sources.state_db()? {
        Some(db) => db
            .has_account_data()
            .map_err(|e| format!("inspect the local data: {e}")),
        None => Ok(false),
    }
}

/// The account this Mac's credentials name, for a Mac with no owner record (a sign-in whose engine has not started):
/// the cached profile's user id and email, else the email of the session in memory, else the Keychain's. A lock that
/// cannot be read names nobody.
#[cfg(not(target_os = "windows"))]
fn credentials_identity(acct: &AccountRuntime) -> account_binding::Identity {
    let (user_id, profile_email) = match acct.cached_profile.lock() {
        Ok(guard) => match guard.as_ref() {
            Some(profile) => (Some(profile.user_id.clone()), Some(profile.email.clone())),
            None => (None, None),
        },
        Err(_) => (None, None),
    };
    let session_email = acct
        .session
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().and_then(|session| session.email.clone()));
    let email = profile_email
        .filter(|email| !email.trim().is_empty())
        .or(session_email)
        .or_else(|| keychain_account_email(acct.id.as_str()));
    account_binding::Identity::new(user_id.as_deref(), email.as_deref())
}

/// R8 + R10: the same account signed in again (decided by `reauth::sign_in_kind`), so the record names it by its
/// user id and the server's own spelling of its email. Never called for another account.
#[cfg(not(target_os = "windows"))]
fn record_local_data_owner(sources: &LocalSources, owner: &account_binding::Identity) -> Result<(), String> {
    match sources.state_db()? {
        Some(db) => db
            .set_owner(owner)
            .map_err(|e| format!("record the local data owner: {e}")),
        None => Ok(()),
    }
}

/// How many changes on this Mac have not uploaded yet: the number in the R8 warning. The queue and the staged copies
/// no queued operation points at, each once (`StateDb::pending_changes_count`). No database yet is none. A count that
/// cannot be read is an error, never 0: the sign-in then settles nothing (`LocalDataUnreadable`), because a warning
/// that says 0 would let the switch delete the only copy of an edit without saying so.
#[cfg(not(target_os = "windows"))]
fn pending_changes_count(sources: &LocalSources) -> Result<u64, String> {
    match sources.state_db()? {
        Some(db) => db
            .pending_changes_count()
            .map_err(|e| format!("count the changes not uploaded: {e}")),
        None => Ok(0),
    }
}

/// Swap the token of the session in memory and keep its keys and email. `false` when no session is in memory. The
/// replaced token is wiped, not just dropped.
#[cfg(not(target_os = "windows"))]
fn replace_session_token_in_memory(_write: &SessionWrite, acct: &AccountRuntime, token: &str) -> Result<bool, String> {
    let mut guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
    match guard.as_mut() {
        Some(session) => {
            let mut replaced = std::mem::replace(&mut session.token, token.to_string());
            zeroize::Zeroize::zeroize(&mut replaced);
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Give the session in memory the server's own spelling of its account's email. A same-account re-sign-in does this
/// so the next engine start compares the identity the owner record holds. `false` when no session is in memory or
/// the email is empty.
#[cfg(not(target_os = "windows"))]
fn name_session_in_memory(_write: &SessionWrite, acct: &AccountRuntime, email: &str) -> Result<bool, String> {
    let email = email.trim();
    if email.is_empty() {
        return Ok(false);
    }
    let mut guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
    match guard.as_mut() {
        Some(session) => {
            session.email = Some(email.to_string());
            Ok(true)
        }
        None => Ok(false),
    }
}

/// What a re-sign-in resets: the revoked-session streak ends, and the session is present again.
#[cfg(not(target_os = "windows"))]
fn reauth_settle_flags(_write: &SessionWrite, state: &AppState, acct: &AccountRuntime, email: &str) {
    acct.auth_health.note_result(None);
    set_auth_present(state, true);
    let email = email.trim();
    if !email.is_empty() {
        set_auth_email(state, Some(email.to_string()));
    }
}

/// What a sign-in found of an earlier account on this Mac.
#[cfg(not(target_os = "windows"))]
struct LocalFacts {
    traces: reauth::LocalTraces,
    /// The recorded owner of the local data, when the record names an account.
    owner: Option<account_binding::Identity>,
    /// Local data that no account is recorded as owning (credentials are not data).
    unowned_data: bool,
    /// The owner record or `state.db` could not be read: nothing can be decided (see `reauth::SignInKind::Unconfirmed`).
    unreadable: bool,
    /// The account the credentials name: used when there is no owner record.
    credentials: account_binding::Identity,
    /// The number in the switch warning. When it cannot be read the facts are `unreadable`, and this is 0.
    pending_changes: u64,
}

/// Every trace of a previous account on this Mac, from its real sources. Reads only: it asks whether a vault key
/// exists and never reads one. An unreadable source (a poisoned lock, an unreadable `state.db`, a Keychain item that
/// answers with an error) counts as a trace, never as an absence: a sign-in is `Fresh` only when NOTHING of an
/// account remains (`reauth::LocalTraces`).
#[cfg(not(target_os = "windows"))]
fn gather_local_facts(state: &AppState, acct: &AccountRuntime, sources: &LocalSources) -> LocalFacts {
    let pending_changes = pending_changes_count(sources);
    // R10: the recorded owner of the local data is the account on this computer.
    let owner = local_data_owner(sources);
    let recorded_owner = !matches!(owner, Ok(None));
    // Every retained trace counts, including what a startup 401 keeps on purpose (R9: the Keychain email and the
    // vault key).
    let traces = reauth::LocalTraces {
        session_in_memory: acct.session.lock().map(|guard| guard.is_some()).unwrap_or(true),
        auth_present: state.auth_present.lock().map(|guard| *guard).unwrap_or(true),
        keychain_token: keychain_session_trace(acct.id.as_str()),
        keychain_email: keychain_email_trace(acct.id.as_str()),
        vault_key: keychain_vault_key_present(acct.id.as_str()),
        recorded_owner,
        cached_profile: acct.cached_profile.lock().map(|guard| guard.is_some()).unwrap_or(true),
        queued_or_staged: pending_changes.as_ref().map_or(true, |count| *count > 0)
            || queued_or_staged_present(sources),
    };
    // An owner record that cannot be read is not "no owner record": it is what makes the sign-in `unreadable`.
    let owner_unreadable = owner.is_err();
    let owner = owner.ok().flatten().filter(account_binding::Identity::is_known);
    let data = if owner.is_some() {
        Ok(false)
    } else {
        unowned_local_data(sources)
    };
    LocalFacts {
        traces,
        owner,
        unowned_data: matches!(data, Ok(true)),
        // The switch warning's count that cannot be read settles nothing either (triage 25).
        unreadable: owner_unreadable || data.is_err() || pending_changes.is_err(),
        credentials: credentials_identity(acct),
        pending_changes: pending_changes.unwrap_or(0),
    }
}

/// R8: what a sign-in as `profile` (the server's own record of the account behind the new session) is on this Mac,
/// and how many changes have not uploaded: the pure decision (`reauth::sign_in_kind`) over the account this Mac holds
/// (the recorded owner of the local data, else what its credentials name) and what else is here. May answer
/// `NeedsKeyProof` (see [`decide_sign_in`]). Reads only.
#[cfg(not(target_os = "windows"))]
fn classify_sign_in(
    state: &AppState,
    acct: &AccountRuntime,
    sources: &LocalSources,
    profile: &account_dto::AccountProfile,
) -> (reauth::SignInKind, u64) {
    classify_sign_in_after(state, acct, sources, profile, None)
}

/// [`classify_sign_in`] with the server's key proof, when one was asked for: the facts are read again (the proof can
/// take a while), and the proof decides only if they still ask for it (`reauth::after_key_proof`).
#[cfg(not(target_os = "windows"))]
fn classify_sign_in_after(
    state: &AppState,
    acct: &AccountRuntime,
    sources: &LocalSources,
    profile: &account_dto::AccountProfile,
    proof: Option<reauth::KeyProof>,
) -> (reauth::SignInKind, u64) {
    let facts = gather_local_facts(state, acct, sources);
    // The account this Mac holds: the recorded owner of the local data, else what its credentials name.
    let account = facts.owner.as_ref().unwrap_or(&facts.credentials);
    let local = reauth::LocalAccount {
        user_id: account.user_id.as_deref(),
        email: account.email.as_deref(),
        traces: facts.traces.any(),
        key_stored: facts.traces.vault_key,
        // A session in memory always holds its vault key. A lock that cannot be read counts as one: the proof then
        // cannot read it either, and settles nothing.
        key_in_memory: facts.traces.session_in_memory,
        unowned_data: facts.unowned_data,
        unreadable: facts.unreadable,
    };
    let kind = match (reauth::sign_in_kind(&local, &profile.user_id, &profile.email), proof) {
        (reauth::SignInKind::NeedsKeyProof, Some(proof)) => {
            reauth::after_key_proof(proof, &local, &profile.user_id, &profile.email)
        }
        (kind, _) => kind,
    };
    (kind, facts.pending_changes)
}

/// The vault key stored in one Keychain layout, for the server proof, as a `MasterKey`: wiped on drop and not `Clone`.
/// The only plain buffer is a `Zeroizing` one that lives inside this function, because the core builds a `MasterKey`
/// from 32 bytes passed by value (it has no constructor that takes a reference, and it wipes the argument it is
/// given). `Ok(None)`: this layout holds no key.
#[cfg(not(target_os = "windows"))]
fn read_vault_key<S: keychain::AuthSecretStore>(
    mut vault: AuthVault<S>,
) -> Result<Option<beebeeb_core::kdf::MasterKey>, String> {
    match vault.unlock() {
        Ok(()) => {}
        Err(keychain::AuthStoreError::NotFound | keychain::AuthStoreError::Unsupported(_)) => return Ok(None),
        Err(error) => return Err(keychain_error("unlock vault key from Keychain", error)),
    }
    let mut raw = zeroize::Zeroizing::new([0u8; 32]);
    raw.copy_from_slice(vault.master_key().map_err(|e| keychain_error("read vault key", e))?);
    Ok(Some(beebeeb_core::kdf::MasterKey::from_bytes(*raw)))
}

/// The vault key this Mac stores (the id-keyed layout, else the legacy one), read for exactly one purpose: the server
/// proof that it belongs to the account that is signing in. A same-account sign-in with no session in memory stores its
/// new token in the id-keyed layout and then loads that layout's key (`reauth_swap_token`, through
/// `load_session_from_keychain`), so the id-keyed key is read first. It is a `MasterKey` from the moment it leaves the
/// Keychain read, so nothing else holds a plain copy.
#[cfg(not(target_os = "windows"))]
fn load_stored_vault_key(account_id: &str) -> Result<Option<beebeeb_core::kdf::MasterKey>, String> {
    if let Some(key) = read_vault_key(AuthVault::new(platform_keychain_store_for(account_id)))? {
        return Ok(Some(key));
    }
    read_vault_key(AuthVault::new(keychain::legacy_platform_keychain_store()))
}

/// The vault key of the session in memory, for the server proof (spec §5.6): a same-account sign-in keeps the key in
/// memory (`reauth_swap_token`), so with a session in memory this is the key that is proved, whatever the Keychain
/// holds. Read under the session lock into a wiped buffer and made a `MasterKey` (wiped on drop, not `Clone`) before
/// the lock is released, as [`read_vault_key`] does. `Ok(None)`: no session in memory. A lock that cannot be read is an
/// error, so the proof settles nothing.
#[cfg(not(target_os = "windows"))]
fn read_session_vault_key(acct: &AccountRuntime) -> Result<Option<beebeeb_core::kdf::MasterKey>, String> {
    let guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
    let Some(session) = guard.as_ref() else {
        return Ok(None);
    };
    let mut raw = zeroize::Zeroizing::new([0u8; 32]);
    raw.copy_from_slice(&session.master_key);
    Ok(Some(beebeeb_core::kdf::MasterKey::from_bytes(*raw)))
}

/// Ask the server whether the vault key this Mac kept is the account behind `token`'s (the same check the recovery
/// phrase goes through). The key proved is the key the sign-in keeps: the key of the session in memory when there is
/// one (`reauth_swap_token` keeps it), else the key stored in the Keychain (see [`load_stored_vault_key`]). Only a
/// completed answer decides: no key, an unreadable key, a client that cannot be built, a network or server error, an
/// expired session:
/// all are `Unavailable`, and nothing is decided. A `400` decides only with an exact code: `invalid_recovery_phrase` is
/// `Mismatch`, `recovery_check_missing` is `NoCheckOnFile`, and any other `400` is `Unavailable`.
#[cfg(not(target_os = "windows"))]
async fn prove_stored_key(acct: &AccountRuntime, base_url: &str, token: &str) -> reauth::KeyProof {
    let kept = match read_session_vault_key(acct) {
        Ok(None) => load_stored_vault_key(acct.id.as_str()),
        in_memory => in_memory,
    };
    let Ok(Some(master_key)) = kept else {
        return reauth::KeyProof::Unavailable;
    };
    let Ok(client) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .default_headers(api_client::provenance_headers())
        .build()
    else {
        return reauth::KeyProof::Unavailable;
    };
    match recovery_check_answer(&client, base_url, token, &master_key).await {
        Ok(RecoveryCheckAnswer::Matches) => reauth::KeyProof::Matches,
        Ok(RecoveryCheckAnswer::Mismatch) => reauth::KeyProof::Mismatch,
        Ok(RecoveryCheckAnswer::NoCheckOnFile) => reauth::KeyProof::NoCheckOnFile,
        Err(_) => reauth::KeyProof::Unavailable,
    }
}

/// R8: what the sign-in is, with the server's proof where the rules ask for one (`NeedsKeyProof`: a vault key is here,
/// and either the user ids match or this Mac's account has no user id). Never `NeedsKeyProof`. Reads only, apart from
/// the one request.
#[cfg(not(target_os = "windows"))]
async fn decide_sign_in(
    state: &AppState,
    acct: &AccountRuntime,
    sources: &LocalSources,
    profile: &account_dto::AccountProfile,
    token: &str,
    base_url: &str,
) -> (reauth::SignInKind, u64) {
    let (kind, pending_changes) = classify_sign_in(state, acct, sources, profile);
    if kind != reauth::SignInKind::NeedsKeyProof {
        return (kind, pending_changes);
    }
    let proof = prove_stored_key(acct, base_url, token).await;
    // The facts are read again after the wait; the caller also refuses when any session transition happened meanwhile.
    classify_sign_in_after(state, acct, sources, profile, Some(proof))
}

/// [`decide_sign_in`] for a sign-in whose turn is `turn`: the decision can wait for the server (the key proof), and if a
/// Lock, a Sign-out or another sign-in completed meanwhile, what it saw is no longer this computer, so nothing is
/// decided (lead ruling 1, Task 12). Nothing is stored yet, so the caller revokes the new session.
#[cfg(not(target_os = "windows"))]
async fn decide_sign_in_in_turn(
    state: &AppState,
    acct: &AccountRuntime,
    sources: &LocalSources,
    profile: &account_dto::AccountProfile,
    token: &str,
    base_url: &str,
    turn: &account::SessionGeneration,
) -> Result<(reauth::SignInKind, u64), ReauthError> {
    let decided = decide_sign_in(state, acct, sources, profile, token, base_url).await;
    if !acct.session_unchanged_since(*turn) {
        return Err(ReauthError::before_store(SessionChanged::of(acct)));
    }
    Ok(decided)
}

/// Does this Mac hold anything of an account? For a sign-in that could not learn which account it is: it cannot be
/// compared, so on a Mac that holds something it is refused (and nothing changes).
#[cfg(not(target_os = "windows"))]
fn sign_in_leaves_account_traces(state: &AppState) -> bool {
    match state.active_account() {
        Ok(acct) => gather_local_facts(state, &acct, &LocalSources::production())
            .traces
            .any(),
        Err(_) => true,
    }
}

/// What a sign-in whose account could not be confirmed says: its record could not be fetched, or the server could not
/// say whether the key stored on this Mac is the signing-in account's. Nothing changed on this computer.
#[cfg(not(target_os = "windows"))]
const SIGN_IN_ACCOUNT_UNKNOWN: &str = "Beebeeb couldn’t confirm which account just signed in, so nothing changed on this computer. Connect to the internet and try again.";

/// What a sign-in says when this computer's own record of its account (the owner record, or `state.db` itself) could
/// not be read (lead ruling 12, Task 12). Nothing changed on this computer; trying again is safe.
#[cfg(not(target_os = "windows"))]
const SIGN_IN_LOCAL_DATA_UNREADABLE: &str = "Beebeeb couldn’t read this Mac’s account data. Try again.";

/// What a successful authentication became on this Mac (R8).
#[cfg(not(target_os = "windows"))]
#[derive(Debug, Clone, PartialEq, Eq)]
enum SignInSettlement {
    Fresh,
    Reauthenticated {
        vault_unlocked: bool,
        /// The vault key this Mac kept was no longer the account's (spec §5.6): it was removed, and the recovery phrase
        /// step follows (`vault_unlocked` is false).
        key_replaced: bool,
    },
    AccountMismatch {
        pending_changes: u64,
        /// On a Mac, the ask snapshot the switch step renders (spec 2026-10-10 W11); elsewhere `None`.
        unsent: Option<sign_out::snapshot::AskCounts>,
        /// On a Mac, the coded E3 when the switch's count could not be read (§5.3 row 1).
        unsent_error: Option<String>,
    },
    /// The server could not say whose key this is: nothing changed, and the new session is revoked.
    Unconfirmed,
}

/// A same-account re-sign-in that failed, and whether the new token had already been written to the Keychain by
/// then. Before that nothing of the new session is stored, so the caller revokes it (as it does for a first
/// sign-in whose token could not be stored); after that the new token is the live one and is kept.
#[cfg(not(target_os = "windows"))]
#[derive(Debug)]
struct ReauthError {
    message: String,
    token_stored: bool,
}

#[cfg(not(target_os = "windows"))]
impl ReauthError {
    fn before_store(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            token_stored: false,
        }
    }

    fn after_store(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            token_stored: true,
        }
    }
}

/// R8: before anything is stored, decide whether this sign-in is fresh, the same account again, or a different
/// account. The same account gets only a new token (`reauth_in_place`), and keeps the vault key this Mac kept only once
/// the server has confirmed it; a key the server no longer accepts is removed instead (`reauth_without_the_kept_key`,
/// spec §5.6). A different account changes nothing here: the caller revokes the new session and returns the warning. A
/// sign-in the server could not place is `Unconfirmed`: nothing changes either. Never reads key material to decide (the
/// one proof reads the kept key into a wiped value). `Fresh` requires that nothing of a previous account remains
/// (`gather_local_facts`). `handoff_key`: the vault key a browser sign-in handed over (`None` for a password sign-in).
/// Not on Windows, where sign-in refuses while a session exists.
#[cfg(not(target_os = "windows"))]
async fn settle_sign_in(
    app: tauri::AppHandle,
    state: &State<'_, AppState>,
    token: &str,
    profile: &account_dto::AccountProfile,
    handoff_key: Option<&[u8; 32]>,
    turn: &mut account::SessionGeneration,
) -> Result<SignInSettlement, ReauthError> {
    let acct = state.active_account().map_err(ReauthError::before_store)?;
    let sources = LocalSources::production();
    let (kind, pending_changes) =
        decide_sign_in_in_turn(state, &acct, &sources, profile, token, &runner::api_base_url(), turn).await?;
    match kind {
        reauth::SignInKind::Fresh => Ok(SignInSettlement::Fresh),
        reauth::SignInKind::DifferentAccount => Ok(switch_settlement(pending_changes, switch_snapshot(state))),
        reauth::SignInKind::NeedsKeyProof | reauth::SignInKind::Unconfirmed => Ok(SignInSettlement::Unconfirmed),
        // Lead ruling 12: this computer's own record of its account could not be read. Its own sentence, not the
        // "connect to the internet" one; nothing is stored, so the caller revokes the new session.
        reauth::SignInKind::LocalDataUnreadable => Err(ReauthError::before_store(SIGN_IN_LOCAL_DATA_UNREADABLE)),
        reauth::SignInKind::SameAccount => {
            let vault_unlocked =
                reauth_in_place(app, state, &acct, &sources, token, profile, handoff_key, turn).await?;
            Ok(SignInSettlement::Reauthenticated {
                vault_unlocked,
                key_replaced: false,
            })
        }
        reauth::SignInKind::KeyOutdated => {
            reauth_without_the_kept_key(state, &acct, &sources, token, profile, turn).await
        }
    }
}

/// W11 and §5.3 row 1 (spec 2026-10-10): the switch settles with what its count found: a snapshot (macOS), none
/// (elsewhere), or E3 for the switch step to show as a line. A count that cannot be read never fails the sign-in:
/// nothing changed, and "Sign out and switch" counts again through the gate.
#[cfg(not(target_os = "windows"))]
fn switch_settlement(
    pending_changes: u64,
    counted: Result<Option<sign_out::snapshot::AskCounts>, String>,
) -> SignInSettlement {
    match counted {
        Ok(unsent) => SignInSettlement::AccountMismatch {
            pending_changes,
            unsent,
            unsent_error: None,
        },
        Err(error) => SignInSettlement::AccountMismatch {
            pending_changes,
            unsent: None,
            unsent_error: Some(error),
        },
    }
}

/// R8, same account: replace ONLY the session token. The queue, the cache, the Finder domain and the vault key are
/// never touched (`reauth_tests` pins it). Keys already in memory stay. Otherwise the key this Mac still holds in the
/// Keychain is loaded (a relaunch after a startup 401). The owner record is rewritten first, with the account's user
/// id and the server's spelling of its email (a legacy record gets both here); if that fails nothing else has
/// changed. Returns whether the vault is unlocked afterwards; `false` sends the person to the recovery phrase.
/// `handoff_key`: the vault key a browser sign-in handed over (`None` for a password sign-in). On a Mac, a sign-in
/// that unlocks asks to resume the edits paused for `auth`, unless the handed-over key replaces the key now in memory:
/// that is decided here, before the ask, because the edits are then held (`key_replaced`) and never sent (F9 re-review
/// N-1).
#[cfg(not(target_os = "windows"))]
fn reauth_swap_token(
    state: &AppState,
    acct: &AccountRuntime,
    sources: &LocalSources,
    token: &str,
    profile: &account_dto::AccountProfile,
    handoff_key: Option<&[u8; 32]>,
    turn: &mut account::SessionGeneration,
) -> Result<bool, ReauthError> {
    let email = session_email(Some(profile));
    // The re-sign-in's turn (lead ruling 1, Task 12): a Lock, a Sign-out or another sign-in that completed since this
    // sign-in began stops it here, before anything is written; everything below is written in this one turn.
    let write = claim_session_write(acct, turn).map_err(ReauthError::before_store)?;
    // R10: the same account (decided by the caller) owns the local data.
    record_local_data_owner(
        sources,
        &account_binding::Identity::new(Some(&profile.user_id), email.as_deref()),
    )
    .map_err(ReauthError::before_store)?;
    persist_session_token_to_keychain(&write, acct.id.as_str(), token, email.as_deref())
        .map_err(ReauthError::before_store)?;
    // From here the new token is the stored one: a failure below is not a sign-in to revoke.
    let mut vault_unlocked = replace_session_token_in_memory(&write, acct, token).map_err(ReauthError::after_store)?;
    if vault_unlocked {
        if let Some(email) = email.as_deref() {
            name_session_in_memory(&write, acct, email).map_err(ReauthError::after_store)?;
        }
    } else {
        match load_session_from_keychain(acct.id.as_str(), email.clone()) {
            Ok(Some(session)) => {
                install_unlocked_session(&write, acct, session).map_err(ReauthError::after_store)?;
                vault_unlocked = true;
            }
            Ok(None) => {}
            Err(error) => {
                tracing::info!(%error, "re-sign-in: no vault key on this Mac; the recovery phrase step follows")
            }
        }
    }
    if let Ok(mut cached) = acct.cached_profile.lock() {
        *cached = Some(profile.clone());
    }
    reauth_settle_flags(&write, state, acct, email.as_deref().unwrap_or_default());
    // Ruling F9: unlocked, so sync resumes; on Linux too since spec D11 (task 1888; Windows does not build this).
    // Without keys here the recovery-phrase unlock asks instead. F9 re-review N-1: a browser sign-in whose handed-over
    // key replaces the kept one is decided here, before the ask, and does not ask: `reauth_in_place` holds what is
    // queued under the kept key as `key_replaced`, and a hold that fails must not leave an open request for a later
    // start to serve with that key. The handoff's own install (`apply_session`) asks once the new key is in memory.
    if vault_unlocked && !handoff_replaces_kept_key(acct, handoff_key) {
        ask_to_resume_auth_paused(&write, acct);
    }
    drop(write);
    Ok(vault_unlocked)
}

/// An engine that did not start after a successful re-sign-in is not a failed sign-in: the session is replaced and the
/// person is signed in. The failure goes where every engine failure goes: the engine status (`sync_status` reads
/// "error") and the Finder reconciler's check that `keys_arrived` asks for, which retries the start and reports it.
#[cfg(not(target_os = "windows"))]
fn note_engine_start_failure(acct: &AccountRuntime, error: &str) {
    tracing::warn!(%error, "re-sign-in: signed in, but the sync engine did not start");
    if let Ok(mut guard) = acct.engine_state.lock() {
        *guard = "error".to_string();
    }
}

/// R8, same account: the token swap above, then, with the keys in memory, a new engine with the new token (the queue in
/// `state.db` is kept and drains) and the reconciler's one "keys are here" trigger. The engine's own failure does not
/// fail the sign-in (see [`note_engine_start_failure`]), so a retry never mints a second server session for an account
/// that is already signed in. A browser sign-in hands over the account's current key (`handoff_key`): when the key in
/// memory differs from it (compared in constant time), no engine starts here and this returns `false`, so the
/// handoff installs its own key (`apply_session`), which starts the engine. On a Mac a browser sign-in swaps the token
/// while holding the engine slot, and when its key replaces the kept one, the outgoing engine stops and the edits queued
/// under the kept key are held (`key_replaced`) before the slot is released (F9 review M2, lead ruling I-1). A hold that
/// cannot be written fails the sign-in, with the new token already stored. The swap of such a sign-in opened no resume
/// request (F9 re-review N-1), so a failed hold leaves none open for a start in this process to serve.
#[cfg(not(target_os = "windows"))]
#[allow(clippy::too_many_arguments)]
async fn reauth_in_place(
    app: tauri::AppHandle,
    state: &State<'_, AppState>,
    acct: &AccountRuntime,
    sources: &LocalSources,
    token: &str,
    profile: &account_dto::AccountProfile,
    handoff_key: Option<&[u8; 32]>,
    turn: &mut account::SessionGeneration,
) -> Result<bool, ReauthError> {
    // F9 review M2 (lead ruling I-1): on a Mac, a browser sign-in swaps the token while holding the engine slot, so no
    // start serves the swap's ask before the handed-over key is compared with the kept one below.
    #[cfg(target_os = "macos")]
    let mut engine_slot = if handoff_key.is_some() {
        Some(acct.engine.lock().await)
    } else {
        None
    };
    let vault_unlocked = reauth_swap_token(state, acct, sources, token, profile, handoff_key, turn)?;
    if vault_unlocked && handoff_replaces_kept_key(acct, handoff_key) {
        tracing::info!("browser sign-in: the vault key this Mac kept is not the one handed over; installing that one");
        // Ruling I-1: nothing queued under the kept key is sent. Held before the slot is released.
        #[cfg(target_os = "macos")]
        if let Some(slot) = engine_slot.as_mut() {
            stop_the_outgoing_engine_and_hold_auth_paused(acct, sources, slot)
                .await
                .map_err(|error| {
                    tracing::warn!(%error, "browser sign-in: the operations queued under the kept key could not be held");
                    ReauthError::after_store(SIGN_IN_LOCAL_DATA_UNREADABLE)
                })?;
        }
        return Ok(false);
    }
    #[cfg(target_os = "macos")]
    drop(engine_slot);
    if vault_unlocked {
        let started = start_engine_if_possible(app, state).await;
        keys_arrived(state, KeysFrom::SignIn);
        if let Err(error) = started {
            note_engine_start_failure(acct, &error);
        }
    }
    tracing::info!(vault_unlocked, "signed in again in place (R8)");
    Ok(vault_unlocked)
}

/// Constant-time equality of two vault keys: every byte is compared, wherever the first difference is.
#[cfg(not(target_os = "windows"))]
fn vault_keys_equal(a: &[u8; 32], b: &[u8; 32]) -> bool {
    let difference = a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y));
    std::hint::black_box(difference) == 0
}

/// Does the session in memory hold exactly `key`? Compared in constant time, under the session lock. No session, or a
/// lock that cannot be read: `false`.
#[cfg(not(target_os = "windows"))]
fn session_holds_vault_key(acct: &AccountRuntime, key: &[u8; 32]) -> bool {
    acct.session
        .lock()
        .map(|guard| {
            guard
                .as_ref()
                .is_some_and(|session| vault_keys_equal(&session.master_key, key))
        })
        .unwrap_or(false)
}

/// A browser sign-in handed over `handoff_key`, the account's current key (spec §5.6). Does it replace what the session
/// in memory holds? Yes unless memory holds exactly that key. `None` (a password sign-in hands over no key): no.
#[cfg(not(target_os = "windows"))]
fn handoff_replaces_kept_key(acct: &AccountRuntime, handoff_key: Option<&[u8; 32]>) -> bool {
    handoff_key.is_some_and(|handoff| !session_holds_vault_key(acct, handoff))
}

/// What a re-sign-in says when the outdated vault key could not be removed from this Mac. The new session is revoked
/// and nothing is installed; trying again is safe.
#[cfg(not(target_os = "windows"))]
const SIGN_IN_KEY_NOT_REMOVED: &str =
    "Beebeeb couldn’t remove this account’s old vault key from this computer, so nothing changed. Try again.";

/// Remove the vault key, and only the key, from both Keychain layouts (the id-keyed and the legacy one), then check
/// that neither still holds one. Only inside a checked turn (`_write`). A key that is not there, or a store that cannot
/// hold secrets, is no failure; a key that cannot be removed, or still reads as present, is.
#[cfg(not(target_os = "windows"))]
fn delete_vault_key_from_keychain(_write: &SessionWrite, account_id: &str) -> Result<(), String> {
    use keychain::AuthSecretStore as _;
    for removed in [
        platform_keychain_store_for(account_id).delete_wrapped_master_key(),
        keychain::legacy_platform_keychain_store().delete_wrapped_master_key(),
    ] {
        match removed {
            Ok(()) | Err(keychain::AuthStoreError::NotFound | keychain::AuthStoreError::Unsupported(_)) => {}
            Err(error) => {
                tracing::warn!(error = %error, "re-sign-in: the outdated vault key could not be removed");
                return Err(SIGN_IN_KEY_NOT_REMOVED.to_string());
            }
        }
    }
    if keychain_vault_key_present(account_id) {
        return Err(SIGN_IN_KEY_NOT_REMOVED.to_string());
    }
    Ok(())
}

/// [`reauth_without_the_kept_key`]'s writes, in the sign-in's turn and under the engine slot (`_engine_slot`), so no
/// engine start can read keys in between (every start reads its keys under the slot). The session in memory is cleared
/// first, as Lock clears it; then the owner is recorded, the outdated key is removed from both Keychain layouts, and
/// the new token is stored with the account's email and no key. A failure up to and including the token write is
/// `before_store` (the caller revokes the new session); the memory clear stands either way.
#[cfg(not(target_os = "windows"))]
fn reauth_swap_token_without_key(
    state: &AppState,
    acct: &AccountRuntime,
    sources: &LocalSources,
    token: &str,
    profile: &account_dto::AccountProfile,
    turn: &mut account::SessionGeneration,
    _engine_slot: &tokio::sync::MutexGuard<'_, Option<EngineRunner>>,
) -> Result<(), ReauthError> {
    let email = session_email(Some(profile));
    let write = claim_session_write(acct, turn).map_err(ReauthError::before_store)?;
    match acct.session.lock() {
        Ok(mut guard) => drop(guard.take()),
        Err(poisoned) => {
            drop(poisoned.into_inner().take());
            acct.session.clear_poison();
        }
    }
    record_local_data_owner(
        sources,
        &account_binding::Identity::new(Some(&profile.user_id), email.as_deref()),
    )
    .map_err(ReauthError::before_store)?;
    // Lead ruling, F9 review I-1: in this turn, under this slot, and before the key is removed, the edits queued under
    // that key are held (kept, never sent). If they cannot be, nothing more changes and the sign-in can be tried again.
    #[cfg(target_os = "macos")]
    if let Err(error) = hold_auth_paused_for_the_replaced_key(sources, _engine_slot) {
        tracing::warn!(%error, "re-sign-in: the operations queued under the replaced key could not be held");
        return Err(ReauthError::before_store(SIGN_IN_LOCAL_DATA_UNREADABLE));
    }
    delete_vault_key_from_keychain(&write, acct.id.as_str()).map_err(ReauthError::before_store)?;
    persist_session_token_to_keychain(&write, acct.id.as_str(), token, email.as_deref())
        .map_err(ReauthError::before_store)?;
    if let Ok(mut cached) = acct.cached_profile.lock() {
        *cached = Some(profile.clone());
    }
    reauth_settle_flags(&write, state, acct, email.as_deref().unwrap_or_default());
    drop(write);
    Ok(())
}

/// R8, same account, but the server says the vault key this Mac kept is no longer the account's: it was changed on
/// another device (spec §5.6). That key must never reach an engine again. The sign-in's turn is taken first, so every
/// transition that began before it is refused from now on, as a Lock's is. A session in memory is then ended the way
/// Lock ends it: on a Mac the reconciler cancels any check and holds (it never removes the domain), and the engine
/// stops under its slot. Still under the slot, [`reauth_swap_token_without_key`] clears memory, removes the key and
/// stores the new token; on a Mac it first holds the edits queued under that key (`key_replaced`: kept, never sent; lead
/// ruling, F9 review I-1). The queue, the cache and Finder stay; the recovery phrase step follows (`vault_unlocked` is
/// false and `key_replaced` says why), and its server check stores the current key.
#[cfg(not(target_os = "windows"))]
async fn reauth_without_the_kept_key(
    state: &AppState,
    acct: &AccountRuntime,
    sources: &LocalSources,
    token: &str,
    profile: &account_dto::AccountProfile,
    turn: &mut account::SessionGeneration,
) -> Result<SignInSettlement, ReauthError> {
    drop(claim_session_write(acct, turn).map_err(ReauthError::before_store)?);
    let keys_in_memory = acct.session.lock().map(|guard| guard.is_some()).unwrap_or(true);
    #[cfg(target_os = "macos")]
    if keys_in_memory && let Err(error) = finder_lock_for(state).await {
        tracing::warn!(%error, "the Finder reconciler did not acknowledge the hold; removing the outdated key anyway");
    }
    let mut engine_slot = acct.engine.lock().await;
    if let Some(engine) = engine_slot.take()
        && !stop_engine_in_slot(acct, engine).await.is_stopped()
    {
        tracing::error!("the sync engine did not confirm it stopped before the outdated vault key was removed");
    }
    if keys_in_memory {
        purge_macos_hydrate_cache("key-outdated");
    }
    let swapped = reauth_swap_token_without_key(state, acct, sources, token, profile, turn, &engine_slot);
    drop(engine_slot);
    if let Ok(mut engine_state) = acct.engine_state.lock() {
        *engine_state = "stopped".to_string();
    }
    if keys_in_memory {
        bump_vault_epoch();
    }
    tracing::info!(
        stored = swapped.is_ok(),
        "signed in again in place without the vault key this Mac kept (spec §5.6)"
    );
    swapped.map(|()| SignInSettlement::Reauthenticated {
        vault_unlocked: false,
        key_replaced: true,
    })
}

/// How long Lock waits for the reconciler to acknowledge it (spec §5.3 (3); lead ruling T8-⚠lock).
///
/// The reconciler handles an event only between operations, so the wait must outlast the longest one
/// it can be inside. That is a `StartEngine` whose socket never comes up: it waits for the engine slot
/// (up to 15 s), then the socket (3 s), then stops the engine it started: the slot again (15 s) and
/// `abort()`'s own 3 s + 2 s windows. 15 + 3 + 15 + 5 = 38 s. A shorter wait would make a slow but
/// working check look like a lock the reconciler never acknowledged.
/// `the_acknowledgement_timeouts_outlast_the_longest_operation_they_can_wait_behind` pins the sum.
#[cfg(target_os = "macos")]
const FINDER_LOCK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);

/// How long Sign-out and Repair wait for the reconciler's removal: the longest operation it can be
/// inside (38 s, see [`FINDER_LOCK_TIMEOUT`]) and then its own `RemoveDomain` (15 s) = 53 s.
#[cfg(target_os = "macos")]
const FINDER_REMOVE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

// A test shortens the waits for the one test that needs a wait to run out, without a global that
// other tests would see: it is scoped to the future that runs the command.
#[cfg(all(test, target_os = "macos"))]
tokio::task_local! {
    static FINDER_WAIT_OVERRIDE: std::time::Duration;
}

// A test learns the moment `lock_vault` (or a sign-out) holds the engine slot from these signals, never by
// polling the slot (task 1891): anything else that briefly holds it looks the same to a poll, such as the
// stand-in reconciler's own look at the slot when it handles the Lock or the removal, and a start that was
// released on that poll gets the slot BEFORE the lock does. Each is scoped to the future that runs the command,
// like the waits above, only signals (it never waits, and it never holds anything), and is compiled out of
// every build that is not a macOS test build.
#[cfg(all(test, target_os = "macos"))]
tokio::task_local! {
    static LOCK_HOLDS_THE_ENGINE_SLOT: std::sync::Arc<tokio::sync::Notify>;
    // A channel, because the sign-out test runs the command on its own thread and waits for this on another.
    static SIGN_OUT_HOLDS_THE_ENGINE_SLOT: std::sync::mpsc::Sender<()>;
}

#[cfg(all(test, target_os = "macos"))]
fn signal_lock_holds_the_engine_slot() {
    let _ = LOCK_HOLDS_THE_ENGINE_SLOT.try_with(|signal| signal.notify_one());
}

#[cfg(all(test, target_os = "macos"))]
fn signal_sign_out_holds_the_engine_slot() {
    let _ = SIGN_OUT_HOLDS_THE_ENGINE_SLOT.try_with(|signal| signal.send(()));
}

#[cfg(target_os = "macos")]
fn finder_wait(production: std::time::Duration) -> std::time::Duration {
    #[cfg(test)]
    if let Ok(short) = FINDER_WAIT_OVERRIDE.try_with(|wait| *wait) {
        return short;
    }
    production
}

/// Shown when Lock could not confirm the reconciler stopped (it did not acknowledge in time, or it has
/// stopped). The keys are already cleared and the engine stopped; the sentence says only what is
/// unconfirmed. No OS text, no path (lead ruling T1-4). A warning on a Lock that happened (FB-24).
const FINDER_LOCK_UNCONFIRMED_WARNING: &str =
    "The vault is locked, but Beebeeb could not confirm that its Finder setup stopped. Restart Beebeeb to be sure.";

/// The same for Sign-out: signed out, keys cleared, but the removal from Finder is unconfirmed.
const FINDER_SIGN_OUT_UNCONFIRMED_WARNING: &str = "You are signed out, but Beebeeb could not confirm that it was removed from Finder. If it still shows there, restart Beebeeb and sign out again.";

/// Lock when the sync engine's stop is unconfirmed (an earlier stop never was, or this one is not):
/// its task may still hold the vault key, so this is never a plain success (lead ruling T8-lockflag).
const LOCK_ENGINE_UNCONFIRMED_WARNING: &str =
    "The vault is locked, but the sync engine did not confirm it stopped. Restart Beebeeb before unlocking again.";

const ENGINE_START_REFUSED_ERROR: &str =
    "Could not start the sync engine: a previous one did not confirm it stopped. Restart Beebeeb.";

/// Lead ruling T8-lockflag: every place that puts an engine in the slot calls this with the slot
/// held, so two engines can never run at once. The flag is set when an engine's stop could not be
/// confirmed (its task may still be running with the keys) and stays set until a restart.
/// `finder_setup_wiring_tests` keeps the list of starts complete.
fn engine_start_refusal(acct: &account::AccountRuntime) -> Result<(), String> {
    if acct.engine_stop_unconfirmed.load(Ordering::SeqCst) {
        tracing::error!("engine start refused: an earlier stop was never confirmed; restart Beebeeb");
        // Spec §5.6: the refusal has its own code and sentence on `sync_status.engine_refusal`, so the surfaces say to
        // quit and reopen instead of offering a "Try again" that cannot work. Cleared like the binding refusals, by
        // every session transition (no start runs while the flag is set).
        *acct
            .engine_refusal
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(account_binding::Refusal::EngineStopUnconfirmed);
        return Err(ENGINE_START_REFUSED_ERROR.to_string());
    }
    Ok(())
}

/// The one way the in-memory session is cleared for a Lock or a Sign-out (lead ruling T10-lockorder). It takes
/// the engine-slot guard as a parameter, so the clear can only be written while the caller still holds the
/// slot: "release the slot, then clear the session" would be a use of a moved guard and does not compile. That
/// order is what keeps a start that was waiting for the slot from finding keys the clear was about to remove
/// (every start reads its keys under the slot). A poisoned mutex still guards the master key: it is recovered
/// and cleared, never skipped (and cleared of its poison, so the next unlock can install a session). Returns the token
/// of the session it took (task 1886: a sign-out ends it on the server; Lock, which keeps the session, drops it).
fn clear_session_holding_slot(
    acct: &account::AccountRuntime,
    _engine_slot: &tokio::sync::MutexGuard<'_, Option<EngineRunner>>,
    by: account::SessionTransition,
) -> Result<Option<zeroize::Zeroizing<String>>, String> {
    // Every session transition that began before this point is refused from now on (Task 12, lead ruling 1): one that
    // put its session in memory before it is cleared just below, and one that has not yet written never will. The
    // session-write lock is not held across the clear (`bump_vault_epoch` must never wait under it).
    drop(end_sessions_in_flight(acct, by));
    let taken = match acct.session.lock() {
        Ok(mut guard) => {
            let taken = guard.take();
            bump_vault_epoch();
            tracing::info!("runtime session cleared");
            taken
        }
        #[cfg(target_os = "windows")]
        Err(_) => return Err("Could not clear the runtime session. The vault is not locked; restart Beebeeb.".into()),
        #[cfg(not(target_os = "windows"))]
        Err(poisoned) => {
            let taken = poisoned.into_inner().take();
            acct.session.clear_poison();
            bump_vault_epoch();
            tracing::warn!("the session mutex was poisoned; the runtime session was cleared anyway");
            taken
        }
    };
    // `Session` wipes itself on drop; its token moves out first, into a value that wipes itself too.
    Ok(taken.map(|mut session| zeroize::Zeroizing::new(std::mem::take(&mut session.token))))
}

/// The one way an engine leaves its slot to be stopped (Task 9 fix round 1, item A; lead ruling
/// T8-lockflag). The caller holds the engine slot and has already taken the handle out of it, so an
/// unconfirmed stop leaves an EMPTY slot while the task may still run with the keys. This sets
/// `engine_stop_unconfirmed` before it returns, so before the caller can release the slot, and every
/// start refuses from then on. Repair and the respawn inside a start used to abort without recording it,
/// and the next start (on a Mac, Repair's own follow-up check) spawned a second engine beside the first.
/// `every_engine_stop_goes_through_the_one_helper_and_every_respawn_refuses_after_it` keeps it the only
/// abort in the app.
async fn stop_engine_in_slot(acct: &account::AccountRuntime, engine: EngineRunner) -> runner::AbortOutcome {
    let outcome = engine.abort().await;
    if !outcome.task_confirmed() {
        acct.engine_stop_unconfirmed.store(true, Ordering::SeqCst);
        tracing::error!(
            "the sync engine's stop could not be confirmed; every engine start refuses until Beebeeb restarts"
        );
    }
    outcome
}

/// Repair's warning when its engine stop is unconfirmed (the same advice as Lock's error). Windows and Linux only: a
/// Mac's surfaces render `engine_stop_unconfirmed`'s fixed sentence instead.
#[cfg(not(target_os = "macos"))]
const REPAIR_ENGINE_UNCONFIRMED_WARNING: &str =
    "The sync engine did not confirm it stopped. Restart Beebeeb before syncing again.";

/// What Repair's engine stop did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RepairEngineStop {
    /// The stale IPC socket was there and is gone.
    removed_socket: bool,
    /// An engine stop on this account is unconfirmed after this one (see [`stop_engine_for_repair`]).
    engine_stop_unconfirmed: bool,
}

/// Repair's engine stop (spec §5.3 (6)): stops the engine and, on Windows and Linux, says so in `warnings` when the
/// stop cannot be confirmed (a Mac's surfaces render the reported flag's fixed sentence instead, so its `warnings`
/// do not repeat it). On a Mac the reconciler then runs its follow-up check, which starts an engine again
/// with the keys still in memory; the flag the helper sets is what makes that start refuse instead of
/// running a second engine beside one that may still run. Reports that flag as it is AFTER this stop, so it
/// covers this stop and an earlier one (a Lock, a Sign-out or a check) that left the slot empty: Repair
/// reports it as `engine_stop_unconfirmed` (spec §5.6). The stale IPC socket is removed (`remove_socket`, which is
/// `remove_stale_ipc_socket` in production) while the slot is still held: an engine that a waiting check starts as
/// soon as the slot is free binds a fresh socket, and Repair must never delete that one.
async fn stop_engine_for_repair(
    acct: &account::AccountRuntime,
    warnings: &mut Vec<String>,
    remove_socket: impl FnOnce() -> Result<bool, String>,
) -> RepairEngineStop {
    let mut engine_slot = acct.engine.lock().await;
    if let Some(prev) = engine_slot.take() {
        // Task 1538 Codex P1 — see `start_engine_if_possible`'s identical respawn guard.
        let outcome = stop_engine_in_slot(acct, prev).await;
        if outcome.is_stopped() {
            tracing::info!("engine aborted for macOS integration reset");
        } else {
            tracing::warn!("engine did not confirm termination before macOS integration reset");
            #[cfg(not(target_os = "macos"))]
            if !outcome.task_confirmed() {
                warnings.push(REPAIR_ENGINE_UNCONFIRMED_WARNING.to_string());
            }
        }
    }
    let removed_socket = match remove_socket() {
        Ok(removed) => removed,
        Err(error) => {
            warnings.push(error);
            false
        }
    };
    drop(engine_slot);
    if let Ok(mut guard) = acct.engine_state.lock() {
        *guard = "stopped".to_string();
    }
    RepairEngineStop {
        removed_socket,
        engine_stop_unconfirmed: acct.engine_stop_unconfirmed.load(Ordering::SeqCst),
    }
}

/// Why an engine start has no keys to start with.
#[derive(Debug, PartialEq, Eq)]
enum EngineStartBlocked {
    /// An engine stop was never confirmed (the sentence says to restart), or the session mutex is unusable.
    Refused(String),
    /// Nothing is signed in with keys in memory: locked, signed out, or a Lock that won the slot first.
    NoSession,
}

/// Shown when a start finds the vault locked at the moment it holds the engine slot.
const ENGINE_START_NO_SESSION: &str = "The vault is locked, so sync didn’t start.";

impl std::fmt::Display for EngineStartBlocked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(sentence) => f.write_str(sentence),
            Self::NoSession => f.write_str(ENGINE_START_NO_SESSION),
        }
    }
}

/// The keys every engine start uses, and the identity of the session they belong to, read together UNDER
/// the engine slot (lead rulings T8-⚠lock, T9-starts and T10-identity). Lock and Sign-out clear the session
/// before they release the slot, so a start that was waiting for the slot finds nothing here instead of
/// starting an engine with keys that were just cleared. No start receives keys from its caller, so there
/// is nothing to read them from but this. The identity comes from the SAME `Session` (its own email, and
/// the cached profile only when that profile has that exact email), never from separate fields that a
/// sign-in updates at another moment. Refuses first while an earlier engine stop is unconfirmed. The
/// copies are `Zeroizing`, so none outlives the engine start that takes them.
fn keys_for_engine_start(
    acct: &account::AccountRuntime,
) -> Result<(EngineKeys, account_binding::Identity), EngineStartBlocked> {
    engine_start_refusal(acct).map_err(EngineStartBlocked::Refused)?;
    let (keys, email) = acct
        .session
        .lock()
        .map_err(|_| EngineStartBlocked::Refused("session mutex poisoned".to_string()))?
        .as_ref()
        .map(|session| {
            (
                (
                    zeroize::Zeroizing::new(session.token.clone()),
                    zeroize::Zeroizing::new(session.master_key),
                ),
                session.email.clone(),
            )
        })
        .ok_or(EngineStartBlocked::NoSession)?;
    let profile = acct.cached_profile.lock().ok().and_then(|guard| guard.clone());
    Ok((keys, identity_of_session(email.as_deref(), profile.as_ref())))
}

/// The session token and master key an engine start hands to the engine, wiped when dropped.
type EngineKeys = (zeroize::Zeroizing<String>, zeroize::Zeroizing<[u8; 32]>);

/// What a bound engine start came to (see [`spawn_bound_engine`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EngineStart {
    Started,
    /// The vault was locked (or signed out) by the time this start held the engine slot, so no engine
    /// was started and nothing was bound.
    NoSession,
    /// A Finder domain removal is owed (macOS): the previous account's domain is still registered, and no engine
    /// may start until a removal is confirmed. Nothing was started; the reconciler removes the domain and its next
    /// check starts the engine.
    FinderRemovalOwed,
}

/// What [`authorize_engine_start`] decided. Only `Go` carries the keys.
enum StartPermit {
    Go(EngineKeys),
    NoSession,
    FinderRemovalOwed,
}

/// Never prints the keys (a failing assertion formats its operands).
impl std::fmt::Debug for StartPermit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Go(_) => f.write_str("Go(<keys>)"),
            Self::NoSession => f.write_str("NoSession"),
            Self::FinderRemovalOwed => f.write_str("FinderRemovalOwed"),
        }
    }
}

const ENGINE_SLOT_OCCUPIED_ERROR: &str = "Could not start the sync engine: one is already running.";

/// The reconciler's start found no keys under the slot (a Lock won it, or nothing is signed in).
#[cfg(target_os = "macos")]
const ENGINE_START_NO_SESSION_FOR_FINDER: &str = "Unlock the vault before adding Beebeeb to Finder.";

/// A start that finds a Finder removal owed: shown to the reconciler's checks (which retry) and never to a person
/// as a failure of their sign-in (`start_engine_if_possible` treats it as "deferred").
#[cfg(target_os = "macos")]
const FINDER_REMOVAL_OWED_ERROR: &str =
    "Beebeeb is still removing the previous account from Finder. Sync starts when that is done.";

/// R10 (spec 2026-10-06 §5.6): the binding's view of this computer's local data.
struct StateDbLocalData {
    db: state_db::StateDb,
    sync_root: Option<PathBuf>,
    /// Directories whose plaintext copies the reset sweeps as a whole (see `purge_local_state_files_in`).
    staging_dirs: Vec<PathBuf>,
    /// Spec 2026-10-10 W12: what the reset's keep step kept (macOS), from the keep until the binding hands its folder
    /// on (`hand_on_reset_kept`), whether or not the purge after it finished.
    kept: std::sync::Mutex<Option<sign_out::kept_set::KeptSet>>,
}

/// Where an engine start looks: the app-local state dir (`state.db`) and the sync root the engine will use.
/// Production resolves the state dir from the process-wide setting; a test passes its own throwaway one.
struct LocalDataPaths {
    state_dir: PathBuf,
    sync_root: PathBuf,
    staging_dirs: Vec<PathBuf>,
}

impl LocalDataPaths {
    fn production(sync_root: &Path) -> Result<Self, String> {
        Ok(Self {
            state_dir: state_paths::beebeeb_state_dir()?,
            sync_root: sync_root.to_path_buf(),
            staging_dirs: production_staging_dirs(),
        })
    }

    #[cfg(test)]
    fn for_test(state_dir: &Path, sync_root: &Path) -> Self {
        Self {
            state_dir: state_dir.to_path_buf(),
            sync_root: sync_root.to_path_buf(),
            staging_dirs: Vec::new(),
        }
    }
}

impl StateDbLocalData {
    /// Opens `state.db` the way the engine will, and creates it if needed, so the owner of a first engine
    /// start is recorded before that engine writes anything. On a first start the state-dir migration runs
    /// here first; the runner's own call then finds `state.db` and skips it.
    fn for_engine_start(paths: &LocalDataPaths) -> Result<Self, String> {
        let path = state_paths::state_db_path_from_state_dir(&paths.state_dir);
        if !path.exists() {
            state_paths::prepare_beebeeb_state_dir_at(&paths.sync_root, &paths.state_dir)?;
        }
        let db = state_db::StateDb::open(&path).map_err(|e| format!("open state.db: {e}"))?;
        // An engine start shuts the upgrade's adoption window on every database it opens: one it creates (or
        // migrates in from an older layout) was never the upgrade's, and an existing one has had its startup pass.
        // Nothing is adopted into either afterwards.
        db.close_adoption_window().map_err(|e| format!("mark state.db: {e}"))?;
        Ok(Self {
            db,
            sync_root: Some(paths.sync_root.clone()),
            staging_dirs: paths.staging_dirs.clone(),
            kept: Default::default(),
        })
    }
}

impl account_binding::LocalData for StateDbLocalData {
    fn owner(&self) -> Result<Option<account_binding::Identity>, String> {
        self.db.owner().map_err(|e| format!("read the local data owner: {e}"))
    }

    fn has_account_data(&self) -> Result<bool, String> {
        self.db
            .has_account_data()
            .map_err(|e| format!("inspect the local data: {e}"))
    }

    /// Spec 2026-10-10 W12: keep (no send, no ask: every missing write and other change is acknowledged), then the
    /// sign-out purge with the guard (its one deletion gate, `is_disposable_cache_path`, never reaches the lifecycle
    /// log directory), then every remaining row. A failed keep fails the reset, and nothing is deleted. An error from
    /// any step means nothing may start.
    fn reset(&self) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        let kept = Some(keep_in(
            &self.db,
            sign_out::keep::Acknowledge::Everything,
            KeepCaller::Reset,
        )?);
        #[cfg(not(target_os = "macos"))]
        let kept: Option<sign_out::kept_set::KeptSet> = None;
        // W12: the folder is recorded now, so it is handed on to be surfaced whether or not the purge below then
        // finishes.
        *self.kept.lock().unwrap_or_else(|p| p.into_inner()) = kept.clone();
        // Spec 2026-10-10 §15 question 3: a reset that stops without purging reopens Finder accepts at once (the keep
        // step already reopens them on its own failure).
        let reopen = || {
            #[cfg(target_os = "macos")]
            if let Err(error) = self.db.reopen_finder_accepts() {
                tracing::warn!(%error, "could not reopen Finder accepts; the next engine start reopens them");
            }
        };
        let summary =
            match purge_local_state_files_in(&self.db, self.sync_root.as_deref(), &self.staging_dirs, kept.as_ref()) {
                Ok(summary) => summary,
                Err(error) => {
                    reopen();
                    return Err(error);
                }
            };
        tracing::info!(
            queued_ops_purged = summary.queued_ops_purged,
            files_removed = summary.files_removed,
            files_skipped = summary.files_skipped,
            others_dropped = kept.as_ref().map_or(0, |kept| kept.others_acknowledged.len()),
            "R10: local data reset before sync started"
        );
        // W13: the purge cleared the queue's rows, so no write can look missing any more. Best effort (§7.3), as the
        // sign-out's: the warning names the step that failed, `path`, `read` or `write`, never an error's text (W10).
        #[cfg(target_os = "macos")]
        if let Err(step) = DesktopConfig::path()
            .map_err(|_| "path".to_string())
            .and_then(|path| sign_out::kept_record::clear_unfinished_at(&path))
        {
            tracing::warn!(step = %step, "could not clear the unfinished sign-out marker");
        }
        // On a Mac the previous account's Finder domain is still registered: the debt is recorded in the same
        // transaction that clears the rows, so no crash can leave the rows gone and the debt forgotten.
        self.db.clear_account_data(cfg!(target_os = "macos")).map_err(|e| {
            reopen();
            format!("clear the local data: {e}")
        })
    }

    fn record_owner(&self, owner: &account_binding::Identity) -> Result<(), String> {
        self.db
            .set_owner(owner)
            .map_err(|e| format!("record the local data owner: {e}"))
    }

    fn adopt_once(&self, candidate: Option<&account_binding::Identity>) -> Result<bool, String> {
        self.db
            .adopt_owner_once(candidate)
            .map_err(|e| format!("adopt the local data owner: {e}"))
    }
}

/// R10: who a session is, from the session's OWN email and, when the cached profile is that account's, its
/// user id. The profile counts only when its email is exactly the session's, so a profile left over from
/// another account is never taken for this one; a session with no email is unidentified (and an owner that
/// cannot be compared with it refuses the start).
fn identity_of_session(
    email: Option<&str>,
    profile: Option<&account_dto::AccountProfile>,
) -> account_binding::Identity {
    let email = email.map(str::trim).filter(|email| !email.is_empty());
    let profile = profile.filter(|profile| email.is_some_and(|email| profile.email.trim() == email));
    account_binding::Identity::new(profile.map(|profile| profile.user_id.as_str()), email)
}

/// R10: bind `data` to `session` before an engine starts. Another account's data is reset first
/// (refused on Windows). An owner that cannot be compared stops the start and deletes nothing. Any
/// error means: do not start the engine.
fn bind_data_to_session(
    state: &AppState,
    data: &StateDbLocalData,
    session: &account_binding::Identity,
) -> Result<(), String> {
    let bound = account_binding::bind_before_engine_start(data, session, !cfg!(target_os = "windows"));
    // W12: a reset that kept files hands its folder on whatever the binding answered. A reset whose purge failed after
    // a completed keep returns an error, and the folder it recorded is on disk either way.
    #[cfg(target_os = "macos")]
    hand_on_reset_kept(state, data);
    match bound? {
        account_binding::Bound::Kept => {}
        account_binding::Bound::Reset => {
            tracing::warn!("R10: local data of another account was reset before sync started");
            // Finder may still list what the reset removed. A Repair removes Beebeeb and adds it back.
            // Never awaited: the reconciler itself starts engines through this function.
            #[cfg(target_os = "macos")]
            notify_finder(state, finder_setup::core::Trigger::Repair);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = state;
    Ok(())
}

/// W12: an R10 reset that kept files hands its folder to the state, to be surfaced once the start returns (the row,
/// then one alert). Taken out, so it is surfaced once.
#[cfg(target_os = "macos")]
fn hand_on_reset_kept(state: &AppState, data: &StateDbLocalData) {
    if let Some(kept) = data.kept.lock().unwrap_or_else(|p| p.into_inner()).take()
        && let Some(folder) = kept.folder
    {
        *state.sign_out.reset_kept.lock().unwrap_or_else(|p| p.into_inner()) =
            Some((folder.to_string_lossy().into_owned(), kept.files));
    }
}

/// Whether the engine may start once the local data is bound.
enum BoundData {
    Ready,
    /// A Finder domain removal is owed (see [`EngineStart::FinderRemovalOwed`]).
    FinderRemovalOwed,
}

/// R10: bind this computer's local data to the session's account before an engine starts. A start refuses while a
/// Finder domain removal is owed, before it binds anything; and a reset on a Mac, which leaves the previous
/// account's domain registered, records that debt itself, so the engine waits for the removal Repair asks for.
fn bind_local_data_to_session(
    state: &AppState,
    identity: &account_binding::Identity,
    paths: &LocalDataPaths,
) -> Result<BoundData, String> {
    let data = StateDbLocalData::for_engine_start(paths)?;
    if data
        .db
        .finder_removal_owed()
        .map_err(|e| format!("read the Finder removal debt: {e}"))?
    {
        return Ok(BoundData::FinderRemovalOwed);
    }
    // A first start onto an UNOWNED, EMPTY database (R3, fix round 2) owes the removal too, as a reset does: the
    // database may have been deleted, or never existed, while the previous account's Finder domain stayed
    // registered. The reconciler's next check settles it (an Observe that finds no domain releases the debt).
    // Not owed when a sign-out whose Finder removal was confirmed left the domain known gone.
    let first_owner_here = cfg!(target_os = "macos")
        && data
            .db
            .owner()
            .map_err(|e| format!("read the local data owner: {e}"))?
            .is_none()
        && !data
            .db
            .has_account_data()
            .map_err(|e| format!("inspect the local data: {e}"))?
        && !data
            .db
            .finder_domain_gone()
            .map_err(|e| format!("read the Finder domain mark: {e}"))?;
    if first_owner_here {
        data.db
            .set_finder_removal_owed(true)
            .map_err(|e| format!("record the Finder removal debt: {e}"))?;
        // Ask for a check so the reconciler looks (it may have no reason to otherwise).
        #[cfg(target_os = "macos")]
        notify_finder(state, finder_setup::core::Trigger::KeysArrived);
    }
    bind_data_to_session(state, &data, identity)?;
    if data
        .db
        .finder_removal_owed()
        .map_err(|e| format!("read the Finder removal debt: {e}"))?
    {
        return Ok(BoundData::FinderRemovalOwed);
    }
    Ok(BoundData::Ready)
}

/// Everything an engine start needs decided before the engine exists, with the caller holding the
/// engine slot (the guard is the proof, and its emptiness is checked). In this order: the keys and the
/// identity of their session, read together under the slot; then the binding of the local data to that
/// identity. `NoSession`: the vault is locked, so there is nothing to start. Any `Err` means: do not start
/// the engine. A binding that failed is returned with `?` and nothing after it runs
/// (`every_engine_start_is_bound_first`).
fn authorize_engine_start(
    state: &AppState,
    acct: &account::AccountRuntime,
    engine_slot: &tokio::sync::MutexGuard<'_, Option<EngineRunner>>,
    paths: &LocalDataPaths,
) -> Result<StartPermit, String> {
    if engine_slot.is_some() {
        return Err(ENGINE_SLOT_OCCUPIED_ERROR.to_string());
    }
    let (keys, identity) = match keys_for_engine_start(acct) {
        Ok(read) => read,
        Err(EngineStartBlocked::NoSession) => return Ok(StartPermit::NoSession),
        Err(refused) => return Err(refused.to_string()),
    };
    // A session that does not know who it is (its sign-in could not fetch the account record) starts nothing and
    // touches nothing: no database opened, no adoption, no purge, no Finder debt. The same message as an owner
    // that cannot be compared: it tells the person to connect and open Beebeeb again.
    if !identity.is_known() {
        return Err(account_binding::IDENTITY_UNKNOWN.to_string());
    }
    let bound = bind_local_data_to_session(state, &identity, paths)?;
    match bound {
        BoundData::Ready => Ok(StartPermit::Go(keys)),
        BoundData::FinderRemovalOwed => Ok(StartPermit::FinderRemovalOwed),
    }
}

/// The start itself, with the engine's constructor passed in: authorize (keys and identity under the slot,
/// then the binding), and only then call `spawn` and put what it returns in the slot. Production passes the
/// one real constructor (see [`spawn_bound_engine`]); a test passes one that looks at the world at the exact
/// moment an engine would start.
fn start_engine_bound(
    state: &AppState,
    acct: &account::AccountRuntime,
    engine_slot: &mut tokio::sync::MutexGuard<'_, Option<EngineRunner>>,
    paths: &LocalDataPaths,
    spawn: impl FnOnce(PathBuf, zeroize::Zeroizing<String>, zeroize::Zeroizing<[u8; 32]>) -> EngineRunner,
) -> Result<EngineStart, String> {
    let turn = acct.session_generation();
    // Ruling F9: the ask is read before the keys are, so it is served only by a start that holds the session of the
    // sign-in that asked, or a later one. A Mac's only: off macOS the engine serves it after its reconcile pass (spec
    // D11, task 1888).
    #[cfg(target_os = "macos")]
    let resume_asked = acct.auth_resume_asked.load(Ordering::SeqCst);
    let (token, key) = match authorize_engine_start(state, acct, &*engine_slot, paths)
        .inspect_err(|error| record_engine_refusal(acct, turn, error))?
    {
        StartPermit::Go(keys) => keys,
        StartPermit::NoSession => return Ok(EngineStart::NoSession),
        StartPermit::FinderRemovalOwed => return Ok(EngineStart::FinderRemovalOwed),
    };
    #[cfg(target_os = "macos")]
    resume_operations_paused_for_auth(acct, paths, resume_asked);
    // Task 2267 (fix round 2): a "Keep on this PC only" a crash interrupted is settled before the engine exists.
    #[cfg(target_os = "windows")]
    settle_an_interrupted_keep(paths);
    **engine_slot = Some(spawn(paths.sync_root.clone(), token, key));
    *acct
        .engine_refusal
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    Ok(EngineStart::Started)
}

/// Lead ruling F9 (spec 2026-10-06 R8: "sync resumes"): a sign-in that has put a session in memory asks for the
/// operations paused for `auth` to be due again, and so does the startup check that confirmed the stored token (F9
/// review I-2). Only inside the writer's checked turn (`_write`) and after the session is in memory. A Mac's engine
/// start serves the ask (see [`resume_operations_paused_for_auth`]); since spec D11 (task 1888) Windows and Linux ask
/// too, and their engine serves it after its reconcile pass and the key check
/// ([`engine_bridge::EngineBridge::reconcile_then_resume`]).
fn ask_to_resume_auth_paused(_write: &SessionWrite, acct: &AccountRuntime) {
    acct.auth_resume_asked.fetch_add(1, Ordering::SeqCst);
}

/// Lead ruling, F9 review I-1 (spec 2026-10-06 R8, §5.6): the vault key this Mac kept is no longer the account's. Every
/// operation paused for `auth` was queued with its names encrypted under that key, so it is re-marked `key_replaced`:
/// kept, never sent, and never made due by any resume (a sign-in's or the startup check's). Only under the engine slot
/// (`_engine_slot`), after the caller took the engine out of it, so no engine pauses another one for `auth` afterwards and
/// no start resumes them in between. No `state.db` yet: nothing is queued. macOS only, like the resume it guards: Windows
/// and Linux keep today's flow (spec R8).
#[cfg(target_os = "macos")]
fn hold_auth_paused_for_the_replaced_key(
    sources: &LocalSources,
    _engine_slot: &tokio::sync::MutexGuard<'_, Option<EngineRunner>>,
) -> Result<usize, String> {
    let held = match sources.state_db()? {
        Some(db) => db
            .hold_operations_paused_for_auth_as_key_replaced(now_unix_seconds())
            .map_err(|e| format!("hold the operations queued under the replaced key: {e}"))?,
        None => 0,
    };
    tracing::info!(
        held,
        "the vault key was replaced: the operations paused for auth are kept and not sent"
    );
    Ok(held)
}

/// F9 review M2, with lead ruling I-1 (spec §5.6): a browser re-sign-in whose handed-over key is not the one this Mac
/// kept. The caller swapped the token while holding the engine slot and still holds it (`engine_slot`), so no start has
/// served the swap's ask. The outgoing engine stops first (its token was revoked, so it pauses every edit it tries for
/// `auth`); only then is every `auth` pause held as `key_replaced` ([`hold_auth_paused_for_the_replaced_key`]). A start
/// after this, with the outgoing key or the handed-over one, finds nothing queued under the outgoing key to resume.
#[cfg(target_os = "macos")]
async fn stop_the_outgoing_engine_and_hold_auth_paused(
    acct: &AccountRuntime,
    sources: &LocalSources,
    engine_slot: &mut tokio::sync::MutexGuard<'_, Option<EngineRunner>>,
) -> Result<usize, String> {
    if let Some(engine) = engine_slot.take()
        && !stop_engine_in_slot(acct, engine).await.is_stopped()
    {
        tracing::error!("the sync engine did not confirm it stopped before the edits under the kept key were held");
    }
    if let Ok(mut engine_state) = acct.engine_state.lock() {
        *engine_state = "stopped".to_string();
    }
    hold_auth_paused_for_the_replaced_key(sources, engine_slot)
}

/// Lead ruling F9 (spec 2026-10-06 R8 and §5.6): when a sign-in asked for it (`asked`, read before this start read its
/// keys) and no start has served that ask yet, every operation paused for `auth` becomes due now, its attempts kept. It
/// runs under the engine slot, after the binding (so another account's operations were already reset) and before the
/// engine exists (so its first tick sends them). Operations paused for any other reason stay paused, those held as
/// `key_replaced` included (F9 review I-1). A database that cannot be written leaves the ask open, so the next engine
/// start tries again (F9 review M3); the engine starts either way. macOS only (spec D11, task 1888): off macOS the
/// engine resumes them after its reconcile pass.
#[cfg(target_os = "macos")]
fn resume_operations_paused_for_auth(acct: &AccountRuntime, paths: &LocalDataPaths, asked: u64) {
    if asked <= acct.auth_resume_done.load(Ordering::SeqCst) {
        return;
    }
    let path = state_paths::state_db_path_from_state_dir(&paths.state_dir);
    match state_db::StateDb::open(&path).and_then(|db| db.resume_operations_paused_for_auth(now_unix_seconds())) {
        Ok(resumed) => {
            acct.auth_resume_done.store(asked, Ordering::SeqCst);
            tracing::info!(resumed, "signed in: the operations paused for auth are due again");
        }
        Err(error) => tracing::warn!(
            %error,
            "signed in, but the operations paused for auth could not be made due; the next engine start tries again"
        ),
    }
}

/// Ruling P (Task 12 fix round 2): a start refused by the binding leaves its closed code on the account, so
/// `sync_status` can show the sentence instead of a stopped engine with no reason. Only the two binding refusals
/// count ([`account_binding::Refusal::of`]); any other error records nothing. Recorded only while the session is still
/// the one this start read (`turn`): a transition since then cleared the field and wins.
fn record_engine_refusal(acct: &account::AccountRuntime, turn: account::SessionGeneration, error: &str) {
    let Some(refusal) = account_binding::Refusal::of(error) else {
        return;
    };
    let mut recorded = acct
        .engine_refusal
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if acct.session_unchanged_since(turn) {
        *recorded = Some(refusal);
    }
}

/// `sync_status.engine_refusal`: `{code, sentence}` for the last binding refusal of the current session, else null.
fn engine_refusal_view(acct: &account::AccountRuntime) -> serde_json::Value {
    match *acct
        .engine_refusal
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
    {
        Some(refusal) => serde_json::json!({ "code": refusal.code(), "sentence": refusal.sentence() }),
        None => serde_json::Value::Null,
    }
}

/// R10 (spec 2026-10-06 §5.6; lead ruling T9-starts): the ONLY place a sync engine starts. The caller
/// holds the engine slot (it passes the guard) and has stopped any earlier engine in it. This reads the
/// session and its keys under that slot, binds the local data to the session's account, and only then
/// spawns, so:
///  - there is no key or token parameter: a Lock that cleared the session before this start got the
///    slot finds nothing to start with, and no engine exists after the Lock returned;
///  - another account's local data is reset first, and any error (a failed reset, an owner that cannot
///    be compared, Windows refusing) means no engine starts.
///
/// The keys reach the engine as `Zeroizing` values. `every_engine_start_is_bound_first` pins that this is
/// the one start in production code and that the binding comes first.
fn spawn_bound_engine(
    app: tauri::AppHandle,
    state: &AppState,
    acct: &account::AccountRuntime,
    engine_slot: &mut tokio::sync::MutexGuard<'_, Option<EngineRunner>>,
    sync_root: PathBuf,
) -> Result<EngineStart, String> {
    let paths = LocalDataPaths::production(&sync_root)?;
    let surface_app = app.clone();
    let started = start_engine_bound(state, acct, engine_slot, &paths, |sync_root, token, key| {
        EngineRunner::spawn(
            app,
            sync_root,
            token,
            key,
            acct.sync_paused.clone(),
            acct.auth_health.clone(),
            runner::ResumeTicket {
                asked: acct.auth_resume_asked.clone(),
                done: acct.auth_resume_done.clone(),
                pass_wanted: acct.reconcile_pass_wanted.clone(),
                pass_served: acct.reconcile_pass_served.clone(),
            },
        )
    });
    // W12: the R10 reset's kept folder is surfaced as the app-start sweep surfaces macOS's: the row, then one alert.
    let reset_kept = state
        .sign_out
        .reset_kept
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take();
    if let Some((folder, files)) = reset_kept {
        surface_reset_folder(&surface_app, &folder, files);
    }
    // Task 2267: a Windows start refused over local data that no owner record matches asks, in the background, whether
    // that data is this account's own, and sync starts once it is proven (`prove_own_data_in_background`).
    // The proof runs for the session this start read: the same account runtime, the turn read under the engine slot.
    #[cfg(target_os = "windows")]
    if matches!(&started, Err(error) if error == account_binding::OTHER_ACCOUNT_ON_WINDOWS)
        && let Ok(active) = state.active_account()
        && std::ptr::eq(Arc::as_ptr(&active), acct)
    {
        prove_own_data_in_background(surface_app, paths.sync_root.clone(), active, acct.session_generation());
    }
    started
}

/// Task 2267: the background half of a Windows start refused with `OTHER_ACCOUNT_ON_WINDOWS`. It proves the local
/// data is the signed-in account's own (`own_data_proof`: its My Files rows decrypt under this session's key), records
/// the session as the owner, and starts sync through the usual start, which binds again and now keeps everything.
/// Another account's data is never adopted and nothing is deleted.
///
/// One proof per session (account runtime and turn) at a time: a start refused again while it runs asks nothing new,
/// a proof for a session that has since changed does not hold the next session's back, and the slot is freed however
/// the proof ends, a panic included. Sync starts only for the session that was proven, checked under the
/// session-transition lock, so a Lock or a sign-out since leaves nothing to start.
#[cfg(target_os = "windows")]
fn prove_own_data_in_background(
    app: tauri::AppHandle,
    sync_root: PathBuf,
    acct: Arc<account::AccountRuntime>,
    turn: account::SessionGeneration,
) {
    type Key = (usize, account::SessionGeneration);
    static IN_FLIGHT: std::sync::Mutex<Option<Key>> = std::sync::Mutex::new(None);
    struct Flight(Key);
    impl Drop for Flight {
        fn drop(&mut self) {
            let mut in_flight = IN_FLIGHT.lock().unwrap_or_else(|p| p.into_inner());
            if *in_flight == Some(self.0) {
                *in_flight = None;
            }
        }
    }
    let key = (Arc::as_ptr(&acct) as usize, turn);
    {
        let mut in_flight = IN_FLIGHT.lock().unwrap_or_else(|p| p.into_inner());
        if *in_flight == Some(key) {
            return;
        }
        *in_flight = Some(key);
    }
    let flight = Flight(key);
    let proof = tauri::async_runtime::spawn(async move {
        let _flight = flight;
        match prove_own_data_of_session(&app, &sync_root, &acct, turn).await {
            Ok(own_data_proof::Outcome::Adopted(tally)) => {
                tracing::info!(?tally, "R10: this PC's local data is proven this account's own; starting sync");
                let transition = SESSION_TRANSITION.lock().await;
                if !session_still_active(&app, &acct, turn) {
                    tracing::info!("R10: the session changed after the proof; nothing to start");
                    return;
                }
                let state = app.state::<AppState>();
                if let Err(error) = start_engine_if_possible(app.clone(), &state, &transition).await {
                    tracing::warn!(%error, "R10: the start after the proof was refused");
                }
            }
            Ok(outcome) => tracing::warn!(?outcome, "R10: this PC's local data was not adopted"),
            Err(error) => tracing::warn!(%error, "R10: the ownership proof could not run"),
        }
    });
    let mut proofs = OWN_DATA_PROOFS.lock().unwrap_or_else(|p| p.into_inner());
    proofs.retain(|running| !running.inner().is_finished());
    proofs.push(proof);
}

/// Task 2267 (review P2-2): the ownership proofs that may still be running. Each holds the session's key and token
/// (its client is registered with `windows_cf::track_credentials`), so a Lock and a sign-out stop them
/// (`stop_own_data_proofs`) before they wait for every credential to be released.
#[cfg(target_os = "windows")]
static OWN_DATA_PROOFS: std::sync::Mutex<Vec<tauri::async_runtime::JoinHandle<()>>> = std::sync::Mutex::new(Vec::new());

/// Stop every running ownership proof. Both callers hold the session-transition lock, which a proof takes only to
/// start sync after an adoption, so a proof is never stopped half way through a start. A proof stopped after it
/// recorded the owner has recorded this same account's own data; one stopped before records nothing.
#[cfg(target_os = "windows")]
fn stop_own_data_proofs() {
    let proofs = std::mem::take(&mut *OWN_DATA_PROOFS.lock().unwrap_or_else(|p| p.into_inner()));
    for proof in proofs {
        proof.abort();
    }
}

/// Whether `acct` is still the active account and its session is still the one read at `turn`.
#[cfg(target_os = "windows")]
fn session_still_active(app: &tauri::AppHandle, acct: &Arc<account::AccountRuntime>, turn: account::SessionGeneration) -> bool {
    acct.session_unchanged_since(turn)
        && app
            .state::<AppState>()
            .active_account()
            .is_ok_and(|active| Arc::ptr_eq(&active, acct))
}

/// How long the proof waits for the whole listing of this account's tree (the snapshot, then the trash, with the
/// client's retries). A Lock or a sign-out does not wait for it: they stop the proof first.
#[cfg(target_os = "windows")]
const LIST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// Self-review F5: how long one request of the proof's listing may take. The snapshot is the whole tree in one response,
/// so the app's 30 s per request would refuse a large tree on a slow link every time, and say "connect to the internet"
/// to someone who is online. Nobody waits on the proof (it runs in the background, and a Lock or a sign-out stops it).
#[cfg(target_os = "windows")]
const PROOF_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// The proof for the session read at `turn`, with its keys and identity read together as an engine start reads them.
/// A session that changes stops the asking and records nothing (`session_still_active`). With no answer at all the
/// refusal becomes the one that says to connect and open Beebeeb again; a token the server refused is not "offline",
/// so that sentence is not shown for it. The next start asks again.
#[cfg(target_os = "windows")]
async fn prove_own_data_of_session(
    app: &tauri::AppHandle,
    sync_root: &Path,
    acct: &Arc<account::AccountRuntime>,
    turn: account::SessionGeneration,
) -> Result<own_data_proof::Outcome, String> {
    if !session_still_active(app, acct, turn) {
        return Ok(own_data_proof::Outcome::Changed(own_data_proof::Tally::default()));
    }
    let Ok(((token, key), identity)) = keys_for_engine_start(acct) else {
        return Ok(own_data_proof::Outcome::NotEligible);
    };
    let paths = LocalDataPaths::production(sync_root)?;
    let Some(db) = state_db_from_state_dir(&paths.state_dir)? else {
        return Ok(own_data_proof::Outcome::NotEligible);
    };
    let api = Arc::new(
        api_client::ApiClient::new(runner::api_base_url(), String::clone(&token), *key)
            .with_request_timeout(PROOF_REQUEST_TIMEOUT),
    );
    windows_cf::track_credentials(&api);
    let outcome = own_data_proof::prove_and_adopt(
        &db,
        &identity,
        &key,
        || {
            let api = Arc::clone(&api);
            async move {
                let listed = tokio::time::timeout(LIST_TIMEOUT, async {
                    let snapshot = api.sync_snapshot().await;
                    let trashed = match &snapshot {
                        Ok(_) => api.list_trashed_files_all().await,
                        Err(_) => Ok(Vec::new()),
                    };
                    own_data_proof::listing_of(snapshot, trashed)
                })
                .await;
                listed.unwrap_or(own_data_proof::Listing::Unavailable)
            }
        },
        || session_still_active(app, acct, turn),
    )
    .await?;
    drop(api);
    if matches!(outcome, own_data_proof::Outcome::Unavailable(tally) if tally.signed_out == 0) {
        record_engine_refusal(acct, turn, account_binding::IDENTITY_UNKNOWN);
    }
    Ok(outcome)
}

/// Task 2267 (fix round 2): what Upload and Keep say when this PC's local data is not the signed-in account's own.
#[cfg(any(target_os = "windows", test))]
const FOUND_WHILE_PAUSED_NOT_YOURS: &str =
    "Beebeeb couldn’t confirm these files belong to the account you’re signed in to, so nothing was changed.";

/// Task 2267 (fix round 2): what Keep says when it stopped part way (the database could not be written). Every item it
/// moved is forgotten in its own transaction, or settled at the next start (`settle_an_interrupted_keep`).
#[cfg(target_os = "windows")]
const FOUND_WHILE_PAUSED_KEEP_STOPPED: &str = "Beebeeb couldn’t finish moving these files. Anything already moved is in “Beebeeb - not uploaded”; the rest stays where it is and isn’t uploaded.";

/// Task 2267 (lead review round 3, P2-2): what the card says when it cannot read the held changes. It reads them again
/// at the next refresh; Upload waits for that read, since it sends only what the card showed.
#[cfg(target_os = "windows")]
const FOUND_WHILE_PAUSED_REVIEW_FAILED: &str =
    "Beebeeb couldn’t read these changes, so nothing was uploaded or moved. It tries again in a moment.";

/// Task 2267 (fix round 2): what Upload says when the held changes could not be released: nothing changed.
#[cfg(target_os = "windows")]
const FOUND_WHILE_PAUSED_UPLOAD_FAILED: &str =
    "Beebeeb couldn’t start uploading these files, so they’re still waiting. Try again.";

/// Task 2267 (fix round 2): what "Show in Explorer" says when the folder the last keep recorded is not there.
#[cfg(any(target_os = "windows", test))]
const KEPT_FOLDER_GONE: &str = "Beebeeb can’t find the folder it moved them to. It may have been moved or renamed.";

/// Task 2267 (fix round 2): who is signed in, for a decision about the changes found while sync was paused: the
/// session's own email and, when the cached profile is that account's, its user id ([`identity_of_session`]). No key is
/// read. No session, or one that does not know who it is, decides nothing.
#[cfg(any(target_os = "windows", test))]
fn decision_identity(state: &AppState) -> Result<account_binding::Identity, String> {
    let not_yours = || FOUND_WHILE_PAUSED_NOT_YOURS.to_string();
    let acct = state.active_account().map_err(|_| not_yours())?;
    let email = acct
        .session
        .lock()
        .map_err(|_| not_yours())?
        .as_ref()
        .map(|session| session.email.clone())
        .ok_or_else(not_yours)?;
    let profile = acct.cached_profile.lock().ok().and_then(|guard| guard.clone());
    Some(identity_of_session(email.as_deref(), profile.as_ref()))
        .filter(|identity| identity.is_known())
        .ok_or_else(not_yours)
}

/// Task 2267 (fix round 2): this PC's `state.db` under `state_dir`, only when its recorded owner is `identity`'s
/// account. Another account's data, data nobody owns yet and no database decide nothing.
#[cfg(any(target_os = "windows", test))]
fn own_state_db(identity: &account_binding::Identity, state_dir: &Path) -> Result<state_db::StateDb, String> {
    let db = state_db_from_state_dir(state_dir)?.ok_or_else(|| FOUND_WHILE_PAUSED_NOT_YOURS.to_string())?;
    let owner = db.owner().map_err(|e| format!("read the local data owner: {e}"))?;
    match owner {
        Some(owner) if account_binding::same_account(&owner, identity) == Some(true) => Ok(db),
        _ => Err(FOUND_WHILE_PAUSED_NOT_YOURS.to_string()),
    }
}

/// Task 2267 (fix round 2): the signed-in account's own local data and the Beebeeb folder, for Upload and Keep.
#[cfg(target_os = "windows")]
fn own_local_data_for_decision(identity: &account_binding::Identity) -> Result<(state_db::StateDb, PathBuf), String> {
    let sync_root = DesktopConfig::load()?
        .sync_root
        .ok_or_else(|| FOUND_WHILE_PAUSED_NOT_YOURS.to_string())?;
    Ok((own_state_db(identity, &state_paths::beebeeb_state_dir()?)?, sync_root))
}

/// Task 2267 (fix round 2): the folder "Show in Explorer" opens: the dated folder the last keep recorded, only when it
/// is a `YYYY-MM-DD` folder directly inside `Beebeeb - not uploaded` next to the Beebeeb folder, and is there.
#[cfg(any(target_os = "windows", test))]
fn kept_folder_to_show(sync_root: &Path, recorded: rusqlite::Result<Option<String>>) -> Result<PathBuf, String> {
    let folder = PathBuf::from(
        recorded
            .map_err(|e| format!("read the kept folder: {e}"))?
            .ok_or(KEPT_FOLDER_GONE)?,
    );
    let kept = sync_root
        .parent()
        .map(|parent| parent.join(found_while_paused::KEPT_FOLDER_NAME));
    let dated = folder.file_name().and_then(|name| name.to_str()).is_some_and(|name| {
        name.len() == 10
            && name
                .bytes()
                .enumerate()
                .all(|(at, byte)| if at == 4 || at == 7 { byte == b'-' } else { byte.is_ascii_digit() })
    });
    if dated && kept.is_some_and(|kept| folder.parent() == Some(kept.as_path())) && folder.is_dir() {
        Ok(folder)
    } else {
        Err(KEPT_FOLDER_GONE.to_string())
    }
}

/// Task 2267 (fix round 2, lead ruling (a) 4): a "Keep on this PC only" that a crash interrupted is settled before the
/// engine exists: finished when the item had moved, undone when it had not (`found_while_paused::recover`). The engine
/// start calls it after the binding, so it acts on this account's own data only. A database that cannot be read or
/// written leaves the journal for the next start; the engine starts either way, and the item's changes stay held.
#[cfg(target_os = "windows")]
fn settle_an_interrupted_keep(paths: &LocalDataPaths) {
    let path = state_paths::state_db_path_from_state_dir(&paths.state_dir);
    let settled = state_db::StateDb::open(&path)
        .map_err(|e| format!("open state.db: {e}"))
        .and_then(|db| found_while_paused::recover(&db, &paths.sync_root));
    match settled {
        Ok(found_while_paused::Recovery::Nothing) => {}
        Ok(recovery) => tracing::info!(?recovery, "keep: a move a crash interrupted was settled before sync started"),
        Err(error) => tracing::warn!(%error, "keep: a move a crash interrupted is settled at the next start"),
    }
}

/// Task 2267 (fix round 2), Settings › Sync: how many changes "Upload to my Beebeeb" released.
#[cfg(target_os = "windows")]
#[derive(Debug, Clone, Copy, serde::Serialize)]
struct FoundWhilePausedReleased {
    released: usize,
}

/// Task 2267 (lead review round 3, P2-2), Settings › Sync: the held changes by kind, the ids "Upload to my Beebeeb"
/// sends back, and which service (if any) syncs the folder a keep moves them next to. Only on the signed-in account's
/// own data, under the session-transition lock. Counts and ids only, never a name.
#[cfg(target_os = "windows")]
#[tauri::command]
async fn found_while_paused_review(state: State<'_, AppState>) -> Result<found_while_paused::Review, String> {
    let _transition = SESSION_TRANSITION.lock().await;
    let identity = decision_identity(&state)?;
    tokio::task::spawn_blocking(move || -> Result<found_while_paused::Review, String> {
        let (db, sync_root) = own_local_data_for_decision(&identity)?;
        let mut review = found_while_paused::review(&db.found_while_paused_changes().map_err(|error| {
            tracing::warn!(%error, "found while paused: the held changes could not be read");
            FOUND_WHILE_PAUSED_REVIEW_FAILED.to_string()
        })?);
        review.synced_by = kept_folder_synced_by(&sync_root);
        Ok(review)
    })
    .await
    .map_err(|e| format!("review task failed: {e}"))?
}

/// Task 2267 (lead review round 3, P3-4): the service that syncs the folder the kept folder is made in (next to the
/// Beebeeb folder), if OneDrive or Dropbox does.
#[cfg(target_os = "windows")]
fn kept_folder_synced_by(sync_root: &Path) -> Option<String> {
    let parent = sync_root.parent()?;
    found_while_paused::synced_by(parent, &found_while_paused::cloud_synced_roots()).map(str::to_string)
}

/// Task 2267 (fix round 2), Settings › Sync, "Upload to my Beebeeb": the changes the card showed (`op_ids`, from
/// [`found_while_paused_review`]; lead review round 3, P3-1) that are still held are due again, so the running engine
/// uploads them. A change held after the card read them stays held. Only on the signed-in account's own data, and
/// under the session-transition lock, so no sign-out or Lock runs in between.
#[cfg(target_os = "windows")]
#[tauri::command]
async fn found_while_paused_upload(state: State<'_, AppState>, op_ids: Vec<String>) -> Result<FoundWhilePausedReleased, String> {
    let _transition = SESSION_TRANSITION.lock().await;
    let identity = decision_identity(&state)?;
    // Task 1888 (Task 1's ruling on 2267's held changes; review P2): a released change may be a ghost folder's create
    // the reconcile pass left to the person; sent before a pass sees it, it would make a folder the server already has
    // a second time. So the release asks for a pass, before it commits and again after: the engine's queue sends
    // nothing while an ask is unserved, and its next successful tick runs the pass (no resume: a release is not a
    // sign-in). Without the account there is no ask, so nothing is released.
    let pass_wanted = state
        .active_account()
        .map_err(|_| FOUND_WHILE_PAUSED_UPLOAD_FAILED.to_string())?
        .reconcile_pass_wanted
        .clone();
    let released = tokio::task::spawn_blocking(move || -> Result<usize, String> {
        let (db, _) = own_local_data_for_decision(&identity)?;
        pass_wanted.fetch_add(1, Ordering::SeqCst);
        let released = db.release_found_while_paused(&op_ids, now_unix_seconds()).map_err(|error| {
            tracing::warn!(%error, "found while paused: the held changes could not be released");
            FOUND_WHILE_PAUSED_UPLOAD_FAILED.to_string()
        })?;
        pass_wanted.fetch_add(1, Ordering::SeqCst);
        Ok(released)
    })
    .await
    .map_err(|e| format!("upload task failed: {e}"))??;
    tracing::info!(released, "found while paused: released to upload");
    Ok(FoundWhilePausedReleased { released })
}

/// Task 2267 (fix round 2), Settings › Sync, "Keep on this PC only" (lead ruling (a)): every item made on this PC that
/// a held change the card showed (`op_ids`, self-review of round 3, F7) acts on moves next to the Beebeeb folder and is
/// forgotten (`found_while_paused::keep`). Only on the
/// signed-in account's own data, and under the session-transition lock. Counts only in the log.
#[cfg(target_os = "windows")]
#[tauri::command]
async fn found_while_paused_keep(
    state: State<'_, AppState>,
    op_ids: Vec<String>,
) -> Result<found_while_paused::KeepResult, String> {
    let _transition = SESSION_TRANSITION.lock().await;
    let identity = decision_identity(&state)?;
    let kept = tokio::task::spawn_blocking(move || -> Result<found_while_paused::KeepResult, String> {
        let (db, sync_root) = own_local_data_for_decision(&identity)?;
        let today = found_while_paused::today();
        let mut kept = found_while_paused::keep(&db, &sync_root, &today, &found_while_paused::rename_no_replace, &op_ids)
            .map_err(|error| {
                tracing::warn!(%error, "found while paused: the keep stopped");
                FOUND_WHILE_PAUSED_KEEP_STOPPED.to_string()
            })?;
        kept.synced_by = kept_folder_synced_by(&sync_root);
        Ok(kept)
    })
    .await
    .map_err(|e| format!("keep task failed: {e}"))??;
    tracing::info!(
        moved_items = kept.moved_items,
        moved_files = kept.moved_files,
        not_moved = kept.not_moved,
        stays_held = kept.stays_held,
        "found while paused: kept on this PC"
    );
    Ok(kept)
}

/// Task 2267 (fix round 2), "Show in Explorer" after a keep: opens the folder the last keep recorded
/// ([`kept_folder_to_show`]). The frontend never passes a path.
#[cfg(target_os = "windows")]
#[tauri::command]
async fn show_kept_folder() -> Result<(), String> {
    tokio::task::spawn_blocking(|| -> Result<(), String> {
        let cfg = DesktopConfig::load()?;
        let sync_root = cfg.sync_root.clone().ok_or(KEPT_FOLDER_GONE)?;
        let db = state_db_for_config(&cfg)?.ok_or(KEPT_FOLDER_GONE)?;
        let folder = kept_folder_to_show(&sync_root, db.last_kept_dir())?;
        std::process::Command::new("explorer")
            .arg(&folder)
            .spawn()
            .map_err(|e| format!("open Explorer: {e}"))?;
        Ok(())
    })
    .await
    .map_err(|e| format!("show task failed: {e}"))?
}

/// Task 2267: the changes found while sync was paused exist on Windows only (the ownership proof holds them). The
/// commands are registered everywhere, because `generate_handler!` takes no `cfg`, and say so elsewhere.
#[cfg(not(target_os = "windows"))]
#[tauri::command]
async fn found_while_paused_review() -> Result<(), String> {
    Err("This works on Windows only.".into())
}

#[cfg(not(target_os = "windows"))]
#[tauri::command]
async fn found_while_paused_upload() -> Result<(), String> {
    Err("This works on Windows only.".into())
}

#[cfg(not(target_os = "windows"))]
#[tauri::command]
async fn found_while_paused_keep() -> Result<(), String> {
    Err("This works on Windows only.".into())
}

#[cfg(not(target_os = "windows"))]
#[tauri::command]
async fn show_kept_folder() -> Result<(), String> {
    Err("This works on Windows only.".into())
}

/// W12 and §9: the folder an R10 reset kept reaches the person as the app-start sweep's does. The reset recorded it
/// already (W9), so the windows are told (an open Settings › Sync reads its row again), then one alert names it. The
/// path is never logged.
fn surface_reset_folder(app: &tauri::AppHandle, folder: &str, files: usize) {
    emit_kept_folder_changed(app);
    if let Some((title, message)) = sign_out::copy::sign_out_alert(Some((folder, files)), None) {
        app.dialog()
            .message(message)
            .title(title)
            .kind(tauri_plugin_dialog::MessageDialogKind::Info)
            .show(|_| {});
    }
}

/// Is a Finder domain removal owed before any engine may start? Read by the reconciler's facts. An unreadable
/// database is not a debt it could act on (the engine start fails to open it too).
#[cfg(target_os = "macos")]
pub(crate) fn finder_removal_owed() -> bool {
    state_paths::beebeeb_state_dir()
        .ok()
        .is_some_and(|dir| finder_removal_owed_in(&dir))
}

#[cfg(target_os = "macos")]
fn finder_removal_owed_in(state_dir: &Path) -> bool {
    matches!(state_db_from_state_dir(state_dir), Ok(Some(db)) if db.finder_removal_owed().unwrap_or(false))
}

/// The reconciler confirmed the domain gone (a removal worked, or an Observe found none): release the debt, which
/// lets the next engine start through.
#[cfg(target_os = "macos")]
pub(crate) fn clear_finder_removal_owed() {
    if let Ok(dir) = state_paths::beebeeb_state_dir() {
        clear_finder_removal_owed_in(&dir);
    }
}

/// The reconciler registered the domain (`addDomain` succeeded): it is no longer known gone, so a later start onto
/// an unowned, empty database owes a check again instead of trusting the mark a confirmed sign-out left.
#[cfg(target_os = "macos")]
pub(crate) fn clear_finder_domain_gone() {
    if let Ok(dir) = state_paths::beebeeb_state_dir() {
        clear_finder_domain_gone_in(&dir);
    }
}

#[cfg(target_os = "macos")]
fn clear_finder_domain_gone_in(state_dir: &Path) {
    if let Ok(Some(db)) = state_db_from_state_dir(state_dir)
        && let Err(error) = db.clear_finder_domain_gone()
    {
        tracing::warn!(%error, "could not clear the Finder domain mark; a later first start may skip its removal check");
    }
}

#[cfg(target_os = "macos")]
fn clear_finder_removal_owed_in(state_dir: &Path) {
    if let Ok(Some(db)) = state_db_from_state_dir(state_dir)
        && let Err(error) = db.set_finder_removal_owed(false)
    {
        tracing::warn!(%error, "could not release the Finder removal debt; engine starts keep waiting for a removal");
    }
}

/// R10, the upgrade path (fix round 1 of Task 10): local data from before the binding has no owner. On the first
/// startup after the upgrade, and only then, the account whose VAULT KEY the Keychain holds (`candidate`; `None`
/// when it holds a token without a key, or nothing) becomes the owner. Any other unowned data is an unknown
/// owner and is reset (or refused on Windows) at the next engine start. The window shuts whatever this finds.
/// Best-effort: an error here leaves the data unowned, which fails closed.
fn adopt_unbound_local_data_at_startup(candidate: Option<&account_binding::Identity>) {
    let Ok(state_dir) = state_paths::beebeeb_state_dir() else {
        return;
    };
    adopt_unbound_local_data_in(&state_dir, candidate);
}

/// [`adopt_unbound_local_data_at_startup`] for a given state dir (a test points it at a throwaway one).
fn adopt_unbound_local_data_in(state_dir: &Path, candidate: Option<&account_binding::Identity>) {
    let Ok(Some(db)) = state_db_from_state_dir(state_dir) else {
        return;
    };
    let data = StateDbLocalData {
        db,
        sync_root: None,
        staging_dirs: Vec::new(),
        kept: Default::default(),
    };
    match account_binding::adopt_unbound(&data, candidate) {
        Ok(true) => tracing::info!("R10: existing local data recorded as this computer's signed-in account's"),
        Ok(false) => {}
        Err(error) => tracing::warn!(%error, "R10: could not record the owner of existing local data"),
    }
}

/// The account to adopt unowned local data into at startup: only a session the Keychain gave back COMPLETE
/// (token and wrapped vault key), and only by its email. A Keychain with a token but no key (a sign-in that
/// stopped before the recovery-phrase step, or any failure to read) names nobody.
fn startup_adoption_candidate(loaded: &Result<Option<Session>, String>) -> Option<account_binding::Identity> {
    let session = loaded.as_ref().ok()?.as_ref()?;
    let candidate = account_binding::Identity::new(None, session.email.as_deref());
    candidate.is_known().then_some(candidate)
}

/// Sign-out (§5.3 (5)) and Repair (§5.3 (6)) on macOS: the reconciler cancels any check and removes
/// the domain; this waits for that removal (bounded), so the caller's next step cannot race a check.
/// `Err` when the reconciler is not running, did not answer in time, or the removal failed: the text
/// is the OS error's domain and code at most (lead ruling T1-4), and callers that report it use a fixed
/// sentence. There is no direct removal fallback: every bridge call goes through the gate (ruling 4).
/// Task 1882: the removal keeps the files that never reached the server, and both answers carry what
/// macOS kept (`kept_location` names the folder), so the caller can show it.
#[cfg(target_os = "macos")]
async fn finder_remove_for(
    state: &AppState,
    trigger: finder_setup::core::Trigger,
) -> Result<finder_removal::DomainRemoval, finder_removal::RemovalFailure> {
    match state.finder_setup.get() {
        Some(handle) => handle.remove(trigger, finder_wait(FINDER_REMOVE_TIMEOUT)).await,
        None => Err(finder_setup::driver::NOT_RUNNING.to_string().into()),
    }
}

/// Lock (§5.3 (3)): the reconciler cancels any check and holds, and this waits for that
/// acknowledgement (bounded), so it can stop the engine without a check starting another.
#[cfg(target_os = "macos")]
async fn finder_lock_for(state: &AppState) -> Result<(), String> {
    match state.finder_setup.get() {
        Some(handle) => handle.lock(finder_wait(FINDER_LOCK_TIMEOUT)).await,
        None => Err(finder_setup::driver::NOT_RUNNING.to_string()),
    }
}

/// Lead ruling T3-⚠2: a sign-out has told the reconciler to remove Finder. If it then stops short
/// (any return before the keys are gone) and the keys are still in memory, the person is still
/// signed in, so the reconciler is told keys are here and re-adds Finder. If the keys are gone the
/// reconciler stays held. Runs on every way out of `clear_session_impl`, including `?`.
#[cfg(target_os = "macos")]
struct FinderRestoreOnAbort<'a> {
    state: &'a AppState,
    acct: &'a account::AccountRuntime,
}

#[cfg(target_os = "macos")]
impl Drop for FinderRestoreOnAbort<'_> {
    fn drop(&mut self) {
        let keys_in_memory = self.acct.session.lock().map(|guard| guard.is_some()).unwrap_or(false);
        // Not while an engine stop is unconfirmed (Task 9 fix round 1, item E): every start refuses until a
        // restart, so the check this would run could only fail, and the restart's `Launch` re-adds Finder.
        let stop_unconfirmed = self.acct.engine_stop_unconfirmed.load(Ordering::SeqCst);
        if keys_in_memory && !stop_unconfirmed {
            notify_finder(self.state, finder_setup::core::Trigger::KeysArrived);
        }
    }
}

fn macos_version_string() -> String {
    sysinfo::System::os_version().unwrap_or_else(|| "unknown".to_string())
}

/// The names the lifecycle log scrubs from an NSError message: the same state.db source as the
/// support bundle (task 1685). Empty when there is no state.db yet; the redaction's allow-list
/// still removes every unknown word. It opens its own `StateDb`, takes no app lock, never
/// panics, and never calls `lifecycle_log::event` or `tail` (lead ruling T5-a): the log calls it
/// with no lock of its own held, but the caller of `event` may hold others.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn lifecycle_known_names() -> diagnostic_redaction::KnownNames {
    let sync_root = DesktopConfig::load()
        .ok()
        .and_then(|cfg| cfg.sync_root)
        .map(|p| p.to_string_lossy().into_owned());
    match state_db_from_app_local_state_dir() {
        Ok(Some(db)) => db
            .known_names(&sync_root.into_iter().collect::<Vec<_>>())
            .unwrap_or_default(),
        _ => diagnostic_redaction::KnownNames::new(),
    }
}

/// Spec §5.5 step 5 (macOS): on `Ready`, save the sync folder and make sure the engine runs.
/// Unlike `persist_sync_root_and_start_engine` it never restarts an engine that is already
/// running: the reconciler started it before `addDomain` (plan "Spec issues" 7), or the
/// relaunch's restore did.
#[cfg(target_os = "macos")]
async fn ensure_sync_root_and_engine(
    app: tauri::AppHandle,
    state: &State<'_, AppState>,
    root: PathBuf,
) -> Result<(), String> {
    // Task 1889: one load-change-save under the config-write lock; the pause state it read is the fallback for the
    // rehydration below, which stores it under the same lock.
    let pause_sync = DesktopConfig::update(|cfg| {
        let changed = cfg.sync_root.as_deref() != Some(root.as_path());
        if changed {
            cfg.sync_root = Some(root.clone());
        }
        (changed, cfg.pause_sync)
    })?;
    let acct = state.active_account()?;
    // The slot first, then the keys under it (lead rulings T8-⚠lock 2 and 3): Lock and Sign-out clear
    // the session before they release the slot, so a check that waited for it finds no keys, and
    // nothing starts beside an engine whose stop was never confirmed.
    let mut engine_slot = acct.engine.lock().await;
    engine_start_refusal(&acct)?;
    if engine_slot.is_none() {
        rehydrate_sync_paused(&acct.sync_paused, pause_sync);
        // The keys are read under the slot, and the local data is bound to the account, inside the start.
        match spawn_bound_engine(app, state, &acct, &mut engine_slot, root)? {
            EngineStart::Started => {}
            EngineStart::NoSession => return Err(ENGINE_START_NO_SESSION_FOR_FINDER.to_string()),
            EngineStart::FinderRemovalOwed => return Err(FINDER_REMOVAL_OWED_ERROR.to_string()),
        }
    }
    Ok(())
}

/// The one wait a time limit may cut off in a check's engine start or stop: the wait for the engine
/// slot (lead ruling T7-1; Task 8 fix round 1). A cut-off here changes nothing: the engine is not yet
/// spawned or taken, so it stays exactly where sign-out and lock look for it. A limit around the work
/// that follows would orphan the engine (see `stop_check_engine`).
#[cfg(target_os = "macos")]
async fn lock_engine_slot(
    acct: &account::AccountRuntime,
    op: finder_setup::core::Op,
    limit: std::time::Duration,
) -> Result<tokio::sync::MutexGuard<'_, Option<EngineRunner>>, finder_setup::error::FpError> {
    finder_setup::driver::within(op, limit, async { Ok(acct.engine.lock().await) }).await
}

/// Spec §5.5 step 2 (macOS): make sure the engine runs before `addDomain`, because the extension talks
/// to the engine's socket. `Ok(true)`: this call started it, so the check must stop it if it fails;
/// `Ok(false)`: one was already running.
///
/// Only the wait for the engine slot is under a time limit. Once the engine is spawned nothing may
/// drop this future: the `started` answer would be lost and a failed check would never stop the
/// engine it started. The socket wait after it has its own 3 s bound.
#[cfg(target_os = "macos")]
async fn start_check_engine(
    app: tauri::AppHandle,
    state: &AppState,
    acct: &account::AccountRuntime,
    root: PathBuf,
    slot_limit: std::time::Duration,
) -> Result<bool, finder_setup::error::FpError> {
    use finder_setup::error::{FpError, app_code};
    let (started, ipc_bind_error) = {
        // The slot first, then the keys under it (lead rulings T8-⚠lock 2 and 3): Lock and Sign-out clear
        // the session before they release the slot, so a check that waited for it finds no keys, and
        // nothing starts beside an engine whose stop was never confirmed. The keys are read, and the local
        // data bound to the account, inside `spawn_bound_engine`.
        let mut engine_slot = lock_engine_slot(acct, finder_setup::core::Op::StartEngine, slot_limit).await?;
        engine_start_refusal(acct).map_err(|error| FpError::app(app_code::ENGINE_START, error))?;
        // Rehydrate the persisted pause state before spawning. Best-effort: an unreadable config means not paused.
        rehydrate_sync_paused(&acct.sync_paused, false);
        if let Some(existing) = engine_slot.as_ref() {
            // Already running (a retry, or the restore started it): reuse its bind-status handle so a
            // REAL bind error is still seen instead of a silent timeout.
            (false, existing.ipc_bind_error_handle())
        } else {
            match spawn_bound_engine(app, state, acct, &mut engine_slot, root)
                .map_err(|error| FpError::app(app_code::ENGINE_START, error))?
            {
                EngineStart::Started => {}
                EngineStart::NoSession => {
                    return Err(FpError::app(app_code::ENGINE_START, ENGINE_START_NO_SESSION_FOR_FINDER));
                }
                EngineStart::FinderRemovalOwed => {
                    return Err(FpError::app(app_code::ENGINE_START, FINDER_REMOVAL_OWED_ERROR));
                }
            }
            let bind_error = engine_slot
                .as_ref()
                .map(|runner| runner.ipc_bind_error_handle())
                .ok_or_else(|| {
                    FpError::app(
                        app_code::ENGINE_START,
                        "the sync engine is not in its slot after it started",
                    )
                })?;
            (true, bind_error)
        }
    };

    if let Err(error) = wait_for_file_provider_ipc_ready(ipc_bind_error).await {
        if started && let Err(stop) = stop_check_engine(acct, slot_limit).await {
            tracing::error!(%stop, "finder setup: could not stop the engine whose socket never came up");
        }
        return Err(FpError::app(app_code::ENGINE_START, error));
    }
    Ok(started)
}

/// Spec §5.5 (macOS): stop the engine a failed check started. `Err` when the slot could not be taken in
/// time (the engine was not touched and is still tracked in its slot) or when `abort()` could not
/// confirm the engine task ended; the core never claims a stop that did not happen.
///
/// Only the wait for the engine slot is under a time limit. Once the engine is out of its slot this
/// future must run to the end: `abort()` gives up on its own after 5 s, but a future dropped inside it
/// detaches the engine task, which would keep running untracked, still holding the session token and
/// master key, while sign-out finds an empty slot and reports done.
#[cfg(target_os = "macos")]
async fn stop_check_engine(
    acct: &account::AccountRuntime,
    slot_limit: std::time::Duration,
) -> Result<(), finder_setup::error::FpError> {
    use finder_setup::error::{FpError, app_code};
    let mut engine_slot = lock_engine_slot(acct, finder_setup::core::Op::StopEngine, slot_limit).await?;
    let Some(engine) = engine_slot.take() else {
        return Ok(());
    };
    if stop_engine_in_slot(acct, engine).await.task_confirmed() {
        return Ok(());
    }
    // The consumed handle leaves an empty slot, so the Bug-A2 flag is what keeps sign-out refused (and
    // says to restart) while the task may still be running; `stop_engine_in_slot` set it before the slot
    // is released.
    tracing::error!("finder setup: could not confirm the sync engine stopped; sign-out stays refused until a restart");
    Err(FpError::app(
        app_code::ENGINE_STOP_UNCONFIRMED,
        "the sync engine did not confirm it stopped",
    ))
}

const FINDER_SETUP_MACOS_ONLY: &str = "Finder setup is only available on macOS.";

fn finder_setup_state_impl(state: &AppState) -> Result<finder_setup::driver::FinderSetupView, String> {
    state
        .finder_setup
        .get()
        .map(|handle| handle.view())
        .ok_or_else(|| FINDER_SETUP_MACOS_ONLY.to_string())
}

/// What "Try again" says while the reconciler is held (after a sign-out or a Lock, until keys arrive): no check can add
/// Beebeeb to Finder before then. The quiet `Missing` row's own sentence (spec §5.2).
const FINDER_SETUP_HELD: &str = "Beebeeb adds itself to Finder when you’re signed in and the vault is unlocked.";

/// "Try again" (§5.3 (4)). `Err` when there is no reconciler or it has stopped, which the frontend
/// shows as its existing "couldn't retry" toast (lead ruling 3), and while the reconciler is held, when the retry
/// could do nothing (area A M4): the sentence says what will add Beebeeb instead of a silent no-op.
fn finder_setup_retry_impl(state: &AppState) -> Result<(), String> {
    let handle = state
        .finder_setup
        .get()
        .ok_or_else(|| FINDER_SETUP_MACOS_ONLY.to_string())?;
    if handle.is_held() {
        return Err(FINDER_SETUP_HELD.to_string());
    }
    handle.trigger(finder_setup::core::Trigger::TryAgain)
}

/// "Try again", against the server at `base_url` (lead ruling 5, Task 12): a session that has no email (so its engine
/// start cannot tell whose it is) asks the server first, so the check this asks for can start the engine the session
/// was refused. Then the reconciler's retry, whatever the answer was. A session with an email is already enough for the
/// engine start (`Identity::is_known`), so its retry never waits on the network (area A M8): that is exactly when the
/// network is slow or absent. An explicit action: not debounced, but never a second request while one is in flight
/// (fix round 1, item 4).
#[cfg(not(target_os = "windows"))]
async fn finder_setup_try_again(state: &AppState, base_url: &str) -> Result<(), String> {
    if session_in_memory_has_no_email(state) {
        identify_unidentified_session(state, base_url, std::time::Instant::now(), IdentifyPace::Now).await;
    }
    finder_setup_retry_impl(state)
}

/// A session is in memory and has no email (its sign-in could not fetch the account record). No session, or a lock that
/// cannot be read: `false` (there is nothing to name).
#[cfg(not(target_os = "windows"))]
fn session_in_memory_has_no_email(state: &AppState) -> bool {
    let Ok(acct) = state.active_account() else {
        return false;
    };
    acct.session
        .lock()
        .map(|guard| {
            guard
                .as_ref()
                .is_some_and(|session| session.email.as_deref().is_none_or(|email| email.trim().is_empty()))
        })
        .unwrap_or(false)
}

/// App activation, against the server at `base_url`, at `now` (lead ruling 5, Task 12; fix round 1, item 4): a session
/// that does not know its account's user id asks the server again, at most once a minute and never while a request is in
/// flight (every window focus lands here). `true` when it is named now, so the caller starts the engine it was refused.
#[cfg(not(target_os = "windows"))]
async fn identify_on_app_activation(state: &AppState, base_url: &str, now: std::time::Instant) -> bool {
    identify_unidentified_session(state, base_url, now, IdentifyPace::Debounced).await
}

/// How often app activation may ask the server who an unidentified session is.
#[cfg(not(target_os = "windows"))]
const IDENTIFY_ON_ACTIVATION_EVERY: std::time::Duration = std::time::Duration::from_secs(60);

/// Whether an identify request waits for [`IDENTIFY_ON_ACTIVATION_EVERY`] since the last one.
#[cfg(not(target_os = "windows"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IdentifyPace {
    /// App activation.
    Debounced,
    /// "Try again" and the startup's second request: asked for once, so not debounced.
    Now,
}

/// One identify request in flight for an account (fix round 1, item 4). While it lives no other starts; dropping it ends
/// the flight.
#[cfg(not(target_os = "windows"))]
struct IdentifyTicket<'a> {
    acct: &'a account::AccountRuntime,
}

#[cfg(not(target_os = "windows"))]
impl Drop for IdentifyTicket<'_> {
    fn drop(&mut self) {
        let mut flight = self
            .acct
            .identify_flight
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        flight.in_flight = false;
    }
}

/// A ticket for one identify request at `now`, or `None`: one is in flight already, or (`Debounced`) the last one
/// started less than [`IDENTIFY_ON_ACTIVATION_EVERY`] ago.
#[cfg(not(target_os = "windows"))]
fn begin_identify(
    acct: &account::AccountRuntime,
    now: std::time::Instant,
    pace: IdentifyPace,
) -> Option<IdentifyTicket<'_>> {
    let mut flight = acct
        .identify_flight
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if flight.in_flight {
        return None;
    }
    if pace == IdentifyPace::Debounced
        && flight
            .last_started
            .is_some_and(|at| now.saturating_duration_since(at) < IDENTIFY_ON_ACTIVATION_EVERY)
    {
        return None;
    }
    flight.in_flight = true;
    flight.last_started = Some(now);
    Some(IdentifyTicket { acct })
}

/// The active account's session, if it is in memory and does not know its account's user id, asks the server who it is
/// (see [`establish_session_identity`]), through the account's single flight (`pace`, see [`begin_identify`]). `true`
/// when it is named now. A session that is identified asks nothing.
#[cfg(not(target_os = "windows"))]
async fn identify_unidentified_session(
    state: &AppState,
    base_url: &str,
    now: std::time::Instant,
    pace: IdentifyPace,
) -> bool {
    let Ok(acct) = state.active_account() else {
        return false;
    };
    // Captured before the session is read, so the Keychain write below refuses if any transition (a sign-out, a
    // lock, another sign-in) happened after this read.
    let turn = acct.session_generation();
    let email = match acct.session.lock() {
        Ok(guard) => match guard.as_ref() {
            Some(session) => session.email.clone(),
            None => return false,
        },
        Err(_) => return false,
    };
    if session_knows_its_user_id(&acct, email.as_deref()) {
        // Task 12 fix round 2, item 3: an email the naming could not write to the Keychain is written now, in a checked
        // turn (no request is needed: the session is named; the email is the one read from the session in memory
        // after `turn` was captured, so a checked turn means no transition happened since).
        if acct.keychain_email_pending.load(Ordering::SeqCst)
            && let Some(email) = email.as_deref()
            && let Ok(write) = check_session_turn(&acct, &turn)
        {
            write_session_email_to_keychain(&write, &acct, email);
        }
        return false;
    }
    let Some(_ticket) = begin_identify(&acct, now, pace) else {
        return false;
    };
    establish_session_identity(state, &acct, base_url, owner_record_dir().as_deref()).await
}

/// Name an unidentified session in the running app (`pace`), and when that worked, run the engine start it was refused
/// (the same start an unlock runs). Spawned, never awaited: app activation (debounced) and the startup restore's second
/// request (fix round 1, item 7), which never holds up the start.
#[cfg(not(target_os = "windows"))]
async fn identify_then_start(app: tauri::AppHandle, pace: IdentifyPace) {
    let state = app.state::<AppState>();
    let named = match pace {
        IdentifyPace::Debounced => {
            identify_on_app_activation(&state, &runner::api_base_url(), std::time::Instant::now()).await
        }
        IdentifyPace::Now => {
            identify_unidentified_session(&state, &runner::api_base_url(), std::time::Instant::now(), pace).await
        }
    };
    if named {
        if let Err(error) = start_engine_unless_running(app.clone(), &state).await {
            tracing::warn!(%error, "the session was named after the fact, but the sync engine did not start");
        }
        // Area A M7: the reconciler may have failed this session's engine start before it was named, and it would show
        // that failure until "Try again". Keys are here, so it checks again by itself (a Ready domain is only confirmed).
        notify_finder(&state, finder_setup::core::Trigger::KeysArrived);
    }
}

/// What a naming in the background did with the engine (Task 12 fix round 2, item 2).
#[cfg(not(target_os = "windows"))]
#[derive(Debug, PartialEq, Eq)]
enum AfterNaming {
    /// An engine was running: the naming only confirmed the account it runs for, so it keeps running.
    KeptRunning,
    /// The slot was empty (the start before was refused, for a session that did not know who it was): this start.
    Started(EngineStart),
    /// An engine was running, but the owner record of the local data and the named session both know a user id and
    /// the ids differ (fix round 3, M1): it was stopped through the one stop helper, and this start went through the
    /// binding.
    Restarted(EngineStart),
}

/// A naming in the background never restarts a running engine of the same account: it named only the session it fetched
/// for (the token is checked), so it confirmed the account that engine runs for. Only an empty slot gets a start, through
/// `start`. The one exception, an owner record that names another account by user id, is checked first by
/// [`keep_or_rebind_after_naming`].
#[cfg(not(target_os = "windows"))]
fn keep_or_start_after_naming<'g>(
    engine_slot: &mut tokio::sync::MutexGuard<'g, Option<EngineRunner>>,
    start: impl FnOnce(&mut tokio::sync::MutexGuard<'g, Option<EngineRunner>>) -> Result<EngineStart, String>,
) -> Result<AfterNaming, String> {
    if engine_slot.is_some() {
        return Ok(AfterNaming::KeptRunning);
    }
    start(engine_slot).map(AfterNaming::Started)
}

/// Fix round 3, M1: the owner record of the local data and the named session both know a user id, and the ids differ.
#[cfg(not(target_os = "windows"))]
fn owner_is_another_account_by_id(
    owner: Option<&account_binding::Identity>,
    named: &account_binding::Identity,
) -> bool {
    matches!(
        (owner.and_then(|owner| owner.user_id.as_deref()), named.user_id.as_deref()),
        (Some(owner), Some(named)) if owner != named
    )
}

/// The engine after a naming in the background (fix round 3, M1). A running engine is kept
/// ([`keep_or_start_after_naming`]) unless the owner record of the local data and the named session both know a user
/// id and the ids differ: then that engine runs over another account's data, so it is stopped through the one stop
/// helper ([`stop_engine_in_slot`]; an unconfirmed stop makes `start` refuse) and `start` goes through the binding,
/// which resets the data or refuses.
#[cfg(not(target_os = "windows"))]
async fn keep_or_rebind_after_naming<'g>(
    acct: &account::AccountRuntime,
    engine_slot: &mut tokio::sync::MutexGuard<'g, Option<EngineRunner>>,
    owner: Option<&account_binding::Identity>,
    named: &account_binding::Identity,
    start: impl FnOnce(&mut tokio::sync::MutexGuard<'g, Option<EngineRunner>>) -> Result<EngineStart, String>,
) -> Result<AfterNaming, String> {
    if owner_is_another_account_by_id(owner, named)
        && let Some(running) = engine_slot.take()
    {
        stop_engine_in_slot(acct, running).await;
        return start(engine_slot).map(AfterNaming::Restarted);
    }
    keep_or_start_after_naming(engine_slot, start)
}

/// Who the session in memory is, from its email and the cached profile only (never its keys): what the naming named.
/// Read by the start after a naming (M1); no start site reads the session itself.
#[cfg(not(target_os = "windows"))]
fn named_session_identity(acct: &account::AccountRuntime) -> account_binding::Identity {
    let email = acct
        .session
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().and_then(|session| session.email.clone()));
    let profile = acct.cached_profile.lock().ok().and_then(|guard| guard.clone());
    identity_of_session(email.as_deref(), profile.as_ref())
}

/// The engine start after a naming in the background (see [`keep_or_rebind_after_naming`]): like
/// `start_engine_if_possible`, but an engine of the same account that runs is kept, never stopped and respawned.
#[cfg(not(target_os = "windows"))]
async fn start_engine_unless_running(app: tauri::AppHandle, state: &AppState) -> Result<(), String> {
    let Ok(cfg) = DesktopConfig::load() else {
        return Ok(());
    };
    let Some(root) = cfg.sync_root else {
        return Ok(());
    };
    let Ok(acct) = state.active_account() else {
        return Ok(());
    };
    let mut engine_slot = acct.engine.lock().await;
    // M1: the owner record and the named session, read under the slot. An owner that cannot be read compares with
    // nothing, so a running engine is kept, as before.
    let owner = owner_record_dir()
        .and_then(|dir| state_db_from_state_dir(&dir).ok().flatten())
        .and_then(|db| db.owner().ok().flatten())
        .filter(account_binding::Identity::is_known);
    let named = named_session_identity(&acct);
    let after = keep_or_rebind_after_naming(&acct, &mut engine_slot, owner.as_ref(), &named, |slot| {
        engine_start_refusal(&acct)?;
        rehydrate_sync_paused(&acct.sync_paused, cfg.pause_sync);
        spawn_bound_engine(app, state, &acct, slot, root)
    })
    .await?;
    tracing::info!(?after, "after the naming in the background");
    Ok(())
}

/// App activation in the running app ([`identify_then_start`], debounced). Spawned from the window-focus handler.
#[cfg(not(target_os = "windows"))]
async fn on_app_activated(app: tauri::AppHandle) {
    identify_then_start(app, IdentifyPace::Debounced).await;
}

/// "Copy details" (§6.2): the one public `copy_details`, which reads the last 200 lifecycle-log
/// lines itself (lead ruling T5-b).
fn finder_setup_copy_details_impl(state: &AppState) -> Result<String, String> {
    let view = finder_setup_state_impl(state)?;
    Ok(finder_setup::driver::copy_details(
        &view,
        real_app_version(),
        &macos_version_string(),
    ))
}

/// Spec §9: the reconciler's published state (replaces `finder_location_state` on macOS).
#[tauri::command]
fn finder_setup_state(state: State<'_, AppState>) -> Result<finder_setup::driver::FinderSetupView, String> {
    finder_setup_state_impl(&state)
}

/// "Try again" (§5.3 (4)): a fresh retry budget. On macOS and Linux an unidentified session asks the server first
/// ([`finder_setup_try_again`]).
#[tauri::command]
async fn finder_setup_retry(state: State<'_, AppState>) -> Result<(), String> {
    #[cfg(not(target_os = "windows"))]
    {
        finder_setup_try_again(&state, &runner::api_base_url()).await
    }
    #[cfg(target_os = "windows")]
    {
        finder_setup_retry_impl(&state)
    }
}

/// "Copy details" (§6.2): the text the frontend puts on the pasteboard.
#[tauri::command]
fn finder_setup_copy_details(state: State<'_, AppState>) -> Result<String, String> {
    finder_setup_copy_details_impl(&state)
}

/// "Show in Finder" for `not_in_applications` (plan "Spec issues" 9): reveal the running app so
/// the person can drag it to Applications.
#[tauri::command]
fn finder_setup_show_app(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let bundle = finder_setup::launch_location::current_bundle_path()
            .ok_or_else(|| "This copy of Beebeeb is not an app bundle.".to_string())?;
        // The opener's own text can name the bundle's path: it goes to the log, never to the caller.
        app.opener().reveal_item_in_dir(&bundle).map_err(|error| {
            tracing::warn!(%error, "finder setup: could not reveal the app bundle");
            "Could not show Beebeeb in Finder.".to_string()
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Err("Only available on macOS.".to_string())
    }
}

/// `async`, with the read on the blocking pool: it waits up to 3 s for the bridge gate, which a plain
/// `#[tauri::command]` fn would do on the main thread (see [`on_the_blocking_pool`]).
#[tauri::command]
async fn finder_domain_user_enabled() -> Result<Option<bool>, String> {
    on_the_blocking_pool(
        "Could not check whether Beebeeb is turned on in System Settings.",
        file_provider_domain_user_enabled,
    )
    .await
}

/// x-apple.systempreferences URL for the "Login Items & Extensions" pane (System
/// Settings -> General -> Login Items & Extensions on macOS 13+/Ventura's System
/// Settings rewrite onward), which lists File Providers -- the exact screen Issue 4's
/// fix (task 1524) needs the user to open to re-enable the Beebeeb domain. No
/// dedicated "File Providers" sub-anchor is documented anywhere, so this opens the
/// pane itself; the user finds "File Providers" as one of its listed sections (the
/// same path Guus's own screenshot showed: System Settings -> General -> Login Items
/// & Extensions -> File Providers).
///
/// Source (cross-checked 2026-09-28, no official Apple reference exists for these
/// URL schemes): <https://gist.github.com/rmcdongit/f66ff91e0dad78d4d6346a75ded4b751>
/// and <https://github.com/jaywcjlove/SystemSettings-URLs-macOS>, both listing
/// `x-apple.systempreferences:com.apple.LoginItems-Settings.extension` as this pane's
/// stable bundle id since System Settings replaced System Preferences (macOS 13
/// Ventura); still current as of macOS 15 Sequoia per the same sources.
const MACOS_LOGIN_ITEMS_SETTINGS_URL: &str = "x-apple.systempreferences:com.apple.LoginItems-Settings.extension";
/// Fallback used only if the deep link above fails to launch (e.g. a future macOS
/// renames or removes that pane id) -- opens System Settings at its default pane.
const MACOS_SYSTEM_SETTINGS_FALLBACK_URL: &str = "x-apple.systempreferences:";

/// `async`, with the open on the blocking pool: it waits up to 5 s for LaunchServices' answer, which a plain
/// `#[tauri::command]` fn would do on the main thread (see [`on_the_blocking_pool`]).
#[tauri::command]
async fn open_login_items_and_extensions_settings() -> Result<(), String> {
    on_the_blocking_pool(
        "Could not open System Settings.",
        open_login_items_and_extensions_settings_blocking,
    )
    .await
}

fn open_login_items_and_extensions_settings_blocking() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        // Through NSWorkspace, in this process (task 1885), and the answer is LaunchServices': a refusal is the
        // command's error, not a discarded one. No bridge gate: this touches no File Provider state, and it must
        // work exactly when the File Provider is stuck.
        match macos_workspace::open_url(MACOS_LOGIN_ITEMS_SETTINGS_URL) {
            Ok(()) => Ok(()),
            Err(primary) => {
                tracing::warn!(%primary, "could not open the Login Items pane; opening System Settings itself");
                macos_workspace::open_url(MACOS_SYSTEM_SETTINGS_FALLBACK_URL).map_err(|fallback| {
                    tracing::warn!(%fallback, "could not open System Settings");
                    fallback.redacted()
                })
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("Only available on macOS.".to_string())
    }
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
fn finder_location_state() -> Result<FinderInstallState, String> {
    let mut cfg = HeldConfig::load()?;
    match file_provider_installed() {
        Ok(FileProviderStatusOutcome::Installed) => {
            if cfg.finder_install_status.as_deref() != Some("installed") {
                persist_finder_install_result(&mut cfg, true, None)?;
            }
            Ok(finder_install_state_from_config(&cfg, true, None))
        }
        Ok(FileProviderStatusOutcome::NotInstalled) => Ok(finder_install_state_from_config(&cfg, false, None)),
        Ok(FileProviderStatusOutcome::UserDisabled) => {
            // Issue 4's status-path leg (PR #63 Codex review, Onboarding.tsx:488): detected
            // live here, WITHOUT waiting for stabilization (macos_file_provider::status
            // short-circuits via the same decide_install_step used by install()) -- so this
            // is a fresh runtime observation, not a stale persisted one. Pass it through as
            // `runtime_error` using the SAME crafted message `install_finder_location`
            // persists, so `finder_install_state_from_config`'s existing
            // "runtime_error always wins over cfg" precedence gives it priority over
            // whatever is currently persisted (e.g. a stale "timeout" category from before
            // this fix), and `classify_finder_install_error` maps it back to
            // "user_disabled" identically to a fresh install attempt. A GENUINE
            // stabilization timeout (the domain is enabled but never comes up) still falls
            // through to the `Err(error)` arm below unchanged, so it still shows timeout
            // copy, never this one.
            Ok(finder_install_state_from_config(
                &cfg,
                false,
                Some(FINDER_USER_DISABLED_MESSAGE.to_string()),
            ))
        }
        Err(error) => Ok(finder_install_state_from_config(&cfg, false, Some(error))),
    }
}

/// Persist the chosen sync root and start the daemon when possible, but do not
/// claim Finder integration succeeded while the `.appex` is absent.
///
/// Windows and Linux only. On macOS Beebeeb adds itself to Finder (spec 2026-10-06, ruling R5);
/// the macOS variant below only refuses.
#[cfg(not(target_os = "macos"))]
#[tauri::command]
async fn install_finder_location(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: Option<String>,
) -> Result<FinderInstallState, String> {
    #[cfg(target_os = "windows")]
    let _transition = SESSION_TRANSITION.lock().await;
    #[cfg(target_os = "windows")]
    {
        windows_cf::ensure_reactivation_allowed()?;
        SESSION_COMMANDS.generation()?;
    }

    let root = path
        .map(PathBuf::from)
        .unwrap_or_else(config::default_sync_root_suggestion);
    if !root.is_absolute() {
        return Err(format!("Finder location must be absolute: {}", root.display()));
    }
    config::ensure_directory(&root)?;

    let mut cfg = HeldConfig::load()?;
    // Spec section 7: the previous attempt's saved failure is cleared when a new attempt starts.
    begin_finder_install_attempt(&mut cfg)?;
    let started_pending_engine = start_engine_for_pending_finder_install(
        app.clone(),
        &state,
        root.clone(),
        #[cfg(target_os = "windows")]
        &_transition,
    )
    .await?;

    match install_file_provider_domain_blocking().await {
        Err(failure) => {
            stop_pending_finder_install_engine(&state, started_pending_engine).await;
            // Review M1 (round 2): the install's own cleanup removed a domain this attempt
            // added; a folder it kept is saved for the row and shown, like the rollback's. Task 1889: the
            // record is written onto the file as it is now (`cfg` is a read copy from before the slow work),
            // so the save just below cannot put an older row back.
            if let Some(location) = failure.kept_folder.as_deref() {
                finder_removal::remember_kept_folder_held(&mut cfg, location);
                show_preserved_files_alert(&app, Some(location));
            }
            // D1 (task 1683 slice 5): saved OR returned as an error, not both.
            return finder_install_failed(&mut cfg, failure.message);
        }
        Ok(FileProviderInstallOutcome::UserDisabled) => {
            // Issue 4: do not wait, do not treat this as a "Continue without install"
            // situation -- persist a distinct, classifiable reason so the frontend can
            // show the real cause and offer the System Settings deep link instead of a
            // generic error or a silent fall-through to no Finder integration at all.
            stop_pending_finder_install_engine(&state, started_pending_engine).await;
            persist_finder_install_result(&mut cfg, false, Some(FINDER_USER_DISABLED_MESSAGE.to_string()))?;
            return Ok(finder_install_state_from_config(&cfg, false, None));
        }
        Ok(FileProviderInstallOutcome::Installed) => {}
    }
    if let Err(error) = persist_sync_root_and_start_engine(
        app.clone(),
        &state,
        &mut cfg,
        root.clone(),
        #[cfg(target_os = "windows")]
        &_transition,
    )
    .await
    {
        stop_pending_finder_install_engine(&state, started_pending_engine).await;
        // Task 1882: `addDomain` also succeeds on a domain that already existed, so this
        // rollback can remove one that holds un-synced files. They are kept; the alert names
        // the folder. A failed rollback stays silent, as before.
        // Review I2 / task 1889: the record is written onto the file as it is now and into `cfg`, so the save
        // `finder_install_failed` makes below cannot overwrite it with a stale copy.
        // Review M2: a folder kept with a failed removal is surfaced too.
        let removal = remove_file_provider_domain_blocking().await;
        if let Some(location) = removal.map_or_else(
            |failure| failure.kept_location("add-to-finder rollback"),
            |removal| removal.kept_location("add-to-finder rollback"),
        ) {
            finder_removal::remember_kept_folder_held(&mut cfg, &location);
            show_preserved_files_alert(&app, Some(&location));
        }
        // D1 (task 1683 slice 5): saved OR returned as an error, not both.
        return finder_install_failed(&mut cfg, error);
    }
    persist_finder_install_result(&mut cfg, true, None)?;
    Ok(FinderInstallState {
        installed: true,
        path: finder_state_path(&cfg, true),
        status: "installed".to_string(),
        last_error: None,
        last_attempt_at: cfg.finder_install_last_attempt_at,
        reason_category: None,
    })
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
async fn continue_without_finder_location(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: Option<String>,
) -> Result<FinderInstallState, String> {
    #[cfg(target_os = "windows")]
    let _transition = SESSION_TRANSITION.lock().await;
    #[cfg(target_os = "windows")]
    {
        windows_cf::ensure_reactivation_allowed()?;
        SESSION_COMMANDS.generation()?;
    }

    let root = path
        .map(PathBuf::from)
        .unwrap_or_else(config::default_sync_root_suggestion);
    if !root.is_absolute() {
        return Err(format!("Finder location must be absolute: {}", root.display()));
    }
    config::ensure_directory(&root)?;

    let mut cfg = HeldConfig::load()?;
    persist_sync_root_and_start_engine(
        app,
        &state,
        &mut cfg,
        root.clone(),
        #[cfg(target_os = "windows")]
        &_transition,
    )
    .await?;
    Ok(finder_install_state_from_config(&cfg, false, None))
}

/// macOS: Beebeeb adds itself to Finder (spec 2026-10-06, ruling R5). Registered so the command
/// table is the same on every platform; nothing on macOS calls it
/// (tests/finderSetupSourceContract.test.ts).
#[cfg(target_os = "macos")]
#[tauri::command]
async fn install_finder_location(path: Option<String>) -> Result<FinderInstallState, String> {
    let _ = path;
    Err("On macOS Beebeeb adds itself to Finder. Use finder_setup_retry.".to_string())
}

/// macOS: there is no "continue without Finder" (spec §4: Finder setup is automatic). Registered
/// so the command table is the same on every platform; nothing on macOS calls it, and it never
/// reads or writes the install-era state (lead ruling 1).
#[cfg(target_os = "macos")]
#[tauri::command]
async fn continue_without_finder_location(path: Option<String>) -> Result<FinderInstallState, String> {
    let _ = path;
    Err("Finder setup is automatic on macOS".to_string())
}

/// macOS: the reconciler's published view is `finder_setup_state`. Registered so the command
/// table is the same on every platform; nothing on macOS calls it.
#[cfg(target_os = "macos")]
#[tauri::command]
fn finder_location_state() -> Result<FinderInstallState, String> {
    Err("On macOS use finder_setup_state.".to_string())
}

// ── Windows shell integration (Cloud Files) ───────────────────────────────────
//
// Windows mirror of the macOS Finder commands above. The Cloud Files sync root
// plays the role the macOS File Provider domain plays: registering it makes the
// sync folder show up in Explorer with on-demand (cloud-only) placeholders. We
// reuse `FinderInstallState` as the return type so the frontend only needs a
// type alias (`FinderInstallState` → `ShellIntegrationState`); the persisted
// `finder_install_*` config fields double as the shell-integration status store
// on Windows, so existing state helpers apply unchanged.
//
// `installed` here means "the Cloud Files sync root is registered with Windows."
// Registration is idempotent at the OS layer (re-registering the same path is a
// no-op), so these commands are safe to call repeatedly. The live registration
// performed by the engine runner (`windows_cf::connect_root`) and the one done
// here converge on the same OS state.

/// Report whether the Beebeeb Cloud Files sync root is registered with Windows.
/// Parallels `finder_location_state`. Returns `installed: true` once the sync
/// root has been registered (status persisted as "installed" in config).
#[tauri::command]
fn windows_shell_integration_state() -> Result<FinderInstallState, String> {
    #[cfg(target_os = "windows")]
    {
        let cfg = DesktopConfig::load()?;
        // Treat a registered sync root as "installed". We persisted the
        // outcome of the last registration attempt under the finder_install_*
        // fields, so the state is reconstructed exactly like the macOS path.
        let installed = cfg.finder_install_status.as_deref() == Some("installed");
        Ok(finder_install_state_from_config(&cfg, installed, None))
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err("Windows shell integration is only available on Windows.".to_string())
    }
}

/// Register the chosen sync root as a Windows Cloud Files sync root and start
/// the daemon. Parallels `install_finder_location`. On success the folder
/// appears in Explorer as a Beebeeb cloud-synced location.
#[tauri::command]
async fn install_windows_shell_integration(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: Option<String>,
) -> Result<FinderInstallState, String> {
    #[cfg(target_os = "windows")]
    let _transition = SESSION_TRANSITION.lock().await;
    #[cfg(target_os = "windows")]
    {
        windows_cf::ensure_reactivation_allowed()?;
        SESSION_COMMANDS.generation()?;
    }

    #[cfg(target_os = "windows")]
    {
        let root = path
            .map(PathBuf::from)
            .unwrap_or_else(config::default_sync_root_suggestion);
        if !root.is_absolute() {
            return Err(format!("Sync root must be absolute: {}", root.display()));
        }
        config::ensure_directory(&root)?;

        let mut cfg = HeldConfig::load()?;

        // Register the Cloud Files sync root with Windows. Idempotent at the OS
        // layer. The engine runner re-registers + connects callbacks on spawn;
        // doing it here gives immediate feedback in onboarding before the engine
        // ticks.
        if let Err(error) = windows_cf::register_sync_root(&root) {
            let error = error.to_string();
            persist_finder_install_result(&mut cfg, false, Some(error.clone()))?;
            return Err(error);
        }

        // Write the Explorer SHELL registration (Status column + cloud/check
        // overlays + "Beebeeb" sidebar entry + "Free up space" menu) right after
        // the Win32 platform registration. PURELY ADDITIVE — the Win32 path above
        // already drives placeholders + hydration; this adds only the shell
        // metadata that makes the folder look like OneDrive in Explorer. The
        // engine runner re-runs it on spawn (windows_cf::connect_root), so a
        // re-login reconverges. Best-effort: a shell-registration failure must
        // NOT fail onboarding (placeholders + hydration still work without it),
        // so we log and continue rather than returning an error here.
        if let Err(error) = windows_cf::register_shell_sync_root(&root) {
            tracing::warn!(error = %error, "Explorer shell sync-root registration failed during onboarding; overlays/column/sidebar may be absent (hydration unaffected)");
        }

        // Persist the sync root and (re)start the engine, which will connect the
        // Cloud Files fetch callbacks and seed placeholders.
        if let Err(error) = persist_sync_root_and_start_engine(
            app,
            &state,
            &mut cfg,
            root.clone(),
            #[cfg(target_os = "windows")]
            &_transition,
        )
        .await
        {
            persist_finder_install_result(&mut cfg, false, Some(error.clone()))?;
            return Err(error);
        }

        persist_finder_install_result(&mut cfg, true, None)?;
        Ok(FinderInstallState {
            installed: true,
            path: Some(root.to_string_lossy().into_owned()),
            status: "installed".to_string(),
            last_error: None,
            last_attempt_at: cfg.finder_install_last_attempt_at,
            reason_category: None,
        })
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (app, state, path);
        Err("Windows shell integration is only available on Windows.".to_string())
    }
}

#[tauri::command]
async fn reset_macos_integration(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<MacosIntegrationResetResult, String> {
    #[cfg(target_os = "windows")]
    let _transition = SESSION_TRANSITION.lock().await;
    #[cfg(target_os = "windows")]
    {
        windows_cf::ensure_reactivation_allowed()?;
        SESSION_COMMANDS.generation()?;
    }

    let mut warnings = Vec::new();
    #[cfg_attr(target_os = "macos", allow(unused_mut))]
    let mut cfg = HeldConfig::load()?;
    let sync_root_preserved = cfg.sync_root.as_ref().map(|path| path.to_string_lossy().into_owned());

    let queue = match state_db_for_config(&cfg) {
        Ok(Some(db)) => {
            let now = now_unix_seconds();
            match db.queue_diagnostics(now) {
                Ok(queue) => Some((db, queue)),
                Err(error) => {
                    warnings.push(format!("Could not read sync queue diagnostics: {error}"));
                    None
                }
            }
        }
        Ok(None) => None,
        Err(error) => {
            warnings.push(error);
            None
        }
    };

    let pending_operations_preserved = queue.as_ref().map(|(_, queue)| queue.queued).unwrap_or(0);

    let mut removed_cache_files = 0usize;
    let mut skipped_cache_files = 0usize;
    if let Some((db, queue)) = &queue {
        if queue.queued > 0 {
            warnings.push(format!(
                "Preserved {count} queued operation(s); disposable cache cleanup was skipped.",
                count = queue.queued
            ));
        } else {
            let mut cleared_paths = Vec::new();
            match db.disposable_unpinned_cache_paths() {
                Ok(paths) => {
                    for path in paths {
                        let path_buf = PathBuf::from(&path);
                        if !is_disposable_cache_path(&path_buf) {
                            skipped_cache_files += 1;
                            continue;
                        }
                        match std::fs::remove_file(&path_buf) {
                            Ok(()) => {
                                removed_cache_files += 1;
                                cleared_paths.push(path);
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                                cleared_paths.push(path);
                            }
                            Err(error) => {
                                skipped_cache_files += 1;
                                warnings.push(format!("Could not remove cache file {}: {error}", path_buf.display()));
                            }
                        }
                    }
                }
                Err(error) => warnings.push(format!("Could not list disposable cache files: {error}")),
            }
            if !cleared_paths.is_empty()
                && let Err(error) = db.clear_cache_metadata_for_paths(&cleared_paths, now_unix_seconds())
            {
                warnings.push(format!("Could not update cache metadata after cleanup: {error}"));
            }
        }
    }

    // The engine + its status string are per-account (decision 0800); the
    // `cfg`/`sync_root` reads in this handler stay on `DesktopConfig` (Group D).
    let acct = state.active_account()?;
    let RepairEngineStop {
        removed_socket,
        engine_stop_unconfirmed,
    } = stop_engine_for_repair(&acct, &mut warnings, remove_stale_ipc_socket).await;

    // Windows: strip the Explorer SHELL registration (nav-pane entry, Status
    // column, overlays) as part of a full integration reset — the analogue of
    // removing the Finder File Provider domain on macOS below. The Win32 Cloud
    // Files registration + placeholders are intentionally left intact (the reset
    // preserves the sync root + queued operations); this removes only the shell
    // chrome, which a subsequent install / login re-creates via connect_root.
    // Best-effort: a failure (incl. "not registered") is collected as a warning,
    // never fatal.
    #[cfg(target_os = "windows")]
    {
        if let Some(root) = cfg.sync_root.clone() {
            if let Err(error) = windows_cf::unregister_shell_sync_root(&root) {
                warnings.push(format!("Could not remove Explorer shell sync-root entry: {error}"));
            }
        }
    }

    // Spec §5.3 (6): Repair removes through the reconciler, which then runs one fresh check by itself,
    // so a signed-in Mac ends with Beebeeb back in Finder. The engine was stopped above, before the
    // reconciler is told: the fresh check starts it again with the keys still in memory.
    #[cfg(target_os = "macos")]
    let removal = finder_remove_for(&state, finder_setup::core::Trigger::Repair).await;
    #[cfg(not(target_os = "macos"))]
    let removal = remove_file_provider_domain_blocking().await;
    // Task 1882: the removal keeps un-synced files; their folder rides the
    // result to the Sync tab (spec 2026-10-09 §5). A failed removal is one warning, as before.
    let (removed_file_provider_domain, preserved_location) = finder_removal::repair_removal(removal, &mut warnings);
    // Review I2: saved for the Settings › Sync row. On macOS this command saves no config (the reconciler owns
    // the Finder state), so the folder is saved on its own, under the config-write lock; Windows and Linux
    // write it onto the file as it is now (task 1889), and into the copy this command loaded, which the save below
    // goes through.
    if let Some(location) = preserved_location.as_deref() {
        #[cfg(target_os = "macos")]
        save_kept_folder_for_row(&app, location);
        #[cfg(not(target_os = "macos"))]
        finder_removal::remember_kept_folder_held(&mut cfg, location);
    }

    let disabled_autostart = match app.autolaunch().is_enabled() {
        Ok(true) => match app.autolaunch().disable() {
            Ok(()) => true,
            Err(error) => {
                warnings.push(format!("Could not disable Start at login: {error}"));
                false
            }
        },
        Ok(false) => false,
        Err(error) => {
            warnings.push(format!("Could not read Start at login state: {error}"));
            false
        }
    };

    // Windows and Linux only: on macOS the reconciler owns the Finder state and nothing writes the
    // install-era keys (spec 2026-10-06 §6.1). Re-review P1 (task 1882): the removal above cannot be
    // undone, so a failed save still shows the folder.
    #[cfg(not(target_os = "macos"))]
    finish_repair_after_removal(
        &mut cfg,
        preserved_location.as_deref(),
        |cfg| persist_finder_install_result(cfg, false, None),
        |location| surface_kept_folder(&app, location),
    )?;

    Ok(MacosIntegrationResetResult {
        removed_file_provider_domain,
        preserved_location,
        disabled_autostart,
        removed_socket,
        removed_cache_files,
        skipped_cache_files,
        pending_operations_preserved,
        sync_root_preserved,
        engine_stop_unconfirmed,
        warnings,
    })
}

#[cfg(not(target_os = "macos"))]
fn require_open_folder_root(path: Option<String>) -> Result<PathBuf, String> {
    path.filter(|root| !root.trim().is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "Not configured on this PC yet".to_string())
}

#[cfg(all(test, not(target_os = "macos")))]
mod open_folder_tests;

/// Runs `call` on tokio's blocking pool and awaits its answer. For work that blocks, such as the File Provider reads
/// that wait up to 3 s (`BRIDGE_GATE_WAIT`) for the bridge gate: a plain `#[tauri::command]` fn and a native-menu
/// action run on the main thread, where that wait would freeze every window, and inline in an async command it would
/// hold a runtime worker. A call that panicked answers `failed`: the panic's text can carry an OS message, so it goes
/// to the log only (lead ruling T1-4).
async fn on_the_blocking_pool<T: Send + 'static>(
    failed: &'static str,
    call: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(call).await.unwrap_or_else(|error| {
        tracing::warn!(%error, failed, "a blocking task stopped before it answered");
        Err(failed.to_string())
    })
}

/// What "Open in Finder" answers when its work stopped before it answered (see [`on_the_blocking_pool`]).
const OPEN_FOLDER_FAILED: &str = "Could not open the Beebeeb folder.";

// Native menu actions have no frontend status: resolve the configured root here.
fn open_current_finder_location() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let path = None;
    #[cfg(not(target_os = "macos"))]
    let path = Some(get_sync_root()?.ok_or_else(|| "Not configured on this PC yet".to_string())?);
    open_finder_location_blocking(path)
}

/// "Open in Finder" (Explorer, or the file manager). `async`, with the work on the blocking pool: on macOS it reads
/// the Finder location through the bridge gate, which waits up to 3 s for a call in flight, and a plain
/// `#[tauri::command]` fn would make that wait on the main thread.
#[tauri::command]
async fn open_finder_location(path: Option<String>) -> Result<(), String> {
    on_the_blocking_pool(OPEN_FOLDER_FAILED, move || open_finder_location_blocking(path)).await
}

/// The blocking body of [`open_finder_location`]. Its other callers: the menu's "Open in Finder" (spawned onto the
/// blocking pool), the upload action (inside a task), and the new-folder action's fallback when the new folder cannot
/// be revealed (on the main thread from the menu; on macOS only when the folder it just made cannot be resolved).
fn open_finder_location_blocking(path: Option<String>) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        // Task 1885: never `open <path>` as a child. The child inherited the sandbox but not the extension on the
        // URL macOS returned, LaunchServices refused with -54, and `spawn()` had answered Ok. The bridge opens the
        // scoped URL itself and this returns what it answered.
        let _ = path;
        return file_provider_open_location();
    }

    #[cfg(not(target_os = "macos"))]
    {
        let root = require_open_folder_root(path)?;
        config::ensure_directory(&root)?;

        #[cfg(target_os = "windows")]
        {
            std::process::Command::new("explorer")
                .arg(&root)
                .spawn()
                .map_err(|e| format!("open Explorer: {e}"))?;
            return Ok(());
        }

        #[cfg(not(target_os = "windows"))]
        {
            std::process::Command::new("xdg-open")
                .arg(&root)
                .spawn()
                .map_err(|e| format!("open file manager: {e}"))?;
            Ok(())
        }
    }
}

/// What "Show in Finder" answers when its work stopped before it answered (see [`on_the_blocking_pool`]).
const SHOW_IN_FINDER_FAILED: &str = "Could not show the file in your file manager.";

/// `async`, with the work on the blocking pool: on macOS it resolves the item's Finder location through the bridge
/// gate (up to 3 s of wait) and waits for the bridge's answer, which a plain `#[tauri::command]` fn would do on the
/// main thread.
#[tauri::command]
async fn open_in_finder(item_id: String, path: Option<String>) -> Result<(), String> {
    on_the_blocking_pool(SHOW_IN_FINDER_FAILED, move || open_in_finder_blocking(item_id, path)).await
}

fn open_in_finder_blocking(item_id: String, path: Option<String>) -> Result<(), String> {
    let _ = path;

    #[cfg(target_os = "macos")]
    {
        // Task 1885: the item's Finder location is the URL macOS returns for its File Provider identifier (the
        // state database's file id), revealed through NSWorkspace. It used to be `sync_root/<path>`, which on a Mac
        // is the app's private state folder and not a Finder location, opened with a child `open -R` whose failure
        // `spawn()` could not see. The identifier is checked the way every identifier off the wire is.
        ipc_socket::macos_validate_hydrate_item_identifier(&item_id).map_err(str::to_string)?;
        // Shared content is webapp-only by ruling 1701: it is not in Finder, so the File Provider is never asked about it
        // (macOS might fail the lookup, or place it under "Beebeeb"). Refused before any bridge call (task 1885, I3).
        // A provisional id whose create has landed is checked against the server file it became (its alias); the
        // reveal still asks for the id the caller holds.
        let db =
            state_db_for_config(&DesktopConfig::load()?)?.ok_or_else(|| finder_open::ITEM_NOT_KNOWN.to_string())?;
        finder_open::finder_may_show(&item_id, |id| finder_open::item_namespace(&db, id))?;
        file_provider_reveal_item(&item_id)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let cfg = DesktopConfig::load()?;
        let sync_root = cfg
            .sync_root
            .clone()
            .ok_or_else(|| "sync root not configured".to_string())?;
        let db = state_db_for_config(&cfg)?
            .ok_or_else(|| "The local sync database has not been created yet.".to_string())?;
        let entry = db
            .get_file(&item_id)
            .map_err(|e| format!("read shared item: {e}"))?
            .ok_or_else(|| "shared item is not available locally yet".to_string())?;

        reject_unsafe_rel_path(&entry.path)?;
        let local_path = sync_root.join(entry.path.replace('/', std::path::MAIN_SEPARATOR_STR));

        #[cfg(target_os = "windows")]
        {
            std::process::Command::new("explorer")
                .arg("/select,")
                .arg(&local_path)
                .spawn()
                .map_err(|e| format!("open Explorer: {e}"))?;
            Ok(())
        }

        #[cfg(not(target_os = "windows"))]
        {
            let open_path = if local_path.is_file() {
                local_path.parent().unwrap_or(&sync_root)
            } else {
                local_path.as_path()
            };
            std::process::Command::new("xdg-open")
                .arg(open_path)
                .spawn()
                .map_err(|e| format!("open file manager: {e}"))?;
            Ok(())
        }
    }
}

// ── reveal_and_open_file — security helpers ───────────────────────────────────
//
// These helpers are NOT cfg-gated so they compile and are testable on any
// platform (the host CI runs on Linux). Each helper carries an allow(dead_code)
// attribute that fires only on non-Windows platforms where the helpers are not
// called from a #[cfg(target_os="windows")] block; under `cargo test` (which
// compiles the test module) they are referenced and the allow never fires.

/// Layer 1: Pure, cross-platform path-traversal guard.
///
/// Validates `rel_path` (as received from the IPC call, before any separator
/// normalisation) against every traversal vector we know of. Must be kept
/// platform-deterministic — it must catch Windows-style attacks even when
/// compiled and tested on Linux.
///
/// Returns `Err(reason)` on any dangerous input.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn reject_unsafe_rel_path(rel_path: &str) -> Result<(), String> {
    // 1. Reject empty or whitespace-only input.
    if rel_path.trim().is_empty() {
        return Err("rel_path must not be empty".to_string());
    }

    // 2. Reject UNC paths (\\server\share or //server/share).
    if rel_path.starts_with("\\\\") || rel_path.starts_with("//") {
        return Err("rel_path must not be a UNC path".to_string());
    }

    // 3. Reject absolute Unix paths (leading '/') and root-relative Windows
    //    paths (leading '\').
    if rel_path.starts_with('/') || rel_path.starts_with('\\') {
        return Err("rel_path must not be absolute".to_string());
    }

    // 4. Reject Windows drive-letter prefixes: 'C:' (followed by anything,
    //    including just 'C:foo' or 'C:\foo'). On Linux `std::path::Component`
    //    never returns `Prefix`, so we must catch this with string inspection.
    {
        let bytes = rel_path.as_bytes();
        if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
            return Err("rel_path must not contain a drive-letter prefix".to_string());
        }
    }

    // 5. Inspect every path component (split on BOTH '/' and '\') for
    //    traversal and ambiguous components in any separator convention.
    for component in rel_path.split(['/', '\\']) {
        if component.is_empty() || component == "." {
            return Err("rel_path must not contain empty or '.' components".to_string());
        }
        if component == ".." {
            return Err("rel_path must not contain '..' components".to_string());
        }
    }

    // 6. Defense-in-depth: also run the platform's path parser.  On Windows
    //    this catches `Component::Prefix` (drive letters, UNC) and
    //    `Component::RootDir`; on Linux it catches `Component::RootDir` and
    //    `Component::ParentDir` for Unix-style paths. Cross-platform: catches
    //    `Component::ParentDir` regardless of OS.
    use std::path::{Component, Path};
    for component in Path::new(rel_path).components() {
        match component {
            Component::ParentDir => {
                return Err("rel_path must not contain '..' (ParentDir) components".to_string());
            }
            Component::RootDir => {
                return Err("rel_path must not be rooted".to_string());
            }
            Component::Prefix(_) => {
                return Err("rel_path must not contain a drive/UNC prefix".to_string());
            }
            Component::CurDir => {
                return Err("rel_path must not contain '.' (CurDir) components".to_string());
            }
            Component::Normal(_) => {}
        }
    }

    Ok(())
}

/// Layer 2: Canonicalization containment check.
///
/// Returns true iff `full` is strictly inside (or equal to) `root` after
/// both paths are canonicalized with the same function — keeping the `\\?\`
/// verbatim prefix consistent on Windows so `starts_with` works correctly.
///
/// `pub(crate)` and no longer `allow(dead_code)`: it is now used cross-platform
/// by `engine_bridge::hydrate_dest_is_allowed` (task 1247 — validating the
/// IPC-supplied hydrate destination), not just by the Windows reveal path.
pub(crate) fn is_contained(root: &std::path::Path, full: &std::path::Path) -> bool {
    let Ok(root_canon) = std::fs::canonicalize(root) else {
        return false;
    };
    let Ok(full_canon) = std::fs::canonicalize(full) else {
        return false;
    };
    full_canon.starts_with(&root_canon)
}

/// Layer 3: Executable denylist for the OPEN step.
///
/// Returns true when the file's extension (case-insensitive) is a type that
/// Windows will execute when opened with `ShellExecute` / `open_path`.
/// The REVEAL step (explorer /select) is safe for any contained path; this
/// guard only blocks the `open_path` call.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn is_dangerous_executable(path: &std::path::Path) -> bool {
    const DANGEROUS_EXTS: &[&str] = &[
        "exe", "bat", "cmd", "com", "scr", "lnk", "url", "hta", "ps1", "vbs", "vbe", "js", "jse", "wsf", "wsh", "msi",
        "reg", "cpl", "pif", "msc", "jar",
    ];
    let Some(ext) = path.extension() else {
        return false;
    };
    let ext_lower = ext.to_string_lossy().to_lowercase();
    DANGEROUS_EXTS.contains(&ext_lower.as_str())
}

/// Open the folder containing a recently-changed file in Windows Explorer
/// (selecting the file) and then open the file with its default application.
///
/// `rel_path` is the path as stored in the state DB — relative to the sync
/// root, using the same separator convention as the DB (forward-slash or
/// back-slash; both are normalised below). The full path is `sync_root/rel`.
///
/// Windows only. On other platforms this returns `Err("unsupported")` so
/// the binary still compiles cross-platform.
///
/// Zero-knowledge: the file path is NOT logged — only a fixed message is.
///
/// Security: four layers of defense against path-traversal / RCE:
///   1. `reject_unsafe_rel_path` — pure string checks before any path join.
///   2. `is_contained` — canonicalization containment after join.
///   3. Fallback containment — parent-dir canonicalization when the file
///      doesn't exist yet (cloud placeholder not yet hydrated).
///   4. `is_dangerous_executable` — executable denylist on the OPEN step only.
#[tauri::command]
fn reveal_and_open_file(app: tauri::AppHandle, rel_path: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use tauri_plugin_opener::OpenerExt;

        // ── Layer 1: reject traversal before any path operations ──────────────
        reject_unsafe_rel_path(&rel_path)?;

        let sync_root = DesktopConfig::load()
            .ok()
            .and_then(|c| c.sync_root)
            .ok_or_else(|| "sync root not configured".to_string())?;

        // Normalise separators and resolve the full path.
        let rel = rel_path.replace('/', std::path::MAIN_SEPARATOR_STR);
        let full_path = sync_root.join(&rel);

        // ── Layer 2: canonicalization containment ─────────────────────────────
        // If the file exists, canonicalize both root and full_path and verify.
        // If the file doesn't exist yet (cloud placeholder not materialized),
        // fall back to verifying the parent directory is contained AND the
        // file name is a single normal component with no embedded separators.
        let contained = if full_path.exists() {
            is_contained(&sync_root, &full_path)
        } else {
            // Fallback: verify parent is contained.
            let parent_ok = full_path.parent().map(|p| is_contained(&sync_root, p)).unwrap_or(false);
            // And the file name component must not contain any separator.
            let name_ok = full_path
                .file_name()
                .map(|n| {
                    let s = n.to_string_lossy();
                    !s.contains('/') && !s.contains('\\')
                })
                .unwrap_or(false);
            parent_ok && name_ok
        };

        if !contained {
            tracing::warn!("reveal_and_open_file: path rejected by containment check");
            return Err("path is outside sync root".to_string());
        }

        // ── (a) REVEAL — always safe for any contained path ───────────────────
        tracing::info!("reveal_and_open_file: launching Explorer");
        std::process::Command::new("explorer")
            .arg("/select,")
            .arg(&full_path)
            .spawn()
            .map_err(|e| format!("reveal in Explorer: {e}"))?;

        // ── (b) OPEN — blocked for executable file types (Layer 3) ───────────
        if is_dangerous_executable(&full_path) {
            // Zero-knowledge: no path in the log.
            tracing::warn!("reveal_and_open_file: open blocked for executable file type");
            return Ok(());
        }

        tracing::info!("reveal_and_open_file: opening file with default app");
        let path_str = full_path.to_string_lossy().into_owned();
        if let Err(e) = app.opener().open_path(path_str, None::<&str>) {
            tracing::warn!(error = %e, "reveal_and_open_file: failed to open file with default app");
        }

        return Ok(());
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (app, rel_path);
        Err("unsupported".to_string())
    }
}

// ── Unit tests for reveal_and_open_file security helpers ─────────────────────
#[cfg(test)]
mod reveal_and_open_file_tests;

/// What [`sync_status`] counts in the state DB: rows moving (downloading + uploading), cloud-only rows (files and
/// folders, as before task 2243), conflicts, and changes held back.
fn sync_status_counts(db: &state_db::StateDb) -> (u32, u32, u32, u32) {
    use state_db::FileStatus;
    // Downloading + Uploading both count toward "in-flight sync work" for the WebView's syncing pill: there is no
    // user-meaningful difference between "we're pulling" and "we're pushing" at the indicator level.
    let count = |status| db.count_by_status(status).unwrap_or(0);
    let syncing = count(FileStatus::Downloading) + count(FileStatus::Uploading);
    // Desktop 0.8.15: queued changes held back (the `auth` pause) are not syncing; the UI shows them as
    // "N changes on hold".
    let on_hold = db.count_operations_on_hold().unwrap_or(0);
    (
        syncing,
        count(FileStatus::CloudOnly),
        count(FileStatus::Conflict),
        on_hold,
    )
}

/// Lightweight status endpoint for the WebView's settings page.
/// Reports whether the Rust side has a session, where the sync root
/// is on disk, whether the engine task is currently running, and the
/// per-status file counts the Status / SyncFolder pages render.
///
/// For richer state (per-tick `idle` / `syncing` transitions) the
/// WebView should still listen to the `engine-status` Tauri event —
/// this command is the first-paint poll the settings UI uses while
/// the event stream is still ramping up.
///
/// File counts are read from the app-local SQLite state DB via a fresh
/// connection per call, so the page cache starts empty each time: every
/// count must come from a count query through an index, never from loading
/// rows. Task 2243: counting the cloud-only files by loading them cost about
/// 1,100 page reads per call on a 43k-row DB, polled every few seconds by
/// every window ([`sync_status_counts`]). If the DB doesn't exist yet (first
/// launch, sync root not picked, or DB not initialised because no engine has
/// run yet) we return zeros.
///
/// `session_only` (task 2243): only `logged_in` and `session_revision`, read from
/// memory, with no config file and no state DB. `AccountSessionBoundary` polls
/// this every second in every window, hidden ones included, to see a sign-out.
#[tauri::command]
async fn sync_status(state: State<'_, AppState>, session_only: Option<bool>) -> Result<serde_json::Value, String> {
    // session / engine / engine_state are per-account (decision 0800);
    // `auth_present` stays on `AppState`.
    let acct = state.active_account()?;
    let vault_unlocked = acct.session.lock().map(|g| g.is_some()).unwrap_or(false);
    let auth_present = state.auth_present.lock().map(|g| *g).unwrap_or(false);
    let logged_in = vault_unlocked || auth_present;
    if session_only == Some(true) {
        return Ok(serde_json::json!({
            "logged_in": logged_in,
            "session_revision": UI_SESSION_REVISION.load(std::sync::atomic::Ordering::SeqCst),
        }));
    }
    // Task 1546 finding 5: 3+ consecutive 401s from the engine's heartbeat /
    // sync-tick calls flip this — the trigger for the persistent "signed out
    // on this device" banner. Explicit and independent of `logged_in`: the
    // stale token can still be installed (so `logged_in` reads `true`) while
    // every call it makes fails.
    let auth_expired = acct.auth_health.is_expired();
    let sync_root_path = DesktopConfig::load().ok().and_then(|c| c.sync_root);
    #[cfg(target_os = "macos")]
    let sync_root = None::<String>;
    #[cfg(not(target_os = "macos"))]
    let sync_root = sync_root_path.as_ref().map(|p| p.to_string_lossy().into_owned());
    let engine_running = acct.engine.lock().await.is_some();

    // Collapse the runner's five-state stream into the tri-state
    // string the WebView's settings pages render. `idle` and
    // `syncing` both mean "engine is alive and doing work" — UI
    // wants a single "running" pill for either; the per-tick
    // distinction is on the engine-status event stream.
    let engine = {
        let raw = acct
            .engine_state
            .lock()
            .map(|g| g.clone())
            .unwrap_or_else(|_| "stopped".to_string());
        let engine = engine_status::sync_status_engine(raw.as_str());
        if logged_in && !vault_unlocked && engine == "stopped" {
            "locked".to_string()
        } else {
            engine.to_string()
        }
    };

    // Count files by status, opening a short-lived connection at the canonical
    // app-local state.db path. No configured root / no DB degrades to zeros.
    let counts = if sync_root_path.is_some() {
        let db_path = state_paths::beebeeb_state_dir()?.join(state_paths::STATE_DB_FILENAME);
        if db_path.exists() {
            state_db::StateDb::open(&db_path)
                .ok()
                .map(|db| sync_status_counts(&db))
                .unwrap_or((0, 0, 0, 0))
        } else {
            (0, 0, 0, 0)
        }
    } else {
        (0, 0, 0, 0)
    };

    Ok(serde_json::json!({
        "logged_in": logged_in,
        "session_revision": UI_SESSION_REVISION.load(std::sync::atomic::Ordering::SeqCst),
        "auth_expired": auth_expired,
        "sync_root": sync_root,
        "engine_running": engine_running,
        "vault_unlocked": vault_unlocked,
        "engine": engine,
        // Why sync did not start, when the local-data binding refused it (ruling P): `{code, sentence}` or null.
        "engine_refusal": engine_refusal_view(&acct),
        "syncing": counts.0,
        "cloud_only": counts.1,
        "conflicts": counts.2,
        // Desktop 0.8.15: changes held back, never counted in `syncing`.
        "changes_on_hold": counts.3,
        // Desktop 0.8.15: the start-up guard's pass, `{running, done, total}` (Windows; at rest
        // elsewhere). While it runs, uploads and downloads flow; only freeing work waits.
        "startup_guard": crate::unknown_check::STARTUP_GUARD.progress(),
        // WS1 — storage byte totals for the tray status line.
        // These are `null` here because polling the billing API on every
        // 3-second status tick would hammer the server. The tray's
        // storage line should call `desktop_storage_summary` (which already
        // exists and hits the API once on demand) rather than piggy-backing
        // on the lightweight `sync_status` poll. Field names are reserved
        // here so the frontend contract is stable once wired.
        "used_bytes": serde_json::Value::Null,
        "quota_bytes": serde_json::Value::Null,
    }))
}

#[derive(Debug, serde::Deserialize)]
struct BillingUsageResponse {
    used_bytes: i64,
    quota_bytes: i64,
}

#[derive(Debug, serde::Serialize)]
struct DesktopStorageSummary {
    used_bytes: i64,
    quota_bytes: i64,
    cache_bytes: i64,
    pinned_bytes: i64,
}

/// Storage summary for the control center.
///
/// Remote usage/quota comes from the API's billing usage endpoint. Local
/// cache/pinned bytes are read from the desktop SQLite state DB so the UI can
/// distinguish account storage from bytes actually present on this Mac.
#[tauri::command]
async fn desktop_storage_summary(state: State<'_, AppState>) -> Result<DesktopStorageSummary, String> {
    session_command!(async {
        let acct = state.active_account()?;
        let token = {
            let guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
            guard
                .as_ref()
                .map(|session| session.token.clone())
                .ok_or_else(|| "vault is locked".to_string())?
        };

        let storage_client = reqwest::Client::builder()
            .default_headers(api_client::provenance_headers())
            .build()
            .map_err(|e| format!("reqwest build: {e}"))?;
        let usage: BillingUsageResponse = storage_client
            .get(format!("{}/api/v1/billing/usage", runner::api_base_url()))
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| format!("load storage usage: {e}"))?
            .error_for_status()
            .map_err(|e| format!("load storage usage: {e}"))?
            .json()
            .await
            .map_err(|e| format!("parse storage usage: {e}"))?;

        let (cache_bytes, pinned_bytes) = if DesktopConfig::load().ok().and_then(|cfg| cfg.sync_root).is_some() {
            let db_path = state_paths::beebeeb_state_dir()?.join(state_paths::STATE_DB_FILENAME);
            if db_path.exists() {
                let values = (|| {
                    let db = state_db::StateDb::open(&db_path).ok()?;
                    let pinned = db.cache_bytes_by_effective_pin(true).ok()?.max(0);
                    let unpinned = db.cache_bytes_by_effective_pin(false).ok()?.max(0);
                    Some((unpinned, pinned))
                })();
                values.unwrap_or((0, 0))
            } else {
                (0, 0)
            }
        } else {
            (0, 0)
        };

        let used_bytes = usage.used_bytes.max(0);
        let quota_bytes = usage.quota_bytes.max(0);

        // Feed the Windows Explorer breadcrumb status-flyout snapshot with the same
        // usage/quota numbers the control center shows, so the flyout's storage bar
        // ("X GB used of Y (Z%)") stays in agreement. Cheap atomic stores; the COM
        // `GetStatusUI` reads them without a lock. No-op on non-Windows.
        #[cfg(target_os = "windows")]
        windows_cf::status_ui::set_quota(used_bytes as u64, quota_bytes as u64);

        Ok(DesktopStorageSummary {
            used_bytes,
            quota_bytes,
            cache_bytes,
            pinned_bytes,
        })
    })
}

/// Everything the menu-bar popover needs for its first frame, in one call (task 1683
/// slice 2, spec section 9). The popover calls it when it is shown (the `popover-shown`
/// event) and after its own actions; the macOS main window reads it a few seconds after
/// each read while it is open (slice 1).
///
/// Reads, in this order: the session (locked / signed out / session ended), the live
/// pause flag, the engine's last `engine-status` (real state, progress, last
/// successful check, reason), the Finder install state (macOS only), the storage
/// summary (cached for five minutes, refetched after an unlock, never when the vault
/// is locked; a refetch holds this call for `popover_data::STORAGE_WAIT` at most and
/// a failed one is not tried again for `STORAGE_FAILURE_BACKOFF_SECS`, task 2179), and
/// the state DB (queue, recent transfers, conflicts). File names are read only while
/// the vault is unlocked, like `desktop_file_overview`.
#[tauri::command]
async fn popover_snapshot(
    state: State<'_, AppState>,
    runtime: State<'_, popover_data::PopoverRuntime>,
    activity_limit: Option<usize>,
) -> Result<popover_data::PopoverSnapshotDto, String> {
    // Windows admission: the snapshot reads the session token, so it leases like every
    // credential command (tests/windows_session_wiring.rs guards this).
    session_command!(async {
        use popover_data::{SnapshotInputs, cached_storage, fetch_storage_usage};

        let now = now_unix_seconds();
        let acct = state.active_account()?;
        let (vault_unlocked, token, session_email) = {
            let guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
            (
                guard.is_some(),
                guard.as_ref().map(|s| s.token.clone()),
                guard.as_ref().and_then(|s| s.email.clone()),
            )
        };
        let auth_present = state.auth_present.lock().map(|g| *g).unwrap_or(false);
        let logged_in = vault_unlocked || auth_present;
        let auth_expired = acct.auth_health.is_expired();
        let paused = acct.sync_paused.load(std::sync::atomic::Ordering::Relaxed);
        let email = session_email.or_else(|| acct.auth_email.lock().ok().and_then(|guard| guard.clone()));

        // Finder: the reconciler's published view (spec 2026-10-06). It never asks the OS here.
        let (finder, finder_reason) = if cfg!(target_os = "macos") {
            match state.finder_setup.get().map(|handle| handle.view()) {
                Some(view) => (view.setup, view.reason),
                None => (crate::surfaces::phase::FinderSetup::Missing, None),
            }
        } else {
            (crate::surfaces::phase::FinderSetup::Ready, None)
        };

        // The storage wait (at most `STORAGE_WAIT`) and the state DB read run side by side, not one after the other
        // (#139 review P3-10).
        let storage = async {
            match token {
                Some(token) if vault_unlocked => {
                    cached_storage(
                        &state.popover_storage,
                        now,
                        VAULT_EPOCH.load(std::sync::atomic::Ordering::SeqCst),
                        || session_bound(async move { fetch_storage_usage(&runner::api_base_url(), &token).await }),
                    )
                    .await
                }
                _ => None,
            }
        };
        let limit = activity_limit.unwrap_or(popover_data::DEFAULT_ACTIVITY_LIMIT);
        let db_view = async {
            if !vault_unlocked {
                return Ok(None);
            }
            tokio::task::spawn_blocking(move || -> Result<_, String> {
                let Some(db) = DesktopConfig::load().and_then(|cfg| state_db_for_config(&cfg))? else {
                    return Ok(None);
                };
                popover_data::gather_db_view(&db, limit, now).map(Some)
            })
            .await
            .map_err(|e| format!("popover data task failed: {e}"))?
        };
        let (storage, db_view) = tokio::join!(storage, db_view);
        let db_view = db_view?;
        // The transfers as of now, after the wait (a file moving now is syncing, `assemble`).
        let board = runtime.transfers.active();
        let popover_data::DbView {
            backlog,
            conflicts,
            in_flight,
            failed,
            recent,
        } = db_view.unwrap_or_default();

        let engine = runtime.status.lock().map(|view| view.clone()).unwrap_or_default();
        let mut snapshot = popover_data::assemble(SnapshotInputs {
            now,
            logged_in,
            vault_unlocked,
            auth_expired,
            paused,
            email,
            engine,
            finder,
            finder_reason,
            finder_failure_elsewhere: false,
            storage,
            backlog,
            conflicts,
            in_flight,
            failed,
            recent,
            board,
            activity_limit: limit,
        });
        snapshot.update = macos_popover::update_offer();
        #[cfg(target_os = "macos")]
        {
            let sending = state.sign_out.phase.lock().map(|phase| phase.is_some()).unwrap_or(false);
            if sending {
                let read = tokio::task::spawn_blocking(read_waiting_changes)
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|read| read);
                snapshot.sign_out_sending = Some(macos_popover::Sending {
                    changes_left: macos_popover::changes_left(read),
                });
            }
        }
        Ok(snapshot)
    })
}

// `pub(crate)` so `free_up_space_blocking` (also `pub(crate)`, reused by the
// Windows shell status-flyout command) can name it as its return type.
#[derive(Debug, serde::Serialize)]
pub(crate) struct FreeUpSpaceResult {
    pub(crate) bytes_freed: u64,
}

/// "Free up space now" — dehydrate every unpinned, fully-local file back to a
/// cloud-only placeholder and delete its on-disk cache copy. Pinned files
/// (effective pin = pinned) are always kept. Mirrors the Windows "Free up
/// space" action and the macOS Storage panel button.
///
/// Returns the number of bytes reclaimed from disk. Safe to call with no sync
/// root / no state DB (returns `bytes_freed: 0`).
///
/// `bytes_freed` is summed from the actual on-disk sizes of the cache files we
/// successfully delete — so the figure reflects bytes really removed from disk,
/// never an estimate. The DB transition (cache_path → NULL, status →
/// cloud_only) is done by `evict_unpinned_cache_until_under(0, now)`. File
/// deletes are bounded to paths under EITHER the sync root OR the engine's
/// system cache dir (`dirs::cache_dir()/beebeeb`) — the two locations the
/// engine writes cache copies — so a corrupt cache_path can never make us
/// delete outside the vault's cache.
///
/// The DB + filesystem work runs on a blocking thread (`spawn_blocking`) so the
/// IPC command thread isn't held by SQLite + `remove_file` syscalls.
#[tauri::command]
async fn free_up_space() -> Result<FreeUpSpaceResult, String> {
    tokio::task::spawn_blocking(free_up_space_blocking)
        .await
        .map_err(|e| format!("free_up_space task join: {e}"))?
}

/// Allowed roots a cache file may live under for `free_up_space` to delete it.
/// The engine writes hydrated cache copies under the sync root (Windows
/// Cloud Files placeholders) and under the system cache dir
/// (`dirs::cache_dir()/beebeeb`, e.g. the macOS File Provider staging area).
/// Both are trusted, engine-written locations.
fn free_up_space_allowed_roots(sync_root: &std::path::Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(canon) = sync_root.canonicalize() {
        roots.push(canon);
    } else {
        roots.push(sync_root.to_path_buf());
    }
    if let Some(cache_dir) = dirs::cache_dir() {
        let beebeeb_cache = cache_dir.join("beebeeb");
        match beebeeb_cache.canonicalize() {
            Ok(canon) => roots.push(canon),
            Err(_) => roots.push(beebeeb_cache),
        }
    }
    // Task 1670 round 2, Codex P1 (PR #75): same reasoning as
    // `disposable_cache_roots` above — a macOS hydrate-cache `cache_path`
    // must be an allowed target for the smart-cache "free up space" eviction
    // path too, not just sign-out's purge.
    #[cfg(target_os = "macos")]
    {
        let hydrate_dir = crate::ipc_socket::macos_hydrate_cache_dir();
        match hydrate_dir.canonicalize() {
            Ok(canon) => roots.push(canon),
            Err(_) => roots.push(hydrate_dir),
        }
    }
    roots
}

#[cfg(target_os = "linux")]
fn remove_linux_freedesktop_thumbnails_for_evicted_files(
    db: &state_db::StateDb,
    sync_root: &std::path::Path,
    file_ids: &[String],
) {
    for file_id in file_ids {
        let entry = match db.get_file(file_id) {
            Ok(Some(entry)) if !entry.is_dir() => entry,
            Ok(_) => continue,
            Err(error) => {
                tracing::debug!(file_id = %file_id, error = %error, "linux thumbnail cleanup skipped: state row unavailable");
                continue;
            }
        };
        let Some(source_path) = linux_thumbnail::source_path_under_sync_root(sync_root, &entry.path) else {
            continue;
        };
        if let Err(error) = linux_thumbnail::remove_freedesktop_thumbnails(&source_path) {
            tracing::warn!(file_id = %file_id, error = %error, "linux freedesktop thumbnail cleanup failed");
        }
    }
}

// `pub(crate)` so the Windows Explorer status-flyout command
// (`windows_cf::status_ui::FreeUpSpaceCommand::Invoke`) can reuse the exact same
// reclaim path the `free_up_space` IPC uses — one implementation, two callers
// (the WebView IPC and the shell flyout button). It is a synchronous blocking fn
// (DB + filesystem work), so the COM `Invoke` can call it directly off the
// arbitrary thread Windows delivers the command on, without a tokio runtime.
pub(crate) fn free_up_space_blocking() -> Result<FreeUpSpaceResult, String> {
    let cfg = DesktopConfig::load()?;
    let Some(sync_root) = cfg.sync_root.clone() else {
        return Ok(FreeUpSpaceResult { bytes_freed: 0 });
    };
    let db_path = state_paths::beebeeb_state_dir()?.join(state_paths::STATE_DB_FILENAME);
    if !db_path.exists() {
        return Ok(FreeUpSpaceResult { bytes_freed: 0 });
    }
    let db = state_db::StateDb::open(&db_path).map_err(|e| format!("open state.db: {e}"))?;

    #[cfg(target_os = "windows")]
    {
        free_up_space_windows(&db, &sync_root)
    }
    #[cfg(not(target_os = "windows"))]
    {
        // Snapshot the on-disk cache paths of every UNPINNED, LOCAL file BEFORE
        // any DB transition (eviction nulls `cache_path`). Pinned files are
        // excluded by the query, so they are always kept.
        let paths = db
            .disposable_unpinned_cache_paths()
            .map_err(|e| format!("list cache paths: {e}"))?;
        free_up_space_unix(&db, &sync_root, paths)
    }
}

/// Windows reclaim path (task 0781). On Cloud Files the hydrated bytes live
/// INSIDE the placeholder's data stream, not in a separate cache copy — so the
/// old Unix-style `remove_file` would either delete the user's whole file or
/// (on a still-present placeholder) no-op while the DB falsely flipped the row
/// to cloud_only with `bytes_freed = 0` (fictional free).
///
/// Here we `CfDehydratePlaceholder` each unpinned file, computing `bytes_freed`
/// from the REAL reclaimed size, and only mark a row cloud_only AFTER its
/// dehydrate succeeds. Pinned files are already excluded from `paths`. The same
/// allowed-roots guard is kept: we only touch placeholders under the sync root
/// (or the engine cache dir), never an arbitrary path from a corrupt DB row.
#[cfg(target_os = "windows")]
fn free_up_space_windows(db: &state_db::StateDb, sync_root: &std::path::Path) -> Result<FreeUpSpaceResult, String> {
    // Reclaim targets are the UNPINNED, LOCAL files — addressed by their
    // server-relative path, NOT by `cache_path`. On Windows CF the bytes live
    // inside the placeholder in the sync root, and `cache_path` records the
    // already-deleted `%TEMP%` decrypt path from the fetch callback, so it is
    // useless as a dehydration target. We reconstruct the real placeholder path
    // exactly as `windows_cf::populate_placeholders` does.
    let candidates = db
        .unpinned_local_files_for_dehydration()
        .map_err(|e| format!("list dehydration candidates: {e}"))?;

    let allowed_roots = free_up_space_allowed_roots(sync_root);
    let mut bytes_freed: u64 = 0;
    let mut dehydrated_ids: Vec<String> = Vec::new();

    for (file_id, rel_path, _size_bytes) in candidates {
        let rel = rel_path.trim_matches('/');
        if rel.is_empty() {
            continue;
        }
        let local_path = sync_root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        // Path-safety: only dehydrate placeholders that actually resolve under
        // an allowed engine root (the sync root). A corrupt `path` row can
        // never point us at a file outside the vault.
        let Ok(canonical) = local_path.canonicalize() else {
            // Placeholder missing on disk — nothing to reclaim; leave the row
            // for the next reconcile to re-seed.
            continue;
        };
        if !allowed_roots.iter().any(|root| canonical.starts_with(root)) {
            continue;
        }
        // Pin TOCTOU re-check: `unpinned_local_files_for_dehydration()` filtered
        // on pin state at query time, but the user could have PINNED this file in
        // the window between that query and now. Dehydrating it would evict bytes
        // the user just asked to keep offline. `mark_cloud_only_after_dehydrate`
        // already re-applies the pin predicate (so a newly-pinned file is never
        // flipped to cloud_only — it self-heals on next open), but that runs
        // AFTER the destructive dehydrate. Re-read the row's effective pin state
        // immediately before dehydrating and skip if it is now pinned, so we
        // never reclaim a pinned file's bytes in the first place. A read error
        // here is non-fatal: fall through to the existing dehydrate + the
        // post-dehydrate predicate guard.
        if let Ok(Some(contract)) = db.get_file_contract_state(&file_id) {
            if contract.effective_pin_state() == crate::state_db::PinState::Pinned {
                tracing::debug!(file_id = %file_id, "free_up_space: file pinned after enumeration; keeping local");
                continue;
            }
        }
        // Task 2240 (spec D2, the lead's ruling on self-review #1): confirm before evict. A pin made in Explorer
        // reaches the DB only when the tick's folder listing shows it, and a listing can lag while another process
        // holds the file open, so the DB's "unpinned" may be stale. The file's own attributes decide: pinned on disk
        // (or unreadable) keeps it local, and its OS pin is never cleared below.
        let direct = crate::windows_cf::placeholders::read_os_placeholder_state(&canonical);
        let pinned_on_disk = |attrs: u32| {
            crate::state_db::decode_os_state(attrs, crate::state_db::FileStatus::Local).1
                == Some(crate::state_db::PinState::Pinned)
        };
        if direct.is_none_or(pinned_on_disk) {
            tracing::info!(file_id = %file_id, "free_up_space: the file is pinned on disk (or unreadable); keeping it local");
            continue;
        }
        // OS-level unpin BEFORE dehydrating. This file passed both pin checks
        // above, so it is MEANT to be reclaimed — but its placeholder may still
        // carry a pin it inherits from a pinned folder, which the DB does not
        // reflect (an explicit pin on the file itself was refused just above).
        // `CfDehydratePlaceholder` fails with ERROR_CLOUD_FILE_PINNED on
        // any OS-pinned placeholder, so we clear the OS pin first (non-recursive —
        // this is a single file). Best-effort: a failure here is logged (never the
        // path) and we still attempt the dehydrate, which simply no-ops/errors if
        // the file was genuinely pinned.
        // Task 1640: never free a file whose bytes are a change that is not sent (an edit, a
        // queued upload) or are not known to be the agreed content: they are the only copy.
        if !crate::windows_cf::placeholders::local_bytes_agreed(db, &file_id, &canonical) {
            tracing::warn!(file_id = %file_id, "free_up_space: the file holds a change that is not sent; keeping it local");
            continue;
        }
        if let Err(error) = crate::windows_cf::placeholders::set_pin_state(&canonical, false, false) {
            tracing::warn!(file_id = %file_id, error = %error, "free_up_space: OS unpin before dehydrate failed; attempting dehydrate anyway");
        }
        match crate::windows_cf::placeholders::dehydrate_placeholder(&canonical) {
            Ok(freed) => {
                bytes_freed = bytes_freed.saturating_add(freed);
                // Only NOW is this file genuinely cloud-only on disk — record
                // it so the DB flip below applies ONLY to files we actually
                // dehydrated. (`freed == 0` still means "successfully cloud-only
                // / already dehydrated", a valid end state.)
                dehydrated_ids.push(file_id);
            }
            Err(error) => {
                // Leave the DB row `local` so the next sweep retries; never
                // claim space we did not actually reclaim. Zero-knowledge: log
                // the file_id only.
                tracing::warn!(file_id = %file_id, error = %error, "free_up_space: CfDehydratePlaceholder failed; keeping file local");
            }
        }
    }

    // Mark cloud_only ONLY for the files we actually dehydrated.
    let now = now_unix_seconds();
    if let Err(e) = db.mark_cloud_only_after_dehydrate(&dehydrated_ids, now) {
        tracing::warn!(error = %e, "free_up_space: could not mark dehydrated files cloud_only");
    }

    tracing::info!(bytes_freed, "free_up_space dehydrated unpinned placeholders");
    Ok(FreeUpSpaceResult { bytes_freed })
}

/// Unix reclaim path (macOS File Provider / Linux FUSE). Unchanged from the
/// original: the engine stores hydrated copies as separate cache files, so a
/// `remove_file` genuinely reclaims the bytes. The DB eviction flips rows to
/// cloud_only and the on-disk cache copies are unlinked.
#[cfg(not(target_os = "windows"))]
fn free_up_space_unix(
    db: &state_db::StateDb,
    sync_root: &std::path::Path,
    paths: Vec<String>,
) -> Result<FreeUpSpaceResult, String> {
    // Flip every unpinned local file to cloud_only in the DB. A budget of 0
    // means "evict everything unpinned."
    let now = now_unix_seconds();
    let evicted_ids = db
        .evict_unpinned_cache_until_under(0, now)
        .map_err(|e| format!("evict unpinned cache: {e}"))?;
    #[cfg(target_os = "linux")]
    remove_linux_freedesktop_thumbnails_for_evicted_files(db, sync_root, &evicted_ids);

    // Delete the actual cache files, bounded to paths under one of the allowed
    // engine cache roots so a bad cache_path can't delete outside the vault.
    // Sum the real file sizes we remove so `bytes_freed` is an exact count, not
    // an estimate. A missing file contributes 0 (already gone — nothing
    // reclaimed by us).
    let allowed_roots = free_up_space_allowed_roots(sync_root);
    let mut bytes_freed: u64 = 0;
    for path in paths {
        let path_buf = PathBuf::from(&path);
        let Ok(canonical) = path_buf.canonicalize() else {
            // Unresolvable path — never delete.
            continue;
        };
        if !allowed_roots.iter().any(|root| canonical.starts_with(root)) {
            // Outside every allowed cache root — never delete.
            continue;
        }
        let size = std::fs::metadata(&canonical).map(|m| m.len()).unwrap_or(0);
        match std::fs::remove_file(&canonical) {
            Ok(()) => bytes_freed = bytes_freed.saturating_add(size),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                tracing::warn!(error = %error, path = %canonical.display(), "free_up_space: could not remove cache file");
            }
        }
    }

    tracing::info!(bytes_freed, "free_up_space evicted unpinned cache");
    Ok(FreeUpSpaceResult { bytes_freed })
}

/// Version of the support-bundle shape. 2 = task 1685: `queue.last_error` is an
/// allow-list-filtered message plus `last_error_code` and `last_error_redactions`.
/// Paths and every file/folder name the state DB knows are removed. Residual: a
/// name the DB does not know that is a standard error word or digits only is
/// kept, and UUIDs and the URL scheme/host/port are kept on purpose.
const DIAGNOSTICS_FORMAT: u32 = 2;

#[tauri::command(async)]
fn export_diagnostics() -> Result<serde_json::Value, String> {
    let cfg = DesktopConfig::load()?;
    let bundle = match cfg.sync_root {
        None => export_diagnostics_from(None, Path::new(""))?,
        Some(root) => {
            let db_path = state_paths::beebeeb_state_dir()?.join(state_paths::STATE_DB_FILENAME);
            export_diagnostics_from(Some(&root), &db_path)?
        }
    };
    Ok(with_lifecycle_tail(
        bundle,
        lifecycle_log::tail(lifecycle_log::TAIL_LINES),
    ))
}

/// Spec 2026-10-06 §8: the support bundle carries the lifecycle log's last 200 lines (already
/// redacted when written). An additive key: `diagnostics_format` is unchanged. Empty on Windows
/// and Linux, where nothing writes the log.
fn with_lifecycle_tail(mut bundle: serde_json::Value, tail: Vec<String>) -> serde_json::Value {
    if let Some(map) = bundle.as_object_mut() {
        map.insert("lifecycle_log".to_string(), serde_json::json!(tail));
    }
    bundle
}

/// Builds the support-bundle JSON. Separate from the command so the real export
/// path (including redaction) is unit-testable against a temp state DB.
fn export_diagnostics_from(sync_root: Option<&Path>, db_path: &Path) -> Result<serde_json::Value, String> {
    let Some(sync_root) = sync_root else {
        return Ok(serde_json::json!({
            "diagnostics_format": DIAGNOSTICS_FORMAT,
            "sync_root_configured": false,
            "queue": serde_json::Value::Null,
        }));
    };
    if !db_path.exists() {
        return Ok(serde_json::json!({
            "diagnostics_format": DIAGNOSTICS_FORMAT,
            "sync_root_configured": true,
            "state_db_exists": false,
            "queue": serde_json::Value::Null,
        }));
    }
    let db = state_db::StateDb::open(db_path).map_err(|e| format!("open state.db: {e}"))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // The sync-root path (and so the account's home folder name) is a name too.
    let extra_paths = vec![sync_root.to_string_lossy().into_owned()];
    let queue = db
        .queue_diagnostics_with_paths(now, &extra_paths)
        .map_err(|e| format!("queue diagnostics: {e}"))?;
    Ok(serde_json::json!({
        "diagnostics_format": DIAGNOSTICS_FORMAT,
        "sync_root_configured": true,
        "state_db_exists": true,
        "queue": queue,
    }))
}

#[cfg(test)]
mod diagnostics_export_tests;

// ── IPC commands: conflict/version center ───────────────────────────────────

#[tauri::command]
fn list_version_conflict_center() -> Result<Vec<engine_bridge::VersionConflictEntry>, String> {
    let cfg = DesktopConfig::load()?;
    let Some(_) = cfg.sync_root else {
        return Ok(Vec::new());
    };
    let db_path = state_paths::beebeeb_state_dir()?.join(state_paths::STATE_DB_FILENAME);
    if !db_path.exists() {
        return Ok(Vec::new());
    }
    let db = state_db::StateDb::open(&db_path).map_err(|e| format!("open state.db: {e}"))?;
    engine_bridge::version_conflict_feed_from_db(&db).map_err(|e| e.to_string())
}

/// Task 1640 (review N9): dismiss one informational review entry (`kept:<id>`, a kept copy). The
/// copy itself stays where it is.
#[tauri::command]
fn dismiss_review_entry(entry_id: String) -> Result<(), String> {
    let Some(id) = entry_id.strip_prefix("kept:").and_then(|id| id.parse::<i64>().ok()) else {
        return Err("This entry can’t be dismissed.".to_string());
    };
    let db_path = state_paths::beebeeb_state_dir()?.join(state_paths::STATE_DB_FILENAME);
    let db = state_db::StateDb::open(&db_path).map_err(|e| format!("open state.db: {e}"))?;
    match db.dismiss_kept_copy(id) {
        Ok(true) => Ok(()),
        Ok(false) => Err("This entry can be dismissed once the file is closed and removed.".to_string()),
        Err(e) => Err(format!("dismiss: {e}")),
    }
}

fn file_versions_payload_for_frontend(body: serde_json::Value) -> Result<serde_json::Value, String> {
    let Some(versions) = body.get("versions") else {
        return Err("list_file_versions response missing versions array".to_string());
    };
    if !versions.is_array() {
        return Err("list_file_versions response versions field is not an array".to_string());
    }
    Ok(body)
}

fn queued_restore_version_response(
    file_id: &str,
    version_id: &str,
    op: &state_db::PendingOperation,
) -> serde_json::Value {
    serde_json::json!({
        "status": "queued",
        "file_id": file_id,
        "version_id": version_id,
        "operation_id": op.op_id,
        "operation": "restore_version",
    })
}

#[tauri::command]
async fn list_file_versions(state: State<'_, AppState>, file_id: String) -> Result<serde_json::Value, String> {
    session_command!(async {
        let acct = state.active_account()?;
        let (token, master_key) = {
            let guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
            match guard.as_ref() {
                Some(s) => (s.token.clone(), s.master_key),
                None => return Err("not signed in".into()),
            }
        };

        let api = api_client::ApiClient::new(runner::api_base_url(), token, master_key);
        let body = api.list_versions(&file_id).await.map_err(|e| e.to_string())?;
        file_versions_payload_for_frontend(body)
    })
}

#[tauri::command]
async fn restore_file_version(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    file_id: String,
    version_id: String,
) -> Result<serde_json::Value, String> {
    session_command!(async {
        let acct = state.active_account()?;
        let (token, master_key) = {
            let guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
            match guard.as_ref() {
                Some(s) => (s.token.clone(), s.master_key),
                None => return Err("not signed in".into()),
            }
        };

        let cfg = DesktopConfig::load()?;
        if cfg.sync_root.is_none() {
            return Err("no sync root configured".to_string());
        }
        let db_path = state_paths::beebeeb_state_dir()?.join(state_paths::STATE_DB_FILENAME);
        let db = std::sync::Arc::new(state_db::StateDb::open(&db_path).map_err(|e| format!("open state.db: {e}"))?);
        let api = std::sync::Arc::new(api_client::ApiClient::new(runner::api_base_url(), token, master_key));
        let bridge = engine_bridge::EngineBridge::new(db, api);
        let op = bridge
            .queue_restore_version(&file_id, &version_id, None)
            .map_err(|e| format!("queue restore version: {e}"))?;
        let body = queued_restore_version_response(&file_id, &version_id, &op);
        let _ = app.emit(
            "version-center-review",
            serde_json::json!({
                "file_id": file_id,
                "version_id": version_id,
                "kind": "restore",
                "operation_id": op.op_id,
            }),
        );
        Ok(body)
    })
}

// ── IPC commands: sync-root config ────────────────────────────────────────────

/// Read the persisted sync root from `~/.config/beebeeb/desktop.toml`.
/// Returns `None` if the user hasn't picked one yet (first launch).
#[tauri::command]
fn get_sync_root() -> Result<Option<String>, String> {
    let cfg = DesktopConfig::load()?;
    Ok(cfg.sync_root.map(|p| p.to_string_lossy().into_owned()))
}

/// Open the native folder picker so the user can choose a sync root.
/// Pre-points the dialog at `~/Beebeeb` (creates that directory first
/// if missing) so accepting the default is a single click.
///
/// On success, persists the chosen path to desktop.toml and returns
/// the absolute string. On user cancel, returns `Ok(None)`.
///
/// If a session is already in memory (user logged in before picking a
/// root), the engine runner is also spawned at this point.
///
/// Spec 030 §2 + §3 (lock acquired by the spawned engine).
#[tauri::command]
async fn pick_sync_root(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<Option<String>, String> {
    #[cfg(target_os = "macos")]
    {
        let _ = app;
        let _ = state;
        return Err(
            "macOS uses the system-managed Beebeeb Finder location; local state is stored privately.".to_string(),
        );
    }

    #[cfg(not(target_os = "macos"))]
    {
        let suggestion = config::default_sync_root_suggestion();
        // Best-effort create — the dialog still opens even if this fails,
        // pointed at the parent dir.
        let _ = config::ensure_directory(&suggestion);

        let dialog = app.dialog().clone();
        // Tauri's blocking_pick_folder is synchronous from the JS caller's
        // perspective but runs the OS dialog modally. We're already on a
        // tokio task because the IPC fn is async, so this is safe.
        let chosen: Option<PathBuf> = dialog
            .file()
            .set_directory(&suggestion)
            .set_title("Choose your Beebeeb sync folder")
            .blocking_pick_folder()
            .and_then(|fp| fp.into_path().ok());

        let Some(path) = chosen else {
            // User cancelled. Don't write anything.
            return Ok(None);
        };

        #[cfg(target_os = "windows")]
        let _transition = SESSION_TRANSITION.lock().await;
        #[cfg(target_os = "windows")]
        let generation = {
            windows_cf::ensure_reactivation_allowed()?;
            SESSION_COMMANDS.generation()?
        };

        if !path.is_absolute() {
            return Err(format!(
                "picker returned a non-absolute path: {} (please report)",
                path.display()
            ));
        }
        config::ensure_directory(&path)?;

        // Task 1889: one load-change-save under the config-write lock; `cfg` is what was saved.
        let cfg = DesktopConfig::update(|cfg| {
            cfg.sync_root = Some(path.clone());
            (true, cfg.clone())
        })?;

        tracing::info!(path = %path.display(), "sync root set");

        // If a session is already loaded, start syncing immediately.
        // (Common path: the browser-login handoff calls apply_session before
        //  pick_sync_root, so the engine couldn't start there; it starts
        //  here instead.)
        // session / engine / sync_paused are per-account (decision 0800).
        let acct = state.active_account()?;
        {
            // Rehydrate the persisted pause state before spawning.
            rehydrate_sync_paused(&acct.sync_paused, cfg.pause_sync);
            let mut engine_slot = acct.engine.lock().await;
            engine_start_refusal(&acct)?;
            if let Some(prev) = engine_slot.take() {
                // Task 1538 Codex P1 — see `start_engine_if_possible`'s
                // identical respawn guard.
                if !stop_engine_in_slot(&acct, prev).await.is_stopped() {
                    #[cfg(target_os = "windows")]
                    return Err(
                        "Could not stop the previous sync engine; retry locking before restarting sync.".into(),
                    );
                    tracing::warn!("previous engine did not confirm termination before respawning a new one");
                }
            }
            // Fix round 1: an unconfirmed stop of the old engine has set the flag; refuse the new one.
            engine_start_refusal(&acct)?;
            #[cfg(target_os = "windows")]
            {
                windows_cf::ensure_reactivation_allowed()?;
                SESSION_COMMANDS.validate_start(generation)?;
            }
            // No session means nothing to start (the folder is saved either way); the keys are read under
            // the slot, so a Lock that got there first leaves nothing to start with.
            spawn_bound_engine(app, &state, &acct, &mut engine_slot, path.clone())?;
        }

        Ok(Some(path.to_string_lossy().into_owned()))
    }
}

/// Suggested default path for the sync root, used by the WebView to
/// show "We'll create ~/Beebeeb if you accept" before opening the
/// native picker.
#[tauri::command]
fn default_sync_root() -> String {
    config::default_sync_root_suggestion().to_string_lossy().into_owned()
}

/// Read-only support snapshot. Tauri's embedded bundle marker is also used by
/// its updater; do not guess NSIS/MSI or AppImage from the host OS.
#[tauri::command]
fn desktop_capabilities(app: tauri::AppHandle) -> desktop_capabilities::DesktopCapabilities {
    use desktop_capabilities::InstallFormat;
    use tauri::utils::config::BundleType;
    let format = match tauri::utils::platform::bundle_type() {
        Some(BundleType::App) => InstallFormat::App,
        Some(BundleType::Nsis) => InstallFormat::Nsis,
        Some(BundleType::Msi) => InstallFormat::Msi,
        Some(BundleType::AppImage) => InstallFormat::Appimage,
        Some(BundleType::Deb) => InstallFormat::Deb,
        Some(BundleType::Rpm) => InstallFormat::Rpm,
        _ => InstallFormat::Unknown,
    };
    desktop_capabilities::snapshot(
        desktop_capabilities::host_os(),
        format,
        app.tray_by_id("tray").is_some(),
    )
}

#[tauri::command]
fn desktop_platform() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "macos"
    }
    #[cfg(target_os = "windows")]
    {
        "windows"
    }
    #[cfg(target_os = "linux")]
    {
        "linux"
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        "unknown"
    }
}

// ── IPC commands: Task 9 — settings page ──────────────────────────────────────

/// Read the current settings (bandwidth caps + notification toggles)
/// from `desktop.toml`. The on-disk `sync_root` is intentionally
/// elided from the response — see [`config::DesktopSettings`] for the
/// rationale.
///
/// Consumed by `repos/desktop/src/pages/Bandwidth.tsx` and
/// `repos/desktop/src/pages/Notifications.tsx` as their first read on
/// page mount. A missing file (no settings ever saved) returns the
/// `Default` values, matching the TS-side `DEFAULT_CONFIG`.
#[tauri::command]
fn get_desktop_config() -> Result<config::DesktopSettings, String> {
    let cfg = DesktopConfig::load()?;
    Ok(config::DesktopSettings::from(&cfg))
}

/// Persist a new settings struct, leaving `sync_root` (and any future
/// non-settings fields) untouched. Performs a load → merge → save as one
/// update under the config-write lock (task 1889), so a concurrent
/// `pick_sync_root` or any other writer cannot race away a change between
/// this load and save.
#[tauri::command]
async fn set_desktop_config(app: tauri::AppHandle, config: config::DesktopSettings) -> Result<(), String> {
    apply_desktop_settings(&app, config).await
}

/// A settings write. Every field but the pause is merged into the file in one update; a pause (the compact flyout's
/// Network page sends one) goes the way the menu's and the tray's go, `set_sync_paused`, so the file, the engine's
/// flag, the engine-status event and the menu's label agree (task 1683 slice 1, review round 2 P2-3). A write without
/// a pause touches neither the pause nor the label.
async fn apply_desktop_settings(
    app: &tauri::AppHandle<impl tauri::Runtime>,
    mut config: config::DesktopSettings,
) -> Result<(), String> {
    let pause = config.pause_sync.take();
    save_desktop_settings(config)?;
    if let Some(paused) = pause {
        set_sync_paused(app, paused).await?;
    }
    Ok(())
}

/// The write behind `set_desktop_config`: one update under the config-write lock (task 1889).
fn save_desktop_settings(config: config::DesktopSettings) -> Result<(), String> {
    DesktopConfig::update(|cfg| {
        cfg.apply_settings(config);
        (true, ())
    })
}

// ── IPC aliases for frontend compatibility ────────────────────────────────────

/// Alias so the frontend can call `desktop_config` (the name used in
/// `WindowsFirstRun.tsx` and the in-app settings view) in addition to the
/// canonical `get_desktop_config`. Both names are registered in
/// `generate_handler!`. Returns `None` when the config file doesn't
/// exist yet (first-run path), which the frontend uses as a signal that
/// sync-mode hasn't been persisted yet.
#[tauri::command]
fn desktop_config() -> Result<Option<config::DesktopSettings>, String> {
    let path = DesktopConfig::path()?;
    if !path.exists() {
        return Ok(None);
    }
    let cfg = DesktopConfig::load()?;
    Ok(Some(config::DesktopSettings::from(&cfg)))
}

/// Open (or focus) the main Beebeeb app window. This is the new primary
/// surface on Windows — the full sidebar + content-router shell
/// (`WindowsApp.tsx`, selected by `?window=main-app&platform=windows`) that
/// hosts the data views. The tray flyout's "Settings" button and the
/// onboarding "Open control center" button both route here; Settings remains
/// reachable from a nav item inside the app.
///
/// On non-Windows platforms there is no dedicated main-app window yet, so we
/// fall back to the compact app window — keeping a single, predictable
/// entry point for callers regardless of OS.
#[tauri::command]
fn show_main_app_window(app: tauri::AppHandle) -> Result<(), String> {
    show_main_app_window_impl(&app);
    Ok(())
}

/// Open (or focus) the main Beebeeb app window and select the consolidated
/// Settings section. Used by the Windows tray flyout's Settings gear.
#[tauri::command]
fn show_main_app_settings(app: tauri::AppHandle) -> Result<(), String> {
    show_main_app_window_with_nav(&app, Some("settings"));
    Ok(())
}

// ── IPC commands: Windows tray (WS1) ─────────────────────────────────────────

/// One row in the tray's recent-activity list.
#[derive(Debug, serde::Serialize)]
struct TrayActivityItem {
    id: String,
    name: String,
    status: String,
    icon: String,
    ok: Option<bool>,
    active: Option<bool>,
    progress: Option<u8>,
}

const TRAY_RECENT_ACTIVITY_LIMIT: usize = 50;

/// Basename of a stored `FileEntry.path`. Uses `std::path::Path::file_name`
/// so Windows cache paths stored with backslash separators yield the correct
/// basename (a naive `rsplit('/')` would return the whole `C:\…\foo.txt`
/// string on Windows). Falls back to the full path if it has no file-name
/// component.
fn entry_basename(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

fn tray_recent_activity_from_db(db: &state_db::StateDb) -> Vec<TrayActivityItem> {
    let mut items: Vec<TrayActivityItem> = Vec::new();

    // In-flight transfers first (downloading + uploading), newest first.
    for status in [state_db::FileStatus::Downloading, state_db::FileStatus::Uploading] {
        let label = if matches!(status, state_db::FileStatus::Downloading) {
            "Downloading…"
        } else {
            "Uploading…"
        };
        let Ok(entries) = db.list_by_status(status) else {
            continue;
        };
        let mut entries = entries;
        entries.sort_by(|a, b| b.modified_at.cmp(&a.modified_at).then_with(|| a.path.cmp(&b.path)));
        for entry in entries {
            items.push(TrayActivityItem {
                id: entry.file_id,
                name: entry_basename(&entry.path),
                status: label.to_string(),
                icon: "cloud".to_string(),
                ok: None,
                active: Some(true),
                progress: None,
            });
        }
    }

    if let Ok(events) = db.list_recent_local_activity(TRAY_RECENT_ACTIVITY_LIMIT) {
        for event in events.into_iter() {
            // Task 1640: a kept copy of changes not sent, beside a file deleted elsewhere.
            let (status, icon) = match event.event_type {
                state_db::LocalActivityKind::MovedToTrash => ("Moved to trash", "trash"),
                state_db::LocalActivityKind::KeptLocalCopy => ("Your changes, kept as a copy", "file"),
                state_db::LocalActivityKind::Restored => continue,
            };
            items.push(TrayActivityItem {
                id: event.file_id.unwrap_or_else(|| format!("activity-{}", event.id)),
                name: event.file_name,
                status: status.to_string(),
                icon: icon.to_string(),
                ok: Some(true),
                active: None,
                progress: None,
            });
        }
    }

    if let Ok(locals) = db.list_by_status(state_db::FileStatus::Local) {
        // Sort by modified_at descending — most recently synced first.
        let mut locals = locals;
        locals.sort_by(|a, b| b.modified_at.cmp(&a.modified_at).then_with(|| a.path.cmp(&b.path)));
        for entry in locals {
            items.push(TrayActivityItem {
                id: entry.file_id,
                name: entry_basename(&entry.path),
                status: "Synced".to_string(),
                icon: "file".to_string(),
                ok: Some(true),
                active: None,
                progress: None,
            });
        }
    }

    items.truncate(TRAY_RECENT_ACTIVITY_LIMIT);
    items
}

/// Return recent sync activity for the Windows tray flyout.
///
/// Derives activity from the state DB:
///   - Files currently downloading or uploading (active, with progress = None
///     because per-file byte progress isn't tracked at this layer)
///   - Recently-synced local files ordered by `modified_at` descending
///
/// Returns an EMPTY vec when there is no state DB (no sync root configured
/// or engine not yet started). Never fabricates rows.
#[tauri::command]
fn tray_recent_activity() -> Vec<TrayActivityItem> {
    let Ok(cfg) = DesktopConfig::load() else {
        return Vec::new();
    };
    let Some(_) = cfg.sync_root else {
        return Vec::new();
    };
    let Ok(state_dir) = state_paths::beebeeb_state_dir() else {
        return Vec::new();
    };
    let db_path = state_dir.join(state_paths::STATE_DB_FILENAME);
    if !db_path.exists() {
        return Vec::new();
    }
    let Ok(db) = state_db::StateDb::open(&db_path) else {
        return Vec::new();
    };

    tray_recent_activity_from_db(&db)
}

/// Pause the sync engine at runtime. Sets the shared `sync_paused` flag
/// so the runner's next tick no-ops. Persists `pause_sync = true` to
/// `desktop.toml` so the paused state survives a restart.
///
/// Emits an `engine-status` event with `state = "paused"` so the tray
/// tooltip and any open app window update immediately.
/// Task 1889 fix round 1 (M6): the pause reaches the in-memory flag in the same step that persists it. The flag is stored
/// while the config-write lock is held, after the write succeeded, so it can never say paused while the file says
/// unpaused or the other way round: a write that fails leaves both as they were, and a rehydration
/// ([`rehydrate_sync_paused`]) or a sync-root write that reads the pause waits for this one and stores what it wrote.
fn persist_sync_paused_at(path: &Path, flag: &AtomicBool, paused: bool) -> Result<(), String> {
    DesktopConfig::update_at_then(
        path,
        |cfg| {
            cfg.pause_sync = paused;
            (true, ())
        },
        |cfg| flag.store(cfg.pause_sync, Ordering::Relaxed),
    )
}

/// Stores the persisted pause state into the in-memory flag, under the config-write lock (see
/// [`persist_sync_paused_at`]). A restart that finds the file paused starts paused.
fn rehydrate_sync_paused_at(path: &Path, flag: &AtomicBool) -> Result<(), String> {
    DesktopConfig::update_at_then(
        path,
        |_| (false, ()),
        |cfg| flag.store(cfg.pause_sync, Ordering::Relaxed),
    )
}

/// [`rehydrate_sync_paused_at`] on the real config. An unreadable config stores `fallback`, which is what each caller
/// stored before when it could not read the file (the value it had already loaded, or "not paused").
fn rehydrate_sync_paused(flag: &AtomicBool, fallback: bool) {
    let stored = DesktopConfig::path().and_then(|path| rehydrate_sync_paused_at(&path, flag));
    if stored.is_err() {
        flag.store(fallback, Ordering::Relaxed);
    }
}

async fn set_sync_paused(app: &tauri::AppHandle<impl tauri::Runtime>, paused: bool) -> Result<(), String> {
    let state = app.state::<AppState>();
    let acct = state.active_account()?;
    // Persist so the paused state survives a restart.
    let persisted = persist_sync_paused_at(&DesktopConfig::path()?, &acct.sync_paused, paused);
    // Two-surface F3: the menu's "Pause Sync" / "Resume Sync" follows what the file now says, also after a failure.
    pause_state_changed(app);
    persisted?;
    // Notify the WebView + tray tooltip listener.
    let event_state = if paused { "paused" } else { "idle" };
    let _ = app.emit("engine-status", serde_json::json!({ "state": event_state }));
    tracing::info!(paused, "sync pause state changed");
    Ok(())
}

#[tauri::command]
async fn tray_pause_sync(app: tauri::AppHandle) -> Result<(), String> {
    set_sync_paused(&app, true).await
}

/// Resume the sync engine. Clears the `sync_paused` flag so the runner's
/// next tick proceeds normally. Persists `pause_sync = false` to
/// `desktop.toml`. Emits an `engine-status` event with `state = "idle"`.
#[tauri::command]
async fn tray_resume_sync(app: tauri::AppHandle) -> Result<(), String> {
    set_sync_paused(&app, false).await
}

/// What sync is doing, for Settings › Sync and the Windows tray popover (0.8.15, see [`sync_activity`]): the live pause
/// flag, the engine's last state, what is moving, the queue in counts, and the startup check while it runs. Counts
/// only; reading it changes nothing and needs no session, so it answers while the vault is locked too.
#[tauri::command]
async fn sync_activity_summary(
    state: State<'_, AppState>,
    runtime: State<'_, popover_data::PopoverRuntime>,
) -> Result<sync_activity::SyncActivitySummary, String> {
    let paused = state.active_account()?.sync_paused.load(Ordering::Relaxed);
    let engine = runtime.status.lock().map(|view| view.clone()).unwrap_or_default();
    let transfers_active = runtime.transfers.active().len();
    let queue = tokio::task::spawn_blocking(|| -> Result<Option<state_db::QueueHold>, String> {
        let Some(db) = DesktopConfig::load().and_then(|cfg| state_db_for_config(&cfg))? else {
            return Ok(None);
        };
        db.queue_hold_summary().map(Some).map_err(|e| format!("read the sync queue: {e}"))
    })
    .await
    .map_err(|e| format!("sync activity task failed: {e}"))??;
    Ok(sync_activity::summary(paused, &engine, transfers_active, queue))
}

/// The menu's Pause / Resume item: flips the pause the file holds. Returns which way it went (`true` = pausing) with the
/// result, so a failure can say which.
async fn toggle_sync_paused_from_menu(app: &tauri::AppHandle) -> (bool, Result<(), String>) {
    let pausing = !DesktopConfig::load().map(|cfg| cfg.pause_sync).unwrap_or(false);
    (pausing, set_sync_paused(app, pausing).await)
}

/// The title of the alert a failed Pause from the menu raises.
const MENU_PAUSE_FAILED_TITLE: &str = "Pause sync";
/// The title of the alert a failed Resume from the menu raises.
const MENU_RESUME_FAILED_TITLE: &str = "Resume sync";
/// What a failed Pause says. Honest about the state: a pause that cannot be written changes neither the file nor the
/// running engine (task 1889, M6), so sync carries on as it was. It never carries the error behind it (a path, an OS
/// message); the caller logs that.
const MENU_PAUSE_FAILED_SENTENCE: &str = "Beebeeb couldn’t pause syncing, so it carries on as before.";
/// What a failed Resume says: the pause stays.
const MENU_RESUME_FAILED_SENTENCE: &str = "Beebeeb couldn’t resume syncing, so it stays paused.";

/// What the menu's Pause / Resume shows the person when its work ended in `result` (task 1889, fix round 2 N2): an `Err`
/// is an alert with the title and the one sentence for the way it was going (`pausing`); success shows nothing. The menu
/// works with every window closed, so it has no window to put a toast in and the alert is native, like 1885's failed
/// "Open in Finder" and the Sign-out menu action's. On every platform: the native menu is the one live caller of the
/// pause commands, and a pause that does nothing must not look like one that worked.
fn menu_pause_failure_alert(result: &Result<(), String>, pausing: bool) -> Option<(&'static str, &'static str)> {
    match (result, pausing) {
        (Ok(()), _) => None,
        (Err(_), true) => Some((MENU_PAUSE_FAILED_TITLE, MENU_PAUSE_FAILED_SENTENCE)),
        (Err(_), false) => Some((MENU_RESUME_FAILED_TITLE, MENU_RESUME_FAILED_SENTENCE)),
    }
}

/// A failed Pause or Resume from the menu, shown to the person before the caller logs it (see
/// [`menu_pause_failure_alert`]).
fn show_menu_pause_failure(app: &tauri::AppHandle, pausing: bool, result: &Result<(), String>) {
    let Some((title, message)) = menu_pause_failure_alert(result, pausing) else {
        return;
    };
    app.dialog()
        .message(message)
        .title(title)
        .kind(tauri_plugin_dialog::MessageDialogKind::Error)
        .show(|_| {});
}

#[tauri::command]
async fn upload_files_to_sync_root(app: tauri::AppHandle) -> Result<usize, String> {
    upload_files_to_sync_root_impl(&app)
}

#[tauri::command]
fn create_folder_in_sync_root(app: tauri::AppHandle) -> Result<String, String> {
    let path = create_folder_in_sync_root_impl(&app)?;
    Ok(path.to_string_lossy().into_owned())
}

/// Result of `report_problem` for the Account page. The bundle is written
/// before the email draft is attempted, so a failed mail open is a partial
/// success, not a failed save (task 1685).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct ProblemReport {
    path: String,
    email_opened: bool,
}

/// `async` so Tauri runs it off the main thread: building the bundle reads every
/// known file name from the state DB and must not stall the window.
#[tauri::command(async)]
fn report_problem(app: tauri::AppHandle) -> Result<ProblemReport, String> {
    report_problem_impl(&app)
}

fn sync_root_for_menu_file_action() -> Result<PathBuf, String> {
    let root = DesktopConfig::load()
        .ok()
        .and_then(|cfg| cfg.sync_root)
        .unwrap_or_else(config::default_sync_root_suggestion);
    config::ensure_directory(&root)?;
    Ok(root)
}

/// Opens the Finder location after an upload or a new folder, WITHOUT making the caller wait (task 1885 fix round 1, m4).
/// The open can wait up to 3 s for the bridge gate and 7 s for LaunchServices. Its callers are the menu handler and a plain
/// command (the main thread), and a runtime worker, none of which may sit in that wait; and nobody needs its answer, so it
/// runs on the blocking pool and a failure is logged.
fn open_finder_location_in_background(sync_root: String) {
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(error) = open_finder_location_blocking(Some(sync_root)) {
            tracing::warn!(%error, "could not open the Beebeeb folder after the menu action");
        }
    });
}

fn upload_files_to_sync_root_impl(app: &tauri::AppHandle) -> Result<usize, String> {
    let sync_root = sync_root_for_menu_file_action()?;
    let selected = app
        .dialog()
        .file()
        .set_title("Upload files to Beebeeb")
        .blocking_pick_files()
        .unwrap_or_default();

    let mut copied = 0usize;
    for file in selected {
        let source = file
            .into_path()
            .map_err(|e| format!("resolve selected file path: {e}"))?;
        if !source.is_file() {
            continue;
        }
        let Some(name) = source.file_name().map(|name| name.to_string_lossy().into_owned()) else {
            continue;
        };
        let destination = unused_child_path(&sync_root, &name)?;
        std::fs::copy(&source, &destination).map_err(|e| {
            format!(
                "copy selected file into Beebeeb folder ({} -> {}): {e}",
                source.display(),
                destination.display()
            )
        })?;
        copied += 1;
    }

    if copied > 0 {
        let _ = app.emit("menu:files-added", serde_json::json!({ "count": copied }));
        open_finder_location_in_background(sync_root.to_string_lossy().into_owned());
    }

    Ok(copied)
}

fn create_folder_in_sync_root_impl(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let sync_root = sync_root_for_menu_file_action()?;
    let path = unused_child_path(&sync_root, "New folder")?;
    std::fs::create_dir(&path).map_err(|e| format!("create Beebeeb folder {}: {e}", path.display()))?;
    let _ = app.emit(
        "menu:folder-created",
        serde_json::json!({ "path": path.to_string_lossy() }),
    );
    if let Err(error) = app.opener().reveal_item_in_dir(&path) {
        tracing::warn!(%error, path = %path.display(), "could not reveal new folder; opening sync root instead");
        open_finder_location_in_background(sync_root.to_string_lossy().into_owned());
    }
    Ok(path)
}

fn diagnostics_bundle_path() -> Result<PathBuf, String> {
    let state_dir = state_paths::beebeeb_state_dir().or_else(|_| {
        DesktopConfig::path().map(|path| path.parent().map(Path::to_path_buf).unwrap_or_else(std::env::temp_dir))
    })?;
    let dir = state_dir.join("diagnostics");
    std::fs::create_dir_all(&dir).map_err(|e| format!("create diagnostics directory {}: {e}", dir.display()))?;
    Ok(dir.join(format!("beebeeb-diagnostics-{}.json", now_unix_seconds())))
}

/// Body of the support email draft. Names only the bundle's file name, never
/// its folder: the folder sits under the user's home directory (task 1685).
fn problem_report_email_body(bundle_path: &Path) -> String {
    let file_name = bundle_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "beebeeb-diagnostics.json".to_string());
    format!(
        "Describe the problem here.\n\nThe support bundle {file_name} was saved on this computer and shown in your file manager. Attach it to this email after you have read it."
    )
}

/// Turns the outcome of opening the mail draft into the report. Called after the
/// bundle has been written, so a mail error never hides the saved bundle.
fn problem_report_outcome<E: std::fmt::Display>(path: &Path, mail: Result<(), E>) -> ProblemReport {
    if let Err(error) = &mail {
        tracing::warn!(%error, "support bundle saved but the email draft could not be opened");
    }
    ProblemReport {
        path: path.to_string_lossy().into_owned(),
        email_opened: mail.is_ok(),
    }
}

fn report_problem_impl(app: &tauri::AppHandle) -> Result<ProblemReport, String> {
    let diagnostics = export_diagnostics()?;
    let path = diagnostics_bundle_path()?;
    let body = serde_json::to_vec_pretty(&diagnostics).map_err(|e| format!("serialize diagnostics: {e}"))?;
    std::fs::write(&path, body).map_err(|e| format!("write diagnostics bundle {}: {e}", path.display()))?;

    let _ = app.opener().reveal_item_in_dir(&path);
    let subject = urlencoding::encode("Beebeeb desktop problem report");
    let body_text = problem_report_email_body(&path);
    let body = urlencoding::encode(&body_text);
    let mail = app.opener().open_url(
        format!("mailto:support@beebeeb.io?subject={subject}&body={body}"),
        None::<&str>,
    );
    Ok(problem_report_outcome(&path, mail))
}

fn unused_child_path(parent: &Path, preferred_name: &str) -> Result<PathBuf, String> {
    let preferred = Path::new(preferred_name)
        .file_name()
        .ok_or_else(|| "destination name is empty".to_string())?
        .to_string_lossy()
        .into_owned();
    let first = parent.join(&preferred);
    if !first.exists() {
        return Ok(first);
    }

    let path = Path::new(&preferred);
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| preferred.clone());
    let extension = path.extension().map(|ext| ext.to_string_lossy().into_owned());

    for suffix in 2..10_000 {
        let name = match &extension {
            Some(ext) if !ext.is_empty() => format!("{stem} {suffix}.{ext}"),
            _ => format!("{stem} {suffix}"),
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(format!(
        "could not find an unused destination name for {} in {}",
        preferred,
        parent.display()
    ))
}

/// Persist the user's chosen sync mode from the Windows first-run wizard.
///
/// `mode` is one of `"everything"` | `"smart"` | `"custom"` | `"online_only"`.
/// The value is stored in `desktop.toml` under `sync_mode`. The runner
/// does not yet act on these modes (full smart-sync requires selective-sync
/// logic in the engine); storing it now means a future version can read it
/// without a migration step.
///
/// The frontend (`WindowsFirstRun.tsx`) uses the presence of a persisted
/// mode as the signal that the sync-mode step was completed; `desktop_config`
/// returns `None` when the file doesn't exist yet, triggering that step on
/// first run.
#[tauri::command]
fn set_sync_mode(mode: String) -> Result<(), String> {
    match mode.as_str() {
        "everything" | "smart" | "custom" | "online_only" => {}
        _ => return Err(format!("invalid sync mode: {mode}")),
    }
    DesktopConfig::update(|cfg| {
        cfg.sync_mode = Some(mode);
        (true, ())
    })
}

// ── IPC commands: Task 0090 — selective sync ──────────────────────────────────

/// Read the persisted list of top-level folder IDs the user has chosen
/// to keep cloud-only on this device. Returns an empty `Vec` (not
/// `Err`) when the key is missing from `desktop.toml`, which is the
/// common case on a fresh install.
///
/// The shape mirrors what `repos/desktop/src/pages/SelectiveSync.tsx`
/// renders: the page only needs the IDs, then cross-references them
/// against the folder list returned by [`list_vault_folders`] to mark
/// the matching checkboxes off.
#[tauri::command]
fn get_selective_sync() -> Result<Vec<String>, String> {
    let cfg = DesktopConfig::load()?;
    Ok(cfg.excluded_folder_ids.unwrap_or_default())
}

/// Persist a new list of excluded folder IDs to `desktop.toml` AND
/// reclaim disk space for any folder that just became excluded (wave-2
/// of task 0090 — the real fix the original no-op comment promised).
///
/// Flow:
///   1. Snapshot the OLD excluded list, then persist the NEW one (an
///      empty list normalises to `None` so the on-disk file stays free of
///      `excluded_folder_ids = []` clutter). Persisting first guarantees
///      the choice survives even if the dehydrate step below errors.
///   2. Diff: folder ids in NEW but not OLD are "newly excluded."
///   3. For each newly-excluded folder, resolve its subtree's files from
///      the local state DB and DEHYDRATE every file that is currently
///      hydrated (`status == Local`) AND not effectively Pinned — the
///      exact pattern [`free_up_space_windows`] uses. The file stays
///      cloud-only (we NEVER delete user data; dehydrate only reclaims the
///      local cache, and the file re-hydrates on demand). A pinned file in
///      an excluded folder is kept — the user explicitly asked for it
///      offline, and an explicit pin wins over an exclusion.
///   4. Newly-INCLUDED (un-excluded) folders need no immediate action —
///      on-demand hydration re-downloads them when next opened.
///
/// Best-effort + idempotent: a dehydrate failure on one file logs and
/// continues; the persisted list is unaffected. On non-Windows builds the
/// DB transition still runs (rows flip to cloud_only via
/// `mark_cloud_only_after_dehydrate`) but the platform dehydrate is a
/// no-op there — the macOS File Provider / Linux FUSE cache eviction is
/// the engine's existing job and is not in scope for this task.
///
/// The DB + dehydrate work runs on a blocking thread so a large excluded
/// subtree doesn't hold the IPC command thread on SQLite + CF syscalls.
#[tauri::command]
async fn set_selective_sync(excluded: Vec<String>) -> Result<(), String> {
    // 1. Snapshot old, persist new. Persist FIRST so the choice sticks
    //    regardless of the reclaim outcome.
    let old_excluded = DesktopConfig::update(|cfg| {
        let old_excluded = cfg.excluded_folder_ids.clone().unwrap_or_default();
        cfg.excluded_folder_ids = if excluded.is_empty() {
            None
        } else {
            Some(excluded.clone())
        };
        (true, old_excluded)
    })?;

    // 2. Diff for the subtrees we must reclaim now.
    let newly_excluded = newly_excluded_ids(&old_excluded, &excluded);
    if newly_excluded.is_empty() {
        return Ok(());
    }

    // 3. Reclaim on a blocking thread. Failures here never undo the persist.
    let join = tokio::task::spawn_blocking(move || dehydrate_excluded_subtrees_blocking(&newly_excluded));
    match join.await {
        Ok(Ok(freed)) => {
            tracing::info!(
                bytes_freed = freed,
                "set_selective_sync: dehydrated newly-excluded subtrees"
            );
        }
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "set_selective_sync: reclaim of excluded subtrees failed (list still persisted)");
        }
        Err(e) => {
            tracing::warn!(error = %e, "set_selective_sync: reclaim task join failed (list still persisted)");
        }
    }
    Ok(())
}

// ── IPC commands: Task 0797 — known-folder backup ("Manage backup") ───────────

/// One row in the "Manage backup" panel: a Windows known folder, its real
/// source path on this machine, whether the user has opted into backing it up,
/// and (when cheap to compute) a rough item count for the source.
///
/// Mirrors the selective-sync IPC shape (a flat DTO the TS side renders without
/// further lookups). `source_path` is `None` when the folder can't be resolved
/// on this OS — on non-Windows there ARE no Windows known folders, so every row
/// reports `source_path: None` + `enabled: false`, which the panel renders as
/// "not available on this platform".
#[derive(Debug, Clone, serde::Serialize)]
struct KnownFolderStatus {
    /// Stable key, e.g. `"documents"`. Matches `desktop.toml`.
    key: String,
    /// UI label / vault subfolder name, e.g. `"Documents"`.
    display_name: String,
    /// Absolute source path on this machine, or `None` if unresolved.
    source_path: Option<String>,
    /// `true` if this folder is in the persisted backup set.
    enabled: bool,
    /// Rough count of files in the source tree (top-level + nested, bounded), or
    /// `None` when the source can't be enumerated. Cheap-ish, best-effort.
    item_count: Option<u64>,
}

/// List the backupable known folders with their resolved source paths + current
/// enabled state. Drives the "Manage backup" panel in the Files dashboard.
///
/// On Windows: resolves each `FOLDERID_*` via `SHGetKnownFolderPath` and counts
/// items (bounded). On other platforms: returns the catalog with `source_path:
/// None` so the panel can render a "Windows only" state rather than 404-ing.
#[tauri::command]
fn get_known_folder_backup() -> Result<Vec<KnownFolderStatus>, String> {
    let cfg = DesktopConfig::load()?;
    let mut out = Vec::with_capacity(known_folder::KNOWN_FOLDERS.len());
    for kf in known_folder::KNOWN_FOLDERS {
        let enabled = cfg.known_folder_enabled(kf.key);

        #[cfg(target_os = "windows")]
        let (source_path, item_count) = match known_folder::resolve_known_folder(kf.key) {
            Some(p) => {
                let count = count_items_bounded(&p);
                (Some(p.to_string_lossy().into_owned()), count)
            }
            None => (None, None),
        };
        #[cfg(not(target_os = "windows"))]
        let (source_path, item_count): (Option<String>, Option<u64>) = (None, None);

        out.push(KnownFolderStatus {
            key: kf.key.to_string(),
            display_name: kf.display_name.to_string(),
            source_path,
            enabled,
            item_count,
        });
    }
    Ok(out)
}

/// Bounded, best-effort count of regular files under `root` (≤ this many before
/// we stop — the panel only needs an "about this many" signal, not an exact
/// total, and an exact recursive count of a huge Documents folder would block
/// the IPC thread). Windows-only (the only caller is gated).
#[cfg(target_os = "windows")]
fn count_items_bounded(root: &std::path::Path) -> Option<u64> {
    const CAP: u64 = 10_000;
    fn walk(dir: &std::path::Path, depth: usize, count: &mut u64) {
        if depth > 32 || *count >= CAP {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            if *count >= CAP {
                return;
            }
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                walk(&entry.path(), depth + 1, count);
            } else if ft.is_file() {
                *count += 1;
            }
        }
    }
    let mut count = 0u64;
    walk(root, 0, &mut count);
    Some(count)
}

/// Enable or disable known-folder backup for one folder key.
///
/// **Returns immediately (task 0811).** This IPC only flips `desktop.toml` and,
/// on disable, purges the folder's pending queue. It NEVER runs the mirror
/// inline — the engine daemon (runner tick) mirrors enabled folders gradually on
/// its own loop (`run_known_folder_mirror`), throttled so a first enable of a
/// large folder (e.g. Music, 5,344 files) can't thunder. Previously this command
/// `spawn_blocking`'d the WHOLE mirror and `await`ed it — once per checked key,
/// each re-mirroring ALL enabled folders — which hung the "Setting up…" UI and
/// crashed the app when the founder enabled five folders at once. Backup still
/// starts on its own shortly after enable (the next daemon pass), with no IPC
/// hang.
///
/// One-way mirror (source → `<sync_root>\Backup\<device>\<DisplayName>`); the
/// enumeration scan uploads what it copies in.
///
/// **Disable purges the queue (founder design, task 0811).** Disabling stops
/// FUTURE copies AND deletes every still-pending upload op tagged with this
/// folder's origin key, so uploads stop immediately and nothing resumes on the
/// next boot. It does NOT delete the already-backed-up vault copies (removing
/// them would delete the user's cloud content, which we never do silently).
///
/// Validates `key` against the catalog so a typo from the frontend can't write a
/// junk entry into the config.
#[tauri::command]
async fn set_known_folder_backup(key: String, enabled: bool) -> Result<(), String> {
    // Reject unknown keys — the catalog is the source of truth.
    if known_folder::known_folder_by_key(&key).is_none() {
        return Err(format!("unknown known-folder key: {key}"));
    }

    let cfg = DesktopConfig::update(|cfg| {
        cfg.set_known_folder(&key, enabled);
        (true, cfg.clone())
    })?;

    // On DISABLE, purge this folder's pending backup ops so they stop now and do
    // not resume on restart. Best-effort: a missing/unopenable state DB (no sync
    // root yet, or backup never ran) just means there is nothing to purge — the
    // persisted config flip already prevents future passes from re-enqueuing.
    if !enabled {
        match state_db_for_config(&cfg) {
            Ok(Some(db)) => match db.purge_backup_source_ops(&key) {
                // Zero-knowledge: the key is a stable catalog identifier (not user
                // content); logging the purged COUNT confirms the cancel worked.
                Ok(purged) => {
                    tracing::info!(folder = %key, purged, "known-folder backup disabled: purged pending upload ops");
                }
                Err(e) => {
                    tracing::warn!(folder = %key, error = %e, "known-folder backup disable: queue purge failed");
                }
            },
            Ok(None) => {
                tracing::debug!(folder = %key, "known-folder backup disable: no state DB yet; nothing to purge");
            }
            Err(e) => {
                tracing::warn!(folder = %key, error = %e, "known-folder backup disable: could not open state DB to purge");
            }
        }
    }

    // On ENABLE there is intentionally NOTHING to do here — the daemon's next
    // throttled mirror pass picks up the new key from `desktop.toml`. Returning
    // immediately is the whole point of this rewrite.
    //
    // NOTE (pin / available-offline): decision 0797 wants backed-up folders kept
    // available-offline (pinned). Pinning operates on Cloud Files PLACEHOLDERS,
    // which only exist AFTER the mirrored plain file has been uploaded and
    // converted to a placeholder — so there's nothing to pin at this point. The
    // honest path is to pin the vault subtree once it materialises as
    // placeholders (a follow-up that hooks the existing `set_recursive_pin` /
    // reconcile flow), not a no-op pin call here. TODO(0797-pin).

    Ok(())
}

// ── IPC commands: Task 0804 — known-folder backup first-run onboarding ─────────

/// Read whether the one-time known-folder backup prompt has already been shown.
///
/// The frontend uses this — together with an empty `known_folder_backup` — to
/// decide whether to AUTOMATICALLY surface the first-run "Back up these folders?"
/// prompt on app start. `false` (the fresh-install default, and the value for an
/// older `desktop.toml` missing the key) means "not yet seen → show it once."
///
/// Mirrors the other simple config reads (`get_selective_sync`): returns the
/// plain boolean, never an error for a missing file (a missing file loads as the
/// default config).
#[tauri::command]
fn get_known_folder_onboarding_seen() -> Result<bool, String> {
    let cfg = DesktopConfig::load()?;
    Ok(cfg.known_folder_onboarding_seen)
}

/// Mark the one-time known-folder backup prompt as shown so it never appears
/// automatically again. Called on BOTH paths of the prompt — "Back up these
/// folders" (after the folders are enabled + pinned) and "Not now" — because
/// either way the user has made their first-run choice and should not be
/// re-nagged.
///
/// Idempotent: setting an already-set flag is a no-op write. Load → set → save
/// so it leaves every other field (including `known_folder_backup`, which the
/// accept path writes via `set_known_folder_backup`) untouched.
#[tauri::command]
fn mark_known_folder_onboarding_seen() -> Result<(), String> {
    DesktopConfig::update(|cfg| {
        cfg.known_folder_onboarding_seen = true;
        (true, ())
    })
}

/// Whether the Beebeeb window's one-time hint on Home ("Beebeeb lives in the menu bar", two-surface spec §7.6) has been
/// dismissed. `false` for a fresh install and for a `desktop.toml` that has no such key.
#[tauri::command]
fn get_menu_bar_hint_seen() -> Result<bool, String> {
    Ok(DesktopConfig::load()?.menu_bar_hint_seen)
}

/// "Got it" on that hint: it is never shown again. Idempotent, and a write only when it changes something. One update, so
/// it lands next to any other save in flight (task 1889).
#[tauri::command]
fn mark_menu_bar_hint_seen() -> Result<(), String> {
    DesktopConfig::update(|cfg| {
        let changed = !cfg.menu_bar_hint_seen;
        cfg.menu_bar_hint_seen = true;
        (changed, ())
    })
}

/// Resolve and dehydrate the local, unpinned files under each newly-excluded
/// folder. Returns total bytes reclaimed. Shared by Windows (real CF
/// dehydrate) and non-Windows (DB-only flip) via the inner cfg blocks —
/// mirrors [`free_up_space_blocking`]. Best-effort: a missing sync root /
/// DB is `Ok(0)`, a per-file dehydrate failure logs and continues.
fn dehydrate_excluded_subtrees_blocking(newly_excluded: &[String]) -> Result<u64, String> {
    let cfg = DesktopConfig::load()?;
    #[cfg(target_os = "windows")]
    let Some(sync_root) = cfg.sync_root.clone() else {
        return Ok(0);
    };
    #[cfg(not(target_os = "windows"))]
    if cfg.sync_root.is_none() {
        return Ok(0);
    }
    let db_path = state_paths::beebeeb_state_dir()?.join(state_paths::STATE_DB_FILENAME);
    if !db_path.exists() {
        return Ok(0);
    }
    let db = state_db::StateDb::open(&db_path).map_err(|e| format!("open state.db: {e}"))?;

    // Flat (id, parent_id, is_folder) view → resolve the union of subtree files.
    let entries = db.list_files().map_err(|e| format!("list files: {e}"))?;
    let flat: Vec<(String, Option<String>, bool)> = entries
        .iter()
        .map(|e| {
            (
                e.file_id.clone(),
                e.parent_id.clone(),
                e.item_kind == state_db::ItemKind::Folder,
            )
        })
        .collect();
    let file_ids = subtree_file_ids(&flat, newly_excluded);
    if file_ids.is_empty() {
        return Ok(0);
    }

    #[cfg(target_os = "windows")]
    {
        dehydrate_excluded_subtrees_windows(&db, &sync_root, &entries, &file_ids)
    }
    #[cfg(not(target_os = "windows"))]
    {
        // Non-Windows: no in-process CF dehydrate. Flip the local+unpinned
        // rows to cloud_only so the DB reflects the exclusion; the engine's
        // own cache eviction reclaims the bytes on its schedule. Pinned rows
        // are excluded by the predicate inside `mark_cloud_only_after_dehydrate`.
        let local_unpinned: Vec<String> = entries
            .iter()
            .filter(|e| file_ids.iter().any(|f| f == &e.file_id))
            .filter(|e| e.status == state_db::FileStatus::Local)
            .filter(|e| {
                db.get_file_contract_state(&e.file_id)
                    .ok()
                    .flatten()
                    .map(|c| c.effective_pin_state() != state_db::PinState::Pinned)
                    .unwrap_or(true)
            })
            .map(|e| e.file_id.clone())
            .collect();
        let now = now_unix_seconds();
        if let Err(e) = db.mark_cloud_only_after_dehydrate(&local_unpinned, now) {
            tracing::warn!(error = %e, "set_selective_sync: could not flip excluded files cloud_only");
        }
        Ok(0)
    }
}

/// Windows reclaim path for newly-excluded subtrees. Mirrors
/// [`free_up_space_windows`] exactly: reconstruct each file's placeholder
/// path under the sync root, path-safety-guard it, skip effectively-pinned
/// files (with a TOCTOU re-check immediately before the destructive
/// dehydrate), `CfDehydratePlaceholder`, then mark cloud_only ONLY for the
/// files that actually dehydrated.
#[cfg(target_os = "windows")]
fn dehydrate_excluded_subtrees_windows(
    db: &state_db::StateDb,
    sync_root: &std::path::Path,
    entries: &[state_db::FileEntry],
    file_ids: &[String],
) -> Result<u64, String> {
    use std::collections::HashSet;
    let target: HashSet<&str> = file_ids.iter().map(String::as_str).collect();
    let allowed_roots = free_up_space_allowed_roots(sync_root);
    let mut bytes_freed: u64 = 0;
    let mut dehydrated_ids: Vec<String> = Vec::new();

    for entry in entries {
        if !target.contains(entry.file_id.as_str()) {
            continue;
        }
        // Only hydrated files have bytes to reclaim.
        if entry.status != state_db::FileStatus::Local {
            continue;
        }
        // Skip effectively-pinned files: an explicit pin wins over the exclusion.
        if let Ok(Some(contract)) = db.get_file_contract_state(&entry.file_id) {
            if contract.effective_pin_state() == state_db::PinState::Pinned {
                tracing::debug!(file_id = %entry.file_id, "set_selective_sync: file pinned; keeping local despite exclusion");
                continue;
            }
        }
        let rel = entry.path.trim_matches('/');
        if rel.is_empty() {
            continue;
        }
        let local_path = sync_root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        let Ok(canonical) = local_path.canonicalize() else {
            continue;
        };
        if !allowed_roots.iter().any(|root| canonical.starts_with(root)) {
            continue;
        }
        // Task 1640: an unsent change stays on disk; see `free_up_space_windows`.
        if !crate::windows_cf::placeholders::local_bytes_agreed(db, &entry.file_id, &canonical) {
            tracing::warn!(file_id = %entry.file_id, "set_selective_sync: the file holds a change that is not sent; keeping it local");
            continue;
        }
        match crate::windows_cf::placeholders::dehydrate_placeholder(&canonical) {
            Ok(freed) => {
                bytes_freed = bytes_freed.saturating_add(freed);
                dehydrated_ids.push(entry.file_id.clone());
            }
            Err(error) => {
                tracing::warn!(file_id = %entry.file_id, error = %error, "set_selective_sync: CfDehydratePlaceholder failed; keeping file local");
            }
        }
    }

    let now = now_unix_seconds();
    if let Err(e) = db.mark_cloud_only_after_dehydrate(&dehydrated_ids, now) {
        tracing::warn!(error = %e, "set_selective_sync: could not mark dehydrated files cloud_only");
    }
    Ok(bytes_freed)
}

/// One node in the SelectiveSync folder tree (task 0090 → wave-2 nesting).
///
/// Each node is a FOLDER the user can toggle. `children` holds nested
/// sub-folders (built from `parent_id` linkage), and the byte/count
/// fields are AGGREGATES over the whole subtree's *files* (folders carry
/// no bytes of their own), so a row can render "12 files · 4.2 GB · 1.1 GB
/// on disk" without the TS side walking the tree.
///
/// JSON shape serialized to the frontend (serde default camel? — NO,
/// serde keeps snake_case here; the existing `is_folder` field already
/// ships snake_case and the TS view reads `is_folder`, so we stay
/// consistent):
/// ```json
/// {
///   "id": "uuid",
///   "name": "Documents",
///   "is_folder": true,
///   "excluded": false,
///   "size_bytes": 4509715660,
///   "file_count": 12,
///   "on_disk_bytes": 1181116006,
///   "children": [ { ...same shape... } ]
/// }
/// ```
/// - `id` / `name` / `is_folder`: as before. `is_folder` is always `true`
///   for a node we emit (we only surface folders as toggle rows); files
///   are folded into the aggregates, never emitted as their own node.
/// - `excluded`: `true` if this folder id ∈ `get_selective_sync()`.
/// - `size_bytes`: sum of `size_bytes` of every file at or below this
///   folder (logical plaintext bytes).
/// - `file_count`: number of files at or below this folder.
/// - `on_disk_bytes`: sum of `size_bytes` of files in the subtree that are
///   currently hydrated locally (`status == Local`) — drives the
///   "X on disk / Y online-only" footer.
/// - `children`: nested sub-folders, sorted by name (case-insensitive)
///   then id for a stable order.
///
/// `name` is taken from the state_db `path` leaf segment (the
/// already-resolved, decrypted relative path the engine wrote when it
/// synced the folder), falling back to a stable id-derived label
/// (`"Folder 7f3a1c…"`) when the path is empty. When we fall back to the
/// top-level API path (empty DB), names are decrypted via
/// [`owner_keys::display_name`] as before.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
struct VaultItem {
    id: String,
    name: String,
    is_folder: bool,
    excluded: bool,
    size_bytes: i64,
    file_count: i64,
    on_disk_bytes: i64,
    children: Vec<VaultItem>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
struct DesktopSharedRoot {
    id: String,
    name: String,
    owner_email: Option<String>,
    permission: String,
    kind: String,
    finder_path: Option<String>,
}

/// Lightweight, decode-free row used by the pure tree-builder. Decoupling
/// from `state_db::FileEntry` keeps [`build_vault_tree`] trivially unit
/// testable without constructing full DB rows.
#[derive(Debug, Clone)]
struct VaultEntryRow {
    id: String,
    parent_id: Option<String>,
    is_folder: bool,
    /// Display label (folder leaf name); ignored for files.
    name: String,
    /// Logical plaintext size in bytes (files only; folders contribute 0).
    size_bytes: i64,
    /// Whether this file is currently hydrated on disk (`status == Local`).
    /// Always `false` for folders.
    on_disk: bool,
    /// The row's path. Rows of the same kind at one path are one entry in the tree (0.8.15, [`one_entry_per_path`]).
    path: String,
    /// Whether the server knows the row (`current_version` or `remote_updated_at` set). A folder this PC created and
    /// never sent has neither.
    server_known: bool,
    /// Whether the row's status is `local`, the status a folder this PC created and never sent keeps.
    local_status: bool,
}

/// Mutable accumulator for one folder while we aggregate the subtree.
struct FolderAgg {
    id: String,
    name: String,
    parent_id: Option<String>,
    children: Vec<String>,
    /// Files DIRECTLY in this folder (not descendants).
    direct_size: i64,
    direct_count: i64,
    direct_on_disk: i64,
}

/// Derive a folder's display leaf name from its server-relative `path`.
/// `"/Docs/Taxes"` → `"Taxes"`, `"Photos"` → `"Photos"`, `""` → `None`.
fn folder_leaf_name(path: &str) -> Option<String> {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    let leaf = trimmed.rsplit('/').next().unwrap_or(trimmed);
    if leaf.is_empty() { None } else { Some(leaf.to_string()) }
}

/// Stable id-derived fallback label, mirroring the existing top-level path.
fn folder_fallback_label(id: &str) -> String {
    let short = id.get(..8).unwrap_or(id);
    format!("Folder {short}…")
}

fn shared_permission_label(bits: i64) -> &'static str {
    if bits & state_db::PERMISSION_OWNER != 0 {
        "admin"
    } else if bits & state_db::PERMISSION_WRITE != 0 {
        "write"
    } else {
        "read"
    }
}

fn local_path_under_sync_root(sync_root: &Path, rel_path: &str) -> Option<String> {
    let rel = rel_path.trim_matches('/');
    if rel.is_empty()
        || rel
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return None;
    }
    Some(
        sync_root
            .join(rel.replace('/', std::path::MAIN_SEPARATOR_STR))
            .to_string_lossy()
            .into_owned(),
    )
}

fn shared_roots_from_db(db: &state_db::StateDb, sync_root: Option<&Path>) -> Result<Vec<DesktopSharedRoot>, String> {
    let contracts = db
        .list_contract_states_by_namespace(state_db::Namespace::SharedWithMe)
        .map_err(|e| format!("list shared roots: {e}"))?;
    let mut roots = Vec::new();

    for contract in contracts.into_iter().filter(|contract| contract.parent_id.is_none()) {
        let Some(entry) = db
            .get_file(&contract.file_id)
            .map_err(|e| format!("read shared root row: {e}"))?
        else {
            continue;
        };
        roots.push(DesktopSharedRoot {
            id: entry.file_id.clone(),
            name: folder_leaf_name(&entry.path).unwrap_or_else(|| folder_fallback_label(&entry.file_id)),
            owner_email: contract.owner_email,
            permission: shared_permission_label(contract.permission_bits).to_string(),
            kind: match contract.item_kind {
                state_db::ItemKind::Folder => "folder".to_string(),
                state_db::ItemKind::File => "file".to_string(),
            },
            finder_path: sync_root.and_then(|root| local_path_under_sync_root(root, &entry.path)),
        });
    }

    Ok(roots)
}

#[tauri::command]
fn list_shared_roots() -> Result<Vec<DesktopSharedRoot>, String> {
    let cfg = DesktopConfig::load()?;
    let sync_root = cfg.sync_root.clone();
    let Some(db) = state_db_for_config(&cfg)? else {
        return Ok(Vec::new());
    };
    shared_roots_from_db(&db, sync_root.as_deref())
}

/// One state DB row as the tree builder reads it.
fn vault_entry_row(entry: &state_db::TreeRow) -> VaultEntryRow {
    VaultEntryRow {
        id: entry.file_id.clone(),
        parent_id: entry.parent_id.clone(),
        is_folder: entry.item_kind == state_db::ItemKind::Folder,
        name: folder_leaf_name(&entry.path).unwrap_or_else(|| folder_fallback_label(&entry.file_id)),
        size_bytes: entry.size_bytes,
        on_disk: entry.status == state_db::FileStatus::Local,
        path: entry.path.clone(),
        server_known: entry.current_version > 0 || entry.remote_updated_at > 0,
        local_status: entry.status == state_db::FileStatus::Local,
    }
}

/// One entry per path in the Selective sync tree (0.8.15). Display only.
///
/// On Guus's PC 2,828 folder paths had two rows: a folder this PC created on 4 Oct whose `create_folder` never landed
/// (`local`, no server version, a locally minted id, its operation paused) and the server's folder at the same path,
/// which arrived later with a snapshot. The tree listed both: the same folder twice, once with 0 B and 0 files.
///
/// Here a row the server does not know folds into the row that stays at its path: the server's (a row the server
/// knows, then one whose status is not `local`, then the smallest id, so the choice is stable). The folded row is not
/// shown, its children hang under the row that stays, and nothing is counted twice. Two rows the server knows are two
/// server items, and both stay. Rows with an empty path are left alone.
///
/// The state DB and the queue are untouched: nothing is deleted, re-parented on disk or resumed. 0.8.16 repairs the data.
fn one_entry_per_path(rows: &[VaultEntryRow]) -> Vec<VaultEntryRow> {
    use std::cmp::Reverse;
    use std::collections::HashMap;

    let mut at_path: HashMap<(bool, &str), Vec<&VaultEntryRow>> = HashMap::new();
    for row in rows {
        let path = row.path.trim_matches('/');
        if !path.is_empty() {
            at_path.entry((row.is_folder, path)).or_default().push(row);
        }
    }
    // Folded row id -> the id of the row that stays at its path. A row that stays never folds, so one lookup resolves.
    let mut folded_into: HashMap<&str, &str> = HashMap::new();
    for same_path in at_path.values().filter(|rows| rows.len() > 1) {
        let Some(stays) = same_path
            .iter()
            .max_by_key(|row| (row.server_known, !row.local_status, Reverse(row.id.as_str())))
        else {
            continue;
        };
        for row in same_path {
            if row.id != stays.id && !row.server_known {
                folded_into.insert(row.id.as_str(), stays.id.as_str());
            }
        }
    }
    rows.iter()
        .filter(|row| !folded_into.contains_key(row.id.as_str()))
        .map(|row| {
            let mut row = row.clone();
            if let Some(stays) = row.parent_id.as_deref().and_then(|parent| folded_into.get(parent)) {
                row.parent_id = Some((*stays).to_string());
            }
            row
        })
        .collect()
}

/// Build the nested SelectiveSync folder tree from a flat set of local
/// rows (already nested via `parent_id`, from task 0784).
///
/// Pure + deterministic: no DB, no network, no clock — so the unit tests
/// can drive it directly. Algorithm:
///   1. Index every FOLDER row by id into a `FolderAgg` (children linked
///      by `parent_id`); roots are folders whose `parent_id` is `None`
///      OR points at a non-folder / unknown id (defensive: a dangling
///      parent must still surface its folder as a root, never drop it).
///   2. Fold every FILE row into its parent folder's *direct* totals
///      (size / count / on-disk). A file whose parent is unknown is
///      dropped from the tree's aggregates (it has no folder to attach
///      to) — acceptable for a folder-picker view.
///   3. Recursively materialize `VaultItem`s, summing each folder's
///      direct totals with its descendants' aggregates. Cycle-safe via a
///      `visiting` guard (a corrupt parent cycle can't infinite-loop).
///   4. `excluded` is set per-node from the `excluded` id set.
///
/// Children are sorted by lowercased name then id for a stable UI order.
///
/// First, one entry per path: a folder this PC created and never sent folds into the server's at its path (0.8.15,
/// [`one_entry_per_path`]).
fn build_vault_tree(rows: &[VaultEntryRow], excluded: &std::collections::HashSet<String>) -> Vec<VaultItem> {
    build_vault_tree_of_rows(&one_entry_per_path(rows), excluded)
}

/// [`build_vault_tree`] over the rows as they are, with no merge: what 0.8.14 showed.
fn build_vault_tree_of_rows(rows: &[VaultEntryRow], excluded: &std::collections::HashSet<String>) -> Vec<VaultItem> {
    use std::collections::HashMap;

    // 1. Index folders.
    let mut folders: HashMap<String, FolderAgg> = HashMap::new();
    for row in rows {
        if row.is_folder {
            folders.entry(row.id.clone()).or_insert_with(|| FolderAgg {
                id: row.id.clone(),
                name: row.name.clone(),
                parent_id: row.parent_id.clone(),
                children: Vec::new(),
                direct_size: 0,
                direct_count: 0,
                direct_on_disk: 0,
            });
        }
    }

    // Link folder → child-folder. A folder whose parent is not a known
    // folder becomes a root (collected below).
    let folder_ids: std::collections::HashSet<String> = folders.keys().cloned().collect();
    let mut roots: Vec<String> = Vec::new();
    let child_links: Vec<(String, String)> = folders
        .values()
        .filter_map(|f| match &f.parent_id {
            Some(p) if folder_ids.contains(p) => Some((p.clone(), f.id.clone())),
            _ => None,
        })
        .collect();
    for (parent, child) in child_links {
        if let Some(p) = folders.get_mut(&parent) {
            p.children.push(child);
        }
    }
    for f in folders.values() {
        let is_root = match &f.parent_id {
            None => true,
            Some(p) => !folder_ids.contains(p),
        };
        if is_root {
            roots.push(f.id.clone());
        }
    }

    // 2. Fold files into their parent folder's direct totals.
    for row in rows {
        if row.is_folder {
            continue;
        }
        if let Some(parent) = row.parent_id.as_ref()
            && let Some(folder) = folders.get_mut(parent)
        {
            folder.direct_size = folder.direct_size.saturating_add(row.size_bytes.max(0));
            folder.direct_count = folder.direct_count.saturating_add(1);
            if row.on_disk {
                folder.direct_on_disk = folder.direct_on_disk.saturating_add(row.size_bytes.max(0));
            }
        }
    }

    // 3 + 4. Materialize recursively with a cycle guard.
    let mut visiting: std::collections::HashSet<String> = std::collections::HashSet::new();
    roots.sort_by_key(|a| folder_sort_key(&folders, a));
    roots
        .iter()
        .filter_map(|id| materialize_folder(id, &folders, excluded, &mut visiting))
        .collect()
}

/// Sort key for a folder id: (lowercased name, id). Stable + case-insensitive.
fn folder_sort_key(folders: &std::collections::HashMap<String, FolderAgg>, id: &str) -> (String, String) {
    match folders.get(id) {
        Some(f) => (f.name.to_lowercase(), f.id.clone()),
        None => (String::new(), id.to_string()),
    }
}

/// Recursively build a `VaultItem`, summing direct totals with descendants.
/// Returns `None` only on a cycle re-entry (defensive against corrupt data).
fn materialize_folder(
    id: &str,
    folders: &std::collections::HashMap<String, FolderAgg>,
    excluded: &std::collections::HashSet<String>,
    visiting: &mut std::collections::HashSet<String>,
) -> Option<VaultItem> {
    let folder = folders.get(id)?;
    if !visiting.insert(id.to_string()) {
        // Already on the current path — a parent cycle. Stop here.
        return None;
    }

    let mut child_ids = folder.children.clone();
    child_ids.sort_by_key(|a| folder_sort_key(folders, a));
    let children: Vec<VaultItem> = child_ids
        .iter()
        .filter_map(|cid| materialize_folder(cid, folders, excluded, visiting))
        .collect();

    let mut size_bytes = folder.direct_size;
    let mut file_count = folder.direct_count;
    let mut on_disk_bytes = folder.direct_on_disk;
    for child in &children {
        size_bytes = size_bytes.saturating_add(child.size_bytes);
        file_count = file_count.saturating_add(child.file_count);
        on_disk_bytes = on_disk_bytes.saturating_add(child.on_disk_bytes);
    }

    visiting.remove(id);

    Some(VaultItem {
        id: folder.id.clone(),
        name: folder.name.clone(),
        is_folder: true,
        excluded: excluded.contains(&folder.id),
        size_bytes,
        file_count,
        on_disk_bytes,
        children,
    })
}

/// Folder ids that are present in `new_excluded` but NOT in `old_excluded`.
/// These are the subtrees that must be dehydrated on this `set_selective_sync`.
/// Order-stable on `new_excluded`'s order; de-duplicated.
fn newly_excluded_ids(old_excluded: &[String], new_excluded: &[String]) -> Vec<String> {
    let old: std::collections::HashSet<&str> = old_excluded.iter().map(String::as_str).collect();
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    new_excluded
        .iter()
        .filter(|id| !old.contains(id.as_str()) && seen.insert(id.as_str()))
        .cloned()
        .collect()
}

/// Resolve the FILE ids that live at or below any of `root_folder_ids`,
/// from a flat (id, parent_id, is_folder) view of the vault. Pure mirror
/// of the recursive-CTE descent in [`state_db::StateDb::set_recursive_pin`]
/// so the diff/subtree logic is unit-testable without SQLite.
///
/// Folders are descended into but never returned (we dehydrate files, not
/// folders). Cycle-safe. The result is de-duplicated even when the
/// excluded roots overlap (one folder nested under another).
fn subtree_file_ids(rows: &[(String, Option<String>, bool)], root_folder_ids: &[String]) -> Vec<String> {
    use std::collections::{HashMap, HashSet, VecDeque};

    // children index: parent_id → [child rows]
    let mut children: HashMap<&str, Vec<&(String, Option<String>, bool)>> = HashMap::new();
    for row in rows {
        if let Some(parent) = row.1.as_deref() {
            children.entry(parent).or_default().push(row);
        }
    }

    let mut visited: HashSet<&str> = HashSet::new();
    let mut out_files: Vec<String> = Vec::new();
    let mut out_seen: HashSet<&str> = HashSet::new();
    let mut queue: VecDeque<&str> = VecDeque::new();

    for root in root_folder_ids {
        queue.push_back(root.as_str());
    }
    while let Some(id) = queue.pop_front() {
        if !visited.insert(id) {
            continue;
        }
        if let Some(kids) = children.get(id) {
            for kid in kids {
                if kid.2 {
                    // folder — descend
                    queue.push_back(kid.0.as_str());
                } else if out_seen.insert(kid.0.as_str()) {
                    out_files.push(kid.0.clone());
                }
            }
        }
    }
    out_files
}

/// Try to decrypt a single file/folder's `name_encrypted` blob using
/// the master key. Returns `None` on any failure — caller falls back
/// to a stable id-derived label. We deliberately swallow errors here
/// because the SelectiveSync page treats these as best-effort: a
/// folder created by an older client with a missing/garbled blob, or
/// an entry encrypted under a different key, should still appear in
/// the list (with a fallback name) rather than abort the whole call.
///
/// Routes through core's resolver (`beebeeb_core::owned_keys::resolve_owned_key`),
/// the single decision path all clients share. The served id class decides
/// (task 2275, L12): an owner id's key is derived and must authenticate the
/// name, and any served attribution or envelope is ignored; a foreign id opens
/// only through its L8 envelope. Unlike the previous `decrypt_name` call this
/// covers the string, binary and read-only uppercase-repair conventions and
/// never treats a foreign row as an owner row. We deliberately swallow the
/// error to `None` so the SelectiveSync row falls back to a stable id label.
/// Build the nested SelectiveSync folder tree.
///
/// Wave-2 change (task 0090 backing): the tree is built from the LOCAL
/// state DB (`StateDb::list_files()`), which already carries the full
/// nested set (`parent_id` + `item_kind`, from task 0784) the engine has
/// synced. We deliberately do NOT recurse the API folder-by-folder — that
/// hammered the server and tripped the 429 rate limit (the 0789 problem).
/// One local SQLite read replaces N network round-trips.
///
/// Each returned [`VaultItem`] is a folder the user can toggle, with its
/// subtree's file size / count / on-disk bytes aggregated up (see the
/// struct doc for the exact JSON shape).
///
/// Returns an empty `Vec` — not `Err` — when:
///   - the user is not signed in (no in-memory session)
///   - no sync root / state DB exists yet (nothing synced)
///
/// Fallback: if the user IS signed in but the local DB is missing or has
/// no folders yet (fresh install, first sync not finished), we fall back
/// to the OLD top-level API path (`api.list_files(None)`, folders only) so
/// the picker isn't empty during the first-sync window. That fallback is
/// FLAT (no nesting, zeroed aggregates) — it is a stopgap, and the tree
/// fills in once the engine has populated state.db. An API failure in the
/// fallback still surfaces as an empty list, not an error banner.
#[tauri::command]
async fn list_vault_folders(state: State<'_, AppState>) -> Result<Vec<VaultItem>, String> {
    session_command!(async {
        // Lift the session pieces we need without holding the mutex over any
        // async API call.
        let acct = state.active_account()?;
        let creds = {
            let guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
            guard.as_ref().map(|s| (s.token.clone(), s.master_key))
        };
        let Some((token, master_key)) = creds else {
            // Not signed in — surface as empty rather than an error.
            return Ok(Vec::new());
        };

        let excluded: std::collections::HashSet<String> = DesktopConfig::load()
            .ok()
            .and_then(|c| c.excluded_folder_ids)
            .unwrap_or_default()
            .into_iter()
            .collect();

        // Primary path: build from the local, already-nested state DB.
        if let Ok(Some(db)) = DesktopConfig::load().and_then(|cfg| state_db_for_config(&cfg))
            && let Ok(entries) = db.list_tree_rows()
        {
            let rows: Vec<VaultEntryRow> = entries.iter().map(vault_entry_row).collect();
            let tree = build_vault_tree(&rows, &excluded);
            if !tree.is_empty() {
                return Ok(tree);
            }
            // DB present but no folders yet — fall through to the API stopgap.
        }

        // Fallback: top-level API folders, FLAT, zeroed aggregates. Only used
        // until the first sync populates the local DB.
        let api = api_client::ApiClient::new(runner::api_base_url(), token, master_key);
        let raw = match api.list_files(None).await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "list_vault_folders: list_files fallback failed");
                return Ok(Vec::new());
            }
        };

        let mut out: Vec<VaultItem> = Vec::with_capacity(raw.len());
        for f in &raw {
            let is_folder = f.get("is_folder").and_then(|v| v.as_bool()).unwrap_or(false);
            if !is_folder {
                continue;
            }
            let Some(id) = f.get("id").and_then(|v| v.as_str()) else {
                continue;
            };

            // Display name: prefer the decrypted `name_encrypted` blob
            // (the canonical zero-knowledge path), then fall back to a
            // server-provided plaintext `path` if present (some legacy
            // entries surface plaintext here), then to a stable, short
            // id-based label so the user can still tell folders apart.
            let display = owner_keys::display_name(f, id, &master_key)
                .or_else(|| {
                    if !f["acted_via_share_id"].is_null() || !f["uploaded_by"].is_null() || !f["owner_wrapped_file_key"].is_null() { return None; }
                    let path = f.get("path").and_then(|v| v.as_str()).unwrap_or("");
                    if path.is_empty() {
                        None
                    } else {
                        Some(path.trim_start_matches('/').to_string())
                    }
                })
                .unwrap_or_else(|| folder_fallback_label(id));

            out.push(VaultItem {
                id: id.to_string(),
                name: display,
                is_folder: true,
                excluded: excluded.contains(id),
                size_bytes: 0,
                file_count: 0,
                on_disk_bytes: 0,
                children: Vec::new(),
            });
        }
        Ok(out)
    })
}

#[tauri::command]
async fn list_remote_tree(state: State<'_, AppState>) -> Result<Vec<VaultItem>, String> {
    list_vault_folders(state).await
}

#[tauri::command]
fn set_recursive_pin(item_id: String, pinned: bool) -> Result<engine_bridge::PinUpdateOutcome, String> {
    let cfg = DesktopConfig::load()?;
    let sync_root = cfg
        .sync_root
        .ok_or_else(|| "Choose a sync folder before changing offline availability.".to_string())?;
    let db_path = state_paths::beebeeb_state_dir()?.join(state_paths::STATE_DB_FILENAME);
    if !db_path.exists() {
        return Err("Beebeeb hasn’t started syncing on this Mac yet. Try again once it has.".to_string());
    }

    let db = std::sync::Arc::new(state_db::StateDb::open(&db_path).map_err(|e| format!("open state.db: {e}"))?);
    let api = std::sync::Arc::new(api_client::ApiClient::new(
        runner::api_base_url(),
        String::new(),
        [0u8; 32],
    ));
    let bridge = engine_bridge::EngineBridge::new(db, api);
    // `sync_root` is threaded through so the Windows path can resolve the
    // affected placeholder(s) and set the OS-level pin state (CfSetPinState);
    // it is unused on macOS/Linux.
    bridge
        .set_recursive_pin(&sync_root, &item_id, pinned)
        .map_err(|e| format!("set recursive pin: {e}"))
}

/// Best-effort lookup of the signed-in user's email, used by
/// `repos/desktop/src/pages/Account.tsx`.
///
/// Currently returns `Ok(None)` whenever no email is reachable from
/// the daemon's session struct — which is always, because Phase 1's
/// `Session` only carries `token` + `master_key` (no email). Surfacing
/// the email here requires either:
///
///   - extending `set_session` to also push a plaintext email, or
///   - calling `GET /api/v1/me` from the Rust side and caching it
///
/// Both are tracked under the auth-persistence work; until one lands
/// the Account page falls back to a "Signed in" placeholder. The
/// command intentionally returns `Ok(None)` rather than `Err(...)` so
/// the TS side doesn't render a noisy error banner for a known gap.
#[tauri::command]
fn account_email(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let acct = state.active_account()?;
    let guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
    if let Some(email) = guard.as_ref().and_then(|s| s.email.clone()) {
        return Ok(Some(email));
    }
    drop(guard);
    Ok(acct.auth_email.lock().ok().and_then(|guard| guard.clone()))
}

/// The last email that signed in on this install, for prefilling the
/// onboarding sign-in form after a sign-out or a startup-401 auto sign-out
/// (the keychain account-email credential is erased by sign-out, so this
/// reads the non-secret `desktop.toml` field `last_signed_in_email` — see
/// its doc for the privacy posture). `Ok(None)` when nothing was ever
/// signed in on this install, or the user hand-edited the value away.
#[tauri::command]
fn last_signed_in_email() -> Result<Option<String>, String> {
    Ok(DesktopConfig::load().ok().and_then(|cfg| cfg.last_signed_in_email))
}

// ── IPC commands: data layer (account / billing / devices / activity) ─────────
//
// PKG-DATA: wrap the EXISTING server endpoints the desktop never called, so
// the data-backed views ("all pages empty") have something to render. Each
// command builds a short-lived `ApiClient` from the in-memory session,
// fetches a typed DTO, and maps any transport error to a `String` the
// WebView can surface. They share the same "vault locked" gate as
// `desktop_storage_summary`: no in-memory session → `Err("vault is locked")`.

/// Build an [`api_client::ApiClient`] from the current in-memory session.
///
/// Returns `Err("vault is locked")` when there is no session — the same
/// signal `desktop_storage_summary` uses, so the frontend can treat "locked"
/// uniformly. Clones the token + master key out from under the lock so the
/// returned client owns its credentials (the `ApiClient` is intentionally
/// `!Clone`, but constructing a fresh one per call is cheap and keeps the
/// one-authoritative-copy invariant on `Session` itself intact).
fn api_client_from_session(state: &State<'_, AppState>) -> Result<api_client::ApiClient, String> {
    // Routing the session lookup here to the active account fixes ALL ~18
    // `account_*` data IPCs in one place (decision 0800): each builds its client
    // through this helper. The `master_key: [u8; 32]` copied into the client is
    // the SAME existing pattern — a `Copy` of the key bytes, not a second
    // authoritative `Session` — so the master-key isolation invariant holds.
    let acct = state.active_account()?;
    let guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
    let session = guard.as_ref().ok_or_else(|| "vault is locked".to_string())?;
    Ok(api_client::ApiClient::new(
        runner::api_base_url(),
        session.token.clone(),
        session.master_key,
    ))
}

/// `GET /api/v1/auth/me` (cached). Serves the profile cached at login if
/// present; otherwise fetches once and caches it. Never double-fetches.
#[tauri::command]
async fn account_profile(state: State<'_, AppState>) -> Result<account_dto::AccountProfile, String> {
    session_command!(async {
        let acct = state.active_account()?;
        {
            let guard = acct
                .cached_profile
                .lock()
                .map_err(|_| "profile mutex poisoned".to_string())?;
            if let Some(profile) = guard.as_ref() {
                return Ok(profile.clone());
            }
        }
        // Captured before the fetch (Task 12 fix round 1, item 6): a Lock or a Sign-out that completes while it is in
        // flight clears the cached profile, and this answer must not put it back.
        let turn = acct.session_generation();
        let api = api_client_from_session(&state)?;
        let profile = api.account_profile().await.map_err(|e| e.to_string())?;
        cache_profile_in_turn(&acct, &turn, profile.clone());
        Ok(profile)
    })
}

/// `GET /api/v1/auth/me`, always from the server. `account_profile` serves what the session cached at sign-in, which is only
/// as old as the sign-in: Settings › Security shows whether the authenticator app is on, and that must be what the account
/// says now (turned off on the web, it said On until the next sign-in; PR #140 delta review, P2-B).
///
/// It writes nothing: not the cache, not the session. The cached profile is an input to `identity_of_session`, which takes
/// it only when its email is exactly the session's; a profile fetched after the email was changed on the web (the legacy
/// path) carries the new address, and caching it would make the session lose the user id it knows (follow-up P3-6). A caller
/// that sees the session move while this runs is told "Account changed" by the command layer, so no turn is captured here.
#[tauri::command]
async fn account_profile_fresh(state: State<'_, AppState>) -> Result<account_dto::AccountProfile, String> {
    session_command!(async {
        let api = api_client_from_session(&state)?;
        api.account_profile().await.map_err(|e| e.to_string())
    })
}

/// The `account_profile` command's cache (Task 12 fix round 1, item 6): written only while no session transition
/// happened since the fetch began (`turn`), checked under the session-write lock. The cached profile IS an input to a
/// sign-in's decision (`credentials_identity`, the `cached_profile` trace), but this one is the session's OWN record,
/// fetched with its own token, so it only makes what is known about the same account more precise; it names no session
/// and does not move the generation on. `true` when it cached.
fn cache_profile_in_turn(
    acct: &account::AccountRuntime,
    turn: &account::SessionGeneration,
    profile: account_dto::AccountProfile,
) -> bool {
    let Ok(_write) = check_session_turn(acct, turn) else {
        return false;
    };
    match acct.cached_profile.lock() {
        Ok(mut guard) => {
            *guard = Some(profile);
            true
        }
        Err(_) => false,
    }
}

/// `GET /api/v1/billing/subscription` — plan + quota + lifecycle.
#[tauri::command]
async fn account_subscription(state: State<'_, AppState>) -> Result<account_dto::Subscription, String> {
    session_command!(async {
        api_client_from_session(&state)?
            .subscription()
            .await
            .map_err(|e| e.to_string())
    })
}

/// `GET /api/v1/billing/usage` — used/quota bytes + percentage.
#[tauri::command]
async fn account_usage(state: State<'_, AppState>) -> Result<account_dto::BillingUsage, String> {
    session_command!(async {
        api_client_from_session(&state)?
            .billing_usage()
            .await
            .map_err(|e| e.to_string())
    })
}

/// `GET /api/v1/me/region` — preferred region + available list. The list is by COUNTRY (task
/// 2050); no city and no provider reaches the frontend.
#[tauri::command]
async fn account_region(state: State<'_, AppState>) -> Result<account_dto::UserRegionResponse, String> {
    session_command!(async {
        api_client_from_session(&state)?
            .account_region()
            .await
            .map_err(|e| e.to_string())
    })
}

/// `PUT /api/v1/me/region` — set or clear the preferred storage region (`europe` or a country
/// id) and, for a country, "only store in <country>" (`region_strict`).
#[tauri::command]
async fn account_set_region(
    state: State<'_, AppState>,
    preferred_region: Option<String>,
    region_strict: Option<bool>,
) -> Result<account_dto::SetPreferredRegionResponse, String> {
    session_command!(async {
        api_client_from_session(&state)?
            .set_preferred_region(preferred_region.as_deref(), region_strict)
            .await
            .map_err(|e| e.to_string())
    })
}

/// `GET /api/v1/account/activity` — recent events + security summary.
#[tauri::command]
async fn account_activity(state: State<'_, AppState>) -> Result<account_dto::AccountActivity, String> {
    session_command!(async {
        api_client_from_session(&state)?
            .account_activity()
            .await
            .map_err(|e| e.to_string())
    })
}

/// `GET /api/v1/account/security-score` — numeric score + factors.
#[tauri::command]
async fn account_security_score(state: State<'_, AppState>) -> Result<account_dto::SecurityScore, String> {
    session_command!(async {
        api_client_from_session(&state)?
            .security_score()
            .await
            .map_err(|e| e.to_string())
    })
}

/// `GET /api/v1/account/sessions` — active account sessions.
#[tauri::command]
async fn account_session_list(state: State<'_, AppState>) -> Result<account_dto::AccountSessionList, String> {
    session_command!(async {
        api_client_from_session(&state)?
            .account_sessions()
            .await
            .map_err(|e| e.to_string())
    })
}

/// `DELETE /api/v1/account/sessions/{id}` — revoke one session.
#[tauri::command]
async fn account_revoke_session(state: State<'_, AppState>, session_id: String) -> Result<(), String> {
    session_command!(async {
        api_client_from_session(&state)?
            .revoke_account_session(&session_id)
            .await
            .map_err(|e| e.to_string())
    })
}

/// `POST /api/v1/account/sessions/revoke-all-others` — revoke every other
/// session; returns the count revoked.
#[tauri::command]
async fn account_revoke_other_sessions(state: State<'_, AppState>) -> Result<account_dto::RevokeAllResult, String> {
    session_command!(async {
        api_client_from_session(&state)?
            .revoke_other_sessions()
            .await
            .map_err(|e| e.to_string())
    })
}

/// `GET /api/v1/clients/devices` — registered client devices.
#[tauri::command]
async fn account_devices(state: State<'_, AppState>) -> Result<account_dto::ClientDeviceList, String> {
    session_command!(async {
        api_client_from_session(&state)?
            .client_devices()
            .await
            .map_err(|e| e.to_string())
    })
}

/// `GET /api/v1/clients/sessions` — per-device sync sessions.
#[tauri::command]
async fn account_client_sessions(state: State<'_, AppState>) -> Result<account_dto::ClientSyncSessionList, String> {
    session_command!(async {
        api_client_from_session(&state)?
            .client_sync_sessions()
            .await
            .map_err(|e| e.to_string())
    })
}

/// `GET /api/v1/activity` — paginated audit-log feed. `page`/`limit` are
/// optional; `None` lets the server defaults apply.
#[tauri::command]
async fn account_activity_feed(
    state: State<'_, AppState>,
    page: Option<u32>,
    limit: Option<u32>,
) -> Result<account_dto::ActivityFeed, String> {
    session_command!(async {
        let mut feed = api_client_from_session(&state)?.activity_feed(page, limit).await.map_err(|e| e.to_string())?;
        if page.unwrap_or(1) == 1 {
            let bridge = parked_conflict_bridge(&state)?;
            bridge.append_shared_activity(&mut feed).await.map_err(|e| e.to_string())?;
        }
        Ok(feed)
    })
}

/// `GET /api/v1/notifications` — notification list + unread count.
#[tauri::command]
async fn account_notifications(state: State<'_, AppState>) -> Result<account_dto::NotificationList, String> {
    session_command!(async {
        api_client_from_session(&state)?
            .notifications()
            .await
            .map_err(|e| e.to_string())
    })
}

/// `GET /api/v1/notifications/preferences` — push-preference toggles.
#[tauri::command]
async fn account_notification_preferences(
    state: State<'_, AppState>,
) -> Result<account_dto::NotificationPreferences, String> {
    session_command!(async {
        api_client_from_session(&state)?
            .notification_preferences()
            .await
            .map_err(|e| e.to_string())
    })
}

/// `PUT /api/v1/notifications/preferences` — partial update; omitted fields
/// keep their current server value.
#[tauri::command]
async fn account_update_notification_preferences(
    state: State<'_, AppState>,
    update: account_dto::NotificationPreferencesUpdate,
) -> Result<account_dto::NotificationPreferences, String> {
    session_command!(async {
        api_client_from_session(&state)?
            .update_notification_preferences(&update)
            .await
            .map_err(|e| e.to_string())
    })
}

fn trash_name_fallback(file_meta: &serde_json::Value, id: &str) -> String {
    if !file_meta["acted_via_share_id"].is_null() || !file_meta["uploaded_by"].is_null() || !file_meta["owner_wrapped_file_key"].is_null() { return owner_keys::blocked_name(id); }
    let from_plain = ["display_name", "decrypted_name", "filename", "file_name", "name"]
        .into_iter()
        .find_map(|key| {
            file_meta
                .get(key)
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.trim().to_string())
        });
    if let Some(name) = from_plain {
        return name;
    }
    if let Some(path) = file_meta
        .get("path")
        .and_then(|v| v.as_str())
        .or_else(|| file_meta.get("display_path").and_then(|v| v.as_str()))
        .and_then(folder_leaf_name)
    {
        return path;
    }
    let short = id.get(..8).unwrap_or(id);
    format!("Encrypted item {short}…")
}

fn trash_item_from_meta(file_meta: &serde_json::Value, master_key: &[u8; 32]) -> Option<account_dto::DesktopTrashItem> {
    let id = file_meta.get("id").and_then(|v| v.as_str())?.to_string();
    let name = owner_keys::display_name(file_meta, &id, master_key).unwrap_or_else(|| trash_name_fallback(file_meta, &id));
    let updated_at = file_meta
        .get("updated_at")
        .and_then(|v| {
            v.as_str()
                .map(str::to_string)
                .or_else(|| v.as_i64().map(|n| n.to_string()))
        })
        .unwrap_or_default();

    Some(account_dto::DesktopTrashItem {
        id,
        name,
        is_folder: file_meta
            .get("is_folder")
            .and_then(|v| v.as_bool())
            .unwrap_or_else(|| file_meta.get("kind").and_then(|v| v.as_str()) == Some("folder")),
        size_bytes: file_meta
            .get("size_bytes")
            .and_then(|v| v.as_i64())
            .or_else(|| file_meta.get("size").and_then(|v| v.as_i64()))
            .unwrap_or(0),
        updated_at,
        parent_id: file_meta.get("parent_id").and_then(|v| v.as_str()).map(str::to_string),
    })
}

async fn confirm_action_plaintext(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    password: &str,
) -> Result<account_dto::ConfirmActionResult, String> {
    let res = client
        .post(format!("{base_url}/api/v1/auth/confirm"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({ "password": password }))
        .send()
        .await
        .map_err(|e| format!("network error: {e}"))?;
    if res.status() == reqwest::StatusCode::UNAUTHORIZED {
        let body = res.json::<serde_json::Value>().await.unwrap_or_default();
        if body.get("error").and_then(|v| v.as_str()) == Some("session_too_old_for_confirmation") {
            return Err(body
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Sign in again before confirming this action.")
                .to_string());
        }
        return Err("Incorrect password".to_string());
    }
    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        return Err(format!("Confirm action failed ({status}): {body}"));
    }
    res.json()
        .await
        .map_err(|e| format!("parse confirm action response: {e}"))
}

/// Exchange the current user's password for a single-use destructive-action
/// token. Mirrors the web app's OPAQUE step-up flow, with a plaintext fallback
/// only for legacy accounts that explicitly report `opaque_unavailable`.
#[tauri::command]
async fn desktop_confirm_action(
    state: State<'_, AppState>,
    password: String,
) -> Result<account_dto::ConfirmActionResult, String> {
    session_command!(async {
        if password.is_empty() {
            return Err("Password is required.".to_string());
        }
        let token = {
            let acct = state.active_account()?;
            let guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
            guard
                .as_ref()
                .map(|s| s.token.clone())
                .ok_or_else(|| "vault is locked".to_string())?
        };

        let base_url = runner::api_base_url();
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .default_headers(api_client::provenance_headers())
            .build()
            .map_err(|e| format!("reqwest build: {e}"))?;

        let login_start = beebeeb_core::opaque_protocol::client_login_start(password.as_bytes())
            .map_err(|e| format!("opaque confirm start: {e}"))?;
        let start_resp = client
            .post(format!("{base_url}/api/v1/auth/confirm-opaque-start"))
            .header("Authorization", format!("Bearer {token}"))
            .json(&serde_json::json!({ "client_message": encode_base64(&login_start.message) }))
            .send()
            .await
            .map_err(|e| format!("network error: {e}"))?;

        if start_resp.status() == reqwest::StatusCode::CONFLICT {
            let body = start_resp.json::<serde_json::Value>().await.unwrap_or_default();
            if body.get("opaque_unavailable").and_then(|v| v.as_bool()) == Some(true) {
                return confirm_action_plaintext(&client, &base_url, &token, &password).await;
            }
            return Err(body
                .get("message")
                .or_else(|| body.get("error"))
                .and_then(|v| v.as_str())
                .unwrap_or("Confirm action failed.")
                .to_string());
        }
        if !start_resp.status().is_success() {
            let status = start_resp.status();
            let body = start_resp.text().await.unwrap_or_default();
            return Err(format!("Confirm action start failed ({status}): {body}"));
        }

        let start_body: serde_json::Value = start_resp
            .json()
            .await
            .map_err(|e| format!("parse confirm start response: {e}"))?;
        let server_message = start_body
            .get("server_message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "No server_message in confirm start response".to_string())?;
        let server_state = start_body
            .get("server_state")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "No server_state in confirm start response".to_string())?;
        let ksf_version = start_body
            .get("ksf_version")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| "No ksf_version in confirm start response".to_string())? as u32;
        let server_message_bytes =
            decode_base64(server_message).map_err(|e| format!("invalid OPAQUE server message: {e}"))?;
        let login_finish = beebeeb_core::opaque_protocol::client_login_finish(
            &login_start.state,
            password.as_bytes(),
            &server_message_bytes,
            ksf_version,
        )
        .map_err(|_| "Incorrect password".to_string())?;

        let finish_resp = client
            .post(format!("{base_url}/api/v1/auth/confirm-opaque-finish"))
            .header("Authorization", format!("Bearer {token}"))
            .json(&serde_json::json!({
                "client_message": encode_base64(&login_finish.message),
                "server_state": server_state,
            }))
            .send()
            .await
            .map_err(|e| format!("network error: {e}"))?;
        if finish_resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err("Incorrect password".to_string());
        }
        if !finish_resp.status().is_success() {
            let status = finish_resp.status();
            let body = finish_resp.text().await.unwrap_or_default();
            return Err(format!("Confirm action finish failed ({status}): {body}"));
        }
        finish_resp
            .json()
            .await
            .map_err(|e| format!("parse confirm finish response: {e}"))
    })
}

#[tauri::command]
async fn desktop_trash_list(state: State<'_, AppState>) -> Result<Vec<account_dto::DesktopTrashItem>, String> {
    session_command!(async {
        let api = api_client_from_session(&state)?;
        let raw = api.list_trashed_files_all().await.map_err(|e| e.to_string())?;
        let db = DesktopConfig::load().ok().and_then(|cfg| state_db_for_config(&cfg).ok().flatten());
        Ok(raw.iter().filter_map(|file_meta| {
            let mut item = trash_item_from_meta(file_meta, api.master_key())?;
            if let Some(db) = db.as_ref()
                && let Ok(Some(name)) = owner_keys::stored_name(db, &item.id, api.master_key()) { item.name = name; }
            Some(item)
        }).collect())
    })
}

#[tauri::command]
async fn desktop_trash_restore(
    state: State<'_, AppState>,
    file_id: String,
    file_name: Option<String>,
) -> Result<(), String> {
    session_command!(async {
        let api = api_client_from_session(&state)?;
        api.restore_file(&file_id).await.map_err(|e| e.to_string())?;

        if let Ok(Some(db)) = DesktopConfig::load().and_then(|cfg| state_db_for_config(&cfg)) {
            let _ = db.record_local_activity(state_db::LocalActivityEventInput {
                event_type: state_db::LocalActivityKind::Restored,
                file_id: Some(file_id.clone()),
                file_name: file_name
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| file_id.clone()),
                rel_path: None,
                occurred_at: now_unix_seconds(),
            });
            let _ = db.request_resnapshot();
        }

        Ok(())
    })
}

#[tauri::command]
async fn desktop_trash_delete_permanently(
    state: State<'_, AppState>,
    file_id: String,
    confirm_token: String,
) -> Result<(), String> {
    session_command!(async {
        if confirm_token.trim().is_empty() {
            return Err("Confirmation token is required.".to_string());
        }
        api_client_from_session(&state)?
            .permanent_delete_file(&file_id, &confirm_token)
            .await
            .map_err(|e| e.to_string())?;
        if let Ok(Some(db)) = DesktopConfig::load().and_then(|cfg| state_db_for_config(&cfg)) {
            let _ = db.request_resnapshot();
        }
        Ok(())
    })
}

/// Local storage breakdown by content type (Media / Documents / Other) plus
/// the N largest files, computed from the desktop SQLite mirror. No server
/// endpoint exists for this — it is computed client-side from
/// `state_db::list_files` (size + path extension). Folders are excluded.
///
/// `largest_limit` caps the returned `largest_files` list; defaults to 10
/// when `None`. Returns an empty breakdown (all three categories present at
/// zero) when no sync root is configured or the state DB is absent — the UI
/// renders a stable empty state rather than an error.
///
/// Gated on an in-memory session (`Err("vault is locked")` when absent) for
/// parity with the other data IPCs: this surfaces local vault file paths, so
/// it must not answer before the user has unlocked.
#[tauri::command]
async fn account_storage_breakdown(
    state: State<'_, AppState>,
    largest_limit: Option<usize>,
) -> Result<account_dto::StorageBreakdown, String> {
    session_command!(async {
        // Auth gate: require an unlocked session, same signal as the others.
        {
            let acct = state.active_account()?;
            let guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
            if guard.is_none() {
                return Err("vault is locked".to_string());
            }
        }

        let limit = largest_limit.unwrap_or(10);

        // Resolve the state DB from app-local data once a sync root is configured.
        // No root / no DB → empty (not an error).
        if DesktopConfig::load().ok().and_then(|cfg| cfg.sync_root).is_none() {
            return Ok(account_dto::compute_storage_breakdown(&[], limit));
        }
        let db_path = state_paths::beebeeb_state_dir()?.join(state_paths::STATE_DB_FILENAME);
        if !db_path.exists() {
            return Ok(account_dto::compute_storage_breakdown(&[], limit));
        }

        // SQLite work is blocking; run it off the async executor (mirrors
        // `free_up_space`, which uses the same `tokio::task::spawn_blocking` + `??`).
        let breakdown = tokio::task::spawn_blocking(move || -> Result<_, String> {
            let db = state_db::StateDb::open(&db_path).map_err(|e| format!("open state db: {e}"))?;
            let entries = db.list_files().map_err(|e| format!("list files: {e}"))?;
            // Folders carry no real size; exclude them from the breakdown.
            let inputs: Vec<account_dto::BreakdownInput> = entries
                .into_iter()
                .filter(|e| !e.is_dir())
                .map(|e| account_dto::BreakdownInput {
                    file_id: e.file_id,
                    path: e.path,
                    size_bytes: e.size_bytes,
                })
                .collect();
            Ok(account_dto::compute_storage_breakdown(&inputs, limit))
        })
        .await
        .map_err(|e| format!("storage breakdown task failed: {e}"))??;

        Ok(breakdown)
    })
}

/// The overview's view of file rows. Folders carry no real size and no sync state worth surfacing; they are left out.
fn file_overview_inputs(rows: Vec<(state_db::FileEntry, bool)>) -> Vec<account_dto::FileOverviewInput> {
    rows.into_iter()
        .filter(|(e, _)| !e.is_dir())
        .map(|(e, pinned)| account_dto::FileOverviewInput {
            path: e.path,
            size_bytes: e.size_bytes,
            status: e.status.as_str().to_string(),
            pinned,
            modified_at: e.modified_at,
        })
        .collect()
}

/// The newest `limit` local activity events, as the overview's recent list takes them.
fn file_overview_activity(
    db: &state_db::StateDb,
    limit: usize,
) -> Result<Vec<account_dto::FileOverviewActivityInput>, String> {
    Ok(db
        .list_recent_local_activity(limit)
        .map_err(|e| format!("list local activity: {e}"))?
        .into_iter()
        .map(|event| account_dto::FileOverviewActivityInput {
            event_type: event.event_type.as_str().to_string(),
            file_name: event.file_name,
            rel_path: event.rel_path,
            occurred_at: event.occurred_at,
        })
        .collect())
}

/// The tray's recent list (task 2243): the overview's `recent`, without its totals.
fn recent_files(db: &state_db::StateDb, limit: usize) -> Result<Vec<account_dto::RecentFile>, String> {
    // The newest `limit` files and the newest `limit` events are all a list of `limit` can hold.
    let rows = db.newest_file_rows(limit).map_err(|e| format!("list files: {e}"))?;
    Ok(
        account_dto::compute_file_overview(&file_overview_inputs(rows), &file_overview_activity(db, limit)?, limit)
            .recent,
    )
}

/// Per-PC **sync-state** overview for the in-app "Files" tab — the device
/// lens that complements `account_storage_breakdown` (the by-TYPE / billing
/// lens). Computed LOCALLY from the desktop SQLite mirror
/// (`state_db::file_overview_rows`): files grouped by sync status
/// (local / cloud_only / downloading / uploading / conflict / error), the N
/// most-recently-changed files (with effective-pinned flag), and totals.
/// Folders are excluded.
///
/// `recent_limit` caps the returned `recent` list; defaults to 15 when `None`.
/// Returns an empty overview when no sync root is configured or the state DB is
/// absent — the UI renders a stable empty state rather than an error.
///
/// Gated on an in-memory session (`Err("vault is locked")` when absent) for
/// parity with the other data IPCs: this surfaces local vault file paths, so it
/// must not answer before the user has unlocked.
#[tauri::command]
async fn desktop_file_overview(
    state: State<'_, AppState>,
    recent_limit: Option<usize>,
) -> Result<account_dto::FileOverview, String> {
    session_command!(async {
        // Auth gate: require an unlocked session, same signal as the others.
        {
            let acct = state.active_account()?;
            let guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
            if guard.is_none() {
                return Err("vault is locked".to_string());
            }
        }

        let limit = recent_limit.unwrap_or(15);

        // Resolve the state DB from app-local data once a sync root is configured.
        // No root / no DB → empty (not an error).
        if DesktopConfig::load().ok().and_then(|cfg| cfg.sync_root).is_none() {
            return Ok(account_dto::compute_file_overview(&[], &[], limit));
        }
        let db_path = state_paths::beebeeb_state_dir()?.join(state_paths::STATE_DB_FILENAME);
        if !db_path.exists() {
            return Ok(account_dto::compute_file_overview(&[], &[], limit));
        }

        // SQLite work is blocking; run it off the async executor (mirrors
        // `account_storage_breakdown`).
        let overview = tokio::task::spawn_blocking(move || -> Result<_, String> {
            let db = state_db::StateDb::open(&db_path).map_err(|e| format!("open state db: {e}"))?;
            let rows = db.file_overview_rows().map_err(|e| format!("list files: {e}"))?;
            Ok(account_dto::compute_file_overview(
                &file_overview_inputs(rows),
                &file_overview_activity(&db, limit)?,
                limit,
            ))
        })
        .await
        .map_err(|e| format!("file overview task failed: {e}"))??;

        Ok(overview)
    })
}

/// The tray's recent list (task 2243), which it polls every few seconds while it is open. [`desktop_file_overview`]
/// reads every row for its totals; this reads the newest rows alone.
#[tauri::command]
async fn desktop_recent_files(
    state: State<'_, AppState>,
    recent_limit: Option<usize>,
) -> Result<Vec<account_dto::RecentFile>, String> {
    session_command!(async {
        {
            let acct = state.active_account()?;
            let guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
            if guard.is_none() {
                return Err("vault is locked".to_string());
            }
        }
        let limit = recent_limit.unwrap_or(15);
        tokio::task::spawn_blocking(move || -> Result<_, String> {
            // As in the overview: no readable config, no sync root or no DB yet is an empty list, not an error (the tray
            // shows an error as signed out).
            let config = DesktopConfig::load().ok();
            let Some(db) = config.as_ref().map(state_db_for_config).transpose()?.flatten() else {
                return Ok(Vec::new());
            };
            recent_files(&db, limit)
        })
        .await
        .map_err(|e| format!("recent files task failed: {e}"))?
    })
}

/// Query the local desktop file-name search index.
///
/// The index is built from the local StateDb mirror and queried in-process, so
/// typing into the desktop quick-open never waits on the network. When the sync
/// root is not configured or the local mirror has not been created yet, return a
/// typed `syncing` response instead of an empty-looking "no matches" result.
#[tauri::command]
async fn desktop_search_files(
    state: State<'_, AppState>,
    query: String,
    limit: Option<usize>,
    for_finder: Option<bool>,
) -> Result<desktop_search::DesktopSearchResponse, String> {
    session_command!(async {
        {
            let acct = state.active_account()?;
            let guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
            if guard.is_none() {
                return Err("vault is locked".to_string());
            }
        }

        let Some(db) = DesktopConfig::load().and_then(|cfg| state_db_for_config(&cfg))? else {
            return Ok(desktop_search::syncing_response(query));
        };

        let max_results = limit.unwrap_or(12).clamp(1, 50);
        // Quick search asks for rows it will show in Finder; a Mac's Finder has no shared-with-me rows (ruling 1701), and
        // they are dropped BEFORE the limit (task 1885 fix round 2, n1). Version history asks for none of this.
        let exclude_shared = for_finder.unwrap_or(false) && cfg!(target_os = "macos");
        let response = tokio::task::spawn_blocking(move || -> Result<_, String> {
            let local = desktop_search::build_local_index(&db).map_err(|e| format!("build search index: {e}"))?;
            Ok(desktop_search::query_local_index(
                &local,
                &query,
                max_results,
                exclude_shared,
            ))
        })
        .await
        .map_err(|e| format!("search task failed: {e}"))??;

        Ok(response)
    })
}

// ── IPC commands: App Activity — local process resources (task 1191) ─────────

static APP_ACTIVITY_MONITOR: LazyLock<Mutex<AppActivityMonitor>> =
    LazyLock::new(|| Mutex::new(AppActivityMonitor::new()));

#[derive(Debug, Clone, serde::Serialize)]
struct AppActivitySnapshot {
    pid: u32,
    process_name: String,
    cpu_percent: f32,
    memory_rss_bytes: u64,
    upload_bps: Option<u64>,
    download_bps: Option<u64>,
    throughput_sample_age_ms: Option<u64>,
    throughput_period_secs: Option<u32>,
}

#[derive(Debug, Clone)]
struct ProcessResourceSnapshot {
    pid: u32,
    process_name: String,
    cpu_percent: f32,
    memory_rss_bytes: u64,
}

#[derive(Debug, Clone)]
struct ThroughputSnapshot {
    upload_bps: Option<u64>,
    download_bps: Option<u64>,
    sample_age_ms: Option<u64>,
    period_secs: Option<u32>,
}

struct AppActivityMonitor {
    system: System,
    pid: sysinfo::Pid,
    logical_core_count: usize,
}

impl AppActivityMonitor {
    fn new() -> Self {
        let pid = sysinfo::get_current_pid().unwrap_or_else(|_| sysinfo::Pid::from_u32(std::process::id()));
        let mut system = System::new();
        system.refresh_cpu_list(CpuRefreshKind::nothing());
        let logical_core_count = system.cpus().len().max(1);
        Self {
            system,
            pid,
            logical_core_count,
        }
    }

    fn process_snapshot(&mut self) -> Result<ProcessResourceSnapshot, String> {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[self.pid]),
            true,
            ProcessRefreshKind::nothing().with_cpu().with_memory(),
        );

        let process = self
            .system
            .process(self.pid)
            .ok_or_else(|| format!("current process {} not found", self.pid))?;
        let cpu_percent = normalize_process_cpu_percent(process.cpu_usage(), self.logical_core_count);

        Ok(ProcessResourceSnapshot {
            pid: self.pid.as_u32(),
            process_name: process.name().to_string_lossy().into_owned(),
            cpu_percent,
            memory_rss_bytes: process.memory(),
        })
    }
}

fn normalize_process_cpu_percent(raw_cpu_percent: f32, logical_core_count: usize) -> f32 {
    if !raw_cpu_percent.is_finite() {
        return 0.0;
    }

    let normalized = raw_cpu_percent.max(0.0) / logical_core_count.max(1) as f32;
    (normalized.clamp(0.0, 100.0) * 10.0).round() / 10.0
}

fn bandwidth_sample_rates(up_bytes: u64, down_bytes: u64, period_secs: u32) -> (u64, u64) {
    if period_secs == 0 {
        return (0, 0);
    }
    (up_bytes / period_secs as u64, down_bytes / period_secs as u64)
}

fn latest_app_activity_throughput_snapshot() -> Result<ThroughputSnapshot, String> {
    if DesktopConfig::load().ok().and_then(|cfg| cfg.sync_root).is_none() {
        return Ok(ThroughputSnapshot {
            upload_bps: None,
            download_bps: None,
            sample_age_ms: None,
            period_secs: None,
        });
    }

    let db_path = state_paths::beebeeb_state_dir()?.join(state_paths::STATE_DB_FILENAME);
    if !db_path.exists() {
        return Ok(ThroughputSnapshot {
            upload_bps: None,
            download_bps: None,
            sample_age_ms: None,
            period_secs: None,
        });
    }

    let db = state_db::StateDb::open(&db_path).map_err(|e| format!("open state db: {e}"))?;
    let Some(sample) = db
        .latest_bandwidth_sample()
        .map_err(|e| format!("latest bandwidth sample: {e}"))?
    else {
        return Ok(ThroughputSnapshot {
            upload_bps: None,
            download_bps: None,
            sample_age_ms: None,
            period_secs: None,
        });
    };

    let (upload_bps, download_bps) = bandwidth_sample_rates(sample.up_bytes, sample.down_bytes, sample.period_secs);
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let sampled_at_secs = if sample.sampled_at > 0 {
        sample.sampled_at as u64
    } else {
        0
    };

    Ok(ThroughputSnapshot {
        upload_bps: Some(upload_bps),
        download_bps: Some(download_bps),
        sample_age_ms: Some(now_secs.saturating_sub(sampled_at_secs).saturating_mul(1000)),
        period_secs: Some(sample.period_secs),
    })
}

#[tauri::command]
async fn app_activity_snapshot() -> Result<AppActivitySnapshot, String> {
    let (process, throughput) = tokio::task::spawn_blocking(|| -> Result<_, String> {
        let process = APP_ACTIVITY_MONITOR
            .lock()
            .map_err(|_| "app activity monitor mutex poisoned".to_string())?
            .process_snapshot()?;
        let throughput = latest_app_activity_throughput_snapshot()?;
        Ok((process, throughput))
    })
    .await
    .map_err(|e| format!("app activity snapshot task failed: {e}"))??;

    Ok(AppActivitySnapshot {
        pid: process.pid,
        process_name: process.process_name,
        cpu_percent: process.cpu_percent,
        memory_rss_bytes: process.memory_rss_bytes,
        upload_bps: throughput.upload_bps,
        download_bps: throughput.download_bps,
        throughput_sample_age_ms: throughput.sample_age_ms,
        throughput_period_secs: throughput.period_secs,
    })
}

// ── IPC commands: Bandwidth view — speedtest + history (task 0810) ───────────

/// Result shape returned by [`run_speedtest`].
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SpeedtestResult {
    pub latency: SpeedtestLatency,
    /// Average upload speed in bytes per second.
    pub upload_bps: u64,
    /// Average download speed in bytes per second.
    pub download_bps: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SpeedtestLatency {
    pub min_ms: u64,
    pub avg_ms: u64,
    pub max_ms: u64,
}

/// What a run measured, kept apart from how it is said: only the pings that were answered and the transfers that
/// finished are in here. Failed ones are not zeros to average in (PR #140 review, P2-7).
#[derive(Debug, Default)]
struct SpeedtestSamples {
    /// Round trips of the pings the server answered, in milliseconds.
    ping_ms: Vec<u64>,
    /// Bytes per second of each upload that finished.
    up_bps: Vec<f64>,
    /// Bytes per second of each download that finished.
    down_bps: Vec<f64>,
}

const SPEEDTEST_NO_PING: &str = "Beebeeb didn’t answer any of the 5 pings. Check your internet connection and try again.";
const SPEEDTEST_NO_UPLOAD: &str = "None of the 3 uploads finished. Check your internet connection and try again.";
const SPEEDTEST_NO_DOWNLOAD: &str = "None of the 3 downloads finished. Check your internet connection and try again.";
const SPEEDTEST_ALREADY_RUNNING: &str = "A speed test is already running.";

/// A run's figures, or the plain reason there are none. A direction that never finished is not "0 Mbps" and an
/// unanswered ping is not a latency: a run without a figure in all three is a failed run, never a result.
fn speedtest_outcome(samples: &SpeedtestSamples) -> Result<SpeedtestResult, String> {
    if samples.ping_ms.is_empty() {
        return Err(SPEEDTEST_NO_PING.to_string());
    }
    if samples.up_bps.is_empty() {
        return Err(SPEEDTEST_NO_UPLOAD.to_string());
    }
    if samples.down_bps.is_empty() {
        return Err(SPEEDTEST_NO_DOWNLOAD.to_string());
    }
    let mean = |speeds: &[f64]| (speeds.iter().sum::<f64>() / speeds.len() as f64) as u64;
    Ok(SpeedtestResult {
        latency: SpeedtestLatency {
            min_ms: *samples.ping_ms.iter().min().unwrap_or(&0),
            avg_ms: samples.ping_ms.iter().sum::<u64>() / samples.ping_ms.len() as u64,
            max_ms: *samples.ping_ms.iter().max().unwrap_or(&0),
        },
        upload_bps: mean(&samples.up_bps),
        download_bps: mean(&samples.down_bps),
    })
}

/// At most one speed test at a time: the test moves 60 MB, and a second press (the Settings page and the Bandwidth view
/// both offer one) would halve both figures. Dropping the guard frees the slot, including when the command is cancelled.
struct SpeedtestSlot<'a>(&'a std::sync::atomic::AtomicBool);

impl<'a> SpeedtestSlot<'a> {
    fn take(flag: &'a std::sync::atomic::AtomicBool) -> Result<Self, String> {
        flag.compare_exchange(false, true, std::sync::atomic::Ordering::AcqRel, std::sync::atomic::Ordering::Acquire)
            .map(|_| Self(flag))
            .map_err(|_| SPEEDTEST_ALREADY_RUNNING.to_string())
    }
}

impl Drop for SpeedtestSlot<'_> {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
}

static SPEEDTEST_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Run a network speedtest against the Beebeeb API.
///
/// Parity with CLI (`repos/cli/src/commands/speedtest.rs`) and iOS
/// (`SpeedtestScreen`): 5 latency pings + 3 × 10 MB download + 3 × 10 MB upload.
/// Uses the authenticated session; returns `Err("vault is locked")` when none. A run in which nothing finished in one
/// of the three parts is an `Err` with the reason (see [`speedtest_outcome`]), and a second run while one is going is
/// refused.
#[tauri::command]
async fn run_speedtest(state: State<'_, AppState>) -> Result<SpeedtestResult, String> {
    session_command!(async {
        let api = api_client_from_session(&state)?;
        let _slot = SpeedtestSlot::take(&SPEEDTEST_RUNNING)?;
        let mut samples = SpeedtestSamples::default();

        // Latency: 5 × GET /api/v1/health. A ping the server did not answer has no round trip to report.
        for _ in 0..5 {
            let start = std::time::Instant::now();
            if api.ping_health().await.is_ok() {
                samples.ping_ms.push(start.elapsed().as_millis() as u64);
            }
        }

        // Upload: 3 × 10 MB POST /api/v1/speedtest
        let blob = vec![0u8; 10_000_000];
        for _ in 0..3 {
            let start = std::time::Instant::now();
            if api.speedtest_upload(&blob).await.is_ok() {
                let elapsed = start.elapsed().as_secs_f64();
                if elapsed > 0.0 {
                    samples.up_bps.push(blob.len() as f64 / elapsed);
                }
            }
        }

        // Download: 3 × 10 MB GET /api/v1/speedtest?size=10000000
        for _ in 0..3 {
            let start = std::time::Instant::now();
            if let Ok(data) = api.speedtest_download(10_000_000).await {
                let elapsed = start.elapsed().as_secs_f64();
                if elapsed > 0.0 {
                    samples.down_bps.push(data.len() as f64 / elapsed);
                }
            }
        }

        speedtest_outcome(&samples)
    })
}

/// Return bandwidth samples from the local state DB for the traffic chart.
///
/// `hours` controls the window (default 20). The DB stores one sample per
/// heartbeat beat (~20 s), so a 20-hour window returns up to ~3600 samples —
/// the frontend downsamples to a fixed number of chart buckets for display.
#[tauri::command]
async fn get_bandwidth_history(
    state: State<'_, AppState>,
    hours: Option<u32>,
) -> Result<Vec<state_db::BandwidthSample>, String> {
    session_command!(async {
        // Auth gate.
        {
            let acct = state.active_account()?;
            let guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
            if guard.is_none() {
                return Err("vault is locked".to_string());
            }
        }

        let window_hours = hours.unwrap_or(20);

        if DesktopConfig::load().ok().and_then(|cfg| cfg.sync_root).is_none() {
            return Ok(vec![]);
        }
        let db_path = state_paths::beebeeb_state_dir()?.join(state_paths::STATE_DB_FILENAME);
        if !db_path.exists() {
            return Ok(vec![]);
        }

        tokio::task::spawn_blocking(move || -> Result<_, String> {
            let db = state_db::StateDb::open(&db_path).map_err(|e| format!("open state db: {e}"))?;
            db.get_bandwidth_history(window_hours)
                .map_err(|e| format!("bandwidth history: {e}"))
        })
        .await
        .map_err(|e| format!("bandwidth history task failed: {e}"))?
    })
}

// ── IPC commands: Task 12 — conflict window ───────────────────────────────────

/// Internal helper that does the actual window-building work. Called
/// by both the [`open_conflict_window`] IPC command (when the user or
/// frontend triggers it) and by [`crate::runner`]'s tick loop (when
/// the engine bridge detects a fresh conflict in Task 10). Splitting
/// this out lets us call directly from Rust without going through the
/// IPC dispatcher; the IPC command is just a thin wrapper.
pub(crate) fn open_conflict_window_impl(
    app: &tauri::AppHandle,
    file_id: &str,
    file_name: &str,
    is_text: bool,
    op_id: Option<&str>,
) -> Result<(), String> {
    let label = format!("conflict-{file_id}");

    // If the user already has the window open for this file, just
    // bring it forward — re-creating would error from Tauri and the
    // user-visible behaviour ("show me this conflict again") is the
    // same.
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.show();
        let _ = existing.set_focus();
        return Ok(());
    }

    // Task 2190: a parked conflict carries its queued write, which the window decides.
    let op_param = op_id.map(|op| format!("&opId={}", urlencoding::encode(op))).unwrap_or_default();
    let url = format!(
        "index.html?window=conflict&fileId={file_id}&fileName={enc_name}&isText={is_text}{op_param}",
        enc_name = urlencoding::encode(file_name),
    );

    tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::App(url.into()))
        .title(format!("Conflict — {file_name}"))
        .inner_size(720.0, 480.0)
        .resizable(true)
        .center()
        .build()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Open a conflict-resolution window for a single file. The label is
/// `conflict-<file_id>` so multiple simultaneous conflicts each get
/// their own window without colliding (re-opening the same conflict
/// is a soft-noop — show + focus instead of error).
///
/// The URL contract matches `repos/desktop/src/ConflictWindow.tsx`:
/// `index.html?window=conflict&fileId=<uuid>&fileName=<utf8>&isText=<bool>`.
/// `main.tsx` reads `?window=conflict` and mounts `ConflictWindow`
/// instead of the default compact app shell.
///
/// Spec: `docs/superpowers/plans/2026-05-07-desktop-sync-client.md`
/// Phase 4 Task 12. Also called internally by Task 10's conflict
/// detection path via [`open_conflict_window_impl`].
#[tauri::command]
async fn open_conflict_window(
    app: tauri::AppHandle,
    file_id: String,
    file_name: String,
    is_text: bool,
    op_id: Option<String>,
) -> Result<(), String> {
    open_conflict_window_impl(&app, &file_id, &file_name, is_text, op_id.as_deref())
}

/// Show a native OS notification announcing a fresh conflict. Used
/// by `repos/desktop/src-tauri/src/runner.rs` when `sync_tick`
/// flags a divergent file, and exposed as an IPC command for the
/// settings page in case it ever wants to manually re-surface a
/// conflict that's been sitting in the queue.
///
/// We respect the user's `notify_conflicts` toggle in
/// `desktop.toml` (Task 9) — a `false` there short-circuits to Ok
/// without raising the OS notification.
///
/// Spec: Phase 4 Task 11.
fn should_show_conflict_notification(config: Result<DesktopConfig, String>) -> bool {
    config.map(|c| c.notify_conflicts).unwrap_or(true)
}

fn should_show_sync_complete_notification(config: Result<DesktopConfig, String>) -> bool {
    config.map(|c| c.notify_sync_complete).unwrap_or(false)
}

fn should_show_quota_warning_notification(config: Result<DesktopConfig, String>) -> bool {
    config.map(|c| c.notify_quota_warnings).unwrap_or(true)
}

pub(crate) fn notify_conflict_impl(app: &tauri::AppHandle, file_name: &str) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;

    // Honour the user's preference. Best-effort load — if the config
    // file is unreadable we fall back to "show", matching the
    // documented Default of `notify_conflicts: true`.
    if !should_show_conflict_notification(DesktopConfig::load()) {
        return Ok(());
    }

    app.notification()
        .builder()
        .title("Conflict detected")
        .body(format!("Conflict in {file_name} — open Beebeeb to resolve"))
        .show()
        .map_err(|e| e.to_string())
}

pub(crate) fn notify_sync_complete_impl(app: &tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;

    if !should_show_sync_complete_notification(DesktopConfig::load()) {
        return Ok(());
    }

    app.notification()
        .builder()
        .title("All caught up")
        .body("Your Beebeeb files are up to date.")
        .show()
        .map_err(|e| e.to_string())
}

pub(crate) fn notify_quota_warning_impl(
    app: &tauri::AppHandle,
    local_cache_bytes: i64,
    local_cache_limit_bytes: i64,
) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;

    if local_cache_limit_bytes <= 0 {
        return Ok(());
    }
    if !should_show_quota_warning_notification(DesktopConfig::load()) {
        return Ok(());
    }

    let used = local_cache_bytes.max(0);
    let cap = local_cache_limit_bytes.max(1);
    let percent = ((used as f64 / cap as f64) * 100.0).round() as i64;

    app.notification()
        .builder()
        .title("Local cache nearly full")
        .body(format!(
            "{} used of {} local cache limit ({}%).",
            format_notification_bytes(used),
            format_notification_bytes(cap),
            percent
        ))
        .show()
        .map_err(|e| e.to_string())
}

fn format_notification_bytes(bytes: i64) -> String {
    let bytes = bytes.max(0) as f64;
    if bytes < 1_000.0 {
        format!("{} B", bytes as i64)
    } else if bytes < 1_000_000.0 {
        format!("{:.0} KB", bytes / 1_000.0)
    } else if bytes < 1_000_000_000.0 {
        format!("{:.0} MB", bytes / 1_000_000.0)
    } else if bytes < 1_000_000_000_000.0 {
        format!("{:.0} GB", bytes / 1_000_000_000.0)
    } else {
        format!("{:.1} TB", bytes / 1_000_000_000_000.0)
    }
}

/// IPC wrapper around [`notify_conflict_impl`]. Mostly here so the
/// frontend can trigger a notification on demand (e.g. for a
/// "remind me later" UI we may add); the engine's auto-detection
/// path calls the impl directly.
#[tauri::command]
async fn notify_conflict(app: tauri::AppHandle, file_name: String, file_id: String) -> Result<(), String> {
    let _ = file_id; // reserved for future click-to-open wiring
    notify_conflict_impl(&app, &file_name)
}

/// Apply the user's conflict resolution choice. `choice` is
/// `"local"` | `"remote"` | `"both"` per the TS-side button labels
/// in ConflictWindow.tsx (Keep Mine / Keep Theirs / Keep Both).
///
/// Routes each choice into the matching `EngineBridge` method:
///
///   - `"local"`  → [`engine_bridge::EngineBridge::resolve_keep_mine`]
///   - `"remote"` → [`engine_bridge::EngineBridge::resolve_keep_theirs`]
///   - `"both"`   → [`engine_bridge::EngineBridge::auto_resolve_keep_both`]
///
/// We build a fresh `EngineBridge` per call rather than reaching into
/// the long-lived runner. SQLite handles concurrent connections to the
/// same DB cleanly in WAL mode; reqwest is cheap to construct; and
/// avoiding a back-channel from the IPC layer into the runner task
/// keeps the dataflow obvious. The cost is one extra DB open per
/// resolve, which happens at user-click cadence — irrelevant.
#[tauri::command]
async fn resolve_conflict(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    file_id: String,
    choice: String,
) -> Result<(), String> {
    session_command!(async {
        // Validate the choice up front so a typo on the TS side fails
        // loudly instead of silently no-opping.
        match choice.as_str() {
            "local" | "remote" | "both" => {}
            _ => return Err(format!("invalid conflict choice: {choice}")),
        }

        // Pull token + master_key out of the in-memory session. We
        // intentionally do NOT clone the entire Session struct — only
        // what the ApiClient needs.
        let acct = state.active_account()?;
        let (token, master_key) = {
            let guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
            match guard.as_ref() {
                Some(s) => (s.token.clone(), s.master_key),
                None => return Err("not signed in".into()),
            }
        };

        // Sync root from desktop.toml — without it the daemon has no
        // place to write the resolved file, so this is a hard error.
        let cfg = DesktopConfig::load()?;
        let sync_root = cfg.sync_root.ok_or_else(|| "no sync root configured".to_string())?;

        // Build a fresh bridge over the app-local state DB (same convention as
        // `runner::run`).
        let db_path = state_paths::beebeeb_state_dir()?.join(state_paths::STATE_DB_FILENAME);
        let db = std::sync::Arc::new(state_db::StateDb::open(&db_path).map_err(|e| format!("open state.db: {e}"))?);
        let api = std::sync::Arc::new(api_client::ApiClient::new(runner::api_base_url(), token, master_key));
        let bridge = engine_bridge::EngineBridge::new(db, api);

        let outcome: Result<(), String> = match choice.as_str() {
            "local" => bridge
                .resolve_keep_mine(&file_id, &sync_root)
                .await
                .map_err(|e| e.to_string()),
            "remote" => bridge
                .resolve_keep_theirs(&file_id, &sync_root)
                .await
                .map_err(|e| e.to_string()),
            "both" => {
                // auto_resolve_keep_both wants the FileEntry; look it up.
                let entry = bridge
                    .db()
                    .get_file(&file_id)
                    .map_err(|e| format!("get_file: {e}"))?
                    .ok_or_else(|| format!("no state.db row for {file_id}"))?;
                bridge
                    .auto_resolve_keep_both(&sync_root, &entry)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
            // Unreachable — the validate above already caught it.
            _ => unreachable!(),
        };
        outcome?;

        // Notify the app window so it can re-poll sync_status and
        // refresh the conflict count without waiting for the next engine
        // tick.
        let _ = app.emit(
            "conflict-resolved",
            serde_json::json!({
                "file_id": file_id,
                "choice": choice,
            }),
        );

        Ok(())
    })
}

/// Task 2190 (plan §3.1): a bridge over the app-local state DB for a parked conflict. Unlike
/// `resolve_conflict`, it needs no sync root: on a Mac a parked write's bytes are its staged copy.
fn parked_conflict_bridge(state: &State<'_, AppState>) -> Result<engine_bridge::EngineBridge, String> {
    let acct = state.active_account()?;
    let (token, master_key) = {
        let guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
        match guard.as_ref() {
            Some(s) => (s.token.clone(), s.master_key),
            None => return Err("not signed in".into()),
        }
    };
    let db_path = state_paths::beebeeb_state_dir()?.join(state_paths::STATE_DB_FILENAME);
    let db = std::sync::Arc::new(state_db::StateDb::open(&db_path).map_err(|e| format!("open state.db: {e}"))?);
    let api = std::sync::Arc::new(api_client::ApiClient::new(runner::api_base_url(), token, master_key));
    Ok(engine_bridge::EngineBridge::new(db, api))
}

/// Task 2190 (plan §3.2, D1): "Keep the other" keeps the Mac's bytes in the "Changes not sent"
/// folder (spec 2026-10-10) FIRST: a new event folder, the copy under the file's path, and the copy
/// added to its OWN list durably (review-169 blocker: never the sign-out's one folder slot, never its
/// unfinished list), so Settings › Sync lists every copy until the person dismisses it. Any failure
/// removes what it made, and the decision changes nothing.
#[cfg(target_os = "macos")]
fn keep_parked_copy(op_id: &str, source: &std::path::Path, rel_path: &str) -> anyhow::Result<std::path::PathBuf> {
    keep_parked_copy_at(
        op_id,
        source,
        rel_path,
        &sign_out_kept_root().map_err(|e| anyhow::anyhow!(e))?,
        &DesktopConfig::path().map_err(|e| anyhow::anyhow!(e))?,
        &sign_out::kept_writer::SystemClone,
        &config::RealConfigFs,
    )
}

/// [`keep_parked_copy`] with its folder, its config file and its file system steps given.
#[cfg(target_os = "macos")]
fn keep_parked_copy_at(
    op_id: &str,
    source: &std::path::Path,
    rel_path: &str,
    root: &std::path::Path,
    config_path: &std::path::Path,
    clone: &dyn sign_out::kept_writer::CloneFile,
    config_fs: &dyn config::ConfigFs,
) -> anyhow::Result<std::path::PathBuf> {
    use sign_out::kept_names::{KeptRole, event_folder_name, kept_relative_path};
    let mut writer = sign_out::kept_writer::KeptWriter::create(root, &event_folder_name(chrono::Local::now()), clone)?;
    let kept = match writer.keep(&kept_relative_path(Some(rel_path), op_id, KeptRole::Newest), source) {
        Ok(kept) => kept,
        Err(error) => {
            writer.abandon();
            return Err(error.into());
        }
    };
    let event = writer.event().to_path_buf();
    let folder = match writer.finish() {
        Ok(folder) => folder,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&event);
            return Err(error.into());
        }
    };
    let kept_text = kept.to_string_lossy().into_owned();
    if let Err(error) = sign_out::kept_record::save_conflict_copy_durably(config_path, &kept_text, config_fs) {
        let _ = std::fs::remove_dir_all(&folder.path);
        return Err(anyhow::anyhow!("record the kept copy: {error}"));
    }
    Ok(kept)
}

/// Task 2190 (review-169 blocker): "Show in Finder" reveals only a copy the list holds.
fn conflict_copy_reveal_target<'a>(listed: &[String], path: &'a str) -> Result<&'a str, String> {
    if listed.iter().any(|copy| copy == path) {
        Ok(path)
    } else {
        Err(sign_out::copy::K3_TOAST.to_string())
    }
}

/// Task 2190: every copy "Keep the other" kept, oldest first, for Settings › Sync.
#[tauri::command]
fn kept_conflict_copies() -> Result<Vec<String>, String> {
    sign_out::kept_record::saved_conflict_copies_at(&DesktopConfig::path()?)
}

/// Task 2190: "Show in Finder" for one kept copy.
#[tauri::command]
fn show_kept_conflict_copy(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let listed = sign_out::kept_record::saved_conflict_copies_at(&DesktopConfig::path()?)?;
    let target = conflict_copy_reveal_target(&listed, &path)?;
    tracing::info!("kept copy reveal requested");
    app.opener()
        .reveal_item_in_dir(target)
        .map_err(|_| sign_out::copy::K3_TOAST.to_string())
}

/// Task 2190: "Dismiss" forgets one kept copy from the list, never deletes it.
#[tauri::command]
fn dismiss_kept_conflict_copy(app: tauri::AppHandle, path: String) -> Result<bool, String> {
    let dismissed = sign_out::kept_record::dismiss_saved_conflict_copy_at(&DesktopConfig::path()?, &path)?;
    emit_kept_folder_changed(&app);
    Ok(dismissed)
}

/// Task 2190 (plan §3.4): both sides of a parked conflict, for the conflict window. Read-only.
#[tauri::command]
async fn parked_conflict_preview(
    state: State<'_, AppState>,
    op_id: String,
) -> Result<engine_bridge::ConflictContentPreview, String> {
    session_command!(async {
        let bridge = parked_conflict_bridge(&state)?;
        bridge.parked_conflict_preview(&op_id).await.map_err(|e| e.to_string())
    })
}

/// Task 2190 (plan §3.2): the person's decision on a parked conflict ("Keep this Mac's", "Keep the
/// other", "Keep both"). A decision that changed something signals the File Provider, so Finder
/// reads the item again; `Moved` changed nothing (the window asks the person to decide again).
#[tauri::command]
async fn resolve_parked_conflict(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    op_id: String,
    choice: engine_bridge::ParkedChoice,
    shown_version: Option<i64>,
) -> Result<engine_bridge::ParkedResolution, String> {
    session_command!(async {
        let bridge = parked_conflict_bridge(&state)?;
        let file_id = bridge.db().get_operation(&op_id).ok().flatten().and_then(|op| op.file_id);
        let device = hostname::get()
            .map(|name| name.to_string_lossy().trim_end_matches(".local").to_string())
            .unwrap_or_else(|_| "this Mac".to_string());
        let hhmm = chrono::Local::now().format("%H.%M").to_string();
        #[cfg(target_os = "macos")]
        let mut keep_copy = |source: &std::path::Path, rel: &str| keep_parked_copy(&op_id, source, rel);
        #[cfg(not(target_os = "macos"))]
        let mut keep_copy = |_: &std::path::Path, _: &str| -> anyhow::Result<std::path::PathBuf> {
            anyhow::bail!("this decision is not available on this platform yet")
        };
        let done = bridge
            .resolve_parked(&op_id, choice, shown_version, &mut keep_copy, &device, &hhmm)
            .map_err(|e| e.to_string())?;
        // The signal goes through the bridge gate. One it does not deliver (the gate is busy, or the
        // signal fails) is owed to the next tick's signal, so Finder is never left stale.
        #[cfg(target_os = "macos")]
        if done != engine_bridge::ParkedResolution::Moved {
            let mut ids: Vec<String> = file_id.into_iter().collect();
            if let engine_bridge::ParkedResolution::KeptBoth { file_id: copy, .. } = &done {
                ids.push(copy.clone());
            }
            if crate::finder_setup::macos_ports::signal_working_set().is_err() {
                crate::runner::owe_working_set_signal(&ids);
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = file_id;
        // Review-169 #8: an open Sync tab reads its kept rows again (a new kept copy, a dropped write).
        if done != engine_bridge::ParkedResolution::Moved {
            emit_kept_folder_changed(&app);
        }
        let _ = app.emit("conflict-resolved", serde_json::json!({ "op_id": op_id }));
        Ok(done)
    })
}

/// Real content preview for the conflict-resolution window (task 1546 finding
/// 2), replacing ConflictWindow.tsx's previous hardcoded placeholder diff
/// body. Read-only — does not resolve anything or touch state.db; see
/// [`engine_bridge::EngineBridge::conflict_content_preview`] for why.
///
/// Same session/sync-root/bridge-construction pattern as `resolve_conflict`
/// just above.
#[tauri::command]
async fn conflict_content_preview(
    state: State<'_, AppState>,
    file_id: String,
) -> Result<engine_bridge::ConflictContentPreview, String> {
    session_command!(async {
        let acct = state.active_account()?;
        let (token, master_key) = {
            let guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
            match guard.as_ref() {
                Some(s) => (s.token.clone(), s.master_key),
                None => return Err("not signed in".into()),
            }
        };

        let cfg = DesktopConfig::load()?;
        let sync_root = cfg.sync_root.ok_or_else(|| "no sync root configured".to_string())?;

        let db_path = state_paths::beebeeb_state_dir()?.join(state_paths::STATE_DB_FILENAME);
        let db = std::sync::Arc::new(state_db::StateDb::open(&db_path).map_err(|e| format!("open state.db: {e}"))?);
        let api = std::sync::Arc::new(api_client::ApiClient::new(runner::api_base_url(), token, master_key));
        let bridge = engine_bridge::EngineBridge::new(db, api);

        bridge
            .conflict_content_preview(&file_id, &sync_root)
            .await
            .map_err(|e| e.to_string())
    })
}

// ── IPC commands: app metadata ────────────────────────────────────────────────

// ── Durations ─────────────────────────────────────────────────────────────────

/// Delay after launch before the first update check (don't block startup).
const UPDATE_CHECK_STARTUP_DELAY_SECS: u64 = 5;
/// How often to re-check for updates while the app is running.
const UPDATE_CHECK_INTERVAL_HOURS: u64 = 4;
const UPDATE_MANIFEST_BASE_URL: &str = "https://releases.beebeeb.io";
const DESKTOP_RELEASE_TAG_PREFIX: &str = "desktop-v";
const DESKTOP_RELEASE_REPO_URL: &str = "https://github.com/beebeeb-io/desktop/releases/tag";

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedReleaseVersion {
    major: u64,
    minor: u64,
    patch: u64,
    prerelease: Option<Vec<PrereleasePart>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PrereleasePart {
    Numeric(u64),
    Text(String),
}

impl Ord for PrereleasePart {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (Self::Numeric(a), Self::Numeric(b)) => a.cmp(b),
            (Self::Numeric(_), Self::Text(_)) => std::cmp::Ordering::Less,
            (Self::Text(_), Self::Numeric(_)) => std::cmp::Ordering::Greater,
            (Self::Text(a), Self::Text(b)) => a.cmp(b),
        }
    }
}

impl PartialOrd for PrereleasePart {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ParsedReleaseVersion {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.major
            .cmp(&other.major)
            .then_with(|| self.minor.cmp(&other.minor))
            .then_with(|| self.patch.cmp(&other.patch))
            .then_with(|| match (&self.prerelease, &other.prerelease) {
                (None, None) => std::cmp::Ordering::Equal,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (Some(_), None) => std::cmp::Ordering::Less,
                (Some(a), Some(b)) => compare_prerelease_parts(a, b),
            })
    }
}

impl PartialOrd for ParsedReleaseVersion {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

fn compare_prerelease_parts(a: &[PrereleasePart], b: &[PrereleasePart]) -> std::cmp::Ordering {
    for (left, right) in a.iter().zip(b.iter()) {
        let ordering = left.cmp(right);
        if !ordering.is_eq() {
            return ordering;
        }
    }
    a.len().cmp(&b.len())
}

fn parse_release_version(version: &str) -> Option<ParsedReleaseVersion> {
    let version_without_build = version.split_once('+').map(|(v, _)| v).unwrap_or(version);
    let (core, prerelease) = version_without_build
        .split_once('-')
        .map_or((version_without_build, None), |(core, pre)| (core, Some(pre)));

    let mut core_parts = core.split('.');
    let major = core_parts.next()?.parse::<u64>().ok()?;
    let minor = core_parts.next()?.parse::<u64>().ok()?;
    let patch = core_parts.next()?.parse::<u64>().ok()?;
    if core_parts.next().is_some() {
        return None;
    }

    let prerelease = match prerelease {
        Some(pre) if pre.is_empty() => return None,
        Some(pre) => {
            let mut parts = Vec::new();
            for part in pre.split('.') {
                if part.is_empty() {
                    return None;
                }
                if part.as_bytes().iter().all(u8::is_ascii_digit) {
                    parts.push(PrereleasePart::Numeric(part.parse::<u64>().ok()?));
                } else {
                    parts.push(PrereleasePart::Text(part.to_string()));
                }
            }
            Some(parts)
        }
        None => None,
    };

    Some(ParsedReleaseVersion {
        major,
        minor,
        patch,
        prerelease,
    })
}

fn release_channel_from_version(version: &str) -> ReleaseChannel {
    let version_without_build = version.split_once('+').map(|(v, _)| v).unwrap_or(version);
    let Some((_, prerelease)) = version_without_build.split_once('-') else {
        return ReleaseChannel::Stable;
    };
    match prerelease.split('.').next() {
        Some("alpha") => ReleaseChannel::Alpha,
        Some("beta") => ReleaseChannel::Beta,
        _ => ReleaseChannel::Stable,
    }
}

fn desktop_manifest_path_for_channel(channel: ReleaseChannel) -> &'static str {
    match channel {
        ReleaseChannel::Stable => "desktop/latest.json",
        ReleaseChannel::Beta => "desktop/beta.json",
        ReleaseChannel::Alpha => "desktop/alpha.json",
    }
}

fn update_manifest_url_for_channel(channel: ReleaseChannel) -> String {
    format!(
        "{UPDATE_MANIFEST_BASE_URL}/{}",
        desktop_manifest_path_for_channel(channel)
    )
}

fn release_notes_url_for_version(version: &str) -> String {
    format!("{DESKTOP_RELEASE_REPO_URL}/{DESKTOP_RELEASE_TAG_PREFIX}{version}")
}

fn compare_channel_release_to_current(current_version: &str, remote_version: &str) -> Option<std::cmp::Ordering> {
    Some(parse_release_version(remote_version)?.cmp(&parse_release_version(current_version)?))
}

fn is_channel_downgrade(current_version: &str, remote_version: &str) -> bool {
    compare_channel_release_to_current(current_version, remote_version) == Some(std::cmp::Ordering::Less)
}

/// Return true only when the selected channel's manifest points to a version
/// newer than the installed app. Background checks deliberately hold the current
/// build until the selected channel catches up; the manual Settings path uses a
/// separate manifest check so it can report older channel builds as downgrades.
fn should_offer_channel_update(current_version: &str, remote_version: &str) -> bool {
    match compare_channel_release_to_current(current_version, remote_version) {
        Some(ordering) => ordering == std::cmp::Ordering::Greater,
        _ => remote_version != current_version,
    }
}

fn configured_release_channel() -> ReleaseChannel {
    match DesktopConfig::load() {
        Ok(cfg) => cfg.release_channel,
        Err(error) => {
            tracing::warn!(%error, "could not load desktop release channel; defaulting updater to stable");
            ReleaseChannel::Stable
        }
    }
}

fn installed_release_channel_from_config(config: Result<DesktopConfig, String>, version: &str) -> ReleaseChannel {
    match config {
        Ok(cfg) => cfg
            .installed_release_channel
            .unwrap_or_else(|| release_channel_from_version(version)),
        Err(error) => {
            tracing::warn!(%error, "could not load installed release channel; deriving legacy channel from version");
            release_channel_from_version(version)
        }
    }
}

fn installed_release_channel() -> ReleaseChannel {
    installed_release_channel_from_config(DesktopConfig::load(), real_app_version())
}

fn record_installed_release_channel(channel: ReleaseChannel) -> Result<(), String> {
    DesktopConfig::update(|cfg| {
        cfg.installed_release_channel = Some(channel);
        (true, ())
    })
}

fn updater_builder_for_release_channel(
    app: &tauri::AppHandle,
    channel: ReleaseChannel,
) -> Result<tauri_plugin_updater::UpdaterBuilder, String> {
    use tauri_plugin_updater::UpdaterExt;

    let endpoint = update_manifest_url_for_channel(channel);
    app.updater_builder()
        .endpoints(vec![
            endpoint
                .parse()
                .map_err(|e| format!("parse update manifest URL {endpoint}: {e}"))?,
        ])
        .map_err(|e| e.to_string())
}

fn updater_for_release_channel(
    app: &tauri::AppHandle,
    channel: ReleaseChannel,
) -> Result<tauri_plugin_updater::Updater, String> {
    updater_builder_for_release_channel(app, channel)?
        .version_comparator(|_current, release| {
            // Deliberately ignore Tauri's own `_current` and compare against
            // real_app_version(), the workflow-provided release version baked
            // in at compile time. That keeps comparisons aligned with the
            // channel manifest and preserves legacy prerelease ordering for
            // users upgrading from pre-build-once releases.
            should_offer_channel_update(real_app_version(), &release.version.to_string())
        })
        .build()
        .map_err(|e| e.to_string())
}

fn manifest_updater_for_release_channel(
    app: &tauri::AppHandle,
    channel: ReleaseChannel,
) -> Result<tauri_plugin_updater::Updater, String> {
    updater_builder_for_release_channel(app, channel)?
        // Manual checks need to see the selected channel's latest manifest
        // version even when it is equal to or older than the running app.
        .version_comparator(|_current, _release| true)
        .build()
        .map_err(|e| e.to_string())
}

/// The real release version displayed by the app and compared against channel
/// manifests. Set by release.yml as $BEEBEEB_RELEASE_VERSION before `cargo
/// build` runs; falls back to the Cargo package version for local/dev builds
/// where CI hasn't set it.
fn real_app_version() -> &'static str {
    option_env!("BEEBEEB_RELEASE_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct UpdateAvailablePayload {
    version: String,
    channel: String,
    body: String,
    release_notes_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum ManualUpdateCheckResult {
    UpToDate {
        current_version: String,
        channel: String,
    },
    UpdateAvailable {
        current_version: String,
        channel: String,
        version: String,
        body: String,
        release_notes_url: String,
    },
    DowngradeAvailable {
        current_version: String,
        current_channel: String,
        channel: String,
        version: String,
        body: String,
        release_notes_url: String,
    },
}

fn manual_update_up_to_date_result(channel: ReleaseChannel) -> ManualUpdateCheckResult {
    ManualUpdateCheckResult::UpToDate {
        current_version: real_app_version().to_string(),
        channel: channel.as_str().to_string(),
    }
}

fn manual_update_available_result(
    channel: ReleaseChannel,
    current_version: &str,
    version: String,
    body: Option<&str>,
) -> (ManualUpdateCheckResult, UpdateAvailablePayload) {
    let body = body.unwrap_or("").to_string();
    let release_notes_url = release_notes_url_for_version(&version);
    let channel = channel.as_str().to_string();
    let payload = UpdateAvailablePayload {
        version: version.clone(),
        channel: channel.clone(),
        body: body.clone(),
        release_notes_url: release_notes_url.clone(),
    };

    (
        ManualUpdateCheckResult::UpdateAvailable {
            current_version: current_version.to_string(),
            channel,
            version,
            body,
            release_notes_url,
        },
        payload,
    )
}

fn manual_update_downgrade_result(
    channel: ReleaseChannel,
    current_channel: ReleaseChannel,
    current_version: &str,
    version: String,
    body: Option<&str>,
) -> ManualUpdateCheckResult {
    ManualUpdateCheckResult::DowngradeAvailable {
        current_version: current_version.to_string(),
        current_channel: current_channel.as_str().to_string(),
        channel: channel.as_str().to_string(),
        version: version.clone(),
        body: body.unwrap_or("").to_string(),
        release_notes_url: release_notes_url_for_version(&version),
    }
}

fn manual_update_result_for_remote(
    channel: ReleaseChannel,
    current_channel: ReleaseChannel,
    current_version: &str,
    version: String,
    body: Option<&str>,
) -> (ManualUpdateCheckResult, Option<UpdateAvailablePayload>) {
    match compare_channel_release_to_current(current_version, &version) {
        Some(std::cmp::Ordering::Less) => (
            manual_update_downgrade_result(channel, current_channel, current_version, version, body),
            None,
        ),
        Some(std::cmp::Ordering::Equal) => (
            ManualUpdateCheckResult::UpToDate {
                current_version: current_version.to_string(),
                channel: channel.as_str().to_string(),
            },
            None,
        ),
        Some(std::cmp::Ordering::Greater) => {
            let (result, payload) = manual_update_available_result(channel, current_version, version, body);
            (result, Some(payload))
        }
        None if version == current_version => (
            ManualUpdateCheckResult::UpToDate {
                current_version: current_version.to_string(),
                channel: channel.as_str().to_string(),
            },
            None,
        ),
        None => {
            let (result, payload) = manual_update_available_result(channel, current_version, version, body);
            (result, Some(payload))
        }
    }
}

/// Returns the app version string — displayed in the web UI Settings page.
#[tauri::command]
fn app_version() -> &'static str {
    real_app_version()
}

/// Manually check the currently selected release channel and report an honest
/// status to Settings. If an update is available, emit the same banner event
/// used by the background updater so the existing one-click flow takes over.
#[tauri::command]
async fn check_for_updates_now(app: tauri::AppHandle) -> Result<ManualUpdateCheckResult, String> {
    let channel = configured_release_channel();
    let updater = manifest_updater_for_release_channel(&app, channel)
        .map_err(|e| format!("Could not prepare the {} update check: {e}", channel.as_str()))?;

    match updater.check().await {
        Ok(Some(update)) => {
            let version = update.version.to_string();
            let (result, payload) = manual_update_result_for_remote(
                channel,
                installed_release_channel(),
                real_app_version(),
                version.clone(),
                update.body.as_deref(),
            );

            match &result {
                ManualUpdateCheckResult::UpdateAvailable { .. } => {
                    tracing::info!(
                        version = %version,
                        channel = %channel.as_str(),
                        "manual update check found an update"
                    );
                }
                ManualUpdateCheckResult::DowngradeAvailable { .. } => {
                    tracing::info!(
                        current_version = %real_app_version(),
                        version = %version,
                        channel = %channel.as_str(),
                        "manual update check found a downgrade"
                    );
                }
                ManualUpdateCheckResult::UpToDate { .. } => {
                    tracing::info!(channel = %channel.as_str(), "manual update check found no update");
                }
            }

            if let Some(payload) = payload {
                macos_popover::note_update(&payload.channel, Some(&payload.version));
                let _ = app.emit("update-available", payload);
            } else {
                macos_popover::note_update(channel.as_str(), None);
            }

            Ok(result)
        }
        Ok(None) => {
            tracing::info!(
                channel = %channel.as_str(),
                "manual update check found no manifest release"
            );
            macos_popover::note_update(channel.as_str(), None);
            Ok(manual_update_up_to_date_result(channel))
        }
        Err(error) => Err(format!(
            "Could not reach the {} update manifest: {error}",
            channel.as_str()
        )),
    }
}

/// The code a refused in-place install leads with, so a caller can tell it from a failed one.
const UPDATE_INSTALL_UNSUPPORTED_CODE: &str = "update_install_unsupported";

/// Task 2176: the App Sandbox denies replacing the running bundle, so on a Mac an in-place install can only fail, after
/// the download, with an error nobody can diagnose. A Mac is offered a download instead (two-surface spec §0 ruling 5),
/// and both install commands refuse here, first, as defence in depth behind the window's own rule. Windows and Linux
/// keep their installers.
fn in_app_install_refusal(host: desktop_capabilities::HostOs) -> Result<(), String> {
    if host == desktop_capabilities::HostOs::Macos {
        return Err(format!(
            "{UPDATE_INSTALL_UNSUPPORTED_CODE}: Beebeeb on a Mac downloads updates and cannot install them in place."
        ));
    }
    Ok(())
}

/// Download and install the pending update, then relaunch.
///
/// Called by the frontend when the user clicks "Restart to update" in the
/// update-available banner (Windows and Linux; a Mac downloads, see `in_app_install_refusal`).
/// Returns an error string if installation fails.
#[tauri::command]
async fn install_update(app: tauri::AppHandle) -> Result<(), String> {
    in_app_install_refusal(desktop_capabilities::host_os())?;
    let channel = configured_release_channel();
    let updater = updater_for_release_channel(&app, channel)?;
    let update = updater.check().await.map_err(|e| e.to_string())?;

    match update {
        Some(u) => {
            tracing::info!(version = %u.version, channel = %channel.as_str(), "downloading update");
            // Spec §8: the update's from and to versions go to the lifecycle log.
            let from_version = real_app_version().to_string();
            let to_version = u.version.to_string();
            let versions = (from_version.clone(), to_version.clone());

            u.download_and_install(
                |downloaded, total| {
                    // Emit progress so the frontend can show a progress bar.
                    // downloaded: usize, total: Option<u64>
                    let pct = total
                        .map(|t| if t > 0 { (downloaded as u64) * 100 / t } else { 0 })
                        .unwrap_or(0);
                    tracing::debug!("update download progress: {pct}%");
                },
                move || {
                    tracing::info!("update installed — relaunching");
                    lifecycle_log::event(lifecycle_log::LifecycleEvent::UpdateDownloaded {
                        from: versions.0,
                        to: versions.1,
                    });
                },
            )
            .await
            .map_err(|e| e.to_string())?;
            lifecycle_log::event(lifecycle_log::LifecycleEvent::UpdateInstalled {
                from: from_version,
                to: to_version,
            });

            record_installed_release_channel(channel).map_err(|e| format!("record installed release channel: {e}"))?;
            #[cfg(target_os = "macos")]
            note_updater_relaunch(&app);
            app.restart();
        }
        None => {
            tracing::info!("install_update called but no update available");
        }
    }

    Ok(())
}

/// Download and install the selected channel's currently advertised older
/// release. The version argument is checked against the manifest before any
/// install attempt so a stale Settings view cannot install a different build.
#[tauri::command]
async fn install_channel_downgrade(app: tauri::AppHandle, version: String) -> Result<(), String> {
    in_app_install_refusal(desktop_capabilities::host_os())?;
    let channel = configured_release_channel();
    let updater = manifest_updater_for_release_channel(&app, channel)?;
    let update = updater.check().await.map_err(|e| e.to_string())?;

    let Some(u) = update else {
        return Err(format!(
            "The {} channel does not currently advertise a desktop release.",
            channel.as_str()
        ));
    };

    if u.version != version {
        return Err(format!(
            "The {} channel now points to version {}. Check for updates again before downgrading.",
            channel.as_str(),
            u.version
        ));
    }

    if !is_channel_downgrade(real_app_version(), &u.version) {
        return Err(format!(
            "Version {} is not older than the running version {}.",
            u.version,
            real_app_version()
        ));
    }

    tracing::info!(
        current_version = %real_app_version(),
        version = %u.version,
        channel = %channel.as_str(),
        "downloading downgrade"
    );
    // Spec §8: the downgrade's from and to versions go to the lifecycle log.
    let from_version = real_app_version().to_string();
    let to_version = u.version.to_string();
    let versions = (from_version.clone(), to_version.clone());

    u.download_and_install(
        |downloaded, total| {
            let pct = total
                .map(|t| if t > 0 { (downloaded as u64) * 100 / t } else { 0 })
                .unwrap_or(0);
            tracing::debug!("downgrade download progress: {pct}%");
        },
        move || {
            tracing::info!("downgrade installed — relaunching");
            lifecycle_log::event(lifecycle_log::LifecycleEvent::UpdateDownloaded {
                from: versions.0,
                to: versions.1,
            });
        },
    )
    .await
    .map_err(|e| e.to_string())?;
    lifecycle_log::event(lifecycle_log::LifecycleEvent::UpdateInstalled {
        from: from_version,
        to: to_version,
    });

    record_installed_release_channel(channel).map_err(|e| format!("record installed release channel: {e}"))?;
    #[cfg(target_os = "macos")]
    note_updater_relaunch(&app);
    app.restart()
}

/// Toggle "Start at login" autostart setting.
#[tauri::command]
fn toggle_autostart(app: tauri::AppHandle) -> Result<bool, String> {
    let manager = app.autolaunch();
    let currently_enabled = manager.is_enabled().map_err(|e| e.to_string())?;
    if currently_enabled {
        manager.disable().map_err(|e| e.to_string())?;
        tracing::info!("autostart disabled");
    } else {
        manager.enable().map_err(|e| e.to_string())?;
        tracing::info!("autostart enabled");
    }
    Ok(!currently_enabled)
}

/// Return the current autostart state (for the frontend settings page).
#[tauri::command]
fn autostart_enabled(app: tauri::AppHandle) -> Result<bool, String> {
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

// ── Entry point ───────────────────────────────────────────────────────────────

const WINDOWS_TRAY_FLYOUT_WIDTH: u32 = 400;
const WINDOWS_TRAY_FLYOUT_HEIGHT: u32 = 560;
const WINDOWS_TRAY_FLYOUT_GAP: i32 = 10;

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn windows_tray_flyout_position(click_x: f64, click_y: f64, scale_factor: f64) -> tauri::PhysicalPosition<i32> {
    let scale_factor = valid_windows_tray_scale_factor(scale_factor);
    let physical_width = (WINDOWS_TRAY_FLYOUT_WIDTH as f64 * scale_factor).round() as i32;
    let physical_height = (WINDOWS_TRAY_FLYOUT_HEIGHT as f64 * scale_factor).round() as i32;
    let physical_gap = (WINDOWS_TRAY_FLYOUT_GAP as f64 * scale_factor).round() as i32;

    tauri::PhysicalPosition {
        x: (click_x.round() as i32).saturating_sub(physical_width).max(0),
        y: (click_y.round() as i32).saturating_sub(physical_height.saturating_add(physical_gap)),
    }
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn valid_windows_tray_scale_factor(scale_factor: f64) -> f64 {
    if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    }
}

#[cfg(target_os = "windows")]
fn windows_tray_flyout_scale_factor(win: &tauri::WebviewWindow, click_x: f64, click_y: f64) -> f64 {
    let click_monitor_scale = win
        .monitor_from_point(click_x, click_y)
        .ok()
        .flatten()
        .map(|monitor| monitor.scale_factor())
        .filter(|scale_factor| scale_factor.is_finite() && *scale_factor > 0.0);

    click_monitor_scale
        .or_else(|| {
            win.scale_factor()
                .ok()
                .filter(|scale_factor| scale_factor.is_finite() && *scale_factor > 0.0)
        })
        .unwrap_or(1.0)
}

// ── macOS tray flyout anchoring (task 1384) ────────────────────────────────
//
// The compact "settings" window (App.tsx) doubles as the macOS/Linux tray
// flyout. Pre-1384 it always opened decorated and `.center()`-ed — a
// 680×540pt window landing mid-screen instead of under the status item.
// macOS had its own anchor math here, anchored from the tray click event's
// `rect`. It retired with the menu-bar popover (task 1683 slice 3, review P3-7
// of #160): the tray click opens the popover, placed by `surfaces::anchor`, and
// nothing hands this window an anchor any more. The statics and the `Popover`
// mode below remain until GL-0009 retires them. Linux keeps the original
// centered/decorated behaviour untouched (see the `#[cfg(not(any(
// target_os = "windows", target_os = "macos")))]` branch in
// `setup_tray`'s click handler, and the `#[cfg(not(target_os = "macos"))]`
// branch in `show_compact_app_window_with_nav` below).

const MACOS_SETTINGS_WINDOW_WIDTH: f64 = 680.0;
const MACOS_SETTINGS_WINDOW_HEIGHT: f64 = 540.0;

/// A one-shot handoff to `show_compact_app_window_with_nav`: the physical
/// position the flyout should open at. The tray click handler set it before
/// the menu-bar popover replaced the flyout; nothing sets it now (review P3-7
/// of #160), so every open reads `None` until GL-0009 retires it. `take()`n
/// (consumed) the moment it's read, so an unrelated open of the same
/// window (the "Open Beebeeb" menu item, a notification, a deep link) that
/// didn't go through the tray click falls back to centering instead of
/// reusing a stale tray position.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
static MACOS_TRAY_FLYOUT_ANCHOR: LazyLock<Mutex<Option<tauri::PhysicalPosition<i32>>>> =
    LazyLock::new(|| Mutex::new(None));

/// Debounces the focus-loss-hide handler against the spurious `Focused(false)`
/// some window managers deliver right as a borderless always-on-top window
/// is created/shown, before it has actually become key — only hide once a
/// real `Focused(true)` was observed since the last show.
#[cfg(target_os = "macos")]
static MACOS_FLYOUT_GAINED_FOCUS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the shared macOS settings window is currently behaving as a
/// tray-anchored popover or as a regular window. Set every time
/// `show_compact_app_window_with_nav` shows the window (same one-shot-handoff
/// family as `MACOS_TRAY_FLYOUT_ANCHOR`/`MACOS_FLYOUT_GAINED_FOCUS`), and read
/// live by the focus-loss handler registered once at window-build time — the
/// window is reused for every subsequent open, so the handler cannot rely on
/// a mode captured in its closure at build time; it must consult this flag
/// each time focus changes (1384 review follow-up: only the tray-click path
/// should hide on focus loss and open chrome-less; "Open Beebeeb", the
/// Preferences/Settings menu items, and `show_main_app_window` must produce
/// the previous regular, decorated, centered, persistent window).
#[cfg(target_os = "macos")]
static MACOS_FLYOUT_IS_POPOVER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Which chrome/behavior the shared macOS settings window should adopt for
/// this open. Pure decision, extracted from `show_compact_app_window_with_nav`
/// so it's unit-testable without a live `AppHandle`/`WebviewWindow`: an
/// anchor is present only when one was handed off through
/// `MACOS_TRAY_FLYOUT_ANCHOR` (consumed via `take()`; nothing sets it since
/// the menu-bar popover, so today every open is `Regular`); every other caller
/// ("Open Beebeeb", Preferences/Settings menu items, a notification, a deep
/// link) passes `None` and gets the regular window treatment.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MacosWindowMode {
    /// Chrome-less, always-on-top, positioned under the status item, hides
    /// on focus loss — reads as a system popover.
    Popover,
    /// Decorated, centered, normal window-manager behavior, stays open when
    /// focus moves elsewhere.
    Regular,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn macos_window_mode_for_anchor(anchor: Option<tauri::PhysicalPosition<i32>>) -> MacosWindowMode {
    match anchor {
        Some(_) => MacosWindowMode::Popover,
        None => MacosWindowMode::Regular,
    }
}

// ── The Beebeeb window on a Mac (task 1683 slice 1) ──────────────────────────
//
// Two-surface spec `docs/specs/2026-10-24-macos-two-surface-app.md` §4, §7, §9: one window, label `main`, built on
// demand from `tauri.macos.conf.json` (`create: false`) and destroyed when closed (`surfaces::policy::close_policy`).
// It replaced the anchored Settings window (`macos-settings`), whose placement code retired with it. Every opener
// shows and focuses the one window and navigates it through one request: Rust stores the latest `MainNav`, emits
// `main:navigate`, and the window pulls it with `consume_main_navigation`. The window also pulls once as soon as it
// listens, so a window built for the request still gets it.

/// The event that tells the Beebeeb window a request is waiting (`src/mainWindowModel.ts` `MAIN_NAVIGATE_EVENT`).
const MAIN_NAVIGATE_EVENT: &str = "main:navigate";

/// One request to the Beebeeb window, serialised as `src/mainWindowModel.ts`'s `MainNav` reads it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "to", rename_all = "lowercase")]
enum MainNav {
    /// Home, and out of Settings (⌘1).
    Home,
    /// Settings on a section (`account`, `general`, `sync`, `about`); `None` opens the last section used in this
    /// window, else General (§7.8).
    Settings { section: Option<&'static str> },
    /// Out of Settings, to where it was entered from (⌘[, View › Back).
    Back,
    /// “Sign in again”, asked for by `open_reauth_window`: the window shows that card whatever the session says, as
    /// Linux's onboarding window does (`mode=reauth`), so the click always lands on it (review P3-1).
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    Reauth,
    /// The popover found no local vault key: continue in the same Set up this Mac card.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    Setup,
}

impl MainNav {
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    const fn settings(section: Option<&'static str>) -> MainNav {
        MainNav::Settings { section }
    }
}

/// The latest request, kept until the window pulls it: one slot, so a burst of openers lands on the last one.
static MAIN_NAV_PENDING: std::sync::Mutex<Option<MainNav>> = std::sync::Mutex::new(None);

fn store_main_nav(slot: &std::sync::Mutex<Option<MainNav>>, nav: MainNav) {
    *slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(nav);
}

fn take_main_nav(slot: &std::sync::Mutex<Option<MainNav>>, label: &str) -> Option<MainNav> {
    if label != surfaces::policy::main_window_label(surfaces::policy::Platform::current()) {
        return None;
    }
    slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take()
}

/// The Beebeeb window pulls the request it was told about (or the one it was built for).
#[tauri::command]
fn consume_main_navigation(window: tauri::WebviewWindow) -> Option<MainNav> {
    take_main_nav(&MAIN_NAV_PENDING, window.label())
}

/// Opens (or shows and focuses) the Beebeeb window and asks it to show `nav`.
#[cfg(target_os = "macos")]
fn show_main_window(app: &tauri::AppHandle, nav: MainNav) {
    macos_popover::hide(app);
    let label = surfaces::policy::main_window_label(surfaces::policy::Platform::current());
    store_main_nav(&MAIN_NAV_PENDING, nav);
    // §9.1: the Dock icon (and with it the app menu) comes first, so it is there when the window is.
    set_macos_dock_icon(app, true);
    if let Some(win) = app.get_webview_window(label) {
        unhide_app(app);
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
        let _ = win.emit(MAIN_NAVIGATE_EVENT, ());
        return;
    }
    let Some(config) = app.config().app.windows.iter().find(|window| window.label == label) else {
        tracing::warn!("main window: no tauri.macos.conf.json entry named {label}; cannot open the Beebeeb window");
        refresh_macos_dock_icon(app, None);
        return;
    };
    match tauri::WebviewWindowBuilder::from_config(app, config).and_then(|builder| builder.build()) {
        Ok(win) => {
            place_main_window(app, &win, (config.width, config.height));
            unhide_app(app);
            let _ = win.show();
            let _ = win.set_focus();
        }
        Err(error) => {
            tracing::warn!(%error, "failed to create the Beebeeb window");
            refresh_macos_dock_icon(app, None);
        }
    }
}

/// Unhides the app (`AppHandle::show`, NSApp `unhide:`), which the tray's Hide and ⌘H hide. Not a window: a window
/// shown in a hidden app stays hidden, and tao's `set_focus` does nothing there, so every opener on a Mac calls this
/// before it shows its window (review round 2, P2-2).
#[cfg(target_os = "macos")]
fn unhide_app(app: &tauri::AppHandle) {
    let _ = app.show();
}

/// `RunEvent::Reopen` (double-clicking Beebeeb.app, Spotlight, the Dock; spec §7.3): the window as it was, or a new one
/// on Home. With the window closed the app lives on in the menu bar, so this is the way back (review round 2, P2-1).
/// Which of the two is `surfaces::launch::reopen_action`.
#[cfg(target_os = "macos")]
fn reopen_main_window(app: &tauri::AppHandle) {
    let label = surfaces::policy::main_window_label(surfaces::policy::Platform::current());
    let window = app.get_webview_window(label);
    match (surfaces::launch::reopen_action(window.is_some()), window) {
        (surfaces::launch::ReopenAction::BringForward, Some(win)) => {
            set_macos_dock_icon(app, true);
            unhide_app(app);
            let _ = win.unminimize();
            let _ = win.show();
            let _ = win.set_focus();
        }
        _ => show_main_window(app, MainNav::Home),
    }
}

/// How this launch happened (two-surface spec §7.3, task 1683 slice 6): the login item's signal (the one seam task 1845
/// plugs into, `surfaces::launch::login_item_signal`) and the flag the updater left just before it restarted the app,
/// which this launch consumes. A Mac only: Windows and Linux are launched the way they always were.
#[cfg(target_os = "macos")]
fn launch_source_now() -> surfaces::launch::LaunchSource {
    let now = u64::try_from(now_unix_seconds()).unwrap_or(0);
    let flag = state_paths::beebeeb_state_dir()
        .ok()
        .and_then(|dir| surfaces::launch::take_relaunch_flag(&dir, now));
    surfaces::launch::launch_source(surfaces::launch::login_item_signal(), flag, now)
}

#[cfg(not(target_os = "macos"))]
fn launch_source_now() -> surfaces::launch::LaunchSource {
    surfaces::launch::LaunchSource::Manual
}

/// Leaves the flag the next launch reads, just before the updater restarts the app: whether the Beebeeb window is open and
/// whether an account is signed in. A failure is logged and the restart goes on; the next launch then opens what a manual
/// one does.
#[cfg(target_os = "macos")]
fn note_updater_relaunch(app: &tauri::AppHandle) {
    let label = surfaces::policy::main_window_label(surfaces::policy::Platform::current());
    let window_open = app.get_webview_window(label).is_some();
    let state = app.state::<AppState>();
    let vault_unlocked = state
        .active_account()
        .ok()
        .and_then(|acct| acct.session.lock().ok().map(|guard| guard.is_some()))
        .unwrap_or(false);
    let auth_present = state.auth_present.lock().map(|guard| *guard).unwrap_or(false);
    let flag = surfaces::launch::RelaunchFlag {
        written_at: u64::try_from(now_unix_seconds()).unwrap_or(0),
        window_open,
        signed_in: vault_unlocked || auth_present,
    };
    let written = state_paths::beebeeb_state_dir()
        .and_then(|dir| surfaces::launch::write_relaunch_flag(&dir, &flag).map_err(|e| e.to_string()));
    if let Err(error) = written {
        tracing::warn!(%error, "could not leave the relaunch flag; the next launch opens as a manual one");
    }
}

/// View › Back (⌘[): only an open window has somewhere to go back from; it never opens one.
#[cfg(target_os = "macos")]
fn back_in_main_window(app: &tauri::AppHandle) {
    let label = surfaces::policy::main_window_label(surfaces::policy::Platform::current());
    if let Some(win) = app.get_webview_window(label) {
        store_main_nav(&MAIN_NAV_PENDING, MainNav::Back);
        let _ = win.emit(MAIN_NAVIGATE_EVENT, ());
    }
}

/// The displays in logical points (each one's physical frame over its own scale, which each `Display` keeps), and
/// the index of the primary one.
#[cfg(target_os = "macos")]
fn macos_displays(win: &tauri::WebviewWindow) -> (Vec<surfaces::window_state::Display>, usize) {
    use surfaces::window_state::{Display, PRect};
    let monitors = win.available_monitors().unwrap_or_default();
    let displays = monitors
        .iter()
        .map(|m| {
            Display::from_physical(
                PRect { x: m.position().x, y: m.position().y, w: m.size().width, h: m.size().height },
                PRect {
                    x: m.work_area().position.x,
                    y: m.work_area().position.y,
                    w: m.work_area().size.width,
                    h: m.work_area().size.height,
                },
                m.scale_factor(),
            )
        })
        .collect();
    let primary = win
        .primary_monitor()
        .ok()
        .flatten()
        .and_then(|p| monitors.iter().position(|m| m.name() == p.name() && m.position() == p.position()))
        .unwrap_or(0);
    (displays, primary)
}

/// The menu-bar icon's rect as tray-icon reports it (points × the icon's own scale).
#[cfg(target_os = "macos")]
fn macos_status_item_rect(app: &tauri::AppHandle) -> Option<surfaces::window_state::PRect> {
    let rect = app.tray_by_id("tray")?.rect().ok().flatten()?;
    let position = rect.position.to_physical::<f64>(1.0);
    let size = rect.size.to_physical::<f64>(1.0);
    Some(surfaces::window_state::PRect {
        x: position.x.round() as i32,
        y: position.y.round() as i32,
        w: size.width.round().max(0.0) as u32,
        h: size.height.round().max(0.0) as u32,
    })
}

/// Where the Beebeeb window's frame is kept (logical points), in the app's config directory.
#[cfg(target_os = "macos")]
fn main_frame_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join(surfaces::window_state::MAIN_FRAME_FILENAME))
}

/// The kept frame, read back. A missing, empty, partial or unreadable file, or one that is not a usable frame, is no
/// frame: the window then opens at its default place.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn read_kept_frame(path: &Path) -> Option<surfaces::window_state::LRect> {
    let bytes = std::fs::read(path).ok()?;
    surfaces::window_state::parse_kept_frame(std::str::from_utf8(&bytes).ok()?)
}

/// Writes the kept frame atomically: a temporary file next to it, then a rename over it, so a crash mid-write leaves
/// the previous frame, never half a file.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn write_kept_frame(path: &Path, frame: &surfaces::window_state::LRect) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, serde_json::to_vec(frame).map_err(std::io::Error::other)?)?;
    std::fs::rename(&temporary, path)
}

/// Keeps the Beebeeb window's frame, when it closes and when the app quits while it is open (§7.2). A full-screen
/// or zoomed window keeps nothing, so the last ordinary frame stays (review item 8).
#[cfg(target_os = "macos")]
fn save_main_window_frame(app: &tauri::AppHandle) {
    let label = surfaces::policy::main_window_label(surfaces::policy::Platform::current());
    let Some(win) = app.get_webview_window(label) else {
        return;
    };
    let (Ok(position), Ok(size), Ok(scale)) = (win.outer_position(), win.inner_size(), win.scale_factor()) else {
        return;
    };
    let fullscreen = win.is_fullscreen().unwrap_or(false);
    let zoomed = win.is_maximized().unwrap_or(false);
    let Some(frame) = surfaces::window_state::frame_to_keep(
        (position.x, position.y),
        (size.width, size.height),
        scale,
        fullscreen,
        zoomed,
    ) else {
        return;
    };
    let Some(path) = main_frame_path(app) else {
        return;
    };
    if let Err(error) = write_kept_frame(&path, &frame) {
        tracing::warn!(%error, "main window: its frame could not be kept");
    }
}

/// §7.2, planned in full before any of it is applied (review item 4): the kept frame when at least half of it is on a
/// display, else at its kept size (or `default_size`, the window's own config) centred on the menu-bar icon's display,
/// else on the primary one. Logical points throughout, so a mixed-DPI desk places it where it was (review item 5).
#[cfg(target_os = "macos")]
fn place_main_window(app: &tauri::AppHandle, win: &tauri::WebviewWindow, default_size: (f64, f64)) {
    use surfaces::window_state as placement;
    let (displays, primary) = macos_displays(win);
    let icon = macos_status_item_rect(app).and_then(|rect| placement::icon_display(rect, &displays));
    let kept = main_frame_path(app).and_then(|path| read_kept_frame(&path));
    let Some(frame) = placement::plan_main_window(kept, &displays, icon, primary, default_size) else {
        tracing::warn!("main window: no display to place it on");
        return;
    };
    if kept.is_some() && kept != Some(frame) {
        tracing::info!("main window: less than half of its kept frame is on a display; centred again");
    }
    let _ = win.set_size(tauri::Size::Logical(tauri::LogicalSize::new(frame.w, frame.h)));
    let _ = win.set_position(tauri::Position::Logical(tauri::LogicalPosition::new(frame.x, frame.y)));
}


/// The activation policy last applied: 0 unknown (the app starts Regular), 1 Dock icon, 2 none.
#[cfg(target_os = "macos")]
static MACOS_DOCK_ICON: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// §9.1: Regular (a Dock icon and the app menu) or Accessory (neither). Applied only when it changes.
#[cfg(target_os = "macos")]
fn set_macos_dock_icon(app: &tauri::AppHandle, wanted: bool) {
    let next = if wanted { 1 } else { 2 };
    if MACOS_DOCK_ICON.swap(next, Ordering::SeqCst) == next {
        return;
    }
    let policy = if wanted {
        tauri::ActivationPolicy::Regular
    } else {
        tauri::ActivationPolicy::Accessory
    };
    if let Err(error) = app.set_activation_policy(policy) {
        tracing::warn!(%error, wanted, "could not change the Dock icon");
    }
}

/// Hides a window whose close policy is Hide. On a Mac the popover goes through `macos_popover::hide`, which tells it,
/// so it stops reading its snapshot (self-review of #202: ⌘W on the popover hid it without telling it).
fn hide_on_close(window: &tauri::Window) {
    #[cfg(target_os = "macos")]
    if window.label() == "popover" {
        macos_popover::hide(window.app_handle());
        return;
    }
    let _ = window.hide();
}

/// The Dock icon for the windows that exist now, leaving out `gone` (a window being destroyed or hidden).
#[cfg(target_os = "macos")]
fn refresh_macos_dock_icon(app: &tauri::AppHandle, gone: Option<&str>) {
    let windows: Vec<(String, bool)> = app
        .webview_windows()
        .into_iter()
        .filter(|(label, _)| Some(label.as_str()) != gone)
        .map(|(label, window)| {
            let shown = window.is_visible().unwrap_or(false) || window.is_minimized().unwrap_or(false);
            (label, shown)
        })
        .collect();
    let wanted = surfaces::policy::dock_icon_wanted(windows.iter().map(|(label, shown)| (label.as_str(), *shown)));
    set_macos_dock_icon(app, wanted);
}

/// What the app menu may offer around the Beebeeb window (two-surface spec §8.2, F15): View › Back only while
/// Settings shows in an open window; View › Home, Beebeeb › Settings… and Check for Updates… (which opens Settings ›
/// About) always, except while a sheet is open, so ⌘1, ⌘[, ⌘, and ⇧⌘U cannot unmount Settings › Account under its
/// sheet (review item 6 chose disabling over a second way to answer the check).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MainMenuState {
    settings: bool,
    home: bool,
    back: bool,
    check_updates: bool,
}

#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn main_menu_state(main_open: bool, settings_showing: bool, sheet_open: bool) -> MainMenuState {
    let sheet = main_open && sheet_open;
    MainMenuState {
        settings: !sheet,
        home: !sheet,
        back: main_open && settings_showing && !sheet,
        check_updates: !sheet,
    }
}

#[cfg(target_os = "macos")]
fn apply_macos_main_menu_state(app: &tauri::AppHandle, state: MainMenuState) {
    let Some(items) = app.try_state::<MacosMenuItems>() else {
        return;
    };
    let _ = items.settings.set_enabled(state.settings);
    let _ = items.check_updates.set_enabled(state.check_updates);
    let _ = items.view_home.set_enabled(state.home);
    let _ = items.view_back.set_enabled(state.back);
}

/// The Beebeeb window says whether Settings shows and whether a sheet is open; the menu follows (§8.2, F15).
#[tauri::command]
fn main_window_state(app: tauri::AppHandle, window: tauri::WebviewWindow, settings_showing: bool, sheet_open: bool) {
    #[cfg(target_os = "macos")]
    if window.label() == surfaces::policy::main_window_label(surfaces::policy::Platform::current()) {
        apply_macos_main_menu_state(&app, main_menu_state(true, settings_showing, sheet_open));
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, window, settings_showing, sheet_open);
}

/// A window's life on a Mac, beside the global close handler: the Beebeeb window keeps its frame when it closes and
/// resets the menu when it is gone, and every window that comes, goes or hides sets the Dock icon again (§9.1).
#[cfg(target_os = "macos")]
fn macos_window_lifecycle(window: &tauri::Window, event: &tauri::WindowEvent) {
    let app = window.app_handle();
    let main = window.label() == surfaces::policy::main_window_label(surfaces::policy::Platform::current());
    match event {
        tauri::WindowEvent::CloseRequested { .. } if main => save_main_window_frame(app),
        tauri::WindowEvent::Destroyed => {
            if main {
                apply_macos_main_menu_state(app, main_menu_state(false, false, false));
            }
            refresh_macos_dock_icon(app, Some(window.label()));
        }
        tauri::WindowEvent::Focused(true) => refresh_macos_dock_icon(app, None),
        _ => {}
    }
}

/// The window-state plugin, on Windows and Linux only, with today's default file and denylist. A Mac does not run
/// it (review item 5 on PR #134): it stores physical numbers without their scale, which cannot be placed back on a
/// mixed-DPI desk, so `place_main_window` and `save_main_window_frame` keep the Beebeeb window's frame in points.
#[cfg(not(target_os = "macos"))]
fn window_state_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_window_state::Builder::default()
        .with_denylist(window_state_denylist_labels())
        .build()
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
fn window_state_denylist_labels() -> &'static [&'static str] {
    &["tray", "windows-onboarding"]
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .try_init();

    let builder = tauri::Builder::default()
        // Shared state — session installed via apply_session (the
        // browser-login handoff) after login. The sync engine task reads
        // from this.
        .manage(AppState::default())
        // Task 1683 slice 2: transfer board, last engine status, cached storage summary.
        .manage(popover_data::PopoverRuntime::default())
        .plugin(tauri_plugin_opener::init())
        // 0.8.15: before any window exists, so its `on_webview_ready` sees every webview (Ctrl+Shift+P printed in 0.8.14).
        .plugin(webview_browser_keys::plugin());
    // Windows and Linux keep the window-state plugin; a Mac keeps the Beebeeb window's frame itself (task 1683 slice 1,
    // review item 5; see `window_state_plugin`).
    #[cfg(not(target_os = "macos"))]
    let builder = builder.plugin(window_state_plugin());
    builder
        // Native folder picker for the first-launch sync-root chooser.
        .plugin(tauri_plugin_dialog::init())
        // Native OS notifications — Task 11. The first conflict
        // notification on macOS triggers Apple's "allow notifications?"
        // permission prompt; we surface that, not a custom UI.
        .plugin(tauri_plugin_notification::init())
        // Auto-updater — endpoints + pubkey configured in tauri.conf.json
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Autostart — LaunchAgent on macOS (no elevated privileges required),
        // registry key on Windows, .desktop file on Linux
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec![])))
        .invoke_handler(tauri::generate_handler![
            app_version,
            consume_main_navigation,
            set_pending_setup,
            pending_setup,
            #[cfg(target_os = "macos")]
            vault_unlock_route,
            main_window_state,
            check_for_updates_now,
            consume_menu_update_check,
            install_update,
            install_channel_downgrade,
            toggle_autostart,
            autostart_enabled,
            clear_session,
            unlock_vault,
            desktop_unlock_with_recovery_phrase,
            lock_vault,
            sync_status,
            popover_snapshot,
            macos_popover::popover_hide,
            macos_popover::popover_quit,
            macos_popover::popover_open_window,
            macos_popover::popover_retry_sync,
            macos_popover::popover_unlock,
            macos_popover::popover_gear_menu,
            desktop_storage_summary,
            free_up_space,
            export_diagnostics,
            list_version_conflict_center,
            dismiss_review_entry,
            list_file_versions,
            restore_file_version,
            get_sync_root,
            pick_sync_root,
            default_sync_root,
            desktop_platform,
            desktop_capabilities,
            finder_location_state,
            kept_unsynced_folder,
            dismiss_kept_unsynced_folder,
            // Spec 2026-10-10 §9: the kept folder's own row.
            kept_unsent_folder,
            dismiss_kept_unsent_folder,
            show_kept_unsent_folder,
            // Spec 2026-10-10 §5.1 and §6: the sign-out flow's count and send phase (a Mac; elsewhere they say so),
            // and the acknowledgement of a shown result (elsewhere nothing is pending, and it does nothing).
            sign_out_count,
            sign_out_unknown_files,
            sign_out_send_start,
            sign_out_send_stop,
            sign_out_result_ack,
            // §5.1: the compact page's and the menu's flow request, which Settings consumes (a Mac; elsewhere there is
            // no flow, and they say so).
            request_sign_out_flow,
            consume_sign_out_flow_request,
            // Spec 2026-10-06: the macOS Finder reconciler's commands (registered on every
            // platform; without a reconciler they say so).
            finder_setup_state,
            finder_setup_retry,
            finder_setup_copy_details,
            finder_setup_show_app,
            install_finder_location,
            continue_without_finder_location,
            // Task 1524 Issue 4 — user-disabled File Provider domain detection
            finder_domain_user_enabled,
            open_login_items_and_extensions_settings,
            // Windows shell integration (Cloud Files) — parallels the macOS finder cmds
            windows_shell_integration_state,
            install_windows_shell_integration,
            reset_macos_integration,
            open_finder_location,
            open_in_finder,
            // Task 9 — settings page IPC
            get_desktop_config,
            set_desktop_config,
            account_email,
            last_signed_in_email,
            // PKG-DATA — account / billing / devices / activity data layer.
            // Wrappers over existing server endpoints (the "all pages empty"
            // fix) plus the locally-computed storage breakdown.
            account_profile,
            account_profile_fresh,
            account_subscription,
            account_usage,
            account_region,
            account_set_region,
            account_activity,
            account_security_score,
            account_session_list,
            account_revoke_session,
            account_revoke_other_sessions,
            account_devices,
            account_client_sessions,
            account_activity_feed,
            account_notifications,
            account_notification_preferences,
            account_update_notification_preferences,
            desktop_confirm_action,
            desktop_trash_list,
            desktop_trash_restore,
            desktop_trash_delete_permanently,
            account_storage_breakdown,
            desktop_file_overview,
            desktop_recent_files,
            desktop_search_files,
            app_activity_snapshot,
            // Task 0090 — selective sync
            get_selective_sync,
            set_selective_sync,
            // Task 0797 — known-folder backup ("Manage backup")
            get_known_folder_backup,
            set_known_folder_backup,
            // Task 0804 — known-folder backup first-run onboarding prompt
            get_known_folder_onboarding_seen,
            mark_known_folder_onboarding_seen,
            get_menu_bar_hint_seen,
            mark_menu_bar_hint_seen,
            list_shared_roots,
            list_vault_folders,
            list_remote_tree,
            set_recursive_pin,
            // Task 12 — conflict window IPC
            open_conflict_window,
            resolve_conflict,
            // Task 1546 finding 2 — real (non-placeholder) conflict content preview
            conflict_content_preview,
            parked_conflict_preview,
            resolve_parked_conflict,
            kept_conflict_copies,
            show_kept_conflict_copy,
            dismiss_kept_conflict_copy,
            // Task 11 — native conflict notification
            notify_conflict,
            // First-launch onboarding (login + folder picker + sync status)
            desktop_login,
            // 2FA second-factor completion for OPAQUE sign-in
            desktop_login_2fa,
            // Browser-based device-code login handoff (Windows primary path)
            browser_login::start_browser_login,
            open_onboarding_window,
            open_reauth_window,
            quit_beebeeb,
            // PKG-SHELL — open the Windows main app window
            show_main_app_window,
            show_main_app_settings,
            // WS1 — Windows UI aliases + tray commands
            desktop_config,
            tray_recent_activity,
            tray_pause_sync,
            tray_resume_sync,
            sync_activity_summary,
            found_while_paused_review,
            found_while_paused_upload,
            found_while_paused_keep,
            show_kept_folder,
            upload_files_to_sync_root,
            create_folder_in_sync_root,
            report_problem,
            set_sync_mode,
            // Task 0812 — tray flyout: click file → reveal in Explorer + open
            reveal_and_open_file,
            // Task 0810 — Bandwidth view: speedtest + 20h traffic chart
            run_speedtest,
            get_bandwidth_history,
        ])
        .setup(|app| {
            let app_handle = app.handle().clone();
            state_paths::init_from_app(&app_handle).map_err(|e| tauri::Error::Anyhow(anyhow::anyhow!(e)))?;
            if let Ok(cfg) = DesktopConfig::load()
                && let Some(sync_root) = cfg.sync_root
                && let Err(error) = state_paths::prepare_beebeeb_state_dir_for_sync_root(&sync_root)
            {
                tracing::warn!(%error, sync_root = %sync_root.display(), "Beebeeb state migration did not complete during setup; runner will retry before opening state.db");
            }

            // Multi-account Phase 1 (decision 0800/0808): resolve the PERSISTED
            // account id, synthesize the single account under it, then lazily
            // migrate the legacy flat keychain trio into the id-segmented layout.
            // This ORDER is load-bearing and MUST run before anything reads
            // per-account credential state — the `auth_present` seed below, the
            // `restore_session_on_startup` task (which lands the keychain session
            // in `accounts[0].session`), and first-launch detection all depend on
            // both a resolvable active account AND the migration having run.
            //
            // Step 1 — persist/read the account id (mints + saves on first login,
            // returns the same value thereafter). On error we fall back to an
            // ephemeral id so the app still boots; the legacy READ FALLBACK in the
            // keychain helpers keeps auto-unlock working in that degraded case.
            let account_id = match DesktopConfig::ensure_account_id() {
                Ok(id) => id,
                Err(error) => {
                    tracing::warn!(%error, "could not persist account id; using ephemeral id this run");
                    AccountId::new_v4().0
                }
            };
            // Step 2 — synthesize the single account under the persisted id.
            // Idempotent; invisible at N=1.
            synthesize_single_account(&app.state::<AppState>(), AccountId(account_id.clone()));

            // Step 3 — lazy migration: legacy flat trio → id-segmented trio. Runs
            // SYNCHRONOUSLY here (decision 0808) BEFORE the auth seed + restore so
            // the auto-unlock below reads the segmented layout. Verify-before-delete
            // + the legacy READ FALLBACK are the two anti-lockout nets; a failure
            // here is LOGGED AND SWALLOWED (must not block startup — the fallback
            // keeps the vault usable).
            if let Err(error) = keychain::migrate_legacy_keychain_to_account(&account_id) {
                tracing::warn!(%error, "legacy keychain migration did not complete; legacy fallback remains in effect");
            }

            setup_native_menu(app)?;
            setup_tray(app)?;
            // Defensive: hide windows that older persisted state or other
            // startup paths may leave visible. Fixed auxiliary windows are now
            // excluded from window-state restore; main-app remains stateful.
            #[cfg(target_os = "windows")]
            {
                // macOS-labelled `settings` window is not used on Windows.
                if let Some(w) = app.get_webview_window("settings") {
                    let _ = w.hide();
                }
                // Tray popup — shown only on tray-icon click.
                if let Some(w) = app.get_webview_window("tray") {
                    let _ = w.hide();
                }
            }
            attach_tray_status_listener(&app.handle().clone());
            {
                let app_state = app.state::<AppState>();
                if let Ok(mut guard) = app_state.auth_present.lock() {
                    // Segmented under the persisted id (with the legacy READ
                    // FALLBACK inside `keychain_session_present` covering a not-yet-
                    // migrated install).
                    *guard = keychain_session_present(&account_id);
                }
            }

            // Spec 2026-10-06: the macOS Finder reconciler. One task for the app's life, created
            // before the restore below so the launch trigger always finds it.
            #[cfg(target_os = "macos")]
            {
                if let Some(dir) = lifecycle_log::default_dir() {
                    lifecycle_log::init(dir, lifecycle_known_names);
                }
                let launch = finder_setup::launch_location::current();
                lifecycle_log::event(lifecycle_log::LifecycleEvent::Launch {
                    app_version: real_app_version().to_string(),
                    macos_version: macos_version_string(),
                    launch_location: launch,
                });
                let persisted = DesktopConfig::load().ok().and_then(|cfg| cfg.finder_last_failure);
                let handle = finder_setup::driver::spawn(
                    finder_setup::macos_ports::MacosPorts::new(app.handle().clone()),
                    finder_setup::driver::SystemClock,
                    launch,
                    persisted,
                    finder_setup::policy::RetryPolicy::default(),
                );
                let _ = app.state::<AppState>().finder_setup.set(handle);
            }

            // Resume a fully unlocked session from the OS credential store.
            // When the per-user credential vault still holds the session token
            // AND the (DPAPI/Keychain-protected) raw master key, this loads the
            // key, unlocks the vault, and starts the engine — so quit+relaunch
            // returns to a working signed-in state with no recovery-phrase
            // prompt (the persistence guarantee users expect). When the master
            // key is genuinely absent, this is a no-op and onboarding still
            // routes to the recovery-phrase unlock step.
            {
                let h = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    restore_session_on_startup(&h).await;
                    // §5.3 (1): the launch check runs once the restore finished, whichever way it
                    // went, so it sees the keys the restore installed.
                    notify_finder(&h.state::<AppState>(), finder_setup::core::Trigger::Launch);
                });
            }

            // Task 1698 part 3 (closes 1696 / audit G8): sweep stale File
            // Provider domains at every app start — signed app context only
            // (the extension and the app both hold the identity; the
            // unsigned CLI cannot do this and never runs this hook).
            // Best-effort: a failure (no identity, transient NSFileProvider
            // Manager error) is logged and never blocks startup. Idempotent:
            // a clean registry removes nothing, and our own domain is never
            // a candidate.
            #[cfg(target_os = "macos")]
            {
                let alert_app = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    // Through the bridge gate like every other bridge call (lead ruling T8-gate-all).
                    match crate::finder_setup::macos_ports::cleanup_stale_domains().await {
                        Ok(cleanup) => {
                            if cleanup.removed_count() > 0 || !cleanup.skipped.is_empty() || !cleanup.ours_present {
                                tracing::info!(
                                    removed = cleanup.removed_count(),
                                    skipped = cleanup.skipped.len(),
                                    ours_present = cleanup.ours_present,
                                    preserved = cleanup.preserved_locations.len(),
                                    "stale File Provider domain sweep complete (task 1698)"
                                );
                            }
                            // Task 1882: a removed domain's un-synced files were kept;
                            // the alert names each folder (the log above only counts them).
                            for location in &cleanup.preserved_locations {
                                surface_kept_folder(&alert_app, Some(location));
                            }
                        }
                        Err(error) => {
                            tracing::warn!(
                                error = %error,
                                "stale File Provider domain sweep unavailable in this context (unsigned CLI gets -2001); \
                                 the next signed app start retries"
                            );
                        }
                    }
                });
            }

            // Spawn background update checker
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                run_update_loop(handle).await;
            });

            // First-launch detection: if no sync_root is configured and no
            // session is present, open the onboarding window automatically.
            // App windows stay hidden (visible:false in tauri.conf.json) until
            // onboarding or the configured-startup path opens the right one.
            let no_sync_root = DesktopConfig::load().map(|c| c.sync_root.is_none()).unwrap_or(true);
            // The decision is `surfaces::policy::startup_surface`: Windows and Linux ignore how the app was started; a Mac
            // follows it (task 1683 slice 6, two-surface spec §7.3). How it was started is `launch_source_now()`.
            let launch_source = launch_source_now();
            tracing::info!(?launch_source, no_sync_root, "launch");
            match surfaces::policy::startup_surface(surfaces::policy::Platform::current(), no_sync_root, launch_source) {
                surfaces::policy::StartupSurface::Onboarding => {
                    let h = app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        // Small delay so the tray and menu are fully initialised first.
                        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                        if let Err(e) = open_onboarding_window_impl(&h) {
                            tracing::warn!(error = %e, "failed to open onboarding window");
                        }
                    });
                }
                surfaces::policy::StartupSurface::MainWindow => {
                    // Already configured — show the main app window after startup
                    // settles. On Windows this is the full sidebar/content shell
                    // (`main-app`); on macOS/Linux `show_main_app_window_impl`
                    // delegates to the compact app window, so behaviour there
                    // is unchanged. The short delay mirrors onboarding and avoids
                    // racing Tauri's window setup.
                    let h = app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                        show_main_app_window_impl(&h);
                    });
                }
                // A Mac's login launch, or an update's relaunch with nothing open: the b in the menu bar is all there is.
                surfaces::policy::StartupSurface::Nothing => {
                    // The app starts Regular (a Dock icon and the app menu), and Accessory is otherwise applied only from window
                    // events: with no window to open, nothing would ever take them away (PR #146 review, P2).
                    #[cfg(target_os = "macos")]
                    set_macos_dock_icon(app.handle(), false);
                    tracing::info!("nothing opens by itself on this launch");
                }
            }

            Ok(())
        })
        .on_menu_event(|app, event| {
            if let Some(spec) = desktop_menu_spec(event.id().as_ref()) {
                handle_desktop_menu_action(app, spec);
            }
        })
        // Spec 2026-10-10 §5.1: a page load that starts in a window (a reload) ends its sign-out flow, as a close does.
        .on_page_load(|webview, payload| {
            #[cfg(target_os = "macos")]
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Started) {
                sign_out_flow_window_closed(webview.app_handle(), webview.label());
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (webview, payload);
        })
        // Red-dot close button: what happens is `surfaces::policy::close_policy`. Windows and Linux hide every
        // window (to the tray), as before. On a Mac the Beebeeb window (`main`) is destroyed (two-surface spec §4,
        // task 1683 slice 1) and every other window is hidden, as before.
        .on_window_event(|window, event| {
            // Spec §7: one read-only userEnabled check when the app becomes active.
            if let tauri::WindowEvent::Focused(true) = event
                && let Some(handle) = window.state::<AppState>().finder_setup.get()
            {
                handle.app_activated();
            }
            // Lead ruling 5 (Task 12): an unidentified session asks the server again when the app becomes active.
            #[cfg(not(target_os = "windows"))]
            if let tauri::WindowEvent::Focused(true) = event {
                tauri::async_runtime::spawn(on_app_activated(window.app_handle().clone()));
            }
            // Spec 2026-10-10 §5.1: a close (on macOS a hide), a destroy or a reload ends that window's sign-out flow.
            #[cfg(target_os = "macos")]
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                sign_out_flow_window_closed(window.app_handle(), window.label());
            }
            #[cfg(target_os = "macos")]
            if let tauri::WindowEvent::Destroyed = event {
                sign_out_flow_window_closed(window.app_handle(), window.label());
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if surfaces::policy::close_policy(surfaces::policy::Platform::current(), window.label())
                    == surfaces::registry::ClosePolicy::Hide
                {
                    api.prevent_close();
                    hide_on_close(window);
                    // Two-surface §9.1: a hidden onboarding or conflict window no longer holds the Dock icon.
                    #[cfg(target_os = "macos")]
                    refresh_macos_dock_icon(window.app_handle(), Some(window.label()));
                }
            }
            // Task 1683 slice 1: the Beebeeb window's end resets the menu and saves its frame; the Dock icon follows.
            #[cfg(target_os = "macos")]
            macos_window_lifecycle(window, event);
        })
        .build(tauri::generate_context!())
        .expect("error while building Beebeeb desktop")
        .run(|app, event| {
            // Review P1 on PR #134: on a Mac the Beebeeb window is destroyed on close, and when it was the last
            // window Tauri asks to exit (`code: None`). Beebeeb lives on in the menu bar; an explicit exit still exits.
            if let tauri::RunEvent::ExitRequested { code, api, .. } = &event
                && surfaces::policy::keep_running_on_exit_request(surfaces::policy::Platform::current(), *code)
            {
                api.prevent_exit();
            }
            // Task 1683 slice 1 (review item 4): a quit while the Beebeeb window is open keeps its frame.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Exit = event {
                save_main_window_frame(app);
            }
            // Review round 2, P2-1: opening the app again (Finder, Spotlight, the Dock) brings the window back.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                reopen_main_window(app);
            }
            #[cfg(not(target_os = "macos"))]
            let _ = app;
        });
}

// ── Update loop ───────────────────────────────────────────────────────────────

/// Background task: wait 5 s on launch, then check every 4 hours.
async fn run_update_loop(app: tauri::AppHandle) {
    // Give the app a few seconds to finish startup before hitting the network.
    tokio::time::sleep(std::time::Duration::from_secs(UPDATE_CHECK_STARTUP_DELAY_SECS)).await;

    loop {
        check_for_update(&app).await;
        tokio::time::sleep(std::time::Duration::from_secs(UPDATE_CHECK_INTERVAL_HOURS * 3600)).await;
    }
}

async fn check_for_update(app: &tauri::AppHandle) {
    let channel = configured_release_channel();
    let updater = match updater_for_release_channel(app, channel) {
        Ok(u) => u,
        Err(e) => {
            tracing::debug!(channel = %channel.as_str(), "updater not available: {e}");
            return;
        }
    };

    match updater.check().await {
        Ok(Some(update)) => {
            tracing::info!(
                version = %update.version,
                channel = %channel.as_str(),
                "update available — notifying frontend"
            );
            let version = update.version.to_string();
            macos_popover::note_update(channel.as_str(), Some(&version));
            // Emit to the webview so the frontend can show a banner.
            // Frontend listens with: import { listen } from "@tauri-apps/api/event"
            //   await listen("update-available", handler)
            let _ = app.emit(
                "update-available",
                serde_json::json!({
                    "version": version,
                    "channel": channel.as_str(),
                    "body": update.body.as_deref().unwrap_or(""),
                    "release_notes_url": release_notes_url_for_version(&version),
                }),
            );
        }
        Ok(None) => {
            macos_popover::note_update(channel.as_str(), None);
            tracing::debug!("no update available");
        }
        Err(e) => {
            // Network errors are expected when the release server isn't live
            // yet — log at debug to avoid noisy startup logs.
            tracing::debug!("update check failed: {e}");
        }
    }
}

// ── Native menu bar ──────────────────────────────────────────────────────────

const MENU_PREFERENCES_ID: &str = "menu_preferences";
const MENU_CHECK_UPDATES_ID: &str = "menu_check_updates";
const MENU_TOGGLE_SYNC_ID: &str = "menu_toggle_sync";
const MENU_SIGN_OUT_ID: &str = "menu_sign_out";
const MENU_QUIT_ID: &str = "menu_quit";
const MENU_OPEN_FOLDER_ID: &str = "menu_open_folder";
const MENU_UPLOAD_FILES_ID: &str = "menu_upload_files";
const MENU_NEW_FOLDER_ID: &str = "menu_new_folder";
const MENU_OPEN_WEB_APP_ID: &str = "menu_open_web_app";
const MENU_SETTINGS_ID: &str = "menu_settings";
const MENU_VIEW_FILES_ID: &str = "menu_view_files";
const MENU_VIEW_ACTIVITY_ID: &str = "menu_view_activity";
#[cfg(test)]
const MENU_VIEW_SHARED_ID: &str = "menu_view_shared";
const MENU_VIEW_TRASH_ID: &str = "menu_view_trash";
// The Beebeeb window's View items on a Mac (task 1683 slice 1, two-surface F11 and F15).
const MENU_VIEW_HOME_ID: &str = "menu_view_home";
const MENU_VIEW_BACK_ID: &str = "menu_view_back";
const MENU_ZOOM_IN_ID: &str = "menu_zoom_in";
const MENU_ZOOM_OUT_ID: &str = "menu_zoom_out";
const MENU_ZOOM_RESET_ID: &str = "menu_zoom_reset";
const MENU_HELP_DOCS_ID: &str = "menu_help_docs";
const MENU_HELP_SHORTCUTS_ID: &str = "menu_help_shortcuts";
const MENU_HELP_REPORT_ID: &str = "menu_help_report";
const MENU_HELP_STATUS_ID: &str = "menu_help_status";

const WEB_APP_URL: &str = "https://app.beebeeb.io";
// Pre-mortem 10 task 8: the old documentation subdomain has never had a DNS
// record. The plan's own literal replacement for KEYBOARD_SHORTCUTS_URL
// (beebeeb.io/faq) is ALSO a dead link -- verified via `curl -I` on the live
// marketing site (2026-09-22): /docs -> 200, /support -> 200, /faq -> 404.
// No dedicated keyboard-shortcuts or FAQ route exists (grepped
// repos/site/src for "faq" -- the only FAQ content lives embedded in
// support.astro's `allFaqs`), so KEYBOARD_SHORTCUTS_URL points at /support
// (the closest real "get help" surface) instead. DOCUMENTATION_URL points at
// /docs, a real, populated docs index (repos/site/src/pages/docs.astro),
// which is also a better topical match than the plan's own /support
// suggestion for a "Documentation" menu item.
const DOCUMENTATION_URL: &str = "https://beebeeb.io/docs";
const KEYBOARD_SHORTCUTS_URL: &str = "https://beebeeb.io/support";
const SERVICE_STATUS_URL: &str = "https://status.beebeeb.io";
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
// The desktop app has no real Shared view yet (task 1186/1243), so any legacy
// Shared navigation falls through to Files instead of opening a fake placeholder.
const WINDOWS_MAIN_APP_VIEW_ROUTES: &[&str] = &["files", "activity", "trash"];
const MENU_ZOOM_MIN: f64 = 0.5;
const MENU_ZOOM_MAX: f64 = 2.0;
const MENU_ZOOM_STEP: f64 = 0.1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DesktopMenuView {
    Files,
    Activity,
    #[cfg_attr(not(test), allow(dead_code))]
    Shared,
    Trash,
}

impl DesktopMenuView {
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    fn route(self) -> &'static str {
        match self {
            Self::Files => "files",
            Self::Activity => "activity",
            Self::Shared => "shared",
            Self::Trash => "trash",
        }
    }
}

/// Where a View item takes the Beebeeb window (macOS only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MainMenuNav {
    Home,
    Back,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuZoomAction {
    In,
    Out,
    Reset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DesktopMenuAction {
    OpenSettings,
    CheckForUpdates,
    ToggleSync,
    SignOut,
    Quit,
    OpenFolder,
    UploadFiles,
    NewFolder,
    OpenWebApp,
    OpenView(DesktopMenuView),
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    NavigateMain(MainMenuNav),
    Zoom(MenuZoomAction),
    OpenExternal(&'static str),
    ReportProblem,
}

#[derive(Debug, Clone, Copy)]
struct DesktopMenuSpec {
    id: &'static str,
    label: &'static str,
    accelerator: Option<&'static str>,
    action: DesktopMenuAction,
}

const DESKTOP_MENU_SPECS: &[DesktopMenuSpec] = &[
    DesktopMenuSpec {
        id: MENU_PREFERENCES_ID,
        label: "Preferences...",
        accelerator: Some("CmdOrCtrl+Comma"),
        action: DesktopMenuAction::OpenSettings,
    },
    DesktopMenuSpec {
        id: MENU_CHECK_UPDATES_ID,
        label: "Check for updates...",
        accelerator: Some("CmdOrCtrl+Shift+U"),
        action: DesktopMenuAction::CheckForUpdates,
    },
    DesktopMenuSpec {
        id: MENU_TOGGLE_SYNC_ID,
        label: "Pause/Resume syncing",
        accelerator: Some("CmdOrCtrl+Shift+P"),
        action: DesktopMenuAction::ToggleSync,
    },
    DesktopMenuSpec {
        id: MENU_SIGN_OUT_ID,
        label: "Sign out",
        accelerator: Some("CmdOrCtrl+Shift+L"),
        action: DesktopMenuAction::SignOut,
    },
    DesktopMenuSpec {
        id: MENU_QUIT_ID,
        label: "Quit Beebeeb",
        accelerator: Some("CmdOrCtrl+Q"),
        action: DesktopMenuAction::Quit,
    },
    DesktopMenuSpec {
        id: MENU_OPEN_FOLDER_ID,
        label: "Open Beebeeb folder",
        accelerator: Some("CmdOrCtrl+O"),
        action: DesktopMenuAction::OpenFolder,
    },
    DesktopMenuSpec {
        id: MENU_UPLOAD_FILES_ID,
        label: "Upload files...",
        accelerator: Some("CmdOrCtrl+U"),
        action: DesktopMenuAction::UploadFiles,
    },
    DesktopMenuSpec {
        id: MENU_NEW_FOLDER_ID,
        label: "New folder...",
        accelerator: Some("CmdOrCtrl+Shift+N"),
        action: DesktopMenuAction::NewFolder,
    },
    DesktopMenuSpec {
        id: MENU_OPEN_WEB_APP_ID,
        label: "Open web app",
        accelerator: Some("CmdOrCtrl+Shift+O"),
        action: DesktopMenuAction::OpenWebApp,
    },
    DesktopMenuSpec {
        id: MENU_SETTINGS_ID,
        label: "Settings...",
        accelerator: Some("CmdOrCtrl+Comma"),
        action: DesktopMenuAction::OpenSettings,
    },
    DesktopMenuSpec {
        id: MENU_VIEW_FILES_ID,
        label: "Files",
        accelerator: Some("CmdOrCtrl+Digit1"),
        action: DesktopMenuAction::OpenView(DesktopMenuView::Files),
    },
    DesktopMenuSpec {
        id: MENU_VIEW_ACTIVITY_ID,
        label: "Activity",
        accelerator: Some("CmdOrCtrl+Digit2"),
        action: DesktopMenuAction::OpenView(DesktopMenuView::Activity),
    },
    DesktopMenuSpec {
        id: MENU_VIEW_TRASH_ID,
        label: "Trash",
        accelerator: Some("CmdOrCtrl+Digit4"),
        action: DesktopMenuAction::OpenView(DesktopMenuView::Trash),
    },
    DesktopMenuSpec {
        id: MENU_ZOOM_IN_ID,
        label: "Zoom in",
        accelerator: Some("CmdOrCtrl+Equal"),
        action: DesktopMenuAction::Zoom(MenuZoomAction::In),
    },
    DesktopMenuSpec {
        id: MENU_ZOOM_OUT_ID,
        label: "Zoom out",
        accelerator: Some("CmdOrCtrl+Minus"),
        action: DesktopMenuAction::Zoom(MenuZoomAction::Out),
    },
    DesktopMenuSpec {
        id: MENU_ZOOM_RESET_ID,
        label: "Actual size",
        accelerator: Some("CmdOrCtrl+Digit0"),
        action: DesktopMenuAction::Zoom(MenuZoomAction::Reset),
    },
    DesktopMenuSpec {
        id: MENU_HELP_DOCS_ID,
        label: "Documentation",
        accelerator: None,
        action: DesktopMenuAction::OpenExternal(DOCUMENTATION_URL),
    },
    DesktopMenuSpec {
        id: MENU_HELP_SHORTCUTS_ID,
        label: "Keyboard shortcuts",
        accelerator: Some("CmdOrCtrl+Slash"),
        action: DesktopMenuAction::OpenExternal(KEYBOARD_SHORTCUTS_URL),
    },
    DesktopMenuSpec {
        id: MENU_HELP_REPORT_ID,
        label: "Report a problem",
        accelerator: None,
        action: DesktopMenuAction::ReportProblem,
    },
    DesktopMenuSpec {
        id: MENU_HELP_STATUS_ID,
        label: "Service status",
        accelerator: None,
        action: DesktopMenuAction::OpenExternal(SERVICE_STATUS_URL),
    },
];

/// The macOS menu (task 1683 slice 1; two-surface spec §5.F rows F1-F15, §9.2): macOS title case and the ellipsis
/// character, one ⌘, (F10's duplicate File › Settings is gone), the View items of the Beebeeb window. Review (⌘2) and
/// Trash (⌘3) join with their panes in slice 4. Windows and Linux keep `DESKTOP_MENU_SPECS`, unchanged.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
const MACOS_MENU_SPECS: &[DesktopMenuSpec] = &[
    DesktopMenuSpec {
        id: MENU_PREFERENCES_ID,
        label: "Settings…",
        accelerator: Some("CmdOrCtrl+Comma"),
        action: DesktopMenuAction::OpenSettings,
    },
    DesktopMenuSpec {
        id: MENU_CHECK_UPDATES_ID,
        label: "Check for Updates…",
        accelerator: Some("CmdOrCtrl+Shift+U"),
        action: DesktopMenuAction::CheckForUpdates,
    },
    DesktopMenuSpec {
        id: MENU_TOGGLE_SYNC_ID,
        // F3: the label follows the state (`refresh_macos_pause_menu_label`); this is the unpaused one.
        label: MENU_PAUSE_SYNC_LABEL,
        accelerator: Some("CmdOrCtrl+Shift+P"),
        action: DesktopMenuAction::ToggleSync,
    },
    DesktopMenuSpec {
        id: MENU_SIGN_OUT_ID,
        // F4, slice 1: behaviour unchanged (slice 1b routes it through the confirm sheet once task 2175 passes).
        label: "Sign Out…",
        accelerator: Some("CmdOrCtrl+Shift+L"),
        action: DesktopMenuAction::SignOut,
    },
    DesktopMenuSpec {
        id: MENU_QUIT_ID,
        label: "Quit Beebeeb",
        accelerator: Some("CmdOrCtrl+Q"),
        action: DesktopMenuAction::Quit,
    },
    DesktopMenuSpec {
        id: MENU_OPEN_FOLDER_ID,
        label: "Open in Finder",
        accelerator: Some("CmdOrCtrl+O"),
        action: DesktopMenuAction::OpenFolder,
    },
    DesktopMenuSpec {
        id: MENU_UPLOAD_FILES_ID,
        label: "Upload Files…",
        accelerator: Some("CmdOrCtrl+U"),
        action: DesktopMenuAction::UploadFiles,
    },
    DesktopMenuSpec {
        id: MENU_NEW_FOLDER_ID,
        label: "New Folder…",
        accelerator: Some("CmdOrCtrl+Shift+N"),
        action: DesktopMenuAction::NewFolder,
    },
    DesktopMenuSpec {
        id: MENU_OPEN_WEB_APP_ID,
        label: "Open Web App",
        accelerator: Some("CmdOrCtrl+Shift+O"),
        action: DesktopMenuAction::OpenWebApp,
    },
    DesktopMenuSpec {
        id: MENU_VIEW_HOME_ID,
        label: "Home",
        accelerator: Some("CmdOrCtrl+Digit1"),
        action: DesktopMenuAction::NavigateMain(MainMenuNav::Home),
    },
    DesktopMenuSpec {
        id: MENU_VIEW_BACK_ID,
        label: "Back",
        accelerator: Some("CmdOrCtrl+BracketLeft"),
        action: DesktopMenuAction::NavigateMain(MainMenuNav::Back),
    },
    // F12 and F13: the zoom and Help items are unchanged, words included.
    DesktopMenuSpec {
        id: MENU_ZOOM_IN_ID,
        label: "Zoom in",
        accelerator: Some("CmdOrCtrl+Equal"),
        action: DesktopMenuAction::Zoom(MenuZoomAction::In),
    },
    DesktopMenuSpec {
        id: MENU_ZOOM_OUT_ID,
        label: "Zoom out",
        accelerator: Some("CmdOrCtrl+Minus"),
        action: DesktopMenuAction::Zoom(MenuZoomAction::Out),
    },
    DesktopMenuSpec {
        id: MENU_ZOOM_RESET_ID,
        label: "Actual size",
        accelerator: Some("CmdOrCtrl+Digit0"),
        action: DesktopMenuAction::Zoom(MenuZoomAction::Reset),
    },
    DesktopMenuSpec {
        id: MENU_HELP_DOCS_ID,
        label: "Documentation",
        accelerator: None,
        action: DesktopMenuAction::OpenExternal(DOCUMENTATION_URL),
    },
    DesktopMenuSpec {
        id: MENU_HELP_SHORTCUTS_ID,
        label: "Keyboard shortcuts",
        accelerator: Some("CmdOrCtrl+Slash"),
        action: DesktopMenuAction::OpenExternal(KEYBOARD_SHORTCUTS_URL),
    },
    DesktopMenuSpec {
        id: MENU_HELP_REPORT_ID,
        label: "Report a problem",
        accelerator: None,
        action: DesktopMenuAction::ReportProblem,
    },
    DesktopMenuSpec {
        id: MENU_HELP_STATUS_ID,
        label: "Service status",
        accelerator: None,
        action: DesktopMenuAction::OpenExternal(SERVICE_STATUS_URL),
    },
];

/// F3's two labels.
const MENU_PAUSE_SYNC_LABEL: &str = "Pause Sync";
const MENU_RESUME_SYNC_LABEL: &str = "Resume Sync";

#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn pause_menu_label(paused: bool) -> &'static str {
    if paused { MENU_RESUME_SYNC_LABEL } else { MENU_PAUSE_SYNC_LABEL }
}

/// F3: the label for what the file says now (an unreadable file reads as not paused, as the menu toggle does).
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn pause_menu_label_now() -> &'static str {
    pause_menu_label(DesktopConfig::load().map(|cfg| cfg.pause_sync).unwrap_or(false))
}

/// Every pause change calls this once it is written, through `set_sync_paused` (the menu, the tray commands, Home's
/// Resume, and a settings write from the compact flyout's Network page): the menu's label follows the file (F3).
fn pause_state_changed(app: &tauri::AppHandle<impl tauri::Runtime>) {
    #[cfg(target_os = "macos")]
    refresh_macos_pause_menu_label(app);
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

/// One entry of the macOS menu bar: an item from `MACOS_MENU_SPECS` by id, or one AppKit draws itself.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MacosMenuEntry {
    Item(&'static str),
    About,
    Hide,
    HideOthers,
    ShowAll,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Minimize,
    Zoom,
    Close,
    BringAllToFront,
    Separator,
}

/// The macOS menu bar, top to bottom per menu (§9.2: "About Beebeeb · Settings… · Check for Updates… · Pause Sync ·
/// — · Sign Out… · — · Hide Beebeeb · Hide Others · Show All · — · Quit Beebeeb"; F15's Window menu and View › Back).
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
const MACOS_MENU_LAYOUT: &[(&str, &[MacosMenuEntry])] = {
    use MacosMenuEntry::*;
    &[
        (
            "Beebeeb",
            &[
                About,
                Item(MENU_PREFERENCES_ID),
                Item(MENU_CHECK_UPDATES_ID),
                Item(MENU_TOGGLE_SYNC_ID),
                Separator,
                Item(MENU_SIGN_OUT_ID),
                Separator,
                Hide,
                HideOthers,
                ShowAll,
                Separator,
                Item(MENU_QUIT_ID),
            ],
        ),
        (
            "File",
            &[
                Item(MENU_OPEN_FOLDER_ID),
                Item(MENU_UPLOAD_FILES_ID),
                Item(MENU_NEW_FOLDER_ID),
                Separator,
                Item(MENU_OPEN_WEB_APP_ID),
            ],
        ),
        ("Edit", &[Cut, Copy, Paste, SelectAll]),
        (
            "View",
            &[
                Item(MENU_VIEW_HOME_ID),
                Separator,
                Item(MENU_VIEW_BACK_ID),
                Separator,
                Item(MENU_ZOOM_IN_ID),
                Item(MENU_ZOOM_OUT_ID),
                Item(MENU_ZOOM_RESET_ID),
            ],
        ),
        ("Window", &[Minimize, Zoom, Close, Separator, BringAllToFront]),
        (
            "Help",
            &[
                Item(MENU_HELP_DOCS_ID),
                Item(MENU_HELP_SHORTCUTS_ID),
                Item(MENU_HELP_REPORT_ID),
                Separator,
                Item(MENU_HELP_STATUS_ID),
            ],
        ),
    ]
};

#[cfg(test)]
fn desktop_menu_specs() -> &'static [DesktopMenuSpec] {
    DESKTOP_MENU_SPECS
}

/// The menu table this build installs: a Mac's own, else the shared one (Windows, Linux).
fn installed_menu_specs() -> &'static [DesktopMenuSpec] {
    if cfg!(target_os = "macos") { MACOS_MENU_SPECS } else { DESKTOP_MENU_SPECS }
}

fn desktop_menu_spec(id: &str) -> Option<&'static DesktopMenuSpec> {
    installed_menu_specs().iter().find(|spec| spec.id == id)
}

fn desktop_menu_spec_required(id: &str) -> &'static DesktopMenuSpec {
    desktop_menu_spec(id).unwrap_or_else(|| panic!("missing desktop menu spec: {id}"))
}

/// The desktop a native menu is built for. Accelerators differ only where that platform's web view takes a key first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuPlatform {
    MacOs,
    Windows,
    Linux,
}

impl MenuPlatform {
    fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Linux
        }
    }
}

/// The accelerator `spec` gets on `platform`.
///
/// Pause / Resume keeps Cmd+Shift+P on macOS only. On Windows, WebView2 acts on Ctrl+Shift+P first and prints the page
/// (0.8.14, Guus's PC); a Ctrl+Alt chord would be AltGr on European keyboards. So Windows and Linux get no shortcut for it:
/// the menu item, the tray menu and Settings › Sync still pause and resume.
fn menu_accelerator(spec: &DesktopMenuSpec, platform: MenuPlatform) -> Option<&'static str> {
    match (spec.id, platform) {
        (MENU_TOGGLE_SYNC_ID, MenuPlatform::Windows | MenuPlatform::Linux) => None,
        _ => spec.accelerator,
    }
}

fn build_desktop_menu_item<M: tauri::Manager<tauri::Wry>>(
    manager: &M,
    spec: &DesktopMenuSpec,
) -> tauri::Result<MenuItem<tauri::Wry>> {
    let mut builder = MenuItemBuilder::new(spec.label).id(spec.id);
    if let Some(accelerator) = menu_accelerator(spec, MenuPlatform::current()) {
        builder = builder.accelerator(accelerator);
    }
    builder.build(manager)
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn menu_view_nav_target(view: DesktopMenuView, supported_routes: &[&str]) -> &'static str {
    let route = view.route();
    if supported_routes.contains(&route) {
        route
    } else {
        "files"
    }
}

fn compact_menu_page_for_view(view: DesktopMenuView) -> &'static str {
    match view {
        DesktopMenuView::Files => "finder",
        DesktopMenuView::Activity => "status",
        DesktopMenuView::Shared => "finder",
        // The compact macOS/Linux shell has no Trash page yet; send it to the
        // file surface instead of leaving a dead native item.
        DesktopMenuView::Trash => "finder",
    }
}

fn next_menu_zoom_scale(current: f64, action: MenuZoomAction) -> f64 {
    let next = match action {
        MenuZoomAction::In => current + MENU_ZOOM_STEP,
        MenuZoomAction::Out => current - MENU_ZOOM_STEP,
        MenuZoomAction::Reset => 1.0,
    };
    ((next.clamp(MENU_ZOOM_MIN, MENU_ZOOM_MAX) * 10.0).round()) / 10.0
}

fn about_metadata() -> AboutMetadata<'static> {
    let channel = installed_release_channel();
    AboutMetadata {
        name: Some("Beebeeb".to_string()),
        version: Some(real_app_version().to_string()),
        comments: Some(format!("Current channel: {}", channel.as_str())),
        website: Some("https://beebeeb.io".to_string()),
        website_label: Some("beebeeb.io".to_string()),
        ..AboutMetadata::default()
    }
}

/// The menu items the app changes after it built the menu (macOS): Settings…, Check for Updates… and the View items
/// follow the Beebeeb window (§8.2, F15), Pause Sync follows the pause (F3).
#[cfg(target_os = "macos")]
struct MacosMenuItems {
    settings: MenuItem<tauri::Wry>,
    check_updates: MenuItem<tauri::Wry>,
    toggle_sync: MenuItem<tauri::Wry>,
    view_home: MenuItem<tauri::Wry>,
    view_back: MenuItem<tauri::Wry>,
}

#[cfg(target_os = "macos")]
impl MacosMenuItems {
    /// The one built item for `id` when it is one of these.
    fn get(&self, id: &str) -> Option<&MenuItem<tauri::Wry>> {
        match id {
            MENU_PREFERENCES_ID => Some(&self.settings),
            MENU_CHECK_UPDATES_ID => Some(&self.check_updates),
            MENU_TOGGLE_SYNC_ID => Some(&self.toggle_sync),
            MENU_VIEW_HOME_ID => Some(&self.view_home),
            MENU_VIEW_BACK_ID => Some(&self.view_back),
            _ => None,
        }
    }
}

/// F3: the menu says "Resume Sync" while the file says paused, else "Pause Sync". Called through
/// `pause_state_changed` only.
#[cfg(target_os = "macos")]
fn refresh_macos_pause_menu_label(app: &tauri::AppHandle<impl tauri::Runtime>) {
    let Some(items) = app.try_state::<MacosMenuItems>() else {
        return;
    };
    let _ = items.toggle_sync.set_text(pause_menu_label_now());
}

/// The macOS menu bar from `MACOS_MENU_LAYOUT` (task 1683 slice 1). The items the app changes later are built from
/// `MACOS_MENU_SPECS` first and placed where the layout names them.
#[cfg(target_os = "macos")]
fn setup_native_menu(app: &mut tauri::App) -> tauri::Result<()> {
    use tauri::menu::IsMenuItem;
    let kept = MacosMenuItems {
        settings: build_desktop_menu_item(app, desktop_menu_spec_required(MENU_PREFERENCES_ID))?,
        check_updates: build_desktop_menu_item(app, desktop_menu_spec_required(MENU_CHECK_UPDATES_ID))?,
        toggle_sync: build_desktop_menu_item(app, desktop_menu_spec_required(MENU_TOGGLE_SYNC_ID))?,
        view_home: build_desktop_menu_item(app, desktop_menu_spec_required(MENU_VIEW_HOME_ID))?,
        view_back: build_desktop_menu_item(app, desktop_menu_spec_required(MENU_VIEW_BACK_ID))?,
    };
    let mut menus: Vec<Submenu<tauri::Wry>> = Vec::new();
    for (title, entries) in MACOS_MENU_LAYOUT {
        let mut items: Vec<Box<dyn IsMenuItem<tauri::Wry>>> = Vec::new();
        for entry in entries.iter() {
            let item: Box<dyn IsMenuItem<tauri::Wry>> = match *entry {
                MacosMenuEntry::Item(id) => match kept.get(id) {
                    Some(item) => Box::new(item.clone()),
                    None => Box::new(build_desktop_menu_item(app, desktop_menu_spec_required(id))?),
                },
                MacosMenuEntry::About => {
                    Box::new(PredefinedMenuItem::about(app, Some("About Beebeeb"), Some(about_metadata()))?)
                }
                MacosMenuEntry::Hide => Box::new(PredefinedMenuItem::hide(app, Some("Hide Beebeeb"))?),
                MacosMenuEntry::HideOthers => Box::new(PredefinedMenuItem::hide_others(app, Some("Hide Others"))?),
                MacosMenuEntry::ShowAll => Box::new(PredefinedMenuItem::show_all(app, Some("Show All"))?),
                MacosMenuEntry::Cut => Box::new(PredefinedMenuItem::cut(app, None)?),
                MacosMenuEntry::Copy => Box::new(PredefinedMenuItem::copy(app, None)?),
                MacosMenuEntry::Paste => Box::new(PredefinedMenuItem::paste(app, None)?),
                MacosMenuEntry::SelectAll => Box::new(PredefinedMenuItem::select_all(app, None)?),
                MacosMenuEntry::Minimize => Box::new(PredefinedMenuItem::minimize(app, Some("Minimize"))?),
                MacosMenuEntry::Zoom => Box::new(PredefinedMenuItem::maximize(app, Some("Zoom"))?),
                MacosMenuEntry::Close => Box::new(PredefinedMenuItem::close_window(app, Some("Close"))?),
                MacosMenuEntry::BringAllToFront => {
                    Box::new(PredefinedMenuItem::bring_all_to_front(app, Some("Bring All to Front"))?)
                }
                MacosMenuEntry::Separator => Box::new(PredefinedMenuItem::separator(app)?),
            };
            items.push(item);
        }
        let refs: Vec<&dyn IsMenuItem<tauri::Wry>> = items.iter().map(|item| item.as_ref()).collect();
        let submenu = Submenu::with_items(app, *title, true, &refs)?;
        if *title == "Window" {
            let _ = submenu.set_as_windows_menu_for_nsapp();
        } else if *title == "Help" {
            let _ = submenu.set_as_help_menu_for_nsapp();
        }
        menus.push(submenu);
    }
    let refs: Vec<&dyn IsMenuItem<tauri::Wry>> = menus.iter().map(|menu| menu as &dyn IsMenuItem<tauri::Wry>).collect();
    let menu = Menu::with_items(app, &refs)?;
    app.set_menu(menu)?;
    app.manage(kept);
    // No Beebeeb window yet: Back off, the rest on; and the pause label as the file says.
    apply_macos_main_menu_state(app.handle(), main_menu_state(false, false, false));
    pause_state_changed(app.handle());
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn setup_native_menu(app: &mut tauri::App) -> tauri::Result<()> {
    let preferences = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_PREFERENCES_ID))?;
    let check_updates = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_CHECK_UPDATES_ID))?;
    let toggle_sync = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_TOGGLE_SYNC_ID))?;
    let sign_out = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_SIGN_OUT_ID))?;
    let quit = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_QUIT_ID))?;
    let open_folder = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_OPEN_FOLDER_ID))?;
    let upload_files = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_UPLOAD_FILES_ID))?;
    let new_folder = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_NEW_FOLDER_ID))?;
    let open_web_app = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_OPEN_WEB_APP_ID))?;
    let settings = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_SETTINGS_ID))?;
    let view_files = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_VIEW_FILES_ID))?;
    let view_activity = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_VIEW_ACTIVITY_ID))?;
    let view_trash = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_VIEW_TRASH_ID))?;
    let zoom_in = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_ZOOM_IN_ID))?;
    let zoom_out = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_ZOOM_OUT_ID))?;
    let zoom_reset = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_ZOOM_RESET_ID))?;
    let documentation = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_HELP_DOCS_ID))?;
    let keyboard_shortcuts = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_HELP_SHORTCUTS_ID))?;
    let report_problem = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_HELP_REPORT_ID))?;
    let service_status = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_HELP_STATUS_ID))?;

    let beebeeb_menu = Submenu::with_items(
        app,
        "Beebeeb",
        true,
        &[
            &PredefinedMenuItem::about(app, Some("About Beebeeb"), Some(about_metadata()))?,
            &PredefinedMenuItem::separator(app)?,
            &preferences,
            &check_updates,
            &toggle_sync,
            &PredefinedMenuItem::separator(app)?,
            &sign_out,
            &quit,
        ],
    )?;

    let file_menu = Submenu::with_items(
        app,
        "File",
        true,
        &[
            &open_folder,
            &upload_files,
            &new_folder,
            &PredefinedMenuItem::separator(app)?,
            &open_web_app,
            &settings,
        ],
    )?;

    let edit_menu = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;

    let view_menu = Submenu::with_items(
        app,
        "View",
        true,
        &[
            &view_files,
            &view_activity,
            &view_trash,
            &PredefinedMenuItem::separator(app)?,
            &zoom_in,
            &zoom_out,
            &zoom_reset,
        ],
    )?;

    let help_menu = Submenu::with_items(
        app,
        "Help",
        true,
        &[
            &documentation,
            &keyboard_shortcuts,
            &report_problem,
            &PredefinedMenuItem::separator(app)?,
            &service_status,
        ],
    )?;

    let menu = Menu::with_items(app, &[&beebeeb_menu, &file_menu, &edit_menu, &view_menu, &help_menu])?;
    app.set_menu(menu)?;
    Ok(())
}

// A native click can create the app webview before React has subscribed. Keep
// one pending request until that webview consumes it after registering its listener.
static MENU_UPDATE_CHECK_PENDING: AtomicBool = AtomicBool::new(false);

fn update_check_window_label() -> &'static str {
    if cfg!(target_os = "windows") {
        "main-app"
    } else if cfg!(target_os = "macos") {
        // Task 1683 slice 1: the Beebeeb window drains the pending menu check in Settings › About's update row.
        "main"
    } else {
        "settings"
    }
}

fn request_menu_update_check(app: &tauri::AppHandle) {
    MENU_UPDATE_CHECK_PENDING.store(true, Ordering::SeqCst);
    // F2: the Beebeeb window on Settings › About, which checks and answers in its row.
    #[cfg(target_os = "macos")]
    show_main_window(app, MainNav::settings(Some("about")));
    #[cfg(not(target_os = "macos"))]
    show_main_app_window_with_nav(app, Some("settings"));
    if let Some(window) = app.get_webview_window(update_check_window_label()) {
        log_menu_result(
            MENU_CHECK_UPDATES_ID,
            window.emit("menu:check-for-updates", ()).map_err(|e| e.to_string()),
        );
    }
}

#[tauri::command]
fn consume_menu_update_check(window: tauri::WebviewWindow) -> bool {
    take_menu_update_check(&MENU_UPDATE_CHECK_PENDING, window.label())
}

fn take_menu_update_check(pending: &AtomicBool, label: &str) -> bool {
    label == update_check_window_label() && pending.swap(false, Ordering::SeqCst)
}

/// Task 1683 slice 1 (two-surface spec `docs/specs/2026-10-24-macos-two-surface-app.md`): the Beebeeb window's request
/// slot, what the menu may offer around it, and the macOS menu bar (§5.F rows F1-F15, §9.2). Pure, so they run on
/// every platform; the window code that uses them is macOS only.
#[cfg(test)]
mod main_window_tests;

#[cfg(test)]
mod menu_update_check_tests;

fn handle_desktop_menu_action(app: &tauri::AppHandle, spec: &'static DesktopMenuSpec) {
    match spec.action {
        DesktopMenuAction::OpenSettings => {
            // F1: the Beebeeb window on Settings (the last section used in it, else General).
            #[cfg(target_os = "macos")]
            show_main_window(app, MainNav::settings(None));
            #[cfg(not(target_os = "macos"))]
            show_main_app_window_with_nav(app, Some("settings"));
        }
        DesktopMenuAction::CheckForUpdates => {
            request_menu_update_check(app);
        }
        DesktopMenuAction::ToggleSync => {
            let app = app.clone();
            // A failure is shown (task 1889, fix round 2 N2) before it is logged: the menu has no window to put it in.
            spawn_menu_task(spec.id, async move {
                let (pausing, result) = toggle_sync_paused_from_menu(&app).await;
                show_menu_pause_failure(&app, pausing, &result);
                result
            });
        }
        DesktopMenuAction::SignOut => {
            let app = app.clone();
            spawn_menu_task(spec.id, async move {
                let state = app.state::<AppState>();
                let result = clear_session_impl(&state, false, None, crate::UnknownFilesArg::Keep, None).await;
                // Task 1882: the menu's sign-out names the folder of kept
                // Finder files in the same alert as the `clear_session` command,
                // also when it failed after the removal (review I1), and saves it for
                // the Settings › Sync row (review I2).
                surface_sign_out_folders(&app, &result);
                // FB-24 (M6): on every platform the person sees a sign-out that stopped (its error) and one that
                // happened with a step it could not confirm (its warning); before, only Windows showed either.
                match &result {
                    // Spec 2026-10-10 §4 row 5: a waiting change opens the flow in Settings; never the error dialog.
                    #[cfg(target_os = "macos")]
                    Err(error) if error.message.starts_with("unsent_changes: ") => open_sign_out_flow(&app),
                    Err(error) => {
                        app.dialog()
                            .message(error.message.clone())
                            .title("Sign-out paused")
                            .kind(tauri_plugin_dialog::MessageDialogKind::Error)
                            .show(|_| {});
                    }
                    Ok(SignedOut {
                        warning: Some(warning), ..
                    }) => {
                        app.dialog()
                            .message(warning.sentence())
                            .title("Signed out")
                            .kind(tauri_plugin_dialog::MessageDialogKind::Info)
                            .show(|_| {});
                    }
                    // Nothing to tear down — tell the user instead of running
                    // (and possibly failing) the full teardown (Bug B). Windows only, as before.
                    #[cfg(target_os = "windows")]
                    Ok(SignedOut {
                        outcome: SignOutOutcome::NotSignedIn,
                        warning: None,
                        ..
                    }) => {
                        app.dialog()
                            .message("You are not signed in on this device.")
                            .title("Sign out")
                            .kind(tauri_plugin_dialog::MessageDialogKind::Info)
                            .show(|_| {});
                    }
                    Ok(_) => {}
                }
                result.map(|_| ()).map_err(|failure| failure.message)
            });
        }
        DesktopMenuAction::Quit => app.exit(0),
        DesktopMenuAction::OpenFolder => {
            let app = app.clone();
            // Off the main thread: on macOS the open waits up to 3 s for the bridge gate and 5 s for LaunchServices.
            // A failure is shown (task 1885, I2) before it is logged: the menu has no window to put it in.
            spawn_menu_task(spec.id, async move {
                let result = on_the_blocking_pool(OPEN_FOLDER_FAILED, open_current_finder_location).await;
                show_menu_open_failure(&app, &result);
                result
            });
        }
        DesktopMenuAction::UploadFiles => {
            let app = app.clone();
            spawn_menu_task(spec.id, async move { upload_files_to_sync_root_impl(&app).map(|_| ()) });
        }
        DesktopMenuAction::NewFolder => log_menu_result(spec.id, create_folder_in_sync_root_impl(app).map(|_| ())),
        DesktopMenuAction::OpenWebApp => log_menu_result(spec.id, open_external_url(app, WEB_APP_URL)),
        DesktopMenuAction::OpenView(view) => log_menu_result(spec.id, open_menu_view(app, view)),
        // F11 and F15: View › Home opens the Beebeeb window on Home; View › Back leaves Settings in an open window.
        DesktopMenuAction::NavigateMain(nav) => {
            #[cfg(target_os = "macos")]
            match nav {
                MainMenuNav::Home => show_main_window(app, MainNav::Home),
                MainMenuNav::Back => back_in_main_window(app),
            }
            #[cfg(not(target_os = "macos"))]
            let _ = nav;
        }
        DesktopMenuAction::Zoom(action) => log_menu_result(spec.id, apply_menu_zoom(app, action)),
        DesktopMenuAction::OpenExternal(url) => log_menu_result(spec.id, open_external_url(app, url)),
        DesktopMenuAction::ReportProblem => {
            // Off the main thread: the bundle build scans every known name.
            let app = app.clone();
            spawn_menu_task(spec.id, async move { report_problem_impl(&app).map(|_| ()) });
        }
    }
}

fn spawn_menu_task<F>(menu_id: &'static str, fut: F)
where
    F: std::future::Future<Output = Result<(), String>> + Send + 'static,
{
    tauri::async_runtime::spawn(async move {
        log_menu_result(menu_id, fut.await);
    });
}

/// A failed "Open in Finder" from the menu, shown to the person (task 1885, I2): a native alert, because the menu works
/// with every window closed and so has no window to put a toast in. It is the Sign-out menu action's kind of alert, and
/// carries the button's one sentence, never the error behind it (which is logged by the caller).
fn show_menu_open_failure(app: &tauri::AppHandle, result: &Result<(), String>) {
    let Some((title, message)) = finder_open::menu_open_failure_alert(result, cfg!(target_os = "macos")) else {
        return;
    };
    app.dialog()
        .message(message)
        .title(title)
        .kind(tauri_plugin_dialog::MessageDialogKind::Error)
        .show(|_| {});
}

fn log_menu_result(menu_id: &str, result: Result<(), String>) {
    if let Err(error) = result {
        tracing::warn!(menu_id, %error, "native menu action failed");
    }
}

fn open_external_url(app: &tauri::AppHandle, url: &str) -> Result<(), String> {
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| format!("open {url}: {e}"))
}

fn open_menu_view(app: &tauri::AppHandle, view: DesktopMenuView) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let nav = menu_view_nav_target(view, WINDOWS_MAIN_APP_VIEW_ROUTES);
        show_main_app_window_with_nav(app, Some(nav));
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    {
        let page = compact_menu_page_for_view(view);
        show_compact_app_window_with_nav(app, Some(page));
        Ok(())
    }
}

fn apply_menu_zoom(app: &tauri::AppHandle, action: MenuZoomAction) -> Result<(), String> {
    let scale = {
        let state = app.state::<AppState>();
        let mut guard = state
            .menu_zoom_scale
            .lock()
            .map_err(|_| "menu zoom mutex poisoned".to_string())?;
        let next = next_menu_zoom_scale(*guard, action);
        *guard = next;
        next
    };

    let mut applied = false;
    for (label, window) in app.webview_windows() {
        if label == "tray" {
            continue;
        }
        if let Err(error) = window.set_zoom(scale) {
            tracing::warn!(window = %label, %error, "could not apply native menu zoom");
        } else {
            applied = true;
        }
    }

    if !applied {
        show_main_app_window_impl(app);
        // A Mac's main window is the Beebeeb window (F12); Windows and Linux keep today's order.
        #[cfg(target_os = "macos")]
        let labels = ["main"];
        #[cfg(not(target_os = "macos"))]
        let labels = ["main-app", "settings"];
        for label in labels {
            if let Some(window) = app.get_webview_window(label) {
                window
                    .set_zoom(scale)
                    .map_err(|e| format!("apply zoom to {label}: {e}"))?;
                return Ok(());
            }
        }
    }

    Ok(())
}

// ── System tray ──────────────────────────────────────────────────────────────

/// Build the tray context menu with the current autostart checked state.
/// Called on first setup and again whenever the autostart state changes.
///
/// The tray context menu keeps the daemon reachable without quitting it.
/// "Open Beebeeb" surfaces the main app on Windows and the compact app window
/// on other platforms; "Hide" tucks the current app surface away.
fn build_tray_menu<M: tauri::Manager<tauri::Wry>>(
    manager: &M,
    autostart_enabled: bool,
) -> tauri::Result<Menu<tauri::Wry>> {
    // "Open Beebeeb" surfaces the main app window (the full sidebar/content
    // shell on Windows; the compact app window on macOS/Linux). The menu-item id
    // stays `open_settings` for backwards compatibility with existing event
    // wiring — only the label and target window changed.
    let show_item = MenuItemBuilder::new("Open Beebeeb")
        .id("open_settings")
        .build(manager)?;
    let hide_item = MenuItemBuilder::new("Hide").id("tray_hide").build(manager)?;
    let autostart_item = CheckMenuItemBuilder::new("Start at login")
        .id("tray_autostart")
        .checked(autostart_enabled)
        .build(manager)?;
    let sep1 = PredefinedMenuItem::separator(manager)?;
    let sep2 = PredefinedMenuItem::separator(manager)?;
    let quit_item = PredefinedMenuItem::quit(manager, Some("Quit"))?;
    Menu::with_items(
        manager,
        &[&show_item, &hide_item, &sep1, &autostart_item, &sep2, &quit_item],
    )
}

/// Attach a Tauri event listener that updates the tray tooltip when
/// the engine emits an `engine-status` event. Spec 030 §5.
///
/// We use tooltip-only animation in v1 — different icon variants
/// (idle / syncing / error PNGs) are a follow-up once design ships
/// the colored set. Updating the tooltip on every tick is cheap and
/// lets the user see "Syncing 5 files…" change as work progresses.
fn attach_tray_status_listener<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    use tauri::Listener;
    let app_for_listener = app.clone();
    app.listen("engine-status", move |event| {
        let payload: serde_json::Value = match serde_json::from_str(event.payload()) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "engine-status payload not valid JSON");
                return;
            }
        };
        // Task 1683 slice 2: `state` can now be `syncing` or `offline`. Nothing a user
        // sees today may change before the flip (slice 6), so the tooltip and
        // `sync_status` keep reading the state the old emitter would have sent
        // (`engine_status::legacy_state`).
        let state = engine_status::legacy_state(&payload);

        // The popover snapshot reads the NEW fields (real state, progress, last check,
        // reason) from this mirror.
        if let Some(runtime) = app_for_listener.try_state::<popover_data::PopoverRuntime>()
            && let Ok(mut view) = runtime.status.lock()
        {
            view.apply(&payload);
        }

        // Mirror the latest state into the active account's `engine_state`
        // (per-account, decision 0800) so the `sync_status` command can return
        // it on first paint without subscribing to the event stream itself.
        // Failures here (poisoned mutex, registry not synthesized yet) are
        // non-fatal — the tray tooltip update below still runs.
        if !state.is_empty()
            && let Some(app_state) = app_for_listener.try_state::<AppState>()
            && let Ok(acct) = app_state.active_account()
            && let Ok(mut guard) = acct.engine_state.lock()
        {
            *guard = state.to_string();
        }

        let tooltip = engine_status::tray_tooltip(&payload);

        if let Some(tray) = app_for_listener.tray_by_id("tray")
            && let Err(e) = tray.set_tooltip(Some(&tooltip))
        {
            tracing::warn!(error = %e, "failed to update tray tooltip");
        }
    });
}

fn setup_tray(app: &mut tauri::App) -> tauri::Result<()> {
    // Read current autostart state so the check mark is correct on launch.
    #[cfg(not(target_os = "macos"))]
    let tray_menu = build_tray_menu(app, app.autolaunch().is_enabled().unwrap_or(false))?;

    // The tray ICON itself is built declaratively from `app.trayIcon` in
    // tauri.conf.json (iconPath, iconAsTemplate, tooltip,
    // showMenuOnLeftClick) — Tauri's `App::build()` constructs it from that
    // config BEFORE this `.setup()` closure runs. Building a second,
    // fully-independent `TrayIconBuilder::with_id("tray")` here — as this
    // function used to — registers a SECOND native status item with the
    // same string id (two blank/duplicate icons in the menu bar, task
    // 1384). Attach the menu + click behaviour to the config-built tray
    // instead of constructing a new one.
    let tray = app
        .tray_by_id("tray")
        .expect("app.trayIcon in tauri.conf.json did not register a tray with id \"tray\"");

    #[cfg(not(target_os = "macos"))]
    tray.set_menu(Some(tray_menu))?;

    tray.on_menu_event(|app, event| match event.id().as_ref() {
        "open_settings" => show_main_app_window_impl(app),
        "tray_hide" => match surfaces::policy::tray_hide(surfaces::policy::Platform::current()) {
            // A Mac hides the app (⌘H): the Beebeeb window and the menu-bar popover; the Dock or a reopen unhides.
            surfaces::policy::TrayHide::App => {
                #[cfg(target_os = "macos")]
                if let Err(error) = app.hide() {
                    tracing::warn!(%error, "the tray's Hide could not hide the app");
                }
            }
            surfaces::policy::TrayHide::Window(label) => {
                if let Some(win) = app.get_webview_window(label) {
                    let _ = win.hide();
                }
            }
        },
        "tray_autostart" => {
            // Toggle autostart and rebuild the tray menu so the check
            // mark reflects the new state. TrayIcon::set_menu replaces
            // the whole menu; there is no per-item getter in Tauri 2.
            let manager = app.autolaunch();
            let currently = manager.is_enabled().unwrap_or(false);
            if currently {
                let _ = manager.disable();
            } else {
                let _ = manager.enable();
            }
            let new_state = !currently;
            if let Some(tray) = app.tray_by_id("tray")
                && let Ok(menu) = build_tray_menu(app, new_state)
            {
                let _ = tray.set_menu(Some(menu));
            }
            tracing::info!(enabled = new_state, "autostart toggled via tray");
        }
        _ => {}
    });

    tray.on_tray_icon_event(|tray, event| {
        // Left-click toggles window visibility.
        // On Windows: show/hide the frameless tray flyout ("tray" window),
        // positioned near the bottom-right corner so it appears above the
        // system tray area. The main app opens from the context menu or
        // the tray flyout itself.
        // On macOS: show/hide the menu-bar popover (`macos_popover::toggle`,
        // task 1683 slice 3), placed under the status item by
        // `surfaces::anchor` from the status item's own frame.
        // On Linux: toggle the compact app window exactly as before.
        if let TrayIconEvent::Click {
            button,
            button_state: MouseButtonState::Up,
            position,
            rect,
            ..
        } = event
        {
            let app = tray.app_handle();
            #[cfg(not(target_os = "macos"))]
            if button != MouseButton::Left { return; }
            #[cfg(target_os = "macos")]
            let _ = button;

            #[cfg(target_os = "windows")]
            {
                let _ = &rect; // unused — Windows anchors from the click point
                if let Some(win) = app.get_webview_window("tray") {
                    if win.is_visible().unwrap_or(false) {
                        let _ = win.hide();
                    } else {
                        // Position the flyout above the tray icon.
                        // The tray event's position is physical desktop
                        // coordinates. The tray window size in tauri.conf.json
                        // is logical, so resolve the scale from the monitor
                        // containing the click; this handles a tray on a
                        // non-primary monitor or a hidden window whose current
                        // monitor is stale. If monitor lookup fails, fall back
                        // to the window scale, then 1.0.
                        let scale_factor = windows_tray_flyout_scale_factor(&win, position.x, position.y);
                        let flyout_position = windows_tray_flyout_position(position.x, position.y, scale_factor);
                        let _ = win.set_position(tauri::Position::Physical(flyout_position));
                        let _ = win.show();
                        let _ = win.set_focus();
                    }
                }
            }

            #[cfg(target_os = "macos")]
            {
                let _ = (position, rect);
                macos_popover::toggle(app);
            }

            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            {
                let _ = position;
                let _ = &rect;
                if let Some(win) = app.get_webview_window("settings") {
                    if win.is_visible().unwrap_or(false) {
                        let _ = win.hide();
                    } else {
                        show_compact_app_window_impl(app);
                    }
                } else {
                    show_compact_app_window_impl(app);
                }
            }
        }
    });

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Internal helper for non-Windows builds, where the compact app/settings
/// shell is still the only primary window.
#[cfg(not(target_os = "windows"))]
fn show_compact_app_window_impl(app: &tauri::AppHandle) {
    show_compact_app_window_with_nav(app, None);
}

/// Internal helper for non-Windows builds, optionally selecting one of the
/// compact app's existing pages (`status`, `finder`, `shared`, ...).
///
/// On macOS this window doubles as BOTH the tray flyout and the regular
/// Settings/Preferences window (task 1384; review follow-up after 8d7b5a1
/// applied the flyout's chrome-less/always-on-top/hide-on-focus-loss
/// treatment to every macOS open, breaking "Open Beebeeb", the Preferences/
/// Settings menu items, and `show_main_app_window`). Which behavior it gets
/// is decided per-open by `macos_window_mode_for_anchor`, purely from
/// whether `MACOS_TRAY_FLYOUT_ANCHOR` has a pending position (consumed here
/// via `take()`; the tray click set it before the menu-bar popover replaced
/// the flyout, and nothing sets it now, so every open is `Regular`):
/// - `Popover` (anchor present): chrome-less, always-on-top, positioned
///   under the status item, hides on focus loss — reads as a system
///   popover.
/// - `Regular` (no anchor — "Open Beebeeb", Preferences/Settings menu
///   items, a notification, a deep link): decorated, centered, normal
///   window-manager behavior, stays open when focus moves elsewhere.
///
/// The window is built once and reused for every later open, so each open
/// explicitly re-applies decorations/always-on-top/position for the mode in
/// effect (`MACOS_FLYOUT_IS_POPOVER` records which one, for the
/// once-registered focus-loss handler to read live) rather than assuming
/// whatever the window happened to be built with.
///
/// The `?platform=macos` URL tag lets the frontend force the sidebar+
/// content two-column grid regardless of this window's 680px width — see
/// `design.css`'s `html.macos-flyout` rule; without it, the existing
/// `max-width: 820px` responsive breakpoint (meant for the browser/
/// main-app case) collapsed to a single stacked column and reproduced
/// 1173's overflow. Linux keeps the original centered/decorated/no-anchor
/// behaviour untouched.
#[cfg(not(target_os = "windows"))]
fn show_compact_app_window_with_nav(app: &tauri::AppHandle, nav: Option<&str>) {
    let (label, width, height, resizable) = (
        surfaces::policy::compact_window_label(surfaces::policy::Platform::current()),
        MACOS_SETTINGS_WINDOW_WIDTH,
        MACOS_SETTINGS_WINDOW_HEIGHT,
        false,
    );
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut url = nav
        .map(|nav| format!("index.html?nav={nav}"))
        .unwrap_or_else(|| "index.html".to_string());
    #[cfg(target_os = "macos")]
    url.push_str(if nav.is_some() {
        "&platform=macos"
    } else {
        "?platform=macos"
    });

    #[cfg(target_os = "macos")]
    let pending_anchor = MACOS_TRAY_FLYOUT_ANCHOR.lock().unwrap().take();
    #[cfg(target_os = "macos")]
    let mode = macos_window_mode_for_anchor(pending_anchor);
    // `swap` both updates the flag for the focus-loss handler and hands back
    // the mode the window was actually left in after its last open, so a
    // Regular open can tell "just switched out of Popover" (needs a
    // recenter — the leftover tray-anchor position assumed no title bar and
    // is often right under the menu bar) from "still Regular" (the user may
    // have dragged this decorated window; leave it exactly where it is, as
    // the pre-1384 behavior did for every reopen).
    #[cfg(target_os = "macos")]
    let was_popover = MACOS_FLYOUT_IS_POPOVER.swap(mode == MacosWindowMode::Popover, Ordering::Relaxed);

    if let Some(win) = app.get_webview_window(label) {
        // The window already exists (this is at least its second open) —
        // make the mode switch explicit rather than leaving it however the
        // previous open left it.
        #[cfg(target_os = "macos")]
        match mode {
            MacosWindowMode::Popover => {
                let _ = win.set_decorations(false);
                let _ = win.set_always_on_top(true);
                if let Some(position) = pending_anchor {
                    let _ = win.set_position(tauri::Position::Physical(position));
                }
            }
            MacosWindowMode::Regular => {
                let _ = win.set_always_on_top(false);
                let _ = win.set_decorations(true);
                if was_popover {
                    let _ = win.center();
                }
            }
        }
        // The tray's Hide hides the app; a window shown in a hidden app stays hidden (review round 2, P2-2).
        #[cfg(target_os = "macos")]
        unhide_app(app);
        let _ = win.show();
        let _ = win.set_focus();
        if let Some(nav) = nav {
            let _ = win.emit("compact-app:navigate", nav);
        }
        return;
    }

    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut builder = tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::App(url.into()))
        .title("Beebeeb")
        .inner_size(width, height)
        .resizable(resizable)
        .center();

    #[cfg(target_os = "macos")]
    {
        // Built invisible either way, so the placement applied after
        // `.build()` below — the anchor position for a popover, or just the
        // `.center()` above for a regular window — never flashes at the
        // wrong spot first.
        builder = match mode {
            MacosWindowMode::Popover => builder.decorations(false).always_on_top(true).visible(false),
            MacosWindowMode::Regular => builder.decorations(true).always_on_top(false).visible(false),
        };
    }

    match builder.build() {
        Ok(win) => {
            #[cfg(target_os = "macos")]
            {
                if mode == MacosWindowMode::Popover {
                    if let Some(position) = pending_anchor {
                        let _ = win.set_position(tauri::Position::Physical(position));
                    }
                }

                // Hide on focus loss, like a system popover — but only
                // while the window is currently in popover mode
                // (`MACOS_FLYOUT_IS_POPOVER`); a regular open (Preferences,
                // "Open Beebeeb", the Settings menu item) must stay open
                // when focus moves elsewhere. Registered once, here, at
                // first build — the window is reused for every later open,
                // so this reads the mode flag live on each focus change
                // rather than one captured at registration time. Also
                // debounced against the spurious immediate blur some window
                // managers deliver right as a borderless always-on-top
                // window is shown (see `MACOS_FLYOUT_GAINED_FOCUS`).
                let hide_target = win.clone();
                win.on_window_event(move |event| {
                    if let tauri::WindowEvent::Focused(focused) = event {
                        if *focused {
                            MACOS_FLYOUT_GAINED_FOCUS.store(true, Ordering::Relaxed);
                        } else if MACOS_FLYOUT_GAINED_FOCUS.swap(false, Ordering::Relaxed)
                            && MACOS_FLYOUT_IS_POPOVER.load(Ordering::Relaxed)
                        {
                            let _ = hide_target.hide();
                        }
                    }
                });
            }

            #[cfg(target_os = "macos")]
            unhide_app(app);
            let _ = win.show();
            let _ = win.set_focus();
            if let Some(nav) = nav {
                let _ = win.emit("compact-app:navigate", nav);
            }
        }
        Err(error) => {
            tracing::warn!(%error, "failed to create compact app window");
        }
    }
}

/// What a route-named opener (`show_main_app_window_with_nav`) asks of the Beebeeb window on a Mac: "settings"
/// opens Settings (the last section used), anything else Home.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn main_nav_for_route(nav: Option<&str>) -> MainNav {
    match nav {
        Some("settings") => MainNav::settings(None),
        _ => MainNav::Home,
    }
}

/// Internal helper — show/create the main Beebeeb app window.
///
fn show_main_app_window_impl(app: &tauri::AppHandle) {
    show_main_app_window_with_nav(app, None);
}

/// Internal helper — show/create the main Beebeeb app window, optionally
/// selecting a route once it is visible. On Windows this is the `main-app`
/// webview (the full sidebar + content-router shell, `WindowsApp.tsx`,
/// selected by `?window=main-app&platform=windows`). On other platforms there
/// is no dedicated main-app shell yet, so we delegate to the compact app window.
///
/// Focuses an existing window if present, otherwise builds one. The window is
/// declared `visible: false` in tauri.conf.json, so the explicit `show()` +
/// `set_focus()` are required.
fn show_main_app_window_with_nav(app: &tauri::AppHandle, nav: Option<&str>) {
    // Task 1683 slice 1 (two-surface spec §7): on a Mac every opener that showed the compact window shows the Beebeeb
    // window, on Settings for "settings" and on Home otherwise. Until slice 3 only the tray click still shows the
    // compact flyout.
    #[cfg(target_os = "macos")]
    show_main_window(app, main_nav_for_route(nav));

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        // No dedicated main-app window on Linux: reuse the compact app. There is no separate compact Settings page;
        // Preferences and Settings both intentionally focus the same compact app window.
        let compact_nav = match nav {
            Some("settings") | None => None,
            Some(other) => Some(other),
        };
        show_compact_app_window_with_nav(app, compact_nav);
    }

    #[cfg(target_os = "windows")]
    {
        let label = surfaces::policy::main_window_label(surfaces::policy::Platform::current());
        let url = match nav {
            Some(nav) => format!("index.html?window=main-app&platform=windows&nav={nav}"),
            None => "index.html?window=main-app&platform=windows".to_string(),
        };

        if let Some(win) = app.get_webview_window(label) {
            let _ = win.show();
            let _ = win.set_focus();
            if let Some(nav) = nav {
                let _ = win.emit("windows-app:navigate", nav);
            }
            return;
        }

        match tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::App(url.into()))
            .title("Beebeeb")
            .inner_size(1100.0, 720.0)
            .min_inner_size(880.0, 560.0)
            .resizable(true)
            .center()
            .build()
        {
            Ok(win) => {
                let _ = win.show();
                let _ = win.set_focus();
                if let Some(nav) = nav {
                    let _ = win.emit("windows-app:navigate", nav);
                }
            }
            Err(error) => {
                tracing::warn!(%error, "failed to create main app window");
            }
        }
    }
}

#[cfg(test)]
mod tests;

/// Flow 7 P0: a recovery-phrase unlock must prove the phrase belongs to the
/// signed-in account (server-side `recovery_check` compare) BEFORE the derived
/// key is persisted. `recover_from_phrase` accepts ANY checksum-valid BIP39
/// phrase, so without this gate a wrong phrase (or a typo that still passes the
/// checksum, ~1 in 16) keys the desktop engine with a foreign key and every
/// upload from this device becomes undecryptable everywhere else.
#[cfg(test)]
mod recovery_phrase_unlock_tests;

/// Runs the REAL `popover_snapshot` command (task 1683 slice 2) on a mock Tauri runtime: real
/// `AppState`, real `PopoverRuntime`, the real session gate, the real storage fetch over HTTP
/// against a loopback API, the real cache. No webview and no window exist, and no state DB is
/// read (a test process has no state directory; that leg is `popover_data::gather_db_view`'s
/// own test against a real DB).
/// Test support: a loopback API for tests that drive real HTTP (the popover's storage fetch, the sign-out's revoke).
#[cfg(all(test, not(target_os = "windows")))]
mod loopback_api_for_tests;

#[cfg(all(test, not(target_os = "windows")))]
mod popover_snapshot_command_tests;

/// Registration wiring for the popover (task 1683 slice 2), asserted on the source because `run()`
/// needs a real window system. Runs on every platform, unlike the mock-runtime tests above.
#[cfg(test)]
mod popover_wiring_tests;

/// The macOS Finder reconciler's commands and wiring (spec 2026-10-06, plan Task 8). The
/// reconciler needs the OS, so the wiring is asserted on the source (like the popover's) and the
/// commands on a handle that has no reconciler behind it.
#[cfg(test)]
mod finder_setup_command_tests;

/// A check's engine stop (spec 2026-10-06 §5.5; Task 8 fix round 1), on a real `AccountRuntime` and a
/// real `EngineRunner` wrapped around a test task. macOS only: `stop_check_engine` is.
#[cfg(all(test, target_os = "macos"))]
mod finder_engine_tests;

/// Sign-out teardown regression tests (engine-stop gate, already-signed-out
/// no-op guard). These drive the REAL `clear_session_impl` against a
/// synthesized in-memory account; the platform credential store is the
/// non-macOS/Linux stub (every method → `Unsupported`), the Tauri app-local
/// state dir is never initialized in tests (the purge logs and continues),
/// and the Windows Cloud Files block is cfg'd out — so these tests exercise
/// exactly the cross-platform control flow, not OS integration.
#[cfg(test)]
mod signout_teardown_tests;

/// Task 1886 (P0): a sign-out on macOS and Linux ends its sessions on the server too (`POST /api/v1/auth/logout` with
/// each token), bounded and best effort as on Windows. These drive the REAL `clear_session_impl`, signed in, against a
/// loopback API the test started (`with_sign_out_api_for_test`). On Linux the test build's credential store is the
/// fail-closed stub, so a signed-in sign-out there ends in the Keychain clear's error; the stub holds nothing, so the
/// tokens the sign-out took are gone from the computer all the same and are ended. Each sign-out is compared with the
/// one whose revoke succeeded: a revoke never changes what the sign-out returns.
#[cfg(all(test, not(target_os = "windows")))]
mod sign_out_revoke_tests;

/// Startup session-probe regression tests (definitive-401 auto sign-out).
/// The probe is pointed at a raw-TCP loopback mock (same technique as the
/// `recovery_phrase_unlock_tests` server; desktop has no axum dev-dep), so
/// the classification is exercised against real HTTP status lines.
#[cfg(test)]
mod startup_session_tests;

/// Lead ruling 4 (Task 12): the owner record of the local data, completed with the account's user id and the server's
/// spelling of its email while the session still works. Only an owner that is this account is completed.
#[cfg(test)]
mod owner_backfill_tests;

/// Lead ruling T1-4: the OS's message never reaches a Tauri command's return value, a saved config
/// or a toast. `FpError::redacted()` is tested in `finder_setup::error`; these pin the two
/// `lib.rs` shims that hand a bridge error to those places.
#[cfg(all(test, target_os = "macos"))]
mod finder_error_redaction_tests;

/// Task 9 fix round 1 (item D): no unit test of the lib reaches the developer's real Keychain. The
/// sign-out tests used to: a sign-out ends in `clear_keychain_session`, which also clears the legacy
/// flat Keychain items. In a macOS test build `keychain::PlatformKeychainStore` is an in-memory store
/// (`keychain::TestKeychainStore`), and no source file but `keychain.rs` can name the real one.
#[cfg(all(test, target_os = "macos"))]
mod keychain_isolation_tests;

/// A stand-in for the Finder reconciler that confirms every Lock and every removal and ignores the
/// rest. On a Mac, Lock and Sign-out wait for the reconciler and report one they cannot confirm
/// (Task 9), so a test that runs the real command and expects success installs this.
#[cfg(test)]
async fn confirm_every_finder_event(mut rx: tokio::sync::mpsc::UnboundedReceiver<finder_setup::driver::Event>) {
    while let Some(event) = rx.recv().await {
        match event {
            finder_setup::driver::Event::Remove { ack, .. } => {
                let _ = ack.send((Ok(()), finder_removal::KeptFolder::default()));
            }
            finder_setup::driver::Event::Lock { ack } => {
                let _ = ack.send(());
            }
            _ => {}
        }
    }
}

/// Task 9 of plan 1834 (spec 2026-10-06 §5.3 and §8) and the lead's rulings on it: every trigger
/// reaches the reconciler, and the order of Lock and Sign-out against the reconciler, the engine and
/// the keys is pinned. The wiring is asserted on the source (like the other wiring tests: `run()`
/// needs a real window system); the behaviour is in `finder_lock_and_sign_out_tests`.
#[cfg(test)]
mod finder_setup_wiring_tests;

/// Lock and Sign-out against a stand-in for the Finder reconciler (Task 9; lead rulings 1, 2, 3, 5
/// and 7). macOS only: nothing else consults the reconciler. They drive the REAL `lock_vault` and
/// `clear_session_impl` on a mock app; a stand-in answers (or does not answer) the events, and
/// records what the engine slot held when it handled each one, which is how the order shows.
///
/// Sign-out is driven only as far as it can go without touching the platform credential store: the
/// already-signed-out path, and the refusals that return before the Keychain clear (a full sign-out
/// ends in `clear_keychain_session`, which also clears the legacy flat Keychain items of the
/// developer running the tests).
#[cfg(all(test, target_os = "macos"))]
mod finder_lock_and_sign_out_tests;

/// R10 (spec 2026-10-06 §5.6): local data is bound to the account that created it.
#[cfg(test)]
mod account_binding_tests;

/// Ruling R8 (spec 2026-10-06 §3): signing in again as the account that is already on this Mac
/// replaces only the session token; another account is an account switch, and nothing local changes until the person
/// confirms it; Windows is unchanged. Behaviour is tested against the in-memory test Keychain and throwaway state
/// directories (macOS), the wiring as source pins (`run()` needs a real window system).
#[cfg(test)]
mod reauth_tests;

/// Task 1882 (spec `docs/specs/2026-10-09-macos-removal-keeps-unsynced-files.md` §5): every
/// removal path in this file hands the folder macOS kept to the person. The pure pieces are
/// tested in `finder_removal`; these pin the wiring between them, which no unit can see.
#[cfg(test)]
mod finder_removal_wiring_tests;

/// 1882 r4: the pool helper really leaves the runtime thread free. A single-threaded runtime
/// drives both the job and the test: if the job ran inline it would hold the only thread while it
/// waits for a release that only this test can send, and it would give up after the timeout.
#[cfg(test)]
mod blocking_pool_tests;

/// Re-review P1 (round 3): Repair removes the Finder location, then saves the config. A save that
/// fails after the removal must still show the kept folder, as a failed sign-out does (review I1):
/// the removal cannot be undone, and a retry finds no domain, so the folder would never be named.
#[cfg(test)]
mod repair_kept_folder_tests;

/// Task 1889: every `desktop.toml` load-modify-save runs as one serialized update (`DesktopConfig::update_at`, or a
/// helper that is one: `update`, `try_update_at`, `update_held`, `ensure_account_id`). There is no `save()` to call, and
/// these pins read the source so that Windows and Linux code, which a Mac does not compile, is held to it as well. They
/// were each seen to fail on a deliberate mutation (the task's notes).
#[cfg(test)]
mod config_write_pin_tests;

/// Spec 2026-10-10 (task 1887): the sign-out's keep step and its purge, on a Mac.
#[cfg(all(test, target_os = "macos"))]
mod sign_out_keep_tests;

/// U32, the other half (spec 2026-10-10 §10): Linux and Windows keep today's sign-out. Runs in the public CI only.
#[cfg(all(test, not(target_os = "macos")))]
mod sign_out_unchanged_elsewhere_tests;

// ── Task 2176: a Mac downloads its update, it never installs one in place ─────────────────────────────────────────
//
// The App Sandbox denies replacing the running bundle (and the administrator prompt the updater falls back to), so on a
// Mac `install_update` can only fail, after downloading 12 MB, with an error nobody can diagnose. A Mac is offered a
// download instead (two-surface spec §0 ruling 5). These pins hold the Rust half: the commands refuse with a typed error
// before they touch the updater, and nothing else in the app installs an update.
#[cfg(test)]
mod mac_update_tests;

// ── Speed test: a run without figures is a failure, and one run at a time (PR #140 review, P2-7 / P3) ───────────────
#[cfg(test)]
mod speedtest_tests;

// ── The Finder install's "not ready in time" sentence keeps its category (PR #140 review, P2-8) ─────────────────────
//
// The sentence was reworded for people (no "daemon", no "Timed out waiting for"), and the Linux and Windows classifier
// keyed on those words: the saved category would have become "unknown". The sentence is one constant, the classifier
// recognises it, and these pins run on every OS (the classifier itself runs on Windows and Linux only).
#[cfg(test)]
mod finder_ipc_timeout_tests;

// ── Task 1683 slice 6: launch and lifecycle (two-surface spec §7.3, §12 row 6) ────────────────────────────────────
//
// The decisions are plain functions in `surfaces::launch` and `surfaces::policy::startup_surface`, tested there. These pins
// hold the wiring: the startup reads how the app was launched, nothing else decides it, the updater leaves its flag before
// every restart, and the login item's mechanism is what it was (task 1845 owns changing it).
#[cfg(test)]
mod launch_wiring_tests;

/// Task 2243: what the window and tray polls cost the disk. Each command opens a fresh connection, so SQLite's
/// page cache is empty every time, and each count below includes `StateDb::open` itself: every poll pays it.
#[cfg(test)]
mod task_2243_poll_cost_tests;

#[cfg(all(test, target_os = "macos"))]
mod task_2190_kept_copies_tests;
