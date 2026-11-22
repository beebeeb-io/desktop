//! Task dc-103c (design 1640 "Not done" (b)): a placeholder that holds RANGES of a version the server has moved past.
//!
//! A pull that raises a file's version writes the row only. A placeholder wholly on disk with agreed content is
//! emptied by the stale-copy pass (task 2523, `stale_copies.rs`). One that holds only some ranges has no agreed content,
//! so that pass never names it, and populate refreshes the identity only of a placeholder that holds nothing. Its
//! next range session therefore continued the minted (older) version (`range_prep`'s `continues_minted`): a reader
//! got the old bytes for new content, for as long as the ranges stayed.
//!
//! The rule here names those rows; Windows empties the placeholder (`windows_cf::stale_ranges`) and the row is then
//! written with [`StateDb::record_stale_ranges_wiped`]: the current version becomes the placeholder's identity and
//! the record of what was fetched is wiped to empty, so the next open fetches the current version from the server.
//! The write is a compare-and-set on `content_seq` (task 2199), read before the dehydrate while no fetch of the file
//! was in flight: a fetch recorded since keeps its record.

use super::*;

/// The rule, on a `files` row: a file whose placeholder was minted from (or a session recorded) a version that is not
/// the one the listing shows now, with no agreed content (a copy with agreed content is the stale-copy pass's), not
/// mid-transfer or in conflict, nothing queued for it, no held edit, not poisoned (the poison sweep owns that one).
const STALE_RANGES_SQL: &str = "item_kind = 'file' AND status IN ('local', 'cloud_only')
     AND local_hash IS NULL
     AND minted_object_version_id IS NOT NULL AND current_object_version_id IS NOT NULL
     AND minted_object_version_id != current_object_version_id
     AND NOT EXISTS (SELECT 1 FROM operation_queue q WHERE q.file_id = files.file_id)
     AND NOT EXISTS (SELECT 1 FROM held_edits h WHERE h.file_id = files.file_id)
     AND NOT EXISTS (SELECT 1 FROM cf_poison p WHERE p.file_id = files.file_id)";

/// One placeholder the rule names, as it read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleRanges {
    pub file_id: String,
    pub path: String,
    /// The version its ranges are (the row's minted version when read): the write is made only while it still is.
    pub minted_object_version_id: String,
    /// The current version's size and time (the pull wrote them): the emptied placeholder's new metadata.
    pub size_bytes: i64,
    pub modified_at: i64,
}

