//! The account sections on the real engine (`EngineAccountSource`), against the engine's fake accounts on
//! loopback with the account routes added here (the display name, the plan, the sessions and their sign-outs):
//! what Account and Devices and sessions draw, every way a read can fail, a sign-out sent once and shown only
//! after Beebeeb's list no longer has the session, a session that changes on the way, another account, and the
//! running app's window. Every name is invented.

use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use beebeeb_engine::app::EngineConfig;
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use beebeeb_engine::test_api::{FakeApi, Handler, Reply, Request};
use rune::{Env, Runtime, Tx};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::*;
use crate::State;
use crate::shell::app::{Event as ShellEvent, SettingsPane};
use crate::shell::host;

const PASSWORD: &str = "an invented password for the account";
const FIRST: &str = "account-first@example.eu";
const SECOND: &str = "account-second@example.eu";
const FIRST_NAME: &str = "Invented Person";
const PHONE: &str = "An invented phone";
const BROWSER: &str = "An invented browser";
const THIS_PC: &str = "An invented PC";

const ANSWER: u8 = 0;
const SERVER_ERROR: u8 = 1;
const SESSION_ENDED: u8 = 2;
const EMPTY: u8 = 3;
const UNREADABLE: u8 = 4;
const DROP_CONNECTION: u8 = 5;

struct Person {
    account: Arc<FakeAccount>,
    phrase: String,
    name: Option<&'static str>,
    sessions: Mutex<Vec<Value>>,
}

/// One fake API for two accounts, with the account routes; the accounts' own handlers answer the rest.
struct Server {
    fake: FakeApi,
    people: [Arc<Person>; 2],
    /// How the display name and the sessions list answer.
    read_mode: Arc<AtomicU8>,
    plan_mode: Arc<AtomicU8>,
    /// How a sign-out answers.
    change_mode: Arc<AtomicU8>,
    read_delay_ms: Arc<AtomicU64>,
    change_delay_ms: Arc<AtomicU64>,
    /// Every session id a DELETE named, in order, and how many times "all others" was asked.
    deleted: Arc<Mutex<Vec<String>>>,
    revoked_all: Arc<AtomicU64>,
    /// Runs once, while the next sessions list is being answered (the call is under way).
    during_read: Arc<Mutex<Option<Box<dyn FnOnce() + Send>>>>,
}

fn sessions_of_the_first() -> Vec<Value> {
    let ago = |minutes: i64| (Utc::now() - chrono::Duration::minutes(minutes)).to_rfc3339();
    vec![
        json!({ "id": "s-this", "device_name": THIS_PC, "device_kind": "desktop", "country_code": "nl",
                "last_active_at": ago(0), "created_at": ago(900), "is_current": true }),
        json!({ "id": "s-phone", "device_name": PHONE, "device_kind": "ios", "country_code": "DE",
                "last_active_at": ago(12), "created_at": ago(900), "is_current": false }),
        json!({ "id": "s-web", "device_name": BROWSER, "device_kind": "web", "country_code": "an-invented-city",
                "last_active_at": null, "created_at": ago(900), "is_current": false }),
    ]
}

fn person(email: &str, name: Option<&'static str>, sessions: Vec<Value>) -> Arc<Person> {
    let (account, phrase) = FakeAccount::register(email, PASSWORD, TEST_PHRASE);
    Arc::new(Person {
        account: Arc::new(account),
        phrase,
        name,
        sessions: Mutex::new(sessions),
    })
}

fn failed(mode: u8) -> Option<Reply> {
    match mode {
        SERVER_ERROR => Some(Reply::json(500, json!({ "error": "a server body" }))),
        SESSION_ENDED => Some(Reply::json(401, json!({ "error": "a server body" }))),
        UNREADABLE => Some(Reply::ok(json!("not the expected shape"))),
        DROP_CONNECTION => Some(Reply::empty(0)),
        _ => None,
    }
}

