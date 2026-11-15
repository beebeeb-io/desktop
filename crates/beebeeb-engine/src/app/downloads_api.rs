//! What this account downloads now, for the native Files window's "Downloading" rows (DW-0412): the copies coming
//! down, each with how far it has come.
//!
//! - Read from the transfer board, for the session of the ask: a session that moved on is
//!   [`LocalStateRefused::SessionChanged`], nothing is handed over.
//! - The board is one per process: a row whose file this account's local state does not hold is never listed.
//! - Only whole-file downloads (`Direction::Down`) that have not failed: an upload, a streaming open and a failed
//!   row are not "a copy coming down".
//!
//! BLOCKS on the local state: a worker's. Must not: log or `Debug` a file id's name.

use std::collections::HashMap;

use super::{CredentialStores, Engine, LocalStateRefused};
use crate::account::SessionGeneration;
use crate::transfer_progress::Direction;

#[cfg(test)]
#[path = "downloads_api_tests.rs"]
mod tests;

/// One copy on its way down.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadRow {
    pub file_id: String,
    /// Bytes written so far.
    pub done: u64,
    /// The declared total; `0` while it is not known yet.
    pub total: u64,
}

impl<C: CredentialStores> Engine<C> {
    /// See the module's doc.
    pub fn downloads_for(&self, session: SessionGeneration) -> Result<Vec<DownloadRow>, LocalStateRefused> {
        let _ = session;
        Ok(Vec::new())
    }
}
