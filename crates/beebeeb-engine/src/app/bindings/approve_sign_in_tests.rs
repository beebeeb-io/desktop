//! Approving another device's sign-in at the engine, against the local fake API: a real sign-in and unlock (the
//! engine's fake account), then core's lookup, its password step-up of purpose `cli_device_approval` (the real OPAQUE
//! client and server sides), the claim and the relay, and what leaves this PC. The "other device" is a real P-256 key
//! (core's `CliEphemeralKey`): the test opens what was relayed. Every name, address, code and password is invented;
//! the fake is on loopback.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use base64::Engine as _;
use beebeeb_core::cli_auth::CliEphemeralKey;
use serde_json::{Value, json};

use super::*;
use crate::app::EngineConfig;
use crate::host::ChannelHost;
use crate::identity::TEST;
use crate::signin::memory_store::MemoryCredentials;
use crate::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use crate::test_api::{ConfirmTokens, FakeApi, Reply, Request};

type TestEngine = Engine<MemoryCredentials>;

const PASSWORD: &str = "an invented approval password";
const NEW_DEVICE_TOKEN: &str = "session-for-the-other-device";
const CODE: &str = "K7QM-4WTD";

const ANSWERS: u8 = 0;
/// The claim is taken and its answer never comes (the connection closes).
const CLAIM_LOST: u8 = 1;
/// The relay is taken and its answer never comes: the keys left.
const RELAY_LOST: u8 = 2;
/// The relay is answered with a server error: the keys left.
const RELAY_FAILS: u8 = 3;
static MODE: AtomicU8 = AtomicU8::new(ANSWERS);
/// An engine whose screen locks the moment the fake takes the step-up's finish (the app lock, during the approval).
static LOCK_AT_FINISH: Mutex<Option<Arc<TestEngine>>> = Mutex::new(None);
/// An engine whose screen locks the moment the fake takes the claim (the app lock, during the claim's round trip).
static LOCK_AT_CLAIM: Mutex<Option<Arc<TestEngine>>> = Mutex::new(None);

fn b64() -> base64::engine::general_purpose::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

/// One registered account for every test here (a registration runs Argon2id once), and its recovery phrase.
fn account() -> (Arc<FakeAccount>, String) {
    static ACCOUNT: OnceLock<(Arc<FakeAccount>, String)> = OnceLock::new();
    ACCOUNT
        .get_or_init(|| {
            let (account, phrase) = FakeAccount::register("approve-sign-in@example.eu", PASSWORD, TEST_PHRASE);
            (Arc::new(account), phrase)
        })
        .clone()
}

/// What the fake measured about the request, and what the requesting device said about itself.
fn facts() -> Value {
    json!({
        "requested_at": "2026-11-15T09:41:12Z",
        "expires_at": "2999-01-01T00:00:00Z",
        "ip": "203.0.113.47",
        "country": "DE",
        "client": "drive",
        "client_version": "1.0",
        "device_name": "An invented phone",
        "os": "iOS"
    })
}

/// The fake API, the account (its master key is the one the session holds once unlocked), and the requesting
/// device's own key (it never leaves the "device").
struct Fake {
    api: FakeApi,
    account: Arc<FakeAccount>,
    phrase: String,
    requester: Arc<CliEphemeralKey>,
}

fn server() -> Fake {
    let (account, phrase) = account();
    let tokens = Arc::new(ConfirmTokens::default());
    let requester = Arc::new(CliEphemeralKey::generate());
    let public = b64().encode(requester.public_key_bytes());
    let (ours, rest) = (account.clone(), account_handler(account.clone()));
    let api = FakeApi::start(Arc::new(move |request: &Request| {
        let route = request.route();
        let approval = route.starts_with("POST /api/v1/auth/confirm-opaque-") || route.contains("/api/v1/auth/cli-");
        if !approval {
            // The sign-in, the unlock and everything else: the account's own fake.
            return rest(request);
        }
        if request.bearer() != Some(ours.session_token.as_str()) {
            return Reply::error(401, "unauthorized");
        }
        let account = &ours.opaque;
        let body = request.json();
        let mode = MODE.load(Ordering::SeqCst);
        match route.as_str() {
            "GET /api/v1/auth/cli-pubkey" if request.query("code") == Some(CODE) => {
                Reply::ok(json!({ "ecdh_public_key_b64": public.as_str(), "request": facts() }))
            }
            "POST /api/v1/auth/confirm-opaque-start" => account.login_start(&body),
            "POST /api/v1/auth/confirm-opaque-finish" => {
                // Bound to this code, for a device approval: the only grant the claim takes.
                let bound = body["purpose"] == "cli_device_approval" && body["cli_code"] == CODE;
                if !(account.login_finish(&body) && bound) {
                    return Reply::error(401, "invalid_credentials");
                }
                if let Some(engine) = LOCK_AT_FINISH.lock().unwrap().take() {
                    engine.lock_screen().expect("the screen locks");
                }
                Reply::ok(json!({ "confirmation_token": tokens.mint(), "expires_at": "soon" }))
            }
            "POST /api/v1/auth/cli-session" if body["user_code"] == CODE && tokens.spend(request) => {
                if let Some(engine) = LOCK_AT_CLAIM.lock().unwrap().take() {
                    engine.lock_screen().expect("the screen locks");
                }
                if mode == CLAIM_LOST {
                    return Reply::hang_up();
                }
                Reply::ok(json!({ "session_token": NEW_DEVICE_TOKEN }))
            }
            "POST /api/v1/auth/cli-session" => Reply::error(403, "step_up_required"),
            "POST /api/v1/auth/cli-authorize" if body["user_code"] == CODE => match mode {
                RELAY_LOST => Reply::hang_up(),
                RELAY_FAILS => Reply::error(500, "internal"),
                _ => Reply::ok(json!({ "ok": true })),
            },
            "POST /api/v1/auth/cli-deny" if body["user_code"] == CODE => Reply::ok(json!({ "denied": true })),
            _ => Reply::error(404, "not_found"),
        }
    }));
    Fake {
        api,
        account,
        phrase,
        requester,
    }
}

