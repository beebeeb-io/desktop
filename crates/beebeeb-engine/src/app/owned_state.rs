//! The local state as a shell reaches it (task 2488): through a handle that is bound to the signed-in account.
//!
//! [`Engine::local_state`] answers a handle only while a session is unlocked in memory, the server has not ended
//! it, and the state's recorded owner is that session's account (`account_binding::decide`, the rule every engine
//! start binds with). Every read through the handle ([`OwnedState::with`]) asks again whether the session is still
//! the one the handle was made for: a lock, a sign-out, another sign-in or a session the server ended turns the
//! handle into a refusal, and an answer that was read across such a change is dropped.
//!
//! One step is let through for a session the server ended (privacy): a session end (either sign-out, or
//! the close of that session) deletes the session's OWN recent searches
//! ([`Engine::delete_own_recent_searches_at_session_end`]). The owner is proven and the session checked as for every
//! handle; only the server's refusal is not asked, and that handle never leaves this module. A delete that fails is
//! logged and made at the account's next open for changes. An engine start with nobody signed in deletes every
//! account's recent searches: the one write here with no owner proven.
//!
//! What a state that is not the session's means is decided here, once, for every window: nothing is read, and the
//! reason is a fixed word ([`LocalStateRefused`]).
//!
//! WHICH database belongs to an account is resolved here too ([`Engine::local_state_path`]): today one file per
//! computer, whose owner is checked; one file per account is a change inside this module.
//!
//! Must not: log or return a file's name, a path, an email, a user id, a token or a key; write anything before the
//! owner is proven (the proof reads on a read-only connection); reset, adopt or delete local data (the one wipe
//! above deletes a single row: the ending session's own recent searches).

use std::path::PathBuf;

use super::lock_api::{Call, CallRefused, LockedAction};
use super::{CredentialStores, Engine};
use crate::account::{AccountRuntime, SessionGeneration};
use crate::account_binding::{self, Binding, BindingKind, Identity};
use crate::signin::start;
use crate::state_db::{ReadOnlyOpenError, StateDb};
use crate::state_paths;
use beebeeb_api::flows::call_may_run;

/// Why the local state is not handed out. A fixed word each: none carries a name, a path, an email or an id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalStateRefused {
    /// Nobody is signed in.
    SignedOut,
    /// Signed in, the account's keys are not in memory (locked, or the master key is still to come).
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

/// Whether a session the server ended (the refusal run, `auth_health.is_expired()`) is refused.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ended {
    /// Every handle and every identity this module answers: refused as [`LocalStateRefused::SessionEnded`].
    Refused,
    /// Only [`Engine::delete_own_recent_searches_at_session_end`], a session end's wipe of the session's own recent
    /// searches: it removes that row and writes nothing else.
    OwnSearchesWipe,
}

/// The local state of the signed-in account. Made by [`Engine::local_state`] or
/// [`Engine::local_state_for_changes`]; it borrows the engine handle, so it cannot be kept beyond it, and it
/// answers only while the session it was made for is still the session.
pub struct OwnedState<'e> {
    acct: &'e AccountRuntime,
    /// The engine's session-write lock: the handle asks whether its session is still there with no session change
    /// half done.
    writes: &'e std::sync::RwLock<()>,
    generation: SessionGeneration,
    db: std::sync::Arc<StateDb>,
    /// [`Ended::Refused`] on every handle a caller outside this module gets.
    ended: Ended,
}

impl OwnedState<'_> {
    /// The session this handle was made for.
    pub fn session(&self) -> SessionGeneration {
        self.generation
    }

    /// Read (or, on a handle for changes, write) through the state database. Refused when the session is no longer
    /// the one this handle was made for, before the closure runs; and when it changed while the closure ran, in
    /// which case the closure's answer is dropped.
    pub fn with<T>(&self, read: impl FnOnce(&StateDb) -> T) -> Result<T, LocalStateRefused> {
        self.still_this_session()?;
        let answer = read(&self.db);
        self.still_this_session()?;
        Ok(answer)
    }

    /// [`Self::with`] for an operation of many steps: `operation` is also handed a question to ask before each step,
    /// "is this still the session the handle was made for?". Once the answer is no, it must take no further step.
    pub(crate) fn with_while<T>(
        &self,
        operation: impl FnOnce(&StateDb, &dyn Fn() -> bool) -> T,
    ) -> Result<T, LocalStateRefused> {
        self.still_this_session()?;
        let answer = operation(&self.db, &|| self.still_this_session().is_ok());
        self.still_this_session()?;
        Ok(answer)
    }

    /// [`Self::with`] for the engine's own operations that need the database shared (the sync bridge takes an
    /// `Arc`). The same checks, before and after; the engine's alone, so a shell cannot keep the `Arc`.
    pub(crate) fn with_shared<T>(
        &self,
        operation: impl FnOnce(std::sync::Arc<StateDb>) -> T,
    ) -> Result<T, LocalStateRefused> {
        self.still_this_session()?;
        let answer = operation(self.db.clone());
        self.still_this_session()?;
        Ok(answer)
    }

    /// Whether the handle would still answer: no session change since it was made, and a session in memory, asked
    /// with no session change half done. Never waits: while a session change is under way the handle answers
    /// nothing. A handle made while a lock or a sign-out was ending its session answers nothing, then and after.
    pub fn still_this_session(&self) -> Result<(), LocalStateRefused> {
        self.acct
            .while_session_is(self.writes, self.generation, |_| ())
            .ok_or(LocalStateRefused::SessionChanged)?;
        if self.ended == Ended::Refused && self.acct.auth_health.is_expired() {
            return Err(LocalStateRefused::SessionEnded);
        }
        Ok(())
    }
}

