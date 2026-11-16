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
    /// How many prompts their owner took away (the cancels called).
    taken_away: Arc<AtomicUsize>,
}

impl HelloPrompter for Prompts {
    fn start(&self, done: HelloDone) -> Result<HelloCancel, ()> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        self.up.lock().unwrap().push(done);
        let taken_away = self.taken_away.clone();
        Ok(Box::new(move || {
            taken_away.fetch_add(1, Ordering::SeqCst);
        }))
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

fn lock_after(done: bool) -> Outcome {
    Outcome {
        switch: Switch::LockAfter,
        done,
        now: None,
    }
}

/// The outcomes in a fixed order (two workers answer in either order).
fn sorted(mut outcomes: Vec<Outcome>) -> Vec<Outcome> {
    outcomes.sort_by_key(|outcome| outcome.switch == Switch::LockAfter);
    outcomes
}

#[test]
fn continue_with_app_lock_on_writes_it_and_the_delay_for_the_session_and_says_it_once_answered() {
    let mut set = set();
    // This source sets the delay: step 2 draws its row.
    assert!(set.source.sets_lock_after());
    set.source.set_app_lock(true, LockAfter::Immediately);
    assert!(
        set.source.take_outcomes().is_empty(),
        "nothing is said before the engine answered"
    );
    settle(&mut set, 2);
    assert_eq!(set.engine.app_lock.lock().unwrap().clone(), [(session(1), true, false)]);
    assert_eq!(
        set.engine.lock_after.lock().unwrap().clone(),
        [(
            session(1),
            beebeeb_engine::app::lock_password_api::LockAfter::Immediately,
            false
        )],
        "up to five minutes asks nothing"
    );
    let mut said = set.source.take_outcomes();
    if said.len() < 2 {
        set.source.poll();
        said.extend(set.source.take_outcomes());
    }
    assert_eq!(
        sorted(said),
        [app_lock(true, Some(true)), vec![lock_after(true)]].concat()
    );
}

#[test]
fn a_longer_delay_asks_windows_hello_first_and_is_written_only_with_its_yes() {
    let mut set = set();
    let prompts = Prompts::default();
    set.source.hello = Box::new(prompts.clone());
    set.source.set_app_lock(true, LockAfter::Min60);
    assert_eq!(
        prompts.asked.load(Ordering::SeqCst),
        1,
        "longer than five minutes: the prompt first"
    );
    settle(&mut set, 1);
    assert_eq!(
        set.source.take_outcomes(),
        app_lock(true, Some(true)),
        "app lock on asks nothing"
    );
    prompts.answer(HelloResult::Cancelled);
    settle(&mut set, 2);
    assert_eq!(set.source.take_outcomes(), [lock_after(false)]);
    assert!(set.engine.lock_after.lock().unwrap().is_empty());
    set.source.set_app_lock(true, LockAfter::Min60);
    prompts.answer(HelloResult::Verified);
    settle(&mut set, 4);
    settle(&mut set, 5);
    assert_eq!(
        set.engine.lock_after.lock().unwrap().clone(),
        [(
            session(1),
            beebeeb_engine::app::lock_password_api::LockAfter::Minutes60,
            true
        )]
    );
}

#[test]
fn app_lock_off_writes_no_delay() {
    let mut set = set();
    let prompts = Prompts::default();
    set.source.hello = Box::new(prompts.clone());
    set.source.set_app_lock(false, LockAfter::Immediately);
    prompts.answer(HelloResult::Verified);
    settle(&mut set, 1);
    settle(&mut set, 2);
    assert_eq!(set.engine.app_lock.lock().unwrap().clone(), [(session(1), false, true)]);
    assert!(
        set.engine.lock_after.lock().unwrap().is_empty(),
        "off never locks: no delay"
    );
    assert_eq!(prompts.asked.load(Ordering::SeqCst), 1);
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

/// Step 2 again (opened from "You're set up"): the engine compares with what it stored, not with the default. A
/// delay longer than that is refused without a proof; the step then asks Windows Hello and writes with its yes.
#[test]
fn step_two_again_asks_windows_hello_when_the_engine_wants_a_proof_against_what_it_stored() {
    let mut set = set();
    let prompts = Prompts::default();
    set.source.hello = Box::new(prompts.clone());
    set.source.set_app_lock(true, LockAfter::Immediately);
    settle(&mut set, 2);
    set.source.poll();
    let _ = set.source.take_outcomes();
    assert_eq!(prompts.asked.load(Ordering::SeqCst), 0);
    // Five minutes is the default, but longer than the Immediately now stored.
    set.source.set_app_lock(true, LockAfter::Min5);
    settle(&mut set, 4);
    set.source.poll();
    assert_eq!(
        prompts.asked.load(Ordering::SeqCst),
        1,
        "the engine's refusal brings the prompt"
    );
    prompts.answer(HelloResult::Verified);
    settle(&mut set, 5);
    settle(&mut set, 6);
    assert_eq!(
        set.engine.lock_after.lock().unwrap().last().copied(),
        Some((
            session(1),
            beebeeb_engine::app::lock_password_api::LockAfter::Minutes5,
            true
        ))
    );
}

/// The first run holds its prompt (`with_hello`): a prompt that answered is never taken away, a newer one takes the
/// place of the one that is up, and leaving the first run (the shell drops its source) takes the one that is up away.
#[test]
fn leaving_the_first_run_takes_its_prompt_away() {
    let prompts = Prompts::default();
    let set = set();
    let mut set = Set {
        source: set.source.with_hello(Box::new(prompts.clone())),
        ..set
    };
    let taken_away = || prompts.taken_away.load(Ordering::SeqCst);
    set.source.set_app_lock(false, LockAfter::Min5);
    assert_eq!(prompts.asked.load(Ordering::SeqCst), 1);
    prompts.answer(HelloResult::Cancelled);
    set.source.set_app_lock(false, LockAfter::Min5);
    assert_eq!(prompts.asked.load(Ordering::SeqCst), 2);
    assert_eq!(taken_away(), 0, "the prompt that answered is not taken away");
    set.source.set_app_lock(false, LockAfter::Min5);
    assert_eq!(prompts.asked.load(Ordering::SeqCst), 3);
    assert_eq!(
        taken_away(),
        1,
        "the newer prompt takes the place of the one that was up"
    );
    drop(set);
    assert_eq!(taken_away(), 2, "leaving the first run takes its prompt away");
}
