//! `prepare_owned_rows` custody repair (tests sf2265_prepare_*).
//!
//! Owns: foreign rows under a forged folder, legacy-proven keys that never pin L8, the swallowed
//! 501 custody put, re-reading the winner after a key-version conflict, asking custody once per
//! folder, and the invite-row proof search order.
//! Must not: run a maintenance pass or fix the search index (`invites`, `index_repair`).
//! Tests: the whole file is the test module.

use super::*;

#[test]
fn sf2265_prepare_blocks_foreign_row_under_forged_folder() {
    use beebeeb_core::kdf::MasterKey;
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000011";
    let item = "20000000-0000-8000-8000-000000000011";
    let invites = json!({"invites":[{
        "id":"inv-11","file_id":root,"is_folder_share":true,"status":"approved",
        "created_at":"2026-10-28T00:00:00Z",
        "encrypted_owner_folder_key":forged_name_blob(&master, root)
    }]});
    let (fixture, seen) = prepare_world(
        root,
        invites,
        json!({"keys":[]}),
        json!({"keys":[],"next_cursor":null}),
        (200, json!({})),
        None,
    );
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        &crate::identity::REAL,
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );
    let prepared = run(bridge.prepare_owned_rows(&[foreign_child(item, root)]))
        .expect("a forged folder must not fail the snapshot");
    assert_eq!(prepared.len(), 1);
    assert!(!prepared[0].readable);
    assert!(prepared[0].record.blocked_fingerprint.is_some());
    assert!(prepared[0].record.pending_owner_wrap.is_none());
    assert_eq!(count(&seen, "/owner-key"), 0, "{seen:?}");
    assert_eq!(count(&seen, &format!("/shared-folders/{root}/key")), 0, "{seen:?}");
    assert!(!bridge.db().has_seen_folder_custody(root).unwrap());
}

#[test]
fn sf2265_prepare_never_pins_l8_under_a_legacy_proven_key() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000012";
    let item = "20000000-0000-8000-8000-000000000012";
    let folder = FileKey::from_bytes([0x8d; 32]);
    // A real key is not valid UTF-8 (L9 rule 1); the child wrap is the proof.
    let child = beebeeb_core::kdf::derive_file_key(&master, item.as_bytes());
    let wrap = STANDARD.encode(beebeeb_core::encrypt::encrypt_chunk_raw(&folder, child.as_bytes()).unwrap());
    // Review 2276 item 1: the fixture must be able to tell the two behaviours
    // apart. The folder's L7 inventory really holds a child wrap bound to this
    // item and this folder key, and the item's name opens under that child key,
    // so a legacy-proven folder key that is (wrongly) treated as custody would
    // resolve the item and pin an L8 envelope under it (5f3fa53).
    let child_wrap =
        STANDARD.encode(beebeeb_sync::key_resolution::wrap_child_key(&folder, root, item, 1, &child).unwrap());
    let sealed = beebeeb_core::encrypt::encrypt_name_for(
        &child,
        "shared.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    let invites = json!({"invites":[{
        "id":"inv-12","file_id":root,"is_folder_share":true,"status":"approved",
        "created_at":"2026-10-28T00:00:00Z",
        "encrypted_owner_folder_key":legacy_copy(&master, root, &folder)
    }]});
    let (fixture, seen) = prepare_world(
        root,
        invites,
        json!({"keys":[{"file_id":item,"encrypted_file_key":wrap}]}),
        json!({"keys":[{"file_id":item,"encrypted_file_key":child_wrap}],"next_cursor":null}),
        (200, json!({})),
        None,
    );
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        &crate::identity::REAL,
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );
    let mut row = foreign_child(item, root);
    row["name_encrypted"] = json!(sealed);
    let prepared = run(bridge.prepare_owned_rows(&[row])).expect("a legacy-proven key must not fail the snapshot");
    assert_eq!(prepared.len(), 1);
    assert!(prepared[0].record.blocked_fingerprint.is_some());
    assert!(prepared[0].record.owner_wrap.is_none());
    assert!(
        prepared[0].record.pending_owner_wrap.is_none(),
        "no L8 envelope under a legacy key"
    );
    assert_eq!(count(&seen, "/owner-key"), 0, "{seen:?}");
    assert_eq!(count(&seen, &format!("/shared-folders/{root}/key")), 0, "{seen:?}");
    assert_eq!(
        count(&seen, &format!("/shared-folders/{root}/keys")),
        0,
        "no L7 child resolution under a legacy key: {seen:?}"
    );
    let error = run(bridge.shared_folder_custody_key(root))
        .err()
        .expect("a legacy-proven key is not custody");
    assert!(error.to_string().contains("no custody"), "{error}");
}

