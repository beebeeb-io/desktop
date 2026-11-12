//! Desktop v2 B14: account settings commands, thin wrappers over core's account flows
//! (`beebeeb_core::account_flows`). Logic lives in core; this module only performs each
//! HTTP step the flow asks for, with the in-memory session, and keeps the platform
//! side effects (the credential store, the engine).
//!
//! Secrets go one way (crypto review 2026-11-08, rules 1-3): passwords and codes come in
//! from the web view and never go back. No command returns a session, a confirmation
//! token or a key. The TOTP secret, its `otpauth` URI and the backup codes are returned
//! on purpose, once, for the set-up screens (DW-A69, DW-A70): the allow-list in
//! `tests.rs` names them with that reason. Errors are stable codes, never a server body.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use beebeeb_core::account_flows::{
    AccountBinding, AccountFlow, AccountFlowError, AccountOutcome, ApiResponse, Method, Step, TotpEnrollment,
};
use beebeeb_core::kdf::MasterKey;
use beebeeb_core::onboarding::{BreachCheck, BreachResponse};
use beebeeb_core::signup_flow::SignupPolicy;
use serde::Serialize;
use tauri::{Manager, State};
use zeroize::{Zeroize, Zeroizing};

use super::*;

#[cfg(test)]
mod tests;

/// Set-up state held in Rust between two commands; never crosses to the web view.
struct Pending {
    totp: Option<TotpEnrollment>,
    /// Delete step 1 checked this password; step 2 uses it for the step-up of the delete.
    delete_password: Option<Zeroizing<String>>,
    /// The mailed delete link, once the person pasted it here (step 3): Undo on step 4 uses it.
    delete_link: Option<Zeroizing<String>>,
}

static PENDING: Mutex<Pending> = Mutex::new(Pending {
    totp: None,
    delete_password: None,
    delete_link: None,
});

fn pending() -> Result<std::sync::MutexGuard<'static, Pending>, String> {
    PENDING.lock().map_err(|_| "internal".to_string())
}

/// DW-A69: the key in mono and the QR payload. Shown once (allow-listed in tests).
#[derive(Serialize)]
pub struct TotpSetupDto {
    pub secret: String,
    pub otpauth_uri: String,
    /// The QR code of `otpauth_uri`, one string of `0`/`1` per module row (dark = `1`).
    pub qr_rows: Vec<String>,
}

fn qr_rows(uri: &str) -> Result<Vec<String>, String> {
    let code = qrcode::QrCode::with_error_correction_level(uri.as_bytes(), qrcode::EcLevel::M)
        .map_err(|_| "internal".to_string())?;
    let w = code.width();
    let colors = code.to_colors();
    Ok((0..w)
        .map(|y| {
            (0..w)
                .map(|x| {
                    if colors[y * w + x] == qrcode::Color::Dark {
                        '1'
                    } else {
                        '0'
                    }
                })
                .collect()
        })
        .collect())
}

#[derive(Serialize)]
pub struct DisplayNameDto {
    pub display_name: Option<String>,
}

#[derive(Serialize)]
pub struct ActivityLoggingDto {
    pub on: bool,
}

