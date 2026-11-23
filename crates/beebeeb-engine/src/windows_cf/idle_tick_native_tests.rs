//! w36 (2026-11-20): an idle engine tick kept one Process handle per sync-root row (`CfCreatePlaceholders` keeps one
//! each time it answers "already exists": 500 calls, +500 handles on Windows 11 23H2 and 25H2) and did about 19
//! metadata operations per row. On real Windows: the handle count and the I/O of an idle tick do not grow with the rows.
use super::*;
use std::os::windows::ffi::OsStrExt;
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
    // dc-164: one create call per folder batch (the root's folders, then each folder's files).
    assert_eq!((first.created, first.create_calls), (rows, FOLDERS + 1), "{first:?}");

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

/// The 8.3 short name of the entry at `path`; `None` when the volume gave it none.
fn short_name_of(path: &std::path::Path) -> Option<String> {
    use windows::Win32::Storage::FileSystem::GetShortPathNameW;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let mut buffer = vec![0u16; 1024];
    // SAFETY: a NUL-terminated path and a buffer that outlive the call.
    let written = unsafe { GetShortPathNameW(PCWSTR(wide.as_ptr()), Some(&mut buffer)) } as usize;
    assert!(written != 0 && written < buffer.len(), "GetShortPathNameW failed");
    let short = std::path::PathBuf::from(String::from_utf16_lossy(&buffer[..written]));
    let (short, long) = (short.file_name()?.to_str()?.to_string(), path.file_name()?.to_str()?);
    (!short.eq_ignore_ascii_case(long)).then_some(short)
}

/// w37: a row whose name the folder shows in another case, or that reaches its entry through the entry's 8.3 short
/// name, is "already exists" too. Before, the listing did not show it under the row's exact characters, so every
/// tick called `CfCreatePlaceholders` for it and kept one Process handle per row.
#[test]
fn w37_placeholder_rows_named_in_another_case_or_by_a_short_name_keep_no_handle() {
    const EACH: usize = 40;
    let r = Root::new();
    r.track("dir", ItemKind::Folder);
    for n in 0..EACH {
        r.track(&format!("dir/case{n}.bin"), ItemKind::File);
        r.track(&format!("dir/a long name number {n}.data"), ItemKind::File);
    }
    let rows = 1 + 2 * EACH;
    let cursor = std::sync::atomic::AtomicUsize::new(0);
    let (first, _, _) = tick(&r, &mut populate::PlacedMemo::default(), &cursor);
    // dc-164: one create call per folder batch (the root's one folder, then its files).
    assert_eq!((first.created, first.create_calls), (rows, 2), "{first:?}");

    // The rows now name their entries in another case, and by the entry's short name.
    let mut by_short_name = 0;
    for n in 0..EACH {
        let mut row = r.db.get_file(&format!("dir/case{n}.bin")).unwrap().unwrap();
        row.path = format!("dir/CASE{n}.BIN");
        r.db.upsert_file(&row).unwrap();
        let long = format!("dir/a long name number {n}.data");
        if let Some(short) = short_name_of(&r.root.join("dir").join(format!("a long name number {n}.data"))) {
            let mut row = r.db.get_file(&long).unwrap().unwrap();
            row.path = format!("dir/{short}");
            r.db.upsert_file(&row).unwrap();
            by_short_name += 1;
        }
    }
    println!("W37 rows={rows} by_case={EACH} by_short_name={by_short_name}");
    assert_eq!(
        by_short_name, EACH,
        "this volume gives no 8.3 short names: the short-name half did not run"
    );

    // The first tick after a start (an empty memo), then three idle ticks: no create call, no handle kept.
    let mut memo = populate::PlacedMemo::default();
    let (start, start_handles, _) = tick(&r, &mut memo, &cursor);
    assert_eq!(
        (start.already_existed, start.create_calls, start.created),
        (rows, 0, 0),
        "{start:?}"
    );
    let mut idle_handles = Vec::new();
    for _ in 0..3 {
        let (idle, tick_handles, _) = tick(&r, &mut memo, &cursor);
        assert_eq!((idle.visited, idle.create_calls, idle.created), (0, 0, 0), "{idle:?}");
        idle_handles.push(tick_handles);
    }
    println!("W37 start_handles={start_handles:+} idle_handles={idle_handles:?}");
    // The counter is the whole test process's (other tests only add): well under one per row.
    assert!(
        start_handles < EACH as i64 / 2,
        "the first tick kept {start_handles} handles"
    );
    let least = *idle_handles.iter().min().unwrap();
    assert!(
        least < EACH as i64 / 2,
        "an idle tick kept {least} handles for {} such rows",
        2 * EACH
    );
    // Nothing was made beside the entries: the folder still holds one entry per row.
    assert_eq!(std::fs::read_dir(r.root.join("dir")).unwrap().count(), 2 * EACH);
    // An entry deleted on disk is minted again under the row's own name.
    std::fs::remove_file(r.root.join("dir").join("case0.bin")).unwrap();
    let (again, _, _) = tick(&r, &mut memo, &cursor);
    assert_eq!(
        (again.visited, again.created, again.create_calls),
        (1, 1, 1),
        "{again:?}"
    );
    assert!(names_of(&r.root.join("dir")).contains(&"CASE0.BIN".to_string()));
}

