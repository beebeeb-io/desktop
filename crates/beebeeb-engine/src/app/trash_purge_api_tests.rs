//! The permanent delete against the local fake API: the password is proven with real OPAQUE, the server's single-use
//! token is spent on the delete of exactly one item, a wrong password deletes nothing, and a token is never reused.

use super::*;
use crate::test_api::{ConfirmTokens, FakeApi, OpaqueAccount, Reply, Request};
use std::sync::{Arc, Mutex, OnceLock};

/// The fake, the calls it saw, and a counter.
type ServerRig = (FakeApi, Arc<Mutex<Vec<String>>>, Arc<Mutex<usize>>);

const PASSWORD: &str = "correct horse battery staple";
const SESSION: &str = "session-of-the-test-account";
const ONE: &str = "00000000-0000-4000-8000-000000000011";
const TWO: &str = "00000000-0000-4000-8000-000000000012";

fn account() -> Arc<OpaqueAccount> {
    static ACCOUNT: OnceLock<Arc<OpaqueAccount>> = OnceLock::new();
    ACCOUNT
        .get_or_init(|| Arc::new(OpaqueAccount::register("purge@example.test", PASSWORD)))
        .clone()
}

/// The fake server: the two step-up legs and the permanent delete, which it refuses without a token it minted.
/// `purged` is what it deleted for good, `minted` how many tokens it handed out.
fn server() -> ServerRig {
    let account = account();
    let tokens = Arc::new(ConfirmTokens::default());
    let purged = Arc::new(Mutex::new(Vec::new()));
    let minted = Arc::new(Mutex::new(0usize));
    let (purged_in, minted_in) = (purged.clone(), minted.clone());
    let api = FakeApi::start(Arc::new(move |request: &Request| {
        if request.bearer() != Some(SESSION) {
            return Reply::error(401, "unauthorized");
        }
        let route = request.route();
        match route.as_str() {
            "POST /api/v1/auth/confirm-opaque-start" => account.login_start(&request.json()),
            "POST /api/v1/auth/confirm-opaque-finish" => {
                if account.login_finish(&request.json()) {
                    *minted_in.lock().unwrap() += 1;
                    Reply::ok(serde_json::json!({ "confirmation_token": tokens.mint(), "expires_at": "soon" }))
                } else {
                    Reply::error(401, "invalid_credentials")
                }
            }
            _ => match route
                .strip_prefix("DELETE /api/v1/files/")
                .and_then(|rest| rest.strip_suffix("/permanent"))
            {
                Some(id) if tokens.spend(request) => {
                    purged_in.lock().unwrap().push(id.to_string());
                    Reply::empty(204)
                }
                Some(_) => Reply::error(403, "step_up_required"),
                None => Reply::error(404, "not_found"),
            },
        }
    }));
    (api, purged, minted)
}

fn run(api: &FakeApi, token: &str, id: &str, password: &str) -> Result<(), PurgeError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let client = auth_transport::client().unwrap();
    runtime.block_on(purge_one(&api.base, &client, token, id, password.as_bytes()))
}

#[test]
fn the_right_password_deletes_that_one_item_for_good_with_one_token() {
    let (api, purged, minted) = server();
    assert_eq!(run(&api, SESSION, ONE, PASSWORD), Ok(()));
    assert_eq!(*purged.lock().unwrap(), vec![ONE.to_string()]);
    assert_eq!(*minted.lock().unwrap(), 1);
    // The next item needs its own proof: a token is never reused.
    assert_eq!(run(&api, SESSION, TWO, PASSWORD), Ok(()));
    assert_eq!(*purged.lock().unwrap(), vec![ONE.to_string(), TWO.to_string()]);
    assert_eq!(*minted.lock().unwrap(), 2);
}

#[test]
fn a_wrong_password_deletes_nothing() {
    let (api, purged, _) = server();
    assert_eq!(
        run(&api, SESSION, ONE, "not the password"),
        Err(PurgeError::WrongPassword)
    );
    assert_eq!(run(&api, SESSION, ONE, ""), Err(PurgeError::WrongPassword));
    assert!(purged.lock().unwrap().is_empty());
}

#[test]
fn a_session_the_server_does_not_accept_is_ended_not_a_wrong_password() {
    let (api, purged, _) = server();
    assert_eq!(
        run(&api, "another-session", ONE, PASSWORD),
        Err(PurgeError::SessionEnded)
    );
    assert!(purged.lock().unwrap().is_empty());
}

#[test]
fn an_id_that_is_not_a_file_id_never_reaches_the_network() {
    let (api, purged, minted) = server();
    for bad in ["", "../x", "A0000000-0000-4000-8000-000000000011", "not-an-id"] {
        assert_eq!(run(&api, SESSION, bad, PASSWORD), Err(PurgeError::Failed), "{bad:?}");
    }
    assert!(purged.lock().unwrap().is_empty());
    assert_eq!(*minted.lock().unwrap(), 0);
    assert!(api.requests().is_empty());
}

