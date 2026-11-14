//! Changing the account's password from the window: how strong a typed password is, and the change itself.
//!
//! The rules are core's. The strength is `onboarding::password::evaluate_password` (no network: a window asks on
//! every draw). The change is `account_flows::AccountFlow::change_password`: the server's policy and the breach
//! check (Beebeeb's own endpoint, the first five characters of a hash only) are applied by core's gate, then an
//! OPAQUE re-registration with a step-up for each leg. This file only sends core's requests and keeps what the
//! answer changes on this PC.
//!
//! What this PC stores follows the change like this. The master key is NOT touched (core: "no master-key wrap":
//! the password only authenticates, it wraps nothing), so the credential store's vault key and its owner tag stay as
//! they are. The server ends every other session and answers this one with a NEW session token: it is stored in the
//! credential store (`AuthVault::install_session`, the account's owner tag put back beside it, rule 1 of
//! `signin::key_owner`) and put in the session in memory in ONE turn (`Engine::claim`), and a running sync engine,
//! which holds the old token, is restarted on the new one.
//!
//! Every method BLOCKS on the handle's runtime: call it from a worker thread, never from the UI thread.
//!
//! Must not: log either password, a token, the master key, a server's words or the breach prefix; keep a password
//! after the call (both are `Zeroizing` and dropped here); send anything before the local checks (same password,
//! below the floor) have passed; store a token for a session that moved on.

use std::time::Duration;

use beebeeb_core::account_flows::{
    AccountBinding, AccountFlow, AccountFlowError, AccountOutcome, ApiRequest, ApiResponse, Method, Step,
};
use beebeeb_core::kdf::MasterKey;
pub use beebeeb_core::onboarding::PasswordStrength;
use beebeeb_core::onboarding::{BreachCheck, BreachResponse, PasswordPolicy, evaluate_password};
use beebeeb_core::signup_flow::SignupPolicy;
use zeroize::Zeroizing;

use super::{CredentialStores, Engine};
use crate::account::SessionGeneration;
use crate::auth_transport;
use crate::keychain::SessionToken;
use crate::signin::key_owner;

/// A request that gets no answer in this long is an offline one (as `auth_transport`'s own client rule).
const WAIT: Duration = Duration::from_secs(30);

/// How strong `password` is, by core's evaluator at core's floor (the server may ask for more at the change; that
/// gate is applied there, not here). No network, no state: cheap enough for every draw.
pub fn password_strength(password: &str) -> PasswordStrength {
    evaluate_password(password, &PasswordPolicy::from_server(0)).strength
}

/// Why a password was not changed. The password is as it was in each of these (see [`Engine::change_password_for`]
/// for the one answer that may have been lost on its way).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordChangeError {
    /// The current password is not the account's.
    WrongPassword,
    /// The new password is the current one.
    SamePassword,
    /// The new password is below the minimum (core's floor or the server's).
    TooWeak,
    /// The new password is in a known breach.
    Breached,
    /// The breach check could not be made and the server does not let a change go on without it.
    BreachCheckUnavailable,
    /// Signed in, but the vault is locked: the change needs the key it re-sends the binding of.
    Locked,
    /// No session, or not the session the press was made for.
    NotSignedIn,
    /// Beebeeb no longer accepts this session.
    SessionEnded,
    /// Too many tries.
    RateLimited,
    /// Beebeeb could not be reached, or did not answer in time.
    Offline,
    /// Beebeeb answered something this client does not read, or an error. A stable code, never the server's words.
    Failed(&'static str),
}

impl PasswordChangeError {
    fn of(error: &AccountFlowError) -> Self {
        match error {
            AccountFlowError::WrongPassword => Self::WrongPassword,
            AccountFlowError::SamePassword => Self::SamePassword,
            AccountFlowError::WeakPassword => Self::TooWeak,
            AccountFlowError::PasswordBreached => Self::Breached,
            AccountFlowError::BreachCheckBlocked | AccountFlowError::BreachCheckMissing => Self::BreachCheckUnavailable,
            AccountFlowError::RateLimited => Self::RateLimited,
            AccountFlowError::Unauthorized => Self::SessionEnded,
            other => Self::Failed(other.code()),
        }
    }
}

/// A snapshot of the session a change is made for.
struct Held {
    token: Zeroizing<String>,
    master_key: Zeroizing<[u8; 32]>,
}

