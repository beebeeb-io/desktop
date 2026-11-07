//! LD-0113 (spec §5.4, §6.3): the Backups root's `system_role` reaches `state.db`, and the engine
//! never sends a change of a protected row (fixture rows only; nothing here talks to a real server).

use super::*;

const ROOT_ID: &str = "0b5ec0de-0000-4000-8000-0000000000a1";

// The fixture seals the name the way the other metadata tests do (a legacy sealer is fine for a fixture).
#[allow(deprecated)]
fn root_row(master_key: [u8; 32], extra: serde_json::Value) -> serde_json::Value {
    let mk = beebeeb_core::kdf::MasterKey::from_bytes(master_key);
    let name_encrypted = beebeeb_core::encrypt::encrypt_name(&mk, ROOT_ID, "Backups", None).unwrap();
    let mut row = serde_json::json!({
        "id": ROOT_ID,
        "name_encrypted": name_encrypted,
        "size_bytes": 0,
        "updated_at": 7,
        "is_folder": true,
    });
    for (key, value) in extra.as_object().unwrap() {
        row[key] = value.clone();
    }
    row
}

fn apply(db: &StateDb, row: &serde_json::Value, master_key: [u8; 32]) {
    apply_metadata_file_row(
        db,
        row,
        Namespace::MyFiles,
        None,
        None,
        PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER,
        100,
        &master_key,
        "",
    )
    .unwrap()
    .unwrap();
}

/// The listing's field sets and clears the role; a row without the field (an older server, or a row
/// this desktop builds from a sparse op) keeps it; the `folder_system_role` op sets and clears it.
#[test]
fn system_role_follows_the_listing_field_and_the_op() {
    let dir = tempfile::tempdir().unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    let master_key = [7u8; 32];
    let db = bridge.db();

    apply(db, &root_row(master_key, serde_json::json!({})), master_key);
    assert_eq!(db.system_role(ROOT_ID).unwrap(), None, "an older server's row has no role");
    assert!(db.backups_roots().unwrap().is_empty());

    apply(db, &root_row(master_key, serde_json::json!({ "system_role": "backups_root" })), master_key);
    assert_eq!(db.system_role(ROOT_ID).unwrap(), Some(beebeeb_types::backup::SystemRole::BackupsRoot));
    assert_eq!(db.backups_roots().unwrap(), vec![(ROOT_ID.to_string(), "Backups".to_string())]);

    apply(db, &root_row(master_key, serde_json::json!({})), master_key);
    assert_eq!(
        db.system_role(ROOT_ID).unwrap(),
        Some(beebeeb_types::backup::SystemRole::BackupsRoot),
        "a row without the field keeps the role"
    );

    apply(db, &root_row(master_key, serde_json::json!({ "system_role": null })), master_key);
    assert_eq!(db.system_role(ROOT_ID).unwrap(), None, "an explicit null clears it");

    apply(db, &root_row(master_key, serde_json::json!({ "system_role": "Backups" })), master_key);
    assert_eq!(db.system_role(ROOT_ID).unwrap(), None, "a folder NAME is never a role");

    let op = |role: serde_json::Value| crate::api_client::SyncOp {
        seq_id: 9,
        op_type: "folder_system_role".into(),
        payload: serde_json::json!({ "id": ROOT_ID, "system_role": role }),
    };
    let applied = apply_sync_op(&bridge, dir.path(), &op(serde_json::json!("backups_root")), 100, &mut Vec::new()).unwrap();
    assert_eq!(applied, vec![ROOT_ID.to_string()], "the op reports the row it changed");
    assert_eq!(db.system_role(ROOT_ID).unwrap(), Some(beebeeb_types::backup::SystemRole::BackupsRoot));
    let again = apply_sync_op(&bridge, dir.path(), &op(serde_json::json!("backups_root")), 100, &mut Vec::new()).unwrap();
    assert!(again.is_empty(), "the same role again changes nothing");
    apply_sync_op(&bridge, dir.path(), &op(serde_json::Value::Null), 100, &mut Vec::new()).unwrap();
    assert_eq!(db.system_role(ROOT_ID).unwrap(), None, "the op clears it");
}

fn flagged_root(bridge: &EngineBridge, status: FileStatus) {
    bridge
        .db
        .upsert_file(&FileEntry {
            file_id: ROOT_ID.into(),
            path: "Backups".into(),
            status,
            size_bytes: 0,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: ItemKind::Folder,
        })
        .unwrap();
    bridge
        .db
        .set_system_role(ROOT_ID, Some(beebeeb_types::backup::SystemRole::BackupsRoot))
        .unwrap();
}

