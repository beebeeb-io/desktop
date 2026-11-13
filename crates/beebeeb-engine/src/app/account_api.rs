//! The account as Beebeeb holds it, for a host without a UI toolkit (the native Account and Devices and sessions
//! panes): who is signed in, the plan, the account's sessions, and ending a session of another device.
//!
//! Every method BLOCKS on the handle's runtime: call it from a worker thread, never from the UI thread.
//!
//! Session rule: the token and the session's generation leave the session's lock together. A read hands its answer
//! over only while the session in memory is still that one, asked with no session change half done
//! ([`Stamped::session`] names it, so a host can drop an answer that is not its window's). A change is sent only
//! for the session the host read its list under: another generation, or a session Beebeeb no longer accepts, sends
//! nothing.
//!
//! The vault key is never handed to a call here: none of them decrypts anything. A failure is an
//! [`AccountCallError`], a kind with fixed words. Must not: log anything a call sends or receives (an email, a
//! device's name, an id).
//!
//! Writes account state: [`Engine::revoke_account_session`] and [`Engine::revoke_other_account_sessions`].

use std::future::Future;

use super::settings_api::{AccountCallError, kind_of};
use super::{CredentialStores, Engine};
use crate::account::SessionGeneration;
use crate::account_dto::{AccountProfile, AccountSessionList, Subscription};
use crate::api_client::ApiClient;

/// An answer, and the session it was made for.
#[derive(Clone)]
pub struct Stamped<T> {
    pub session: SessionGeneration,
    pub value: T,
}

// By hand: an answer holds an email, a name or a device's name, which must not reach a log through `{:?}`.
impl<T> std::fmt::Debug for Stamped<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stamped")
            .field("session", &self.session)
            .finish_non_exhaustive()
    }
}

impl<C: CredentialStores> Engine<C> {
    /// Who is signed in (`GET /api/v1/auth/me`): the email, whether it is verified, whether two-factor is on.
    pub fn account_profile(&self) -> Result<Stamped<AccountProfile>, AccountCallError> {
        self.account_read(|api| async move { api.account_profile().await })
    }

    /// The name the account shows (`GET /api/v1/me/profile`); `None`: none was set.
    pub fn account_display_name(&self) -> Result<Stamped<Option<String>>, AccountCallError> {
        self.account_read(|api| async move { api.account_display_name().await })
    }

    /// The plan: its size, what is used, the billing period and its end (`GET /api/v1/billing/subscription`).
    pub fn account_subscription(&self) -> Result<Stamped<Subscription>, AccountCallError> {
        self.account_read(|api| async move { api.subscription().await })
    }

    /// Every session of the account, one row per session, this one marked (`GET /api/v1/account/sessions`).
    pub fn account_sessions(&self) -> Result<Stamped<AccountSessionList>, AccountCallError> {
        self.account_read(|api| async move { api.account_sessions().await })
    }

    /// WRITES: end one session of the account (`DELETE /api/v1/account/sessions/{id}`). `listed_under`: the
    /// session the host read its list under ([`Stamped::session`]); when it is not the session in memory now,
    /// nothing is sent ([`AccountCallError::NotSignedIn`]). A caller never passes this session's own id.
    pub fn revoke_account_session(&self, listed_under: SessionGeneration, id: &str) -> Result<(), AccountCallError> {
        let id = id.to_string();
        self.account_change(
            listed_under,
            move |api| async move { api.revoke_account_session(&id).await },
        )
    }

    /// WRITES: end every session of the account but this one
    /// (`POST /api/v1/account/sessions/revoke-all-others`); the answer is how many ended. `listed_under` as for
    /// [`Engine::revoke_account_session`].
    pub fn revoke_other_account_sessions(&self, listed_under: SessionGeneration) -> Result<i64, AccountCallError> {
        self.account_change(listed_under, |api| async move {
            api.revoke_other_sessions().await.map(|answer| answer.revoked)
        })
    }

    /// The token and the generation of the session in memory, taken under one lock. No session: `NotSignedIn`.
    /// A session Beebeeb no longer accepts: `SessionEnded`, and nothing is asked.
    fn token_of_this_session(&self) -> Result<(String, SessionGeneration), AccountCallError> {
        let stamped = {
            let guard = self.acct.session.lock().map_err(|_| AccountCallError::NotSignedIn)?;
            let current = guard.as_ref().ok_or(AccountCallError::NotSignedIn)?;
            (current.token.clone(), self.acct.session_generation())
        };
        if self.acct.auth_health.is_expired() {
            return Err(AccountCallError::SessionEnded);
        }
        Ok(stamped)
    }

    /// Whether the session in memory is, now, the one `generation` names: asked with no session change half done
    /// (the session-write lock is held for the question). Never waits: during a session change the answer is no.
    fn account_session_is_still(&self, generation: SessionGeneration) -> bool {
        self.acct.while_session_is(&self.writes, generation, |_| ()).is_some()
    }

    /// The one path of this file's reads: the answer is handed over only for the session it was asked with.
    fn account_read<T, F, Fut>(&self, call: F) -> Result<Stamped<T>, AccountCallError>
    where
        F: FnOnce(ApiClient) -> Fut,
        Fut: Future<Output = anyhow::Result<T>>,
    {
        let (token, session) = self.token_of_this_session()?;
        // No vault key: these calls decrypt nothing. The client wipes the token when it is dropped.
        let api = ApiClient::new(self.config.api_base.clone(), token, [0; 32]);
        let value = self.runtime.block_on(call(api)).map_err(|error| kind_of(&error))?;
        if !self.account_session_is_still(session) {
            return Err(AccountCallError::NotSignedIn);
        }
        Ok(Stamped { session, value })
    }

    /// The one path of this file's changes: sent only while the session in memory is `listed_under`.
    fn account_change<T, F, Fut>(&self, listed_under: SessionGeneration, call: F) -> Result<T, AccountCallError>
    where
        F: FnOnce(ApiClient) -> Fut,
        Fut: Future<Output = anyhow::Result<T>>,
    {
        let (token, session) = self.token_of_this_session()?;
        if session != listed_under || !self.account_session_is_still(listed_under) {
            return Err(AccountCallError::NotSignedIn);
        }
        let api = ApiClient::new(self.config.api_base.clone(), token, [0; 32]);
        self.runtime.block_on(call(api)).map_err(|error| kind_of(&error))
    }
}
