//! A19: an owner move that takes an F item (or a folder with F descendants)
//! out of its shared root adopts every foreign item first, confirmed by
//! read-back, or the move is refused.

use super::*;
use crate::state_db::{FileEntry, FileStatus, ItemKind};
use beebeeb_core::chunk_stream::ExpectedChunkFormat;
use beebeeb_core::kdf::{FileKey, MasterKey};
use beebeeb_sync::key_resolution::{wrap_child_key, wrap_folder_custody, wrap_owner_file_key};

const ROOT: &str = "10000000-0000-4000-8000-0000000000c1";
const OWNER_FOLDER: &str = "40000000-0000-4000-8000-0000000000c2";
const GUEST: &str = "20000000-0000-8000-8000-0000000000c3";
const PRIVATE: &str = "40000000-0000-4000-8000-0000000000c4";
const ROOT_TWO: &str = "10000000-0000-4000-8000-0000000000c5";

fn seed(bridge: &crate::engine_bridge::EngineBridge, id: &str, kind: ItemKind, parent: Option<&str>) {
    let entry = FileEntry {
        file_id: id.into(),
        path: id.into(),
        status: FileStatus::Local,
        size_bytes: 0,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 0,
        parent_id: None,
        item_kind: kind.clone(),
    };
    bridge
        .db()
        .upsert_file_with_contract(&entry, |contract| {
            contract.item_kind = kind;
            contract.parent_id = parent.map(str::to_string);
        })
        .unwrap();
}

/// The shared root `ROOT`, an owner folder inside it, and one F file under
/// that folder. `poison` makes the server answer the adoption POST with an
/// envelope that does not open to our key, so the read-back cannot confirm it.
fn world(poison: bool) -> (Fixture, Arc<Mutex<Vec<String>>>) {
    let master = MasterKey::from_bytes([7; 32]);
    let folder = FileKey::from_bytes([55; 32]);
    let guest = FileKey::from_bytes([66; 32]);
    let guest_name =
        beebeeb_core::encrypt::encrypt_name_for(&guest, "added.txt", None, ExpectedChunkFormat::Unrecorded).unwrap();
    let wrap = STANDARD.encode(wrap_child_key(&folder, ROOT, GUEST, 1, &guest).unwrap());
    let custody = STANDARD.encode(wrap_folder_custody(&master, ROOT, &folder).unwrap());
    let served = Arc::new(Mutex::new(json!({
        "id": GUEST, "name_encrypted": guest_name, "parent_id": OWNER_FOLDER,
        "is_folder": false, "version_number": 1, "size_bytes": 4, "updated_at": 1,
        "acted_via_share_id": "fixture-share", "uploaded_by": null,
        "owner_wrapped_file_key": null,
    })));
    let row = served.clone();
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = seen.clone();
    let fixture = Fixture::wire(move |req| {
        log.lock().unwrap().push(format!("{} {}", req.method, req.path));
        let body = if req.path == "/api/v1/shares/invites/sent" {
            json!({"invites": []})
        } else if req.path == format!("/api/v1/shared-folders/{ROOT}") {
            json!({"folder_id": ROOT, "role": "owner", "key_version": 1,
                   "owner_wrapped_folder_key": custody})
        } else if req.path.starts_with(&format!("/api/v1/shared-folders/{ROOT}/keys")) {
            json!({"keys": [{"file_id": GUEST, "encrypted_file_key": wrap}], "next_cursor": null})
        } else if req.path == format!("/api/v1/files/{GUEST}/owner-key") {
            let stored = if poison {
                json!("not-a-frame")
            } else {
                req.body["owner_wrapped_file_key"].clone()
            };
            row.lock().unwrap()["owner_wrapped_file_key"] = stored;
            json!({"adopted": true})
        } else if req.path == format!("/api/v1/files/{OWNER_FOLDER}") {
            json!({"id": OWNER_FOLDER, "parent_id": ROOT, "is_folder": true, "version_number": 1,
                   "name_encrypted": "opaque", "size_bytes": 0, "updated_at": 1})
        } else if req.path == format!("/api/v1/files/{ROOT}") {
            json!({"id": ROOT, "parent_id": null, "is_folder": true, "version_number": 1,
                   "name_encrypted": "opaque", "size_bytes": 0, "updated_at": 1})
        } else if req.path == format!("/api/v1/files/{GUEST}") {
            return Some((200, serde_json::to_vec(&*row.lock().unwrap()).unwrap()));
        } else {
            json!({})
        };
        Some((200, serde_json::to_vec(&body).unwrap()))
    });
    (fixture, seen)
}

