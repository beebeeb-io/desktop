//! LD-0113 (spec §6.2, §6.3, Amendment 6): what this device refuses to delete, rename or move in its
//! drive.
//!
//! Guus, 2026-11-06 17:49, "Protect the whole tree": in Explorer (and Finder) nothing at or under the
//! Backups root (the row the server flagged `backups_root`, `files.system_role`) is trashed, deleted,
//! renamed or moved, and nothing is moved into it from elsewhere in the drive. Backups are managed in
//! the app; opening and downloading them stay. The rule and its words come from core
//! (`beebeeb_core::backup::check_drive_mutation`), in this order:
//!
//! - **The root** has its own words (`MSG_DELETE`, `MSG_RENAME`, `MSG_MOVE`).
//! - **This device's own backup of a folder it backs up now** (`Backups/<this device>/<Folder>` and
//!   everything under it, while that folder is on) says where the original is (`own_backup_refusal`):
//!   a change there would only come back on the next pass, because the original still has the file.
//! - **Anything else under the root** (another device's backups, this device's folder itself, a folder
//!   that is turned off) has `MSG_MANAGED_IN_APP`, and so does a move into the tree.
//!
//! [`protected_reason`] and [`protected_destination`] answer from the state DB and the in-memory
//! [`OwnBackups`] only: no network, no config read, so the Windows delete and rename callbacks can ask
//! them on every delete without making Explorer wait. The answer is shown to the person, never logged:
//! the own-backup words carry a path on this device.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use beebeeb_core::backup::{Mutation, Protected, check_drive_mutation};

use crate::state_db::StateDb;

/// One folder this device backs up now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OwnBackupFolder {
    /// The category folder's name under the device folder (`"Pictures"`).
    pub(crate) display_name: String,
    /// Where the original folder is on this device (`C:\Users\guus\Pictures`).
    pub(crate) source_path: PathBuf,
}

/// What this device backs up now, and under which device folder of the Backups root.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct OwnBackups {
    /// The device folder's name under the root (`"GLegion"`); `None` when the device has none.
    pub(crate) device_folder: Option<String>,
    /// The folders that are on, in the catalog's order.
    pub(crate) folders: Vec<OwnBackupFolder>,
}

static OWN: RwLock<OwnBackups> = RwLock::new(OwnBackups {
    device_folder: None,
    folders: Vec::new(),
});

/// Replace what this device backs up now. The runner calls [`refresh_own_backups`]; tests set it.
pub(crate) fn set_own_backups(own: OwnBackups) {
    if let Ok(mut slot) = OWN.write() {
        *slot = own;
    }
}

/// What this device backs up now, as last set.
pub(crate) fn own_backups() -> OwnBackups {
    OWN.read().map(|own| own.clone()).unwrap_or_default()
}

/// [`own_backups`] without the copy, for a caller that asks once per item of a listing.
pub(crate) fn with_own_backups<T>(f: impl FnOnce(&OwnBackups) -> T) -> T {
    match OWN.read() {
        Ok(own) => f(&own),
        Err(_) => f(&OwnBackups::default()),
    }
}

/// Read what this device backs up now from `desktop.toml` and set it. Windows resolves each enabled
/// known folder's original (`SHGetKnownFolderPath`, no network) under the device folder the backup
/// writes today (`known_folder::sanitize_device_segment` of the computer name). The other systems
/// have no backup sources yet (macOS's arrive with LD-0113 D5), so only the root is protected there.
/// A config that cannot be read keeps the last answer: a read error never lifts a protection.
pub(crate) fn refresh_own_backups() {
    match crate::config::DesktopConfig::load() {
        Ok(cfg) => set_own_backups(own_backups_for(&cfg.known_folder_backup)),
        Err(error) => tracing::debug!(%error, "backup protection: config unreadable; keeping the last answer"),
    }
}

#[cfg(target_os = "windows")]
fn own_backups_for(enabled_keys: &[String]) -> OwnBackups {
    let folders = crate::known_folder::KNOWN_FOLDERS
        .iter()
        .filter(|folder| enabled_keys.iter().any(|key| key == folder.key))
        .filter_map(|folder| {
            Some(OwnBackupFolder {
                display_name: folder.display_name.to_string(),
                source_path: crate::known_folder::resolve_known_folder(folder.key)?,
            })
        })
        .collect();
    OwnBackups {
        device_folder: crate::runner::device_name().map(|name| crate::known_folder::sanitize_device_segment(&name)),
        folders,
    }
}

