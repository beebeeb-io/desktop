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
use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::*;

#[path = "account_tests_sign_out.rs"]
mod sign_out;
#[path = "account_tests_window.rs"]
mod window;

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
/// A sign-out only: Beebeeb takes it, and a gateway in front answers 502.
const TAKEN_BEHIND_A_502: u8 = 6;

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
    /// How the display name alone answers (when `read_mode` answers).
    name_mode: Arc<AtomicU8>,
    plan_mode: Arc<AtomicU8>,
    /// The plan's `status`, as Beebeeb names it.
    plan_status: Arc<Mutex<&'static str>>,
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
    let (name_mode, plan_status) = (Arc::new(AtomicU8::new(ANSWER)), Arc::new(Mutex::new("active")));
    let (name_in, status_in) = (name_mode.clone(), plan_status.clone());
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
            // Who is signed in: the accounts' own handler answers it, unless a read is asked to fail.
            "GET /api/v1/auth/me" if authed && failed(read).is_some() => failed(read).expect("checked"),
            "GET /api/v1/me/profile" => failed(read)
                .or_else(|| failed(name_in.load(Ordering::SeqCst)))
                .unwrap_or_else(|| {
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
                    "plan": "personal", "billing_cycle": "yearly", "status": *status_in.lock().unwrap(), "seats": 1,
                    "region": "eu",
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
                    let mut sessions = person.sessions.lock().unwrap();
                    let before = sessions.len();
                    sessions.retain(|s| s["id"] != json!(id));
                    if sessions.len() == before {
                        // As Beebeeb does: no such session of this account.
                        return Reply::json(404, json!({ "error": "a server body" }));
                    }
                    if change == TAKEN_BEHIND_A_502 {
                        return Reply::json(502, json!({ "error": "a gateway's body" }));
                    }
                    Reply::ok(json!({ "revoked": true }))
                })
            }
            _ => rest[who](req),
        }
    }));
    Server {
        fake,
        people,
        read_mode,
        name_mode,
        plan_mode,
        plan_status,
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
    assert_eq!(
        who.initials, "AS",
        "from the part before the @: never a letter of the domain"
    );
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
                s.country.as_str(),
                s.this_device,
            )
        })
        .collect();
    assert_eq!(
        brief,
        [
            ("s-this", THIS_PC, DeviceKind::Laptop, "Desktop app", "NL", true),
            // `ios` is a phone's app or its browser: the words name the system only.
            ("s-phone", PHONE, DeviceKind::Phone, "iPhone or iPad", "DE", false),
            // Not two letters: never shown (a city can not reach the pane).
            ("s-web", BROWSER, DeviceKind::Browser, "Web browser", "", false),
        ]
    );
    // How long ago, on this computer's clock (the words themselves: the test of `ago`); no time is said so.
    assert!(rows[1].last_active.ends_with(" min ago"), "{}", rows[1].last_active);
    assert_eq!(rows[2].last_active, "not recorded");
    assert_eq!(
        kind_of("an-unknown-kind"),
        (DeviceKind::Browser, "Other client"),
        "never a guess"
    );
    assert_eq!(kind_of("api").1, "Other client");
    assert_eq!(kind_of("app"), (DeviceKind::Laptop, "Beebeeb app"));
    assert_eq!(kind_of("android"), (DeviceKind::Phone, "Android"));
    assert_eq!(kind_of("cli"), (DeviceKind::CommandLine, "Command line"));
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

