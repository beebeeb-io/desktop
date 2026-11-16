//! The two-level lock of this app, bound to core's rules (decision 2495; core's contract
//! `docs/superpowers/specs/2026-11-13-two-level-lock-design.md`).
//!
//! The rules are core's and are not restated here: the state machine is `beebeeb_api::flows::LockFlow` (one per
//! handle, every move of the lock is one of its steps), what a locked screen refuses is `flows::call_may_run`, and
//! how the app starts is `LockFlow::at_start`. What this file adds is the handle's side of core's lock service
//! (`beebeeb-client`'s `auth/lock.rs`, which this engine cannot call: it keeps its own session and credential
//! store), step for step in that service's order:
//!
//! - the flags a start reads are stored beside the session under core's entry names, as `1` / `0`; a vault flag that
//!   cannot be read starts the vault closed, and a start that is closed writes the mark; app lock (absent or
//!   unreadable is on) is the screen level's alone: off, a start that would be screen-locked starts open
//!   (`app_lock_api.rs`);
//! - a vault lock writes the mark BEFORE the key leaves memory, and reads it back after;
//! - only a strong proof (the password accepted by the server, or the twelve words) clears the mark;
//! - while the vault is marked closed the key entry is not read at all, not even to ask whether it is there;
//! - a session the server no longer accepts closes the vault ([`Engine::close_ended_session`], asked every second by
//!   [`Engine::watch_session`]); a person's call reaches the session only through [`Engine::person_session`].
//!
//! Neither level encrypts anything on this computer: the key stays in the credential store. The screen lock gates
//! the window (sync keeps running); the vault lock takes the key out of this process and stops the work.
//!
//! Must not: open any level on a proof that did not complete; hold a state of its own beside the flow.

pub use beebeeb_api::flows::{Call, CallRefused, LockState, LockedAction};
use beebeeb_api::flows::{FlowError, LockFlow, LockLevel, StartFacts, call_may_run, key_may_be_loaded};
use zeroize::Zeroizing;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use super::{CredentialStores, Engine, SyncStop};
use crate::account::{SessionGeneration, SessionTransition};
use crate::keychain::{AuthVault, LockFlag};
use crate::session::Session;
use crate::signin::gate::{NextAfterSignIn, SignInProblem};
use crate::signin::store;

/// How often the session watcher asks whether the server still accepts the session.
pub(super) const SESSION_WATCH_EVERY: Duration = Duration::from_secs(1);

/// What a handle knows before its start read anything: no session.
pub(super) const NO_SESSION: StartFacts = StartFacts {
    session_stored: false,
    ask_password_at_start: None,
    vault_closed: None,
    app_lock: None,
};

/// Why a step of the lock did nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockRefused {
    /// The app is not locked at this level (core's `StepNotRequired`): nothing to open, or nothing to lock.
    NotAtThisLevel,
    /// Signed out, or the vault is closed: only the account password, proven to the server, opens it (core's
    /// `NotReady`).
    NeedsThePassword,
    /// The session or the lock moved since the press this step answers: a late answer opens nothing.
    Overtaken,
    /// Nothing was typed: nothing is checked and nothing is counted.
    NothingTyped,
    /// A password check is already on its way: this press checks nothing.
    AlreadyChecking,
    /// The check could not run (the count could not be stored, the account's owner is unknown, or the local check
    /// could not allocate): nothing opened and nothing was sent.
    CouldNotCheck,
}

fn refused(error: FlowError) -> LockRefused {
    match error {
        FlowError::StepNotRequired => LockRefused::NotAtThisLevel,
        FlowError::StepOutOfOrder | FlowError::NotReady => LockRefused::NeedsThePassword,
    }
}

/// Why a person's call got no session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PersonRefused {
    /// A session is in memory behind a locked screen.
    ScreenLocked,
    /// A session is in memory and the vault is closed (core's `closed_vault_allows`: a person may only prove, lock or
    /// sign out there; credentials on their way to an install).
    VaultClosed,
    /// No session in memory (signed out, a closed vault whose key left, a session change under way), whatever the
    /// lock says: never "locked".
    NoSession,
}

impl PersonRefused {
    /// THE one mapping of a refusal to an API's words: `locked` for a session that exists behind a locked screen or a
    /// closed vault, `not_signed_in` for no session.
    pub(super) fn said_as<E>(self, locked: E, not_signed_in: E) -> E {
        match self {
            Self::ScreenLocked | Self::VaultClosed => locked,
            Self::NoSession => not_signed_in,
        }
    }
}

