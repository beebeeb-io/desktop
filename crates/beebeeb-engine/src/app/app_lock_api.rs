//! App lock, the screen level's per-device switch (core #117: off, the app never
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
//! "Locks again after" (decision 2519): core holds no clock and no delay ("the host says when"), so the delay the
//! person chose is this engine's entry `lock.after` beside it ([`LockAfter`]; absent is five minutes, unreadable or
//! not one of the choices is the strictest; removed with the other lock entries), used only while app lock is on. The host measures and calls [`Engine::away_for`]; the
//! comparison with the delay is here, the lock is core's `lock_after_away`. Lengthening it is a weaker lock and asks
//! core's rule for a weaker app lock: a proof given for that change.
//!
//! A change that checks a password BLOCKS on the handle's runtime: call it from a worker, never the UI thread.
//!
//! Must not: write "off" or a longer delay without a proof given for this change, or keep a proof for later; write
//! after the session moved on or the screen locked while the proof was checked; log the typed password; lock from
//! an idle time measured in an earlier turn of the session.

use std::time::Duration;

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

/// Delay writes, one at a time (`set_lock_after_for_at`). Taken before the session's write lock's read side, never
/// while holding it.
static LOCK_AFTER_WRITES: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// "Locks again after" (DW-S03, DW-0202): how long the person may be away before an open app locks its screen by
/// itself, while app lock is on (decision 2519; app lock off never locks). The
/// menu's order, strictest first: a later choice is a longer delay, a weaker lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LockAfter {
    /// As soon as the person leaves Beebeeb (its window closed to the tray), and after [`LockAfter::SHORTEST_AWAY`]
    /// without input on this PC while its window stays: no time at all cannot be told from a person reading.
    Immediately,
    Minutes5,
    Minutes30,
    Minutes60,
}

impl LockAfter {
    /// While nothing is stored (decision 2519).
    pub const DEFAULT: Self = Self::Minutes5;
    /// For an entry that is not one of the choices, or that cannot be read: the strictest, so a
    /// stored "Immediately" that reads back as anything else never becomes a longer delay, and no change from it skips
    /// the proof a longer one needs.
    pub const STRICTEST: Self = Self::Immediately;
    /// The menu's choices, strictest first.
    pub const ALL: [Self; 4] = [Self::Immediately, Self::Minutes5, Self::Minutes30, Self::Minutes60];
    /// The shortest time without input this app takes as "away" (what [`LockAfter::Immediately`] waits for while the
    /// window stays).
    pub const SHORTEST_AWAY: Duration = Duration::from_secs(60);

    /// How long without input on this PC locks.
    pub const fn idle(self) -> Duration {
        match self {
            Self::Immediately => Self::SHORTEST_AWAY,
            Self::Minutes5 => Duration::from_secs(300),
            Self::Minutes30 => Duration::from_secs(1_800),
            Self::Minutes60 => Duration::from_secs(3_600),
        }
    }

    /// Whether leaving Beebeeb (its window closed to the tray) locks at once.
    pub const fn locks_on_leaving(self) -> bool {
        matches!(self, Self::Immediately)
    }

    /// The stored bytes: the delay in whole seconds, as text.
    fn to_bytes(self) -> Vec<u8> {
        let seconds: u32 = match self {
            Self::Immediately => 0,
            Self::Minutes5 => 300,
            Self::Minutes30 => 1_800,
            Self::Minutes60 => 3_600,
        };
        seconds.to_string().into_bytes()
    }

    fn from_bytes(bytes: &[u8]) -> Option<Self> {
        Self::ALL.into_iter().find(|after| after.to_bytes() == bytes)
    }
}

/// What the host saw, for the automatic lock. Core holds no clock: the host says when and core says what; this
/// engine compares the host's time with the delay the person chose (the host holds no rule).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Away {
    /// No input on this PC for this long, counted by the host from no earlier than the screen last opened.
    Idle(Duration),
    /// The person left Beebeeb: its window closed to the tray.
    LeftTheApp,
    /// The operating system's session was locked (Windows: Win+L, or its own lock).
    SessionLocked,
}

