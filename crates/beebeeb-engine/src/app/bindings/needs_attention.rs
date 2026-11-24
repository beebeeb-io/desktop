//! LD-0105 Amendment 4 (desktop #225) in the engine: the files that need the person, and their one action,
//! "Discard and download again". A needs-attention file is a failed download whose file changed on this device since:
//! it is kept as it is, refuses every fetch, and only the person's discard ends its row.
//!
//! Session-bound like its neighbours (`*_for(session)`): each call is asked for the session the host read at its press,
//! through the account's own local state (the person's door: a locked screen, a lock, a sign-out, another session or
//! another account's data refuse it), and its answer is handed over only while that session is still the one in
//! memory.
//!
//! - [`Engine::needs_attention_for`]: the rows, read from the account's state database
//!   ([`crate::transfer_progress::needs_attention`]). A locked vault names no file.
//! - [`Engine::discard_and_download_again_for`]: empties the file (whatever was written to it), clears the poison and
//!   fetches it again (Windows: `windows_cf::poison::discard_and_download_again`). Refused, with nothing emptied, while
//!   an upload of the file is on the transfer board: discarding then would throw away the change being sent.
//!
//! BLOCKS (the state database, and on Windows an exclusive open and a dehydrate): a worker's, never the window's
//! thread. Must not: log a name, a path or an id.

use crate::account::SessionGeneration;
use crate::app::{CredentialStores, Engine, LocalStateRefused};
use crate::transfer_progress::{ATTENTION_CACHE, ATTENTION_READS, Direction, NeedsAttention, TransferBoard};

/// Why [`Engine::discard_and_download_again_for`] discarded nothing. A fixed word each ([`Self::kind`]): none carries a
/// name, a path or an id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiscardRefusal {
    /// The account's local state was not handed out for the session of the press (signed out, locked, the session
    /// moved on, the data is not provably this account's, unreadable). Nothing was looked at.
    LocalState(LocalStateRefused),
    /// An upload of this file is on the transfer board: the person's change is being sent.
    Uploading,
    /// This account's local state holds no poison for the id (already discarded, or never poisoned).
    NotPoisoned,
    /// The file is open somewhere: close it first.
    InUse,
    /// The file's row or its place in the sync folder is gone.
    Missing,
    /// No transfer board is shared with this host, so an upload in flight could not be ruled out.
    NoBoard,
    /// This build has no Cloud Files provider: only Windows poisons a file.
    NotSupported,
    /// A step of the discard failed; the word names the step.
    Failed(&'static str),
}

impl DiscardRefusal {
    /// The refusal's word, for a log line and for the host's honest state.
    pub fn kind(self) -> &'static str {
        match self {
            Self::LocalState(refused) => refused.kind(),
            Self::Uploading => "uploading",
            Self::NotPoisoned => "not_poisoned",
            Self::InUse => "in_use",
            Self::Missing => "missing",
            Self::NoBoard => "no_board",
            Self::NotSupported => "not_supported",
            Self::Failed(step) => step,
        }
    }
}

impl<C: CredentialStores> Engine<C> {
    /// The files of this account that need the person (see the module's doc), each with its vault path and size. Read
    /// only. A poison with no file row is not listed (it has no exit), nor is a poison the sweep may still empty.
    pub fn needs_attention_for(&self, session: SessionGeneration) -> Result<Vec<NeedsAttention>, LocalStateRefused> {
        let state = self.local_state()?;
        if state.session() != session {
            return Err(LocalStateRefused::SessionChanged);
        }
        state
            .with(crate::transfer_progress::needs_attention)?
            .ok_or(LocalStateRefused::Unreadable)
    }

    /// "Discard and download again" on `file_id`'s needs-attention row, for the session of the press. See the
    /// module's doc. `Ok`: the file was emptied, its poison cleared and its download started; the row is gone from
    /// the kept rows, and no read that started before brings it back.
    pub fn discard_and_download_again_for(
        &self,
        session: SessionGeneration,
        file_id: &str,
    ) -> Result<(), DiscardRefusal> {
        self.discard_with(session, file_id, platform_discard)
    }

    /// [`Self::discard_and_download_again_for`] with the platform's step handed in. In order: the session of the
    /// press, the poison in this account's own state, no upload of the file on the board, then the step. The handle
    /// asks for the session again after the step; a session that moved on meanwhile is answered as refused, and the
    /// host drops that answer.
    fn discard_with(
        &self,
        session: SessionGeneration,
        file_id: &str,
        step: impl FnOnce(&str) -> Result<(), DiscardRefusal>,
    ) -> Result<(), DiscardRefusal> {
        let state = self.local_state_for_changes().map_err(DiscardRefusal::LocalState)?;
        if state.session() != session {
            return Err(DiscardRefusal::LocalState(LocalStateRefused::SessionChanged));
        }
        let board = self.host.transfer_board();
        let done = state
            .with(|db| {
                match db.poison(file_id) {
                    Ok(Some(_)) => {}
                    Ok(None) => return Err(DiscardRefusal::NotPoisoned),
                    Err(_) => return Err(DiscardRefusal::LocalState(LocalStateRefused::Unreadable)),
                }
                refuse_an_upload(board.as_deref(), file_id)?;
                step(file_id)
            })
            .map_err(DiscardRefusal::LocalState)?;
        if done.is_ok() {
            // Review P3-6 of desktop #225: the row leaves the kept rows, and no read already running brings it back.
            let next = ATTENTION_READS.load(std::sync::atomic::Ordering::SeqCst);
            if let Ok(mut cache) = ATTENTION_CACHE.lock() {
                cache.forget(file_id, next);
            }
        }
        done
    }
}

/// Delta review P2-1 of desktop #225: an upload of the file the board still holds ([`TransferBoard::get`]: its guard is
/// alive) is the person's change being sent; discarding would throw it away. An upload that ended (a failed one is then
/// only shown, for its time to live) is no longer sent and does not refuse. No board shared with the host: an upload
/// could not be ruled out, so nothing is discarded either.
fn refuse_an_upload(board: Option<&TransferBoard>, file_id: &str) -> Result<(), DiscardRefusal> {
    let board = board.ok_or(DiscardRefusal::NoBoard)?;
    if board.get(file_id).is_some_and(|row| row.direction == Direction::Up) {
        return Err(DiscardRefusal::Uploading);
    }
    Ok(())
}

/// Windows: the connected Cloud Files bridge empties the placeholder under the exclusive handle, clears the poison and
/// starts a whole-file fetch on the bridge's runtime.
#[cfg(target_os = "windows")]
fn platform_discard(file_id: &str) -> Result<(), DiscardRefusal> {
    use crate::windows_cf::poison::DiscardRefused as Cf;
    crate::windows_cf::poison::discard_and_download_again(file_id).map_err(|refused| match refused {
        Cf::NotPoisoned => DiscardRefusal::NotPoisoned,
        Cf::InUse => DiscardRefusal::InUse,
        Cf::Missing => DiscardRefusal::Missing,
        Cf::Failed(step) => DiscardRefusal::Failed(step),
    })
}

/// Off Windows no file is ever poisoned (the Cloud Files provider is Windows-only).
#[cfg(not(target_os = "windows"))]
fn platform_discard(_file_id: &str) -> Result<(), DiscardRefusal> {
    Err(DiscardRefusal::NotSupported)
}

#[cfg(test)]
#[path = "needs_attention_tests.rs"]
mod tests;
