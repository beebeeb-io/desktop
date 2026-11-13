//! Sign-in on the real engine (`EngineSignInSource`), against the engine's fake accounts on loopback, with the
//! routes the fake account does not have answered here in the server's shape: the code step
//! (`/auth/2fa/verify`) and the step-up of a locked vault's password (`/auth/confirm-opaque-*`).
//!
//! Each test drives the source as the screens do (`begin_*`, then `poll`) and reads both the event the screen gets
//! and what the engine holds afterwards. Every account, password, code and phrase is invented. Engine handles are
//! on the process's state folder, which nothing here writes in.

use std::sync::atomic::{AtomicBool, AtomicU8};
use std::sync::{Condvar, Mutex};
use std::time::Instant;

use beebeeb_engine::app::EngineConfig;
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::gate::GateState as EngineGate;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use beebeeb_engine::test_api::{FakeApi, Handler, Reply, Request};
use serde_json::{Value, json};

use super::*;
use crate::frames::signin::source::TwoFactorProblem;

const PASSWORD: &str = "an invented password for sign-in";
const TOTP: &str = "482019";
const BACKUP: &str = "48201395";
/// A valid phrase that is not the accounts' (BIP-39's second test vector).
const OTHER_PHRASE: &str = "legal winner thank year wave sausage worth useful legal winner thank yellow";

const ANSWERS: u8 = 0;
const RATE_LIMITED: u8 = 1;
const NO_ANSWER: u8 = 2;

type Source = EngineSignInSource<MemoryCredentials>;

/// A door a request waits at until the test opens it: a step that is on its way, without any timing.
#[derive(Default)]
struct Door {
    open: Mutex<bool>,
    opened: Condvar,
}

impl Door {
    fn shut(&self) {
        *self.open.lock().unwrap() = false;
    }

    fn open(&self) {
        *self.open.lock().unwrap() = true;
        self.opened.notify_all();
    }

    fn pass(&self) {
        let mut open = self.open.lock().unwrap();
        while !*open {
            open = self.opened.wait(open).unwrap();
        }
    }
}

/// Two accounts on one fake. A sign-in leg goes to the account its email names (an address nobody has is refused
/// as a wrong password is); anything else to the account whose token it carries.
struct Fake {
    fake: FakeApi,
    first: Arc<FakeAccount>,
    second: Arc<FakeAccount>,
    /// The first account asks for a code after its password.
    two_factor: Arc<AtomicBool>,
    /// How the password legs, the code step and the step-up answer.
    sign_in: Arc<AtomicU8>,
    code: Arc<AtomicU8>,
    step_up: Arc<AtomicU8>,
    /// The code step waits here before it answers.
    code_door: Arc<Door>,
}

fn email_of(req: &Request) -> String {
    req.json()
        .get("email")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_lowercase()
}

fn refused(mode: u8) -> Option<Reply> {
    match mode {
        RATE_LIMITED => Some(Reply::error(429, "rate_limited")),
        NO_ANSWER => Some(Reply::hang_up()),
        _ => None,
    }
}

