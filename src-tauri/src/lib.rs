use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::{
    Emitter, Manager, State,
    menu::{AboutMetadata, CheckMenuItemBuilder, Menu, MenuItemBuilder, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_autostart::ManagerExt as AutostartManagerExt;
#[cfg(not(target_os = "macos"))]
use tauri_plugin_dialog::DialogExt;
use tracing_subscriber::EnvFilter;

mod account_dto;
mod api_client;
mod browser_login;
mod config;
mod conflict;
mod engine_bridge;
// Unix-domain-socket IPC (macOS File Provider extension + Linux FUSE). Unix
// sockets don't exist on Windows; the Windows Cloud Files callback runs
// in-process (see `windows_cf`), so this module is `unix`-only.
#[cfg(unix)]
mod ipc_socket;
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
#[cfg(all(test, not(target_os = "linux")))]
mod linux_fuse {
    #[path = "inode_map.rs"]
    pub mod inode_map;
}
mod lockfile;
#[cfg(target_os = "macos")]
mod macos_file_provider;
mod runner;
mod state_db;
// Sync-root filesystem watcher — the local-create UPLOAD trigger (task 0780).
// Primarily for Windows, where there is no OS extension / IPC socket to fire
// `QueueFinderCreate`; the macOS File Provider and Linux FUSE paths drive that
// over the Unix socket instead. Compiled everywhere (cheap, cross-platform via
// `notify`) but the runner only spawns it on Windows (see `runner::run`).
mod watcher;
// Windows Cloud Files API — Phase 2 Task 6. Gated to Windows only;
// the module's own files start with `#![cfg(target_os = "windows")]`
// so this `mod` declaration plus the conditional are belt-and-braces.
#[cfg(target_os = "windows")]
mod windows_cf;
use config::DesktopConfig;
// `platform_keychain_store()` resolves to the macOS Keychain store on macOS and
// the Windows Credential Manager store on Windows (Linux keeps the fail-closed
// stub). All session/vault persistence below routes through it so secrets land
// in the OS-native credential vault for the current target.
use keychain::{AuthVault, SecretBytes, SessionToken, platform_keychain_store};
use runner::EngineRunner;

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
/// thread (IPC handlers) and the future sync-engine task both need
/// access. Cheap to lock — sessions change rarely.
pub struct AppState {
    pub session: Mutex<Option<Session>>,
    pub auth_present: Mutex<bool>,
    pub auth_email: Mutex<Option<String>>,
    /// A password-verified login awaiting its TOTP second factor. Set by
    /// `desktop_login` when the OPAQUE finish returns `requires_2fa: true`;
    /// consumed (and cleared on success) by `desktop_login_2fa`. `None`
    /// whenever no 2FA challenge is in flight.
    pending_2fa: Mutex<Option<Pending2fa>>,
    /// Active engine runner, if any. `set_session` spawns one when a
    /// sync_root is also configured; `clear_session` aborts it.
    pub engine: tokio::sync::Mutex<Option<EngineRunner>>,
    /// Latest `state` field from the runner's `engine-status` events.
    /// Updated by `attach_tray_status_listener`; read by `sync_status`
    /// so the WebView can render the tri-state indicator (running /
    /// stopped / error) on the settings Status page without having to
    /// subscribe to the event stream itself for first-paint.
    ///
    /// Possible values match the runner's emit calls in
    /// `runner::emit_status` (`"running"`, `"idle"`, `"syncing"`,
    /// `"error"`, `"stopped"`). The `sync_status` handler collapses
    /// `"idle"` and `"syncing"` to `"running"` for the WebView's
    /// simpler tri-state expectation.
    pub engine_state: Mutex<String>,
    /// Runtime pause flag. Set by `tray_pause_sync` / cleared by
    /// `tray_resume_sync`. Shared with the runner so it can skip sync
    /// work on each tick without a lock round-trip.
    pub sync_paused: Arc<AtomicBool>,
    /// Account profile fetched once from `/api/v1/auth/me`. Populated by
    /// the startup session-verification path (which already hits that
    /// endpoint, so we cache the payload instead of discarding it) and
    /// served by the `account_profile` IPC without a second round-trip.
    /// `None` until the first fetch lands.
    pub cached_profile: Mutex<Option<account_dto::AccountProfile>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            session: Mutex::new(None),
            auth_present: Mutex::new(false),
            auth_email: Mutex::new(None),
            pending_2fa: Mutex::new(None),
            engine: tokio::sync::Mutex::new(None),
            // Default to "stopped" — once the runner spawns and emits
            // its first event, the listener overwrites this.
            engine_state: Mutex::new("stopped".to_string()),
            sync_paused: Arc::new(AtomicBool::new(false)),
            cached_profile: Mutex::new(None),
        }
    }
}

fn keychain_error(context: &str, error: impl fmt::Display) -> String {
    format!("{context}: {error}")
}

/// Drop the cached `/auth/me` profile from `AppState`.
///
/// Called on logout (`clear_session`) and lock (`lock_vault`) so the
/// `account_profile` IPC can never cache-hit a stale (logged-out / locked)
/// identity. A subsequent login / unlock re-fetches and re-caches. Poisoned
/// mutex is swallowed — clearing must never fail a logout/lock from the
/// user's POV. Takes `&AppState` (not `State<'_, _>`) so it's unit-testable
/// against a bare `AppState::default()`.
fn clear_cached_profile(state: &AppState) {
    if let Ok(mut guard) = state.cached_profile.lock() {
        *guard = None;
    }
}

fn persist_session_to_keychain(token: &str, master_key: [u8; 32], email: Option<&str>) -> Result<(), String> {
    let vault = AuthVault::new(platform_keychain_store());
    let token = SessionToken::new(token.to_string()).map_err(|e| keychain_error("session token", e))?;
    vault
        .install_session(token)
        .map_err(|e| keychain_error("store session in Keychain", e))?;
    vault
        .store_wrapped_master_key(SecretBytes::new_master_key(master_key))
        .map_err(|e| keychain_error("store vault key in Keychain", e))?;
    // Persist the account email alongside the secrets so the Account page can
    // show it after an auto-unlock on relaunch. Non-fatal (see helper): the email
    // is display metadata, so a store that can't hold it never fails the login.
    // Empty/None → skip.
    persist_account_email_to_keychain(&vault, email)
}

fn persist_session_token_to_keychain(token: &str, email: Option<&str>) -> Result<(), String> {
    let vault = AuthVault::new(platform_keychain_store());
    let token = SessionToken::new(token.to_string()).map_err(|e| keychain_error("session token", e))?;
    vault
        .install_session(token)
        .map_err(|e| keychain_error("store session in Keychain", e))?;
    persist_account_email_to_keychain(&vault, email)
}

/// Store the account email in the credential vault if one is provided.
///
/// Always returns `Ok(())`: the email is display metadata (it lets the Account
/// page show who is signed in after an auto-unlock), NOT key material required
/// to authenticate or unlock. A store that can't hold it must never fail an
/// otherwise-successful login — so a persist error is logged and swallowed
/// rather than surfaced. `None`/empty is a no-op.
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

fn persist_vault_key_to_keychain(master_key: [u8; 32]) -> Result<(), String> {
    AuthVault::new(platform_keychain_store())
        .store_wrapped_master_key(SecretBytes::new_master_key(master_key))
        .map_err(|e| keychain_error("store vault key in Keychain", e))
}

fn load_session_token_from_keychain() -> Result<Option<String>, String> {
    AuthVault::new(platform_keychain_store())
        .session_token()
        .map_err(|e| keychain_error("load session from Keychain", e))
        .map(|token| token.map(|t| t.expose_for_request().to_string()))
}

fn load_session_from_keychain(email: Option<String>) -> Result<Option<Session>, String> {
    let mut vault = AuthVault::new(platform_keychain_store());
    let Some(token) = vault
        .session_token()
        .map_err(|e| keychain_error("load session from Keychain", e))?
    else {
        return Ok(None);
    };
    vault
        .unlock()
        .map_err(|e| keychain_error("unlock vault key from Keychain", e))?;
    let mut master_key = [0u8; 32];
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
        master_key,
        email,
    }))
}

fn keychain_session_present() -> bool {
    AuthVault::new(platform_keychain_store())
        .session_token()
        .map(|token| token.is_some())
        .unwrap_or(false)
}

fn clear_keychain_session() -> Result<(), String> {
    let mut vault = AuthVault::new(platform_keychain_store());
    vault
        .clear_session()
        .map_err(|e| keychain_error("clear Keychain session", e))
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
    let state = app.state::<AppState>();

    // Don't clobber a session already installed this run (e.g. a fresh login
    // that landed before this startup task ran).
    if state.session.lock().map(|g| g.is_some()).unwrap_or(false) {
        return;
    }

    let email = state.auth_email.lock().ok().and_then(|guard| guard.clone());
    let session = match load_session_from_keychain(email) {
        // Both token + master key present → vault is unlocked; resume.
        Ok(Some(session)) => session,
        // No stored session TOKEN at all (never signed in / logged out).
        // Leave onboarding to prompt for sign-in. Not an error.
        Ok(None) => return,
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
            return;
        }
    };

    let token = session.token.clone();
    let master_key = session.master_key;
    let email = session.email.clone();
    {
        match state.session.lock() {
            Ok(mut guard) => *guard = Some(session),
            Err(_) => {
                tracing::warn!("session mutex poisoned during startup restore");
                return;
            }
        }
    }
    set_auth_present(&state, true);
    // Mirror `apply_session` / the live-unlock path: keep `auth_email` in step
    // with the restored session so the Account page shows the signed-in email
    // after an auto-unlock, instead of going blank until the next login. Only
    // overwrite when the restored session actually carries an email — never
    // clobber an already-known address with `None`.
    if email.is_some() {
        set_auth_email(&state, email);
    }
    tracing::info!("vault auto-unlocked from credential store on startup");
    start_engine_if_possible(app.clone(), &state, token, master_key).await;
}

fn set_auth_present(state: &State<'_, AppState>, present: bool) {
    if let Ok(mut guard) = state.auth_present.lock() {
        *guard = present;
    }
}

fn set_auth_email(state: &State<'_, AppState>, email: Option<String>) {
    if let Ok(mut guard) = state.auth_email.lock() {
        *guard = email;
    }
}

async fn start_engine_if_possible(
    app: tauri::AppHandle,
    state: &State<'_, AppState>,
    token: String,
    master_key: [u8; 32],
) {
    if let Some(cfg) = DesktopConfig::load().ok() {
        let Some(root) = cfg.sync_root else { return };
        // Rehydrate the persisted pause state before spawning so a
        // restart-while-paused stays paused (the in-memory AtomicBool
        // always defaults to false; desktop.toml is the source of truth).
        state.sync_paused.store(cfg.pause_sync, Ordering::Relaxed);
        let pause_flag = state.sync_paused.clone();
        let mut engine_slot = state.engine.lock().await;
        if let Some(prev) = engine_slot.take() {
            prev.abort().await;
        }
        *engine_slot = Some(EngineRunner::spawn(app, root, token, master_key, pause_flag));
    }
}

// ── IPC commands: first-launch onboarding ────────────────────────────────────

