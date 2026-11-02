//! `errors` items split out of `engine_bridge.rs`.

use super::*;

/// A Finder write for an id this Mac never knew and no alias resolves (spec §7.3). The
/// reply is an error the extension reports as `cannotSynchronize`, so the edit stays on
/// disk; never "no such item", which would make the system delete it. It carries the id
/// the request named, for the refusal's log line (spec §11, through [`loggable_id`]); the
/// text never names it.
#[derive(Debug)]
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(crate) struct UnknownItem {
    pub requested_id: String,
}

impl std::fmt::Display for UnknownItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("this item is not known to Beebeeb")
    }
}

impl std::error::Error for UnknownItem {}

/// A fetch of an item whose held write is still queued, while that write's staged copy is
/// missing (spec §7.4). The bytes the token names exist nowhere else: the server holds only
/// the previous version, which is never served under the new token. Path-free on purpose: the
/// text reaches the extension and the person.
#[derive(Debug)]
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(crate) struct QueuedCopyMissing;

impl std::fmt::Display for QueuedCopyMissing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the latest save of this file is waiting to upload, and its copy cannot be read")
    }
}

impl std::error::Error for QueuedCopyMissing {}

/// Task 1640: a hydrate that would write over a sync-folder file holding a change that is not
/// sent (its bytes differ from the content this device last agreed with the server on, or its
/// upload is queued). Path-free on purpose.
#[derive(Debug)]
pub(crate) struct UnsentLocalChange;

impl std::fmt::Display for UnsentLocalChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("this file has a change that is not uploaded yet, so it is not replaced")
    }
}

impl std::error::Error for UnsentLocalChange {}

/// §5.6 safe default: no thumbnail is fetched while the item's held write is queued. The
/// server holds only the previous version's, and the system would cache it under the token.
#[derive(Debug)]
#[cfg_attr(not(any(unix, test)), allow(dead_code))]
pub(crate) struct ThumbnailNotYet;

impl std::fmt::Display for ThumbnailNotYet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the thumbnail is available once the save has uploaded")
    }
}

impl std::error::Error for ThumbnailNotYet {}

/// Spec §11 M5: an id taken from the wire is logged only when it parses as a UUID. Anything
/// else (on Linux, where the identifier check is a no-op, any caller-supplied text) is logged as
/// its kind. Log hygiene on every platform, not a behaviour change.
pub(crate) fn loggable_id(id: &str) -> &str {
    if uuid::Uuid::parse_str(id).is_ok() {
        id
    } else {
        NON_UUID
    }
}

