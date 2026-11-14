//! The first-time guide's mark on the real engine (`EngineGuideMarks`), against the engine's fake accounts on
//! loopback and a local state database of each test's own: done once per account, kept across a sign-out and a
//! sign-in, never another account's, held and asked again when it cannot be kept yet, and never taken in for a
//! session that ended. Every name and address is invented.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use beebeeb_engine::account_binding::Identity;
use beebeeb_engine::app::{Engine, EngineConfig};
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::state_db::StateDb;
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use beebeeb_engine::test_api::{FakeApi, Handler, Request};
use zeroize::Zeroizing;

use super::*;

const PASSWORD: &str = "an invented password for the guide";

/// Two accounts on one fake: a sign-in leg goes to the account its email names, anything else to the account whose
/// token it carries.
struct Fake {
    fake: FakeApi,
    first: Arc<FakeAccount>,
    second: Arc<FakeAccount>,
    phrases: [String; 2],
}

fn fake(tag: &str) -> Fake {
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
    Fake {
        fake,
        first,
        second,
        phrases: [one, two],
    }
}

type TestEngine = Engine<MemoryCredentials>;
type Marks = EngineGuideMarks<MemoryCredentials>;

fn engine_at(base: &str) -> Arc<TestEngine> {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "guide-test-slot".into(),
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

/// A local state recorded as `owner`'s.
fn state_of(dir: &Path, owner: &FakeAccount) -> PathBuf {
    let path = dir.join("state.db");
    StateDb::open(&path)
        .unwrap()
        .set_owner(&Identity::new(Some(&owner.user_id), Some(&owner.email)))
        .unwrap();
    path
}

/// Ask, and poll as the window does until the answer is there (at most 10 s).
fn asked(marks: &mut Marks) -> Option<Known> {
    marks.ask(None);
    let began = Instant::now();
    loop {
        if let Some(known) = marks.answer(None) {
            return Some(known);
        }
        if began.elapsed() > Duration::from_secs(10) {
            return None;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Poll until no mark is on its way.
fn settled(marks: &mut Marks) {
    let began = Instant::now();
    loop {
        let _ = marks.answer(None);
        if !marks.busy() {
            return;
        }
        assert!(began.elapsed() < Duration::from_secs(10), "the mark did not answer");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn done_once_per_account_across_a_sign_out_and_never_another_accounts() {
    let fake = fake("guide-once");
    let engine = engine_at(&fake.fake.base);
    let dir = tempfile::tempdir().unwrap();
    engine.use_local_state_for_test(state_of(dir.path(), &fake.first));
    let mut marks = Marks::new(engine.clone());

    // Nobody signed in: nothing is asked, nothing is known, and nothing is kept.
    marks.ask(None);
    assert_eq!(marks.answer(None), None);
    marks.mark_done(None);
    assert!(!marks.owes() && !marks.busy(), "no mark without a session");

    sign_in(&engine, &fake.first, &fake.phrases[0]);
    assert_eq!(asked(&mut marks), Some(Known::NotDone));
    marks.mark_done(None);
    assert!(marks.busy(), "the mark is on its way");
    settled(&mut marks);
    assert!(!marks.owes(), "kept");
    assert_eq!(asked(&mut marks), Some(Known::Done));

    // A sign-out and a sign-in of the same account, and a window started anew (a new source): still done.
    engine.sign_out_keeping_local_data().expect("signed out");
    sign_in(&engine, &fake.first, &fake.phrases[0]);
    assert_eq!(asked(&mut marks), Some(Known::Done));
    assert_eq!(asked(&mut Marks::new(engine.clone())), Some(Known::Done));

    // Another account on this PC: the state is not its own, so nothing is known and no guide starts by itself;
    // and its mark is not written into the first account's state.
    engine.sign_out_keeping_local_data().expect("signed out");
    sign_in(&engine, &fake.second, &fake.phrases[1]);
    assert_eq!(asked(&mut marks), Some(Known::NotKnown));
    marks.mark_done(None);
    settled(&mut marks);
    assert!(
        !marks.owes(),
        "it cannot be kept in another account's state, now or later: it is not held"
    );
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    assert!(db.guide_done(&fake.first.user_id).unwrap());
    assert!(
        !db.guide_done(&fake.second.user_id).unwrap(),
        "nothing of the second account was written into the first one's state"
    );
}

#[test]
fn a_mark_that_cannot_be_kept_yet_is_held_for_the_account_and_asked_again() {
    let fake = fake("guide-held");
    let engine = engine_at(&fake.fake.base);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    engine.use_local_state_for_test(path.clone());
    let mut marks = Marks::new(engine.clone());
    sign_in(&engine, &fake.first, &fake.phrases[0]);

    // Nothing has synced on this PC: not done, and the mark has nowhere to go yet.
    assert_eq!(asked(&mut marks), Some(Known::NotDone));
    marks.mark_done(None);
    settled(&mut marks);
    assert!(marks.owes(), "held");
    assert!(!path.exists(), "no database is made for the mark");
    assert_eq!(
        asked(&mut marks),
        Some(Known::Done),
        "in this run the account is done: the guide does not start again"
    );
    // Before the wait is over nothing is tried.
    let _ = marks.answer(None);
    assert!(!marks.busy(), "not tried again before the wait is over");

    // Another account in between is not handed the first one's held mark.
    engine.sign_out_keeping_local_data().expect("signed out");
    sign_in(&engine, &fake.second, &fake.phrases[1]);
    assert_eq!(
        asked(&mut marks),
        Some(Known::NotDone),
        "the second account is not done"
    );
    marks.wait_is_over();
    let _ = marks.answer(None);
    assert!(
        !marks.busy(),
        "the first account's mark is not sent under the second account's session"
    );
    // The second account's own mark is held beside the first one's.
    marks.mark_done(None);
    settled(&mut marks);
    assert_eq!(
        asked(&mut marks),
        Some(Known::Done),
        "the second account is done in this run"
    );
    engine.sign_out_keeping_local_data().expect("signed out");

    // The first account again, and by now its state is there: the held mark is kept on the next try.
    state_of(dir.path(), &fake.first);
    sign_in(&engine, &fake.first, &fake.phrases[0]);
    marks.wait_is_over();
    let _ = marks.answer(None);
    assert!(marks.busy(), "tried again once the wait is over");
    settled(&mut marks);
    assert!(marks.owes(), "the second account's is still held");
    assert_eq!(
        asked(&mut marks),
        Some(Known::Done),
        "the first account's mark was not lost to the second one's"
    );
    assert!(StateDb::open(&path).unwrap().guide_done(&fake.first.user_id).unwrap());
    assert!(!StateDb::open(&path).unwrap().guide_done(&fake.second.user_id).unwrap());
    assert_eq!(asked(&mut Marks::new(engine.clone())), Some(Known::Done));
}

#[test]
fn an_answer_for_a_session_that_ended_is_never_taken_in() {
    let fake = fake("guide-late");
    let engine = engine_at(&fake.fake.base);
    let dir = tempfile::tempdir().unwrap();
    engine.use_local_state_for_test(state_of(dir.path(), &fake.first));
    let mut marks = Marks::new(engine.clone());
    sign_in(&engine, &fake.first, &fake.phrases[0]);
    assert_eq!(asked(&mut marks), Some(Known::NotDone), "the control: it answers");

    // Asked, and the session ends before the answer is taken in: a lock and an unlock of the same account.
    marks.ask(None);
    std::thread::sleep(Duration::from_millis(300));
    let _ = engine.lock();
    engine
        .unlock_with_phrase(Zeroizing::new(fake.phrases[0].clone()))
        .expect("the phrase unlocks");
    assert_eq!(
        marks.answer(None),
        None,
        "an answer read under the session before is not this session's"
    );
    assert_eq!(
        asked(&mut marks),
        Some(Known::NotDone),
        "asked again under this session, it answers"
    );
}