/// A plain authenticated GET for the two reads the account flows have no flow for.
async fn get_json(state: &State<'_, AppState>, path: &str) -> Result<serde_json::Value, String> {
    let (_acct, token, _binding) = session_parts(state)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(crate::link_health::API_REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(|_| "network".to_string())?;
    let resp = client
        .get(format!("{}{}", runner::api_base_url().trim_end_matches('/'), path))
        .bearer_auth(token.as_str())
        .header("X-Beebeeb-Client", "desktop")
        .send()
        .await
        .map_err(|_| "network".to_string())?;
    match resp.status().as_u16() {
        200..=299 => resp.json().await.map_err(|_| "protocol".to_string()),
        401 => Err("unauthorized".to_string()),
        s => Err(format!("server_error_{s}")),
    }
}

/// DW-A51 / A64: the display name (`GET /api/v1/me/profile`).
#[tauri::command]
pub(super) async fn account_display_name(state: State<'_, AppState>) -> Result<DisplayNameDto, String> {
    session_command!(async {
        let v = get_json(&state, "/api/v1/me/profile").await?;
        Ok(DisplayNameDto {
            display_name: v.get("display_name").and_then(|n| n.as_str()).map(str::to_string),
        })
    })
}

/// DW-A77: whether sign-in activity is recorded (`GET /api/v1/me/tracking`).
#[tauri::command]
pub(super) async fn account_activity_logging(state: State<'_, AppState>) -> Result<ActivityLoggingDto, String> {
    session_command!(async {
        let v = get_json(&state, "/api/v1/me/tracking").await?;
        Ok(ActivityLoggingDto {
            on: v.get("opted_in").and_then(|b| b.as_bool()).unwrap_or(false),
        })
    })
}

/// DW-A70 Save as text: the person picks the file; the codes come from the web view
/// (which already shows them) and are written once. `false` when the dialog was cancelled.
#[tauri::command]
pub(super) async fn account_save_backup_codes(app: tauri::AppHandle, text: String) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;
    let text = Zeroizing::new(text);
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("Text", &["txt"])
            .set_file_name("Beebeeb backup codes.txt")
            .blocking_save_file()
    })
    .await
    .map_err(|_| "internal".to_string())?;
    let Some(path) = picked.and_then(|p| p.into_path().ok()) else {
        return Ok(false);
    };
    std::fs::write(&path, text.as_bytes()).map_err(|_| "write_failed".to_string())?;
    Ok(true)
}

/// DW-A70: the 8 single-use codes, shown once (allow-listed in tests).
#[derive(Serialize)]
pub struct BackupCodesDto {
    pub backup_codes: Vec<String>,
}

#[derive(Serialize)]
pub struct EmailChangedDto {
    pub email: String,
}

/// DW-A81: the server mailed a single-use link and scheduled nothing yet.
#[derive(Serialize)]
pub struct DeleteRequestedDto {
    pub email_link_sent: bool,
    pub link_expires_at: String,
}

/// DW-A82: the link was used; the deletion is scheduled.
#[derive(Serialize)]
pub struct DeletionScheduledDto {
    pub shred_after: String,
    pub cancel_until: Option<String>,
}

/// DW-A83: the deletion is cancelled; `not_restored` names what the cancel does not bring back.
#[derive(Serialize)]
pub struct DeletionCancelledDto {
    pub not_restored: Vec<String>,
}

fn code(e: AccountFlowError) -> String {
    e.code().to_string()
}

/// The in-memory session: its account, a copy of the token, and the public binding
/// values derived from the master key (the key itself stays here and is wiped).
fn session_parts(
    state: &State<'_, AppState>,
) -> Result<(Arc<account::AccountRuntime>, Zeroizing<String>, AccountBinding), String> {
    let acct = state.active_account()?;
    let guard = acct.session.lock().map_err(|_| "internal".to_string())?;
    let session = guard.as_ref().ok_or_else(|| "vault_locked".to_string())?;
    let token = Zeroizing::new(session.token.clone());
    let binding = AccountBinding::from_master_key(&MasterKey::from_bytes(session.master_key));
    drop(guard);
    Ok((acct, token, binding))
}

/// What a driven flow ended with: an ordinary outcome, or the password change's detached session commit.
enum Driven {
    Outcome(AccountOutcome),
    Rotated(PasswordChangedDto),
}

/// A password change's session turn and what the detached commit needs: the app for the engine restart and the
/// credential store, the account, and the generation the turn was captured at.
struct SessionCommit {
    app: tauri::AppHandle,
    acct: Arc<account::AccountRuntime>,
    turn: account::SessionGeneration,
}

