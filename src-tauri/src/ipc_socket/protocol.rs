//! `protocol` items split out of `ipc_socket.rs`.

use super::*;

#[derive(Debug, Serialize, Deserialize)]
pub enum IpcRequest {
    GetFileStatus {
        file_id: String,
    },
    /// Task 1697: the replica enumerator's change feed. `since_anchor` is the
    /// opaque sync-anchor cursor from a previous `FileProviderChanges` reply
    /// (or absent at genesis). Paged: at most 100 changes per reply, with
    /// `next_anchor` doubling as the resume token while more remain.
    ListChanges {
        /// Opaque cursor bytes (decimal ASCII of the change-log rowid).
        /// Absent = enumerate from the beginning of retained history.
        #[serde(default)]
        since_anchor: Option<String>,
        /// Requested page size; clamped to `[1, 100]`.
        #[serde(default)]
        limit: Option<u32>,
    },
    /// Task 1697: the replica's `currentSyncAnchor` — served from the
    /// daemon's persistent cursor so it survives extension process death.
    GetSyncAnchor,
    /// Task 1697: publish the materialized container set (from the
    /// system's materialized-items callbacks) so the daemon can filter
    /// working-set signals to materialized parents.
    ReportMaterialized {
        container_ids: Vec<String>,
    },
    HydrateFile {
        file_id: String,
        dest_path: String,
        /// Opt in to `HydrateProgress` frames before the final reply (task
        /// 1670 issue 3). Absent/false for old extensions, which read exactly
        /// one reply and would mistake the first progress frame for it.
        #[serde(default)]
        progress: bool,
    },
    ListFileProviderItems {
        container_id: String,
    },
    QueueFinderCreate {
        parent_id: Option<String>,
        filename: String,
        kind: String,
        contents_path: Option<String>,
        content_type: Option<String>,
        /// Idempotency key (task 1684), stable across the system's retries of
        /// the same logical `createItem`. Absent from an old extension, which
        /// then gets exactly the pre-1684 behaviour. See `crate::ipc_write_dedup`
        /// and docs/IPC_PROTOCOL.md.
        #[serde(default)]
        request_id: Option<String>,
        /// The system recreates an item it could not delete because the person edited it
        /// (`NSFileProviderCreateItemDeletionConflicted`, spec 2026-10-09 §7.2). Absent from
        /// an older extension, which then gets an ordinary create.
        #[serde(default)]
        deletion_conflicted: bool,
        /// With `deletion_conflicted`: the template's identifier, the item the delete was for.
        #[serde(default)]
        template_identifier: Option<String>,
        /// With `deletion_conflicted`: the template's content version, the base of the edit.
        #[serde(default)]
        template_content_version: Option<String>,
    },
    QueueFinderModify {
        file_id: String,
        parent_id: Option<String>,
        filename: String,
        kind: String,
        contents_path: Option<String>,
        content_type: Option<String>,
        base_version_identifier: Option<String>,
        /// Idempotency key (task 1684); see `QueueFinderCreate::request_id`.
        #[serde(default)]
        request_id: Option<String>,
    },
    QueueFinderDelete {
        file_id: String,
        base_version_identifier: Option<String>,
    },
    /// Task 1699: fetch the server's encrypted thumbnail variant for one
    /// file, decrypt it, and stage the plaintext at `dest_path` (bounded to
    /// the allowed roots, written `.part` + rename). The extension reads the
    /// staging file back and deletes it, mirroring the hydrate flow.
    FetchThumbnail {
        file_id: String,
        dest_path: String,
        /// Requested maximum pixel dimension; picks the server variant
        /// (`thumbnail_variant`, mirroring the Windows Cloud Files picker).
        max_dimension: u32,
    },
    GetSyncSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpcResponse {
    FileStatus(FileProviderItemPayload),
    FileProviderItems {
        items: Vec<FileProviderItemPayload>,
    },
    /// Success with no payload. Deliberately an (empty) object on the wire —
    /// `{"Ok":{}}` — not the bare string `"Ok"` a unit variant serialises to:
    /// JSONSerialization on the Swift side rejects a top-level string unless
    /// fragments are allowed, which is what made every successful Finder
    /// hydrate surface as "daemon response was not valid JSON" (task 1670
    /// issue 3). The Swift reader still accepts the legacy bare string.
    Ok {},
    /// Sent only in answer to `HydrateFile { progress: true }`, zero or more
    /// times before the final reply. `total` is the plaintext size in bytes
    /// (0 when unknown); `done` is plaintext bytes decrypted so far.
    HydrateProgress {
        done: u64,
        total: u64,
    },
    Error {
        message: String,
    },
    WriteQueued {
        item: Option<FileProviderItemPayload>,
        ignored: bool,
        message: String,
    },
    /// A create or modify the app could not take NOW: nothing was staged or
    /// queued, and the person's file is unchanged on disk (spec
    /// 2026-10-09 §8.4: the app's staging folder cannot be created or
    /// written to). The extension reports it as a transient error
    /// (`uploadStagingFailed`, `serverUnreachable`), so the system keeps the
    /// change and retries the write; an `Error` would reach the system as
    /// `cannotSynchronize`, which it does not retry until the item changes
    /// again. `message` is a fixed reason, never a path.
    WriteRetryLater {
        message: String,
    },
    SyncSummary {
        syncing: u32,
        cloud_only: u32,
        conflicts: u32,
    },
    /// Task 1697: one page of the change log. `next_anchor` is present while
    /// more changes remain or after a delivered batch (the resume token /
    /// new anchor); `None` only when the log is empty.
    FileProviderChanges {
        changes: Vec<FileProviderChangePayload>,
        next_anchor: Option<String>,
    },
    /// Task 1697: `currentSyncAnchor` is served from the daemon's persistent
    /// cursor so it survives extension process death.
    FileProviderSyncAnchor {
        anchor: Option<String>,
    },
    /// Task 1699: `FetchThumbnail` succeeded; the decrypted thumbnail
    /// plaintext was staged atomically at the requested destination.
    /// `size_bytes` is the plaintext byte count (the extension reads the
    /// staging file back and deletes it).
    ThumbnailWritten {
        size_bytes: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileProviderItemPayload {
    pub identifier: String,
    pub parent_identifier: String,
    pub filename: String,
    pub kind: String,
    pub size_bytes: i64,
    pub content_type: Option<String>,
    pub status: String,
    pub capabilities: u32,
    pub version_identifier: Option<String>,
    /// Task 1697: server item creation time, seconds since the Unix epoch.
    /// `#[serde(default)]` keeps the payload readable by the 0.8.8 Swift
    /// decoder (absent -> no date).
    #[serde(default)]
    pub created_at: Option<i64>,
    /// Task 1697: content modification time, seconds since the Unix epoch —
    /// what Finder renders as "Modified".
    #[serde(default)]
    pub modified_at: Option<i64>,
    /// Task 1697: real child count for folders (files: absent). The Swift
    /// `childItemCount` was hardcoded to 0 before this field existed.
    #[serde(default)]
    pub child_item_count: Option<i64>,
    /// Task 1697: content-version component of `itemVersion` — the value whose
    /// change means "re-download + invalidate the thumbnail cache".
    #[serde(default)]
    pub content_version: Option<String>,
    /// Task 1697: metadata-version component — mtime/size/parent/name identity.
    /// Its change alone means "refresh metadata, keep the cached content".
    #[serde(default)]
    pub metadata_version: Option<String>,
    /// Task 1698: the row's effective pin state (own `pin_state`, else
    /// `inherited_pin_state`). Swift maps it to
    /// `.downloadEagerlyAndKeepDownloaded`; the root's `.downloadLazily`
    /// governs everything else. The cross-device pin backend is task 1683 —
    /// the pin set is this device's own today (deviation 1, task 1698).
    #[serde(default)]
    pub pinned: bool,
    /// L12: this item's id is foreign-class, so it was added by someone else.
    /// Decided from the id class, never a server field. Older Swift decoders
    /// ignore it (`serde(default)` on the way back in).
    #[serde(default)]
    pub added_by_someone_else: bool,
}

/// One change-log row crossing the IPC bridge (task 1697).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileProviderChangePayload {
    pub file_id: String,
    /// created | modified | deleted | reparented
    pub kind: String,
    #[serde(default)]
    pub old_parent_id: Option<String>,
    #[serde(default)]
    pub new_parent_id: Option<String>,
    /// The FULL item payload for created/modified/reparented rows (absent for
    /// deletions) so the replica's `didUpdateItems` needs no second
    /// round-trip. `#[serde(default)]` keeps the decoder tolerant.
    #[serde(default)]
    pub item: Option<FileProviderItemPayload>,
}

pub(super) const FP_ROOT: &str = "__fp_root__";

pub(super) const FP_ROOT_APPLE: &str = "NSFileProviderRootContainerItemIdentifier";

/// Task 1698: the SYSTEM trash container (macOS Trash / `supportsSyncingTrash`,
/// default YES — ruling `.claude/tasks/decisions/finder-trash-full-sync.md`).
/// Trashed items are presented as children of THIS container, outside the
/// single-root ruling (the trash view is system-managed, not a synthetic
/// folder under "Beebeeb").
pub(crate) const FP_TRASH_APPLE: &str = "NSFileProviderTrashContainerItemIdentifier";

// Task 1701: the OTHER namespace constants (`my_files`, `offline`,
// `conflicts`) are gone with the synthetic containers. This one survives as
// a WRITE-path guard only: a stale Finder client can still drop onto a
// cached `namespace:shared_with_me` folder, and that drop must be refused
// (read-only), not silently re-parented to the vault root by
// `normalize_parent_id`. It is never an enumeration surface any more.
pub(super) const NAMESPACE_SHARED_WITH_ME: &str = "namespace:shared_with_me";

pub(super) const CAP_READ: u32 = 1 << 0;

pub(crate) const CAP_WRITE: u32 = 1 << 1;

pub(super) const CAP_RENAME: u32 = 1 << 2;

pub(super) const CAP_DELETE: u32 = 1 << 3;

/// `.allowsAddingSubItems` on the Swift side (task 1694). Bit 4, with the
/// SAME numbering on both sides of the XPC bridge — the payload crosses it
/// as a plain u32 (`FileProviderItemPayload.capabilities` ↔
/// `BeebeebProviderItem.capabilities`, Swift: `BeebeebProviderItem.addSubItems`).
pub(super) const CAP_ADD_SUBITEMS: u32 = 1 << 4;

/// `.allowsReparenting` on the Swift side (task 1697). Bit 5, same numbering
/// both sides of the bridge — Finder refuses a drag-MOVE out of an item
/// without it.
pub(super) const CAP_REPARENT: u32 = 1 << 5;

/// `.allowsTrashing` on the Swift side (task 1697). Bit 6, same numbering both
/// sides. Trash SEMANTICS landed in task 1698 (ruling: full trash sync —
/// the trash-container reparent maps to the server trash op).
pub(super) const CAP_TRASH: u32 = 1 << 6;

/// Deliberately short — see [`macos_ipc_socket_path_in`]'s doc comment.
#[cfg(target_os = "macos")]
pub(super) const MACOS_IPC_SOCKET_FILENAME: &str = "ipc.sock";
