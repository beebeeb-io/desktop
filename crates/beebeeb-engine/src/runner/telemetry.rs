//! `telemetry` items split out of `runner.rs`.

use super::*;

/// After this many CONSECUTIVE auth (401) failures across a session's
/// heartbeat + sync-tick API calls, [`AuthHealth::expired`] flips true and
/// `sync_status` starts reporting `auth_expired: true` — the trigger for the
/// persistent "You're signed out on this device" banner (task 1546 Codex
/// round 2, finding 5 / lead decision). ANY successful call resets the
/// streak to 0 (and clears `expired`) — a single working request means the
/// session is fine again.
pub(super) const AUTH_EXPIRED_THRESHOLD: u32 = 3;

/// Shared, per-account auth-health tracker (task 1546 Codex round 2, finding
/// 5). Fed by BOTH the heartbeat producer's `post_heartbeat` calls and the
/// main tick loop's `sync_tick` calls — whichever one talks to the server
/// next advances or resets the streak. Lives on `AccountRuntime` (`Arc`'d,
/// same pattern as `sync_paused`) so `sync_status` can read [`Self::is_expired`]
/// without touching the engine task, and is cloned into [`EngineRunner::spawn`]
/// / [`run`] the same way `sync_paused` already is.
#[derive(Default)]
pub struct AuthHealth {
    pub(super) consecutive_failures: AtomicU32,
    pub(super) expired: AtomicBool,
}

impl AuthHealth {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_expired(&self) -> bool {
        self.expired.load(Ordering::Relaxed)
    }

    /// Record the outcome of one API call. `error` is `None` on success,
    /// which always clears the streak and the expired flag — a working
    /// request proves the session is fine again. `Some(e)` on failure
    /// advances the streak only when `e` classifies as an auth (401)
    /// failure via [`classify_operation_error`]; any other error (network
    /// blip, 5xx, …) leaves the streak untouched rather than resetting OR
    /// advancing it, so an unrelated hiccup between two real 401s doesn't
    /// erase the count that matters. `pub`: also called from
    /// `lib.rs`'s `clear_session_impl` to reset the streak on sign-out.
    pub fn note_result(&self, error: Option<&anyhow::Error>) {
        match error {
            None => {
                self.consecutive_failures.store(0, Ordering::Relaxed);
                self.expired.store(false, Ordering::Relaxed);
            }
            Some(e) if matches!(classify_operation_error(&e.to_string()), OperationFailureClass::Auth) => {
                let n = self.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;
                if n >= AUTH_EXPIRED_THRESHOLD {
                    self.expired.store(true, Ordering::Relaxed);
                }
            }
            Some(_) => {}
        }
    }
}

/// Fallback heartbeat cadence (seconds) if the server returns no interval and
/// we send none. The engine asks the server for 20s on session creation (a sane
/// middle ground: live enough for the Bandwidth view, gentle on the API).
pub(super) const DEFAULT_HEARTBEAT_INTERVAL_SECS: i32 = 20;

/// Hard floor so a misconfigured/garbage server interval can never turn the
/// producer into a hot loop hammering the heartbeat endpoint.
pub(super) const MIN_HEARTBEAT_INTERVAL_SECS: i32 = 5;

/// OS family string the server records on the device row. Kept here (not in
/// `lockfile::platform`) so the value matches the read side's `platform`
/// filter exactly (`windows` / `macos` / `linux`).
pub(super) fn platform_str() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "windows"
    }
    #[cfg(target_os = "macos")]
    {
        "macos"
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        "linux"
    }
}

/// This machine's hostname, or `"unknown"` if the OS won't tell us. Cross-
/// platform via the `hostname` crate (already a dependency for the lock file).
///
/// The device row's display name (`register_session`) uses it, and on macOS the
/// backup source tag (spec §2: no macOS change). The Windows and Linux backup
/// uses [`device_name`], which has no fallback, so a lossy name never starts a
/// `Backups/unknown` folder (spec D9, tasks 1888, 2226).
pub fn hostname_or_unknown() -> String {
    hostname::get()
        .ok()
        .and_then(|s| s.into_string().ok())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Spec D9 (tasks 1888, 2226): this machine's name, or `None` when the OS will not give one that is valid Unicode. The
/// backup mirror and the backup source key use this: with `None` they make nothing, so a lossy name never starts a
/// second `Backups/<device>` folder. [`hostname_or_unknown`] stays for the device row's display name and the lock file.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn device_name() -> Option<String> {
    hostname::get().ok().and_then(|s| s.into_string().ok())
}

