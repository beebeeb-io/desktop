//! `hydrate_files` engine tests, split out of `engine_bridge.rs`.

use super::*;
/// Task 1247 P0 follow-up: the SAME race proven end-to-end through the real
/// `hydrate_file`. A broken symlink at the destination passes the top-of-fn
/// containment guard (its `exists()` is false → not-yet-exists branch: parent
/// contained + single-component name), then a full download+decrypt runs, and
/// only the O_NOFOLLOW write stops the decrypted plaintext from being written
/// through the symlink to a target outside the allowed root. Load-bearing:
/// with a plain `fs::write` the outside target WOULD be created.
#[cfg(unix)]
#[test]
fn hydrate_file_fails_closed_on_symlink_destination_toctou() {
    let dir = tempfile::tempdir().unwrap(); // the allowed root
    let outside = tempfile::tempdir().unwrap(); // outside every allowed root
    let master_key = [9u8; 32];
    let file_key = hydration_test_key(master_key, TEST_FILE_ID);

    let plaintext = b"decrypted-vault-plaintext-must-not-escape".to_vec();
    let server = IpcHydrationMock::start(file_key, vec![plaintext.clone()]);

    let db = Arc::new(StateDb::open(dir.path().join("state.db")).unwrap());
    let api = Arc::new(ApiClient::new(server.base_url.clone(), "token".into(), master_key));
    let bridge = Arc::new(EngineBridge::new(db.clone(), api));
    seed_bridge_row(
        &bridge,
        TEST_FILE_ID,
        "/legit.bin",
        None,
        FileStatus::CloudOnly,
        plaintext.len() as i64,
    );

    // Attacker plants a symlink at the guard-passing destination pointing to a
    // not-yet-existing file OUTSIDE the allowed root (simulating the swap
    // landing during the do_hydrate window — a broken symlink so the guard's
    // exists() check is false and it passes containment).
    let planted_target = outside.path().join("authorized_keys");
    let dest = dir.path().join("bb_target_link");
    std::os::unix::fs::symlink(&planted_target, &dest).unwrap();
    assert!(
        !planted_target.exists(),
        "precondition: symlink target must not exist yet"
    );
    // Sanity: the destination genuinely passes the containment guard, so this
    // test really is exercising the write-site O_NOFOLLOW defense, not the guard.
    assert!(
        hydrate_dest_is_allowed(&dest, &[dir.path()]),
        "the symlink destination must pass the containment guard (so only O_NOFOLLOW can stop it)"
    );

    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(async { bridge.hydrate_file(TEST_FILE_ID, &dest, &[dir.path()]).await });

    assert!(
        result.is_err(),
        "hydrate_file must fail closed on a symlink destination, got {result:?}"
    );
    assert!(
        !planted_target.exists(),
        "O_NOFOLLOW must prevent following the planted symlink — decrypted plaintext must NOT reach the outside target"
    );

    server.stop_and_count();
}

// ── Task 1698 part 2: trash semantics (ruling: full sync) ────────────────

#[test]
fn test_1698_hydrate_final_status_keeps_the_trashing_marker() {
    // Opening a trashed file from the trash view hydrates it; the marker
    // must survive (a `Local` flip would move the item OUT of the trash
    // container in the replica).
    assert_eq!(
        hydrate_final_status(FileStatus::Trashing),
        FileStatus::Trashing,
        "a Trashing row stays Trashing after a successful hydrate"
    );
    assert_eq!(hydrate_final_status(FileStatus::CloudOnly), FileStatus::Local);
    assert_eq!(hydrate_final_status(FileStatus::Downloading), FileStatus::Local);
}

