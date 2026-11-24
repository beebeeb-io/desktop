//! The first-time guide's "done" mark: one `sync_state` row per account, `guide_done:{user id}`.
//!
//! Owner ruling 2026-11-13: the guide is done once per ACCOUNT on each computer. An account that finished or
//! skipped it does not get it again here, also after a sign-out and a sign-in; another account on the same
//! computer gets it once. So the row is keyed by the account's own id, a sign-out keeps it (as it keeps the share
//! rows, `share_marks::SHARE_TRUST_ROWS`), it is not counted as account data (a kept row must not make a signed-out
//! database read as somebody's). `StateDb::forget_share_trust_rows` removes the marks of every account with the
//! share rows: the 0.8 app calls it when it resets this computer's local data for another account. The engine
//! handle of the native app has no reset of its own, so there the marks stay until that reset exists.
//!
//! The key holds the user id and nothing else: never an email. Who may read which row is the engine handle's
//! (`app::owned_state::guide_mark`): only the session's own.

use super::*;

/// The rows of this file, as a `LIKE` pattern.
pub(super) const GUIDE_DONE_ROWS: &str = "guide_done:%";

/// ` AND key NOT LIKE 'guide_done:%'`: appended to a `sync_state` statement that leaves the marks alone.
pub(super) fn keep_guide_rows_sql() -> String {
    format!(" AND key NOT LIKE '{GUIDE_DONE_ROWS}'")
}

fn key(user_id: &str) -> String {
    format!("guide_done:{user_id}")
}

impl StateDb {
    /// Whether the account `user_id` finished or skipped the first-time guide on this computer.
    pub fn guide_done(&self, user_id: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let value: Option<String> = conn
            .query_row(
                "SELECT value FROM sync_state WHERE key = ?1",
                params![key(user_id)],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value.is_some())
    }

    /// The account `user_id` finished or skipped the first-time guide. Idempotent.
    pub fn set_guide_done(&self, user_id: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "INSERT INTO sync_state (key, value) VALUES (?1, '1') ON CONFLICT(key) DO UPDATE SET value = '1'",
            params![key(user_id)],
        )?;
        Ok(())
    }
}
