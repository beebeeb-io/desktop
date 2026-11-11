//! Tests for file rows, contracts, operations and local-state purges, split out of `state_db.rs`.

use super::*;

#[test]
fn get_file_by_path_matches_across_leading_slash() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();

    // Row stored WITH a leading slash (the `/leaf` shape resolve_relative_path
    // can produce). The watcher queries the bare form — must still hit.
    db.upsert_file(&FileEntry {
        file_id: "slash-row".into(),
        path: "/docs/a.txt".into(),
        status: FileStatus::CloudOnly,
        size_bytes: 1,
        modified_at: 0,
        content_hash: None,
        remote_updated_at: 0,
        parent_id: None,
        item_kind: ItemKind::File,
    })
    .unwrap();
    assert_eq!(
        db.get_file_by_path("docs/a.txt").unwrap().map(|e| e.file_id),
        Some("slash-row".to_string())
    );
    // Exact form still hits.
    assert_eq!(
        db.get_file_by_path("/docs/a.txt").unwrap().map(|e| e.file_id),
        Some("slash-row".to_string())
    );

    // Row stored WITHOUT a leading slash (the bare relative shape). A query
    // that arrives with a leading slash must still hit.
    db.upsert_file(&FileEntry {
        file_id: "bare-row".into(),
        path: "photo.jpg".into(),
        status: FileStatus::CloudOnly,
        size_bytes: 1,
        modified_at: 0,
        content_hash: None,
        remote_updated_at: 0,
        parent_id: None,
        item_kind: ItemKind::File,
    })
    .unwrap();
    assert_eq!(
        db.get_file_by_path("/photo.jpg").unwrap().map(|e| e.file_id),
        Some("bare-row".to_string())
    );
    assert_eq!(
        db.get_file_by_path("photo.jpg").unwrap().map(|e| e.file_id),
        Some("bare-row".to_string())
    );

    // A genuinely-absent path returns None (and the slash-toggle of an
    // empty string must not panic / false-match).
    assert!(db.get_file_by_path("nope.txt").unwrap().is_none());
    assert!(db.get_file_by_path("").unwrap().is_none());
}

#[test]
fn test_list_by_status() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    let e = FileEntry {
        file_id: "x1".into(),
        path: "/f.txt".into(),
        status: FileStatus::Conflict,
        size_bytes: 0,
        modified_at: 0,
        content_hash: None,
        remote_updated_at: 0,
        parent_id: None,
        item_kind: ItemKind::File,
    };
    db.upsert_file(&e).unwrap();
    let conflicts = db.list_by_status(FileStatus::Conflict).unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].file_id, "x1");
}

#[test]
fn decode_os_state_status_only_for_user_owned_states() {
    // Cloud-only on disk → RECALL bit set.
    const RECALL: u32 = 0x0040_0000;
    const PINNED: u32 = 0x0008_0000;
    const UNPINNED: u32 = 0x0010_0000;

    // Local row, RECALL set on disk ⇒ desired CloudOnly; no pin bits ⇒ None.
    let (status, pin) = decode_os_state(RECALL, FileStatus::Local);
    assert_eq!(status, Some(FileStatus::CloudOnly));
    assert_eq!(pin, None);

    // CloudOnly row, RECALL clear (bytes resident) ⇒ desired Local.
    let (status, pin) = decode_os_state(0, FileStatus::CloudOnly);
    assert_eq!(status, Some(FileStatus::Local));
    assert_eq!(pin, None);

    // Engine-owned statuses are NEVER touched, regardless of attrs.
    // `Trashing` is included so a native attribute snapshot can't flip a
    // locally-deleted-but-not-yet-trashed file back to a seedable state.
    for owned in [
        FileStatus::Uploading,
        FileStatus::Downloading,
        FileStatus::Conflict,
        FileStatus::Error,
        FileStatus::Trashing,
    ] {
        let (status, _) = decode_os_state(RECALL, owned.clone());
        assert_eq!(status, None, "{owned:?} must be left alone");
    }

    // Pin bits decode independently of status.
    let (_, pin) = decode_os_state(PINNED, FileStatus::Local);
    assert_eq!(pin, Some(PinState::Pinned));
    let (_, pin) = decode_os_state(UNPINNED, FileStatus::Local);
    assert_eq!(pin, Some(PinState::Unpinned));
    // Both set: PINNED wins (checked first — a placeholder shouldn't carry
    // both, but be deterministic if it does).
    let (_, pin) = decode_os_state(PINNED | UNPINNED, FileStatus::CloudOnly);
    assert_eq!(pin, Some(PinState::Pinned));
    // RECALL set, no pin bit, engine-owned status: status None, pin None.
    let (status, pin) = decode_os_state(RECALL, FileStatus::Uploading);
    assert_eq!(status, None);
    assert_eq!(pin, None);
}

