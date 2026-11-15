//! App lock's switch on Sign-in and security, on the real engine and the parent module's fake: what the page is told
//! (`security_facts().app_lock`), what turning it off asks first (a Windows Hello prompt the test answers), and that
//! an answer for another session is dropped. Every name is invented; nothing here waits on Windows.

use std::sync::atomic::AtomicUsize;

use beebeeb_engine::app::lock_password_api::AppLockProof;

use super::*;
use crate::engine_host::hello::{HelloCancel, HelloDone, HelloPrompter, HelloResult};

/// The prompts that are up, oldest first.
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
    fn asked(&self) -> usize {
        self.asked.load(Ordering::SeqCst)
    }

    /// The oldest prompt that is up ends with `result` (on the prompt's own thread, in the app).
    fn answer(&self, result: HelloResult) {
        let done = self.up.lock().unwrap().remove(0);
        done(result);
    }
}

fn with_prompts(server: &Server) -> (TestEngine, Source, Prompts) {
    let engine = signed_in(server);
    let prompts = Prompts::default();
    let mut source = Source::new(engine.clone()).with_hello(Box::new(prompts.clone()));
    opened(&mut source, Pane::Security);
    (engine, source, prompts)
}

fn switch(source: &Source) -> Option<bool> {
    source.security_facts().app_lock.map(|(on, _)| on)
}

fn stored(engine: &TestEngine) -> bool {
    engine.app_lock_for(engine.session_turn()).unwrap()
}

#[test]
fn the_switch_says_the_engines_value_and_nothing_before_it_answered() {
    let server = server();
    let engine = signed_in(&server);
    let mut source = Source::new(engine.clone());
    assert_eq!(
        source.security_facts().app_lock,
        None,
        "not known before the engine answered"
    );
    opened(&mut source, Pane::Security);
    // On (the default), and no delay: nothing in this version acts on one, so none is drawn.
    assert_eq!(source.security_facts().app_lock, Some((true, String::new())));
    engine
        .set_app_lock_for(engine.session_turn(), false, Some(AppLockProof::PlatformVerified))
        .unwrap();
    source.closed();
    opened(&mut source, Pane::Security);
    assert_eq!(switch(&source), Some(false), "read again when the page opens");
}

#[test]
fn turning_it_off_asks_windows_hello_first_and_only_its_yes_is_written() {
    let server = server();
    let (engine, mut source, prompts) = with_prompts(&server);
    source.set_app_lock(false);
    assert_eq!(prompts.asked(), 1);
    source.pump();
    assert_eq!(switch(&source), Some(true), "the switch never flips before the answer");
    assert!(stored(&engine));
    // A cancelled or failed prompt changes nothing, and says so.
    for result in [HelloResult::Cancelled, HelloResult::Failed] {
        if prompts.up.lock().unwrap().is_empty() {
            source.set_app_lock(false);
        }
        prompts.answer(result);
        settle(&mut source);
        assert_eq!(switch(&source), Some(true), "{result:?}");
        assert!(stored(&engine), "{result:?}");
        assert_eq!(
            source.notice(),
            Some("App lock is still on: Windows Hello did not confirm it's you.")
        );
        assert!(source.notice_failed());
    }
    assert_eq!(prompts.asked(), 2);
    // Verified: the engine writes it, and the switch follows the engine's answer.
    source.set_app_lock(false);
    prompts.answer(HelloResult::Verified);
    settle(&mut source);
    assert_eq!(switch(&source), Some(false));
    assert!(!stored(&engine));
    assert_eq!(source.notice(), None);
}

#[test]
fn turning_it_on_asks_nothing() {
    let server = server();
    let (engine, mut source, prompts) = with_prompts(&server);
    engine
        .set_app_lock_for(engine.session_turn(), false, Some(AppLockProof::PlatformVerified))
        .unwrap();
    source.closed();
    opened(&mut source, Pane::Security);
    assert_eq!(switch(&source), Some(false));
    source.set_app_lock(true);
    settle(&mut source);
    assert_eq!(prompts.asked(), 0, "a stronger lock needs no proof");
    assert_eq!(switch(&source), Some(true));
    assert!(stored(&engine));
}

#[test]
fn a_pc_without_a_prompt_keeps_it_on_and_says_why() {
    let server = server();
    let engine = signed_in(&server);
    let mut source = Source::new(engine.clone());
    opened(&mut source, Pane::Security);
    source.set_app_lock(false);
    settle(&mut source);
    assert_eq!(switch(&source), Some(true));
    assert!(stored(&engine));
    assert_eq!(
        source.notice(),
        Some("App lock is still on: turning it off needs Windows Hello, which is not set up on this PC.")
    );
}

#[test]
fn a_late_answer_after_a_session_change_is_dropped() {
    let server = server();
    let (engine, mut source, prompts) = with_prompts(&server);
    source.set_app_lock(false);
    // The screen locks and opens again while the prompt is up: another turn of the session.
    engine.lock_screen().unwrap();
    engine.unlock_screen_platform_verified(engine.session_turn()).unwrap();
    source.pump();
    prompts.answer(HelloResult::Verified);
    settle(&mut source);
    assert!(stored(&engine), "a yes asked for in another turn writes nothing");
    assert_eq!(switch(&source), Some(true));
    assert_eq!(source.notice(), None, "and says nothing to this turn");
    // The page works for the new turn.
    source.set_app_lock(false);
    prompts.answer(HelloResult::Verified);
    settle(&mut source);
    assert!(!stored(&engine));
}
