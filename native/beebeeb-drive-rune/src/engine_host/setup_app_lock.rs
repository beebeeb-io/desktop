//! Step 2 of the first run, "Lock Beebeeb" (DW-0202), on the real engine: the switch goes to core's rule through
//! the engine ([`beebeeb_engine::app::Engine::set_app_lock_for`]), as Sign-in and security's does.
//!
//! On (the default) asks nothing. Off is a weaker lock, and core wants a proof given for that change here too (the
//! sign-in that came before is not one): Windows Hello is asked first, and only its "verified" goes to the engine. A
//! cancelled, failed or unavailable prompt changes nothing, and "You're set up" says app lock was not changed. The
//! answer is drawn only once the engine gave it (the step's source answers later).
//!
//! Must not: write "off" before the prompt answered, or keep its answer for another change.

pub(super) use beebeeb_engine::app::lock_password_api::{AppLockProof, AppLockRefused};

use super::{Answer, EngineSetupSource, SetupEngine, log};
use crate::engine_host::hello::{HelloPrompter, HelloResult};
use crate::frames::signin::setup::source::{Outcome, Switch};

const NOT_DONE: Outcome = Outcome {
    switch: Switch::AppLock,
    done: false,
    now: None,
};

impl<E: SetupEngine> EngineSetupSource<E> {
    /// The source with the main window's Windows Hello prompt, which turning app lock off asks first.
    pub fn with_hello(mut self, hello: Box<dyn HelloPrompter>) -> Self {
        self.hello = hello;
        self
    }

    /// "Continue" on step 2 with the switch at `on`.
    pub(super) fn ask_app_lock(&mut self, on: bool) {
        if on {
            return self.write_app_lock(true, None);
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
