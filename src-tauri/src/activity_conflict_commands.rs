//! `activity_conflict_commands` items split out of `lib.rs`.

use super::*;

pub(super) static APP_ACTIVITY_MONITOR: LazyLock<Mutex<AppActivityMonitor>> =
    LazyLock::new(|| Mutex::new(AppActivityMonitor::new()));

#[derive(Debug, Clone, serde::Serialize)]
pub(super) struct AppActivitySnapshot {
    pub(super) pid: u32,
    pub(super) process_name: String,
    pub(super) cpu_percent: f32,
    pub(super) memory_rss_bytes: u64,
    pub(super) upload_bps: Option<u64>,
    pub(super) download_bps: Option<u64>,
    pub(super) throughput_sample_age_ms: Option<u64>,
    pub(super) throughput_period_secs: Option<u32>,
}

#[derive(Debug, Clone)]
pub(super) struct ProcessResourceSnapshot {
    pub(super) pid: u32,
    pub(super) process_name: String,
    pub(super) cpu_percent: f32,
    pub(super) memory_rss_bytes: u64,
}

#[derive(Debug, Clone)]
pub(super) struct ThroughputSnapshot {
    pub(super) upload_bps: Option<u64>,
    pub(super) download_bps: Option<u64>,
    pub(super) sample_age_ms: Option<u64>,
    pub(super) period_secs: Option<u32>,
}

pub(super) struct AppActivityMonitor {
    pub(super) system: System,
    pub(super) pid: sysinfo::Pid,
    pub(super) logical_core_count: usize,
}

impl AppActivityMonitor {
    pub(super) fn new() -> Self {
        let pid = sysinfo::get_current_pid().unwrap_or_else(|_| sysinfo::Pid::from_u32(std::process::id()));
        let mut system = System::new();
        system.refresh_cpu_list(CpuRefreshKind::nothing());
        let logical_core_count = system.cpus().len().max(1);
        Self {
            system,
            pid,
            logical_core_count,
        }
    }

    pub(super) fn process_snapshot(&mut self) -> Result<ProcessResourceSnapshot, String> {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[self.pid]),
            true,
            ProcessRefreshKind::nothing().with_cpu().with_memory(),
        );

        let process = self
            .system
            .process(self.pid)
            .ok_or_else(|| format!("current process {} not found", self.pid))?;
        let cpu_percent = normalize_process_cpu_percent(process.cpu_usage(), self.logical_core_count);

        Ok(ProcessResourceSnapshot {
            pid: self.pid.as_u32(),
            process_name: process.name().to_string_lossy().into_owned(),
            cpu_percent,
            memory_rss_bytes: process.memory(),
        })
    }
}

pub(super) fn normalize_process_cpu_percent(raw_cpu_percent: f32, logical_core_count: usize) -> f32 {
    if !raw_cpu_percent.is_finite() {
        return 0.0;
    }

    let normalized = raw_cpu_percent.max(0.0) / logical_core_count.max(1) as f32;
    (normalized.clamp(0.0, 100.0) * 10.0).round() / 10.0
}

pub(super) fn bandwidth_sample_rates(up_bytes: u64, down_bytes: u64, period_secs: u32) -> (u64, u64) {
    if period_secs == 0 {
        return (0, 0);
    }
    (up_bytes / period_secs as u64, down_bytes / period_secs as u64)
}

pub(super) fn latest_app_activity_throughput_snapshot() -> Result<ThroughputSnapshot, String> {
    if DesktopConfig::load().ok().and_then(|cfg| cfg.sync_root).is_none() {
        return Ok(ThroughputSnapshot {
            upload_bps: None,
            download_bps: None,
            sample_age_ms: None,
            period_secs: None,
        });
    }

    let db_path = state_paths::beebeeb_state_dir()?.join(state_paths::STATE_DB_FILENAME);
    if !db_path.exists() {
        return Ok(ThroughputSnapshot {
            upload_bps: None,
            download_bps: None,
            sample_age_ms: None,
            period_secs: None,
        });
    }

    let db = state_db::StateDb::open(&db_path).map_err(|e| format!("open state db: {e}"))?;
    let Some(sample) = db
        .latest_bandwidth_sample()
        .map_err(|e| format!("latest bandwidth sample: {e}"))?
    else {
        return Ok(ThroughputSnapshot {
            upload_bps: None,
            download_bps: None,
            sample_age_ms: None,
            period_secs: None,
        });
    };

    let (upload_bps, download_bps) = bandwidth_sample_rates(sample.up_bytes, sample.down_bytes, sample.period_secs);
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let sampled_at_secs = if sample.sampled_at > 0 {
        sample.sampled_at as u64
    } else {
        0
    };

    Ok(ThroughputSnapshot {
        upload_bps: Some(upload_bps),
        download_bps: Some(download_bps),
        sample_age_ms: Some(now_secs.saturating_sub(sampled_at_secs).saturating_mul(1000)),
        period_secs: Some(sample.period_secs),
    })
}

