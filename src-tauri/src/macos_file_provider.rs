use std::ffi::CStr;
use std::os::raw::c_char;

unsafe extern "C" {
    fn beebeeb_fp_status(error_buffer: *mut c_char, error_buffer_len: usize) -> i32;
    fn beebeeb_fp_visible_url(
        url_buffer: *mut c_char,
        url_buffer_len: usize,
        error_buffer: *mut c_char,
        error_buffer_len: usize,
    ) -> i32;
    fn beebeeb_fp_domain_exists(error_buffer: *mut c_char, error_buffer_len: usize) -> i32;
    fn beebeeb_fp_domain_user_enabled(error_buffer: *mut c_char, error_buffer_len: usize) -> i32;
    fn beebeeb_fp_add_domain(error_buffer: *mut c_char, error_buffer_len: usize) -> i32;
    fn beebeeb_fp_wait_for_domain_ready(
        timeout_seconds: f64,
        error_buffer: *mut c_char,
        error_buffer_len: usize,
    ) -> i32;
    fn beebeeb_fp_remove(
        location_buffer: *mut c_char,
        location_buffer_len: usize,
        kept_state: *mut i32,
        error_buffer: *mut c_char,
        error_buffer_len: usize,
    ) -> i32;
}

/// Room for the folder macOS reports after a removal that kept files (task 1882). File-system
/// paths on macOS are limited to `PATH_MAX` (1024) bytes, well under this.
const PRESERVED_LOCATION_BUFFER_LEN: usize = 4096;

/// How long `install()` waits for a freshly (re-)added domain to stabilize before
/// giving up with a real timeout. Unchanged from the pre-1524-issue-4 behavior.
const INSTALL_STABILIZATION_TIMEOUT_SECONDS: f64 = 10.0;

/// Result of an `install()` attempt that did not error outright.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallOutcome {
    /// The domain was added (or already existed) and stabilized normally.
    Installed,
    /// The domain exists but is disabled by the user in System Settings -- installing
    /// it further is pointless until the user re-enables it there. This is Issue 4:
    /// `addDomain` reports success and `waitForStabilization` never fires, which used
    /// to surface as a generic, misleading 10s timeout.
    UserDisabled,
}

/// Tri-state read of `NSFileProviderDomain.userEnabled` for the Beebeeb domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomainUserEnabledState {
    Enabled,
    Disabled,
    /// The domain is not registered with the system yet (a fresh install, or after a
    /// clean `removeDomain`). Not itself evidence of anything being wrong.
    NotRegistered,
}

/// What `install()` should do next, given a `userEnabled` lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallDecision {
    /// Proceed with the normal add-domain-then-wait-for-stabilization flow.
    Proceed,
    /// Short-circuit: report `UserDisabled` without waiting for stabilization.
    UserDisabled,
}

/// Pure decision core for Issue 4, used both BEFORE `addDomain` (when the domain
/// already exists) and AFTER it (before waiting for stabilization) -- the exact same
/// rule applies at both call sites, so there is exactly one place this branches:
///
/// - `Enabled` or `NotRegistered` -> [`InstallDecision::Proceed`]: either the domain is
///   fine, or it doesn't exist yet (the normal case for a fresh install, where
///   `addDomain` still needs to run) -- both are the ordinary wait path.
/// - `Disabled` -> [`InstallDecision::UserDisabled`], no wait: a user-disabled domain
///   never launches its extension, so `waitForStabilizationWithCompletionHandler`'s
///   completion block would never fire and the caller would otherwise always burn the
///   full timeout for a state we already know about.
/// - A lookup `Err` -> [`InstallDecision::Proceed`]: never invent a `UserDisabled`
///   verdict from an unreliable signal -- fall back to the pre-existing wait/timeout
///   behavior, which is what shipped before this fix and is still correct on error.
pub fn decide_install_step(lookup: &Result<DomainUserEnabledState, String>) -> InstallDecision {
    match lookup {
        Ok(DomainUserEnabledState::Disabled) => InstallDecision::UserDisabled,
        Ok(DomainUserEnabledState::Enabled) | Ok(DomainUserEnabledState::NotRegistered) | Err(_) => {
            InstallDecision::Proceed
        }
    }
}

