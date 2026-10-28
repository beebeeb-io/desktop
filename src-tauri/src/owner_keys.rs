//! Opaque owner-key state and account-scoped transport. Crypto belongs to core.

use crate::{api_client::ApiClient, state_db::StateDb};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};

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
