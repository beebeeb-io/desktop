//! `metadata_2` engine tests, split out of `engine_bridge.rs`.

use super::*;
/// The `parent_id` of each `PATCH /files/{file_id}` the server got, in order: `None` when the
/// body had no `parent_id` key, else its value (`null` for the top level).
pub(super) fn patched_parents(state: &VersionedServerState, file_id: &str) -> Vec<Option<serde_json::Value>> {
    state
        .patches
        .iter()
        .filter(|(id, _)| id == file_id)
        .map(|(_, body)| body.get("parent_id").cloned())
        .collect()
}

/// A rename or move without contents, as the extension sends it.
fn fp_rename_or_move(bridge: &EngineBridge, file_id: &str, filename: &str, parent_id: Option<&str>) -> FpWrite {
    bridge
        .queue_file_provider_modify(FinderWriteTarget {
            file_id: Some(file_id.into()),
            parent_id: parent_id.map(str::to_string),
            filename: filename.into(),
            rel_path: None,
            kind: FinderWriteItemKind::File,
            contents_path: None,
            content_type: Some("text/plain".into()),
            base_version_identifier: Some("1".into()),
        })
        .unwrap()
}

/// Spec §7.2 (final review): a Finder move to the top level is a move. Its op names the top
/// level, its `PATCH` carries `"parent_id": null`, and the server's echo of it leaves the
/// row at the top level.
#[cfg(unix)] // names `crate::ipc_socket::list_file_provider_items`
#[tokio::test]
async fn a_finder_move_to_the_top_level_sends_null_and_ends_at_the_top_level() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync-root");
    std::fs::create_dir_all(&sync_root).unwrap();
    let master_key = [103u8; 32];
    let server = VersionedServerMock::start();
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
    seed_row_in_folder(&bridge, &server, "c95cb134-ba2c-52e1-93e9-25cb8b280748");
    fp_rename_or_move(&bridge, "c95cb134-ba2c-52e1-93e9-25cb8b280748", "at-top.txt", None);
    let ops = bridge
        .db
        .list_operations_for_file("c95cb134-ba2c-52e1-93e9-25cb8b280748")
        .unwrap();
    assert_eq!(
        (ops[0].kind.clone(), ops[0].parent_id.clone()),
        (OperationKind::MoveFile, None),
        "a move, to the top level"
    );

    drain_upload_queue(&bridge, &sync_root).await;
    let state = server.finish();
    assert_eq!(
        patched_parents(&state, "c95cb134-ba2c-52e1-93e9-25cb8b280748"),
        vec![Some(serde_json::Value::Null)],
        "the PATCH names the top level"
    );
    let name = state.patches[0].1["name_encrypted"].clone();
    // The server's echo, as `update_file` emits it: the move, then the rename.
    for (seq_id, op_type, payload) in [
        (
            501,
            "file_move",
            serde_json::json!({ "id": "c95cb134-ba2c-52e1-93e9-25cb8b280748", "old_parent_id": "14d2a297-b916-52d6-959d-7cc821b7a978", "new_parent_id": null }),
        ),
        (
            502,
            "file_rename",
            serde_json::json!({ "id": "c95cb134-ba2c-52e1-93e9-25cb8b280748", "old_name_encrypted": "x", "new_name_encrypted": name }),
        ),
    ] {
        let op = crate::api_client::SyncOp {
            seq_id,
            op_type: op_type.into(),
            payload,
        };
        apply_sync_op(&bridge, &sync_root, &op, now_secs(), &mut Vec::new()).unwrap();
    }
    assert_eq!(
        name_parent_status(&bridge, "c95cb134-ba2c-52e1-93e9-25cb8b280748"),
        ("at-top.txt".into(), None, FileStatus::Local),
        "at the top level"
    );
    assert!(listed_under(&bridge, FP_ROOT).contains(&"c95cb134-ba2c-52e1-93e9-25cb8b280748".to_string()));
    assert!(
        !listed_under(&bridge, "14d2a297-b916-52d6-959d-7cc821b7a978")
            .contains(&"c95cb134-ba2c-52e1-93e9-25cb8b280748".to_string())
    );
}

