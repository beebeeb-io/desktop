//! Step 2, "Lock Beebeeb", on the engine double of `setup_tests.rs`: on is written at once; off asks Windows Hello
//! first (core wants a proof for it in the first run too) and only its "verified" is written; every answer is said
//! only once the engine gave it.

use std::sync::atomic::AtomicUsize;

use super::*;
use crate::engine_host::hello::{HelloCancel, HelloDone, HelloPrompter, HelloResult};
use crate::frames::signin::setup::source::LockAfter;
use beebeeb_engine::app::lock_password_api::{AppLockProof, AppLockRefused};

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

// ---------------------------------------------------------------------------
// "Not now" and "Skip setup" on the Lock step, in the first run's window on the real source: a Windows Hello that
// does not confirm leaves app lock as it was, and "You're set up" says "Not changed", never "Off".
// ---------------------------------------------------------------------------

/// The double of `setup_tests.rs`, but one that binds app lock (the double alone leaves step 2 "not available").
struct Binds(Arc<Fake>);

impl SetupPasskeyEngine for Binds {}

impl SetupEngine for Binds {
    fn view(&self) -> SyncView {
        self.0.view()
    }
    fn stamp(&self) -> Stamp {
        self.0.stamp()
    }
    fn wake(&self) {
        self.0.wake()
    }
    fn first_run_due(&self) -> Result<bool, SetupRefused> {
        self.0.first_run_due()
    }
    fn mark_done_for(&self, session: SessionGeneration) -> Result<(), SetupRefused> {
        self.0.mark_done_for(session)
    }
    fn set_notifications_for(&self, session: SessionGeneration, on: bool) -> Result<(), SetupRefused> {
        self.0.set_notifications_for(session, on)
    }
    fn sync_folder(&self) -> Option<PathBuf> {
        self.0.sync_folder()
    }
    fn settings_folder(&self) -> Result<Option<PathBuf>, SetupRefused> {
        self.0.settings_folder()
    }
    fn suggested_folder(&self) -> PathBuf {
        self.0.suggested_folder()
    }
    fn check_folder(&self, folder: &Path) -> Result<(), SyncFolderRefused> {
        self.0.check_folder(folder)
    }
    fn start(&self, folder: PathBuf) -> Result<SyncStart, SyncStartRefused> {
        self.0.start(folder)
    }
    fn keep_folder(&self, folder: &Path) -> Result<bool, SetupRefused> {
        self.0.keep_folder(folder)
    }
    fn ask_explorer_again(&self) {
        self.0.ask_explorer_again()
    }
    fn set_app_lock_for(
        &self,
        session: SessionGeneration,
        on: bool,
        proof: Option<AppLockProof>,
    ) -> Result<(), AppLockRefused> {
        self.0.set_app_lock_for(session, on, proof)
    }
    fn binds_app_lock(&self) -> bool {
        true
    }
}

/// The first run's window on `Binds`, with `prompts` as its Windows Hello.
fn lock_window(prompts: &Prompts) -> (Runtime<State>, Arc<Fake>) {
    let set = set();
    set.engine.set(Start::Running, EngineExplorer::Added);
    let Set {
        engine,
        asked,
        platform,
        first_run,
        ..
    } = set;
    let fixture = Arc::new(FixtureEngine::ready());
    fixture.set_account(FIRST);
    let mut state = crate::shell::state::init_with(Shell::fixture_on(fixture.clone(), platform.clone()));
    let (binds, source_hooks, hello) = (Arc::new(Binds(engine.clone())), hooks(&asked, platform, first_run), prompts.clone());
    state.setup_source = Some(Box::new(move || {
        Box::new(EngineSetupSource::new(binds.clone(), source_hooks.clone()).with_hello(Box::new(hello.clone())))
    }));
    let mut rt = Runtime::new(
        state,
        crate::view,
        (960.0, 640.0),
        Env {
            system_dark: false,
            scale: 2.0,
        },
    );
    rt.on_key = Some(host::on_key);
    rt.poll = Some(|s: &mut State, seconds: f64, tx: &mut Tx| host::poll(s, seconds, tx));
    rt.frame(0.0);
    fixture.set_gate_for_scene(ShellGate::FirstRun);
    wait_for(&mut rt, "the first run", |rt| rt.state.setup.is_some());
    (rt, engine)
}

