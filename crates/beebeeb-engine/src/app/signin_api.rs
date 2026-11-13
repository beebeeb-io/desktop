//! Two sign-in steps a host's screens need beside the password, the code and the twelve words (`super`): opening a
//! locked vault with the account's password, and leaving a sign-in that is half done.
//!
//! Every method BLOCKS on the handle's runtime, as every method of the handle does: call it from a worker thread,
//! never from the UI thread.
//!
//! [`Engine::unlock_with_password`] adds no cryptography and changes no parameter: the password is proven to the
//! server by core's step-up (`auth_flow::step_up`, OPAQUE with the server's `ksf_version`, on the stored session),
//! and the key that is put in memory is the one this computer's credential store already holds, read through the
//! same guarded loader as the startup restore (`store::load_session_from_vault`: whose key it is, is asked first).
//! The password does not derive, wrap or unwrap anything here.
//!
//! Must not: log the password, the token, the key or a server's words; unlock on a proof that did not complete (a
//! server that cannot be reached is never read as "right").

use beebeeb_core::auth_flow::step_up::{StepUp, StepUpPurpose, StepUpStep};
use zeroize::Zeroizing;

use super::{CredentialStores, Engine};
use crate::auth_transport;
use crate::signin::gate::{GateState, SignInProblem};
use crate::signin::key_owner::{self, StoredKey};
use crate::signin::{flow_code, store};

/// What the step-up's first leg answers when the stored session is no longer accepted: no password was judged.
const SESSION_NOT_ACCEPTED: &str = "session_not_accepted";

impl<C: CredentialStores> Engine<C> {
    /// Open a locked vault (signed in, the key is in this computer's credential store, not in memory) with the
    /// account's password. The server proves the password for the stored session; only then is the stored key
    /// loaded, in this step's turn. Needs the network: a proof that cannot be made unlocks nothing.
    ///
    /// `Ok`: unlocked (also when it already was). `Invalid` / `LockedHint`: not the account's password. `Offline`,
    /// `RateLimited`: as the server (or the lack of one) said. `Other`: nothing to unlock this way (signed out, or
    /// no usable key here: the twelve words are the way), the session is no longer accepted, or the session changed
    /// while the proof was on its way. Nothing is stored or removed by a refusal.
    pub fn unlock_with_password(&self, password: Zeroizing<String>) -> Result<(), SignInProblem> {
        let result = self.unlock_with_password_step(password);
        self.publish();
        result
    }

    fn unlock_with_password_step(&self, password: Zeroizing<String>) -> Result<(), SignInProblem> {
        self.runtime.block_on(async {
            let _step = self.transition.lock().await;
            let turn = self.acct.session_generation();
            if self.unlocked() {
                return Ok(());
            }
            let vault = self.vault();
            let token = match vault.session_token() {
                Ok(Some(token)) => Zeroizing::new(token.expose_for_request().to_string()),
                _ => return Err(SignInProblem::Other),
            };
            // Only a key the loader will accept is unlocked this way; no proof is asked for one it would refuse.
            if key_owner::stored_key_state(&vault) != StoredKey::Usable {
                return Err(SignInProblem::Other);
            }
            self.prove_password(&token, &password)
                .await
                .map_err(|code| self.problem(&code))?;
            drop(password);
            let session = store::load_session_from_vault(self.vault(), self.account_email())
                .ok()
                .flatten()
                .ok_or(SignInProblem::Other)?;
            {
                let _write = self.claim(turn).map_err(|_| SignInProblem::Other)?;
                // The session the password was proven for is the one whose key was loaded, in this turn.
                if session.token != token.as_str() || self.unlocked() {
                    return Err(SignInProblem::Other);
                }
                let mut guard = self.acct.session.lock().map_err(|_| SignInProblem::Other)?;
                *guard = Some(session);
                drop(guard);
                self.acct
                    .auth_resume_asked
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            self.failures.store(0, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        })
    }

    /// Core's step-up for the stored session. The grant it mints is dropped unspent (it is single-use and expires;
    /// core gives no way to read it). `Err` is a code for [`Engine::problem`].
    async fn prove_password(&self, token: &str, password: &str) -> Result<(), String> {
        let (mut flow, mut request) =
            StepUp::start(password.as_bytes(), StepUpPurpose::Generic).map_err(|e| flow_code(&e))?;
        let mut first_leg = true;
        loop {
            let response = auth_transport::send_to(&self.config.api_base, &self.client, Some(token), request).await?;
            // The first leg carries no proof of the password: a 401 there is about the session, not the password.
            if first_leg && response.status == 401 {
                return Err(SESSION_NOT_ACCEPTED.to_string());
            }
            first_leg = false;
            match flow.on_response(response).map_err(|e| flow_code(&e))? {
                StepUpStep::Send(next) => request = next,
                StepUpStep::Confirmed(_grant) => return Ok(()),
            }
        }
    }

    /// The person left a sign-in that was half done. A proven password that waits for its code is forgotten (a
    /// code check that is still on its way then ends in "expired", and nothing is stored). An account that is
    /// signed in on a computer that holds no key of it (the twelve words were being asked) is signed out, so
    /// another sign-in can start; the local files and the local state stay. A locked vault (its key is here) and
    /// an open one are left as they are.
    ///
    /// Takes its turn after a step that is on its way: what that step finished stands, and the gate says so.
    /// `Err`: the sign-out did not finish (`Engine::sign_out_keeping_local_data`); still signed in.
    pub fn discard_partial_sign_in(&self) -> Result<(), String> {
        let sign_out = self.runtime.block_on(async {
            let _step = self.transition.lock().await;
            if let Ok(mut pending) = self.pending_2fa.lock() {
                *pending = None;
            }
            self.gate() == GateState::NeedsRecoveryPhrase
        });
        if sign_out {
            return self.sign_out_keeping_local_data();
        }
        self.publish();
        Ok(())
    }
}
