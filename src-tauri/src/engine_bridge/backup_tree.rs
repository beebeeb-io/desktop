//! LD-0113 Task D4: the backup pass's server side of the tree (`BackupHost::{device_tree, ensure_folder,
//! move_file}`): listing, folder creates and moves under `Backups/<device>/<category>`. Every request goes through
//! the backup client (`X-Beebeeb-Origin: backup`, spec Amendment 8), never through the app's mutation commands, so
//! the protection layer (`check_app_mutation`, `check_drive_mutation`) is never on the way (it guards the drive
//! surface; the engine's own placeholder changes pass it as `by_this_process`). Names are sealed and opened on this
//! device only; the logs carry ids and counts.

use super::*;

/// The most entries one backup listing reads before it stops and reports itself partial (core then marks nothing
/// removed elsewhere from it).
pub(crate) const BACKUP_LISTING_CAP: usize = 200_000;

/// One live child of a folder, its name opened on this device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BackupChild {
    pub(crate) id: String,
    /// `None` when the name cannot be opened (yet): live by its id, never matched by name.
    pub(crate) name: Option<String>,
    pub(crate) is_folder: bool,
    pub(crate) size: u64,
    /// The server's `uploaded_at` (else `updated_at`), in Unix ms.
    pub(crate) uploaded_at_ms: i64,
}

/// A folder the backup knows by id, as the server has it now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackupFolderState {
    Live,
    /// Trashed, or no longer there (404, 410).
    Gone,
}

fn timestamp_ms(entry: &serde_json::Value) -> i64 {
    ["uploaded_at", "updated_at"]
        .iter()
        .find_map(|field| entry[*field].as_str())
        .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
        .map_or(0, |at| at.timestamp_millis())
}

impl EngineBridge {
    fn open_backup_name(&self, id: &str, name_encrypted: Option<&str>) -> Option<String> {
        let (key, _) = owned_key_and_pin(&self.db, self.api.master_key(), id).ok()?;
        beebeeb_core::encrypt::decrypt_name_with_key(&key, name_encrypted?)
    }

    /// Every live child of `parent`, every page read. `complete` is `false` when a page cursor repeated, the cap
    /// was reached, or a page was cut short: the caller then lists what it has and claims nothing about the rest.
    pub(crate) async fn backup_list_children(&self, parent: &str) -> Result<(Vec<BackupChild>, bool), EngineError> {
        let mut children = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen = std::collections::HashSet::new();
        loop {
            let page = self.backup_api().list_owned_folder_page(parent, cursor.as_deref()).await?;
            let Some(files) = page["files"].as_array() else {
                return Ok((children, false));
            };
            for entry in files {
                let Some(id) = entry["id"].as_str() else {
                    return Ok((children, false));
                };
                if entry["is_trashed"].as_bool() == Some(true) {
                    continue;
                }
                children.push(BackupChild {
                    id: id.to_string(),
                    name: self.open_backup_name(id, entry["name_encrypted"].as_str()),
                    is_folder: entry["is_folder"].as_bool() == Some(true),
                    size: entry["size_bytes"].as_u64().unwrap_or(0),
                    uploaded_at_ms: timestamp_ms(entry),
                });
            }
            if children.len() >= BACKUP_LISTING_CAP {
                return Ok((children, false));
            }
            match &page["next_cursor"] {
                serde_json::Value::String(next) if !next.is_empty() && seen.insert(next.clone()) => {
                    cursor = Some(next.clone())
                }
                serde_json::Value::Null => return Ok((children, true)),
                _ => return Ok((children, false)),
            }
        }
    }

    /// Whether the folder `id` is live: `GET /files/{id}`, trashed or 404/410 is gone. Any other failure is an error,
    /// never "gone" (a turn-off from a network error would be a silent loss of the backup).
    pub(crate) async fn backup_folder_state(&self, id: &str) -> Result<BackupFolderState, EngineError> {
        match self.backup_api().get_file(id).await {
            Ok(file) if file["is_trashed"].as_bool() == Some(true) => Ok(BackupFolderState::Gone),
            Ok(_) => Ok(BackupFolderState::Live),
            Err(error) if matches!(error_http_status(&error), Some(404 | 410)) => Ok(BackupFolderState::Gone),
            Err(error) => Err(error.into()),
        }
    }

    /// `BackupHost::ensure_folder`: the live folder `name` under `parent`, else a new one with a minted id and the
    /// name sealed for it. The listing match is exact (the pass folds names itself where the platform does).
    pub(crate) async fn backup_ensure_folder(&self, parent: &str, name: &str) -> Result<String, EngineError> {
        let (children, _) = self.backup_list_children(parent).await?;
        let mut existing: Vec<&BackupChild> =
            children.iter().filter(|c| c.is_folder && c.name.as_deref() == Some(name)).collect();
        existing.sort_by(|a, b| a.id.cmp(&b.id));
        if let Some(found) = existing.first() {
            return Ok(found.id.clone());
        }
        let master = beebeeb_core::kdf::MasterKey::from_bytes(*self.api.master_key());
        let id = beebeeb_core::item_id::NewItemId::owner(&master)?.as_str().to_string();
        let sealed = owned_metadata_for_name(&self.db, self.api.master_key(), &id, name, None)?;
        let created = self.backup_api().create_folder(&sealed, Some(parent), Some(&id)).await?;
        if created["id"].as_str() != Some(id.as_str()) {
            return Err(anyhow::anyhow!(
                "server_changed_item_id: the folder create did not answer with the id this device chose"
            )
            .into());
        }
        tracing::info!(folder_id = %id, "backup: folder created");
        Ok(id)
    }

    /// `BackupHost::move_file`: move and rename the server file `id` (its name sealed again for its own id).
    pub(crate) async fn backup_move_file(&self, id: &str, new_parent: &str, new_name: &str) -> Result<(), EngineError> {
        let mime = beebeeb_core::media::guess_mime_type(new_name);
        let sealed = owned_metadata_for_rename(&self.db, self.api.master_key(), id, new_name, mime)?;
        self.backup_api()
            .update_metadata(id, Some(&sealed), crate::api_client::ParentChange::Folder(new_parent))
            .await?;
        tracing::info!(file_id = %id, "backup: file moved");
        Ok(())
    }
}
