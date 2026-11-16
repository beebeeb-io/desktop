//! Deleting the account at the engine (slice A6), against the local fake API: core's OPAQUE step-up (the real client
//! and server sides), the request with its second factor, the state read, the signed-in cancel, and the answers that
//! must stay honest. The fake answers as Beebeeb's `routes/account_delete.rs` does: the step-up's token is consumed
//! BEFORE the code is read. Every name, address, code and password is invented; the fake is on loopback.

use std::sync::atomic::AtomicU8;
use std::sync::{Arc, Mutex, OnceLock};

use super::*;
use crate::account::SessionTransition;
use crate::app::EngineConfig;
use crate::host::ChannelHost;
use crate::identity::TEST;
use crate::keychain::{SecretBytes, SessionToken};
use crate::session::Session;
use crate::signin::memory_store::MemoryCredentials;
use crate::test_api::{ConfirmTokens, FakeApi, OpaqueAccount, Reply, Request};

type TestEngine = Engine<MemoryCredentials>;

const PASSWORD: &str = "an invented password for deleting";
const CODE: &str = "514208";
const WRONG_CODE: &str = "000000";
const TOKEN: &str = "session-of-the-deleting-account";
const OWNER: &str = "22222222-2222-4222-8222-222222222222";
const CANCEL_UNTIL: &str = "2099-12-14T18:42:00+00:00";
const SHRED_AFTER: &str = "2099-12-14T19:42:00+00:00";

const ANSWERS: u8 = 0;
/// `delete-request` answers 409 `delete_subscription_active`.
const PLAN_STILL_CHARGED: u8 = 1;
/// `delete-request` answers 409 `delete_already_scheduled`.
const ALREADY_SCHEDULED: u8 = 2;
/// `delete-cancel` spends the token and closes the connection: no answer.
const CANCEL_ANSWER_LOST: u8 = 3;

/// The fake Beebeeb, and what it saw.
struct Fake {
    api: FakeApi,
    /// `none`, `requested` or `scheduled`.
    state: Arc<Mutex<&'static str>>,
    mode: Arc<AtomicU8>,
    /// Every request body and confirm header it was sent, as text.
    seen: Arc<Mutex<Vec<String>>>,
    /// Runs once, while the next `delete-request` is answered (the call is out).
    during_request: Arc<Mutex<Option<Box<dyn FnOnce() + Send>>>>,
    /// Runs once, while the next step-up's finish is answered (the step-up is out).
    during_step_up: Arc<Mutex<Option<Box<dyn FnOnce() + Send>>>>,
}

impl Fake {
    fn asked(&self, route: &str) -> usize {
        self.api.routes().iter().filter(|r| r.as_str() == route).count()
    }
}

/// One registered account for every test here (a registration runs Argon2id once).
fn account() -> Arc<OpaqueAccount> {
    static ACCOUNT: OnceLock<Arc<OpaqueAccount>> = OnceLock::new();
    ACCOUNT
        .get_or_init(|| Arc::new(OpaqueAccount::register("deleting@example.test", PASSWORD)))
        .clone()
}