/// Run `work` as a task of its own and await its handle. A Lock or a sign-out drops a Windows session command at an
/// await (`SessionCommands::within`); the response body read and the token store must not be in that future, so they
/// run here instead and the command only awaits their handle. This task deliberately takes no `SessionCommands` lease
/// (`SessionCommands::bind`): a lease cancels the work at an await, which is exactly the loss this prevents once the
/// server has rotated the token. The turn claim before the store is the guard instead.
async fn detached<T>(work: impl std::future::Future<Output = Result<T, String>> + Send + 'static) -> Result<T, String>
where
    T: Send + 'static,
{
    tauri::async_runtime::spawn(work)
        .await
        .map_err(|_| "internal".to_string())?
}

/// The one body read. A commit reaches it only inside [`detached`], so the read outlives a dropped command; no body is
/// read inline by the command's own future.
async fn read_body(resp: reqwest::Response) -> String {
    resp.text().await.unwrap_or_default()
}

/// Perform each step of `flow` against the API with `token` (none for the public delete-link routes), to the end.
///
/// A `commit` (a password change) drives every step inside [`detached`]: the server has ended the old token by the time
/// it answers, so the body read and the store of the token it issued in its place must survive a Lock or a sign-out
/// that drops the command.
async fn drive(flow: AccountFlow, token: Option<&str>, commit: Option<SessionCommit>) -> Result<Driven, String> {
    let Some(commit) = commit else {
        return drive_steps(flow, token, None).await;
    };
    // A zeroizing copy travels into the task; the caller's token never leaves this frame.
    let token = token.map(|t| Zeroizing::new(t.to_string()));
    detached(async move { drive_steps(flow, token.as_ref().map(|t| t.as_str()), Some(commit)).await }).await
}

async fn drive_steps(
    mut flow: AccountFlow,
    token: Option<&str>,
    mut commit: Option<SessionCommit>,
) -> Result<Driven, String> {
    let base = runner::api_base_url();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(crate::link_health::API_REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(|_| "network".to_string())?;
    let mut step = flow.start().map_err(code)?;
    loop {
        match step {
            Step::Done(outcome) => return Ok(Driven::Outcome(outcome)),
            Step::Send(req) => {
                let method = match req.method {
                    Method::Get => reqwest::Method::GET,
                    Method::Post => reqwest::Method::POST,
                    Method::Put => reqwest::Method::PUT,
                    Method::Delete => reqwest::Method::DELETE,
                    Method::Patch => reqwest::Method::PATCH,
                };
                let url = format!("{}{}", base.trim_end_matches('/'), req.path);
                let mut rb = client.request(method, url).header("X-Beebeeb-Client", "desktop");
                if let Some(token) = token {
                    rb = rb.bearer_auth(token);
                }
                if let Some(t) = req.confirm_token() {
                    rb = rb.header("X-Confirm-Token", t);
                }
                if req.body.is_some() {
                    rb = rb
                        .header(reqwest::header::CONTENT_TYPE, "application/json")
                        .body(req.body_json());
                }
                drop(req);
                let resp = rb.send().await.map_err(|_| "network".to_string())?;
                let status = resp.status().as_u16();
                let mut text = read_body(resp).await;
                let parsed = ApiResponse::from_text(status, &text);
                text.zeroize();
                let next = flow.resume(parsed).map_err(code)?;
                if let Step::Done(AccountOutcome::PasswordChanged { session_token }) = &next
                    && let Some(commit) = commit.take()
                {
                    // The server has rotated the token; this task (not the command's future) stores it.
                    let SessionCommit { app, acct, turn } = commit;
                    let state = app.state::<AppState>();
                    let mut turn = turn;
                    let dto =
                        install_rotated_session(app.clone(), &state, &acct, &mut turn, session_token.as_str()).await;
                    return Ok(Driven::Rotated(dto));
                }
                step = next;
            }
        }
    }
}

async fn run_simple(
    state: &State<'_, AppState>,
    flow: Result<AccountFlow, AccountFlowError>,
) -> Result<AccountOutcome, String> {
    let flow = flow.map_err(code)?;
    let (_acct, token, _binding) = session_parts(state)?;
    match drive(flow, Some(&token), None).await? {
        Driven::Outcome(outcome) => Ok(outcome),
        Driven::Rotated(_) => Err("protocol".to_string()),
    }
}

/// The server's password policy (`GET /api/v1/onboarding`), the one sign-up reads. No fallback policy: an answer
/// that cannot be read stops the change.
async fn password_policy() -> Result<SignupPolicy, String> {
    let client = crate::auth_v2::transport::client().map_err(|_| "network".to_string())?;
    let resp = crate::auth_v2::transport::send(&client, None, SignupPolicy::request())
        .await
        .map_err(|_| "network".to_string())?;
    SignupPolicy::from_response(&resp).map_err(|_| "policy_unavailable".to_string())
}

/// Sign-up's k-anonymity breach check for `password`, made and answered, when the policy declares one (only the
/// 5-character prefix of its hash leaves this PC). An outage is recorded as one: core's gate then applies the
/// policy's own fail-open rule.
async fn breach_check(policy: &SignupPolicy, password: &str) -> Option<BreachCheck> {
    let endpoint = policy.breach_endpoint.as_deref()?;
    let mut check = BreachCheck::new(password);
    let prefix = check.prefix().to_owned();
    let (status, body) = crate::auth_v2::signup_v2::breach_answer_at(&endpoint.replace("{prefix}", &prefix)).await;
    let response = match status {
        Some(s) if (200..300).contains(&s) => BreachResponse::Body(&body),
        _ => BreachResponse::Unavailable,
    };
    if check.record(&prefix, response).is_err() {
        let _ = check.record(&prefix, BreachResponse::Unavailable);
    }
    Some(check)
}

/// DW-A67: change the password (OPAQUE re-registration in core, a step-up per leg).
/// The new password passes sign-up's gate first (core v0.2.8 item 6): the server's
/// minimum, and the breach check when the server declares one.
/// DW-A67: what followed the change. The server has already committed when this answers, so
/// `signed_in` false means the new token could not be stored (the old one is dead) and
/// `restart` says whether the engine came back. No key, token, code or path.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PasswordChangedDto {
    pub status: &'static str,
    pub signed_in: bool,
    pub restart: &'static str,
}

/// The server ends every other session and mints a new one for this device. Once its answer arrives, the body read
/// and the store of that token run in a task of their own that the command awaits: a Lock or a sign-out may drop the
/// command's future at an await after the server already rotated the token, and that task must not go with it. The
/// task takes no callback lease (`SessionCommands::bind`), only the turn the change captured; it claims that turn
/// before it stores, so a turn a Lock or another sign-in spent writes nothing and the person signs in again. The engine
/// then restarts with the new token. No master-key wrap (the key stays in the credential store). Once the server has
/// committed, a failure to store or restart is reported in the DTO, never as a failed change: the password did change.
#[tauri::command]
pub(super) async fn account_change_password(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    current: String,
    new_password: String,
) -> Result<PasswordChangedDto, String> {
    let current = Zeroizing::new(current);
    let new_password = Zeroizing::new(new_password);
    session_command!(async {
        let (acct, token, binding) = session_parts(&state)?;
        let turn = acct.session_generation();
        let policy = password_policy().await?;
        let breach = breach_check(&policy, &new_password).await;
        let flow =
            AccountFlow::change_password(&current, &new_password, binding, &policy, breach.as_ref()).map_err(code)?;
        let commit = SessionCommit {
            app,
            acct: acct.clone(),
            turn,
        };
        match drive(flow, Some(&token), Some(commit)).await? {
            Driven::Rotated(dto) => Ok(dto),
            Driven::Outcome(_) => Err("protocol".to_string()),
        }
    })
}

/// Why a rotated session was not stored.
#[derive(Debug, PartialEq, Eq)]
enum RotatedStoreError {
    /// A Lock, a sign-out or another sign-in moved the account's generation before the body arrived: nothing may be
    /// written, and the person signs in again.
    TurnSpent,
    /// The credential store, or the swap into the session in memory, failed.
    Store(String),
}

/// The credential-store side of a rotated session, behind one trait so the commit can be exercised without an OS
/// vault (Linux has none; a test must never write the developer's real one).
trait RotatedSessionStore {
    /// Store `token` for `account_id`, with the session's own email.
    fn store_token(
        &self,
        write: &SessionWrite,
        account_id: &str,
        token: &str,
        email: Option<&str>,
    ) -> Result<(), String>;
    /// Carry the previous session's owner tag over to the token just stored (task 2447).
    fn carry_owner(&self, write: &SessionWrite, account_id: &str, owner: &str);
}

struct PlatformRotatedSessionStore;

impl RotatedSessionStore for PlatformRotatedSessionStore {
    fn store_token(
        &self,
        write: &SessionWrite,
        account_id: &str,
        token: &str,
        email: Option<&str>,
    ) -> Result<(), String> {
        persist_session_token_to_keychain(write, account_id, token, email)
    }

    fn carry_owner(&self, write: &SessionWrite, account_id: &str, owner: &str) {
        crate::bind_session_owner_in_keychain(write, account_id, owner);
    }
}

/// Claim the turn the password change captured and store the token the server issued in its place, then swap it into
/// the session in memory. A spent turn writes nothing and never resurrects an ended session. Only inside the claimed
/// turn (`write`).
fn store_rotated_session(
    acct: &Arc<account::AccountRuntime>,
    turn: &mut account::SessionGeneration,
    new_token: &str,
) -> Result<(), RotatedStoreError> {
    store_rotated_session_with(acct, turn, new_token, &PlatformRotatedSessionStore)
}

fn store_rotated_session_with<S: RotatedSessionStore>(
    acct: &Arc<account::AccountRuntime>,
    turn: &mut account::SessionGeneration,
    new_token: &str,
    store: &S,
) -> Result<(), RotatedStoreError> {
    let write = claim_session_write_for_commit(acct, turn).map_err(|_| RotatedStoreError::TurnSpent)?;
    let email = acct
        .session
        .lock()
        .ok()
        .and_then(|g| g.as_ref().and_then(|s| s.email.clone()));
    // Task 2447: the same session with the token the server issued in its place, so its owner tag is carried over
    // (read only when the stored token is the one being replaced; the store removes the tag on the write).
    let owner = acct.session.lock().ok().and_then(|g| {
        g.as_ref()
            .and_then(|s| crate::session_owner_for_rotation(acct.id.as_str(), &s.token))
    });
    store
        .store_token(&write, acct.id.as_str(), new_token, email.as_deref())
        .map_err(RotatedStoreError::Store)?;
    if let Some(owner) = owner {
        store.carry_owner(&write, acct.id.as_str(), &owner);
    }
    let mut guard = acct
        .session
        .lock()
        .map_err(|_| RotatedStoreError::Store("internal".to_string()))?;
    if let Some(session) = guard.as_mut() {
        let mut old = std::mem::replace(&mut session.token, new_token.to_string());
        old.zeroize();
    }
    drop(guard);
    drop(write);
    Ok(())
}

/// Store and swap the new token in the session turn the change captured, then start the engine again with it, on
/// every platform; the engine holds the ended token. macOS and Linux stop the engine first and start it as after a
/// naming (`start_engine_unless_running`).
///
/// The store and swap happen before any await on Windows: the server has already ended the old token, so nothing may
/// come between its answer and [`store_rotated_session`]. (The body read and this store also run in [`detached`], so a
/// Lock or a sign-out that drops the command cannot drop them.) The engine then comes back under the
/// session-transition lock through `start_engine_if_possible`, which starts with the session gate, its generation and
/// the binding's Windows rules, like every other Windows start. The old engine is stopped under that lock first (task
/// 2439 item 1): the start begins with `if let Ok(cfg) = DesktopConfig::load()`, so a config read failure would
/// otherwise return before it takes the slot and leave the old engine on the ended token. A running R10 ownership
/// proof holds the ended token too: it is stopped under the lock first, as a Lock and a sign-out stop it.
///
/// A store or restart failure after the server committed is not an error: the DTO says what followed, so the pane
/// never claims the password did not change.
async fn install_rotated_session(
    app: tauri::AppHandle,
    state: &State<'_, AppState>,
    acct: &Arc<account::AccountRuntime>,
    turn: &mut account::SessionGeneration,
    new_token: &str,
) -> PasswordChangedDto {
    #[cfg(not(target_os = "windows"))]
    {
        let mut slot = acct.engine.lock().await;
        if let Some(engine) = slot.take() {
            let _ = stop_engine_in_slot(acct, engine).await;
        }
    }
    let stored = store_rotated_session(acct, turn, new_token);
    if stored.is_err() {
        // The old token is dead and the new one is not on disk: the person signs in again.
        return PasswordChangedDto {
            status: "changed",
            signed_in: false,
            restart: "failed",
        };
    }
    #[cfg(not(target_os = "windows"))]
    let restart = start_engine_unless_running(app, state.inner()).await;
    #[cfg(target_os = "windows")]
    let restart = {
        let transition = SESSION_TRANSITION.lock().await;
        stop_own_data_proofs();
        {
            let mut slot = acct.engine.lock().await;
            if let Some(engine) = slot.take() {
                let _ = stop_engine_in_slot(acct, engine).await;
            }
        }
        start_engine_if_possible(app, state, &transition).await
    };
    PasswordChangedDto {
        status: "changed",
        signed_in: true,
        restart: if restart.is_ok() { "ok" } else { "failed" },
    }
}

/// DW-A65: change the sign-in email (step-up and OPAQUE re-registration under the new
/// address in core). The server then sends an 8-digit code to the new address.
#[tauri::command]
pub(super) async fn account_change_email(
    state: State<'_, AppState>,
    new_email: String,
    password: String,
) -> Result<EmailChangedDto, String> {
    let password = Zeroizing::new(password);
    session_command!(async {
        let (_acct, token, binding) = session_parts(&state)?;
        let flow = AccountFlow::change_email(&new_email, &password, binding).map_err(code)?;
        match drive(flow, Some(&token), None).await? {
            Driven::Outcome(AccountOutcome::EmailChanged { email }) => Ok(EmailChangedDto { email }),
            _ => Err("protocol".to_string()),
        }
    })
}

/// DW-A66: the 8-digit code sent to the new address.
#[tauri::command]
pub(super) async fn account_verify_email(state: State<'_, AppState>, code: String) -> Result<(), String> {
    let code = Zeroizing::new(code);
    session_command!(async { run_simple(&state, AccountFlow::verify_email(&code)).await.map(|_| ()) })
}

/// DW-A64: the display name, 1 to 64 characters.
#[tauri::command]
pub(super) async fn account_set_name(state: State<'_, AppState>, name: String) -> Result<(), String> {
    session_command!(async {
        run_simple(&state, AccountFlow::set_display_name(&name))
            .await
            .map(|_| ())
    })
}

/// DW-A69: start TOTP set-up. With TOTP on, pass the password (step-up in core) or the
/// current code (moving to a new authenticator). The backup codes stay in Rust.
#[tauri::command]
pub(super) async fn account_totp_setup(
    state: State<'_, AppState>,
    password: Option<String>,
    current_code: Option<String>,
) -> Result<TotpSetupDto, String> {
    let password = password.map(Zeroizing::new);
    let current_code = current_code.map(Zeroizing::new);
    session_command!(async {
        let flow = AccountFlow::totp_setup(
            password.as_deref().map(String::as_str),
            current_code.as_deref().map(String::as_str),
        );
        match run_simple(&state, flow).await? {
            AccountOutcome::TotpEnrollment(enrollment) => {
                let dto = TotpSetupDto {
                    secret: enrollment.secret.to_string(),
                    otpauth_uri: enrollment.otpauth_uri.to_string(),
                    qr_rows: qr_rows(&enrollment.otpauth_uri)?,
                };
                pending()?.totp = Some(enrollment);
                Ok(dto)
            }
            _ => Err("protocol".to_string()),
        }
    })
}

/// DW-A69 Turn on: confirm with the first code; returns the backup codes once (DW-A70).
#[tauri::command]
pub(super) async fn account_totp_confirm(state: State<'_, AppState>, code: String) -> Result<BackupCodesDto, String> {
    let code = Zeroizing::new(code);
    session_command!(async {
        let enrollment = pending()?.totp.take().ok_or_else(|| "out_of_order".to_string())?;
        match run_simple(&state, AccountFlow::totp_confirm(enrollment, &code)).await? {
            AccountOutcome::BackupCodes(codes) => Ok(BackupCodesDto {
                backup_codes: codes.to_vec(),
            }),
            _ => Err("protocol".to_string()),
        }
    })
}

/// DW-A72: turn TOTP off with a current code.
#[tauri::command]
pub(super) async fn account_totp_disable(state: State<'_, AppState>, code: String) -> Result<(), String> {
    let code = Zeroizing::new(code);
    session_command!(async { run_simple(&state, AccountFlow::totp_disable(&code)).await.map(|_| ()) })
}

/// DW-A77 to A79: record sign-in activity on or off. Off takes the password: core steps up inside (server #422).
#[tauri::command]
pub(super) async fn account_set_activity_logging(
    state: State<'_, AppState>,
    on: bool,
    password: Option<String>,
) -> Result<(), String> {
    let password = password.map(Zeroizing::new);
    session_command!(async {
        run_simple(
            &state,
            AccountFlow::set_activity_logging(on, password.as_deref().map(String::as_str)),
        )
        .await
        .map(|_| ())
    })
}

/// DW-A71 Make new backup codes: the current authenticator code; the authenticator stays, the old codes stop
/// working. Returns the 8 new codes once (DW-A70).
#[tauri::command]
pub(super) async fn account_backup_codes_regenerate(
    state: State<'_, AppState>,
    code: String,
) -> Result<BackupCodesDto, String> {
    let code = Zeroizing::new(code);
    session_command!(async {
        match run_simple(&state, AccountFlow::regenerate_backup_codes(&code)).await? {
            AccountOutcome::BackupCodes(codes) => Ok(BackupCodesDto {
                backup_codes: codes.to_vec(),
            }),
            _ => Err("protocol".to_string()),
        }
    })
}

/// DW-A58, delete step 1: check the password (a step-up whose token is dropped). Nothing
/// is scheduled. The checked password stays in Rust for step 2's step-up.
#[tauri::command]
pub(super) async fn account_delete_check_password(state: State<'_, AppState>, password: String) -> Result<(), String> {
    let password = Zeroizing::new(password);
    session_command!(async {
        run_simple(&state, AccountFlow::verify_password(&password)).await?;
        pending()?.delete_password = Some(password.clone());
        Ok(())
    })
}

/// DW-A80, delete step 2: the second factor, then the delete request (step-up inside core,
/// `POST /auth/account/delete-request`). `code`: 6-digit authenticator or 8-digit backup code;
/// `None` for an account without two-factor. The server mails a single-use link and schedules
/// nothing. A wrong or missing code keeps the checked password for another try.
#[tauri::command]
pub(super) async fn account_delete_confirm(
    state: State<'_, AppState>,
    code: Option<String>,
) -> Result<DeleteRequestedDto, String> {
    let code = code.map(Zeroizing::new);
    session_command!(async {
        let password = pending()?
            .delete_password
            .clone()
            .ok_or_else(|| "out_of_order".to_string())?;
        let flow = AccountFlow::delete_account(&password, code.as_deref().map(String::as_str));
        match run_simple(&state, flow).await? {
            AccountOutcome::DeletionRequested { link_expires_at } => {
                pending()?.delete_password = None;
                Ok(DeleteRequestedDto {
                    email_link_sent: true,
                    link_expires_at,
                })
            }
            _ => Err("protocol".to_string()),
        }
    })
}

/// DW-A81 → A82: confirm with the mailed link, pasted here (the link's own page in the browser does the same).
/// Public route: no session needed. The link stays in Rust for Undo.
#[tauri::command]
pub(super) async fn account_delete_confirm_link(link: String) -> Result<DeletionScheduledDto, String> {
    let link = Zeroizing::new(link);
    match drive(AccountFlow::delete_confirm(&link).map_err(code)?, None, None).await? {
        Driven::Outcome(AccountOutcome::DeletionScheduled {
            shred_after,
            cancel_until,
        }) => {
            pending()?.delete_link = Some(link);
            Ok(DeletionScheduledDto {
                shred_after,
                cancel_until,
            })
        }
        _ => Err("protocol".to_string()),
    }
}

/// Leave the delete flow: the checked password is wiped.
#[tauri::command]
pub(super) fn account_delete_cancel() -> Result<(), String> {
    pending()?.delete_password = None;
    Ok(())
}

/// DW-A82 Undo deletion (`POST /auth/account/delete-cancel`, public): with the link the person
/// pastes, or the one pasted on step 3. Neither: `link_needed`, and the screen asks for it.
#[tauri::command]
pub(super) async fn account_delete_undo(link: Option<String>) -> Result<DeletionCancelledDto, String> {
    let given = link.filter(|l| !l.trim().is_empty()).map(Zeroizing::new);
    let link = match given {
        Some(l) => l,
        None => pending()?
            .delete_link
            .clone()
            .ok_or_else(|| "link_needed".to_string())?,
    };
    match drive(AccountFlow::delete_cancel(&link).map_err(code)?, None, None).await? {
        Driven::Outcome(AccountOutcome::DeletionCancelled { not_restored }) => {
            pending()?.delete_link = None;
            Ok(DeletionCancelledDto { not_restored })
        }
        _ => Err("protocol".to_string()),
    }
}

/// Leave TOTP set-up without confirming: the pending enrollment is wiped.
#[tauri::command]
pub(super) fn account_totp_abandon() -> Result<(), String> {
    pending()?.totp = None;
    Ok(())
}
