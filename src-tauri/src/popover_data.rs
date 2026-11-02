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
//! ## Who reads it, and how often
//!
//! Nothing here runs on a timer. The popover calls the snapshot when it is shown
//! (`popover-shown`) and after its own actions; the main window (task 1683 slice 1)
//! reads it a few seconds after each read while it is open. The storage summary is
//! fetched only when the cached one is older than [`STORAGE_TTL_SECS`] (or from
//! another session), and a snapshot never waits for that fetch longer than
//! [`STORAGE_WAIT`] (task 2179): a slow API answers from the cache while the fetch
//! finishes in the background, and a failed fetch is not tried again for
//! [`STORAGE_FAILURE_BACKOFF_SECS`].
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
/// Emitted by the window code (`macos_popover.rs`). The constant lives here so the
/// contract has one home.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const POPOVER_SHOWN_EVENT: &str = "popover-shown";

/// Event Rust emits each time the popover is hidden; the UI stops reading its
/// snapshot until it is shown again (review P2-2 of #160: the window is hidden,
/// never destroyed).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const POPOVER_HIDDEN_EVENT: &str = "popover-hidden";

/// How long a storage summary is trusted (spec section 9: "5 minutes, refetched
/// after unlock; never fetched on every open").
pub const STORAGE_TTL_SECS: i64 = 300;

/// The longest a snapshot waits for a storage fetch before it answers from the cache
/// (task 2179). The fetch's own timeout is 8 s and the main window reads a snapshot every
/// few seconds, so a slow API must not hold every read for 8 s.
pub const STORAGE_WAIT: std::time::Duration = std::time::Duration::from_secs(1);

/// After a failed fetch, snapshots answer from the cache for this long without asking the
/// API again (task 2179): an API that is down is not asked on every read.
pub const STORAGE_FAILURE_BACKOFF_SECS: i64 = 30;

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
}