/// Result of a [`status()`] check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusOutcome {
    /// The domain is registered, enabled, and stabilized -- Finder integration is live.
    Installed,
    /// No Beebeeb domain is registered at all (a fresh install, or after a clean
    /// `removeDomain`). Not an error.
    NotInstalled,
    /// The domain is registered but the user (or macOS) has disabled it in System
    /// Settings -- Issue 4's condition (task 1524), detected WITHOUT waiting for
    /// stabilization, which would never complete for a disabled domain.
    UserDisabled,
}

/// Pure orchestration for [`status()`], with the live `beebeeb_fp_status` FFI call
/// injected as `live_status` so this is unit-testable without a live
/// `NSFileProviderManager` (task 1524 Issue 4, status-path leg -- PR #63 Codex review,
/// `Onboarding.tsx:488`).
///
/// Uses the SAME pure decision ([`decide_install_step`]) as `install()`: a disabled
/// domain short-circuits to [`StatusOutcome::UserDisabled`] WITHOUT calling
/// `live_status` at all. That call is exactly the 2s
/// `waitForStabilizationWithCompletionHandler` that used to always time out for a
/// disabled domain (`beebeeb_fp_status`'s old unconditional wait) -- surfacing a
/// fresh, generic "timed out" runtime error that `finder_install_state_from_config`
/// (lib.rs) then preferred over any persisted `user_disabled` state, so reloading the
/// onboarding card after leaving and coming back showed the stale timeout copy
/// instead of the disabled-domain card. Short-circuiting here means the runtime error
/// `status()` can still produce is only ever a GENUINE stabilization timeout (the
/// domain is enabled but never came up) -- see [`decide_install_step`]'s own doc
/// comment for why `Enabled`/`NotRegistered`/lookup-`Err` all fall through to the
/// normal wait/timeout path.
fn status_outcome_from(
    user_enabled: &Result<DomainUserEnabledState, String>,
    live_status: impl FnOnce() -> Result<bool, String>,
) -> Result<StatusOutcome, String> {
    if decide_install_step(user_enabled) == InstallDecision::UserDisabled {
        return Ok(StatusOutcome::UserDisabled);
    }
    Ok(if live_status()? {
        StatusOutcome::Installed
    } else {
        StatusOutcome::NotInstalled
    })
}

pub fn status() -> Result<StatusOutcome, String> {
    status_outcome_from(&domain_user_enabled_state(), || Ok(call_bridge(beebeeb_fp_status)? == 1))
}

pub fn visible_url() -> Result<Option<String>, String> {
    let mut url_buffer = [0i8; 2048];
    let mut error_buffer = [0i8; 1024];
    let code = unsafe {
        beebeeb_fp_visible_url(
            url_buffer.as_mut_ptr(),
            url_buffer.len(),
            error_buffer.as_mut_ptr(),
            error_buffer.len(),
        )
    };
    if code < 0 {
        return Err(
            buffer_to_string(&error_buffer).unwrap_or_else(|| "Resolve File Provider location failed".to_string())
        );
    }
    if code == 0 {
        return Ok(None);
    }
    Ok(buffer_to_string(&url_buffer))
}

/// Whether the Beebeeb domain is currently registered with the system.
fn domain_exists() -> Result<bool, String> {
    Ok(call_bridge(beebeeb_fp_domain_exists)? == 1)
}

/// Read `NSFileProviderDomain.userEnabled` for the Beebeeb domain via the OS's own
/// live registry (`getDomainsWithCompletionHandler`), not a locally-constructed
/// `NSFileProviderDomain` object (which carries no system state).
fn domain_user_enabled_state() -> Result<DomainUserEnabledState, String> {
    let mut error_buffer = [0i8; 1024];
    let code = unsafe { beebeeb_fp_domain_user_enabled(error_buffer.as_mut_ptr(), error_buffer.len()) };
    match code {
        1 => Ok(DomainUserEnabledState::Enabled),
        0 => Ok(DomainUserEnabledState::Disabled),
        2 => Ok(DomainUserEnabledState::NotRegistered),
        _ => Err(buffer_to_string(&error_buffer)
            .unwrap_or_else(|| "look up File Provider domain state failed".to_string())),
    }
}

/// Public, frontend-facing tri-state read of whether the Beebeeb domain is currently
/// user-enabled. Used by the "Beebeeb is turned off in System Settings" card to poll
/// while it is shown, so it can continue installation automatically once the user
/// flips System Settings back on -- without re-attempting `addDomain` on every poll.
pub fn domain_user_enabled() -> Result<DomainUserEnabledState, String> {
    domain_user_enabled_state()
}

