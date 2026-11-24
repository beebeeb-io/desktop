//! Deleting the account on the real engine, against the engine's fake account on loopback with Beebeeb's
//! deletion routes added here, answered as `routes/account_delete.rs` answers them: the step-up's token is consumed
//! BEFORE the code is read, the state is `none | requested | scheduled`, and the signed-in cancel takes a scheduled
//! deletion only. Every name, code and password is invented.

use std::sync::atomic::{AtomicU8, Ordering};

use beebeeb_engine::app::EngineConfig;
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::gate::GateState;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use beebeeb_engine::test_api::{ConfirmTokens, FakeApi, Reply, Request};
use serde_json::json;

use super::*;

const PASSWORD: &str = "an invented password for the deleting account";
const EMAIL: &str = "deleting-account@example.eu";
const CODE: &str = "514208";
const WRONG_CODE: &str = "000000";
const CANCEL_UNTIL: &str = "2099-12-14T18:42:00+00:00";
const SHRED_AFTER: &str = "2099-12-14T19:42:00+00:00";

const ANSWER: u8 = 0;
/// `delete-request` answers 409 `delete_subscription_active`.
const PLAN_STILL_CHARGED: u8 = 1;
/// `delete-cancel` spends the token and closes the connection: no answer, nothing cancelled.
const CANCEL_ANSWER_LOST: u8 = 2;
/// The state read answers 500.
const STATE_FAILS: u8 = 3;

/// The fake Beebeeb, and what it saw.
struct Fake {
    api: FakeApi,
    account: Arc<FakeAccount>,
    phrase: String,
    /// `none`, `requested` or `scheduled`.
    state: Arc<Mutex<&'static str>>,
    mode: Arc<AtomicU8>,
    /// The body and the confirm header of every step-up and deletion request, as text.
    seen: Arc<Mutex<Vec<String>>>,
    /// Runs once, while the next `delete-request` is answered (the call is out).
    during_request: Hook,
}

impl Fake {
    fn asked(&self, route: &str) -> usize {
        self.api.routes().iter().filter(|r| r.as_str() == route).count()
    }
}