/// A handle of its own on the fake, signed in and unlocked: the session in memory holds the account's master key.
fn signed_in(fake: &Fake) -> (Arc<TestEngine>, SessionGeneration) {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "an-invented-approval-slot".into(),
        state_dir: crate::state_paths::init_for_test(),
        api_base: fake.api.base.clone(),
    };
    let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the handle opens");
    engine.restore();
    engine
        .sign_in_password(&fake.account.email, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(fake.phrase.clone()))
        .expect("the phrase unlocks");
    let turn = engine.session_turn();
    assert!(engine.has_session(), "signed in and unlocked");
    (Arc::new(engine), turn)
}

fn password(text: &str) -> Zeroizing<Vec<u8>> {
    Zeroizing::new(text.as_bytes().to_vec())
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|window| window == needle)
}

/// The approval's own routes, in order (the sign-in's are the account fake's).
fn routes(fake: &Fake) -> Vec<String> {
    fake.api
        .routes()
        .into_iter()
        .filter(|route| route.starts_with("POST /api/v1/auth/confirm-opaque-") || route.contains("/api/v1/auth/cli-"))
        .collect()
}

fn asked(fake: &Fake, route: &str) -> usize {
    routes(fake).iter().filter(|r| r.as_str() == route).count()
}

/// The tests share the loopback mode flag.
fn serial() -> std::sync::MutexGuard<'static, ()> {
    static SERIAL: Mutex<()> = Mutex::new(());
    let guard = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    MODE.store(ANSWERS, Ordering::SeqCst);
    *LOCK_AT_FINISH.lock().unwrap() = None;
    *LOCK_AT_CLAIM.lock().unwrap() = None;
    guard
}

