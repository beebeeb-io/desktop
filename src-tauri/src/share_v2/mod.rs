//! Desktop v2 (B05): Trash's permanent delete and the Shared commands (spec 3.2), called from
//! `src/v2/sections/{shared,trash}` through `desktopApi.ts`'s `command()`.
//!
//! The boundary is the one every v2 command keeps (crypto review 2026-11-08):
//! - a secret goes one way, web view to Rust: `drive_delete_permanently` takes the password and does
//!   core's OPAQUE step-up (`auth_flow::step_up`) itself, per delete, and drops each single-use token
//!   on the request that spends it (rule 11). No command returns a token, a key or a link fragment.
//! - a share link that carries a key is put on the clipboard by Rust; the web view gets only the
//!   display form without the part after `#`.
//!
//! The share flows run in core (`beebeeb_core::share_flow`, B05b/B05c): `share_invite` (a person
//! share of a file or, through `folder_flows.rs`, a folder), `share_link_copy` (rebuild the owner's
//! link, or mint one: a file link, a folder link, or a bundle over several files, with the Files
//! dialog's expiry and password), `share_revoke`, and `file_request_create|copy|update` (close) in
//! `requests.rs`. `reads.rs` (B05d) holds the lists and member commands; four commands with no server route or core flow answer `unsupported: not implemented`, and the
//! window shows its honest "isn't available in this version" copy. Never a half flow behind a
//! working command.

use beebeeb_core::auth_flow::step_up::{StepUp, StepUpPurpose, StepUpStep};
use beebeeb_core::auth_flow::{AuthFlowError, HttpRequest, Method};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};
use zeroize::Zeroizing;

use crate::AppState;
use crate::auth_v2::transport;

type Answer<T> = Result<T, String>;

const EVENT: &str = "drive://event";
const NOT_YET: &str = "unsupported: not implemented in this build";

pub(crate) mod flows;
pub(crate) mod folder_flows;
#[cfg(test)]
mod marks_tests;
pub(crate) mod reads;
pub(crate) mod requests;
pub(crate) mod safety;
#[cfg(test)]
mod safety_tests;
#[cfg(test)]
mod tests;

/// The unlocked session's bearer token, copied into a wiped buffer. Locked: `locked`.
fn session_token(state: &AppState) -> Answer<Zeroizing<String>> {
    let acct = state.active_account().map_err(|_| "signed-out".to_string())?;
    let guard = acct.session.lock().map_err(|_| "failed".to_string())?;
    let session = guard.as_ref().ok_or_else(|| "locked".to_string())?;
    Ok(Zeroizing::new(session.token.clone()))
}

fn auth_code(e: AuthFlowError) -> String {
    e.code().to_string()
}

/// The permanent delete of one item, on the session, without a token: core puts the token on it.
pub(crate) fn permanent_delete_request(id: &str) -> HttpRequest {
    flows::request(Method::Delete, format!("/api/v1/files/{id}/permanent"), None)
}

/// One OPAQUE step-up through core, and the DELETE that spends its single-use token. Core attaches
/// the token to the request itself (`ConfirmToken::attach`); this app never holds it as a string,
/// and the transport reads it only through `confirm_token_header()` (core v0.2.8 item 4).
async fn delete_with_step_up(client: &reqwest::Client, session: &str, password: &[u8], id: &str) -> Answer<()> {
    let (mut flow, mut req) = StepUp::start(password, StepUpPurpose::Generic).map_err(auth_code)?;
    let token = loop {
        let resp = transport::send(client, Some(session), req).await?;
        match flow.on_response(resp).map_err(auth_code)? {
            StepUpStep::Send(next) => req = next,
            StepUpStep::Confirmed(token) => break token,
        }
    };
    let resp = transport::send(client, Some(session), token.attach(permanent_delete_request(id))).await?;
    delete_answer(resp.status)
}

/// What the permanent DELETE's status answers: the window's signed-out and rate-limit words for an
/// ended session and a rate limit, as the other Shared and Trash commands answer them.
pub(crate) fn delete_answer(status: u16) -> Answer<()> {
    match status {
        200..=299 => Ok(()),
        401 | 403 => Err("signed-out".into()),
        404 => Err("not-found".into()),
        429 => Err("rate_limited".into()),
        _ => Err("failed".into()),
    }
}

/// Checks the ids before anything is sent: non-empty, each already a canonical lower-case UUID
/// (never normalized, as `flows::item_id`), no repeats.
pub(crate) fn valid_ids(ids: &[String]) -> bool {
    !ids.is_empty()
        && ids.iter().all(|id| flows::item_id(id).is_ok())
        && ids.iter().collect::<std::collections::HashSet<_>>().len() == ids.len()
}

/// DW-0748, DW-0749, DW-0749b: delete items in Trash for good. The password proves the account
/// (core's step-up; a wrong one stops before anything is deleted); each delete spends its own
/// single-use token. A `removed` event per item drives the progress note. A two-factor account
/// steps up with its password too. An authenticator code alone (the board's DW-0749) cannot mint a
/// token: the server's step-up legs (`/auth/confirm`, `confirm-opaque-*`) take no TOTP or backup
/// code (B05b, checked against the server routes), so it stays `unsupported`.
#[tauri::command]
pub(crate) async fn drive_delete_permanently(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    ids: Vec<String>,
    password: Option<String>,
    code: Option<String>,
) -> Answer<()> {
    let password = Zeroizing::new(password.unwrap_or_default());
    let code = Zeroizing::new(code.unwrap_or_default());
    if !valid_ids(&ids) {
        return Err("not-found".into());
    }
    if password.is_empty() {
        return Err(if code.is_empty() {
            "invalid_credentials".into()
        } else {
            NOT_YET.into()
        });
    }
    let session = session_token(&state)?;
    let client = transport::client()?;
    for id in &ids {
        delete_with_step_up(&client, &session, password.as_bytes(), id).await?;
        let _ = app.emit(EVENT, serde_json::json!({ "kind": "removed", "id": id }));
    }
    let _ = app.emit(EVENT, serde_json::json!({ "kind": "changed", "folderId": null }));
    Ok(())
}

// ── Shared (spec 3.2). DTOs carry ids, names, emails, choices and display text only. ──────────

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ShareLists {
    pub(super) invitations: Vec<serde_json::Value>,
    pub(super) links: Vec<serde_json::Value>,
    pub(super) requests: Vec<serde_json::Value>,
    pub(super) pending: Vec<String>,
}

/// A link Rust put on the clipboard: the display form only (no `#` part).
#[derive(Serialize)]
pub(crate) struct CopiedLink {
    display: String,
}

/// The display form of a share link: everything before `#`. The key never crosses to the web view.
pub(crate) fn display_form(link: &str) -> String {
    beebeeb_core::share_flow::display_form(link)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileRequestInput {
    pub title: String,
    pub note: String,
    pub folder_name: String,
    pub expires_days: Option<u32>,
    pub max_files: Option<u32>,
    pub max_bytes: Option<u64>,
}

#[cfg(test)]
mod reads_tests;
