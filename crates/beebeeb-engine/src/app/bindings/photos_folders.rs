//! Photo sources' "Folders from Files" (DW-0604; slice K4 of batch 2, on B1's file).
//!
//! Session-bound like its neighbours (`*_for(session)`): every call is asked for the session the host read at its
//! press, and its answer is handed over only while that session is still the one in memory.
//!
//! What Photos shows from a folder of Files is a row of the ACCOUNT's local state (`state_db::photo_folders`, keyed
//! by the session's own user id): the folders the person added, each with its switch. The library rule lives in
//! `photos_api::photos_library`: a photo at or under an added folder that is switched OFF is left out (its
//! subfolders' photos too); a folder switched on changes nothing; removing a folder shows its photos again; with no
//! folder added the library is what it always was. Nothing is moved or deleted, and backups keep running: this only
//! changes what Photos shows.
//!
//! Only a folder of the account's own tree can be added: not a file, not the Backups root or anything under it, not
//! a row someone else shares with the account (`finder_show::item_namespace`).
//!
//! BLOCKS on the state database: a worker's call, never the window's thread. Must not: log a name, a path, a user
//! id or a file id.

use crate::account::SessionGeneration;
use crate::app::photos_api::at_or_under;
use crate::app::{CredentialStores, Engine, LocalStateRefused, OwnedState};
use crate::drive_v2::model::{is_photo_or_video, leaf_name, live_rows};
use crate::finder_show::item_namespace;
use crate::state_db::{Namespace, StateDb};

/// One folder from Files that Photos shows (Photo sources). Holds a decrypted name: its `Debug` prints none.
#[derive(Clone, PartialEq, Eq)]
pub struct PhotoFolder {
    /// The folder's file id.
    pub id: String,
    /// The folder's own name ("Lisbon trip").
    pub name: String,
    /// The photos and videos at or under it, its subfolders' included.
    pub items: u64,
    /// Whether Photos shows them.
    pub shown: bool,
}

impl std::fmt::Debug for PhotoFolder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhotoFolder")
            .field("items", &self.items)
            .field("shown", &self.shown)
            .finish_non_exhaustive()
    }
}

/// Why a photo folder call answered nothing or changed nothing. A fixed word each; nothing was changed in any.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhotoFoldersRefused {
    /// No session in memory, it is locked, or it is not the session of the press any more.
    NotSignedIn,
    /// Add: the id is not a folder of the account's own tree (a file, a Backups row, a row someone else shares, an
    /// unknown id). Switch and Remove: the account did not add that folder.
    NotAFolder,
    /// The account's local state could not be read or written, is not provably the account's, or the session's
    /// user id is not known (a row is keyed by it, never by an email).
    NotSaved,
}

impl PhotoFoldersRefused {
    /// A fixed word for a log line.
    pub fn kind(self) -> &'static str {
        match self {
            Self::NotSignedIn => "not signed in",
            Self::NotAFolder => "not a folder",
            Self::NotSaved => "not saved",
        }
    }
}

fn refused(refusal: LocalStateRefused) -> PhotoFoldersRefused {
    match refusal {
        LocalStateRefused::SignedOut
        | LocalStateRefused::Locked
        | LocalStateRefused::SessionEnded
        | LocalStateRefused::SessionChanged => PhotoFoldersRefused::NotSignedIn,
        LocalStateRefused::NothingSynced
        | LocalStateRefused::Unowned
        | LocalStateRefused::OtherAccount
        | LocalStateRefused::IdentityUnknown
        | LocalStateRefused::Unreadable => PhotoFoldersRefused::NotSaved,
    }
}