fn server() -> Server {
    let people = [
        person(FIRST, Some(FIRST_NAME), sessions_of_the_first()),
        person(
            SECOND,
            None,
            vec![
                json!({ "id": "t-this", "device_name": "Another invented PC", "device_kind": "desktop",
                         "country_code": "FI", "last_active_at": null, "is_current": true }),
            ],
        ),
    ];
    let rest: [Handler; 2] = [
        account_handler(people[0].account.clone()),
        account_handler(people[1].account.clone()),
    ];
    let (read_mode, plan_mode, change_mode) = (
        Arc::new(AtomicU8::new(ANSWER)),
        Arc::new(AtomicU8::new(ANSWER)),
        Arc::new(AtomicU8::new(ANSWER)),
    );
    let (read_delay_ms, change_delay_ms) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
    let (deleted, revoked_all) = (Arc::new(Mutex::new(Vec::new())), Arc::new(AtomicU64::new(0)));
    let (people_in, read_in, plan_in, change_in, read_delay_in, change_delay_in, deleted_in, revoked_in) = (
        people.clone(),
        read_mode.clone(),
        plan_mode.clone(),
        change_mode.clone(),
        read_delay_ms.clone(),
        change_delay_ms.clone(),
        deleted.clone(),
        revoked_all.clone(),
    );
    let during_read: Arc<Mutex<Option<Box<dyn FnOnce() + Send>>>> = Arc::new(Mutex::new(None));
    let during_read_in = during_read.clone();
    let signing_in_second = Mutex::new(false);
    let fake = FakeApi::start(Arc::new(move |req: &Request| {
        let who = match req.bearer() {
            Some(token) => usize::from(token == people_in[1].account.session_token),
            None => {
                let mut second = signing_in_second.lock().unwrap();
                if let Some(email) = req.json().get("email").and_then(|email| email.as_str()) {
                    *second = email == SECOND;
                }
                usize::from(*second)
            }
        };
        let person = &people_in[who];
        let route = req.route();
        let (read, plan, change) = (
            read_in.load(Ordering::SeqCst),
            plan_in.load(Ordering::SeqCst),
            change_in.load(Ordering::SeqCst),
        );
        let authed = req.bearer() == Some(person.account.session_token.as_str());
        match route.as_str() {
            "GET /api/v1/me/profile" => failed(read).unwrap_or_else(|| {
                if !authed {
                    return Reply::json(401, json!({}));
                }
                Reply::ok(json!({ "username": null, "display_name": person.name }))
            }),
            "GET /api/v1/billing/subscription" if authed => failed(plan).unwrap_or_else(|| {
                let first = who == 0;
                Reply::ok(json!({
                    // What the sign-in gate reads.
                    "plan_name": "Pro", "storage_limit_bytes": person.account.quota_bytes,
                    // What the Account pane reads.
                    "plan": "personal", "billing_cycle": "yearly", "status": "active", "seats": 1, "region": "eu",
                    "created_at": "2025-01-01T00:00:00Z",
                    "current_period_end": if first { json!("2027-03-14T12:00:00Z") } else { Value::Null },
                    "quota_bytes": if plan == EMPTY { 0 } else { 1_000_000_000_000_i64 },
                    "used_bytes": if first { 240_000_000_000_i64 } else { 1_500_000_000 },
                }))
            }),
            "GET /api/v1/account/sessions" => failed(read).unwrap_or_else(|| {
                if !authed {
                    return Reply::json(401, json!({}));
                }
                let now = person.sessions.lock().unwrap().clone();
                let hook = during_read_in.lock().unwrap().take();
                if let Some(hook) = hook {
                    hook();
                }
                std::thread::sleep(Duration::from_millis(read_delay_in.load(Ordering::SeqCst)));
                Reply::ok(json!({ "sessions": if read == EMPTY { Vec::new() } else { now } }))
            }),
            "POST /api/v1/account/sessions/revoke-all-others" => failed(change).unwrap_or_else(|| {
                if !authed {
                    return Reply::json(401, json!({}));
                }
                std::thread::sleep(Duration::from_millis(change_delay_in.load(Ordering::SeqCst)));
                revoked_in.fetch_add(1, Ordering::SeqCst);
                let mut sessions = person.sessions.lock().unwrap();
                let before = sessions.len();
                sessions.retain(|s| s["is_current"] == json!(true));
                Reply::ok(json!({ "revoked": before - sessions.len() }))
            }),
            other if other.starts_with("DELETE /api/v1/account/sessions/") => {
                let id = other.rsplit('/').next().unwrap_or_default().to_string();
                std::thread::sleep(Duration::from_millis(change_delay_in.load(Ordering::SeqCst)));
                deleted_in.lock().unwrap().push(id.clone());
                failed(change).unwrap_or_else(|| {
                    if !authed {
                        return Reply::json(401, json!({}));
                    }
                    person.sessions.lock().unwrap().retain(|s| s["id"] != json!(id));
                    Reply::ok(json!({}))
                })
            }
            _ => rest[who](req),
        }
    }));
    Server {
        fake,
        people,
        read_mode,
        plan_mode,
        change_mode,
        read_delay_ms,
        change_delay_ms,
        deleted,
        revoked_all,
        during_read,
    }
}

impl Server {
    fn asked(&self, route: &str) -> usize {
        self.fake.routes().iter().filter(|r| r.as_str() == route).count()
    }
}

type TestEngine = Arc<Engine<MemoryCredentials>>;

/// A handle of its own on the fake, signed out.
fn engine_on(server: &Server) -> TestEngine {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "account-test-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: server.fake.base.clone(),
    };
    let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens");
    engine.restore();
    Arc::new(engine)
}

fn sign_in(engine: &TestEngine, person: &Person) {
    engine
        .sign_in_password(&person.account.email, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(person.phrase.clone()))
        .expect("the phrase unlocks");
}

fn signed_in(server: &Server) -> TestEngine {
    let engine = engine_on(server);
    sign_in(&engine, &server.people[0]);
    engine
}

