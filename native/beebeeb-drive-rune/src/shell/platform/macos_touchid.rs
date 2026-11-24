//! Touch ID as the macOS half of `windows_hello.rs` (its `imp` on macOS, included there by path: this file names
//! nothing but `super`, so `examples/hello_probe.rs` compiles it as it is).
//!
//! `LAContext` with `LAPolicy::DeviceOwnerAuthenticationWithBiometrics`: fingerprint only, no password fallback of
//! the system (the lock screen's own password field is the other way in). The reason line is the caller's. The
//! answers are mapped to the same [`Seen`] the Windows half produces, so the one-prompt rule, the cancel contract
//! and the dropped-answer rule stay `windows_hello.rs`'s. A cancel or a newer prompt `invalidate`s the context
//! from the prompt's own thread; the reply that follows is ignored. Nothing is logged, and no error text is read:
//! only the code of the `LAError`.

use std::sync::{Arc, Mutex};

use block2::RcBlock;
use objc2::runtime::Bool;
use objc2_foundation::{NSError, NSString};
use objc2_local_authentication::{LAContext, LAPolicy};

use super::{raw, Control, HelloAvailability, HelloResult, HelloWindow, Seen, Woken};

/// `LAError` codes (`LAError.h`), the ones the mapping names. Any other code is a failure.
mod la {
    pub const AUTHENTICATION_FAILED: isize = -1;
    pub const USER_CANCEL: isize = -2;
    pub const USER_FALLBACK: isize = -3;
    pub const SYSTEM_CANCEL: isize = -4;
    pub const PASSCODE_NOT_SET: isize = -5;
    pub const BIOMETRY_NOT_AVAILABLE: isize = -6;
    pub const BIOMETRY_NOT_ENROLLED: isize = -7;
    pub const BIOMETRY_LOCKOUT: isize = -8;
    pub const APP_CANCEL: isize = -9;
}

/// How a prompt that did not succeed ended, from its `LAError` code (`None`: no error was handed over).
fn result_of_code(code: Option<isize>) -> HelloResult {
    match code {
        // A person who chose the fallback wants the password field: the same as closing the prompt.
        Some(la::USER_CANCEL | la::APP_CANCEL | la::SYSTEM_CANCEL | la::USER_FALLBACK) => HelloResult::Cancelled,
        Some(la::BIOMETRY_NOT_AVAILABLE | la::BIOMETRY_NOT_ENROLLED | la::PASSCODE_NOT_SET) => HelloResult::Unavailable,
        Some(la::AUTHENTICATION_FAILED | la::BIOMETRY_LOCKOUT) => HelloResult::Failed,
        Some(_) | None => HelloResult::Failed,
    }
}

/// Whether the policy can be evaluated, from the `LAError` code of `canEvaluatePolicy` (`None`: it can).
fn availability_of_code(code: Option<isize>) -> HelloAvailability {
    match code {
        None => HelloAvailability::Available,
        Some(la::BIOMETRY_NOT_ENROLLED | la::PASSCODE_NOT_SET) => HelloAvailability::NotConfigured,
        // A lockout is a set-up Touch ID that is shut for now: the prompt is offered and ends `Failed`.
        Some(la::BIOMETRY_LOCKOUT) => HelloAvailability::Available,
        Some(_) => HelloAvailability::Unsupported,
    }
}

/// What the platform half saw, from the reply of the prompt.
fn seen_of(verified: bool, code: Option<isize>) -> Seen {
    if verified {
        return Seen::Ended {
            status: raw::STATUS_COMPLETED,
            result: Some(raw::VERIFIED),
        };
    }
    match result_of_code(code) {
        HelloResult::Cancelled => Seen::Ended {
            status: raw::STATUS_CANCELED,
            result: None,
        },
        HelloResult::Unavailable => Seen::NoPrompt,
        HelloResult::Verified | HelloResult::Failed => Seen::Refused,
    }
}

fn availability_code() -> Option<isize> {
    // SAFETY: a fresh context used on this thread only.
    let context = unsafe { LAContext::new() };
    // SAFETY: plain query on the live context.
    match unsafe { context.canEvaluatePolicy_error(LAPolicy::DeviceOwnerAuthenticationWithBiometrics) } {
        Ok(()) => None,
        Err(error) => Some(error.code()),
    }
}