fn fake() -> Fake {
    let account = account();
    let tokens = Arc::new(ConfirmTokens::default());
    let state = Arc::new(Mutex::new("none"));
    let mode = Arc::new(AtomicU8::new(ANSWERS));
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let during_request: Arc<Mutex<Option<Box<dyn FnOnce() + Send>>>> = Arc::default();
    let during_step_up: Arc<Mutex<Option<Box<dyn FnOnce() + Send>>>> = Arc::default();
    let (state_in, mode_in, seen_in, during_in) = (state.clone(), mode.clone(), seen.clone(), during_request.clone());
    let step_up_in = during_step_up.clone();
    let api = FakeApi::start(Arc::new(move |request: &Request| {
        seen_in.lock().unwrap().push(format!(
            "{} {}",
            String::from_utf8_lossy(&request.body),
            request.header("x-confirm-token").unwrap_or_default()
        ));
        if request.bearer() != Some(TOKEN) {
            return Reply::error(401, "unauthorized");
        }
        let body = request.json();
        match request.route().as_str() {
            "POST /api/v1/auth/confirm-opaque-start" => account.login_start(&body),
            "POST /api/v1/auth/confirm-opaque-finish" => {
                if let Some(hook) = step_up_in.lock().unwrap().take() {
                    hook();
                }
                if account.login_finish(&body) {
                    Reply::ok(json!({ "confirmation_token": tokens.mint(), "expires_at": "soon" }))
                } else {
                    Reply::error(401, "invalid_credentials")
                }
            }
            "POST /api/v1/auth/account/delete-request" => {
                // As Beebeeb: the extractor consumes the token before the handler reads anything.
                if !tokens.spend(request) {
                    return Reply::error(403, "step_up_required");
                }
                if let Some(hook) = during_in.lock().unwrap().take() {
                    hook();
                }
                let refusal = |code: &str| Reply::json(409, json!({ "error": code, "code": code, "message": "words" }));
                match mode_in.load(Ordering::SeqCst) {
                    PLAN_STILL_CHARGED => return refusal("delete_subscription_active"),
                    ALREADY_SCHEDULED => return refusal("delete_already_scheduled"),
                    _ => {}
                }
                if body["code"] != json!(CODE) {
                    // As Beebeeb: a wrong second factor is 400 `invalid_totp_code`.
                    return Reply::json(400, json!({ "error": "invalid_totp_code" }));
                }
                *state_in.lock().unwrap() = "requested";
                Reply::ok(
                    json!({ "email_link_sent": true, "link_expires_at": "2099-01-01T00:00:00Z", "scheduled": false }),
                )
            }
            "GET /api/v1/auth/account/deletion" => match *state_in.lock().unwrap() {
                "scheduled" => Reply::ok(json!({
                    "state": "scheduled", "shred_after": SHRED_AFTER, "cancel_until": CANCEL_UNTIL,
                })),
                "requested" => Reply::ok(json!({ "state": "requested", "link_expires_at": "2099-01-01T00:00:00Z" })),
                _ => Reply::ok(json!({ "state": "none" })),
            },
            "POST /api/v1/auth/account/delete-cancel" => {
                if body.get("token").is_some_and(|t| !t.is_null()) {
                    return Reply::error(404, "delete_link_invalid");
                }
                if !tokens.spend(request) {
                    return Reply::error(403, "step_up_required");
                }
                if mode_in.load(Ordering::SeqCst) == CANCEL_ANSWER_LOST {
                    return Reply::hang_up();
                }
                let mut state = state_in.lock().unwrap();
                if *state != "scheduled" {
                    return Reply::json(
                        404,
                        json!({ "error": "delete_not_scheduled", "code": "delete_not_scheduled" }),
                    );
                }
                *state = "none";
                Reply::ok(json!({ "cancelled": true, "not_restored": [] }))
            }
            _ => Reply::error(404, "not_found"),
        }
    }));
    Fake {
        api,
        state,
        mode,
        seen,
        during_request,
        during_step_up,
    }
}

fn engine(base: &str) -> Arc<TestEngine> {
    let state_dir = crate::state_paths::init_for_test();
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "an-invented-slot".into(),
        state_dir,
        api_base: base.to_string(),
    };
    Arc::new(Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the handle opens"))
}

/// Signed in and unlocked, with the session in memory.
fn signed_in(engine: &TestEngine) -> SessionGeneration {
    {
        let _writes = engine.writes.write().unwrap();
        let vault = engine.vault();
        vault.install_session(SessionToken::new(TOKEN).unwrap()).unwrap();
        vault.bind_session_owner(OWNER).unwrap();
        vault
            .store_wrapped_master_key(SecretBytes::new(vec![7; 32]).unwrap())
            .unwrap();
        vault.bind_vault_key_owner(OWNER).unwrap();
        engine
            .acct
            .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
        *engine.acct.session.lock().unwrap() = Some(Session {
            token: TOKEN.into(),
            master_key: [7; 32],
            email: Some("deleting@example.eu".into()),
        });
        engine.set_auth_present(true);
    }
    engine.session_turn()
}

fn secret(text: &str) -> Zeroizing<String> {
    Zeroizing::new(text.to_string())
}

#[test]
fn deletion_request_schedules_nothing() {
    let fake = fake();
    let engine = engine(&fake.api.base);
    let turn = signed_in(&engine);

    engine
        .request_account_deletion_for(turn, secret(PASSWORD), CODE)
        .expect("the link goes out");
    assert_eq!(fake.asked("POST /api/v1/auth/account/delete-request"), 1);
    assert_eq!(
        fake.asked("POST /api/v1/auth/confirm-opaque-start"),
        1,
        "core proved the password itself, once"
    );
    assert_eq!(
        engine.account_deletion_for(turn),
        Ok(DeletionState::Requested),
        "a link is out and nothing is scheduled"
    );
}

