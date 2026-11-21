//! The screen lock's password path at the engine, against the local fake API: core's counted tries, the server's
//! grant-nothing password proof (the real client and server sides of OPAQUE) and the local check record. Every name,
//! address and password is invented; the fake is on loopback. Times are the caller's clock (`*_at(now_ms)`): no test
//! sleeps.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use beebeeb_core::screen_lock::{AttemptState, LOCAL_CHECK_OFF_AT};
use serde_json::json;

use super::*;
use crate::account::SessionTransition;
use crate::app::EngineConfig;
use crate::app::lock_api::SessionMoved;
use crate::host::ChannelHost;
use crate::identity::TEST;
use crate::keychain::{AuthSecretStore, SecretBytes, SessionToken};
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;
use crate::test_api::{FakeApi, OpaqueAccount, Reply, Request};

type TestEngine = Engine<MemoryCredentials>;

pub(super) const PASSWORD: &str = "correct horse battery staple";
pub(super) const WRONG: &str = "not the right password at all";
const TOKEN: &str = "session-of-the-screen-lock";
const OWNER: &str = "22222222-2222-4222-8222-222222222222";
pub(super) const SLOT: &str = "an-invented-slot";
pub(super) const T0: u64 = 1_700_000_000_000;
const HOUR: u64 = 3_600_000;

const ANSWERS: u8 = 0;
const LIMITED: u8 = 1;
const REFUSED_401: u8 = 2;
const HANGS_UP: u8 = 3;
const REFUSED_401_ME_LIVES: u8 = 4;
const STALE_ONCE: u8 = 5;
const STALE_ALWAYS: u8 = 6;
static MODE: AtomicU8 = AtomicU8::new(ANSWERS);
static STALES_SENT: AtomicU8 = AtomicU8::new(0);
/// Run when the finish leg arrives (a test that moves the session while an answer is on its way).
static ON_FINISH: Mutex<Option<Box<dyn Fn() + Send>>> = Mutex::new(None);

fn account() -> Arc<OpaqueAccount> {
    static ACCOUNT: OnceLock<Arc<OpaqueAccount>> = OnceLock::new();
    ACCOUNT
        .get_or_init(|| Arc::new(OpaqueAccount::register("screen-lock@example.test", PASSWORD)))
        .clone()
}

pub(super) fn server() -> FakeApi {
    let account = account();
    FakeApi::start(Arc::new(move |request: &Request| {
        let mode = MODE.load(Ordering::SeqCst);
        if mode == HANGS_UP {
            return Reply::hang_up();
        }
        if request.bearer() != Some(TOKEN) {
            return Reply::error(401, "unauthorized");
        }
        match request.route().as_str() {
            "GET /api/v1/auth/me" if mode == REFUSED_401 => Reply::error(401, "unauthorized"),
            "GET /api/v1/auth/me" => Reply::ok(json!({ "id": OWNER })),
            "POST /api/v1/auth/password-proof/start" => match mode {
                LIMITED => Reply {
                    retry_after: Some(42),
                    ..Reply::error(429, "password_proof_limited")
                },
                REFUSED_401 | REFUSED_401_ME_LIVES => Reply::error(401, "unauthorized"),
                STALE_ONCE if STALES_SENT.fetch_add(1, Ordering::SeqCst) == 0 => {
                    Reply::error(409, "password_proof_stale")
                }
                STALE_ALWAYS => Reply::error(409, "password_proof_stale"),
                _ => account.login_start(&request.json()),
            },
            "POST /api/v1/auth/password-proof/finish" => {
                if let Some(hook) = ON_FINISH.lock().unwrap().as_ref() {
                    hook();
                }
                if account.login_finish(&request.json()) {
                    Reply::ok(json!({ "proven": true, "password_counter": 3 }))
                } else {
                    Reply::error(403, "password_not_proven")
                }
            }
            _ => Reply::error(404, "not_found"),
        }
    }))
}

