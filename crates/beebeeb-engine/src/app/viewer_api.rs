//! An open file, for a host without a UI toolkit (the native viewer window): what the local state says about the
//! file ([`Engine::open_file_facts`]) and its content, downloaded and decrypted IN MEMORY
//! ([`Engine::open_file_in_memory`]).
//!
//! The content comes by the one download path every hydrate uses (`EngineBridge::open_in_memory`, which ends in
//! `hydrate_whole_file_metered`: the same keys, the same version pin, the same per-chunk checks). No crypto of
//! its own. Nothing about the file is recorded and nothing is written to disk: no status, no cache row, no temp
//! file. The bytes are handed over in a [`Zeroizing`] buffer, wiped when the host drops it; a download that is
//! cancelled or outlives its session is dropped (and wiped) here and never handed over.
//!
//! Session rule: the token, the vault key and the session's generation leave the session's lock together; the
//! local state is reached through the account-bound handle (task 2488); the hand-over happens only while the
//! session in memory is still the very one they were copied from ([`Engine::session_is_still`], asked with no
//! session change half done). Every answer carries that generation, for the host to compare.
//!
//! A file whose bytes are wholly on this computer is read from there instead ([`Engine::open_local_copy_in_memory`]):
//! it opens without a connection, and a change made here that is not sent yet is what is shown (never the older
//! version the server holds). That read is bounded, never fetches a file that is not wholly here, and its answer
//! says where the content came from ([`OnThisPc`]).
//!
//! The handle is the one for changes: resolving the key of a file in a shared folder may record that folder's
//! custody (the engine's own ratchet, as every other download of such a file does), and that is the only thing
//! this module can cause to be written.
//!
//! BLOCKS: [`Engine::open_file_in_memory`] is a worker's (the network); [`Engine::open_file_facts`] reads the
//! local state alone, as a listing does. Must not: log or return a name, a path, a key or a byte of content
//! outside the two answer types; write a decrypted byte anywhere.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use zeroize::Zeroizing;

use super::settings_api::{AccountCallError, kind_of};
use super::{CredentialStores, Engine, LocalStateRefused};
use crate::account::SessionGeneration;
use crate::api_client::{ApiClient, ChunkUnreachable};
use crate::direct_download::DirectError;
use crate::engine_bridge::{EngineBridge, EngineError, InMemoryOpen, error_http_status};
use crate::state_db::FileStatus;

#[cfg(test)]
#[path = "viewer_api_tests.rs"]
mod tests;

/// How often a download under way asks whether it was cancelled or its session ended.
const ASK_EVERY: Duration = Duration::from_millis(50);

/// What the local state says about a file, and the session it was read for.
#[derive(Clone, PartialEq, Eq)]
pub struct OpenFileFacts {
    /// A host compares it with [`Engine::session_turn`] and drops facts whose session is gone.
    pub session: SessionGeneration,
    /// The file's name, decrypted (the last part of its path in the account's files).
    pub name: String,
    /// The folders above it, outermost first.
    pub folders: Vec<String>,
    /// Its size in bytes, as last synced.
    pub size: u64,
    /// What the local state says about the file's bytes on this computer. `None`: not here (online only, on
    /// its way down, or in a state that is not a copy to show). What is really on disk is asked again when the
    /// copy is read ([`Engine::open_local_copy_in_memory`]).
    pub on_this_pc: Option<OnThisPc>,
    /// The local state says this computer's copy and Beebeeb's version are not settled with each other (two
    /// versions to choose from, or a sync of this file that failed). Such a copy is not read as the file; a
    /// download shows the version Beebeeb has, and a host says that it may not be what is on this computer.
    pub unsettled_here: bool,
}

/// A file's bytes are on this computer, as the local state has it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnThisPc {
    /// The copy here is the version Beebeeb has.
    InSync,
    /// The copy here has a change that is not in Beebeeb yet.
    NotSentYet,
}

