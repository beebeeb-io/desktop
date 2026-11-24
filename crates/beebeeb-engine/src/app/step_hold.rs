//! A test's hold of the engine's step lock (`transition`, private to `app`): a test that must see what a step does
//! while it WAITS for that lock (a session-ending step counts before it waits: `Engine::session_steps_asked`) holds
//! the lock here, starts the step on a thread of its own, and lets go by dropping the hold. Test builds only.

use super::super::{CredentialStores, Engine};

/// The engine's step lock, held until this is dropped. Taken WITHOUT counting a session step: it ends no session.
pub struct StepHold<'a> {
    _step: tokio::sync::MutexGuard<'a, ()>,
}

impl<C: CredentialStores> Engine<C> {
    /// Takes the engine's step lock and returns once it holds it; dropping the hold lets it go. Every step that
    /// takes the lock (a sign-out, a lock of the vault, a stop of sync) waits meanwhile. BLOCKS on the engine's
    /// runtime: call it from a test's own thread, never from a task of that runtime.
    pub fn hold_session_step_for_test(&self) -> StepHold<'_> {
        StepHold {
            _step: self.runtime.block_on(self.transition.lock()),
        }
    }
}
