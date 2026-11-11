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
    /// The file's content key right after it was marked not in sync (contract Amendment 4):
    /// `(LastWriteTime in 100 ns ticks since 1970, size)`. Our own restore writes (not in sync, the pin,
    /// in sync; by handle or by path) change metadata only, so an unchanged key means the bytes are the
    /// failed attempt's own and the empty step may run. `None`: not read; the row then needs attention.
    pub key: Option<(i64, i64)>,
    /// Amendment 4: the key changed (something wrote to the file) or was never read. The file is never
    /// emptied and never cleared silently; only "Discard and download again" ends the row.
    pub needs_attention: bool,
}

/// The rest of a placeholder's identity (I3, I4), recorded before its first transfer: the chunk
/// format word (`v1` / `v2`) and a v2 file's version id (frame 0's nonce, hex).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedIdentity {
    pub object_version_id: String,
    pub format: String,
    pub version_id_hex: Option<String>,
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
        let ids = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<_>>()?;
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

    /// Spec S6: record `file_id` as poisoned with its content `key` (Amendment 4). A later record for the
    /// same file replaces the earlier one, and starts it again as a row the sweep may empty.
    pub fn record_poison(&self, file_id: &str, reason: &str, key: Option<(i64, i64)>, now: i64) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "INSERT INTO cf_poison(file_id, poisoned_at, reason, write_ticks, key_size, needs_attention)
             VALUES (?1, ?2, ?3, ?4, ?5, 0)
             ON CONFLICT(file_id) DO UPDATE SET
               poisoned_at = excluded.poisoned_at, reason = excluded.reason,
               write_ticks = excluded.write_ticks, key_size = excluded.key_size,
               needs_attention = 0",
            params![file_id, now, reason, key.map(|k| k.0), key.map(|k| k.1)],
        )?;
        Ok(())
    }

    /// Amendment 4: the poisoned file changed, or its key was never read. It stays fetch-refused and is
    /// shown as needing attention until the person discards it. `Ok(true)`: the row was there.
    pub fn mark_poison_needs_attention(&self, file_id: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Ok(conn.execute(
            "UPDATE cf_poison SET needs_attention = 1 WHERE file_id = ?1",
            params![file_id],
        )? == 1)
    }

    /// Amendment 4 (desktop #225 review P2): a pin state (`CF_PIN_STATE` value) the empty step could put
    /// back neither through its handle nor by path. Kept in its own table, so it outlives the poison
    /// (an empty that succeeded clears the poison right after); every sweep retries it by path until
    /// it holds ("Always keep on this device" is never silently lost).
    pub fn record_pin_to_restore(&self, file_id: &str, pin: i32) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "INSERT INTO cf_pin_restore(file_id, pin) VALUES (?1, ?2)
             ON CONFLICT(file_id) DO UPDATE SET pin = excluded.pin",
            params![file_id, pin],
        )?;
        Ok(())
    }

    /// The pin to restore for `file_id`, if one is waiting.
    pub fn pin_to_restore(&self, file_id: &str) -> Result<Option<i32>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Ok(conn
            .query_row(
                "SELECT pin FROM cf_pin_restore WHERE file_id = ?1",
                params![file_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Every pin waiting to be restored, `(file_id, pin)`.
    pub fn pins_to_restore(&self) -> Result<Vec<(String, i32)>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare("SELECT file_id, pin FROM cf_pin_restore ORDER BY file_id")?;
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// The pin of `file_id` holds again (or there is no placeholder left to pin). `Ok(true)`: one was waiting.
    pub fn clear_pin_to_restore(&self, file_id: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Ok(conn.execute("DELETE FROM cf_pin_restore WHERE file_id = ?1", params![file_id])? == 1)
    }

    /// LD-0105 W2 (I3, I4, task 2180): a range session is about to transfer its first byte into the
    /// placeholder of `file_id`; its identity is recorded first. The minted version is the one the
    /// session pins (it equals the recorded one, or none was recorded and nothing was on disk).
    pub fn record_range_identity(&self, file_id: &str, identity: &RecordedIdentity) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Ok(conn.execute(
            "UPDATE files SET minted_object_version_id = ?2, minted_format = ?3, minted_version_id = ?4 WHERE file_id = ?1",
            params![file_id, identity.object_version_id, identity.format, identity.version_id_hex],
        )? == 1)
    }

    /// The identity a range session recorded for `file_id`'s placeholder; `None` when no session ever
    /// recorded one (only a minted version, or nothing).
    pub fn range_identity(&self, file_id: &str) -> Result<Option<RecordedIdentity>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Ok(conn
            .query_row(
                "SELECT minted_object_version_id, minted_format, minted_version_id FROM files WHERE file_id = ?1",
                params![file_id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()?
            .and_then(|(version, format, id)| {
                Some(RecordedIdentity {
                    object_version_id: version?,
                    format: format?,
                    version_id_hex: id,
                })
            }))
    }

    /// Spec S9 (task 1640): a range session records its candidate `(version, NULL hash)` before its
    /// first transfer. A NULL hash is never promotable: only [`StateDb::record_local_baseline`], after
    /// the session's one "whole" verdict, makes the bytes agreed content. Like `record_fetched_version`,
    /// the candidate only moves down or becomes mixed.
    pub fn record_fetch_candidate(&self, file_id: &str, version: i64) -> Result<()> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        super::sql::fold_agreed_into_candidate_conn(&tx, file_id)?;
        tx.execute(
            "UPDATE files SET
                 fetched_hash = NULL,
                 fetched_min_version = CASE
                     WHEN fetched_min_version = 0 THEN ?2
                     WHEN fetched_min_version < 0 THEN -2
                     ELSE MIN(fetched_min_version, ?2) END
             WHERE file_id = ?1",
            params![file_id, version],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Spec I5: is `file_id` poisoned? Every fetch of it fails at once while it is.
    pub fn poison(&self, file_id: &str) -> Result<Option<PoisonRow>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Ok(conn
            .query_row(
                "SELECT file_id, poisoned_at, reason, write_ticks, key_size, needs_attention FROM cf_poison WHERE file_id = ?1",
                params![file_id],
                poison_row,
            )
            .optional()?)
    }

    /// Spec S6: every poisoned placeholder, oldest first (the sweep and the start-up pass).
    pub fn poisoned(&self) -> Result<Vec<PoisonRow>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT file_id, poisoned_at, reason, write_ticks, key_size, needs_attention FROM cf_poison ORDER BY poisoned_at, file_id",
        )?;
        let rows = stmt.query_map([], poison_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
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
        conn.execute_batch("ALTER TABLE files RENAME TO files_unreadable_for_test")
            .unwrap();
    }
}

fn poison_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PoisonRow> {
    let ticks: Option<i64> = row.get(3)?;
    let size: Option<i64> = row.get(4)?;
    Ok(PoisonRow {
        file_id: row.get(0)?,
        poisoned_at: row.get(1)?,
        reason: row.get(2)?,
        key: ticks.zip(size),
        needs_attention: row.get::<_, i64>(5)? != 0,
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
            (f.reason.as_str(), f.key, f.needs_attention),
            ("corrupt", Some((133_000_000_000_000_000, 42)), false)
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
        assert_eq!(db.poison("f").unwrap().unwrap().key, None);
        assert!(db.clear_poison("f").unwrap());
        assert!(!db.clear_poison("f").unwrap());
        assert_eq!(db.poison("f").unwrap(), None);
    }

    #[test]
    fn ld0105_a4_needs_attention_is_durable_and_a_new_poison_starts_clean() {
        let (_dir, db) = db_with("f", Some(A));
        db.record_poison("f", "corrupt", Some((1, 2)), 1).unwrap();
        assert!(db.mark_poison_needs_attention("f").unwrap());
        assert!(db.poison("f").unwrap().unwrap().needs_attention);
        // A new failure of the same file is a new attempt's bytes: the sweep may empty it again.
        db.record_poison("f", "identity_mismatch", Some((3, 4)), 2).unwrap();
        let row = db.poison("f").unwrap().unwrap();
        assert_eq!((row.key, row.needs_attention), (Some((3, 4)), false));
        assert!(!db.mark_poison_needs_attention("nobody").unwrap());
    }

    /// Desktop #225 review P2 (poison.rs:429): a pin the empty step could not put back is kept after
    /// the empty succeeded and its poison was cleared, across a restart, until it holds. Red on the
    /// mutant "clearing the poison clears the pin with it" (the old `pending_pin` on the poison row).
    #[test]
    fn ld0105_p2_a_pin_to_restore_outlives_the_poison_it_came_from() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        {
            let db = StateDb::open(&path).unwrap();
            db.record_poison("f", "corrupt", Some((1, 2)), 1).unwrap();
            db.record_pin_to_restore("f", 1).unwrap();
            assert!(db.clear_poison("f").unwrap(), "the empty step succeeded");
        }
        let db = StateDb::open(&path).unwrap();
        assert_eq!(db.poison("f").unwrap(), None);
        assert_eq!(db.pin_to_restore("f").unwrap(), Some(1), "the pin is still to restore");
        assert_eq!(db.pins_to_restore().unwrap(), vec![("f".to_string(), 1)]);
        db.record_poison("f", "corrupt", Some((3, 4)), 2).unwrap();
        assert_eq!(
            db.pin_to_restore("f").unwrap(),
            Some(1),
            "a new poison does not drop it either"
        );
        assert!(db.clear_pin_to_restore("f").unwrap());
        assert!(!db.clear_pin_to_restore("f").unwrap());
        assert_eq!(db.pins_to_restore().unwrap(), Vec::<(String, i32)>::new());
    }

    #[test]
    fn ld0105_w2_a_range_identity_is_recorded_whole_or_read_as_none() {
        let (_dir, db) = db_with("f", Some(A));
        assert_eq!(db.range_identity("f").unwrap(), None, "nothing recorded");
        assert!(db.record_minted_version("f", 10).unwrap());
        assert_eq!(
            db.range_identity("f").unwrap(),
            None,
            "a minted version alone is no session identity"
        );
        let id = RecordedIdentity {
            object_version_id: A.into(),
            format: "v2".into(),
            version_id_hex: Some("00ff".into()),
        };
        assert!(db.record_range_identity("f", &id).unwrap());
        assert_eq!(db.range_identity("f").unwrap(), Some(id));
        assert_eq!(db.minted_version("f").unwrap().as_deref(), Some(A));
    }

    /// Spec S9 (task 1640): the session's candidate carries no hash, so it can never be promoted by the
    /// stamp pass; only `record_local_baseline` after "whole" makes agreed content.
    #[test]
    fn ld0105_s9_the_range_candidate_has_no_hash_and_only_moves_down() {
        let (_dir, db) = db_with("f", Some(A));
        db.mark_no_fetched_content("f").unwrap();
        db.record_fetch_candidate("f", 7).unwrap();
        assert_eq!(db.fetched_content("f").unwrap(), Some((7, None)));
        db.record_fetch_candidate("f", 9).unwrap();
        assert_eq!(db.fetched_content("f").unwrap(), Some((7, None)), "only moves down");
        db.record_local_baseline("f", 9, "h").unwrap();
        assert_eq!(db.fetched_content("f").unwrap(), None, "a whole write replaces it");
    }
}
