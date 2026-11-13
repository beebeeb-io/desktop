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
//! The handle is the one for changes: resolving the key of a file in a shared folder may record that folder's
//! custody (the engine's own ratchet, as every other download of such a file does), and that is the only thing
//! this module can cause to be written.
//!
//! BLOCKS: [`Engine::open_file_in_memory`] is a worker's (the network); [`Engine::open_file_facts`] reads the
//! local state alone, as a listing does. Must not: log or return a name, a path, a key or a byte of content
//! outside the two answer types; write a decrypted byte anywhere.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use zeroize::Zeroizing;

use super::settings_api::{AccountCallError, kind_of};
use super::{CredentialStores, Engine, LocalStateRefused};
use crate::account::SessionGeneration;
use crate::api_client::ApiClient;
use crate::engine_bridge::{EngineBridge, EngineError, InMemoryOpen, error_http_status};

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
}

/// The session and the size, never the name or a folder: a `{:?}` must not print one.
impl std::fmt::Debug for OpenFileFacts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenFileFacts")
            .field("session", &self.session)
            .field("size", &self.size)
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
    /// The file is larger than the caller holds in memory; nothing of it was downloaded.
    TooLarge { size: u64 },
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

/// The kind of a failed download, read from the error's chain (never its text).
fn failure(error: EngineError) -> OpenFileError {
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
        })
    }

    /// The content of the file `file_id`, downloaded and decrypted in memory for the session in memory, by the
    /// download path every hydrate uses. See the module's doc for what is (not) written and the session rule.
    ///
    /// A file over `max_bytes` is [`OpenFileError::TooLarge`] before a chunk is asked for. `progress` is
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