/// The DELETE is sent only after the password proof, so a connection that drops AFTER it left
/// says nothing about whether the item is gone. It is "not confirmed", never "offline" (which promises that nothing
/// was sent) and never "refused".
#[test]
fn a_connection_that_drops_after_the_delete_left_is_not_confirmed() {
    let account = account();
    let tokens = Arc::new(ConfirmTokens::default());
    let api = FakeApi::start(Arc::new(move |request: &Request| match request.route().as_str() {
        "POST /api/v1/auth/confirm-opaque-start" => account.login_start(&request.json()),
        "POST /api/v1/auth/confirm-opaque-finish" => {
            if account.login_finish(&request.json()) {
                Reply::ok(serde_json::json!({ "confirmation_token": tokens.mint(), "expires_at": "soon" }))
            } else {
                Reply::error(401, "invalid_credentials")
            }
        }
        _ => Reply::hang_up(),
    }));
    let error = run(&api, "any-session", ONE, PASSWORD).unwrap_err();
    assert_eq!(
        format!("{error:?}"),
        "NotConfirmed",
        "the delete left; nothing is known"
    );
    assert!(
        api.routes().iter().any(|route| route.starts_with("DELETE ")),
        "the delete did leave"
    );
}

/// A gateway error (502, 503, 504) after the DELETE left is as unknown as a dropped
/// connection: the proxy may have passed the delete on.
#[test]
fn a_gateway_error_after_the_delete_left_is_not_confirmed() {
    for status in [502u16, 503, 504] {
        let account = account();
        let tokens = Arc::new(ConfirmTokens::default());
        let api = FakeApi::start(Arc::new(move |request: &Request| match request.route().as_str() {
            "POST /api/v1/auth/confirm-opaque-start" => account.login_start(&request.json()),
            "POST /api/v1/auth/confirm-opaque-finish" => {
                if account.login_finish(&request.json()) {
                    Reply::ok(serde_json::json!({ "confirmation_token": tokens.mint(), "expires_at": "soon" }))
                } else {
                    Reply::error(401, "invalid_credentials")
                }
            }
            _ => Reply::error(status, "a gateway's words"),
        }));
        let error = run(&api, "any-session", ONE, PASSWORD).unwrap_err();
        assert_eq!(format!("{error:?}"), "NotConfirmed", "{status}");
    }
    // A refusal in so many words (a 500 from the app itself, a 403) is still just refused.
    let (api, _, _) = server();
    assert_eq!(run(&api, SESSION, ONE, PASSWORD), Ok(()));
}

/// A real handle signed in to the engine's fake account, then screen-locked: the turn is read under the lock, as a
/// press drawn over the lock screen would read it (an unlock does not move the turn).
fn screen_locked_engine(
    tag: &str,
) -> (
    crate::app::Engine<crate::signin::memory_store::MemoryCredentials>,
    FakeApi,
    crate::account::SessionGeneration,
) {
    use crate::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
    let (account, phrase) = FakeAccount::register(&format!("{tag}@example.eu"), PASSWORD, TEST_PHRASE);
    let account = Arc::new(account);
    let api = FakeApi::start(account_handler(account.clone()));
    let (host, _messages) = crate::host::ChannelHost::new(|| {});
    let config = crate::app::EngineConfig {
        account_id: format!("{tag}-slot"),
        state_dir: crate::state_paths::init_for_test(),
        api_base: api.base.clone(),
    };
    let engine = crate::app::Engine::open(
        &crate::identity::TEST,
        config,
        host,
        crate::signin::memory_store::MemoryCredentials::default(),
    )
    .expect("the engine opens");
    engine.restore();
    engine
        .sign_in_password(&account.email, zeroize::Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(zeroize::Zeroizing::new(phrase))
        .expect("the phrase unlocks");
    engine.lock_screen().expect("an open lock can lock the screen");
    let session = engine.session_turn();
    (engine, api, session)
}

/// A locked screen refuses a permanent delete before anything is
/// sent, also for a turn read while it was locked.
#[test]
fn a_locked_screen_deletes_nothing_and_sends_nothing() {
    let (engine, api, session) = screen_locked_engine("purge-locked");
    let before = api.routes().len();
    let purged = engine.purge_trash_for(session, &[ONE.to_string()], PASSWORD.as_bytes(), |_| {});
    assert_eq!(purged.done, 0);
    assert_eq!(purged.error, Some(PurgeError::NotSignedIn));
    assert_eq!(
        api.routes()[before..].to_vec(),
        Vec::<String>::new(),
        "nothing is sent under a locked screen"
    );
}