fn trash_op(op_id: &str) -> PendingOperation {
    PendingOperation {
        op_id: op_id.into(),
        kind: OperationKind::TrashFile,
        file_id: Some(ROOT_ID.into()),
        parent_id: None,
        target_path: None,
        metadata_json: Some(serde_json::json!({ "operation": "trash" }).to_string()),
        payload_path: None,
        base_version: None,
        base_object_version_id: None,
        attempts: 0,
        max_attempts: 25,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: 100,
        updated_at: 100,
    }
}

/// Spec §6.3: an op the server refuses with `protected_folder` (one an older client queued, or one
/// queued before this device knew the flag) is dropped at once, never retried or held, and the row
/// is live again. Any other 409 keeps today's retry.
#[tokio::test]
async fn a_protected_folder_refusal_drops_the_op_and_puts_the_row_back() {
    let dir = tempfile::tempdir().unwrap();
    let server = SyncMockServer::start(vec![
        (
            "409 Conflict".into(),
            serde_json::json!({ "error": "protected_folder", "message": beebeeb_core::backup::MSG_DELETE }),
        ),
        ("409 Conflict".into(), serde_json::json!({ "error": "conflict" })),
    ]);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [9u8; 32]);
    flagged_root(&bridge, FileStatus::Trashing);
    bridge.db.enqueue_operation(&trash_op("op-protected")).unwrap();

    let outcome = bridge.process_due_operations(dir.path(), 200).await.unwrap();
    assert!(outcome.completed_op_ids.is_empty(), "a refusal is not a completion");
    assert!(outcome.retried_op_ids.is_empty(), "a refusal is never retried");
    assert!(outcome.paused_op_ids.is_empty(), "a refusal is never held");
    assert!(bridge.db.get_operation("op-protected").unwrap().is_none(), "the op is dropped");
    assert_eq!(bridge.db.get_file(ROOT_ID).unwrap().unwrap().status, FileStatus::CloudOnly, "the row is live again");

    // The same 409 status with another body is not a protection refusal: the op stays and retries.
    bridge.db.set_status(ROOT_ID, FileStatus::Trashing).unwrap();
    bridge.db.enqueue_operation(&trash_op("op-other-conflict")).unwrap();
    let outcome = bridge.process_due_operations(dir.path(), 300).await.unwrap();
    assert_eq!(outcome.retried_op_ids, vec!["op-other-conflict".to_string()]);
    assert!(bridge.db.get_operation("op-other-conflict").unwrap().is_some());
    let requests = server.finish();
    assert_eq!(requests.len(), 2, "one trash request per op");
}

/// Spec §6.3 (refused after the fact): a local delete of the root that got through queues nothing for
/// the server and leaves the row live; an ordinary folder's delete still queues its trash.
#[tokio::test]
async fn a_delete_of_the_root_that_got_through_queues_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("root");
    std::fs::create_dir_all(&sync_root).unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    flagged_root(&bridge, FileStatus::CloudOnly);
    bridge
        .db
        .upsert_file(&FileEntry {
            file_id: "0b5ec0de-0000-4000-8000-0000000000a2".into(),
            path: "Documents".into(),
            status: FileStatus::CloudOnly,
            size_bytes: 0,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: ItemKind::Folder,
        })
        .unwrap();

    crate::watcher::handle_delete(&bridge, &sync_root, &sync_root.join("Backups")).await;
    assert!(bridge.db.list_operations_for_file(ROOT_ID).unwrap().is_empty(), "nothing is queued for the root");
    assert_eq!(bridge.db.get_file(ROOT_ID).unwrap().unwrap().status, FileStatus::CloudOnly, "the root stays live");

    crate::watcher::handle_rename(&bridge, &sync_root, &sync_root.join("Backups"), &sync_root.join("Old backups")).await;
    assert!(bridge.db.list_operations_for_file(ROOT_ID).unwrap().is_empty(), "nor for its rename");

    crate::watcher::handle_delete(&bridge, &sync_root, &sync_root.join("Documents")).await;
    assert_eq!(
        bridge.db.list_operations_for_file("0b5ec0de-0000-4000-8000-0000000000a2").unwrap().len(),
        1,
        "an ordinary folder's delete still queues its trash"
    );
}

