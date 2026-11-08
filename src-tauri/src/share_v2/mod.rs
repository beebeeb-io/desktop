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
//! `requests.rs`. The list commands and the rest answer `unsupported: not implemented`, and the
//! window shows its honest "isn't available in this version" copy. Never a half flow behind a
//! working command.

use beebeeb_core::auth_flow::step_up::{StepUp, StepUpPurpose, StepUpStep};
use beebeeb_core::auth_flow::{AuthFlowError, HttpRequest, Method, RequestAuth};
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
pub(crate) mod requests;
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

/// One OPAQUE step-up through core; the confirmation token comes back only as the header value
/// for the one request that spends it, in a wiped buffer, and never leaves this module.
async fn confirm_once(client: &reqwest::Client, session: &str, password: &[u8]) -> Answer<Zeroizing<String>> {
    let (mut flow, mut req) = StepUp::start(password, StepUpPurpose::Generic).map_err(auth_code)?;
    loop {
        let resp = transport::send(client, Some(session), req).await?;
        match flow.on_response(resp).map_err(auth_code)? {
            StepUpStep::Send(next) => req = next,
            StepUpStep::Confirmed(token) => {
                // `ConfirmToken` has no string accessor: it only attaches to a request. Attach it to a
                // carrier and move the header value straight onto the DELETE below.
                let carrier = token.attach(HttpRequest {
                    method: Method::Post,
                    path: String::new(),
                    body: None,
                    auth: RequestAuth::Session,
                    confirm_token: None,
                });
                return carrier.confirm_token.ok_or_else(|| "failed".to_string());
            }
        }
    }
}

/// Checks the ids before anything is sent: non-empty, each a UUID, no repeats.
pub(crate) fn valid_ids(ids: &[String]) -> bool {
    !ids.is_empty()
        && ids.iter().all(|id| id.parse::<uuid::Uuid>().is_ok())
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
    let api = crate::account_commands::api_client_from_session(&state)?;
    let client = transport::client()?;
    for id in &ids {
        let token = confirm_once(&client, &session, password.as_bytes()).await?;
        api.permanent_delete_file(id, &token).await.map_err(|e| {
            let t = e.to_string().to_ascii_lowercase();
            if t.contains("404") {
                "not-found".to_string()
            } else if t.contains("error sending") || t.contains("timed out") {
                transport::OFFLINE.to_string()
            } else {
                "failed".to_string()
            }
        })?;
        drop(token);
        let _ = app.emit(EVENT, serde_json::json!({ "kind": "removed", "id": id }));
    }
    let _ = app.emit(EVENT, serde_json::json!({ "kind": "changed", "folderId": null }));
    Ok(())
}

// ── Shared (spec 3.2). DTOs carry ids, names, emails, choices and display text only. ──────────

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ShareLists {
    invitations: Vec<serde_json::Value>,
    links: Vec<serde_json::Value>,
    requests: Vec<serde_json::Value>,
    pending: Vec<String>,
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

#[tauri::command]
pub(crate) async fn share_list(state: State<'_, AppState>) -> Answer<ShareLists> {
    session_token(&state)?;
    Err(NOT_YET.into())
}

macro_rules! not_yet {
    ($(fn $name:ident($($arg:ident: $ty:ty),*) -> $ret:ty;)*) => {$(
        #[tauri::command]
        #[allow(unused_variables)]
        pub(crate) async fn $name(state: State<'_, AppState>, $($arg: $ty),*) -> Answer<$ret> {
            session_token(&state)?;
            Err(NOT_YET.into())
        }
    )*};
}

not_yet! {
    fn share_accept(id: String) -> ();
    fn share_decline(id: String) -> ();
    fn share_leave(id: String) -> ();
    fn share_people(item_id: String) -> Vec<serde_json::Value>;
    fn share_set_access(item_id: String, member_id: String, can_edit: bool) -> ();
    fn share_link_update(item_id: String, expires_days: Option<u32>, set_expiry: bool, password: Option<String>, set_password: bool) -> ();
    fn share_safety_number(member_id: String) -> serde_json::Value;
    fn share_mark_compared(member_id: String, matched: bool) -> ();
    fn file_request_remove(id: String) -> ();
}
