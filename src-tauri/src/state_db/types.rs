//! `types` items split out of `state_db.rs`.

use super::*;

/// Server completion is independent from native identity stamping. This journal
/// survives queue removal, restart and failed proof unlink.
#[derive(Debug, Clone, PartialEq)]
pub struct UploadFinalization {
    pub op_id: String,
    pub local_file_id: String,
    pub server_file_id: String,
    pub target_path: String,
    pub payload_path: String,
    pub stamped: bool,
    /// Task 2270 (spec R2): the reason label this row is held for review (`row_gone`, `moved`, `unreadable`,
    /// `other_file`, `partly_on_disk`, `platform_error`), or `None` while it is pending. Never a path or a name.
    pub review: Option<String>,
}

/// High-level sync status for a single file. Maps 1:1 to the icon
/// overlays rendered by the platform extensions.
#[derive(Debug, Clone, PartialEq)]
pub enum FileStatus {
    CloudOnly,
    Downloading,
    Local,
    Uploading,
    Conflict,
    Error,
    /// A locally-deleted file whose server-trash is still pending. The on-disk
    /// placeholder is already gone (the user deleted it); the row is kept ONLY
    /// so the queued `TrashFile` op stays coherent with it and so the Windows
    /// placeholder seeder (`populate_placeholders`, which only mints for
    /// `CloudOnly`) does NOT re-create the disk placeholder before the trash
    /// round-trips — the "deleted file comes back" bug (task 0802). The
    /// `TrashFile` op deletes this row on success; if the trash permanently
    /// fails the row stays `Trashing` (recoverable — the file still exists on
    /// the server) rather than reappearing on disk.
    Trashing,
}

impl FileStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            FileStatus::CloudOnly => "cloud_only",
            FileStatus::Downloading => "downloading",
            FileStatus::Local => "local",
            FileStatus::Uploading => "uploading",
            FileStatus::Conflict => "conflict",
            FileStatus::Error => "error",
            FileStatus::Trashing => "trashing",
        }
    }
    pub(super) fn from_str(s: &str) -> Self {
        match s {
            "cloud_only" => FileStatus::CloudOnly,
            "downloading" => FileStatus::Downloading,
            "local" => FileStatus::Local,
            "uploading" => FileStatus::Uploading,
            "conflict" => FileStatus::Conflict,
            "trashing" => FileStatus::Trashing,
            _ => FileStatus::Error,
        }
    }
}

pub const PERMISSION_READ: i64 = 1 << 0;

pub const PERMISSION_WRITE: i64 = 1 << 1;

pub const PERMISSION_SHARE: i64 = 1 << 2;

pub const PERMISSION_OWNER: i64 = 1 << 3;

#[derive(Debug, Clone, PartialEq)]
pub enum Namespace {
    MyFiles,
    SharedWithMe,
    Offline,
    Conflicts,
}

impl Namespace {
    pub(super) fn as_str(&self) -> &'static str {
        match self {
            Namespace::MyFiles => "my_files",
            Namespace::SharedWithMe => "shared_with_me",
            Namespace::Offline => "offline",
            Namespace::Conflicts => "conflicts",
        }
    }

    pub(super) fn from_str(s: &str) -> Self {
        match s {
            "shared_with_me" => Namespace::SharedWithMe,
            "offline" => Namespace::Offline,
            "conflicts" => Namespace::Conflicts,
            _ => Namespace::MyFiles,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ItemKind {
    File,
    Folder,
}

impl ItemKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ItemKind::File => "file",
            ItemKind::Folder => "folder",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "folder" => ItemKind::Folder,
            _ => ItemKind::File,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PinState {
    Inherit,
    Pinned,
    Unpinned,
}

impl PinState {
    pub(super) fn as_str(&self) -> &'static str {
        match self {
            PinState::Inherit => "inherit",
            PinState::Pinned => "pinned",
            PinState::Unpinned => "unpinned",
        }
    }

    pub(super) fn from_str(s: &str) -> Self {
        match s {
            "pinned" => PinState::Pinned,
            "unpinned" => PinState::Unpinned,
            _ => PinState::Inherit,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum OperationKind {
    HydrateFile,

    PinTree,
    UploadVersion,
    UploadFile,
    CreateFolder,
    RenameFile,
    MoveFile,
    TrashFile,
    RestoreFile,
    RestoreVersion,
}

impl OperationKind {
    pub(super) fn as_str(&self) -> &'static str {
        match self {
            OperationKind::HydrateFile => "hydrate_file",
            OperationKind::PinTree => "pin_tree",
            OperationKind::UploadVersion => "upload_version",
            OperationKind::UploadFile => "upload_file",
            OperationKind::CreateFolder => "create_folder",
            OperationKind::RenameFile => "rename_file",
            OperationKind::MoveFile => "move_file",
            OperationKind::TrashFile => "trash_file",
            OperationKind::RestoreFile => "restore_file",
            OperationKind::RestoreVersion => "restore_version",
        }
    }

    pub(super) fn from_str(s: &str) -> Self {
        match s {
            "hydrate_file" => OperationKind::HydrateFile,
            "pin_tree" => OperationKind::PinTree,
            "upload_version" => OperationKind::UploadVersion,
            "create_folder" => OperationKind::CreateFolder,
            "rename_file" => OperationKind::RenameFile,
            "move_file" => OperationKind::MoveFile,
            "trash_file" => OperationKind::TrashFile,
            "restore_file" => OperationKind::RestoreFile,
            "restore_version" => OperationKind::RestoreVersion,
            _ => OperationKind::UploadFile,
        }
    }
}

/// Task 1697: what changed about an item, for the File Provider change log.
/// `Reparented` exists because the materialized-set filter (Apple's contract,
/// `NSFileProviderReplicatedExtension.h`) tests "old OR new parent is
/// materialized" for a MOVE — a plain `Modified` row cannot carry the old
/// parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FpChangeKind {
    Created,
    Modified,
    Deleted,
    Reparented,
}

impl FpChangeKind {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            FpChangeKind::Created => "created",
            FpChangeKind::Modified => "modified",
            FpChangeKind::Deleted => "deleted",
            FpChangeKind::Reparented => "reparented",
        }
    }

    pub(super) fn from_str(s: &str) -> Option<Self> {
        match s {
            "created" => Some(FpChangeKind::Created),
            "modified" => Some(FpChangeKind::Modified),
            "deleted" => Some(FpChangeKind::Deleted),
            "reparented" => Some(FpChangeKind::Reparented),
            _ => None,
        }
    }
}

