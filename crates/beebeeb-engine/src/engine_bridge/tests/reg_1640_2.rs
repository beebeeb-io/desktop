//! `reg_1640_2` engine tests, split out of `engine_bridge.rs`.

use super::*;
/// A candidate decides like a baseline: equal bytes are agreed; other bytes are a change on
/// the candidate's version; a mixed candidate is never agreed and sends on the oldest version.
#[test]
fn regression_1640_a_candidate_decides_like_a_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(&dir.path().join("state.db")).unwrap();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, b"version three").unwrap();
    db.upsert_file(&FileEntry {
        file_id: "f97d7f90-511b-52b5-9303-7201b53df85f".into(),
        path: "notes.txt".into(),
        status: FileStatus::Local,
        size_bytes: 13,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 1,
        parent_id: None,
        item_kind: ItemKind::File,
    })
    .unwrap();
    assert!(
        db.mark_no_fetched_content("f97d7f90-511b-52b5-9303-7201b53df85f")
            .unwrap()
    );
    db.record_fetched_version("f97d7f90-511b-52b5-9303-7201b53df85f", 3, &sha256_hex(b"version three"))
        .unwrap();
    let content = || sync_folder_content(&db, "f97d7f90-511b-52b5-9303-7201b53df85f", &path, None).unwrap();
    assert_eq!(content(), SyncFolderContent::Agreed, "the candidate's bytes");
    std::fs::write(&path, b"edited, never sent").unwrap();
    assert_eq!(
        content(),
        SyncFolderContent::Changed { base: Some(3) },
        "an edit, on the candidate's version"
    );
    db.record_fetched_version("f97d7f90-511b-52b5-9303-7201b53df85f", 2, &sha256_hex(b"version two"))
        .unwrap();
    assert_eq!(
        content(),
        SyncFolderContent::Changed { base: Some(2) },
        "mixed: never agreed, the oldest"
    );
}

/// The edit an Office save makes before the stamp pass promoted anything is sent, on the
/// candidate's version.
#[tokio::test]
async fn regression_1640_an_edit_before_promotion_is_sent_on_the_candidates_version() {
    let master_key = [182u8; 32];
    let file = "16400000-0000-4000-8000-000000000042";
    let (_dir, sync_root, server, bridge) = rule3_setup(master_key);
    seed_uploaded_row(&bridge, &server, file);
    let local = sync_root.join("notes.txt");
    std::fs::write(&local, b"version one").unwrap();
    assert!(bridge.db.mark_no_fetched_content(file).unwrap());
    bridge
        .db
        .record_fetched_version(file, 1, &sha256_hex(b"version one"))
        .unwrap();
    assert!(
        !crate::watcher::dispatch_local_create(&bridge, &sync_root, &local, "scan"),
        "agreed: nothing"
    );
    std::fs::write(&local, b"saved by an editor").unwrap();
    assert!(crate::watcher::dispatch_local_create(
        &bridge, &sync_root, &local, "notify"
    ));
    assert_eq!(
        bridge.db.list_operations_for_file(file).unwrap()[0].base_version,
        Some(1)
    );
    drain_upload_queue(&bridge, &sync_root).await;
    let state = server.finish();
    assert_eq!(state.init_summary(), vec![(json!(file), json!(1), 201)]);
    assert_eq!(state.latest_plaintext(file, master_key), b"saved by an editor");
}

/// Condition 2: promotion needs the platform to say in sync with no modified data, the whole
/// file on disk, the bytes read under the handle equal to the candidate, a candidate of one
/// version, and the row still `Local` with that candidate.
#[test]
fn regression_1640_a_candidate_is_promoted_only_when_the_bytes_are_its_own() {
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
    let candidate = sha256_hex(b"version three");
    assert!(
        db.mark_no_fetched_content("f97d7f90-511b-52b5-9303-7201b53df85f")
            .unwrap()
    );
    db.record_fetched_version("f97d7f90-511b-52b5-9303-7201b53df85f", 3, &candidate)
        .unwrap();
    let promote = |local, wholly, bytes: &'static [u8]| {
        promote_fetched_content(&db, "f97d7f90-511b-52b5-9303-7201b53df85f", local, wholly, &mut || {
            Ok(sha256_hex(bytes))
        })
        .unwrap()
    };
    assert!(!promote(LocalEntry::InSync, true, b"other bytes"), "other bytes: no");
    assert!(
        !promote(LocalEntry::PartlyLocal, true, b"version three"),
        "out of sync or modified: no"
    );
    assert!(
        !promote(LocalEntry::InSync, false, b"version three"),
        "not wholly on disk: no"
    );
    db.set_status("f97d7f90-511b-52b5-9303-7201b53df85f", FileStatus::Uploading)
        .unwrap();
    assert!(
        !promote(LocalEntry::InSync, true, b"version three"),
        "the row moved on: no"
    );
    db.set_status("f97d7f90-511b-52b5-9303-7201b53df85f", FileStatus::Local)
        .unwrap();
    assert!(
        promote(LocalEntry::InSync, true, b"version three"),
        "its own bytes: promoted"
    );
    let contract = db
        .get_file_contract_state("f97d7f90-511b-52b5-9303-7201b53df85f")
        .unwrap()
        .unwrap();
    assert_eq!((contract.local_base_version, contract.local_hash), (3, Some(candidate)));
    assert_eq!(
        db.fetched_content("f97d7f90-511b-52b5-9303-7201b53df85f").unwrap(),
        None
    );
    // A mixed candidate is never promoted.
    db.upsert_file(&FileEntry {
        file_id: "ead1903c-d8ae-5ad1-a577-ff626ebb2b20".into(),
        path: "mixed.txt".into(),
        status: FileStatus::Local,
        size_bytes: 1,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 1,
        parent_id: None,
        item_kind: ItemKind::File,
    })
    .unwrap();
    assert!(
        db.mark_no_fetched_content("ead1903c-d8ae-5ad1-a577-ff626ebb2b20")
            .unwrap()
    );
    db.record_fetched_version("ead1903c-d8ae-5ad1-a577-ff626ebb2b20", 3, "h3")
        .unwrap();
    db.record_fetched_version("ead1903c-d8ae-5ad1-a577-ff626ebb2b20", 2, "h2")
        .unwrap();
    assert!(
        !promote_fetched_content(
            &db,
            "ead1903c-d8ae-5ad1-a577-ff626ebb2b20",
            LocalEntry::InSync,
            true,
            &mut || Ok("h2".into())
        )
        .unwrap(),
        "mixed: never"
    );
}

