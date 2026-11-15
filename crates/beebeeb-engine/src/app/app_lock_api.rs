//! App lock, the screen level's per-device switch (core #117; the owner's ruling of 2026-11-14: off, the app never
//! locks by itself and opens directly for anyone signed in to this PC; on, the default, it opens screen-locked).
//!
//! As its parent (`lock_password_api.rs`, DECISION 2509) copies the order of core's password path, this file copies
//! the ORDER of core's `Client::app_lock`, `Client::set_app_lock` and `Client::lock_after_away`
//! (`beebeeb-client/src/auth/lock.rs`, `lock_password.rs`), because this engine cannot call that client. The rules
//! are core's and are called, not restated: `flows::app_lock_may_change` (only an open app changes it; turning it off
//! needs a proof the screen unlock accepts, given for this change), `flows::away_locks_screen` and
//! `LockFlow::lock_after_away` (what the automatic lock does), and `LockFlow::at_start` with `StartFacts::app_lock`
//! (how a start reads it: `lock_api::lock_start_rule`).
//!
//! Stored as core's entry `lock.app_lock` beside the session (`1` / `0`; absent, or unreadable, is ON), removed first
//! at a sign-out and at an account's first sign-in (`keychain::LockFlag::AppLock`). Not a secret.
//!
//! A change that checks a password BLOCKS on the handle's runtime: call it from a worker, never the UI thread.
//!
//! Must not: write "off" without a proof given for this change, or keep a proof for later; write after the session
//! moved on or the screen locked while the proof was checked; log the typed password.

use beebeeb_api::flows::{FlowError, LockState, ScreenProof, app_lock_may_change, away_locks_screen};
use zeroize::Zeroizing;

use super::super::{CredentialStores, Engine};
use super::{Judgement, LOCAL_CHECK_OFF_AT, now_ms, wait_after_ms};
use crate::account::{SessionGeneration, SessionTransition};
use crate::keychain::LockFlag;

/// The proof for turning app lock off (core's `AppLockProof`): given by the person for this change, right before it,
/// and used once. The unlock that opened the app earlier is not one.
pub enum AppLockProof {
    /// Windows Hello returned success for the person at the keyboard just now: the host's statement, trusted as
    /// [`Engine::unlock_screen_platform_verified`] trusts it. A cancelled or unavailable prompt gives no proof.
    PlatformVerified,
    /// The account password, typed for this change. Checked as the screen unlock checks it (online, or the local
    /// check when the server cannot be reached) and counted as its tries are.
    Password(Zeroizing<String>),
}

impl std::fmt::Debug for AppLockProof {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::PlatformVerified => "PlatformVerified",
            Self::Password(_) => "Password(..)",
        })
    }
}

/// Why app lock was not changed. Nothing was written for any of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppLockRefused {
    /// The app is not open (core's `NotReady`): only an open app changes the setting.
    NotOpen,
    /// Turning it off needs a proof the screen unlock accepts, given for this change (core's `StepOutOfOrder`). An
    /// empty password is none.
    ProofNeeded,
    /// The session moved on (a lock, a sign-out, another sign-in) since the press this answers.
    Overtaken,
    /// A wrong password, counted (core's `AuthError::PasswordWrong`): `wait_ms` before the next try (0: none), and
    /// whether this tenth wrong try turned the offline check off.
    Wrong { wait_ms: u64, local_check_off: bool },
    /// Nothing was checked: a wait after earlier wrong tries is still running.
    Wait { wait_ms: u64 },
    /// The server's own limit on password checks. `None`: it named no wait.
    ServerLimit { wait_ms: Option<u64> },
    /// The server cannot be reached and this PC has no usable record of the password.
    Offline,
    /// The account is still on the legacy key stretch: signing in again finishes its upgrade.
    UpgradeNeeded,
    /// The server no longer accepts the session: the vault is closed.
    SessionEnded,
    /// A password check is already on its way, or the check could not run (a count that cannot be stored, an unknown
    /// owner).
    CouldNotCheck,
    /// The setting could not be stored.
    NotSaved,
}

fn screen_proof(proof: &AppLockProof) -> ScreenProof {
    match proof {
        AppLockProof::PlatformVerified => ScreenProof::PlatformVerified,
        AppLockProof::Password(_) => ScreenProof::PasswordChecked,
    }
}

impl<C: CredentialStores> Engine<C> {
    /// The app lock switch of this PC for the session `session` (core's `Client::app_lock`): on unless it was read
    /// as off. `Err(Overtaken)`: the session moved on, and the answer is not for the window that asked.
    pub fn app_lock_for(&self, session: SessionGeneration) -> Result<bool, AppLockRefused> {
        // RED: today's behaviour, always on.
        let on = true;
        if !self.acct.session_unchanged_since(session) {
            return Err(AppLockRefused::Overtaken);
        }
        Ok(on)
    }