pub(super) fn availability() -> Option<i32> {
    Some(match availability_of_code(availability_code()) {
        HelloAvailability::Available => raw::AVAILABLE,
        HelloAvailability::NotConfigured => raw::NOT_CONFIGURED_FOR_USER,
        HelloAvailability::Disabled => raw::DISABLED_BY_POLICY,
        HelloAvailability::Unsupported => raw::DEVICE_NOT_PRESENT,
    })
}

pub(super) fn verify(_window: HelloWindow, reason: &str, control: &Arc<Control>) -> Seen {
    if availability_of_code(availability_code()) != HelloAvailability::Available {
        return Seen::NoPrompt;
    }
    // Whoever asked has already left, or a newer prompt took this one's place: no prompt for them.
    if control.taken_away() {
        return Seen::Refused;
    }
    // SAFETY: a fresh context, used on this thread only.
    let context = unsafe { LAContext::new() };
    let reply_seen: Arc<Mutex<Option<Seen>>> = Arc::new(Mutex::new(None));
    let (ended, slot) = (control.clone(), reply_seen.clone());
    let reply = RcBlock::new(move |verified: Bool, error: *mut NSError| {
        // SAFETY: the system hands over null or a live error for the duration of the call.
        let code = unsafe { error.as_ref() }.map(NSError::code);
        *super::lock(&slot) = Some(seen_of(verified.as_bool(), code));
        ended.prompt_ended(raw::STATUS_COMPLETED);
    });
    let reason = NSString::from_str(reason);
    // SAFETY: the reply captures two `Arc`s, which are `Send`; it runs once, on a system queue.
    unsafe {
        context.evaluatePolicy_localizedReason_reply(
            LAPolicy::DeviceOwnerAuthenticationWithBiometrics,
            &reason,
            &reply,
        );
    }
    match control.wait() {
        Woken::PromptEnded(_) => super::lock(&reply_seen).take().unwrap_or(Seen::Refused),
        Woken::TakenAway => {
            // Closes the sheet; the reply that follows (appCancel) finds nobody waiting.
            // SAFETY: plain call on the live context, from the thread that made it.
            unsafe { context.invalidate() };
            Seen::Refused
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::settle;
    use super::*;

    #[test]
    fn every_la_error_maps_to_its_own_result() {
        let table = [
            (Some(la::USER_CANCEL), HelloResult::Cancelled),
            (Some(la::APP_CANCEL), HelloResult::Cancelled),
            (Some(la::SYSTEM_CANCEL), HelloResult::Cancelled),
            (Some(la::AUTHENTICATION_FAILED), HelloResult::Failed),
            (Some(la::BIOMETRY_LOCKOUT), HelloResult::Failed),
            (Some(la::BIOMETRY_NOT_AVAILABLE), HelloResult::Unavailable),
            (Some(la::BIOMETRY_NOT_ENROLLED), HelloResult::Unavailable),
            (Some(la::PASSCODE_NOT_SET), HelloResult::Unavailable),
            (Some(-1004), HelloResult::Failed),
            (None, HelloResult::Failed),
        ];
        for (code, want) in table {
            assert_eq!(result_of_code(code), want, "{code:?}");
            if want != HelloResult::Verified {
                assert_eq!(settle(seen_of(false, code), false), want, "{code:?} through settle");
            }
        }
    }

    #[test]
    fn only_a_yes_verifies() {
        assert_eq!(settle(seen_of(true, None), false), HelloResult::Verified);
        for code in [None, Some(-1), Some(-2), Some(-8), Some(0)] {
            assert_ne!(settle(seen_of(false, code), false), HelloResult::Verified);
        }
    }

    #[test]
    fn every_availability_code_maps_to_its_own_answer() {
        assert_eq!(availability_of_code(None), HelloAvailability::Available);
        assert_eq!(
            availability_of_code(Some(la::BIOMETRY_NOT_ENROLLED)),
            HelloAvailability::NotConfigured
        );
        assert_eq!(
            availability_of_code(Some(la::PASSCODE_NOT_SET)),
            HelloAvailability::NotConfigured
        );
        assert_eq!(
            availability_of_code(Some(la::BIOMETRY_NOT_AVAILABLE)),
            HelloAvailability::Unsupported
        );
        assert_eq!(
            availability_of_code(Some(la::BIOMETRY_LOCKOUT)),
            HelloAvailability::Available
        );
        assert_eq!(availability_of_code(Some(-1004)), HelloAvailability::Unsupported);
    }
}
