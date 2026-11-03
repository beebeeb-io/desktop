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

/// An L14 world: the root's proven custody and inventory, one listing under the
/// root, and the row detail for every listed id.
fn l14_world(
    root: &'static str,
    folder: &beebeeb_core::kdf::FileKey,
    rows: Vec<Value>,
    inventory: Vec<Value>,
) -> (Fixture, Arc<Mutex<Vec<String>>>) {
    use beebeeb_core::kdf::MasterKey;
    let master = MasterKey::from_bytes([7; 32]);
    let custody = STANDARD.encode(beebeeb_sync::key_resolution::wrap_folder_custody(&master, root, folder).unwrap());
    let served = rows.clone();
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = seen.clone();
    let fixture = Fixture::wire(move |req| {
        log.lock().unwrap().push(format!("{} {}", req.method, req.path));
        let row = served.iter().find(|row| {
            req.path == format!("/api/v1/files/{}", row["id"].as_str().unwrap_or_default())
        });
        let (status, body) = if req.path == "/api/v1/shares/invites/sent" {
            (200, json!({"invites":[]}))
        } else if req.path == format!("/api/v1/shared-folders/{root}") {
            (200, json!({"folder_id":root,"role":"owner","key_version":1,"owner_wrapped_folder_key":custody}))
        } else if req.path.starts_with(&format!("/api/v1/shared-folders/{root}/keys")) {
            (200, json!({"keys":inventory,"next_cursor":null}))
        } else if let Some(row) = row {
            (200, row.clone())
        } else if let Some(parent) = req.path.split("parent_id=").nth(1) {
            let parent = parent.split('&').next().unwrap_or_default();
            let page: Vec<Value> = served
                .iter()
                .filter(|row| row["parent_id"].as_str() == Some(parent))
                .cloned()
                .collect();
            (200, json!({"files":page,"next_cursor":null}))
        } else if req.path == format!("/api/v1/shared-folders/{root}/search-index/shards") {
            (200, json!({"shards":[]}))
        } else if req.method == "PUT" && req.path.contains("/search-index/shards/") {
            (200, json!({"bucket":0,"page":0,"version":1}))
        } else {
            (404, json!({}))
        };
        Some((status, serde_json::to_vec(&body).unwrap()))
    });
    (fixture, seen)
}

fn l14_bridge(fixture: &Fixture) -> (Arc<StateDb>, crate::engine_bridge::EngineBridge) {
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(StateDb::open(temp.path().join("db")).unwrap());
    let bridge = crate::engine_bridge::EngineBridge::new(db.clone(), Arc::new(fixture.api()));
    // Keep the temp dir alive for the DB's lifetime by leaking it: the test
    // process is the only user.
    std::mem::forget(temp);
    (db, bridge)
}

/// A10 (L14): an item listed under R, readable through its own derived key but
/// with no wrap for (R, X, kind), is not indexed.
#[test]
fn a10_index_skips_listed_item_without_wrap() {
    use beebeeb_core::kdf::MasterKey;
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000040";
    let item = "40000000-0000-4000-8000-000000000040";
    let derived = beebeeb_core::kdf::derive_file_key(&master, item.as_bytes());
    let name = beebeeb_core::encrypt::encrypt_name_for(
        &derived,
        "outside.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    let row = json!({"id":item,"name_encrypted":name,"parent_id":root,"is_folder":false,
        "version_number":1,"size_bytes":4,"updated_at":1});
    let (fixture, seen) = l14_world(root, &beebeeb_core::kdf::FileKey::from_bytes([55; 32]), vec![row], vec![]);
    let (_, bridge) = l14_bridge(&fixture);
    let report = run(bridge.repair_shared_folder_index(root)).unwrap();
    assert_eq!(report.skip_reasons.get("no_wrap"), Some(&1), "{seen:?}");
    assert_eq!(report.written, 0, "nothing is indexed: {seen:?}");
}

/// A11 (L14): a wrap of another item's key is skipped as `wrap_key_mismatch`,
/// even though the item itself reads through its owner envelope.
#[test]
fn a11_index_skips_wrap_of_another_items_key() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000041";
    let item = "20000000-0000-8000-8000-000000000041";
    let folder = FileKey::from_bytes([55; 32]);
    let item_key = FileKey::from_bytes([66; 32]);
    let other = FileKey::from_bytes([161; 32]);
    let name = beebeeb_core::encrypt::encrypt_name_for(
        &item_key,
        "transferred.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    let row = json!({"id":item,"name_encrypted":name,"parent_id":root,"is_folder":false,
        "version_number":1,"size_bytes":4,"updated_at":1,
        "acted_via_share_id":"fixture-share","uploaded_by":null,
        "owner_wrapped_file_key":STANDARD.encode(
            beebeeb_sync::key_resolution::wrap_owner_file_key(&master, item, &item_key).unwrap())});
    let inventory = vec![json!({
        "file_id":item,
        "encrypted_file_key":STANDARD.encode(
            beebeeb_core::owned_keys::wrap_child_key(&folder, root, item, 1, &other).unwrap()),
    })];
    let (fixture, seen) = l14_world(root, &folder, vec![row], inventory);
    let (_, bridge) = l14_bridge(&fixture);
    let report = run(bridge.repair_shared_folder_index(root)).unwrap();
    assert_eq!(report.skip_reasons.get("wrap_key_mismatch"), Some(&1), "{seen:?}");
    assert_eq!(report.written, 0, "nothing is indexed: {seen:?}");
}