#[test]
fn trashing_status_round_trips_and_is_not_seeded_as_cloud_only() {
    // task 0802: the `Trashing` status (a locally-deleted file awaiting its
    // server-trash) must (a) round-trip through the string codec the DB uses
    // and (b) be EXCLUDED from `list_by_status(CloudOnly)` — the exact query
    // the Windows placeholder seeder (`populate_placeholders`) walks. If a
    // `Trashing` row leaked into that list the placeholder would be re-minted
    // and the deleted file would reappear on disk.
    assert_eq!(FileStatus::Trashing.as_str(), "trashing");
    assert_eq!(FileStatus::from_str("trashing"), FileStatus::Trashing);

    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();

    // One cloud-only row (would be seeded) and one trashing row (must NOT be).
    db.upsert_file(&FileEntry {
        file_id: "cloud-1".into(),
        path: "/keep.txt".into(),
        status: FileStatus::CloudOnly,
        size_bytes: 1,
        modified_at: 0,
        content_hash: None,
        remote_updated_at: 0,
        parent_id: None,
        item_kind: ItemKind::File,
    })
    .unwrap();
    db.upsert_file(&FileEntry {
        file_id: "trash-1".into(),
        path: "/gone.txt".into(),
        status: FileStatus::Trashing,
        size_bytes: 1,
        modified_at: 0,
        content_hash: None,
        remote_updated_at: 0,
        parent_id: None,
        item_kind: ItemKind::File,
    })
    .unwrap();

    // Persisted status survives a reload (string codec round-trip).
    assert_eq!(db.get_file("trash-1").unwrap().unwrap().status, FileStatus::Trashing);

    // The seeder's source list contains ONLY the cloud-only row.
    let cloud_only = db.list_by_status(FileStatus::CloudOnly).unwrap();
    assert_eq!(cloud_only.len(), 1);
    assert_eq!(cloud_only[0].file_id, "cloud-1");
    assert!(
        !cloud_only.iter().any(|e| e.file_id == "trash-1"),
        "a Trashing row must never be returned for CloudOnly seeding"
    );

    // The transition `watcher::handle_delete` performs after enqueuing the
    // trash op: flip a CloudOnly row to Trashing → it drops out of the seed
    // list, so the just-removed placeholder is not re-created.
    db.set_status("cloud-1", FileStatus::Trashing).unwrap();
    assert_eq!(db.get_file("cloud-1").unwrap().unwrap().status, FileStatus::Trashing);
    assert!(
        db.list_by_status(FileStatus::CloudOnly).unwrap().is_empty(),
        "after handle_delete marks the row Trashing, nothing remains to seed"
    );
}

#[test]
fn reconcile_os_state_applies_status_and_pin_deltas() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();

    // Empty input → fast Ok(0), no writes.
    assert_eq!(db.reconcile_os_state(&[], 100).unwrap(), 0);

    let seed = |file_id: &str| FileEntry {
        file_id: file_id.into(),
        path: format!("/{file_id}.txt"),
        status: FileStatus::CloudOnly,
        size_bytes: 0,
        modified_at: 0,
        content_hash: None,
        remote_updated_at: 0,
        parent_id: None,
        item_kind: ItemKind::File,
    };
    // status-only target, pin-only target, both target.
    for id in ["s_only", "p_only", "both"] {
        db.upsert_file(&seed(id)).unwrap();
    }

    let deltas = vec![
        // status-only: flip CloudOnly → Local, re-stamps last_sync_at.
        ("s_only".to_string(), Some(FileStatus::Local), None),
        // pin-only: leave status, set pinned.
        ("p_only".to_string(), None, Some(PinState::Pinned)),
        // both: status + pin.
        (
            "both".to_string(),
            Some(FileStatus::CloudOnly),
            Some(PinState::Unpinned),
        ),
        // missing row: WHERE matches nothing, contributes 0 to the count.
        ("ghost".to_string(), Some(FileStatus::Local), None),
    ];
    // s_only(1) + p_only(1) + both(2) + ghost(0) = 4 rows touched.
    let touched = db.reconcile_os_state(&deltas, 12345).unwrap();
    assert_eq!(touched, 4);

    // status-only applied and last_sync_at re-stamped.
    let s = db.get_file("s_only").unwrap().unwrap();
    assert_eq!(s.status, FileStatus::Local);
    let s_contract = db.get_file_contract_state("s_only").unwrap().unwrap();
    assert_eq!(s_contract.last_sync_at, 12345);

    // pin-only applied, status untouched.
    let p = db.get_file("p_only").unwrap().unwrap();
    assert_eq!(p.status, FileStatus::CloudOnly);
    let p_contract = db.get_file_contract_state("p_only").unwrap().unwrap();
    assert_eq!(p_contract.pin_state, PinState::Pinned);

    // both applied.
    let b_contract = db.get_file_contract_state("both").unwrap().unwrap();
    assert_eq!(b_contract.pin_state, PinState::Unpinned);
    assert_eq!(b_contract.last_sync_at, 12345);
    assert_eq!(db.get_file("both").unwrap().unwrap().status, FileStatus::CloudOnly);
}

