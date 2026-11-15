//! "Locks again after" and the away lock at the engine (decision 2519): the stored delay `lock.after`, its default,
//! who may change it with which proof, and the host's idle and OS-lock hooks going through core's
//! `lock_after_away`. Every name, address and password is invented.

use std::time::Duration;

use super::super::tests::{PASSWORD, SLOT, T0, WRONG, entry, open_engine, secret, serial, server};
use super::*;
use crate::keychain::AuthSecretStore;
use crate::signin::memory_store::MemoryCredentials;

fn stored(credentials: &MemoryCredentials) -> Option<Vec<u8>> {
    entry(credentials, LockFlag::LockAfter)
}

fn idle(seconds: u64) -> Away {
    Away::Idle(Duration::from_secs(seconds))
}

#[test]
fn the_delay_is_five_minutes_while_nothing_is_stored() {
    let _serial = serial();
    let api = server();
    let (engine, credentials) = open_engine(&api.base);
    assert_eq!(stored(&credentials), None);
    assert_eq!(
        engine.lock_after_for(engine.session_turn()),
        Ok(Some(LockAfter::Minutes5))
    );
    assert_eq!(LockAfter::DEFAULT, LockAfter::Minutes5);
}

#[test]
fn idle_past_the_delay_locks_the_screen_with_app_lock_on() {
    let _serial = serial();
    let api = server();
    let (engine, _credentials) = open_engine(&api.base);
    let turn = engine.session_turn();
    // Short of five minutes: nothing locks, and the answer says when it could.
    assert_eq!(
        engine.away_for(turn, idle(299)),
        AwayAnswer {
            state: LockState::Open,
            check_again_in: Some(Duration::from_secs(1)),
        }
    );
    assert_eq!(engine.session_turn(), turn);
    // Five minutes: locked, as core's `lock_after_away` locks (every answer asked for before is of another turn).
    assert_eq!(
        engine.away_for(turn, idle(300)),
        AwayAnswer {
            state: LockState::ScreenLocked,
            check_again_in: None,
        }
    );
    assert_ne!(engine.session_turn(), turn);
}

#[test]
fn with_app_lock_off_nothing_ever_locks() {
    let _serial = serial();
    let api = server();
    let (engine, _credentials) = open_engine(&api.base);
    let turn = engine.session_turn();
    engine
        .set_app_lock_for_at(turn, false, Some(AppLockProof::PlatformVerified), T0)
        .unwrap();
    for seen in [idle(10 * 3_600), Away::LeftTheApp, Away::SessionLocked] {
        assert_eq!(
            engine.away_for(turn, seen),
            AwayAnswer {
                state: LockState::Open,
                check_again_in: None,
            },
            "{seen:?}"
        );
    }
    assert_eq!(engine.session_turn(), turn);
}

#[test]
fn the_operating_systems_lock_locks_with_app_lock_on_whatever_the_delay() {
    let _serial = serial();
    let api = server();
    let (engine, _credentials) = open_engine(&api.base);
    let measured = engine.session_turn();
    engine
        .set_lock_after_for_at(measured, LockAfter::Minutes60, Some(AppLockProof::PlatformVerified), T0)
        .unwrap();
    assert_eq!(
        engine.away_for(measured, Away::SessionLocked).state,
        LockState::ScreenLocked
    );
}

#[test]
fn a_shorter_delay_asks_nothing_and_takes_effect_at_once() {
    let _serial = serial();
    let api = server();
    let (engine, credentials) = open_engine(&api.base);
    let turn = engine.session_turn();
    assert_eq!(
        engine.set_lock_after_for_at(turn, LockAfter::Immediately, None, T0),
        Ok(())
    );
    assert_eq!(stored(&credentials).as_deref(), Some(&b"0"[..]));
    assert_eq!(engine.lock_after_for(turn), Ok(Some(LockAfter::Immediately)));
    // "Immediately": a minute without input while the window stays, or the window closed to the tray.
    assert_eq!(
        engine.away_for(turn, idle(59)).check_again_in,
        Some(Duration::from_secs(1))
    );
    assert_eq!(engine.away_for(turn, Away::LeftTheApp).state, LockState::ScreenLocked);
}

#[test]
fn leaving_the_app_locks_only_with_immediately() {
    let _serial = serial();
    let api = server();
    let (engine, _credentials) = open_engine(&api.base);
    let turn = engine.session_turn();
    assert_eq!(
        engine.away_for(turn, Away::LeftTheApp),
        AwayAnswer {
            state: LockState::Open,
            check_again_in: None,
        }
    );
    assert_eq!(engine.session_turn(), turn);
    engine
        .set_lock_after_for_at(turn, LockAfter::Immediately, None, T0)
        .unwrap();
    assert_eq!(engine.away_for(turn, idle(60)).state, LockState::ScreenLocked);
}

