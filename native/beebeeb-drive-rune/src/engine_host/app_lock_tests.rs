//! App lock's switch on Sign-in and security, on the real engine and the parent module's fake: what the page is told
//! (`security_facts().app_lock`), what turning it off asks first (a Windows Hello prompt the test answers), and that
//! an answer for another session is dropped. Every name is invented; nothing here waits on Windows.

use std::sync::atomic::AtomicUsize;

use beebeeb_engine::app::lock_password_api::{AppLockProof, LockAfter};
use beebeeb_engine::keychain::{AuthSecretStore, LockFlag};

use super::*;
use crate::engine_host::hello::{HelloCancel, HelloDone, HelloPrompter, HelloResult};

/// The prompts that are up, oldest first, and how many were taken away. A prompt that was taken away keeps its
/// answer here: the test hands it out late, as one decided before its cancel would arrive.
#[derive(Clone, Default)]
struct Prompts {
    up: Arc<Mutex<Vec<HelloDone>>>,
    asked: Arc<AtomicUsize>,
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
    fn asked(&self) -> usize {
        self.asked.load(Ordering::SeqCst)
    }

    fn taken_away(&self) -> usize {
        self.taken_away.load(Ordering::SeqCst)
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
    // On (the default), and the delay with no entry stored: five minutes (decision 2519), in the menu's words.
    assert_eq!(source.security_facts().app_lock, Some((true, "5 min".to_string())));
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
        Some("App lock is still on: Windows Hello is not set up on this PC. Use your password instead.")
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

fn delay(source: &Source) -> Option<String> {
    source.security_facts().app_lock.map(|(_, after)| after)
}

fn stored_after(engine: &TestEngine) -> Option<LockAfter> {
    engine.lock_after_for(engine.session_turn()).unwrap()
}

#[test]
fn the_menu_writes_lock_after_and_the_row_shows_it() {
    let server = server();
    let (engine, mut source, prompts) = with_prompts(&server);
    // Shorter than five minutes: written at once, nothing asked.
    source.set_lock_after("Immediately");
    assert_eq!(
        delay(&source).as_deref(),
        Some("5 min"),
        "never drawn before the engine answered"
    );
    settle(&mut source);
    assert_eq!(prompts.asked(), 0, "a shorter delay needs no proof");
    assert_eq!(stored_after(&engine), Some(LockAfter::Immediately));
    assert_eq!(delay(&source).as_deref(), Some("Immediately"));
    // Longer: Windows Hello first. Cancelled: nothing written, and said.
    source.set_lock_after("30 min");
    assert_eq!(prompts.asked(), 1);
    prompts.answer(HelloResult::Cancelled);
    settle(&mut source);
    assert_eq!(stored_after(&engine), Some(LockAfter::Immediately));
    assert_eq!(delay(&source).as_deref(), Some("Immediately"));
    assert_eq!(
        source.notice(),
        Some("The lock delay was not changed: Windows Hello did not confirm it's you.")
    );
    // Verified: the engine writes it, and the row follows the engine's answer.
    source.set_lock_after("30 min");
    prompts.answer(HelloResult::Verified);
    settle(&mut source);
    assert_eq!(stored_after(&engine), Some(LockAfter::Minutes30));
    assert_eq!(delay(&source).as_deref(), Some("30 min"));
    // Words the menu does not have choose nothing.
    source.set_lock_after("2 hours");
    settle(&mut source);
    assert_eq!(prompts.asked(), 2);
    assert_eq!(stored_after(&engine), Some(LockAfter::Minutes30));
}

#[test]
fn with_app_lock_off_no_delay_is_set() {
    let server = server();
    let (engine, mut source, prompts) = with_prompts(&server);
    engine
        .set_app_lock_for(engine.session_turn(), false, Some(AppLockProof::PlatformVerified))
        .unwrap();
    source.closed();
    opened(&mut source, Pane::Security);
    assert_eq!(switch(&source), Some(false));
    source.set_lock_after("Immediately");
    source.set_lock_after("60 min");
    settle(&mut source);
    assert_eq!(prompts.asked(), 0);
    assert_eq!(
        stored_after(&engine),
        Some(LockAfter::Minutes5),
        "nothing stored: the default"
    );
}

#[test]
fn a_longer_delay_on_a_pc_without_a_prompt_is_not_changed_and_says_why() {
    let server = server();
    let engine = signed_in(&server);
    let mut source = Source::new(engine.clone());
    opened(&mut source, Pane::Security);
    source.set_lock_after("60 min");
    settle(&mut source);
    assert_eq!(stored_after(&engine), Some(LockAfter::Minutes5));
    assert_eq!(
        source.notice(),
        Some("The lock delay was not changed: Windows Hello is not set up on this PC. Use your password instead.")
    );
    // A shorter one still works without it.
    source.set_lock_after("Immediately");
    settle(&mut source);
    assert_eq!(stored_after(&engine), Some(LockAfter::Immediately));
}

#[test]
fn a_stored_delay_the_menu_does_not_have_is_drawn_as_not_known() {
    let server = server();
    let credentials = MemoryCredentials::default();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "account-test-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: server.fake.base.clone(),
    };
    let engine = Arc::new(Engine::open(&TEST, config, host, credentials.clone()).expect("the engine opens"));
    engine.restore();
    sign_in(&engine, &server.people[0]);
    credentials
        .store_for("account-test-slot")
        .save_lock_flag(LockFlag::LockAfter, b"86400")
        .unwrap();
    let mut source = Source::new(engine.clone());
    opened(&mut source, Pane::Security);
    assert_eq!(source.security_facts().app_lock, Some((true, "Not known".to_string())));
}

/// Review of #452 (P3-3): the page's delay went stale (the engine's is shorter by now, written elsewhere): the engine
/// wants a proof for the change the page thought shorter, and the page asks Windows Hello for it, never saying that
/// Windows Hello did not confirm a prompt that was never shown.
#[test]
fn a_stale_delay_on_the_page_asks_windows_hello_when_the_engine_wants_a_proof() {
    let server = server();
    let (engine, mut source, prompts) = with_prompts(&server);
    engine
        .set_lock_after_for(
            engine.session_turn(),
            LockAfter::Minutes60,
            Some(AppLockProof::PlatformVerified),
        )
        .unwrap();
    source.closed();
    opened(&mut source, Pane::Security);
    assert_eq!(delay(&source).as_deref(), Some("60 min"));
    // Shorter meanwhile, from another place (the first run's step 2 on this PC).
    engine
        .set_lock_after_for(engine.session_turn(), LockAfter::Immediately, None)
        .unwrap();
    source.set_lock_after("30 min");
    // The engine's answer comes on a worker; Windows Hello is asked when it is taken in.
    let began = Instant::now();
    while prompts.asked() == 0 && source.pump().busy {
        assert!(began.elapsed() < Duration::from_secs(10), "an answer did not come back");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        prompts.asked(),
        1,
        "Windows Hello is asked for the proof the engine wants"
    );
    assert_eq!(source.notice(), None);
    prompts.answer(HelloResult::Verified);
    settle(&mut source);
    assert_eq!(stored_after(&engine), Some(LockAfter::Minutes30));
    assert_eq!(delay(&source).as_deref(), Some("30 min"));
}

/// L274, the cancel contract (`hello::HelloCancel`): a prompt taken away when the page went hands nothing to app lock,
/// not even a yes decided before its cancel that arrives late, also while a newer prompt is up under the same session
/// and account. The page is not left waiting for it, and the newer prompt's own answer still counts. (A prompt is up:
/// one `pump` takes the late answer in; `settle` would wait for the person.)
#[test]
fn no_answer_after_cancel_reaches_the_app_lock() {
    let server = server();
    let (engine, mut source, prompts) = with_prompts(&server);
    source.set_app_lock(false);
    assert_eq!(prompts.asked(), 1);
    source.closed();
    assert_eq!(prompts.taken_away(), 1, "the page went: its prompt is taken away");
    // The page comes back and works: nothing waits for the prompt that was taken away.
    opened(&mut source, Pane::Security);
    source.set_app_lock(false);
    assert_eq!(prompts.asked(), 2, "turning it off asks again");
    // The first prompt's yes arrives now, with the second prompt up under the same stamp.
    prompts.answer(HelloResult::Verified);
    source.pump();
    assert!(stored(&engine), "a yes of a prompt that was taken away writes nothing");
    assert_eq!(switch(&source), Some(true));
    assert_eq!(source.notice(), None, "and says nothing");
    assert_eq!(prompts.taken_away(), 1, "the second prompt stays up");
    // The second prompt is the one that counts.
    prompts.answer(HelloResult::Cancelled);
    settle(&mut source);
    assert!(stored(&engine));
    assert_eq!(
        source.notice(),
        Some("App lock is still on: Windows Hello did not confirm it's you.")
    );
    // A yes that arrives after the page went, with no prompt up any more, writes nothing either.
    source.set_app_lock(false);
    assert_eq!(prompts.asked(), 3);
    source.closed();
    prompts.answer(HelloResult::Verified);
    settle(&mut source);
    assert!(stored(&engine), "a yes after the page went writes nothing");
    assert_eq!(switch(&source), Some(true));
}

/// "Use your password" (no board frame: D-2).
#[path = "app_lock_password_tests.rs"]
mod password;
