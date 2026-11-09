//! Tests for the change log, owner records and account binding, split out of `state_db.rs`.

use super::*;

#[test]
fn bandwidth_samples_insert_and_history() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();

    // Empty history — no rows yet.
    let history = db.get_bandwidth_history(20).unwrap();
    assert!(history.is_empty(), "fresh DB should have no samples");

    // Insert a few samples at different times.
    let t0: i64 = 1_700_000_000; // arbitrary epoch anchor
    db.insert_bandwidth_sample(t0, 100, 200, 20).unwrap();
    db.insert_bandwidth_sample(t0 + 20, 300, 400, 20).unwrap();
    db.insert_bandwidth_sample(t0 + 40, 500, 600, 20).unwrap();

    // get_bandwidth_history_since(0) returns all rows regardless of wall clock,
    // making this test time-independent.
    let all = db.get_bandwidth_history_since(0).unwrap();
    assert_eq!(all.len(), 3, "expected 3 samples");
    assert_eq!(all[0].up_bytes, 100);
    assert_eq!(all[0].down_bytes, 200);
    assert_eq!(all[0].period_secs, 20);
    assert_eq!(all[2].up_bytes, 500);

    // Samples are ordered oldest-first.
    assert!(all[0].sampled_at < all[1].sampled_at);
    assert!(all[1].sampled_at < all[2].sampled_at);
}

#[test]
fn latest_bandwidth_sample_returns_newest_row() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();

    assert!(db.latest_bandwidth_sample().unwrap().is_none());

    let t0: i64 = 1_700_000_000;
    db.insert_bandwidth_sample(t0, 100, 200, 20).unwrap();
    db.insert_bandwidth_sample(t0 + 40, 500, 600, 20).unwrap();
    db.insert_bandwidth_sample(t0 + 20, 300, 400, 20).unwrap();

    let latest = db.latest_bandwidth_sample().unwrap().expect("latest sample");
    assert_eq!(latest.sampled_at, t0 + 40);
    assert_eq!(latest.up_bytes, 500);
    assert_eq!(latest.down_bytes, 600);
    assert_eq!(latest.period_secs, 20);
}

#[test]
fn bandwidth_samples_prune() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();

    let t0: i64 = 1_700_000_000;
    db.insert_bandwidth_sample(t0, 1, 2, 20).unwrap();
    db.insert_bandwidth_sample(t0 + 3600, 3, 4, 20).unwrap();
    db.insert_bandwidth_sample(t0 + 7200, 5, 6, 20).unwrap();

    // Prune everything before t0 + 3600: should remove the first sample.
    let removed = db.prune_bandwidth_samples(t0 + 3600).unwrap();
    assert_eq!(removed, 1, "one sample should be pruned");

    let remaining = db.get_bandwidth_history_since(0).unwrap();
    assert_eq!(remaining.len(), 2);
    // The remaining ones start at t0 + 3600.
    assert_eq!(remaining[0].sampled_at, t0 + 3600);
}

#[test]
fn mbps_to_kbps_conversion() {
    // 1 Mbps = 1000 kbps (SI decimal, matching the network convention used
    // by `formatBytes` on the frontend and the `speed_bar` helper in the CLI).
    // NOTE: the conversion is intentionally decimal (1 Mbps = 1000 kbps),
    // not binary (1 Mibps = 1024 Kibps), to match browser and CLI display.
    let mbps_to_kbps = |mbps: f64| -> u64 { (mbps * 1000.0) as u64 };
    let kbps_to_mbps = |kbps: u64| -> f64 { kbps as f64 / 1000.0 };

    assert_eq!(mbps_to_kbps(1.0), 1000, "1 Mbps = 1000 kbps");
    assert_eq!(mbps_to_kbps(10.0), 10_000, "10 Mbps = 10 000 kbps");
    assert_eq!(mbps_to_kbps(100.0), 100_000, "100 Mbps = 100 000 kbps");
    assert_eq!(mbps_to_kbps(0.0), 0, "0 Mbps (unlimited) = 0 kbps");

    assert!((kbps_to_mbps(1000) - 1.0).abs() < 1e-9);
    assert!((kbps_to_mbps(10_000) - 10.0).abs() < 1e-9);
    assert_eq!(kbps_to_mbps(0), 0.0);

    // Round-trip: any value survives mbps → kbps → mbps within ±0.001 Mbps.
    for mbps in [0.5f64, 2.5, 50.0, 200.0] {
        let kbps = mbps_to_kbps(mbps);
        let back = kbps_to_mbps(kbps);
        assert!((back - mbps).abs() < 0.001, "round-trip {mbps} Mbps failed: got {back}");
    }
}