/// Add (or update) the Beebeeb File Provider domain and wait for it to come up,
/// short-circuiting with [`InstallOutcome::UserDisabled`] instead of waiting when
/// Issue 4's condition is detected -- either because the domain already existed and
/// was disabled before this call, or because it is disabled immediately after
/// `addDomain` replies (same check, same decision, see [`decide_install_step`]).
///
/// On a genuine stabilization failure for a domain that did NOT exist before this
/// call, the just-created domain is removed again (unchanged cleanup behavior from
/// the pre-1524-issue-4 implementation) so a failed install doesn't leave an orphaned
/// domain registered.
pub fn install() -> Result<InstallOutcome, String> {
    let existed_before_add = domain_exists()?;

    if existed_before_add
        && decide_install_step(&domain_user_enabled_state()) == InstallDecision::UserDisabled
    {
        return Ok(InstallOutcome::UserDisabled);
    }

    call_bridge(beebeeb_fp_add_domain)?;

    if decide_install_step(&domain_user_enabled_state()) == InstallDecision::UserDisabled {
        // Do NOT remove the domain here: it is exactly the domain the user needs to
        // re-enable in System Settings. Removing it would strand the frontend's
        // "wait for it to be re-enabled" poll with nothing to observe.
        return Ok(InstallOutcome::UserDisabled);
    }

    let mut error_buffer = [0i8; 1024];
    let wait_code = unsafe {
        beebeeb_fp_wait_for_domain_ready(
            INSTALL_STABILIZATION_TIMEOUT_SECONDS,
            error_buffer.as_mut_ptr(),
            error_buffer.len(),
        )
    };
    if wait_code < 0 {
        let setup_error =
            buffer_to_string(&error_buffer).unwrap_or_else(|| "File Provider operation failed".to_string());
        if !existed_before_add
            && let Err(cleanup_error) = remove()
        {
            return Err(format!("{setup_error}; cleanup failed: {cleanup_error}"));
        }
        return Err(setup_error);
    }

    Ok(InstallOutcome::Installed)
}

/// Removes our Finder location, keeping the files that never reached the server (task 1882,
/// `NSFileProviderDomainRemovalModePreserveDirtyUserData`). The result says whether macOS kept
/// anything, checked on disk (round 2, spec §4), and names the folder if so.
pub fn remove() -> Result<crate::finder_removal::DomainRemoval, String> {
    let mut location_buffer = [0 as c_char; PRESERVED_LOCATION_BUFFER_LEN];
    let mut kept_state: i32 = crate::finder_removal::KEPT_NONE_REPORTED;
    let mut error_buffer = [0 as c_char; 1024];
    let code = unsafe {
        beebeeb_fp_remove(
            location_buffer.as_mut_ptr(),
            location_buffer.len(),
            &mut kept_state,
            error_buffer.as_mut_ptr(),
            error_buffer.len(),
        )
    };
    crate::finder_removal::removal_from_bridge(
        code,
        kept_state,
        buffer_to_exact_string(&location_buffer),
        buffer_to_string(&error_buffer),
    )
}

/// Result of a best-effort working-set signal (task 1697).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkingSetSignalOutcome {
    /// The system accepted the signal (or the domain is not registered —
    /// nothing to signal is not a failure; sync continues regardless).
    Delivered,
    /// The signal itself failed (timeout, NSFileProviderManager error). Logged
    /// by the caller; never blocks the sync tick that produced it.
    Failed,
}

/// Task 1698 part 3 (closes 1696, audit G8): the ONE domain we own. The
/// zombie `io.beebeeb.desktop.FileProvider` domain from the pre-rename bundle
/// id (task 1696 forensics) keeps its "Beebeeb-Drive" volume in
/// `~/Library/CloudStorage` until a SIGNED context enumerates and removes it
/// (`NSFileProviderManager.getDomains` needs app identity — an unsigned CLI
/// gets error -2001). Must stay identical to `BeebeebDomainIdentifier` in
/// `src-tauri/macos/FileProviderBridge.m`.
pub const DOMAIN_IDENTIFIER: &str = "io.beebeeb.app.domain";