fn fake(tag: &str) -> Fake {
    let (first, _) = FakeAccount::register(&format!("{tag}-one@example.eu"), PASSWORD, TEST_PHRASE);
    let (second, _) = FakeAccount::register(&format!("{tag}-two@example.eu"), PASSWORD, TEST_PHRASE);
    let (first, second) = (Arc::new(first), Arc::new(second));
    let (one, two): (Handler, Handler) = (account_handler(first.clone()), account_handler(second.clone()));
    let (two_factor, sign_in, code, step_up, code_door) = (
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicU8::new(ANSWERS)),
        Arc::new(AtomicU8::new(ANSWERS)),
        Arc::new(AtomicU8::new(ANSWERS)),
        Arc::new(Door::default()),
    );
    code_door.open();
    let (first_in, second_in) = (first.clone(), second.clone());
    let (two_factor_in, sign_in_in, code_in, step_up_in, door_in) = (
        two_factor.clone(),
        sign_in.clone(),
        code.clone(),
        step_up.clone(),
        code_door.clone(),
    );
    let fake = FakeApi::start(Arc::new(move |req: &Request| {
        let route = req.route();
        match route.as_str() {
            "POST /api/v1/opaque/login-start" | "POST /api/v1/opaque/login-finish" => {
                if let Some(reply) = refused(sign_in_in.load(Ordering::SeqCst)) {
                    return reply;
                }
                let email = email_of(req);
                let firsts = email == first_in.email;
                if !firsts && email != second_in.email {
                    // Nobody's address: answered as a wrong password, at the finish leg.
                    return if route.ends_with("start") {
                        one(req)
                    } else {
                        Reply::json(401, json!({}))
                    };
                }
                let reply = if firsts { one(req) } else { two(req) };
                let proven = route.ends_with("finish") && reply.status == 200;
                if proven && firsts && two_factor_in.load(Ordering::SeqCst) {
                    return Reply::ok(json!({ "requires_2fa": true, "partial_token": "an-invented-partial" }));
                }
                reply
            }
            "POST /api/v1/auth/2fa/verify" => {
                door_in.pass();
                if let Some(reply) = refused(code_in.load(Ordering::SeqCst)) {
                    return reply;
                }
                let body = req.json();
                let right =
                    body["partial_token"] == "an-invented-partial" && (body["code"] == TOTP || body["code"] == BACKUP);
                if right {
                    Reply::ok(json!({ "session_token": first_in.session_token }))
                } else {
                    Reply::error(401, "invalid_code")
                }
            }
            "POST /api/v1/auth/confirm-opaque-start" | "POST /api/v1/auth/confirm-opaque-finish" => {
                if let Some(reply) = refused(step_up_in.load(Ordering::SeqCst)) {
                    return reply;
                }
                let account = if req.bearer() == Some(second_in.session_token.as_str()) {
                    &second_in
                } else if req.bearer() == Some(first_in.session_token.as_str()) {
                    &first_in
                } else {
                    return Reply::error(401, "unauthorized");
                };
                if route.ends_with("start") {
                    account.opaque.login_start(&req.json())
                } else if account.opaque.login_finish(&req.json()) {
                    Reply::ok(
                        json!({ "confirmation_token": "an-invented-grant", "expires_at": "2026-01-01T00:05:00Z" }),
                    )
                } else {
                    Reply::error(401, "invalid_credentials")
                }
            }
            _ if req.bearer() == Some(second_in.session_token.as_str()) => two(req),
            _ => one(req),
        }
    }));
    Fake {
        fake,
        first,
        second,
        two_factor,
        sign_in,
        code,
        step_up,
        code_door,
    }
}

fn engine_at(base: &str) -> Arc<Engine<MemoryCredentials>> {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "signin-test-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: base.to_string(),
    };
    let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens");
    engine.restore();
    Arc::new(engine)
}