/// What [`Engine::lock_vault`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VaultLocked {
    pub stopped: SyncStop,
    /// The closed mark is stored. `false`: the vault is closed in this run only; say that the lock will not
    /// survive a restart.
    pub survives_restart: bool,
}

/// What became of the session in memory, for the lock to follow in the same hold of the session's write lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SessionMoved {
    /// A strong proof put the key in memory: the password accepted by the server, or the twelve words.
    StrongProof,
    /// A session is stored and its key is not in memory (a sign-in on a computer without the key).
    Keyless,
    /// The key left memory; the stored session stays.
    KeyDropped,
    SignedOut,
    /// The server no longer accepts the session: the key is dropped and only a fresh password sign-in opens it.
    Ended,
}

impl SessionMoved {
    /// What a session transition that ended the session in memory means for the lock: a sign-out signs out, every
    /// other one drops the key.
    pub(super) fn after(by: SessionTransition) -> Self {
        if by == SessionTransition::SignOut {
            Self::SignedOut
        } else {
            Self::KeyDropped
        }
    }
}

impl<C: CredentialStores> Engine<C> {
    /// The stored key of this account, loaded through the guarded loader at the start.
    pub(super) fn restore_key(&self, turn: SessionGeneration, vault: AuthVault<C::Store>) {
        match store::load_session_from_vault(vault, None) {
            Ok(Some(session)) => {
                if let Ok(_write) = self.claim(turn)
                    && let Ok(mut guard) = self.acct.session.lock()
                {
                    if let Ok(mut email) = self.acct.auth_email.lock() {
                        email.clone_from(&session.email);
                    }
                    *guard = Some(session);
                    self.acct.auth_resume_asked.fetch_add(1, Ordering::SeqCst);
                }
            }
            Ok(None) => {}
            Err(_) => tracing::info!("startup restore: signed in, the vault stays locked"),
        }
    }

    fn flow<T>(&self, step: impl FnOnce(&mut LockFlow) -> T) -> T {
        step(&mut self.lock_flow.lock().unwrap_or_else(|p| p.into_inner()))
    }

    /// Where the lock is, by core's state machine.
    pub fn lock_state(&self) -> LockState {
        self.flow(|flow| flow.state())
    }

    /// Core's answer for one call under the lock as it is now (`flows::call_may_run`): a locked screen refuses
    /// what a person drives and lets the work run; a closed vault refuses the work.
    pub fn check_call(&self, call: Call) -> Result<(), CallRefused> {
        call_may_run(self.lock_state(), call)
    }

    /// THE way a person's call reaches the session in memory: the lock is asked and `with` runs in ONE hold of the
    /// session-write lock (the read side, which never waits: a session change under way answers
    /// [`PersonRefused::NoSession`]). Every lock move holds that lock for writing, so the check and the copy
    /// cannot be split. `with` must only copy or compare: every session change waits for it.
    pub(super) fn person_session<T>(
        &self,
        with: impl FnOnce(&Session, SessionGeneration) -> T,
    ) -> Result<T, PersonRefused> {
        self.person_session_for(LockedAction::ViewStatus, with)
    }

    /// [`Self::person_session`] for the one action `action`: the screen lock's own unlock is the call a locked
    /// screen lets through (core's `screen_locked_allows`), so the password proof asks as `UnlockScreen`.
    pub(super) fn person_session_for<T>(
        &self,
        action: LockedAction,
        with: impl FnOnce(&Session, SessionGeneration) -> T,
    ) -> Result<T, PersonRefused> {
        let _writes = match self.writes.try_read() {
            Ok(writes) => writes,
            Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Err(PersonRefused::NoSession),
        };
        // The lock's answer, then the session, in this one hold (every lock move waits for it): no session is
        // `NoSession` whatever the lock says, and a session the lock refuses is never handed to `with`.
        let answer = call_may_run(self.lock_state(), Call::Person(action));
        let guard = self
            .acct
            .session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(session) = guard.as_ref() else {
            return Err(PersonRefused::NoSession);
        };
        match answer {
            Ok(()) => Ok(with(session, self.acct.session_generation())),
            Err(CallRefused::ScreenLocked) => Err(PersonRefused::ScreenLocked),
            Err(CallRefused::VaultClosed) => Err(PersonRefused::VaultClosed),
        }
    }

