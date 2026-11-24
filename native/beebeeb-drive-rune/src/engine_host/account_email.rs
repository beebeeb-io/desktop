//! Changing the sign-in email on the real engine: the calls are in `account_twofactor.rs` (`StartEmail`,
//! `ConfirmEmail`, `ResendEmail`: one worker for all); this file holds what is true of the email page alone.
//!
//! - "Send again" ([`Engine::email_change_resend`]) is sent once per press. A 429 that says to wait counts down from
//!   Beebeeb's `Retry-After` ([`Waiting`]): the link is not pressable until it is 0, and the page says "Wait 42 s
//!   before asking again."; a limit says there are no more codes for this change; a change that is gone goes back to
//!   step 1. No wait is invented: before a 429 there is none.
//! - The password of the press reaches the engine inside a wiping buffer for the one call (core proves it with its
//!   own OPAQUE step-up and registers it again under the new address); this side keeps no copy.
//! - The address Beebeeb named (the start's answer, or a resend's) is held, for this session and account only, until
//!   the code is taken; then it replaces the address the engine mirrors ([`Engine::email_changed_for`]), so the old
//!   address is not shown again (Account, the sign-in screen's "Continue as"). The account is read again after.

use std::time::{Duration, Instant};

use super::Stamp;

/// A wait Beebeeb asked for, for the session and account it was said to.
#[derive(Default)]
pub(super) struct Waiting(Option<(Stamp, Instant)>);

impl Waiting {
    pub(super) fn start(&mut self, stamp: Stamp, seconds: u32) {
        self.0 = Some((stamp, Instant::now() + Duration::from_secs(u64::from(seconds))));
    }

    pub(super) fn clear(&mut self) {
        self.0 = None;
    }

    /// Whole seconds left, rounded up; 0 when over, never asked, or said to another session or account.
    pub(super) fn left(&self, now: &Stamp) -> u32 {
        match &self.0 {
            Some((stamp, until)) if stamp == now => {
                let left = until.saturating_duration_since(Instant::now());
                (left.as_millis().div_ceil(1000)) as u32
            }
            _ => 0,
        }
    }
}