    /// Set app lock for the session `session` (core's `Client::set_app_lock`). Turning it on needs nothing; turning
    /// it off needs `proof`. Blocks while a password is checked.
    pub fn set_app_lock_for(
        &self,
        session: SessionGeneration,
        on: bool,
        proof: Option<AppLockProof>,
    ) -> Result<(), AppLockRefused> {
        self.set_app_lock_for_at(session, on, proof, now_ms())
    }

    /// [`Self::set_app_lock_for`] on the caller's clock (milliseconds since the epoch), for a test.
    pub(crate) fn set_app_lock_for_at(
        &self,
        session: SessionGeneration,
        on: bool,
        proof: Option<AppLockProof>,
        now_ms: u64,
    ) -> Result<(), AppLockRefused> {
        if true {
            // RED: today's behaviour, nothing is stored.
            return Err(AppLockRefused::NotSaved);
        }
        let given = proof.as_ref().map(screen_proof);
        self.app_lock_may_change_now(on, given)?;
        if let (false, Some(AppLockProof::Password(password))) = (on, proof) {
            if password.is_empty() {
                return Err(AppLockRefused::ProofNeeded);
            }
            let judged = self
                .runtime
                .block_on(self.counted_password_check(&password, now_ms, true))
                .map_err(|_| AppLockRefused::CouldNotCheck)?;
            drop(password);
            match judged {
                Judgement::Accepted => {}
                Judgement::Wrong { failures } => {
                    let local_check_off = failures == LOCAL_CHECK_OFF_AT;
                    return Err(AppLockRefused::Wrong {
                        wait_ms: if local_check_off { 0 } else { wait_after_ms(failures) },
                        local_check_off,
                    });
                }
                Judgement::Wait { wait_ms } => return Err(AppLockRefused::Wait { wait_ms }),
                Judgement::ServerLimit { wait_ms } => return Err(AppLockRefused::ServerLimit { wait_ms }),
                Judgement::OfflineNoRecord => return Err(AppLockRefused::Offline),
                Judgement::UpgradeNeeded => return Err(AppLockRefused::UpgradeNeeded),
                Judgement::SessionEnded => {
                    self.end_session_the_server_refused(session);
                    return Err(AppLockRefused::SessionEnded);
                }
            }
        }
        // The write, in the hold every lock move waits for: a lock, a sign-out or another session that landed while
        // the proof was checked (or the prompt was up) writes nothing.
        let _writes = self.writes.read().unwrap_or_else(|p| p.into_inner());
        if !self.acct.session_unchanged_since(session) {
            return Err(AppLockRefused::Overtaken);
        }
        self.app_lock_may_change_now(on, given)?;
        self.vault()
            .set_lock_flag(LockFlag::AppLock, on)
            .map_err(|_| AppLockRefused::NotSaved)
    }

    /// The automatic lock (core's `Client::lock_after_away`): the host says the person was away for the delay they
    /// chose, or the operating system's session was locked. With app lock on (or unreadable) an open app becomes
    /// screen-locked, as [`Engine::lock_screen`] makes it, and every answer asked for before is for another turn; with
    /// app lock off nothing changes. From any other state nothing changes either, so a host may call it whenever its
    /// timer fires. The answer is the state after. "Lock now" by hand is [`Engine::lock_screen`].
    pub fn lock_after_away(&self) -> LockState {
        let app_lock = self.vault().lock_setting(LockFlag::AppLock, true);
        if true {
            // RED: today's behaviour, no automatic lock.
            return self.lock_state();
        }
        let locked = {
            let _writes = self.writes.write().unwrap_or_else(|p| p.into_inner());
            let mut flow = self.lock_flow.lock().unwrap_or_else(|p| p.into_inner());
            let before = flow.state();
            flow.lock_after_away(app_lock);
            let locked = flow.state() != before;
            drop(flow);
            if locked {
                self.acct
                    .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
            }
            locked
        };
        if locked {
            self.publish();
        }
        self.lock_state()
    }

    fn app_lock_may_change_now(&self, on: bool, proof: Option<ScreenProof>) -> Result<(), AppLockRefused> {
        app_lock_may_change(self.lock_state(), on, proof).map_err(|error| match error {
            FlowError::StepOutOfOrder => AppLockRefused::ProofNeeded,
            FlowError::StepNotRequired | FlowError::NotReady => AppLockRefused::NotOpen,
        })
    }
}

#[cfg(test)]
#[path = "app_lock_api_tests.rs"]
mod tests;