#[test]
fn set_size_bytes_corrects_row_to_decrypted_length() {
    // Task 0783 Tier-1: a placeholder seeded with the (wrong) encrypted
    // size must be correctable to the AES-256-GCM-authenticated plaintext
    // length so the next hydration mints an aligned placeholder.
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();

    // file_size = plaintext_len + 28 (one single-chunk GCM nonce+tag) is
    // exactly the bad-row shape from the live trace.
    let plaintext_len: i64 = 20;
    let bad_size: i64 = plaintext_len + 28; // 48
    db.upsert_file(&FileEntry {
        file_id: "size-mismatch".into(),
        path: "/bb-test-upload.txt".into(),
        status: FileStatus::CloudOnly,
        size_bytes: bad_size,
        modified_at: 0,
        content_hash: None,
        remote_updated_at: 0,
        parent_id: None,
        item_kind: ItemKind::File,
    })
    .unwrap();

    // Correcting an existing row rewrites the column and reports 1 row.
    let updated = db.set_size_bytes("size-mismatch", plaintext_len).unwrap();
    assert_eq!(updated, 1);
    assert_eq!(db.get_file("size-mismatch").unwrap().unwrap().size_bytes, plaintext_len);

    // A negative length is clamped to 0 (defensive; never expected).
    db.set_size_bytes("size-mismatch", -5).unwrap();
    assert_eq!(db.get_file("size-mismatch").unwrap().unwrap().size_bytes, 0);

    // An absent file_id is a no-op (0 rows), not an error.
    assert_eq!(db.set_size_bytes("does-not-exist", 10).unwrap(), 0);
}

#[test]
fn test_migrates_drive_contract_columns_and_operation_queue() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("state.db");
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "
                CREATE TABLE files (
                    file_id TEXT PRIMARY KEY,
                    path TEXT NOT NULL,
                    status TEXT NOT NULL DEFAULT 'cloud_only',
                    size_bytes INTEGER NOT NULL DEFAULT 0,
                    modified_at INTEGER NOT NULL DEFAULT 0,
                    content_hash TEXT
                );
                INSERT INTO files (file_id, path, status)
                VALUES ('legacy', '/legacy.txt', 'local');
                ",
        )
        .unwrap();
    }

    let db = StateDb::open(&path).unwrap();
    let conn = db.0.lock().expect("state_db mutex poisoned");
    for column in [
        "remote_updated_at",
        "namespace",
        "parent_id",
        "shared_root_id",
        "share_id",
        "permission_bits",
        "item_kind",
        "content_type",
        "current_version",
        "current_object_version_id",
        "local_base_version",
        "local_hash",
        "cache_path",
        "cache_bytes",
        "pin_state",
        "inherited_pin_state",
        "last_sync_at",
        "last_opened_at",
    ] {
        assert!(has_column(&conn, "files", column).unwrap(), "missing {column}");
    }
    assert!(has_table(&conn, "operation_queue").unwrap());
    for column in [
        "op_id",
        "kind",
        "attempts",
        "next_retry_at",
        "last_error",
        "last_error_class",
        "paused_reason",
    ] {
        assert!(
            has_column(&conn, "operation_queue", column).unwrap(),
            "missing {column}"
        );
    }

    let row: (String, i64, String) = conn
        .query_row(
            "SELECT namespace, permission_bits, pin_state FROM files WHERE file_id = 'legacy'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(row, ("my_files".into(), 0, "inherit".into()));
}

#[test]
fn test_persists_drive_contract_state() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    db.upsert_file(&FileEntry {
        file_id: "file1".into(),
        path: "/Shared/report.pdf".into(),
        status: FileStatus::Local,
        size_bytes: 2048,
        modified_at: 10,
        content_hash: Some("remote-hash".into()),
        remote_updated_at: 11,
        parent_id: None,
        item_kind: ItemKind::File,
    })
    .unwrap();

    let contract = FileContractState {
        file_id: "file1".into(),
        namespace: Namespace::SharedWithMe,
        parent_id: Some("parent1".into()),
        shared_root_id: Some("root1".into()),
        share_id: Some("share1".into()),
        owner_email: None,
        permission_bits: PERMISSION_READ | PERMISSION_WRITE,
        item_kind: ItemKind::Folder,
        content_type: Some("public.folder".into()),
        current_version: 7,
        current_object_version_id: Some("object7".into()),
        local_base_version: 6,
        local_hash: Some("local-hash".into()),
        cache_path: Some("/tmp/beebeeb-cache/file1".into()),
        cache_bytes: 2048,
        pin_state: PinState::Inherit,
        inherited_pin_state: PinState::Pinned,
        last_sync_at: 1234,
    };
    db.set_file_contract_state(&contract).unwrap();

    let got = db.get_file_contract_state("file1").unwrap().unwrap();
    assert_eq!(got.namespace, Namespace::SharedWithMe);
    assert_eq!(got.shared_root_id.as_deref(), Some("root1"));
    assert_eq!(got.share_id.as_deref(), Some("share1"));
    assert_eq!(got.permission_bits, PERMISSION_READ | PERMISSION_WRITE);
    assert_eq!(got.item_kind, ItemKind::Folder);
    assert_eq!(got.content_type.as_deref(), Some("public.folder"));
    assert!(got.can_read());
    assert!(got.can_write());
    assert!(got.is_shared());
    assert_eq!(got.current_version, 7);
    assert_eq!(got.current_object_version_id.as_deref(), Some("object7"));
    assert_eq!(got.local_base_version, 6);
    assert_eq!(got.local_hash.as_deref(), Some("local-hash"));
    assert_eq!(got.cache_path.as_deref(), Some("/tmp/beebeeb-cache/file1"));
    assert_eq!(got.cache_bytes, 2048);
    assert_eq!(got.effective_pin_state(), PinState::Pinned);
}