fn on_this_pc(status: &FileStatus) -> Option<OnThisPc> {
    match status {
        FileStatus::Local => Some(OnThisPc::InSync),
        FileStatus::Uploading => Some(OnThisPc::NotSentYet),
        // Not a copy to show as the file: not here, half here, two versions to choose from, or on its way out.
        FileStatus::CloudOnly
        | FileStatus::Downloading
        | FileStatus::Conflict
        | FileStatus::Error
        | FileStatus::Trashing => None,
    }
}

fn unsettled_here(status: &FileStatus) -> bool {
    matches!(status, FileStatus::Conflict | FileStatus::Error)
}

/// The session and the size, never the name or a folder: a `{:?}` must not print one.
impl std::fmt::Debug for OpenFileFacts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenFileFacts")
            .field("session", &self.session)
            .field("size", &self.size)
            .field("on_this_pc", &self.on_this_pc)
            .field("unsettled_here", &self.unsettled_here)
            .finish_non_exhaustive()
    }
}

/// A file's content, decrypted, and the session it was decrypted for. The bytes are wiped when this is dropped.
pub struct OpenedFile {
    /// A host compares it with [`Engine::session_turn`] and drops content whose session is gone.
    pub session: SessionGeneration,
    pub bytes: Zeroizing<Vec<u8>>,
}

/// How many bytes, never one of them.
impl std::fmt::Debug for OpenedFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenedFile")
            .field("session", &self.session)
            .field("bytes", &self.bytes.len())
            .finish()
    }
}

/// A file's content as it is on this computer, and the session it was read for. The bytes are wiped when this
/// is dropped.
pub struct OpenedLocalCopy {
    pub session: SessionGeneration,
    pub bytes: Zeroizing<Vec<u8>>,
    /// Whether this copy is what Beebeeb has, by the local state and the copy's own size.
    pub copy: OnThisPc,
}

/// How many bytes, never one of them.
impl std::fmt::Debug for OpenedLocalCopy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenedLocalCopy")
            .field("session", &self.session)
            .field("bytes", &self.bytes.len())
            .field("copy", &self.copy)
            .finish()
    }
}

/// What reading the copy on this computer came to.
#[derive(Debug)]
pub enum LocalCopy {
    /// The whole copy, read into memory.
    Opened(OpenedLocalCopy),
    /// The file's bytes are not wholly on this computer (or cannot be told to be, or no folder is set up for
    /// them): nothing was read and nothing was fetched. The caller downloads the file instead.
    NotHere,
}

/// Why a file was not opened. Fixed kinds: none carries a name, an id or a server's words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenFileError {
    /// No session in memory (signed out, or locked), or it changed while the call ran: nothing is handed over.
    NotSignedIn,
    /// Beebeeb no longer accepts this session.
    SessionEnded,
    /// The local state on this computer is not provably the signed-in account's: nothing of it is read.
    NotYours,
    /// No such file: not in the local state, a folder, or gone from Beebeeb.
    NotFound,
    /// The file is larger than the caller holds in memory, by its stated size or the copy's own length:
    /// `size`. Nothing of it was downloaded or read.
    TooLarge { size: u64 },
    /// The file passed the caller's limit while it was downloaded or read, although its stated size (or the
    /// copy's length when it was asked) was within it. It was given up and what was held was wiped. How large
    /// it is, is not known.
    OverLimit,
    /// This computer holds a change to the file that is not in Beebeeb yet, and the copy could not be read
    /// just now (another program holds it, it is being written, or it is not where the local state says). The
    /// version Beebeeb has is NOT handed over in its place.
    CopyNotReadable,
    /// The caller stopped the download; nothing of it is kept.
    Cancelled,
    /// Beebeeb could not be reached.
    Offline,
    /// The download or the decryption failed. Nothing changed.
    Failed,
}

impl OpenFileError {
    /// The kind in a word, for a log line.
    pub fn kind(self) -> &'static str {
        match self {
            Self::NotSignedIn => "not_signed_in",
            Self::SessionEnded => "session_ended",
            Self::NotYours => "not_this_accounts",
            Self::NotFound => "not_found",
            Self::TooLarge { .. } => "too_large",
            Self::OverLimit => "over_limit",
            Self::CopyNotReadable => "copy_not_readable",
            Self::Cancelled => "cancelled",
            Self::Offline => "offline",
            Self::Failed => "failed",
        }
    }
}

