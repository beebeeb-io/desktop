//! Invite-row failures during the custody proof search (tests sf2276_*).
//!
//! Owns: which invite-row errors abort the search and which fall through (404 gone, no-custody
//! repair, the custody put's 404, and a 501 that does not name the feature).
//! Must not: test the maintenance loop that schedules these passes (`index_repair`).
//! Tests: the whole file is the test module.

use super::*;

#[test]
fn sf2276_invite_rows_failures_that_are_not_gone_abort_the_proof_search() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000018";
    let item = "20000000-0000-8000-8000-000000000018";
    let folder = FileKey::from_bytes([0x8d; 32]);
    let child = beebeeb_core::kdf::derive_file_key(&master, item.as_bytes());
    let wrap = STANDARD.encode(beebeeb_core::encrypt::encrypt_chunk_raw(&folder, child.as_bytes()).unwrap());
    let invites = json!({"invites":[
        {"id":"inv-18-new","file_id":root,"is_folder_share":true,"status":"approved",
         "created_at":"2026-10-28T00:00:00Z","encrypted_owner_folder_key":legacy_copy(&master, root, &folder)},
        {"id":"inv-18-old","file_id":root,"is_folder_share":true,"status":"approved",
         "created_at":"2026-10-20T00:00:00Z","encrypted_owner_folder_key":legacy_copy(&master, root, &folder)}
    ]});
    // Review P3-A: only the server's "gone / not approved" answers on the
    // proof rows (400 invite-not-approved, 404 invite missing, invites.rs) may
    // skip an approved invite. Auth failures, other client errors and server
    // errors are the batch's failure, never a reason to read on.
    for status in [401u16, 403, 500] {
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let (log, invites, wrap) = (seen.clone(), invites.clone(), wrap.clone());
        let fixture = Fixture::wire(move |req| {
            log.lock().unwrap().push(format!("{} {}", req.method, req.path));
            let (code, body) = if req.path == "/api/v1/shares/invites/sent" {
                (200, invites.clone())
            } else if req.path == "/api/v1/shares/invites/inv-18-new/folder-keys" {
                (status, json!({"error":"unexpected"}))
            } else if req.path == "/api/v1/shares/invites/inv-18-old/folder-keys" {
                (200, json!({"keys":[{"file_id":item,"encrypted_file_key":wrap}]}))
            } else {
                (404, json!({}))
            };
            Some((code, serde_json::to_vec(&body).unwrap()))
        });
        let db = StateDb::open(":memory:").unwrap();
        let api = fixture.api();
        let stop = AtomicBool::new(false);
        let scope = AccountScope::new(&db, &api, &stop);
        let error = run(Batch::default().invite_owner_key(&scope, &master, root))
            .err()
            .unwrap_or_else(|| panic!("a {status} from the proof rows must abort the batch"));
        assert!(error.to_string().contains(&status.to_string()), "{status}: {error}");
        let seen = seen.lock().unwrap().clone();
        assert!(
            !seen.iter().any(|line| line.contains("inv-18-old")),
            "a {status} is not 'try the next invite': {seen:?}"
        );
    }
}