pub(super) fn secret(text: &str) -> Zeroizing<String> {
    Zeroizing::new(text.to_string())
}

/// Signed in, unlocked, then SCREEN-LOCKED, with the credential store the test can read.
fn locked_engine(base: &str) -> (Arc<TestEngine>, MemoryCredentials) {
    let credentials = MemoryCredentials::default();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: SLOT.into(),
        state_dir: crate::state_paths::init_for_test(),
        api_base: base.to_string(),
    };
    let engine = Arc::new(Engine::open(&TEST, config, host, credentials.clone()).expect("the handle opens"));
    {
        let _writes = engine.writes.write().unwrap();
        let vault = engine.vault();
        vault.install_session(SessionToken::new(TOKEN).unwrap()).unwrap();
        vault.bind_session_owner(OWNER).unwrap();
        vault
            .store_wrapped_master_key(SecretBytes::new(vec![7; 32]).unwrap())
            .unwrap();
        vault.bind_vault_key_owner(OWNER).unwrap();
        engine
            .acct
            .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
        *engine.acct.session.lock().unwrap() = Some(Session {
            token: TOKEN.into(),
            master_key: [7; 32],
            email: Some("a@example.eu".into()),
        });
        engine.set_auth_present(true);
        engine.lock_follows(SessionMoved::StrongProof);
    }
    assert_eq!(engine.lock_state(), LockState::Open);
    engine.lock_screen().unwrap();
    assert_eq!(engine.lock_state(), LockState::ScreenLocked);
    (engine, credentials)
}

pub(super) fn entry(credentials: &MemoryCredentials, entry: LockFlag) -> Option<Vec<u8>> {
    credentials.store_for(SLOT).load_lock_flag(entry).unwrap()
}

fn failures(credentials: &MemoryCredentials) -> u32 {
    entry(credentials, LockFlag::Attempts).map_or(0, |bytes| AttemptState::from_bytes(&bytes).unwrap().failures())
}

fn starts(api: &FakeApi) -> usize {
    api.routes()
        .iter()
        .filter(|route| route.contains("password-proof/start"))
        .count()
}

pub(super) fn requests(api: &FakeApi) -> usize {
    api.routes().len()
}

fn stored_token(credentials: &MemoryCredentials) -> Vec<u8> {
    credentials
        .store_for(SLOT)
        .load_session_token()
        .unwrap()
        .unwrap()
        .expose_for_request()
        .as_bytes()
        .to_vec()
}

/// The tests share the loopback mode flags.
pub(super) fn serial() -> std::sync::MutexGuard<'static, ()> {
    static SERIAL: Mutex<()> = Mutex::new(());
    let guard = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    MODE.store(ANSWERS, Ordering::SeqCst);
    STALES_SENT.store(0, Ordering::SeqCst);
    *ON_FINISH.lock().unwrap() = None;
    guard
}

/// A right password, online: the record exists afterwards, and the screen is locked again.
fn enrolled(engine: &TestEngine, credentials: &MemoryCredentials, now: u64) {
    assert_eq!(
        engine.unlock_screen_with_password_at(secret(PASSWORD), now),
        Ok(ScreenUnlock::Opened)
    );
    assert!(entry(credentials, LockFlag::ScreenCheck).is_some());
    engine.lock_screen().unwrap();
}

#[test]
fn a_right_password_opens_through_the_server_and_makes_no_session() {
    let _one = serial();
    let api = server();
    let (engine, credentials) = locked_engine(&api.base);
    let token = stored_token(&credentials);
    assert_eq!(
        engine.unlock_screen_with_password_at(secret(PASSWORD), T0),
        Ok(ScreenUnlock::Opened)
    );
    assert_eq!(engine.lock_state(), LockState::Open);
    assert_eq!(entry(&credentials, LockFlag::Attempts), None, "the count is deleted");
    assert!(
        entry(&credentials, LockFlag::ScreenCheck).is_some(),
        "the record is written"
    );
    assert_eq!(stored_token(&credentials), token, "no new session was made");
    assert_eq!(
        api.routes(),
        vec![
            "POST /api/v1/auth/password-proof/start".to_string(),
            "POST /api/v1/auth/password-proof/finish".to_string()
        ],
        "the two legs of the proof and nothing else"
    );
}