#[tauri::command]
pub(super) async fn app_activity_snapshot() -> Result<AppActivitySnapshot, String> {
    let (process, throughput) = tokio::task::spawn_blocking(|| -> Result<_, String> {
        let process = APP_ACTIVITY_MONITOR
            .lock()
            .map_err(|_| "app activity monitor mutex poisoned".to_string())?
            .process_snapshot()?;
        let throughput = latest_app_activity_throughput_snapshot()?;
        Ok((process, throughput))
    })
    .await
    .map_err(|e| format!("app activity snapshot task failed: {e}"))??;

    Ok(AppActivitySnapshot {
        pid: process.pid,
        process_name: process.process_name,
        cpu_percent: process.cpu_percent,
        memory_rss_bytes: process.memory_rss_bytes,
        upload_bps: throughput.upload_bps,
        download_bps: throughput.download_bps,
        throughput_sample_age_ms: throughput.sample_age_ms,
        throughput_period_secs: throughput.period_secs,
    })
}

// ── IPC commands: Bandwidth view — speedtest + history (task 0810) ───────────

/// Result shape returned by [`run_speedtest`].
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SpeedtestResult {
    pub latency: SpeedtestLatency,
    /// Average upload speed in bytes per second.
    pub upload_bps: u64,
    /// Average download speed in bytes per second.
    pub download_bps: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SpeedtestLatency {
    pub min_ms: u64,
    pub avg_ms: u64,
    pub max_ms: u64,
}

/// What a run measured, kept apart from how it is said: only the pings that were answered and the transfers that
/// finished are in here. Failed ones are not zeros to average in (PR #140 review, P2-7).
#[derive(Debug, Default)]
pub(super) struct SpeedtestSamples {
    /// Round trips of the pings the server answered, in milliseconds.
    pub(super) ping_ms: Vec<u64>,
    /// Bytes per second of each upload that finished.
    pub(super) up_bps: Vec<f64>,
    /// Bytes per second of each download that finished.
    pub(super) down_bps: Vec<f64>,
}

pub(super) const SPEEDTEST_NO_PING: &str =
    "Beebeeb didn’t answer any of the 5 pings. Check your internet connection and try again.";
pub(super) const SPEEDTEST_NO_UPLOAD: &str =
    "None of the 3 uploads finished. Check your internet connection and try again.";
pub(super) const SPEEDTEST_NO_DOWNLOAD: &str =
    "None of the 3 downloads finished. Check your internet connection and try again.";
pub(super) const SPEEDTEST_ALREADY_RUNNING: &str = "A speed test is already running.";

/// A run's figures, or the plain reason there are none. A direction that never finished is not "0 Mbps" and an
/// unanswered ping is not a latency: a run without a figure in all three is a failed run, never a result.
pub(super) fn speedtest_outcome(samples: &SpeedtestSamples) -> Result<SpeedtestResult, String> {
    if samples.ping_ms.is_empty() {
        return Err(SPEEDTEST_NO_PING.to_string());
    }
    if samples.up_bps.is_empty() {
        return Err(SPEEDTEST_NO_UPLOAD.to_string());
    }
    if samples.down_bps.is_empty() {
        return Err(SPEEDTEST_NO_DOWNLOAD.to_string());
    }
    let mean = |speeds: &[f64]| (speeds.iter().sum::<f64>() / speeds.len() as f64) as u64;
    Ok(SpeedtestResult {
        latency: SpeedtestLatency {
            min_ms: *samples.ping_ms.iter().min().unwrap_or(&0),
            avg_ms: samples.ping_ms.iter().sum::<u64>() / samples.ping_ms.len() as u64,
            max_ms: *samples.ping_ms.iter().max().unwrap_or(&0),
        },
        upload_bps: mean(&samples.up_bps),
        download_bps: mean(&samples.down_bps),
    })
}

/// At most one speed test at a time: the test moves 60 MB, and a second press (the Settings page and the Bandwidth view
/// both offer one) would halve both figures. Dropping the guard frees the slot, including when the command is cancelled.
pub(super) struct SpeedtestSlot<'a>(&'a std::sync::atomic::AtomicBool);

