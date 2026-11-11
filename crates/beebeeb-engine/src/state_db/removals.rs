//! Trashing, pruning, removed folders and the kept copies of a conflict (split out of `state_db.rs`).

use super::*;

impl StateDb {
    /// The runner deletes rows only inside a transaction (`delete_file_conn`, the landing);
    /// tests set up states with this.
    #[cfg(any(test, feature = "test-support"))]
    pub fn delete_file(&self, file_id: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        delete_file_conn(&conn, file_id)
    }
    /// Delete the row `file_id` AND — when it is a FOLDER — its whole descendant
    /// subtree by PATH-PREFIX, returning every removed row as a [`PrunedRow`]
    /// ordered CHILDREN-BEFORE-PARENT (deepest path first, then the root last).
    /// Task 0806: the OPS reconcile path (`apply_sync_op` for `file_trash` /
    /// `file_delete`) uses this so a remotely-trashed FOLDER also prunes its
    /// orphaned children (the server trash is NOT recursive — task 0807 — so a
    /// `file_trash` op for a folder is the ONLY signal its children are gone) and
    /// so the Windows caller can remove each on-disk placeholder, leaves first.
    ///
    /// The descendant match is the same metacharacter-safe `substr(path,…) = R || '/'`
    /// prefix equality used by [`Self::prune_absent`] and
    /// [`Self::cloud_only_file_descendants`] — `parent_id` is universally empty on
    /// live rows, so the hierarchy is path-based. A FILE row (or an unknown id)
    /// simply removes the single row. Returns an empty vec if the id is unknown.
    /// One transaction so a concurrent reader never sees a half-pruned subtree.
    /// Task 1698 (trash ruling — full sync): mark `file_id` and, for a folder,
    /// its whole path-prefix subtree as `Trashing` — the local mirror of the
    /// server's trash (`DELETE /files/{id}` sets `is_trashed=TRUE`; the
    /// server trash is the ONE trash model). Every flipped row records a
    /// `Modified` change so the replica moves it into the trash container
    /// (the payload builder parents top-of-trash rows there). Rows already
    /// `Trashing` are left untouched (no duplicate changes). Returns the
    /// affected rows (path + kind, `PrunedRow` shape) so the caller can still
    /// remove on-disk placeholders the way the delete path did.
    pub fn mark_subtree_trashing(&self, file_id: &str) -> Result<Vec<PrunedRow>> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;

        // Same shape as `delete_file_subtree`: the row itself, plus (for a
        // folder) every descendant matched by PATH PREFIX — children always
        // stored leading-slash-free, the folder possibly leading-slash-first
        // (task 0806). Deepest-first ordering keeps the returned rows in the
        // children-before-parent order the placeholder remover expects.
        let root: Option<PrunedRow> = {
            let mut stmt = tx.prepare("SELECT file_id, path, item_kind, local_hash, CASE WHEN fetched_min_version > 0 THEN fetched_hash END FROM files WHERE file_id = ?1")?;
            let mut rows = stmt.query(params![file_id])?;
            if let Some(row) = rows.next()? {
                Some(PrunedRow {
                    file_id: row.get(0)?,
                    path: row.get(1)?,
                    is_dir: ItemKind::from_str(&row.get::<_, String>(2)?) == ItemKind::Folder,
                    local_hash: row.get(3)?,
                    fetched_hash: row.get(4)?,
                })
            } else {
                None
            }
        };
        let Some(root) = root else {
            tx.rollback()?;
            return Ok(Vec::new());
        };

        let mut marked: Vec<PrunedRow> = Vec::new();
        let mark = |tx: &rusqlite::Connection, row: &PrunedRow| -> Result<bool> {
            // Flip only when it changes anything: a no-op flip must not mint
            // a duplicate change row (same contract as `set_status`).
            let status: String = tx
                .query_row(
                    "SELECT status FROM files WHERE file_id = ?1",
                    params![row.file_id],
                    |r| r.get(0),
                )
                .optional()?
                .unwrap_or_default();
            if status == FileStatus::Trashing.as_str() {
                return Ok(false);
            }
            tx.execute(
                "UPDATE files SET status = ?1 WHERE file_id = ?2",
                params![FileStatus::Trashing.as_str(), row.file_id],
            )?;
            record_file_change_conn(&tx, &row.file_id, FpChangeKind::Modified, None)?;
            Ok(true)
        };

