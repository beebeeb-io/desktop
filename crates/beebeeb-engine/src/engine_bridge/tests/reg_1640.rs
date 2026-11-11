//! `reg_1640` engine tests, split out of `engine_bridge.rs`.

use super::*;
/// One chunk of `plaintext` as the server stores it for `file_id`.
pub(super) fn regression_1640_chunk(master_key: [u8; 32], file_id: &str, plaintext: &[u8]) -> Vec<u8> {
    beebeeb_core::encrypt::encrypt_chunk_raw(
        &beebeeb_core::kdf::derive_file_key(
            &beebeeb_core::kdf::MasterKey::from_bytes(master_key),
            file_id.as_bytes(),
        ),
        plaintext,
    )
    .unwrap()
}

/// The web saves `plaintext` as `file_id`'s next version, and the next snapshot lists it.
pub(super) fn regression_1640_web_saves(
    server: &VersionedServerMock,
    master_key: [u8; 32],
    file_id: &str,
    plaintext: &[u8],
) {
    let mut s = server.state.lock().unwrap();
    let chunk = regression_1640_chunk(master_key, file_id, plaintext);
    let file = s.files.get_mut(file_id).unwrap();
    file.versions.push(vec![chunk]);
    let version = file.versions.len() as i64;
    let mut node = snap_node(&master_key, file_id, "notes.txt", None, false, 1_700_000_500);
    node["version_number"] = json!(version);
    node["size_bytes"] = json!(plaintext.len());
    s.authenticated_names
        .insert(file_id.into(), node["name_encrypted"].as_str().unwrap().into());
    s.snapshots
        .push_back(("200 OK".into(), json!({ "seq_id": 1, "nodes": [node] })));
}

/// A synced file: the server holds `b"version one"` at version 1, and it was hydrated into
/// its place in the sync folder. Returns that place.
pub(super) async fn regression_1640_synced_file(
    master_key: [u8; 32],
    file: &str,
) -> (tempfile::TempDir, PathBuf, VersionedServerMock, EngineBridge, PathBuf) {
    let (dir, sync_root, server, bridge) = rule3_setup(master_key);
    seed_uploaded_row(&bridge, &server, file);
    {
        let mut s = server.state.lock().unwrap();
        s.files.get_mut(file).unwrap().versions[0] = vec![regression_1640_chunk(master_key, file, b"version one")];
        s.serve_downloads = true;
    }
    let local = sync_root.join("notes.txt");
    bridge.hydrate_file(file, &local, &[sync_root.as_path()]).await.unwrap();
    assert_eq!(std::fs::read(&local).unwrap(), b"version one", "precondition: hydrated");
    (dir, sync_root, server, bridge, local)
}

#[tokio::test]
async fn regression_1640_a_hydrate_into_the_sync_folder_records_the_agreed_content() {
    let file = "16400000-0000-4000-8000-000000000001";
    let (_dir, _root, server, bridge, _local) = regression_1640_synced_file([164u8; 32], file).await;
    let contract = bridge.db.get_file_contract_state(file).unwrap().unwrap();
    assert_eq!(contract.local_base_version, 1);
    assert_eq!(
        contract.local_hash.as_deref(),
        Some(sha256_hex(b"version one").as_str())
    );
    drop(server.finish());
}

#[tokio::test]
async fn regression_1640_a_close_without_an_edit_queues_nothing() {
    let file = "16400000-0000-4000-8000-000000000002";
    let (_dir, sync_root, server, bridge, local) = regression_1640_synced_file([165u8; 32], file).await;
    assert!(!crate::watcher::dispatch_local_create(
        &bridge, &sync_root, &local, "notify"
    ));
    assert!(bridge.db.list_operations_for_file(file).unwrap().is_empty());
    assert_eq!(bridge.db.get_file(file).unwrap().unwrap().status, FileStatus::Local);
    drop(server.finish());
}

#[tokio::test]
async fn regression_1640_an_edit_of_a_synced_file_is_queued_on_its_synced_base() {
    let file = "16400000-0000-4000-8000-000000000003";
    let (_dir, sync_root, server, bridge, local) = regression_1640_synced_file([166u8; 32], file).await;
    std::fs::write(&local, b"edited in the sync folder").unwrap();
    assert!(crate::watcher::dispatch_local_create(
        &bridge, &sync_root, &local, "notify"
    ));
    let ops = bridge.db.list_operations_for_file(file).unwrap();
    assert_eq!(ops.len(), 1, "one upload queued");
    assert_eq!(ops[0].kind, OperationKind::UploadVersion);
    assert_eq!(
        ops[0].base_version,
        Some(1),
        "sent on the version the bytes are based on"
    );
    assert_eq!(
        ops[0].target_path.as_deref(),
        Some("notes.txt"),
        "the full relative path"
    );
    assert_eq!(
        std::fs::read(ops[0].payload_path.as_deref().unwrap()).unwrap(),
        b"edited in the sync folder"
    );
    assert_eq!(bridge.db.get_file(file).unwrap().unwrap().status, FileStatus::Uploading);
    assert!(
        !crate::watcher::dispatch_local_create(&bridge, &sync_root, &local, "scan"),
        "a second close or scan queues nothing more while the upload is queued"
    );
    assert_eq!(bridge.db.list_operations_for_file(file).unwrap().len(), 1);
    drop(server.finish());
}

/// Task 2271: two rows at one path, as the 0.8.18 reconcile pass left 89 on the maintainer's PC. The first is the
/// server's own file of that name (no content recorded on this PC), the second the copy this PC sent (its content
/// agreed at version 1). The scan judges the bytes by the row that records them: unchanged bytes queue nothing,
/// and an edit is queued on that row, never settled as "Up to date" because the other row has nothing to compare.
#[tokio::test]
async fn regression_2271_a_path_with_two_rows_is_judged_by_the_row_that_records_its_content() {
    let master_key = [227u8; 32];
    let server_twin = "22710000-0000-4000-8000-000000000001";
    let file = "22710000-0000-4000-8000-000000000002";
    let (_dir, sync_root, server, bridge) = rule3_setup(master_key);
    seed_uploaded_db_row(&bridge, server_twin);
    seed_uploaded_row(&bridge, &server, file);
    {
        let mut s = server.state.lock().unwrap();
        s.files.get_mut(file).unwrap().versions[0] = vec![regression_1640_chunk(master_key, file, b"version one")];
        s.serve_downloads = true;
    }
    let local = sync_root.join("notes.txt");
    bridge.hydrate_file(file, &local, &[sync_root.as_path()]).await.unwrap();
    assert_eq!(
        bridge.db.get_file_by_path("notes.txt").unwrap().unwrap().file_id,
        server_twin,
        "precondition: the path's first row is the server's file, which records no content here"
    );

    assert!(
        !crate::watcher::dispatch_local_create(&bridge, &sync_root, &local, "scan"),
        "agreed bytes"
    );
    assert!(bridge.db.list_operations_for_file(file).unwrap().is_empty());

    std::fs::write(&local, b"edited in the sync folder").unwrap();
    assert!(
        crate::watcher::dispatch_local_create(&bridge, &sync_root, &local, "scan"),
        "the edit is queued"
    );
    let ops = bridge.db.list_operations_for_file(file).unwrap();
    assert_eq!(ops.len(), 1, "one upload, on the row that records the content");
    assert_eq!(
        (ops[0].kind.clone(), ops[0].base_version),
        (OperationKind::UploadVersion, Some(1))
    );
    assert!(
        bridge.db.list_operations_for_file(server_twin).unwrap().is_empty(),
        "nothing on the server's file"
    );
    drop(server.finish());
}

