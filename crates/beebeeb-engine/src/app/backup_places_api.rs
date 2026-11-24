//! What the app WINDOW may change of the rows it shows, for a host without a UI toolkit (the native Files window's
//! Backups tree): [`Engine::backup_places_for`] reads, in ONE pass over the signed-in account's local state, the place
//! of a folder and of its rows in the Backups tree.
//!
//! - The rule is ASKED, never re-derived: a row has a place only where [`backup_actions::verdict`] (the app rule,
//!   core's `check_app_mutation`) refuses a rename of it. [`crate::backup_protect`] is the DRIVE surface's rule and is
//!   not used here (its words send the person "to the app").
//! - The roots are read strictly: a database that cannot be read is an error, never "outside the tree" (that would
//!   allow every change). Ids only: no name and no path leave this module.
//! - `this_pc` comes from the account's own backup layout (the device folder it resolved), never from a name.
//!
//! BLOCKS on the database: a worker's, once per folder shown, never a draw's. No network. Must not: log a name, a
//! path or an id.

use std::collections::HashSet;

use beebeeb_core::backup::BackupPosition;

use super::{CredentialStores, Engine, LocalStateRefused};
use crate::account::SessionGeneration;
use crate::backup_actions::{Action, Subject, Verdict, verdict};
use crate::backup_protect::BackupsTree;
use crate::state_db::{FileEntry, StateDb};

#[cfg(test)]
#[path = "backup_places_api_tests.rs"]
mod tests;

/// How far up a row's parents are followed to find its device's folder (the tree is a handful of levels deep).
const MOST_LEVELS: usize = 64;

/// What the app window may change of a row, by the app rule. Ids only: no name, no path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackupPlace {
    /// Not in the Backups tree: nothing is refused.
    Outside,
    /// The Backups folder itself.
    Root,
    /// A device's folder; `id` is that folder's file id.
    Device { id: String, this_pc: bool },
    /// Anything below a device's folder; `device` as above.
    Inside { device: String, this_pc: bool },
}

/// The place of a folder and of the rows read in it, and the session they were read for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackupPlaces {
    pub session: SessionGeneration,
    /// The folder asked about (`None`: the root of Files).
    pub folder: Option<String>,
    pub of_folder: BackupPlace,
    /// Each row listed in the folder (hidden rows left out), with its place.
    pub rows: Vec<(String, BackupPlace)>,
}

/// Whether the app rule keeps this row from being renamed: the rule is asked, with nothing else known of the row.
fn refused(position: BackupPosition, is_folder: bool) -> bool {
    matches!(
        verdict(
            &Subject {
                position,
                is_folder,
                own_active_source: None
            },
            Action::Rename
        ),
        Verdict::Refused(_)
    )
}

/// The file id of the folder directly under a Backups root that holds `entry`: `entry` itself when it is one.
fn device_of(db: &StateDb, entry: &FileEntry, roots: &HashSet<String>) -> Result<Option<String>, LocalStateRefused> {
    let mut current = entry.clone();
    for _ in 0..MOST_LEVELS {
        let Some(parent) = current.parent_id.clone() else {
            return Ok(None);
        };
        if roots.contains(&parent) {
            return Ok(Some(current.file_id));
        }
        match db.get_file(&parent).map_err(|_| LocalStateRefused::Unreadable)? {
            Some(up) => current = up,
            None => return Ok(None),
        }
    }
    Ok(None)
}

impl<C: CredentialStores> Engine<C> {
    /// See the module's doc. `session` is [`Engine::session_turn`] as the host read it AT THE EVENT; a handle of
    /// another session answers [`LocalStateRefused::SessionChanged`] and nothing is read.
    pub fn backup_places_for(
        &self,
        session: SessionGeneration,
        folder_id: Option<&str>,
    ) -> Result<BackupPlaces, LocalStateRefused> {
        let state = self.local_state()?;
        if state.session() != session {
            return Err(LocalStateRefused::SessionChanged);
        }
        let folder = folder_id.map(str::to_string);
        let (of_folder, rows) = state.with(|db| read_places(db, folder_id))??;
        Ok(BackupPlaces {
            session,
            folder,
            of_folder,
            rows,
        })
    }
}

type Read = (BackupPlace, Vec<(String, BackupPlace)>);

fn read_places(db: &StateDb, folder_id: Option<&str>) -> Result<Read, LocalStateRefused> {
    // Read strictly: `BackupsTree::read` treats a failed read as "no tree", which would allow every change.
    let roots: HashSet<String> = db
        .backups_roots()
        .map_err(|_| LocalStateRefused::Unreadable)?
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    let tree = BackupsTree::read(db);
    let this_pc = db.backup_layout().ok().flatten().map(|layout| layout.device_folder_id);
    let place_of = |entry: &FileEntry, device: Option<String>| -> BackupPlace {
        let position = tree.position(&entry.path);
        if !refused(position, entry.is_dir()) {
            return BackupPlace::Outside;
        }
        let is_this_pc = |id: &str| this_pc.as_deref() == Some(id);
        match (position, device) {
            (BackupPosition::Root, _) => BackupPlace::Root,
            (BackupPosition::DeviceFolder, _) => BackupPlace::Device {
                this_pc: is_this_pc(&entry.file_id),
                id: entry.file_id.clone(),
            },
            (_, Some(device)) => BackupPlace::Inside {
                this_pc: is_this_pc(&device),
                device,
            },
            // Inside the tree, and no device folder could be found above it: its own id stands for the device, so the
            // row is still refused (never "outside").
            (_, None) => BackupPlace::Inside {
                device: entry.file_id.clone(),
                this_pc: false,
            },
        }
    };
    let folder_row = match folder_id {
        Some(id) => db.get_file(id).map_err(|_| LocalStateRefused::Unreadable)?,
        None => None,
    };
    let (of_folder, folder_device) = match &folder_row {
        Some(row) => {
            let device = device_of(db, row, &roots)?;
            (place_of(row, device.clone()), device)
        }
        None => (BackupPlace::Outside, None),
    };
    let (children, _) = db
        .children_paged(folder_id, 0, i64::MAX)
        .map_err(|_| LocalStateRefused::Unreadable)?;
    let mut rows = Vec::with_capacity(children.len());
    for (entry, _, _) in &children {
        if tree.is_hidden(&entry.path) {
            continue;
        }
        // A row's device: the folder's own, or (directly under a root) the row itself.
        let device = match folder_device.clone() {
            Some(device) => Some(device),
            None if entry.parent_id.as_ref().is_some_and(|parent| roots.contains(parent)) => {
                Some(entry.file_id.clone())
            }
            None => None,
        };
        rows.push((entry.file_id.clone(), place_of(entry, device)));
    }
    Ok((of_folder, rows))
}
