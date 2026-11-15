//! Task 2530 (wave 2, slice S3): the ways out of a parked upload that This PC's Sync history offers, beyond the stale
//! base the conflict window decides ([`DECIDABLE_PARKED_SQL`], unchanged). One closed list per way out:
//! - Keep the other (`EngineBridge::resolve_parked` `Remote`): `predecessor_lost`, `permission_denied`.
//! - Keep both (`resolve_parked` `Both`): `predecessor_lost`.
//! - Try again: `rekey_failed`, made due now with its base and its bytes as they are.
//! - Remove from the list: `payload_missing`, the one discard, only while no bytes of the change are on this PC: its
//!   staged copy is gone, and the file's place is not the change (it is on macOS, where a File Provider write's place
//!   is the system's own copy of it; on Windows and Linux a write-keyed save is the viewer's, from a staged copy only:
//!   `engine_bridge::upload::landing_wrote_the_place`).
//!
//! Each way out keeps every guard of the stale-base predicate but one: the file need not be past the op's base (a
//! refused write sits at its base). The op is a content write with a write id and a base (a modify: a create has no
//! version to keep the other of), parked (attempts used up, its class recorded, not paused), unclaimed, with no
//! completion recorded, nothing queued after it for the file, and neither it nor a folder above it being trashed or
//! moved. Every list read and every choice uses these predicates, and every choice checks again inside the
//! transaction that acts on it.

use super::*;

/// The parked ops Keep the other may settle beyond the stale base, as the alias `this`. The guards are
/// [`DECIDABLE_PARKED_SQL`]'s but the version past the base; the classes are a closed list.
pub(super) const DECIDABLE_PARKED_ANY_SQL: &str = "this.kind = 'upload_version' AND this.write_id IS NOT NULL
    AND this.last_error_class IN ('predecessor_lost', 'permission_denied') AND this.attempts >= this.max_attempts
    AND this.paused_reason IS NULL AND this.claim_id IS NULL AND this.base_version IS NOT NULL
    AND this.payload_path IS NOT NULL
    AND NOT EXISTS (SELECT 1 FROM upload_resume AS resume
                    WHERE resume.op_id = this.op_id AND resume.completed_version IS NOT NULL)
    AND NOT EXISTS (SELECT 1 FROM operation_queue AS later
                    WHERE later.file_id = this.file_id AND later.op_id != this.op_id AND later.rowid > this.rowid)
    AND EXISTS (SELECT 1 FROM files AS file WHERE file.file_id = this.file_id AND file.status != 'trashing')
    AND NOT EXISTS (
        WITH RECURSIVE above(id, depth) AS (
            SELECT parent_id, 1 FROM files WHERE file_id = this.file_id
            UNION ALL
            SELECT folder.parent_id, above.depth + 1 FROM files AS folder JOIN above ON folder.file_id = above.id
             WHERE above.depth < 512)
        SELECT 1 FROM above
         WHERE above.depth >= 512
            OR EXISTS (SELECT 1 FROM files AS folder WHERE folder.file_id = above.id AND folder.status = 'trashing')
            OR EXISTS (SELECT 1 FROM operation_queue AS moving
                       WHERE moving.file_id = above.id AND moving.kind IN ('trash_file', 'move_file', 'rename_file')))";

/// Keep both takes fewer classes than Keep the other: a write the server refused (`permission_denied`) would be
/// refused again as a new file in the same folder.
const KEEP_BOTH_ONLY_SQL: &str = "this.last_error_class = 'predecessor_lost'";

/// A `rekey_failed` park Try again may make due again, as the alias `this`.
const TRY_AGAIN_PARKED_SQL: &str = "this.last_error_class = 'rekey_failed' AND this.attempts >= this.max_attempts
    AND this.paused_reason IS NULL AND this.claim_id IS NULL
    AND NOT EXISTS (SELECT 1 FROM upload_resume AS resume
                    WHERE resume.op_id = this.op_id AND resume.completed_version IS NOT NULL)";