/// Task 2271: several rows at one path that each record content: the bytes pick the row they agree with, and bytes
/// that agree with none are never sent, neither as an edit of a guessed row nor as a new file.
#[tokio::test]
async fn regression_2271_several_rows_with_content_at_one_path_are_never_guessed() {
    let (_dir, sync_root, server, bridge) = rule3_setup([228u8; 32]);
    let (a, b) = (
        "22710000-0000-4000-8000-000000000003",
        "22710000-0000-4000-8000-000000000004",
    );
    for (id, bytes) in [(a, b"copy a".as_slice()), (b, b"copy b".as_slice())] {
        seed_uploaded_db_row(&bridge, id);
        let mut contract = bridge.db.get_file_contract_state(id).unwrap().unwrap();
        contract.local_hash = Some(sha256_hex(bytes));
        contract.local_base_version = 1;
        bridge.db.set_file_contract_state(&contract).unwrap();
    }
    let queued = || -> i64 {
        bridge
            .db
            .hold_lock_for_test()
            .query_row("SELECT COUNT(*) FROM operation_queue", [], |row| row.get(0))
            .unwrap()
    };
    let local = sync_root.join("notes.txt");
    std::fs::write(&local, b"copy b").unwrap();
    assert!(
        !crate::watcher::dispatch_local_create(&bridge, &sync_root, &local, "scan"),
        "the bytes are b's"
    );
    assert_eq!(queued(), 0);
    std::fs::write(&local, b"edited, agrees with neither").unwrap();
    let mut dispatched = true;
    let logs = capture_logs_async(async {
        dispatched = crate::watcher::dispatch_local_create(&bridge, &sync_root, &local, "scan");
    })
    .await;
    assert!(!dispatched, "no row is guessed");
    assert_eq!(queued(), 0, "nothing sent: no edit of a guessed row, no new file");
    // Lead ruling P2 (contract §7): file ids and the row count, never the path (log lines can leave the PC in the
    // support bundle).
    assert!(
        logs.contains(a) && logs.contains(b) && logs.contains("rows=2"),
        "{logs}"
    );
    assert!(!logs.contains("notes.txt"), "no path in the log: {logs}");
    drop(server.finish());
}

/// Task 2271 review: several rows that record content at the path of a cloud-only placeholder. Choosing a row
/// would read the bytes, and reading a placeholder that is not wholly on disk fetches it (here nothing answers, so
/// the read blocks for about a minute). The scan asks again later instead, as it does for one row, and reads
/// nothing. The root is registered and connected with no callbacks, as `found_while_paused`'s tests do.
#[cfg(target_os = "windows")]
#[tokio::test]
async fn regression_2271_several_rows_at_a_cloud_only_placeholder_read_no_bytes() {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::CloudFilters::{
        CF_CALLBACK_REGISTRATION, CF_CALLBACK_TYPE_NONE, CF_CONNECT_FLAG_NONE, CfConnectSyncRoot, CfDisconnectSyncRoot,
    };
    struct Root(PathBuf, windows::Win32::Storage::CloudFilters::CF_CONNECTION_KEY);
    impl Drop for Root {
        fn drop(&mut self) {
            // SAFETY: the key of the connection made below, disconnected once.
            let _ = unsafe { CfDisconnectSyncRoot(self.1) };
            let _ = crate::windows_cf::unregister_sync_root(&self.0);
        }
    }
    let (_dir, sync_root, server, bridge) = rule3_setup([229u8; 32]);
    crate::windows_cf::register_test_sync_root(&sync_root).unwrap();
    let table = Box::new([CF_CALLBACK_REGISTRATION {
        Type: CF_CALLBACK_TYPE_NONE,
        Callback: None,
    }]);
    let wide: Vec<u16> = sync_root.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: the path and the table outlive the connection (the table until the end of the test).
    let key = unsafe {
        CfConnectSyncRoot(
            windows::core::PCWSTR(wide.as_ptr()),
            table.as_ptr(),
            None,
            CF_CONNECT_FLAG_NONE,
        )
    }
    .unwrap();
    let _root = Root(sync_root.clone(), key);
    let (a, b) = (
        "22710000-0000-4000-8000-000000000005",
        "22710000-0000-4000-8000-000000000006",
    );
    for id in [a, b] {
        seed_uploaded_db_row(&bridge, id);
        let mut contract = bridge.db.get_file_contract_state(id).unwrap().unwrap();
        contract.local_hash = Some(sha256_hex(id.as_bytes()));
        contract.local_base_version = 1;
        bridge.db.set_file_contract_state(&contract).unwrap();
    }
    assert!(
        crate::windows_cf::placeholders::create_placeholder(&sync_root, b, "notes.txt", 1 << 20, 1_700_000_000, false)
            .unwrap()
    );
    let local = sync_root.join("notes.txt");
    assert!(
        !crate::windows_cf::placeholders::fully_on_disk(&local),
        "precondition: cloud-only"
    );

    let reads = HASH_READS.with(std::cell::Cell::get);
    let started = std::time::Instant::now();
    let verdict = bridge.classify_local_path_verdict(&sync_root, &local);
    let took = started.elapsed();

    assert!(matches!(verdict, LocalPathVerdict::Skip), "asked again later");
    assert_eq!(HASH_READS.with(std::cell::Cell::get), reads, "no bytes read");
    assert!(
        took < std::time::Duration::from_secs(20),
        "nothing asked for the bytes: {took:?}"
    );
    assert!(
        !crate::windows_cf::placeholders::fully_on_disk(&local),
        "still cloud-only"
    );
    drop(server.finish());
    // `_root` (declared after `table`) drops first: the root is disconnected before its callback table is freed.
}

#[tokio::test]
async fn regression_1640_an_edit_lands_as_the_next_version_and_becomes_the_agreed_content() {
    let master_key = [167u8; 32];
    let file = "16400000-0000-4000-8000-000000000004";
    let (_dir, sync_root, server, bridge, local) = regression_1640_synced_file(master_key, file).await;
    std::fs::write(&local, b"edited in the sync folder").unwrap();
    assert!(crate::watcher::dispatch_local_create(
        &bridge, &sync_root, &local, "notify"
    ));
    drain_upload_queue(&bridge, &sync_root).await;
    assert!(bridge.db.list_operations_for_file(file).unwrap().is_empty(), "landed");
    let row = bridge.db.get_file(file).unwrap().unwrap();
    assert_eq!((row.status, row.path.as_str()), (FileStatus::Local, "notes.txt"));
    let contract = bridge.db.get_file_contract_state(file).unwrap().unwrap();
    assert_eq!(contract.local_base_version, 2);
    assert_eq!(
        contract.local_hash.as_deref(),
        Some(sha256_hex(b"edited in the sync folder").as_str()),
        "what was sent is the agreed content now"
    );
    assert!(
        !crate::watcher::dispatch_local_create(&bridge, &sync_root, &local, "notify"),
        "the landed bytes are not an edit"
    );
    // The next edit goes on the version this one produced.
    std::fs::write(&local, b"edited again").unwrap();
    assert!(crate::watcher::dispatch_local_create(
        &bridge, &sync_root, &local, "notify"
    ));
    assert_eq!(
        bridge.db.list_operations_for_file(file).unwrap()[0].base_version,
        Some(2)
    );
    let state = server.finish();
    assert_eq!(state.init_summary(), vec![(json!(file), json!(1), 201)]);
    assert_eq!(state.latest_plaintext(file, master_key), b"edited in the sync folder");
}

