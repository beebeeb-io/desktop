//! Changing the password at the engine, against the local fake API: the policy, the breach answer, core's OPAQUE
//! step-up and re-registration (the real client and server sides), and what this PC keeps afterwards. Every
//! name, address and password is invented; the fake is on loopback.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use base64::Engine as _;
use beebeeb_core::onboarding::BreachQuery;
use beebeeb_core::opaque_protocol as opaque;
use serde_json::json;

use super::*;
use crate::account::SessionTransition;
use crate::app::EngineConfig;
use crate::host::ChannelHost;
use crate::identity::REAL;
use crate::keychain::SecretBytes;
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;
use crate::test_api::{ConfirmTokens, FakeApi, OpaqueAccount, Reply, Request};

type TestEngine = Engine<MemoryCredentials>;

const PASSWORD: &str = "correct horse battery staple";
const NEW_PASSWORD: &str = "A Different Staple 42 Battery";
const OLD_TOKEN: &str = "session-before-the-change";
const NEW_TOKEN: &str = "session-after-the-change";
const OWNER: &str = "11111111-1111-4111-8111-111111111111";

const ANSWERS: u8 = 0;
const SESSION_ENDED: u8 = 1;
const BREACH_LISTS_NEW: u8 = 2;
static MODE: AtomicU8 = AtomicU8::new(ANSWERS);

fn b64() -> base64::engine::general_purpose::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

/// One registered account for every test here (a registration runs Argon2id once).
fn account() -> Arc<OpaqueAccount> {
    static ACCOUNT: OnceLock<Arc<OpaqueAccount>> = OnceLock::new();
    ACCOUNT
        .get_or_init(|| Arc::new(OpaqueAccount::register("password-change@example.test", PASSWORD)))
        .clone()
}

/// The fake server. `taken` is what the change's finish carried: (the recovery check was sent, the public key was).
fn server(mode: &'static AtomicU8) -> (FakeApi, Arc<Mutex<Vec<(bool, bool)>>>) {
    let account = account();
    let tokens = Arc::new(ConfirmTokens::default());
    let setup = opaque::create_server_setup();
    let taken = Arc::new(Mutex::new(Vec::new()));
    let taken_in = taken.clone();
    let listed = format!("{}:7\n", BreachQuery::from_password(NEW_PASSWORD).suffix());
    let api = FakeApi::start(Arc::new(move |request: &Request| {
        let route = request.route();
        let body = request.json();
        let breach_declared = mode.load(Ordering::SeqCst) == BREACH_LISTS_NEW;
        if route == "GET /api/v1/onboarding" {
            let mut policy = json!({ "password": { "min_length": 12 } });
            if breach_declared {
                policy["password"]["breach_check"] =
                    json!({ "endpoint": "/api/v1/auth/pwned-range/{prefix}", "fail_open": false });
            }
            return Reply::ok(json!({ "policy": policy }));
        }
        if route.starts_with("GET /api/v1/auth/pwned-range/") {
            return Reply {
                status: 200,
                content_type: "text/plain",
                body: listed.clone().into_bytes(),
            };
        }
        if request.bearer() != Some(OLD_TOKEN) || mode.load(Ordering::SeqCst) == SESSION_ENDED {
            return Reply::error(401, "unauthorized");
        }
        match route.as_str() {
            "POST /api/v1/auth/confirm-opaque-start" => account.login_start(&body),
            "POST /api/v1/auth/confirm-opaque-finish" => {
                if account.login_finish(&body) {
                    Reply::ok(json!({ "confirmation_token": tokens.mint(), "expires_at": "soon" }))
                } else {
                    Reply::error(401, "invalid_credentials")
                }
            }
            "POST /api/v1/auth/change-password-start" if tokens.spend(request) => {
                let message = body["client_message"]
                    .as_str()
                    .and_then(|m| b64().decode(m).ok())
                    .unwrap_or_default();
                match opaque::server_registration_start(&setup, &message, b"password-change@example.test") {
                    Ok(reply) => Reply::ok(json!({ "server_message": b64().encode(reply) })),
                    Err(_) => Reply::error(400, "bad_request"),
                }
            }
            "POST /api/v1/auth/change-password-finish" if tokens.spend(request) => {
                let upload = body["client_message"]
                    .as_str()
                    .and_then(|m| b64().decode(m).ok())
                    .unwrap_or_default();
                if opaque::server_registration_finish(&upload).is_err() {
                    return Reply::error(400, "bad_request");
                }
                taken_in.lock().unwrap().push((
                    body["recovery_check"].is_string(),
                    body["x25519_public_key"].is_string(),
                ));
                Reply::ok(json!({ "session_token": NEW_TOKEN }))
            }
            "POST /api/v1/auth/change-password-start" | "POST /api/v1/auth/change-password-finish" => {
                Reply::error(403, "step_up_required")
            }
            _ => Reply::error(404, "not_found"),
        }
    }));
    (api, taken)
}

