//! Task 2409: the changes on this computer that are not sent yet, one row each: the list behind the "Not sent" count.
//!
//! On Guus's PC Home said "Not sent 2" and the tray "2 changes waiting to sync", and no screen named them: the count
//! read the queue ([`StateDb::queue_hold_summary`]) and the one per-item list, the review feed, read other rows and was
//! never shown on Windows. This read selects exactly the rows the count counts, through the same predicates
//! ([`UNSENT_KINDS_SQL`], the pause and attempt columns, `HELD_FOLDER_CTE`, the finalizations held for review), and puts
//! each in the group the count puts it in; `state_db::tests::unsent` pins count == list for every kind and reason.
//!
//! Unlike the summary it carries names (the path of each change), so the command that serves it answers only while the
//! vault is unlocked, and the window reads it only while the list is on screen. It changes nothing.

use super::*;

/// The queue rows "Not sent" counts: every operation but a download (task 2224). A `hydrate_file` or `pin_tree` brings
/// bytes to this computer; nothing of the person's is waiting to leave it. Unqualified on purpose: every query that uses
/// it reads `operation_queue` alongside tables without a `kind` column (`files`, the `held_folder` CTE).
pub(super) const UNSENT_KINDS_SQL: &str = "kind NOT IN ('hydrate_file', 'pin_tree')";

/// One change that is not sent yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnsentChange {
    /// The operation's id, or `finalization:<op id>` for an upload this PC could not mark as synced.
    pub id: String,
    pub file_id: Option<String>,
    /// What the change is: `new_file`, `edit`, `folder`, `rename`, `move`, `delete` or `restore`.
    pub kind: &'static str,
    /// Where it is: relative to the sync folder, or, for a backup, the backed-up file's own path on this computer.
    pub path: String,
    /// A backup of a folder on this computer (LD-0113), not a change in the sync folder.
    pub backup: bool,
    /// Why it is not sent, as the summary groups it: `waiting`, `waiting_for_folder`, `gave_up`, `not_marked_synced`, or
    /// the pause reason (`PAUSE_REASON_LABELS`, anything else as `other`).
    pub group: String,
    /// Unix seconds it was queued (for `not_marked_synced`, when it was held for review; 0 = not recorded).
    pub queued_at: i64,
    /// A waiting change that failed and waits out its backoff: when it is tried again (unix seconds).
    pub retry_at: Option<i64>,
    /// Given up because it was parked (a stale base, a missing copy, a refused write: [`PARK_REASONS`]), not because
    /// it failed again and again. Trying it again sends nothing new, so it has no Try again.
    pub parked: bool,
    /// Try again applies (Guus, 13:39): it stopped after repeated errors, or it waits out a backoff after an error.
    /// Never a held row (it keeps its own action) or a parked one.
    pub retryable: bool,
}

/// What [`StateDb::retry_unsent_change`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryOutcome {
    /// Its attempts and next-attempt time were reset: the next queue step sends it.
    Retried,
    /// It was already due, or is being sent right now: nothing to change (a second Try again lands here).
    AlreadyQueued,
    /// No such change in the queue (sent meanwhile, or never there), or a download.
    Unknown,
    /// Paused for a reason (signed out, storage full, ...): it keeps its own action.
    Held,
    /// Parked: trying it again sends nothing new.
    Parked,
}

/// Every reason an upload parks with its bytes kept (`park_claimed`, `park_unclaimed_conn`, the rekey landing):
/// `last_error_class` holds one of these on a parked row, and NULL on a row that ran out of attempts by failing.
const PARK_REASONS: [ParkReason; 7] = [
    ParkReason::StaleBase,
    ParkReason::BaseUnknown,
    ParkReason::PayloadMissing,
    ParkReason::PredecessorParked,
    ParkReason::PredecessorLost,
    ParkReason::RekeyFailed,
    ParkReason::PermissionDenied,
];

/// Exhaustive on purpose: a new park reason fails to compile here until it is added to [`PARK_REASONS`].
#[allow(dead_code)]
fn park_reasons_are_all_listed(reason: ParkReason) -> bool {
    match reason {
        ParkReason::StaleBase
        | ParkReason::BaseUnknown
        | ParkReason::PayloadMissing
        | ParkReason::PredecessorParked
        | ParkReason::PredecessorLost
        | ParkReason::RekeyFailed
        | ParkReason::PermissionDenied => PARK_REASONS.contains(&reason),
    }
}

