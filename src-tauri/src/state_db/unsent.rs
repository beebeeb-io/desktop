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

/// Task 2414: the downloads a pin queued that will not finish by themselves: paused, or out of attempts. "Not
/// downloaded" counts and lists exactly these ([`StateDb::queue_hold_summary`]'s `not_downloaded`,
/// [`StateDb::not_downloaded`]); a download still due or backing off is on its way and is in neither.
pub(super) const NOT_DOWNLOADED_SQL: &str =
    "kind IN ('hydrate_file', 'pin_tree') AND (paused_reason IS NOT NULL OR attempts >= max_attempts)";

/// One change that is not sent yet ([`StateDb::unsent_changes`]), or one download a pin queued that is not done
/// ([`StateDb::not_downloaded`], task 2414).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnsentChange {
    /// The operation's id, or `finalization:<op id>` for an upload this PC could not mark as synced.
    pub id: String,
    pub file_id: Option<String>,
    /// What the change is: `new_file`, `edit`, `folder`, `rename`, `move`, `delete` or `restore`; a download is
    /// `download` (a file) or `folder` (a folder's pin).
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
    /// Parked (`last_error_class` set: a stale base, a missing copy, a refused write), not failed again and again. The
    /// queue's own rule (spec D8, `held_queue::types::QueueRow::parked`, the reconcile pass): a park is never re-armed,
    /// so it has no Try again.
    pub parked: bool,
    /// Try again applies (Guus, 13:39): it stopped after repeated errors, or it waits out a backoff after an error.
    /// Never a held row (it keeps its own action), a parked one, one being sent, or one [`Self::blocked`].
    pub retryable: bool,
    /// Task 2414: a change that failed which Try again would not send, by spec D8's checks (the reconcile pass's):
    /// `superseded` (a newer version of the file is already in Beebeeb, or the server already holds the new file) or
    /// `copy_gone` (the staged copy to send is gone). [`StateDb::retry_unsent_change`] refuses it for the same reason.
    pub blocked: Option<&'static str>,
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
    /// Task 2414, spec D8: sent, it would make old content current (a newer version is already in Beebeeb).
    Superseded,
    /// Task 2414, spec D8: an upload whose staged copy is gone; sent, it only fails again.
    CopyGone,
}

impl RetryOutcome {
    /// The refusal of a change that failed but cannot be sent as it is ([`UnsentChange::blocked`]'s words).
    fn blocked_by(block: crate::held_queue::PresendBlock) -> Self {
        match block {
            crate::held_queue::PresendBlock::Superseded => RetryOutcome::Superseded,
            crate::held_queue::PresendBlock::NoPayload => RetryOutcome::CopyGone,
        }
    }
}

/// [`UnsentChange::blocked`]'s word for a check that failed.
fn blocked_word(block: crate::held_queue::PresendBlock) -> &'static str {
    match block {
        crate::held_queue::PresendBlock::Superseded => "superseded",
        crate::held_queue::PresendBlock::NoPayload => "copy_gone",
    }
}

/// One page of [`UnsentChange`]s and how many there are in all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnsentPage {
    pub total: i64,
    pub items: Vec<UnsentChange>,
}

