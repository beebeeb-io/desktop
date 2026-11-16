//! The bulk permanent delete against the local fake API: ONE real OPAQUE proof for the whole batch, one call with
//! the ids, the 500 cap with its honest count, per-item refusals read as kept folders, and a wrong password that
//! deletes nothing.

use super::*;
use crate::test_api::{ConfirmTokens, FakeApi, OpaqueAccount, Reply, Request};
use std::sync::{Arc, Mutex, OnceLock};

const PASSWORD: &str = "correct horse battery staple";
const SESSION: &str = "session-of-the-test-account";

fn account() -> Arc<OpaqueAccount> {
    static ACCOUNT: OnceLock<Arc<OpaqueAccount>> = OnceLock::new();
    ACCOUNT
        .get_or_init(|| Arc::new(OpaqueAccount::register("bulk@example.test", PASSWORD)))
        .clone()
}

fn id(n: usize) -> String {
    format!("00000000-0000-4000-8000-{n:012}")
}

/// What the fake saw: the proofs it started, and the ids of each bulk call.
struct Seen {
    proofs: usize,
    calls: Vec<Vec<String>>,
}

/// The fake server: the two step-up legs and the bulk route (which refuses without a token it minted). `folder` is
/// an id it keeps as a folder that holds `3` live items; the others are deleted.
fn server(folder: Option<String>) -> (FakeApi, Arc<Mutex<Seen>>) {
    let account = account();
    let tokens = Arc::new(ConfirmTokens::default());
    let seen = Arc::new(Mutex::new(Seen {
        proofs: 0,
        calls: Vec::new(),
    }));
    let seen_in = seen.clone();
    let api = FakeApi::start(Arc::new(move |request: &Request| {
        if request.bearer() != Some(SESSION) {
            return Reply::error(401, "unauthorized");
        }
        match request.route().as_str() {
            "POST /api/v1/auth/confirm-opaque-start" => {
                seen_in.lock().unwrap().proofs += 1;
                account.login_start(&request.json())
            }
            "POST /api/v1/auth/confirm-opaque-finish" => {
                if account.login_finish(&request.json()) {
                    Reply::ok(serde_json::json!({ "confirmation_token": tokens.mint(), "expires_at": "soon" }))
                } else {
                    Reply::error(401, "invalid_credentials")
                }
            }
            "POST /api/v1/files/permanent" => {
                if !tokens.spend(request) {
                    return Reply::error(403, "step_up_required");
                }
                let ids: Vec<String> = request.json()["ids"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_string())
                    .collect();
                seen_in.lock().unwrap().calls.push(ids.clone());
                let (kept, deleted): (Vec<_>, Vec<_>) = ids.into_iter().partition(|i| Some(i) == folder.as_ref());
                if kept.is_empty() {
                    Reply::ok(serde_json::json!({ "deleted": deleted, "skipped_not_trashed": [], "missing": [] }))
                } else {
                    Reply::json(
                        409,
                        serde_json::json!({
                            "error": "has_live_descendants",
                            "message": "words the window never shows",
                            "refused": [{ "id": kept[0], "live_descendants": 3, "blocking_descendants": 3 }],
                            "cycles": [], "deleted": deleted, "skipped_not_trashed": [], "missing": [],
                        }),
                    )
                }
            }
            _ => Reply::error(404, "not_found"),
        }
    }));
    (api, seen)
}

fn run(api: &FakeApi, token: &str, ids: &[String], password: &str) -> BulkPurged {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let client = auth_transport::client().unwrap();
    runtime.block_on(purge_many(&api.base, &client, token, ids, password.as_bytes()))
}

#[test]
fn three_items_cost_one_proof_and_one_call() {
    let (api, seen) = server(None);
    let ids: Vec<String> = (1..=3).map(id).collect();
    let out = run(&api, SESSION, &ids, PASSWORD);
    assert_eq!(out.error, None);
    assert_eq!(out.deleted, ids);
    let seen = seen.lock().unwrap();
    assert_eq!((seen.proofs, seen.calls.len()), (1, 1), "proof starts = 1, calls = 1");
    assert_eq!(seen.calls[0], ids);
}

#[test]
fn a_folder_that_holds_live_items_is_kept_with_its_reason_and_the_rest_goes() {
    let (api, _) = server(Some(id(2)));
    let ids: Vec<String> = (1..=3).map(id).collect();
    let out = run(&api, SESSION, &ids, PASSWORD);
    assert_eq!(out.error, None);
    assert_eq!(out.deleted, vec![id(1), id(3)]);
    assert_eq!(
        out.kept,
        vec![Kept {
            id: id(2),
            why: KeptWhy::HoldsLiveItems(3)
        }]
    );
}

