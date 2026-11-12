//! T3.2 (task 2275, L12/L13): 2236 request rows on the desktop.
//!
//! Owns: a request row's keys — content under the sealed request key `C`, name
//! under the derived string convention first — and the read-only, never-adopted
//! rule.
//! Must not: assert anything about other row classes.
//! Tests: `a9c_`.

use super::requests::{RequestName, RequestNameKey};
use super::*;
use beebeeb_core::chunk_stream::ExpectedChunkFormat;

const OWNER: &str = "40000000-0000-4000-8000-00000000000b";
const REQUEST: &str = "0f000000-0000-4000-8000-000000000001";

/// A request row whose name an old client re-sealed under the derived (string)
/// convention while its content stayed under `C` (A9c).
#[test]
fn a9c_request_row_keeps_content_under_c_and_name_under_derived() {
    use beebeeb_core::kdf::{MasterKey, derive_file_key, owner_file_key_for_name};
    use beebeeb_core::opaque::derive_x25519_public;

    let master = MasterKey::from_bytes([7; 32]);
    let content = [0x77u8; 32];
    let r_priv = [0x21u8; 32];
    let r_pub = derive_x25519_public(&r_priv);
    // Pre-2236: the uploader seals with the empty id, the owner wraps with the
    // empty context. Both are the R5 legacy shape core cannot open.
    let sealed = beebeeb_core::file_request::seal_to_request(&r_pub, b"", &content).unwrap();
    let (wrapped_private, wrap_nonce) =
        beebeeb_core::file_request::wrap_request_private(&master, b"", &r_priv).unwrap();

    let derived = derive_file_key(&master, OWNER.as_bytes());
    let name = beebeeb_core::encrypt::encrypt_name_for(
        &derived,
        "report.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    let row = json!({
        "id": OWNER,
        "name_encrypted": name,
        "parent_id": null,
        "is_folder": false,
        "version_number": 1,
        "size_bytes": 4,
        "updated_at": 1,
        "acted_via_share_id": "fixture-share",
        "uploaded_by": null,
        "file_request_id": REQUEST,
        "sender_ephemeral_pubkey": STANDARD.encode(sealed.e_pub),
        "wrapped_content_key": STANDARD.encode(&sealed.wrapped_key),
    });

    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = seen.clone();
    let served = row.clone();
    let requests = json!({"file_requests": [{
        "id": REQUEST,
        "wrapped_private_key": STANDARD.encode(&wrapped_private),
        "wrap_nonce": STANDARD.encode(&wrap_nonce),
    }]});
    let fixture = Fixture::wire(move |req| {
        log.lock().unwrap().push(format!("{} {}", req.method, req.path));
        let body = if req.path.contains("/file-requests") {
            requests.clone()
        } else {
            served.clone()
        };
        Some((200, serde_json::to_vec(&body).unwrap()))
    });
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        &crate::identity::REAL,
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );

    let prepared = run(bridge.prepare_owned_rows(&[row.clone()])).unwrap();
    assert_eq!(prepared.len(), 1);
    assert!(prepared[0].readable, "the derived name authenticates");
    assert!(!prepared[0].record.foreign_added);
    assert!(
        prepared[0].record.pending_owner_wrap.is_none(),
        "a request row is never adopted"
    );
    assert!(prepared[0].record.owner_wrap.is_none());

    // The name is the derived convention's, so the display path works offline.
    assert_eq!(
        crate::owner_keys::display_name(&row, OWNER, bridge.api().master_key()).as_deref(),
        Some("report.txt")
    );
    assert!(matches!(
        owner_file_key_for_name(&master, OWNER, row["name_encrypted"].as_str().unwrap()).map(|(_, c)| c),
        Ok(beebeeb_core::kdf::OwnerKeyConvention::String)
    ));

    // The content key is the sealed request key `C`, never the derived one.
    let resolved = run(bridge.request_content_key(&row)).unwrap();
    assert_eq!(resolved.as_bytes(), &content);
    assert_ne!(resolved.as_bytes(), derived.as_bytes());

    // Read-only: nothing was adopted or published.
    assert_eq!(count(&seen, "/owner-key"), 0, "{seen:?}");
}

