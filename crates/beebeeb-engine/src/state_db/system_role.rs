//! LD-0113 (spec §5.4, §6.3): the server's `system_role` of a row, kept in `files.system_role`.
//!
//! The Backups root is the row whose role is `backups_root`. It is never found by its path or its
//! name: a folder called "Backups" without the role is an ordinary folder. The protection rule that
//! reads it is `crate::backup_protect`; the after-the-fact restore of its tree reads
//! [`StateDb::local_rows_at_or_under`].

use super::*;
use beebeeb_types::backup::SystemRole;

impl StateDb {
    /// Store `role` for `file_id`. Only a change is written, and a change records a `Modified` entry
    /// in the File Provider change log for the row and for every row under it, so Finder reads them
    /// again: the whole tree drops (or regains) its delete, rename, move and add capabilities with
    /// the root's role (Amendment 6). A Backups folder that existed before the server flagged it (an
    /// iOS backup) would otherwise keep offering Delete on what is in it. One transaction. Returns
    /// whether the stored role changed; an unknown `file_id` changes nothing.
    pub fn set_system_role(&self, file_id: &str, role: Option<SystemRole>) -> Result<bool> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        // Every listing row that carries the field lands here: read first, so an unchanged role (or an
        // unknown id) opens no write transaction.
        let stored: Option<Option<String>> = conn
            .query_row(
                "SELECT system_role FROM files WHERE file_id = ?1",
                params![file_id],
                |r| r.get(0),
            )
            .optional()?;
        if stored.is_none_or(|stored| stored.as_deref() == role.map(SystemRole::as_str)) {
            return Ok(false);
        }
        let tx = conn.transaction()?;
        let changed = tx.execute(
            "UPDATE files SET system_role = ?2 WHERE file_id = ?1 AND system_role IS NOT ?2",
            params![file_id, role.map(SystemRole::as_str)],
        )? > 0;
        if changed {
            record_file_change_conn(&tx, file_id, FpChangeKind::Modified, None)?;
            let path: String = tx.query_row(
                "SELECT trim(path, '/') FROM files WHERE file_id = ?1",
                params![file_id],
                |r| r.get(0),
            )?;
            if !path.is_empty() {
                let under: Vec<String> = tx
                    // A child may be stored with a leading slash (`/Backups/x`), as `untrash_subtree` allows.
                    .prepare("SELECT file_id FROM files WHERE substr(ltrim(path, '/'), 1, length(?1) + 1) = ?1 || '/'")?
                    .query_map(params![path], |r| r.get(0))?
                    .collect::<Result<_>>()?;
                for id in under {
                    record_file_change_conn(&tx, &id, FpChangeKind::Modified, None)?;
                }
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    /// The stored role of `file_id`; `None` for an ordinary row, an unknown id, or a role string this
    /// build does not know (it is never treated as a role it is not). The rule reads the roots through
    /// [`Self::backups_roots`]; this read is the tests'.
    #[cfg(any(test, feature = "test-support"))]
    pub fn system_role(&self, file_id: &str) -> Result<Option<SystemRole>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let raw: Option<String> = conn
            .query_row(
                "SELECT system_role FROM files WHERE file_id = ?1",
                params![file_id],
                |r| r.get(0),
            )
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
        let mut stmt =
            conn.prepare_cached("SELECT file_id, path FROM files WHERE system_role = 'backups_root' ORDER BY file_id")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }

    /// The rows at or under `path` (the `/`-joined path, `Backups/OtherPC`) whose bytes are on this
    /// device (`local`), as `(file_id, path)`, the item itself first. A protected item a local change
    /// took away is put back from these (`crate::watcher`). Live rows leave `parent_id` empty, so the
    /// subtree is a path range: every path that starts with `path/` sorts between `path/` and `path0`
    /// (`'0'` follows `'/'`), which `idx_files_path` serves without reading the whole table, and no
    /// metacharacter in a name can widen. Any row may be stored with a leading slash (`/Backups/x`, as
    /// `untrash_subtree` allows), so both forms are ranged. Names
    /// compare exactly: Windows names each item back as its placeholder was minted, from these rows.
    pub fn local_rows_at_or_under(&self, path: &str) -> Result<Vec<(String, String)>> {
        let path = path.trim_matches('/');
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT file_id, path FROM files
             WHERE status = 'local'
               AND (path = ?1 OR path = '/' || ?1
                    OR (path >= ?1 || '/' AND path < ?1 || '0')
                    OR (path >= '/' || ?1 || '/' AND path < '/' || ?1 || '0'))
             ORDER BY length(path), path",
        )?;
        let rows = stmt.query_map(params![path], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }

    /// The path of every row at or under `path`, whatever its status, the item itself first: the
    /// range of [`Self::local_rows_at_or_under`] without its status. The drive's read-only state of a
    /// Backups tree goes on these (LD-0113 Amendment 7, Windows).
    #[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
    pub fn paths_at_or_under(&self, path: &str) -> Result<Vec<String>> {
        let path = path.trim_matches('/');
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT path FROM files
             WHERE path = ?1 OR path = '/' || ?1
                OR (path >= ?1 || '/' AND path < ?1 || '0')
                OR (path >= '/' || ?1 || '/' AND path < '/' || ?1 || '0')
             ORDER BY length(path), path",
        )?;
        let rows = stmt.query_map(params![path], |r| r.get(0))?;
        rows.collect()
    }
}
