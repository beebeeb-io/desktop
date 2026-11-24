//! About's updates: whether the person who pressed "Restart to update" is still the one in front of Beebeeb
//! ([`Presence`]). The press runs on by itself (download, check, hand-off), and the person may lock Beebeeb, sign out,
//! or walk away meanwhile: the hand-off only runs for the session the press was taken in, with its lock open.
//!
//! The mark is the engine's session turn (`Engine::session_turn`, read in the engine at this head): it moves on at a
//! screen lock (by hand or away), a vault lock, a session the server ended, a sign-out, and every sign-in write
//! (a password or passkey sign-in, the the 12 words of your master key, the start's restore), and also at two writes of the person's
//! own session (a confirmed change of the sign-in email, a password change). A screen unlock with Windows Hello does
//! not move it. So any of these between the press and the hand-off makes the update wait, and the press is offered
//! again once the lock is open: for the two writes of the person's own, that is a wait they did not need, never an
//! install they did not ask for.
//!
//! The turn alone misses a step that is ASKED but not done: a sign-out, a vault lock, or the close of a session the
//! server ended that waits behind the hand-off's own stop of sync (the engine's step lock, held for seconds while a
//! running sync stops) moves the turn only once it runs, after the hand-off asked. So the press also takes the
//! engine's count of session-ending steps asked (`Engine::session_steps_asked`, counted before such a step waits),
//! and the hand-off runs only while both are as they were. A password change queued the same way is not counted: the
//! person is there and their session goes on (wave 2, D-4).

use std::sync::Mutex;

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::lock_api::LockState;

use super::*;

/// The session turn now (`None`: no session to speak of), whether its lock is open, and how many session-ending steps
/// were asked so far.
type Now = Box<dyn Fn() -> (Option<SessionGeneration>, bool, u64) + Send + Sync>;

/// What a press was taken in: the session turn, and the count of session-ending steps asked at that moment.
#[derive(Clone, Copy)]
struct Pressed {
    turn: SessionGeneration,
    steps: u64,
}

/// What the press and the hand-off ask about the person and the session. One per updater: the part takes the press
/// with it, the worker asks it before the hand-off, and the hand-off asks it again right before the installer starts.
pub(crate) struct Presence {
    now: Now,
    /// A stop of sync was not confirmed in this process (any account's).
    unconfirmed: Box<dyn Fn() -> bool + Send + Sync>,
    /// The turn and the count the last press was taken in.
    pressed: Mutex<Option<Pressed>>,
    /// The last press's hand-off was refused right before the installer, because the person or the session was no
    /// longer the one of the press: the row says the update waits for them. A new press clears it.
    refused_at_launch: AtomicBool,
}

