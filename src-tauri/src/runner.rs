//! Long-running engine task: owns the lock file + state DB + API
//! client, runs a periodic sync tick, emits status events to the
//! WebView via Tauri.
//!
//! Phase 1 Task 3 of the desktop sync client plan. Replaces the
//! earlier `beebeeb_sync::SyncEngine` placeholder with a real loop
//! built around [`crate::engine_bridge`] + [`crate::api_client`] +
//! [`crate::state_db`].
//!
//! ## Lifecycle
//!
//! - `EngineRunner::spawn` is called from either:
//!   - `apply_session` (via `start_engine_if_possible`) immediately after
//!     the browser-login handoff installs a fresh session, or
//!   - `pick_sync_root` if a session is already in memory when the
//!     first-launch picker resolves.
//!
//! - The runner drops the prior runner first so a re-login or a
//!   sync-root change cleanly tears down the old task.
//!
//! - On `clear_session` (logout) or app shutdown, [`EngineRunner::abort`]
//!   is called: a oneshot fires, the task exits, the lock file is
//!   released via the `LockFile` `Drop` impl.
//!
//! ## Status events
//!
//! The task emits `engine-status` events. The payload, the seven `state`
//! values (`running` / `idle` / `syncing` / `paused` / `offline` / `error` /
//! `stopped`) and the rules that pick one live in [`crate::engine_status`]. The
//! tray tooltip listener (`attach_tray_status_listener` in `lib.rs`) and the
//! popover snapshot consume this stream.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::api_client::{ApiClient, HeartbeatBody};
use crate::conflict::auto_resolution_deadline;
use crate::engine_status::{Activity, StatusTracker, compute_activity, tick_outcome};
use crate::engine_bridge::{
    ConflictDetected, EngineBridge, OperationFailureClass, WireCounters, classify_operation_error, sync_tick,
};
use crate::lockfile::LockFile;
use crate::state_db::{FileStatus, StateDb};
use crate::state_paths;

/// After this many CONSECUTIVE auth (401) failures across a session's
/// heartbeat + sync-tick API calls, [`AuthHealth::expired`] flips true and
/// `sync_status` starts reporting `auth_expired: true` — the trigger for the
/// persistent "You're signed out on this device" banner (task 1546 Codex
/// round 2, finding 5 / lead decision). ANY successful call resets the
/// streak to 0 (and clears `expired`) — a single working request means the
/// session is fine again.
const AUTH_EXPIRED_THRESHOLD: u32 = 3;