fn bridge(dir: &std::path::Path, fixture: &Fixture) -> crate::engine_bridge::EngineBridge {
    let db = Arc::new(StateDb::open(dir.join("db")).unwrap());
    let bridge = crate::engine_bridge::EngineBridge::new(db, Arc::new(fixture.api()));
    bridge.db().remember_folder_custody_at_revision(ROOT, 0).unwrap();
    seed(&bridge, ROOT, ItemKind::Folder, None);
    seed(&bridge, OWNER_FOLDER, ItemKind::Folder, Some(ROOT));
    seed(&bridge, GUEST, ItemKind::File, Some(OWNER_FOLDER));
    bridge
}

#[test]
fn a19_a_move_out_adopts_every_foreign_descendant_first() {
    let (fixture, seen) = world(false);
    let temp = tempfile::tempdir().unwrap();
    let bridge = bridge(temp.path(), &fixture);
    assert!(bridge.db().get_owned_key_record(GUEST).unwrap().is_none(), "nothing is adopted yet");
    run(bridge.adopt_foreign_subtree(OWNER_FOLDER, Some(ROOT), None)).expect("adoption lands before the move");
    let record = bridge.db().get_owned_key_record(GUEST).unwrap().expect("an adopted record");
    assert!(record.owner_wrap.is_some(), "read back: the served envelope opens");
    assert!(record.pending_owner_wrap.is_none());
    // Still inside the root: no adoption, no request.
    let before = count(&seen, "/owner-key");
    run(bridge.adopt_foreign_subtree(OWNER_FOLDER, Some(ROOT), Some(ROOT))).expect("a move inside the root needs nothing");
    assert_eq!(count(&seen, "/owner-key"), before);
}

#[test]
fn a19_a_failed_adoption_refuses_the_move() {
    let (fixture, seen) = world(true);
    let temp = tempfile::tempdir().unwrap();
    let bridge = bridge(temp.path(), &fixture);
    let error = run(bridge.adopt_foreign_subtree(OWNER_FOLDER, Some(ROOT), None)).expect_err("refused");
    assert!(format!("{error}").contains("move needs adoption"), "{error}");
    let record = bridge.db().get_owned_key_record(GUEST).unwrap().unwrap();
    assert!(record.owner_wrap.is_none(), "an envelope that does not open is never adopted");
    assert_eq!(count(&seen, "/owner-key"), 1, "one attempt, then the refusal: {seen:?}");
}

/// A shared root with NO inventory wraps: the only proof of the guest item's key
/// is the owner's pending envelope, so it is not adopted yet. The upload init
/// answers a one-chunk plan for `GUEST`.
fn bare_world() -> (Fixture, Arc<Mutex<Vec<String>>>) {
    let master = MasterKey::from_bytes([7; 32]);
    let folder = FileKey::from_bytes([55; 32]);
    let guest = FileKey::from_bytes([66; 32]);
    let guest_name =
        beebeeb_core::encrypt::encrypt_name_for(&guest, "added.txt", None, ExpectedChunkFormat::Unrecorded).unwrap();
    let custody = STANDARD.encode(wrap_folder_custody(&master, ROOT, &folder).unwrap());
    let custody_two = STANDARD.encode(wrap_folder_custody(&master, ROOT_TWO, &folder).unwrap());
    let row = json!({
        "id": GUEST, "name_encrypted": guest_name, "parent_id": OWNER_FOLDER,
        "is_folder": false, "version_number": 1, "size_bytes": 4, "updated_at": 1,
        "acted_via_share_id": "fixture-share", "uploaded_by": null,
        "owner_wrapped_file_key": null,
    });
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = seen.clone();
    let fixture = Fixture::wire(move |req| {
        log.lock().unwrap().push(format!("{} {}", req.method, req.path));
        let body = if req.path == "/api/v1/shares/invites/sent" {
            json!({"invites": []})
        } else if req.path == format!("/api/v1/shared-folders/{ROOT}") {
            json!({"folder_id": ROOT, "role": "owner", "key_version": 1,
                   "owner_wrapped_folder_key": custody})
        } else if req.path == format!("/api/v1/shared-folders/{ROOT_TWO}") {
            json!({"folder_id": ROOT_TWO, "role": "owner", "key_version": 1,
                   "owner_wrapped_folder_key": custody_two})
        } else if req.path.starts_with(&format!("/api/v1/shared-folders/{ROOT}/keys")) {
            json!({"keys": [], "next_cursor": null})
        } else if req.path == format!("/api/v1/files/{GUEST}") {
            row.clone()
        } else if req.path == "/api/v1/uploads/init" {
            json!({"file_id": GUEST, "tenant_id": "fixture", "object_version_id": "obj-1",
                   "upload_session_id": "session-1", "chunk_size_bytes": 4, "chunk_count": 1,
                   "storage_format_version": 1, "storage_pool_id": "pool", "region": "eu"})
        } else {
            json!({})
        };
        Some((200, serde_json::to_vec(&body).unwrap()))
    });
    (fixture, seen)
}