/// Pure filter: which of the system's registered domains must be REMOVED?
/// Every domain whose identifier is not EXACTLY ours is stale — a zombie
/// from an older bundle id, a renamed registration, or another app's domain
/// that must never have been created in our app group context. Our own
/// identifier is never stale (the cleanup can never remove the live domain,
/// even if the system reports it twice). The comparison is exact and
/// case-sensitive: `io.beebeeb.app.domain` ≠ `IO.BEEBEEB.APP.DOMAIN`.
pub fn stale_domain_identifiers<'a>(domains: &'a [String], ours: &str) -> Vec<&'a str> {
    domains
        .iter()
        .map(String::as_str)
        .filter(|identifier| *identifier != ours)
        .collect()
}

/// What one app-start cleanup run did (logged honestly; also surfaced for
/// tests and diagnostics).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleDomainCleanup {
    /// Identifiers that were present and stale, with their removal result:
    /// Ok(()) = removed, Err(message) = the system refused (logged, NOT
    /// retried here — the next app start retries the whole sweep).
    pub removals: Vec<(String, Result<(), String>)>,
    /// Task 1882: the folders where macOS kept un-synced files of a removed
    /// domain, one per such removal. Shown to the person, never logged.
    pub preserved_locations: Vec<String>,
    /// Domains found foreign but not attempted (enumeration/other errors).
    pub skipped: Vec<String>,
    /// Our own domain was present (or not) — informational only; it is never
    /// touched.
    pub ours_present: bool,
}

impl StaleDomainCleanup {
    pub fn removed_count(&self) -> usize {
        self.removals.iter().filter(|(_, result)| result.is_ok()).count()
    }
}

#[cfg(target_os = "macos")]
mod cleanup_ffi {
    use std::ffi::CStr;
    use std::os::raw::c_char;

    unsafe extern "C" {
        fn beebeeb_fp_list_domains(
            ids_buffer: *mut c_char,
            ids_buffer_len: usize,
            error_buffer: *mut c_char,
            error_buffer_len: usize,
        ) -> i32;
        fn beebeeb_fp_remove_domain_by_id(
            identifier: *const c_char,
            location_buffer: *mut c_char,
            location_buffer_len: usize,
            kept_state: *mut i32,
            error_buffer: *mut c_char,
            error_buffer_len: usize,
        ) -> i32;
    }

    /// Enumerate every registered domain identifier via the ObjC bridge.
    /// Newline-separated in `buffer`; returns the count, or Err(message).
    pub fn list_domains() -> Result<Vec<String>, String> {
        let mut ids_buffer = [0i8; 8192];
        let mut error_buffer = [0i8; 1024];
        let count = unsafe {
            beebeeb_fp_list_domains(
                ids_buffer.as_mut_ptr(),
                ids_buffer.len(),
                error_buffer.as_mut_ptr(),
                error_buffer.len(),
            )
        };
        if count < 0 {
            return Err(super::buffer_to_string(&error_buffer)
                .unwrap_or_else(|| "enumerate File Provider domains failed".to_string()));
        }
        let raw = super::buffer_to_string(&ids_buffer).unwrap_or_default();
        Ok(raw
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect())
    }

    /// Task 1882: keeps the domain's un-synced files, like every removal.
    pub fn remove_domain(identifier: &str) -> Result<crate::finder_removal::DomainRemoval, String> {
        let mut location_buffer = [0 as c_char; super::PRESERVED_LOCATION_BUFFER_LEN];
        let mut kept_state: i32 = crate::finder_removal::KEPT_NONE_REPORTED;
        let mut error_buffer = [0i8; 1024];
        let code = unsafe {
            let c_id = std::ffi::CString::new(identifier)
                .map_err(|_| "domain identifier contained a NUL byte".to_string())?;
            beebeeb_fp_remove_domain_by_id(
                c_id.as_ptr(),
                location_buffer.as_mut_ptr(),
                location_buffer.len(),
                &mut kept_state,
                error_buffer.as_mut_ptr(),
                error_buffer.len(),
            )
        };
        crate::finder_removal::removal_from_bridge(
            code,
            kept_state,
            super::buffer_to_exact_string(&location_buffer),
            super::buffer_to_string(&error_buffer),
        )
    }
}