    /// The start rule, for a start that found no key in memory. A closed start writes the mark, so every later
    /// start stays closed until the strong proof. The flow is not installed yet: [`Self::lock_started`] does that
    /// once the start knows whether a key was loaded.
    pub(super) fn lock_start_rule(&self, vault: &AuthVault<C::Store>, session_stored: bool) -> LockFlow {
        let facts = StartFacts {
            session_stored,
            ask_password_at_start: vault.lock_flag(LockFlag::AskPasswordAtStart),
            vault_closed: vault.lock_flag(LockFlag::VaultClosed),
            // Absent is ON, and so is an entry that cannot be read (core's `read_lock_setting(.., true)`).
            app_lock: vault.lock_setting(LockFlag::AppLock, true),
        };
        let flow = LockFlow::at_start(facts);
        if flow.state() == LockState::VaultClosed && facts.vault_closed != Some(true) {
            // A mark that cannot be written leaves this run closed all the same.
            let _ = vault.set_lock_flag(LockFlag::VaultClosed, true);
        }
        flow
    }

    /// The start is over: `flow` is the lock. A start that may hold the key and found none usable (a session
    /// without its key on this computer) is closed without a mark: the twelve words are its strong proof.
    pub(super) fn lock_started(&self, mut flow: LockFlow) {
        if key_may_be_loaded(flow.state()) && !self.unlocked() {
            let _ = flow.lock(LockLevel::Vault);
        }
        self.flow(|current| *current = flow);
    }

    /// The lock follows the session, in the caller's hold of the session's write lock.
    pub(super) fn lock_follows(&self, moved: SessionMoved) {
        match moved {
            SessionMoved::StrongProof => {
                // Cleared here and nowhere else. A mark that cannot be removed closes the next start again.
                let _ = self.vault().clear_lock_flag(LockFlag::VaultClosed);
                self.flow(|flow| {
                    let _ = flow.known_device_unlocked();
                });
            }
            SessionMoved::Keyless => {
                // The password was proven, so a mark is cleared here too: from now on the gate may ask the store
                // whether a key is there (none is: the twelve words are next).
                let _ = self.vault().clear_lock_flag(LockFlag::VaultClosed);
                // A stored session whose key is not in this process: closed, whatever the flags say.
                self.flow(|flow| {
                    *flow = LockFlow::at_start(StartFacts {
                        session_stored: true,
                        ..NO_SESSION
                    });
                });
            }
            SessionMoved::KeyDropped => self.flow(|flow| {
                let _ = flow.lock(LockLevel::Vault);
            }),
            SessionMoved::SignedOut => self.flow(LockFlow::signed_out),
            SessionMoved::Ended => self.flow(LockFlow::session_ended),
        }
    }

    /// The credential store for a sign-in's install (`store_first_sign_in`, which a sign-in and a sign-up both pass),
    /// with every lock entry of another account removed first. An account's first install on this computer, or one
    /// after another account's, carries nothing of a lock setting (core's first `Session::save` and its account
    /// change), so a sign-out cut short never hands its `lock.app_lock = 0` to the next account. The entries stay
    /// when the stored identity is this account's (a strong unlock, a token the server rejected at a start): the
    /// session's owner tag, else the key's, else (no tag at all) the stored email. A store that cannot say refuses.
    pub(super) fn vault_for_install(&self, user_id: &str, email: &str) -> Result<AuthVault<C::Store>, String> {
        const UNREAD: &str = "the lock settings stored on this computer could not be read";
        let vault = self.vault();
        let owner = match vault.session_owner().map_err(|_| UNREAD.to_string())? {
            Some(owner) => Some(owner),
            None => vault.vault_key_owner().map_err(|_| UNREAD.to_string())?,
        };
        let ours = match owner {
            Some(owner) => owner.trim() == user_id.trim(),
            None => vault
                .account_email()
                .map_err(|_| UNREAD.to_string())?
                .is_some_and(|stored| stored.trim().eq_ignore_ascii_case(email.trim())),
        };
        if !ours {
            vault
                .clear_lock_entries()
                .map_err(|_| "the lock settings of an earlier account could not be removed".to_string())?;
        }
        Ok(vault)
    }

    /// Is the vault marked closed (or can the mark not be read)? Then the key entry is not read, also not to ask
    /// whether a key is there.
    pub(super) fn marked_closed(&self, vault: &AuthVault<C::Store>) -> bool {
        vault.lock_flag(LockFlag::VaultClosed) != Some(false)
    }