#[test]
fn sf2265_prepare_swallows_a_501_from_the_custody_put() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000013";
    let item = "20000000-0000-8000-8000-000000000013";
    let folder = FileKey::from_bytes([55; 32]);
    let invites = json!({"invites":[{
        "id":"inv-13","file_id":root,"is_folder_share":true,"status":"approved",
        "created_at":"2026-10-28T00:00:00Z",
        "encrypted_owner_folder_key":purpose_bound_copy(&master, root, &folder)
    }]});
    // Exactly the server's feature-gate refusal (feature_gate.rs).
    let (fixture, seen) = prepare_world(
        root,
        invites,
        json!({"keys":[]}),
        json!({"keys":[],"next_cursor":null}),
        (
            501,
            json!({"error":"shared_folders_v1","message":"This server does not support shared folders yet."}),
        ),
        None,
    );
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        &crate::identity::REAL,
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );
    let prepared = run(bridge.prepare_owned_rows(&[foreign_child(item, root)]))
        .unwrap_or_else(|error| panic!("a 501 from the custody PUT must not fail the snapshot: {error}; {seen:?}"));
    assert_eq!(prepared.len(), 1);
    assert!(prepared[0].record.blocked_fingerprint.is_some());
    assert_eq!(
        count(&seen, &format!("PUT /api/v1/shared-folders/{root}/key")),
        1,
        "{seen:?}"
    );
    assert_eq!(
        count(&seen, "/folder-keys"),
        0,
        "never retried as a legacy read: {seen:?}"
    );
    assert_eq!(count(&seen, "/owner-key"), 0, "{seen:?}");
    assert!(!bridge.db().has_seen_folder_custody(root).unwrap());
}

#[test]
fn sf2265_prepare_rereads_the_winner_after_key_version_conflict() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000014";
    let item = "20000000-0000-8000-8000-000000000014";
    let folder = FileKey::from_bytes([55; 32]);
    let winner_key = FileKey::from_bytes([0x44; 32]);
    let invites = json!({"invites":[{
        "id":"inv-14","file_id":root,"is_folder_share":true,"status":"approved",
        "created_at":"2026-10-28T00:00:00Z",
        "encrypted_owner_folder_key":purpose_bound_copy(&master, root, &folder)
    }]});
    let winner = json!({
        "folder_id":root,"role":"owner","key_version":2,
        "owner_wrapped_folder_key":STANDARD.encode(
            beebeeb_sync::key_resolution::wrap_folder_custody(&master, root, &winner_key).unwrap()
        )
    });
    let (fixture, seen) = prepare_world(
        root,
        invites,
        json!({"keys":[]}),
        json!({"keys":[],"next_cursor":null}),
        (
            409,
            json!({"error":"key_version_conflict","message":"The folder key was re-wrapped at a newer version; re-read and retry."}),
        ),
        Some(winner),
    );
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        &crate::identity::REAL,
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );
    let prepared = run(bridge.prepare_owned_rows(&[foreign_child(item, root)]))
        .expect("a lost registration race is re-read, not fatal");
    assert_eq!(prepared.len(), 1);
    assert_eq!(
        count(&seen, &format!("PUT /api/v1/shared-folders/{root}/key")),
        1,
        "{seen:?}"
    );
    assert_eq!(
        exact(&seen, &format!("GET /api/v1/shared-folders/{root}")),
        2,
        "the stale record, then the winner: {seen:?}"
    );
    assert!(bridge.db().has_seen_folder_custody(root).unwrap());
    let custody = run(bridge.shared_folder_custody_key(root)).unwrap();
    assert_eq!(custody.as_bytes(), winner_key.as_bytes());
}

