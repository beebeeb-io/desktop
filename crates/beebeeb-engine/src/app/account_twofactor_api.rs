//! Two-factor authentication of the signed-in account, over core's `account_flows` (`totp_setup`, `totp_confirm`,
//! `totp_disable`, `regenerate_backup_codes`), for the native Account panes.
//!
//! Every method BLOCKS on the handle's runtime: call it from a worker thread, never from the UI thread.
//!
//! Session rule: each method is for ONE session ([`SessionGeneration`], read by the host with the press). It sends
//! nothing for another session, and an answer made while the session moved on is dropped (its secrets wiped), as
//! `NotSignedIn`. A key or a backup code leaves here only inside a [`PendingEnrollment`] or a `Zeroizing` list.
//!
//! The run of 401s ([`Engine::account_session_refused`]) is NOT touched by these calls: Beebeeb answers a wrong
//! code with a 4xx that looks like a refused session, and a wrong code must never end a session in the window's
//! eyes. A session Beebeeb really no longer accepts is found by the engine's own loop and the page reads.
//!
//! Known gap (core): a code that was refused spends the [`PendingEnrollment`]; the host asks for a new key.
//!
//! Must not: log a code, a key, a backup code or a server's body.

use beebeeb_core::account_flows::{
    AccountFlow, AccountFlowError, AccountOutcome, AccountTransport, ApiRequest, ApiResponse, TotpEnrollment, run,
};
use zeroize::Zeroizing;

use super::{CredentialStores, Engine};
use crate::account::SessionGeneration;
use crate::auth_transport;

/// Why a two-factor or email call did not happen. Nothing changed in any of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountFlowFailure {
    /// No session, or not the one the call was made for.
    NotSignedIn,
    /// Beebeeb could not be reached.
    Offline,
    /// The code (or the password) was not taken.
    WrongCode,
    /// The server wants a confirmation this call cannot give (a step-up, or a code).
    NeedsConfirmation,
    /// This email is already in use (change email only).
    EmailInUse,
    /// The address is not one (change email only).
    NotAnAddress,
    /// The password was wrong (change email only).
    WrongPassword,
    /// Too many tries: wait.
    RateLimited,
    /// Beebeeb answered with an error, or in a shape that does not read.
    Server,
}

/// Fixed words of core's error: the server's body never gets here.
pub(super) fn failure_of(error: &AccountFlowError) -> AccountFlowFailure {
    match error {
        AccountFlowError::InvalidCode => AccountFlowFailure::WrongCode,
        AccountFlowError::WrongPassword => AccountFlowFailure::WrongPassword,
        AccountFlowError::SecondFactorRequired | AccountFlowError::ConfirmationRequired => {
            AccountFlowFailure::NeedsConfirmation
        }
        AccountFlowError::EmailInUse => AccountFlowFailure::EmailInUse,
        AccountFlowError::InvalidEmail => AccountFlowFailure::NotAnAddress,
        AccountFlowError::RateLimited => AccountFlowFailure::RateLimited,
        _ => AccountFlowFailure::Server,
    }
}

/// A started two-factor set-up: the key and the link a QR code carries, and the backup codes core holds until the
/// first code is taken. No `Debug`, no `Clone`: it is a secret.
pub struct PendingEnrollment {
    session: SessionGeneration,
    enrollment: TotpEnrollment,
}

impl PendingEnrollment {
    /// The session this set-up was started in.
    pub fn session(&self) -> SessionGeneration {
        self.session
    }
    /// The key, as the server made it (base 32), to type into an authenticator.
    pub fn key(&self) -> &str {
        self.enrollment.secret.as_str()
    }
    /// The `otpauth://` link the QR code carries.
    pub fn otpauth_uri(&self) -> &str {
        self.enrollment.otpauth_uri.as_str()
    }
}

/// Blocking transport of core's flows over reqwest: the session's token and the confirm token stay in here.
pub(super) struct FlowTransport<'a> {
    pub runtime: &'a tokio::runtime::Runtime,
    pub api_base: &'a str,
    pub token: &'a str,
    pub offline: bool,
}

impl AccountTransport for FlowTransport<'_> {
    fn send(&mut self, request: &ApiRequest) -> Result<ApiResponse, AccountFlowError> {
        let Ok(client) = auth_transport::client_for(self.api_base) else {
            self.offline = true;
            return Err(AccountFlowError::Protocol);
        };
        let url = format!("{}{}", self.api_base, request.path);
        let method =
            reqwest::Method::from_bytes(request.method.as_str().as_bytes()).map_err(|_| AccountFlowError::Protocol)?;
        let mut builder = client.request(method, &url).bearer_auth(self.token);
        if let Some(confirm) = request.confirm_token() {
            builder = builder.header("X-Confirm-Token", confirm);
        }
        if let Some(body) = request.body.as_ref() {
            builder = builder.json(body);
        }
        let sent = self.runtime.block_on(async {
            let response = builder.send().await.ok()?;
            let status = response.status().as_u16();
            let text = Zeroizing::new(response.text().await.unwrap_or_default());
            Some(ApiResponse::from_text(status, &text))
        });
        match sent {
            Some(response) => Ok(response),
            None => {
                self.offline = true;
                Err(AccountFlowError::Protocol)
            }
        }
    }
}