#[test]
fn test_operation_queue_persists_retry_state() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    let op = PendingOperation {
        op_id: "op-1".into(),
        kind: OperationKind::UploadVersion,
        file_id: Some("file1".into()),
        parent_id: Some("parent1".into()),
        target_path: Some("/report.pdf".into()),
        metadata_json: Some(r#"{"name":"encrypted"}"#.into()),
        payload_path: Some("/tmp/payload".into()),
        base_version: Some(3),
        base_object_version_id: Some("object3".into()),
        attempts: 0,
        max_attempts: 5,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: 100,
        updated_at: 100,
    };
    db.enqueue_operation(&op).unwrap();

    db.record_operation_attempt("op-1", 2, 300, Some("timeout")).unwrap();
    let queued = db.list_due_operations(301).unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].kind, OperationKind::UploadVersion);
    assert_eq!(queued[0].attempts, 2);
    assert_eq!(queued[0].next_retry_at, 300);
    assert_eq!(queued[0].last_error.as_deref(), Some("timeout"));

    db.remove_operation("op-1").unwrap();
    assert!(db.list_due_operations(999).unwrap().is_empty());
}

/// Task 0811: disabling a known-folder backup purges EXACTLY its tagged ops.
/// A normal (untagged) op and a DIFFERENT folder's tagged op must survive,
/// and the surviving ops must keep their tag through an enqueue→read round
/// trip (so a later disable of the other folder still finds them).
#[test]
fn test_purge_backup_source_ops_is_surgical() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();

    let mk = |op_id: &str, key: Option<&str>| PendingOperation {
        op_id: op_id.into(),
        kind: OperationKind::UploadVersion,
        file_id: Some(format!("file-{op_id}")),
        parent_id: None,
        target_path: Some(format!("/{op_id}.bin")),
        metadata_json: Some(r#"{"operation":"upload_version"}"#.into()),
        payload_path: Some("/tmp/payload".into()),
        base_version: None,
        base_object_version_id: None,
        attempts: 0,
        max_attempts: 5,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: key.map(str::to_string),
        created_at: 100,
        updated_at: 100,
    };

    // Two ops for `music`, one for `pictures`, one normal (untagged).
    db.enqueue_operation(&mk("music-1", Some("music"))).unwrap();
    db.enqueue_operation(&mk("music-2", Some("music"))).unwrap();
    db.enqueue_operation(&mk("pics-1", Some("pictures"))).unwrap();
    db.enqueue_operation(&mk("normal-1", None)).unwrap();

    // The tag survives the enqueue→read round trip.
    let due = db.list_due_operations(999).unwrap();
    assert_eq!(due.len(), 4);
    let music_2 = due.iter().find(|o| o.op_id == "music-2").unwrap();
    assert_eq!(music_2.backup_source_key.as_deref(), Some("music"));
    let normal = due.iter().find(|o| o.op_id == "normal-1").unwrap();
    assert_eq!(normal.backup_source_key, None);

    // Disabling `music` purges exactly its two ops.
    let purged = db.purge_backup_source_ops("music").unwrap();
    assert_eq!(purged, 2, "both music ops purged");

    let remaining = db.list_due_operations(999).unwrap();
    let ids: std::collections::HashSet<&str> = remaining.iter().map(|o| o.op_id.as_str()).collect();
    assert_eq!(remaining.len(), 2, "only the pictures op + the normal op remain");
    assert!(ids.contains("pics-1"), "other folder's op untouched");
    assert!(ids.contains("normal-1"), "non-backup op untouched");
    assert!(!ids.contains("music-1") && !ids.contains("music-2"), "music ops gone");

    // Purging a key with no rows is a harmless no-op.
    assert_eq!(db.purge_backup_source_ops("videos").unwrap(), 0);
}

/// Task 0811: a tagged op survives a DB close/reopen with its tag intact
/// (the column is durable), so a disable AFTER a restart still purges it —
/// the whole point of tagging at insertion rather than in memory.
#[test]
fn test_backup_source_key_survives_restart() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("state.db");
    {
        let db = StateDb::open(&path).unwrap();
        db.enqueue_operation(&PendingOperation {
            op_id: "music-boot".into(),
            kind: OperationKind::UploadVersion,
            file_id: Some("file-boot".into()),
            parent_id: None,
            target_path: Some("/Backup/Dev/Music/a.mp3".into()),
            metadata_json: Some(r#"{"operation":"upload_version"}"#.into()),
            payload_path: Some("/tmp/payload".into()),
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 5,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: Some("music".into()),
            created_at: 100,
            updated_at: 100,
        })
        .unwrap();
    }
    // Reopen (simulates an app restart) and confirm the tag persisted, then
    // a disable purges it.
    let reopened = StateDb::open(&path).unwrap();
    let due = reopened.list_due_operations(999).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].backup_source_key.as_deref(), Some("music"));
    assert_eq!(reopened.purge_backup_source_ops("music").unwrap(), 1);
    assert!(reopened.list_due_operations(999).unwrap().is_empty());
}

