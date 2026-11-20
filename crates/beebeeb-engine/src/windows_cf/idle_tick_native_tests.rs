//! w36 (2026-11-20): an idle engine tick kept one Process handle per sync-root row (`CfCreatePlaceholders` keeps one
//! each time it answers "already exists": 500 calls, +500 handles on Windows 11 23H2 and 25H2) and did about 19
//! metadata operations per row. On real Windows: the handle count and the I/O of an idle tick do not grow with the rows.
use super::*;
use crate::state_db::{FileContractState, FileEntry, FileStatus, ItemKind, Namespace, PinState, StateDb};
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessHandleCount, GetProcessIoCounters, IO_COUNTERS};

/// The handles this process holds now.
fn handles() -> i64 {
    let mut count = 0u32;
    // SAFETY: the pseudo-handle of this process; `count` outlives the call.
    unsafe { GetProcessHandleCount(GetCurrentProcess(), &mut count) }.unwrap();
    count as i64
}

/// The "other" (metadata) I/O operations this process made so far.
fn other_io() -> u64 {
    let mut io = IO_COUNTERS::default();
    // SAFETY: as in `handles`.
    unsafe { GetProcessIoCounters(GetCurrentProcess(), &mut io) }.unwrap();
    io.OtherOperationCount
}

struct Root {
    connected: Option<CF_CONNECTION_KEY>,
    db: StateDb,
    root: std::path::PathBuf,
    _temp: tempfile::TempDir,
}

impl Root {
    /// A registered scratch root, connected with no callbacks (as the running app is connected).
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let db = StateDb::open(temp.path().join("state.db")).unwrap();
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

    /// A cloud-only row (file id = its path).
    fn track(&self, rel: &str, kind: ItemKind) {
        self.db
            .upsert_file(&FileEntry {
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
        self.db
            .set_file_contract_state(&FileContractState {
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

    /// `folders` folders of `per_folder` cloud-only files each; the number of rows.
    fn seed(&self, folders: usize, per_folder: usize) -> usize {
        for d in 0..folders {
            self.track(&format!("dir{d}"), ItemKind::Folder);
            for f in 0..per_folder {
                self.track(&format!("dir{d}/file{f}.bin"), ItemKind::File);
            }
        }
        folders * (per_folder + 1)
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

/// The tick's two per-row passes (`runner.rs`: `refresh_placeholders`, then `reconcile_placeholder_state`), with
/// what they added to this process: (populate stats, handles, metadata I/O operations).
fn tick(
    r: &Root,
    memo: &mut populate::PlacedMemo,
    cursor: &std::sync::atomic::AtomicUsize,
) -> (populate::PopulateStats, i64, u64) {
    let (h, io) = (handles(), other_io());
    let stats = populate_placeholders(&r.db, &r.root, memo);
    reconcile_placeholder_state_with(&r.db, &r.root, cursor);
    (stats, handles() - h, other_io() - io)
}

#[test]
fn w36_an_idle_tick_keeps_no_handle_and_does_no_work_per_row() {
    const FOLDERS: usize = 6;
    let r = Root::new();
    let rows = r.seed(FOLDERS, 60);
    let cursor = std::sync::atomic::AtomicUsize::new(0);
    let mut memo = populate::PlacedMemo::default();

    let (first, _, first_io) = tick(&r, &mut memo, &cursor);
    assert_eq!((first.created, first.create_calls), (rows, rows), "{first:?}");

    // The first tick after a start: every placeholder is there, the memo is empty. No create call, so no handle.
    let (start, start_handles, _) = tick(&r, &mut populate::PlacedMemo::default(), &cursor);
    assert_eq!(
        (start.already_existed, start.create_calls, start.visited),
        (rows, 0, rows),
        "{start:?}"
    );

    // Idle ticks. The counters are the whole test process's, and other tests run next to this one: they only add,
    // so the quietest tick is the measure. Before: +1 handle and 8 to 19 operations per row, every tick.
    let mut least = (i64::MAX, u64::MAX);
    for _ in 0..5 {
        let (idle, tick_handles, tick_io) = tick(&r, &mut memo, &cursor);
        assert_eq!(
            (idle.visited, idle.create_calls, idle.dirs_listed, idle.created),
            (0, 0, FOLDERS + 1, 0),
            "{idle:?}"
        );
        least = (least.0.min(tick_handles), least.1.min(tick_io));
    }
    println!(
        "W36 rows={rows} first_tick_io={first_io} start_handles={start_handles:+} idle_handles={:+} idle_io={}",
        least.0, least.1
    );
    assert!(
        start_handles < rows as i64 / 4,
        "the first tick after a start kept {start_handles} handles for {rows} rows"
    );
    assert!(
        least.0 < rows as i64 / 4,
        "an idle tick kept {} handles for {rows} rows",
        least.0
    );
    assert!(
        least.1 < rows as u64,
        "an idle tick did {} metadata operations for {rows} rows",
        least.1
    );

    // The memo hides nothing: a placeholder deleted on disk is minted again, a new row is minted, the rest is skipped.
    std::fs::remove_file(r.root.join("dir0").join("file0.bin")).unwrap();
    r.track("dir1/new.bin", ItemKind::File);
    let (changed, _, _) = tick(&r, &mut memo, &cursor);
    assert_eq!(
        (changed.visited, changed.created, changed.create_calls),
        (2, 2, 2),
        "{changed:?}"
    );
    assert!(r.root.join("dir0").join("file0.bin").exists() && r.root.join("dir1").join("new.bin").exists());
    // A row that moved is placed at its new path.
    let mut moved = r.db.get_file("dir1/new.bin").unwrap().unwrap();
    moved.path = "dir3/new.bin".into();
    r.db.upsert_file(&moved).unwrap();
    let (after_move, _, _) = tick(&r, &mut memo, &cursor);
    assert_eq!((after_move.visited, after_move.created), (1, 1), "{after_move:?}");
}
