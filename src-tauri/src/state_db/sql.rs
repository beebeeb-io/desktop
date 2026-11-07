//! `sql` items split out of `state_db.rs`.

use super::*;

/// R10 (spec 2026-10-06 §5.6): every table holding one account's data. `has_account_data` counts
/// them and `clear_account_data` empties them. `every_table_is_classified_for_the_account_binding`
/// fails when a new table is not listed here or in `DEVICE_TABLES`.
pub(super) const ACCOUNT_TABLES: [&str; 26] = [
    "owned_key_records",
    "files",
    "operation_queue",
    "local_activity",
    "transfer_activity",
    "fp_changes",
    "fp_sync_anchor",
    "fp_materialized",
    "upload_finalizations",
    "staged_payloads",
    "upload_resume",
    // A File Provider create's provisional id and the server id it landed as (spec §5.4 row 15): the account's.
    "id_aliases",
    // Task 1640: copies kept from files deleted elsewhere (file ids and names): the account's.
    "kept_copies",
    // Task 1640 (review F3): folders deleted elsewhere that stay on disk for what they hold (names): the account's.
    "removed_folders",
    // Task 2191: the check state of files of unknown content (file ids): the account's.
    "unknown_checks",
    // Task 2191 (D12): placeholders left at another path that hold unknown bytes (names): the account's.
    "unknown_orphans",
    // Task 2190: sync-folder edits held, not sent, because their base is unknown (file ids): the account's.
    "held_edits",
    // LD-0105 S6: placeholders a failed fetch wrote into, until emptied (file ids): the account's.
    "cf_poison",
    // LD-0105 Amendment 4 (desktop #225): pins the empty step could not put back (file ids): the account's.
    "cf_pin_restore",
    // LD-0113: what this device backed up (keys are paths inside a backed-up folder, remote ids): the account's.
    "backup_records",
    // LD-0113: a backup op's server upload session (session and file ids): the account's.
    "backup_upload_resume",
    // LD-0113 D4: where the backup lives, its category folders and its stop notices (folder ids): the account's.
    "backup_layout",
    "backup_category_folders",
    "backup_stops",
    // LD-0139: when each backup last checked the account's notices, and when it was turned on: the account's.
    "backup_pass_marks",
    // LD-0113: the account's flagged Backups root (a folder id): the account's.
    "backup_root",
];

/// Task 2191: a file row of unknown content (synced before task 1640: no agreed content and no
/// fetch candidate), whatever its status. The guarded set (spec D2).
pub(super) const GUARDED_SQL: &str = "item_kind = 'file' AND local_hash IS NULL AND fetched_min_version < 0";

/// Task 2191: a guarded row, as the guard reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct GuardedRow {
    pub file_id: String,
    pub path: String,
    pub status: FileStatus,
    pub fetched_min_version: i64,
}

pub(super) fn guarded_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<GuardedRow> {
    Ok(GuardedRow {
        file_id: row.get(0)?,
        path: row.get(1)?,
        status: FileStatus::from_str(&row.get::<_, String>(2)?),
        fetched_min_version: row.get(3)?,
    })
}

/// Per-device tables: not counted as account data, emptied by a reset anyway.
pub(super) const DEVICE_TABLES: [&str; 2] = [
    "bandwidth_samples",
    // LD-0113 Amendment 9: which of this device's folders OneDrive moved and the question about each (paths).
    "backup_source_questions",
];

pub(super) const OWNER_USER_ID_KEY: &str = "owner_user_id";

pub(super) const OWNER_EMAIL_KEY: &str = "owner_email";

/// `PRAGMA user_version` once the one-time adoption window is over (see `adopt_owner_once`). It lives in the
/// database header, not in a table, so no purge or reset can clear it. 0 is every database from before R10.
pub(super) const ADOPTION_CLOSED: i64 = 1;

/// A Finder domain removal is owed before any engine may start (macOS; task 1834 fix round 1, F3). A `sync_state`
/// row like the owner record: not account data, written in the same transaction as the reset that creates the debt.
pub(super) const FINDER_REMOVAL_OWED_KEY: &str = "finder_removal_owed";

/// A clear that owed no removal (a sign-out whose Finder removal was confirmed) leaves this mark: the domain is
/// known gone, so the next account's first start, onto an empty and unowned database, owes nothing. Not account data.
pub(super) const FINDER_DOMAIN_GONE_KEY: &str = "finder_domain_gone";

/// Spec 2026-10-10 §15 question 3, option A. Set by a sign-out's keep step after the engine stop, so a Finder accept
/// that outlived the stop queues nothing. Cleared at once by a sign-out that stops without purging, by the purge with
/// the account's rows, by `engine_start_repair`, and at launch.
pub(super) const FINDER_ACCEPTS_CLOSED_KEY: &str = "finder_accepts_closed";