/// The a9c row: content under `C`, name under the derived string convention,
/// with the pre-2236 empty-context wraps.
fn request_world(
    content: [u8; 32],
    name_key: &beebeeb_core::kdf::FileKey,
    plaintext: &[u8],
    pin: ExpectedChunkFormat,
) -> (Fixture, Arc<Mutex<Vec<String>>>, Value) {
    use beebeeb_core::opaque::derive_x25519_public;

    let master = MasterKey::from_bytes([7; 32]);
    let r_priv = [0x21u8; 32];
    let r_pub = derive_x25519_public(&r_priv);
    let sealed = beebeeb_core::file_request::seal_to_request(&r_pub, b"", &content).unwrap();
    let (wrapped_private, wrap_nonce) =
        beebeeb_core::file_request::wrap_request_private(&master, b"", &r_priv).unwrap();
    let name = beebeeb_core::encrypt::encrypt_name_for(name_key, "report.txt", None, pin).unwrap();
    let row = json!({
        "id": OWNER,
        "name_encrypted": name,
        "parent_id": null,
        "is_folder": false,
        "version_number": 1,
        "size_bytes": plaintext.len(),
        "chunk_count": 1,
        "chunk_size_bytes": plaintext.len(),
        "updated_at": 1,
        "acted_via_share_id": "fixture-share",
        "uploaded_by": null,
        "file_request_id": REQUEST,
        "sender_ephemeral_pubkey": STANDARD.encode(sealed.e_pub),
        "wrapped_content_key": STANDARD.encode(&sealed.wrapped_key),
    });
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = seen.clone();
    let served = row.clone();
    let requests = json!({"file_requests": [{
        "id": REQUEST,
        "wrapped_private_key": STANDARD.encode(&wrapped_private),
        "wrap_nonce": STANDARD.encode(&wrap_nonce),
    }]});
    let wire = {
        let key = beebeeb_core::kdf::FileKey::from_bytes(content);
        let blob = beebeeb_core::encrypt::encrypt_chunk(&key, plaintext).unwrap();
        let mut wire = blob.nonce.clone();
        wire.extend_from_slice(&blob.ciphertext);
        wire
    };
    let fixture = Fixture::wire(move |req| {
        log.lock().unwrap().push(format!("{} {}", req.method, req.path));
        if req.path.contains("/file-requests") {
            Some((200, serde_json::to_vec(&requests).unwrap()))
        } else if req.path.ends_with("/chunks/0") {
            Some((200, wire.clone()))
        } else if req.path == format!("/api/v1/files/{OWNER}") {
            Some((200, serde_json::to_vec(&served).unwrap()))
        } else {
            Some((404, b"{}".to_vec()))
        }
    });
    (fixture, seen, row)
}

#[test]
fn review_p2_2_request_keys_load_once_per_bridge() {
    let derived = beebeeb_core::kdf::derive_file_key(&MasterKey::from_bytes([7; 32]), OWNER.as_bytes());
    let (fixture, seen, row) = request_world([0x77; 32], &derived, b"hello", ExpectedChunkFormat::Unrecorded);
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        &crate::identity::REAL,
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );
    let first = run(bridge.request_row_keys(&row)).unwrap();
    let second = run(bridge.request_row_keys(&row)).unwrap();
    assert_eq!(first.content_key.as_bytes(), [0x77; 32].as_slice());
    assert_eq!(second.content_key.as_bytes(), first.content_key.as_bytes());
    assert_eq!(
        count(&seen, "/file-requests"),
        1,
        "the request list is loaded once and cached on the bridge: {seen:?}"
    );
}

