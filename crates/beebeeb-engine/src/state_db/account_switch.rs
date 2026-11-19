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
