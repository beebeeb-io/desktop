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
//!   file, whose edit was never sent and is in no history: its bytes are copied first to where nothing syncs, the
//!   engine's own app data, per account and one folder per file ([`REPLACED_FOLDER`]),
//!   and when that copy cannot be made nothing is replaced. When Keep there works the copy stays there (a later Keep
//!   there of the same file adds its copy beside it, and the same bytes are kept once); when it fails the copy stays
//!   there too (data safety over disk: a failure may have replaced the file first). No earlier copy is ever deleted.
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
use crate::engine_bridge::{EngineBridge, PlaceSeen, local_file_path_under_sync_root};
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

/// The folder, in the engine's own app data (never in the sync folder: nothing in it is uploaded), where Keep there
/// keeps this PC's version of a file before the server's version replaces it:
/// `<app data>/Replaced on this PC/<account>/<file>/<the file's name>`. One copy per file: a later Keep there of the
/// same file replaces it. `<account>` is a digest of the account's user id (its email when no id is known) and
/// `<file>` one of the file's id: never an id itself, and never a path a name or an id could lead out of the folder.
/// Sign-out leaves it: it is the person's own data.
pub const REPLACED_FOLDER: &str = concat!("Replaced on ", crate::device_word!());

/// The name of the copy being made, beside the kept one, until Keep there is known to have worked.
const INCOMING: &str = ".incoming";

/// 16 hex characters of the SHA-256 of `text`: a folder name that holds nothing of `text` and cannot be a path.
pub(super) fn digest_name(text: &str) -> String {
    use sha2::Digest as _;
    sha2::Sha256::digest(text.as_bytes())
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// `<state dir>/Replaced on this PC/<account digest>`: the account's folder of replaced versions.
pub fn replaced_folder_in(state_dir: &Path, account: &crate::account_binding::Identity) -> Option<PathBuf> {
    let who = account.user_id.as_deref().or(account.email.as_deref())?;
    Some(state_dir.join(REPLACED_FOLDER).join(digest_name(who)))
}

/// `<account's folder>/<file digest>`: the one folder that holds the kept copy of the file with this id.
pub fn replaced_folder_of(account_folder: &Path, file_id: &str) -> PathBuf {
    account_folder.join(digest_name(file_id))
}

/// This PC's version of one file, copied aside before Keep there replaces it (see [`REPLACED_FOLDER`]).
struct SafetyCopy {
    /// The copy as made, not yet the file's kept copy.
    incoming: PathBuf,
    /// Where it is kept once Keep there ran (it worked or failed: the copy stays either way).
    kept: PathBuf,
}

fn sha256_of(file: &Path) -> std::io::Result<[u8; 32]> {
    use sha2::Digest as _;
    use std::io::Read;
    let mut source = std::fs::File::open(file)?;
    let mut hasher = sha2::Sha256::new();
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        let read = source.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().into())
}

impl SafetyCopy {
    /// Copy `file` into `folder` (made when missing) as the incoming copy. The file's earlier kept copy, if any, is
    /// not touched yet. An incoming copy a run that ended early left behind goes only when it holds what `file` still
    /// is; otherwise it is the only copy of a version that was replaced, and it becomes the kept copy first.
    fn make(file: &Path, folder: &Path) -> std::io::Result<Self> {
        use std::io::{Read, Write};
        let name = file
            .file_name()
            .ok_or_else(|| std::io::Error::other("no file name"))?
            .to_os_string();
        std::fs::create_dir_all(folder)?;
        let incoming = folder.join(INCOMING);
        match std::fs::symlink_metadata(&incoming) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
            Ok(_) => {
                let spare = matches!((sha256_of(&incoming), sha256_of(file)), (Ok(left), Ok(now)) if left == now);
                if spare {
                    std::fs::remove_file(&incoming)?;
                } else {
                    Self {
                        incoming: incoming.clone(),
                        kept: folder.join(&name),
                    }
                    .keep()?;
                }
            }
        }
        let mut source = std::fs::File::open(file)?;
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&incoming)?;
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
        drop(out);
        if let Err(e) = copied {
            let _ = std::fs::remove_file(&incoming);
            return Err(e);
        }
        Ok(Self {
            incoming,
            kept: folder.join(name),
        })
    }

    /// Keep there ran (it worked or failed): this copy becomes one of the file's kept copies. NOTHING kept earlier is
    /// ever deleted (an earlier copy can be the only copy of an edit that is in no
    /// history). A copy of the same bytes as one already kept is not kept twice (the incoming one goes); a different
    /// edit is kept beside the earlier ones under a free name. Returns where the bytes are kept.
    fn keep(self) -> std::io::Result<PathBuf> {
        let folder = self.kept.parent().ok_or_else(|| std::io::Error::other("no folder"))?;
        let incoming_digest = sha256_of(&self.incoming)?;
        for entry in std::fs::read_dir(folder)? {
            let path = entry?.path();
            if path != self.incoming && sha256_of(&path).is_ok_and(|kept| kept == incoming_digest) {
                std::fs::remove_file(&self.incoming)?;
                return Ok(path);
            }
        }
        let target = free_name_beside(&self.kept);
        std::fs::rename(&self.incoming, &target)?;
        Ok(target)
    }
}

