//! The two-level lock of this app, bound to core's rules (decision 2495; core's contract
//! `docs/superpowers/specs/2026-11-13-two-level-lock-design.md`).
//!
//! The rules are core's and are not restated here: the state machine is `beebeeb_api::flows::LockFlow` (one per
//! handle, every move of the lock is one of its steps), what a locked screen refuses is `flows::call_may_run`, and
//! how the app starts is `LockFlow::at_start`. What this file adds is the handle's side of core's lock service
//! (`beebeeb-client`'s `auth/lock.rs`, which this engine cannot call: it keeps its own session and credential
//! store), step for step in that service's order:
//!
//! - the two flags a start reads are stored beside the session under core's entry names, as `1` / `0`; a flag that
//!   cannot be read starts the vault closed, and a start that is closed writes the mark;
//! - a vault lock writes the mark BEFORE the key leaves memory, and reads it back after;
//! - only a strong proof (the password accepted by the server, or the twelve words) clears the mark;
//! - while the vault is marked closed the key entry is not read at all, not even to ask whether it is there.
//!
//! Neither level encrypts anything on this computer: the key stays in the credential store. The screen lock gates
//! the window (sync keeps running); the vault lock takes the key out of this process and stops the work.
//!
//! Must not: open any level on a proof that did not complete; hold a state of its own beside the flow.

pub use beebeeb_api::flows::{Call, CallRefused, LockState, LockedAction};
use beebeeb_api::flows::{FlowError, LockFlow, LockLevel, StartFacts, call_may_run, key_may_be_loaded};
use zeroize::Zeroizing;

use super::{CredentialStores, Engine, SyncStop};
use crate::account::{SessionGeneration, SessionTransition};
use crate::keychain::{AuthVault, LockFlag};
use crate::signin::gate::{NextAfterSignIn, SignInProblem};

/// Core's switch for the password path of the lock (`ClientConfig::lock_password_path`): the offline password
/// check of the screen lock and the start setting. Off until the server's half is deployed; while it is off both
/// answer [`LockRefused::PasswordCheckUnavailable`], as core does.
pub const LOCK_PASSWORD_PATH: bool = false;

/// What a handle knows before its start read anything: no session.
pub(super) const NO_SESSION: StartFacts = StartFacts {
    session_stored: false,
    ask_password_at_start: None,
    vault_closed: None,
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
    /// The password path is off in this build ([`LOCK_PASSWORD_PATH`]).
    PasswordCheckUnavailable,
}

fn refused(error: FlowError) -> LockRefused {
    match error {
        FlowError::StepNotRequired => LockRefused::NotAtThisLevel,
        FlowError::StepOutOfOrder | FlowError::NotReady => LockRefused::NeedsThePassword,
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
}

impl<C: CredentialStores> Engine<C> {
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

    /// A person's read or change (anything but unlock, lock vault and sign out) is refused by the lock now.
    pub(super) fn screen_refuses_a_person(&self) -> bool {
        self.check_call(Call::Person(LockedAction::ViewStatus)) == Err(CallRefused::ScreenLocked)
    }

    /// The start rule, for a start that found no key in memory. A closed start writes the mark, so every later
    /// start stays closed until the strong proof. The flow is not installed yet: [`Self::lock_started`] does that
    /// once the start knows whether a key was loaded.
    pub(super) fn lock_start_rule(&self, vault: &AuthVault<C::Store>, session_stored: bool) -> LockFlow {
        let facts = StartFacts {
            session_stored,
            ask_password_at_start: vault.lock_flag(LockFlag::AskPasswordAtStart),
            vault_closed: vault.lock_flag(LockFlag::VaultClosed),
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
        }
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

    /// The screen lock's password path. Off in this build ([`LOCK_PASSWORD_PATH`]): nothing is checked, nothing is
    /// counted, and the typed password is wiped.
    pub fn unlock_screen_with_password(&self, password: Zeroizing<String>) -> Result<(), LockRefused> {
        drop(password);
        Err(LockRefused::PasswordCheckUnavailable)
    }

    /// "Ask for my password when Beebeeb starts". `None`: the setting could not be read (the next start is closed).
    pub fn ask_password_at_start(&self) -> Option<bool> {
        self.vault().lock_flag(LockFlag::AskPasswordAtStart)
    }

    /// Change the start setting. Off in this build, as in core while the password path is off: a setting that
    /// could be turned on and never off again (turning it off needs the password proven) would trap a person.
    pub fn set_ask_password_at_start(&self, _on: bool) -> Result<(), LockRefused> {
        Err(LockRefused::PasswordCheckUnavailable)
    }

    /// Close the vault. In core's order: the closed mark is written; the engine stops and the key leaves this
    /// process (the session's turn moves on, so every step that began before is refused at its write); the mark is
    /// read back and written again when a strong proof that finished just before cleared it. The stored session and
    /// key stay in the credential store. Signed out: the stop alone, and nothing is marked.
    pub fn lock_vault(&self) -> VaultLocked {
        let locked = self.runtime.block_on(async {
            let _step = self.transition.lock().await;
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
        if self.stop_holding_step().await == SyncStop::Unconfirmed {
            return Err(crate::account_binding::ENGINE_STOP_UNCONFIRMED.to_string());
        }
        self.forget_session(SessionTransition::Lock);
        Ok((self.acct.session_generation(), Some(old)))
    }
}
