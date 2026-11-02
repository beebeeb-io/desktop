//! `types` items split out of `engine_bridge.rs`.

use super::*;

/// Shared wire-byte counters threaded from `EngineBridge` to `runner`
/// to the heartbeat producer. Wrapped in `Arc` so it can be cloned
/// cheaply into the spawned heartbeat task.
pub struct WireCounters {
    /// Bytes written to the server in upload chunk loops (plaintext length
    /// of each chunk before encryption — the user's data rate, not wire overhead).
    pub upload_bytes: AtomicU64,
    /// Bytes received from the server in download chunk loops (raw wire bytes
    /// including the encryption envelope, which is all the client can measure
    /// without re-decrypting — close enough for throughput display).
    pub download_bytes: AtomicU64,
}

impl WireCounters {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            upload_bytes: AtomicU64::new(0),
            download_bytes: AtomicU64::new(0),
        })
    }

    /// Drain and return `(upload_bytes, download_bytes)` since the last drain.
    /// Both counters are atomically reset to 0. Called once per heartbeat beat.
    pub fn drain(&self) -> (u64, u64) {
        let up = self.upload_bytes.swap(0, Ordering::Relaxed);
        let dn = self.download_bytes.swap(0, Ordering::Relaxed);
        (up, dn)
    }
}

/// Events that drive a file through its sync lifecycle.
#[cfg(test)]
pub enum FileEvent {
    DownloadStart,
    DownloadComplete,
    UploadStart,
}

/// In-memory state tracker for a single file. The persisted state is
/// in [`StateDb`]; this struct exists so callers can validate the
/// _next_ state before writing it.
#[cfg(test)]
pub struct FileSM {
    pub(super) state: FileStatus,
}

#[cfg(test)]
impl FileSM {
    pub fn new(state: FileStatus) -> Self {
        Self { state }
    }
    pub fn state(&self) -> FileStatus {
        self.state.clone()
    }

    /// Apply `event`. Returns `Err` if the event isn't legal in the
    /// current state — callers should treat that as a programming bug,
    /// not a runtime error to recover from. The whitelist below is the
    /// canonical sync state machine; deny by default.
    pub fn transition(&mut self, event: FileEvent) -> Result<(), &'static str> {
        use FileEvent::*;
        use FileStatus::*;
        self.state = match (&self.state, event) {
            (CloudOnly, DownloadStart) => Downloading,
            (Downloading, DownloadComplete) => Local,
            (Local, UploadStart) => Uploading,
            _ => return Err("invalid state transition"),
        };
        Ok(())
    }
}

/// A File Provider write's outcome, with the token its reply names (spec §5) and, for a
/// write under a provisional id whose create has landed, the identifier the reply is
/// presented under: the system's own id for the item until it applies the swap (spec §7.2).
#[derive(Debug, Clone, PartialEq)]
pub struct FpWrite {
    pub outcome: FinderWriteOutcome,
    pub token: Option<String>,
    pub present_as: Option<String>,
}

impl FpWrite {
    pub fn plain(outcome: FinderWriteOutcome) -> Self {
        Self {
            outcome,
            token: None,
            present_as: None,
        }
    }
}

/// What [`loggable_id`] answers for an id that is not a UUID.
pub(crate) const NON_UUID: &str = "non_uuid";

/// One line per request that reached the server id through an alias (spec §7.2, §11). The
/// provisional id comes from the wire (M5); the server id is the row's.
pub(crate) fn log_alias_resolved(request: &'static str, provisional_id: &str, file_id: &str) {
    let (provisional_id, id_kind) = wire_id_fields(provisional_id);
    tracing::warn!(
        request,
        provisional_id,
        id_kind,
        file_id = %file_id,
        "provisional id resolved to the server id"
    );
}

