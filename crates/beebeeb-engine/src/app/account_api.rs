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
//! One rule for "Beebeeb no longer accepts this session", for the engine's own loop and for these calls: every
//! call here that was sent notes how it ended in the engine's run of 401s (`AuthHealth`), once per call and only
//! while the session is still the one it was asked with. One 401 is a call that failed; a run of them makes the
//! engine hold the session as ended ([`Engine::account_session_refused`]), and then nothing more is asked
//! ([`AccountCallError::SessionEnded`] without a request); a call that works clears the run. A host draws "your
//! session ended" from that one flag, never from a single answer.
//!
//! The vault key is never handed to a call here: none of them decrypts anything. A failure is an
//! [`AccountCallError`], a kind with fixed words. Must not: log anything a call sends or receives (an email, a
//! device's name, an id).
//!
//! Writes account state: [`Engine::revoke_account_session`], [`Engine::end_account_session`] and
//! [`Engine::revoke_other_account_sessions`].

use std::future::Future;

use zeroize::Zeroizing;

use super::settings_api::{AccountCallError, kind_of};
use super::{CredentialStores, Engine};
use crate::account::SessionGeneration;
use crate::account_dto::{AccountProfile, AccountSessionList, Subscription};
use crate::api_client::{ApiClient, DesktopApiError};

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

/// What Account draws, read in one call ([`Engine::account_basics`]). The profile is the answer itself; the name
/// and the plan each carry their own failure, so a host can draw the rest without them. No `Debug`: it holds the
/// email and the name.
#[derive(Clone)]
pub struct AccountBasics {
    pub profile: AccountProfile,
    /// `Ok(None)`: no name was set.
    pub display_name: Result<Option<String>, AccountCallError>,
    pub subscription: Result<Subscription, AccountCallError>,
}

/// How a sign-out of one session ended ([`Engine::end_account_session`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEnd {
    /// Beebeeb ended the session.
    Ended,
    /// Beebeeb has no such session of this account (it answered 404): it had ended before.
    AlreadyGone,
}

/// Whether `error` is Beebeeb answering 404.
fn is_not_found(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<DesktopApiError>(),
            Some(DesktopApiError::HttpStatus { status, .. }) if status.as_u16() == 404
        )
    })
}

impl<C: CredentialStores> Engine<C> {
    /// Everything Account draws, asked at the same time on ONE client under one session check: who is signed in
    /// (`GET /api/v1/auth/me`), the name the account shows (`GET /api/v1/me/profile`) and the plan
    /// (`GET /api/v1/billing/subscription`). A profile that does not answer fails the call; the other two fail
    /// alone, each with its kind.
    ///
    /// The three requests are ONE call for the run of 401s: a 401 on any of them is one refusal (and the call
    /// answers [`AccountCallError::SessionEnded`], with nothing of the account), never three.
    pub fn account_basics(&self) -> Result<Stamped<AccountBasics>, AccountCallError> {
        self.account_read_noting(
            |api| async move {
                let (profile, display_name, subscription) =
                    futures_util::future::join3(api.account_profile(), api.account_display_name(), api.subscription())
                        .await;
                Ok(AccountBasics {
                    profile: profile?,
                    display_name: display_name.map_err(|error| kind_of(&error)),
                    subscription: subscription.map_err(|error| kind_of(&error)),
                })
            },
            |basics| {
                basics.display_name == Err(AccountCallError::SessionEnded)
                    || matches!(basics.subscription, Err(AccountCallError::SessionEnded))
            },
        )
    }

    /// Whether the engine holds this session as one Beebeeb no longer accepts (its calls, this file's among them,
    /// were answered 401 again and again). It is cleared by the engine's next call that works. In memory: a
    /// window may ask on every poll.
    pub fn account_session_refused(&self) -> bool {
        self.acct.auth_health.is_expired()
    }

    /// Test builds only: record how one of the engine's own calls ended, as its sync loop does. `refused`: Beebeeb
    /// answered it 401 (three in a row and the session is held as ended); else it worked, which clears that.
    #[cfg(any(test, feature = "test-support"))]
    pub fn note_call_for_test(&self, refused: bool) {
        let refusal = anyhow::anyhow!("401 Unauthorized");
        self.acct.auth_health.note_result(refused.then_some(&refusal));
    }

