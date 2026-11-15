//! Slice A7 (plan40 batch 1): Add a folder and Stop backing up (DW-0737, DW-0738, DW-1410): a folder this PC backs up,
//! added or removed. Folds wave-3 S11 in: which folder a known folder OneDrive moved is backed up from.
//!
//! - [`Engine::add_backup_folder_for`]: DW-0738's picker answered with a folder. The rule ([`check_chosen`], here and
//!   nowhere else): Beebeeb's own folder and any folder inside it can't be chosen, nor a folder that holds it, a folder
//!   already backed up (a chosen one, or a known folder's location), or anything that is not a folder on this PC. A
//!   folder that passes joins the settings file's backup folders (`backup_chosen_folders`, its key `custom:<name>` in
//!   the backup set) under core's custom name (`custom_category_name`: safe, unique, never changed after), its backup
//!   starts over in the state database (no records, no stop, no last pass), and the runner passes it like a known
//!   folder from its next tick (`backup::sources::chosen_sources`).
//! - [`Engine::remove_backup_folder_for`]: DW-1410. The folder leaves the backup set first (settings, then its pending
//!   uploads purged and the protection refreshed: it is no longer this PC's active backup). "Stop and keep the
//!   backup" ends there: its backup stays in Backups, read-only. "Stop and move the backup to Trash" then proves the
//!   ACCOUNT PASSWORD and moves that one folder's backup to Trash through the engine's one delete of a backup
//!   (`Engine::delete_device_backup_for`: core's OPAQUE step-up, one single-use token, one move to Trash); a refusal
//!   before anything was sent puts the folder back as it was ([`remove_steps`]). No file on this PC is touched,
//!   either way: nothing here opens, moves or deletes anything under the folder's path.
//! - [`Engine::choose_backup_source`] (S11): the answer to "which folder do you want to back up?" for a moved known
//!   folder, written to `known_folder_source`; its question is settled and the protection follows.
//!
//! Session-bound like its neighbours (`*_for(session)`): every call is asked for the session the host read at its
//! press, and refused, with nothing written, once that session is no longer the one in memory.
//!
//! Must not: log a path, a folder's name, an id or the password; forge a step-up (the password is proven to the server
//! or nothing moves); delete anything in Beebeeb for good; touch the chosen folder on this PC.

use std::path::{Path, PathBuf};

use beebeeb_core::backup::{category_folder_name, custom_category_name, fold_name};

use crate::account::SessionGeneration;
use crate::app::backups_api::BackupsError;
use crate::app::{CredentialStores, Engine, LocalStateRefused, OwnedState};
use crate::backup::kfm;
use crate::backup::sources::{self, ChosenFolder, SourceRefusal};
use crate::backup_delete::DeleteBackupError;
use crate::config::{DesktopConfig, KnownFolderSource};
use crate::identity::EngineIdentity;
use crate::known_folder::{KNOWN_FOLDERS, known_folder_by_key};
use crate::state_db::StateDb;

#[cfg(test)]
#[path = "backup_folders_tests.rs"]
mod tests;

/// Why a folder was not added. Nothing was written in any of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddRefused {
    /// Beebeeb's own folder in File Explorer (the sync folder), or a folder inside it (DW-0738's rule).
    InsideSyncFolder,
    /// A folder that holds Beebeeb's own folder.
    HoldsSyncFolder,
    /// Backed up already: a folder chosen before, or where one of the known folders is.
    AlreadyBackedUp,
    /// Not a folder on this PC (a file, a folder that is gone, a path that is not absolute).
    NotAFolder,
    /// The local state or the settings file refused (signed out, another session, unreadable, ...).
    Backups(BackupsError),
}

