//! fd-18: the backup digest cache on disk (`backup_digest_cache`), so a restart does not walk every folder of every
//! backup again: one row per folder of a backup as its last listing gave it (the digest trusted for its subtree, the
//! children encoded by `backup::digest_cache`, whether that listing was complete), keyed by the account, the backup
//! (category) and the folder id, and stamped with the device folder and category folder it was read under.
//!
//! Owns: the table (created with the schema, emptied by sign-out as account data). Rows of another account, another
//! device folder or another category folder are dropped when read or written. Must not: decide what is trusted
//! (`backup::digest_cache`, `backup::host`) or log an id.

use super::*;

/// Create the table (`CREATE TABLE IF NOT EXISTS`, so a re-open is a no-op).
pub(super) fn create_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS backup_digest_cache (
             account_owner TEXT NOT NULL,
             device_folder_id TEXT NOT NULL,
             category TEXT NOT NULL,
             category_folder_id TEXT NOT NULL,
             folder_id TEXT NOT NULL,
             digest TEXT,
             complete INTEGER NOT NULL,
             children_json TEXT NOT NULL,
             full_at_ms INTEGER,
             PRIMARY KEY (account_owner, category, folder_id)
         );",
    )
}

/// One folder of a backup's digest cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupDigestRow {
    pub folder_id: String,
    pub digest: Option<String>,
    pub complete: bool,
    pub children_json: String,
}

/// One backup's stored cache.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BackupDigestCategory {
    pub category: String,
    pub category_folder_id: String,
    /// When the backup was last compared in full with one snapshot (Unix ms).
    pub full_at_ms: Option<i64>,
    pub folders: Vec<BackupDigestRow>,
}

/// One pass's changes to one backup's cache, written in one transaction.
pub struct BackupDigestWrite<'a> {
    pub device_folder_id: &'a str,
    pub category: &'a str,
    pub category_folder_id: &'a str,
    /// `Some`: the backup was compared in full now.
    pub full_at_ms: Option<i64>,
    pub put: &'a [BackupDigestRow],
    pub forget: &'a [String],
}

