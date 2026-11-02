//! Wire DTOs and pure classifiers: sync deltas, upload init/refusal, metadata
//! updates, search shards, and the account/device/telemetry bodies.
//!
//! Owns: serde shapes only — field and variant names are the server contract
//! and stay verbatim. Must not: perform requests or read client state.
//! Tests: `api_client/tests/` (serialization and upload areas).

use super::*;

#[derive(Debug, Clone, Copy, Default)]
pub struct ListFilesScope<'a> {
    pub parent_id: Option<FolderId<'a>>,
    pub shared_folder_id: Option<SharedFolderId<'a>>,
    pub trashed: Option<bool>,
    pub cursor: Option<&'a str>,
}

/// `GET /api/v1/sync/snapshot` response. `nodes` are raw rows (same shape the
/// ingest path already reads); `seq_id` is the latest op cursor (0 if the user
/// has no ops yet — a brand-new vault).
#[derive(Debug, Clone, Deserialize)]
// ── /sync delta engine DTOs (task 0789) ────────────────────────────────────────
//
// Mirror the server contract in `repos/server/.../routes/sync.rs` EXACTLY:
//   * `GET /api/v1/sync/snapshot` → `{ seq_id, nodes: [ … ] }` — the entire
//     non-trashed tree in ONE call, plus the latest op cursor.
//   * `GET /api/v1/sync/ops?since={n}` → `{ ops: [ … ], since }` — every op with
//     `seq_id > since`, oldest-first.
//
// The desktop replaces the per-folder full-tree `/files` re-walk (which fired
// `1 + folders` requests/tick and never pruned server deletions) with: ONE
// snapshot to bootstrap, then ONE ops?since per tick. Both ride
// `send_with_retry`, so a transient 429 backs off instead of erroring.
//
// We keep each node's metadata as a raw `serde_json::Value` so the existing
// `apply_metadata_file_row` / `resolve_relative_path` ingest path consumes a
// snapshot node BYTE-FOR-BYTE the same way it consumed a `/files` row — the
// node already carries `id`, `parent_id`, `name_encrypted`, `size_bytes`,
// `is_folder`, `content_hash`, `version_number`, `is_trashed`, `is_starred`,
// `updated_at` (confirmed against `sync.rs::snapshot`). `seq_id` is the cursor.

pub struct SyncSnapshot {
    pub seq_id: i64,
    #[serde(default)]
    pub nodes: Vec<serde_json::Value>,
}

/// One op from `GET /api/v1/sync/ops`. `op_type` is the server's discriminator
/// (`file_create` / `file_update` / `folder_create` / `file_rename` /
/// `folder_rename` / `file_move` / `folder_move` / `file_trash` /
/// `file_restore` / `file_delete`); `payload` is the op-specific body. `seq_id`
/// advances the cursor.
#[derive(Debug, Clone, Deserialize)]
pub struct SyncOp {
    pub seq_id: i64,
    pub op_type: String,
    #[serde(default)]
    pub payload: serde_json::Value,
}

/// `GET /api/v1/sync/ops?since={n}` response.
#[derive(Debug, Clone, Deserialize)]
pub struct SyncOps {
    #[serde(default)]
    pub ops: Vec<SyncOp>,
    /// Echo of the `since` the server applied (it clamps a missing value to 0).
    #[serde(default)]
    pub since: i64,
    /// Authoritative per-user log head; absent on servers predating task 1909.
    #[serde(default)]
    pub head_seq: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DesktopUploadInitRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_id: Option<String>,
    pub file_name: String,
    pub file_size_bytes: u64,
    pub mime_type: Option<String>,
    pub parent_id: Option<String>,
    pub profile: String,
    pub is_media: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_version_number: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DesktopUploadInitResponse {
    pub file_id: String,
    pub tenant_id: String,
    pub object_version_id: String,
    pub upload_session_id: String,
    pub chunk_size_bytes: i64,
    pub chunk_count: i64,
    pub storage_format_version: i64,
    pub storage_pool_id: String,
    pub region: String,
}

/// The server's 409 messages at `init` (spec §8.4); a test pins both.
pub const STALE_BASE_MESSAGE: &str = "stale base version for replacement upload";

pub const IN_PROGRESS_MESSAGE: &str = "upload is already in progress for this file";

/// Why `init` answered 409 (spec §8.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitConflictClass {
    /// The base is not the file's current version; it never becomes valid again.
    StaleBase,
    /// Another upload of the file is in progress.
    InProgress,
    /// Anything else, a changed message included: retried, as before.
    Other,
}

impl InitConflictClass {
    pub fn as_str(self) -> &'static str {
        match self {
            InitConflictClass::StaleBase => "stale_base",
            InitConflictClass::InProgress => "in_progress",
            InitConflictClass::Other => "other",
        }
    }
}

/// A changed server message falls back to `Other` (retry), today's behaviour.
pub fn classify_init_conflict(message: &str) -> InitConflictClass {
    match message {
        STALE_BASE_MESSAGE => InitConflictClass::StaleBase,
        IN_PROGRESS_MESSAGE => InitConflictClass::InProgress,
        _ => InitConflictClass::Other,
    }
}