impl AddRefused {
    /// What the window says, one sentence (it adds that nothing was changed).
    pub fn words(self) -> &'static str {
        match self {
            Self::InsideSyncFolder => {
                "Beebeeb's own folder, and any folder inside it, can't be chosen. Its files are in Beebeeb already."
            }
            Self::HoldsSyncFolder => {
                "This folder holds Beebeeb's own folder, so it can't be chosen. Choose a folder inside it."
            }
            Self::AlreadyBackedUp => "This folder is backed up already.",
            Self::NotAFolder => "This isn't a folder on this PC.",
            Self::Backups(_) => "Couldn't add this folder. Try again in a moment.",
        }
    }

    /// The refusal's name, for a log line (no path, no name).
    pub fn kind(self) -> &'static str {
        match self {
            Self::InsideSyncFolder => "inside_sync_folder",
            Self::HoldsSyncFolder => "holds_sync_folder",
            Self::AlreadyBackedUp => "already_backed_up",
            Self::NotAFolder => "not_a_folder",
            Self::Backups(error) => error.kind(),
        }
    }
}

/// Why "Stop backing up" did not do all it was asked. Each says whether anything changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoveRefused {
    /// No folder the person chose has this id (removed meanwhile). Nothing changed.
    NotChosen,
    /// The local state or the settings file refused. Nothing changed (a half-done removal is put back).
    Backups(BackupsError),
    /// The move to Trash was refused before anything was sent (the password, the rule, the session): the folder was
    /// put back in the backup set as it was. Nothing changed.
    PasswordRefused(DeleteBackupError),
    /// The folder is no longer backed up, but the move to Trash may not have happened (no answer, or the server's
    /// error): its backup may still be in Backups.
    TrashNotDone(DeleteBackupError),
}

impl RemoveRefused {
    /// What the window says, whole sentences that say whether anything changed.
    pub fn words(&self) -> &'static str {
        match self {
            Self::NotChosen => "This folder is no longer in the list. Nothing was changed.",
            Self::Backups(_) => "Couldn't stop backing up this folder. Nothing was changed.",
            Self::PasswordRefused(DeleteBackupError::WrongPassword) => {
                "That password isn't right. Nothing was changed."
            }
            Self::PasswordRefused(DeleteBackupError::RateLimited(_)) => {
                "Too many tries. Wait a minute, then try again. Nothing was changed."
            }
            Self::PasswordRefused(DeleteBackupError::SignedOut) => "You're no longer signed in. Nothing was changed.",
            Self::PasswordRefused(DeleteBackupError::Refused(_) | DeleteBackupError::NotABackup) => {
                "This backup can't be moved to Trash from here. Nothing was changed."
            }
            Self::PasswordRefused(_) => "Couldn't move this backup to Trash. Nothing was changed.",
            Self::TrashNotDone(_) => {
                "This folder is no longer backed up. Beebeeb didn't confirm the move to Trash, so its backup may still be in Backups."
            }
        }
    }

    /// The refusal's name, for a log line (no path, no name, never the server's words).
    pub fn kind(&self) -> &'static str {
        match self {
            Self::NotChosen => "not_chosen",
            Self::Backups(error) => error.kind(),
            Self::PasswordRefused(_) => "trash_refused",
            Self::TrashNotDone(_) => "trash_unconfirmed",
        }
    }

    /// The folder left the backup set (only [`RemoveRefused::TrashNotDone`]).
    pub fn stopped_backing_up(&self) -> bool {
        matches!(self, Self::TrashNotDone(_))
    }
}

/// DW-0738's rule: whether `path` may be chosen while Beebeeb's own folder is `sync_root` and `taken` are the folders
/// this PC backs up already. Compared as the backup's own rule compares (`backup::sources`: component by component,
/// without case on Windows and macOS, as given and as resolved). Reads only whether `path` is a folder.
pub fn check_chosen(path: &Path, sync_root: &Path, taken: &[PathBuf]) -> Result<(), AddRefused> {
    if !path.is_absolute() || !path.is_dir() {
        return Err(AddRefused::NotAFolder);
    }
    // A sync folder that cannot be placed is never guessed around: refused.
    if !sync_root.is_absolute() {
        return Err(AddRefused::Backups(BackupsError::Settings));
    }
    let inside = matches!(
        sources::check_source(path, sync_root),
        Err(SourceRefusal::InsideSyncRoot)
    );
    if inside {
        return Err(AddRefused::InsideSyncFolder);
    }
    let holds = matches!(
        sources::check_source(sync_root, path),
        Err(SourceRefusal::InsideSyncRoot)
    );
    if holds {
        return Err(AddRefused::HoldsSyncFolder);
    }
    if taken.iter().any(|folder| sources::same_folder(folder, path)) {
        return Err(AddRefused::AlreadyBackedUp);
    }
    Ok(())
}

