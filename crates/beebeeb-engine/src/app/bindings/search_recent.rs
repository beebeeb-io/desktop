//! Slice A8 (plan40 batch 1): the recent searches of Search (DW-0801): kept, read and cleared on this PC, per account.
//!
//! Session-bound like its neighbours (`*_for(session)`): every call is asked for the session the host read at its
//! press, and its answer is handed over only while that session is still the one in memory.
//!
//! Where they are: the account's own row of the local state (`state_db::search_recent`), keyed by the session's user
//! id (never an email), reached only through the account-bound handle (task 2488): a local state that is not the
//! session's account's is neither read nor written, and the refusal is a fixed word ([`LocalStateRefused`]). On a
//! computer where nothing has synced yet no database is made for them ([`LocalStateRefused::NothingSynced`]): nothing
//! is kept, and the host shows none.
//!
//! When they go (lead ruling 2026-11-15 17:25, privacy): DELETED at sign-out, not only dropped from memory:
//! [`Engine::sign_out_for`], the sign-out that keeps the local data and the close of a session the server ended
//! (a 401 a second question confirmed) call [`Engine::delete_recent_searches_at_sign_out`] while the session is still
//! in memory. A screen lock and a vault lock keep them (the same person unlocks). The purge of an account's rows
//! (`StateDb::clear_account_data`) wipes them as well; Clear deletes the row. Not deleted: after a crash, and when the
//! session ended by the refusal run (the session watcher, or a sign-out pressed after it), because the account-bound
//! handle then refuses as `SessionEnded`; the row stays until that account's next sign-out, Clear or purge, and
//! another account never reads it (the row is keyed by user id).
//!
//! BLOCKS on the state database: a worker's call. Must not: log a search, a user id, an email or a path.

use crate::account::SessionGeneration;
use crate::app::{CredentialStores, Engine, LocalStateRefused, OwnedState};

#[cfg(test)]
#[path = "search_recent_tests.rs"]
mod tests;

/// At most this many recent searches are kept per account (DI-0801: "Up to 10 recent searches"). The window's
/// `frames::search::source::RECENT_KEPT` is the same number (a test of the native crate holds them together).
pub const SEARCHES_KEPT: usize = 10;

impl<C: CredentialStores> Engine<C> {
    /// The recent searches of the account of `session`, newest first. `Err`: why none can be read, in a fixed word;
    /// `session` moved on, or moves on while this reads: [`LocalStateRefused::SessionChanged`].
    pub fn recent_searches_for(&self, session: SessionGeneration) -> Result<Vec<String>, LocalStateRefused> {
        let state = self.local_state()?;
        let user_id = self.recent_searches_key(&state, session)?;
        state
            .with(|db| db.search_recent(&user_id))?
            .map_err(|_| LocalStateRefused::Unreadable)
    }

    /// WRITES: `query` is the newest recent search of the account of `session` (the same words in another case are
    /// moved, not kept twice; at most [`SEARCHES_KEPT`]). Answers the list as it is now stored. Only into a state
    /// proven that account's, and only while `session` is the session in memory; `Err`: nothing was written.
    pub fn remember_search_for(
        &self,
        session: SessionGeneration,
        query: &str,
    ) -> Result<Vec<String>, LocalStateRefused> {
        let state = self.local_state_for_changes()?;
        let user_id = self.recent_searches_key(&state, session)?;
        state
            .with(|db| db.remember_search(&user_id, query, SEARCHES_KEPT))?
            .map_err(|_| LocalStateRefused::Unreadable)
    }

    /// WRITES (Clear): the recent searches of the account of `session` are deleted from this computer. The same
    /// conditions as [`Self::remember_search_for`]; `Err`: nothing was deleted.
    pub fn clear_recent_searches_for(&self, session: SessionGeneration) -> Result<(), LocalStateRefused> {
        let state = self.local_state_for_changes()?;
        let user_id = self.recent_searches_key(&state, session)?;
        state
            .with(|db| db.clear_search_recent(&user_id))?
            .map_err(|_| LocalStateRefused::Unreadable)
    }

    /// WRITES, a step of the sign-out (lead ruling 2026-11-15 17:25): the recent searches of the account of `session`
    /// are deleted from this computer. [`Engine::sign_out_for`] calls it after its session check, before the session
    /// leaves memory. A locked screen does not refuse it (the sign-out's own work, not a person's read: a search is
    /// never left on disk because the screen was locked). `Err` (nothing synced, a state that is not this account's:
    /// nothing of it was ever written there) never stops the sign-out.
    pub(crate) fn delete_recent_searches_at_sign_out(
        &self,
        session: SessionGeneration,
    ) -> Result<(), LocalStateRefused> {
        let state = self.local_state_for_work()?;
        let user_id = self.recent_searches_key(&state, session)?;
        state
            .with(|db| db.clear_search_recent(&user_id))?
            .map_err(|_| LocalStateRefused::Unreadable)
    }

    /// The row's key: the user id of the session the handle `state` was made for, when that is `session`. An email
    /// never stands in for it.
    fn recent_searches_key(
        &self,
        state: &OwnedState<'_>,
        session: SessionGeneration,
    ) -> Result<String, LocalStateRefused> {
        // The handle's session is the one the host named, not a later one that opened in between.
        if state.session() != session {
            return Err(LocalStateRefused::SessionChanged);
        }
        let user_id = self
            .session_identity()?
            .user_id
            .ok_or(LocalStateRefused::IdentityUnknown)?;
        // Read after the handle was made: the handle's own question proves it is still that session's.
        state.still_this_session()?;
        Ok(user_id)
    }
}