/// App-start stale-domain sweep (task 1698 part 3 / 1696 acceptance). Runs in
/// the SIGNED app process only — the Tauri app holds the Developer ID
/// identity the NSFileProviderManager calls require; an unsigned CLI context
/// gets -2001, which surfaces here as Err and is logged by the caller
/// WITHOUT failing anything (the engine does not depend on this).
///
/// Idempotent by construction: a second run finds no foreign domains and
/// removes nothing. Our own domain is NEVER a removal candidate (the pure
/// filter above guarantees it). Per-domain removal failures are recorded and
/// skipped — one stubborn zombie never blocks the rest or the app.
#[cfg(target_os = "macos")]
pub fn cleanup_stale_domains() -> Result<StaleDomainCleanup, String> {
    let domains = match cleanup_ffi::list_domains() {
        Ok(domains) => domains,
        Err(error) => return Err(error),
    };
    let ours_present = domains.iter().any(|identifier| identifier == DOMAIN_IDENTIFIER);
    let stale = stale_domain_identifiers(&domains, DOMAIN_IDENTIFIER);
    let mut cleanup = StaleDomainCleanup {
        removals: Vec::new(),
        preserved_locations: Vec::new(),
        skipped: Vec::new(),
        ours_present,
    };
    for identifier in stale {
        match cleanup_ffi::remove_domain(identifier) {
            Ok(removal) => {
                if let Some(location) = removal.kept_location("stale-domain sweep") {
                    cleanup.preserved_locations.push(location);
                }
                cleanup.removals.push((identifier.to_string(), Ok(())));
            }
            Err(error) => {
                tracing::warn!(identifier = %identifier, error = %error, "stale-domain removal failed; the next app start retries");
                cleanup.skipped.push(identifier.to_string());
            }
        }
    }
    Ok(cleanup)
}

/// Pure decision core for the runner's signal path: an operation batch that
/// changed items always asks for a working-set signal (Replicated honors ONLY
/// `.workingSet`); anything else never touches the bridge.
pub fn should_signal_working_set(changed_item_ids: &[String]) -> bool {
    !changed_item_ids.is_empty()
}

/// Task 1697: call the FFI bridge to signal the replica's working set after
/// the daemon applied a change batch. Best-effort by contract — a failure is
/// returned for logging and must never fail the sync tick that produced it.
pub fn signal_working_set() -> Result<WorkingSetSignalOutcome, String> {
    let mut error_buffer = [0i8; 1024];
    let code = unsafe {
        beebeeb_fp_signal_working_set(error_buffer.as_mut_ptr(), error_buffer.len())
    };
    match code {
        0 => Ok(WorkingSetSignalOutcome::Delivered),
        -1 => Err(buffer_to_string(&error_buffer)
            .unwrap_or_else(|| "signal the Beebeeb File Provider working set failed".to_string())),
        other => Err(format!("beebeeb_fp_signal_working_set returned unexpected code {other}")),
    }
}

unsafe extern "C" {
    fn beebeeb_fp_signal_working_set(error_buffer: *mut c_char, error_buffer_len: usize) -> i32;
}

fn call_bridge(function: unsafe extern "C" fn(*mut c_char, usize) -> i32) -> Result<i32, String> {
    let mut error_buffer = [0i8; 1024];
    let code = unsafe { function(error_buffer.as_mut_ptr(), error_buffer.len()) };
    if code >= 0 {
        return Ok(code);
    }
    let message = unsafe { CStr::from_ptr(error_buffer.as_ptr()) }
        .to_string_lossy()
        .trim()
        .to_string();
    Err(if message.is_empty() {
        "File Provider operation failed".to_string()
    } else {
        message
    })
}

