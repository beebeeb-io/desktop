//! Folder custody, owned keys and the shared-index dirty flags (split out of `state_db.rs`).

use super::*;

impl StateDb {
    /// One bounded maintenance pass per 30 seconds; account purge clears this clock.
    pub fn claim_owned_key_maintenance(&self, now: i64) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Ok(conn.execute("INSERT INTO sync_state (key,value) VALUES ('owned_key_retry_after',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value WHERE CAST(sync_state.value AS INTEGER)<=?2", params![now.saturating_add(30).to_string(),now])? > 0)
    }
    /// L7d: device-wide, durable format knowledge; contains no key material.
    /// Nothing the server says and no sign-out path removes an entry (see
    /// `purge_all_local_state`, `clear_account_data` and `finish_windows_signout`).
    pub fn has_seen_folder_custody(&self, folder_id: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Self::has_seen_folder_custody_conn(&conn, folder_id)
    }
    /// L7d: the set stores canonical lowercase hyphenated UUIDs only. A write
    /// with any other form is refused, so an id shape the server reports can
    /// never create a second entry for a folder this device has seen.
    fn custody_seen_key(folder_id: &str) -> Result<String> {
        match uuid::Uuid::parse_str(folder_id) {
            Ok(canonical) if canonical.hyphenated().to_string() == folder_id => {
                Ok(format!("shared_folder_custody_seen:{folder_id}"))
            }
            _ => Err(rusqlite::Error::InvalidParameterName(folder_id.to_string())),
        }
    }
    /// The same key for a read, canonicalized: a folder id reported in another
    /// UUID form (upper case, no hyphens, braced) maps to the entry its
    /// canonical form stored, so a server cannot shrink the set by re-spelling
    /// an id. An id that is not a UUID cannot be in the set (a real folder id
    /// always is), so it reads as never seen rather than failing every read.
    fn custody_lookup_key(folder_id: &str) -> Option<String> {
        uuid::Uuid::parse_str(folder_id)
            .ok()
            .map(|canonical| format!("shared_folder_custody_seen:{}", canonical.hyphenated()))
    }
    fn has_seen_folder_custody_conn(conn: &Connection, folder_id: &str) -> Result<bool> {
        let Some(key) = Self::custody_lookup_key(folder_id) else {
            return Ok(false);
        };
        Ok(conn
            .query_row("SELECT 1 FROM sync_state WHERE key=?1", [key], |_| Ok(()))
            .optional()?
            .is_some())
    }
    /// Serialize legacy key opening with custody observation. An old inventory
    /// cannot authorize a read after another request durably remembers custody.
    /// The synchronous callback only opens a key in core; it must not use the DB.
    pub fn read_without_folder_custody<T>(&self, folder_id: &str, read: impl FnOnce() -> T) -> Result<Option<T>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        if Self::has_seen_folder_custody_conn(&conn, folder_id)? {
            return Ok(None);
        }
        Ok(Some(read()))
    }
    pub fn remember_folder_custody_at_revision(&self, folder_id: &str, revision: u64) -> Result<()> {
        let key = Self::custody_seen_key(folder_id)?;
        let conn = self.0.lock().expect("state_db mutex poisoned");
        if self.1.load(std::sync::atomic::Ordering::Acquire) != revision {
            return Err(rusqlite::Error::InvalidQuery);
        }
        conn.execute(
            "INSERT INTO sync_state (key,value) VALUES (?1,'1') ON CONFLICT(key) DO NOTHING",
            [key],
        )?;
        Ok(())
    }
    pub fn mark_shared_index_dirty(&self, folder_id: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute("INSERT INTO sync_state (key,value) VALUES (?1,'1') ON CONFLICT(key) DO UPDATE SET value=CAST(CAST(value AS INTEGER)+1 AS TEXT)", [format!("shared_folder_index_dirty:{folder_id}")])?;
        Ok(())
    }
    pub fn dirty_shared_index_roots(&self) -> Result<Vec<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt =
            conn.prepare("SELECT substr(key,27) FROM sync_state WHERE key LIKE 'shared_folder_index_dirty:%'")?;
        stmt.query_map([], |r| r.get(0))?.collect()
    }
    #[cfg(test)]
    pub fn clear_shared_index_dirty(&self, folder_id: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "DELETE FROM sync_state WHERE key=?1",
            [format!("shared_folder_index_dirty:{folder_id}")],
        )?;
        Ok(())
    }
    pub fn shared_index_dirty_revision(&self, id: &str) -> Result<Option<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            "SELECT value FROM sync_state WHERE key=?1",
            [format!("shared_folder_index_dirty:{id}")],
            |r| r.get(0),
        )
        .optional()
    }
    pub fn clear_shared_index_dirty_at_revision(&self, id: &str, revision: Option<&str>) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "DELETE FROM sync_state WHERE key=?1 AND value=?2",
            params![format!("shared_folder_index_dirty:{id}"), revision],
        )?;
        Ok(())
    }
    /// Task 2279 item 1: index work that cannot run before the root has
    /// custody. It is kept durably instead of being cleared, but is not part of
    /// the dirty set, so maintenance does not retry it every 30 seconds.
    pub fn defer_shared_index_until_custody(&self, folder_id: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "INSERT INTO sync_state (key,value) VALUES (?1,'1') ON CONFLICT(key) DO NOTHING",
            [format!("shared_folder_index_deferred:{folder_id}")],
        )?;
        Ok(())
    }
    /// Custody became available for the root: revive the deferred repair by
    /// marking the index dirty again. Returns whether work was waiting.
    pub fn revive_deferred_shared_index(&self, folder_id: &str) -> Result<bool> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        // Both statements in one transaction: a failure or crash between the delete and the dirty marker must not
        // consume the deferral, or nobody would ever schedule the repair again.
        let tx = conn.transaction()?;
        let removed = tx.execute(
            "DELETE FROM sync_state WHERE key=?1",
            [format!("shared_folder_index_deferred:{folder_id}")],
        )?;
        if removed > 0 {
            tx.execute(
                "INSERT INTO sync_state (key,value) VALUES (?1,'1') ON CONFLICT(key) DO UPDATE SET value=CAST(CAST(value AS INTEGER)+1 AS TEXT)",
                [format!("shared_folder_index_dirty:{folder_id}")],
            )?;
        }
        tx.commit()?;
        Ok(removed > 0)
    }
    /// A repair that got its custody key runs to completion, so the provisional
    /// deferral it wrote must go. Unlike `revive_deferred_shared_index`, this
    /// does not mark the index dirty: the repair itself just ran.
    pub fn cancel_shared_index_deferral(&self, folder_id: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "DELETE FROM sync_state WHERE key=?1",
            [format!("shared_folder_index_deferred:{folder_id}")],
        )?;
        Ok(())
    }
    pub fn list_owned_key_records(&self) -> Result<Vec<crate::owner_keys::OwnedKeyRecord>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare("SELECT record FROM owned_key_records")?;
        let records = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>>>()?;
        records
            .into_iter()
            .map(|s| {
                serde_json::from_str(&s)
                    .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))
            })
            .collect()
    }
    /// Generation of this account-bound DB handle. The runner also stops its
    /// old bridge before reset, so old requests cannot join a fresh engine.
    pub fn account_revision(&self) -> u64 {
        let _conn = self.0.lock().expect("state_db mutex poisoned");
        self.1.load(std::sync::atomic::Ordering::Acquire)
    }
    pub fn get_owned_key_record(&self, id: &str) -> Result<Option<crate::owner_keys::OwnedKeyRecord>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let record: Option<String> = conn
            .query_row("SELECT record FROM owned_key_records WHERE file_id = ?1", [id], |r| {
                r.get(0)
            })
            .optional()?;
        record
            .map(|s| {
                serde_json::from_str(&s)
                    .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))
            })
            .transpose()
    }
    #[cfg(test)]
    pub fn put_owned_key_record(&self, row: &crate::owner_keys::OwnedKeyRecord) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Self::put_owned_key_record_conn(&conn, row)
    }
    pub fn put_owned_key_record_at_revision(
        &self,
        row: &crate::owner_keys::OwnedKeyRecord,
        revision: u64,
    ) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        if self.1.load(std::sync::atomic::Ordering::Acquire) != revision {
            return Err(rusqlite::Error::InvalidQuery);
        }
        Self::put_owned_key_record_conn(&conn, row)
    }
    fn put_owned_key_record_conn(conn: &Connection, row: &crate::owner_keys::OwnedKeyRecord) -> Result<()> {
        let record = serde_json::to_string(row).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        conn.execute("INSERT INTO owned_key_records (file_id, record) VALUES (?1, ?2) ON CONFLICT(file_id) DO UPDATE SET record = excluded.record", params![row.file_id, record])?;
        Ok(())
    }
    #[cfg(test)]
    pub fn delete_owned_key_record(&self, id: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute("DELETE FROM owned_key_records WHERE file_id = ?1", [id])?;
        Ok(())
    }
    /// Task 2446: whether any key record is blocked or any stored row has no path: what a snapshot with null
    /// request fields left behind in a build up to 0.8.24. Decides whether the one healing snapshot is asked for.
    pub fn has_blocked_or_nameless_rows(&self) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM owned_key_records WHERE json_extract(record, '$.blocked_fingerprint') IS NOT NULL)
                 OR EXISTS(SELECT 1 FROM files WHERE path = '' AND status != 'trashing')",
            [],
            |row| row.get(0),
        )
    }
    /// Task 2446: whether this database's one healing snapshot succeeded (or was never needed).
    pub fn name_heal_done(&self) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Ok(conn
            .query_row("SELECT 1 FROM sync_state WHERE key = 'name_heal_2446'", [], |_| Ok(()))
            .optional()?
            .is_some())
    }
    /// Task 2446: record that the healing snapshot succeeded (or was never needed). The retry mark goes with it.
    pub fn mark_name_heal_done(&self) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute_batch(
            "BEGIN;
             INSERT INTO sync_state (key, value) VALUES ('name_heal_2446', '1') ON CONFLICT(key) DO NOTHING;
             DELETE FROM sync_state WHERE key = 'name_heal_2446_retry';
             COMMIT;",
        )
    }
    /// Task 2446: what the last healing snapshot that did not complete left behind, or `None` when none failed yet.
    /// A value this build cannot read counts as none, so the heal is asked for again at once.
    pub fn name_heal_retry(&self) -> Result<Option<NameHealRetry>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Self::name_heal_retry_conn(&conn)
    }
    fn name_heal_retry_conn(conn: &Connection) -> Result<Option<NameHealRetry>> {
        let stored: Option<String> = conn
            .query_row(
                "SELECT value FROM sync_state WHERE key = 'name_heal_2446_retry'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(stored.as_deref().and_then(NameHealRetry::parse))
    }
    /// Task 2446: a healing snapshot failed as a whole (`rows_left` is `None`: the count an earlier one left is
    /// kept), or was applied but left `rows_left` rows as they were. Counts it and records when the next one may be
    /// asked for (`NameHealRetry::delay_secs`), in the state database, so a restart keeps the wait.
    pub fn note_name_heal_failed(&self, now: i64, rows_left: Option<u32>) -> Result<NameHealRetry> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let before = Self::name_heal_retry_conn(&conn)?;
        let failures = before.map_or(0, |retry| retry.failures).saturating_add(1);
        let retry = NameHealRetry {
            failures,
            next_at: now.saturating_add(NameHealRetry::delay_secs(failures)),
            rows_left: rows_left.unwrap_or_else(|| before.map_or(0, |retry| retry.rows_left)),
        };
        conn.execute(
            "INSERT INTO sync_state (key, value) VALUES ('name_heal_2446_retry', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![retry.stored()],
        )?;
        Ok(retry)
    }
}