#[test]
// `file_provider_change_payloads`/`FP_TRASH_APPLE` live in `ipc_socket`,
// which is `#[cfg(unix)]` (lib.rs) — this test compiles out on Windows.
#[cfg(unix)]
fn test_1698_queue_finder_delete_parks_trashing_immediately() {
    // The Finder-side trash: `queue_finder_delete` parks the row
    // `Trashing` AFTER enqueueing the op, so the change feed immediately
    // presents the item under the trash container (ruling step 2), while
    // a failed enqueue leaves the row untouched.
    let dir = tempfile::tempdir().unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    let file = "qfd00000-0000-4000-8000-000000000001";
    seed_bridge_entry(&bridge, file, "doomed.txt", None, FileStatus::Local, false, 10);

    let outcome = bridge.queue_finder_delete(file, None).unwrap();
    assert!(matches!(
        outcome,
        crate::engine_bridge::FinderWriteOutcome::Queued { .. }
    ));
    let row = bridge.db().get_file(file).unwrap().unwrap();
    assert_eq!(row.status, FileStatus::Trashing, "the row parks Trashing right away");
    assert_eq!(
        bridge.db().list_due_operations(now_secs()).unwrap().len(),
        1,
        "the server trash op is queued"
    );
    let changes = bridge.db().list_file_changes_paged(None, 100).unwrap().unwrap().0;
    let payloads = crate::ipc_socket::file_provider_change_payloads(bridge.db(), changes);
    let last = payloads.last().and_then(|change| change.item.as_ref()).unwrap();
    assert_eq!(last.status, "trashing");
    assert_eq!(last.parent_identifier, crate::ipc_socket::FP_TRASH_APPLE);
}

#[test]
fn test_1698_queue_finder_delete_unknown_item_is_an_idempotent_success() {
    // Ruling: "unknown items report success" — the item may have been
    // deleted remotely already. No op is queued, no row is touched.
    let dir = tempfile::tempdir().unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    let outcome = bridge
        .queue_finder_delete("does0000-0000-4000-8000-000000000000", None)
        .unwrap();
    assert!(
        matches!(outcome, crate::engine_bridge::FinderWriteOutcome::Ignored { .. }),
        "an unknown item must be an Ignored (idempotent success), got {outcome:?}"
    );
    assert!(
        bridge.db().list_due_operations(now_secs()).unwrap().is_empty(),
        "no trash op is queued for an unknown item"
    );
}

#[test]
fn test_1698_remote_file_trash_echo_marks_subtree_trashing_not_deleted() {
    // Ruling step 3: a REMOTE trash (another device trashed the file) is
    // the server trash — the mirror must flip the subtree to `Trashing`
    // (trash view), NOT delete the rows.
    let dir = tempfile::tempdir().unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    let folder = "fold0000-0000-4000-8000-000000000001";
    let child = "chil0000-0000-4000-8000-000000000002";
    seed_bridge_entry(&bridge, folder, "docs", None, FileStatus::CloudOnly, true, 10);
    seed_bridge_entry(
        &bridge,
        child,
        "docs/notes.txt",
        Some(folder),
        FileStatus::CloudOnly,
        false,
        10,
    );

    let op = crate::api_client::SyncOp {
        seq_id: 4,
        op_type: "file_trash".into(),
        payload: serde_json::json!({ "id": folder }),
    };
    let mut conflicts = Vec::new();
    let applied = apply_sync_op(&bridge, dir.path(), &op, 200, &mut conflicts).unwrap();
    assert!(
        applied.contains(&folder.to_string()) && applied.contains(&child.to_string()),
        "the flip reports the folder AND its descendants for the working-set signal: {applied:?}"
    );
    for id in [folder, child] {
        let row = bridge
            .db()
            .get_file(id)
            .unwrap()
            .expect("the row SURVIVES (trash view)");
        assert_eq!(row.status, FileStatus::Trashing, "{id} must be Trashing, not deleted");
    }
}