impl<'a> SpeedtestSlot<'a> {
    pub(super) fn take(flag: &'a std::sync::atomic::AtomicBool) -> Result<Self, String> {
        flag.compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        )
        .map(|_| Self(flag))
        .map_err(|_| SPEEDTEST_ALREADY_RUNNING.to_string())
    }
}

impl Drop for SpeedtestSlot<'_> {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
}

pub(super) static SPEEDTEST_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Run a network speedtest against the Beebeeb API.
///
/// Parity with CLI (`repos/cli/src/commands/speedtest.rs`) and iOS
/// (`SpeedtestScreen`): 5 latency pings + 3 × 10 MB download + 3 × 10 MB upload.
/// Uses the authenticated session; returns `Err("vault is locked")` when none. A run in which nothing finished in one
/// of the three parts is an `Err` with the reason (see [`speedtest_outcome`]), and a second run while one is going is
/// refused.
#[tauri::command]
pub(super) async fn run_speedtest(state: State<'_, AppState>) -> Result<SpeedtestResult, String> {
    session_command!(async {
        let api = api_client_from_session(&state)?;
        let _slot = SpeedtestSlot::take(&SPEEDTEST_RUNNING)?;
        let mut samples = SpeedtestSamples::default();

        // Latency: 5 × GET /api/v1/health. A ping the server did not answer has no round trip to report.
        for _ in 0..5 {
            let start = std::time::Instant::now();
            if api.ping_health().await.is_ok() {
                samples.ping_ms.push(start.elapsed().as_millis() as u64);
            }
        }

        // Upload: 3 × 10 MB POST /api/v1/speedtest
        let blob = vec![0u8; 10_000_000];
        for _ in 0..3 {
            let start = std::time::Instant::now();
            if api.speedtest_upload(&blob).await.is_ok() {
                let elapsed = start.elapsed().as_secs_f64();
                if elapsed > 0.0 {
                    samples.up_bps.push(blob.len() as f64 / elapsed);
                }
            }
        }

        // Download: 3 × 10 MB GET /api/v1/speedtest?size=10000000
        for _ in 0..3 {
            let start = std::time::Instant::now();
            if let Ok(data) = api.speedtest_download(10_000_000).await {
                let elapsed = start.elapsed().as_secs_f64();
                if elapsed > 0.0 {
                    samples.down_bps.push(data.len() as f64 / elapsed);
                }
            }
        }

        speedtest_outcome(&samples)
    })
}

/// Return bandwidth samples from the local state DB for the traffic chart.
///
/// `hours` controls the window (default 20). The DB stores one sample per
/// heartbeat beat (~20 s), so a 20-hour window returns up to ~3600 samples —
/// the frontend downsamples to a fixed number of chart buckets for display.
#[tauri::command]
pub(super) async fn get_bandwidth_history(
    state: State<'_, AppState>,
    hours: Option<u32>,
) -> Result<Vec<state_db::BandwidthSample>, String> {
    session_command!(async {
        // Auth gate.
        {
            let acct = state.active_account()?;
            let guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
            if guard.is_none() {
                return Err("vault is locked".to_string());
            }
        }

        let window_hours = hours.unwrap_or(20);

        if DesktopConfig::load().ok().and_then(|cfg| cfg.sync_root).is_none() {
            return Ok(vec![]);
        }
        let db_path = state_paths::beebeeb_state_dir()?.join(state_paths::STATE_DB_FILENAME);
        if !db_path.exists() {
            return Ok(vec![]);
        }

        tokio::task::spawn_blocking(move || -> Result<_, String> {
            let db = state_db::StateDb::open(&db_path).map_err(|e| format!("open state db: {e}"))?;
            db.get_bandwidth_history(window_hours)
                .map_err(|e| format!("bandwidth history: {e}"))
        })
        .await
        .map_err(|e| format!("bandwidth history task failed: {e}"))?
    })
}

