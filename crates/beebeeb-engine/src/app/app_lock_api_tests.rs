//! App lock at the engine, against the loopback fake of the password path (`lock_password_api_tests.rs`): core's rule
//! for who may change it and with which proof, the session a change was asked for, and the automatic lock. Every
//! name, address and password is invented. The restart (what a start does with it) is in `tests/host_contract.rs`.

use beebeeb_core::screen_lock::wait_after_ms;

use super::super::tests::{PASSWORD, SLOT, T0, WRONG, entry, open_engine, requests, secret, serial, server};
use super::*;
use crate::keychain::AuthSecretStore;
use crate::signin::memory_store::MemoryCredentials;

fn stored(credentials: &MemoryCredentials) -> Option<Vec<u8>> {
    entry(credentials, LockFlag::AppLock)
}

#[test]
fn app_lock_is_on_until_a_proof_given_for_that_change_turns_it_off_and_on_asks_nothing() {
    let _serial = serial();
    let api = server();
    let (engine, credentials) = open_engine(&api.base);
    let turn = engine.session_turn();
    assert_eq!(engine.app_lock_for(turn), Ok(true), "absent is on");
    let asked = requests(&api);
    // Off without a proof, or with an empty password: core's "the proof comes first", nothing written or sent.
    assert_eq!(
        engine.set_app_lock_for_at(turn, false, None, T0),
        Err(AppLockRefused::ProofNeeded)
    );
    assert_eq!(
        engine.set_app_lock_for_at(turn, false, Some(AppLockProof::Password(secret(""))), T0),
        Err(AppLockRefused::ProofNeeded)
    );
    assert_eq!(stored(&credentials), None);
    // Windows Hello verified the person, for this change: off.
    assert_eq!(
        engine.set_app_lock_for_at(turn, false, Some(AppLockProof::PlatformVerified), T0),
        Ok(())
    );
    assert_eq!(stored(&credentials).as_deref(), Some(&b"0"[..]));
    assert_eq!(engine.app_lock_for(turn), Ok(false));
    // On asks for nothing.
    assert_eq!(engine.set_app_lock_for_at(turn, true, None, T0), Ok(()));
    assert_eq!(stored(&credentials).as_deref(), Some(&b"1"[..]));
    assert_eq!(engine.app_lock_for(turn), Ok(true));
    assert_eq!(requests(&api), asked, "nothing of this was asked of the server");
}

#[test]
fn a_wrong_password_changes_nothing_and_says_the_wait_and_the_right_one_turns_it_off() {
    let _serial = serial();
    let api = server();
    let (engine, credentials) = open_engine(&api.base);
    let turn = engine.session_turn();
    assert_eq!(
        engine.set_app_lock_for_at(turn, false, Some(AppLockProof::Password(secret(WRONG))), T0),
        Err(AppLockRefused::Wrong {
            wait_ms: wait_after_ms(1),
            local_check_off: false
        })
    );
    assert_eq!(stored(&credentials), None, "a wrong password writes nothing");
    assert_eq!(engine.app_lock_for(turn), Ok(true));
    let after_the_wait = T0 + wait_after_ms(1) + 1;
    assert_eq!(
        engine.set_app_lock_for_at(
            turn,
            false,
            Some(AppLockProof::Password(secret(PASSWORD))),
            after_the_wait
        ),
        Ok(())
    );
    assert_eq!(stored(&credentials).as_deref(), Some(&b"0"[..]));
}

#[test]
fn an_answer_for_a_session_that_moved_on_writes_nothing() {
    let _serial = serial();
    let api = server();
    let (engine, credentials) = open_engine(&api.base);
    let pressed = engine.session_turn();
    // The screen locks and opens again while the prompt is up: the press is of another turn.
    engine.lock_screen().unwrap();
    engine.unlock_screen_platform_verified(engine.session_turn()).unwrap();
    assert_eq!(engine.lock_state(), LockState::Open);
    assert_eq!(
        engine.set_app_lock_for_at(pressed, false, Some(AppLockProof::PlatformVerified), T0),
        Err(AppLockRefused::Overtaken)
    );
    assert_eq!(
        engine.set_app_lock_for_at(pressed, true, None, T0),
        Err(AppLockRefused::Overtaken)
    );
    assert_eq!(engine.app_lock_for(pressed), Err(AppLockRefused::Overtaken));
    assert_eq!(stored(&credentials), None);
}

#[test]
fn only_an_open_app_changes_it() {
    let _serial = serial();
    let api = server();
    let (engine, credentials) = open_engine(&api.base);
    engine.lock_screen().unwrap();
    let turn = engine.session_turn();
    assert_eq!(
        engine.set_app_lock_for_at(turn, true, None, T0),
        Err(AppLockRefused::NotOpen)
    );
    assert_eq!(
        engine.set_app_lock_for_at(turn, false, Some(AppLockProof::PlatformVerified), T0),
        Err(AppLockRefused::NotOpen)
    );
    assert_eq!(stored(&credentials), None);
}

#[test]
fn the_automatic_lock_follows_app_lock_and_lock_now_locks_whatever_it_says() {
    let _serial = serial();
    let api = server();
    let (engine, _credentials) = open_engine(&api.base);
    // On (absent): away locks the screen, once; every earlier answer is for another turn.
    let before = engine.session_turn();
    assert_eq!(engine.lock_after_away(), LockState::ScreenLocked);
    let locked = engine.session_turn();
    assert_ne!(locked, before);
    assert_eq!(engine.lock_after_away(), LockState::ScreenLocked);
    assert_eq!(engine.session_turn(), locked, "already locked: not again");
    // Off: away changes nothing, and "Lock now" still locks.
    engine.unlock_screen_platform_verified(engine.session_turn()).unwrap();
    let turn = engine.session_turn();
    engine
        .set_app_lock_for_at(turn, false, Some(AppLockProof::PlatformVerified), T0)
        .unwrap();
    assert_eq!(engine.lock_after_away(), LockState::Open);
    assert_eq!(engine.session_turn(), turn);
    engine.lock_screen().unwrap();
    assert_eq!(engine.lock_state(), LockState::ScreenLocked);
}

#[test]
fn an_app_lock_entry_that_does_not_read_counts_as_on() {
    let _serial = serial();
    let api = server();
    let (engine, credentials) = open_engine(&api.base);
    credentials
        .store_for(SLOT)
        .save_lock_flag(LockFlag::AppLock, b"maybe")
        .unwrap();
    assert_eq!(engine.app_lock_for(engine.session_turn()), Ok(true));
    assert_eq!(engine.lock_after_away(), LockState::ScreenLocked);
}