/// Where the folders this PC backs up already are: the chosen ones, and each known folder's default and current
/// location (whether its switch is on or not: a known folder is switched, never added).
pub fn taken_paths(config: &DesktopConfig) -> Vec<PathBuf> {
    let mut taken: Vec<PathBuf> = config.backup_chosen_folders.iter().map(|f| f.path.clone()).collect();
    for folder in KNOWN_FOLDERS {
        let both = kfm::known_folder_paths(folder.key);
        taken.extend(both.default);
        taken.extend(both.current);
    }
    taken
}

/// The names of the folders in this device's backup (`Backups/<device>/*`) the local state knows, so a new custom
/// name never lands in the backup of a folder removed earlier and kept. No layout yet: none.
pub fn backup_folder_names(db: &StateDb) -> Result<Vec<String>, BackupsError> {
    let Some(layout) = db.backup_layout().map_err(|_| BackupsError::State)? else {
        return Ok(Vec::new());
    };
    let children = db
        .children_of(Some(&layout.device_folder_id))
        .map_err(|_| BackupsError::State)?;
    Ok(children
        .into_iter()
        .filter(|(entry, _)| entry.is_dir())
        .filter_map(|(entry, _)| entry.path.rsplit('/').next().map(str::to_string))
        .filter(|name| !name.is_empty())
        .collect())
}

/// The settings' part of an add: `path` joins the chosen folders under core's custom name (unique among the chosen
/// names and `in_backup`, the names already in this device's backup) and its key joins the backup set. Its key back.
/// Refused, with nothing changed, when the file already holds the same folder (another add landed first).
pub fn add_to(config: &mut DesktopConfig, path: &Path, in_backup: &[String]) -> Result<String, AddRefused> {
    if config
        .backup_chosen_folders
        .iter()
        .any(|folder| sources::same_folder(&folder.path, path))
    {
        return Err(AddRefused::AlreadyBackedUp);
    }
    let mut taken: Vec<&str> = config
        .backup_chosen_folders
        .iter()
        .map(|folder| folder.name.as_str())
        .collect();
    taken.extend(in_backup.iter().map(String::as_str));
    let raw = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let folder = ChosenFolder {
        name: custom_category_name(&raw, &taken),
        path: path.to_path_buf(),
    };
    let key = folder.key();
    config.backup_chosen_folders.push(folder);
    config.set_known_folder(&key, true);
    Ok(key)
}

/// What a removal took out of the settings, to put it back exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removed {
    pub folder: ChosenFolder,
    /// Its place in the list.
    pub at: usize,
    /// It was in the backup set (on).
    pub was_on: bool,
}

/// The settings' part of a removal: the chosen folder `key` leaves the list and the backup set. `None`: no such folder.
pub fn remove_from(config: &mut DesktopConfig, key: &str) -> Option<Removed> {
    let at = config
        .backup_chosen_folders
        .iter()
        .position(|folder| folder.key() == key)?;
    let folder = config.backup_chosen_folders.remove(at);
    let was_on = config.known_folder_enabled(key);
    config.set_known_folder(key, false);
    Some(Removed { folder, at, was_on })
}

/// Undo [`remove_from`]: the folder back in its place, its switch as it was.
pub fn put_back_into(config: &mut DesktopConfig, removed: &Removed) {
    let key = removed.folder.key();
    if !config.backup_chosen_folders.iter().any(|folder| folder.key() == key) {
        let at = removed.at.min(config.backup_chosen_folders.len());
        config.backup_chosen_folders.insert(at, removed.folder.clone());
    }
    config.set_known_folder(&key, removed.was_on);
}

