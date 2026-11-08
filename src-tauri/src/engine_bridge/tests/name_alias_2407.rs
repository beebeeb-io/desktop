//! Task 2407 (core 2404): every sync-folder site that creates, replaces, renames, copies or deletes an entry by name
//! asks core's 8.3-alias check first, and on a conflict takes its conflict path. Here the check's answer is stubbed
//! (`name_alias::stub`), so each site's decision runs on any host: on Linux the stubbed path is an ordinary entry that
//! stands in for the different file the name reaches on Windows, and a site that ignores the answer creates, replaces
//! or deletes it. The Windows tests (`name_alias_2407_windows.rs`) prove the same against real 8.3 names.

use super::*;
use crate::name_alias::{NameConflict, stub};

fn conflict_in_chain(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause.is::<NameConflict>()
            || cause
                .downcast_ref::<std::io::Error>()
                .and_then(std::io::Error::get_ref)
                .is_some_and(|inner| inner.is::<NameConflict>())
    })
}

// ── Remote deletions (`landing.rs`): never the entry a name reaches ──

#[test]
fn a_remote_deletion_never_deletes_the_entry_an_alias_reaches() {
    let dir = tempfile::tempdir().unwrap();
    let (db, sync_root) = super::reg_1640::regression_1640_edited_file_in_a_folder(dir.path());
    let notes = sync_root.join("docs").join("notes.txt");
    let _alias = stub::alias(&notes);
    let rows = db.mark_subtree_trashing("folder-docs").unwrap();
    remove_pruned_entries(&db, &sync_root, &rows, &|_, _| LocalEntry::InSync);
    assert_eq!(
        std::fs::read(&notes).unwrap(),
        b"edited, never sent",
        "the entry the name reaches is untouched"
    );
    assert!(
        db.list_kept_copies().unwrap().is_empty(),
        "nothing of this file is on disk to keep"
    );
}

#[test]
fn a_remote_deletion_never_deletes_below_a_folder_an_alias_reaches() {
    let dir = tempfile::tempdir().unwrap();
    let (db, sync_root) = super::reg_1640::regression_1640_edited_file_in_a_folder(dir.path());
    let _alias = stub::alias(&sync_root.join("docs"));
    let rows = db.mark_subtree_trashing("folder-docs").unwrap();
    remove_pruned_entries(&db, &sync_root, &rows, &|_, _| LocalEntry::InSync);
    assert_eq!(
        std::fs::read(sync_root.join("docs").join("notes.txt")).unwrap(),
        b"edited, never sent"
    );
}

/// A check that cannot finish never deletes: the file is kept where it is, for the retry.
#[test]
fn a_remote_deletion_whose_check_fails_keeps_the_file_for_the_retry() {
    let dir = tempfile::tempdir().unwrap();
    let (db, sync_root) = super::reg_1640::regression_1640_edited_file_in_a_folder(dir.path());
    let notes = sync_root.join("docs").join("notes.txt");
    let _failing = stub::failing(&notes);
    let rows = db.mark_subtree_trashing("folder-docs").unwrap();
    remove_pruned_entries(&db, &sync_root, &rows, &|_, _| LocalEntry::InSync);
    assert_eq!(std::fs::read(&notes).unwrap(), b"edited, never sent");
    let kept = db.list_kept_copies().unwrap();
    assert_eq!(kept.len(), 1, "one entry waits for the retry");
    assert_eq!((kept[0].copy_path.as_str(), kept[0].original_removed), ("", false));
}

#[test]
fn the_kept_original_retry_never_removes_the_entry_an_alias_reaches() {
    let dir = tempfile::tempdir().unwrap();
    let (db, sync_root) = super::reg_1640::regression_1640_edited_file_in_a_folder(dir.path());
    let notes = sync_root.join("docs").join("notes.txt");
    let rows = db.mark_subtree_trashing("folder-docs").unwrap();
    remove_pruned_entries(&db, &sync_root, &rows, &|_, _| LocalEntry::Unreadable);
    assert_eq!(
        db.list_kept_copies().unwrap().len(),
        1,
        "precondition: the original waits for the retry"
    );
    let _alias = stub::alias(&notes);
    retry_kept_originals(&db, &sync_root, &|_, _| LocalEntry::InSync);
    assert_eq!(
        std::fs::read(&notes).unwrap(),
        b"edited, never sent",
        "the entry the name reaches is untouched"
    );
}

