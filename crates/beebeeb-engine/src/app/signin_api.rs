//! A sign-in step a host's screens need beside the password, the code and the twelve words (`super`): leaving a
//! sign-in that is half done. (Opening a closed vault with the account's password is the lock's:
//! `lock_api::unlock_vault_with_password`, decision 2495. The step-up proof that stood here is gone.)
//!
//! Every method BLOCKS on the handle's runtime, as every method of the handle does: call it from a worker thread,
//! never from the UI thread.
//!
//! What a half-done step holds here goes when the step ends, however it ends (`Engine::forget_half_done_sign_in`):
//! the proven password's code challenge (core's partial token, and the email beside it), the password's stretch for
//! the screen-lock check (core's check key) and a held recovery (the key the twelve words derived, the server's
//! recovery token). Each is a wiping buffer of core's or ours, so "gone" is "wiped".

use super::{CredentialStores, Engine};
use crate::signin::gate::GateState;

impl<C: CredentialStores> Engine<C> {
    /// The person left a sign-in that was half done. A proven password that waits for its code is forgotten, so
    /// no later code completes it. An account that is signed in on a computer that holds no key of it (the twelve
    /// words were being asked) is signed out, so another sign-in can start; the local files and the local state
    /// stay. A locked vault (its key is here) and an open one are left as they are. A held recovery is forgotten
    /// too (its key and recovery token are wiped).
    ///
    /// One hold of the step lock: the question "is this a keyless session?" and the sign-out are not separated, so
    /// no other step lands between them. A step that is already on its way is NOT interrupted: this waits for it,
    /// and what it finished stands as far as this method goes (a code check that was on its way signs in first;
    /// if that leaves a keyless session, it is signed out here). A host that lets a person leave a running step
    /// undoes that step's result itself (the native adapter does).
    /// `Err`: the sign-out did not finish (`Engine::sign_out_keeping_local_data`); still signed in.
    pub fn discard_partial_sign_in(&self) -> Result<(), String> {
        let result = self.runtime.block_on(async {
            let _step = self.session_ending_step().await;
            self.forget_half_done_sign_in();
            if self.gate() == GateState::NeedsRecoveryPhrase {
                self.sign_out_holding_step().await
            } else {
                Ok(())
            }
        });
        self.publish();
        result
    }

    /// Every half-done step's copies, wiped as they drop: the code challenge of a proven password, that password's
    /// stretch for the screen-lock check, and a held recovery. Called wherever a sign-in ends or is replaced: when a
    /// half-done sign-in is left, when a new password step starts, when a session is stored, and when the session is
    /// forgotten (a sign-out, a lock). Never waits on the network or on a step.
    pub(super) fn forget_half_done_sign_in(&self) {
        *self.pending_2fa.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        self.lock_password.drop_pending();
        self.recovery_forget();
    }
}

#[cfg(test)]
#[path = "signin_api_tests.rs"]
mod tests;
