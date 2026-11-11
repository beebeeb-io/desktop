//! `scan` tests: the page-cache sizing (task 2240), an explorer drop queued
//! as an `Uploading` row (direct and atomic temp rename), and the Windows Cloud
//! Files identity rules.

use super::*;

/// Task 2240 (T7, D1): every scan pass sizes the engine connection's page cache before it walks the tree, so the
/// row lookups of the pass hit memory instead of the file. A failure to size it is logged, and the pass goes on with
/// the cache it has.
#[test]
fn task_2240_every_scan_pass_fits_the_page_cache_first() {
    use crate::source_pin::body_between;
    use crate::source_pin::squeeze;
    let source = include_str!("../../watcher.rs").replace("\r\n", "\n");
    let body = squeeze(body_between(&source, "pub fn run_one_scan_with(", "\n}\n"));
    let fit = body
        .find("ifletErr(e)=bridge.db().fit_page_cache(){")
        .expect("the pass fits the page cache");
    let memo = body.find("memo.settled.clear()").expect("the memo reset");
    let walk = body.find("walk_dir(").expect("the walk");
    assert!(fit < memo && fit < walk, "the fit comes first:\n{body}");
}

fn test_bridge(db_path: &std::path::Path) -> (Arc<StateDb>, EngineBridge) {
    let db = Arc::new(StateDb::open(db_path).unwrap());
    let api = Arc::new(ApiClient::new(
        "https://api.beebeeb.io".into(),
        "token".into(),
        [17u8; 32],
    ));
    let bridge = EngineBridge::new(db.clone(), api);
    (db, bridge)
}

fn assert_uploading_row(db: &StateDb, rel_path: &str, expected_bytes: i64) {
    let row = db
        .get_file_by_path(rel_path)
        .unwrap()
        .unwrap_or_else(|| panic!("expected Uploading state.db row for {rel_path}"));
    assert_eq!(row.path, rel_path);
    assert_eq!(row.status, FileStatus::Uploading);
    assert_eq!(row.size_bytes, expected_bytes);

    let queued = db.list_due_operations(i64::MAX).unwrap();
    assert!(
        queued
            .iter()
            .any(|op| { op.kind == OperationKind::UploadVersion && op.target_path.as_deref() == Some(rel_path) }),
        "expected queued upload_version operation for {rel_path}, got: {queued:?}"
    );
}

#[test]
fn scan_queues_direct_explorer_drop_as_uploading_row() {
    let temp = tempfile::tempdir().unwrap();
    let sync_root = temp.path().join("sync-root");
    let state_dir = temp.path().join("state");
    fs::create_dir_all(&sync_root).unwrap();
    fs::create_dir_all(&state_dir).unwrap();
    let (db, bridge) = test_bridge(&state_dir.join("state.db"));

    let file = sync_root.join("direct-write.txt");
    fs::write(&file, b"direct explorer payload").unwrap();

    run_one_scan(&bridge, &sync_root);

    assert_uploading_row(&db, "direct-write.txt", 23);
}

#[test]
fn scan_queues_atomic_temp_rename_explorer_drop_as_uploading_row() {
    let temp = tempfile::tempdir().unwrap();
    let sync_root = temp.path().join("sync-root");
    let state_dir = temp.path().join("state");
    fs::create_dir_all(&sync_root).unwrap();
    fs::create_dir_all(&state_dir).unwrap();
    let (db, bridge) = test_bridge(&state_dir.join("state.db"));

    let temp_file = sync_root.join("renamed-write.txt.tmp");
    let final_file = sync_root.join("renamed-write.txt");
    fs::write(&temp_file, b"atomic rename explorer payload").unwrap();

    run_one_scan(&bridge, &sync_root);
    assert!(
        db.get_file_by_path("renamed-write.txt.tmp").unwrap().is_none(),
        "temporary Explorer copy name must not be queued"
    );

    fs::rename(&temp_file, &final_file).unwrap();
    run_one_scan(&bridge, &sync_root);

    assert_uploading_row(&db, "renamed-write.txt", 30);
}