/// Rule 1 of slice A5: the master key leaves this PC only sealed to the other device's key. Every request that left
/// is searched for the key's bytes and its base64; the relayed payload is opened with the requester's own key and
/// holds the key and the new device's session.
#[test]
fn approve_sends_the_master_key_only_sealed() {
    let _one = serial();
    let fake = server();
    let (engine, turn) = signed_in(&fake);

    // Typed as a person types it: lower case, a space for the hyphen.
    let found = engine
        .find_sign_in_request_for(turn, "k7qm 4wtd")
        .expect("the code's request is found");
    assert!(found.code_matches(CODE), "core's rule: the shapes of one code match");
    assert!(!found.code_matches("K7QM-4WTE"));
    let facts = found.facts();
    assert_eq!(facts.country.as_deref(), Some("DE"));
    assert_eq!(facts.address, "203.0.113.47");
    assert_eq!(facts.device_name.as_deref(), Some("An invented phone"));
    assert_eq!(
        format!("{facts:?}"),
        "RequestFacts { .. }",
        "printed without its values"
    );
    assert!(found.seconds_left(0).is_some_and(|left| left > 0));

    engine
        .approve_sign_in_for(turn, found, CODE, password(PASSWORD))
        .expect("the approval goes through");

    let master_key = fake.account.master_key;
    let key_b64 = b64().encode(master_key);
    for request in fake.api.requests() {
        let route = request.route();
        assert!(!contains(&request.body, &master_key), "the key's bytes left in {route}");
        assert!(
            !contains(&request.body, key_b64.as_bytes()),
            "the key's base64 left in {route}"
        );
        assert!(
            !contains(request.path.as_bytes(), key_b64.as_bytes()),
            "the key in the path of {route}"
        );
        assert!(
            !contains(&request.body, PASSWORD.as_bytes()),
            "the password left in {route}"
        );
    }

    // The other device opens the relay with its own key, and finds its keys inside.
    let relays: Vec<Request> = fake
        .api
        .requests()
        .into_iter()
        .filter(|r| r.route() == "POST /api/v1/auth/cli-authorize")
        .collect();
    assert_eq!(relays.len(), 1, "one relay");
    let body: Value = serde_json::from_slice(&relays[0].body).expect("the relay is JSON");
    let field = |name: &str| b64().decode(body[name].as_str().expect(name)).expect(name);
    // Decoded, no field of the relay holds the key either (a plain payload in base64 would).
    for name in ["encrypted_payload_b64", "nonce_b64", "browser_ecdh_public_b64"] {
        let decoded = field(name);
        assert!(!contains(&decoded, &master_key), "the key's bytes inside {name}");
        assert!(
            !contains(&decoded, key_b64.as_bytes()),
            "the key's base64 inside {name}"
        );
    }
    let plain = fake
        .requester
        .decrypt_browser_payload(
            &field("browser_ecdh_public_b64"),
            &field("nonce_b64"),
            &field("encrypted_payload_b64"),
        )
        .expect("sealed to the requesting device's key");
    let opened: Value = serde_json::from_slice(&plain).expect("the payload is JSON");
    assert_eq!(opened["master_key_b64"], key_b64.as_str(), "the session's own key");
    assert_eq!(
        opened["session_token"], NEW_DEVICE_TOKEN,
        "the session the claim minted"
    );

    // Core's order: the lookup, the step-up's two legs, the claim, the relay.
    assert_eq!(
        routes(&fake),
        [
            "GET /api/v1/auth/cli-pubkey",
            "POST /api/v1/auth/confirm-opaque-start",
            "POST /api/v1/auth/confirm-opaque-finish",
            "POST /api/v1/auth/cli-session",
            "POST /api/v1/auth/cli-authorize",
        ]
    );
}

#[test]
fn an_unknown_code_is_not_found_and_a_malformed_one_sends_nothing() {
    let _one = serial();
    let fake = server();
    let (engine, turn) = signed_in(&fake);
    assert!(matches!(
        engine.find_sign_in_request_for(turn, "ZZZZ-9999"),
        Err(ApproveError::NotFound)
    ));
    assert_eq!(asked(&fake, "GET /api/v1/auth/cli-pubkey"), 1);
    assert!(matches!(
        engine.find_sign_in_request_for(turn, "K7QM"),
        Err(ApproveError::NotFound)
    ));
    assert_eq!(
        asked(&fake, "GET /api/v1/auth/cli-pubkey"),
        1,
        "a malformed code is not sent"
    );
}

#[test]
fn a_wrong_password_claims_nothing() {
    let _one = serial();
    let fake = server();
    let (engine, turn) = signed_in(&fake);
    let found = engine.find_sign_in_request_for(turn, CODE).unwrap();
    assert_eq!(
        engine.approve_sign_in_for(turn, found, CODE, password("not the account's password")),
        Err(ApproveError::WrongPassword)
    );
    let found = engine.find_sign_in_request_for(turn, CODE).unwrap();
    assert_eq!(
        engine.approve_sign_in_for(turn, found, CODE, password("")),
        Err(ApproveError::WrongPassword),
        "an empty password sends nothing"
    );
    assert_eq!(asked(&fake, "POST /api/v1/auth/cli-session"), 0);
    assert_eq!(asked(&fake, "POST /api/v1/auth/cli-authorize"), 0);
}

