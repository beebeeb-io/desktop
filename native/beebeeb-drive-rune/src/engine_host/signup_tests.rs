//! Creating an account on the real engine (`EngineSignInSource`'s sign-up half), against the engine's fake server
//! (`test_api::fake_signup`: the onboarding document, the email code, the breach range, a real OPAQUE registration,
//! the profile, the terms, the logout).
//!
//! Each test drives the source as the screens do (`begin_sign_up_*`, then `poll`) and reads both the event the
//! screen gets and what the engine and the fake server hold afterwards. Every address, code, password and word is
//! invented. Engine handles are on the process's state folder, which nothing here writes in.

use std::sync::atomic::Ordering;
use std::sync::{Condvar, Mutex};
use std::time::Instant;

use beebeeb_engine::app::EngineConfig;
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::test_api::fake_signup::{
    self as fake_signup, CODE, FakeSignup, OPEN, WEB_ONLY_CALLS, WEB_ONLY_DOCUMENT, signup_handler,
};
use beebeeb_engine::test_api::{FakeApi, Handler, Request};

use super::*;
use crate::frames::signin::create_source::CONFIRM_WORDS;
use crate::frames::signin::source::{GateState, SignInSource};

const EMAIL: &str = "New.Person@Example.eu";
const PASSWORD: &str = "an invented sign-up password";

type Source = EngineSignInSource<MemoryCredentials>;

struct Server {
    fake: FakeApi,
    state: Arc<FakeSignup>,
}

/// A request waits at this door until the test opens it: a step that is on its way, without any timing.
#[derive(Default)]
struct Door {
    open: Mutex<bool>,
    opened: Condvar,
}

impl Door {
    fn pass(&self) {
        let mut open = self.open.lock().unwrap();
        while !*open {
            open = self.opened.wait(open).unwrap();
        }
    }

    fn open(&self) {
        *self.open.lock().unwrap() = true;
        self.opened.notify_all();
    }
}

fn server(mode: u8) -> Server {
    let state = Arc::new(FakeSignup::default());
    state.mode.store(mode, Ordering::SeqCst);
    let fake = FakeApi::start(signup_handler(state.clone()));
    Server { fake, state }
}

/// A server whose email-start waits at `door` before it answers.
fn server_with_door(door: Arc<Door>) -> Server {
    let state = Arc::new(FakeSignup::default());
    let inner: Handler = signup_handler(state.clone());
    let fake = FakeApi::start(Arc::new(move |req: &Request| {
        if req.route() == "POST /api/v1/auth/signup/email-start" {
            door.pass();
        }
        inner(req)
    }));
    Server { fake, state }
}

fn source_on(server: &Server) -> (Arc<Engine<MemoryCredentials>>, Source, MemoryCredentials) {
    let store = MemoryCredentials::default();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "signup-test-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: server.fake.base.clone(),
    };
    let engine = Engine::open(&TEST, config, host, store.clone()).expect("the engine opens");
    engine.restore();
    let engine = Arc::new(engine);
    let source = EngineSignInSource::with_tick(engine.clone(), Duration::from_millis(2));
    (engine, source, store)
}

/// The next answer the screen gets, the wait's ticks left out.
fn next(source: &mut Source) -> SignUpEvent {
    let began = Instant::now();
    loop {
        match source.poll() {
            Some(SignInEvent::SignUp(SignUpEvent::ResendTick { .. })) => {}
            Some(SignInEvent::SignUp(event)) => return event,
            Some(other) => panic!("not a sign-up answer: {other:?}"),
            None => {}
        }
        assert!(began.elapsed() < Duration::from_secs(60), "no answer came");
        thread::sleep(Duration::from_millis(5));
    }
}

fn requests(server: &Server, route: &str) -> usize {
    server.fake.routes().iter().filter(|seen| seen.ends_with(route)).count()
}

fn lent_words(source: &Source) -> Vec<String> {
    let mut words = Vec::new();
    source.with_sign_up_phrase(&mut |lent: &[&str]| words = lent.iter().map(|w| w.to_string()).collect());
    words
}

