//! `finder_queue_3` engine tests, split out of `engine_bridge.rs`.

use super::*;
/// Spec §7.2 (final review): a folder created inside a folder is listed in that folder
/// only, with the full relative path.
#[cfg(unix)] // names `crate::ipc_socket::list_file_provider_items`
#[test]
fn a_nested_folder_create_is_listed_in_its_folder_only() {
    let dir = tempfile::tempdir().unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    seed_folder_row(&bridge, "1ad409cd-dac8-56e2-bcd5-cef1881546f4", "r9-folder");
    let outcome = bridge
        .queue_finder_create(FinderWriteTarget {
            file_id: None,
            parent_id: Some("1ad409cd-dac8-56e2-bcd5-cef1881546f4".into()),
            filename: "sub".into(),
            rel_path: None,
            kind: FinderWriteItemKind::Folder,
            contents_path: None,
            content_type: None,
            base_version_identifier: None,
        })
        .unwrap();
    let FinderWriteOutcome::Queued { file_id: Some(sub), .. } = outcome else {
        panic!("a queued folder create: {outcome:?}")
    };
    assert_eq!(
        name_parent_status(&bridge, &sub),
        (
            "r9-folder/sub".into(),
            Some("1ad409cd-dac8-56e2-bcd5-cef1881546f4".into()),
            FileStatus::Local
        )
    );
    assert!(!listed_under(&bridge, FP_ROOT).contains(&sub), "not at the top level");
    assert_eq!(listed_under(&bridge, "1ad409cd-dac8-56e2-bcd5-cef1881546f4"), vec![sub]);
}

/// Spec §7.2 (final review): a contents-only save of a nested file never lists it at the
/// top level, and keeps its full relative path and parent through its landing. Neither
/// its accept nor its landing records a move.
#[cfg(unix)] // names `crate::ipc_socket::list_file_provider_items`
#[tokio::test]
async fn a_nested_save_is_never_listed_at_the_top_level() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync-root");
    std::fs::create_dir_all(&sync_root).unwrap();
    let master_key = [100u8; 32];
    let server = VersionedServerMock::start();
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
    seed_row_in_folder(&bridge, &server, "b40fda24-471c-5d57-8e44-d1adfc18929a");
    let moves_before = reparented_changes(&bridge, "b40fda24-471c-5d57-8e44-d1adfc18929a");
    fp_save_moved(
        &bridge,
        dir.path(),
        "b40fda24-471c-5d57-8e44-d1adfc18929a",
        "notes.txt",
        "14d2a297-b916-52d6-959d-7cc821b7a978",
        b"edited in place",
        "1",
    );
    assert_eq!(
        reparented_changes(&bridge, "b40fda24-471c-5d57-8e44-d1adfc18929a"),
        moves_before,
        "a save that neither renames nor moves records no move"
    );
    assert_eq!(
        name_parent_status(&bridge, "b40fda24-471c-5d57-8e44-d1adfc18929a"),
        (
            "Docs/notes.txt".into(),
            Some("14d2a297-b916-52d6-959d-7cc821b7a978".into()),
            FileStatus::Uploading
        )
    );
    assert!(!listed_under(&bridge, FP_ROOT).contains(&"b40fda24-471c-5d57-8e44-d1adfc18929a".to_string()));
    assert!(
        listed_under(&bridge, "14d2a297-b916-52d6-959d-7cc821b7a978")
            .contains(&"b40fda24-471c-5d57-8e44-d1adfc18929a".to_string())
    );

    drain_upload_queue(&bridge, &sync_root).await;
    assert_eq!(
        name_parent_status(&bridge, "b40fda24-471c-5d57-8e44-d1adfc18929a"),
        (
            "Docs/notes.txt".into(),
            Some("14d2a297-b916-52d6-959d-7cc821b7a978".into()),
            FileStatus::Local
        ),
        "the landing keeps the full relative path"
    );
    assert!(
        !listed_under(&bridge, FP_ROOT).contains(&"b40fda24-471c-5d57-8e44-d1adfc18929a".to_string()),
        "not at the top level once landed"
    );
    assert_eq!(
        reparented_changes(&bridge, "b40fda24-471c-5d57-8e44-d1adfc18929a"),
        moves_before,
        "nor does its landing"
    );
    drop(server.finish());
}

