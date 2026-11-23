//! `tick_loop` items split out of `runner.rs`.

use super::*;

/// Whether the runner is doing sync work or the person paused it (spec §3.4: a named mode instead of
/// a bare `paused` boolean). The shared flag stays an `AtomicBool` because `lib.rs`'s `AppState`
/// flips it; this is its meaning at every place the loop branches on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    Running,
    Paused,
}

impl RunMode {
    pub fn of_flag(paused: bool) -> Self {
        if paused { Self::Paused } else { Self::Running }
    }

    pub fn is_paused(self) -> bool {
        self == Self::Paused
    }
}

/// The tick's power/network hold (task dc-031). `None` while the person paused sync (that pause says it
/// all). Also records the hold for the status pulse.
pub(super) fn power_hold(identity: &'static crate::identity::EngineIdentity, user_paused: bool) -> Option<crate::power::PausedFor> {
    let hold = if user_paused {
        None
    } else {
        let metered = crate::config::DesktopConfig::load_for(identity).ok().and_then(|c| c.metered);
        crate::power::hold_now(metered, &crate::power::SystemPower)
    };
    crate::power::set_current(hold);
    hold
}

/// The `paused` status, with the reason in words when a power hold caused it.
pub(super) fn paused_payload(tracker: &StatusTracker, hold: Option<crate::power::PausedFor>) -> crate::engine_status::StatusPayload {
    let mut p = tracker.paused();
    if let Some(h) = hold {
        p.reason = Some(crate::engine_status::Reason {
            code: h.code(),
            detail: Some(h.words().to_string()),
        });
    }
    p
}

/// How often the runner pulls the file list from the server and
/// refreshes the state DB. The previous 5s cadence re-walked the WHOLE
/// remote tree (`GET /api/v1/files`) every tick, which by itself could
/// saturate the server's per-IP rate limit and pin the client in a 429
/// loop. 30s is a stopgap to stop the bleeding; once the `/sync`
/// snapshot+ops delta path lands (task 0789) refresh becomes
/// event-driven and this fixed poll goes away.
pub(super) const TICK_INTERVAL: Duration = Duration::from_secs(30);

/// Task 2521: how long a paused tick waits for its check of the session (`ApiClient::session_check`). Well under the
/// runner stop's graceful wait (`GRACEFUL_ABORT_TIMEOUT`, 3 s), so a stop that comes during the check still lands
/// inside it.
pub(super) const PAUSED_SESSION_CHECK_TIMEOUT: Duration = Duration::from_secs(2);

pub(super) const LOCAL_CACHE_WARNING_THRESHOLD_NUMERATOR: i128 = 9;

pub(super) const LOCAL_CACHE_WARNING_THRESHOLD_DENOMINATOR: i128 = 10;

/// Task 1670 round 3: cadence for the macOS
/// hydrate-cache TTL sweep. With `TICK_INTERVAL` at 30s, every 2 ticks is
/// ~60s — the "every Nth tick of the daemon's one already-running loop" idiom,
/// reusing the existing runtime rather than spawning a second timer task.
///
/// Why a periodic sweep exists at all: round 3 made `FileProviderExtension
/// .fetchContents` (Swift) stop deleting its staged file on the success
/// path and relied on this sweep to bound its lifetime instead — but round 4
/// (Codex P1 on PR #75) found that was still handing OUR staging URL to the
/// system and cleaning it up on a timer, which can race a busy/suspended
/// `fileproviderd`. `fetchContents` now copies into the SYSTEM's own
/// `NSFileProviderManager.temporaryDirectoryURL()` and deletes OUR copy
/// immediately after that copy succeeds, so this sweep — applying
/// `crate::ipc_socket::MACOS_HYDRATE_CACHE_TTL` (see its doc comment for the
/// current 2-minute value and why) — is a **crash backstop**, not the
/// primary bound: on the no-crash path, nothing is ever left here long
/// enough for the sweep to matter. It still runs unconditionally, catching
/// staging orphaned by a crash between decrypt and that copy.
#[cfg(target_os = "macos")]
pub(super) const MACOS_HYDRATE_SWEEP_EVERY_N_TICKS: u64 = 2;

/// Upload-staging purge cadence: every 10 ticks (5 minutes at the 30 s
/// tick). The age bound it applies is an hour
/// (`crate::ipc_socket::UPLOAD_STAGING_MAX_AGE`), so a faster cadence would
/// buy nothing. The counter starts at 1 because startup already purged.
#[cfg(target_os = "macos")]
pub(super) const MACOS_UPLOAD_STAGING_SWEEP_EVERY_N_TICKS: u64 = 10;