#[test]
fn an_emptied_kept_folder_is_never_removed_through_an_alias() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    let sync_root = dir.path().join("sync-root");
    let folder = sync_root.join("OLDFOL~1");
    std::fs::create_dir_all(&folder).unwrap();
    db.record_removed_folder("OLDFOL~1", 1_700_000_000).unwrap();
    let _alias = stub::alias(&folder);
    landing::remove_empty_kept_folders(&db, &sync_root);
    assert!(folder.is_dir(), "the empty folder the name reaches is untouched");
}

#[test]
fn an_office_owner_file_an_alias_reaches_is_never_removed() {
    let dir = tempfile::tempdir().unwrap();
    let owner = dir.path().join("~$REPO~1.DOC");
    std::fs::write(&owner, b"another document's owner file").unwrap();
    let _alias = stub::alias(&owner);
    landing::remove_office_owner_files(&dir.path().join("REPO~1.DOC"));
    assert!(owner.is_file(), "the owner file the name reaches is untouched");
}

#[test]
fn scaffold_folders_are_never_removed_through_an_alias() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("PHOTOS~1");
    std::fs::create_dir_all(&folder).unwrap();
    let _alias = stub::alias(&folder);
    landing::remove_scaffold_dirs(dir.path(), &["PHOTOS~1".to_string()]);
    assert!(folder.is_dir(), "the empty folder the name reaches is untouched");
}

// ── Conflicts (`conflicts.rs`) ──

fn conflicted_row(bridge: &EngineBridge, file_id: &str, path: &str) -> FileEntry {
    bridge
        .db
        .upsert_file(&FileEntry {
            file_id: file_id.into(),
            path: path.into(),
            status: FileStatus::Conflict,
            size_bytes: 20,
            modified_at: 100,
            content_hash: Some("local-conflict-hash".into()),
            remote_updated_at: 90,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
    bridge.db.get_file(file_id).unwrap().unwrap()
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Keep Both renames "the local copy" first: through an alias that is a different file, renamed away from its name.
#[tokio::test]
async fn keep_both_never_renames_the_entry_an_alias_reaches() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync-root");
    std::fs::create_dir_all(&sync_root).unwrap();
    let original = sync_root.join("CONFLI~1.TXT");
    std::fs::write(&original, b"a different file").unwrap();
    let file_id = "bbbb0000-0000-4000-8000-000000002407";
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://127.0.0.1:9".into(), [15u8; 32]);
    let entry = conflicted_row(&bridge, file_id, "CONFLI~1.TXT");
    let _alias = stub::alias(&original);

    let error = bridge.auto_resolve_keep_both(&sync_root, &entry).await.unwrap_err();

    assert!(
        conflict_in_chain(&error),
        "a name conflict, not another failure: {error:#}"
    );
    assert_eq!(std::fs::read(&original).unwrap(), b"a different file");
    assert_eq!(
        names_in(&sync_root),
        ["CONFLI~1.TXT"],
        "nothing renamed, nothing written"
    );
    assert_eq!(
        bridge.db.get_file(file_id).unwrap().unwrap().status,
        FileStatus::Conflict,
        "still the person's to resolve"
    );
}

/// The conflict copy's name already holds an earlier conflict copy (a second conflict the same day): it is never
/// replaced; the local bytes take the next free name.
#[tokio::test]
async fn keep_both_never_replaces_an_earlier_conflict_copy() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync-root");
    std::fs::create_dir_all(&sync_root).unwrap();
    std::fs::write(sync_root.join("notes.txt"), b"local conflict bytes").unwrap();
    let host = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "device".into());
    let date = chrono::Utc::now().format("%Y-%m-%d");
    let earlier = sync_root.join(format!("notes (conflict - {host} - {date}).txt"));
    std::fs::write(&earlier, b"the earlier conflict copy").unwrap();
    let file_id = "bbbb0000-0000-4000-8000-000000002408";
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://127.0.0.1:9".into(), [15u8; 32]);
    let entry = conflicted_row(&bridge, file_id, "notes.txt");

    // The remote hydrate fails (nothing listens): the local bytes stay in their copy, the row is Error.
    let _ = bridge.auto_resolve_keep_both(&sync_root, &entry).await.unwrap_err();

    assert_eq!(
        std::fs::read(&earlier).unwrap(),
        b"the earlier conflict copy",
        "the earlier copy is never replaced"
    );
    let next = sync_root.join(format!("notes (conflict - {host} - {date}) 2.txt"));
    assert_eq!(
        std::fs::read(&next).unwrap(),
        b"local conflict bytes",
        "the local bytes take the next free name"
    );
}