type Source = EngineAccountSource<MemoryCredentials>;

/// Pump until nothing is out any more (at most 10 s).
fn settle(source: &mut Source) {
    let began = Instant::now();
    while source.pump().busy {
        assert!(began.elapsed() < Duration::from_secs(10), "an answer did not come back");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn opened(source: &mut Source, pane: Pane) {
    source.opened(pane);
    settle(source);
}

fn ids(source: &Source) -> Vec<String> {
    source.sessions().into_iter().map(|s| s.id).collect()
}

#[test]
fn account_draws_who_is_signed_in_and_the_plan() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    assert_eq!(
        server.asked("GET /api/v1/me/profile"),
        0,
        "nothing is asked before a pane opens"
    );
    assert_eq!(source.pane_state(Pane::Account), PaneState::Loading);
    assert_eq!(source.identity().email, "", "nothing of the account before it is read");
    opened(&mut source, Pane::Account);
    assert_eq!(source.pane_state(Pane::Account), PaneState::Ready);
    assert_eq!(
        source.identity(),
        Identity {
            name: FIRST_NAME.into(),
            initials: "IP".into(),
            email: FIRST.into()
        }
    );
    assert_eq!(
        source.plan(),
        Some(Plan {
            title: "Drive plan \u{b7} 1 TB".into(),
            used: "240 GB".into(),
            rest: "used \u{b7} renews 14 Mar 2027".into(),
        })
    );
    let details = source
        .plan_details()
        .expect("a renewal day and a billing period: the dialog opens");
    assert_eq!(
        (
            details.size.as_str(),
            details.used.as_str(),
            details.renews.as_str(),
            details.billing.as_str()
        ),
        ("1 TB", "240 GB", "14 Mar 2027", "Yearly")
    );
    assert!((details.share - 0.24).abs() < 0.001);
    assert_eq!(
        source.products(),
        [(ProductKind::Drive, ProductState::ThisApp)],
        "this app only: no product that was not checked is listed"
    );
    // Fresh: opening again asks nothing.
    opened(&mut source, Pane::Account);
    assert_eq!(server.asked("GET /api/v1/me/profile"), 1);
}

#[test]
fn only_account_and_devices_answer_every_other_pane_is_the_shells_frame() {
    let server = server();
    let source = Source::new(signed_in(&server));
    for pane in [
        Pane::Security,
        Pane::Checkup,
        Pane::Name,
        Pane::Email,
        Pane::EmailCode,
        Pane::Password,
        Pane::TwoFactor,
        Pane::TwoFactorSetup,
        Pane::BackupCodes,
        Pane::Passkeys,
        Pane::Activity,
        Pane::Delete,
        Pane::DeleteFactor,
        Pane::DeleteLink,
        Pane::DeleteScheduled,
        Pane::DeleteUndone,
        Pane::ApproveCode,
        Pane::ApproveReview,
    ] {
        assert_eq!(source.pane_state(pane), PaneState::Unwired, "{pane:?}");
    }
    assert_ne!(source.pane_state(Pane::Account), PaneState::Unwired);
    assert_ne!(source.pane_state(Pane::Sessions), PaneState::Unwired);
}

#[test]
fn an_account_without_a_name_or_a_plan_draws_the_email_and_no_card() {
    let server = server();
    let engine = engine_on(&server);
    sign_in(&engine, &server.people[1]);
    let mut source = Source::new(engine);
    opened(&mut source, Pane::Account);
    let who = source.identity();
    assert_eq!(
        (who.name.as_str(), who.email.as_str()),
        (SECOND, SECOND),
        "no name set: the email"
    );
    assert_eq!(who.initials, "AE");
    let plan = source.plan().expect("a plan with a size");
    assert_eq!(
        (plan.used.as_str(), plan.rest.as_str()),
        ("1.5 GB", "used"),
        "no renewal day: none is said"
    );
    assert!(
        source.plan_details().is_none(),
        "no renewal day: the dialog is not opened"
    );

    // A plan without a size, and a plan that does not read: no card, never a guessed plan.
    for mode in [EMPTY, UNREADABLE, SERVER_ERROR] {
        server.plan_mode.store(mode, Ordering::SeqCst);
        source.reload(Pane::Account);
        settle(&mut source);
        assert_eq!(source.pane_state(Pane::Account), PaneState::Ready, "mode {mode}");
        assert!(
            source.plan().is_none() && source.plan_details().is_none(),
            "mode {mode}"
        );
        assert_eq!(source.identity().email, SECOND);
    }
}

#[test]
fn a_read_that_fails_draws_nothing_of_the_account_and_is_asked_again() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Account);
    opened(&mut source, Pane::Sessions);
    assert_eq!(ids(&source).len(), 3);
    for mode in [SERVER_ERROR, UNREADABLE, DROP_CONNECTION] {
        server.read_mode.store(mode, Ordering::SeqCst);
        for pane in [Pane::Account, Pane::Sessions] {
            source.reload(pane);
            settle(&mut source);
            assert_eq!(source.pane_state(pane), PaneState::Failed, "{pane:?}, mode {mode}");
        }
        assert_eq!(source.identity().email, "", "mode {mode}");
        assert!(source.plan().is_none() && source.products().is_empty());
        assert!(source.sessions().is_empty());
        // A failed read is not fresh: the next opening asks again.
        let before = server.asked("GET /api/v1/account/sessions");
        opened(&mut source, Pane::Sessions);
        assert_eq!(server.asked("GET /api/v1/account/sessions"), before + 1, "mode {mode}");
    }
    server.read_mode.store(ANSWER, Ordering::SeqCst);
    source.reload(Pane::Account);
    source.reload(Pane::Sessions);
    settle(&mut source);
    assert_eq!(source.pane_state(Pane::Account), PaneState::Ready);
    assert_eq!(source.identity().email, FIRST);
    assert_eq!(ids(&source).len(), 3);
}