#[test]
fn test_operation_pause_survives_restart_and_diagnostics_redacts() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("state.db");
    let db = StateDb::open(&path).unwrap();
    db.enqueue_operation(&PendingOperation {
        op_id: "op-auth".into(),
        kind: OperationKind::UploadVersion,
        file_id: Some("file1".into()),
        parent_id: None,
        target_path: Some("/secret.txt".into()),
        metadata_json: Some(r#"{"operation":"upload_version","token":"not-diagnostic"}"#.into()),
        payload_path: Some("/tmp/payload".into()),
        base_version: Some(3),
        base_object_version_id: Some("object3".into()),
        attempts: 1,
        max_attempts: 5,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: 100,
        updated_at: 100,
    })
    .unwrap();
    db.record_operation_pause(
        "op-auth",
        OperationPauseReason::Auth,
        Some("401 unauthorized Bearer abc.def.ghi session_token=super-secret"),
        200,
    )
    .unwrap();

    drop(db);
    let reopened = StateDb::open(&path).unwrap();
    assert!(reopened.list_due_operations(999).unwrap().is_empty());

    let diagnostics = reopened.queue_diagnostics(999).unwrap();
    assert_eq!(diagnostics.queued, 1);
    assert_eq!(diagnostics.due, 0);
    assert_eq!(diagnostics.paused, 1);
    assert_eq!(diagnostics.paused_by_reason.get("auth"), Some(&1));
    assert_eq!(diagnostics.last_error_class.as_deref(), Some("auth"));
    let last_error = diagnostics.last_error.unwrap();
    assert!(last_error.contains("Bearer [redacted]"));
    assert!(last_error.contains("session_token=[redacted]"));
    assert!(!last_error.contains("abc.def.ghi"));
    assert!(!last_error.contains("super-secret"));
}

/// Task 1685 (RED-FIRST): the support bundle promised "without secrets or
/// plaintext names", but only tokens were redacted. A real upload failure
/// carries the local path and the (possibly multi-word) folder/file names.
#[test]
fn diagnostics_export_never_contains_paths_or_known_names() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    seed_contract_row(&db, "f-tax", "Tax 2025/aangifte.pdf", None, FileStatus::Local, 10);
    db.enqueue_operation(&PendingOperation {
        op_id: "op-1".into(),
        kind: OperationKind::UploadVersion,
        file_id: Some("f-tax".into()),
        parent_id: None,
        target_path: Some("Tax 2025/aangifte.pdf".into()),
        metadata_json: None,
        payload_path: None,
        base_version: None,
        base_object_version_id: None,
        attempts: 0,
        max_attempts: 5,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: 100,
        updated_at: 100,
    })
    .unwrap();
    db.record_operation_attempt(
        "op-1",
        1,
        130,
        Some("/Users/guus/Library/CloudStorage/Beebeeb-Drive/Tax 2025/aangifte.pdf failed"),
    )
    .unwrap();

    let diagnostics = db.queue_diagnostics(200).unwrap();
    let exported = serde_json::to_string(&diagnostics).unwrap();
    assert!(!exported.contains("/Users/guus"), "path leaked: {exported}");
    assert!(!exported.contains("CloudStorage"), "path leaked: {exported}");
    assert!(!exported.contains("Tax 2025"), "known folder name leaked: {exported}");
    assert!(!exported.contains("aangifte"), "known file name leaked: {exported}");
    // The failure itself is still reported, and the removal is counted.
    assert!(
        diagnostics.last_error.as_deref().unwrap().contains("failed"),
        "{exported}"
    );
    assert!(diagnostics.last_error_redactions >= 1, "{exported}");
    assert_eq!(
        diagnostics.last_error_code,
        Some(DiagnosticErrorCode::Other),
        "{exported}"
    );
}

/// Task 1685: a name that exists only in a queued op's staged payload path
/// (not in `files`) is still known to the daemon and must be scrubbed. `2026`
/// is a bare number the allow-list keeps, so only the name scan can remove it.
#[test]
fn diagnostics_scrubs_names_known_only_from_a_staged_payload_path() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    db.enqueue_operation(&PendingOperation {
        op_id: "op-1".into(),
        kind: OperationKind::UploadFile,
        file_id: None,
        parent_id: None,
        target_path: None,
        metadata_json: None,
        payload_path: Some("/staging/ab12/Q3 2026".into()),
        base_version: None,
        base_object_version_id: None,
        attempts: 0,
        max_attempts: 5,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: 1,
        updated_at: 1,
    })
    .unwrap();
    db.record_operation_attempt("op-1", 1, 10, Some("copy of Q3 2026 failed"))
        .unwrap();
    let exported = serde_json::to_string(&db.queue_diagnostics(200).unwrap()).unwrap();
    assert!(!exported.contains("2026"), "{exported}");
    assert!(!exported.contains("Q3"), "{exported}");
    assert!(exported.contains("failed"), "{exported}");
}