/// The master key is read from the session of the press only: a locked screen, or a session turn that moved on,
/// sends nothing; and a lock that comes during the step-up stops the claim.
#[test]
fn a_lock_or_another_session_claims_nothing() {
    let _one = serial();
    let fake = server();
    let (engine, turn) = signed_in(&fake);

    // The screen locks between the review and the approval (the app lock).
    let found = engine.find_sign_in_request_for(turn, CODE).unwrap();
    engine.lock_screen().expect("the screen locks");
    let locked = engine.session_turn();
    assert_ne!(locked, turn, "a lock is another turn of the session");
    assert_eq!(engine.sign_in_approval_allowed_for(locked), Err(ApproveError::Locked));
    assert_eq!(
        engine.approve_sign_in_for(locked, found, CODE, password(PASSWORD)),
        Err(ApproveError::Locked)
    );
    assert_eq!(
        asked(&fake, "POST /api/v1/auth/confirm-opaque-start"),
        0,
        "nothing was sent"
    );

    // Opened again: the press made before the lock is for a turn that is over.
    engine
        .unlock_screen_platform_verified(locked)
        .expect("the screen opens");
    let open = engine.session_turn();
    assert_ne!(open, turn);
    let found = engine.find_sign_in_request_for(open, CODE).unwrap();
    assert_eq!(
        engine.approve_sign_in_for(turn, found, CODE, password(PASSWORD)),
        Err(ApproveError::NotSignedIn)
    );
    assert_eq!(
        asked(&fake, "POST /api/v1/auth/confirm-opaque-start"),
        0,
        "nothing was sent"
    );

    // The screen locks while the step-up is on its way: the claim does not leave.
    assert_eq!(engine.sign_in_approval_allowed_for(open), Ok(()));
    let found = engine.find_sign_in_request_for(open, CODE).unwrap();
    *LOCK_AT_FINISH.lock().unwrap() = Some(engine.clone());
    let refused = engine.approve_sign_in_for(open, found, CODE, password(PASSWORD));
    assert!(
        matches!(refused, Err(ApproveError::Locked | ApproveError::NotSignedIn)),
        "{refused:?}"
    );
    assert_eq!(
        asked(&fake, "POST /api/v1/auth/confirm-opaque-finish"),
        1,
        "the step-up was made"
    );
    assert_eq!(
        asked(&fake, "POST /api/v1/auth/cli-session"),
        0,
        "and nothing was claimed"
    );
}

/// The relay is the point of no return, so the session of the press is asked again right before it leaves: a lock
/// that lands during the claim's round trip (Lock, Win+L, closing to the tray under "Immediately") relays nothing.
/// The claim was taken (it minted a session nobody can use: the other device only gets its token inside the relay),
/// and the keys stayed on this PC.
#[test]
fn a_lock_during_the_claim_relays_nothing() {
    let _one = serial();
    let fake = server();
    let (engine, turn) = signed_in(&fake);
    let found = engine.find_sign_in_request_for(turn, CODE).unwrap();
    *LOCK_AT_CLAIM.lock().unwrap() = Some(engine.clone());
    let refused = engine.approve_sign_in_for(turn, found, CODE, password(PASSWORD));
    assert!(
        matches!(refused, Err(ApproveError::Locked | ApproveError::NotSignedIn)),
        "{refused:?}"
    );
    assert_eq!(asked(&fake, "POST /api/v1/auth/cli-session"), 1, "the claim was taken");
    assert_eq!(
        asked(&fake, "POST /api/v1/auth/cli-authorize"),
        0,
        "and the keys did not leave"
    );
}

/// Until the relay leaves, the keys have not left this PC: a claim whose answer was lost approved nothing. Once the
/// relay left, no answer or a server error is "not known", never "nothing was approved".
#[test]
fn only_a_relay_that_left_can_be_unknown() {
    let _one = serial();
    let fake = server();
    let (engine, turn) = signed_in(&fake);

    MODE.store(CLAIM_LOST, Ordering::SeqCst);
    let found = engine.find_sign_in_request_for(turn, CODE).unwrap();
    assert_eq!(
        engine.approve_sign_in_for(turn, found, CODE, password(PASSWORD)),
        Err(ApproveError::Offline)
    );
    assert_eq!(asked(&fake, "POST /api/v1/auth/cli-authorize"), 0, "no keys left");

    for mode in [RELAY_LOST, RELAY_FAILS] {
        MODE.store(mode, Ordering::SeqCst);
        let found = engine.find_sign_in_request_for(turn, CODE).unwrap();
        assert_eq!(
            engine.approve_sign_in_for(turn, found, CODE, password(PASSWORD)),
            Err(ApproveError::Unknown),
            "mode {mode}"
        );
    }
}

/// The denial carries the code and nothing else: no step-up, no token in its body, no key.
#[test]
fn a_denial_sends_the_code_alone() {
    let _one = serial();
    let fake = server();
    let (engine, turn) = signed_in(&fake);
    let found = engine.find_sign_in_request_for(turn, CODE).unwrap();
    let handle = found.deny_handle();
    drop(found);
    assert_eq!(engine.deny_sign_in_for(turn, &handle), Ok(()));
    let denials: Vec<Request> = fake
        .api
        .requests()
        .into_iter()
        .filter(|r| r.route() == "POST /api/v1/auth/cli-deny")
        .collect();
    assert_eq!(denials.len(), 1);
    assert_eq!(denials[0].json(), json!({ "user_code": CODE }));
    assert_eq!(
        denials[0].header("x-confirm-token"),
        None,
        "denying grants nothing: no step-up"
    );
    assert_eq!(asked(&fake, "POST /api/v1/auth/confirm-opaque-start"), 0);
}