pub const LOCAL_ACTIVITY_MAX_ROWS: usize = 200;

/// How long a provisional id keeps resolving to the server id its create landed as (spec §7.1).
pub const ALIAS_MAX_AGE_SECS: i64 = 30 * 86_400;

/// Cap on `transfer_activity` (task 1683 slice 2). The popover shows five rows.
pub const TRANSFER_ACTIVITY_MAX_ROWS: usize = 100;

/// Windows Cloud Files on-disk placeholder attribute bits, decoded into the
/// app's [`FileStatus`] / [`PinState`] vocabulary. **Pure + cross-platform** so
/// it is unit-testable on Linux: it takes a raw `u32` attribute mask (what
/// `GetFileAttributesW` returns) plus the row's current status, and returns the
/// *desired* status/pin — the caller (the Windows reconcile pass) does the delta
/// check against the live row and only persists when something actually changed.
///
/// Bit values are the `Win32_Storage_FileSystem` `FILE_ATTRIBUTE_*` constants
/// (hardcoded here as plain `u32` so this fn never references a windows-only
/// symbol):
/// - `RECALL_ON_DATA_ACCESS` (`0x0040_0000`) — the placeholder is *dehydrated*
///   (cloud-only). Cleared ⇒ the bytes are resident on disk (local).
/// - `PINNED` (`0x0008_0000`) — "Always keep on this device".
/// - `UNPINNED` (`0x0010_0000`) — "Free up space" / online-only pin.
///
/// ## Status rule (never fight the engine)
///
/// Status is only decoded when the current status is one of the two
/// *user/OS-owned* terminal states — [`FileStatus::Local`] or
/// [`FileStatus::CloudOnly`]. The transient/engine-owned states
/// (`Uploading` / `Downloading` / `Conflict` / `Error`) are left ALONE
/// (`None`) because a native Explorer attribute snapshot must never clobber an
/// in-flight transfer or a conflict the engine is mid-resolving.
///
/// Returned status is the *desired* one for Local/CloudOnly candidates; the
/// caller compares it to the live row and skips no-op writes.
///
/// ## Pin rule (don't fight inheritance)
///
/// `PINNED` set ⇒ [`PinState::Pinned`]; `UNPINNED` set ⇒ [`PinState::Unpinned`];
/// neither bit set ⇒ `None` (leave the row's pin as-is — most placeholders
/// simply inherit their parent's pin and carry no explicit bit).
pub fn decode_os_state(attrs: u32, current_status: FileStatus) -> (Option<FileStatus>, Option<PinState>) {
    const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;
    const FILE_ATTRIBUTE_PINNED: u32 = 0x0008_0000;
    const FILE_ATTRIBUTE_UNPINNED: u32 = 0x0010_0000;

    let status = match current_status {
        // Only the user/OS-owned terminal states are reconciled from disk.
        FileStatus::Local | FileStatus::CloudOnly => {
            if attrs & FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS != 0 {
                Some(FileStatus::CloudOnly)
            } else {
                Some(FileStatus::Local)
            }
        }
        // Engine-owned: leave it alone. `Trashing` (a locally-deleted file whose
        // server-trash is pending) is included here so a native attribute
        // snapshot can never flip it back to `CloudOnly`/`Local` and re-seed the
        // placeholder we just removed.
        FileStatus::Uploading
        | FileStatus::Downloading
        | FileStatus::Conflict
        | FileStatus::Error
        | FileStatus::Trashing => None,
    };

    let pin = if attrs & FILE_ATTRIBUTE_PINNED != 0 {
        Some(PinState::Pinned)
    } else if attrs & FILE_ATTRIBUTE_UNPINNED != 0 {
        Some(PinState::Unpinned)
    } else {
        None
    };

    (status, pin)
}

/// The sync-anchor wire encoding for change-log sequence `seq`: decimal ASCII
/// of the rowid. Strictly ascending by construction (`seq` is an AUTOINCREMENT
/// rowid), lexicographically ordered while under 10 digits, and far inside
/// Apple's 500-byte anchor budget.
pub(super) fn anchor_bytes(seq: i64) -> Vec<u8> {
    seq.to_string().into_bytes()
}

/// Wall-clock seconds since the epoch (0 on a clock before 1970 — a degraded
/// stamp beats a panic in the change-log insert path).
pub(super) fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The `operation_queue` columns [`pending_operation_from_row`] reads, in order.
pub(super) const PENDING_OPERATION_COLUMNS: &str = "op_id, kind, file_id, parent_id, target_path, metadata_json, payload_path,
    base_version, base_object_version_id, attempts, max_attempts, next_retry_at,
    last_error, backup_source_key, created_at, updated_at";