/// `init` answered 409. Carries the class only: the server's text never reaches a log.
#[derive(Debug)]
pub struct InitConflict {
    pub class: InitConflictClass,
    /// The server's code, when it is one the client acts on (the upload gate's 409s).
    pub refusal: Option<UploadRefusalCode>,
}

/// The server's code on a refused upload request (`{"error": <code>, ...}`), for the codes the
/// client acts on: each is a refusal the person cures (free space, subscribe, pay), spec §8.4.
/// Read from the `error` field by exact match. Nothing else of the body is kept, and the body's
/// text never reaches a log or a stored error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadRefusalCode {
    /// 413: the account is over its storage quota.
    QuotaExceeded,
    /// 403: the account has reached its stored-object limit.
    ObjectBudgetExceeded,
    /// 403: the account is read-only for an unpaid invoice.
    BillingReadOnly,
    /// 409 (the upload gate): the trial ended and the account is read-only.
    AccountLapsed,
    /// 409 (the upload gate): the account has no plan.
    PlanRequired,
    /// 409 (the upload gate): a never-paid trial was cancelled, so uploads are paused.
    TrialCancelledReadOnly,
}

impl UploadRefusalCode {
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "quota_exceeded" => Some(UploadRefusalCode::QuotaExceeded),
            "object_budget_exceeded" => Some(UploadRefusalCode::ObjectBudgetExceeded),
            "billing_read_only" => Some(UploadRefusalCode::BillingReadOnly),
            "account_lapsed" => Some(UploadRefusalCode::AccountLapsed),
            "plan_required" => Some(UploadRefusalCode::PlanRequired),
            "trial_cancelled_read_only" => Some(UploadRefusalCode::TrialCancelledReadOnly),
            _ => None,
        }
    }
}

/// An upload request (init, a chunk, complete) the server answered with a non-2xx status, other
/// than init's 409 ([`InitConflict`]). It shows, and has as its source, the `reqwest` error
/// `error_for_status` gives (the status and the URL, never the body), so every reading of the
/// error's text and status stays as it was. It adds the server's code when the client acts on it.
#[derive(Debug)]
pub struct UploadRefused {
    pub code: Option<UploadRefusalCode>,
    pub(super) status_error: reqwest::Error,
}

impl UploadRefused {
    pub fn status(&self) -> Option<u16> {
        self.status_error.status().map(|status| status.as_u16())
    }
}

impl std::fmt::Display for UploadRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.status_error.fmt(f)
    }
}

impl std::error::Error for UploadRefused {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.status_error)
    }
}

/// `resp` when it is a 2xx; else the [`UploadRefused`] with the server's code read from the body's
/// `error` field.
pub(super) async fn upload_response(resp: reqwest::Response) -> ApiResult<reqwest::Response> {
    let status_error = resp.error_for_status_ref().err();
    let Some(status_error) = status_error else {
        return Ok(resp);
    };
    let body: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
    let code = body
        .get("error")
        .and_then(|value| value.as_str())
        .and_then(UploadRefusalCode::from_code);
    Err(DesktopApiError::UploadRefused(UploadRefused { code, status_error }))
}

impl std::fmt::Display for InitConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "upload init refused with 409 Conflict ({})", self.class.as_str())
    }
}

impl std::error::Error for InitConflict {}

#[derive(Debug, Clone, Deserialize)]
pub struct UploadChunkResponse {
    pub index: u32,
    pub size: i64,
    pub skipped: Option<bool>,
}

/// The parent a metadata update (`PATCH /api/v1/files/{id}`) sends. The server reads an
/// omitted `parent_id` as "leave the parent alone", an explicit `null` as "move to the top
/// level", and an id as "move into that folder".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParentChange<'a> {
    /// The field is omitted.
    Unchanged,
    /// `"parent_id": null`.
    TopLevel,
    /// `"parent_id": "<id>"`.
    Folder(&'a str),
}

impl<'a> ParentChange<'a> {
    /// A move to `parent`: that folder, or the top level when there is none.
    pub fn to(parent: Option<&'a str>) -> Self {
        parent.map_or(Self::TopLevel, Self::Folder)
    }
}

/// The body of a metadata update.
pub(super) fn metadata_update_body(
    name_encrypted: Option<&str>,
    parent: ParentChange<'_>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut body = serde_json::Map::new();
    if let Some(name) = name_encrypted {
        body.insert("name_encrypted".into(), serde_json::json!(name));
    }
    match parent {
        ParentChange::Unchanged => {}
        ParentChange::TopLevel => {
            body.insert("parent_id".into(), serde_json::Value::Null);
        }
        ParentChange::Folder(parent_id) => {
            body.insert("parent_id".into(), serde_json::json!(parent_id));
        }
    }
    body
}

/// One entry in `GET /api/v1/search-index/shards`.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub struct SearchShardRef {
    pub bucket: u32,
    pub page: u32,
    pub version: i64,
}