#[test]
fn orphaned_children_of_absent_folder_are_swept() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();

    // Seed two direct children of an absent folder ("folder-gone").
    // `set_file_contract_state` writes `parent_id` into `files`, so we
    // simulate what the snapshot ingest path does: upsert the file row,
    // then call set_file_contract_state to write parent_id.
    let seed = |file_id: &str, path: &str, parent_id: &str, is_folder: bool| {
        let kind = if is_folder { ItemKind::Folder } else { ItemKind::File };
        db.upsert_file(&FileEntry {
            file_id: file_id.to_string(),
            path: path.to_string(),
            status: FileStatus::CloudOnly,
            size_bytes: 0,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None, // upsert_file ignores this field
            item_kind: kind.clone(),
        })
        .unwrap();
        db.set_file_contract_state(&FileContractState {
            file_id: file_id.to_string(),
            namespace: Namespace::MyFiles,
            parent_id: Some(parent_id.to_string()),
            shared_root_id: None,
            share_id: None,
            owner_email: None,
            permission_bits: PERMISSION_READ,
            item_kind: kind,
            content_type: None,
            current_version: 1,
            current_object_version_id: None,
            local_base_version: 0,
            local_hash: None,
            cache_path: None,
            cache_bytes: 0,
            pin_state: PinState::Inherit,
            inherited_pin_state: PinState::Unpinned,
            last_sync_at: 0,
        })
        .unwrap();
    };

    // Two direct children of the absent folder.
    seed("child-file-1", "docs/report.pdf", "folder-gone", false);
    seed("child-file-2", "docs/notes.txt", "folder-gone", false);
    // A sub-folder child (also orphaned) and ITS child, so we test
    // recursive path-prefix sweep for nested orphans.
    seed("child-folder", "docs/sub", "folder-gone", true);
    seed("grandchild", "docs/sub/deep.txt", "child-folder", false);

    // Verify the folder row itself is absent.
    assert!(
        db.get_file("folder-gone").unwrap().is_none(),
        "folder-gone must not exist (mimics the bug condition)"
    );

    // All four rows exist before the sweep.
    assert!(db.get_file("child-file-1").unwrap().is_some());
    assert!(db.get_file("child-file-2").unwrap().is_some());
    assert!(db.get_file("child-folder").unwrap().is_some());
    assert!(db.get_file("grandchild").unwrap().is_some());

    // Execute the fix: sweep orphaned children of the absent folder.
    let removed = db.delete_orphaned_children_of_absent_folder("folder-gone").unwrap();

    // All four rows must be gone.
    assert!(
        db.get_file("child-file-1").unwrap().is_none(),
        "child-file-1 should be removed"
    );
    assert!(
        db.get_file("child-file-2").unwrap().is_none(),
        "child-file-2 should be removed"
    );
    assert!(
        db.get_file("child-folder").unwrap().is_none(),
        "child-folder should be removed"
    );
    assert!(
        db.get_file("grandchild").unwrap().is_none(),
        "grandchild should be removed (nested path-prefix sweep)"
    );

    // The returned PrunedRow vec must contain all four (order may vary, but
    // the grandchild must precede the child-folder it lives under — that is
    // the children-before-parent contract the Windows placeholder remover
    // depends on).
    assert_eq!(removed.len(), 4, "should have removed exactly 4 rows");
    let ids: Vec<&str> = removed.iter().map(|r| r.file_id.as_str()).collect();
    assert!(ids.contains(&"child-file-1"));
    assert!(ids.contains(&"child-file-2"));
    assert!(ids.contains(&"child-folder"));
    assert!(ids.contains(&"grandchild"));

    let grandchild_pos = ids.iter().position(|&id| id == "grandchild").unwrap();
    let child_folder_pos = ids.iter().position(|&id| id == "child-folder").unwrap();
    assert!(
        grandchild_pos < child_folder_pos,
        "grandchild ({grandchild_pos}) must appear before child-folder ({child_folder_pos}) \
             so the Windows placeholder remover can remove leaf-before-dir"
    );
}

#[test]
fn orphaned_children_sweep_is_noop_when_no_children() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();

    // No rows in the DB at all — the sweep must be a safe no-op.
    let removed = db
        .delete_orphaned_children_of_absent_folder("nonexistent-folder")
        .unwrap();
    assert!(removed.is_empty(), "no children → should return empty vec");
}

/// Flow 7 / PR #58 merge with task 1538: every purge that drops queued
/// upload ops must drop their persisted upload sessions too. A resume row
/// outliving its op is dead state; across a sign-out it would carry the
/// previous account's upload session id, server file id and staged path
/// into the next account's database.
#[test]
fn test_every_operation_purge_drops_the_upload_resume_rows() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();

    let mk_op = |op_id: &str, file_id: &str, key: Option<&str>| PendingOperation {
        op_id: op_id.into(),
        kind: OperationKind::UploadFile,
        file_id: Some(file_id.into()),
        parent_id: None,
        target_path: Some(format!("/{op_id}.bin")),
        metadata_json: Some(r#"{"operation":"create_file"}"#.into()),
        payload_path: Some(format!("/staging/{op_id}")),
        base_version: None,
        base_object_version_id: None,
        attempts: 1,
        max_attempts: 5,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: key.map(str::to_string),
        created_at: 100,
        updated_at: 100,
    };
    let mk_resume = |op_id: &str| UploadResume {
        op_id: op_id.into(),
        payload_path: format!("/staging/{op_id}"),
        payload_size: 3 * 1024,
        payload_mtime_ns: 42,
        upload_session_id: format!("session-{op_id}"),
        server_file_id: format!("server-{op_id}"),
        object_version_id: format!("ov-{op_id}"),
        chunk_size_bytes: 1024,
        chunk_count: 3,
        acked_chunks: 1,
        metadata_applied: true,
        is_create: true,
        self_minted: false,
        completed_version: None,
        completed_object_version_id: None,
        completed_mime_type: None,
    };
    let seed = |op_id: &str, file_id: &str, key: Option<&str>| {
        db.enqueue_operation(&mk_op(op_id, file_id, key)).unwrap();
        db.put_upload_resume(&mk_resume(op_id)).unwrap();
    };

    // (1) Disabling a known-folder backup: only that folder's session goes.
    seed("music-1", "file-music-1", Some("music"));
    seed("normal-1", "file-normal-1", None);
    assert_eq!(db.purge_backup_source_ops("music").unwrap(), 1);
    assert!(
        db.get_upload_resume("music-1").unwrap().is_none(),
        "a purged backup op's upload session must go with it"
    );
    assert!(
        db.get_upload_resume("normal-1").unwrap().is_some(),
        "a surviving op keeps its upload session"
    );

    // (2) Revoked shared content: the revoked file's queued upload goes, and
    //     so does its session.
    seed_contract_row(&db, "revoked", "/Shared with me/Revoked", None, FileStatus::Local, 20);
    let mut revoked = db.get_file_contract_state("revoked").unwrap().unwrap();
    revoked.namespace = Namespace::SharedWithMe;
    revoked.shared_root_id = Some("revoked".into());
    revoked.share_id = Some("invite-revoked".into());
    revoked.permission_bits = PERMISSION_READ | PERMISSION_WRITE;
    db.set_file_contract_state(&revoked).unwrap();
    seed("shared-1", "revoked", None);
    db.purge_revoked_shared_content(&[]).unwrap();
    assert!(
        db.get_upload_resume("shared-1").unwrap().is_none(),
        "a revoked share's queued upload must not keep its upload session"
    );
    assert!(db.get_upload_resume("normal-1").unwrap().is_some());

    // (3) Sign-out: nothing of the leaving account's uploads survives.
    let purge = db.purge_all_local_state().unwrap();
    assert!(purge.queued_ops_purged >= 1);
    assert!(
        db.get_upload_resume("normal-1").unwrap().is_none(),
        "sign-out must purge every persisted upload session"
    );
}