#[test]
fn sf2276_a_404_from_a_gone_invite_still_tries_the_next_one() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000019";
    let item = "20000000-0000-8000-8000-000000000019";
    let folder = FileKey::from_bytes([0x8d; 32]);
    let child = beebeeb_core::kdf::derive_file_key(&master, item.as_bytes());
    let wrap = STANDARD.encode(beebeeb_core::encrypt::encrypt_chunk_raw(&folder, child.as_bytes()).unwrap());
    let invites = json!({"invites":[
        {"id":"inv-19-new","file_id":root,"is_folder_share":true,"status":"approved",
         "created_at":"2026-10-28T00:00:00Z","encrypted_owner_folder_key":legacy_copy(&master, root, &folder)},
        {"id":"inv-19-old","file_id":root,"is_folder_share":true,"status":"approved",
         "created_at":"2026-10-20T00:00:00Z","encrypted_owner_folder_key":legacy_copy(&master, root, &folder)}
    ]});
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = seen.clone();
    let fixture = Fixture::wire(move |req| {
        log.lock().unwrap().push(format!("{} {}", req.method, req.path));
        let (status, body) = if req.path == "/api/v1/shares/invites/sent" {
            (200, invites.clone())
        } else if req.path == "/api/v1/shares/invites/inv-19-new/folder-keys" {
            (404, json!({"error":"not_found"}))
        } else if req.path == "/api/v1/shares/invites/inv-19-old/folder-keys" {
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
        .expect("a 404 from one invite must not abort the batch")
        .expect("the older approved invite still proves the key");
    assert_eq!(custody.key.as_bytes(), folder.as_bytes());
    assert!(!custody.remember);
    let seen = seen.lock().unwrap().clone();
    assert!(
        seen.iter().any(|line| line.contains("inv-19-old/folder-keys")),
        "{seen:?}"
    );
}

#[test]
fn sf2276_a_no_custody_repair_stops_retrying_the_index() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000020";
    let item = "20000000-0000-8000-8000-000000000020";
    let folder = FileKey::from_bytes([0x8d; 32]);
    let child = beebeeb_core::kdf::derive_file_key(&master, item.as_bytes());
    let wrap = STANDARD.encode(beebeeb_core::encrypt::encrypt_chunk_raw(&folder, child.as_bytes()).unwrap());
    let invites = json!({"invites":[{
        "id":"inv-20","file_id":root,"is_folder_share":true,"status":"approved",
        "created_at":"2026-10-28T00:00:00Z",
        "encrypted_owner_folder_key":legacy_copy(&master, root, &folder)
    }]});
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = seen.clone();
    let fixture = Fixture::wire(move |req| {
        log.lock().unwrap().push(format!("{} {}", req.method, req.path));
        let (status, body) = if req.path == "/api/v1/shares/invites/sent" {
            (200, invites.clone())
        } else if req.path.ends_with("/folder-keys") {
            (200, json!({"keys":[{"file_id":item,"encrypted_file_key":wrap}]}))
        } else {
            (404, json!({}))
        };
        Some((status, serde_json::to_vec(&body).unwrap()))
    });
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(StateDb::open(temp.path().join("db")).unwrap());
    let bridge = crate::engine_bridge::EngineBridge::new(db.clone(), Arc::new(fixture.api()));
    db.mark_shared_index_dirty(root).unwrap();
    run(bridge.maintain_owned_keys(1_000)).unwrap();
    let after_first = seen.lock().unwrap().len();
    assert!(after_first > 0, "{seen:?}");
    // Review P3-B: a legacy-proven folder has no custody, and no retry can
    // change that before its owner upgrades. The maintenance pass clears the
    // dirty marker at the revision it read instead of asking again every 30 s.
    assert_eq!(
        db.shared_index_dirty_revision(root).unwrap(),
        None,
        "the index marker is cleared: {seen:?}"
    );
    run(bridge.maintain_owned_keys(2_000)).unwrap();
    assert_eq!(
        seen.lock().unwrap().len(),
        after_first,
        "a second pass makes no requests: {seen:?}"
    );
}

#[test]
fn sf2276_a_404_from_the_custody_put_does_not_abort_the_snapshot() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000021";
    let item = "20000000-0000-8000-8000-000000000021";
    let folder = FileKey::from_bytes([55; 32]);
    let invites = json!({"invites":[{
        "id":"inv-21","file_id":root,"is_folder_share":true,"status":"approved",
        "created_at":"2026-10-28T00:00:00Z",
        "encrypted_owner_folder_key":purpose_bound_copy(&master, root, &folder)
    }]});
    // Review P3-C: the server answers 404 from the custody PUT when the folder
    // is missing, not owned, or trashed (shared_folders.rs:293-304) and when
    // the stored row is gone or owned by someone else (:650). A snapshot can
    // hold a live row under a trashed folder (its query has no ancestor
    // check), so this is a typed no-custody, not a batch failure.
    let (fixture, seen) = prepare_world(
        root,
        invites,
        json!({"keys":[]}),
        json!({"keys":[],"next_cursor":null}),
        (404, json!({"error":"not_found"})),
        None,
    );
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );
    let rows = vec![
        foreign_child(item, root),
        foreign_child("20000000-0000-8000-8000-000000000221", root),
    ];
    let prepared = run(bridge.prepare_owned_rows(&rows))
        .unwrap_or_else(|error| panic!("a 404 from the custody PUT must not abort the snapshot: {error}; {seen:?}"));
    assert_eq!(prepared.len(), 2);
    assert!(prepared.iter().all(|row| !row.readable));
    assert!(prepared.iter().all(|row| row.record.blocked_fingerprint.is_some()));
    assert!(prepared.iter().all(|row| row.record.pending_owner_wrap.is_none()));
    assert_eq!(
        count(&seen, &format!("PUT /api/v1/shared-folders/{root}/key")),
        1,
        "one typed no-custody per folder, not per row: {seen:?}"
    );
    assert_eq!(
        exact(&seen, &format!("GET /api/v1/shared-folders/{root}")),
        1,
        "{seen:?}"
    );
    assert_eq!(
        count(&seen, "/folder-keys"),
        0,
        "a typed no-custody is never retried as a legacy read: {seen:?}"
    );
    assert_eq!(count(&seen, "/owner-key"), 0, "{seen:?}");
    assert!(!bridge.db().has_seen_folder_custody(root).unwrap());
}

#[test]
fn sf2276_a_501_that_does_not_name_the_feature_stays_an_error() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000022";
    let item = "20000000-0000-8000-8000-000000000022";
    let folder = FileKey::from_bytes([55; 32]);
    let invites = json!({"invites":[{
        "id":"inv-22","file_id":root,"is_folder_share":true,"status":"approved",
        "created_at":"2026-10-28T00:00:00Z",
        "encrypted_owner_folder_key":purpose_bound_copy(&master, root, &folder)
    }]});
    // Review P3-D: the 501 that means "no custody here" is the shared-folders
    // feature gate, and it names itself in its body. Any other 501 is a server
    // failure the snapshot must surface.
    let (fixture, seen) = prepare_world(
        root,
        invites,
        json!({"keys":[]}),
        json!({"keys":[],"next_cursor":null}),
        (
            501,
            json!({"error":"not_implemented_here","message":"some other refusal"}),
        ),
        None,
    );
    let temp = tempfile::tempdir().unwrap();
    let bridge = crate::engine_bridge::EngineBridge::new(
        Arc::new(StateDb::open(temp.path().join("db")).unwrap()),
        Arc::new(fixture.api()),
    );
    let error = run(bridge.prepare_owned_rows(&[foreign_child(item, root)]))
        .err()
        .unwrap_or_else(|| panic!("only the feature gate's own 501 is 'no custody': {seen:?}"));
    assert!(error.to_string().contains("not_implemented_here"), "{error}");
}
