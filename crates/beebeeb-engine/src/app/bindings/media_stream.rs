//! Slice M1 (`media-range`): the bytes of a file being played, decrypted in memory range by range, for the native
//! app's player (the media plan, (c)).
//!
//! [`Engine::open_media_for`] reads the file's metadata ONCE and pins its version; [`MediaReader::read_at`] then
//! fetches and opens only the chunks that cover the range, each authenticated at its own index for the pinned version,
//! and hands the plaintext over in a `Zeroizing` buffer. Nothing is written to disk, nothing reaches the transfer
//! board, no row status changes. A file wholly kept on this PC is read from its copy in the sync folder instead
//! ([`MediaOrigin::ThisPc`]); nothing is decrypted for it, and nothing recalls a placeholder.
//!
//! Session-bound like its neighbours (`*_for(session)`): asked FOR the session the host read at its press; a lock, a
//! sign-out or an account change wipes what the reader holds, and every later read answers
//! [`MediaReadError::NotSignedIn`].
//!
//! The scaffold: the types and the signatures. Until M1 fills it, nothing is opened: `open_media_for` answers
//! [`OpenFileError::Failed`] (the player shows "This file could not be played." with "Open in another app…").

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use zeroize::Zeroizing;

use crate::account::SessionGeneration;
use crate::app::viewer_api::OpenFileError;
use crate::app::{CredentialStores, Engine};

/// The most decrypted chunk bytes one reader holds (a zeroized LRU; chunk 0 stays).
pub const MEDIA_CACHE_MAX: u64 = 32 << 20;
/// How far past the last read the reader fetches ahead, on a worker.
pub const MEDIA_READ_AHEAD: u64 = 8 << 20;
/// What one playback may fetch beyond twice the file's size (a helper that loops seeks cannot make the app download
/// without end); past it every read answers [`MediaReadError::OverBudget`].
pub const MEDIA_REFETCH_SLACK: u64 = 256 << 20;

/// Where the bytes a reader hands over come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaOrigin {
    /// The copy kept in the sync folder on this PC (nothing decrypted).
    ThisPc,
    /// Beebeeb's encrypted chunks, decrypted in memory.
    Beebeeb,
}

/// Why a range was not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaReadError {
    /// The session the reader was opened for is not the one in memory (a lock, a sign-out, another account).
    NotSignedIn,
    /// The pinned version is no longer served (a newer one replaced it, or the file is gone).
    Gone,
    /// Beebeeb could not be reached.
    Unreachable,
    /// The playback fetched more than twice the file's size plus [`MEDIA_REFETCH_SLACK`].
    OverBudget,
    /// The range is not inside the file.
    OutOfRange,
    /// The caller's cancel flag was set.
    Cancelled,
    /// Anything else (a chunk that did not authenticate among them). Nothing is handed over.
    Failed,
}

/// A file opened for playback: its length, where its bytes come from, and (once M1 fills it) the pinned version, the
/// session it was opened for and the zeroized chunk cache. `Send`, not `Clone`; its `Debug` prints no id and no byte.
pub struct MediaReader {
    len: u64,
    origin: MediaOrigin,
}

/// The length and the origin, never the file's id or a byte of it.
impl fmt::Debug for MediaReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MediaReader")
            .field("len", &self.len)
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}

impl<C: CredentialStores> Engine<C> {
    /// Opens `file_id` for playback FOR `session` (the session the host read at its press): one metadata read, the
    /// version pinned. Refused when `session` is not the one in memory. The scaffold opens nothing:
    /// [`OpenFileError::Failed`].
    pub fn open_media_for(
        self: &Arc<Self>,
        session: SessionGeneration,
        file_id: &str,
    ) -> Result<MediaReader, OpenFileError> {
        let _ = (session, file_id);
        Err(OpenFileError::Failed)
    }
}

impl MediaReader {
    /// The file's plaintext length in bytes.
    pub fn len(&self) -> u64 {
        self.len
    }

    /// Where the bytes come from.
    pub fn origin(&self) -> MediaOrigin {
        self.origin
    }

    /// Bytes `offset .. offset + len` of the file (`len` ≤ the protocol's `MAX_READ`), decrypted in memory. The
    /// scaffold reads nothing: [`MediaReadError::Failed`].
    pub fn read_at(
        &mut self,
        offset: u64,
        len: u32,
        cancel: &AtomicBool,
    ) -> Result<Zeroizing<Vec<u8>>, MediaReadError> {
        let _ = (offset, len, cancel);
        Err(MediaReadError::Failed)
    }
}
