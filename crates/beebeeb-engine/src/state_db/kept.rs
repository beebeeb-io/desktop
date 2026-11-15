//! What this PC keeps (Available offline), read in one query each: the kept rows for This PC's list, and the count and
//! size for a line that only needs the numbers (the native Sync pane). Never one query per row over the whole file
//! list (review of desktop#377, P2-2). Pure reads.

use super::*;

/// A row is kept when its own pin says so, or it inherits one (`FileContractState::effective_pin_state`).
const KEPT_SQL: &str = "(pin_state = 'pinned' OR (pin_state = 'inherit' AND inherited_pin_state = 'pinned'))
     AND status != 'trashing'";

/// How much this PC keeps: the top-most kept items (a kept folder counts once, not its contents) and the bytes of
/// every kept file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeptSummary {
    pub items: u64,
    pub bytes: i64,
}

/// A kept file this PC does not hold, as [`StateDb::kept_files_to_fetch`] reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeptRow {
    pub file_id: String,
    pub path: String,
    pub size_bytes: i64,
}

impl StateDb {
    /// The kept files this PC does not hold, oldest row first, at most `limit`: kept by [`KEPT_SQL`] (its own pin or
    /// an inherited one), a file with bytes, `cloud_only` (and `error` when `failed_too`: a failed Windows fetch ends
    /// there), with nothing queued and no edit held (the stale rule's two clauses). Task 2523 S6: what a tick fetches.
    pub fn kept_files_to_fetch(&self, limit: usize, failed_too: bool) -> Result<Vec<KeptRow>> {
        let statuses = if failed_too {
            "'cloud_only', 'error'"
        } else {
            "'cloud_only'"
        };
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(&format!(
            "SELECT file_id, path, size_bytes FROM files WHERE {KEPT_SQL}
               AND item_kind = 'file' AND size_bytes > 0 AND status IN ({statuses})
               AND NOT EXISTS (SELECT 1 FROM operation_queue q WHERE q.file_id = files.file_id)
               AND NOT EXISTS (SELECT 1 FROM held_edits h WHERE h.file_id = files.file_id)
             ORDER BY rowid LIMIT ?1"
        ))?;
        let rows = stmt.query_map(params![i64::try_from(limit).unwrap_or(i64::MAX)], |row| {
            Ok(KeptRow {
                file_id: row.get(0)?,
                path: row.get(1)?,
                size_bytes: row.get(2)?,
            })
        })?;
        rows.collect()
    }