#[cfg(test)]
impl FpWrite {
    /// The file id of a queued write.
    pub(super) fn outcome_file_id(&self) -> String {
        match &self.outcome {
            FinderWriteOutcome::Queued {
                file_id: Some(file_id), ..
            } => file_id.clone(),
            other => panic!("not a queued write with a file id: {other:?}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachePolicy {
    pub max_unpinned_cache_bytes: i64,
    pub disk_pressure_min_free_bytes: u64,
}

impl Default for CachePolicy {
    fn default() -> Self {
        Self {
            max_unpinned_cache_bytes: crate::config::LOCAL_CACHE_LIMIT_100_GB_BYTES,
            disk_pressure_min_free_bytes: 5 * 1024 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinUpdateOutcome {
    pub changed_item_ids: Vec<String>,
    pub hydrate_operations: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheCleanupOutcome {
    pub evicted_file_ids: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TransferLoopOutcome {
    pub completed_op_ids: Vec<String>,
    pub retried_op_ids: Vec<String>,
    pub paused_op_ids: Vec<String>,
    pub invalidated_item_ids: Vec<String>,
    /// Task 1700: errors from best-effort post-complete upload work
    /// (thumbnail generation/upload). These never fail the op — the upload
    /// itself is committed — but they used to vanish into a
    /// `tracing::warn!` that no test subscriber captures, so a red CI run
    /// said only "medium thumbnail upload with blurhash query" with no
    /// trail. Each entry is `"<op_id>: <error>"`.
    pub post_complete_errors: Vec<String>,
    /// Spec 2026-10-10 §6.4: an attempt failed at the link level (`offline`), the server refused the session
    /// (`session`), or, by the §15 question 4 ruling, the account refused a Finder write it can cure (`account`).
    pub link_failed: bool,
    pub session_refused: bool,
    pub account_refused: bool,
}

/// The two states a pin toggle can ask for. The Tauri command still takes a
/// flag; naming it here keeps `true` from having to stand for "pin".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PinToggle {
    Pin,
    Unpin,
}

impl PinToggle {
    pub(super) fn from_flag(pinned: bool) -> Self {
        if pinned {
            PinToggle::Pin
        } else {
            PinToggle::Unpin
        }
    }

    pub(super) fn is_pin(self) -> bool {
        matches!(self, PinToggle::Pin)
    }
}

/// Which upload init this is. A create sends no base; a File Provider write
/// that replaces a file is never sent without one; a plain replace keeps
/// today's base. The guard reads the kind, so `true, true` cannot mean two
/// different things at a call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InitKind {
    Create,
    FinderReplace,
    Replace,
}

/// Whether a kept copy's original left the disk in this pass, or stayed because
/// it was in use (a later tick removes it). The name keeps a bare `true` from
/// standing for "removed" at the prune and at the record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OriginalRemoval {
    Removed,
    InPlace,
}

impl OriginalRemoval {
    pub(super) fn is_removed(self) -> bool {
        matches!(self, OriginalRemoval::Removed)
    }
}

/// Whether a landed write still waits for its new thumbnails (spec §5.6).
/// Naming the two states keeps `false` from having to stand for "settled".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ThumbnailWait {
    Waiting,
    Settled,
}

impl ThumbnailWait {
    pub(super) fn is_waiting(self) -> bool {
        matches!(self, ThumbnailWait::Waiting)
    }
}

/// The due work of one pass, in due order. It lists again only after a folder create of the pass landed (D7), at most
/// `DUE_LISTINGS_PER_PASS` times in all, and an op listed earlier in the pass is not run again in it.
pub(super) struct DueOperations {
    pub(super) queue: VecDeque<PendingOperation>,
    pub(super) listed: HashSet<String>,
    pub(super) listings: usize,
    pub(super) folder_landed: bool,
}

impl DueOperations {
    pub(super) fn new(operations: Vec<PendingOperation>) -> Self {
        Self {
            listed: operations.iter().map(|op| op.op_id.clone()).collect(),
            queue: operations.into(),
            listings: 1,
            folder_landed: false,
        }
    }

    /// A folder create of this pass landed: the changes that waited for it may be due now.
    pub(super) fn folder_landed(&mut self) {
        self.folder_landed = true;
    }

    pub(super) fn next(&mut self, db: &StateDb, now: i64) -> Result<Option<PendingOperation>, EngineError> {
        while self.queue.is_empty() && self.folder_landed && self.listings < DUE_LISTINGS_PER_PASS {
            self.folder_landed = false;
            self.listings += 1;
            let listed = &mut self.listed;
            self.queue = db
                .list_due_operations(now)?
                .into_iter()
                .filter(|op| listed.insert(op.op_id.clone()))
                .collect();
        }
        Ok(self.queue.pop_front())
    }
}

/// One side (local or remote) of a conflict-resolution content preview.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ConflictContentSide {
    pub size_bytes: Option<u64>,
    /// UTF-8 text content — present only when the caller asked for a text
    /// preview AND the bytes are valid UTF-8 AND within
    /// [`CONFLICT_PREVIEW_TEXT_MAX_BYTES`]. `None` always means "see
    /// `unavailable_reason`", never a silent truncation.
    pub text: Option<String>,
    /// Human-readable reason `text` is absent (too large, not UTF-8, local
    /// read failed, remote download failed) — `None` when `text` is present
    /// or this is the (expected-textless) binary branch.
    pub unavailable_reason: Option<String>,
}

/// Task 2190 (plan §3.2): what the person chose for a parked conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParkedChoice {
    /// "Keep this Mac's".
    Local,
    /// "Keep the other".
    Remote,
    /// "Keep both".
    Both,
}

/// Task 2190: what a decision on a parked conflict did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ParkedResolution {
    /// The write is sent again, on the server's version `version`.
    Rebased { version: i64 },
    /// The Mac's bytes are kept at `kept_copy`, and the system reads the server's version.
    KeptOther { kept_copy: String },
    /// The Mac's bytes are queued as the new file `name`, beside the original.
    KeptBoth { file_id: String, name: String },
    /// The write moved since the review was read: nothing changed.
    Moved,
}

/// The longest device name a "Keep both" copy names, in UTF-8 bytes: a host name's label is at most 63
/// characters, so a Mac's own one-label name is never cut, and the cap keeps the suffix far inside the
/// 255-byte name limit (review-169 F3, plan §7.4).
pub(super) const KEEP_BOTH_DEVICE_MAX_BYTES: usize = 64;

/// Result of [`EngineBridge::conflict_content_preview`] — the real content
/// (or an honest reason it's unavailable) for both sides of a conflict.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConflictContentPreview {
    pub is_text: bool,
    pub local: ConflictContentSide,
    pub remote: ConflictContentSide,
    /// Task 2190 (review-169 #1): for a parked conflict, the file's version the person decides
    /// against; the decision is refused when the file is past it. `None` for a `Conflict` row.
    pub version: Option<i64>,
}