/// The state database's part of an add: the folder's backup starts over (no records, no last pass, no stop), and the
/// protection takes it in (it is this PC's active backup now). Run after the settings file holds it.
pub fn start_over(identity: &'static EngineIdentity, db: &StateDb, key: &str) -> Result<(), BackupsError> {
    let now = chrono::Utc::now().timestamp_millis();
    db.restart_backup_pass(key, now).map_err(|_| BackupsError::State)?;
    db.clear_backup_stop(key).map_err(|_| BackupsError::State)?;
    crate::backup_protect::refresh_own_backups(identity, db);
    Ok(())
}

/// The state database's part of a removal: the folder's pending uploads go, and the protection lets it go (it is no
/// longer this PC's active backup, so its backup may be moved to Trash). Run after the settings file let it go. What is
/// backed up already stays; nothing on this PC is touched. How many uploads went.
pub fn leave_set(identity: &'static EngineIdentity, db: &StateDb, key: &str) -> Result<usize, BackupsError> {
    let purged = db.purge_backup_source_ops(key).map_err(|_| BackupsError::State)?;
    crate::backup_protect::refresh_own_backups(identity, db);
    tracing::info!(purged, "backups: a chosen folder was removed; its pending uploads went");
    Ok(purged)
}

/// The id of `folder`'s backup in Beebeeb (`Backups/<device>/<name>`), as the local state knows it. `None`: no layout
/// yet, or no such folder (it never passed). Only that one folder: never another folder's backup, never the device's.
pub fn backup_copy_of(db: &StateDb, folder: &ChosenFolder) -> Result<Option<String>, BackupsError> {
    let Some(layout) = db.backup_layout().map_err(|_| BackupsError::State)? else {
        return Ok(None);
    };
    let wanted = fold_name(&category_folder_name(&folder.category()));
    let children = db
        .children_of(Some(&layout.device_folder_id))
        .map_err(|_| BackupsError::State)?;
    Ok(children
        .into_iter()
        .map(|(entry, _)| entry)
        .find(|entry| {
            entry.is_dir()
                && entry
                    .path
                    .rsplit('/')
                    .next()
                    .is_some_and(|leaf| fold_name(leaf) == wanted)
        })
        .map(|entry| entry.file_id))
}

/// Whether a refused move to Trash may have reached Beebeeb (then nothing is put back: the folder stays out of the set).
fn may_have_reached(error: &DeleteBackupError) -> bool {
    matches!(
        error,
        DeleteBackupError::Offline | DeleteBackupError::NotFound | DeleteBackupError::Failed("server")
    )
}

/// DW-1410 as one sequence, its steps handed in (the engine's: the settings file and the state database, the
/// password-proven move to Trash; a test's: fakes):
/// 1. `leave`: the folder leaves the backup set. A refusal there changed nothing (`leave` puts back what it did).
/// 2. Only with `password` and a `copy` (its backup's id): `trash` proves the password and moves that one folder to
///    Trash. Refused before anything was sent: `put_back` puts the folder back as it was, nothing changed. Refused
///    after it may have reached Beebeeb: the folder stays out of the set, and the refusal says so.
///
/// Without a password ("Stop and keep the backup") `trash` is never called.
pub fn remove_steps<Undo>(
    password: Option<&[u8]>,
    copy: Option<&str>,
    leave: impl FnOnce() -> Result<Undo, BackupsError>,
    trash: impl FnOnce(&str, &[u8]) -> Result<(), DeleteBackupError>,
    put_back: impl FnOnce(Undo),
) -> Result<(), RemoveRefused> {
    let undo = leave().map_err(RemoveRefused::Backups)?;
    let (Some(password), Some(copy)) = (password, copy) else {
        return Ok(());
    };
    match trash(copy, password) {
        Ok(()) => Ok(()),
        Err(error) if may_have_reached(&error) => Err(RemoveRefused::TrashNotDone(error)),
        Err(error) => {
            put_back(undo);
            Err(RemoveRefused::PasswordRefused(error))
        }
    }
}

