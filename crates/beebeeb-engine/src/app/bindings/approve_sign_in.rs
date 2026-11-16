//! Slice A5 (plan40 batch 1): approving a sign-in from another device (DW-A12 to DW-A17): the request a code names, the
//! approval that seals the keys to the other device, and the denial.
//!
//! The rules and the crypto are core's (`beebeeb_core::auth_flow::device`). The lookup is `Approval::lookup`
//! (`GET /api/v1/auth/cli-pubkey`, on the session). The approval is `PendingApproval::approve`: a password step-up of
//! purpose `cli_device_approval` bound to the code, then `POST /api/v1/auth/cli-session` (the claim, answered with
//! the new device's session token) and `POST /api/v1/auth/cli-authorize` (the relay of `{session_token,
//! master_key_b64}`, sealed by core to the other device's key: Beebeeb sees ciphertext only). The denial is
//! `Approval::deny` (`POST /api/v1/auth/cli-deny`). This file only sends core's requests on the session's token and
//! says how each ended. The `device_link` routes are another channel and are not used.
//!
//! Session-bound like its neighbours (`*_for(session)`): every call is asked for the session the host read at its
//! press. The token (and for the approval the master key) is copied only while the session in memory is that one and
//! the screen is not locked, asked as a person's call (`person_session`): a locked screen, a closed vault, another
//! session or none sends nothing. The approval asks again right before the claim leaves and again right before the
//! relay leaves: a lock or a sign-out during the step-up claims nothing, and one during the claim's round trip
//! relays nothing (the keys stay on this PC).
//!
//! The point of no return is the RELAY, not the claim. Until the relay leaves, the keys have not left this PC
//! whatever else happened: a claim whose answer was lost minted at most a session that nobody can use (the other
//! device only gets its token inside the sealed relay). So every failure before the relay leaves is "nothing was
//! approved". Once it has left, only a 2xx (approved), a 404 (the request was gone: not delivered) or a 401 (the
//! session was refused at the door: not taken) says how it ended; no answer, a server error or anything else is
//! [`ApproveError::Unknown`].
//!
//! Every method BLOCKS on the handle's runtime: call it from a worker thread. The lookup and the denial are one
//! request each, bounded by the client's own 30 s.
//!
//! Must not: log the code, the password, a token, the master key, the request's facts or a server's words; format a
//! core request (its `Debug` prints the path, and the lookup's path carries the code); keep the password or the key
//! after the call (both are `Zeroizing` and dropped here).

use std::fmt;

use beebeeb_core::auth_flow::device::{Approval, ApproveStep, PendingApproval};
use beebeeb_core::auth_flow::{AuthFlowError, HttpRequest, HttpResponse, normalize_device_code};
use zeroize::Zeroizing;

use crate::account::SessionGeneration;
use crate::app::lock_api::PersonRefused;
use crate::app::{CredentialStores, Engine};
use crate::auth_transport;
use crate::session::Session;

/// Why a call about a sign-in request did not end as asked. Fixed answers: never a server's words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApproveError {
    /// No session in memory, or not the session of the press: nothing was approved. Asked before the first
    /// request and again before the claim and the relay, so a claim may have left (a session nobody can use); the
    /// keys did not.
    NotSignedIn,
    /// A session is in memory behind a locked screen or a closed vault: nothing was approved (as
    /// [`ApproveError::NotSignedIn`], a claim may have left; the keys did not).
    Locked,
    /// Beebeeb no longer accepts this session. Nothing was approved.
    SessionEnded,
    /// No waiting request with that code: wrong, expired or already used (Beebeeb says not which).
    NotFound,
    /// The request ran out before the approval reached it. Nothing was approved.
    Expired,
    /// Another device answered the request first. Nothing was approved from this PC.
    AlreadyHandled,
    /// The password was empty or not the account's: nothing was claimed.
    WrongPassword,
    /// Too many tries; seconds to wait. Nothing was claimed.
    RateLimited(u64),
    /// Beebeeb could not be reached before the keys left: nothing was approved.
    Offline,
    /// Beebeeb refused, or answered something this client does not read, before the keys left: nothing was
    /// approved.
    Refused,
    /// The keys left (the relay was sent) and no answer said how it ended: the other device MAY be signed in.
    Unknown,
}

