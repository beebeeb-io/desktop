//! Two versions of a file, for a host without a UI toolkit (the native Files window's conflict dialog): the files
//! that changed in two places and wait for a choice ([`Engine::conflicts_for`]) and the answer to one
//! ([`Engine::resolve_conflict_for`]), each bound to the session of the press and answering only for it.
//!
//! - Only rows in `FileStatus::Conflict` are listed. The other kinds of the review feed (held edits, review
//!   operations, kept copies) stay where they are shown: This PC's "did not sync" list.
//! - "Here" is read from the file on this PC (its length and last change on disk), never from the state row (which
//!   holds the day the conflict was found). "There" is the server's current version of the file (when, size). The
//!   server names no device, so none is said. A side that cannot be read is not guessed: the file is counted in
//!   [`Conflicts::unread`] and left out.
//! - An answer writes. Keep here uploads this PC's file as a new version (the server's stays in the history); Keep
//!   both renames this PC's file beside the original and downloads the server's; Keep there REPLACES this PC's
//!   file, whose edit was never sent and is in no history: its bytes are copied beside the file first
//!   ([`replaced_copy_name`]), and when that copy cannot be made nothing is replaced.
//! - A session that moved on is [`ConflictsRefused::NotSignedIn`] / [`ResolveRefused::NotSignedIn`], with nothing
//!   sent and nothing handed over.
//!
//! BLOCKS on the handle's runtime: both calls are a worker's. Must not: log a name, an id or anything the server
//! sends; copy a token or a key outside the session-write lock's question.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use zeroize::Zeroizing;

use super::settings_api::AccountCallError;
use super::{CredentialStores, Engine, LocalStateRefused};
use crate::account::SessionGeneration;
use crate::api_client::ApiClient;
use crate::engine_bridge::{EngineBridge, local_file_path_under_sync_root};
use crate::state_db::FileStatus;

#[cfg(test)]
#[path = "conflicts_api_tests.rs"]
mod tests;

/// One file that waits for a choice, with both of its sides.
#[derive(Clone, PartialEq, Eq)]
pub struct WaitingConflict {
    pub file_id: String,
    /// The file's name (the last part of its path).
    pub name: String,
    /// The file on this PC: its last change on disk as `2026-09-01T10:00:00Z`, and its length in bytes.
    pub here_modified: String,
    pub here_size: u64,
    /// The server's current version: when it was made (the server's own timestamp) and its size in bytes.
    pub there_created: String,
    pub there_size: u64,
}

/// No name, no date: a `{:?}` prints nothing of a file.
impl std::fmt::Debug for WaitingConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WaitingConflict").finish_non_exhaustive()
    }
}

/// The files that wait for a choice, and the session the list was read for.
#[derive(Clone, Debug)]
pub struct Conflicts {
    pub session: SessionGeneration,
    pub files: Vec<WaitingConflict>,
    /// Files in conflict that are not in `files` because a side could not be read now (offline, a file moved away
    /// on this PC). They still wait; they are not drawn until both sides are known.
    pub unread: usize,
}

/// Why the list was not made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictsRefused {
    /// The press's session is gone or moved on: nothing was sent, nothing is handed over.
    NotSignedIn,
    /// The account's local state is not handed out.
    NoLocalState,
    /// No sync folder is set.
    NoSyncFolder,
}

/// The answer a person gave.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictChoice {
    /// This PC's file stays the file (uploaded as a new version).
    KeepHere,
    /// The server's version becomes the file on this PC (this PC's copy is kept beside it first).
    KeepThere,
    /// Both: this PC's file is renamed beside the original, the server's version takes the original name.
    KeepBoth,
}

/// Why an answer was not applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolveRefused {
    /// The press's session is gone or moved on: nothing was done.
    NotSignedIn,
    /// The account's local state is not handed out.
    NoLocalState,
    /// No sync folder is set.
    NoSyncFolder,
    /// The id is not a file of this account that waits for a choice: nothing was done.
    NotWaiting,
    /// This PC's file could not be kept beside the original, so nothing was replaced.
    CopyFailed,
    /// The call failed, by its kind.
    Call(AccountCallError),
    /// The work started and did not finish; the file still waits (or both of its versions are on disk).
    Failed,
}

fn local<T: From<LocalRefusal>>(refusal: LocalStateRefused) -> T {
    LocalRefusal(refusal).into()
}

struct LocalRefusal(LocalStateRefused);

impl From<LocalRefusal> for ConflictsRefused {
    fn from(refusal: LocalRefusal) -> Self {
        match refusal.0 {
            LocalStateRefused::SessionChanged
            | LocalStateRefused::SessionEnded
            | LocalStateRefused::SignedOut
            | LocalStateRefused::Locked => ConflictsRefused::NotSignedIn,
            _ => ConflictsRefused::NoLocalState,
        }
    }
}

impl From<LocalRefusal> for ResolveRefused {
    fn from(refusal: LocalRefusal) -> Self {
        match ConflictsRefused::from(refusal) {
            ConflictsRefused::NotSignedIn => ResolveRefused::NotSignedIn,
            _ => ResolveRefused::NoLocalState,
        }
    }
}