#[derive(Debug, Deserialize)]
pub(super) struct SearchShardManifest {
    #[serde(default)]
    pub(super) shards: Vec<SearchShardRef>,
}

/// Server cap for one encrypted search shard blob. Core writes <=128 KiB pages;
/// the API accepts a little extra framing headroom, matching web.
pub const MAX_SEARCH_SHARD_BLOB_BYTES: usize = 160 * 1024;

/// Server response to `POST /api/v1/clients/devices`. We only need the `id`
/// (to open a session against it), but capture the rest so a caller could
/// surface the device row without a follow-up GET. Extra/unknown server fields
/// are ignored by serde, matching the read-side DTO tolerance.
#[derive(Debug, Clone, Deserialize)]
// ── Client device + sync-session telemetry (the WRITE/PRODUCE side) ────────────
//
// These power the desktop's half of the Bandwidth view: on login the runner
// registers THIS device and opens ONE sync session, then the heartbeat producer
// posts live telemetry every interval. The READ side (`client_devices` /
// `client_sync_sessions` GETs above) is what the BandwidthView polls. Server
// contract: `repos/server/.../routes/clients.rs` under `/api/v1/clients`.

pub struct RegisteredDevice {
    pub id: String,
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub bb_version: Option<String>,
}

/// Server response to `POST /api/v1/clients/sessions`. The heartbeat producer
/// keys off `id` + the (possibly server-defaulted) `heartbeat_interval_secs`.
#[derive(Debug, Clone, Deserialize)]
pub struct CreatedSession {
    pub id: String,
    /// Server defaults this to 60 when the request omits it; we send our own.
    #[serde(default)]
    pub heartbeat_interval_secs: Option<i32>,
}

/// Body for `POST /api/v1/clients/sessions/{id}/heartbeat`.
///
/// Mirrors the server's `HeartbeatBody` exactly. `status` is required (a free-
/// form lowercase string the read side maps to a label: `watching` / `syncing`
/// / `paused` / `idle` / `error`); every other field is optional and skipped on
/// the wire when `None`, so a minimal heartbeat is just `{status}`.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct HeartbeatBody {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files_synced: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files_total: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_synced: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_total: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed_bps: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Owned HTTP client + credentials. One per running daemon process.
///
/// Cloning is intentionally not implemented — the master key has the
/// same security invariant as in the Tauri `Session` struct: exactly
/// one authoritative copy in memory at a time.
///
/// Unlike `Session` (which derives `ZeroizeOnDrop`), this doesn't derive it
/// — `client: reqwest::Client` isn't `Zeroize` — so [`Drop`] is implemented
/// by hand below, wiping `master_key` and the bearer token. Task 1538 Codex P1 (PR #49,
/// lib.rs:1087 thread, "invalidate the session key the engine holds"): once
/// `EngineRunner::abort` confirms the engine's task has actually terminated
/// (not just detached — see that function's doc comment), every `Arc<Self>`
/// clone the task held (the tick loop, the IPC socket server, the Windows
/// upload watcher, the heartbeat producer) is dropped with it, and this Drop
/// impl is what actually erases the key from memory rather than leaving it
/// to linger in a freed allocation.
/// The server refused a custody write because the folder still has legacy key
/// rows and the owner must upgrade it first (task 2258, decision L11). Kept as
/// its own type so no caller can mistake it for a transport error or retry it
/// as a legacy write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UpgradeRequired;

impl std::fmt::Display for UpgradeRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the owner must upgrade this folder before its keys can be shared here")
    }
}

impl std::error::Error for UpgradeRequired {}

/// Whether `error` is the typed L11 refusal (`409 upgrade_required`), on either
/// error shape a caller can hold: the typed [`DesktopApiError`] or an
/// `anyhow::Error` that wraps it.
pub(crate) fn upgrade_required<E: ErrorChain>(error: &E) -> bool {
    error
        .causes()
        .iter()
        .any(|cause| cause.downcast_ref::<UpgradeRequired>().is_some())
}

/// Whether a custody write was refused because this deployment has no custody:
/// a mount without the route answers 405, and the shared-folders feature gate
/// answers `501 {"error":"shared_folders_v1"}` (server
/// `feature_gate::require_shared_folders_enabled`). That is a state, not a
/// failed write to retry. The 501 counts only when it names that gate: any
/// other 501 is a server failure the caller must see (task 2276 P3-D).
pub(crate) fn custody_unavailable<E: ErrorChain + std::fmt::Display>(error: &E) -> bool {
    let text = error.to_string();
    let status = error
        .causes()
        .iter()
        .find_map(|cause| cause.downcast_ref::<reqwest::Error>())
        .and_then(reqwest::Error::status)
        .map(|status| status.as_u16());
    let missing_route = status == Some(405) || text.starts_with("HTTP 405 ");
    let feature_gate =
        (status == Some(501) || text.starts_with("HTTP 501 ")) && text.contains("shared_folders_v1");
    missing_route || feature_gate
}