#[test]
fn a_wrong_code_hands_the_confirmation_back() {
    let fake = fake();
    let engine = engine(&fake.api.base);
    let turn = signed_in(&engine);

    assert_eq!(
        engine.request_account_deletion_for(turn, secret(PASSWORD), WRONG_CODE),
        Err(DeletionError::WrongCode)
    );
    assert_eq!(*fake.state.lock().unwrap(), "none", "a wrong code asks nothing");
    // Not a code at all: refused before anything is sent.
    let before = fake.api.routes().len();
    assert_eq!(
        engine.request_account_deletion_for(turn, secret(PASSWORD), "12"),
        Err(DeletionError::WrongCode)
    );
    assert_eq!(
        fake.api.routes().len(),
        before,
        "nothing was sent for a code that is not one"
    );
    // The same password, kept by the host, works with the right code: the server spent the first token, so core
    // proves the password again (a new step-up) and the person never types it twice.
    engine
        .request_account_deletion_for(turn, secret(PASSWORD), CODE)
        .expect("the right code goes through");
    assert_eq!(fake.asked("POST /api/v1/auth/confirm-opaque-start"), 2);
    assert_eq!(*fake.state.lock().unwrap(), "requested");
}

#[test]
fn undo_needs_the_step_up() {
    let fake = fake();
    let engine = engine(&fake.api.base);
    let turn = signed_in(&engine);
    *fake.state.lock().unwrap() = "scheduled";

    // A wrong password gives no grant: nothing to cancel with, and nothing is sent.
    assert_eq!(
        engine.step_up_for(turn, secret("not the password")).err(),
        Some(ActionError::WrongPassword)
    );
    assert_eq!(fake.asked("POST /api/v1/auth/account/delete-cancel"), 0);
    assert_eq!(
        *fake.state.lock().unwrap(),
        "scheduled",
        "a wrong password changes nothing"
    );

    let grant = engine
        .step_up_for(turn, secret(PASSWORD))
        .expect("the password is proven");
    engine
        .cancel_account_deletion_for(turn, grant)
        .expect("the signed-in cancel goes through");
    assert_eq!(*fake.state.lock().unwrap(), "none");
    assert_eq!(engine.account_deletion_for(turn), Ok(DeletionState::None));
    // The grant was spent on that one request: the next cancel needs a new proof (there is no other way to make one).
    let grant = engine.step_up_for(turn, secret(PASSWORD)).expect("proven again");
    assert_eq!(
        engine.cancel_account_deletion_for(turn, grant),
        Err(DeletionError::NotScheduled),
        "nothing is scheduled any more"
    );
}

#[test]
fn a_link_not_used_yet_cannot_be_cancelled_and_says_so() {
    let fake = fake();
    let engine = engine(&fake.api.base);
    let turn = signed_in(&engine);
    *fake.state.lock().unwrap() = "requested";

    let grant = engine.step_up_for(turn, secret(PASSWORD)).expect("proven");
    assert_eq!(
        engine.cancel_account_deletion_for(turn, grant),
        Err(DeletionError::NotScheduled)
    );
    assert_eq!(engine.account_deletion_for(turn), Ok(DeletionState::Requested));
}

#[test]
fn subscription_refusal_is_honest() {
    let fake = fake();
    let engine = engine(&fake.api.base);
    let turn = signed_in(&engine);

    fake.mode.store(PLAN_STILL_CHARGED, Ordering::SeqCst);
    assert_eq!(
        engine.request_account_deletion_for(turn, secret(PASSWORD), CODE),
        Err(DeletionError::SubscriptionActive),
        "never a wrong code, never 'offline'"
    );
    fake.mode.store(ALREADY_SCHEDULED, Ordering::SeqCst);
    assert_eq!(
        engine.request_account_deletion_for(turn, secret(PASSWORD), CODE),
        Err(DeletionError::AlreadyScheduled)
    );
    assert_eq!(*fake.state.lock().unwrap(), "none", "no link went out");
}