/// A `payload_missing` park Remove from the list may retire, as the alias `this` (its staged copy is checked apart, in
/// the same transaction): a modify with a write id, parked, unclaimed, no completion recorded, nothing queued after it.
const REMOVABLE_PARKED_SQL: &str = "this.kind = 'upload_version' AND this.write_id IS NOT NULL
    AND this.last_error_class = 'payload_missing' AND this.attempts >= this.max_attempts
    AND this.paused_reason IS NULL AND this.claim_id IS NULL AND this.base_version IS NOT NULL
    AND this.payload_path IS NOT NULL
    AND NOT EXISTS (SELECT 1 FROM upload_resume AS resume
                    WHERE resume.op_id = this.op_id AND resume.completed_version IS NOT NULL)
    AND NOT EXISTS (SELECT 1 FROM operation_queue AS later
                    WHERE later.file_id = this.file_id AND later.op_id != this.op_id AND later.rowid > this.rowid)";

/// macOS: a File Provider write's place is the system's own copy of the change, so a missing staged copy never means
/// the change's bytes are gone (`engine_bridge::upload::landing_wrote_the_place`). Remove from the list is never offered
/// there; that row is the macOS phase's.
const PLACE_HOLDS_THE_CHANGE: bool = cfg!(target_os = "macos");

/// A choice that settles a parked write by keeping this PC's bytes, beyond "Keep this PC's" (stale base only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkedSettle {
    /// Keep the other: this PC's bytes are kept first, then the write goes.
    KeepOther,
    /// Keep both: this PC's bytes are queued as a new file and the write goes, in one transaction.
    KeepBoth,
}

/// A way out a parked or held change's row may offer (This PC's Sync history › Problems).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParkExit {
    TryAgain,
    KeepBoth,
    KeepOther,
    KeepThisPcs,
    RemoveFromList,
}

/// What Remove from the list did ([`StateDb::remove_parked_without_bytes`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoveParked {
    /// The write is gone from the list; the file on Beebeeb is as it was.
    Removed,
    /// Bytes of the change are on this PC (its staged copy is there again, or the file's place is the change): nothing
    /// changed.
    BytesHere,
    /// It is no longer such a write (sent, settled, claimed, or another change queued after it): nothing changed.
    Moved,
}

fn exists_where(conn: &Connection, op_id: &str, predicate: &str, tail: &str) -> Result<bool> {
    conn.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM operation_queue AS this WHERE this.op_id = ?1 AND {predicate}{tail})"),
        params![op_id],
        |row| row.get(0),
    )
}

/// `settle` may take `op_id` beyond the stale base, whatever version its file is at.
fn widened_conn(conn: &Connection, op_id: &str, settle: ParkedSettle) -> Result<bool> {
    let extra = match settle {
        ParkedSettle::KeepOther => String::new(),
        ParkedSettle::KeepBoth => format!(" AND {KEEP_BOTH_ONLY_SQL}"),
    };
    exists_where(conn, op_id, DECIDABLE_PARKED_ANY_SQL, &extra)
}

/// `settle` may take `op_id` beyond the stale base, and its file is still at `shown`, the version the person was shown.
fn widened_at_conn(conn: &Connection, op_id: &str, shown: i64, settle: ParkedSettle) -> Result<bool> {
    let extra = match settle {
        ParkedSettle::KeepOther => String::new(),
        ParkedSettle::KeepBoth => format!(" AND {KEEP_BOTH_ONLY_SQL}"),
    };
    conn.query_row(
        &format!(
            "SELECT EXISTS(SELECT 1 FROM operation_queue AS this WHERE this.op_id = ?1 AND {DECIDABLE_PARKED_ANY_SQL}{extra}
               AND EXISTS (SELECT 1 FROM files AS shown WHERE shown.file_id = this.file_id AND shown.current_version = ?2))"
        ),
        params![op_id, shown],
        |row| row.get(0),
    )
}

/// THE check of every Keep the other and Keep both, inside the transaction that acts: the stale-base predicate as it
/// is, or `settle`'s own closed list; and the file still at `shown`.
pub(super) fn settleable_at_conn(conn: &Connection, op_id: &str, shown: i64, settle: ParkedSettle) -> Result<bool> {
    Ok(decidable_parked_at_conn(conn, op_id, shown)? || widened_at_conn(conn, op_id, shown, settle)?)
}