#[test]
fn a_wrong_password_deletes_nothing_and_sends_no_call() {
    let (api, seen) = server(None);
    let out = run(&api, SESSION, &[id(1)], "not the password");
    assert_eq!(out.error, Some(PurgeError::WrongPassword));
    assert!(out.deleted.is_empty());
    assert!(seen.lock().unwrap().calls.is_empty());
}

#[test]
fn ids_that_are_not_file_ids_or_too_many_never_reach_the_network() {
    let (api, seen) = server(None);
    let too_many: Vec<String> = (1..=MAX_BULK_IDS + 1).map(id).collect();
    for bad in [vec![], vec!["../x".to_string()], too_many] {
        assert_eq!(run(&api, SESSION, &bad, PASSWORD).error, Some(PurgeError::Failed));
    }
    assert_eq!(seen.lock().unwrap().proofs, 0);
    assert!(api.requests().is_empty());
}

#[test]
fn a_session_the_server_does_not_accept_is_ended_not_a_wrong_password() {
    let (api, _) = server(None);
    assert_eq!(
        run(&api, "another-session", &[id(1)], PASSWORD).error,
        Some(PurgeError::SessionEnded)
    );
}

#[test]
fn the_answer_of_each_status_is_read_without_the_servers_words() {
    let r = |status, body| parse_answer(&HttpResponse::new(status, body));
    assert_eq!(r(200, serde_json::json!({ "deleted": ["a"] })).deleted, vec!["a"]);
    let cycle = r(
        409,
        serde_json::json!({ "error": "folder_cycle", "cycles": ["c"], "deleted": ["a"] }),
    );
    assert_eq!(
        cycle.kept,
        vec![Kept {
            id: "c".into(),
            why: KeptWhy::Loop
        }]
    );
    assert_eq!(cycle.deleted, vec!["a"]);
    assert_eq!(
        r(409, serde_json::json!({ "error": "protected_folder" })).error,
        Some(PurgeError::Failed)
    );
    assert_eq!(r(403, serde_json::json!({})).error, Some(PurgeError::Failed));
    // A gateway error after the bulk delete left: the proxy may have passed it on (review of desktop#424).
    for status in [502, 503, 504] {
        assert_eq!(
            format!("{:?}", r(status, serde_json::json!({})).error),
            "Some(NotConfirmed)",
            "{status}"
        );
    }
    assert_eq!(r(500, serde_json::json!({})).error, Some(PurgeError::Failed));
    assert_eq!(r(401, serde_json::json!({})).error, Some(PurgeError::SessionEnded));
}

/// Review of desktop#424: the bulk DELETE is sent only after the proof, so a connection that drops after it left is
/// "not confirmed" (the items may be gone), never "offline".
#[test]
fn a_connection_that_drops_after_the_bulk_delete_left_is_not_confirmed() {
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
    let ids: Vec<String> = (1..=2).map(id).collect();
    let out = run(&api, "any-session", &ids, PASSWORD);
    assert_eq!(
        format!("{:?}", out.error),
        "Some(NotConfirmed)",
        "the delete left; nothing is known"
    );
    assert!(out.deleted.is_empty());
    assert!(
        api.routes().iter().any(|route| route == "POST /api/v1/files/permanent"),
        "the delete did leave"
    );
}

/// Review of desktop#424 (engine P2-1, crypto P2-2): a locked screen refuses the bulk permanent delete before
/// anything is sent, also for a turn read while it was locked (an unlock does not move the turn).
#[test]
fn a_locked_screen_deletes_nothing_and_sends_nothing() {
    use crate::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
    let (account, phrase) = FakeAccount::register("bulk-locked@example.eu", PASSWORD, TEST_PHRASE);
    let account = Arc::new(account);
    let api = FakeApi::start(account_handler(account.clone()));
    let (host, _messages) = crate::host::ChannelHost::new(|| {});
    let config = crate::app::EngineConfig {
        account_id: "bulk-locked-slot".into(),
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
    let before = api.routes().len();
    let purged = engine.purge_trash_bulk_for(session, &[id(1), id(2)], PASSWORD.as_bytes());
    assert!(purged.deleted.is_empty());
    assert_eq!(purged.error, Some(PurgeError::NotSignedIn));
    assert_eq!(
        api.routes()[before..].to_vec(),
        Vec::<String>::new(),
        "nothing is sent under a locked screen"
    );
}
