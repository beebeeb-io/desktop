//! The local state as a shell reaches it (task 2488): through a handle that is bound to the signed-in account.
//!
//! [`Engine::local_state`] answers a handle only while a session is unlocked in memory, the server has not ended
//! it, and the state's recorded owner is that session's account (`account_binding::decide`, the rule every engine
//! start binds with). Every read through the handle ([`OwnedState::with`]) asks again whether the session is still
//! the one the handle was made for: a lock, a sign-out, another sign-in or a session the server ended turns the
//! handle into a refusal, and an answer that was read across such a change is dropped.
//!
//! What a state that is not the session's means is decided here, once, for every window: nothing is read, and the
//! reason is a fixed word ([`LocalStateRefused`]).
//!
//! WHICH database belongs to an account is resolved here too ([`Engine::local_state_path`]): today one file per
//! computer, whose owner is checked; one file per account is a change inside this module.
//!
//! Must not: log or return a file's name, a path, an email, a user id, a token or a key; write anything before the
//! owner is proven (the proof reads on a read-only connection); reset, adopt or delete local data.

use std::path::PathBuf;

use super::{CredentialStores, Engine};
use crate::account::{AccountRuntime, SessionGeneration};
use crate::account_binding::{self, Binding, BindingKind, Identity};
use crate::signin::start;
use crate::state_db::{ReadOnlyOpenError, StateDb};
use crate::state_paths;

/// Why the local state is not handed out. A fixed word each: none carries a name, a path, an email or an id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalStateRefused {
    /// Nobody is signed in.
    SignedOut,
    /// Signed in, the account's keys are not in memory (locked, or the recovery phrase is still to come).
    Locked,
    /// The server refused this session's token again and again: it has ended.
    SessionEnded,
    /// Nothing has synced on this computer yet (no database, or one with no owner and no account data).
    NothingSynced,
    /// The local state holds account data and no record of whose it is.
    Unowned,
    /// The local state is recorded as another account's.
    OtherAccount,
    /// Whose the session is, or whose the local state is, could not be compared.
    IdentityUnknown,
    /// The session changed since the handle was made (a lock, a sign-out, another sign-in).
    SessionChanged,
    /// The local state could not be opened or read.
    Unreadable,
}

impl LocalStateRefused {
    /// The refusal's name, for a log line.
    pub fn kind(self) -> &'static str {
        match self {
            Self::SignedOut => "signed_out",
            Self::Locked => "locked",
            Self::SessionEnded => "session_ended",
            Self::NothingSynced => "nothing_synced",
            Self::Unowned => "unowned",
            Self::OtherAccount => "other_account",
            Self::IdentityUnknown => "identity_unknown",
            Self::SessionChanged => "session_changed",
            Self::Unreadable => "unreadable",
        }
    }

    /// This computer holds local data that is not provably the signed-in account's: a window shows none of it and
    /// says so (the same three cases a sync start refuses as `OtherAccountsData` or `IdentityUnknown`).
    pub fn not_this_accounts(self) -> bool {
        matches!(self, Self::Unowned | Self::OtherAccount | Self::IdentityUnknown)
    }
}

impl std::fmt::Display for LocalStateRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.kind())
    }
}

/// How the handle's connection is opened.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Read-only (`StateDb::open_read_only`): no schema check under a write lock, nothing can be written.
    Read,
    /// Read-write, opened only after the owner was proven on a read-only connection.
    Change,
}

/// The local state of the signed-in account. Made by [`Engine::local_state`] or
/// [`Engine::local_state_for_changes`]; it borrows the engine handle, so it cannot be kept beyond it, and it
/// answers only while the session it was made for is still the session.
pub struct OwnedState<'e> {
    acct: &'e AccountRuntime,
    generation: SessionGeneration,
    db: StateDb,
}

impl OwnedState<'_> {
    /// Read (or, on a handle for changes, write) through the state database. Refused when the session is no longer
    /// the one this handle was made for, before the closure runs; and when it changed while the closure ran, in
    /// which case the closure's answer is dropped.
    pub fn with<T>(&self, read: impl FnOnce(&StateDb) -> T) -> Result<T, LocalStateRefused> {
        self.still_this_session()?;
        let answer = read(&self.db);
        self.still_this_session()?;
        Ok(answer)
    }

    /// Whether the handle would still answer.
    pub fn still_this_session(&self) -> Result<(), LocalStateRefused> {
        if !self.acct.session_unchanged_since(self.generation) {
            return Err(LocalStateRefused::SessionChanged);
        }
        if self.acct.auth_health.is_expired() {
            return Err(LocalStateRefused::SessionEnded);
        }
        Ok(())
    }
}

