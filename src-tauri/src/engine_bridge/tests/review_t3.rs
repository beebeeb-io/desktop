//! T3 review pins (task 2275): the request-row version upload and the foreign
//! item's missing key.
//!
//! Owns: P1-1 (a version upload of a request row becomes a new item) and P2-8
//! (an unknown foreign item never falls back to a derived key).
//! Must not: call `adopt_foreign_subtree` directly or change a stored format.
//! Tests: `review_p1_1_`, `review_p2_8_`.

use super::*;
use crate::owner_keys::OwnedKeyRecord;

const REQUEST: &str = "0f000000-0000-4000-8000-000000000011";
const FOLDER: &str = "40000000-0000-4000-8000-0000000000d1";
const FOREIGN: &str = "20000000-0000-8000-8000-0000000000d2";

#[tokio::test]
async fn review_p1_1_a_version_upload_of_a_request_row_creates_a_new_item() {
    use beebeeb_core::kdf::{MasterKey, derive_file_key};
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync");
    std::fs::create_dir_all(&sync_root).unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    seed_folder_row(&bridge, FOLDER, "Inbox");
    seed_bridge_row(
        &bridge,
        REQUEST,
        "Inbox/report.txt",
        Some(FOLDER),
        FileStatus::Local,
        4,
    );

    // The row's name is sealed under the derived string convention, and the row
    // is a 2236 request row (the durable L12 fact P3-3 persists).
    let master = MasterKey::from_bytes([7; 32]);
    let derived = derive_file_key(&master, REQUEST.as_bytes());
    let name = beebeeb_core::encrypt::encrypt_name_for(
        &derived,
        "report.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    bridge
        .db()
        .put_owned_key_record(&OwnedKeyRecord {
            file_id: REQUEST.into(),
            name_encrypted: name.clone(),
            request_row: true,
            ..Default::default()
        })
        .unwrap();

    let payload = dir.path().join("payload.bin");
    std::fs::write(&payload, b"edit").unwrap();
    // P3-2: the request row's bytes on disk must stay exactly as they were.
    let original_disk = sync_root.join("Inbox/report.txt");
    std::fs::create_dir_all(original_disk.parent().unwrap()).unwrap();
    std::fs::write(&original_disk, b"original request bytes").unwrap();
    let op = PendingOperation {
        op_id: "op-request-edit".into(),
        kind: OperationKind::UploadVersion,
        file_id: Some(REQUEST.into()),
        parent_id: Some(FOLDER.into()),
        target_path: Some("Inbox/report.txt".into()),
        metadata_json: Some(
            serde_json::json!({"name_encrypted": name, "content_type": "text/plain"}).to_string(),
        ),
        payload_path: Some(payload.to_string_lossy().into_owned()),
        base_version: None,
        base_object_version_id: None,
        attempts: 0,
        max_attempts: 25,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: 0,
        updated_at: 0,
    };
    let mut errors = Vec::new();
    let done = bridge
        .upload_version(&op, None, &sync_root, &mut errors)
        .await
        .expect("a request-row edit is re-planned, not refused");

    assert!(matches!(done, OpDone::Remove { .. }), "the request op is done, not sent");
    let rows = bridge.db().list_files().unwrap();
    let created: Vec<_> = rows
        .iter()
        .filter(|row| row.path == "Inbox/report (edited).txt")
        .collect();
    assert_eq!(created.len(), 1, "a NEW item under a fresh id: {rows:?}");
    assert_ne!(created[0].file_id, REQUEST, "the request row is never the upload target");
    let request = bridge
        .db()
        .get_file(REQUEST)
        .unwrap()
        .expect("the request row still exists");
    assert_eq!(request.path, "Inbox/report.txt", "the original row is unchanged");
    assert!(
        bridge.db().list_operations_for_file(REQUEST).unwrap().is_empty(),
        "no version is uploaded under the request row"
    );
    let ops = bridge.db().list_operations_for_file(&created[0].file_id).unwrap();
    let upload = ops
        .iter()
        .find(|queued| queued.kind == OperationKind::UploadVersion)
        .expect("the new item carries the version upload");
    // P3-2: the fresh id's derived key opens the copy's name...
    let fresh = beebeeb_core::kdf::derive_file_key(&master, created[0].file_id.as_bytes());
    let metadata: serde_json::Value =
        serde_json::from_str(upload.metadata_json.as_deref().expect("the create carries metadata")).unwrap();
    let name_encrypted = metadata["name_encrypted"].as_str().expect("the create seals a name");
    let decoded = beebeeb_core::encrypt::decrypt_name_metadata_with_key(&fresh, name_encrypted)
        .expect("the copy's name opens under the fresh id's derived key");
    assert_eq!(decoded.name, "report (edited).txt");
    // ...and points at the edited bytes, not the original.
    let copied = std::fs::read(upload.payload_path.as_deref().expect("the create carries the copy")).unwrap();
    assert_eq!(copied, b"edit", "the copy holds the staged edit");
    // The request row's own bytes are untouched.
    assert_eq!(
        std::fs::read(&original_disk).unwrap(),
        b"original request bytes",
        "the request row's bytes are never rewritten"
    );
}

/// P1-1: a remote rename of a request row arrives as a synthesized sync op that
/// carries no 2236 envelope. The durable fact must survive it, so the next local
/// edit is still re-planned as a new item instead of a version sealed under the
/// derived name key.
#[tokio::test]
async fn review_p1_1_a_remote_rename_of_a_request_row_keeps_the_fact() {
    use beebeeb_core::kdf::{MasterKey, derive_file_key};
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync");
    std::fs::create_dir_all(&sync_root).unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    seed_folder_row(&bridge, FOLDER, "Inbox");
    seed_bridge_row(
        &bridge,
        REQUEST,
        "Inbox/report.txt",
        Some(FOLDER),
        FileStatus::Local,
        4,
    );
    let master = MasterKey::from_bytes([7; 32]);
    let derived = derive_file_key(&master, REQUEST.as_bytes());
    let renamed = beebeeb_core::encrypt::encrypt_name_for(
        &derived,
        "renamed.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    bridge
        .db()
        .put_owned_key_record(&OwnedKeyRecord {
            file_id: REQUEST.into(),
            name_encrypted: renamed.clone(),
            request_row: true,
            ..Default::default()
        })
        .unwrap();

    // The server renames the request row under the derived string convention and
    // ships a sparse `file_rename` op: no 2236 envelope fields.
    let op = crate::api_client::SyncOp {
        seq_id: 9,
        op_type: "file_rename".into(),
        payload: serde_json::json!({
            "id": REQUEST,
            "new_name_encrypted": renamed,
            "parent_id": FOLDER,
        }),
    };
    let prepared = crate::engine_bridge::syncflow::prepare_owned_sync_op(&bridge, &op)
        .await
        .expect("the rename is enriched");
    assert!(prepared.is_some(), "a rename op is carried through");
    assert!(
        bridge.db().get_owned_key_record(REQUEST).unwrap().unwrap().request_row,
        "the durable request-row fact survives a synthesized sync row"
    );
    assert!(bridge.is_request_row(REQUEST), "the in-memory set keeps it too");

    // A following local edit must be re-planned as a NEW owner item.
    let payload_path = dir.path().join("edit.bin");
    std::fs::write(&payload_path, b"edited body").unwrap();
    let edit = PendingOperation {
        op_id: "op-request-edit-after-rename".into(),
        kind: OperationKind::UploadVersion,
        file_id: Some(REQUEST.into()),
        parent_id: Some(FOLDER.into()),
        target_path: Some("Inbox/renamed.txt".into()),
        metadata_json: Some(
            serde_json::json!({"name_encrypted": renamed, "content_type": "text/plain"}).to_string(),
        ),
        payload_path: Some(payload_path.to_string_lossy().into_owned()),
        base_version: None,
        base_object_version_id: None,
        attempts: 0,
        max_attempts: 25,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: 0,
        updated_at: 0,
    };
    let mut errors = Vec::new();
    let done = bridge
        .upload_version(&edit, None, &sync_root, &mut errors)
        .await
        .expect("the edit is re-planned, not sealed under the derived key");
    assert!(matches!(done, OpDone::Remove { .. }), "the request op is done, not sent");
    let rows = bridge.db().list_files().unwrap();
    let created: Vec<_> = rows
        .iter()
        .filter(|row| row.path == "Inbox/renamed (edited).txt")
        .collect();
    assert_eq!(created.len(), 1, "a NEW item under a fresh id: {rows:?}");
    assert_ne!(created[0].file_id, REQUEST, "never a version under the request row");
    assert!(
        bridge.db().list_operations_for_file(REQUEST).unwrap().is_empty(),
        "no version is sealed under the request row's derived key"
    );
}

/// The sealed 2236 envelope fields of a real request row, plus the owner's
/// wrapped request private key (served at `GET /api/v1/file-requests`).
fn request_envelope_fixture(request_id: &str, content: [u8; 32]) -> (serde_json::Value, serde_json::Value) {
    use beebeeb_core::kdf::MasterKey;
    use beebeeb_core::opaque::derive_x25519_public;
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    let master = MasterKey::from_bytes([7; 32]);
    let r_priv = [0x21u8; 32];
    let r_pub = derive_x25519_public(&r_priv);
    let sealed = beebeeb_core::file_request::seal_to_request(&r_pub, b"", &content).unwrap();
    let (wrapped_private, wrap_nonce) =
        beebeeb_core::file_request::wrap_request_private(&master, b"", &r_priv).unwrap();
    let envelope = serde_json::json!({
        "file_request_id": request_id,
        "sender_ephemeral_pubkey": STANDARD.encode(sealed.e_pub),
        "wrapped_content_key": STANDARD.encode(&sealed.wrapped_key),
    });
    let requests = serde_json::json!({"file_requests": [{
        "id": request_id,
        "wrapped_private_key": STANDARD.encode(&wrapped_private),
        "wrap_nonce": STANDARD.encode(&wrap_nonce),
    }]});
    (envelope, requests)
}

/// P1-1: a row that carries the 2236 envelope with one malformed field is
/// unreadable, and never clears the durable fact or overwrites the record.
///
/// Review 4 P2-1: it also no longer aborts the whole pass — an uploader could
/// otherwise stall the owner's every prepare by adding one such row. The other
/// rows in the same snapshot still prepare.
///
/// Review 4 P3-4: mutant-proven — treating a malformed envelope as absent
/// (`request_envelope(&row).ok().flatten()`) turns this red.
#[tokio::test]
async fn review_p1_1_a_malformed_envelope_keeps_the_fact_and_refuses_the_write() {
    use beebeeb_core::kdf::{MasterKey, derive_file_key};
    const REQUEST_ID: &str = "0f000000-0000-4000-8000-0000000000e2";
    let master = MasterKey::from_bytes([7; 32]);
    let derived = derive_file_key(&master, REQUEST.as_bytes());
    let name = beebeeb_core::encrypt::encrypt_name_for(
        &derived,
        "report.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    let (envelope, requests) = request_envelope_fixture(REQUEST_ID, [0x44u8; 32]);
    let mut row = serde_json::json!({
        "id": REQUEST,
        "name_encrypted": name,
        "parent_id": FOLDER,
        "is_folder": false,
        "version_number": 1,
        "size_bytes": 4,
        "updated_at": 1,
    });
    row.as_object_mut().unwrap().extend(envelope.as_object().unwrap().clone());
    let (base, _server) = json_server(move |path| {
        if path == "/api/v1/file-requests" {
            Some(requests.clone())
        } else {
            None
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), base, [7u8; 32]);
    // The real row sets the durable fact through core's resolver.
    let prepared = bridge
        .prepare_owned_rows(&[row.clone()])
        .await
        .expect("the request row is readable");
    assert_eq!(prepared.len(), 1);
    let record = bridge.db().get_owned_key_record(REQUEST).unwrap().expect("the row is recorded");
    assert!(record.request_row, "core resolved the request row");
    assert!(bridge.is_request_row(REQUEST));

    // The same row with one field that does not decode: carrying-but-malformed.
    let mut malformed = row.clone();
    malformed["wrapped_content_key"] = serde_json::json!("%%%not-base64%%%");
    malformed["name_encrypted"] = serde_json::json!(beebeeb_core::encrypt::encrypt_name_for(
        &derived,
        "changed.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap());
    // Review 4 P2-1: the same pass also carries a healthy owner row. One
    // malformed envelope must not abort the snapshot for it.
    const OTHER: &str = "40000000-0000-4000-8000-0000000000e5";
    let other_name = beebeeb_core::encrypt::encrypt_name_for(
        &derive_file_key(&master, OTHER.as_bytes()),
        "other.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    let other = serde_json::json!({
        "id": OTHER,
        "name_encrypted": other_name,
        "parent_id": FOLDER,
        "is_folder": false,
        "version_number": 1,
        "size_bytes": 4,
        "updated_at": 1,
    });
    let prepared = bridge
        .prepare_owned_rows(&[malformed.clone(), other])
        .await
        .expect("one malformed envelope must not abort the pass");
    assert_eq!(prepared.len(), 2, "every row in the pass is reported");
    let unreadable = prepared
        .iter()
        .find(|prepared| prepared.row["id"].as_str() == Some(REQUEST))
        .expect("the malformed row is reported, not dropped");
    assert!(!unreadable.readable, "the malformed row is unreadable");
    let other = prepared
        .iter()
        .find(|prepared| prepared.row["id"].as_str() == Some(OTHER))
        .expect("the other row is reported");
    assert!(other.readable, "the other rows in the same pass still prepare");
    // A write through the ordinary owned-key seam refuses it.
    let err = match crate::engine_bridge::meta::owned_metadata_for_name(
        &bridge.db(),
        &[7u8; 32],
        REQUEST,
        "changed.txt",
        None,
    ) {
        Ok(_) => panic!("an unreadable request row refuses a write"),
        Err(err) => err,
    };
    assert!(
        err.to_string().contains("request row has no content key"),
        "the row refuses a write: {err}"
    );
    let record = bridge.db().get_owned_key_record(REQUEST).unwrap().expect("the row is still recorded");
    assert!(record.request_row, "the durable fact is never cleared by server data");
    assert_eq!(record.name_encrypted, name, "the unreadable row writes nothing");
    assert!(bridge.is_request_row(REQUEST), "the in-memory set keeps it too");
}

/// P1-1: a row this build has never stored that carries a malformed envelope
/// must still get its block into the DB. It was reported unreadable but written
/// nowhere, so `get_owned_key_record` stayed `None` and every write seam took
/// `owned_key_and_pin`'s no-record branch and derived the string key.
///
/// Mutants: dropping the `scope.put` keeps the record `None`; dropping the
/// `blocked_fingerprint` assignment stores it unblocked. Either turns this red.
#[tokio::test]
async fn review_p1_1_a_never_seen_malformed_row_is_blocked_and_has_no_write_key() {
    use beebeeb_core::kdf::{MasterKey, derive_file_key};
    const REQUEST_ID: &str = "0f000000-0000-4000-8000-0000000000e8";
    let master = MasterKey::from_bytes([7; 32]);
    let (envelope, requests) = request_envelope_fixture(REQUEST_ID, [0x49u8; 32]);
    let (base, _server) = json_server(move |path| (path == "/api/v1/file-requests").then(|| requests.clone()));
    let dir = tempfile::tempdir().unwrap();
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), base, [7u8; 32]);
    // (file id, the served field value that makes the carried envelope malformed)
    let malformed = [
        (
            "40000000-0000-4000-8000-0000000000e8",
            serde_json::json!("%%%not-base64%%%"),
        ),
        // P3-3: a present-but-null field carries (malformed), never absent.
        ("40000000-0000-4000-8000-0000000000e9", serde_json::Value::Null),
    ];
    for (row_id, _) in malformed.iter() {
        assert!(
            bridge.db().get_owned_key_record(row_id).unwrap().is_none(),
            "the row is never seen: {row_id}"
        );
    }
    let mut rows = Vec::new();
    for (row_id, wrapped) in malformed.iter() {
        let name = beebeeb_core::encrypt::encrypt_name_for(
            &derive_file_key(&master, row_id.as_bytes()),
            "report.txt",
            None,
            beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
        )
        .unwrap();
        let mut row = serde_json::json!({
            "id": row_id,
            "name_encrypted": name,
            "parent_id": FOLDER,
            "is_folder": false,
            "version_number": 1,
            "size_bytes": 4,
            "updated_at": 1,
        });
        row.as_object_mut().unwrap().extend(envelope.as_object().unwrap().clone());
        row["wrapped_content_key"] = wrapped.clone();
        rows.push(row);
    }
    let prepared = bridge.prepare_owned_rows(&rows).await.expect("one malformed row never aborts the pass");
    assert_eq!(prepared.len(), rows.len());
    assert!(prepared.iter().all(|prepared| !prepared.readable), "every malformed row is unreadable");
    for (row_id, _) in malformed.iter() {
        let record = bridge
            .db()
            .get_owned_key_record(row_id)
            .unwrap()
            .unwrap_or_else(|| panic!("the block is persisted: {row_id}"));
        assert!(record.blocked_fingerprint.is_some(), "the malformed row's block reaches the DB: {row_id}");
        let seams = [
            ("content key", owned_key_and_pin(&bridge.db(), &[7u8; 32], row_id).map(|_| ())),
            (
                "name",
                crate::engine_bridge::meta::owned_metadata_for_name(&bridge.db(), &[7u8; 32], row_id, "edited.txt", None)
                    .map(|_| ()),
            ),
            (
                "rename",
                crate::engine_bridge::meta::owned_metadata_for_rename(&bridge.db(), &[7u8; 32], row_id, "edited.txt", None)
                    .map(|_| ()),
            ),
        ];
        for (seam, result) in seams {
            let error = match result {
                Ok(()) => panic!("a blocked row has no write key ({seam}): {row_id}"),
                Err(error) => error.to_string(),
            };
            assert!(error.contains("item key is unavailable"), "{seam}: {error}");
        }
    }
}

/// P1-1: three well-formed envelope fields on a normal OWNER row come from the
/// server, so only core's `resolve_request_row` success may make it a request
/// row — here the seal does not open, so it stays blocked, never an owner row
/// (review 4 P2-2), and never flips the durable fact.
#[tokio::test]
async fn review_p1_1_a_spoofed_envelope_on_an_owner_row_is_not_a_request_row() {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use beebeeb_core::kdf::{MasterKey, derive_file_key};
    const REQUEST_ID: &str = "0f000000-0000-4000-8000-0000000000e3";
    let (_, requests) = request_envelope_fixture(REQUEST_ID, [0x45u8; 32]);
    let (base, _server) = json_server(move |path| {
        if path == "/api/v1/file-requests" {
            Some(requests.clone())
        } else {
            None
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), base, [7u8; 32]);
    let owner = "4f000000-0000-4000-8000-0000000000e4";
    let master = MasterKey::from_bytes([7; 32]);
    let derived = derive_file_key(&master, owner.as_bytes());
    let name = beebeeb_core::encrypt::encrypt_name_for(
        &derived,
        "plain.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    // The three fields decode, but no seal opens under the request key.
    let row = serde_json::json!({
        "id": owner,
        "name_encrypted": name,
        "parent_id": FOLDER,
        "is_folder": false,
        "version_number": 1,
        "size_bytes": 4,
        "updated_at": 1,
        "file_request_id": REQUEST_ID,
        "sender_ephemeral_pubkey": STANDARD.encode([0x55u8; 32]),
        "wrapped_content_key": STANDARD.encode([0x66u8; 48]),
    });
    let prepared = bridge
        .prepare_owned_rows(&[row])
        .await
        .expect("a well-formed-shaped envelope does not break an owner row");
    assert_eq!(prepared.len(), 1);
    let record = bridge.db().get_owned_key_record(owner).unwrap().expect("the owner row is recorded");
    assert!(!record.request_row, "a spoofed envelope never flips an owner row to request_row");
    assert!(!bridge.is_request_row(owner), "nor the in-memory set");
}

/// P2-2: a row that carries a WELL-FORMED envelope core cannot resolve (the
/// request list did not load, or has no such id) must never fall through to the
/// ordinary owner path. It stays blocked with no derived write key, so a local
/// edit is not sealed as a version under the string key; a later pass that can
/// load the request list resolves it.
#[tokio::test]
async fn review_p2_2_an_unresolvable_envelope_row_never_becomes_an_owner_row() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use beebeeb_core::kdf::{MasterKey, derive_file_key};
    const REQUEST_ID: &str = "0f000000-0000-4000-8000-0000000000e6";
    const ROW: &str = "4f000000-0000-4000-8000-0000000000e6";
    let master = MasterKey::from_bytes([7; 32]);
    let derived = derive_file_key(&master, ROW.as_bytes());
    let name = beebeeb_core::encrypt::encrypt_name_for(
        &derived,
        "report.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    let (envelope, requests) = request_envelope_fixture(REQUEST_ID, [0x46u8; 32]);
    let loaded = std::sync::Arc::new(AtomicBool::new(false));
    let flag = loaded.clone();
    let (base, _server) = json_server(move |path| {
        (path == "/api/v1/file-requests").then(|| {
            if flag.load(Ordering::SeqCst) {
                requests.clone()
            } else {
                serde_json::json!({"file_requests": []})
            }
        })
    });
    let dir = tempfile::tempdir().unwrap();
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), base, [7u8; 32]);
    let mut row = serde_json::json!({
        "id": ROW,
        "name_encrypted": name,
        "parent_id": FOLDER,
        "is_folder": false,
        "version_number": 1,
        "size_bytes": 4,
        "updated_at": 1,
    });
    row.as_object_mut().unwrap().extend(envelope.as_object().unwrap().clone());

    // The request list is withheld. The derived name opens, but the row carries
    // an envelope, so it must not become an owner row.
    let prepared = bridge
        .prepare_owned_rows(&[row.clone()])
        .await
        .expect("one unresolved row never aborts the pass");
    assert_eq!(prepared.len(), 1);
    assert!(!prepared[0].readable, "a carrying-but-unresolved row is blocked");
    let record = bridge.db().get_owned_key_record(ROW).unwrap().expect("the row is recorded");
    assert!(record.blocked_fingerprint.is_some(), "it is blocked, not an owner row");
    assert!(!record.request_row, "core never resolved it");
    // No derived write key: a local edit cannot be sealed as a version under it.
    let err = match owned_key_and_pin(&bridge.db(), &[7u8; 32], ROW) {
        Ok(_) => panic!("an unresolved envelope row has no write key"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("item key is unavailable"), "{err}");
    let err = match crate::engine_bridge::meta::owned_metadata_for_name(
        &bridge.db(),
        &[7u8; 32],
        ROW,
        "edited.txt",
        None,
    ) {
        Ok(_) => panic!("a local edit of an unresolved row is not sealed under the derived key"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("item key is unavailable"), "{err}");

    // A later pass that loads the request list resolves the same row.
    loaded.store(true, Ordering::SeqCst);
    let prepared = bridge.prepare_owned_rows(&[row]).await.expect("the resolved row prepares");
    assert!(prepared[0].readable, "core resolves the row once the list loads");
    let record = bridge.db().get_owned_key_record(ROW).unwrap().expect("the row is recorded");
    assert!(record.request_row, "the durable fact is learned");
    assert!(bridge.is_request_row(ROW));
}

/// P3-1: the durable request fact is sticky against a well-formed envelope the
/// pass cannot resolve. Prepare the valid row (fact true), then the same row
/// with its request key withheld: the record and `is_request_row` still say
/// true. Mutant: clearing the fact when `row_keys` returns None turns it red.
#[tokio::test]
async fn review_p3_1_an_unresolvable_envelope_never_clears_the_fact() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use beebeeb_core::kdf::{MasterKey, derive_file_key};
    const REQUEST_ID: &str = "0f000000-0000-4000-8000-0000000000e7";
    const ROW: &str = "4f000000-0000-4000-8000-0000000000e7";
    let master = MasterKey::from_bytes([7; 32]);
    let derived = derive_file_key(&master, ROW.as_bytes());
    let name = beebeeb_core::encrypt::encrypt_name_for(
        &derived,
        "report.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    let (envelope, requests) = request_envelope_fixture(REQUEST_ID, [0x47u8; 32]);
    let loaded = std::sync::Arc::new(AtomicBool::new(true));
    let flag = loaded.clone();
    let (base, _server) = json_server(move |path| {
        (path == "/api/v1/file-requests").then(|| {
            if flag.load(Ordering::SeqCst) {
                requests.clone()
            } else {
                serde_json::json!({"file_requests": []})
            }
        })
    });
    let dir = tempfile::tempdir().unwrap();
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), base, [7u8; 32]);
    let mut row = json!({
        "id": ROW,
        "name_encrypted": name,
        "parent_id": FOLDER,
        "is_folder": false,
        "version_number": 1,
        "size_bytes": 4,
        "updated_at": 1,
    });
    row.as_object_mut().unwrap().extend(envelope.as_object().unwrap().clone());

    // P3-2: before any record is written, only the in-memory set can carry the
    // fact. A resolution that skips `set_request_row` never reaches the DB (the
    // later `scope.put` would hide the omission), so pin the set itself here.
    let mut scratch = OwnedKeyRecord { file_id: ROW.into(), ..Default::default() };
    let mut cached = None;
    let _resolved = bridge
        .resolve_envelope(&master, &row, &mut cached, &mut scratch, ROW)
        .await
        .expect("core resolves the row");
    assert!(scratch.request_row, "core resolved the request row");
    assert!(bridge.db().get_owned_key_record(ROW).unwrap().is_none(), "nothing is persisted yet");
    assert!(
        bridge.is_request_row(ROW),
        "the in-memory set carries the fact before the DB does"
    );

    let prepared = bridge.prepare_owned_rows(&[row.clone()]).await.expect("the valid row prepares");
    assert!(prepared[0].readable);
    let record = bridge.db().get_owned_key_record(ROW).unwrap().expect("the row is recorded");
    assert!(record.request_row, "core resolved the request row");
    let request_name = record.request_name.expect("core recorded the request name");

    // The request key is withheld on the next pass: unresolved, but the fact stays.
    loaded.store(false, Ordering::SeqCst);
    let prepared = bridge.prepare_owned_rows(&[row]).await.expect("an unresolved row never aborts the pass");
    assert!(!prepared[0].readable, "the unresolved row is blocked");
    let record = bridge.db().get_owned_key_record(ROW).unwrap().expect("the row is still recorded");
    assert!(record.request_row, "an unresolvable envelope never clears the sticky fact");
    assert_eq!(record.request_name, Some(request_name), "an unresolved pass keeps the request name");
    assert!(bridge.is_request_row(ROW), "the in-memory view keeps it too");
    // P1-1: a blocked request row is not writable, not even by a rename.
    let error = match crate::engine_bridge::meta::owned_metadata_for_rename(
        &bridge.db(),
        &[7u8; 32],
        ROW,
        "renamed.txt",
        None,
    ) {
        Ok(_) => panic!("a blocked request row refuses a rename"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("item key is unavailable"), "{error}");
}

/// P2-1: a non-canonical id the SERVER planted as a roster row still gets no
/// derived write key — holding a row under that text proves nothing, and the
/// derived key would be one nobody can open.
///
/// Review 4 P3-3: mutant-proven — deleting the unconditional class bail in
/// `owned_key_and_pin` turns this red.
#[test]
fn review_p2_1_a_server_planted_non_canonical_row_has_no_derived_write_key() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync");
    std::fs::create_dir_all(&sync_root).unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    seed_bridge_row(&bridge, "not-a-uuid", "planted.txt", None, FileStatus::Local, 4);
    let err = match owned_key_and_pin(&bridge.db(), &[7u8; 32], "not-a-uuid") {
        Ok(_) => panic!("a server-planted non-canonical row must have no write key"),
        Err(err) => err,
    };
    assert!(
        err.to_string().contains("item key is unavailable"),
        "a non-canonical row gets no derived write key: {err}"
    );
}

#[test]
fn review_p2_2_a_non_canonical_id_has_no_derived_key() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    for id in [
        "not-a-uuid",
        "40000000-0000-4000-8000-0000000000zz",
        "40000000-0000-4000-8000-0000000000D3",
    ] {
        let err = match owned_key_and_pin(&db, &[7u8; 32], id) {
            Ok(_) => panic!("a non-canonical id must have no write key: {id}"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("item key is unavailable"),
            "a non-canonical id gets no derived write key: {id}: {err}"
        );
    }
}

#[test]
fn review_p2_8_an_unknown_foreign_item_has_no_derived_key() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    let err = match owned_key_and_pin(&db, &[7u8; 32], FOREIGN) {
        Ok(_) => panic!("a foreign item with no record must be unavailable"),
        Err(err) => err,
    };
    assert!(
        err.to_string().contains("item key is unavailable"),
        "a foreign item with no record is unavailable, never derived: {err}"
    );
    let (key, format) = owned_key_and_pin(&db, &[7u8; 32], "40000000-0000-4000-8000-0000000000d3")
        .expect("an owner id still derives its key");
    assert_eq!(format, beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded);
    let _ = key;
}

/// A one-route HTTP fixture: answers each request with the JSON body chosen by
/// its path, and 404 otherwise. Enough for the request-row name fetch.
fn json_server(
    answer: impl Fn(&str) -> Option<serde_json::Value> + Send + 'static,
) -> (String, std::thread::JoinHandle<()>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                }
            }
            let head = String::from_utf8_lossy(&buf);
            let path = head.split_whitespace().nth(1).unwrap_or_default().to_string();
            let body = answer(&path).map(|value| serde_json::to_vec(&value).unwrap());
            let (status, body) = match body {
                Some(body) => ("200 OK", body),
                None => ("404 Not Found", b"{}".to_vec()),
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(&body);
        }
    });
    (base, handle)
}