/// Task 1685: group labels are an allow-list, so a stray DB value (a name in
/// `kind` or `paused_reason`) can never become a key in the export.
#[test]
fn diagnostics_group_labels_outside_the_allow_list_become_other() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    for id in ["op-a", "op-b"] {
        db.enqueue_operation(&PendingOperation {
            op_id: id.into(),
            kind: OperationKind::UploadVersion,
            file_id: None,
            parent_id: None,
            target_path: None,
            metadata_json: None,
            payload_path: None,
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 5,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 100,
            updated_at: 100,
        })
        .unwrap();
    }
    {
        let conn = db.0.lock().unwrap();
        conn.execute("UPDATE operation_queue SET kind = 'Tax 2025' WHERE op_id = 'op-a'", [])
            .unwrap();
        conn.execute(
                "UPDATE operation_queue SET paused_reason = '/Users/guus/Tax 2025', last_error_class = 'aangifte.pdf' WHERE op_id = 'op-b'",
                [],
            )
            .unwrap();
    }
    let diagnostics = db.queue_diagnostics(200).unwrap();
    let exported = serde_json::to_string(&diagnostics).unwrap();
    assert!(!exported.contains("Tax"), "{exported}");
    assert!(!exported.contains("guus"), "{exported}");
    assert!(!exported.contains("aangifte"), "{exported}");
    assert_eq!(diagnostics.by_kind.get("other"), Some(&1), "{exported}");
    assert_eq!(diagnostics.by_kind.get("upload_version"), Some(&1), "{exported}");
    assert_eq!(diagnostics.paused_by_reason.get("other"), Some(&1), "{exported}");
}

#[test]
fn test_revoked_shared_content_is_removed_and_cache_paths_returned() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    seed_contract_row(&db, "active", "/Shared with me/Active", None, FileStatus::Local, 10);
    seed_contract_row(&db, "revoked", "/Shared with me/Revoked", None, FileStatus::Local, 20);

    let mut active = db.get_file_contract_state("active").unwrap().unwrap();
    active.namespace = Namespace::SharedWithMe;
    active.shared_root_id = Some("active".into());
    active.share_id = Some("invite-active".into());
    active.permission_bits = PERMISSION_READ;
    active.cache_path = Some("/cache/active".into());
    db.set_file_contract_state(&active).unwrap();

    let mut revoked = db.get_file_contract_state("revoked").unwrap().unwrap();
    revoked.namespace = Namespace::SharedWithMe;
    revoked.shared_root_id = Some("revoked".into());
    revoked.share_id = Some("invite-revoked".into());
    revoked.permission_bits = PERMISSION_READ;
    revoked.cache_path = Some("/cache/revoked".into());
    db.set_file_contract_state(&revoked).unwrap();

    let removed = db.purge_revoked_shared_content(&["active".to_string()]).unwrap();

    assert_eq!(
        removed,
        vec![RevokedSharedCache {
            file_id: "revoked".into(),
            cache_path: Some("/cache/revoked".into()),
        }]
    );
    assert!(db.get_file("active").unwrap().is_some());
    assert!(db.get_file("revoked").unwrap().is_none());
}

#[test]
fn test_recursive_pin_inheritance_persists_for_folder_tree() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    seed_contract_row(&db, "folder-a", "/Projects", None, FileStatus::CloudOnly, 0);
    seed_contract_row(
        &db,
        "file-a",
        "/Projects/report.txt",
        Some("folder-a"),
        FileStatus::CloudOnly,
        100,
    );
    seed_contract_row(
        &db,
        "nested-folder",
        "/Projects/Nested",
        Some("folder-a"),
        FileStatus::CloudOnly,
        0,
    );
    seed_contract_row(
        &db,
        "file-b",
        "/Projects/Nested/spec.md",
        Some("nested-folder"),
        FileStatus::CloudOnly,
        200,
    );

    let changed = db.set_recursive_pin("folder-a", true, 1000).unwrap();
    assert_eq!(changed.len(), 4);
    assert_eq!(
        db.get_file_contract_state("folder-a").unwrap().unwrap().pin_state,
        PinState::Pinned
    );
    assert_eq!(
        db.get_file_contract_state("file-b")
            .unwrap()
            .unwrap()
            .effective_pin_state(),
        PinState::Pinned
    );

    db.set_recursive_pin("folder-a", false, 2000).unwrap();
    assert_eq!(
        db.get_file_contract_state("file-b")
            .unwrap()
            .unwrap()
            .effective_pin_state(),
        PinState::Unpinned
    );
}

#[test]
fn test_cache_eviction_preserves_pinned_and_uploading_files() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    seed_contract_row(&db, "pinned", "/Pinned.txt", None, FileStatus::Local, 800);
    seed_contract_row(&db, "uploading", "/Uploading.txt", None, FileStatus::Uploading, 900);
    seed_contract_row(&db, "old", "/Old.txt", None, FileStatus::Local, 700);
    seed_contract_row(&db, "new", "/New.txt", None, FileStatus::Local, 600);

    db.set_recursive_pin("pinned", true, 10).unwrap();
    db.mark_cached("pinned", "/cache/pinned", 800, 10).unwrap();
    db.mark_cached("uploading", "/cache/uploading", 900, 20).unwrap();
    db.mark_cached("old", "/cache/old", 700, 30).unwrap();
    db.mark_cached("new", "/cache/new", 600, 40).unwrap();

    let evicted = db.evict_unpinned_cache_until_under(1_000, 50).unwrap();
    assert_eq!(evicted, vec!["old".to_string(), "new".to_string()]);
    assert_eq!(db.get_file("old").unwrap().unwrap().status, FileStatus::CloudOnly);
    assert_eq!(db.get_file("new").unwrap().unwrap().status, FileStatus::CloudOnly);
    assert_eq!(db.get_file("pinned").unwrap().unwrap().status, FileStatus::Local);
    assert_eq!(db.get_file("uploading").unwrap().unwrap().status, FileStatus::Uploading);
}

