//! `classify` items split out of `diagnostic_redaction.rs`.

use super::*;

/// Closed set of failure classes the support bundle may report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticErrorCode {
    Auth,
    Quota,
    Permission,
    Locked,
    StaleVersion,
    PayloadMissing,
    PayloadUnreadable,
    LocalWriteFailed,
    PathRejected,
    Timeout,
    Network,
    ServerError,
    Crypto,
    Thumbnail,
    EngineStopping,
    WorkerNotAttached,
    Other,
}

/// Classify a raw engine error into a [`DiagnosticErrorCode`]. Reads the raw
/// message (names and all) but returns a value that cannot contain any of it.
pub fn classify_error_code(error: &str) -> DiagnosticErrorCode {
    use crate::operation_failure::{OperationFailureClass, classify_operation_error};
    match classify_operation_error(error) {
        OperationFailureClass::Auth => return DiagnosticErrorCode::Auth,
        OperationFailureClass::Quota => return DiagnosticErrorCode::Quota,
        OperationFailureClass::Permission => return DiagnosticErrorCode::Permission,
        OperationFailureClass::Locked => return DiagnosticErrorCode::Locked,
        OperationFailureClass::Retryable => {}
    }
    // Same rule as `classify_operation_error`: never classify on the URL tail.
    let message = error.split(" for url (").next().unwrap_or(error);
    let m = message.to_ascii_lowercase();
    let has = |needle: &str| m.contains(needle);
    if has("engine is stopping") {
        DiagnosticErrorCode::EngineStopping
    } else if has("not yet attached") {
        DiagnosticErrorCode::WorkerNotAttached
    } else if has("stale") || has("base version") {
        DiagnosticErrorCode::StaleVersion
    } else if has("staged upload payload is missing") || has("staged upload ended") {
        DiagnosticErrorCode::PayloadMissing
    } else if has("staged upload payload could not be read") {
        // The copy exists but cannot be read (permission, I/O, not a regular file).
        DiagnosticErrorCode::PayloadUnreadable
    } else if has("must stay under")
        || has("unsafe")
        || has("not within an allowed root")
        || has("rel_path must")
        || has("non-normal path")
        || has("no parent directory")
    {
        DiagnosticErrorCode::PathRejected
    } else if m.starts_with("write ")
        || m.starts_with("rename ")
        || has("create dest dir")
        || has("hydrate destination")
    {
        DiagnosticErrorCode::LocalWriteFailed
    } else if has("thumbnail") || has("ffmpeg") {
        DiagnosticErrorCode::Thumbnail
    } else if has("encrypt") || has("decrypt") {
        DiagnosticErrorCode::Crypto
    } else if has("timed out") || has("timeout") {
        DiagnosticErrorCode::Timeout
    } else if has("server error")
        || has("internal server error")
        || has("bad gateway")
        || has("service unavailable")
        || has("gateway timeout")
    {
        DiagnosticErrorCode::ServerError
    } else if has("dns error") || has("connect") || has("connection") || has("error sending request") || has("network")
    {
        DiagnosticErrorCode::Network
    } else {
        DiagnosticErrorCode::Other
    }
}

/// Queue-group labels that may appear as keys in an export. Anything else is
/// folded into `other` so a stray DB value can never become a key.
pub const QUEUE_KIND_LABELS: &[&str] = &[
    "hydrate_file",
    "pin_tree",
    "upload_version",
    "upload_file",
    "create_folder",
    "rename_file",
    "move_file",
    "trash_file",
    "restore_file",
    "restore_version",
];

/// `needs_review` is a reconcile hold and `waiting_for_folder` the hold summary's computed group (tasks 1888, 2226;
/// spec D4, D7, D13).
pub const PAUSE_REASON_LABELS: &[&str] = &[
    "auth",
    "quota",
    "permission",
    "locked",
    "key_replaced",
    "found_while_paused",
    "needs_review",
    "waiting_for_folder",
];

/// `label` if it is one of `allowed`, otherwise `other`.
pub fn allowed_label(label: &str, allowed: &[&str]) -> String {
    if allowed.contains(&label) {
        label.to_string()
    } else {
        "other".to_string()
    }
}