#[test]
fn has_pending_trash_sees_only_a_queued_trash_for_that_file() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    let op = |op_id: &str, kind: OperationKind, file_id: &str| PendingOperation {
        op_id: op_id.into(),
        kind,
        file_id: Some(file_id.into()),
        parent_id: None,
        target_path: None,
        metadata_json: None,
        payload_path: None,
        base_version: None,
        base_object_version_id: None,
        attempts: 25,
        max_attempts: 25, // out of attempts: still a queued delete
        next_retry_at: i64::MAX,
        last_error: None,
        backup_source_key: None,
        created_at: 0,
        updated_at: 0,
    };
    db.enqueue_operation(&op("t1", OperationKind::TrashFile, "trashed"))
        .unwrap();
    db.enqueue_operation(&op("u1", OperationKind::UploadVersion, "uploaded"))
        .unwrap();
    assert!(
        db.has_pending_trash("trashed").unwrap(),
        "a queued trash, even out of retries"
    );
    assert!(
        !db.has_pending_trash("uploaded").unwrap(),
        "another kind of op is not a delete"
    );
    assert!(!db.has_pending_trash("never-seen").unwrap());
    db.remove_operation("t1").unwrap();
    assert!(!db.has_pending_trash("trashed").unwrap(), "gone once the op is removed");
}

#[test]
fn change_log_anchors_are_monotonic_across_batches() {
    let db = StateDb::open(":memory:").unwrap();
    seed_child(&db, "f-1", "/a.txt", ItemKind::File);
    db.record_file_change("f-1", FpChangeKind::Created, None).unwrap();
    let first = db.list_file_changes(None).unwrap();
    let (_, anchor_1) = first.expect("changes since nil");
    let anchor_1 = anchor_1.expect("an anchor after real changes");
    assert!(
        anchor_1.len() <= 500,
        "anchor must stay within Apple's 500-byte budget, got {} bytes",
        anchor_1.len()
    );
    assert!(!anchor_1.is_empty(), "the anchor after changes must not be empty");

    db.record_file_change("f-1", FpChangeKind::Modified, None).unwrap();
    let (_, anchor_2) = db
        .list_file_changes(Some(&anchor_1))
        .unwrap()
        .expect("changes since anchor 1");
    let anchor_2 = anchor_2.expect("an anchor after further changes");
    assert!(
        anchor_2 > anchor_1,
        "anchors must sort strictly ascending so 'anchor_1 > anchor_2' can never be true (got {:?} then {:?})",
        anchor_1,
        anchor_2
    );
}

#[test]
fn change_log_anchor_is_stable_when_nothing_changed() {
    let db = StateDb::open(":memory:").unwrap();
    seed_child(&db, "f-1", "/a.txt", ItemKind::File);
    db.record_file_change("f-1", FpChangeKind::Created, None).unwrap();
    let (_, anchor_1) = db.list_file_changes(None).unwrap().unwrap();
    let anchor_1 = anchor_1.unwrap();
    // Nothing happened since: the anchor must be stable and the batch empty.
    let (changes, anchor_2) = db.list_file_changes(Some(&anchor_1)).unwrap().unwrap();
    assert!(changes.is_empty(), "no changes since the fresh anchor: {changes:?}");
    assert_eq!(
        anchor_2.as_deref(),
        Some(anchor_1.as_ref()),
        "a no-op poll must not move the anchor"
    );
}

#[test]
fn change_log_reports_old_and_new_parent_for_reparents() {
    let db = StateDb::open(":memory:").unwrap();
    seed_child_under(&db, "child-1", "/docs/child-1", ItemKind::File, Some("old-parent"));
    seed_child(&db, "old-parent", "/docs", ItemKind::Folder);
    seed_child(&db, "new-parent", "/archive", ItemKind::Folder);
    db.record_file_change("child-1", FpChangeKind::Created, None).unwrap();
    // The move itself re-parents the row (what a real reparent does) —
    // `set_file_contract_state` records the Reparented change itself now.
    db.set_file_contract_state(&FileContractState {
        parent_id: Some("new-parent".into()),
        ..db.get_file_contract_state("child-1").unwrap().unwrap()
    })
    .unwrap();
    let (changes, _) = db.list_file_changes(None).unwrap().unwrap();
    let reparent = changes
        .iter()
        .find(|change| {
            change.file_id == "child-1"
                && change.kind == FpChangeKind::Reparented
                && change.old_parent_id.as_deref() == Some("old-parent")
        })
        .expect("the real reparent (old parent = old-parent) must come back");
    assert_eq!(
        reparent.old_parent_id.as_deref(),
        Some("old-parent"),
        "old parent for materialized-set filtering"
    );
    assert_eq!(
        reparent.new_parent_id.as_deref(),
        Some("new-parent"),
        "new parent read from the files row at record time"
    );
}