/// Cap on how large a text file's content preview may be. Deliberately
/// small: the whole file round-trips over Tauri's IPC as a JSON string and
/// is diffed synchronously in the webview, so this bounds both the IPC
/// payload and the diff algorithm's input size.
pub(super) const CONFLICT_PREVIEW_TEXT_MAX_BYTES: usize = 256 * 1024;

/// Task 1640 (review F1): what a fetch served: the version, and the SHA-256 of all of that
/// version's plaintext.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedVersion {
    pub version: i64,
    pub hash: String,
}

/// Task 1640: one fetch's plaintext ([`EngineBridge::hydrate_file_to_memory`]), what it served
/// (`None`: the download named no version), and its [`FetchTicket`], held until the transfer ended.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub struct FetchedBytes {
    pub bytes: Zeroizing<Vec<u8>>,
    pub version: Option<FetchedVersion>,
    pub ticket: FetchTicket,
}

/// Task 1640 (self-review #1): a file's fetches in flight, and how often one started or ended.
#[derive(Debug, Default)]
pub(super) struct FetchActivity {
    pub(super) in_flight: usize,
    pub(super) events: u64,
}

/// Task 1640 (self-review #1; design "The fetch tracker"): every file's fetches in flight, for the
/// whole process, not one bridge: a bridge made later in the same process (a new sign-in) sees what
/// an earlier one still has in flight. It is empty only when the process starts, and then no fetch
/// of an earlier process can transfer anything any more: only this process's callbacks write into
/// its placeholders.
pub(super) static FETCHES_IN_FLIGHT: std::sync::LazyLock<std::sync::Mutex<HashMap<String, FetchActivity>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