/// "Not now" or "Skip setup" on the Lock step, then Windows Hello answers `result`: app lock was asked nothing of
/// the engine, and the last page says "Not changed" for it.
fn leave_the_lock_step_with(skip: bool, result: HelloResult) {
    let prompts = Prompts::default();
    let (mut rt, engine) = lock_window(&prompts);
    // Step 1 has nothing here: on to step 2.
    press(&mut rt, "Continue");
    assert!(says(&rt, "Lock Beebeeb"), "the Lock step is offered");
    rt.update(|s, _| {
        let ui = s.setup.as_mut().expect("the first run");
        if skip { ui.skip() } else { ui.not_now() }
    });
    assert_eq!(prompts.asked.load(Ordering::SeqCst), 1, "Windows Hello is asked first");
    rt.update(|s, _| {
        if let Some(ui) = s.setup.as_mut() {
            ui.skip();
        }
    });
    wait_for(&mut rt, "the last page", |rt| says(rt, "You're set up"));
    assert!(says(&rt, "App lock, Saving"), "no value before Windows Hello answered");
    assert!(engine.app_lock.lock().unwrap().is_empty());
    prompts.answer(result);
    wait_for(&mut rt, "not changed", |rt| says(rt, "App lock, Not changed"));
    assert!(!says(&rt, "App lock, Off") && !says(&rt, "App lock, On"));
    assert!(
        engine.app_lock.lock().unwrap().is_empty(),
        "nothing was written for a prompt that did not confirm ({result:?})"
    );
}

#[test]
fn not_now_on_the_lock_step_with_a_hello_that_does_not_confirm_says_not_changed() {
    for result in [HelloResult::Cancelled, HelloResult::Failed, HelloResult::Unavailable] {
        leave_the_lock_step_with(false, result);
    }
}

#[test]
fn skip_setup_on_the_lock_step_with_a_hello_that_does_not_confirm_says_not_changed() {
    for result in [HelloResult::Cancelled, HelloResult::Failed, HelloResult::Unavailable] {
        leave_the_lock_step_with(true, result);
    }
}

/// The yes of the same prompt still turns it off, and says "Off" (the other half of the rule).
#[test]
fn skip_setup_on_the_lock_step_with_hello_verified_turns_app_lock_off_with_the_proof() {
    let prompts = Prompts::default();
    let (mut rt, engine) = lock_window(&prompts);
    press(&mut rt, "Continue");
    rt.update(|s, _| {
        if let Some(ui) = s.setup.as_mut() {
            ui.skip();
        }
    });
    wait_for(&mut rt, "the last page", |rt| says(rt, "You're set up"));
    prompts.answer(HelloResult::Verified);
    wait_for(&mut rt, "off", |rt| says(rt, "App lock, Off"));
    assert_eq!(engine.app_lock.lock().unwrap().clone(), [(session(1), false, true)]);
}

fn step_of(rt: &mut Runtime<State>) -> crate::frames::signin::setup::Step {
    rt.state.setup.as_ref().expect("the first run").step
}

/// "Not now" stays on the Lock step while Windows Hello is up (its prompt must not show over the next step), and
/// moves on once it answered, whatever the answer.
#[test]
fn not_now_stays_on_the_lock_step_until_windows_hello_answers() {
    use crate::frames::signin::setup::Step;
    for result in [
        HelloResult::Verified,
        HelloResult::Cancelled,
        HelloResult::Failed,
        HelloResult::Unavailable,
    ] {
        let prompts = Prompts::default();
        let (mut rt, _engine) = lock_window(&prompts);
        press(&mut rt, "Continue");
        assert_eq!(step_of(&mut rt), Step::Lock);
        rt.update(|s, _| s.setup.as_mut().expect("the first run").not_now());
        assert_eq!(prompts.asked.load(Ordering::SeqCst), 1);
        for _ in 0..5 {
            rt.frame(0.1);
        }
        assert_eq!(step_of(&mut rt), Step::Lock, "still on Lock while Hello is up ({result:?})");
        prompts.answer(result);
        wait_for(&mut rt, "the next step", |rt| {
            rt.state.setup.as_ref().is_some_and(|ui| ui.step == Step::Backup)
        });
    }
}
