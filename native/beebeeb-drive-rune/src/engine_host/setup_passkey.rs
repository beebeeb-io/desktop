//! First-run step 1, "Add a passkey" (DW-0201), with the account password (owner ruling: a passkey change proves
//! the password). Part of [`EngineSetupSource`] (`setup.rs`).
//!
//! The same engine path as Settings › Passkeys (`passkeys.rs`), on a worker of the adapter's own `ask` (so the press
//! is stamped with this first run's session, and an answer under another session is dropped by `poll`):
//! 1. the password is moved once into a wiping buffer (`passkeys::wiping`); an empty one is refused here and nothing
//!    is asked of Beebeeb or of Windows;
//! 2. on the worker the account's passkeys are read under the session of the press (Settings' rule: an add is made
//!    only on a list read under that session), then core's step-up, Windows' dialog and the registration run
//!    (`passkeys::add_with_dialog`) under the session the list was read under.
//!
//! The press answers "started" only: the window is never held while Windows' dialog is open. How it ended comes
//! down the adapter's channel, is taken in by `poll` and read once by the window (`take_passkey_answer`).
//!
//! Must not: log or keep the password (it lives in the wiping buffer until the engine call drops it); log a
//! passkey's name or a server's words. A log line names the kind of refusal only.

use beebeeb_engine::app::passkeys_api::PasskeyProblem;
use zeroize::Zeroizing;

use super::*;
use crate::frames::signin::setup::source::PasskeyRefused;

/// What step 1 asks of the engine handle: part of [`SetupEngine`] (its supertrait). The real one is [`Engine`]; a
/// test's double that binds no passkey keeps the refusing defaults.
pub trait SetupPasskeyEngine {
    /// Whether this engine adds a passkey in step 1: the real one does on Windows (Windows' passkey dialog).
    fn binds_passkey(&self) -> bool {
        false
    }
    /// The account's passkeys, read for `session` (`Engine::passkeys`): an add is made only on a list read under the
    /// session of the press (Settings › Passkeys' rule, the engine's `listed_under`). `Ok`: the session it was read
    /// under, which is `session`. BLOCKS (a request).
    fn read_passkeys_for(&self, _session: SessionGeneration) -> Result<SessionGeneration, PasskeyProblem> {
        Err(PasskeyProblem::Unavailable)
    }
    /// Add a passkey under the list's session: core's step-up from `password`, Windows' passkey dialog, the
    /// registration (`passkeys::add_with_dialog`, Settings › Passkeys' own worker function). The password is moved in
    /// once, in a wiping buffer. BLOCKS for as long as the dialog is open.
    fn add_passkey_for(
        &self,
        _listed_under: SessionGeneration,
        _password: Zeroizing<String>,
    ) -> Result<(), PasskeyProblem> {
        Err(PasskeyProblem::Unavailable)
    }
}

impl<C: CredentialStores> SetupPasskeyEngine for Engine<C> {
    fn binds_passkey(&self) -> bool {
        cfg!(windows)
    }
    fn read_passkeys_for(&self, session: SessionGeneration) -> Result<SessionGeneration, PasskeyProblem> {
        // Handed over only while the session in memory is still the one of the press.
        let listed = self.passkeys()?;
        if listed.session == session {
            Ok(listed.session)
        } else {
            Err(PasskeyProblem::NotSignedIn)
        }
    }
    fn add_passkey_for(
        &self,
        listed_under: SessionGeneration,
        password: Zeroizing<String>,
    ) -> Result<(), PasskeyProblem> {
        // The window that owns Windows' dialog: this process's own (as the sign-in's passkey reads it on its worker).
        let Some(hwnd) = crate::engine_host::passkeys::host_window() else {
            return Err(PasskeyProblem::Unavailable);
        };
        crate::engine_host::passkeys::add_with_dialog(self, hwnd, listed_under, password).map(|_| ())
    }
}

/// What step 1 keeps between a press and its answer. The adapter holds one (`EngineSetupSource::setup_passkey`).
#[derive(Default)]
pub(super) struct SetupPasskeyCalls {
    /// An add is on its worker (Windows' dialog may be open): no second one is asked.
    adding: bool,
    /// How the last add ended, until the window takes it.
    answer: Option<PasskeyAnswer>,
}

/// The step's kind of refusal for the engine's. Each is said in Settings › Passkeys' words.
pub(super) fn refused_of(problem: PasskeyProblem) -> PasskeyRefused {
    match problem {
        PasskeyProblem::WrongPassword => PasskeyRefused::WrongPassword,
        PasskeyProblem::Cancelled => PasskeyRefused::Cancelled,
        PasskeyProblem::Offline => PasskeyRefused::Offline,
        PasskeyProblem::RateLimited => PasskeyRefused::TooManyTries,
        PasskeyProblem::Unavailable => PasskeyRefused::NotOnThisPc,
        PasskeyProblem::NotSignedIn
        | PasskeyProblem::SessionEnded
        | PasskeyProblem::InvalidInput
        | PasskeyProblem::Refused
        | PasskeyProblem::Platform => PasskeyRefused::NotAdded,
    }
}

impl<E: SetupEngine> EngineSetupSource<E> {
    /// "Add a passkey", for this first run's session. `true`: started (the answer comes with a later `poll`).
    /// `false`: nothing was started: one is on its way, the step is not offered, the press is another session's, or
    /// the password is empty (that refusal is said at once, through `take_passkey_answer`).
    pub(super) fn setup_passkey_add_passkey(&mut self, password: Password<'_>) -> bool {
        // One at a time (Windows' dialog is one); another session's press sends and says nothing.
        if self.setup_passkey.adding || !self.engine.binds_passkey() || !self.mine() {
            return false;
        }
        // No password: refused here. Nothing is asked of Beebeeb or of Windows.
        let Some(password) = crate::engine_host::passkeys::wiping(&password) else {
            self.setup_passkey.answer = Some(PasskeyAnswer::Refused(PasskeyRefused::NoPassword));
            self.changed = true;
            return false;
        };
        self.setup_passkey.adding = true;
        self.setup_passkey.answer = None;
        self.ask(
            "first-run-passkey",
            move |engine, session| {
                // The list first, under the session of the press; then the add under the session it was read under.
                // The password is moved into the add and dropped (wiped) with it, or here when the read refused.
                let added = engine
                    .read_passkeys_for(session)
                    .and_then(|listed_under| engine.add_passkey_for(listed_under, password));
                match added {
                    Ok(()) => Answer::Passkey(PasskeyAnswer::Added),
                    Err(problem) => {
                        log(format!("first run: a passkey was not added ({problem:?})"));
                        Answer::Passkey(PasskeyAnswer::Refused(refused_of(problem)))
                    }
                }
            },
            // A worker that could not start, or that panicked: not added, said as that.
            Some(Answer::Passkey(PasskeyAnswer::Refused(PasskeyRefused::NotAdded))),
        );
        true
    }

    /// `poll` took in how the add ended (this first run's session only).
    pub(super) fn took_passkey(&mut self, answer: PasskeyAnswer) {
        self.setup_passkey.adding = false;
        self.setup_passkey.answer = Some(answer);
    }

    /// The answer, once.
    pub(super) fn setup_passkey_answer(&mut self) -> Option<PasskeyAnswer> {
        self.setup_passkey.answer.take()
    }
}