/// What Beebeeb knows about a waiting request, as plain text. `requested_at`, `expires_at` (RFC 3339), `address` and
/// `country` are measured by Beebeeb; `device_name`, `client`, `client_version` and `os` are what the requesting
/// device says about itself (hints, never proof). Printed without its values.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct RequestFacts {
    pub requested_at: String,
    pub expires_at: String,
    pub address: String,
    pub country: Option<String>,
    pub device_name: Option<String>,
    pub client: Option<String>,
    pub client_version: Option<String>,
    pub os: Option<String>,
}

// By hand: a device's name and a network address must not reach a log through `{:?}`.
impl fmt::Debug for RequestFacts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RequestFacts { .. }")
    }
}

/// A waiting request, found by its code and held for review: core's own `PendingApproval` (which keeps the code
/// inside) and nothing else. No `Debug` and no `Clone`: the code is never printed or copied. An approval consumes
/// it; a denial needs only its [`DenyHandle`].
pub struct FoundRequest {
    pending: PendingApproval,
}

impl FoundRequest {
    /// What Beebeeb knows about the request.
    pub fn facts(&self) -> RequestFacts {
        let facts = self.pending.facts();
        RequestFacts {
            requested_at: facts.requested_at.clone(),
            expires_at: facts.expires_at.clone(),
            address: facts.ip.clone(),
            country: facts.country.clone(),
            device_name: facts.device_name.clone(),
            client: facts.client.clone(),
            client_version: facts.client_version.clone(),
            os: facts.os.clone(),
        }
    }

    /// Whether `typed` is this request's code, in any of the shapes a person types it (core's rule, DW-A13). No
    /// network.
    pub fn code_matches(&self, typed: &str) -> bool {
        self.pending.code_matches(typed)
    }

    /// Seconds until the request runs out at `now_unix` (negative once it has), or `None` when Beebeeb's time does
    /// not read. No network.
    pub fn seconds_left(&self, now_unix: i64) -> Option<i64> {
        self.pending.facts().seconds_left(now_unix)
    }

    /// What a later Deny needs: the code, wiped on drop. Kept by a host when the request itself goes to an
    /// approval.
    pub fn deny_handle(&self) -> DenyHandle {
        DenyHandle {
            code: Zeroizing::new(self.pending.code().to_owned()),
        }
    }
}

/// The code of a request that may still be denied. No `Debug`, no `Clone`, no accessor: it only goes back to
/// [`Engine::deny_sign_in_for`], and answers whether a typed code is its code.
pub struct DenyHandle {
    code: Zeroizing<String>,
}

impl DenyHandle {
    /// Whether `typed` is this request's code, in any of the shapes a person types it (core's own normalisation). No
    /// network: a host answers a code it already holds from here.
    pub fn code_matches(&self, typed: &str) -> bool {
        normalize_device_code(typed).is_some_and(|code| code.as_str() == self.code.as_str())
    }
}

/// Where an approval stands: the relay is the one request after which the keys may have reached the device.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Leg {
    StepUp,
    Claim,
    Relay,
}

impl<C: CredentialStores> Engine<C> {
    /// Whether a person's approval may be asked for under `session` now: the session in memory is that one, the
    /// screen is not locked and Beebeeb still accepts it. No network, never waits on a session step: a host asks this
    /// on the window's thread BEFORE it raises Windows Hello, so a locked app raises no prompt.
    pub fn sign_in_approval_allowed_for(&self, session: SessionGeneration) -> Result<(), ApproveError> {
        self.approval_copy(session, |_| ())
    }