/// Keep Mine uploads "the local copy": through an alias that is a different file's bytes, never uploaded as this file's
/// version. Refused before anything is staged or sent.
#[tokio::test]
async fn keep_mine_never_uploads_the_file_an_alias_reaches() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync-root");
    std::fs::create_dir_all(&sync_root).unwrap();
    let original = sync_root.join("CONFLI~1.TXT");
    std::fs::write(&original, b"a different file").unwrap();
    let file_id = "bbbb0000-0000-4000-8000-000000002410";
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://127.0.0.1:9".into(), [15u8; 32]);
    conflicted_row(&bridge, file_id, "CONFLI~1.TXT");
    let _alias = stub::alias(&original);

    let error = bridge.resolve_keep_mine(file_id, &sync_root).await.unwrap_err();

    assert!(
        conflict_in_chain(&error),
        "a name conflict, not another failure: {error:#}"
    );
    assert_eq!(
        bridge.db.get_file(file_id).unwrap().unwrap().status,
        FileStatus::Conflict
    );
}

/// The conflict window never shows a different file's content as the local side.
#[tokio::test]
async fn the_conflict_preview_never_shows_the_file_an_alias_reaches() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync-root");
    std::fs::create_dir_all(&sync_root).unwrap();
    let original = sync_root.join("CONFLI~1.TXT");
    std::fs::write(&original, b"a different file").unwrap();
    let file_id = "bbbb0000-0000-4000-8000-000000002411";
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://127.0.0.1:9".into(), [15u8; 32]);
    conflicted_row(&bridge, file_id, "CONFLI~1.TXT");
    let _alias = stub::alias(&original);

    let preview = bridge.conflict_content_preview(file_id, &sync_root).await.unwrap();

    assert_eq!(
        (preview.local.text, preview.local.size_bytes),
        (None, None),
        "nothing of the other file is read"
    );
    assert!(
        preview
            .local
            .unavailable_reason
            .is_some_and(|reason| !reason.contains("CONFLI"))
    );
}

// ── Hydration into the sync folder (`hydrate.rs` → `meta.rs`) ──

/// A download never writes through a name that reaches a different file: refused before the download, and the file
/// keeps its bytes.
#[tokio::test]
async fn a_hydrate_never_writes_through_an_alias() {
    let file = "11111111-2222-4333-8444-555555552407";
    let (_dir, sync_root, _server, bridge, _local) =
        super::reg_1640::regression_1640_synced_file([9u8; 32], file).await;
    let dest = sync_root.join("REPORT~1.TXT");
    std::fs::write(&dest, b"a different file").unwrap();
    let _alias = stub::alias(&dest);

    let error = bridge
        .hydrate_file(file, &dest, &[sync_root.as_path()])
        .await
        .unwrap_err();

    assert!(
        conflict_in_chain(&error),
        "a name conflict, not another failure: {error:#}"
    );
    assert_eq!(
        std::fs::read(&dest).unwrap(),
        b"a different file",
        "never truncated or replaced"
    );
}

// ── A request row's edit, saved as a new sibling (`upload.rs`) ──

