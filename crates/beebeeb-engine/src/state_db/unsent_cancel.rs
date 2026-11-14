//! What the Uploads panel needs beyond the list: the sizes of the files it names, and the cancel of an upload of a
//! NEW file (one never in Beebeeb): its queued change and its row go, in one transaction, and nothing is sent.
//! The local copy is the caller's (it goes to the Recycle Bin first). Task: bind-all, files-uploads.

use super::*;
use std::collections::HashMap;

/// What [`StateDb::new_upload_target`] and [`StateDb::drop_new_upload`] say of a change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CancelNew {
    /// A new file's upload, not being sent, with nothing else queued for its row: `file_id`'s row.
    Ready { file_id: String },
    /// No such change (sent meanwhile, or never there).
    Unknown,
    /// Not the upload of a new file (an edit, a rename, a backup, a folder, a download): it is never cancelled here.
    NotNew,
    /// Being sent right now, or something else is queued for the same file: left as it is.
    Busy,
}

impl StateDb {
    /// The size in bytes of each of these files this account holds (one query). A file with no row is left out.
    pub fn sizes_of(&self, file_ids: &[String]) -> Result<HashMap<String, i64>> {
        if file_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let marks = vec!["?"; file_ids.len()].join(",");
        let mut stmt = conn.prepare(&format!(
            "SELECT file_id, size_bytes FROM files WHERE file_id IN ({marks})"
        ))?;
        let rows = stmt.query_map(rusqlite::params_from_iter(file_ids.iter()), |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;
        rows.collect()
    }

    /// Is this change the upload of a NEW file that may be cancelled? A read; [`Self::drop_new_upload`] decides again.
    pub fn new_upload_target(&self, op_id: &str) -> Result<CancelNew> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        decide_conn(&conn, op_id)
    }

    /// Cancel the upload of a NEW file: its change, its resume row and the file's row go, in one transaction, the
    /// staged copy is released to the journal. Decided again inside it ([`CancelNew::Ready`] was dropped), so a
    /// change that moved since is left as it is. Nothing is sent to Beebeeb.
    pub fn drop_new_upload(&self, op_id: &str) -> Result<CancelNew> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let CancelNew::Ready { file_id } = decide_conn(&tx, op_id)? else {
            return decide_conn(&tx, op_id);
        };
        let payload: Option<String> = tx
            .query_row(
                "SELECT payload_path FROM operation_queue WHERE op_id = ?1",
                params![op_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        tx.execute("DELETE FROM operation_queue WHERE op_id = ?1", params![op_id])?;
        tx.execute("DELETE FROM upload_resume WHERE op_id = ?1", params![op_id])?;
        if let Some(path) = payload {
            tx.execute(
                "INSERT INTO staged_payloads(path, completed) VALUES (?1, 1)
                 ON CONFLICT(path) DO UPDATE SET completed = 1",
                params![path],
            )?;
        }
        delete_file_conn(&tx, &file_id)?;
        tx.commit()?;
        Ok(CancelNew::Ready { file_id })
    }
}

fn decide_conn(conn: &Connection, op_id: &str) -> Result<CancelNew> {
    let row: Option<(String, Option<String>, Option<String>, Option<String>, Option<String>)> = conn
        .query_row(
            &format!(
                "SELECT kind, metadata_json, origin, claim_id, file_id FROM operation_queue
                 WHERE op_id = ?1 AND {UNSENT_KINDS_SQL}"
            ),
            params![op_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .optional()?;
    let Some((kind, metadata, origin, claim, file_id)) = row else {
        return Ok(CancelNew::Unknown);
    };
    if origin.as_deref() == Some("backup") || super::unsent::change_kind(&kind, metadata.as_deref()) != "new_file" {
        return Ok(CancelNew::NotNew);
    }
    let Some(file_id) = file_id else {
        return Ok(CancelNew::NotNew);
    };
    if claim.is_some() {
        return Ok(CancelNew::Busy);
    }
    let others: i64 = conn.query_row(
        "SELECT COUNT(*) FROM operation_queue WHERE file_id = ?1 AND op_id <> ?2",
        params![file_id, op_id],
        |row| row.get(0),
    )?;
    Ok(if others > 0 {
        CancelNew::Busy
    } else {
        CancelNew::Ready { file_id }
    })
}