#[test]
fn sf2265_prepare_asks_custody_once_per_folder_not_once_per_row() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000015";
    let folder = FileKey::from_bytes([55; 32]);
    let invites = json!({"invites":[{
        "id":"inv-15","file_id":root,"is_folder_share":true,"status":"approved",
        "created_at":"2026-10-28T00:00:00Z",
        "encrypted_owner_folder_key":purpose_bound_copy(&master, root, &folder)
    }]});
    // A typed failure to remember (review P3-4): without the per-folder cache
    // the batch repeats the GET and the 501 PUT once per row.
    let (fixture, seen) = prepare_world(
        root,
        invites,
        json!({"keys":[]}),
        json!({"keys":[],"next_cursor":null}),
        (
            501,
            json!({"error":"shared_folders_v1","message":"This server does not support shared folders yet."}),
        ),
        None,
    );
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        &crate::identity::REAL,
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );
    let rows: Vec<Value> = (0..3)
        .map(|n| foreign_child(&format!("20000000-0000-8000-8000-00000000002{n}"), root))
        .collect();
    let prepared = run(bridge.prepare_owned_rows(&rows)).unwrap();
    assert_eq!(prepared.len(), 3);
    assert!(prepared.iter().all(|row| !row.readable));
    assert_eq!(
        count(&seen, &format!("PUT /api/v1/shared-folders/{root}/key")),
        1,
        "{seen:?}"
    );
    assert_eq!(
        exact(&seen, &format!("GET /api/v1/shared-folders/{root}")),
        1,
        "{seen:?}"
    );
    assert_eq!(count(&seen, "GET /api/v1/shares/invites/sent"), 1, "{seen:?}");
}

#[test]
fn sf2265_one_invites_missing_rows_does_not_abort_the_proof_search() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000016";
    let item = "20000000-0000-8000-8000-000000000016";
    let folder = FileKey::from_bytes([0x8d; 32]);
    let child = beebeeb_core::kdf::derive_file_key(&master, item.as_bytes());
    let wrap = STANDARD.encode(beebeeb_core::encrypt::encrypt_chunk_raw(&folder, child.as_bytes()).unwrap());
    // The newer invite's rows are gone (a race); the older one still proves.
    let invites = json!({"invites":[
        {"id":"inv-16-new","file_id":root,"is_folder_share":true,"status":"approved",
         "created_at":"2026-10-28T00:00:00Z","encrypted_owner_folder_key":legacy_copy(&master, root, &folder)},
        {"id":"inv-16-old","file_id":root,"is_folder_share":true,"status":"approved",
         "created_at":"2026-10-20T00:00:00Z","encrypted_owner_folder_key":legacy_copy(&master, root, &folder)}
    ]});
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = seen.clone();
    let fixture = Fixture::wire(move |req| {
        log.lock().unwrap().push(format!("{} {}", req.method, req.path));
        let (status, body) = if req.path == "/api/v1/shares/invites/sent" {
            (200, invites.clone())
        } else if req.path == "/api/v1/shares/invites/inv-16-new/folder-keys" {
            (400, json!({"error":"invite_not_approved"}))
        } else if req.path == "/api/v1/shares/invites/inv-16-old/folder-keys" {
            (200, json!({"keys":[{"file_id":item,"encrypted_file_key":wrap}]}))
        } else {
            (404, json!({}))
        };
        Some((status, serde_json::to_vec(&body).unwrap()))
    });
    let db = StateDb::open(":memory:").unwrap();
    let api = fixture.api();
    let stop = AtomicBool::new(false);
    let scope = AccountScope::new(&db, &api, &stop);
    let custody = run(Batch::default().invite_owner_key(&scope, &master, root))
        .expect("a gone invite must not abort the batch")
        .expect("the older approved invite still proves the key");
    assert_eq!(custody.key.as_bytes(), folder.as_bytes());
    assert!(!custody.remember);
    let seen = seen.lock().unwrap().clone();
    assert!(
        seen.iter().any(|line| line.contains("inv-16-new/folder-keys")),
        "{seen:?}"
    );
    assert!(
        seen.iter().any(|line| line.contains("inv-16-old/folder-keys")),
        "{seen:?}"
    );
}

