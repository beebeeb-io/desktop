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

/// The HTTP status an error text names, read only where a status is written, never from a number in a body, a
/// wait or an id: after `HTTP ` (this crate's `HTTP <status>: <body>`), inside reqwest's
/// `HTTP status client error (<status> …)`, after `status`, or as the first word (`401 Unauthorized`).
fn named_status(lower: &str) -> Option<u16> {
    fn code_at(text: &str) -> Option<u16> {
        let digits = text.bytes().take_while(u8::is_ascii_digit).count();
        (digits == 3)
            .then(|| text[..3].parse::<u16>().ok())
            .flatten()
            .filter(|code| (100..600).contains(code))
    }
    let after = |marker: &str| {
        lower
            .match_indices(marker)
            .find_map(|(at, _)| code_at(lower[at + marker.len()..].trim_start_matches([' ', ':', '=', '('])))
    };
    after("http ")
        .or_else(|| after("error ("))
        .or_else(|| after("status"))
        .or_else(|| code_at(lower.trim_start()))
}

/// Classify a typed API failure by its kind and status. The text is read only for what a status does not say
/// (a quota or lock named in a refusal's body).
pub fn classify_api_error(error: &crate::api_client::DesktopApiError) -> OperationFailureClass {
    use crate::api_client::DesktopApiError;
    match error {
        DesktopApiError::HttpStatus { status, snippet } => {
            classify(Some(status.as_u16()), &snippet.to_ascii_lowercase())
        }
        DesktopApiError::Transport(source) => classify(
            source.status().map(|status| status.as_u16()),
            &strip_url(&source.to_string()).to_ascii_lowercase(),
        ),
        other => classify_operation_error(&other.to_string()),
    }
}

/// reqwest's Display appends `" for url (…)"`; nothing past it is part of the failure.
fn strip_url(error: &str) -> &str {
    error.split(" for url (").next().unwrap_or(error)
}

pub fn classify_operation_error(error: &str) -> OperationFailureClass {
    // Classify on the failure MESSAGE only, never on the request URL: its ephemeral port and opaque path ids carry
    // digits that are not status codes (task 1252: a 500 sent to `127.0.0.1:45401` was read as Auth and paused).
    //
    // And never on a bare "401" or "403" anywhere in the text: a 429 whose body says `"retry_after": 401` is a wait
    // of 401 seconds, not a signed-out session. The status is read where a status is written (`named_status`).
    let lower = strip_url(error).to_ascii_lowercase();
    classify(named_status(&lower), &lower)
}

fn classify(status: Option<u16>, lower: &str) -> OperationFailureClass {
    match status {
        // A 429 is a wait, whatever its body says: its numbers are seconds and limits.
        Some(429) => return OperationFailureClass::Retryable,
        Some(401) => return OperationFailureClass::Auth,
        _ => {}
    }
    if lower.contains("unauthorized") || lower.contains("invalid token") {
        OperationFailureClass::Auth
    } else if status == Some(507)
        || lower.contains("quota")
        || lower.contains("insufficient storage")
        || lower.contains("storage limit")
    {
        OperationFailureClass::Quota
    } else if status == Some(403)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_client::DesktopApiError;
    use OperationFailureClass::{Auth, Permission, Quota, Retryable};

    fn http(status: u16, snippet: &str) -> DesktopApiError {
        DesktopApiError::HttpStatus {
            status: reqwest::StatusCode::from_u16(status).unwrap(),
            snippet: snippet.into(),
        }
    }

    #[test]
    fn operation_failure_a_retry_after_of_401_is_not_an_auth_failure() {
        for body in [
            r#"{"error":"rate_limited","retry_after":401}"#,
            r#"{"error":"banned","retry_after":403}"#,
            r#"{"error":"rate_limited","limit":401,"retry_after":4}"#,
        ] {
            let error = http(429, body);
            assert_eq!(classify_api_error(&error), Retryable, "{body}");
            assert_eq!(classify_operation_error(&error.to_string()), Retryable, "{body}");
            assert_eq!(
                classify_operation_error(&format!("listing the folder: {error}")),
                Retryable,
                "{body}"
            );
        }
    }

    #[test]
    fn operation_failure_reads_the_status_where_one_is_written() {
        for (text, class) in [
            ("HTTP 401 Unauthorized: bad token", Auth),
            ("HTTP 401: {}", Auth),
            ("401 Unauthorized", Auth),
            ("sync failed: HTTP 403 Forbidden: {}", Permission),
            (
                "HTTP status client error (401 Unauthorized) for url (http://127.0.0.1:1/x)",
                Auth,
            ),
            (
                "HTTP status client error (403 Forbidden) for url (http://127.0.0.1:45401/x)",
                Permission,
            ),
            ("server answered with status 403", Permission),
            ("HTTP 507 Insufficient Storage: {}", Quota),
            (r#"HTTP 403 Forbidden: {"error":"quota_exceeded"}"#, Quota),
            // Numbers that are not a status.
            (r#"HTTP 500 Internal Server Error: {"trace":"401"}"#, Retryable),
            ("HTTP 404 Not Found: file 4031 is gone", Retryable),
            (
                "HTTP status server error (500 Internal Server Error) for url (http://127.0.0.1:40113/x)",
                Retryable,
            ),
            ("moved 401 files, then the connection was reset", Retryable),
        ] {
            assert_eq!(classify_operation_error(text), class, "{text}");
        }
    }

    #[test]
    fn operation_failure_typed_errors_classify_by_status() {
        assert_eq!(classify_api_error(&http(401, "")), Auth);
        assert_eq!(classify_api_error(&http(403, "")), Permission);
        assert_eq!(classify_api_error(&http(507, "")), Quota);
        assert_eq!(classify_api_error(&http(500, r#"{"id":"403"}"#)), Retryable);
        assert_eq!(classify_api_error(&http(429, r#"{"retry_after":401}"#)), Retryable);
    }
}
