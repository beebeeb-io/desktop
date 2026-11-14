//! What a host's account screens do beyond reading (`super::account_api`): sign THIS app out, prove the password
//! for a guarded change (step-up), change the name the account shows, and the sign-in activity log with its switch
//! on. Every rule is core's (`auth_flow::step_up`, `account_flows`) or the engine's own sign-out
//! ([`Engine::sign_out_keeping_local_data`]); this file only sends, session-bound.
//!
//! Every method BLOCKS on the handle's runtime: call it from a worker thread, never from the UI thread.
//!
//! Session rule: each call is made for the session ([`SessionGeneration`]) the host's press was made under. A call
//! for a session that is not the one in memory (any more) sends nothing ([`ActionError::NotSignedIn`]). Calls that
//! were sent note how they ended in the engine's run of 401s, as `account_api` does.
//!
//! Sign-out keeps the local files and the local state (nothing on disk is deleted, the state stays bound to this
//! account), so changes that were not sent yet stay on this PC and are sent after the next sign-in of the same
//! account; another account cannot sync into that state.
//!
//! Must not: log a password, a token, a name or a server's words; keep the password past the call.

use std::time::Duration;

use beebeeb_core::account_flows::{
    AccountFlow, AccountFlowError, AccountOutcome, ApiRequest, ApiResponse, Method, Step,
};
use beebeeb_core::auth_flow::step_up::{ConfirmToken, StepUp, StepUpPurpose, StepUpStep};
use zeroize::Zeroizing;

use super::account_api::Stamped;
use super::settings_api::{AccountCallError, kind_of};
use super::{CredentialStores, Engine};
use crate::account::SessionGeneration;
use crate::api_client::{ApiClient, MyActivity};
use crate::auth_transport;
use crate::signin::flow_code;
use crate::signin::gate::{SignInProblem, problem_from};

/// How long a password proof may take, from the first request to the confirmation: two requests of the OPAQUE
/// exchange. A host that cannot wait for the engine (it asks from a thread that must not stall) gets
/// [`ActionError::Offline`] after this, never a window that waits for the network's own timeout.
pub const STEP_UP_WITHIN: Duration = Duration::from_secs(12);

/// How many events the activity read asks for.
pub const ACTIVITY_LIMIT: u32 = 50;

/// What Beebeeb gave for a proven password: single use, short lived, bound to the session and account it was made
/// for. A host holds it and hands it back to the engine call it guards ([`StepUpGrant::into_token`], for the engine's
/// own calls); it cannot be read, copied or printed.
pub struct StepUpGrant(ConfirmToken);

impl StepUpGrant {
    /// The core token, for an engine call that sends it with the one request it guards.
    pub fn into_token(self) -> ConfirmToken {
        self.0
    }
}

impl std::fmt::Debug for StepUpGrant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StepUpGrant(<redacted>)")
    }
}

/// Why a call of this file did not do what it was asked. Nothing changed in any of these, except where the call's
/// own doc says a request may have been sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionError {
    /// No session in memory, or not the one the press was made under: nothing was sent.
    NotSignedIn,
    /// Beebeeb no longer accepts this session (401).
    SessionEnded,
    /// Beebeeb could not be reached, or did not answer in time.
    Offline,
    /// The password was not the account's (it counts toward the sign-in lockout).
    WrongPassword,
    /// Beebeeb said to wait: too many tries.
    TooMany,
    /// Beebeeb answered with an error, an answer that did not read, or the engine did not finish (sign-out).
    Failed,
}

impl ActionError {
    /// A few words, for a log line (never a server's own words).
    pub fn words(self) -> &'static str {
        match self {
            Self::NotSignedIn => "not signed in",
            Self::SessionEnded => "the session ended",
            Self::Offline => "offline",
            Self::WrongPassword => "wrong password",
            Self::TooMany => "too many tries",
            Self::Failed => "not done",
        }
    }
}

fn from_call(error: AccountCallError) -> ActionError {
    match error {
        AccountCallError::NotSignedIn => ActionError::NotSignedIn,
        AccountCallError::Offline => ActionError::Offline,
        AccountCallError::SessionEnded => ActionError::SessionEnded,
        AccountCallError::ServerError | AccountCallError::Unreadable => ActionError::Failed,
    }
}

fn from_flow(error: AccountFlowError) -> ActionError {
    match error {
        AccountFlowError::WrongPassword => ActionError::WrongPassword,
        AccountFlowError::Unauthorized => ActionError::SessionEnded,
        AccountFlowError::RateLimited => ActionError::TooMany,
        _ => ActionError::Failed,
    }
}