/// The 2190 case, with the trigger: the web saves version 2 and the tick re-anchors the row
/// before the edit is seen. The edit is sent on 1 (not the row's 2), refused as stale, parks
/// with its bytes, and no hydrate writes the web's version over it. Both versions survive.
#[tokio::test]
async fn regression_1640_an_edit_made_while_the_web_saved_parks_with_its_bytes_and_is_never_overwritten() {
    let master_key = [168u8; 32];
    let file = "16400000-0000-4000-8000-000000000005";
    let (_dir, sync_root, server, bridge, local) = regression_1640_synced_file(master_key, file).await;
    regression_1640_web_saves(&server, master_key, file, b"edited on the web");
    sync_tick_outcome(&bridge, &sync_root).await.unwrap();
    assert_eq!(
        bridge
            .db
            .get_file_contract_state(file)
            .unwrap()
            .unwrap()
            .current_version,
        2,
        "precondition: the tick raised the row to the web's version"
    );
    std::fs::write(&local, b"edited in the sync folder").unwrap();
    assert!(crate::watcher::dispatch_local_create(
        &bridge, &sync_root, &local, "notify"
    ));
    assert_eq!(
        bridge.db.list_operations_for_file(file).unwrap()[0].base_version,
        Some(1),
        "the base is the version the bytes are based on, never the row's current one"
    );
    bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
    let parked = bridge.db.list_operations_for_file(file).unwrap().remove(0);
    assert_eq!(parked.attempts, parked.max_attempts, "parked at once");
    assert_eq!(parked.last_error.as_deref(), Some("stale_base"));
    assert_eq!(
        std::fs::read(parked.payload_path.as_deref().unwrap()).unwrap(),
        b"edited in the sync folder",
        "the parked op keeps the edit's bytes"
    );
    assert_eq!(bridge.db.get_file(file).unwrap().unwrap().status, FileStatus::Error);
    let refused = bridge
        .hydrate_file(file, &local, &[sync_root.as_path()])
        .await
        .unwrap_err();
    assert!(engine_cause::<UnsentLocalChange>(&refused).is_some(), "{refused:#}");
    assert_eq!(
        std::fs::read(&local).unwrap(),
        b"edited in the sync folder",
        "the edit stays on disk"
    );
    let kinds: Vec<String> = bridge
        .version_conflict_feed()
        .unwrap()
        .into_iter()
        .map(|entry| entry.kind)
        .collect();
    assert_eq!(kinds, vec!["stale_base".to_string()], "the person sees it");
    let state = server.finish();
    assert_eq!(state.init_summary(), vec![(json!(file), json!(1), 409)]);
    assert_eq!(state.latest_plaintext(file, master_key), b"edited on the web");
}

#[tokio::test]
async fn regression_1640_a_hydrate_never_writes_over_an_edit_that_is_not_queued() {
    let master_key = [169u8; 32];
    let file = "16400000-0000-4000-8000-000000000006";
    let (_dir, sync_root, server, bridge, local) = regression_1640_synced_file(master_key, file).await;
    regression_1640_web_saves(&server, master_key, file, b"edited on the web");
    std::fs::write(&local, b"edited in the sync folder").unwrap();
    let refused = bridge
        .hydrate_file(file, &local, &[sync_root.as_path()])
        .await
        .unwrap_err();
    assert!(engine_cause::<UnsentLocalChange>(&refused).is_some(), "{refused:#}");
    assert_eq!(std::fs::read(&local).unwrap(), b"edited in the sync folder");
    assert_eq!(
        bridge.db.get_file(file).unwrap().unwrap().status,
        FileStatus::Local,
        "status untouched"
    );
    drop(server.finish());
}

#[tokio::test]
async fn regression_1640_a_hydrate_still_refreshes_agreed_bytes() {
    let master_key = [170u8; 32];
    let file = "16400000-0000-4000-8000-000000000007";
    let (_dir, sync_root, server, bridge, local) = regression_1640_synced_file(master_key, file).await;
    regression_1640_web_saves(&server, master_key, file, b"edited on the web");
    sync_tick_outcome(&bridge, &sync_root).await.unwrap();
    bridge.hydrate_file(file, &local, &[sync_root.as_path()]).await.unwrap();
    assert_eq!(std::fs::read(&local).unwrap(), b"edited on the web");
    let contract = bridge.db.get_file_contract_state(file).unwrap().unwrap();
    assert_eq!(contract.local_base_version, 2);
    assert_eq!(
        contract.local_hash.as_deref(),
        Some(sha256_hex(b"edited on the web").as_str())
    );
    drop(server.finish());
}

/// A nested file keeps its folder: the landing writes the op's target path to the row, and a
/// sync-folder edit names the full relative path (a Finder modify names the file only).
#[tokio::test]
async fn regression_1640_an_edit_of_a_nested_file_lands_in_its_folder() {
    let master_key = [171u8; 32];
    let file = "16400000-0000-4000-8000-000000000008";
    let (_dir, sync_root, server, bridge) = rule3_setup(master_key);
    bridge
        .db
        .upsert_file(&FileEntry {
            file_id: "16400000-0000-4000-8000-0000000000f1".into(),
            path: "docs".into(),
            status: FileStatus::Local,
            size_bytes: 0,
            modified_at: 1_700_000_000,
            content_hash: None,
            remote_updated_at: 1_700_000_000,
            parent_id: None,
            item_kind: ItemKind::Folder,
        })
        .unwrap();
    seed_uploaded_row(&bridge, &server, file);
    let mut row = bridge.db.get_file(file).unwrap().unwrap();
    row.path = "docs/notes.txt".into();
    bridge.db.upsert_file(&row).unwrap();
    let mut contract = bridge.db.get_file_contract_state(file).unwrap().unwrap();
    contract.parent_id = Some("16400000-0000-4000-8000-0000000000f1".into());
    bridge.db.set_file_contract_state(&contract).unwrap();
    {
        let mut s = server.state.lock().unwrap();
        s.files.get_mut(file).unwrap().versions[0] = vec![regression_1640_chunk(master_key, file, b"version one")];
        s.serve_downloads = true;
    }
    let local = sync_root.join("docs").join("notes.txt");
    std::fs::create_dir_all(local.parent().unwrap()).unwrap();
    bridge.hydrate_file(file, &local, &[sync_root.as_path()]).await.unwrap();
    std::fs::write(&local, b"edited in a folder").unwrap();
    assert!(crate::watcher::dispatch_local_create(
        &bridge, &sync_root, &local, "notify"
    ));
    assert_eq!(
        bridge.db.list_operations_for_file(file).unwrap()[0]
            .target_path
            .as_deref(),
        Some("docs/notes.txt")
    );
    drain_upload_queue(&bridge, &sync_root).await;
    assert_eq!(
        bridge.db.get_file(file).unwrap().unwrap().path,
        "docs/notes.txt",
        "still in its folder"
    );
    let state = server.finish();
    assert_eq!(state.latest_plaintext(file, master_key), b"edited in a folder");
}

#[test]
fn regression_1640_sync_folder_content_decides_from_the_agreed_content() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(&dir.path().join("state.db")).unwrap();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, b"agreed").unwrap();
    db.upsert_file(&FileEntry {
        file_id: "f97d7f90-511b-52b5-9303-7201b53df85f".into(),
        path: "notes.txt".into(),
        status: FileStatus::Local,
        size_bytes: 6,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 1,
        parent_id: None,
        item_kind: ItemKind::File,
    })
    .unwrap();
    let content =
        |os_in_sync| sync_folder_content(&db, "f97d7f90-511b-52b5-9303-7201b53df85f", &path, os_in_sync).unwrap();
    assert_eq!(content(None), SyncFolderContent::Unknown, "no agreed content recorded");
    // Task 2191 (G-engine): with no agreed content recorded, the platform's "in sync" is not
    // agreement (0.8.12 marked edits in sync). It was Agreed before 2191.
    assert_eq!(
        content(Some(true)),
        SyncFolderContent::Unknown,
        "in sync, but the content is unknown"
    );
    db.record_local_baseline("f97d7f90-511b-52b5-9303-7201b53df85f", 3, &sha256_hex(b"agreed"))
        .unwrap();
    assert_eq!(
        content(Some(true)),
        SyncFolderContent::Agreed,
        "the platform says in sync"
    );
    assert_eq!(content(None), SyncFolderContent::Agreed);
    std::fs::write(&path, b"edited").unwrap();
    assert_eq!(content(None), SyncFolderContent::Changed { base: Some(3) });
    assert_eq!(content(Some(false)), SyncFolderContent::Changed { base: Some(3) });
    db.record_local_baseline("f97d7f90-511b-52b5-9303-7201b53df85f", 0, &sha256_hex(b"agreed"))
        .unwrap();
    assert_eq!(
        content(None),
        SyncFolderContent::Changed { base: None },
        "no base known"
    );
    db.enqueue_operation(&PendingOperation {
        op_id: "u".into(),
        kind: OperationKind::UploadVersion,
        file_id: Some("f97d7f90-511b-52b5-9303-7201b53df85f".into()),
        parent_id: None,
        target_path: Some("notes.txt".into()),
        metadata_json: None,
        payload_path: Some(path.to_string_lossy().into_owned()),
        base_version: Some(3),
        base_object_version_id: None,
        attempts: 0,
        max_attempts: 25,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: 0,
        updated_at: 0,
    })
    .unwrap();
    assert_eq!(
        content(Some(true)),
        SyncFolderContent::Queued,
        "a queued upload wins over in sync"
    );
}

