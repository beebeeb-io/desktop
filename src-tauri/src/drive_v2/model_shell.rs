//! The two view models that read a shell module (`desktop_search`, `account_dto`). Every other one is the engine
//! crate's (`beebeeb_engine::drive_v2::model`, no Tauri); `drive_v2::model` is both, under the old path.

use beebeeb_engine::drive_v2::model::*;

use super::DriveError;
use super::dto::*;
use crate::state_db::StateDb;

/// `drive_search`: names from the local index (the quick-open index), matched rows only.
pub(crate) fn search(db: &StateDb, query: &str) -> Result<Vec<DriveItem>, DriveError> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let index = crate::desktop_search::build_local_index(db).map_err(|_| DriveError::Failed("search index"))?;
    let response = crate::desktop_search::query_local_index(&index, query, 50, false);
    let mut out = Vec::new();
    for hit in response.results {
        if let Ok(entry) = resolve(db, &hit.file_id) {
            out.push(item_for_entry(db, &entry)?);
        }
    }
    Ok(out)
}

/// `drive_trash`: the server's trash list (names already decrypted in Rust), with where each item was.
pub(crate) fn trash_entries(db: Option<&StateDb>, raw: Vec<crate::account_dto::DesktopTrashItem>) -> Vec<TrashEntry> {
    raw.into_iter()
        .map(|t| {
            let parent = t
                .parent_id
                .as_deref()
                .and_then(|p| db.and_then(|db| db.get_file(p).ok().flatten()));
            let original_path = match &parent {
                Some(p) if !p.path.trim_matches('/').is_empty() => format!("{ROOT_NAME}/{}", p.path.trim_matches('/')),
                _ => ROOT_NAME.to_string(),
            };
            let item = DriveItem {
                has_thumbnail: !t.is_folder && has_thumbnail(&t.name),
                mime: (!t.is_folder)
                    .then(|| mime_for_name(&t.name).map(str::to_string))
                    .flatten(),
                id: t.id,
                parent_id: t.parent_id,
                is_folder: t.is_folder,
                size: (!t.is_folder).then_some(t.size_bytes.max(0)),
                child_count: None,
                modified_at: t.updated_at.clone(),
                shared: SharedState::None,
                local: None,
                read_only: None,
                name: t.name,
            };
            TrashEntry {
                item,
                trashed_at: t.updated_at,
                original_path,
            }
        })
        .collect()
}
