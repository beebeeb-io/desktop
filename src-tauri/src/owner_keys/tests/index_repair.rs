//! Index repair waiting for custody (tests sf2279_*).
//!
//! Owns: the deferred index work that resumes when custody appears, an upgrade-required folder that
//! is not retried every pass, a seen custody 404 that blocks only its row, a seen custody root that
//! keeps its dirty marker, a custody write that arrives mid-pass, and reviving a deferred root.
//! Must not: decide which invite row fails the proof search (`invites`).
//! Tests: the whole file is the test module.

use super::*;

/// Review 2279 item 1: index work that cannot run before custody is not
/// dropped. It is deferred durably, makes no request on later passes, and the
/// custody GET that finally opens revives it even though the rows never change.
#[test]
fn sf2279_index_repair_waits_for_custody_and_resumes_when_it_appears() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000030";
    let item = "20000000-0000-8000-8000-000000000030";
    let folder = FileKey::from_bytes([55; 32]);
    let child = FileKey::from_bytes([66; 32]);
    let child_wrap =
        STANDARD.encode(beebeeb_sync::key_resolution::wrap_child_key(&folder, root, item, 1, &child).unwrap());
    let item_name = beebeeb_core::encrypt::encrypt_name_for(
        &child,
        "holiday.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    let row = json!({"id":item,"name_encrypted":item_name,"parent_id":root,"is_folder":false,
        "version_number":1,"size_bytes":4,"updated_at":1,
        "acted_via_share_id":"fixture-share","uploaded_by":null,
        "owner_wrapped_file_key":null});
    let available = Arc::new(Mutex::new(false));
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let (log, ready) = (seen.clone(), available.clone());
    let served_row = row.clone();
    let fixture = Fixture::wire(move |req| {
        log.lock().unwrap().push(format!("{} {}", req.method, req.path));
        let ready = *ready.lock().unwrap();
        let (status, body) = if req.path == "/api/v1/shares/invites/sent" {
            (200, json!({"invites":[]}))
        } else if req.path == format!("/api/v1/shared-folders/{root}") && !ready {
            (404, json!({}))
        } else if req.path == format!("/api/v1/shared-folders/{root}") {
            (
                200,
                json!({"folder_id":root,"role":"owner","key_version":1,
                "owner_wrapped_folder_key":STANDARD.encode(
                    beebeeb_sync::key_resolution::wrap_folder_custody(&master, root, &folder).unwrap())}),
            )
        } else if req.path.starts_with(&format!("/api/v1/shared-folders/{root}/keys")) {
            (
                200,
                json!({"keys":[{"file_id":item,"encrypted_file_key":child_wrap}],"next_cursor":null}),
            )
        } else if req.path == format!("/api/v1/files/{item}") {
            (200, served_row.clone())
        } else if req.path.starts_with("/api/v1/files?") {
            (200, json!({"files":[served_row.clone()],"next_cursor":null}))
        } else if req.path == format!("/api/v1/shared-folders/{root}/search-index/shards") {
            (200, json!({"shards":[]}))
        } else if req.method == "PUT" && req.path.contains("/search-index/shards/") {
            (200, json!({"bucket":0,"page":0,"version":1}))
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
    assert!(
        db.dirty_shared_index_roots().unwrap().is_empty(),
        "a no-custody root is deferred, not retried: {seen:?}"
    );
    assert_eq!(
        count(&seen, "/search-index/"),
        0,
        "no index work without custody: {seen:?}"
    );
    let after_first = seen.lock().unwrap().len();
    run(bridge.maintain_owned_keys(2_000)).unwrap();
    assert_eq!(
        seen.lock().unwrap().len(),
        after_first,
        "a later pass makes no request at all: {seen:?}"
    );
    // The owner upgrades the folder; its rows do not change.
    *available.lock().unwrap() = true;
    run(bridge.shared_folder_custody_key(root)).unwrap();
    assert_eq!(
        db.dirty_shared_index_roots().unwrap(),
        vec![root.to_string()],
        "the custody GET that opens revives the deferred repair: {seen:?}"
    );
    run(bridge.maintain_owned_keys(3_000)).unwrap();
    assert!(
        count(&seen, "/search-index/shards/") > 0,
        "the next pass repairs the index: {seen:?}"
    );
    // Ingesting the row for the first time legitimately re-marks the root
    // during that same pass (the row's fingerprint changed), so the marker
    // clears on the pass after that one.
    run(bridge.maintain_owned_keys(4_000)).unwrap();
    assert!(db.dirty_shared_index_roots().unwrap().is_empty(), "{seen:?}");
}

/// Review 2279 item 2 / R1: an `upgrade_required` folder is deferred like "no
/// custody" instead of sending the same refused custody PUT every 30 seconds.
#[test]
fn sf2279_an_upgrade_required_folder_is_not_retried_every_pass() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000031";
    let folder = FileKey::from_bytes([0x9a; 32]);
    let invites = json!({"invites":[{
        "id":"inv-31","file_id":root,"is_folder_share":true,"status":"approved",
        "created_at":"2026-10-29T00:00:00Z",
        "encrypted_owner_folder_key":purpose_bound_copy(&master, root, &folder)
    }]});
    let (fixture, seen) = prepare_world(
        root,
        invites,
        json!({"keys":[]}),
        json!({"keys":[],"next_cursor":null}),
        (409, json!({"error":"upgrade_required"})),
        None,
    );
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(StateDb::open(temp.path().join("db")).unwrap());
    let bridge = crate::engine_bridge::EngineBridge::new(db.clone(), Arc::new(fixture.api()));
    db.mark_shared_index_dirty(root).unwrap();
    run(bridge.maintain_owned_keys(1_000)).unwrap();
    let put = format!("PUT /api/v1/shared-folders/{root}/key");
    assert_eq!(exact(&seen, &put), 1, "{seen:?}");
    run(bridge.maintain_owned_keys(2_000)).unwrap();
    assert_eq!(
        exact(&seen, &put),
        1,
        "an upgrade-required folder makes no custody PUT on the second pass: {seen:?}"
    );
}

/// Review 2279 item 3 / R2 / L7c: a custody 404 for a folder this device has
/// seen with custody refuses only that row. It is never read as legacy, the
/// other rows proceed, and the seen-custody fact is never forgotten.
#[test]
fn sf2279_a_seen_custody_404_blocks_only_its_row() {
    use beebeeb_core::kdf::MasterKey;
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000032";
    let item = "20000000-0000-8000-8000-000000000032";
    let owned = "40000000-0000-4000-8000-000000000032";
    let owned_key = beebeeb_core::kdf::derive_file_key(&master, owned.as_bytes());
    let owned_name = beebeeb_core::encrypt::encrypt_name_for(
        &owned_key,
        "owned.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    let owned_row = json!({"id":owned,"name_encrypted":owned_name,"parent_id":root,
        "is_folder":false,"version_number":1,"size_bytes":4,"updated_at":1});
    let (fixture, seen) = prepare_world(
        root,
        json!({"invites":[]}),
        json!({"keys":[]}),
        json!({"keys":[],"next_cursor":null}),
        (404, json!({})),
        None,
    );
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(StateDb::open(temp.path().join("db")).unwrap());
    let bridge = crate::engine_bridge::EngineBridge::new(db.clone(), Arc::new(fixture.api()));
    // This device has seen custody for the folder before (L7c/L7d).
    db.remember_folder_custody_at_revision(root, db.account_revision())
        .unwrap();
    let prepared = run(bridge.prepare_owned_rows(&[foreign_child(item, root), owned_row]))
        .unwrap_or_else(|error| panic!("a seen-custody 404 must not abort the snapshot: {error}; {seen:?}"));
    assert_eq!(prepared.len(), 2);
    let blocked = prepared.iter().find(|row| row.record.file_id == item).unwrap();
    assert!(!blocked.readable, "the refused row is blocked");
    assert!(blocked.record.blocked_fingerprint.is_some());
    assert!(
        blocked.record.shared_root_id.is_none(),
        "no root is recorded for a refused row"
    );
    assert!(
        prepared
            .iter()
            .find(|row| row.record.file_id == owned)
            .unwrap()
            .readable,
        "the other rows proceed: {seen:?}"
    );
    assert_eq!(count(&seen, "/folder-keys"), 0, "never read as legacy: {seen:?}");
    assert_eq!(
        count(&seen, "/shared-folders"),
        1,
        "one custody ask per folder: {seen:?}"
    );
    assert!(
        db.has_seen_folder_custody(root).unwrap(),
        "the seen-custody fact is not forgotten"
    );
}

