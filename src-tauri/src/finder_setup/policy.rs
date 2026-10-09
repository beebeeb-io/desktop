//! Spec §6.2 (classification) and §7 (retry policy). The schedule and the clock live here
//! and nowhere else; the clock itself is injected by the caller (`started`, `now`).

use std::time::{Duration, Instant};

use super::error::{
    APP_DOMAIN, BRIDGE_DOMAIN, COCOA_DOMAIN, FILE_PROVIDER_DOMAIN, FpError, POSIX_DOMAIN, app_code, bridge_code,
};
use crate::surfaces::phase::FinderFailureReason;

/// `NSFileProviderErrorDomain` codes, macOS 27 SDK `NSFileProviderError.h` (verified by the
/// lead on the build Mac, 2026-10-06).
pub mod fp_code {
    pub const PROVIDER_NOT_FOUND: i64 = -2001;
    pub const PROVIDER_TRANSLOCATED: i64 = -2002;
    pub const OLDER_EXTENSION_VERSION_RUNNING: i64 = -2003;
    pub const NEWER_EXTENSION_VERSION_FOUND: i64 = -2004;
    // Verified codes with no §6.2 row (Spec issue 10): named so the list is complete, and used by tests.
    #[allow(dead_code)]
    pub const CANNOT_SYNCHRONIZE: i64 = -2005;
    pub const DOMAIN_DISABLED: i64 = -2011;
    pub const PROVIDER_DOMAIN_TEMPORARILY_UNAVAILABLE: i64 = -2012;
    #[allow(dead_code)]
    pub const PROVIDER_DOMAIN_NOT_FOUND: i64 = -2013;
    pub const APPLICATION_EXTENSION_NOT_FOUND: i64 = -2014;
}

/// `NSFileWriteFileExistsError`. PROVISIONAL until device check D0 confirms it (plan "Spec
/// issues" 5): the 0.8.11 app saved exactly this for the orphan collision on a developer Mac
/// ("…a file with the same name already exists. (NSCocoaErrorDomain 516)", 2026-10-06
/// 13:05:09Z). D0 also records the underlying error, which may be the POSIX EEXIST below.
pub const COCOA_FILE_WRITE_FILE_EXISTS: i64 = 516;
pub const POSIX_EEXIST: i64 = 17;

/// `signing` (§6.2): entitlement, app-group or provisioning errors. EMPTY on purpose: the
/// bridge has no provisioning-specific path (plan "Spec issues" 6), so no code can be listed
/// from code. A captured code is added here together with its row in
/// `classify_covers_every_row_of_the_table`.
pub const SIGNING_CODES: &[(&str, i64)] = &[];

pub fn classify(error: &FpError) -> FinderFailureReason {
    use FinderFailureReason::*;
    if error.domain == FILE_PROVIDER_DOMAIN {
        match error.code {
            fp_code::PROVIDER_NOT_FOUND
            | fp_code::OLDER_EXTENSION_VERSION_RUNNING
            | fp_code::NEWER_EXTENSION_VERSION_FOUND
            | fp_code::PROVIDER_DOMAIN_TEMPORARILY_UNAVAILABLE
            | fp_code::APPLICATION_EXTENSION_NOT_FOUND => return ExtensionLoading,
            fp_code::DOMAIN_DISABLED => return UserDisabled,
            fp_code::PROVIDER_TRANSLOCATED => return NotInApplications,
            _ => {}
        }
    }
    let is = |domain: &str, code: i64| {
        (error.domain == domain && error.code == code)
            || error
                .underlying
                .as_ref()
                .is_some_and(|u| u.domain == domain && u.code == code)
    };
    if is(COCOA_DOMAIN, COCOA_FILE_WRITE_FILE_EXISTS) || is(POSIX_DOMAIN, POSIX_EEXIST) {
        return FolderTaken;
    }
    if SIGNING_CODES.iter().any(|(domain, code)| is(domain, *code)) {
        return Signing;
    }
    if error.domain == BRIDGE_DOMAIN && error.code == bridge_code::STABILIZATION_TIMEOUT {
        return Timeout;
    }
    // A port call that never returned (`driver::within`): the same plain "did not finish".
    if error.domain == APP_DOMAIN && error.code == app_code::OP_TIMEOUT {
        return Timeout;
    }
    Unknown
}

/// One attempt, for reasons that are never retried.
const ONE_ATTEMPT: [Duration; 1] = [Duration::ZERO];