/// Review N5: the kept copy's final move never replaces a file (Windows: MoveFileExW without
/// REPLACE_EXISTING).
#[test]
fn regression_1640_a_move_never_replaces_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let (from, existing, free) = (
        dir.path().join("from"),
        dir.path().join("existing"),
        dir.path().join("free"),
    );
    std::fs::write(&from, b"new").unwrap();
    std::fs::write(&existing, b"there first").unwrap();
    let refused = move_no_replace(&from, &existing).unwrap_err();
    assert_eq!(refused.kind(), std::io::ErrorKind::AlreadyExists, "{refused}");
    assert_eq!(
        std::fs::read(&existing).unwrap(),
        b"there first",
        "the existing file is untouched"
    );
    assert_eq!(std::fs::read(&from).unwrap(), b"new", "and the source stays");
    move_no_replace(&from, &free).unwrap();
    assert_eq!(std::fs::read(&free).unwrap(), b"new");
    assert!(!from.exists());
}

// ── Task 1640, review F1: the candidate only ever moves down or becomes mixed ──

/// A `Local` row for `file_id` at `path`, whose placeholder was created where nothing was.
pub(super) fn regression_1640_new_placeholder(db: &StateDb, file_id: &str, path: &str) {
    db.upsert_file(&FileEntry {
        file_id: file_id.into(),
        path: path.into(),
        status: FileStatus::Local,
        size_bytes: 1,
        modified_at: 1,
        content_hash: None,
        remote_updated_at: 1,
        parent_id: None,
        item_kind: ItemKind::File,
    })
    .unwrap();
    assert!(db.mark_no_fetched_content(file_id).unwrap());
}

/// One fetch as `hydrate_file_to_memory` records it: `Downloading`, the candidate before the
/// transfer, then `Local`.
fn regression_1640_fetch(db: &StateDb, file_id: &str, version: i64, plaintext: &[u8]) {
    db.set_status(file_id, FileStatus::Downloading).unwrap();
    db.record_fetched_version(file_id, version, &sha256_hex(plaintext))
        .unwrap();
    db.set_status(file_id, FileStatus::Local).unwrap();
}

/// One tick's reconcile of a placeholder that is partly on disk (`RECALL_ON_DATA_ACCESS`):
/// the decode reads it as cloud-only.
fn regression_1640_tick_partly_on_disk(db: &StateDb, file_id: &str) {
    let row = db.get_file(file_id).unwrap().unwrap();
    let (status, _) = crate::state_db::decode_os_state(0x0040_0000, row.status);
    assert_eq!(status, Some(FileStatus::CloudOnly), "precondition: the decode");
    db.reconcile_os_state(&[(file_id.to_string(), status, None)], 1)
        .unwrap();
    assert_eq!(db.get_file(file_id).unwrap().unwrap().status, FileStatus::CloudOnly);
}

/// Review F1: ranges of version 1 are fetched; a tick reads the partial placeholder as
/// cloud-only; the web saves version 2; the rest is fetched at 2. The bytes are 1 and 2
/// spliced, so an edit is sent on 1 (a stale park when the server moved), never on 2. Before,
/// the tick wiped the candidate and the next fetch started a fresh one at 2.
#[test]
fn regression_1640_a_tick_between_range_fetches_never_raises_the_base() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(&dir.path().join("state.db")).unwrap();
    let path = dir.path().join("notes.txt");
    regression_1640_new_placeholder(&db, "f97d7f90-511b-52b5-9303-7201b53df85f", "notes.txt");
    regression_1640_fetch(&db, "f97d7f90-511b-52b5-9303-7201b53df85f", 1, b"version one");
    regression_1640_tick_partly_on_disk(&db, "f97d7f90-511b-52b5-9303-7201b53df85f");
    regression_1640_fetch(&db, "f97d7f90-511b-52b5-9303-7201b53df85f", 2, b"version two");
    regression_1640_tick_partly_on_disk(&db, "f97d7f90-511b-52b5-9303-7201b53df85f");
    db.set_status("f97d7f90-511b-52b5-9303-7201b53df85f", FileStatus::Local)
        .unwrap();
    assert_eq!(
        db.fetched_content("f97d7f90-511b-52b5-9303-7201b53df85f").unwrap(),
        Some((1, None)),
        "mixed, at the lower version"
    );
    std::fs::write(&path, b"ranges of one and two, then edited").unwrap();
    assert_eq!(
        sync_folder_content(&db, "f97d7f90-511b-52b5-9303-7201b53df85f", &path, None).unwrap(),
        SyncFolderContent::Changed { base: Some(1) }
    );
    // Same when the file had agreed content at 1 before it was partly freed.
    regression_1640_new_placeholder(&db, "g", "other.txt");
    db.record_local_baseline("g", 1, &sha256_hex(b"version one")).unwrap();
    regression_1640_tick_partly_on_disk(&db, "g");
    regression_1640_fetch(&db, "g", 2, b"version two");
    db.set_status("g", FileStatus::Local).unwrap();
    let other = dir.path().join("other.txt");
    std::fs::write(&other, b"ranges of one and two, then edited").unwrap();
    assert_eq!(
        sync_folder_content(&db, "g", &other, None).unwrap(),
        SyncFolderContent::Changed { base: Some(1) }
    );
    // A person's write into a partly hydrated file (a dirty partial placeholder), ticks around it:
    // its bytes are not the candidate's, so they are a change on the candidate's version.
    regression_1640_new_placeholder(&db, "w", "written.txt");
    regression_1640_fetch(&db, "w", 3, b"version three");
    regression_1640_tick_partly_on_disk(&db, "w");
    let written = dir.path().join("written.txt");
    std::fs::write(&written, b"part of version three, then written over").unwrap();
    db.set_status("w", FileStatus::Local).unwrap();
    regression_1640_tick_partly_on_disk(&db, "w");
    assert_eq!(
        db.fetched_content("w").unwrap(),
        Some((3, Some(sha256_hex(b"version three")))),
        "the write kept it"
    );
    assert_eq!(
        sync_folder_content(&db, "w", &written, None).unwrap(),
        SyncFolderContent::Changed { base: Some(3) },
        "a write: a change on the candidate's version, never agreed"
    );
}