/// Self-review P1: the engine's own renames inside this device's backup (a kept copy's temp name
/// becoming `a (this PC).jpg`, a conflict rename) are never put back as a protected change; the same
/// rename by someone else is.
#[tokio::test]
async fn the_engines_own_rename_inside_an_own_backup_is_never_put_back() {
    use crate::watcher::{ProtectionCheck, handle_rename_with};
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("root");
    let pictures = sync_root.join("Backups").join("GLegion").join("Pictures");
    std::fs::create_dir_all(&pictures).unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    flagged_root(&bridge, FileStatus::CloudOnly);
    let before = crate::backup_protect::own_backups();
    crate::backup_protect::set_own_backups(crate::backup_protect::OwnBackups {
        device_folder: Some("GLegion".into()),
        folders: vec![crate::backup_protect::OwnBackupFolder {
            display_name: "Pictures".into(),
            source_path: dir.path().join("src").join("Pictures"),
        }],
    });

    let (temp, kept) = (pictures.join(".a.tmp"), pictures.join("a (this PC).jpg"));
    std::fs::write(&kept, b"kept").unwrap();
    handle_rename_with(&bridge, &sync_root, &temp, &kept, ProtectionCheck::Skip).await;
    let engine_kept = kept.exists() && !temp.exists();

    let (original, renamed) = (pictures.join("b.jpg"), pictures.join("b2.jpg"));
    std::fs::write(&renamed, b"b").unwrap();
    handle_rename_with(&bridge, &sync_root, &original, &renamed, ProtectionCheck::Check).await;
    let person_put_back = original.exists() && !renamed.exists();

    crate::backup_protect::set_own_backups(before);
    assert!(engine_kept, "the engine's own rename stays where the engine put it");
    assert!(person_put_back, "someone else's rename of a backed-up file goes back");
}

fn live_row(bridge: &EngineBridge, file_id: &str, path: &str, kind: ItemKind) {
    bridge
        .db
        .upsert_file(&FileEntry {
            file_id: file_id.into(),
            path: path.into(),
            status: FileStatus::Local,
            size_bytes: if kind == ItemKind::Folder { 0 } else { 4 },
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: kind,
        })
        .unwrap();
}

/// A bridge whose API is a localhost listener that counts connections (never a real server).
fn counting_bridge(dir: &std::path::Path) -> (EngineBridge, std::net::TcpListener) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    (test_bridge_with_api(&dir.join("state.db"), base, [7u8; 32]), listener)
}

/// A transfer pass, then how many connections the listener saw.
async fn server_calls(bridge: &EngineBridge, sync_root: &std::path::Path, listener: &std::net::TcpListener) -> usize {
    let outcome = bridge.process_due_operations(sync_root, i64::MAX / 2).await.unwrap();
    assert!(outcome.completed_op_ids.is_empty() && outcome.retried_op_ids.is_empty(), "nothing was due");
    std::iter::from_fn(|| match listener.accept() {
        Ok(_) => Some(()),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => None,
        Err(error) => panic!("{error}"),
    })
    .count()
}

/// Amendment 6 ("Protect the whole tree"): a delete under the root that got through (the app was not
/// running) sends nothing, and every row of the deleted subtree whose bytes went with it, folders
/// included, is cloud-only again, so its placeholder is minted again. A row outside it is untouched.
#[tokio::test]
async fn a_delete_under_the_root_that_got_through_puts_the_subtree_back() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("root");
    std::fs::create_dir_all(sync_root.join("Docs")).unwrap();
    std::fs::write(sync_root.join("Docs").join("kept.txt"), b"kept").unwrap();
    let (bridge, listener) = counting_bridge(dir.path());
    flagged_root(&bridge, FileStatus::CloudOnly);
    let subtree = [
        ("0b5ec0de-0000-4000-8000-0000000000a5", "Backups/OtherPC", ItemKind::Folder),
        ("0b5ec0de-0000-4000-8000-0000000000a6", "Backups/OtherPC/Pictures", ItemKind::Folder),
        ("0b5ec0de-0000-4000-8000-0000000000a7", "Backups/OtherPC/Pictures/a.jpg", ItemKind::File),
    ];
    for (id, path, kind) in subtree.clone() {
        live_row(&bridge, id, path, kind);
    }
    let elsewhere = ("0b5ec0de-0000-4000-8000-0000000000a8", "Backups/GLegion/b.jpg");
    live_row(&bridge, elsewhere.0, elsewhere.1, ItemKind::File);
    let outside = ("0b5ec0de-0000-4000-8000-0000000000a9", "Docs/kept.txt");
    live_row(&bridge, outside.0, outside.1, ItemKind::File);

    crate::watcher::handle_delete(&bridge, &sync_root, &sync_root.join("Backups").join("OtherPC")).await;

    let status = |id: &str| bridge.db.get_file(id).unwrap().unwrap().status;
    let restored = subtree.iter().filter(|(id, _, _)| status(id) == FileStatus::CloudOnly).count();
    assert_eq!(restored, subtree.len(), "every row of the deleted subtree is cloud-only again");
    for id in [ROOT_ID, subtree[0].0, subtree[1].0, subtree[2].0, elsewhere.0, outside.0] {
        assert!(bridge.db.list_operations_for_file(id).unwrap().is_empty(), "nothing is queued for {id}");
    }
    assert_eq!(status(elsewhere.0), FileStatus::Local, "a row beside the deleted subtree is untouched");
    assert_eq!(status(outside.0), FileStatus::Local, "a row outside the tree is untouched");
    let calls = server_calls(&bridge, &sync_root, &listener).await;
    assert_eq!(calls, 0, "zero server calls");
    println!("ld0113 subtree_restore: restored={restored} server_calls={calls}");
}

