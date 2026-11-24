//! What the Uploads panel needs beyond the list: the sizes of the files it names, and the cancel of an upload of a
//! NEW file (one never in Beebeeb): its queued change and its row go, and its copy goes to the Recycle Bin, in ONE
//! write transaction ([`StateDb::cancel_new_upload`]). Nothing is sent. Task: bind-all, files-uploads; review of
//! desktop#423, P1 #2.

use super::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// What the decision of [`StateDb::cancel_new_upload`] says of a change.
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

    /// Cancel the upload of a NEW file, in ONE write transaction (the lock the engine's own claim of a change takes,
    /// `BEGIN IMMEDIATE`): the change is decided cancellable, its change, its resume row and the file's row are
    /// dropped, its staged copy is released to the journal, THEN `recycle` moves the file at the path `path_of`
    /// makes of the row's path; the transaction is committed only when the move worked. A refused move rolls it all
    /// back: every row is as it was and the change is still queued. No claim can come between the decision and the
    /// move, since a claim waits for this transaction and then finds the change gone.
    ///
    /// `recycle` runs while the write lock is held: it must be one move of one file, and must not touch this
    /// database. A commit that fails after the move leaves the change queued for a file in the Recycle Bin (the
    /// upload then stops on a missing file; nothing is lost).
    pub fn cancel_new_upload(
        &self,
        op_id: &str,
        path_of: impl FnOnce(&str) -> Option<PathBuf>,
        recycle: impl FnOnce(&Path) -> std::io::Result<()>,
    ) -> Result<CancelDone> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let file_id = match decide_conn(&tx, op_id)? {
            CancelNew::Ready { file_id } => file_id,
            left => return Ok(CancelDone::Left(left)),
        };
        let rel: Option<String> = tx
            .query_row("SELECT path FROM files WHERE file_id = ?1", params![file_id], |row| {
                row.get(0)
            })
            .optional()?;
        let Some(path) = rel.as_deref().and_then(path_of) else {
            return Ok(CancelDone::NoPath);
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
        if recycle(&path).is_err() {
            tx.rollback()?;
            return Ok(CancelDone::NotMoved);
        }
        tx.commit()?;
        Ok(CancelDone::Cancelled)
    }
}

/// What [`StateDb::cancel_new_upload`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CancelDone {
    /// The copy was moved, and the change, its resume row and the file's row are gone: committed together.
    Cancelled,
    /// Not cancellable ([`CancelNew::Unknown`], [`CancelNew::NotNew`] or [`CancelNew::Busy`]): nothing was asked of
    /// the file, nothing was changed.
    Left(CancelNew),
    /// The file's row names no path that can be moved: nothing was changed.
    NoPath,
    /// The move was refused: rolled back, every row is as it was.
    NotMoved,
}

fn decide_conn(conn: &Connection, op_id: &str) -> Result<CancelNew> {
    type OpRow = (String, Option<String>, Option<String>, Option<String>, Option<String>);
    let row: Option<OpRow> = conn
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