// ── IPC commands: Task 12 — conflict window ───────────────────────────────────

/// Internal helper that does the actual window-building work. Called
/// by both the [`open_conflict_window`] IPC command (when the user or
/// frontend triggers it) and by [`crate::runner`]'s tick loop (when
/// the engine bridge detects a fresh conflict in Task 10). Splitting
/// this out lets us call directly from Rust without going through the
/// IPC dispatcher; the IPC command is just a thin wrapper.
pub(crate) fn open_conflict_window_impl(
    app: &tauri::AppHandle,
    file_id: &str,
    file_name: &str,
    is_text: bool,
    op_id: Option<&str>,
) -> Result<(), String> {
    let label = format!("conflict-{file_id}");

    // If the user already has the window open for this file, just
    // bring it forward — re-creating would error from Tauri and the
    // user-visible behaviour ("show me this conflict again") is the
    // same.
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.show();
        let _ = existing.set_focus();
        return Ok(());
    }

    // Task 2190: a parked conflict carries its queued write, which the window decides.
    let op_param = op_id
        .map(|op| format!("&opId={}", urlencoding::encode(op)))
        .unwrap_or_default();
    let url = format!(
        "index.html?window=conflict&fileId={file_id}&fileName={enc_name}&isText={is_text}{op_param}",
        enc_name = urlencoding::encode(file_name),
    );

    tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::App(url.into()))
        .title(format!("Conflict — {file_name}"))
        .inner_size(720.0, 480.0)
        .resizable(true)
        .center()
        .build()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Open a conflict-resolution window for a single file. The label is
/// `conflict-<file_id>` so multiple simultaneous conflicts each get
/// their own window without colliding (re-opening the same conflict
/// is a soft-noop — show + focus instead of error).
///
/// The URL contract matches `repos/desktop/src/ConflictWindow.tsx`:
/// `index.html?window=conflict&fileId=<uuid>&fileName=<utf8>&isText=<bool>`.
/// `main.tsx` reads `?window=conflict` and mounts `ConflictWindow`
/// instead of the default compact app shell.
///
/// Spec: `docs/superpowers/plans/2026-05-07-desktop-sync-client.md`
/// Phase 4 Task 12. Also called internally by Task 10's conflict
/// detection path via [`open_conflict_window_impl`].
#[tauri::command]
pub(super) async fn open_conflict_window(
    app: tauri::AppHandle,
    file_id: String,
    file_name: String,
    is_text: bool,
    op_id: Option<String>,
) -> Result<(), String> {
    open_conflict_window_impl(&app, &file_id, &file_name, is_text, op_id.as_deref())
}

/// Show a native OS notification announcing a fresh conflict. Used
/// by `repos/desktop/src-tauri/src/runner.rs` when `sync_tick`
/// flags a divergent file, and exposed as an IPC command for the
/// settings page in case it ever wants to manually re-surface a
/// conflict that's been sitting in the queue.
///
/// We respect the user's `notify_conflicts` toggle in
/// `desktop.toml` (Task 9) — a `false` there short-circuits to Ok
/// without raising the OS notification.
///
/// Spec: Phase 4 Task 11.
pub(super) fn should_show_conflict_notification(config: Result<DesktopConfig, String>) -> bool {
    config.map(|c| c.notify_conflicts).unwrap_or(true)
}

pub(super) fn should_show_sync_complete_notification(config: Result<DesktopConfig, String>) -> bool {
    config.map(|c| c.notify_sync_complete).unwrap_or(false)
}

pub(super) fn should_show_quota_warning_notification(config: Result<DesktopConfig, String>) -> bool {
    config.map(|c| c.notify_quota_warnings).unwrap_or(true)
}