#[test]
fn wrong_passwords_are_counted_and_the_fifth_waits() {
    let _one = serial();
    let api = server();
    let (engine, credentials) = locked_engine(&api.base);
    for tried in 1..=4u64 {
        assert_eq!(
            engine.unlock_screen_with_password_at(secret(WRONG), T0 + tried),
            Ok(ScreenUnlock::Wrong { wait_ms: 0 })
        );
        assert_eq!(failures(&credentials), u32::try_from(tried).unwrap());
    }
    assert_eq!(
        engine.unlock_screen_with_password_at(secret(WRONG), T0 + 5),
        Ok(ScreenUnlock::Wrong { wait_ms: 30_000 })
    );
    assert_eq!(failures(&credentials), 5);
    // A wrong password is judged on this PC, in the OPAQUE finish: the server saw five starts and no finish.
    assert_eq!(starts(&api), 5);
    // A try during the wait checks nothing and asks the server nothing.
    let before = requests(&api);
    let waiting = engine.unlock_screen_with_password_at(secret(PASSWORD), T0 + 6);
    assert_eq!(waiting, Ok(ScreenUnlock::Wait { wait_ms: 29_999 }));
    assert_eq!(requests(&api), before, "the fake saw no start request");
    assert_eq!(failures(&credentials), 5, "a wait counts nothing");
    // After the wait the right password opens and the count is gone.
    assert_eq!(
        engine.unlock_screen_with_password_at(secret(PASSWORD), T0 + 31_000),
        Ok(ScreenUnlock::Opened)
    );
    assert_eq!(entry(&credentials, LockFlag::Attempts), None);
}

#[test]
fn the_servers_own_limit_is_not_a_wrong_try() {
    let _one = serial();
    let api = server();
    let (engine, credentials) = locked_engine(&api.base);
    assert_eq!(
        engine.unlock_screen_with_password_at(secret(WRONG), T0),
        Ok(ScreenUnlock::Wrong { wait_ms: 0 })
    );
    assert_eq!(failures(&credentials), 1);
    MODE.store(LIMITED, Ordering::SeqCst);
    assert_eq!(
        engine.unlock_screen_with_password_at(secret(PASSWORD), T0 + 1),
        Ok(ScreenUnlock::ServerLimit { wait_ms: Some(42_000) })
    );
    assert_eq!(failures(&credentials), 1, "the count is what it was before the try");
    assert_eq!(engine.lock_state(), LockState::ScreenLocked);
}

#[test]
fn a_401_the_server_stands_behind_is_not_a_wrong_try_and_one_it_does_not_is_a_limit() {
    let _one = serial();
    let api = server();
    let (engine, credentials) = locked_engine(&api.base);
    // The proof says 401 but `/auth/me` says the session lives: nothing was judged.
    MODE.store(REFUSED_401_ME_LIVES, Ordering::SeqCst);
    assert_eq!(
        engine.unlock_screen_with_password_at(secret(PASSWORD), T0),
        Ok(ScreenUnlock::ServerLimit { wait_ms: None })
    );
    assert_eq!(failures(&credentials), 0);
    assert_eq!(engine.lock_state(), LockState::ScreenLocked);
    assert!(api.routes().iter().any(|route| route == "GET /api/v1/auth/me"));
}