pub(super) fn local_cache_over_warning_threshold(local_cache_limit_bytes: Option<i64>, local_cache_bytes: i64) -> bool {
    let Some(limit) = local_cache_limit_bytes else {
        return false;
    };
    if limit <= 0 {
        return false;
    }

    let usage = local_cache_bytes.max(0) as i128;
    let limit = limit as i128;
    usage.saturating_mul(LOCAL_CACHE_WARNING_THRESHOLD_DENOMINATOR)
        >= limit.saturating_mul(LOCAL_CACHE_WARNING_THRESHOLD_NUMERATOR)
}

/// Emit an `engine-status` event the WebView + tray listen to.
/// Best-effort: a missing main window or serialisation issue is
/// logged but doesn't break the runner.
pub(super) fn emit_status(app: &Host, state: &str, sync_root: Option<&PathBuf>, error: Option<&str>) {
    emit_payload(
        app,
        crate::engine_status::plain_status(state, sync_root.map(|p| p.to_string_lossy().into_owned()), error),
    );
}

/// The one place an `engine-status` event leaves the engine: the status with the start-up guard's progress.
pub(super) fn emit_payload(app: &Host, status: crate::engine_status::StatusPayload) {
    app.event(EngineEvent::Status(with_startup_guard(status)));
}

/// The work in flight right now: queue and downloading rows from the state DB,
/// bytes from the transfer board.
pub(super) fn current_activity(db: &StateDb, transfers: &crate::transfer_progress::TransferBoard) -> Activity {
    compute_activity(&db.transfer_backlog(now_secs()).unwrap_or_default(), transfers)
}

/// What a finished tick means, as the event to emit.
pub(super) fn finish_tick_payload(
    tracker: &StatusTracker,
    result: Result<(), &anyhow::Error>,
    link: &crate::link_health::LinkMonitor,
    tick_started_at: i64,
    db: &StateDb,
    transfers: &crate::transfer_progress::TransferBoard,
) -> crate::engine_status::StatusPayload {
    let outcome = tick_outcome(result, &link.snapshot(), tick_started_at);
    let activity = current_activity(db, transfers);
    let payload = tracker.finish_tick(&outcome, now_secs(), activity);
    if activity.files_remaining == 0 {
        // Nothing left: the batch is over, the next burst starts its bar at 0.
        transfers.reset_batch();
    }
    payload
}

/// w31: has the account's file list never arrived in this local state? True only when the cursor reads as unset: a
/// read that fails changes nothing (the tick's own read of it reports that).
pub(super) fn first_list_owed(db: &StateDb) -> bool {
    matches!(db.get_sync_cursor(), Ok(None))
}

/// w31: what the board says of the first list after a tick's pull. `still_owed`: the cursor is still unset.
/// `reached_server`: the pull came back. A pull that did not, with no list yet, is a failure a window must say: its
/// rows are not the account's files.
pub(super) fn first_list_after_pull(still_owed: bool, reached_server: bool) -> crate::transfer_progress::FirstList {
    use crate::transfer_progress::FirstList;
    match (still_owed, reached_server) {
        (false, _) => FirstList::Here,
        (true, true) => FirstList::Awaited,
        (true, false) => FirstList::Failed,
    }
}

/// How often the pulse looks. The look itself is free while nothing is moving.
pub(super) const STATUS_PULSE_INTERVAL: Duration = Duration::from_secs(1);

/// Reports `syncing` (with progress) while files move, and the way back to `idle`.
/// Runs for the engine's lifetime and is aborted with it.
pub(super) async fn run_status_pulse(
    app: Host,
    tracker: Arc<StatusTracker>,
    db: Arc<StateDb>,
    transfers: Arc<crate::transfer_progress::TransferBoard>,
    sync_paused: Arc<AtomicBool>,
) {
    let mut ticker = tokio::time::interval(STATUS_PULSE_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;
        let paused = sync_paused.load(Ordering::Relaxed) || crate::power::current().is_some();
        let payload = status_pulse_once(&tracker, &db, &transfers, paused);
        if let Some(payload) = payload {
            let going_quiet = payload.state == crate::engine_status::State::Idle;
            emit_payload(&app, payload);
            if going_quiet {
                // The batch is over: the next burst of work starts its bar at 0.
                transfers.reset_batch();
            }
        }
    }
}

/// One look of the pulse: the event to emit, if the picture changed.
pub(super) fn status_pulse_once(
    tracker: &StatusTracker,
    db: &StateDb,
    transfers: &crate::transfer_progress::TransferBoard,
    paused: bool,
) -> Option<crate::engine_status::StatusPayload> {
    let transfer_in_flight = !transfers.active().is_empty();
    tracker.pulse(paused, transfer_in_flight, || current_activity(db, transfers))
}

