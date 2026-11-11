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

pub use beebeeb_engine::drive_v2::DriveError;
pub use beebeeb_engine::drive_v2::model::{SortDir, SortKey};

use crate::source::{Crumb, DriveItem, DriveSource, Listing, StorageSummary};

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
    /// Open `path` read-only. The error is a fixed word (`"not found"`, `"not a state database"`), never
    /// SQLite's text.
    pub fn open(path: &Path) -> Result<Self, &'static str> {
        if !path.is_file() {
            return Err("not found");
        }
        let db = StateDb::open_read_only(path).map_err(|_| "not a state database")?;
        // Prove the file is a state database before the window opens on it.
        db.children_paged(None, 0, 1).map_err(|_| "not a state database")?;
        Ok(Self { db })
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
    }
}

impl DriveSource for EngineSource {
    /// The whole folder, name ascending. A folder that is gone (or an id that is not a folder) shows as the
    /// empty root rather than panicking the window; `page` is the call that reports the error.
    fn list(&self, folder_id: Option<&str>) -> Listing {
        let root = || Crumb {
            id: None,
            name: model::ROOT_NAME.to_string(),
        };
        match self.page(folder_id, Sort::default(), 0, usize::MAX) {
            Ok(page) => Listing {
                folder: page.path.last().cloned().unwrap_or_else(root),
                path: page.path,
                items: page.items,
            },
            Err(_) => Listing {
                folder: root(),
                path: vec![root()],
                items: Vec::new(),
            },
        }
    }

    /// Used and total bytes are the account's, from the server. A state.db does not hold them.
    fn storage(&self) -> Option<StorageSummary> {
        None
    }
}

#[cfg(test)]
#[path = "source_engine_tests.rs"]
mod tests;