fn engine(base: &str) -> Arc<TestEngine> {
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "an-invented-slot".into(),
        state_dir,
        api_base: base.to_string(),
    };
    Arc::new(Engine::open(&REAL, config, host, MemoryCredentials::default()).expect("the handle opens"))
}

/// Signed in and unlocked: the old token and a key in the credential store, with the account's owner tag, and the
/// session in memory.
fn signed_in(engine: &TestEngine) -> SessionGeneration {
    {
        let _writes = engine.writes.write().unwrap();
        let vault = engine.vault();
        vault.install_session(SessionToken::new(OLD_TOKEN).unwrap()).unwrap();
        vault.bind_session_owner(OWNER).unwrap();
        vault
            .store_wrapped_master_key(SecretBytes::new(vec![7; 32]).unwrap())
            .unwrap();
        vault.bind_vault_key_owner(OWNER).unwrap();
        engine
            .acct
            .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
        *engine.acct.session.lock().unwrap() = Some(Session {
            token: OLD_TOKEN.into(),
            master_key: [7; 32],
            email: Some("a@example.eu".into()),
        });
        engine.set_auth_present(true);
    }
    engine.session_turn()
}

fn secret(text: &str) -> Zeroizing<String> {
    Zeroizing::new(text.to_string())
}

fn stored_token(engine: &TestEngine) -> String {
    engine
        .vault()
        .session_token()
        .unwrap()
        .unwrap()
        .expose_for_request()
        .to_string()
}

fn memory_token(engine: &TestEngine) -> String {
    engine.acct.session.lock().unwrap().as_ref().unwrap().token.clone()
}

fn routes_of(api: &FakeApi, part: &str) -> usize {
    api.routes().iter().filter(|route| route.contains(part)).count()
}

/// The tests share the loopback mode flag.
fn serial() -> std::sync::MutexGuard<'static, ()> {
    static SERIAL: Mutex<()> = Mutex::new(());
    let guard = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    MODE.store(ANSWERS, Ordering::SeqCst);
    guard
}

#[test]
fn a_changed_password_stores_the_new_session_and_keeps_the_key() {
    let _one = serial();
    let (api, taken) = server(&MODE);
    let engine = engine(&api.base);
    let turn = signed_in(&engine);

    engine
        .change_password_for(turn, secret(PASSWORD), secret(NEW_PASSWORD))
        .expect("the change goes through");

    // The token was replaced in the credential store and in memory; the owner tag is beside the new one.
    assert_eq!(stored_token(&engine), NEW_TOKEN);
    assert_eq!(memory_token(&engine), NEW_TOKEN);
    assert_eq!(engine.vault().session_owner().unwrap().as_deref(), Some(OWNER));
    // The key and its tag are as they were: the password wraps nothing.
    assert!(engine.vault().has_vault_key().unwrap());
    assert_eq!(engine.vault().vault_key_owner().unwrap().as_deref(), Some(OWNER));
    assert_eq!(
        engine.acct.session.lock().unwrap().as_ref().unwrap().master_key,
        [7; 32]
    );
    // The session moved on: an answer made for the old one is dropped by its host.
    assert_ne!(engine.session_turn(), turn);
    // Core's flow: a step-up for each leg (2 requests each) and the two re-registration legs; the binding went.
    assert_eq!(routes_of(&api, "/auth/confirm-opaque-"), 4);
    assert_eq!(routes_of(&api, "/auth/change-password-"), 2);
    assert_eq!(*taken.lock().unwrap(), vec![(true, true)]);
}

