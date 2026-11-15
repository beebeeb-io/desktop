//! About's updates: whether the person who pressed "Restart to update" is still the one in front of Beebeeb
//! ([`Presence`]). The press runs on by itself (download, check, hand-off), and the person may lock Beebeeb, sign out,
//! or walk away meanwhile: the hand-off only runs for the session the press was taken in, with its lock open.
//!
//! The mark is the engine's session turn (`Engine::session_turn`, read in the engine at this head): it moves on at a
//! screen lock (by hand or away), a vault lock, a session the server ended, a sign-out, and every sign-in write
//! (a password or passkey sign-in, the recovery words, the start's restore), and also at two writes of the person's
//! own session (a confirmed change of the sign-in email, a password change). A screen unlock with Windows Hello does
//! not move it. So any of these between the press and the hand-off makes the update wait, and the press is offered
//! again once the lock is open: for the two writes of the person's own, that is a wait they did not need, never an
//! install they did not ask for.

use std::sync::Mutex;

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::lock_api::LockState;

use super::*;

/// The session turn now (`None`: no session to speak of), and whether its lock is open.
type Now = Box<dyn Fn() -> (Option<SessionGeneration>, bool) + Send + Sync>;

/// What the press and the hand-off ask about the person and the session. One per updater: the part takes the press
/// with it, the worker asks it before the hand-off, and the hand-off asks it again right before the installer starts.
pub(crate) struct Presence {
    now: Now,
    /// A stop of sync was not confirmed in this process (any account's).
    unconfirmed: Box<dyn Fn() -> bool + Send + Sync>,
    /// The turn the last press was taken in.
    pressed: Mutex<Option<SessionGeneration>>,
}

impl Presence {
    /// On this app's engine.
    pub(crate) fn of_engine<C: CredentialStores + 'static>(engine: Arc<Engine<C>>) -> Arc<Self> {
        let stops = engine.clone();
        Arc::new(Self {
            now: Box::new(move || (Some(engine.session_turn()), engine.lock_state() == LockState::Open)),
            unconfirmed: Box::new(move || any_stop_unconfirmed(&stops)),
            pressed: Mutex::new(None),
        })
    }

    /// No session, no press: the part of a build whose updater did not start (nothing is pressed there).
    pub(crate) fn closed() -> Arc<Self> {
        Arc::new(Self {
            now: Box::new(|| (None, false)),
            unconfirmed: Box::new(|| false),
            pressed: Mutex::new(None),
        })
    }

    /// A test's: the lock open in one turn, and no stop left unconfirmed.
    #[cfg(test)]
    pub(crate) fn open() -> Arc<Self> {
        Self::from_fns(|| (Some(SessionGeneration::for_test(1)), true), || false)
    }

    #[cfg(test)]
    pub(crate) fn from_fns(
        now: impl Fn() -> (Option<SessionGeneration>, bool) + Send + Sync + 'static,
        unconfirmed: impl Fn() -> bool + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            now: Box::new(now),
            unconfirmed: Box::new(unconfirmed),
            pressed: Mutex::new(None),
        })
    }

    /// The lock is open now: a press may be taken (and is offered).
    pub(crate) fn lock_open(&self) -> bool {
        (self.now)().1
    }

    /// Takes the press in the turn of now, when the lock is open. `false`: no press now.
    pub(crate) fn press(&self) -> bool {
        let (turn, open) = (self.now)();
        let taken = if open { turn } else { None };
        *self.pressed.lock().unwrap_or_else(|p| p.into_inner()) = taken;
        taken.is_some()
    }

    /// The session is still the one the press was taken in, and its lock is open: the hand-off may run.
    pub(crate) fn holds(&self) -> bool {
        let (turn, open) = (self.now)();
        let pressed = *self.pressed.lock().unwrap_or_else(|p| p.into_inner());
        open && pressed.is_some() && pressed == turn
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
    engine.accounts().accounts.lock().map_or(true, |accounts| {
        accounts
            .iter()
            .any(|account| account.engine_stop_unconfirmed.load(Ordering::SeqCst))
    })
}