#[test]
fn sf2265_purpose_bound_invites_are_tried_before_newer_legacy_ones() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000017";
    let item = "20000000-0000-8000-8000-000000000017";
    let purpose_key = FileKey::from_bytes([0x51; 32]);
    let legacy_key = FileKey::from_bytes([0x8d; 32]);
    let child = beebeeb_core::kdf::derive_file_key(&master, item.as_bytes());
    let wrap = STANDARD.encode(beebeeb_core::encrypt::encrypt_chunk_raw(&legacy_key, child.as_bytes()).unwrap());
    // The newer invite is legacy and would prove; the older one is
    // purpose-bound and must win (review P3-6).
    let invites = json!({"invites":[
        {"id":"inv-17-legacy","file_id":root,"is_folder_share":true,"status":"approved",
         "created_at":"2026-10-28T00:00:00Z","encrypted_owner_folder_key":legacy_copy(&master, root, &legacy_key)},
        {"id":"inv-17-purposed","file_id":root,"is_folder_share":true,"status":"approved",
         "created_at":"2026-10-20T00:00:00Z","encrypted_owner_folder_key":purpose_bound_copy(&master, root, &purpose_key)}
    ]});
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let bodies = Arc::new(Mutex::new(Vec::<Value>::new()));
    let (log, body_log, stored) = (seen.clone(), bodies.clone(), bodies.clone());
    let fixture = Fixture::wire(move |req| {
        log.lock().unwrap().push(format!("{} {}", req.method, req.path));
        let (status, body) = if req.path == "/api/v1/shares/invites/sent" {
            (200, invites.clone())
        } else if req.path.ends_with("/folder-keys") {
            (200, json!({"keys":[{"file_id":item,"encrypted_file_key":wrap}]}))
        } else if req.path == format!("/api/v1/shared-folders/{root}/key") && req.method == "PUT" {
            body_log.lock().unwrap().push(req.body.clone());
            (200, json!({"created":true,"key_version":1}))
        } else if req.path == format!("/api/v1/shared-folders/{root}") {
            match stored.lock().unwrap().first() {
                Some(body) => (
                    200,
                    json!({"folder_id":root,"role":"owner","key_version":1,
                           "owner_wrapped_folder_key":body["owner_wrapped_folder_key"]}),
                ),
                None => (404, json!({})),
            }
        } else {
            (404, json!({}))
        };
        Some((status, serde_json::to_vec(&body).unwrap()))
    });
    let db = StateDb::open(":memory:").unwrap();
    let api = fixture.api();
    let stop = AtomicBool::new(false);
    let scope = AccountScope::new(&db, &api, &stop);
    let custody = run(Batch::default().invite_owner_key(&scope, &master, root))
        .unwrap()
        .expect("the purpose-bound invite opens");
    assert_eq!(custody.key.as_bytes(), purpose_key.as_bytes());
    assert!(custody.remember, "a purpose-bound copy registers custody");
    assert_eq!(bodies.lock().unwrap().len(), 1);
    assert_eq!(count(&seen, "/folder-keys"), 0, "no legacy read at all: {seen:?}");
}

#[test]
fn a1b_persisted_owner_record_is_cleared_and_never_posted() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let owner = "11111111-1111-4111-8111-111111111111";
    let key = FileKey::from_bytes([0x31; 32]);
    // Pre-L12 code wrapped whatever id it was given; the bytes persist today.
    let envelope =
        beebeeb_sync::key_resolution::wrap_owner_file_key(&master, "30000000-0000-8000-8000-000000000009", &key)
            .unwrap();
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = seen.clone();
    let fixture = Fixture::wire(move |req| {
        log.lock().unwrap().push(format!("{} {}", req.method, req.path));
        Some((404, serde_json::to_vec(&json!({})).unwrap()))
    });
    let db = Arc::new(StateDb::open(":memory:").unwrap());
    db.put_owned_key_record(&OwnedKeyRecord {
        file_id: owner.into(),
        name_encrypted: "opaque-name".into(),
        pending_owner_wrap: Some(envelope.clone()),
        foreign_added: true,
        ..Default::default()
    })
    .unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(&crate::identity::REAL, db.clone(), Arc::new(fixture.api()));
    // A1b: a persisted pre-L12 owner record never adopts.
    assert_eq!(run(bridge.retry_owner_adoptions()).unwrap(), 0);
    assert_eq!(count(&seen, "/owner-key"), 0, "{seen:?}");
    assert!(
        db.get_owned_key_record(owner)
            .unwrap()
            .unwrap()
            .pending_owner_wrap
            .is_none()
    );

    // The prepare pass clears a served envelope too, and never mints one.
    let derived = beebeeb_core::kdf::derive_file_key(&master, owner.as_bytes());
    let name = beebeeb_core::encrypt::encrypt_name_for(
        &derived,
        "notes.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    let row = json!({"id":owner,"name_encrypted":name,"parent_id":null,"is_folder":false,
        "version_number":1,"size_bytes":4,"updated_at":1,
        "acted_via_share_id":"fixture-share","uploaded_by":null,
        "owner_wrapped_file_key":STANDARD.encode(&envelope)});
    let prepared = run(bridge.prepare_owned_rows(&[row])).unwrap();
    assert_eq!(prepared.len(), 1);
    assert!(prepared[0].readable);
    assert!(!prepared[0].record.foreign_added, "the id class, not attribution");
    assert!(prepared[0].record.owner_wrap.is_none());
    assert!(prepared[0].record.pending_owner_wrap.is_none());
    assert_eq!(count(&seen, "/owner-key"), 0, "{seen:?}");

    // A1b: beyond the record fields, the persisted row resolves to the derived
    // key and lists its real name — a class-blind record would not.
    db.put_owned_key_record(&prepared[0].record).unwrap();
    assert_eq!(
        run(bridge.resolved_owned_key(owner)).unwrap().as_bytes(),
        derived.as_bytes()
    );
    assert_eq!(
        crate::owner_keys::stored_name(&db, owner, &master.to_bytes())
            .unwrap()
            .as_deref(),
        Some("notes.txt")
    );
}