/// A source on a fresh signed-out handle, counting its wait down fast.
fn source(fake: &Fake) -> (Arc<Engine<MemoryCredentials>>, Source) {
    let engine = engine_at(&fake.fake.base);
    let source = EngineSignInSource::with_tick(engine.clone(), Duration::from_millis(2));
    (engine, source)
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let began = Instant::now();
    while !done() {
        assert!(
            began.elapsed() < Duration::from_secs(20),
            "timed out waiting for {what}"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

/// The next answer the screen gets.
fn next(source: &mut Source) -> SignInEvent {
    let began = Instant::now();
    loop {
        if let Some(event) = source.poll() {
            return event;
        }
        assert!(began.elapsed() < Duration::from_secs(20), "no answer came");
        thread::sleep(Duration::from_millis(5));
    }
}

fn requests(fake: &Fake, route: &str) -> usize {
    fake.fake.routes().iter().filter(|seen| seen.ends_with(route)).count()
}

fn words(phrase: &str) -> Vec<&str> {
    phrase.split(' ').collect()
}

/// Signed in as the first account and unlocked with the twelve words, through the source.
fn signed_in(fake: &Fake) -> (Arc<Engine<MemoryCredentials>>, Source) {
    let (engine, mut source) = source(fake);
    source.begin_sign_in(&fake.first.email, PASSWORD);
    assert_eq!(
        next(&mut source),
        SignInEvent::SignedIn(Ok(NextAfterSignIn::RecoveryPhrase))
    );
    source.begin_restore(&words(TEST_PHRASE));
    assert_eq!(
        next(&mut source),
        SignInEvent::PhraseRestored(Ok(NextAfterSignIn::Drive))
    );
    assert_eq!(source.gate(), GateState::Ready);
    (engine, source)
}

#[test]
fn the_password_the_code_and_the_twelve_words_sign_in_and_unlock() {
    let fake = fake("flow");
    fake.two_factor.store(true, Ordering::SeqCst);
    let (engine, mut source) = source(&fake);
    assert_eq!(source.gate(), GateState::SignedOut);

    source.begin_sign_in(&fake.first.email, PASSWORD);
    assert_eq!(next(&mut source), SignInEvent::SignedIn(Ok(NextAfterSignIn::TwoFactor)));
    assert_eq!(
        source.gate(),
        GateState::SignedOut,
        "nothing is signed in before the code"
    );

    // A wrong code: said as that, and the step stays.
    source.begin_two_factor("000000", CodeKind::Authenticator);
    assert_eq!(
        next(&mut source),
        SignInEvent::TwoFactor(Err(TwoFactorProblem::WrongCode))
    );
    assert_eq!(source.gate(), GateState::SignedOut);

    // The code: signed in, and this computer holds no key yet.
    source.begin_two_factor(TOTP, CodeKind::Authenticator);
    assert_eq!(
        next(&mut source),
        SignInEvent::TwoFactor(Ok(NextAfterSignIn::RecoveryPhrase))
    );
    assert_eq!(source.gate(), GateState::NeedsRecoveryPhrase);

    // Twelve valid words of another account: not this one's, nothing restored.
    source.begin_restore(&words(OTHER_PHRASE));
    assert_eq!(
        next(&mut source),
        SignInEvent::PhraseRestored(Err(PhraseProblem::WrongPhrase { bad_word: None }))
    );
    assert_eq!(source.gate(), GateState::NeedsRecoveryPhrase);

    source.begin_restore(&words(TEST_PHRASE));
    assert_eq!(
        next(&mut source),
        SignInEvent::PhraseRestored(Ok(NextAfterSignIn::Drive))
    );
    assert_eq!(source.gate(), GateState::Ready);
    assert_eq!(engine.account_email().as_deref(), Some(fake.first.email.as_str()));
    assert_eq!(source.poll(), None, "one answer per step");
}

#[test]
fn a_backup_code_passes_the_code_step() {
    let fake = fake("backup");
    fake.two_factor.store(true, Ordering::SeqCst);
    let (_engine, mut source) = source(&fake);
    source.begin_sign_in(&fake.first.email, PASSWORD);
    assert_eq!(next(&mut source), SignInEvent::SignedIn(Ok(NextAfterSignIn::TwoFactor)));
    source.begin_two_factor(BACKUP, CodeKind::Backup);
    assert_eq!(
        next(&mut source),
        SignInEvent::TwoFactor(Ok(NextAfterSignIn::RecoveryPhrase))
    );
    assert_eq!(source.gate(), GateState::NeedsRecoveryPhrase);
}

#[test]
fn a_wrong_password_and_an_address_nobody_has_get_the_same_answer_and_store_nothing() {
    let fake = fake("wrong");
    let (engine, mut source) = source(&fake);
    source.begin_sign_in(&fake.first.email, "not the password");
    let wrong_password = next(&mut source);
    source.begin_sign_in("nobody-at-all@example.eu", PASSWORD);
    let unknown_address = next(&mut source);
    assert_eq!(wrong_password, SignInEvent::SignedIn(Err(SignInProblem::Invalid)));
    assert_eq!(
        unknown_address, wrong_password,
        "the answer does not say whether the address exists"
    );
    assert_eq!(source.gate(), GateState::SignedOut);
    assert_eq!(engine.account_email(), None);
    assert_eq!(requests(&fake, "/auth/me"), 0, "no session was made");
}

#[test]
fn a_server_that_does_not_answer_is_offline_at_every_step() {
    let fake = fake("offline");
    fake.two_factor.store(true, Ordering::SeqCst);
    let (_engine, mut source) = source(&fake);
    fake.sign_in.store(NO_ANSWER, Ordering::SeqCst);
    source.begin_sign_in(&fake.first.email, PASSWORD);
    assert_eq!(next(&mut source), SignInEvent::SignedIn(Err(SignInProblem::Offline)));
    fake.sign_in.store(ANSWERS, Ordering::SeqCst);

    source.begin_sign_in(&fake.first.email, PASSWORD);
    assert_eq!(next(&mut source), SignInEvent::SignedIn(Ok(NextAfterSignIn::TwoFactor)));
    fake.code.store(NO_ANSWER, Ordering::SeqCst);
    source.begin_two_factor(TOTP, CodeKind::Authenticator);
    assert_eq!(
        next(&mut source),
        SignInEvent::TwoFactor(Err(TwoFactorProblem::Offline))
    );
    assert_eq!(source.gate(), GateState::SignedOut);
    // The step is still there: the same code passes once the server answers.
    fake.code.store(ANSWERS, Ordering::SeqCst);
    source.begin_two_factor(TOTP, CodeKind::Authenticator);
    assert_eq!(
        next(&mut source),
        SignInEvent::TwoFactor(Ok(NextAfterSignIn::RecoveryPhrase))
    );
}

#[test]
fn too_many_tries_is_said_with_the_servers_wait_and_counted_down_to_zero() {
    let fake = fake("limit");
    fake.two_factor.store(true, Ordering::SeqCst);
    let (_engine, mut source) = source(&fake);
    fake.sign_in.store(RATE_LIMITED, Ordering::SeqCst);
    source.begin_sign_in(&fake.first.email, PASSWORD);
    assert_eq!(
        next(&mut source),
        SignInEvent::SignedIn(Err(SignInProblem::RateLimited { seconds: 30 }))
    );
    let mut ticks = Vec::new();
    while ticks.last() != Some(&0) {
        match next(&mut source) {
            SignInEvent::RetryAfter { seconds } => ticks.push(seconds),
            other => panic!("only the countdown answers here, not {other:?}"),
        }
    }
    assert_eq!(
        ticks,
        (0..30).rev().collect::<Vec<u32>>(),
        "every second, down to the end"
    );
    thread::sleep(Duration::from_millis(20));
    assert_eq!(source.poll(), None, "the countdown ended");

    // The code step's limit has no seconds: said as a limit, not as a wrong code.
    fake.sign_in.store(ANSWERS, Ordering::SeqCst);
    source.begin_sign_in(&fake.first.email, PASSWORD);
    assert_eq!(next(&mut source), SignInEvent::SignedIn(Ok(NextAfterSignIn::TwoFactor)));
    fake.code.store(RATE_LIMITED, Ordering::SeqCst);
    source.begin_two_factor(TOTP, CodeKind::Authenticator);
    assert_eq!(
        next(&mut source),
        SignInEvent::TwoFactor(Err(TwoFactorProblem::RateLimited))
    );
}

#[test]
fn a_locked_vault_opens_with_the_password_and_with_nothing_else() {
    let fake = fake("lock");
    let (engine, mut source) = signed_in(&fake);
    let _ = engine.lock();
    assert_eq!(source.gate(), GateState::Locked);

    source.begin_unlock("not the password");
    assert_eq!(
        next(&mut source),
        SignInEvent::VaultUnlocked(Err(SignInProblem::Invalid))
    );
    assert_eq!(source.gate(), GateState::Locked);

    fake.step_up.store(RATE_LIMITED, Ordering::SeqCst);
    source.begin_unlock(PASSWORD);
    assert_eq!(
        next(&mut source),
        SignInEvent::VaultUnlocked(Err(SignInProblem::RateLimited { seconds: 30 }))
    );
    fake.step_up.store(NO_ANSWER, Ordering::SeqCst);
    source.begin_unlock(PASSWORD);
    assert_eq!(
        next(&mut source),
        SignInEvent::VaultUnlocked(Err(SignInProblem::Offline))
    );
    assert_eq!(
        source.gate(),
        GateState::Locked,
        "a proof that was not made unlocks nothing"
    );
    fake.step_up.store(ANSWERS, Ordering::SeqCst);

    source.begin_unlock(PASSWORD);
    assert_eq!(
        next(&mut source),
        SignInEvent::VaultUnlocked(Ok(NextAfterSignIn::Drive))
    );
    assert_eq!(source.gate(), GateState::Ready);
    assert_eq!(engine.account_email().as_deref(), Some(fake.first.email.as_str()));
    // Lock, then unlock: the account's local state is handed out for the new session.
    assert!(engine.keys_for_engine_start().is_ok());
}

#[test]
fn another_account_signs_in_after_the_first_and_gets_only_its_own_session() {
    let fake = fake("other");
    let (engine, mut source) = signed_in(&fake);
    // One account at a time: a second sign-in is refused while the first is signed in, and changes nothing.
    source.begin_sign_in(&fake.second.email, PASSWORD);
    assert_eq!(next(&mut source), SignInEvent::SignedIn(Err(SignInProblem::Other)));
    assert_eq!(engine.user_id().as_deref(), Some(fake.first.user_id.as_str()));

    engine.sign_out_keeping_local_data().expect("signed out");
    assert_eq!(source.gate(), GateState::SignedOut);
    source.begin_sign_in(&fake.second.email, PASSWORD);
    assert_eq!(
        next(&mut source),
        SignInEvent::SignedIn(Ok(NextAfterSignIn::RecoveryPhrase))
    );
    assert_eq!(engine.user_id().as_deref(), Some(fake.second.user_id.as_str()));
    assert_eq!(engine.account_email().as_deref(), Some(fake.second.email.as_str()));
    source.begin_restore(&words(TEST_PHRASE));
    assert_eq!(
        next(&mut source),
        SignInEvent::PhraseRestored(Ok(NextAfterSignIn::Drive))
    );
    assert_eq!(source.gate(), GateState::Ready);
    assert_eq!(
        source.poll(),
        None,
        "nothing of the first account's steps is left to answer"
    );
}

#[test]
fn leaving_the_code_step_drops_its_answer_and_a_code_that_had_not_been_sent_yet() {
    let fake = fake("leave");
    fake.two_factor.store(true, Ordering::SeqCst);
    let (engine, mut source) = source(&fake);
    source.begin_sign_in(&fake.first.email, PASSWORD);
    assert_eq!(next(&mut source), SignInEvent::SignedIn(Ok(NextAfterSignIn::TwoFactor)));

    // The right code is at the server and not answered yet; a second press waits behind it; then the person
    // goes back.
    fake.code_door.shut();
    source.begin_two_factor(TOTP, CodeKind::Authenticator);
    wait_until("the code to be sent", || requests(&fake, "/auth/2fa/verify") == 1);
    source.begin_two_factor(TOTP, CodeKind::Authenticator);
    source.discard_partial_sign_in();
    fake.code_door.open();

    // The first code's sign-in finished at the server, so the engine signs that session out again; the second
    // code is never sent; neither answer reaches the screen.
    wait_until("the session to be signed out", || requests(&fake, "/auth/logout") == 1);
    wait_until("the gate to close", || source.gate() == GateState::SignedOut);
    assert_eq!(
        requests(&fake, "/auth/2fa/verify"),
        1,
        "a step that was left is not started"
    );
    assert_eq!(source.poll(), None, "an answer of a step that was left is dropped");
    assert_eq!(engine.account_email(), None);
}

#[test]
fn a_code_that_was_waiting_behind_another_when_the_person_left_is_never_sent() {
    let fake = fake("queued");
    fake.two_factor.store(true, Ordering::SeqCst);
    let (engine, mut source) = source(&fake);
    source.begin_sign_in(&fake.first.email, PASSWORD);
    assert_eq!(next(&mut source), SignInEvent::SignedIn(Ok(NextAfterSignIn::TwoFactor)));

    // A wrong code is at the server (it keeps the step); the right one waits behind it; the person goes back.
    fake.code_door.shut();
    source.begin_two_factor("000000", CodeKind::Authenticator);
    wait_until("the code to be sent", || requests(&fake, "/auth/2fa/verify") == 1);
    source.begin_two_factor(TOTP, CodeKind::Authenticator);
    source.discard_partial_sign_in();
    // The next thing asked runs after all of it: its answer says the earlier jobs are done.
    source.begin_sign_in(&fake.second.email, PASSWORD);
    fake.code_door.open();
    assert_eq!(
        next(&mut source),
        SignInEvent::SignedIn(Ok(NextAfterSignIn::RecoveryPhrase))
    );

    assert_eq!(
        requests(&fake, "/auth/2fa/verify"),
        1,
        "the waiting code was never sent"
    );
    assert_eq!(requests(&fake, "/auth/logout"), 0, "so nobody was signed in by it");
    assert_eq!(engine.user_id().as_deref(), Some(fake.second.user_id.as_str()));
}

#[test]
fn going_back_from_the_twelve_words_signs_out_so_another_account_can_sign_in() {
    let fake = fake("back");
    let (engine, mut source) = source(&fake);
    source.begin_sign_in(&fake.first.email, PASSWORD);
    assert_eq!(
        next(&mut source),
        SignInEvent::SignedIn(Ok(NextAfterSignIn::RecoveryPhrase))
    );
    assert_eq!(engine.view().gate, EngineGate::NeedsRecoveryPhrase);

    source.discard_partial_sign_in();
    // Asked at once, before the engine took its turn: the sign-in waits behind the sign-out.
    source.begin_sign_in(&fake.second.email, PASSWORD);
    assert_eq!(
        next(&mut source),
        SignInEvent::SignedIn(Ok(NextAfterSignIn::RecoveryPhrase))
    );
    assert_eq!(engine.user_id().as_deref(), Some(fake.second.user_id.as_str()));
    assert_eq!(
        requests(&fake, "/auth/logout"),
        1,
        "the first session was ended at the server"
    );
}

#[test]
fn what_is_not_bound_answers_its_screens_refusal_at_once_and_asks_the_server_nothing() {
    let fake = fake("unbound");
    let (_engine, mut source) = source(&fake);
    assert_eq!(source.shared_session(), None);
    assert!(!source.sign_up_available());

    source.begin_recovery_verify("someone@example.eu", &words(TEST_PHRASE));
    assert_eq!(
        source.poll(),
        Some(SignInEvent::RecoveryPhraseVerified(Err(PhraseProblem::Other)))
    );
    source.begin_set_password("an invented new password");
    assert_eq!(source.poll(), Some(SignInEvent::PasswordSet(Err(SignInProblem::Other))));
    source.begin_passkey();
    assert_eq!(source.poll(), Some(SignInEvent::Passkey(PasskeyOutcome::Unavailable)));
    source.begin_device_request();
    assert_eq!(
        source.poll(),
        Some(SignInEvent::DeviceRequested(Err(SignInProblem::Other)))
    );
    source.begin_hello();
    assert_eq!(source.poll(), Some(SignInEvent::Hello(HelloOutcome::Cancelled)));
    // Leaving a step does not drop these: they belong to no step that can be left.
    source.begin_passkey();
    source.discard_partial_sign_in();
    assert_eq!(source.poll(), Some(SignInEvent::Passkey(PasskeyOutcome::Unavailable)));
    assert_eq!(fake.fake.routes().len(), 0, "nothing was asked of the server");
}

#[test]
fn a_secret_reaches_the_engine_as_it_was_typed_and_the_words_as_one_phrase() {
    assert_eq!(secret(PASSWORD).as_str(), PASSWORD);
    assert_eq!(phrase_of(&words(TEST_PHRASE)).as_str(), TEST_PHRASE);
    assert_eq!(phrase_of(&[]).as_str(), "");
}
