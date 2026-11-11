//! LD-0113 (spec §4.2 rules 3–6, §5.2): the server side of the backup's root resolution. The Backups root
//! (`GET`/`PUT /api/v1/backups/root`), the root-level listing, the merge's moves and trashes, and the device
//! manifest (`.device.json`), a small encrypted file written and read here. Every request goes through the backup
//! client (`X-Beebeeb-Origin: backup`). Names are sealed and opened on this device only; logs carry ids and counts.

use super::*;

/// The largest device manifest this device reads. A bigger `.device.json` is unreadable (not mine, never adopted).
pub const MANIFEST_READ_CAP: u64 = 1 << 20;

/// One root-level item, as the root resolution needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootListed {
    pub id: String,
    /// `None` when the name cannot be opened: it never counts as `Backups`.
    pub name: Option<String>,
    pub is_folder: bool,
    pub is_trashed: bool,
    pub created_at_ms: i64,
    pub system_role: Option<beebeeb_types::backup::SystemRole>,
}

fn created_at_ms(entry: &serde_json::Value) -> i64 {
    entry["created_at"]
        .as_str()
        .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
        .map_or(0, |at| at.timestamp_millis())
}

impl EngineBridge {
    /// `GET /api/v1/backups/root`: the flagged root's id, `None` when the account has none yet.
    pub async fn backup_root_flagged(&self) -> Result<Option<String>, EngineError> {
        Ok(self.backup_api().get_backups_root().await?)
    }

    /// `PUT /api/v1/backups/root` with `folder_id`: the account's root as the server resolved it (another device's
    /// root when it won a race). `create`: the name `Backups` is sealed for `folder_id`, so the server can create it.
    pub async fn backup_root_put(&self, folder_id: &str, create: bool) -> Result<String, EngineError> {
        let sealed = if create {
            Some(owned_metadata_for_name(
                &self.db,
                self.api.master_key(),
                folder_id,
                beebeeb_core::backup::ROOT_NAME,
                None,
            )?)
        } else {
            None
        };
        let answer = self.backup_api().put_backups_root(folder_id, sealed.as_deref()).await?;
        if uuid::Uuid::parse_str(&answer.id).is_err() {
            return Err(anyhow::anyhow!("the backups root answer is not a uuid").into());
        }
        tracing::info!(root_id = %answer.id, outcome = %answer.outcome, "backup: root resolved on the server");
        Ok(answer.id)
    }

    /// A fresh id for a root this device creates.
    pub fn backup_mint_id(&self) -> Result<String, EngineError> {
        let master = beebeeb_core::kdf::MasterKey::from_bytes(*self.api.master_key());
        Ok(beebeeb_core::item_id::NewItemId::owner(&master)?.as_str().to_string())
    }

    /// Every root-level item of the account, trashed ones included (flagged), every page read. `complete` is
    /// `false` when a cursor repeated, a page had no `files`, an id was missing or the cap was reached.
    pub async fn backup_list_root(&self) -> Result<(Vec<RootListed>, bool), EngineError> {
        let mut listed = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen = std::collections::HashSet::new();
        loop {
            let page = self.backup_api().list_root_page(cursor.as_deref()).await?;
            let Some(files) = page["files"].as_array() else {
                return Ok((listed, false));
            };
            for entry in files {
                let Some(id) = entry["id"].as_str() else {
                    return Ok((listed, false));
                };
                let is_folder = entry["is_folder"].as_bool() == Some(true);
                listed.push(RootListed {
                    id: id.to_string(),
                    name: is_folder
                        .then(|| self.open_backup_name(id, entry["name_encrypted"].as_str()))
                        .flatten(),
                    is_folder,
                    is_trashed: entry["is_trashed"].as_bool() == Some(true),
                    created_at_ms: created_at_ms(entry),
                    system_role: entry["system_role"]
                        .as_str()
                        .and_then(beebeeb_types::backup::SystemRole::parse),
                });
            }
            if listed.len() >= super::backup_tree::BACKUP_LISTING_CAP {
                return Ok((listed, false));
            }
            match &page["next_cursor"] {
                serde_json::Value::String(next) if !next.is_empty() && seen.insert(next.clone()) => {
                    cursor = Some(next.clone())
                }
                serde_json::Value::Null => return Ok((listed, true)),
                _ => return Ok((listed, false)),
            }
        }
    }