/// Review F1, the other two ways a candidate was reset while bytes stayed: the size resolution
/// (a resize that does not dehydrate) and the startup repair (a download cut off by a stop).
/// Both make the content mixed, version kept, so an edit is sent on the oldest version and the
/// file is never agreed (never freed, never marked in sync), even once its bytes are one version.
#[test]
fn regression_1640_a_resize_or_a_cut_off_download_makes_the_content_mixed_never_resets_it() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(&dir.path().join("state.db")).unwrap();
    // Size resolution: ranges of 1 on disk; a fetch at 2 (recorded, then failed on the size).
    regression_1640_new_placeholder(&db, "resized", "resized.txt");
    regression_1640_fetch(&db, "resized", 1, b"version one");
    db.set_status("resized", FileStatus::Downloading).unwrap();
    db.record_fetched_version("resized", 2, &sha256_hex(b"version two, longer"))
        .unwrap();
    db.poison_local_content("resized").unwrap();
    db.set_status("resized", FileStatus::CloudOnly).unwrap();
    regression_1640_fetch(&db, "resized", 2, b"version two, longer");
    let resized = dir.path().join("resized.txt");
    std::fs::write(&resized, b"ranges of one and two, then edited").unwrap();
    assert_eq!(
        sync_folder_content(&db, "resized", &resized, None).unwrap(),
        SyncFolderContent::Changed { base: Some(1) },
        "a resize never resets the candidate"
    );
    // A one-version candidate made mixed is never agreed, even with that version's bytes.
    regression_1640_new_placeholder(&db, "one", "one.txt");
    regression_1640_fetch(&db, "one", 2, b"version two");
    db.poison_local_content("one").unwrap();
    regression_1640_fetch(&db, "one", 2, b"version two");
    let one = dir.path().join("one.txt");
    std::fs::write(&one, b"version two").unwrap();
    assert_eq!(
        sync_folder_content(&db, "one", &one, None).unwrap(),
        SyncFolderContent::Changed { base: Some(2) }
    );
    // Startup repair: one row cut off mid-fetch with ranges of 1, one with agreed content at 1.
    regression_1640_new_placeholder(&db, "cut", "cut.txt");
    regression_1640_fetch(&db, "cut", 1, b"version one");
    db.set_status("cut", FileStatus::Downloading).unwrap();
    regression_1640_new_placeholder(&db, "agreed", "agreed.txt");
    db.record_local_baseline("agreed", 1, &sha256_hex(b"version one"))
        .unwrap();
    db.set_status("agreed", FileStatus::Downloading).unwrap();
    db.reconcile_stale_in_flight_on_startup().unwrap();
    for id in ["cut", "agreed"] {
        assert_eq!(db.get_file(id).unwrap().unwrap().status, FileStatus::CloudOnly, "{id}");
        assert_eq!(
            db.get_file_contract_state(id).unwrap().unwrap().local_hash,
            None,
            "{id}"
        );
        assert_eq!(
            db.fetched_content(id).unwrap(),
            Some((1, None)),
            "{id}: mixed, version kept"
        );
    }
}

