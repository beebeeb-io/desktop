//! The data the menu-bar popover reads (task 1683 slice 2; spec
//! `docs/specs/2026-09-30-macos-menubar-popover.md` section 9).
//!
//! One snapshot command returns everything the popover's first frame needs, so the
//! React side (slice 3) makes one `invoke` when it is shown instead of seven. This
//! module is the pure half: given what the command gathered (the engine's last
//! status, the state DB's queue and activity, the account, the Finder state, the
//! cached storage summary) it assembles the snapshot DTO, including the phase from
//! `surfaces::phase::popover_phase`. The gathering, which touches the keychain, the
//! network and the file-provider probe, is `popover_snapshot` in `lib.rs`.
//!
//! ## Event driven, no polling
//!
//! Nothing here runs on a timer. The storage summary is fetched only when the
//! snapshot is requested and the cached one is older than
//! [`STORAGE_TTL_SECS`] (or from another session); a hidden popover therefore costs
//! nothing. Rust tells the UI to refresh with the `popover-shown` event (slice 6
//! emits it when the window is shown); the UI calls the snapshot then.
//!
//! ## What is deliberately not here
//!
//! - The window. Nothing in this module shows, hides or creates one.
//! - Text. The snapshot carries facts and stable codes; the words are the UI's
//!   (spec section 4), except the two mono lines whose content is data
//!   (`reason.detail`, `finder.reason_line`).

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::engine_status::{EngineStatusView, SharedStatusView};
use crate::state_db::{FileEntry, TransferActivity, TransferBacklog};
use crate::surfaces::phase::{FinderFailureReason, FinderSetup, PopoverPhase, PopoverSnapshot, popover_phase};
use crate::transfer_progress::{Direction, Transfer, TransferBoard, display_name, parent_folder};

/// Event Rust emits when the popover is shown; the UI refreshes its snapshot then.
/// Emitted by slice 6 (the window code). The constant lives here so the contract
/// has one home.
#[allow(dead_code)]
pub const POPOVER_SHOWN_EVENT: &str = "popover-shown";

/// How long a storage summary is trusted (spec section 9: "5 minutes, refetched
/// after unlock; never fetched on every open").
pub const STORAGE_TTL_SECS: i64 = 300;

/// How many activity rows a snapshot carries by default (the popover fits five).
pub const DEFAULT_ACTIVITY_LIMIT: usize = 8;
const MAX_ACTIVITY_LIMIT: usize = 20;
/// How many conflicts the snapshot names (`Review` opens the first).
pub const CONFLICT_FILES_LIMIT: usize = 5;

// ── Shared runtime state ────────────────────────────────────────────────────

/// What the engine and the commands share, registered once as Tauri managed state.
/// One engine runs at a time (decision 0800, N = 1), so this is process-wide.
#[derive(Clone)]
pub struct PopoverRuntime {
    /// Bytes done and total per file in flight (written by the engine's chunk loops).
    pub transfers: Arc<TransferBoard>,
    /// The last `engine-status` payload the listener saw.
    pub status: SharedStatusView,
    pub storage: Arc<Mutex<StorageCache>>,
}

impl Default for PopoverRuntime {
    fn default() -> Self {
        Self {
            transfers: TransferBoard::new(),
            status: Arc::new(Mutex::new(EngineStatusView::default())),
            storage: Arc::new(Mutex::new(StorageCache::default())),
        }
    }
}

// ── Storage summary, cached ─────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct StorageUsage {
    pub used_bytes: i64,
    pub quota_bytes: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CacheEntry {
    usage: StorageUsage,
    fetched_at: i64,
    /// The in-memory session the summary was fetched under (`VAULT_EPOCH` in `lib.rs`, which moves
    /// whenever a session is installed or cleared): a sign-in, a lock or a sign-out changes it, so
    /// a summary from before is refetched. Not `UI_SESSION_REVISION`: a lock does not move that.
    vault_epoch: u64,
}

#[derive(Debug, Default)]
pub struct StorageCache {
    entry: Option<CacheEntry>,
}

impl StorageCache {
    /// The cached summary if it is from this session and younger than the TTL.
    fn fresh(&self, now: i64, vault_epoch: u64) -> Option<(StorageUsage, i64)> {
        self.entry
            .filter(|e| e.vault_epoch == vault_epoch && now.saturating_sub(e.fetched_at) < STORAGE_TTL_SECS)
            .map(|e| (e.usage, e.fetched_at))
    }

    /// The cached summary from this session whatever its age (used when the
    /// refetch fails: an old number, marked stale, beats an empty header).
    fn last_known(&self, vault_epoch: u64) -> Option<(StorageUsage, i64)> {
        self.entry
            .filter(|e| e.vault_epoch == vault_epoch)
            .map(|e| (e.usage, e.fetched_at))
    }

