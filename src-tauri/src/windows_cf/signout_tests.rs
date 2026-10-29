//! Real Windows filesystem/CFAPI and SQLite tests; no provider registration or account.
use super::*;
use crate::state_db::{FileContractState, FileEntry, FileStatus, ItemKind, Namespace, PinState};

struct Fixture {
    db: StateDb,
    root: std::path::PathBuf,
    temp: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let db = StateDb::open(temp.path().join("state.db")).unwrap();
        Self { db, root, temp }
    }

    /// A row with known content (task 2191: not a file of unknown content), as before.
    fn track(&self, path: &str, kind: ItemKind, status: FileStatus) {
        self.track_unknown(path, kind, status);
        self.db.mark_no_fetched_content(path).unwrap();
    }

    /// Task 2191: a row of unknown content (synced before task 1640).
    fn track_unknown(&self, path: &str, kind: ItemKind, status: FileStatus) {
        self.db
            .upsert_file(&FileEntry {
                file_id: path.into(),
                path: path.into(),
                status,
                size_bytes: 0,
                modified_at: 0,
                content_hash: None,
                remote_updated_at: 1,
                parent_id: None,
                item_kind: kind.clone(),
            })
            .unwrap();
        // upsert_file deliberately does not write item_kind; the engine uses
        // the contract setter too when a local CreateFolder has completed.
        self.db
            .set_file_contract_state(&FileContractState {
                file_id: path.into(),
                namespace: Namespace::MyFiles,
                parent_id: None,
                shared_root_id: None,
                share_id: None,
                owner_email: None,
                permission_bits: 1,
                item_kind: kind,
                content_type: None,
                current_version: 1,
                current_object_version_id: None,
                local_base_version: 1,
                local_hash: None,
                cache_path: None,
                cache_bytes: 0,
                pin_state: PinState::Inherit,
                inherited_pin_state: PinState::Unpinned,
                last_sync_at: 1,
            })
            .unwrap();
    }

    fn folder(&self, path: &str) {
        std::fs::create_dir(self.root.join(path)).unwrap();
        self.track(path, ItemKind::Folder, FileStatus::Local);
    }
}

#[test]
fn synced_plain_directories_sign_out_and_clear_rows() {
    let f = Fixture::new();
    f.folder("local");
    f.folder("local/nested");
    assert_eq!(f.db.list_files().unwrap().len(), 2);
    let result = purge(&f.db, Some(&f.root), &UnknownFiles::Delete);
    assert!(result.is_ok(), "synced plain directories must sign out: {result:?}");
    assert_eq!(std::fs::read_dir(&f.root).unwrap().count(), 0);
    assert_eq!(f.db.list_files().unwrap().len(), 0);
}

#[test]
fn plain_directory_unsynced_child_preserves_bytes_and_rows() {
    let f = Fixture::new();
    f.folder("local");
    let child = f.root.join("local/unsynced.txt");
    std::fs::write(&child, b"unsynced bytes").unwrap();
    let error = purge(&f.db, Some(&f.root), &UnknownFiles::Delete).unwrap_err();
    assert!(
        error.to_string().contains("Unsynced files remain"),
        "must inspect children: {error:?}"
    );
    assert_eq!(std::fs::read(child).unwrap(), b"unsynced bytes");
    assert_eq!(f.db.list_files().unwrap().len(), 1);
}

#[test]
fn tracked_plain_file_still_requires_cloud_identity() {
    let f = Fixture::new();
    f.folder("local");
    let child = f.root.join("local/tracked.txt");
    std::fs::write(&child, b"potentially unsynced bytes").unwrap();
    f.track("local/tracked.txt", ItemKind::File, FileStatus::Local);
    assert!(purge(&f.db, Some(&f.root), &UnknownFiles::Delete).is_err());
    assert_eq!(std::fs::read(child).unwrap(), b"potentially unsynced bytes");
    assert_eq!(f.db.list_files().unwrap().len(), 2);
}

#[test]
fn unsynced_plain_directory_preserves_rows() {
    let f = Fixture::new();
    f.folder("local");
    f.track("local", ItemKind::Folder, FileStatus::Uploading);
    let error = purge(&f.db, Some(&f.root), &UnknownFiles::Delete).unwrap_err();
    assert!(error.to_string().contains("Pending changes remain"));
    assert!(f.root.join("local").is_dir());
    assert_eq!(f.db.list_files().unwrap().len(), 1);
}