/// Spec §7.2 (final review): the watcher's rename tells "the root" from "not found". A file in
/// a folder, renamed under a directory with no folder row (a stale path after a folder
/// rename, a scan that has not written the folder yet, a database error), queues a rename
/// that leaves the parent alone: its `PATCH` has no `parent_id` key. Before, the miss read
/// as the root, and the move rule sent the file to the top level.
#[tokio::test]
async fn a_watcher_rename_under_an_unknown_folder_leaves_the_parent_alone() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync-root");
    std::fs::create_dir_all(sync_root.join("Docs")).unwrap();
    std::fs::create_dir_all(sync_root.join("Unknown")).unwrap();
    let master_key = [108u8; 32];
    let server = VersionedServerMock::start();
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
    seed_row_in_folder(&bridge, &server, "dbfd72f1-a5db-5d18-ad12-79b2013dc0b8");
    crate::watcher::handle_rename(
        &bridge,
        &sync_root,
        &sync_root.join("Docs").join("notes.txt"),
        &sync_root.join("Unknown").join("renamed.txt"),
    )
    .await;
    let ops = bridge
        .db
        .list_operations_for_file("dbfd72f1-a5db-5d18-ad12-79b2013dc0b8")
        .unwrap();
    assert_eq!(ops[0].kind, OperationKind::RenameFile, "a rename");
    // The sync root itself is still the top level: a move there names it.
    server.seed_file("a964aa59-fb2d-5ac2-9dd1-e25d15b90188", 1);
    bridge
        .db
        .upsert_file_with_contract(
            &FileEntry {
                file_id: "a964aa59-fb2d-5ac2-9dd1-e25d15b90188".into(),
                path: "Docs/other.txt".into(),
                status: FileStatus::Local,
                size_bytes: 10,
                modified_at: 1_700_000_000,
                content_hash: None,
                remote_updated_at: 1_700_000_000,
                parent_id: Some("14d2a297-b916-52d6-959d-7cc821b7a978".into()),
                item_kind: ItemKind::File,
            },
            |contract| {
                contract.parent_id = Some("14d2a297-b916-52d6-959d-7cc821b7a978".into());
                contract.current_version = 1;
            },
        )
        .unwrap();
    crate::watcher::handle_rename(
        &bridge,
        &sync_root,
        &sync_root.join("Docs").join("other.txt"),
        &sync_root.join("other.txt"),
    )
    .await;
    let ops = bridge
        .db
        .list_operations_for_file("a964aa59-fb2d-5ac2-9dd1-e25d15b90188")
        .unwrap();
    assert_eq!(
        (ops[0].kind.clone(), ops[0].parent_id.clone()),
        (OperationKind::MoveFile, None),
        "a move to the top level"
    );
    drain_upload_queue(&bridge, &sync_root).await;
    let state = server.finish();
    assert_eq!(
        patched_parents(&state, "dbfd72f1-a5db-5d18-ad12-79b2013dc0b8"),
        vec![None],
        "no parent_id key"
    );
    assert_eq!(
        patched_parents(&state, "a964aa59-fb2d-5ac2-9dd1-e25d15b90188"),
        vec![Some(serde_json::Value::Null)],
        "the top level"
    );
}

/// Spec §7.2 (final review): a rename in place leaves the parent alone: its `PATCH` has no
/// `parent_id` key.
#[tokio::test]
async fn a_rename_in_place_sends_no_parent() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync-root");
    std::fs::create_dir_all(&sync_root).unwrap();
    let master_key = [104u8; 32];
    let server = VersionedServerMock::start();
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
    seed_row_in_folder(&bridge, &server, "0fae1d7f-fc86-5d53-bf32-202636c2d9d7");
    fp_rename_or_move(
        &bridge,
        "0fae1d7f-fc86-5d53-bf32-202636c2d9d7",
        "renamed.txt",
        Some("14d2a297-b916-52d6-959d-7cc821b7a978"),
    );
    let ops = bridge
        .db
        .list_operations_for_file("0fae1d7f-fc86-5d53-bf32-202636c2d9d7")
        .unwrap();
    assert_eq!(ops[0].kind, OperationKind::RenameFile, "a rename");
    drain_upload_queue(&bridge, &sync_root).await;
    let state = server.finish();
    assert_eq!(
        patched_parents(&state, "0fae1d7f-fc86-5d53-bf32-202636c2d9d7"),
        vec![None],
        "the PATCH has no parent_id key"
    );
    assert!(state.patches[0].1.get("name_encrypted").is_some());
}

