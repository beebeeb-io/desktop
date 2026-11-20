//! Slice A6 (plan40 batch 1): deleting the account from this PC (DW-A58, DW-A80 to DW-A83): the deletion's state, the
//! request with the step-up and the second factor, the link's use, the cancel and the undo.
//!
//! The rules are core's and the server's; this file only sends, session-bound:
//! - The request is core's `AccountFlow::delete_account(password, code)`: core proves the password itself (its own
//!   step-up, OPAQUE, the password in no request) and spends that proof on `POST /api/v1/auth/account/delete-request`
//!   with the second factor. Beebeeb mails a single-use link; NOTHING is scheduled until that link is used. A wrong
//!   code changes nothing (the server consumes the step-up's token before it reads the code, so every try runs a new
//!   step-up, from the password the host kept for that).
//! - The state is `GET /api/v1/auth/account/deletion`: `none`, `requested` (a link is out), or `scheduled` with
//!   `shred_after` (when the account is deleted) and `cancel_until` (the last moment it can be undone).
//! - The cancel is the server's SIGNED-IN form of `POST /api/v1/auth/account/delete-cancel` (no link token in the
//!   body; a step-up's token in `X-Confirm-Token`). Core has no flow for that form, so the request is built here with
//!   core's `HttpRequest` and the token put on it by core (`ConfirmToken::attach`). Beebeeb cancels only a SCHEDULED
//!   deletion that is still inside `cancel_until`; a link that was not used yet cannot be stopped by any route
//!   (it expires by itself), and Beebeeb answers that as `delete_not_scheduled`.
//! - Core folds the server's 409s into one `Server(409)`: the two that mean something to the person (a plan that is
//!   still charged, a deletion that is already scheduled) are read from the answer's code before core sees it.
//!
//! Session-bound like its neighbours (`*_for(session)`): every call is asked for the session the host read at its
//! press, sends nothing for any other, and its answer is handed over only while that session is still the one in
//! memory ([`DeletionError::NotSignedIn`] otherwise: a late answer is dropped, also when the change was made). The
//! request asks again after its password step-up, right before the request that mails the link leaves: a lock, a
//! sign-out or another sign-in that landed during the step-up sends no link.
//! Every call takes at most [`DELETION_WITHIN`].
//!
//! Every method BLOCKS on the handle's runtime.
//!
//! Must not: log a password, a code, a token, an email or a server's words; keep the password past the call.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use beebeeb_core::account_flows::{
    AccountFlow, AccountFlowError, AccountOutcome, ApiRequest, ApiResponse, Method, PATH_DELETE_CANCEL, Step,
};
use beebeeb_core::auth_flow::{HttpRequest, HttpResponse, Method as AuthMethod};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::account::SessionGeneration;
use crate::app::account_actions_api::{ActionError, StepUpGrant};
use crate::app::{CredentialStores, Engine};
use crate::auth_transport;

/// The longest one call of this file takes, from its first request to its answer (a step-up is two requests, and
/// the request it guards a third). After it the call is answered as offline (or, when the guarded request had left,
/// as not known), never a window that waits for the network's own timeout.
pub const DELETION_WITHIN: Duration = Duration::from_secs(20);

/// `GET`: where the account's deletion stands.
pub const PATH_DELETION_STATE: &str = "/api/v1/auth/account/deletion";

/// Where the account's deletion stands, as Beebeeb answered it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeletionState {
    /// Nothing was asked, it was cancelled, or a link expired unused.
    None,
    /// A link went out and was not used: nothing is scheduled.
    Requested,
    /// Scheduled. `cancel_until`: the last moment it can be undone; `shred_after`: when the account is deleted.
    /// `None`: Beebeeb did not say (or said it in a form that does not read).
    Scheduled {
        shred_after: Option<DateTime<Utc>>,
        cancel_until: Option<DateTime<Utc>>,
    },
}