/// Spec §7.2 (final review): a save that renames its file while the file's create is
/// still queued renames the provisional row. The create's landing keeps that name on the
/// server file, and the save, re-keyed to it, lands under it.
#[cfg(unix)] // names `crate::ipc_socket`
#[tokio::test]
async fn a_save_that_renames_a_queued_create_keeps_the_new_name_through_the_swap() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync-root");
    std::fs::create_dir_all(&sync_root).unwrap();
    let master_key = [101u8; 32];
    let server = VersionedServerMock::start();
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
    seed_folder_row(&bridge, "14d2a297-b916-52d6-959d-7cc821b7a978", "Docs");
    // The save's first chunk fails once, so the create lands alone first.
    server
        .state
        .lock()
        .unwrap()
        .fail_first_chunk_once
        .insert("session-2".into());
    let created = fp_create_in(
        &bridge,
        dir.path(),
        "14d2a297-b916-52d6-959d-7cc821b7a978",
        "draft.txt",
        b"draft",
    );
    let p = created.outcome_file_id();
    let saved = fp_save_moved(
        &bridge,
        dir.path(),
        &p,
        "final.txt",
        "14d2a297-b916-52d6-959d-7cc821b7a978",
        b"final",
        &created.token.clone().unwrap(),
    );
    let ipc = crate::ipc_socket::write_outcome_response("modify", &bridge.db, Ok(saved));
    let crate::ipc_socket::IpcResponse::WriteQueued { item: Some(item), .. } = ipc else {
        panic!("an item, never None: {ipc:?}")
    };
    assert_eq!(
        (
            item.identifier.as_str(),
            item.filename.as_str(),
            item.parent_identifier.as_str()
        ),
        (p.as_str(), "final.txt", "14d2a297-b916-52d6-959d-7cc821b7a978")
    );
    assert_eq!(
        name_parent_status(&bridge, &p),
        (
            "Docs/final.txt".into(),
            Some("14d2a297-b916-52d6-959d-7cc821b7a978".into()),
            FileStatus::Uploading
        )
    );

    bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
    // SF-0001 (L13): the create landed under the id it chose, so P is the server id.
    let s = p.clone();
    assert!(
        bridge.db.get_file(&p).unwrap().is_some(),
        "the create lands under the id this device chose"
    );
    assert_eq!(
        name_parent_status(&bridge, &s),
        (
            "Docs/final.txt".into(),
            Some("14d2a297-b916-52d6-959d-7cc821b7a978".into()),
            FileStatus::Uploading
        ),
        "the create's landing kept the save's name"
    );

    drain_upload_queue(&bridge, &sync_root).await;
    assert_eq!(
        name_parent_status(&bridge, &s),
        (
            "Docs/final.txt".into(),
            Some("14d2a297-b916-52d6-959d-7cc821b7a978".into()),
            FileStatus::Local
        )
    );
    assert_eq!(
        listed_under(&bridge, "14d2a297-b916-52d6-959d-7cc821b7a978"),
        vec![s.clone()]
    );
    assert!(!listed_under(&bridge, FP_ROOT).contains(&s));
    let state = server.finish();
    assert_eq!(state.files_with_content(), vec![s.clone()], "one server file");
    assert_eq!(state.files[&s].versions.len(), 2);
    assert_eq!(state.latest_plaintext(&s, master_key), b"final");
}