/// Task 1640 (self-review #1): one fetch of a file, from before it records anything until its
/// transfer ended (when it is dropped). It is "alone" when it was the file's only fetch in flight in
/// this process when it started, and no other fetch of the file has started or ended since.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
pub struct FetchTicket {
    pub(super) file_id: String,
    pub(super) alone_since: Option<u64>,
}

impl FetchTicket {
    /// Whether this fetch is alone, read from the tracker's locked map.
    #[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
    pub(super) fn alone_in(&self, fetches: &HashMap<String, FetchActivity>) -> bool {
        let Some(since) = self.alone_since else {
            return false;
        };
        fetches.get(&self.file_id).is_some_and(|activity| activity.events == since)
    }
}

impl Drop for FetchTicket {
    fn drop(&mut self) {
        {
            let mut fetches = FETCHES_IN_FLIGHT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(activity) = fetches.get_mut(&self.file_id) {
                activity.in_flight = activity.in_flight.saturating_sub(1);
                activity.events += 1;
                if activity.in_flight == 0 {
                    fetches.remove(&self.file_id);
                }
            }
        }
    }
}

/// Task 1640: what a tracked sync-folder file's bytes are (Windows: Linux runs no upload driver), against the content
/// this device last agreed with the server on: `local_hash` at `local_base_version`, written by a
/// hydrate into the sync folder and by a landing. The tick's detector is not this: it never sees
/// the device's bytes (task 2190).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SyncFolderContent {
    /// The bytes are the agreed content: they may be replaced, freed or marked in sync.
    Agreed,
    /// An upload of the file's content is queued (parked included): the bytes are not sent yet.
    Queued,
    /// The bytes differ from the agreed content and nothing is queued: a change that is not
    /// sent. `base` is the version they are based on, `None` when it is not known.
    Changed { base: Option<i64> },
    /// No agreed content is recorded (a file hydrated before task 1640).
    Unknown,
}

/// Task 2204 S0.4: a Windows rename is sent this long after it is queued, so a safe save
/// (rename away, new file, delete) completes first and folds back onto the original.
pub(crate) const RENAME_SETTLE_SECS: i64 = 10;

/// SF-0001 (L13): whether `id` is exactly the canonical lowercase hyphenated
/// form of a UUID — the form a client-chosen item id must have. A UUID in any
/// other spelling (uppercase, braces, simple form, surrounding whitespace) or a
/// non-UUID local id is not canonical and must not be sent as the new file id.
///
pub(super) fn is_canonical_item_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|parsed| parsed.hyphenated().to_string() == id)
}

pub(super) fn file_key_for(master_key: &[u8; 32], server_file_id: &str) -> beebeeb_core::kdf::FileKey {
    let master_key = beebeeb_core::kdf::MasterKey::from_bytes(*master_key);
    beebeeb_core::kdf::derive_file_key(&master_key, server_file_id.as_bytes())
}