#[cfg(not(target_os = "windows"))]
fn own_backups_for(_enabled_keys: &[String]) -> OwnBackups {
    OwnBackups::default()
}

/// Why the item at `rel_path` (the state DB's `/`-joined path) must not be changed by `mutation`, in
/// the words to show; `None` when it may. See the module doc for what is protected. A state DB that
/// cannot be read answers `None`: Explorer is never held on an error, and a delete that gets through
/// is restored after the fact ([`crate::watcher::handle_delete`]).
pub(crate) fn protected_reason(db: &StateDb, own: &OwnBackups, rel_path: &str, mutation: Mutation) -> Option<String> {
    let [reason] = protected_reasons(db, own, rel_path, [mutation]);
    reason
}

/// [`protected_reason`] for several mutations of one item, reading the roots once (a Finder listing
/// asks for three per item). All `None` outside the tree.
pub(crate) fn protected_reasons<const N: usize>(
    db: &StateDb,
    own: &OwnBackups,
    rel_path: &str,
    mutations: [Mutation; N],
) -> [Option<String>; N] {
    let Some(below_root) = below_a_root(db, rel_path.trim_matches('/')) else {
        return [const { None }; N];
    };
    let own_source = own_active_source(own, below_root);
    mutations.map(|m| words(check_drive_mutation(below_root.is_empty(), !below_root.is_empty(), own_source.as_deref(), m)))
}

/// Why an item from outside the Backups tree must not be moved to `target_rel` (the state DB path it
/// would have there); `None` when the target is not in the tree. The item itself is not protected, so
/// the words are the tree's: core's for an item under the root that is not this device's own backup.
pub(crate) fn protected_destination(db: &StateDb, target_rel: &str) -> Option<String> {
    below_a_root(db, target_rel.trim_matches('/'))?;
    words(check_drive_mutation(false, true, None, Mutation::Move))
}

/// Why the item at `source_rel` must not be renamed or moved (`mutation`) to `target_rel` (`None`: a
/// target outside this drive, such as the Recycle Bin, or one that could not be read): the item is in
/// the Backups tree (its own words), or the target is (a move into the tree). The Windows rename
/// callback and the watcher's after-the-fact check both ask this.
pub(crate) fn move_refusal(
    db: &StateDb,
    own: &OwnBackups,
    source_rel: &str,
    mutation: Mutation,
    target_rel: Option<&str>,
) -> Option<String> {
    protected_reason(db, own, source_rel, mutation).or_else(|| protected_destination(db, target_rel?))
}

/// A drive surface's delete, rename or move that the app refuses before anything is queued (the
/// shared queueing functions, behind the Windows callbacks and the Finder extension). Its text is
/// core's words, which the File Provider reply carries to Finder.
#[derive(Debug)]
pub(crate) struct DriveChangeRefused {
    pub(crate) message: String,
}

impl std::fmt::Display for DriveChangeRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for DriveChangeRefused {}

fn words(rule: Result<(), Protected>) -> Option<String> {
    rule.err().map(|protected| protected.message.into_owned())
}

/// `rel` below the flagged Backups root it is at or under (`""` for the root itself), or `None` when it
/// is in no Backups tree or the state DB cannot be read.
fn below_a_root<'a>(db: &StateDb, rel: &'a str) -> Option<&'a str> {
    if rel.is_empty() {
        return None;
    }
    let roots = match db.backups_roots() {
        Ok(roots) => roots,
        Err(error) => {
            // Zero-knowledge: no path, only that the read failed.
            tracing::warn!(%error, "backup protection: the Backups root could not be read");
            return None;
        }
    };
    roots.iter().find_map(|(_, root_path)| strip_segments(rel, root_path.trim_matches('/')))
}