#[test]
fn late_answer_dropped() {
    let fake = fake();
    let engine = engine(&fake.api.base);
    let turn = signed_in(&engine);

    // A call for a session that is not the one in memory sends nothing.
    let other = {
        let _writes = engine.writes.write().unwrap();
        engine
            .acct
            .advance_session_generation_holding_session_write_lock(SessionTransition::Write)
    };
    assert_ne!(other, turn);
    assert_eq!(
        engine.request_account_deletion_for(turn, secret(PASSWORD), CODE),
        Err(DeletionError::NotSignedIn)
    );
    assert_eq!(engine.account_deletion_for(turn), Err(DeletionError::NotSignedIn));
    assert_eq!(
        fake.api.routes().len(),
        0,
        "nothing was asked for the session that is over"
    );

    // The session moves on (a lock, a sign-in) while the request is out: its answer is not handed over.
    let now = engine.session_turn();
    let moving = engine.clone();
    *fake.during_request.lock().unwrap() = Some(Box::new(move || {
        let _writes = moving.writes.write().unwrap();
        moving
            .acct
            .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
    }));
    // The link went out before the session moved, so the answer says it was sent (the window tells it after an unlock
    // for the same account), never "nothing was sent".
    assert_eq!(
        engine.request_account_deletion_for(now, secret(PASSWORD), CODE),
        Err(DeletionError::SentThenMoved)
    );
}

/// The request is asked again after its password step-up: a lock (decided off the window's thread) that lands while
/// the step-up is out moves the session on, and the request that mails the link does not leave. Nothing was sent
/// that changes anything, so there is nothing to tell the person.
#[test]
fn a_lock_during_the_step_up_mails_no_link() {
    let fake = fake();
    let engine = engine(&fake.api.base);
    let turn = signed_in(&engine);
    let moving = engine.clone();
    *fake.during_step_up.lock().unwrap() = Some(Box::new(move || {
        // What a lock does to the session: its turn moves on, under the session-write lock.
        let _writes = moving.writes.write().unwrap();
        moving
            .acct
            .advance_session_generation_holding_session_write_lock(SessionTransition::Write);
    }));
    assert_eq!(
        engine.request_account_deletion_for(turn, secret(PASSWORD), CODE),
        Err(DeletionError::NotSignedIn)
    );
    assert_eq!(
        fake.asked("POST /api/v1/auth/confirm-opaque-finish"),
        1,
        "the step-up was made"
    );
    assert_eq!(
        fake.asked("POST /api/v1/auth/account/delete-request"),
        0,
        "and no link was asked for"
    );
    assert_eq!(*fake.state.lock().unwrap(), "none");
}

#[test]
fn the_state_reads_beebeebs_three_answers() {
    let fake = fake();
    let engine = engine(&fake.api.base);
    let turn = signed_in(&engine);

    assert_eq!(engine.account_deletion_for(turn), Ok(DeletionState::None));
    *fake.state.lock().unwrap() = "requested";
    assert_eq!(engine.account_deletion_for(turn), Ok(DeletionState::Requested));
    *fake.state.lock().unwrap() = "scheduled";
    let at = |text: &str| Some(DateTime::parse_from_rfc3339(text).unwrap().with_timezone(&Utc));
    assert_eq!(
        engine.account_deletion_for(turn),
        Ok(DeletionState::Scheduled {
            shred_after: at(SHRED_AFTER),
            cancel_until: at(CANCEL_UNTIL),
        })
    );
}

#[test]
fn password_never_logged() {
    let fake = fake();
    let engine = engine(&fake.api.base);
    let turn = signed_in(&engine);

    let wrong = engine.request_account_deletion_for(turn, secret(PASSWORD), WRONG_CODE);
    engine
        .request_account_deletion_for(turn, secret(PASSWORD), CODE)
        .expect("asked");
    // OPAQUE: the password is in no request, neither a body nor a header.
    let seen = fake.seen.lock().unwrap().clone();
    assert!(!seen.is_empty());
    assert!(
        seen.iter().all(|text| !text.contains(PASSWORD)),
        "the password left in a request"
    );
    // And in no printable value this file hands out.
    let printed = format!("{wrong:?} {:?}", engine.account_deletion_for(turn));
    assert!(!printed.contains(PASSWORD));
    assert!(!printed.contains(TOKEN));
}

#[test]
fn a_lost_cancel_answer_is_not_known() {
    let fake = fake();
    let engine = engine(&fake.api.base);
    let turn = signed_in(&engine);
    *fake.state.lock().unwrap() = "scheduled";
    fake.mode.store(CANCEL_ANSWER_LOST, Ordering::SeqCst);

    let grant = engine.step_up_for(turn, secret(PASSWORD)).expect("proven");
    assert_eq!(
        engine.cancel_account_deletion_for(turn, grant),
        Err(DeletionError::Unconfirmed),
        "it left and no answer came: never 'not scheduled', never 'cancelled'"
    );
}