impl<C: CredentialStores> Engine<C> {
    /// WRITES: change the account's password (`POST /api/v1/auth/change-password-start` + `-finish`, each behind a
    /// step-up of `current`). `session`: the session the press was made for ([`Engine::session_turn`]); another
    /// one changes nothing. On `Ok` the new session token is stored and in memory, the server has ended every other
    /// session, and sync runs again on the new token when it was running.
    ///
    /// `Err` leaves the password as it was, except one case the server decides: a finish whose answer never came
    /// (`Offline` after the second leg) may have been taken; the next call that is refused 401 says so, and the
    /// person signs in again with the new password.
    pub fn change_password_for(
        &self,
        session: SessionGeneration,
        current: Zeroizing<String>,
        new: Zeroizing<String>,
    ) -> Result<(), PasswordChangeError> {
        // Local checks first: nothing is sent for a password core would refuse anyway.
        if *current == *new {
            return Err(PasswordChangeError::SamePassword);
        }
        if !evaluate_password(&new, &PasswordPolicy::from_server(0)).meets_minimum {
            return Err(PasswordChangeError::TooWeak);
        }
        let syncing = self.is_syncing();
        let root = self.sync_root().ok();
        self.runtime
            .block_on(self.change_password_step(session, current, new))?;
        // The running engine holds the token that was just ended: it starts again on the new one. A refusal here is
        // not the change's: the password is changed and stored; sync waits for the next start.
        if syncing {
            let stopped = self.stop();
            match (stopped, root) {
                (crate::app::SyncStop::Unconfirmed, _) | (_, None) => {
                    tracing::warn!("password changed; sync was not started again on the new session");
                }
                (_, Some(root)) => {
                    if self.start_sync(root).is_err() {
                        tracing::warn!("password changed; sync was not started again on the new session");
                    }
                }
            }
        }
        Ok(())
    }

    async fn change_password_step(
        &self,
        session: SessionGeneration,
        current: Zeroizing<String>,
        new: Zeroizing<String>,
    ) -> Result<(), PasswordChangeError> {
        let _step = self.transition.lock().await;
        let held = self.held_session(session)?;
        let base = self.config.api_base.clone();

        // The server's policy, and the breach answer for THIS password when the policy declares a check.
        let policy = self.signup_policy(&base).await?;
        let breach = self.breach_check(&base, &policy, &new).await;

        let binding = {
            let key = MasterKey::from_bytes(*held.master_key);
            AccountBinding::from_master_key(&key)
        };
        let mut flow = AccountFlow::change_password(&current, &new, binding, &policy, breach.as_ref())
            .map_err(|error| PasswordChangeError::of(&error))?;
        drop(new);
        drop(breach);

        let mut step = flow.start().map_err(|error| PasswordChangeError::of(&error))?;
        let new_token = loop {
            match step {
                Step::Done(AccountOutcome::PasswordChanged { session_token }) => break session_token,
                Step::Done(_) => return Err(PasswordChangeError::Failed("protocol")),
                Step::Send(request) => {
                    let is_step_up_start = request.path.ends_with("/confirm-opaque-start");
                    let response = self.send_account(&base, &held.token, &request).await;
                    drop(request);
                    let response = match response {
                        // The first leg of a step-up carries no proof of the password: a 401 there is about the
                        // session (core would read it as a wrong password).
                        Ok(response) if response.status == 401 && is_step_up_start => {
                            self.note_session_refused(session);
                            return Err(PasswordChangeError::SessionEnded);
                        }
                        Ok(response) => response,
                        Err(error) => return Err(error),
                    };
                    step = flow.resume(response).map_err(|error| {
                        let error = PasswordChangeError::of(&error);
                        if error == PasswordChangeError::SessionEnded {
                            self.note_session_refused(session);
                        }
                        error
                    })?;
                }
            }
        };
        drop(current);

        // The password is changed on the server and the old token is ended there: keep the new one.
        self.keep_new_session(session, &held.token, &new_token)
    }

    /// The token and the key of the session in memory, when it is the one `session` names.
    fn held_session(&self, session: SessionGeneration) -> Result<Held, PasswordChangeError> {
        let held = {
            let guard = self.acct.session.lock().map_err(|_| PasswordChangeError::NotSignedIn)?;
            match guard.as_ref() {
                Some(current) if self.acct.session_generation() == session => Some(Held {
                    token: Zeroizing::new(current.token.clone()),
                    master_key: Zeroizing::new(current.master_key),
                }),
                Some(_) => return Err(PasswordChangeError::NotSignedIn),
                None => None,
            }
        };
        match held {
            Some(held) if self.acct.auth_health.is_expired() => {
                drop(held);
                Err(PasswordChangeError::SessionEnded)
            }
            Some(held) => Ok(held),
            None if self.auth_present.lock().map(|present| *present).unwrap_or(false) => {
                Err(PasswordChangeError::Locked)
            }
            None => Err(PasswordChangeError::NotSignedIn),
        }
    }