    /// Lock the screen: by hand, by the host's policy, or with the operating system's session. The key and the
    /// work stay as they are. Every answer asked for before this moment is for another turn of the session
    /// ([`Engine::session_turn`] moves on), so a host drops it. Never waits on a session step.
    pub fn lock_screen(&self) -> Result<(), LockRefused> {
        {
            let _writes = self.writes.write().unwrap_or_else(|p| p.into_inner());
            self.flow(|flow| flow.lock(LockLevel::Screen)).map_err(refused)?;
            self.acct
                .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
        }
        self.publish();
        Ok(())
    }

    /// Open the screen lock on the host's statement that the operating system verified the person at the keyboard
    /// (Windows Hello returned success). `asked_at` is [`Engine::session_turn`] as the host read it WHEN THE PROMPT
    /// WAS ASKED FOR: an answer that outlived a lock, an unlock, a sign-out or another sign-in opens nothing. It
    /// never opens a closed vault. A cancelled, failed or unavailable prompt calls nothing.
    pub fn unlock_screen_platform_verified(&self, asked_at: SessionGeneration) -> Result<(), LockRefused> {
        {
            let _writes = self.writes.write().unwrap_or_else(|p| p.into_inner());
            if !self.acct.session_unchanged_since(asked_at) {
                return Err(LockRefused::Overtaken);
            }
            self.flow(LockFlow::platform_verified).map_err(refused)?;
        }
        self.publish();
        Ok(())
    }

    /// "Ask for my password when Beebeeb starts". `None`: the setting could not be read (the next start is closed).
    pub fn ask_password_at_start(&self) -> Option<bool> {
        self.vault().lock_flag(LockFlag::AskPasswordAtStart)
    }

    /// Close the vault. In core's order: the closed mark is written; the engine stops and the key leaves this
    /// process (the session's turn moves on, so every step that began before is refused at its write); the mark is
    /// read back and written again when a strong proof that finished just before cleared it. The stored session and
    /// key stay in the credential store. Signed out: the stop alone, and nothing is marked.
    pub fn lock_vault(&self) -> VaultLocked {
        let locked = self.runtime.block_on(async {
            let _step = self.session_ending_step().await;
            // A proof that did not end in time (`ProofStop::Unconfirmed`) does not hold the Lock back: the vault closes
            // all the same, and the proof's seat refuses every start until its client is gone.
            let _ = self.stop_proofs().await;
            let vault = self.vault();
            let signed_in = self.lock_state() != LockState::SignedOut;
            let mut survives_restart = signed_in && vault.set_lock_flag(LockFlag::VaultClosed, true).is_ok();
            let stopped = self.stop_holding_step().await;
            self.forget_session(SessionTransition::Lock);
            if signed_in && vault.lock_flag(LockFlag::VaultClosed) != Some(true) {
                survives_restart = vault.set_lock_flag(LockFlag::VaultClosed, true).is_ok();
            }
            VaultLocked {
                stopped,
                survives_restart,
            }
        });
        self.publish();
        locked
    }

    /// A session the server no longer accepts closes the vault, as core's lock does. Core's words: "The server no
    /// longer accepts the session: the key is dropped, and only a fresh password sign-in with the known-device
    /// unlock opens the app again" (`LockFlow::session_ended`); and its client service: "the key is dropped if it
    /// was still held, the check record and the wrong-try count are deleted, and the closed mark is written, so the
    /// next start does not load the key for a session the server no longer accepts" (this engine stores both, under
    /// core's entry names, and deletes them here too).
    ///
    /// Does nothing (`None`) while the session is accepted, or when no key is held (signed out, or the vault is
    /// already closed). The stored session and key stay in the credential store; sign-out is not done. The
    /// refusal run is left alone: a strong unlock clears it before its new session is installed, so a close that
    /// runs after one finds nothing to close.
    ///
    /// Blocks on the engine's runtime: never call it from the window's thread or from a task of that runtime.
    pub fn close_ended_session(&self) -> Option<VaultLocked> {
        self.close_session_the_server_refused(None)
    }