/// Review F1, end to end through the fetch path (Windows): ranges fetched at version 1, a
/// tick, the web saves 2, the rest fetched at 2, an edit. It is sent on 1, refused as stale
/// and parks with its bytes; the web's version 2 is not replaced.
#[cfg(target_os = "windows")]
#[tokio::test]
async fn regression_1640_range_fetches_across_a_tick_and_a_web_save_never_replace_the_web_version() {
    let master_key = [183u8; 32];
    let file = "16400000-0000-4000-8000-000000000051";
    let (_dir, sync_root, server, bridge) = rule3_setup(master_key);
    seed_uploaded_row(&bridge, &server, file);
    {
        let mut s = server.state.lock().unwrap();
        s.files.get_mut(file).unwrap().versions[0] = vec![regression_1640_chunk(master_key, file, b"version one")];
        s.serve_downloads = true;
    }
    assert!(bridge.db.mark_no_fetched_content(file).unwrap());
    let fetch = bridge.hydrate_file_to_memory(file).await.unwrap();
    assert_eq!(fetch.version.as_ref().unwrap().version, 1);
    drop(fetch);
    regression_1640_tick_partly_on_disk(&bridge.db, file);
    regression_1640_web_saves(&server, master_key, file, b"version two");
    let fetch = bridge.hydrate_file_to_memory(file).await.unwrap();
    assert_eq!(fetch.version.as_ref().unwrap().version, 2);
    drop(fetch);
    assert_eq!(bridge.db.fetched_content(file).unwrap(), Some((1, None)));
    let local = sync_root.join("notes.txt");
    std::fs::write(&local, b"ranges of one and two, then edited").unwrap();
    assert!(crate::watcher::dispatch_local_create(
        &bridge, &sync_root, &local, "notify"
    ));
    assert_eq!(
        bridge.db.list_operations_for_file(file).unwrap()[0].base_version,
        Some(1)
    );
    drain_upload_queue(&bridge, &sync_root).await;
    assert_eq!(
        std::fs::read(&local).unwrap(),
        b"ranges of one and two, then edited",
        "the edit stays"
    );
    let state = server.finish();
    assert_eq!(
        state.latest_plaintext(file, master_key),
        b"version two",
        "the web's version is not replaced"
    );
}

/// Review F1: a landing sent the whole file, so nothing fetched is left on disk: its candidate
/// goes with the old agreed content.
#[tokio::test]
async fn regression_1640_a_landing_replaces_the_candidate() {
    let master_key = [184u8; 32];
    let file = "16400000-0000-4000-8000-000000000052";
    let (_dir, sync_root, server, bridge, local) = regression_1640_synced_file(master_key, file).await;
    // Partly freed and fetched again at 1: the agreed content is the candidate now.
    bridge
        .db
        .record_fetched_version(file, 1, &sha256_hex(b"version one"))
        .unwrap();
    assert_eq!(
        bridge.db.fetched_content(file).unwrap(),
        Some((1, Some(sha256_hex(b"version one"))))
    );
    std::fs::write(&local, b"edited in the sync folder").unwrap();
    assert!(crate::watcher::dispatch_local_create(
        &bridge, &sync_root, &local, "notify"
    ));
    drain_upload_queue(&bridge, &sync_root).await;
    let contract = bridge.db.get_file_contract_state(file).unwrap().unwrap();
    assert_eq!(
        (contract.local_base_version, contract.local_hash),
        (2, Some(sha256_hex(b"edited in the sync folder")))
    );
    assert_eq!(
        bridge.db.fetched_content(file).unwrap(),
        None,
        "the sent file replaced it"
    );
    drop(server.finish());
}

/// A landing records what it SENT. Bytes saved after the staging copy was taken are not the
/// agreed content: they read as a change on the version that landed.
#[tokio::test]
async fn regression_1640_a_landing_whose_disk_changed_after_staging_is_not_agreed() {
    let master_key = [187u8; 32];
    let file = "16400000-0000-4000-8000-000000000053";
    let (_dir, sync_root, server, bridge, local) = regression_1640_synced_file(master_key, file).await;
    std::fs::write(&local, b"first edit").unwrap();
    assert!(crate::watcher::dispatch_local_create(
        &bridge, &sync_root, &local, "notify"
    ));
    // Saved again after the staging copy, before the upload ran; its event is not seen yet.
    std::fs::write(&local, b"second edit, after staging").unwrap();
    drain_upload_queue(&bridge, &sync_root).await;
    let contract = bridge.db.get_file_contract_state(file).unwrap().unwrap();
    assert_eq!(
        (contract.local_base_version, contract.local_hash),
        (2, Some(sha256_hex(b"first edit"))),
        "what was sent is the agreed content"
    );
    assert_eq!(
        sync_folder_content(&bridge.db, file, &local, None).unwrap(),
        SyncFolderContent::Changed { base: Some(2) },
        "the later bytes: a change on the version that landed, never agreed"
    );
    let state = server.finish();
    assert_eq!(state.latest_plaintext(file, master_key), b"first edit");
}