#[test]
fn change_log_records_deletes_with_the_old_parent() {
    let db = StateDb::open(":memory:").unwrap();
    seed_child_under(&db, "child-1", "/docs/child-1", ItemKind::File, Some("parent-1"));
    seed_child(&db, "parent-1", "/docs", ItemKind::Folder);
    db.record_file_change("child-1", FpChangeKind::Created, None).unwrap();
    db.delete_file("child-1").unwrap();
    // The row is gone; the recorder must have captured the parent
    // (from the files row) BEFORE the delete — the caller passes it as
    // old_parent_id and record_file_change pins it as the change's parent
    // when the row no longer exists.
    db.record_file_change("child-1", FpChangeKind::Deleted, Some("parent-1".into()))
        .unwrap();
    let (changes, _) = db.list_file_changes(None).unwrap().unwrap();
    let deletion = changes
        .iter()
        .find(|change| change.file_id == "child-1" && change.kind == FpChangeKind::Deleted)
        .expect("the deleted change must come back");
    assert_eq!(
        deletion.new_parent_id.as_deref(),
        Some("parent-1"),
        "deleted items report their old parent as new_parent_id"
    );
}

#[test]
fn change_log_paging_returns_every_change_exactly_once() {
    let db = StateDb::open(":memory:").unwrap();
    // Raw upserts only: each one records its own Created change (the
    // wired behavior). No contracts — parent/contract writes would add
    // their own changes and shift the page math.
    for i in 0..250 {
        db.upsert_file(&FileEntry {
            file_id: format!("f-{i}"),
            path: format!("/f-{i}"),
            status: FileStatus::Local,
            size_bytes: 5,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
    }
    let mut seen = std::collections::HashSet::new();
    let mut pages = 0;
    let mut cursor: Option<Vec<u8>> = None;
    loop {
        let (changes, anchor) = db.list_file_changes_paged(cursor.as_deref(), 100).unwrap().unwrap();
        assert!(
            changes.len() <= 100,
            "a page must honor the limit, got {}",
            changes.len()
        );
        for change in &changes {
            assert!(
                seen.insert((change.file_id.clone(), change.kind)),
                "a change came back twice: {:?}",
                change
            );
        }
        pages += 1;
        match anchor {
            Some(a) if a == cursor.clone().unwrap_or_default() => {
                // The up-to-date marker echoes the caller's own cursor —
                // the window is complete.
                break;
            }
            Some(a) => {
                assert!(
                    a > cursor.clone().unwrap_or_default(),
                    "paging anchors must strictly increase"
                );
                cursor = Some(a);
            }
            None => break,
        }
        assert!(
            pages < 10,
            "250 changes at 100/page must take 3 pages, not spin forever"
        );
    }
    assert_eq!(seen.len(), 250, "every change must be delivered exactly once");
    // 3 delivery pages + 1 final round trip whose echoed anchor breaks
    // the loop.
    assert_eq!(pages, 4, "3 delivery pages + 1 up-to-date round trip, got {pages}");
}

#[test]
fn change_log_is_per_domain_and_sweeps_only_old_rows() {
    let db = StateDb::open(":memory:").unwrap();
    seed_child(&db, "f-1", "/a.txt", ItemKind::File);
    db.record_file_change("f-1", FpChangeKind::Created, None).unwrap();
    let (_, anchor) = db.list_file_changes(None).unwrap().unwrap();
    let anchor = anchor.unwrap();
    // Sweeping everything older than now prunes the consumed change but
    // must never touch the anchor row (the replica's cursor survives).
    db.sweep_file_changes(2_000_000_000).unwrap();
    let (changes, anchor_after) = db.list_file_changes(Some(&anchor)).unwrap().unwrap();
    assert!(
        changes.is_empty(),
        "swept changes are no longer deliverable: {changes:?}"
    );
    assert_eq!(
        anchor_after.as_deref(),
        Some(anchor.as_ref()),
        "the anchor survives the sweep"
    );
    // After the sweep the log is empty (tip = 0), so a from-nil listing
    // starts a FRESH enumeration — the sweep means "history older than the
    // cutoff is gone", not "the anchor resets". The persistent anchor ROW
    // (fp_last_anchor) is what must survive for currentSyncAnchor:
    assert_eq!(
        db.fp_last_anchor().unwrap().as_deref(),
        Some(anchor.as_ref()),
        "the persistent anchor survives the sweep (crash recovery reads it)"
    );
}

#[test]
fn change_log_anchor_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = StateDb::open(&path).unwrap();
    seed_child(&db, "f-1", "/a.txt", ItemKind::File);
    db.record_file_change("f-1", FpChangeKind::Created, None).unwrap();
    let (_, anchor) = db.list_file_changes(None).unwrap().unwrap();
    let anchor = anchor.unwrap();
    drop(db);
    let db = StateDb::open(&path).unwrap();
    let (_, reopened) = db.list_file_changes(Some(&anchor)).unwrap().unwrap();
    assert_eq!(
        reopened.as_deref(),
        Some(anchor.as_ref()),
        "the anchor and consumed state must survive process death"
    );
}

#[test]
fn change_log_rows_record_the_real_insertion_time() {
    let db = StateDb::open(":memory:").unwrap();
    seed_child(&db, "f-1", "/a.txt", ItemKind::File);
    db.record_file_change("f-1", FpChangeKind::Created, None).unwrap();
    let recorded_at = recorded_at_of(&db, "f-1");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    assert!(
        recorded_at > 0,
        "a recorded change must carry its real insertion time, got recorded_at={recorded_at}"
    );
    assert!(
        recorded_at >= now - 60 && recorded_at <= now + 60,
        "recorded_at={recorded_at} must be wall-clock now (~{now}), not the 0 sentinel"
    );
}

#[test]
fn change_log_sweep_keeps_fresh_rows() {
    // The signal path sweeps with cutoff = now - 7 days. A change
    // recorded NOW must survive it — the sweep exists to age out history,
    // not to wipe the feed File Provider was just asked to enumerate.
    let db = StateDb::open(":memory:").unwrap();
    seed_child(&db, "f-1", "/a.txt", ItemKind::File);
    db.record_file_change("f-1", FpChangeKind::Created, None).unwrap();
    let fresh_rows: i64 =
        db.0.lock()
            .expect("state_db mutex poisoned")
            .query_row("SELECT COUNT(*) FROM fp_changes WHERE file_id = 'f-1'", [], |row| {
                row.get(0)
            })
            .unwrap();
    assert!(fresh_rows > 0, "the seed must have written change rows");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let deleted = db.sweep_file_changes(now - 7 * 24 * 3600).unwrap();
    assert_eq!(
        deleted, 0,
        "fresh rows must NEVER be swept (was: every row, recorded_at=0 < cutoff)"
    );
    // The change is still deliverable AFTER the sweep (no pre-consume:
    // a from-nil listing must still see it).
    let (changes, _) = db.list_file_changes(None).unwrap().unwrap();
    assert!(
        !changes.is_empty() && changes.iter().all(|change| change.file_id == "f-1"),
        "the fresh changes are still deliverable after the sweep: {changes:?}"
    );
    // Invariant: the sweep never touches the anchor row.
    assert!(
        db.fp_last_anchor().unwrap().is_some(),
        "the persistent anchor survives the sweep (crash recovery reads it)"
    );
}

#[test]
fn change_log_sweep_deletes_rows_older_than_cutoff_and_keeps_the_boundary() {
    // Backdated rows (direct SQL in test setup — production inserts are
    // always fresh) must age out; the comparison is STRICT `<`, so a row
    // recorded exactly AT the cutoff survives.
    let db = StateDb::open(":memory:").unwrap();
    seed_child(&db, "old-1", "/old.txt", ItemKind::File);
    seed_child(&db, "edge-1", "/edge.txt", ItemKind::File);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let cutoff = now - 7 * 24 * 3600;
    {
        let conn = db.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "UPDATE fp_changes SET recorded_at = ?1 WHERE file_id = 'old-1'",
            params![cutoff - 1],
        )
        .unwrap();
        conn.execute(
            "UPDATE fp_changes SET recorded_at = ?1 WHERE file_id = 'edge-1'",
            params![cutoff],
        )
        .unwrap();
    }
    let count = |file_id: &str| -> i64 {
        db.0.lock()
            .expect("state_db mutex poisoned")
            .query_row(
                "SELECT COUNT(*) FROM fp_changes WHERE file_id = ?1",
                params![file_id],
                |row| row.get(0),
            )
            .unwrap()
    };
    let old_rows = count("old-1");
    let edge_rows = count("edge-1");
    assert!(
        old_rows > 0 && edge_rows > 0,
        "both files must have backdated change rows"
    );
    let deleted = db.sweep_file_changes(cutoff).unwrap();
    assert_eq!(
        deleted as i64, old_rows,
        "strict <: only the rows strictly OLDER than the cutoff are swept"
    );
    assert_eq!(count("old-1"), 0, "aged-out rows are gone");
    assert_eq!(
        count("edge-1"),
        edge_rows,
        "a row recorded exactly AT the cutoff survives (strict <)"
    );
    // Invariant: the sweep never touches the anchor row.
    assert!(
        db.fp_last_anchor().unwrap().is_some(),
        "the anchor cursor outlives swept history"
    );
}