#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
impl StateDb {
    /// Up to `limit` rows the rule names, oldest row first.
    pub fn list_stale_ranges(&self, limit: usize) -> Result<Vec<StaleRanges>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(&format!(
            "SELECT file_id, path, minted_object_version_id, size_bytes, modified_at
             FROM files WHERE {STALE_RANGES_SQL} ORDER BY rowid LIMIT ?1"
        ))?;
        let rows = stmt.query_map(params![i64::try_from(limit).unwrap_or(i64::MAX)], |row| {
            Ok(StaleRanges {
                file_id: row.get(0)?,
                path: row.get(1)?,
                minted_object_version_id: row.get(2)?,
                size_bytes: row.get(3)?,
                modified_at: row.get(4)?,
            })
        })?;
        rows.collect()
    }

    /// `stale`'s placeholder was emptied (nothing on disk) and given the row's size and time. In one statement, only
    /// while the rule still names the row with the same minted version and `content_seq` still reads `seq_before`
    /// (read before the dehydrate with no fetch in flight): the current version becomes the placeholder's identity
    /// (as at a mint: `NULL` when the row's size is no longer the one the placeholder was given), the session's
    /// recorded format and version id are dropped (they were the older version's), the fetch record is wiped to
    /// empty (`0`) and the row is cloud-only. `false`: the row moved; nothing was written.
    pub fn record_stale_ranges_wiped(&self, stale: &StaleRanges, seq_before: i64) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Ok(conn.execute(
            &format!(
                "UPDATE files SET
                     minted_object_version_id = CASE WHEN size_bytes = ?4 THEN current_object_version_id ELSE NULL END,
                     minted_format = NULL, minted_version_id = NULL,
                     local_hash = NULL, fetched_min_version = 0, fetched_hash = NULL,
                     status = 'cloud_only', cache_path = NULL, cache_bytes = 0
                 WHERE file_id = ?1 AND content_seq = ?2 AND minted_object_version_id = ?3 AND {STALE_RANGES_SQL}"
            ),
            params![
                stale.file_id,
                seq_before,
                stale.minted_object_version_id,
                stale.size_bytes
            ],
        )? == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const V1: &str = "11111111-2222-4333-8444-555555555555";
    const V2: &str = "99999999-2222-4333-8444-555555555555";

    /// File F, minted from V1, with ranges of V1 fetched (a range session recorded its candidate).
    fn file_with_v1_ranges() -> (tempfile::TempDir, StateDb) {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.test_execute(&format!(
            "INSERT INTO files (file_id, path, status, item_kind, size_bytes, modified_at, current_version,
                                current_object_version_id, minted_object_version_id, minted_format, minted_version_id,
                                local_base_version, fetched_min_version)
             VALUES ('F', 'big.mkv', 'cloud_only', 'file', 100, 1700000000, 1, '{V1}', '{V1}', 'v2', 'aa', 0, 0)"
        ))
        .unwrap();
        db.record_fetch_candidate("F", 1).unwrap();
        (dir, db)
    }

    /// The pull applies another device's version V2 (the row only: its version, size and time).
    fn pull_v2(db: &StateDb) {
        db.test_execute(&format!(
            "UPDATE files SET current_version = 2, current_object_version_id = '{V2}', size_bytes = 200,
                              modified_at = 1700000500 WHERE file_id = 'F'"
        ))
        .unwrap();
    }

    fn row(
        db: &StateDb,
    ) -> (
        Option<String>,
        Option<String>,
        Option<(i64, Option<String>)>,
        FileStatus,
    ) {
        let minted = db.minted_version("F").unwrap();
        let format = db.range_identity("F").unwrap().map(|identity| identity.format);
        let status = db.get_file("F").unwrap().unwrap().status;
        (minted, format, db.fetched_content("F").unwrap(), status)
    }

    /// The red test of dc-103c: F's ranges of V1 are cached; the pull applies V2. The rule names F (before, the next
    /// range session continued V1: the old bytes for new content). Once its placeholder is emptied the row is written:
    /// V2 is the identity, so the next hydration is a fresh session of V2 from the server, and the record of V1's
    /// ranges is gone.
    #[test]
    fn range_wipe_after_a_pull_moves_a_partial_placeholder_to_the_current_version() {
        let (_dir, db) = file_with_v1_ranges();
        assert!(db.list_stale_ranges(10).unwrap().is_empty(), "current: nothing stale");
        assert_eq!(
            db.fetched_content("F").unwrap(),
            Some((1, None)),
            "V1's ranges are recorded"
        );
        pull_v2(&db);
        let stale = db.list_stale_ranges(10).unwrap();
        assert_eq!(
            stale,
            vec![StaleRanges {
                file_id: "F".into(),
                path: "big.mkv".into(),
                minted_object_version_id: V1.into(),
                size_bytes: 200,
                modified_at: 1_700_000_500,
            }]
        );
        let seq_before = db.content_seq("F").unwrap().unwrap();
        assert!(db.record_stale_ranges_wiped(&stale[0], seq_before).unwrap());
        assert_eq!(
            row(&db),
            (Some(V2.to_string()), None, None, FileStatus::CloudOnly),
            "V2 is the identity, V1's session record and ranges are gone"
        );
        assert!(db.list_stale_ranges(10).unwrap().is_empty(), "once");
        assert!(
            !db.record_stale_ranges_wiped(&stale[0], db.content_seq("F").unwrap().unwrap())
                .unwrap()
        );
    }

    /// A fetch recorded after the sequence was read (an opener after the handle closed) keeps its record: nothing is
    /// written, and the next pass reads the row again.
    #[test]
    fn range_wipe_keeps_a_fetch_recorded_since() {
        let (_dir, db) = file_with_v1_ranges();
        pull_v2(&db);
        let stale = db.list_stale_ranges(10).unwrap().remove(0);
        let seq_before = db.content_seq("F").unwrap().unwrap();
        db.record_fetch_candidate("F", 1).unwrap();
        assert!(!db.record_stale_ranges_wiped(&stale, seq_before).unwrap());
        assert_eq!(row(&db).0.as_deref(), Some(V1), "untouched");
        assert_eq!(db.fetched_content("F").unwrap(), Some((1, None)));
    }

    /// The rule leaves alone: a copy with agreed content (the stale-copy pass's), one with an upload queued, a held
    /// edit, a poisoned one, one mid-download, a legacy row with no identity; and the size it was given is checked.
    #[test]
    fn range_wipe_rule_names_only_partial_copies_it_owns() {
        let (_dir, db) = file_with_v1_ranges();
        pull_v2(&db);
        for (id, extra) in [
            ("agreed", "local_hash = 'h1', local_base_version = 1"),
            ("queued", "status = 'cloud_only'"),
            ("held", "status = 'cloud_only'"),
            ("poisoned", "status = 'cloud_only'"),
            ("downloading", "status = 'downloading'"),
            ("legacy", "minted_object_version_id = NULL"),
        ] {
            db.test_execute(&format!(
                "INSERT INTO files (file_id, path, status, item_kind, size_bytes, modified_at, current_version,
                                    current_object_version_id, minted_object_version_id)
                 VALUES ('{id}', '{id}.bin', 'local', 'file', 1, 1, 2, '{V2}', '{V1}')"
            ))
            .unwrap();
            db.test_execute(&format!("UPDATE files SET {extra} WHERE file_id = '{id}'"))
                .unwrap();
        }
        for sql in [
            "INSERT INTO operation_queue (op_id, kind, file_id) VALUES ('op', 'upload', 'queued')",
            "INSERT INTO held_edits (file_id, reason, fingerprint, detected_at) VALUES ('held', 'base_unknown', 'f', 1)",
            "INSERT INTO cf_poison (file_id, poisoned_at, reason) VALUES ('poisoned', 1, 'corrupt')",
        ] {
            db.test_execute(sql).unwrap();
        }
        let stale = db.list_stale_ranges(10).unwrap();
        assert_eq!(stale.iter().map(|s| s.file_id.as_str()).collect::<Vec<_>>(), vec!["F"]);
        // The size moved again after the placeholder was given 200: no identity, so nothing on it is trusted.
        db.test_execute("UPDATE files SET size_bytes = 300 WHERE file_id = 'F'")
            .unwrap();
        let seq_before = db.content_seq("F").unwrap().unwrap();
        assert!(db.record_stale_ranges_wiped(&stale[0], seq_before).unwrap());
        assert_eq!(db.minted_version("F").unwrap(), None);
    }
}