/// Open the first-launch onboarding window.
///
/// The window is wide enough for the two-column onboarding shell and centred. It mounts
/// `Onboarding.tsx` (via `?window=onboarding`) which walks the user
/// through three steps: sign-in, sync-folder selection, and a first-sync
/// progress view. Called from `setup` if no sync_root is configured and
/// no session is loaded (i.e., fresh install or post-uninstall run).
///
/// Re-calling when the window already exists is a no-op (focus only).
pub(crate) fn open_onboarding_window_impl(app: &tauri::AppHandle) -> Result<(), String> {
    // Windows uses a wider two-column layout (WindowsFirstRun.tsx) selected by
    // the `platform=windows` query param, matching the pattern in
    // `show_settings_window` and the `windows-onboarding` entry in tauri.conf.json.
    // macOS/Linux keep the original compact single-column Onboarding.tsx (no param).
    #[cfg(target_os = "windows")]
    let (label, url, width, height) = (
        "windows-onboarding",
        "index.html?window=onboarding&platform=windows",
        860.0_f64,
        640.0_f64,
    );
    #[cfg(not(target_os = "windows"))]
    let (label, url, width, height) = (
        "onboarding",
        "index.html?window=onboarding",
        860.0_f64,
        640.0_f64,
    );

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

/// IPC wrapper so the frontend can also open the onboarding window
/// (e.g., from Account page → "Switch account").
#[tauri::command]
async fn open_onboarding_window(app: tauri::AppHandle) -> Result<(), String> {
    open_onboarding_window_impl(&app)
}

/// Show the settings window — called by `Onboarding.tsx` after the user
/// completes the 3-step flow and closes the onboarding window.
/// Also aliased as `show_settings_window` for callers in the Windows UI.
#[tauri::command]
fn show_settings(app: tauri::AppHandle) -> Result<(), String> {
    show_settings_window_impl(&app);
    Ok(())
}

/// Result of an OPAQUE sign-in attempt handed back to the frontend.
///
/// `requires_2fa == false` means the session is fully installed and onboarding
/// can proceed to the recovery-phrase step. `requires_2fa == true` means the
/// password was accepted but a TOTP code is still needed: `desktop_login`
/// stashed a [`Pending2fa`] in `AppState`, and the frontend must collect the
/// 6-digit code and call `desktop_login_2fa` to complete sign-in.
#[derive(serde::Serialize)]
struct LoginOutcome {
    requires_2fa: bool,
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
    state: State<'_, AppState>,
    email: String,
    password: String,
) -> Result<LoginOutcome, String> {
    let base_url = runner::api_base_url();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("reqwest build: {e}"))?;

    let email = email.trim().to_lowercase();
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
    if start_resp.status() == reqwest::StatusCode::UNAUTHORIZED || start_resp.status() == reqwest::StatusCode::NOT_FOUND
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
    // `ksf_version` selects the KSF the account's password file was registered
    // under (0 = legacy Identity KSF, any other value = current Argon2idKsf).
    // Mirrors web (`/opaque/login-start` → `ksf_version`). Default to the
    // current suite (1) if the server omits it, matching the web client.
    let ksf_version = start_body
        .get("ksf_version")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(1);
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
        let partial_token = finish_body
            .get("partial_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "No partial_token in login finish response".to_string())?
            .to_string();
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
        return Ok(LoginOutcome { requires_2fa: true });
    }
    let session_token = finish_body
        .get("session_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "No session_token in login finish response".to_string())?
        .to_string();

    let profile = fetch_session_profile(&client, &base_url, &session_token).await?;
    // Cache the profile we just fetched so the `account_profile` IPC can serve
    // the Account page immediately without re-hitting `/auth/me`.
    if let Ok(mut guard) = state.cached_profile.lock() {
        *guard = Some(profile);
    }

    if let Err(e) = persist_session_token_to_keychain(&session_token, Some(&email)) {
        let _ = revoke_desktop_session(&client, &base_url, &session_token).await;
        return Err(e);
    }

    set_auth_present(&state, true);
    set_auth_email(&state, Some(email.clone()));
    tracing::info!("desktop account session installed");

    Ok(LoginOutcome { requires_2fa: false })
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
async fn desktop_login_2fa(state: State<'_, AppState>, code: String) -> Result<(), String> {
    let base_url = runner::api_base_url();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
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
    let session_token = verify_body
        .get("session_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "No session_token in 2FA verify response".to_string())?
        .to_string();

    // Same post-finish setup as `desktop_login`'s no-2FA success path.
    let profile = fetch_session_profile(&client, &base_url, &session_token).await?;
    if let Ok(mut guard) = state.cached_profile.lock() {
        *guard = Some(profile);
    }

    if let Err(e) = persist_session_token_to_keychain(&session_token, Some(&email)) {
        let _ = revoke_desktop_session(&client, &base_url, &session_token).await;
        return Err(e);
    }

    set_auth_present(&state, true);
    set_auth_email(&state, Some(email));
    // Challenge satisfied — drop the pending state so a stale partial token
    // can't be reused.
    if let Ok(mut guard) = state.pending_2fa.lock() {
        *guard = None;
    }
    tracing::info!("desktop account session installed after 2FA");

    Ok(())
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
    let existing = state
        .session
        .lock()
        .map_err(|_| "session mutex poisoned".to_string())?
        .as_ref()
        .map(|session| (session.token.clone(), session.master_key));
    if let Some((token, master_key)) = existing {
        start_engine_if_possible(app, &state, token, master_key).await;
        return Ok(());
    }

    let token = load_session_token_from_keychain()?.ok_or_else(|| "Sign in before unlocking the vault.".to_string())?;
    let email = state.auth_email.lock().ok().and_then(|guard| guard.clone());
    let recovery_phrase = normalize_recovery_phrase_input(&recovery_phrase)?;
    let master_key_struct = beebeeb_core::recovery::recover_from_phrase(&recovery_phrase)
        .map_err(|_| "Recovery phrase does not match a valid 12-word Beebeeb phrase.".to_string())?;
    let master_key: [u8; 32] = master_key_struct.to_bytes();

    persist_vault_key_to_keychain(master_key)?;
    // `desktop_login` already persisted the email when it stored the token, but
    // persist again here (idempotent) so the invariant "a fully-provisioned
    // session has its email in the store" holds even if memory and store drift.
    if let Some(email) = email.as_deref() {
        let vault = AuthVault::new(platform_keychain_store());
        if let Err(e) = vault.store_account_email(email) {
            // Non-fatal: the email is display metadata, not required to unlock.
            tracing::warn!(error = %e, "could not persist account email during recovery-phrase unlock");
        }
    }

    {
        let mut guard = state.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
        *guard = Some(Session {
            token: token.clone(),
            master_key,
            email,
        });
    }
    set_auth_present(&state, true);
    tracing::info!("vault provisioned from recovery phrase");
    start_engine_if_possible(app, &state, token, master_key).await;
    Ok(())
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

async fn revoke_desktop_session(client: &reqwest::Client, base_url: &str, session_token: &str) -> Result<(), String> {
    client
        .post(format!("{base_url}/api/v1/auth/logout"))
        .bearer_auth(session_token)
        .send()
        .await
        .map_err(|e| format!("logout failed: {e}"))?
        .error_for_status()
        .map(|_| ())
        .map_err(|e| format!("logout failed: {e}"))
}

// ── IPC commands: session ─────────────────────────────────────────────────────

/// Stash an authenticated session in app state. Called by the WebView
/// immediately after a successful OPAQUE (or legacy) login on the
/// frontend side. Idempotent — overwrites any previous session.
///
/// If a sync_root is also configured, spawns the engine runner.
/// Otherwise the engine waits for `pick_sync_root` to land first.
///
/// Returns `Err` if `master_key` is not exactly 32 bytes; the WebView
/// should treat that as a programming bug and surface it loudly.
#[tauri::command]
async fn set_session(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    token: String,
    master_key: Vec<u8>,
    email: Option<String>,
) -> Result<(), String> {
    if master_key.len() != 32 {
        return Err(format!("master_key must be 32 bytes, got {}", master_key.len()));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&master_key);
    apply_session(app, &state, token, arr, email).await
}

/// Persist a freshly-authenticated session and start the engine.
///
/// This is the shared core of `set_session` (called by the web client) and
/// `start_browser_login` (the Windows browser-handoff flow): both arrive at a
/// session token + 32-byte master key + email and need the exact same
/// side-effects — write to the platform credential store, stash in memory, and
/// spawn the sync engine if a sync_root is already configured. Keeping it in one
/// place means the browser handoff can never drift from the IPC path.
pub(crate) async fn apply_session(
    app: tauri::AppHandle,
    state: &State<'_, AppState>,
    token: String,
    master_key: [u8; 32],
    email: Option<String>,
) -> Result<(), String> {
    persist_session_to_keychain(&token, master_key, email.as_deref())?;
    let token_clone = token.clone();

    {
        let mut guard = state.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
        *guard = Some(Session {
            token,
            master_key,
            email: email.clone(),
        });
    }
    set_auth_present(state, true);
    set_auth_email(state, email);
    tracing::info!("session installed");

    // If we already know the sync_root, kick off the engine. Otherwise
    // it'll start when the first-launch picker resolves.
    start_engine_if_possible(app, state, token_clone, master_key).await;
    Ok(())
}

/// Drop any cached session and abort the engine if running. Called by
/// the WebView on logout. Returns `Ok(())` even if the session mutex
/// was poisoned, because logout should always appear to succeed from
/// the user's POV — the only error path Tauri requires here is for
/// the macro's async-with-state contract.
#[tauri::command]
async fn clear_session(state: State<'_, AppState>) -> Result<(), String> {
    // Stop the engine before dropping memory so the IPC listener cannot accept
    // new File Provider operations with a cloned master key.
    let mut engine_slot = state.engine.lock().await;
    if let Some(prev) = engine_slot.take() {
        prev.abort().await;
        tracing::info!("engine aborted on logout");
    }
    drop(engine_slot);

    // Windows: remove the Explorer SHELL registration (nav-pane entry, Status
    // column, overlays) on sign-out so a logged-out machine doesn't show a dead
    // "Beebeeb" sidebar entry pointing at a folder the user is no longer signed
    // into. The Win32 Cloud Files registration + placeholders are left in place
    // (they re-converge on the next login via connect_root); this strips only the
    // shell chrome. Best-effort and engine-independent: the Id is reconstructed
    // from the persisted sync-root path, so this works after the engine is
    // aborted above. A failure (incl. "not registered") is logged, not surfaced —
    // logout must always appear to succeed.
    #[cfg(target_os = "windows")]
    {
        if let Some(root) = DesktopConfig::load().ok().and_then(|cfg| cfg.sync_root) {
            if let Err(error) = windows_cf::unregister_shell_sync_root(&root) {
                tracing::warn!(error = %error, "Explorer shell sync-root unregister on logout failed (best-effort)");
            }
        }
    }

    match state.session.lock() {
        Ok(mut guard) => {
            guard.take();
            tracing::info!("session cleared via IPC");
        }
        Err(_) => {
            tracing::warn!("session mutex poisoned during clear_session");
        }
    }
    // Drop the cached account profile so a subsequent `account_profile` IPC
    // can't cache-hit a stale (logged-out) identity. A fresh login repopulates it.
    clear_cached_profile(&state);
    // Sign out = clean slate: drop any in-flight 2FA challenge so an abandoned
    // partial token can't linger (and is zeroized) past a logout.
    if let Ok(mut guard) = state.pending_2fa.lock() {
        *guard = None;
    }
    if let Ok(mut guard) = state.engine_state.lock() {
        *guard = "stopped".to_string();
    }
    clear_keychain_session()?;
    set_auth_present(&state, false);
    set_auth_email(&state, None);
    Ok(())
}

/// Restore a Keychain-backed session into memory and start the engine for the
/// configured sync root. Keychain may show the OS unlock prompt depending on
/// the user's security settings. Until the user calls this command, hydration
/// and upload commands stay unavailable because there is no in-memory key.
#[tauri::command]
async fn unlock_vault(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let existing = state
        .session
        .lock()
        .map_err(|_| "session mutex poisoned".to_string())?
        .as_ref()
        .map(|session| (session.token.clone(), session.master_key));
    if let Some((token, master_key)) = existing {
        start_engine_if_possible(app, &state, token, master_key).await;
        return Ok(());
    }

    let email = state.auth_email.lock().ok().and_then(|guard| guard.clone());
    let session =
        load_session_from_keychain(email)?.ok_or_else(|| "Sign in before unlocking the vault.".to_string())?;
    let token = session.token.clone();
    let master_key = session.master_key;
    {
        let mut guard = state.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
        *guard = Some(session);
    }
    set_auth_present(&state, true);
    tracing::info!("vault unlocked from Keychain");
    start_engine_if_possible(app, &state, token, master_key).await;
    Ok(())
}

/// Lock clears all runtime key material and stops the sync daemon, but keeps
/// the Keychain session so the user can unlock again without re-entering their
/// recovery phrase.
#[tauri::command]
async fn lock_vault(state: State<'_, AppState>) -> Result<(), String> {
    let mut engine_slot = state.engine.lock().await;
    if let Some(prev) = engine_slot.take() {
        prev.abort().await;
        tracing::info!("engine aborted on vault lock");
    }
    drop(engine_slot);

    match state.session.lock() {
        Ok(mut guard) => {
            guard.take();
            tracing::info!("vault locked; runtime session cleared");
        }
        Err(_) => {
            tracing::warn!("session mutex poisoned during lock_vault");
        }
    }
    // Drop the cached account profile on lock too, so "locked == no profile"
    // stays honest; the next `unlock_vault` re-fetches.
    clear_cached_profile(&state);
    // Lock = clean slate: drop any in-flight 2FA challenge so an abandoned
    // partial token can't linger (and is zeroized) past a lock.
    if let Ok(mut guard) = state.pending_2fa.lock() {
        *guard = None;
    }
    if let Ok(mut guard) = state.engine_state.lock() {
        *guard = "stopped".to_string();
    }
    set_auth_present(&state, keychain_session_present());
    Ok(())
}

#[derive(Debug, serde::Serialize)]
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
    disabled_autostart: bool,
    removed_socket: bool,
    removed_cache_files: usize,
    skipped_cache_files: usize,
    pending_operations_preserved: i64,
    sync_root_preserved: Option<String>,
    warnings: Vec<String>,
}

fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

fn classify_finder_install_error(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if lower.contains("provision") || lower.contains("entitlement") || lower.contains("app-group") {
        "provisioning".to_string()
    } else if lower.contains("disabled") || lower.contains("-2011") || lower.contains("sync is not enabled") {
        "disabled".to_string()
    } else if lower.contains("timed out") || lower.contains("timeout") {
        "timeout".to_string()
    } else if lower.contains("not available") || lower.contains("only available") || lower.contains("unsupported") {
        "unsupported".to_string()
    } else {
        "unknown".to_string()
    }
}

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

fn finder_state_path(cfg: &DesktopConfig, installed: bool) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let _ = cfg;
        if installed {
            return file_provider_visible_location().ok().flatten();
        }
        return None;
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = installed;
        cfg.sync_root.as_ref().map(|p| p.to_string_lossy().into_owned())
    }
}

fn persist_finder_install_result(
    cfg: &mut DesktopConfig,
    installed: bool,
    error: Option<String>,
) -> Result<(), String> {
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
    cfg.save()
}

fn state_db_for_config(cfg: &DesktopConfig) -> Result<Option<state_db::StateDb>, String> {
    let Some(sync_root) = &cfg.sync_root else {
        return Ok(None);
    };
    let db_path = sync_root.join(".beebeeb").join("state.db");
    if !db_path.exists() {
        return Ok(None);
    }
    state_db::StateDb::open(&db_path)
        .map(Some)
        .map_err(|e| format!("open state.db: {e}"))
}

fn disposable_cache_roots() -> Vec<PathBuf> {
    let mut roots = vec![std::env::temp_dir()];
    if let Some(cache_dir) = dirs::cache_dir() {
        roots.push(cache_dir);
    }
    roots
}

fn is_disposable_cache_path(path: &std::path::Path) -> bool {
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
    disposable_cache_roots()
        .into_iter()
        .filter_map(|root| root.canonicalize().ok())
        .any(|root| canonical_path.starts_with(root))
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

async fn persist_sync_root_and_start_engine(
    app: tauri::AppHandle,
    state: &State<'_, AppState>,
    cfg: &mut DesktopConfig,
    root: PathBuf,
) -> Result<(), String> {
    cfg.sync_root = Some(root.clone());
    cfg.save()?;

    let session = state
        .session
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|s| (s.token.clone(), s.master_key)));
    if let Some((token, key)) = session {
        // Rehydrate the persisted pause state before spawning.
        state.sync_paused.store(cfg.pause_sync, Ordering::Relaxed);
        let pause_flag = state.sync_paused.clone();
        let mut engine_slot = state.engine.lock().await;
        if let Some(prev) = engine_slot.take() {
            prev.abort().await;
        }
        *engine_slot = Some(EngineRunner::spawn(app, root, token, key, pause_flag));
    }

    Ok(())
}