#[test]
fn directory_at_file_row_is_not_accepted_as_synced_folder() {
    let f = Fixture::new();
    f.folder("local");
    f.track("local", ItemKind::File, FileStatus::Local);
    let error = purge(&f.db, Some(&f.root), &UnknownFiles::Delete).unwrap_err();
    assert!(
        error.to_string().contains("Item type changed"),
        "must reject the DB/disk kind mismatch before traversal: {error:?}"
    );
    assert!(f.root.join("local").is_dir());
    assert_eq!(f.db.list_files().unwrap().len(), 1);
}

#[test]
fn directory_junction_is_not_traversed_or_removed() {
    let f = Fixture::new();
    let outside = f.temp.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("precious.txt"), b"outside bytes").unwrap();
    let link = f.root.join("link");
    let output = std::process::Command::new("cmd.exe")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(&outside)
        .output()
        .unwrap();
    assert!(output.status.success(), "junction fixture failed: {output:?}");
    f.track("link", ItemKind::Folder, FileStatus::Local);
    let error = purge(&f.db, Some(&f.root), &UnknownFiles::Delete).unwrap_err();
    assert!(
        !error.to_string().contains("Unsynced files remain"),
        "junction was traversed: {error:?}"
    );
    assert!(link.exists());
    assert_eq!(std::fs::read(outside.join("precious.txt")).unwrap(), b"outside bytes");
    assert_eq!(f.db.list_files().unwrap().len(), 1);
    std::fs::remove_dir(link).unwrap();
}

#[test]
fn prepared_directory_cannot_be_replaced_during_cleanup() {
    let f = Fixture::new();
    f.folder("local");
    let rows = f.db.list_files().unwrap();
    let known = rows.iter().map(|r| (r.path.clone(), r)).collect();
    let (mut files, mut directories) = (Vec::new(), Vec::new());
    prepare(
        &f.db,
        &f.root,
        &f.root,
        &known,
        &HashMap::new(),
        &UnknownFiles::Delete,
        &mut Vec::new(),
        &mut files,
        &mut directories,
    )
    .unwrap();
    assert_eq!(files.len(), 0);
    assert_eq!(directories.len(), 1);
    assert!(
        std::fs::rename(f.root.join("local"), f.root.join("replaced")).is_err(),
        "prepared directory must remain bound to the validated path"
    );
    drop(directories);
    purge(&f.db, Some(&f.root), &UnknownFiles::Delete).unwrap();
    assert_eq!(f.db.list_files().unwrap().len(), 0);
}

#[test]
fn child_arriving_after_prepare_blocks_nonrecursive_deletion() {
    let f = Fixture::new();
    f.folder("local");
    let rows = f.db.list_files().unwrap();
    let known = rows.iter().map(|r| (r.path.clone(), r)).collect();
    let (mut files, mut directories) = (Vec::new(), Vec::new());
    prepare(
        &f.db,
        &f.root,
        &f.root,
        &known,
        &HashMap::new(),
        &UnknownFiles::Delete,
        &mut Vec::new(),
        &mut files,
        &mut directories,
    )
    .unwrap();
    assert_eq!(directories.len(), 1);
    let child = f.root.join("local/late.txt");
    std::fs::write(&child, b"new unsynced bytes").unwrap();
    assert!(
        mark_for_deletion(&directories[0]).is_err(),
        "nonempty directory must survive"
    );
    drop(directories);
    assert_eq!(std::fs::read(&child).unwrap(), b"new unsynced bytes");
    assert_eq!(f.db.list_files().unwrap().len(), 1);
    assert!(purge(&f.db, Some(&f.root), &UnknownFiles::Delete).is_err());
    assert_eq!(std::fs::read(child).unwrap(), b"new unsynced bytes");
}

#[test]
fn round5_signout_retries_busy_completed_upload_and_leaves_zero_staged_plaintext() {
    let f = Fixture::new();
    let payload = f.temp.path().join("staged-plaintext");
    std::fs::write(&payload, b"server completed content").unwrap();
    f.db.track_staged_payload(&payload.to_string_lossy(), None, true)
        .unwrap();
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&payload)
        .unwrap();
    assert!(purge(&f.db, Some(&f.root), &UnknownFiles::Delete).is_err());
    assert_eq!(f.db.staged_payloads_for_signout().unwrap().len(), 1);
    drop(held);
    purge(&f.db, Some(&f.root), &UnknownFiles::Delete).unwrap();
    assert_eq!(f.db.staged_payloads_for_signout().unwrap().len(), 0);
    assert_eq!([payload].iter().filter(|p| p.exists()).count(), 0);
}

