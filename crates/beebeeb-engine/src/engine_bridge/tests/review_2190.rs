//! `review_2190` engine tests, split out of `engine_bridge.rs`.

use super::*;
/// Row 1: the bytes are the agreed content, so the tick re-anchors the row to the server's
/// version as before, and the base stays the agreed one.
#[tokio::test]
async fn task_2190_fu1_agreed_bytes_re_anchor_to_the_server() {
    let master_key = [222u8; 32];
    let file = "21900000-0000-4000-8000-000000000012";
    let (_dir, _sync_root, _server, bridge, _before, tick, _) = task_2190_fu1_tick_after(master_key, file, None).await;
    assert!(tick.conflicts.is_empty());
    let row = bridge.db.get_file(file).unwrap().unwrap();
    let contract = bridge.db.get_file_contract_state(file).unwrap().unwrap();
    assert_eq!(
        row.size_bytes,
        b"edited on the web".len() as i64,
        "re-anchored to the server's size"
    );
    assert_eq!(
        row.remote_updated_at, 1_700_000_500,
        "re-anchored to the server's stamp"
    );
    assert_eq!((contract.current_version, contract.local_base_version), (2, 1));
}

/// E3: a `Local` row whose base is unknown keeps it unknown when the pull sees it; the
/// server's version is never written over it as the base of the bytes on disk.
#[tokio::test]
async fn task_2190_fu1_a_local_row_with_an_unknown_base_keeps_it_unknown() {
    let master_key = [223u8; 32];
    let file = "21900000-0000-4000-8000-000000000013";
    let (_dir, sync_root, server, bridge) = rule3_setup(master_key);
    seed_uploaded_row(&bridge, &server, file);
    bridge
        .db
        .test_execute(&format!(
            "UPDATE files SET local_base_version = 0 WHERE file_id = '{file}'"
        ))
        .unwrap();
    task_2190_web_saves(&server, master_key, file, b"edited on the web");
    sync_tick_outcome(&bridge, &sync_root).await.unwrap();
    let contract = bridge.db.get_file_contract_state(file).unwrap().unwrap();
    assert_eq!(contract.current_version, 2);
    assert_eq!(
        contract.local_base_version, 0,
        "the base stays unknown, never the server's version"
    );
}

/// The macOS pin's steps 1–4: a Mac save on version 1, the web's version 2, the tick, and the
/// upload parked as stale. Returns the parked op, its staged copy and the Mac's token.
#[cfg(target_os = "macos")]
async fn task_2190_parked(
    master_key: [u8; 32],
    file: &str,
) -> (
    tempfile::TempDir,
    PathBuf,
    VersionedServerMock,
    EngineBridge,
    String,
    String,
    String,
) {
    let (dir, sync_root, server, bridge) = rule3_setup(master_key);
    seed_uploaded_row(&bridge, &server, file);
    let mac_save = fp_save(&bridge, dir.path(), file, "notes.txt", b"edited on this Mac", "1");
    let token = mac_save.token.clone().expect("a Finder save names its bytes");
    task_2190_web_saves(&server, master_key, file, b"edited on the web");
    sync_tick_outcome(&bridge, &sync_root).await.unwrap();
    bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
    let parked = bridge.db.list_operations_for_file(file).unwrap().remove(0);
    assert_eq!(
        parked.last_error.as_deref(),
        Some("stale_base"),
        "precondition: parked as stale"
    );
    assert!(
        bridge.db.is_decidable_parked(&parked.op_id).unwrap(),
        "precondition: decidable"
    );
    let staged = parked.payload_path.clone().unwrap();
    (dir, sync_root, server, bridge, parked.op_id, staged, token)
}

/// A `keep_copy` for the tests: the bytes go to `<root>/kept/<row path>`.
#[cfg(target_os = "macos")]
fn task_2190_keep_into(root: &Path) -> impl FnMut(&Path, &str) -> anyhow::Result<PathBuf> + '_ {
    move |source, rel| {
        let dest = root.join("kept").join(rel);
        std::fs::create_dir_all(dest.parent().unwrap())?;
        std::fs::copy(source, &dest)?;
        Ok(dest)
    }
}

/// "Keep this Mac's": the same op is rebased on version 2 and sent again; the server's new
/// current version is the Mac's bytes, and the web's stays in its history.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn task_2190_fu2_keep_this_macs_sends_the_write_on_the_servers_version() {
    let master_key = [224u8; 32];
    let file = "21900000-0000-4000-8000-000000000021";
    let (dir, sync_root, server, bridge, op_id, _staged, _token) = task_2190_parked(master_key, file).await;
    let mut keep = task_2190_keep_into(dir.path());
    let done = bridge
        .resolve_parked(&op_id, ParkedChoice::Local, Some(2), &mut keep, "Mac", "10.00")
        .unwrap();
    assert_eq!(done, ParkedResolution::Rebased { version: 2 });
    let op = bridge.db.get_operation(&op_id).unwrap().unwrap();
    assert_eq!(
        (op.base_version, op.attempts, op.last_error.as_deref()),
        (Some(2), 0, None)
    );
    assert_eq!(bridge.db.get_file(file).unwrap().unwrap().status, FileStatus::Uploading);
    bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
    assert!(
        bridge.db.list_operations_for_file(file).unwrap().is_empty(),
        "sent and landed"
    );
    drop(bridge);
    let state = server.finish();
    assert_eq!(
        state.init_summary(),
        vec![(json!(file), json!(1), 409), (json!(file), json!(2), 201)]
    );
    assert_eq!(
        state.files[file].versions.len(),
        3,
        "the web's version stays in the history"
    );
    assert_eq!(state.latest_plaintext(file, master_key), b"edited on this Mac");
}