/// One change-log row returned by [`StateDb::list_file_changes`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    /// Opaque, strictly-ascending change-log cursor (also what `seq` stores).
    pub seq: i64,
    pub file_id: String,
    pub kind: FpChangeKind,
    /// Parent BEFORE the change (recorded at record time; survives the row).
    pub old_parent_id: Option<String>,
    /// Parent AFTER the change — read from the `files` row at record time.
    /// For a `Deleted` item this is the parent it was deleted FROM (the row is
    /// already gone by the time the consumer reads the log). The Swift
    /// enumerator maps this onto `parentItemIdentifier`.
    pub new_parent_id: Option<String>,
}

/// A row removed by [`StateDb::prune_absent`] (task 0806). Carries the minimum
/// needed to locate and delete the row's on-disk Cloud Files placeholder on
/// Windows: the `file_id` (for logging / dedupe), the server-relative `path`
/// (joined onto the sync root the same way `populate_placeholders` builds it),
/// and `is_dir` (a folder placeholder is removed with `remove_dir_all`, a file
/// with `remove_file`). Previously `prune_absent` returned only `Vec<String>`
/// (file_ids), which the snapshot reconcile path could LOG but not act on — so
/// the placeholder lingered in Explorer (the ghost-file bug).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrunedRow {
    pub file_id: String,
    pub path: String,
    pub is_dir: bool,
    /// Task 1640 (review N2): the file's agreed content when the row was read (a pruned row is
    /// gone from the table by the time its entry is removed).
    pub local_hash: Option<String>,
    /// Task 1640 (review N1): the hash of the one version the range fetches served, when it was one.
    /// Desktop 0.8.15: read only when that version is known (`fetched_min_version > 0`, as
    /// `fetched_content` reads it). A file of unknown content (`-1`, `-2`) can still carry a hash
    /// a fold left, which is no agreed content; with the start-up guard now running beside the
    /// engine, a remote deletion may reach such a file before the guard has marked it, and the
    /// platform's in-sync state is then not trusted for it.
    pub fetched_hash: Option<String>,
}

