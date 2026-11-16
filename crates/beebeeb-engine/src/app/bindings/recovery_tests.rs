//! The in-app recovery with the twelve words (slice K6), on the engine's fake API: one invented account, and the
//! server's three recovery routes answered here in the shape `routes/recovery.rs` answers them (the start checks
//! the recovery check, the register runs the server side of OPAQUE's registration, the finalize answers a session
//! or "sign in normally"). Every other route goes to the fake account. Every address, password, word and token is
//! invented.
//!
//! The rules: an unknown word sends nothing; the words never leave the computer; nothing is stored for
//! "sign in normally" or when the session moved meanwhile; the held recovery is dropped when it is forgotten and
//! when the session changes.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use beebeeb_core::opaque_protocol as opaque;
use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::{RecoveryDone, RecoveryRefused};
use crate::app::{Engine, EngineConfig};
use crate::host::ChannelHost;
use crate::identity::TEST;
use crate::keychain::AuthVault;
use crate::signin::gate::GateState;
use crate::signin::memory_store::MemoryCredentials;
use crate::test_api::fake_account::{FakeAccount, account_handler};
use crate::test_api::{FakeApi, Handler, Reply, Request};

type TestEngine = Engine<MemoryCredentials>;

/// A valid phrase (BIP-39's second test vector) with words long enough to never be part of an address or a route.
const PHRASE: &str = "legal winner thank year wave sausage worth useful legal winner thank yellow";
const OLD_PASSWORD: &str = "an invented password that was forgotten";
const NEW_PASSWORD: &str = "An invented new password, 2026";
/// The server's minimum in this fake's onboarding document: not core's default, so the meter shows it was read.
const MIN_LENGTH: u32 = 14;
const RECOVERY_TOKEN: &str = "an-invented-recovery-token";

const ONBOARDING: &str = "GET /api/v1/onboarding";
const START: &str = "POST /api/v1/auth/recover-with-phrase-start";
const REGISTER: &str = "POST /api/v1/auth/recover-opaque-register";
const FINALIZE: &str = "POST /api/v1/auth/recover-with-phrase-finalize";
const PROFILE: &str = "GET /api/v1/auth/me";
const LOGOUT: &str = "POST /api/v1/auth/logout";

/// How the finalize answers.
const SIGNS_IN: u8 = 0;
/// `{requires_2fa: true}`, with a session token beside it: a client that stores any token it sees is caught.
const TWO_FACTOR: u8 = 1;
/// The password step's registration refuses the recovery token (`invalid_recovery_token`): the recovery is over.
const REFUSES_TOKEN: u8 = 2;
/// The finalize gets no answer: the server hangs up after it has the request.
const HANGS_UP: u8 = 3;

type Meanwhile = Box<dyn FnOnce() + Send>;

/// The fake, its account, and how the finalize answers.
struct Recovery {
    api: FakeApi,
    account: Arc<FakeAccount>,
    finalize: Arc<AtomicU8>,
    /// Run once while the finalize is being answered (the server has the request; the client waits for its answer).
    during_finalize: Arc<Mutex<Option<Meanwhile>>>,
}

fn b64() -> base64::engine::general_purpose::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