/// "Keep the other" (D1): the Mac's bytes are copied first, then the op goes, the token is
/// cleared (the system reads version 2), and the staged copy is released after the commit.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn task_2190_fu2_keep_the_other_keeps_a_copy_first_then_reads_the_servers_version() {
    let master_key = [225u8; 32];
    let file = "21900000-0000-4000-8000-000000000022";
    let (dir, _sync_root, server, bridge, op_id, staged, token) = task_2190_parked(master_key, file).await;
    let mut keep = task_2190_keep_into(dir.path());
    let done = bridge
        .resolve_parked(&op_id, ParkedChoice::Remote, Some(2), &mut keep, "Mac", "10.00")
        .unwrap();
    let ParkedResolution::KeptOther { kept_copy } = done else {
        panic!("not kept: {done:?}")
    };
    assert_eq!(
        std::fs::read(&kept_copy).unwrap(),
        b"edited on this Mac",
        "the Mac's edit is kept"
    );
    assert!(
        bridge.db.get_operation(&op_id).unwrap().is_none(),
        "the parked op is gone"
    );
    assert!(!Path::new(&staged).exists(), "the staged copy is released");
    // Read before any presentation: the payload builder clears a held token whose write is gone
    // (§5.3 housekeeping), so only this order shows that the decision left none behind itself.
    assert!(
        bridge.db.item_presentation(file).unwrap().unwrap().held.is_none(),
        "the decision clears the held token's columns itself"
    );
    assert_ne!(
        held_content_version(&bridge, file),
        token,
        "Finder is shown the server's version"
    );
    assert_eq!(bridge.db.get_file(file).unwrap().unwrap().status, FileStatus::Local);
    drop(bridge);
    let state = server.finish();
    assert_eq!(state.files[file].versions.len(), 2);
    assert_eq!(state.latest_plaintext(file, master_key), b"edited on the web");
}

/// D1: when the copy cannot be made, "Keep the other" fails and changes nothing.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn task_2190_fu2_keep_the_other_changes_nothing_when_the_copy_fails() {
    let master_key = [226u8; 32];
    let file = "21900000-0000-4000-8000-000000000023";
    let (_dir, _sync_root, _server, bridge, op_id, staged, token) = task_2190_parked(master_key, file).await;
    let mut failing = |_: &Path, _: &str| -> anyhow::Result<PathBuf> { anyhow::bail!("the disk is full") };
    assert!(
        bridge
            .resolve_parked(&op_id, ParkedChoice::Remote, Some(2), &mut failing, "Mac", "10.00")
            .is_err()
    );
    assert!(
        bridge.db.is_decidable_parked(&op_id).unwrap(),
        "still parked and decidable"
    );
    assert_eq!(
        std::fs::read(&staged).unwrap(),
        b"edited on this Mac",
        "the staged copy is kept"
    );
    assert_eq!(held_content_version(&bridge, file), token, "the token is kept");
}

/// "Keep both" (D4): the Mac's bytes are queued as a new file with a unique name beside the
/// original, and the original reads the server's version; both land on the server.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn task_2190_fu2_keep_both_queues_the_macs_bytes_as_a_new_file_with_a_unique_name() {
    let master_key = [227u8; 32];
    let file = "21900000-0000-4000-8000-000000000024";
    let (dir, sync_root, server, bridge, op_id, _staged, token) = task_2190_parked(master_key, file).await;
    let mut keep = task_2190_keep_into(dir.path());
    let done = bridge
        .resolve_parked(&op_id, ParkedChoice::Both, Some(2), &mut keep, "Mac", "10.00")
        .unwrap();
    let ParkedResolution::KeptBoth { file_id: new_id, name } = done else {
        panic!("not both: {done:?}")
    };
    assert_eq!(name, "notes (Mac, 10.00).txt");
    assert_eq!(
        bridge.db.get_file(&new_id).unwrap().unwrap().path,
        "notes (Mac, 10.00).txt"
    );
    assert!(
        bridge.db.get_operation(&op_id).unwrap().is_none(),
        "the parked op is gone"
    );
    assert!(
        bridge.db.item_presentation(file).unwrap().unwrap().held.is_none(),
        "the decision clears the held token's columns itself (read before any presentation)"
    );
    assert_ne!(
        held_content_version(&bridge, file),
        token,
        "the original reads the server's version"
    );
    assert!(!dir.path().join("kept").exists(), "keep both makes no kept copy");
    bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
    drop(bridge);
    let state = server.finish();
    assert_eq!(state.latest_plaintext(file, master_key), b"edited on the web");
    let others: Vec<Vec<u8>> = state
        .files
        .keys()
        .filter(|id| id.as_str() != file)
        .map(|id| state.latest_plaintext(id, master_key))
        .collect();
    assert_eq!(
        others,
        vec![b"edited on this Mac".to_vec()],
        "the Mac's bytes are one new server file"
    );
}

/// D2: a newer save of the file queued behind the park makes every choice `Moved`, with
/// nothing changed; the feed then offers no decision for the older park.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn task_2190_fu2_a_newer_save_behind_the_park_makes_every_choice_moved() {
    let master_key = [228u8; 32];
    let file = "21900000-0000-4000-8000-000000000025";
    let (dir, _sync_root, _server, bridge, op_id, staged, _token) = task_2190_parked(master_key, file).await;
    fp_save(&bridge, dir.path(), file, "notes.txt", b"saved again on this Mac", "1");
    let mut keep = task_2190_keep_into(dir.path());
    for choice in [ParkedChoice::Local, ParkedChoice::Remote, ParkedChoice::Both] {
        assert_eq!(
            bridge
                .resolve_parked(&op_id, choice, Some(2), &mut keep, "Mac", "10.00")
                .unwrap(),
            ParkedResolution::Moved,
            "{choice:?}"
        );
    }
    assert!(Path::new(&staged).exists(), "nothing released");
    assert!(!dir.path().join("kept").exists(), "nothing copied");
    assert_eq!(
        bridge.db.list_operations_for_file(file).unwrap().len(),
        2,
        "both saves still queued"
    );
    let parked = bridge
        .version_conflict_feed()
        .unwrap()
        .into_iter()
        .find(|entry| entry.op_id.as_deref() == Some(op_id.as_str()));
    assert_ne!(parked.map(|entry| entry.action), Some("open_conflict".to_string()));
}

/// Review-169 #1: the person decides against the version the window showed. The file past it (a
/// newer server version the pull recorded since) makes every choice `Moved`, with nothing changed;
/// at the version shown, the choice acts and Keep Mine rebases on exactly it.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn task_2190_fu2_a_choice_on_a_version_the_file_moved_past_is_moved() {
    let master_key = [235u8; 32];
    let file = "21900000-0000-4000-8000-000000000051";
    let (dir, _sync_root, _server, bridge, op_id, staged, _token) = task_2190_parked(master_key, file).await;
    bridge
        .db
        .test_execute(&format!(
            "UPDATE files SET current_version = 3 WHERE file_id = '{file}'"
        ))
        .unwrap();
    let mut keep = task_2190_keep_into(dir.path());
    for choice in [ParkedChoice::Local, ParkedChoice::Remote, ParkedChoice::Both] {
        assert_eq!(
            bridge
                .resolve_parked(&op_id, choice, Some(2), &mut keep, "Mac", "10.00")
                .unwrap(),
            ParkedResolution::Moved,
            "{choice:?} on version 2 while the file is at 3"
        );
        assert_eq!(
            bridge
                .resolve_parked(&op_id, choice, None, &mut keep, "Mac", "10.00")
                .unwrap(),
            ParkedResolution::Moved,
            "{choice:?} with no version shown"
        );
    }
    assert!(
        Path::new(&staged).exists() && !dir.path().join("kept").exists(),
        "nothing changed"
    );
    assert_eq!(
        bridge
            .resolve_parked(&op_id, ParkedChoice::Local, Some(3), &mut keep, "Mac", "10.00")
            .unwrap(),
        ParkedResolution::Rebased { version: 3 },
        "at the version shown, Keep Mine rebases on exactly it"
    );
}

