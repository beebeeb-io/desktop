//! What the Files window's sidebar says about the account beside the list, for a host without a UI toolkit: how
//! many files are kept on this PC and what the storage in use is made of ([`Engine::files_facts`]).
//!
//! Read from the signed-in account's local state, through the account-bound handle (task 2488): a state that is not
//! the session's answers nothing. The answer carries the session it was read for, for the host to compare.
//!
//! BLOCKS: both figures read every row of the local state; call it from a worker, and only after a change (never
//! per frame). No network. Must not: log or return a name or a path.

use super::{CredentialStores, Engine, LocalStateRefused};
use crate::account::SessionGeneration;
use crate::drive_v2::model::{available_offline_count, storage_split};

#[cfg(test)]
#[path = "files_facts_api_tests.rs"]
mod tests;

/// The figures, and the session they were read for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FilesFacts {
    pub session: SessionGeneration,
    /// Files kept on this PC (pinned, by their own pin or a folder's).
    pub available_offline: u32,
    /// Bytes of the files outside Backups that are no photo or video.
    pub files_bytes: i64,
    /// Bytes of the photos and videos outside Backups.
    pub photos_bytes: i64,
}

impl<C: CredentialStores> Engine<C> {
    /// See the module's doc.
    pub fn files_facts(&self) -> Result<FilesFacts, LocalStateRefused> {
        // First: a session change during the read makes the facts older than the session, never newer.
        let session = self.acct.session_generation();
        let (available_offline, split) = self
            .local_state()?
            .with(|db| (available_offline_count(db), storage_split(db)))?;
        let (files_bytes, photos_bytes, _backups) = split.map_err(|_| LocalStateRefused::Unreadable)?;
        Ok(FilesFacts {
            session,
            available_offline,
            files_bytes,
            photos_bytes,
        })
    }
}
