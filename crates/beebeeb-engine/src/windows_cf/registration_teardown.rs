//! Task drive-complete-006b: what the uninstaller asks the app to do before its files go (`beebeeb-drive.exe
//! --uninstall-sync-folder keep|remove`, run by `packaging/windows/hooks.nsh` once the app has stopped).
//!
//! For every sync folder of THIS identity (the Explorer registrations whose id starts `<provider>!`, plus the
//! config's `sync_root`), in this order:
//!   1. [`SyncFolderChoice::KeepDownloaded`] (the default): every placeholder wholly on this PC becomes a plain file
//!      or folder in place (`CfRevertPlaceholder`, children first); a file whose bytes are not all here (cloud-only
//!      or partly fetched) is removed, since it cannot be opened once the folder is unregistered and its content is
//!      on the server. A file that is no placeholder (made here, never sent) and a link are never touched.
//!   2. The Explorer registration goes: the status flyout and thumbnail bindings and their COM classes, the
//!      navigation-pane entry (`StorageProviderSyncRootManager::Unregister`) and our per-user SyncRootManager key
//!      ([`super::unregister_shell_sync_root`]).
//!   3. The Cloud Files registration goes ([`super::unregister_sync_root`]: refused for another provider's folder).
//!   4. [`SyncFolderChoice::Remove`]: the folder is deleted, unless a known folder (Desktop, Documents, …) lives in
//!      it; then it is kept as in step 1 and the report says so.
//!
//! Never fetches: no file is opened for its data. A failure on one item is counted and the rest goes on; the
//! process exit code says whether anything was left (the uninstaller carries on either way).

use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};

use windows::Win32::Storage::CloudFilters::*;
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS;

use crate::identity::EngineIdentity;

/// What happens to the sync folder's content on uninstall.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncFolderChoice {
    /// Downloaded files stay as plain files; cloud-only ones go.
    KeepDownloaded,
    /// The whole folder goes.
    Remove,
}

impl SyncFolderChoice {
    /// The uninstaller's word: `keep` or `remove`.
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "keep" => Some(Self::KeepDownloaded),
            "remove" => Some(Self::Remove),
            _ => None,
        }
    }
}

/// What one uninstall did, for the log (counts only: names are never logged).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TeardownReport {
    pub roots: usize,
    /// Placeholders (files and folders) made plain.
    pub kept: usize,
    /// Cloud-only or partly fetched files removed.
    pub cloud_only_removed: usize,
    /// Folders deleted whole ([`SyncFolderChoice::Remove`]).
    pub folders_removed: usize,
    /// Remove refused because a known folder is inside; kept instead.
    pub remove_refused: usize,
    /// Items or steps that failed.
    pub failures: usize,
}

/// What the keep walk does with one entry, from what a reparse-point open reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntryAction {
    /// A link: leave it, never follow it.
    Leave,
    /// Make it plain if it is a placeholder (a plain file answers "not a cloud file" and is left as it is). A
    /// placeholder wholly on disk does not show the reparse bit to an ordinary process (Windows disguises it), so
    /// the attempt is the test.
    Revert,
    /// A file whose bytes are not all on this PC (`RECALL_ON_DATA_ACCESS`, set only by the Cloud Files filter):
    /// remove it, its content is on the server.
    RemoveCloudOnly,
}

/// Pure: the action for an entry with these attributes. `is_link` is a symlink or a junction (never followed).
pub(crate) fn entry_action(is_dir: bool, is_link: bool, attributes: u32) -> EntryAction {
    if is_link {
        return EntryAction::Leave;
    }
    if !is_dir && attributes & FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS.0 != 0 {
        return EntryAction::RemoveCloudOnly;
    }
    EntryAction::Revert
}

/// Pure: whether `path` is `dir` or under it, ignoring case (NTFS).
fn inside(path: &Path, dir: &Path) -> bool {
    let (path, dir) = (
        path.to_string_lossy().to_lowercase(),
        dir.to_string_lossy().to_lowercase(),
    );
    let (path, dir) = (path.trim_end_matches(['/', '\\']), dir.trim_end_matches(['/', '\\']));
    path == dir || path.strip_prefix(dir).is_some_and(|rest| rest.starts_with(['/', '\\']))
}