/// One row in the `files` table.
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub file_id: String,
    pub path: String,
    pub status: FileStatus,
    pub size_bytes: i64,
    /// Seconds since Unix epoch. Conflated meaning today: usually the
    /// server's `updated_at` from the last sweep, but Task 10's
    /// conflict detector overwrites this with `now` when it flips a
    /// file to `Conflict` so the auto-resolution deadline (Task 13)
    /// can read it as "detected_at." This conflation is acceptable
    /// for MVP — the value is only meaningful in the context of the
    /// row's current `status`.
    pub modified_at: i64,
    pub content_hash: Option<String>,
    /// Server's `updated_at` snapshot at the time we last considered
    /// this file fully synced (status transitioned to `Local`). Used
    /// by [`crate::engine_bridge::sync_tick`] as the "base version"
    /// in [`crate::conflict::is_conflict`]: a remote `updated_at` past
    /// this value means a sibling device has touched the file since
    /// our last sync.
    ///
    /// Defaults to `0` for rows that pre-date this column — those
    /// will look like "remote always changed" until the next
    /// successful sync re-anchors them, which is the safe direction
    /// (over-detect rather than miss a real conflict).
    pub remote_updated_at: i64,
    /// Server UUID of this row's parent folder, or `None` at the vault
    /// root. **Read-only on `FileEntry`**: it is populated from the
    /// `files.parent_id` column by the SELECT mappers below, but
    /// [`StateDb::upsert_file`] does NOT write it — the parent linkage
    /// (and `item_kind`) is owned by [`StateDb::set_file_contract_state`]
    /// via [`FileContractState`], which every metadata sweep calls right
    /// after `upsert_file`. Exposing it here lets
    /// [`crate::windows_cf::populate_placeholders`] order parents-before-
    /// children and place nested placeholders without an N+1 contract fetch.
    pub parent_id: Option<String>,
    /// Whether this row is a folder or a file. **Read-only on `FileEntry`**
    /// for the same reason as [`Self::parent_id`]: read from the
    /// `files.item_kind` column, written only via
    /// [`StateDb::set_file_contract_state`]. The Windows Cloud Files layer
    /// uses it to mint a DIRECTORY placeholder (vs a file placeholder) so
    /// folders are real, openable directories in Explorer.
    pub item_kind: ItemKind,
}

impl FileEntry {
    /// True if this row is a folder. Used by the Windows Cloud Files
    /// placeholder seeder to decide between a directory and a file
    /// placeholder.
    pub fn is_dir(&self) -> bool {
        self.item_kind == ItemKind::Folder
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileContractState {
    pub file_id: String,
    pub namespace: Namespace,
    pub parent_id: Option<String>,
    pub shared_root_id: Option<String>,
    pub share_id: Option<String>,
    pub owner_email: Option<String>,
    pub permission_bits: i64,
    pub item_kind: ItemKind,
    pub content_type: Option<String>,
    pub current_version: i64,
    pub current_object_version_id: Option<String>,
    pub local_base_version: i64,
    pub local_hash: Option<String>,
    pub cache_path: Option<String>,
    pub cache_bytes: i64,
    pub pin_state: PinState,
    pub inherited_pin_state: PinState,
    pub last_sync_at: i64,
}

impl FileContractState {
    pub fn effective_pin_state(&self) -> PinState {
        match self.pin_state {
            PinState::Inherit => self.inherited_pin_state.clone(),
            _ => self.pin_state.clone(),
        }
    }

    pub fn can_read(&self) -> bool {
        self.permission_bits & PERMISSION_READ != 0
    }

    pub fn can_write(&self) -> bool {
        self.permission_bits & PERMISSION_WRITE != 0
    }

    pub fn is_shared(&self) -> bool {
        self.namespace == Namespace::SharedWithMe
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RevokedSharedCache {
    pub file_id: String,
    pub cache_path: Option<String>,
}

/// Result of [`StateDb::purge_all_local_state`] (task 1538): every queued
/// operation and cached plaintext path that was cleared from the DB, for the
/// caller to delete from disk.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalStatePurge {
    /// Number of `operation_queue` rows deleted.
    pub queued_ops_purged: usize,
    /// Staged plaintext payload paths (`stage_finder_payload`) from the
    /// deleted `operation_queue` rows — the on-disk file at each path must
    /// still be removed by the caller.
    pub payload_paths: Vec<String>,
    /// Every `files.cache_path` that was cleared — the on-disk file at each
    /// path must still be removed by the caller.
    pub cache_paths: Vec<String>,
    /// `(file_id, server_relative_path)` for every row that was in `local`
    /// status (task 1538 Codex P1) — captured BEFORE the status flip, so a
    /// Windows Cloud Files placeholder (whose plaintext lives in the sync
    /// root, not at `cache_path`) can still be found and dehydrated/removed
    /// by the caller even though `cache_paths` above never pointed at it.
    /// Present regardless of platform (the query itself is cross-platform);
    /// only the Windows caller acts on it.
    pub local_placeholder_paths: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalActivityKind {
    MovedToTrash,
    Restored,
    /// Task 1640: a file deleted elsewhere held changes this device had not sent; they were kept
    /// as a copy beside it (`file_name` is the copy's).
    KeptLocalCopy,
}

impl LocalActivityKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            LocalActivityKind::MovedToTrash => "moved_to_trash",
            LocalActivityKind::Restored => "restored",
            LocalActivityKind::KeptLocalCopy => "kept_local_copy",
        }
    }

    pub(super) fn from_str(s: &str) -> Self {
        match s {
            "restored" => LocalActivityKind::Restored,
            "kept_local_copy" => LocalActivityKind::KeptLocalCopy,
            _ => LocalActivityKind::MovedToTrash,
        }
    }
}

/// Task 1640 (review F5): why a file deleted elsewhere was kept on this PC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeptReason {
    /// It held changes not sent: they are kept as a copy (`copy_path`).
    Copy,
    /// Only part of it was on this PC, out of sync: it stays where it is.
    PartlyLocal,
    /// It was open (in use), so it could not be removed yet; a later tick removes it.
    InUse,
    /// It held changes not sent, and they could not be copied: it stays where it is.
    CopyFailed,
}