impl Default for PopoverRuntime {
    fn default() -> Self {
        Self {
            transfers: TransferBoard::new(),
            status: Arc::new(Mutex::new(EngineStatusView::default())),
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
    /// The fetch in flight: a snapshot meanwhile waits for it (at most [`STORAGE_WAIT`]) instead of
    /// starting another, and only its answer may land (#139 review P2-3).
    inflight: Option<Inflight>,
    /// When a fetch for a session last failed (when its failure came, #139 review P3-9).
    failed_at: Option<(u64, i64)>,
    /// Numbers the fetches. Kept across [`StorageCache::clear`], so a fetch from before a clear
    /// never holds the ticket of one after it.
    next_ticket: u64,
}

#[derive(Debug)]
struct Inflight {
    ticket: u64,
    vault_epoch: u64,
    done: tokio::sync::watch::Receiver<bool>,
}

impl StorageCache {
    /// Forget the summary, the failure and the fetch in flight, whose answer then lands nowhere: what
    /// sign-out leaves of the account before (task 1887's rule; #139 review P2-3).
    pub fn clear(&mut self) {
        *self = Self {
            next_ticket: self.next_ticket,
            ..Self::default()
        };
    }

    /// Nothing cached, failed or in flight (what a sign-out leaves).
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.entry.is_none() && self.failed_at.is_none() && self.inflight.is_none()
    }

    /// `ticket` is the fetch in flight: a newer session's fetch or a clear has not taken its place.
    fn holds(&self, ticket: u64) -> bool {
        self.inflight.as_ref().is_some_and(|inflight| inflight.ticket == ticket)
    }

    /// A fetch for this session failed less than [`STORAGE_FAILURE_BACKOFF_SECS`] ago.
    fn recently_failed(&self, now: i64, vault_epoch: u64) -> bool {
        self.failed_at
            .is_some_and(|(epoch, at)| epoch == vault_epoch && now.saturating_sub(at) < STORAGE_FAILURE_BACKOFF_SECS)
    }

    /// The best summary to show without fetching: the fresh one, else the last known one marked stale.
    fn answer(&self, now: i64, vault_epoch: u64) -> Option<StorageDto> {
        if let Some((usage, fetched_at)) = self.fresh(now, vault_epoch) {
            return Some(StorageDto::of(usage, fetched_at, false));
        }
        self.last_known(vault_epoch)
            .map(|(usage, fetched_at)| StorageDto::of(usage, fetched_at, true))
    }

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

/// [`StorageCache::clear`] through the shared handle, poisoned or not: a sign-out never fails on it.
pub fn clear_storage(cache: &Mutex<StorageCache>) {
    cache.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clear();
}

/// One fetch's turn, ended however its task ends (an answer, a panic, a cancelled runtime): the slot
/// is freed if it is still this fetch's, and every snapshot waiting on it is woken (#139 review P3-8).
struct FetchTurn {
    cache: Arc<Mutex<StorageCache>>,
    ticket: u64,
    done: tokio::sync::watch::Sender<bool>,
}

impl Drop for FetchTurn {
    fn drop(&mut self) {
        let mut guard = self.cache.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if guard.holds(self.ticket) {
            guard.inflight = None;
        }
        drop(guard);
        let _ = self.done.send(true);
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

impl StorageDto {
    fn of(usage: StorageUsage, fetched_at: i64, stale: bool) -> Self {
        StorageDto {
            used_bytes: usage.used_bytes,
            quota_bytes: usage.quota_bytes,
            fetched_at,
            stale,
        }
    }
}

/// The storage summary for a snapshot: the cached one while it is fresh; otherwise what a
/// fetch returns within [`STORAGE_WAIT`]; otherwise an older cached one marked stale;
/// otherwise nothing (the header then shows the email only).
///
/// The fetch runs as a task of its own (one per session at a time) and fills the cache
/// when it finishes, so a fetch slower than the wait still serves the next snapshot. Only
/// the fetch still in flight writes: one that a newer session's fetch or a sign-out's
/// clear replaced answers into nothing. A failed fetch is not started again for
/// [`STORAGE_FAILURE_BACKOFF_SECS`] after its failure came. No lock is held across an await.
pub async fn cached_storage<F, Fut>(cache: &Arc<Mutex<StorageCache>>, now: i64, vault_epoch: u64, fetch: F) -> Option<StorageDto>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<StorageUsage, String>> + Send + 'static,
{
    cached_storage_with(cache, now, vault_epoch, STORAGE_WAIT, fetch).await
}

/// [`cached_storage`] with the wait as an argument (#139 review P3-11: tests pass their own).
pub async fn cached_storage_with<F, Fut>(
    cache: &Arc<Mutex<StorageCache>>,
    now: i64,
    vault_epoch: u64,
    wait: std::time::Duration,
    fetch: F,
) -> Option<StorageDto>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<StorageUsage, String>> + Send + 'static,
{
    let mut finished = {
        let mut guard = cache.lock().ok()?;
        if guard.fresh(now, vault_epoch).is_some() || guard.recently_failed(now, vault_epoch) {
            return guard.answer(now, vault_epoch);
        }
        match &guard.inflight {
            Some(inflight) if inflight.vault_epoch == vault_epoch => inflight.done.clone(),
            _ => {
                let (done, running) = tokio::sync::watch::channel(false);
                let ticket = guard.next_ticket;
                guard.next_ticket = ticket.wrapping_add(1);
                guard.inflight = Some(Inflight {
                    ticket,
                    vault_epoch,
                    done: running.clone(),
                });
                let pending = fetch();
                let turn = FetchTurn {
                    cache: Arc::clone(cache),
                    ticket,
                    done,
                };
                let started = tokio::time::Instant::now();
                tokio::spawn(async move {
                    let result = pending.await;
                    // When the answer came, on the clock `now` is on (a test's paused clock too).
                    let at = now.saturating_add(i64::try_from(started.elapsed().as_secs()).unwrap_or(i64::MAX));
                    if let Ok(mut guard) = turn.cache.lock()
                        && guard.holds(turn.ticket)
                    {
                        match result {
                            Ok(usage) => {
                                guard.put(usage, now, vault_epoch);
                                guard.failed_at = None;
                            }
                            Err(error) => {
                                tracing::debug!(error = %error, "storage summary refetch failed; using the last known one if any");
                                guard.failed_at = Some((vault_epoch, at));
                            }
                        }
                    }
                    drop(turn);
                });
                running
            }
        }
    };
    // Wait for the fetch, briefly: a slower one finishes in the background either way.
    let _ = tokio::time::timeout(wait, async {
        while !*finished.borrow_and_update() {
            if finished.changed().await.is_err() {
                break;
            }
        }
    })
    .await;
    cache.lock().ok()?.answer(now, vault_epoch)
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
    /// `phase` with the Finder setup taken as ready: the sync part of the state alone (task 2179). The main window's
    /// Home row shows it beside its own Finder row; the one precedence stays here, in `popover_phase`.
    pub sync_phase: PopoverPhase,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update: Option<crate::macos_popover::UpdateOffer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sign_out_sending: Option<crate::macos_popover::Sending>,
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

    let reducer_input = PopoverSnapshot {
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
    };
    let phase = popover_phase(&reducer_input);
    let sync_phase = popover_phase(&PopoverSnapshot {
        finder: FinderSetup::Ready,
        finder_reason: None,
        finder_failure_elsewhere: false,
        ..reducer_input
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
        sync_phase,
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
        update: None,
        sign_out_sending: None,
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
mod tests;