fn server(tag: &str) -> Recovery {
    let (account, _) = FakeAccount::register(&format!("recovery-{tag}@example.eu"), OLD_PASSWORD, PHRASE);
    let account = Arc::new(account);
    let rest = account_handler(account.clone());
    let setup = opaque::create_server_setup();
    let finalize = Arc::new(AtomicU8::new(SIGNS_IN));
    let during_finalize: Arc<Mutex<Option<Meanwhile>>> = Arc::new(Mutex::new(None));
    let (account_in, finalize_in, during_in) = (account.clone(), finalize.clone(), during_finalize.clone());
    let handler: Handler = Arc::new(move |req: &Request| {
        let body = req.json();
        let token_ok = body["recovery_token"] == RECOVERY_TOKEN;
        let check_ok = body["recovery_check"] == account_in.recovery_check.as_str();
        match req.route().as_str() {
            ONBOARDING => Reply::ok(json!({ "policy": { "password": { "min_length": MIN_LENGTH } } })),
            START => {
                let email_ok = body["email"]
                    .as_str()
                    .is_some_and(|email| email.eq_ignore_ascii_case(&account_in.email));
                if email_ok && check_ok {
                    Reply::ok(json!({ "recovery_token": RECOVERY_TOKEN }))
                } else {
                    Reply::error(400, "invalid_recovery_phrase")
                }
            }
            REGISTER => {
                if finalize_in.load(Ordering::SeqCst) == REFUSES_TOKEN {
                    return Reply::error(400, "invalid_recovery_token");
                }
                let message = body["client_message"]
                    .as_str()
                    .and_then(|message| b64().decode(message).ok())
                    .filter(|_| token_ok);
                let answer = message.and_then(|message| {
                    opaque::server_registration_start(&setup, &message, account_in.email.as_bytes()).ok()
                });
                match answer {
                    Some(answer) => Reply::ok(json!({ "server_message": b64().encode(answer) })),
                    None => Reply::error(400, "invalid_recovery_token"),
                }
            }
            FINALIZE => {
                let complete = body["opaque_registration"].is_string() && body["x25519_public_key"].is_string();
                if !(token_ok && check_ok && complete) {
                    return Reply::error(400, "invalid_recovery_token");
                }
                let meanwhile = during_in.lock().expect("the finalize hook").take();
                if let Some(meanwhile) = meanwhile {
                    meanwhile();
                }
                match finalize_in.load(Ordering::SeqCst) {
                    HANGS_UP => Reply::hang_up(),
                    TWO_FACTOR => Reply::ok(json!({
                        "requires_2fa": true,
                        "session_token": account_in.session_token,
                    })),
                    _ => Reply::ok(json!({ "session_token": account_in.session_token })),
                }
            }
            _ => rest(req),
        }
    });
    Recovery {
        api: FakeApi::start(handler),
        account,
        finalize,
        during_finalize,
    }
}

/// A signed-out engine of its own slot on `api`, and the credential store it writes.
fn opened(api: &FakeApi, slot: &str) -> (Arc<TestEngine>, MemoryCredentials) {
    let store = MemoryCredentials::default();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: format!("recovery-{slot}"),
        state_dir: crate::state_paths::init_for_test(),
        api_base: api.base.clone(),
    };
    let engine = Engine::open(&TEST, config, host, store.clone()).expect("the engine opens");
    engine.restore();
    (Arc::new(engine), store)
}

/// The session token the credential store holds for the slot, if any.
fn stored_token(store: &MemoryCredentials, slot: &str) -> Option<String> {
    AuthVault::new(store.store_for(&format!("recovery-{slot}")))
        .session_token()
        .ok()
        .flatten()
        .map(|token| token.expose_for_request().to_string())
}

/// Whether the credential store holds a vault key for the slot.
fn stored_key(store: &MemoryCredentials, slot: &str) -> bool {
    matches!(
        AuthVault::new(store.store_for(&format!("recovery-{slot}"))).has_vault_key(),
        Ok(true)
    )
}

fn count(api: &FakeApi, route: &str) -> usize {
    api.routes().iter().filter(|seen| seen.as_str() == route).count()
}

fn phrase() -> Zeroizing<String> {
    Zeroizing::new(PHRASE.to_string())
}

fn new_password() -> Zeroizing<String> {
    Zeroizing::new(NEW_PASSWORD.to_string())
}

/// The words verified for the account, at the session turn of now.
fn verified(engine: &TestEngine, fake: &Recovery) {
    assert_eq!(
        engine.recovery_verify_for(engine.session_turn(), &fake.account.email, phrase()),
        Ok(()),
        "the account's own words are accepted"
    );
}

/// Whether this engine holds a recovery in the slot (taken out or not).
fn holds_a_recovery(engine: &TestEngine) -> bool {
    super::held().iter().any(|entry| engine.owns_recovery(entry))
}

/// Every string in a JSON value, at any depth (keys included).
fn strings_of(value: &Value, into: &mut Vec<String>) {
    match value {
        Value::String(text) => into.push(text.clone()),
        Value::Array(items) => items.iter().for_each(|item| strings_of(item, into)),
        Value::Object(fields) => {
            for (key, item) in fields {
                into.push(key.clone());
                strings_of(item, into);
            }
        }
        _ => {}
    }
}

