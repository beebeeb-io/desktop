//! File rows: upsert, lookup, status, content versions and contract state (split out of `state_db.rs`).

use super::*;

impl StateDb {
    /// Insert or update a row keyed by `file_id`. ON CONFLICT replaces
    /// the entire row except the primary key.
    pub fn upsert_file(&self, e: &FileEntry) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        upsert_file_conn(&conn, e)
    }
    /// Spec 2026-10-09 §7.2 (final review): insert or update a row and write its contract
    /// (parent, namespace, kind and the rest) in ONE transaction. Where a row is listed is
    /// decided by its parent alone, and a new row's columns default to no parent and the
    /// person's own namespace, so no reader may see a new row between the two writes. `update`
    /// gets the row's contract as it is after the upsert (a new row's: the column defaults);
    /// it runs under the database lock and must not call back into the database.
    pub fn upsert_file_with_contract(
        &self,
        entry: &FileEntry,
        update: impl FnOnce(&mut FileContractState),
    ) -> Result<()> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        upsert_file_conn(&tx, entry)?;
        let mut contract =
            get_file_contract_state_conn(&tx, &entry.file_id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
        update(&mut contract);
        set_file_contract_state_conn(&tx, &contract)?;
        tx.commit()
    }
    /// Task 2204 S0.3 (review-155): the pull's write of a server row, as
    /// [`Self::upsert_file_with_contract`], with the Windows-rename hold decided in the same
    /// transaction: while a `local_rename` op of the file is queued, the row keeps its current path
    /// and parent whatever `entry` and `update` say. Returns the path written.
    pub fn upsert_server_file_row(
        &self,
        entry: &FileEntry,
        update: impl FnOnce(&mut FileContractState),
    ) -> Result<String> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        // Only `handle_rename` (Windows) queues these ops, so other builds skip the query.
        let held: Option<(String, Option<String>)> = if cfg!(any(target_os = "windows", test)) {
            tx.query_row(
                "SELECT path, parent_id FROM files
                 WHERE file_id = ?1 AND item_kind = 'file'
                   AND EXISTS(SELECT 1 FROM operation_queue
                     WHERE file_id = ?1 AND kind IN ('rename_file', 'move_file')
                       AND CASE WHEN json_valid(metadata_json)
                           THEN json_extract(metadata_json, '$.local_rename') END = 1)",
                params![entry.file_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
        } else {
            None
        };
        let mut entry = entry.clone();
        if let Some((path, _)) = &held {
            entry.path = path.clone();
        }
        upsert_file_conn(&tx, &entry)?;
        let mut contract =
            get_file_contract_state_conn(&tx, &entry.file_id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
        update(&mut contract);
        if let Some((_, parent_id)) = held {
            contract.parent_id = parent_id;
        }
        set_file_contract_state_conn(&tx, &contract)?;
        tx.commit()?;
        Ok(entry.path)
    }
    /// Task 2191: `file_id` is a file of unknown content, whatever its status (the guarded set).
    pub fn is_unknown_content(&self, file_id: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM files WHERE file_id = ?1 AND {GUARDED_SQL})"),
            params![file_id],
            |r| r.get(0),
        )
    }
    /// Task 2191 (review-157 F7): `file_id` has agreed content recorded (`local_hash`).
    pub fn has_agreed_content(&self, file_id: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM files WHERE file_id = ?1 AND local_hash IS NOT NULL)",
            params![file_id],
            |r| r.get(0),
        )
    }
    /// Task 2191: a file of unknown content that is `local` or `cloud_only` (the population the
    /// check resolves; a conflict or an error is the person's to resolve).
    pub fn is_unknown_population(&self, file_id: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM files WHERE file_id = ?1 AND {GUARDED_SQL}
                   AND status IN ('local', 'cloud_only'))"
            ),
            params![file_id],
            |r| r.get(0),
        )
    }
    /// Task 2191: every file of unknown content (the start-up guard walks them all).
    #[cfg(any(target_os = "windows", test, feature = "test-support"))]
    pub fn list_guarded(&self) -> Result<Vec<GuardedRow>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(&format!(
            "SELECT file_id, path, status, fetched_min_version FROM files WHERE {GUARDED_SQL} ORDER BY rowid"
        ))?;
        let rows = stmt.query_map([], guarded_row)?;
        rows.collect()
    }
    /// Task 2191: a "not in sync" mark of `file_id` is still to be made (`true`) or done.
    pub fn set_mark_pending(&self, file_id: &str, pending: impl Into<MarkPending>) -> Result<()> {
        let pending: MarkPending = pending.into();
        let conn = self.0.lock().expect("state_db mutex poisoned");
        if pending.as_bool() {
            conn.execute(
                "INSERT INTO unknown_checks(file_id, mark_pending, updated_at)
                 VALUES (?1, 1, CAST(strftime('%s', 'now') AS INTEGER))
                 ON CONFLICT(file_id) DO UPDATE SET mark_pending = 1, updated_at = excluded.updated_at",
                params![file_id],
            )?;
        } else {
            // A cleared flag writes only a row that has one (no row per guarded file, self-review).
            conn.execute(
                "UPDATE unknown_checks SET mark_pending = 0 WHERE file_id = ?1 AND mark_pending = 1",
                params![file_id],
            )?;
        }
        Ok(())
    }
    /// Desktop 0.8.15: the files whose "not in sync" mark is pending now. The start-up pass reads
    /// them once, so a row that needs nothing gets no write.
    pub fn mark_pending_ids(&self) -> Result<HashSet<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare("SELECT file_id FROM unknown_checks WHERE mark_pending = 1")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect()
    }
    /// Task 2191: [`Self::mark_no_fetched_content`] for many rows, in one transaction (the
    /// start-up guard's empty placeholders). Returns how many were marked.
    pub fn mark_no_fetched_content_many(&self, file_ids: &[String]) -> Result<usize> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        let mut marked = 0;
        for file_id in file_ids {
            marked += tx.execute(
                "UPDATE files SET fetched_min_version = 0, fetched_hash = NULL
                 WHERE file_id = ?1 AND fetched_min_version = -1 AND local_hash IS NULL",
                params![file_id],
            )?;
        }
        tx.commit()?;
        Ok(marked)
    }
    /// Task 2191: the guarded rows whose mark is still to be made, at most `limit`, oldest first.
    /// A flag left on a row that is no longer guarded is cleared here.
    pub fn list_mark_pending(&self, limit: usize) -> Result<Vec<GuardedRow>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            &format!(
                "UPDATE unknown_checks SET mark_pending = 0
                 WHERE mark_pending = 1 AND NOT EXISTS (
                     SELECT 1 FROM files AS f WHERE f.file_id = unknown_checks.file_id AND {})",
                GUARDED_SQL
                    .replace("item_kind", "f.item_kind")
                    .replace("local_hash", "f.local_hash")
                    .replace("fetched_min_version", "f.fetched_min_version")
            ),
            [],
        )?;
        let mut stmt = conn.prepare(&format!(
            "SELECT f.file_id, f.path, f.status, f.fetched_min_version
             FROM unknown_checks AS u JOIN files AS f ON f.file_id = u.file_id
             WHERE u.mark_pending = 1 AND {}
             ORDER BY u.updated_at, u.file_id LIMIT ?1",
            GUARDED_SQL
                .replace("item_kind", "f.item_kind")
                .replace("local_hash", "f.local_hash")
                .replace("fetched_min_version", "f.fetched_min_version")
        ))?;
        let rows = stmt.query_map(params![limit as i64], guarded_row)?;
        rows.collect()
    }
    /// Task 2191 (D12): record a placeholder at `path` whose identity is `file_id`, a file of
    /// unknown content now at another path; `mark_pending` until its "not in sync" mark is made.
    pub fn put_unknown_orphan(&self, path: &str, file_id: &str, mark_pending: impl Into<MarkPending>) -> Result<()> {
        let mark_pending: MarkPending = mark_pending.into();
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "INSERT INTO unknown_orphans(path, file_id, mark_pending, updated_at)
             VALUES (?1, ?2, ?3, CAST(strftime('%s', 'now') AS INTEGER))
             ON CONFLICT(path) DO UPDATE SET file_id = ?2, mark_pending = ?3, updated_at = excluded.updated_at",
            params![path, file_id, mark_pending.as_bool()],
        )?;
        Ok(())
    }
    /// Task 2191 (D12): every recorded orphan: (path, the identity's file id, mark still pending).
    pub fn list_unknown_orphans(&self) -> Result<Vec<(String, String, bool)>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare("SELECT path, file_id, mark_pending FROM unknown_orphans ORDER BY path")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect()
    }
    /// Task 2191 (D12): forget the orphan at `path` (its placeholder is gone, or holds nothing).
    pub fn forget_unknown_orphan(&self, path: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute("DELETE FROM unknown_orphans WHERE path = ?1", params![path])?;
        Ok(())
    }
    /// Fetch a single row by `file_id`. `Ok(None)` if absent.
    pub fn get_file(&self, file_id: &str) -> Result<Option<FileEntry>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        get_file_conn(&conn, file_id)
    }
    /// Fetch a single row by its path inside the sync root. `Ok(None)`
    /// if absent. Used by OS virtual-filesystem layers that receive
    /// path/inode callbacks instead of server UUIDs.
    /// Look up a file row by its server-relative path.
    ///
    /// Stored `files.path` values come from `resolve_relative_path`, which can
    /// yield EITHER a leading-slash form (`/leaf`, e.g. when the row falls
    /// through to a server-provided plaintext `path` field) OR a bare relative
    /// form (`docs/a.txt`). The upload watcher (`watcher::relative_db_path`)
    /// always queries the bare form. An exact-only match would therefore MISS a
    /// row stored as `/leaf` when the watcher asks for `leaf`, causing the
    /// watcher to treat an already-tracked server file as brand-new and
    /// re-upload it. To stay robust to that shape mismatch we try the path as
    /// given first, then the leading-slash-toggled variant. This is a read-only
    /// defense-in-depth lookup — it never mutates state.
    pub fn get_file_by_path(&self, path: &str) -> Result<Option<FileEntry>> {
        if let Some(entry) = self.get_file_by_exact_path(path)? {
            return Ok(Some(entry));
        }
        // Toggle the leading slash and try once more: `/leaf` ⇄ `leaf`.
        let alt = match path.strip_prefix('/') {
            Some(stripped) => stripped.to_string(),
            None => format!("/{path}"),
        };
        if alt == path {
            return Ok(None);
        }
        self.get_file_by_exact_path(&alt)
    }
    /// Task 2229: a stamp of the rows [`Self::file_rows_at_path`] finds at `path`, over every column,
    /// so any write to any of them changes it; `None` without a row. Valid within this process only.
    /// Task 2271: every row, not only the first: when several share a path the scan may judge the
    /// bytes by another one.
    pub fn file_row_stamp_by_path(&self, path: &str) -> Result<Option<u64>> {
        use rusqlite::types::ValueRef;
        use std::hash::{Hash, Hasher};
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare_cached(FILE_ROW_BY_PATH_SQL)?;
        for candidate in [path.to_string(), toggled_slash(path)] {
            let mut rows = stmt.query(params![candidate])?;
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            let mut found = false;
            while let Some(row) = rows.next()? {
                found = true;
                for column in 0..row.as_ref().column_count() {
                    match row.get_ref(column)? {
                        ValueRef::Null => 0u8.hash(&mut hasher),
                        ValueRef::Integer(i) => (1u8, i).hash(&mut hasher),
                        ValueRef::Real(f) => (2u8, f.to_bits()).hash(&mut hasher),
                        ValueRef::Text(t) => (3u8, t).hash(&mut hasher),
                        ValueRef::Blob(b) => (4u8, b).hash(&mut hasher),
                    }
                }
            }
            if found {
                return Ok(Some(hasher.finish()));
            }
        }
        Ok(None)
    }
    /// Task 2271: every row at `path`, in the order [`Self::get_file_by_path`] meets them (its first
    /// is that one), in the stored slash shape it finds; empty without a row.
    pub fn file_rows_at_path(&self, path: &str) -> Result<Vec<FileEntry>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare_cached(FILE_BY_PATH_SQL)?;
        for candidate in [path.to_string(), toggled_slash(path)] {
            let entries = stmt
                .query_map(params![candidate], file_entry_from_path_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if !entries.is_empty() {
                return Ok(entries);
            }
        }
        Ok(Vec::new())
    }
    fn get_file_by_exact_path(&self, path: &str) -> Result<Option<FileEntry>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare_cached(FILE_BY_PATH_SQL)?;
        let mut rows = stmt.query(params![path])?;
        match rows.next()? {
            Some(row) => Ok(Some(file_entry_from_path_row(row)?)),
            None => Ok(None),
        }
    }
    /// Update just the `status` column for a known file. No-op if
    /// `file_id` doesn't exist; callers should pair with `upsert_file`
    /// when they want create-or-update semantics.
    pub fn set_status(&self, file_id: &str, status: FileStatus) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        // Task 1697: a status flip changes the item's metadata (isUploaded
        // gating etc.) — record it so the replica's metadata version moves.
        // A no-op flip records nothing (compare first).
        let old_status: Option<String> = conn
            .query_row("SELECT status FROM files WHERE file_id = ?1", params![file_id], |row| {
                row.get(0)
            })
            .optional()?;
        conn.execute(
            "UPDATE files SET status = ?1 WHERE file_id = ?2",
            params![status.as_str(), file_id],
        )?;
        if old_status.as_deref() != Some(status.as_str()) {
            record_file_change_conn(&conn, file_id, FpChangeKind::Modified, None)?;
        }
        Ok(())
    }
    /// [`Self::set_status`], only while the row's status is `expected`: a writer that set the
    /// status itself (a hydrate's `Downloading`) never writes over a status another writer set
    /// meanwhile (spec §9.2). One statement. A flip is recorded for the system. Returns whether
    /// the row changed.
    pub fn set_status_if(&self, file_id: &str, expected: FileStatus, status: FileStatus) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let changed = conn.execute(
            "UPDATE files SET status = ?3 WHERE file_id = ?1 AND status = ?2",
            params![file_id, expected.as_str(), status.as_str()],
        )? == 1;
        if changed && expected != status {
            record_file_change_conn(&conn, file_id, FpChangeKind::Modified, None)?;
        }
        Ok(changed)
    }
    /// §9.1 (m-8): a Finder write's attempt marks its row `Uploading`, except a `Trashing` row,
    /// which keeps its trash presentation. One statement, so a trash applied meanwhile is never
    /// overwritten. A flip is recorded for the system, as [`Self::set_status`] does.
    pub fn set_uploading_unless_trashing(&self, file_id: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let flipped = conn.execute(
            "UPDATE files SET status = 'uploading' WHERE file_id = ?1 AND status NOT IN ('uploading', 'trashing')",
            params![file_id],
        )?;
        if flipped == 1 {
            record_file_change_conn(&conn, file_id, FpChangeKind::Modified, None)?;
        }
        Ok(())
    }
    /// Task 1640 (review F1): bytes of a version nobody can name are (or may be) on disk: a
    /// download that named no version. The file's content is not known: never agreed, never sent.
    pub fn mark_content_unknown(&self, file_id: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "UPDATE files SET local_hash = NULL, fetched_min_version = -2, fetched_hash = NULL WHERE file_id = ?1",
            params![file_id],
        )?;
        Ok(())
    }
    /// Task 1640 (review F1): the file's placeholder was just created where nothing was: no
    /// fetched bytes. Only a row whose content is not known and was never fetched into since
    /// (`-1`; a fetch makes it `-2` before it transfers) is marked, so a fetch on the new
    /// placeholder is never forgotten. `true`: marked.
    #[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
    pub fn mark_no_fetched_content(&self, file_id: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Ok(conn.execute(
            "UPDATE files SET fetched_min_version = 0, fetched_hash = NULL
             WHERE file_id = ?1 AND fetched_min_version = -1 AND local_hash IS NULL",
            params![file_id],
        )? == 1)
    }
    /// Task 1640 (review N1): the file's candidate, `(version, hash)`; `hash` is `None` when
    /// mixed. `None`: no candidate.
    pub fn fetched_content(&self, file_id: &str) -> Result<Option<(i64, Option<String>)>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Ok(conn
            .query_row(
                "SELECT fetched_min_version, fetched_hash FROM files WHERE file_id = ?1",
                params![file_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()?
            .filter(|(version, _)| *version > 0))
    }
    /// Task 1640 (review F1): the placeholder's bytes may have changed in part without a fetch (a
    /// resize, a download cut off by a stop): its agreed content joins the candidate, which is then
    /// mixed: never agreed, a change sent on its version. Its version is kept (never reset).
    #[cfg(any(target_os = "windows", test, feature = "test-support"))]
    pub fn poison_local_content(&self, file_id: &str) -> Result<()> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        poison_local_content_conn(&tx, file_id)?;
        tx.commit()?;
        Ok(())
    }
    /// Task 1640 (review F1): the file moved on the server; its bytes did not. Its candidate is
    /// never promoted at the new place (it becomes mixed, its version kept); agreed content stays.
    pub fn poison_fetched_content(&self, file_id: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "UPDATE files SET fetched_hash = NULL WHERE file_id = ?1",
            params![file_id],
        )?;
        Ok(())
    }
    /// Task 1640 (review N1, condition 2): make the candidate the agreed content, only while the
    /// row is still `Local` with no agreed content and the candidate is still exactly
    /// `(version, hash)`, all in one statement. `false`: something changed; nothing was written.
    pub fn promote_fetched_content(&self, file_id: &str, version: i64, hash: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Ok(conn.execute(
            "UPDATE files SET local_base_version = ?2, local_hash = ?3, fetched_min_version = 0, fetched_hash = NULL
             WHERE file_id = ?1 AND status = 'local' AND local_hash IS NULL
               AND fetched_min_version = ?2 AND fetched_hash = ?3",
            params![file_id, version, hash],
        )? == 1)
    }
    /// Task 1640 (review P2-5): the `Local` files the stamp pass may mark in sync: a recorded
    /// agreed content and no sync-folder upload queued. Everything else is never marked, so it is
    /// filtered here, before the pass's per-tick cap.
    #[cfg(windows)]
    pub fn list_stamp_candidates(&self) -> Result<Vec<(String, String)>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT file_id, path FROM files
             WHERE status = 'local' AND item_kind = 'file' AND (local_hash IS NOT NULL OR fetched_hash IS NOT NULL)
               AND file_id NOT IN (SELECT file_id FROM operation_queue
                                   WHERE file_id IS NOT NULL AND write_id IS NULL
                                     AND kind IN ('upload_version', 'upload_file'))
             ORDER BY file_id",
        )?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect()
    }
    /// The full relative path of the item `name` under `parent_id` (spec §7.2; see
    /// `relative_path_under_conn`).
    pub fn relative_path_under(&self, parent_id: Option<&str>, name: &str) -> Result<String> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        relative_path_under_conn(&conn, parent_id, name)
    }
    /// Correct the logical plaintext `size_bytes` for a known file row.
    ///
    /// Used by the Windows Cloud Files hydration resolve-or-error path
    /// (task 0783): when a placeholder's recorded `size_bytes` (mirrored
    /// from the server) disagrees with the AES-256-GCM-authenticated
    /// decrypted plaintext length, the decrypted length is ground truth —
    /// so we rewrite the row to the true plaintext size. The next
    /// placeholder (re)creation then mints an aligned size and hydration
    /// succeeds. Returns the number of rows affected (0 if `file_id` is
    /// absent). A negative size is clamped to 0: a caller should never
    /// pass a negative length, but clamping keeps a pathological value
    /// from corrupting the column.
    #[cfg(any(target_os = "windows", test, feature = "test-support"))]
    pub fn set_size_bytes(&self, file_id: &str, size_bytes: i64) -> Result<usize> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let updated = conn.execute(
            "UPDATE files SET size_bytes = ?1 WHERE file_id = ?2",
            params![size_bytes.max(0), file_id],
        )?;
        // Task 1697: a size change moves the metadata version — record it.
        if updated > 0 {
            record_file_change_conn(&conn, file_id, FpChangeKind::Modified, None)?;
        }
        Ok(updated)
    }
    /// How many rows (files and folders) have `status`, counted in `idx_status` alone (task 2243). The windows poll it
    /// every few seconds; loading the rows to count them read the whole set from disk each time.
    pub fn count_by_status(&self, status: FileStatus) -> Result<u32> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let count: i64 = conn
            .prepare_cached("SELECT COUNT(*) FROM files WHERE status = ?1")?
            .query_row(params![status.as_str()], |row| row.get(0))?;
        Ok(u32::try_from(count.max(0)).unwrap_or(u32::MAX))
    }
    /// Return every row whose `status` matches. Used by the conflict
    /// resolution UI (status = Conflict) and the daemon's
    /// "what still needs uploading?" sweep (status = Uploading).
    pub fn list_by_status(&self, status: FileStatus) -> Result<Vec<FileEntry>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT file_id, path, status, size_bytes, modified_at, content_hash, remote_updated_at,
                    parent_id, item_kind
             FROM files WHERE status = ?1",
        )?;
        let rows = stmt.query_map(params![status.as_str()], |row| {
            Ok(FileEntry {
                file_id: row.get(0)?,
                path: row.get(1)?,
                status: FileStatus::from_str(&row.get::<_, String>(2)?),
                size_bytes: row.get(3)?,
                modified_at: row.get(4)?,
                content_hash: row.get(5)?,
                remote_updated_at: row.get(6)?,
                parent_id: row.get(7)?,
                item_kind: ItemKind::from_str(&row.get::<_, String>(8)?),
            })
        })?;
        rows.collect()
    }
    /// Return every tracked file. Used by virtual filesystem directory
    /// enumeration to expose known cloud-only and local files.
    pub fn list_files(&self) -> Result<Vec<FileEntry>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT file_id, path, status, size_bytes, modified_at, content_hash, remote_updated_at,
                    parent_id, item_kind
             FROM files ORDER BY path ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(FileEntry {
                file_id: row.get(0)?,
                path: row.get(1)?,
                status: FileStatus::from_str(&row.get::<_, String>(2)?),
                size_bytes: row.get(3)?,
                modified_at: row.get(4)?,
                content_hash: row.get(5)?,
                remote_updated_at: row.get(6)?,
                parent_id: row.get(7)?,
                item_kind: ItemKind::from_str(&row.get::<_, String>(8)?),
            })
        })?;
        rows.collect()
    }
    /// Return every tracked file paired with its **effective-pinned** flag —
    /// the per-PC sync-state lens behind the in-app "Files" tab
    /// (`account_dto::compute_file_overview`).
    ///
    /// This exists alongside [`Self::list_files`] because `FileEntry` does not
    /// carry pin state: pinning lives on the `pin_state` / `inherited_pin_state`
    /// columns owned by [`FileContractState`]. Rather than widen `FileEntry`
    /// (and every caller of it), this method SELECTs those two extra columns and
    /// computes `pinned` with the SAME predicate as
    /// [`FileContractState::effective_pin_state`]: the row's own `pin_state` if
    /// it is set (`pinned` / `unpinned`), otherwise the `inherited_pin_state` —
    /// pinned only when that resolves to [`PinState::Pinned`].
    pub fn file_overview_rows(&self) -> Result<Vec<(FileEntry, bool)>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(&format!("SELECT {FILE_OVERVIEW_COLUMNS} FROM files ORDER BY path ASC"))?;
        let rows = stmt.query_map([], file_overview_row)?;
        rows.collect()
    }
    /// The newest `limit` files (folders left out), newest first and ties by path, in the row shape of
    /// [`StateDb::file_overview_rows`]: the tray's recent list (task 2243). `compute_file_overview` sorts by the same
    /// keys, so its newest `limit` files are among these. Read through `idx_files_modified_at`, not by loading every row.
    pub fn newest_file_rows(&self, limit: usize) -> Result<Vec<(FileEntry, bool)>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT {FILE_OVERVIEW_COLUMNS} FROM files WHERE item_kind != 'folder'
             ORDER BY modified_at DESC, path ASC LIMIT ?1"
        ))?;
        let rows = stmt.query_map(params![i64::try_from(limit).unwrap_or(i64::MAX)], file_overview_row)?;
        rows.collect()
    }
    /// Tests only (spec §7.2, final review): production writes a row's contract together with
    /// the row, through [`StateDb::upsert_file_with_contract`].
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_file_contract_state(&self, state: &FileContractState) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        set_file_contract_state_conn(&conn, state)
    }
    pub fn get_file_contract_state(&self, file_id: &str) -> Result<Option<FileContractState>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        get_file_contract_state_conn(&conn, file_id)
    }
    /// The predicate's inputs and the status override's, from one locked read
    /// (spec §5.3 "One read", §8.7 S1.9).
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub fn item_presentation(&self, file_id: &str) -> Result<Option<ItemPresentation>> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        let Some(contract) = get_file_contract_state_conn(&tx, file_id)? else {
            return Ok(None);
        };
        let (held, _version_filled) = held_write_conn(&tx, file_id)?;
        let held_write_queued = match &held {
            Some(held) => tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM operation_queue WHERE write_id = ?1)",
                params![held.write_id],
                |row| row.get(0),
            )?,
            None => false,
        };
        let (unparked_finder_upload, parked_finder_upload): (bool, bool) = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM operation_queue
                            WHERE file_id = ?1 AND write_id IS NOT NULL
                              AND kind IN ('upload_version', 'upload_file')
                              AND attempts < max_attempts),
                    EXISTS(SELECT 1 FROM operation_queue
                            WHERE file_id = ?1 AND write_id IS NOT NULL
                              AND kind IN ('upload_version', 'upload_file')
                              AND attempts >= max_attempts)",
            params![file_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        tx.commit()?;
        Ok(Some(ItemPresentation {
            contract,
            held,
            held_write_queued,
            unparked_finder_upload,
            parked_finder_upload,
            #[cfg(any(test, feature = "test-support"))]
            version_filled: _version_filled,
        }))
    }
    /// Task 2190 (plan §3.3, D4): the leaf names of the rows in `parent_id`'s folder (the root when
    /// `None`), lowercased: a Mac compares names case-insensitively. A create that has not landed
    /// has its row already (the accept inserts it), so it counts.
    pub fn child_names_lowercase(&self, parent_id: Option<&str>) -> Result<std::collections::HashSet<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare("SELECT path FROM files WHERE parent_id IS ?1")?;
        let names = stmt
            .query_map(params![parent_id], |row| row.get::<_, String>(0))?
            .filter_map(|path| path.ok())
            .map(|path| path.rsplit('/').next().unwrap_or_default().to_lowercase())
            .collect();
        Ok(names)
    }
    // ── File Provider change log + sync anchor (task 1697) ────────────────────
    //
    // The replica's enumerator (`NSFileProviderReplicatedExtension`) needs a
    // daemon-side change cursor to answer `enumerateChanges(for:from:)` and
    // `currentSyncAnchor`. Every mutating row operation funnels through
    // `record_file_change`, which appends to `fp_changes` and advances the
    // persistent `fp_sync_anchor` cursor. Anchors are the change-log rowid:
    // strictly ascending, compact (decimal ASCII ≤ 500 bytes for any realistic
    // history), stable across a no-op poll, and durable across process death
    // (they live in state.db, next to the log they point into).
    /// Append one change to the log and advance the persistent anchor cursor.
    /// `old_parent_id` is only meaningful for [`FpChangeKind::Reparented`] (and
    /// informational elsewhere); the NEW parent is read from the current
    /// `files` row when it exists (for `Deleted` it was captured by the caller
    /// BEFORE `delete_file` — see `delete_file_for_fp`).
    #[cfg(any(test, feature = "test-support"))]
    pub fn record_file_change(&self, file_id: &str, kind: FpChangeKind, old_parent_id: Option<String>) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        record_file_change_conn(&conn, file_id, kind, old_parent_id)
    }
    /// Number of live rows whose contract parent is `folder_id` (task 1697:
    /// the File Provider `childItemCount`). Counts every row regardless of
    /// status — a folder's cloud-only children still show in Finder.
    pub fn child_item_count(&self, folder_id: &str) -> Result<i64> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            "SELECT COUNT(*) FROM files WHERE parent_id = ?1",
            params![folder_id],
            |row| row.get(0),
        )
    }
    /// Task 2445: the children of `parent_id` (or root when `None`), not trashing,
    /// as `(FileEntry, pinned)` pairs, using `idx_files_parent_id`. Replaces the
    /// `live_rows` full-table scan for `model::list`.
    pub fn children_of(&self, parent_id: Option<&str>) -> Result<Vec<(FileEntry, bool)>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT {FILE_OVERVIEW_COLUMNS} FROM files
             WHERE parent_id IS ?1 AND status != 'trashing'
             ORDER BY item_kind ASC, path ASC"
        ))?;
        let rows = stmt.query_map(params![parent_id], file_overview_row)?;
        rows.collect()
    }
    /// P2-5 (task 2445): a paged listing with contract joined in SQL. Returns
    /// `(entry, contract, child_count)` for the children of `parent_id`, with
    /// LIMIT/OFFSET paging, plus the total count. Child counts for folders are
    /// joined with a correlated subquery so no per-row query is needed.
    pub fn children_paged(
        &self,
        parent_id: Option<&str>,
        offset: i64,
        limit: i64,
    ) -> Result<(Vec<(FileEntry, FileContractState, i64)>, i64)> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let total: i64 = conn.query_row(
            "SELECT COUNT(*) FROM files WHERE parent_id IS ?1 AND status != 'trashing'",
            params![parent_id],
            |r| r.get(0),
        )?;
        let mut stmt = conn.prepare_cached(
            "SELECT file_id, path, status, size_bytes, modified_at, content_hash, remote_updated_at,
                    parent_id, item_kind, pin_state, inherited_pin_state,
                    namespace, shared_root_id, share_id, permission_bits,
                    content_type, current_version, current_object_version_id,
                    local_base_version, local_hash, cache_path, cache_bytes,
                    last_sync_at, owner_email,
                    (SELECT COUNT(*) FROM files AS c WHERE c.parent_id = files.file_id AND c.status != 'trashing')
             FROM files
             WHERE parent_id IS ?1 AND status != 'trashing'
             ORDER BY item_kind ASC, path ASC
             LIMIT ?2 OFFSET ?3",
        )?;
        let rows = stmt.query_map(params![parent_id, limit, offset], |row| {
            let entry = FileEntry {
                file_id: row.get(0)?,
                path: row.get(1)?,
                status: FileStatus::from_str(&row.get::<_, String>(2)?),
                size_bytes: row.get(3)?,
                modified_at: row.get(4)?,
                content_hash: row.get(5)?,
                remote_updated_at: row.get(6)?,
                parent_id: row.get(7)?,
                item_kind: ItemKind::from_str(&row.get::<_, String>(8)?),
            };
            let pin_state = PinState::from_str(&row.get::<_, String>(9)?);
            let inherited_pin_state = PinState::from_str(&row.get::<_, String>(10)?);
            let contract = FileContractState {
                file_id: entry.file_id.clone(),
                namespace: Namespace::from_str(&row.get::<_, String>(11)?),
                parent_id: entry.parent_id.clone(),
                shared_root_id: row.get(12)?,
                share_id: row.get(13)?,
                owner_email: row.get(23)?,
                permission_bits: row.get(14)?,
                item_kind: entry.item_kind.clone(),
                content_type: row.get(15)?,
                current_version: row.get(16)?,
                current_object_version_id: row.get(17)?,
                local_base_version: row.get(18)?,
                local_hash: row.get(19)?,
                cache_path: row.get(20)?,
                cache_bytes: row.get(21)?,
                pin_state,
                inherited_pin_state,
                last_sync_at: row.get(22)?,
            };
            let child_count: i64 = row.get(24)?;
            Ok((entry, contract, child_count))
        })?;
        Ok((rows.collect::<Result<Vec<_>>>()?, total))
    }
    pub fn list_contract_states_by_namespace(&self, namespace: Namespace) -> Result<Vec<FileContractState>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT file_id, namespace, parent_id, shared_root_id, share_id, permission_bits,
                    item_kind, content_type, current_version, current_object_version_id,
                    local_base_version, local_hash, cache_path, cache_bytes, pin_state,
                    inherited_pin_state, last_sync_at, owner_email
             FROM files WHERE namespace = ?1 ORDER BY path ASC",
        )?;
        let rows = stmt.query_map(params![namespace.as_str()], |row| {
            Ok(FileContractState {
                file_id: row.get(0)?,
                namespace: Namespace::from_str(&row.get::<_, String>(1)?),
                parent_id: row.get(2)?,
                shared_root_id: row.get(3)?,
                share_id: row.get(4)?,
                owner_email: row.get(17)?,
                permission_bits: row.get(5)?,
                item_kind: ItemKind::from_str(&row.get::<_, String>(6)?),
                content_type: row.get(7)?,
                current_version: row.get(8)?,
                current_object_version_id: row.get(9)?,
                local_base_version: row.get(10)?,
                local_hash: row.get(11)?,
                cache_path: row.get(12)?,
                cache_bytes: row.get(13)?,
                pin_state: PinState::from_str(&row.get::<_, String>(14)?),
                inherited_pin_state: PinState::from_str(&row.get::<_, String>(15)?),
                last_sync_at: row.get(16)?,
            })
        })?;
        rows.collect()
    }
    pub fn purge_revoked_shared_content(&self, active_shared_root_ids: &[String]) -> Result<Vec<RevokedSharedCache>> {
        let active: HashSet<&str> = active_shared_root_ids.iter().map(String::as_str).collect();
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        let candidates = {
            let mut stmt = tx.prepare(
                "SELECT file_id, shared_root_id, cache_path
                 FROM files
                 WHERE namespace = 'shared_with_me'",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?;
            rows.collect::<Result<Vec<_>>>()?
        };

        let mut revoked = Vec::new();
        for (file_id, shared_root_id, cache_path) in candidates {
            let root_id = shared_root_id.as_deref().unwrap_or(file_id.as_str());
            if active.contains(root_id) {
                continue;
            }
            // The ops' staged copies are released in this transaction, so the next start's
            // release path unlinks them (spec §8.7 S6). Before, the op went and its plaintext
            // copy stayed until a sign-out. macOS only: the release journal is read only there.
            #[cfg(target_os = "macos")]
            tx.execute(
                "INSERT INTO staged_payloads(path, completed)
                 SELECT payload_path, 1 FROM operation_queue WHERE file_id = ?1 AND payload_path IS NOT NULL
                 ON CONFLICT(path) DO UPDATE SET completed = 1",
                params![file_id],
            )?;
            tx.execute("DELETE FROM operation_queue WHERE file_id = ?1", params![file_id])?;
            tx.execute("DELETE FROM files WHERE file_id = ?1", params![file_id])?;
            revoked.push(RevokedSharedCache { file_id, cache_path });
        }
        drop_orphaned_upload_resumes(&tx)?;
        tx.commit()?;
        Ok(revoked)
    }
    /// Every `files` row as [`TreeRow`], for the Selective sync tree (0.8.15). Changes nothing.
    pub fn list_tree_rows(&self) -> Result<Vec<TreeRow>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT file_id, path, status, size_bytes, parent_id, item_kind, current_version, remote_updated_at
             FROM files ORDER BY path ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(TreeRow {
                file_id: row.get(0)?,
                path: row.get(1)?,
                status: FileStatus::from_str(&row.get::<_, String>(2)?),
                size_bytes: row.get(3)?,
                parent_id: row.get(4)?,
                item_kind: ItemKind::from_str(&row.get::<_, String>(5)?),
                current_version: row.get(6)?,
                remote_updated_at: row.get(7)?,
            })
        })?;
        rows.collect()
    }
}