    /// The waiting request `code` names (`GET /api/v1/auth/cli-pubkey`), for the session of the press. A code that
    /// is not in the XXXX-XXXX shape sends nothing and is [`ApproveError::NotFound`]. Beebeeb counts every miss as a
    /// guess: a host answers a code it already holds from that [`FoundRequest`] ([`FoundRequest::code_matches`]),
    /// never by asking again. BLOCKS.
    pub fn find_sign_in_request_for(
        &self,
        session: SessionGeneration,
        code: &str,
    ) -> Result<FoundRequest, ApproveError> {
        let (lookup, request) = Approval::lookup(code).map_err(|_| ApproveError::NotFound)?;
        let token = self.approval_copy(session, |current| Zeroizing::new(current.token.clone()))?;
        let response = self
            .send_approval_call(&token, request)
            .map_err(|_| ApproveError::Offline)?;
        if response.status == 401 {
            self.note_approval_session_refused(session);
            return Err(ApproveError::SessionEnded);
        }
        let pending = lookup.on_response(response).map_err(|error| match error {
            AuthFlowError::RequestNotFound => ApproveError::NotFound,
            AuthFlowError::RateLimited { retry_after_secs } => ApproveError::RateLimited(retry_after_secs),
            _ => ApproveError::Refused,
        })?;
        Ok(FoundRequest { pending })
    }

    /// WRITES, and lets another device in: approve `request` for the session of the press. `typed_code` must be the
    /// request's code (core refuses otherwise, before anything is sent); `password` is the account's, for core's
    /// step-up (purpose `cli_device_approval`, bound to the code). The host has raised Windows Hello and had it
    /// verified BEFORE this call: nothing here asks it.
    ///
    /// The master key is read from the session in memory only, under `session` and an open screen; it is sealed by
    /// core to the other device's key, and only the sealed payload leaves. `Ok`: the relay was taken and the other
    /// device receives its keys. See the module's doc for what each `Err` says was (not) sent. BLOCKS.
    pub fn approve_sign_in_for(
        &self,
        session: SessionGeneration,
        request: FoundRequest,
        typed_code: &str,
        password: Zeroizing<Vec<u8>>,
    ) -> Result<(), ApproveError> {
        if password.is_empty() {
            return Err(ApproveError::WrongPassword);
        }
        let (token, master_key) = self.approval_copy(session, |current| {
            (
                Zeroizing::new(current.token.clone()),
                Zeroizing::new(current.master_key),
            )
        })?;
        let started = request.pending.approve(typed_code, &password, &master_key);
        // Core holds its own wiped copies from here on.
        drop(master_key);
        drop(password);
        // A typed code that is not the request's, or a code core cannot step up for: nothing was sent.
        let (mut run, mut next) = started.map_err(|_| ApproveError::Refused)?;
        let mut leg = Leg::StepUp;
        let mut first_leg = true;
        loop {
            if leg != Leg::StepUp {
                // A lock, a sign-out or another sign-in during the step-up: the claim does not leave. During the
                // claim's round trip: the relay (the point of no return) does not leave, and nothing was approved.
                self.approval_copy(session, |_| ())?;
            }
            let sent = self.send_approval_call(&token, next);
            let response = match sent {
                Ok(response) => response,
                Err(_) if leg == Leg::Relay => return Err(ApproveError::Unknown),
                Err(_) => return Err(ApproveError::Offline),
            };
            // The first leg of the step-up carries no proof of the password: a 401 there is about the session.
            // A 401 on the claim or the relay is about the session too: refused at the door, nothing was taken.
            if response.status == 401 && (first_leg || leg != Leg::StepUp) {
                self.note_approval_session_refused(session);
                return Err(ApproveError::SessionEnded);
            }
            first_leg = false;
            let status = response.status;
            match run.on_response(response) {
                Ok(ApproveStep::Approved) => return Ok(()),
                Ok(ApproveStep::Send(request)) => {
                    leg = match (leg, request.path.as_str()) {
                        (_, path) if path.ends_with("/cli-session") => Leg::Claim,
                        (_, path) if path.ends_with("/cli-authorize") => Leg::Relay,
                        (current, _) => current,
                    };
                    next = request;
                }
                Err(error) => return Err(Self::approval_error(leg, status, error)),
            }
        }
    }

