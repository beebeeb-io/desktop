//! Backups as a shell reaches them (LD-0113): what the Backups section reads about THIS computer's backup, and the
//! switch of one known folder. Additive: it composes the engine's own parts (`backup::kfm`, `backup::stop`,
//! `backup_protect`, the settings file and the state database's backup rows); nothing here changes how a backup
//! pass runs.
//!
//! The local state is reached only through the account-bound handle (task 2488): a state that is not the signed-in
//! account's is never read or changed, and the settings file is not written for it either. A computer where nothing
//! has synced yet has no state database: the folders are still read from the settings file (they are this
//! computer's), there is no Backups root yet, and a switch writes the settings file alone, as the 0.8 app does.
//!
//! The folders a person chose beside the six (DW-0737 "chosen by you") are rows too, after the six: added and removed
//! in `app/bindings/backup_folders.rs`, switched like a known folder ("Back up again" after a stop).
//!
//! Not here (no engine function for them yet): other devices' backups, the items that could not back up, the
//! running pass's progress, and a "back up now" trigger (the scheduler is the runner's own).
//!
//! Must not: log a folder's path or a name; walk a folder on the caller's thread for the switch; hold the state
//! database while a folder is counted.

use std::collections::BTreeMap;
use std::path::Path;

use super::{CredentialStores, Engine, LocalStateRefused};
use crate::app::bindings::backup_folders::switch_on_refusal;
use crate::backup::kfm::{self, KnownFolderPaths, SourceResolution};
use crate::backup::sources::ChosenFolder;
use crate::config::{DesktopConfig, KnownFolderSource};
use crate::known_folder::{KNOWN_FOLDERS, known_folder_by_key};
use crate::state_db::StateDb;

/// The account's Backups root and this computer's folder inside it, as the backup layout resolved them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupsRootRow {
    pub root_id: String,
    /// This computer's folder name (core's `find_device_folder`, e.g. `Laptop (a3f2)` after a collision).
    pub device_folder: String,
}

/// Where one known folder's backup stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderState {
    /// Switched off by the person (or never on).
    Off,
    /// On, and read from where it is.
    On,
    /// On, but OneDrive moved it and the person has not said which folder to back up: not backed up until then.
    Waiting,
    /// Turned off by the backup itself (its folder was removed or deleted elsewhere), with "Back up again".
    Stopped,
    /// This computer has no such folder.
    Unavailable,
}

/// One of Windows' six known folders, or a folder the person chose (its key `custom:<name>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupFolderRow {
    /// The settings file's key (`documents`, `custom:Projects`): a catalog id or core's custom key, never a path.
    pub key: String,
    pub name: String,
    /// In the backup set (`known_folder_backup`).
    pub on: bool,
    pub state: FolderState,
    /// A bounded count of its files, when it is read from its default location (never inside OneDrive's folder:
    /// listing an online-only one can fetch it). `None`: not counted.
    pub items: Option<u64>,
    /// When its last pass ran (server clock, milliseconds); `None` before its first.
    pub last_pass_at_ms: Option<i64>,
    /// Why the backup turned itself off, in core's words (`backup_stop_words`); `Some` only while it is stopped.
    pub stop_words: Option<String>,
}

impl BackupFolderRow {
    /// A folder the person chose (DW-0737 "chosen by you"): removed, never switched off.
    pub fn chosen_by_you(&self) -> bool {
        matches!(
            beebeeb_types::backup::Category::from_key(&self.key),
            Some(beebeeb_types::backup::Category::Custom(_))
        )
    }
}

/// What the Backups section reads about this computer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BackupsOverview {
    /// `None`: no layout yet (no pass has resolved the root, or nothing has synced on this computer).
    pub root: Option<BackupsRootRow>,
    pub folders: Vec<BackupFolderRow>,
}

/// Why a Backups read or a switch did not happen. Nothing was changed in any of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupsError {
    /// The local state is not handed out (signed out, locked, another account's, the session changed, ...).
    Refused(LocalStateRefused),
    /// Not one of the six known folders, nor a folder the person chose.
    UnknownFolder,
    /// The settings file could not be read or written.
    Settings,
    /// The state database answered with an error.
    State,
    /// Switching it on would back a folder up twice (P3-3): it is, holds or lies inside another folder of the
    /// account's that is backed up now. `Engine::switch_on_refused` names that folder.
    Nested,
}

impl BackupsError {
    /// The error's name, for a log line (no name, no path).
    pub fn kind(self) -> &'static str {
        match self {
            Self::Refused(refused) => refused.kind(),
            Self::UnknownFolder => "unknown_folder",
            Self::Settings => "settings",
            Self::State => "state",
            Self::Nested => "nested",
        }
    }
}

