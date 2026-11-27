//! `EngineSource`: the real `DriveSource`, over the Tauri-free engine crate (`beebeeb_engine`).
//!
//! It opens one `state.db` read-only and answers from it alone: no network, no key, no write. Names in a
//! state.db are already the decrypted ones the engine stored; nothing here logs them.
//! The listing, sorting, crumbs and details are the engine's own view models (`drive_v2::model`), the same
//! code the Tauri shell's `drive_*` commands call.

use std::path::Path;

use beebeeb_engine::drive_v2::dto;
use beebeeb_engine::drive_v2::model::{self, Sort};
use beebeeb_engine::state_db::StateDb;

pub use beebeeb_engine::state_db::ReadOnlyOpenError;

pub use beebeeb_engine::drive_v2::DriveError;
pub use beebeeb_engine::drive_v2::model::{SortDir, SortKey};

use crate::source::{Crumb, DriveItem, DriveSource, Listing, SourceError, StorageSummary};

/// One page of a folder: the rows `offset..offset + limit` of the sorted listing, how many rows the folder
/// has in all, and the crumbs from the root to the folder (both included).
#[derive(Debug, Clone)]
pub struct Page {
    pub items: Vec<DriveItem>,
    pub total: usize,
    pub path: Vec<Crumb>,
}

/// What the detail pane shows for one item.
#[derive(Debug, Clone)]
pub struct Details {
    pub item: DriveItem,
    /// The crumbs to the item's folder.
    pub path: Vec<Crumb>,
    /// 0 for a folder.
    pub version_count: i64,
}

pub struct EngineSource {
    db: StateDb,
}

impl EngineSource {
    /// Open `path` read-only. A file that is missing, not a database, not a state database, from an older build or
    /// held by another program is refused with that reason (`ReadOnlyOpenError`), before any window opens on it.
    ///
    /// The one read of a state database in this app that asks nobody whose it is (task 2488): the engine has this
    /// door in a development build only (`local_state::open_for_dev_viewer`). In a shipped build it is refused
    /// here, and `--db` is not read at all (`shell::instance::DEV_BUILD`).
    pub fn open(path: &Path) -> Result<Self, ReadOnlyOpenError> {
        #[cfg(any(debug_assertions, feature = "dev-tools"))]
        {
            Ok(Self {
                db: beebeeb_engine::local_state::open_for_dev_viewer(path)?,
            })
        }
        #[cfg(not(any(debug_assertions, feature = "dev-tools")))]
        {
            let _ = path;
            Err(ReadOnlyOpenError::NotFound)
        }
    }

    /// A page of `folder_id` (`None`: the root), sorted folders-first by `sort`.
    pub fn page(&self, folder_id: Option<&str>, sort: Sort, offset: usize, limit: usize) -> Result<Page, DriveError> {
        let (rows, total) = model::children_of(&self.db, folder_id, sort, offset, limit)?;
        let folder = model::resolve_folder(&self.db, folder_id)?;
        let path = model::crumbs(&self.db, folder.as_ref())?;
        Ok(Page {
            items: rows.into_iter().map(item).collect(),
            total,
            path: path.into_iter().map(crumb).collect(),
        })
    }

    pub fn details(&self, id: &str) -> Result<Details, DriveError> {
        let details = model::details(&self.db, id)?;
        Ok(Details {
            item: item(details.item),
            path: details.path.into_iter().map(crumb).collect(),
            version_count: details.version_count,
        })
    }
}

pub(crate) fn crumb(crumb: dto::Crumb) -> Crumb {
    Crumb {
        id: crumb.id,
        name: crumb.name,
    }
}

/// `pc_status` of a row whose place on this PC is another item's with the very same name.
pub const NAME_SHARED: &str = "name_shared";
/// `pc_status` of a row whose place on this PC is another item's whose name differs only by capital letters.
pub const NAME_SHARED_BUT_CASE: &str = "name_shared_but_case";

/// Dm-75: on a Mac a file's mark is Finder's real state (dataless or not, by the system's list of materialised
/// items: `engine_host::macos_downloaded::finder_state`), not the engine's own record of a copy. Elsewhere, for a
/// folder and until the system has answered, the engine's mark stands.
fn finder_truth(id: &str, is_folder: bool, mark: Option<String>) -> Option<String> {
    #[cfg(target_os = "macos")]
    if !is_folder {
        use crate::engine_host::macos_downloaded::finder_state;
        return finder_state::mark(mark, finder_state::here(id));
    }
    let _ = (id, is_folder);
    mark
}

/// The engine's row as the UI's row. One place, so a field the UI adds is mapped once.
pub(crate) fn item(item: dto::DriveItem) -> DriveItem {
    let pc_status = finder_truth(&item.id, item.is_folder, item.local.as_ref().and_then(|local| match local {
        dto::LocalState::Downloaded => Some("on_this_pc".to_string()),
        dto::LocalState::OnlineOnly => Some("online_only".to_string()),
        dto::LocalState::AvailableOffline => Some("available_offline".to_string()),
        // Its place on this PC is another item's (`case_place`): marked, its presses on disk refused.
        dto::LocalState::NameShared => Some(NAME_SHARED.to_string()),
        dto::LocalState::NameSharedButCase => Some(NAME_SHARED_BUT_CASE.to_string()),
        // Syncing and failed have no mark in this column yet.
        dto::LocalState::Syncing | dto::LocalState::Failed => None,
    }));
    DriveItem {
        id: item.id,
        parent_id: item.parent_id,
        name: item.name,
        is_folder: item.is_folder,
        size: item.size,
        child_count: item.child_count,
        modified_at: item.modified_at,
        mime: item.mime,
        has_thumbnail: item.has_thumbnail,
        // The state database holds no share links or people for a row of the person's own: none shown.
        shared_type: None,
        shared_badges: Vec::new(),
        pc_status,
        // The Backups root (LD-0113): the engine's listing marks it, hides the manifest and puts the root first.
        is_protected: item.backup.is_protected,
    }
}

impl DriveSource for EngineSource {
    /// The whole folder, name ascending. A folder that is gone and a database that cannot be read are errors:
    /// neither is ever shown as an empty folder.
    fn list(&self, folder_id: Option<&str>) -> Result<Listing, SourceError> {
        let page = self
            .page(folder_id, Sort::default(), 0, usize::MAX)
            .map_err(|error| match error {
                DriveError::NotFound => SourceError::NotFound,
                _ => SourceError::Unreadable,
            })?;
        let folder = page.path.last().cloned().ok_or(SourceError::Unreadable)?;
        Ok(Listing {
            folder,
            path: page.path,
            items: page.items,
        })
    }

    /// Used and total bytes are the account's, from the server. A state.db does not hold them.
    fn storage(&self) -> Option<StorageSummary> {
        None
    }
}

#[cfg(test)]
#[path = "source_engine_tests.rs"]
mod tests;
