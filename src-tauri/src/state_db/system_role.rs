//! LD-0113 (spec §5.4, §6.3): the server's `system_role` of a row, kept in `files.system_role`.
//!
//! The Backups root is the row whose role is `backups_root`. It is never found by its path or its
//! name: a folder called "Backups" without the role is an ordinary folder. The protection rule that
//! reads it is `crate::backup_protect`.

use super::*;
use beebeeb_types::backup::SystemRole;

impl StateDb {
    /// Store `role` for `file_id`. Only a change is written, and a change records a `Modified` entry
    /// in the File Provider change log, so Finder reads the item again and drops (or regains) its
    /// delete, rename and move capabilities. Returns whether the stored role changed; an unknown
    /// `file_id` changes nothing.
    pub fn set_system_role(&self, file_id: &str, role: Option<SystemRole>) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let changed = conn.execute(
            "UPDATE files SET system_role = ?2 WHERE file_id = ?1 AND system_role IS NOT ?2",
            params![file_id, role.map(SystemRole::as_str)],
        )? > 0;
        if changed {
            record_file_change_conn(&conn, file_id, FpChangeKind::Modified, None)?;
        }
        Ok(changed)
    }

    /// The stored role of `file_id`; `None` for an ordinary row, an unknown id, or a role string this
    /// build does not know (it is never treated as a role it is not).
    pub fn system_role(&self, file_id: &str) -> Result<Option<SystemRole>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let raw: Option<String> = conn
            .query_row("SELECT system_role FROM files WHERE file_id = ?1", params![file_id], |r| r.get(0))
            .optional()?
            .flatten();
        Ok(raw.as_deref().and_then(SystemRole::parse))
    }

    /// Every row the server flagged as a Backups root, as `(file_id, path)`, ordered by id. The server
    /// keeps one per account (a unique index); a list keeps this answer honest if a database ever holds
    /// two (each is protected). Served by `idx_files_system_role`, so a Cloud Files callback that asks
    /// on every delete reads a handful of index entries, never the whole table.
    pub fn backups_roots(&self) -> Result<Vec<(String, String)>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare_cached(
            "SELECT file_id, path FROM files WHERE system_role = 'backups_root' ORDER BY file_id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }
}
