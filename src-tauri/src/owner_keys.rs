//! Opaque owner-key state and account-scoped transport. Crypto belongs to core.

use crate::{api_client::ApiClient, state_db::StateDb};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use beebeeb_core::{
    encrypt::NameMetadata,
    kdf::{FileKey, MasterKey},
};
use beebeeb_sync::key_resolution::{self as keys, FolderWrap};
use std::collections::{HashMap, HashSet};

pub struct PreparedOwnedRow {
    pub row: Value,
    pub record: OwnedKeyRecord,
    pub readable: bool,
}

struct Custody {
    key: FileKey,
    version: i32,
}
#[derive(Default)]
struct Batch {
    rows: HashMap<String, Value>,
    custody: HashMap<String, Option<Custody>>,
    inventory: HashMap<String, HashMap<String, Value>>,
    invites: Option<Vec<Value>>,
}

fn missing(error: &anyhow::Error) -> bool {
    crate::engine_bridge::error_http_status(error) == Some(404)
        || error.to_string().starts_with("HTTP 404 ")
}

/// These statuses describe the pre-feature custody endpoint only. Authentication,
/// transport and other server failures must never select the legacy reader.
pub(crate) fn custody_absent(error: &anyhow::Error) -> bool {
    matches!(crate::engine_bridge::error_http_status(error), Some(404 | 405 | 501))
        || ["HTTP 404 ", "HTTP 405 ", "HTTP 501 "].iter().any(|prefix| error.to_string().starts_with(prefix))
}

fn foreign(row: &Value) -> bool {
    !row["acted_via_share_id"].is_null() || !row["uploaded_by"].is_null()
}

fn folder(row: &Value) -> bool {
    row["is_folder"].as_bool() == Some(true) || row["kind"] == "folder"
}

fn authenticate(key: &FileKey, encrypted: &str) -> anyhow::Result<NameMetadata> {
    // Known foreign folders need authenticated names too. Core's display-only
    // folder reader permits plaintext legacy names; the file reader refuses it.
    let metadata = beebeeb_core::encrypt::decrypt_file_metadata_with_key(key, encrypted)?;
    anyhow::ensure!(
        !metadata.name.is_empty()
            && !metadata.name.contains('/')
            && !metadata.name.contains('\\')
            && crate::reject_unsafe_rel_path(&metadata.name).is_ok(),
        "unsafe decrypted item name"
    );
    Ok(metadata)
}

pub fn record_key(master: &MasterKey, record: &OwnedKeyRecord) -> anyhow::Result<(FileKey, NameMetadata)> {
    if record.blocked_fingerprint.as_ref() == Some(&record.fingerprint()) {
        anyhow::bail!("item key is unavailable");
    }
    for wrap in [record.owner_wrap.as_deref(), record.pending_owner_wrap.as_deref()]
        .into_iter()
        .flatten()
    {
        if let Ok(key) = keys::unwrap_owner_key(master, &record.file_id, wrap)
            && let Ok(metadata) = authenticate(&key, &record.name_encrypted)
        {
            return Ok((key, metadata));
        }
    }
    anyhow::ensure!(
        !record.foreign_added && record.owner_wrap.is_none() && record.pending_owner_wrap.is_none(),
        "item key is unavailable"
    );
    let resolved = keys::resolve_owned_key(master, &record.file_id, None, None, false)?;
    let metadata = authenticate(&resolved.key, &record.name_encrypted)?;
    Ok((resolved.key, metadata))
}

