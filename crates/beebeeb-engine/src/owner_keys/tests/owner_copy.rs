//! Owner-copy custody proofs (tests sf2237d_l8_* and sf2265_*).
//!
//! Owns: the purpose-bound L8 wrap accept/refuse cases, the forged name blob, the legacy copy that
//! proves a batch key but never custody, and the non-approved invite that is never read.
//! Must not: exercise `prepare_owned_rows` (`prepare`) or the invite failure search (`invites`).
//! Tests: the whole file is the test module.

use super::*;

#[test]
fn sf2237d_l8_old_owner_wrap_is_rejected_without_fallback() {
    use beebeeb_core::{
        chunk_stream::ExpectedChunkFormat,
        kdf::{FileKey, MasterKey},
    };
    let master = MasterKey::from_bytes([7; 32]);
    // Canonical version-8 id: the class is Foreign (L12).
    let id = "20000000-0000-8000-8000-000000000001";
    let key = FileKey::from_bytes([91; 32]);
    // The removed format was an empty-AAD frame under derive_file_key(master,id).
    // Build the historical wire bytes through core, never a compatibility reader.
    let derived = beebeeb_core::kdf::derive_file_key(&master, id.as_bytes());
    let old = beebeeb_core::encrypt::encrypt_chunk_raw(&derived, key.as_bytes()).unwrap();
    let encrypted =
        beebeeb_core::encrypt::encrypt_name_for(&key, "legacy.txt", None, ExpectedChunkFormat::Unrecorded).unwrap();
    for pending in [false, true] {
        let record = OwnedKeyRecord {
            file_id: id.into(),
            name_encrypted: encrypted.clone(),
            foreign_added: true,
            owner_wrap: (!pending).then(|| old.clone()),
            pending_owner_wrap: pending.then(|| old.clone()),
            ..Default::default()
        };
        assert!(
            record_key(&master, &record, beebeeb_core::owned_keys::ItemKind::File).is_err(),
            "old owner envelopes must never open or select derived-key fallback"
        );
    }
    // A1: for an owner id the served and persisted envelopes are ignored
    // entirely; only the derived key may authenticate the name.
    let owner = "11111111-1111-4111-8111-111111111111";
    let owner_record = OwnedKeyRecord {
        file_id: owner.into(),
        name_encrypted: encrypted,
        foreign_added: true,
        owner_wrap: Some(old.clone()),
        pending_owner_wrap: Some(old),
        ..Default::default()
    };
    assert!(
        record_key(&master, &owner_record, beebeeb_core::owned_keys::ItemKind::File).is_err(),
        "A1: an owner id never opens a served or persisted envelope"
    );
}

#[test]
fn sf2265_forged_name_blob_owner_copy_is_refused() {
    use beebeeb_core::kdf::MasterKey;
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000001";
    // The folder's own 32-byte name JSON, sealed under the legacy folder key.
    // A server can replay this as an "owner copy" after answering 404.
    let name_json = br#"{"mime_type":null,"name":"Docs"}"#;
    assert_eq!(name_json.len(), 32);
    let derived = beebeeb_core::kdf::derive_file_key(&master, root.as_bytes());
    let forged = STANDARD.encode(beebeeb_core::encrypt::encrypt_chunk_raw(&derived, name_json).unwrap());
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = seen.clone();
    let row = forged.clone();
    let fixture = Fixture::wire(move |req| {
        log.lock().unwrap().push(format!("{} {}", req.method, req.path));
        let (status, body) = if req.path == "/api/v1/shares/invites/sent" {
            (
                200,
                json!({"invites":[{
                    "id":"inv-1","file_id":root,"is_folder_share":true,"status":"approved",
                    "created_at":"2026-10-28T00:00:00Z","encrypted_owner_folder_key":forged
                }]}),
            )
        } else if req.path == "/api/v1/shares/invites/inv-1/folder-keys" {
            (200, json!({"keys":[{"file_id":root,"encrypted_file_key":row}]}))
        } else {
            (404, json!({}))
        };
        Some((status, serde_json::to_vec(&body).unwrap()))
    });
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );
    let error = run(bridge.shared_folder_custody_key(root))
        .err()
        .expect("forged copy must not open");
    assert!(error.to_string().contains("Can't verify"), "{error}");
    assert!(!bridge.db().has_seen_folder_custody(root).unwrap());
    let seen = seen.lock().unwrap().clone();
    assert!(
        seen.iter()
            .any(|line| line == "GET /api/v1/shares/invites/inv-1/folder-keys"),
        "proof rows were read: {seen:?}"
    );
    assert_eq!(
        seen.iter()
            .filter(|line| line.contains("/key") || line.contains("/owner-key"))
            .count(),
        0,
        "no key may be written or pinned: {seen:?}"
    );
}