#[test]
fn the_owner_round_trips_and_a_sign_out_purge_forgets_it_with_the_staged_payloads() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    assert_eq!(db.owner().unwrap(), None);
    assert!(!db.has_account_data().unwrap(), "a new database holds no account data");
    let owner = crate::account_binding::Identity::new(Some("u-a"), Some("a@beebeeb.io"));
    db.set_owner(&owner).unwrap();
    assert_eq!(db.owner().unwrap(), Some(owner));
    assert!(
        !db.has_account_data().unwrap(),
        "the owner record alone is not account data"
    );
    db.track_staged_payload("/tmp/bb-r10-staged-1.bin", None, false)
        .unwrap();
    assert!(db.has_account_data().unwrap());
    let purge = db.purge_all_local_state().unwrap();
    assert!(
        purge.payload_paths.contains(&"/tmp/bb-r10-staged-1.bin".to_string()),
        "staged files are returned for deletion"
    );
    let staged: i64 =
        db.0.lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM staged_payloads", [], |r| r.get(0))
            .unwrap();
    assert_eq!(staged, 0, "staged payload rows are gone");
    assert_eq!(db.owner().unwrap(), None, "a sign-out forgets the owner");
}

#[test]
fn clearing_account_data_leaves_no_row_and_no_owner() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    db.0.lock()
        .unwrap()
        .execute("INSERT INTO sync_state(key, value) VALUES ('cursor', '12')", [])
        .unwrap();
    db.track_staged_payload("/tmp/bb-r10-staged-2.bin", None, true).unwrap();
    db.set_owner(&crate::account_binding::Identity::new(Some("u-a"), None))
        .unwrap();
    assert!(db.has_account_data().unwrap(), "a sync cursor is account data");
    db.clear_account_data(false).unwrap();
    assert!(!db.has_account_data().unwrap());
    assert_eq!(db.owner().unwrap(), None);
}

/// Task 2279 item 1: index work waiting for custody names a folder of one
/// account. A full local purge forgets it with the rest of that account's state.
#[test]
fn sf2279_purge_all_local_state_drops_deferred_index_work() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    db.defer_shared_index_until_custody("f-1").unwrap();
    let deferred = |db: &StateDb| -> i64 {
        db.0.lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM sync_state WHERE key LIKE 'shared_folder_index_deferred:%'",
                [],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert_eq!(deferred(&db), 1, "the deferral is durable before the purge");
    db.purge_all_local_state().unwrap();
    assert_eq!(deferred(&db), 0, "a purge forgets index work waiting for custody");
}

