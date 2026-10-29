//! Shared-folder index repair. All sealing and merging of encrypted pages uses
//! core; the HTTP adapter sends only opaque bytes with the observed CAS version.
use crate::{engine_bridge::EngineBridge, owner_keys};
use beebeeb_core::{
    kdf::MasterKey,
    search_index::{EncryptedShard, SearchIndex, ShardKeySource},
};
use std::collections::{BTreeMap, HashSet, VecDeque};

#[derive(Debug, Default)]
pub struct IndexRepairReport {
    pub written: usize,
    pub deferred: usize,
    pub partial: bool,
}

impl EngineBridge {
    pub async fn repair_shared_folder_index(&self, folder_id: &str) -> anyhow::Result<IndexRepairReport> {
        let scope = self.owner_key_scope();
        scope.check()?;
        let dirty = self.db().shared_index_dirty_revision(folder_id)?;
        let key = match self.shared_folder_custody_key(folder_id).await {
            Ok(key) => key,
            // Review 2276 P3-B: a legacy-proven folder has no custody and no
            // retry can give it one before its owner upgrades (L11). Clear the
            // marker at the revision read above so maintenance does not ask
            // this folder for custody again every 30 seconds.
            Err(error)
                if error.downcast_ref::<owner_keys::SharedFolderKeyError>()
                    == Some(&owner_keys::SharedFolderKeyError::NoCustody) =>
            {
                self.db()
                    .clear_shared_index_dirty_at_revision(folder_id, dirty.as_deref())?;
                return Ok(IndexRepairReport::default());
            }
            Err(error) => return Err(error),
        };
        let mut report = IndexRepairReport::default();
        let mut queue = VecDeque::from([folder_id.to_string()]);
        let mut seen = HashSet::new();
        let mut pairs = Vec::new();
        let mut listed = HashSet::new();
        while let Some(parent) = queue.pop_front() {
            if !seen.insert(parent.clone()) {
                report.partial = true;
                continue;
            }
            let mut cursor: Option<String> = None;
            let mut cursors = HashSet::new();
            loop {
                scope.check()?;
                let body = self.api().list_owned_folder_page(&parent, cursor.as_deref()).await?;
                scope.check()?;
                let rows = body["files"]
                    .as_array()
                    .ok_or_else(|| anyhow::anyhow!("invalid folder listing"))?;
                let prepared = self.prepare_owned_rows(rows).await?;
                for item in prepared {
                    if listed.len() >= 20_000 {
                        report.partial = true;
                        break;
                    }
                    let id = item.record.file_id.clone();
                    listed.insert(id.clone());
                    if !item.readable {
                        report.partial = true;
                        continue;
                    }
                    if item.row["is_folder"] == true {
                        queue.push_back(id.clone());
                    }
                    let master = MasterKey::from_bytes(*self.api().master_key());
                    let name = owner_keys::record_key(&master, &item.record)
                        .map(|(_, m)| m.name)
                        .or_else(|_| {
                            beebeeb_core::encrypt::decrypt_name(&master, &id, &item.record.name_encrypted)
                                .map_err(anyhow::Error::from)
                        })?;
                    pairs.push((id, name));
                }
                if listed.len() >= 20_000 {
                    queue.clear();
                    break;
                }
                match &body["next_cursor"] {
                    serde_json::Value::Null => break,
                    serde_json::Value::String(next) if !next.is_empty() && cursors.insert(next.clone()) => {
                        cursor = Some(next.clone())
                    }
                    _ => anyhow::bail!("invalid or repeated folder listing cursor"),
                }
            }
        }
        if report.partial {
            report.deferred = 1;
            return Ok(report);
        }
        // Replay only our known changes/removals. A concurrent device's entries
        // survive even if a fresh listing has not observed that device yet.
        let removals: Vec<String> = self
            .db()
            .list_owned_key_records()?
            .into_iter()
            .filter(|r| r.shared_root_id.as_deref() == Some(folder_id) && !listed.contains(&r.file_id))
            .map(|r| r.file_id)
            .collect();
        for attempt in 0..3 {
            scope.check()?;
            let manifest = self.api().shared_index_manifest(folder_id).await?;
            scope.check()?;
            let mut versions = BTreeMap::new();
            let mut shards = Vec::new();
            for entry in manifest["shards"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("invalid folder index manifest"))?
            {
                let bucket = entry["bucket"]
                    .as_u64()
                    .filter(|n| *n < 64)
                    .ok_or_else(|| anyhow::anyhow!("invalid index bucket"))? as u32;
                let page = entry["page"]
                    .as_u64()
                    .filter(|n| *n < 16)
                    .ok_or_else(|| anyhow::anyhow!("invalid index page"))? as u32;
                let version = entry["version"]
                    .as_i64()
                    .filter(|n| *n > 0)
                    .ok_or_else(|| anyhow::anyhow!("invalid index version"))?;
                versions.insert((bucket, page), version);
                if let Some(blob) = self.api().shared_index_get(folder_id, bucket, page).await? {
                    let shard = EncryptedShard { bucket, page, blob };
                    // A corrupt page falls back to the freshly authenticated listing.
                    if SearchIndex::from_encrypted_shards_with(
                        ShardKeySource::Folder(&key),
                        std::slice::from_ref(&shard),
                        64,
                    )
                    .is_ok()
                    {
                        shards.push(shard);
                    }
                }
                scope.check()?;
            }
            let mut index = SearchIndex::from_encrypted_shards_with(ShardKeySource::Folder(&key), &shards, 64)?;
            for id in &removals {
                index.remove(id);
            }
            for (id, name) in &pairs {
                index.upsert(id, name);
            }
            let encrypted = index.encrypt_shards_with(ShardKeySource::Folder(&key))?;
            let mut updates: BTreeMap<(u32, u32), Option<Vec<u8>>> =
                versions.keys().map(|coord| (*coord, None)).collect();
            for shard in encrypted {
                updates.insert((shard.bucket, shard.page), Some(shard.blob));
            }
            let mut conflict = false;
            for ((bucket, page), blob) in updates {
                scope.check()?;
                match self
                    .api()
                    .shared_index_write(
                        folder_id,
                        bucket,
                        page,
                        versions.get(&(bucket, page)).copied().unwrap_or(0),
                        blob.as_deref(),
                    )
                    .await
                {
                    Ok(()) => report.written += 1,
                    Err(error) if error.to_string().contains("index_version_conflict") => {
                        conflict = true;
                        break;
                    }
                    Err(error) => return Err(error),
                }
                scope.check()?;
            }
            if !conflict {
                scope.check()?;
                self.db()
                    .clear_shared_index_dirty_at_revision(folder_id, dirty.as_deref())?;
                return Ok(report);
            }
            if attempt == 2 {
                report.deferred = 1;
            }
        }
        Ok(report)
    }
}