    fn put(&mut self, usage: StorageUsage, fetched_at: i64, vault_epoch: u64) {
        self.entry = Some(CacheEntry {
            usage,
            fetched_at,
            vault_epoch,
        });
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct StorageDto {
    pub used_bytes: i64,
    pub quota_bytes: i64,
    pub fetched_at: i64,
    /// `true` when the refetch failed and this is an older number.
    pub stale: bool,
}

/// The storage summary for a snapshot: the cached one while it is fresh, otherwise
/// whatever `fetch` returns, otherwise an older cached one marked stale, otherwise
/// nothing (the header then shows the email only). `fetch` is only awaited when
/// the cache cannot answer, and no lock is held across it.
pub async fn cached_storage<F, Fut>(
    cache: &Mutex<StorageCache>,
    now: i64,
    vault_epoch: u64,
    fetch: F,
) -> Option<StorageDto>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<StorageUsage, String>>,
{
    if let Some((usage, fetched_at)) = cache.lock().ok()?.fresh(now, vault_epoch) {
        return Some(StorageDto {
            used_bytes: usage.used_bytes,
            quota_bytes: usage.quota_bytes,
            fetched_at,
            stale: false,
        });
    }
    match fetch().await {
        Ok(usage) => {
            if let Ok(mut guard) = cache.lock() {
                guard.put(usage, now, vault_epoch);
            }
            Some(StorageDto {
                used_bytes: usage.used_bytes,
                quota_bytes: usage.quota_bytes,
                fetched_at: now,
                stale: false,
            })
        }
        Err(error) => {
            tracing::debug!(error = %error, "storage summary refetch failed; using the last known one if any");
            cache
                .lock()
                .ok()
                .and_then(|guard| guard.last_known(vault_epoch))
                .map(|(usage, fetched_at)| StorageDto {
                    used_bytes: usage.used_bytes,
                    quota_bytes: usage.quota_bytes,
                    fetched_at,
                    stale: true,
                })
        }
    }
}

#[derive(Debug, Deserialize)]
struct BillingUsage {
    used_bytes: i64,
    quota_bytes: i64,
}

/// `GET /api/v1/billing/usage` with the session token. A short timeout: the
/// popover's first frame must not wait 30 s on a dead server. The error text never
/// contains the token.
pub async fn fetch_storage_usage(api_base: &str, token: &str) -> Result<StorageUsage, String> {
    let client = reqwest::Client::builder()
        .default_headers(crate::api_client::provenance_headers())
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|e| format!("reqwest build: {e}"))?;
    let usage: BillingUsage = client
        .get(format!("{api_base}/api/v1/billing/usage"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("load storage usage: {e}"))?
        .error_for_status()
        .map_err(|e| format!("load storage usage: {e}"))?
        .json()
        .await
        .map_err(|e| format!("parse storage usage: {e}"))?;
    Ok(StorageUsage {
        used_bytes: usage.used_bytes,
        quota_bytes: usage.quota_bytes,
    })
}

/// Used at or over a positive quota. A quota of 0 or less means "unknown", never
/// "full".
pub fn storage_is_full(usage: &StorageUsage) -> bool {
    usage.quota_bytes > 0 && usage.used_bytes >= usage.quota_bytes
}

// ── Finder ──────────────────────────────────────────────────────────────────

/// The mono line under state f2 (spec section 4: "reason: timeout"; at most 30
/// characters, single line). Only for a failure with a reason. The reason is the reconciler's
/// typed vocabulary (`FinderFailureReason`, spec 2026-10-06 §6.2): no OS or Rust free text can
/// reach this line. The popover's Finder state itself is the reconciler's published view; there
/// is nothing left here to derive it from a saved install state.
pub fn finder_reason_line(setup: FinderSetup, reason: Option<FinderFailureReason>) -> Option<String> {
    if setup != FinderSetup::Failed {
        return None;
    }
    let line = format!("reason: {}", reason?.as_str());
    Some(if line.chars().count() > 30 {
        let mut cut: String = line.chars().take(29).collect();
        cut.push('…');
        cut
    } else {
        line
    })
}

// ── The snapshot ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AccountDto {
    pub email: Option<String>,
    pub logged_in: bool,
    pub vault_unlocked: bool,
    pub auth_expired: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EngineDto {
    /// `running` | `idle` | `syncing` | `paused` | `offline` | `error` | `stopped`,
    /// or `""` before the engine has said anything.
    pub state: String,
    pub files_remaining: Option<u32>,
    pub bytes_total: Option<u64>,
    pub bytes_done: Option<u64>,
    /// Unix seconds of the last check that reached the server. `None` = unknown:
    /// the UI drops the "Last checked" tooltip rather than invent one.
    pub last_tick_ok_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReasonDto {
    pub code: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FinderDto {
    pub setup: FinderSetup,
    /// Why setup failed (`extension_loading`, `timeout`, ...): a snake_case string, or null.
    pub reason: Option<FinderFailureReason>,
    pub reason_line: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConflictFile {
    pub file_id: String,
    pub file_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConflictsDto {
    pub count: u32,
    /// Oldest first (its 24 h auto-resolution deadline is nearest). `Review` opens
    /// the first.
    pub files: Vec<ConflictFile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityState {
    Active,
    Done,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ActivityRow {
    pub id: String,
    pub name: String,
    /// The name of the folder the file sits in (`Investigations`, not `Work/Investigations`),
    /// `""` at the vault root: the short label under the file name.
    pub folder: String,
    /// The full path inside the vault, without a leading separator (for a tooltip).
    pub path: String,
    /// `None` only for a failed row (the state DB does not record which way it was going).
    pub direction: Option<Direction>,
    pub state: ActivityState,
    /// Unix seconds: when it finished (done), or its last change (active, failed).
    pub at: i64,
    pub size_bytes: Option<u64>,
    /// Per-file progress while active. `None` = unknown; the UI shows no percentage
    /// then (spec state b'), never a guess.
    pub done_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PopoverSnapshotDto {
    pub phase: PopoverPhase,
    pub generated_at: i64,
    pub account: AccountDto,
    pub paused: bool,
    pub engine: EngineDto,
    pub reason: Option<ReasonDto>,
    pub finder: FinderDto,
    pub storage: Option<StorageDto>,
    pub storage_full: bool,
    /// Queued changes (state c2's "{n} changes are waiting", state s's "{n} changes
    /// can't upload").
    pub pending_changes: u32,
    pub conflicts: ConflictsDto,
    pub activity: Vec<ActivityRow>,
}

/// Everything the command gathered.
#[derive(Debug, Clone)]
pub struct SnapshotInputs {
    pub now: i64,
    pub logged_in: bool,
    pub vault_unlocked: bool,
    pub auth_expired: bool,
    /// The live pause flag (`sync_paused`), the source of truth.
    pub paused: bool,
    pub email: Option<String>,
    pub engine: EngineStatusView,
    pub finder: FinderSetup,
    pub finder_reason: Option<FinderFailureReason>,
    /// Another surface is showing the failed install (see `surfaces::failure`).
    pub finder_failure_elsewhere: bool,
    pub storage: Option<StorageDto>,
    pub backlog: TransferBacklog,
    /// Every row in `conflict` status.
    pub conflicts: Vec<FileEntry>,
    /// Rows in `uploading` / `downloading` status.
    pub in_flight: Vec<FileEntry>,
    /// Rows in `error` status.
    pub failed: Vec<FileEntry>,
    /// Finished transfers, newest first.
    pub recent: Vec<TransferActivity>,
    /// The transfer board right now.
    pub board: Vec<(String, Transfer)>,
    pub activity_limit: usize,
}

fn entry_row(
    entry: &FileEntry,
    direction: Option<Direction>,
    state: ActivityState,
    transfer: Option<&Transfer>,
) -> ActivityRow {
    ActivityRow {
        id: entry.file_id.clone(),
        name: display_name(&entry.path),
        folder: display_name(&parent_folder(&entry.path)),
        path: entry.path.trim_start_matches(['/', '\\']).to_string(),
        direction,
        state,
        at: entry.modified_at,
        size_bytes: u64::try_from(entry.size_bytes).ok(),
        done_bytes: transfer.map(|t| t.done),
        total_bytes: transfer.map(|t| t.total),
    }
}

fn newest_first(entries: &mut [&FileEntry]) {
    entries.sort_by(|a, b| b.modified_at.cmp(&a.modified_at).then_with(|| a.path.cmp(&b.path)));
}

/// The activity list: what is moving now, then what failed, then what finished.
/// A finished transfer of a file that is moving again is left out (the active row
/// says it), and a file appears once among the finished.
pub fn build_activity_rows(inputs: &SnapshotInputs) -> Vec<ActivityRow> {
    let board: HashMap<&str, &Transfer> = inputs.board.iter().map(|(id, t)| (id.as_str(), t)).collect();
    let mut rows: Vec<ActivityRow> = Vec::new();

    let mut active: Vec<&FileEntry> = inputs
        .in_flight
        .iter()
        .filter(|e| {
            matches!(
                e.status,
                crate::state_db::FileStatus::Uploading | crate::state_db::FileStatus::Downloading
            )
        })
        .collect();
    newest_first(&mut active);
    for entry in &active {
        let direction = if entry.status == crate::state_db::FileStatus::Uploading {
            Direction::Up
        } else {
            Direction::Down
        };
        rows.push(entry_row(
            entry,
            Some(direction),
            ActivityState::Active,
            board.get(entry.file_id.as_str()).copied(),
        ));
    }

    let mut failed: Vec<&FileEntry> = inputs.failed.iter().collect();
    newest_first(&mut failed);
    for entry in failed {
        rows.push(entry_row(entry, None, ActivityState::Failed, None));
    }

    let moving: std::collections::HashSet<&str> = active.iter().map(|e| e.file_id.as_str()).collect();
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for done in &inputs.recent {
        if let Some(id) = done.file_id.as_deref() {
            if moving.contains(id) || !seen.insert(id) {
                continue;
            }
        }
        let path = done.rel_path.as_deref().unwrap_or(&done.file_name);
        rows.push(ActivityRow {
            id: done.file_id.clone().unwrap_or_else(|| format!("transfer-{}", done.id)),
            name: display_name(path),
            folder: display_name(&parent_folder(path)),
            path: path.trim_start_matches(['/', '\\']).to_string(),
            direction: match done.direction.as_str() {
                "up" => Some(Direction::Up),
                "down" => Some(Direction::Down),
                _ => None,
            },
            state: ActivityState::Done,
            at: done.occurred_at,
            size_bytes: u64::try_from(done.bytes).ok(),
            done_bytes: None,
            total_bytes: None,
        });
    }

    rows.truncate(inputs.activity_limit.clamp(1, MAX_ACTIVITY_LIMIT));
    rows
}

/// The state-DB half of the snapshot's inputs.
#[derive(Debug, Clone, Default)]
pub struct DbView {
    pub backlog: TransferBacklog,
    pub conflicts: Vec<FileEntry>,
    pub in_flight: Vec<FileEntry>,
    pub failed: Vec<FileEntry>,
    pub recent: Vec<TransferActivity>,
}

/// Read what the snapshot needs from the state DB: the queue, the rows in flight,
/// failed and in conflict, and the newest finished transfers. `now` is unix seconds, for which
/// queued uploads are due.
pub fn gather_db_view(db: &crate::state_db::StateDb, activity_limit: usize, now: i64) -> Result<DbView, String> {
    use crate::state_db::FileStatus;
    let list = |status| db.list_by_status(status).map_err(|e| format!("list files: {e}"));
    let mut in_flight = list(FileStatus::Uploading)?;
    in_flight.extend(list(FileStatus::Downloading)?);
    Ok(DbView {
        backlog: db.transfer_backlog(now).map_err(|e| format!("transfer backlog: {e}"))?,
        conflicts: list(FileStatus::Conflict)?,
        in_flight,
        failed: list(FileStatus::Error)?,
        // Twice the list length: finished rows of files that are moving again are dropped.
        recent: db
            .list_recent_transfer_activity(activity_limit.saturating_mul(2))
            .map_err(|e| format!("recent transfers: {e}"))?,
    })
}

/// Assemble the snapshot. Pure: every input is a value, nothing is read.
pub fn assemble(inputs: SnapshotInputs) -> PopoverSnapshotDto {
    let storage_full = inputs.storage.as_ref().is_some_and(|s| {
        storage_is_full(&StorageUsage {
            used_bytes: s.used_bytes,
            quota_bytes: s.quota_bytes,
        })
    }) || inputs.backlog.paused_for_quota > 0;

    let phase = popover_phase(&PopoverSnapshot {
        logged_in: inputs.logged_in,
        auth_expired: inputs.auth_expired,
        vault_unlocked: inputs.vault_unlocked,
        finder: inputs.finder,
        finder_reason: inputs.finder_reason,
        paused: inputs.paused,
        storage_full,
        connectivity: inputs.engine.connectivity(),
        // A file moving right now is syncing even if the last event said idle (the pulse is up to
        // a second behind): never an active row under a "synced" phase.
        syncing: inputs.engine.is_syncing() || !inputs.board.is_empty(),
        finder_failure_elsewhere: inputs.finder_failure_elsewhere,
    });

    let link_down = matches!(inputs.engine.state.as_str(), "offline" | "error");
    let reason = if link_down {
        inputs.engine.reason_code.clone().map(|code| ReasonDto {
            code,
            detail: inputs.engine.reason_detail.clone(),
        })
    } else {
        None
    };

    let activity = build_activity_rows(&inputs);

    let mut conflicts: Vec<&FileEntry> = inputs.conflicts.iter().collect();
    conflicts.sort_by(|a, b| a.modified_at.cmp(&b.modified_at).then_with(|| a.path.cmp(&b.path)));
    let conflict_files = conflicts
        .iter()
        .take(CONFLICT_FILES_LIMIT)
        .map(|e| ConflictFile {
            file_id: e.file_id.clone(),
            file_name: display_name(&e.path),
        })
        .collect();

    PopoverSnapshotDto {
        phase,
        generated_at: inputs.now,
        account: AccountDto {
            email: inputs.email,
            logged_in: inputs.logged_in,
            vault_unlocked: inputs.vault_unlocked,
            auth_expired: inputs.auth_expired,
        },
        paused: inputs.paused,
        engine: EngineDto {
            state: inputs.engine.state.clone(),
            files_remaining: inputs.engine.files_remaining,
            bytes_total: inputs.engine.bytes_total,
            bytes_done: inputs.engine.bytes_done,
            last_tick_ok_at: inputs.engine.last_tick_ok_at,
        },
        reason,
        finder: FinderDto {
            setup: inputs.finder,
            reason_line: finder_reason_line(inputs.finder, inputs.finder_reason),
            reason: inputs.finder_reason,
        },
        storage: inputs.storage,
        storage_full,
        pending_changes: inputs.backlog.queued_ops,
        conflicts: ConflictsDto {
            count: u32::try_from(inputs.conflicts.len()).unwrap_or(u32::MAX),
            files: conflict_files,
        },
        activity,
    }
}

// ── The review window's retarget event ──────────────────────────────────────

/// Event the popover (or a notification click) sends to point the single `review`
/// window at another file (spec section 6, rule 9). `ConflictWindow` reads
/// `fileId`, `fileName` and `isText` from its URL once at mount, so one window
/// needs a way to be retargeted: it handles this event and remounts keyed by
/// `fileId`. The keys are camelCase, like the URL parameters they replace.
pub const REVIEW_OPEN_EVENT: &str = "review:open";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewMode {
    Conflict,
    Versions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewOpen {
    pub file_id: String,
    pub file_name: String,
    pub is_text: bool,
    pub mode: ReviewMode,
}

impl ReviewOpen {
    /// `None` when the event cannot point anywhere (no file id), so a malformed
    /// request never blanks the window.
    pub fn new(file_id: &str, file_name: &str, is_text: bool, mode: ReviewMode) -> Option<Self> {
        if file_id.trim().is_empty() {
            return None;
        }
        Some(Self {
            file_id: file_id.to_string(),
            file_name: if file_name.is_empty() {
                "Unknown file".to_string()
            } else {
                file_name.to_string()
            },
            is_text,
            mode,
        })
    }
}

/// Send the retarget event. The `review` window that handles it is slice 6's.
#[allow(dead_code)]
pub fn emit_review_open(app: &tauri::AppHandle, payload: &ReviewOpen) -> tauri::Result<()> {
    use tauri::Emitter;
    app.emit(REVIEW_OPEN_EVENT, payload)
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_db::{FileStatus, ItemKind};
    use serde_json::json;

    fn entry(id: &str, path: &str, status: FileStatus, size: i64, at: i64) -> FileEntry {
        FileEntry {
            file_id: id.into(),
            path: path.into(),
            status,
            size_bytes: size,
            modified_at: at,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: ItemKind::File,
        }
    }

    fn done(id: &str, dir: &str, path: &str, at: i64) -> TransferActivity {
        TransferActivity {
            id: at,
            direction: dir.into(),
            file_id: Some(id.into()),
            file_name: display_name(path),
            rel_path: Some(path.into()),
            bytes: 10,
            occurred_at: at,
        }
    }

    fn view(payload: serde_json::Value) -> EngineStatusView {
        let mut v = EngineStatusView::default();
        v.apply(&payload);
        v
    }

    fn healthy() -> SnapshotInputs {
        SnapshotInputs {
            now: 1_000,
            logged_in: true,
            vault_unlocked: true,
            auth_expired: false,
            paused: false,
            email: Some("sam@example.eu".into()),
            engine: view(
                json!({ "state": "idle", "files_remaining": 0, "bytes_total": 0, "bytes_done": 0, "last_tick_ok_at": 990 }),
            ),
            finder: FinderSetup::Ready,
            finder_reason: None,
            finder_failure_elsewhere: false,
            storage: Some(StorageDto {
                used_bytes: 84_300_000_000,
                quota_bytes: 200_000_000_000,
                fetched_at: 900,
                stale: false,
            }),
            backlog: TransferBacklog::default(),
            conflicts: vec![],
            in_flight: vec![],
            failed: vec![],
            recent: vec![],
            board: vec![],
            activity_limit: DEFAULT_ACTIVITY_LIMIT,
        }
    }

    // ── phase, one test per state ───────────────────────────────────────────

    fn phase_of(inputs: SnapshotInputs) -> PopoverPhase {
        assemble(inputs).phase
    }

    #[test]
    fn every_phase_is_reachable_from_the_inputs_the_command_gathers() {
        use PopoverPhase::*;
        assert_eq!(phase_of(healthy()), Synced);

        let mut i = healthy();
        i.logged_in = false;
        i.vault_unlocked = false;
        assert_eq!(phase_of(i), SignedOut);

        let mut i = healthy();
        i.auth_expired = true;
        assert_eq!(phase_of(i), SessionEnded);

        let mut i = healthy();
        i.vault_unlocked = false;
        assert_eq!(phase_of(i), Locked);

        let mut i = healthy();
        i.finder = FinderSetup::Failed;
        assert_eq!(phase_of(i), FinderFailed);

        let mut i = healthy();
        i.finder = FinderSetup::UserDisabled;
        assert_eq!(
            phase_of(i),
            FinderUserDisabled,
            "lead ruling: Finder user_disabled has its own phase"
        );

        let mut i = healthy();
        i.finder = FinderSetup::Adding;
        assert_eq!(phase_of(i), FinderAdding);

        let mut i = healthy();
        i.finder = FinderSetup::Missing;
        assert_eq!(phase_of(i), FinderMissing);

        let mut i = healthy();
        i.paused = true;
        assert_eq!(phase_of(i), Paused);

        let mut i = healthy();
        i.storage = Some(StorageDto {
            used_bytes: 200,
            quota_bytes: 200,
            fetched_at: 1,
            stale: false,
        });
        assert_eq!(phase_of(i), StorageFull);

        let mut i = healthy();
        i.engine = view(json!({ "state": "offline", "reason": { "code": "connect", "detail": null } }));
        assert_eq!(phase_of(i), Offline);

        let mut i = healthy();
        i.engine = view(json!({ "state": "error", "reason": { "code": "timeout", "detail": "timeout after 30 s" } }));
        assert_eq!(phase_of(i), Error);

        let mut i = healthy();
        i.engine = view(json!({ "state": "syncing", "files_remaining": 3 }));
        assert_eq!(phase_of(i), Syncing);
    }

    #[test]
    fn a_file_on_the_board_is_syncing_even_when_the_last_event_said_idle() {
        let mut i = healthy();
        i.board = vec![(
            "u1".into(),
            Transfer {
                direction: Direction::Up,
                done: 1,
                total: 9,
            },
        )];
        assert_eq!(phase_of(i.clone()), PopoverPhase::Syncing);
        // Higher phases still win.
        i.paused = true;
        assert_eq!(phase_of(i), PopoverPhase::Paused);
    }

    #[test]
    fn a_quota_paused_upload_is_storage_full_even_when_the_usage_call_failed() {
        let mut i = healthy();
        i.storage = None;
        i.backlog.paused_for_quota = 2;
        let s = assemble(i);
        assert!(s.storage_full);
        assert_eq!(s.phase, PopoverPhase::StorageFull);
    }

    #[test]
    fn a_zero_quota_is_unknown_not_full() {
        assert!(!storage_is_full(&StorageUsage {
            used_bytes: 0,
            quota_bytes: 0
        }));
        assert!(!storage_is_full(&StorageUsage {
            used_bytes: 5,
            quota_bytes: -1
        }));
        assert!(storage_is_full(&StorageUsage {
            used_bytes: 200,
            quota_bytes: 200
        }));
        assert!(!storage_is_full(&StorageUsage {
            used_bytes: 199,
            quota_bytes: 200
        }));
    }

    #[test]
    fn a_failure_shown_elsewhere_does_not_paint_f2_here() {
        let mut i = healthy();
        i.finder = FinderSetup::Failed;
        i.finder_failure_elsewhere = true;
        assert_eq!(phase_of(i), PopoverPhase::FinderMissing);
    }

    #[test]
    fn a_401_error_is_not_a_connectivity_phase() {
        let mut i = healthy();
        i.engine = view(json!({ "state": "error", "reason": { "code": "auth", "detail": null } }));
        assert_eq!(phase_of(i), PopoverPhase::Synced);
    }

    // ── the serialized contract ─────────────────────────────────────────────

    /// The JSON the TypeScript side validates (`tests/popoverContract.test.ts` runs the same
    /// files through `parsePopoverSnapshot`), so the two languages cannot drift apart silently.
    fn fixture(name: &str) -> serde_json::Value {
        let text = match name {
            "synced" => include_str!("../../tests/fixtures/popover-snapshot.synced.json"),
            "syncing" => include_str!("../../tests/fixtures/popover-snapshot.syncing.json"),
            "error" => include_str!("../../tests/fixtures/popover-snapshot.error.json"),
            other => panic!("no fixture {other}"),
        };
        serde_json::from_str(text).expect("fixture is JSON")
    }

    #[test]
    fn the_healthy_snapshot_serializes_to_exactly_the_shared_fixture() {
        assert_eq!(serde_json::to_value(assemble(healthy())).unwrap(), fixture("synced"));
    }

    fn error_inputs() -> SnapshotInputs {
        let mut i = healthy();
        i.engine = view(
            json!({ "state": "error", "reason": { "code": "timeout", "detail": "timeout after 30 s" }, "last_tick_ok_at": 500 }),
        );
        i
    }

    #[test]
    fn the_error_snapshot_serializes_to_exactly_the_shared_fixture() {
        assert_eq!(
            serde_json::to_value(assemble(error_inputs())).unwrap(),
            fixture("error")
        );
    }

    fn syncing_inputs() -> SnapshotInputs {
        let mut i = healthy();
        i.engine = view(json!({
            "state": "syncing", "files_remaining": 14,
            "bytes_total": 1_200_000_000_u64, "bytes_done": 456_000_000_u64
        }));
        i.in_flight = vec![
            entry("u1", "Investigations/ledger.xlsx", FileStatus::Uploading, 100, 50),
            entry("d1", "Notes/meeting.md", FileStatus::Downloading, 200, 40),
        ];
        i.board = vec![(
            "u1".into(),
            Transfer {
                direction: Direction::Up,
                done: 62,
                total: 100,
            },
        )];
        i.failed = vec![entry("e1", "Notes/bad.bin", FileStatus::Error, 10, 20)];
        i.recent = vec![done("r1", "up", "Reports/q3.pdf", 10)];
        i
    }

    #[test]
    fn the_syncing_snapshot_serializes_to_exactly_the_shared_fixture() {
        assert_eq!(
            serde_json::to_value(assemble(syncing_inputs())).unwrap(),
            fixture("syncing")
        );
    }

    #[test]
    fn the_reason_is_omitted_when_the_link_is_fine() {
        let mut i = healthy();
        // A stale reason left in the view must not leak into a healthy snapshot.
        i.engine.reason_code = Some("timeout".into());
        assert_eq!(assemble(i).reason, None);
    }

    #[test]
    fn the_finder_failure_carries_the_reason_line() {
        let mut i = healthy();
        i.finder = FinderSetup::Failed;
        i.finder_reason = Some(FinderFailureReason::Timeout);
        let s = serde_json::to_value(assemble(i)).unwrap();
        assert_eq!(
            s["finder"],
            json!({ "setup": "failed", "reason": "timeout", "reason_line": "reason: timeout" })
        );
    }

    #[test]
    fn pending_changes_and_conflicts_are_reported() {
        let mut i = healthy();
        i.backlog.queued_ops = 7;
        i.conflicts = vec![
            entry("c2", "Docs/b.txt", FileStatus::Conflict, 1, 200),
            entry("c1", "Docs/a.txt", FileStatus::Conflict, 1, 100),
        ];
        let s = assemble(i);
        assert_eq!(s.pending_changes, 7);
        assert_eq!(s.conflicts.count, 2);
        assert_eq!(
            s.conflicts.files[0],
            ConflictFile {
                file_id: "c1".into(),
                file_name: "a.txt".into()
            }
        );
        assert_eq!(s.conflicts.files[1].file_id, "c2");
    }

    #[test]
    fn only_a_handful_of_conflicts_are_named_but_all_are_counted() {
        let mut i = healthy();
        i.conflicts = (0..9)
            .map(|n| entry(&format!("c{n}"), &format!("f{n}.txt"), FileStatus::Conflict, 1, n))
            .collect();
        let s = assemble(i);
        assert_eq!(s.conflicts.count, 9);
        assert_eq!(s.conflicts.files.len(), CONFLICT_FILES_LIMIT);
    }

    // ── activity rows ───────────────────────────────────────────────────────

    #[test]
    fn rows_are_active_first_then_failed_then_done_with_direction_each() {
        let mut i = healthy();
        i.in_flight = vec![
            entry("d1", "A/down.bin", FileStatus::Downloading, 10, 30),
            entry("u1", "A/up.bin", FileStatus::Uploading, 10, 40),
        ];
        i.failed = vec![entry("e1", "A/bad.bin", FileStatus::Error, 10, 20)];
        i.recent = vec![done("r1", "up", "B/sent.txt", 10), done("r2", "down", "B/got.txt", 5)];
        let rows = build_activity_rows(&i);
        let shape: Vec<(&str, ActivityState, Option<Direction>)> =
            rows.iter().map(|r| (r.id.as_str(), r.state, r.direction)).collect();
        assert_eq!(
            shape,
            vec![
                ("u1", ActivityState::Active, Some(Direction::Up)),
                ("d1", ActivityState::Active, Some(Direction::Down)),
                ("e1", ActivityState::Failed, None),
                ("r1", ActivityState::Done, Some(Direction::Up)),
                ("r2", ActivityState::Done, Some(Direction::Down)),
            ]
        );
    }

    #[test]
    fn an_active_row_without_board_bytes_has_no_percentage_to_show() {
        let mut i = healthy();
        i.in_flight = vec![entry("u1", "x.bin", FileStatus::Uploading, 10, 1)];
        let rows = build_activity_rows(&i);
        assert_eq!(
            (rows[0].done_bytes, rows[0].total_bytes),
            (None, None),
            "state b', never a guessed percentage"
        );
    }

    #[test]
    fn a_finished_transfer_of_a_file_moving_again_is_not_listed_twice() {
        let mut i = healthy();
        i.in_flight = vec![entry("x", "f.bin", FileStatus::Uploading, 10, 50)];
        i.recent = vec![done("x", "up", "f.bin", 40), done("y", "up", "g.bin", 30)];
        let ids: Vec<String> = build_activity_rows(&i).into_iter().map(|r| r.id).collect();
        assert_eq!(ids, vec!["x".to_string(), "y".to_string()]);
    }

    #[test]
    fn a_file_uploaded_twice_is_one_finished_row_the_newest() {
        let mut i = healthy();
        i.recent = vec![done("x", "up", "f.bin", 40), done("x", "up", "f.bin", 10)];
        let rows = build_activity_rows(&i);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].at, 40);
    }

    #[test]
    fn the_list_is_capped_and_the_cap_is_clamped() {
        let mut i = healthy();
        i.recent = (0..30)
            .map(|n| done(&format!("r{n}"), "up", &format!("f{n}.txt"), 100 - n))
            .collect();
        i.activity_limit = 5;
        assert_eq!(build_activity_rows(&i).len(), 5);
        i.activity_limit = 0;
        assert_eq!(build_activity_rows(&i).len(), 1, "a zero limit still returns one row");
        i.activity_limit = 500;
        assert_eq!(build_activity_rows(&i).len(), MAX_ACTIVITY_LIMIT);
    }

    #[test]
    fn a_row_path_splits_into_name_folder_name_and_path() {
        let mut i = healthy();
        i.recent = vec![
            done("r", "down", "Work/2026/Q3/ledger.xlsx", 3),
            done("s", "up", "/Top/notes.md", 2),
            done("t", "up", "root.txt", 1),
        ];
        let rows = build_activity_rows(&i);
        let shape: Vec<(&str, &str, &str)> = rows
            .iter()
            .map(|r| (r.name.as_str(), r.folder.as_str(), r.path.as_str()))
            .collect();
        assert_eq!(
            shape,
            vec![
                ("ledger.xlsx", "Q3", "Work/2026/Q3/ledger.xlsx"),
                ("notes.md", "Top", "Top/notes.md"),
                ("root.txt", "", "root.txt"),
            ],
            "the short label is the immediate parent folder; the path has no leading slash"
        );
    }

    // ── finder ──────────────────────────────────────────────────────────────

    // The Finder state itself is the reconciler's published view (spec 2026-10-06): the saved
    // install state and the "adding" flag these tests used to map are gone with `finder_setup_for`
    // and `PopoverRuntime::finder_adding`. `lib.rs`'s popover command tests cover the view.

    #[test]
    fn the_finder_reason_line_adds_the_typed_reason_and_fits_30_characters() {
        assert_eq!(
            finder_reason_line(FinderSetup::Failed, Some(FinderFailureReason::Timeout)).as_deref(),
            Some("reason: timeout")
        );
        assert_eq!(finder_reason_line(FinderSetup::Failed, None), None);
        assert_eq!(
            finder_reason_line(FinderSetup::Missing, Some(FinderFailureReason::Timeout)),
            None,
            "only a failure has a reason line"
        );
    }

    #[test]
    fn every_reason_line_fits_30_characters() {
        for reason in FinderFailureReason::ALL {
            let line =
                finder_reason_line(FinderSetup::Failed, Some(reason)).expect("a failure with a reason has a line");
            assert!(line.chars().count() <= 30, "{line}");
            assert_eq!(
                line,
                format!("reason: {}", reason.as_str()),
                "the typed reason, whole, never a cut word"
            );
        }
    }

    // ── the storage cache ───────────────────────────────────────────────────

    fn usage(used: i64) -> StorageUsage {
        StorageUsage {
            used_bytes: used,
            quota_bytes: 200,
        }
    }

    fn block<T>(f: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(f)
    }

    #[test]
    fn a_fresh_summary_is_served_without_fetching() {
        let cache = Mutex::new(StorageCache::default());
        let calls = std::cell::Cell::new(0);
        let fetch = || {
            calls.set(calls.get() + 1);
            async { Ok(usage(10)) }
        };
        let a = block(cached_storage(&cache, 1_000, 7, fetch)).unwrap();
        assert_eq!((a.used_bytes, a.stale, a.fetched_at), (10, false, 1_000));
        // 20 "opens" inside the TTL: still one fetch.
        for n in 1..=20 {
            let again = block(cached_storage(&cache, 1_000 + n, 7, || {
                calls.set(calls.get() + 1);
                async { Ok(usage(99)) }
            }))
            .unwrap();
            assert_eq!(again.used_bytes, 10, "served from the cache");
        }
        assert_eq!(calls.get(), 1, "21 snapshots inside the TTL must fetch exactly once");
    }

    #[test]
    fn the_summary_is_refetched_after_the_ttl() {
        let cache = Mutex::new(StorageCache::default());
        block(cached_storage(&cache, 0, 1, || async { Ok(usage(10)) })).unwrap();
        let just_inside = block(cached_storage(&cache, STORAGE_TTL_SECS - 1, 1, || async {
            Ok(usage(20))
        }))
        .unwrap();
        assert_eq!(just_inside.used_bytes, 10);
        let at_ttl = block(cached_storage(&cache, STORAGE_TTL_SECS, 1, || async { Ok(usage(30)) })).unwrap();
        assert_eq!(at_ttl.used_bytes, 30, "at the TTL the summary is refetched");
    }

    #[test]
    fn a_new_session_refetches_even_inside_the_ttl() {
        // Spec: "refetched after unlock". The session revision changes on unlock.
        let cache = Mutex::new(StorageCache::default());
        block(cached_storage(&cache, 0, 1, || async { Ok(usage(10)) })).unwrap();
        let after_unlock = block(cached_storage(&cache, 5, 2, || async { Ok(usage(55)) })).unwrap();
        assert_eq!(after_unlock.used_bytes, 55);
    }

    #[test]
    fn a_failed_refetch_serves_the_old_number_marked_stale() {
        let cache = Mutex::new(StorageCache::default());
        block(cached_storage(&cache, 0, 1, || async { Ok(usage(10)) })).unwrap();
        let stale = block(cached_storage(&cache, STORAGE_TTL_SECS + 50, 1, || async {
            Err("offline".to_string())
        }))
        .unwrap();
        assert_eq!((stale.used_bytes, stale.stale, stale.fetched_at), (10, true, 0));
    }

    #[test]
    fn a_failed_fetch_with_nothing_cached_is_no_summary_and_another_sessions_is_not_reused() {
        let cache = Mutex::new(StorageCache::default());
        assert_eq!(
            block(cached_storage(&cache, 0, 1, || async { Err("offline".to_string()) })),
            None
        );
        block(cached_storage(&cache, 0, 1, || async { Ok(usage(10)) })).unwrap();
        assert_eq!(
            block(cached_storage(&cache, 1, 2, || async { Err("offline".to_string()) })),
            None,
            "a summary from another session must never be shown"
        );
    }

    // ── the real fetch, against a loopback API ──────────────────────────────

    fn serve_once(raw: &'static str) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 8192];
            let n = stream.read(&mut buf).unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]).into_owned();
            let _ = stream.write_all(raw.as_bytes());
            request
        });
        (base, handle)
    }

    #[test]
    fn the_storage_fetch_calls_billing_usage_with_the_bearer_token_and_parses_it() {
        let (base, server) = serve_once(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 53\r\nconnection: close\r\n\r\n{\"used_bytes\":84300000000,\"quota_bytes\":200000000000}",
        );
        // reqwest honours proxy env vars; a loopback API must be reached directly.
        let got = block(fetch_storage_usage(&base, "tok-123")).unwrap();
        assert_eq!(
            got,
            StorageUsage {
                used_bytes: 84_300_000_000,
                quota_bytes: 200_000_000_000
            }
        );
        let request = server.join().unwrap().to_ascii_lowercase();
        assert!(request.starts_with("get /api/v1/billing/usage"), "{request}");
        assert!(request.contains("authorization: bearer tok-123"), "{request}");
    }

    #[test]
    fn a_401_from_the_usage_call_is_an_error_that_does_not_carry_the_token() {
        let (base, server) = serve_once("HTTP/1.1 401 Unauthorized\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
        let err = block(fetch_storage_usage(&base, "secret-token-abc")).unwrap_err();
        server.join().unwrap();
        assert!(err.contains("401"), "{err}");
        assert!(!err.contains("secret-token-abc"), "{err}");
    }

    // ── the review:open contract ────────────────────────────────────────────

    #[test]
    fn the_review_event_is_camel_case_like_the_url_it_replaces() {
        let ev = ReviewOpen::new("f-1", "ledger.xlsx", false, ReviewMode::Conflict).unwrap();
        assert_eq!(
            serde_json::to_value(&ev).unwrap(),
            json!({ "fileId": "f-1", "fileName": "ledger.xlsx", "isText": false, "mode": "conflict" })
        );
        // The same file the TypeScript test parses.
        let shared: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/review-open.conflict.json")).unwrap();
        assert_eq!(serde_json::to_value(&ev).unwrap(), shared);
        let back: ReviewOpen =
            serde_json::from_value(json!({ "fileId": "f-2", "fileName": "a", "isText": true, "mode": "versions" }))
                .unwrap();
        assert_eq!(back.mode, ReviewMode::Versions);
        assert!(back.is_text);
        assert_eq!(REVIEW_OPEN_EVENT, "review:open");
    }

    #[test]
    fn a_review_event_with_no_file_id_is_refused_and_a_missing_name_gets_the_window_default() {
        assert_eq!(ReviewOpen::new("", "x", false, ReviewMode::Conflict), None);
        assert_eq!(ReviewOpen::new("   ", "x", false, ReviewMode::Conflict), None);
        assert_eq!(
            ReviewOpen::new("f", "", false, ReviewMode::Versions).unwrap().file_name,
            "Unknown file"
        );
    }

    // ── from a real state DB ────────────────────────────────────────────────

    fn seed(db: &crate::state_db::StateDb, id: &str, path: &str, status: FileStatus, size: i64, at: i64) {
        db.upsert_file(&entry(id, path, status, size, at)).unwrap();
    }

    fn queued(id: &str, kind: crate::state_db::OperationKind, file_id: &str) -> crate::state_db::PendingOperation {
        crate::state_db::PendingOperation {
            op_id: id.into(),
            kind,
            file_id: Some(file_id.into()),
            parent_id: None,
            target_path: None,
            metadata_json: None,
            payload_path: None,
            base_version: None,
            base_object_version_id: None,
            attempts: 0,
            max_attempts: 5,
            next_retry_at: 0,
            last_error: None,
            backup_source_key: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    #[test]
    fn the_snapshot_reads_the_queue_as_of_now_and_skips_an_upload_in_retry_backoff() {
        use crate::state_db::{OperationKind, StateDb};
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed(&db, "a", "A/due.bin", FileStatus::Local, 100, 1);
        seed(&db, "b", "A/waiting.bin", FileStatus::Local, 5_000, 1);
        let mut due = queued("op-a", OperationKind::UploadFile, "a");
        due.next_retry_at = 50;
        let mut waiting = queued("op-b", OperationKind::UploadFile, "b");
        waiting.attempts = 1;
        waiting.next_retry_at = 500;
        db.enqueue_operation(&due).unwrap();
        db.enqueue_operation(&waiting).unwrap();

        let at_100 = gather_db_view(&db, 5, 100).unwrap().backlog;
        assert_eq!(
            (at_100.upload_files, at_100.upload_bytes),
            (1, 100),
            "only op-a is due at t=100"
        );
        let at_500 = gather_db_view(&db, 5, 500).unwrap().backlog;
        assert_eq!(
            (at_500.upload_files, at_500.upload_bytes),
            (2, 5_100),
            "op-b is due again at t=500"
        );
    }

    #[test]
    fn a_real_state_db_becomes_the_snapshot_the_popover_shows() {
        use crate::state_db::{OperationKind, OperationPauseReason, StateDb, TransferActivityInput};
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        seed(&db, "u1", "Investigations/ledger.xlsx", FileStatus::Uploading, 100, 50);
        seed(&db, "d1", "Notes/meeting.md", FileStatus::Downloading, 200, 40);
        seed(&db, "e1", "Notes/bad.bin", FileStatus::Error, 10, 20);
        seed(&db, "c1", "Docs/plan.txt", FileStatus::Conflict, 5, 30);
        seed(&db, "l1", "Reports/q3.pdf", FileStatus::Local, 10, 10);
        seed(&db, "p1", "Big/paused.bin", FileStatus::Local, 999, 5);
        db.enqueue_operation(&queued("op-u1", OperationKind::UploadFile, "u1"))
            .unwrap();
        db.enqueue_operation(&queued("op-p1", OperationKind::UploadFile, "p1"))
            .unwrap();
        db.record_operation_pause("op-p1", OperationPauseReason::Quota, Some("quota exceeded"), 9)
            .unwrap();
        db.record_transfer_activity(TransferActivityInput {
            direction: "up",
            file_id: Some("l1".into()),
            file_name: "q3.pdf".into(),
            rel_path: Some("Reports/q3.pdf".into()),
            bytes: 10,
            occurred_at: 10,
        })
        .unwrap();

        let view = gather_db_view(&db, 5, 100).unwrap();
        assert_eq!(view.backlog.upload_files, 1, "op-u1 is due, op-p1 is paused for quota");
        assert_eq!(view.backlog.download_files, 1);
        assert_eq!(view.backlog.queued_ops, 2);
        assert_eq!(view.backlog.paused_for_quota, 1);
        assert_eq!(view.in_flight.len(), 2);
        assert_eq!(view.failed.len(), 1);
        assert_eq!(view.conflicts.len(), 1);
        assert_eq!(view.recent.len(), 1);

        let mut inputs = healthy();
        inputs.engine = view_of_syncing();
        inputs.backlog = view.backlog;
        inputs.conflicts = view.conflicts;
        inputs.in_flight = view.in_flight;
        inputs.failed = view.failed;
        inputs.recent = view.recent;
        inputs.board = vec![(
            "u1".into(),
            Transfer {
                direction: Direction::Up,
                done: 62,
                total: 100,
            },
        )];
        let snapshot = assemble(inputs);
        // A quota-paused upload outranks syncing (spec precedence: storage full above syncing).
        assert_eq!(snapshot.phase, PopoverPhase::StorageFull);
        assert!(snapshot.storage_full);
        assert_eq!(snapshot.pending_changes, 2);
        assert_eq!(
            snapshot.conflicts.files,
            vec![ConflictFile {
                file_id: "c1".into(),
                file_name: "plan.txt".into()
            }]
        );
        let rows: Vec<(&str, ActivityState)> = snapshot.activity.iter().map(|r| (r.id.as_str(), r.state)).collect();
        assert_eq!(
            rows,
            vec![
                ("u1", ActivityState::Active),
                ("d1", ActivityState::Active),
                ("e1", ActivityState::Failed),
                ("l1", ActivityState::Done)
            ]
        );
        assert_eq!(
            snapshot.activity[0].done_bytes,
            Some(62),
            "the board's bytes reach the row"
        );
        assert_eq!(
            snapshot.activity[1].done_bytes, None,
            "a download the board does not know has no percentage"
        );
    }

    fn view_of_syncing() -> EngineStatusView {
        view(json!({ "state": "syncing", "files_remaining": 2, "bytes_total": 300, "bytes_done": 62 }))
    }

    #[test]
    fn an_empty_state_db_is_an_empty_snapshot_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::state_db::StateDb::open(dir.path().join("state.db")).unwrap();
        let view = gather_db_view(&db, 8, 100).unwrap();
        assert_eq!(view.backlog, TransferBacklog::default());
        assert!(
            view.conflicts.is_empty() && view.in_flight.is_empty() && view.failed.is_empty() && view.recent.is_empty()
        );
    }
}