    /// End the request with nothing shared (`POST /api/v1/auth/cli-deny`, no step-up: denying grants nothing), for
    /// the session of the press. `Ok`: Beebeeb confirmed the denial and the other device was told. A 404 is
    /// [`ApproveError::NotFound`]: the request is gone (it ran out, was answered, or another account claimed it).
    /// BLOCKS.
    pub fn deny_sign_in_for(&self, session: SessionGeneration, handle: &DenyHandle) -> Result<(), ApproveError> {
        let (denial, request) = Approval::deny(&handle.code).map_err(|_| ApproveError::NotFound)?;
        let token = self.approval_copy(session, |current| Zeroizing::new(current.token.clone()))?;
        let response = self
            .send_approval_call(&token, request)
            .map_err(|_| ApproveError::Offline)?;
        if response.status == 401 {
            self.note_approval_session_refused(session);
            return Err(ApproveError::SessionEnded);
        }
        denial.on_response(response).map_err(|error| match error {
            AuthFlowError::RequestNotFound => ApproveError::NotFound,
            AuthFlowError::RateLimited { retry_after_secs } => ApproveError::RateLimited(retry_after_secs),
            _ => ApproveError::Refused,
        })
    }

    /// What core's refusal on `leg` (answered with `status`) says was (not) sent.
    fn approval_error(leg: Leg, status: u16, error: AuthFlowError) -> ApproveError {
        match (leg, error) {
            (Leg::StepUp, AuthFlowError::InvalidCredentials | AuthFlowError::InvalidInput(_)) => {
                ApproveError::WrongPassword
            }
            (_, AuthFlowError::RateLimited { retry_after_secs }) if leg != Leg::Relay => {
                ApproveError::RateLimited(retry_after_secs)
            }
            (Leg::Claim, AuthFlowError::AlreadyHandled) => ApproveError::AlreadyHandled,
            (Leg::Claim, AuthFlowError::RequestNotFound) => ApproveError::Expired,
            // Core reads the relay's 404 as "the request was gone": it was not delivered.
            (Leg::Relay, AuthFlowError::Expired) => ApproveError::Expired,
            // The keys left: anything but a 2xx or a 404 does not say they did not arrive.
            (Leg::Relay, _) => ApproveError::Unknown,
            // Before the relay: a server error or an answer this client cannot read. Nothing was approved.
            _ if (500..600).contains(&status) => ApproveError::Offline,
            _ => ApproveError::Refused,
        }
    }

    /// `take` of the session in memory, when it is the one `session` names and a person's call may run (the screen is
    /// not locked, the vault is open) and Beebeeb still accepts it.
    fn approval_copy<T>(
        &self,
        session: SessionGeneration,
        take: impl FnOnce(&Session) -> T,
    ) -> Result<T, ApproveError> {
        let held = match self.person_session(|current, generation| (generation == session).then(|| take(current))) {
            Ok(Some(held)) => held,
            Ok(None) => return Err(ApproveError::NotSignedIn),
            Err(PersonRefused::ScreenLocked | PersonRefused::VaultClosed) => return Err(ApproveError::Locked),
            Err(PersonRefused::NoSession) => {
                let present = self.auth_present.lock().map(|present| *present).unwrap_or(false);
                return Err(if present {
                    ApproveError::Locked
                } else {
                    ApproveError::NotSignedIn
                });
            }
        };
        if self.acct.auth_health.is_expired() {
            drop(held);
            return Err(ApproveError::SessionEnded);
        }
        Ok(held)
    }

    /// One of core's requests on the session's token. Neither the request nor the answer is logged or formatted.
    fn send_approval_call(&self, token: &str, request: HttpRequest) -> Result<HttpResponse, ()> {
        let base = self.config.api_base.trim_end_matches('/').to_string();
        self.runtime
            .block_on(auth_transport::send_to(&base, &self.client, Some(token), request))
            .map_err(|_| ())
    }

    /// A 401 counts in the engine's run of them, only while the session is still the one of the press.
    fn note_approval_session_refused(&self, session: SessionGeneration) {
        let _ = self.acct.while_session_is(&self.writes, session, |_| {
            self.acct
                .auth_health
                .note_result(Some(&anyhow::anyhow!("401 Unauthorized")));
        });
    }
}

#[cfg(test)]
#[path = "approve_sign_in_tests.rs"]
mod tests;