/// Spec §7.2 (final review): a content save of an item in the trash is refused, as a create
/// in the trash is, and the item stays in the trash. Three saves: a real `Trashing` row
/// saved under the trash container (as the system holds it) and under its old folder, and
/// a live row whose template names the trash container. Each reply is an error, the row
/// keeps its status, path and parent, and no upload is queued.
#[cfg(unix)] // names `crate::ipc_socket::write_outcome_response`
#[tokio::test]
async fn a_save_of_an_item_in_the_trash_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let master_key = [102u8; 32];
    let server = VersionedServerMock::start();
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
    seed_row_in_folder(&bridge, &server, "59bb89a6-2511-5960-9c1c-8dae7e97c307");
    bridge
        .queue_finder_delete("59bb89a6-2511-5960-9c1c-8dae7e97c307", Some("1".into()))
        .unwrap();
    assert_eq!(
        bridge
            .db
            .get_file("59bb89a6-2511-5960-9c1c-8dae7e97c307")
            .unwrap()
            .unwrap()
            .status,
        FileStatus::Trashing
    );
    seed_uploaded_row(&bridge, &server, "3f073819-c91c-50d0-9651-c7580d996a29");
    let logs = capture_logs_async(async {
        let save = |file_id: &str, parent: &str| {
            let contents = dir.path().join(format!("save-{}.txt", uuid::Uuid::new_v4()));
            std::fs::write(&contents, b"edited").unwrap();
            let mut target = finder_file_target(Some(file_id), "notes.txt", &contents, Some("1".into()));
            target.parent_id = Some(parent.to_string());
            crate::ipc_socket::write_outcome_response("modify", &bridge.db, bridge.queue_file_provider_modify(target))
        };
        for (file_id, parent, status, path, row_parent) in [
            (
                "59bb89a6-2511-5960-9c1c-8dae7e97c307",
                "NSFileProviderTrashContainerItemIdentifier",
                FileStatus::Trashing,
                "Docs/notes.txt",
                Some("14d2a297-b916-52d6-959d-7cc821b7a978"),
            ),
            (
                "59bb89a6-2511-5960-9c1c-8dae7e97c307",
                "14d2a297-b916-52d6-959d-7cc821b7a978",
                FileStatus::Trashing,
                "Docs/notes.txt",
                Some("14d2a297-b916-52d6-959d-7cc821b7a978"),
            ),
            (
                "3f073819-c91c-50d0-9651-c7580d996a29",
                "NSFileProviderTrashContainerItemIdentifier",
                FileStatus::Local,
                "notes.txt",
                None,
            ),
        ] {
            let crate::ipc_socket::IpcResponse::Error { message } = save(file_id, parent) else {
                panic!("{file_id} under {parent}: refused")
            };
            assert_eq!(message, "Beebeeb cannot change items inside the Trash.", "{file_id}");
            assert_eq!(
                name_parent_status(&bridge, file_id),
                (path.into(), row_parent.map(str::to_string), status),
                "{file_id} under {parent}: unchanged"
            );
            assert!(
                bridge
                    .db
                    .list_operations_for_file(file_id)
                    .unwrap()
                    .iter()
                    .all(|op| op.kind != OperationKind::UploadVersion),
                "{file_id} under {parent}: no upload queued"
            );
        }
    })
    .await;
    // §11: each refusal is logged with its category, never a name or a path.
    let refused: Vec<&str> = logs
        .lines()
        .filter(|line| line.contains("Finder write refused"))
        .collect();
    assert_eq!(refused.len(), 3, "{logs}");
    assert!(refused.iter().all(|line| line.contains("in_trash")), "{logs}");
    assert!(!logs.contains("notes.txt") && !logs.contains("Docs"), "{logs}");
    drop(server.finish());
}

