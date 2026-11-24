//! How a failed operation's error text is classed (auth, quota, permission, locked, or retryable). Moved out of
//! `src-tauri/src/engine_bridge/errors.rs` unchanged: the diagnostics classifier and the state database both read it.

use crate::state_db::OperationPauseReason;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationFailureClass {
    Retryable,
    Auth,
    Quota,
    Permission,
    Locked,
}

impl OperationFailureClass {
    pub fn pause_reason(self) -> Option<OperationPauseReason> {
        match self {
            OperationFailureClass::Retryable => None,
            OperationFailureClass::Auth => Some(OperationPauseReason::Auth),
            OperationFailureClass::Quota => Some(OperationPauseReason::Quota),
            OperationFailureClass::Permission => Some(OperationPauseReason::Permission),
            OperationFailureClass::Locked => Some(OperationPauseReason::Locked),
        }
    }
}

pub fn classify_operation_error(error: &str) -> OperationFailureClass {
    // Classify on the failure MESSAGE only, never on the request URL. reqwest's
    // Display appends `" for url (…)"` (for both HTTP-status and connection
    // errors), and that URL carries the random ephemeral port plus opaque path
    // ids. Those digits are not status codes, but the bare-code substring checks
    // below ("401", "403") would happily match them — so a plain HTTP 500 sent
    // to, say, `127.0.0.1:45401` was being misread as an Auth failure and
    // *paused* (never retried). That is the entire flake in task 1252: it fired
    // only when the OS handed the mock server a port whose digits contained
    // "401"/"403", which is why it reproduced in CI but ~never locally. Dropping
    // the URL suffix keeps classification keyed on reqwest's status reason phrase
    // ("401 Unauthorized", "403 Forbidden", "507 Insufficient Storage", …) and
    // on our own descriptive errors, none of which live past `" for url ("`.
    let message = error.split(" for url (").next().unwrap_or(error);
    let lower = message.to_ascii_lowercase();
    if lower.contains("401") || lower.contains("unauthorized") || lower.contains("invalid token") {
        OperationFailureClass::Auth
    } else if lower.contains("quota") || lower.contains("insufficient storage") || lower.contains("storage limit") {
        OperationFailureClass::Quota
    } else if lower.contains("403")
        || lower.contains("forbidden")
        || lower.contains("permission")
        || lower.contains("read-only")
    {
        OperationFailureClass::Permission
    } else if lower.contains("vault locked") || lower.contains("locked") || lower.contains("unlock") {
        OperationFailureClass::Locked
    } else {
        OperationFailureClass::Retryable
    }
}