#[test]
fn an_ended_session_closes_the_vault_and_deletes_the_count_and_the_record() {
    let _one = serial();
    let api = server();
    let (engine, credentials) = locked_engine(&api.base);
    enrolled(&engine, &credentials, T0);
    assert_eq!(
        engine.unlock_screen_with_password_at(secret(WRONG), T0 + 1),
        Ok(ScreenUnlock::Wrong { wait_ms: 0 })
    );
    MODE.store(REFUSED_401, Ordering::SeqCst);
    assert_eq!(
        engine.unlock_screen_with_password_at(secret(PASSWORD), T0 + 2),
        Ok(ScreenUnlock::SessionEnded)
    );
    assert_eq!(engine.lock_state(), LockState::VaultClosed);
    assert!(!engine.unlocked(), "the key left memory");
    assert_eq!(entry(&credentials, LockFlag::VaultClosed).as_deref(), Some(&b"1"[..]));
    assert_eq!(entry(&credentials, LockFlag::Attempts), None);
    assert_eq!(entry(&credentials, LockFlag::ScreenCheck), None);
}

#[test]
fn offline_the_local_record_decides_and_only_for_the_right_password() {
    let _one = serial();
    let api = server();
    let (engine, credentials) = locked_engine(&api.base);
    enrolled(&engine, &credentials, T0);
    MODE.store(HANGS_UP, Ordering::SeqCst);
    assert_eq!(
        engine.unlock_screen_with_password_at(secret(WRONG), T0 + 1),
        Ok(ScreenUnlock::Wrong { wait_ms: 0 })
    );
    assert_eq!(failures(&credentials), 1);
    assert_eq!(
        engine.unlock_screen_with_password_at(secret(PASSWORD), T0 + 2),
        Ok(ScreenUnlock::Opened)
    );
    assert_eq!(entry(&credentials, LockFlag::Attempts), None);
    assert_eq!(engine.lock_state(), LockState::Open);
}

#[test]
fn offline_without_a_record_nothing_opens_and_the_count_is_restored() {
    let _one = serial();
    let api = server();
    let (engine, credentials) = locked_engine(&api.base);
    MODE.store(HANGS_UP, Ordering::SeqCst);
    assert_eq!(
        engine.unlock_screen_with_password_at(secret(PASSWORD), T0),
        Ok(ScreenUnlock::OfflineNoRecord)
    );
    assert_eq!(failures(&credentials), 0, "the try judged nothing");
    assert_eq!(engine.lock_state(), LockState::ScreenLocked);
}

#[test]
fn the_tenth_wrong_try_turns_the_local_check_off_until_an_online_success() {
    let _one = serial();
    let api = server();
    let (engine, credentials) = locked_engine(&api.base);
    enrolled(&engine, &credentials, T0);
    for tried in 1..LOCAL_CHECK_OFF_AT {
        assert!(matches!(
            engine.unlock_screen_with_password_at(secret(WRONG), T0 + u64::from(tried) * 2 * HOUR),
            Ok(ScreenUnlock::Wrong { .. })
        ));
    }
    assert!(
        entry(&credentials, LockFlag::ScreenCheck).is_some(),
        "nine wrong tries keep it"
    );
    assert_eq!(
        engine.unlock_screen_with_password_at(secret(WRONG), T0 + 20 * HOUR),
        Ok(ScreenUnlock::LocalCheckOff)
    );
    assert_eq!(
        entry(&credentials, LockFlag::ScreenCheck),
        None,
        "the record is deleted"
    );
    // Offline, the right password never opens again until an online success.
    MODE.store(HANGS_UP, Ordering::SeqCst);
    assert_eq!(
        engine.unlock_screen_with_password_at(secret(PASSWORD), T0 + 40 * HOUR),
        Ok(ScreenUnlock::OfflineNoRecord)
    );
    MODE.store(ANSWERS, Ordering::SeqCst);
    assert_eq!(
        engine.unlock_screen_with_password_at(secret(PASSWORD), T0 + 60 * HOUR),
        Ok(ScreenUnlock::Opened)
    );
    assert!(entry(&credentials, LockFlag::ScreenCheck).is_some());
}