fn fake() -> Fake {
    let (account, phrase) = FakeAccount::register(EMAIL, PASSWORD, TEST_PHRASE);
    let account = Arc::new(account);
    let rest = account_handler(account.clone());
    let tokens = Arc::new(ConfirmTokens::default());
    let state = Arc::new(Mutex::new("none"));
    let mode = Arc::new(AtomicU8::new(ANSWER));
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let during_request: Hook = Arc::default();
    let (account_in, state_in, mode_in, seen_in, during_in) = (
        account.clone(),
        state.clone(),
        mode.clone(),
        seen.clone(),
        during_request.clone(),
    );
    let api = FakeApi::start(Arc::new(move |req: &Request| {
        let route = req.route();
        let authed = req.bearer() == Some(account_in.session_token.as_str());
        if route.contains("confirm-opaque") || route.contains("/auth/account/") {
            seen_in.lock().unwrap().push(format!(
                "{} {}",
                String::from_utf8_lossy(&req.body),
                req.header("x-confirm-token").unwrap_or_default()
            ));
        }
        match route.as_str() {
            "POST /api/v1/auth/confirm-opaque-start" | "POST /api/v1/auth/confirm-opaque-finish" => {
                if !authed {
                    return Reply::error(401, "unauthorized");
                }
                if route.ends_with("start") {
                    account_in.opaque.login_start(&req.json())
                } else if account_in.opaque.login_finish(&req.json()) {
                    Reply::ok(json!({ "confirmation_token": tokens.mint(), "expires_at": "2099-01-01T00:05:00Z" }))
                } else {
                    Reply::error(401, "invalid_credentials")
                }
            }
            "POST /api/v1/auth/account/delete-request" => {
                if !authed {
                    return Reply::error(401, "unauthorized");
                }
                // As Beebeeb: the extractor consumes the token before the handler reads anything.
                if !tokens.spend(req) {
                    return Reply::error(403, "step_up_required");
                }
                if let Some(hook) = during_in.lock().unwrap().take() {
                    hook();
                }
                if mode_in.load(Ordering::SeqCst) == PLAN_STILL_CHARGED {
                    let code = "delete_subscription_active";
                    return Reply::json(
                        409,
                        json!({ "error": code, "code": code, "message": "Beebeeb's own words" }),
                    );
                }
                if req.json()["code"] != json!(CODE) {
                    // As Beebeeb: a wrong second factor is 400 `invalid_totp_code`.
                    return Reply::json(400, json!({ "error": "invalid_totp_code" }));
                }
                *state_in.lock().unwrap() = "requested";
                Reply::ok(
                    json!({ "email_link_sent": true, "link_expires_at": "2099-01-01T00:00:00Z", "scheduled": false }),
                )
            }
            "GET /api/v1/auth/account/deletion" if authed && mode_in.load(Ordering::SeqCst) == STATE_FAILS => {
                Reply::json(500, json!({ "error": "internal" }))
            }
            "GET /api/v1/auth/account/deletion" if authed => match *state_in.lock().unwrap() {
                "scheduled" => Reply::ok(json!({
                    "state": "scheduled", "shred_after": SHRED_AFTER, "cancel_until": CANCEL_UNTIL,
                })),
                "requested" => Reply::ok(json!({ "state": "requested", "link_expires_at": "2099-01-01T00:00:00Z" })),
                _ => Reply::ok(json!({ "state": "none" })),
            },
            "POST /api/v1/auth/account/delete-cancel" => {
                if req.json().get("token").is_some_and(|t| !t.is_null()) {
                    return Reply::error(404, "delete_link_invalid");
                }
                if !authed {
                    return Reply::error(401, "unauthorized");
                }
                if !tokens.spend(req) {
                    return Reply::error(403, "step_up_required");
                }
                if mode_in.load(Ordering::SeqCst) == CANCEL_ANSWER_LOST {
                    return Reply::hang_up();
                }
                let mut state = state_in.lock().unwrap();
                if *state != "scheduled" {
                    let code = "delete_not_scheduled";
                    return Reply::json(404, json!({ "error": code, "code": code }));
                }
                *state = "none";
                Reply::ok(json!({ "cancelled": true, "not_restored": [] }))
            }
            _ => rest(req),
        }
    }));
    Fake {
        api,
        account,
        phrase,
        state,
        mode,
        seen,
        during_request,
    }
}

type TestEngine = Arc<Engine<MemoryCredentials>>;
type Source = EngineAccountSource<MemoryCredentials>;

/// A handle of its own on the fake, signed in and unlocked.
fn signed_in(fake: &Fake) -> TestEngine {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "account-deletion-test-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: fake.api.base.clone(),
    };
    let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens");
    engine.restore();
    engine
        .sign_in_password(&fake.account.email, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(fake.phrase.clone()))
        .expect("the phrase unlocks");
    Arc::new(engine)
}

/// Whether the source keeps a typed password.
fn kept(source: &Source) -> bool {
    held(&source.account_deletion.kept).is_some()
}

/// Step 1 (DW-A58): the password, proven.
fn proven(source: &mut Source) -> StepUp {
    source
        .step_up(Password::from_field(PASSWORD), Purpose::DeleteAccount)
        .expect("the password is proven")
}

#[test]
fn deletion_request_schedules_nothing() {
    let fake = fake();
    let mut source = Source::new(signed_in(&fake));
    for pane in [
        Pane::Delete,
        Pane::DeleteFactor,
        Pane::DeleteLink,
        Pane::DeleteScheduled,
        Pane::DeleteUndone,
    ] {
        assert_eq!(source.pane_state(pane), PaneState::Ready, "{pane:?}");
    }
    let confirmation = proven(&mut source);
    assert!(kept(&source));
    source.request_deletion(confirmation, CODE).expect("the link goes out");
    assert_eq!(source.deletion(), Deletion::LinkSent, "a link is out");
    assert_eq!(*fake.state.lock().unwrap(), "requested", "and nothing is scheduled");
    assert!(!kept(&source), "the password goes once the request was asked");
    assert!(!source.deletion_link_used(), "Beebeeb says the link was not used");
    assert_eq!(source.deletion(), Deletion::LinkSent);
}