fn refused(refusal: LocalStateRefused) -> OpenFileError {
    match refusal {
        LocalStateRefused::SessionEnded => OpenFileError::SessionEnded,
        LocalStateRefused::NothingSynced => OpenFileError::NotFound,
        LocalStateRefused::Unreadable => OpenFileError::Failed,
        refusal if refusal.not_this_accounts() => OpenFileError::NotYours,
        _ => OpenFileError::NotSignedIn,
    }
}

/// The kind of a failed download, read from the error's chain (never its text). Every download route is named
/// here: the metadata request and the unpinned chunk route fail with the API client's own typed errors; the
/// pinned and the direct route with [`DirectError`] (an authorization answer or a not-found, never routed
/// around) or [`ChunkUnreachable`] (no whole answer after the route's own retries).
pub(crate) fn failure(error: EngineError) -> OpenFileError {
    if let Some(direct) = error.caused_by::<DirectError>() {
        return match direct {
            DirectError::Refused(401) => OpenFileError::SessionEnded,
            DirectError::Refused(404) => OpenFileError::NotFound,
            // Another refusal (403: no longer allowed), a version that kept changing, a chunk that did not
            // authenticate: the download failed, and nothing changed.
            DirectError::Refused(_) | DirectError::VersionChanged | DirectError::Corrupt => OpenFileError::Failed,
        };
    }
    if error.caused_by::<ChunkUnreachable>().is_some() {
        return OpenFileError::Offline;
    }
    let EngineError::Api(error) = error else {
        return OpenFileError::Failed;
    };
    if error_http_status(&error) == Some(404) {
        return OpenFileError::NotFound;
    }
    match kind_of(&error) {
        AccountCallError::SessionEnded => OpenFileError::SessionEnded,
        AccountCallError::Offline => OpenFileError::Offline,
        AccountCallError::NotSignedIn => OpenFileError::NotSignedIn,
        AccountCallError::ServerError | AccountCallError::Unreadable => OpenFileError::Failed,
    }
}

impl<C: CredentialStores> Engine<C> {
    /// What the signed-in account's local state says about the file `file_id`: its name, the folders above it and
    /// its size. Read through the account-bound handle (task 2488): a state that is not the session's answers
    /// nothing. A folder is [`OpenFileError::NotFound`]: only a file opens. No network.
    pub fn open_file_facts(&self, file_id: &str) -> Result<OpenFileFacts, OpenFileError> {
        // First: a session change during the read makes the facts older than the session, never newer.
        let session = self.acct.session_generation();
        let entry = self
            .local_state()
            .map_err(refused)?
            .with(|db| db.get_file(file_id))
            .map_err(refused)?
            .map_err(|_| OpenFileError::Failed)?
            .ok_or(OpenFileError::NotFound)?;
        if entry.is_dir() {
            return Err(OpenFileError::NotFound);
        }
        let mut parts: Vec<String> = entry
            .path
            .split('/')
            .filter(|part| !part.is_empty())
            .map(str::to_string)
            .collect();
        let name = parts.pop().ok_or(OpenFileError::NotFound)?;
        Ok(OpenFileFacts {
            session,
            name,
            folders: parts,
            size: u64::try_from(entry.size_bytes).unwrap_or(0),
            on_this_pc: on_this_pc(&entry.status),
            unsettled_here: unsettled_here(&entry.status),
        })
    }

