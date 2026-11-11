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
    pub fn open(path: &Path) -> Result<Self, ReadOnlyOpenError> {
        Ok(Self {
            db: StateDb::open_read_only(path)?,
        })
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

fn crumb(crumb: dto::Crumb) -> Crumb {
    Crumb {
        id: crumb.id,
        name: crumb.name,
    }
}

/// The engine's row as the UI's row. One place, so a field the UI adds is mapped once.
fn item(item: dto::DriveItem) -> DriveItem {
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
        pc_status: item.local.as_ref().and_then(|local| match local {
            dto::LocalState::Downloaded => Some("on_this_pc".to_string()),
            dto::LocalState::OnlineOnly => Some("online_only".to_string()),
            dto::LocalState::AvailableOffline => Some("available_offline".to_string()),
            // Syncing and failed have no mark in this column yet.
            dto::LocalState::Syncing | dto::LocalState::Failed => None,
        }),
        // Not read yet: the Backups root's flag (`system_role`) is not in the listing's row.
        is_protected: false,
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
