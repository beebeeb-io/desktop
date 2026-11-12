//! State-database tests that need a shell module (`session_reauth`'s R10 reset, `staged_payload`'s removal,
//! the bridge's staging folders). They lived in `state_db`'s own test modules until the database moved to the
//! engine crate (`crates/beebeeb-engine`), which cannot name the shell; the bodies are unchanged.

use crate::state_db::*;
use tempfile::tempdir;

/// B05e (lead ruling 2026-11-09, desktop #265 delta 1): a reset of local data (R10's `reset`, the real one)
/// removes the share marks, the sealed and offered keys and the accepted fingerprints, and leaves no owner
/// and no account data. (That they survive a sign-out: `b05e_share_rows_survive_a_sign_out` in the engine.)
#[test]
fn b05e_a_reset_of_local_data_removes_the_share_rows() {
    const OWN: &str = "0b3c7a52-5d0e-4f1a-9d2e-1c6f3a8b9e01";
    const PEER: &str = "7f9e2d14-3a6b-4c8d-8e1f-2b4a6c8d0e12";
    const INVITE: &str = "6f1c2b9e-1d1a-4c55-9a51-3f7a2b8c9d01";
    const REFUSED: &str = "1a2b3c4d-5e6f-4a1b-8c2d-3e4f5a6b7c8d";
    const KEY: &str = "de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f";
    const FP: &str = "f35e5616160a30bf3c6e79fa73c576d40205e8fc3ba4e1c6dcf93e6b98e857b4";
    let fill = |db: &StateDb| {
        db.set_owner(&crate::account_binding::Identity::new(
            Some(OWN),
            Some("anna@example.com"),
        ))
        .unwrap();
        db.set_share_mark(OWN, PEER, FP, MarkKind::Matched).unwrap();
        db.set_share_email_mark(OWN, "jonas@northwind.studio", FP, MarkKind::Matched)
            .unwrap();
        db.set_share_sealed_key(OWN, INVITE, KEY).unwrap();
        db.set_share_offered_key(OWN, REFUSED, KEY).unwrap();
        db.set_share_accepted_fingerprint(OWN, INVITE, FP).unwrap();
        db.test_execute("INSERT INTO sync_state(key, value) VALUES ('cursor', '12')")
            .unwrap();
        assert!(db.has_account_data().unwrap(), "fixture: the cursor is account data");
    };
    let kept = |db: &StateDb| {
        [
            db.share_mark(OWN, PEER).unwrap().is_some(),
            db.share_email_mark(OWN, " Jonas@Northwind.Studio").unwrap().is_some(),
            db.share_sealed_key(OWN, INVITE).unwrap().is_some(),
            db.share_offered_key(OWN, REFUSED).unwrap().is_some(),
            db.share_accepted_fingerprint(OWN, INVITE).unwrap().is_some(),
        ]
        .iter()
        .filter(|k| **k)
        .count()
    };
    // The R10 reset itself (macOS and Linux reset another account's data before an engine starts).
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    fill(&db);
    let local = crate::session_reauth::StateDbLocalData {
        db,
        sync_root: None,
        staging_dirs: Vec::new(),
        kept: Default::default(),
    };
    crate::account_binding::LocalData::reset(&local).unwrap();
    assert_eq!(kept(&local.db), 0, "a reset of local data removes them");
    assert_eq!(local.db.owner().unwrap(), None);
    assert!(!local.db.has_account_data().unwrap());
}

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

#[cfg(target_os = "macos")]
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

/// P2-1: purging a revoked share's content marks its queued ops' copies released in the same
/// transaction, so the next start's release path removes them. Before, the op went and its
/// plaintext copy stayed until a sign-out.
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
        op_id: "op-revoked".into(),
        kind: OperationKind::UploadVersion,
        file_id: Some("revoked".into()),
        parent_id: None,
        target_path: None,
        metadata_json: None,
        payload_path: Some(copy_path.clone()),
        base_version: Some(1),
        base_object_version_id: None,
        attempts: 0,
        max_attempts: 25,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: 1,
        updated_at: 1,
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

/// Spec §8.4 (the staging folder) and §8.7 S6: engine start unlinks a released copy wherever it was staged,
/// by its journalled absolute path: in the data root, and in the old cache root that builds before the move
/// used. A copy a queued op still owns stays in each root, with its journal row. (The release journal is
/// macOS-only, so the test is too.)
#[cfg(target_os = "macos")]
#[test]
fn engine_start_unlinks_released_copies_in_the_new_and_the_old_root_and_keeps_live_ones() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    let bases = crate::engine_bridge::FinderStagingBases::current(crate::APP);
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