/// Self-review #1: a whole fetch is the agreed content only when no other fetch of the file
/// overlapped it: one already in flight when it started, or one that started or ended before its
/// transfer ended, could put bytes of another version around it. Its candidate then stays.
#[tokio::test]
async fn regression_1640_a_whole_fetch_is_agreed_only_when_no_other_fetch_overlapped_it() {
    let (_dir, _sync_root, server, bridge) = rule3_setup([188u8; 32]);
    let one = FetchedVersion {
        version: 1,
        hash: sha256_hex(b"version one"),
    };
    let two = FetchedVersion {
        version: 2,
        hash: sha256_hex(b"version two"),
    };
    let fetch = |file: &str, fetched: &FetchedVersion| {
        let ticket = bridge.begin_fetch(file);
        bridge
            .db
            .record_fetched_version(file, fetched.version, &fetched.hash)
            .unwrap();
        ticket
    };
    // Alone: the whole file is version 2's.
    regression_1640_new_placeholder(&bridge.db, "alone", "alone.txt");
    let whole = fetch("alone", &two);
    assert!(bridge.record_whole_fetch("alone", &two, &whole).unwrap());
    drop(whole);
    assert_eq!(
        bridge.db.get_file_contract_state("alone").unwrap().unwrap().local_hash,
        Some(two.hash.clone())
    );
    // A range fetch of version 1 still in flight when the whole fetch started.
    regression_1640_new_placeholder(&bridge.db, "earlier", "earlier.txt");
    let range = fetch("earlier", &one);
    let whole = fetch("earlier", &two);
    assert!(!bridge.record_whole_fetch("earlier", &two, &whole).unwrap());
    drop((range, whole));
    // A fetch that started and ended during the whole fetch.
    regression_1640_new_placeholder(&bridge.db, "during", "during.txt");
    let whole = fetch("during", &two);
    drop(fetch("during", &one));
    assert!(!bridge.record_whole_fetch("during", &two, &whole).unwrap());
    drop(whole);
    // The range fetch's bytes landed after the whole fetch's: the file is never agreed, and an
    // edit is sent on the older version.
    let spliced = _sync_root.join("spliced.txt");
    std::fs::write(&spliced, b"version two, with a range of version one").unwrap();
    for id in ["earlier", "during"] {
        assert_eq!(
            bridge.db.get_file_contract_state(id).unwrap().unwrap().local_hash,
            None,
            "{id}"
        );
        assert_eq!(
            bridge.db.fetched_content(id).unwrap(),
            Some((1, None)),
            "{id}: its candidate stays"
        );
        assert_eq!(
            sync_folder_content(&bridge.db, id, &spliced, None).unwrap(),
            SyncFolderContent::Changed { base: Some(1) },
            "{id}: not agreed, the older version"
        );
    }
    // A range fetch that starts after a whole fetch's agreed content: it joins it (lower or mixed).
    drop(fetch("alone", &one));
    assert_eq!(bridge.db.fetched_content("alone").unwrap(), Some((1, None)));
    assert_eq!(
        sync_folder_content(&bridge.db, "alone", &spliced, None).unwrap(),
        SyncFolderContent::Changed { base: Some(1) }
    );
    // Later, alone again.
    let whole = fetch("during", &two);
    assert!(bridge.record_whole_fetch("during", &two, &whole).unwrap());
    drop(server.finish());
}

/// Self-review #5: an entry whose platform view could not be read (open exclusively) is kept as
/// in use and retried, never "partly on disk" (which may be dismissed); and an entry whose changes
/// could not be copied never becomes "partly on disk" on a later read either.
#[cfg(unix)]
#[test]
fn regression_1640_an_unreadable_or_uncopied_file_is_never_dismissable() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let (db, sync_root) = regression_1640_edited_file_in_a_folder(dir.path());
    let rows = db.mark_subtree_trashing("folder-docs").unwrap();
    remove_pruned_entries(&db, &sync_root, &rows, &|_, _| LocalEntry::Unreadable);
    let kept = db.list_kept_copies().unwrap();
    assert_eq!(
        (kept.len(), kept[0].reason, kept[0].dismissable()),
        (1, KeptReason::InUse, false)
    );
    assert!(sync_root.join("docs").join("notes.txt").exists(), "kept");
    // Changes that could not be copied, then read as partly on disk.
    let dir = tempfile::tempdir().unwrap();
    let (db, sync_root) = regression_1640_edited_file_in_a_folder(dir.path());
    let rows = db.mark_subtree_trashing("folder-docs").unwrap();
    std::fs::set_permissions(&sync_root, std::fs::Permissions::from_mode(0o555)).unwrap();
    remove_pruned_entries(&db, &sync_root, &rows, &|_, _| LocalEntry::WhollyLocal);
    std::fs::set_permissions(&sync_root, std::fs::Permissions::from_mode(0o755)).unwrap();
    retry_kept_originals(&db, &sync_root, &|_, _| LocalEntry::PartlyLocal);
    let kept = db.list_kept_copies().unwrap();
    assert_eq!(
        (kept.len(), kept[0].reason, kept[0].dismissable()),
        (1, KeptReason::CopyFailed, false)
    );
}

/// Review-151d P2-2 (self-review #8 corrected): once a kept original is removed, the Office owner
/// file it left goes too, and nothing else: a file the sync folder ignores (a backup `~` file, a
/// partial download) was never synced, so it stays, and so does its folder (not created again on
/// the server). Once the folder is empty, it goes.
#[test]
fn regression_1640_a_kept_folder_keeps_the_files_it_never_synced() {
    let dir = tempfile::tempdir().unwrap();
    let (db, sync_root) = regression_1640_edited_file_in_a_folder(dir.path());
    let docs = sync_root.join("docs");
    std::fs::write(docs.join("~$tes.txt"), b"owner of notes.txt").unwrap();
    std::fs::write(docs.join("notes.txt~"), b"the editor's backup").unwrap();
    std::fs::write(docs.join("big.zip.part"), b"a download not finished").unwrap();
    let rows = db.delete_file_subtree("folder-docs").unwrap();
    remove_pruned_entries(&db, &sync_root, &rows, &|_, _| LocalEntry::PartlyLocal);
    assert!(docs.exists(), "precondition: kept for the file");
    retry_kept_originals(&db, &sync_root, &|_, _| LocalEntry::NothingLocal);
    assert!(!docs.join("notes.txt").exists(), "the original is removed");
    assert!(!docs.join("~$tes.txt").exists(), "and its owner file");
    assert_eq!(
        std::fs::read(docs.join("notes.txt~")).unwrap(),
        b"the editor's backup",
        "never synced: kept"
    );
    assert_eq!(
        std::fs::read(docs.join("big.zip.part")).unwrap(),
        b"a download not finished",
        "never synced: kept"
    );
    assert_eq!(
        db.list_removed_folders().unwrap(),
        vec!["docs".to_string()],
        "still the server's deletion"
    );
    std::fs::remove_file(docs.join("notes.txt~")).unwrap();
    std::fs::remove_file(docs.join("big.zip.part")).unwrap();
    retry_kept_originals(&db, &sync_root, &|_, _| LocalEntry::NothingLocal);
    assert!(!docs.exists(), "empty: removed");
    assert!(db.list_removed_folders().unwrap().is_empty());
}

