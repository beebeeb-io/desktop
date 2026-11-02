//! `conn` items split out of `state_db.rs`.

use super::*;

/// [`StateDb::upsert_file`] on an open connection or transaction.
pub(super) fn upsert_file_conn<C: std::ops::Deref<Target = Connection>>(conn: &C, e: &FileEntry) -> Result<()> {
    // Task 1697: capture the OLD row before the upsert so the change log
    // can (a) tell created from modified, (b) skip no-op re-upserts — the
    // metadata sweeps re-upsert every row every tick, and recording every
    // one would advance the anchor with no real change — and (c) carry
    // the old parent for reparent detection.
    let old: Option<(String, String, i64, i64, i64, Option<String>)> = conn
        .query_row(
            "SELECT path, status, size_bytes, modified_at, remote_updated_at, parent_id
             FROM files WHERE file_id = ?1",
            params![e.file_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            },
        )
        .optional()?;
    conn.execute(
        "INSERT INTO files (file_id, path, status, size_bytes, modified_at, content_hash, remote_updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(file_id) DO UPDATE SET
           path=excluded.path, status=excluded.status,
           size_bytes=excluded.size_bytes, modified_at=excluded.modified_at,
           content_hash=excluded.content_hash,
           remote_updated_at=excluded.remote_updated_at",
        params![
            e.file_id,
            e.path,
            e.status.as_str(),
            e.size_bytes,
            e.modified_at,
            e.content_hash,
            e.remote_updated_at
        ],
    )?;
    let changed = match &old {
        None => true,
        Some((path, status, size, modified_at, remote_updated_at, _)) => {
            path != &e.path
                || status != e.status.as_str()
                || *size != e.size_bytes
                || *modified_at != e.modified_at
                || *remote_updated_at != e.remote_updated_at
        }
    };
    if changed {
        // A path change is a rename OR a move; Reparented carries the old
        // parent so the materialized-set filter can test old-OR-new
        // (Apple's Replicated contract). Created/Modified need no old
        // parent.
        let (kind, old_parent): (FpChangeKind, Option<String>) = match &old {
            None => (FpChangeKind::Created, None),
            Some((old_path, _, _, _, _, old_parent)) => {
                if old_path != &e.path {
                    (FpChangeKind::Reparented, old_parent.clone())
                } else {
                    (FpChangeKind::Modified, None)
                }
            }
        };
        record_file_change_conn(conn, &e.file_id, kind, old_parent)?;
    }
    Ok(())
}

/// [`StateDb::get_file`] on an open connection or transaction.
pub(super) fn get_file_conn<C: std::ops::Deref<Target = Connection>>(conn: &C, file_id: &str) -> Result<Option<FileEntry>> {
    let mut stmt = conn.prepare(
        "SELECT file_id, path, status, size_bytes, modified_at, content_hash, remote_updated_at,
                parent_id, item_kind
         FROM files WHERE file_id = ?1",
    )?;
    let mut rows = stmt.query(params![file_id])?;
    if let Some(row) = rows.next()? {
        Ok(Some(FileEntry {
            file_id: row.get(0)?,
            path: row.get(1)?,
            status: FileStatus::from_str(&row.get::<_, String>(2)?),
            size_bytes: row.get(3)?,
            modified_at: row.get(4)?,
            content_hash: row.get(5)?,
            remote_updated_at: row.get(6)?,
            parent_id: row.get(7)?,
            item_kind: ItemKind::from_str(&row.get::<_, String>(8)?),
        }))
    } else {
        Ok(None)
    }
}