async fn start_engine_for_pending_finder_install(
    app: tauri::AppHandle,
    state: &State<'_, AppState>,
    root: PathBuf,
) -> Result<bool, String> {
    let session = state
        .session
        .lock()
        .map_err(|_| "session mutex poisoned".to_string())?
        .as_ref()
        .map(|s| (s.token.clone(), s.master_key));

    let Some((token, key)) = session else {
        return Err("Unlock the vault before installing the Finder location.".to_string());
    };

    // Rehydrate the persisted pause state before spawning. Best-effort —
    // a missing/unreadable config defaults to not-paused.
    let paused = DesktopConfig::load().map(|c| c.pause_sync).unwrap_or(false);
    state.sync_paused.store(paused, Ordering::Relaxed);

    let started = {
        let pause_flag = state.sync_paused.clone();
        let mut engine_slot = state.engine.lock().await;
        if engine_slot.is_some() {
            false
        } else {
            *engine_slot = Some(EngineRunner::spawn(app, root, token, key, pause_flag));
            true
        }
    };

    // The macOS File Provider extension talks to the daemon over a Unix
    // socket, so we wait for that endpoint before claiming the install is
    // ready. Windows has no such socket (the Cloud Files provider is
    // in-process), so the readiness probe is unix-only.
    #[cfg(unix)]
    if let Err(error) = wait_for_file_provider_ipc_ready().await {
        stop_pending_finder_install_engine(state, started).await;
        return Err(error);
    }

    Ok(started)
}

async fn stop_pending_finder_install_engine(state: &State<'_, AppState>, started: bool) {
    if !started {
        return;
    }
    let mut engine_slot = state.engine.lock().await;
    if let Some(prev) = engine_slot.take() {
        prev.abort().await;
    }
}

#[cfg(unix)]
async fn wait_for_file_provider_ipc_ready() -> Result<(), String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;
    use tokio::time::{Duration, Instant, sleep, timeout};

    let path = ipc_socket::ipc_socket_path();
    let deadline = Instant::now() + Duration::from_secs(3);
    let request = serde_json::to_vec(&ipc_socket::IpcRequest::GetSyncSummary)
        .map_err(|e| format!("encode IPC readiness probe: {e}"))?;

    loop {
        match timeout(Duration::from_millis(300), UnixStream::connect(&path)).await {
            Ok(Ok(mut stream)) => {
                let mut response = vec![0u8; 4096];
                if stream.write_all(&request).await.is_ok()
                    && let Ok(Ok(bytes_read)) = timeout(Duration::from_millis(300), stream.read(&mut response)).await
                    && bytes_read > 0
                    && serde_json::from_slice::<ipc_socket::IpcResponse>(&response[..bytes_read]).is_ok()
                {
                    return Ok(());
                }
            }
            Ok(Err(_)) | Err(_) => {}
        }

        if Instant::now() >= deadline {
            return Err(
                "Timed out waiting for the local Beebeeb sync daemon before installing Finder location.".to_string(),
            );
        }
        sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(target_os = "macos")]
fn file_provider_installed() -> Result<bool, String> {
    macos_file_provider::status()
}

#[cfg(not(target_os = "macos"))]
fn file_provider_installed() -> Result<bool, String> {
    Err("File Provider is only available on macOS.".to_string())
}

#[cfg(target_os = "macos")]
fn install_file_provider_domain() -> Result<(), String> {
    macos_file_provider::install()
}

#[cfg(not(target_os = "macos"))]
fn install_file_provider_domain() -> Result<(), String> {
    Err("File Provider is only available on macOS.".to_string())
}

#[cfg(target_os = "macos")]
fn remove_file_provider_domain() -> Result<(), String> {
    macos_file_provider::remove()
}

#[cfg(not(target_os = "macos"))]
fn remove_file_provider_domain() -> Result<(), String> {
    Err("File Provider is only available on macOS.".to_string())
}

#[cfg(target_os = "macos")]
fn file_provider_visible_location() -> Result<Option<String>, String> {
    macos_file_provider::visible_url()
}

#[cfg(not(target_os = "macos"))]
fn file_provider_visible_location() -> Result<Option<String>, String> {
    Ok(None)
}

#[tauri::command]
fn finder_location_state() -> Result<FinderInstallState, String> {
    let mut cfg = DesktopConfig::load()?;
    match file_provider_installed() {
        Ok(installed) => {
            if installed && cfg.finder_install_status.as_deref() != Some("installed") {
                persist_finder_install_result(&mut cfg, true, None)?;
            }
            Ok(finder_install_state_from_config(&cfg, installed, None))
        }
        Err(error) => Ok(finder_install_state_from_config(&cfg, false, Some(error))),
    }
}