/// The rows, one per change, as `unsent(id, file_id, kind, metadata_json, origin, path, grp, queued_at, attempts,
/// next_retry_at, error_class, claimed)`. `grp` is `paused:<reason>` for a paused row (the label is applied in Rust, as the summary
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
                      error_class, claimed) AS (
             SELECT q.op_id, q.file_id, q.kind, q.metadata_json, q.origin,
                    COALESCE(q.source_path, f.path, q.target_path, ''),
                    CASE WHEN q.paused_reason IS NOT NULL THEN 'paused:' || q.paused_reason
                         WHEN q.attempts >= q.max_attempts THEN 'gave_up'
                         {held_folder_case}
                         ELSE 'waiting' END,
                    q.created_at, q.attempts, q.next_retry_at, q.last_error_class, q.claim_id IS NOT NULL
             FROM operation_queue q LEFT JOIN files f ON f.file_id = q.file_id
             WHERE {UNSENT_KINDS_SQL}
             UNION ALL
             -- Always a new file: the landing journals a finalization for a `create_file` upload only
             -- (`defer_local_upload_finalization` in `engine_bridge/upload.rs` returns none for an edit).
             SELECT 'finalization:' || u.op_id, u.local_file_id, 'upload_file', NULL, 'sync', u.target_path,
                    'not_marked_synced', COALESCE(u.review_at, 0), 0, 0, NULL, 0
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
        // Listed by "Not downloaded" only (`NOT_DOWNLOADED_SQL`), never by "Not sent" (`UNSENT_KINDS_SQL`).
        OperationKind::HydrateFile => "download",
        OperationKind::PinTree => "folder",
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
        let mut items = {
            let mut stmt = tx.prepare(&unsent_sql(
                "SELECT id, file_id, kind, metadata_json, origin, path, grp, queued_at, attempts, next_retry_at,
                        error_class, claimed
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
                    let raw_group: String = row.get(6)?;
                    let paused = raw_group.starts_with("paused:");
                    let group = group_label(&raw_group);
                    let attempts: i64 = row.get(8)?;
                    let next_retry_at: i64 = row.get(9)?;
                    let error_class: Option<String> = row.get(10)?;
                    let claimed: bool = row.get(11)?;
                    // Spec D8: any class is a park, as the reconcile pass reads it; a held row keeps its own action.
                    let parked = !paused && error_class.is_some();
                    let backing_off = !parked && group == "waiting" && attempts > 0 && next_retry_at > now;
                    // As `retry_unsent_change` decides: a claimed row is being sent (2409 review P3).
                    let failed = !claimed && (backing_off || (!parked && group == "gave_up"));
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
                        // Until the checks below say otherwise.
                        retryable: failed,
                        blocked: None,
                    })
                })?
                .collect::<Result<Vec<_>>>()?;
            items
        };
        // Task 2414: Try again's own checks (spec D8, the reconcile pass's), so the list never offers it for a change
        // the command would refuse, and says why instead. Only the failed rows on this page, by primary key.
        for item in items.iter_mut().filter(|item| item.retryable) {
            if let Some(block) = presend_block_conn(&tx, &item.id)? {
                item.retryable = false;
                item.blocked = Some(blocked_word(block));
            }
        }
        tx.commit()?;
        Ok(UnsentPage { total, items })
    }

    /// Task 2414: the downloads a pin queued that will not finish by themselves (paused, or out of attempts:
    /// [`NOT_DOWNLOADED_SQL`]), `limit` of them from `offset`, oldest first. What `queue_hold_summary`'s
    /// `not_downloaded` counts, one each; 2409 took them out of "Not sent" (task 2224), and they were counted nowhere.
    /// Each carries its group as "Not sent" words it (`gave_up`, or the pause reason); none has Try again. A read.
    pub fn not_downloaded(&self, offset: u32, limit: u32) -> Result<UnsentPage> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        let total: i64 = tx.query_row(
            &format!("SELECT COUNT(*) FROM operation_queue WHERE {NOT_DOWNLOADED_SQL}"),
            [],
            |row| row.get(0),
        )?;
        let items = {
            let mut stmt = tx.prepare(&format!(
                "SELECT q.op_id, q.file_id, q.kind, COALESCE(f.path, q.target_path, ''),
                        CASE WHEN q.paused_reason IS NOT NULL THEN 'paused:' || q.paused_reason ELSE 'gave_up' END,
                        q.created_at
                 FROM operation_queue q LEFT JOIN files f ON f.file_id = q.file_id
                 WHERE {NOT_DOWNLOADED_SQL}
                 ORDER BY q.created_at, q.op_id
                 LIMIT ?1 OFFSET ?2"
            ))?;
            stmt.query_map(params![i64::from(limit), i64::from(offset)], |row| {
                let kind: String = row.get(2)?;
                let raw_group: String = row.get(4)?;
                Ok(UnsentChange {
                    id: row.get(0)?,
                    file_id: row.get(1)?,
                    kind: change_kind(&kind, None),
                    path: row.get(3)?,
                    backup: false,
                    group: group_label(&raw_group),
                    queued_at: row.get(5)?,
                    retry_at: None,
                    parked: false,
                    retryable: false,
                    blocked: None,
                })
            })?
            .collect::<Result<Vec<_>>>()?
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
        // Spec D8: a park (any `last_error_class`) is never re-armed, whatever its attempts.
        if class.is_some() {
            return Ok(RetryOutcome::Parked);
        }
        let gave_up = attempts >= max_attempts;
        if claim.is_some() || (!gave_up && (attempts == 0 || next_retry_at <= now)) {
            return Ok(RetryOutcome::AlreadyQueued);
        }
        // Task 2414: the reconcile pass's checks before it re-arms a change (spec D8). Sent now, a superseded change
        // would make old content current, and one whose staged copy is gone would only fail again: refused, and the
        // row stays as it is (the list words it, `UnsentChange::blocked`), never "Queued" and then stopped again.
        if let Some(block) = presend_block_conn(&tx, op_id)? {
            return Ok(RetryOutcome::blocked_by(block));
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
