//! The away lock on the real engine: the shell's look ([`actions::Away`], measured by `shell/away.rs`) becomes the
//! engine's ([`Engine::away_for`]), which compares it with the stored "Locks again after" and locks through core's
//! `lock_after_away` (app lock off: nothing). The job is made on the window's thread, where the session it was
//! measured for is read, and runs on the shell's worker: the engine reads the credential store and takes the
//! lock's write hold.

use std::sync::Arc;

use beebeeb_engine::app::lock_api::LockState;
use beebeeb_engine::app::lock_password_api::{Away, AwayAnswer};
use beebeeb_engine::app::{CredentialStores, Engine};

use crate::shell::actions;

/// The job for one look at `seen`, for the session in memory now.
pub(super) fn job<C: CredentialStores + 'static>(engine: &Arc<Engine<C>>, seen: actions::Away) -> actions::AwayJob {
    let (engine, measured) = (engine.clone(), engine.session_turn());
    let seen = match seen {
        actions::Away::Idle(idle) => Away::Idle(idle),
        actions::Away::LeftTheApp => Away::LeftTheApp,
        actions::Away::SessionLocked => Away::SessionLocked,
    };
    Box::new(move || {
        let before = engine.lock_state();
        let AwayAnswer { state, check_again_in } = engine.away_for(measured, seen);
        if before == LockState::Open && state == LockState::ScreenLocked {
            // The Finder reconciler holds (cancels a running check), acknowledged before this returns; a no-op
            // where it does not run.
            super::finder_setup::lock();
        }
        actions::AwayAnswer {
            locked: before == LockState::Open && state == LockState::ScreenLocked,
            check_again_in,
        }
    })
}

/// Core's lock is open: the away lock counts idle time (`shell/away.rs`), whatever the window shows. The lock's state
/// is one short hold of core's state machine.
pub(super) fn screen_open<C: CredentialStores + 'static>(engine: &Engine<C>) -> bool {
    engine.lock_state() == LockState::Open
}