    /// [`Self::close_ended_session`], or (`confirmed_at`: the turn of a password proof the server refused with a 401
    /// that a second question, `GET /auth/me`, confirmed) the same close without waiting for the refusal streak. A
    /// session that moved since `confirmed_at` closes nothing. A running ownership proof stops first. The account's
    /// recent searches are deleted on both closes, after the refusal streak too: the one wipe the account-bound handle
    /// still makes for a session the server ended ([`Self::delete_own_recent_searches_at_session_end`]).
    pub(super) fn close_session_the_server_refused(
        &self,
        confirmed_at: Option<SessionGeneration>,
    ) -> Option<VaultLocked> {
        let may_close = || {
            let ended = match confirmed_at {
                Some(turn) => self.acct.session_unchanged_since(turn),
                None => self.acct.auth_health.is_expired(),
            };
            ended && key_may_be_loaded(self.lock_state())
        };
        if !may_close() {
            return None;
        }
        let locked = self.runtime.block_on(async {
            let _step = self.session_ending_step().await;
            if !may_close() {
                return None;
            }
            // A proof stops wherever a session ends, before the engine, as at a lock and a sign-out. One that did not
            // end in time does not hold the close back: its seat refuses every start until its client is gone.
            let _ = self.stop_proofs().await;
            // The account's recent searches leave this computer while the session is still in memory, as at a
            // sign-out, also once the refusal run marked the session ended (its own row only).
            let _ = self.delete_own_recent_searches_at_session_end(self.acct.session_generation());
            let vault = self.vault();
            // Core: the check record and the wrong-try count are deleted with the key, and the mark is written.
            let _ = vault.clear_lock_flag(LockFlag::ScreenCheck);
            let _ = vault.clear_lock_flag(LockFlag::Attempts);
            let mut survives_restart = vault.set_lock_flag(LockFlag::VaultClosed, true).is_ok();
            let stopped = self.stop_holding_step().await;
            self.forget_session_as(SessionTransition::Lock, SessionMoved::Ended);
            if vault.lock_flag(LockFlag::VaultClosed) != Some(true) {
                survives_restart = vault.set_lock_flag(LockFlag::VaultClosed, true).is_ok();
            }
            Some(VaultLocked {
                stopped,
                survives_restart,
            })
        });
        if locked.is_some() {
            self.publish();
        }
        locked
    }

    /// How many session-ending steps were asked in this engine so far (a sign-out, a vault lock, the close of a
    /// session the server ended, leaving a half-done sign-in). Monotonic, in memory, never waits: a host that took a
    /// decision in the turn of a moment compares it with the count of that moment, and a step that is on its way to
    /// end that turn (still waiting for the step lock) has moved it already.
    pub fn session_steps_asked(&self) -> u64 {
        self.acct.session_steps_asked()
    }

    /// The step lock (`transition`) for a step that may END the session: the step is counted BEFORE the wait
    /// ([`Self::session_steps_asked`]), so a host that took a decision for the turn of now (About's update hand-off,
    /// asked again right before the installer starts) sees that a step which will end that turn is on its way,
    /// also while that step still waits behind another one (the hand-off's own stop of sync holds this lock for
    /// seconds). Counted: `lock_vault`, `close_session_the_server_refused` (once its first check found something to
    /// close), `sign_out_for`, `sign_out_keeping_local_data`, `discard_partial_sign_in`.
    ///
    /// Not counted, and why:
    /// - `restore`, `sign_in_password_step`, `two_factor_step`, `unlock_with_phrase_step`, `sign_in_passkey_step`,
    ///   `signup_create_step`: they run only while no session is open, and a press of the update needs an open lock;
    /// - `start_holding_step`, `add_to_file_explorer_again` and `stop` (the hand-off's own stop, and the quit's):
    ///   they end no session;
    /// - `change_password_step`: the person is there and their session goes on. Landed before the hand-off asks, its
    ///   turn move refuses the hand-off anyway; still queued, the install for that same person is fine (wave 2,
    ///   decision D-4: a choice, not an oversight).
    pub(super) async fn session_ending_step(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.acct.note_session_step_asked();
        self.transition.lock().await
    }

    /// Start the session watcher: ONE thread (`session-watch`) that asks [`Self::close_ended_session`] every
    /// [`SESSION_WATCH_EVERY`]. It holds only a `Weak` to this handle: it ends with it. A second call starts nothing.
    pub fn watch_session(self: &Arc<Self>) {
        self.watch_session_every_inner(SESSION_WATCH_EVERY);
    }

