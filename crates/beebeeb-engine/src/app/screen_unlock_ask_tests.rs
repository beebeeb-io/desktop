//! A Windows Hello prompt's claim on the screen lock (`Engine::ask_screen_unlock`): it holds only while the lock it
//! was asked for holds, and a "verified" handed back after that lock ended opens nothing, also not a later lock of the
//! same session. On the local fake API of `lock_password_api_tests.rs`, so the screen password is a real proof.

use super::*;

/// Opened by another prompt's yes (the screen was locked when this is called).
fn opened_by_another_prompt(engine: &TestEngine) {
    let other = engine.ask_screen_unlock().expect("a locked screen is claimed");
    assert_eq!(engine.unlock_screen_for(other), Ok(()));
    assert_eq!(engine.lock_state(), LockState::Open);
}

#[test]
fn a_claim_is_made_only_for_a_locked_screen_and_ends_with_a_closed_vault() {
    let _one = serial();
    let api = server();
    let (engine, _credentials) = open_engine(&api.base);
    assert_eq!(engine.ask_screen_unlock(), None, "open: nothing a prompt could open");
    engine.lock_screen().unwrap();
    let ask = engine.ask_screen_unlock().expect("a locked screen is claimed");
    assert!(engine.screen_unlock_ask_holds(ask), "the lock it was asked for holds");
    assert!(engine.screen_unlock_ask_holds(ask), "asking again changes nothing");
    let _ = engine.lock_vault();
    assert!(
        !engine.screen_unlock_ask_holds(ask),
        "a closed vault is not the screen lock"
    );
    assert_eq!(engine.ask_screen_unlock(), None, "a closed vault is never claimed");
    assert_eq!(engine.unlock_screen_for(ask), Err(LockRefused::Overtaken));
    assert_eq!(engine.lock_state(), LockState::VaultClosed);
}

#[test]
fn the_password_ends_the_claim_and_its_late_yes_opens_no_later_lock() {
    let _one = serial();
    let api = server();
    let (engine, _credentials) = locked_engine(&api.base);
    let ask = engine.ask_screen_unlock().expect("a locked screen is claimed");
    // Opened by the password while the prompt is up: the lock it was asked for is over.
    assert_eq!(
        engine.unlock_screen_with_password_at(secret(PASSWORD), T0),
        Ok(ScreenUnlock::Opened)
    );
    assert!(!engine.screen_unlock_ask_holds(ask), "the password ended the claim");
    // Its yes while the screen is open: nothing to open (core's rule), and nothing changes.
    assert_eq!(engine.unlock_screen_for(ask), Err(LockRefused::NotAtThisLevel));
    assert_eq!(engine.lock_state(), LockState::Open);
    // Locked again: another lock. The first prompt's yes does not open it.
    engine.lock_screen().unwrap();
    assert!(!engine.screen_unlock_ask_holds(ask), "a later lock is not the claim's");
    assert_eq!(engine.unlock_screen_for(ask), Err(LockRefused::Overtaken));
    assert_eq!(engine.lock_state(), LockState::ScreenLocked, "the later lock holds");
    // The later lock's own claim is another, and it opens that lock.
    let again = engine.ask_screen_unlock().expect("the later lock is claimed");
    assert_ne!(again, ask);
    assert!(engine.screen_unlock_ask_holds(again));
    assert_eq!(engine.unlock_screen_for(again), Ok(()));
    assert_eq!(engine.lock_state(), LockState::Open);
}

#[test]
fn a_yes_asked_before_an_unlock_and_the_away_lock_after_it_opens_nothing() {
    let _one = serial();
    let api = server();
    let (engine, _credentials) = locked_engine(&api.base);
    let ask = engine.ask_screen_unlock().expect("a locked screen is claimed");
    opened_by_another_prompt(&engine);
    assert!(!engine.screen_unlock_ask_holds(ask), "another prompt opened the lock");
    // Away past the delay (app lock is on: no entry is on): the automatic lock is another lock.
    assert_eq!(engine.lock_after_away(), LockState::ScreenLocked);
    assert!(!engine.screen_unlock_ask_holds(ask));
    assert_eq!(engine.unlock_screen_for(ask), Err(LockRefused::Overtaken));
    assert_eq!(engine.lock_state(), LockState::ScreenLocked, "the away lock holds");
}
