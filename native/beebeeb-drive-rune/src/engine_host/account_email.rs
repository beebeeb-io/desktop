//! Changing the sign-in email on the real engine: the calls are in `account_twofactor.rs` (`StartEmail`,
//! `ConfirmEmail`: one worker for both); this file holds what is true of the email page alone.
//!
//! - Beebeeb has no route that sends the email's code again, so there is no "Send again": [`can_send_again`] says
//!   so and the page draws no link. The wait is 0 and nothing is sent.
//! - The password of the press reaches the engine inside a wiping buffer for the one call (core proves it with its
//!   own OPAQUE step-up and registers it again under the new address); this side keeps no copy.
//! - After the code is taken, the account is read again, so Account shows the new address from Beebeeb.

/// Whether the page may offer "Send again" for the email's code: never, on this engine (see the module's doc).
pub(super) fn can_send_again() -> bool {
    false
}