#[test]
fn a_session_beebeeb_ended_is_said_and_not_asked_again() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    assert_eq!(ids(&source).len(), 3);
    server.read_mode.store(SESSION_ENDED, Ordering::SeqCst);
    source.reload(Pane::Sessions);
    settle(&mut source);
    assert_eq!(source.pane_state(Pane::Sessions), PaneState::SessionEnded);
    assert_eq!(source.pane_state(Pane::Account), PaneState::SessionEnded);
    assert!(source.sessions().is_empty(), "nothing of the account after a 401");
    let asked = server.fake.routes().len();
    opened(&mut source, Pane::Sessions);
    opened(&mut source, Pane::Account);
    source.reload(Pane::Sessions);
    assert!(!source.sign_out_device("s-phone"));
    source.sign_out_everywhere_else();
    settle(&mut source);
    assert_eq!(
        server.fake.routes().len(),
        asked,
        "nothing is asked or sent for an ended session"
    );
}

#[test]
fn signed_out_or_locked_nothing_is_asked_and_nothing_of_an_account_is_held() {
    let server = server();
    let engine = engine_on(&server);
    let mut source = Source::new(engine.clone());
    let asked = server.fake.routes().len();
    opened(&mut source, Pane::Account);
    opened(&mut source, Pane::Sessions);
    assert_eq!(source.pane_state(Pane::Account), PaneState::SignedOut);
    assert_eq!(source.pane_state(Pane::Sessions), PaneState::SignedOut);
    assert_eq!(server.fake.routes().len(), asked, "signed out: nothing is asked");

    sign_in(&engine, &server.people[0]);
    // The pane that shows is read for the new session without a reopening.
    assert!(source.pump().changed);
    settle(&mut source);
    assert_eq!(ids(&source).len(), 3);
    engine.lock();
    assert_eq!(source.pane_state(Pane::Sessions), PaneState::SignedOut, "locked");
    assert!(
        source.sessions().is_empty(),
        "an accessor hands out nothing before the poll saw the lock"
    );
    assert!(!source.sign_out_device("s-phone"), "and nothing is sent");
    assert!(source.pump().changed, "the poll drops what it held");
    assert!(server.deleted.lock().unwrap().is_empty());
}

#[test]
fn devices_and_sessions_lists_one_row_per_session_by_country() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    let rows = source.sessions();
    let brief: Vec<_> = rows
        .iter()
        .map(|s| {
            (
                s.id.as_str(),
                s.name.as_str(),
                s.kind,
                s.apps.as_str(),
                s.last_active.as_str(),
                s.country.as_str(),
                s.this_device,
            )
        })
        .collect();
    assert_eq!(
        brief,
        [
            ("s-this", THIS_PC, DeviceKind::Laptop, "Desktop app", "now", "NL", true),
            (
                "s-phone",
                PHONE,
                DeviceKind::Phone,
                "Mobile app",
                "12 min ago",
                "DE",
                false
            ),
            // Not two letters: never shown (a city can not reach the pane). No time: said so.
            (
                "s-web",
                BROWSER,
                DeviceKind::Browser,
                "Web browser",
                "last active not known",
                "",
                false
            ),
        ]
    );
    for row in &rows {
        assert!(!row.apps_sentence.contains(" and "), "no product list is claimed");
    }

    // The empty answer: the pane is ready, with no row.
    server.read_mode.store(EMPTY, Ordering::SeqCst);
    source.reload(Pane::Sessions);
    settle(&mut source);
    assert_eq!(source.pane_state(Pane::Sessions), PaneState::Ready);
    assert!(source.sessions().is_empty());
}