/// The automatic lock's answer to a host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AwayAnswer {
    /// The lock's state after.
    pub state: LockState,
    /// For [`Away::Idle`]: how long before an idle time could lock, so when to ask again (`Some(ZERO)`: the time was
    /// measured for an earlier turn of the session; measure again). `None`: nothing the host says could lock now
    /// (app lock off, the app not open, or it just locked), and for an event that did not lock.
    pub check_again_in: Option<Duration>,
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
        let on = away_locks_screen(self.vault().lock_setting(LockFlag::AppLock, true));
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
        if !self.acct.session_unchanged_since(session) {
            return Err(AppLockRefused::Overtaken);
        }
        let given = proof.as_ref().map(screen_proof);
        self.app_lock_may_change_now(on, given)?;
        if let (false, Some(AppLockProof::Password(password))) = (on, proof) {
            self.password_proof_checked(session, password, now_ms)?;
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
        self.lock_after_away_of(None)
    }

    /// [`Self::lock_after_away`]; with `measured_for`, only while the session is still that turn (checked in the
    /// write hold, where every other lock move waits).
    fn lock_after_away_of(&self, measured_for: Option<SessionGeneration>) -> LockState {
        let app_lock = self.vault().lock_setting(LockFlag::AppLock, true);
        if !away_locks_screen(app_lock) {
            return self.lock_state();
        }
        let locked = {
            let _writes = self.writes.write().unwrap_or_else(|p| p.into_inner());
            if measured_for.is_some_and(|turn| !self.acct.session_unchanged_since(turn)) {
                return self.lock_state();
            }
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

    /// The stored delay of this PC for the session `session`. `Ok(None)`: an entry is there that is not one of the
    /// choices (another build's value): the away lock uses [`LockAfter::DEFAULT`] and a window says it is not known.
    pub fn lock_after_for(&self, session: SessionGeneration) -> Result<Option<LockAfter>, AppLockRefused> {
        let stored = self.stored_lock_after();
        if !self.acct.session_unchanged_since(session) {
            return Err(AppLockRefused::Overtaken);
        }
        Ok(stored)
    }

    /// Set the delay for the session `session`. The same or a shorter delay needs nothing. A longer one lowers what
    /// the screen lock protects, so it needs `proof` given for this change, as turning app lock off does: core's
    /// rule for a weaker app lock (`flows::app_lock_may_change`, only an open app changes it) is asked for it.
    /// Blocks while a password is checked.
    pub fn set_lock_after_for(
        &self,
        session: SessionGeneration,
        after: LockAfter,
        proof: Option<AppLockProof>,
    ) -> Result<(), AppLockRefused> {
        self.set_lock_after_for_at(session, after, proof, now_ms())
    }

    /// [`Self::set_lock_after_for`] on the caller's clock, for a test.
    pub(crate) fn set_lock_after_for_at(
        &self,
        session: SessionGeneration,
        after: LockAfter,
        proof: Option<AppLockProof>,
        now_ms: u64,
    ) -> Result<(), AppLockRefused> {
        if !self.acct.session_unchanged_since(session) {
            return Err(AppLockRefused::Overtaken);
        }
        let given = proof.as_ref().map(screen_proof);
        let longer = after > self.lock_after_in_force();
        self.app_lock_may_change_now(!longer, given)?;
        if let (true, Some(AppLockProof::Password(password))) = (longer, proof) {
            self.password_proof_checked(session, password, now_ms)?;
        }
        // One delay write at a time: the read of what is stored and the write below are one
        // step, so a write judged against an older value never lands after a newer one.
        let _one_at_a_time = LOCK_AFTER_WRITES.lock().unwrap_or_else(|p| p.into_inner());
        let _writes = self.writes.read().unwrap_or_else(|p| p.into_inner());
        if !self.acct.session_unchanged_since(session) {
            return Err(AppLockRefused::Overtaken);
        }
        // What is stored may have moved while the proof was checked: the rule again, against it. A change that has
        // become longer since had no proof checked for it (a password is checked only for a longer delay).
        let longer_now = after > self.lock_after_in_force();
        if longer_now && !longer {
            return Err(AppLockRefused::ProofNeeded);
        }
        self.app_lock_may_change_now(!longer_now, given)?;
        self.vault()
            .set_lock_entry(LockFlag::LockAfter, &after.to_bytes())
            .map_err(|_| AppLockRefused::NotSaved)
    }

    /// The host's idle and OS-lock hooks (core's `Client::lock_after_away` is the lock; this is when). `session`:
    /// the turn the host measured in. With app lock on and the app open: [`Away::SessionLocked`] locks;
    /// [`Away::LeftTheApp`] locks with [`LockAfter::Immediately`]; [`Away::Idle`] locks once it reaches the stored
    /// delay's [`LockAfter::idle`], and otherwise says how long is left (an idle time of an earlier turn locks
    /// nothing: measure again). App lock off, or any state but open: nothing, ever.
    pub fn away_for(&self, session: SessionGeneration, seen: Away) -> AwayAnswer {
        let state = self.lock_state();
        let nothing = AwayAnswer {
            state,
            check_again_in: None,
        };
        if state != LockState::Open || !away_locks_screen(self.vault().lock_setting(LockFlag::AppLock, true)) {
            return nothing;
        }
        let after = self.lock_after_in_force();
        let measured_for = match seen {
            Away::SessionLocked => None,
            Away::LeftTheApp if after.locks_on_leaving() => None,
            Away::LeftTheApp => return nothing,
            Away::Idle(_) if !self.acct.session_unchanged_since(session) => {
                return AwayAnswer {
                    state,
                    check_again_in: Some(Duration::ZERO),
                };
            }
            Away::Idle(idle) if idle < after.idle() => {
                return AwayAnswer {
                    state,
                    check_again_in: Some(after.idle() - idle),
                };
            }
            Away::Idle(_) => Some(session),
        };
        AwayAnswer {
            state: self.lock_after_away_of(measured_for),
            check_again_in: None,
        }
    }

    /// The stored delay: absent is the default; `None`: an entry that is not one of the choices, or a store that
    /// could not be read.
    fn stored_lock_after(&self) -> Option<LockAfter> {
        match self.vault().lock_entry(LockFlag::LockAfter) {
            Ok(None) => Some(LockAfter::DEFAULT),
            Ok(Some(bytes)) => LockAfter::from_bytes(&bytes),
            Err(_) => None,
        }
    }

    /// The delay the away lock uses, and the base a change is weighed against: the stored one (absent: the default),
    /// else the strictest.
    fn lock_after_in_force(&self) -> LockAfter {
        self.stored_lock_after().unwrap_or(LockAfter::STRICTEST)
    }

    /// The typed password given as the proof for a change, checked as the screen unlock checks it (online, or the
    /// local check when the server cannot be reached) and counted as its tries are. An empty one is none. BLOCKS.
    fn password_proof_checked(
        &self,
        session: SessionGeneration,
        password: Zeroizing<String>,
        now_ms: u64,
    ) -> Result<(), AppLockRefused> {
        if password.is_empty() {
            return Err(AppLockRefused::ProofNeeded);
        }
        let judged = self
            .runtime
            .block_on(self.counted_password_check(&password, now_ms, true))
            .map_err(|_| AppLockRefused::CouldNotCheck)?;
        drop(password);
        match judged {
            Judgement::Accepted => Ok(()),
            Judgement::Wrong { failures } => {
                let local_check_off = failures == LOCAL_CHECK_OFF_AT;
                Err(AppLockRefused::Wrong {
                    wait_ms: if local_check_off { 0 } else { wait_after_ms(failures) },
                    local_check_off,
                })
            }
            Judgement::Wait { wait_ms } => Err(AppLockRefused::Wait { wait_ms }),
            Judgement::ServerLimit { wait_ms } => Err(AppLockRefused::ServerLimit { wait_ms }),
            Judgement::OfflineNoRecord => Err(AppLockRefused::Offline),
            Judgement::UpgradeNeeded => Err(AppLockRefused::UpgradeNeeded),
            Judgement::SessionEnded => {
                self.end_session_the_server_refused(session);
                Err(AppLockRefused::SessionEnded)
            }
        }
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

#[cfg(test)]
#[path = "app_lock_after_tests.rs"]
mod after_tests;