/// Review-169 #2 (D2): a trash, move or rename of the file queued behind the parked write makes
/// every choice `Moved` too, not only a newer save.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn task_2190_fu2_a_trash_or_rename_behind_the_park_makes_every_choice_moved() {
    for kind in [
        OperationKind::TrashFile,
        OperationKind::RenameFile,
        OperationKind::MoveFile,
    ] {
        let master_key = [236u8; 32];
        let file = "21900000-0000-4000-8000-000000000052";
        let (dir, _sync_root, _server, bridge, op_id, _staged, _token) = task_2190_parked(master_key, file).await;
        bridge
            .db
            .enqueue_operation(&PendingOperation {
                op_id: format!("behind-{kind:?}"),
                kind: kind.clone(),
                file_id: Some(file.to_string()),
                parent_id: None,
                target_path: Some("notes renamed.txt".to_string()),
                metadata_json: None,
                payload_path: None,
                base_version: None,
                base_object_version_id: None,
                attempts: 0,
                max_attempts: 25,
                next_retry_at: 0,
                last_error: None,
                backup_source_key: None,
                created_at: now_secs(),
                updated_at: now_secs(),
            })
            .unwrap();
        let mut keep = task_2190_keep_into(dir.path());
        for choice in [ParkedChoice::Local, ParkedChoice::Remote, ParkedChoice::Both] {
            assert_eq!(
                bridge
                    .resolve_parked(&op_id, choice, Some(2), &mut keep, "Mac", "10.00")
                    .unwrap(),
                ParkedResolution::Moved,
                "{choice:?} with a {kind:?} behind"
            );
        }
    }
}

/// Review-169 #3: "Keep both" on a write that moved queues nothing: no new row, no create, and the
/// copy staged for it removed. Decided again at the right version, it gives exactly one copy,
/// without " 2" (nothing was left behind by the first try).
#[cfg(target_os = "macos")]
#[tokio::test]
async fn task_2190_fu2_keep_both_on_a_moved_write_queues_nothing() {
    let master_key = [237u8; 32];
    let file = "21900000-0000-4000-8000-000000000053";
    let (dir, _sync_root, _server, bridge, op_id, _staged, _token) = task_2190_parked(master_key, file).await;
    let rows = |bridge: &EngineBridge| bridge.db.child_names_lowercase(None).unwrap().len();
    let unfinished = |bridge: &EngineBridge| {
        bridge
            .db
            .staged_payloads_for_signout()
            .unwrap()
            .into_iter()
            .filter(|(_, _, completed)| !completed)
            .count()
    };
    let (rows_before, unfinished_before) = (rows(&bridge), unfinished(&bridge));
    let mut keep = task_2190_keep_into(dir.path());
    assert_eq!(
        bridge
            .resolve_parked(&op_id, ParkedChoice::Both, Some(1), &mut keep, "Mac", "10.00")
            .unwrap(),
        ParkedResolution::Moved
    );
    assert_eq!(rows(&bridge), rows_before, "no new row");
    assert_eq!(
        unfinished(&bridge),
        unfinished_before,
        "no staged copy left for a create"
    );
    assert_eq!(
        bridge.db.list_operations_for_file(file).unwrap().len(),
        1,
        "only the parked write"
    );
    let done = bridge
        .resolve_parked(&op_id, ParkedChoice::Both, Some(2), &mut keep, "Mac", "10.00")
        .unwrap();
    assert_eq!(
        done.clone(),
        match done {
            ParkedResolution::KeptBoth { ref file_id, .. } => {
                ParkedResolution::KeptBoth {
                    file_id: file_id.clone(),
                    name: "notes (Mac, 10.00).txt".to_string(),
                }
            }
            other => other,
        },
        "one copy, with the plain name"
    );
    assert_eq!(rows(&bridge), rows_before + 1, "exactly one new row");
}

/// Review-169 #3, the race: the parked write moves between the decision's first check and the
/// copy's accept (its seam). The accept's own check, in its transaction, refuses it: nothing is
/// queued and no staged copy is left.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn task_2190_fu2_keep_both_racing_a_move_queues_nothing() {
    let master_key = [240u8; 32];
    let file = "21900000-0000-4000-8000-000000000056";
    let (dir, _sync_root, _server, bridge, op_id, _staged, _token) = task_2190_parked(master_key, file).await;
    let rows = bridge.db.child_names_lowercase(None).unwrap().len();
    let unfinished = |bridge: &EngineBridge| {
        bridge
            .db
            .staged_payloads_for_signout()
            .unwrap()
            .into_iter()
            .filter(|(_, _, completed)| !completed)
            .count()
    };
    let unfinished_before = unfinished(&bridge);
    let db = std::sync::Arc::clone(&bridge.db);
    let moved = file.to_string();
    bridge.seams.arm("accept:before_tx", move || {
        db.test_execute(&format!(
            "UPDATE files SET current_version = 3 WHERE file_id = '{moved}'"
        ))
        .unwrap();
    });
    let mut keep = task_2190_keep_into(dir.path());
    assert_eq!(
        bridge
            .resolve_parked(&op_id, ParkedChoice::Both, Some(2), &mut keep, "Mac", "10.00")
            .unwrap(),
        ParkedResolution::Moved,
        "the accept refused the moved write"
    );
    assert_eq!(bridge.db.child_names_lowercase(None).unwrap().len(), rows, "no new row");
    assert_eq!(unfinished(&bridge), unfinished_before, "no staged copy left");
    assert_eq!(
        bridge.db.list_operations_for_file(file).unwrap().len(),
        1,
        "only the parked write"
    );
}

