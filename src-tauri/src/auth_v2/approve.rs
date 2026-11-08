//! Approving a sign-in from this PC (DW-A12 to DW-A17).
//!
//! `auth_approve_lookup(code)` finds the waiting request (core `Approval::lookup`) and returns what the server measured
//! and what the device reported. `auth_approve_request(code, password, approve)` with `approve = true` raises Windows
//! Hello for this window first (crypto review rule 4), then runs core's `ApproveRun`: the OPAQUE step-up with purpose
//! `cli_device_approval` bound to the code, the claim (the confirm token goes on that request and is dropped), and the
//! relay of `{session_token, master_key}` sealed to the requester's key. The password, the confirm token, the new
//! session token and the master key never leave Rust; the web view gets a status. `approve = false` denies it
//! through the server's deny route: nothing is shared and the requesting device shows DW-A09.
use std::sync::{LazyLock, Mutex};

use beebeeb_core::auth_flow::AuthFlowError;
use beebeeb_core::auth_flow::device::{Approval, ApproveStep, PendingApproval};

use super::hello::{self, Verification};
use super::transport;

/// The review screen's facts. Measured by the server: `country`, `ip`, `requested_at`, `expires_at`. Reported by the
/// requesting device (unverified): `device_name`, `client`, `os`.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub(crate) struct ApprovalReview {
    pub(crate) code: String,
    pub(crate) country: Option<String>,
    pub(crate) ip: String,
    pub(crate) requested_at: String,
    pub(crate) expires_at: String,
    pub(crate) seconds_left: i64,
    pub(crate) device_name: Option<String>,
    pub(crate) client: Option<String>,
    pub(crate) os: Option<String>,
}

/// `approved`, `denied` (the person said no here).
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub(crate) struct ApprovalResult {
    pub(crate) status: String,
}

static PENDING: LazyLock<Mutex<Option<PendingApproval>>> = LazyLock::new(|| Mutex::new(None));

fn session_token(state: &crate::AppState) -> Result<zeroize::Zeroizing<String>, String> {
    let acct = state.active_account()?;
    let guard = acct.session.lock().map_err(|_| "locked".to_string())?;
    guard
        .as_ref()
        .map(|s| zeroize::Zeroizing::new(s.token.clone()))
        .ok_or_else(|| "locked".to_string())
}

fn master_key(state: &crate::AppState) -> Result<zeroize::Zeroizing<[u8; 32]>, String> {
    let acct = state.active_account()?;
    let guard = acct.session.lock().map_err(|_| "locked".to_string())?;
    guard
        .as_ref()
        .map(|s| zeroize::Zeroizing::new(s.master_key))
        .ok_or_else(|| "locked".to_string())
}

/// Find a waiting request by the typed code. One honest answer for a wrong, expired or used code
/// (`request_not_found`).
#[tauri::command]
pub(crate) async fn auth_approve_lookup(
    state: tauri::State<'_, crate::AppState>,
    code: String,
) -> Result<ApprovalReview, String> {
    let token = session_token(&state)?;
    let (lookup, req) = Approval::lookup(&code).map_err(|e| super::flow_code(&e))?;
    let client = transport::client()?;
    let resp = transport::send(&client, Some(&token), req).await?;
    let pending = lookup.on_response(resp).map_err(|e| super::flow_code(&e))?;
    let f = pending.facts().clone();
    let now = chrono::Utc::now().timestamp();
    let review = ApprovalReview {
        code: pending.code().to_string(),
        seconds_left: f.seconds_left(now).unwrap_or(0),
        country: f.country,
        ip: f.ip,
        requested_at: f.requested_at,
        expires_at: f.expires_at,
        device_name: f.device_name,
        client: f.client,
        os: f.os,
    };
    *PENDING.lock().map_err(|_| "failed".to_string())? = Some(pending);
    Ok(review)
}

/// Approve (Windows Hello, then step-up, claim, relay) or deny the request found by `auth_approve_lookup`.
#[tauri::command]
pub(crate) async fn auth_approve_request(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, crate::AppState>,
    code: String,
    password: String,
    approve: bool,
) -> Result<ApprovalResult, String> {
    let password = zeroize::Zeroizing::new(password);
    if !approve {
        // Deny (server #422, `POST /auth/cli-deny`): nothing is claimed or relayed, and the requesting device is told
        // "denied" at once (DW-A09). No step-up: a denial grants nothing.
        let pending = PENDING
            .lock()
            .map_err(|_| "failed".to_string())?
            .take()
            .ok_or_else(|| "request_not_found".to_string())?;
        let token = session_token(&state)?;
        let (denial, req) = pending.deny();
        let resp = transport::send(&transport::client()?, Some(&token), req).await?;
        denial.on_response(resp).map_err(|e| super::flow_code(&e))?;
        return Ok(ApprovalResult {
            status: "denied".into(),
        });
    }
    {
        let guard = PENDING.lock().map_err(|_| "failed".to_string())?;
        let pending = guard.as_ref().ok_or_else(|| "request_not_found".to_string())?;
        if !pending.code_matches(&code) {
            return Err("code_mismatch".into());
        }
    }
    let reason = {
        let guard = PENDING.lock().map_err(|_| "failed".to_string())?;
        let name = guard
            .as_ref()
            .and_then(|p| p.facts().device_name.clone())
            .unwrap_or_else(|| "that device".into());
        format!("Approve Drive on {name}")
    };
    // Rule 4: a fresh native verification every time, before anything is claimed.
    match hello::verify(&window, reason).await {
        Verification::Verified => {}
        other => return Err(other.code().into()),
    }
    let pending = PENDING
        .lock()
        .map_err(|_| "failed".to_string())?
        .take()
        .ok_or_else(|| "request_not_found".to_string())?;
    let token = session_token(&state)?;
    let key = master_key(&state)?;
    let (mut run, mut req) = pending
        .approve(&code, password.as_bytes(), &key)
        .map_err(|e| super::flow_code(&e))?;
    drop(key);
    let client = transport::client()?;
    loop {
        let resp = transport::send(&client, Some(&token), req).await?;
        match run.on_response(resp) {
            Ok(ApproveStep::Send(next)) => req = next,
            Ok(ApproveStep::Approved) => {
                return Ok(ApprovalResult {
                    status: "approved".into(),
                });
            }
            Err(AuthFlowError::InvalidCredentials) => return Err("invalid_credentials".into()),
            Err(e) => return Err(super::flow_code(&e)),
        }
    }
}
