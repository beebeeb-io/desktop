//! The two-level lock through `EngineSignInSource`, on the real engine and the fake accounts of `signin_tests.rs`:
//! what a Windows Hello answer opens (a locked screen, and only on a verified prompt of the turn it was asked
//! in), and what the account's password opens (either level, with the code step when the account has one).
//! The prompt is a script the test answers; nothing here waits on Windows.

use std::sync::atomic::AtomicUsize;

use beebeeb_engine::app::lock_api::LockState;

use super::*;
use crate::engine_host::hello::{HelloCancel, HelloDone, HelloPrompter, HelloResult};

/// The prompts that are up, oldest first, and how many were taken away.
#[derive(Clone, Default)]
struct Prompts {
    up: Arc<Mutex<Vec<HelloDone>>>,
    taken_away: Arc<AtomicUsize>,
    asked: Arc<AtomicUsize>,
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

    /// The oldest prompt that is up ends with `result`, on this thread (the prompt's own, in the app).
    fn answer(&self, result: HelloResult) {
        let done = self.up.lock().unwrap().remove(0);
        done(result);
    }
}

/// Signed in as the first account, the keys in memory, with a prompt the test answers.
fn open_with_prompts(fake: &Fake) -> (Arc<Engine<MemoryCredentials>>, Source, Prompts) {
    let (engine, mut source) = signed_in(fake);
    let prompts = Prompts::default();
    source.hello = Box::new(prompts.clone());
    (engine, source, prompts)
}

#[test]
fn windows_hello_opens_a_locked_screen_only_on_a_verified_prompt() {
    let fake = fake("hello");
    let (engine, mut source, prompts) = open_with_prompts(&fake);
    // Open: nothing a prompt could open, so none is shown.
    source.begin_hello();
    assert_eq!(next(&mut source), SignInEvent::Hello(HelloOutcome::Cancelled));
    assert_eq!(prompts.asked(), 0);

    assert_eq!(engine.lock_screen(), Ok(()));
    assert_eq!(
        source.gate(),
        GateState::Locked,
        "a locked screen is the lock screen's gate"
    );
    let asked_of_the_server = fake.fake.routes().len();
    for (result, outcome) in [
        (HelloResult::Failed, HelloOutcome::NotRecognized),
        (HelloResult::Cancelled, HelloOutcome::Cancelled),
        (HelloResult::Unavailable, HelloOutcome::Unavailable),
    ] {
        source.begin_hello();
        prompts.answer(result);
        assert_eq!(next(&mut source), SignInEvent::Hello(outcome));
        assert_eq!(engine.lock_state(), LockState::ScreenLocked, "{result:?} opens nothing");
    }
    assert_eq!(prompts.asked(), 3);

    source.begin_hello();
    prompts.answer(HelloResult::Verified);
    assert_eq!(
        next(&mut source),
        SignInEvent::Hello(HelloOutcome::Succeeded(NextAfterSignIn::Drive))
    );
    assert_eq!(engine.lock_state(), LockState::Open);
    assert_eq!(source.gate(), GateState::Ready);
    assert_eq!(
        fake.fake.routes().len(),
        asked_of_the_server,
        "the screen lock opens without the server"
    );
}

#[test]
fn a_verified_prompt_of_an_earlier_turn_opens_nothing() {
    let fake = fake("late");
    let (engine, mut source, prompts) = open_with_prompts(&fake);
    assert_eq!(engine.lock_screen(), Ok(()));

    // Asked in one lock; the screen was opened and locked again before the prompt answered.
    source.begin_hello();
    assert_eq!(engine.unlock_screen_platform_verified(engine.session_turn()), Ok(()));
    assert_eq!(engine.lock_screen(), Ok(()));
    prompts.answer(HelloResult::Verified);
    assert_eq!(next(&mut source), SignInEvent::Hello(HelloOutcome::Cancelled));
    assert_eq!(engine.lock_state(), LockState::ScreenLocked);

    // The person left the prompt: it is taken away, its yes opens nothing and reaches no screen.
    source.begin_hello();
    source.cancel_hello();
    assert_eq!(prompts.taken_away.load(Ordering::SeqCst), 1);
    prompts.answer(HelloResult::Verified);
    assert_eq!(engine.lock_state(), LockState::ScreenLocked);
    assert_eq!(source.poll(), None, "an answer of a prompt that was left is dropped");

    // Asked under the first account; it signed out and ANOTHER account signed in and locked its screen
    // before the prompt answered: the yes is not that account's.
    source.begin_hello();
    engine.sign_out_keeping_local_data().expect("signed out");
    source.begin_sign_in(&fake.second.email, PASSWORD);
    assert_eq!(
        next(&mut source),
        SignInEvent::SignedIn(Ok(NextAfterSignIn::RecoveryPhrase))
    );
    source.begin_restore(&words(TEST_PHRASE));
    assert_eq!(
        next(&mut source),
        SignInEvent::PhraseRestored(Ok(NextAfterSignIn::Drive))
    );
    assert_eq!(engine.lock_screen(), Ok(()));
    prompts.answer(HelloResult::Verified);
    assert_eq!(next(&mut source), SignInEvent::Hello(HelloOutcome::Cancelled));
    assert_eq!(engine.lock_state(), LockState::ScreenLocked);
    assert_eq!(engine.user_id().as_deref(), Some(fake.second.user_id.as_str()));
}