// ── Task 1640, review P1-1: a remote deletion never deletes a change that was not sent ──

/// A synced folder `docs` holding `notes.txt`, whose local bytes were edited and never sent
/// (no agreed content recorded, nothing queued), in `db` under `sync_root`.
pub(super) fn regression_1640_edited_file_in_a_folder_in(db: &StateDb, sync_root: &Path) {
    std::fs::create_dir_all(sync_root.join("docs")).unwrap();
    let row = |file_id: &str, path: &str, item_kind: ItemKind| FileEntry {
        file_id: file_id.into(),
        path: path.into(),
        status: FileStatus::Local,
        size_bytes: 0,
        modified_at: 1_700_000_000,
        content_hash: None,
        remote_updated_at: 1_700_000_000,
        parent_id: None,
        item_kind,
    };
    db.upsert_file_with_contract(&row("folder-docs", "docs", ItemKind::Folder), |contract| {
        contract.item_kind = ItemKind::Folder;
    })
    .unwrap();
    db.upsert_file_with_contract(&row("file-notes", "docs/notes.txt", ItemKind::File), |contract| {
        contract.item_kind = ItemKind::File;
        contract.parent_id = Some("folder-docs".into());
    })
    .unwrap();
    std::fs::write(sync_root.join("docs").join("notes.txt"), b"edited, never sent").unwrap();
}

pub(super) fn regression_1640_edited_file_in_a_folder(dir: &Path) -> (StateDb, PathBuf) {
    let db = StateDb::open(&dir.join("state.db")).unwrap();
    let sync_root = dir.join("sync-root");
    regression_1640_edited_file_in_a_folder_in(&db, &sync_root);
    (db, sync_root)
}

/// The edit is kept as a copy where it can upload: the folder was removed with the trash, so
/// at the top of the sync folder. The emptied folder is gone; one review entry says where.
fn regression_1640_assert_kept_as_a_copy(db: &StateDb, sync_root: &Path) {
    assert!(
        !sync_root.join("docs").exists(),
        "the original and its emptied folder are removed"
    );
    assert_eq!(
        std::fs::read(sync_root.join("notes (this PC).txt")).unwrap(),
        b"edited, never sent",
        "the edit is kept as a copy where it can upload"
    );
    let kept = db.list_kept_copies().unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(
        (
            kept[0].original_path.as_str(),
            kept[0].copy_path.as_str(),
            kept[0].original_removed
        ),
        ("docs/notes.txt", "notes (this PC).txt", true)
    );
    assert_eq!(kept[0].copied_hash, sha256_hex(b"edited, never sent"));
    let feed: Vec<(String, String, String)> = version_conflict_feed_from_db(db)
        .unwrap()
        .into_iter()
        .map(|entry| (entry.kind, entry.file_name, entry.action))
        .collect();
    assert_eq!(
        feed,
        vec![(
            "kept_copy".to_string(),
            "notes (this PC).txt".to_string(),
            "dismiss".to_string()
        )],
        "one review entry"
    );
    assert!(
        std::fs::read_dir(sync_root).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")),
        "no temporary copy is left"
    );
}

#[test]
fn regression_1640_a_web_trash_keeps_an_unsent_edit_as_a_this_pc_copy() {
    let dir = tempfile::tempdir().unwrap();
    let (db, sync_root) = regression_1640_edited_file_in_a_folder(dir.path());
    // The web trashed the folder: the `file_trash` op's rows.
    let rows = db.mark_subtree_trashing("folder-docs").unwrap();
    assert_eq!(rows.len(), 2, "the folder and its file");
    remove_pruned_entries(&db, &sync_root, &rows, &|_, _| LocalEntry::WhollyLocal);
    regression_1640_assert_kept_as_a_copy(&db, &sync_root);
}

#[test]
fn regression_1640_a_snapshot_prune_keeps_an_unsent_edit_as_a_this_pc_copy() {
    let dir = tempfile::tempdir().unwrap();
    let (db, sync_root) = regression_1640_edited_file_in_a_folder(dir.path());
    // A snapshot that no longer lists either (it lists another file: an empty snapshot is
    // refused as degraded).
    let seen: HashSet<String> = ["a-file-the-snapshot-still-lists".to_string()].into();
    let rows = db.prune_absent(&seen, 1_800_000_000).unwrap();
    assert_eq!(rows.len(), 2, "the folder and its file");
    remove_pruned_entries(&db, &sync_root, &rows, &|_, _| LocalEntry::WhollyLocal);
    regression_1640_assert_kept_as_a_copy(&db, &sync_root);
}

#[test]
fn regression_1640_a_remote_deletion_still_removes_agreed_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let (db, sync_root) = regression_1640_edited_file_in_a_folder(dir.path());
    let rows = db.mark_subtree_trashing("folder-docs").unwrap();
    remove_pruned_entries(&db, &sync_root, &rows, &|_, _| LocalEntry::InSync);
    assert!(
        !sync_root.join("docs").exists(),
        "agreed bytes and the then-empty folder are removed"
    );
    assert!(version_conflict_feed_from_db(&db).unwrap().is_empty());
}

// ── Task 2191 S1: files of unknown content (synced before task 1640) are never freed unread ──

/// G-engine (review-114 F-P1-4): a remote deletion of a file of unknown content never takes
/// the platform's "in sync" for agreement (0.8.12 marked edits in sync): it is read by size,
/// so a wholly local one is copied first. A file with agreed content is trusted as before.
#[test]
fn task_2191_a_remote_deletion_copies_an_unknown_file_the_platform_calls_in_sync() {
    let by_trust = |_: &Path, trust: bool| {
        if trust {
            LocalEntry::InSync
        } else {
            LocalEntry::WhollyLocal
        }
    };
    let dir = tempfile::tempdir().unwrap();
    let (db, sync_root) = regression_1640_edited_file_in_a_folder(dir.path());
    let rows = db.mark_subtree_trashing("folder-docs").unwrap();
    remove_pruned_entries(&db, &sync_root, &rows, &by_trust);
    regression_1640_assert_kept_as_a_copy(&db, &sync_root);
    let known = tempfile::tempdir().unwrap();
    let (db, sync_root) = regression_1640_edited_file_in_a_folder(known.path());
    db.record_local_baseline("file-notes", 2, &sha256_hex(b"the agreed bytes"))
        .unwrap();
    let rows = db.mark_subtree_trashing("folder-docs").unwrap();
    remove_pruned_entries(&db, &sync_root, &rows, &by_trust);
    assert!(
        !sync_root.join("docs").exists(),
        "known content in sync: removed as before"
    );
    assert!(!sync_root.join("notes (this PC).txt").exists());
}

/// Desktop 0.8.15: the start-up guard now runs beside the engine, so a remote deletion can
/// reach a file of unknown content before the guard has marked it not in sync. A guarded row
/// can still carry a fetched hash (a fold of a row with no agreed version leaves one at
/// `fetched_min_version` -1), and that hash is no agreed content: the platform's in-sync
/// state is not trusted for it, and the bytes are copied first, whether the guard got there
/// or not.
#[test]
fn desktop_0815_a_remote_deletion_never_trusts_a_guarded_row_s_fetched_hash() {
    let by_trust = |_: &Path, trust: bool| {
        if trust {
            LocalEntry::InSync
        } else {
            LocalEntry::WhollyLocal
        }
    };
    let dir = tempfile::tempdir().unwrap();
    let (db, sync_root) = regression_1640_edited_file_in_a_folder(dir.path());
    db.test_execute(&format!(
            "UPDATE files SET local_hash = NULL, fetched_min_version = -1, fetched_hash = '{}' WHERE file_id = 'file-notes'",
            sha256_hex(b"some version the guard never saw")
        ))
        .unwrap();
    assert!(db.is_unknown_content("file-notes").unwrap(), "precondition: guarded");
    let rows = db.mark_subtree_trashing("folder-docs").unwrap();
    remove_pruned_entries(&db, &sync_root, &rows, &by_trust);
    regression_1640_assert_kept_as_a_copy(&db, &sync_root);
}