/// Spec §11: the fixed category of an error, for a log line about a step that
/// failed. The error's text can carry a path (an I/O error's context, a database file name),
/// so a line logs this instead. The first cause it recognises decides. The category comes from
/// closed codes only (SQLite's result code, the I/O error kind), never from a message, and it
/// tells apart the causes that have different answers: a busy database, a full disk, a corrupt
/// file, and a statement the schema does not match (a defect of ours).
pub(crate) fn error_category(error: &(dyn std::error::Error + 'static)) -> &'static str {
    let mut cause = Some(error);
    while let Some(current) = cause {
        if let Some(database) = current.downcast_ref::<rusqlite::Error>() {
            return database_error_category(database);
        }
        if let Some(io) = current.downcast_ref::<std::io::Error>() {
            return match io.kind() {
                std::io::ErrorKind::NotFound => "not_found",
                std::io::ErrorKind::PermissionDenied => "permission_denied",
                std::io::ErrorKind::StorageFull | std::io::ErrorKind::QuotaExceeded => "disk_full",
                std::io::ErrorKind::ReadOnlyFilesystem => "read_only",
                _ => "io",
            };
        }
        if current.downcast_ref::<reqwest::Error>().is_some() {
            return "network";
        }
        // Task 2173: a direct download's authorization answer, in the categories above.
        if let Some(crate::direct_download::DirectError::Refused(status)) =
            current.downcast_ref::<crate::direct_download::DirectError>()
        {
            return match status {
                401 | 403 => "permission_denied",
                404 => "not_found",
                _ => "other",
            };
        }
        cause = current.source();
    }
    "other"
}

/// [`error_category`] for a database error, from rusqlite's variant and SQLite's result code.
pub(super) fn database_error_category(error: &rusqlite::Error) -> &'static str {
    use rusqlite::ffi::ErrorCode;
    match error {
        rusqlite::Error::SqliteFailure(failure, _) => match failure.code {
            ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked => "database_busy",
            ErrorCode::DiskFull => "database_full",
            ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase => "database_corrupt",
            ErrorCode::ReadOnly => "database_read_only",
            ErrorCode::CannotOpen => "database_cannot_open",
            ErrorCode::SystemIoFailure => "database_io",
            ErrorCode::ConstraintViolation => "database_constraint",
            // The generic SQLITE_ERROR: a statement that names a table or column the schema
            // does not have.
            ErrorCode::Unknown if failure.extended_code & 0xff == rusqlite::ffi::SQLITE_ERROR => "database_schema",
            _ => "database",
        },
        // The code and the schema disagree about a column, a value or a statement. A statement
        // SQLite rejects at prepare (a column it does not have, a syntax error) is
        // `SqlInputError`, which this crate's `bundled` build always has.
        rusqlite::Error::SqlInputError { .. }
        | rusqlite::Error::InvalidColumnType(..)
        | rusqlite::Error::InvalidColumnIndex(_)
        | rusqlite::Error::InvalidColumnName(_)
        | rusqlite::Error::FromSqlConversionFailure(..)
        | rusqlite::Error::IntegralValueOutOfRange(..)
        | rusqlite::Error::InvalidParameterCount(..)
        | rusqlite::Error::InvalidParameterName(_)
        | rusqlite::Error::MultipleStatement
        | rusqlite::Error::ExecuteReturnedResults => "database_schema",
        _ => "database",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationFailureClass {
    Retryable,
    Auth,
    Quota,
    Permission,
    Locked,
}

impl OperationFailureClass {
    pub(super) fn pause_reason(self) -> Option<OperationPauseReason> {
        match self {
            OperationFailureClass::Retryable => None,
            OperationFailureClass::Auth => Some(OperationPauseReason::Auth),
            OperationFailureClass::Quota => Some(OperationPauseReason::Quota),
            OperationFailureClass::Permission => Some(OperationPauseReason::Permission),
            OperationFailureClass::Locked => Some(OperationPauseReason::Locked),
        }
    }
}

/// Spec §8.4: does waiting cure this failed attempt of a File Provider write? A transient one
/// counts for the backoff but never uses up the write's attempts; any other uses them up and the
/// write parks at the cap, presented read-only, so it never shows `uploading` for good while
/// later saves wait behind it. Decided by the error's type and kind, never by its text:
/// - `Locked` (the vault, or SQLite busy) and `Quota`: transient;
/// - `Retryable` with an HTTP status: transient for 408, 425, 429 and every 5xx, counted for any
///   other (400, 404, 410, 413, 422, a 409 that is not a stale base, ...);
/// - `Retryable` whose resumed upload session the server reaped (404, 410) and the attempt
///   abandoned ([`SessionAbandoned`]): transient, because the next attempt starts a fresh upload;
/// - `Retryable` with no HTTP status: transient only when the chain carries a transport failure
///   (see [`has_transport_cause`]); a local error is counted;
/// - `Auth` and `Permission` never reach it (they pause and park): `false`.
pub(super) fn finder_write_failure_is_transient(class: OperationFailureClass, error: &anyhow::Error) -> bool {
    match class {
        OperationFailureClass::Locked | OperationFailureClass::Quota => true,
        // A gone session, abandoned: the next attempt starts a fresh upload.
        OperationFailureClass::Retryable if error.chain().any(|cause| cause.is::<SessionAbandoned>()) => true,
        OperationFailureClass::Retryable => match error_http_status(error) {
            Some(status) => matches!(status, 408 | 425 | 429 | 500..=599),
            None => has_transport_cause(error),
        },
        OperationFailureClass::Auth | OperationFailureClass::Permission => false,
    }
}

/// A transport failure in `error`'s chain: a `reqwest::Error` that never got a response (it could
/// not connect, timed out, or failed while sending), or, below a `reqwest::Error`, an I/O error of a
/// kind the network path gives (a refused, reset, aborted or broken connection, a timeout, an
/// early end). The same I/O kinds with no request above them come from a local operation and are
/// not a transport failure.
pub(super) fn has_transport_cause(error: &anyhow::Error) -> bool {
    let mut on_the_network_path = false;
    for cause in error.chain() {
        if let Some(request) = cause.downcast_ref::<reqwest::Error>() {
            if request.status().is_none() && (request.is_connect() || request.is_timeout() || request.is_request()) {
                return true;
            }
            on_the_network_path = true;
        }
        if on_the_network_path
            && let Some(io) = cause.downcast_ref::<std::io::Error>()
            && matches!(
                io.kind(),
                std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::NotConnected
                    | std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::UnexpectedEof
            )
        {
            return true;
        }
    }
    false
}

/// Spec §8.4: a refusal the person cures by freeing space, subscribing or paying, read from the
/// server's code by type, never from text, at init, a chunk or complete: 413 `quota_exceeded`, 403
/// `object_budget_exceeded`, 403 `billing_read_only`, and the upload gate's 409s
/// (`account_lapsed`, `plan_required`, `trial_cancelled_read_only`). For a File Provider write it is
/// a quota refusal: never paused, never parked, so the save lands once it is cured; the app's
/// account-state banner, not the file, says why. Any other 413, 403 or 409 keeps its rule.
pub(super) fn is_curable_refusal(error: &anyhow::Error) -> bool {
    use crate::api_client::{InitConflict, UploadRefusalCode as Code, UploadRefused};
    let upload_gate = |code: Option<Code>| {
        matches!(
            code,
            Some(Code::AccountLapsed | Code::PlanRequired | Code::TrialCancelledReadOnly)
        )
    };
    error.chain().any(|cause| {
        cause
            .downcast_ref::<UploadRefused>()
            .is_some_and(|refused| match (refused.status(), refused.code) {
                (Some(413), Some(Code::QuotaExceeded)) => true,
                (Some(403), Some(Code::ObjectBudgetExceeded | Code::BillingReadOnly)) => true,
                (Some(409), code) => upload_gate(code),
                _ => false,
            })
            || cause
                .downcast_ref::<InitConflict>()
                .is_some_and(|conflict| upload_gate(conflict.refusal))
    })
}

/// The accept found its staged copy's journal row gone: an engine-start sweep removed the copy
/// between the copy and the accept's transaction (spec §8.7 S6). Nothing was queued; the reply is
/// `WriteRetryLater`, so the system hands the write over again. Path-free on purpose: the text
/// reaches the extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(crate) struct StagedCopyGone;

impl std::fmt::Display for StagedCopyGone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the staged copy of this write was removed before it was queued")
    }
}

impl std::error::Error for StagedCopyGone {}

/// A sign-out closed Finder accepts after its engine stop (spec 2026-10-10 §15 question 3). Nothing was queued; the
/// reply is `WriteRetryLater`, so macOS keeps the change and retries. Path-free: the text reaches the extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AcceptsClosed;

impl std::fmt::Display for AcceptsClosed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Beebeeb is signing out; this write was not queued")
    }
}