/// The settings file without the folder an add wrote, best effort (an add whose state part failed is not left half
/// done).
fn undo_add(identity: &'static EngineIdentity, key: &str) {
    let undone = DesktopConfig::update_for(identity, |config| (remove_from(config, key).is_some(), ()));
    if undone.is_err() {
        tracing::warn!("backups: a failed add could not be taken out of the settings file");
    }
}

/// Put a removed folder back, best effort: the settings file, then the protection.
fn restore(identity: &'static EngineIdentity, handle: Option<&OwnedState<'_>>, removed: &Removed) {
    let written = DesktopConfig::update_for(identity, |config| {
        put_back_into(config, removed);
        (true, ())
    });
    if written.is_err() {
        tracing::warn!("backups: a chosen folder could not be put back in the settings file");
    }
    if let Some(handle) = handle {
        let _ = handle.with(|db| crate::backup_protect::refresh_own_backups(identity, db));
    }
}

/// S11: the state database's part of a moved known folder's answer: its question is settled, the protection follows.
pub fn source_chosen(identity: &'static EngineIdentity, db: &StateDb, key: &str) -> Result<(), BackupsError> {
    db.settle_backup_source_question(key).map_err(|_| BackupsError::State)?;
    crate::backup_protect::refresh_own_backups(identity, db);
    Ok(())
}

