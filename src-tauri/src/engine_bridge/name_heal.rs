//! Task 2446, the heal: the one snapshot that reads names again in a state database an earlier build left with
//! blocked rows. When it is due, what a failed one costs (a wait, never the tick), and the rows a pass leaves as
//! they are stored. The tick (`syncflow`) calls in here; `state_db/custody.rs` keeps the marks.

use super::*;

/// What one snapshot did: the ids it applied or pruned, and how many rows a healing one left as they were.
pub(super) struct SnapshotPass {
    pub(super) applied: Vec<String>,
    pub(super) rows_left: usize,
}

/// The rows a healing snapshot leaves exactly as they are stored: a row whose write failed and every row under it
/// (its place depends on a row that was not stored), in `ids`; and the rows that were read but keep their blocked
/// label until a change of this PC is done with them (`waits_for_its_change`), counted in `waiting` (the rows under
/// one of those are applied as usual). Each was listed by the snapshot, so each is seen: the prune never takes one
/// for a row the server no longer has.
#[derive(Default)]
pub(super) struct LeftRows {
    pub(super) ids: HashSet<String>,
    pub(super) waiting: usize,
}

impl LeftRows {
    /// Leaves `file_id` as it is stored. Zero-knowledge: the id, never a name or an error's text.
    pub(super) fn leave(&mut self, file_id: &str) {
        tracing::debug!(file_id = %file_id, "sync_tick: a row of the healing snapshot was not stored; left as it is");
        self.ids.insert(file_id.to_string());
    }

    /// How many rows the pass left, logged as a count when any.
    pub(super) fn count(&self) -> usize {
        let rows_left = self.ids.len() + self.waiting;
        if rows_left > 0 {
            tracing::warn!(
                rows_left,
                "sync_tick: the snapshot that reads names again left rows as they were"
            );
        }
        rows_left
    }
}

/// Whether this tick takes the snapshot that reads names again. Not once it is done; done at once when no row needs
/// it; otherwise only when the wait after the last one that failed, or left rows as they were, is over.
pub(super) fn name_heal_is_due(db: &StateDb, now_secs: i64) -> Result<bool, EngineError> {
    if db.name_heal_done()? {
        return Ok(false);
    }
    let retry = db.name_heal_retry()?;
    let rows_left = retry.is_some_and(|retry| retry.rows_left > 0);
    if !rows_left && !db.has_blocked_or_nameless_rows()? {
        db.mark_name_heal_done()?;
        return Ok(false);
    }
    let due = retry.is_none_or(|retry| retry.is_due(now_secs));
    if due {
        tracing::info!("sync_tick: rows without a name; taking one snapshot to read them again");
    }
    Ok(due)
}

/// A healing snapshot was applied. With no row left as it was the heal is done; otherwise it stays owed and is
/// asked for again after the wait.
pub(super) fn settle_name_heal(db: &StateDb, now_secs: i64, rows_left: usize) -> Result<(), EngineError> {
    if rows_left == 0 {
        db.mark_name_heal_done()?;
    } else {
        db.note_name_heal_failed(now_secs, Some(u32::try_from(rows_left).unwrap_or(u32::MAX)))?;
    }
    Ok(())
}

/// A healing snapshot failed as a whole: the wait before the next one starts, or grows. Another account's session
/// ends the tick here, as it does at every other step.
pub(super) fn name_heal_snapshot_failed(
    bridge: &EngineBridge,
    now_secs: i64,
    error: &anyhow::Error,
) -> anyhow::Result<()> {
    bridge.owner_key_scope().check()?;
    let retry = bridge.db().note_name_heal_failed(now_secs, None)?;
    tracing::warn!(
        failures = retry.failures,
        wait_secs = retry.next_at.saturating_sub(now_secs),
        reason = error_category(error.as_ref()),
        "sync_tick: the snapshot that reads names again failed; asked again after the wait"
    );
    Ok(())
}

/// Stores one snapshot row (`process_metadata_row`). In a test build a test can make the write of a row fail.
pub(super) fn store_snapshot_row(
    bridge: &EngineBridge,
    row: &serde_json::Value,
    parent_rel_path: &str,
    now_secs: i64,
    conflicts: &mut Vec<ConflictDetected>,
) -> Result<Option<(String, ItemKind)>, EngineError> {
    #[cfg(test)]
    if let Some(fault) = super::tests::snapshot_row_fault(row["id"].as_str().unwrap_or_default()) {
        return Err(fault);
    }
    process_metadata_row(bridge, row, parent_rel_path, now_secs, RowSource::Snapshot, conflicts)
}