/// Spec §7.2 (final review): a folder deleted elsewhere while a create in it is queued, by
/// each of the three sweeps: a `file_delete` of the folder, its absence from a snapshot, and
/// a `file_delete` of a folder this Mac has no row for (the orphan sweep). The sweep keeps the
/// create's row and re-homes it at the top level in its transaction: no parent, its name
/// alone as the path, a `reparented` change, and the write's parent cleared. The row is
/// listed at the root from the sweep on, and the create lands there. A row with no queued
/// write is still swept.
#[cfg(unix)] // names `crate::ipc_socket::list_file_provider_items`
#[tokio::test]
async fn a_queued_create_lands_at_the_top_level_after_its_folder_is_deleted_elsewhere() {
    for (how, folder_row) in [("file_delete", true), ("snapshot", true), ("orphan sweep", false)] {
        let dir = tempfile::tempdir().unwrap();
        let sync_root = dir.path().join("sync-root");
        std::fs::create_dir_all(&sync_root).unwrap();
        let master_key = [109u8; 32];
        let server = VersionedServerMock::start();
        let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
        if folder_row {
            seed_folder_row(&bridge, "1ad409cd-dac8-56e2-bcd5-cef1881546f4", "r9-folder");
        }
        let created = fp_create_in(
            &bridge,
            dir.path(),
            "1ad409cd-dac8-56e2-bcd5-cef1881546f4",
            "n1.txt",
            b"queued",
        );
        let p = created.outcome_file_id();
        bridge
            .db
            .upsert_file_with_contract(
                &FileEntry {
                    file_id: "f02d586b-c44d-55aa-92e8-46cb8de76a0e".into(),
                    path: "r9-folder/synced.txt".into(),
                    status: FileStatus::CloudOnly,
                    size_bytes: 3,
                    modified_at: 1_700_000_000,
                    content_hash: None,
                    remote_updated_at: 1_700_000_000,
                    parent_id: Some("1ad409cd-dac8-56e2-bcd5-cef1881546f4".into()),
                    item_kind: ItemKind::File,
                },
                |contract| contract.parent_id = Some("1ad409cd-dac8-56e2-bcd5-cef1881546f4".into()),
            )
            .unwrap();
        let moves_before = reparented_changes(&bridge, &p);
        if how == "snapshot" {
            let seen: HashSet<String> = ["elsewhere".to_string()].into_iter().collect();
            bridge.db.prune_absent(&seen, now_secs() + 100).unwrap();
        } else {
            let delete = crate::api_client::SyncOp {
                seq_id: 701,
                op_type: "file_delete".into(),
                payload: serde_json::json!({ "id": "1ad409cd-dac8-56e2-bcd5-cef1881546f4" }),
            };
            apply_sync_op(&bridge, &sync_root, &delete, now_secs(), &mut Vec::new()).unwrap();
        }
        assert!(
            bridge
                .db
                .get_file("1ad409cd-dac8-56e2-bcd5-cef1881546f4")
                .unwrap()
                .is_none(),
            "{how}: no folder row"
        );
        assert!(
            bridge
                .db
                .get_file("f02d586b-c44d-55aa-92e8-46cb8de76a0e")
                .unwrap()
                .is_none(),
            "{how}: a row with no write is swept"
        );
        assert!(
            bridge.db.get_file(&p).unwrap().is_some(),
            "{how}: the queued create's row is kept"
        );
        assert_eq!(
            name_parent_status(&bridge, &p),
            ("n1.txt".into(), None, FileStatus::Uploading),
            "{how}: re-homed at the top level"
        );
        assert_eq!(
            reparented_changes(&bridge, &p),
            moves_before + 1,
            "{how}: one move recorded"
        );
        assert!(
            listed_under(&bridge, FP_ROOT).contains(&p),
            "{how}: listed at the root from the sweep on"
        );
        let ops = bridge.db.list_operations_for_file(&p).unwrap();
        assert_eq!(ops[0].parent_id, None, "{how}: the write's parent is cleared");

        drain_upload_queue(&bridge, &sync_root).await;
        // SF-0001 (L13): the create landed under the id it chose, so P is the server id.
        let s = p.clone();
        assert!(
            bridge.db.get_file(&p).unwrap().is_some(),
            "{how}: the create lands under the id this device chose"
        );
        assert_eq!(
            name_parent_status(&bridge, &s),
            ("n1.txt".into(), None, FileStatus::Local),
            "{how}: landed at the top level"
        );
        assert!(listed_under(&bridge, FP_ROOT).contains(&s), "{how}: listed at the root");
        let state = server.finish();
        assert_eq!(state.latest_plaintext(&s, master_key), b"queued");
        let init = &state.inits[0].0;
        assert!(
            init.get("parent_id").is_none_or(serde_json::Value::is_null),
            "{how}: the create's init names no folder: {init}"
        );
    }
}