/// Publish the loop's current high-level engine state to the shared telemetry
/// slot the heartbeat producer reads. Best-effort: a poisoned mutex is swallowed
/// (a stale heartbeat status is harmless — the next tick re-publishes).
pub(super) fn set_telemetry_state(telemetry: &Arc<Mutex<TelemetryState>>, state: &str) {
    if let Ok(mut guard) = telemetry.lock() {
        guard.engine_state = state.to_string();
    }
}

/// Refresh the Windows Explorer breadcrumb status-flyout snapshot with the live
/// sync state, derived from the SAME counts `sync_status` exposes (downloading +
/// uploading ⇒ syncing; conflict rows ⇒ error). The COM `GetStatusUI` reads only
/// this cached snapshot (cheap atomics), so it never opens the DB or blocks the
/// shell. Counting reuses the `db` the loop already holds, so this adds one
/// indexed count query per tick. No-op on non-Windows.
pub(super) fn file_status_count(db: &StateDb, status: FileStatus) -> u32 {
    db.count_by_status(status).unwrap_or(0)
}

pub(super) fn sync_in_flight_count(db: &StateDb) -> u32 {
    file_status_count(db, FileStatus::Downloading).saturating_add(file_status_count(db, FileStatus::Uploading))
}

pub(super) fn unresolved_sync_issue_count(db: &StateDb) -> u32 {
    file_status_count(db, FileStatus::Conflict).saturating_add(file_status_count(db, FileStatus::Error))
}

#[cfg(target_os = "windows")]
pub(super) fn refresh_status_ui_snapshot(db: &StateDb, mode: RunMode, engine_error: bool) {
    let syncing = sync_in_flight_count(db);
    let conflicts = file_status_count(db, FileStatus::Conflict);
    crate::windows_cf::status_ui::set_engine_state(mode.is_paused(), syncing, conflicts, engine_error);
}

pub(super) fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub(super) fn enforce_cache_budget(
    app: &Host,
    bridge: &Arc<EngineBridge>,
    quota_warning_notifications: &mut QuotaWarningNotificationState,
) {
    // Honor the Windows "Files On-Demand" toggle. When the user has turned
    // it OFF (`Some(false)`), they want every file kept fully local — so we
    // must NOT evict unpinned cached files down to the budget. Unset
    // (`None`) or `Some(true)` keeps the default on-demand behaviour.
    // A missing/unreadable config defaults to on-demand (the safe, low-disk
    // behaviour). This is one cheap TOML read per 5s tick.
    let cfg = crate::config::DesktopConfig::load_for(bridge.identity).unwrap_or_default();
    let local_cache_limit = cfg.local_cache_limit_for_eviction();
    match local_cache_limit {
        Some(limit) => match bridge.local_cache_usage_bytes() {
            Ok(usage) if quota_warning_notifications.observe(Some(limit), usage) => {
                if let Err(e) = app.notify(Notice::QuotaWarning {
                    usage_bytes: usage,
                    limit_bytes: limit,
                }) {
                    tracing::warn!(error = %e, "notify_quota_warning failed");
                }
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "local cache usage check failed"),
        },
        None => {
            quota_warning_notifications.observe(None, 0);
        }
    }

    let files_on_demand = cfg.files_on_demand.unwrap_or(true);
    if !files_on_demand {
        // User opted into keeping everything local — skip eviction.
        return;
    }

    match bridge.enforce_local_cache_limit(local_cache_limit) {
        Ok(outcome) if !outcome.evicted_file_ids.is_empty() => {
            tracing::info!(
                evicted = outcome.evicted_file_ids.len(),
                "smart cache cleanup evicted unpinned files"
            );
        }
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "smart cache cleanup failed"),
    }
}

/// Surface a freshly-detected conflict to the user: open the
/// resolution window, fire a native notification, emit a Tauri event
/// so any open settings page can refresh its conflict counter without
/// waiting for the next 5 s tick. All three are best-effort — a failure
/// in any one path is logged and the others still run, since the
/// underlying state DB has already been updated and the conflict won't
/// be silently dropped.
pub(super) fn handle_new_conflict(app: &Host, c: &ConflictDetected) {
    // `conflict_auto_opens_window` is true on every platform today (task 1683
    // slice 1 pins that for Windows and Linux); slice 6 makes macOS false.
    let notice = ConflictNotice {
        file_id: c.file_id.clone(),
        file_name: c.file_name.clone(),
        is_text: c.is_text,
    };
    if app.conflict_opens_window()
        && let Err(e) = app.open_conflict(&notice) {
            tracing::warn!(error = %e, file_id = %c.file_id, "open_conflict_window failed");
        }
    if let Err(e) = app.notify(Notice::Conflict {
        file_name: c.file_name.clone(),
    }) {
        // Don't escalate — Linux without a notification daemon, or
        // macOS where the user denied the permission prompt, will
        // both fail here. The window + event still fired.
        tracing::warn!(error = %e, file_id = %c.file_id, "notify_conflict failed");
    }
    app.event(EngineEvent::Conflict(notice));
}

