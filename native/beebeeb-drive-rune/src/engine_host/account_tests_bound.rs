//! Sign out of Drive, the step-up, the name and sign-in activity on the real engine, against a fake of Beebeeb's
//! account routes on loopback (the routes the shared fake does not have are answered here). Every name, password
//! and device is invented.

use std::sync::atomic::AtomicBool;

use super::activity::{NOT_STARTED, STARTED};
use super::*;
use crate::frames::account::source::{EventKind, NameWhy};

pub(super) const NEW_NAME: &str = "Another Invented Person";
const BROWSER_NOW: &str = "An invented browser";

/// One account on loopback with the routes of this file.
pub(super) struct Bound {
    fake: FakeApi,
    account: Arc<FakeAccount>,
    phrase: String,
    /// The display name Beebeeb holds.
    pub(super) name: Arc<Mutex<Option<String>>>,
    /// Whether the account records its sign-ins.
    tracking: Arc<AtomicBool>,
    /// How a change (the name, the recording) answers: `ANSWER`, `SERVER_ERROR`, `SESSION_ENDED`, `DROP_CONNECTION`.
    mode: Arc<AtomicU8>,
    /// How a step-up answers (same codes).
    step_mode: Arc<AtomicU8>,
    pub(super) delay_ms: Arc<AtomicU64>,
}

fn event(kind: &str, outcome: &str, device: Value, country: Value, minutes_ago: i64) -> Value {
    json!({
        "id": format!("e-{kind}-{minutes_ago}"), "type": kind, "description": "Beebeeb's own words", "category": "auth",
        "outcome": outcome, "device": device, "country_code": country,
        "created_at": (Utc::now() - chrono::Duration::minutes(minutes_ago)).to_rfc3339(),
    })
}

pub(super) fn bound() -> Bound {
    let (account, phrase) = FakeAccount::register(FIRST, PASSWORD, TEST_PHRASE);
    let account = Arc::new(account);
    let rest = account_handler(account.clone());
    let name = Arc::new(Mutex::new(Some(FIRST_NAME.to_string())));
    let tracking = Arc::new(AtomicBool::new(true));
    let (mode, step_mode, delay_ms) = (
        Arc::new(AtomicU8::new(ANSWER)),
        Arc::new(AtomicU8::new(ANSWER)),
        Arc::new(AtomicU64::new(0)),
    );
    let (account_in, name_in, tracking_in, mode_in, step_in, delay_in) = (
        account.clone(),
        name.clone(),
        tracking.clone(),
        mode.clone(),
        step_mode.clone(),
        delay_ms.clone(),
    );
    let fake = FakeApi::start(Arc::new(move |req: &Request| {
        let route = req.route();
        let authed = req.bearer() == Some(account_in.session_token.as_str());
        match route.as_str() {
            "GET /api/v1/me/profile" if authed => {
                Reply::ok(json!({ "username": null, "display_name": *name_in.lock().unwrap() }))
            }
            "PUT /api/v1/me/profile" => {
                std::thread::sleep(Duration::from_millis(delay_in.load(Ordering::SeqCst)));
                if let Some(reply) = failed(mode_in.load(Ordering::SeqCst)) {
                    return reply;
                }
                if !authed {
                    return Reply::json(401, json!({}));
                }
                *name_in.lock().unwrap() = req.json()["display_name"].as_str().map(str::to_string);
                Reply::ok(json!({ "display_name": *name_in.lock().unwrap() }))
            }
            "GET /api/v1/me/activity" if authed => {
                if !tracking_in.load(Ordering::SeqCst) {
                    return Reply::ok(json!({ "opted_in": false, "events": [] }));
                }
                Reply::ok(json!({ "opted_in": true, "events": [
                    event("auth.login", "success", json!(BROWSER_NOW), json!("nl"), 5),
                    event("file.upload", "success", Value::Null, Value::Null, 6),
                    event("auth.login.failed", "failure", Value::Null, json!("an-invented-city"), 2 * 24 * 60),
                    event("account.password_change", "success", Value::Null, json!("DE"), 400 * 24 * 60),
                    event("share.create", "success", Value::Null, Value::Null, 500 * 24 * 60),
                ]}))
            }
            "PUT /api/v1/me/tracking" => {
                std::thread::sleep(Duration::from_millis(delay_in.load(Ordering::SeqCst)));
                if let Some(reply) = failed(mode_in.load(Ordering::SeqCst)) {
                    return reply;
                }
                if !authed {
                    return Reply::json(401, json!({}));
                }
                tracking_in.store(req.json()["opted_in"] == json!(true), Ordering::SeqCst);
                Reply::ok(json!({ "tracking_opted_in": true }))
            }
            "POST /api/v1/auth/confirm-opaque-start" | "POST /api/v1/auth/confirm-opaque-finish" => {
                if let Some(reply) = failed(step_in.load(Ordering::SeqCst)) {
                    return reply;
                }
                if !authed {
                    return Reply::error(401, "unauthorized");
                }
                if route.ends_with("start") {
                    account_in.opaque.login_start(&req.json())
                } else if account_in.opaque.login_finish(&req.json()) {
                    Reply::ok(
                        json!({ "confirmation_token": "an-invented-grant", "expires_at": "2099-01-01T00:05:00Z" }),
                    )
                } else {
                    Reply::error(401, "invalid_credentials")
                }
            }
            _ => rest(req),
        }
    }));
    Bound {
        fake,
        account,
        phrase,
        name,
        tracking,
        mode,
        step_mode,
        delay_ms,
    }
}