/// Review-169 #4: "Keep the other" releases the parked write's staged copy in the journal in its
/// own transaction: right after the drop (before any unlink) the journal row is released, so a
/// crash there leaves no unfinished row to block a sign-out; the engine-start sweep unlinks it.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn task_2190_fu2_dropping_a_parked_write_releases_its_copy_in_the_same_transaction() {
    let master_key = [238u8; 32];
    let file = "21900000-0000-4000-8000-000000000054";
    let (_dir, _sync_root, _server, bridge, op_id, staged, _token) = task_2190_parked(master_key, file).await;
    let journal = |bridge: &EngineBridge| {
        bridge
            .db
            .staged_payloads_for_signout()
            .unwrap()
            .into_iter()
            .find(|(path, _, _)| path == &staged)
            .map(|(_, _, completed)| completed)
    };
    assert_eq!(
        journal(&bridge),
        Some(false),
        "precondition: the parked write's copy is unfinished"
    );
    assert_eq!(bridge.db.drop_parked(&op_id, 2).unwrap(), Some(Some(staged.clone())));
    assert_eq!(journal(&bridge), Some(true), "released in the drop's transaction");
    assert!(Path::new(&staged).exists(), "the unlink comes after the commit");
}

/// Review-169 #5: one review entry that cannot be read is left out; the rest of the feed shows.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn task_2190_fu2_one_unreadable_review_entry_leaves_the_rest_of_the_feed() {
    let master_key = [239u8; 32];
    let file = "21900000-0000-4000-8000-000000000055";
    let (_dir, _sync_root, _server, bridge, op_id, _staged, _token) = task_2190_parked(master_key, file).await;
    FAIL_DECIDABLE_READ.with(|fail| *fail.borrow_mut() = Some(op_id.clone()));
    let feed = bridge.version_conflict_feed();
    FAIL_DECIDABLE_READ.with(|fail| *fail.borrow_mut() = None);
    let feed = feed.expect("the feed still answers");
    let entry = feed
        .iter()
        .find(|entry| entry.op_id.as_deref() == Some(op_id.as_str()))
        .expect("the row stays");
    assert_eq!(entry.action, "review_upload", "a failed check offers no decision");
}

/// Review-169 F1: the window decides against the version of the bytes it showed, from the metadata
/// that produced them, never the mirror read after the download. Another device saves version 3
/// while the preview downloads version 2, and the pull raises the mirror to 3 (the seam, between
/// the metadata and the chunks): the preview still names 2, so every choice made on it is `Moved`.
/// A legacy file (no `download_version`) names the mirror as it was before the fetch, which is
/// never newer than the bytes.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn task_2190_fu2_the_preview_names_the_version_of_the_bytes_it_shows() {
    for legacy in [false, true] {
        let master_key = [241u8; 32];
        let file = "21900000-0000-4000-8000-000000000057";
        let (dir, _sync_root, server, bridge, op_id, staged, _token) = task_2190_parked(master_key, file).await;
        {
            let mut s = server.state.lock().unwrap();
            s.serve_downloads = true;
            s.legacy_downloads = legacy;
        }
        let (db, state, saved) = (
            std::sync::Arc::clone(&bridge.db),
            std::sync::Arc::clone(&server.state),
            file.to_string(),
        );
        bridge.seams.arm("preview:before_download", move || {
            let chunk = task_2190_server_chunk(master_key, &saved, b"saved on another device");
            state
                .lock()
                .unwrap()
                .files
                .get_mut(&saved)
                .unwrap()
                .versions
                .push(vec![chunk]);
            db.test_execute(&format!(
                "UPDATE files SET current_version = 3 WHERE file_id = '{saved}'"
            ))
            .unwrap();
        });
        let preview = bridge.parked_conflict_preview(&op_id).await.unwrap();
        assert_eq!(
            server.state.lock().unwrap().files[file].versions.len(),
            3,
            "the seam ran: version 3 landed mid-download"
        );
        assert_eq!(
            preview.version,
            Some(2),
            "legacy {legacy}: the version of the bytes shown, or older"
        );
        if !legacy {
            assert_eq!(
                preview.remote.text.as_deref(),
                Some("edited on the web"),
                "the pinned download is version 2"
            );
        }
        let mut keep = task_2190_keep_into(dir.path());
        for choice in [ParkedChoice::Local, ParkedChoice::Remote, ParkedChoice::Both] {
            assert_eq!(
                bridge
                    .resolve_parked(&op_id, choice, preview.version, &mut keep, "Mac", "10.00")
                    .unwrap(),
                ParkedResolution::Moved,
                "legacy {legacy}: {choice:?} on the version the preview returned"
            );
        }
        assert!(
            Path::new(&staged).exists() && !dir.path().join("kept").exists(),
            "nothing changed"
        );
        let op = bridge.db.get_operation(&op_id).unwrap().unwrap();
        assert_eq!(
            (op.base_version, op.last_error.as_deref()),
            (Some(1), Some("stale_base")),
            "still parked on its base"
        );
    }
}

/// Review-169 F2: another device trashes the file, or the folder it is in, while the window is
/// open. The pull marks the row `trashing` (`mark_subtree_trashing`, the pull's `file_trash`) and
/// leaves its version and its parked write as they were: every choice is `Moved`, the upload stays
/// parked, and the feed offers no decision.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn task_2190_fu2_a_remote_trash_of_the_file_or_its_folder_makes_every_choice_moved() {
    for trash_folder in [false, true] {
        let master_key = [242u8; 32];
        let file = "21900000-0000-4000-8000-000000000058";
        let folder = "21900000-0000-4000-8000-000000000059";
        let (dir, _sync_root, _server, bridge, op_id, staged, _token) = task_2190_parked(master_key, file).await;
        let trashed = if trash_folder {
            seed_folder_row(&bridge, folder, "Docs");
            bridge
                .db
                .test_execute(&format!(
                    "UPDATE files SET path = 'Docs/notes.txt', parent_id = '{folder}' WHERE file_id = '{file}'"
                ))
                .unwrap();
            folder
        } else {
            file
        };
        let marked = bridge.db.mark_subtree_trashing(trashed).unwrap();
        assert!(
            marked.iter().any(|row| row.file_id == file),
            "precondition: the file is marked trashing"
        );
        let mut keep = task_2190_keep_into(dir.path());
        for choice in [ParkedChoice::Local, ParkedChoice::Remote, ParkedChoice::Both] {
            assert_eq!(
                bridge
                    .resolve_parked(&op_id, choice, Some(2), &mut keep, "Mac", "10.00")
                    .unwrap(),
                ParkedResolution::Moved,
                "{choice:?}, the folder trashed: {trash_folder}"
            );
        }
        let op = bridge.db.get_operation(&op_id).unwrap().unwrap();
        assert!(
            op.attempts >= op.max_attempts && op.last_error.as_deref() == Some("stale_base"),
            "the upload stays parked: {op:?}"
        );
        assert_eq!(bridge.db.get_file(file).unwrap().unwrap().status, FileStatus::Trashing);
        assert!(
            Path::new(&staged).exists() && !dir.path().join("kept").exists(),
            "nothing changed"
        );
        let entry = bridge
            .version_conflict_feed()
            .unwrap()
            .into_iter()
            .find(|entry| entry.op_id.as_deref() == Some(op_id.as_str()));
        assert_ne!(
            entry.map(|entry| entry.action),
            Some("open_conflict".to_string()),
            "no decision offered"
        );
    }
}

