//! LD-0113 (spec §6.2, §6.3, Amendments 6, 7 and 8): what this device refuses to change in its drive.
//!
//! Guus, 2026-11-06 17:49, "Protect the whole tree", and 19:30, "Makes sense, do it" (read-only,
//! contents included): in Explorer (and Finder) nothing at or under the Backups root (the row the
//! server flagged `backups_root`, `files.system_role`) is trashed, deleted, renamed, moved or written,
//! and nothing is moved into it from elsewhere in the drive. Backups are managed in the app; opening and
//! downloading them stay. The rule and its words come from core
//! (`beebeeb_core::backup::check_drive_mutation`, on the row's [`BackupPosition`]), in this order:
//!
//! - **The root** has its own words for a delete, rename or move (`MSG_DELETE`, `MSG_RENAME`,
//!   `MSG_MOVE`).
//! - **This device's own backup of a folder it backs up now** (`Backups/<this device>/<Folder>` and
//!   everything under it, while that folder is on) says where the original is (`own_backup_refusal`):
//!   a change there would only come back on the next pass, because the original still has the file.
//! - **Anything else under the root** (another device's backups, this device's folder itself, a folder
//!   that is turned off) has `MSG_MANAGED_IN_APP` for a delete, rename or move, and `MSG_READ_ONLY` for
//!   a content change. A device's manifest (`.device.json`, [`BackupPosition::Manifest`]) has
//!   `MSG_READ_ONLY` for everything, and no drive or app surface lists it ([`BackupsTree::is_hidden`]).
//! - **A move into the tree** is core's `Mutation::Create` on the folder it would go into
//!   ([`protected_destination`]): `MSG_READ_ONLY`.
//!
//! [`protected_reason`] and [`protected_destination`] answer from the state DB and the in-memory
//! [`OwnBackups`] only: no network, no config read, so the Windows delete and rename callbacks can ask
//! them on every delete without making Explorer wait. The answer is shown to the person, never logged:
//! the own-backup words carry a path on this device.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use beebeeb_core::backup::{BackupPosition, Mutation, Protected, check_drive_mutation, is_hidden_backup_item};

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

/// Read what this device backs up now and set it: the folders that are on (`desktop.toml`), each with where its
/// original is read from, under the device folder the backup layout resolved (core's `find_device_folder`, so
/// `GLegion (a3f2)` after a name collision; never re-derived from the hostname). No network. A config that cannot
/// be read keeps the last answer: a read error never lifts a protection. See [`crate::backup::own_backups`].
pub(crate) fn refresh_own_backups(db: &StateDb) {
    match crate::backup::own_backups(db) {
        Some(own) => set_own_backups(own),
        None => tracing::debug!("backup protection: config unreadable; keeping the last answer"),
    }
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
    let tree = BackupsTree::read(db);
    let Some(below_root) = tree.below_root(rel_path) else {
        return [const { None }; N];
    };
    let own_source = own_active_source(own, below_root);
    mutations.map(|m| words(check_drive_mutation(position_below(below_root), own_source.as_deref(), m)))
}

/// Why an item from outside the Backups tree must not be moved to `target_rel` (the state DB path it
/// would have there); `None` when the target is not in the tree. It is core's `Create` on the folder the
/// item would go into (the root itself for `Backups/x`); the item is not a backup, so there are no
/// own-backup words.
pub(crate) fn protected_destination(db: &StateDb, target_rel: &str) -> Option<String> {
    let tree = BackupsTree::read(db);
    let below_root = tree.below_root(target_rel)?;
    let into = below_root.rsplit_once('/').map_or("", |(parent, _)| parent);
    words(check_drive_mutation(position_below(into), None, Mutation::Create))
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
/// shared queueing functions, behind the Finder extension). `message` is core's words, which the File
/// Provider reply carries to Finder (`IpcError::from_engine`); the error's text is fixed, because the
/// words can carry a path on this device and an error's text reaches the log.
#[derive(Debug)]
pub(crate) struct DriveChangeRefused {
    pub(crate) message: String,
}

impl std::fmt::Display for DriveChangeRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a drive change of a protected Backups item was refused")
    }
}

/// Who applies the drive rule to a queued change. `Check`: the queueing function does (the File
/// Provider replies, any new caller). `Decided`: the caller already did, and can put the item back,
/// which the queue cannot: the Windows watcher, which also passes the engine's own changes and a move
/// it could not undo (sent as made) on purpose. One authority per surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DriveGuard {
    Check,
    Decided,
}

impl std::error::Error for DriveChangeRefused {}

fn words(rule: Result<(), Protected>) -> Option<String> {
    rule.err().map(|protected| protected.message.into_owned())
}

/// The flagged Backups roots, read once for a caller that asks about many items (a listing, a
/// placeholder pass). A state DB that cannot be read gives an empty tree: nothing is protected or
/// hidden on an error, and Explorer is never held on one.
#[derive(Clone, Debug, Default)]
pub(crate) struct BackupsTree {
    /// Each root's `/`-joined path, without leading or trailing slashes.
    roots: Vec<String>,
}

