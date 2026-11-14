//! Change password (DW-A67, DW-A68) on the real engine: [`PasswordCalls`], the part of `EngineAccountSource` behind
//! `password_strength`, the step-up for [`Purpose::ChangePassword`] and `change_password`.
//!
//! - Strength is core's evaluator ([`password_strength`]): on this PC, no network, a draw may ask.
//! - The change is [`Engine::change_password_for`] (core's `account_flows`, the server's policy and the breach check
//!   included). Core's flow proves the CURRENT password itself, once for each of its two legs, so the form's step-up
//!   for this purpose proves nothing on its own: it keeps the typed password, for this one change, with the session
//!   and the account of its press. A wrong password is refused by the flow's own step-up, before anything changes,
//!   and the form shows it as it shows a refused step-up.
//! - The kept password is used once: by the change that carries this confirmation, under the same session and
//!   account. A second step-up, a session or account that moved on, or any answer of the change drops it. It is
//!   wiped when dropped ([`Zeroizing`]).
//! - The new password leaves the form's field only as a wiped copy for the one call.
//!
//! Thread rule, said plainly: [`AccountSource::change_password`] returns its answer, so it waits for the engine's
//! call on the window's thread (a handful of requests, each given up after 30 seconds). The engine never touches a
//! window; a frame that shows "Changing..." while a worker runs needs the form to ask in two parts, which the form
//! does not yet.
//!
//! Must not: log either password; keep one past the call; send anything for a confirmation of another purpose,
//! one already used, or one made under another session or account.

use std::sync::Arc;

use beebeeb_engine::app::account_password_api::{
    PasswordChangeError, PasswordStrength, password_strength as core_strength,
};
use beebeeb_engine::app::{CredentialStores, Engine};
use zeroize::Zeroizing;

use super::{Stamp, stamp};
use crate::frames::account::source::{PaneState, Password, Purpose, Refused, StepUp, Strength};

/// What a typed password is kept for: one change, under one session and account.
struct Kept {
    password: Zeroizing<String>,
    pressed: Stamp,
    handle: u64,
}

/// See the module's doc.
pub struct PasswordCalls<C: CredentialStores> {
    engine: Arc<Engine<C>>,
    kept: Option<Kept>,
    made: u64,
}

impl<C: CredentialStores> PasswordCalls<C> {
    pub fn new(engine: Arc<Engine<C>>) -> Self {
        Self {
            engine,
            kept: None,
            made: 0,
        }
    }

    /// Change password answers when someone is signed in and unlocked: it reads nothing, so there is nothing to wait
    /// for. `refused`: the engine holds the session as ended.
    pub fn pane_state(&self, refused: bool) -> PaneState {
        if !self.engine.has_session() {
            PaneState::SignedOut
        } else if refused {
            PaneState::SessionEnded
        } else {
            PaneState::Ready
        }
    }

    /// Core's evaluator at core's floor. A password that is not text reads as too short.
    pub fn strength(&self, candidate: &Password<'_>) -> Strength {
        match std::str::from_utf8(candidate.for_opaque()) {
            Ok(text) => match core_strength(text) {
                PasswordStrength::TooShort => Strength::TooShort,
                PasswordStrength::Fair => Strength::Fair,
                PasswordStrength::Good => Strength::Good,
                PasswordStrength::Strong => Strength::Strong,
            },
            Err(_) => Strength::TooShort,
        }
    }

    /// The step-up of [`Purpose::ChangePassword`]: the typed password is kept for the change (see the module's doc).
    pub fn keep(&mut self, password: Password<'_>) -> Result<StepUp, Refused> {
        self.kept = None;
        let Ok(text) = std::str::from_utf8(password.for_opaque()) else {
            return Err(Refused::WrongPassword);
        };
        if text.is_empty() {
            return Err(Refused::WrongPassword);
        }
        if !self.engine.has_session() {
            return Err(Refused::Locked);
        }
        self.made += 1;
        self.kept = Some(Kept {
            password: Zeroizing::new(text.to_string()),
            pressed: stamp(&self.engine),
            handle: self.made,
        });
        Ok(StepUp::granted(Purpose::ChangePassword, self.made))
    }

    /// The change: for the confirmation [`Self::keep`] made, under the session and account it was made under.
    pub fn change(&mut self, confirmation: StepUp, new: Password<'_>) -> Result<(), Refused> {
        let Some(kept) = self.kept.take() else {
            return Err(Refused::NotConfirmed);
        };
        if confirmation.purpose() != Purpose::ChangePassword
            || confirmation.handle() != kept.handle
            || stamp(&self.engine) != kept.pressed
        {
            return Err(Refused::NotConfirmed);
        }
        let Ok(new) = std::str::from_utf8(new.for_opaque()) else {
            return Err(Refused::Weak);
        };
        let new = Zeroizing::new(new.to_string());
        self.engine
            .change_password_for(kept.pressed.0, kept.password, new)
            .map_err(refusal)
    }

    /// The session or account moved on, or the pane closed: what was kept is gone.
    pub fn forget_if_moved(&mut self) {
        if self
            .kept
            .as_ref()
            .is_some_and(|kept| stamp(&self.engine) != kept.pressed)
        {
            self.kept = None;
        }
    }

    pub fn forget(&mut self) {
        self.kept = None;
    }
}

/// The form's words for each way a change is refused: only what the form can say. The engine's kinds that are not
/// the password or its strength say "Beebeeb can't be reached. Nothing was changed.": nothing was.
fn refusal(error: PasswordChangeError) -> Refused {
    match error {
        PasswordChangeError::WrongPassword => Refused::WrongPassword,
        PasswordChangeError::SamePassword | PasswordChangeError::TooWeak | PasswordChangeError::Breached => {
            Refused::Weak
        }
        PasswordChangeError::Locked => Refused::Locked,
        PasswordChangeError::NotSignedIn => Refused::NotConfirmed,
        PasswordChangeError::SessionEnded
        | PasswordChangeError::BreachCheckUnavailable
        | PasswordChangeError::RateLimited
        | PasswordChangeError::Offline
        | PasswordChangeError::Failed(_) => Refused::Offline,
    }
}

#[cfg(test)]
#[path = "account_password_tests.rs"]
mod tests;