/// Review-169, after F2 (lead): this Mac trashes or moves a folder the file is in while the window is
/// open. A local trash marks only the folder's row and queues its op on the folder (`queue_finder_delete`);
/// a move queues on the folder. The file's own row and queue stay as they were until the server's echo.
/// The ancestor walk sees both, at any depth: every choice is `Moved`, and nothing is sent again. The last
/// case is a folder above marked `trashing` with nothing queued on it: the walk follows `parent_id`, while
/// the pull's mark of a remote trash follows path prefixes, so a row the prefix misses is still caught.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn task_2190_fu2_a_trash_or_move_of_a_folder_above_makes_every_choice_moved() {
    let (outer, parent) = (
        "21900000-0000-4000-8000-000000000061",
        "21900000-0000-4000-8000-000000000062",
    );
    for (case, folder) in [
        ("trash the parent", parent),
        ("trash a folder above", outer),
        ("move the parent", parent),
        ("a folder above marked trashing, nothing queued", outer),
    ] {
        let master_key = [243u8; 32];
        let file = "21900000-0000-4000-8000-000000000060";
        let (dir, _sync_root, _server, bridge, op_id, staged, _token) = task_2190_parked(master_key, file).await;
        // Outer/Docs/notes.txt: the walk has to go past the parent.
        seed_folder_row(&bridge, outer, "Outer");
        seed_folder_row(&bridge, parent, "Outer/Docs");
        for sql in [
            format!("UPDATE files SET parent_id = '{outer}' WHERE file_id = '{parent}'"),
            format!("UPDATE files SET path = 'Outer/Docs/notes.txt', parent_id = '{parent}' WHERE file_id = '{file}'"),
        ] {
            bridge.db.test_execute(&sql).unwrap();
        }
        assert!(
            bridge.db.is_decidable_parked(&op_id).unwrap(),
            "{case}: precondition, decidable in its folders"
        );
        if case.starts_with("trash") {
            bridge.queue_finder_delete(folder, None).unwrap();
            assert_eq!(
                bridge.db.get_file(folder).unwrap().unwrap().status,
                FileStatus::Trashing
            );
        } else if case.starts_with("a folder above marked") {
            bridge
                .db
                .test_execute(&format!(
                    "UPDATE files SET status = 'trashing' WHERE file_id = '{folder}'"
                ))
                .unwrap();
            assert!(
                bridge.db.list_operations_for_file(folder).unwrap().is_empty(),
                "precondition: nothing queued on it"
            );
        } else {
            bridge
                .queue_finder_modify(FinderWriteTarget {
                    file_id: Some(folder.to_string()),
                    parent_id: None,
                    filename: "Docs".to_string(),
                    rel_path: None,
                    kind: FinderWriteItemKind::Folder,
                    contents_path: None,
                    content_type: None,
                    base_version_identifier: None,
                })
                .unwrap();
            let queued = bridge.db.list_operations_for_file(folder).unwrap();
            assert_eq!(
                queued.iter().map(|op| op.kind.clone()).collect::<Vec<_>>(),
                vec![OperationKind::MoveFile]
            );
        }
        assert_eq!(
            bridge.db.list_operations_for_file(file).unwrap().len(),
            1,
            "{case}: nothing queued on the file"
        );
        assert_ne!(
            bridge.db.get_file(file).unwrap().unwrap().status,
            FileStatus::Trashing,
            "{case}: its row not marked"
        );
        let mut keep = task_2190_keep_into(dir.path());
        for choice in [ParkedChoice::Local, ParkedChoice::Remote, ParkedChoice::Both] {
            assert_eq!(
                bridge
                    .resolve_parked(&op_id, choice, Some(2), &mut keep, "Mac", "10.00")
                    .unwrap(),
                ParkedResolution::Moved,
                "{case}: {choice:?}"
            );
        }
        let op = bridge.db.get_operation(&op_id).unwrap().unwrap();
        assert!(
            op.attempts >= op.max_attempts && op.last_error.as_deref() == Some("stale_base"),
            "{case}: the upload stays parked: {op:?}"
        );
        assert!(
            Path::new(&staged).exists() && !dir.path().join("kept").exists(),
            "{case}: nothing changed"
        );
        let entry = bridge
            .version_conflict_feed()
            .unwrap()
            .into_iter()
            .find(|entry| entry.op_id.as_deref() == Some(op_id.as_str()));
        assert_ne!(
            entry.map(|entry| entry.action),
            Some("open_conflict".to_string()),
            "{case}: no decision offered"
        );
    }
}

/// The walk's depth cap (plan §7.4): folders whose `parent_id`s form a cycle end the walk at the cap,
/// and the write is not decidable; nothing loops.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn task_2190_fu2_a_cycle_above_the_file_ends_the_walk_and_is_not_decidable() {
    let (outer, parent) = (
        "21900000-0000-4000-8000-000000000064",
        "21900000-0000-4000-8000-000000000065",
    );
    let master_key = [244u8; 32];
    let file = "21900000-0000-4000-8000-000000000063";
    let (_dir, _sync_root, _server, bridge, op_id, _staged, _token) = task_2190_parked(master_key, file).await;
    seed_folder_row(&bridge, outer, "Outer");
    seed_folder_row(&bridge, parent, "Outer/Docs");
    for sql in [
        format!("UPDATE files SET parent_id = '{outer}' WHERE file_id = '{parent}'"),
        format!("UPDATE files SET path = 'Outer/Docs/notes.txt', parent_id = '{parent}' WHERE file_id = '{file}'"),
    ] {
        bridge.db.test_execute(&sql).unwrap();
    }
    assert!(
        bridge.db.is_decidable_parked(&op_id).unwrap(),
        "precondition: decidable under a real chain"
    );
    bridge
        .db
        .test_execute(&format!(
            "UPDATE files SET parent_id = '{parent}' WHERE file_id = '{outer}'"
        ))
        .unwrap();
    assert!(
        !bridge.db.is_decidable_parked(&op_id).unwrap(),
        "a cycle reaches the cap: not decidable"
    );
}