/// What the local state says about the backup, read under the handle (no disk walk).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StateRead {
    pub root: Option<BackupsRootRow>,
    /// The known folders the backup turned off itself.
    pub stopped: Vec<String>,
    /// Each known folder's last pass (server clock, milliseconds).
    pub last_pass: BTreeMap<String, i64>,
    /// The stop rows themselves (reason and time), for the words of a stopped folder.
    pub stops: Vec<crate::state_db::BackupStop>,
}

/// The backup rows of the signed-in account's state database.
pub fn read_state(db: &StateDb) -> Result<StateRead, BackupsError> {
    let root = db
        .backup_layout()
        .map_err(|_| BackupsError::State)?
        .map(|layout| BackupsRootRow {
            root_id: layout.root_id,
            device_folder: layout.device_folder_name,
        });
    let stops = db.backup_stops().map_err(|_| BackupsError::State)?;
    let stopped = stops.iter().map(|stop| stop.category.clone()).collect();
    let mut last_pass = BTreeMap::new();
    for folder in KNOWN_FOLDERS {
        let mark = db.backup_pass_mark(folder.key).map_err(|_| BackupsError::State)?;
        if let Some(at) = mark.last_pass_at_ms {
            last_pass.insert(folder.key.to_string(), at);
        }
    }
    Ok(StateRead {
        root,
        stopped,
        last_pass,
        stops,
    })
}

/// The six rows, from the settings' backup set and answers, what the state says, each folder's two locations
/// (`paths`) and a bounded count (`count`, called only for a folder read from its default location).
pub fn overview_from(
    enabled: &[String],
    choices: &BTreeMap<String, KnownFolderSource>,
    state: &StateRead,
    paths: impl Fn(&str) -> KnownFolderPaths,
    count: impl Fn(&Path) -> Option<u64>,
) -> BackupsOverview {
    let folders = KNOWN_FOLDERS
        .iter()
        .map(|folder| {
            let on = enabled.iter().any(|key| key == folder.key);
            let both = paths(folder.key);
            let resolution = kfm::resolve(&both, choices.get(folder.key).copied());
            let stopped = state.stopped.iter().any(|key| key == folder.key);
            let (folder_state, read_from) = match resolution {
                SourceResolution::Unavailable => (FolderState::Unavailable, None),
                SourceResolution::Waiting { .. } if on => (FolderState::Waiting, None),
                SourceResolution::Waiting { .. } => (
                    if stopped {
                        FolderState::Stopped
                    } else {
                        FolderState::Off
                    },
                    None,
                ),
                SourceResolution::Read(path) => {
                    let folder_state = match (on, stopped) {
                        (true, _) => FolderState::On,
                        (false, true) => FolderState::Stopped,
                        (false, false) => FolderState::Off,
                    };
                    (folder_state, Some(path))
                }
            };
            let reads_default = read_from.is_some() && read_from == both.default;
            BackupFolderRow {
                key: folder.key.to_string(),
                name: folder.display_name.to_string(),
                on,
                state: folder_state,
                items: read_from.as_deref().filter(|_| reads_default).and_then(&count),
                last_pass_at_ms: state.last_pass.get(folder.key).copied(),
                stop_words: (folder_state == FolderState::Stopped)
                    .then(|| crate::backup::stop::stop_view(&state.stops, folder.key, folder.display_name, false))
                    .and_then(|view| view.stop_words),
            }
        })
        .collect();
    BackupsOverview {
        root: state.root.clone(),
        folders,
    }
}

/// [`read_state`], and the last pass of each folder the person chose.
pub fn read_state_with(db: &StateDb, chosen: &[ChosenFolder]) -> Result<StateRead, BackupsError> {
    let mut state = read_state(db)?;
    for folder in chosen {
        let key = folder.key();
        let mark = db.backup_pass_mark(&key).map_err(|_| BackupsError::State)?;
        if let Some(at) = mark.last_pass_at_ms {
            state.last_pass.insert(key, at);
        }
    }
    Ok(state)
}