    fn note_session_refused(&self, session: SessionGeneration) {
        let _ = self.acct.while_session_is(&self.writes, session, |_| {
            self.acct
                .auth_health
                .note_result(Some(&anyhow::anyhow!("401 Unauthorized")));
        });
    }

    /// `GET /api/v1/onboarding` (no session): the numbers the server declares, floored by core.
    async fn signup_policy(&self, base: &str) -> Result<SignupPolicy, PasswordChangeError> {
        let response = auth_transport::send_to(base, &self.client, None, SignupPolicy::request())
            .await
            .map_err(|_| PasswordChangeError::Offline)?;
        if response.status == 429 {
            return Err(PasswordChangeError::RateLimited);
        }
        SignupPolicy::from_response(&response).map_err(|_| PasswordChangeError::Failed("policy"))
    }

    /// The breach check of `new`, answered, when the policy declares one; `None` otherwise. A call that fails is
    /// recorded as unavailable: core's gate then says whether the policy lets the change go on.
    async fn breach_check(&self, base: &str, policy: &SignupPolicy, new: &str) -> Option<BreachCheck> {
        let endpoint = policy.breach_endpoint.as_ref()?;
        let mut check = BreachCheck::new(new);
        let prefix = check.prefix().to_owned();
        let path = endpoint.replace("{prefix}", &prefix);
        let answer = self
            .client
            .get(format!("{base}{path}"))
            .timeout(WAIT)
            .send()
            .await
            .ok()
            .filter(|response| response.status().is_success());
        let body = match answer {
            Some(response) => response.text().await.ok().map(Zeroizing::new),
            None => None,
        };
        let response = match body.as_ref() {
            Some(text) => BreachResponse::Body(text.as_str()),
            None => BreachResponse::Unavailable,
        };
        if check.record(&prefix, response).is_err() {
            let _ = check.record(&prefix, BreachResponse::Unavailable);
        }
        Some(check)
    }

    /// One request of core's flow, on the session's token. The answer's body is parsed, never logged.
    async fn send_account(
        &self,
        base: &str,
        token: &str,
        request: &ApiRequest,
    ) -> Result<ApiResponse, PasswordChangeError> {
        let url = format!("{base}{}", request.path);
        let builder = match request.method {
            Method::Get => self.client.get(&url),
            Method::Post => self.client.post(&url),
            Method::Put => self.client.put(&url),
            Method::Delete => self.client.delete(&url),
            Method::Patch => self.client.patch(&url),
        };
        let mut builder = builder.timeout(WAIT).bearer_auth(token);
        if let Some(confirm) = request.confirm_token() {
            builder = builder.header("X-Confirm-Token", confirm);
        }
        if let Some(body) = request.body.as_ref() {
            builder = builder.json(body);
        }
        let response = builder.send().await.map_err(|_| PasswordChangeError::Offline)?;
        let status = response.status().as_u16();
        let text = Zeroizing::new(response.text().await.unwrap_or_default());
        Ok(ApiResponse::from_text(status, &text))
    }

    /// Store the new token and put it in memory, in one turn, only for the session the change was made for. The
    /// vault key and its owner tag are not touched. A credential store that refuses the write leaves the new token
    /// in memory only: this run works; the next start finds the old token ended and asks for the new password.
    fn keep_new_session(
        &self,
        session: SessionGeneration,
        old_token: &str,
        new_token: &str,
    ) -> Result<(), PasswordChangeError> {
        let _write = self.claim(session).map_err(|_| PasswordChangeError::NotSignedIn)?;
        let vault = self.vault();
        // Put back beside the new token whose session it is (`install_session` removes the old tag first).
        let owner = key_owner::session_owner_of(&vault, old_token);
        let stored = SessionToken::new(new_token.to_string())
            .and_then(|token| vault.install_session(token))
            .and_then(|()| match owner.as_deref() {
                Some(owner) => vault.bind_session_owner(owner),
                None => Ok(()),
            });
        if stored.is_err() {
            tracing::warn!("password changed; the new session could not be stored on this computer");
        }
        if let Ok(mut guard) = self.acct.session.lock()
            && let Some(current) = guard.as_mut()
        {
            current.token = new_token.to_string();
        }
        self.acct.auth_health.note_result(None);
        Ok(())
    }
}

#[cfg(test)]
#[path = "account_password_api_tests.rs"]
mod tests;