/// The staged copy of the change is on this PC (unreadable counts as there: never discard on a doubt).
fn staged_copy_here(path: &str) -> bool {
    !matches!(std::fs::symlink_metadata(path), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
}

/// The ways out `op_id`'s row offers now, by the predicates every choice checks again. A row with none says why in
/// words. Read in the list's own transaction.
pub(super) fn park_exits_conn(conn: &Connection, op_id: &str) -> Result<Vec<ParkExit>> {
    if decidable_parked_conn(conn, op_id)? {
        return Ok(vec![ParkExit::KeepBoth, ParkExit::KeepOther, ParkExit::KeepThisPcs]);
    }
    let mut exits = Vec::new();
    if widened_conn(conn, op_id, ParkedSettle::KeepBoth)? {
        exits.push(ParkExit::KeepBoth);
    }
    if widened_conn(conn, op_id, ParkedSettle::KeepOther)? {
        exits.push(ParkExit::KeepOther);
    }
    if exists_where(conn, op_id, TRY_AGAIN_PARKED_SQL, "")? {
        exits.push(ParkExit::TryAgain);
    }
    if !PLACE_HOLDS_THE_CHANGE && exists_where(conn, op_id, REMOVABLE_PARKED_SQL, "")? {
        let path: Option<String> = conn.query_row(
            "SELECT payload_path FROM operation_queue WHERE op_id = ?1",
            params![op_id],
            |row| row.get(0),
        )?;
        if path.as_deref().is_some_and(|path| !staged_copy_here(path)) {
            exits.push(ParkExit::RemoveFromList);
        }
    }
    Ok(exits)
}

impl StateDb {
    /// Keep the other or Keep both may settle `op_id` (stale base, or `settle`'s closed list), its file still at `shown`.
    pub fn is_settleable_parked_at(&self, op_id: &str, shown: i64, settle: ParkedSettle) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        settleable_at_conn(&conn, op_id, shown, settle)
    }

    /// The ways out `op_id`'s row offers now ([`park_exits_conn`]).
    pub fn park_exits(&self, op_id: &str) -> Result<Vec<ParkExit>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        park_exits_conn(&conn, op_id)
    }

    /// Try again on a `rekey_failed` park, in one transaction: made due now with its attempts back to 0 and its error
    /// cleared, as `rebase_parked` does, but its base, its bytes and its metadata stay as they are; a write-keyed file
    /// that presents `error` presents `uploading` again. `false`: it is no longer such a park, and nothing changed.
    pub fn try_parked_again(&self, op_id: &str, now: i64) -> Result<bool> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if !exists_where(&tx, op_id, TRY_AGAIN_PARKED_SQL, "")? {
            return Ok(false);
        }
        let (file_id, write_keyed): (Option<String>, bool) = tx.query_row(
            "SELECT file_id, write_id IS NOT NULL AND kind IN ('upload_version', 'upload_file')
             FROM operation_queue WHERE op_id = ?1",
            params![op_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        tx.execute(
            "UPDATE operation_queue
             SET attempts = 0, last_error = NULL, last_error_class = NULL, next_retry_at = ?2, updated_at = ?2
             WHERE op_id = ?1 AND claim_id IS NULL",
            params![op_id, now],
        )?;
        if let (true, Some(file_id)) = (write_keyed, file_id.as_deref())
            && tx.execute(
                "UPDATE files SET status = 'uploading' WHERE file_id = ?1 AND status = 'error'",
                params![file_id],
            )? == 1
        {
            record_file_change_conn(&tx, file_id, FpChangeKind::Modified, None)?;
        }
        tx.commit()?;
        Ok(true)
    }

    /// Remove from the list, the one discard, in one transaction: a `payload_missing` park whose staged copy is still
    /// gone is retired as Keep the other retires a write ([`retire_parked_conn`]: the op, its session row and the
    /// file's held write go, the file presents `local` and reads the server's version). Only the queue row goes: no
    /// bytes of the change are on this PC to lose. The staged copy is looked at again inside the transaction.
    pub fn remove_parked_without_bytes(&self, op_id: &str) -> Result<RemoveParked> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if !exists_where(&tx, op_id, REMOVABLE_PARKED_SQL, "")? {
            return Ok(RemoveParked::Moved);
        }
        let path: String = tx.query_row(
            "SELECT payload_path FROM operation_queue WHERE op_id = ?1",
            params![op_id],
            |row| row.get(0),
        )?;
        if PLACE_HOLDS_THE_CHANGE || staged_copy_here(&path) {
            return Ok(RemoveParked::BytesHere);
        }
        retire_parked_conn(&tx, op_id)?;
        tx.commit()?;
        Ok(RemoveParked::Removed)
    }
}
