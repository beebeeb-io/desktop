//! The lock screen's Windows Hello prompt lives as long as the lock it was asked for (core's claim,
//! `Engine::ask_screen_unlock`). When that lock ends any other way (the password opened it, another prompt opened it and
//! the screen locked again, a sign-out, another account), the source's own poll takes the prompt away and tells the
//! screen `Cancelled`, so Unlock asks again. Whatever the taken-away prompt still answers changes nothing. The prompt is
//! the script of `signin_lock_tests.rs`: its `answer` still hands a yes to a prompt that was taken away, as Windows
//! may for one decided before its cancel.

use super::*;

fn taken(prompts: &Prompts) -> usize {
    prompts.taken_away.load(Ordering::SeqCst)
}

/// Locked, with the lock screen's prompt up and its lock holding.
fn locked_with_a_prompt_up(tag: &str) -> (Fake, Arc<Engine<MemoryCredentials>>, Source, Prompts) {
    let fake = fake(tag);
    let (engine, mut source, prompts) = open_with_prompts(&fake);
    assert_eq!(engine.lock_screen(), Ok(()));
    source.begin_hello();
    assert_eq!(prompts.asked(), 1);
    assert_eq!(source.poll(), None, "the lock it was asked for holds: the prompt stays");
    assert_eq!(taken(&prompts), 0);
    (fake, engine, source, prompts)
}

#[test]
fn a_prompt_for_the_lock_that_holds_stays_up_and_its_yes_opens_it() {
    let (_fake, engine, mut source, prompts) = locked_with_a_prompt_up("hello-stays");
    for _ in 0..3 {
        assert_eq!(source.poll(), None);
    }
    assert_eq!(
        taken(&prompts),
        0,
        "a prompt of the lock that holds is never taken away"
    );
    prompts.answer(HelloResult::Verified);
    assert_eq!(
        next(&mut source),
        SignInEvent::Hello(HelloOutcome::Succeeded(NextAfterSignIn::Drive))
    );
    assert_eq!(engine.lock_state(), LockState::Open);
    assert_eq!(taken(&prompts), 0, "an answered prompt is not taken away by the poll");
}

#[test]
fn the_password_opening_the_screen_takes_the_prompt_away_and_its_late_yes_changes_nothing() {
    let (_fake, engine, mut source, prompts) = locked_with_a_prompt_up("hello-password");
    // The screen opened with the account's password while the prompt is up.
    assert_eq!(
        engine.unlock_screen_with_password(secret(PASSWORD)),
        Ok(ScreenUnlock::Opened)
    );
    assert_eq!(
        source.poll(),
        Some(SignInEvent::Hello(HelloOutcome::Cancelled)),
        "the screen is told, so Unlock asks again"
    );
    assert_eq!(taken(&prompts), 1, "the prompt is taken away, not left on screen");
    // Locked again before the taken-away prompt answers: its yes opens nothing and reaches no screen.
    assert_eq!(engine.lock_screen(), Ok(()));
    prompts.answer(HelloResult::Verified);
    assert_eq!(engine.lock_state(), LockState::ScreenLocked);
    assert_eq!(source.poll(), None);
    assert_eq!(taken(&prompts), 1, "nothing else was taken away");
}

#[test]
fn a_lock_after_an_unlock_between_two_polls_takes_the_prompt_away_and_unlock_asks_again() {
    let (_fake, engine, mut source, prompts) = locked_with_a_prompt_up("hello-relock");
    // Opened by another prompt's yes, then locked again, all before the source's next poll: the window's own follow
    // of the lock sees a locked screen both times.
    assert_eq!(engine.unlock_screen_platform_verified(engine.session_turn()), Ok(()));
    assert_eq!(engine.lock_screen(), Ok(()));
    assert_eq!(source.poll(), Some(SignInEvent::Hello(HelloOutcome::Cancelled)));
    assert_eq!(taken(&prompts), 1);
    prompts.answer(HelloResult::Verified);
    assert_eq!(engine.lock_state(), LockState::ScreenLocked, "the later lock holds");
    assert_eq!(source.poll(), None);
    // Unlock again: a prompt for the later lock, and its yes opens it.
    source.begin_hello();
    assert_eq!(prompts.asked(), 2);
    prompts.answer(HelloResult::Verified);
    assert_eq!(
        next(&mut source),
        SignInEvent::Hello(HelloOutcome::Succeeded(NextAfterSignIn::Drive))
    );
    assert_eq!(engine.lock_state(), LockState::Open);
}

#[test]
fn a_sign_out_takes_the_prompt_away_and_its_late_yes_opens_nothing() {
    let (_fake, engine, mut source, prompts) = locked_with_a_prompt_up("hello-signout");
    engine.sign_out_keeping_local_data().expect("signed out");
    assert_eq!(source.poll(), Some(SignInEvent::Hello(HelloOutcome::Cancelled)));
    assert_eq!(taken(&prompts), 1);
    prompts.answer(HelloResult::Verified);
    assert_eq!(engine.lock_state(), LockState::SignedOut);
    assert_eq!(source.poll(), None);
}

#[test]
fn another_account_between_two_polls_takes_the_prompt_away_and_its_yes_is_not_that_accounts() {
    let (fake, engine, mut source, prompts) = locked_with_a_prompt_up("hello-account");
    // Signed out, ANOTHER account signed in and its screen locked, all before the source's next poll.
    engine.sign_out_keeping_local_data().expect("signed out");
    engine
        .sign_in_password(&fake.second.email, secret(PASSWORD))
        .expect("the second account signs in");
    engine
        .unlock_with_phrase(phrase_of(&words(TEST_PHRASE)))
        .expect("its twelve words");
    assert_eq!(engine.lock_screen(), Ok(()));
    assert_eq!(source.poll(), Some(SignInEvent::Hello(HelloOutcome::Cancelled)));
    assert_eq!(taken(&prompts), 1);
    prompts.answer(HelloResult::Verified);
    assert_eq!(engine.lock_state(), LockState::ScreenLocked);
    assert_eq!(engine.user_id().as_deref(), Some(fake.second.user_id.as_str()));
    assert_eq!(source.poll(), None);
}

#[test]
fn a_source_that_goes_takes_its_prompt_away() {
    let (_fake, engine, source, prompts) = locked_with_a_prompt_up("hello-drop");
    drop(source);
    assert_eq!(taken(&prompts), 1, "never left on screen with nobody to answer");
    prompts.answer(HelloResult::Verified);
    assert_eq!(engine.lock_state(), LockState::ScreenLocked);
}