/// Every attempt start, as an offset from the check's start (§7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryPolicy {
    /// `extension_loading`, `timeout`: 0, 5, 15, 45 s.
    pub transient: Vec<Duration>,
    /// `unknown`: 0, 5 s (one silent retry).
    pub once: Vec<Duration>,
    /// How long one attempt waits for a freshly added domain to stabilize (≤ 10 s).
    pub stabilize_after_add: Duration,
    /// How long an existing registration gets to confirm it is stable (§5.5 step 3).
    pub confirm_existing: Duration,
    /// The read-only `userEnabled` poll while Beebeeb is turned off in System Settings.
    pub user_disabled_poll: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        let s = Duration::from_secs;
        Self {
            transient: vec![s(0), s(5), s(15), s(45)],
            once: vec![s(0), s(5)],
            stabilize_after_add: s(10),
            confirm_existing: s(2),
            user_disabled_poll: s(3),
        }
    }
}

impl RetryPolicy {
    pub fn offsets(&self, reason: FinderFailureReason) -> &[Duration] {
        use FinderFailureReason::*;
        match reason {
            ExtensionLoading | Timeout => &self.transient,
            Unknown => &self.once,
            UserDisabled | NotInApplications | FolderTaken | Signing => &ONE_ATTEMPT,
        }
    }

    pub fn max_attempts(&self, reason: FinderFailureReason) -> u8 {
        u8::try_from(self.offsets(reason).len()).unwrap_or(u8::MAX)
    }