    /// The content of the file `file_id` as it is ON THIS COMPUTER, read into memory for the session in memory:
    /// for a file kept here it needs no connection, and a change made here that is not sent yet is what it
    /// answers. No network, and nothing is written.
    ///
    /// [`LocalCopy::NotHere`] unless the local state says the file's bytes are here AND the file in the
    /// account's folder is a plain file whose every byte is on disk (on Windows a file that is not wholly on
    /// disk is never opened: opening it would fetch it; this process's own fetch of it is refused for the
    /// length of the read as well). A name that leads to a different file on this computer is `NotHere` too.
    ///
    /// A copy over `max_bytes` is [`OpenFileError::TooLarge`] from its length alone; the read itself is bounded
    /// too (a file that grows past the limit meanwhile is [`OpenFileError::OverLimit`], never more than
    /// `max_bytes` + 1 in memory). `cancel` and the session are asked between pieces. The hand-over is the same
    /// as a download's: only while the session in memory is still the one the read began for.
    ///
    /// A copy the local state says holds a change that is not sent yet, and that cannot be read now, is
    /// [`OpenFileError::CopyNotReadable`], never `NotHere`: the caller must not show the older version Beebeeb
    /// has as if it were the file.
    pub fn open_local_copy_in_memory(
        &self,
        file_id: &str,
        max_bytes: u64,
        cancel: &AtomicBool,
    ) -> Result<LocalCopy, OpenFileError> {
        let turn = self.acct.session_generation();
        let entry = self
            .local_state()
            .map_err(refused)?
            .with(|db| db.get_file(file_id))
            .map_err(refused)?
            .map_err(|_| OpenFileError::Failed)?
            .ok_or(OpenFileError::NotFound)?;
        if entry.is_dir() {
            return Err(OpenFileError::NotFound);
        }
        let Some(recorded) = on_this_pc(&entry.status) else {
            return Ok(LocalCopy::NotHere);
        };
        // The key of the session the read is for: the hand-over below asks whether it is still that session.
        let key = {
            let guard = self.acct.session.lock().map_err(|_| OpenFileError::NotSignedIn)?;
            let current = guard.as_ref().ok_or(OpenFileError::NotSignedIn)?;
            if !self.acct.session_unchanged_since(turn) {
                return Err(OpenFileError::NotSignedIn);
            }
            Zeroizing::new(current.master_key)
        };
        // A copy that is the version Beebeeb has may be downloaded instead; one with a change that is not sent
        // may not.
        let not_read = || match recorded {
            OnThisPc::InSync => Ok(LocalCopy::NotHere),
            OnThisPc::NotSentYet => Err(OpenFileError::CopyNotReadable),
        };
        let Some(path) = self.local_copy_path(&entry.path) else {
            return not_read();
        };
        // This process's own fetch of the file is refused until the read is over: a file that stops being
        // wholly on disk between the check and the open is not downloaded by the open.
        let _no_fetch = crate::no_fetch_scope::enter(&[file_id]);
        let still = || self.acct.session_unchanged_since(turn);
        let mut tries = 0;
        let bytes = loop {
            let Some(length) = wholly_here(&path) else {
                return not_read();
            };
            if length > max_bytes {
                return Err(OpenFileError::TooLarge { size: length });
            }
            match read_bounded(&path, length, max_bytes, cancel, &still) {
                Ok(bytes) => break bytes,
                Err(ReadStopped::Refused(error)) => return Err(error),
                // It went away, or cannot be read, between the check and the read: the download is the way
                // for a copy in sync.
                Err(ReadStopped::Unreadable) => return not_read(),
                // It is being written: its length is asked again, a few times. Then it could not be read for
                // now (never the version Beebeeb has in its place: this copy is the newer one).
                Err(ReadStopped::Grew) if tries < GREW_TRIES => tries += 1,
                Err(ReadStopped::Grew) => return Err(OpenFileError::CopyNotReadable),
            }
        };
        if !self.session_is_still(turn, &key) {
            return Err(OpenFileError::NotSignedIn);
        }
        // A copy whose length is not the synced size was changed here, whatever the row says so far.
        let copy = if recorded == OnThisPc::InSync && u64::try_from(entry.size_bytes).ok() == Some(bytes.len() as u64) {
            OnThisPc::InSync
        } else {
            OnThisPc::NotSentYet
        };
        Ok(LocalCopy::Opened(OpenedLocalCopy {
            session: turn,
            bytes,
            copy,
        }))
    }