/// The original of the item at `below_root` (`GLegion/Pictures/2024/a.jpg`) when it is this device's
/// own backup of a folder it backs up now: that folder's backup and everything under it. The device
/// folder itself holds several backups, not one, so it has none.
fn own_active_source(own: &OwnBackups, below_root: &str) -> Option<String> {
    let below_device = strip_segments(below_root, own.device_folder.as_deref()?)?;
    if below_device.is_empty() {
        return None;
    }
    let (category, below_category) = below_device.split_once('/').unwrap_or((below_device, ""));
    let folder = own.folders.iter().find(|folder| same_name(&folder.display_name, category))?;
    Some(display(&original_of(&folder.source_path, below_category)))
}

/// Every `local` row at or under `rel` (the item and its subtree) whose file is gone from this device
/// is cloud-only again, so the placeholder pass mints it again. Returns how many. Used by the
/// after-the-fact restore of a protected item and, for each Backups root, at start-up (a downloaded
/// file deleted while no callback could answer).
pub(crate) fn mark_missing_cloud_only(db: &StateDb, sync_root: &Path, rel: &str) -> usize {
    let rows = db.local_rows_at_or_under(rel).unwrap_or_else(|error| {
        tracing::warn!(%error, "protected item: its rows could not be read; the placeholder pass mints what it can");
        Vec::new()
    });
    rows.into_iter()
        // A row may be stored `/Backups/…`; the on-disk lookup takes the slash-free form.
        .filter(|(_, path)| {
            crate::engine_bridge::local_file_path_under_sync_root(sync_root, path.trim_start_matches('/'))
                .is_ok_and(|on_disk| !on_disk.exists())
        })
        .filter(|(file_id, _)| match db.set_status(file_id, crate::state_db::FileStatus::CloudOnly) {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(%error, %file_id, "protected item: could not mark it cloud-only");
                false
            }
        })
        .count()
}

/// Start-up (LD-0113 review #4): [`mark_missing_cloud_only`] for each Backups root, before the first
/// placeholder pass. Zero-knowledge: a count only. Windows calls it (`windows_cf::seed_placeholders`).
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn reconcile_backups_tree(db: &StateDb, sync_root: &Path) {
    let roots = db.backups_roots().unwrap_or_default();
    let restored: usize = roots.iter().map(|(_, root)| mark_missing_cloud_only(db, sync_root, root)).sum();
    if restored > 0 {
        tracing::info!(restored, "backup protection: downloaded items missing from the Backups tree are put back");
    }
}

/// A placeholder pass for what a refusal put back, off the async task that refused it: the pass walks
/// every cloud-only row and calls Cloud Files per item.
#[cfg(target_os = "windows")]
pub(crate) fn restore_placeholders_soon(db: std::sync::Arc<StateDb>, sync_root: &Path) {
    let sync_root = sync_root.to_path_buf();
    let pass = move || {
        let restored = crate::windows_cf::restore_placeholders(&db, &sync_root);
        // Zero-knowledge: a count, never the item.
        tracing::info!(restored, "protected item: placeholders put back");
    };
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => drop(handle.spawn_blocking(pass)),
        Err(_) => pass(),
    }
}

/// What a local rename from `source` to `target` (absolute paths) does to the item: Explorer's
/// "Delete" is a move into the Recycle Bin (`<volume>\$Recycle.Bin\…`), so that is a trash; another
/// folder is a move; the same folder is a rename.
pub(crate) fn rename_mutation(source: &Path, target: &Path) -> Mutation {
    let folder = |path: &Path| path.parent().map(|parent| parent.to_string_lossy().to_lowercase());
    if target.components().any(|part| part.as_os_str().eq_ignore_ascii_case("$Recycle.Bin")) {
        Mutation::Trash
    } else if folder(source) != folder(target) {
        Mutation::Move
    } else {
        Mutation::Rename
    }
}

/// `path` with its leading segments equal to `prefix`'s removed (`""` when they are all of it), or
/// `None` when `path` is not `prefix` or under it. Names compare case-insensitively: Explorer and
/// Finder treat `Backups` and `backups` as one folder.
fn strip_segments<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    if prefix.is_empty() {
        return None;
    }
    let mut rest = path;
    for want in prefix.split('/') {
        let (head, tail) = rest.split_once('/').unwrap_or((rest, ""));
        if head.is_empty() || !same_name(head, want) {
            return None;
        }
        rest = tail;
    }
    Some(rest)
}

fn same_name(a: &str, b: &str) -> bool {
    a == b || a.to_lowercase() == b.to_lowercase()
}