#[test]
fn round5_signout_preserves_abandoned_legacy_upload_only_copy() {
    let f = Fixture::new();
    let payload = f.temp.path().join("legacy-only-copy");
    std::fs::write(&payload, b"unsynced version").unwrap();
    f.db.put_upload_resume(&crate::state_db::UploadResume {
        op_id: "abandoned-keep-mine".into(),
        payload_path: payload.to_string_lossy().into_owned(),
        payload_size: 16,
        payload_mtime_ns: 0,
        upload_session_id: "session".into(),
        server_file_id: "old-file".into(),
        object_version_id: "version".into(),
        chunk_size_bytes: 16,
        chunk_count: 1,
        acked_chunks: 0,
        metadata_applied: false,
        is_create: false,
        completed_version: None,
        completed_object_version_id: None,
        completed_mime_type: None,
    })
    .unwrap();
    assert!(
        purge(&f.db, Some(&f.root), &UnknownFiles::Delete)
            .unwrap_err()
            .to_string()
            .contains("recoverable bytes")
    );
    assert_eq!(std::fs::read(&payload).unwrap(), b"unsynced version");
    assert!(f.db.get_upload_resume("abandoned-keep-mine").unwrap().is_some());
    // User recovers their only copy outside the provider; retry may finish.
    let recovered = f.temp.path().join("recovered");
    std::fs::rename(&payload, &recovered).unwrap();
    purge(&f.db, Some(&f.root), &UnknownFiles::Delete).unwrap();
    assert_eq!(std::fs::read(recovered).unwrap(), b"unsynced version");
    assert!(f.db.get_upload_resume("abandoned-keep-mine").unwrap().is_none());
}

// Registered roots are confined to a fresh temporary directory and revoked on drop.
struct RegisteredRoot(Fixture);
impl RegisteredRoot {
    fn new() -> Self {
        let f = Fixture::new();
        crate::windows_cf::register_sync_root(&f.root).unwrap();
        Self(f)
    }
}
impl Drop for RegisteredRoot {
    fn drop(&mut self) {
        crate::windows_cf::unregister_sync_root(&self.0.root).unwrap();
    }
}

#[test]
fn round6_uploaded_local_identity_matches_server_and_signs_out() {
    let registered = RegisteredRoot::new();
    let f = &registered.0;
    let path = f.root.join("server-id");
    std::fs::write(&path, b"completed uploaded bytes").unwrap();
    crate::windows_cf::placeholders::convert_to_unsynced_placeholder(&path, "temporary-local-id").unwrap();
    f.track("server-id", ItemKind::File, FileStatus::Local);
    crate::windows_cf::placeholders::convert_to_in_sync_placeholder(&path, "server-id").unwrap();
    let result = purge(&f.db, Some(&f.root), &UnknownFiles::Delete);
    assert!(result.is_ok(), "completed local upload must sign out: {result:?}");
    assert_eq!(std::fs::read_dir(&f.root).unwrap().count(), 0);
    assert_eq!(f.db.list_files().unwrap().len(), 0);
}

#[test]
fn round6_foreign_and_dirty_placeholders_keep_bytes() {
    for foreign in [true, false] {
        let registered = RegisteredRoot::new();
        let f = &registered.0;
        let path = f.root.join("server-id");
        std::fs::write(&path, b"must preserve bytes").unwrap();
        let identity = if foreign { "foreign-id" } else { "server-id" };
        crate::windows_cf::placeholders::convert_to_unsynced_placeholder(&path, identity).unwrap();
        f.track("server-id", ItemKind::File, FileStatus::Local);
        let error = purge(&f.db, Some(&f.root), &UnknownFiles::Delete).unwrap_err().to_string();
        assert!(error.contains(if foreign { "identity changed" } else { "Unsynced changes" }), "{error}");
        assert_eq!(std::fs::read(&path).unwrap(), b"must preserve bytes");
        assert_eq!(f.db.list_files().unwrap().len(), 1);
    }
}