/// The engine held the session as ended (its own calls were answered 401 again and again): nothing is asked, the
/// pane says so, and once a later call of the engine works the pane reads again by itself.
#[test]
fn a_session_the_engine_held_as_ended_is_asked_again_once_a_later_call_works() {
    let server = server();
    let engine = signed_in(&server);
    let mut source = Source::new(engine.clone());
    for _ in 0..3 {
        engine.note_call_for_test(true);
    }
    assert!(
        engine.account_session_refused(),
        "precondition: the engine's 401 streak"
    );
    let asked = server.fake.routes().len();
    source.opened(Pane::Account);
    let pumped = source.pump();
    assert_eq!(source.pane_state(Pane::Account), PaneState::SessionEnded);
    assert_eq!(source.pane_state(Pane::Sessions), PaneState::SessionEnded);
    assert!(!pumped.busy, "nothing is out");
    assert!(
        pumped.watch,
        "the window looks again: nothing else wakes it when the engine's flag clears"
    );
    source.reload(Pane::Account);
    assert_eq!(
        server.fake.routes().len(),
        asked,
        "nothing is asked while the engine refuses the session"
    );

    // The engine's next call works.
    engine.note_call_for_test(false);
    assert!(source.pump().changed);
    assert_eq!(
        source.pane_state(Pane::Account),
        PaneState::Loading,
        "asked again, without a reopening"
    );
    settle(&mut source);
    assert_eq!(source.pane_state(Pane::Account), PaneState::Ready);
    assert_eq!(source.identity().email, FIRST);
    assert!(!source.pump().watch);

    // No pane on screen: nothing is watched, and nothing is read when the flag clears.
    for _ in 0..3 {
        engine.note_call_for_test(true);
    }
    source.reload(Pane::Account);
    source.closed();
    assert!(!source.pump().watch);
    let asked = server.fake.routes().len();
    engine.note_call_for_test(false);
    source.pump();
    settle(&mut source);
    assert_eq!(server.fake.routes().len(), asked);

    // A 401 one of the account's own calls got is not the engine's finding: it stays for the session.
    source.opened(Pane::Sessions);
    settle(&mut source);
    assert_eq!(source.pane_state(Pane::Sessions), PaneState::Ready);
    server.read_mode.store(SESSION_ENDED, Ordering::SeqCst);
    source.reload(Pane::Sessions);
    settle(&mut source);
    assert_eq!(source.pane_state(Pane::Sessions), PaneState::SessionEnded);
    server.read_mode.store(ANSWER, Ordering::SeqCst);
    let asked = server.fake.routes().len();
    engine.note_call_for_test(false);
    let pumped = source.pump();
    assert!(!pumped.watch && !pumped.busy);
    assert_eq!(source.pane_state(Pane::Sessions), PaneState::SessionEnded);
    assert_eq!(server.fake.routes().len(), asked);
}

/// A pane that is not on screen is not read again for a new session.
#[test]
fn nothing_is_read_again_for_a_pane_that_is_no_longer_on_screen() {
    let server = server();
    let engine = signed_in(&server);
    let mut source = Source::new(engine.clone());
    opened(&mut source, Pane::Account);
    let again = |engine: &TestEngine| {
        engine.lock();
        engine
            .unlock_with_phrase(Zeroizing::new(server.people[0].phrase.clone()))
            .expect("the phrase unlocks");
    };
    // On screen: a new session is read once, without a reopening.
    again(&engine);
    source.pump();
    settle(&mut source);
    assert_eq!(server.asked("GET /api/v1/me/profile"), 2);
    assert_eq!(source.pane_state(Pane::Account), PaneState::Ready);

    source.closed();
    again(&engine);
    for _ in 0..3 {
        source.pump();
    }
    settle(&mut source);
    assert_eq!(
        server.asked("GET /api/v1/me/profile"),
        2,
        "no pane shows: nothing is asked"
    );
    assert_eq!(
        source.identity().email,
        "",
        "and the old session's answer is not handed out"
    );
    opened(&mut source, Pane::Account);
    assert_eq!(server.asked("GET /api/v1/me/profile"), 3);
    assert_eq!(source.identity().email, FIRST);
}

/// The name could not be read: the email stands in, as for an account with no name, and the rest draws.
#[test]
fn a_name_that_cannot_be_read_is_the_email_and_not_a_failed_pane() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    for mode in [SERVER_ERROR, UNREADABLE] {
        server.name_mode.store(mode, Ordering::SeqCst);
        source.reload(Pane::Account);
        settle(&mut source);
        assert_eq!(source.pane_state(Pane::Account), PaneState::Ready, "mode {mode}");
        let who = source.identity();
        assert_eq!((who.name.as_str(), who.email.as_str()), (FIRST, FIRST), "mode {mode}");
        assert_eq!(who.initials, "AF", "mode {mode}");
        assert!(source.plan().is_some(), "mode {mode}: the plan still draws");
    }
    // A 401 on the name is a 401.
    server.name_mode.store(SESSION_ENDED, Ordering::SeqCst);
    source.reload(Pane::Account);
    settle(&mut source);
    assert_eq!(source.pane_state(Pane::Account), PaneState::SessionEnded);
}

