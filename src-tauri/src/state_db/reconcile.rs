//! `reconcile` items split out of `state_db.rs`.

use super::*;

/// Tasks 1888 + 2226 (spec §3): the rows and ops the reconcile pass plans over, read in one transaction. Rows in rowid
/// order, so folders at the same depth are decided in the order they were written; ops in the queue's order.
pub(super) fn reconcile_plan_conn(
    conn: &Connection,
    opts: &crate::held_queue::Options,
    disk_has_file: &dyn Fn(&str) -> bool,
) -> Result<(Vec<crate::held_queue::Unit>, crate::held_queue::Counts)> {
    use crate::held_queue::{Op, Row};
    let rows = conn
        .prepare(
            "SELECT file_id, NULLIF(parent_id, ''), path, item_kind = 'folder',
                    current_version > 0 OR remote_updated_at > 0, current_version
             FROM files ORDER BY rowid",
        )?
        .query_map([], |row| {
            Ok(Row {
                file_id: row.get(0)?,
                parent_id: row.get(1)?,
                path: row.get(2)?,
                is_folder: row.get(3)?,
                server_known: row.get(4)?,
                current_version: row.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>>>()?;
    let ops = conn
        .prepare(&format!(
            "SELECT op_id, kind, NULLIF(file_id, ''), NULLIF(parent_id, ''), target_path,
                    kind IN ('upload_version', 'upload_file') AND COALESCE(CASE WHEN json_valid(metadata_json)
                        THEN json_extract(metadata_json, '$.operation') = 'create_file' END, 0),
                    paused_reason,
                    attempts >= max_attempts AND paused_reason IS NULL AND last_error_class IS NULL,
                    last_error_class IS NOT NULL, last_error, payload_path, base_version
             FROM operation_queue {DUE_ORDER_SQL}"
        ))?
        .query_map([], |row| {
            Ok(Op {
                op_id: row.get(0)?,
                kind: row.get(1)?,
                file_id: row.get(2)?,
                parent_id: row.get(3)?,
                target_path: row.get(4)?,
                create_file: row.get(5)?,
                paused_reason: row.get(6)?,
                given_up: row.get(7)?,
                parked: row.get(8)?,
                last_error: row.get(9)?,
                payload_path: row.get(10)?,
                base_version: row.get(11)?,
            })
        })?
        .collect::<Result<Vec<_>>>()?;
    Ok(crate::held_queue::plan(&rows, &ops, opts, &|path| Path::new(path).exists(), disk_has_file))
}

/// A remap's statements, once per remapped folder: each reads through a `parent_id` index (`idx_files_parent_id`,
/// `idx_operation_queue_parent`), never a scan of the whole table (measured on a 43k-row copy: 5.5 s per 256 remaps
/// without them, 0.25 s with).
pub(super) const REMAP_FILES_SQL: &str = "UPDATE files SET parent_id = ?2 WHERE parent_id = ?1";

pub(super) const REMAP_OPS_SQL: &str = "UPDATE operation_queue SET parent_id = ?2, updated_at = ?3 WHERE parent_id = ?1";

/// I1: how many rows and ops still name the ghost.
pub(super) const GHOST_REFERENCES_SQL: &str = "SELECT (SELECT COUNT(*) FROM files WHERE file_id = ?1 OR parent_id = ?1)
      + (SELECT COUNT(*) FROM operation_queue WHERE file_id = ?1 OR parent_id = ?1)";

/// Spec D8: a fresh retry budget, due from now. Never touches `created_at` or `rowid` (the queue's order). A change
/// held for the person keeps its pause (D8 amendment). Any other goes under `auth` (Goal 2, I6; cross-lane review
/// P2-2): its name was sealed under the key in force when it was queued, so the key check that follows every pass
/// (D11, D12) resumes it under this session's key or holds it as `key_replaced`, as every other `auth` change.
pub(super) fn rearm_operation_conn(conn: &Connection, op_id: &str, now: i64) -> Result<()> {
    conn.execute(
        "UPDATE operation_queue
         SET attempts = 0, next_retry_at = ?2, last_error = NULL, updated_at = ?2, paused_reason = COALESCE(paused_reason, ?3)
         WHERE op_id = ?1",
        params![op_id, now, OperationPauseReason::Auth.as_str()],
    )?;
    Ok(())
}

/// One whole unit of the reconcile plan, inside the caller's transaction.
pub(super) fn apply_reconcile_unit_conn(conn: &Connection, unit: &crate::held_queue::Unit, now: i64) -> Result<()> {
    use crate::held_queue::Unit;
    let needs_review = OperationPauseReason::NeedsReview.as_str();
    let hold = |op_id: &str| {
        conn.execute(
            "UPDATE operation_queue SET paused_reason = ?2, updated_at = ?3 WHERE op_id = ?1",
            params![op_id, needs_review, now],
        )
    };
    match unit {
        Unit::DropLanded { op_id } => {
            conn.execute("DELETE FROM operation_queue WHERE op_id = ?1", params![op_id])?;
            drop_orphaned_upload_resumes(conn)?;
        }
        Unit::Remap { ghost, twin, ghost_op, rearm, hold: superseded } => {
            conn.execute(REMAP_FILES_SQL, params![ghost, twin])?;
            conn.execute(REMAP_OPS_SQL, params![ghost, twin, now])?;
            for op_id in rearm {
                rearm_operation_conn(conn, op_id, now)?;
            }
            for op_id in superseded {
                hold(op_id)?;
            }
            conn.execute("DELETE FROM operation_queue WHERE op_id = ?1", params![ghost_op])?;
            conn.execute("DELETE FROM files WHERE file_id = ?1", params![ghost])?;
            // I1: nothing names the ghost any more, or the whole unit rolls back with its transaction.
            let left: i64 = conn.query_row(GHOST_REFERENCES_SQL, params![ghost], |row| row.get(0))?;
            if left != 0 {
                return Err(rusqlite::Error::StatementChangedRows(left as usize));
            }
        }
        Unit::Hold { op_id } => {
            hold(op_id)?;
        }
        Unit::Release { op_id } => {
            // Back to `auth`, never straight to due (Goal 2, I6): the key check that follows every pass (D11, D12)
            // resumes it under this session's key or holds it as `key_replaced`, as every other `auth` change.
            conn.execute(
                "UPDATE operation_queue SET paused_reason = ?3, next_retry_at = ?2, updated_at = ?2
                 WHERE op_id = ?1 AND paused_reason = ?4",
                params![op_id, now, OperationPauseReason::Auth.as_str(), needs_review],
            )?;
        }
        Unit::Rearm { op_id } => rearm_operation_conn(conn, op_id, now)?,
        Unit::ResolveParent { op_id, file_id, parent } => {
            // D6: the op and its row both get the folder found by path, by primary key. It comes before any remap, so
            // a ghost's remap re-points what was just put under it.
            conn.execute(
                "UPDATE operation_queue SET parent_id = ?2, updated_at = ?3 WHERE op_id = ?1",
                params![op_id, parent, now],
            )?;
            if let Some(file_id) = file_id {
                conn.execute("UPDATE files SET parent_id = ?2 WHERE file_id = ?1", params![file_id, parent])?;
            }
        }
        Unit::DropScaffold { op_ids, file_ids, .. } => {
            // D5: the scaffolding's creates and rows go; its directories are the caller's, after the commit.
            for op_id in op_ids {
                conn.execute("DELETE FROM operation_queue WHERE op_id = ?1", params![op_id])?;
            }
            for file_id in file_ids {
                conn.execute("DELETE FROM files WHERE file_id = ?1", params![file_id])?;
            }
            drop_orphaned_upload_resumes(conn)?;
            // I1: nothing names a dropped row any more, or the whole unit rolls back with its transaction. One scan for
            // the whole subtree (`parent_id` has no index).
            let ids = serde_json::to_string(file_ids).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            let left: i64 = conn.query_row(
                "SELECT (SELECT COUNT(*) FROM files
                         WHERE file_id IN (SELECT value FROM json_each(?1)) OR parent_id IN (SELECT value FROM json_each(?1)))
                      + (SELECT COUNT(*) FROM operation_queue
                         WHERE file_id IN (SELECT value FROM json_each(?1)) OR parent_id IN (SELECT value FROM json_each(?1)))",
                params![ids],
                |row| row.get(0),
            )?;
            if left != 0 {
                return Err(rusqlite::Error::StatementChangedRows(left as usize));
            }
        }
    }
    Ok(())
}

/// Drop every persisted upload session whose queued op no longer exists.
/// Called by the bulk `operation_queue` purges so a session never outlives
/// its op (`finish_claimed` already clears its own row).
pub(super) fn drop_orphaned_upload_resumes(conn: &Connection) -> Result<usize> {
    // LD-0113 (review 2026-11-07): a backup op's session (its own table, no path) outlives its op until the runner
    // abandons it on the server (`EngineBridge::abandon_orphaned_backup_sessions`): a session dropped here unabandoned
    // keeps the server file "uploading" and refuses its next version until the lease sweep.
    conn.execute(
        "DELETE FROM upload_resume WHERE op_id NOT IN (SELECT op_id FROM operation_queue)",
        [],
    )
}

/// Tasks 1888 + 2226 (spec I4): whole units per reconcile transaction. Measured on a copy of the maintainer's queue
/// (plan Task 1 Step 7, 29 Oct: 2,828 remaps, 60 re-arms; Windows, test build, `parent_id` indexed): the longest
/// transaction took 300 ms at 256 units, 293 ms at 128 and 278 ms at 64. Each round re-plans the whole queue, and that
/// fixed cost is most of a transaction, so smaller batches barely shorten the lock and multiply the re-plans
/// (2.3 s in all at 256, 7.1 s at 64).
pub const RECONCILE_UNITS_PER_TX: usize = 256;

/// `base_pending` is 1 + the successful snapshots that did not report the file (plan Spec
/// issue 3). The op parks when the count would pass 10 (spec §6.3.3).
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(super) const BASE_PENDING_PARK_AT: i64 = 11;