    /// [`Self::watch_session`] with its own period, for a test.
    #[cfg(any(test, feature = "test-support"))]
    pub fn watch_session_every(self: &Arc<Self>, every: Duration) {
        self.watch_session_every_inner(every);
    }

    fn watch_session_every_inner(self: &Arc<Self>, every: Duration) {
        if self.session_watch_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let engine = Arc::downgrade(self);
        let spawned = std::thread::Builder::new()
            .name("session-watch".to_string())
            .spawn(move || {
                loop {
                    let Some(engine) = engine.upgrade() else { return };
                    engine.close_ended_session();
                    // A session stored before the owner tag: its owner, once Beebeeb can say (at most once a minute,
                    // on a thread of its own).
                    engine.settle_session_owner_aside();
                    drop(engine);
                    std::thread::sleep(every);
                }
            });
        if let Err(error) = spawned {
            tracing::warn!("the session watcher did not start: {error}");
        }
    }

    /// The strong unlock: the account's password, proven to the server in a new sign-in for the account whose
    /// session is stored here (core's `sign_in_on_device` + `unlock_known_device`, in this engine's own steps:
    /// [`Engine::sign_in_password`]'s). `TwoFactor`: [`Engine::two_factor`] finishes it. `Drive`: the stored key is
    /// in memory under the new session, the mark is cleared, the replaced session is ended at the server. It opens a
    /// closed vault, and a locked screen too (a proof stronger than the screen lock; the engine restarts under the
    /// new session). Needs the network. Signed out, or no account named for the stored session: `Other`.
    pub fn unlock_vault_with_password(&self, password: Zeroizing<String>) -> Result<NextAfterSignIn, SignInProblem> {
        let email = match (self.lock_state(), self.account_email()) {
            (LockState::SignedOut, _) => None,
            (_, Some(email)) => Some(email),
            (_, None) => self.vault().account_email().ok().flatten(),
        };
        let result = match email {
            Some(email) => self.sign_in_password_step(&email, password, true),
            None => Err(SignInProblem::Other),
        };
        self.publish();
        result
    }

    /// A proven sign-in (`profile` is the server's word for its new session) is about to be stored, in the turn
    /// `turn`. No session is stored: a first sign-in, nothing to do. One is: this is the strong unlock, and
    ///
    /// - the proof must be for the account whose session is stored here (a recorded owner that differs refuses);
    /// - an app that holds the key (a locked screen) closes first, as core closes the vault when a sign-in's
    ///   credentials are set: the engine stops and the key leaves memory, so nothing keeps working under the
    ///   session that is about to be ended.
    ///
    /// `Ok`: the turn to store in, and the replaced session's token (to end it at the server afterwards).
    pub(super) async fn prepare_replacement(
        &self,
        turn: SessionGeneration,
        profile: &crate::account_dto::AccountProfile,
    ) -> Result<(SessionGeneration, Option<Zeroizing<String>>), String> {
        const REFUSED: &str = "the stored session is another account's, or it changed";
        let vault = self.vault();
        let Some(old) = vault.session_token().map_err(|_| REFUSED.to_string())? else {
            return Ok((turn, None));
        };
        let old = Zeroizing::new(old.expose_for_request().to_string());
        let owner = vault.session_owner().map_err(|_| REFUSED.to_string())?;
        if owner.is_some_and(|owner| owner.trim() != profile.user_id.trim()) || !self.acct.session_unchanged_since(turn)
        {
            return Err(REFUSED.to_string());
        }
        if !self.unlocked() {
            return Ok((turn, Some(old)));
        }
        // A proof that did not end in time does not refuse the strong unlock, unlike an engine task that may still run
        // (`unfinished_sign_out`): the old token is ended at the server afterwards, and the proof's seat refuses every
        // start until it let go.
        let _ = self.stop_proofs().await;
        if super::unfinished_sign_out::a_task_may_hold_the_keys(&self.acct, self.stop_holding_step().await) {
            return Err(crate::account_binding::ENGINE_STOP_UNCONFIRMED.to_string());
        }
        self.forget_session(SessionTransition::Lock);
        Ok((self.acct.session_generation(), Some(old)))
    }
}

/// A test's hold of the engine's step lock (`hold_session_step_for_test`).
#[cfg(any(test, feature = "test-support"))]
#[path = "step_hold.rs"]
pub mod step_hold;

#[cfg(test)]
#[path = "lock_api_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "session_steps_tests.rs"]
mod session_steps_tests;