fn plan_with(status: &str, ends: Option<&str>) -> Subscription {
    serde_json::from_value(json!({
        "plan": "personal", "billing_cycle": "yearly", "status": status, "seats": 1, "region": "eu",
        "current_period_end": ends, "quota_bytes": 1_000_000_000_000_i64, "used_bytes": 239_960_000_000_i64,
    }))
    .expect("the plan reads")
}

/// How the plan's period ends is said from the plan's status: "renews" only for a plan that renews.
#[test]
fn the_plan_says_how_its_period_ends_and_never_renews_for_a_plan_that_does_not() {
    const DAY: Option<&str> = Some("2027-03-14T12:00:00Z");
    let said = |status: &str, ends: Option<&str>| {
        let (card, details) = plan_of(&plan_with(status, ends)).expect("a plan with a size");
        (card.rest, details.map(|d| (d.day_label, d.renews)))
    };
    let on = |label: &'static str| Some((label, "14 Mar 2027".to_string()));
    assert_eq!(
        said("active", DAY),
        ("used \u{b7} renews 14 Mar 2027".into(), on("Renews"))
    );
    assert_eq!(
        said("cancelling", DAY),
        ("used \u{b7} cancels on 14 Mar 2027".into(), on("Cancels on"))
    );
    assert_eq!(
        said("trialing", DAY),
        ("used \u{b7} trial ends 14 Mar 2027".into(), on("Trial ends"))
    );
    assert_eq!(said("paused", DAY), ("used \u{b7} paused".into(), None));
    assert_eq!(said("paused", None), ("used \u{b7} paused".into(), None));
    // A status with no words here, or one this app does not know: less is said, never more.
    for status in ["past_due", "cancelled", "a-status-from-next-year", "", "Active"] {
        assert_eq!(said(status, DAY), ("used".into(), None), "{status:?}");
    }
    for status in ["active", "cancelling", "trialing"] {
        assert_eq!(said(status, None), ("used".into(), None), "{status:?} without a day");
    }
    // The size is the storage figure's own formatter: it truncates, so the same bytes read the same everywhere.
    let (card, _) = plan_of(&plan_with("active", DAY)).expect("a plan with a size");
    assert_eq!(card.used, "239 GB");
    assert_eq!(card.used, crate::frames::global::source::size_words(239_960_000_000));
}

