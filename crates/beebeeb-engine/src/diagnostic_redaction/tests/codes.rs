//! `codes` tests split out of `diagnostic_redaction.rs`.
//!
//! Typed error-code tests, split out of `diagnostic_redaction.rs`.

use super::*;

#[test]
fn error_code_is_a_closed_value_that_cannot_carry_a_name() {
    let json = serde_json::to_string(&classify_error_code(ISSUE_ERROR)).unwrap();
    assert_eq!(json, "\"other\"");
    assert_eq!(
        classify_error_code("write /Users/guus/Tax 2025/aangifte.pdf: Permission denied (os error 13)"),
        DiagnosticErrorCode::Permission
    );
    assert_eq!(
        classify_error_code("staged upload payload is missing: /Users/guus/x.pdf"),
        DiagnosticErrorCode::PayloadMissing
    );
    assert_eq!(
        classify_error_code("create dest dir /Users/guus/Tax 2025: No such file or directory (os error 2)"),
        DiagnosticErrorCode::LocalWriteFailed
    );
    assert_eq!(
        classify_error_code("local path must stay under the sync root: rel_path must not be absolute"),
        DiagnosticErrorCode::PathRejected
    );
    assert_eq!(
        classify_error_code("ffmpeg video thumbnail failed: /Users/guus/movie.mov: Invalid data"),
        DiagnosticErrorCode::Thumbnail
    );
    assert_eq!(
        classify_error_code("error sending request for url (http://x/): connection refused"),
        DiagnosticErrorCode::Network
    );
    assert_eq!(
        classify_error_code("500 Internal Server Error"),
        DiagnosticErrorCode::ServerError
    );
    assert_eq!(
        classify_error_code("engine is stopping; refusing to enqueue a new local write"),
        DiagnosticErrorCode::EngineStopping
    );
    assert_eq!(
        classify_error_code("stale base version rejected"),
        DiagnosticErrorCode::StaleVersion
    );
}

/// A staged copy that exists but cannot be read (permission, I/O, not a
/// regular file) has its own code, distinct from a missing copy, never `Other`.
#[test]
fn an_unreadable_staged_copy_has_its_own_code() {
    assert_eq!(
        classify_error_code("staged upload payload could not be read"),
        DiagnosticErrorCode::PayloadUnreadable
    );
    assert_eq!(
        serde_json::to_string(&DiagnosticErrorCode::PayloadUnreadable).unwrap(),
        "\"payload_unreadable\""
    );
    assert_eq!(
        classify_error_code("staged upload payload is missing"),
        DiagnosticErrorCode::PayloadMissing,
        "the missing copy keeps its code, with or without a path"
    );
}

#[test]
fn group_labels_outside_the_allow_list_fold_into_other() {
    assert_eq!(allowed_label("upload_version", QUEUE_KIND_LABELS), "upload_version");
    assert_eq!(allowed_label("Tax 2025", QUEUE_KIND_LABELS), "other");
    assert_eq!(allowed_label("auth", PAUSE_REASON_LABELS), "auth");
    assert_eq!(allowed_label("key_replaced", PAUSE_REASON_LABELS), "key_replaced");
    // Task 2267 (fix round 2): changes found while sync was paused, held for the person's review.
    assert_eq!(
        allowed_label("found_while_paused", PAUSE_REASON_LABELS),
        "found_while_paused"
    );
    // Tasks 1888, 2226 (spec D4, D7): a reconcile hold, and the hold summary's computed group.
    assert_eq!(allowed_label("needs_review", PAUSE_REASON_LABELS), "needs_review");
    assert_eq!(
        allowed_label("waiting_for_folder", PAUSE_REASON_LABELS),
        "waiting_for_folder"
    );
    assert_eq!(allowed_label("/Users/guus", PAUSE_REASON_LABELS), "other");
}
