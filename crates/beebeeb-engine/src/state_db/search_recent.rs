//! The recent searches of Search (DW-0801): one `sync_state` row per account, `search_recent:{user id}`, its value the
//! searches as a JSON list, newest first.
//!
//! Deleted at sign-out: every session end deletes the row of its account
//! ([`StateDb::clear_search_recent`]), and an engine start with nobody signed in deletes every account's
//! ([`StateDb::clear_every_search_recent`], lead ruling 23:03). The purge of an account's rows (`clear_account_data`) wipes the
//! rows of every account too: `sign_out_sync_state_purge` keeps only the rows it names, and these are not among them.
//! Unlike the guide mark they count as account data (`sql::account_rows`): they are only ever written into a state
//! proven the account's own, so they never make a state nobody owns read as somebody's.
//!
//! The key holds the user id and nothing else: never an email. Who may read or write which row is the engine handle's
//! (`app::bindings::search_recent`): only the session's own. A search is the person's own words: never logged, never
//! in an error.

use super::*;
use rusqlite::TransactionBehavior;

/// Every row of recent searches starts with this (and only these rows do).
const PREFIX: &str = "search_recent:";

fn key(user_id: &str) -> String {
    format!("{PREFIX}{user_id}")
}

/// A `GLOB` pattern for every row of recent searches (in `GLOB`, unlike `LIKE`, the `_` of the prefix is itself).
fn every_key() -> String {
    format!("{PREFIX}*")
}

/// A stored value as the list it holds. A value that does not read as a list of words is no list: nothing is shown,
/// and the next change replaces it.
fn list_of(value: Option<String>) -> Vec<String> {
    value
        .and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
        .unwrap_or_default()
}

/// `recent` with `query` as its newest: once (the same words in another case are the same search), at most `kept`.
fn with_newest(mut recent: Vec<String>, query: &str, kept: usize) -> Vec<String> {
    let same = query.to_lowercase();
    recent.retain(|earlier| earlier.to_lowercase() != same);
    recent.insert(0, query.to_string());
    recent.truncate(kept);
    recent
}

impl StateDb {
    /// The recent searches of the account `user_id`, newest first. None kept: empty.
    pub fn search_recent(&self, user_id: &str) -> Result<Vec<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let value: Option<String> = conn
            .query_row(
                "SELECT value FROM sync_state WHERE key = ?1",
                params![key(user_id)],
                |row| row.get(0),
            )
            .optional()?;
        Ok(list_of(value))
    }

    /// WRITES: `query` (trimmed) is the newest recent search of the account `user_id`; the same words in another
    /// case are moved, not kept twice; at most `kept` stay. Read and written in ONE immediate transaction, so two
    /// changes at once never lose one. Answers the list as it is now stored. An empty query changes nothing.
    pub fn remember_search(&self, user_id: &str, query: &str, kept: usize) -> Result<Vec<String>> {
        let query = query.trim();
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let value: Option<String> = tx
            .query_row(
                "SELECT value FROM sync_state WHERE key = ?1",
                params![key(user_id)],
                |row| row.get(0),
            )
            .optional()?;
        let recent = list_of(value);
        if query.is_empty() || kept == 0 {
            return Ok(recent);
        }
        let recent = with_newest(recent, query, kept);
        let value =
            serde_json::to_string(&recent).map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
        tx.execute(
            "INSERT INTO sync_state (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key(user_id), value],
        )?;
        tx.commit()?;
        Ok(recent)
    }

    /// WRITES: the recent searches of the account `user_id` are gone: its row is DELETED, not emptied. Idempotent.
    pub fn clear_search_recent(&self, user_id: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute("DELETE FROM sync_state WHERE key = ?1", params![key(user_id)])?;
        Ok(())
    }

    /// Whether any account has a row of recent searches here. Reads only: asked on a read-only connection.
    pub fn has_any_search_recent(&self) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let rows: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sync_state WHERE key GLOB ?1",
            params![every_key()],
            |row| row.get(0),
        )?;
        Ok(rows > 0)
    }

    /// WRITES: the recent searches of EVERY account are gone (the engine's start with nobody signed in, lead ruling
    /// 2026-11-15 23:03: `app::owned_state`). Answers how many rows were deleted. Idempotent.
    pub fn clear_every_search_recent(&self) -> Result<usize> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute("DELETE FROM sync_state WHERE key GLOB ?1", params![every_key()])
    }

    /// Whether the account `user_id` has a row of recent searches on disk at all (a test asks: deleted, not emptied).
    #[cfg(any(test, feature = "test-support"))]
    pub fn has_search_recent_row(&self, user_id: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let rows: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sync_state WHERE key = ?1",
            params![key(user_id)],
            |row| row.get(0),
        )?;
        Ok(rows > 0)
    }
}