/// Review-114 F-P1-4: the retry of a kept original recorded with no agreed content never
/// removes it as "in sync": once it can be read, it is copied first.
#[test]
fn task_2191_the_retry_never_removes_an_unknown_file_unread() {
    let dir = tempfile::tempdir().unwrap();
    let (db, sync_root) = regression_1640_edited_file_in_a_folder(dir.path());
    let rows = db.mark_subtree_trashing("folder-docs").unwrap();
    remove_pruned_entries(&db, &sync_root, &rows, &|_, _| LocalEntry::Unreadable);
    assert!(
        sync_root.join("docs").join("notes.txt").exists(),
        "precondition: kept where it is, in use"
    );
    retry_kept_originals(&db, &sync_root, &|_, trust| {
        if trust {
            LocalEntry::InSync
        } else {
            LocalEntry::WhollyLocal
        }
    });
    assert_eq!(
        std::fs::read(sync_root.join("notes (this PC).txt")).unwrap(),
        b"edited, never sent",
        "copied before the original went"
    );
}

/// G-engine: in sync is never agreement for a file of unknown content; with agreed content
/// recorded, it is.
#[test]
fn task_2191_an_unknown_file_in_sync_is_never_agreed() {
    let dir = tempfile::tempdir().unwrap();
    let (db, sync_root) = regression_1640_edited_file_in_a_folder(dir.path());
    let path = sync_root.join("docs").join("notes.txt");
    assert_eq!(
        sync_folder_content(&db, "file-notes", &path, Some(true)).unwrap(),
        SyncFolderContent::Unknown
    );
    db.record_local_baseline("file-notes", 1, &sha256_hex(b"edited, never sent"))
        .unwrap();
    assert_eq!(
        sync_folder_content(&db, "file-notes", &path, Some(true)).unwrap(),
        SyncFolderContent::Agreed
    );
}

/// G-engine: a hydrate never writes over a file of unknown content that holds bytes; a
/// conflict is the person's to resolve and is not refused.
#[tokio::test]
async fn task_2191_a_hydrate_never_writes_over_an_unknown_file() {
    let master_key = [191u8; 32];
    let file = "21910000-0000-4000-8000-000000000001";
    let (_dir, sync_root, server, bridge, local) = regression_1640_synced_file(master_key, file).await;
    bridge
        .db
        .test_execute(&format!(
            "UPDATE files SET local_hash = NULL, fetched_min_version = -1 WHERE file_id = '{file}'"
        ))
        .unwrap();
    std::fs::write(&local, b"an edit 0.8.12 never sent").unwrap();
    let refused = bridge
        .hydrate_file(file, &local, &[sync_root.as_path()])
        .await
        .unwrap_err();
    assert!(engine_cause::<UnsentLocalChange>(&refused).is_some(), "{refused:#}");
    assert_eq!(std::fs::read(&local).unwrap(), b"an edit 0.8.12 never sent");
    bridge.db.set_status(file, FileStatus::Conflict).unwrap();
    bridge.hydrate_file(file, &local, &[sync_root.as_path()]).await.unwrap();
    drop(server.finish());
}

/// D12 (review-114 F-P1-7, R2-P2-2): a remote rename of a file records both its old and its
/// new place for the guard's retry, which checks each one's identity.
#[test]
fn task_2191_a_web_rename_records_both_places_for_the_guard() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    let master_key = [192u8; 32];
    let file_id = "21910000-0000-4000-8000-000000000002";
    let mk = beebeeb_core::kdf::MasterKey::from_bytes(master_key);
    let row = |name: &str| {
        serde_json::json!({
            "id": file_id,
            "name_encrypted": beebeeb_core::encrypt::encrypt_name(&mk, file_id, name, None).unwrap(),
            "size_bytes": 5,
            "updated_at": 100,
            "is_folder": false,
        })
    };
    let pull = |name: &str| {
        apply_metadata_file_row(
            &db,
            &row(name),
            Namespace::MyFiles,
            None,
            None,
            PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER,
            200,
            &master_key,
            "",
        )
        .unwrap()
    };
    pull("a.docx");
    assert!(
        db.list_unknown_orphans().unwrap().is_empty(),
        "no move, nothing recorded"
    );
    pull("b.docx");
    let mut recorded = db.list_unknown_orphans().unwrap();
    recorded.sort();
    assert_eq!(
        recorded,
        vec![
            ("a.docx".into(), file_id.into(), true),
            ("b.docx".into(), file_id.into(), true)
        ]
    );
}

/// Review N2: an atomic save leaves a plain file (never in sync) holding the agreed bytes. It
/// is removed like a file in sync: no false "(this PC)" duplicate.
#[test]
fn regression_1640_an_agreed_plain_file_is_removed_without_a_copy() {
    let dir = tempfile::tempdir().unwrap();
    let (db, sync_root) = regression_1640_edited_file_in_a_folder(dir.path());
    db.record_local_baseline("file-notes", 2, &sha256_hex(b"edited, never sent"))
        .unwrap();
    let rows = db.mark_subtree_trashing("folder-docs").unwrap();
    remove_pruned_entries(&db, &sync_root, &rows, &|_, _| LocalEntry::WhollyLocal);
    assert!(!sync_root.join("docs").exists(), "removed, folder and all");
    assert!(
        !sync_root.join("notes (this PC).txt").exists(),
        "no copy of the agreed bytes"
    );
    assert!(db.list_kept_copies().unwrap().is_empty());
}

/// Review N6: a file with no bytes on disk is removed without reading it (nothing to keep);
/// one with only some is left as it is (reading it would fetch the rest).
#[test]
fn regression_1640_a_file_with_no_or_only_some_local_bytes_is_not_read() {
    let dir = tempfile::tempdir().unwrap();
    let (db, sync_root) = regression_1640_edited_file_in_a_folder(dir.path());
    let rows = db.mark_subtree_trashing("folder-docs").unwrap();
    remove_pruned_entries(&db, &sync_root, &rows, &|_, _| LocalEntry::PartlyLocal);
    assert_eq!(
        std::fs::read(sync_root.join("docs").join("notes.txt")).unwrap(),
        b"edited, never sent",
        "partly on disk: kept in place"
    );
    assert!(!sync_root.join("notes (this PC).txt").exists(), "and not copied");
    let kept = db.list_kept_copies().unwrap();
    assert_eq!(
        (kept.len(), kept[0].copy_path.as_str(), kept[0].original_removed),
        (1, "", false),
        "one entry: kept in place"
    );
    // Review F4: a second remote op leaves it to the retry (no second entry); the retry
    // removes it once nothing is on disk, and its entry goes (nothing was kept).
    remove_pruned_entries(&db, &sync_root, &rows, &|_, _| LocalEntry::NothingLocal);
    assert_eq!(db.list_kept_copies().unwrap().len(), 1, "no second entry");
    retry_kept_originals(&db, &sync_root, &|_, _| LocalEntry::NothingLocal);
    assert!(
        !sync_root.join("docs").exists(),
        "nothing on disk: removed, and the emptied folder too"
    );
    assert!(db.list_kept_copies().unwrap().is_empty(), "nothing kept: no entry");
}