/// Review-169 F3: the "Keep both" name is at most 255 bytes (`kept_names::MAX_NAME_BYTES`, as kept
/// files are) whatever the original, the device or the counter: the stem is cut at a character
/// boundary, and the suffix and the extension never are.
#[test]
fn task_2190_fu2_keep_both_name_fits_the_name_limit() {
    use crate::sign_out::kept_names::MAX_NAME_BYTES;
    let none = std::collections::HashSet::new();
    // The review's case: 240 ASCII bytes and `.txt` (244 bytes), on `MacBook Pro` at 10.00.
    let ascii = format!("{}.txt", "a".repeat(240));
    let name = keep_both_name(&ascii, "MacBook Pro", "10.00", &none);
    assert_eq!(name, format!("{} (MacBook Pro, 10.00).txt", "a".repeat(230)));
    assert_eq!(name.len(), MAX_NAME_BYTES);
    // The counter at the limit: the stem gives way, the counter stays.
    let taken = std::collections::HashSet::from([name.to_lowercase()]);
    let second = keep_both_name(&ascii, "MacBook Pro", "10.00", &taken);
    assert_eq!(second, format!("{} (MacBook Pro, 10.00) 2.txt", "a".repeat(228)));
    // Multibyte: 80 three-byte characters (240 bytes) are cut at a character boundary.
    let wide = format!("{}.txt", "日".repeat(80));
    let name = keep_both_name(&wide, "MacBook Pro", "10.00", &none);
    assert_eq!(name, format!("{} (MacBook Pro, 10.00).txt", "日".repeat(76)));
    assert!(name.len() <= MAX_NAME_BYTES);
    // A multibyte device name, and one far too long: the device is cut; the time and the extension stay.
    let curly = keep_both_name(&wide, "Guus’s MacBook Pro", "10.00", &none);
    assert!(
        curly.len() <= MAX_NAME_BYTES && curly.ends_with(" (Guus’s MacBook Pro, 10.00).txt"),
        "{curly}"
    );
    let long = keep_both_name("notes.txt", &"x".repeat(300), "10.00", &none);
    assert!(
        long.len() <= MAX_NAME_BYTES && long.starts_with("notes (x") && long.ends_with("x, 10.00).txt"),
        "{long}"
    );
}

/// D4: the copy's name is unique among the parent's names, case-insensitively.
#[test]
fn task_2190_fu2_keep_both_name_is_unique() {
    let taken = |names: &[&str]| {
        names
            .iter()
            .map(|name| name.to_lowercase())
            .collect::<std::collections::HashSet<_>>()
    };
    assert_eq!(
        keep_both_name("notes.txt", "Mac", "10.00", &taken(&[])),
        "notes (Mac, 10.00).txt"
    );
    assert_eq!(
        keep_both_name("notes.txt", "Mac", "10.00", &taken(&["notes (Mac, 10.00).txt"])),
        "notes (Mac, 10.00) 2.txt"
    );
    assert_eq!(
        keep_both_name(
            "notes.txt",
            "Mac",
            "10.00",
            &taken(&["NOTES (MAC, 10.00).TXT", "notes (Mac, 10.00) 2.txt"])
        ),
        "notes (Mac, 10.00) 3.txt",
        "case-insensitive, and the counter keeps counting"
    );
    assert_eq!(
        keep_both_name("README", "Mac", "10.00", &taken(&[])),
        "README (Mac, 10.00)",
        "no extension"
    );
    assert_eq!(
        keep_both_name(".env", "Mac", "10.00", &taken(&[])),
        ".env (Mac, 10.00)",
        "a dotfile"
    );
    assert_eq!(
        keep_both_name("a (1).tar.gz", "Mac", "10.00", &taken(&[])),
        "a (1).tar (Mac, 10.00).gz",
        "the last dot"
    );
}

/// The lead's condition on E3, for a sync folder: a hydrated file whose row is then put in
/// `state`, edited on this PC while the web saves version 2, then the save path, the scan, the
/// tick and a hydrate. Returns the feed's kinds for the file, the content, whether the hydrate
/// was refused, the bytes on disk, the queued uploads and the server's current bytes.
async fn task_2190_held(
    master_key: [u8; 32],
    file: &str,
    state_sql: &str,
) -> (Vec<String>, SyncFolderContent, bool, Vec<u8>, usize, Vec<u8>) {
    let (_dir, sync_root, server, bridge) = rule3_setup(master_key);
    seed_uploaded_row(&bridge, &server, file);
    {
        let mut s = server.state.lock().unwrap();
        s.files.get_mut(file).unwrap().versions[0] = vec![task_2190_server_chunk(master_key, file, b"version one")];
        s.serve_downloads = true;
    }
    let local = sync_root.join("notes.txt");
    bridge.hydrate_file(file, &local, &[sync_root.as_path()]).await.unwrap();
    bridge.db.test_execute(&state_sql.replace("{file}", file)).unwrap();
    std::fs::write(&local, b"saved on this PC").unwrap();
    task_2190_web_saves(&server, master_key, file, b"edited on the web");
    crate::watcher::dispatch_local_create(&bridge, &sync_root, &local, "notify");
    crate::watcher::run_one_scan(&bridge, &sync_root);
    sync_tick_outcome(&bridge, &sync_root).await.unwrap();
    let refused = matches!(
        bridge.hydrate_file(file, &local, &[sync_root.as_path()]).await,
        Err(e) if engine_cause::<UnsentLocalChange>(&e).is_some()
    );
    let kinds = bridge
        .version_conflict_feed()
        .unwrap()
        .into_iter()
        .filter(|entry| entry.file_id == file)
        .map(|entry| entry.kind)
        .collect();
    let content = sync_folder_content(&bridge.db, file, &local, None).unwrap();
    let uploads = bridge
        .db
        .list_operations_for_file(file)
        .unwrap()
        .into_iter()
        .filter(|op| matches!(op.kind, OperationKind::UploadVersion | OperationKind::UploadFile))
        .count();
    let on_disk = std::fs::read(&local).unwrap();
    drop(bridge);
    let server_now = server.finish().latest_plaintext(file, master_key);
    (kinds, content, refused, on_disk, uploads, server_now)
}

