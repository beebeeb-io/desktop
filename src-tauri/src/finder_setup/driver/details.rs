//! The "Copy details" text (spec §6.2): app version, macOS version, reason, NSError domain and
//! code, attempt count, the time of the last attempt, and the lifecycle log's tail (§8). Never a
//! path, a file name, the email or the account id.
//!
//! Tests: `tests/view.rs`.

use super::*;

/// The text of "Copy details" (spec §6.2): app version, macOS version, reason, NSError domain +
/// code, attempt count, the time of the last attempt, and the lifecycle log's tail (§8). Never a
/// path, a file name, the email or the account id: the view carries none, and the tail is already
/// redacted. Private on purpose: callers use [`copy_details`], which reads the tail itself, so no
/// caller can hand it a short or an empty one.
pub(super) fn copy_details_text(
    view: &FinderSetupView,
    app_version: &str,
    macos_version: &str,
    lifecycle_tail: &[String],
) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Beebeeb {} on macOS {}",
        lifecycle_log::token(app_version),
        lifecycle_log::token(macos_version)
    );
    let _ = writeln!(out, "Finder setup: {}", view.setup.as_str());
    let reason = view.reason.or(view.last_failure.as_ref().map(|f| f.reason));
    let _ = writeln!(out, "Reason: {}", reason.map_or("none", FinderFailureReason::as_str));
    match &view.last_failure {
        Some(failure) => {
            let _ = writeln!(out, "Error: {} {}", lifecycle_log::token(&failure.domain), failure.code);
            let at = chrono::DateTime::from_timestamp(failure.at, 0)
                .map(|d| d.format("%Y-%m-%dT%H:%M:%SZ").to_string())
                .unwrap_or_else(|| "unknown".to_string());
            let _ = writeln!(out, "Last attempt: {at}");
        }
        None => {
            let _ = writeln!(out, "Error: none");
        }
    }
    let _ = writeln!(
        out,
        "Attempts: {} of {}",
        view.attempt,
        view.max_attempts.max(view.attempt)
    );
    let _ = writeln!(out, "Launched from: {}", view.launch_location.as_str());
    let _ = writeln!(out, "\nLifecycle log (last {} lines):", lifecycle_tail.len());
    for line in lifecycle_tail {
        let _ = writeln!(out, "{line}");
    }
    out
}

/// The one "Copy details" there is, for the commands to call: the view plus the last
/// `TAIL_LINES` lines of the real lifecycle log (lead ruling T5-b).
pub fn copy_details(view: &FinderSetupView, app_version: &str, macos_version: &str) -> String {
    copy_details_from(view, app_version, macos_version, lifecycle_log::tail)
}

pub(super) fn copy_details_from(
    view: &FinderSetupView,
    app_version: &str,
    macos_version: &str,
    tail: impl FnOnce(usize) -> Vec<String>,
) -> String {
    copy_details_text(view, app_version, macos_version, &tail(lifecycle_log::TAIL_LINES))
}
