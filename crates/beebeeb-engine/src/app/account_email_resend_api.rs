//! Changing the sign-in email, the parts after the first step (`account_email_api`): send the 8-digit code AGAIN
//! (`POST /api/v1/me/email/resend`), and, once Beebeeb took the code, the email this engine mirrors for the session.
//!
//! Every method BLOCKS on the handle's runtime: call it from a worker thread. Session rule as `account_actions_api`:
//! a call is for ONE session; another sends nothing.
//!
//! The resend is sent ONCE per press, by hand (`auth_transport::send_to`): never through a path that rides out a 429,
//! which would send it again silently and hide the wait the person must see. The two 429s are told apart by the
//! body's code: `email_code_resend_wait` carries `Retry-After`, `email_code_resend_limit` is final for this change.
//!
//! Mirror: after a confirmed change the address the server named replaces the one the session, the credential store
//! and the engine's account label hold, so the old address is never shown again. The account keeps its user id, so the
//! local data stays this account's. Only an address the caller got FROM the server is stored.
//!
//! Must not: log an address, a code or a server's words.

use serde_json::Value;

use beebeeb_core::auth_flow::{HttpRequest, Method};

use super::account_actions_api::ActionError;
use super::{CredentialStores, Engine};
use crate::account::SessionGeneration;
use crate::auth_transport;

/// What a resend did, in the server's words: the address the code went to, how long it lives, how many resends are
/// left for this change.
#[derive(Clone, PartialEq, Eq)]
pub struct EmailResent {
    pub pending_email: String,
    pub code_expires_in: u64,
    pub resends_left: u64,
}

// By hand: an address is the person's own.
impl std::fmt::Debug for EmailResent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmailResent").finish_non_exhaustive()
    }
}

/// Why a code was not sent again. Fixed words only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmailResendError {
    /// No session in memory, or not the session of the press: nothing was sent.
    NotSignedIn,
    /// Beebeeb no longer accepts this session (401).
    SessionEnded,
    /// Beebeeb could not be reached.
    Offline,
    /// No change is waiting for a code (404): it was confirmed, expired or never started.
    Gone,
    /// Too soon (429): seconds to wait, from `Retry-After`.
    Wait(u64),
    /// No more codes for this change (429): start again.
    Limit,
    /// Beebeeb refused, or answered something that does not read.
    Failed,
}

const RESEND_PATH: &str = "/api/v1/me/email/resend";
/// The wait when Beebeeb said to wait and not for how long.
const WAIT_FALLBACK_SECS: u64 = 30;

fn resend_request() -> HttpRequest {
    let mut request = HttpRequest::session_post(RESEND_PATH.to_string(), Value::Null);
    request.method = Method::Post;
    request.body = None;
    request
}

impl<C: CredentialStores> Engine<C> {
    /// WRITES: ask Beebeeb to send the 8-digit code again, for the session `session`. Once per call, never retried.
    pub fn email_change_resend(&self, session: SessionGeneration) -> Result<EmailResent, EmailResendError> {
        let (token, turn) = self.own_session().map_err(|error| match error {
            ActionError::SessionEnded => EmailResendError::SessionEnded,
            _ => EmailResendError::NotSignedIn,
        })?;
        if turn != session || !self.session_is_in_memory(session) {
            return Err(EmailResendError::NotSignedIn);
        }
        let base = self.config.api_base.trim_end_matches('/');
        let sent = self
            .runtime
            .block_on(auth_transport::send_to(
                base,
                &self.client,
                Some(&token),
                resend_request(),
            ))
            .map_err(|_| EmailResendError::Offline);
        let answer = sent.and_then(|response| match response.status {
            200..=299 => {
                let text = |key: &str| response.body.get(key).and_then(Value::as_str).map(str::to_string);
                let number = |key: &str| response.body.get(key).and_then(Value::as_u64);
                match (text("pending_email"), number("code_expires_in"), number("resends_left")) {
                    (Some(pending_email), Some(code_expires_in), Some(resends_left)) if !pending_email.is_empty() => {
                        Ok(EmailResent {
                            pending_email,
                            code_expires_in,
                            resends_left,
                        })
                    }
                    _ => Err(EmailResendError::Failed),
                }
            }
            401 => Err(EmailResendError::SessionEnded),
            404 => Err(EmailResendError::Gone),
            429 => match response.body.get("error").and_then(Value::as_str) {
                Some("email_code_resend_limit") => Err(EmailResendError::Limit),
                _ => Err(EmailResendError::Wait(
                    response.retry_after_secs.unwrap_or(WAIT_FALLBACK_SECS).max(1),
                )),
            },
            _ => Err(EmailResendError::Failed),
        });
        // Only a 401 advances the engine's run of 401s; an answer clears it.
        let failed = match answer {
            Err(EmailResendError::SessionEnded) => Some(ActionError::SessionEnded),
            _ => None,
        };
        self.note_action(session, failed);
        answer
    }

    /// The change of the sign-in email was confirmed: `new_email` (the address the SERVER named) replaces the one
    /// the session, the credential store and the engine's account label hold. `None`: not done (another session,
    /// or nothing to store); the old address then stays and follows at the next sign-in. Moves the session
    /// generation on, as every write of the session does: whatever the host holds for the old one is read again.
    /// `Some` is the generation this write moved to, so a host can tell its own write from a lock or a sign-in.
    pub fn email_changed_for(&self, session: SessionGeneration, new_email: &str) -> Option<SessionGeneration> {
        let email = new_email.trim();
        if email.is_empty() {
            return None;
        }
        let Ok(_write) = self.claim(session) else {
            return None;
        };
        // The generation the claim moved to, read under its write lock: no other transition is between.
        let moved_to = self.acct.session_generation();
        if let Err(error) = self.vault().store_account_email(email) {
            tracing::warn!(%error, "could not store the new account email (it follows at the next sign-in)");
        }
        if let Ok(mut label) = self.acct.auth_email.lock() {
            *label = Some(email.to_string());
        }
        if let Ok(mut current) = self.acct.session.lock()
            && let Some(held) = current.as_mut()
        {
            held.email = Some(email.to_string());
        }
        Some(moved_to)
    }
}

#[cfg(test)]
#[path = "account_email_resend_api_tests.rs"]
mod tests;