/// Task 2446: the wait between two healing snapshots that did not complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NameHealRetry {
    /// Healing snapshots in a row that failed or left rows as they were.
    pub failures: u32,
    /// The second from which the next one may be asked for.
    pub next_at: i64,
    /// Rows the last one left as they were (0 when it failed as a whole). While any is left the heal stays owed,
    /// whether or not a blocked record still exists.
    pub rows_left: u32,
}

impl NameHealRetry {
    /// The first wait.
    pub const FIRST_DELAY_SECS: i64 = 60;
    /// The longest wait.
    pub const MAX_DELAY_SECS: i64 = 60 * 60;

    /// One minute after the first failure, twice as long after each further one, at most an hour.
    pub fn delay_secs(failures: u32) -> i64 {
        let doublings = failures.saturating_sub(1).min(16);
        (Self::FIRST_DELAY_SECS << doublings).min(Self::MAX_DELAY_SECS)
    }

    /// Whether the next healing snapshot may be asked for at `now`. A mark further ahead than the longest wait was
    /// written by a clock that has since been set back: it is not believed.
    pub fn is_due(&self, now: i64) -> bool {
        now >= self.next_at || self.next_at > now.saturating_add(Self::MAX_DELAY_SECS)
    }

    fn stored(&self) -> String {
        format!("{}:{}:{}", self.failures, self.next_at, self.rows_left)
    }