/// The rows of the folders the person chose, after the six, in the order they were added. On while its key is in the
/// backup set and it is still a folder on this PC; stopped (core's words) when its backup turned itself off; a folder
/// gone from this PC is `Unavailable` and not on (it backs up nothing), and stays listed so it can be removed. Counted
/// (`count`) only while on: the pass walks it then anyway.
pub fn chosen_rows(
    chosen: &[ChosenFolder],
    enabled: &[String],
    state: &StateRead,
    count: impl Fn(&Path) -> Option<u64>,
) -> Vec<BackupFolderRow> {
    chosen
        .iter()
        .map(|folder| {
            let key = folder.key();
            let switched_on = enabled.iter().any(|k| *k == key);
            let present = folder.path.is_absolute() && folder.path.is_dir();
            let stopped = state.stopped.iter().any(|k| *k == key);
            let folder_state = match (present, switched_on, stopped) {
                (false, _, _) => FolderState::Unavailable,
                (true, true, _) => FolderState::On,
                (true, false, true) => FolderState::Stopped,
                (true, false, false) => FolderState::Off,
            };
            let on = folder_state == FolderState::On;
            BackupFolderRow {
                items: on.then(|| count(&folder.path)).flatten(),
                last_pass_at_ms: state.last_pass.get(&key).copied(),
                stop_words: (folder_state == FolderState::Stopped)
                    .then(|| crate::backup::stop::stop_view(&state.stops, &key, &folder.name, false))
                    .and_then(|view| view.stop_words),
                key,
                name: folder.name.clone(),
                on,
                state: folder_state,
            }
        })
        .collect()
}

/// The switch's work in the state database: protection follows the switch at once; off purges the folder's
/// pending uploads (what is already backed up stays: nothing in Beebeeb is deleted); on clears a stop and starts
/// over ("Back up again"). The 0.8 app's `set_known_folder_backup`, through the handle.
pub fn apply_switch(
    identity: &'static crate::identity::EngineIdentity,
    db: &StateDb,
    key: &str,
    on: bool,
) -> Result<(), BackupsError> {
    crate::backup_protect::refresh_own_backups(identity, db);
    if on {
        crate::backup::stop::restart(db, key).map_err(|_| BackupsError::State)
    } else {
        let purged = db.purge_backup_source_ops(key).map_err(|_| BackupsError::State)?;
        tracing::info!(folder = %key, purged, "backups: a known folder was switched off; its pending uploads went");
        Ok(())
    }
}

/// A bounded count of a known folder's files. Windows only: no other system has known folders.
fn count(path: &Path) -> Option<u64> {
    #[cfg(target_os = "windows")]
    {
        crate::backup::sources::count_items_bounded(path)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
        None
    }
}

impl<C: CredentialStores> Engine<C> {
    /// This computer's backup, for the signed-in account. The state rows are read through the account-bound handle
    /// first; the folders are counted after it (a disk walk: BLOCKS, a worker's). Nothing has synced yet: the
    /// folders without a root. Any other refusal: nothing.
    pub fn backups_overview(&self) -> Result<BackupsOverview, BackupsError> {
        // The refusal first: nothing is read, not even the settings file, for a state that is not handed out.
        let handle = match self.local_state() {
            Ok(handle) => Some(handle),
            Err(LocalStateRefused::NothingSynced) => None,
            Err(refused) => return Err(BackupsError::Refused(refused)),
        };
        let config = DesktopConfig::load_for(self.identity).map_err(|_| BackupsError::Settings)?;
        // This account's chosen folders alone (P3-8).
        let chosen = self.chosen_folders_seen(handle.as_ref(), &config.backup_chosen_folders);
        // The handle goes before anything is counted.
        let state = match handle {
            Some(handle) => handle
                .with(|db| read_state_with(db, &chosen))
                .map_err(BackupsError::Refused)??,
            None => StateRead::default(),
        };
        let mut overview = overview_from(
            &config.known_folder_backup,
            &config.known_folder_source,
            &state,
            kfm::known_folder_paths,
            count,
        );
        overview
            .folders
            .extend(chosen_rows(&chosen, &config.known_folder_backup, &state, count));
        Ok(overview)
    }