/// `2026-09-01T10:00:00Z` for a time on disk.
fn iso(time: SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(time).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// The name this PC's file is kept under before the server's version replaces it: `name (replaced on this PC)`,
/// then ` 2`, ` 3` while the name is taken; the extension stays last.
pub fn replaced_copy_name(original: &str, n: u32) -> String {
    let suffix = if n <= 1 {
        " (replaced on this PC)".to_string()
    } else {
        format!(" (replaced on this PC) {n}")
    };
    crate::sign_out::kept_names::with_suffix(original, &suffix)
}

/// Copy `file` beside itself under [`replaced_copy_name`], never over another file.
fn keep_replaced_copy(file: &Path) -> std::io::Result<PathBuf> {
    use std::io::{Read, Write};
    let name = file
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| std::io::Error::other("no file name"))?;
    let mut source = std::fs::File::open(file)?;
    for n in 1u32..=100 {
        let target = file.with_file_name(replaced_copy_name(name, n));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&target) {
            Ok(mut out) => {
                let copied = (|| -> std::io::Result<()> {
                    let mut buffer = vec![0u8; 256 * 1024];
                    loop {
                        let read = source.read(&mut buffer)?;
                        if read == 0 {
                            break;
                        }
                        out.write_all(&buffer[..read])?;
                    }
                    out.sync_all()
                })();
                if let Err(e) = copied {
                    drop(out);
                    let _ = std::fs::remove_file(&target);
                    return Err(e);
                }
                return Ok(target);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("no free name for the copy"))
}

impl<C: CredentialStores> Engine<C> {
    /// The files that wait for a choice, each with this PC's file and the server's current version, for the session
    /// a person pressed under. See the module's doc.
    pub fn conflicts_for(&self, session: SessionGeneration) -> Result<Conflicts, ConflictsRefused> {
        let state = self.local_state().map_err(local::<ConflictsRefused>)?;
        if state.session() != session {
            return Err(ConflictsRefused::NotSignedIn);
        }
        let waiting = state
            .with(|db| db.list_by_status(FileStatus::Conflict).unwrap_or_default())
            .map_err(local::<ConflictsRefused>)?;
        drop(state);
        let waiting: Vec<_> = waiting.into_iter().filter(|entry| !entry.is_dir()).collect();
        if waiting.is_empty() {
            return Ok(Conflicts {
                session,
                files: Vec::new(),
                unread: 0,
            });
        }
        let root = self
            .configured_sync_root()
            .ok()
            .flatten()
            .ok_or(ConflictsRefused::NoSyncFolder)?;
        let mut files = Vec::new();
        let mut unread = 0;
        for entry in waiting {
            let here = local_file_path_under_sync_root(&root, &entry.path)
                .ok()
                .and_then(|path| std::fs::metadata(path).ok())
                .filter(|meta| meta.is_file())
                .and_then(|meta| Some((iso(meta.modified().ok()?), meta.len())));
            let there = self
                .file_versions_for(session, &entry.file_id)
                .ok()
                .and_then(|listed| listed.versions.into_iter().find(|version| version.current));
            match (here, there) {
                (Some((here_modified, here_size)), Some(there)) => files.push(WaitingConflict {
                    name: entry.path.rsplit('/').next().unwrap_or(&entry.path).to_string(),
                    file_id: entry.file_id,
                    here_modified,
                    here_size,
                    there_created: there.created_at,
                    there_size: there.size.max(0) as u64,
                }),
                _ => unread += 1,
            }
        }
        // A list made for a session that ended is not handed over.
        if self.acct.while_session_is(&self.writes, session, |_| ()).is_none() {
            return Err(ConflictsRefused::NotSignedIn);
        }
        Ok(Conflicts { session, files, unread })
    }

    /// WRITES: apply `choice` to the file `file_id`, which must wait for a choice, for the session a person pressed
    /// under and no other. See the module's doc for what each choice does to the file on this PC.
    pub fn resolve_conflict_for(
        &self,
        session: SessionGeneration,
        file_id: &str,
        choice: ConflictChoice,
    ) -> Result<(), ResolveRefused> {
        if self.acct.session.is_poisoned() {
            return Err(ResolveRefused::NotSignedIn);
        }
        let (token, key) = self
            .acct
            .while_session_is(&self.writes, session, |current| {
                (current.token.clone(), Zeroizing::new(current.master_key))
            })
            .ok_or(ResolveRefused::NotSignedIn)?;
        let state = self.local_state_for_changes().map_err(local::<ResolveRefused>)?;
        if state.session() != session {
            return Err(ResolveRefused::NotSignedIn);
        }
        let entry = state
            .with(|db| db.get_file(file_id).ok().flatten())
            .map_err(local::<ResolveRefused>)?
            .filter(|entry| !entry.is_dir() && entry.status == FileStatus::Conflict)
            .ok_or(ResolveRefused::NotWaiting)?;
        let root = self
            .configured_sync_root()
            .ok()
            .flatten()
            .ok_or(ResolveRefused::NoSyncFolder)?;
        if choice == ConflictChoice::KeepThere {
            // The edit on this PC was never sent: it is in no version history. Keep its bytes before they go.
            if let Ok(path) = local_file_path_under_sync_root(&root, &entry.path)
                && path.is_file()
                && keep_replaced_copy(&path).is_err()
            {
                return Err(ResolveRefused::CopyFailed);
            }
        }
        let done = state
            .with_shared(|db| {
                let api = Arc::new(ApiClient::new(self.config.api_base.clone(), token, *key));
                let bridge = EngineBridge::new(self.identity, db, api);
                self.runtime.block_on(async {
                    match choice {
                        ConflictChoice::KeepHere => bridge.resolve_keep_mine(file_id, &root).await,
                        ConflictChoice::KeepThere => bridge.resolve_keep_theirs(file_id, &root).await,
                        ConflictChoice::KeepBoth => bridge.auto_resolve_keep_both(&root, &entry).await.map(|_| ()),
                    }
                })
            })
            // The session moved while the call ran.
            .map_err(local::<ResolveRefused>)?;
        match done {
            Ok(()) => {
                self.request_resnapshot();
                Ok(())
            }
            Err(_) => Err(ResolveRefused::Failed),
        }
    }
}
