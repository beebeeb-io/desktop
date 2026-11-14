//! The permanent delete against the local fake API: the password is proven with real OPAQUE, the server's single-use
//! token is spent on the delete of exactly one item, a wrong password deletes nothing, and a token is never reused.

use super::*;
use crate::test_api::{ConfirmTokens, FakeApi, OpaqueAccount, Reply, Request};
use std::sync::{Arc, Mutex, OnceLock};

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
fn server() -> (FakeApi, Arc<Mutex<Vec<String>>>, Arc<Mutex<usize>>) {
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