impl std::error::Error for AcceptsClosed {}

/// A content save of an item in the trash (spec §7.2): its row is `Trashing`, or the system names
/// the trash container as its parent. Refused as a create in the trash is, so the item stays in
/// the trash; nothing was queued. Path-free: the text reaches the extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(crate) struct ItemInTrash;

impl std::fmt::Display for ItemInTrash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Beebeeb cannot change items inside the Trash.")
    }
}

impl std::error::Error for ItemInTrash {}

/// The attempt resumed its persisted upload session, found it reaped (the server answered 404 or
/// 410) and abandoned it: the next attempt starts a fresh upload, so for a File Provider write the
/// failure is transient, never a counted one (spec §8.4). A 400 at any session, and a 404 or 410 at a
/// session opened in the same attempt, abandon the session too, but stay counted. It shows, and has
/// as its source, the failure it wraps, so every reading of the error's text and status stays as it
/// was.
#[derive(Debug)]
pub(super) struct SessionAbandoned(anyhow::Error);

impl std::fmt::Display for SessionAbandoned {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for SessionAbandoned {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        let inner: &(dyn std::error::Error + 'static) = self.0.as_ref();
        Some(inner)
    }
}

/// What the runner does with a failed attempt that neither parked at once nor found its op
/// gone ([`failed_attempt_rule`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FailedAttemptRule {
    /// Paused until something resumes it (only `auth` is resumed, at sign-in).
    Pause(OperationPauseReason),
    /// Parked with its bytes kept (spec §8.4).
    Park(ParkReason),
    /// Recorded for the backoff, the count moved as [`AttemptCount`] says.
    Retry(AttemptCount),
}

/// How a retried attempt moves the op's attempt count ([`next_attempt_count`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AttemptCount {
    /// One more: at `max_attempts` the op has parked by the queue's rule.
    UsesUp,
    /// One more, but never past `max_attempts - 1` (a recorded completion, §8.6 rule 6).
    ShortOfTheCap,
    /// One more, but never past the attempt where the backoff stops growing
    /// ([`backoff_cap_attempts`]), and never past `max_attempts - 1`; a count already there or
    /// above stays (a transient failure of a File Provider write, §8.4). The backoff still reaches
    /// its cap, and an outage leaves the remaining attempts to the failures waiting does not cure.
    UpToTheBackoffCap,
}

/// The count an attempt records ([`AttemptCount`]).
pub(super) fn next_attempt_count(attempts: i64, max_attempts: i64, count: AttemptCount) -> i64 {
    let below_the_cap = max_attempts.saturating_sub(1);
    match count {
        AttemptCount::UsesUp => attempts.saturating_add(1),
        AttemptCount::ShortOfTheCap => attempts.saturating_add(1).min(below_the_cap),
        AttemptCount::UpToTheBackoffCap => {
            let limit = backoff_cap_attempts().min(below_the_cap);
            if attempts >= limit {
                attempts
            } else {
                attempts.saturating_add(1)
            }
        }
    }
}