    /// Where the file at `rel_path` of the local state is in the account's folder on this computer. `None`: no
    /// folder is known, the path is not one below it, or its name leads to a different file here (a Windows
    /// short name): that file is never read as this one.
    fn local_copy_path(&self, rel_path: &str) -> Option<PathBuf> {
        let root = match self.configured_sync_root().ok()? {
            Some(root) => root,
            // No folder named in the settings: the identity's own, as `Engine::sync_root` answers. A test names
            // its folder or has none (it never reads the developer's own).
            #[cfg(not(any(test, feature = "test-support")))]
            None => crate::config::default_sync_root_suggestion(self.identity),
            #[cfg(any(test, feature = "test-support"))]
            None => return None,
        };
        // As the Files window resolves a row for the system's Open: below the folder, by plain names only.
        let path = crate::file_ops::full_path(&root, rel_path).ok()?;
        matches!(crate::name_alias::conflict_below(&root, &path), Ok(None)).then_some(path)
    }
}

/// The length of the plain file at `path` when every byte of it is on disk, asked from its attributes alone
/// (nothing is opened for data). `None`: not a plain file (a link, a folder, gone), or not wholly on disk.
fn wholly_here(path: &Path) -> Option<u64> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::MetadataExt;
        // FILE_ATTRIBUTE_OFFLINE, _RECALL_ON_OPEN, _RECALL_ON_DATA_ACCESS: reading it would fetch it.
        const NOT_ON_DISK: u32 = 0x0000_1000 | 0x0004_0000 | 0x0040_0000;
        if metadata.file_attributes() & NOT_ON_DISK != 0 {
            return None;
        }
    }
    Some(metadata.len())
}

/// How often a copy that grew while it was read is read again.
const GREW_TRIES: u32 = 3;

enum ReadStopped {
    /// Cancelled, the session moved on, or the file is past the limit.
    Refused(OpenFileError),
    Unreadable,
    /// The file is longer than the length it was read for, and (so far) within the limit.
    Grew,
}

/// The file at `path` into one buffer that is wiped when dropped, in pieces with `cancel` and `still` asked
/// between them. The buffer is sized once, for `length` + 1 bytes and never more than `max_bytes` + 1, and the
/// read never goes past it: no earlier copy of the content is left behind by a growing allocation, and a file
/// that is longer than `length` by now costs one byte more, not the file.
fn read_bounded(
    path: &Path,
    length: u64,
    max_bytes: u64,
    cancel: &AtomicBool,
    still: &dyn Fn() -> bool,
) -> Result<Zeroizing<Vec<u8>>, ReadStopped> {
    const PIECE: usize = 1 << 20;
    let file = std::fs::File::open(path).map_err(|_| ReadStopped::Unreadable)?;
    // Asked again through the handle: it describes the file the read then reads.
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::MetadataExt;
        const NOT_ON_DISK: u32 = 0x0000_1000 | 0x0004_0000 | 0x0040_0000;
        match file.metadata() {
            Ok(metadata) if metadata.is_file() && metadata.file_attributes() & NOT_ON_DISK == 0 => {}
            _ => return Err(ReadStopped::Unreadable),
        }
    }
    let ceiling = max_bytes.saturating_add(1);
    let room = usize::try_from(length.saturating_add(1).min(ceiling)).map_err(|_| ReadStopped::Unreadable)?;
    let mut bytes = Zeroizing::new(Vec::with_capacity(room));
    let mut file = file;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(ReadStopped::Refused(OpenFileError::Cancelled));
        }
        if !still() {
            return Err(ReadStopped::Refused(OpenFileError::NotSignedIn));
        }
        // Never past the buffer's own room.
        let want = PIECE.min(bytes.capacity() - bytes.len());
        if want == 0 {
            break;
        }
        let at = bytes.len();
        bytes.resize(at + want, 0);
        let read = match file.read(&mut bytes[at..]) {
            Ok(read) => read,
            Err(_) => return Err(ReadStopped::Unreadable),
        };
        bytes.truncate(at + read);
        if read == 0 {
            break;
        }
    }
    if bytes.len() as u64 > max_bytes {
        return Err(ReadStopped::Refused(OpenFileError::OverLimit));
    }
    // The file ended where its length said, or the buffer is full: one more byte means it grew.
    if bytes.len() as u64 > length {
        return Err(ReadStopped::Grew);
    }
    Ok(bytes)
}