#[cfg(target_os = "windows")]
#[test]
fn round6_real_watcher_uses_queued_file_identity() {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::{Foundation::HANDLE, Storage::CloudFilters::*};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    fs::create_dir(&root).unwrap();
    crate::windows_cf::register_test_sync_root(&root).unwrap();
    let (db, bridge) = test_bridge(&temp.path().join("state.db"));
    let path = root.join("local.txt");
    fs::write(&path, b"watcher upload bytes").unwrap();
    assert!(dispatch_local_create(&bridge, &root, &path, "scan"));
    let ops = db.list_due_operations(i64::MAX).unwrap();
    assert_eq!(ops.len(), 1);
    let expected = ops[0].file_id.as_ref().unwrap();
    assert_eq!(&db.get_file_by_path("local.txt").unwrap().unwrap().file_id, expected);
    let file = fs::File::open(&path).unwrap();
    let mut buffer = vec![0u64; 1024];
    unsafe {
        CfGetPlaceholderInfo(
            HANDLE(file.as_raw_handle()),
            CF_PLACEHOLDER_INFO_STANDARD,
            buffer.as_mut_ptr().cast(),
            (buffer.len() * 8) as u32,
            None,
        )
        .unwrap();
    }
    let info = unsafe { &*buffer.as_ptr().cast::<CF_PLACEHOLDER_STANDARD_INFO>() };
    let actual =
        unsafe { std::slice::from_raw_parts(info.FileIdentity.as_ptr(), info.FileIdentityLength as usize) }.to_vec();
    drop(file);
    crate::windows_cf::unregister_sync_root(&root).unwrap();
    for op in ops.iter() {
        if let Some(payload) = &op.payload_path {
            fs::remove_file(payload).unwrap();
        }
    }
    assert_eq!(
        String::from_utf8(actual).unwrap(),
        *expected,
        "watcher identity must belong to its queued DB row"
    );
}

/// Tasks 1888 + 2226 (spec §4 "Windows Cloud Files identity", plan Task 1 Step 6b): a folder made on this PC gets a
/// row and a create, but no Cloud Files identity on disk. So when the reconcile pass re-points its row to the
/// server twin and deletes the ghost's id, nothing on disk still names that id.
#[cfg(target_os = "windows")]
#[test]
fn a_folder_made_on_this_pc_gets_no_cloud_files_identity() {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::{Foundation::HANDLE, Storage::CloudFilters::*};
    // FILE_FLAG_BACKUP_SEMANTICS: needed to open a directory.
    const OPEN_A_DIRECTORY: u32 = 0x0200_0000;
    const ERROR_NOT_A_CLOUD_FILE: u32 = 0x8007_0178;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    fs::create_dir(&root).unwrap();
    crate::windows_cf::register_test_sync_root(&root).unwrap();
    let (_db, bridge) = test_bridge(&temp.path().join("state.db"));
    let dir = root.join("new");
    fs::create_dir(&dir).unwrap();
    let id = bridge.ensure_local_folder(&root, &dir);
    let handle = fs::OpenOptions::new()
        .read(true)
        .custom_flags(OPEN_A_DIRECTORY)
        .open(&dir)
        .unwrap();
    let mut buffer = vec![0u64; 1024];
    let info = unsafe {
        CfGetPlaceholderInfo(
            HANDLE(handle.as_raw_handle()),
            CF_PLACEHOLDER_INFO_STANDARD,
            buffer.as_mut_ptr().cast(),
            (buffer.len() * 8) as u32,
            None,
        )
    };
    drop(handle);
    crate::windows_cf::unregister_sync_root(&root).unwrap();
    assert!(id.is_some(), "the folder gets a row and a create");
    let error = info.expect_err("a folder made on this PC is no Cloud Files placeholder");
    assert_eq!(error.code().0 as u32, ERROR_NOT_A_CLOUD_FILE, "{error}");
}
