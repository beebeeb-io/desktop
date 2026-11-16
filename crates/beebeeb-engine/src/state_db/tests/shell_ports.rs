//! Ports of the state-database tests the old shell crate kept (`state_db_shell_tests.rs`): what engine start does with
//! the staging journal's released copies, and what a revoked share's purge releases. Both are engine behaviour; the
//! release journal is written and read on macOS only, so the two ports are macOS-only too, and one test per
//! other platform pins that the journal is left alone there.

use super::*;

#[cfg(target_os = "macos")]
fn seed_contract_row(db: &StateDb, file_id: &str, path: &str, status: FileStatus, cache_bytes: i64) {
    db.upsert_file(&FileEntry {
        file_id: file_id.into(),
        path: path.into(),
        status,
        size_bytes: cache_bytes,
        modified_at: 0,
        content_hash: None,
        remote_updated_at: 0,
        parent_id: None,
        item_kind: ItemKind::File,
    })
    .unwrap();
    let mut contract = db.get_file_contract_state(file_id).unwrap().unwrap();
    contract.parent_id = None;
    contract.cache_bytes = cache_bytes;
    db.set_file_contract_state(&contract).unwrap();
}

fn queued(op_id: &str, kind: OperationKind, file_id: &str, payload: Option<&str>) -> PendingOperation {
    PendingOperation {
        op_id: op_id.into(),
        kind,
        file_id: Some(file_id.into()),
        parent_id: None,
        target_path: None,
        metadata_json: None,
        payload_path: payload.map(str::to_string),
        base_version: None,
        base_object_version_id: None,
        attempts: 0,
        max_attempts: 25,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: 0,
        updated_at: 0,
    }
}

/// Ported from `state_db_shell_tests::a_revoked_shares_queued_copy_is_released_and_removed_at_the_next_start`.
///
/// P2-1: purging a revoked share's content marks its queued ops' copies released in the same transaction, so the next
/// start's release path removes them. Before, the op went and its plaintext copy stayed until a sign-out.
#[cfg(target_os = "macos")]
#[test]
fn a_revoked_shares_queued_copy_is_released_and_removed_at_the_next_start() {
    let temp = tempfile::tempdir().unwrap();
    let db = StateDb::open(temp.path().join("state.db")).unwrap();
    seed_contract_row(&db, "revoked", "/Shared with me/Revoked", FileStatus::Local, 20);
    let mut revoked = db.get_file_contract_state("revoked").unwrap().unwrap();
    revoked.namespace = Namespace::SharedWithMe;
    revoked.shared_root_id = Some("revoked".into());
    revoked.share_id = Some("invite-revoked".into());
    revoked.permission_bits = PERMISSION_READ | PERMISSION_WRITE;
    db.set_file_contract_state(&revoked).unwrap();
    let copy = temp.path().join("copy");
    std::fs::write(&copy, b"a save to a share that was revoked").unwrap();
    let copy_path = copy.to_string_lossy().into_owned();
    db.track_staged_payload(&copy_path, Some("/handed-over"), false)
        .unwrap();
    db.enqueue_operation(&PendingOperation {
        base_version: Some(1),
        max_attempts: 25,
        created_at: 1,
        updated_at: 1,
        ..queued("op-revoked", OperationKind::UploadVersion, "revoked", Some(&copy_path))
    })
    .unwrap();

    db.purge_revoked_shared_content(&[]).unwrap();

    assert!(db.get_operation("op-revoked").unwrap().is_none(), "the op is gone");
    assert_eq!(
        db.staged_payloads_for_signout().unwrap(),
        vec![(copy_path.clone(), Some("/handed-over".to_string()), true)],
        "its copy is released in the purge's transaction"
    );
    let repair = db.engine_start_repair().unwrap();
    assert_eq!(repair.released_payloads, vec![copy_path]);
    assert_eq!(
        crate::staged_payload::remove_released(&db, &repair.released_payloads),
        1
    );
    assert!(!copy.exists(), "the next start removed it");
    assert!(db.staged_payloads_for_signout().unwrap().is_empty());
}