    /// WRITES: end one session of the account, the one path of [`Engine::revoke_account_session`], with a 404
    /// told apart: the session had ended before, so there is nothing left to end ([`SessionEnd::AlreadyGone`]).
    pub fn end_account_session(
        &self,
        listed_under: SessionGeneration,
        id: &str,
    ) -> Result<SessionEnd, AccountCallError> {
        let id = id.to_string();
        self.account_change(listed_under, move |api| async move {
            match api.revoke_account_session(&id).await {
                Ok(()) => Ok(SessionEnd::Ended),
                Err(error) if is_not_found(&error) => Ok(SessionEnd::AlreadyGone),
                Err(error) => Err(error),
            }
        })
    }

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
    /// nothing is sent ([`AccountCallError::NotSignedIn`]). A caller never passes this session's own id. A
    /// session that had ended before (404) is `Ok`: there is nothing left to end; [`Engine::end_account_session`]
    /// tells the two apart.
    pub fn revoke_account_session(&self, listed_under: SessionGeneration, id: &str) -> Result<(), AccountCallError> {
        self.end_account_session(listed_under, id).map(|_| ())
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
    /// A session Beebeeb no longer accepts: `SessionEnded`, and nothing is asked. The copy of the token wipes
    /// itself when it is dropped: a refusal after this point leaves none behind.
    fn token_of_this_session(&self) -> Result<(Zeroizing<String>, SessionGeneration), AccountCallError> {
        let stamped = self
            .person_session(|session, generation| (Zeroizing::new(session.token.clone()), generation))
            .map_err(|_| AccountCallError::NotSignedIn)?;
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

    /// How one call of this file ended, for the engine's run of 401s: a 401 advances it, an answer clears it, any
    /// other failure (offline, a 5xx, an answer that does not read) leaves it as it is. Decided from the call's
    /// kind, never from an error's text.
    fn note_account_call(&self, failed_as: Option<AccountCallError>) {
        match failed_as {
            None => self.acct.auth_health.note_result(None),
            Some(AccountCallError::SessionEnded) => {
                let refusal = anyhow::anyhow!("401 Unauthorized");
                self.acct.auth_health.note_result(Some(&refusal));
            }
            Some(_) => {}
        }
    }

    /// [`Engine::note_account_call`], only while the session in memory is still `session` (asked with no session
    /// change half done): a late answer of an earlier session never counts for, or clears, the next one's run.
    /// `false`: the session moved on, and nothing was noted.
    fn note_for_session(&self, session: SessionGeneration, failed_as: Option<AccountCallError>) -> bool {
        self.acct
            .while_session_is(&self.writes, session, |_| self.note_account_call(failed_as))
            .is_some()
    }

    /// The one path of this file's reads: the answer is handed over only for the session it was asked with.
    fn account_read<T, F, Fut>(&self, call: F) -> Result<Stamped<T>, AccountCallError>
    where
        F: FnOnce(ApiClient) -> Fut,
        Fut: Future<Output = anyhow::Result<T>>,
    {
        self.account_read_noting(call, |_| false)
    }

    /// [`Engine::account_read`] for a call of several requests: `refused` says whether a part of an answer was a
    /// 401, which makes the whole call one refusal.
    fn account_read_noting<T, F, Fut>(
        &self,
        call: F,
        refused: impl FnOnce(&T) -> bool,
    ) -> Result<Stamped<T>, AccountCallError>
    where
        F: FnOnce(ApiClient) -> Fut,
        Fut: Future<Output = anyhow::Result<T>>,
    {
        let (mut token, session) = self.token_of_this_session()?;
        // No vault key: these calls decrypt nothing. The token is moved (not copied) into the client, which wipes
        // it when it is dropped.
        let api = ApiClient::new(self.config.api_base.clone(), std::mem::take(&mut *token), [0; 32]);
        let answer = match self.runtime.block_on(call(api)) {
            Ok(value) if refused(&value) => Err(AccountCallError::SessionEnded),
            Ok(value) => Ok(value),
            Err(error) => Err(kind_of(&error)),
        };
        if !self.note_for_session(session, answer.as_ref().err().copied()) {
            return Err(AccountCallError::NotSignedIn);
        }
        answer.map(|value| Stamped { session, value })
    }

    /// The one path of this file's changes: sent only while the session in memory is `listed_under`.
    fn account_change<T, F, Fut>(&self, listed_under: SessionGeneration, call: F) -> Result<T, AccountCallError>
    where
        F: FnOnce(ApiClient) -> Fut,
        Fut: Future<Output = anyhow::Result<T>>,
    {
        let (mut token, session) = self.token_of_this_session()?;
        if session != listed_under || !self.account_session_is_still(listed_under) {
            return Err(AccountCallError::NotSignedIn);
        }
        let api = ApiClient::new(self.config.api_base.clone(), std::mem::take(&mut *token), [0; 32]);
        let answer = self.runtime.block_on(call(api)).map_err(|error| kind_of(&error));
        // How the change ended is the caller's whatever the session did since; only the run of 401s is not
        // touched for a session that moved on.
        self.note_for_session(listed_under, answer.as_ref().err().copied());
        answer
    }
}