#[test]
fn review_p2_3_a_request_row_hydrates_whole_and_by_range() {
    use crate::state_db::{FileEntry, FileStatus, ItemKind};

    let master = MasterKey::from_bytes([7; 32]);
    let derived = beebeeb_core::kdf::derive_file_key(&master, OWNER.as_bytes());
    let plaintext = b"hello range".to_vec();
    let (fixture, seen, row) = request_world([0x77; 32], &derived, &plaintext, ExpectedChunkFormat::Unrecorded);
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(StateDb::open(temp.path().join("db")).unwrap());
    let entry = FileEntry {
        file_id: OWNER.into(),
        path: "report.txt".into(),
        status: FileStatus::CloudOnly,
        size_bytes: plaintext.len() as i64,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 0,
        parent_id: None,
        item_kind: ItemKind::File,
    };
    db.upsert_file_with_contract(&entry, |_| {}).unwrap();
    // The durable L12 fact: the row is a request upload, so its content key is
    // `C` and its name key the derived convention.
    db.put_owned_key_record(&OwnedKeyRecord {
        file_id: OWNER.into(),
        name_encrypted: row["name_encrypted"].as_str().unwrap().into(),
        request_row: true,
        ..Default::default()
    })
    .unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(&crate::identity::REAL, db.clone(), Arc::new(fixture.api()));

    let dest = temp.path().join("report.txt");
    run(bridge.hydrate_file(OWNER, &dest, &[temp.path()])).unwrap();
    assert_eq!(
        std::fs::read(&dest).unwrap(),
        plaintext,
        "the whole file decrypts under C with the name key"
    );

    let range = run(bridge.hydrate_file_range(OWNER, 6, 5))
        .unwrap()
        .expect("a range is served");
    let start = (6 - range.range_start_bytes) as usize;
    assert_eq!(
        &range.data[start..start + 5],
        b"range",
        "the range reads under C with the name key"
    );
    assert!(range.covers_whole_file);
    assert_eq!(count(&seen, "/owner-key"), 0, "{seen:?}");
}

/// P2-2 (L13/§15): the first rename of a request row, whose name the uploader
/// sealed under `C`. The rename seals under the string-derived key with the pin
/// core read from that `C`-sealed name, and the new name never opens under `C`.
#[test]
fn review_p2_2_a_c_sealed_request_row_renames_under_the_string_key() {
    use crate::engine_bridge::{FinderWriteItemKind, FinderWriteTarget};
    use beebeeb_core::kdf::derive_file_key;

    let master = MasterKey::from_bytes([7; 32]);
    let content = [0x77u8; 32];
    let c_key = beebeeb_core::kdf::FileKey::from_bytes(content);
    let (fixture, _seen, row) = request_world(content, &c_key, b"hello", ExpectedChunkFormat::V2);
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(StateDb::open(temp.path().join("db")).unwrap());
    db.upsert_file(&crate::state_db::FileEntry {
        file_id: OWNER.into(),
        path: "report.txt".into(),
        status: crate::state_db::FileStatus::CloudOnly,
        size_bytes: 5,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 0,
        parent_id: None,
        item_kind: crate::state_db::ItemKind::File,
    })
    .unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(&crate::identity::REAL, db.clone(), Arc::new(fixture.api()));

    // The real ingest: core opens the `C`-sealed name and the pin it records is
    // kept on the durable record.
    let prepared = run(bridge.prepare_owned_rows(&[row.clone()])).unwrap();
    assert!(prepared[0].readable, "core opens the name under C");
    assert_eq!(
        prepared[0].record.request_name,
        Some(RequestName {
            key: RequestNameKey::Content,
            pin: ExpectedChunkFormat::V2.as_u8(),
        }),
        "core's resolution of the name is durable"
    );

    bridge
        .queue_finder_modify(FinderWriteTarget {
            file_id: Some(OWNER.into()),
            parent_id: None,
            filename: "renamed.txt".into(),
            rel_path: None,
            kind: FinderWriteItemKind::File,
            contents_path: None,
            content_type: None,
            base_version_identifier: None,
        })
        .unwrap();

    let op = bridge.db().list_operations_for_file(OWNER).unwrap().remove(0);
    let payload: Value = serde_json::from_str(op.metadata_json.as_deref().unwrap()).unwrap();
    let name_encrypted = payload["name_encrypted"].as_str().unwrap();
    let renamed = beebeeb_core::encrypt::decrypt_name_metadata_with_key(
        &derive_file_key(&master, OWNER.as_bytes()),
        name_encrypted,
    )
    .expect("the rename seals under the string-derived key");
    assert_eq!(renamed.name, "renamed.txt");
    assert_eq!(
        renamed.chunk_format,
        ExpectedChunkFormat::V2,
        "the pin core read is kept"
    );
    assert!(
        beebeeb_core::encrypt::decrypt_name_metadata_with_key(&c_key, name_encrypted).is_err(),
        "the new name does not open under C"
    );
}

/// The files row a rename needs, seeded for `id` at `path`.
fn seed_rename_row(db: &StateDb, id: &str, path: &str) {
    db.upsert_file(&crate::state_db::FileEntry {
        file_id: id.into(),
        path: path.into(),
        status: crate::state_db::FileStatus::CloudOnly,
        size_bytes: 5,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 0,
        parent_id: None,
        item_kind: crate::state_db::ItemKind::File,
    })
    .unwrap();
}