    /// When attempt number `attempts_made + 1` may start, or `None` once the budget for
    /// `reason` is spent. An attempt that is still running at that moment simply starts the
    /// next one late; the caller takes `max(at, now)`.
    pub fn next_attempt_at(&self, reason: FinderFailureReason, attempts_made: u8, started: Instant) -> Option<Instant> {
        self.offsets(reason)
            .get(usize::from(attempts_made))
            .map(|offset| started + *offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finder_setup::error::{APP_DOMAIN, app_code};
    use FinderFailureReason::*;

    fn e(domain: &str, code: i64) -> FpError {
        FpError::new(domain, code, "fixture message")
    }

    /// Spec §6.2, row by row: each entry is one row's macOS signal.
    #[test]
    fn classify_covers_every_row_of_the_table() {
        let rows: Vec<(&str, FpError, FinderFailureReason)> = vec![
            (
                "-2001 ProviderNotFound",
                e(FILE_PROVIDER_DOMAIN, -2001),
                ExtensionLoading,
            ),
            (
                "-2003 OlderExtensionVersionRunning",
                e(FILE_PROVIDER_DOMAIN, -2003),
                ExtensionLoading,
            ),
            (
                "-2004 NewerExtensionVersionFound",
                e(FILE_PROVIDER_DOMAIN, -2004),
                ExtensionLoading,
            ),
            (
                "-2012 ProviderDomainTemporarilyUnavailable",
                e(FILE_PROVIDER_DOMAIN, -2012),
                ExtensionLoading,
            ),
            (
                "-2014 ApplicationExtensionNotFound",
                e(FILE_PROVIDER_DOMAIN, -2014),
                ExtensionLoading,
            ),
            ("-2011 DomainDisabled", e(FILE_PROVIDER_DOMAIN, -2011), UserDisabled),
            (
                "-2002 ProviderTranslocated",
                e(FILE_PROVIDER_DOMAIN, -2002),
                NotInApplications,
            ),
            (
                "folder taken, top level (0.8.11 evidence)",
                e(COCOA_DOMAIN, 516),
                FolderTaken,
            ),
            (
                "folder taken, underlying EEXIST",
                e("SomeWrapperDomain", 1).with_underlying(POSIX_DOMAIN, 17),
                FolderTaken,
            ),
            (
                "stabilization timeout",
                e(BRIDGE_DOMAIN, bridge_code::STABILIZATION_TIMEOUT),
                Timeout,
            ),
            // Task 8, lead ruling T7-1: a port call that never returned is cut off by the driver's
            // per-operation limit. macOS did not finish, which is what `timeout` says.
            (
                "an operation outran its limit",
                e(APP_DOMAIN, app_code::OP_TIMEOUT),
                Timeout,
            ),
        ];
        for (name, error, expected) in &rows {
            assert_eq!(classify(error), *expected, "{name}");
        }
    }

    #[test]
    fn everything_else_is_unknown() {
        for error in [
            e(FILE_PROVIDER_DOMAIN, -2005), // CannotSynchronize: verified code, no §6.2 row (Spec issue 10)
            e(FILE_PROVIDER_DOMAIN, -2013), // ProviderDomainNotFound: same
            e(FILE_PROVIDER_DOMAIN, -1000),
            e(COCOA_DOMAIN, 4099),
            e(POSIX_DOMAIN, 13),
            e(BRIDGE_DOMAIN, bridge_code::MANAGER_UNAVAILABLE),
            e(APP_DOMAIN, app_code::ENGINE_START),
            e("", 0),
            // A matching code under the wrong domain is not a match: each domain guard in
            // `classify` has a row here that fails without it.
            e(APP_DOMAIN, app_code::FINISH_READY), // same code as bridge STABILIZATION_TIMEOUT (2): our own failure, not a timeout
            e(COCOA_DOMAIN, -2001),                // a FileProvider code under the Cocoa domain
            e(POSIX_DOMAIN, -2011),                // a FileProvider code under the POSIX domain
            e(COCOA_DOMAIN, 17),                   // the POSIX EEXIST code under the Cocoa domain
            e(POSIX_DOMAIN, 516),                  // the Cocoa file-exists code under the POSIX domain
            e(BRIDGE_DOMAIN, 516),                 // the Cocoa file-exists code under the bridge domain
            e(BRIDGE_DOMAIN, app_code::OP_TIMEOUT), // the operation-limit code under the bridge domain: the domain guard
            // The same rule for the underlying error: its domain is compared too.
            e("SomeWrapperDomain", 1).with_underlying(COCOA_DOMAIN, 17), // POSIX EEXIST code, Cocoa domain
            e("SomeWrapperDomain", 1).with_underlying(POSIX_DOMAIN, 516), // Cocoa file-exists code, POSIX domain
        ] {
            assert_eq!(classify(&error), Unknown, "{error:?}");
        }
    }

    #[test]
    fn the_message_is_never_read() {
        // The old macOS classifier matched English fragments of our own copy. The same domain +
        // code with any message must classify the same.
        for message in [
            "Provisioning profile entitlement app-group mismatch",
            "Timed out waiting",
            "Beebeeb is turned off in System Settings",
            "",
        ] {
            assert_eq!(classify(&FpError::new("SomeDomain", 7, message)), Unknown, "{message}");
            assert_eq!(
                classify(&FpError::new(FILE_PROVIDER_DOMAIN, -2001, message)),
                ExtensionLoading,
                "{message}"
            );
        }
    }

    #[test]
    fn signing_has_no_code_until_one_is_captured() {
        assert!(
            SIGNING_CODES.is_empty(),
            "add the captured code's row to classify_covers_every_row_of_the_table"
        );
        for code in -2020..=-2000 {
            assert_ne!(classify(&e(FILE_PROVIDER_DOMAIN, code)), Signing, "{code}");
        }
    }

    #[test]
    fn retry_classes_follow_section_7() {
        let policy = RetryPolicy::default();
        let secs = |reason| policy.offsets(reason).iter().map(|d| d.as_secs()).collect::<Vec<_>>();
        assert_eq!(secs(ExtensionLoading), vec![0, 5, 15, 45]);
        assert_eq!(secs(Timeout), vec![0, 5, 15, 45]);
        assert_eq!(secs(Unknown), vec![0, 5]);
        for reason in [FolderTaken, Signing, NotInApplications, UserDisabled] {
            assert_eq!(secs(reason), vec![0], "{reason:?}");
        }
        assert_eq!(policy.max_attempts(ExtensionLoading), 4);
        assert_eq!(policy.max_attempts(Unknown), 2);
        assert_eq!(policy.max_attempts(FolderTaken), 1);
        assert_eq!(policy.stabilize_after_add, Duration::from_secs(10));
        assert_eq!(policy.confirm_existing, Duration::from_secs(2));
        assert_eq!(policy.user_disabled_poll, Duration::from_secs(3));
    }

    #[test]
    fn next_attempt_at_counts_from_the_check_start_and_ends_with_the_budget() {
        let policy = RetryPolicy::default();
        let t0 = Instant::now();
        let at = |reason: FinderFailureReason, made: u8| {
            policy.next_attempt_at(reason, made, t0).map(|i| (i - t0).as_secs())
        };
        assert_eq!(
            (1u8..=4).map(|made| at(Timeout, made)).collect::<Vec<_>>(),
            vec![Some(5), Some(15), Some(45), None]
        );
        assert_eq!(at(Unknown, 1), Some(5));
        assert_eq!(at(Unknown, 2), None);
        assert_eq!(at(FolderTaken, 1), None);
    }
}
