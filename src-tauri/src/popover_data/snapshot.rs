//! The snapshot DTO and the pure assembly behind it: the account, engine, Finder and
//! conflict shapes the popover serializes, the activity rows built from the state
//! DB's view, and `assemble`, which turns what the command gathered into one
//! `PopoverSnapshotDto`.
//!
//! No I/O happens here (the gathering is `popover_snapshot` in `lib.rs`); the Finder
//! reason line is the one piece of text this module makes, and it is data
//! (`reason: timeout`), never prose.

use super::*;

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
    pub update: Option<super::UpdateOffer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sign_out_sending: Option<super::Sending>,
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

pub(super) fn entry_row(
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

pub(super) fn newest_first(entries: &mut [&FileEntry]) {
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
