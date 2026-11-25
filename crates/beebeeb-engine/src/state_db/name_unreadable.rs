//! dc-136b: a row whose newest server name could not be decrypted (an old client, a wrong key, a damaged blob).
//! The row keeps its last good path, so the file on disk keeps its name; this mark is the honest state the
//! window shows ("name could not be read") until a later listing carries a name that decrypts.
//!
//! dc-136c: a mark is not final. When the keys this PC holds change (the vault is unlocked, the account's key is
//! rotated, the key of an accepted share or an owner record arrives), [`StateDb::retry_unreadable_names`] asks for
//! the server's list again; the sync applies every marked row from that list, whatever its times say, and a name
//! that now opens replaces the kept one and clears the mark.

use super::*;

impl StateDb {
    /// Mark or clear `file_id`'s unreadable name. Returns whether the stored mark changed (an unknown id, or a
    /// mark that already says so, changes nothing).
    pub fn set_name_unreadable(&self, file_id: &str, unreadable: bool) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Ok(conn
            .prepare_cached(
                "UPDATE files SET name_unreadable = ?2 WHERE file_id = ?1 AND COALESCE(name_unreadable, 0) IS NOT ?2",
            )?
            .execute(params![file_id, i64::from(unreadable)])?
            > 0)
    }

    /// Whether `file_id`'s newest server name could not be read. `false` for an unknown id.
    pub fn name_unreadable(&self, file_id: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let flag: Option<Option<i64>> = conn
            .prepare_cached("SELECT name_unreadable FROM files WHERE file_id = ?1")?
            .query_row(params![file_id], |r| r.get(0))
            .optional()?;
        Ok(flag.flatten().is_some_and(|flag| flag != 0))
    }

    /// How many rows carry the mark.
    pub fn unreadable_name_count(&self) -> Result<usize> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Self::unreadable_name_count_conn(&conn)
    }

    fn unreadable_name_count_conn(conn: &Connection) -> Result<usize> {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM files WHERE COALESCE(name_unreadable, 0) <> 0",
            [],
            |r| r.get(0),
        )?;
        Ok(usize::try_from(count).unwrap_or(0))
    }

    /// dc-136c: the keys this PC holds changed, so every name that could not be read is tried again: when at least
    /// one row is marked, the server's list is asked for again (the count and the request in one lock). Returns
    /// how many rows wait for it; with none, nothing is asked. The marks stay until a name really opens, so a
    /// call that changes nothing costs one list and is no loop: only a key change calls this.
    #[track_caller]
    pub fn retry_unreadable_names(&self) -> Result<usize> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let marked = Self::unreadable_name_count_conn(&conn)?;
        if marked > 0 {
            Self::request_resnapshot_conn(&conn)?;
        }
        Ok(marked)
    }
}