/// Round 4 of 2279: reviving a deferral is one atomic step. The dirty marker is what schedules the repair, so if
/// it cannot be written the deferral must still be there. With two autocommit statements the delete committed on
/// its own and a failure or crash in between lost the repair for good.
#[test]
fn sf2279_revive_keeps_the_deferral_when_the_dirty_marker_fails() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    let root = "10000000-0000-4000-8000-000000000061";
    db.defer_shared_index_until_custody(root).unwrap();
    // The upsert either inserts the dirty row or updates it; refuse both events so the second statement always
    // fails, whichever way SQLite resolves the conflict.
    for event in ["INSERT", "UPDATE"] {
        db.0.lock()
            .unwrap()
            .execute_batch(&format!(
                "CREATE TRIGGER refuse_dirty_{event} BEFORE {event} ON sync_state
                     WHEN NEW.key = 'shared_folder_index_dirty:{root}'
                     BEGIN SELECT RAISE(ABORT, 'dirty marker refused'); END;"
            ))
            .unwrap();
    }
    let deferred = |db: &StateDb| -> i64 {
        db.0.lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM sync_state WHERE key = ?1",
                [format!("shared_folder_index_deferred:{root}")],
                |r| r.get(0),
            )
            .unwrap()
    };
    let revived = db.revive_deferred_shared_index(root);
    let err = revived.expect_err("the refused dirty marker fails the revive");
    assert!(
        err.to_string().contains("dirty marker refused"),
        "the trigger, not something else, refused it: {err}"
    );
    assert_eq!(deferred(&db), 1, "the deferral survives so the repair is not lost");

    // With the trigger gone the same call revives it exactly once.
    for event in ["INSERT", "UPDATE"] {
        db.0.lock()
            .unwrap()
            .execute_batch(&format!("DROP TRIGGER refuse_dirty_{event}"))
            .unwrap();
    }
    assert!(
        db.revive_deferred_shared_index(root).unwrap(),
        "the retried revive finds the deferral"
    );
    assert_eq!(deferred(&db), 0, "the revival consumed the deferral");
    assert!(
        db.dirty_shared_index_roots().unwrap().contains(&root.to_string()),
        "the revival scheduled the repair"
    );
}

/// L7d (review 2279 F2): the custody memory is device-wide and monotonic.
/// Every local-state wipe keeps it; only the user removing the app's data
/// can drop it. The dirty marker, the deferral and the maintenance clock
/// are account work and still go.
#[test]
fn sf2279_custody_memory_survives_every_local_state_wipe() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    let root = "10000000-0000-4000-8000-000000000041";
    let seen = |db: &StateDb| db.has_seen_folder_custody(root).unwrap();
    let wipe = |db: &StateDb| {
        db.mark_shared_index_dirty(root).unwrap();
        db.defer_shared_index_until_custody(root).unwrap();
        db.remember_folder_custody_at_revision(root, db.account_revision())
            .unwrap();
        assert!(seen(db), "the fixture starts with the custody memory");
    };

    wipe(&db);
    db.purge_all_local_state().unwrap();
    assert!(seen(&db), "the sign-out row purge keeps the custody memory");
    assert!(
        db.dirty_shared_index_roots().unwrap().is_empty(),
        "account index work still goes"
    );
    assert!(
        !db.revive_deferred_shared_index(root).unwrap(),
        "the deferral still goes"
    );

    wipe(&db);
    db.clear_account_data(false).unwrap();
    assert!(seen(&db), "a reset or account switch keeps the custody memory");
    assert!(db.dirty_shared_index_roots().unwrap().is_empty());

    wipe(&db);
    db.finish_windows_signout().unwrap();
    assert!(seen(&db), "the Windows sign-out keeps the custody memory");
}

/// N1 (round 3 of 2279): the kept custody row must not read as account data. F2 keeps
/// `shared_folder_custody_seen:%` through every wipe, and `account_rows` counted every `sync_state` key it did
/// not name, so `has_account_data` stayed true after a completed sign-out — forever, the set only grows. The next
/// engine start then wanted a reset on macOS/Linux and was refused as another account's data on Windows.
#[test]
fn sf2279_the_kept_custody_row_is_not_account_data_after_every_wipe() {
    let root = "10000000-0000-4000-8000-000000000053";
    let after_wipe = |name: &str, wipe: &dyn Fn(&StateDb)| {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.remember_folder_custody_at_revision(root, db.account_revision())
            .unwrap();
        assert!(db.has_seen_folder_custody(root).unwrap(), "{name}: fixture");
        wipe(&db);
        assert!(
            db.has_seen_folder_custody(root).unwrap(),
            "{name}: the custody row survives the wipe (F2)"
        );
        assert!(
            !db.has_account_data().unwrap(),
            "{name}: the kept custody row is not account data"
        );
    };
    after_wipe("purge_all_local_state", &|db| {
        db.purge_all_local_state().unwrap();
    });
    after_wipe("clear_account_data", &|db| {
        db.clear_account_data(false).unwrap();
    });
    after_wipe("finish_windows_signout", &|db| {
        db.finish_windows_signout().unwrap();
    });
}