#[test]
fn the_count_is_written_before_the_check_and_a_store_that_refuses_stops_the_try() {
    let _one = serial();
    let api = server();
    let (engine, credentials) = locked_engine(&api.base);
    credentials.fail_lock_writes(true);
    assert_eq!(
        engine.unlock_screen_with_password_at(secret(PASSWORD), T0),
        Err(LockRefused::CouldNotCheck)
    );
    assert_eq!(requests(&api), 0, "nothing was sent");
    assert_eq!(engine.lock_state(), LockState::ScreenLocked);
}

#[test]
fn an_unreadable_count_buys_no_tries() {
    let _one = serial();
    let api = server();
    let (engine, credentials) = locked_engine(&api.base);
    credentials
        .store_for(SLOT)
        .save_lock_flag(LockFlag::Attempts, b"garbage")
        .unwrap();
    let answer = engine.unlock_screen_with_password_at(secret(PASSWORD), T0);
    assert!(
        matches!(answer, Ok(ScreenUnlock::Wait { wait_ms }) if wait_ms > 0),
        "{answer:?}"
    );
    assert_eq!(requests(&api), 0, "nothing was sent");
    assert_eq!(engine.lock_state(), LockState::ScreenLocked);
}

#[test]
fn an_answer_that_arrives_after_the_session_moved_opens_nothing() {
    let _one = serial();
    let api = server();
    let (engine, _credentials) = locked_engine(&api.base);
    let mover = engine.clone();
    *ON_FINISH.lock().unwrap() = Some(Box::new(move || {
        let _writes = mover.writes.write().unwrap();
        mover
            .acct
            .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
    }));
    assert_eq!(
        engine.unlock_screen_with_password_at(secret(PASSWORD), T0),
        Err(LockRefused::Overtaken)
    );
    assert_eq!(engine.lock_state(), LockState::ScreenLocked, "still locked");
}

#[test]
fn a_press_while_a_check_runs_checks_nothing_and_an_empty_password_is_not_a_try() {
    let _one = serial();
    let api = server();
    let (engine, credentials) = locked_engine(&api.base);
    {
        let _running = engine.lock_password.one_at_a_time.lock().unwrap();
        assert_eq!(
            engine.unlock_screen_with_password_at(secret(PASSWORD), T0),
            Err(LockRefused::AlreadyChecking)
        );
    }
    assert_eq!(
        engine.unlock_screen_with_password_at(secret(""), T0),
        Err(LockRefused::NothingTyped)
    );
    assert_eq!(requests(&api), 0);
    assert_eq!(failures(&credentials), 0);
}

#[test]
fn a_start_that_went_stale_is_begun_again_once() {
    let _one = serial();
    let api = server();
    let (engine, _credentials) = locked_engine(&api.base);
    MODE.store(STALE_ONCE, Ordering::SeqCst);
    assert_eq!(
        engine.unlock_screen_with_password_at(secret(PASSWORD), T0),
        Ok(ScreenUnlock::Opened)
    );
    assert_eq!(starts(&api), 2);
    engine.lock_screen().unwrap();
    MODE.store(STALE_ALWAYS, Ordering::SeqCst);
    assert_eq!(
        engine.unlock_screen_with_password_at(secret(PASSWORD), T0 + 1),
        Err(LockRefused::CouldNotCheck)
    );
}

#[test]
fn the_screen_password_path_is_for_a_locked_screen_only() {
    let _one = serial();
    let api = server();
    let (engine, _credentials) = locked_engine(&api.base);
    engine.unlock_screen_platform_verified(engine.session_turn()).unwrap();
    assert_eq!(
        engine.unlock_screen_with_password_at(secret(PASSWORD), T0),
        Err(LockRefused::NotAtThisLevel)
    );
    assert_eq!(requests(&api), 0);
}