fn is_park_class(class: Option<&str>) -> bool {
    class.is_some_and(|class| PARK_REASONS.iter().any(|reason| reason.as_str() == class))
}

/// One page of [`UnsentChange`]s and how many there are in all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnsentPage {
    pub total: i64,
    pub items: Vec<UnsentChange>,
}

/// The rows, one per change, as `unsent(id, file_id, kind, metadata_json, origin, path, grp, queued_at, attempts,
/// next_retry_at, rank)`. `grp` is `paused:<reason>` for a paused row (the label is applied in Rust, as the summary
/// applies it). `rank` orders what needs the person first: held and given up, then waiting for a folder, then waiting.
#[cfg(not(target_os = "macos"))]
fn unsent_sql(tail: &str) -> String {
    unsent_sql_with(
        &format!("WITH RECURSIVE {HELD_FOLDER_CTE},"),
        "WHEN q.parent_id IN (SELECT file_id FROM held_folder) THEN 'waiting_for_folder'",
        tail,
    )
}

/// macOS has no parent-first gate (spec D7), so no change waits for its folder there.
#[cfg(target_os = "macos")]
fn unsent_sql(tail: &str) -> String {
    unsent_sql_with("WITH", "", tail)
}

fn unsent_sql_with(with: &str, held_folder_case: &str, tail: &str) -> String {
    format!(
        "{with} unsent(id, file_id, kind, metadata_json, origin, path, grp, queued_at, attempts, next_retry_at,
                      error_class) AS (
             SELECT q.op_id, q.file_id, q.kind, q.metadata_json, q.origin,
                    COALESCE(q.source_path, f.path, q.target_path, ''),
                    CASE WHEN q.paused_reason IS NOT NULL THEN 'paused:' || q.paused_reason
                         WHEN q.attempts >= q.max_attempts THEN 'gave_up'
                         {held_folder_case}
                         ELSE 'waiting' END,
                    q.created_at, q.attempts, q.next_retry_at, q.last_error_class
             FROM operation_queue q LEFT JOIN files f ON f.file_id = q.file_id
             WHERE {UNSENT_KINDS_SQL}
             UNION ALL
             SELECT 'finalization:' || u.op_id, u.local_file_id, 'upload_file', NULL, 'sync', u.target_path,
                    'not_marked_synced', COALESCE(u.review_at, 0), 0, 0, NULL
             FROM upload_finalizations u WHERE u.review IS NOT NULL
         ) {tail}"
    )
}

/// The kind word's key for one queue row. A `create_file` upload is a new file, whichever kind queued it.
fn change_kind(kind: &str, metadata_json: Option<&str>) -> &'static str {
    match OperationKind::from_str(kind) {
        OperationKind::UploadFile => "new_file",
        OperationKind::UploadVersion => {
            let created = metadata_json
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                .is_some_and(|meta| meta["operation"].as_str() == Some("create_file"));
            if created { "new_file" } else { "edit" }
        }
        OperationKind::CreateFolder => "folder",
        OperationKind::RenameFile => "rename",
        OperationKind::MoveFile => "move",
        OperationKind::TrashFile => "delete",
        OperationKind::RestoreFile | OperationKind::RestoreVersion => "restore",
        // Never selected (`UNSENT_KINDS_SQL`); named so a new kind is a compile error here, not a silent label.
        OperationKind::HydrateFile | OperationKind::PinTree => "new_file",
    }
}

/// The summary's group for a row's `grp` column: a pause reason goes through the same label filter the summary uses.
fn group_label(grp: &str) -> String {
    match grp.strip_prefix("paused:") {
        Some(reason) => allowed_label(reason, PAUSE_REASON_LABELS),
        None => grp.to_string(),
    }
}