impl<C: CredentialStores> Engine<C> {
    /// WRITES: sign this app out of the session `session`, keeping the local files and the local state: the sync
    /// engine stops, Beebeeb is told to end the session (best effort), the token and the key leave the credential
    /// store and memory ([`Engine::sign_out_keeping_local_data`]). The check and the sign-out are ONE hold of the
    /// engine's step lock: a session that moved on since the press (another sign-in, another account) is never
    /// signed out by a press made under the one before ([`ActionError::NotSignedIn`], nothing done).
    /// [`ActionError::Failed`]: the engine did not confirm its stop or the credential store could not be cleared;
    /// still signed in.
    pub fn sign_out_for(&self, session: SessionGeneration) -> Result<(), ActionError> {
        let result = self.runtime.block_on(async {
            let _step = self.transition.lock().await;
            if !self.session_is_in_memory(session) {
                return Err(ActionError::NotSignedIn);
            }
            self.sign_out_holding_step().await.map_err(|_| ActionError::Failed)
        });
        self.publish();
        result
    }

    /// WRITES (spends a password attempt): prove the account's password to Beebeeb for the session `session`, by
    /// core's step-up (OPAQUE; the password is in no request). `Ok`: the confirmation Beebeeb minted (single use,
    /// short lived: whoever holds it spends it on the one request it guards). A wrong password counts toward the
    /// sign-in lockout. Takes at most [`STEP_UP_WITHIN`].
    pub fn step_up_for(
        &self,
        session: SessionGeneration,
        password: Zeroizing<String>,
    ) -> Result<StepUpGrant, ActionError> {
        let (token, turn) = self.own_session()?;
        if turn != session || !self.session_is_in_memory(session) {
            return Err(ActionError::NotSignedIn);
        }
        let result = self.runtime.block_on(async {
            tokio::time::timeout(STEP_UP_WITHIN, self.confirm_password(&token, &password))
                .await
                .unwrap_or(Err(ActionError::Offline))
        });
        drop(password);
        // Only a 401 on the first request is the session's; a wrong password is not.
        self.note_action(
            session,
            result
                .as_ref()
                .err()
                .copied()
                .filter(|e| *e == ActionError::SessionEnded),
        );
        result
    }

    /// WRITES: set the name the account shows (`PUT /api/v1/me/profile`, core's `AccountFlow::set_display_name`:
    /// trimmed, 1 to 64 characters; a name core refuses is [`ActionError::Failed`] and nothing is sent).
    pub fn set_display_name_for(&self, session: SessionGeneration, name: &str) -> Result<(), ActionError> {
        let flow = AccountFlow::set_display_name(name).map_err(|_| ActionError::Failed)?;
        self.change_for(session, flow)
    }

    /// WRITES: turn the sign-in activity log on (`PUT /api/v1/me/tracking`, core's
    /// `AccountFlow::set_activity_logging(true, None)`): a new log starts at once; no password is asked.
    pub fn start_activity_log_for(&self, session: SessionGeneration) -> Result<(), ActionError> {
        let flow = AccountFlow::set_activity_logging(true, None).map_err(from_flow)?;
        self.change_for(session, flow)
    }

    /// The sign-in activity log: whether the account records it and, when it does, the newest events
    /// (`GET /api/v1/me/activity`).
    pub fn activity_log(&self) -> Result<Stamped<MyActivity>, ActionError> {
        let (mut token, session) = self.own_session()?;
        let api = ApiClient::new(self.config.api_base.clone(), std::mem::take(&mut *token), [0; 32]);
        let answer = self
            .runtime
            .block_on(async { api.my_activity(ACTIVITY_LIMIT).await })
            .map_err(|error| from_call(kind_of(&error)));
        if !self.note_action(session, answer.as_ref().err().copied()) {
            return Err(ActionError::NotSignedIn);
        }
        answer.map(|value| Stamped { session, value })
    }

    /// Whether the session in memory is, now, the one `session` names (asked with no session change half done).
    fn session_is_in_memory(&self, session: SessionGeneration) -> bool {
        self.acct.while_session_is(&self.writes, session, |_| ()).is_some()
    }