/// A row with agreed content but no base (0), edited while the web changed: the edit is held,
/// shown in Review as needing a decision, never reads as agreed (so the stamp pass cannot mark
/// it in sync), and the hydrate refuses to write over it. Both versions are kept.
#[tokio::test]
async fn task_2190_held_a_base_0_edit_with_a_web_edit_is_visible_never_stamped_never_overwritten() {
    let (kinds, content, refused, on_disk, uploads, server_now) = task_2190_held(
        [231u8; 32],
        "21900000-0000-4000-8000-000000000041",
        "UPDATE files SET local_base_version = 0 WHERE file_id = '{file}'",
    )
    .await;
    assert_eq!(kinds, vec!["held_edit".to_string()], "shown as needing a decision");
    assert_eq!(
        content,
        SyncFolderContent::Changed { base: None },
        "never reads as agreed"
    );
    assert!(refused, "the hydrate refuses to write over it");
    assert_eq!(
        (on_disk.as_slice(), uploads),
        (&b"saved on this PC"[..], 0),
        "kept on this PC, not sent"
    );
    assert_eq!(server_now, b"edited on the web", "the web's version is kept");
}

/// A file hydrated before task 1640 (unknown content), edited while the web changed: the edit
/// is held, shown in Review in the one "not checked yet" row (the scan cannot tell an edited
/// unknown file from an untouched one, so they are never "needs a decision" per file), never
/// reads as agreed, and the hydrate refuses to write over it. Both versions are kept.
#[tokio::test]
async fn task_2190_held_a_pre_1640_edit_with_a_web_edit_is_visible_never_stamped_never_overwritten() {
    let (kinds, content, refused, on_disk, uploads, server_now) = task_2190_held(
            [232u8; 32],
            "21900000-0000-4000-8000-000000000042",
            "UPDATE files SET local_hash = NULL, local_base_version = 0, fetched_min_version = -2, fetched_hash = NULL WHERE file_id = '{file}'",
        )
        .await;
    assert_eq!(
        kinds,
        vec!["held_unknown".to_string()],
        "shown in the one not-checked-yet row"
    );
    assert_eq!(content, SyncFolderContent::Unknown, "never reads as agreed");
    assert!(refused, "the hydrate refuses to write over it");
    assert_eq!(
        (on_disk.as_slice(), uploads),
        (&b"saved on this PC"[..], 0),
        "kept on this PC, not sent"
    );
    assert_eq!(server_now, b"edited on the web", "the web's version is kept");
}

/// macOS: a Finder save whose base is not known (an identifier with no server version), while
/// the web changed: it parks `base_unknown` with its bytes and is shown in Review; Finder keeps
/// the Mac's token, so nothing writes over it, and nothing is sent. Both versions are kept.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn task_2190_held_a_finder_edit_on_an_unknown_base_is_visible_and_keeps_both() {
    let master_key = [233u8; 32];
    let file = "21900000-0000-4000-8000-000000000043";
    let (dir, sync_root, server, bridge) = rule3_setup(master_key);
    seed_uploaded_row(&bridge, &server, file);
    let save = fp_save(
        &bridge,
        dir.path(),
        file,
        "notes.txt",
        b"edited on this Mac",
        "no-version",
    );
    let token = save.token.clone().expect("a Finder save names its bytes");
    task_2190_web_saves(&server, master_key, file, b"edited on the web");
    sync_tick_outcome(&bridge, &sync_root).await.unwrap();
    bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();
    let parked = bridge.db.list_operations_for_file(file).unwrap().remove(0);
    assert_eq!(parked.last_error.as_deref(), Some("base_unknown"));
    assert_eq!(
        std::fs::read(parked.payload_path.as_ref().unwrap()).unwrap(),
        b"edited on this Mac"
    );
    assert_eq!(
        held_content_version(&bridge, file),
        token,
        "Finder keeps the Mac's bytes"
    );
    let feed = bridge.version_conflict_feed().unwrap();
    assert!(
        feed.iter()
            .any(|entry| entry.op_id.as_deref() == Some(parked.op_id.as_str())),
        "shown in Review: {feed:?}"
    );
    drop(bridge);
    let state = server.finish();
    assert!(state.init_summary().is_empty(), "nothing was sent");
    assert_eq!(state.latest_plaintext(file, master_key), b"edited on the web");
}

/// The lead's check on glegion's constraint 1: a held edit keeps #167's scan memo working.
/// Holding it writes only `held_edits`, and no trigger writes `files`, so the row stamp the memo
/// keys on does not move, and the second pass skips the held file as settled instead of
/// classifying it again.
#[tokio::test]
async fn task_2190_held_a_held_edit_stays_settled_in_the_scan_memo() {
    let master_key = [234u8; 32];
    let file = "21900000-0000-4000-8000-000000000044";
    let (_dir, sync_root, server, bridge) = rule3_setup(master_key);
    seed_uploaded_row(&bridge, &server, file);
    {
        let mut s = server.state.lock().unwrap();
        s.files.get_mut(file).unwrap().versions[0] = vec![task_2190_server_chunk(master_key, file, b"version one")];
        s.serve_downloads = true;
    }
    let local = sync_root.join("notes.txt");
    bridge.hydrate_file(file, &local, &[sync_root.as_path()]).await.unwrap();
    bridge
        .db
        .test_execute(&format!(
            "UPDATE files SET local_base_version = 0 WHERE file_id = '{file}'"
        ))
        .unwrap();
    std::fs::write(&local, b"saved on this PC").unwrap();
    let mut memo = crate::watcher::ScanMemo::default();
    crate::watcher::run_one_scan_with(&bridge, &sync_root, &mut memo);
    assert_eq!(memo.classified, 1, "the first pass classifies the edit");
    assert_eq!(bridge.db.list_held_edits().unwrap().len(), 1, "and holds it");
    let stamp = bridge.db.file_row_stamp_by_path("notes.txt").unwrap();
    crate::watcher::run_one_scan_with(&bridge, &sync_root, &mut memo);
    assert_eq!(
        bridge.db.file_row_stamp_by_path("notes.txt").unwrap(),
        stamp,
        "the row stamp did not move"
    );
    assert_eq!(
        (memo.classified, memo.skipped),
        (0, 1),
        "the second pass skips the held file as settled"
    );
    assert_eq!(bridge.db.list_held_edits().unwrap().len(), 1, "still held");
}

/// What [`task_2190_sync_folder_sequence`] saw, step by step.
#[derive(Debug)]
struct SyncFolderEdit {
    /// After the edit: did the upload driver queue it, how many ops, and on which base.
    dispatched: bool,
    queued_after_edit: usize,
    base_after_edit: Option<i64>,
    /// The tick's conflicts, and the row after it.
    tick_conflicts: usize,
    row_after_tick: FileEntry,
    /// Whether the next hydrate refused to write over the edit, and the local file's bytes after it.
    hydrate_refused: bool,
    local_after_hydrate: Vec<u8>,
    /// Every copy of the edit under the test's root, and every queued op of the file, after it.
    copies_of_the_edit: Vec<PathBuf>,
    ops_after: usize,
    /// The review feed's kinds and the `Conflict` rows, after it.
    feed_kinds: Vec<String>,
    conflict_rows: usize,
    /// The server: every init, its versions and its current plaintext.
    inits: Vec<(serde_json::Value, serde_json::Value, u16)>,
    server_versions: usize,
    server_current: Vec<u8>,
}