/// Like `buffer_to_string`, but never trims: a folder the system reported is shown byte for byte
/// (task 1882). `None` when the buffer is empty.
fn buffer_to_exact_string(buffer: &[c_char]) -> Option<String> {
    let text = unsafe { CStr::from_ptr(buffer.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    if text.is_empty() { None } else { Some(text) }
}

fn buffer_to_string(buffer: &[i8]) -> Option<String> {
    let message = unsafe { CStr::from_ptr(buffer.as_ptr()) }
        .to_string_lossy()
        .trim()
        .to_string();
    if message.is_empty() { None } else { Some(message) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decide_install_step_enabled_proceeds() {
        assert_eq!(
            decide_install_step(&Ok(DomainUserEnabledState::Enabled)),
            InstallDecision::Proceed
        );
    }

    // ── Task 1698 part 3: stale domain cleanup (closes 1696, audit G8) ─────

    #[test]
    fn test_1698_stale_domain_filter_removes_foreign_domains_keeps_ours() {
        // The zombie from the 0.8.6 era (io.beebeeb.desktop.FileProvider) and
        // anything else foreign must be removed; the CURRENT domain never.
        let domains = vec![
            "io.beebeeb.desktop.FileProvider".to_string(),
            "io.beebeeb.app.domain".to_string(),
            "com.other.zombie".to_string(),
        ];
        let stale = stale_domain_identifiers(&domains, "io.beebeeb.app.domain");
        assert_eq!(
            stale,
            vec!["io.beebeeb.desktop.FileProvider", "com.other.zombie"],
            "every non-Beebeeb domain is stale"
        );
    }

    #[test]
    fn test_1698_stale_domain_filter_is_idempotent_and_safe() {
        // A clean registry removes nothing.
        assert_eq!(
            stale_domain_identifiers(&["io.beebeeb.app.domain".to_string()], "io.beebeeb.app.domain"),
            Vec::<&str>::new()
        );
        // An empty registry removes nothing.
        assert_eq!(stale_domain_identifiers(&[], "io.beebeeb.app.domain"), Vec::<&str>::new());
        // NEVER a partial match: only exact foreign identifiers are stale.
        assert_eq!(
            stale_domain_identifiers(&["io.beebeeb.app.domain.stale".to_string()], "io.beebeeb.app.domain"),
            vec!["io.beebeeb.app.domain.stale"]
        );
        // Case matters: the identifier is compared exactly.
        assert_eq!(
            stale_domain_identifiers(&["IO.BEEBEEB.APP.DOMAIN".to_string()], "io.beebeeb.app.domain"),
            vec!["IO.BEEBEEB.APP.DOMAIN"],
            "a case-mangled copy of our id is NOT ours and must be cleaned"
        );
    }

    #[test]
    fn decide_install_step_not_registered_proceeds() {
        // A fresh install (domain doesn't exist yet) must still go through the normal
        // addDomain-then-wait path -- NotRegistered is not itself Issue 4's condition.
        assert_eq!(
            decide_install_step(&Ok(DomainUserEnabledState::NotRegistered)),
            InstallDecision::Proceed
        );
    }

    #[test]
    fn decide_install_step_disabled_short_circuits_without_waiting() {
        assert_eq!(
            decide_install_step(&Ok(DomainUserEnabledState::Disabled)),
            InstallDecision::UserDisabled
        );
    }

    #[test]
    fn decide_install_step_lookup_error_falls_back_to_proceed() {
        // A lookup failure must never be treated as evidence of Issue 4 -- fall back to
        // the pre-existing wait/timeout behavior instead of guessing.
        assert_eq!(
            decide_install_step(&Err("getDomainsWithCompletionHandler failed".to_string())),
            InstallDecision::Proceed
        );
    }

    #[test]
    fn status_outcome_disabled_short_circuits_without_calling_live_status() {
        // The whole point of the status-path fix (PR #63 Codex review,
        // Onboarding.tsx:488): a disabled domain must never reach the
        // stabilization-wait FFI call at all, not just resolve to the same
        // answer eventually.
        let called = std::cell::Cell::new(false);
        let result = status_outcome_from(&Ok(DomainUserEnabledState::Disabled), || {
            called.set(true);
            Ok(true)
        });
        assert_eq!(result, Ok(StatusOutcome::UserDisabled));
        assert!(!called.get(), "live_status must not be called for a disabled domain");
    }

    #[test]
    fn status_outcome_enabled_proceeds_to_live_status() {
        assert_eq!(
            status_outcome_from(&Ok(DomainUserEnabledState::Enabled), || Ok(true)),
            Ok(StatusOutcome::Installed)
        );
        assert_eq!(
            status_outcome_from(&Ok(DomainUserEnabledState::Enabled), || Ok(false)),
            Ok(StatusOutcome::NotInstalled)
        );
    }

    #[test]
    fn status_outcome_not_registered_proceeds_to_live_status() {
        // A fresh install (domain doesn't exist yet) still goes through the normal
        // check -- NotRegistered is not itself Issue 4's condition.
        assert_eq!(
            status_outcome_from(&Ok(DomainUserEnabledState::NotRegistered), || Ok(false)),
            Ok(StatusOutcome::NotInstalled)
        );
    }

    #[test]
    fn status_outcome_lookup_error_falls_back_to_live_status() {
        // Never invent a UserDisabled verdict from an unreliable userEnabled lookup --
        // fall back to the pre-existing live-status/timeout behavior.
        assert_eq!(
            status_outcome_from(&Err("getDomainsWithCompletionHandler failed".to_string()), || Ok(true)),
            Ok(StatusOutcome::Installed)
        );
    }

    #[test]
    fn status_outcome_propagates_a_genuine_live_status_error() {
        // When the domain IS enabled but the live stabilization wait itself times out
        // (a real, unrelated failure), that error must still surface unchanged --
        // short-circuiting only ever applies to the disabled case.
        assert_eq!(
            status_outcome_from(&Ok(DomainUserEnabledState::Enabled), || Err(
                "Timed out waiting for the Beebeeb File Provider domain to become available".to_string()
            )),
            Err("Timed out waiting for the Beebeeb File Provider domain to become available".to_string())
        );
    }
}

/// Task 1882 round 2 (device K-F2, review I3, spec §4/§8): the bridge's own folder check, run on
/// a real disk through its test-only entry point, which builds the same `NSURL` and calls the
/// same function the removal calls.
#[cfg(test)]
mod kept_folder_check_tests {
    use crate::finder_removal::{KEPT_EMPTY, KEPT_HAS_ENTRIES, KEPT_MISSING, KEPT_NONE_REPORTED, KEPT_UNCHECKED};
    use std::ffi::CString;
    use std::os::unix::fs::PermissionsExt;

    unsafe extern "C" {
        fn beebeeb_fp_kept_folder_state_for_path(path: *const std::os::raw::c_char) -> i32;
    }

    fn state_of(path: &std::path::Path) -> i32 {
        let c_path = CString::new(path.as_os_str().as_encoded_bytes()).expect("no NUL in a temp path");
        unsafe { beebeeb_fp_kept_folder_state_for_path(c_path.as_ptr()) }
    }

    #[test]
    fn test_1882_r2_bridge_check_a_missing_folder_is_missing() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert_eq!(state_of(&dir.path().join("Beebeeb-Drive (10-10-2026 10:52)")), KEPT_MISSING);
        assert_eq!(
            unsafe { beebeeb_fp_kept_folder_state_for_path(std::ptr::null()) },
            KEPT_NONE_REPORTED,
            "no URL is 'none reported'"
        );
    }

    #[test]
    fn test_1882_r2_bridge_check_an_empty_folder_is_empty() {
        let dir = tempfile::tempdir().expect("temp dir");
        let kept = dir.path().join("Beebeeb-Beebeeb (10-10-2026 10:50)");
        std::fs::create_dir(&kept).expect("kept folder");
        assert_eq!(state_of(&kept), KEPT_EMPTY);
    }

    #[test]
    fn test_1882_r2_bridge_check_a_folder_with_one_item_has_entries() {
        let dir = tempfile::tempdir().expect("temp dir");
        let kept = dir.path().join("Beebeeb-Beebeeb (10-10-2026 10:50)");
        std::fs::create_dir(&kept).expect("kept folder");
        std::fs::write(kept.join("k1882.txt"), b"never uploaded").expect("kept file");
        assert_eq!(state_of(&kept), KEPT_HAS_ENTRIES);
    }

    #[test]
    fn test_1882_r2_bridge_check_a_hidden_item_or_a_single_file_counts() {
        let dir = tempfile::tempdir().expect("temp dir");
        let hidden_only = dir.path().join("hidden-only");
        std::fs::create_dir(&hidden_only).expect("folder");
        std::fs::write(hidden_only.join(".env"), b"a dotfile is the person's data too").expect("dotfile");
        assert_eq!(state_of(&hidden_only), KEPT_HAS_ENTRIES);

        let single_file = dir.path().join("kept-file.bin");
        std::fs::write(&single_file, b"one kept file").expect("file");
        assert_eq!(state_of(&single_file), KEPT_HAS_ENTRIES);
    }

    #[test]
    fn test_1882_r2_bridge_check_a_folder_it_may_not_list_is_unchecked() {
        let dir = tempfile::tempdir().expect("temp dir");
        let locked = dir.path().join("locked");
        std::fs::create_dir(&locked).expect("folder");
        std::fs::write(locked.join("k1882.txt"), b"x").expect("file");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).expect("chmod 000");
        let state = state_of(&locked);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).expect("chmod back");
        assert_eq!(
            state, KEPT_UNCHECKED,
            "an existing folder the app may not list is shown (spec §4's fallback), never hidden"
        );
    }
}
