//! The Passkeys pane and the sign-in screen's passkey door on the real engine, against the engine's fake account on
//! loopback with the passkey routes added here: the list, a removal with the password, a late answer after the
//! session moved on, the honest refusals, and what is NOT offered. Every name and id is invented.

use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use beebeeb_engine::app::{Engine, EngineConfig};
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use beebeeb_engine::test_api::{ConfirmTokens, FakeApi, OpaqueAccount, Reply, Request};
use serde_json::json;
use zeroize::Zeroizing;

use super::*;
use crate::engine_host::EngineSignInSource;
use crate::engine_host::account::EngineAccountSource;
use crate::frames::account::source::{AccountSource, Refused};
use crate::frames::signin::source::{SignInEvent, SignInSource};

const PASSWORD: &str = "an invented password for passkeys";
const EMAIL: &str = "passkeys-host@example.eu";
const ONE: &str = "d0000000-0000-4000-8000-000000000001";
const TWO: &str = "d0000000-0000-4000-8000-000000000002";

struct Server {
    fake: FakeApi,
    account: Arc<FakeAccount>,
    phrase: String,
    /// The status the list answers with (200: the rows).
    list_status: Arc<AtomicU16>,
    /// How long the list takes to answer.
    list_delay_ms: Arc<AtomicU64>,
    deleted: Arc<Mutex<Vec<String>>>,
}

fn server() -> Server {
    let (account, phrase) = FakeAccount::register(EMAIL, PASSWORD, TEST_PHRASE);
    let account = Arc::new(account);
    let rest = account_handler(account.clone());
    let step = Arc::new(OpaqueAccount::register("passkeys-host-step@example.eu", PASSWORD));
    let tokens = Arc::new(ConfirmTokens::default());
    let list_status = Arc::new(AtomicU16::new(200));
    let list_delay_ms = Arc::new(AtomicU64::new(0));
    let deleted = Arc::new(Mutex::new(Vec::new()));
    let (account_in, status_in, delay_in, deleted_in) = (
        account.clone(),
        list_status.clone(),
        list_delay_ms.clone(),
        deleted.clone(),
    );
    let fake = FakeApi::start(Arc::new(move |req: &Request| {
        let route = req.route();
        match route.as_str() {
            "POST /api/v1/auth/confirm-opaque-start" => step.login_start(&req.json()),
            "POST /api/v1/auth/confirm-opaque-finish" => {
                if step.login_finish(&req.json()) {
                    Reply::ok(json!({ "confirmation_token": tokens.mint(), "expires_at": "soon" }))
                } else {
                    Reply::error(401, "invalid_credentials")
                }
            }
            "GET /api/v1/auth/passkeys" => {
                std::thread::sleep(Duration::from_millis(delay_in.load(Ordering::SeqCst)));
                match status_in.load(Ordering::SeqCst) {
                    200 if req.bearer() == Some(account_in.session_token.as_str()) => Reply::ok(json!({ "passkeys": [
                        { "id": ONE, "name": "An invented key", "created_at": "2026-10-28T09:00:00Z", "prf": true },
                        { "id": TWO, "name": "Another invented key", "created_at": "2026-11-02T09:00:00Z" },
                    ] })),
                    200 => Reply::error(401, "unauthorized"),
                    status => Reply::error(status, "refused"),
                }
            }
            other if other.starts_with("DELETE /api/v1/auth/passkeys/") => {
                if tokens.spend(req) {
                    deleted_in
                        .lock()
                        .unwrap()
                        .push(other.trim_start_matches("DELETE /api/v1/auth/passkeys/").to_string());
                    Reply::empty(204)
                } else {
                    Reply::error(403, "confirmation_required")
                }
            }
            _ => rest(req),
        }
    }));
    Server {
        fake,
        account,
        phrase,
        list_status,
        list_delay_ms,
        deleted,
    }
}

impl Server {
    fn asked(&self, route: &str) -> usize {
        self.fake.routes().iter().filter(|r| r.as_str() == route).count()
    }
}

type TestEngine = Arc<Engine<MemoryCredentials>>;

fn engine_on(server: &Server) -> TestEngine {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "passkeys-host-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: server.fake.base.clone(),
    };
    let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens");
    engine.restore();
    Arc::new(engine)
}

fn sign_in(engine: &TestEngine, server: &Server) {
    engine
        .sign_in_password(EMAIL, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(server.phrase.clone()))
        .expect("the phrase unlocks");
}

fn signed_in(server: &Server) -> TestEngine {
    let engine = engine_on(server);
    sign_in(&engine, server);
    engine
}

