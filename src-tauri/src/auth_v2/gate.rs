//! The auth gate's commands. The rules they answer with (task 2445, Guus 2026-11-11: "every rule in Rust, not in
//! TSX") live in the engine (`beebeeb_engine::signin::gate`), where the native shell calls them directly; they are
//! re-exported here under their old path.

pub(crate) use beebeeb_engine::signin::gate::*;

use serde::Serialize;

use crate::auth_v2::key_owner::StoredKey;

/// The gate state for the main window: what screen to show. Reads the session
/// and auth_present flags synchronously; the plan check is driven by the caller.
///
/// The TSX calls this as: 1) `sync_status` (already exists), 2) if unlocked,
/// `account_subscription` (already exists), 3) `auth_gate_state` with the results.
/// This way the plan check stays one command call, and all rules live in Rust.
#[tauri::command]
pub(crate) async fn auth_gate_state(
    state: tauri::State<'_, crate::AppState>,
    plan_state: Option<String>,
    plan: Option<String>,
    plan_status: Option<String>,
    quota_bytes: Option<i64>,
) -> Result<GateState, String> {
    let acct = state.active_account()?;
    let vault_unlocked = acct.session.lock().map(|g| g.is_some()).unwrap_or(false);
    let auth_present = state.auth_present.lock().map(|g| *g).unwrap_or(false);
    let logged_in = vault_unlocked || auth_present;
    // Signed in but not unlocked: ask the credential store about the key the loader would read (the store that holds
    // the session token, with the loader's own ownership rule). The read can block (DPAPI, a locked Keychain) and on
    // Windows it loads the key and drops it at once, so it runs off the command thread. A store, or a worker, that
    // cannot answer is `Unreadable`: never "present".
    let stored_key = if logged_in && !vault_unlocked {
        let account_id = acct.id.as_str().to_string();
        tauri::async_runtime::spawn_blocking(move || crate::stored_key_state_for(&account_id))
            .await
            .unwrap_or(StoredKey::Unreadable)
    } else {
        StoredKey::Absent
    };
    let no_plan = has_no_plan(
        plan_state.as_deref(),
        plan.as_deref(),
        plan_status.as_deref(),
        quota_bytes,
    );
    Ok(decide_gate(logged_in, vault_unlocked, stored_key, no_plan))
}

/// Map a sign-in error code and the failure count to the board's problem and text.
/// The TSX calls this instead of doing the mapping itself.
#[tauri::command]
pub(crate) fn auth_sign_in_problem(code: String, consecutive_failures: u32) -> AuthSignInProblemDto {
    let problem = problem_from(&code, consecutive_failures);
    let text = problem_text(&problem).to_string();
    let lock_note = matches!(problem, SignInProblem::LockedHint).then(|| LOCK_NOTE.to_string());
    let wait_seconds = match &problem {
        SignInProblem::RateLimited { seconds } => Some(*seconds),
        _ => None,
    };
    AuthSignInProblemDto {
        problem,
        text,
        lock_note,
        wait_seconds,
    }
}

/// What `auth_sign_in_problem` returns.
#[derive(Debug, Serialize)]
pub(crate) struct AuthSignInProblemDto {
    pub problem: SignInProblem,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lock_note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait_seconds: Option<u32>,
}

/// What screen follows a sign-in. The TSX calls this with the `LoginOutcome`
/// fields and picks the screen from the result.
#[tauri::command]
pub(crate) fn auth_next_after_sign_in(
    requires_2fa: bool,
    vault_unlocked: bool,
    reauthenticated: bool,
    account_mismatch: bool,
) -> NextAfterSignIn {
    next_after_sign_in(requires_2fa, vault_unlocked, reauthenticated, account_mismatch)
}

/// The board's words for a 2FA verify error. The TSX shows this verbatim.
#[tauri::command]
pub(crate) fn auth_two_factor_error(code: String) -> String {
    two_factor_error(&code).to_string()
}

/// The board's words for a recovery-phrase error. The TSX shows this verbatim.
#[tauri::command]
pub(crate) fn auth_phrase_error(code: String) -> String {
    phrase_error(&code).to_string()
}

/// The board's words for an approve error. The TSX shows this verbatim.
#[tauri::command]
pub(crate) fn auth_approve_error_text(code: String) -> String {
    approve_error(&code).to_string()
}

/// The board's words for a device-request status. The TSX shows this verbatim.
#[tauri::command]
pub(crate) fn auth_device_status_text(status: String) -> String {
    device_status_text(&status).to_string()
}