#[test]
fn round6_unregister_removes_empty_owned_hkcu_key() {
    use windows::Win32::System::Registry::*;
    use windows::Win32::Foundation::{ERROR_SUCCESS, ERROR_FILE_NOT_FOUND};
    use windows::core::HSTRING;
    let f = Fixture::new();
    let id = crate::windows_cf::shell_sync_root_id(&f.root);
    let key = HSTRING::from(format!("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Explorer\\SyncRootManager\\{id}"));
    unsafe {
        let mut handle = HKEY::default();
        assert_eq!(RegCreateKeyExW(HKEY_CURRENT_USER, &key, 0, None, REG_OPTION_NON_VOLATILE, KEY_READ, None, &mut handle, None), ERROR_SUCCESS);
        assert_eq!(RegCloseKey(handle), ERROR_SUCCESS);
        let result = crate::windows_cf::unregister_shell_sync_root(&f.root);
        let remaining = RegOpenKeyExW(HKEY_CURRENT_USER, &key, 0, KEY_READ, &mut handle);
        if remaining == ERROR_SUCCESS { assert_eq!(RegCloseKey(handle), ERROR_SUCCESS); }
        // Test cleanup even when the baseline leaves the key behind.
        let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &key);
        assert!(result.is_ok(), "unregister: {result:?}");
        assert_eq!(remaining, ERROR_FILE_NOT_FOUND, "owned HKCU key must be absent");
    }
}

/// A finalization journal row: the upload `op` of the file at `target` became the server file `server_id`.
fn journal_finalization(f: &Fixture, op: &str, local_id: &str, server_id: &str, target: &str, payload: &std::path::Path) {
    f.db.put_upload_finalization(&crate::state_db::UploadFinalization {
        op_id: op.into(),
        local_file_id: local_id.into(),
        server_file_id: server_id.into(),
        target_path: target.into(),
        payload_path: payload.to_string_lossy().into_owned(),
        stamped: false,
        review: None,
    })
    .unwrap();
}

/// Tasks 2270 / 2280 changed this test's meaning (0.8.19 refused both cases with an error on every tick):
/// another file's placeholder is held for review and never stamped (spec F4); bytes that are neither the payload
/// nor the agreed content get the server identity only, not in sync (F7c). Either way the bytes stay, and sign-out
/// still refuses to delete them.
#[test]
fn round6_upload_finalization_preserves_foreign_or_changed_bytes() {
    use crate::unknown_check::{OnDisk, PlatformView};
    for foreign in [false, true] {
        let registered = RegisteredRoot::new();
        let f = &registered.0;
        let path = f.root.join("server-id");
        let payload = f.temp.path().join("uploaded");
        std::fs::write(&path, b"new local edit").unwrap();
        std::fs::write(&payload, if foreign { b"new local edit".as_slice() } else { b"uploaded original".as_slice() }).unwrap();
        crate::windows_cf::placeholders::convert_to_unsynced_placeholder(&path, if foreign { "foreign-id" } else { "local-id" }).unwrap();
        f.track("server-id", ItemKind::File, FileStatus::Local);
        journal_finalization(f, "op", "local-id", "server-id", "server-id", &payload);
        crate::windows_cf::upload_finalization::retry(&f.db, &f.root);
        let identity = if foreign { "foreign-id" } else { "server-id" };
        assert_eq!(
            crate::windows_cf::placeholders::platform_view(&path),
            PlatformView::Placeholder { on_disk: OnDisk::Wholly, in_sync: false, identity: Some(identity.into()) },
            "foreign={foreign}"
        );
        let reviews: Vec<_> = f.db.upload_finalizations().unwrap().into_iter().map(|row| row.review).collect();
        assert_eq!(reviews, if foreign { vec![Some("other_file".to_string())] } else { vec![] }, "foreign={foreign}");
        assert_eq!(std::fs::read(&path).unwrap(), b"new local edit");
        assert!(purge(&f.db, Some(&f.root), &UnknownFiles::Delete).is_err(), "foreign={foreign}");
        assert_eq!(std::fs::read(&path).unwrap(), b"new local edit", "foreign={foreign}");
    }
}

#[test]
fn round6_verified_upload_finalization_updates_identity() {
    let registered = RegisteredRoot::new();
    let f = &registered.0;
    let path = f.root.join("server-id");
    let payload = f.temp.path().join("uploaded");
    std::fs::write(&path, b"uploaded bytes").unwrap();
    std::fs::write(&payload, b"uploaded bytes").unwrap();
    crate::windows_cf::placeholders::convert_to_unsynced_placeholder(&path, "local-id").unwrap();
    f.track("server-id", ItemKind::File, FileStatus::Local);
    journal_finalization(f, "op", "local-id", "server-id", "server-id", &payload);
    purge(&f.db, Some(&f.root), &UnknownFiles::Delete).unwrap();
    assert!(f.db.upload_finalizations().unwrap().is_empty());
    assert_eq!(std::fs::read_dir(&f.root).unwrap().count(), 0);
}

