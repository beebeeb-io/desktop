//! An account switch's guard (w26-account-switch): what of the leaving account's still needs a person's answer.

use super::*;

impl StateDb {
    /// The first kind of the previous account's work a person has not answered for, as a fixed word (never a name):
    /// a queued change, a staged copy, an upload to finish (held for review or not), a held edit, a file of unknown
    /// content (sign-out's D11 rule: never deleted unasked), or a file in a state that is not at rest. Any of it keeps
    /// another account out of this data with nothing deleted; the account that owns it signs in and settles it.
    pub fn work_waiting_for_a_person(&self) -> Result<Option<&'static str>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let kinds: [(&str, String); 7] = [
            ("queued change", "SELECT COUNT(*) FROM operation_queue".into()),
            ("staged copy", "SELECT COUNT(*) FROM staged_payloads".into()),
            ("upload to finish", "SELECT COUNT(*) FROM upload_finalizations".into()),
            ("held edit", "SELECT COUNT(*) FROM held_edits".into()),
            ("file of unknown content", "SELECT COUNT(*) FROM unknown_orphans".into()),
            (
                "file not at rest",
                "SELECT COUNT(*) FROM files WHERE status IN ('uploading', 'conflict', 'error', 'trashing')".into(),
            ),
            ("guarded file", format!("SELECT COUNT(*) FROM files WHERE {GUARDED_SQL}")),
        ];
        for (word, sql) in kinds {
            if conn.query_row(&sql, [], |row| row.get::<_, i64>(0))? != 0 {
                return Ok(Some(word));
            }
        }
        Ok(None)
    }
}