/// Pump until nothing is out any more (at most 10 s).
fn settle<C: CredentialStores + 'static>(calls: &mut PasskeyCalls<C>) {
    let began = Instant::now();
    while calls.pump().1 {
        assert!(began.elapsed() < Duration::from_secs(10), "an answer did not come back");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn opened(calls: &mut PasskeyCalls<MemoryCredentials>) {
    calls.opened(Pane::Passkeys);
    settle(calls);
}

fn pw(text: &str) -> Zeroizing<String> {
    Zeroizing::new(text.to_string())
}

#[test]
fn the_pane_loads_then_draws_the_servers_rows_and_nothing_before() {
    let server = server();
    server.list_delay_ms.store(150, Ordering::SeqCst);
    let engine = signed_in(&server);
    let mut calls = PasskeyCalls::new(engine);
    assert_eq!(calls.pane_state(), PaneState::Loading);
    calls.opened(Pane::Passkeys);
    // Asked, not answered: a loading state and no row.
    assert_eq!(calls.pane_state(), PaneState::Loading);
    assert!(calls.passkeys().is_empty());
    settle(&mut calls);
    assert_eq!(calls.pane_state(), PaneState::Ready);
    let rows = calls.passkeys();
    assert_eq!(rows.len(), 2);
    assert_eq!((rows[0].id.as_str(), rows[0].added.as_str()), (ONE, "28 Oct"));
    assert_eq!(rows[0].does(), "Can unlock data");
    // The server did not say: the row says only what is true of every passkey.
    assert_eq!((rows[1].added.as_str(), rows[1].does()), ("2 Nov", "Signs you in"));
    assert_eq!(server.asked("GET /api/v1/auth/passkeys"), 1);
    // Opened again while fresh: nothing is asked again.
    opened(&mut calls);
    assert_eq!(server.asked("GET /api/v1/auth/passkeys"), 1);
}

#[test]
fn a_failed_read_draws_no_rows_and_try_again_recovers() {
    let server = server();
    server.list_status.store(500, Ordering::SeqCst);
    let mut calls = PasskeyCalls::new(signed_in(&server));
    opened(&mut calls);
    assert_eq!(calls.pane_state(), PaneState::Failed);
    assert!(calls.passkeys().is_empty());
    // A failed read is never fresh: opening again asks again.
    server.list_status.store(200, Ordering::SeqCst);
    calls.reload();
    settle(&mut calls);
    assert_eq!(calls.pane_state(), PaneState::Ready);
    assert_eq!(calls.passkeys().len(), 2);
}

#[test]
fn removing_with_the_password_deletes_once_says_so_and_reads_the_list_again() {
    let server = server();
    let mut calls = PasskeyCalls::new(signed_in(&server));
    opened(&mut calls);
    assert!(calls.remove(ONE, pw(PASSWORD)));
    settle(&mut calls);
    assert_eq!(*server.deleted.lock().unwrap(), [ONE]);
    assert_eq!(calls.notice(), Some((REMOVED, false)));
    // Send, then read again: the list is the server's, not an edit of the old one.
    assert_eq!(server.asked("GET /api/v1/auth/passkeys"), 2);
}

#[test]
fn a_wrong_password_removes_nothing_and_says_so_in_words_that_name_it() {
    let server = server();
    let mut calls = PasskeyCalls::new(signed_in(&server));
    opened(&mut calls);
    assert!(calls.remove(ONE, pw("not the password")));
    settle(&mut calls);
    assert!(server.deleted.lock().unwrap().is_empty());
    assert_eq!(calls.notice(), Some((WRONG, true)));
    // The pane still draws the rows the server has: they were read again.
    assert_eq!(calls.passkeys().len(), 2);
}

#[test]
fn a_refusal_keeps_its_words_even_when_the_read_after_it_fails() {
    let server = server();
    let mut calls = PasskeyCalls::new(signed_in(&server));
    opened(&mut calls);
    server.list_status.store(500, Ordering::SeqCst);
    assert!(calls.remove(ONE, pw("not the password")));
    settle(&mut calls);
    assert_eq!(calls.notice(), Some((WRONG, true)));
    // The re-read failed: no row is drawn that cannot be vouched for, and the pane offers Try again.
    assert_eq!(calls.pane_state(), PaneState::Failed);
    assert!(calls.passkeys().is_empty());
}

#[test]
fn an_answer_that_lands_after_the_session_moved_on_is_dropped() {
    let server = server();
    server.list_delay_ms.store(300, Ordering::SeqCst);
    let engine = signed_in(&server);
    let mut calls = PasskeyCalls::new(engine.clone());
    calls.opened(Pane::Passkeys);
    // The session ends while the read is out.
    engine.lock();
    std::thread::sleep(Duration::from_millis(600));
    settle(&mut calls);
    assert!(calls.passkeys().is_empty());
    assert_eq!(calls.pane_state(), PaneState::SignedOut);
    // A new session of the same account starts with nothing of the first one's read.
    server.list_delay_ms.store(0, Ordering::SeqCst);
    engine
        .unlock_with_phrase(Zeroizing::new(server.phrase.clone()))
        .expect("the phrase unlocks");
    assert!(calls.passkeys().is_empty());
}

#[test]
fn a_press_on_rows_of_another_session_sends_nothing() {
    let server = server();
    let engine = signed_in(&server);
    let mut calls = PasskeyCalls::new(engine.clone());
    opened(&mut calls);
    engine.lock();
    engine
        .unlock_with_phrase(Zeroizing::new(server.phrase.clone()))
        .expect("the phrase unlocks");
    // The rows are the first session's: the press is refused here, before any worker, and nothing is sent.
    assert!(!calls.remove(ONE, pw(PASSWORD)));
    assert!(server.deleted.lock().unwrap().is_empty());
    assert_eq!(server.asked("POST /api/v1/auth/confirm-opaque-start"), 0);
}

#[test]
fn adding_with_no_dialog_to_show_says_so_and_sends_nothing() {
    let server = server();
    let mut calls = PasskeyCalls::new(signed_in(&server));
    opened(&mut calls);
    // This test process has no window to own the system's dialog (and off Windows there is no dialog at all).
    assert!(!calls.add(pw(PASSWORD)));
    assert_eq!(calls.notice(), Some((NO_DIALOG, true)));
    assert_eq!(server.asked("POST /api/v1/auth/confirm-opaque-start"), 0);
    assert!(!calls.can_rename(), "Beebeeb has no route to rename a passkey");
}

#[test]
fn the_account_source_asks_for_the_password_and_never_offers_a_rename() {
    let server = server();
    let mut source = EngineAccountSource::new(signed_in(&server));
    assert!(source.passkeys_need_password());
    assert!(!source.can_rename_passkey());
    // The calls with no password are not a way in: they claim nothing.
    assert_eq!(source.add_passkey(), Err(Refused::NotConfirmed));
    assert!(!source.rename_passkey(ONE, "A new name"));
    assert!(!source.remove_passkey(ONE));
    assert_eq!(server.asked("POST /api/v1/auth/confirm-opaque-start"), 0);
    source.opened(Pane::Passkeys);
    let began = Instant::now();
    while source.pane_state(Pane::Passkeys) != PaneState::Ready {
        source.pump();
        assert!(began.elapsed() < Duration::from_secs(10), "the list did not come back");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(source.passkeys().len(), 2);
    assert!(source.remove_passkey_confirmed(ONE, Password::from_field(PASSWORD)));
    let began = Instant::now();
    while source.notice().is_none() {
        source.pump();
        assert!(began.elapsed() < Duration::from_secs(10), "the removal did not answer");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!((source.notice(), source.notice_failed()), (Some(REMOVED), false));
    assert_eq!(*server.deleted.lock().unwrap(), [ONE]);
}

#[test]
fn the_sign_in_door_is_there_only_where_the_dialog_is_and_a_left_attempt_is_never_drawn() {
    let server = server();
    let engine = engine_on(&server);
    let mut source = EngineSignInSource::new(engine);
    // Off Windows this PC has no system dialog: the source says so, and the press answers "unavailable".
    #[cfg(not(windows))]
    {
        assert!(!source.passkey_available());
        source.begin_passkey_for(EMAIL);
        let began = Instant::now();
        loop {
            match source.poll() {
                Some(SignInEvent::Passkey(outcome)) => {
                    assert_eq!(outcome, crate::frames::signin::source::PasskeyOutcome::Unavailable);
                    break;
                }
                Some(_) => {}
                None => {
                    assert!(began.elapsed() < Duration::from_secs(10), "no answer");
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }
    }
    // The person left while the dialog was up: no `Passkey` event of that attempt is ever drawn.
    source.begin_passkey_for(EMAIL);
    source.cancel_passkey();
    std::thread::sleep(Duration::from_millis(300));
    while let Some(event) = source.poll() {
        assert!(
            !matches!(event, SignInEvent::Passkey(_)),
            "an answer of a left attempt was handed out"
        );
    }
    assert_eq!(server.asked("POST /api/v1/auth/passkey/login-finish"), 0);
}
