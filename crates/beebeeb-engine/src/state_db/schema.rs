//! Opening the database and its schema migration (split out of `state_db.rs`).

use super::*;

impl StateDb {
    /// Open or create the state database at `path`. Idempotent — the
    /// schema migration uses `CREATE TABLE IF NOT EXISTS` so re-opens
    /// are cheap.
    ///
    /// Task 2488: `pub` only with the `direct-state-db` feature (the Tauri shell, tests). Without it the constructor
    /// is the crate's own, and a shell reaches the local state through `app::Engine::local_state` alone.
    #[cfg(any(test, feature = "direct-state-db"))]
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_at(path.as_ref())
    }
    #[cfg(not(any(test, feature = "direct-state-db")))]
    pub(crate) fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_at(path.as_ref())
    }
    fn open_at(path: &Path) -> Result<Self> {
        let mut conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        // Task 2278: the journal mode cannot be changed inside a transaction, so it is set first, on its own.
        enable_wal(&conn)?;
        // Task 2278: the schema and every column migration run under ONE write lock. Two connections opening a
        // fresh database at the same moment used to both see a column missing — `has_column`, then `ALTER TABLE
        // ... ADD COLUMN`, with no transaction around them — and the second ALTER returned "duplicate column
        // name: fetched_min_version" (macOS CI, 2026-10-29, `a_sign_out_leaves_no_account_row_and_no_owner`).
        // A second opener now waits on the busy timeout above for this transaction, then finds the schema done.
        {
            let migration = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            Self::migrate_schema(&migration)?;
            migration.commit()?;
        }
        Ok(Self(Mutex::new(conn), std::sync::atomic::AtomicU64::new(0)))
    }
    /// Open an existing state database for reading only: no table is created or migrated, no row is written and
    /// the journal mode is left as it is, so a viewer can look at a database the engine owns. (SQLite itself may
    /// create the `-shm` side file of a WAL database; the database file is not written.)
    ///
    /// Two locks keep the handle read-only: the connection is opened `SQLITE_OPEN_READ_ONLY`, and `query_only` is
    /// set on it, so every writer of the returned `StateDb` fails in SQLite before it changes anything
    /// (`state_db::read_only_tests`). Before it returns, the listing's own columns are read once, so a file that
    /// is not a state database, or one from a build too old to list, is refused here with its own reason and
    /// never shows as an empty folder.
    ///
    /// Task 2488: `pub` only with the `direct-state-db` feature, as [`Self::open`] is.
    #[cfg(any(test, feature = "direct-state-db"))]
    pub fn open_read_only(path: impl AsRef<Path>) -> std::result::Result<Self, ReadOnlyOpenError> {
        Self::open_read_only_at(path.as_ref())
    }
    #[cfg(not(any(test, feature = "direct-state-db")))]
    pub(crate) fn open_read_only(path: impl AsRef<Path>) -> std::result::Result<Self, ReadOnlyOpenError> {
        Self::open_read_only_at(path.as_ref())
    }
    fn open_read_only_at(path: &Path) -> std::result::Result<Self, ReadOnlyOpenError> {
        if !path.is_file() {
            return Err(ReadOnlyOpenError::NotFound);
        }
        let conn = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| ReadOnlyOpenError::classify(&e))?;
        // A short wait for the proof read, so a database another process holds answers "locked" at once.
        conn.busy_timeout(std::time::Duration::from_millis(250))
            .map_err(|e| ReadOnlyOpenError::classify(&e))?;
        conn.pragma_update(None, "query_only", true)
            .map_err(|e| ReadOnlyOpenError::classify(&e))?;
        let has_files: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'files')",
                [],
                |row| row.get(0),
            )
            .map_err(|e| ReadOnlyOpenError::classify(&e))?;
        if !has_files {
            return Err(ReadOnlyOpenError::NotAStateDatabase);
        }
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| ReadOnlyOpenError::classify(&e))?;
        let db = Self(Mutex::new(conn), std::sync::atomic::AtomicU64::new(0));
        // The listing statement itself, for no rows: it names every column a listing reads.
        db.children_paged(None, 0, 0)
            .map_err(|e| ReadOnlyOpenError::classify(&e))?;
        Ok(db)
    }
    /// The schema of a state database, created if missing and brought up to today's columns. Runs inside
    /// [`StateDb::open`]'s `BEGIN IMMEDIATE`, so one opener applies it and any other opener waits and then finds it
    /// finished. `CREATE TABLE IF NOT EXISTS` covers both the first-run case and a re-open of a database that already
    /// has every column.
    fn migrate_schema(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS owned_key_records (
                file_id TEXT PRIMARY KEY,
                record TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS files (
                file_id TEXT PRIMARY KEY,
                path TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'cloud_only',
                size_bytes INTEGER NOT NULL DEFAULT 0,
                modified_at INTEGER NOT NULL DEFAULT 0,
                content_hash TEXT,
                remote_updated_at INTEGER NOT NULL DEFAULT 0,
                namespace TEXT NOT NULL DEFAULT 'my_files',
                parent_id TEXT,
                shared_root_id TEXT,
                share_id TEXT,
                owner_email TEXT,
                permission_bits INTEGER NOT NULL DEFAULT 0,
                item_kind TEXT NOT NULL DEFAULT 'file',
                content_type TEXT,
                current_version INTEGER NOT NULL DEFAULT 0,
                current_object_version_id TEXT,
                local_base_version INTEGER NOT NULL DEFAULT 0,
                local_hash TEXT,
                cache_path TEXT,
                cache_bytes INTEGER NOT NULL DEFAULT 0,
                pin_state TEXT NOT NULL DEFAULT 'inherit',
                inherited_pin_state TEXT NOT NULL DEFAULT 'unpinned',
                last_opened_at INTEGER NOT NULL DEFAULT 0,
                last_sync_at INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_status ON files(status);
            CREATE TABLE IF NOT EXISTS operation_queue (
                op_id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                file_id TEXT,
                parent_id TEXT,
                target_path TEXT,
                metadata_json TEXT,
                payload_path TEXT,
                base_version INTEGER,
                base_object_version_id TEXT,
                attempts INTEGER NOT NULL DEFAULT 0,
                max_attempts INTEGER NOT NULL DEFAULT 5,
                next_retry_at INTEGER NOT NULL DEFAULT 0,
                last_error TEXT,
                last_error_class TEXT,
                paused_reason TEXT,
                -- Origin known-folder key (task 0811) for backup-originated ops.
                -- NULL for every normal (non-backup) op. Disabling a known-folder
                -- backup purges exactly its tagged ops so nothing resumes on boot.
                backup_source_key TEXT,
                created_at INTEGER NOT NULL DEFAULT 0,
                updated_at INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_operation_queue_due ON operation_queue(next_retry_at, created_at);
            CREATE TABLE IF NOT EXISTS sync_state (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS bandwidth_samples (
                sampled_at  INTEGER NOT NULL,
                up_bytes    INTEGER NOT NULL DEFAULT 0,
                down_bytes  INTEGER NOT NULL DEFAULT 0,
                period_secs INTEGER NOT NULL DEFAULT 20
            );
            CREATE INDEX IF NOT EXISTS idx_bandwidth_samples_at ON bandwidth_samples(sampled_at);
            CREATE TABLE IF NOT EXISTS local_activity (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                event_type TEXT NOT NULL,
                file_id TEXT,
                file_name TEXT NOT NULL,
                rel_path TEXT,
                occurred_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_local_activity_recent ON local_activity(occurred_at DESC, id DESC);
            -- Task 1683 slice 2: which way each recent transfer went (the popover's
            -- arrow). A separate table, not new `local_activity` kinds: that table's
            -- cap is 200 rows and a large sync would push the trash events out of it.
            CREATE TABLE IF NOT EXISTS transfer_activity (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                direction TEXT NOT NULL,
                file_id TEXT,
                file_name TEXT NOT NULL,
                rel_path TEXT,
                bytes INTEGER NOT NULL DEFAULT 0,
                occurred_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_transfer_activity_recent ON transfer_activity(occurred_at DESC, id DESC);
            -- Task 1697 (File Provider FPFS conformance): the daemon-side change
            -- log the replica's enumerator pages through. `id` is the change
            -- cursor: an AUTOINCREMENT rowid, so anchors are monotonic and
            -- compact (<= 500 bytes as decimal ASCII). `seq` mirrors `id` so the
            -- sweep can leave the anchor row while pruning consumed changes.
            -- `old_parent_id`/`new_parent_id` drive the materialized-set filter
            -- (old OR new parent materialized, per Apple's Replicated contract).
            -- A change is never rewritten once appended; readers page by cursor.
            CREATE TABLE IF NOT EXISTS fp_changes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                seq INTEGER NOT NULL,
                file_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                old_parent_id TEXT,
                new_parent_id TEXT,
                recorded_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_fp_changes_seq ON fp_changes(seq);
            -- The persistent, monotonic sync-anchor cursor the replica reads
            -- back after process death (keyed per domain id; one domain today).
            CREATE TABLE IF NOT EXISTS fp_sync_anchor (
                domain_id TEXT PRIMARY KEY,
                last_anchor INTEGER NOT NULL
            );
            -- The materialized container set: folders the SYSTEM reports as
            -- materialized on disk (via the extension's App Group journal,
            -- tracked here so the working-set filter can run daemon-side).
            CREATE TABLE IF NOT EXISTS fp_materialized (
                container_id TEXT PRIMARY KEY,
                updated_at INTEGER NOT NULL
            );
        ",
        )?;
        ensure_column(conn, "files", "remote_updated_at", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(conn, "files", "namespace", "TEXT NOT NULL DEFAULT 'my_files'")?;
        ensure_column(conn, "files", "parent_id", "TEXT")?;
        ensure_column(conn, "files", "shared_root_id", "TEXT")?;
        ensure_column(conn, "files", "share_id", "TEXT")?;
        ensure_column(conn, "files", "owner_email", "TEXT")?;
        ensure_column(conn, "files", "permission_bits", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(conn, "files", "item_kind", "TEXT NOT NULL DEFAULT 'file'")?;
        ensure_column(conn, "files", "content_type", "TEXT")?;
        ensure_column(conn, "files", "current_version", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(conn, "files", "current_object_version_id", "TEXT")?;
        ensure_column(conn, "files", "local_base_version", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(conn, "files", "local_hash", "TEXT")?;
        // Task 1640 (review N1, F1): what the fetches put on disk (see `record_fetched_version` and
        // docs/design/1640-local-content-identity.md). -1, the default, is "not known": every row from
        // before this column, and every new one until its placeholder is created where nothing was. A
        // new name (self-review #2): a database from a round-2 build has `fetched_version` DEFAULT 0, which
        // no ALTER can change; that column is left unused.
        ensure_column(conn, "files", "fetched_min_version", "INTEGER NOT NULL DEFAULT -1")?;
        ensure_column(conn, "files", "fetched_hash", "TEXT")?;
        ensure_column(conn, "files", "cache_path", "TEXT")?;
        ensure_column(conn, "files", "cache_bytes", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(conn, "files", "pin_state", "TEXT NOT NULL DEFAULT 'inherit'")?;
        ensure_column(conn, "files", "inherited_pin_state", "TEXT NOT NULL DEFAULT 'unpinned'")?;
        ensure_column(conn, "files", "last_opened_at", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(conn, "files", "last_sync_at", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(conn, "operation_queue", "last_error_class", "TEXT")?;
        ensure_column(conn, "operation_queue", "paused_reason", "TEXT")?;
        // Task 0811: additive backup-origin tag. Existing rows migrate to NULL
        // (untagged → treated as normal ops, never purged by a folder disable).
        ensure_column(conn, "operation_queue", "backup_source_key", "TEXT")?;
        // Task 1697: which local write created the row (watcher paths set this;
        // Finder-queue paths do not) — the change-log recorder reads it.
        ensure_column(conn, "files", "creator_for_fp", "TEXT")?;
        // Spec 2026-10-09 §5.2: the write token. Additive and nullable, no backfill;
        // only the dedicated functions write them.
        ensure_column(conn, "files", "held_write_id", "TEXT")?;
        ensure_column(conn, "files", "held_base", "INTEGER")?;
        ensure_column(conn, "files", "held_version", "INTEGER")?;
        ensure_column(conn, "files", "held_object_version_id", "TEXT")?;
        ensure_column(conn, "files", "version_filled", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(conn, "operation_queue", "write_id", "TEXT")?;
        ensure_column(conn, "operation_queue", "write_origin", "TEXT")?;
        ensure_column(conn, "operation_queue", "after_write_id", "TEXT")?;
        ensure_column(conn, "operation_queue", "base_pending", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(conn, "operation_queue", "claim_id", "TEXT")?;
        ensure_column(conn, "operation_queue", "claimed_at", "INTEGER")?;
        // LD-0113 (spec §5.4): the server's `system_role` of a row (`backups_root` or NULL). Additive and
        // nullable; only `set_system_role` writes it, from the listing's field or the `folder_system_role` op.
        ensure_column(conn, "files", "system_role", "TEXT")?;
        conn.execute_batch(
            "
            -- LD-0113: the Windows delete/rename callbacks ask for the Backups root on every delete; a
            -- partial index keeps that a read of the flagged rows only.
            CREATE INDEX IF NOT EXISTS idx_files_system_role ON files(system_role) WHERE system_role IS NOT NULL;
            CREATE INDEX IF NOT EXISTS idx_files_namespace ON files(namespace);
            CREATE INDEX IF NOT EXISTS idx_files_shared_root ON files(shared_root_id);
            -- Task 2229: `FILE_BY_PATH_SQL`, once per file and folder on every enumeration pass.
            CREATE INDEX IF NOT EXISTS idx_files_path ON files(path);
            -- Task 2243: the tray's recent list reads the newest rows through it, not every row.
            CREATE INDEX IF NOT EXISTS idx_files_modified_at ON files(modified_at);
            -- Task 2243: `queue_hold_summary`, polled by the tray every few seconds, reads this alone. It starts
            -- with `paused_reason`, so it also serves every lookup the old `idx_operation_queue_paused` did, and
            -- that index is dropped: one index to keep up to date on each queue write, not two.
            -- Task 2409: the summary leaves the downloads out by `kind` (`UNSENT_KINDS_SQL`), so the index carries
            -- it and the poll still reads no operation; it replaces `idx_operation_queue_hold_summary`.
            DROP INDEX IF EXISTS idx_operation_queue_hold_summary;
            CREATE INDEX IF NOT EXISTS idx_operation_queue_unsent
                ON operation_queue(paused_reason, created_at, attempts, max_attempts, kind);
            DROP INDEX IF EXISTS idx_operation_queue_paused;
            CREATE INDEX IF NOT EXISTS idx_operation_queue_backup_source ON operation_queue(backup_source_key);
            -- Flow 7 (interrupted upload resume): the server upload session an
            -- UploadVersion/UploadFile op is writing to, persisted the moment
            -- `POST /uploads/init` returns and advanced after every
            -- acknowledged chunk, so a retry resumes instead of minting a
            -- second server file row. Keyed by op_id; the payload fingerprint
            -- (path + size + mtime) guards against resuming onto other bytes.
            CREATE TABLE IF NOT EXISTS upload_finalizations (
                op_id TEXT PRIMARY KEY,
                local_file_id TEXT NOT NULL,
                server_file_id TEXT NOT NULL,
                target_path TEXT NOT NULL,
                payload_path TEXT NOT NULL,
                stamped INTEGER NOT NULL DEFAULT 0
            );
            -- Task 1640 (review N9, N13): a file deleted elsewhere while it held changes this
            -- device had not sent, kept as a copy (paths relative to the sync root). The review
            -- feed lists each until it is dismissed; an original that could not be removed when
            -- the copy was made (`original_removed = 0`) is removed by a later tick.
            CREATE TABLE IF NOT EXISTS kept_copies (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                original_file_id TEXT NOT NULL,
                original_path TEXT NOT NULL,
                copy_path TEXT NOT NULL,
                copied_hash TEXT NOT NULL,
                original_removed INTEGER NOT NULL DEFAULT 0,
                kept_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_kept_copies_original_path ON kept_copies(original_path);
            CREATE TABLE IF NOT EXISTS removed_folders (
                path TEXT PRIMARY KEY,
                removed_at INTEGER NOT NULL
            );
            -- Task 2191: a file of unknown content (spec section 4). In S1 only `mark_pending` is
            -- used: a not-in-sync mark that is still to be made (it failed, or the row joined the
            -- guarded set during the session). The other columns are the check's (S2).
            CREATE TABLE IF NOT EXISTS unknown_checks (
                file_id TEXT PRIMARY KEY,
                state TEXT,
                attempts INTEGER NOT NULL DEFAULT 0,
                stops INTEGER NOT NULL DEFAULT 0,
                next_attempt_at INTEGER NOT NULL DEFAULT 0,
                last_error TEXT,
                copied_hash TEXT,
                mark_pending INTEGER NOT NULL DEFAULT 0,
                recorded_hash TEXT,
                recorded_size INTEGER,
                recorded_mtime INTEGER,
                resume_candidate TEXT,
                resume_chunk INTEGER,
                resume_digest TEXT,
                updated_at INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_unknown_checks_mark_pending ON unknown_checks(mark_pending) WHERE mark_pending = 1;
            -- Task 2191 (D12): a placeholder at `path` whose identity names `file_id`, a file of
            -- unknown content that lives at another path (a remote rename or move left it there).
            -- Task 2190 (the lead's condition on E3): a sync-folder edit the scan holds and does not
            -- send, because the version it is based on is not known (`base_unknown`) or the file's
            -- content is unknown (`content_unknown`, hydrated before task 1640). The review feed
            -- shows it, so a held edit never looks synced in the app. Kept OUT of the `files` row
            -- (#167's scan memo keys on that row), and cleared by the triggers below when the row goes,
            -- moves or leaves `local`. `fingerprint` is the file's content fingerprint when held.
            CREATE TABLE IF NOT EXISTS held_edits (
                file_id TEXT PRIMARY KEY,
                reason TEXT NOT NULL,
                fingerprint TEXT NOT NULL,
                detected_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_held_edits_detected ON held_edits(detected_at, file_id);
            CREATE TRIGGER IF NOT EXISTS files_held_edit_follows_delete
            AFTER DELETE ON files
            BEGIN
                DELETE FROM held_edits WHERE file_id = OLD.file_id;
            END;
            CREATE TRIGGER IF NOT EXISTS files_held_edit_follows_change
            AFTER UPDATE OF file_id, path, status ON files
            WHEN NEW.file_id IS NOT OLD.file_id OR NEW.path IS NOT OLD.path
              OR (OLD.status = 'local' AND NEW.status != 'local')
            BEGIN
                DELETE FROM held_edits WHERE file_id = OLD.file_id;
            END;
            CREATE TABLE IF NOT EXISTS unknown_orphans (
                path TEXT PRIMARY KEY,
                file_id TEXT NOT NULL,
                state TEXT,
                attempts INTEGER NOT NULL DEFAULT 0,
                next_attempt_at INTEGER NOT NULL DEFAULT 0,
                mark_pending INTEGER NOT NULL DEFAULT 0,
                copied_hash TEXT,
                updated_at INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS staged_payloads (
                path TEXT PRIMARY KEY,
                source_path TEXT,
                completed INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS upload_resume (
                op_id TEXT PRIMARY KEY,
                payload_path TEXT NOT NULL,
                payload_size INTEGER NOT NULL,
                payload_mtime_ns INTEGER NOT NULL,
                upload_session_id TEXT NOT NULL,
                server_file_id TEXT NOT NULL,
                object_version_id TEXT NOT NULL,
                chunk_size_bytes INTEGER NOT NULL,
                chunk_count INTEGER NOT NULL,
                acked_chunks INTEGER NOT NULL DEFAULT 0,
                metadata_applied INTEGER NOT NULL DEFAULT 0,
                is_create INTEGER NOT NULL DEFAULT 0,
                updated_at INTEGER NOT NULL DEFAULT 0
            );
            ",
        )?;
        ensure_column(conn, "upload_resume", "completed_version", "INTEGER")?;
        ensure_column(conn, "upload_resume", "completed_object_version_id", "TEXT")?;
        ensure_column(conn, "upload_resume", "completed_mime_type", "TEXT")?;
        // SF-0001 (L13): sessions persisted before this flag existed were opened
        // under a server-chosen id; only a self-minted CREATE may be resumed.
        ensure_column(conn, "upload_resume", "self_minted", "INTEGER NOT NULL DEFAULT 0")?;
        // Chunk format v2: the sealed name (with the upload's version commitment) that `complete` publishes,
        // stored before the first chunk is sent. NULL on every session an older build opened: those are v1.
        ensure_column(conn, "upload_resume", "completion_name_encrypted", "TEXT")?;
        // Chunk format v2: the highest version counter this device wrote or authenticated for a file, so a
        // replacement's counter is above it (core's rule) whatever the local key record still says.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS file_version_counters (
                file_id TEXT PRIMARY KEY,
                highest INTEGER NOT NULL,
                published_name TEXT
            );",
        )?;
        // Task 2270 (spec M1): a finalization that cannot complete for a lasting reason is held for review.
        ensure_column(conn, "upload_finalizations", "review", "TEXT")?;
        ensure_column(conn, "upload_finalizations", "review_at", "INTEGER")?;
        conn.execute_batch(
            "
            CREATE INDEX IF NOT EXISTS idx_operation_queue_write_id ON operation_queue(write_id);
            CREATE INDEX IF NOT EXISTS idx_operation_queue_after_write_id ON operation_queue(after_write_id);
            -- The one-read presentation looks up a file's queued uploads on every item.
            CREATE INDEX IF NOT EXISTS idx_operation_queue_file_id ON operation_queue(file_id);
            CREATE TABLE IF NOT EXISTS id_aliases (
                provisional_id TEXT PRIMARY KEY,
                server_id TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            ",
        )?;
        // Tasks 1888, 2226 (spec D7): `queue_hold_summary`'s `waiting_for_folder` group reads these two alone, so the
        // tray's poll stays inside task 2243's budget. The first holds only the folder creates; the second finds the
        // changes in a folder, and with `idx_files_parent_id` serves the reconcile pass's remaps (`REMAP_FILES_SQL`,
        // `REMAP_OPS_SQL`), which would otherwise scan every row once per remapped folder. Indexes only, no table or
        // column change. macOS has neither the parent-first gate nor the pass. Task 2409: the second carries `kind`, as
        // the summary's group leaves the downloads out (`UNSENT_KINDS_SQL`); it replaces `idx_operation_queue_parent`.
        #[cfg(not(target_os = "macos"))]
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_files_parent_id ON files(parent_id);
             CREATE INDEX IF NOT EXISTS idx_operation_queue_create_folder
                 ON operation_queue(parent_id, file_id, paused_reason, attempts, max_attempts)
                 WHERE kind = 'create_folder';
             DROP INDEX IF EXISTS idx_operation_queue_parent;
             CREATE INDEX IF NOT EXISTS idx_operation_queue_parent_kind
                 ON operation_queue(parent_id, paused_reason, attempts, max_attempts, created_at, kind);",
        )?;
        // Task 1640 (review F5): why a file deleted elsewhere was kept (`KeptReason`).
        ensure_column(conn, "kept_copies", "reason", "TEXT NOT NULL DEFAULT 'copy'")?;
        // Task 1640 (review-151d P2-1): the person dismissed the entry of a file still kept where it
        // is; the row stays as a tombstone, so its folder is still the server's deletion.
        ensure_column(conn, "kept_copies", "dismissed", "INTEGER NOT NULL DEFAULT 0")?;
        // Task 1640 (review F1, design T6): a file that goes cloud-only may still hold some of its
        // bytes (a placeholder partly on disk reads as cloud-only on every tick, and no dehydration is
        // known to remove them all). Its agreed content no longer describes every byte, so it joins
        // the fetch candidate (`FOLD_AGREED_INTO_CANDIDATE`); nothing is forgotten. A dehydration only
        // removes bytes, so the candidate stays true of what is left. The earlier triggers (round 2's
        // wipe, and the first fold, on `fetched_version`) are dropped; a change to the fold takes a new
        // trigger name, since `IF NOT EXISTS` keeps an existing definition.
        conn.execute_batch(&format!(
            "DROP TRIGGER IF EXISTS files_cloud_only_forgets_local_content;
             DROP TRIGGER IF EXISTS files_cloud_only_folds_agreed_content;
             CREATE TRIGGER IF NOT EXISTS files_cloud_only_folds_agreed_into_candidate
             AFTER UPDATE OF status ON files
             WHEN NEW.status = 'cloud_only' AND OLD.status != 'cloud_only' AND NEW.local_hash IS NOT NULL
             BEGIN
                 UPDATE files SET {FOLD_AGREED_INTO_CANDIDATE} WHERE file_id = NEW.file_id;
             END;"
        ))?;
        // Task 2191 (review-114 R2-P2-3): a row that joins the guarded set during the session (a
        // pull inserting a row whose 0.8.12 placeholder already exists, or a row losing its
        // agreed content) gets its "not in sync" mark at the next tick, not at the next start.
        // The tick's retry reads its place once and clears the flag.
        // Only the Windows guard reads these flags, so other builds carry no triggers (self-review).
        if !cfg!(any(target_os = "windows", test, feature = "test-support")) {
            conn.execute_batch(
                "DROP TRIGGER IF EXISTS files_unknown_marks_after_insert;
                 DROP TRIGGER IF EXISTS files_unknown_marks_after_insert_any_status;
                 DROP TRIGGER IF EXISTS files_unknown_marks_after_update;",
            )?;
        } else {
            conn.execute_batch(&format!(
                "CREATE TRIGGER IF NOT EXISTS files_unknown_marks_after_insert_any_status
             AFTER INSERT ON files
             WHEN NEW.local_hash IS NULL AND NEW.fetched_min_version < 0
             BEGIN
                 INSERT INTO unknown_checks(file_id, mark_pending, updated_at)
                 VALUES (NEW.file_id, 1, CAST(strftime('%s', 'now') AS INTEGER))
                 ON CONFLICT(file_id) DO UPDATE SET mark_pending = 1;
             END;
             CREATE TRIGGER IF NOT EXISTS files_unknown_marks_after_update
             AFTER UPDATE OF local_hash, fetched_min_version ON files
             WHEN NEW.local_hash IS NULL AND NEW.fetched_min_version < 0
              AND NOT (OLD.local_hash IS NULL AND OLD.fetched_min_version < 0)
             BEGIN
                 INSERT INTO unknown_checks(file_id, mark_pending, updated_at)
                 VALUES (NEW.file_id, 1, CAST(strftime('%s', 'now') AS INTEGER))
                 ON CONFLICT(file_id) DO UPDATE SET mark_pending = 1;
             END;
             CREATE TRIGGER IF NOT EXISTS files_unknown_checks_follow_delete
             AFTER DELETE ON files
             BEGIN
                 DELETE FROM unknown_checks WHERE file_id = OLD.file_id;
             END;
             DROP TRIGGER IF EXISTS files_unknown_marks_after_insert;"
            ))?;
        }
        // LD-0105 / task 2180: the version a Windows placeholder was minted from (`cf_streaming.rs`).
        // NULL on every row from before it: no identity, so bytes on disk for it are never trusted.
        ensure_column(conn, "files", "minted_object_version_id", "TEXT")?;
        // LD-0105 S6: placeholders a failed fetch wrote into, until the empty step has dehydrated them.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS cf_poison (
                file_id TEXT PRIMARY KEY,
                poisoned_at INTEGER NOT NULL,
                reason TEXT NOT NULL,
                change_ticks INTEGER,
                file_index INTEGER
            );",
        )?;
        // LD-0105 Amendment 4: the poison's key is the file's content (`LastWriteTime` + size), not its
        // metadata; a changed or missing key is "needs attention".
        ensure_column(conn, "cf_poison", "write_ticks", "INTEGER")?;
        ensure_column(conn, "cf_poison", "key_size", "INTEGER")?;
        ensure_column(conn, "cf_poison", "needs_attention", "INTEGER NOT NULL DEFAULT 0")?;
        // Amendment 4 (desktop #225 review P2): a pin the empty step could put back neither through its
        // handle nor by path. Its own table, so it outlives the poison row it came from; every sweep retries it.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS cf_pin_restore (
                file_id TEXT PRIMARY KEY,
                pin INTEGER NOT NULL
            );",
        )?;
        // LD-0105 W2 (I3, I4): the rest of a placeholder's identity, recorded before its first transfer.
        ensure_column(conn, "files", "minted_format", "TEXT")?;
        ensure_column(conn, "files", "minted_version_id", "TEXT")?;
        Self::migrate_backup_schema(conn)?;
        // Upgrade inventory: old one-shot Keep Mine rows may have no queue
        // owner. Preserve their plaintext reference before any resume purge.
        conn.execute(
            // LD-0113: never a queued backup source (the `staged_payloads` trigger would abort the open itself).
            "INSERT OR IGNORE INTO staged_payloads(path, completed) SELECT payload_path, 0 FROM upload_resume
             WHERE payload_path NOT IN (SELECT source_path FROM operation_queue WHERE source_path IS NOT NULL)",
            [],
        )?;
        Ok(())
    }

    /// LD-0113 (spec 2026-11-06 §6.1 point 4): a backup uploads a file from the person's own folder, where it is. Its
    /// op says so (`origin = 'backup'`) and names the file in `source_path`; `payload_path` stays NULL, because desktop
    /// treats a payload as a staged copy it owns and deletes it once the upload is done (`staged_payload::remove`, which
    /// has no gate). The triggers make that a property of the tables rather than of each writer:
    ///
    /// - a backup op never has a `payload_path`, and only a backup op has a `source_path`;
    /// - no backup op ever gets an `upload_resume` row (its `payload_path` is copied into the staged-copy journal at
    ///   every open, above) or an `upload_finalizations` row (finalization unlinks its `payload_path`);
    /// - the staged-copy journal never names the source of a queued backup op.
    ///
    /// A backup session persists in `backup_upload_resume`, which has no path column at all. `backup_records` is what
    /// this device backed up, per account (R10): `account_owner` is part of the key, so another account's records are
    /// never read even before the R10 reset clears them.
    fn migrate_backup_schema(conn: &Connection) -> Result<()> {
        ensure_column(conn, "operation_queue", "origin", "TEXT NOT NULL DEFAULT 'sync'")?;
        ensure_column(conn, "operation_queue", "source_path", "TEXT")?;
        conn.execute_batch(
            "CREATE TRIGGER IF NOT EXISTS operation_queue_backup_source_insert
             BEFORE INSERT ON operation_queue
             WHEN NEW.origin NOT IN ('sync', 'backup')
               OR (NEW.origin = 'backup' AND (NEW.payload_path IS NOT NULL OR NEW.source_path IS NULL))
               OR (NEW.origin = 'sync' AND NEW.source_path IS NOT NULL)
             BEGIN
                 SELECT RAISE(ABORT, 'ld0113: a backup op keeps its source in source_path, never in payload_path');
             END;
             CREATE TRIGGER IF NOT EXISTS operation_queue_backup_source_update
             BEFORE UPDATE OF origin, payload_path, source_path ON operation_queue
             WHEN NEW.origin NOT IN ('sync', 'backup')
               OR (NEW.origin = 'backup' AND (NEW.payload_path IS NOT NULL OR NEW.source_path IS NULL))
               OR (NEW.origin = 'sync' AND NEW.source_path IS NOT NULL)
             BEGIN
                 SELECT RAISE(ABORT, 'ld0113: a backup op keeps its source in source_path, never in payload_path');
             END;
             CREATE TRIGGER IF NOT EXISTS upload_resume_never_for_a_backup_op
             BEFORE INSERT ON upload_resume
             WHEN EXISTS (SELECT 1 FROM operation_queue WHERE op_id = NEW.op_id AND origin = 'backup')
             BEGIN
                 SELECT RAISE(ABORT, 'ld0113: a backup session persists in backup_upload_resume');
             END;
             CREATE TRIGGER IF NOT EXISTS upload_finalizations_never_for_a_backup_op
             BEFORE INSERT ON upload_finalizations
             WHEN EXISTS (SELECT 1 FROM operation_queue WHERE op_id = NEW.op_id AND origin = 'backup')
             BEGIN
                 SELECT RAISE(ABORT, 'ld0113: a backup op is never finalized in the sync root');
             END;
             CREATE INDEX IF NOT EXISTS idx_operation_queue_source_path
                 ON operation_queue(source_path) WHERE source_path IS NOT NULL;
             CREATE TRIGGER IF NOT EXISTS staged_payloads_never_a_backup_source
             BEFORE INSERT ON staged_payloads
             WHEN EXISTS (SELECT 1 FROM operation_queue WHERE source_path = NEW.path)
             BEGIN
                 SELECT RAISE(ABORT, 'ld0113: a backup source is the person''s own file, never a staged copy');
             END;
             CREATE TABLE IF NOT EXISTS backup_records (
                 account_owner TEXT NOT NULL,
                 category TEXT NOT NULL,
                 key TEXT NOT NULL,
                 size INTEGER NOT NULL,
                 modified_ms INTEGER NOT NULL,
                 remote_id TEXT NOT NULL,
                 uploaded_at_ms INTEGER NOT NULL,
                 state TEXT NOT NULL DEFAULT 'backed_up',
                 PRIMARY KEY (account_owner, category, key)
             );
             CREATE TABLE IF NOT EXISTS backup_upload_resume (
                 op_id TEXT PRIMARY KEY,
                 server_file_id TEXT NOT NULL,
                 upload_session_id TEXT NOT NULL,
                 object_version_id TEXT NOT NULL,
                 chunk_size_bytes INTEGER NOT NULL,
                 chunk_count INTEGER NOT NULL,
                 acked_chunks INTEGER NOT NULL DEFAULT 0,
                 chunk_format INTEGER NOT NULL,
                 source_size INTEGER NOT NULL,
                 source_modified_ms INTEGER NOT NULL,
                 source_file_id TEXT,
                 updated_at INTEGER NOT NULL DEFAULT 0
             );",
        )?;
        // LD-0113 Task D4: where this account's backup lives on the server (resolved once, then cached), the
        // category folders it created or adopted, and the backups that were turned off with why (rule 11).
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS backup_layout (
                 account_owner TEXT PRIMARY KEY,
                 root_id TEXT NOT NULL,
                 device_folder_id TEXT NOT NULL,
                 device_folder_name TEXT NOT NULL,
                 updated_at INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS backup_category_folders (
                 account_owner TEXT NOT NULL,
                 category TEXT NOT NULL,
                 folder_id TEXT NOT NULL,
                 PRIMARY KEY (account_owner, category)
             );
             CREATE TABLE IF NOT EXISTS backup_stops (
                 account_owner TEXT NOT NULL,
                 category TEXT NOT NULL,
                 reason TEXT NOT NULL,
                 at_ms INTEGER NOT NULL,
                 PRIMARY KEY (account_owner, category)
             );",
        )?;
        // LD-0113 (spec §4.2 rule 5, §5.2): the account's flagged Backups root, once resolved (it lives as long as the
        // account, so it is never looked up again). Amendment 9: the question asked once per folder OneDrive's Known
        // Folder Move has moved, until the person answers it (a device's folders, not the account's).
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS backup_root (
                 account_owner TEXT PRIMARY KEY,
                 root_id TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS backup_source_questions (
                 category TEXT PRIMARY KEY,
                 default_path TEXT NOT NULL,
                 moved_path TEXT NOT NULL,
                 asked_at_ms INTEGER NOT NULL
             );",
        )?;
        // LD-0139 (spec Amendments 12 and 12c): per backup, `last_pass_at_ms` on the SERVER's clock (the `Date` of
        // the notices check of its last pass; NULL until its first pass) and `setup_at_ms` on this device's clock
        // (when the person last turned it on or chose "Back up again"; NULL when not known, e.g. on an upgrade).
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS backup_pass_marks (
                 account_owner TEXT NOT NULL,
                 category TEXT NOT NULL,
                 last_pass_at_ms INTEGER,
                 setup_at_ms INTEGER,
                 PRIMARY KEY (account_owner, category)
             );",
        )?;
        // LD-0139 (spec Amendment 12d, core v0.2.5): the notice ids the backup's last pass acknowledged (core's
        // `acknowledged`, a sorted JSON array of strings), written in the same transaction as its `last_pass_at_ms`
        // and any stop, and kept by "Back up again". A column, not a table, so one row write carries both.
        ensure_column(
            conn,
            "backup_pass_marks",
            "acknowledged_json",
            "TEXT NOT NULL DEFAULT '[]'",
        )?;
        Ok(())
    }
}
