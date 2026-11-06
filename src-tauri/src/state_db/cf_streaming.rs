//! LD-0105 (Windows Cloud Files streaming), the local records:
//!
//! - **The placeholder's minted version (task 2180, Option 1; spec S3, I4).** When a placeholder is
//!   minted, the version the listing showed for it (`files.current_object_version_id`) is copied to
//!   `files.minted_object_version_id`. A fetch pins to it; a later version means "refresh the
//!   placeholder", never "serve the new version's bytes into it". A row with no record (an upgrade
//!   from 0.8.21, a lost or reset state.db, a legacy file) has no identity: bytes already on disk
//!   for it are never trusted ([`crate::download_pin::placeholder_identity`]).
//! - **Poisoned placeholders (spec S6, I5).** A placeholder a failed fetch wrote into is recorded
//!   here, durably, until the empty step (`windows_cf::poison`) has dehydrated it. Every fetch of a
//!   poisoned file fails at once with no request, and the record survives a restart (no close event
//!   comes after one), so the start-up sweep still empties it.
//!
//! Nothing here is secret: file ids, a version id, times.

use super::*;

/// One poisoned placeholder (spec S6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoisonRow {
    pub file_id: String,
    /// Unix seconds.
    pub poisoned_at: i64,
    /// A status word (`corrupt`, `version_gone`, `identity_mismatch`, …): never error text.
    pub reason: String,
    /// The placeholder's change stamp right after it was marked not in sync (`ChangeTime` in 100 ns
    /// ticks and the file index): the empty step dehydrates only while it is unchanged, so a write the
    /// person made since is never thrown away. `None`: not read; the empty step then refuses.
    pub change_ticks: Option<i64>,
    pub file_index: Option<i64>,
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
impl StateDb {
    /// Task 2180: the placeholder of `file_id` was just minted (or refreshed) from the listing with
    /// `minted_size` bytes: record the version the listing shows for it now as its identity, but only
    /// while the row still has that size. When a newer listing changed the size between the snapshot
    /// and the mint, `NULL` is recorded: no identity, so nothing is ever trusted over that placeholder.
    /// (An equal-size newer version is recorded as it is: the placeholder holds no bytes at its mint,
    /// so its first fetch serves that version whole.) A listing with no version also records `NULL`.
    /// `Ok(true)`: a row was updated.
    pub fn record_minted_version(&self, file_id: &str, minted_size: i64) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Ok(conn.execute(
            "UPDATE files SET minted_object_version_id =
               CASE WHEN size_bytes = ?2 THEN current_object_version_id ELSE NULL END
             WHERE file_id = ?1",
            params![file_id, minted_size],
        )? == 1)
    }

    /// Review P2 (task 2180): the files whose placeholder identity is not the version the listing shows
    /// now (none recorded, or another one), among those whose listing names a version.
    pub fn files_needing_identity(&self) -> Result<std::collections::HashSet<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT file_id FROM files WHERE current_object_version_id IS NOT NULL
               AND minted_object_version_id IS NOT current_object_version_id",
        )?;
        let ids = stmt.query_map([], |row| row.get::<_, String>(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(ids)
    }

    /// Test only: the listing's version and the recorded identity of `file_id`, as a 0.8.21 database
    /// holds them (a version, no identity). Its callers are Windows-native tests.
    #[cfg(all(test, windows))]
    pub(crate) fn set_versions_for_test(&self, file_id: &str, current: Option<&str>, minted: Option<&str>) {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "UPDATE files SET current_object_version_id = ?2, minted_object_version_id = ?3 WHERE file_id = ?1",
            params![file_id, current, minted],
        )
        .unwrap();
    }

    /// Task 2180: the version `file_id`'s placeholder was minted from; `None` when nothing was recorded.
    pub fn minted_version(&self, file_id: &str) -> Result<Option<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Ok(conn
            .query_row(
                "SELECT minted_object_version_id FROM files WHERE file_id = ?1",
                params![file_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten())
    }

    /// Spec S6: record `file_id` as poisoned (a later record for the same file replaces the earlier one).
    pub fn record_poison(&self, file_id: &str, reason: &str, stamp: Option<(i64, i64)>, now: i64) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "INSERT INTO cf_poison(file_id, poisoned_at, reason, change_ticks, file_index)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(file_id) DO UPDATE SET
               poisoned_at = excluded.poisoned_at, reason = excluded.reason,
               change_ticks = excluded.change_ticks, file_index = excluded.file_index",
            params![file_id, now, reason, stamp.map(|s| s.0), stamp.map(|s| s.1)],
        )?;
        Ok(())
    }

    /// Spec I5: is `file_id` poisoned? Every fetch of it fails at once while it is.
    pub fn poison(&self, file_id: &str) -> Result<Option<PoisonRow>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Ok(conn
            .query_row(
                "SELECT file_id, poisoned_at, reason, change_ticks, file_index FROM cf_poison WHERE file_id = ?1",
                params![file_id],
                poison_row,
            )
            .optional()?)
    }

    /// Spec S6: every poisoned placeholder, oldest first (the sweep and the start-up pass).
    pub fn poisoned(&self) -> Result<Vec<PoisonRow>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT file_id, poisoned_at, reason, change_ticks, file_index FROM cf_poison ORDER BY poisoned_at, file_id",
        )?;
        let rows = stmt.query_map([], poison_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Review P1: a failed empty step restored the file with our own writes, which move its stamp; the
    /// poison takes the stamp they left, so the next sweep can empty the file. `Ok(true)`: it was there.
    pub fn restamp_poison(&self, file_id: &str, stamp: (i64, i64)) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Ok(conn.execute(
            "UPDATE cf_poison SET change_ticks = ?2, file_index = ?3 WHERE file_id = ?1",
            params![file_id, stamp.0, stamp.1],
        )? == 1)
    }

    /// Spec S6 step 7: the placeholder is verifiably empty; the poison goes. `Ok(true)`: it was there.
    pub fn clear_poison(&self, file_id: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Ok(conn.execute("DELETE FROM cf_poison WHERE file_id = ?1", params![file_id])? == 1)
    }

    /// Test only: every read of a `files` row fails from now on (the table is renamed away), while
    /// `cf_poison` stays readable. Its one caller is a Windows-native test.
    #[cfg(all(test, windows))]
    pub(crate) fn make_file_rows_unreadable_for_test(&self) {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute_batch("ALTER TABLE files RENAME TO files_unreadable_for_test").unwrap();
    }
}