impl Bound {
    pub(super) fn engine(&self) -> TestEngine {
        let (host, _messages) = ChannelHost::new(|| {});
        let config = EngineConfig {
            account_id: "account-test-slot".into(),
            state_dir: crate::engine_host::tests::process_state_dir(),
            api_base: self.fake.base.clone(),
        };
        let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens");
        engine.restore();
        Arc::new(engine)
    }

    pub(super) fn signed_in(&self) -> TestEngine {
        let engine = self.engine();
        engine
            .sign_in_password(&self.account.email, Zeroizing::new(PASSWORD.into()))
            .expect("the fake accepts the password");
        engine
            .unlock_with_phrase(Zeroizing::new(self.phrase.clone()))
            .expect("the phrase unlocks");
        engine
    }

    fn asked(&self, route: &str) -> usize {
        self.fake.routes().iter().filter(|r| r.as_str() == route).count()
    }
}

/// Take in the sign-out's answer: poll until it comes (at most 10 s).
fn sign_out_answer(source: &mut Source) -> SignOutAnswer {
    let began = Instant::now();
    loop {
        source.pump();
        if let Some(answer) = source.sign_out_result() {
            return answer;
        }
        assert!(began.elapsed() < Duration::from_secs(10), "no answer to the sign-out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn name_answer(source: &mut Source) -> Option<NameAnswer> {
    let began = Instant::now();
    loop {
        source.pump();
        if let Some(answer) = source.name_result() {
            return Some(answer);
        }
        if began.elapsed() > Duration::from_secs(3) {
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

// ---- Sign out of Drive ------------------------------------------------------------------------------------

#[test]
fn signing_out_of_drive_is_the_engines_and_is_done_only_when_it_answered() {
    let server = bound();
    let engine = server.signed_in();
    let mut source = Source::new(engine.clone());
    opened(&mut source, Pane::Security);
    let facts = source.security_facts();
    assert!(facts.can_sign_out, "Drive can be signed out");
    assert!(
        !facts.can_sign_out_all,
        "all apps has nothing to mean: Drive is the one app"
    );

    assert_eq!(source.sign_out_this_app(), SignOutAnswer::Pending, "sent, not done");
    // A second press while it is out sends nothing more.
    assert_eq!(source.sign_out_this_app(), SignOutAnswer::Pending);
    assert_eq!(sign_out_answer(&mut source), SignOutAnswer::Done);
    assert!(!engine.has_session(), "the engine's session is over");
    assert_eq!(source.sign_out_result(), None, "an answer is taken in once");
    // Nothing of the account stays in the source.
    source.pump();
    assert_eq!(source.identity().email, "");
    assert_eq!(source.pane_state(Pane::Security), PaneState::SignedOut);
}

#[test]
fn a_sign_out_pressed_under_a_session_that_moved_on_signs_nothing_out() {
    let server = bound();
    let engine = server.signed_in();
    let before = engine.session_turn();
    engine.sign_out_keeping_local_data().expect("signs out");
    engine
        .sign_in_password(&server.account.email, Zeroizing::new(PASSWORD.into()))
        .expect("signs in again");
    engine
        .unlock_with_phrase(Zeroizing::new(server.phrase.clone()))
        .expect("unlocks");
    assert_ne!(engine.session_turn(), before);
    assert_eq!(
        engine.sign_out_for(before),
        Err(beebeeb_engine::app::account_actions_api::ActionError::NotSignedIn)
    );
    assert!(engine.has_session(), "the newer session was not touched");
    assert_eq!(engine.sign_out_for(engine.session_turn()), Ok(()));
    assert!(!engine.has_session());
}

#[test]
fn a_sign_out_with_no_session_is_refused_in_words() {
    let server = bound();
    let engine = server.engine();
    let mut source = Source::new(engine);
    match source.sign_out_this_app() {
        SignOutAnswer::Refused(words) => assert!(words.contains("Nothing was signed out"), "{words}"),
        other => panic!("{other:?}"),
    }
}

// ---- The name -----------------------------------------------------------------------------------------------

#[test]
fn the_name_is_drawn_only_after_beebeeb_has_it() {
    let server = bound();
    server.delay_ms.store(300, Ordering::SeqCst);
    let mut source = Source::new(signed_in_on(&server));
    opened(&mut source, Pane::Name);
    assert_eq!(
        source.pane_state(Pane::Name),
        PaneState::Ready,
        "the Name page answers now"
    );
    assert_eq!(source.identity().name, FIRST_NAME);

    assert_eq!(source.save_name(NEW_NAME), NameAnswer::Pending);
    assert_eq!(source.save_name(NEW_NAME), NameAnswer::Pending, "one save at a time");
    assert_eq!(source.name_result(), None, "not answered yet");
    assert_eq!(
        source.identity().name,
        FIRST_NAME,
        "nothing of the new name before Beebeeb has it"
    );

    assert_eq!(name_answer(&mut source), Some(NameAnswer::Saved));
    assert_eq!(server.name.lock().unwrap().as_deref(), Some(NEW_NAME));
    assert_eq!(server.asked("PUT /api/v1/me/profile"), 1, "sent once");
    // The Account read is made again: the name shows with it.
    settle(&mut source);
    assert_eq!(source.identity().name, NEW_NAME);
}

#[test]
fn a_name_beebeeb_did_not_take_is_said_and_never_drawn() {
    let server = bound();
    let mut source = Source::new(signed_in_on(&server));
    opened(&mut source, Pane::Name);
    for (mode, why) in [
        (SERVER_ERROR, NameWhy::Refused),
        (SESSION_ENDED, NameWhy::Refused),
        (DROP_CONNECTION, NameWhy::Offline),
    ] {
        server.mode.store(mode, Ordering::SeqCst);
        assert_eq!(source.save_name(NEW_NAME), NameAnswer::Pending, "{mode}");
        assert_eq!(name_answer(&mut source), Some(NameAnswer::NotSaved(why)), "{mode}");
        assert_eq!(server.name.lock().unwrap().as_deref(), Some(FIRST_NAME), "{mode}");
        assert_eq!(source.identity().name, FIRST_NAME, "{mode}");
    }
}

#[test]
fn a_name_answered_after_the_account_left_is_dropped() {
    let server = bound();
    server.delay_ms.store(300, Ordering::SeqCst);
    let engine = server.signed_in();
    let mut source = Source::new(engine.clone());
    opened(&mut source, Pane::Name);
    assert_eq!(source.save_name(NEW_NAME), NameAnswer::Pending);
    engine.sign_out_keeping_local_data().expect("signs out");
    assert_eq!(
        name_answer(&mut source),
        None,
        "the late answer is for an account that is gone"
    );
    assert_eq!(source.identity().name, "");
}

fn signed_in_on(server: &Bound) -> TestEngine {
    server.signed_in()
}

// ---- The step-up --------------------------------------------------------------------------------------------

#[test]
fn a_step_up_proves_the_password_and_its_confirmation_is_spent_once_for_its_purpose() {
    let server = bound();
    let engine = server.signed_in();
    let mut source = Source::new(engine.clone());

    let confirmation = source
        .step_up(Password::from_field(PASSWORD), Purpose::ChangeEmail)
        .expect("the right password is confirmed");
    assert_eq!(confirmation.purpose(), Purpose::ChangeEmail);
    assert_eq!(source.actions.held(), 1);
    assert_eq!(server.asked("POST /api/v1/auth/confirm-opaque-start"), 1);
    assert_eq!(server.asked("POST /api/v1/auth/confirm-opaque-finish"), 1);
    // For its purpose only, and once.
    let other = source
        .step_up(Password::from_field(PASSWORD), Purpose::DeleteAccount)
        .expect("confirmed");
    assert!(
        source.actions.spend(&other, Purpose::ChangeEmail).is_none(),
        "another purpose"
    );
    assert!(source.actions.spend(&confirmation, Purpose::ChangeEmail).is_some());
    assert!(
        source.actions.spend(&confirmation, Purpose::ChangeEmail).is_none(),
        "spent"
    );

    // A wrong password is a wrong password, and holds nothing.
    let wrong = source.step_up(
        Password::from_field("an invented wrong password"),
        Purpose::ChangePassword,
    );
    assert_eq!(wrong.err(), Some(Refused::WrongPassword));
    assert_eq!(source.actions.held(), 0);
}

#[test]
fn a_step_up_that_cannot_be_made_confirms_nothing() {
    let server = bound();
    let mut source = Source::new(server.signed_in());
    for mode in [SERVER_ERROR, DROP_CONNECTION, SESSION_ENDED] {
        server.step_mode.store(mode, Ordering::SeqCst);
        let answer = source.step_up(Password::from_field(PASSWORD), Purpose::TwoFactor);
        assert_eq!(
            answer.err(),
            Some(Refused::Offline),
            "{mode}: never read as a right or a wrong password"
        );
        assert_eq!(source.actions.held(), 0, "{mode}");
    }
}

#[test]
fn a_confirmation_does_not_outlive_its_session_or_its_time() {
    let server = bound();
    let engine = server.signed_in();
    let mut source = Source::new(engine.clone());
    let early = source
        .step_up(Password::from_field(PASSWORD), Purpose::AddPasskey)
        .expect("confirmed");
    source.actions.ahead = Duration::from_secs(5 * 60);
    assert!(source.actions.spend(&early, Purpose::AddPasskey).is_none(), "too old");
    source.actions.ahead = Duration::ZERO;
    let late = source
        .step_up(Password::from_field(PASSWORD), Purpose::AddPasskey)
        .expect("confirmed");
    engine.sign_out_keeping_local_data().expect("signs out");
    source.pump();
    assert_eq!(source.actions.held(), 0, "the session is over");
    assert!(source.actions.spend(&late, Purpose::AddPasskey).is_none());
}

// ---- Sign-in activity --------------------------------------------------------------------------------------

#[test]
fn the_log_shows_sign_ins_and_security_changes_and_nothing_else() {
    let server = bound();
    let mut source = Source::new(signed_in_on(&server));
    assert_eq!(source.pane_state(Pane::Activity), PaneState::Loading);
    opened(&mut source, Pane::Activity);
    assert_eq!(source.pane_state(Pane::Activity), PaneState::Ready);
    let periods = source.activity().expect("recording is on");
    let events: Vec<&Event> = periods.iter().flat_map(|(_, events)| events).collect();
    // The upload and the share are not on this page; the other three are, newest first.
    assert_eq!(events.len(), 3, "{periods:?}");
    assert_eq!(events[0].kind, EventKind::SignIn);
    assert_eq!(events[0].what, format!("Signed in on {BROWSER_NOW}"));
    assert_eq!(events[0].when_where, "5 min ago · NL");
    assert_eq!(events[1].kind, EventKind::WrongPassword);
    assert_eq!(events[1].when_where, "2 days ago", "a city-like value is no country");
    assert_eq!(events[2].kind, EventKind::PasswordChanged);
    assert_eq!(periods.last().expect("a period").0, "Earlier", "a year ago");
    // The Security page gets the same events, and the recording cannot be stopped from here.
    let facts = source.security_facts();
    assert_eq!(facts.activity.as_ref().and_then(|a| a.as_ref()).map(Vec::len), Some(3));
    assert!(!source.can_stop_activity());
}

#[test]
fn a_log_that_is_off_says_off_and_is_switched_on_only_when_beebeeb_says_so() {
    let server = bound();
    server.tracking.store(false, Ordering::SeqCst);
    let mut source = Source::new(signed_in_on(&server));
    opened(&mut source, Pane::Activity);
    assert_eq!(source.pane_state(Pane::Activity), PaneState::Ready);
    assert_eq!(source.activity(), None, "off");
    assert_eq!(
        source.security_facts().activity,
        Some(None),
        "off is said as off, not as nothing"
    );

    server.delay_ms.store(300, Ordering::SeqCst);
    source.start_activity();
    source.start_activity();
    assert_eq!(source.activity(), None, "not drawn as on before Beebeeb took it");
    settle(&mut source);
    assert_eq!(server.asked("PUT /api/v1/me/tracking"), 1, "sent once per press");
    assert!(server.tracking.load(Ordering::SeqCst));
    assert!(source.activity().is_some(), "on, read again");
    assert_eq!(source.notice(), Some(STARTED));
    assert!(!source.notice_failed());
}

#[test]
fn a_recording_beebeeb_did_not_switch_on_stays_off_and_says_so() {
    let server = bound();
    server.tracking.store(false, Ordering::SeqCst);
    let mut source = Source::new(signed_in_on(&server));
    opened(&mut source, Pane::Activity);
    server.mode.store(SERVER_ERROR, Ordering::SeqCst);
    source.start_activity();
    settle(&mut source);
    assert_eq!(source.activity(), None);
    assert_eq!(source.notice(), Some(NOT_STARTED));
    assert!(source.notice_failed(), "said as a failure");
}

#[test]
fn a_log_that_cannot_be_read_is_a_failed_pane_and_the_log_of_another_account_is_never_kept() {
    let server = bound();
    let engine = server.signed_in();
    let mut source = Source::new(engine.clone());
    opened(&mut source, Pane::Activity);
    assert!(source.activity().is_some());
    engine.sign_out_keeping_local_data().expect("signs out");
    source.pump();
    assert_eq!(source.activity(), None, "nothing of the account stays");
    assert_eq!(source.security_facts().activity, None);
    assert_eq!(source.pane_state(Pane::Activity), PaneState::SignedOut);
}

// ---- Pure: what the log keeps, and its periods -------------------------------------------------------------

fn wire(
    kind: &str,
    outcome: &str,
    device: Option<&str>,
    country: Option<&str>,
    at: &str,
) -> beebeeb_engine::api_client::MyActivityEvent {
    serde_json::from_value(
        json!({ "type": kind, "outcome": outcome, "device": device, "country_code": country,
        "created_at": at }),
    )
    .expect("an event")
}

#[test]
fn only_allow_listed_event_types_reach_the_page_and_a_log_that_is_off_is_none() {
    use super::activity::log_of;
    let now = DateTime::parse_from_rfc3339("2026-11-14T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let on = beebeeb_engine::api_client::MyActivity {
        opted_in: true,
        events: vec![
            wire("file.downloaded", "success", None, None, "2026-11-14T11:59:00Z"),
            wire("share.create", "success", None, None, "2026-11-14T11:59:00Z"),
            wire("auth.session.revoke", "success", None, None, "2026-11-14T11:59:00Z"),
            wire("something.new", "success", None, None, "2026-11-14T11:59:00Z"),
            wire(
                "auth.signin.passkey",
                "success",
                None,
                Some("fi"),
                "2026-11-14T11:30:00Z",
            ),
            wire("auth.login", "failure", None, None, "2026-11-14T11:00:00Z"),
            wire("user.signup", "success", None, None, "not a time"),
        ],
    };
    let log = log_of(&on, now).expect("on");
    let kinds: Vec<EventKind> = log.iter().map(|l| l.event.kind).collect();
    // A sign-in that failed is not "signed in"; a time that does not read is last and "not recorded".
    assert_eq!(kinds, [EventKind::SignIn, EventKind::AccountCreated]);
    assert_eq!(log[0].event.what, "Signed in with a passkey");
    assert_eq!(log[0].event.when_where, "30 min ago · FI");
    assert_eq!(log[1].event.when_where, "not recorded");
    let off = beebeeb_engine::api_client::MyActivity {
        opted_in: false,
        events: on.events.clone(),
    };
    assert!(
        log_of(&off, now).is_none(),
        "off holds no events, whatever came with it"
    );
}

#[test]
fn periods_are_today_this_years_month_and_earlier() {
    use super::activity::{Logged, periods_of};
    let now = DateTime::parse_from_rfc3339("2026-11-14T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let at = |text: &str| Some(DateTime::parse_from_rfc3339(text).unwrap().timestamp());
    let make = |at, what: &str| Logged {
        at,
        event: Event {
            kind: EventKind::SignIn,
            what: what.into(),
            when_where: String::new(),
        },
    };
    let periods = periods_of(
        &[
            make(at("2026-11-14T08:00:00Z"), "a"),
            make(at("2026-11-14T01:00:00Z"), "b"),
            make(at("2026-10-30T08:00:00Z"), "c"),
            make(at("2025-12-31T23:00:00Z"), "d"),
            make(None, "e"),
        ],
        now,
    );
    let titles: Vec<&str> = periods.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(titles, ["Today", "October", "Earlier"]);
    assert_eq!(periods[0].1.len(), 2);
    assert_eq!(periods[2].1.len(), 2, "d and e share Earlier");
}