/// Staged-payload modification time in nanoseconds (0 when unavailable). Part
/// of the resume fingerprint: a session is resumed only onto the same bytes.
pub(super) fn payload_mtime_ns(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// One warning per upload attempt the server refused with 409, with its class
/// (a stale base, another upload of the file in progress, or other), and a
/// distinct one when that attempt parks the op, worded by why: a stale base
/// parked it at once (`parked_at_once`, spec §8.4; `attempt` is the attempt that
/// was made), or its attempts are used up. Ids, counts and the class only: the
/// file's name, the request URL and the server's text never reach the log.
pub(super) fn log_refused_upload(
    op: &PendingOperation,
    attempt: i64,
    class: crate::api_client::InitConflictClass,
    parked_at_once: bool,
) {
    let file_id = op.file_id.as_deref().unwrap_or_default();
    if parked_at_once {
        tracing::warn!(
            op_id = %op.op_id,
            file_id,
            attempt,
            max_attempts = op.max_attempts,
            base_version = ?op.base_version,
            class = class.as_str(),
            reason = ParkReason::StaleBase.as_str(),
            "upload refused by the server (409 Conflict); its base is stale, parked with its bytes kept in the queue"
        );
    } else if attempt >= op.max_attempts {
        let reason = (class == crate::api_client::InitConflictClass::StaleBase).then(|| ParkReason::StaleBase.as_str());
        tracing::warn!(
            op_id = %op.op_id,
            file_id,
            attempt,
            max_attempts = op.max_attempts,
            base_version = ?op.base_version,
            class = class.as_str(),
            reason,
            "upload refused by the server (409 Conflict); attempts used up, parked with its bytes kept in the queue"
        );
    } else {
        tracing::warn!(
            op_id = %op.op_id,
            file_id,
            attempt,
            max_attempts = op.max_attempts,
            base_version = ?op.base_version,
            class = class.as_str(),
            "upload refused by the server (409 Conflict); will retry"
        );
    }
}

/// What the runner still does after an op succeeded.
#[derive(Debug)]
pub(crate) enum OpDone {
    /// Remove the op now (`finish_claimed`).
    Remove { release: Option<String> },
    /// The landing transaction already removed it; only unlink the released copy.
    Removed { release: Option<String> },
}

/// The `"operation"` tag a queued op's `metadata_json` carries. The stored JSON is a
/// format the queue already holds, so each variant spells the exact string the rows
/// carry; the enum only keeps that set from drifting apart across call sites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Op {
    CreateFolder,
    CreateFile,
    MetadataUpdate,
    UploadVersion,
    Trash,
    RestoreVersion,
    PinTree,
    HydrateFile,
}

impl Op {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Op::CreateFolder => "create_folder",
            Op::CreateFile => "create_file",
            Op::MetadataUpdate => "metadata_update",
            Op::UploadVersion => "upload_version",
            Op::Trash => "trash",
            Op::RestoreVersion => "restore_version",
            Op::PinTree => "pin_tree",
            Op::HydrateFile => "hydrate_file",
        }
    }
}

/// The object version the server answered with when its `complete` succeeded: the
/// version number the row lands on, and the object version id its content now has.
/// Two values the server answered with, nothing else; the landing reads both. The
/// number is not an id: it counts a file's versions, `VersionId` names one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProducedObject<'a> {
    pub(crate) version_number: i64,
    pub(crate) object_version_id: VersionId<'a>,
}

/// Everything one landing needs: the op and its claim, the server row the upload
/// produced, and the completion's own size and types. Borrows the caller's ids,
/// op and error list; nothing is cloned but the object the server answered with.
pub(crate) struct LandRequest<'a> {
    pub(crate) local_file_id: LocalFileId<'a>,
    pub(crate) op: &'a PendingOperation,
    pub(crate) claim: Option<&'a ClaimedOp>,
    pub(crate) server_file_id: ServerFileId<'a>,
    pub(crate) produced: ProducedObject<'a>,
    pub(crate) size_bytes: u64,
    pub(crate) content_type: Option<String>,
    pub(crate) mime_type: Option<String>,
    pub(crate) post_complete_errors: &'a mut Vec<String>,
    pub(crate) sync_root: &'a Path,
}

/// One line per retried landing of an upload the server completed (spec §8.6 rule 6):
/// ids and the attempt only (spec §11).
pub(super) fn log_completed_landing_retried(op_id: &str, file_id: Option<&str>, attempt: i64) {
    tracing::warn!(
        op_id = %op_id,
        file_id = file_id.unwrap_or_default(),
        attempt,
        "upload completed on the server; local landing will be retried"
    );
}

