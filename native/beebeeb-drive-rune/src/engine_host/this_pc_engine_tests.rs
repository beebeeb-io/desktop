//! This PC on the engine itself: `impl ThisPcEngine for Engine`, the glue the running app uses, against the engine's
//! fake accounts on loopback and a local state database of each test's own (the adapter's other tests use a stand-in
//! for the engine). Signed out, signed in, locked, a second account on a PC that holds the first one's local state,
//! and a change refused while sync does not run. Every name and address is invented.
//!
//! Not reached here: the pin change itself (`Engine::set_kept`), which needs sync running on a sync folder.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use beebeeb_engine::account_binding::Identity;
use beebeeb_engine::app::{Engine, EngineConfig};
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::state_db::{FileStatus, StateDb};
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use beebeeb_engine::test_api::{FakeApi, Handler, Request};
use zeroize::Zeroizing;

use super::tests::{file, now};
use super::*;

const PASSWORD: &str = "an invented password for this pc";

type TestEngine = Engine<MemoryCredentials>;

/// Two accounts on one fake: a sign-in leg goes to the account its email names, anything else to the account whose
/// token it carries.
struct Accounts {
    fake: FakeApi,
    first: Arc<FakeAccount>,
    second: Arc<FakeAccount>,
    phrases: [String; 2],
}

fn accounts(tag: &str) -> Accounts {
    let (first, one) = FakeAccount::register(&format!("{tag}-one@example.eu"), PASSWORD, TEST_PHRASE);
    let (second, two) = FakeAccount::register(&format!("{tag}-two@example.eu"), PASSWORD, TEST_PHRASE);
    let (first, second) = (Arc::new(first), Arc::new(second));
    let (a, b): (Handler, Handler) = (account_handler(first.clone()), account_handler(second.clone()));
    let (email, token) = (second.email.clone(), second.session_token.clone());
    let fake = FakeApi::start(Arc::new(move |req: &Request| {
        let theirs = if req.route().starts_with("POST /api/v1/opaque/") {
            req.json()
                .get("email")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|sent| sent.eq_ignore_ascii_case(&email))
        } else {
            req.bearer() == Some(token.as_str())
        };
        if theirs { b(req) } else { a(req) }
    }));
    Accounts {
        fake,
        first,
        second,
        phrases: [one, two],
    }
}

fn engine_at(base: &str) -> Arc<TestEngine> {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "this-pc-test-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: base.to_string(),
    };
    let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens");
    engine.restore();
    Arc::new(engine)
}

fn sign_in(engine: &TestEngine, account: &FakeAccount, phrase: &str) {
    engine
        .sign_in_password(&account.email, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(phrase.to_string()))
        .expect("the phrase unlocks");
}

/// A local state recorded as `owner`'s, with one kept file in it.
fn state_of(dir: &Path, owner: &FakeAccount) -> PathBuf {
    let path = dir.join("state.db");
    let db = StateDb::open(&path).unwrap();
    db.set_owner(&Identity::new(Some(&owner.user_id), Some(&owner.email)))
        .unwrap();
    file(
        &db,
        "f-invented",
        "Invented rates.xlsx",
        FileStatus::Local,
        80_000,
        false,
    );
    db.set_recursive_pin("f-invented", true, now()).unwrap();
    path
}

/// Pump as the window does until nothing is out (at most 10 s): the last refusal said on the way.
fn settle(source: &mut EngineThisPcSource<TestEngine>) -> Option<&'static str> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut said = None;
    loop {
        said = source.pump().refusal.or(said);
        if source.idle() {
            return source.pump().refusal.or(said);
        }
        assert!(Instant::now() < deadline, "the worker did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn the_engine_answers_for_the_signed_in_account_and_no_other() {
    let accounts = accounts("this-pc-glue");
    let engine = engine_at(&accounts.fake.base);
    let dir = tempfile::tempdir().unwrap();
    engine.use_local_state_for_test(state_of(dir.path(), &accounts.first));
    let mut source = EngineThisPcSource::new(engine.clone());

    // Nobody signed in: the engine names no account, hands out no local state, and the window holds nothing.
    assert!(!ThisPcEngine::unlocked(&*engine));
    assert_eq!(ThisPcEngine::account(&*engine), None);
    assert!(ThisPcEngine::local_state(&*engine, |_| ()).is_err());
    settle(&mut source);
    assert_eq!(source.kept_count(), 0);
    assert_eq!(source.standing(), Standing::Reading);

    // The first account, whose local state this is: its kept file shows.
    sign_in(&engine, &accounts.first, &accounts.phrases[0]);
    assert!(ThisPcEngine::unlocked(&*engine));
    assert_eq!(ThisPcEngine::session(&*engine), engine.session_turn());
    let signed_in_as = ThisPcEngine::account(&*engine).expect("an account is named");
    assert!(signed_in_as.eq_ignore_ascii_case(&accounts.first.email));
    settle(&mut source);
    assert_eq!(source.standing(), Standing::Ready);
    assert_eq!(source.kept().len(), 1);
    assert_eq!(source.kept()[0].id, "f-invented");

    // Sync does not run in this test: Remove from this PC is refused in the window's words, and nothing changes.
    assert!(!ThisPcEngine::syncing(&*engine));
    source.remove_from_pc("f-invented");
    assert_eq!(settle(&mut source), Some(NOT_SYNCING));
    assert_eq!(source.kept().len(), 1, "still kept");

    // A lock: no name at once (the engine's session moved on), before any pump.
    let before = engine.session_turn();
    let _ = engine.lock();
    assert_ne!(engine.session_turn(), before, "a lock is a new session");
    assert_eq!(source.kept_count(), 0);
    assert!(source.kept().is_empty() && source.history().is_empty());
    assert_eq!(source.standing(), Standing::Reading);
    settle(&mut source);
    assert!(source.held.is_none(), "dropped in memory too");

    // Another account signs in on this PC, which still holds the first one's local state: nothing of it is read.
    engine.sign_out_keeping_local_data().expect("signed out");
    sign_in(&engine, &accounts.second, &accounts.phrases[1]);
    settle(&mut source);
    assert_eq!(source.kept_count(), 0, "never the first account's kept file");
    assert!(source.history().is_empty());
    assert_eq!(source.standing(), Standing::AnotherAccounts);
    // Try again and Remove all have nothing to act on.
    source.retry_all();
    source.remove_all_from_pc();
    assert_eq!(settle(&mut source), None);
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    assert_eq!(db.kept_rows().unwrap().len(), 1, "the first account's pin is untouched");
}