/// A12 (L14): a folder listed under R that the proven key does not prove is not
/// descended into, so its (well-wrapped) child is not indexed either.
#[test]
fn a12_index_does_not_descend_into_unproven_folder() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000042";
    let nested = "20000000-0000-8000-8000-000000000042";
    let child = "20000000-0000-8000-8000-000000000043";
    let folder = FileKey::from_bytes([55; 32]);
    let nested_key = FileKey::from_bytes([66; 32]);
    let child_key = FileKey::from_bytes([77; 32]);
    let folder_name = beebeeb_core::encrypt::encrypt_name_for(
        &nested_key,
        "Nested",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    let nested_row = json!({"id":nested,"name_encrypted":folder_name,"parent_id":root,"is_folder":true,
        "version_number":1,"size_bytes":4,"updated_at":1,
        "acted_via_share_id":"fixture-share","uploaded_by":null,
        "owner_wrapped_file_key":STANDARD.encode(
            beebeeb_sync::key_resolution::wrap_owner_file_key(&master, nested, &nested_key).unwrap())});
    let child_name = beebeeb_core::encrypt::encrypt_name_for(
        &child_key,
        "inner.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    let child_row = json!({"id":child,"name_encrypted":child_name,"parent_id":nested,"is_folder":false,
        "version_number":1,"size_bytes":4,"updated_at":1,
        "acted_via_share_id":"fixture-share","uploaded_by":null,
        "owner_wrapped_file_key":STANDARD.encode(
            beebeeb_sync::key_resolution::wrap_owner_file_key(&master, child, &child_key).unwrap())});
    // Only the grandchild has a wrap; the unproven folder itself has none.
    let inventory = vec![json!({
        "file_id":child,
        "encrypted_file_key":STANDARD.encode(
            beebeeb_core::owned_keys::wrap_child_key(&folder, root, child, 1, &child_key).unwrap()),
    })];
    let (fixture, seen) = l14_world(root, &folder, vec![nested_row, child_row], inventory);
    let (_, bridge) = l14_bridge(&fixture);
    let report = run(bridge.repair_shared_folder_index(root)).unwrap();
    assert_eq!(report.skip_reasons.get("no_wrap"), Some(&1), "{seen:?}");
    assert_eq!(
        count(&seen, &format!("parent_id={nested}")),
        0,
        "an unproven folder is never walked: {seen:?}"
    );
    assert_eq!(report.written, 0, "nothing is indexed: {seen:?}");
}

/// P3-6: a wrap held by ANOTHER root is no evidence at all, so the skip reason
/// is `no_wrap` — never the old `wrap_does_not_open`.
#[test]
fn review_p3_6_a_wrap_under_another_root_does_not_change_the_skip_reason() {
    use beebeeb_core::kdf::MasterKey;
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000046";
    let other_root = "10000000-0000-4000-8000-000000000047";
    let item = "40000000-0000-4000-8000-000000000046";
    let derived = beebeeb_core::kdf::derive_file_key(&master, item.as_bytes());
    let name = beebeeb_core::encrypt::encrypt_name_for(
        &derived,
        "elsewhere.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    let row = json!({"id":item,"name_encrypted":name,"parent_id":root,"is_folder":false,
        "version_number":1,"size_bytes":4,"updated_at":1});
    let (fixture, seen) = l14_world(root, &beebeeb_core::kdf::FileKey::from_bytes([55; 32]), vec![row], vec![]);
    let (db, bridge) = l14_bridge(&fixture);
    // An earlier pass indexed the item under another root: that record's wrap
    // proves nothing about membership in THIS root.
    db.put_owned_key_record(&OwnedKeyRecord {
        file_id: item.into(),
        name_encrypted: name,
        shared_root_id: Some(other_root.into()),
        ..Default::default()
    })
    .unwrap();
    let report = run(bridge.repair_shared_folder_index(root)).unwrap();
    assert_eq!(report.skip_reasons.get("no_wrap"), Some(&1), "{seen:?}");
    assert!(
        report.skip_reasons.get("wrap_does_not_open").is_none(),
        "another root's wrap is not a reason: {:?}",
        report.skip_reasons
    );
    assert_eq!(report.written, 0, "nothing is indexed: {seen:?}");
}