impl KeptReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::PartlyLocal => "partly_local",
            Self::InUse => "in_use",
            Self::CopyFailed => "copy_failed",
        }
    }

    pub(super) fn from_str(value: &str) -> Self {
        match value {
            "partly_local" => Self::PartlyLocal,
            "in_use" => Self::InUse,
            "copy_failed" => Self::CopyFailed,
            _ => Self::Copy,
        }
    }
}

/// Task 1640 (review N9, N13): a kept copy to record, [`StateDb::record_kept_copy`].
#[derive(Debug, Clone)]
pub struct KeptCopyInput<'a> {
    pub original_file_id: &'a str,
    pub original_path: &'a str,
    pub copy_path: &'a str,
    pub copied_hash: &'a str,
    pub original_removed: bool,
    pub reason: KeptReason,
    pub kept_at: i64,
}

/// Task 1640 (review N9, N13): one kept copy (paths relative to the sync root, '/'-joined).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeptCopy {
    pub id: i64,
    pub original_file_id: String,
    pub original_path: String,
    pub copy_path: String,
    pub copied_hash: String,
    pub original_removed: bool,
    pub reason: KeptReason,
    pub kept_at: i64,
}

impl KeptCopy {
    /// Task 1640 (review F7): whether the person may dismiss its entry: nothing waits to be
    /// removed, or only part of the file was on this PC and the rest can no longer be fetched,
    /// so no tick will ever resolve it.
    pub fn dismissable(&self) -> bool {
        self.original_removed || self.reason == KeptReason::PartlyLocal
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalActivityEventInput {
    pub event_type: LocalActivityKind,
    pub file_id: Option<String>,
    pub file_name: String,
    pub rel_path: Option<String>,
    pub occurred_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LocalActivityEvent {
    pub id: i64,
    pub event_type: LocalActivityKind,
    pub file_id: Option<String>,
    pub file_name: String,
    pub rel_path: Option<String>,
    pub occurred_at: i64,
}

/// One finished transfer, for the popover's "Recent activity" (task 1683 slice 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferActivityInput {
    /// `"up"` (uploaded) or `"down"` (downloaded).
    pub direction: &'static str,
    pub file_id: Option<String>,
    pub file_name: String,
    pub rel_path: Option<String>,
    pub bytes: i64,
    pub occurred_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferActivity {
    pub id: i64,
    pub direction: String,
    pub file_id: Option<String>,
    pub file_name: String,
    pub rel_path: Option<String>,
    pub bytes: i64,
    pub occurred_at: i64,
}

/// What is waiting to transfer, read in one locked pass (task 1683 slice 2).
/// `upload_files` counts queued upload operations that are due to run (`next_retry_at` has
/// come, not paused, attempts left), so an upload sitting out a retry backoff is not a file
/// left; `download_*` counts rows currently `downloading`. The bytes are
/// the plaintext sizes of those files' rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TransferBacklog {
    pub upload_files: u32,
    pub upload_bytes: u64,
    pub download_files: u32,
    pub download_bytes: u64,
    /// Every queued operation (renames, trashes and uploads), paused or not.
    pub queued_ops: u32,
    /// Queued operations paused because the account is over its storage quota.
    pub paused_for_quota: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PendingOperation {
    pub op_id: String,
    pub kind: OperationKind,
    pub file_id: Option<String>,
    pub parent_id: Option<String>,
    pub target_path: Option<String>,
    pub metadata_json: Option<String>,
    pub payload_path: Option<String>,
    pub base_version: Option<i64>,
    pub base_object_version_id: Option<String>,
    pub attempts: i64,
    pub max_attempts: i64,
    pub next_retry_at: i64,
    pub last_error: Option<String>,
    /// Origin known-folder key (task 0811) when this op was enqueued by the
    /// known-folder backup mirror/upload (e.g. `"music"`); `None` for every
    /// normal user-initiated op. Disabling a folder's backup deletes exactly the
    /// queue rows carrying its key — see [`StateDb::purge_backup_source_ops`].
    pub backup_source_key: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Persisted resumable upload session for one queued upload op (flow 7).
/// See the `upload_resume` table in [`StateDb::open`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadResume {
    pub op_id: String,
    pub payload_path: String,
    pub payload_size: i64,
    pub payload_mtime_ns: i64,
    pub upload_session_id: String,
    pub server_file_id: String,
    pub object_version_id: String,
    pub chunk_size_bytes: i64,
    pub chunk_count: i64,
    /// Chunks `0..acked_chunks` were acknowledged (2xx) by the server.
    pub acked_chunks: i64,
    /// The post-init `PATCH /files/{id}` (encrypted name/parent) succeeded.
    pub metadata_applied: bool,
    /// The op created a NEW server file (no prior version): on abandonment
    /// its `is_uploading` row is an orphan the client may trash.
    pub is_create: bool,
    /// SF-0001 (L13): the session was opened under an id the desktop itself
    /// chose. Every session created by the new code sets this; a CREATE row
    /// written by an older version (server-chosen id) has `false` and must
    /// never be resumed.
    pub self_minted: bool,
    /// §8.6 rule 1: the version `complete` produced, recorded before any local
    /// bookkeeping. A row with it is never abandoned and lands without the network.
    pub completed_version: Option<i64>,
    /// The object version id that completion produced.
    pub completed_object_version_id: Option<String>,
    /// The server's `mime_type` in that completion: the landing's fallback content type.
    pub completed_mime_type: Option<String>,
}

/// Who queued a File Provider upload: this build's accept, or an earlier build (spec §10.2).
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOrigin {
    /// Minted by the round-4 accept transaction: its bytes are the ones its token names.
    Minted,
    /// Queued by an earlier build, given a write id at engine start (spec §10.2).
    EarlierBuild,
}

#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
impl WriteOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            WriteOrigin::Minted => "minted",
            WriteOrigin::EarlierBuild => "earlier_build",
        }
    }