/// Self-review #2: the candidate's column has its own name, so a database from a round-2 build
/// (whose `fetched_version` had DEFAULT 0) reads every row as not known.
#[test]
fn regression_1640_a_round_two_database_reads_every_row_as_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    drop(StateDb::open(&path).unwrap());
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "DROP TRIGGER IF EXISTS files_cloud_only_folds_agreed_into_candidate;
                 DROP TRIGGER IF EXISTS files_unknown_marks_after_insert;
                 DROP TRIGGER IF EXISTS files_unknown_marks_after_insert_any_status;
                 DROP TRIGGER IF EXISTS files_unknown_marks_after_update;
                 ALTER TABLE files DROP COLUMN fetched_min_version;
                 ALTER TABLE files ADD COLUMN fetched_version INTEGER NOT NULL DEFAULT 0;
                 INSERT INTO files (file_id, path, status) VALUES ('old', 'old.txt', 'local');",
        )
        .unwrap();
    }
    let db = StateDb::open(&path).unwrap();
    db.record_fetched_version("old", 2, "h2").unwrap();
    assert_eq!(
        db.fetched_content("old").unwrap(),
        None,
        "not known: no candidate over its bytes"
    );
    assert!(!db.mark_no_fetched_content("old").unwrap());
}

// ── Task 1640, review F2–F7: files kept from a remote deletion ──

/// Review F2: an original that waits (in use) is never removed once the file is restored on the
/// web (its own row, live again). When ANOTHER file moved to the place, the bytes there are still
/// the deleted file's (already copied): they go, as the engine's own remove (self-review #4).
#[cfg(unix)]
#[test]
fn regression_1640_a_retry_never_removes_a_file_the_server_lists_again() {
    use std::os::unix::fs::PermissionsExt;
    for case in ["restored on the web", "another file moved there"] {
        let dir = tempfile::tempdir().unwrap();
        let (db, sync_root) = regression_1640_edited_file_in_a_folder(dir.path());
        let docs = sync_root.join("docs");
        let rows = db.mark_subtree_trashing("folder-docs").unwrap();
        std::fs::set_permissions(&docs, std::fs::Permissions::from_mode(0o555)).unwrap();
        remove_pruned_entries(&db, &sync_root, &rows, &|_, _| LocalEntry::WhollyLocal);
        std::fs::set_permissions(&docs, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            db.is_kept_original_pending("docs/notes.txt").unwrap(),
            "{case}: precondition"
        );
        if case == "restored on the web" {
            db.set_status("folder-docs", FileStatus::Local).unwrap();
            db.set_status("file-notes", FileStatus::Local).unwrap();
        } else {
            db.delete_file_subtree("folder-docs").unwrap();
            db.upsert_file(&FileEntry {
                file_id: "fe32d353-1497-59d2-a4df-b3cbef768941".into(),
                path: "docs/notes.txt".into(),
                status: FileStatus::Local,
                size_bytes: 1,
                modified_at: 1,
                content_hash: None,
                remote_updated_at: 1,
                parent_id: None,
                item_kind: ItemKind::File,
            })
            .unwrap();
        }
        retry_kept_originals(&db, &sync_root, &|_, _| LocalEntry::WhollyLocal);
        if case == "restored on the web" {
            assert_eq!(
                std::fs::read(docs.join("notes.txt")).unwrap(),
                b"edited, never sent",
                "{case}: the live file stays"
            );
        } else {
            // Self-review #4: the bytes there are still the deleted file's (copied already); they go
            // so the other file's placeholder can be created there. The remove is the engine's own.
            assert!(!docs.join("notes.txt").exists(), "{case}: the deleted file's bytes go");
            assert_eq!(
                db.get_file_by_path("docs/notes.txt").unwrap().unwrap().file_id,
                "fe32d353-1497-59d2-a4df-b3cbef768941",
                "{case}"
            );
        }
        assert!(
            !db.is_kept_original_pending("docs/notes.txt").unwrap(),
            "{case}: nothing waits any more"
        );
        let kept = db.list_kept_copies().unwrap();
        assert_eq!(kept.len(), 1, "{case}: the copy's entry stays");
        assert!(kept[0].dismissable(), "{case}");
    }
}

/// Review F3: a folder deleted on the web stays on disk for a file kept in it (partly on
/// disk), and the scan does not create it again on the server. Once the file is resolved its
/// copy goes where it can upload (not into that folder), and the emptied folder goes.
#[tokio::test]
async fn regression_1640_a_folder_deleted_elsewhere_is_not_created_again_for_what_it_keeps() {
    let (_dir, sync_root, server, bridge) = rule3_setup([185u8; 32]);
    regression_1640_edited_file_in_a_folder_in(&bridge.db, &sync_root);
    let docs = sync_root.join("docs");
    let rows = bridge.db.delete_file_subtree("folder-docs").unwrap();
    remove_pruned_entries(&bridge.db, &sync_root, &rows, &|_, _| LocalEntry::PartlyLocal);
    assert!(docs.join("notes.txt").exists(), "precondition: kept in place");
    assert_eq!(bridge.ensure_local_folder(&sync_root, &docs), None, "not created again");
    assert!(
        bridge.db.get_file_by_path("docs").unwrap().is_none(),
        "no folder row, no CreateFolder"
    );
    // Now wholly on disk: copied where it can upload, the original and the folder removed.
    retry_kept_originals(&bridge.db, &sync_root, &|_, _| LocalEntry::WhollyLocal);
    assert_eq!(
        std::fs::read(sync_root.join("notes (this PC).txt")).unwrap(),
        b"edited, never sent"
    );
    assert!(!docs.exists(), "the emptied folder is removed");
    assert!(bridge.db.list_removed_folders().unwrap().is_empty());
    // Review F6: one entry, updated where it is.
    let feed: Vec<(String, String)> = version_conflict_feed_from_db(&bridge.db)
        .unwrap()
        .into_iter()
        .map(|entry| (entry.status, entry.file_name))
        .collect();
    assert_eq!(
        feed,
        vec![("kept as a copy".to_string(), "notes (this PC).txt".to_string())]
    );
    drop(server.finish());
}