/// What a change does to the account's rows.
enum Change<'a> {
    Add(&'a str),
    Shown(&'a str, bool),
    Remove(&'a str),
}

/// Whether `item_id` is a folder of the account's own tree that Photos may show: a folder row of My files, not the
/// Backups root and not under it. `Err`: the state could not be read.
fn own_folder(db: &StateDb, item_id: &str) -> Result<bool, String> {
    if item_namespace(db, item_id)? != Some(Namespace::MyFiles) {
        return Ok(false);
    }
    let Some(row) = db.get_file(item_id).map_err(|error| format!("read item: {error}"))? else {
        return Ok(false);
    };
    if !row.is_dir() {
        return Ok(false);
    }
    let roots = db.backups_roots().map_err(|error| format!("read item: {error}"))?;
    let backups = roots
        .iter()
        .any(|(root_id, root_path)| *root_id == row.file_id || at_or_under(&row.path, root_path));
    Ok(!backups)
}

/// The account `user_id`'s folders as Photo sources lists them: a folder that no longer exists is not listed.
/// `None`: the state could not be read.
fn folders_of(db: &StateDb, user_id: &str) -> Option<Vec<PhotoFolder>> {
    let added = db.photo_folders(user_id).ok()?;
    if added.is_empty() {
        return Some(Vec::new());
    }
    let rows = live_rows(db).ok()?;
    let mut out = Vec::with_capacity(added.len());
    for (id, shown) in added {
        let Some(folder) = rows.iter().find(|row| row.is_dir() && row.file_id == id) else {
            continue;
        };
        let path = folder.path.trim_matches('/');
        let items = rows
            .iter()
            .filter(|row| !row.is_dir() && at_or_under(&row.path, path) && is_photo_or_video(&leaf_name(row)))
            .count() as u64;
        out.push(PhotoFolder {
            name: leaf_name(folder),
            id,
            items,
            shown,
        });
    }
    Some(out)
}

impl<C: CredentialStores> Engine<C> {
    /// The folders from Files the account of `session` added to Photo sources, in the order they were added, each
    /// with its picture count and its switch. Nothing synced on this PC yet: none. A session that moved on, or moves
    /// on while this reads: `NotSignedIn`, and the host drops that answer.
    pub fn photo_folders_for(&self, session: SessionGeneration) -> Result<Vec<PhotoFolder>, PhotoFoldersRefused> {
        let (state, user_id) = match self.photo_folder_state(session, false) {
            Ok(found) => found,
            // No local state yet: nothing was added on this PC.
            Err(Some(LocalStateRefused::NothingSynced)) => return Ok(Vec::new()),
            Err(Some(refusal)) => return Err(refused(refusal)),
            Err(None) => return Err(PhotoFoldersRefused::NotSignedIn),
        };
        let list = state
            .with(|db| folders_of(db, &user_id))
            .map_err(refused)?
            .ok_or(PhotoFoldersRefused::NotSaved)?;
        if !self.acct.session_unchanged_since(session) {
            return Err(PhotoFoldersRefused::NotSignedIn);
        }
        Ok(list)
    }

    /// WRITES: the account of `session` adds the folder `folder_id` of its own tree to Photo sources, shown. One that
    /// is there already stays as it is (its switch too). Refused for anything that is not a folder of the account's
    /// own tree (`NotAFolder`), and nothing is written for it.
    pub fn add_photo_folder_for(&self, session: SessionGeneration, folder_id: &str) -> Result<(), PhotoFoldersRefused> {
        self.change_photo_folder(session, Change::Add(folder_id))
    }

    /// WRITES: whether Photos shows the folder `folder_id` the account of `session` added. Off: its photos and its
    /// subfolders' leave the library; on: they show again. `NotAFolder`: the account did not add it.
    pub fn set_photo_folder_shown_for(
        &self,
        session: SessionGeneration,
        folder_id: &str,
        shown: bool,
    ) -> Result<(), PhotoFoldersRefused> {
        self.change_photo_folder(session, Change::Shown(folder_id, shown))
    }

    /// WRITES: the account of `session` takes the folder `folder_id` out of Photo sources: its photos show again.
    /// Nothing is moved or deleted. `NotAFolder`: the account had not added it.
    pub fn remove_photo_folder_for(
        &self,
        session: SessionGeneration,
        folder_id: &str,
    ) -> Result<(), PhotoFoldersRefused> {
        self.change_photo_folder(session, Change::Remove(folder_id))
    }

    /// The account-bound state for `session` and the session's own user id (the key of the account's rows), read in
    /// that order: the user id between two checks that the session is still `session`, so it is never a later
    /// session's. `for_changes`: the read-write handle. `Err(None)`: not the session of the press any more;
    /// `Err(Some(_))`: the handle's refusal.
    fn photo_folder_state(
        &self,
        session: SessionGeneration,
        for_changes: bool,
    ) -> Result<(OwnedState<'_>, String), Option<LocalStateRefused>> {
        if !self.acct.session_unchanged_since(session) {
            return Err(None);
        }
        // The row's key. An email never stands in for it.
        let user_id = self
            .session_identity()
            .map_err(Some)?
            .user_id
            .ok_or(Some(LocalStateRefused::IdentityUnknown))?;
        let state = (if for_changes {
            self.local_state_for_changes()
        } else {
            self.local_state()
        })
        .map_err(Some)?;
        // The handle's session is the one the host named, not a later one that opened in between.
        if state.session() != session {
            return Err(None);
        }
        Ok((state, user_id))
    }

    fn change_photo_folder(&self, session: SessionGeneration, change: Change<'_>) -> Result<(), PhotoFoldersRefused> {
        let (state, user_id) = self
            .photo_folder_state(session, true)
            .map_err(|refusal| match refusal {
                Some(refusal) => refused(refusal),
                None => PhotoFoldersRefused::NotSignedIn,
            })?;
        let done = match change {
            Change::Add(folder_id) => {
                // Only a folder of the account's own tree: never a file, a backup or a shared row through this id.
                match state.with(|db| own_folder(db, folder_id)).map_err(refused)? {
                    Ok(true) => {}
                    Ok(false) => return Err(PhotoFoldersRefused::NotAFolder),
                    Err(_) => return Err(PhotoFoldersRefused::NotSaved),
                }
                // `false`: added already, and that is no refusal (nothing changed).
                state
                    .with(|db| db.add_photo_folder(&user_id, folder_id))
                    .map_err(refused)?
                    .map(|_| true)
            }
            Change::Shown(folder_id, shown) => state
                .with(|db| db.set_photo_folder_shown(&user_id, folder_id, shown))
                .map_err(refused)?,
            Change::Remove(folder_id) => state
                .with(|db| db.remove_photo_folder(&user_id, folder_id))
                .map_err(refused)?,
        };
        let changed = done.map_err(|_| PhotoFoldersRefused::NotSaved)?;
        // Asked again after the write: a session that moved on meanwhile answers `NotSignedIn`, and the host drops it.
        if !self.acct.session_unchanged_since(session) {
            return Err(PhotoFoldersRefused::NotSignedIn);
        }
        if !changed {
            return Err(PhotoFoldersRefused::NotAFolder);
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "photos_folders_tests.rs"]
mod tests;