pub(super) fn open_engine(base: &str) -> (Arc<TestEngine>, MemoryCredentials) {
    let (engine, credentials) = locked_engine(base);
    engine.unlock_screen_platform_verified(engine.session_turn()).unwrap();
    assert_eq!(engine.lock_state(), LockState::Open);
    (engine, credentials)
}

#[test]
fn the_start_setting_turns_on_asking_nothing_and_off_only_with_the_password_proven_online() {
    let _one = serial();
    let api = server();
    let (engine, credentials) = open_engine(&api.base);
    assert_eq!(engine.set_ask_password_at_start_at(true, None, T0), Ok(()));
    assert_eq!(engine.ask_password_at_start(), Some(true));
    assert_eq!(requests(&api), 0, "turning it on asks nothing");
    assert_eq!(
        engine.set_ask_password_at_start_at(false, None, T0),
        Err(StartSettingRefused::PasswordNeeded)
    );
    assert_eq!(
        engine.set_ask_password_at_start_at(false, Some(secret(WRONG)), T0),
        Err(StartSettingRefused::Wrong {
            wait_ms: beebeeb_core::screen_lock::wait_after_ms(1),
            local_check_off: false
        })
    );
    assert_eq!(
        engine.ask_password_at_start(),
        Some(true),
        "a wrong password changes nothing"
    );
    assert_eq!(failures(&credentials), 1, "the wrong-try rule counts these tries too");
    enrolled_open(&engine);
    MODE.store(HANGS_UP, Ordering::SeqCst);
    assert_eq!(
        engine.set_ask_password_at_start_at(false, Some(secret(PASSWORD)), T0 + 1),
        Err(StartSettingRefused::Offline)
    );
    assert_eq!(engine.ask_password_at_start(), Some(true), "never turned off offline");
    MODE.store(ANSWERS, Ordering::SeqCst);
    assert_eq!(
        engine.set_ask_password_at_start_at(false, Some(secret(PASSWORD)), T0 + 2),
        Ok(())
    );
    assert_eq!(engine.ask_password_at_start(), Some(false));
}

/// A local record exists (so an offline check COULD answer): the setting still must not use it.
fn enrolled_open(engine: &TestEngine) {
    engine.lock_screen().unwrap();
    assert_eq!(
        engine.unlock_screen_with_password_at(secret(PASSWORD), T0 + 3 * HOUR),
        Ok(ScreenUnlock::Opened)
    );
}

#[test]
fn a_password_confirmed_online_changes_no_lock_and_waits_like_any_try() {
    let _one = serial();
    let api = server();
    let (engine, credentials) = open_engine(&api.base);
    assert_eq!(engine.confirm_password_online_at(secret(PASSWORD), T0), Ok(()));
    assert_eq!(engine.lock_state(), LockState::Open);
    assert_eq!(
        engine.confirm_password_online_at(secret(""), T0),
        Err(StartSettingRefused::PasswordNeeded)
    );
    MODE.store(LIMITED, Ordering::SeqCst);
    assert_eq!(
        engine.confirm_password_online_at(secret(PASSWORD), T0 + 1),
        Err(StartSettingRefused::ServerLimit { wait_ms: Some(42_000) })
    );
    MODE.store(ANSWERS, Ordering::SeqCst);
    for tried in 0..5u64 {
        let _ = engine.confirm_password_online_at(secret(WRONG), T0 + 10 + tried);
    }
    assert_eq!(failures(&credentials), 5);
    assert_eq!(
        engine.confirm_password_online_at(secret(PASSWORD), T0 + 20),
        Err(StartSettingRefused::Wait { wait_ms: 29_994 })
    );
}

#[test]
fn nothing_in_the_password_path_logs() {
    let source =
        std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/app/lock_password_api.rs"))
            .unwrap();
    let code: String = source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect();
    for needle in ["tracing::", "println!", "eprintln!", "dbg!", "log::"] {
        assert!(!code.contains(needle), "{needle} in the password path");
    }
}

#[path = "screen_unlock_ask_tests.rs"]
mod screen_unlock_ask;