/// The email, the code and a password, up to the phrase being shown.
fn to_the_phrase(source: &mut Source) {
    source.begin_sign_up_code(EMAIL);
    assert!(matches!(next(source), SignUpEvent::CodeSent(Ok(_))));
    source.begin_sign_up_verify(CODE);
    assert_eq!(next(source), SignUpEvent::Verified(Ok(())));
    source.begin_sign_up_password(PASSWORD);
    assert_eq!(next(source), SignUpEvent::PasswordChecked(Ok(())));
}

/// The three words asked for, typed back correctly.
fn right_words(source: &Source) -> Vec<String> {
    let phrase = lent_words(source);
    assert_eq!(phrase.len(), 12);
    let positions = source.sign_up_confirm_positions().expect("the positions are known");
    positions.iter().map(|at| phrase[*at].clone()).collect()
}

fn typed(words: &[String]) -> Vec<&str> {
    words.iter().map(String::as_str).collect()
}

#[test]
fn an_account_is_created_and_signed_in_through_every_step_of_the_screens() {
    let server = server(OPEN);
    let (engine, mut source, store) = source_on(&server);
    assert!(source.sign_up_available());
    assert_eq!(source.gate(), GateState::SignedOut);

    // The email: the server's numbers come back, the wait for a new code counts down.
    source.begin_sign_up_code(EMAIL);
    assert_eq!(
        next(&mut source),
        SignUpEvent::CodeSent(Ok(CodeSent {
            digits: 8,
            resend_seconds: 60
        }))
    );
    // The code: a wrong one is said as that, the right one verifies.
    source.begin_sign_up_verify("00000000");
    assert_eq!(
        next(&mut source),
        SignUpEvent::Verified(Err(CodeProblem::WrongOrExpired))
    );
    source.begin_sign_up_verify(CODE);
    assert_eq!(next(&mut source), SignUpEvent::Verified(Ok(())));

    // The password: judged by core's evaluator with the server's minimum, as it is typed.
    let short = source.sign_up_password("short one").expect("a verdict");
    assert!(!short.long_enough);
    assert_eq!(short.bars, 1);
    assert_eq!(short.words, "Needs at least 12 characters, 3 more.");
    let strong = source.sign_up_password("Abcdefghij-12345").expect("a verdict");
    assert!(strong.long_enough);
    assert_eq!((strong.bars, strong.words.as_str()), (4, "Strong."));
    assert_eq!(
        source.sign_up_confirm_positions(),
        None,
        "no phrase before the password"
    );
    assert!(lent_words(&source).is_empty(), "no phrase before the password");

    source.begin_sign_up_password(PASSWORD);
    assert_eq!(next(&mut source), SignUpEvent::PasswordChecked(Ok(())));
    let positions = source.sign_up_confirm_positions().expect("three positions");
    assert_eq!(positions.len(), CONFIRM_WORDS);
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]) && positions[2] < 12);
    let phrase = lent_words(&source);
    assert_eq!(phrase.len(), 12, "twelve words are lent while the engine holds them");
    assert_eq!(lent_words(&source), phrase, "the phrase does not change between draws");

    // Words typed back: a wrong one says no (and not which), the right ones say yes.
    let wrong = vec!["zzzz".to_string(); CONFIRM_WORDS];
    source.begin_sign_up_confirm(&typed(&wrong));
    assert_eq!(next(&mut source), SignUpEvent::PhraseConfirmed(false));
    let right = right_words(&source);
    source.begin_sign_up_confirm(&typed(&right));
    assert_eq!(next(&mut source), SignUpEvent::PhraseConfirmed(true));
    assert!(
        lent_words(&source).is_empty(),
        "the phrase is wiped once it is confirmed"
    );
    assert_eq!(source.sign_up_confirm_positions(), None);

    // Create: the account exists at the server, and this computer is signed in to it with its key.
    assert!(!source.sign_up_device_name().is_empty());
    source.begin_sign_up_create("A computer of mine");
    assert_eq!(next(&mut source), SignUpEvent::Created(Ok(())));
    assert_eq!(source.gate(), GateState::Ready);
    assert_eq!(engine.account_email().as_deref(), Some("new.person@example.eu"));
    assert!(
        server.state.registered.lock().unwrap().is_some(),
        "the OPAQUE registration finished"
    );
    assert_eq!(server.state.terms_accepted.load(Ordering::SeqCst), 1);
    assert!(store.len() >= 2, "the session and the key are in the credential store");
    assert_eq!(requests(&server, "/auth/logout"), 0);
}