/// Queue order (spec §8.5, M2). On macOS it is insertion order, `rowid` alone: a
/// wall clock that steps back must not run a newer save before an older one. Other
/// platforms keep round 3's order, `created_at` then `rowid`.
#[cfg(target_os = "macos")]
pub(super) const DUE_ORDER_SQL: &str = "ORDER BY rowid ASC";

#[cfg(not(target_os = "macos"))]
pub(super) const DUE_ORDER_SQL: &str = "ORDER BY created_at ASC, rowid ASC";

/// Parent first (spec D7, I5; task 2226): an op whose parent folder still has a `create_folder` op of its own is not
/// due, whatever state that create is in, and its claim claims nothing, so it spends no attempt on the parent's 404.
/// It becomes due once that create landed (its op is gone). Appended to a `WHERE` on `operation_queue` without an alias;
/// the lookup goes through `idx_operation_queue_file_id`. Empty on macOS: its File Provider path does not change.
#[cfg(target_os = "macos")]
pub(super) const DUE_PARENT_GATE_SQL: &str = "";

#[cfg(not(target_os = "macos"))]
pub(super) const DUE_PARENT_GATE_SQL: &str = "AND NOT EXISTS (SELECT 1 FROM operation_queue p
                     WHERE p.kind = 'create_folder' AND p.file_id = operation_queue.parent_id)";

/// D7: the folders whose create cannot land until the person or a pass acts: paused or out of attempts, and every folder
/// made inside one of them, at any depth. A change in one waits for its folder (`waiting_for_folder`) and is no
/// transfer work. For `WITH RECURSIVE`; reads `idx_operation_queue_create_folder` alone.
#[cfg(not(target_os = "macos"))]
pub(super) const HELD_FOLDER_CTE: &str = "held_folder(file_id) AS (
         SELECT file_id FROM operation_queue
         WHERE kind = 'create_folder' AND file_id IS NOT NULL
           AND (paused_reason IS NOT NULL OR attempts >= max_attempts)
         UNION
         SELECT inner_create.file_id FROM held_folder
         JOIN operation_queue inner_create
           ON inner_create.kind = 'create_folder' AND inner_create.parent_id = held_folder.file_id
         WHERE inner_create.file_id IS NOT NULL
     )";

/// Task 2204: a Windows rename (`metadata.local_rename`) not sent yet: unclaimed, and neither
/// parked nor paused.
pub(super) const PENDING_LOCAL_RENAME_SQL: &str = "kind IN ('rename_file', 'move_file')
       AND CASE WHEN json_valid(metadata_json) THEN json_extract(metadata_json, '$.local_rename') END = 1
       AND claim_id IS NULL AND attempts < max_attempts AND paused_reason IS NULL";

/// The columns [`file_overview_row`] reads.
pub(super) const FILE_OVERVIEW_COLUMNS: &str = "file_id, path, status, size_bytes, modified_at, content_hash, remote_updated_at,
     parent_id, item_kind, pin_state, inherited_pin_state";

/// A [`FILE_OVERVIEW_COLUMNS`] row and whether it is pinned.
pub(super) fn file_overview_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<(FileEntry, bool)> {
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
    // Mirror FileContractState::effective_pin_state: own pin wins unless
    // it's `inherit`, in which case the inherited state decides.
    let effective = match pin_state {
        PinState::Inherit => inherited_pin_state,
        explicit => explicit,
    };
    Ok((entry, effective == PinState::Pinned))
}

/// Task 2229: the row at a path. The Windows enumeration scan runs this for every file and
/// folder on disk on every pass, so it must reach the row through `idx_files_path`: a full read
/// of `files` per lookup burned a core on a 43k-file tree (0.8.13 to 0.8.15).
pub(super) const FILE_BY_PATH_SQL: &str = "SELECT file_id, path, status, size_bytes, modified_at, content_hash, remote_updated_at,
            parent_id, item_kind
     FROM files WHERE path = ?1";

/// A [`FILE_BY_PATH_SQL`] row as a [`FileEntry`], for every lookup that runs it.
pub(super) fn file_entry_from_path_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FileEntry> {
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
}

/// Task 2229: every column of the row at a path, for [`StateDb::file_row_stamp_by_path`]. The scan
/// runs it for every file it skips, so it too goes through `idx_files_path`.
pub(super) const FILE_ROW_BY_PATH_SQL: &str = "SELECT * FROM files WHERE path = ?1";

/// Task 2240 (D2): the scan's page cache never goes below SQLite's default, `cache_size = -2000` (2,000 KiB): the value
/// every fresh connection reads, which `task_2240_ui_handles_keep_the_default_cache` checks …
pub(crate) const PAGE_CACHE_FLOOR_KIB: i64 = 2_000;

/// … nor above 128 MiB, which holds `files` and `idx_files_path` up to about 300k rows.
pub(crate) const PAGE_CACHE_CEILING_KIB: i64 = 128 * 1024;