impl StateDb {
    /// The current owner's stored digest caches under `device_folder_id`, and the owner key they belong to (`None`:
    /// no owner, nothing stored). Rows of another owner or another device folder are dropped.
    pub fn backup_digest_cache(&self, device_folder_id: &str) -> Result<(Option<String>, Vec<BackupDigestCategory>)> {
        let owner = self.backup_owner_key()?;
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let Some(owner) = owner else {
            conn.execute("DELETE FROM backup_digest_cache", [])?;
            return Ok((None, Vec::new()));
        };
        conn.execute(
            "DELETE FROM backup_digest_cache WHERE account_owner != ?1 OR device_folder_id != ?2",
            params![owner, device_folder_id],
        )?;
        let mut stmt = conn.prepare(
            "SELECT category, category_folder_id, folder_id, digest, complete, children_json, full_at_ms
             FROM backup_digest_cache WHERE account_owner = ?1 ORDER BY category",
        )?;
        let rows = stmt.query_map(params![owner], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                BackupDigestRow {
                    folder_id: r.get(2)?,
                    digest: r.get(3)?,
                    complete: r.get::<_, i64>(4)? != 0,
                    children_json: r.get(5)?,
                },
                r.get::<_, Option<i64>>(6)?,
            ))
        })?;
        let mut out: Vec<BackupDigestCategory> = Vec::new();
        for row in rows {
            let (category, category_folder_id, folder, full_at_ms) = row?;
            let same = out
                .last()
                .is_some_and(|last| last.category == category && last.category_folder_id == category_folder_id);
            if !same {
                out.push(BackupDigestCategory {
                    category,
                    category_folder_id,
                    full_at_ms: None,
                    folders: Vec::new(),
                });
            }
            let last = out.last_mut().expect("pushed above");
            last.full_at_ms = last.full_at_ms.max(full_at_ms);
            last.folders.push(folder);
        }
        Ok((Some(owner), out))
    }

    /// Write one pass's changes to one backup's cache (nothing when no owner is recorded). The backup's rows read under
    /// another device or category folder go first.
    pub fn put_backup_digest_cache(&self, write: &BackupDigestWrite<'_>) -> Result<()> {
        let Some(owner) = self.backup_owner_key()? else {
            return Ok(());
        };
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM backup_digest_cache
             WHERE account_owner = ?1 AND category = ?2 AND (device_folder_id != ?3 OR category_folder_id != ?4)",
            params![owner, write.category, write.device_folder_id, write.category_folder_id],
        )?;
        {
            let mut put = tx.prepare_cached(
                "INSERT INTO backup_digest_cache
                     (account_owner, device_folder_id, category, category_folder_id, folder_id, digest, complete,
                      children_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(account_owner, category, folder_id) DO UPDATE
                     SET device_folder_id = excluded.device_folder_id,
                         category_folder_id = excluded.category_folder_id,
                         digest = excluded.digest,
                         complete = excluded.complete,
                         children_json = excluded.children_json",
            )?;
            for row in write.put {
                put.execute(params![
                    owner,
                    write.device_folder_id,
                    write.category,
                    write.category_folder_id,
                    row.folder_id,
                    row.digest,
                    row.complete as i64,
                    row.children_json
                ])?;
            }
            let mut forget = tx.prepare_cached(
                "DELETE FROM backup_digest_cache WHERE account_owner = ?1 AND category = ?2 AND folder_id = ?3",
            )?;
            for folder in write.forget {
                forget.execute(params![owner, write.category, folder])?;
            }
        }
        if let Some(at) = write.full_at_ms {
            tx.execute(
                "UPDATE backup_digest_cache SET full_at_ms = ?3
                 WHERE account_owner = ?1 AND category = ?2 AND folder_id = category_folder_id",
                params![owner, write.category, at],
            )?;
        }
        tx.commit()
    }
}

#[cfg(test)]
mod tests {
    use crate::state_db::StateDb;

    use super::{BackupDigestRow, BackupDigestWrite};

    fn row(id: &str, digest: Option<&str>) -> BackupDigestRow {
        BackupDigestRow {
            folder_id: id.into(),
            digest: digest.map(Into::into),
            complete: true,
            children_json: "[]".into(),
        }
    }

    #[test]
    fn the_cache_is_the_owners_and_the_device_folders() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.set_owner(&crate::account_binding::Identity::new(Some("u-1"), None))
            .unwrap();
        let write = |category: &str, root: &str, put: &[BackupDigestRow], forget: &[String], full: Option<i64>| {
            db.put_backup_digest_cache(&BackupDigestWrite {
                device_folder_id: "dev",
                category,
                category_folder_id: root,
                full_at_ms: full,
                put,
                forget,
            })
            .unwrap()
        };
        write(
            "pictures",
            "cat",
            &[row("cat", Some("d1")), row("a", None)],
            &[],
            Some(7),
        );
        write("pictures", "cat", &[row("b", Some("d2"))], &["a".to_string()], None);
        let (owner, stored) = db.backup_digest_cache("dev").unwrap();
        assert_eq!(owner.as_deref(), Some("user:u-1"));
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].full_at_ms, Some(7));
        let mut ids: Vec<&str> = stored[0].folders.iter().map(|f| f.folder_id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, ["b", "cat"]);

        // Another category folder replaces the backup's rows.
        write("pictures", "cat2", &[row("cat2", None)], &[], None);
        let (_, stored) = db.backup_digest_cache("dev").unwrap();
        assert_eq!(stored[0].category_folder_id, "cat2");
        assert_eq!(stored[0].folders.len(), 1);

        // Another device folder drops them all.
        let (_, stored) = db.backup_digest_cache("other").unwrap();
        assert!(stored.is_empty());
        assert!(db.backup_digest_cache("dev").unwrap().1.is_empty());
    }
}