    fn parse(stored: &str) -> Option<Self> {
        let mut parts = stored.split(':');
        let retry = Self {
            failures: parts.next()?.parse().ok()?,
            next_at: parts.next()?.parse().ok()?,
            rows_left: parts.next()?.parse().ok()?,
        };
        parts.next().is_none().then_some(retry)
    }
}

#[cfg(test)]
mod name_heal_retry_tests {
    use super::NameHealRetry;

    /// The wait: one minute, doubling, at most an hour; a mark a clock that ran ahead wrote is not believed.
    #[test]
    fn the_wait_doubles_from_a_minute_to_an_hour() {
        let waits: Vec<i64> = (1..=9).map(NameHealRetry::delay_secs).collect();
        assert_eq!(waits, vec![60, 120, 240, 480, 960, 1920, 3600, 3600, 3600]);
        assert_eq!(NameHealRetry::delay_secs(0), 60);
        assert_eq!(NameHealRetry::delay_secs(u32::MAX), 3600, "no overflow at any count");

        let retry = NameHealRetry {
            failures: 1,
            next_at: 1060,
            rows_left: 0,
        };
        assert!(!retry.is_due(1059));
        assert!(retry.is_due(1060));
        assert!(
            !retry.is_due(1060 - 3600),
            "exactly the longest wait ahead is a real mark"
        );
        assert!(
            retry.is_due(1060 - 3601),
            "further ahead than the longest wait: the clock was set back"
        );
        assert_eq!(NameHealRetry::parse(&retry.stored()), Some(retry));
        for unreadable in ["", "1:2", "1:2:3:4", "a:2:3", "1:2:-3"] {
            assert_eq!(
                NameHealRetry::parse(unreadable),
                None,
                "{unreadable:?} counts as no mark"
            );
        }
    }
}
