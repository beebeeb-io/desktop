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
    fn beebeeb_fp_remove(error_buffer: *mut c_char, error_buffer_len: usize) -> i32;
}

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

#[allow(dead_code)]
pub fn remove() -> Result<(), String> {
    call_bridge(beebeeb_fp_remove).map(|_| ())
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
