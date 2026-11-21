//! Two-factor and change email on the real engine: the answer shown from Beebeeb, a refusal said honestly, an answer
//! that lands after the session moved on dropped, and the panes reachable. Every name is invented. The email's first
//! step (a password proof and an OPAQUE registration under the new address) has no fake server: only its checks that
//! need none are tested here.

use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use beebeeb_engine::app::EngineConfig;
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use beebeeb_engine::test_api::password_change::PasswordChangeRoutes;
use beebeeb_engine::test_api::{FakeApi, Handler, Reply};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::*;
use crate::frames::account::source::{AccountSource, Password, Purpose, Refused, Resend};

const PASSWORD: &str = "an invented password for the account";
const SETUP: &str = r#"{"secret":"JBSWY3DPEHPK3PXP","qr_uri":"otpauth://totp/Beebeeb:x?secret=JBSWY3DPEHPK3PXP","backup_codes":["11111111","22222222"]}"#;

struct Fake {
    api: FakeApi,
    /// The enable route answers 401 (a wrong code).
    wrong_code: Arc<AtomicBool>,
    /// The disable route waits this many milliseconds before it answers.
    disable_delay_ms: Arc<AtomicU64>,
    /// The disable and the backup-codes routes do not answer while this is set (the test lets them go).
    held: Arc<AtomicBool>,
    asked: Arc<Mutex<Vec<String>>>,
    /// The status the email's resend answers with (200: sent; 404; 429 wait; 500).
    resend_status: Arc<AtomicU16>,
    /// The email's code is taken (200) or refused (400).
    verify_ok: Arc<AtomicBool>,
    account: Arc<FakeAccount>,
    phrase: String,
    /// Beebeeb takes a password change (the step-up and the new registration, `PasswordChangeRoutes`); off: no route.
    password_changes: Arc<AtomicBool>,
}

fn fake() -> Fake {
    let (account, phrase) = FakeAccount::register("two-factor@example.eu", PASSWORD, TEST_PHRASE);
    let account = Arc::new(account);
    let rest: Handler = account_handler(account.clone());
    let wrong_code = Arc::new(AtomicBool::new(false));
    let disable_delay_ms = Arc::new(AtomicU64::new(0));
    let asked: Arc<Mutex<Vec<String>>> = Arc::default();
    let (wrong, delay, log) = (wrong_code.clone(), disable_delay_ms.clone(), asked.clone());
    let held_out = Arc::new(AtomicBool::new(false));
    let held = held_out.clone();
    let (resend_status, verify_ok) = (Arc::new(AtomicU16::new(200)), Arc::new(AtomicBool::new(false)));
    let (resend_in, verify_in) = (resend_status.clone(), verify_ok.clone());
    let password_changes = Arc::new(AtomicBool::new(false));
    let (changes_in, changing) = (password_changes.clone(), account.clone());
    let routes = PasswordChangeRoutes::new(&account.email);
    let api = FakeApi::start(Arc::new(move |req| {
        let route = format!("{} {}", req.method, req.path);
        log.lock().unwrap().push(route.clone());
        if changes_in.load(Ordering::SeqCst)
            && let Some(reply) = routes.answer(req, &changing.opaque)
        {
            return reply;
        }
        match route.as_str() {
            "POST /api/v1/auth/2fa/setup" => Reply::ok(serde_json::from_str::<Value>(SETUP).unwrap()),
            "POST /api/v1/auth/2fa/enable" if wrong.load(Ordering::SeqCst) => Reply::json(401, json!({})),
            "POST /api/v1/auth/2fa/enable" => Reply::ok(json!({})),
            "POST /api/v1/auth/2fa/disable" => {
                hold(&held);
                std::thread::sleep(Duration::from_millis(delay.load(Ordering::SeqCst)));
                Reply::ok(json!({}))
            }
            "POST /api/v1/auth/2fa/backup-codes" => {
                hold(&held);
                std::thread::sleep(Duration::from_millis(delay.load(Ordering::SeqCst)));
                Reply::ok(json!({ "backup_codes": ["11111111", "22222222"] }))
            }
            "POST /api/v1/auth/verify-email" if verify_in.load(Ordering::SeqCst) => Reply::ok(json!({})),
            "POST /api/v1/auth/verify-email" => Reply::json(400, json!({ "error": "invalid_code" })),
            "POST /api/v1/me/email/resend" => match resend_in.load(Ordering::SeqCst) {
                200 => Reply::ok(
                    json!({ "pending_email": "new.address@example.eu", "code_expires_in": 900, "resends_left": 2 }),
                ),
                429 => Reply::error(429, "email_code_resend_wait").retry_after(42),
                status => Reply::error(status, "refused"),
            },
            "GET /api/v1/auth/me" => {
                // The profile says two-factor is on.
                let mut reply = rest(req);
                if let Ok(mut body) = serde_json::from_slice::<Value>(&reply.body) {
                    body["totp_enabled"] = json!(true);
                    reply.body = serde_json::to_vec(&body).unwrap();
                }
                reply
            }
            _ => rest(req),
        }
    }));
    Fake {
        api,
        wrong_code,
        disable_delay_ms,
        held: held_out,
        asked,
        resend_status,
        verify_ok,
        account,
        phrase,
        password_changes,
    }
}

