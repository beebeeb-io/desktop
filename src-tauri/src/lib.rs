use std::fmt;
use std::path::PathBuf;
use std::sync::Mutex;

use tauri::{
    Emitter, Manager, State,
    menu::{AboutMetadata, CheckMenuItemBuilder, Menu, MenuItemBuilder, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_autostart::ManagerExt as AutostartManagerExt;
use tauri_plugin_dialog::DialogExt;
use tracing_subscriber::EnvFilter;

mod api_client;
mod config;
mod conflict;
mod engine_bridge;
mod ipc_socket;
mod keychain;
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
// Windows Cloud Files API — Phase 2 Task 6. Gated to Windows only;
// the module's own files start with `#![cfg(target_os = "windows")]`
// so this `mod` declaration plus the conditional are belt-and-braces.
#[cfg(target_os = "windows")]
mod windows_cf;
use config::DesktopConfig;
use keychain::{AuthVault, MacOsKeychainStore, SecretBytes, SessionToken};
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
            .field("email", &self.email)
            .finish()
    }
}

/// Tauri-managed shared state. Held behind a `Mutex` because the web
/// thread (IPC handlers) and the future sync-engine task both need
/// access. Cheap to lock — sessions change rarely.
pub struct AppState {
    pub session: Mutex<Option<Session>>,
    pub auth_present: Mutex<bool>,
    pub auth_email: Mutex<Option<String>>,
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
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            session: Mutex::new(None),
            auth_present: Mutex::new(false),
            auth_email: Mutex::new(None),
            engine: tokio::sync::Mutex::new(None),
            // Default to "stopped" — once the runner spawns and emits
            // its first event, the listener overwrites this.
            engine_state: Mutex::new("stopped".to_string()),
        }
    }
}

fn keychain_error(context: &str, error: impl fmt::Display) -> String {
    format!("{context}: {error}")
}

fn persist_session_to_keychain(token: &str, master_key: [u8; 32]) -> Result<(), String> {
    let vault = AuthVault::new(MacOsKeychainStore::new());
    let token = SessionToken::new(token.to_string()).map_err(|e| keychain_error("session token", e))?;
    vault
        .install_session(token)
        .map_err(|e| keychain_error("store session in Keychain", e))?;
    vault
        .store_wrapped_master_key(SecretBytes::new_master_key(master_key))
        .map_err(|e| keychain_error("store vault key in Keychain", e))
}

fn persist_session_token_to_keychain(token: &str) -> Result<(), String> {
    let vault = AuthVault::new(MacOsKeychainStore::new());
    let token = SessionToken::new(token.to_string()).map_err(|e| keychain_error("session token", e))?;
    vault
        .install_session(token)
        .map_err(|e| keychain_error("store session in Keychain", e))
}

fn persist_vault_key_to_keychain(master_key: [u8; 32]) -> Result<(), String> {
    AuthVault::new(MacOsKeychainStore::new())
        .store_wrapped_master_key(SecretBytes::new_master_key(master_key))
        .map_err(|e| keychain_error("store vault key in Keychain", e))
}

fn load_session_token_from_keychain() -> Result<Option<String>, String> {
    AuthVault::new(MacOsKeychainStore::new())
        .session_token()
        .map_err(|e| keychain_error("load session from Keychain", e))
        .map(|token| token.map(|t| t.expose_for_request().to_string()))
}

fn load_session_from_keychain(email: Option<String>) -> Result<Option<Session>, String> {
    let mut vault = AuthVault::new(MacOsKeychainStore::new());
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
    Ok(Some(Session {
        token: token.expose_for_request().to_string(),
        master_key,
        email,
    }))
}

fn keychain_session_present() -> bool {
    AuthVault::new(MacOsKeychainStore::new())
        .session_token()
        .map(|token| token.is_some())
        .unwrap_or(false)
}