    /// The token and the generation of the session in memory, taken under one lock. The copy wipes itself.
    fn own_session(&self) -> Result<(Zeroizing<String>, SessionGeneration), ActionError> {
        let pair = {
            let guard = self.acct.session.lock().map_err(|_| ActionError::NotSignedIn)?;
            let current = guard.as_ref().ok_or(ActionError::NotSignedIn)?;
            (Zeroizing::new(current.token.clone()), self.acct.session_generation())
        };
        if self.acct.auth_health.is_expired() {
            return Err(ActionError::SessionEnded);
        }
        Ok(pair)
    }

    /// How one call ended, for the engine's run of 401s: a 401 advances it, an answer clears it, any other failure
    /// leaves it. Only while the session in memory is still `session`; `false`: it moved on, nothing was noted.
    fn note_action(&self, session: SessionGeneration, failed_as: Option<ActionError>) -> bool {
        self.acct
            .while_session_is(&self.writes, session, |_| match failed_as {
                None => self.acct.auth_health.note_result(None),
                Some(ActionError::SessionEnded) => {
                    let refusal = anyhow::anyhow!("401 Unauthorized");
                    self.acct.auth_health.note_result(Some(&refusal));
                }
                Some(_) => {}
            })
            .is_some()
    }

    /// One change by one core flow, sent only while the session in memory is `session`.
    fn change_for(&self, session: SessionGeneration, flow: AccountFlow) -> Result<(), ActionError> {
        let (token, turn) = self.own_session()?;
        if turn != session || !self.session_is_in_memory(session) {
            return Err(ActionError::NotSignedIn);
        }
        let result = self
            .runtime
            .block_on(self.drive(&token, flow))
            .and_then(|outcome| match outcome {
                AccountOutcome::Done => Ok(()),
                _ => Err(ActionError::Failed),
            });
        self.note_action(session, result.as_ref().err().copied());
        result
    }

    /// Run a core flow to its end: send what it asks, hand it the answer.
    async fn drive(&self, token: &str, mut flow: AccountFlow) -> Result<AccountOutcome, ActionError> {
        let mut step = flow.start().map_err(from_flow)?;
        loop {
            match step {
                Step::Done(outcome) => return Ok(outcome),
                Step::Send(request) => {
                    let response = self.send_account(token, &request).await?;
                    drop(request);
                    step = flow.resume(response).map_err(from_flow)?;
                }
            }
        }
    }

    /// One request of a core flow: the session's token as bearer, the confirmation as its header, the body as
    /// JSON. A body is never logged.
    async fn send_account(&self, token: &str, request: &ApiRequest) -> Result<ApiResponse, ActionError> {
        let url = format!("{}{}", self.config.api_base, request.path);
        let mut builder = match request.method {
            Method::Get => self.client.get(&url),
            Method::Post => self.client.post(&url),
            Method::Put => self.client.put(&url),
            Method::Delete => self.client.delete(&url),
            Method::Patch => self.client.patch(&url),
        }
        .bearer_auth(token);
        if let Some(confirmation) = request.confirm_token() {
            builder = builder.header("X-Confirm-Token", confirmation);
        }
        if let Some(body) = request.body.as_ref() {
            builder = builder.json(body);
        }
        let response = builder.send().await.map_err(|_| ActionError::Offline)?;
        let status = response.status().as_u16();
        let text = Zeroizing::new(response.text().await.unwrap_or_default());
        Ok(ApiResponse::from_text(status, &text))
    }

    /// Core's step-up for the session whose token this is: both legs, the confirmation at the end.
    async fn confirm_password(&self, token: &str, password: &str) -> Result<ConfirmToken, ActionError> {
        let (mut flow, mut request) =
            StepUp::start(password.as_bytes(), StepUpPurpose::Generic).map_err(|_| ActionError::WrongPassword)?;
        let mut first_leg = true;
        loop {
            let response = auth_transport::send_to(&self.config.api_base, &self.client, Some(token), request)
                .await
                .map_err(|_| ActionError::Offline)?;
            // The first leg carries no proof of the password: a 401 there is about the session.
            if first_leg && response.status == 401 {
                return Err(ActionError::SessionEnded);
            }
            first_leg = false;
            match flow.on_response(response) {
                Ok(StepUpStep::Send(next)) => request = next,
                Ok(StepUpStep::Confirmed(grant)) => return Ok(grant),
                Err(error) => {
                    return Err(match problem_from(&flow_code(&error), 0) {
                        SignInProblem::Invalid | SignInProblem::LockedHint => ActionError::WrongPassword,
                        SignInProblem::Offline => ActionError::Offline,
                        SignInProblem::RateLimited { .. } => ActionError::TooMany,
                        _ => ActionError::Failed,
                    });
                }
            }
        }
    }
}
