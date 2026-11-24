//! dc-168: the server's `has_thumbnail` of a row, kept in `files.has_thumbnail` with the `current_version` it was
//! stored at (`files.has_thumbnail_version`). The photo previews' upload (115) reads it to leave a file the server
//! already holds a thumbnail for, without fetching that thumbnail first.

use super::*;

impl StateDb {
    /// Store the server's `has_thumbnail` for `file_id`, at the row's `current_version` as it stands now (so call
    /// it after the row's version was written). Returns whether a row was there; an unknown id changes nothing.
    pub fn set_server_has_thumbnail(&self, file_id: &str, has: bool) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Ok(conn.execute(
            "UPDATE files SET has_thumbnail = ?2, has_thumbnail_version = current_version
             WHERE file_id = ?1 AND (has_thumbnail IS NOT ?2 OR has_thumbnail_version IS NOT current_version)",
            params![file_id, i64::from(has)],
        )? > 0
            || conn
                .query_row("SELECT 1 FROM files WHERE file_id = ?1", params![file_id], |_| Ok(()))
                .optional()?
                .is_some())
    }

    /// What the server last said of `file_id`'s thumbnail, only when it was said at `version` and that is still the
    /// row's current version. `None`: not known (no flag stored, an older database, another version, an unknown id).
    pub fn server_has_thumbnail(&self, file_id: &str, version: i64) -> Result<Option<bool>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let flag: Option<Option<i64>> = conn
            .query_row(
                "SELECT has_thumbnail FROM files
                 WHERE file_id = ?1 AND has_thumbnail_version = ?2 AND current_version = ?2",
                params![file_id, version],
                |r| r.get(0),
            )
            .optional()?;
        Ok(flag.flatten().map(|flag| flag != 0))
    }
}