/// Tasks 2270 / 2280 (spec X1, I5; review focus): a row held for review no longer blocks sign-out (the person cannot
/// act on it), and sign-out forgets it. A row still pending (its file is open in another program) still blocks it,
/// with today's message, until it finishes.
#[test]
fn signout_purges_review_rows_and_still_refuses_a_pending_one() {
    let registered = RegisteredRoot::new();
    let f = &registered.0;
    // In review since an earlier tick of this start: its file moved before it was finalized (F1).
    journal_finalization(f, "review-op", "review-local", "review-server", "moved-away", &f.temp.path().join("review-payload"));
    f.db.mark_upload_finalization_review("review-op", "moved", 1).unwrap();
    crate::windows_cf::upload_finalization::mark_review_retried(&f.root);
    // Pending: the uploaded file is open in another program.
    let path = f.root.join("server-id");
    let payload = f.temp.path().join("uploaded");
    std::fs::write(&path, b"uploaded bytes").unwrap();
    std::fs::write(&payload, b"uploaded bytes").unwrap();
    f.track("server-id", ItemKind::File, FileStatus::Local);
    journal_finalization(f, "pending-op", "local-id", "server-id", "server-id", &payload);
    let reader = std::fs::File::open(&path).unwrap();
    let error = purge(&f.db, Some(&f.root), &UnknownFiles::Delete).unwrap_err();
    assert!(error.to_string().contains("still needs its local identity repaired"), "{error:?}");
    let ops: Vec<_> = f.db.upload_finalizations().unwrap().into_iter().map(|row| row.op_id).collect();
    assert_eq!(ops, vec!["pending-op".to_string()], "the review row is forgotten, the pending one kept");
    assert_eq!(std::fs::read(&path).unwrap(), b"uploaded bytes");
    drop(reader);
    purge(&f.db, Some(&f.root), &UnknownFiles::Delete).unwrap();
    assert!(f.db.upload_finalizations().unwrap().is_empty());
    assert_eq!(std::fs::read_dir(&f.root).unwrap().count(), 0);
}

/// Task 2191 (D11, Decision (lead, 2026-10-26)): a file of unknown content is copied out of the
/// sync folder when the person chose "Keep copies", before the original goes.
#[test]
fn task_2191_sign_out_keeps_an_unknown_file_as_a_copy() {
    let f = Fixture::new();
    f.folder("docs");
    std::fs::write(f.root.join("docs").join("a.txt"), b"an edit 0.8.12 never sent").unwrap();
    f.track_unknown("docs/a.txt", ItemKind::File, FileStatus::Local);
    let dir = f.temp.path().join("Beebeeb kept files 2026-10-26 1200");
    let kept = purge(&f.db, Some(&f.root), &UnknownFiles::Keep { dir: dir.clone() }).unwrap();
    assert_eq!(kept, KeptAtSignOut { files: 1, dir: Some(dir.clone()) });
    assert_eq!(std::fs::read(dir.join("docs").join("a.txt")).unwrap(), b"an edit 0.8.12 never sent");
    assert_eq!(std::fs::read_dir(&f.root).unwrap().count(), 0, "the sync folder is emptied");
}

/// Task 2191: "Delete them" deletes a file of unknown content with the rest, and keeps nothing.
#[test]
fn task_2191_sign_out_deletes_an_unknown_file_when_asked() {
    let f = Fixture::new();
    std::fs::write(f.root.join("a.txt"), b"an edit 0.8.12 never sent").unwrap();
    f.track_unknown("a.txt", ItemKind::File, FileStatus::Local);
    let kept = purge(&f.db, Some(&f.root), &UnknownFiles::Delete).unwrap();
    assert_eq!(kept, KeptAtSignOut::default());
    assert_eq!(std::fs::read_dir(&f.root).unwrap().count(), 0);
}

/// Task 2191: a file of KNOWN content that is not a clean placeholder still refuses sign-out.
#[test]
fn task_2191_a_known_file_that_is_not_clean_still_refuses_sign_out() {
    let f = Fixture::new();
    std::fs::write(f.root.join("a.txt"), b"plain bytes").unwrap();
    f.track("a.txt", ItemKind::File, FileStatus::Local);
    assert!(purge(&f.db, Some(&f.root), &UnknownFiles::Delete).is_err());
    assert!(f.root.join("a.txt").exists(), "nothing deleted");
}