/// Condition 5: a file kept in place (partly on disk) is listed once, however often a remote
/// op or a tick looks at it, and is resolved once it is wholly on disk: its changed bytes are
/// kept as a copy where it can upload, and the original is removed.
#[test]
fn regression_1640_a_file_partly_on_disk_is_listed_once_and_resolved_later() {
    let dir = tempfile::tempdir().unwrap();
    let (db, sync_root) = regression_1640_edited_file_in_a_folder(dir.path());
    let rows = db.mark_subtree_trashing("folder-docs").unwrap();
    remove_pruned_entries(&db, &sync_root, &rows, &|_, _| LocalEntry::PartlyLocal);
    remove_pruned_entries(&db, &sync_root, &rows, &|_, _| LocalEntry::PartlyLocal);
    for _ in 0..3 {
        retry_kept_originals(&db, &sync_root, &|_, _| LocalEntry::PartlyLocal);
    }
    assert_eq!(db.list_kept_copies().unwrap().len(), 1, "one entry, no loop");
    let feed = version_conflict_feed_from_db(&db).unwrap();
    assert_eq!(
        (feed.len(), feed[0].status.as_str(), feed[0].action.as_str()),
        (1, "kept in place", "dismiss")
    );
    // Now wholly on disk: resolved.
    retry_kept_originals(&db, &sync_root, &|_, _| LocalEntry::WhollyLocal);
    assert!(
        !sync_root.join("docs").join("notes.txt").exists(),
        "the original is removed"
    );
    assert_eq!(
        std::fs::read(sync_root.join("notes (this PC).txt")).unwrap(),
        b"edited, never sent",
        "its bytes are kept as a copy where it can upload"
    );
    let kept = db.list_kept_copies().unwrap();
    assert!(
        kept.iter().all(|copy| copy.original_removed),
        "nothing waits any more: {kept:?}"
    );
    assert_eq!(kept.iter().filter(|copy| !copy.copy_path.is_empty()).count(), 1);
    // Review F6: one entry, updated where it is.
    let feed: Vec<(String, String)> = version_conflict_feed_from_db(&db)
        .unwrap()
        .into_iter()
        .map(|entry| (entry.status, entry.file_name))
        .collect();
    assert_eq!(
        feed,
        vec![("kept as a copy".to_string(), "notes (this PC).txt".to_string())]
    );
}

#[test]
fn regression_1640_a_remotely_deleted_folder_keeps_what_it_still_holds() {
    let dir = tempfile::tempdir().unwrap();
    let (db, sync_root) = regression_1640_edited_file_in_a_folder(dir.path());
    // A new file the upload driver has not sent yet: no row at all.
    std::fs::write(sync_root.join("docs").join("new.txt"), b"not uploaded yet").unwrap();
    let rows = db.mark_subtree_trashing("folder-docs").unwrap();
    remove_pruned_entries(&db, &sync_root, &rows, &|_, _| LocalEntry::InSync);
    assert!(
        !sync_root.join("docs").join("notes.txt").exists(),
        "the agreed file is removed"
    );
    assert_eq!(
        std::fs::read(sync_root.join("docs").join("new.txt")).unwrap(),
        b"not uploaded yet",
        "the folder stays with what it still holds"
    );
}

#[test]
fn regression_1640_a_this_pc_copy_never_overwrites_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("notes.txt");
    std::fs::write(&original, b"mine").unwrap();
    std::fs::write(dir.path().join("notes (this PC).txt"), b"an earlier copy").unwrap();
    let (copy, hash) = copy_as_this_pc(&original, dir.path()).unwrap();
    assert_eq!(copy, dir.path().join("notes (this PC 2).txt"));
    assert_eq!(
        (std::fs::read(&copy).unwrap(), hash),
        (b"mine".to_vec(), sha256_hex(b"mine"))
    );
    assert_eq!(
        std::fs::read(dir.path().join("notes (this PC).txt")).unwrap(),
        b"an earlier copy",
        "the earlier copy is untouched"
    );
    let bare = dir.path().join("Makefile");
    std::fs::write(&bare, b"all:").unwrap();
    assert_eq!(
        copy_as_this_pc(&bare, dir.path()).unwrap().0,
        dir.path().join("Makefile (this PC)")
    );
    assert!(
        std::fs::read_dir(dir.path()).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")),
        "no temporary copy is left"
    );
}

/// Review N3: a copy kept from a folder trashed on the web goes where it can upload (the
/// trashed folder would be refused as its parent) and reaches the server; emptying the trash
/// later does not take the copy with it.
#[tokio::test]
async fn regression_1640_a_kept_copy_from_a_trashed_folder_uploads_where_it_can() {
    let master_key = [180u8; 32];
    let (_dir, sync_root, server, bridge) = rule3_setup(master_key);
    regression_1640_edited_file_in_a_folder_in(&bridge.db, &sync_root);
    let rows = bridge.db.mark_subtree_trashing("folder-docs").unwrap();
    remove_pruned_entries(&bridge.db, &sync_root, &rows, &|_, _| LocalEntry::WhollyLocal);
    let copy = sync_root.join("notes (this PC).txt");
    assert!(crate::watcher::dispatch_local_create(
        &bridge, &sync_root, &copy, "scan"
    ));
    drain_upload_queue(&bridge, &sync_root).await;
    let emptied = bridge.db.delete_file_subtree("folder-docs").unwrap();
    assert!(
        emptied.iter().all(|row| row.path.starts_with("docs")),
        "emptying the trash takes only the trashed folder: {emptied:?}"
    );
    let state = server.finish();
    let created: Vec<&serde_json::Value> = state.inits.iter().map(|(body, _)| body).collect();
    assert_eq!(created.len(), 1, "one upload: the copy");
    assert!(
        created[0]["file_id"].as_str().is_some_and(is_canonical_item_id),
        "a create carries the canonical id this device chose"
    );
    assert!(
        created[0]["parent_id"].is_null(),
        "at the top, not under the trashed folder"
    );
    let server_id = state.files_with_content()[0].clone();
    assert_eq!(state.latest_plaintext(&server_id, master_key), b"edited, never sent");
}

/// Review N13: a copy made while the original could not be removed (in use) is recorded; the
/// upload driver never sends that original; a later tick removes it, and never copies the same
/// bytes twice. The entry can be dismissed once the original is gone.
#[cfg(unix)]
#[test]
fn regression_1640_an_original_in_use_is_removed_later_and_never_copied_twice() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let (db, sync_root) = regression_1640_edited_file_in_a_folder(dir.path());
    let docs = sync_root.join("docs");
    let rows = db.mark_subtree_trashing("folder-docs").unwrap();
    // The original cannot be removed: its folder refuses changes.
    std::fs::set_permissions(&docs, std::fs::Permissions::from_mode(0o555)).unwrap();
    remove_pruned_entries(&db, &sync_root, &rows, &|_, _| LocalEntry::WhollyLocal);
    let kept = db.list_kept_copies().unwrap();
    assert_eq!(
        (kept.len(), kept[0].original_removed),
        (1, false),
        "copied; the original stays for now"
    );
    assert!(
        db.is_kept_original_pending("docs/notes.txt").unwrap(),
        "the upload driver skips it"
    );
    assert!(
        !db.dismiss_kept_copy(kept[0].id).unwrap(),
        "not dismissed while its original waits"
    );
    let copies = |root: &Path| {
        std::fs::read_dir(root)
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains("(this PC")
            })
            .count()
    };
    retry_kept_originals(&db, &sync_root, &|_, _| LocalEntry::WhollyLocal);
    assert_eq!(copies(&sync_root), 1, "still in use: retried later, not copied again");
    std::fs::set_permissions(&docs, std::fs::Permissions::from_mode(0o755)).unwrap();
    retry_kept_originals(&db, &sync_root, &|_, _| LocalEntry::WhollyLocal);
    assert!(!docs.join("notes.txt").exists(), "a later tick removes the original");
    assert_eq!(copies(&sync_root), 1, "and never copies the same bytes twice");
    assert!(!db.is_kept_original_pending("docs/notes.txt").unwrap());
    assert!(db.dismiss_kept_copy(kept[0].id).unwrap(), "now it can be dismissed");
    assert!(version_conflict_feed_from_db(&db).unwrap().is_empty());
}

// ── Task 1640, review P1-2: a baseline names the downloaded version, and only whole bytes ──