pub(crate) fn notify_conflict_impl(app: &tauri::AppHandle, file_name: &str) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;

    // Honour the user's preference. Best-effort load — if the config
    // file is unreadable we fall back to "show", matching the
    // documented Default of `notify_conflicts: true`.
    if !should_show_conflict_notification(DesktopConfig::load()) {
        return Ok(());
    }

    app.notification()
        .builder()
        .title("Conflict detected")
        .body(format!("Conflict in {file_name} — open Beebeeb to resolve"))
        .show()
        .map_err(|e| e.to_string())
}

pub(crate) fn notify_sync_complete_impl(app: &tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;

    if !should_show_sync_complete_notification(DesktopConfig::load()) {
        return Ok(());
    }

    app.notification()
        .builder()
        .title("All caught up")
        .body("Your Beebeeb files are up to date.")
        .show()
        .map_err(|e| e.to_string())
}

pub(crate) fn notify_quota_warning_impl(
    app: &tauri::AppHandle,
    local_cache_bytes: i64,
    local_cache_limit_bytes: i64,
) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;

    if local_cache_limit_bytes <= 0 {
        return Ok(());
    }
    if !should_show_quota_warning_notification(DesktopConfig::load()) {
        return Ok(());
    }

    let used = local_cache_bytes.max(0);
    let cap = local_cache_limit_bytes.max(1);
    let percent = ((used as f64 / cap as f64) * 100.0).round() as i64;

    app.notification()
        .builder()
        .title("Local cache nearly full")
        .body(format!(
            "{} used of {} local cache limit ({}%).",
            format_notification_bytes(used),
            format_notification_bytes(cap),
            percent
        ))
        .show()
        .map_err(|e| e.to_string())
}

pub(super) fn format_notification_bytes(bytes: i64) -> String {
    let bytes = bytes.max(0) as f64;
    if bytes < 1_000.0 {
        format!("{} B", bytes as i64)
    } else if bytes < 1_000_000.0 {
        format!("{:.0} KB", bytes / 1_000.0)
    } else if bytes < 1_000_000_000.0 {
        format!("{:.0} MB", bytes / 1_000_000.0)
    } else if bytes < 1_000_000_000_000.0 {
        format!("{:.0} GB", bytes / 1_000_000_000.0)
    } else {
        format!("{:.1} TB", bytes / 1_000_000_000_000.0)
    }
}

/// IPC wrapper around [`notify_conflict_impl`]. Mostly here so the
/// frontend can trigger a notification on demand (e.g. for a
/// "remind me later" UI we may add); the engine's auto-detection
/// path calls the impl directly.
#[tauri::command]
pub(super) async fn notify_conflict(app: tauri::AppHandle, file_name: String, file_id: String) -> Result<(), String> {
    let _ = file_id; // reserved for future click-to-open wiring
    notify_conflict_impl(&app, &file_name)
}

