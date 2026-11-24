//! The two-factor status the Account pages show (added, last used, backup codes left), read from Beebeeb for the
//! session in memory. Blocks on the handle's runtime: call it from a worker thread.
//!
//! Session rule: as `Engine::activity_log` (`account_actions_api`): the answer is stamped with the session it was read
//! for and dropped ([`AccountCallError::NotSignedIn`]) when that session moved on. A read that failed says its kind;
//! the host then draws no value (never a made-up one).
//!
//! Must not: log a date, a count or a server's words.

use super::account_actions_api::from_call;
use super::account_api::Stamped;
use super::settings_api::{AccountCallError, kind_of};
use super::{CredentialStores, Engine};
use crate::api_client::{ApiClient, TwoFactorStatus};

impl<C: CredentialStores> Engine<C> {
    /// `GET /api/v1/me/two-factor` for the session in memory.
    pub fn two_factor_status(&self) -> Result<Stamped<TwoFactorStatus>, AccountCallError> {
        let (mut token, session) = self.own_session().map_err(|_| AccountCallError::NotSignedIn)?;
        let api = ApiClient::new(self.config.api_base.clone(), std::mem::take(&mut *token), [0; 32]);
        let answer = self
            .runtime
            .block_on(async { api.two_factor_status().await })
            .map_err(|error| kind_of(&error));
        let failed = answer.as_ref().err().copied().map(from_call);
        if !self.note_action(session, failed) {
            return Err(AccountCallError::NotSignedIn);
        }
        answer.map(|value| Stamped { session, value })
    }
}

#[cfg(test)]
#[path = "account_twofactor_status_api_tests.rs"]
mod tests;