impl<C: CredentialStores> Engine<C> {
    /// The local state for a change asked under `session`: `None` while nothing has synced on this computer (the
    /// settings file alone is written then, as `set_backup_folder` does). Refused once `session` is not the one in
    /// memory, before anything is read.
    fn backup_folders_state(&self, session: SessionGeneration) -> Result<Option<OwnedState<'_>>, BackupsError> {
        if self.session_turn() != session {
            return Err(BackupsError::Refused(LocalStateRefused::SessionChanged));
        }
        let handle = match self.local_state_for_changes() {
            Ok(handle) => handle,
            Err(LocalStateRefused::NothingSynced) => return Ok(None),
            Err(refused) => return Err(BackupsError::Refused(refused)),
        };
        if handle.session() != session {
            return Err(BackupsError::Refused(LocalStateRefused::SessionChanged));
        }
        handle.still_this_session().map_err(BackupsError::Refused)?;
        Ok(Some(handle))
    }

    /// WRITES: add `path` (DW-0738's picker's answer) to the folders this PC backs up, for the session a person
    /// pressed under. The rule is [`check_chosen`]; see the module's doc. Its key back (`custom:<name>`, the id the
    /// window lists it under). Never runs a pass here: the runner's next tick passes it. BLOCKS: a worker's.
    pub fn add_backup_folder_for(&self, session: SessionGeneration, path: &Path) -> Result<String, AddRefused> {
        let handle = self.backup_folders_state(session).map_err(AddRefused::Backups)?;
        let identity = self.identity;
        let sync_root = self
            .sync_root()
            .map_err(|_| AddRefused::Backups(BackupsError::Settings))?;
        let config = DesktopConfig::load_for(identity).map_err(|_| AddRefused::Backups(BackupsError::Settings))?;
        check_chosen(path, &sync_root, &taken_paths(&config))?;
        let in_backup = match &handle {
            Some(handle) => handle
                .with(backup_folder_names)
                .map_err(|refused| AddRefused::Backups(BackupsError::Refused(refused)))?
                .map_err(AddRefused::Backups)?,
            None => Vec::new(),
        };
        // Written under the settings file's lock: the duplicate is checked again against what the file holds now.
        let key = DesktopConfig::update_for(identity, |config| match add_to(config, path, &in_backup) {
            Ok(key) => (true, Ok(key)),
            Err(refused) => (false, Err(refused)),
        })
        .map_err(|_| AddRefused::Backups(BackupsError::Settings))??;
        if let Some(handle) = &handle {
            let started = handle
                .with(|db| start_over(identity, db, &key))
                .map_err(BackupsError::Refused)
                .and_then(|started| started);
            if let Err(error) = started {
                undo_add(identity, &key);
                return Err(AddRefused::Backups(error));
            }
        }
        tracing::info!("backups: a chosen folder was added");
        Ok(key)
    }

    /// WRITES: stop backing up the chosen folder `id` (DW-1410), for the session a person pressed under. Without
    /// `trash_password` its backup stays in Backups ("Stop and keep the backup"); with it, the account password is
    /// proven and that one folder's backup moves to Trash ("Stop and move the backup to Trash"). See
    /// [`remove_steps`] for what a refusal leaves. Never touches the folder on this PC. BLOCKS: a worker's.
    pub fn remove_backup_folder_for(
        &self,
        session: SessionGeneration,
        id: &str,
        trash_password: Option<&[u8]>,
    ) -> Result<(), RemoveRefused> {
        let handle = self.backup_folders_state(session).map_err(RemoveRefused::Backups)?;
        let identity = self.identity;
        let config = DesktopConfig::load_for(identity).map_err(|_| RemoveRefused::Backups(BackupsError::Settings))?;
        let Some(folder) = config
            .backup_chosen_folders
            .iter()
            .find(|folder| folder.key() == id)
            .cloned()
        else {
            return Err(RemoveRefused::NotChosen);
        };
        let key = folder.key();
        // Its backup, found while the folder is still in the set (only when it goes to Trash).
        let copy = match (&handle, trash_password) {
            (Some(handle), Some(_)) => handle
                .with(|db| backup_copy_of(db, &folder))
                .map_err(|refused| RemoveRefused::Backups(BackupsError::Refused(refused)))?
                .map_err(RemoveRefused::Backups)?,
            _ => None,
        };
        let leave = || -> Result<Removed, BackupsError> {
            let removed = DesktopConfig::update_for(identity, |config| match remove_from(config, &key) {
                Some(removed) => (true, Some(removed)),
                None => (false, None),
            })
            .map_err(|_| BackupsError::Settings)?
            .ok_or(BackupsError::UnknownFolder)?;
            if let Some(handle) = &handle {
                let left = handle
                    .with(|db| leave_set(identity, db, &key))
                    .map_err(BackupsError::Refused)
                    .and_then(|left| left);
                if let Err(error) = left {
                    restore(identity, Some(handle), &removed);
                    return Err(error);
                }
            }
            Ok(removed)
        };
        let removed = remove_steps(
            trash_password,
            copy.as_deref(),
            leave,
            |copy, password| self.delete_device_backup_for(session, copy, password),
            |removed: Removed| restore(identity, handle.as_ref(), &removed),
        );
        match &removed {
            Ok(()) => tracing::info!("backups: a chosen folder stopped backing up"),
            Err(refused) => tracing::info!(
                kind = refused.kind(),
                "backups: a chosen folder was not stopped as asked"
            ),
        }
        removed
    }

    /// S11 (wave 3, folded into A7): which folder the known folder `key`, moved by OneDrive's Known Folder Move, is
    /// backed up from (LD-0113 Amendment 9, "Ask once"). The answer is written to the settings file
    /// (`known_folder_source`), its question is settled and the protection follows. Refused, with nothing written, for
    /// a key that is not one of the six, and unless the local state is the signed-in account's or nothing has synced
    /// on this computer yet. BLOCKS: a worker's.
    pub fn choose_backup_source(&self, key: &str, choice: KnownFolderSource) -> Result<(), BackupsError> {
        if known_folder_by_key(key).is_none() {
            return Err(BackupsError::UnknownFolder);
        }
        let handle = match self.local_state_for_changes() {
            Ok(handle) => Some(handle),
            Err(LocalStateRefused::NothingSynced) => None,
            Err(refused) => return Err(BackupsError::Refused(refused)),
        };
        if let Some(handle) = &handle {
            handle.still_this_session().map_err(BackupsError::Refused)?;
        }
        let identity = self.identity;
        DesktopConfig::update_for(identity, |config| {
            let was = config.known_folder_source.insert(key.to_string(), choice);
            (was != Some(choice), ())
        })
        .map_err(|_| BackupsError::Settings)?;
        if let Some(handle) = &handle {
            handle
                .with(|db| source_chosen(identity, db, key))
                .map_err(BackupsError::Refused)??;
        }
        Ok(())
    }
}
