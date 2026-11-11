//! Sync cursor, re-snapshot, the reconcile pass and the File Provider change log (split out of `state_db.rs`).

use super::*;

impl StateDb {
    // ── /sync delta-engine cursor + prune (task 0789) ──────────────────────────
    //
    // The `sync_state` kv table holds the `/sync/ops` cursor (the highest
    // `seq_id` we have applied). On boot the cursor is unset → the engine pulls
    // a full `/sync/snapshot` (authoritative tree + seq_id), then advances the
    // cursor by `seq_id` as it applies each delta op. This replaces the old
    // per-tick full-tree `/files` re-walk and is what fixes the silent
    // deletion-reconciliation bug (the snapshot path prunes server-deleted
    // rows; the ops path applies trash/delete ops).
    const SYNC_CURSOR_KEY: &'static str = "sync_ops_cursor";
    const NEEDS_RESNAPSHOT_KEY: &'static str = "sync_needs_resnapshot";
    /// Read the persisted `/sync/ops` cursor (highest applied `seq_id`).
    /// `Ok(None)` when unset (fresh DB / never bootstrapped) — the caller treats
    /// that as "needs a full snapshot". A stored-but-unparseable value is also
    /// reported as `None` (forces a safe re-bootstrap rather than a panic).
    pub fn get_sync_cursor(&self) -> Result<Option<i64>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let raw: Option<String> = conn
            .query_row(
                "SELECT value FROM sync_state WHERE key = ?1",
                params![Self::SYNC_CURSOR_KEY],
                |row| row.get(0),
            )
            .optional()?;
        Ok(raw.and_then(|s| s.parse::<i64>().ok()))
    }
    /// Persist the `/sync/ops` cursor (highest applied `seq_id`). Stored as TEXT
    /// so the kv table stays type-agnostic; idempotent upsert on the fixed key.
    pub fn set_sync_cursor(&self, seq_id: i64) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "INSERT INTO sync_state (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![Self::SYNC_CURSOR_KEY, seq_id.to_string()],
        )?;
        Ok(())
    }
    /// Mark that the next `sync_tick` must re-bootstrap from a fresh
    /// `/sync/snapshot` regardless of the cursor. Used by gap-recovery cases the
    /// delta path cannot reconcile from the op alone — notably `file_restore`,
    /// whose op payload is only `{ id }`, so the row it un-trashes cannot be
    /// rebuilt without the authoritative snapshot. Persisted so the request
    /// survives a restart between ticks. On macOS each call increments a request
    /// counter (spec §6.3.2, I-2(b)); elsewhere it sets a take-once flag.
    pub fn request_resnapshot(&self) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Self::request_resnapshot_conn(&conn)
    }
    /// [`Self::request_resnapshot`] inside the caller's connection or transaction.
    /// The value counts the requests, so a request made while a snapshot runs is
    /// still pending after that snapshot clears the one it read.
    #[cfg(target_os = "macos")]
    pub(super) fn request_resnapshot_conn(conn: &Connection) -> Result<()> {
        conn.execute(
            "INSERT INTO sync_state (key, value) VALUES (?1, '1')
             ON CONFLICT(key) DO UPDATE SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)",
            params![Self::NEEDS_RESNAPSHOT_KEY],
        )?;
        Ok(())
    }
    /// [`Self::request_resnapshot`] inside the caller's connection or transaction.
    #[cfg(not(target_os = "macos"))]
    pub(super) fn request_resnapshot_conn(conn: &Connection) -> Result<()> {
        conn.execute(
            "INSERT INTO sync_state (key, value) VALUES (?1, '1')
             ON CONFLICT(key) DO UPDATE SET value = '1'",
            params![Self::NEEDS_RESNAPSHOT_KEY],
        )?;
        Ok(())
    }
    /// Atomically read-and-clear the "needs re-snapshot" flag. Returns `true`
    /// exactly once per [`Self::request_resnapshot`] call, so the bootstrap runs
    /// on the very next tick and not on every subsequent tick. The DELETE in the
    /// same locked critical section makes the take-and-clear race-free against a
    /// concurrent `request_resnapshot`. Not on macOS: there the request is read with
    /// [`Self::peek_resnapshot_request`] and cleared only after the snapshot succeeded.
    #[cfg(not(target_os = "macos"))]
    pub fn take_needs_resnapshot(&self) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let present: Option<String> = conn
            .query_row(
                "SELECT value FROM sync_state WHERE key = ?1",
                params![Self::NEEDS_RESNAPSHOT_KEY],
                |row| row.get(0),
            )
            .optional()?;
        if present.is_some() {
            conn.execute(
                "DELETE FROM sync_state WHERE key = ?1",
                params![Self::NEEDS_RESNAPSHOT_KEY],
            )?;
        }
        Ok(present.is_some())
    }
    /// The pending request's counter, or `None`. Read at the start of a tick; the
    /// request is cleared only after the snapshot succeeded (§6.3.2, I-2(b)).
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn peek_resnapshot_request(&self) -> Result<Option<i64>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            "SELECT CAST(value AS INTEGER) FROM sync_state WHERE key = ?1",
            params![Self::NEEDS_RESNAPSHOT_KEY],
            |row| row.get(0),
        )
        .optional()
    }
    /// Clear the request the tick read; a request made meanwhile survives.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn clear_resnapshot_request(&self, seen: i64) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let n = conn.execute(
            "DELETE FROM sync_state WHERE key = ?1 AND CAST(value AS INTEGER) = ?2",
            params![Self::NEEDS_RESNAPSHOT_KEY, seen],
        )?;
        Ok(n == 1)
    }
    /// Task 1640: a sync-folder upload of this file's content (no write id: Windows) is
    /// queued, in any state, parked included. Its bytes are not sent until it lands.
    pub fn sync_folder_upload_queued(&self, file_id: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM operation_queue
                            WHERE file_id = ?1 AND write_id IS NULL
                              AND kind IN ('upload_version', 'upload_file'))",
            params![file_id],
            |row| row.get(0),
        )
    }
    /// Clear transient transfer states left behind by a previous process.
    ///
    /// `Uploading` and `Downloading` mean "the current engine is actively moving
    /// bytes". After a crash, abort, logout, or process kill there is no active
    /// transfer owning those rows anymore, so startup must reconcile them before
    /// tray/status snapshots count them as live work:
    /// - `Downloading` falls back to `CloudOnly`; a later open/pin can rehydrate.
    /// - `Uploading` becomes `Error`; the durable operation queue still carries
    ///   any staged upload retry, but the row is not reported as in-flight.
    /// - Except a row with a Finder upload that has not parked (spec §9.2): it stays
    ///   `Uploading`, because that upload resumes after the relaunch and the item stays
    ///   writable and not evictable meanwhile. Only a Finder upload carries a write id.
    ///
    /// Returns the ids of the rows it reset: their count is what the engine-start line reports,
    /// and the first tick's working-set signal carries them. A row it keeps `Uploading` is not
    /// touched, so not returned. Each reset changes how the item is presented, so it is recorded
    /// for the system, as every status change is; without the record the system kept showing
    /// the item as it was until the item changed again.
    pub fn reconcile_stale_in_flight_on_startup(&self) -> Result<Vec<String>> {
        const STALE: &str = "status = 'downloading'
                OR (status = 'uploading' AND NOT EXISTS (
                        SELECT 1 FROM operation_queue q
                        WHERE q.file_id = files.file_id AND q.write_id IS NOT NULL
                          AND q.kind IN ('upload_version', 'upload_file') AND q.attempts < q.max_attempts))";
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let reset = {
            let mut stmt = tx.prepare(&format!("SELECT file_id FROM files WHERE {STALE} ORDER BY file_id"))?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>>>()?
        };
        // Task 1640 (review F1): a download cut off by the stop may have written some bytes.
        let downloading = {
            let mut stmt = tx.prepare("SELECT file_id FROM files WHERE status = 'downloading'")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>>>()?
        };
        tx.execute(
            &format!(
                "UPDATE files SET status = CASE status WHEN 'downloading' THEN 'cloud_only' ELSE 'error' END
                 WHERE {STALE}"
            ),
            [],
        )?;
        for file_id in &downloading {
            poison_local_content_conn(&tx, file_id)?;
        }
        for file_id in &reset {
            record_file_change_conn(&tx, file_id, FpChangeKind::Modified, None)?;
        }
        tx.commit()?;
        Ok(reset)
    }
    /// Apply a batch of native-Explorer-derived state deltas in ONE
    /// transaction. Each tuple is `(file_id, status_opt, pin_opt)` where a
    /// `Some` means "the on-disk placeholder disagrees with the DB, write this";
    /// a `None` means "leave that column untouched".
    ///
    /// Drives the Windows per-tick reconcile pass
    /// ([`crate::windows_cf`]'s `reconcile_placeholder_state`): a NATIVE pin /
    /// free-up the user did in Explorer (not through the in-app pin path) shows
    /// up here as a delta and is written back so the app's view matches reality.
    /// Callers pass DELTAS ONLY — the reconcile pass compares each desired value
    /// to the live row and never enqueues a no-op — so this method stays churn-
    /// free in steady state. The in-app pin path writes DB + OS together, so the
    /// next reconcile sees no delta for those files.
    ///
    /// - A `Some(status)` updates `status` and re-stamps `last_sync_at = now`.
    /// - A `Some(pin)` updates `pin_state` only (inheritance/effective resolution
    ///   is unchanged — we only persist the explicit per-file pin the OS reports).
    ///
    /// Returns the number of rows touched (a row updated for both status and pin
    /// counts the affected-row total across both statements). Empty input is a
    /// fast `Ok(0)` with no transaction opened.
    #[cfg(any(target_os = "windows", test, feature = "test-support"))]
    pub fn reconcile_os_state(
        &self,
        deltas: &[(String, Option<FileStatus>, Option<PinState>)],
        now: i64,
    ) -> Result<usize> {
        if deltas.is_empty() {
            return Ok(0);
        }
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        let mut touched = 0usize;
        for (file_id, status_opt, pin_opt) in deltas {
            if let Some(status) = status_opt {
                touched += tx.execute(
                    "UPDATE files SET status = ?1, last_sync_at = ?2 WHERE file_id = ?3",
                    params![status.as_str(), now, file_id],
                )?;
            }
            if let Some(pin) = pin_opt {
                touched += tx.execute(
                    "UPDATE files SET pin_state = ?1 WHERE file_id = ?2",
                    params![pin.as_str(), file_id],
                )?;
            }
        }
        tx.commit()?;
        Ok(touched)
    }
    /// §6.3.2: a snapshot node's version, applied to the row in one transaction (S1.5).
    /// The fill (the row is at 0) and the raise (the node is newer) set `current_version`
    /// in any status but `Trashing` and `Conflict`; a node still uploading is skipped and
    /// the snapshot request stays. Nothing else of the row changes here.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn apply_snapshot_version(
        &self,
        file_id: &str,
        node_version: i64,
        node_size: i64,
        node_is_uploading: impl Into<SnapshotNodeState>,
    ) -> Result<SnapshotVersion> {
        let node_is_uploading: SnapshotNodeState = node_is_uploading.into();
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let row: Option<(String, i64)> = tx
            .query_row(
                "SELECT status, current_version FROM files WHERE file_id = ?1",
                params![file_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((status, current)) = row else {
            return Ok(SnapshotVersion::Unchanged);
        };
        // A Conflict row owns its transitions; a Trashing row is leaving.
        if status == FileStatus::Trashing.as_str() || status == FileStatus::Conflict.as_str() {
            return Ok(SnapshotVersion::Unchanged);
        }
        let fill = current == 0 && node_version > 0;
        let raise = current > 0 && node_version > current;
        if !fill && !raise {
            return Ok(SnapshotVersion::Unchanged);
        }
        if node_is_uploading.as_bool() {
            // The clear at the end of this tick then matches no row: the request stays.
            Self::request_resnapshot_conn(&tx)?;
            tx.commit()?;
            return Ok(SnapshotVersion::SkippedUploading);
        }
        let write_queued: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM operation_queue WHERE file_id = ?1 AND write_id IS NOT NULL
                            AND kind IN ('upload_version', 'upload_file'))",
            params![file_id],
            |r| r.get(0),
        )?;
        // §6.3.2 sets `current_version` and the size, and writes `local_base_version` in
        // neither branch: it is the stale marker. On a row with a queued write the raise
        // changes `current_version` only: the size is that write's, and the object id and
        // `version_filled` stay.
        if write_queued {
            tx.execute(
                "UPDATE files SET current_version = ?2 WHERE file_id = ?1",
                params![file_id, node_version],
            )?;
        } else {
            tx.execute(
                "UPDATE files SET current_version = ?2, size_bytes = ?3 WHERE file_id = ?1",
                params![file_id, node_version, node_size],
            )?;
            if raise {
                // The snapshot carries no object version id; the old one is no longer current.
                tx.execute(
                    "UPDATE files SET current_object_version_id = NULL, version_filled = 0 WHERE file_id = ?1",
                    params![file_id],
                )?;
            }
        }
        let outcome = if fill {
            tx.execute(
                "UPDATE files SET version_filled = 1 WHERE file_id = ?1",
                params![file_id],
            )?;
            let resolved_ops = {
                let mut stmt = tx.prepare(
                    "SELECT op_id FROM operation_queue WHERE file_id = ?1 AND base_pending > 0 ORDER BY rowid",
                )?;
                let rows = stmt.query_map(params![file_id], |r| r.get::<_, String>(0))?;
                rows.collect::<Result<Vec<_>>>()?
            };
            tx.execute(
                "UPDATE operation_queue SET base_version = ?2, base_pending = 0
                 WHERE file_id = ?1 AND base_pending > 0",
                params![file_id, node_version],
            )?;
            SnapshotVersion::Filled { resolved_ops }
        } else {
            SnapshotVersion::Raised {
                old: current,
                new: node_version,
            }
        };
        record_file_change_conn(&tx, file_id, FpChangeKind::Modified, None)?;
        tx.commit()?;
        Ok(outcome)
    }
    /// An upload waits for a snapshot to learn its base (spec §6.3.3).
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn has_base_pending_uploads(&self) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM operation_queue WHERE base_pending > 0)",
            [],
            |r| r.get(0),
        )
    }
    /// After a successful snapshot: one more miss for every op still waiting for its base;
    /// at the 10th it parks `base_unknown` with its bytes (§6.3.3), through the same park
    /// statement the claim uses. Returns the ops it parked: `(op_id, file_id)`.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn note_snapshot_for_base_pending(&self, now: i64) -> Result<Vec<(String, Option<String>)>> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE operation_queue SET base_pending = base_pending + 1 WHERE base_pending > 0",
            [],
        )?;
        let counted_out = {
            let mut stmt =
                tx.prepare("SELECT op_id, file_id FROM operation_queue WHERE base_pending >= ?1 ORDER BY rowid")?;
            let rows = stmt.query_map(params![BASE_PENDING_PARK_AT], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
            })?;
            rows.collect::<Result<Vec<_>>>()?
        };
        let mut parked = Vec::new();
        for (op_id, file_id) in counted_out {
            // No longer waiting, so no pass asks a snapshot for it any more.
            tx.execute(
                "UPDATE operation_queue SET base_pending = 0 WHERE op_id = ?1",
                params![op_id],
            )?;
            // A recorded completion is never parked (§8.6 rule 6).
            if park_unclaimed_conn(&tx, &op_id, ParkReason::BaseUnknown, now)? == 1 {
                if let Some(file_id) = file_id.as_deref() {
                    settle_status_after_park_conn(&tx, file_id)?;
                }
                parked.push((op_id, file_id));
            }
        }
        tx.commit()?;
        Ok(parked)
    }
    /// A content op, a landing or a restore touched the row: its version no longer came
    /// from a snapshot fill (spec §6.1, `version_filled`).
    #[cfg(target_os = "macos")]
    pub fn clear_version_filled(&self, file_id: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "UPDATE files SET version_filled = 0 WHERE file_id = ?1",
            params![file_id],
        )?;
        Ok(())
    }
    /// Changes after `since_anchor`, ALL of them, with the anchor to persist
    /// afterwards. The daemon RPC adds paging on top of this (see
    /// [`Self::list_file_changes_paged`]); this all-at-once form is the unit
    /// level the change-log tests pin.
    ///
    /// Returns `(changes, Some(next_anchor))` — or `(vec![], Some(anchor))`
    /// unchanged when the caller is already up to date — and `None` (no rows)
    /// never happens for the default domain: a fresh, empty log reports an
    /// empty anchor. `next_anchor` moves only when changes were delivered.
    #[cfg(any(test, feature = "test-support"))]
    pub fn list_file_changes(&self, since_anchor: Option<&[u8]>) -> Result<Option<(Vec<FileChange>, Option<Vec<u8>>)>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let since: i64 = match since_anchor {
            None => 0,
            Some(bytes) => std::str::from_utf8(bytes)
                .ok()
                .and_then(|text| text.parse::<i64>().ok())
                .ok_or_else(|| {
                    rusqlite::Error::InvalidParameterName(
                        "fp sync anchor is not a recognisable cursor (expired)".into(),
                    )
                })?,
        };
        let tip: i64 = conn.query_row("SELECT COALESCE((SELECT MAX(seq) FROM fp_changes), 0)", [], |row| {
            row.get(0)
        })?;
        if tip <= since {
            // Up to date: echo the caller's anchor (or nil at genesis) and
            // deliver nothing. The anchor must not move — a stable anchor on a
            // no-op poll is what tells the system `moreComing: false` is honest.
            return Ok(Some((Vec::new(), since_anchor.map(|bytes| bytes.to_vec()))));
        }
        let mut stmt = conn.prepare(
            "SELECT seq, file_id, kind, old_parent_id, new_parent_id
             FROM fp_changes WHERE seq > ?1 ORDER BY seq ASC",
        )?;
        let changes = stmt
            .query_map(params![since], |row| {
                Ok(FileChange {
                    seq: row.get(0)?,
                    file_id: row.get(1)?,
                    kind: FpChangeKind::from_str(&row.get::<_, String>(2)?).ok_or_else(|| {
                        rusqlite::Error::InvalidColumnType(2, "kind".to_string(), rusqlite::types::Type::Text)
                    })?,
                    old_parent_id: row.get(3)?,
                    new_parent_id: row.get(4)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let next_anchor = anchor_bytes(tip);
        Ok(Some((changes, Some(next_anchor))))
    }
    /// Paged variant of [`Self::list_file_changes`]: at most `limit` changes
    /// per call; `next_anchor` is `Some(next_cursor_bytes)` while MORE changes
    /// remain (resume token for the next page) and `None` when the batch
    /// completes the change window. The resume token IS the anchor: paging
    /// consumes the same monotonic cursor, so a client that stops mid-window
    /// resumes exactly where it stopped, and a client that never resumes
    /// cannot lose data (the anchor only advances when it re-polls from nil).
    pub fn list_file_changes_paged(
        &self,
        since_anchor: Option<&[u8]>,
        limit: usize,
    ) -> Result<Option<(Vec<FileChange>, Option<Vec<u8>>)>> {
        let limit = limit.max(1);
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let since: i64 = match since_anchor {
            None => 0,
            Some(bytes) => std::str::from_utf8(bytes)
                .ok()
                .and_then(|text| text.parse::<i64>().ok())
                .ok_or_else(|| {
                    rusqlite::Error::InvalidParameterName(
                        "fp sync anchor is not a recognisable cursor (expired)".into(),
                    )
                })?,
        };
        let mut stmt = conn.prepare(
            "SELECT seq, file_id, kind, old_parent_id, new_parent_id
             FROM fp_changes WHERE seq > ?1 ORDER BY seq ASC LIMIT ?2",
        )?;
        let changes = stmt
            .query_map(params![since, limit as i64], |row| {
                Ok(FileChange {
                    seq: row.get(0)?,
                    file_id: row.get(1)?,
                    kind: FpChangeKind::from_str(&row.get::<_, String>(2)?).ok_or_else(|| {
                        rusqlite::Error::InvalidColumnType(2, "kind".to_string(), rusqlite::types::Type::Text)
                    })?,
                    old_parent_id: row.get(3)?,
                    new_parent_id: row.get(4)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if changes.len() < limit {
            // Short page. It is a COMPLETED batch when it reaches the log tip
            // (or the tip is at/below the caller's cursor) — otherwise the
            // caller still needs a resume token to fetch the rest.
            let tip: i64 = conn.query_row("SELECT COALESCE((SELECT MAX(seq) FROM fp_changes), 0)", [], |row| {
                row.get(0)
            })?;
            let reached_tip = changes.last().map(|change| change.seq >= tip).unwrap_or(tip <= since);
            if reached_tip {
                // Up to date AND the batch ends here: the anchor is the log
                // tip — "you are now current as of tip". Only a completely
                // empty log (no anchor ever minted) reports None.
                let anchor = if tip > 0 {
                    Some(anchor_bytes(tip))
                } else {
                    since_anchor.map(|bytes| bytes.to_vec())
                };
                return Ok(Some((changes, anchor)));
            }
            let next = changes.last().map(|change| change.seq).unwrap_or(since);
            return Ok(Some((changes, Some(anchor_bytes(next)))));
        }
        // Full page: there may be more — resume from the last delivered seq.
        let next = changes.last().map(|change| change.seq).unwrap_or(since);
        Ok(Some((changes, Some(anchor_bytes(next)))))
    }
    /// Delete consumed change rows recorded before `before_epoch` (the sweep
    /// runs on a delay so a slow replica can still page through recent
    /// history). NEVER touches `fp_sync_anchor`: the replica's cursor position
    /// must survive even after the changes it points past are gone.
    #[cfg(any(target_os = "macos", test, feature = "test-support"))]
    pub fn sweep_file_changes(&self, before_epoch: i64) -> Result<usize> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute("DELETE FROM fp_changes WHERE recorded_at < ?1", params![before_epoch])
    }
    /// The persistent anchor cursor (per domain; one default domain today).
    /// `currentSyncAnchor` is served from here after extension process death.
    pub fn fp_last_anchor(&self) -> Result<Option<Vec<u8>>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let value: Option<i64> = conn
            .query_row(
                "SELECT last_anchor FROM fp_sync_anchor WHERE domain_id = '__default__'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value.map(anchor_bytes))
    }
    /// Record the materialized containers the system reported. Idempotent; the
    /// full set is written each time the extension reports it.
    pub fn set_materialized_containers(&self, container_ids: &[String]) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.unchecked_transaction()?;
        tx.execute("DELETE FROM fp_materialized", [])?;
        for id in container_ids {
            tx.execute(
                "INSERT OR IGNORE INTO fp_materialized (container_id, updated_at) VALUES (?1, 0)",
                params![id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    #[cfg(target_os = "macos")]
    pub fn materialized_containers(&self) -> Result<Vec<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare("SELECT container_id FROM fp_materialized ORDER BY container_id ASC")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect()
    }
    /// Tasks 1888 + 2226 (spec D1-D4, D8, D14; I1-I4): the reconcile pass. Plans over the whole queue and applies the
    /// plan in transactions of at most [`RECONCILE_UNITS_PER_TX`] whole units, re-planning after each, until the plan
    /// has no unit left or a round applied nothing. Returns the first round's counts: the plan the dry run shows.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    /// `disk_has_file` says whether a sync-root-relative directory holds any file (D5); see [`tree_holds_a_file`].
    pub fn reconcile_held_queue(
        &self,
        opts: &crate::held_queue::Options,
        now: i64,
        disk_has_file: &dyn Fn(&str) -> bool,
    ) -> Result<crate::held_queue::Counts> {
        self.reconcile_in_batches(opts, now, RECONCILE_UNITS_PER_TX, disk_has_file, &mut |_, _| {})
    }
    /// The pass with its batch size, and `between` called after each committed batch (its 1-based number) while no
    /// lock is held: the tests' seam for a change made between two batches.
    pub fn reconcile_in_batches(
        &self,
        opts: &crate::held_queue::Options,
        now: i64,
        per_tx: usize,
        disk_has_file: &dyn Fn(&str) -> bool,
        between: &mut dyn FnMut(&StateDb, usize),
    ) -> Result<crate::held_queue::Counts> {
        let per_tx = per_tx.max(1);
        let mut first: Option<(usize, crate::held_queue::Counts)> = None;
        let mut batch = 0;
        loop {
            let applied = {
                let mut conn = self.0.lock().expect("state_db mutex poisoned");
                let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                let (units, counts) = reconcile_plan_conn(&tx, opts, disk_has_file)?;
                let rounds_allowed = first.get_or_insert((units.len(), counts)).0 / per_tx + 2;
                // Every unit removes its own cause, so the plan shrinks each round; a plan that does not is a bug, and
                // the pass stops rather than loop.
                if batch >= rounds_allowed {
                    return Err(rusqlite::Error::SqliteFailure(
                        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_MISUSE),
                        Some("reconcile: the plan did not shrink".into()),
                    ));
                }
                let take = units.len().min(per_tx);
                for unit in &units[..take] {
                    apply_reconcile_unit_conn(&tx, unit, now)?;
                }
                tx.commit()?;
                take
            };
            if applied == 0 {
                break;
            }
            batch += 1;
            between(self, batch);
        }
        Ok(first.map(|(_, counts)| counts).unwrap_or_default())
    }
    /// Spec D14: the plan's counts, from one read transaction; writes nothing.
    #[cfg(any(test, feature = "test-support"))]
    pub fn reconcile_dry_run(
        &self,
        opts: &crate::held_queue::Options,
        disk_has_file: &dyn Fn(&str) -> bool,
    ) -> Result<crate::held_queue::Counts> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        let (_, counts) = reconcile_plan_conn(&tx, opts, disk_has_file)?;
        tx.rollback()?;
        Ok(counts)
    }
    /// Engine start: no runner survives a restart, so every claim is cleared (S3), and
    /// aliases older than [`ALIAS_MAX_AGE_SECS`] are swept (§7.1).
    /// On macOS it also lists the journalled payloads marked released that nothing
    /// references any more, for the caller to unlink (S6).
    pub fn engine_start_repair(&self) -> Result<EngineStartRepair> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        // Spec 2026-10-10 §15 question 3: an engine that starts takes Finder writes again, whatever a stopped
        // sign-out left closed. A statement of its own, before the repair's transaction: a repair that fails rolls
        // back only its own work, and the runner serves Finder writes after a failed repair too.
        conn.execute(
            "DELETE FROM sync_state WHERE key = ?1",
            params![FINDER_ACCEPTS_CLOSED_KEY],
        )?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let claims_cleared = tx.execute(
            "UPDATE operation_queue SET claim_id = NULL, claimed_at = NULL WHERE claim_id IS NOT NULL",
            [],
        )?;
        // §7.1: an alias is kept 30 days; the daily sweep is the runner's.
        tx.execute(
            "DELETE FROM id_aliases WHERE created_at < ?1",
            params![now_secs() - ALIAS_MAX_AGE_SECS],
        )?;
        // macOS only: the release journal is written only by the macOS landing.
        #[cfg(target_os = "macos")]
        let released_payloads = {
            let mut stmt = tx.prepare(
                "SELECT path FROM staged_payloads
                 WHERE completed = 1
                   AND path NOT IN (SELECT payload_path FROM operation_queue WHERE payload_path IS NOT NULL)
                   AND path NOT IN (SELECT payload_path FROM upload_resume)
                   AND path NOT IN (SELECT payload_path FROM upload_finalizations)
                 ORDER BY path",
            )?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>>>()?
        };
        #[cfg(not(target_os = "macos"))]
        let released_payloads: Vec<String> = Vec::new();
        // §6.3.2: a server-known file row at version 0 asks for a snapshot to learn its
        // version. A provisional row (its create is queued) has no server version yet. A
        // malformed metadata row must not fail the start, so it is read as no create.
        #[cfg(target_os = "macos")]
        let resnapshot_requested = {
            let version_zero: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM files f
                  WHERE f.current_version = 0 AND f.item_kind = 'file' AND f.namespace = 'my_files'
                    AND f.status != 'trashing'
                    AND NOT EXISTS (SELECT 1 FROM operation_queue q
                                    WHERE q.file_id = f.file_id
                                      AND CASE WHEN json_valid(q.metadata_json)
                                               THEN json_extract(q.metadata_json, '$.operation')
                                          END = 'create_file'))",
                [],
                |r| r.get(0),
            )?;
            if version_zero {
                Self::request_resnapshot_conn(&tx)?;
            }
            version_zero
        };
        #[cfg(not(target_os = "macos"))]
        let resnapshot_requested = false;
        tx.commit()?;
        Ok(EngineStartRepair {
            claims_cleared,
            released_payloads,
            resnapshot_requested,
        })
    }
    /// §10.2–§10.3: every upload an earlier build queued gets a write id and is marked
    /// earlier-build; its base is left as it is. First (m-4c) the held columns of its file
    /// are cleared: a re-upgrade must not revive a token the system no longer holds.
    /// Nothing else changes: a pause, the attempts and the staged copy's path stay as they are.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn mark_earlier_build_uploads(&self) -> Result<usize> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE files SET held_write_id = NULL, held_base = NULL, held_version = NULL, held_object_version_id = NULL
             WHERE file_id IN (SELECT file_id FROM operation_queue
                               WHERE write_id IS NULL AND file_id IS NOT NULL
                                 AND kind IN ('upload_version', 'upload_file'))",
            [],
        )?;
        let ops: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT op_id FROM operation_queue
                 WHERE write_id IS NULL AND kind IN ('upload_version', 'upload_file') ORDER BY rowid",
            )?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>>>()?
        };
        for op_id in &ops {
            tx.execute(
                "UPDATE operation_queue SET write_id = ?2, write_origin = ?3 WHERE op_id = ?1",
                params![
                    op_id,
                    crate::write_token::mint_write_id(),
                    WriteOrigin::EarlierBuild.as_str()
                ],
            )?;
        }
        tx.commit()?;
        Ok(ops.len())
    }
}
