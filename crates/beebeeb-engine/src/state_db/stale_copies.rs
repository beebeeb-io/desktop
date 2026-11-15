//! Task 2523 (F1): a copy on this PC whose agreed content is behind the version the server holds.
//!
//! A pull that raises a file's version writes the row only: the bytes in the sync folder stay the older version's,
//! marked in sync. The rule that decides which copies may be brought to the current version lives here, as ONE query,
//! so it is the same on every platform and testable on Linux. A copy with a change is never one of them: its bytes
//! differ from the agreed content (checked by the caller under its own handle), or its upload is queued, or an edit
//! of it is held. Those keep their bytes, and an edit sent on the older base is parked as the two-versions question.

use super::*;

/// The rule, on a `files` row: wholly ours to refresh. `local`, a file, agreed content recorded at a known version
/// that is behind the current one, nothing queued for the file (an upload, a hydrate this pass queued earlier, a
/// rename) and no held edit.
const STALE_AGREED_COPY_SQL: &str = "status = 'local' AND item_kind = 'file'
     AND local_hash IS NOT NULL AND local_base_version > 0 AND current_version > local_base_version
     AND NOT EXISTS (SELECT 1 FROM operation_queue q WHERE q.file_id = files.file_id)
     AND NOT EXISTS (SELECT 1 FROM held_edits h WHERE h.file_id = files.file_id)";

/// One copy behind its current version, as the rule read it: the caller re-checks the bytes against `local_hash`
/// and acts only while the row still reads the same (`base`, `local_hash`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleCopy {
    pub file_id: String,
    pub path: String,
    pub current_version: i64,
    pub local_base_version: i64,
    pub local_hash: String,
    /// The current version's size and time (the pull wrote them): the placeholder's new metadata on Windows.
    pub size_bytes: i64,
    pub modified_at: i64,
    /// Kept on this PC ("Available offline"): on Windows it is fetched again right after it is emptied.
    pub pinned: bool,
}

