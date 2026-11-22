//! An account switch's guard (w26-account-switch): what of the leaving account's still needs a person's answer.

use super::*;

impl StateDb {
    /// The first kind of the previous account's work a person has not answered for, as a fixed word (never a name):
    /// a queued change, a staged copy, an upload to finish (held for review or not), a held edit, a placeholder left
    /// at another path with unknown bytes (sign-out's D11 rule: never deleted unasked), or a file in a state that is
    /// not at rest. Any of it keeps another account out of this data with nothing deleted; the account that owns it
    /// signs in and settles it. A file row of unknown content is not decided here: see [`Self::guarded_paths`].
    pub fn work_waiting_for_a_person(&self) -> Result<Option<&'static str>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let kinds: [(&str, String); 6] = [
            ("queued change", "SELECT COUNT(*) FROM operation_queue".into()),
            ("staged copy", "SELECT COUNT(*) FROM staged_payloads".into()),
            ("upload to finish", "SELECT COUNT(*) FROM upload_finalizations".into()),
            ("held edit", "SELECT COUNT(*) FROM held_edits".into()),
            ("file of unknown content", "SELECT COUNT(*) FROM unknown_orphans".into()),
            (
                "file not at rest",
                "SELECT COUNT(*) FROM files WHERE status IN ('uploading', 'conflict', 'error', 'trashing')".into(),
            ),
        ];
        for (word, sql) in kinds {
            if conn.query_row(&sql, [], |row| row.get::<_, i64>(0))? != 0 {
                return Ok(Some(word));
            }
        }
        Ok(None)
    }

    /// RC2-5: the waiting work of [`Self::work_waiting_for_a_person`] whose every byte is an item in the sync folder,
    /// by the path of that item: a new file or an edit queued while sync was paused, a new
    /// folder queued, an edit held for review (a rebuilt state.db holds every file of unknown content so), and the
    /// staged copies of those queued changes. A person's answer about those items (keep them aside, or discard them)
    /// settles all of it. `None` when any other work waits (another kind of change, an upload the server may already
    /// hold, a file of unknown content at another path, a conflict, an error, a trash): that needs its account.
    pub fn work_in_the_folder(&self) -> Result<Option<Vec<String>>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let count = |sql: &str| conn.query_row(sql, [], |row| row.get::<_, i64>(0));
        let other_work = [
            "SELECT COUNT(*) FROM operation_queue WHERE kind NOT IN ('upload_file', 'upload_version', 'create_folder')",
            "SELECT COUNT(*) FROM operation_queue o WHERE NOT EXISTS (SELECT 1 FROM files f WHERE f.file_id = o.file_id)",
            "SELECT COUNT(*) FROM staged_payloads s
               WHERE NOT EXISTS (SELECT 1 FROM operation_queue o WHERE o.payload_path = s.path)",
            "SELECT COUNT(*) FROM upload_finalizations",
            "SELECT COUNT(*) FROM unknown_orphans",
            "SELECT COUNT(*) FROM files WHERE status IN ('conflict', 'error', 'trashing')",
            "SELECT COUNT(*) FROM files f WHERE f.status = 'uploading'
               AND NOT EXISTS (SELECT 1 FROM operation_queue o WHERE o.file_id = f.file_id)
               AND NOT EXISTS (SELECT 1 FROM held_edits h WHERE h.file_id = f.file_id)",
        ];
        for sql in other_work {
            if count(sql)? != 0 {
                return Ok(None);
            }
        }
        let mut items = conn.prepare(
            "SELECT f.path FROM operation_queue o JOIN files f ON f.file_id = o.file_id
             UNION SELECT f.path FROM held_edits h JOIN files f ON f.file_id = h.file_id
             ORDER BY 1",
        )?;
        let items = items.query_map([], |row| row.get::<_, String>(0))?;
        items.collect::<Result<Vec<_>>>().map(Some)
    }

    /// Where each file row of unknown content is, as the database writes paths (w27). Such a row alone is no work of
    /// a person's: a document made in the app is one, with nothing of it in the folder. It keeps another account out
    /// only while the item there may hold bytes on this PC, which the OS answers (`SyncRootPort::may_hold_bytes`).
    pub fn guarded_paths(&self) -> Result<Vec<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut rows = conn.prepare(&format!("SELECT path FROM files WHERE {GUARDED_SQL} ORDER BY rowid"))?;
        let paths = rows.query_map([], |row| row.get::<_, String>(0))?;
        paths.collect()
    }
}