/// B05e (lead ruling 2026-11-09, desktop #265 delta 1): the share marks, the sealed and offered keys
/// and the accepted fingerprints survive a sign-out (macOS/Linux and Windows), like the custody
/// memory; a reset of local data (R10's `reset`, the real one) removes them. They are not account
/// data, so a completed sign-out still leaves no account data and no owner (the R10 binding's
/// rule), and the next start keeps.
#[test]
fn b05e_share_rows_survive_a_sign_out_and_a_reset_of_local_data_removes_them() {
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
        db.0.lock()
            .unwrap()
            .execute("INSERT INTO sync_state(key, value) VALUES ('cursor', '12')", [])
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
    // macOS/Linux: `purge_local_data_for_sign_out` purges, then `clear_account_data`; Windows: one call.
    type Wipe<'a> = &'a dyn Fn(&StateDb);
    let sign_outs: [(&str, Wipe); 2] = [
        ("purge_all_local_state + clear_account_data", &|db| {
            db.purge_all_local_state().unwrap();
            db.clear_account_data(false).unwrap();
        }),
        ("finish_windows_signout", &|db| db.finish_windows_signout().unwrap()),
    ];
    for (name, sign_out) in sign_outs {
        let dir = tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        fill(&db);
        sign_out(&db);
        assert_eq!(kept(&db), 5, "{name}: a sign-out keeps every share row");
        let rows: Vec<String> =
            db.0.lock()
                .unwrap()
                .prepare("SELECT key || ' ' || value FROM sync_state")
                .unwrap()
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
        assert!(
            !rows.iter().any(|r| r.contains("northwind")),
            "{name}: no kept row holds the address: {rows:?}"
        );
        assert_eq!(db.owner().unwrap(), None, "{name}: and still forgets the owner");
        assert!(
            !db.has_account_data().unwrap(),
            "{name}: the kept share rows are not account data"
        );
    }

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

/// The pin under the ruling above: share rows on their own are not account data (as the custody row, task
/// 2279 N1). Counted, they would keep `has_account_data` true after every completed sign-out: the next engine
/// start would want a reset on macOS/Linux and be refused as another account's data on Windows.
#[test]
fn b05e_share_rows_alone_are_not_account_data() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    let id = "7f9e2d14-3a6b-4c8d-8e1f-2b4a6c8d0e12";
    db.set_share_mark(id, id, "ab", MarkKind::Accepted).unwrap();
    db.set_share_email_mark(id, "jonas@northwind.studio", "ab", MarkKind::NotMatched)
        .unwrap();
    db.set_share_sealed_key(id, id, "cd").unwrap();
    db.set_share_offered_key(id, "other", "cd").unwrap();
    db.set_share_accepted_fingerprint(id, id, "ab").unwrap();
    assert!(!db.has_account_data().unwrap());
}

/// L7d: the set stores canonical lowercase hyphenated UUIDs only. A write
/// with any other form is refused, so an id shape the server reports can
/// never create a second entry for a folder this device has seen. A read
/// canonicalizes, so a re-spelled id finds the entry instead of missing it.
#[test]
fn sf2279_custody_memory_refuses_a_non_canonical_id() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    let revision = db.account_revision();
    for bad in [
        "",
        "f-1",
        "10000000-0000-4000-8000-00000000004A",
        "10000000000040008000000000000042",
        "{10000000-0000-4000-8000-000000000042}",
    ] {
        assert!(
            db.remember_folder_custody_at_revision(bad, revision).is_err(),
            "{bad:?} must be refused on write"
        );
    }
    let stored: i64 =
        db.0.lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM sync_state WHERE key LIKE 'shared_folder_custody_seen:%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
    assert_eq!(stored, 0, "no non-canonical id was stored");
    let canonical = "10000000-0000-4000-8000-00000000004a";
    db.remember_folder_custody_at_revision(canonical, revision).unwrap();
    for respelled in [
        canonical,
        "10000000-0000-4000-8000-00000000004A",
        "1000000000004000800000000000004a",
        "{10000000-0000-4000-8000-00000000004a}",
    ] {
        assert!(
            db.has_seen_folder_custody(respelled).unwrap(),
            "{respelled:?} finds the canonical entry"
        );
    }
    assert!(
        !db.has_seen_folder_custody("not-a-uuid").unwrap(),
        "an id that is not a UUID cannot be in the set"
    );
}

/// Added after a mutation survived (task 1834, M20): the sync cursor is account data on its own, and the
/// owner record, which lives in the same table, is not. Nothing else is in the database here, so this
/// is the only row `has_account_data` can be counting.
#[test]
fn a_sync_cursor_alone_is_account_data_and_the_owner_record_alone_is_not() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    db.set_owner(&crate::account_binding::Identity::new(
        Some("u-a"),
        Some("a@beebeeb.io"),
    ))
    .unwrap();
    assert!(!db.has_account_data().unwrap(), "the owner record is not account data");
    db.0.lock()
        .unwrap()
        .execute("INSERT INTO sync_state(key, value) VALUES ('cursor', '12')", [])
        .unwrap();
    assert!(db.has_account_data().unwrap(), "a sync cursor is");
}

/// F2 (fix round 1 of Task 10): adopting unowned local data is ONE conditional step, once per database, and
/// the "already done" mark is not a row, so no purge or reset can clear it and re-open the window.
#[test]
fn adoption_is_one_conditional_step_that_happens_at_most_once() {
    use crate::account_binding::Identity;
    let dir = tempdir().unwrap();
    let a = Identity::new(None, Some("a@beebeeb.io"));
    let b = Identity::new(None, Some("b@beebeeb.io"));
    let seed = |db: &StateDb| {
        db.0.lock()
            .unwrap()
            .execute("INSERT INTO sync_state(key, value) VALUES ('cursor', '1')", [])
            .unwrap();
    };

    // Unowned data and a candidate: adopted, and the window is shut behind it.
    let db = StateDb::open(dir.path().join("one.db")).unwrap();
    seed(&db);
    assert!(db.adopt_owner_once(Some(&a)).unwrap());
    assert_eq!(db.owner().unwrap(), Some(a.clone()));
    assert!(
        !db.adopt_owner_once(Some(&b)).unwrap(),
        "the window is shut: a second candidate is refused"
    );
    assert_eq!(db.owner().unwrap(), Some(a.clone()), "and the owner is never replaced");

    // A first look with no candidate (no vault key) also shuts the window: later candidates are too late.
    let db = StateDb::open(dir.path().join("two.db")).unwrap();
    seed(&db);
    assert!(!db.adopt_owner_once(None).unwrap());
    assert!(!db.adopt_owner_once(Some(&a)).unwrap(), "the first startup has passed");
    assert_eq!(db.owner().unwrap(), None);

    // Nothing to adopt (no account data) shuts the window without recording anybody.
    let db = StateDb::open(dir.path().join("three.db")).unwrap();
    assert!(!db.adopt_owner_once(Some(&a)).unwrap());
    seed(&db);
    assert!(
        !db.adopt_owner_once(Some(&a)).unwrap(),
        "data that appears later is not the upgrade's"
    );
    assert_eq!(db.owner().unwrap(), None);

    // The mark survives a purge and a reset: it is a header value, not a row.
    let db = StateDb::open(dir.path().join("four.db")).unwrap();
    seed(&db);
    db.close_adoption_window().unwrap();
    db.purge_all_local_state().unwrap();
    db.clear_account_data(false).unwrap();
    seed(&db);
    assert!(
        !db.adopt_owner_once(Some(&a)).unwrap(),
        "a reset does not re-open the window"
    );
    assert!(!db.has_account_data().unwrap_or(false) || db.owner().unwrap().is_none());
}