/// Why a call of this file did not do what it was asked. Nothing changed in any of these, except
/// [`DeletionError::Unconfirmed`] and a [`DeletionError::NotSignedIn`] that came after the request left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeletionError {
    /// No session in memory, or not the one the press was made under (also: it moved on while the call was out, and
    /// the answer was dropped).
    NotSignedIn,
    /// Beebeeb no longer accepts this session (401).
    SessionEnded,
    /// Beebeeb could not be reached, or did not answer in time, before anything that changes was sent.
    Offline,
    /// The password was not the account's (it counts toward the sign-in lockout).
    WrongPassword,
    /// Too many tries of the password: Beebeeb said to wait.
    TooManyTries,
    /// The second factor was wrong, or not a 6-digit or 8-digit code (then nothing was sent).
    WrongCode,
    /// The account has two-factor and no code went with the request.
    SecondFactorRequired,
    /// 409 `delete_subscription_active`: a plan is still charged; it is cancelled first. No link was sent.
    SubscriptionActive,
    /// 409 `delete_already_scheduled`: a deletion is scheduled already (a new request never resets it).
    AlreadyScheduled,
    /// 429 on the request: a link went out less than a minute ago.
    TooSoon,
    /// 502 `delete_email_failed`: the link's email was not accepted, so nothing was asked.
    EmailNotSent,
    /// The step-up's token was refused (spent, expired, for another account).
    NotConfirmed,
    /// 404 `delete_not_scheduled`: nothing that may still be cancelled (no scheduled deletion, a link not used yet,
    /// or the time to undo has passed).
    NotScheduled,
    /// The guarded request left and no answer came: it may have been taken.
    Unconfirmed,
    /// Beebeeb answered with an error, or an answer that did not read.
    Failed,
    /// Only for the request: the link WENT OUT (Beebeeb took it), and the session moved on while it was out (a lock, a
    /// sign-out, another sign-in), so the answer is dropped as a [`DeletionError::NotSignedIn`] would be. The caller
    /// decides, for the session and account it stamped, whether the person is told the link was sent.
    SentThenMoved,
}

impl DeletionError {
    /// A few words, for a log line (never a server's own words).
    pub fn words(self) -> &'static str {
        match self {
            Self::NotSignedIn => "not signed in",
            Self::SessionEnded => "the session ended",
            Self::Offline => "offline",
            Self::WrongPassword => "wrong password",
            Self::TooManyTries => "too many tries",
            Self::WrongCode => "wrong code",
            Self::SecondFactorRequired => "no code",
            Self::SubscriptionActive => "a plan is still charged",
            Self::AlreadyScheduled => "already scheduled",
            Self::TooSoon => "too soon",
            Self::EmailNotSent => "the email was not sent",
            Self::NotConfirmed => "not confirmed",
            Self::NotScheduled => "nothing scheduled",
            Self::Unconfirmed => "not known",
            Self::Failed => "not done",
            Self::SentThenMoved => "sent, then the session moved on",
        }
    }
}

fn from_action(error: ActionError) -> DeletionError {
    match error {
        ActionError::NotSignedIn => DeletionError::NotSignedIn,
        ActionError::SessionEnded => DeletionError::SessionEnded,
        ActionError::Offline => DeletionError::Offline,
        ActionError::WrongPassword => DeletionError::WrongPassword,
        ActionError::TooMany => DeletionError::TooManyTries,
        ActionError::Failed | ActionError::UnsavedEdits => DeletionError::Failed,
    }
}

fn from_flow(error: &AccountFlowError) -> DeletionError {
    match error {
        AccountFlowError::WrongPassword => DeletionError::WrongPassword,
        AccountFlowError::InvalidCode => DeletionError::WrongCode,
        AccountFlowError::SecondFactorRequired => DeletionError::SecondFactorRequired,
        AccountFlowError::Unauthorized => DeletionError::SessionEnded,
        AccountFlowError::RateLimited => DeletionError::TooManyTries,
        AccountFlowError::ConfirmationRequired => DeletionError::NotConfirmed,
        _ => DeletionError::Failed,
    }
}

/// The server's machine code of a refusal (`code`, else `error`).
fn code_of(body: &Value) -> Option<&str> {
    body.get("code").or_else(|| body.get("error")).and_then(Value::as_str)
}

/// The refusals of `delete-request` that core reads as one generic error, read here first.
fn request_refusal(response: &ApiResponse) -> Option<DeletionError> {
    match (response.status, code_of(&response.body)) {
        (409, Some("delete_subscription_active")) => Some(DeletionError::SubscriptionActive),
        (409, Some("delete_already_scheduled")) => Some(DeletionError::AlreadyScheduled),
        (429, _) => Some(DeletionError::TooSoon),
        (502, Some("delete_email_failed")) => Some(DeletionError::EmailNotSent),
        _ => None,
    }
}

