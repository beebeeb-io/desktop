//! dc-136b: a row whose newest server name could not be decrypted (an old client, a wrong key, a damaged blob).
//! The row keeps its last good path, so the file on disk keeps its name; this mark is the honest state the
//! window shows ("name could not be read") until a later listing carries a name that decrypts.

use super::*;

impl StateDb {
    /// Mark or clear `file_id`'s unreadable name. Returns whether the stored mark changed (an unknown id, or a
    /// mark that already says so, changes nothing).
    pub fn set_name_unreadable(&self, file_id: &str, unreadable: bool) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Ok(conn.execute(
            "UPDATE files SET name_unreadable = ?2 WHERE file_id = ?1 AND COALESCE(name_unreadable, 0) IS NOT ?2",
            params![file_id, i64::from(unreadable)],
        )? > 0)
    }

    /// Whether `file_id`'s newest server name could not be read. `false` for an unknown id.
    pub fn name_unreadable(&self, file_id: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let flag: Option<Option<i64>> = conn
            .query_row(
                "SELECT name_unreadable FROM files WHERE file_id = ?1",
                params![file_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(flag.flatten().is_some_and(|flag| flag != 0))
    }
}
