//! Delete a whole backup from the app's window (LD-0113): a device's folder, or one backed-up folder of a device.
//!
//! The web app's call: the password is proven to the server (core's OPAQUE step-up), which mints one single-use
//! confirmation token; the delete of that one folder (`DELETE /api/v1/files/{id}`, a move to Trash) spends it.
//! Core puts the token on the request itself; this module never holds it as a string. The rule is asked first
//! ([`crate::backup_actions::check`]): only a row that answers [`Verdict::NeedsPassword`] is sent, so the Backups
//! root, a row outside the tree, one item inside a backup and this device's own active backup never reach the
//! network.
//!
//! Must not: log the password, the token, or a refusal's words; delete anything but the one row asked for; retry
//! with the same token; fall back to a call without the token.

use beebeeb_core::auth_flow::step_up::{StepUp, StepUpPurpose, StepUpStep};
use beebeeb_core::auth_flow::{AuthFlowError, HttpRequest, Method};

use crate::auth_transport;
use crate::backup_actions::{self, Action, Verdict};
use crate::backup_protect::OwnBackups;
use crate::state_db::StateDb;

/// Why a backup was not deleted. Nothing was deleted in any of these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteBackupError {
    /// No such row (or an id that is not a file id).
    NotFound,
    /// The row is not a whole backup (outside the tree, or one item inside a backup).
    NotABackup,
    /// The Backups rule refuses it (the root, this device's own active backup); the words to show, never to log.
    Refused(String),
    /// The password was empty or not the account's.
    WrongPassword,
    /// The session ended.
    SignedOut,
    /// Too many tries; seconds to wait.
    RateLimited(u64),
    /// No answer from the server.
    Offline,
    /// The state database could not be read, or the server answered something else. A stable code, no detail.
    Failed(&'static str),
}

/// The delete of one item on the session, without a token: core puts the token on it.
fn delete_request(id: &str) -> HttpRequest {
    let mut req = HttpRequest::session_post(format!("/api/v1/files/{id}"), serde_json::Value::Null);
    req.method = Method::Delete;
    req.body = None;
    req
}

/// A server file id as the listing gives it: a canonical lower-case UUID, never normalized (it goes into a URL).
fn is_file_id(id: &str) -> bool {
    matches!(uuid::Uuid::parse_str(id), Ok(u) if u.hyphenated().to_string() == id)
}

fn step_up_error(e: AuthFlowError) -> DeleteBackupError {
    match e {
        AuthFlowError::InvalidCredentials | AuthFlowError::InvalidInput(_) => DeleteBackupError::WrongPassword,
        AuthFlowError::RateLimited { retry_after_secs } => DeleteBackupError::RateLimited(retry_after_secs),
        other => DeleteBackupError::Failed(other.code()),
    }
}

/// The transport fails only when no answer came (a session is always passed).
fn transport_error(_: String) -> DeleteBackupError {
    DeleteBackupError::Offline
}

/// The rule's answer for `file_id`, before anything is sent.
fn allowed(db: &StateDb, own: &OwnBackups, file_id: &str) -> Result<(), DeleteBackupError> {
    if !is_file_id(file_id) {
        return Err(DeleteBackupError::NotFound);
    }
    match backup_actions::check(db, own, file_id, Action::DeleteBackup) {
        Err(_) => Err(DeleteBackupError::Failed("state")),
        Ok(None) => Err(DeleteBackupError::NotFound),
        Ok(Some(Verdict::NeedsPassword)) => Ok(()),
        Ok(Some(Verdict::Refused(words))) => Err(DeleteBackupError::Refused(words)),
        Ok(Some(_)) => Err(DeleteBackupError::NotABackup),
    }
}

/// Delete the backup `file_id` (a device's folder or one backed-up folder) after proving `password`.
///
/// `session` is the signed-in session's token; `api_base` has no trailing slash. The folder goes to Trash on the
/// server; this device's rows follow on the engine's next pull.
pub async fn delete_backup(
    api_base: &str,
    client: &reqwest::Client,
    session: &str,
    db: &StateDb,
    own: &OwnBackups,
    file_id: &str,
    password: &[u8],
) -> Result<(), DeleteBackupError> {
    allowed(db, own, file_id)?;
    let (mut flow, mut req) = StepUp::start(password, StepUpPurpose::Generic).map_err(step_up_error)?;
    let token = loop {
        let resp = auth_transport::send_to(api_base, client, Some(session), req)
            .await
            .map_err(transport_error)?;
        match flow.on_response(resp).map_err(step_up_error)? {
            StepUpStep::Send(next) => req = next,
            StepUpStep::Confirmed(token) => break token,
        }
    };
    let resp = auth_transport::send_to(api_base, client, Some(session), token.attach(delete_request(file_id)))
        .await
        .map_err(transport_error)?;
    match resp.status {
        200..=299 => Ok(()),
        401 => Err(DeleteBackupError::SignedOut),
        404 => Err(DeleteBackupError::NotFound),
        429 => Err(DeleteBackupError::RateLimited(resp.retry_after_secs.unwrap_or(30))),
        _ => Err(DeleteBackupError::Failed("server")),
    }
}

#[cfg(test)]
#[path = "backup_delete_tests.rs"]
mod tests;
