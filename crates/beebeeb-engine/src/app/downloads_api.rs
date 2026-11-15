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
    /// See the module's doc. One read of the board, one query to keep this account's files.
    pub fn downloads_for(&self, session: SessionGeneration) -> Result<Vec<DownloadRow>, LocalStateRefused> {
        let state = self.local_state()?;
        if state.session() != session {
            return Err(LocalStateRefused::SessionChanged);
        }
        let moving: Vec<_> = self
            .host
            .transfer_board()
            .map(|b| b.snapshot_dtos())
            .unwrap_or_default()
            .into_iter()
            .filter(|t| t.direction == Direction::Down && !t.failed)
            .collect();
        if moving.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<String> = moving.iter().map(|t| t.file_id.clone()).collect();
        let mine: HashMap<String, i64> = state
            .with(|db| db.sizes_of(&ids))?
            .map_err(|_| LocalStateRefused::Unreadable)?;
        // The session may have moved while the database was read: nothing is handed over then.
        if state.session() != session {
            return Err(LocalStateRefused::SessionChanged);
        }
        Ok(moving
            .into_iter()
            .filter(|t| mine.contains_key(&t.file_id))
            .map(|t| DownloadRow {
                file_id: t.file_id,
                done: t.done,
                total: t.total,
            })
            .collect())
    }
}