/// Task 2240 (D2): the page cache, in KiB, for a DB of `db_bytes`: the whole DB plus an eighth, within
/// [`PAGE_CACHE_FLOOR_KIB`, `PAGE_CACHE_CEILING_KIB`]. The scan's working set (`files` and `idx_files_path`) is part of
/// the DB, so the DB's size always covers it. SQLite fills the cache only with pages the connection reads, so its memory
/// follows what is read, up to this limit.
pub(crate) fn page_cache_limit_kib(db_bytes: i64) -> i64 {
    let kib = db_bytes.max(0) / 1024;
    (kib + kib / 8).clamp(PAGE_CACHE_FLOOR_KIB, PAGE_CACHE_CEILING_KIB)
}

/// `get_file_by_path`'s other stored shape: `/leaf` ⇄ `leaf`.
pub(super) fn toggled_slash(path: &str) -> String {
    match path.strip_prefix('/') {
        Some(stripped) => stripped.to_string(),
        None => format!("/{path}"),
    }
}

/// `earlier` comes before `this` in [`DUE_ORDER_SQL`]'s order.
#[cfg(target_os = "macos")]
pub(super) const EARLIER_IN_QUEUE_SQL: &str = "earlier.rowid < this.rowid";

#[cfg(not(target_os = "macos"))]
pub(super) const EARLIER_IN_QUEUE_SQL: &str =
    "earlier.created_at < this.created_at OR (earlier.created_at = this.created_at AND earlier.rowid < this.rowid)";

/// Task 2190 (plan §3.1, D2): a parked op the person may decide, as the alias `this`: a File
/// Provider content write (it has a write id) parked on a stale base, unclaimed, with no completion
/// recorded; nothing queued behind it for the file (D2: no newer save, and no trash, move or rename,
/// review-169 #2); and a file the pull has seen at a version past that base, so "Keep this Mac's" has a
/// version to rebase on, and that is not being trashed (review-169 F2: a trash from another device, of
/// the file or of a folder it is in, marks the row `trashing` and queues nothing; a file deleted for
/// good has no row). No folder above it is being trashed or moved on this Mac either (the lead, after
/// F2): a local trash marks only the folder's row, and a local trash, move or rename queues on the
/// folder, never on the file. The walk up `parent_id` is a primary-key lookup per level, and stops at
/// 512 levels (the walk's depth cap, plan §7.4): macOS paths are at most 1024 bytes (`PATH_MAX`) and every
/// level takes at least two (`a/`), so no real file nests deeper. A chain that reaches the cap (a cycle in
/// the rows) is not decidable. The review feed and every decision use this one predicate, so the feed
/// never offers a choice the decision refuses.
pub(super) const DECIDABLE_PARKED_SQL: &str = "this.kind = 'upload_version' AND this.write_id IS NOT NULL
    AND this.last_error_class = 'stale_base' AND this.attempts >= this.max_attempts
    AND this.claim_id IS NULL AND this.base_version IS NOT NULL
    AND NOT EXISTS (SELECT 1 FROM upload_resume AS resume
                    WHERE resume.op_id = this.op_id AND resume.completed_version IS NOT NULL)
    AND NOT EXISTS (SELECT 1 FROM operation_queue AS later
                    WHERE later.file_id = this.file_id AND later.op_id != this.op_id AND later.rowid > this.rowid)
    AND EXISTS (SELECT 1 FROM files AS file
                WHERE file.file_id = this.file_id AND file.current_version > this.base_version
                  AND file.status != 'trashing')
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

/// Task 2190: [`StateDb::list_held_edits`]'s read, kept in one place for its query-plan test.
pub(super) const LIST_HELD_EDITS_SQL: &str = "SELECT held.file_id, file.path, held.reason, held.detected_at
     FROM held_edits AS held INDEXED BY idx_held_edits_detected
     JOIN files AS file ON file.file_id = held.file_id
     ORDER BY held.detected_at, held.file_id";

pub(super) fn decidable_parked_conn(conn: &Connection, op_id: &str) -> Result<bool> {
    conn.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM operation_queue AS this WHERE this.op_id = ?1 AND {DECIDABLE_PARKED_SQL})"),
        params![op_id],
        |row| row.get(0),
    )
}

/// Task 2190 (review-169 #1): decidable, and its file still at `shown`, the version the person was
/// shown when they chose. A file past it is a version they have not seen: the decision is `Moved`.
pub(super) fn decidable_parked_at_conn(conn: &Connection, op_id: &str, shown: i64) -> Result<bool> {
    conn.query_row(
        &format!(
            "SELECT EXISTS(SELECT 1 FROM operation_queue AS this WHERE this.op_id = ?1 AND {DECIDABLE_PARKED_SQL}
               AND EXISTS (SELECT 1 FROM files AS shown WHERE shown.file_id = this.file_id AND shown.current_version = ?2))"
        ),
        params![op_id, shown],
        |row| row.get(0),
    )
}