/// A11 (owner name): the wrap of another key served under an owner item's id is
/// skipped exactly as a foreign one is — the name class changes nothing.
#[test]
fn a11_owner_item_wrap_of_another_key_is_skipped() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000048";
    let item = "40000000-0000-4000-8000-000000000048";
    let folder = FileKey::from_bytes([55; 32]);
    let derived = beebeeb_core::kdf::derive_file_key(&master, item.as_bytes());
    let other = FileKey::from_bytes([0xa1; 32]);
    let name = beebeeb_core::encrypt::encrypt_name_for(
        &derived,
        "owner-named.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    let row = json!({"id":item,"name_encrypted":name,"parent_id":root,"is_folder":false,
        "version_number":1,"size_bytes":4,"updated_at":1});
    let inventory = vec![json!({
        "file_id":item,
        "encrypted_file_key":STANDARD.encode(
            beebeeb_core::owned_keys::wrap_child_key(&folder, root, item, 1, &other).unwrap()),
    })];
    let (fixture, seen) = l14_world(root, &folder, vec![row], inventory);
    let (_, bridge) = l14_bridge(&fixture);
    let report = run(bridge.repair_shared_folder_index(root)).unwrap();
    assert_eq!(report.skip_reasons.get("wrap_key_mismatch"), Some(&1), "{seen:?}");
    assert_eq!(report.written, 0, "nothing is indexed: {seen:?}");
}

/// A10 (positive control): an owner item wrapped correctly for this root IS
/// indexed — the L14 gates do not exclude the ordinary case.
#[test]
fn a10_a_correctly_wrapped_owner_item_is_indexed() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-000000000049";
    let item = "40000000-0000-4000-8000-000000000049";
    let folder = FileKey::from_bytes([55; 32]);
    let derived = beebeeb_core::kdf::derive_file_key(&master, item.as_bytes());
    let name = beebeeb_core::encrypt::encrypt_name_for(
        &derived,
        "indexed.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    let row = json!({"id":item,"name_encrypted":name,"parent_id":root,"is_folder":false,
        "version_number":1,"size_bytes":4,"updated_at":1});
    let inventory = vec![json!({
        "file_id":item,
        "encrypted_file_key":STANDARD.encode(
            beebeeb_core::owned_keys::wrap_child_key(&folder, root, item, 1, &derived).unwrap()),
    })];
    let (fixture, seen) = l14_world(root, &folder, vec![row], inventory);
    let (_, bridge) = l14_bridge(&fixture);
    let report = run(bridge.repair_shared_folder_index(root)).unwrap();
    assert!(report.written >= 1, "the correctly wrapped item is indexed: {:?} {seen:?}", report.skip_reasons);
    assert!(report.skip_reasons.is_empty(), "{:?}", report.skip_reasons);
}

/// A12 (positive control): a proven folder is descended into, and its correctly
/// wrapped child is indexed too.
#[test]
fn a12_a_correctly_wrapped_child_of_a_proven_folder_is_indexed() {
    use beebeeb_core::kdf::{FileKey, MasterKey};
    let master = MasterKey::from_bytes([7; 32]);
    let root = "10000000-0000-4000-8000-00000000004a";
    let nested = "20000000-0000-8000-8000-00000000004a";
    let child = "20000000-0000-8000-8000-00000000004b";
    let folder = FileKey::from_bytes([55; 32]);
    let nested_key = FileKey::from_bytes([66; 32]);
    let child_key = FileKey::from_bytes([77; 32]);
    let nested_name = beebeeb_core::encrypt::encrypt_name_for(
        &nested_key,
        "Nested",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    let nested_row = json!({"id":nested,"name_encrypted":nested_name,"parent_id":root,"is_folder":true,
        "version_number":1,"size_bytes":4,"updated_at":1,
        "acted_via_share_id":"fixture-share","uploaded_by":null,
        "owner_wrapped_file_key":STANDARD.encode(
            beebeeb_sync::key_resolution::wrap_owner_file_key(&master, nested, &nested_key).unwrap())});
    let child_name = beebeeb_core::encrypt::encrypt_name_for(
        &child_key,
        "inner.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    let child_row = json!({"id":child,"name_encrypted":child_name,"parent_id":nested,"is_folder":false,
        "version_number":1,"size_bytes":4,"updated_at":1,
        "acted_via_share_id":"fixture-share","uploaded_by":null,
        "owner_wrapped_file_key":STANDARD.encode(
            beebeeb_sync::key_resolution::wrap_owner_file_key(&master, child, &child_key).unwrap())});
    let inventory = vec![
        json!({"file_id":nested,"encrypted_file_key":STANDARD.encode(
            beebeeb_core::owned_keys::wrap_child_key(&folder, root, nested, 2, &nested_key).unwrap())}),
        json!({"file_id":child,"encrypted_file_key":STANDARD.encode(
            beebeeb_core::owned_keys::wrap_child_key(&folder, root, child, 1, &child_key).unwrap())}),
    ];
    let (fixture, seen) = l14_world(root, &folder, vec![nested_row, child_row], inventory);
    let (_, bridge) = l14_bridge(&fixture);
    let report = run(bridge.repair_shared_folder_index(root)).unwrap();
    assert!(report.written >= 1, "folder and child are indexed: {:?} {seen:?}", report.skip_reasons);
    assert_eq!(count(&seen, &format!("parent_id={nested}")), 1, "the proven folder is walked: {seen:?}");
}