/// Apply the user's conflict resolution choice. `choice` is
/// `"local"` | `"remote"` | `"both"` per the TS-side button labels
/// in ConflictWindow.tsx (Keep Mine / Keep Theirs / Keep Both).
///
/// Routes each choice into the matching `EngineBridge` method:
///
///   - `"local"`  → [`engine_bridge::EngineBridge::resolve_keep_mine`]
///   - `"remote"` → [`engine_bridge::EngineBridge::resolve_keep_theirs`]
///   - `"both"`   → [`engine_bridge::EngineBridge::auto_resolve_keep_both`]
///
/// We build a fresh `EngineBridge` per call rather than reaching into
/// the long-lived runner. SQLite handles concurrent connections to the
/// same DB cleanly in WAL mode; reqwest is cheap to construct; and
/// avoiding a back-channel from the IPC layer into the runner task
/// keeps the dataflow obvious. The cost is one extra DB open per
/// resolve, which happens at user-click cadence — irrelevant.
#[tauri::command]
pub(super) async fn resolve_conflict(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    file_id: String,
    choice: String,
) -> Result<(), String> {
    session_command!(async {
        // Validate the choice up front so a typo on the TS side fails
        // loudly instead of silently no-opping.
        match choice.as_str() {
            "local" | "remote" | "both" => {}
            _ => return Err(format!("invalid conflict choice: {choice}")),
        }

        // Pull token + master_key out of the in-memory session. We
        // intentionally do NOT clone the entire Session struct — only
        // what the ApiClient needs.
        let acct = state.active_account()?;
        let (token, master_key) = {
            let guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
            match guard.as_ref() {
                Some(s) => (s.token.clone(), s.master_key),
                None => return Err("not signed in".into()),
            }
        };

        // Sync root from desktop.toml — without it the daemon has no
        // place to write the resolved file, so this is a hard error.
        let cfg = DesktopConfig::load()?;
        let sync_root = cfg.sync_root.ok_or_else(|| "no sync root configured".to_string())?;

        // Build a fresh bridge over the app-local state DB (same convention as
        // `runner::run`).
        let db_path = state_paths::beebeeb_state_dir()?.join(state_paths::STATE_DB_FILENAME);
        let db = std::sync::Arc::new(state_db::StateDb::open(&db_path).map_err(|e| format!("open state.db: {e}"))?);
        let api = std::sync::Arc::new(api_client::ApiClient::new(runner::api_base_url(), token, master_key));
        let bridge = engine_bridge::EngineBridge::new(db, api);

        let outcome: Result<(), String> = match choice.as_str() {
            "local" => bridge
                .resolve_keep_mine(&file_id, &sync_root)
                .await
                .map_err(|e| e.to_string()),
            "remote" => bridge
                .resolve_keep_theirs(&file_id, &sync_root)
                .await
                .map_err(|e| e.to_string()),
            "both" => {
                // auto_resolve_keep_both wants the FileEntry; look it up.
                let entry = bridge
                    .db()
                    .get_file(&file_id)
                    .map_err(|e| format!("get_file: {e}"))?
                    .ok_or_else(|| format!("no state.db row for {file_id}"))?;
                bridge
                    .auto_resolve_keep_both(&sync_root, &entry)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
            // Unreachable — the validate above already caught it.
            _ => unreachable!(),
        };
        outcome?;

        // Notify the app window so it can re-poll sync_status and
        // refresh the conflict count without waiting for the next engine
        // tick.
        let _ = app.emit(
            "conflict-resolved",
            serde_json::json!({
                "file_id": file_id,
                "choice": choice,
            }),
        );

        Ok(())
    })
}

/// Task 2190 (plan §3.1): a bridge over the app-local state DB for a parked conflict. Unlike
/// `resolve_conflict`, it needs no sync root: on a Mac a parked write's bytes are its staged copy.
pub(super) fn parked_conflict_bridge(state: &State<'_, AppState>) -> Result<engine_bridge::EngineBridge, String> {
    let acct = state.active_account()?;
    let (token, master_key) = {
        let guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
        match guard.as_ref() {
            Some(s) => (s.token.clone(), s.master_key),
            None => return Err("not signed in".into()),
        }
    };
    let db_path = state_paths::beebeeb_state_dir()?.join(state_paths::STATE_DB_FILENAME);
    let db = std::sync::Arc::new(state_db::StateDb::open(&db_path).map_err(|e| format!("open state.db: {e}"))?);
    let api = std::sync::Arc::new(api_client::ApiClient::new(runner::api_base_url(), token, master_key));
    Ok(engine_bridge::EngineBridge::new(db, api))
}

/// Task 2190 (plan §3.2, D1): "Keep the other" keeps the Mac's bytes in the "Changes not sent"
/// folder (spec 2026-10-10) FIRST: a new event folder, the copy under the file's path, and the copy
/// added to its OWN list durably (review-169 blocker: never the sign-out's one folder slot, never its
/// unfinished list), so Settings › Sync lists every copy until the person dismisses it. Any failure
/// removes what it made, and the decision changes nothing.
#[cfg(target_os = "macos")]
pub(super) fn keep_parked_copy(
    op_id: &str,
    source: &std::path::Path,
    rel_path: &str,
) -> anyhow::Result<std::path::PathBuf> {
    keep_parked_copy_at(
        op_id,
        source,
        rel_path,
        &sign_out_kept_root().map_err(|e| anyhow::anyhow!(e))?,
        &DesktopConfig::path().map_err(|e| anyhow::anyhow!(e))?,
        &sign_out::kept_writer::SystemClone,
        &config::RealConfigFs,
    )
}

/// [`keep_parked_copy`] with its folder, its config file and its file system steps given.
#[cfg(target_os = "macos")]
pub(super) fn keep_parked_copy_at(
    op_id: &str,
    source: &std::path::Path,
    rel_path: &str,
    root: &std::path::Path,
    config_path: &std::path::Path,
    clone: &dyn sign_out::kept_writer::CloneFile,
    config_fs: &dyn config::ConfigFs,
) -> anyhow::Result<std::path::PathBuf> {
    use sign_out::kept_names::{KeptRole, event_folder_name, kept_relative_path};
    let mut writer = sign_out::kept_writer::KeptWriter::create(root, &event_folder_name(chrono::Local::now()), clone)?;
    let kept = match writer.keep(&kept_relative_path(Some(rel_path), op_id, KeptRole::Newest), source) {
        Ok(kept) => kept,
        Err(error) => {
            writer.abandon();
            return Err(error.into());
        }
    };
    let event = writer.event().to_path_buf();
    let folder = match writer.finish() {
        Ok(folder) => folder,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&event);
            return Err(error.into());
        }
    };
    let kept_text = kept.to_string_lossy().into_owned();
    if let Err(error) = sign_out::kept_record::save_conflict_copy_durably(config_path, &kept_text, config_fs) {
        let _ = std::fs::remove_dir_all(&folder.path);
        return Err(anyhow::anyhow!("record the kept copy: {error}"));
    }
    Ok(kept)
}