#[test]
fn a_wrong_code_hands_the_confirmation_back() {
    let fake = fake();
    let mut source = Source::new(signed_in(&fake));
    let confirmation = proven(&mut source);
    let answer = source
        .request_deletion(confirmation, WRONG_CODE)
        .expect_err("a wrong code is refused");
    assert_eq!(answer.why, CodeRefused::Wrong);
    assert_eq!(*fake.state.lock().unwrap(), "none", "nothing was asked");
    assert!(kept(&source), "the proof of the password stands");
    // The same step stays: the person retypes the code with the confirmation handed back, never the password.
    let confirmation = answer
        .confirmation
        .expect("handed back: the code is retyped, not the password");
    source
        .request_deletion(confirmation, CODE)
        .expect("the right code goes through");
    assert_eq!(*fake.state.lock().unwrap(), "requested");
    assert_eq!(source.deletion(), Deletion::LinkSent);
}

#[test]
fn a_lock_in_between_drops_the_kept_password() {
    let fake = fake();
    let engine = signed_in(&fake);
    let mut source = Source::new(engine.clone());
    let confirmation = proven(&mut source);
    assert!(kept(&source));
    engine.lock_screen().expect("an open lock locks the screen");
    // Dropped at once, with no call of the source made.
    let began = Instant::now();
    while kept(&source) {
        assert!(
            began.elapsed() < Duration::from_secs(2),
            "the lock did not drop the kept password"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let answer = source
        .request_deletion(confirmation, CODE)
        .expect_err("nothing is kept");
    assert_eq!(answer.why, CodeRefused::NotConfirmed);
    assert!(answer.confirmation.is_none(), "the password is asked again");
    assert_eq!(fake.asked("POST /api/v1/auth/account/delete-request"), 0);
}

#[test]
fn undo_needs_the_step_up() {
    let fake = fake();
    let mut source = Source::new(signed_in(&fake));
    *fake.state.lock().unwrap() = "scheduled";
    // A confirmation this source did not make, or one for another purpose, sends nothing.
    assert_eq!(
        source.undo_deletion(StepUp::granted(Purpose::DeleteAccount, 9_999)),
        Err(Refused::NotConfirmed)
    );
    let other = source
        .step_up(Password::from_field(PASSWORD), Purpose::ChangeEmail)
        .expect("proven for another purpose");
    assert_eq!(source.undo_deletion(other), Err(Refused::NotConfirmed));
    // A wrong password proves nothing and keeps nothing.
    assert_eq!(
        source
            .step_up(Password::from_field("not the password"), Purpose::DeleteAccount)
            .err(),
        Some(Refused::WrongPassword)
    );
    assert!(!kept(&source));
    assert_eq!(fake.asked("POST /api/v1/auth/account/delete-cancel"), 0);
    assert_eq!(*fake.state.lock().unwrap(), "scheduled", "nothing changed");

    let confirmation = proven(&mut source);
    assert_eq!(source.undo_deletion(confirmation), Ok(Deletion::None));
    assert_eq!(*fake.state.lock().unwrap(), "none");
    assert_eq!(source.deletion(), Deletion::None);
    assert!(!kept(&source), "used up");
    let again = StepUp::granted(Purpose::DeleteAccount, DELETION_HANDLES + source.account_deletion.made);
    assert_eq!(
        source.undo_deletion(again),
        Err(Refused::NotConfirmed),
        "a confirmation is used once"
    );
    assert_eq!(fake.asked("POST /api/v1/auth/account/delete-cancel"), 1);
}

#[test]
fn a_cancel_before_the_link_is_used_says_nothing_is_scheduled() {
    let fake = fake();
    let mut source = Source::new(signed_in(&fake));
    *fake.state.lock().unwrap() = "requested";
    let confirmation = proven(&mut source);
    // Beebeeb has no route that stops a link not used yet: nothing was cancelled, and it says so.
    assert_eq!(source.cancel_deletion_request(confirmation), Ok(Deletion::LinkSent));
    assert_eq!(fake.asked("POST /api/v1/auth/account/delete-cancel"), 1);
    assert_eq!(*fake.state.lock().unwrap(), "requested");
}

#[test]
fn subscription_refusal_is_honest() {
    let fake = fake();
    let mut source = Source::new(signed_in(&fake));
    fake.mode.store(PLAN_STILL_CHARGED, Ordering::SeqCst);
    let confirmation = proven(&mut source);
    let answer = source.request_deletion(confirmation, CODE).expect_err("refused");
    assert_ne!(answer.why, CodeRefused::Wrong, "never said as a wrong code");
    assert_ne!(answer.why, CodeRefused::Offline, "never said as 'can't be reached'");
    assert!(answer.confirmation.is_none(), "no code can change it");
    assert_eq!(source.notice(), Some(PLAN_FIRST));
    assert!(source.notice_failed());
    assert_eq!(source.deletion(), Deletion::None);
    assert!(!kept(&source));
}

#[test]
fn late_answer_dropped() {
    let fake = fake();
    let engine = signed_in(&fake);
    let mut source = Source::new(engine.clone());
    let confirmation = proven(&mut source);
    // The screen locks while the request is out: its answer belongs to a turn that is over.
    let locking = engine.clone();
    *fake.during_request.lock().unwrap() = Some(Box::new(move || {
        let _ = locking.lock_screen();
    }));
    let answer = source
        .request_deletion(confirmation, CODE)
        .expect_err("the answer came for a session that is over");
    assert!(answer.confirmation.is_none());
    assert_eq!(source.deletion(), Deletion::None, "nothing is shown as asked");
    assert!(!kept(&source));
    assert_eq!(source.notice(), None, "nothing is said");
}

/// A request whose link went out while the screen locked (the lock lands in the round trip, after the re-check).
fn requested_under_a_lock(fake: &Fake, engine: &TestEngine, source: &mut Source) {
    let confirmation = proven(source);
    let locking = engine.clone();
    *fake.during_request.lock().unwrap() = Some(Box::new(move || {
        let _ = locking.lock_screen();
    }));
    source
        .request_deletion(confirmation, CODE)
        .expect_err("the answer came for a turn that is over");
    assert_eq!(
        fake.asked("POST /api/v1/auth/account/delete-request"),
        1,
        "the link went out"
    );
}

/// P3-6 residual: the link was mailed under a lock, so the person is told after the unlock (once).
#[test]
fn a_link_mailed_under_a_lock_is_told_after_the_unlock() {
    let fake = fake();
    let engine = signed_in(&fake);
    let mut source = Source::new(engine.clone());
    source.opened(Pane::Security);
    requested_under_a_lock(&fake, &engine, &mut source);
    assert_eq!(engine.gate(), GateState::ScreenLocked);
    source.pump();
    assert_eq!(source.notice(), None, "nothing is said over the lock");

    engine
        .unlock_screen_platform_verified(engine.session_turn())
        .expect("the screen opens");
    source.pump();
    assert_eq!(source.said().map(|said| said.words), Some(LINK_SENT_LATE));
    assert!(source.account_deletion.late.is_none(), "said once");
    source.clear_notice();
    source.pump();
    assert_eq!(source.notice(), None, "not said again");
}

/// A closed vault (a sign-out) wipes it unsaid.
#[test]
fn a_link_mailed_under_a_lock_is_never_told_after_a_sign_out() {
    let fake = fake();
    let engine = signed_in(&fake);
    let mut source = Source::new(engine.clone());
    source.opened(Pane::Security);
    requested_under_a_lock(&fake, &engine, &mut source);
    let _ = engine.lock_vault();
    source.pump();
    assert!(source.account_deletion.late.is_none(), "wiped with the session");
    assert_eq!(source.notice(), None);
}

/// Another account signed in: the kept link of the first is never said to the second.
#[test]
fn a_link_mailed_for_another_account_is_never_told() {
    let fake = fake();
    let engine = signed_in(&fake);
    let mut source = Source::new(engine.clone());
    source.opened(Pane::Security);
    let mut other = stamp(&engine);
    other.1 = Some("another-account@example.eu".to_owned());
    source.account_deletion.late = Some(other);
    source.pump();
    assert!(source.account_deletion.late.is_none(), "wiped unsaid");
    assert_eq!(source.notice(), None);
}

/// The mutant's companion: "I've opened the link" is Beebeeb's answer, never this PC's guess.
#[test]
fn link_used_reads_the_server() {
    let fake = fake();
    let mut source = Source::new(signed_in(&fake));
    *fake.state.lock().unwrap() = "requested";
    assert!(
        !source.deletion_link_used(),
        "Beebeeb says the link was not used: nothing is scheduled"
    );
    assert_eq!(fake.asked("GET /api/v1/auth/account/deletion"), 1, "Beebeeb was asked");
    assert_eq!(source.deletion(), Deletion::LinkSent);
    *fake.state.lock().unwrap() = "scheduled";
    assert!(source.deletion_link_used());
    let until = DateTime::parse_from_rfc3339(CANCEL_UNTIL).unwrap().with_timezone(&Utc);
    assert_eq!(
        source.deletion(),
        Deletion::Scheduled { on: on_words(until) },
        "named by Beebeeb's last moment to undo it, never a length of time this app assumes"
    );
    assert!(on_words(until).contains("2099"), "another year is said");
}

#[test]
fn password_never_logged() {
    let fake = fake();
    let mut source = Source::new(signed_in(&fake));
    let confirmation = proven(&mut source);
    let answer = source
        .request_deletion(confirmation, WRONG_CODE)
        .expect_err("a wrong code");
    let printed = format!("{answer:?}");
    source
        .request_deletion(answer.confirmation.expect("handed back"), CODE)
        .expect("asked");
    assert!(!printed.contains(PASSWORD));
    // OPAQUE: the password is in no request, neither a body nor a header.
    let seen = fake.seen.lock().unwrap().clone();
    assert!(!seen.is_empty());
    assert!(
        seen.iter().all(|text| !text.contains(PASSWORD)),
        "the password left in a request"
    );
    // Every log line of the slice says a kind and the engine's few words, never a value it was given.
    let code = include_str!("account_deletion.rs");
    let logs = code.matches("log(format!(").count();
    assert_eq!(logs, 4);
    assert_eq!(
        code.matches(" ({})\", error.words()));").count(),
        logs,
        "a log line formats something other than the error's words"
    );
}

/// The delete step-up's confirmation is never taken for one `account_actions.rs` holds for another purpose (that file
/// looks a confirmation up by its number before it checks the purpose).
#[test]
fn a_delete_step_up_never_spends_another_purposes_confirmation() {
    let fake = fake();
    let mut source = Source::new(signed_in(&fake));
    let email = source
        .step_up(Password::from_field(PASSWORD), Purpose::ChangeEmail)
        .expect("proven for the email change");
    let delete = proven(&mut source);
    assert_ne!(delete.handle(), email.handle());
    assert!(source.actions.spend(&delete, Purpose::ChangeEmail).is_none());
    assert_eq!(
        source.actions.held(),
        1,
        "the email change's confirmation is still held"
    );
    assert!(source.actions.spend(&email, Purpose::ChangeEmail).is_some());
}

/// A cancel whose answer is lost, while Beebeeb still has the deletion scheduled, did not land: nothing changed, and
/// it is never said as "it can no longer be undone".
#[test]
fn a_cancel_whose_answer_is_lost_is_never_said_as_too_late() {
    let fake = fake();
    let mut source = Source::new(signed_in(&fake));
    *fake.state.lock().unwrap() = "scheduled";
    fake.mode.store(CANCEL_ANSWER_LOST, Ordering::SeqCst);
    let confirmation = proven(&mut source);
    assert_eq!(source.undo_deletion(confirmation), Err(Refused::Offline));
    assert_ne!(source.notice(), Some(TOO_LATE_TO_UNDO));
    assert!(
        matches!(source.deletion(), Deletion::Scheduled { .. }),
        "Beebeeb still has it"
    );
    assert_eq!(*fake.state.lock().unwrap(), "scheduled");
}

/// "I've opened the link" when Beebeeb cannot be asked: not known, said as a check that failed, never "not used".
#[test]
fn a_check_that_failed_never_says_the_link_was_not_used() {
    let fake = fake();
    let mut source = Source::new(signed_in(&fake));
    *fake.state.lock().unwrap() = "scheduled";
    fake.mode.store(STATE_FAILS, Ordering::SeqCst);
    assert!(!source.deletion_link_used());
    assert_eq!(source.notice(), Some(NOT_CHECKED));
    assert!(
        source.notice_failed(),
        "the page reads this and does not say the link was not used"
    );
    assert_eq!(
        source.deletion(),
        Deletion::None,
        "nothing was read, nothing is shown as known"
    );
}

/// A one-shot hook a test sets to run during a request.
type Hook = Arc<Mutex<Option<Box<dyn FnOnce() + Send>>>>;
