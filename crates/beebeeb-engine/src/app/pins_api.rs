//! Pinned folders for a host (the sidebar's "Pinned" section): the account's `pinned_folders` preference, the one the
//! web app keeps, read and written for one session. BLOCKS: a worker's call. Must not log an id.

use super::settings_api::AccountCallError;
use super::{CredentialStores, Engine};
use crate::account::SessionGeneration;

impl<C: CredentialStores> Engine<C> {
    /// The pinned folder ids, in order, for `session`. A failed read is an `Err`, never "none".
    pub fn pinned_folders_for(&self, session: SessionGeneration) -> Result<Vec<String>, AccountCallError> {
        self.account_call_for(session, |api| async move { api.pinned_folders().await })
    }

    /// Stores the pinned folder ids (the whole list) for `session`.
    pub fn set_pinned_folders_for(&self, session: SessionGeneration, ids: Vec<String>) -> Result<(), AccountCallError> {
        self.account_call_for(session, move |api| async move { Ok(api.set_pinned_folders(&ids).await?) })
    }
}