/// P3-8: a request row whose name is sealed under `C` (the common un-renamed
/// case) must name its edited copy from core's resolved request-row name, not
/// "edited file".
#[tokio::test]
async fn review_p3_8_a_c_sealed_request_row_names_the_copy_from_core() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    use beebeeb_core::opaque::derive_x25519_public;
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    const REQUEST_ID: &str = "0f000000-0000-4000-8000-0000000000e1";
    let master = MasterKey::from_bytes([7; 32]);
    let content = [0x33u8; 32];
    let r_priv = [0x21u8; 32];
    let r_pub = derive_x25519_public(&r_priv);
    let sealed = beebeeb_core::file_request::seal_to_request(&r_pub, b"", &content).unwrap();
    let (wrapped_private, wrap_nonce) =
        beebeeb_core::file_request::wrap_request_private(&master, b"", &r_priv).unwrap();
    let name = beebeeb_core::encrypt::encrypt_name_for(
        &FileKey::from_bytes(content),
        "report.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    let row = serde_json::json!({
        "id": REQUEST,
        "name_encrypted": name,
        "parent_id": FOLDER,
        "is_folder": false,
        "version_number": 1,
        "size_bytes": 4,
        "updated_at": 1,
        "file_request_id": REQUEST_ID,
        "sender_ephemeral_pubkey": STANDARD.encode(sealed.e_pub),
        "wrapped_content_key": STANDARD.encode(&sealed.wrapped_key),
    });
    let requests = serde_json::json!({"file_requests": [{
        "id": REQUEST_ID,
        "wrapped_private_key": STANDARD.encode(&wrapped_private),
        "wrap_nonce": STANDARD.encode(&wrap_nonce),
    }]});
    let (base, _server) = json_server(move |path| {
        if path == "/api/v1/file-requests" {
            Some(requests.clone())
        } else if path == format!("/api/v1/files/{REQUEST}") {
            Some(row.clone())
        } else {
            None
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync");
    std::fs::create_dir_all(&sync_root).unwrap();
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), base, [7u8; 32]);
    seed_folder_row(&bridge, FOLDER, "Inbox");
    seed_bridge_row(&bridge, REQUEST, "Inbox/report.txt", Some(FOLDER), FileStatus::Local, 4);
    bridge
        .db()
        .put_owned_key_record(&OwnedKeyRecord {
            file_id: REQUEST.into(),
            name_encrypted: name.clone(),
            request_row: true,
            ..Default::default()
        })
        .unwrap();
    let payload = dir.path().join("payload.bin");
    std::fs::write(&payload, b"edit").unwrap();
    let op = PendingOperation {
        op_id: "op-p3-8-c".into(),
        kind: OperationKind::UploadVersion,
        file_id: Some(REQUEST.into()),
        parent_id: Some(FOLDER.into()),
        target_path: Some("Inbox/report.txt".into()),
        metadata_json: Some(serde_json::json!({"name_encrypted": name}).to_string()),
        payload_path: Some(payload.to_string_lossy().into_owned()),
        base_version: None,
        base_object_version_id: None,
        attempts: 0,
        max_attempts: 25,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: 0,
        updated_at: 0,
    };
    let mut errors = Vec::new();
    bridge.upload_version(&op, None, &sync_root, &mut errors).await.expect("re-planned");
    let rows = bridge.db().list_files().unwrap();
    assert!(
        rows.iter().any(|row| row.path == "Inbox/report (edited).txt"),
        "the copy keeps the C-sealed row's real name: {rows:?}"
    );
    assert!(
        !rows.iter().any(|row| row.path.contains("edited file")),
        "never the placeholder name: {rows:?}"
    );
}