/// Task 2190 (plan §3.2, review-169 #4): the end of "Keep the other" and "Keep both", in the caller's
/// transaction: the parked op goes, the file's held token is cleared (only this write's), the file
/// presents `local` again, a change is recorded, and the staged copy is RELEASED in the journal
/// (`completed = 1`, as `finish_claimed` does), so a crash before the unlink leaves no unfinished row:
/// the engine-start sweep unlinks it (S6). Returns the copy for the caller to unlink after the commit.
pub(super) fn retire_parked_conn(tx: &rusqlite::Transaction<'_>, op_id: &str) -> Result<Option<String>> {
    let (file_id, write_id, payload): (String, String, Option<String>) = tx.query_row(
        "SELECT file_id, write_id, payload_path FROM operation_queue WHERE op_id = ?1",
        params![op_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    tx.execute("DELETE FROM operation_queue WHERE op_id = ?1", params![op_id])?;
    tx.execute("DELETE FROM upload_resume WHERE op_id = ?1", params![op_id])?;
    tx.execute(
        "UPDATE files SET held_write_id = NULL, held_base = NULL, held_version = NULL,
                          held_object_version_id = NULL
         WHERE file_id = ?1 AND held_write_id = ?2",
        params![file_id, write_id],
    )?;
    tx.execute(
        "UPDATE files SET status = 'local'
         WHERE file_id = ?1 AND status IN ('error', 'uploading')
           AND NOT EXISTS (SELECT 1 FROM operation_queue
                           WHERE file_id = ?1 AND kind IN ('upload_version', 'upload_file'))",
        params![file_id],
    )?;
    record_file_change_conn(tx, &file_id, FpChangeKind::Modified, None)?;
    if let Some(path) = payload.as_deref() {
        tx.execute(
            "INSERT INTO staged_payloads(path, completed) VALUES (?1, 1)
             ON CONFLICT(path) DO UPDATE SET completed = 1",
            params![path],
        )?;
    }
    Ok(payload)
}

pub(super) fn pending_operation_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PendingOperation> {
    Ok(PendingOperation {
        op_id: row.get(0)?,
        kind: OperationKind::from_str(&row.get::<_, String>(1)?),
        file_id: row.get(2)?,
        parent_id: row.get(3)?,
        target_path: row.get(4)?,
        metadata_json: row.get(5)?,
        payload_path: row.get(6)?,
        base_version: row.get(7)?,
        base_object_version_id: row.get(8)?,
        attempts: row.get(9)?,
        max_attempts: row.get(10)?,
        next_retry_at: row.get(11)?,
        last_error: row.get(12)?,
        backup_source_key: row.get(13)?,
        created_at: row.get(14)?,
        updated_at: row.get(15)?,
    })
}

/// Task 1640 (review F1): the file's agreed content joins its fetch candidate, as the
/// `files_cloud_only_folds_agreed_into_candidate` trigger does: its bytes are no longer known to be all of
/// that version. An agreed content with no version makes the content not known.
pub(super) fn fold_agreed_into_candidate_conn(conn: &Connection, file_id: &str) -> Result<()> {
    conn.execute(
        &format!("UPDATE files SET {FOLD_AGREED_INTO_CANDIDATE} WHERE file_id = ?1 AND local_hash IS NOT NULL"),
        params![file_id],
    )?;
    Ok(())
}

/// The fold's SET clause, shared by [`fold_agreed_into_candidate_conn`] and the
/// `files_cloud_only_folds_agreed_into_candidate` trigger (self-review #9: one definition). Every
/// right-hand side reads the row as it was before the update.
pub(super) const FOLD_AGREED_INTO_CANDIDATE: &str = "
    fetched_hash = CASE
        WHEN fetched_min_version = 0 THEN local_hash
        WHEN fetched_min_version = local_base_version AND fetched_hash = local_hash THEN fetched_hash
        ELSE NULL END,
    fetched_min_version = CASE
        WHEN local_base_version <= 0 THEN -1
        WHEN fetched_min_version = 0 THEN local_base_version
        ELSE MIN(fetched_min_version, local_base_version) END,
    local_hash = NULL";

pub(super) const KEPT_COPY_COLUMNS: &str = "id, original_file_id, original_path, copy_path, copied_hash, original_removed, reason, kept_at";

pub(super) fn kept_copy_from_row(row: &rusqlite::Row<'_>) -> Result<KeptCopy> {
    Ok(KeptCopy {
        id: row.get(0)?,
        original_file_id: row.get(1)?,
        original_path: row.get(2)?,
        copy_path: row.get(3)?,
        copied_hash: row.get(4)?,
        original_removed: row.get(5)?,
        reason: KeptReason::from_str(&row.get::<_, String>(6)?),
        kept_at: row.get(7)?,
    })
}

/// [`StateDb::poison_local_content`] on an existing connection.
pub(super) fn poison_local_content_conn(conn: &Connection, file_id: &str) -> Result<()> {
    fold_agreed_into_candidate_conn(conn, file_id)?;
    conn.execute("UPDATE files SET fetched_hash = NULL WHERE file_id = ?1", params![file_id])?;
    Ok(())
}

/// Append one change to `fp_changes` and advance the persistent anchor
/// cursor, on an existing connection (the public [`StateDb::record_file_change`]
/// and the mutating row operations below share this).
pub(super) fn record_file_change_conn<C: std::ops::Deref<Target = Connection>>(
    conn: &C,
    file_id: &str,
    kind: FpChangeKind,
    old_parent_id: Option<String>,
) -> Result<()> {
    // The NEW parent is whatever the row says NOW. `parent_id` lives in the
    // files column (written by set_file_contract_state).
    let new_parent_id: Option<String> = conn
        .query_row(
            "SELECT parent_id FROM files WHERE file_id = ?1",
            params![file_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten()
        // Deleted rows no longer exist: the change must still carry the
        // parent it was deleted FROM (the materialized-set filter reads it),
        // which the caller passes as old_parent_id.
        .or_else(|| old_parent_id.clone());
    // Task 1697 review fix: stamp the REAL insertion time. The old
    // `recorded_at = 0` sentinel made the signal path's 7-day sweep
    // (`sweep_file_changes(now - 7d)`) delete the ENTIRE fresh log right
    // after asking File Provider to enumerate it — Finder received an empty
    // feed and missed every update.
    let recorded_at = now_secs();
    conn.execute(
        "INSERT INTO fp_changes (seq, file_id, kind, old_parent_id, new_parent_id, recorded_at)
         VALUES (0, ?1, ?2, ?3, ?4, ?5)",
        params![file_id, kind.as_str(), old_parent_id, new_parent_id, recorded_at],
    )?;
    let seq = conn.last_insert_rowid();
    conn.execute("UPDATE fp_changes SET seq = ?1 WHERE id = ?1", params![seq])?;
    // The anchor cursor tracks the log tip even before any consumer reads:
    // `list_file_changes` never rewinds it, so a poll that races a write
    // cannot lose changes.
    conn.execute(
        "INSERT INTO fp_sync_anchor (domain_id, last_anchor) VALUES ('__default__', ?1)
         ON CONFLICT(domain_id) DO UPDATE SET last_anchor = MAX(last_anchor, excluded.last_anchor)",
        params![seq],
    )?;
    Ok(())
}

/// Spec D5: whether `dir` holds a file anywhere under it. A missing directory holds none; anything that is not a
/// directory (a file, a link, a reparse point) counts as a file, and a directory that cannot be read counts as holding
/// one, so the pass never drops what it could not look at. Links are never followed.
pub(crate) fn tree_holds_a_file(dir: &Path) -> bool {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return false,
        Err(_) => return true,
    };
    for entry in entries {
        let Ok(entry) = entry else { return true };
        match std::fs::symlink_metadata(entry.path()) {
            Ok(meta) if meta.file_type().is_dir() => {
                if tree_holds_a_file(&entry.path()) {
                    return true;
                }
            }
            _ => return true,
        }
    }
    false
}


/// One injected landing failure, if this thread has one left.
#[cfg(test)]
pub(super) fn injected_landing_failure() -> Option<rusqlite::Error> {
    FAIL_LANDINGS.with(|left| {
        let (n, locked) = left.get();
        if n == 0 {
            return None;
        }
        left.set((n - 1, locked));
        // Not `InvalidQuery`: it displays as "Query is not read-only", which
        // `classify_operation_error` pauses as Permission, and a paused op is never listed
        // again. A full disk classifies as a retryable failure.
        Some(if locked {
            rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
                Some("database is locked".into()),
            )
        } else {
            rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_FULL),
                Some("injected landing failure".into()),
            )
        })
    })
}