    pub fn from_db(value: Option<&str>) -> Option<Self> {
        match value? {
            "minted" => Some(WriteOrigin::Minted),
            "earlier_build" => Some(WriteOrigin::EarlierBuild),
            _ => None,
        }
    }
}

/// The round-4 columns of one File Provider upload op. `PendingOperation` is
/// deliberately unchanged (52 literal construction sites); these columns are read
/// and written only by the dedicated round-4 functions.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinderWrite {
    pub write_id: String,
    pub origin: WriteOrigin,
    pub after_write_id: Option<String>,
    /// 0: the base is known. n >= 1: waiting for a snapshot, after n - 1
    /// successful snapshots that gave no base for the file (spec §6.3.3).
    pub base_pending: i64,
}

/// Everything the payload builder needs, read in one locked call (spec §5.3).
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
#[derive(Debug, Clone)]
pub struct ItemPresentation {
    pub contract: FileContractState,
    pub held: Option<crate::write_token::HeldWrite>,
    /// An op carries `held.write_id`, in any state, parked included.
    pub held_write_queued: bool,
    /// A Finder upload of this file exists that has not parked (spec §9.1).
    pub unparked_finder_upload: bool,
    /// A Finder upload of this file has parked (attempts used up, spec §9.1).
    pub parked_finder_upload: bool,
    pub version_filled: bool,
}

/// One queued operation as a sign-out reads it (spec 2026-10-10 §2, W2): every op but a hydrate or a
/// pin, with its file's path and whether its upload has a recorded completion.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
#[derive(Debug, Clone, PartialEq)]
pub struct WaitingRow {
    pub op_id: String,
    pub kind: OperationKind,
    pub file_id: Option<String>,
    pub file_path: Option<String>,
    pub payload_path: Option<String>,
    pub origin: Option<WriteOrigin>,
    /// The queue's order (`rowid`): higher was queued later.
    pub queue_order: i64,
    pub attempts: i64,
    pub max_attempts: i64,
    pub paused: bool,
    /// `upload_resume.completed_version` is set: the server has these bytes (§8 item 3).
    pub completed: bool,
}

/// Spec 2026-10-10 §5.2 step 8: what the purge guard read. `unkept`: waiting changes the keep step did not keep or
/// acknowledge (the purge refuses). `waiting_payloads`: every waiting write's staged copy, deleted last (W13).
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PurgeGuardRead {
    pub unkept: Vec<String>,
    pub waiting_payloads: Vec<String>,
}

/// Spec 2026-10-10 §5.2 step 6: what a fenced Finder enqueue did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinderEnqueue {
    Queued,
    /// A sign-out closed Finder accepts: nothing was written.
    AcceptsClosed,
}

/// A Finder folder create's row, written in the same transaction as its op, so a refused create leaves no row behind.
pub struct FinderFolderRow<'a> {
    pub entry: &'a FileEntry,
    pub parent_id: Option<&'a str>,
}