impl Presence {
    /// On this app's engine.
    pub(crate) fn of_engine<C: CredentialStores + 'static>(engine: Arc<Engine<C>>) -> Arc<Self> {
        let stops = engine.clone();
        Arc::new(Self {
            now: Box::new(move || {
                (
                    Some(engine.session_turn()),
                    engine.lock_state() == LockState::Open,
                    engine.session_steps_asked(),
                )
            }),
            unconfirmed: Box::new(move || any_stop_unconfirmed(&stops)),
            pressed: Mutex::new(None),
            refused_at_launch: AtomicBool::new(false),
        })
    }

    /// No session, no press: the part of a build whose updater did not start (nothing is pressed there).
    pub(crate) fn closed() -> Arc<Self> {
        Arc::new(Self {
            now: Box::new(|| (None, false, 0)),
            unconfirmed: Box::new(|| false),
            pressed: Mutex::new(None),
            refused_at_launch: AtomicBool::new(false),
        })
    }

    /// A test's: the lock open in one turn, no session step asked, and no stop left unconfirmed.
    #[cfg(test)]
    pub(crate) fn open() -> Arc<Self> {
        Self::from_fns(|| (Some(SessionGeneration::for_test(1)), true), || 0, || false)
    }

    /// A test's: the turn and the lock (`now`), the count of session-ending steps asked (`steps`), and whether a
    /// stop was left unconfirmed, each from its own function.
    #[cfg(test)]
    pub(crate) fn from_fns(
        now: impl Fn() -> (Option<SessionGeneration>, bool) + Send + Sync + 'static,
        steps: impl Fn() -> u64 + Send + Sync + 'static,
        unconfirmed: impl Fn() -> bool + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            now: Box::new(move || {
                let (turn, open) = now();
                (turn, open, steps())
            }),
            unconfirmed: Box::new(unconfirmed),
            pressed: Mutex::new(None),
            refused_at_launch: AtomicBool::new(false),
        })
    }

    /// The lock is open now: a press may be taken (and is offered).
    pub(crate) fn lock_open(&self) -> bool {
        (self.now)().1
    }

    /// Takes the press in the turn and the count of now, when the lock is open. `false`: no press now. A press taken
    /// starts a new order: the refusal of the last one is no longer what the row says.
    pub(crate) fn press(&self) -> bool {
        let (turn, open, steps) = (self.now)();
        let taken = turn.filter(|_| open).map(|turn| Pressed { turn, steps });
        *self.pressed.lock().unwrap_or_else(|p| p.into_inner()) = taken;
        if taken.is_some() {
            self.refused_at_launch.store(false, Ordering::SeqCst);
        }
        taken.is_some()
    }

    /// The session is still the one the press was taken in, its lock is open, and no session-ending step was asked
    /// since (one may still wait for the engine's step lock): the hand-off may run.
    pub(crate) fn holds(&self) -> bool {
        let (turn, open, steps) = (self.now)();
        let pressed = *self.pressed.lock().unwrap_or_else(|p| p.into_inner());
        open && pressed.is_some_and(|pressed| Some(pressed.turn) == turn && pressed.steps == steps)
    }

    /// The hand-off was refused right before the installer ([`Self::holds`] said no): the row says the update waits
    /// for the person, and offers the press.
    pub(crate) fn note_refused_at_launch(&self) {
        self.refused_at_launch.store(true, Ordering::SeqCst);
    }

    /// The last press's hand-off was refused right before the installer, and no press was taken since.
    pub(crate) fn waited(&self) -> bool {
        self.refused_at_launch.load(Ordering::SeqCst)
    }

    /// A stop of sync was not confirmed in this process: no installer can start until Beebeeb restarts.
    pub(crate) fn stop_unconfirmed(&self) -> bool {
        (self.unconfirmed)()
    }
}

/// A stop of sync was not confirmed in this process, by any path (a hand-off's, a vault lock's, a sign-out's:
/// `Engine::stop` and the lock's stop set the account's `engine_stop_unconfirmed`), for any account this engine
/// knows. Its engine slot is empty since, so a stop now would say `WasNotRunning` about a task that may still run. A
/// registry that cannot be read counts as such a stop: no proof without knowing.
pub(crate) fn any_stop_unconfirmed<C: CredentialStores>(engine: &Engine<C>) -> bool {
    let recorded = engine.accounts().accounts.lock().map_or(true, |accounts| {
        accounts
            .iter()
            .any(|account| account.engine_stop_unconfirmed.load(Ordering::SeqCst))
    });
    unconfirmed_by(recorded, callbacks_not_let_go())
}

/// A stop is unconfirmed when an account's record says so, or (Windows) a stop's task ended but File Explorer's
/// callbacks were not revoked (`AbortOutcome::RevokeFailed`: no account record, only the engine's start fence). The
/// installer must not start over callbacks that may still be registered: the hand-off waits for a restart, as 0.8.x's
/// did. Nothing holds the keys then, so this widens no access; it only keeps the installer from starting.
fn unconfirmed_by(recorded: bool, callbacks_not_let_go: bool) -> bool {
    recorded || callbacks_not_let_go
}

/// Windows: the engine's start fence is up (a revoke of the callbacks failed or never confirmed). Elsewhere never.
fn callbacks_not_let_go() -> bool {
    #[cfg(target_os = "windows")]
    {
        beebeeb_engine::windows_cf::ensure_reactivation_allowed().is_err()
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::unconfirmed_by;

    /// After an earlier `RevokeFailed` there is no account record, yet the hand-off still waits (0.8.x did).
    #[test]
    fn a_revoke_failure_without_a_record_still_makes_the_hand_off_wait() {
        assert!(unconfirmed_by(false, true));
        assert!(unconfirmed_by(true, false));
        assert!(!unconfirmed_by(false, false));
    }
}