#[test]
fn a_sign_out_is_sent_once_by_id_and_the_row_goes_only_when_beebeeb_says_so() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);

    // Never this session, never an id the list does not have, never the plural through an id.
    for id in ["s-this", "s-unknown", "", "*"] {
        assert!(!source.sign_out_device(id), "{id:?}");
    }
    settle(&mut source);
    assert!(server.deleted.lock().unwrap().is_empty());
    assert_eq!(server.revoked_all.load(Ordering::SeqCst), 0);
    assert_eq!(source.notice(), None);

    server.change_delay_ms.store(300, Ordering::SeqCst);
    assert!(source.sign_out_device("s-phone"));
    assert!(
        !source.sign_out_device("s-phone"),
        "a second press while it is on its way sends nothing"
    );
    assert_eq!(source.notice(), Some(STILL_SIGNING_OUT));
    assert_eq!(
        ids(&source),
        ["s-this", "s-phone", "s-web"],
        "the row stays until Beebeeb answered"
    );
    settle(&mut source);
    assert_eq!(
        server.deleted.lock().unwrap().clone(),
        ["s-phone"],
        "sent once, by its id"
    );
    assert_eq!(source.notice(), Some(SIGNED_OUT));
    assert_eq!(ids(&source), ["s-this", "s-web"], "read again after the answer");

    // The mark is cleared: another session can be signed out, and the same press works again later.
    server.change_delay_ms.store(0, Ordering::SeqCst);
    assert!(source.sign_out_device("s-web"));
    assert_eq!(source.notice(), None, "the old toast goes with the next press");
    settle(&mut source);
    assert_eq!(ids(&source), ["s-this"]);
    assert_eq!(server.deleted.lock().unwrap().len(), 2);
    source.clear_notice();
    assert_eq!(source.notice(), None);
}

#[test]
fn a_refused_sign_out_changes_nothing_and_an_unconfirmed_one_reads_again() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);

    server.change_mode.store(SERVER_ERROR, Ordering::SeqCst);
    assert!(source.sign_out_device("s-phone"));
    let reads = server.asked("GET /api/v1/account/sessions");
    settle(&mut source);
    assert_eq!(source.notice(), Some(NOT_SIGNED_OUT));
    assert_eq!(ids(&source).len(), 3, "the row stands");
    assert_eq!(
        server.asked("GET /api/v1/account/sessions"),
        reads,
        "a refusal needs no re-read"
    );
    // Refused, not stuck: the press works again.
    source.sign_out_everywhere_else();
    settle(&mut source);
    assert_eq!(source.notice(), Some(NOT_SIGNED_OUT));
    assert_eq!(server.revoked_all.load(Ordering::SeqCst), 0);

    // The connection broke: not known, so the list is read again; and when that read fails too, the words stay.
    server.change_mode.store(DROP_CONNECTION, Ordering::SeqCst);
    assert!(source.sign_out_device("s-phone"));
    settle(&mut source);
    assert_eq!(source.notice(), Some(UNKNOWN_SIGN_OUT));
    assert_eq!(server.asked("GET /api/v1/account/sessions"), reads + 1);
    server.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    assert!(source.sign_out_device("s-phone"));
    settle(&mut source);
    assert_eq!(
        source.notice(),
        Some(UNKNOWN_SIGN_OUT),
        "the re-read failed: the words stay"
    );
    assert_eq!(source.pane_state(Pane::Sessions), PaneState::Failed);

    // Made, then the re-read fails: said as made, and the list is not drawn from before the change.
    server.read_mode.store(ANSWER, Ordering::SeqCst);
    source.reload(Pane::Sessions);
    settle(&mut source);
    server.change_mode.store(ANSWER, Ordering::SeqCst);
    assert!(source.sign_out_device("s-phone"));
    server.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    settle(&mut source);
    assert_eq!(source.notice(), Some(SIGNED_OUT));
    assert!(source.sessions().is_empty(), "never the list from before the change");
}

#[test]
fn sign_out_everywhere_else_is_sent_once_and_leaves_this_session() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    server.change_delay_ms.store(200, Ordering::SeqCst);
    source.sign_out_everywhere_else();
    source.sign_out_everywhere_else();
    assert_eq!(source.notice(), Some(STILL_SIGNING_OUT));
    settle(&mut source);
    assert_eq!(server.revoked_all.load(Ordering::SeqCst), 1, "sent once");
    assert!(server.deleted.lock().unwrap().is_empty());
    assert_eq!(source.notice(), Some(SIGNED_OUT_ELSEWHERE));
    assert_eq!(ids(&source), ["s-this"]);
    // Nobody else is signed in: nothing to send.
    source.sign_out_everywhere_else();
    settle(&mut source);
    assert_eq!(server.revoked_all.load(Ordering::SeqCst), 1);
}