    /// Every kept row (files and folders, a kept folder's contents included), by path, with nothing else of the file
    /// list: one query.
    pub fn kept_rows(&self) -> Result<Vec<FileEntry>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(&format!(
            "SELECT {FILE_OVERVIEW_COLUMNS} FROM files WHERE {KEPT_SQL} ORDER BY path ASC"
        ))?;
        let rows = stmt.query_map([], file_overview_row)?;
        rows.map(|row| row.map(|(entry, _)| entry)).collect()
    }

    /// For each of these files this account holds: its size in bytes and whether it is kept (the same rule as
    /// [`Self::kept_rows`]), in ONE query. A file with no row is left out.
    pub fn sizes_and_kept(&self, file_ids: &[String]) -> Result<std::collections::HashMap<String, (i64, bool)>> {
        if file_ids.is_empty() {
            return Ok(Default::default());
        }
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let marks = vec!["?"; file_ids.len()].join(",");
        let mut stmt = conn.prepare(&format!(
            "SELECT file_id, size_bytes, {KEPT_SQL} FROM files WHERE file_id IN ({marks})"
        ))?;
        let rows = stmt.query_map(rusqlite::params_from_iter(file_ids.iter()), |row| {
            Ok((row.get(0)?, (row.get(1)?, row.get::<_, bool>(2)?)))
        })?;
        rows.collect()
    }

    /// The count and size of what is kept, worked out by the database. A kept row counts as an item only when no
    /// kept folder holds it (by path: live rows carry the hierarchy in `path`).
    pub fn kept_summary(&self) -> Result<KeptSummary> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            &format!(
                "WITH kept AS (SELECT path, item_kind, size_bytes FROM files WHERE {KEPT_SQL})
                 SELECT
                   (SELECT COUNT(*) FROM kept k WHERE NOT EXISTS (
                      SELECT 1 FROM kept p WHERE substr(k.path, 1, length(p.path) + 1) = p.path || '/')),
                   (SELECT COALESCE(SUM(MAX(size_bytes, 0)), 0) FROM kept WHERE item_kind != 'folder')"
            ),
            [],
            |row| {
                Ok(KeptSummary {
                    items: u64::try_from(row.get::<_, i64>(0)?).unwrap_or(0),
                    bytes: row.get(1)?,
                })
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> (tempfile::TempDir, StateDb) {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        (dir, db)
    }

    fn put(db: &StateDb, id: &str, path: &str, size: i64, folder: bool, status: FileStatus) {
        let entry = FileEntry {
            file_id: id.into(),
            path: path.into(),
            status,
            size_bytes: size,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: if folder { ItemKind::Folder } else { ItemKind::File },
        };
        // As production writes a row: with its contract (the kind lives there too).
        db.upsert_file_with_contract(&entry, |contract| contract.item_kind = entry.item_kind.clone())
            .unwrap();
    }

    /// A kept folder with two files, a kept file elsewhere, a file not kept, a kept file being trashed: the rows and
    /// the numbers count exactly the kept ones, the folder once.
    #[test]
    fn kept_rows_and_summary_count_only_what_is_kept() {
        let (_dir, db) = db();
        // A folder row with a size of its own (as a server can report one): never counted as bytes kept.
        put(&db, "d-a", "A", 4_096, true, FileStatus::Local);
        put(&db, "f-1", "A/one.txt", 100, false, FileStatus::Local);
        put(&db, "f-2", "A/two.txt", 50, false, FileStatus::CloudOnly);
        put(&db, "f-3", "loose.txt", 7, false, FileStatus::Local);
        put(&db, "f-4", "free.txt", 1_000, false, FileStatus::Local);
        put(&db, "f-5", "going.txt", 9, false, FileStatus::Trashing);
        db.set_recursive_pin("d-a", true, 1).unwrap();
        db.set_recursive_pin("f-3", true, 1).unwrap();
        db.set_recursive_pin("f-5", true, 1).unwrap();
        let paths: Vec<String> = db.kept_rows().unwrap().into_iter().map(|e| e.path).collect();
        assert_eq!(paths, ["A", "A/one.txt", "A/two.txt", "loose.txt"]);
        assert_eq!(db.kept_summary().unwrap(), KeptSummary { items: 2, bytes: 157 });
        // A folder released: its contents go with it.
        db.set_recursive_pin("d-a", false, 2).unwrap();
        assert_eq!(db.kept_summary().unwrap(), KeptSummary { items: 1, bytes: 7 });
        assert_eq!(db.kept_rows().unwrap().len(), 1);
    }

    /// S6 R1: the tick names a kept file this PC does not hold, kept by its own pin or by its folder's, with bytes,
    /// nothing queued and no edit held; `error` only when the platform's failed fetch ends there (Windows).
    #[test]
    fn s6_kept_the_tick_names_kept_files_this_pc_does_not_hold() {
        let (_dir, db) = db();
        put(&db, "own", "own.txt", 10, false, FileStatus::CloudOnly);
        put(&db, "d", "D", 0, true, FileStatus::CloudOnly);
        put(&db, "inh", "D/in.txt", 10, false, FileStatus::CloudOnly);
        put(&db, "err", "err.txt", 10, false, FileStatus::Error);
        put(&db, "free", "free.txt", 10, false, FileStatus::CloudOnly);
        put(&db, "here", "here.txt", 10, false, FileStatus::Local);
        put(&db, "down", "down.txt", 10, false, FileStatus::Downloading);
        put(&db, "conf", "conf.txt", 10, false, FileStatus::Conflict);
        put(&db, "trash", "trash.txt", 10, false, FileStatus::Trashing);
        put(&db, "empty", "empty.txt", 0, false, FileStatus::CloudOnly);
        put(&db, "op", "op.txt", 10, false, FileStatus::CloudOnly);
        put(&db, "held", "held.txt", 10, false, FileStatus::CloudOnly);
        for id in [
            "own", "d", "err", "here", "down", "conf", "trash", "empty", "op", "held",
        ] {
            db.set_recursive_pin(id, true, 1).unwrap();
        }
        for sql in [
            "INSERT INTO operation_queue (op_id, kind, file_id) VALUES ('q', 'hydrate_file', 'op')",
            "INSERT INTO held_edits (file_id, reason, fingerprint, detected_at) VALUES ('held', 'base_unknown', 'f', 1)",
        ] {
            db.test_execute(sql).unwrap();
        }
        let named = |failed_too: bool| -> Vec<String> {
            let rows = db.kept_files_to_fetch(100, failed_too).unwrap();
            rows.into_iter().map(|row| row.file_id).collect()
        };
        println!("s6 kept rule: {:?} / {:?}", named(false), named(true));
        assert_eq!(named(false), ["own", "inh"], "own pin and inherited, cloud-only");
        assert_eq!(named(true), ["own", "inh", "err"], "a failed fetch's row too");
        let row = &db.kept_files_to_fetch(1, false).unwrap()[0];
        assert_eq!(
            row,
            &KeptRow {
                file_id: "own".into(),
                path: "own.txt".into(),
                size_bytes: 10
            }
        );
    }

    /// Nothing kept: no rows, zero and zero.
    #[test]
    fn nothing_kept_reads_as_zero() {
        let (_dir, db) = db();
        put(&db, "f-1", "one.txt", 100, false, FileStatus::Local);
        assert!(db.kept_rows().unwrap().is_empty());
        assert_eq!(db.kept_summary().unwrap(), KeptSummary::default());
    }
}