/// A failed row remains visible and deletable; this label is never ciphertext
/// or a path supplied by the server.
pub fn blocked_name(id: &str) -> String {
    let suffix = Sha256::digest(id.as_bytes())[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    format!("Unavailable file {suffix}")
}

pub fn stored_name(db: &StateDb, id: &str, master: &[u8; 32]) -> anyhow::Result<Option<String>> {
    let Some(record) = db.get_owned_key_record(id)? else {
        return Ok(None);
    };
    if record.blocked_fingerprint.is_some() {
        return Ok(Some(blocked_name(id)));
    }
    if record.foreign_added || record.owner_wrap.is_some() || record.pending_owner_wrap.is_some() {
        return Ok(Some(record_key(&MasterKey::from_bytes(*master), &record)?.1.name));
    }
    Ok(None)
}

impl Batch {
    async fn custody(&mut self, scope: &AccountScope<'_>, id: &str) -> anyhow::Result<()> {
        if self.custody.contains_key(id) {
            return Ok(());
        }
        scope.check()?;
        let response = scope.api.get_shared_folder(id).await;
        scope.check()?;
        let master = MasterKey::from_bytes(*scope.api.master_key());
        let custody = match response {
            Ok(row) => {
                // An invalid permanent record is never seeded from an invite.
                normalize_wrap(&row["owner_wrapped_folder_key"])
                    .ok()
                    .flatten()
                    .and_then(|wrap| keys::unwrap_folder_custody(&master, id, &wrap).ok())
                    .map(|key| Custody {
                        key,
                        version: row["key_version"].as_i64().unwrap_or(1) as i32,
                    })
            }
            Err(error) if custody_absent(&error) => {
                anyhow::ensure!(!scope.db.has_seen_folder_custody(id)?, "This folder's keys are unavailable");
                if !missing(&error) {
                    self.custody.insert(id.to_string(), None);
                    return Ok(());
                }
                if self.invites.is_none() {
                    let body = scope.api.sent_folder_invites().await?;
                    scope.check()?;
                    self.invites = Some(body["invites"].as_array().cloned().unwrap_or_default());
                }
                let seed = self
                    .invites
                    .as_ref()
                    .unwrap()
                    .iter()
                    .filter(|invite| invite["file_id"] == id && invite["is_folder_share"] == true)
                    .find_map(|invite| normalize_wrap(&invite["encrypted_owner_folder_key"]).ok().flatten());
                if let Some(seed) = seed {
                    // Named legacy invite seeding path only. No child or custody
                    // record ever falls back to the old unbound format.
                    if let Ok(key) = keys::unwrap_owner_key(&master, id, &seed) {
                        let wrap = STANDARD.encode(keys::wrap_folder_custody(&master, id, &key)?);
                        let registered = scope.api.register_shared_folder_key(id, &wrap, 1).await;
                        scope.check()?;
                        if let Err(error) = registered {
                            if !error.to_string().contains("key_version_conflict") {
                                return Err(error);
                            }
                        }
                        let winner = scope.api.get_shared_folder(id).await?;
                        scope.check()?;
                        normalize_wrap(&winner["owner_wrapped_folder_key"])
                            .ok()
                            .flatten()
                            .and_then(|wrap| keys::unwrap_folder_custody(&master, id, &wrap).ok())
                            .map(|key| Custody {
                                key,
                                version: winner["key_version"].as_i64().unwrap_or(1) as i32,
                            })
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            Err(error) => return Err(error),
        };
        if custody.is_some() {
            scope.remember_custody(id)?;
        }
        self.custody.insert(id.to_string(), custody);
        Ok(())
    }

    async fn shared_root(
        &mut self,
        scope: &AccountScope<'_>,
        row: &Value,
        previous: Option<&str>,
    ) -> anyhow::Result<Option<String>> {
        let mut next = row["parent_id"]
            .as_str()
            .map(str::to_string)
            .or_else(|| previous.map(str::to_string));
        let mut seen = HashSet::new();
        while let Some(id) = next {
            if !seen.insert(id.clone()) {
                return Ok(None);
            }
            self.custody(scope, &id).await?;
            if self.custody.get(&id).is_some_and(Option::is_some) {
                return Ok(Some(id));
            }
            if !self.rows.contains_key(&id) {
                scope.check()?;
                let parent = scope.api.get_file(&id).await;
                scope.check()?;
                match parent {
                    Ok(parent) => {
                        self.rows.insert(id.clone(), parent);
                    }
                    Err(e) if missing(&e) => return Ok(None),
                    Err(e) => return Err(e),
                }
            }
            next = self.rows[&id]["parent_id"].as_str().map(str::to_string);
        }
        Ok(None)
    }

    async fn child_wrap(&mut self, scope: &AccountScope<'_>, root: &str, id: &str) -> anyhow::Result<Option<Vec<u8>>> {
        if !self.inventory.contains_key(root) {
            let inventory = scope
                .inventory(root)
                .await?
                .into_iter()
                .filter_map(|r| Some((r["file_id"].as_str()?.to_string(), r)))
                .collect();
            self.inventory.insert(root.to_string(), inventory);
        }
        let value = self.inventory[root]
            .get(id)
            .map(|r| &r["encrypted_file_key"])
            .unwrap_or(&Value::Null);
        Ok(normalize_wrap(value).ok().flatten())
    }
}

async fn verify_winner(
    scope: &AccountScope<'_>,
    record: &mut OwnedKeyRecord,
    key: &FileKey,
    canonical: &Value,
) -> anyhow::Result<bool> {
    let Some(frame) = normalize_wrap(&canonical["owner_wrapped_file_key"])? else {
        return Ok(false);
    };
    let winner = keys::unwrap_owner_key(&MasterKey::from_bytes(*scope.api.master_key()), &record.file_id, &frame)?;
    if winner.as_bytes() != key.as_bytes() {
        record.blocked_fingerprint = Some(record.fingerprint());
        scope.put(record)?;
        anyhow::bail!("immutable owner envelope conflicts with authenticated item key");
    }
    record.owner_wrap = Some(frame);
    record.pending_owner_wrap = None;
    record.blocked_fingerprint = None;
    scope.put(record)?;
    Ok(true)
}

async fn adopt(
    scope: &AccountScope<'_>,
    record: &mut OwnedKeyRecord,
    key: &FileKey,
    read_first: bool,
) -> anyhow::Result<bool> {
    if read_first {
        let canonical = scope.api.get_file(&record.file_id).await?;
        scope.check()?;
        if verify_winner(scope, record, key, &canonical).await? {
            return Ok(true);
        }
    }
    // Persist before POST. A network error can follow a committed write, so
    // confirm the immutable winner even when POST's answer was lost.
    let _answer = scope.adopt(record).await;
    scope.check()?;
    let canonical = scope.api.get_file(&record.file_id).await?;
    scope.check()?;
    verify_winner(scope, record, key, &canonical).await
}

impl crate::engine_bridge::EngineBridge {
    pub(crate) async fn maintain_owned_keys(&self, now: i64) -> anyhow::Result<()> {
        let scope = self.owner_key_scope();
        scope.check()?;
        let dirty = self.db().dirty_shared_index_roots()?;
        let pending = self
            .db()
            .list_owned_key_records()?
            .iter()
            .any(|r| r.pending_owner_wrap.is_some() && r.blocked_fingerprint.is_none());
        if (pending || !dirty.is_empty()) && self.db().claim_owned_key_maintenance(now)? {
            let _ = self.retry_owner_adoptions().await;
            scope.check()?;
            for root in dirty {
                // Durable markers retain failed or partial work for the next pass.
                let _ = self.repair_shared_folder_index(&root).await;
                scope.check()?;
            }
        }
        Ok(())
    }

    pub(crate) async fn append_shared_activity(
        &self,
        feed: &mut crate::account_dto::ActivityFeed,
    ) -> anyhow::Result<()> {
        let scope = self.owner_key_scope();
        let roots: std::collections::BTreeSet<_> = self
            .db()
            .list_owned_key_records()?
            .into_iter()
            .filter_map(|r| r.shared_root_id)
            .collect();
        for root in roots {
            let Ok(rows) = self.shared_folder_activity(&root).await else {
                scope.check()?;
                continue;
            };
            for row in rows {
                let id = format!("shared-folder:{}:{}", root, row["id"].as_str().unwrap_or_default());
                if feed.events.iter().any(|e| e.id == id) {
                    continue;
                }
                let label = row["label"].as_str().unwrap_or("shared folder");
                let actor = row["actor"].as_str().unwrap_or("Someone");
                feed.events.push(crate::account_dto::ActivityFeedEvent {
                    id,
                    event_type: "shared_folder.activity".into(),
                    subject: Some(label.into()),
                    details: Some(format!(
                        "{actor}: {} ({})",
                        row["kind"].as_str().unwrap_or("updated"),
                        row["item_count"].as_u64().unwrap_or(1)
                    )),
                    origin: None,
                    created_at: row["last_at"].as_str().unwrap_or_default().into(),
                });
                feed.total += 1;
            }
        }
        scope.check()?;
        feed.events
            .sort_by(|a, b| b.created_at.cmp(&a.created_at).then_with(|| a.id.cmp(&b.id)));
        Ok(())
    }

    pub async fn prepare_owned_rows(&self, rows: &[Value]) -> anyhow::Result<Vec<PreparedOwnedRow>> {
        let scope = self.owner_key_scope();
        let master = MasterKey::from_bytes(*self.api().master_key());
        let mut batch = Batch::default();
        for row in rows {
            if let Some(id) = row["id"].as_str() {
                batch.rows.insert(id.to_string(), row.clone());
            }
        }
        let mut prepared = Vec::with_capacity(rows.len());
        for input in rows {
            scope.check()?;
            let Some(id) = input["id"].as_str().filter(|s| !s.is_empty()) else {
                continue;
            };
            let old = self.db().get_owned_key_record(id)?;
            let mut row = input.clone();
            let mut record = old.clone().unwrap_or_else(|| OwnedKeyRecord {
                file_id: id.to_string(),
                ..Default::default()
            });
            let known_foreign = record.foreign_added || foreign(input) || !input["owner_wrapped_file_key"].is_null();
            let text = input["name_encrypted"].as_str();
            let derived_readable =
                text.is_some_and(|text| beebeeb_core::encrypt::decrypt_name_metadata(&master, id, text).is_ok());
            let sparse = text.is_none() && input["name"].is_null() && input["path"].is_null();
            if sparse || (!derived_readable && !known_foreign && text.is_some()) {
                row = scope.api.get_file(id).await?;
                scope.check()?;
                anyhow::ensure!(row["id"].as_str() == Some(id), "canonical item id mismatch");
                batch.rows.insert(id.to_string(), row.clone());
            }
            record.foreign_added |= foreign(&row) || !row["owner_wrapped_file_key"].is_null();
            if let Some(text) = row["name_encrypted"].as_str() {
                record.name_encrypted = text.to_string();
            }
            let malformed = match normalize_wrap(&row["owner_wrapped_file_key"]) {
                Ok(Some(wrap)) => {
                    record.owner_wrap = Some(wrap);
                    false
                }
                Ok(None) => false,
                Err(_) => true,
            };
            record.member_added |= !row["uploaded_by"].is_null();
            record.origin_label_encrypted = row["origin_label_encrypted"]
                .as_str()
                .map(str::to_string)
                .or(record.origin_label_encrypted);
            record.blocked_fingerprint = None;
            let mut candidate = if malformed {
                None
            } else {
                record_key(&master, &record).ok()
            };
            let ordinary_legacy = !record.foreign_added
                && record.owner_wrap.is_none()
                && record.pending_owner_wrap.is_none()
                && (derived_readable || (text.is_none() && !sparse));
            if candidate.is_none() && !ordinary_legacy && !malformed {
                if let Some(root) = batch
                    .shared_root(&scope, &row, record.shared_root_id.as_deref())
                    .await?
                {
                    record.shared_root_id = Some(root.clone());
                    record.folder_key_version = batch.custody[&root].as_ref().map(|c| c.version);
                    record.folder_wrap = batch.child_wrap(&scope, &root, id).await?;
                    if let Some(custody) = batch.custody[&root].as_ref() {
                        let wrap = record.folder_wrap.as_deref().map(|wrapped_key| FolderWrap {
                            shared_folder_id: &root,
                            kind: if folder(&row) { 2 } else { 1 },
                            folder_key: &custody.key,
                            wrapped_key,
                        });
                        if let Ok(resolved) =
                            keys::resolve_owned_key(&master, id, record.owner_wrap.as_deref(), wrap, true)
                            && let Ok(metadata) = authenticate(&resolved.key, &record.name_encrypted)
                        {
                            record.foreign_added = true;
                            candidate = Some((resolved.key, metadata));
                        }
                    }
                }
            }
            let mut readable = candidate.is_some() || ordinary_legacy;
            if let Some((key, _)) = candidate.as_ref()
                && record.foreign_added
                && record.owner_wrap.is_none()
            {
                if record.pending_owner_wrap.is_none() {
                    record.pending_owner_wrap = Some(keys::wrap_key_to_owner(&master, id, key)?);
                }
            }
            let tree_or_content_changed = self.db().get_file_contract_state(id)?.is_some_and(|previous| {
                previous.parent_id.as_deref() != row["parent_id"].as_str()
                    || row["version_number"]
                        .as_i64()
                        .is_some_and(|version| version != previous.current_version)
            });
            let changed =
                tree_or_content_changed || old.as_ref().is_none_or(|old| old.fingerprint() != record.fingerprint());
            if changed {
                for root in old
                    .as_ref()
                    .and_then(|r| r.shared_root_id.as_deref())
                    .into_iter()
                    .chain(record.shared_root_id.as_deref())
                {
                    self.db().mark_shared_index_dirty(root)?;
                }
            }
            if !readable {
                record.blocked_fingerprint = Some(record.fingerprint());
            }
            scope.put(&record)?;
            if let Some((key, _)) = candidate.as_ref()
                && record.pending_owner_wrap.is_some()
            {
                // Adoption availability never blocks authenticated local reads.
                if let Err(_error) = adopt(&scope, &mut record, key, false).await {
                    scope.check()?;
                    if record.blocked_fingerprint.is_some() {
                        readable = false;
                    }
                }
            }
            prepared.push(PreparedOwnedRow { row, record, readable });
        }
        scope.check()?;
        Ok(prepared)
    }

    pub async fn resolved_owned_key(&self, id: &str) -> anyhow::Result<Arc<FileKey>> {
        let scope = self.owner_key_scope();
        scope.check()?;
        let master = MasterKey::from_bytes(*self.api().master_key());
        if let Some(record) = self.db().get_owned_key_record(id)? {
            return Ok(Arc::new(record_key(&master, &record)?.0));
        }
        Ok(Arc::new(keys::resolve_owned_key(&master, id, None, None, false)?.key))
    }

    pub async fn retry_owner_adoptions(&self) -> anyhow::Result<usize> {
        let scope = self.owner_key_scope();
        let master = MasterKey::from_bytes(*self.api().master_key());
        let mut confirmed = 0;
        for mut record in self.db().list_owned_key_records()? {
            scope.check()?;
            if record.pending_owner_wrap.is_none() || record.blocked_fingerprint.is_some() {
                continue;
            }
            if let Ok((key, _)) = record_key(&master, &record) {
                if adopt(&scope, &mut record, &key, true).await.unwrap_or(false) {
                    confirmed += 1;
                }
            }
        }
        scope.check()?;
        Ok(confirmed)
    }

    pub(crate) async fn shared_folder_custody_key(&self, id: &str) -> anyhow::Result<Arc<FileKey>> {
        let scope = self.owner_key_scope();
        let mut batch = Batch::default();
        batch.custody(&scope, id).await?;
        Ok(Arc::new(
            batch
                .custody
                .remove(id)
                .flatten()
                .ok_or_else(|| anyhow::anyhow!("folder custody unavailable"))?
                .key,
        ))
    }

    pub async fn shared_folder_activity(&self, id: &str) -> anyhow::Result<Vec<Value>> {
        let scope = self.owner_key_scope();
        let mut batch = Batch::default();
        batch.custody(&scope, id).await?;
        let key = &batch
            .custody
            .get(id)
            .and_then(Option::as_ref)
            .ok_or_else(|| anyhow::anyhow!("folder custody unavailable"))?
            .key;
        let body = self.api().shared_folder_activity(id).await?;
        scope.check()?;
        let mut result = Vec::new();
        for row in body["activity"].as_array().into_iter().flatten() {
            let mut dto = serde_json::json!({"id":row["id"],"kind":row["kind"],"item_count":row["item_count"],"first_at":row["first_at"],"last_at":row["last_at"]});
            for (encrypted, plain) in [("share_label_encrypted", "label"), ("actor_name_encrypted", "actor")] {
                dto[plain] = row[encrypted]
                    .as_str()
                    .filter(|s| s.starts_with('{'))
                    .and_then(|s| beebeeb_core::encrypt::decrypt_name_metadata_with_key(key, s).ok())
                    .map(|m| Value::String(m.name))
                    .unwrap_or(Value::Null);
            }
            result.push(dto);
        }
        Ok(result)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedKeyRecord {
    pub file_id: String,
    pub name_encrypted: String,
    pub owner_wrap: Option<Vec<u8>>,
    pub pending_owner_wrap: Option<Vec<u8>>,
    pub shared_root_id: Option<String>,
    pub folder_wrap: Option<Vec<u8>>,
    pub folder_key_version: Option<i32>,
    pub foreign_added: bool,
    #[serde(default)]
    pub member_added: bool,
    pub origin_label_encrypted: Option<String>,
    pub blocked_fingerprint: Option<String>,
}

impl OwnedKeyRecord {
    pub fn fingerprint(&self) -> String {
        let mut opaque = self.clone();
        opaque.blocked_fingerprint = None;
        STANDARD.encode(Sha256::digest(serde_json::to_vec(&opaque).expect("opaque record")))
    }
}

pub fn normalize_wrap(value: &Value) -> anyhow::Result<Option<Vec<u8>>> {
    let frame = match value {
        Value::Null => return Ok(None),
        Value::String(text) => STANDARD.decode(text)?,
        Value::Array(values) => values
            .iter()
            .map(|v| {
                v.as_u64()
                    .and_then(|n| u8::try_from(n).ok())
                    .ok_or_else(|| anyhow::anyhow!("invalid envelope byte"))
            })
            .collect::<anyhow::Result<Vec<_>>>()?,
        _ => anyhow::bail!("invalid envelope encoding"),
    };
    anyhow::ensure!(frame.len() == 60, "invalid envelope length");
    Ok(Some(frame))
}

/// A request result belongs to the account/session that started it. No mutex is
/// held over an await, and the DB performs the final generation check itself.
pub struct AccountScope<'a> {
    pub db: &'a StateDb,
    pub api: &'a ApiClient,
    pub stopping: &'a AtomicBool,
    revision: u64,
}

impl<'a> AccountScope<'a> {
    pub fn new(db: &'a StateDb, api: &'a ApiClient, stopping: &'a AtomicBool) -> Self {
        Self {
            db,
            api,
            stopping,
            revision: db.account_revision(),
        }
    }

    pub fn check(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.stopping.load(Ordering::Acquire) && self.db.account_revision() == self.revision,
            "owner key request cancelled by account change"
        );
        Ok(())
    }

    pub fn remember_custody(&self, folder_id: &str) -> anyhow::Result<()> {
        self.check()?;
        self.db.remember_folder_custody_at_revision(folder_id, self.revision)?;
        Ok(())
    }

    pub fn put(&self, row: &OwnedKeyRecord) -> anyhow::Result<()> {
        self.check()?;
        self.db.put_owned_key_record_at_revision(row, self.revision)?;
        Ok(())
    }

    pub async fn inventory(&self, root: &str) -> anyhow::Result<Vec<Value>> {
        let mut inventory = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen = std::collections::HashSet::new();
        loop {
            self.check()?;
            let page = self.api.get_shared_folder_keys(root, cursor.as_deref()).await?;
            self.check()?;
            inventory.extend(
                page["keys"]
                    .as_array()
                    .ok_or_else(|| anyhow::anyhow!("invalid key inventory"))?
                    .iter()
                    .cloned(),
            );
            match &page["next_cursor"] {
                Value::Null => break,
                Value::String(next) if !next.is_empty() && seen.insert(next.clone()) => cursor = Some(next.clone()),
                _ => anyhow::bail!("invalid or repeated key inventory cursor"),
            }
        }
        Ok(inventory)
    }

    pub async fn adopt(&self, row: &OwnedKeyRecord) -> anyhow::Result<bool> {
        let frame = row
            .pending_owner_wrap
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no pending owner envelope"))?;
        self.put(row)?;
        let result = self.api.adopt_owner_key(&row.file_id, &STANDARD.encode(frame)).await;
        self.check()?;
        result
    }
}

#[cfg(test)]
mod tests;

/// Recoverable item copy. Only an authenticated, decrypted label may be passed here.
pub fn cannot_open_message(label: Option<&str>, member_added: bool) -> String {
    let source = label.filter(|s| !s.trim().is_empty()).unwrap_or(if member_added { "shared folder" } else { "link" });
    format!("Can't open this file (added via {source})")
}
