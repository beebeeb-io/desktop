//! What this account downloads now, for the native Files window's "Downloading" rows (DW-0412): the copies coming
//! down, each with how far it has come.
//!
//! - Read from the transfer board, for the session of the ask: a session that moved on is
//!   [`LocalStateRefused::SessionChanged`], nothing is handed over.
//! - The board is one per process: a row whose file this account's local state does not hold is never listed.
//! - Only `Direction::Down` rows that have not failed: an upload, a streaming open and a failed row are not "a copy
//!   coming down". A `Down` row is not always a keep: a Windows range session (an app reading part of a placeholder),
//!   a fetch of a placeholder that is not kept and a conflict copy are `Down` rows too, so each row says whether this
//!   account KEEPS the file (`DownloadRow::kept`); only a kept one may be drawn as Available offline.
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
    /// This account keeps the file on this PC. A range read, a placeholder fetch or a conflict copy also shows on
    /// the board, and is NOT a keep.
    pub kept: bool,
}

/// The host's window learns of a download row beginning, failing or leaving the board by a wake: the pulse that
/// would say so is not sent while the engine is paused, offline or stopped. Called once, when the handle opens.
pub(super) fn wake_the_host_for_downloads(host: &std::sync::Arc<dyn crate::host::EngineHost>) {
    if let Some(board) = host.transfer_board() {
        let host = host.clone();
        board.set_wake(move || host.wake());
    }
}

impl<C: CredentialStores> Engine<C> {
    /// Bytes done so far on every running upload and download (up, down), read from the transfer board; no name, no id.
    /// Two reads a few seconds apart give the pane's live rate. `None`: no board yet.
    pub fn transfer_totals(&self) -> Option<(u64, u64)> {
        let board = self.host.transfer_board()?;
        let (mut up, mut down) = (0u64, 0u64);
        for t in board.snapshot_dtos().into_iter().filter(|t| !t.failed) {
            match t.direction {
                Direction::Up => up += t.done,
                Direction::Down => down += t.done,
                Direction::Stream => {}
            }
        }
        Some((up, down))
    }

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
        // `with` refuses an answer read under a session that moved on meanwhile.
        let mine: HashMap<String, (i64, bool)> = state
            .with(|db| db.sizes_and_kept(&ids))?
            .map_err(|_| LocalStateRefused::Unreadable)?;
        Ok(moving
            .into_iter()
            .filter_map(|t| {
                let &(size, kept) = mine.get(&t.file_id)?;
                Some(DownloadRow {
                    // A fetch holds a total of 0 until its metadata says otherwise: the file's own size then.
                    total: if t.total == 0 { size.max(0) as u64 } else { t.total },
                    file_id: t.file_id,
                    done: t.done,
                    kept,
                })
            })
            .collect())
    }
}