fn clear_keychain_session() -> Result<(), String> {
    let mut vault = AuthVault::new(MacOsKeychainStore::new());
    vault
        .clear_session()
        .map_err(|e| keychain_error("clear Keychain session", e))
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
    if let Some(root) = DesktopConfig::load().ok().and_then(|c| c.sync_root) {
        let mut engine_slot = state.engine.lock().await;
        if let Some(prev) = engine_slot.take() {
            prev.abort().await;
        }
        *engine_slot = Some(EngineRunner::spawn(app, root, token, master_key));
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
    if let Some(existing) = app.get_webview_window("onboarding") {
        let _ = existing.show();
        let _ = existing.set_focus();
        return Ok(());
    }

    tauri::WebviewWindowBuilder::new(
        app,
        "onboarding",
        tauri::WebviewUrl::App("index.html?window=onboarding".into()),
    )
    .title("Welcome to Beebeeb")
    .inner_size(860.0, 640.0)
    .min_inner_size(780.0, 560.0)
    .resizable(true)
    .center()
    .build()
    .map_err(|e| e.to_string())?;

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
#[tauri::command]
fn show_settings(app: tauri::AppHandle) -> Result<(), String> {
    show_settings_window(&app);
    Ok(())
}

/// Authenticate with the current OPAQUE login endpoints.
///
/// This intentionally mirrors web and iOS: sign-in proves account access with
/// email/password only. If this Mac does not already have the vault key in
/// Keychain, onboarding moves to the separate recovery-phrase unlock step.
///
/// Returns `Err` on HTTP errors, network failures, OPAQUE failures, or 2FA
/// requirements.
///
/// Spec: docs/superpowers/plans/2026-05-07-desktop-sync-client.md (onboarding §1)
#[tauri::command]
async fn desktop_login(
    state: State<'_, AppState>,
    email: String,
    password: String,
) -> Result<(), String> {
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
    let server_message_bytes =
        decode_base64(server_message).map_err(|e| format!("invalid OPAQUE server message: {e}"))?;
    let login_finish = beebeeb_core::opaque_protocol::client_login_finish(
        &login_start.state,
        password.as_bytes(),
        &server_message_bytes,
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
        return Err("Two-factor authentication is required. Please sign in at app.beebeeb.io to complete 2FA before using desktop sync.".to_string());
    }
    let session_token = finish_body
        .get("session_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "No session_token in login finish response".to_string())?
        .to_string();

    if desktop_session_has_totp(&client, &base_url, &session_token).await? {
        let _ = revoke_desktop_session(&client, &base_url, &session_token).await;
        return Err("Two-factor authentication is required. Please sign in at app.beebeeb.io to complete 2FA before using desktop sync.".to_string());
    }

    if let Err(e) = persist_session_token_to_keychain(&session_token) {
        let _ = revoke_desktop_session(&client, &base_url, &session_token).await;
        return Err(e);
    }

    set_auth_present(&state, true);
    set_auth_email(&state, Some(email.clone()));
    tracing::info!(email = %email, "desktop account session installed");

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

async fn desktop_session_has_totp(
    client: &reqwest::Client,
    base_url: &str,
    session_token: &str,
) -> Result<bool, String> {
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
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("parse session verification response: {e}"))?;
    Ok(body.get("totp_enabled").and_then(|v| v.as_bool()).unwrap_or(false))
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
    persist_session_to_keychain(&token, arr)?;
    let token_clone = token.clone();
    let key_clone = arr;

    {
        let mut guard = state.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
        *guard = Some(Session {
            token,
            master_key: arr,
            email: email.clone(),
        });
    }
    set_auth_present(&state, true);
    set_auth_email(&state, email);
    tracing::info!("session installed via IPC");

    // If we already know the sync_root, kick off the engine. Otherwise
    // it'll start when the first-launch picker resolves.
    start_engine_if_possible(app, &state, token_clone, key_clone).await;
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

    match state.session.lock() {
        Ok(mut guard) => {
            guard.take();
            tracing::info!("session cleared via IPC");
        }
        Err(_) => {
            tracing::warn!("session mutex poisoned during clear_session");
        }
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
    let session = load_session_from_keychain(email)?.ok_or_else(|| "Sign in before unlocking the vault.".to_string())?;
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

#[tauri::command]
fn finder_location_state() -> Result<FinderInstallState, String> {
    let path = DesktopConfig::load()
        .ok()
        .and_then(|c| c.sync_root)
        .map(|p| p.to_string_lossy().into_owned());
    let installed = file_provider_installed()?;
    Ok(FinderInstallState { installed, path })
}

/// Persist the chosen sync root and start the daemon when possible, but do not
/// claim Finder integration succeeded while the `.appex` is absent.
#[tauri::command]
async fn install_finder_location(
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
    cfg.sync_root = Some(root.clone());
    cfg.save()?;

    let session = state
        .session
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|s| (s.token.clone(), s.master_key)));
    if let Some((token, key)) = session {
        let mut engine_slot = state.engine.lock().await;
        if let Some(prev) = engine_slot.take() {
            prev.abort().await;
        }
        *engine_slot = Some(EngineRunner::spawn(app, root.clone(), token, key));
    }

    install_file_provider_domain()?;
    Ok(FinderInstallState {
        installed: true,
        path: Some(root.to_string_lossy().into_owned()),
    })
}

#[tauri::command]
fn open_finder_location(path: Option<String>) -> Result<(), String> {
    let root = path
        .map(PathBuf::from)
        .or_else(|| DesktopConfig::load().ok().and_then(|c| c.sync_root))
        .unwrap_or_else(config::default_sync_root_suggestion);
    config::ensure_directory(&root)?;

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&root)
            .spawn()
            .map_err(|e| format!("open Finder: {e}"))?;
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&root)
            .spawn()
            .map_err(|e| format!("open Explorer: {e}"))?;
        return Ok(());
    }

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        std::process::Command::new("xdg-open")
            .arg(&root)
            .spawn()
            .map_err(|e| format!("open file manager: {e}"))?;
        Ok(())
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
        let guard = state
            .session
            .lock()
            .map_err(|_| "session mutex poisoned".to_string())?;
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

    Ok(DesktopStorageSummary {
        used_bytes: usage.used_bytes.max(0),
        quota_bytes: usage.quota_bytes.max(0),
        cache_bytes,
        pinned_bytes,
    })
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
        let mut engine_slot = state.engine.lock().await;
        if let Some(prev) = engine_slot.take() {
            prev.abort().await;
        }
        *engine_slot = Some(EngineRunner::spawn(app, path.clone(), token, key));
    }

    Ok(Some(path.to_string_lossy().into_owned()))
}