/// Pure: whether deleting `root` would take a known folder with it (`known` as the shell resolves them now).
pub(crate) fn remove_would_take_known_folder(root: &Path, known: &[PathBuf]) -> bool {
    known.iter().any(|folder| inside(folder, root))
}

/// Pure: the roots to tear down, from the Explorer registrations `(id, path)` and the config's root: those whose
/// id is this provider's (`<provider>!…`, so `Beebeeb!` never matches `BeebeebTest!`), then the config's, each once.
pub(crate) fn roots_to_tear_down(
    provider_name: &str,
    shell_roots: &[(String, PathBuf)],
    config_root: Option<&Path>,
) -> Vec<PathBuf> {
    let prefix = format!("{provider_name}!");
    let mut roots: Vec<PathBuf> = Vec::new();
    let candidates = shell_roots
        .iter()
        .filter(|(id, _)| id.starts_with(&prefix))
        .map(|(_, path)| path.as_path())
        .chain(config_root);
    for path in candidates {
        if !roots.iter().any(|known| inside(known, path) && inside(path, known)) {
            roots.push(path.to_path_buf());
        }
    }
    roots
}

/// Step 1 for one root: make every placeholder wholly on disk plain, remove every file placeholder that is not.
/// Children first, the root itself last. Counts into `report`.
pub(crate) fn keep_downloaded(root: &Path, report: &mut TeardownReport) {
    walk(root, report);
}

fn walk(path: &Path, report: &mut TeardownReport) {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        report.failures += 1;
        return;
    };
    // std reads a junction as a link too (a name-surrogate reparse point); a cloud placeholder is not one.
    let is_link = metadata.file_type().is_symlink();
    if metadata.is_dir() && !is_link {
        match std::fs::read_dir(path) {
            Ok(entries) => {
                for entry in entries {
                    match entry {
                        Ok(entry) => walk(&entry.path(), report),
                        Err(_) => report.failures += 1,
                    }
                }
            }
            Err(_) => report.failures += 1,
        }
    }
    match entry_action(metadata.is_dir(), is_link, metadata.file_attributes()) {
        EntryAction::Leave => {}
        EntryAction::Revert => match revert(path) {
            Ok(true) => report.kept += 1,
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(error = %error, "uninstall: a placeholder could not be made plain");
                report.failures += 1;
            }
        },
        EntryAction::RemoveCloudOnly => {
            let _writable = super::read_only::Lifted::new(path);
            match std::fs::remove_file(path) {
                Ok(()) => report.cloud_only_removed += 1,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    tracing::warn!(error = %error, "uninstall: a cloud-only file could not be removed");
                    report.failures += 1;
                }
            }
        }
    }
}

/// `CfRevertPlaceholder` on `path`. `Ok(false)` when it is not a cloud file after all.
fn revert(path: &Path) -> std::io::Result<bool> {
    const NOT_A_CLOUD_FILE: windows::core::HRESULT = windows::core::HRESULT(0x8007_0178_u32 as i32);
    let _writable = super::read_only::Lifted::new(path);
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: `wide` outlives both calls; the handle is closed on every path.
    unsafe {
        let handle = CfOpenFileWithOplock(
            windows::core::PCWSTR(wide.as_ptr()),
            CF_OPEN_FILE_FLAG_EXCLUSIVE | CF_OPEN_FILE_FLAG_WRITE_ACCESS,
        )
        .map_err(|e| std::io::Error::from_raw_os_error(e.code().0 & 0xFFFF))?;
        let reverted = CfRevertPlaceholder(handle, CF_REVERT_FLAG_NONE, None);
        CfCloseHandle(handle);
        match reverted {
            Ok(()) => Ok(true),
            Err(error) if error.code() == NOT_A_CLOUD_FILE => Ok(false),
            Err(error) => Err(std::io::Error::from_raw_os_error(error.code().0 & 0xFFFF)),
        }
    }
}