/// L7d F1 (review 2279): a folder this device has seen with custody is never
/// deferred. Its dirty marker stays, so maintenance asks again (one custody GET
/// per 30 s pass) instead of hiding the work forever behind a deferral.
#[test]
fn sf2279_a_seen_custody_root_keeps_its_dirty_marker() {
    let root = "10000000-0000-4000-8000-000000000033";
    let (fixture, seen) = prepare_world(
        root,
        json!({"invites":[]}),
        json!({"keys":[]}),
        json!({"keys":[],"next_cursor":null}),
        (404, json!({})),
        None,
    );
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(StateDb::open(temp.path().join("db")).unwrap());
    let bridge = crate::engine_bridge::EngineBridge::new(db.clone(), Arc::new(fixture.api()));
    db.remember_folder_custody_at_revision(root, db.account_revision())
        .unwrap();
    db.mark_shared_index_dirty(root).unwrap();
    let ask = format!("GET /api/v1/shared-folders/{root}");
    run(bridge.maintain_owned_keys(1_000)).unwrap();
    assert_eq!(
        db.dirty_shared_index_roots().unwrap(),
        vec![root.to_string()],
        "the dirty marker stays for a seen-custody root: {seen:?}"
    );
    assert!(
        !db.revive_deferred_shared_index(root).unwrap(),
        "no deferral is written: {seen:?}"
    );
    assert_eq!(exact(&seen, &ask), 1, "one custody ask: {seen:?}");
    assert_eq!(count(&seen, "/search-index/"), 0, "no index work: {seen:?}");
    run(bridge.maintain_owned_keys(2_000)).unwrap();
    assert_eq!(
        exact(&seen, &ask),
        2,
        "the next pass asks again, as on main before this PR: {seen:?}"
    );
    assert_eq!(db.dirty_shared_index_roots().unwrap(), vec![root.to_string()]);
}

