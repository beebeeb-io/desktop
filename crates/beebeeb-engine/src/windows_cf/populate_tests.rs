//! dc-164 (task dc-086: 41,510 rows, one `CfCreatePlaceholders` call and two commits per row, nothing in Explorer until
//! the whole list was in). On real Windows, on a QA sync root: one create call per folder batch, and the placeholders of
//! the top folders exist while the first pull is still writing rows.
use super::*;
use crate::state_db::{FileContractState, FileEntry, FileStatus, ItemKind, Namespace, PinState, StateDb};
use std::os::windows::ffi::OsStrExt;
use windows::Win32::Storage::CloudFilters::{
    CF_CALLBACK_REGISTRATION, CF_CALLBACK_TYPE_NONE, CF_CONNECT_FLAG_NONE, CF_CONNECTION_KEY, CfConnectSyncRoot,
    CfDisconnectSyncRoot,
};
use windows::core::PCWSTR;

struct Root {
    connected: Option<CF_CONNECTION_KEY>,
    db: std::sync::Arc<StateDb>,
    root: std::path::PathBuf,
    _temp: tempfile::TempDir,
}

impl Root {
    /// A registered scratch root, connected with no callbacks (as the running app is connected).
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let db = std::sync::Arc::new(StateDb::open(temp.path().join("state.db")).unwrap());
        crate::windows_cf::register_test_sync_root(&root).unwrap();
        let table = [CF_CALLBACK_REGISTRATION {
            Type: CF_CALLBACK_TYPE_NONE,
            Callback: None,
        }];
        let root_wide: Vec<u16> = root.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        // SAFETY: the path and the table outlive the call; no callback is registered.
        let key = unsafe { CfConnectSyncRoot(PCWSTR(root_wide.as_ptr()), table.as_ptr(), None, CF_CONNECT_FLAG_NONE) }
            .expect("connect the scratch root");
        Self {
            connected: Some(key),
            db,
            root,
            _temp: temp,
        }
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        if let Some(key) = self.connected.take() {
            // SAFETY: the key of the connection made in `new`, disconnected once.
            let _ = unsafe { CfDisconnectSyncRoot(key) };
        }
        crate::windows_cf::unregister_any_sync_root(&self.root).unwrap();
    }
}

/// A cloud-only row (file id = its path), as the pull writes it.
fn track(db: &StateDb, rel: &str, kind: ItemKind) {
    db.upsert_file(&FileEntry {
        file_id: rel.into(),
        path: rel.into(),
        status: FileStatus::CloudOnly,
        size_bytes: if kind == ItemKind::File { 1234 } else { 0 },
        modified_at: 1_700_000_000,
        content_hash: None,
        remote_updated_at: 1,
        parent_id: None,
        item_kind: kind.clone(),
    })
    .unwrap();
    db.set_file_contract_state(&FileContractState {
        file_id: rel.into(),
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

#[test]
fn populate_mints_each_folder_batch_in_one_create_call() {
    let r = Root::new();
    track(&r.db, "a.txt", ItemKind::File);
    track(&r.db, "b.txt", ItemKind::File);
    for d in 0..3 {
        track(&r.db, &format!("dir{d}"), ItemKind::Folder);
        for f in 0..40 {
            track(&r.db, &format!("dir{d}/file{f}.bin"), ItemKind::File);
        }
    }
    track(&r.db, "big", ItemKind::Folder);
    for f in 0..=MINT_BATCH {
        track(&r.db, &format!("big/file{f}.bin"), ItemKind::File);
    }
    let rows = 2 + 3 * 41 + 1 + MINT_BATCH + 1;

    let mut memo = PlacedMemo::default();
    let s = populate_placeholders(&r.db, &r.root, &mut memo);
    assert_eq!((s.rows, s.created, s.create_errors), (rows, rows, 0), "{s:?}");
    // The root (2 files, 4 folders), dir0..dir2, and `big` in two (MINT_BATCH + 1 rows).
    assert_eq!(s.create_calls, 1 + 3 + 2, "one create call per folder batch: {s:?}");
    for rel in ["a.txt", "dir2", "dir2\\file39.bin", "big\\file0.bin"] {
        assert!(placeholders::is_cloud_placeholder(&r.root.join(rel)), "{rel} is a placeholder");
    }
    let last = format!("big\\file{MINT_BATCH}.bin");
    assert!(placeholders::is_cloud_placeholder(&r.root.join(last)));

    // The next pass finds them all and calls nothing.
    let again = populate_placeholders(&r.db, &r.root, &mut memo);
    assert_eq!((again.create_calls, again.created), (0, 0), "{again:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn populate_early_placeholders_for_a_folder_exist_before_the_pull_ends() {
    const FOLDERS: usize = 12;
    const PER_FOLDER: usize = 500;
    const EVERY: usize = 200;
    let r = Root::new();
    let (db, root) = (r.db.clone(), r.root.clone());
    let early = EarlyPasses::start_with(std::time::Duration::from_millis(10), move || {
        Some(populate_placeholders_in(&db, &root, &mut PlacedMemo::default(), Mode::Early))
    });

    // The first pull: top level first (as `order_snapshot_nodes` orders it), committed every EVERY rows.
    let (db, root) = (r.db.clone(), r.root.clone());
    let total = FOLDERS * (PER_FOLDER + 1);
    let seen_at = std::thread::spawn(move || {
        let mut rels: Vec<(String, ItemKind)> = (0..FOLDERS).map(|d| (format!("top{d}"), ItemKind::Folder)).collect();
        for d in 0..FOLDERS {
            rels.extend((0..PER_FOLDER).map(|f| (format!("top{d}/file{f}.bin"), ItemKind::File)));
        }
        let mut seen_at = None;
        let mut batch = db.write_batch(EVERY).unwrap();
        for (written, (rel, kind)) in rels.into_iter().enumerate() {
            if written > 0 {
                batch
                    .row_done(|rows| {
                        if seen_at.is_none() && placeholders::is_cloud_placeholder(&root.join("top0")) {
                            seen_at = Some(rows);
                        }
                    })
                    .unwrap();
            }
            track(&db, &rel, kind);
        }
        batch.finish().unwrap();
        seen_at
    })
    .join()
    .unwrap();
    early.stop().await;

    let seen_at = seen_at.expect("a top folder's placeholder appeared while the pull wrote rows");
    assert!(seen_at < total, "seen after {seen_at} of {total} rows");
    // No folder became a plain directory where its placeholder belongs.
    for d in 0..FOLDERS {
        assert!(placeholders::is_cloud_placeholder(&r.root.join(format!("top{d}"))), "top{d}");
    }
    // The tick's pass after the pull places the rest.
    let s = populate_placeholders(&r.db, &r.root, &mut PlacedMemo::default());
    assert_eq!(s.create_errors + s.ensure_dir_errors + s.name_check_errors, 0, "{s:?}");
    assert_eq!(s.created + s.already_existed, total, "{s:?}");
}