/// Steps 1 to 4 for one root, with the registration teardown passed in (a test passes the QA provider's).
pub(crate) fn tear_down_root(
    root: &Path,
    choice: SyncFolderChoice,
    known_folders: &[PathBuf],
    check_owner: impl FnOnce(&Path) -> anyhow::Result<()>,
    unregister_shell: impl FnOnce(&Path) -> anyhow::Result<()>,
    unregister_cloud_files: impl FnOnce(&Path) -> anyhow::Result<()>,
    report: &mut TeardownReport,
) {
    report.roots += 1;
    // Before anything is walked or deleted: a folder registered to another provider is never ours to touch.
    if let Err(error) = check_owner(root) {
        tracing::warn!(error = %error, "uninstall: the sync folder is not ours; left untouched");
        report.failures += 1;
        return;
    }
    let remove = match choice {
        SyncFolderChoice::Remove if remove_would_take_known_folder(root, known_folders) => {
            tracing::warn!("uninstall: a known folder is inside the sync folder; it is kept, not removed");
            report.remove_refused += 1;
            false
        }
        SyncFolderChoice::Remove => true,
        SyncFolderChoice::KeepDownloaded => false,
    };
    let exists = root.is_dir();
    if exists && !remove {
        keep_downloaded(root, report);
    }
    if let Err(error) = unregister_shell(root) {
        tracing::warn!(error = %error, "uninstall: Explorer registration not removed");
        report.failures += 1;
    }
    let unregistered = unregister_cloud_files(root);
    if let Err(error) = &unregistered {
        tracing::warn!(error = %error, "uninstall: Cloud Files registration not removed");
        report.failures += 1;
    }
    if exists && remove && unregistered.is_ok() {
        match std::fs::remove_dir_all(root) {
            Ok(()) => report.folders_removed += 1,
            Err(error) => {
                tracing::warn!(error = %error, "uninstall: the sync folder could not be removed");
                report.failures += 1;
            }
        }
    }
}

/// The Explorer sync roots registered for this user: `(id, path)`. Empty when they cannot be read.
fn shell_roots() -> Vec<(String, PathBuf)> {
    use windows::Storage::Provider::StorageProviderSyncRootManager;
    let mut found = Vec::new();
    let Ok(roots) = StorageProviderSyncRootManager::GetCurrentSyncRoots() else {
        return found;
    };
    for index in 0..roots.Size().unwrap_or(0) {
        let Ok(info) = roots.GetAt(index) else { continue };
        let (Ok(id), Ok(folder)) = (info.Id(), info.Path()) else {
            continue;
        };
        if let Ok(path) = folder.Path() {
            found.push((id.to_string(), PathBuf::from(path.to_string())));
        }
    }
    found
}

