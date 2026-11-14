//! A sign-in step a host's screens need beside the password, the code and the twelve words (`super`): leaving a
//! sign-in that is half done. (Opening a closed vault with the account's password is the lock's:
//! `lock_api::unlock_vault_with_password`, decision 2495. The step-up proof that stood here is gone.)
//!
//! Every method BLOCKS on the handle's runtime, as every method of the handle does: call it from a worker thread,
//! never from the UI thread.

use super::{CredentialStores, Engine};
use crate::signin::gate::GateState;

impl<C: CredentialStores> Engine<C> {
    /// The person left a sign-in that was half done. A proven password that waits for its code is forgotten, so
    /// no later code completes it. An account that is signed in on a computer that holds no key of it (the twelve
    /// words were being asked) is signed out, so another sign-in can start; the local files and the local state
    /// stay. A locked vault (its key is here) and an open one are left as they are.
    ///
    /// One hold of the step lock: the question "is this a keyless session?" and the sign-out are not separated, so
    /// no other step lands between them. A step that is already on its way is NOT interrupted: this waits for it,
    /// and what it finished stands as far as this method goes (a code check that was on its way signs in first;
    /// if that leaves a keyless session, it is signed out here). A host that lets a person leave a running step
    /// undoes that step's result itself (the native adapter does).
    /// `Err`: the sign-out did not finish (`Engine::sign_out_keeping_local_data`); still signed in.
    pub fn discard_partial_sign_in(&self) -> Result<(), String> {
        let result = self.runtime.block_on(async {
            let _step = self.transition.lock().await;
            if let Ok(mut pending) = self.pending_2fa.lock() {
                *pending = None;
            }
            if self.gate() == GateState::NeedsRecoveryPhrase {
                self.sign_out_holding_step().await
            } else {
                Ok(())
            }
        });
        self.publish();
        result
    }
}
