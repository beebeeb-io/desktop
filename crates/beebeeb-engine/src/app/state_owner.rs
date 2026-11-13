//! Is the local state on this computer the signed-in account's? A read-only check for a window that shows what the
//! local state database holds (names, sizes, the queue): after a sign-out that keeps local data, the state on disk can
//! still belong to an EARLIER account while another one is signed in (task 2488). Such a reader asks this first and
//! shows nothing when the answer is `false`.
//!
//! Compares the owner the engine start recorded (`StateDb::owner`, ruling R10) with the signed-in account through
//! the binding's own rule (`account_binding::same_account`). Changes nothing: it never records, adopts or resets an
//! owner (that stays `account_binding::bind_before_engine_start`'s). Must not: log a name, an email or an id.

use super::{CredentialStores, Engine};
use crate::account_binding::{Identity, same_account};
use crate::state_db::StateDb;

#[cfg(test)]
#[path = "state_owner_tests.rs"]
mod tests;

impl<C: CredentialStores> Engine<C> {
    /// `true` only when `db`'s recorded owner is the signed-in account. No one signed in, no owner recorded, an owner
    /// that cannot be compared or cannot be read: `false` (never shown). In memory and one small read: never waits
    /// on the network.
    pub fn owns_local_state(&self, db: &StateDb) -> bool {
        owned_by(
            db,
            &Identity::new(self.user_id().as_deref(), self.account_email().as_deref()),
        )
    }
}

/// [`Engine::owns_local_state`] for a known `session`: the owner is recorded, and the binding's rule says it is the
/// same account.
pub fn owned_by(db: &StateDb, session: &Identity) -> bool {
    if !session.is_known() {
        return false;
    }
    match db.owner() {
        Ok(Some(owner)) if owner.is_known() => same_account(&owner, session) == Some(true),
        _ => false,
    }
}