impl BackupsTree {
    pub(crate) fn read(db: &StateDb) -> Self {
        match db.backups_roots() {
            Ok(roots) => Self {
                roots: roots
                    .into_iter()
                    .map(|(_, path)| path.trim_matches('/').to_string())
                    .filter(|path| !path.is_empty())
                    .collect(),
            },
            Err(error) => {
                // Zero-knowledge: no path, only that the read failed.
                tracing::warn!(%error, "backup protection: the Backups root could not be read");
                Self::default()
            }
        }
    }

    /// `rel` (a state DB path, with or without a leading slash) below the root it is at or under (`""`
    /// for the root itself), or `None` outside every Backups tree.
    pub(crate) fn below_root<'a>(&self, rel: &'a str) -> Option<&'a str> {
        let rel = rel.trim_matches('/');
        if rel.is_empty() {
            return None;
        }
        self.roots.iter().find_map(|root| strip_segments(rel, root))
    }

    /// Core's position of the row at `rel`.
    pub(crate) fn position(&self, rel: &str) -> BackupPosition {
        self.below_root(rel).map_or(BackupPosition::Outside, position_below)
    }

    /// Whether no surface may list the row at `rel`: core's `is_hidden_backup_item` (a device folder's
    /// manifest, `.device.json`).
    pub(crate) fn is_hidden(&self, rel: &str) -> bool {
        self.below_root(rel).is_some_and(|below| {
            let (depth, name) = depth_and_name(below);
            is_hidden_backup_item(Some(depth), name)
        })
    }
}

/// How deep `below_root` is under its root (0 for the root itself) and the row's own name.
fn depth_and_name(below_root: &str) -> (usize, &str) {
    if below_root.is_empty() {
        return (0, "");
    }
    (below_root.split('/').count(), below_root.rsplit('/').next().unwrap_or(""))
}

/// Core's position of a row `below_root` (as [`BackupsTree::below_root`] gives it).
fn position_below(below_root: &str) -> BackupPosition {
    let (depth, name) = depth_and_name(below_root);
    BackupPosition::at_depth(Some(depth), name)
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

/// Start-up (LD-0113 review #4): [`mark_missing_cloud_only`] for each Backups root. Returns how many
/// rows are cloud-only again. Zero-knowledge: a count only.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn reconcile_backups_tree(db: &StateDb, sync_root: &Path) -> usize {
    let roots = db.backups_roots().unwrap_or_default();
    let restored: usize = roots.iter().map(|(_, root)| mark_missing_cloud_only(db, sync_root, root)).sum();
    if restored > 0 {
        tracing::info!(restored, "backup protection: downloaded items missing from the Backups tree are put back");
    }
    restored
}

/// [`reconcile_backups_tree`] off the start-up path (it stats every downloaded item of the tree), then
/// a placeholder pass when anything came back, then the read-only state of each tree (Amendment 7: the
/// placeholders an older build minted take the attribute; Amendment 8a: the root's badge).
#[cfg(target_os = "windows")]
pub(crate) fn reconcile_backups_tree_soon(db: std::sync::Arc<StateDb>, sync_root: &Path) {
    let sync_root = sync_root.to_path_buf();
    let spawned = std::thread::Builder::new().name("backups-reconcile".into()).spawn(move || {
        if reconcile_backups_tree(&db, &sync_root) > 0 {
            restore_placeholders_soon(db.clone(), &sync_root);
        }
        for (_, root) in db.backups_roots().unwrap_or_default() {
            apply_tree_read_only(&db, &sync_root, &root, true);
        }
    });
    if let Err(error) = spawned {
        tracing::warn!(%error, "backup protection: the start-up reconcile could not start; the next start retries");
    }
}

/// LD-0113: the server flagged (or cleared) a folder's system role, and `set_system_role` stored the
/// change. On Windows the tree under it takes (or loses) its read-only state on disk
/// ([`apply_tree_read_only`]), off the caller's thread: it changes every file of the tree. Finder needs
/// nothing here: the change log names every row under the root, and the payload carries the state.
pub(crate) fn system_role_changed(db: &StateDb, file_id: &str) {
    #[cfg(target_os = "windows")]
    {
        let Some(sync_root) = crate::windows_cf::sync_root() else {
            return;
        };
        let Ok(Some(row)) = db.get_file(file_id) else {
            return;
        };
        let flagged = db.backups_roots().is_ok_and(|roots| roots.iter().any(|(id, _)| id == file_id));
        let Some((root, files)) = tree_on_disk(db, &sync_root, &row.path) else {
            return;
        };
        let spawned = std::thread::Builder::new()
            .name("backups-read-only".into())
            .spawn(move || {
                apply_read_only(&root, &files, flagged);
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "backup protection: the read-only state could not be applied; the next start does");
        }
    }
    #[cfg(not(target_os = "windows"))]
    let _ = (db, file_id);
}