/// One line for an attempt whose op moved since its claim: the op id and the
/// step only (spec §11).
pub(super) fn log_queue_state_moved(op_id: &str, step: &'static str) {
    tracing::warn!(op_id = %op_id, step, "queue state moved");
}

/// One line when a queued write takes over a parked predecessor at its claim
/// (spec §8.4): ids only.
pub(super) fn log_took_over(op_id: &str, file_id: Option<&str>, parked_op_id: &str) {
    tracing::warn!(
        op_id = %op_id,
        file_id = file_id.unwrap_or_default(),
        parked_op_id = %parked_op_id,
        "queued write took over a parked one"
    );
}

/// One line when an upload parks: ids and the reason only (spec §11).
pub(super) fn log_parked(op_id: &str, file_id: Option<&str>, reason: ParkReason) {
    tracing::warn!(
        op_id = %op_id,
        file_id = file_id.unwrap_or_default(),
        reason = reason.as_str(),
        "upload parked with its bytes kept in the queue"
    );
}

/// Spec §11: a File Provider write whose attempts a failure waiting does not cure used up parks by
/// the queue's rule. A 409 says so on its refused-upload line; any other failure logs the park line
/// once, with `reason="attempts_used_up"`, a log reason and not a [`ParkReason`]. `file_id` by M5.
pub(super) fn log_parked_attempts_used_up(op_id: &str, file_id: Option<&str>) {
    let (file_id, id_kind) = wire_id_fields(file_id.unwrap_or_default());
    tracing::warn!(
        op_id = %op_id,
        file_id,
        id_kind,
        reason = "attempts_used_up",
        "upload parked with its bytes kept in the queue"
    );
}

pub(super) fn is_create_file_operation(metadata: &serde_json::Value) -> bool {
    metadata["operation"].as_str() == Some("create_file")
}

pub(super) fn recorded_content_format(key: &beebeeb_core::kdf::FileKey, meta: &serde_json::Value)
    -> Result<beebeeb_core::chunk_stream::ExpectedChunkFormat, EngineError> {
    let name = meta["name_encrypted"].as_str().ok_or_else(|| anyhow::anyhow!("missing authenticated file metadata"))?;
    Ok(beebeeb_core::encrypt::decrypt_file_metadata_with_key(key, name)?.chunk_format)
}

/// Restores a row's status if [`EngineBridge::hydrate_file_with_progress`] is
/// dropped mid-download (the IPC client hung up and the future was cancelled).
/// Without it a cancelled hydrate leaves the row on `Downloading` forever,
/// which the overlay renders as a permanent spinner. Restores the status the
/// row had before hydration started; a stale `Downloading` (or a missing row)
/// falls back to `CloudOnly`, the state hydration is meant to leave.
pub(super) struct DownloadingStatusGuard<'a> {
    pub(super) db: &'a StateDb,
    pub(super) file_id: &'a str,
    pub(super) restore: FileStatus,
    pub(super) armed: bool,
}

impl<'a> DownloadingStatusGuard<'a> {
    pub(super) fn arm(db: &'a StateDb, file_id: &'a str) -> Self {
        let restore = match db.get_file(file_id).ok().flatten().map(|entry| entry.status) {
            Some(FileStatus::Downloading) | None => FileStatus::CloudOnly,
            Some(previous) => previous,
        };
        Self {
            db,
            file_id,
            restore,
            armed: true,
        }
    }

    pub(super) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for DownloadingStatusGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            // Only over the hydrate's own `Downloading` (spec §9.2).
            let _ = self
                .db
                .set_status_if(self.file_id, FileStatus::Downloading, self.restore.clone());
        }
    }
}

pub(super) fn operation_version_id(op: &PendingOperation) -> Option<String> {
    op.base_object_version_id.clone().or_else(|| {
        op.metadata_json
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .and_then(|value| value.get("version_id").and_then(|v| v.as_str()).map(str::to_string))
    })
}

