//! The in-app recovery on the real engine (`signin_recovery.rs`), against the engine's fake account on loopback, with
//! the onboarding document and the recovery's first route answered here in the server's shape. Each test drives the
//! source as the screens do (`begin_*`, `password_strength`, then `poll`). Every account, password and word is
//! invented. Engine handles are on the process's state folder, which nothing here writes in.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use beebeeb_engine::app::{Engine, EngineConfig};
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::test_api::fake_account::{FakeAccount, account_handler};
use beebeeb_engine::test_api::{FakeApi, Handler, Reply, Request};
use serde_json::json;

use super::super::{EngineSignInSource, Wait};
use crate::frames::signin::source::{NewPasswordProblem, PhraseProblem, SignInEvent, SignInSource};

/// A valid phrase (BIP-39's second test vector): the fake account's.
const PHRASE: &str = "legal winner thank year wave sausage worth useful legal winner thank yellow";
const PASSWORD: &str = "an invented password that was forgotten";
/// The server's minimum in this fake's onboarding document: not core's default, so the meter shows it was read.
const MIN_LENGTH: u32 = 14;
const START: &str = "POST /api/v1/auth/recover-with-phrase-start";
const REGISTER: &str = "POST /api/v1/auth/recover-opaque-register";

type Source = EngineSignInSource<MemoryCredentials>;

/// A door the recovery's start waits at until the test opens it: a press that is on its way, without any timing.
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

struct Fake {
    fake: FakeApi,
    account: Arc<FakeAccount>,
    start_door: Arc<Door>,
}

fn fake(tag: &str) -> Fake {
    let (account, _) = FakeAccount::register(&format!("{tag}-recovery@example.eu"), PASSWORD, PHRASE);
    let account = Arc::new(account);
    let rest = account_handler(account.clone());
    let start_door = Arc::new(Door::default());
    start_door.open();
    let (account_in, door_in) = (account.clone(), start_door.clone());
    let handler: Handler = Arc::new(move |req: &Request| match req.route().as_str() {
        "GET /api/v1/onboarding" => Reply::ok(json!({ "policy": { "password": { "min_length": MIN_LENGTH } } })),
        START => {
            door_in.pass();
            if req.json()["recovery_check"] == account_in.recovery_check.as_str() {
                Reply::ok(json!({ "recovery_token": "an-invented-recovery-token" }))
            } else {
                Reply::error(400, "invalid_recovery_phrase")
            }
        }
        _ => rest(req),
    });
    Fake {
        fake: FakeApi::start(handler),
        account,
        start_door,
    }
}

/// A source on a fresh signed-out handle.
fn source(fake: &Fake) -> Source {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "signin-recovery-test-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: fake.fake.base.clone(),
    };
    let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens");
    engine.restore();
    EngineSignInSource::with_tick(Arc::new(engine), Duration::from_millis(2))
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
    fake.fake.routes().iter().filter(|seen| seen.as_str() == route).count()
}

/// How many steps were asked of the worker, over every wait.
fn steps_asked(source: &Source) -> u64 {
    source
        .attempts
        .iter()
        .map(|attempt| attempt.load(Ordering::SeqCst))
        .sum()
}

#[test]
fn signin_recovery_late_answer_dropped() {
    let fake = fake("late");
    let mut source = source(&fake);
    let words: Vec<&str> = PHRASE.split(' ').collect();

    // The first press is on its way: the server holds its start.
    fake.start_door.shut();
    source.begin_recovery_verify(&fake.account.email, &words);
    wait_until("the first press reached the server", || requests(&fake, START) == 1);

    // The person presses again, with a word that is not in the list, before the first answer came.
    let mut typo = words.clone();
    typo[6] = "notaword";
    source.begin_recovery_verify(&fake.account.email, &typo);
    fake.start_door.open();

    // The only answer handed out is the second press's; the first one's ("verified") is dropped.
    assert_eq!(
        next(&mut source),
        SignInEvent::RecoveryPhraseVerified(Err(PhraseProblem::WrongPhrase { bad_word: Some(6) }))
    );
    assert_eq!(source.poll(), None);
    assert_eq!(requests(&fake, START), 1, "the second press sent nothing");

    // The engine holds no recovery of the first press either: the next step sends nothing.
    source.begin_set_password("An invented new password, 2026");
    assert_eq!(
        next(&mut source),
        SignInEvent::PasswordSet(Err(NewPasswordProblem::Ended))
    );
    assert_eq!(requests(&fake, REGISTER), 0);
}