/// A successor's direct predecessor, as the claim reads it (spec §8.4).
pub(super) struct Predecessor {
    pub(super) op_id: String,
    pub(super) kind: String,
    pub(super) parent_id: Option<String>,
    pub(super) target_path: Option<String>,
    pub(super) metadata_json: Option<String>,
    pub(super) payload_path: Option<String>,
    pub(super) base_version: Option<i64>,
    pub(super) base_object_version_id: Option<String>,
    pub(super) after_write_id: Option<String>,
    pub(super) base_pending: i64,
    pub(super) attempts: i64,
    pub(super) max_attempts: i64,
    pub(super) origin: Option<WriteOrigin>,
    /// Its `complete` answered and the resume row recorded it (§8.6 rule 1).
    pub(super) completed: bool,
    /// Why it parked, as the park recorded it (`last_error_class`).
    pub(super) park_reason: Option<String>,
}

/// One File Provider content write, staged and described, for [`StateDb::accept_finder_write`].
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
#[derive(Debug)]
pub struct FinderAccept<'a> {
    pub op_id: &'a str,
    pub file_id: &'a str,
    pub kind: FinderAcceptKind<'a>,
    pub parent_id: Option<&'a str>,
    pub target_path: Option<&'a str>,
    pub metadata_json: &'a str,
    pub payload_path: &'a str,
    /// The staged copy's size and modification time (a modify's row takes them).
    pub size_bytes: i64,
    pub modified_at: i64,
    pub backup_source_key: Option<&'a str>,
    pub now: i64,
    /// Task 2190 ("Keep both", review-169 #3): a create that takes a parked write's place. In the
    /// same transaction, the parked op is checked (decidable, its file still at the version shown)
    /// and retired ([`retire_parked_conn`]); if it is not decidable, nothing is written (`Moved`).
    pub retire_parked: Option<RetireParked<'a>>,
}

/// Task 2190: the parked write a create retires, and the version the person was shown.
#[derive(Debug, Clone, Copy)]
pub struct RetireParked<'a> {
    pub op_id: &'a str,
    pub shown_version: i64,
}

#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
#[derive(Debug)]
pub enum FinderAcceptKind<'a> {
    /// A new file: `row` is inserted in the same transaction.
    Create { row: &'a FileEntry },
    /// New bytes for an existing file, on the base identifier the system sent.
    Modify { incoming_base: Option<&'a str> },
}

#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcceptOutcome {
    /// Queued. `decision` is `None` for a create.
    Queued {
        token: String,
        decision: Option<crate::write_token::BaseDecision>,
    },
    /// Queued and parked at once with its bytes (rule 6a′).
    ParkedAtOnce { token: String, reason: ParkReason },
    /// A modify of a file this database has no row for: nothing was written.
    UnknownItem,
    /// The staged copy's journal row is no longer an unfinished one: an engine-start sweep removed
    /// it with its copy between the copy and this transaction (spec §8.7 S6). Nothing was
    /// written; the write is retried.
    CopyGone,
    /// A sign-out closed Finder accepts after its engine stop (spec 2026-10-10 §15 question 3). Nothing was
    /// written; the write is retried, and macOS keeps the change until it is.
    AcceptsClosed,
    /// Task 2190: the parked write this create was to retire is no longer decidable. Nothing was
    /// written. `released` is never set here.
    ParkedMoved,
    /// Task 2190: queued, and the parked write retired in the same transaction; `released` is its
    /// staged copy, released in the journal, for the caller to unlink.
    QueuedRetiring { token: String, released: Option<String> },
    /// A modify of an item in the trash: its row is `Trashing`, or the system names the trash
    /// container as its parent (spec §7.2). Nothing was written; the save is refused.
    InTrash,
}

/// Why a File Provider upload parked with its bytes kept (spec §11, "Parked").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkReason {
    StaleBase,
    BaseUnknown,
    PayloadMissing,
    PredecessorParked,
    PredecessorLost,
    RekeyFailed,
    /// The server refused the write with 403: waiting will not cure it, so it parks at once
    /// and presents read-only (§8.4).
    PermissionDenied,
}

impl ParkReason {
    pub fn as_str(self) -> &'static str {
        match self {
            ParkReason::StaleBase => "stale_base",
            ParkReason::BaseUnknown => "base_unknown",
            ParkReason::PayloadMissing => "payload_missing",
            ParkReason::PredecessorParked => "predecessor_parked",
            ParkReason::PredecessorLost => "predecessor_lost",
            ParkReason::RekeyFailed => "rekey_failed",
            ParkReason::PermissionDenied => "permission_denied",
        }
    }
}