/// L7d F3 (review 2279): the deferral is written BEFORE the custody lookup, so
/// a custody GET that opens while the repair runs revives the work instead of
/// being stranded behind a deferral written after that revive. The fixture's
/// handler writes custody from its own thread while it answers 404 — the exact
/// interleaving the old ordering lost. The next pass then repairs the index.
#[test]
fn sf2279_a_custody_write_during_the_pass_is_not_stranded() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000034";
    let item = "20000000-0000-8000-8000-000000000034";
    let folder = FileKey::from_bytes([55; 32]);
    let child = FileKey::from_bytes([66; 32]);
    let child_wrap =
        STANDARD.encode(beebeeb_sync::key_resolution::wrap_child_key(&folder, root, item, 1, &child).unwrap());
    let item_name = beebeeb_core::encrypt::encrypt_name_for(
        &child,
        "holiday.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    let row = json!({"id":item,"name_encrypted":item_name,"parent_id":root,"is_folder":false,
        "version_number":1,"size_bytes":4,"updated_at":1,
        "acted_via_share_id":"fixture-share","uploaded_by":null,
        "owner_wrapped_file_key":null});
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(StateDb::open(temp.path().join("db")).unwrap());
    let revision = db.account_revision();
    let available = Arc::new(Mutex::new(false));
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let (log, ready, handle) = (seen.clone(), available.clone(), db.clone());
    let fixture = Fixture::wire(move |req| {
        log.lock().unwrap().push(format!("{} {}", req.method, req.path));
        let ready = *ready.lock().unwrap();
        let (status, body) = if req.path == "/api/v1/shares/invites/sent" {
            (200, json!({"invites":[]}))
        } else if req.path == format!("/api/v1/shared-folders/{root}") && !ready {
            // Custody opens while the repair is in flight: the production
            // `AccountScope::remember_custody` inserts the fact, then revives
            // the deferred index work.
            handle.remember_folder_custody_at_revision(root, revision).unwrap();
            handle.revive_deferred_shared_index(root).unwrap();
            (404, json!({}))
        } else if req.path == format!("/api/v1/shared-folders/{root}") {
            (
                200,
                json!({"folder_id":root,"role":"owner","key_version":1,
                "owner_wrapped_folder_key":STANDARD.encode(
                    beebeeb_sync::key_resolution::wrap_folder_custody(&master, root, &folder).unwrap())}),
            )
        } else if req.path.starts_with(&format!("/api/v1/shared-folders/{root}/keys")) {
            (
                200,
                json!({"keys":[{"file_id":item,"encrypted_file_key":child_wrap}],"next_cursor":null}),
            )
        } else if req.path == format!("/api/v1/files/{item}") {
            (200, row.clone())
        } else if req.path.starts_with("/api/v1/files?") {
            (200, json!({"files":[row.clone()],"next_cursor":null}))
        } else if req.path == format!("/api/v1/shared-folders/{root}/search-index/shards") {
            (200, json!({"shards":[]}))
        } else if req.method == "PUT" && req.path.contains("/search-index/shards/") {
            (200, json!({"bucket":0,"page":0,"version":1}))
        } else {
            (404, json!({}))
        };
        Some((status, serde_json::to_vec(&body).unwrap()))
    });
    let bridge = crate::engine_bridge::EngineBridge::new(db.clone(), Arc::new(fixture.api()));
    db.mark_shared_index_dirty(root).unwrap();
    let report = run(bridge.repair_shared_folder_index(root)).unwrap();
    assert_eq!(
        report.deferred, 0,
        "a custody write during the pass revives instead of deferring: {seen:?}"
    );
    assert_eq!(
        db.dirty_shared_index_roots().unwrap(),
        vec![root.to_string()],
        "the revive re-dirties the root: {seen:?}"
    );
    assert!(
        !db.revive_deferred_shared_index(root).unwrap(),
        "no deferral is left behind: {seen:?}"
    );
    assert_eq!(
        count(&seen, "/search-index/"),
        0,
        "nothing ran before custody was open: {seen:?}"
    );
    *available.lock().unwrap() = true;
    run(bridge.maintain_owned_keys(2_000)).unwrap();
    assert!(
        count(&seen, "/search-index/shards/") > 0,
        "the next pass repairs the index: {seen:?}"
    );
}