/// Task 2190 (review-169 blocker): "Show in Finder" reveals only a copy the list holds.
pub(super) fn conflict_copy_reveal_target<'a>(listed: &[String], path: &'a str) -> Result<&'a str, String> {
    if listed.iter().any(|copy| copy == path) {
        Ok(path)
    } else {
        Err(sign_out::copy::K3_TOAST.to_string())
    }
}

/// Task 2190: every copy "Keep the other" kept, oldest first, for Settings › Sync.
#[tauri::command]
pub(super) fn kept_conflict_copies() -> Result<Vec<String>, String> {
    sign_out::kept_record::saved_conflict_copies_at(&DesktopConfig::path()?)
}

/// Task 2190: "Show in Finder" for one kept copy.
#[tauri::command]
pub(super) fn show_kept_conflict_copy(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let listed = sign_out::kept_record::saved_conflict_copies_at(&DesktopConfig::path()?)?;
    let target = conflict_copy_reveal_target(&listed, &path)?;
    tracing::info!("kept copy reveal requested");
    app.opener()
        .reveal_item_in_dir(target)
        .map_err(|_| sign_out::copy::K3_TOAST.to_string())
}

/// Task 2190: "Dismiss" forgets one kept copy from the list, never deletes it.
#[tauri::command]
pub(super) fn dismiss_kept_conflict_copy(app: tauri::AppHandle, path: String) -> Result<bool, String> {
    let dismissed = sign_out::kept_record::dismiss_saved_conflict_copy_at(&DesktopConfig::path()?, &path)?;
    emit_kept_folder_changed(&app);
    Ok(dismissed)
}

/// Task 2190 (plan §3.4): both sides of a parked conflict, for the conflict window. Read-only.
#[tauri::command]
pub(super) async fn parked_conflict_preview(
    state: State<'_, AppState>,
    op_id: String,
) -> Result<engine_bridge::ConflictContentPreview, String> {
    session_command!(async {
        let bridge = parked_conflict_bridge(&state)?;
        bridge.parked_conflict_preview(&op_id).await.map_err(|e| e.to_string())
    })
}

