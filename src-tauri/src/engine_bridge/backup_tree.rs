//! LD-0113 Task D4: the backup pass's server side of the tree (`BackupHost::{device_tree, ensure_folder,
//! move_file}`): listing, folder creates and moves under `Backups/<device>/<category>`. Every request goes through
//! the backup client (`X-Beebeeb-Origin: backup`, spec Amendment 8), never through the app's mutation commands, so
//! the protection layer (`check_app_mutation`, `check_drive_mutation`) is never on the way (it guards the drive
//! surface; the engine's own placeholder changes pass it as `by_this_process`). Names are sealed and opened on this
//! device only; the logs carry ids and counts.

use super::*;
use beebeeb_core::backup::RebuildClock;

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
    /// How many chunks the file's current version has (a small file's read, the device manifest's).
    pub(crate) chunk_count: u32,
}

/// A folder the backup knows by id, as the server has it now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackupFolderState {
    Live,
    /// Trashed, or no longer there (404, 410).
    Gone,
}

/// Of two clocks of the same listing, the measured one with the smaller margin; a measured clock always beats
/// `RebuildClock::UNKNOWN`.
pub(crate) fn tighter_clock(a: RebuildClock, b: RebuildClock) -> RebuildClock {
    match (a.offset_ms, b.offset_ms) {
        (Some(_), Some(_)) if b.margin_ms < a.margin_ms => b,
        (Some(_), _) => a,
        (None, _) => b,
    }
}

pub(super) fn timestamp_ms(entry: &serde_json::Value) -> i64 {
    ["uploaded_at", "updated_at"]
        .iter()
        .find_map(|field| entry[*field].as_str())
        .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
        .map_or(0, |at| at.timestamp_millis())
}

impl EngineBridge {
    pub(super) fn open_backup_name(&self, id: &str, name_encrypted: Option<&str>) -> Option<String> {
        let (key, _) = owned_key_and_pin(&self.db, self.api.master_key(), id).ok()?;
        beebeeb_core::encrypt::decrypt_name_with_key(&key, name_encrypted?)
    }

    /// Every live child of `parent`, every page read. `complete` is `false` when a page cursor repeated, the cap
    /// was reached, or a page was cut short: the caller then lists what it has and claims nothing about the rest.
    pub(crate) async fn backup_list_children(&self, parent: &str) -> Result<(Vec<BackupChild>, bool), EngineError> {
        let (children, complete, _) = self.backup_list_children_clocked(parent).await?;
        Ok((children, complete))
    }

    /// [`Self::backup_list_children`] with the listing's clock (LD-0123): every page measures the device's offset
    /// from the server's `Date` header, and the measurement with the smallest margin is kept (each bounds the same
    /// offset; the shortest round trip bounds it tightest). No page measured: `RebuildClock::UNKNOWN`.
    pub(crate) async fn backup_list_children_clocked(
        &self,
        parent: &str,
    ) -> Result<(Vec<BackupChild>, bool, RebuildClock), EngineError> {
        let mut children = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen = std::collections::HashSet::new();
        let mut clock = RebuildClock::UNKNOWN;
        loop {
            let (page, page_clock) = self.backup_api().list_owned_folder_page_clocked(parent, cursor.as_deref()).await?;
            clock = tighter_clock(clock, page_clock);
            let Some(files) = page["files"].as_array() else {
                return Ok((children, false, clock));
            };
            for entry in files {
                let Some(id) = entry["id"].as_str() else {
                    return Ok((children, false, clock));
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
                    chunk_count: entry["chunk_count"].as_u64().and_then(|n| u32::try_from(n).ok()).unwrap_or(0),
                });
            }
            if children.len() >= BACKUP_LISTING_CAP {
                return Ok((children, false, clock));
            }
            match &page["next_cursor"] {
                serde_json::Value::String(next) if !next.is_empty() && seen.insert(next.clone()) => {
                    cursor = Some(next.clone())
                }
                serde_json::Value::Null => return Ok((children, true, clock)),
                _ => return Ok((children, false, clock)),
            }
        }
    }

    /// LD-0139: the notice check a backup pass starts with (`crate::backup::stop::check_from`). Any failure is the
    /// failed check (no pass runs), in words with no server text.
    pub(crate) async fn backup_notice_check(&self) -> Result<crate::backup::stop::NoticeCheck, String> {
        let (body, clock, server_now_ms) = self.backup_api().list_notifications_clocked().await.map_err(|error| {
            let error = EngineError::from(error);
            format!("the notices request failed ({})", error_category(&error))
        })?;
        crate::backup::stop::check_from(&body, clock, server_now_ms)
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
        let (children, complete) = self.backup_list_children(parent).await?;
        let mut existing: Vec<&BackupChild> =
            children.iter().filter(|c| c.is_folder && c.name.as_deref() == Some(name)).collect();
        existing.sort_by(|a, b| a.id.cmp(&b.id));
        if let Some(found) = existing.first() {
            return Ok(found.id.clone());
        }
        // Review (2026-11-07): a folder missing from a partial listing may be there (the old mirror's
        // `GLegion/Pictures`); creating one would start a second folder of that name and upload it all again.
        if !complete {
            return Err(anyhow::anyhow!("backup_listing_incomplete: the folder is not created from a partial listing").into());
        }
        self.backup_create_folder(parent, name).await
    }

    /// A new folder `name` under `parent`, with an id minted here and its name sealed for it. Never adopts an existing
    /// folder of that name: a device folder named like another device's is that device's (core's identity rule).
    pub(crate) async fn backup_create_folder(&self, parent: &str, name: &str) -> Result<String, EngineError> {
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