/// Everything one landing writes (spec §8.6.2, §8.7 S1.2), for [`StateDb::apply_landing`].
#[derive(Debug)]
pub struct LandingInput<'a> {
    pub op_id: &'a str,
    /// The runner's claim. `None` is Keep Mine's inline run, outside the queue.
    pub claim_id: Option<&'a str>,
    /// A File Provider write's id. Everything the landing does beyond round 3 is keyed on it.
    pub write_id: Option<&'a str>,
    pub local_file_id: &'a str,
    pub server_file_id: &'a str,
    pub target_path: Option<&'a str>,
    pub parent_id: Option<&'a str>,
    /// The base the landed upload was sent with.
    pub landed_base: Option<i64>,
    /// The version and object version id the server's completion produced (§8.6.1).
    pub produced_version: i64,
    pub produced_object_version_id: &'a str,
    pub size_bytes: i64,
    pub content_type: Option<&'a str>,
    /// The server's `mime_type`: the fallback for `content_type`.
    pub mime_type: Option<&'a str>,
    /// The op's staged payload, marked `completed = 1` in the release journal on every platform.
    pub completed_payload: Option<&'a str>,
    /// The SHA-256 of that payload's plaintext, for a sync-folder upload (no write id): what this
    /// device now agrees with the server on (task 1640). `None` leaves the file's baseline unknown.
    pub completed_payload_hash: Option<&'a str>,
    /// Windows: a create's finalization journal row, written in the landing's own transaction
    /// so the op is never gone without it. `None` everywhere else.
    pub finalization: Option<&'a UploadFinalization>,
    pub now: i64,
}

/// What the landing did to the ops queued after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LandingOutcome {
    /// Successors parked because they could not be re-keyed (§8.6 rule 3).
    pub parked_successors: Vec<(String, ParkReason)>,
    /// Successors the chain step gave the produced version.
    pub resolved_successors: usize,
}

/// What a snapshot node's version did to the row (spec §6.3.2, the fill and the raise).
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotVersion {
    Unchanged,
    /// The row was at 0: filled, `version_filled = 1`, and these ops got their base.
    Filled {
        resolved_ops: Vec<String>,
    },
    /// A newer version than the row's (I-3): the token clears through the predicate.
    Raised {
        old: i64,
        new: i64,
    },
    /// The node is mid-upload: the legacy init bumps the version first (FILES:2814-2830).
    SkippedUploading,
}

/// One op the runner may run now. Every later write for this attempt names `claim_id`
/// (spec §8.7 S2–S4).
#[derive(Debug, Clone)]
pub struct ClaimedOp {
    pub op: PendingOperation,
    pub claim_id: String,
    pub write: Option<FinderWrite>,
    /// The claim handed a parked predecessor's role to this op (spec §8.4).
    pub took_over: Option<TookOver>,
}

/// A parked predecessor whose role its direct successor took at the claim (spec §8.4).
/// Its op and resume row are gone and its staged payload is journalled for release;
/// the caller unlinks the payload after the commit (S6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TookOver {
    pub parked_op_id: String,
    pub released_payload: Option<String>,
}

#[derive(Debug, Clone)]
pub enum ClaimOutcome {
    /// The op is gone.
    Gone,
    /// Not an attempt: an earlier content op of the same file is queued and has not parked.
    Wait,
    /// Not an attempt: the hand-over committed, and the successor itself must still wait
    /// (its inherited base is pending, or its inherited predecessor is queued).
    WaitAfterHandOver(TookOver),
    /// Boxed: the op is large and the other outcomes carry nothing.
    Claimed(Box<ClaimedOp>),
    /// Not an attempt: the claim parked the op with its bytes (spec §8.4, S5). A
    /// hand-over committed in the same transaction rides along.
    Parked {
        op_id: String,
        file_id: Option<String>,
        reason: ParkReason,
        took_over: Option<TookOver>,
    },
}

/// What engine start repaired (spec §8.7 S3, S6; §10.2).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EngineStartRepair {
    pub claims_cleared: usize,
    /// Journalled payloads marked released that no op, resume row or Windows
    /// finalization references: the caller unlinks them.
    pub released_payloads: Vec<String>,
    /// A server-known file row is at version 0, so a snapshot was requested to learn
    /// its version (spec §6.3.2). Always `false` off macOS.
    pub resnapshot_requested: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum OperationPauseReason {
    Auth,
    Quota,
    Permission,
    Locked,
    /// Lead ruling, F9 review I-1 (spec 2026-10-06 R8, §5.6): paused for `auth` when the server said the vault key this
    /// Mac kept was no longer the account's. The names in these operations were encrypted under that key, so they are
    /// kept and never sent; no resume makes them due again.
    ///
    /// Built only by the macOS-only hold ([`StateDb::hold_operations_paused_for_auth_as_key_replaced`]); the `as_str`
    /// arm does not count as building it, so other platforms would report it never constructed.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    KeyReplaced,
    /// Task 2267 (fix round 2): a change the ownership proof could not tie to this account's own data (a new item at
    /// the top level, or in a folder that is not proven its own, such as a shared one), found when Beebeeb adopted the
    /// data on this PC. It is never sent on its own: the person uploads it or keeps it on this PC only.
    FoundWhilePaused,
    /// Tasks 1888 + 2226 (spec D4, D8): the reconcile pass could not fold a folder made on this PC into its server twin
    /// safely (two twins, an op acting on it, parents that disagree, a busy twin), or a re-armed upload would make old
    /// content current. It waits; the next pass looks again.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    NeedsReview,
}