#[test]
fn a_session_that_changes_while_a_sign_out_is_on_its_way_says_nothing_to_the_next_one() {
    let server = server();
    let engine = signed_in(&server);
    let mut source = Source::new(engine.clone());
    opened(&mut source, Pane::Sessions);
    server.change_delay_ms.store(300, Ordering::SeqCst);
    assert!(source.sign_out_device("s-phone"));
    // Lock, then unlock: another session of the same account.
    engine.lock();
    engine
        .unlock_with_phrase(Zeroizing::new(server.people[0].phrase.clone()))
        .expect("the phrase unlocks");
    assert!(
        source.sessions().is_empty(),
        "the list of the session before is not handed out"
    );
    settle(&mut source);
    assert_eq!(source.notice(), None, "the answer of the session before is not said");
    assert_eq!(ids(&source), ["s-this", "s-web"], "the new session read its own list");
    assert_eq!(source.pump(), Pumped::default(), "no mark of the old change is left");
}

#[test]
fn another_account_never_sees_the_first_ones_name_plan_or_sessions() {
    let server = server();
    let engine = signed_in(&server);
    let mut source = Source::new(engine.clone());
    opened(&mut source, Pane::Account);
    opened(&mut source, Pane::Sessions);
    assert_eq!(source.identity().name, FIRST_NAME);
    // The first account's list is slow: its answer lands after the second account signed in.
    server.read_delay_ms.store(400, Ordering::SeqCst);
    source.reload(Pane::Sessions);
    engine.sign_out_keeping_local_data().expect("signs out");
    assert_eq!(source.identity().email, "");
    assert!(source.sessions().is_empty() && source.plan().is_none());
    sign_in(&engine, &server.people[1]);
    assert!(
        source.sessions().is_empty(),
        "before the poll: still nothing of the first"
    );
    assert_eq!(source.identity().email, "");
    server.read_delay_ms.store(0, Ordering::SeqCst);
    settle(&mut source);
    std::thread::sleep(Duration::from_millis(500));
    settle(&mut source);
    assert_eq!(ids(&source), ["t-this"], "the second account's own list");
    opened(&mut source, Pane::Account);
    assert_eq!(source.identity().email, SECOND);
    assert!(
        !source.sign_out_device("s-phone"),
        "a row of the first account is not a row of this list"
    );
    assert!(server.deleted.lock().unwrap().is_empty());
}

/// The engine's own wrappers against its fake: the answer names its session; a change for a session that is not
/// the one in memory sends nothing.
#[test]
fn the_engines_account_calls_answer_for_their_session_and_send_only_for_it() {
    let server = server();
    let engine = signed_in(&server);
    let turn = engine.session_turn();
    let profile = engine.account_profile().expect("answers");
    assert_eq!((profile.session, profile.value.email.as_str()), (turn, FIRST));
    assert!(
        !format!("{profile:?}").contains(FIRST),
        "an answer's Debug names no email"
    );
    assert_eq!(
        engine.account_display_name().expect("answers").value.as_deref(),
        Some(FIRST_NAME)
    );
    let plan = engine.account_subscription().expect("answers");
    assert_eq!((plan.session, plan.value.quota_bytes), (turn, 1_000_000_000_000));
    let list = engine.account_sessions().expect("answers");
    assert_eq!((list.session, list.value.sessions.len()), (turn, 3));
    server.read_mode.store(EMPTY, Ordering::SeqCst);
    assert!(engine.account_sessions().expect("answers").value.sessions.is_empty());
    server.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    assert_eq!(engine.account_sessions().err(), Some(AccountCallError::ServerError));
    assert_eq!(engine.account_display_name().err(), Some(AccountCallError::ServerError));
    server.read_mode.store(SESSION_ENDED, Ordering::SeqCst);
    assert_eq!(engine.account_sessions().err(), Some(AccountCallError::SessionEnded));
    server.read_mode.store(ANSWER, Ordering::SeqCst);

    server.change_mode.store(SERVER_ERROR, Ordering::SeqCst);
    assert_eq!(
        engine.revoke_account_session(turn, "s-web").err(),
        Some(AccountCallError::ServerError)
    );
    assert_eq!(
        engine.revoke_other_account_sessions(turn).err(),
        Some(AccountCallError::ServerError)
    );
    server.change_mode.store(ANSWER, Ordering::SeqCst);
    assert_eq!(engine.revoke_account_session(turn, "s-web"), Ok(()));
    assert_eq!(server.deleted.lock().unwrap().clone(), ["s-web", "s-web"]);

    // Another session (a lock and an unlock), then another account: a change stamped with the old session sends
    // nothing, and a read answers for the account that is signed in.
    engine.lock();
    assert_eq!(engine.account_sessions().err(), Some(AccountCallError::NotSignedIn));
    assert_eq!(
        engine.revoke_account_session(turn, "s-phone").err(),
        Some(AccountCallError::NotSignedIn)
    );
    engine
        .unlock_with_phrase(Zeroizing::new(server.people[0].phrase.clone()))
        .expect("the phrase unlocks");
    assert_eq!(
        engine.revoke_account_session(turn, "s-phone").err(),
        Some(AccountCallError::NotSignedIn)
    );
    assert_eq!(
        engine.revoke_other_account_sessions(turn).err(),
        Some(AccountCallError::NotSignedIn)
    );
    engine.sign_out_keeping_local_data().expect("signs out");
    sign_in(&engine, &server.people[1]);
    assert_eq!(
        engine.revoke_account_session(turn, "t-this").err(),
        Some(AccountCallError::NotSignedIn)
    );
    assert_eq!(server.deleted.lock().unwrap().len(), 2, "nothing more was sent");
    assert_eq!(server.revoked_all.load(Ordering::SeqCst), 0);
    assert_eq!(engine.account_profile().expect("answers").value.email, SECOND);
    assert_eq!(engine.account_display_name().expect("answers").value, None);
    assert_eq!(engine.revoke_other_account_sessions(engine.session_turn()), Ok(0));
}