/// The sync folder (the engine as Windows and Linux run it, with no write id): a hydrated
/// file at version 1 is edited on disk, the web saves version 2, the tick runs, then the
/// upload attempt and the next hydrate.
async fn task_2190_sync_folder_sequence(master_key: [u8; 32], file: &str) -> SyncFolderEdit {
    let (dir, sync_root, server, bridge) = rule3_setup(master_key);
    seed_uploaded_row(&bridge, &server, file);
    {
        let mut s = server.state.lock().unwrap();
        s.files.get_mut(file).unwrap().versions[0] = vec![task_2190_server_chunk(master_key, file, b"version one")];
        s.serve_downloads = true;
    }
    let local = sync_root.join("notes.txt");
    bridge.hydrate_file(file, &local, &[sync_root.as_path()]).await.unwrap();
    assert_eq!(std::fs::read(&local).unwrap(), b"version one", "precondition: hydrated");
    assert_eq!(
        bridge.db.get_file(file).unwrap().unwrap().status,
        FileStatus::Local,
        "precondition"
    );

    // 1. The person edits the file in the sync folder; the upload driver sees its close.
    std::fs::write(&local, b"edited in the sync folder").unwrap();
    let dispatched = crate::watcher::dispatch_local_create(&bridge, &sync_root, &local, "notify");
    let queued = bridge.db.list_operations_for_file(file).unwrap();
    let (queued_after_edit, base_after_edit) = (queued.len(), queued.first().and_then(|op| op.base_version));

    // 2. The web saves version 2.
    task_2190_web_saves(&server, master_key, file, b"edited on the web");

    // 3. The tick.
    let tick = sync_tick_outcome(&bridge, &sync_root).await.unwrap();
    let row_after_tick = bridge.db.get_file(file).unwrap().unwrap();

    // 4. The upload attempt.
    bridge.process_due_operations(&sync_root, now_secs()).await.unwrap();

    // 5. The next hydrate, as a fetch after "Free up space" (or a pinned refresh) runs it.
    let hydrate_refused = match bridge.hydrate_file(file, &local, &[sync_root.as_path()]).await {
        Ok(()) => false,
        Err(e) if engine_cause::<UnsentLocalChange>(&e).is_some() => true,
        Err(e) => panic!("the hydrate failed for another reason: {e:#}"),
    };

    let feed_kinds = bridge
        .version_conflict_feed()
        .unwrap()
        .into_iter()
        .map(|entry| entry.kind)
        .collect();
    let observed = (
        std::fs::read(&local).unwrap(),
        task_2190_files_holding(dir.path(), b"edited in the sync folder"),
        bridge.db.list_operations_for_file(file).unwrap().len(),
        bridge.db.list_by_status(FileStatus::Conflict).unwrap().len(),
    );
    drop(bridge);
    let state = server.finish();
    SyncFolderEdit {
        dispatched,
        queued_after_edit,
        base_after_edit,
        tick_conflicts: tick.conflicts.len(),
        row_after_tick,
        hydrate_refused,
        local_after_hydrate: observed.0,
        copies_of_the_edit: observed.1,
        ops_after: observed.2,
        feed_kinds,
        conflict_rows: observed.3,
        inits: state.init_summary(),
        server_versions: state.files[file].versions.len(),
        server_current: state.latest_plaintext(file, master_key),
    }
}

/// Since task 1640, step by step: the upload driver queues the edit as a modify on version 1,
/// the tick still reports no conflict (the row is `Uploading`, which the detector skips), the
/// upload is refused as stale and parks with its bytes, and the next hydrate refuses to write
/// over the edit. Both versions survive: the web's on the server, the edit on disk and in the
/// parked op's copy, shown as one `stale_base` review entry. Before task 1640 the edit was
/// never queued and the hydrate wrote over it (red on main b89380d0).
#[tokio::test]
async fn task_2190_a_sync_folder_edit_is_queued_on_its_base_parks_as_stale_and_is_never_overwritten() {
    let master_key = [220u8; 32];
    let seen = task_2190_sync_folder_sequence(master_key, "21900000-0000-4000-8000-000000000002").await;
    assert!(seen.dispatched, "the upload driver queued the edit");
    assert_eq!(
        (seen.queued_after_edit, seen.base_after_edit),
        (1, Some(1)),
        "one modify, on version 1"
    );
    assert_eq!(seen.tick_conflicts, 0, "the tick still reports no conflict");
    assert_eq!(
        seen.row_after_tick.status,
        FileStatus::Uploading,
        "the queued edit owns the row"
    );
    assert_eq!(
        seen.inits,
        vec![(json!("21900000-0000-4000-8000-000000000002"), json!(1), 409)],
        "sent on 1, refused as stale"
    );
    assert!(seen.hydrate_refused, "the hydrate refused to write over the edit");
    assert_eq!(
        seen.local_after_hydrate, b"edited in the sync folder",
        "the edit stays on disk"
    );
    assert!(!seen.copies_of_the_edit.is_empty());
    assert_eq!(seen.ops_after, 1, "the parked op keeps its copy");
    assert_eq!(
        seen.feed_kinds,
        vec!["stale_base".to_string()],
        "shown as one review entry"
    );
    assert_eq!(
        seen.conflict_rows, 0,
        "no Conflict row: the detector is not task 1640's"
    );
    assert_eq!(seen.server_versions, 2);
    assert_eq!(seen.server_current, b"edited on the web");
}

/// The outcome the task asks for (its verification line), for the same sequence: the tick
/// raises a conflict, or the edit is kept somewhere the person can get it back from, and the
/// web's version is on the server. Red on main b89380d0 (the edit survived nowhere); green
/// since task 1640.
#[tokio::test]
async fn task_2190_wanted_a_sync_folder_edit_and_a_web_edit_both_survive() {
    let master_key = [221u8; 32];
    let seen = task_2190_sync_folder_sequence(master_key, "21900000-0000-4000-8000-000000000003").await;
    let flagged = seen.tick_conflicts > 0 || seen.conflict_rows > 0;
    let edit_kept = !seen.copies_of_the_edit.is_empty() || seen.ops_after > 0;
    assert!(
        flagged || edit_kept,
        "the sync-folder edit was lost with nothing shown: {seen:#?}"
    );
    assert_eq!(
        seen.server_current, b"edited on the web",
        "the web's version is on the server"
    );
}