#[test]
fn without_a_prompt_the_lock_screen_is_told_so_at_once() {
    let fake = fake("noprompt");
    // The source as built without a window: no prompt can be shown.
    let (engine, mut source) = signed_in(&fake);
    assert_eq!(engine.lock_screen(), Ok(()));
    source.begin_hello();
    assert_eq!(source.poll(), Some(SignInEvent::Hello(HelloOutcome::Unavailable)));
    assert_eq!(engine.lock_state(), LockState::ScreenLocked);
}

#[test]
fn a_closed_vault_opens_with_the_password_and_with_nothing_else() {
    let fake = fake("vault");
    let (engine, mut source, prompts) = open_with_prompts(&fake);
    let _ = engine.lock_vault();
    assert_eq!(
        source.gate(),
        GateState::VaultClosed,
        "a closed vault is not the lock screen's gate"
    );
    assert_eq!(engine.lock_state(), LockState::VaultClosed);

    // Windows Hello opens no closed vault: no prompt is even shown.
    source.begin_hello();
    assert_eq!(next(&mut source), SignInEvent::Hello(HelloOutcome::Cancelled));
    assert_eq!(prompts.asked(), 0);

    source.begin_unlock("not the password");
    assert_eq!(
        next(&mut source),
        SignInEvent::VaultUnlocked(Err(SignInProblem::Invalid))
    );
    fake.sign_in.store(RATE_LIMITED, Ordering::SeqCst);
    source.begin_unlock(PASSWORD);
    assert_eq!(
        next(&mut source),
        SignInEvent::VaultUnlocked(Err(SignInProblem::RateLimited { seconds: 30 }))
    );
    fake.sign_in.store(NO_ANSWER, Ordering::SeqCst);
    source.begin_unlock(PASSWORD);
    assert_eq!(
        next(&mut source),
        SignInEvent::VaultUnlocked(Err(SignInProblem::Offline))
    );
    assert_eq!(
        source.gate(),
        GateState::VaultClosed,
        "a proof that was not made unlocks nothing"
    );
    fake.sign_in.store(ANSWERS, Ordering::SeqCst);

    source.begin_unlock(PASSWORD);
    assert_eq!(
        next(&mut source),
        SignInEvent::VaultUnlocked(Ok(NextAfterSignIn::Drive))
    );
    assert_eq!(source.gate(), GateState::Ready);
    assert_eq!(engine.account_email().as_deref(), Some(fake.first.email.as_str()));
    assert!(engine.keys_for_engine_start().is_ok());
}

#[test]
fn the_password_opens_a_locked_screen_too_and_asks_the_code_when_the_account_has_one() {
    let fake = fake("strong");
    let (engine, mut source, _prompts) = open_with_prompts(&fake);
    assert_eq!(engine.lock_screen(), Ok(()));
    source.begin_unlock(PASSWORD);
    assert_eq!(
        next(&mut source),
        SignInEvent::VaultUnlocked(Ok(NextAfterSignIn::Drive))
    );
    assert_eq!(engine.lock_state(), LockState::Open);

    // With an authenticator: the password alone opens nothing; the code finishes it.
    fake.two_factor.store(true, Ordering::SeqCst);
    let _ = engine.lock_vault();
    source.begin_unlock(PASSWORD);
    assert_eq!(
        next(&mut source),
        SignInEvent::VaultUnlocked(Ok(NextAfterSignIn::TwoFactor))
    );
    assert_eq!(source.gate(), GateState::VaultClosed);
    source.begin_two_factor(TOTP, CodeKind::Authenticator);
    assert_eq!(next(&mut source), SignInEvent::TwoFactor(Ok(NextAfterSignIn::Drive)));
    assert_eq!(source.gate(), GateState::Ready);
}

/// The person went back while the code that finishes an unlock was at the server, and it was accepted anyway:
/// the lock is put back at the level it had, and nobody is signed out.
#[test]
fn an_unlock_that_was_left_and_finished_anyway_is_locked_again_at_its_level_never_signed_out() {
    for level in [LockState::ScreenLocked, LockState::VaultClosed] {
        let fake = fake(if level == LockState::ScreenLocked {
            "left-s"
        } else {
            "left-v"
        });
        let (engine, mut source, _prompts) = open_with_prompts(&fake);
        fake.two_factor.store(true, Ordering::SeqCst);
        if level == LockState::ScreenLocked {
            assert_eq!(engine.lock_screen(), Ok(()));
        } else {
            let _ = engine.lock_vault();
        }
        source.begin_unlock(PASSWORD);
        assert_eq!(
            next(&mut source),
            SignInEvent::VaultUnlocked(Ok(NextAfterSignIn::TwoFactor))
        );
        fake.code_door.shut();
        source.begin_two_factor(TOTP, CodeKind::Authenticator);
        wait_until("the code to be sent", || requests(&fake, "/auth/2fa/verify") == 1);
        source.discard_partial_sign_in();
        fake.code_door.open();

        wait_until("the lock to be put back", || engine.lock_state() == level);
        // The worker is done with the step once its wake arrived; the state holds.
        thread::sleep(Duration::from_millis(50));
        assert_eq!(engine.lock_state(), level, "{level:?}");
        assert_eq!(
            engine.user_id().as_deref(),
            Some(fake.first.user_id.as_str()),
            "{level:?}"
        );
        assert_eq!(source.poll(), None, "an answer of a step that was left is dropped");
    }
}