/// Amendment 6: a move from elsewhere in the drive INTO the tree that got through is not sent; the item
/// goes back. A rename outside the tree is sent as before.
#[tokio::test]
async fn a_move_into_the_tree_that_got_through_goes_back() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("root");
    std::fs::create_dir_all(sync_root.join("Docs")).unwrap();
    std::fs::create_dir_all(sync_root.join("Backups").join("OtherPC")).unwrap();
    let (bridge, listener) = counting_bridge(dir.path());
    flagged_root(&bridge, FileStatus::CloudOnly);
    let report = "0b5ec0de-0000-4000-8000-0000000000aa";
    live_row(&bridge, report, "Docs/report.pdf", ItemKind::File);
    let (source, target) = (sync_root.join("Docs").join("report.pdf"), sync_root.join("Backups").join("OtherPC").join("report.pdf"));
    std::fs::write(&target, b"pdf!").unwrap();

    crate::watcher::handle_rename(&bridge, &sync_root, &source, &target).await;
    assert!(source.exists() && !target.exists(), "the item is back where it was");
    assert!(bridge.db.list_operations_for_file(report).unwrap().is_empty(), "no move is queued");
    assert_eq!(bridge.db.get_file(report).unwrap().unwrap().path, "Docs/report.pdf");
    let calls = server_calls(&bridge, &sync_root, &listener).await;
    assert_eq!(calls, 0, "zero server calls");

    let renamed = sync_root.join("Docs").join("report-2.pdf");
    std::fs::rename(&source, &renamed).unwrap();
    crate::watcher::handle_rename(&bridge, &sync_root, &source, &renamed).await;
    assert_eq!(bridge.db.list_operations_for_file(report).unwrap().len(), 1, "an ordinary rename is still queued");
}

/// Self-review (2026-11-06): an item that cannot go back (its old folder refuses the write) is sent as
/// made. Minting it again at its old place would leave the moved copy untracked, to upload as a new
/// item into the tree.
#[cfg(unix)]
#[tokio::test]
async fn a_move_into_the_tree_that_cannot_go_back_is_sent_as_made() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("root");
    let docs = sync_root.join("Docs");
    std::fs::create_dir_all(&docs).unwrap();
    std::fs::create_dir_all(sync_root.join("Backups").join("OtherPC")).unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    flagged_root(&bridge, FileStatus::CloudOnly);
    live_row(&bridge, "0b5ec0de-0000-4000-8000-0000000000ab", "Backups/OtherPC", ItemKind::Folder);
    let report = "0b5ec0de-0000-4000-8000-0000000000ac";
    live_row(&bridge, report, "Docs/report.pdf", ItemKind::File);
    let (source, target) = (docs.join("report.pdf"), sync_root.join("Backups").join("OtherPC").join("report.pdf"));
    std::fs::write(&target, b"pdf!").unwrap();
    std::fs::set_permissions(&docs, std::fs::Permissions::from_mode(0o555)).unwrap();
    if std::fs::write(docs.join(".probe"), b"").is_ok() {
        std::fs::set_permissions(&docs, std::fs::Permissions::from_mode(0o755)).unwrap();
        println!("ld0113 cannot_go_back: skipped, this user writes through a read-only folder (root)");
        return;
    }

    crate::watcher::handle_rename(&bridge, &sync_root, &source, &target).await;
    std::fs::set_permissions(&docs, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(target.exists() && !source.exists(), "the move back was refused");
    assert_eq!(bridge.db.get_file(report).unwrap().unwrap().status, FileStatus::Local, "nothing is minted at its old place");
    assert_eq!(bridge.db.list_operations_for_file(report).unwrap().len(), 1, "the move is sent as made");
}