/// The tick has not seen the web's version 2 yet (the row says 1): the hydrate downloads 2,
/// and its baseline says 2, from the download metadata. Before, it said the row's 1 (a later
/// edit then parked as stale for nothing).
#[tokio::test]
async fn regression_1640_a_hydrate_records_the_downloaded_version_not_the_rows() {
    let master_key = [176u8; 32];
    let file = "16400000-0000-4000-8000-000000000021";
    let (_dir, sync_root, server, bridge, local) = regression_1640_synced_file(master_key, file).await;
    regression_1640_web_saves(&server, master_key, file, b"edited on the web");
    assert_eq!(
        bridge
            .db
            .get_file_contract_state(file)
            .unwrap()
            .unwrap()
            .current_version,
        1,
        "precondition: the row has not seen version 2"
    );
    std::fs::remove_file(&local).unwrap();
    bridge.hydrate_file(file, &local, &[sync_root.as_path()]).await.unwrap();
    let contract = bridge.db.get_file_contract_state(file).unwrap().unwrap();
    assert_eq!(
        (contract.local_base_version, contract.local_hash),
        (2, Some(sha256_hex(b"edited on the web"))),
        "the baseline is the version the download named"
    );
    drop(server.finish());
}

/// Review N1b: a download whose metadata names no version (a legacy file) records the row's
/// version, read before the download: at most what the bytes are (here the web has saved 2
/// and the row still says 1), so a later change parks as stale, never lands over a newer one.
#[tokio::test]
async fn regression_1640_a_legacy_download_records_the_rows_version() {
    let master_key = [177u8; 32];
    let file = "16400000-0000-4000-8000-000000000022";
    let (_dir, sync_root, server, bridge, local) = regression_1640_synced_file(master_key, file).await;
    assert!(
        bridge
            .db
            .get_file_contract_state(file)
            .unwrap()
            .unwrap()
            .local_hash
            .is_some(),
        "precondition"
    );
    regression_1640_web_saves(&server, master_key, file, b"edited on the web");
    server.state.lock().unwrap().legacy_downloads = true;
    std::fs::remove_file(&local).unwrap();
    bridge.hydrate_file(file, &local, &[sync_root.as_path()]).await.unwrap();
    assert_eq!(std::fs::read(&local).unwrap(), b"edited on the web");
    let contract = bridge.db.get_file_contract_state(file).unwrap().unwrap();
    assert_eq!(
        (contract.local_base_version, contract.local_hash),
        (1, Some(sha256_hex(b"edited on the web"))),
        "the row's version, never more"
    );
    drop(server.finish());
}

/// Windows: a fetch records a baseline only when it delivered the whole file; a partial range
/// leaves it alone (the placeholder's other ranges may hold writes that were not sent).
#[cfg(target_os = "windows")]
#[tokio::test]
async fn regression_1640_only_a_whole_file_fetch_records_a_baseline() {
    let master_key = [178u8; 32];
    let file = "16400000-0000-4000-8000-000000000023";
    let (_dir, sync_root, server, bridge) = rule3_setup(master_key);
    seed_uploaded_row(&bridge, &server, file);
    {
        let mut s = server.state.lock().unwrap();
        s.files.get_mut(file).unwrap().versions[0] = vec![regression_1640_chunk(master_key, file, b"version one")];
        s.serve_downloads = true;
    }
    let _ = &sync_root;
    assert!(
        bridge.db.mark_no_fetched_content(file).unwrap(),
        "its placeholder created where nothing was"
    );
    let fetch = bridge.hydrate_file_to_memory(file).await.unwrap();
    let fetched = fetch.version.clone().expect("the download names its version");
    assert_eq!(
        fetched,
        FetchedVersion {
            version: 1,
            hash: sha256_hex(b"version one")
        }
    );
    assert_eq!(
        bridge.db.get_file_contract_state(file).unwrap().unwrap().local_hash,
        None,
        "review F1: no baseline before the transfer (one cut off would leave it naming other bytes)"
    );
    assert_eq!(
        bridge.db.fetched_content(file).unwrap(),
        Some((1, Some(sha256_hex(b"version one")))),
        "recorded as the candidate before the transfer"
    );
    // The whole file was transferred, and no other fetch overlapped it.
    assert!(bridge.record_whole_fetch(file, &fetched, &fetch.ticket).unwrap());
    drop(fetch);
    let contract = bridge.db.get_file_contract_state(file).unwrap().unwrap();
    assert_eq!(
        (contract.local_base_version, contract.local_hash),
        (1, Some(sha256_hex(b"version one")))
    );
    assert_eq!(bridge.db.fetched_content(file).unwrap(), None);
    drop(server.finish());
}

// ── Task 1640, review P2-5 and P3-4 ──

/// The hash of a file whose fingerprint has not changed is not read again; a change of it reads the file
/// again. The test counts the reads. Until task 2232 it showed the cache by its documented residual: a write
/// that kept the size and set the modification time back returned the earlier hash. That residual was the
/// bug (an edit read as agreed was never sent); such a write now moves the change time and is read again.
#[test]
fn regression_1640_an_unchanged_file_is_not_hashed_again() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    let reads = || HASH_READS.with(std::cell::Cell::get);
    std::fs::write(&path, b"first bytes").unwrap();
    // A hash taken right after a write is not reused: a later write in the same timer tick
    // would carry the same times.
    let start = reads();
    for _ in 0..2 {
        assert_eq!(
            cached_sha256_file_hex("cache-file", &path).unwrap(),
            sha256_hex(b"first bytes")
        );
    }
    assert_eq!(reads() - start, 2, "a fresh write is read every time");
    // A hash taken well after the file's times is reused while its fingerprint holds.
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(3600);
    assert_eq!(
        cached_sha256_file_hex_at("cache-file", &path, later).unwrap(),
        sha256_hex(b"first bytes")
    );
    let settled = reads();
    for _ in 0..3 {
        assert_eq!(
            cached_sha256_file_hex("cache-file", &path).unwrap(),
            sha256_hex(b"first bytes")
        );
    }
    assert_eq!(reads(), settled, "unchanged: the cached hash, not read again");
    regression_2229_set_modified(&path, std::time::SystemTime::now() - std::time::Duration::from_secs(60));
    assert_eq!(
        cached_sha256_file_hex("cache-file", &path).unwrap(),
        sha256_hex(b"first bytes")
    );
    assert_eq!(reads(), settled + 1, "a new modification time: read again");
}

/// Review-151d P3-6 (review P3-4 corrected): a landing that cannot hash the copy it sent records
/// no agreed content: the file's content is mixed at the version that landed, so its bytes are a
/// change on that version and are never agreed, also when they return to the previous bytes.
#[tokio::test]
async fn regression_1640_a_landing_that_cannot_hash_the_sent_copy_is_never_agreed() {
    let master_key = [179u8; 32];
    let file = "16400000-0000-4000-8000-000000000031";
    let (_dir, sync_root, server, bridge, local) = regression_1640_synced_file(master_key, file).await;
    std::fs::write(&local, b"edited in the sync folder").unwrap();
    assert!(crate::watcher::dispatch_local_create(
        &bridge, &sync_root, &local, "notify"
    ));
    let payload = bridge.db.list_operations_for_file(file).unwrap()[0]
        .payload_path
        .clone()
        .unwrap();
    bridge.seams.arm("landing:after_complete", move || {
        std::fs::remove_file(&payload).unwrap();
    });
    drain_upload_queue(&bridge, &sync_root).await;
    assert!(bridge.db.list_operations_for_file(file).unwrap().is_empty(), "landed");
    let contract = bridge.db.get_file_contract_state(file).unwrap().unwrap();
    assert_eq!(
        (contract.local_base_version, contract.local_hash),
        (2, None),
        "no agreed content"
    );
    assert_eq!(
        bridge.db.fetched_content(file).unwrap(),
        Some((2, None)),
        "mixed at the version that landed"
    );
    // Undone back to the previous bytes: still a change on version 2, never agreed (or freed).
    std::fs::write(&local, b"version one").unwrap();
    assert_eq!(
        sync_folder_content(&bridge.db, file, &local, None).unwrap(),
        SyncFolderContent::Changed { base: Some(2) }
    );
    let state = server.finish();
    assert_eq!(state.latest_plaintext(file, master_key), b"edited in the sync folder");
}

// ── Task 1640, review N1: a file brought in by range fetches gets its agreed content ──