/// Core's flow to its end over `transport`, in one function the tests drive with a fake server.
pub(super) fn drive(
    flow: Result<AccountFlow, AccountFlowError>,
    transport: &mut dyn AccountTransport,
) -> Result<AccountOutcome, AccountFlowError> {
    run(flow?, transport)
}

/// The four flows of this file, over any transport (tests use a fake server).
pub(super) mod flows {
    use super::*;

    pub fn begin(
        transport: &mut dyn AccountTransport,
        current_code: Option<&str>,
        session: SessionGeneration,
    ) -> Result<PendingEnrollment, AccountFlowError> {
        match drive(AccountFlow::totp_setup(None, current_code), transport)? {
            AccountOutcome::TotpEnrollment(enrollment) => Ok(PendingEnrollment { session, enrollment }),
            _ => Err(AccountFlowError::Protocol),
        }
    }

    pub fn turn_on(
        transport: &mut dyn AccountTransport,
        pending: PendingEnrollment,
        code: &str,
    ) -> Result<Zeroizing<Vec<String>>, AccountFlowError> {
        match drive(AccountFlow::totp_confirm(pending.enrollment, code), transport)? {
            AccountOutcome::BackupCodes(codes) => Ok(codes),
            _ => Err(AccountFlowError::Protocol),
        }
    }

    pub fn turn_off(transport: &mut dyn AccountTransport, code: &str) -> Result<(), AccountFlowError> {
        match drive(AccountFlow::totp_disable(code), transport)? {
            AccountOutcome::Done => Ok(()),
            _ => Err(AccountFlowError::Protocol),
        }
    }

    pub fn new_codes(
        transport: &mut dyn AccountTransport,
        code: &str,
    ) -> Result<Zeroizing<Vec<String>>, AccountFlowError> {
        match drive(AccountFlow::regenerate_backup_codes(code), transport)? {
            AccountOutcome::BackupCodes(codes) => Ok(codes),
            _ => Err(AccountFlowError::Protocol),
        }
    }
}

impl<C: CredentialStores> Engine<C> {
    /// Runs `work` over the real transport for `session`: nothing is sent when the session in memory is another
    /// one, and an answer made while it moved on is dropped.
    pub(super) fn account_flow<T>(
        &self,
        session: SessionGeneration,
        work: impl FnOnce(&mut dyn AccountTransport) -> Result<T, AccountFlowError>,
    ) -> Result<T, AccountFlowFailure> {
        let (token, now) = self
            .token_of_this_session()
            .map_err(|_| AccountFlowFailure::NotSignedIn)?;
        if now != session || !self.account_session_is_still(session) {
            return Err(AccountFlowFailure::NotSignedIn);
        }
        let mut transport = FlowTransport {
            runtime: &self.runtime,
            api_base: &self.config.api_base,
            token: token.as_str(),
            offline: false,
        };
        let answer = work(&mut transport);
        let offline = transport.offline;
        if !self.account_session_is_still(session) {
            // The answer belongs to a session that is over: its secrets are dropped (wiped) here.
            return Err(AccountFlowFailure::NotSignedIn);
        }
        answer.map_err(|error| {
            if offline {
                AccountFlowFailure::Offline
            } else {
                failure_of(&error)
            }
        })
    }

    /// WRITES (a pending key on Beebeeb): starts the set-up of two-factor. With two-factor off it needs nothing; with
    /// it on, `current_code` is a current authenticator code (moving to a new authenticator).
    pub fn two_factor_begin(
        &self,
        session: SessionGeneration,
        current_code: Option<&str>,
    ) -> Result<PendingEnrollment, AccountFlowFailure> {
        self.account_flow(session, |t| flows::begin(t, current_code, session))
    }

    /// WRITES: turns two-factor on with the first code from the new key; the 8 backup codes come back once. A code
    /// that is refused spends `pending` (core's rule): start again with [`Engine::two_factor_begin`].
    pub fn two_factor_turn_on(
        &self,
        session: SessionGeneration,
        pending: PendingEnrollment,
        code: &str,
    ) -> Result<Zeroizing<Vec<String>>, AccountFlowFailure> {
        if pending.session != session {
            return Err(AccountFlowFailure::NotSignedIn);
        }
        self.account_flow(session, |t| flows::turn_on(t, pending, code))
    }

    /// WRITES: turns two-factor off with a current code.
    pub fn two_factor_turn_off(&self, session: SessionGeneration, code: &str) -> Result<(), AccountFlowFailure> {
        self.account_flow(session, |t| flows::turn_off(t, code))
    }

    /// WRITES: 8 new backup codes with a current authenticator code (6 digits); the old ones stop working.
    pub fn two_factor_new_backup_codes(
        &self,
        session: SessionGeneration,
        code: &str,
    ) -> Result<Zeroizing<Vec<String>>, AccountFlowFailure> {
        self.account_flow(session, |t| flows::new_codes(t, code))
    }
}

#[cfg(test)]
#[path = "account_twofactor_api_tests.rs"]
mod tests;