impl<C: CredentialStores> Engine<C> {
    /// The signed-in account's local state, for reading. See the module's doc for when it is refused.
    pub fn local_state(&self) -> Result<OwnedState<'_>, LocalStateRefused> {
        self.owned_state(Mode::Read)
    }

    /// The signed-in account's local state, for an operation that writes (a rename, a move, a request to the sync
    /// engine). The owner is proven on a read-only connection first: nothing is written into a state that is not
    /// the session's, not even a schema migration.
    pub fn local_state_for_changes(&self) -> Result<OwnedState<'_>, LocalStateRefused> {
        self.owned_state(Mode::Change)
    }

    /// Why the local state is not handed out now, for a shell that says so; `None` while it is.
    pub fn local_state_refusal(&self) -> Option<LocalStateRefused> {
        self.local_state().err()
    }

    /// Where the signed-in account's state database is. Today: the one database of this computer's state folder,
    /// whoever is signed in (its owner is checked by [`Self::owned_state`]). One database per account resolves
    /// here, from the session's identity.
    fn local_state_path(&self, _session: &Identity) -> PathBuf {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(path) = self
            .local_state_for_test
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
        {
            return path;
        }
        state_paths::state_db_path_from_state_dir(&self.config.state_dir)
    }

    /// Test builds only: the handle resolves `path` as the account's state database, so a test reads a database of
    /// its own instead of the process's one state folder. A sync start is not moved by it.
    #[cfg(any(test, feature = "test-support"))]
    pub fn use_local_state_for_test(&self, path: PathBuf) {
        *self
            .local_state_for_test
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(path);
    }

    /// Who the session in memory is, as the binding compares it: the session's own email, the user id the server
    /// named for it (the cached profile when it is that email's, else the id recorded with the stored session).
    /// `Err`: there is no session to hand local state to.
    pub(super) fn session_identity(&self) -> Result<Identity, LocalStateRefused> {
        let email = {
            let session = self
                .acct
                .session
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match session.as_ref() {
                Some(session) => session.email.clone(),
                None => {
                    let present = self.auth_present.lock().map(|present| *present).unwrap_or(false);
                    return Err(if present {
                        LocalStateRefused::Locked
                    } else {
                        LocalStateRefused::SignedOut
                    });
                }
            }
        };
        if self.acct.auth_health.is_expired() {
            return Err(LocalStateRefused::SessionEnded);
        }
        let profile = self.acct.cached_profile.lock().ok().and_then(|guard| guard.clone());
        Ok(self.with_recorded_user_id(start::identity_of_session(email.as_deref(), profile.as_ref())))
    }

    /// `identity`, with the user id recorded for this session (at its sign-in, or with the stored session a restart
    /// read) when the identity has none: an email alone never decides while the id is known.
    pub(super) fn with_recorded_user_id(&self, mut identity: Identity) -> Identity {
        if identity.user_id.is_none() {
            identity.user_id = self
                .session_user_id
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
        }
        identity
    }

    fn owned_state(&self, mode: Mode) -> Result<OwnedState<'_>, LocalStateRefused> {
        // First, so a transition during anything below turns the handle into a refusal at its first read.
        let generation = self.acct.session_generation();
        let session = self.session_identity()?;
        let path = self.local_state_path(&session);
        if !path.exists() {
            return Err(LocalStateRefused::NothingSynced);
        }
        let proof = StateDb::open_read_only(&path).map_err(|error| match error {
            ReadOnlyOpenError::NotFound => LocalStateRefused::NothingSynced,
            _ => LocalStateRefused::Unreadable,
        })?;
        let owner = proof
            .owner()
            .map_err(|_| LocalStateRefused::Unreadable)?
            .filter(Identity::is_known);
        let Some(owner) = owner else {
            // No record of an owner: nothing is read, whatever is there. With account data it is somebody's.
            let holds_data = proof.has_account_data().map_err(|_| LocalStateRefused::Unreadable)?;
            return Err(if holds_data {
                LocalStateRefused::Unowned
            } else {
                LocalStateRefused::NothingSynced
            });
        };
        // The binding's own rule (R10), with data assumed: an owner is recorded.
        match account_binding::decide(Some(&owner), &session, BindingKind::LocalData) {
            Binding::Proceed => {}
            Binding::Reset => return Err(LocalStateRefused::OtherAccount),
            Binding::Refuse => return Err(LocalStateRefused::IdentityUnknown),
        }
        let db = match mode {
            Mode::Read => proof,
            Mode::Change => {
                drop(proof);
                StateDb::open(&path).map_err(|_| LocalStateRefused::Unreadable)?
            }
        };
        let state = OwnedState {
            acct: &self.acct,
            generation,
            db,
        };
        state.still_this_session()?;
        Ok(state)
    }
}

#[cfg(test)]
#[path = "owned_state_tests.rs"]
mod tests;