fn bare_bridge(dir: &std::path::Path, fixture: &Fixture) -> crate::engine_bridge::EngineBridge {
    let db = Arc::new(StateDb::open(dir.join("db")).unwrap());
    let bridge = crate::engine_bridge::EngineBridge::new(db, Arc::new(fixture.api()));
    bridge.db().remember_folder_custody_at_revision(ROOT, 0).unwrap();
    bridge.db().remember_folder_custody_at_revision(ROOT_TWO, 0).unwrap();
    seed(&bridge, ROOT, ItemKind::Folder, None);
    seed(&bridge, OWNER_FOLDER, ItemKind::Folder, Some(ROOT));
    seed(&bridge, GUEST, ItemKind::File, Some(OWNER_FOLDER));
    seed(&bridge, PRIVATE, ItemKind::Folder, None);
    // Not adopted: the owner's pending envelope opens the name, but no owner
    // wrap has been confirmed by read-back.
    let master = MasterKey::from_bytes([7; 32]);
    let guest = FileKey::from_bytes([66; 32]);
    bridge
        .db()
        .put_owned_key_record(&OwnedKeyRecord {
            file_id: GUEST.into(),
            name_encrypted: beebeeb_core::encrypt::encrypt_name_for(
                &guest,
                "added.txt",
                None,
                ExpectedChunkFormat::Unrecorded,
            )
            .unwrap(),
            pending_owner_wrap: Some(wrap_owner_file_key(&master, GUEST, &guest).unwrap()),
            foreign_added: true,
            ..Default::default()
        })
        .unwrap();
    bridge
}

fn move_target(parent: &str) -> crate::engine_bridge::FinderWriteTarget {
    crate::engine_bridge::FinderWriteTarget {
        file_id: Some(GUEST.into()),
        parent_id: Some(parent.into()),
        filename: "added.txt".into(),
        rel_path: Some("Private/added.txt".into()),
        kind: crate::engine_bridge::FinderWriteItemKind::File,
        contents_path: None,
        content_type: Some("text/plain".into()),
        base_version_identifier: None,
    }
}

/// The queued move, run through the op runner, refuses: the F item is not
/// adopted, and the move would leave the shared root.
#[test]
fn review_p1_2_a_queued_move_out_of_a_shared_root_is_refused() {
    let (fixture, seen) = bare_world();
    let temp = tempfile::tempdir().unwrap();
    let sync_root = temp.path().join("sync");
    std::fs::create_dir_all(&sync_root).unwrap();
    let bridge = bare_bridge(temp.path(), &fixture);
    let queued = bridge.queue_local_rename(move_target(PRIVATE), "Private/added.txt").unwrap();
    let op_id = match queued {
        crate::engine_bridge::FinderWriteOutcome::Queued { op_id, .. } => op_id,
        other => panic!("a move is queued: {other:?}"),
    };
    let outcome = run(bridge.process_due_operations(&sync_root, i64::MAX / 2)).unwrap();
    assert!(
        outcome.retried_op_ids.contains(&op_id) || outcome.paused_op_ids.contains(&op_id),
        "the move did not land: {outcome:?}"
    );
    let op = bridge.db().get_operation(&op_id).unwrap().expect("the op is kept");
    let error = op.last_error.unwrap_or_default();
    assert!(error.contains("move needs adoption"), "{error}");
    assert_eq!(
        count(&seen, &format!("PATCH /api/v1/files/{GUEST}")),
        0,
        "no metadata move is sent before the adoption is proven: {seen:?}"
    );
}