/// Review F3: a folder deleted on the web that the person put something new into is wanted
/// again: the scan creates it.
#[tokio::test]
async fn regression_1640_a_folder_deleted_elsewhere_with_something_new_is_created_again() {
    let (_dir, sync_root, server, bridge) = rule3_setup([186u8; 32]);
    regression_1640_edited_file_in_a_folder_in(&bridge.db, &sync_root);
    let docs = sync_root.join("docs");
    let rows = bridge.db.delete_file_subtree("folder-docs").unwrap();
    remove_pruned_entries(&bridge.db, &sync_root, &rows, &|_, _| LocalEntry::PartlyLocal);
    std::fs::write(docs.join("new.txt"), b"made after the delete").unwrap();
    assert!(bridge.ensure_local_folder(&sync_root, &docs).is_some(), "created again");
    assert!(bridge.db.list_removed_folders().unwrap().is_empty());
    drop(server.finish());
}

/// Review F4: a web trash copies an edit that Word holds open; then the trash is emptied. The
/// second remote op copies nothing again: one copy, one entry.
#[cfg(unix)]
#[test]
fn regression_1640_a_second_remote_op_copies_nothing_again() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let (db, sync_root) = regression_1640_edited_file_in_a_folder(dir.path());
    let docs = sync_root.join("docs");
    let trashed = db.mark_subtree_trashing("folder-docs").unwrap();
    std::fs::set_permissions(&docs, std::fs::Permissions::from_mode(0o555)).unwrap();
    remove_pruned_entries(&db, &sync_root, &trashed, &|_, _| LocalEntry::WhollyLocal);
    let emptied = db.delete_file_subtree("folder-docs").unwrap();
    assert!(
        emptied.iter().any(|row| row.path == "docs/notes.txt"),
        "precondition: the file again"
    );
    remove_pruned_entries(&db, &sync_root, &emptied, &|_, _| LocalEntry::WhollyLocal);
    std::fs::set_permissions(&docs, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(sync_root.join("notes (this PC).txt").exists());
    assert!(
        !sync_root.join("notes (this PC 2).txt").exists(),
        "the same bytes are not copied twice"
    );
    assert_eq!(db.list_kept_copies().unwrap().len(), 1, "one entry");
}

/// Review F5, F6: an open file in sync (nothing to keep) says it waits to close, then goes
/// once removed; one whose changes could not be copied says so, and is resolved where it is.
#[cfg(unix)]
#[test]
fn regression_1640_a_kept_file_says_why_and_is_resolved_where_it_is() {
    use std::os::unix::fs::PermissionsExt;
    let feed = |db: &StateDb| -> Vec<(String, String, String)> {
        version_conflict_feed_from_db(db)
            .unwrap()
            .into_iter()
            .map(|entry| (entry.status, entry.detail, entry.action))
            .collect()
    };
    // Open, in sync.
    let dir = tempfile::tempdir().unwrap();
    let (db, sync_root) = regression_1640_edited_file_in_a_folder(dir.path());
    let docs = sync_root.join("docs");
    let rows = db.mark_subtree_trashing("folder-docs").unwrap();
    std::fs::set_permissions(&docs, std::fs::Permissions::from_mode(0o555)).unwrap();
    remove_pruned_entries(&db, &sync_root, &rows, &|_, _| LocalEntry::InSync);
    let open = feed(&db);
    assert_eq!(open.len(), 1);
    assert_eq!((open[0].0.as_str(), open[0].2.as_str()), ("waiting to close", "none"));
    assert!(open[0].1.contains("open on this PC"), "{}", open[0].1);
    std::fs::set_permissions(&docs, std::fs::Permissions::from_mode(0o755)).unwrap();
    retry_kept_originals(&db, &sync_root, &|_, _| LocalEntry::InSync);
    assert!(!docs.join("notes.txt").exists(), "removed once closed");
    assert!(feed(&db).is_empty(), "nothing was kept: the entry goes");
    // Changes that could not be copied (no room for the copy).
    let dir = tempfile::tempdir().unwrap();
    let (db, sync_root) = regression_1640_edited_file_in_a_folder(dir.path());
    let rows = db.mark_subtree_trashing("folder-docs").unwrap();
    std::fs::set_permissions(&sync_root, std::fs::Permissions::from_mode(0o555)).unwrap();
    remove_pruned_entries(&db, &sync_root, &rows, &|_, _| LocalEntry::WhollyLocal);
    std::fs::set_permissions(&sync_root, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        std::fs::read(sync_root.join("docs").join("notes.txt")).unwrap(),
        b"edited, never sent",
        "kept where it is"
    );
    let failed = feed(&db);
    assert_eq!(
        (failed.len(), failed[0].0.as_str(), failed[0].2.as_str()),
        (1, "kept in place", "none")
    );
    assert!(failed[0].1.contains("could not be copied"), "{}", failed[0].1);
    retry_kept_originals(&db, &sync_root, &|_, _| LocalEntry::WhollyLocal);
    let copied = feed(&db);
    assert_eq!(
        (copied.len(), copied[0].0.as_str(), copied[0].2.as_str()),
        (1, "kept as a copy", "dismiss")
    );
}