/// The edited copy is written beside the request row, in its folder: never into a different folder the folder's name
/// reaches.
#[tokio::test]
async fn a_request_edit_is_never_copied_into_a_folder_an_alias_reaches() {
    use crate::owner_keys::OwnedKeyRecord;
    use beebeeb_core::kdf::{MasterKey, derive_file_key};
    const REQUEST: &str = "0f000000-0000-4000-8000-000000002407";
    const FOLDER: &str = "40000000-0000-4000-8000-000000002407";
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync");
    std::fs::create_dir_all(sync_root.join("INBOX~1")).unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    seed_folder_row(&bridge, FOLDER, "INBOX~1");
    seed_bridge_row(
        &bridge,
        REQUEST,
        "INBOX~1/report.txt",
        Some(FOLDER),
        FileStatus::Local,
        4,
    );
    let derived = derive_file_key(&MasterKey::from_bytes([7; 32]), REQUEST.as_bytes());
    let name = beebeeb_core::encrypt::encrypt_name_for(
        &derived,
        "report.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    bridge
        .db()
        .put_owned_key_record(&OwnedKeyRecord {
            file_id: REQUEST.into(),
            name_encrypted: name.clone(),
            request_row: true,
            ..Default::default()
        })
        .unwrap();
    let payload = dir.path().join("payload.bin");
    std::fs::write(&payload, b"edit").unwrap();
    let op = PendingOperation {
        op_id: "op-2407-request-edit".into(),
        kind: OperationKind::UploadVersion,
        file_id: Some(REQUEST.into()),
        parent_id: Some(FOLDER.into()),
        target_path: Some("INBOX~1/report.txt".into()),
        metadata_json: Some(serde_json::json!({"name_encrypted": name}).to_string()),
        payload_path: Some(payload.to_string_lossy().into_owned()),
        base_version: None,
        base_object_version_id: None,
        attempts: 0,
        max_attempts: 25,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: 0,
        updated_at: 0,
    };
    let _alias = stub::alias(&sync_root.join("INBOX~1"));

    let error = anyhow::Error::from(
        bridge
            .upload_version(&op, None, &sync_root, &mut Vec::new())
            .await
            .unwrap_err(),
    );

    assert!(
        conflict_in_chain(&error),
        "a name conflict, not another failure: {error:#}"
    );
    assert!(
        names_in(&sync_root.join("INBOX~1")).is_empty(),
        "nothing is copied into the folder the name reaches"
    );
    assert!(
        bridge
            .db()
            .list_files()
            .unwrap()
            .iter()
            .all(|row| !row.path.contains("(edited")),
        "no create is queued"
    );
}

// ── Delta round (review of #237) ──

/// The kept-original retry never removes Office owner files through a FOLDER whose name reaches a different folder:
/// the owner names are checked from the sync root, not from the original's own folder.
#[test]
fn the_retry_never_removes_owner_files_through_a_folder_an_alias_reaches() {
    let dir = tempfile::tempdir().unwrap();
    let (db, sync_root) = super::reg_1640::regression_1640_edited_file_in_a_folder(dir.path());
    let rows = db.mark_subtree_trashing("folder-docs").unwrap();
    remove_pruned_entries(&db, &sync_root, &rows, &|_, _| LocalEntry::Unreadable);
    assert_eq!(
        db.list_kept_copies().unwrap().len(),
        1,
        "precondition: the original waits for the retry"
    );
    let owner = sync_root.join("docs").join("~$notes.txt");
    std::fs::write(&owner, b"the other folder's open document").unwrap();
    let _alias = stub::alias(&sync_root.join("docs"));

    retry_kept_originals(&db, &sync_root, &|_, _| LocalEntry::InSync);

    assert!(
        owner.is_file(),
        "the owner file in the folder the name reaches is untouched"
    );
    assert_eq!(
        std::fs::read(sync_root.join("docs").join("notes.txt")).unwrap(),
        b"edited, never sent"
    );
}

/// A Windows reclaim (free up space, a selective-sync exclusion) dehydrates a row's file only at its own place.
#[test]
fn a_reclaim_never_targets_the_file_an_alias_reaches() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let _alias = stub::alias(&root.join("Docs").join("REPORT~1.PDF"));
    let _folder = stub::alias(&root.join("PHOTOS~1"));
    let _failing = stub::failing(&root.join("Locked"));
    assert_eq!(crate::name_alias::reclaim_place(root, "Docs/REPORT~1.PDF"), None);
    assert_eq!(crate::name_alias::reclaim_place(root, "/PHOTOS~1/a.jpg"), None);
    assert_eq!(
        crate::name_alias::reclaim_place(root, "Locked/a.txt"),
        None,
        "a check that fails keeps it local"
    );
    assert_eq!(crate::name_alias::reclaim_place(root, ""), None);
    assert_eq!(
        crate::name_alias::reclaim_place(root, "/Docs/Report for 2026.pdf/"),
        Some(root.join("Docs").join("Report for 2026.pdf"))
    );
}