impl<C: CredentialStores> Engine<C> {
    /// The content of the file `file_id`, downloaded and decrypted in memory for the session in memory, by the
    /// download path every hydrate uses. See the module's doc for what is (not) written and the session rule.
    ///
    /// A file over `max_bytes` by its stated size is [`OpenFileError::TooLarge`] before a chunk is asked for;
    /// one that passes it while it downloads is [`OpenFileError::OverLimit`]. `progress` is
    /// `(decrypted bytes, bytes in all)`, called from the download as it goes: it must be cheap. Once `cancel` is
    /// set, or the session changes, the download is dropped within [`ASK_EVERY`]: what was decrypted until then
    /// is wiped, and the answer is [`OpenFileError::Cancelled`] or [`OpenFileError::NotSignedIn`].
    pub fn open_file_in_memory(
        &self,
        file_id: &str,
        max_bytes: u64,
        progress: &(dyn Fn(u64, u64) + Send + Sync + 'static),
        cancel: &AtomicBool,
    ) -> Result<OpenedFile, OpenFileError> {
        let turn = self.acct.session_generation();
        // The account-bound handle first (it answers nothing for a session the server ended), then the token and the key of the SAME session: copied under the
        // session's lock, and only while no session change happened since `turn` (the handle is that turn's or it
        // answers nothing). Both are copies of their own, wiped when this call ends (inside the call's client; the
        // key also in a `Zeroizing`).
        let state = self.local_state_for_changes().map_err(refused)?;
        let (token, key, session) = {
            let guard = self.acct.session.lock().map_err(|_| OpenFileError::NotSignedIn)?;
            let current = guard.as_ref().ok_or(OpenFileError::NotSignedIn)?;
            if !self.acct.session_unchanged_since(turn) {
                return Err(OpenFileError::NotSignedIn);
            }
            (current.token.clone(), Zeroizing::new(current.master_key), turn)
        };
        let fetched = state
            .with_shared(|db| {
                let api = Arc::new(ApiClient::new(self.config.api_base.clone(), token, *key));
                let bridge = EngineBridge::new(self.identity, db, api);
                self.runtime.block_on(async {
                    let mut fetch = std::pin::pin!(bridge.open_in_memory(file_id, max_bytes, Some(progress)));
                    loop {
                        match tokio::time::timeout(ASK_EVERY, fetch.as_mut()).await {
                            Ok(done) => break Ok(done),
                            Err(_) if cancel.load(Ordering::SeqCst) => break Err(OpenFileError::Cancelled),
                            Err(_) if !self.acct.session_unchanged_since(session) => {
                                break Err(OpenFileError::NotSignedIn);
                            }
                            Err(_) => {}
                        }
                    }
                })
            })
            // The session changed or ended while the download ran: what it answered was dropped (and wiped) in the
            // handle, and the refusal keeps its kind.
            .map_err(refused)??;
        let bytes = match fetched.map_err(failure)? {
            InMemoryOpen::Opened(bytes) => bytes,
            InMemoryOpen::TooLarge { size } => return Err(OpenFileError::TooLarge { size }),
            InMemoryOpen::OverLimit { .. } => return Err(OpenFileError::OverLimit),
        };
        if cancel.load(Ordering::SeqCst) {
            return Err(OpenFileError::Cancelled);
        }
        // The hand-over: only while the session in memory is the one the token and the key were copied from,
        // asked with no session change half done. Content decrypted for a session that is ending is wiped here.
        if !self.session_is_still(session, &key) {
            return Err(OpenFileError::NotSignedIn);
        }
        Ok(OpenedFile { session, bytes })
    }
}