/// A time Beebeeb sent as RFC 3339.
fn time_of(body: &Value, name: &str) -> Option<DateTime<Utc>> {
    body.get(name)
        .and_then(Value::as_str)
        .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
        .map(|at| at.with_timezone(&Utc))
}

/// The state read's answer.
fn state_of(response: &HttpResponse) -> Result<DeletionState, DeletionError> {
    match response.status {
        200..=299 => {}
        401 => return Err(DeletionError::SessionEnded),
        _ => return Err(DeletionError::Failed),
    }
    match response.body.get("state").and_then(Value::as_str) {
        Some("none") => Ok(DeletionState::None),
        Some("requested") => Ok(DeletionState::Requested),
        Some("scheduled") => Ok(DeletionState::Scheduled {
            shred_after: time_of(&response.body, "shred_after"),
            cancel_until: time_of(&response.body, "cancel_until"),
        }),
        _ => Err(DeletionError::Failed),
    }
}

/// The signed-in cancel's answer.
fn cancel_of(response: &HttpResponse) -> Result<(), DeletionError> {
    match response.status {
        200..=299 if response.body.get("cancelled").and_then(Value::as_bool) == Some(true) => Ok(()),
        404 if code_of(&response.body) == Some("delete_not_scheduled") => Err(DeletionError::NotScheduled),
        401 => Err(DeletionError::SessionEnded),
        403 => Err(DeletionError::NotConfirmed),
        429 => Err(DeletionError::TooManyTries),
        _ => Err(DeletionError::Failed),
    }
}

/// A GET on the signed-in session (core's `HttpRequest` has a POST constructor only).
fn session_get(path: &str) -> HttpRequest {
    let mut request = HttpRequest::session_post(path.to_string(), Value::Null);
    request.method = AuthMethod::Get;
    request.body = None;
    request
}

impl<C: CredentialStores> Engine<C> {
    /// READS: where the account's deletion stands (`GET /api/v1/auth/account/deletion`), for the session `session`.
    pub fn account_deletion_for(&self, session: SessionGeneration) -> Result<DeletionState, DeletionError> {
        let token = self.deletion_session(session)?;
        let request = session_get(PATH_DELETION_STATE);
        let answer = self.runtime.block_on(async {
            tokio::time::timeout(
                DELETION_WITHIN,
                auth_transport::send_to(&self.config.api_base, &self.client, Some(token.as_str()), request),
            )
            .await
        });
        let result = match answer {
            Ok(Ok(response)) => state_of(&response),
            Ok(Err(_)) | Err(_) => Err(DeletionError::Offline),
        };
        self.deletion_answered(session, result)
    }

    /// WRITES (spends a password attempt; mails a link): ask Beebeeb to delete the account, for the session
    /// `session`. Core's `AccountFlow::delete_account(password, code)`: its own step-up, then the request with the
    /// second factor (`code`: a 6-digit authenticator code or an 8-digit backup code; empty: none). `Ok`: the link
    /// went out; nothing is scheduled until it is used. A code core does not take as a code is
    /// [`DeletionError::WrongCode`] and nothing is sent. The password is wiped when the call ends.
    pub fn request_account_deletion_for(
        &self,
        session: SessionGeneration,
        password: Zeroizing<String>,
        code: &str,
    ) -> Result<(), DeletionError> {
        let code = code.trim();
        let flow = AccountFlow::delete_account(password.as_str(), (!code.is_empty()).then_some(code))
            .map_err(|error| from_flow(&error))?;
        drop(password);
        let token = self.deletion_session(session)?;
        let sent = AtomicBool::new(false);
        let result = self.runtime.block_on(async {
            match tokio::time::timeout(DELETION_WITHIN, self.drive_deletion(session, &token, flow, &sent)).await {
                Ok(result) => result,
                Err(_) if sent.load(Ordering::SeqCst) => Err(DeletionError::Unconfirmed),
                Err(_) => Err(DeletionError::Offline),
            }
        });
        let result = match result {
            Ok(AccountOutcome::DeletionRequested { .. }) => Ok(()),
            Ok(_) => Err(DeletionError::Failed),
            Err(error) => Err(error),
        };
        let went_out = result.is_ok();
        match self.deletion_answered(session, result) {
            Err(DeletionError::NotSignedIn) if went_out => Err(DeletionError::SentThenMoved),
            answered => answered,
        }
    }