/// The candidate: the first fetch sets it; one of the same version keeps it; one of another
/// version makes it mixed, at the LOWER version and with no hash, and mixed it stays; a
/// baseline clears it.
#[test]
fn regression_1640_a_range_fetch_candidate_merges_by_version() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(&dir.path().join("state.db")).unwrap();
    db.upsert_file(&FileEntry {
        file_id: "f97d7f90-511b-52b5-9303-7201b53df85f".into(),
        path: "notes.txt".into(),
        status: FileStatus::Local,
        size_bytes: 1,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 1,
        parent_id: None,
        item_kind: ItemKind::File,
    })
    .unwrap();
    assert_eq!(
        db.fetched_content("f97d7f90-511b-52b5-9303-7201b53df85f").unwrap(),
        None
    );
    assert!(
        db.mark_no_fetched_content("f97d7f90-511b-52b5-9303-7201b53df85f")
            .unwrap(),
        "its placeholder created where nothing was"
    );
    db.record_fetched_version("f97d7f90-511b-52b5-9303-7201b53df85f", 3, "h3")
        .unwrap();
    assert_eq!(
        db.fetched_content("f97d7f90-511b-52b5-9303-7201b53df85f").unwrap(),
        Some((3, Some("h3".into())))
    );
    db.record_fetched_version("f97d7f90-511b-52b5-9303-7201b53df85f", 3, "h3")
        .unwrap();
    assert_eq!(
        db.fetched_content("f97d7f90-511b-52b5-9303-7201b53df85f").unwrap(),
        Some((3, Some("h3".into()))),
        "same version: kept"
    );
    // Self-review #7: the same version with other bytes (a legacy download that under-claims).
    regression_1640_new_placeholder(&db, "g", "g.txt");
    db.record_fetched_version("g", 3, "h3").unwrap();
    db.record_fetched_version("g", 3, "other bytes labelled 3").unwrap();
    assert_eq!(
        db.fetched_content("g").unwrap(),
        Some((3, None)),
        "the same version, other bytes: mixed"
    );
    db.record_fetched_version("f97d7f90-511b-52b5-9303-7201b53df85f", 2, "h2")
        .unwrap();
    assert_eq!(
        db.fetched_content("f97d7f90-511b-52b5-9303-7201b53df85f").unwrap(),
        Some((2, None)),
        "another version: mixed, the lower one"
    );
    db.record_fetched_version("f97d7f90-511b-52b5-9303-7201b53df85f", 2, "h2")
        .unwrap();
    assert_eq!(
        db.fetched_content("f97d7f90-511b-52b5-9303-7201b53df85f").unwrap(),
        Some((2, None)),
        "mixed stays mixed"
    );
    db.record_local_baseline("f97d7f90-511b-52b5-9303-7201b53df85f", 4, "h4")
        .unwrap();
    assert_eq!(
        db.fetched_content("f97d7f90-511b-52b5-9303-7201b53df85f").unwrap(),
        None,
        "a write of the whole file replaces it"
    );
    db.record_fetched_version("f97d7f90-511b-52b5-9303-7201b53df85f", 5, "h5")
        .unwrap();
    assert_eq!(
        (
            db.get_file_contract_state("f97d7f90-511b-52b5-9303-7201b53df85f")
                .unwrap()
                .unwrap()
                .local_hash,
            db.fetched_content("f97d7f90-511b-52b5-9303-7201b53df85f").unwrap()
        ),
        (None, Some((4, None))),
        "a fetch on agreed bytes: they join the candidate, which is mixed at the lower version"
    );
}

/// Review F1: a row whose bytes on disk are not known (every row from before the candidate,
/// every new one until its placeholder is created where nothing was) stays not known through
/// fetches: never agreed, never sent. Only a write of the whole file makes it known.
#[test]
fn regression_1640_a_file_of_unknown_content_stays_unknown_through_fetches() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(&dir.path().join("state.db")).unwrap();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, b"part of an old version, part of version two").unwrap();
    db.upsert_file(&FileEntry {
        file_id: "f97d7f90-511b-52b5-9303-7201b53df85f".into(),
        path: "notes.txt".into(),
        status: FileStatus::Local,
        size_bytes: 1,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 1,
        parent_id: None,
        item_kind: ItemKind::File,
    })
    .unwrap();
    db.record_fetched_version("f97d7f90-511b-52b5-9303-7201b53df85f", 2, &sha256_hex(b"version two"))
        .unwrap();
    assert_eq!(
        db.fetched_content("f97d7f90-511b-52b5-9303-7201b53df85f").unwrap(),
        None,
        "no candidate over bytes nobody can name"
    );
    assert_eq!(
        sync_folder_content(&db, "f97d7f90-511b-52b5-9303-7201b53df85f", &path, None).unwrap(),
        SyncFolderContent::Unknown
    );
    assert!(
        !db.mark_no_fetched_content("f97d7f90-511b-52b5-9303-7201b53df85f")
            .unwrap(),
        "fetched into: never marked empty after the fact"
    );
    db.record_local_baseline("f97d7f90-511b-52b5-9303-7201b53df85f", 2, &sha256_hex(b"version two"))
        .unwrap();
    assert!(
        db.get_file_contract_state("f97d7f90-511b-52b5-9303-7201b53df85f")
            .unwrap()
            .unwrap()
            .local_hash
            .is_some(),
        "a whole write makes it known"
    );
}

/// Review F1 (replaces condition 1's clear): a file that goes cloud-only (dehydrated, freed,
/// evicted, or a partial placeholder read on a tick) may still hold some of its bytes; its
/// agreed content joins the candidate (never forgotten, never raised). A remote move or rename
/// changes no byte: its candidate becomes mixed (never promoted at the new place), its agreed
/// content stays.
#[tokio::test]
async fn regression_1640_dehydrating_or_moving_a_file_keeps_what_it_held() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(&dir.path().join("state.db")).unwrap();
    let local = |id: &str| FileEntry {
        file_id: id.into(),
        path: format!("{id}.txt"),
        status: FileStatus::Local,
        size_bytes: 1,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 1,
        parent_id: None,
        item_kind: ItemKind::File,
    };
    for id in ["by-status", "by-free-up"] {
        db.upsert_file(&local(id)).unwrap();
        db.record_local_baseline(id, 2, "agreed").unwrap();
    }
    db.set_status("by-status", FileStatus::CloudOnly).unwrap();
    db.mark_cloud_only_after_dehydrate(&["by-free-up".to_string()], 5)
        .unwrap();
    for id in ["by-status", "by-free-up"] {
        let contract = db.get_file_contract_state(id).unwrap().unwrap();
        assert_eq!(
            (contract.local_hash, db.fetched_content(id).unwrap()),
            (None, Some((2, Some("agreed".to_string())))),
            "{id}: the agreed content is the candidate now"
        );
    }
    // A remote rename: the metadata row names another path.
    let master_key = [181u8; 32];
    let (_dir2, sync_root, server, bridge) = rule3_setup(master_key);
    let file = "16400000-0000-4000-8000-000000000041";
    seed_uploaded_row(&bridge, &server, file);
    assert!(bridge.db.mark_no_fetched_content(file).unwrap());
    bridge.db.record_fetched_version(file, 1, "candidate").unwrap();
    let mut node = snap_node(&master_key, file, "renamed.txt", None, false, 1_700_000_500);
    node["version_number"] = json!(1);
    server
        .state
        .lock()
        .unwrap()
        .snapshots
        .push_back(("200 OK".into(), json!({ "seq_id": 1, "nodes": [node] })));
    sync_tick_outcome(&bridge, &sync_root).await.unwrap();
    assert_eq!(
        bridge.db.get_file(file).unwrap().unwrap().path,
        "renamed.txt",
        "precondition"
    );
    assert_eq!(
        bridge.db.fetched_content(file).unwrap(),
        Some((1, None)),
        "a move: mixed, its version kept"
    );
    drop(server.finish());
    db.record_local_baseline("by-status", 3, "agreed three").unwrap();
    db.poison_fetched_content("by-status").unwrap();
    assert_eq!(
        db.get_file_contract_state("by-status")
            .unwrap()
            .unwrap()
            .local_hash
            .as_deref(),
        Some("agreed three"),
        "a move keeps the agreed content"
    );
}