/// The exact names in `dir`.
fn names_of(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect()
}

/// RC2-6b: the tick skips its two placeholder passes while its journal feed names nothing under the root. That holds
/// only if (1) a native pin, unpin and "Free up space" each leave a journal record under the root (the passes then
/// run and reconcile them), and (2) what an idle tick itself does to an in-sync file (the stamp pass's size-and-time
/// look, its in-sync read, the reconcile's listing and direct read) leaves none, so the gate can close at all.
#[test]
fn rc2_6b_native_pin_and_free_up_name_the_root_and_idle_reads_do_not() {
    use crate::watcher::Changes;
    let r = Root::new();
    let dir = r.root.join("dir");
    std::fs::create_dir(&dir).unwrap();
    let file = dir.join("held.bin");
    std::fs::write(&file, vec![7u8; 64 * 1024]).unwrap();
    placeholders::convert_to_in_sync_placeholder(&file, "held").unwrap();
    let mut feed = crate::watcher::change_feed::open(&r.root);
    // Drain what making the file wrote; a journal that cannot be read here says so instead of passing.
    let first = feed.take();
    assert_ne!(first, Changes::Unknown, "no change journal on this volume");
    let named = |changes: Changes, what: &str| match changes {
        Changes::Paths(paths) => assert!(
            paths.iter().any(|p| p.starts_with(&r.root)),
            "{what}: named only {paths:?}"
        ),
        other => panic!("{what} left no record under the root: {other:?}"),
    };

    // (2) An idle tick's reads of an in-sync file.
    for _ in 0..3 {
        let _ = std::fs::metadata(&file).unwrap().modified();
        assert_eq!(placeholders::placeholder_in_sync(&file), Some(true));
        placeholders::list_dir_attributes(&dir).unwrap();
        placeholders::read_os_placeholder_state(&file).unwrap();
        std::fs::read_dir(&r.root).unwrap().for_each(drop);
    }
    assert_eq!(feed.take(), Changes::Nothing, "an idle tick's own reads named the root");

    // (1) The native changes Explorer makes.
    placeholders::set_pin_state(&file, true, false).unwrap();
    named(feed.take(), "a pin");
    placeholders::set_pin_state(&file, false, false).unwrap();
    named(feed.take(), "an unpin");
    assert!(placeholders::dehydrate_placeholder(&file).unwrap() > 0, "nothing was freed");
    named(feed.take(), "a free up space");
    assert_eq!(feed.take(), Changes::Nothing);
}