pub(super) fn json_timestamp_secs(value: &serde_json::Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| {
        let field = value.get(*key)?;
        field.as_i64().or_else(|| field.as_str().and_then(parse_rfc3339_secs))
    })
}

/// Whether `sealed` (a `name_encrypted` envelope) is this account's name for `file_id`. Core's file metadata reader
/// (`decrypt_file_metadata`) opens it under the file key derived from `master_key` and `file_id`, in every format the
/// engine reads: the string id (as every client seals today) and the 16-byte binary id (the old CLI); the name subkey
/// (`beebeeb/name/v1`, task 1908) and the legacy raw file key; both envelopes. It refuses a plaintext name, which proves
/// nothing. The key is derived from this id, so a name sealed for another file, or under another account's key, never
/// opens. Task 2267's proof (`own_data_proof::probe`) and task 1888's key check (spec D12) read names this way.
pub fn name_opens(master_key: &[u8; 32], file_id: &str, sealed: &str) -> bool {
    let master = beebeeb_core::kdf::MasterKey::from_bytes(*master_key);
    beebeeb_core::encrypt::decrypt_file_metadata(&master, file_id, sealed)
        .is_ok_and(|metadata| !metadata.name.trim().is_empty())
}

pub(super) fn string_field(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| value.get(*key).and_then(|field| field.as_str()))
        .map(str::trim)
        .find(|field| !field.is_empty())
        .map(str::to_string)
}

pub(super) fn parse_rfc3339_secs(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.timestamp())
}

pub(super) fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// One file the latest tick noticed has diverged on both sides.
/// Returned from [`sync_tick`] so [`crate::runner`] can fan it out
/// to UI: open a conflict window + fire a notification.
#[derive(Debug, Clone)]
pub struct ConflictDetected {
    pub file_id: String,
    pub file_name: String,
    pub is_text: bool,
}

/// Pull the user's file list from the API, refresh the state DB, and
/// flag any newly-divergent files. Called from [`crate::runner`]'s
/// tick loop.
///
/// Three-way decision per remote file:
///
/// 1. **New to us** — no row exists. Insert as `cloud_only`. The OS
///    extension surfaces it as a placeholder; the user gets it on
///    demand.
///
/// 2. **Known and locally `Local`, remote moved** — the row exists,
///    its status is `Local`, and the server's `updated_at` is past
///    `remote_updated_at`. The row is re-anchored to the server's
///    version (task 2190, plan §7.2). A `Local` row has no pending write;
///    an edit not yet queued keeps its base (1640's agreed content), is
///    queued on it and parks as stale when the server moved: the parked
///    op is the conflict (E1). The pull reads no file.
///
/// 3. **Known and not in `Local`** — pending download/upload, etc.
///    Leave alone; the upload/download path owns those transitions.
/// Task 1697 review fix (T3): what one sync tick did. The remote ingestion
/// path (delta ops + snapshot bootstrap) records every item id it APPLIED so
/// the runner can signal the File Provider working set about REMOTE changes
/// — previously only local ops (`operations_applied`) signaled, so Finder
/// stayed blind to server-side creates/modifies/moves/deletes until an
/// unrelated signal.
#[derive(Debug, Default)]
pub struct SyncTickOutcome {
    pub conflicts: Vec<ConflictDetected>,
    pub applied_item_ids: Vec<String>,
    /// Spec D11 (task 1888, review P1): the pull came back from the server: a snapshot, or the `/sync/ops` delta. A
    /// tick whose `GET /sync/ops` failed (offline, a 5xx, a 401) still returns `Ok`, so `Ok` alone proves neither that
    /// the server accepts this session's token nor that the rows the reconcile pass plans against are current; the
    /// pass and the resume wait for a tick where this is true.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub reached_server: bool,
}

/// Drop duplicate ids from one tick's applied batch (an item can be applied
/// by two ops in one delta — the signal cares that it changed, not how often).
pub(super) fn dedupe(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids.dedup();
    ids
}

