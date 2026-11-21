//! `PATCH /api/v1/files/{id}` on the fake (`fake_uploads::patch_file`): the metadata update the engine's upload step
//! sends for a file that moves (the R-UP rung: an upload the fake could not finish). Each test runs the real client
//! (`ApiClient::update_metadata`, or the engine's own queued upload) against the real handler over loopback.

use std::sync::Arc;

use serde_json::{Value, json};

use super::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use super::{FakeApi, Request};
use crate::api_client::{ApiClient, ParentChange};
use crate::state_db::{OperationKind, PendingOperation, StateDb};

const FOLDER: &str = "c0000000-0000-4000-8000-0000000000f1";
const OWN: &str = "c0000000-0000-4000-8000-0000000000a1";
const NEW_FILE: &str = "c0000000-0000-4000-8000-0000000000b1";
/// A file id the account does not hold: another account's, as far as this server can tell.
const STRANGER: &str = "c0000000-0000-4000-8000-0000000000e9";

fn world() -> (Arc<FakeAccount>, FakeApi, ApiClient) {
    let (account, _) = FakeAccount::register("patch@example.invalid", "a password", TEST_PHRASE);
    let account = Arc::new(account);
    account.add_item(FOLDER, None, "Docs", true, 0);
    account.add_item(OWN, None, "own.txt", false, 3);
    let api = FakeApi::start(account_handler(account.clone()));
    let client = ApiClient::new(api.base.clone(), account.session_token.clone(), account.master_key);
    (account, api, client)
}

fn run<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

/// An upload the engine makes of a new file into a folder: the step that moves it is a `PATCH`, then the chunks and
/// `complete`. With the fake lacking the route the upload never completed (0 completed in the upgrade rung).
#[test]
fn an_engine_upload_into_a_folder_completes_and_the_row_says_uploaded() {
    let (account, api, client) = world();
    let dir = tempfile::tempdir().unwrap();
    let payload = dir.path().join("payload.bin");
    std::fs::write(&payload, b"patched upload").unwrap();
    let db = Arc::new(StateDb::open(dir.path().join("db")).unwrap());
    let bridge = crate::engine_bridge::EngineBridge::new(&crate::identity::TEST, db, Arc::new(client));
    let op = PendingOperation {
        op_id: "op-patch-upload".into(),
        kind: OperationKind::UploadVersion,
        file_id: Some(NEW_FILE.into()),
        parent_id: Some(FOLDER.into()),
        target_path: Some("Docs/new.txt".into()),
        metadata_json: Some(
            json!({
                "content_type": "text/plain",
                "parent_changed": true,
                "display_name": "new.txt",
                "name_encrypted": account.encrypted_name(NEW_FILE, "new.txt"),
            })
            .to_string(),
        ),
        payload_path: Some(payload.to_string_lossy().into_owned()),
        base_version: None,
        base_object_version_id: None,
        attempts: 0,
        max_attempts: 5,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: 1,
        updated_at: 1,
    };
    bridge.db().enqueue_operation(&op).unwrap();
    let outcome = run(bridge.process_due_operations(dir.path(), i64::MAX / 2)).unwrap();
    let error = bridge
        .db()
        .get_operation(&op.op_id)
        .unwrap()
        .and_then(|o| o.last_error)
        .unwrap_or_default();
    let patches: Vec<(String, u16)> = api
        .requests()
        .iter()
        .filter(|r| r.method == "PATCH")
        .map(|r| (r.path.clone(), 200))
        .collect();
    assert_eq!(
        (
            outcome.completed_op_ids,
            patches.len(),
            account.item_state(NEW_FILE).map(|s| s.0)
        ),
        (vec!["op-patch-upload".to_string()], 1, Some(1)),
        "completed ops, PATCH calls, the row's version on the fake; last error: {error}"
    );
    assert_eq!(patches[0].0, format!("/api/v1/files/{NEW_FILE}"));
    assert!(
        account.stored_frames(NEW_FILE).is_some(),
        "the fake holds the uploaded content"
    );
    let snapshot = account.snapshot_nodes();
    let row = snapshot
        .iter()
        .find(|n| n["id"] == NEW_FILE)
        .expect("the row is in the snapshot");
    assert_eq!(
        row["parent_id"], FOLDER,
        "the row sits in the folder it was uploaded into"
    );
}

/// The client's own call: a rename and a move answer the row; the fake keeps both.
#[test]
fn a_patch_of_an_own_file_renames_and_moves_it_and_answers_the_row() {
    let (account, _api, client) = world();
    let sealed = account.encrypted_name(OWN, "renamed.txt");
    let row = run(client.update_metadata(OWN, Some(&sealed), ParentChange::Folder(FOLDER))).expect("an own file");
    assert_eq!(
        (
            row["id"].clone(),
            row["parent_id"].clone(),
            row["name_encrypted"].clone()
        ),
        (json!(OWN), json!(FOLDER), json!(sealed)),
        "the answer is the row as it is now"
    );
    assert_eq!(
        account.find_by_path("Docs/renamed.txt").as_deref(),
        Some(OWN),
        "the fake kept the move and the name"
    );
    let top = run(client.update_metadata(OWN, None, ParentChange::TopLevel)).expect("back to the top level");
    assert_eq!(
        top["parent_id"],
        Value::Null,
        "an explicit null parent is the top level"
    );
}

/// Another account's file (an id this account does not hold) is `404 not_found`, the same as no file at all; a missing
/// token is `401`; an empty body is `400`. None of them changes the account's own rows.
#[test]
fn a_patch_for_another_accounts_file_is_refused_as_not_found() {
    let (account, api, client) = world();
    let before = account.snapshot_nodes();
    let error = run(client.update_metadata(STRANGER, Some("x"), ParentChange::Folder(FOLDER))).expect_err("refused");
    let refused = error
        .downcast_ref::<reqwest::Error>()
        .and_then(|e| e.status())
        .map(|s| s.as_u16());
    let call = |token: Option<&str>, id: &str, body: Value| {
        let handler = account_handler(account.clone());
        handler(&Request {
            method: "PATCH".into(),
            path: format!("/api/v1/files/{id}"),
            headers: token
                .map(|t| vec![("Authorization".to_string(), format!("Bearer {t}"))])
                .unwrap_or_default(),
            body: body.to_string().into_bytes(),
        })
    };
    let stranger = call(Some(&account.session_token), STRANGER, json!({ "name_encrypted": "x" }));
    let no_token = call(None, OWN, json!({ "name_encrypted": "x" }));
    let empty = call(Some(&account.session_token), OWN, json!({}));
    let into_a_stranger_folder = call(Some(&account.session_token), OWN, json!({ "parent_id": STRANGER }));
    assert_eq!(
        (
            stranger.status,
            serde_json::from_slice::<Value>(&stranger.body).unwrap()["error"].clone(),
            no_token.status,
            empty.status,
            into_a_stranger_folder.status,
        ),
        (404, json!("not_found"), 401, 400, 404),
        "stranger's file, its error code, no token, empty body, a folder that is not this account's"
    );
    assert!(
        refused.is_none() || refused == Some(404),
        "the client sees the refusal as an error: {error}"
    );
    assert_eq!(account.snapshot_nodes(), before, "no row of the account changed");
    drop(api);
}