/// Nothing personal reaches a log through `{:?}`.
#[test]
fn the_types_that_hold_personal_values_do_not_print_them() {
    let who = Identity {
        name: FIRST_NAME.into(),
        initials: "IP".into(),
        email: FIRST.into(),
    };
    let printed = format!("{who:?}");
    assert!(
        !printed.contains(FIRST) && !printed.contains(FIRST_NAME) && !printed.contains("IP"),
        "{printed}"
    );
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    let printed = format!("{:?}", source.sessions());
    for words in [PHONE, BROWSER, THIS_PC, "s-phone", "s-web", "s-this"] {
        assert!(!printed.contains(words), "{words}: {printed}");
    }
    let printed = format!(
        "{:?}",
        [
            crate::frames::account::Dialog::SignOutDevice("s-phone".into()),
            crate::frames::account::Dialog::PasskeyMenu("s-phone".into()),
            crate::frames::account::Dialog::RemovePasskey("s-phone".into()),
        ]
    );
    assert!(!printed.contains("s-phone"), "{printed}");
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

    // The one read Account makes: the three answers, for this session, each part with its own failure.
    let asked = server.fake.routes().len();
    let basics = engine.account_basics().expect("answers");
    assert_eq!(server.fake.routes().len(), asked + 3, "three requests");
    assert_eq!((basics.session, basics.value.profile.email.as_str()), (turn, FIRST));
    assert_eq!(basics.value.display_name, Ok(Some(FIRST_NAME.to_string())));
    assert_eq!(
        basics.value.subscription.as_ref().map(|plan| plan.quota_bytes),
        Ok(1_000_000_000_000)
    );
    assert!(
        !format!("{basics:?}").contains(FIRST),
        "an answer's Debug names no email"
    );
    server.name_mode.store(SERVER_ERROR, Ordering::SeqCst);
    server.plan_mode.store(UNREADABLE, Ordering::SeqCst);
    let basics = engine.account_basics().expect("the profile answered").value;
    assert_eq!(basics.profile.email, FIRST);
    assert_eq!(basics.display_name, Err(AccountCallError::ServerError));
    assert_eq!(basics.subscription.err(), Some(AccountCallError::Unreadable));
    server.name_mode.store(SESSION_ENDED, Ordering::SeqCst);
    server.plan_mode.store(ANSWER, Ordering::SeqCst);
    let basics = engine.account_basics().expect("the profile answered").value;
    assert_eq!(basics.display_name, Err(AccountCallError::SessionEnded));
    server.name_mode.store(ANSWER, Ordering::SeqCst);
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
    assert_eq!(
        engine.end_account_session(turn, "s-web").err(),
        Some(AccountCallError::ServerError),
        "a 500 is not a 404"
    );
    server.change_mode.store(ANSWER, Ordering::SeqCst);
    assert_eq!(engine.revoke_account_session(turn, "s-web"), Ok(()));
    assert_eq!(server.deleted.lock().unwrap().clone(), ["s-web", "s-web", "s-web"]);
    // Ended before: Beebeeb answers 404, told apart from every other refusal. The plain call keeps it an error.
    assert_eq!(engine.end_account_session(turn, "s-web"), Ok(SessionEnd::AlreadyGone));
    assert_eq!(
        engine.revoke_account_session(turn, "s-web").err(),
        Some(AccountCallError::ServerError)
    );
    assert_eq!(engine.end_account_session(turn, "s-phone"), Ok(SessionEnd::Ended));
    assert_eq!(server.deleted.lock().unwrap().len(), 6);

    // The engine holds the session as ended: nothing is asked and nothing is sent.
    for _ in 0..3 {
        engine.note_call_for_test(true);
    }
    assert!(engine.account_session_refused());
    let asked = server.fake.routes().len();
    assert_eq!(engine.account_basics().err(), Some(AccountCallError::SessionEnded));
    assert_eq!(
        engine.end_account_session(turn, "s-this").err(),
        Some(AccountCallError::SessionEnded)
    );
    assert_eq!(server.fake.routes().len(), asked);
    engine.note_call_for_test(false);
    assert!(!engine.account_session_refused());

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
    assert_eq!(
        engine.end_account_session(turn, "t-this").err(),
        Some(AccountCallError::NotSignedIn)
    );
    assert_eq!(server.deleted.lock().unwrap().len(), 6, "nothing more was sent");
    assert_eq!(server.revoked_all.load(Ordering::SeqCst), 0);
    assert_eq!(engine.account_profile().expect("answers").value.email, SECOND);
    assert_eq!(engine.account_display_name().expect("answers").value, None);
    let basics = engine.account_basics().expect("answers");
    assert_eq!(basics.session, engine.session_turn());
    assert_eq!(
        (basics.value.profile.email.as_str(), &basics.value.display_name),
        (SECOND, &Ok(None))
    );
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
fn initials_and_times_read_as_the_board_writes_them() {
    assert_eq!(initials_of("Invented Middle Person"), "IP");
    // An email: the part before the @ only.
    assert_eq!(initials_of("invented@example.io"), "I");
    assert_eq!(initials_of("invented.person@example.io"), "IP");
    assert_eq!(initials_of("solo"), "S");
    assert_eq!(initials_of(""), "");
    let now = 1_000_000;
    assert_eq!(ago(None, now), "not recorded");
    assert_eq!(ago(Some(now - 59), now), "now");
    assert_eq!(ago(Some(now - 12 * 60 - 30), now), "12 min ago");
    assert_eq!(ago(Some(now + 30), now), "now", "a clock ahead is not the future");
    assert_eq!(ago(Some(now - 3_600), now), "1 hour ago");
    assert_eq!(ago(Some(now - 7_200), now), "2 hours ago");
    assert_eq!(ago(Some(now - 86_400), now), "1 day ago");
    assert_eq!(ago(Some(now - 3 * 86_400), now), "3 days ago");
}