#[test]
fn recovery_unknown_word_sends_nothing() {
    let fake = server("unknown");
    let (engine, store) = opened(&fake.api, "unknown");
    let mut words: Vec<&str> = PHRASE.split(' ').collect();
    words[4] = "notaword";
    words[9] = "zzzz";
    let typed = Zeroizing::new(words.join(" "));

    let answer = engine.recovery_verify_for(engine.session_turn(), &fake.account.email, typed);

    assert_eq!(answer, Err(RecoveryRefused::UnknownWords(vec![4, 9])));
    assert!(
        fake.api.requests().is_empty(),
        "nothing is sent, not even the onboarding document: {:?}",
        fake.api.routes()
    );
    assert!(!holds_a_recovery(&engine));
    assert_eq!(stored_token(&store, "unknown"), None);
}

#[test]
fn recovery_start_body_holds_no_phrase_word() {
    let fake = server("body");
    let (engine, _store) = opened(&fake.api, "body");
    verified(&engine, &fake);

    let starts: Vec<Request> = fake
        .api
        .requests()
        .into_iter()
        .filter(|request| request.route() == START)
        .collect();
    assert_eq!(starts.len(), 1, "one start request: {:?}", fake.api.routes());
    let body = starts[0].json();
    let mut fields: Vec<&str> = body
        .as_object()
        .expect("the start body is a JSON object")
        .keys()
        .map(String::as_str)
        .collect();
    fields.sort_unstable();
    assert_eq!(
        fields,
        ["email", "recovery_check"],
        "the address and the check, nothing else"
    );
    assert_eq!(
        body["recovery_check"],
        fake.account.recovery_check.as_str(),
        "the check derived from the key"
    );
    let check = b64()
        .decode(body["recovery_check"].as_str().expect("a base64 check"))
        .expect("base64");
    assert_eq!(check.len(), 32);

    // No request at all carries the phrase or one of its words, in any field or in its path (read as parsed
    // fields and whole tokens, never as a search in base64 text).
    let words: Vec<&str> = PHRASE.split(' ').collect();
    for request in fake.api.requests() {
        let mut texts = vec![request.path.clone()];
        strings_of(&request.json(), &mut texts);
        for text in &texts {
            assert!(!text.contains(PHRASE), "{} carries the phrase", request.route());
            for token in text.split(|c: char| !c.is_ascii_alphabetic()) {
                assert!(
                    !words.contains(&token),
                    "{} carries a word of the phrase",
                    request.route()
                );
            }
        }
    }
}

#[test]
fn recovery_signed_in_stores_the_session_once() {
    let fake = server("signed-in");
    let (engine, store) = opened(&fake.api, "signed-in");
    verified(&engine, &fake);

    let done = engine.recovery_set_password_for(engine.session_turn(), new_password());

    assert_eq!(done, Ok(RecoveryDone::SignedIn));
    assert!(engine.has_session(), "the key is in memory");
    assert_eq!(engine.gate(), GateState::Ready);
    assert_eq!(
        stored_token(&store, "signed-in").as_deref(),
        Some(fake.account.session_token.as_str())
    );
    assert!(stored_key(&store, "signed-in"), "the account key is stored");
    for (route, times) in [(START, 1), (REGISTER, 1), (FINALIZE, 1), (PROFILE, 1), (LOGOUT, 0)] {
        assert_eq!(count(&fake.api, route), times, "{route}: {:?}", fake.api.routes());
    }
    assert!(!holds_a_recovery(&engine), "the last step consumed the recovery");

    // A second press finds no recovery: nothing is sent, nothing changes.
    assert_eq!(
        engine.recovery_set_password_for(engine.session_turn(), new_password()),
        Err(RecoveryRefused::Overtaken)
    );
    assert_eq!(count(&fake.api, REGISTER), 1);
    assert_eq!(count(&fake.api, PROFILE), 1);
    assert!(engine.has_session());
}

