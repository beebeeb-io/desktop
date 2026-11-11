//! Local and transfer activity, backlog, queue diagnostics and bandwidth samples (split out of `state_db.rs`).

use super::*;

impl StateDb {
    pub fn record_local_activity(&self, input: LocalActivityEventInput) -> Result<()> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO local_activity (event_type, file_id, file_name, rel_path, occurred_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                input.event_type.as_str(),
                input.file_id,
                input.file_name,
                input.rel_path,
                input.occurred_at
            ],
        )?;
        tx.execute(
            "DELETE FROM local_activity
             WHERE id NOT IN (
               SELECT id FROM local_activity
               ORDER BY occurred_at DESC, id DESC
               LIMIT ?1
             )",
            params![LOCAL_ACTIVITY_MAX_ROWS as i64],
        )?;
        tx.commit()?;
        Ok(())
    }
    /// Return newest local activity events, capped by the caller's limit.
    pub fn list_recent_local_activity(&self, limit: usize) -> Result<Vec<LocalActivityEvent>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = limit.min(LOCAL_ACTIVITY_MAX_ROWS);
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, event_type, file_id, file_name, rel_path, occurred_at
             FROM local_activity
             ORDER BY occurred_at DESC, id DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            let event_type = LocalActivityKind::from_str(&row.get::<_, String>(1)?);
            Ok(LocalActivityEvent {
                id: row.get(0)?,
                event_type,
                file_id: row.get(2)?,
                file_name: row.get(3)?,
                rel_path: row.get(4)?,
                occurred_at: row.get(5)?,
            })
        })?;
        rows.collect()
    }
    /// Record a finished upload or download and prune old rows.
    pub fn record_transfer_activity(&self, input: TransferActivityInput) -> Result<()> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO transfer_activity (direction, file_id, file_name, rel_path, bytes, occurred_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                input.direction,
                input.file_id,
                input.file_name,
                input.rel_path,
                input.bytes.max(0),
                input.occurred_at
            ],
        )?;
        tx.execute(
            "DELETE FROM transfer_activity
             WHERE id NOT IN (
               SELECT id FROM transfer_activity
               ORDER BY occurred_at DESC, id DESC
               LIMIT ?1
             )",
            params![TRANSFER_ACTIVITY_MAX_ROWS as i64],
        )?;
        tx.commit()?;
        Ok(())
    }
    /// Newest finished transfers first, capped by the caller's limit.
    pub fn list_recent_transfer_activity(&self, limit: usize) -> Result<Vec<TransferActivity>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = limit.min(TRANSFER_ACTIVITY_MAX_ROWS);
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, direction, file_id, file_name, rel_path, bytes, occurred_at
             FROM transfer_activity
             ORDER BY occurred_at DESC, id DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(TransferActivity {
                id: row.get(0)?,
                direction: row.get(1)?,
                file_id: row.get(2)?,
                file_name: row.get(3)?,
                rel_path: row.get(4)?,
                bytes: row.get(5)?,
                occurred_at: row.get(6)?,
            })
        })?;
        rows.collect()
    }
    /// Desktop 0.8.15 (deliverable 3): queued changes held back (`paused_reason` set: the `auth`
    /// pause, a quota, a review). The UI shows them as "N changes on hold", never as syncing; they
    /// are not transfer work ([`Self::transfer_backlog`] leaves them out).
    pub fn count_operations_on_hold(&self) -> Result<u32> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let held: i64 = conn.query_row(
            "SELECT COUNT(*) FROM operation_queue WHERE paused_reason IS NOT NULL",
            [],
            |row| row.get(0),
        )?;
        Ok(u32::try_from(held.max(0)).unwrap_or(u32::MAX))
    }
    /// Pages this connection has read from the database file since it opened: SQLite's page-cache misses. Task 2243's
    /// tests measure what one poll costs the disk with it.
    #[cfg(any(test, feature = "test-support"))]
    pub fn page_reads(&self) -> i64 {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let (mut current, mut highwater) = (0, 0);
        // SAFETY: `conn` is an open connection, held for the call; both out-pointers live on this frame.
        let rc = unsafe {
            rusqlite::ffi::sqlite3_db_status(
                conn.handle(),
                rusqlite::ffi::SQLITE_DBSTATUS_CACHE_MISS,
                &mut current,
                &mut highwater,
                0,
            )
        };
        assert_eq!(rc, rusqlite::ffi::SQLITE_OK, "sqlite3_db_status");
        i64::from(current)
    }
    /// What is waiting to transfer (see [`TransferBacklog`]). `now` is unix seconds: an upload whose
    /// retry time is still ahead of it is waiting out a backoff and does not count.
    pub fn transfer_backlog(&self, now: i64) -> Result<TransferBacklog> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        // D7: an upload waiting for a held folder is not sent until the hold ends, so it is not transfer work; counted,
        // it would keep the engine "syncing" for as long as the hold lasts. One waiting for a folder still on its way
        // is (`DUE_PARENT_GATE_SQL` holds it for that folder's landing only).
        #[cfg(target_os = "macos")]
        let not_held = "";
        #[cfg(not(target_os = "macos"))]
        let not_held = format!(
            "AND (q.parent_id IS NULL
                  OR q.parent_id NOT IN (WITH RECURSIVE {HELD_FOLDER_CTE} SELECT file_id FROM held_folder))"
        );
        let (upload_files, upload_bytes): (i64, i64) = conn.query_row(
            &format!(
                "SELECT COUNT(*), COALESCE(SUM(MAX(COALESCE(f.size_bytes, 0), 0)), 0)
                 FROM operation_queue q LEFT JOIN files f ON f.file_id = q.file_id
                 WHERE q.kind IN ('upload_file', 'upload_version')
                   AND q.next_retry_at <= ?1
                   AND q.paused_reason IS NULL AND q.attempts < q.max_attempts
                   {not_held}"
            ),
            params![now],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let (download_files, download_bytes): (i64, i64) = conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(MAX(size_bytes, 0)), 0) FROM files WHERE status = 'downloading'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let queued_ops: i64 = conn.query_row("SELECT COUNT(*) FROM operation_queue", [], |row| row.get(0))?;
        let paused_for_quota: i64 = conn.query_row(
            "SELECT COUNT(*) FROM operation_queue WHERE paused_reason = 'quota'",
            [],
            |row| row.get(0),
        )?;
        let to_u32 = |n: i64| u32::try_from(n.max(0)).unwrap_or(u32::MAX);
        Ok(TransferBacklog {
            upload_files: to_u32(upload_files),
            upload_bytes: upload_bytes.max(0) as u64,
            download_files: to_u32(download_files),
            download_bytes: download_bytes.max(0) as u64,
            queued_ops: to_u32(queued_ops),
            paused_for_quota: to_u32(paused_for_quota),
        })
    }
    /// Queue counts and the last error, redacted for the support bundle.
    /// Names known to the state DB are scrubbed from the error text.
    pub fn queue_diagnostics(&self, now: i64) -> Result<QueueDiagnostics> {
        self.queue_diagnostics_with_paths(now, &[])
    }
    /// Every plaintext name this daemon knows locally, for redacting the lifecycle log
    /// (spec 2026-10-06 §8). Same source as the support bundle's redaction.
    pub fn known_names(&self, extra_paths: &[String]) -> Result<KnownNames> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        collect_known_names(&conn, extra_paths)
    }
    /// Like [`Self::queue_diagnostics`], additionally treating every component
    /// of `extra_paths` (for example the sync root) as a name to scrub.
    pub fn queue_diagnostics_with_paths(&self, now: i64, extra_paths: &[String]) -> Result<QueueDiagnostics> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let queued = conn.query_row("SELECT COUNT(*) FROM operation_queue", [], |row| row.get(0))?;
        let due = conn.query_row(
            &format!(
                "SELECT COUNT(*) FROM operation_queue
                 WHERE next_retry_at <= ?1 AND attempts < max_attempts AND paused_reason IS NULL {DUE_PARENT_GATE_SQL}"
            ),
            params![now],
            |row| row.get(0),
        )?;
        let paused = conn.query_row(
            "SELECT COUNT(*) FROM operation_queue WHERE paused_reason IS NOT NULL",
            [],
            |row| row.get(0),
        )?;

        let by_kind = allowed_group_labels(count_queue_groups(&conn, "kind")?, QUEUE_KIND_LABELS);
        let paused_by_reason = allowed_group_labels(count_queue_groups(&conn, "paused_reason")?, PAUSE_REASON_LABELS);
        let (last_error, last_error_class) = conn
            .query_row(
                "SELECT last_error, last_error_class
                 FROM operation_queue
                 WHERE last_error IS NOT NULL
                 ORDER BY updated_at DESC, created_at DESC
                 LIMIT 1",
                [],
                |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .unwrap_or((None, None));

        let (last_error, last_error_code, last_error_redactions) = match last_error.as_deref() {
            Some(raw) => {
                let names = collect_known_names(&conn, extra_paths)?;
                let redacted = redact_for_export(raw, &names);
                (Some(redacted.text), Some(classify_error_code(raw)), redacted.redactions)
            }
            None => (None, None, 0),
        };

        Ok(QueueDiagnostics {
            queued,
            due,
            paused,
            by_kind,
            paused_by_reason,
            last_error,
            last_error_code,
            last_error_redactions,
            last_error_class: last_error_class
                .as_deref()
                .map(|class| allowed_label(class, PAUSE_REASON_LABELS)),
        })
    }
    /// [`QueueHold`] for the tray and Settings › Sync (0.8.15). Reads counts only, without the name-redaction pass of
    /// [`Self::queue_diagnostics`], so a window can poll it. Changes nothing.
    ///
    /// Task 2409: [`Self::unsent_changes`] lists exactly the rows counted here, in the same groups. Both leave out the
    /// downloads a pin queues ([`UNSENT_KINDS_SQL`], task 2224): they are not changes waiting to leave this computer.
    /// Task 2414: those that will not finish by themselves are `not_downloaded` ([`NOT_DOWNLOADED_SQL`]), which
    /// [`Self::not_downloaded`] lists.
    pub fn queue_hold_summary(&self) -> Result<QueueHold> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let (waiting, gave_up) = conn.query_row(
            &format!(
                "SELECT COALESCE(SUM(attempts < max_attempts), 0), COALESCE(SUM(attempts >= max_attempts), 0)
                 FROM operation_queue
                 WHERE paused_reason IS NULL AND {UNSENT_KINDS_SQL}"
            ),
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )?;
        // Task 2414: `NOT_DOWNLOADED_SQL`, split on `paused_reason` so each half searches the two ranges of
        // `idx_operation_queue_unsent` the counts above and below already read (one scan of the whole index read 26
        // more pages from disk in task 2243's poll test). The index carries every column, so no operation row.
        let not_downloaded: i64 = conn.query_row(
            &format!(
                "SELECT (SELECT COUNT(*) FROM operation_queue WHERE paused_reason IS NOT NULL AND {NOT_DOWNLOADED_SQL})
                      + (SELECT COUNT(*) FROM operation_queue WHERE paused_reason IS NULL AND {NOT_DOWNLOADED_SQL})"
            ),
            [],
            |row| row.get(0),
        )?;
        let mut stmt = conn.prepare(&format!(
            "SELECT paused_reason, COUNT(*), MIN(created_at), MAX(created_at)
             FROM operation_queue
             WHERE paused_reason IS NOT NULL AND {UNSENT_KINDS_SQL}
             GROUP BY paused_reason"
        ))?;
        let groups = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;
        let mut held: BTreeMap<String, HeldGroup> = BTreeMap::new();
        let mut add = |reason: String, count: i64, oldest_at: i64, newest_at: i64| {
            let entry = held.entry(reason.clone()).or_insert(HeldGroup {
                reason,
                count: 0,
                oldest_at,
                newest_at,
            });
            entry.count += count;
            entry.oldest_at = entry.oldest_at.min(oldest_at);
            entry.newest_at = entry.newest_at.max(newest_at);
        };
        for group in groups {
            let (reason, count, oldest_at, newest_at) = group?;
            add(allowed_label(&reason, PAUSE_REASON_LABELS), count, oldest_at, newest_at);
        }
        // D7: a change that waits for its folder's create (`DUE_PARENT_GATE_SQL`) while that create is paused or out of
        // attempts is counted as `waiting_for_folder`, not as waiting: nothing sends it until the hold ends. The same
        // goes for a change in a folder made inside such a folder, at any depth. Read from the two D7 indexes alone;
        // CROSS JOIN keeps the held folders as the outer loop, so no change outside them is read.
        #[cfg(not(target_os = "macos"))]
        let waiting = {
            let (count, oldest_at, newest_at) = conn.query_row(
                &format!(
                    "WITH RECURSIVE {HELD_FOLDER_CTE}
                     SELECT COUNT(*), COALESCE(MIN(op.created_at), 0), COALESCE(MAX(op.created_at), 0)
                     FROM held_folder CROSS JOIN operation_queue op ON op.parent_id = held_folder.file_id
                     WHERE op.paused_reason IS NULL AND op.attempts < op.max_attempts AND {UNSENT_KINDS_SQL}"
                ),
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?)),
            )?;
            if count > 0 {
                add("waiting_for_folder".to_string(), count, oldest_at, newest_at);
            }
            waiting - count
        };
        // Task 2270 (spec U1): uploads this PC could not mark as synced for a lasting reason, dated by when each was
        // held. Not a pause reason, and not an operation: the upload itself landed.
        let (count, oldest_at, newest_at) = conn.query_row(
            "SELECT COUNT(*), COALESCE(MIN(review_at), 0), COALESCE(MAX(review_at), 0)
             FROM upload_finalizations WHERE review IS NOT NULL",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?)),
        )?;
        if count > 0 {
            add("not_marked_synced".to_string(), count, oldest_at, newest_at);
        }
        Ok(QueueHold {
            waiting,
            gave_up,
            held: held.into_values().collect(),
            not_downloaded,
        })
    }
    // ── Bandwidth samples (P3 — 20h traffic chart, task 0810) ────────────────
    //
    // One row per heartbeat beat (~20 s).  The chart reads the last N hours and
    // downsamples to a fixed number of display buckets client-side (in React).
    // Only the last ~25 h are kept on disk; `prune_bandwidth_samples` removes older
    // rows after every write so the table stays bounded.
    /// Insert one bandwidth sample.  `sampled_at` is seconds-since-epoch.
    pub fn insert_bandwidth_sample(
        &self,
        sampled_at: i64,
        up_bytes: u64,
        down_bytes: u64,
        period_secs: u32,
    ) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "INSERT INTO bandwidth_samples (sampled_at, up_bytes, down_bytes, period_secs)
             VALUES (?1, ?2, ?3, ?4)",
            params![sampled_at, up_bytes as i64, down_bytes as i64, period_secs as i64],
        )?;
        Ok(())
    }
    /// Fetch bandwidth samples with `sampled_at >= since_secs`, oldest first.
    /// Deterministic (no wall-clock read) — used by callers that already have a
    /// cutoff and by tests.
    pub fn get_bandwidth_history_since(&self, since_secs: i64) -> Result<Vec<BandwidthSample>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT sampled_at, up_bytes, down_bytes, period_secs
             FROM bandwidth_samples
             WHERE sampled_at >= ?1
             ORDER BY sampled_at ASC",
        )?;
        let rows = stmt.query_map(params![since_secs], |row| {
            Ok(BandwidthSample {
                sampled_at: row.get(0)?,
                up_bytes: row.get::<_, i64>(1)? as u64,
                down_bytes: row.get::<_, i64>(2)? as u64,
                period_secs: row.get::<_, i64>(3)? as u32,
            })
        })?;
        rows.collect()
    }
    /// Fetch bandwidth samples from the last `hours` hours (oldest first).
    pub fn get_bandwidth_history(&self, hours: u32) -> Result<Vec<BandwidthSample>> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        self.get_bandwidth_history_since(now - hours as i64 * 3600)
    }
    /// Fetch the newest bandwidth sample, if any.
    pub fn latest_bandwidth_sample(&self) -> Result<Option<BandwidthSample>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT sampled_at, up_bytes, down_bytes, period_secs
             FROM bandwidth_samples
             ORDER BY sampled_at DESC
             LIMIT 1",
        )?;
        let mut rows = stmt.query([])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(BandwidthSample {
            sampled_at: row.get(0)?,
            up_bytes: row.get::<_, i64>(1)? as u64,
            down_bytes: row.get::<_, i64>(2)? as u64,
            period_secs: row.get::<_, i64>(3)? as u32,
        }))
    }
    /// Remove samples older than `cutoff_secs` (seconds-since-epoch) to bound DB size.
    pub fn prune_bandwidth_samples(&self, cutoff_secs: i64) -> Result<usize> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let n = conn.execute(
            "DELETE FROM bandwidth_samples WHERE sampled_at < ?1",
            params![cutoff_secs],
        )?;
        Ok(n)
    }
}