/// Persist the chosen sync root and start the daemon when possible, but do not
/// claim Finder integration succeeded while the `.appex` is absent.
#[tauri::command]
async fn install_finder_location(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: Option<String>,
) -> Result<FinderInstallState, String> {
    #[cfg(target_os = "macos")]
    let root = config::default_sync_root_suggestion();
    #[cfg(not(target_os = "macos"))]
    let root = path
        .map(PathBuf::from)
        .unwrap_or_else(config::default_sync_root_suggestion);
    #[cfg(target_os = "macos")]
    let _ = path;
    if !root.is_absolute() {
        return Err(format!("Finder location must be absolute: {}", root.display()));
    }
    config::ensure_directory(&root)?;

    let mut cfg = DesktopConfig::load()?;
    let started_pending_engine = start_engine_for_pending_finder_install(app.clone(), &state, root.clone()).await?;

    if let Err(error) = install_file_provider_domain() {
        stop_pending_finder_install_engine(&state, started_pending_engine).await;
        persist_finder_install_result(&mut cfg, false, Some(error.clone()))?;
        return Err(error);
    }
    if let Err(error) = persist_sync_root_and_start_engine(app, &state, &mut cfg, root.clone()).await {
        stop_pending_finder_install_engine(&state, started_pending_engine).await;
        let _ = remove_file_provider_domain();
        return Err(error);
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

#[tauri::command]
async fn continue_without_finder_location(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: Option<String>,
) -> Result<FinderInstallState, String> {
    let root = path
        .map(PathBuf::from)
        .unwrap_or_else(config::default_sync_root_suggestion);
    if !root.is_absolute() {
        return Err(format!("Finder location must be absolute: {}", root.display()));
    }
    config::ensure_directory(&root)?;

    let mut cfg = DesktopConfig::load()?;
    persist_sync_root_and_start_engine(app, &state, &mut cfg, root.clone()).await?;
    Ok(finder_install_state_from_config(&cfg, false, None))
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
    {
        let root = path
            .map(PathBuf::from)
            .unwrap_or_else(config::default_sync_root_suggestion);
        if !root.is_absolute() {
            return Err(format!("Sync root must be absolute: {}", root.display()));
        }
        config::ensure_directory(&root)?;

        let mut cfg = DesktopConfig::load()?;

        // Register the Cloud Files sync root with Windows. Idempotent at the OS
        // layer. The engine runner re-registers + connects callbacks on spawn
        // (`windows_cf::connect_root`, called before the engine writes its lock
        // and state.db into the root); doing it here gives immediate feedback in
        // onboarding before the engine ticks.
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
        if let Err(error) = persist_sync_root_and_start_engine(app, &state, &mut cfg, root.clone()).await {
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
    let mut warnings = Vec::new();
    let mut cfg = DesktopConfig::load()?;
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

    let mut engine_slot = state.engine.lock().await;
    if let Some(prev) = engine_slot.take() {
        prev.abort().await;
        tracing::info!("engine aborted for macOS integration reset");
    }
    drop(engine_slot);
    if let Ok(mut guard) = state.engine_state.lock() {
        *guard = "stopped".to_string();
    }

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

    let removed_socket = match remove_stale_ipc_socket() {
        Ok(removed) => removed,
        Err(error) => {
            warnings.push(error);
            false
        }
    };

    let removed_file_provider_domain = match remove_file_provider_domain() {
        Ok(()) => true,
        Err(error) => {
            warnings.push(format!("Could not remove Finder File Provider domain: {error}"));
            false
        }
    };

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

    persist_finder_install_result(&mut cfg, false, None)?;

    Ok(MacosIntegrationResetResult {
        removed_file_provider_domain,
        disabled_autostart,
        removed_socket,
        removed_cache_files,
        skipped_cache_files,
        pending_operations_preserved,
        sync_root_preserved,
        warnings,
    })
}

#[tauri::command]
fn open_finder_location(path: Option<String>) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let _ = path;
        let visible = file_provider_visible_location()?
            .ok_or_else(|| "Install the Beebeeb Finder location before opening it.".to_string())?;
        std::process::Command::new("open")
            .arg(&visible)
            .spawn()
            .map_err(|e| format!("open Finder: {e}"))?;
        return Ok(());
    }

    #[cfg(not(target_os = "macos"))]
    {
        let root = path
            .map(PathBuf::from)
            .or_else(|| DesktopConfig::load().ok().and_then(|c| c.sync_root))
            .unwrap_or_else(config::default_sync_root_suggestion);
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
/// File counts are read from the SQLite state DB at
/// `<sync_root>/.beebeeb/state.db` via a fresh connection per call.
/// SQLite is happy to share read access with the runner's connection
/// (default journal mode supports many concurrent readers); a poll
/// takes well under a millisecond on a typical vault. If the DB
/// doesn't exist yet (first launch, sync root not picked, or DB not
/// initialised because no engine has run yet) we return zeros.
#[tauri::command]
async fn sync_status(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    use state_db::FileStatus;

    let vault_unlocked = state.session.lock().map(|g| g.is_some()).unwrap_or(false);
    let auth_present = state.auth_present.lock().map(|g| *g).unwrap_or(false);
    let logged_in = vault_unlocked || auth_present;
    let sync_root_path = DesktopConfig::load().ok().and_then(|c| c.sync_root);
    #[cfg(target_os = "macos")]
    let sync_root = None::<String>;
    #[cfg(not(target_os = "macos"))]
    let sync_root = sync_root_path.as_ref().map(|p| p.to_string_lossy().into_owned());
    let engine_running = state.engine.lock().await.is_some();

    // Collapse the runner's five-state stream into the tri-state
    // string the WebView's settings pages render. `idle` and
    // `syncing` both mean "engine is alive and doing work" — UI
    // wants a single "running" pill for either; the per-tick
    // distinction is on the engine-status event stream.
    let engine = {
        let raw = state
            .engine_state
            .lock()
            .map(|g| g.clone())
            .unwrap_or_else(|_| "stopped".to_string());
        let engine = match raw.as_str() {
            "running" | "idle" | "syncing" => "running",
            "error" => "error",
            _ => "stopped",
        };
        if logged_in && !vault_unlocked && engine == "stopped" {
            "locked".to_string()
        } else {
            engine.to_string()
        }
    };

    // Count files by status, opening a short-lived connection at the
    // canonical state.db path. Wrap in a closure so an early `None`
    // on any step cleanly degrades to zeros without unwraps.
    let counts = sync_root_path
        .as_ref()
        .and_then(|root| {
            let db_path = root.join(".beebeeb").join("state.db");
            if !db_path.exists() {
                return None;
            }
            let db = state_db::StateDb::open(&db_path).ok()?;
            // Downloading + Uploading both count toward "in-flight
            // sync work" for the WebView's syncing pill — there's no
            // user-meaningful difference between "we're pulling" and
            // "we're pushing" at the indicator level.
            let downloading = db
                .list_by_status(FileStatus::Downloading)
                .ok()
                .map(|v| v.len() as u32)
                .unwrap_or(0);
            let uploading = db
                .list_by_status(FileStatus::Uploading)
                .ok()
                .map(|v| v.len() as u32)
                .unwrap_or(0);
            let cloud_only = db
                .list_by_status(FileStatus::CloudOnly)
                .ok()
                .map(|v| v.len() as u32)
                .unwrap_or(0);
            let conflicts = db
                .list_by_status(FileStatus::Conflict)
                .ok()
                .map(|v| v.len() as u32)
                .unwrap_or(0);
            Some((downloading + uploading, cloud_only, conflicts))
        })
        .unwrap_or((0, 0, 0));

    Ok(serde_json::json!({
        "logged_in": logged_in,
        "sync_root": sync_root,
        "engine_running": engine_running,
        "vault_unlocked": vault_unlocked,
        "engine": engine,
        "syncing": counts.0,
        "cloud_only": counts.1,
        "conflicts": counts.2,
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
    let token = {
        let guard = state.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
        guard
            .as_ref()
            .map(|session| session.token.clone())
            .ok_or_else(|| "vault is locked".to_string())?
    };

    let usage: BillingUsageResponse = reqwest::Client::new()
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

    let (cache_bytes, pinned_bytes) = DesktopConfig::load()
        .ok()
        .and_then(|cfg| cfg.sync_root)
        .and_then(|root| {
            let db_path = root.join(".beebeeb").join("state.db");
            if !db_path.exists() {
                return None;
            }
            let db = state_db::StateDb::open(&db_path).ok()?;
            let pinned = db.cache_bytes_by_effective_pin(true).ok()?.max(0);
            let unpinned = db.cache_bytes_by_effective_pin(false).ok()?.max(0);
            Some((pinned.saturating_add(unpinned), pinned))
        })
        .unwrap_or((0, 0));

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
    roots
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
    let db_path = sync_root.join(".beebeeb").join("state.db");
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
        // OS-level unpin BEFORE dehydrating. This file passed the DB pinned-skip
        // above, so it is MEANT to be reclaimed — but its placeholder may still
        // carry an OS pin (CfSetPinState) from an earlier "available offline"
        // toggle or an inherited recursive parent pin that the DB no longer
        // reflects. `CfDehydratePlaceholder` fails with ERROR_CLOUD_FILE_PINNED on
        // any OS-pinned placeholder, so we clear the OS pin first (non-recursive —
        // this is a single file). Best-effort: a failure here is logged (never the
        // path) and we still attempt the dehydrate, which simply no-ops/errors if
        // the file was genuinely pinned.
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
    db.evict_unpinned_cache_until_under(0, now)
        .map_err(|e| format!("evict unpinned cache: {e}"))?;

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

#[tauri::command]
fn export_diagnostics() -> Result<serde_json::Value, String> {
    let cfg = DesktopConfig::load()?;
    let Some(sync_root) = cfg.sync_root else {
        return Ok(serde_json::json!({
            "sync_root_configured": false,
            "queue": serde_json::Value::Null,
        }));
    };
    let db_path = sync_root.join(".beebeeb").join("state.db");
    if !db_path.exists() {
        return Ok(serde_json::json!({
            "sync_root_configured": true,
            "state_db_exists": false,
            "queue": serde_json::Value::Null,
        }));
    }
    let db = state_db::StateDb::open(&db_path).map_err(|e| format!("open state.db: {e}"))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let queue = db
        .queue_diagnostics(now)
        .map_err(|e| format!("queue diagnostics: {e}"))?;
    Ok(serde_json::json!({
        "sync_root_configured": true,
        "state_db_exists": true,
        "queue": queue,
    }))
}

// ── IPC commands: conflict/version center ───────────────────────────────────

#[tauri::command]
fn list_version_conflict_center() -> Result<Vec<engine_bridge::VersionConflictEntry>, String> {
    let cfg = DesktopConfig::load()?;
    let Some(sync_root) = cfg.sync_root else {
        return Ok(Vec::new());
    };
    let db_path = sync_root.join(".beebeeb").join("state.db");
    if !db_path.exists() {
        return Ok(Vec::new());
    }
    let db = state_db::StateDb::open(&db_path).map_err(|e| format!("open state.db: {e}"))?;
    engine_bridge::version_conflict_feed_from_db(&db).map_err(|e| e.to_string())
}

#[tauri::command]
async fn list_file_versions(state: State<'_, AppState>, file_id: String) -> Result<serde_json::Value, String> {
    let (token, master_key) = {
        let guard = state.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
        match guard.as_ref() {
            Some(s) => (s.token.clone(), s.master_key),
            None => return Err("not signed in".into()),
        }
    };

    let api = api_client::ApiClient::new(runner::api_base_url(), token, master_key);
    api.list_versions(&file_id).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn restore_file_version(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    file_id: String,
    version_id: String,
) -> Result<serde_json::Value, String> {
    let (token, master_key) = {
        let guard = state.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
        match guard.as_ref() {
            Some(s) => (s.token.clone(), s.master_key),
            None => return Err("not signed in".into()),
        }
    };

    let cfg = DesktopConfig::load()?;
    let sync_root = cfg.sync_root.ok_or_else(|| "no sync root configured".to_string())?;
    let db_path = sync_root.join(".beebeeb").join("state.db");
    let db = std::sync::Arc::new(state_db::StateDb::open(&db_path).map_err(|e| format!("open state.db: {e}"))?);
    let api = std::sync::Arc::new(api_client::ApiClient::new(runner::api_base_url(), token, master_key));
    let bridge = engine_bridge::EngineBridge::new(db, api.clone());

    match api.restore_version(&file_id, &version_id).await {
        Ok(body) => {
            let _ = app.emit(
                "version-restored",
                serde_json::json!({
                    "file_id": file_id,
                    "version_id": version_id,
                }),
            );
            Ok(body)
        }
        Err(e) => {
            let msg = e.to_string();
            if let Err(queue_err) =
                bridge.queue_restore_version(&file_id, &version_id, Some(format!("direct restore failed: {msg}")))
            {
                tracing::warn!(
                    error = %queue_err,
                    file_id = %file_id,
                    version_id = %version_id,
                    "failed to queue restore review operation"
                );
            }
            let _ = app.emit(
                "version-center-review",
                serde_json::json!({
                    "file_id": file_id,
                    "version_id": version_id,
                    "kind": "restore",
                }),
            );
            Err(format!(
                "restore failed; queued for review if state DB was available: {msg}"
            ))
        }
    }
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

        if !path.is_absolute() {
            return Err(format!(
                "picker returned a non-absolute path: {} (please report)",
                path.display()
            ));
        }
        config::ensure_directory(&path)?;

        let mut cfg = DesktopConfig::load()?;
        cfg.sync_root = Some(path.clone());
        cfg.save()?;

        tracing::info!(path = %path.display(), "sync root set");

        // If a session is already loaded, start syncing immediately.
        // (Common path: WebView calls set_session before pick_sync_root,
        //  so the engine couldn't start there; it starts here instead.)
        let session = state
            .session
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|s| (s.token.clone(), s.master_key)));
        if let Some((token, key)) = session {
            // Rehydrate the persisted pause state before spawning.
            state.sync_paused.store(cfg.pause_sync, Ordering::Relaxed);
            let pause_flag = state.sync_paused.clone();
            let mut engine_slot = state.engine.lock().await;
            if let Some(prev) = engine_slot.take() {
                prev.abort().await;
            }
            *engine_slot = Some(EngineRunner::spawn(app, path.clone(), token, key, pause_flag));
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
/// non-settings fields) untouched. Performs a load → merge → save so a
/// concurrent `pick_sync_root` doesn't race away a sync_root update;
/// the worst case here is a missed sync_root change between this load
/// and save, but `pick_sync_root` writes much less frequently than the
/// settings pages (one-shot vs. debounced sliders), so the practical
/// risk is near zero.
#[tauri::command]
fn set_desktop_config(config: config::DesktopSettings) -> Result<(), String> {
    let mut cfg = DesktopConfig::load()?;
    cfg.apply_settings(config);
    cfg.save()?;
    Ok(())
}

// ── IPC aliases for frontend compatibility ────────────────────────────────────

/// Alias so the frontend can call `desktop_config` (the name used in
/// `WindowsFirstRun.tsx` and `WindowsSettings.tsx`) in addition to the
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

/// Alias so the frontend (`WindowsTray.tsx`, `WindowsFirstRun.tsx`) can
/// call `show_settings_window` while the Rust side also keeps the
/// original `show_settings` command for any existing callers.
#[tauri::command]
fn show_settings_window(app: tauri::AppHandle) -> Result<(), String> {
    show_settings_window_impl(&app);
    Ok(())
}

/// Open (or focus) the main Beebeeb app window. This is the new primary
/// surface on Windows — the full sidebar + content-router shell
/// (`WindowsApp.tsx`, selected by `?window=main-app&platform=windows`) that
/// hosts the data views. The tray flyout's "Settings" button and the
/// onboarding "Open control center" button both route here; Settings remains
/// reachable from a nav item inside the app.
///
/// On non-Windows platforms there is no dedicated main-app window yet, so we
/// fall back to the existing settings window — keeping a single, predictable
/// entry point for callers regardless of OS.
#[tauri::command]
fn show_main_app_window(app: tauri::AppHandle) -> Result<(), String> {
    show_main_app_window_impl(&app);
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

/// Return recent sync activity for the Windows tray flyout.
///
/// Derives up to 8 items from the state DB:
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
    let Some(sync_root) = cfg.sync_root else {
        return Vec::new();
    };
    let db_path = sync_root.join(".beebeeb").join("state.db");
    if !db_path.exists() {
        return Vec::new();
    }
    let Ok(db) = state_db::StateDb::open(&db_path) else {
        return Vec::new();
    };

    let mut items: Vec<TrayActivityItem> = Vec::new();

    // In-flight transfers first (downloading + uploading, capped at 4 each).
    for status in [state_db::FileStatus::Downloading, state_db::FileStatus::Uploading] {
        let label = if matches!(status, state_db::FileStatus::Downloading) {
            "Downloading…"
        } else {
            "Uploading…"
        };
        let Ok(entries) = db.list_by_status(status) else { continue };
        for entry in entries.into_iter().take(4) {
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

    // Recently-synced local files (up to fill 8 total rows).
    let remaining = 8usize.saturating_sub(items.len());
    if remaining > 0 {
        if let Ok(locals) = db.list_by_status(state_db::FileStatus::Local) {
            // Sort by modified_at descending — most recently synced first.
            let mut locals = locals;
            locals.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));
            for entry in locals.into_iter().take(remaining) {
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
    }

    items
}

/// Pause the sync engine at runtime. Sets the shared `sync_paused` flag
/// so the runner's next tick no-ops. Persists `pause_sync = true` to
/// `desktop.toml` so the paused state survives a restart.
///
/// Emits an `engine-status` event with `state = "paused"` so the tray
/// tooltip and any open settings window update immediately.
#[tauri::command]
async fn tray_pause_sync(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state.sync_paused.store(true, Ordering::Relaxed);
    // Persist so the paused state survives a restart.
    let mut cfg = DesktopConfig::load()?;
    cfg.pause_sync = true;
    cfg.save()?;
    // Notify the WebView + tray tooltip listener.
    let _ = app.emit("engine-status", serde_json::json!({ "state": "paused" }));
    tracing::info!("sync paused via tray");
    Ok(())
}

/// Resume the sync engine. Clears the `sync_paused` flag so the runner's
/// next tick proceeds normally. Persists `pause_sync = false` to
/// `desktop.toml`. Emits an `engine-status` event with `state = "idle"`.
#[tauri::command]
async fn tray_resume_sync(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state.sync_paused.store(false, Ordering::Relaxed);
    let mut cfg = DesktopConfig::load()?;
    cfg.pause_sync = false;
    cfg.save()?;
    let _ = app.emit("engine-status", serde_json::json!({ "state": "idle" }));
    tracing::info!("sync resumed via tray");
    Ok(())
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
    let mut cfg = DesktopConfig::load()?;
    cfg.sync_mode = Some(mode);
    cfg.save()
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
    let mut cfg = DesktopConfig::load()?;
    let old_excluded = cfg.excluded_folder_ids.clone().unwrap_or_default();
    cfg.excluded_folder_ids = if excluded.is_empty() { None } else { Some(excluded.clone()) };
    cfg.save()?;

    // 2. Diff for the subtrees we must reclaim now.
    let newly_excluded = newly_excluded_ids(&old_excluded, &excluded);
    if newly_excluded.is_empty() {
        return Ok(());
    }

    // 3. Reclaim on a blocking thread. Failures here never undo the persist.
    let join = tokio::task::spawn_blocking(move || dehydrate_excluded_subtrees_blocking(&newly_excluded));
    match join.await {
        Ok(Ok(freed)) => {
            tracing::info!(bytes_freed = freed, "set_selective_sync: dehydrated newly-excluded subtrees");
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
/// Persists the new state to `desktop.toml`, then (on Windows) triggers an
/// immediate mirror pass so the user sees their files start backing up right
/// away rather than waiting for the next slow runner tick. The mirror is
/// one-way (source → `<sync_root>\<DisplayName>`); the existing enumeration scan
/// uploads whatever it copies in. Disabling only stops FUTURE copies — it does
/// NOT delete the already-backed-up vault copies (removing them would delete the
/// user's cloud vault content, which we never do silently).
///
/// Validates `key` against the catalog so a typo from the frontend can't write a
/// junk entry into the config.
#[tauri::command]
async fn set_known_folder_backup(key: String, enabled: bool) -> Result<(), String> {
    // Reject unknown keys — the catalog is the source of truth.
    if known_folder::known_folder_by_key(&key).is_none() {
        return Err(format!("unknown known-folder key: {key}"));
    }

    let mut cfg = DesktopConfig::load()?;
    cfg.set_known_folder(&key, enabled);
    cfg.save()?;

    // On enable, kick an immediate mirror so backup starts now. Best-effort:
    // a missing sync root or a copy error doesn't undo the persisted choice.
    // (On disable there's nothing to do — future passes just skip the folder.)
    //
    // NOTE (pin / available-offline): decision 0797 wants backed-up folders kept
    // available-offline (pinned). Pinning operates on Cloud Files PLACEHOLDERS,
    // which only exist AFTER the mirrored plain file has been uploaded and
    // converted to a placeholder — so there's nothing to pin at this point. The
    // honest path is to pin the vault subtree once it materialises as
    // placeholders (a follow-up that hooks the existing `set_recursive_pin` /
    // reconcile flow), not a no-op pin call here. TODO(0797-pin).
    #[cfg(target_os = "windows")]
    if enabled {
        if let Ok(cfg) = DesktopConfig::load() {
            if let Some(sync_root) = cfg.sync_root.clone() {
                let enabled_keys = cfg.known_folder_backup.clone();
                let join = tokio::task::spawn_blocking(move || {
                    known_folder::mirror_enabled_known_folders(&sync_root, &enabled_keys);
                });
                if let Err(e) = join.await {
                    tracing::warn!(error = %e, "set_known_folder_backup: immediate mirror task panicked");
                }
            } else {
                tracing::debug!("set_known_folder_backup: no sync root yet; mirror deferred to next tick");
            }
        }
    }

    Ok(())
}

/// Resolve and dehydrate the local, unpinned files under each newly-excluded
/// folder. Returns total bytes reclaimed. Shared by Windows (real CF
/// dehydrate) and non-Windows (DB-only flip) via the inner cfg blocks —
/// mirrors [`free_up_space_blocking`]. Best-effort: a missing sync root /
/// DB is `Ok(0)`, a per-file dehydrate failure logs and continues.
fn dehydrate_excluded_subtrees_blocking(newly_excluded: &[String]) -> Result<u64, String> {
    let cfg = DesktopConfig::load()?;
    let Some(sync_root) = cfg.sync_root.clone() else {
        return Ok(0);
    };
    let db_path = sync_root.join(".beebeeb").join("state.db");
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
/// [`try_decrypt_name`] as before.
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
    if leaf.is_empty() {
        None
    } else {
        Some(leaf.to_string())
    }
}

/// Stable id-derived fallback label, mirroring the existing top-level path.
fn folder_fallback_label(id: &str) -> String {
    let short = id.get(..8).unwrap_or(id);
    format!("Folder {short}…")
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
fn build_vault_tree(rows: &[VaultEntryRow], excluded: &std::collections::HashSet<String>) -> Vec<VaultItem> {
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
        if let Some(parent) = row.parent_id.as_ref() {
            if let Some(folder) = folders.get_mut(parent) {
                folder.direct_size = folder.direct_size.saturating_add(row.size_bytes.max(0));
                folder.direct_count = folder.direct_count.saturating_add(1);
                if row.on_disk {
                    folder.direct_on_disk = folder.direct_on_disk.saturating_add(row.size_bytes.max(0));
                }
            }
        }
    }

    // 3 + 4. Materialize recursively with a cycle guard.
    let mut visiting: std::collections::HashSet<String> = std::collections::HashSet::new();
    roots.sort_by(|a, b| folder_sort_key(&folders, a).cmp(&folder_sort_key(&folders, b)));
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
    child_ids.sort_by(|a, b| folder_sort_key(folders, a).cmp(&folder_sort_key(folders, b)));
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
/// Routes through the consolidated `beebeeb_core::encrypt::decrypt_name`, the
/// single decrypt path all clients share (web/cli/desktop). Unlike the previous
/// local helper — which handled only native `EncryptedBlob` blobs derived from
/// the BINARY UUID — `decrypt_name` covers the full format/key matrix: native
/// blobs AND web base64 `{nonce,ciphertext}` envelopes, derived from EITHER the
/// string-UUID (the web app's `TextEncoder.encode(fileId)`) or the binary UUID.
/// A bare plaintext name passes straight through. We deliberately swallow the
/// error to `None` so the SelectiveSync row falls back to a stable id label.
fn try_decrypt_name(file_meta: &serde_json::Value, id: &str, master_key: &[u8; 32]) -> Option<String> {
    let name_enc_str = file_meta.get("name_encrypted").and_then(|v| v.as_str())?;

    // MasterKey::from_bytes consumes the array (it zeroizes on drop),
    // so copy from the borrow first.
    let mk_bytes: [u8; 32] = *master_key;
    let mk = beebeeb_core::kdf::MasterKey::from_bytes(mk_bytes);

    beebeeb_core::encrypt::decrypt_name(&mk, id, name_enc_str).ok()
}

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
    // Lift the session pieces we need without holding the mutex over any
    // async API call.
    let creds = {
        let guard = state.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
        guard.as_ref().map(|s| (s.token.clone(), s.master_key))
    };
    let Some((token, master_key)) = creds else {
        // Not signed in — surface as empty rather than an error.
        return Ok(Vec::new());
    };

    let excluded: std::collections::HashSet<String> =
        DesktopConfig::load().ok().and_then(|c| c.excluded_folder_ids).unwrap_or_default().into_iter().collect();

    // Primary path: build from the local, already-nested state DB.
    if let Ok(Some(db)) = DesktopConfig::load().and_then(|cfg| state_db_for_config(&cfg)) {
        if let Ok(entries) = db.list_files() {
            let rows: Vec<VaultEntryRow> = entries
                .iter()
                .map(|e| VaultEntryRow {
                    id: e.file_id.clone(),
                    parent_id: e.parent_id.clone(),
                    is_folder: e.item_kind == state_db::ItemKind::Folder,
                    name: folder_leaf_name(&e.path).unwrap_or_else(|| folder_fallback_label(&e.file_id)),
                    size_bytes: e.size_bytes,
                    on_disk: e.status == state_db::FileStatus::Local,
                })
                .collect();
            let tree = build_vault_tree(&rows, &excluded);
            if !tree.is_empty() {
                return Ok(tree);
            }
            // DB present but no folders yet — fall through to the API stopgap.
        }
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
        let display = try_decrypt_name(f, id, &master_key)
            .or_else(|| {
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
    let db_path = sync_root.join(".beebeeb").join("state.db");
    if !db_path.exists() {
        return Err("The local sync database has not been created yet. Start the daemon once before changing offline availability.".to_string());
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
    let guard = state.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
    if let Some(email) = guard.as_ref().and_then(|s| s.email.clone()) {
        return Ok(Some(email));
    }
    drop(guard);
    Ok(state.auth_email.lock().ok().and_then(|guard| guard.clone()))
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
    let guard = state.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
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
    {
        let guard = state.cached_profile.lock().map_err(|_| "profile mutex poisoned".to_string())?;
        if let Some(profile) = guard.as_ref() {
            return Ok(profile.clone());
        }
    }
    let api = api_client_from_session(&state)?;
    let profile = api.account_profile().await.map_err(|e| e.to_string())?;
    if let Ok(mut guard) = state.cached_profile.lock() {
        *guard = Some(profile.clone());
    }
    Ok(profile)
}

/// `GET /api/v1/billing/subscription` — plan + quota + lifecycle.
#[tauri::command]
async fn account_subscription(state: State<'_, AppState>) -> Result<account_dto::Subscription, String> {
    api_client_from_session(&state)?
        .subscription()
        .await
        .map_err(|e| e.to_string())
}

/// `GET /api/v1/billing/usage` — used/quota bytes + percentage.
#[tauri::command]
async fn account_usage(state: State<'_, AppState>) -> Result<account_dto::BillingUsage, String> {
    api_client_from_session(&state)?
        .billing_usage()
        .await
        .map_err(|e| e.to_string())
}

/// `GET /api/v1/me/region` — preferred region + available list. The frontend
/// resolves the effective region's CITY from this (never the provider).
#[tauri::command]
async fn account_region(state: State<'_, AppState>) -> Result<account_dto::UserRegionResponse, String> {
    api_client_from_session(&state)?
        .account_region()
        .await
        .map_err(|e| e.to_string())
}

/// `GET /api/v1/account/activity` — recent events + security summary.
#[tauri::command]
async fn account_activity(state: State<'_, AppState>) -> Result<account_dto::AccountActivity, String> {
    api_client_from_session(&state)?
        .account_activity()
        .await
        .map_err(|e| e.to_string())
}

/// `GET /api/v1/account/security-score` — numeric score + factors.
#[tauri::command]
async fn account_security_score(state: State<'_, AppState>) -> Result<account_dto::SecurityScore, String> {
    api_client_from_session(&state)?
        .security_score()
        .await
        .map_err(|e| e.to_string())
}

/// `GET /api/v1/account/sessions` — active account sessions.
#[tauri::command]
async fn account_session_list(
    state: State<'_, AppState>,
) -> Result<account_dto::AccountSessionList, String> {
    api_client_from_session(&state)?
        .account_sessions()
        .await
        .map_err(|e| e.to_string())
}

/// `DELETE /api/v1/account/sessions/{id}` — revoke one session.
#[tauri::command]
async fn account_revoke_session(state: State<'_, AppState>, session_id: String) -> Result<(), String> {
    api_client_from_session(&state)?
        .revoke_account_session(&session_id)
        .await
        .map_err(|e| e.to_string())
}

/// `POST /api/v1/account/sessions/revoke-all-others` — revoke every other
/// session; returns the count revoked.
#[tauri::command]
async fn account_revoke_other_sessions(
    state: State<'_, AppState>,
) -> Result<account_dto::RevokeAllResult, String> {
    api_client_from_session(&state)?
        .revoke_other_sessions()
        .await
        .map_err(|e| e.to_string())
}

/// `GET /api/v1/clients/devices` — registered client devices.
#[tauri::command]
async fn account_devices(state: State<'_, AppState>) -> Result<account_dto::ClientDeviceList, String> {
    api_client_from_session(&state)?
        .client_devices()
        .await
        .map_err(|e| e.to_string())
}

/// `GET /api/v1/clients/sessions` — per-device sync sessions.
#[tauri::command]
async fn account_client_sessions(
    state: State<'_, AppState>,
) -> Result<account_dto::ClientSyncSessionList, String> {
    api_client_from_session(&state)?
        .client_sync_sessions()
        .await
        .map_err(|e| e.to_string())
}

/// `GET /api/v1/activity` — paginated audit-log feed. `page`/`limit` are
/// optional; `None` lets the server defaults apply.
#[tauri::command]
async fn account_activity_feed(
    state: State<'_, AppState>,
    page: Option<u32>,
    limit: Option<u32>,
) -> Result<account_dto::ActivityFeed, String> {
    api_client_from_session(&state)?
        .activity_feed(page, limit)
        .await
        .map_err(|e| e.to_string())
}

/// `GET /api/v1/notifications` — notification list + unread count.
#[tauri::command]
async fn account_notifications(
    state: State<'_, AppState>,
) -> Result<account_dto::NotificationList, String> {
    api_client_from_session(&state)?
        .notifications()
        .await
        .map_err(|e| e.to_string())
}

/// `GET /api/v1/notifications/preferences` — push-preference toggles.
#[tauri::command]
async fn account_notification_preferences(
    state: State<'_, AppState>,
) -> Result<account_dto::NotificationPreferences, String> {
    api_client_from_session(&state)?
        .notification_preferences()
        .await
        .map_err(|e| e.to_string())
}

/// `PUT /api/v1/notifications/preferences` — partial update; omitted fields
/// keep their current server value.
#[tauri::command]
async fn account_update_notification_preferences(
    state: State<'_, AppState>,
    update: account_dto::NotificationPreferencesUpdate,
) -> Result<account_dto::NotificationPreferences, String> {
    api_client_from_session(&state)?
        .update_notification_preferences(&update)
        .await
        .map_err(|e| e.to_string())
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
    // Auth gate: require an unlocked session, same signal as the others.
    {
        let guard = state.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
        if guard.is_none() {
            return Err("vault is locked".to_string());
        }
    }

    let limit = largest_limit.unwrap_or(10);

    // Resolve the state DB from the configured sync root, mirroring
    // `desktop_storage_summary`. No root / no DB → empty (not an error).
    let db_path = match DesktopConfig::load().ok().and_then(|cfg| cfg.sync_root) {
        Some(root) => root.join(".beebeeb").join("state.db"),
        None => return Ok(account_dto::compute_storage_breakdown(&[], limit)),
    };
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
    // Auth gate: require an unlocked session, same signal as the others.
    {
        let guard = state.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
        if guard.is_none() {
            return Err("vault is locked".to_string());
        }
    }

    let limit = recent_limit.unwrap_or(15);

    // Resolve the state DB from the configured sync root, mirroring
    // `account_storage_breakdown`. No root / no DB → empty (not an error).
    let db_path = match DesktopConfig::load().ok().and_then(|cfg| cfg.sync_root) {
        Some(root) => root.join(".beebeeb").join("state.db"),
        None => return Ok(account_dto::compute_file_overview(&[], limit)),
    };
    if !db_path.exists() {
        return Ok(account_dto::compute_file_overview(&[], limit));
    }

    // SQLite work is blocking; run it off the async executor (mirrors
    // `account_storage_breakdown`).
    let overview = tokio::task::spawn_blocking(move || -> Result<_, String> {
        let db = state_db::StateDb::open(&db_path).map_err(|e| format!("open state db: {e}"))?;
        let rows = db.file_overview_rows().map_err(|e| format!("list files: {e}"))?;
        // Folders carry no real size and no sync state worth surfacing; exclude
        // them from the overview.
        let inputs: Vec<account_dto::FileOverviewInput> = rows
            .into_iter()
            .filter(|(e, _)| !e.is_dir())
            .map(|(e, pinned)| account_dto::FileOverviewInput {
                path: e.path,
                size_bytes: e.size_bytes,
                status: e.status.as_str().to_string(),
                pinned,
                modified_at: e.modified_at,
            })
            .collect();
        Ok(account_dto::compute_file_overview(&inputs, limit))
    })
    .await
    .map_err(|e| format!("file overview task failed: {e}"))??;

    Ok(overview)
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

    let url = format!(
        "index.html?window=conflict&fileId={file_id}&fileName={enc_name}&isText={is_text}",
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
/// instead of the default settings shell.
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
) -> Result<(), String> {
    open_conflict_window_impl(&app, &file_id, &file_name, is_text)
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
pub(crate) fn notify_conflict_impl(app: &tauri::AppHandle, file_name: &str) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;

    // Honour the user's preference. Best-effort load — if the config
    // file is unreadable we fall back to "show", matching the
    // documented Default of `notify_conflicts: true`.
    let want = DesktopConfig::load().map(|c| c.notify_conflicts).unwrap_or(true);
    if !want {
        return Ok(());
    }

    app.notification()
        .builder()
        .title("Conflict detected")
        .body(format!("Conflict in {file_name} — open Beebeeb to resolve"))
        .show()
        .map_err(|e| e.to_string())
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
    // Validate the choice up front so a typo on the TS side fails
    // loudly instead of silently no-opping.
    match choice.as_str() {
        "local" | "remote" | "both" => {}
        _ => return Err(format!("invalid conflict choice: {choice}")),
    }

    // Pull token + master_key out of the in-memory session. We
    // intentionally do NOT clone the entire Session struct — only
    // what the ApiClient needs.
    let (token, master_key) = {
        let guard = state.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
        match guard.as_ref() {
            Some(s) => (s.token.clone(), s.master_key),
            None => return Err("not signed in".into()),
        }
    };

    // Sync root from desktop.toml — without it the daemon has no
    // place to write the resolved file, so this is a hard error.
    let cfg = DesktopConfig::load()?;
    let sync_root = cfg.sync_root.ok_or_else(|| "no sync root configured".to_string())?;

    // Build a fresh bridge. The state DB is at a deterministic path
    // under the sync root (same convention as `runner::run`).
    let db_path = sync_root.join(".beebeeb").join("state.db");
    let db = std::sync::Arc::new(state_db::StateDb::open(&db_path).map_err(|e| format!("open state.db: {e}"))?);
    let api = std::sync::Arc::new(api_client::ApiClient::new(runner::api_base_url(), token, master_key));
    let bridge = engine_bridge::EngineBridge::new(db, api);

    let outcome: Result<(), String> = match choice.as_str() {
        "local" => bridge.resolve_keep_mine(&file_id).await.map_err(|e| e.to_string()),
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

    // Notify the settings window so it can re-poll sync_status and
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
}

// ── IPC commands: app metadata ────────────────────────────────────────────────

// ── Durations ─────────────────────────────────────────────────────────────────

/// Delay after launch before the first update check (don't block startup).
const UPDATE_CHECK_STARTUP_DELAY_SECS: u64 = 5;
/// How often to re-check for updates while the app is running.
const UPDATE_CHECK_INTERVAL_HOURS: u64 = 4;

/// Returns the app version string — displayed in the web UI Settings page.
#[tauri::command]
fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Download and install the pending update, then relaunch.
///
/// Called by the frontend when the user clicks "Restart to update" in the
/// update-available banner. Returns an error string if installation fails.
#[tauri::command]
async fn install_update(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;

    let updater = app.updater().map_err(|e| e.to_string())?;
    let update = updater.check().await.map_err(|e| e.to_string())?;

    match update {
        Some(u) => {
            tracing::info!(version = %u.version, "downloading update");

            u.download_and_install(
                |downloaded, total| {
                    // Emit progress so the frontend can show a progress bar.
                    // downloaded: usize, total: Option<u64>
                    let pct = total
                        .map(|t| if t > 0 { (downloaded as u64) * 100 / t } else { 0 })
                        .unwrap_or(0);
                    tracing::debug!("update download progress: {pct}%");
                },
                || {
                    tracing::info!("update installed — relaunching");
                },
            )
            .await
            .map_err(|e| e.to_string())?;

            app.restart();
        }
        None => {
            tracing::info!("install_update called but no update available");
        }
    }

    Ok(())
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .try_init();

    tauri::Builder::default()
        // Shared state — session pushed in by the WebView via set_session
        // IPC after login. The future sync engine task reads from this.
        .manage(AppState::default())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
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
            install_update,
            toggle_autostart,
            autostart_enabled,
            set_session,
            clear_session,
            unlock_vault,
            desktop_unlock_with_recovery_phrase,
            lock_vault,
            sync_status,
            desktop_storage_summary,
            free_up_space,
            export_diagnostics,
            list_version_conflict_center,
            list_file_versions,
            restore_file_version,
            get_sync_root,
            pick_sync_root,
            default_sync_root,
            desktop_platform,
            finder_location_state,
            install_finder_location,
            continue_without_finder_location,
            // Windows shell integration (Cloud Files) — parallels the macOS finder cmds
            windows_shell_integration_state,
            install_windows_shell_integration,
            reset_macos_integration,
            open_finder_location,
            // Task 9 — settings page IPC
            get_desktop_config,
            set_desktop_config,
            account_email,
            // PKG-DATA — account / billing / devices / activity data layer.
            // Wrappers over existing server endpoints (the "all pages empty"
            // fix) plus the locally-computed storage breakdown.
            account_profile,
            account_subscription,
            account_usage,
            account_region,
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
            account_storage_breakdown,
            desktop_file_overview,
            // Task 0090 — selective sync
            get_selective_sync,
            set_selective_sync,
            // Task 0797 — known-folder backup ("Manage backup")
            get_known_folder_backup,
            set_known_folder_backup,
            list_vault_folders,
            list_remote_tree,
            set_recursive_pin,
            // Task 12 — conflict window IPC
            open_conflict_window,
            resolve_conflict,
            // Task 11 — native conflict notification
            notify_conflict,
            // First-launch onboarding (login + folder picker + sync status)
            desktop_login,
            // 2FA second-factor completion for OPAQUE sign-in
            desktop_login_2fa,
            // Browser-based device-code login handoff (Windows primary path)
            browser_login::start_browser_login,
            open_onboarding_window,
            show_settings,
            // WS1 — Windows UI aliases + tray commands
            show_settings_window,
            // PKG-SHELL — open the Windows main app window
            show_main_app_window,
            desktop_config,
            tray_recent_activity,
            tray_pause_sync,
            tray_resume_sync,
            set_sync_mode,
        ])
        .setup(|app| {
            setup_native_menu(app)?;
            setup_tray(app)?;
            // Defensive: hide windows that tauri_plugin_window_state may have
            // restored visible on Windows. Only the onboarding window should
            // appear on first-run; tray and settings surfaces are on-demand.
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
                // Windows settings surface — shown only on user action.
                if let Some(w) = app.get_webview_window("windows-settings") {
                    let _ = w.hide();
                }
            }
            attach_tray_status_listener(&app.handle().clone());
            {
                let app_state = app.state::<AppState>();
                if let Ok(mut guard) = app_state.auth_present.lock() {
                    *guard = keychain_session_present();
                }
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
                });
            }

            // Spawn background update checker
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                run_update_loop(handle).await;
            });

            // First-launch detection: if no sync_root is configured and no
            // session is present, open the onboarding window automatically.
            // The settings window stays hidden (visible:false in tauri.conf.json)
            // until the user completes onboarding.
            let no_sync_root = DesktopConfig::load().map(|c| c.sync_root.is_none()).unwrap_or(true);
            if no_sync_root {
                let h = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    // Small delay so the tray and menu are fully initialised first.
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                    if let Err(e) = open_onboarding_window_impl(&h) {
                        tracing::warn!(error = %e, "failed to open onboarding window");
                    }
                });
            } else {
                // Already configured — show the main app window after startup
                // settles. On Windows this is the full sidebar/content shell
                // (`main-app`); on macOS/Linux `show_main_app_window_impl`
                // delegates to the existing settings window, so behaviour there
                // is unchanged. The short delay mirrors onboarding and avoids
                // racing Tauri's window setup.
                let h = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                    show_main_app_window_impl(&h);
                });
            }

            Ok(())
        })
        // Cmd+W / "Close Window" → hide to tray
        .on_menu_event(|app, event| {
            if event.id().as_ref() == "close_window" {
                if let Some(win) = app.get_webview_window("settings") {
                    let _ = win.hide();
                }
            }
        })
        // Red-dot close button → hide to tray instead of quitting
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Beebeeb desktop");
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
    use tauri_plugin_updater::UpdaterExt;

    let updater = match app.updater() {
        Ok(u) => u,
        Err(e) => {
            tracing::debug!("updater not available: {e}");
            return;
        }
    };

    match updater.check().await {
        Ok(Some(update)) => {
            tracing::info!(
                version = %update.version,
                "update available — notifying frontend"
            );
            // Emit to the webview so the frontend can show a banner.
            // Frontend listens with: import { listen } from "@tauri-apps/api/event"
            //   await listen("update-available", handler)
            let _ = app.emit(
                "update-available",
                serde_json::json!({
                    "version": update.version,
                    "body": update.body.as_deref().unwrap_or(""),
                }),
            );
        }
        Ok(None) => {
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

fn setup_native_menu(app: &mut tauri::App) -> tauri::Result<()> {
    // Beebeeb (app-name menu — macOS leftmost)
    let beebeeb_menu = Submenu::with_items(
        app,
        "Beebeeb",
        true,
        &[
            &PredefinedMenuItem::about(app, Some("About Beebeeb"), Some(AboutMetadata::default()))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::show_all(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, Some("Quit Beebeeb"))?,
        ],
    )?;

    // File
    let file_menu = Submenu::with_items(
        app,
        "File",
        true,
        &[
            // Cmd+W hides the window (standard macOS behaviour for apps that
            // live in the menu bar — it doesn't quit the app).
            &MenuItemBuilder::new("Close Window")
                .id("close_window")
                .accelerator("CmdOrCtrl+W")
                .build(app)?,
        ],
    )?;

    // Edit — essential for WebView text-input fields to get clipboard bindings
    let edit_menu = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;

    let menu = Menu::with_items(app, &[&beebeeb_menu, &file_menu, &edit_menu])?;
    app.set_menu(menu)?;
    Ok(())
}

// ── System tray ──────────────────────────────────────────────────────────────

/// Build the tray context menu with the current autostart checked state.
/// Called on first setup and again whenever the autostart state changes.
///
/// As of Task 8B the desktop app is tray-only — the WebView is a small
/// fixed-size settings window rather than the full web client at
/// `app.beebeeb.io`. The "Open Settings" tray item is the primary way
/// users surface that window; "Hide" tucks it back without quitting
/// the daemon.
fn build_tray_menu<M: tauri::Manager<tauri::Wry>>(
    manager: &M,
    autostart_enabled: bool,
) -> tauri::Result<Menu<tauri::Wry>> {
    // "Open Beebeeb" surfaces the main app window (the full sidebar/content
    // shell on Windows; the settings window on macOS/Linux). The menu-item id
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
fn attach_tray_status_listener(app: &tauri::AppHandle) {
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
        let state = payload.get("state").and_then(|v| v.as_str()).unwrap_or("");
        let files_remaining = payload.get("files_remaining").and_then(|v| v.as_u64());
        let error = payload.get("error").and_then(|v| v.as_str());

        // Mirror the latest state into AppState so the `sync_status`
        // command can return it on first paint without subscribing
        // to the event stream itself. Failures here (poisoned mutex,
        // state not registered yet) are non-fatal — the tray tooltip
        // update below still runs.
        if !state.is_empty()
            && let Some(app_state) = app_for_listener.try_state::<AppState>()
            && let Ok(mut guard) = app_state.engine_state.lock()
        {
            *guard = state.to_string();
        }

        let tooltip = match state {
            "idle" => "Beebeeb · Synced".to_string(),
            "syncing" => match files_remaining {
                Some(0) | None => "Beebeeb · Syncing…".to_string(),
                Some(1) => "Beebeeb · Syncing 1 file…".to_string(),
                Some(n) => format!("Beebeeb · Syncing {n} files…"),
            },
            "paused" => "Beebeeb · Paused".to_string(),
            "offline" => "Beebeeb · Offline".to_string(),
            "error" => match error {
                Some(msg) if !msg.is_empty() => format!("Beebeeb · Error: {msg}"),
                _ => "Beebeeb · Error".to_string(),
            },
            "stopped" => "Beebeeb · Not signed in".to_string(),
            other => format!("Beebeeb · {other}"),
        };

        if let Some(tray) = app_for_listener.tray_by_id("tray")
            && let Err(e) = tray.set_tooltip(Some(&tooltip))
        {
            tracing::warn!(error = %e, "failed to update tray tooltip");
        }
    });
}

/// Suppress the Windows 11 DWM window outline (rounded-rect border + system
/// shadow) on the given Tauri window, leaving only what the WebView paints.
///
/// The tray flyout window is frameless + transparent, yet Win11's DWM still
/// draws a rounded border and shadow on the window *rectangle* — visible as a
/// faint "frame" around the white card (which is inset 8px inside the window).
/// Setting `DWMWA_WINDOW_CORNER_PREFERENCE = DWMWCP_DONOTROUND` drops the
/// rounded outline; `DWMWA_BORDER_COLOR = DWMWA_COLOR_NONE` removes the 1px DWM
/// border line. The card's own CSS shadow then floats it instead.
///
/// HWND version bridge: our DIRECT `windows` dep is 0.58, but Tauri 2.x pulls
/// `windows` 0.61, so `tray_window.hwnd()` hands back a 0.61 `HWND` — a DISTINCT
/// type from the 0.58 `HWND` that `DwmSetWindowAttribute@0.58` wants. Both
/// newtypes wrap `*mut c_void`, so we reconstruct the 0.58 HWND from the raw
/// pointer (`raw.0 as _`). Any failure is logged and ignored — this is a purely
/// cosmetic refinement, never fatal.
#[cfg(target_os = "windows")]
fn disable_dwm_window_frame(window: &tauri::WebviewWindow) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE,
        DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
    };

    let raw = match window.hwnd() {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!(error = %e, "tray frame: could not get HWND; leaving DWM default");
            return;
        }
    };
    // Bridge windows@0.61 HWND -> windows@0.58 HWND (both are *mut c_void).
    let hwnd = HWND(raw.0 as _);

    // SAFETY: `hwnd` is a live top-level window handle owned by this process.
    // Both attributes take a value the size of their pointed-to constant; we
    // pass the exact byte length (4) for each i32/u32 payload.
    unsafe {
        let pref = DWMWCP_DONOTROUND; // DWM_WINDOW_CORNER_PREFERENCE(1)
        if let Err(e) = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            std::ptr::addr_of!(pref) as *const core::ffi::c_void,
            std::mem::size_of_val(&pref) as u32,
        ) {
            tracing::warn!(error = %e, "tray frame: DWMWA_WINDOW_CORNER_PREFERENCE failed");
        }

        let border_none = DWMWA_COLOR_NONE; // 0xFFFFFFFE — suppress DWM border line
        if let Err(e) = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            std::ptr::addr_of!(border_none) as *const core::ffi::c_void,
            std::mem::size_of_val(&border_none) as u32,
        ) {
            tracing::warn!(error = %e, "tray frame: DWMWA_BORDER_COLOR failed");
        }
    }
}

fn setup_tray(app: &mut tauri::App) -> tauri::Result<()> {
    // Read current autostart state so the check mark is correct on launch.
    let autostart_enabled = app.autolaunch().is_enabled().unwrap_or(false);
    let tray_menu = build_tray_menu(app, autostart_enabled)?;

    // Windows 11 draws a rounded outline + shadow on the transparent tray
    // flyout's window rectangle, which reads as a faint frame around the white
    // card. Turn that off once at startup (the window is declared statically in
    // tauri.conf.json, so it already exists here). Non-fatal cosmetic step.
    #[cfg(target_os = "windows")]
    if let Some(tray_window) = app.get_webview_window("tray") {
        disable_dwm_window_frame(&tray_window);
    }

    let icon = app
        .default_window_icon()
        .cloned()
        .expect("no app icon found — check icons/icon.png exists");

    let _tray = TrayIconBuilder::with_id("tray")
        .icon(icon)
        .icon_as_template(true) // macOS: monochrome, respects dark/light mode
        .menu(&tray_menu)
        .show_menu_on_left_click(false)
        .tooltip("Beebeeb")
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open_settings" => show_main_app_window_impl(app),
            "tray_hide" => {
                if let Some(win) = app.get_webview_window("settings") {
                    let _ = win.hide();
                }
            }
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
                if let Some(tray) = app.tray_by_id("tray") {
                    if let Ok(menu) = build_tray_menu(app, new_state) {
                        let _ = tray.set_menu(Some(menu));
                    }
                }
                tracing::info!(enabled = new_state, "autostart toggled via tray");
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // Left-click toggles window visibility.
            // On Windows: show/hide the frameless tray flyout ("tray" window),
            // positioned near the bottom-right corner so it appears above the
            // system tray area. The "settings" window is only opened via the
            // context-menu "Open Settings" item or the tray flyout itself.
            // On macOS/Linux: toggle the main settings window as before.
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                position,
                ..
            } = event
            {
                let app = tray.app_handle();

                #[cfg(target_os = "windows")]
                {
                    if let Some(win) = app.get_webview_window("tray") {
                        if win.is_visible().unwrap_or(false) {
                            let _ = win.hide();
                        } else {
                            // Position the flyout above the tray icon.
                            // The window is 400×390; place it so the bottom-right
                            // corner aligns with the click position (tray icon centre).
                            // The tray event's `position` is already in physical
                            // pixels, so we pass `Position::Physical` and offset in
                            // the same (physical) coordinate space — no DPI
                            // conversion needed here. The Y offset matches the
                            // window height (+ a small gap) so the flyout sits
                            // above, not over, the taskbar.
                            let _ = win.set_position(tauri::Position::Physical(
                                tauri::PhysicalPosition {
                                    x: (position.x as i32).saturating_sub(400),
                                    y: (position.y as i32).saturating_sub(400),
                                },
                            ));
                            let _ = win.show();
                            let _ = win.set_focus();
                        }
                    }
                }

                #[cfg(not(target_os = "windows"))]
                {
                    let _ = position; // unused on non-Windows
                    if let Some(win) = app.get_webview_window("settings") {
                        if win.is_visible().unwrap_or(false) {
                            let _ = win.hide();
                        } else {
                            show_settings_window_impl(app);
                        }
                    }
                }
            }
        })
        .build(app)?;

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Internal helper — show/create the appropriate settings window for the
/// current platform. Not an IPC command; callers use the `show_settings`
/// or `show_settings_window` IPC commands from the frontend.
fn show_settings_window_impl(app: &tauri::AppHandle) {
    // Windows ships a dedicated, larger settings shell (`windows-settings`)
    // whose React route is selected by the `platform=windows` query param.
    // macOS keeps the original compact `settings` window untouched.
    #[cfg(target_os = "windows")]
    let (label, url, width, height, resizable) = (
        "windows-settings",
        "index.html?window=settings&platform=windows",
        1000.0,
        680.0,
        true,
    );
    #[cfg(not(target_os = "windows"))]
    let (label, url, width, height, resizable) = ("settings", "index.html", 680.0, 540.0, false);

    if let Some(win) = app.get_webview_window(label) {
        let _ = win.show();
        let _ = win.set_focus();
        return;
    }

    match tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::App(url.into()))
        .title("Beebeeb")
        .inner_size(width, height)
        .resizable(resizable)
        .center()
        .build()
    {
        Ok(win) => {
            let _ = win.show();
            let _ = win.set_focus();
        }
        Err(error) => {
            tracing::warn!(%error, "failed to create settings window");
        }
    }
}

/// Internal helper — show/create the main Beebeeb app window. On Windows this
/// is the `main-app` webview (the full sidebar + content-router shell,
/// `WindowsApp.tsx`, selected by `?window=main-app&platform=windows`). On
/// other platforms there is no dedicated main-app shell yet, so we delegate to
/// `show_settings_window_impl` — callers get a sensible window either way.
///
/// Mirrors `show_settings_window_impl`: focus an existing window if present,
/// otherwise build one. The window is declared `visible: false` in
/// tauri.conf.json, so the explicit `show()` + `set_focus()` are required.
fn show_main_app_window_impl(app: &tauri::AppHandle) {
    #[cfg(not(target_os = "windows"))]
    {
        // No dedicated main-app window outside Windows — reuse settings.
        show_settings_window_impl(app);
        return;
    }

    #[cfg(target_os = "windows")]
    {
        let label = "main-app";
        let url = "index.html?window=main-app&platform=windows";

        if let Some(win) = app.get_webview_window(label) {
            let _ = win.show();
            let _ = win.set_focus();
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
            }
            Err(error) => {
                tracing::warn!(%error, "failed to create main app window");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AppState, VaultEntryRow, build_vault_tree, classify_finder_install_error, clear_cached_profile,
        finder_install_state_from_config, folder_leaf_name, is_disposable_cache_path, newly_excluded_ids,
        normalize_recovery_phrase_input, subtree_file_ids,
    };
    use crate::account_dto::AccountProfile;
    use crate::config::DesktopConfig;
    use std::collections::HashSet;

    fn folder(id: &str, parent: Option<&str>, name: &str) -> VaultEntryRow {
        VaultEntryRow {
            id: id.to_string(),
            parent_id: parent.map(str::to_string),
            is_folder: true,
            name: name.to_string(),
            size_bytes: 0,
            on_disk: false,
        }
    }

    fn file(id: &str, parent: Option<&str>, size: i64, on_disk: bool) -> VaultEntryRow {
        VaultEntryRow {
            id: id.to_string(),
            parent_id: parent.map(str::to_string),
            is_folder: false,
            name: String::new(),
            size_bytes: size,
            on_disk,
        }
    }

    #[test]
    fn clear_cached_profile_drops_stale_identity() {
        let state = AppState::default();
        // Seed a cached profile, as login would.
        {
            let mut guard = state.cached_profile.lock().unwrap();
            *guard = Some(AccountProfile {
                user_id: "u1".into(),
                email: "old@example.com".into(),
                email_verified: true,
                created_at: "2026-01-01T00:00:00Z".into(),
                frozen_at: None,
                role: Some("user".into()),
                totp_enabled: Some(false),
                is_impersonation: Some(false),
                admin_user_id: None,
            });
        }
        assert!(state.cached_profile.lock().unwrap().is_some());

        // Logout/lock path clears it so account_profile can't serve a stale hit.
        clear_cached_profile(&state);
        assert!(
            state.cached_profile.lock().unwrap().is_none(),
            "cached profile must be cleared on lock/logout"
        );
    }

    #[test]
    fn normalizes_recovery_phrase_in_rust() {
        let input = "Alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima";
        let normalized = normalize_recovery_phrase_input(input).expect("phrase should normalize");

        assert_eq!(
            normalized,
            "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima"
        );
    }

    #[test]
    fn rejects_non_twelve_word_recovery_phrase() {
        let error = normalize_recovery_phrase_input("alpha bravo charlie").expect_err("phrase should be incomplete");

        assert_eq!(error, "Recovery phrase must contain exactly 12 words.");
    }

    #[test]
    fn classifies_file_provider_setup_errors() {
        assert_eq!(
            classify_finder_install_error("Entitlement com.apple.security.application-groups is ignored"),
            "provisioning"
        );
        assert_eq!(
            classify_finder_install_error("NSFileProviderErrorDomain Code=-2011"),
            "disabled"
        );
        assert_eq!(
            classify_finder_install_error("Timed out waiting for the Beebeeb File Provider domain"),
            "timeout"
        );
    }

    #[test]
    fn maps_persisted_file_provider_error_into_state() {
        let cfg = DesktopConfig {
            finder_install_status: Some("error".to_string()),
            finder_install_last_error: Some("Timed out waiting for setup".to_string()),
            finder_install_last_attempt_at: Some(42),
            finder_install_reason_category: Some("timeout".to_string()),
            ..DesktopConfig::default()
        };

        let state = finder_install_state_from_config(&cfg, false, None);

        assert!(!state.installed);
        assert_eq!(state.status, "error");
        assert_eq!(state.last_error.as_deref(), Some("Timed out waiting for setup"));
        assert_eq!(state.last_attempt_at, Some(42));
        assert_eq!(state.reason_category.as_deref(), Some("timeout"));
    }

    #[test]
    fn session_zeroizes_master_key_on_drop() {
        use zeroize::Zeroize;

        // Compile-time guarantee: `Session` derives `ZeroizeOnDrop` (which
        // requires `Drop`). If the derive is ever removed this stops compiling.
        fn assert_zeroize_on_drop<T: zeroize::ZeroizeOnDrop>() {}
        assert_zeroize_on_drop::<super::Session>();

        // Sanity-check the field types the derive relies on are themselves
        // `Zeroize` — the property the `ZeroizeOnDrop` derive expands to per
        // field. `[u8; 32]`, `String`, and `Option<String>` all qualify.
        let mut key = [7u8; 32];
        key.zeroize();
        assert_eq!(key, [0u8; 32]);
        let mut token = String::from("super-secret-session-token");
        token.zeroize();
        assert!(token.is_empty());

        // Exercise the real drop path (no panic / double-free) on a populated
        // session; the wipe itself runs inside `Drop` where the bytes are no
        // longer observably aliased.
        let session = super::Session {
            token: "tok".to_string(),
            master_key: [9u8; 32],
            email: Some("user@example.com".to_string()),
        };
        drop(session);
    }

    #[test]
    fn disposable_cache_path_check_accepts_temp_files_only() {
        let temp_file = std::env::temp_dir().join("beebeeb-disposable-cache-test");
        std::fs::write(&temp_file, b"cache").unwrap();
        assert!(is_disposable_cache_path(&temp_file));
        let _ = std::fs::remove_file(&temp_file);

        let config_path = DesktopConfig::path().unwrap();
        assert!(!is_disposable_cache_path(&config_path));
    }

    // ── SelectiveSync wave-2: tree-build + aggregation + exclude-diff ──────────

    #[test]
    fn folder_leaf_name_takes_last_path_segment() {
        assert_eq!(folder_leaf_name("/Docs/Taxes").as_deref(), Some("Taxes"));
        assert_eq!(folder_leaf_name("Photos").as_deref(), Some("Photos"));
        assert_eq!(folder_leaf_name("/Photos/").as_deref(), Some("Photos"));
        assert_eq!(folder_leaf_name(""), None);
        assert_eq!(folder_leaf_name("/"), None);
    }

    #[test]
    fn build_tree_nests_folders_and_aggregates_subtree() {
        // root
        //  ├─ A (folder)
        //  │   ├─ a1.txt  100  on_disk
        //  │   └─ B (folder)
        //  │        └─ b1.txt 50  cloud-only
        //  └─ C (folder)  (empty)
        let rows = vec![
            folder("A", None, "Alpha"),
            folder("B", Some("A"), "Bravo"),
            folder("C", None, "Charlie"),
            file("a1", Some("A"), 100, true),
            file("b1", Some("B"), 50, false),
        ];
        let excluded = HashSet::new();
        let tree = build_vault_tree(&rows, &excluded);

        // Two roots, sorted by name: Alpha, Charlie.
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].id, "A");
        assert_eq!(tree[1].id, "C");

        let a = &tree[0];
        // A aggregates a1 (100) + b1 (50) = 150 bytes, 2 files, 100 on disk.
        assert_eq!(a.size_bytes, 150);
        assert_eq!(a.file_count, 2);
        assert_eq!(a.on_disk_bytes, 100);
        assert!(a.is_folder);

        // A has one child folder B.
        assert_eq!(a.children.len(), 1);
        let b = &a.children[0];
        assert_eq!(b.id, "B");
        assert_eq!(b.size_bytes, 50);
        assert_eq!(b.file_count, 1);
        assert_eq!(b.on_disk_bytes, 0);

        // C is empty.
        let c = &tree[1];
        assert_eq!(c.size_bytes, 0);
        assert_eq!(c.file_count, 0);
        assert!(c.children.is_empty());
    }

    #[test]
    fn build_tree_marks_excluded_and_dangling_parents_become_roots() {
        let rows = vec![
            folder("A", None, "Alpha"),
            // D's parent points at an unknown folder → must surface as a root,
            // never silently dropped.
            folder("D", Some("ghost"), "Delta"),
            file("a1", Some("A"), 10, true),
        ];
        let mut excluded = HashSet::new();
        excluded.insert("A".to_string());

        let tree = build_vault_tree(&rows, &excluded);
        let ids: Vec<&str> = tree.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&"A"));
        assert!(ids.contains(&"D"), "dangling-parent folder must still appear as a root");

        let a = tree.iter().find(|n| n.id == "A").unwrap();
        assert!(a.excluded, "folder in the excluded set must be marked excluded");
        let d = tree.iter().find(|n| n.id == "D").unwrap();
        assert!(!d.excluded);
    }

    #[test]
    fn build_tree_is_cycle_safe() {
        // A → B → A parent cycle (corrupt data). With edges A→B and B→A,
        // BOTH nodes have a known parent inside the folder set, so neither
        // qualifies as a root. The builder produces an empty tree (roots = [])
        // rather than infinite-looping. The assertion verifies termination —
        // tree.len() is 0 in practice; the ≤ 2 upper bound accommodates any
        // future builder change that surfaces cycle members as roots.
        let rows = vec![folder("A", Some("B"), "Alpha"), folder("B", Some("A"), "Bravo")];
        let tree = build_vault_tree(&rows, &HashSet::new());
        // No panic / hang; at most the reachable nodes are emitted.
        assert!(tree.len() <= 2);
    }

    #[test]
    fn newly_excluded_ids_diffs_against_old() {
        let old = vec!["a".to_string(), "b".to_string()];
        let new = vec!["b".to_string(), "c".to_string(), "d".to_string(), "c".to_string()];
        // c and d are new; b was already excluded; duplicate c de-duped.
        assert_eq!(newly_excluded_ids(&old, &new), vec!["c".to_string(), "d".to_string()]);

        // Un-excluding (removing) yields nothing newly-excluded.
        assert!(newly_excluded_ids(&old, &[]).is_empty());
        // No change → empty.
        assert!(newly_excluded_ids(&old, &old).is_empty());
    }

    #[test]
    fn subtree_file_ids_collects_descendant_files_only() {
        // A ─ a1, A ─ B ─ b1, A ─ B ─ b2(folder) ─ c1; sibling root Z ─ z1.
        let rows = vec![
            ("A".to_string(), None, true),
            ("B".to_string(), Some("A".to_string()), true),
            ("a1".to_string(), Some("A".to_string()), false),
            ("b1".to_string(), Some("B".to_string()), false),
            ("b2".to_string(), Some("B".to_string()), true),
            ("c1".to_string(), Some("b2".to_string()), false),
            ("Z".to_string(), None, true),
            ("z1".to_string(), Some("Z".to_string()), false),
        ];
        let mut got = subtree_file_ids(&rows, &["A".to_string()]);
        got.sort();
        // Files under A: a1, b1, c1. NOT z1 (under Z), and folders excluded.
        assert_eq!(got, vec!["a1".to_string(), "b1".to_string(), "c1".to_string()]);
    }

    #[test]
    fn subtree_file_ids_dedupes_overlapping_excluded_roots() {
        // Excluding both A and its child B must not list B's files twice.
        let rows = vec![
            ("A".to_string(), None, true),
            ("B".to_string(), Some("A".to_string()), true),
            ("b1".to_string(), Some("B".to_string()), false),
        ];
        let got = subtree_file_ids(&rows, &["A".to_string(), "B".to_string()]);
        assert_eq!(got, vec!["b1".to_string()]);
    }
}
