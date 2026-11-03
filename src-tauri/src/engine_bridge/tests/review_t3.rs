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
    let err = match bridge.prepare_owned_rows(&[malformed]).await {
        Ok(_) => panic!("a carrying-but-malformed envelope is an error"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("malformed"), "the row is unreadable: {err}");
    let record = bridge.db().get_owned_key_record(REQUEST).unwrap().expect("the row is still recorded");
    assert!(record.request_row, "the durable fact is never cleared by server data");
    assert_eq!(record.name_encrypted, name, "the unreadable row writes nothing");
    assert!(bridge.is_request_row(REQUEST), "the in-memory set keeps it too");
}

/// P1-1: three well-formed envelope fields on a normal OWNER row come from the
/// server, so only core's `resolve_request_row` success may make it a request
/// row — here the seal does not open, so it stays an ordinary owner row.
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