/// The production walk revives a deferred root: `prepare_owned_rows` opens the
/// root's custody record for a row, and `remember_custody` re-dirties the
/// index. The resume test above drives `shared_folder_custody_key` directly
/// (the reviewer's note); this one goes through the snapshot walk the index
/// repair and the activity feed also use.
#[test]
fn sf2279_prepare_owned_rows_revives_a_deferred_root() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000035";
    let item = "20000000-0000-8000-8000-000000000035";
    let folder = FileKey::from_bytes([55; 32]);
    let child = FileKey::from_bytes([66; 32]);
    let child_wrap =
        STANDARD.encode(beebeeb_sync::key_resolution::wrap_child_key(&folder, root, item, 1, &child).unwrap());
    let item_name = beebeeb_core::encrypt::encrypt_name_for(
        &child,
        "holiday.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    let row = json!({"id":item,"name_encrypted":item_name,"parent_id":root,"is_folder":false,
        "version_number":1,"size_bytes":4,"updated_at":1,
        "acted_via_share_id":"fixture-share","uploaded_by":null,
        "owner_wrapped_file_key":null});
    let available = Arc::new(Mutex::new(false));
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let (log, ready) = (seen.clone(), available.clone());
    let served_row = row.clone();
    let fixture = Fixture::wire(move |req| {
        log.lock().unwrap().push(format!("{} {}", req.method, req.path));
        let ready = *ready.lock().unwrap();
        let (status, body) = if req.path == "/api/v1/shares/invites/sent" {
            (200, json!({"invites":[]}))
        } else if req.path == format!("/api/v1/shared-folders/{root}") && !ready {
            (404, json!({}))
        } else if req.path == format!("/api/v1/shared-folders/{root}") {
            (
                200,
                json!({"folder_id":root,"role":"owner","key_version":1,
                "owner_wrapped_folder_key":STANDARD.encode(
                    beebeeb_sync::key_resolution::wrap_folder_custody(&master, root, &folder).unwrap())}),
            )
        } else if req.path.starts_with(&format!("/api/v1/shared-folders/{root}/keys")) {
            (
                200,
                json!({"keys":[{"file_id":item,"encrypted_file_key":child_wrap}],"next_cursor":null}),
            )
        } else if req.path == format!("/api/v1/files/{item}") {
            (200, served_row.clone())
        } else if req.path.starts_with("/api/v1/files?") {
            (200, json!({"files":[served_row.clone()],"next_cursor":null}))
        } else if req.path == format!("/api/v1/shared-folders/{root}/search-index/shards") {
            (200, json!({"shards":[]}))
        } else if req.method == "PUT" && req.path.contains("/search-index/shards/") {
            (200, json!({"bucket":0,"page":0,"version":1}))
        } else {
            (404, json!({}))
        };
        Some((status, serde_json::to_vec(&body).unwrap()))
    });
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(StateDb::open(temp.path().join("db")).unwrap());
    let bridge = crate::engine_bridge::EngineBridge::new(db.clone(), Arc::new(fixture.api()));
    db.mark_shared_index_dirty(root).unwrap();
    let report = run(bridge.repair_shared_folder_index(root)).unwrap();
    assert_eq!(report.deferred, 1, "a never-seen root is deferred: {seen:?}");
    assert!(
        db.dirty_shared_index_roots().unwrap().is_empty(),
        "the deferral replaces the dirty marker: {seen:?}"
    );
    // The owner upgrades the folder; its rows do not change.
    *available.lock().unwrap() = true;
    let prepared = run(bridge.prepare_owned_rows(std::slice::from_ref(&row))).unwrap();
    assert_eq!(prepared.len(), 1);
    assert!(prepared[0].readable, "the walk opens the row: {seen:?}");
    assert_eq!(
        db.dirty_shared_index_roots().unwrap(),
        vec![root.to_string()],
        "the walk's remember_custody revives the deferred root: {seen:?}"
    );
    run(bridge.maintain_owned_keys(2_000)).unwrap();
    assert!(
        count(&seen, "/search-index/shards/") > 0,
        "the next pass repairs the index: {seen:?}"
    );
}