/// The system's trash container, as the File Provider extension names it (the identifier
/// `NSFileProviderTrashContainerItemIdentifier`, or its short form).
pub(crate) fn is_file_provider_trash(id: &str) -> bool {
    matches!(id, "NSFileProviderTrashContainerItemIdentifier" | "trashContainer")
}

/// The last component of a row's path: the item's name.
pub(super) fn path_leaf(path: &str) -> &str {
    path.trim_matches('/').rsplit('/').next().unwrap_or_default()
}

/// The contract of a file the database has no row for: our own new file.
pub(super) fn default_contract(file_id: &str) -> FileContractState {
    FileContractState {
        file_id: file_id.to_string(),
        namespace: Namespace::MyFiles,
        parent_id: None,
        shared_root_id: None,
        share_id: None,
        owner_email: None,
        permission_bits: PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER,
        item_kind: ItemKind::File,
        content_type: None,
        current_version: 0,
        current_object_version_id: None,
        local_base_version: 0,
        local_hash: None,
        cache_path: None,
        cache_bytes: 0,
        pin_state: PinState::Inherit,
        inherited_pin_state: PinState::Inherit,
        last_sync_at: 0,
    }
}

/// Uploads and restores of one file run in insertion order (spec §8.5, m-9).
pub(super) fn is_content_kind(kind: &OperationKind) -> bool {
    matches!(
        kind,
        OperationKind::UploadVersion | OperationKind::UploadFile | OperationKind::RestoreVersion
    )
}