/// A move between two shared roots is refused the same way: R -> R2 is not
/// "still inside the root".
#[test]
fn review_p1_2_a_move_from_one_shared_root_to_another_is_refused() {
    let (fixture, seen) = bare_world();
    let temp = tempfile::tempdir().unwrap();
    let sync_root = temp.path().join("sync");
    std::fs::create_dir_all(&sync_root).unwrap();
    let bridge = bare_bridge(temp.path(), &fixture);
    let queued = bridge.queue_local_rename(move_target(ROOT_TWO), "Other/added.txt").unwrap();
    let op_id = match queued {
        crate::engine_bridge::FinderWriteOutcome::Queued { op_id, .. } => op_id,
        other => panic!("a move is queued: {other:?}"),
    };
    let outcome = run(bridge.process_due_operations(&sync_root, i64::MAX / 2)).unwrap();
    assert!(
        outcome.retried_op_ids.contains(&op_id) || outcome.paused_op_ids.contains(&op_id),
        "the move did not land: {outcome:?}"
    );
    let error = bridge.db().get_operation(&op_id).unwrap().unwrap().last_error.unwrap_or_default();
    assert!(error.contains("move needs adoption"), "{error}");
    assert_eq!(count(&seen, &format!("PATCH /api/v1/files/{GUEST}")), 0, "{seen:?}");
}

/// An upload whose metadata carries a parent change runs the same guard, before
/// the PATCH.
#[test]
fn review_p1_2_an_upload_with_a_parent_change_is_refused() {
    let (fixture, seen) = bare_world();
    let temp = tempfile::tempdir().unwrap();
    let sync_root = temp.path().join("sync");
    std::fs::create_dir_all(&sync_root).unwrap();
    let bridge = bare_bridge(temp.path(), &fixture);
    let payload = temp.path().join("payload.bin");
    std::fs::write(&payload, b"abcd").unwrap();
    let guest = FileKey::from_bytes([66; 32]);
    let name = beebeeb_core::encrypt::encrypt_name_for(&guest, "added.txt", None, ExpectedChunkFormat::Unrecorded).unwrap();
    let metadata = json!({
        "name_encrypted": name,
        "content_type": "text/plain",
        "parent_changed": true,
        "vacated_parent_id": OWNER_FOLDER,
    });
    let op = crate::state_db::PendingOperation {
        op_id: "op-p1-2-upload".into(),
        kind: crate::state_db::OperationKind::UploadVersion,
        file_id: Some(GUEST.into()),
        parent_id: Some(PRIVATE.into()),
        target_path: Some("Private/added.txt".into()),
        metadata_json: Some(metadata.to_string()),
        payload_path: Some(payload.to_string_lossy().into_owned()),
        base_version: None,
        base_object_version_id: None,
        attempts: 0,
        max_attempts: 25,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: 1,
        updated_at: 1,
    };
    bridge.db().enqueue_operation(&op).unwrap();
    let outcome = run(bridge.process_due_operations(&sync_root, i64::MAX / 2)).unwrap();
    assert!(
        outcome.retried_op_ids.contains(&op.op_id) || outcome.paused_op_ids.contains(&op.op_id),
        "the upload did not land: {outcome:?}"
    );
    let error = bridge.db().get_operation(&op.op_id).unwrap().unwrap().last_error.unwrap_or_default();
    assert!(error.contains("move needs adoption"), "{error}");
    assert_eq!(count(&seen, &format!("PATCH /api/v1/files/{GUEST}")), 0, "{seen:?}");
}