/// An owner that is already recorded is never replaced by an adoption, and shuts the window too.
#[test]
fn adoption_never_replaces_a_recorded_owner() {
    use crate::account_binding::Identity;
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    db.0.lock()
        .unwrap()
        .execute("INSERT INTO sync_state(key, value) VALUES ('cursor', '1')", [])
        .unwrap();
    db.set_owner(&Identity::new(Some("u-a"), None)).unwrap();
    assert!(
        !db.adopt_owner_once(Some(&Identity::new(None, Some("b@beebeeb.io"))))
            .unwrap()
    );
    assert_eq!(db.owner().unwrap(), Some(Identity::new(Some("u-a"), None)));
}

/// R3 (fix round 2): a clear that owes no removal leaves the "domain gone" mark; one that owes it leaves the debt
/// and no mark; a debt recorded later wipes the mark (a domain may be registered again). Neither is account data.
#[test]
fn the_domain_gone_mark_and_the_removal_debt_exclude_each_other_and_are_not_account_data() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    db.clear_account_data(false).unwrap();
    assert!(db.finder_domain_gone().unwrap() && !db.finder_removal_owed().unwrap());
    assert!(!db.has_account_data().unwrap(), "the mark is not account data");
    db.clear_account_data(true).unwrap();
    assert!(
        !db.finder_domain_gone().unwrap() && db.finder_removal_owed().unwrap(),
        "owing a removal leaves no mark"
    );
    assert!(!db.has_account_data().unwrap(), "the debt is not account data");
    db.clear_account_data(false).unwrap();
    db.set_finder_removal_owed(true).unwrap();
    assert!(
        !db.finder_domain_gone().unwrap(),
        "a debt recorded later wipes the mark"
    );
    db.set_finder_removal_owed(false).unwrap();
    assert!(!db.finder_removal_owed().unwrap());
}

/// Fix round 3: a domain registered again is no longer known gone. Clearing the mark touches nothing else: the
/// owner, a removal debt and the account rows stay.
#[test]
fn a_domain_registered_again_clears_the_gone_mark_and_nothing_else() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    db.clear_account_data(false).unwrap();
    assert!(db.finder_domain_gone().unwrap());
    db.clear_finder_domain_gone().unwrap();
    assert!(!db.finder_domain_gone().unwrap(), "the mark is gone");
    db.clear_finder_domain_gone().unwrap(); // nothing to clear is not an error
    let owner = crate::account_binding::Identity::new(Some("u-a"), Some("a@beebeeb.io"));
    db.set_owner(&owner).unwrap();
    db.set_finder_removal_owed(true).unwrap();
    db.clear_finder_domain_gone().unwrap();
    assert_eq!(db.owner().unwrap(), Some(owner), "the owner stays");
    assert!(db.finder_removal_owed().unwrap(), "a removal debt stays");
}

#[test]
fn every_table_is_classified_for_the_account_binding() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    let conn = db.0.lock().unwrap();
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .unwrap();
    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_>>()
        .unwrap();
    assert!(tables.len() >= 12, "{tables:?}");
    for table in &tables {
        assert!(
            ACCOUNT_TABLES.contains(&table.as_str())
                || DEVICE_TABLES.contains(&table.as_str())
                || table == "sync_state",
            "{table}: list it in ACCOUNT_TABLES (R10), or in DEVICE_TABLES if it holds no account data"
        );
    }
}

#[test]
fn queued_or_staged_counts_the_queue_and_the_staged_payloads() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    assert_eq!(db.queued_or_staged_count().unwrap(), 0);
    db.track_staged_payload("/tmp/bb-p1-staged.bin", None, false).unwrap();
    assert_eq!(db.queued_or_staged_count().unwrap(), 1);
    db.enqueue_operation(&PendingOperation {
        op_id: "op-p1".into(),
        kind: OperationKind::UploadVersion,
        file_id: Some("file-p1".into()),
        parent_id: None,
        target_path: Some("/P1.txt".into()),
        metadata_json: None,
        payload_path: None,
        base_version: None,
        base_object_version_id: None,
        attempts: 0,
        max_attempts: 5,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: 0,
        updated_at: 0,
    })
    .unwrap();
    assert_eq!(
        db.queued_or_staged_count().unwrap(),
        2,
        "a queued change and a staged payload both count"
    );
}

#[test]
fn custody_seen_ids_lists_every_remembered_folder_and_nothing_else() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    assert!(db.custody_seen_ids().unwrap().is_empty());
    let a = "2237c000-0000-4000-8000-000000000001";
    let b = "2237c000-0000-4000-8000-000000000002";
    db.remember_folder_custody(a).unwrap();
    db.remember_folder_custody(b).unwrap();
    db.remember_folder_custody(a).unwrap();
    assert!(
        db.remember_folder_custody("2237C000-0000-4000-8000-000000000003")
            .is_err(),
        "canonical ids only"
    );
    let mut ids = db.custody_seen_ids().unwrap();
    ids.sort();
    assert_eq!(ids, vec![a.to_string(), b.to_string()]);
    assert!(db.has_seen_folder_custody(&a.to_uppercase()).unwrap());
}