type OwnerCopyObservation = (Fixture, Arc<Mutex<Vec<String>>>, Arc<Mutex<Vec<Value>>>);

fn owner_copy_test(root: &'static str, invites: Value, folder_keys: Value) -> OwnerCopyObservation {
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let bodies = Arc::new(Mutex::new(Vec::<Value>::new()));
    let stored = Arc::new(Mutex::new(None::<String>));
    let log = seen.clone();
    let body_log = bodies.clone();
    let store = stored.clone();
    let fixture = Fixture::wire(move |req| {
        log.lock().unwrap().push(format!("{} {}", req.method, req.path));
        let (status, body) = if req.path == "/api/v1/shares/invites/sent" {
            (200, invites.clone())
        } else if req.path.ends_with("/folder-keys") {
            (200, folder_keys.clone())
        } else if req.path == format!("/api/v1/shared-folders/{root}/key") && req.method == "PUT" {
            *store.lock().unwrap() = req.body["owner_wrapped_folder_key"].as_str().map(str::to_string);
            body_log.lock().unwrap().push(req.body.clone());
            (200, json!({"created":true,"key_version":1}))
        } else if req.path == format!("/api/v1/shared-folders/{root}") {
            match store.lock().unwrap().clone() {
                Some(wrap) => (
                    200,
                    json!({"folder_id":root,"role":"owner","key_version":1,"owner_wrapped_folder_key":wrap}),
                ),
                None => (404, json!({})),
            }
        } else {
            (404, json!({}))
        };
        Some((status, serde_json::to_vec(&body).unwrap()))
    });
    (fixture, seen, bodies)
}

#[test]
fn sf2265_purpose_bound_owner_copy_opens_and_registers_l7_custody() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000002";
    let folder = FileKey::from_bytes([55; 32]);
    let blob = beebeeb_core::kdf::wrap_invite_owner_copy(&master, root.as_bytes(), &folder).unwrap();
    let mut copy = blob.nonce;
    copy.extend_from_slice(&blob.ciphertext);
    let invites = json!({"invites":[{
        "id":"inv-2","file_id":root,"is_folder_share":true,"status":"approved",
        "created_at":"2026-10-28T00:00:00Z","encrypted_owner_folder_key":STANDARD.encode(&copy)
    }]});
    let (fixture, seen, bodies) = owner_copy_test(root, invites, json!({"keys":[]}));
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );
    // The fixture answers 404 for the custody GET, then accepts the write.
    let key = run(bridge.shared_folder_custody_key(root));
    assert!(
        key.is_ok(),
        "purpose-bound copy must open: {}",
        key.as_ref().err().map(|e| e.to_string()).unwrap_or_default()
    );
    assert_eq!(key.unwrap().as_bytes(), folder.as_bytes());
    let bodies = bodies.lock().unwrap().clone();
    assert_eq!(bodies.len(), 1, "exactly one custody write");
    assert_eq!(bodies[0]["format"], "l7");
    assert_eq!(bodies[0]["key_version"], 1);
    assert!(bridge.db().has_seen_folder_custody(root).unwrap());
    let seen = seen.lock().unwrap().clone();
    assert!(
        !seen.iter().any(|line| line.ends_with("/folder-keys")),
        "a purpose-bound copy needs no proof rows: {seen:?}"
    );
}

#[test]
fn sf2265_legacy_copy_proves_batch_key_but_never_custody() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000003";
    let item = "20000000-0000-4000-8000-000000000001";
    // A real key is not valid UTF-8; core refuses a text candidate (L9 rule 1).
    let folder = FileKey::from_bytes([0x8d; 32]);
    let derived = beebeeb_core::kdf::derive_file_key(&master, root.as_bytes());
    let copy = STANDARD.encode(beebeeb_core::encrypt::encrypt_chunk_raw(&derived, folder.as_bytes()).unwrap());
    let child = beebeeb_core::kdf::derive_file_key(&master, item.as_bytes());
    let wrap = STANDARD.encode(beebeeb_core::encrypt::encrypt_chunk_raw(&folder, child.as_bytes()).unwrap());
    let invites = json!({"invites":[{
        "id":"inv-3","file_id":root,"is_folder_share":true,"status":"approved",
        "created_at":"2026-10-28T00:00:00Z","encrypted_owner_folder_key":copy
    }]});
    let (fixture, _seen, bodies) = owner_copy_test(
        root,
        invites,
        json!({"keys":[{"file_id":item,"encrypted_file_key":wrap}]}),
    );
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );
    // Review P2-1: the proven key is batch-only. It is not custody, so no
    // caller may treat it as one (it would seal index names under a key the
    // server could have chosen).
    let error = run(bridge.shared_folder_custody_key(root))
        .err()
        .expect("a proven legacy key must never be custody");
    assert!(error.to_string().contains("no custody"), "{error}");
    assert_eq!(
        bodies.lock().unwrap().len(),
        0,
        "a legacy key is never registered (L11)"
    );
    assert!(!bridge.db().has_seen_folder_custody(root).unwrap());
}

