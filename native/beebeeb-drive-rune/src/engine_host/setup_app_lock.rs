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
//!
//! The prompt that is up is the first run's to take away (`FirstRunPrompt`, installed by `with_hello`): leaving the
//! first run takes it away, and a newer prompt of the first run takes its place.

pub(super) use beebeeb_engine::app::lock_password_api::{AppLockProof, AppLockRefused, LockAfter as EngineLockAfter};

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::{Answer, EngineSetupSource, SetupEngine, log};
use crate::engine_host::hello::{HelloCancel, HelloDone, HelloPrompter, HelloResult};
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

/// The prompt that is up: its number, and its cancel.
type Up = Arc<Mutex<Option<(u64, HelloCancel)>>>;

/// The first run's Windows Hello prompt. The cancel of the prompt it asked for is kept and called when the first run
/// asks a newer one or is left: the shell drops the source (and with it this prompter) the moment the gate shows
/// anything but the first run, or another account's first run takes its place, so a prompt still up goes with it.
/// After the cancel no answer of that prompt is decided any more (`hello::HelloCancel`). A prompt that answered is
/// no longer held, so it is never taken away after its answer.
struct FirstRunPrompt {
    prompter: Box<dyn HelloPrompter>,
    up: Up,
    /// The number of the last prompt asked.
    asked: AtomicU64,
    /// The highest number that answered: a prompt that answered before its cancel was kept is never kept.
    answered: Arc<AtomicU64>,
}

impl FirstRunPrompt {
    fn new(prompter: Box<dyn HelloPrompter>) -> Self {
        Self {
            prompter,
            up: Up::default(),
            asked: AtomicU64::new(0),
            answered: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Takes the prompt that is up away (only when it is `number`, if one is named). The cancel runs outside the
    /// lock: a `done` that runs meanwhile waits for nothing.
    fn take_away(up: &Up, number: Option<u64>) {
        let held = {
            let mut up = up.lock().unwrap_or_else(|p| p.into_inner());
            let another = matches!((number, up.as_ref()), (Some(number), Some((held, _))) if *held != number);
            if another { None } else { up.take() }
        };
        if let Some((_, cancel)) = held {
            cancel();
        }
    }
}

impl HelloPrompter for FirstRunPrompt {
    fn start(&self, done: HelloDone) -> Result<HelloCancel, ()> {
        // One prompt of the first run at a time: the one that is up goes first.
        Self::take_away(&self.up, None);
        let number = self.asked.fetch_add(1, Ordering::SeqCst) + 1;
        let (up, answered) = (self.up.clone(), self.answered.clone());
        let cancel = self.prompter.start(Box::new(move |result| {
            // Answered: nothing of it is up any more.
            answered.fetch_max(number, Ordering::SeqCst);
            {
                let mut up = up.lock().unwrap_or_else(|p| p.into_inner());
                if up.as_ref().is_some_and(|(held, _)| *held == number) {
                    *up = None;
                }
            }
            done(result);
        }))?;
        {
            let mut up = self.up.lock().unwrap_or_else(|p| p.into_inner());
            if self.answered.load(Ordering::SeqCst) < number {
                *up = Some((number, cancel));
            }
        }
        let up = self.up.clone();
        Ok(Box::new(move || Self::take_away(&up, Some(number))))
    }
}

impl Drop for FirstRunPrompt {
    fn drop(&mut self) {
        Self::take_away(&self.up, None);
    }
}

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
    /// The source with the main window's Windows Hello prompt, which turning app lock off asks first. The first run
    /// holds the prompt it asked for (`FirstRunPrompt`): it goes when the first run is left.
    pub fn with_hello(mut self, hello: Box<dyn HelloPrompter>) -> Self {
        self.hello = Box::new(FirstRunPrompt::new(hello));
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

    /// The delay: up to the default it asks nothing; longer, Windows Hello first. (Step 2 again, from "You're set
    /// up", may find a shorter delay stored: the engine then refuses a write without a proof, and the prompt comes
    /// after all, `prove_lock_after`.)
    fn ask_lock_after(&mut self, after: EngineLockAfter) {
        if after <= EngineLockAfter::DEFAULT {
            return self.write_lock_after(after, None);
        }
        self.prove_lock_after(after);
    }

    /// Windows Hello for a delay the engine counts as longer; its answer comes through the channel.
    pub(super) fn prove_lock_after(&mut self, after: EngineLockAfter) {
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
        let proved = proof.is_some();
        self.ask(
            "first-run-lock-after",
            move |engine, session| match engine.set_lock_after_for(session, after, proof) {
                Ok(()) => Answer::Switched(Outcome {
                    done: true,
                    ..AFTER_NOT_DONE
                }),
                // Longer than what the engine has stored, and asked without a proof: the prompt, once.
                Err(AppLockRefused::ProofNeeded) if !proved => Answer::AfterNeedsProof(after),
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