/// [`StateDb::record_local_write`] on an open connection or transaction.
pub(super) fn record_local_write_conn<C: std::ops::Deref<Target = Connection>>(
    conn: &C,
    file_id: &str,
    size_bytes: i64,
    modified_at: i64,
) -> Result<()> {
    let old: Option<(String, i64, i64)> = conn
        .query_row(
            "SELECT status, size_bytes, modified_at FROM files WHERE file_id = ?1",
            params![file_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((status, old_size, old_modified_at)) = old else {
        return Ok(());
    };
    conn.execute(
        "UPDATE files SET status = ?1, size_bytes = ?2, modified_at = ?3 WHERE file_id = ?4",
        params![FileStatus::Uploading.as_str(), size_bytes, modified_at, file_id],
    )?;
    if status != FileStatus::Uploading.as_str() || old_size != size_bytes || old_modified_at != modified_at {
        record_file_change_conn(conn, file_id, FpChangeKind::Modified, None)?;
    }
    Ok(())
}

/// Spec §7.2 (final review): the full relative path of the item `name` under `parent_id`, as a
/// snapshot writes it: the parent row's path, `/`, the name. The name alone at the root, and
/// when the parent row is unknown or has no path. Where an item is listed is decided by its
/// parent alone; the path carries its name, and its folders for the readers that use them.
pub(super) fn relative_path_under_conn<C: std::ops::Deref<Target = Connection>>(
    conn: &C,
    parent_id: Option<&str>,
    name: &str,
) -> Result<String> {
    let name = path_leaf(name);
    let parent_path: Option<String> = match parent_id {
        Some(parent_id) => conn
            .query_row("SELECT path FROM files WHERE file_id = ?1", params![parent_id], |row| {
                row.get(0)
            })
            .optional()?,
        None => None,
    };
    Ok(match parent_path.as_deref().map(|path| path.trim_matches('/')) {
        Some(parent_path) if !parent_path.is_empty() => format!("{parent_path}/{name}"),
        _ => name.to_string(),
    })
}

/// Spec §7.2 (final review): `file_id` has a queued or parked File Provider write (an upload
/// with a write id). The path sweeps never delete such a row.
pub(super) fn has_finder_write_conn<C: std::ops::Deref<Target = Connection>>(conn: &C, file_id: &str) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM operation_queue
                        WHERE file_id = ?1 AND write_id IS NOT NULL AND kind IN ('upload_version', 'upload_file'))",
        params![file_id],
        |row| row.get(0),
    )
}

/// Whether a sign-out closed Finder accepts, read inside the caller's transaction (spec 2026-10-10 §15 question 3).
pub(super) fn finder_accepts_closed_conn(conn: &Connection) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sync_state WHERE key = ?1)",
        params![FINDER_ACCEPTS_CLOSED_KEY],
        |row| row.get(0),
    )
}

/// [`StateDb::enqueue_operation`]'s insert on an open connection or transaction: a new op, or an op with the same id
/// replaced.
/// Queue `op`, or replace the op with its id. Task 2267 (self-review F3): an op acting on an item whose changes are held
/// for the person's decision (`found_while_paused`: the item itself, or the folder it is in) is held with them, and a
/// held op queued again stays held, so nothing of that item is sent before the person decides.
pub(super) fn insert_operation_conn(conn: &Connection, op: &PendingOperation) -> Result<()> {
    conn.execute(
        "INSERT INTO operation_queue (
                op_id, kind, file_id, parent_id, target_path, metadata_json, payload_path,
                base_version, base_object_version_id, attempts, max_attempts, next_retry_at,
                last_error, last_error_class, paused_reason, backup_source_key, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, NULL,
                CASE WHEN EXISTS (SELECT 1 FROM operation_queue held
                                  WHERE held.paused_reason = ?17 AND held.file_id IN (?3, ?4))
                     THEN ?17 END,
                ?14, ?15, ?16)
             ON CONFLICT(op_id) DO UPDATE SET
                kind = excluded.kind,
                file_id = excluded.file_id,
                parent_id = excluded.parent_id,
                target_path = excluded.target_path,
                metadata_json = excluded.metadata_json,
                payload_path = excluded.payload_path,
                base_version = excluded.base_version,
                base_object_version_id = excluded.base_object_version_id,
                attempts = excluded.attempts,
                max_attempts = excluded.max_attempts,
                next_retry_at = excluded.next_retry_at,
                last_error = excluded.last_error,
                last_error_class = excluded.last_error_class,
                paused_reason = CASE WHEN operation_queue.paused_reason = ?17 THEN ?17 ELSE excluded.paused_reason END,
                backup_source_key = excluded.backup_source_key,
                updated_at = excluded.updated_at",
        params![
            op.op_id,
            op.kind.as_str(),
            op.file_id,
            op.parent_id,
            op.target_path,
            op.metadata_json,
            op.payload_path,
            op.base_version,
            op.base_object_version_id,
            op.attempts,
            op.max_attempts,
            op.next_retry_at,
            op.last_error,
            op.backup_source_key,
            op.created_at,
            op.updated_at,
            OperationPauseReason::FoundWhilePaused.as_str()
        ],
    )?;
    Ok(())
}