#[test]
fn sf2265_legacy_copy_with_forged_rows_is_refused() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000004";
    let item = "20000000-0000-4000-8000-000000000002";
    // A real key is not valid UTF-8; core refuses a text candidate (L9 rule 1).
    let folder = FileKey::from_bytes([0x8d; 32]);
    let derived = beebeeb_core::kdf::derive_file_key(&master, root.as_bytes());
    let copy = STANDARD.encode(beebeeb_core::encrypt::encrypt_chunk_raw(&derived, folder.as_bytes()).unwrap());
    // A row that opens under the candidate but carries the wrong child key.
    let wrap = STANDARD.encode(beebeeb_core::encrypt::encrypt_chunk_raw(&folder, &[9u8; 32]).unwrap());
    let invites = json!({"invites":[{
        "id":"inv-4","file_id":root,"is_folder_share":true,"status":"approved",
        "created_at":"2026-10-28T00:00:00Z","encrypted_owner_folder_key":copy
    }]});
    let (fixture, _seen, bodies) = owner_copy_test(
        root,
        invites,
        json!({"keys":[{"file_id":item,"encrypted_file_key":wrap}]}),
    );
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );
    let error = run(bridge.shared_folder_custody_key(root))
        .err()
        .expect("forged rows must not prove");
    assert!(error.to_string().contains("Can't verify"), "{error}");
    assert_eq!(bodies.lock().unwrap().len(), 0);
    assert!(!bridge.db().has_seen_folder_custody(root).unwrap());
}

#[test]
fn sf2265_non_approved_invite_is_never_read() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000005";
    let item = "20000000-0000-4000-8000-000000000003";
    // A real key is not valid UTF-8; core refuses a text candidate (L9 rule 1).
    let folder = FileKey::from_bytes([0x8d; 32]);
    let derived = beebeeb_core::kdf::derive_file_key(&master, root.as_bytes());
    let copy = STANDARD.encode(beebeeb_core::encrypt::encrypt_chunk_raw(&derived, folder.as_bytes()).unwrap());
    let child = beebeeb_core::kdf::derive_file_key(&master, item.as_bytes());
    let wrap = STANDARD.encode(beebeeb_core::encrypt::encrypt_chunk_raw(&folder, child.as_bytes()).unwrap());
    // The pending invite is newer and would open, but only approved copies count.
    let invites = json!({"invites":[
        {"id":"inv-approved","file_id":root,"is_folder_share":true,"status":"approved",
         "created_at":"2026-10-20T00:00:00Z","encrypted_owner_folder_key":copy},
        {"id":"inv-pending","file_id":root,"is_folder_share":true,"status":"invited",
         "created_at":"2026-10-28T00:00:00Z","encrypted_owner_folder_key":copy}
    ]});
    let (fixture, seen, _bodies) = owner_copy_test(
        root,
        invites,
        json!({"keys":[{"file_id":item,"encrypted_file_key":wrap}]}),
    );
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );
    // The approved legacy copy proves the key for a batch only; the pending,
    // newer invite is never read and never becomes custody (P2-1).
    let error = run(bridge.shared_folder_custody_key(root))
        .err()
        .expect("a legacy key is not custody");
    assert!(error.to_string().contains("no custody"), "{error}");
    let seen = seen.lock().unwrap().clone();
    assert!(
        seen.iter()
            .any(|line| line == "GET /api/v1/shares/invites/inv-approved/folder-keys"),
        "{seen:?}"
    );
    assert!(
        !seen.iter().any(|line| line.contains("inv-pending")),
        "a non-approved invite is never read: {seen:?}"
    );
}