/// Spec §7.2 (final review): a sweep re-homes only a write whose own folder is gone. A folder
/// in the trash and a live folder with the same path; a save queued in the live one; the
/// trashed folder purged (`file_delete`). The prefix `Docs/` reaches the save's row, but its
/// parent is still there: the row keeps its parent and path, and its PATCH has no
/// `parent_id`.
#[tokio::test]
async fn a_purged_folder_never_re_homes_a_write_in_a_live_folder_with_the_same_path() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync-root");
    std::fs::create_dir_all(&sync_root).unwrap();
    let master_key = [112u8; 32];
    let server = VersionedServerMock::start();
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), master_key);
    seed_folder_row(&bridge, "33952615-c708-522b-a118-81a30db4f320", "Docs");
    bridge
        .db
        .set_status("33952615-c708-522b-a118-81a30db4f320", FileStatus::Trashing)
        .unwrap();
    seed_folder_row(&bridge, "b224bc21-8560-54f0-a515-aac77a7ec6b7", "Docs");
    server.seed_file("4cc1d957-b7e5-5ca1-b8e0-f7b479568a72", 1);
    bridge
        .db
        .upsert_file_with_contract(
            &FileEntry {
                file_id: "4cc1d957-b7e5-5ca1-b8e0-f7b479568a72".into(),
                path: "Docs/report.txt".into(),
                status: FileStatus::Local,
                size_bytes: 10,
                modified_at: 1_700_000_000,
                content_hash: None,
                remote_updated_at: 1_700_000_000,
                parent_id: Some("b224bc21-8560-54f0-a515-aac77a7ec6b7".into()),
                item_kind: ItemKind::File,
            },
            |contract| {
                contract.parent_id = Some("b224bc21-8560-54f0-a515-aac77a7ec6b7".into());
                contract.current_version = 1;
            },
        )
        .unwrap();
    fp_save_moved(
        &bridge,
        dir.path(),
        "4cc1d957-b7e5-5ca1-b8e0-f7b479568a72",
        "report.txt",
        "b224bc21-8560-54f0-a515-aac77a7ec6b7",
        b"saved in the live folder",
        "1",
    );
    let purge = crate::api_client::SyncOp {
        seq_id: 704,
        op_type: "file_delete".into(),
        payload: serde_json::json!({ "id": "33952615-c708-522b-a118-81a30db4f320" }),
    };
    apply_sync_op(&bridge, &sync_root, &purge, now_secs(), &mut Vec::new()).unwrap();
    assert!(
        bridge
            .db
            .get_file("33952615-c708-522b-a118-81a30db4f320")
            .unwrap()
            .is_none(),
        "the trashed folder is purged"
    );
    assert_eq!(
        name_parent_status(&bridge, "4cc1d957-b7e5-5ca1-b8e0-f7b479568a72"),
        (
            "Docs/report.txt".into(),
            Some("b224bc21-8560-54f0-a515-aac77a7ec6b7".into()),
            FileStatus::Uploading
        ),
        "its folder is still there: the row is left exactly as it is"
    );

    drain_upload_queue(&bridge, &sync_root).await;
    let state = server.finish();
    assert_eq!(
        state.latest_plaintext("4cc1d957-b7e5-5ca1-b8e0-f7b479568a72", master_key),
        b"saved in the live folder"
    );
    assert_eq!(
        patched_parents(&state, "4cc1d957-b7e5-5ca1-b8e0-f7b479568a72"),
        vec![None],
        "the save does not move the server file"
    );
}

/// A14b / L15 (option A): an owner write into a shared folder is never
/// published. Its queue reply carries the exact owner-only copy, and no
/// pending key envelope is written for the new item.
#[tokio::test]
async fn a14b_owner_write_into_a_shared_folder_stays_owner_only() {
    let dir = tempfile::tempdir().unwrap();
    let server = VersionedServerMock::start();
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), server.base_url.clone(), [7u8; 32]);
    let sync_root = dir.path().join("sync");
    std::fs::create_dir_all(&sync_root).unwrap();
    seed_folder_row(&bridge, "40000000-0000-4000-8000-0000000000a1", "r14-plain");
    seed_folder_row(&bridge, "40000000-0000-4000-8000-0000000000a2", "r14-shared");
    bridge
        .db()
        .remember_folder_custody_at_revision("40000000-0000-4000-8000-0000000000a2", 0)
        .unwrap();
    let target = |parent: &str, name: &str| FinderWriteTarget {
        file_id: None,
        parent_id: Some(parent.into()),
        filename: name.into(),
        rel_path: None,
        kind: FinderWriteItemKind::Folder,
        contents_path: None,
        content_type: None,
        base_version_identifier: None,
    };
    let FinderWriteOutcome::Queued {
        file_id: Some(plain),
        message: plain_message,
        ..
    } = bridge
        .queue_finder_create(target("40000000-0000-4000-8000-0000000000a1", "plain"))
        .unwrap()
    else {
        panic!("a queued folder create")
    };
    assert_eq!(plain_message, "queued for encrypted sync");
    let FinderWriteOutcome::Queued {
        file_id: Some(shared),
        message,
        ..
    } = bridge
        .queue_finder_create(target("40000000-0000-4000-8000-0000000000a2", "owner-only"))
        .unwrap()
    else {
        panic!("a queued folder create")
    };
    assert_eq!(message, crate::owner_keys::OWNER_ONLY_NOTICE);
    for id in [&plain, &shared] {
        let record = bridge.db().get_owned_key_record(id).unwrap();
        assert!(
            record.and_then(|record| record.pending_owner_wrap).is_none(),
            "an owner write is never wrapped for a share: {id}"
        );
    }
    // A14b: run the queue against a watching mock. The creates reach the
    // server, and neither the plain nor the shared folder posts a key.
    // The mock has no create route, so the op may retry; the request itself is
    // the evidence that the write ran and published nothing.
    bridge.process_due_operations(&sync_root, i64::MAX / 2).await.unwrap();
    let state = server.finish();
    assert!(
        state
            .requests
            .iter()
            .any(|(method, path)| method == "POST" && path == "/api/v1/files/folder"),
        "the creates reached the server: {:?}",
        state.requests
    );
    for (method, path) in &state.requests {
        assert!(
            !(matches!(method.as_str(), "POST" | "PUT") && path.starts_with("/api/v1/shared-folders/")),
            "an owner write publishes no key: {method} {path}"
        );
        assert!(
            !path.contains("/owner-key"),
            "an owner write adopts nothing: {method} {path}"
        );
    }
}