#[test]
fn recovery_sign_in_required_stores_nothing() {
    let fake = server("two-factor");
    fake.finalize.store(TWO_FACTOR, Ordering::SeqCst);
    let (engine, store) = opened(&fake.api, "two-factor");
    verified(&engine, &fake);

    let done = engine.recovery_set_password_for(engine.session_turn(), new_password());

    assert_eq!(done, Ok(RecoveryDone::SignInRequired));
    assert_eq!(count(&fake.api, FINALIZE), 1, "the password was set at the server");
    assert!(!engine.has_session());
    assert_eq!(engine.gate(), GateState::SignedOut);
    assert_eq!(
        stored_token(&store, "two-factor"),
        None,
        "the token beside requires_2fa is not stored"
    );
    assert!(!stored_key(&store, "two-factor"));
    assert_eq!(count(&fake.api, PROFILE), 0, "nothing was asked for a session");
    assert!(!holds_a_recovery(&engine));
}

#[test]
fn recovery_finalize_with_no_answer_holds_nothing() {
    let fake = server("no-answer");
    fake.finalize.store(HANGS_UP, Ordering::SeqCst);
    let (engine, store) = opened(&fake.api, "no-answer");
    verified(&engine, &fake);

    let done = engine.recovery_set_password_for(engine.session_turn(), new_password());

    assert_eq!(done, Err(RecoveryRefused::Offline));
    assert_eq!(count(&fake.api, FINALIZE), 1, "the last step was sent");
    assert!(
        !holds_a_recovery(&engine),
        "no answer: the recovery is over, nothing is kept for a retry"
    );
    assert!(!engine.has_session());
    assert_eq!(engine.gate(), GateState::SignedOut);
    assert_eq!(stored_token(&store, "no-answer"), None);
    assert!(!stored_key(&store, "no-answer"));
    assert_eq!(count(&fake.api, PROFILE), 0);

    // A retry has nothing to continue from: nothing is sent.
    assert_eq!(
        engine.recovery_set_password_for(engine.session_turn(), new_password()),
        Err(RecoveryRefused::Overtaken)
    );
    assert_eq!(count(&fake.api, REGISTER), 1);
    assert_eq!(count(&fake.api, FINALIZE), 1);
}

#[test]
fn recovery_overtaken_stores_nothing() {
    let fake = server("overtaken");
    let (engine, store) = opened(&fake.api, "overtaken");
    verified(&engine, &fake);
    let turn = engine.session_turn();
    // A sign-out on this computer while the server answers the last step: the press's session moved on.
    let meanwhile = engine.clone();
    *fake.during_finalize.lock().expect("the finalize hook") = Some(Box::new(move || {
        let _ = meanwhile.sign_out_keeping_local_data();
    }));

    let done = engine.recovery_set_password_for(turn, new_password());

    assert_eq!(done, Err(RecoveryRefused::Overtaken));
    assert_ne!(engine.session_turn(), turn, "the session did move");
    assert_eq!(count(&fake.api, FINALIZE), 1, "the password was set at the server");
    assert_eq!(count(&fake.api, PROFILE), 0, "nothing was asked for the new session");
    assert_eq!(count(&fake.api, LOGOUT), 1, "the session the finalize made is ended");
    assert!(!engine.has_session());
    assert_eq!(stored_token(&store, "overtaken"), None);
    assert!(!stored_key(&store, "overtaken"));
    assert_eq!(engine.gate(), GateState::SignedOut);
    assert!(!holds_a_recovery(&engine), "the last step consumed the recovery");
}

#[test]
fn recovery_forget_drops_the_held_flow() {
    let fake = server("forget");
    let (engine, _store) = opened(&fake.api, "forget");

    // Forgotten: nothing is held, and the next step sends nothing.
    verified(&engine, &fake);
    assert!(holds_a_recovery(&engine));
    engine.recovery_forget();
    assert!(!holds_a_recovery(&engine));
    assert_eq!(
        engine.recovery_set_password_for(engine.session_turn(), new_password()),
        Err(RecoveryRefused::Overtaken)
    );
    assert_eq!(count(&fake.api, REGISTER), 0);

    // A session change after the words were verified: the next step drops the recovery and sends nothing.
    verified(&engine, &fake);
    let _ = engine.sign_out_keeping_local_data();
    assert_eq!(
        engine.recovery_set_password_for(engine.session_turn(), new_password()),
        Err(RecoveryRefused::Overtaken)
    );
    assert!(!holds_a_recovery(&engine));
    assert_eq!(count(&fake.api, REGISTER), 0);
}