impl StateDb {
    /// Task 2409: the changes "Not sent" counts, `limit` of them from `offset`, needing-the-person first, then oldest
    /// first. `now` is unix seconds: a waiting change whose retry is still ahead of it carries `retry_at`. One read
    /// transaction, so the total and the page are the same moment's.
    pub fn unsent_changes(&self, offset: u32, limit: u32, now: i64) -> Result<UnsentPage> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        let total: i64 = tx.query_row(&unsent_sql("SELECT COUNT(*) FROM unsent"), [], |row| row.get(0))?;
        let items = {
            let mut stmt = tx.prepare(&unsent_sql(
                "SELECT id, file_id, kind, metadata_json, origin, path, grp, queued_at, attempts, next_retry_at,
                        error_class
                 FROM unsent
                 ORDER BY CASE grp WHEN 'waiting' THEN 2 WHEN 'waiting_for_folder' THEN 1
                                   WHEN 'paused:waiting_for_folder' THEN 1 ELSE 0 END,
                          queued_at, id
                 LIMIT ?1 OFFSET ?2",
            ))?;
            let items = stmt
                .query_map(params![i64::from(limit), i64::from(offset)], |row| {
                    let kind: String = row.get(2)?;
                    let metadata_json: Option<String> = row.get(3)?;
                    let origin: Option<String> = row.get(4)?;
                    let group = group_label(&row.get::<_, String>(6)?);
                    let attempts: i64 = row.get(8)?;
                    let next_retry_at: i64 = row.get(9)?;
                    let error_class: Option<String> = row.get(10)?;
                    let backing_off = group == "waiting" && attempts > 0 && next_retry_at > now;
                    let parked = group == "gave_up" && is_park_class(error_class.as_deref());
                    let retryable = backing_off || (group == "gave_up" && !parked);
                    Ok(UnsentChange {
                        id: row.get(0)?,
                        file_id: row.get(1)?,
                        kind: change_kind(&kind, metadata_json.as_deref()),
                        path: row.get(5)?,
                        backup: origin.as_deref() == Some("backup"),
                        group,
                        queued_at: row.get(7)?,
                        retry_at: backing_off.then_some(next_retry_at),
                        parked,
                        retryable,
                    })
                })?
                .collect::<Result<Vec<_>>>()?;
            items
        };
        tx.commit()?;
        Ok(UnsentPage { total, items })
    }

    /// Task 2409 (Guus, 13:39, "Add in the try again directly if it failed"): a change that stopped after repeated
    /// errors, or waits out a backoff after one, is made due now with its attempts back to 0, so the next queue step
    /// sends it. Only that row changes, in one transaction. A held, parked, unknown or download row is refused and
    /// nothing changes; a row already due or being sent is left as it is. A Finder upload whose give-up presented its
    /// file as `error` presents it `uploading` again, as a rebase does. The count does not change: the row moves from
    /// given up (or waiting) to waiting.
    pub fn retry_unsent_change(&self, op_id: &str, now: i64) -> Result<RetryOutcome> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        type Row = (
            String,
            Option<String>,
            i64,
            i64,
            i64,
            Option<String>,
            Option<String>,
            Option<String>,
            bool,
        );
        let row: Option<Row> = tx
            .query_row(
                &format!(
                    "SELECT kind, paused_reason, attempts, max_attempts, next_retry_at, last_error_class, claim_id,
                            file_id, write_id IS NOT NULL
                     FROM operation_queue WHERE op_id = ?1 AND {UNSENT_KINDS_SQL}"
                ),
                params![op_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                    ))
                },
            )
            .optional()?;
        let Some((kind, paused, attempts, max_attempts, next_retry_at, class, claim, file_id, has_write)) = row else {
            return Ok(RetryOutcome::Unknown);
        };
        if paused.is_some() {
            return Ok(RetryOutcome::Held);
        }
        let gave_up = attempts >= max_attempts;
        if gave_up && is_park_class(class.as_deref()) {
            return Ok(RetryOutcome::Parked);
        }
        if claim.is_some() || (!gave_up && (attempts == 0 || next_retry_at <= now)) {
            return Ok(RetryOutcome::AlreadyQueued);
        }
        tx.execute(
            "UPDATE operation_queue
             SET attempts = 0, next_retry_at = ?2, last_error = NULL, last_error_class = NULL, updated_at = ?2
             WHERE op_id = ?1 AND paused_reason IS NULL AND claim_id IS NULL",
            params![op_id, now],
        )?;
        let finder_upload = has_write && matches!(kind.as_str(), "upload_version" | "upload_file");
        if let (true, Some(file_id)) = (finder_upload, file_id.as_deref())
            && tx.execute(
                "UPDATE files SET status = 'uploading' WHERE file_id = ?1 AND status = 'error'",
                params![file_id],
            )? == 1
        {
            record_file_change_conn(&tx, file_id, FpChangeKind::Modified, None)?;
        }
        tx.commit()?;
        Ok(RetryOutcome::Retried)
    }
}