/// LD-0113 Amendments 7 and 8a, Windows: the drive's read-only state of the Backups tree at `root_rel`
/// (a state DB path): `FILE_ATTRIBUTE_READONLY` on every file at or under it and the badge on the root
/// when `flagged`, both taken away when not (the role was cleared). Returns how many files changed.
#[cfg(target_os = "windows")]
pub(crate) fn apply_tree_read_only(db: &StateDb, sync_root: &Path, root_rel: &str, flagged: bool) -> usize {
    let Some((root, files)) = tree_on_disk(db, sync_root, root_rel) else {
        return 0;
    };
    apply_read_only(&root, &files, flagged)
}

/// The root folder of the tree at `root_rel` on disk, and where each row under it is on disk. Each path goes through
/// the one map every placeholder caller shares (`EngineBridge::placeholder_path_under`, task 2413), and then the link
/// walk (`stored_path::refuse_links_below`): a path through a link (a junction or symlink made inside the sync folder)
/// leads to another place, whose files are never made read-only or writable. `None` when the root's path does not map
/// or goes through a link (nothing is changed); a row whose path does is left out.
#[cfg(target_os = "windows")]
fn tree_on_disk(db: &StateDb, sync_root: &Path, root_rel: &str) -> Option<(PathBuf, Vec<PathBuf>)> {
    let root_rel = root_rel.trim_matches('/');
    let linked = std::cell::Cell::new(0usize);
    let on_disk = |rel: &str| {
        let path = crate::engine_bridge::EngineBridge::placeholder_path_under(sync_root, rel)?;
        if crate::stored_path::refuse_links_below(sync_root, &path).is_err() {
            linked.set(linked.get() + 1);
            return None;
        }
        Some(path)
    };
    let Some(root) = on_disk(root_rel) else {
        tracing::warn!("backup protection: the Backups root's stored path does not map onto the sync folder, or goes through a link; skipped");
        return None;
    };
    let rows = db.paths_at_or_under(root_rel).unwrap_or_else(|error| {
        tracing::warn!(%error, "backup protection: the Backups tree's rows could not be read");
        Vec::new()
    });
    let files = rows.iter().filter(|rel| rel.trim_matches('/') != root_rel).filter_map(|rel| on_disk(rel)).collect();
    if linked.get() > 0 {
        // Zero-knowledge: a count, never a path.
        tracing::warn!(skipped = linked.get(), "backup protection: rows whose path goes through a link were left alone");
    }
    Some((root, files))
}

/// [`apply_tree_read_only`] on paths already read. A folder, or a row with nothing on disk yet (the
/// placeholder pass mints it read-only), changes nothing. Zero-knowledge: counts only.
#[cfg(target_os = "windows")]
fn apply_read_only(root: &Path, files: &[PathBuf], flagged: bool) -> usize {
    let mut failed = 0usize;
    // One attribute read per row (`GetFileAttributesW`, nothing opened): only a file whose attribute is
    // not as wanted is opened and changed, so the pass at every start costs one call per file.
    const DIRECTORY: u32 = 0x10;
    const READONLY: u32 = 0x1;
    let changed = files
        .iter()
        .filter(|path| {
            crate::windows_cf::placeholders::read_os_placeholder_state(path)
                .is_some_and(|attributes| attributes & DIRECTORY == 0 && (attributes & READONLY != 0) != flagged)
        })
        .filter(|path| match crate::windows_cf::read_only::set_read_only(path, flagged) {
            Ok(changed) => changed,
            Err(error) => {
                failed += 1;
                tracing::debug!(%error, "backup protection: a file's read-only state could not be changed");
                false
            }
        })
        .count();
    show_read_only_badge(root, flagged);
    if changed > 0 || failed > 0 {
        tracing::info!(changed, failed, flagged, "backup protection: the Backups tree's read-only state was applied");
    }
    changed
}

/// LD-0113 Amendment 8a, Windows: core's `MSG_BADGE` on the Backups root folder at `path` (`on`), or
/// taken away, as a Cloud Files custom state. Only an Explorer registration made by this build or a
/// later one has the state's definition (an existing registration is kept as it is, LD-0105 S7); on an
/// older one Explorer shows nothing and this reports `false`. Zero-knowledge: never the path.
#[cfg(target_os = "windows")]
pub(crate) fn show_read_only_badge(path: &Path, on: bool) -> bool {
    match crate::windows_cf::read_only::set_read_only_badge(path, beebeeb_core::backup::MSG_BADGE, on) {
        Ok(()) => true,
        Err(error) => {
            tracing::debug!(%error, on, "backup protection: the Read-only badge could not be set");
            false
        }
    }
}

/// One placeholder pass at a time for what refusals put back, off the task that refused (the pass walks
/// every cloud-only row and calls Cloud Files per item). A request while a pass runs is covered by it,
/// or by the tick's refresh: passes never overlap, so parents are minted before children.
#[cfg(target_os = "windows")]
pub(crate) fn restore_placeholders_soon(db: std::sync::Arc<StateDb>, sync_root: &Path) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static RUNNING: AtomicBool = AtomicBool::new(false);
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let sync_root = sync_root.to_path_buf();
    let pass = move || {
        let restored = crate::windows_cf::restore_placeholders(&db, &sync_root);
        RUNNING.store(false, Ordering::SeqCst);
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