/// Suggested default path for the sync root, used by the WebView to
/// show "We'll create ~/Beebeeb if you accept" before opening the
/// native picker.
#[tauri::command]
fn default_sync_root() -> String {
    config::default_sync_root_suggestion().to_string_lossy().into_owned()
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

/// Persist a new list of excluded top-level folder IDs to
/// `desktop.toml`. An empty incoming list is normalised to `None` so
/// the on-disk file stays free of `excluded_folder_ids = []` clutter
/// when the user has nothing excluded.
///
/// Round-trips through `DesktopConfig::load` → mutate → `save` so a
/// concurrent settings save (Bandwidth/Notifications page) doesn't
/// race away the new exclusion list — same merge pattern as
/// [`set_desktop_config`].
///
/// Note: changing the excluded list does NOT immediately tear down
/// in-flight syncs for newly excluded folders. The engine reads this
/// list on its next tick and de-prioritises matching files; cleaning
/// up already-downloaded local copies is a follow-up task. Surfacing
/// this gap in the UI (a "Files already downloaded will stay until
/// you remove them" hint) is the SelectiveSync page's job.
#[tauri::command]
fn set_selective_sync(excluded: Vec<String>) -> Result<(), String> {
    let mut cfg = DesktopConfig::load()?;
    cfg.excluded_folder_ids = if excluded.is_empty() { None } else { Some(excluded) };
    cfg.save()
}

/// Plain-old-data view of one top-level vault item the SelectiveSync
/// page renders as a row. We deliberately keep this small: only the
/// fields the checkbox UI needs.
///
/// `name` is decrypted from the server's `name_encrypted` blob via
/// [`try_decrypt_name`] when possible, falling back to a stable
/// id-derived label (e.g. `"Folder 7f3a1c…"`) when decryption is not
/// possible (missing field, bad envelope, wrong key…). This mirrors
/// the canonical decrypt path used by `repos/cli/src/commands/pull.rs`.
#[derive(Debug, Clone, serde::Serialize)]
struct VaultItem {
    id: String,
    name: String,
    is_folder: bool,
}

/// Try to decrypt a single file/folder's `name_encrypted` blob using
/// the master key. Returns `None` on any failure — caller falls back
/// to a stable id-derived label. We deliberately swallow errors here
/// because the SelectiveSync page treats these as best-effort: a
/// folder created by an older client with a missing/garbled blob, or
/// an entry encrypted under a different key, should still appear in
/// the list (with a fallback name) rather than abort the whole call.
///
/// Mirrors the decrypt flow in `repos/cli/src/commands/pull.rs`:
///   - parse `id` as a UUID for the HKDF info
///   - parse `name_encrypted` (a JSON-encoded `EncryptedBlob` string)
///   - derive per-file key, decrypt to UTF-8
fn try_decrypt_name(file_meta: &serde_json::Value, id: &str, master_key: &[u8; 32]) -> Option<String> {
    let file_uuid = uuid::Uuid::parse_str(id).ok()?;

    let name_enc_str = file_meta.get("name_encrypted").and_then(|v| v.as_str())?;
    let blob: beebeeb_types::EncryptedBlob = serde_json::from_str(name_enc_str).ok()?;

    // MasterKey::from_bytes consumes the array (it zeroizes on drop),
    // so copy from the borrow first.
    let mk_bytes: [u8; 32] = *master_key;
    let mk = beebeeb_core::kdf::MasterKey::from_bytes(mk_bytes);
    let file_key = beebeeb_core::kdf::derive_file_key(&mk, file_uuid.as_bytes());

    beebeeb_core::encrypt::decrypt_metadata(&file_key, &blob).ok()
}

/// Fetch the top-level entries in the user's vault (parent_id =
/// None) and return only the folders. Used by the SelectiveSync page
/// to populate its checkbox list.
///
/// Returns an empty list — not `Err` — when:
///   - the user is not signed in (no in-memory session)
///   - the API call fails for any reason
///
/// The page treats an empty list as "nothing to choose from yet" and
/// renders a friendly empty state. Surfacing every transient network
/// blip as a red error banner would be noisy for a settings page that
/// re-opens every time the user clicks the tab.
#[tauri::command]
async fn list_vault_folders(state: State<'_, AppState>) -> Result<Vec<VaultItem>, String> {
    // Lift the session pieces we need without holding the mutex over
    // the async API call.
    let creds = {
        let guard = state.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
        guard.as_ref().map(|s| (s.token.clone(), s.master_key))
    };
    let Some((token, master_key)) = creds else {
        // Not signed in — surface as empty rather than an error.
        return Ok(Vec::new());
    };

    let api = api_client::ApiClient::new(runner::api_base_url(), token, master_key);
    let raw = match api.list_files(None).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "list_vault_folders: list_files failed");
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
            .unwrap_or_else(|| {
                let short = id.get(..8).unwrap_or(id);
                format!("Folder {short}…")
            });

        out.push(VaultItem {
            id: id.to_string(),
            name: display,
            is_folder: true,
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
    bridge
        .set_recursive_pin(&item_id, pinned)
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
            export_diagnostics,
            list_version_conflict_center,
            list_file_versions,
            restore_file_version,
            get_sync_root,
            pick_sync_root,
            default_sync_root,
            finder_location_state,
            install_finder_location,
            open_finder_location,
            // Task 9 — settings page IPC
            get_desktop_config,
            set_desktop_config,
            account_email,
            // Task 0090 — selective sync
            get_selective_sync,
            set_selective_sync,
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
            open_onboarding_window,
            show_settings,
        ])
        .setup(|app| {
            setup_native_menu(app)?;
            setup_tray(app)?;
            attach_tray_status_listener(&app.handle().clone());
            {
                let app_state = app.state::<AppState>();
                if let Ok(mut guard) = app_state.auth_present.lock() {
                    *guard = keychain_session_present();
                }
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
                // Already configured — show the settings window after startup
                // settles (same as left-click on the tray). The short delay
                // mirrors onboarding and avoids racing Tauri's window setup.
                let h = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                    show_settings_window(&h);
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
    let show_item = MenuItemBuilder::new("Open Settings")
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

fn setup_tray(app: &mut tauri::App) -> tauri::Result<()> {
    // Read current autostart state so the check mark is correct on launch.
    let autostart_enabled = app.autolaunch().is_enabled().unwrap_or(false);
    let tray_menu = build_tray_menu(app, autostart_enabled)?;

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
            "open_settings" => show_settings_window(app),
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
            // Left-click toggles window visibility
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(win) = app.get_webview_window("settings") {
                    if win.is_visible().unwrap_or(false) {
                        let _ = win.hide();
                    } else {
                        show_settings_window(app);
                    }
                }
            }
        })
        .build(app)?;

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Show and focus the settings window — the only persistent window
/// we ship as of Task 8B. Conflict windows are minted on demand by
/// `open_conflict_window` and live independently; nothing in here
/// touches them.
fn show_settings_window(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("settings") {
        let _ = win.show();
        let _ = win.set_focus();
        return;
    }

    match tauri::WebviewWindowBuilder::new(app, "settings", tauri::WebviewUrl::App("index.html".into()))
        .title("Beebeeb")
        .inner_size(680.0, 540.0)
        .resizable(false)
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

#[cfg(test)]
mod tests {
    use super::normalize_recovery_phrase_input;

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
}
