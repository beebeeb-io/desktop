//! The first-time guide's "done" mark, as a shell reaches it: for the session's own account and no other.
//!
//! Owner ruling 2026-11-13: the guide is done once per ACCOUNT on each computer (an account that finished or
//! skipped it does not get it again here, also after a sign-out and a sign-in; another account on the same computer
//! gets it once). The mark is a row of the account's local state keyed by the account's own user id
//! (`state_db::guide_mark`); a sign-out keeps it. The owner proof below is `Engine::owned_state`'s, with one case
//! more (a state that is nobody's): the two are kept side by side in this module's parent on purpose, and a change
//! to one is a change to the other.
//!
//! Neither function takes an account: whose mark is read or written is the session's, as the engine knows it. Both
//! take the session ([`Engine::session_turn`]) the host asked under, and answer nothing for another.
//!
//! The read does not go through [`Engine::local_state`], and that is deliberate. That handle answers "nothing is
//! read" for a state with no recorded owner, which is exactly what a sign-out that clears the account's rows
//! leaves behind, mark included: read through it, the guide would come back after every such sign-out. Here the
//! one row whose key is the session's own user id is read from a state that is the session's, or that is nobody's
//! (no owner and no account data). Nothing else of the state is read, and nothing of a state recorded as another
//! account's, or holding data with no owner.
//!
//! The write goes through [`Engine::local_state_for_changes`] unchanged: nothing is written into a state before it
//! is proven the session's, and no database is created for the mark. So on a computer where nothing has synced
//! yet the mark cannot be kept ([`LocalStateRefused::NothingSynced`]); the host holds it and asks again.
//!
//! BLOCKS on the state database: a worker's call. Must not: log a user id, an email or a path.

use super::{LocalStateRefused, Mode, PERSON};
use crate::account::SessionGeneration;
use crate::account_binding::{self, Binding, BindingKind, Identity};
use crate::app::{CredentialStores, Engine};
use crate::state_db::{ReadOnlyOpenError, StateDb};

/// What this computer knows about the session's account and the first-time guide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuideMark {
    /// The account finished or skipped the guide here.
    Done,
    /// It did not (also: nothing of any account is kept on this computer yet).
    NotDone,
    /// It cannot be said, in a fixed word: a host starts no guide on this.
    NotKnown(LocalStateRefused),
}

impl<C: CredentialStores> Engine<C> {
    /// Whether the account of `session` finished or skipped the first-time guide on this computer. See the module's
    /// doc for what is read. `session` moved on, or moves on while this reads: [`GuideMark::NotKnown`].
    pub fn guide_mark_for(&self, session: SessionGeneration) -> GuideMark {
        match self.read_guide_mark(session) {
            Ok(true) => GuideMark::Done,
            Ok(false) => GuideMark::NotDone,
            Err(refused) => GuideMark::NotKnown(refused),
        }
    }

    fn read_guide_mark(&self, session: SessionGeneration) -> Result<bool, LocalStateRefused> {
        // As the handle's own question (`OwnedState::still_this_session`): the session in memory is the one that
        // asked, with no session change half done, and the server has not ended it.
        let still = || {
            self.acct
                .while_session_is(&self.writes, session, |_| ())
                .ok_or(LocalStateRefused::SessionChanged)?;
            if self.acct.auth_health.is_expired() {
                return Err(LocalStateRefused::SessionEnded);
            }
            Ok(())
        };
        still()?;
        self.person_asks(PERSON)?;
        let identity = self.session_identity()?;
        // The row's key. An email never stands in for it: a row a sign-out keeps holds no address.
        let user_id = identity.user_id.clone().ok_or(LocalStateRefused::IdentityUnknown)?;
        let path = self.local_state_path(&identity);
        if !path.exists() {
            still()?;
            return Ok(false);
        }
        let state = match StateDb::open_read_only(&path) {
            Ok(state) => state,
            Err(ReadOnlyOpenError::NotFound) => {
                still()?;
                return Ok(false);
            }
            Err(_) => return Err(LocalStateRefused::Unreadable),
        };
        let owner = state
            .owner()
            .map_err(|_| LocalStateRefused::Unreadable)?
            .filter(Identity::is_known);
        match owner {
            Some(owner) => match account_binding::decide(Some(&owner), &identity, BindingKind::LocalData) {
                Binding::Proceed => {}
                Binding::Reset => return Err(LocalStateRefused::OtherAccount),
                Binding::Refuse => return Err(LocalStateRefused::IdentityUnknown),
            },
            // Nobody's: only when it holds no account data either (a state a sign-out cleared, or a new one).
            None => {
                if state.has_account_data().map_err(|_| LocalStateRefused::Unreadable)? {
                    return Err(LocalStateRefused::Unowned);
                }
            }
        }
        let done = state.guide_done(&user_id).map_err(|_| LocalStateRefused::Unreadable)?;
        still()?;
        Ok(done)
    }

    /// WRITES: the account of `session` finished or skipped the first-time guide. Only into a state proven that
    /// account's ([`Engine::local_state_for_changes`]), and only while `session` is the session in memory: a lock, a
    /// sign-out or another sign-in since the host read it writes nothing. `Err`: why not, in a fixed word; nothing
    /// was written.
    pub fn mark_guide_done_for(&self, session: SessionGeneration) -> Result<(), LocalStateRefused> {
        self.acct
            .while_session_is(&self.writes, session, |_| ())
            .ok_or(LocalStateRefused::SessionChanged)?;
        let user_id = self
            .session_identity()?
            .user_id
            .ok_or(LocalStateRefused::IdentityUnknown)?;
        let state = self.owned_state(Mode::Change, PERSON)?;
        // The handle's session is the one the host named, not a later one that opened in between.
        if state.generation != session {
            return Err(LocalStateRefused::SessionChanged);
        }
        state
            .with(|db| db.set_guide_done(&user_id))?
            .map_err(|_| LocalStateRefused::Unreadable)
    }
}

#[cfg(test)]
#[path = "guide_mark_tests.rs"]
mod tests;
