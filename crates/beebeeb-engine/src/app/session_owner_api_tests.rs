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

/// Census row (lane signin-wipe): a session that comes back with no email beside it (none stored, as after an email
/// write that failed at the sign-in) is named by the same answer that names its owner: the session in memory, the
/// account label a window reads, and the credential store, which still holds the token that was asked about. The email
/// is there before the id, so a window that reads "the email, else the id" never sees the id in between.
#[test]
fn signin_wipe_a_later_named_session_gets_its_email_with_its_id() {
    let (engine, _api, account) = a_session_from_before_the_owner_tag("owner-unnamed");
    // What a restart leaves of a session stored without its email: the token is stored again on its own, and the
    // session in memory and the account label know no address.
    let mut vault = engine.vault();
    let token = vault.session_token().unwrap().expect("a stored token");
    vault.clear_session().unwrap();
    vault.install_session(token).unwrap();
    assert_eq!(vault.account_email().unwrap(), None);
    engine
        .acct
        .session
        .lock()
        .unwrap()
        .as_mut()
        .expect("a session in memory")
        .email = None;
    *engine.acct.auth_email.lock().unwrap() = None;
    assert_eq!(engine.account_email(), None);

    assert_eq!(engine.settle_session_owner(), OwnerSettled::Recorded);
    assert_eq!(engine.user_id().as_deref(), Some(account.user_id.as_str()));
    assert_eq!(
        engine.account_email().as_deref(),
        Some(account.email.as_str()),
        "the account label"
    );
    assert_eq!(
        engine
            .acct
            .session
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|s| s.email.clone())
            .as_deref(),
        Some(account.email.as_str()),
        "the session in memory"
    );
    assert_eq!(
        engine.vault().account_email().unwrap().as_deref(),
        Some(account.email.as_str()),
        "the credential store, for the next start"
    );
}

/// A session that has its email keeps it: the owner's answer names only a session that has none.
#[test]
fn signin_wipe_a_named_session_keeps_its_email() {
    let (engine, _api, account) = a_session_from_before_the_owner_tag("owner-named");
    let before = engine.account_email();
    assert_eq!(before.as_deref(), Some(account.email.as_str()));
    assert_eq!(engine.settle_session_owner(), OwnerSettled::Recorded);
    assert_eq!(engine.account_email(), before);
}

/// A session whose owner id is known but whose email was never stored is asked again, and named.
#[test]
fn signin_wipe_a_session_with_an_id_but_no_email_is_named_by_the_next_ask() {
    let (engine, _api, account) = a_session_from_before_the_owner_tag("owner-id-only");
    assert_eq!(engine.settle_session_owner(), OwnerSettled::Recorded);
    let mut vault = engine.vault();
    let token = vault.session_token().unwrap().expect("a stored token");
    vault.clear_session().unwrap();
    vault.install_session(token).unwrap();
    engine.acct.session.lock().unwrap().as_mut().unwrap().email = None;
    *engine.acct.auth_email.lock().unwrap() = None;
    assert!(engine.user_id().is_some(), "the id is known");
    assert_eq!(engine.account_email(), None);

    // The minute's throttle belongs to the first ask; the ask of a later minute is the same call.
    assert_eq!(engine.settle_session_owner_now(), OwnerSettled::Recorded);
    assert_eq!(engine.account_email().as_deref(), Some(account.email.as_str()));
    assert_eq!(
        engine.vault().account_email().unwrap().as_deref(),
        Some(account.email.as_str())
    );
}