#[test]
fn signin_recovery_strength_is_local() {
    let fake = fake("strength");
    let mut source = source(&fake);

    // Before the words: core's default minimum, by sign-up's rules and words.
    let short = source.password_strength("short").expect("a verdict");
    assert_eq!(
        (short.bars, short.words.as_str()),
        (1, "Needs at least 12 characters, 7 more.")
    );
    let strong = source.password_strength("Mixed-Case-and-1-number").expect("a verdict");
    assert_eq!((strong.bars, strong.words.as_str()), (4, "Strong."));
    assert_eq!(source.password_strength(""), None);
    assert!(fake.fake.requests().is_empty(), "no request: {:?}", fake.fake.routes());
    assert_eq!(steps_asked(&source), 0, "no step was given to the worker");
    assert_eq!(source.poll(), None);

    // After the words: the server's minimum, still with no request and no step.
    let words: Vec<&str> = PHRASE.split(' ').collect();
    source.begin_recovery_verify(&fake.account.email, &words);
    assert_eq!(next(&mut source), SignInEvent::RecoveryPhraseVerified(Ok(())));
    let (seen, asked) = (fake.fake.requests().len(), steps_asked(&source));
    assert_eq!(asked, 1);
    assert_eq!(
        source.attempts[Wait::Restore as usize].load(Ordering::SeqCst),
        1,
        "the verify is the recovery's one step"
    );
    let thirteen = source.password_strength("thirteen-char").expect("a verdict");
    assert_eq!(
        thirteen.words,
        format!("Needs at least {MIN_LENGTH} characters, 1 more.")
    );
    assert_eq!(fake.fake.requests().len(), seen, "the meter sent nothing");
    assert_eq!(steps_asked(&source), asked, "the meter gave the worker nothing");
    assert_eq!(source.poll(), None);
}

#[test]
fn signin_recovery_discard_forgets_the_words() {
    let fake = fake("discard");
    let mut source = source(&fake);
    let words: Vec<&str> = PHRASE.split(' ').collect();
    source.begin_recovery_verify(&fake.account.email, &words);
    assert_eq!(next(&mut source), SignInEvent::RecoveryPhraseVerified(Ok(())));

    // The person leaves a half-done sign-in: the engine forgets the recovery it held (the key the words derived and
    // the server's recovery token), so the next step has nothing to go on and sends nothing.
    source.discard_partial_sign_in();
    source.begin_set_password("An invented new password, 2026");
    assert_eq!(
        next(&mut source),
        SignInEvent::PasswordSet(Err(NewPasswordProblem::Ended))
    );
    assert_eq!(
        requests(&fake, REGISTER),
        0,
        "the forgotten recovery sent nothing: {:?}",
        fake.fake.routes()
    );
}

#[test]
fn signin_recovery_marks_the_box_of_an_unknown_word() {
    let fake = fake("box");
    let mut source = source(&fake);
    let mut words: Vec<&str> = PHRASE.split(' ').collect();
    // A box typed with a space after its word (as a person moving on to the next box might), and a word that is not
    // in the list four boxes later: the mark is on that box, not one further.
    words[2] = "thank ";
    words[6] = "notaword";
    source.begin_recovery_verify(&fake.account.email, &words);
    assert_eq!(
        next(&mut source),
        SignInEvent::RecoveryPhraseVerified(Err(PhraseProblem::WrongPhrase { bad_word: Some(6) }))
    );
    assert!(
        fake.fake.requests().is_empty(),
        "nothing is sent: {:?}",
        fake.fake.routes()
    );
}

/// Row 3.5 of checkpoint 2: each answer of the new password reaches the screen as its own kind, so the screen can say
/// what is true (the two-factor account after recovery is told the password WAS changed). Mutant: `SignInRequired` mapped
/// to `NewPasswordProblem::Other` (the screen then says the step did not finish).
#[test]
fn signin_recovery_every_answer_of_the_new_password_keeps_its_kind() {
    use NewPasswordProblem as Kind;
    use beebeeb_engine::app::bindings::recovery::{RecoveryDone, RecoveryRefused};
    assert_eq!(super::password_set(Ok(RecoveryDone::SignedIn)), Ok(()));
    assert_eq!(
        super::password_set(Ok(RecoveryDone::SignInRequired)),
        Err(Kind::SignInRequired)
    );
    for (refused, kind) in [
        (RecoveryRefused::Weak, Kind::TooShort),
        (RecoveryRefused::Breached, Kind::Breached),
        (RecoveryRefused::Offline, Kind::Offline),
        (
            RecoveryRefused::RateLimited { seconds: 30 },
            Kind::RateLimited { seconds: 30 },
        ),
        (RecoveryRefused::Overtaken, Kind::Ended),
        (RecoveryRefused::WrongPhrase, Kind::Other),
        (RecoveryRefused::UnknownWords(vec![3]), Kind::Other),
        (RecoveryRefused::Other, Kind::Other),
    ] {
        assert_eq!(super::password_set(Err(refused.clone())), Err(kind), "{refused:?}");
    }
}