#[test]
fn the_wait_for_a_new_code_counts_down_to_zero_once_a_second() {
    let server = server(OPEN);
    let (_engine, mut source, _store) = source_on(&server);
    source.begin_sign_up_code(EMAIL);
    let began = Instant::now();
    let mut seconds = Vec::new();
    let mut sent = false;
    while seconds.last() != Some(&0) {
        match source.poll() {
            Some(SignInEvent::SignUp(SignUpEvent::CodeSent(Ok(_)))) => sent = true,
            Some(SignInEvent::SignUp(SignUpEvent::ResendTick { seconds: left })) => seconds.push(left),
            _ => thread::sleep(Duration::from_millis(2)),
        }
        assert!(began.elapsed() < Duration::from_secs(20), "the wait never ended");
    }
    assert!(sent, "the answer came before its first tick");
    assert_eq!(seconds, (0..60).rev().collect::<Vec<u32>>());
}

#[test]
fn a_server_that_refuses_sign_up_from_this_app_is_said_with_its_page_and_nothing_is_sent_or_stored() {
    // The onboarding document says it (what the server answers today).
    let server = server(WEB_ONLY_DOCUMENT);
    let (_engine, mut source, store) = source_on(&server);
    assert_eq!(source.sign_up_web_address(), None);
    source.begin_sign_up_code(EMAIL);
    assert_eq!(next(&mut source), SignUpEvent::CodeSent(Err(SendProblem::WebOnly)));
    assert_eq!(
        source.sign_up_web_address().as_deref(),
        Some("https://app.beebeeb.io/signup")
    );
    assert_eq!(
        requests(&server, "/signup/email-start"),
        0,
        "no email was sent for an address"
    );
    assert!(store.is_empty());
    assert_eq!(source.gate(), GateState::SignedOut);

    // The create call itself answers 403 `signup_web_only`.
    let server = self::server(WEB_ONLY_CALLS);
    let (_engine, mut source, store) = source_on(&server);
    source.begin_sign_up_code(EMAIL);
    assert_eq!(next(&mut source), SignUpEvent::CodeSent(Err(SendProblem::WebOnly)));
    assert_eq!(
        source.sign_up_web_address().as_deref(),
        Some("https://app.beebeeb.io/signup")
    );
    assert_eq!(requests(&server, "/signup/email-start"), 1);
    assert!(store.is_empty());
    // Leaving forgets the address.
    source.cancel_sign_up();
    source.begin_sign_up_verify(CODE);
    assert_eq!(next(&mut source), SignUpEvent::Verified(Err(CodeProblem::Other)));
    assert_eq!(source.sign_up_web_address(), None);
}

#[test]
fn a_password_from_a_breach_is_refused_and_no_phrase_is_made() {
    let server = server(OPEN);
    *server.state.breached_password.lock().unwrap() = Some(PASSWORD.to_string());
    let (_engine, mut source, _store) = source_on(&server);
    source.begin_sign_up_code(EMAIL);
    assert!(matches!(next(&mut source), SignUpEvent::CodeSent(Ok(_))));
    source.begin_sign_up_verify(CODE);
    assert_eq!(next(&mut source), SignUpEvent::Verified(Ok(())));
    source.begin_sign_up_password(PASSWORD);
    assert_eq!(
        next(&mut source),
        SignUpEvent::PasswordChecked(Err(PasswordProblem::Breached))
    );
    assert_eq!(source.sign_up_confirm_positions(), None);
    assert!(lent_words(&source).is_empty());
    // Another password passes the same check.
    source.begin_sign_up_password("another invented sign-up password");
    assert_eq!(next(&mut source), SignUpEvent::PasswordChecked(Ok(())));
}