/// Task 2267 (fix round 2): the `sync_state` keys of "Keep on this PC only" (`found_while_paused.rs`).
#[cfg(any(target_os = "windows", test))]
pub(super) const KEEP_JOURNAL_KEY: &str = "found_while_paused.keep_journal";

#[cfg(any(target_os = "windows", test))]
pub(super) const KEPT_DIR_KEY: &str = "found_while_paused.kept_dir";

/// Task 2267 (lead review of round 2, P2-1): an op (aliased `o`) that creates its item on the server: a folder's
/// create, or an upload whose metadata says it creates the file (`engine_bridge::is_create_file_operation`).
#[cfg(any(target_os = "windows", test))]
pub(super) const CREATE_OP_SQL: &str = "(o.kind = 'create_folder'
     OR (o.kind IN ('upload_version', 'upload_file')
         AND COALESCE(CASE WHEN json_valid(o.metadata_json) THEN json_extract(o.metadata_json, '$.operation') END = 'create_file', 0)))";

/// A [`ProofOp`] from `op_id, file_id, parent_id, kind`.
#[cfg(any(target_os = "windows", test))]
pub(super) fn proof_op_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProofOp> {
    Ok(ProofOp {
        op_id: row.get(0)?,
        file_id: row.get::<_, Option<String>>(1)?.filter(|id| !id.is_empty()),
        parent_id: row.get::<_, Option<String>>(2)?.filter(|id| !id.is_empty()),
        kind: row.get(3)?,
    })
}

pub(super) fn ensure_column(conn: &Connection, table: &str, column: &str, definition: &str) -> Result<()> {
    if !has_column(conn, table, column)? {
        add_column(conn, table, column, definition)?;
    }
    Ok(())
}

/// The `ALTER TABLE` alone, after [`has_column`] said the column is missing. Task 2278, belt and braces: another
/// connection may have added it in that window, and SQLite answers "duplicate column name". That is success, so the
/// column is looked up again rather than trusted on the error's text alone. Any other failure still propagates.
/// (`migrate_schema` now holds one write lock for the whole migration, so this cannot be reached by two openers; it
/// is here so a duplicate column is never an error whatever order a caller runs these in.)
pub(super) fn add_column(conn: &Connection, table: &str, column: &str, definition: &str) -> Result<()> {
    match conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"), []) {
        Ok(_) => Ok(()),
        Err(error) if is_duplicate_column(&error) => {
            if has_column(conn, table, column)? {
                Ok(())
            } else {
                Err(error)
            }
        }
        Err(error) => Err(error),
    }
}

/// SQLite's "duplicate column name: <column>" from an `ALTER TABLE ... ADD COLUMN` that another connection won.
pub(super) fn is_duplicate_column(error: &rusqlite::Error) -> bool {
    matches!(error, rusqlite::Error::SqliteFailure(_, Some(text)) if text.contains("duplicate column name"))
}

/// Task 2278: putting a database into WAL mode takes a brief exclusive lock, and SQLite does NOT run the busy
/// handler for a journal-mode change — it answers `SQLITE_BUSY` ("database is locked") at once, whatever
/// `busy_timeout` says. On a fresh file several first opens therefore meet here, before they ever reach the schema
/// transaction. The pragma is idempotent, so it is retried for the time the busy timeout would have bought: the
/// connection that converts the file wins, and every other one then reads back `wal`.
pub(super) fn enable_wal(conn: &Connection) -> Result<()> {
    const BUDGET: std::time::Duration = std::time::Duration::from_secs(5);
    const PAUSE: std::time::Duration = std::time::Duration::from_millis(5);
    let deadline = std::time::Instant::now() + BUDGET;
    loop {
        match conn.execute_batch("PRAGMA journal_mode = WAL;") {
            Ok(()) => return Ok(()),
            Err(error) if is_lock_contention(&error) && std::time::Instant::now() < deadline => {
                std::thread::sleep(PAUSE);
            }
            Err(error) => return Err(error),
        }
    }
}