/// The session is locked while Beebeeb is answering: the engine hands the answer to nobody, and the source
/// (asked the same way) holds nothing of it.
#[test]
fn an_answer_that_lands_after_a_lock_is_handed_to_nobody() {
    let server = server();
    let engine = signed_in(&server);
    let lock = |engine: &TestEngine| {
        let engine = engine.clone();
        Box::new(move || {
            engine.lock();
        }) as Box<dyn FnOnce() + Send>
    };
    *server.during_read.lock().unwrap() = Some(lock(&engine));
    assert_eq!(engine.account_sessions().err(), Some(AccountCallError::NotSignedIn));
    assert_eq!(
        server.asked("GET /api/v1/account/sessions"),
        1,
        "it was asked, and answered"
    );

    engine
        .unlock_with_phrase(Zeroizing::new(server.people[0].phrase.clone()))
        .expect("the phrase unlocks");
    let mut source = Source::new(engine.clone());
    *server.during_read.lock().unwrap() = Some(lock(&engine));
    opened(&mut source, Pane::Sessions);
    assert!(source.sessions().is_empty());
    assert_eq!(source.pane_state(Pane::Sessions), PaneState::SignedOut);
}

#[test]
fn sizes_initials_and_times_read_as_the_board_writes_them() {
    assert_eq!(size_words(1_000_000_000_000), "1 TB");
    assert_eq!(size_words(240_000_000_000), "240 GB");
    assert_eq!(size_words(1_500_000_000), "1.5 GB");
    assert_eq!(size_words(999), "999 B");
    assert_eq!(size_words(-5), "0 B");
    assert_eq!(initials_of("Invented Middle Person"), "IP");
    assert_eq!(initials_of("solo"), "S");
    assert_eq!(initials_of(""), "");
    let now = 1_000_000;
    assert_eq!(ago(Some(now - 59), now), "now");
    assert_eq!(ago(Some(now + 30), now), "now", "a clock ahead is not the future");
    assert_eq!(ago(Some(now - 3_600), now), "1 hour ago");
    assert_eq!(ago(Some(now - 7_200), now), "2 hours ago");
    assert_eq!(ago(Some(now - 86_400), now), "1 day ago");
    assert_eq!(ago(Some(now - 3 * 86_400), now), "3 days ago");
}

// ---- The running app's window.

fn window_on(engine: TestEngine, section: SettingsPane) -> Runtime<State> {
    let state = crate::shell::state::init_on_engine(crate::shell::host::Shell::fixture(), engine);
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, crate::view, (958.0, 638.0), env);
    rt.on_key = Some(host::on_key);
    rt.poll = Some(|s: &mut State, _: f64, tx: &mut Tx| crate::frames::account::poll(s, tx));
    rt.frame(0.0);
    rt.update(move |s, _| {
        host::dispatch(s, ShellEvent::OpenSettings(section));
    });
    rt.frame(0.016);
    rt
}

fn shows(rt: &Runtime<State>, words: &str) -> bool {
    rt.elements().iter().any(|e| {
        !e.exiting && (e.text.as_deref() == Some(words) || e.label.as_deref().is_some_and(|l| l.contains(words)))
    })
}

fn says_anywhere(rt: &Runtime<State>, words: &str) -> bool {
    rt.elements().iter().any(|e| {
        e.text.as_deref().is_some_and(|t| t.contains(words)) || e.label.as_deref().is_some_and(|l| l.contains(words))
    })
}

fn has(rt: &Runtime<State>, key: &str) -> bool {
    rt.elements()
        .iter()
        .any(|e| !e.exiting && e.key.as_deref() == Some(key))
}

fn wait_for(rt: &mut Runtime<State>, what: &str, done: impl Fn(&Runtime<State>) -> bool) {
    let began = Instant::now();
    while !done(rt) {
        assert!(began.elapsed() < Duration::from_secs(10), "never: {what}");
        std::thread::sleep(Duration::from_millis(20));
        rt.frame(0.1);
    }
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let rect = rt
        .elements()
        .into_iter()
        .find(|e| !e.exiting && e.key.as_deref() == Some(key))
        .unwrap_or_else(|| panic!("no element keyed {key}"))
        .rect;
    let (x, y) = (rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.016);
    rt.frame(0.4);
}