/// P1-2: a File Provider content save moves the row inside the accept
/// transaction, so by the time the upload's guard runs the DB parent is the
/// destination. With no `vacated_parent_id` recorded, the old guard read the
/// destination, found no shared root and allowed the move — leaving the F
/// subtree without adoption. The source is unknown here; the guard must refuse.
#[test]
fn review_p1_2_an_accept_that_moved_the_row_first_still_refuses_the_move() {
    let (fixture, seen) = bare_world();
    let temp = tempfile::tempdir().unwrap();
    let sync_root = temp.path().join("sync");
    std::fs::create_dir_all(&sync_root).unwrap();
    let db = Arc::new(StateDb::open(temp.path().join("db")).unwrap());
    let bridge = crate::engine_bridge::EngineBridge::new(db, Arc::new(fixture.api()));
    bridge.db().remember_folder_custody_at_revision(ROOT, 0).unwrap();
    seed(&bridge, ROOT, ItemKind::Folder, None);
    seed(&bridge, PRIVATE, ItemKind::Folder, None);
    // The accept already moved the row to the destination.
    seed(&bridge, GUEST, ItemKind::File, Some(PRIVATE));
    let payload = temp.path().join("payload.bin");
    std::fs::write(&payload, b"abcd").unwrap();
    let guest = FileKey::from_bytes([66; 32]);
    let name = beebeeb_core::encrypt::encrypt_name_for(&guest, "added.txt", None, ExpectedChunkFormat::Unrecorded).unwrap();
    let metadata = json!({
        "name_encrypted": name,
        "content_type": "text/plain",
        "parent_changed": true,
    });
    let op = crate::state_db::PendingOperation {
        op_id: "op-p1-2-accept".into(),
        kind: crate::state_db::OperationKind::UploadVersion,
        file_id: Some(GUEST.into()),
        parent_id: Some(PRIVATE.into()),
        target_path: Some("Private/added.txt".into()),
        metadata_json: Some(metadata.to_string()),
        payload_path: Some(payload.to_string_lossy().into_owned()),
        base_version: None,
        base_object_version_id: None,
        attempts: 0,
        max_attempts: 25,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: 1,
        updated_at: 1,
    };
    bridge.db().enqueue_operation(&op).unwrap();
    let outcome = run(bridge.process_due_operations(&sync_root, i64::MAX / 2)).unwrap();
    assert!(
        outcome.retried_op_ids.contains(&op.op_id) || outcome.paused_op_ids.contains(&op.op_id),
        "the upload did not land: {outcome:?}"
    );
    let error = bridge.db().get_operation(&op.op_id).unwrap().unwrap().last_error.unwrap_or_default();
    assert!(error.contains("move needs adoption"), "{error}");
    assert_eq!(count(&seen, &format!("PATCH /api/v1/files/{GUEST}")), 0, "{seen:?}");
}

/// P1-2/P2-4: a rehomed kept row has no parent at all, so its source is
/// unknown too. A move of a subtree that holds F ids is refused, never allowed
/// on the destination's word.
#[test]
fn review_p1_2_a_rehomed_kept_row_with_a_foreign_subtree_is_refused() {
    const KEPT: &str = "40000000-0000-4000-8000-0000000000c6";
    const GUEST_TWO: &str = "20000000-0000-8000-8000-0000000000c7";
    let (fixture, _seen) = bare_world();
    let temp = tempfile::tempdir().unwrap();
    let bridge = bare_bridge(temp.path(), &fixture);
    seed(&bridge, KEPT, ItemKind::Folder, None);
    seed(&bridge, GUEST_TWO, ItemKind::File, Some(KEPT));
    let error = run(bridge.adopt_foreign_subtree(KEPT, None, Some(PRIVATE)))
        .expect_err("a rehomed kept row's source is unknown; its F subtree is not adopted blind");
    assert!(format!("{error}").contains("move needs adoption"), "{error}");
}

/// More than 10,000 foreign items under a moved folder is refused with a
/// message that says so, never a silent stop.
#[test]
fn review_p2_7_a_move_of_more_than_ten_thousand_foreign_items_is_refused() {
    let (fixture, _seen) = bare_world();
    let temp = tempfile::tempdir().unwrap();
    let bridge = bare_bridge(temp.path(), &fixture);
    for index in 0..10_001u32 {
        let id = format!("20000000-0000-8000-8000-{index:012x}");
        seed(&bridge, &id, ItemKind::File, Some(OWNER_FOLDER));
    }
    let error = run(bridge.adopt_foreign_subtree(OWNER_FOLDER, Some(ROOT), None))
        .expect_err("above the bound the move is refused");
    let text = format!("{error}");
    assert!(text.contains("above the 10,000"), "{text}");
}
