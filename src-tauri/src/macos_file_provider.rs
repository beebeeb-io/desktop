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

pub fn status() -> Result<bool, String> {
    Ok(call_bridge(beebeeb_fp_status)? == 1)
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
}