/// `kept`, or `<stem> (2).<ext>`, `(3)`, ... when that name is taken: the first one that is free.
fn free_name_beside(kept: &Path) -> PathBuf {
    if !kept.exists() {
        return kept.to_path_buf();
    }
    let stem = kept
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = kept
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    (2u32..)
        .map(|n| kept.with_file_name(format!("{stem} ({n}){ext}")))
        .find(|candidate| !candidate.exists())
        .unwrap_or_else(|| kept.to_path_buf())
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
        // The edit on this PC was never sent: it is in no version history. Keep its bytes, where nothing syncs, before
        // they go. S6 (R9): the hydrate's start look is taken first, before the copy reads a byte, and handed to it: a
        // save after the look (between the copy and the download, or during the download) is never replaced.
        let mut safety = None;
        let mut seen = None;
        if choice == ConflictChoice::KeepThere {
            let path = local_file_path_under_sync_root(&root, &entry.path).map_err(|_| ResolveRefused::Failed)?;
            seen = Some(PlaceSeen::take(&path).map_err(|_| ResolveRefused::Failed)?);
            if path.is_file() {
                let folder = self
                    .session_identity()
                    .ok()
                    .and_then(|account| replaced_folder_in(&self.config.state_dir, &account))
                    .map(|account_folder| replaced_folder_of(&account_folder, file_id))
                    .ok_or(ResolveRefused::CopyFailed)?;
                let copy = SafetyCopy::make(&path, &folder).map_err(|_| ResolveRefused::CopyFailed)?;
                safety = Some((copy, path));
            }
        }
        let done = state
            .with_shared(|db| {
                let api = Arc::new(ApiClient::new(self.config.api_base.clone(), token, *key));
                let bridge = EngineBridge::new(self.identity, db, api);
                self.runtime.block_on(async {
                    match choice {
                        ConflictChoice::KeepHere => bridge.resolve_keep_mine(file_id, &root).await,
                        ConflictChoice::KeepThere => match seen {
                            Some(seen) => bridge.resolve_theirs_seen(file_id, &root, seen).await,
                            None => Err(anyhow::anyhow!("keep there took no look at the place")),
                        },
                        ConflictChoice::KeepBoth => bridge.auto_resolve_keep_both(&root, &entry).await.map(|_| ()),
                    }
                })
            })
            // The session moved while the call ran.
            .map_err(local::<ResolveRefused>);
        // Data safety over disk (lead, 2026-11-14 21:20): the copy is kept whether Keep there worked or not. A failure
        // may have replaced this PC's file before it failed, and the copy is then all that is left of the edit.
        if let Some((copy, _)) = safety
            && copy.keep().is_err()
        {
            tracing::warn!("keep there ran; this PC's version stays in the incoming copy");
        }
        match done? {
            Ok(()) => {
                self.request_resnapshot();
                Ok(())
            }
            Err(_) => Err(ResolveRefused::Failed),
        }
    }
}
