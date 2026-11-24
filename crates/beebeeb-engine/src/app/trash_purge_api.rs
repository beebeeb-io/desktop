//! Delete from the account's Trash for good, for a host without a UI toolkit (the native Trash window): "Empty Trash"
//! and "Delete permanently". The account PASSWORD is proven to the server first (core's OPAQUE step-up); the single-use
//! confirmation token that proof mints is spent on the permanent delete of exactly ONE item
//! (`DELETE /api/v1/files/{id}/permanent`), so an item costs one proof. Core puts the token on the request itself:
//! this module never holds it as a string.
//!
//! Session rule (as [`Engine::restore_items_for`]): bound to the session of the press. Before EACH item the session's
//! token is copied only while the session in memory is still that one and the screen is not locked, asked as a
//! person's call (`person_session`, under the session-write lock); a session that moved on since (a lock, a sign-out,
//! another sign-in), or a locked screen (also for a turn read while it was locked), is [`PurgeError::NotSignedIn`],
//! and nothing (more) is sent. Stops at the first item that fails: the ones before it are
//! gone for good, and [`Purged::done`] says how many.
//!
//! [`Engine::purge_trash_for`] BLOCKS on the handle's runtime: call it from a worker thread, never the UI thread.
//!
//! Must not: log the password, a token, an id or a server's words; delete anything that is not an id the caller named;
//! retry with the same token; fall back to a call without the token.

use beebeeb_core::auth_flow::step_up::{StepUp, StepUpPurpose, StepUpStep};
use beebeeb_core::auth_flow::{AuthFlowError, HttpRequest, Method};

use super::{CredentialStores, Engine};
use crate::account::SessionGeneration;
use crate::auth_transport;
use zeroize::Zeroizing;

/// Why an item was not deleted for good. Fixed words: never a server's body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PurgeError {
    /// No session in memory, or not the session of the press: nothing was sent.
    NotSignedIn,
    /// Beebeeb no longer accepts this session.
    SessionEnded,
    /// The password was empty or not the account's.
    WrongPassword,
    /// Too many tries; seconds to wait.
    RateLimited(u64),
    /// Beebeeb could not be reached: nothing was sent that deletes (the proof did not go through).
    Offline,
    /// The delete request LEFT and no answer came back (the connection dropped after it was sent): the item may be
    /// gone for good. Never worded as "nothing was deleted".
    NotConfirmed,
    /// The id is not a file id, or Beebeeb refused or answered something else.
    Failed,
}

/// What a purge got through: how many items are gone for good, and why it stopped (if it did).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Purged {
    pub done: usize,
    pub error: Option<PurgeError>,
}

/// A server file id as the listing gives it: a canonical lower-case UUID, never normalized (it goes into a URL).
pub(super) fn is_file_id(id: &str) -> bool {
    matches!(uuid::Uuid::parse_str(id), Ok(u) if u.hyphenated().to_string() == id)
}

pub(super) fn step_up_error(e: AuthFlowError) -> PurgeError {
    match e {
        AuthFlowError::InvalidCredentials | AuthFlowError::InvalidInput(_) => PurgeError::WrongPassword,
        AuthFlowError::RateLimited { retry_after_secs } => PurgeError::RateLimited(retry_after_secs),
        _ => PurgeError::Failed,
    }
}

fn permanent_request(id: &str) -> HttpRequest {
    let mut req = HttpRequest::session_post(format!("/api/v1/files/{id}/permanent"), serde_json::Value::Null);
    req.method = Method::Delete;
    req.body = None;
    req
}

/// Prove `password` for the session `token`: core's OPAQUE step-up, which mints ONE single-use confirmation token.
pub(super) async fn prove(
    api_base: &str,
    client: &reqwest::Client,
    token: &str,
    password: &[u8],
) -> Result<beebeeb_core::auth_flow::step_up::ConfirmToken, PurgeError> {
    let (mut flow, mut req) = StepUp::start(password, StepUpPurpose::Generic).map_err(step_up_error)?;
    let mut first_leg = true;
    loop {
        let resp = auth_transport::send_to(api_base, client, Some(token), req)
            .await
            .map_err(|_| PurgeError::Offline)?;
        // The first leg carries no proof of the password: a 401 there is about the session, not the password.
        if first_leg && resp.status == 401 {
            return Err(PurgeError::SessionEnded);
        }
        first_leg = false;
        match flow.on_response(resp).map_err(step_up_error)? {
            StepUpStep::Send(next) => req = next,
            StepUpStep::Confirmed(grant) => return Ok(grant),
        }
    }
}

/// Prove `password` for the session `token`, then delete `file_id` for good with the one token the proof minted.
/// `api_base` has no trailing slash. A 404 is "already gone": done.
pub async fn purge_one(
    api_base: &str,
    client: &reqwest::Client,
    token: &str,
    file_id: &str,
    password: &[u8],
) -> Result<(), PurgeError> {
    if !is_file_id(file_id) {
        return Err(PurgeError::Failed);
    }
    let grant = prove(api_base, client, token, password).await?;
    let resp = auth_transport::send_to(api_base, client, Some(token), grant.attach(permanent_request(file_id)))
        .await
        .map_err(|_| PurgeError::NotConfirmed)?;
    match resp.status {
        200..=299 | 404 => Ok(()),
        401 => Err(PurgeError::SessionEnded),
        429 => Err(PurgeError::RateLimited(resp.retry_after_secs.unwrap_or(30))),
        // A gateway that answers for a server that may have done it: as unknown as a dropped connection.
        502..=504 => Err(PurgeError::NotConfirmed),
        _ => Err(PurgeError::Failed),
    }
}

impl<C: CredentialStores> Engine<C> {
    /// WRITES, for good: delete `file_ids` from Trash one after the other, each after proving `password`, for the
    /// session a person pressed under and no other (`session` is [`Engine::session_turn`] as the host read it AT THE
    /// PRESS). `progress` is called with the number gone after each item. See the module's doc.
    ///
    /// BLOCKS on the handle's runtime: call it from a worker thread.
    pub fn purge_trash_for(
        &self,
        session: SessionGeneration,
        file_ids: &[String],
        password: &[u8],
        mut progress: impl FnMut(usize),
    ) -> Purged {
        let mut done = 0;
        for id in file_ids {
            // A session mutex a panic poisoned: nothing is sent.
            let token = if self.acct.session.is_poisoned() {
                None
            } else {
                self.person_session(|current, generation| {
                    (generation == session).then(|| Zeroizing::new(current.token.clone()))
                })
                .ok()
                .flatten()
            };
            let Some(token) = token else {
                return self.purged(done, Some(PurgeError::NotSignedIn));
            };
            let base = self.config.api_base.trim_end_matches('/').to_string();
            let sent = self
                .runtime
                .block_on(purge_one(&base, &self.client, token.as_str(), id, password));
            match sent {
                Ok(()) => {
                    done += 1;
                    progress(done);
                }
                Err(error) => return self.purged(done, Some(error)),
            }
        }
        self.purged(done, None)
    }

    /// Ask the sync engine to take the server's list again when anything was deleted (also when a later item failed),
    /// and when the last delete is not confirmed (it may have gone through).
    fn purged(&self, done: usize, error: Option<PurgeError>) -> Purged {
        if done > 0 || error == Some(PurgeError::NotConfirmed) {
            self.request_resnapshot();
        }
        Purged { done, error }
    }
}

#[cfg(test)]
#[path = "trash_purge_api_tests.rs"]
mod tests;