type TestEngine = Arc<Engine<MemoryCredentials>>;
type Source = EngineAccountSource<MemoryCredentials>;

fn signed_in(fake: &Fake) -> TestEngine {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "two-factor-test-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: fake.api.base.clone(),
    };
    let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens");
    engine.restore();
    let engine = Arc::new(engine);
    engine
        .sign_in_password(&fake.account.email, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(fake.phrase.clone()))
        .expect("the phrase unlocks");
    engine
}

/// A wait that lasts this long is a call or a request that hangs: the test fails. Never what a test waits for, and
/// never how long one may take: every wait ends on its own event.
const HANGS: Duration = Duration::from_secs(300);

/// Pump until a call answers. `None`: no call is out, so nothing is left that could answer. A call ends when its
/// worker does (it sends an answer whatever happened, a panic included), not within a time: a password change is
/// several key derivations and takes as long as the machine gives it.
fn answer(source: &mut Source) -> Option<(CallKind, Answered)> {
    let began = Instant::now();
    loop {
        source.pump();
        if let Some(answered) = source.call_answer() {
            return Some(answered);
        }
        if source.calls.out.is_empty() {
            return None;
        }
        assert!(began.elapsed() < HANGS, "a call that is out never ended");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A route's answer waits here while the test holds it.
fn hold(held: &AtomicBool) {
    let began = Instant::now();
    while held.load(Ordering::SeqCst) && began.elapsed() < HANGS {
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Wait until Beebeeb was asked `route` (the request is out, sent under the session as it is now).
fn asked(fake: &Fake, route: &str) {
    let began = Instant::now();
    while !fake.asked.lock().unwrap().iter().any(|asked| asked.ends_with(route)) {
        assert!(began.elapsed() < HANGS, "{route} was never asked");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn settle(source: &mut Source) {
    let began = Instant::now();
    while source.pump().busy {
        assert!(began.elapsed() < Duration::from_secs(10), "a read did not come back");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_set_up_shows_the_key_beebeeb_made_and_the_first_code_turns_it_on() {
    let fake = fake();
    let mut source = Source::new(signed_in(&fake));
    assert!(source.start_call(CallKind::BeginTwoFactor, "", ""));
    // A second press while the key is being made sends nothing new.
    assert!(source.start_call(CallKind::BeginTwoFactor, "", ""));
    let Some((CallKind::BeginTwoFactor, Answered::Setup(setup))) = answer(&mut source) else {
        panic!("the set-up was not answered with a key");
    };
    assert_eq!(setup.key.expose(), "JBSW Y3DP EHPK 3PXP");
    assert!(setup.modules.len() >= 21 && setup.modules.iter().all(|row| row.len() == setup.modules.len()));
    assert!(
        setup.modules.iter().flatten().any(|dark| *dark),
        "a code with dark modules"
    );
    let setups = fake
        .asked
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.ends_with("2fa/setup"))
        .count();
    assert_eq!(setups, 1);

    assert!(source.start_call(CallKind::TurnOnTwoFactor, "123456", ""));
    let Some((CallKind::TurnOnTwoFactor, Answered::Codes(codes))) = answer(&mut source) else {
        panic!("the code was not taken");
    };
    assert_eq!(codes.len(), 2);
    assert_eq!(codes[0].expose(), "11111111");
}

#[test]
fn a_wrong_first_code_is_said_and_the_spent_key_is_replaced() {
    let fake = fake();
    let mut source = Source::new(signed_in(&fake));
    assert!(source.start_call(CallKind::BeginTwoFactor, "", ""));
    assert!(matches!(answer(&mut source), Some((_, Answered::Setup(_)))));
    fake.wrong_code.store(true, Ordering::SeqCst);
    assert!(source.start_call(CallKind::TurnOnTwoFactor, "000000", ""));
    assert!(matches!(
        answer(&mut source),
        Some((CallKind::TurnOnTwoFactor, Answered::NewKey(_)))
    ));
    let setups = fake
        .asked
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.ends_with("2fa/setup"))
        .count();
    assert_eq!(setups, 2, "a new key was asked for");
    // The old key is gone with the code: a press with a right code now works on the new key.
    fake.wrong_code.store(false, Ordering::SeqCst);
    assert!(source.start_call(CallKind::TurnOnTwoFactor, "123456", ""));
    assert!(matches!(answer(&mut source), Some((_, Answered::Codes(_)))));
}

#[test]
fn a_refused_code_to_turn_off_is_a_wrong_code_and_offline_is_not() {
    let fake = fake();
    let mut source = Source::new(signed_in(&fake));
    assert!(source.start_call(CallKind::TurnOffTwoFactor, "123456", ""));
    assert!(matches!(
        answer(&mut source),
        Some((CallKind::TurnOffTwoFactor, Answered::Done))
    ));
    // A short code never reaches Beebeeb.
    assert!(source.start_call(CallKind::ConfirmEmail, "12", ""));
    assert!(matches!(
        answer(&mut source),
        Some((CallKind::ConfirmEmail, Answered::Code(CodeRefused::Wrong)))
    ));
    assert!(source.start_call(CallKind::StartEmail, "not an address", "a password"));
    assert!(matches!(
        answer(&mut source),
        Some((CallKind::StartEmail, Answered::Email(EmailRefused::NotAnAddress)))
    ));
    // The server answering 400 to the email's code is a wrong code, and the session is not held as ended.
    assert!(source.start_call(CallKind::ConfirmEmail, "12345678", ""));
    assert!(matches!(
        answer(&mut source),
        Some((CallKind::ConfirmEmail, Answered::Code(CodeRefused::Wrong)))
    ));
    assert_ne!(source.pane_state(Pane::Email), PaneState::SessionEnded);
}

#[test]
fn an_answer_that_lands_after_the_session_moved_on_is_dropped() {
    let fake = fake();
    let engine = signed_in(&fake);
    let mut source = Source::new(engine.clone());
    fake.disable_delay_ms.store(400, Ordering::SeqCst);
    assert!(source.start_call(CallKind::TurnOffTwoFactor, "123456", ""));
    engine.sign_out_keeping_local_data().expect("signs out");
    std::thread::sleep(Duration::from_millis(700));
    source.pump();
    assert!(
        source.call_answer().is_none(),
        "nothing of the first session is answered"
    );
    assert!(
        !source.start_call(CallKind::TurnOffTwoFactor, "123456", ""),
        "no session: nothing is started"
    );
}

#[test]
fn the_panes_of_email_and_two_factor_are_reachable_and_two_factor_waits_for_the_mark() {
    let fake = fake();
    let mut source = Source::new(signed_in(&fake));
    for pane in [Pane::Email, Pane::EmailCode, Pane::TwoFactorSetup, Pane::BackupCodes] {
        assert_eq!(source.pane_state(pane), PaneState::Ready, "{pane:?}");
    }
    assert_eq!(
        source.pane_state(Pane::TwoFactor),
        PaneState::Loading,
        "no mark read yet"
    );
    assert!(!source.two_factor().on, "nothing says it is on before it was read");
    source.opened(Pane::Security);
    settle(&mut source);
    assert_eq!(source.pane_state(Pane::TwoFactor), PaneState::Ready);
    let two = source.two_factor();
    assert!(two.on);
    assert_eq!(
        (
            two.added.as_str(),
            two.last_used.as_str(),
            two.codes_left,
            two.codes_total
        ),
        ("", "", 0, 0),
        "what Beebeeb does not say is not drawn"
    );
    assert!(
        !source.can_send_again_deletion(),
        "no route sends the deletion's code again"
    );
    // The Name page and the password change answer too (`account_actions.rs`, `account_password.rs`).
    assert_ne!(source.pane_state(Pane::Name), PaneState::Unwired);
    assert_ne!(source.pane_state(Pane::Password), PaneState::Unwired);
    // Deleting the account answers too (slice A6, `account_deletion.rs`).
    assert_ne!(source.pane_state(Pane::Delete), PaneState::Unwired);
}

fn resends(fake: &Fake) -> usize {
    fake.asked
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.ends_with("/me/email/resend"))
        .count()
}

#[test]
fn send_again_is_one_request_per_press_and_each_answer_is_its_state() {
    let fake = fake();
    let mut source = Source::new(signed_in(&fake));
    assert_eq!(
        source.resend_wait(),
        0,
        "no wait is invented before Beebeeb asks for one"
    );
    assert!(source.start_call(CallKind::ResendEmail, "", ""));
    assert!(matches!(
        answer(&mut source),
        Some((CallKind::ResendEmail, Answered::Done))
    ));
    assert_eq!(resends(&fake), 1);

    // The change is gone: the page goes back to step 1.
    fake.resend_status.store(404, Ordering::SeqCst);
    assert!(source.start_call(CallKind::ResendEmail, "", ""));
    assert!(matches!(
        answer(&mut source),
        Some((CallKind::ResendEmail, Answered::Resend(Resend::Gone)))
    ));
    // Too soon: the wait is Beebeeb's Retry-After, counted down by the source, which wakes the window each second.
    fake.resend_status.store(429, Ordering::SeqCst);
    assert!(source.start_call(CallKind::ResendEmail, "", ""));
    assert!(matches!(
        answer(&mut source),
        Some((CallKind::ResendEmail, Answered::Resend(Resend::Wait(42))))
    ));
    assert!((41..=42).contains(&source.resend_wait()), "{}", source.resend_wait());
    let pumped = source.pump();
    assert!(pumped.changed && pumped.wake_in == Some(Duration::from_secs(1)));
    assert_eq!(resends(&fake), 3, "three presses, three requests, no hidden retry");
    // Another failure is offline, not a wait.
    fake.resend_status.store(500, Ordering::SeqCst);
    assert!(source.start_call(CallKind::ResendEmail, "", ""));
    assert!(matches!(
        answer(&mut source),
        Some((CallKind::ResendEmail, Answered::Resend(Resend::Offline)))
    ));
}

#[test]
fn a_confirmed_email_change_replaces_the_mirrored_address_and_the_answer_is_not_lost() {
    let fake = fake();
    let engine = signed_in(&fake);
    let mut source = Source::new(engine.clone());
    let old = engine.account_email();
    assert!(old.is_some());
    // A confirm with no address Beebeeb named leaves the mirrored address alone.
    fake.verify_ok.store(true, Ordering::SeqCst);
    assert!(source.start_call(CallKind::ConfirmEmail, "12345678", ""));
    assert!(matches!(
        answer(&mut source),
        Some((CallKind::ConfirmEmail, Answered::Done))
    ));
    assert_eq!(engine.account_email(), old);

    // A resend names the address; the confirm then makes it the account's.
    assert!(source.start_call(CallKind::ResendEmail, "", ""));
    assert!(matches!(
        answer(&mut source),
        Some((CallKind::ResendEmail, Answered::Done))
    ));
    assert!(source.start_call(CallKind::ConfirmEmail, "12345678", ""));
    assert!(
        matches!(answer(&mut source), Some((CallKind::ConfirmEmail, Answered::Done))),
        "the answer is not dropped by the address it changed"
    );
    assert_eq!(engine.account_email().as_deref(), Some("new.address@example.eu"));
    assert!(
        source.call_mark(CallKind::ConfirmEmail).unwrap().current,
        "the drain's own write of the address re-stamps the call (#440 review, P2)"
    );
    // A refused code changes nothing.
    fake.verify_ok.store(false, Ordering::SeqCst);
    assert!(source.start_call(CallKind::ResendEmail, "", ""));
    let _ = answer(&mut source);
    assert!(source.start_call(CallKind::ConfirmEmail, "12345678", ""));
    assert!(matches!(
        answer(&mut source),
        Some((CallKind::ConfirmEmail, Answered::Code(CodeRefused::Wrong)))
    ));
    assert_eq!(engine.account_email().as_deref(), Some("new.address@example.eu"));
}

/// Review of desktop#424, delta: new backup codes Beebeeb already made reach the pane when the screen locks while the
/// request is out (the session's turn moves, the account is the same): kept by the source while the screen is locked,
/// handed over once after the unlock (`late_codes`), then wiped. Another account's, or an ended session's, answer is
/// still dropped (the test above); the window's side is `account_twofactor_tests_window.rs`.
#[test]
fn new_backup_codes_reach_the_pane_when_the_screen_locks_while_the_request_is_out() {
    let fake = fake();
    let engine = signed_in(&fake);
    let mut source = Source::new(engine.clone());
    // Beebeeb holds its answer from the moment it was asked until the screen is locked: the request went out under
    // the open screen and its answer lands under the lock, however long the worker and the lock take.
    fake.held.store(true, Ordering::SeqCst);
    assert!(source.start_call(CallKind::NewBackupCodes, "123456", ""));
    asked(&fake, "2fa/backup-codes");
    engine.lock_screen().expect("an open lock locks the screen");
    fake.held.store(false, Ordering::SeqCst);
    // The answer lands under the lock: kept, never handed over while the screen is locked. The call is out until
    // its answer was taken in, so the loop ends on that and not on a time.
    let began = Instant::now();
    loop {
        source.pump();
        assert!(source.call_answer().is_none(), "not answered under the lock");
        assert!(source.late_codes().is_none(), "never handed over under the lock");
        if source.calls.out.is_empty() {
            break;
        }
        assert!(began.elapsed() < HANGS, "the call never ended");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(source.late_codes().is_none(), "never handed over under the lock");
    engine
        .unlock_screen_platform_verified(engine.session_turn())
        .expect("the platform's yes unlocks the screen");
    let (kind, codes) = source
        .late_codes()
        .expect("the codes Beebeeb made are handed over after the unlock, not wiped unseen");
    assert_eq!(kind, CallKind::NewBackupCodes);
    assert_eq!(codes.len(), 2);
    assert!(
        source.late_codes().is_none() && source.call_answer().is_none(),
        "shown once: nothing is held for a second look"
    );
}

#[path = "account_twofactor_tests_window.rs"]
mod window;

const NEW_PASSWORD: &str = "A New Invented Password 42";

fn asks_for_a_change(fake: &Fake) -> usize {
    fake.asked
        .lock()
        .unwrap()
        .iter()
        .filter(|route| route.contains("password"))
        .count()
}

#[test]
fn a_password_change_is_asked_on_a_worker_and_answered_through_the_call() {
    let fake = fake();
    let mut source = Source::new(signed_in(&fake));
    source
        .step_up(Password::from_field(PASSWORD), Purpose::ChangePassword)
        .expect("the step-up keeps the typed password");
    assert!(
        source.start_call(CallKind::ChangePassword, NEW_PASSWORD, ""),
        "taken: the engine's call runs on a worker"
    );
    let Some((CallKind::ChangePassword, answered)) = answer(&mut source) else {
        panic!("the change was not answered");
    };
    // The fake has no password route: Beebeeb did not take it, and the form says so (never a fake success).
    assert!(matches!(answered, Answered::Refused(_)), "nothing went through");
    assert!(answer(&mut source).is_none(), "one answer for one press");
}

#[test]
fn a_password_change_with_nothing_kept_is_not_started_and_asks_nothing() {
    let fake = fake();
    let mut source = Source::new(signed_in(&fake));
    let before = asks_for_a_change(&fake);
    assert!(
        !source.start_call(CallKind::ChangePassword, NEW_PASSWORD, ""),
        "no step-up kept a password: nothing starts"
    );
    // The form then asks the sync method, which refuses it as not confirmed.
    let forged = crate::frames::account::source::StepUp::granted(Purpose::ChangePassword, 1);
    assert_eq!(
        source.change_password(forged, Password::from_field(NEW_PASSWORD)).err(),
        Some(Refused::NotConfirmed)
    );
    assert_eq!(asks_for_a_change(&fake), before, "nothing was sent");
}

#[test]
fn a_password_change_answered_after_the_session_ended_is_dropped() {
    let fake = fake();
    let engine = signed_in(&fake);
    let mut source = Source::new(engine.clone());
    source
        .step_up(Password::from_field(PASSWORD), Purpose::ChangePassword)
        .expect("kept");
    assert!(source.start_call(CallKind::ChangePassword, NEW_PASSWORD, ""));
    engine.sign_out_keeping_local_data().expect("signs out");
    std::thread::sleep(Duration::from_millis(700));
    source.pump();
    assert!(
        source.call_answer().is_none(),
        "nothing of the ended session is answered"
    );
}

#[test]
fn a_second_password_press_while_the_first_is_out_is_never_started() {
    let fake = fake();
    let mut source = Source::new(signed_in(&fake));
    source
        .step_up(Password::from_field(PASSWORD), Purpose::ChangePassword)
        .expect("kept");
    assert!(source.start_call(CallKind::ChangePassword, NEW_PASSWORD, ""), "press A");
    // Press B: its step-up keeps a password again, and the call is refused: not "started", no answer of A's is B's.
    source
        .step_up(Password::from_field(PASSWORD), Purpose::ChangePassword)
        .expect("kept");
    assert!(
        !source.start_call(CallKind::ChangePassword, "A Second Invented Password 7", ""),
        "B is not taken while A is out"
    );
    // The password B's step-up kept is wiped: nothing is left to start a change with.
    assert!(!source.start_call(CallKind::ChangePassword, "A Third Invented Password 9", ""));
    let Some((CallKind::ChangePassword, _)) = answer(&mut source) else {
        panic!("A's answer comes");
    };
    assert!(answer(&mut source).is_none(), "one answer, for A only");
}

#[test]
fn a_page_that_comes_back_can_see_that_a_password_change_is_still_out_and_which_call() {
    let fake = fake();
    let mut source = Source::new(signed_in(&fake));
    assert_eq!(source.call_mark(CallKind::ChangePassword), None);
    source
        .step_up(Password::from_field(PASSWORD), Purpose::ChangePassword)
        .expect("kept");
    assert!(source.start_call(CallKind::ChangePassword, NEW_PASSWORD, ""));
    let first = source.call_mark(CallKind::ChangePassword).expect("started");
    assert!(first.out, "out until its answer is taken");
    assert!(answer(&mut source).is_some());
    let done = source
        .call_mark(CallKind::ChangePassword)
        .expect("the call is remembered");
    assert!(!done.out, "taken: no longer out");
    assert_eq!(done.serial, first.serial, "the answer is for the call that was started");
    // A later press is a later call: its serial is never the earlier one's.
    source
        .step_up(Password::from_field(PASSWORD), Purpose::ChangePassword)
        .expect("kept");
    assert!(source.start_call(CallKind::ChangePassword, NEW_PASSWORD, ""));
    assert_ne!(source.call_mark(CallKind::ChangePassword).unwrap().serial, first.serial);
}

#[test]
fn an_answer_taken_under_one_session_is_not_current_once_the_session_moved() {
    let fake = fake();
    let engine = signed_in(&fake);
    let mut source = Source::new(engine.clone());
    source
        .step_up(Password::from_field(PASSWORD), Purpose::ChangePassword)
        .expect("kept");
    assert!(source.start_call(CallKind::ChangePassword, NEW_PASSWORD, ""));
    assert!(
        source.call_mark(CallKind::ChangePassword).unwrap().current,
        "out: current"
    );
    assert!(answer(&mut source).is_some());
    assert!(
        source.call_mark(CallKind::ChangePassword).unwrap().current,
        "taken and nothing moved: current"
    );
    engine.sign_out_keeping_local_data().expect("signs out");
    assert!(
        !source.call_mark(CallKind::ChangePassword).unwrap().current,
        "another session now: a late answer kept for the page is dropped"
    );
}

/// The same account signs in again (or a lock and an unlock move the session's turn): `current` answers for the
/// press's own session, by kind.
fn sign_in_again(engine: &TestEngine, fake: &Fake) {
    engine
        .sign_in_password(&fake.account.email, Zeroizing::new(PASSWORD.into()))
        .expect("signs in again");
    engine
        .unlock_with_phrase(Zeroizing::new(fake.phrase.clone()))
        .expect("the phrase unlocks");
}

/// Desktop#427, the lead's follow-up (1a): a password press, then a sign-out and a sign-in to the SAME account before
/// its answer is taken. The answer is still handed over (the same account), but it was made for the earlier session:
/// it is not current, so a page that kept it never says it in the new session.
#[test]
fn a_password_answer_taken_after_a_sign_out_and_sign_in_to_the_same_account_is_not_current() {
    let fake = fake();
    let engine = signed_in(&fake);
    let mut source = Source::new(engine.clone());
    source
        .step_up(Password::from_field(PASSWORD), Purpose::ChangePassword)
        .expect("kept");
    assert!(source.start_call(CallKind::ChangePassword, NEW_PASSWORD, ""));
    // Nothing takes the answer before the second sign-in.
    std::thread::sleep(Duration::from_millis(300));
    engine.sign_out_keeping_local_data().expect("signs out");
    sign_in_again(&engine, &fake);
    let Some((CallKind::ChangePassword, _)) = answer(&mut source) else {
        panic!("the same account: the answer is handed over");
    };
    assert!(
        !source.call_mark(CallKind::ChangePassword).unwrap().current,
        "made for the earlier session: not current in this one"
    );
}

/// Desktop#427, the lead's follow-up (1b): one slot for every kind let another kind's answer, taken under a moved turn,
/// make the password answer current again. Each kind answers for its own press.
#[test]
fn a_password_answer_stays_not_current_after_another_kind_is_taken_under_a_moved_turn() {
    let fake = fake();
    let engine = signed_in(&fake);
    let mut source = Source::new(engine.clone());
    source
        .step_up(Password::from_field(PASSWORD), Purpose::ChangePassword)
        .expect("kept");
    assert!(source.start_call(CallKind::ChangePassword, NEW_PASSWORD, ""));
    assert!(answer(&mut source).is_some());
    assert!(source.call_mark(CallKind::ChangePassword).unwrap().current);
    // A screen lock and its unlock move the session's turn.
    engine.lock_screen().expect("an open lock locks the screen");
    engine
        .unlock_screen_platform_verified(engine.session_turn())
        .expect("the platform's yes unlocks the screen");
    assert!(source.start_call(CallKind::TurnOffTwoFactor, "123456", ""));
    assert!(matches!(
        answer(&mut source),
        Some((CallKind::TurnOffTwoFactor, Answered::Done))
    ));
    assert!(
        source.call_mark(CallKind::TurnOffTwoFactor).unwrap().current,
        "made and taken under this turn"
    );
    assert!(
        !source.call_mark(CallKind::ChangePassword).unwrap().current,
        "the password press was made under the earlier turn"
    );
}

/// A changed password stores its new session token, and that write moves the session's turn on (`claim`). The change's
/// own answer is still its press's: current under the turn its own write made, which the engine hands back (#440
/// review, P1: only the call's own write extends the mark).
#[test]
fn a_password_change_that_moves_the_turn_by_its_own_write_stays_current() {
    let fake = fake();
    fake.password_changes.store(true, Ordering::SeqCst);
    let engine = signed_in(&fake);
    let mut source = Source::new(engine.clone());
    source
        .step_up(Password::from_field(PASSWORD), Purpose::ChangePassword)
        .expect("kept");
    let before = engine.session_turn();
    assert!(source.start_call(CallKind::ChangePassword, NEW_PASSWORD, ""));
    let Some((CallKind::ChangePassword, answered)) = answer(&mut source) else {
        panic!("the change was not answered");
    };
    assert!(matches!(answered, Answered::Done), "the fake takes the change");
    assert_ne!(
        engine.session_turn(),
        before,
        "the change's own write moved the turn on"
    );
    assert!(
        source.call_mark(CallKind::ChangePassword).unwrap().current,
        "the change's own answer, under the turn its own write made"
    );
}

/// #440 review, P1: a success whose call was out across a lock and an unlock was made for the earlier turn. The engine
/// lets a same-account answer through; only the call's own write extends its mark, so it is not current after the
/// unlock and no page says it in the new turn.
#[test]
fn a_success_answered_after_a_lock_and_an_unlock_is_not_current() {
    let fake = fake();
    let engine = signed_in(&fake);
    let mut source = Source::new(engine.clone());
    // Beebeeb holds its answer from the moment it was asked until the screen was locked and unlocked: the call is
    // out across both, however long its worker took to send it and however long the lock and the unlock take.
    fake.held.store(true, Ordering::SeqCst);
    assert!(source.start_call(CallKind::TurnOffTwoFactor, "123456", ""));
    asked(&fake, "2fa/disable");
    engine.lock_screen().expect("an open lock locks the screen");
    engine
        .unlock_screen_platform_verified(engine.session_turn())
        .expect("the platform's yes unlocks the screen");
    fake.held.store(false, Ordering::SeqCst);
    let Some((CallKind::TurnOffTwoFactor, Answered::Done)) = answer(&mut source) else {
        panic!("the same account: Beebeeb's answer is handed over");
    };
    assert!(
        !source.call_mark(CallKind::TurnOffTwoFactor).unwrap().current,
        "made for the turn before the lock: not current after the unlock"
    );
}
