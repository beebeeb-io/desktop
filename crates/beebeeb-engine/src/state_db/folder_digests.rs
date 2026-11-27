//! Folder digests the sync drift check trusts (folder digests, task 13): `remote_folder_digests`, one row per folder of
//! the person's own files whose server subtree the mirror was found to agree with, keyed by folder id (`""` = the
//! account root), and the mirror reads the check compares a server listing with.
//!
//! Owns: the table (created with the schema, emptied by sign-out as account data) and the check's reads of `files`
//! and `operation_queue`. Must not: decide what agrees (`engine_bridge::drift_check`) or log an id.

use std::collections::HashMap;

use super::*;

/// Create the table (`CREATE TABLE IF NOT EXISTS`, so a re-open is a no-op).
pub(super) fn create_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS remote_folder_digests (
             folder_id TEXT PRIMARY KEY,
             digest TEXT NOT NULL
         );",
    )
}

/// The rows directly under one folder of the person's own files (`""` = the root), as `files` holds them.
const CHILDREN_SQL: &str = "namespace = 'my_files'
     AND CASE WHEN ?1 = '' THEN (parent_id IS NULL OR parent_id = '') ELSE parent_id = ?1 END";

/// One mirror row under a folder, the fields the drift check compares.
#[derive(Debug, Clone, PartialEq)]
pub struct DriftChild {
    pub file_id: String,
    pub path: String,
    pub status: FileStatus,
    pub is_folder: bool,
    pub size_bytes: i64,
    pub current_version: i64,
    pub current_object_version_id: Option<String>,
    /// The row's newest server name could not be read: its path is the last good one.
    pub name_unreadable: bool,
}

impl StateDb {
    /// Every stored folder digest: folder id -> digest hex.
    pub fn remote_folder_digests(&self) -> Result<HashMap<String, String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare("SELECT folder_id, digest FROM remote_folder_digests")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        rows.collect()
    }

    /// Store `trusted` (folder id, digest) and forget `forget`, in one transaction.
    pub fn update_remote_folder_digests(&self, trusted: &[(String, String)], forget: &[String]) -> Result<()> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        {
            let mut put = tx.prepare_cached(
                "INSERT INTO remote_folder_digests (folder_id, digest) VALUES (?1, ?2)
                 ON CONFLICT(folder_id) DO UPDATE SET digest = excluded.digest",
            )?;
            for (folder, digest) in trusted {
                put.execute(params![folder, digest])?;
            }
            let mut drop = tx.prepare_cached("DELETE FROM remote_folder_digests WHERE folder_id = ?1")?;
            for folder in forget {
                drop.execute(params![folder])?;
            }
        }
        tx.commit()
    }

    /// Forget every stored folder digest (a re-snapshot was asked for: the next check reads every folder).
    pub fn clear_remote_folder_digests(&self) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute("DELETE FROM remote_folder_digests", [])?;
        Ok(())
    }

    /// The mirror's rows directly under `folder_id` (`""` = the root) in the person's own files, every status.
    pub fn drift_children(&self, folder_id: &str) -> Result<Vec<DriftChild>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT file_id, path, status, item_kind, size_bytes, current_version, current_object_version_id,
                    COALESCE(name_unreadable, 0)
             FROM files WHERE {CHILDREN_SQL}"
        ))?;
        let rows = stmt.query_map(params![folder_id], |r| {
            Ok(DriftChild {
                file_id: r.get(0)?,
                path: r.get(1)?,
                status: FileStatus::from_str(&r.get::<_, String>(2)?),
                is_folder: r.get::<_, String>(3)? == "folder",
                size_bytes: r.get(4)?,
                current_version: r.get(5)?,
                current_object_version_id: r.get(6)?,
                name_unreadable: r.get::<_, i64>(7)? != 0,
            })
        })?;
        rows.collect()
    }

    /// A local change waits for the server in `folder_id` (`""` = the root): an op (parked included) on one of
    /// its rows, or one that creates or moves something into it. A download is not a change.
    pub fn local_op_queued_in_folder(&self, folder_id: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM operation_queue
                 WHERE kind != 'hydrate_file'
                   AND ((?1 != '' AND parent_id = ?1)
                        OR file_id IN (SELECT file_id FROM files WHERE {CHILDREN_SQL})))"
            ),
            params![folder_id],
            |r| r.get(0),
        )
    }
}
