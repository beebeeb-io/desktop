//! A session-ending step counts BEFORE it waits for the engine's step lock: a host that took a decision in the turn
//! of now (About's "Restart to update" hand-off) sees that a step which will end that turn is on its way, also while
//! the step still waits behind another one (the hand-off's own stop of sync holds the lock for seconds). The steps
//! that run only with no session open count nothing. On a real sign-in against the engine's fake API; every name,
//! address and password is invented.

use std::sync::Arc;
use std::time::{Duration, Instant};

use zeroize::Zeroizing;

use super::super::{Engine, EngineConfig};
use super::LockState;
use crate::account::SessionGeneration;
use crate::host::ChannelHost;
use crate::identity::TEST;
use crate::signin::memory_store::MemoryCredentials;
use crate::test_api::FakeApi;
use crate::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};

type TestEngine = Engine<MemoryCredentials>;

const PASSWORD: &str = "an invented password for the session steps";

/// One registered account on a fake server of its own, and the 12 words of its master key.
fn account(tag: &str) -> (FakeApi, Arc<FakeAccount>, String) {
    let (account, phrase) = FakeAccount::register(&format!("{tag}@example.eu"), PASSWORD, TEST_PHRASE);
    let account = Arc::new(account);
    let api = FakeApi::start(account_handler(account.clone()));
    (api, account, phrase)
}

/// An engine of its own slot on `api`, nothing done yet.
fn opened(api: &FakeApi, slot: &str) -> Arc<TestEngine> {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: format!("session-steps-{slot}"),
        state_dir: crate::state_paths::init_for_test(),
        api_base: api.base.clone(),
    };
    Arc::new(Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens"))
}

/// Signs in with the password and the 12 words of the master key: the lock is open.
fn sign_in(engine: &TestEngine, account: &FakeAccount, phrase: &str) {
    engine
        .sign_in_password(&account.email, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(phrase.to_string()))
        .expect("the words unlock");
}

/// A session-ending step, as a call a thread makes in the turn the session was in.
type Step = fn(&TestEngine, SessionGeneration);

/// The five session-ending steps.
fn steps() -> [(&'static str, Step); 5] {
    [
        ("close_session_the_server_refused", |engine, turn| {
            let _ = engine.close_session_the_server_refused(Some(turn));
        }),
        ("lock_vault", |engine, _| {
            let _ = engine.lock_vault();
        }),
        ("sign_out_for", |engine, turn| {
            let _ = engine.sign_out_for(turn);
        }),
        ("sign_out_keeping_local_data", |engine, _| {
            let _ = engine.sign_out_keeping_local_data();
        }),
        ("discard_partial_sign_in", |engine, _| {
            let _ = engine.discard_partial_sign_in();
        }),
    ]
}

/// Each of the five: while a hold of the step lock keeps it waiting, the count has moved by exactly one and the
/// session's turn has not moved yet. The step then finishes once the hold lets go (leaving a half-done sign-in on an
/// open vault ends nothing: it counts all the same, as it may end a keyless session).
#[test]
fn a_session_ending_step_counts_before_it_waits() {
    let (api, account, phrase) = account("session-steps-count");
    for (name, step) in steps() {
        let engine = opened(&api, name);
        engine.restore();
        sign_in(&engine, &account, &phrase);
        assert_eq!(engine.lock_state(), LockState::Open, "{name}: signed in and open");
        let (turn, before) = (engine.session_turn(), engine.session_steps_asked());

        let hold = engine.hold_session_step_for_test();
        let asker = engine.clone();
        let call = std::thread::spawn(move || step(&asker, turn));
        let began = Instant::now();
        while engine.session_steps_asked() == before {
            assert!(
                began.elapsed() < Duration::from_secs(2),
                "{name}: the step did not count while it waited for the step lock"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(
            engine.session_steps_asked(),
            before + 1,
            "{name}: counted once, while it waits"
        );
        assert_eq!(
            engine.session_turn(),
            turn,
            "{name}: the turn has not moved yet: the step still waits"
        );
        assert!(!call.is_finished(), "{name}: the step waits for the hold");
        drop(hold);
        call.join().expect("the step's thread");
        assert_eq!(engine.session_steps_asked(), before + 1, "{name}: once in all");
    }
}

/// The start's restore, a password sign-in and a master-key sign-in run while no session is open: none of them is a
/// session-ending step, so a hand-off pressed before cannot be refused by them through the count.
#[test]
fn a_password_sign_in_counts_no_session_step() {
    let (api, account, phrase) = account("session-steps-signin");
    let engine = opened(&api, "signin");
    let before = engine.session_steps_asked();
    engine.restore();
    sign_in(&engine, &account, &phrase);
    assert_eq!(engine.lock_state(), LockState::Open);
    assert_eq!(
        engine.session_steps_asked(),
        before,
        "a sign-in asks no session-ending step"
    );
}