#[test]
fn a_longer_delay_needs_a_proof_given_for_that_change() {
    let _serial = serial();
    let api = server();
    let (engine, credentials) = open_engine(&api.base);
    let turn = engine.session_turn();
    assert_eq!(
        engine.set_lock_after_for_at(turn, LockAfter::Minutes30, None, T0),
        Err(AppLockRefused::ProofNeeded)
    );
    assert_eq!(
        engine.set_lock_after_for_at(
            turn,
            LockAfter::Minutes30,
            Some(AppLockProof::Password(secret(WRONG))),
            T0
        ),
        Err(AppLockRefused::Wrong {
            wait_ms: beebeeb_core::screen_lock::wait_after_ms(1),
            local_check_off: false
        })
    );
    assert_eq!(stored(&credentials), None, "nothing written without the proof");
    assert_eq!(
        engine.away_for(turn, idle(300)).state,
        LockState::ScreenLocked,
        "still five minutes"
    );
    engine.unlock_screen_platform_verified(engine.session_turn()).unwrap();
    let turn = engine.session_turn();
    assert_eq!(
        engine.set_lock_after_for_at(turn, LockAfter::Minutes30, Some(AppLockProof::PlatformVerified), T0),
        Ok(())
    );
    assert_eq!(stored(&credentials).as_deref(), Some(&b"1800"[..]));
    assert_eq!(
        engine.away_for(turn, idle(600)).check_again_in,
        Some(Duration::from_secs(1_200))
    );
    let later = T0 + beebeeb_core::screen_lock::wait_after_ms(1) + 1;
    assert_eq!(
        engine.set_lock_after_for_at(
            turn,
            LockAfter::Minutes60,
            Some(AppLockProof::Password(secret(PASSWORD))),
            later
        ),
        Ok(())
    );
    assert_eq!(engine.lock_after_for(turn), Ok(Some(LockAfter::Minutes60)));
}

#[test]
fn only_an_open_app_changes_the_delay_and_a_moved_session_writes_nothing() {
    let _serial = serial();
    let api = server();
    let (engine, credentials) = open_engine(&api.base);
    let pressed = engine.session_turn();
    engine.lock_screen().unwrap();
    assert_eq!(
        engine.set_lock_after_for_at(engine.session_turn(), LockAfter::Immediately, None, T0),
        Err(AppLockRefused::NotOpen)
    );
    engine.unlock_screen_platform_verified(engine.session_turn()).unwrap();
    assert_eq!(
        engine.set_lock_after_for_at(pressed, LockAfter::Immediately, None, T0),
        Err(AppLockRefused::Overtaken)
    );
    assert_eq!(engine.lock_after_for(pressed), Err(AppLockRefused::Overtaken));
    assert_eq!(stored(&credentials), None);
}

#[test]
fn an_idle_time_measured_for_an_earlier_turn_locks_nothing() {
    let _serial = serial();
    let api = server();
    let (engine, _credentials) = open_engine(&api.base);
    let measured = engine.session_turn();
    // The screen locked and opened again after the host measured: its time is not of this turn.
    engine.lock_screen().unwrap();
    engine.unlock_screen_platform_verified(engine.session_turn()).unwrap();
    let now = engine.session_turn();
    assert_eq!(
        engine.away_for(measured, idle(3_600)),
        AwayAnswer {
            state: LockState::Open,
            check_again_in: Some(Duration::ZERO),
        }
    );
    assert_eq!(engine.session_turn(), now);
}

/// Review of #452 (P3-4): what cannot be read as one of the choices counts as the strictest, never the default: an
/// "Immediately" that reads back as something else must not become five minutes, and no change from it skips the
/// proof a longer delay needs.
#[test]
fn an_entry_that_is_not_a_choice_is_not_known_and_counts_as_the_strictest() {
    let _serial = serial();
    let api = server();
    let (engine, credentials) = open_engine(&api.base);
    credentials
        .store_for(SLOT)
        .save_lock_flag(LockFlag::LockAfter, b"86400")
        .unwrap();
    let turn = engine.session_turn();
    assert_eq!(engine.lock_after_for(turn), Ok(None));
    assert_eq!(
        engine.set_lock_after_for_at(turn, LockAfter::Minutes5, None, T0),
        Err(AppLockRefused::ProofNeeded),
        "five minutes is longer than the strictest"
    );
    assert_eq!(
        engine.away_for(turn, idle(LockAfter::SHORTEST_AWAY.as_secs())).state,
        LockState::ScreenLocked
    );
}

#[test]
fn a_delay_that_cannot_be_read_counts_as_the_strictest() {
    let _serial = serial();
    let api = server();
    let (engine, credentials) = open_engine(&api.base);
    let turn = engine.session_turn();
    engine
        .set_lock_after_for_at(turn, LockAfter::Immediately, None, T0)
        .unwrap();
    // The credential store stops answering: a minute without input still locks.
    credentials.fail_lock_flags(true);
    let answer = engine.away_for(turn, idle(LockAfter::SHORTEST_AWAY.as_secs()));
    credentials.fail_lock_flags(false);
    assert_eq!(answer.state, LockState::ScreenLocked);
}

#[test]
fn a_sign_out_removes_the_delay_with_the_other_lock_entries() {
    let _serial = serial();
    let api = server();
    let (engine, credentials) = open_engine(&api.base);
    engine
        .set_lock_after_for_at(engine.session_turn(), LockAfter::Immediately, None, T0)
        .unwrap();
    assert!(stored(&credentials).is_some());
    assert!(LockFlag::ALL.contains(&LockFlag::LockAfter));
    engine.vault().clear_session().unwrap();
    assert_eq!(stored(&credentials), None);
}