#[test]
fn recovery_meter_reads_the_servers_minimum_and_a_short_password_sends_nothing() {
    let fake = server("meter");
    let (engine, _store) = opened(&fake.api, "meter");
    assert_eq!(
        engine.recovery_password_verdict("short").min_length,
        12,
        "core's default before the server's numbers"
    );
    verified(&engine, &fake);
    let short = engine.recovery_password_verdict("thirteen-char");
    assert_eq!(
        (short.meets_minimum, short.min_length, short.missing),
        (false, MIN_LENGTH, 1)
    );
    assert!(engine.recovery_password_verdict(NEW_PASSWORD).meets_minimum);

    assert_eq!(
        engine.recovery_set_password_for(engine.session_turn(), Zeroizing::new("thirteen-char".to_string())),
        Err(RecoveryRefused::Weak)
    );
    assert_eq!(count(&fake.api, REGISTER), 0, "nothing is sent for a short password");
    assert!(holds_a_recovery(&engine), "the words stay verified");

    // The words stay verified: the right password goes on without them.
    assert_eq!(
        engine.recovery_set_password_for(engine.session_turn(), new_password()),
        Ok(RecoveryDone::SignedIn)
    );
    assert_eq!(count(&fake.api, START), 1);
}

// ---------------------------------------------------------------------------
// The wipe (census row, lane signin-wipe): the held recovery, which holds the key the words derived and the server's
// recovery token, ends with the flow: when the flow is left, when a sign-in replaces it, and when its engine goes.
// ---------------------------------------------------------------------------

/// Leaving a half-done sign-in is the engine's own rule for the recovery too: the held flow is forgotten by
/// `discard_partial_sign_in` itself, whichever host calls it.
#[test]
fn signin_wipe_leaving_a_half_done_sign_in_forgets_the_held_recovery() {
    let fake = server("wipe-leave");
    let (engine, _store) = opened(&fake.api, "wipe-leave");
    verified(&engine, &fake);
    assert!(holds_a_recovery(&engine), "the words were verified: the flow is held");
    engine.discard_partial_sign_in().expect("nothing is signed in to end");
    assert!(
        !holds_a_recovery(&engine),
        "the key and the recovery token go when the recovery is left"
    );
}

/// A sign-in that stores a session ends every other half-done step: a held recovery does not wait behind it.
#[test]
fn signin_wipe_a_sign_in_ends_a_held_recovery() {
    let fake = server("wipe-replaced");
    let (engine, _store) = opened(&fake.api, "wipe-replaced");
    verified(&engine, &fake);
    assert!(holds_a_recovery(&engine), "the words were verified: the flow is held");
    engine
        .sign_in_password(&fake.account.email, Zeroizing::new(OLD_PASSWORD.to_string()))
        .expect("the password still signs in");
    assert!(
        !holds_a_recovery(&engine),
        "the sign-in replaced the recovery: its key and token are gone"
    );
}

/// An engine that goes takes its held recovery with it at once, not at another engine's next look into the slot.
#[test]
fn signin_wipe_a_dropped_engine_leaves_no_held_recovery() {
    let fake = server("wipe-drop");
    let (engine, _store) = opened(&fake.api, "wipe-drop");
    verified(&engine, &fake);
    // A second hold of the account runtime keeps the entry's owner alive, so another test's look into the slot (which
    // drops the entries of dead owners) cannot pass this test for the wrong reason.
    let acct = engine.acct.clone();
    let owner = Arc::downgrade(&acct);
    assert!(super::held_for(&owner), "the words were verified: the flow is held");
    drop(engine);
    assert!(
        !super::held_for(&owner),
        "the engine went: its recovery's key and token went with it"
    );
    drop(acct);
}

/// The server refuses the recovery token at the password step: the recovery is over, and its key and token go with it.
#[test]
fn signin_wipe_a_refused_recovery_token_ends_the_held_recovery() {
    let fake = server("wipe-refused");
    let (engine, _store) = opened(&fake.api, "wipe-refused");
    verified(&engine, &fake);
    fake.finalize.store(REFUSES_TOKEN, Ordering::SeqCst);
    assert_eq!(
        engine.recovery_set_password_for(engine.session_turn(), new_password()),
        Err(RecoveryRefused::Overtaken)
    );
    assert!(!holds_a_recovery(&engine), "a refused recovery holds nothing");
}