fn poison_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PoisonRow> {
    Ok(PoisonRow {
        file_id: row.get(0)?,
        poisoned_at: row.get(1)?,
        reason: row.get(2)?,
        change_ticks: row.get(3)?,
        file_index: row.get(4)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db_with(file_id: &str, current: Option<&str>) -> (tempfile::TempDir, StateDb) {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.upsert_file(&FileEntry {
            file_id: file_id.into(),
            path: "/a.mp4".into(),
            status: FileStatus::CloudOnly,
            size_bytes: 10,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
        set_current(&db, file_id, current);
        (dir, db)
    }

    fn set_current(db: &StateDb, file_id: &str, current: Option<&str>) {
        let conn = db.0.lock().unwrap();
        conn.execute(
            "UPDATE files SET current_object_version_id = ?2 WHERE file_id = ?1",
            params![file_id, current],
        )
        .unwrap();
    }

    const A: &str = "11111111-2222-4333-8444-555555555555";
    const B: &str = "99999999-2222-4333-8444-555555555555";

    #[test]
    fn task_2180_the_minted_version_is_the_listings_at_mint_and_stays_when_the_listing_moves_on() {
        let (_dir, db) = db_with("f", Some(A));
        assert_eq!(
            db.minted_version("f").unwrap(),
            None,
            "nothing recorded before the mint"
        );
        assert!(db.record_minted_version("f", 10).unwrap());
        assert_eq!(db.minted_version("f").unwrap().as_deref(), Some(A));
        // B commits on the server; the listing moves on, the placeholder's identity does not.
        set_current(&db, "f", Some(B));
        assert_eq!(
            db.minted_version("f").unwrap().as_deref(),
            Some(A),
            "the placeholder still holds A"
        );
        // A refresh of the placeholder records the new identity.
        assert!(db.record_minted_version("f", 10).unwrap());
        assert_eq!(db.minted_version("f").unwrap().as_deref(), Some(B));
        // A listing that changed the size after the snapshot the placeholder was minted from: no identity.
        assert!(db.record_minted_version("f", 11).unwrap());
        assert_eq!(
            db.minted_version("f").unwrap(),
            None,
            "minted from another size: never trusted"
        );
    }

    #[test]
    fn task_2180_a_listing_with_no_version_records_none_and_an_unknown_file_has_none() {
        let (_dir, db) = db_with("legacy", None);
        assert!(db.record_minted_version("legacy", 10).unwrap());
        assert_eq!(db.minted_version("legacy").unwrap(), None);
        assert!(!db.record_minted_version("absent", 10).unwrap());
        assert_eq!(db.minted_version("absent").unwrap(), None);
    }

    #[test]
    fn task_2180_the_column_is_added_to_a_database_from_before_it() {
        // A 0.8.21 state.db has no `minted_object_version_id`: opening it adds the column, every row NULL.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        {
            let db = StateDb::open(&path).unwrap();
            db.0.lock()
                .unwrap()
                .execute_batch("ALTER TABLE files DROP COLUMN minted_object_version_id;")
                .unwrap();
        }
        let db = StateDb::open(&path).unwrap();
        db.upsert_file(&FileEntry {
            file_id: "old".into(),
            path: "/old.bin".into(),
            status: FileStatus::CloudOnly,
            size_bytes: 1,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
        assert_eq!(db.minted_version("old").unwrap(), None);
    }

    #[test]
    fn ld0105_s6_a_poison_is_durable_until_cleared_and_the_sweep_lists_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        {
            let db = StateDb::open(&path).unwrap();
            assert_eq!(db.poison("f").unwrap(), None);
            db.record_poison("f", "corrupt", Some((133_000_000_000_000_000, 42)), 100)
                .unwrap();
            db.record_poison("g", "identity_mismatch", None, 50).unwrap();
        }
        // A restart: the record is still there (no close event comes after one).
        let db = StateDb::open(&path).unwrap();
        let f = db.poison("f").unwrap().expect("still poisoned after a restart");
        assert_eq!(
            (f.reason.as_str(), f.change_ticks, f.file_index),
            ("corrupt", Some(133_000_000_000_000_000), Some(42))
        );
        assert_eq!(
            db.poisoned()
                .unwrap()
                .iter()
                .map(|r| r.file_id.as_str())
                .collect::<Vec<_>>(),
            vec!["g", "f"],
            "oldest first"
        );
        // A second failure of the same file replaces the record.
        db.record_poison("f", "version_gone", None, 200).unwrap();
        assert_eq!(db.poison("f").unwrap().unwrap().change_ticks, None);
        assert!(db.clear_poison("f").unwrap());
        assert!(!db.clear_poison("f").unwrap());
        assert_eq!(db.poison("f").unwrap(), None);
    }
}
