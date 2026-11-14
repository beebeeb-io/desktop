//! Changing the sign-in email, over core's `account_flows` (`change_email`, `verify_email`), for the native Account
//! panes. Every method BLOCKS on the handle's runtime: call it from a worker thread.
//!
//! Step 1 ([`Engine::email_change_start`]): core proves the password with its own OPAQUE step-up and registers the
//! password again under the new address (the password file is bound to the email), then Beebeeb mails an 8-digit
//! code to the NEW address. The old address and password keep working until step 2. The password is handed in as
//! text for this one call: it is copied into a wiping buffer and never logged or kept.
//! Step 2 ([`Engine::email_change_confirm`]): the 8 digits.
//!
//! Session rule and 401 rule: as `account_twofactor_api`. The signed-in session is unlocked by definition (it holds
//! the vault key), so there is no "locked" answer here.
//!
//! Known gap: this engine does not rewrite the email it mirrors for the session (`account_email`) after step 2; it
//! follows at the next sign-in. Beebeeb has no route to send the code again, so there is no resend.
//!
//! Must not: log an address, a password or a code.

use beebeeb_core::account_flows::{AccountBinding, AccountFlow, AccountOutcome};
use beebeeb_core::kdf::MasterKey;
use zeroize::Zeroizing;

use super::account_twofactor_api::{AccountFlowFailure, drive};
use super::{CredentialStores, Engine};
use crate::account::SessionGeneration;

impl<C: CredentialStores> Engine<C> {
    /// WRITES: asks Beebeeb to change the sign-in email to `new_email`, proving `password`; the code goes to the new
    /// address. `Ok`: the address core normalised, which the code page shows.
    pub fn email_change_start(
        &self,
        session: SessionGeneration,
        new_email: &str,
        password: &str,
    ) -> Result<String, AccountFlowFailure> {
        let password = Zeroizing::new(password.to_string());
        let key = {
            let guard = self.acct.session.lock().map_err(|_| AccountFlowFailure::NotSignedIn)?;
            Zeroizing::new(guard.as_ref().ok_or(AccountFlowFailure::NotSignedIn)?.master_key)
        };
        self.account_flow(session, |transport| {
            let binding = AccountBinding::from_master_key(&MasterKey::from_bytes(*key));
            match drive(
                AccountFlow::change_email(new_email, password.as_str(), binding),
                transport,
            )? {
                AccountOutcome::EmailChanged { email } => Ok(email),
                _ => Err(beebeeb_core::account_flows::AccountFlowError::Protocol),
            }
        })
    }

    /// WRITES: confirms the new address with the 8 digits Beebeeb mailed to it.
    pub fn email_change_confirm(&self, session: SessionGeneration, code: &str) -> Result<(), AccountFlowFailure> {
        self.account_flow(session, |transport| {
            match drive(AccountFlow::verify_email(code), transport)? {
                AccountOutcome::Done => Ok(()),
                _ => Err(beebeeb_core::account_flows::AccountFlowError::Protocol),
            }
        })
    }
}