/// Review F7: a file kept where it is because only part of it was on this PC can be dismissed
/// (no tick can resolve it when the rest is gone); the file stays, and is no longer held back.
#[tokio::test]
async fn regression_1640_a_file_kept_partly_on_disk_can_be_dismissed() {
    let (_dir, sync_root, server, bridge) = rule3_setup([189u8; 32]);
    let db = &bridge.db;
    regression_1640_edited_file_in_a_folder_in(db, &sync_root);
    let docs = sync_root.join("docs");
    let rows = db.delete_file_subtree("folder-docs").unwrap();
    remove_pruned_entries(db, &sync_root, &rows, &|_, _| LocalEntry::PartlyLocal);
    let feed = version_conflict_feed_from_db(db).unwrap();
    assert_eq!((feed.len(), feed[0].action.as_str()), (1, "dismiss"));
    let id: i64 = feed[0].id.strip_prefix("kept:").unwrap().parse().unwrap();
    assert!(db.dismiss_kept_copy(id).unwrap());
    assert!(docs.join("notes.txt").exists(), "the file stays");
    assert!(
        !db.is_kept_original_pending("docs/notes.txt").unwrap(),
        "and is no longer held back"
    );
    assert!(version_conflict_feed_from_db(db).unwrap().is_empty());
    // Review-151d P2-1: the folder the server deleted is still not created again for it.
    retry_kept_originals(db, &sync_root, &|_, _| LocalEntry::PartlyLocal);
    assert_eq!(bridge.ensure_local_folder(&sync_root, &docs), None, "not created again");
    assert!(
        db.get_file_by_path("docs").unwrap().is_none(),
        "no folder row, no CreateFolder"
    );
    // Once the person removes the file, the folder goes and nothing is left of the entry.
    std::fs::remove_file(docs.join("notes.txt")).unwrap();
    retry_kept_originals(db, &sync_root, &|_, _| LocalEntry::PartlyLocal);
    assert!(!docs.exists());
    assert!(
        !db.is_kept_original("docs/notes.txt").unwrap(),
        "the tombstone goes with the file"
    );
    drop(server.finish());
}

// ── Task 1640 follow-up: a new file is scanned again once its row exists ──

/// Every upload queued for `rel_path`, under any file id.
pub(super) fn regression_1640_uploads_queued_for(bridge: &EngineBridge, rel_path: &str) -> usize {
    bridge
        .db
        .list_due_operations(i64::MAX)
        .unwrap()
        .iter()
        .filter(|op| op.target_path.as_deref() == Some(rel_path))
        .count()
}

/// A new file the scan queued is a tracked file from then on. A scan while its upload is
/// queued queues nothing more (under any id); once it landed, an atomic save of it (no change
/// notification) is seen by the next scan and sent on the version that landed. Before, the
/// scan kept skipping the path (its `in_flight` set) until it left the disk, so that save
/// waited for a restart.
#[tokio::test]
async fn regression_1640_a_new_file_is_scanned_again_once_its_row_exists() {
    let master_key = [190u8; 32];
    let (_dir, sync_root, server, bridge) = rule3_setup(master_key);
    let local = sync_root.join("fresh.txt");
    std::fs::write(&local, b"made on this PC").unwrap();
    crate::watcher::run_one_scan(&bridge, &sync_root);
    assert_eq!(
        regression_1640_uploads_queued_for(&bridge, "fresh.txt"),
        1,
        "precondition: queued as a new file"
    );
    crate::watcher::run_one_scan(&bridge, &sync_root);
    assert_eq!(
        regression_1640_uploads_queued_for(&bridge, "fresh.txt"),
        1,
        "no second upload while it is queued"
    );
    drain_upload_queue(&bridge, &sync_root).await;
    let row = bridge.db.get_file_by_path("fresh.txt").unwrap().unwrap();
    assert_eq!(row.status, FileStatus::Local, "precondition: landed");
    crate::watcher::run_one_scan(&bridge, &sync_root);
    assert_eq!(
        regression_1640_uploads_queued_for(&bridge, "fresh.txt"),
        0,
        "landed, unchanged: nothing"
    );
    // An atomic save: the new bytes written elsewhere, then renamed over the file.
    let saved = sync_root.join("fresh.txt.tmp");
    std::fs::write(&saved, b"saved by an editor").unwrap();
    std::fs::rename(&saved, &local).unwrap();
    crate::watcher::run_one_scan(&bridge, &sync_root);
    let ops = bridge.db.list_operations_for_file(&row.file_id).unwrap();
    assert_eq!(
        (regression_1640_uploads_queued_for(&bridge, "fresh.txt"), ops.len()),
        (1, 1),
        "the save is queued, as an edit of the file"
    );
    assert_eq!(ops[0].base_version, Some(1), "on the version that landed");
    drain_upload_queue(&bridge, &sync_root).await;
    let state = server.finish();
    assert_eq!(state.latest_plaintext(&row.file_id, master_key), b"saved by an editor");
}

// ── Task 2244: a pass that holds edits logs one count, not a line per file ──