/// A request row's edit whose staged copy is gone reads the row's own file only: never a different file its name
/// reaches, whose bytes would be copied and uploaded.
#[tokio::test]
async fn a_request_edit_never_reads_the_file_an_alias_reaches() {
    use crate::owner_keys::OwnedKeyRecord;
    use beebeeb_core::kdf::{MasterKey, derive_file_key};
    const REQUEST: &str = "0f000000-0000-4000-8000-000000002412";
    const FOLDER: &str = "40000000-0000-4000-8000-000000002412";
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync");
    std::fs::create_dir_all(sync_root.join("Inbox")).unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    seed_folder_row(&bridge, FOLDER, "Inbox");
    seed_bridge_row(
        &bridge,
        REQUEST,
        "Inbox/REPORT~1.TXT",
        Some(FOLDER),
        FileStatus::Local,
        4,
    );
    let derived = derive_file_key(&MasterKey::from_bytes([7; 32]), REQUEST.as_bytes());
    let name = beebeeb_core::encrypt::encrypt_name_for(
        &derived,
        "report.txt",
        None,
        beebeeb_core::chunk_stream::ExpectedChunkFormat::Unrecorded,
    )
    .unwrap();
    bridge
        .db()
        .put_owned_key_record(&OwnedKeyRecord {
            file_id: REQUEST.into(),
            name_encrypted: name.clone(),
            request_row: true,
            ..Default::default()
        })
        .unwrap();
    let source = sync_root.join("Inbox").join("REPORT~1.TXT");
    std::fs::write(&source, b"a different private file").unwrap();
    let op = PendingOperation {
        op_id: "op-2407-request-source".into(),
        kind: OperationKind::UploadVersion,
        file_id: Some(REQUEST.into()),
        parent_id: Some(FOLDER.into()),
        target_path: Some("Inbox/REPORT~1.TXT".into()),
        metadata_json: Some(serde_json::json!({"name_encrypted": name}).to_string()),
        // The staged copy is gone.
        payload_path: Some(dir.path().join("gone.bin").to_string_lossy().into_owned()),
        base_version: None,
        base_object_version_id: None,
        attempts: 0,
        max_attempts: 25,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: 0,
        updated_at: 0,
    };
    let _alias = stub::alias(&source);

    let error = anyhow::Error::from(
        bridge
            .upload_version(&op, None, &sync_root, &mut Vec::new())
            .await
            .unwrap_err(),
    );

    assert!(
        conflict_in_chain(&error),
        "a name conflict, not another failure: {error:#}"
    );
    assert_eq!(
        names_in(&sync_root.join("Inbox")),
        ["REPORT~1.TXT"],
        "no copy of the other file is made"
    );
    assert!(
        bridge
            .db()
            .list_files()
            .unwrap()
            .iter()
            .all(|row| !row.path.contains("(edited")),
        "no create is queued"
    );
}

/// Keep Both off Windows moves by hard link, then removes the old name. When that removal fails, the new link is
/// removed again and the error returned: no conflict copy is left, and no further numbered links are made.
#[cfg(not(target_os = "windows"))]
#[tokio::test]
async fn keep_both_whose_removal_after_the_link_fails_leaves_no_extra_links() {
    let dir = tempfile::tempdir().unwrap();
    let sync_root = dir.path().join("sync-root");
    std::fs::create_dir_all(&sync_root).unwrap();
    std::fs::write(sync_root.join("notes.txt"), b"local conflict bytes").unwrap();
    let file_id = "bbbb0000-0000-4000-8000-000000002412";
    let bridge = test_bridge_with_api(&dir.path().join("state.db"), "http://127.0.0.1:9".into(), [15u8; 32]);
    let entry = conflicted_row(&bridge, file_id, "notes.txt");
    super::super::fs::FAIL_REMOVE_AFTER_LINK.with(|fail| fail.set(true));

    let result = bridge.auto_resolve_keep_both(&sync_root, &entry).await;
    super::super::fs::FAIL_REMOVE_AFTER_LINK.with(|fail| fail.set(false));

    let error = result.unwrap_err();
    assert!(
        error.chain().any(|cause| cause.to_string().contains("ermission")),
        "the removal's own error: {error:#}"
    );
    assert_eq!(
        names_in(&sync_root),
        ["notes.txt"],
        "no conflict copy and no extra links are left"
    );
    assert_eq!(
        std::fs::read(sync_root.join("notes.txt")).unwrap(),
        b"local conflict bytes"
    );
}