/// Spec §7.2 (final review): a save's upload sends a parent only when the save moved the
/// file: none for a save in place, `null` for a save that moved it to the top level.
#[cfg(unix)] // names `crate::ipc_socket::list_file_provider_items`
#[tokio::test]
async fn a_save_sends_a_parent_only_when_it_moved_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync-root");
    std::fs::create_dir_all(&sync_root).unwrap();
    let master_key = [105u8; 32];
    let server = VersionedServerMock::start();
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
    seed_row_in_folder(&bridge, &server, "89ea34e4-1bd4-5c43-87a5-3479bccfadfa");
    let in_place = fp_save_moved(
        &bridge,
        dir.path(),
        "89ea34e4-1bd4-5c43-87a5-3479bccfadfa",
        "notes.txt",
        "14d2a297-b916-52d6-959d-7cc821b7a978",
        b"in place",
        "1",
    );
    drain_upload_queue(&bridge, &sync_root).await;
    fp_save(
        &bridge,
        dir.path(),
        "89ea34e4-1bd4-5c43-87a5-3479bccfadfa",
        "notes.txt",
        b"moved to the top",
        &in_place.token.unwrap(),
    );
    assert_eq!(
        name_parent_status(&bridge, "89ea34e4-1bd4-5c43-87a5-3479bccfadfa"),
        ("notes.txt".into(), None, FileStatus::Uploading)
    );
    drain_upload_queue(&bridge, &sync_root).await;
    assert!(listed_under(&bridge, FP_ROOT).contains(&"89ea34e4-1bd4-5c43-87a5-3479bccfadfa".to_string()));
    let state = server.finish();
    assert_eq!(
        state.latest_plaintext("89ea34e4-1bd4-5c43-87a5-3479bccfadfa", master_key),
        b"moved to the top"
    );
    assert_eq!(
        patched_parents(&state, "89ea34e4-1bd4-5c43-87a5-3479bccfadfa"),
        vec![Some(serde_json::Value::Null)],
        "no metadata update for the save in place (the name travels with `complete`); null for the move to the top level"
    );
}

/// Spec §7.2 (final review): a save that follows a queued save which moved the file sends the
/// move too, so a move whose own save parks before its metadata step still reaches the
/// server.
#[tokio::test]
async fn a_save_after_a_parked_move_still_sends_the_move() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync-root");
    std::fs::create_dir_all(&sync_root).unwrap();
    let master_key = [106u8; 32];
    let server = VersionedServerMock::start();
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
    seed_row_in_folder(&bridge, &server, "543e440f-b7da-526d-be98-8b341fe8265d");
    // W moves the file to the top level; N, a save in place after it, does not.
    let w = fp_save(
        &bridge,
        dir.path(),
        "543e440f-b7da-526d-be98-8b341fe8265d",
        "notes.txt",
        b"W",
        "1",
    );
    fp_save(
        &bridge,
        dir.path(),
        "543e440f-b7da-526d-be98-8b341fe8265d",
        "notes.txt",
        b"W N",
        w.token.as_deref().unwrap(),
    );
    // W parks before its metadata step (its copy is gone), and N takes over.
    let w_op = bridge
        .db
        .list_operations_for_file("543e440f-b7da-526d-be98-8b341fe8265d")
        .unwrap()
        .remove(0);
    std::fs::remove_file(w_op.payload_path.as_deref().unwrap()).unwrap();
    drain_upload_queue(&bridge, &sync_root).await;
    let state = server.finish();
    assert_eq!(
        state.latest_plaintext("543e440f-b7da-526d-be98-8b341fe8265d", master_key),
        b"W N"
    );
    assert_eq!(
        patched_parents(&state, "543e440f-b7da-526d-be98-8b341fe8265d"),
        vec![Some(serde_json::Value::Null)],
        "N carried W's move to the top level"
    );
}