pub(super) fn is_lock_contention(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(code, _)
            if matches!(code.code, rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
    )
}

pub(super) fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let names: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(names.iter().any(|n| n == column))
}

pub(super) fn count_queue_groups(conn: &Connection, column: &str) -> Result<BTreeMap<String, i64>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {column}, COUNT(*) FROM operation_queue WHERE {column} IS NOT NULL GROUP BY {column}"
    ))?;
    let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))?;
    rows.collect()
}

/// Credential-token stripping for error text persisted in the state DB. The
/// privacy boundary for names and paths is the export pass
/// ([`crate::diagnostic_redaction::export`]'s `redact_for_export`), not this.
pub(super) fn redact_diagnostic_error(error: &str) -> String {
    redact_secrets_only(error)
}

pub(super) fn allowed_group_labels(groups: BTreeMap<String, i64>, allowed: &[&str]) -> BTreeMap<String, i64> {
    let mut out = BTreeMap::new();
    for (label, count) in groups {
        *out.entry(allowed_label(&label, allowed)).or_insert(0) += count;
    }
    out
}

/// Every plaintext name the daemon knows locally: synced paths, queued
/// targets and staged payload leaves, recent activity, plus `extra_paths`.
pub(super) fn collect_known_names(conn: &Connection, extra_paths: &[String]) -> Result<KnownNames> {
    let mut names = KnownNames::new();
    for path in extra_paths {
        names.add_path(path);
    }
    for sql in [
        "SELECT path FROM files",
        "SELECT target_path FROM operation_queue WHERE target_path IS NOT NULL",
        "SELECT payload_path FROM operation_queue WHERE payload_path IS NOT NULL",
        "SELECT file_name FROM local_activity",
        "SELECT rel_path FROM local_activity WHERE rel_path IS NOT NULL",
        // Task 1683 slice 2: the recent-transfer names are known names too, so the
        // support bundle's error text is scrubbed of them (task 1685).
        "SELECT file_name FROM transfer_activity",
        "SELECT rel_path FROM transfer_activity WHERE rel_path IS NOT NULL",
    ] {
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        for path in rows {
            names.add_path(&path?);
        }
    }
    Ok(names.finish())
}

/// The owner record in `sync_state`, if any field of it is there.
pub(super) fn read_owner(conn: &Connection) -> Result<Option<crate::account_binding::Identity>> {
    let read = |key: &str| {
        conn.query_row("SELECT value FROM sync_state WHERE key = ?1", params![key], |row| {
            row.get::<_, String>(0)
        })
        .optional()
    };
    let owner =
        crate::account_binding::Identity::new(read(OWNER_USER_ID_KEY)?.as_deref(), read(OWNER_EMAIL_KEY)?.as_deref());
    Ok(owner.is_known().then_some(owner))
}

/// Both owner fields at once; an unknown field is removed.
pub(super) fn write_owner(conn: &Connection, owner: &crate::account_binding::Identity) -> Result<()> {
    for (key, value) in [(OWNER_USER_ID_KEY, &owner.user_id), (OWNER_EMAIL_KEY, &owner.email)] {
        match value {
            Some(value) => conn.execute(
                "INSERT INTO sync_state (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )?,
            None => conn.execute("DELETE FROM sync_state WHERE key = ?1", params![key])?,
        };
    }
    Ok(())
}

/// How many rows of an account live here: every `ACCOUNT_TABLES` row and every `sync_state` row except the
/// owner record, the device keys (the Finder removal debt, the domain-gone mark, closed Finder accepts) and the
/// custody memory (`shared_folder_custody_seen:%`, task 2279 N1). That memory is device-wide and survives every
/// local-state wipe, so counting it would keep `has_account_data` true after a completed sign-out — permanently,
/// because the set only grows — and the next engine start would want a reset or refuse on Windows.
pub(super) fn account_rows(conn: &Connection) -> Result<i64> {
    let mut rows: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sync_state
         WHERE key NOT IN (?1, ?2, ?3, ?4, ?5) AND key NOT LIKE 'shared_folder_custody_seen:%'",
        params![
            OWNER_USER_ID_KEY,
            OWNER_EMAIL_KEY,
            FINDER_REMOVAL_OWED_KEY,
            FINDER_DOMAIN_GONE_KEY,
            FINDER_ACCEPTS_CLOSED_KEY
        ],
        |row| row.get(0),
    )?;
    for table in ACCOUNT_TABLES {
        rows += conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get::<_, i64>(0))?;
    }
    Ok(rows)
}

#[cfg(test)]
pub(super) fn has_table(conn: &Connection, table: &str) -> Result<bool> {
    let mut stmt = conn.prepare("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1")?;
    let mut rows = stmt.query(params![table])?;
    Ok(rows.next()?.is_some())
}