#[test]
fn test_1698_file_delete_echo_removes_trashing_rows_permanently() {
    // `file_delete` = server-side PERMANENT delete (app trash view / web /
    // retention janitor). A Trashing row must leave the trash view: the
    // row is deleted and a `deleted` change is recorded.
    let dir = tempfile::tempdir().unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    let file = "perm0000-0000-4000-8000-000000000001";
    seed_bridge_entry(&bridge, file, "doomed.txt", None, FileStatus::Trashing, false, 10);

    let op = crate::api_client::SyncOp {
        seq_id: 5,
        op_type: "file_delete".into(),
        payload: serde_json::json!({ "id": file }),
    };
    let mut conflicts = Vec::new();
    let applied = apply_sync_op(&bridge, dir.path(), &op, 200, &mut conflicts).unwrap();
    assert!(
        bridge.db().get_file(file).unwrap().is_none(),
        "the permanently-deleted row leaves the mirror"
    );
    assert!(
        applied.contains(&file.to_string()),
        "the removal is reported for the working set"
    );
    let changes = bridge.db().list_file_changes_paged(None, 100).unwrap().unwrap().0;
    assert!(
        changes
            .iter()
            .any(|change| change.file_id == file && change.kind == crate::state_db::FpChangeKind::Deleted),
        "a `deleted` change must reach the feed so the replica drops the item"
    );
}

#[test]
fn test_1698_file_restore_echo_un_trashes_the_row() {
    // Un-trashing (ruling: reparent back out of the trash container → the
    // server's existing restore path). A Trashing row flips back to
    // CloudOnly (placeholder re-mints; content re-downloads on open).
    let dir = tempfile::tempdir().unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    let file = "rest0000-0000-4000-8000-000000000001";
    seed_bridge_entry(&bridge, file, "restored.txt", None, FileStatus::Trashing, false, 10);

    let op = crate::api_client::SyncOp {
        seq_id: 6,
        op_type: "file_restore".into(),
        payload: serde_json::json!({ "id": file }),
    };
    let mut conflicts = Vec::new();
    let applied = apply_sync_op(&bridge, dir.path(), &op, 200, &mut conflicts).unwrap();
    let row = bridge.db().get_file(file).unwrap().unwrap();
    assert_eq!(
        row.status,
        FileStatus::CloudOnly,
        "the restored row leaves the trash view"
    );
    assert!(
        applied.contains(&file.to_string()),
        "the flip is reported for the working set"
    );
}

#[test]
fn test_1698_review_hydrate_keeps_a_trashed_row_trashed_after_a_successful_open() {
    // PR #100 review (Codex P1, engine_bridge.rs ~2500): the pre-hydrate
    // status must be the row's status BEFORE the flip to `Downloading`.
    // The trash-view scenario — the user opens a file from the macOS
    // Trash — re-read the DB AFTER `hydrate_file_with_progress` had
    // already flipped the row, so it always saw `Downloading`,
    // `hydrate_final_status` returned `Local`, and the item reparented
    // itself OUT of the trash view (the `Trashing` marker lost).
    let dir = tempfile::tempdir().unwrap();
    let master_key = [7u8; 32];
    let file_key = hydration_test_key(master_key, TEST_FILE_ID);
    // 1 metadata GET + 1 chunk GET
    // + 1 discovery GET (task 2173): this server has no direct route (a JSON 404), so the API path serves it.
    let server = HydrationMockServer::start(file_key, vec![vec![b't'; 8]], 3);
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
    seed_bridge_row(&bridge, TEST_FILE_ID, "trash-view.txt", None, FileStatus::Trashing, 8);
    let dest = dir.path().join("trash-view.txt");
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(async {
            bridge
                .hydrate_file_with_progress(TEST_FILE_ID, &dest, &[dir.path()], None)
                .await
        })
        .unwrap();
    server.finish();
    let row = bridge.db.get_file(TEST_FILE_ID).unwrap().unwrap();
    assert_eq!(
        row.status,
        FileStatus::Trashing,
        "opening a file from the macOS Trash view must NOT un-trash it"
    );
}