#[test]
fn a1_owner_class_ignores_a_member_wrap_and_never_posts() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let owner = "22222222-2222-4222-8222-222222222222";
    let member = FileKey::from_bytes([0x52; 32]);
    let name = beebeeb_core::encrypt::encrypt_name_for(
        &member,
        "member.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    // A member's envelope, replayed against an owner id.
    let member_wrap =
        beebeeb_sync::key_resolution::wrap_owner_file_key(&master, "30000000-0000-8000-8000-00000000000a", &member)
            .unwrap();
    // A1: the owner id with a member's wrap and a name sealed under it.
    let root = "10000000-0000-4000-8000-0000000000a2";
    let folder = FileKey::from_bytes([57; 32]);
    let served = json!({"id":owner,"name_encrypted":name,"parent_id":root,"is_folder":false,
        "version_number":1,"size_bytes":4,"updated_at":1,
        "acted_via_share_id":"fixture-share","uploaded_by":"member",
        "owner_wrapped_file_key":STANDARD.encode(&member_wrap)});
    // A real custody root with an L7 wrap of the member key for the owner id.
    let custody = json!({
        "folder_id":root,"role":"owner","key_version":1,
        "owner_wrapped_folder_key":STANDARD.encode(
            beebeeb_sync::key_resolution::wrap_folder_custody(&master, root, &folder).unwrap()
        )
    });
    let inventory = json!({"keys":[{
        "file_id":owner,
        "encrypted_file_key":STANDARD.encode(
            beebeeb_sync::key_resolution::wrap_child_key(&folder, root, owner, 1, &member).unwrap()
        )
    }],"next_cursor":null});
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = seen.clone();
    let reply = served.clone();
    let fixture = Fixture::wire(move |req| {
        log.lock().unwrap().push(format!("{} {}", req.method, req.path));
        let body = if req.path == "/api/v1/shares/invites/sent" {
            json!({"invites":[]})
        } else if req.path == format!("/api/v1/shared-folders/{root}") {
            custody.clone()
        } else if req.path.starts_with(&format!("/api/v1/shared-folders/{root}/keys")) {
            inventory.clone()
        } else {
            reply.clone()
        };
        Some((200, serde_json::to_vec(&body).unwrap()))
    });
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        &crate::identity::REAL,
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );
    let prepared = run(bridge.prepare_owned_rows(&[served])).expect("A1 must not abort the snapshot");
    assert_eq!(prepared.len(), 1);
    assert!(!prepared[0].readable, "a member key never authenticates an owner id");
    assert!(prepared[0].record.blocked_fingerprint.is_some());
    assert!(prepared[0].record.owner_wrap.is_none());
    assert!(prepared[0].record.pending_owner_wrap.is_none());
    assert_eq!(count(&seen, "/owner-key"), 0, "{seen:?}");
    assert_eq!(
        count(&seen, "/shared-folders"),
        0,
        "an owner row asks no custody: {seen:?}"
    );
    assert!(run(bridge.resolved_owned_key(owner)).is_err());
}