/// P3-8 (data loss): a file on disk that the scan has not seen yet must not be
/// overwritten by the edited copy; the next free sibling name is used.
#[tokio::test]
async fn review_p3_8_an_unscanned_file_on_disk_is_never_overwritten() {
    use beebeeb_core::kdf::{MasterKey, derive_file_key};
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync");
    std::fs::create_dir_all(sync_root.join("Inbox")).unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    seed_folder_row(&bridge, FOLDER, "Inbox");
    seed_bridge_row(&bridge, REQUEST, "Inbox/report.txt", Some(FOLDER), FileStatus::Local, 4);
    let master = MasterKey::from_bytes([7; 32]);
    let derived = derive_file_key(&master, REQUEST.as_bytes());
    let name = beebeeb_core::encrypt::encrypt_name_for(
        &derived,
        "report.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    bridge
        .db()
        .put_owned_key_record(&OwnedKeyRecord {
            file_id: REQUEST.into(),
            name_encrypted: name.clone(),
            request_row: true,
            ..Default::default()
        })
        .unwrap();
    // On disk, not in the DB: the scan has not seen it yet.
    let unscanned = sync_root.join("Inbox/report (edited).txt");
    std::fs::write(&unscanned, b"someone else's bytes").unwrap();
    let payload = dir.path().join("payload.bin");
    std::fs::write(&payload, b"edit").unwrap();
    let op = PendingOperation {
        op_id: "op-p3-8-disk".into(),
        kind: OperationKind::UploadVersion,
        file_id: Some(REQUEST.into()),
        parent_id: Some(FOLDER.into()),
        target_path: Some("Inbox/report.txt".into()),
        metadata_json: Some(serde_json::json!({"name_encrypted": name}).to_string()),
        payload_path: Some(payload.to_string_lossy().into_owned()),
        base_version: None,
        base_object_version_id: None,
        attempts: 0,
        max_attempts: 25,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: 0,
        updated_at: 0,
    };
    let mut errors = Vec::new();
    bridge.upload_version(&op, None, &sync_root, &mut errors).await.expect("re-planned");
    assert_eq!(
        std::fs::read(&unscanned).unwrap(),
        b"someone else's bytes",
        "the unscanned file is untouched"
    );
    let rows = bridge.db().list_files().unwrap();
    assert!(
        rows.iter().any(|row| row.path == "Inbox/report (edited 2).txt"),
        "the copy takes the next free name: {rows:?}"
    );
}

/// P3-5: a request row has no content key, so the content/name path refuses it;
/// only the rename entry point supplies rename material, from core's durable
/// resolution.
#[test]
fn review_p3_5_a_request_row_has_no_content_key() {
    use crate::engine_bridge::meta::{owned_metadata_for_name, owned_metadata_for_rename};
    use beebeeb_core::chunk_stream::ExpectedChunkFormat;
    use beebeeb_core::kdf::{MasterKey, derive_file_key};

    let dir = tempfile::tempdir().unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    let master = MasterKey::from_bytes([7; 32]);
    let request = "0f000000-0000-4000-8000-0000000000e5";
    let derived = derive_file_key(&master, request.as_bytes());
    let name = beebeeb_core::encrypt::encrypt_name_for(&derived, "report.txt", None, ExpectedChunkFormat::Unrecorded).unwrap();
    bridge
        .db()
        .put_owned_key_record(&OwnedKeyRecord {
            file_id: request.into(),
            name_encrypted: name,
            request_row: true,
            request_name: Some(crate::owner_keys::RequestName {
                key: crate::owner_keys::RequestNameKey::Derived,
                pin: ExpectedChunkFormat::Unrecorded.as_u8(),
            }),
            ..Default::default()
        })
        .unwrap();
    let key = bridge.api().master_key();

    let error = match owned_metadata_for_name(&bridge.db(), key, request, "renamed.txt", None) {
        Ok(_) => panic!("a request row has no content key"),
        Err(error) => format!("{error}"),
    };
    assert!(error.contains("request row"), "{error}");

    // The rename entry point still resolves it: the string-derived key, never C.
    let sealed = owned_metadata_for_rename(&bridge.db(), key, request, "renamed.txt", None).unwrap();
    let renamed = beebeeb_core::encrypt::decrypt_name_metadata_with_key(&derived, &sealed).unwrap();
    assert_eq!(renamed.name, "renamed.txt");
}

/// P3-6: a request row's name is server-side data, and the copy's name must be
/// a safe single path component. `safe_display_name` is the one filter both the
/// stored name and core's resolved name run before `edited_sibling_name`.
#[tokio::test]
async fn review_p3_6_an_unsafe_request_row_name_never_names_the_copy() {
    use beebeeb_core::kdf::{MasterKey, derive_file_key};
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync");
    std::fs::create_dir_all(&sync_root).unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    seed_folder_row(&bridge, FOLDER, "Inbox");
    seed_bridge_row(
        &bridge,
        REQUEST,
        "Inbox/report.txt",
        Some(FOLDER),
        FileStatus::Local,
        4,
    );

    let master = MasterKey::from_bytes([7; 32]);
    let derived = derive_file_key(&master, REQUEST.as_bytes());
    let unsafe_name = beebeeb_core::encrypt::encrypt_name_for(
        &derived,
        "C:evil.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    bridge
        .db()
        .put_owned_key_record(&OwnedKeyRecord {
            file_id: REQUEST.into(),
            name_encrypted: unsafe_name.clone(),
            request_row: true,
            ..Default::default()
        })
        .unwrap();

    let payload = dir.path().join("payload.bin");
    std::fs::write(&payload, b"edit").unwrap();
    let original_disk = sync_root.join("Inbox/report.txt");
    std::fs::create_dir_all(original_disk.parent().unwrap()).unwrap();
    std::fs::write(&original_disk, b"original request bytes").unwrap();
    let op = PendingOperation {
        op_id: "op-request-unsafe-name".into(),
        kind: OperationKind::UploadVersion,
        file_id: Some(REQUEST.into()),
        parent_id: Some(FOLDER.into()),
        target_path: Some("Inbox/report.txt".into()),
        metadata_json: Some(
            serde_json::json!({"name_encrypted": unsafe_name, "content_type": "text/plain"}).to_string(),
        ),
        payload_path: Some(payload.to_string_lossy().into_owned()),
        base_version: None,
        base_object_version_id: None,
        attempts: 0,
        max_attempts: 25,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: 0,
        updated_at: 0,
    };
    let mut errors = Vec::new();
    bridge
        .upload_version(&op, None, &sync_root, &mut errors)
        .await
        .expect("a request-row edit is re-planned, not refused");

    let rows = bridge.db().list_files().unwrap();
    let created: Vec<_> = rows.iter().filter(|row| row.file_id != REQUEST && row.path != "Inbox").collect();
    assert_eq!(created.len(), 1, "one new item: {rows:?}");
    assert_eq!(
        created[0].path, "Inbox/edited file (edited)",
        "the unsafe name never names the copy"
    );
    assert!(
        rows.iter().all(|row| !row.path.contains("C:")),
        "no row is named from the unsafe component: {rows:?}"
    );
    assert!(
        !sync_root.join("Inbox/C:evil.txt").exists(),
        "nothing was written under the unsafe name"
    );
}

/// P3-6: the copy's name must be a safe single path component — the filter that
/// `request_row_display_name` now runs on both the stored name and core's
/// resolved name, exactly as `display_name` does.
#[test]
fn review_p3_6_a_name_that_is_not_a_safe_component_never_names_the_copy() {
    for unsafe_name in ["", " ", "C:evil.txt", "evil/name.txt", "evil\\name.txt", "/abs.txt", "\\\\srv\\share.txt"] {
        assert!(
            !crate::owner_keys::safe_display_name(unsafe_name),
            "{unsafe_name:?} must never name the copy"
        );
    }
    for safe in ["report.txt", "edited file (edited).txt", "report v2.txt"] {
        assert!(crate::owner_keys::safe_display_name(safe), "{safe:?} is a plain name");
    }
}

/// P3-7: the copy's free-name probe must not follow a symlink. A DANGLING link
/// at the first candidate looked free (`exists()` follows the link) and the copy
/// was written through it, landing outside the sync tree. The link now counts as
/// occupied and the copy takes the next sibling name.
#[tokio::test]
async fn review_p3_7_a_dangling_symlink_is_not_a_free_name() {
    use beebeeb_core::kdf::{MasterKey, derive_file_key};
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync");
    std::fs::create_dir_all(sync_root.join("Inbox")).unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    seed_folder_row(&bridge, FOLDER, "Inbox");
    seed_bridge_row(
        &bridge,
        REQUEST,
        "Inbox/report.txt",
        Some(FOLDER),
        FileStatus::Local,
        4,
    );

    let master = MasterKey::from_bytes([7; 32]);
    let derived = derive_file_key(&master, REQUEST.as_bytes());
    let name = beebeeb_core::encrypt::encrypt_name_for(
        &derived,
        "report.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    bridge
        .db()
        .put_owned_key_record(&OwnedKeyRecord {
            file_id: REQUEST.into(),
            name_encrypted: name.clone(),
            request_row: true,
            ..Default::default()
        })
        .unwrap();

    let payload = dir.path().join("payload.bin");
    std::fs::write(&payload, b"edit").unwrap();
    std::fs::write(sync_root.join("Inbox/report.txt"), b"original request bytes").unwrap();
    // The trap: a dangling link where the first copy would land.
    let outside = sync_root.join("outside.txt");
    std::os::unix::fs::symlink(&outside, sync_root.join("Inbox/report (edited).txt")).unwrap();

    let op = PendingOperation {
        op_id: "op-request-dangling".into(),
        kind: OperationKind::UploadVersion,
        file_id: Some(REQUEST.into()),
        parent_id: Some(FOLDER.into()),
        target_path: Some("Inbox/report.txt".into()),
        metadata_json: Some(
            serde_json::json!({"name_encrypted": name, "content_type": "text/plain"}).to_string(),
        ),
        payload_path: Some(payload.to_string_lossy().into_owned()),
        base_version: None,
        base_object_version_id: None,
        attempts: 0,
        max_attempts: 25,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: 0,
        updated_at: 0,
    };
    let mut errors = Vec::new();
    bridge
        .upload_version(&op, None, &sync_root, &mut errors)
        .await
        .expect("a request-row edit is re-planned, not refused");

    assert!(!outside.exists(), "the copy was written through the dangling link");
    let rows = bridge.db().list_files().unwrap();
    let created: Vec<_> = rows.iter().filter(|row| row.file_id != REQUEST && row.path != "Inbox").collect();
    assert_eq!(created.len(), 1, "one new item: {rows:?}");
    assert_eq!(
        created[0].path, "Inbox/report (edited 2).txt",
        "the occupied name is skipped"
    );
    assert_eq!(
        std::fs::read(sync_root.join("Inbox/report (edited 2).txt")).unwrap(),
        b"edit",
        "the copy lands at the next free name"
    );
}
