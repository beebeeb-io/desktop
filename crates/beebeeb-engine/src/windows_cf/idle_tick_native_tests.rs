//! w36 (Guus's PC, 2026-11-20): an idle engine tick leaked one Process handle per sync-root row. On real Windows.
use super::*;
use crate::state_db::{FileEntry, FileStatus, ItemKind, StateDb};
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
    fn new(connect: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let db = StateDb::open(temp.path().join("state.db")).unwrap();
        crate::windows_cf::register_test_sync_root(&root).unwrap();
        let mut connected = None;
        if connect {
            let table = [CF_CALLBACK_REGISTRATION {
                Type: CF_CALLBACK_TYPE_NONE,
                Callback: None,
            }];
            let root_wide: Vec<u16> = root.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
            // SAFETY: the path and the table outlive the call; no callback is registered.
            let key =
                unsafe { CfConnectSyncRoot(PCWSTR(root_wide.as_ptr()), table.as_ptr(), None, CF_CONNECT_FLAG_NONE) }
                    .expect("connect the scratch root");
            connected = Some(key);
        }
        Self {
            connected,
            db,
            root,
            _temp: temp,
        }
    }

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
                item_kind: kind,
            })
            .unwrap();
    }

    /// `folders` folders of `per_folder` cloud-only files each.
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

/// PROBE (diagnosis, removed before merge): which per-row call of the tick keeps a handle.
#[test]
fn w36_probe_which_call_keeps_a_handle() {
    for connect in [false, true] {
        let r = Root::new(connect);
        let rows = r.seed(10, 100);
        let first = populate_placeholders(&r.db, &r.root);
        eprintln!("W36PROBE connect={connect} rows={rows} created={} errors={}", first.created, first.create_errors);
        let (h0, io0) = (handles(), other_io());
        for _ in 0..3 {
            populate_placeholders(&r.db, &r.root);
        }
        eprintln!(
            "W36PROBE connect={connect} populate x3: handles {:+} other_io {:+}",
            handles() - h0,
            other_io() - io0
        );
        let dir = r.root.join("dir0");
        let file = dir.join("file0.bin");
        let n = 500;
        macro_rules! probe {
            ($name:literal, $body:expr) => {{
                let (h, io) = (handles(), other_io());
                for _ in 0..n {
                    let _ = $body;
                }
                eprintln!(
                    "W36PROBE connect={connect} {} x{n}: handles {:+} other_io {:+}",
                    $name,
                    handles() - h,
                    other_io() - io
                );
            }};
        }
        probe!(
            "create_placeholder(existing file)",
            placeholders::create_placeholder_with(&dir, "dir0/file0.bin", "file0.bin", 1234, 1_700_000_000, false, false)
        );
        probe!(
            "create_placeholder(existing dir)",
            placeholders::create_placeholder_with(&r.root, "dir0", "dir0", 0, 1_700_000_000, true, false)
        );
        probe!("ensure_directory", crate::config::ensure_directory(&dir));
        probe!("CheckedDirs::check", crate::name_alias::CheckedDirs::new(&r.root).check(&dir));
        probe!("conflict_below", crate::name_alias::conflict_below(&r.root, &file));
        probe!("platform_view", placeholders::platform_view(&file));
        probe!("list_dir_attributes", placeholders::list_dir_attributes(&dir));
        probe!("read_os_placeholder_state", placeholders::read_os_placeholder_state(&file));
        probe!("placeholder_in_sync", placeholders::placeholder_in_sync(&file));
        probe!("is_cloud_placeholder", placeholders::is_cloud_placeholder(&file));
        probe!("fully_on_disk", placeholders::fully_on_disk(&file));
        probe!("change_stamp", placeholders::change_stamp(&file));
        probe!("fs::metadata", std::fs::metadata(&file));
        probe!("placeholder_local_entry", placeholders::placeholder_local_entry(&file));
        probe!("mark_not_in_sync_path", placeholders::mark_not_in_sync_path(&file));
        probe!("identity_at", identity_at::identity_at(&file));
        let cursor = std::sync::atomic::AtomicUsize::new(0);
        probe!("reconcile_placeholder_state_with", reconcile_placeholder_state_with(&r.db, &r.root, &cursor));
    }
}