/// P2-5 / L15: the owner-only notice belongs to a create. A rename or move of
/// an existing item (even inside a shared folder) says only "queued".
#[test]
fn review_p2_5_only_a_create_shows_the_owner_only_notice() {
    let dir = tempfile::tempdir().unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    let root = "40000000-0000-4000-8000-0000000000a4";
    let item = "40000000-0000-4000-8000-0000000000a5";
    seed_folder_row(&bridge, root, "shared");
    seed_bridge_row(&bridge, item, "shared/item.txt", Some(root), FileStatus::Local, 0);
    bridge.db().remember_folder_custody_at_revision(root, 0).unwrap();

    let target = |file_id: Option<&str>, name: &str, kind| FinderWriteTarget {
        file_id: file_id.map(str::to_string),
        parent_id: Some(root.into()),
        filename: name.into(),
        rel_path: None,
        kind,
        contents_path: None,
        content_type: None,
        base_version_identifier: None,
    };
    let FinderWriteOutcome::Queued { message, .. } = bridge
        .queue_local_rename(
            target(Some(item), "renamed.txt", FinderWriteItemKind::File),
            "shared/renamed.txt",
        )
        .unwrap()
    else {
        panic!("a queued rename")
    };
    assert_eq!(
        message, "queued for encrypted sync",
        "a rename of an existing item is not a create and never claims the notice"
    );

    // A move of an existing item out of the shared folder: same rule.
    let private = "40000000-0000-4000-8000-0000000000a6";
    let moved = "40000000-0000-4000-8000-0000000000a7";
    seed_folder_row(&bridge, private, "private");
    seed_bridge_row(&bridge, moved, "shared/other.txt", Some(root), FileStatus::Local, 0);
    let mut move_target = target(Some(moved), "other.txt", FinderWriteItemKind::File);
    move_target.parent_id = Some(private.into());
    let FinderWriteOutcome::Queued { message, .. } =
        bridge.queue_local_rename(move_target, "private/other.txt").unwrap()
    else {
        panic!("a queued move")
    };
    assert_eq!(
        message, "queued for encrypted sync",
        "a move of an existing item never claims the notice"
    );

    // A new version of an existing item: the version path, not a create.
    let versioned = "40000000-0000-4000-8000-0000000000a8";
    seed_bridge_row(
        &bridge,
        versioned,
        "shared/versioned.txt",
        Some(root),
        FileStatus::Local,
        0,
    );
    let payload_path = dir.path().join("version.txt");
    std::fs::write(&payload_path, b"new bytes").unwrap();
    let mut version_target = target(Some(versioned), "versioned.txt", FinderWriteItemKind::File);
    version_target.contents_path = Some(payload_path.to_string_lossy().into_owned());
    let written = bridge.queue_file_provider_modify_from(version_target, None).unwrap();
    match written.outcome {
        FinderWriteOutcome::Queued { message, .. } => assert_eq!(
            message, "queued for encrypted sync",
            "a new version of an existing item never claims the notice"
        ),
        _ => panic!("a queued version"),
    }

    let FinderWriteOutcome::Queued { message, .. } = bridge
        .queue_finder_create(target(None, "fresh", FinderWriteItemKind::Folder))
        .unwrap()
    else {
        panic!("a queued create")
    };
    assert_eq!(
        message,
        crate::owner_keys::OWNER_ONLY_NOTICE,
        "a self-minted create in a shared folder still says it stays owner-only"
    );
}
