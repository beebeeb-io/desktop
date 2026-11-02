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
        let conn=self.0.lock().expect("state_db mutex poisoned");
        conn.execute("INSERT INTO sync_state (key,value) VALUES (?1,'1') ON CONFLICT(key) DO UPDATE SET value=CAST(CAST(value AS INTEGER)+1 AS TEXT)", [format!("shared_folder_index_dirty:{folder_id}")])?;
        Ok(())
    }
    pub fn dirty_shared_index_roots(&self) -> Result<Vec<String>> {
        let conn=self.0.lock().expect("state_db mutex poisoned");
        let mut stmt=conn.prepare("SELECT substr(key,27) FROM sync_state WHERE key LIKE 'shared_folder_index_dirty:%'")?;
        stmt.query_map([],|r|r.get(0))?.collect()
    }
    pub fn clear_shared_index_dirty(&self, folder_id: &str) -> Result<()> {
        let conn=self.0.lock().expect("state_db mutex poisoned");
        conn.execute("DELETE FROM sync_state WHERE key=?1", [format!("shared_folder_index_dirty:{folder_id}")])?;
        Ok(())
    }
    pub fn shared_index_dirty_revision(&self,id:&str)->Result<Option<String>> {
        let conn=self.0.lock().expect("state_db mutex poisoned");
        conn.query_row("SELECT value FROM sync_state WHERE key=?1",[format!("shared_folder_index_dirty:{id}")],|r|r.get(0)).optional()
    }
    pub fn clear_shared_index_dirty_at_revision(&self,id:&str,revision:Option<&str>)->Result<()> {
        let conn=self.0.lock().expect("state_db mutex poisoned");
        conn.execute("DELETE FROM sync_state WHERE key=?1 AND value=?2",params![format!("shared_folder_index_dirty:{id}"),revision])?;
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
    pub fn list_owned_key_records(&self)->Result<Vec<crate::owner_keys::OwnedKeyRecord>> {
        let conn=self.0.lock().expect("state_db mutex poisoned");
        let mut stmt=conn.prepare("SELECT record FROM owned_key_records")?;
        let records=stmt.query_map([],|r|r.get::<_,String>(0))?.collect::<Result<Vec<_>>>()?;
        records.into_iter().map(|s|serde_json::from_str(&s).map_err(|e|rusqlite::Error::FromSqlConversionFailure(0,rusqlite::types::Type::Text,Box::new(e)))).collect()
    }
    /// Generation of this account-bound DB handle. The runner also stops its
    /// old bridge before reset, so old requests cannot join a fresh engine.
    pub fn account_revision(&self) -> u64 {
        let _conn = self.0.lock().expect("state_db mutex poisoned");
        self.1.load(std::sync::atomic::Ordering::Acquire)
    }
    pub fn get_owned_key_record(&self, id: &str) -> Result<Option<crate::owner_keys::OwnedKeyRecord>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let record: Option<String> = conn.query_row("SELECT record FROM owned_key_records WHERE file_id = ?1", [id], |r|r.get(0)).optional()?;
        record.map(|s| serde_json::from_str(&s).map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))).transpose()
    }
    pub fn put_owned_key_record(&self, row: &crate::owner_keys::OwnedKeyRecord) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Self::put_owned_key_record_conn(&conn, row)
    }
    pub fn put_owned_key_record_at_revision(&self, row: &crate::owner_keys::OwnedKeyRecord, revision: u64) -> Result<()> {
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
    pub fn delete_owned_key_record(&self, id: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute("DELETE FROM owned_key_records WHERE file_id = ?1", [id])?;
        Ok(())
    }
}