/// Ported from `state_db_shell_tests::engine_start_unlinks_released_copies_in_the_new_and_the_old_root_and_keeps_live_ones`.
///
/// Spec §8.4 (the staging folder) and §8.7 S6: engine start unlinks a released copy wherever it was staged, by its
/// journalled absolute path: in the data root, and in the old cache root that builds before the move used. A copy a
/// queued op still owns stays in each root, with its journal row.
#[cfg(target_os = "macos")]
#[test]
fn engine_start_unlinks_released_copies_in_the_new_and_the_old_root_and_keeps_live_ones() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    let bases = crate::engine_bridge::FinderStagingBases::current(&crate::identity::TEST);
    let mut released = Vec::new();
    let mut live = Vec::new();
    for (label, root) in [
        ("new", bases.durable_root().expect("the test sandbox has a data dir")),
        ("old", bases.cache_root()),
    ] {
        std::fs::create_dir_all(&root).unwrap();
        // Landed: marked released in the journal, and no op, resume row or finalization names it.
        let done = root.join(uuid::Uuid::new_v4().to_string());
        std::fs::write(&done, b"landed").unwrap();
        db.track_staged_payload(&done.to_string_lossy(), None, true).unwrap();
        // Still queued: an upload reads it.
        let queued_copy = root.join(uuid::Uuid::new_v4().to_string());
        std::fs::write(&queued_copy, b"waiting").unwrap();
        db.track_staged_payload(&queued_copy.to_string_lossy(), Some("/source"), false)
            .unwrap();
        db.enqueue_operation(&queued(
            &format!("live-{label}"),
            OperationKind::UploadVersion,
            &format!("f-{label}"),
            Some(&queued_copy.to_string_lossy()),
        ))
        .unwrap();
        released.push(done);
        live.push(queued_copy);
    }

    let repair = db.engine_start_repair().unwrap();
    let removed = crate::staged_payload::remove_released(&db, &repair.released_payloads);

    assert_eq!(removed, 2, "one released copy in each root");
    for path in &released {
        assert!(!path.exists(), "a released copy is unlinked: {path:?}");
    }
    for path in &live {
        assert!(path.exists(), "a copy a queued op owns stays: {path:?}");
    }
    let mut journalled: Vec<String> = db
        .staged_payloads_for_signout()
        .unwrap()
        .into_iter()
        .map(|(path, _, _)| path)
        .collect();
    journalled.sort();
    let mut expected: Vec<String> = live.iter().map(|path| path.to_string_lossy().into_owned()).collect();
    expected.sort();
    assert_eq!(journalled, expected, "only the live copies keep their journal rows");
    for path in &live {
        std::fs::remove_file(path).unwrap();
    }
}

/// New (the other-platform half of the two ports above): off macOS the start lists no released copy, and a copy whose
/// upload is done keeps its journal row, so the sign-out's sweep still finds it. Fails if the release journal is ever
/// read there without the port being widened.
#[cfg(not(target_os = "macos"))]
#[test]
fn off_macos_engine_start_lists_no_released_copy_and_keeps_the_journal_row() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    let landed = dir.path().join("landed");
    std::fs::write(&landed, b"landed").unwrap();
    let landed_path = landed.to_string_lossy().into_owned();
    db.track_staged_payload(&landed_path, None, true).unwrap();
    db.enqueue_operation(&queued("other", OperationKind::UploadVersion, "f-other", None))
        .unwrap();

    let repair = db.engine_start_repair().unwrap();

    assert!(
        repair.released_payloads.is_empty(),
        "nothing is listed for removal off macOS"
    );
    assert!(landed.exists(), "the copy is not touched by the start");
    assert_eq!(
        db.staged_payloads_for_signout().unwrap(),
        vec![(landed_path, None, true)],
        "its journal row stays for the sign-out"
    );
}