        if root.is_dir {
            let root_path = root.path.trim_matches('/').to_string();
            if !root_path.is_empty() {
                let descendants: Vec<PrunedRow> = {
                    let mut dstmt = tx.prepare(
                        "SELECT file_id, path, item_kind, local_hash, CASE WHEN fetched_min_version > 0 THEN fetched_hash END FROM files
                         WHERE namespace = 'my_files'
                           AND substr(ltrim(path, '/'), 1, length(?1) + 1) = ?1 || '/'
                         ORDER BY length(path) DESC, path DESC",
                    )?;
                    let drows = dstmt.query_map(params![root_path], |r| {
                        Ok(PrunedRow {
                            file_id: r.get(0)?,
                            path: r.get(1)?,
                            is_dir: ItemKind::from_str(&r.get::<_, String>(2)?) == ItemKind::Folder,
                            local_hash: r.get(3)?,
                            fetched_hash: r.get(4)?,
                        })
                    })?;
                    drows.collect::<Result<Vec<_>>>()?
                };
                for d in descendants {
                    if mark(&tx, &d)? {
                        marked.push(d);
                    }
                }
            }
        }
        if mark(&tx, &root)? {
            marked.push(root);
        }
        tx.commit()?;
        Ok(marked)
    }
    /// PR #100 review (Codex P1): the trash view's inverse of
    /// [`Self::mark_subtree_trashing`] — a restored FOLDER must take its
    /// whole held subtree out of the trash view, not just the root row (the
    /// authoritative re-snapshot preserves `Trashing` rows by design, so it
    /// can never repair the descendants). Flips every `Trashing` row in the
    /// subtree (the row itself plus, for a folder, every descendant matched
    /// by PATH PREFIX — same shape as `mark_subtree_trashing`) to `CloudOnly`,
    /// records a `Modified` change per actual flip, and returns the flipped
    /// rows deepest-first (children before parent). Rows not in `Trashing`
    /// are left untouched; a no-op flip records no change row. Same subtree
    /// matching contract as `mark_subtree_trashing` (children always stored
    /// leading-slash-free, the folder possibly leading-slash-first, task
    /// 0806), and the same caller contract: the caller removes on-disk
    /// placeholders and reports the returned ids for the working-set signal.
    pub fn untrash_subtree(&self, file_id: &str) -> Result<Vec<PrunedRow>> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;

        let root: Option<PrunedRow> = {
            let mut stmt = tx.prepare("SELECT file_id, path, item_kind, local_hash, CASE WHEN fetched_min_version > 0 THEN fetched_hash END FROM files WHERE file_id = ?1")?;
            let mut rows = stmt.query(params![file_id])?;
            if let Some(row) = rows.next()? {
                Some(PrunedRow {
                    file_id: row.get(0)?,
                    path: row.get(1)?,
                    is_dir: ItemKind::from_str(&row.get::<_, String>(2)?) == ItemKind::Folder,
                    local_hash: row.get(3)?,
                    fetched_hash: row.get(4)?,
                })
            } else {
                None
            }
        };
        let Some(root) = root else {
            tx.rollback()?;
            return Ok(Vec::new());
        };

        let mut flipped: Vec<PrunedRow> = Vec::new();
        let flip = |tx: &rusqlite::Connection, row: &PrunedRow| -> Result<bool> {
            // Flip only Trashing rows: the restore must not touch live rows
            // that happen to sit in the subtree (e.g. a child the local
            // flow never parked), and a no-op must not mint a change row.
            let status: String = tx
                .query_row(
                    "SELECT status FROM files WHERE file_id = ?1",
                    params![row.file_id],
                    |r| r.get(0),
                )
                .optional()?
                .unwrap_or_default();
            if status != FileStatus::Trashing.as_str() {
                return Ok(false);
            }
            tx.execute(
                "UPDATE files SET status = ?1 WHERE file_id = ?2",
                params![FileStatus::CloudOnly.as_str(), row.file_id],
            )?;
            record_file_change_conn(&tx, &row.file_id, FpChangeKind::Modified, None)?;
            Ok(true)
        };

        if root.is_dir {
            let root_path = root.path.trim_matches('/').to_string();
            if !root_path.is_empty() {
                let descendants: Vec<PrunedRow> = {
                    let mut dstmt = tx.prepare(
                        "SELECT file_id, path, item_kind, local_hash, CASE WHEN fetched_min_version > 0 THEN fetched_hash END FROM files
                         WHERE namespace = 'my_files'
                           AND substr(ltrim(path, '/'), 1, length(?1) + 1) = ?1 || '/'
                         ORDER BY length(path) DESC, path DESC",
                    )?;
                    let drows = dstmt.query_map(params![root_path], |r| {
                        Ok(PrunedRow {
                            file_id: r.get(0)?,
                            path: r.get(1)?,
                            is_dir: ItemKind::from_str(&r.get::<_, String>(2)?) == ItemKind::Folder,
                            local_hash: r.get(3)?,
                            fetched_hash: r.get(4)?,
                        })
                    })?;
                    drows.collect::<Result<Vec<_>>>()?
                };
                for d in descendants {
                    if flip(&tx, &d)? {
                        flipped.push(d);
                    }
                }
            }
        }
        if flip(&tx, &root)? {
            flipped.push(root);
        }
        tx.commit()?;
        Ok(flipped)
    }
    pub fn delete_file_subtree(&self, file_id: &str) -> Result<Vec<PrunedRow>> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;

        // Look up the row to delete (its path + kind drives the subtree prune).
        let root: Option<PrunedRow> = {
            let mut stmt = tx.prepare("SELECT file_id, path, item_kind, local_hash, CASE WHEN fetched_min_version > 0 THEN fetched_hash END FROM files WHERE file_id = ?1")?;
            let mut rows = stmt.query(params![file_id])?;
            if let Some(row) = rows.next()? {
                Some(PrunedRow {
                    file_id: row.get(0)?,
                    path: row.get(1)?,
                    is_dir: ItemKind::from_str(&row.get::<_, String>(2)?) == ItemKind::Folder,
                    local_hash: row.get(3)?,
                    fetched_hash: row.get(4)?,
                })
            } else {
                None
            }
        };
        let Some(root) = root else {
            tx.rollback()?;
            return Ok(Vec::new());
        };

        let mut removed: Vec<PrunedRow> = Vec::new();
        // Task 1697: capture each removed row's parent BEFORE its delete —
        // the change log must tell the replica's materialized filter where
        // each deleted item lived. (parent_id read per row inside the tx.)
        let old_parent_of = |tx: &rusqlite::Transaction, file_id: &str| -> Option<String> {
            tx.query_row(
                "SELECT parent_id FROM files WHERE file_id = ?1",
                params![file_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .ok()
            .flatten()
            .flatten()
        };
        // Slice 58: a removed row must not be named by a queued operation any more. The
        // reconcile path enforces the same invariant (nothing names a dropped row); a
        // terminal-failed op would otherwise keep the row in `waiting_rows` forever.
        let drop_ops = |tx: &rusqlite::Transaction, file_id: &str| -> rusqlite::Result<()> {
            tx.execute(
                "DELETE FROM upload_resume WHERE op_id IN (SELECT op_id FROM operation_queue WHERE file_id = ?1)",
                params![file_id],
            )?;
            tx.execute("DELETE FROM operation_queue WHERE file_id = ?1", params![file_id])?;
            Ok(())
        };
        if root.is_dir {
            // Normalize BOTH ends (`trim_matches`), matching `placeholder_path_under`
            // / `safe_join_under_root`: a folder row can be stored leading-slash-
            // first (`/docs`) on the degraded-decrypt path while its children are
            // ALWAYS stored leading-slash-free (`docs/a.txt`). Trimming only the
            // trailing slash would make the prefix `/docs/` miss `docs/a.txt` and
            // orphan the children (task 0806 review, high). The descendant match
            // also `ltrim`s the stored path so a mixed-form child in either shape
            // is caught.
            let root_path = root.path.trim_matches('/').to_string();
            if !root_path.is_empty() {
                let descendants: Vec<PrunedRow> = {
                    let mut dstmt = tx.prepare(
                        "SELECT file_id, path, item_kind, local_hash, CASE WHEN fetched_min_version > 0 THEN fetched_hash END FROM files
                         WHERE substr(ltrim(path, '/'), 1, length(?1) + 1) = ?1 || '/'
                         ORDER BY length(path) DESC, path DESC",
                    )?;
                    let drows = dstmt.query_map(params![root_path], |r| {
                        Ok(PrunedRow {
                            file_id: r.get(0)?,
                            path: r.get(1)?,
                            is_dir: ItemKind::from_str(&r.get::<_, String>(2)?) == ItemKind::Folder,
                            local_hash: r.get(3)?,
                            fetched_hash: r.get(4)?,
                        })
                    })?;
                    drows.collect::<Result<Vec<_>>>()?
                };
                for d in descendants {
                    // Spec §7.2 (final review): a row with a queued or parked Finder write is
                    // kept; it is re-homed below only if its own folder is gone.
                    if has_finder_write_conn(&tx, &d.file_id)? {
                        continue;
                    }
                    let old_parent = old_parent_of(&tx, &d.file_id);
                    tx.execute("DELETE FROM files WHERE file_id = ?1", params![d.file_id])?;
                    drop_ops(&tx, &d.file_id)?;
                    record_file_change_conn(&tx, &d.file_id, FpChangeKind::Deleted, old_parent)?;
                    removed.push(d);
                }
            }
        }

        // Spec §7.2 (final review): the deleted item itself, with a queued or parked Finder write,
        // is kept where it is: the edit wins, and the save re-creates the file under its id.
        if !has_finder_write_conn(&tx, &root.file_id)? {
            let root_old_parent = old_parent_of(&tx, &root.file_id);
            tx.execute("DELETE FROM files WHERE file_id = ?1", params![root.file_id])?;
            drop_ops(&tx, &root.file_id)?;
            record_file_change_conn(&tx, &root.file_id, FpChangeKind::Deleted, root_old_parent)?;
            removed.push(root);
        }
        rehome_orphaned_writes_conn(&tx)?;
        tx.commit()?;
        Ok(removed)
    }
    /// Append a local-only activity event and prune old rows. This table is the
    /// durable recent-activity surface for events whose canonical file row may
    /// legitimately disappear from `files` during sync convergence.
    /// Task 1640 (review N9, N13): record a kept copy; its id.
    pub fn record_kept_copy(&self, copy: &KeptCopyInput<'_>) -> Result<i64> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "INSERT INTO kept_copies (original_file_id, original_path, copy_path, copied_hash, original_removed, reason, kept_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                copy.original_file_id,
                copy.original_path,
                copy.copy_path,
                copy.copied_hash,
                copy.original_removed,
                copy.reason.as_str(),
                copy.kept_at
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }
    /// Task 1640 (review F6): kept copy `id` now says `copy` (the retry resolved it): one entry
    /// per file, updated where it is.
    pub fn update_kept_copy(&self, id: i64, copy: &KeptCopyInput<'_>) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "UPDATE kept_copies SET original_file_id = ?2, original_path = ?3, copy_path = ?4, copied_hash = ?5,
                                    original_removed = ?6, reason = ?7, kept_at = ?8
             WHERE id = ?1",
            params![
                id,
                copy.original_file_id,
                copy.original_path,
                copy.copy_path,
                copy.copied_hash,
                copy.original_removed,
                copy.reason.as_str(),
                copy.kept_at
            ],
        )?;
        Ok(())
    }
    /// Task 1640 (review F6): kept entry `id` is resolved with nothing kept (the file was removed
    /// with no copy): its entry goes.
    pub fn delete_kept_copy(&self, id: i64) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute("DELETE FROM kept_copies WHERE id = ?1", params![id])?;
        Ok(())
    }
    /// Task 1640: every kept copy not dismissed, newest first.
    pub fn list_kept_copies(&self) -> Result<Vec<KeptCopy>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(&format!(
            "SELECT {KEPT_COPY_COLUMNS} FROM kept_copies WHERE dismissed = 0 ORDER BY kept_at DESC, id DESC"
        ))?;
        let rows = stmt.query_map([], kept_copy_from_row)?;
        rows.collect()
    }
    /// Task 1640 (review F3): folder `rel_path` was deleted elsewhere but stays on disk for what
    /// it still holds. It is never created again on the server from this PC.
    pub fn record_removed_folder(&self, rel_path: &str, now: i64) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "INSERT INTO removed_folders (path, removed_at) VALUES (?1, ?2) ON CONFLICT(path) DO NOTHING",
            params![rel_path, now],
        )?;
        Ok(())
    }
    /// Task 1640 (review F3): every folder deleted elsewhere that is still on disk, deepest first.
    pub fn list_removed_folders(&self) -> Result<Vec<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare("SELECT path FROM removed_folders ORDER BY length(path) DESC, path DESC")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect()
    }
    /// Task 1640 (review F3): folder `rel_path` is gone from disk, or the server lists it again.
    pub fn forget_removed_folder(&self, rel_path: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute("DELETE FROM removed_folders WHERE path = ?1", params![rel_path])?;
        Ok(())
    }
    /// Task 1640 (review F3): `rel_path` is a folder deleted elsewhere that stays on disk.
    pub fn is_removed_folder(&self, rel_path: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM removed_folders WHERE path = ?1)",
            params![rel_path],
            |row| row.get(0),
        )
    }
    /// Task 1640 (review F3): the outermost folder deleted elsewhere that is `rel_path` or holds it.
    pub fn outermost_removed_folder(&self, rel_path: &str) -> Result<Option<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            "SELECT path FROM removed_folders
             WHERE path = ?1 OR substr(?1, 1, length(path) + 1) = path || '/'
             ORDER BY length(path) LIMIT 1",
            params![rel_path],
            |row| row.get(0),
        )
        .optional()
    }
    /// Task 1640 (review N9, F7): the person dismissed kept copy `id` (its review entry). The copy
    /// itself stays, and so does a file kept where it is. `false`: no such entry, or its original
    /// still waits to be removed by a later tick ([`KeptCopy::dismissable`]).
    pub fn dismiss_kept_copy(&self, id: i64) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let kept = conn
            .query_row(
                &format!("SELECT {KEPT_COPY_COLUMNS} FROM kept_copies WHERE id = ?1 AND dismissed = 0"),
                params![id],
                kept_copy_from_row,
            )
            .optional()?;
        let Some(kept) = kept.filter(KeptCopy::dismissable) else {
            return Ok(false);
        };
        // Review-151d P2-1: a file still kept where it is leaves a tombstone: its folder stays the
        // server's deletion (never created again from this PC while it holds only that file).
        let changed = if kept.original_removed {
            conn.execute("DELETE FROM kept_copies WHERE id = ?1", params![id])?
        } else {
            conn.execute("UPDATE kept_copies SET dismissed = 1 WHERE id = ?1", params![id])?
        };
        Ok(changed == 1)
    }
    /// Sweep and delete every descendant of `folder_id` from the DB when the
    /// folder's OWN row is ABSENT (the "ghost children" bug, task 0828).
    ///
    /// ## When is this needed?
    ///
    /// `delete_file_subtree` (used by the normal `file_trash`/`file_delete`
    /// reconcile path) first looks up the folder's own row to obtain its `path`,
    /// then prunes descendants via a PATH-PREFIX query.  When the folder row is
    /// absent — because the folder was already trashed on the server when this
    /// desktop's snapshot was taken, so the snapshot excluded it, but its
    /// CHILDREN were already ingested (stored with bare leaf-name paths) — there
    /// is no path to start from, so `delete_file_subtree` returns an empty vec
    /// and the children are never removed.  They then show as permanent ghost
    /// placeholders in Explorer.
    ///
    /// ## How this fixes it
    ///
    /// The snapshot ingest path (`apply_metadata_file_row`) calls
    /// `set_file_contract_state` right after `upsert_file`, which writes the
    /// server's `parent_id` into `files.parent_id`.  So even when the parent
    /// FOLDER row is absent, its children carry `files.parent_id = folder_id`
    /// and can be found by a direct column scan — which is exactly what this
    /// function does.
    ///
    /// For any found child that is itself a folder we additionally remove its
    /// descendants via the same PATH-PREFIX sweep `delete_file_subtree` uses,
    /// so multi-level subtrees are fully pruned.
    ///
    /// All removals happen in a SINGLE transaction so a concurrent reader
    /// never sees a half-pruned subtree.  Returns the removed rows ordered
    /// CHILDREN-BEFORE-PARENTS (deepest path first) so the Windows caller can
    /// remove leaf placeholders before their containing directory placeholder.
    /// Returns an empty vec if no children are found (idempotent).
    ///
    /// ## Scope guarantee
    ///
    /// Only rows whose `parent_id` column equals `folder_id` (and their
    /// path-prefix descendants) are ever touched.  No broader match is possible:
    /// the query is `WHERE parent_id = ?1` with a single bound parameter.
    pub fn delete_orphaned_children_of_absent_folder(&self, folder_id: &str) -> Result<Vec<PrunedRow>> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;

        // Find every direct child whose parent_id matches the absent folder.
        // Using `parent_id` (not path-prefix) because we have no path to start
        // from — that is precisely the condition that triggered this call.
        // Task 1698: `Trashing` children are NOT swept — they are the server
        // trash's content (each has its own `file_trash` op converging it
        // into the trash view); deleting them would erase trash-view rows.
        let direct_children: Vec<PrunedRow> = {
            let mut stmt =
                tx.prepare("SELECT file_id, path, item_kind, local_hash, CASE WHEN fetched_min_version > 0 THEN fetched_hash END FROM files WHERE parent_id = ?1 AND status != 'trashing'")?;
            let rows = stmt.query_map(params![folder_id], |r| {
                Ok(PrunedRow {
                    file_id: r.get(0)?,
                    path: r.get(1)?,
                    is_dir: ItemKind::from_str(&r.get::<_, String>(2)?) == ItemKind::Folder,
                    local_hash: r.get(3)?,
                    fetched_hash: r.get(4)?,
                })
            })?;
            rows.collect::<Result<Vec<_>>>()?
        };

        if direct_children.is_empty() {
            tx.rollback()?;
            return Ok(Vec::new());
        }

        // For the children-before-parents ordering we collect: deep descendants
        // first (for each child folder, via path-prefix), then the direct
        // children themselves (folders after their leaves).
        let mut removed: Vec<PrunedRow> = Vec::new();

        for child in &direct_children {
            if child.is_dir {
                // Remove the folder's descendants first (children-before-parent
                // ordering within this subtree).  Mirror the path-prefix sweep
                // in `delete_file_subtree`.
                let child_path = child.path.trim_matches('/').to_string();
                if !child_path.is_empty() {
                    let descendants: Vec<PrunedRow> = {
                        let mut dstmt = tx.prepare(
                            "SELECT file_id, path, item_kind, local_hash, CASE WHEN fetched_min_version > 0 THEN fetched_hash END FROM files
                             WHERE substr(ltrim(path, '/'), 1, length(?1) + 1) = ?1 || '/'
                             ORDER BY length(path) DESC, path DESC",
                        )?;
                        let drows = dstmt.query_map(params![child_path], |r| {
                            Ok(PrunedRow {
                                file_id: r.get(0)?,
                                path: r.get(1)?,
                                is_dir: ItemKind::from_str(&r.get::<_, String>(2)?) == ItemKind::Folder,
                                local_hash: r.get(3)?,
                                fetched_hash: r.get(4)?,
                            })
                        })?;
                        drows.collect::<Result<Vec<_>>>()?
                    };
                    for d in descendants {
                        // Spec §7.2 (final review): kept, as in the other sweeps.
                        if has_finder_write_conn(&tx, &d.file_id)? {
                            continue;
                        }
                        tx.execute("DELETE FROM files WHERE file_id = ?1", params![d.file_id])?;
                        removed.push(d);
                    }
                }
            }
        }

        // Delete the direct children and append them last (after their own
        // descendants) to maintain children-before-parent ordering. Spec §7.2 (final
        // review): a child with a queued or parked Finder write is kept, then re-homed at the
        // top level with every write whose folder is gone.
        for child in direct_children {
            if has_finder_write_conn(&tx, &child.file_id)? {
                continue;
            }
            tx.execute("DELETE FROM files WHERE file_id = ?1", params![child.file_id])?;
            removed.push(child);
        }
        rehome_orphaned_writes_conn(&tx)?;

        tx.commit()?;
        Ok(removed)
    }
    /// Reconcile a fresh `/sync/snapshot` against the local mirror: delete every
    /// OWN-tree row whose `file_id` is NOT in `seen_file_ids` (the snapshot is
    /// authoritative for the user's non-trashed tree, so an absent row was
    /// deleted/trashed server-side). Returns the deleted rows as [`PrunedRow`]s
    /// (file_id + path + is_dir) so the caller can locate and remove each row's
    /// on-disk Cloud Files placeholder + cache — NOT just the bare file_ids
    /// (task 0806: the snapshot path used to only LOG the pruned ids, leaving the
    /// placeholder ghosting in Explorer).
    ///
    /// ## Orphan-subtree removal (no recursive server trash — task 0807)
    ///
    /// The server's trash is NOT recursive: trashing a FOLDER removes only the
    /// folder node from the snapshot; its CHILDREN remain present in `seen`. So a
    /// pruned folder's descendants are NOT caught by the snapshot-absence test
    /// above — they'd be left orphaned (rows + placeholders pointing at a deleted
    /// parent). For every pruned FOLDER row we therefore additionally remove its
    /// DESCENDANTS by PATH-PREFIX (the hierarchy is path-based — `parent_id` is
    /// universally empty on live rows, see [`Self::cloud_only_file_descendants`]),
    /// using the same metacharacter-safe `substr(path,…) = R || '/'` prefix
    /// equality. Descendants are returned in the result too, ordered
    /// CHILDREN-BEFORE-PARENTS (deepest path first) so the Windows caller can
    /// remove leaf placeholders before their containing directory placeholder.
    /// Descendants are removed even if they appear in `seen` (an orphan whose
    /// trashed parent is gone is itself gone).
    ///
    /// ## What is intentionally NEVER pruned
    ///
    /// 1. **Shared rows** (`namespace != 'my_files'`). The snapshot only returns
    ///    the user's OWN tree; shared-with-me content has its own purge path
    ///    ([`Self::purge_revoked_shared_content`]). Pruning here would wrongly
    ///    nuke every shared file on the first snapshot.
    /// 2. **Rows with a pending operation** (`file_id` present in
    ///    `operation_queue`). A locally-created-but-not-yet-uploaded file lives
    ///    in the mirror under a CLIENT-minted UUID (re-keyed to the server id by
    ///    `defer_local_upload_finalization` only AFTER the upload completes),
    ///    and is referenced by its `operation_queue` row. That client UUID is
    ///    NOT in the server snapshot's `seen` set, so without this guard the very
    ///    next snapshot would delete the user's in-flight upload. The
    ///    `operation_queue` join is the authoritative "do not touch" signal.
    /// 3. **Rows still `uploading`** — belt-and-suspenders for the same in-flight
    ///    case, in the narrow window where the queue row was already consumed but
    ///    the re-key hasn't landed.
    ///
    /// 4. **Rows stamped at/after `snapshot_fetched_at`** — a row whose
    ///    `remote_updated_at >= snapshot_fetched_at` was touched locally (e.g. a
    ///    just-completed upload re-keyed to the server id via
    ///    the landing, `apply_landing`, which stamps `remote_updated_at = now`) AT OR
    ///    AFTER the snapshot was taken, so the server snapshot legitimately
    ///    predates it and CANNOT be authoritative about its existence. Pruning it
    ///    would delete a freshly-uploaded file whenever the server snapshot lags
    ///    the upload by a tick. The re-keyed server row is `Local` and no longer
    ///    has an `operation_queue` entry (the queue row is keyed on the old client
    ///    UUID and removed by `process_due_operations`), so guards (2)/(3) miss
    ///    it — this freshness cutoff is what protects it.
    ///
    /// ## Empty-snapshot safety
    ///
    /// An EMPTY `seen` set against a NON-empty prunable own-tree is treated as a
    /// suspicious/degraded snapshot (server bug, degraded replica, a future
    /// paginated response with no `has_more` contract) and is REFUSED — we return
    /// `Ok(vec![])` without deleting anything. Refusing costs at worst a stale row
    /// for one tick; wrongly pruning on a spurious empty snapshot destroys the
    /// user's whole tree in one transaction. A genuinely-emptied vault converges
    /// the moment the next non-empty snapshot (or the per-row trash/delete ops)
    /// arrives, so this fail-closed choice loses no correctness, only immediacy.
    ///
    /// Done in ONE transaction so a concurrent reader never sees a half-pruned
    /// tree.
    pub fn prune_absent(&self, seen_file_ids: &HashSet<String>, snapshot_fetched_at: i64) -> Result<Vec<PrunedRow>> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        // Candidate set: own-tree rows that are NOT pending an upload/op, not
        // mid-upload, and NOT stamped at/after the snapshot fetch time (a row a
        // recent local completion just touched can't be contradicted by an older
        // snapshot). Everything else is off-limits per the doc above. We now also
        // pull `path` + `item_kind` so the caller can locate the on-disk
        // placeholder and (for a folder) we can prune its orphaned descendants.
        //
        // Task 1698 (trash ruling): `Trashing` rows are the macOS Trash view
        // and the snapshot NEVER lists trashed files — absence is now their
        // steady state, not a convergence signal. Rows whose PARENT row is
        // `Trashing` are that trash view's folder content (same server trash)
        // and are equally protected. Removal converges through the server's
        // `file_delete` (permanent) / `file_restore` ops instead.
        let candidates: Vec<PrunedRow> = {
            let mut stmt = tx.prepare(
                "SELECT file_id, path, item_kind, local_hash, CASE WHEN fetched_min_version > 0 THEN fetched_hash END FROM files
                 WHERE namespace = 'my_files'
                   AND status != 'uploading'
                   AND status != 'trashing'
                   AND remote_updated_at < ?1
                   AND (parent_id IS NULL OR parent_id NOT IN
                        (SELECT file_id FROM files WHERE status = 'trashing'))
                   AND file_id NOT IN (
                       SELECT file_id FROM operation_queue WHERE file_id IS NOT NULL
                   )",
            )?;
            let rows = stmt.query_map(params![snapshot_fetched_at], |row| {
                Ok(PrunedRow {
                    file_id: row.get(0)?,
                    path: row.get(1)?,
                    is_dir: ItemKind::from_str(&row.get::<_, String>(2)?) == ItemKind::Folder,
                    local_hash: row.get(3)?,
                    fetched_hash: row.get(4)?,
                })
            })?;
            rows.collect::<Result<Vec<_>>>()?
        };

        // Empty-snapshot guard: if the snapshot says "nothing exists" but we DO
        // have prunable own-tree rows, the snapshot is almost certainly degraded
        // (empty 200, replica lag, or an unannounced pagination cut). Fail closed
        // — refuse to prune the entire tree on a single suspicious empty list.
        if seen_file_ids.is_empty() && !candidates.is_empty() {
            tx.rollback()?;
            tracing::warn!(
                local_prunable = candidates.len(),
                "prune_absent: refusing to prune — empty snapshot against a non-empty own-tree (suspected degraded snapshot)"
            );
            return Ok(Vec::new());
        }

        // Track everything removed (directly absent + orphaned descendants) so we
        // never delete or return a row twice (a pruned folder and an
        // independently-absent child both reaching the descendant sweep).
        let mut removed_ids: HashSet<String> = HashSet::new();
        let mut pruned: Vec<PrunedRow> = Vec::new();

        for row in candidates {
            if seen_file_ids.contains(&row.file_id) {
                continue;
            }
            if !removed_ids.insert(row.file_id.clone()) {
                continue; // already removed as a descendant of an earlier folder
            }

            // ORPHAN SUBTREE (task 0806/0807): a trashed FOLDER leaves its children
            // in the snapshot (no recursive server trash), so they won't be caught
            // by the absence test. Remove the folder's descendants by PATH-PREFIX
            // FIRST (children-before-parent) and emit them ahead of the folder so
            // the Windows caller removes leaf placeholders before the directory.
            if row.is_dir {
                // `trim_matches` (both ends), matching `placeholder_path_under`:
                // a folder stored leading-slash-first (`/docs`, degraded-decrypt
                // path) has children stored leading-slash-free (`docs/a.txt`), so
                // trimming only the trailing slash would orphan them (task 0806
                // review, high). The match `ltrim`s the stored path so a child in
                // either shape is caught.
                let root_path = row.path.trim_matches('/').to_string();
                if !root_path.is_empty() {
                    let descendants: Vec<PrunedRow> = {
                        // Strict descendants: `substr(ltrim(path,'/'),1,len(R)+1)
                        // = R || '/'` — metacharacter-safe prefix equality (NOT
                        // `LIKE`), the same machinery as `cloud_only_file_descendants`.
                        // Excludes the root row itself. Deepest-first so children
                        // precede their parent dirs in the returned order. Descendants
                        // with a pending op / mid-upload are NOT excluded here: their
                        // parent is gone server-side, so the orphan must go too —
                        // any stale queued op against it is moot. Spec §7.2 (final
                        // review): except a queued or parked Finder write, whose bytes
                        // are the person's: its row is kept, and re-homed at the top
                        // level after the deletes when its own folder is gone. Task 1698:
                        // `Trashing` descendants are NOT excluded here by a pending
                        // op, but ARE excluded by STATUS — they are the server
                        // trash's content (recoverable), not dead rows; deleting
                        // them would erase the trash view's folder contents.
                        let mut dstmt = tx.prepare(
                            "SELECT file_id, path, item_kind, local_hash, CASE WHEN fetched_min_version > 0 THEN fetched_hash END FROM files
                             WHERE namespace = 'my_files'
                               AND status != 'trashing'
                               AND substr(ltrim(path, '/'), 1, length(?1) + 1) = ?1 || '/'
                             ORDER BY length(path) DESC, path DESC",
                        )?;
                        let drows = dstmt.query_map(params![root_path], |r| {
                            Ok(PrunedRow {
                                file_id: r.get(0)?,
                                path: r.get(1)?,
                                is_dir: ItemKind::from_str(&r.get::<_, String>(2)?) == ItemKind::Folder,
                                local_hash: r.get(3)?,
                                fetched_hash: r.get(4)?,
                            })
                        })?;
                        drows.collect::<Result<Vec<_>>>()?
                    };
                    for d in descendants {
                        // Spec §7.2 (final review): kept; re-homed below only if its own
                        // folder is gone.
                        if has_finder_write_conn(&tx, &d.file_id)? {
                            continue;
                        }
                        if removed_ids.insert(d.file_id.clone()) {
                            let old_parent = tx
                                .query_row(
                                    "SELECT parent_id FROM files WHERE file_id = ?1",
                                    params![d.file_id],
                                    |row| row.get::<_, Option<String>>(0),
                                )
                                .optional()
                                .ok()
                                .flatten()
                                .flatten();
                            tx.execute("DELETE FROM files WHERE file_id = ?1", params![d.file_id])?;
                            record_file_change_conn(&tx, &d.file_id, FpChangeKind::Deleted, old_parent)?;
                            pruned.push(d);
                        }
                    }
                }
            }

            let old_parent = tx
                .query_row(
                    "SELECT parent_id FROM files WHERE file_id = ?1",
                    params![row.file_id],
                    |row2| row2.get::<_, Option<String>>(0),
                )
                .optional()
                .ok()
                .flatten()
                .flatten();
            tx.execute("DELETE FROM files WHERE file_id = ?1", params![row.file_id])?;
            record_file_change_conn(&tx, &row.file_id, FpChangeKind::Deleted, old_parent)?;
            pruned.push(row);
        }
        rehome_orphaned_writes_conn(&tx)?;
        tx.commit()?;
        Ok(pruned)
    }
}