impl OperationPauseReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            OperationPauseReason::Auth => "auth",
            OperationPauseReason::Quota => "quota",
            OperationPauseReason::Permission => "permission",
            OperationPauseReason::Locked => "locked",
            OperationPauseReason::KeyReplaced => "key_replaced",
            OperationPauseReason::FoundWhilePaused => "found_while_paused",
            OperationPauseReason::NeedsReview => crate::held_queue::NEEDS_REVIEW,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QueueDiagnostics {
    pub queued: i64,
    pub due: i64,
    pub paused: i64,
    pub by_kind: BTreeMap<String, i64>,
    pub paused_by_reason: BTreeMap<String, i64>,
    /// Last queue error with secrets, paths and known file/folder names removed
    /// (task 1685). A name the state DB does not know survives only if it is a
    /// standard error word or digits. See [`crate::diagnostic_redaction`].
    pub last_error: Option<String>,
    /// Closed-enum classification of `last_error`; cannot carry a name.
    pub last_error_code: Option<DiagnosticErrorCode>,
    /// Number of placeholders (`[path]`, `[name]`, `[redacted]`) written into
    /// `last_error`. `0` with a non-null `last_error` means nothing was removed.
    pub last_error_redactions: u32,
    pub last_error_class: Option<String>,
}

/// What the Windows tray and Settings › Sync say about the queue (0.8.15): counts and dates only, never a name, a path
/// or an error text.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct QueueHold {
    /// Not paused and not given up: due now, backing off, or being sent. On Windows and Linux, not one that waits for a
    /// held folder either (spec D7: those are `held`'s `waiting_for_folder`).
    pub waiting: i64,
    /// Not paused, but out of attempts: nothing retries them.
    pub gave_up: i64,
    /// Paused, one group per reason (`PAUSE_REASON_LABELS`, anything else as `other`), ordered by reason. On Windows
    /// and Linux it also holds the computed `waiting_for_folder` group (spec D7).
    pub held: Vec<HeldGroup>,
}

/// The operations paused for one reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HeldGroup {
    pub reason: String,
    pub count: i64,
    /// Unix seconds the oldest and newest of them were queued (0 = not recorded).
    pub oldest_at: i64,
    pub newest_at: i64,
}

/// Task 2267 (fix round 2): one `files` row as the ownership proof judges it.
#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofRow {
    pub file_id: String,
    pub parent_id: Option<String>,
    /// Its path under the sync root ('/'-separated). 128 of the 2,979 nested rows made on the maintainer's PC have no
    /// parent link, so a walk up falls back to the row at the path's folder.
    pub path: String,
    /// In the person's own files (`namespace = 'my_files'`), not a folder shared with them.
    pub my_files: bool,
    /// Judged against the server's tree: not made on this PC. A row made here has `current_version` and
    /// `remote_updated_at` both at 0 and a queued create names it (lead review of round 2, P2-1).
    pub server_named: bool,
}

/// Task 2267 (fix round 2): one queued operation, by what it acts on.
#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofOp {
    pub op_id: String,
    pub file_id: Option<String>,
    pub parent_id: Option<String>,
    /// `operation_queue.kind`, as stored.
    pub kind: String,
}

/// Task 2267 (lead review round 3, P2-2): one change held as `found_while_paused`, as the card counts it.
#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldChange {
    pub op_id: String,
    /// `operation_queue.kind`, as stored.
    pub kind: String,
    /// It creates its item (`CREATE_OP_SQL`).
    pub creates: bool,
    /// Its own row or its folder is in a folder shared with this account (`namespace != 'my_files'`).
    pub shared: bool,
}

/// One `files` row as the Selective sync tree reads it (0.8.15). A row the server never confirmed (a folder this PC
/// created whose `create_folder` never landed) has `current_version` 0 and `remote_updated_at` 0; a server row has one
/// of them set.
#[derive(Debug, Clone, PartialEq)]
pub struct TreeRow {
    pub file_id: String,
    pub path: String,
    pub status: FileStatus,
    pub size_bytes: i64,
    pub parent_id: Option<String>,
    pub item_kind: ItemKind,
    pub current_version: i64,
    pub remote_updated_at: i64,
}

/// A single bandwidth measurement point.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BandwidthSample {
    /// Unix timestamp (seconds) when the sample was recorded.
    pub sampled_at: i64,
    /// Upload bytes transferred during `period_secs`.
    pub up_bytes: u64,
    /// Download bytes transferred during `period_secs`.
    pub down_bytes: u64,
    /// Duration of the measurement window (seconds).
    pub period_secs: u32,
}
