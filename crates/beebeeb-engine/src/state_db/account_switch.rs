//! An account switch's guard (w26-account-switch): what of the leaving account's still needs a person's answer.

use super::*;

impl StateDb {
    /// Whether anything here is the previous account's work a person has not answered for: a queued change, a staged
    /// copy, an upload to finish (held for review or not), a file in a state that is not at rest, a held edit, or a file
    /// of unknown content (sign-out's D11 rule: never deleted unasked). Any of it keeps another account out of this
    /// data with nothing deleted; the account that owns it signs in and settles it.
    pub fn holds_work_for_a_person(&self) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let count: i64 = conn.query_row(
            &format!(
                "SELECT (SELECT COUNT(*) FROM operation_queue)
                      + (SELECT COUNT(*) FROM staged_payloads)
                      + (SELECT COUNT(*) FROM upload_finalizations)
                      + (SELECT COUNT(*) FROM held_edits)
                      + (SELECT COUNT(*) FROM unknown_orphans)
                      + (SELECT COUNT(*) FROM files WHERE status IN ('uploading', 'conflict', 'error', 'trashing'))
                      + (SELECT COUNT(*) FROM files WHERE {GUARDED_SQL})"
            ),
            [],
            |row| row.get(0),
        )?;
        Ok(count != 0)
    }
}
