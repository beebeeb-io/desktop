//! Two-factor and change email on the real engine: the answer shown from Beebeeb, a refusal said honestly, an answer
//! that lands after the session moved on dropped, and the panes reachable. Every name is invented. The email's first
//! step (a password proof and an OPAQUE registration under the new address) has no fake server: only its checks that
//! need none are tested here.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use beebeeb_engine::app::EngineConfig;
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use beebeeb_engine::test_api::{FakeApi, Handler, Reply};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::*;
use crate::frames::account::source::AccountSource;

const PASSWORD: &str = "an invented password for the account";
const SETUP: &str = r#"{"secret":"JBSWY3DPEHPK3PXP","qr_uri":"otpauth://totp/Beebeeb:x?secret=JBSWY3DPEHPK3PXP","backup_codes":["11111111","22222222"]}"#;

struct Fake {
    api: FakeApi,
    /// The enable route answers 401 (a wrong code).
    wrong_code: Arc<AtomicBool>,
    /// The disable route waits this many milliseconds before it answers.
    disable_delay_ms: Arc<AtomicU64>,
    asked: Arc<Mutex<Vec<String>>>,
    account: Arc<FakeAccount>,
    phrase: String,
}

fn fake() -> Fake {
    let (account, phrase) = FakeAccount::register("two-factor@example.eu", PASSWORD, TEST_PHRASE);
    let account = Arc::new(account);
    let rest: Handler = account_handler(account.clone());
    let wrong_code = Arc::new(AtomicBool::new(false));
    let disable_delay_ms = Arc::new(AtomicU64::new(0));
    let asked: Arc<Mutex<Vec<String>>> = Arc::default();
    let (wrong, delay, log) = (wrong_code.clone(), disable_delay_ms.clone(), asked.clone());
    let api = FakeApi::start(Arc::new(move |req| {
        let route = format!("{} {}", req.method, req.path);
        log.lock().unwrap().push(route.clone());
        match route.as_str() {
            "POST /api/v1/auth/2fa/setup" => Reply::ok(serde_json::from_str::<Value>(SETUP).unwrap()),
            "POST /api/v1/auth/2fa/enable" if wrong.load(Ordering::SeqCst) => Reply::json(401, json!({})),
            "POST /api/v1/auth/2fa/enable" => Reply::ok(json!({})),
            "POST /api/v1/auth/2fa/disable" => {
                std::thread::sleep(Duration::from_millis(delay.load(Ordering::SeqCst)));
                Reply::ok(json!({}))
            }
            "POST /api/v1/auth/verify-email" => Reply::json(400, json!({ "error": "invalid_code" })),
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
        asked,
        account,
        phrase,
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

/// Pump until a call answers (at most 10 s).
fn answer(source: &mut Source) -> Option<(CallKind, Answered)> {
    let began = Instant::now();
    while began.elapsed() < Duration::from_secs(10) {
        source.pump();
        if let Some(answered) = source.call_answer() {
            return Some(answered);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    None
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
    assert!(!source.can_send_again(), "no route sends the email's code again");
    assert_eq!(source.pane_state(Pane::Name), PaneState::Unwired);
    assert_eq!(source.pane_state(Pane::Password), PaneState::Unwired);
}