/// Windows: remove the on-disk Cloud Files placeholder for each row the deletion
/// reconcile just dropped from the DB (task 0806). The DB row is ALREADY gone (the
/// caller deleted it before calling this), so the watcher's `handle_delete` finds
/// no row and queues no server trash; we ALSO register each path in the watcher's
/// engine-delete suppression set so the `NOTIFY_DELETE_COMPLETION` our own remove
/// fires is dropped deterministically rather than echoing into a redundant trash.
///
/// `rows` MUST be ordered children-before-parents (both `prune_absent` and
/// `delete_file_subtree` return that order) so a directory's leaf placeholders are
/// removed before the directory placeholder itself.
///
/// A row whose path can't be safely resolved under the root (empty/traversal) is
/// skipped. Per-row failures are logged (file_id only — zero-knowledge) and never
/// abort the sweep. No-op on macOS/Linux.
/// Task 1640: what a remotely deleted file's local entry holds, as the platform reports it.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocalEntry {
    /// A placeholder in sync with no modified data: the agreed bytes, or none.
    InSync,
    /// A placeholder with no data on disk: nothing local to keep.
    NothingLocal,
    /// Every byte is on disk: a hydrated placeholder out of sync, or a plain file (what an atomic
    /// save leaves).
    WhollyLocal,
    /// Some bytes are on disk and it is out of sync: reading it would fetch the rest.
    PartlyLocal,
    /// The platform's view could not be read (self-review #5: the file is open exclusively, or the
    /// query failed): treated as in use, kept and retried, never as partly on disk.
    Unreadable,
}

/// Task 1640 (review P1-1): what removing one remotely deleted sync-folder entry did.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PrunedEntry {
    /// Removed: agreed bytes, no local bytes, or an empty folder.
    Removed,
    /// Nothing was there.
    Gone,
    /// The file held bytes the server does not have: they were copied to `copy` (whose SHA-256
    /// is `copied_hash`), where the copy can upload, then the original was removed, or not
    /// (`removal`: it was in use; a later tick removes it).
    KeptCopy { copy: PathBuf, copied_hash: String, removal: OriginalRemoval },
    /// A file left as it is, and why (review F5): partly on disk (reading it would fetch the
    /// rest), in use (it could not be removed or read), or holding changes whose copy failed.
    KeptInPlace(KeptReason),
    /// A folder that still holds something (review F3).
    FolderKept,
}

/// Task 1640 (review-151d P3-1): the engine's own remove of a sync-folder entry. It is registered
/// with the watcher right before it happens, so its delete notification queues no trash, and the
/// registration is dropped again when it did not happen (a later delete by the person counts).
/// What the path being removed is, so the remove picks the matching syscall.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RemovedKind {
    File,
    Dir,
}

#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
pub(super) fn engine_remove(path: &Path, kind: RemovedKind) -> std::io::Result<()> {
    crate::watcher::suppress_engine_delete(path);
    let removed = match kind {
        RemovedKind::Dir => std::fs::remove_dir(path),
        RemovedKind::File => std::fs::remove_file(path),
    };
    if removed.is_err() {
        crate::watcher::cancel_engine_delete_suppression(path);
    }
    removed
}

/// Where a row fed to [`process_metadata_row`] came from. Decides whether the
/// `updated_at` short-circuit for an already-`Local` row may fire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RowSource {
    /// A `/sync/snapshot` node. The whole tree is re-ingested, so an unchanged
    /// row must be skipped; `updated_at` is the server's own timestamp.
    Snapshot,
    /// A row synthesised from ONE `/sync/ops` delta. Every op is a real remote
    /// change delivered exactly once (the cursor advances past it), and its
    /// `updated_at` is only the local wall-clock second `synthesize_op_row`
    /// stamped — two ops in one tick share it. Gating on it dropped the
    /// `file_update` that follows a same-second `file_rename` (flow-7 P0), so
    /// op rows are always applied. Re-applying an op after a crash before the
    /// cursor persisted is idempotent: ops replay in order to the same state.
    Op,
}