/// The desktop crate version, surfaced on the device row as `bb_version`.
pub(super) fn bb_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// The latest engine status + the file currently in flight, written by the sync
/// loop and read by the heartbeat producer. Cheap to lock (updated once per tick
/// / read once per beat).
#[derive(Debug, Clone, Default)]
pub(super) struct TelemetryState {
    /// One of the `engine-status` strings the loop emits: `running` / `idle` /
    /// `syncing` / `paused` / `error` / `stopped`.
    pub(super) engine_state: String,
    /// The name of a file actively transferring, if the loop knows one. `None`
    /// when nothing is in flight (the producer then omits `current_file`).
    pub(super) current_file: Option<String>,
}

#[derive(Debug, Default)]
pub(super) struct SyncCompleteNotificationState {
    pub(super) saw_active_sync: bool,
}

impl SyncCompleteNotificationState {
    pub(super) fn observe(&mut self, sync_activity_seen: bool, in_flight_after: u32, unresolved_after: u32) -> bool {
        if sync_activity_seen {
            self.saw_active_sync = true;
        }

        let caught_up = in_flight_after == 0 && unresolved_after == 0;
        if self.saw_active_sync && caught_up {
            self.saw_active_sync = false;
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct QuotaWarningNotificationState {
    pub(super) was_over_threshold: bool,
}

impl QuotaWarningNotificationState {
    pub(super) fn observe(&mut self, local_cache_limit_bytes: Option<i64>, local_cache_bytes: i64) -> bool {
        let over_threshold = local_cache_over_warning_threshold(local_cache_limit_bytes, local_cache_bytes);
        let should_notify = over_threshold && !self.was_over_threshold;
        self.was_over_threshold = over_threshold;
        should_notify
    }
}

/// Map the runner's internal engine-state string to the lowercase heartbeat
/// status the read side (web `devices.tsx`, the desktop Bandwidth view) maps to
/// a display label. The CLI uses the same lowercase vocabulary, so a mixed
/// CLI+desktop account renders consistently.
///
/// - `paused`            → `paused`  (user paused sync)
/// - `syncing`           → `syncing` (files in flight)
/// - `error`             → `error`
/// - `running` / `idle`  → `watching` when the engine is alive and caught up
/// - anything else        → `idle`    (stopped / unknown — a degraded default)
pub(super) fn engine_state_to_heartbeat_status(engine_state: &str, files_in_flight: i64) -> &'static str {
    match engine_state {
        "paused" => "paused",
        "error" => "error",
        // Prefer the live "syncing" signal whenever work is actually in flight,
        // even if the loop's last emitted state was the post-tick "idle".
        _ if files_in_flight > 0 => "syncing",
        "syncing" => "syncing",
        "running" | "idle" => "watching",
        _ => "idle",
    }
}

/// Wall-clock seconds between two `now_secs()` readings, floored at 1. Used as
/// the speed denominator so a delayed/suspended loop reports true speed (bytes
/// over REAL elapsed time) instead of dividing by the nominal interval. The
/// floor guards against divide-by-zero (two beats in one second) and a clock
/// that stepped backwards (returns 1, not a negative).
pub(super) fn beat_elapsed_secs(now: i64, last_beat: i64) -> i64 {
    (now - last_beat).max(1)
}

/// A cheap snapshot of sync progress derived entirely from the state DB — no
/// transfer-loop instrumentation. Counts/bytes are computed from rows already
/// maintained by the sync engine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct SyncSnapshot {
    /// Files fully present locally (`Local` status) — "synced".
    pub(super) files_synced: i64,
    /// All non-folder rows the engine tracks — the denominator.
    pub(super) files_total: i64,
    /// Files actively downloading or uploading right now.
    pub(super) files_in_flight: i64,
    /// Sum of `size_bytes` for `Local` files (bytes that have landed locally).
    pub(super) bytes_synced: i64,
    /// Sum of `size_bytes` across all tracked non-folder files.
    pub(super) bytes_total: i64,
}

/// Snapshot the state DB. Best-effort: any per-status query failure degrades
/// that bucket to zero rather than failing the whole heartbeat — a heartbeat
/// with partial counts is far better than no heartbeat. Folders (`is_dir`) are
/// excluded from counts and byte sums; they carry no transferable payload.
pub(super) fn snapshot_sync_state(db: &StateDb) -> SyncSnapshot {
    let mut snap = SyncSnapshot::default();
    // Every tracked file, for the totals.
    if let Ok(all) = db.list_files() {
        for entry in &all {
            if entry.is_dir() {
                continue;
            }
            snap.files_total += 1;
            snap.bytes_total = snap.bytes_total.saturating_add(entry.size_bytes.max(0));
            if entry.status == FileStatus::Local {
                snap.files_synced += 1;
                snap.bytes_synced = snap.bytes_synced.saturating_add(entry.size_bytes.max(0));
            }
        }
    }
    // In-flight = currently downloading + uploading.
    let in_flight = |status: FileStatus| -> i64 {
        db.list_by_status(status)
            .map(|v| v.iter().filter(|e| !e.is_dir()).count() as i64)
            .unwrap_or(0)
    };
    snap.files_in_flight = in_flight(FileStatus::Downloading) + in_flight(FileStatus::Uploading);
    snap
}

/// Build the heartbeat body for one beat from the engine state + a DB snapshot.
///
/// **Speed source (P1 fix — task 0810):** `speed_bps` is now derived from the
/// *wire-byte counters* drained from the chunk loops, NOT from file-completion
/// deltas (`bytes_synced` diff). The old approach produced `speed_bps = 0`
/// during active transfers because mid-transfer files have status
/// `Downloading`/`Uploading`, not `Local`, so their bytes were excluded from
/// `bytes_synced`. The wire counters count every chunk byte as it passes through
/// the hot path, giving a true real-time transfer speed. `bytes_synced` is
/// retained only for cumulative progress display (files landed / total bytes).
pub(super) fn build_heartbeat(
    telemetry: &TelemetryState,
    snap: SyncSnapshot,
    wire_up: u64,
    wire_down: u64,
    elapsed_secs: i64,
) -> HeartbeatBody {
    let status = engine_state_to_heartbeat_status(&telemetry.engine_state, snap.files_in_flight);

    // True wire speed: total bytes that crossed the network this beat divided
    // by the real elapsed interval. Both counters are reset to 0 by the
    // `drain()` call in the producer loop, so this is a per-beat delta.
    let wire_total = wire_up.saturating_add(wire_down);
    let speed_bps = if elapsed_secs > 0 && wire_total > 0 {
        Some((wire_total / elapsed_secs as u64) as i64)
    } else {
        Some(0)
    };

    // current_file only makes sense while we're actually moving bytes.
    let current_file = if snap.files_in_flight > 0 {
        telemetry.current_file.clone()
    } else {
        None
    };

    HeartbeatBody {
        status: status.to_string(),
        files_synced: Some(snap.files_synced),
        files_total: Some(snap.files_total),
        bytes_synced: Some(snap.bytes_synced),
        bytes_total: Some(snap.bytes_total),
        current_file,
        speed_bps,
        detail: None,
    }
}

/// Normalize a sync-root path so the SAME folder always produces the SAME key
/// the server dedupes on (task 0833). `register_session` runs on every engine
/// spawn (app restart, re-login, reconnect); without a stable key the server
/// would append a new `client_sessions` row each time and the folder would show
/// up multiple times in the device/sync list.
///
/// - **Windows:** the filesystem is case-insensitive and accepts both `\` and
///   `/`, and the same folder can be reached via different drive-letter casing
///   (`C:\X` vs `c:\x`). We canonicalize when possible (resolves `..`/symlinks
///   and yields a consistent form; canonicalize can fail e.g. if the folder is
///   temporarily missing, so we fall back to the raw path), strip the `\\?\`
///   verbatim/UNC prefix `canonicalize` adds, then lowercase and convert `\` to
///   `/` for a stable comparison key. The server can still resolve a normal
///   path from this form.
/// - **Unix:** the filesystem is case-sensitive, so we leave the path as-is
///   (only resolving via canonicalize when it succeeds, to collapse `..` and
///   symlinks; the case is preserved).
pub(super) fn normalize_sync_path(sync_root: &Path) -> String {
    let resolved = std::fs::canonicalize(sync_root).unwrap_or_else(|_| sync_root.to_path_buf());

    #[cfg(windows)]
    {
        let s = resolved.to_string_lossy();
        // Strip the verbatim/UNC prefix std::fs::canonicalize adds on Windows.
        let s = s
            .strip_prefix(r"\\?\UNC\")
            .map(|rest| format!(r"\\{rest}"))
            .unwrap_or_else(|| s.strip_prefix(r"\\?\").unwrap_or(&s).to_string());
        s.replace('\\', "/").to_lowercase()
    }

    #[cfg(not(windows))]
    {
        resolved.to_string_lossy().into_owned()
    }
}

/// Register THIS device + open ONE sync session, best-effort.
///
/// Idempotent on the device (server UPSERTs on hostname+platform); the session
/// is opened once per engine spawn. Returns `Some((session_id, interval))` on
/// success. **A failure here must NOT break login / the engine** — every error
/// path logs and returns `None`, and the caller simply runs the engine without
/// a heartbeat producer. The Bandwidth view degrades to "no live session" — the
/// sync engine itself is unaffected.
pub(super) async fn register_session(api: &ApiClient, sync_root: &Path) -> Option<(String, i32)> {
    let hostname = hostname_or_unknown();
    let device = match api.register_device(&hostname, platform_str(), bb_version()).await {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(error = %e, "client device registration failed; running engine without heartbeat telemetry");
            return None;
        }
    };

    let local_path = normalize_sync_path(sync_root);
    let session = match api
        .create_session(
            &device.id,
            "Beebeeb Desktop",
            "sync",
            Some(&local_path),
            "/",
            DEFAULT_HEARTBEAT_INTERVAL_SECS,
        )
        .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, device_id = %device.id, "client sync-session creation failed; running engine without heartbeat telemetry");
            return None;
        }
    };