#[test]
fn a_breach_check_that_cannot_be_made_lets_nobody_through() {
    let server = server(OPEN);
    server.state.breach_down.store(true, Ordering::SeqCst);
    let (_engine, mut source, _store) = source_on(&server);
    source.begin_sign_up_code(EMAIL);
    assert!(matches!(next(&mut source), SignUpEvent::CodeSent(Ok(_))));
    source.begin_sign_up_verify(CODE);
    assert_eq!(next(&mut source), SignUpEvent::Verified(Ok(())));
    source.begin_sign_up_password(PASSWORD);
    assert_eq!(
        next(&mut source),
        SignUpEvent::PasswordChecked(Err(PasswordProblem::CheckUnreachable))
    );
    assert_eq!(source.sign_up_confirm_positions(), None);
}

#[test]
fn leaving_sign_up_drops_the_answer_that_was_on_its_way_and_wipes_the_attempt() {
    let door = Arc::new(Door::default());
    let server = server_with_door(door.clone());
    let (_engine, mut source, _store) = source_on(&server);
    source.begin_sign_up_code(EMAIL);
    // The request is at the server; the person leaves.
    let began = Instant::now();
    while requests(&server, "/signup/email-start") == 0 {
        assert!(began.elapsed() < Duration::from_secs(20), "the request never arrived");
        thread::sleep(Duration::from_millis(2));
    }
    source.cancel_sign_up();
    door.open();
    // The next step asked is answered; the answer of the left one never is, and the attempt holds nothing.
    source.begin_sign_up_verify(CODE);
    assert_eq!(next(&mut source), SignUpEvent::Verified(Err(CodeProblem::Other)));
    assert_eq!(
        requests(&server, "/signup/email-verify"),
        0,
        "nothing of the left attempt was sent on"
    );
    assert_eq!(source.poll(), None, "no tick and no answer of the step that was left");
}

#[test]
fn the_account_exists_but_this_computer_could_not_sign_in_to_it_says_so_and_ends_the_session() {
    let server = server(OPEN);
    server.state.profile_fails.store(true, Ordering::SeqCst);
    let (_engine, mut source, store) = source_on(&server);
    to_the_phrase(&mut source);
    let right = right_words(&source);
    source.begin_sign_up_confirm(&typed(&right));
    assert_eq!(next(&mut source), SignUpEvent::PhraseConfirmed(true));
    source.begin_sign_up_create("A computer of mine");
    assert_eq!(
        next(&mut source),
        SignUpEvent::Created(Err(CreateProblem::NotFinishedHere))
    );
    assert!(server.state.registered.lock().unwrap().is_some(), "the account exists");
    assert_eq!(
        requests(&server, "/auth/logout"),
        1,
        "the session the server made was ended"
    );
    assert!(store.is_empty(), "nothing of the account is on this computer");
    assert_eq!(source.gate(), GateState::SignedOut);
}

#[test]
fn a_registration_the_server_refuses_stores_nothing_and_can_be_tried_again() {
    let server = server(OPEN);
    server.state.finish_fails.store(true, Ordering::SeqCst);
    let (_engine, mut source, store) = source_on(&server);
    to_the_phrase(&mut source);
    let right = right_words(&source);
    source.begin_sign_up_confirm(&typed(&right));
    assert_eq!(next(&mut source), SignUpEvent::PhraseConfirmed(true));
    source.begin_sign_up_create("A computer of mine");
    assert_eq!(
        next(&mut source),
        SignUpEvent::Created(Err(CreateProblem::NothingStored))
    );
    assert!(store.is_empty());
    assert!(server.state.registered.lock().unwrap().is_none());
    assert_eq!(source.gate(), GateState::SignedOut);

    // The retry: the password and the phrase were kept.
    server.state.finish_fails.store(false, Ordering::SeqCst);
    source.begin_sign_up_create("A computer of mine");
    assert_eq!(next(&mut source), SignUpEvent::Created(Ok(())));
    assert_eq!(source.gate(), GateState::Ready);
}

#[test]
fn what_is_typed_is_completed_from_the_public_word_list_and_nothing_is_offered_for_nothing() {
    let server = server(OPEN);
    let (_engine, source, _store) = source_on(&server);
    assert!(source.sign_up_suggestions("").is_empty());
    let offered = source.sign_up_suggestions("thun");
    assert!(offered.contains(&"thunder") && offered.iter().all(|word| word.starts_with("thun")));
    assert!(source.sign_up_suggestions("qqqq").is_empty());
    assert_eq!(fake_signup::USER_ID, "an-invented-new-user");
}