/// Task 2190 (plan §3.2): the person's decision on a parked conflict ("Keep this Mac's", "Keep the
/// other", "Keep both"). A decision that changed something signals the File Provider, so Finder
/// reads the item again; `Moved` changed nothing (the window asks the person to decide again).
#[tauri::command]
pub(super) async fn resolve_parked_conflict(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    op_id: String,
    choice: engine_bridge::ParkedChoice,
    shown_version: Option<i64>,
) -> Result<engine_bridge::ParkedResolution, String> {
    session_command!(async {
        let bridge = parked_conflict_bridge(&state)?;
        let file_id = bridge
            .db()
            .get_operation(&op_id)
            .ok()
            .flatten()
            .and_then(|op| op.file_id);
        let device = hostname::get()
            .map(|name| name.to_string_lossy().trim_end_matches(".local").to_string())
            .unwrap_or_else(|_| "this Mac".to_string());
        let hhmm = chrono::Local::now().format("%H.%M").to_string();
        #[cfg(target_os = "macos")]
        let mut keep_copy = |source: &std::path::Path, rel: &str| keep_parked_copy(&op_id, source, rel);
        #[cfg(not(target_os = "macos"))]
        let mut keep_copy = |_: &std::path::Path, _: &str| -> anyhow::Result<std::path::PathBuf> {
            anyhow::bail!("this decision is not available on this platform yet")
        };
        let done = bridge
            .resolve_parked(&op_id, choice, shown_version, &mut keep_copy, &device, &hhmm)
            .map_err(|e| e.to_string())?;
        // The signal goes through the bridge gate. One it does not deliver (the gate is busy, or the
        // signal fails) is owed to the next tick's signal, so Finder is never left stale.
        #[cfg(target_os = "macos")]
        if done != engine_bridge::ParkedResolution::Moved {
            let mut ids: Vec<String> = file_id.into_iter().collect();
            if let engine_bridge::ParkedResolution::KeptBoth { file_id: copy, .. } = &done {
                ids.push(copy.clone());
            }
            if crate::finder_setup::macos_ports::signal_working_set().is_err() {
                crate::runner::owe_working_set_signal(&ids);
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = file_id;
        // Review-169 #8: an open Sync tab reads its kept rows again (a new kept copy, a dropped write).
        if done != engine_bridge::ParkedResolution::Moved {
            emit_kept_folder_changed(&app);
        }
        let _ = app.emit("conflict-resolved", serde_json::json!({ "op_id": op_id }));
        Ok(done)
    })
}

/// Real content preview for the conflict-resolution window (task 1546 finding
/// 2), replacing ConflictWindow.tsx's previous hardcoded placeholder diff
/// body. Read-only — does not resolve anything or touch state.db; see
/// [`engine_bridge::EngineBridge::conflict_content_preview`] for why.
///
/// Same session/sync-root/bridge-construction pattern as `resolve_conflict`
/// just above.
#[tauri::command]
pub(super) async fn conflict_content_preview(
    state: State<'_, AppState>,
    file_id: String,
) -> Result<engine_bridge::ConflictContentPreview, String> {
    session_command!(async {
        let acct = state.active_account()?;
        let (token, master_key) = {
            let guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
            match guard.as_ref() {
                Some(s) => (s.token.clone(), s.master_key),
                None => return Err("not signed in".into()),
            }
        };

        let cfg = DesktopConfig::load()?;
        let sync_root = cfg.sync_root.ok_or_else(|| "no sync root configured".to_string())?;

        let db_path = state_paths::beebeeb_state_dir()?.join(state_paths::STATE_DB_FILENAME);
        let db = std::sync::Arc::new(state_db::StateDb::open(&db_path).map_err(|e| format!("open state.db: {e}"))?);
        let api = std::sync::Arc::new(api_client::ApiClient::new(runner::api_base_url(), token, master_key));
        let bridge = engine_bridge::EngineBridge::new(db, api);

        bridge
            .conflict_content_preview(&file_id, &sync_root)
            .await
            .map_err(|e| e.to_string())
    })
}

// ── IPC commands: app metadata ────────────────────────────────────────────────

// ── Durations ─────────────────────────────────────────────────────────────────

/// Delay after launch before the first update check (don't block startup).
pub(super) const UPDATE_CHECK_STARTUP_DELAY_SECS: u64 = 5;
/// How often to re-check for updates while the app is running.
pub(super) const UPDATE_CHECK_INTERVAL_HOURS: u64 = 4;
pub(super) const UPDATE_MANIFEST_BASE_URL: &str = "https://releases.beebeeb.io";
pub(super) const DESKTOP_RELEASE_TAG_PREFIX: &str = "desktop-v";
pub(super) const DESKTOP_RELEASE_REPO_URL: &str = "https://github.com/beebeeb-io/desktop/releases/tag";