    // Honour the server's echoed interval (it may clamp/override ours), floored
    // so a bad value can't spin the producer.
    let interval = session
        .heartbeat_interval_secs
        .unwrap_or(DEFAULT_HEARTBEAT_INTERVAL_SECS)
        .max(MIN_HEARTBEAT_INTERVAL_SECS);

    tracing::info!(session_id = %session.id, interval, "client sync session registered; heartbeat producer starting");
    Some((session.id, interval))
}

/// Spawn the heartbeat producer task. It beats every `interval` seconds until
/// `cancel` fires (the runner loop exiting on logout/lock/shutdown), reading the
/// shared `telemetry` + `sync_paused` and snapshotting `db` each beat. Holds its
/// own `Arc<ApiClient>` + `Arc<StateDb>` clones, which drop with the task on
/// cancel — releasing the session token / DB handle alongside the rest of the
/// engine.
pub(super) fn spawn_heartbeat_producer(
    api: Arc<ApiClient>,
    db: Arc<StateDb>,
    session_id: String,
    interval_secs: i32,
    telemetry: Arc<Mutex<TelemetryState>>,
    sync_paused: Arc<AtomicBool>,
    wire: Arc<WireCounters>,
    auth_health: Arc<AuthHealth>,
    mut cancel: oneshot::Receiver<()>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let interval = Duration::from_secs(interval_secs.max(MIN_HEARTBEAT_INTERVAL_SECS) as u64);
        let mut ticker = tokio::time::interval(interval);
        // Track the WALL-CLOCK time of the last beat so the speed denominator is
        // the real elapsed interval, not the configured one.
        let mut last_beat_secs = now_secs();

        loop {
            tokio::select! {
                biased;
                _ = &mut cancel => break,
                _ = ticker.tick() => {
                    // Snapshot the engine's live telemetry + the DB.
                    let telem = telemetry.lock().map(|g| g.clone()).unwrap_or_default();
                    let snap = snapshot_sync_state(&db);

                    // Actual wall-clock seconds since the previous beat (floored
                    // at 1) — the real speed denominator, not the nominal interval.
                    let now = now_secs();
                    let elapsed_secs = beat_elapsed_secs(now, last_beat_secs);

                    // Drain the wire counters ONCE per beat (swap to 0) so the
                    // values are the delta since the last beat, not a cumulative total.
                    let (wire_up, wire_down) = wire.drain();

                    // P3 — persist the bandwidth sample to state.db for the 20h chart.
                    // Best-effort: a write failure is not fatal; the chart may just
                    // have a gap for that beat. `elapsed_secs` is the actual beat
                    // period (real clock, not nominal), stored as the denominator so
                    // the read side can reconstruct the rate.
                    if let Err(e) = db.insert_bandwidth_sample(now, wire_up, wire_down, elapsed_secs as u32) {
                        tracing::debug!(error = %e, "bandwidth_samples write failed; beat will be missing from chart");
                    }
                    // Prune samples older than ~25h (25 * 3600) to bound DB growth.
                    let _ = db.prune_bandwidth_samples(now - 25 * 3600);

                    // Respect pause/lock: never report "syncing" while paused —
                    // overwrite the mapped status with a flat "paused" beat so
                    // the read side shows the user's intent, not stale activity.
                    let body = if sync_paused.load(Ordering::Relaxed) {
                        HeartbeatBody {
                            status: "paused".to_string(),
                            files_synced: Some(snap.files_synced),
                            files_total: Some(snap.files_total),
                            bytes_synced: Some(snap.bytes_synced),
                            bytes_total: Some(snap.bytes_total),
                            current_file: None,
                            speed_bps: Some(0),
                            detail: None,
                        }
                    } else {
                        build_heartbeat(&telem, snap, wire_up, wire_down, elapsed_secs)
                    };

                    last_beat_secs = now;

                    match api.post_heartbeat(&session_id, &body).await {
                        Ok(()) => auth_health.note_result(None),
                        Err(e) => {
                            // Fire-and-forget: a missed beat is non-fatal (the next
                            // beat refreshes the row). Network blips + token-rotation
                            // 401s are expected; log at debug to avoid noise.
                            tracing::debug!(error = %e, "heartbeat post failed; will retry next beat");
                            auth_health.note_result(Some(&e));
                        }
                    }
                }
            }
        }
        tracing::debug!(session_id = %session_id, "heartbeat producer stopped");
    })
}