#[test]
fn test_disposable_cache_cleanup_candidates_skip_pinned_and_uploading_files() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    seed_contract_row(&db, "pinned", "/Pinned.txt", None, FileStatus::Local, 800);
    seed_contract_row(&db, "uploading", "/Uploading.txt", None, FileStatus::Uploading, 900);
    seed_contract_row(&db, "local", "/Local.txt", None, FileStatus::Local, 700);

    db.set_recursive_pin("pinned", true, 10).unwrap();
    db.mark_cached("pinned", "/cache/pinned", 800, 10).unwrap();
    db.mark_cached("uploading", "/cache/uploading", 900, 20).unwrap();
    db.mark_cached("local", "/cache/local", 700, 30).unwrap();

    assert_eq!(
        db.disposable_unpinned_cache_paths().unwrap(),
        vec!["/cache/local".to_string()]
    );

    let cleared = db
        .clear_cache_metadata_for_paths(&["/cache/local".to_string()], 100)
        .unwrap();
    assert_eq!(cleared, 1);
    assert_eq!(db.get_file("local").unwrap().unwrap().status, FileStatus::CloudOnly);
    assert_eq!(db.get_file_contract_state("local").unwrap().unwrap().cache_path, None);
    assert_eq!(
        db.get_file_contract_state("pinned")
            .unwrap()
            .unwrap()
            .cache_path
            .as_deref(),
        Some("/cache/pinned")
    );
    assert_eq!(
        db.get_file_contract_state("uploading")
            .unwrap()
            .unwrap()
            .cache_path
            .as_deref(),
        Some("/cache/uploading")
    );
}

/// Task 1538 finding 1 + 2: sign-out must purge EVERY queued operation
/// (regardless of retry/paused state) and EVERY cached plaintext path
/// (regardless of pin state), unlike `purge_backup_source_ops` (scoped to
/// one backup tag) or `evict_unpinned_cache_until_under`/
/// `disposable_unpinned_cache_paths` (both explicitly skip pinned and
/// non-`local`-status files) — a pin set by the previous account must not
/// protect that account's plaintext from being purged on sign-out.
#[test]
fn test_purge_all_local_state_clears_every_queued_op_and_cached_path() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();

    // A pinned file and an uploading file survive normal cache eviction —
    // sign-out must clear their cache_path anyway.
    seed_contract_row(&db, "pinned", "/Pinned.txt", None, FileStatus::Local, 800);
    seed_contract_row(&db, "uploading", "/Uploading.txt", None, FileStatus::Uploading, 900);
    db.set_recursive_pin("pinned", true, 10).unwrap();
    db.mark_cached("pinned", "/cache/pinned", 800, 10).unwrap();
    db.mark_cached("uploading", "/cache/uploading", 900, 20).unwrap();

    // A paused op (would never show up in `list_due_operations`) still
    // must be purged — an account switch must not leave it to resume
    // silently if a later `resume`/retry ever clears the pause.
    let queued_op = PendingOperation {
        op_id: "op-1".into(),
        kind: OperationKind::UploadVersion,
        file_id: Some("pinned".into()),
        parent_id: None,
        target_path: Some("/Pinned.txt".into()),
        metadata_json: None,
        payload_path: Some("/staging/op-1-payload".into()),
        base_version: None,
        base_object_version_id: None,
        attempts: 0,
        max_attempts: 5,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: 100,
        updated_at: 100,
    };
    db.enqueue_operation(&queued_op).unwrap();
    db.record_operation_pause("op-1", OperationPauseReason::Auth, Some("offline"), 100)
        .unwrap();

    let purge = db.purge_all_local_state().unwrap();

    assert_eq!(purge.queued_ops_purged, 1);
    assert_eq!(purge.payload_paths, vec!["/staging/op-1-payload".to_string()]);
    let mut cache_paths = purge.cache_paths.clone();
    cache_paths.sort();
    assert_eq!(
        cache_paths,
        vec!["/cache/pinned".to_string(), "/cache/uploading".to_string()]
    );
    // Only the `local`-status row ("pinned") is a placeholder-dehydration
    // candidate — "uploading" is mid-transfer, not a materialized local
    // copy, so it must not appear here.
    assert_eq!(
        purge.local_placeholder_paths,
        vec![("pinned".to_string(), "/Pinned.txt".to_string())]
    );

    // Nothing is left behind to be found by a later account's engine.
    assert!(db.list_due_operations(i64::MAX).unwrap().is_empty());
    assert!(db.list_review_operations().unwrap().is_empty());
    assert_eq!(db.get_file("pinned").unwrap().unwrap().status, FileStatus::CloudOnly);
    assert_eq!(db.get_file_contract_state("pinned").unwrap().unwrap().cache_path, None);
    assert_eq!(
        db.get_file_contract_state("uploading").unwrap().unwrap().cache_path,
        None
    );
}