/// Walk every file currently in `Conflict` status. For any whose
/// detection timestamp (stored in `modified_at` after Task 10 anchored
/// it on detect) is past the 24 h deadline, apply Keep Both: the local
/// copy gets a `(conflict - <hostname> - <date>)` suffix, the remote
/// becomes the new authoritative file, status flips back to `Local`.
///
/// Errors are logged per-file and do not stop the sweep — one bad file
/// shouldn't keep the rest from being resolved. The bridge's
/// [`crate::engine_bridge::EngineBridge::auto_resolve_keep_both`] is
/// engineered so that a partial failure leaves the local copy on disk
/// (renamed) plus the row in `Error` status — the user never loses
/// data, the next tick retries the remote hydrate.
pub(super) async fn sweep_auto_resolutions(app: &Host, bridge: &Arc<EngineBridge>, sync_root: &Path) {
    let conflicts = match bridge.db().list_by_status(FileStatus::Conflict) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "list_by_status(Conflict) failed");
            return;
        }
    };
    if conflicts.is_empty() {
        return;
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    for entry in conflicts {
        // `modified_at` was anchored to the detection time when the
        // tick flipped this row to Conflict. A 0 here would mean
        // "detected at epoch" which is also fine (immediate auto-
        // resolve on launch is not a real scenario, since the row
        // had to be written by an earlier sync_tick).
        let detected = entry.modified_at as u64;
        if now < auto_resolution_deadline(detected) {
            continue;
        }

        tracing::info!(
            file_id = %entry.file_id,
            path = %entry.path,
            elapsed_secs = now.saturating_sub(detected),
            "auto-resolving conflict (24h elapsed) via Keep Both"
        );
        match bridge.auto_resolve_keep_both(sync_root, &entry).await {
            Ok(conflict_copy_name) => {
                app.event(EngineEvent::ConflictAutoResolved(ConflictAutoResolved {
                    file_id: entry.file_id.clone(),
                    file_name: entry.path.clone(),
                    conflict_copy_name,
                }));
            }
            Err(e) => {
                tracing::warn!(
                    file_id = %entry.file_id,
                    error = %e,
                    "auto_resolve_keep_both failed — file kept on disk; will retry next tick"
                );
            }
        }
    }
}

// Re-export Path for parity with the previous file's signature surface.
// `_path` parameter type kept for forward-compat with the eventual
// per-file emit calls from engine_bridge.rs.
#[allow(dead_code)]
pub(super) fn _unused_path(_p: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    /// The old `paused` branches, one row per boolean: the mode says the same thing.
    #[test]
    fn run_mode_maps_the_pause_flag_table_driven() {
        for (paused, mode) in [(false, RunMode::Running), (true, RunMode::Paused)] {
            assert_eq!(RunMode::of_flag(paused), mode, "paused = {paused}");
            assert_eq!(mode.is_paused(), paused, "paused = {paused}");
        }
    }

    /// w31 (the Windows test machine, 2026-11-20): a local state that never got the account's file list (no cursor) owes it, also
    /// when this PC's own sync folder already gave it folder rows; the backup passes wait for it and the board says
    /// it is awaited. A cursor of 0 is a list that arrived (an account with an empty op log), not an owed one.
    #[test]
    fn w31_a_state_with_no_cursor_owes_the_first_list_and_one_with_a_cursor_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        assert!(first_list_owed(&db), "never listed: owed");
        db.set_sync_cursor(0).unwrap();
        assert!(!first_list_owed(&db), "a cursor of 0 is a list that arrived");
        db.set_sync_cursor(177_670).unwrap();
        assert!(!first_list_owed(&db));
    }

    /// w31: after a pull, a list that is still owed is awaited while the server answers and failed when it did not;
    /// once the cursor is there it is here, whatever that pull said.
    #[test]
    fn w31_the_board_says_awaited_failed_or_here_after_a_pull() {
        use crate::transfer_progress::FirstList;
        assert_eq!(first_list_after_pull(true, true), FirstList::Awaited);
        assert_eq!(first_list_after_pull(true, false), FirstList::Failed);
        assert_eq!(first_list_after_pull(false, true), FirstList::Here);
        assert_eq!(first_list_after_pull(false, false), FirstList::Here);
    }
}