    /// WRITES: cancel a SCHEDULED deletion from this session, with the step-up's token `grant` (the server's signed-in
    /// form of `POST /api/v1/auth/account/delete-cancel`). `Ok`: cancelled; Beebeeb mails the owner that it was.
    /// [`DeletionError::NotScheduled`]: nothing Beebeeb may still cancel (see the module's doc). The grant is spent
    /// by the request (or dropped unspent when nothing is sent).
    pub fn cancel_account_deletion_for(
        &self,
        session: SessionGeneration,
        grant: StepUpGrant,
    ) -> Result<(), DeletionError> {
        let token = self.deletion_session(session)?;
        // No link token in the body: the signed-in cancel. The step-up's token goes on this one request.
        let request = grant
            .into_token()
            .attach(HttpRequest::session_post(PATH_DELETE_CANCEL.to_string(), json!({})));
        let answer = self.runtime.block_on(async {
            tokio::time::timeout(
                DELETION_WITHIN,
                auth_transport::send_to(&self.config.api_base, &self.client, Some(token.as_str()), request),
            )
            .await
        });
        let result = match answer {
            Ok(Ok(response)) => cancel_of(&response),
            // It left and no answer came: Beebeeb may have cancelled it.
            Ok(Err(_)) | Err(_) => Err(DeletionError::Unconfirmed),
        };
        self.deletion_answered(session, result)
    }

    /// The session's token, when the session in memory is `session`.
    fn deletion_session(&self, session: SessionGeneration) -> Result<Zeroizing<String>, DeletionError> {
        let (token, turn) = self.own_session().map_err(from_action)?;
        if turn != session || !self.session_is_in_memory(session) {
            return Err(DeletionError::NotSignedIn);
        }
        Ok(token)
    }

    /// How a call ended, for the engine's run of 401s; an answer for a session that moved on is dropped.
    fn deletion_answered<T>(
        &self,
        session: SessionGeneration,
        result: Result<T, DeletionError>,
    ) -> Result<T, DeletionError> {
        let failed_as = match &result {
            Ok(_) => None,
            Err(DeletionError::SessionEnded) => Some(ActionError::SessionEnded),
            Err(_) => Some(ActionError::Failed),
        };
        if !self.note_action(session, failed_as) {
            return Err(DeletionError::NotSignedIn);
        }
        result
    }

    /// Run core's request flow: send what it asks, hand it the answer. `sent`: the guarded request has left. The
    /// guarded request (the one the step-up's token is spent on) leaves only while the session in memory is still
    /// `session` and a person's call may run.
    async fn drive_deletion(
        &self,
        session: SessionGeneration,
        token: &str,
        mut flow: AccountFlow,
        sent: &AtomicBool,
    ) -> Result<AccountOutcome, DeletionError> {
        let mut step = flow.start().map_err(|error| from_flow(&error))?;
        loop {
            match step {
                Step::Done(outcome) => return Ok(outcome),
                Step::Send(request) => {
                    // The first leg of a step-up carries no proof of the password: a 401 there is about the session.
                    let first_leg = request.path.ends_with("/confirm-opaque-start");
                    let guarded = request.confirm_token().is_some();
                    if guarded {
                        // A lock decided off the window's thread, a sign-out or another sign-in during the step-up:
                        // no link is mailed. Never waits (`deletion_session` only tries the session's locks).
                        self.deletion_session(session)?;
                        sent.store(true, Ordering::SeqCst);
                    }
                    let response = self.send_deletion_call(token, &request).await;
                    drop(request);
                    let response = match response {
                        Ok(response) => response,
                        Err(_) if guarded => return Err(DeletionError::Unconfirmed),
                        Err(error) => return Err(error),
                    };
                    if first_leg && response.status == 401 {
                        return Err(DeletionError::SessionEnded);
                    }
                    if guarded && let Some(refusal) = request_refusal(&response) {
                        return Err(refusal);
                    }
                    step = flow.resume(response).map_err(|error| from_flow(&error))?;
                }
            }
        }
    }

    /// One request of core's flow, on the session's token. The answer's body is parsed, never logged.
    async fn send_deletion_call(&self, token: &str, request: &ApiRequest) -> Result<ApiResponse, DeletionError> {
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
        let response = builder.send().await.map_err(|_| DeletionError::Offline)?;
        let status = response.status().as_u16();
        let text = Zeroizing::new(response.text().await.unwrap_or_default());
        Ok(ApiResponse::from_text(status, &text))
    }
}

#[cfg(test)]
#[path = "account_deletion_tests.rs"]
mod tests;
