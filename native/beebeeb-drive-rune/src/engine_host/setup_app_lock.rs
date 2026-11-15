//! Step 2 of the first run, "Lock Beebeeb" (DW-0202), on the real engine: the switch goes to core's rule through
//! the engine ([`beebeeb_engine::app::Engine::set_app_lock_for`]), as Sign-in and security's does, and with it on,
//! "Lock again after" ([`beebeeb_engine::app::Engine::set_lock_after_for`], the engine's `lock.after`).
//!
//! On (the default) asks nothing. Off is a weaker lock, and core wants a proof given for that change here too (the
//! sign-in that came before is not one): Windows Hello is asked first, and only its "verified" goes to the engine. A
//! cancelled, failed or unavailable prompt changes nothing, and "You're set up" says app lock was not changed. The
//! answer is drawn only once the engine gave it (the step's source answers later).
//!
//! The delay is written only with the switch on (off never locks). Up to the default (five minutes; a first run's
//! PC stores none) it asks nothing; longer is a weaker lock, so Windows Hello is asked first, as for off, and a
//! delay that was not stored is said in "You're set up".
//!
//! Must not: write "off" or a longer delay before the prompt answered, or keep its answer for another change.

pub(super) use beebeeb_engine::app::lock_password_api::{AppLockProof, AppLockRefused, LockAfter as EngineLockAfter};

use super::{Answer, EngineSetupSource, SetupEngine, log};
use crate::engine_host::hello::{HelloPrompter, HelloResult};
use crate::frames::signin::setup::source::{LockAfter, Outcome, Switch};

const NOT_DONE: Outcome = Outcome {
    switch: Switch::AppLock,
    done: false,
    now: None,
};

const AFTER_NOT_DONE: Outcome = Outcome {
    switch: Switch::LockAfter,
    done: false,
    now: None,
};

/// The step's choice as the engine's.
pub(super) fn engine_after(after: LockAfter) -> EngineLockAfter {
    match after {
        LockAfter::Immediately => EngineLockAfter::Immediately,
        LockAfter::Min5 => EngineLockAfter::Minutes5,
        LockAfter::Min30 => EngineLockAfter::Minutes30,
        LockAfter::Min60 => EngineLockAfter::Minutes60,
    }
}

impl<E: SetupEngine> EngineSetupSource<E> {
    /// The source with the main window's Windows Hello prompt, which turning app lock off asks first.
    pub fn with_hello(mut self, hello: Box<dyn HelloPrompter>) -> Self {
        self.hello = hello;
        self
    }

    /// "Continue" on step 2 with the switch at `on` and the delay `after` (written only with the switch on).
    pub(super) fn ask_app_lock(&mut self, on: bool, after: LockAfter) {
        if on {
            self.write_app_lock(true, None);
            if self.engine.binds_lock_after() {
                self.ask_lock_after(engine_after(after));
            }
            return;
        }
        if !self.mine() {
            return;
        }
        let (answers, stamp, engine) = (self.answers.clone(), self.made_for.clone(), self.engine.clone());
        let asked = self.hello.start(Box::new(move |result| {
            let _ = answers.send((stamp, Answer::LockProved(result)));
            engine.wake();
        }));
        if asked.is_err() {
            log("first run: app lock was not turned off (no Windows Hello prompt)");
            self.outcomes.push(NOT_DONE);
            self.changed = true;
        }
    }

    /// The prompt for turning it off answered (for this first run's session: `poll` dropped any other).
    pub(super) fn app_lock_proved(&mut self, result: HelloResult) {
        match result {
            HelloResult::Verified => self.write_app_lock(false, Some(AppLockProof::PlatformVerified)),
            HelloResult::Cancelled | HelloResult::Failed | HelloResult::Unavailable => {
                log("first run: app lock was not turned off (Windows Hello did not confirm)");
                self.outcomes.push(NOT_DONE);
            }
        }
    }

    /// The delay: up to the default it asks nothing; longer, Windows Hello first.
    fn ask_lock_after(&mut self, after: EngineLockAfter) {
        if after <= EngineLockAfter::DEFAULT {
            return self.write_lock_after(after, None);
        }
        if !self.mine() {
            return;
        }
        let (answers, stamp, engine) = (self.answers.clone(), self.made_for.clone(), self.engine.clone());
        let asked = self.hello.start(Box::new(move |result| {
            let _ = answers.send((stamp, Answer::AfterProved(after, result)));
            engine.wake();
        }));
        if asked.is_err() {
            log("first run: the lock delay was not changed (no Windows Hello prompt)");
            self.outcomes.push(AFTER_NOT_DONE);
            self.changed = true;
        }
    }

    /// The prompt for a longer delay answered (for this first run's session: `poll` dropped any other).
    pub(super) fn lock_after_proved(&mut self, after: EngineLockAfter, result: HelloResult) {
        match result {
            HelloResult::Verified => self.write_lock_after(after, Some(AppLockProof::PlatformVerified)),
            HelloResult::Cancelled | HelloResult::Failed | HelloResult::Unavailable => {
                log("first run: the lock delay was not changed (Windows Hello did not confirm)");
                self.outcomes.push(AFTER_NOT_DONE);
            }
        }
    }

    fn write_lock_after(&mut self, after: EngineLockAfter, proof: Option<AppLockProof>) {
        self.ask(
            "first-run-lock-after",
            move |engine, session| match engine.set_lock_after_for(session, after, proof) {
                Ok(()) => Answer::Switched(Outcome {
                    done: true,
                    ..AFTER_NOT_DONE
                }),
                Err(refused) => {
                    log(format!("first run: the lock delay was not changed ({refused:?})"));
                    Answer::Switched(AFTER_NOT_DONE)
                }
            },
            Some(Answer::Switched(AFTER_NOT_DONE)),
        );
    }

    fn write_app_lock(&mut self, on: bool, proof: Option<AppLockProof>) {
        self.ask(
            "first-run-app-lock",
            move |engine, session| match engine.set_app_lock_for(session, on, proof) {
                Ok(()) => Answer::Switched(Outcome {
                    done: true,
                    now: Some(on),
                    ..NOT_DONE
                }),
                Err(refused) => {
                    log(format!("first run: app lock was not changed ({refused:?})"));
                    Answer::Switched(NOT_DONE)
                }
            },
            Some(Answer::Switched(NOT_DONE)),
        );
    }
}