#[test]
fn a_wrong_current_password_changes_nothing() {
    let _one = serial();
    let (api, taken) = server(&MODE);
    let engine = engine(&api.base);
    let turn = signed_in(&engine);

    let result = engine.change_password_for(turn, secret("not the password at all"), secret(NEW_PASSWORD));

    assert_eq!(result, Err(PasswordChangeError::WrongPassword));
    assert_eq!(stored_token(&engine), OLD_TOKEN);
    assert_eq!(memory_token(&engine), OLD_TOKEN);
    assert_eq!(routes_of(&api, "/auth/change-password-"), 0);
    assert!(taken.lock().unwrap().is_empty());
    assert_eq!(engine.session_turn(), turn, "the session did not move");
}

#[test]
fn a_password_core_refuses_is_refused_before_anything_is_sent() {
    let _one = serial();
    let (api, _taken) = server(&MODE);
    let engine = engine(&api.base);
    let turn = signed_in(&engine);

    assert_eq!(
        engine.change_password_for(turn, secret(PASSWORD), secret(PASSWORD)),
        Err(PasswordChangeError::SamePassword)
    );
    assert_eq!(
        engine.change_password_for(turn, secret(PASSWORD), secret("short")),
        Err(PasswordChangeError::TooWeak)
    );
    assert!(api.routes().is_empty(), "no request went out: {:?}", api.routes());
    assert_eq!(stored_token(&engine), OLD_TOKEN);
}

#[test]
fn a_password_in_a_known_breach_is_refused_before_the_proof() {
    let _one = serial();
    MODE.store(BREACH_LISTS_NEW, Ordering::SeqCst);
    let (api, taken) = server(&MODE);
    let engine = engine(&api.base);
    let turn = signed_in(&engine);

    let result = engine.change_password_for(turn, secret(PASSWORD), secret(NEW_PASSWORD));

    assert_eq!(result, Err(PasswordChangeError::Breached));
    // Only the first five characters of the hash went out, and no step-up was asked for.
    assert_eq!(routes_of(&api, "/auth/pwned-range/"), 1);
    assert_eq!(routes_of(&api, "/auth/confirm-opaque-"), 0);
    assert!(taken.lock().unwrap().is_empty());
    assert_eq!(stored_token(&engine), OLD_TOKEN);
}

#[test]
fn a_locked_vault_a_signed_out_handle_and_another_session_send_nothing() {
    let _one = serial();
    let (api, _taken) = server(&MODE);
    let engine = engine(&api.base);
    let turn = signed_in(&engine);

    // Another session than the one the press was made for.
    {
        let _writes = engine.writes.write().unwrap();
        engine
            .acct
            .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
    }
    assert_eq!(
        engine.change_password_for(turn, secret(PASSWORD), secret(NEW_PASSWORD)),
        Err(PasswordChangeError::NotSignedIn)
    );
    // Locked: a token is stored, the key is not in memory.
    *engine.acct.session.lock().unwrap() = None;
    assert_eq!(
        engine.change_password_for(engine.session_turn(), secret(PASSWORD), secret(NEW_PASSWORD)),
        Err(PasswordChangeError::Locked)
    );
    // Signed out.
    engine.set_auth_present(false);
    assert_eq!(
        engine.change_password_for(engine.session_turn(), secret(PASSWORD), secret(NEW_PASSWORD)),
        Err(PasswordChangeError::NotSignedIn)
    );
    assert_eq!(routes_of(&api, "/auth/"), 0);
}

#[test]
fn a_session_beebeeb_no_longer_accepts_stores_no_token() {
    let _one = serial();
    MODE.store(SESSION_ENDED, Ordering::SeqCst);
    let (api, taken) = server(&MODE);
    let engine = engine(&api.base);
    let turn = signed_in(&engine);

    let result = engine.change_password_for(turn, secret(PASSWORD), secret(NEW_PASSWORD));

    assert_eq!(result, Err(PasswordChangeError::SessionEnded));
    assert_eq!(stored_token(&engine), OLD_TOKEN);
    assert!(taken.lock().unwrap().is_empty());
}

#[test]
fn the_strength_is_cores_and_needs_no_network() {
    assert_eq!(password_strength(""), PasswordStrength::TooShort);
    assert_eq!(password_strength("short"), PasswordStrength::TooShort);
    assert_eq!(password_strength("alllowercaseletters"), PasswordStrength::Fair);
    assert_eq!(password_strength("MixedCaseLettersOnly"), PasswordStrength::Good);
    assert_eq!(password_strength("Mixed Case Letters 42"), PasswordStrength::Strong);
}