/// Task 1538 Codex P1 (PR #49, state_db.rs:1785 thread): a Windows Cloud
/// Files placeholder stores its plaintext directly in the sync root, not
/// at `cache_path` — a materialized `local` row can have `cache_path =
/// NULL` the whole time (see `unpinned_local_files_for_dehydration`'s doc
/// comment). The OLD `cache_path IS NOT NULL`-only purge silently skips
/// such a row entirely: it stays `local` forever, and its on-disk
/// plaintext (the placeholder) is never found by ANY later cleanup pass,
/// because `unpinned_local_files_for_dehydration()` also filters on
/// `status = 'local'` — a status this purge would have left unchanged.
#[test]
fn test_purge_all_local_state_captures_windows_style_local_row_with_no_cache_path() {
    let dir = tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();

    // A materialized Windows placeholder: `local` status, PINNED (a
    // sign-out purge must sweep it anyway — pins don't survive the
    // account that set them), and — unlike every other seeded row in
    // this file — `cache_path` is never populated via `mark_cached`.
    // This is the exact shape a real Windows Cloud Files placeholder
    // leaves in the DB (the fetch callback's `%TEMP%` decrypt copy is
    // already deleted by the time the placeholder is materialized).
    seed_contract_row(&db, "win-local", "/Docs/report.docx", None, FileStatus::Local, 4096);
    db.set_recursive_pin("win-local", true, 10).unwrap();
    assert_eq!(
        db.get_file_contract_state("win-local").unwrap().unwrap().cache_path,
        None,
        "precondition: this row must never have a cache_path, like a real Windows placeholder"
    );

    let purge = db.purge_all_local_state().unwrap();

    // The row has no cache_path, so it can never appear in `cache_paths`
    // — proving the assertions below exercise a genuinely different code
    // path, not a duplicate of the existing cache_path-based one.
    assert!(purge.cache_paths.is_empty());
    assert_eq!(
        purge.local_placeholder_paths,
        vec![("win-local".to_string(), "/Docs/report.docx".to_string())],
        "a `local` row with no cache_path must still surface as a purge candidate \
             for the caller to dehydrate/remove"
    );
    assert_eq!(
        db.get_file("win-local").unwrap().unwrap().status,
        FileStatus::CloudOnly,
        "the row must not be left at `local` status after a sign-out purge — a \
             lingering `local` row hides real on-disk plaintext from every later cleanup pass"
    );
}

/// Spec 2026-10-10 §5.2 steps 7-8: a sign-out's purge can meet another writer on `state.db` (the keep step's close
/// of Finder accepts, or a Finder change that reads that fence in its own write transaction). The purge reads
/// before it writes. In a deferred transaction SQLite answers its first write "database is locked" at once while
/// another connection holds the write lock (a reader that asks to write is not given the busy timeout), and a
/// sign-out has deleted every file by then. The purge takes the write lock as it begins, so it waits as any other
/// writer here does.
#[test]
fn a_purge_waits_for_another_writer_instead_of_failing() {
    let hold = std::time::Duration::from_millis(200);
    for _ in 0..5 {
        let dir = tempdir().unwrap();
        let path = dir.path().join("state.db");
        let db = StateDb::open(&path).unwrap();
        let writer = StateDb::open(&path).unwrap();
        db.enqueue_operation(&queued("w1", OperationKind::UploadVersion, "f1", Some("/staged/w1")))
            .unwrap();
        let (held, wait_held) = std::sync::mpsc::channel();
        let holder = std::thread::spawn(move || {
            let mut conn = writer.hold_lock_for_test();
            let tx = conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .unwrap();
            tx.execute(
                "INSERT INTO sync_state (key, value) VALUES (?1, '1')",
                params![FINDER_ACCEPTS_CLOSED_KEY],
            )
            .unwrap();
            held.send(()).unwrap();
            std::thread::sleep(hold);
            let released = std::time::Instant::now();
            tx.commit().unwrap();
            released
        });
        wait_held.recv().unwrap();
        let started = std::time::Instant::now();
        let purge = db.purge_all_local_state();
        let waited = started.elapsed();
        let released = holder.join().unwrap();
        if started >= released {
            // This thread ran only after the hold had ended, so this purge never met the writer: once more.
            continue;
        }
        let purge = purge
            .unwrap_or_else(|error| panic!("a purge that meets another writer waits for it: {error} after {waited:?}"));
        assert_eq!(
            (purge.queued_ops_purged, purge.payload_paths),
            (1, vec!["/staged/w1".to_string()]),
            "the purge that waited did its work"
        );
        let left: i64 = db
            .hold_lock_for_test()
            .query_row("SELECT COUNT(*) FROM operation_queue", [], |row| row.get(0))
            .unwrap();
        assert_eq!(left, 0, "nothing is left queued");
        return;
    }
    panic!("in 5 tries the purge never began while the other connection held the write lock");
}
