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
use tauri::State;
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

/// Perform each step of `flow` against the API with `token` (none for the public delete-link routes), to the end.
async fn drive(mut flow: AccountFlow, token: Option<&str>) -> Result<AccountOutcome, String> {
    let base = runner::api_base_url();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(crate::link_health::API_REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(|_| "network".to_string())?;
    let mut step = flow.start().map_err(code)?;
    loop {
        match step {
            Step::Done(outcome) => return Ok(outcome),
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
                let mut text = resp.text().await.unwrap_or_default();
                let parsed = ApiResponse::from_text(status, &text);
                text.zeroize();
                step = flow.resume(parsed).map_err(code)?;
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
    drive(flow, Some(&token)).await
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
/// The server ends every other session and mints a new one for this device: it is
/// stored in the credential store and swapped into memory in one session turn, and the
/// engine restarts with it. No master-key wrap (the key stays in the credential store).
#[tauri::command]
pub(super) async fn account_change_password(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    current: String,
    new_password: String,
) -> Result<(), String> {
    let current = Zeroizing::new(current);
    let new_password = Zeroizing::new(new_password);
    session_command!(async {
        let (acct, token, binding) = session_parts(&state)?;
        let mut turn = acct.session_generation();
        let policy = password_policy().await?;
        let breach = breach_check(&policy, &new_password).await;
        let flow =
            AccountFlow::change_password(&current, &new_password, binding, &policy, breach.as_ref()).map_err(code)?;
        let outcome = drive(flow, Some(&token)).await?;
        let AccountOutcome::PasswordChanged { session_token } = outcome else {
            return Err("protocol".to_string());
        };
        install_rotated_session(app, &state, &acct, &mut turn, &session_token).await
    })
}

/// Stop the engine (it holds the ended token), store and swap the new token in one
/// session turn, then start the engine again with it.
async fn install_rotated_session(
    app: tauri::AppHandle,
    state: &State<'_, AppState>,
    acct: &Arc<account::AccountRuntime>,
    turn: &mut account::SessionGeneration,
    new_token: &str,
) -> Result<(), String> {
    {
        let mut slot = acct.engine.lock().await;
        if let Some(engine) = slot.take() {
            let _ = stop_engine_in_slot(acct, engine).await;
        }
    }
    {
        let write = claim_session_write(acct, turn).map_err(String::from)?;
        let email = acct
            .session
            .lock()
            .ok()
            .and_then(|g| g.as_ref().and_then(|s| s.email.clone()));
        persist_session_token_to_keychain(&write, acct.id.as_str(), new_token, email.as_deref())?;
        let mut guard = acct.session.lock().map_err(|_| "internal".to_string())?;
        if let Some(session) = guard.as_mut() {
            let mut old = std::mem::replace(&mut session.token, new_token.to_string());
            old.zeroize();
        }
        drop(guard);
        drop(write);
    }
    start_engine_unless_running(app, state.inner()).await
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
        match drive(flow, Some(&token)).await? {
            AccountOutcome::EmailChanged { email } => Ok(EmailChangedDto { email }),
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
    match drive(AccountFlow::delete_confirm(&link).map_err(code)?, None).await? {
        AccountOutcome::DeletionScheduled {
            shred_after,
            cancel_until,
        } => {
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
    match drive(AccountFlow::delete_cancel(&link).map_err(code)?, None).await? {
        AccountOutcome::DeletionCancelled { not_restored } => {
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