/// Spec §7.2 (final review): the path sweeps' last step, in their transaction, after their
/// deletes. Each row with a queued or parked File Provider write whose parent no longer names a
/// row is re-homed at the top level ([`rehome_kept_row_conn`]). A row whose folder is still
/// there is left exactly as it is: a path prefix also reaches the rows of another folder with
/// the same path (one in the trash, or two of the same name), and those are not orphaned.
pub(super) fn rehome_orphaned_writes_conn<C: std::ops::Deref<Target = Connection>>(conn: &C) -> Result<()> {
    let orphans: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT DISTINCT f.file_id FROM files AS f
             JOIN operation_queue AS o ON o.file_id = f.file_id
             WHERE o.write_id IS NOT NULL AND o.kind IN ('upload_version', 'upload_file')
               AND f.parent_id IS NOT NULL
               AND NOT EXISTS(SELECT 1 FROM files AS p WHERE p.file_id = f.parent_id)",
        )?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>>>()?
    };
    for file_id in orphans {
        rehome_kept_row_conn(conn, &file_id)?;
    }
    Ok(())
}

/// Spec §7.2 (final review): a row a path sweep kept, because a File Provider write of it is
/// queued or parked, whose folder is gone ([`rehome_orphaned_writes_conn`]). It is re-homed at
/// the top level in the sweep's transaction: no parent, its name alone as the path, one
/// `reparented` change, and its writes' parent cleared. A create then lands at the top level; a
/// save sends the move there with its bytes (`parent_changed`). The row stays listed, at the
/// root, throughout.
pub(super) fn rehome_kept_row_conn<C: std::ops::Deref<Target = Connection>>(conn: &C, file_id: &str) -> Result<()> {
    let Some(row) = get_file_conn(conn, file_id)? else {
        return Ok(());
    };
    conn.execute(
        "UPDATE files SET parent_id = NULL, path = ?2 WHERE file_id = ?1",
        params![file_id, path_leaf(&row.path)],
    )?;
    let writes: Vec<(String, Option<String>)> = {
        let mut stmt = conn.prepare(
            "SELECT op_id, metadata_json FROM operation_queue
             WHERE file_id = ?1 AND write_id IS NOT NULL AND kind IN ('upload_version', 'upload_file')",
        )?;
        let rows = stmt.query_map(params![file_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<Vec<_>>>()?
    };
    for (op_id, metadata_json) in writes {
        // A save (not a create, which its `init` places) records the move, so its metadata step
        // sends the top level.
        let metadata_json = metadata_json.map(|raw| match serde_json::from_str::<serde_json::Value>(&raw) {
            Ok(mut metadata) if metadata["operation"].as_str() != Some("create_file") && metadata.is_object() => {
                metadata["parent_changed"] = serde_json::json!(true);
                metadata.to_string()
            }
            _ => raw,
        });
        conn.execute(
            "UPDATE operation_queue SET parent_id = NULL, metadata_json = ?2 WHERE op_id = ?1",
            params![op_id, metadata_json],
        )?;
    }
    record_file_change_conn(conn, file_id, FpChangeKind::Reparented, row.parent_id)
}

/// Spec §7.2 (final review): whether a queued File Provider save of `file_id`, parked or not,
/// recorded that it moved the file.
pub(super) fn queued_save_moved_conn<C: std::ops::Deref<Target = Connection>>(conn: &C, file_id: &str) -> Result<bool> {
    let mut stmt = conn.prepare(
        "SELECT metadata_json FROM operation_queue
         WHERE file_id = ?1 AND write_id IS NOT NULL AND kind IN ('upload_version', 'upload_file')",
    )?;
    let rows = stmt.query_map(params![file_id], |row| row.get::<_, Option<String>>(0))?;
    for raw in rows {
        let moved = raw?
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .and_then(|metadata| metadata.get("parent_changed").and_then(serde_json::Value::as_bool));
        if moved == Some(true) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The name and parent a File Provider save was made under, applied in its accept (spec §7.2).
/// The system hands a rename or a move over with the contents and takes the reply's item as
/// the truth for both, so the row describes them from the accept on; a snapshot or an op echo
/// leaves an `Uploading` or `Error` row's name alone. The path is the full relative path under
/// the new parent. Nothing is written when neither changed.
pub(super) fn apply_save_name_and_parent_conn<C: std::ops::Deref<Target = Connection>>(
    conn: &C,
    entry: &FileEntry,
    target_path: Option<&str>,
    parent_id: Option<&str>,
) -> Result<()> {
    let Some(target_path) = target_path else {
        return Ok(());
    };
    let moved = entry.parent_id.as_deref() != parent_id;
    if path_leaf(&entry.path) == path_leaf(target_path) && !moved {
        return Ok(());
    }
    let path = relative_path_under_conn(conn, parent_id, target_path)?;
    conn.execute(
        "UPDATE files SET path = ?2, parent_id = ?3 WHERE file_id = ?1",
        params![entry.file_id, path, parent_id],
    )?;
    // A rename or a move, with the old parent, as `upsert_file_conn` records a path change.
    record_file_change_conn(conn, &entry.file_id, FpChangeKind::Reparented, entry.parent_id.clone())
}

/// [`StateDb::delete_file`] on an open connection or transaction.
/// Task 2267, "Keep on this PC only": the queued operations acting on the rows `ids` (their own row or the folder they
/// act in), and the staged copies they hold (payload and resume paths), sorted, once each.
#[cfg(any(target_os = "windows", test))]
pub(super) fn keep_ops_conn(conn: &Connection, ids: &[String]) -> Result<(Vec<String>, Vec<String>)> {
    let mut ops: Vec<(String, Option<String>)> = Vec::new();
    let mut stmt = conn.prepare("SELECT op_id, payload_path FROM operation_queue WHERE file_id = ?1 OR parent_id = ?1")?;
    for id in ids {
        for op in stmt.query_map(params![id], |row| Ok((row.get(0)?, row.get(1)?)))? {
            let op = op?;
            if !ops.iter().any(|(op_id, _)| *op_id == op.0) {
                ops.push(op);
            }
        }
    }
    let mut staged: Vec<String> = Vec::new();
    for (op_id, payload) in &ops {
        let resumed: Option<String> = conn
            .query_row("SELECT payload_path FROM upload_resume WHERE op_id = ?1", params![op_id], |row| row.get(0))
            .optional()?;
        staged.extend(payload.iter().chain(resumed.iter()).filter(|path| !path.is_empty()).cloned());
    }
    staged.sort();
    staged.dedup();
    Ok((ops.into_iter().map(|(op_id, _)| op_id).collect(), staged))
}

pub(super) fn delete_file_conn<C: std::ops::Deref<Target = Connection>>(conn: &C, file_id: &str) -> Result<()> {
    // Task 1697: capture the parent BEFORE the delete — the change row is
    // what tells the replica's materialized filter where the item was.
    let old_parent: Option<String> = conn
        .query_row(
            "SELECT parent_id FROM files WHERE file_id = ?1",
            params![file_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    conn.execute("DELETE FROM files WHERE file_id = ?1", params![file_id])?;
    record_file_change_conn(conn, file_id, FpChangeKind::Deleted, old_parent)?;
    Ok(())
}

/// [`StateDb::set_file_contract_state`] on an open connection or transaction.
pub(super) fn set_file_contract_state_conn<C: std::ops::Deref<Target = Connection>>(
    conn: &C,
    state: &FileContractState,
) -> Result<()> {
    // Task 1697: the contract write is what moves an item BETWEEN
    // containers (parent change) — record a reparent with the old parent
    // so the materialized filter can test old-or-new. Other contract
    // metadata changes (kind, current_version, content_type) record a
    // plain modified. A no-op write records nothing.
    let old: Option<(Option<String>, String, String, i64)> = conn
        .query_row(
            "SELECT parent_id, item_kind, content_type, current_version
             FROM files WHERE file_id = ?1",
            params![state.file_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()?;
    conn.execute(
        "UPDATE files SET
           namespace = ?2,
           parent_id = ?3,
           shared_root_id = ?4,
           share_id = ?5,
           permission_bits = ?6,
           item_kind = ?7,
           content_type = ?8,
           current_version = ?9,
           current_object_version_id = ?10,
           local_base_version = ?11,
           local_hash = ?12,
           cache_path = ?13,
           cache_bytes = ?14,
           pin_state = ?15,
           inherited_pin_state = ?16,
           last_sync_at = ?17,
           owner_email = ?18
         WHERE file_id = ?1",
        params![
            state.file_id,
            state.namespace.as_str(),
            state.parent_id,
            state.shared_root_id,
            state.share_id,
            state.permission_bits,
            state.item_kind.as_str(),
            state.content_type,
            state.current_version,
            state.current_object_version_id,
            state.local_base_version,
            state.local_hash,
            state.cache_path,
            state.cache_bytes,
            state.pin_state.as_str(),
            state.inherited_pin_state.as_str(),
            state.last_sync_at,
            state.owner_email,
        ],
    )?;
    if let Some((old_parent, old_kind, old_content_type, old_version)) = old {
        let parent_changed = old_parent != state.parent_id;
        let metadata_changed = old_kind != state.item_kind.as_str()
            || old_content_type != state.content_type.clone().unwrap_or_default()
            || old_version != state.current_version;
        if parent_changed {
            record_file_change_conn(conn, &state.file_id, FpChangeKind::Reparented, old_parent)?;
        } else if metadata_changed {
            record_file_change_conn(conn, &state.file_id, FpChangeKind::Modified, None)?;
        }
    }
    Ok(())
}

pub(super) fn get_file_contract_state_conn(conn: &Connection, file_id: &str) -> Result<Option<FileContractState>> {
    let mut stmt = conn.prepare(
        "SELECT file_id, namespace, parent_id, shared_root_id, share_id, permission_bits,
                item_kind, content_type, current_version, current_object_version_id,
                local_base_version, local_hash, cache_path, cache_bytes, pin_state,
                inherited_pin_state, last_sync_at, owner_email
         FROM files WHERE file_id = ?1",
    )?;
    let mut rows = stmt.query(params![file_id])?;
    if let Some(row) = rows.next()? {
        Ok(Some(FileContractState {
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
        }))
    } else {
        Ok(None)
    }
}

#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(super) fn finder_write_conn(conn: &Connection, op_id: &str) -> Result<Option<FinderWrite>> {
    conn.query_row(
        "SELECT write_id, write_origin, after_write_id, base_pending FROM operation_queue
         WHERE op_id = ?1 AND write_id IS NOT NULL",
        params![op_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, i64>(3)?,
            ))
        },
    )
    .optional()
    .map(|found| {
        found.map(|(write_id, origin, after_write_id, base_pending)| FinderWrite {
            write_id,
            // A write id without an origin can only come from a bug; treat it as
            // earlier-build so it is never handed over (spec §8.4, m-2).
            origin: WriteOrigin::from_db(origin.as_deref()).unwrap_or(WriteOrigin::EarlierBuild),
            after_write_id,
            base_pending,
        })
    })
}

/// The claim's transaction ends in a wait, which is not an attempt (S3 steps 2–4). After
/// a hand-over it commits that, and the caller logs it and releases the payload (S6).
pub(super) fn finish_wait(tx: rusqlite::Transaction<'_>, took_over: Option<TookOver>) -> Result<ClaimOutcome> {
    tx.commit()?;
    Ok(match took_over {
        None => ClaimOutcome::Wait,
        Some(took_over) => ClaimOutcome::WaitAfterHandOver(took_over),
    })
}

/// The park of an op that holds no claim (spec §8.4): attempts used up, the reason
/// recorded, the bytes kept. The claim's parks and the snapshot count's both use it, so
/// a later save on the parked write's token takes its role the same way at its claim.
/// An op whose completion is recorded is never parked (§8.6 rule 6): 0 rows.
pub(super) fn park_unclaimed_conn(conn: &Connection, op_id: &str, reason: ParkReason, now: i64) -> Result<usize> {
    conn.execute(
        "UPDATE operation_queue
         SET attempts = max_attempts, last_error = ?2, last_error_class = ?2, updated_at = ?3
         WHERE op_id = ?1
           AND NOT EXISTS (SELECT 1 FROM upload_resume WHERE op_id = ?1 AND completed_version IS NOT NULL)",
        params![op_id, reason.as_str(), now],
    )
}

/// §9.1–§9.2, after a park, in the park's own transaction: a parked upload presents `error`
/// (read-only, still not evictable), unless another Finder upload of the file has not parked
/// (§8.4: a successor waiting to take over keeps the item `uploading`). Only an `uploading` row
/// moves: a `Trashing` row keeps its trash presentation (m-8), a `Conflict` row its own.
/// Write-keyed: only a Finder upload is ever parked, so Windows and Linux never reach it. A flip
/// is recorded for the system, as every status change is.
pub(super) fn settle_status_after_park_conn<C: std::ops::Deref<Target = Connection>>(conn: &C, file_id: &str) -> Result<()> {
    let flipped = conn.execute(
        "UPDATE files SET status = 'error'
         WHERE file_id = ?1 AND status = 'uploading'
           AND NOT EXISTS (SELECT 1 FROM operation_queue
                           WHERE file_id = ?1 AND write_id IS NOT NULL
                             AND kind IN ('upload_version', 'upload_file')
                             AND attempts < max_attempts)",
        params![file_id],
    )?;
    if flipped == 1 {
        record_file_change_conn(conn, file_id, FpChangeKind::Modified, None)?;
    }
    Ok(())
}

/// The claim parks the op with its bytes (spec §8.4, S5), in its own transaction.
pub(super) fn park_in_claim(
    tx: rusqlite::Transaction<'_>,
    op: &PendingOperation,
    reason: ParkReason,
    took_over: Option<TookOver>,
    now: i64,
) -> Result<ClaimOutcome> {
    // The claim never parks an op whose completion is recorded (it skips steps 3–4 for it);
    // were one to reach here, the park refuses it and it waits, which is not an attempt.
    if park_unclaimed_conn(&tx, &op.op_id, reason, now)? != 1 {
        return finish_wait(tx, took_over);
    }
    if let Some(file_id) = op.file_id.as_deref() {
        settle_status_after_park_conn(&tx, file_id)?;
    }
    tx.commit()?;
    Ok(ClaimOutcome::Parked {
        op_id: op.op_id.clone(),
        file_id: op.file_id.clone(),
        reason,
        took_over,
    })
}

pub(super) fn predecessor_conn(conn: &Connection, write_id: &str) -> Result<Option<Predecessor>> {
    conn.query_row(
        "SELECT q.op_id, q.kind, q.parent_id, q.target_path, q.metadata_json, q.payload_path,
                q.base_version, q.base_object_version_id, q.after_write_id, q.base_pending,
                q.attempts, q.max_attempts, q.write_origin,
                EXISTS (SELECT 1 FROM upload_resume r
                        WHERE r.op_id = q.op_id AND r.completed_version IS NOT NULL),
                q.last_error_class
         FROM operation_queue q WHERE q.write_id = ?1",
        params![write_id],
        |row| {
            Ok(Predecessor {
                op_id: row.get(0)?,
                kind: row.get(1)?,
                parent_id: row.get(2)?,
                target_path: row.get(3)?,
                metadata_json: row.get(4)?,
                payload_path: row.get(5)?,
                base_version: row.get(6)?,
                base_object_version_id: row.get(7)?,
                after_write_id: row.get(8)?,
                base_pending: row.get(9)?,
                attempts: row.get(10)?,
                max_attempts: row.get(11)?,
                origin: WriteOrigin::from_db(row.get::<_, Option<String>>(12)?.as_deref()),
                completed: row.get(13)?,
                park_reason: row.get(14)?,
            })
        },
    )
    .optional()
}

/// §8.4: successor N takes parked predecessor W's role. N's bytes contain W's (§8.2,
/// last point), so no byte the person saved is lost. W's op and resume row are removed
/// and its payload is journalled for release; the caller unlinks it after the commit
/// (S6). Both rows are addressed exactly; anything else fails and the transaction
/// rolls back.
pub(super) fn hand_over_conn(
    conn: &Connection,
    n_op_id: &str,
    n_write_id: &str,
    n: &PendingOperation,
    w: &Predecessor,
) -> Result<()> {
    let w_is_create = w
        .metadata_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .is_some_and(|m| m["operation"].as_str() == Some("create_file"));
    // A Finder create is `upload_version` with `"operation": "create_file"`.
    // N takes W's create: W's kind, parent and path, and N's metadata as a create.
    let (kind, parent_id, target_path, metadata_json) = if w_is_create {
        let mut metadata: serde_json::Value = n
            .metadata_json
            .as_deref()
            .and_then(|raw| serde_json::from_str(raw).ok())
            .unwrap_or_else(|| serde_json::json!({}));
        metadata["operation"] = serde_json::json!("create_file");
        if let Some(map) = metadata.as_object_mut() {
            map.remove("base_version_identifier");
        }
        (
            w.kind.clone(),
            w.parent_id.clone(),
            w.target_path.clone(),
            Some(metadata.to_string()),
        )
    } else {
        (
            n.kind.as_str().to_string(),
            n.parent_id.clone(),
            n.target_path.clone(),
            n.metadata_json.clone(),
        )
    };
    let moved = conn.execute(
        "UPDATE operation_queue
         SET base_version = ?3, base_object_version_id = ?4, after_write_id = ?5, base_pending = ?6,
             kind = ?7, parent_id = ?8, target_path = ?9, metadata_json = ?10
         WHERE op_id = ?1 AND write_id = ?2",
        params![
            n_op_id,
            n_write_id,
            w.base_version,
            w.base_object_version_id,
            w.after_write_id,
            w.base_pending,
            kind,
            parent_id,
            target_path,
            metadata_json,
        ],
    )?;
    let retired = conn.execute("DELETE FROM operation_queue WHERE op_id = ?1", params![w.op_id])?;
    if moved != 1 || retired != 1 {
        // The caller's `?` drops the transaction, which rolls back.
        return Err(rusqlite::Error::StatementChangedRows(moved + retired));
    }
    conn.execute("DELETE FROM upload_resume WHERE op_id = ?1", params![w.op_id])?;
    if let Some(path) = &w.payload_path {
        conn.execute(
            "INSERT INTO staged_payloads(path, completed) VALUES (?1, 1)
             ON CONFLICT(path) DO UPDATE SET completed = 1",
            params![path],
        )?;
    }
    Ok(())
}

#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(super) fn held_write_conn(conn: &Connection, file_id: &str) -> Result<(Option<crate::write_token::HeldWrite>, bool)> {
    conn.query_row(
        "SELECT held_write_id, held_base, held_version, held_object_version_id, version_filled
         FROM files WHERE file_id = ?1",
        params![file_id],
        |row| {
            let write_id: Option<String> = row.get(0)?;
            let held = match write_id {
                Some(write_id) => Some(crate::write_token::HeldWrite {
                    write_id,
                    // The two are written together (`set_held_write_conn`). A NULL
                    // `held_base` beside a write id is a bug, and reading it fails
                    // rather than map a base through `b = 0`.
                    base: row.get::<_, i64>(1)?,
                    version: row.get(2)?,
                    object_version_id: row.get(3)?,
                }),
                None => None,
            };
            Ok((held, row.get::<_, i64>(4)? != 0))
        },
    )
}

/// The one writer of a new held write: `held_write_id` and `held_base` are set together,
/// in one statement, and the landing columns are cleared. Exactly one row must match.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(super) fn set_held_write_conn(conn: &Connection, file_id: &str, write_id: &str, base: i64) -> Result<()> {
    let n = conn.execute(
        "UPDATE files SET held_write_id = ?2, held_base = ?3, held_version = NULL,
                          held_object_version_id = NULL
         WHERE file_id = ?1",
        params![file_id, write_id, base],
    )?;
    if n == 1 {
        Ok(())
    } else {
        Err(rusqlite::Error::QueryReturnedNoRows)
    }
}

/// The §6.1 facts of `file_id`, read inside the caller's transaction (spec §8.7 S1.1).
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(super) fn base_facts_conn(
    conn: &Connection,
    file_id: &str,
    contract: &FileContractState,
    legacy_identifiers: Vec<String>,
) -> Result<crate::write_token::BaseFacts> {
    let (held, version_filled) = held_write_conn(conn, file_id)?;
    let held_write_queued = match &held {
        Some(held) => conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM operation_queue WHERE write_id = ?1)",
            params![held.write_id],
            |row| row.get(0),
        )?,
        None => false,
    };
    // The CASE keeps a malformed metadata row from failing every save of the file.
    let create_queued: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM operation_queue
          WHERE file_id = ?1 AND kind IN ('upload_version', 'upload_file')
            AND CASE WHEN json_valid(metadata_json)
                     THEN json_extract(metadata_json, '$.operation') END = 'create_file')",
        params![file_id],
        |row| row.get(0),
    )?;
    let minted_resolved_bases = {
        let mut stmt = conn.prepare(
            "SELECT base_version FROM operation_queue
             WHERE file_id = ?1 AND write_origin = ?2 AND after_write_id IS NULL
               AND base_pending = 0 AND base_version IS NOT NULL",
        )?;
        let rows = stmt.query_map(params![file_id, WriteOrigin::Minted.as_str()], |row| {
            row.get::<_, i64>(0)
        })?;
        rows.collect::<Result<Vec<_>>>()?
    };
    Ok(crate::write_token::BaseFacts {
        current_version: contract.current_version,
        version_filled,
        held,
        held_write_queued,
        create_queued: create_queued && contract.current_version == 0,
        minted_resolved_bases,
        legacy_identifiers,
    })
}

/// A finalization journal row, with its payload marked completed: the landing writes it in
/// its own transaction (Windows creates).
pub(super) fn insert_upload_finalization_conn(conn: &Connection, row: &UploadFinalization) -> Result<()> {
    conn.execute(
        "INSERT INTO upload_finalizations(op_id,local_file_id,server_file_id,target_path,payload_path,stamped)
        VALUES(?1,?2,?3,?4,?5,0)",
        params![
            row.op_id,
            row.local_file_id,
            row.server_file_id,
            row.target_path,
            row.payload_path
        ],
    )?;
    conn.execute(
        "INSERT INTO staged_payloads(path,completed) VALUES(?1,1)
        ON CONFLICT(path) DO UPDATE SET completed=1",
        params![row.payload_path],
    )?;
    Ok(())
}