/// The uninstall: every sync folder of `identity`, torn down per `choice`. The app must not be running.
pub fn uninstall(identity: &'static EngineIdentity, choice: SyncFolderChoice) -> TeardownReport {
    let config_root = crate::config::DesktopConfig::load_for(identity)
        .ok()
        .and_then(|config| config.sync_root);
    let roots = roots_to_tear_down(identity.cloud_files_provider_name(), &shell_roots(), config_root.as_deref());
    let known: Vec<PathBuf> = crate::known_folder::KNOWN_FOLDERS
        .iter()
        .filter_map(|folder| crate::known_folder::resolve_known_folder_current(folder.key))
        .collect();
    let mut report = TeardownReport::default();
    for root in roots {
        tear_down_root(
            &root,
            choice,
            &known,
            // A folder no longer registered is fine; one registered to another provider is refused (never ours).
            |root| crate::sync_root_owner::check(identity, root).map_err(|refusal| anyhow::anyhow!(refusal)),
            |root| super::unregister_shell_sync_root(identity, root),
            super::unregister_any_sync_root,
            &mut report,
        );
    }
    tracing::info!(?report, "uninstall: sync folders torn down");
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::windows_cf::placeholders::{
        convert_to_in_sync_placeholder, create_placeholder,
    };

    /// Whether Windows holds a placeholder at `path` (read through `CfGetPlaceholderInfo`: a placeholder wholly on
    /// disk hides its reparse bit from the attributes).
    fn is_cloud_placeholder(path: &Path) -> bool {
        use std::os::windows::fs::OpenOptionsExt;
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
            FILE_SHARE_READ, FILE_SHARE_WRITE,
        };
        let file = std::fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES.0)
            .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0 | FILE_FLAG_BACKUP_SEMANTICS.0)
            .open(path)
            .unwrap();
        crate::windows_cf::placeholders::read_standard_info(windows::Win32::Foundation::HANDLE(file.as_raw_handle()))
            .unwrap()
            .is_some()
    }
    use crate::windows_cf::{HYDRATION_POLICY, QA_PROVIDER, register_sync_root_with, registered_provider_name};

    const REPARSE: u32 = 0x400;
    const RECALL: u32 = 0x0040_0000;

    #[test]
    fn registration_teardown_keeps_what_is_here_and_removes_what_is_not() {
        assert_eq!(entry_action(false, false, 0x20), EntryAction::Revert, "tried; a plain file answers no");
        assert_eq!(entry_action(false, false, REPARSE), EntryAction::Revert, "downloaded");
        assert_eq!(entry_action(false, false, RECALL), EntryAction::RemoveCloudOnly, "cloud-only, disguised");
        assert_eq!(entry_action(false, false, REPARSE | RECALL), EntryAction::RemoveCloudOnly);
        assert_eq!(entry_action(true, false, REPARSE | RECALL), EntryAction::Revert, "a folder is never removed");
        assert_eq!(entry_action(true, true, REPARSE), EntryAction::Leave, "a junction is not followed");
        assert_eq!(entry_action(false, true, REPARSE | RECALL), EntryAction::Leave, "a symlink");
    }

    #[test]
    fn registration_teardown_finds_only_this_providers_roots_once() {
        let shell = vec![
            ("Beebeeb!00aa".to_string(), PathBuf::from(r"C:\Users\a\Beebeeb")),
            ("BeebeebTest!00bb".to_string(), PathBuf::from(r"C:\Users\a\Beebeeb Test")),
            ("OneDrive!S-1!Personal".to_string(), PathBuf::from(r"C:\Users\a\OneDrive")),
        ];
        assert_eq!(
            roots_to_tear_down("Beebeeb", &shell, Some(Path::new(r"c:\users\a\beebeeb"))),
            vec![PathBuf::from(r"C:\Users\a\Beebeeb")]
        );
        assert_eq!(
            roots_to_tear_down("BeebeebTest", &shell, None),
            vec![PathBuf::from(r"C:\Users\a\Beebeeb Test")]
        );
        assert_eq!(
            roots_to_tear_down("Beebeeb", &[], Some(Path::new(r"D:\Sync"))),
            vec![PathBuf::from(r"D:\Sync")]
        );
    }

    #[test]
    fn registration_teardown_never_removes_a_folder_holding_a_known_folder() {
        let root = Path::new(r"C:\Users\a\Beebeeb");
        assert!(remove_would_take_known_folder(root, &[PathBuf::from(r"c:\users\a\beebeeb\Documents")]));
        assert!(!remove_would_take_known_folder(root, &[PathBuf::from(r"C:\Users\a\Documents")]));
        assert!(!remove_would_take_known_folder(root, &[PathBuf::from(r"C:\Users\a\Beebeeb Docs")]));
    }

    /// A QA sync root with a downloaded file in a downloaded folder, a cloud-only file and a file made here.
    struct QaRoot {
        _dir: tempfile::TempDir,
        root: PathBuf,
    }

    impl QaRoot {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("qa-root");
            std::fs::create_dir(&root).unwrap();
            register_sync_root_with(&root, &QA_PROVIDER, HYDRATION_POLICY).unwrap();
            std::fs::create_dir(root.join("folder")).unwrap();
            let downloaded = root.join("folder").join("downloaded.txt");
            std::fs::write(&downloaded, b"kept bytes").unwrap();
            convert_to_in_sync_placeholder(&downloaded, "f-1").unwrap();
            // Cloud-only: minted with no bytes (no provider is connected, so nothing can fetch it).
            assert!(create_placeholder(&root, "f-2", "cloud.bin", 4096, 0, false).unwrap());
            std::fs::write(root.join("made-here.txt"), b"never sent").unwrap();
            assert!(is_cloud_placeholder(&root.join("folder").join("downloaded.txt")));
            assert!(is_cloud_placeholder(&root.join("cloud.bin")));
            Self { _dir: dir, root }
        }
    }

    impl Drop for QaRoot {
        fn drop(&mut self) {
            let _ = crate::windows_cf::unregister_any_sync_root(&self.root);
        }
    }

    fn not_registered(root: &Path) -> bool {
        registered_provider_name(root).is_err()
    }

    #[test]
    fn registration_teardown_keep_leaves_downloaded_files_as_plain_files() {
        let qa = QaRoot::new();
        let mut report = TeardownReport::default();
        tear_down_root(
            &qa.root,
            SyncFolderChoice::KeepDownloaded,
            &[],
            |_| Ok(()),
            |_| Ok(()),
            crate::windows_cf::unregister_any_sync_root,
            &mut report,
        );
        assert_eq!(report.failures, 0, "{report:?}");
        assert!(not_registered(&qa.root), "the sync root is gone");
        let downloaded = qa.root.join("folder").join("downloaded.txt");
        assert!(!is_cloud_placeholder(&downloaded), "a plain file now");
        assert_eq!(std::fs::read(&downloaded).unwrap(), b"kept bytes");
        assert!(!qa.root.join("cloud.bin").exists(), "the cloud-only file is gone");
        assert_eq!(std::fs::read(qa.root.join("made-here.txt")).unwrap(), b"never sent");
        assert_eq!(report.cloud_only_removed, 1);
        assert!(report.kept >= 1, "{report:?}");
    }

    #[test]
    fn registration_teardown_remove_deletes_the_folder() {
        let qa = QaRoot::new();
        let mut report = TeardownReport::default();
        tear_down_root(
            &qa.root,
            SyncFolderChoice::Remove,
            &[],
            |_| Ok(()),
            |_| Ok(()),
            crate::windows_cf::unregister_any_sync_root,
            &mut report,
        );
        assert_eq!(report.failures, 0, "{report:?}");
        assert!(!qa.root.exists(), "the folder is gone");
        assert_eq!(report.folders_removed, 1);
    }

    #[test]
    fn registration_teardown_remove_with_a_known_folder_inside_keeps_instead() {
        let qa = QaRoot::new();
        let mut report = TeardownReport::default();
        tear_down_root(
            &qa.root,
            SyncFolderChoice::Remove,
            &[qa.root.join("folder")],
            |_| Ok(()),
            |_| Ok(()),
            crate::windows_cf::unregister_any_sync_root,
            &mut report,
        );
        assert_eq!(report.remove_refused, 1);
        assert!(qa.root.join("folder").join("downloaded.txt").exists());
        assert!(not_registered(&qa.root));
    }

    #[test]
    fn registration_teardown_leaves_a_folder_that_is_not_ours_untouched() {
        for choice in [SyncFolderChoice::KeepDownloaded, SyncFolderChoice::Remove] {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("Other");
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(root.join("theirs.txt"), b"theirs").unwrap();
            let mut report = TeardownReport::default();
            tear_down_root(
                &root,
                choice,
                &[],
                |_| Err(anyhow::anyhow!("registered to another provider")),
                |_| panic!("must not unregister the shell root"),
                |_| panic!("must not unregister the cloud root"),
                &mut report,
            );
            assert_eq!(report.failures, 1, "{report:?}");
            assert_eq!(report.folders_removed, 0);
            assert_eq!(std::fs::read(root.join("theirs.txt")).unwrap(), b"theirs");
        }
    }

    #[test]
    fn registration_teardown_remove_keeps_the_folder_when_unregister_fails() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Mine");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.txt"), b"a").unwrap();
        let mut report = TeardownReport::default();
        tear_down_root(
            &root,
            SyncFolderChoice::Remove,
            &[],
            |_| Ok(()),
            |_| Ok(()),
            |_| Err(anyhow::anyhow!("refused")),
            &mut report,
        );
        assert_eq!(report.failures, 1, "{report:?}");
        assert!(root.join("a.txt").exists());
    }
}