/// The original of a backed-up item: the source folder joined with the item's `/`-joined path below
/// the category folder, with this system's separator.
fn original_of(source: &Path, below_category: &str) -> PathBuf {
    below_category
        .split('/')
        .filter(|segment| !segment.is_empty())
        .fold(source.to_path_buf(), |path, segment| path.join(segment))
}

fn display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Where a refusal's words are shown on Windows: Explorer shows only its own "access denied" for a
/// refused delete or rename, so the words reach the person as a notification. Set once by the runner.
/// Not in a test build: cloning and dropping an `AppHandle` where a test can reach it links the window
/// runtime (comctl32, user32, dwmapi, gdi32) into the lib test binary, which then fails to load on
/// Windows (STATUS_ENTRYPOINT_NOT_FOUND, measured on 115826a); no test has an app to show it through.
#[cfg(all(target_os = "windows", not(test)))]
static NOTIFIER: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

/// Which refusals the person is told about. Windows asks the delete callback several times for one
/// delete (4 times for one `del` of one file on the LD-0113 native rung), and one folder delete can
/// refuse many items, so: the same words at most once per [`NoticeGate::QUIET`], and no other words
/// within [`NoticeGate::FLOOR`] of the last notice. Every callback is still answered; only the
/// notice is held back.
#[derive(Debug, Default)]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) struct NoticeGate {
    shown: std::collections::HashMap<String, std::time::Instant>,
    last: Option<std::time::Instant>,
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
impl NoticeGate {
    pub(crate) const QUIET: std::time::Duration = std::time::Duration::from_secs(5);
    pub(crate) const FLOOR: std::time::Duration = std::time::Duration::from_secs(1);

    /// Whether `message` is to be shown at `now`; a notice that is due is recorded as shown.
    pub(crate) fn due(&mut self, message: &str, now: std::time::Instant) -> bool {
        let since = |at: std::time::Instant| now.saturating_duration_since(at);
        if self.last.is_some_and(|at| since(at) < Self::FLOOR)
            || self.shown.get(message).is_some_and(|at| since(*at) < Self::QUIET)
        {
            return false;
        }
        self.shown.retain(|_, at| since(*at) < Self::QUIET);
        self.shown.insert(message.to_string(), now);
        self.last = Some(now);
        true
    }
}

#[cfg(target_os = "windows")]
static NOTICES: std::sync::Mutex<Option<NoticeGate>> = std::sync::Mutex::new(None);

/// The app the refusal notifications are shown through (the runner, once per process).
#[cfg(all(target_os = "windows", not(test)))]
pub(crate) fn set_notifier(app: &tauri::AppHandle) {
    let _ = NOTIFIER.set(app.clone());
}

#[cfg(all(target_os = "windows", test))]
pub(crate) fn set_notifier(_app: &tauri::AppHandle) {}

/// Show a refusal's words to the person, when [`NoticeGate`] says they are due. Never blocks the
/// caller (a Cloud Files callback calls it after it answered Windows): the notification is shown on
/// its own thread. Returns whether the words were due.
#[cfg(target_os = "windows")]
pub(crate) fn note_refusal(message: &str) -> bool {
    let due = NOTICES
        .lock()
        .is_ok_and(|mut gate| gate.get_or_insert_with(NoticeGate::default).due(message, std::time::Instant::now()));
    if due {
        show_notification(message);
    }
    due
}

/// The toast itself, on its own thread (see [`NOTIFIER`] for why a test build has none).
#[cfg(all(target_os = "windows", not(test)))]
fn show_notification(message: &str) {
    let Some(app) = NOTIFIER.get().cloned() else {
        return;
    };
    let body = message.to_string();
    std::thread::spawn(move || {
        use tauri_plugin_notification::NotificationExt;
        if let Err(error) = app.notification().builder().title("Beebeeb").body(body).show() {
            // Zero-knowledge: the words carry a path; only the failure is logged.
            tracing::debug!(%error, "backup protection: the refusal notification could not be shown");
        }
    });
}

#[cfg(all(target_os = "windows", test))]
fn show_notification(_message: &str) {}

#[cfg(test)]
#[path = "backup_protect_tests.rs"]
mod tests;