/// Review #1 (lead, 2026-11-06): the queueing functions every drive surface reaches (the Windows
/// watcher, the File Provider replies) never queue a trash, rename or move of an item in the tree, nor
/// a move into it, whatever the surface in front of them did; an ordinary rename queues as before.
#[tokio::test]
async fn the_queueing_functions_refuse_drive_changes_of_the_tree() {
    use crate::backup_protect::DriveChangeRefused;
    let dir = tempfile::tempdir().unwrap();
    let (bridge, _listener) = counting_bridge(dir.path());
    flagged_root(&bridge, FileStatus::CloudOnly);
    let other = "0b5ec0de-0000-4000-8000-0000000000b1";
    let photo = "0b5ec0de-0000-4000-8000-0000000000b2";
    let report = "0b5ec0de-0000-4000-8000-0000000000b3";
    live_row(&bridge, other, "Backups/OtherPC", ItemKind::Folder);
    live_row(&bridge, photo, "Backups/OtherPC/a.jpg", ItemKind::File);
    live_row(&bridge, report, "Docs/report.pdf", ItemKind::File);
    let target = |file_id: &str, parent_id: Option<&str>, filename: &str| FinderWriteTarget {
        file_id: Some(file_id.into()),
        parent_id: parent_id.map(Into::into),
        filename: filename.into(),
        rel_path: None,
        kind: FinderWriteItemKind::File,
        contents_path: None,
        content_type: None,
        base_version_identifier: None,
    };
    let refused = |result: anyhow::Result<FinderWriteOutcome>| {
        result.err().and_then(|error| engine_cause::<DriveChangeRefused>(&error).map(|refused| refused.message.clone()))
    };
    let managed = Some(beebeeb_core::backup::MSG_MANAGED_IN_APP.to_string());
    // A move into the tree is core's Create on the folder it goes into (v0.2.1): read-only.
    let read_only = Some(beebeeb_core::backup::MSG_READ_ONLY.to_string());

    assert_eq!(refused(bridge.queue_finder_delete(photo, None)), managed, "a trash under Backups/OtherPC");
    assert_eq!(refused(bridge.queue_finder_delete(ROOT_ID, None)).as_deref(), Some(beebeeb_core::backup::MSG_DELETE), "the root");
    assert_eq!(refused(bridge.queue_finder_modify(target(photo, None, "b.jpg"))), managed, "a rename in the tree");
    assert_eq!(refused(bridge.queue_finder_modify(target(report, Some(other), "report.pdf"))), read_only, "a move into the tree");
    assert_eq!(
        refused(bridge.queue_local_rename(target(report, Some(other), "report.pdf"), "Backups/OtherPC/report.pdf")),
        read_only,
        "the Windows re-keying rename into the tree"
    );
    let queued: usize = [ROOT_ID, other, photo, report].iter().map(|id| bridge.db.list_operations_for_file(id).unwrap().len()).sum();
    assert_eq!(queued, 0, "0 ops queued");

    assert!(
        matches!(bridge.queue_finder_modify(target(report, None, "report-2.pdf")), Ok(FinderWriteOutcome::Queued { .. })),
        "an ordinary rename is queued"
    );
    assert_eq!(bridge.db.list_operations_for_file(report).unwrap().len(), 1);
    assert_eq!(refused(bridge.queue_finder_modify(target(photo, None, "a.jpg"))), None, "no change of name or parent is not refused");
    assert_eq!(
        refused(bridge.queue_finder_modify(target(photo, Some("0b5ec0de-0000-4000-8000-0000000000bf"), "a.jpg"))),
        None,
        "the same name under a parent this device does not know shows no change (a content edit)"
    );
    // The Windows watcher applied the rule itself (the engine's own change, or a move sent as made).
    assert!(
        matches!(bridge.queue_finder_delete_decided(photo), Ok(FinderWriteOutcome::Queued { .. })),
        "a Decided change is queued"
    );
}