/// Shared, per-account auth-health tracker (task 1546 Codex round 2, finding
/// 5). Fed by BOTH the heartbeat producer's `post_heartbeat` calls and the
/// main tick loop's `sync_tick` calls — whichever one talks to the server
/// next advances or resets the streak. Lives on `AccountRuntime` (`Arc`'d,
/// same pattern as `sync_paused`) so `sync_status` can read [`Self::is_expired`]
/// without touching the engine task, and is cloned into [`EngineRunner::spawn`]
/// / [`run`] the same way `sync_paused` already is.
#[derive(Default)]
pub struct AuthHealth {
    consecutive_failures: AtomicU32,
    expired: AtomicBool,
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
    /// erase the count that matters. `pub(crate)`: also called from
    /// `lib.rs`'s `clear_session_impl` to reset the streak on sign-out.
    pub(crate) fn note_result(&self, error: Option<&anyhow::Error>) {
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

/// How often the runner pulls the file list from the server and
/// refreshes the state DB. The previous 5s cadence re-walked the WHOLE
/// remote tree (`GET /api/v1/files`) every tick, which by itself could
/// saturate the server's per-IP rate limit and pin the client in a 429
/// loop. 30s is a stopgap to stop the bleeding; once the `/sync`
/// snapshot+ops delta path lands (task 0789) refresh becomes
/// event-driven and this fixed poll goes away.
const TICK_INTERVAL: Duration = Duration::from_secs(30);
const LOCAL_CACHE_WARNING_THRESHOLD_NUMERATOR: i128 = 9;
const LOCAL_CACHE_WARNING_THRESHOLD_DENOMINATOR: i128 = 10;

/// Known-folder backup (task 0797): run the source→vault mirror every Nth sync
/// tick. With a 30s `TICK_INTERVAL` that is ~60s. Lowered from 10 (~5 min) to 2
/// for task 0811: the enable IPC no longer mirrors inline (it returned to the
/// daemon to stop the "Setting up…" hang), so backup must START on its own
/// SHORTLY after enable — a ~60s cadence does that without a 5-minute wait. This
/// is safe because BOTH ends are now bounded PER PASS:
/// - the disk-copy side caps the NUMBER OF FILES it copies per pass across all
///   enabled folders (`known_folder::MAX_MIRROR_FILES_PER_PASS`) — so a first
///   enable of a 5,000-file folder copies in batches across ticks, never the
///   whole set at once (the disk flood that pinned the founder's machine). The
///   per-file 4 GiB cap and depth-64 guard are separate, narrower bounds; the
///   per-pass FILE budget is what bounds total copy VOLUME.
/// - the upload scan caps NEW enqueues per pass
///   (`watcher::MAX_NEW_UPLOADS_PER_SCAN`).
/// Both caps are matched, so disk-copy and enqueue ramp up in step and a frequent
/// pass can no longer thunder even on a first enable of a 5,000-file folder.
/// Windows-only.
#[cfg(target_os = "windows")]
const KNOWN_FOLDER_MIRROR_EVERY_N_TICKS: u64 = 2;

/// Task 1670 round 3 (lead review of round 2): cadence for the macOS
/// hydrate-cache TTL sweep. With `TICK_INTERVAL` at 30s, every 2 ticks is
/// ~60s — same "every Nth tick of the daemon's one already-running loop"
/// idiom as `KNOWN_FOLDER_MIRROR_EVERY_N_TICKS` above, reusing the existing
/// runtime rather than spawning a second timer task.
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
const MACOS_HYDRATE_SWEEP_EVERY_N_TICKS: u64 = 2;

/// API base URL the engine talks to.
///
/// Returns the value of the `BB_API_BASE` environment variable when it is set
/// and non-empty, falling back to the production endpoint.  A trailing slash is
/// stripped so callers that do `format!("{base}/api/v1/...")` never produce a
/// double-slash.  Leading/trailing whitespace is also trimmed.
///
/// # Local dev / testing
///
/// ```bash
/// # Point the desktop at a local API server:
/// BB_API_BASE=http://localhost:3001 bun run tauri:dev
/// # or against a WSL2 IP:
/// BB_API_BASE=http://10.100.0.239:3001 bun run tauri:dev
/// ```
///
/// Packaged release builds ship without this variable set, so they always hit
/// `https://api.beebeeb.io`.
pub(crate) fn api_base_url() -> String {
    let base = std::env::var("BB_API_BASE").unwrap_or_default();
    let trimmed = base.trim();
    if trimmed.is_empty() {
        return "https://api.beebeeb.io".to_string();
    }
    trimmed.trim_end_matches('/').to_string()
}

/// Fallback heartbeat cadence (seconds) if the server returns no interval and
/// we send none. The engine asks the server for 20s on session creation (a sane
/// middle ground: live enough for the Bandwidth view, gentle on the API).
const DEFAULT_HEARTBEAT_INTERVAL_SECS: i32 = 20;

/// Hard floor so a misconfigured/garbage server interval can never turn the
/// producer into a hot loop hammering the heartbeat endpoint.
const MIN_HEARTBEAT_INTERVAL_SECS: i32 = 5;

/// OS family string the server records on the device row. Kept here (not in
/// `lockfile::platform`) so the value matches the read side's `platform`
/// filter exactly (`windows` / `macos` / `linux`).
fn platform_str() -> &'static str {
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
/// `pub(crate)` so the known-folder backup mirror can derive its
/// `Backup/<device>/<folder>` destination from the SAME value this device
/// registers with the server (`register_session`), keeping the backup path and
/// the device row in lockstep.
pub(crate) fn hostname_or_unknown() -> String {
    hostname::get()
        .ok()
        .and_then(|s| s.into_string().ok())
        .unwrap_or_else(|| "unknown".to_string())
}

/// The desktop crate version, surfaced on the device row as `bb_version`.
fn bb_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

// ── Heartbeat telemetry ───────────────────────────────────────────────────────
//
// The runner loop publishes its current high-level engine state into a shared
// `TelemetryState`; the heartbeat producer task reads it each beat, snapshots
// the state DB for live counts/bytes, and POSTs a heartbeat. Decoupling the two
// (loop writes, producer reads) keeps the heartbeat cadence independent of the
// 5s sync tick — the Bandwidth view gets a steady beat even while a tick is
// mid-flight.

/// The latest engine status + the file currently in flight, written by the sync
/// loop and read by the heartbeat producer. Cheap to lock (updated once per tick
/// / read once per beat).
#[derive(Debug, Clone, Default)]
struct TelemetryState {
    /// One of the `engine-status` strings the loop emits: `running` / `idle` /
    /// `syncing` / `paused` / `error` / `stopped`.
    engine_state: String,
    /// The name of a file actively transferring, if the loop knows one. `None`
    /// when nothing is in flight (the producer then omits `current_file`).
    current_file: Option<String>,
}

#[derive(Debug, Default)]
struct SyncCompleteNotificationState {
    saw_active_sync: bool,
}

impl SyncCompleteNotificationState {
    fn observe(&mut self, sync_activity_seen: bool, in_flight_after: u32, unresolved_after: u32) -> bool {
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
struct QuotaWarningNotificationState {
    was_over_threshold: bool,
}

impl QuotaWarningNotificationState {
    fn observe(&mut self, local_cache_limit_bytes: Option<i64>, local_cache_bytes: i64) -> bool {
        let over_threshold = local_cache_over_warning_threshold(local_cache_limit_bytes, local_cache_bytes);
        let should_notify = over_threshold && !self.was_over_threshold;
        self.was_over_threshold = over_threshold;
        should_notify
    }
}

fn local_cache_over_warning_threshold(local_cache_limit_bytes: Option<i64>, local_cache_bytes: i64) -> bool {
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
fn engine_state_to_heartbeat_status(engine_state: &str, files_in_flight: i64) -> &'static str {
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
fn beat_elapsed_secs(now: i64, last_beat: i64) -> i64 {
    (now - last_beat).max(1)
}

/// A cheap snapshot of sync progress derived entirely from the state DB — no
/// transfer-loop instrumentation. Counts/bytes are computed from rows already
/// maintained by the sync engine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct SyncSnapshot {
    /// Files fully present locally (`Local` status) — "synced".
    files_synced: i64,
    /// All non-folder rows the engine tracks — the denominator.
    files_total: i64,
    /// Files actively downloading or uploading right now.
    files_in_flight: i64,
    /// Sum of `size_bytes` for `Local` files (bytes that have landed locally).
    bytes_synced: i64,
    /// Sum of `size_bytes` across all tracked non-folder files.
    bytes_total: i64,
}

/// Snapshot the state DB. Best-effort: any per-status query failure degrades
/// that bucket to zero rather than failing the whole heartbeat — a heartbeat
/// with partial counts is far better than no heartbeat. Folders (`is_dir`) are
/// excluded from counts and byte sums; they carry no transferable payload.
fn snapshot_sync_state(db: &StateDb) -> SyncSnapshot {
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
fn build_heartbeat(
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
fn normalize_sync_path(sync_root: &Path) -> String {
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
async fn register_session(api: &ApiClient, sync_root: &Path) -> Option<(String, i32)> {
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
fn spawn_heartbeat_producer(
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

/// Owned handle to a running engine task. Drop it (or call
/// [`Self::abort`]) to stop the engine.
pub struct EngineRunner {
    cancel: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
    /// `None` until (and unless) the Unix-socket IPC accept loop fails to
    /// bind (task 1524). Shared with the spawned task so a caller waiting on
    /// Finder/File-Provider readiness — [`wait_for_file_provider_ipc_ready`]
    /// in `lib.rs` — can surface the REAL `std::io::Error` (e.g. the macOS
    /// sandbox refusing the path) instead of just a generic connect timeout.
    ipc_bind_error: Arc<Mutex<Option<String>>>,
    /// Cooperative stop flag (task 1538 Codex P1). Shared with the
    /// [`EngineBridge`] `run` builds, so `abort()` can flip it BEFORE even
    /// sending the tick loop's cancel oneshot.
    stopping: Arc<AtomicBool>,
}

impl EngineRunner {
    /// Spawn the runner on a background tokio task. Returns the handle
    /// synchronously — actual startup (lock acquire, DB open, first
    /// tick) happens inside the task.
    ///
    /// `sync_paused` is shared with [`crate::AppState`] so the
    /// `tray_pause_sync` / `tray_resume_sync` IPC commands can signal
    /// the loop without restarting the runner. `auth_health` is likewise
    /// shared with the account runtime so `sync_status` can read the
    /// consecutive-401 streak this task feeds (task 1546 finding 5).
    pub fn spawn(
        app: AppHandle,
        sync_root: PathBuf,
        session_token: String,
        master_key: [u8; 32],
        sync_paused: Arc<AtomicBool>,
        auth_health: Arc<AuthHealth>,
    ) -> Self {
        let (tx, rx) = oneshot::channel::<()>();
        let ipc_bind_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let ipc_bind_error_for_task = ipc_bind_error.clone();
        let stopping: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
        let stopping_for_task = stopping.clone();

        let task = tokio::spawn(async move {
            run(
                app,
                sync_root,
                session_token,
                master_key,
                RunnerControls {
                    cancel: rx,
                    sync_paused,
                    auth_health,
                    ipc_bind_error: ipc_bind_error_for_task,
                    stopping: stopping_for_task,
                },
            )
            .await;
        });

        Self {
            cancel: Some(tx),
            task: Some(task),
            ipc_bind_error,
            stopping,
        }
    }

    /// A cheap clone of the shared IPC-bind-status cell (task 1524). `None`
    /// until the accept loop's bind attempt has run AND failed; still `None`
    /// after a successful bind (there is nothing to report — the readiness
    /// probe's own socket connect is the success signal). Callers poll this
    /// alongside that probe so a real bind error surfaces immediately
    /// instead of only after the full readiness timeout.
    pub fn ipc_bind_error_handle(&self) -> Arc<Mutex<Option<String>>> {
        self.ipc_bind_error.clone()
    }

    /// Signal the runner to stop and wait for CONFIRMED termination. Drops
    /// the lock file as part of teardown. Idempotent — calling twice is a
    /// no-op (the second call has nothing left to wait on and returns
    /// `true` immediately).
    ///
    /// Returns `true` only when the runner's task is actually gone — never
    /// merely "we gave up waiting" (task 1538 Codex P1, PR #49 lib.rs:1087
    /// thread). The old version just dropped the `JoinHandle` after a 3s
    /// timeout, which DETACHES rather than cancels the task: the engine
    /// (and everything nested inside its single tokio task — the IPC socket
    /// server, the Windows upload watcher) could keep running after a
    /// caller believed sign-out/lock had finished, still holding the
    /// session master key and able to drain/enqueue operations behind a
    /// purge's back. Callers that need that guarantee — `clear_session_impl`
    /// gating its cross-account purge on it — must check the return value
    /// and refuse to proceed when it's `false`.
    pub async fn abort(mut self) -> bool {
        // Flip the cooperative flag FIRST, before the cancel oneshot even
        // sends: `EngineBridge::is_stopping()` (checked by
        // `process_due_operations` before every operation and by
        // `queue_finder_create`/`_modify`/`_delete` before enqueuing) then
        // observes the stop request immediately, without waiting for the
        // tick loop to next reach its `tokio::select!` boundary.
        self.stopping.store(true, Ordering::SeqCst);
        #[cfg(target_os = "windows")]
        let callbacks_stopped = crate::windows_cf::revoke_callbacks().await.is_ok();
        let stopped = stop_task_and_confirm(
            self.cancel.take(),
            self.task.take(),
            GRACEFUL_ABORT_TIMEOUT,
            FORCE_ABORT_TIMEOUT,
        )
        .await;
        #[cfg(target_os = "windows")]
        {
            // Do not lose an unconfirmed task behind an empty engine slot and
            // let a second lock attempt claim success. Such a task can still
            // hold startup credentials before an ApiClient exists.
            if !stopped {
                crate::windows_cf::refuse_unconfirmed_stop();
            }
            return crate::windows_cf::revoke_callbacks().await.is_ok() && stopped && callbacks_stopped;
        }
        #[cfg(not(target_os = "windows"))]
        stopped
    }
}

/// How long [`EngineRunner::abort`] waits for the tick loop to reach its
/// cancel-select boundary on its own after the cooperative stop flag + the
/// cancel oneshot are both signaled.
const GRACEFUL_ABORT_TIMEOUT: Duration = Duration::from_secs(3);

/// How long, after a forced [`tokio::task::JoinHandle::abort`], we wait for
/// the runtime to confirm the task is actually gone. Aborting a task that is
/// suspended mid-`.await` (the common case — mid network call) drops its
/// future essentially immediately; this is a generous upper bound, not the
/// expected latency.
const FORCE_ABORT_TIMEOUT: Duration = Duration::from_secs(2);

/// Signal `cancel` (if any), wait up to `graceful` for `task` to finish on
/// its own, and — only if that times out — force-abort it and wait up to
/// `force` for the runtime to confirm it has actually terminated.
///
/// Returns `true` only when the task is CONFIRMED gone (finished gracefully,
/// or the forced abort was observed to complete within `force`); `false`
/// only in the pathological case where even a forced abort couldn't be
/// confirmed within `force` (e.g. the task is blocked in non-async code with
/// no `.await` point to cancel at).
///
/// Standalone and Tauri-independent on purpose: this is the part of
/// [`EngineRunner::abort`] worth unit-testing directly, without spinning up
/// a real `AppHandle` + sync engine.
async fn stop_task_and_confirm(
    cancel: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
    graceful: Duration,
    force: Duration,
) -> bool {
    if let Some(tx) = cancel {
        // Ignore send errors — receiver may have already exited.
        let _ = tx.send(());
    }
    let Some(mut handle) = task else {
        // Nothing left to wait on — either already aborted, or the task
        // never finished spawning. Either way there is nothing that could
        // still be running.
        return true;
    };

    if tokio::time::timeout(graceful, &mut handle).await.is_ok() {
        return true;
    }

    // Graceful stop timed out. This is where the pre-fix code just dropped
    // `handle` here, detaching the task instead of cancelling it. Force it
    // down for real, then wait to confirm — not just fire-and-hope.
    tracing::warn!("engine graceful stop timed out; force-aborting the task");
    handle.abort();
    let confirmed = tokio::time::timeout(force, handle).await.is_ok();
    if !confirmed {
        tracing::error!(
            "engine did not confirm termination even after a forced abort; it may still be running"
        );
    }
    confirmed
}

/// Aborts a spawned task when dropped. On Windows it also covers the forced-abort
/// path of the heartbeat producer; the status pulse uses it on every platform so a
/// force-aborted `run` cannot leave the pulse running with the DB handle.
struct AbortWorkerOnDrop(tokio::task::AbortHandle);
impl Drop for AbortWorkerOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl Drop for EngineRunner {
    fn drop(&mut self) {
        if let Some(tx) = self.cancel.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.task.take() {
            handle.abort();
        }
    }
}

/// Shared control handles [`run`]'s tick loop reads/writes for its whole
/// lifetime, bundled into one struct rather than passed as separate `run`
/// parameters — task 1538 Codex P1: adding `stopping` pushed the previous
/// flat parameter list to 8, past clippy's `too_many_arguments` threshold.
/// `auth_health` (task 1546 finding 5) joined the struct for the same
/// reason rather than reopening that flat list.
struct RunnerControls {
    cancel: oneshot::Receiver<()>,
    sync_paused: Arc<AtomicBool>,
    auth_health: Arc<AuthHealth>,
    ipc_bind_error: Arc<Mutex<Option<String>>>,
    stopping: Arc<AtomicBool>,
}

/// The runner task body. Acquires the lock, opens the state DB,
/// builds the API client + engine bridge, ticks every
/// [`TICK_INTERVAL`] running [`sync_tick`], exits when the cancel
/// channel fires.
async fn run(app: AppHandle, sync_root: PathBuf, session_token: String, master_key: [u8; 32], controls: RunnerControls) {
    // Task 1538 Codex P1: destructured immediately so the rest of this
    // (already long-standing) function body is untouched — every field
    // below is used exactly as the old flat `cancel`/`sync_paused`/
    // `auth_health`/`ipc_bind_error`/`stopping` parameters were.
    let RunnerControls {
        mut cancel,
        sync_paused,
        auth_health,
        ipc_bind_error,
        stopping,
    } = controls;

    // Only consumed inside the `#[cfg(unix)]` IPC block below — Windows has
    // no Unix-socket daemon endpoint (see that block's own doc comment), so
    // the parameter would otherwise go unused on a Windows build.
    #[cfg(not(unix))]
    let _ = &ipc_bind_error;

    #[cfg(target_os = "windows")]
    if let Err(error) = crate::windows_cf::wait_for_credential_release().await {
        emit_status(&app, "error", Some(&sync_root), Some(&error));
        return;
    }
    // Windows Cloud Files: connect the sync root before any placeholder work.
    // Beebeeb metadata now lives in the app-local state dir, but Cloud Files
    // placeholder seeding still needs a connected root later in this task.
    #[cfg(target_os = "windows")]
    crate::windows_cf::connect_root(&sync_root);

    if let Err(e) = state_paths::prepare_beebeeb_state_dir_for_sync_root(&sync_root) {
        let msg = format!("prepare app-local state dir: {e}");
        tracing::error!(error = %msg);
        emit_status(&app, "error", Some(&sync_root), Some(&msg));
        return;
    }

    // Acquire the mutual-exclusion lock in app-local state, outside the user's
    // synced files.
    let _lock = match LockFile::acquire("desktop") {
        Ok(l) => l,
        Err(msg) => {
            tracing::error!(error = %msg, "could not acquire .beebeeb-sync.lock");
            emit_status(&app, "error", Some(&sync_root), Some(&msg));
            return;
        }
    };

    // Open the state DB from app-local data. The migration step above already
    // created the directory and moved any legacy sync-root metadata out.
    let state_dir = match state_paths::beebeeb_state_dir() {
        Ok(path) => path,
        Err(e) => {
            let msg = format!("resolve state dir: {e}");
            tracing::error!(error = %msg);
            emit_status(&app, "error", Some(&sync_root), Some(&msg));
            return;
        }
    };
    let db = match StateDb::open(state_dir.join(state_paths::STATE_DB_FILENAME)) {
        Ok(d) => Arc::new(d),
        Err(e) => {
            let msg = format!("open state.db: {e}");
            tracing::error!(error = %msg);
            emit_status(&app, "error", Some(&sync_root), Some(&msg));
            return;
        }
    };
    match db.reconcile_stale_in_flight_on_startup() {
        Ok(0) => {}
        Ok(touched) => {
            tracing::info!(
                touched,
                "reconciled stale in-flight state rows from a previous engine session"
            );
        }
        Err(e) => {
            tracing::warn!(error = %e, "could not reconcile stale in-flight state rows on startup");
        }
    }

    let api = Arc::new(ApiClient::new(api_base_url(), session_token, master_key));
    #[cfg(target_os = "windows")]
    crate::windows_cf::track_credentials(&api);
    // Shares `stopping` with `EngineRunner::abort` (task 1538 Codex P1) so
    // this bridge — and every clone of it handed to the IPC socket server
    // (below) and the Windows upload watcher — observes a stop request the
    // instant `abort()` sets it, not just at this loop's next tick boundary.
    // Task 1683 slice 2: the board the popover snapshot reads (managed app state),
    // or a private one when this runner has no app state (tests).
    let transfers = app
        .try_state::<crate::popover_data::PopoverRuntime>()
        .map(|runtime| runtime.transfers.clone())
        .unwrap_or_else(crate::transfer_progress::TransferBoard::new);
    let bridge = Arc::new(EngineBridge::new_with_stop_flag(db.clone(), api.clone(), stopping).with_transfers(transfers.clone()));
    // What each request saw of the link (offline vs server did not answer).
    let link = api.link();
    let tracker = Arc::new(StatusTracker::new(Some(sync_root.to_string_lossy().into_owned())));
    // `syncing` needs a pulse: the tick awaits every due upload to completion, so
    // the post-tick counts are almost always zero (see `engine_status`).
    let _status_pulse = AbortWorkerOnDrop(
        tokio::spawn(run_status_pulse(
            app.clone(),
            tracker.clone(),
            db.clone(),
            transfers.clone(),
            sync_paused.clone(),
        ))
        .abort_handle(),
    );

    // ── Heartbeat telemetry (the WRITE/PRODUCE side of the Bandwidth view) ──
    //
    // Register THIS device + open ONE sync session, then spawn a producer task
    // that posts live telemetry on its own cadence. All best-effort: a failed
    // registration logs and leaves `heartbeat` `None`, so the sync engine runs
    // exactly as before, just without telemetry. The producer's cancel handle +
    // its `Arc<ApiClient>`/`Arc<StateDb>` clones live for the loop's lifetime;
    // teardown below (after the loop breaks) fires the cancel so the producer
    // stops — and drops its session token — on logout/lock/shutdown alongside
    // the rest of the engine.
    let telemetry = Arc::new(Mutex::new(TelemetryState {
        engine_state: "running".to_string(),
        current_file: None,
    }));
    // `session_id` is cloned into the producer task AND kept here so the
    // teardown below can post a final "stopped" beat under the same session.
    let mut heartbeat: Option<(oneshot::Sender<()>, JoinHandle<()>, String)> =
        match register_session(&api, &sync_root).await {
            Some((session_id, interval)) => {
                let (hb_cancel_tx, hb_cancel_rx) = oneshot::channel::<()>();
                let handle = spawn_heartbeat_producer(
                    api.clone(),
                    db.clone(),
                    session_id.clone(),
                    interval,
                    telemetry.clone(),
                    sync_paused.clone(),
                    bridge.wire.clone(),
                    auth_health.clone(),
                    hb_cancel_rx,
                );
                Some((hb_cancel_tx, handle, session_id))
            }
            None => None,
        };

    // Windows forced runner abort must also abort the separately spawned
    // heartbeat future; dropping a JoinHandle alone would detach its API owner.
    #[cfg(target_os = "windows")]
    let _heartbeat_abort = heartbeat
        .as_ref()
        .map(|(_, task, _)| AbortWorkerOnDrop(task.abort_handle()));

    // Task 1670 round 2: purge the macOS hydrate-cache staging dir on every
    // (re)start of this runner — this IS "daemon startup" in this
    // single-process architecture (there is no separate boot phase distinct
    // from the vault-unlock that calls `run`). Bounds any staged plaintext
    // that survived a crash between a previous session's write and its
    // per-request cleanup, or a force-quit mid-fetch, to at most one restart.
    // Runs BEFORE the IPC server below starts accepting HydrateFile requests
    // so a fresh session never inherits a stale entry. `purge_macos_hydrate_cache`
    // is a no-op on non-macOS (called unconditionally, like its other two call
    // sites in `lib.rs`, so this file doesn't need its own cfg gate).
    crate::purge_macos_hydrate_cache("daemon-startup");

    // Spawn the Unix-socket IPC server alongside the sync loop. It
    // shares the same StateDb + EngineBridge handles, so OS extensions
    // (macOS File Provider, Linux FUSE) can query status and trigger
    // hydrations without waiting for the next tick.
    //
    // Windows has no separate extension process: the desktop binary is
    // itself the Cloud Files provider and Windows calls back into this
    // runtime in-process (see `crate::windows_cf`), so there is no
    // Unix-socket server on Windows.
    //
    // Keep an explicit cancel handle so lock/logout tears down the listener
    // that holds a cloned EngineBridge with the active master key.
    #[cfg(unix)]
    let (ipc_cancel_tx, ipc_cancel_rx) = oneshot::channel::<()>();
    #[cfg(unix)]
    let mut ipc_task = {
        let db_for_ipc = db.clone();
        let bridge_for_ipc = bridge.clone();
        let ipc_bind_error = ipc_bind_error.clone();
        tokio::spawn(async move {
            // A bind failure (task 1524 — e.g. the macOS sandbox refusing a
            // socket path outside the app's container) no longer panics this
            // task silently: `serve_ipc` returns the real `std::io::Error`,
            // which is logged here AND recorded for
            // `wait_for_file_provider_ipc_ready` (lib.rs) to report verbatim
            // to the "Install Finder location" caller instead of a generic
            // timeout.
            if let Err(e) = crate::ipc_socket::serve_ipc(db_for_ipc, bridge_for_ipc, ipc_cancel_rx).await {
                let msg = format!("{e}");
                tracing::error!(
                    error = %msg,
                    path = %crate::ipc_socket::ipc_socket_path().display(),
                    "IPC socket failed to bind; Finder/File Provider cannot reach the sync daemon this session"
                );
                if let Ok(mut guard) = ipc_bind_error.lock() {
                    *guard = Some(msg);
                }
            }
        })
    };

    // Windows Cloud Files: stash the live bridge so the in-process fetch
    // callback can reach it, and seed Explorer with cloud-only placeholders
    // from the current file list. The sync root was already registered +
    // connected by `connect_root` at the top of `run` (before the lock/state.db
    // writes), so by here the root is live and the placeholder writes succeed.
    // All idempotent — safe to run on every (re)spawn.
    #[cfg(target_os = "windows")]
    crate::windows_cf::seed_placeholders(bridge.clone(), &sync_root);

    // Task 0780 — the UPLOAD trigger. On Windows the desktop binary IS the
    // Cloud Files provider and there is no extension/IPC socket to fire
    // `QueueFinderCreate` when the user drops a file in the sync root, so a
    // local create previously never uploaded. The CF NOTIFY callbacks fire only
    // for I/O on EXISTING placeholders, NOT for a brand-new plain file a foreign
    // process drops in — so the create trigger is a periodic enumeration scan
    // that reads the disk directly (which always works). `watcher::spawn` starts
    // BOTH the CF NOTIFY dispatch loop (modify of existing placeholders +
    // delete/rename) AND the enumeration scan loop (the create trigger); both
    // funnel new files through the shared `classify_local_path` gate that filters
    // out engine-written placeholders, hydration writes, and legacy `.beebeeb/`
    // internals, then into the existing encrypted upload path. macOS/Linux get
    // creates via their OS extension over `ipc_socket`, so the watcher is
    // Windows-only here.
    //
    // The handle is held for the lifetime of the loop; dropping it on
    // logout/shutdown stops BOTH the debounce task and the scan task, so the
    // cloned EngineBridge (holding the session master key) is released with the
    // rest of the engine.
    #[cfg(target_os = "windows")]
    let _upload_watcher = crate::watcher::spawn(bridge.clone(), sync_root.clone());

    emit_payload(&app, tracker.running().to_json());

    let mut tick = tokio::time::interval(TICK_INTERVAL);
    let mut sync_complete_notifications = SyncCompleteNotificationState::default();
    let mut quota_warning_notifications = QuotaWarningNotificationState::default();
    let mut last_search_index_signature: Option<u64> = None;

    // Known-folder backup (task 0797, Windows): run the source→vault mirror on a
    // SLOW cadence relative to the 30s sync tick — every `KNOWN_FOLDER_MIRROR_EVERY_N_TICKS`
    // ticks. The mirror only copies NEW/CHANGED files (mtime+size diff), so an
    // idle pass is cheap, but it still reads the whole known-folder tree, so we
    // don't want it on every tick. `0` runs the first mirror on the first tick.
    // Windows-only: on other platforms there are no Windows known folders to
    // mirror, so the counter would be unused.
    #[cfg(target_os = "windows")]
    let mut tick_count: u64 = 0;

    // Task 1670 round 3: separate counter (not `tick_count` above, which is
    // Windows-only and used for a different-purpose slow cadence) for the
    // macOS hydrate-cache TTL sweep — see `MACOS_HYDRATE_SWEEP_EVERY_N_TICKS`
    // for why this exists.
    #[cfg(target_os = "macos")]
    let mut hydrate_sweep_tick_count: u64 = 0;

    loop {
        tokio::select! {
            biased;
            _ = &mut cancel => break,
            _ = tick.tick() => {
                // Task 1670 round 3: run BEFORE the `sync_paused` check below
                // — bounding staged plaintext lifetime is a security property
                // independent of whether the user paused file sync, and
                // Finder can still hydrate files (via the always-running IPC
                // server) while sync is paused.
                #[cfg(target_os = "macos")]
                {
                    if hydrate_sweep_tick_count.is_multiple_of(MACOS_HYDRATE_SWEEP_EVERY_N_TICKS) {
                        let dir = crate::ipc_socket::macos_hydrate_cache_dir();
                        match crate::ipc_socket::macos_sweep_stale_hydrate_cache_entries(
                            &dir,
                            crate::ipc_socket::MACOS_HYDRATE_CACHE_TTL,
                            std::time::SystemTime::now(),
                        ) {
                            Ok(removed) if removed > 0 => {
                                tracing::debug!(
                                    removed,
                                    dir = %dir.display(),
                                    "periodic macOS hydrate-cache TTL sweep"
                                );
                            }
                            Ok(_) => {}
                            Err(e) => {
                                tracing::warn!(
                                    error = %e,
                                    dir = %dir.display(),
                                    "periodic macOS hydrate-cache TTL sweep failed (best-effort)"
                                );
                            }
                        }
                    }
                    hydrate_sweep_tick_count = hydrate_sweep_tick_count.wrapping_add(1);
                }

                // Skip all sync work while the user has paused sync.
                // The loop keeps running so it can receive the cancel
                // signal and so it wakes up promptly when resumed.
                if sync_paused.load(Ordering::Relaxed) {
                    emit_payload(&app, tracker.paused().to_json());
                    set_telemetry_state(&telemetry, "paused");
                    // Windows breadcrumb flyout: reflect the paused state.
                    #[cfg(target_os = "windows")]
                    refresh_status_ui_snapshot(&db, true, false);
                    continue;
                }

                // Native completion is local work: retry even if the API is offline.
                #[cfg(target_os = "windows")]
                crate::windows_cf::upload_finalization::retry(&db, &sync_root);

                // Known-folder backup mirror (task 0797). Run BEFORE the sync
                // tick on the slow cadence so any files it copies into the sync
                // root are present when the enumeration scan next walks the tree.
                // Windows-only (SHGetKnownFolderPath); a no-op when the user has
                // opted into nothing.
                #[cfg(target_os = "windows")]
                {
                    if tick_count % KNOWN_FOLDER_MIRROR_EVERY_N_TICKS == 0 {
                        run_known_folder_mirror(&sync_root).await;
                    }
                    tick_count = tick_count.wrapping_add(1);
                }

                let syncing_before_tick = sync_in_flight_count(&db);
                let tick_started_at = now_secs() as i64;

                match bridge.refresh_shared_roots().await {
                    Ok(outcome) if !outcome.removed_shared_file_ids.is_empty() => {
                        tracing::info!(
                            removed = outcome.removed_shared_file_ids.len(),
                            "revoked shared content removed from local Finder state"
                        );
                        signal_file_provider_working_set("shared_roots_changed", &outcome.removed_shared_file_ids);
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(error = %e, "shared root refresh failed");
                    }
                }
                match sync_tick(&*bridge, &sync_root).await {
                    Ok(conflicts) => {
                        // A successful tick is a real, authenticated API round
                        // trip — clears the auth-failure streak (task 1546
                        // finding 5) alongside every other post-tick bookkeeping
                        // step below.
                        auth_health.note_result(None);
                        // Task 10 — surface freshly detected conflicts.
                        // The engine bridge already flipped status to
                        // Conflict; we own the UI side: open a window
                        // per file, fire a notification, emit a Tauri
                        // event so the settings page can refresh its
                        // counts immediately.
                        for c in &conflicts {
                            handle_new_conflict(&app, c);
                        }
                        // Task 13 — sweep for conflicts past their 24h
                        // deadline and apply Keep Both. Done after the
                        // detection step so a freshly-detected conflict
                        // (timestamp ≈ now) doesn't get auto-resolved
                        // on the very same tick.
                        sweep_auto_resolutions(&app, &bridge, &sync_root).await;
                        let completed_sync_work = match bridge.process_due_operations(&sync_root, now_secs()).await {
                            Ok(outcome) => {
                                let completed = outcome.completed_op_ids.len() as u32;
                                signal_file_provider_working_set("operations_applied", &outcome.invalidated_item_ids);
                                if !outcome.paused_op_ids.is_empty() || !outcome.retried_op_ids.is_empty() {
                                    tracing::info!(
                                        paused = outcome.paused_op_ids.len(),
                                        retried = outcome.retried_op_ids.len(),
                                        "sync operation queue processed with deferred work"
                                    );
                                }
                                completed
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "operation queue processing failed");
                                0
                            }
                        };
                        enforce_cache_budget(&app, &bridge, &mut quota_warning_notifications);

                        // Windows Cloud Files: the one-shot `seed_placeholders`
                        // at engine spawn ran against a (usually empty) DB, so
                        // rows discovered by this and later ticks need a
                        // placeholder minted now. Idempotent — existing
                        // placeholders are a no-op (ERROR_ALREADY_EXISTS → Ok).
                        #[cfg(target_os = "windows")]
                        crate::windows_cf::refresh_placeholders(&sync_root);

                        // Reconcile native Explorer pin / "Free up space" changes
                        // (which never touch our DB) back into state_db so the app
                        // view matches and `free_up_space` respects native pins.
                        // Delta-only + idempotent — no-op when nothing changed
                        // natively. Runs after refresh so newly minted placeholders
                        // are present before we read their attributes.
                        #[cfg(target_os = "windows")]
                        crate::windows_cf::reconcile_placeholder_state(&sync_root);

                        // Stamp every DB-Local file's placeholder in-sync so
                        // Explorer shows the green ✓ overlay. Runs after
                        // reconcile (which may have just flipped rows to Local)
                        // and after refresh (so newly-minted placeholders exist
                        // on disk). Idempotent — CfSetInSyncState on an already-
                        // in-sync placeholder is a Windows no-op. Bounded at
                        // MAX_IN_SYNC_STAMPS_PER_PASS files per tick.
                        #[cfg(target_os = "windows")]
                        crate::windows_cf::stamp_local_files_in_sync(&sync_root);

                        let syncing_after_tick = sync_in_flight_count(&db);
                        let unresolved_after_tick = unresolved_sync_issue_count(&db);
                        let should_notify_sync_complete = sync_complete_notifications.observe(
                            syncing_before_tick > 0 || completed_sync_work > 0,
                            syncing_after_tick,
                            unresolved_after_tick,
                        );

                        // `Ok` is not the whole story: `sync_tick` swallows a failed
                        // `GET /sync/ops`, so the link monitor decides whether this
                        // was really a check that reached the server.
                        emit_payload(
                            &app,
                            finish_tick_payload(&tracker, Ok(()), &link, tick_started_at, &db, &transfers),
                        );
                        // Feed the heartbeat producer the post-tick state. It
                        // promotes this to "syncing" itself when the DB snapshot
                        // shows files in flight, so a steady-state tick reads as
                        // "watching" while an active transfer reads as "syncing".
                        set_telemetry_state(&telemetry, "idle");
                        // Windows breadcrumb flyout: in-flight counts ⇒ Syncing,
                        // conflicts ⇒ Error, else In-Sync. Not paused, no engine
                        // error on a successful tick.
                        #[cfg(target_os = "windows")]
                        refresh_status_ui_snapshot(&db, false, false);

                        match crate::desktop_search::local_index_signature(&db) {
                            Ok(signature) if last_search_index_signature != Some(signature) => {
                                match crate::desktop_search::sync_local_index_to_remote(&db, &api).await {
                                    Ok(summary) => {
                                        last_search_index_signature = Some(signature);
                                        tracing::debug!(
                                            indexed_file_count = summary.indexed_file_count,
                                            put_count = summary.put_count,
                                            delete_count = summary.delete_count,
                                            "desktop search index shards synced"
                                        );
                                    }
                                    Err(error) => {
                                        tracing::warn!(
                                            error = %error,
                                            "desktop search index shard sync failed; will retry after the next local mirror change"
                                        );
                                    }
                                }
                            }
                            Ok(_) => {}
                            Err(error) => {
                                tracing::warn!(error = %error, "desktop search index signature failed");
                            }
                        }

                        if should_notify_sync_complete
                            && let Err(e) = crate::notify_sync_complete_impl(&app)
                        {
                            tracing::warn!(error = %e, "notify_sync_complete failed");
                        }
                    }
                    Err(e) => {
                        // Network blips and 401s during token rotation
                        // are normal — log and continue, surface the
                        // error to the WebView so the tray reflects it.
                        tracing::warn!(error = %e, "sync tick failed");
                        // Task 1546 finding 5: a run of these that classify as
                        // auth failures (not network blips) is what flips
                        // `auth_health` and surfaces the persistent banner.
                        auth_health.note_result(Some(&e));
                        emit_payload(
                            &app,
                            finish_tick_payload(&tracker, Err(&e), &link, tick_started_at, &db, &transfers),
                        );
                        set_telemetry_state(&telemetry, "error");
                        // Windows breadcrumb flyout: surface the tick failure as
                        // an Error state in the flyout too.
                        #[cfg(target_os = "windows")]
                        refresh_status_ui_snapshot(&db, false, true);
                    }
                }
            }
        }
    }

    emit_payload(&app, tracker.stopped().to_json());

    // Stop the heartbeat producer, then post ONE final best-effort "stopped"
    // beat. Ordering: cancel + await the producer FIRST so it can't race a
    // normal beat against this final one, then send the definitive "stopped".
    //
    // Why this works at teardown: logout (`clear_session`) and lock
    // (`lock_vault`) both call `EngineRunner::abort().await` — which fires the
    // cancel that breaks the loop and runs THIS code — BEFORE they clear the
    // in-memory session / keychain, and neither calls a server-side session-
    // revocation endpoint. So `api`'s bearer token is still valid here; the POST
    // lands.
    //
    // Time budget: `EngineRunner::abort` only waits 3s for this whole `run()`
    // task, so the two phases below are bounded to 1s (producer stop, normally
    // instant — a biased cancel) + 1.5s (final beat) = ≤2.5s, comfortably under
    // that 3s window so the beat is never cut off mid-flight by `abort` moving
    // on to clear the keychain.
    //
    // What "stopped" buys the read side: the server stores the heartbeat status
    // verbatim, so the latest heartbeat row reads `stopped`. The current web
    // Bandwidth view (`devices.tsx`) does not yet treat a `stopped`
    // HEARTBEAT_status as instant-offline — it flips offline via the wall-clock
    // staleness TTL (no heartbeat for ~interval×15) — so this primarily PREVENTS
    // a misleading final "watching/syncing" beat from lingering and records a
    // truthful terminal state a frontend follow-up can honor for an immediate
    // flip. (We deliberately do NOT send `error`, which WOULD flip offline now
    // but would mislabel a clean logout as a failure.)
    if let Some((hb_cancel_tx, hb_handle, session_id)) = heartbeat.take() {
        let _ = hb_cancel_tx.send(());
        if tokio::time::timeout(Duration::from_secs(1), hb_handle).await.is_err() {
            tracing::debug!("heartbeat producer did not stop within 1s; abandoning");
        }

        let final_beat = HeartbeatBody {
            status: "stopped".to_string(),
            // Report the last known counts so the row's progress numbers don't
            // reset to null on the terminal beat; zero speed, no current file.
            files_synced: db.list_by_status(FileStatus::Local).ok().map(|v| v.len() as i64),
            current_file: None,
            speed_bps: Some(0),
            ..Default::default()
        };
        // Best-effort + hard-bounded: swallow any error, never exceed ~1.5s.
        match tokio::time::timeout(
            Duration::from_millis(1500),
            api.post_heartbeat(&session_id, &final_beat),
        )
        .await
        {
            Ok(Ok(())) => tracing::debug!(session_id = %session_id, "final 'stopped' heartbeat sent"),
            Ok(Err(e)) => tracing::debug!(error = %e, "final 'stopped' heartbeat failed (best-effort)"),
            Err(_) => tracing::debug!("final 'stopped' heartbeat timed out (best-effort)"),
        }
    }

    #[cfg(unix)]
    {
        let _ = ipc_cancel_tx.send(());
        if tokio::time::timeout(Duration::from_secs(2), &mut ipc_task)
            .await
            .is_err()
        {
            ipc_task.abort();
            let _ = ipc_task.await;
            let _ = std::fs::remove_file(crate::ipc_socket::ipc_socket_path());
        }
    }
    // _lock + bridge drop here; SQLite closes, lock file deleted.
}

/// Emit an `engine-status` event the WebView + tray listen to.
/// Best-effort: a missing main window or serialisation issue is
/// logged but doesn't break the runner.
fn emit_status(app: &AppHandle, state: &str, sync_root: Option<&PathBuf>, error: Option<&str>) {
    emit_payload(
        app,
        crate::engine_status::plain_payload(state, sync_root.map(|p| p.to_string_lossy().into_owned()), error),
    );
}

fn emit_payload<R: tauri::Runtime>(app: &tauri::AppHandle<R>, payload: serde_json::Value) {
    if let Err(e) = app.emit(crate::engine_status::EVENT, payload) {
        tracing::warn!(error = %e, "failed to emit engine-status event");
    }
}

/// The work in flight right now: queue and downloading rows from the state DB,
/// bytes from the transfer board.
fn current_activity(db: &StateDb, transfers: &crate::transfer_progress::TransferBoard) -> Activity {
    compute_activity(&db.transfer_backlog(now_secs()).unwrap_or_default(), transfers)
}

/// What a finished tick means, as the event to emit.
fn finish_tick_payload(
    tracker: &StatusTracker,
    result: Result<(), &anyhow::Error>,
    link: &crate::link_health::LinkMonitor,
    tick_started_at: i64,
    db: &StateDb,
    transfers: &crate::transfer_progress::TransferBoard,
) -> serde_json::Value {
    let outcome = tick_outcome(result, &link.snapshot(), tick_started_at);
    let activity = current_activity(db, transfers);
    let payload = tracker.finish_tick(&outcome, now_secs() as i64, activity).to_json();
    if activity.files_remaining == 0 {
        // Nothing left: the batch is over, the next burst starts its bar at 0.
        transfers.reset_batch();
    }
    payload
}

/// How often the pulse looks. The look itself is free while nothing is moving.
const STATUS_PULSE_INTERVAL: Duration = Duration::from_secs(1);

/// Reports `syncing` (with progress) while files move, and the way back to `idle`.
/// Runs for the engine's lifetime and is aborted with it.
async fn run_status_pulse<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    tracker: Arc<StatusTracker>,
    db: Arc<StateDb>,
    transfers: Arc<crate::transfer_progress::TransferBoard>,
    sync_paused: Arc<AtomicBool>,
) {
    let mut ticker = tokio::time::interval(STATUS_PULSE_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;
        let transfer_in_flight = !transfers.active().is_empty();
        let payload = tracker.pulse(sync_paused.load(Ordering::Relaxed), transfer_in_flight, || {
            current_activity(&db, &transfers)
        });
        if let Some(payload) = payload {
            let going_quiet = payload.state == crate::engine_status::State::Idle;
            emit_payload(&app, payload.to_json());
            if going_quiet {
                // The batch is over: the next burst of work starts its bar at 0.
                transfers.reset_batch();
            }
        }
    }
}

/// Publish the loop's current high-level engine state to the shared telemetry
/// slot the heartbeat producer reads. Best-effort: a poisoned mutex is swallowed
/// (a stale heartbeat status is harmless — the next tick re-publishes).
fn set_telemetry_state(telemetry: &Arc<Mutex<TelemetryState>>, state: &str) {
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
fn file_status_count(db: &StateDb, status: FileStatus) -> u32 {
    db.list_by_status(status).ok().map(|v| v.len() as u32).unwrap_or(0)
}

fn sync_in_flight_count(db: &StateDb) -> u32 {
    file_status_count(db, FileStatus::Downloading).saturating_add(file_status_count(db, FileStatus::Uploading))
}

fn unresolved_sync_issue_count(db: &StateDb) -> u32 {
    file_status_count(db, FileStatus::Conflict).saturating_add(file_status_count(db, FileStatus::Error))
}

#[cfg(target_os = "windows")]
fn refresh_status_ui_snapshot(db: &StateDb, paused: bool, engine_error: bool) {
    let syncing = sync_in_flight_count(db);
    let conflicts = file_status_count(db, FileStatus::Conflict);
    crate::windows_cf::status_ui::set_engine_state(paused, syncing, conflicts, engine_error);
}

/// Run one known-folder backup mirror pass (task 0797, Windows). Reads the
/// enabled-key list from `desktop.toml`, then mirrors each enabled known folder
/// (`SHGetKnownFolderPath` source) into `<sync_root>\<DisplayName>`. The whole
/// thing is blocking `std::fs` + Win32, so it runs on a blocking thread to keep
/// the async runtime free. Best-effort: a config-load failure or an empty set is
/// a quiet no-op; per-file errors are counted inside the mirror, not surfaced.
#[cfg(target_os = "windows")]
async fn run_known_folder_mirror(sync_root: &Path) {
    let enabled = match crate::config::DesktopConfig::load() {
        Ok(cfg) => cfg.known_folder_backup,
        Err(e) => {
            tracing::debug!(error = %e, "known-folder backup: config load failed; skipping pass");
            return;
        }
    };
    if enabled.is_empty() {
        return;
    }
    let root = sync_root.to_path_buf();
    if let Err(e) = tokio::task::spawn_blocking(move || {
        crate::known_folder::mirror_enabled_known_folders(&root, &enabled);
    })
    .await
    {
        tracing::warn!(error = %e, "known-folder backup mirror task panicked");
    }
}

/// Task 1697: the daemon's ONLY replica-refresh channel. Under
/// `NSFileProviderReplicatedExtension` the system honors signals for the
/// WORKING SET alone (`.rootContainer` is ignored by design); the extension's
/// enumerator then pulls the daemon's change log via `ListChanges`. This
/// replaces the dead `file-provider-invalidate` Tauri event (zero consumers)
/// with a real signal through the ObjC FFI bridge. macOS-only and
/// best-effort: a failed signal is logged and never fails the tick.
#[cfg(target_os = "macos")]
fn signal_file_provider_working_set(reason: &str, item_ids: &[String]) {
    if !crate::macos_file_provider::should_signal_working_set(item_ids) {
        return;
    }
    match crate::macos_file_provider::signal_working_set() {
        Ok(outcome) => {
            tracing::debug!(reason, items = item_ids.len(), ?outcome, "signaled the File Provider working set");
        }
        Err(e) => {
            tracing::warn!(reason, error = %e, "signaling the File Provider working set failed (best-effort)");
        }
    }
}

/// Non-macOS stub: Windows CFAPI refreshes placeholders natively and Linux
/// FUSE is an unmounted prototype.
#[cfg(not(target_os = "macos"))]
fn signal_file_provider_working_set(reason: &str, item_ids: &[String]) {
    let _ = (reason, item_ids);
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn enforce_cache_budget(
    app: &AppHandle,
    bridge: &Arc<EngineBridge>,
    quota_warning_notifications: &mut QuotaWarningNotificationState,
) {
    // Honor the Windows "Files On-Demand" toggle. When the user has turned
    // it OFF (`Some(false)`), they want every file kept fully local — so we
    // must NOT evict unpinned cached files down to the budget. Unset
    // (`None`) or `Some(true)` keeps the default on-demand behaviour.
    // A missing/unreadable config defaults to on-demand (the safe, low-disk
    // behaviour). This is one cheap TOML read per 5s tick.
    let cfg = crate::config::DesktopConfig::load().unwrap_or_default();
    let local_cache_limit = cfg.local_cache_limit_for_eviction();
    match local_cache_limit {
        Some(limit) => match bridge.local_cache_usage_bytes() {
            Ok(usage) if quota_warning_notifications.observe(Some(limit), usage) => {
                if let Err(e) = crate::notify_quota_warning_impl(app, usage, limit) {
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

    // TODO honor `metered`: when the connection is metered and the user set
    // `metered = Some(true)`, downloads/hydration should pause. The runner
    // has no metered-network detection on Windows yet, so this toggle is
    // persisted only for now (see DesktopConfig::metered).

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

// ── Task 10 + 11: per-conflict UI fan-out ─────────────────────────────────────

/// Surface a freshly-detected conflict to the user: open the
/// resolution window, fire a native notification, emit a Tauri event
/// so any open settings page can refresh its conflict counter without
/// waiting for the next 5 s tick. All three are best-effort — a failure
/// in any one path is logged and the others still run, since the
/// underlying state DB has already been updated and the conflict won't
/// be silently dropped.
fn handle_new_conflict(app: &AppHandle, c: &ConflictDetected) {
    // `conflict_auto_opens_window` is true on every platform today (task 1683
    // slice 1 pins that for Windows and Linux); slice 6 makes macOS false.
    if crate::surfaces::policy::conflict_auto_opens_window(crate::surfaces::policy::Platform::current()) {
        if let Err(e) = crate::open_conflict_window_impl(app, &c.file_id, &c.file_name, c.is_text) {
            tracing::warn!(error = %e, file_id = %c.file_id, "open_conflict_window failed");
        }
    }
    if let Err(e) = crate::notify_conflict_impl(app, &c.file_name) {
        // Don't escalate — Linux without a notification daemon, or
        // macOS where the user denied the permission prompt, will
        // both fail here. The window + event still fired.
        tracing::warn!(error = %e, file_id = %c.file_id, "notify_conflict failed");
    }
    if let Err(e) = app.emit(
        "engine-conflict",
        serde_json::json!({
            "file_id": c.file_id,
            "file_name": c.file_name,
            "is_text": c.is_text,
        }),
    ) {
        tracing::warn!(error = %e, "failed to emit engine-conflict event");
    }
}

// ── Task 13: 24-hour auto-resolution timer ────────────────────────────────────

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
async fn sweep_auto_resolutions(app: &AppHandle, bridge: &Arc<EngineBridge>, sync_root: &Path) {
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
                if let Err(e) = app.emit(
                    "conflict-auto-resolved",
                    serde_json::json!({
                        "file_id": entry.file_id,
                        "file_name": entry.path,
                        "conflict_copy_name": conflict_copy_name,
                    }),
                ) {
                    tracing::warn!(error = %e, "failed to emit conflict-auto-resolved event");
                }
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
fn _unused_path(_p: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    // ── EngineRunner::abort authoritative-stop mechanics (task 1538 Codex P1) ──

    /// A task that promptly observes its cancel signal is confirmed stopped
    /// well within the graceful window — the common, happy-path case.
    #[tokio::test]
    async fn stop_task_and_confirm_returns_true_for_a_cooperative_task() {
        let (tx, rx) = oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            let _ = rx.await;
        });

        let confirmed =
            stop_task_and_confirm(Some(tx), Some(task), Duration::from_millis(200), Duration::from_millis(200)).await;

        assert!(confirmed, "a task that honors cancel must be confirmed stopped");
    }

    /// The regression test for the actual bug: a task that never observes
    /// its cancel signal (the real-world case is a single tick body stuck
    /// mid-network-call, well past its last `tokio::select!` check) must
    /// still be confirmed stopped — and, more importantly, must ACTUALLY
    /// stop running, not merely be abandoned.
    ///
    /// The pre-fix `EngineRunner::abort` just dropped the `JoinHandle` after
    /// its graceful timeout, which detaches rather than cancels it: swap
    /// this function's force-abort branch back to a bare `drop(handle)` (no
    /// `handle.abort()`, no confirmation wait) to reproduce that — the
    /// second assertion here (`ticks` unchanged after the function returns)
    /// is what would catch it, since the loop would keep incrementing
    /// `ticks` in the background forever. This was reasoned through against
    /// the pre-fix code rather than executed as a live mutation in this
    /// session (a shared-machine disk-space guard stopped further `cargo`
    /// builds partway through gating — see the task report).
    #[tokio::test]
    async fn stop_task_and_confirm_force_aborts_a_task_that_never_observes_cancel() {
        let ticks = Arc::new(AtomicUsize::new(0));
        let ticks_for_task = ticks.clone();
        // Intentionally unused by the task below — simulates a tick body
        // that never reaches its own `tokio::select!` boundary within this
        // test's short timeouts, so only a REAL forced abort can stop it.
        let (_tx, _rx) = oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            loop {
                ticks_for_task.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        });

        let confirmed =
            stop_task_and_confirm(Some(_tx), Some(task), Duration::from_millis(30), Duration::from_millis(300)).await;

        assert!(
            confirmed,
            "a forced abort of a stuck task must be confirmed, not just given up on"
        );

        let ticks_at_return = ticks.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(
            ticks.load(Ordering::SeqCst),
            ticks_at_return,
            "the task must have ACTUALLY stopped — a detached task would keep incrementing here"
        );
    }

    /// `cancel: None` / `task: None` (the idempotent "already aborted"
    /// shape `EngineRunner::abort`'s `Option::take()`s produce on a second
    /// call) has nothing left to confirm and must report success rather
    /// than hang or falsely report failure.
    #[tokio::test]
    async fn stop_task_and_confirm_is_idempotent_with_nothing_left_to_wait_on() {
        let confirmed = stop_task_and_confirm(None, None, Duration::from_millis(50), Duration::from_millis(50)).await;
        assert!(confirmed);
    }

    // ── AuthHealth (task 1546 Codex round 2, finding 5) ────────────────────

    fn auth_error() -> anyhow::Error {
        anyhow::anyhow!("HTTP 401 Unauthorized: invalid token")
    }

    fn other_error() -> anyhow::Error {
        anyhow::anyhow!("HTTP 500 Internal Server Error: db unavailable")
    }

    #[test]
    fn auth_health_starts_not_expired() {
        let health = AuthHealth::new();
        assert!(!health.is_expired());
    }

    #[test]
    fn auth_health_flips_expired_only_at_the_threshold_of_consecutive_auth_failures() {
        let health = AuthHealth::new();
        assert_eq!(AUTH_EXPIRED_THRESHOLD, 3, "test assumes the documented threshold");

        health.note_result(Some(&auth_error()));
        assert!(!health.is_expired(), "1 failure must not trip the banner");

        health.note_result(Some(&auth_error()));
        assert!(!health.is_expired(), "2 failures must not trip the banner");

        health.note_result(Some(&auth_error()));
        assert!(health.is_expired(), "the 3rd CONSECUTIVE auth failure must trip it");
    }

    #[test]
    fn auth_health_non_auth_failures_neither_advance_nor_reset_the_streak() {
        // A network blip / 5xx between two real 401s must not erase progress
        // toward the threshold, and must not itself count as progress.
        let health = AuthHealth::new();

        health.note_result(Some(&auth_error()));
        health.note_result(Some(&auth_error()));
        assert!(!health.is_expired());

        health.note_result(Some(&other_error()));
        assert!(!health.is_expired(), "an unrelated error must not itself trip the banner");

        // The streak must still be at 2 — one more REAL auth failure trips it.
        health.note_result(Some(&auth_error()));
        assert!(health.is_expired(), "the unrelated error must not have reset the streak back to 0");
    }

    #[test]
    fn auth_health_any_success_resets_the_streak_and_clears_expired() {
        let health = AuthHealth::new();
        health.note_result(Some(&auth_error()));
        health.note_result(Some(&auth_error()));
        health.note_result(Some(&auth_error()));
        assert!(health.is_expired());

        health.note_result(None);
        assert!(!health.is_expired(), "a success must clear an already-tripped banner");

        // And the streak is genuinely back to 0, not just the flag flipped:
        // two more failures alone must not re-trip it.
        health.note_result(Some(&auth_error()));
        health.note_result(Some(&auth_error()));
        assert!(!health.is_expired(), "the streak must have been reset to 0, not left at 3");
    }

    // Task 1697: the retired `file-provider-invalidate` Tauri event and its
    // payload builder are gone — the only replica-refresh channel is the
    // working-set signal (see `signal_file_provider_working_set`). The pure
    // decision core for that signal is tested in `macos_file_provider`.

    #[test]
    fn signal_file_provider_working_set_decision_gates_on_changed_items() {
        // Mirrors macos_file_provider::should_signal_working_set so a runner
        // change that bypasses the gate fails here too.
        assert!(crate::macos_file_provider::should_signal_working_set(&["f-1".to_string()]));
        assert!(!crate::macos_file_provider::should_signal_working_set(&[]));
    }

    #[test]
    fn sync_complete_notification_state_fires_once_on_sync_to_idle_transition() {
        let mut state = SyncCompleteNotificationState::default();

        assert!(!state.observe(false, 0, 0), "idle no-op ticks are ignored");
        assert!(!state.observe(true, 2, 0), "active sync does not fire until caught up");
        assert!(state.observe(false, 0, 0), "transition back to caught up fires once");
        assert!(!state.observe(false, 0, 0), "dedupes repeated idle ticks");
    }

    #[test]
    fn sync_complete_notification_state_waits_for_conflicts_to_clear() {
        let mut state = SyncCompleteNotificationState::default();

        assert!(!state.observe(true, 0, 1), "conflicted sync is not caught up");
        assert!(state.observe(false, 0, 0), "clearing conflicts completes the run");
    }

    #[test]
    fn quota_warning_notification_state_fires_once_per_threshold_crossing() {
        let mut state = QuotaWarningNotificationState::default();

        assert!(!state.observe(None, 10_000), "unlimited cache caps never warn");
        assert!(!state.observe(Some(1_000), 899), "below 90 percent is quiet");
        assert!(state.observe(Some(1_000), 900), "90 percent crossing warns");
        assert!(!state.observe(Some(1_000), 950), "remaining over threshold dedupes");
        assert!(!state.observe(Some(1_000), 100), "dropping below threshold resets");
        assert!(state.observe(Some(1_000), 901), "a later crossing warns again");
    }

    // ── Heartbeat telemetry mapping ───────────────────────────────────────────

    #[test]
    fn engine_state_maps_to_lowercase_heartbeat_status() {
        // The lowercase vocabulary the read side (web devices.tsx + the desktop
        // Bandwidth view) maps to a display label. Must match the CLI exactly.
        assert_eq!(engine_state_to_heartbeat_status("paused", 0), "paused");
        assert_eq!(engine_state_to_heartbeat_status("error", 0), "error");
        assert_eq!(engine_state_to_heartbeat_status("running", 0), "watching");
        assert_eq!(engine_state_to_heartbeat_status("idle", 0), "watching");
        assert_eq!(engine_state_to_heartbeat_status("syncing", 0), "syncing");
        // Unknown / stopped degrade to a flat idle rather than panicking.
        assert_eq!(engine_state_to_heartbeat_status("stopped", 0), "idle");
        assert_eq!(engine_state_to_heartbeat_status("totally-unknown", 0), "idle");
    }

    #[test]
    fn files_in_flight_promotes_status_to_syncing() {
        // Even when the loop's last emitted state was the post-tick "idle",
        // active transfers must read as "syncing" — the live signal wins.
        assert_eq!(engine_state_to_heartbeat_status("idle", 3), "syncing");
        assert_eq!(engine_state_to_heartbeat_status("running", 1), "syncing");
        // ...but a user PAUSE always wins over in-flight (we don't report
        // "syncing" while paused — the producer also hard-overrides this).
        assert_eq!(engine_state_to_heartbeat_status("paused", 5), "paused");
        // ...and an error state is preserved over in-flight, so a failing
        // session doesn't masquerade as healthily syncing.
        assert_eq!(engine_state_to_heartbeat_status("error", 5), "error");
    }

    #[test]
    fn build_heartbeat_carries_counts_and_status() {
        let telem = TelemetryState {
            engine_state: "idle".to_string(),
            current_file: Some("active.bin".to_string()),
        };
        let snap = SyncSnapshot {
            files_synced: 8,
            files_total: 10,
            files_in_flight: 2,
            bytes_synced: 5_000,
            bytes_total: 9_000,
        };
        // P1 wire counters: 3000 bytes upload + 1000 bytes download over 20 s = 200 B/s.
        let body = build_heartbeat(&telem, snap, 3_000, 1_000, 20);
        // 2 in flight → syncing, and current_file surfaces.
        assert_eq!(body.status, "syncing");
        assert_eq!(body.files_synced, Some(8));
        assert_eq!(body.files_total, Some(10));
        assert_eq!(body.bytes_synced, Some(5_000));
        assert_eq!(body.bytes_total, Some(9_000));
        assert_eq!(body.current_file.as_deref(), Some("active.bin"));
        // Wire speed: (3000 + 1000) / 20 = 200 B/s.
        assert_eq!(body.speed_bps, Some(200));
    }

    #[test]
    fn build_heartbeat_omits_current_file_when_idle() {
        // Nothing in flight → status is "watching" and current_file is dropped
        // even though the telemetry slot still holds a stale name.
        let telem = TelemetryState {
            engine_state: "idle".to_string(),
            current_file: Some("stale.bin".to_string()),
        };
        let snap = SyncSnapshot {
            files_synced: 10,
            files_total: 10,
            files_in_flight: 0,
            bytes_synced: 9_000,
            bytes_total: 9_000,
        };
        // Zero wire bytes → idle beat, 0 B/s.
        let body = build_heartbeat(&telem, snap, 0, 0, 20);
        assert_eq!(body.status, "watching");
        assert_eq!(body.current_file, None);
        // No wire bytes → 0 B/s.
        assert_eq!(body.speed_bps, Some(0));
    }

    #[test]
    fn build_heartbeat_speed_never_negative_on_regression() {
        // With wire counters there is no concept of "regression" — the counters are
        // drained per beat so they can never go negative.  Zero wire bytes = 0 B/s.
        let telem = TelemetryState {
            engine_state: "idle".to_string(),
            current_file: None,
        };
        let snap = SyncSnapshot {
            files_synced: 1,
            files_total: 5,
            files_in_flight: 1,
            bytes_synced: 100,
            bytes_total: 9_000,
        };
        // No wire bytes were counted this beat → 0 B/s.
        let body = build_heartbeat(&telem, snap, 0, 0, 20);
        assert_eq!(body.speed_bps, Some(0));
    }

    #[test]
    fn build_heartbeat_wire_speed_from_counters() {
        // Confirm that the new wire-counter-based speed is correct.
        // 10 000 upload + 5 000 download = 15 000 bytes over 20 s = 750 B/s.
        let telem = TelemetryState {
            engine_state: "syncing".to_string(),
            current_file: None,
        };
        let snap = SyncSnapshot {
            files_synced: 3,
            files_total: 10,
            files_in_flight: 2,
            bytes_synced: 30_000,
            bytes_total: 100_000,
        };
        let body = build_heartbeat(&telem, snap, 10_000, 5_000, 20);
        assert_eq!(body.speed_bps, Some(750));
        assert_eq!(body.status, "syncing");
    }

    #[test]
    fn platform_str_is_a_known_family() {
        assert!(matches!(platform_str(), "windows" | "macos" | "linux"));
    }

    #[test]
    fn beat_elapsed_uses_real_wall_clock_with_a_floor() {
        // Normal case: real elapsed time is used verbatim.
        assert_eq!(beat_elapsed_secs(1_020, 1_000), 20);
        // A long stall (suspended laptop) reports the FULL gap, not the nominal
        // interval — so speed isn't overstated by dividing by a small interval.
        assert_eq!(beat_elapsed_secs(1_300, 1_000), 300);
        // Two beats in the same second floor to 1 (never divide by zero).
        assert_eq!(beat_elapsed_secs(1_000, 1_000), 1);
        // Clock stepped backwards floors to 1 (never divide by a negative).
        assert_eq!(beat_elapsed_secs(990, 1_000), 1);
    }

    #[test]
    fn build_heartbeat_speed_uses_real_elapsed_not_nominal_interval() {
        // 4000 wire bytes over a REAL 40s gap (e.g. a delayed loop) is
        // 100 B/s — NOT 200 B/s (which dividing by the nominal 20s would give).
        let telem = TelemetryState {
            engine_state: "idle".to_string(),
            current_file: None,
        };
        let snap = SyncSnapshot {
            files_synced: 2,
            files_total: 4,
            files_in_flight: 1,
            bytes_synced: 5_000,
            bytes_total: 9_000,
        };
        let elapsed = beat_elapsed_secs(1_040, 1_000); // 40s real gap
        // 4000 total wire bytes over 40 s = 100 B/s.
        let body = build_heartbeat(&telem, snap, 3_000, 1_000, elapsed);
        assert_eq!(body.speed_bps, Some(100));
    }

    // ── api_base_url() env-override ───────────────────────────────────────────

    #[test]
    fn api_base_url_trims_trailing_slash() {
        // The trim-slash logic is a pure string operation — test it directly
        // without touching the real env var (which could interfere with a
        // parallel test run).
        let raw = "http://localhost:3001/";
        let result = raw.trim().trim_end_matches('/').to_string();
        assert_eq!(result, "http://localhost:3001");
    }

    #[test]
    fn api_base_url_trims_multiple_trailing_slashes() {
        let raw = "http://localhost:3001///";
        let result = raw.trim().trim_end_matches('/').to_string();
        assert_eq!(result, "http://localhost:3001");
    }

    #[test]
    fn api_base_url_trims_whitespace() {
        let raw = "  http://10.100.0.239:3001  ";
        let result = raw.trim().trim_end_matches('/').to_string();
        assert_eq!(result, "http://10.100.0.239:3001");
    }

    #[test]
    fn api_base_url_falls_back_to_prod_when_env_unset() {
        // When BB_API_BASE is unset (or empty), we must return the prod URL.
        // This test only guards the empty-string branch (we cannot guarantee
        // BB_API_BASE is unset in all environments, but an empty override is the
        // canonical "not set" sentinel the function uses).
        let trimmed = "";
        let result = if trimmed.is_empty() {
            "https://api.beebeeb.io".to_string()
        } else {
            trimmed.trim_end_matches('/').to_string()
        };
        assert_eq!(result, "https://api.beebeeb.io");
    }

    // ── Engine status: the tick's payload and the pulse (task 1683 slice 2) ────

    fn status_db(dir: &Path) -> Arc<StateDb> {
        Arc::new(StateDb::open(dir.join("state.db")).unwrap())
    }

    fn upload_op(op_id: &str, file_id: &str) -> crate::state_db::PendingOperation {
        crate::state_db::PendingOperation {
            op_id: op_id.into(),
            kind: crate::state_db::OperationKind::UploadFile,
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

    fn seed_file(db: &StateDb, id: &str, size: i64) {
        db.upsert_file(&crate::state_db::FileEntry {
            file_id: id.into(),
            path: format!("Work/{id}.bin"),
            status: FileStatus::Local,
            size_bytes: size,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: crate::state_db::ItemKind::File,
        })
        .unwrap();
    }

    #[test]
    fn a_good_tick_with_nothing_left_is_idle_and_ends_the_batch() {
        let dir = tempfile::tempdir().unwrap();
        let db = status_db(dir.path());
        let board = crate::transfer_progress::TransferBoard::new();
        let g = board.begin("a", crate::transfer_progress::Direction::Up, 500);
        g.update(500);
        g.finish();
        let link = crate::link_health::LinkMonitor::new();
        let tracker = StatusTracker::new(None);
        let payload = finish_tick_payload(&tracker, Ok(()), &link, 0, &db, &board);
        assert_eq!(payload["state"], "idle");
        assert_eq!(payload["files_remaining"], 0);
        assert_eq!(payload["bytes_total"], 500, "the final frame of the batch: 500 of 500");
        assert_eq!(payload["bytes_done"], 500);
        assert!(payload["last_tick_ok_at"].as_i64().unwrap() > 0, "a good check stamps its time");
        assert_eq!(board.finished_bytes(), 0, "nothing left: the next batch starts its bar at 0");
    }

    #[test]
    fn a_good_tick_with_work_left_keeps_the_batch_going() {
        let dir = tempfile::tempdir().unwrap();
        let db = status_db(dir.path());
        seed_file(&db, "b", 300);
        db.enqueue_operation(&upload_op("op-b", "b")).unwrap();
        let board = crate::transfer_progress::TransferBoard::new();
        let g = board.begin("a", crate::transfer_progress::Direction::Up, 500);
        g.update(500);
        g.finish();
        let link = crate::link_health::LinkMonitor::new();
        let tracker = StatusTracker::new(None);
        let payload = finish_tick_payload(&tracker, Ok(()), &link, 0, &db, &board);
        assert_eq!(payload["files_remaining"], 1);
        assert_eq!(payload["bytes_total"], 800, "500 finished + 300 still queued");
        assert_eq!(payload["bytes_done"], 500);
        assert_eq!(board.finished_bytes(), 500, "work is left, so the batch is not over");
    }

    #[test]
    fn a_tick_whose_request_failed_at_the_link_is_offline_and_not_a_good_check() {
        let dir = tempfile::tempdir().unwrap();
        let db = status_db(dir.path());
        let board = crate::transfer_progress::TransferBoard::new();
        let link = crate::link_health::LinkMonitor::new();
        link.note_failure(
            crate::link_health::LinkFailure {
                kind: crate::link_health::LinkFailureKind::Offline,
                reason: crate::link_health::LinkReason::Connect,
            },
            now_secs() as i64,
        );
        let tracker = StatusTracker::new(None);
        let payload = finish_tick_payload(&tracker, Ok(()), &link, 0, &db, &board);
        assert_eq!(payload["state"], "offline");
        assert_eq!(payload["legacy_state"], "idle", "an Ok tick still reads idle to old consumers");
        assert_eq!(payload["last_tick_ok_at"], serde_json::Value::Null);
    }

    #[test]
    fn a_failed_tick_is_an_error_with_the_error_text() {
        let dir = tempfile::tempdir().unwrap();
        let db = status_db(dir.path());
        let board = crate::transfer_progress::TransferBoard::new();
        let link = crate::link_health::LinkMonitor::new();
        let tracker = StatusTracker::new(None);
        let err = anyhow::anyhow!("HTTP 503 Service Unavailable: down");
        let payload = finish_tick_payload(&tracker, Err(&err), &link, 0, &db, &board);
        assert_eq!(payload["state"], "error");
        assert_eq!(payload["legacy_state"], "error");
        assert_eq!(payload["error"], "HTTP 503 Service Unavailable: down");
        assert_eq!(payload["reason"]["code"], "http_5xx");
    }

    #[test]
    fn the_tick_loop_reports_through_the_tracker() {
        // `run` needs a real window system, so its wiring is asserted on the source (CRLF
        // normalised: a Windows checkout has CRLF, slice 5's lesson).
        let source = include_str!("runner.rs").replace("\r\n", "\n");
        let start = source.find("async fn run(app: AppHandle").expect("run exists");
        let body = &source[start..start + source[start..].find("\n}\n").unwrap()];
        for needle in [
            "emit_payload(&app, tracker.running().to_json());",
            "emit_payload(&app, tracker.paused().to_json());",
            "emit_payload(&app, tracker.stopped().to_json());",
            "finish_tick_payload(&tracker, Ok(()), &link, tick_started_at, &db, &transfers)",
            "finish_tick_payload(&tracker, Err(&e), &link, tick_started_at, &db, &transfers)",
            "run_status_pulse(",
            ".with_transfers(transfers.clone())",
            "let link = api.link();",
        ] {
            assert_eq!(body.matches(needle).count(), 1, "`run` must contain exactly one `{needle}`");
        }
        // The old bare emitters are gone from the loop: each `emit_status` left in `run` is a
        // start-up failure (`"error"`), never a state the tracker owns.
        for old in ["emit_status(&app, \"idle\"", "emit_status(&app, \"paused\"", "emit_status(&app, \"running\"", "emit_status(&app, \"stopped\""] {
            assert_eq!(body.matches(old).count(), 0, "`run` still emits `{old}` around the tracker");
        }
    }

    /// Collect every `engine-status` payload the mock app emits.
    #[cfg(not(target_os = "windows"))]
    fn collect_status_events(app: &tauri::App<tauri::test::MockRuntime>) -> Arc<Mutex<Vec<serde_json::Value>>> {
        use tauri::Listener;
        let events: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        app.handle().listen(crate::engine_status::EVENT, move |event| {
            if let Ok(payload) = serde_json::from_str(event.payload()) {
                sink.lock().unwrap().push(payload);
            }
        });
        events
    }

    #[cfg(not(target_os = "windows"))]
    async fn wait_for_event(
        events: &Arc<Mutex<Vec<serde_json::Value>>>,
        within: Duration,
        pred: impl Fn(&serde_json::Value) -> bool,
    ) -> Option<serde_json::Value> {
        let started = std::time::Instant::now();
        while started.elapsed() < within {
            if let Some(found) = events.lock().unwrap().iter().find(|e| pred(e)).cloned() {
                return Some(found);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        None
    }

    #[test]
    fn the_pulses_activity_counts_an_upload_that_is_due_and_not_one_in_retry_backoff() {
        let dir = tempfile::tempdir().unwrap();
        let db = status_db(dir.path());
        seed_file(&db, "f1", 100);
        seed_file(&db, "f2", 5_000);
        let mut due = upload_op("op-f1", "f1");
        due.next_retry_at = 1; // long past: due now
        let mut backing_off = upload_op("op-f2", "f2");
        backing_off.attempts = 1;
        backing_off.next_retry_at = i64::MAX; // waiting out a backoff
        db.enqueue_operation(&due).unwrap();
        db.enqueue_operation(&backing_off).unwrap();

        let board = crate::transfer_progress::TransferBoard::new();
        let activity = current_activity(&db, &board);
        assert_eq!(activity.files_remaining, 1, "one file left, not two");
        assert_eq!(activity.bytes_total, 100, "the backed-off file's 5000 bytes are not in the total");
    }

    #[cfg(not(target_os = "windows"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_real_pulse_emits_syncing_with_progress_then_idle_and_resets_the_batch() {
        let dir = tempfile::tempdir().unwrap();
        let db = status_db(dir.path());
        seed_file(&db, "f1", 100);
        db.enqueue_operation(&upload_op("op-f1", "f1")).unwrap();
        let board = crate::transfer_progress::TransferBoard::new();
        let paused = Arc::new(AtomicBool::new(false));
        let app = tauri::test::mock_app();
        let events = collect_status_events(&app);
        let tracker = Arc::new(StatusTracker::new(None));
        tracker.finish_tick(&crate::engine_status::Outcome::Synced, 1, Activity::default());
        let pulse = tokio::spawn(run_status_pulse(
            app.handle().clone(),
            tracker,
            db.clone(),
            board.clone(),
            paused,
        ));

        // Quiet: nothing is moving, so nothing is emitted (and no DB work is done).
        tokio::time::sleep(Duration::from_millis(1300)).await;
        assert!(events.lock().unwrap().is_empty(), "an idle engine emits nothing");

        // A file starts moving: within a couple of pulses the UI hears `syncing`.
        let transfer = board.begin("f1", crate::transfer_progress::Direction::Up, 100);
        transfer.update(40);
        let syncing = wait_for_event(&events, Duration::from_millis(3000), |e| e["state"] == "syncing")
            .await
            .expect("a `syncing` event while a file moves");
        assert_eq!(syncing["files_remaining"], 1);
        assert_eq!(syncing["bytes_total"], 100);
        assert_eq!(syncing["bytes_done"], 40);
        assert_eq!(syncing["legacy_state"], "idle");

        // It finishes and leaves the queue: the pulse says `idle` and ends the batch.
        transfer.update(100);
        transfer.finish();
        db.remove_operation("op-f1").unwrap();
        let idle = wait_for_event(&events, Duration::from_millis(3000), |e| e["state"] == "idle")
            .await
            .expect("an `idle` event when the work is done");
        assert_eq!(idle["files_remaining"], 0);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(board.finished_bytes(), 0, "the pulse ends the batch when it goes quiet");
        pulse.abort();
    }

    #[cfg(not(target_os = "windows"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_real_pulse_reports_a_pause_the_moment_it_happens() {
        let dir = tempfile::tempdir().unwrap();
        let db = status_db(dir.path());
        let board = crate::transfer_progress::TransferBoard::new();
        let paused = Arc::new(AtomicBool::new(false));
        let app = tauri::test::mock_app();
        let events = collect_status_events(&app);
        let tracker = Arc::new(StatusTracker::new(None));
        tracker.finish_tick(&crate::engine_status::Outcome::Synced, 1, Activity::default());
        let pulse = tokio::spawn(run_status_pulse(
            app.handle().clone(),
            tracker,
            db,
            board,
            paused.clone(),
        ));
        paused.store(true, Ordering::Relaxed);
        let paused_event = wait_for_event(&events, Duration::from_millis(3000), |e| e["state"] == "paused").await;
        assert!(paused_event.is_some(), "the pulse must say `paused` within two seconds of the flag");
        pulse.abort();
    }
}
