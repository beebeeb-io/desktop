//! Step 2, "Lock Beebeeb", on the engine double of `setup_tests.rs`: on is written at once; off asks Windows Hello
//! first (core wants a proof for it in the first run too) and only its "verified" is written; every answer is said
//! only once the engine gave it.

use std::sync::atomic::AtomicUsize;

use super::*;
use crate::engine_host::hello::{HelloCancel, HelloDone, HelloPrompter, HelloResult};
use crate::frames::signin::setup::source::LockAfter;

#[derive(Clone, Default)]
struct Prompts {
    up: Arc<Mutex<Vec<HelloDone>>>,
    asked: Arc<AtomicUsize>,
}

impl HelloPrompter for Prompts {
    fn start(&self, done: HelloDone) -> Result<HelloCancel, ()> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        self.up.lock().unwrap().push(done);
        Ok(Box::new(|| {}))
    }
}

impl Prompts {
    fn answer(&self, result: HelloResult) {
        let done = self.up.lock().unwrap().remove(0);
        done(result);
    }
}

fn app_lock(done: bool, now: Option<bool>) -> Vec<Outcome> {
    vec![Outcome {
        switch: Switch::AppLock,
        done,
        now,
    }]
}

#[test]
fn continue_with_app_lock_on_writes_it_for_the_session_and_says_it_once_answered() {
    let mut set = set();
    set.source.set_app_lock(true, LockAfter::Min30);
    assert!(
        set.source.take_outcomes().is_empty(),
        "nothing is said before the engine answered"
    );
    assert!(settle(&mut set, 1));
    assert_eq!(set.engine.app_lock.lock().unwrap().clone(), [(session(1), true, false)]);
    assert_eq!(set.source.take_outcomes(), app_lock(true, Some(true)));
    // The delay is not set by this source: step 2 draws no such row.
    assert!(!set.source.sets_lock_after());
}

#[test]
fn continue_with_app_lock_off_asks_windows_hello_first_and_writes_only_its_yes() {
    let mut set = set();
    let prompts = Prompts::default();
    set.source.hello = Box::new(prompts.clone());
    // Cancelled: nothing written, and "not changed".
    set.source.set_app_lock(false, LockAfter::Min5);
    assert_eq!(prompts.asked.load(Ordering::SeqCst), 1);
    assert!(
        set.source.take_outcomes().is_empty(),
        "nothing is said before the prompt answered"
    );
    prompts.answer(HelloResult::Cancelled);
    assert!(settle(&mut set, 1));
    assert_eq!(set.source.take_outcomes(), app_lock(false, None));
    assert!(set.engine.app_lock.lock().unwrap().is_empty());
    // Verified: written off, with the proof (the prompt's answer, then the write's worker).
    set.source.set_app_lock(false, LockAfter::Min5);
    prompts.answer(HelloResult::Verified);
    settle(&mut set, 2);
    settle(&mut set, 3);
    assert_eq!(set.engine.app_lock.lock().unwrap().clone(), [(session(1), false, true)]);
    assert_eq!(set.source.take_outcomes(), app_lock(true, Some(false)));
}

#[test]
fn a_pc_without_a_prompt_keeps_app_lock_on_and_says_not_changed() {
    let mut set = set();
    set.source.set_app_lock(false, LockAfter::Min5);
    set.source.poll();
    assert_eq!(set.source.take_outcomes(), app_lock(false, None));
    assert!(set.engine.app_lock.lock().unwrap().is_empty());
}