/// The queued rename's sealed name for `id`.
fn queued_rename_name(bridge: &crate::engine_bridge::EngineBridge, id: &str) -> String {
    use crate::engine_bridge::{FinderWriteItemKind, FinderWriteTarget};
    bridge
        .queue_finder_modify(FinderWriteTarget {
            file_id: Some(id.into()),
            parent_id: None,
            filename: "renamed.txt".into(),
            rel_path: None,
            kind: FinderWriteItemKind::File,
            contents_path: None,
            content_type: None,
            base_version_identifier: None,
        })
        .unwrap();
    let op = bridge.db().list_operations_for_file(id).unwrap().remove(0);
    let payload: Value = serde_json::from_str(op.metadata_json.as_deref().unwrap()).unwrap();
    payload["name_encrypted"].as_str().unwrap().to_string()
}

/// P3-2: a request row whose name authenticates under the derived string
/// convention renames under that same string key, carrying the pin its name
/// records.
#[test]
fn review_p3_2_a_derived_name_request_row_renames_with_its_pin() {
    use beebeeb_core::kdf::derive_file_key;

    let master = MasterKey::from_bytes([7; 32]);
    let derived = derive_file_key(&master, OWNER.as_bytes());
    let (fixture, _seen, row) = request_world([0x77; 32], &derived, b"hello", ExpectedChunkFormat::V2);
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(StateDb::open(temp.path().join("db")).unwrap());
    seed_rename_row(&db, OWNER, "report.txt");
    let bridge = crate::engine_bridge::EngineBridge::new(&crate::identity::REAL, db, Arc::new(fixture.api()));
    let prepared = run(bridge.prepare_owned_rows(&[row.clone()])).unwrap();
    assert_eq!(
        prepared[0].record.request_name,
        Some(RequestName {
            key: RequestNameKey::Derived,
            pin: ExpectedChunkFormat::V2.as_u8(),
        })
    );

    let name_encrypted = queued_rename_name(&bridge, OWNER);
    let renamed = beebeeb_core::encrypt::decrypt_name_metadata_with_key(&derived, &name_encrypted)
        .expect("the rename seals under the derived string key");
    assert_eq!(renamed.name, "renamed.txt");
    assert_eq!(
        renamed.chunk_format,
        ExpectedChunkFormat::V2,
        "the pin the name records is kept"
    );
}

/// P3-2: the rename contract for every name origin core can report — the
/// string-derived key with the pin core read, `C` included, a binary-derived
/// name too — and a refusal for an upper-repair (read-only) or an unresolved
/// name.
#[test]
fn review_p3_2_rename_material_takes_the_string_key_and_pin_for_every_origin() {
    use beebeeb_core::kdf::derive_file_key;

    let master = MasterKey::from_bytes([7; 32]);
    let string = derive_file_key(&master, OWNER.as_bytes());
    let record = |name: Option<RequestName>| OwnedKeyRecord {
        file_id: OWNER.into(),
        request_row: true,
        request_name: name,
        ..Default::default()
    };
    for (key, pin) in [
        (RequestNameKey::Derived, 2u8),
        (RequestNameKey::DerivedBinary, 0),
        (RequestNameKey::Content, 2),
    ] {
        let (found, format) =
            crate::owner_keys::rename_material(&master, OWNER, &record(Some(RequestName { key, pin })))
                .expect("a writable origin renames");
        assert_eq!(
            found.as_bytes(),
            string.as_bytes(),
            "{key:?} renames under the string key"
        );
        assert_eq!(format.as_u8(), pin, "{key:?} keeps the pin its name records");
    }
    let upper = record(Some(RequestName {
        key: RequestNameKey::UpperRepair,
        pin: 0,
    }));
    let error = match crate::owner_keys::rename_material(&master, OWNER, &upper) {
        Ok(_) => panic!("an upper-repair name is read-only"),
        Err(error) => format!("{error}"),
    };
    assert!(error.contains("read-only"), "{error}");
    assert!(
        crate::owner_keys::rename_material(&master, OWNER, &record(None)).is_err(),
        "a name that was never resolved on this device fails closed"
    );
}