    /// The merge's move (`new_name: None`) or move with a new name (a clash kept both, core's `keep_both_name`).
    pub async fn backup_move(&self, id: &str, new_parent: &str, new_name: Option<&str>) -> Result<(), EngineError> {
        match new_name {
            Some(name) => self.backup_move_file(id, new_parent, name).await,
            None => {
                self.backup_api()
                    .update_metadata(id, None, crate::api_client::ParentChange::Folder(new_parent))
                    .await?;
                tracing::info!(file_id = %id, "backup: item moved");
                Ok(())
            }
        }
    }

    /// The merge's last step: an emptied duplicate root goes to Trash (never a permanent delete).
    pub(crate) async fn backup_trash(&self, id: &str) -> Result<(), EngineError> {
        self.backup_api().trash_file(id).await?;
        tracing::info!(file_id = %id, "backup: emptied duplicate moved to Trash");
        Ok(())
    }

    /// Upload `bytes` as a new file `name` under `parent` (the device manifest): an id minted here, the name sealed
    /// for it, the bytes encrypted for it, one upload session. Answers the new file's id.
    pub async fn backup_write_small_file(
        &self,
        parent: &str,
        name: &str,
        bytes: &[u8],
        mime: &str,
    ) -> Result<String, EngineError> {
        let id = self.backup_mint_id()?;
        let name_encrypted = owned_metadata_for_name(&self.db, self.api.master_key(), &id, name, Some(mime))?;
        let size = bytes.len() as u64;
        let request = upload_init_request_for_operation(
            &id,
            &name_encrypted,
            Some(mime.to_string()),
            Some(parent.to_string()),
            size,
            None,
        );
        let upload = self.backup_api().init_upload(&request).await?;
        if upload.file_id != id {
            return Err(anyhow::anyhow!(
                "server_changed_item_id: the upload init answered with an id this device did not choose"
            )
            .into());
        }
        let chunk_size = u64::try_from(upload.chunk_size_bytes).unwrap_or(0);
        let chunk_count = u64::try_from(upload.chunk_count).unwrap_or(0);
        if chunk_size == 0 || chunk_count == 0 {
            return Err(anyhow::anyhow!("upload init returned invalid chunk plan").into());
        }
        let (file_key, pin) = owned_key_and_pin(&self.db, self.api.master_key(), &id)?;
        let mut writer = beebeeb_core::chunk_stream::ChunkEncryptor::from_reader_with_file_key(
            file_key,
            &id,
            size,
            chunk_size,
            std::io::Cursor::new(bytes.to_vec()),
        )?
        .with_recorded_pin(pin)?;
        for index in 0..chunk_count {
            let chunk = writer
                .next_chunk()?
                .ok_or_else(|| anyhow::anyhow!("small file ended early"))?;
            self.backup_api()
                .upload_session_chunk(&upload.upload_session_id, index as u32, &chunk.data)
                .await?;
        }
        writer.finish()?;
        self.backup_api()
            .complete_upload_session(&upload.upload_session_id)
            .await?;
        tracing::info!(file_id = %id, bytes = size, "backup: small file written");
        Ok(id)
    }

    /// The plaintext of the small file `id` (`chunk_count` chunks), read in full. `Ok(None)`: it cannot be opened on
    /// this device (no key, a frame that does not open, or over [`MANIFEST_READ_CAP`]). A network or server failure
    /// is an error, never `None`: an unread manifest must not look like another device's.
    pub async fn backup_read_small_file(
        &self,
        id: &str,
        chunk_count: u32,
        size: u64,
    ) -> Result<Option<Vec<u8>>, EngineError> {
        if size > MANIFEST_READ_CAP || chunk_count == 0 {
            return Ok(None);
        }
        let Ok((file_key, pin)) = owned_key_and_pin(&self.db, self.api.master_key(), id) else {
            return Ok(None);
        };
        let mut reader = beebeeb_core::chunk_stream::ChunkDecryptor::for_push_with_file_key(file_key, id, pin);
        let mut plain = Vec::new();
        for index in 0..chunk_count {
            let bytes = self.backup_api().download_chunk(id, index, &|_| {}).await?;
            let Ok(frame) = content_frame(&bytes) else {
                return Ok(None);
            };
            match reader.push_frame(&frame) {
                Ok(opened) => plain.extend_from_slice(&opened.data),
                Err(_) => return Ok(None),
            }
            if plain.len() as u64 > MANIFEST_READ_CAP {
                return Ok(None);
            }
        }
        Ok(reader.finish().is_ok().then_some(plain))
    }
}
