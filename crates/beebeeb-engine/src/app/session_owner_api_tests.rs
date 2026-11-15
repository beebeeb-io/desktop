//! The owner backfill on a real handle against the engine's fake account on loopback: a session stored before the
//! owner tag gets its owner from Beebeeb once, and nothing is asked under a locked screen. Every name is invented.

use std::sync::Arc;

use zeroize::Zeroizing;

use super::super::super::{Engine, EngineConfig};
use super::*;
use crate::host::ChannelHost;
use crate::identity::REAL;
use crate::signin::memory_store::MemoryCredentials;
use crate::test_api::FakeApi;
use crate::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};

const PASSWORD: &str = "an invented password for the owner";

/// A handle signed in and unlocked, then turned into what a restart leaves of a session stored before the owner tag
/// (task 2447): no owner tag beside the stored token, and no owner id in memory.
fn a_session_from_before_the_owner_tag(tag: &str) -> (Engine<MemoryCredentials>, FakeApi, Arc<FakeAccount>) {
    let (account, phrase) = FakeAccount::register(&format!("{tag}@example.eu"), PASSWORD, TEST_PHRASE);
    let account = Arc::new(account);
    let api = FakeApi::start(account_handler(account.clone()));
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: format!("{tag}-slot"),
        state_dir: crate::state_paths::init_for_test(),
        api_base: api.base.clone(),
    };
    let engine = Engine::open(&REAL, config, host, MemoryCredentials::default()).expect("the engine opens");
    engine.restore();
    engine
        .sign_in_password(&account.email, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(phrase))
        .expect("the phrase unlocks");
    let vault = engine.vault();
    let token = vault.session_token().unwrap().expect("a stored token");
    // Storing the token again removes its owner tag first (`AuthVault::install_session`).
    vault.install_session(token).unwrap();
    assert_eq!(vault.session_owner().unwrap(), None);
    engine.remember_session_user_id(None);
    (engine, api, account)
}

/// Review of desktop#424 (delta 2 P2): the owner is learned from Beebeeb's answer for this session's own token,
/// recorded in memory and as the stored session's owner tag, without moving the session's turn; once known, nothing
/// more is asked.
#[test]
fn a_session_stored_before_the_owner_tag_gets_its_owner_from_beebeeb_once() {
    let (engine, api, account) = a_session_from_before_the_owner_tag("owner-backfill");
    let turn = engine.session_turn();
    let before = api.routes().len();
    assert_eq!(engine.settle_session_owner(), OwnerSettled::Recorded);
    assert_eq!(api.routes()[before..].to_vec(), ["GET /api/v1/auth/me".to_string()]);
    assert_eq!(
        engine.session_user_id.lock().unwrap().as_deref(),
        Some(account.user_id.as_str())
    );
    assert_eq!(
        engine.vault().session_owner().unwrap().as_deref(),
        Some(account.user_id.as_str())
    );
    assert_eq!(engine.session_turn(), turn, "learning the owner moves no turn");
    assert_eq!(engine.settle_session_owner(), OwnerSettled::Known);
    assert_eq!(api.routes().len(), before + 1, "asked once");
}

/// A locked screen asks nothing (a person's call), and does not hold back the ask after the unlock.
#[test]
fn a_locked_screen_asks_nothing_and_the_unlock_asks_at_once() {
    let (engine, api, account) = a_session_from_before_the_owner_tag("owner-locked");
    engine.lock_screen().expect("an open lock can lock the screen");
    let before = api.routes().len();
    assert_eq!(engine.settle_session_owner(), OwnerSettled::NotNow);
    assert_eq!(engine.settle_session_owner_now(), OwnerSettled::NotNow);
    assert_eq!(api.routes().len(), before, "nothing is sent under a locked screen");
    assert_eq!(*engine.session_user_id.lock().unwrap(), None);
    engine
        .unlock_screen_platform_verified(engine.session_turn())
        .expect("the platform's yes unlocks the screen");
    assert_eq!(engine.settle_session_owner(), OwnerSettled::Recorded);
    assert_eq!(
        engine.session_user_id.lock().unwrap().as_deref(),
        Some(account.user_id.as_str())
    );
}

/// The session watcher's ask runs on a thread of its own and records the owner there.
#[test]
fn the_watchers_ask_records_the_owner_off_its_own_thread() {
    let (engine, _api, account) = a_session_from_before_the_owner_tag("owner-aside");
    let engine = Arc::new(engine);
    engine.settle_session_owner_aside();
    let began = std::time::Instant::now();
    while engine.session_user_id.lock().unwrap().is_none() {
        assert!(began.elapsed() < std::time::Duration::from_secs(10), "never recorded");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(
        engine.session_user_id.lock().unwrap().as_deref(),
        Some(account.user_id.as_str())
    );
}