impl StateDb {
    /// Up to `limit` copies the rule names, oldest row first.
    pub fn list_stale_agreed_copies(&self, limit: usize) -> Result<Vec<StaleCopy>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(&format!(
            "SELECT file_id, path, current_version, local_base_version, local_hash, size_bytes, modified_at,
                    (pin_state = 'pinned' OR (pin_state = 'inherit' AND inherited_pin_state = 'pinned'))
             FROM files WHERE {STALE_AGREED_COPY_SQL} ORDER BY rowid LIMIT ?1"
        ))?;
        let rows = stmt.query_map(params![i64::try_from(limit).unwrap_or(i64::MAX)], |row| {
            Ok(StaleCopy {
                file_id: row.get(0)?,
                path: row.get(1)?,
                current_version: row.get(2)?,
                local_base_version: row.get(3)?,
                local_hash: row.get(4)?,
                size_bytes: row.get(5)?,
                modified_at: row.get(6)?,
                pinned: row.get(7)?,
            })
        })?;
        rows.collect()
    }

    /// The rule still names `copy`'s row, with the same agreed content: asked again right before the copy changes.
    pub fn stale_copy_still_behind(&self, copy: &StaleCopy) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM files WHERE file_id = ?1 AND local_base_version = ?2 AND local_hash = ?3
                   AND {STALE_AGREED_COPY_SQL})"
            ),
            params![copy.file_id, copy.local_base_version, copy.local_hash],
            |row| row.get(0),
        )
    }

    /// Windows: `copy`'s placeholder was emptied and verified empty under one exclusive handle. The row says so in
    /// one statement, only while the rule still names it with the same agreed content: cloud-only, as "Free up space"
    /// and the poison step record an emptied placeholder. The cloud-only trigger folds the agreed content into the
    /// fetch candidate (version and hash of the older version): an app that still had the older version open and
    /// saves into the empty placeholder makes a change sent on that version (the two-versions question), never one
    /// of unknown content that is held and never sent; a whole fetch of the current version records it as agreed
    /// (`record_whole_fetch`). A kept file is recorded cloud-only too (it is, until it is fetched again). The row keeps
    /// the pulled version's size and time, which the placeholder now carries. `false`: the row moved; nothing was
    /// written.
    pub fn record_stale_copy_emptied(&self, copy: &StaleCopy) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Ok(conn.execute(
            &format!(
                "UPDATE files SET status = 'cloud_only', cache_path = NULL, cache_bytes = 0
                 WHERE file_id = ?1 AND local_base_version = ?2 AND local_hash = ?3 AND {STALE_AGREED_COPY_SQL}"
            ),
            params![copy.file_id, copy.local_base_version, copy.local_hash],
        )? == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A state database with `rows`: `(file_id, status, current, base, local_hash)`, each a file at `<id>.txt`.
    fn db_with(rows: &[(&str, &str, i64, i64, Option<&str>)]) -> (tempfile::TempDir, StateDb) {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        for (id, status, current, base, hash) in rows {
            let hash = hash.map_or("NULL".to_string(), |h| format!("'{h}'"));
            db.test_execute(&format!(
                "INSERT INTO files (file_id, path, status, item_kind, size_bytes, modified_at, current_version,
                                    local_base_version, local_hash, fetched_min_version)
                 VALUES ('{id}', '{id}.txt', '{status}', 'file', 17, 1700000500, {current}, {base}, {hash}, 0)"
            ))
            .unwrap();
        }
        (dir, db)
    }

    fn ids(copies: &[StaleCopy]) -> Vec<&str> {
        copies.iter().map(|copy| copy.file_id.as_str()).collect()
    }

    /// The rule names a `local` file whose agreed content is behind its current version, and nothing else: not a file
    /// at its current version, nor one with no agreed content or an unknown base, nor one not on this PC, nor one with
    /// anything queued or an edit held.
    #[test]
    fn the_rule_names_only_an_agreed_copy_behind_its_current_version() {
        let (_dir, db) = db_with(&[
            ("behind", "local", 2, 1, Some("h1")),
            ("current", "local", 2, 2, Some("h2")),
            ("no-content", "local", 2, 1, None),
            ("no-base", "local", 2, 0, Some("h0")),
            ("cloud", "cloud_only", 2, 1, Some("h1")),
            ("conflict", "conflict", 2, 1, Some("h1")),
            ("queued", "local", 2, 1, Some("h1")),
            ("held", "local", 2, 1, Some("h1")),
            ("pinned", "local", 3, 1, Some("h1")),
        ]);
        for sql in [
            "INSERT INTO operation_queue (op_id, kind, file_id) VALUES ('op', 'hydrate_file', 'queued')",
            "INSERT INTO held_edits (file_id, reason, fingerprint, detected_at) VALUES ('held', 'base_unknown', 'f', 1)",
            "UPDATE files SET pin_state = 'pinned' WHERE file_id = 'pinned'",
        ] {
            db.test_execute(sql).unwrap();
        }
        let copies = db.list_stale_agreed_copies(10).unwrap();
        assert_eq!(ids(&copies), vec!["behind", "pinned"]);
        assert_eq!(
            copies[1],
            StaleCopy {
                file_id: "pinned".into(),
                path: "pinned.txt".into(),
                current_version: 3,
                local_base_version: 1,
                local_hash: "h1".into(),
                size_bytes: 17,
                modified_at: 1_700_000_500,
                pinned: true,
            }
        );
        assert!(!copies[0].pinned);
        assert_eq!(
            ids(&db.list_stale_agreed_copies(1).unwrap()),
            vec!["behind"],
            "at most `limit`"
        );
        assert!(db.stale_copy_still_behind(&copies[0]).unwrap());
        let other_content = StaleCopy {
            local_hash: "h-other".into(),
            ..copies[0].clone()
        };
        assert!(
            !db.stale_copy_still_behind(&other_content).unwrap(),
            "only with the agreed content it was read with"
        );
    }

    /// Windows' row write after a verified-empty dehydrate: one compare-and-set on the content it checked. The row is
    /// cloud-only, keeps the pulled version's size and time, and its older agreed content joins the fetch candidate:
    /// an app that saves into the empty placeholder with the older version still open makes a change sent on version
    /// 1 (the two-versions question), never bytes of unknown content that would be held and never sent. A row that
    /// moved is not written.
    #[test]
    fn an_emptied_copy_is_recorded_once_and_only_as_it_was_checked() {
        let (_dir, db) = db_with(&[("f", "local", 2, 1, Some("h1")), ("g", "local", 2, 1, Some("h1"))]);
        db.test_execute("UPDATE files SET pin_state = 'pinned' WHERE file_id = 'f'")
            .unwrap();
        let copies = db.list_stale_agreed_copies(10).unwrap();
        let moved = StaleCopy {
            local_hash: "h-other".into(),
            ..copies[1].clone()
        };
        assert!(
            !db.record_stale_copy_emptied(&moved).unwrap(),
            "a row that moved is not written"
        );
        assert!(db.record_stale_copy_emptied(&copies[0]).unwrap(), "a kept file too");
        assert!(!db.record_stale_copy_emptied(&copies[0]).unwrap(), "once");
        let row = db.get_file("f").unwrap().unwrap();
        let contract = db.get_file_contract_state("f").unwrap().unwrap();
        assert_eq!(row.status, FileStatus::CloudOnly);
        assert_eq!(
            (row.size_bytes, row.modified_at),
            (17, 1_700_000_500),
            "the pulled version's"
        );
        assert_eq!(contract.local_hash, None);
        assert_eq!(
            db.fetched_content("f").unwrap(),
            Some((1, Some("h1".to_string()))),
            "the older version's content is the candidate"
        );
        assert!(!db.is_unknown_content("f").unwrap(), "not a file of unknown content");
        assert_eq!(ids(&db.list_stale_agreed_copies(10).unwrap()), vec!["g"]);
        // An editor that still had version 1 open saves into the emptied placeholder, before any fetch.
        let dir = tempfile::tempdir().unwrap();
        let place = dir.path().join("f.txt");
        std::fs::write(&place, b"saved over the emptied placeholder").unwrap();
        assert_eq!(
            crate::engine_bridge::sync_folder_content(&db, "f", &place, None).unwrap(),
            crate::engine_bridge::SyncFolderContent::Changed { base: Some(1) },
            "a change sent on version 1: the two-versions question, never held as content unknown"
        );
    }

    /// F2 (task 2523): a landing that did not write the file's place (the viewer on Windows and Linux) moves the
    /// current version only; the place keeps its agreed version and content, so it reads as behind. One that did
    /// (a sync-folder upload, a macOS File Provider write) records the landed version, as before.
    #[test]
    fn a_landing_that_did_not_write_the_place_leaves_its_agreed_content() {
        let rekey = |op: &PendingOperation, _: &str| -> std::result::Result<Option<String>, StateDbError> {
            Ok(op.metadata_json.clone())
        };
        let landing = |file_id: &'static str, place_written: bool| LandingInput {
            op_id: "viewer-op",
            claim_id: None,
            write_id: Some("w-viewer"),
            local_file_id: file_id,
            server_file_id: file_id,
            target_path: None,
            parent_id: None,
            landed_base: Some(4),
            produced_version: 5,
            produced_object_version_id: "object-5",
            size_bytes: 19,
            content_type: None,
            mime_type: None,
            completed_payload: Some("/staged/viewer"),
            completed_payload_hash: None,
            place_written,
            finalization: None,
            now: 30,
        };
        let (_dir, db) = db_with(&[
            ("viewer", "local", 4, 4, Some("h4")),
            ("provider", "local", 4, 4, Some("h4")),
            ("no-content", "local", 4, 4, None),
        ]);
        db.apply_landing(&landing("viewer", false), &rekey)
            .unwrap()
            .expect("it lands");
        db.apply_landing(&landing("provider", true), &rekey)
            .unwrap()
            .expect("it lands");
        db.apply_landing(&landing("no-content", false), &rekey)
            .unwrap()
            .expect("it lands");
        let read = |id| {
            let c = db.get_file_contract_state(id).unwrap().unwrap();
            (c.current_version, c.local_base_version, c.local_hash)
        };
        assert_eq!(
            read("viewer"),
            (5, 4, Some("h4".to_string())),
            "behind: agreed at 4, current 5"
        );
        assert_eq!(read("provider"), (5, 5, None), "as before");
        assert_eq!(read("no-content"), (5, 5, None), "nothing agreed to keep: as before");
        assert_eq!(ids(&db.list_stale_agreed_copies(10).unwrap()), vec!["viewer"]);
    }
}