/// What a person asks through the window (core's `Call::Person`): a locked screen refuses it.
const PERSON: Call = Call::Person(LockedAction::ViewStatus);

impl<C: CredentialStores> Engine<C> {
    /// The signed-in account's local state, for reading. See the module's doc for when it is refused.
    ///
    /// For what a PERSON reads through the window: a locked screen is refused as `Locked` (core's `call_may_run`;
    /// core cannot see the local state, so the host's side applies its rule). The work and the operating system's
    /// file manager, which reach the sync folder on a locked screen by design, use [`Self::local_state_for_work`].
    pub fn local_state(&self) -> Result<OwnedState<'_>, LocalStateRefused> {
        self.owned_state(Mode::Read, PERSON)
    }

    /// A locked screen refuses `call` as `Locked` (core's `call_may_run`); the work is not refused.
    pub(super) fn person_asks(&self, call: Call) -> Result<(), LocalStateRefused> {
        if call_may_run(self.lock_state(), call) == Err(CallRefused::ScreenLocked) {
            return Err(LocalStateRefused::Locked);
        }
        Ok(())
    }

    /// The account's local state for a change that no person drives through the window (Explorer's own command):
    /// not asked of the screen lock; every other refusal is [`Self::local_state_for_changes`]'s.
    pub(super) fn local_state_for_work(&self) -> Result<OwnedState<'_>, LocalStateRefused> {
        self.owned_state(Mode::Change, Call::Work)
    }

    /// Explorer's command-state question: a read-only connection, no schema check under a write lock, so it never
    /// waits on the sync engine's writes. Not refused behind a locked screen (the work).
    pub(super) fn local_state_work_refusal(&self) -> Option<LocalStateRefused> {
        self.owned_state(Mode::Read, Call::Work).err()
    }

    /// The signed-in account's local state, for an operation that writes (a rename, a move, a request to the sync
    /// engine). The owner is proven on a read-only connection first: nothing is written into a state that is not
    /// the session's, not even a schema migration.
    pub fn local_state_for_changes(&self) -> Result<OwnedState<'_>, LocalStateRefused> {
        self.owned_state(Mode::Change, PERSON)
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
        self.session_identity_as(Ended::Refused)
    }

    /// [`Self::session_identity`]; `ended` says whether a session the server ended is refused.
    fn session_identity_as(&self, ended: Ended) -> Result<Identity, LocalStateRefused> {
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
        if ended == Ended::Refused && self.acct.auth_health.is_expired() {
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

    /// WRITES, one step of every session end that keeps the local state: both sign-outs ([`Engine::sign_out_for`],
    /// [`Engine::sign_out_keeping_local_data`]) and the close of a session the server ended
    /// ([`Engine::close_ended_session`], the session watcher's, and the close a confirmed 401 asks). The recent
    /// searches of the account of `session` are deleted from this computer while the session is still in memory. The one step the account-bound handle lets through after the refusal run: it deletes that session's OWN row (keyed by its user id, never an email) and writes nothing else. Every
    /// other check is the handle's: the owner proven on a read-only connection first, `session` still the session in
    /// memory before and after. A locked screen does not refuse it (the step's own work, not a person's read).
    ///
    /// `Err`: nothing was deleted, and the step goes on. A state that is not provably this account's, or none at all,
    /// holds nothing of it. A state that could not be opened or written (`Unreadable`: the sync engine held it, for
    /// one) is logged once, and the delete stays due: it is made at the account's next open for changes, and until
    /// then the account's recent searches are neither shown nor added to ([`Self::recent_searches_due`]).
    pub(super) fn delete_own_recent_searches_at_session_end(
        &self,
        session: SessionGeneration,
    ) -> Result<(), LocalStateRefused> {
        // The user id first: a delete that fails further on is still due for it.
        let user_id = self
            .session_identity_as(Ended::OwnSearchesWipe)?
            .user_id
            .ok_or(LocalStateRefused::IdentityUnknown)?;
        let deleted = self
            .owned_state_as(Mode::Change, Call::Work, Ended::OwnSearchesWipe)
            .and_then(|state| {
                // The handle's session is the one the step named, not a later one that opened in between; the turn only
                // moves on, so the user id read above is that session's too.
                if state.session() != session {
                    return Err(LocalStateRefused::SessionChanged);
                }
                state
                    .with(|db| db.clear_search_recent(&user_id))?
                    .map_err(|_| LocalStateRefused::Unreadable)
            });
        match deleted {
            Ok(()) => {
                self.mark_recent_searches_due(&user_id, false);
            }
            Err(LocalStateRefused::Unreadable) => {
                if self.mark_recent_searches_due(&user_id, true) {
                    tracing::warn!("search query row not deleted at session end: pending, retry at next open");
                }
            }
            Err(_) => {}
        }
        deleted
    }

    /// WRITES, at the engine's start ([`Engine::restore`]): a start with
    /// nobody signed in (`session_stored` false, no session in memory) deletes EVERY account's recent searches still on
    /// this computer: a signed-out start owns nobody's searches. That covers a restart after a session end whose delete
    /// failed (its due mark lived in memory). A start with a stored session keeps them: the same person, and the owner
    /// check keeps any other account out of them.
    ///
    /// The one write this module makes with no owner proven, and only of these rows: they are looked for on a
    /// read-only connection first, so a start where none is there writes nothing (not even a schema migration), and a
    /// computer where nothing has synced gets no database. A delete that fails is logged.
    pub(super) fn delete_recent_searches_at_signed_out_start(&self, session_stored: bool) {
        if session_stored || self.unlocked() {
            return;
        }
        let path = self.local_state_path(&Identity::new(None, None));
        // `None`: whether any is there could not be read.
        let any = match StateDb::open_read_only(&path) {
            Ok(proof) => proof.has_any_search_recent().ok(),
            // Nothing synced, or no state database of this app's (or one too old to hold any): nothing to delete.
            Err(
                ReadOnlyOpenError::NotFound
                | ReadOnlyOpenError::NotADatabase
                | ReadOnlyOpenError::NotAStateDatabase
                | ReadOnlyOpenError::OlderSchema,
            ) => return,
            Err(ReadOnlyOpenError::Locked | ReadOnlyOpenError::Unreadable) => None,
        };
        let deleted = match any {
            Some(false) => return,
            Some(true) => StateDb::open(&path)
                .and_then(|db| db.clear_every_search_recent())
                .is_ok(),
            None => false,
        };
        if deleted {
            self.recent_searches_due
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clear();
            tracing::info!("search query rows deleted at start");
        } else {
            tracing::warn!("search query rows not deleted at start");
        }
    }

    /// Whether a session end's delete of the recent searches of `user_id` is still due: it failed, and
    /// no open made it since. Held in memory: a restart forgets it, and a start with nobody signed in deletes every
    /// recent search ([`Self::delete_recent_searches_at_signed_out_start`]).
    pub(super) fn recent_searches_due(&self, user_id: &str) -> bool {
        self.recent_searches_due
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(user_id)
    }

    /// Marks the delete of `user_id`'s recent searches due (`true`) or made (`false`); answers whether that changed.
    fn mark_recent_searches_due(&self, user_id: &str, due: bool) -> bool {
        let mut users = self
            .recent_searches_due
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if due {
            users.insert(user_id.to_string())
        } else {
            users.remove(user_id)
        }
    }

    /// The delete a session end could not make, made at a later open for changes of the account's state: through
    /// `state`, a handle proven the account of `user_id`'s. Fails quietly (it was logged once, and it stays due).
    fn make_due_recent_searches_delete(&self, state: &OwnedState<'_>, user_id: &str) {
        if !self.recent_searches_due(user_id) {
            return;
        }
        if let Ok(Ok(())) = state.with(|db| db.clear_search_recent(user_id)) {
            self.mark_recent_searches_due(user_id, false);
            tracing::info!("pending search query row deleted at open");
        }
    }

    fn owned_state(&self, mode: Mode, call: Call) -> Result<OwnedState<'_>, LocalStateRefused> {
        self.owned_state_as(mode, call, Ended::Refused)
    }

    fn owned_state_as(&self, mode: Mode, call: Call, ended: Ended) -> Result<OwnedState<'_>, LocalStateRefused> {
        // First, so a transition during anything below turns the handle into a refusal at its first read.
        let generation = self.acct.session_generation();
        self.person_asks(call)?;
        let session = self.session_identity_as(ended)?;
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
        // The binding's own rule, with data assumed: an owner is recorded.
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
            writes: &self.writes,
            generation,
            db: std::sync::Arc::new(db),
            ended,
        };
        state.still_this_session()?;
        // A delete a session end could not make is made at the account's next open for changes.
        if mode == Mode::Change
            && ended == Ended::Refused
            && let Some(user_id) = session.user_id.as_deref()
        {
            self.make_due_recent_searches_delete(&state, user_id);
        }
        Ok(state)
    }
}

/// The first-time guide's "done" mark of the session's own account (a child of this module: it resolves the
/// account's state and proves its owner with this module's own rules).
#[path = "guide_mark.rs"]
pub mod guide_mark;

/// Task 2530: a way out of a parked upload, chosen on This PC's Sync history (the window reads its rows through this
/// module's handle).
#[path = "parked_exit_api.rs"]
pub mod parked_exit_api;

#[cfg(test)]
#[path = "owned_state_tests.rs"]
mod tests;