/// The rule for a failed attempt of class `class`.
///
/// - A recorded completion (`completed`, §8.6 rule 6) only ever retries, and never uses up its
///   attempts.
/// - A File Provider write (`finder_write`: an upload with a write id) never stops uploading for
///   a reason that waiting or the person can cure (§8.4): a `transient` failure
///   ([`finder_write_failure_is_transient`]: the network, a 5xx, 408/425/429, a lock, a quota
///   refusal) retries on the normal backoff without using up the attempts, so the save lands once
///   the network, the lock or the space comes back; any other `Retryable` failure uses them up and
///   parks the write at the cap; a permission refusal parks it at once as `permission_denied`,
///   which presents it read-only; `auth` pauses it until the next sign-in.
/// - Every other upload keeps the rules it had: `Retryable` uses up its attempts, every other
///   class pauses it.
pub(super) fn failed_attempt_rule(
    class: OperationFailureClass,
    finder_write: bool,
    completed: bool,
    transient: bool,
) -> FailedAttemptRule {
    if completed {
        return FailedAttemptRule::Retry(AttemptCount::ShortOfTheCap);
    }
    if !finder_write {
        return match class.pause_reason() {
            Some(reason) => FailedAttemptRule::Pause(reason),
            None => FailedAttemptRule::Retry(AttemptCount::UsesUp),
        };
    }
    match class {
        OperationFailureClass::Auth => FailedAttemptRule::Pause(OperationPauseReason::Auth),
        OperationFailureClass::Permission => FailedAttemptRule::Park(ParkReason::PermissionDenied),
        OperationFailureClass::Retryable | OperationFailureClass::Locked | OperationFailureClass::Quota => {
            FailedAttemptRule::Retry(if transient {
                AttemptCount::UpToTheBackoffCap
            } else {
                AttemptCount::UsesUp
            })
        }
    }
}

/// The HTTP status of the request that failed somewhere in `error`'s chain,
/// if it was an HTTP error. An `init` refused with 409 carries its class instead
/// of reqwest's error (spec §8.4), and still answers 409 here.
pub(crate) fn error_http_status(error: &anyhow::Error) -> Option<u16> {
    if init_conflict_class(error).is_some() {
        return Some(409);
    }
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<reqwest::Error>())
        .and_then(reqwest::Error::status)
        .map(|status| status.as_u16())
}

/// A guarded queue write matched no row: the op moved since its claim (spec §8.7 S2, S4).
#[derive(Debug)]
pub(crate) struct QueueStateMoved;

impl std::fmt::Display for QueueStateMoved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("queue state moved")
    }
}

impl std::error::Error for QueueStateMoved {}

/// Park the claimed op now, with its bytes kept (spec §8.4, §6.3.1).
#[derive(Debug)]
pub(crate) struct ParkNow(pub ParkReason);

impl std::fmt::Display for ParkNow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "upload parked: {}", self.0.as_str())
    }
}

impl std::error::Error for ParkNow {}

/// A Finder write could not be staged: the staging folder cannot be created or written to, and
/// on macOS nothing falls back to a folder the system may purge (spec §8.4). The reply is
/// `WriteRetryLater`, which the extension reports as a transient error, so the system keeps the
/// change and retries. Path-free on purpose: the text reaches the extension and the person.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(crate) struct FinderStagingUnavailable(Option<std::io::ErrorKind>);

impl std::fmt::Display for FinderStagingUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Some(kind) => write!(f, "the staging folder is unavailable: {kind}"),
            None => f.write_str("the staging folder is unavailable: no data directory"),
        }
    }
}

impl std::error::Error for FinderStagingUnavailable {}

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

/// The attempt count at which [`retry_delay_seconds`] stops growing: the most a transient failure
/// raises a File Provider write's count to (spec §8.4). Derived from the backoff, so changing the
/// backoff moves it too.
pub(super) fn backoff_cap_attempts() -> i64 {
    /// Where the search stops if the backoff ever had no cap; the backoff reaches its cap at 6 today.
    const SEARCH_LIMIT: i64 = 64;
    let cap = retry_delay_seconds(i64::MAX);
    let found = (1..=SEARCH_LIMIT).find(|&attempts| retry_delay_seconds(attempts) >= cap);
    debug_assert!(
        found.is_some(),
        "retry_delay_seconds reaches its cap within {SEARCH_LIMIT} attempts"
    );
    found.unwrap_or(SEARCH_LIMIT)
}

pub fn retry_delay_seconds(attempts: i64) -> i64 {
    let exponent = attempts.clamp(1, 6) as u32;
    30_i64.saturating_mul(2_i64.saturating_pow(exponent - 1))
}