    /// Start or stop backing up one known folder, or a folder the person chose ("Back up again" after its stop).
    /// Never runs a pass here: the runner's scheduler sees the change on its next tick. Switching off deletes nothing
    /// in Beebeeb. Refused, with nothing written, unless the local state is the signed-in account's or nothing has
    /// synced on this computer yet. BLOCKS: a worker's.
    pub fn set_backup_folder(&self, key: &str, on: bool) -> Result<(), BackupsError> {
        // A chosen folder of this account's alone (P3-8).
        let chosen = || self.chosen_key_seen(key);
        if known_folder_by_key(key).is_none() && !chosen() {
            return Err(BackupsError::UnknownFolder);
        }
        let handle = match self.local_state_for_changes() {
            Ok(handle) => Some(handle),
            Err(LocalStateRefused::NothingSynced) => None,
            Err(refused) => return Err(BackupsError::Refused(refused)),
        };
        if let Some(handle) = &handle {
            // Still this session before the settings file is written.
            handle.still_this_session().map_err(BackupsError::Refused)?;
        }
        let identity = self.identity;
        let asker = self.switch_asker(handle.as_ref());
        switch_in_order(
            on,
            || {
                DesktopConfig::update_for(identity, |config| {
                    // Never backed up twice (P3-3), read from the file this update holds: the rule is the add's.
                    if on && switch_on_refusal(config, key, asker.as_ref()).is_some() {
                        return (false, Err(BackupsError::Nested));
                    }
                    let was = config.known_folder_enabled(key);
                    config.set_known_folder(key, on);
                    (was != on, Ok(was))
                })
                .map_err(|_| BackupsError::Settings)?
            },
            || match handle {
                Some(handle) => handle
                    .with(|db| apply_switch(identity, db, key, on))
                    .map_err(BackupsError::Refused)?,
                None => Ok(()),
            },
            |was| {
                // Best effort: the switch must read as it did before, not half done.
                if DesktopConfig::update_for(identity, |config| {
                    config.set_known_folder(key, was);
                    (true, ())
                })
                .is_err()
                {
                    tracing::warn!("backups: a failed switch could not be put back in the settings file");
                }
            },
        )
    }
}

/// Task drive-complete-118 (backlog 2398): the order of the two parts. Off: the settings file first, then the purge
/// (`switched_or_undone`). On: the database part first (it clears the stop), then the settings flip. With the flip
/// first, `reconcile_stopped_switches` could run between the two, see a stop row with its switch on, and switch it
/// off again. A failed flip after the clear leaves the backup off with no stop: the person's click can be repeated.
fn switch_in_order<W: Copy>(
    on: bool,
    write: impl FnOnce() -> Result<W, BackupsError>,
    database: impl FnOnce() -> Result<(), BackupsError>,
    undo: impl FnOnce(W),
) -> Result<(), BackupsError> {
    if on {
        database()?;
        write().map(|_| ())
    } else {
        switched_or_undone(write, database, undo)
    }
}

/// The switch as one step: `write` changes the settings file first (and says what it was), then `database` does the
/// state database's part (off purges the folder's pending uploads, on clears a stop). When the database part does not
/// happen (a failed purge, or the session changed in between) `undo` puts the settings file back, so a folder is
/// never left "off" with its uploads still queued (review of desktop#384). Nothing here names a folder.
fn switched_or_undone<W: Copy>(
    write: impl FnOnce() -> Result<W, BackupsError>,
    database: impl FnOnce() -> Result<(), BackupsError>,
    undo: impl FnOnce(W),
) -> Result<(), BackupsError> {
    let was = write()?;
    match database() {
        Ok(()) => Ok(()),
        Err(error) => {
            undo(was);
            Err(error)
        }
    }
}

#[cfg(test)]
#[path = "backups_api_tests.rs"]
mod tests;

#[cfg(test)]
mod order_tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn switching_on_clears_the_stop_before_the_switch_flips() {
        let log = RefCell::new(Vec::new());
        let done = switch_in_order(
            true,
            || {
                log.borrow_mut().push("flip");
                Ok(false)
            },
            || {
                log.borrow_mut().push("clear");
                Ok(())
            },
            |_| log.borrow_mut().push("undo"),
        );
        assert_eq!(done, Ok(()));
        assert_eq!(*log.borrow(), ["clear", "flip"], "a flip first races reconcile_stopped_switches");
    }

    #[test]
    fn switching_off_still_flips_first_and_undoes_a_failed_purge() {
        let log = RefCell::new(Vec::new());
        let done = switch_in_order(
            false,
            || {
                log.borrow_mut().push("flip");
                Ok(true)
            },
            || {
                log.borrow_mut().push("purge");
                Err(BackupsError::State)
            },
            |_| log.borrow_mut().push("undo"),
        );
        assert_eq!(done, Err(BackupsError::State));
        assert_eq!(*log.borrow(), ["flip", "purge", "undo"]);
    }

    #[test]
    fn a_failed_clear_flips_nothing() {
        let done = switch_in_order(
            true,
            || -> Result<bool, BackupsError> { panic!("no flip after a failed clear") },
            || Err(BackupsError::State),
            |_| panic!("nothing to undo"),
        );
        assert_eq!(done, Err(BackupsError::State));
    }
}
