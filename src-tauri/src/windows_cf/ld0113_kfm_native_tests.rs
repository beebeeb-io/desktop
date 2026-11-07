//! LD-0113 Amendment 9, the Windows-native rung: a known folder OneDrive moved, backed up from OneDrive's folder.
//! OneDrive's folder is a temporary Cloud Files root registered under the QA provider (never the app's provider,
//! never the person's folders), holding 2 local files and 1 cloud-only placeholder (`RECALL_ON_DATA_ACCESS`). The
//! pass runs over a host that records what it is asked to upload. Nothing here talks to a server.

use std::collections::HashMap;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::MetadataExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use beebeeb_backup::{BackupHost, DeviceTree, HostError, UploadRequest};
use beebeeb_core::backup::{BackupRecord, FolderPresence, PassBudget, RecordRef, SkipReason};
use beebeeb_types::backup::Category;
use uuid::Uuid;
use windows::Win32::Storage::CloudFilters::*;
use windows::core::PCWSTR;

use crate::backup::kfm::KnownFolderPaths;
use crate::config::KnownFolderSource;

static FETCHES: AtomicUsize = AtomicUsize::new(0);

/// FETCH_DATA: counted and never answered. Reading the placeholder would hang here, so the count is the proof.
unsafe extern "system" fn count_fetch(_info: *const CF_CALLBACK_INFO, _params: *const CF_CALLBACK_PARAMETERS) {
    FETCHES.fetch_add(1, Ordering::SeqCst);
}

/// The host side of the pass: an empty first backup; every upload request is recorded.
#[derive(Default)]
struct RecordingHost {
    uploads: Mutex<Vec<String>>,
}

impl BackupHost for RecordingHost {
    async fn device_tree(&self, _c: &Category) -> Result<DeviceTree, HostError> {
        Ok(DeviceTree {
            device_folder_live: true,
            category_folder_live: None,
            folders: HashMap::from([(String::new(), Uuid::from_u128(1))]),
            files: Vec::new(),
            complete: true,
        })
    }
    async fn ensure_folder(&self, _parent: Uuid, _name: &str) -> Result<Uuid, HostError> {
        Ok(Uuid::new_v4())
    }
    async fn move_file(&self, _id: Uuid, _new_parent: Uuid, _new_name: &str) -> Result<(), HostError> {
        Ok(())
    }
    async fn enqueue_upload(&self, req: UploadRequest) -> Result<(), HostError> {
        self.uploads.lock().unwrap().push(req.key);
        Ok(())
    }
    async fn records(&self, _c: &Category) -> Result<Vec<BackupRecord>, HostError> {
        Ok(Vec::new())
    }
    async fn held_keys(&self, _c: &Category) -> Result<Vec<String>, HostError> {
        Ok(Vec::new())
    }
    async fn put_record(&self, _c: &Category, _e: &RecordRef, _r: BackupRecord) -> Result<bool, HostError> {
        Ok(true)
    }
    async fn mark_removed_elsewhere(&self, _c: &Category, _r: &[RecordRef]) -> Result<(), HostError> {
        Ok(())
    }
    async fn turn_off(&self, _c: &Category, _why: FolderPresence) -> Result<(), HostError> {
        Ok(())
    }
}

/// Amendment 9: with OneDrive's folder chosen, a fixture of 2 local files and 1 recall-on-access placeholder gives 2
/// uploads, 1 `NotLocal` skip and 0 fetches; the placeholder is still cloud-only afterwards.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kfm_onedrive_choice_backs_up_local_files_only() {
    let dir = tempfile::tempdir().unwrap();
    let default = dir.path().join("Default").join("Pictures");
    std::fs::create_dir_all(&default).unwrap();
    let onedrive = dir.path().join("OneDrive").join("Pictures");
    std::fs::create_dir_all(&onedrive).unwrap();
    let onedrive = PathBuf::from(std::fs::canonicalize(&onedrive).unwrap().display().to_string().trim_start_matches(r"\\?\"));
    crate::windows_cf::register_sync_root_with(&onedrive, &crate::windows_cf::QA_PROVIDER, crate::windows_cf::HYDRATION_POLICY)
        .unwrap();
    struct Unregister(PathBuf, Option<CF_CONNECTION_KEY>);
    impl Drop for Unregister {
        fn drop(&mut self) {
            if let Some(key) = self.1.take() {
                // SAFETY: the key of the connection made below, disconnected once.
                let _ = unsafe { CfDisconnectSyncRoot(key) };
            }
            let _ = crate::windows_cf::unregister_sync_root(&self.0);
        }
    }
    let mut guard = Unregister(onedrive.clone(), None);
    let table = [
        CF_CALLBACK_REGISTRATION { Type: CF_CALLBACK_TYPE_FETCH_DATA, Callback: Some(count_fetch) },
        CF_CALLBACK_REGISTRATION { Type: CF_CALLBACK_TYPE_NONE, Callback: None },
    ];
    let wide: Vec<u16> = onedrive.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: the path and the table outlive the call; no context is passed.
    let key = unsafe { CfConnectSyncRoot(PCWSTR(wide.as_ptr()), table.as_ptr(), None, CF_CONNECT_FLAG_NONE) }.unwrap();
    guard.1 = Some(key);

    std::fs::write(onedrive.join("beach.jpg"), b"local photo").unwrap();
    std::fs::write(onedrive.join("dog.jpg"), b"another local photo").unwrap();
    assert!(crate::windows_cf::placeholders::create_placeholder(&onedrive, "kfm-cloud", "cloud.jpg", 4096, 1_700_000_000, false).unwrap());
    let recall = |path: &std::path::Path| std::fs::symlink_metadata(path).unwrap().file_attributes() & 0x0040_0000 != 0;
    assert!(recall(&onedrive.join("cloud.jpg")), "the premise: the placeholder is recall-on-access");

    let db = crate::state_db::StateDb::open(dir.path().join("state.db")).unwrap();
    let choices = std::collections::BTreeMap::from([("pictures".to_string(), KnownFolderSource::OneDrive)]);
    let paths = |_: &str| KnownFolderPaths { default: Some(default.clone()), current: Some(onedrive.clone()) };
    let read = crate::backup::sources::backup_sources_with(Some(&db), &["pictures".to_string()], &choices, paths);
    assert_eq!(read.sources.len(), 1);
    assert_eq!(read.sources[0].path, onedrive, "OneDrive's folder is read");

    let host = RecordingHost::default();
    let budget = PassBudget { max_uploads: 100, max_bytes: u64::MAX };
    let report = beebeeb_backup::run_pass(&host, &read.sources[0].path, &[], &Category::Pictures, budget).await.unwrap();
    let mut uploads = host.uploads.lock().unwrap().clone();
    uploads.sort();
    println!("ld0113 kfm uploads={uploads:?} skipped={:?} fetches={}", report.skipped, FETCHES.load(Ordering::SeqCst));
    assert_eq!(uploads, ["beach.jpg", "dog.jpg"], "2 uploads, the local files only");
    assert_eq!(report.skipped.get(&SkipReason::NotLocal).copied(), Some(1), "1 NotLocal skip");
    assert_eq!(FETCHES.load(Ordering::SeqCst), 0, "0 fetches: the placeholder is never read");
    assert!(recall(&onedrive.join("cloud.jpg")), "the placeholder is still cloud-only");
    assert_eq!(std::fs::read_dir(&default).unwrap().count(), 0, "nothing is written anywhere");
}