#[test]
fn the_running_apps_account_draws_loading_then_the_account_and_never_a_sample() {
    let server = server();
    server.read_delay_ms.store(0, Ordering::SeqCst);
    let mut rt = window_on(signed_in(&server), SettingsPane::Account);
    wait_for(&mut rt, "the account", |rt| shows(rt, FIRST));
    assert!(shows(&rt, FIRST_NAME) && shows(&rt, "240 GB"));
    assert!(!has(&rt, "not-built"));
    for sample in ["Anna", "northwind", "Pass", "VPN", "Chat", "Mail"] {
        assert!(!says_anywhere(&rt, sample), "a sample value shows: {sample}");
    }
    // A pane nothing answers for: the shell's frame, with nothing of the account's panes in it.
    for section in [SettingsPane::Security] {
        rt.update(move |s, _| {
            host::dispatch(s, ShellEvent::OpenSettings(section));
        });
        rt.frame(0.016);
        rt.frame(0.4);
        assert!(has(&rt, "not-built"), "{section:?}");
        assert!(!says_anywhere(&rt, "Two-factor") && !says_anywhere(&rt, "anyone using this PC"));
    }
    rt.update(|s, _| crate::frames::account::go(s, "DW-A64"));
    rt.frame(0.016);
    assert!(has(&rt, "not-built"), "Name is not bound");
}

#[test]
fn the_window_drops_the_first_account_when_another_signs_in() {
    let server = server();
    let engine = signed_in(&server);
    let mut rt = window_on(engine.clone(), SettingsPane::DevicesAndSessions);
    wait_for(&mut rt, "the sessions", |rt| shows(rt, PHONE));
    engine.sign_out_keeping_local_data().expect("signs out");
    // Before any poll of the source: a draw alone shows nothing of the account.
    rt.poll = None;
    rt.update(|_, _| {});
    rt.frame(0.016);
    rt.frame(1.0);
    for words in [PHONE, BROWSER, THIS_PC, FIRST, FIRST_NAME] {
        assert!(!says_anywhere(&rt, words), "signed out, still shows {words}");
    }
    assert!(shows(&rt, "Not signed in"));
    sign_in(&engine, &server.people[1]);
    rt.poll = Some(|s: &mut State, _: f64, tx: &mut Tx| crate::frames::account::poll(s, tx));
    wait_for(&mut rt, "the second account's list", |rt| {
        shows(rt, "Another invented PC")
    });
    for words in [PHONE, BROWSER, THIS_PC, FIRST, FIRST_NAME] {
        assert!(!says_anywhere(&rt, words), "the second account sees {words}");
    }
}

#[test]
fn a_sign_out_in_the_window_is_confirmed_sent_once_and_said() {
    let server = server();
    let mut rt = window_on(signed_in(&server), SettingsPane::DevicesAndSessions);
    wait_for(&mut rt, "the sessions", |rt| shows(rt, PHONE));
    // The first row's button: nothing is sent before the confirm.
    click(&mut rt, "btn-0");
    assert!(shows(&rt, &format!("Sign out {PHONE}?")), "the confirm (DW-A56)");
    assert!(server.deleted.lock().unwrap().is_empty());
    // The confirm's own button, numbered after the rows' two.
    click(&mut rt, "btn-2");
    wait_for(&mut rt, "the toast", |rt| shows(rt, SIGNED_OUT));
    assert_eq!(server.deleted.lock().unwrap().clone(), ["s-phone"]);
    wait_for(&mut rt, "the row goes", |rt| !says_anywhere(rt, PHONE));
    assert!(shows(&rt, BROWSER));

    // A refusal is drawn, and the row stands.
    server.change_mode.store(SERVER_ERROR, Ordering::SeqCst);
    click(&mut rt, "btn-0");
    click(&mut rt, "btn-1");
    wait_for(&mut rt, "the refusal", |rt| shows(rt, NOT_SIGNED_OUT));
    assert!(shows(&rt, BROWSER));
    // The toast goes when the pane is left.
    rt.update(|s, _| crate::frames::account::go(s, "DW-A51"));
    wait_for(&mut rt, "the account", |rt| shows(rt, FIRST));
    assert!(!says_anywhere(&rt, NOT_SIGNED_OUT));
}

#[test]
fn a_failed_read_is_drawn_with_a_way_to_try_again() {
    let server = server();
    server.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    let mut rt = window_on(signed_in(&server), SettingsPane::DevicesAndSessions);
    wait_for(&mut rt, "the failure", |rt| shows(rt, "Couldn't load this"));
    assert!(!says_anywhere(&rt, PHONE));
    server.read_mode.store(ANSWER, Ordering::SeqCst);
    click(&mut rt, "btn-0");
    wait_for(&mut rt, "the sessions", |rt| shows(rt, PHONE));
    assert!(!says_anywhere(&rt, "Couldn't load this"));
}
