//! `file_open_commands` items split out of `lib.rs`.

use super::*;
pub(crate) use beebeeb_engine::path_safety::{is_contained, reject_unsafe_rel_path};

/// Runs `call` on tokio's blocking pool and awaits its answer. For work that blocks, such as the File Provider reads
/// that wait up to 3 s (`BRIDGE_GATE_WAIT`) for the bridge gate: a plain `#[tauri::command]` fn and a native-menu
/// action run on the main thread, where that wait would freeze every window, and inline in an async command it would
/// hold a runtime worker. A call that panicked answers `failed`: the panic's text can carry an OS message, so it goes
/// to the log only (lead ruling T1-4).
pub(super) async fn on_the_blocking_pool<T: Send + 'static>(
    failed: &'static str,
    call: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(call).await.unwrap_or_else(|error| {
        tracing::warn!(%error, failed, "a blocking task stopped before it answered");
        Err(failed.to_string())
    })
}

/// What "Open in Finder" answers when its work stopped before it answered (see [`on_the_blocking_pool`]).
pub(super) const OPEN_FOLDER_FAILED: &str = "Could not open the Beebeeb folder.";

// Native menu actions have no frontend status: resolve the configured root here.
pub(super) fn open_current_finder_location() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let path = None;
    #[cfg(not(target_os = "macos"))]
    let path = Some(get_sync_root()?.ok_or_else(|| "Not configured on this PC yet".to_string())?);
    open_finder_location_blocking(path)
}

/// "Open in Finder" (Explorer, or the file manager). `async`, with the work on the blocking pool: on macOS it reads
/// the Finder location through the bridge gate, which waits up to 3 s for a call in flight, and a plain
/// `#[tauri::command]` fn would make that wait on the main thread.
#[tauri::command]
pub(super) async fn open_finder_location(path: Option<String>) -> Result<(), String> {
    on_the_blocking_pool(OPEN_FOLDER_FAILED, move || open_finder_location_blocking(path)).await
}

/// The blocking body of [`open_finder_location`]. Its other callers: the menu's "Open in Finder" (spawned onto the
/// blocking pool), the upload action (inside a task), and the new-folder action's fallback when the new folder cannot
/// be revealed (on the main thread from the menu; on macOS only when the folder it just made cannot be resolved).
pub(super) fn open_finder_location_blocking(path: Option<String>) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        // Task 1885: never `open <path>` as a child. The child inherited the sandbox but not the extension on the
        // URL macOS returned, LaunchServices refused with -54, and `spawn()` had answered Ok. The bridge opens the
        // scoped URL itself and this returns what it answered.
        let _ = path;
        return file_provider_open_location();
    }

    #[cfg(not(target_os = "macos"))]
    {
        let root = require_open_folder_root(path)?;
        config::ensure_directory(&root)?;

        #[cfg(target_os = "windows")]
        {
            std::process::Command::new("explorer")
                .arg(&root)
                .spawn()
                .map_err(|e| format!("open Explorer: {e}"))?;
            return Ok(());
        }

        #[cfg(not(target_os = "windows"))]
        {
            std::process::Command::new("xdg-open")
                .arg(&root)
                .spawn()
                .map_err(|e| format!("open file manager: {e}"))?;
            Ok(())
        }
    }
}

/// What "Show in Finder" answers when its work stopped before it answered (see [`on_the_blocking_pool`]).
pub(super) const SHOW_IN_FINDER_FAILED: &str = "Could not show the file in your file manager.";

/// `async`, with the work on the blocking pool: on macOS it resolves the item's Finder location through the bridge
/// gate (up to 3 s of wait) and waits for the bridge's answer, which a plain `#[tauri::command]` fn would do on the
/// main thread.
#[tauri::command]
pub(super) async fn open_in_finder(item_id: String, path: Option<String>) -> Result<(), String> {
    on_the_blocking_pool(SHOW_IN_FINDER_FAILED, move || open_in_finder_blocking(item_id, path)).await
}

pub(super) fn open_in_finder_blocking(item_id: String, path: Option<String>) -> Result<(), String> {
    let _ = path;

    #[cfg(target_os = "macos")]
    {
        // Task 1885: the item's Finder location is the URL macOS returns for its File Provider identifier (the
        // state database's file id), revealed through NSWorkspace. It used to be `sync_root/<path>`, which on a Mac
        // is the app's private state folder and not a Finder location, opened with a child `open -R` whose failure
        // `spawn()` could not see. The identifier is checked the way every identifier off the wire is.
        ipc_socket::macos_validate_hydrate_item_identifier(&item_id).map_err(str::to_string)?;
        // Shared content is webapp-only by ruling 1701: it is not in Finder, so the File Provider is never asked about it
        // (macOS might fail the lookup, or place it under "Beebeeb"). Refused before any bridge call (task 1885, I3).
        // A provisional id whose create has landed is checked against the server file it became (its alias); the
        // reveal still asks for the id the caller holds.
        let db =
            state_db_for_config(&DesktopConfig::load()?)?.ok_or_else(|| finder_open::ITEM_NOT_KNOWN.to_string())?;
        finder_open::finder_may_show(&item_id, |id| finder_open::item_namespace(&db, id))?;
        file_provider_reveal_item(&item_id)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let cfg = DesktopConfig::load()?;
        let sync_root = cfg
            .sync_root
            .clone()
            .ok_or_else(|| "sync root not configured".to_string())?;
        let db = state_db_for_config(&cfg)?
            .ok_or_else(|| "The local sync database has not been created yet.".to_string())?;
        let entry = db
            .get_file(&item_id)
            .map_err(|e| format!("read shared item: {e}"))?
            .ok_or_else(|| "shared item is not available locally yet".to_string())?;

        reject_unsafe_rel_path(&entry.path)?;
        let local_path = sync_root.join(entry.path.replace('/', std::path::MAIN_SEPARATOR_STR));

        #[cfg(target_os = "windows")]
        {
            std::process::Command::new("explorer")
                .arg("/select,")
                .arg(&local_path)
                .spawn()
                .map_err(|e| format!("open Explorer: {e}"))?;
            Ok(())
        }

        #[cfg(not(target_os = "windows"))]
        {
            let open_path = if local_path.is_file() {
                local_path.parent().unwrap_or(&sync_root)
            } else {
                local_path.as_path()
            };
            std::process::Command::new("xdg-open")
                .arg(open_path)
                .spawn()
                .map_err(|e| format!("open file manager: {e}"))?;
            Ok(())
        }
    }
}

// ── reveal_and_open_file — security helpers ───────────────────────────────────
//
// These helpers are NOT cfg-gated so they compile and are testable on any
// platform (the host CI runs on Linux). Each helper carries an allow(dead_code)
// attribute that fires only on non-Windows platforms where the helpers are not
// called from a #[cfg(target_os="windows")] block; under `cargo test` (which
// compiles the test module) they are referenced and the allow never fires.

/// Layer 3: Executable denylist for the OPEN step.
///
/// Returns true when the file's extension (case-insensitive) is a type that
/// Windows will execute when opened with `ShellExecute` / `open_path`.
/// The REVEAL step (explorer /select) is safe for any contained path; this
/// guard only blocks the `open_path` call.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(super) fn is_dangerous_executable(path: &std::path::Path) -> bool {
    const DANGEROUS_EXTS: &[&str] = &[
        "exe", "bat", "cmd", "com", "scr", "lnk", "url", "hta", "ps1", "vbs", "vbe", "js", "jse", "wsf", "wsh", "msi",
        "reg", "cpl", "pif", "msc", "jar",
    ];
    let Some(ext) = path.extension() else {
        return false;
    };
    let ext_lower = ext.to_string_lossy().to_lowercase();
    DANGEROUS_EXTS.contains(&ext_lower.as_str())
}

/// Open the folder containing a recently-changed file in Windows Explorer
/// (selecting the file) and then open the file with its default application.
///
/// `rel_path` is the path as stored in the state DB — relative to the sync
/// root, using the same separator convention as the DB (forward-slash or
/// back-slash; both are normalised below). The full path is `sync_root/rel`.
///
/// Windows only. On other platforms this returns `Err("unsupported")` so
/// the binary still compiles cross-platform.
///
/// Zero-knowledge: the file path is NOT logged — only a fixed message is.
///
/// Security: four layers of defense against path-traversal / RCE:
///   1. `reject_unsafe_rel_path` — pure string checks before any path join.
///   2. `is_contained` — canonicalization containment after join.
///   3. Fallback containment — parent-dir canonicalization when the file
///      doesn't exist yet (cloud placeholder not yet hydrated).
///   4. `is_dangerous_executable` — executable denylist on the OPEN step only.
#[tauri::command]
pub(super) fn reveal_and_open_file(app: tauri::AppHandle, rel_path: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use tauri_plugin_opener::OpenerExt;

        // ── Layer 1: reject traversal before any path operations ──────────────
        reject_unsafe_rel_path(&rel_path)?;

        let sync_root = DesktopConfig::load()
            .ok()
            .and_then(|c| c.sync_root)
            .ok_or_else(|| "sync root not configured".to_string())?;

        // Normalise separators and resolve the full path.
        let rel = rel_path.replace('/', std::path::MAIN_SEPARATOR_STR);
        let full_path = sync_root.join(&rel);

        // ── Layer 2: canonicalization containment ─────────────────────────────
        // If the file exists, canonicalize both root and full_path and verify.
        // If the file doesn't exist yet (cloud placeholder not materialized),
        // fall back to verifying the parent directory is contained AND the
        // file name is a single normal component with no embedded separators.
        let contained = if full_path.exists() {
            is_contained(&sync_root, &full_path)
        } else {
            // Fallback: verify parent is contained.
            let parent_ok = full_path.parent().map(|p| is_contained(&sync_root, p)).unwrap_or(false);
            // And the file name component must not contain any separator.
            let name_ok = full_path
                .file_name()
                .map(|n| {
                    let s = n.to_string_lossy();
                    !s.contains('/') && !s.contains('\\')
                })
                .unwrap_or(false);
            parent_ok && name_ok
        };

        if !contained {
            tracing::warn!("reveal_and_open_file: path rejected by containment check");
            return Err("path is outside sync root".to_string());
        }

        // ── (a) REVEAL — always safe for any contained path ───────────────────
        tracing::info!("reveal_and_open_file: launching Explorer");
        std::process::Command::new("explorer")
            .arg("/select,")
            .arg(&full_path)
            .spawn()
            .map_err(|e| format!("reveal in Explorer: {e}"))?;

        // ── (b) OPEN — blocked for executable file types (Layer 3) ───────────
        if is_dangerous_executable(&full_path) {
            // Zero-knowledge: no path in the log.
            tracing::warn!("reveal_and_open_file: open blocked for executable file type");
            return Ok(());
        }

        tracing::info!("reveal_and_open_file: opening file with default app");
        let path_str = full_path.to_string_lossy().into_owned();
        if let Err(e) = app.opener().open_path(path_str, None::<&str>) {
            tracing::warn!(error = %e, "reveal_and_open_file: failed to open file with default app");
        }

        return Ok(());
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (app, rel_path);
        Err("unsupported".to_string())
    }
}

/// What [`sync_status`] counts in the state DB: rows moving (downloading + uploading), cloud-only rows (files and
/// folders, as before task 2243), conflicts, and changes held back.
pub(super) fn sync_status_counts(db: &state_db::StateDb) -> (u32, u32, u32, u32) {
    use state_db::FileStatus;
    // Downloading + Uploading both count toward "in-flight sync work" for the WebView's syncing pill: there is no
    // user-meaningful difference between "we're pulling" and "we're pushing" at the indicator level.
    let count = |status| db.count_by_status(status).unwrap_or(0);
    let syncing = count(FileStatus::Downloading) + count(FileStatus::Uploading);
    // Desktop 0.8.15: queued changes held back (the `auth` pause) are not syncing; the UI shows them as
    // "N changes on hold".
    let on_hold = db.count_operations_on_hold().unwrap_or(0);
    (
        syncing,
        count(FileStatus::CloudOnly),
        count(FileStatus::Conflict),
        on_hold,
    )
}

/// Lightweight status endpoint for the WebView's settings page.
/// Reports whether the Rust side has a session, where the sync root
/// is on disk, whether the engine task is currently running, and the
/// per-status file counts the Status / SyncFolder pages render.
///
/// For richer state (per-tick `idle` / `syncing` transitions) the
/// WebView should still listen to the `engine-status` Tauri event —
/// this command is the first-paint poll the settings UI uses while
/// the event stream is still ramping up.
///
/// File counts are read from the app-local SQLite state DB via a fresh
/// connection per call, so the page cache starts empty each time: every
/// count must come from a count query through an index, never from loading
/// rows. Task 2243: counting the cloud-only files by loading them cost about
/// 1,100 page reads per call on a 43k-row DB, polled every few seconds by
/// every window ([`sync_status_counts`]). If the DB doesn't exist yet (first
/// launch, sync root not picked, or DB not initialised because no engine has
/// run yet) we return zeros.
///
/// `session_only` (task 2243): only `logged_in` and `session_revision`, read from
/// memory, with no config file and no state DB. `AccountSessionBoundary` polls
/// this every second in every window, hidden ones included, to see a sign-out.
#[tauri::command]
pub(super) async fn sync_status(
    state: State<'_, AppState>,
    session_only: Option<bool>,
) -> Result<serde_json::Value, String> {
    // session / engine / engine_state are per-account (decision 0800);
    // `auth_present` stays on `AppState`.
    let acct = state.active_account()?;
    let vault_unlocked = acct.session.lock().map(|g| g.is_some()).unwrap_or(false);
    let auth_present = state.auth_present.lock().map(|g| *g).unwrap_or(false);
    let logged_in = vault_unlocked || auth_present;
    if session_only == Some(true) {
        return Ok(serde_json::json!({
            "logged_in": logged_in,
            "session_revision": UI_SESSION_REVISION.load(std::sync::atomic::Ordering::SeqCst),
        }));
    }
    // Task 1546 finding 5: 3+ consecutive 401s from the engine's heartbeat /
    // sync-tick calls flip this — the trigger for the persistent "signed out
    // on this device" banner. Explicit and independent of `logged_in`: the
    // stale token can still be installed (so `logged_in` reads `true`) while
    // every call it makes fails.
    let auth_expired = acct.auth_health.is_expired();
    let sync_root_path = DesktopConfig::load().ok().and_then(|c| c.sync_root);
    #[cfg(target_os = "macos")]
    let sync_root = None::<String>;
    #[cfg(not(target_os = "macos"))]
    let sync_root = sync_root_path.as_ref().map(|p| p.to_string_lossy().into_owned());
    let engine_running = acct.engine.lock().await.is_some();

    // Collapse the runner's five-state stream into the tri-state
    // string the WebView's settings pages render. `idle` and
    // `syncing` both mean "engine is alive and doing work" — UI
    // wants a single "running" pill for either; the per-tick
    // distinction is on the engine-status event stream.
    let engine = {
        let raw = acct
            .engine_state
            .lock()
            .map(|g| g.clone())
            .unwrap_or_else(|_| "stopped".to_string());
        let engine = engine_status::sync_status_engine(raw.as_str());
        if logged_in && !vault_unlocked && engine == "stopped" {
            "locked".to_string()
        } else {
            engine.to_string()
        }
    };

    // Count files by status, opening a short-lived connection at the canonical
    // app-local state.db path. No configured root / no DB degrades to zeros.
    let counts = if sync_root_path.is_some() {
        let db_path = state_paths::beebeeb_state_dir()?.join(state_paths::STATE_DB_FILENAME);
        if db_path.exists() {
            state_db::StateDb::open(&db_path)
                .ok()
                .map(|db| sync_status_counts(&db))
                .unwrap_or((0, 0, 0, 0))
        } else {
            (0, 0, 0, 0)
        }
    } else {
        (0, 0, 0, 0)
    };

    Ok(serde_json::json!({
        "logged_in": logged_in,
        "session_revision": UI_SESSION_REVISION.load(std::sync::atomic::Ordering::SeqCst),
        "auth_expired": auth_expired,
        "sync_root": sync_root,
        "engine_running": engine_running,
        "vault_unlocked": vault_unlocked,
        "engine": engine,
        // Why sync did not start, when the local-data binding refused it (ruling P): `{code, sentence}` or null.
        "engine_refusal": engine_refusal_view(&acct),
        "syncing": counts.0,
        "cloud_only": counts.1,
        "conflicts": counts.2,
        // Desktop 0.8.15: changes held back, never counted in `syncing`.
        "changes_on_hold": counts.3,
        // Desktop 0.8.15: the start-up guard's pass, `{running, done, total}` (Windows; at rest
        // elsewhere). While it runs, uploads and downloads flow; only freeing work waits.
        "startup_guard": crate::unknown_check::STARTUP_GUARD.progress(),
        // WS1 — storage byte totals for the tray status line.
        // These are `null` here because polling the billing API on every
        // 3-second status tick would hammer the server. The tray's
        // storage line should call `desktop_storage_summary` (which already
        // exists and hits the API once on demand) rather than piggy-backing
        // on the lightweight `sync_status` poll. Field names are reserved
        // here so the frontend contract is stable once wired.
        "used_bytes": serde_json::Value::Null,
        "quota_bytes": serde_json::Value::Null,
    }))
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct BillingUsageResponse {
    pub(super) used_bytes: i64,
    pub(super) quota_bytes: i64,
}

#[derive(Debug, serde::Serialize)]
pub(super) struct DesktopStorageSummary {
    pub(super) used_bytes: i64,
    pub(super) quota_bytes: i64,
    pub(super) cache_bytes: i64,
    pub(super) pinned_bytes: i64,
}

/// Storage summary for the control center.
///
/// Remote usage/quota comes from the API's billing usage endpoint. Local
/// cache/pinned bytes are read from the desktop SQLite state DB so the UI can
/// distinguish account storage from bytes actually present on this Mac.
#[tauri::command]
pub(super) async fn desktop_storage_summary(state: State<'_, AppState>) -> Result<DesktopStorageSummary, String> {
    session_command!(async {
        let acct = state.active_account()?;
        let token = {
            let guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
            guard
                .as_ref()
                .map(|session| session.token.clone())
                .ok_or_else(|| "vault is locked".to_string())?
        };

        let storage_client = reqwest::Client::builder()
            .default_headers(api_client::provenance_headers())
            .build()
            .map_err(|e| format!("reqwest build: {e}"))?;
        let usage: BillingUsageResponse = storage_client
            .get(format!("{}/api/v1/billing/usage", runner::api_base_url()))
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| format!("load storage usage: {e}"))?
            .error_for_status()
            .map_err(|e| format!("load storage usage: {e}"))?
            .json()
            .await
            .map_err(|e| format!("parse storage usage: {e}"))?;

        let (cache_bytes, pinned_bytes) = if DesktopConfig::load().ok().and_then(|cfg| cfg.sync_root).is_some() {
            let db_path = state_paths::beebeeb_state_dir()?.join(state_paths::STATE_DB_FILENAME);
            if db_path.exists() {
                let values = (|| {
                    let db = state_db::StateDb::open(&db_path).ok()?;
                    let pinned = db.cache_bytes_by_effective_pin(true).ok()?.max(0);
                    let unpinned = db.cache_bytes_by_effective_pin(false).ok()?.max(0);
                    Some((unpinned, pinned))
                })();
                values.unwrap_or((0, 0))
            } else {
                (0, 0)
            }
        } else {
            (0, 0)
        };

        let used_bytes = usage.used_bytes.max(0);
        let quota_bytes = usage.quota_bytes.max(0);

        // Feed the Windows Explorer breadcrumb status-flyout snapshot with the same
        // usage/quota numbers the control center shows, so the flyout's storage bar
        // ("X GB used of Y (Z%)") stays in agreement. Cheap atomic stores; the COM
        // `GetStatusUI` reads them without a lock. No-op on non-Windows.
        #[cfg(target_os = "windows")]
        windows_cf::status_ui::set_quota(used_bytes as u64, quota_bytes as u64);

        Ok(DesktopStorageSummary {
            used_bytes,
            quota_bytes,
            cache_bytes,
            pinned_bytes,
        })
    })
}

/// Everything the menu-bar popover needs for its first frame, in one call (task 1683
/// slice 2, spec section 9). The popover calls it when it is shown (the `popover-shown`
/// event) and after its own actions; the macOS main window reads it a few seconds after
/// each read while it is open (slice 1).
///
/// Reads, in this order: the session (locked / signed out / session ended), the live
/// pause flag, the engine's last `engine-status` (real state, progress, last
/// successful check, reason), the Finder install state (macOS only), the storage
/// summary (cached for five minutes, refetched after an unlock, never when the vault
/// is locked; a refetch holds this call for `popover_data::STORAGE_WAIT` at most and
/// a failed one is not tried again for `STORAGE_FAILURE_BACKOFF_SECS`, task 2179), and
/// the state DB (queue, recent transfers, conflicts). File names are read only while
/// the vault is unlocked, like `desktop_file_overview`.
#[tauri::command]
pub(super) async fn popover_snapshot(
    state: State<'_, AppState>,
    runtime: State<'_, popover_data::PopoverRuntime>,
    activity_limit: Option<usize>,
) -> Result<popover_data::PopoverSnapshotDto, String> {
    // Windows admission: the snapshot reads the session token, so it leases like every
    // credential command (tests/windows_session_wiring.rs guards this).
    session_command!(async {
        use popover_data::{SnapshotInputs, cached_storage, fetch_storage_usage};

        let now = now_unix_seconds();
        let acct = state.active_account()?;
        let (vault_unlocked, token, session_email) = {
            let guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
            (
                guard.is_some(),
                guard.as_ref().map(|s| s.token.clone()),
                guard.as_ref().and_then(|s| s.email.clone()),
            )
        };
        let auth_present = state.auth_present.lock().map(|g| *g).unwrap_or(false);
        let logged_in = vault_unlocked || auth_present;
        let auth_expired = acct.auth_health.is_expired();
        let paused = acct.sync_paused.load(std::sync::atomic::Ordering::Relaxed);
        let email = session_email.or_else(|| acct.auth_email.lock().ok().and_then(|guard| guard.clone()));

        // Finder: the reconciler's published view (spec 2026-10-06). It never asks the OS here.
        let (finder, finder_reason) = if cfg!(target_os = "macos") {
            match state.finder_setup.get().map(|handle| handle.view()) {
                Some(view) => (view.setup, view.reason),
                None => (crate::surfaces::phase::FinderSetup::Missing, None),
            }
        } else {
            (crate::surfaces::phase::FinderSetup::Ready, None)
        };

        // The storage wait (at most `STORAGE_WAIT`) and the state DB read run side by side, not one after the other
        // (#139 review P3-10).
        let storage = async {
            match token {
                Some(token) if vault_unlocked => {
                    cached_storage(
                        &state.popover_storage,
                        now,
                        VAULT_EPOCH.load(std::sync::atomic::Ordering::SeqCst),
                        || session_bound(async move { fetch_storage_usage(&runner::api_base_url(), &token).await }),
                    )
                    .await
                }
                _ => None,
            }
        };
        let limit = activity_limit.unwrap_or(popover_data::DEFAULT_ACTIVITY_LIMIT);
        let db_view = async {
            if !vault_unlocked {
                return Ok(None);
            }
            tokio::task::spawn_blocking(move || -> Result<_, String> {
                let Some(db) = DesktopConfig::load().and_then(|cfg| state_db_for_config(&cfg))? else {
                    return Ok(None);
                };
                popover_data::gather_db_view(&db, limit, now).map(Some)
            })
            .await
            .map_err(|e| format!("popover data task failed: {e}"))?
        };
        let (storage, db_view) = tokio::join!(storage, db_view);
        let db_view = db_view?;
        // The transfers as of now, after the wait (a file moving now is syncing, `assemble`).
        let board = runtime.transfers.active();
        let popover_data::DbView {
            backlog,
            conflicts,
            in_flight,
            failed,
            recent,
        } = db_view.unwrap_or_default();

        let engine = runtime.status.lock().map(|view| view.clone()).unwrap_or_default();
        let mut snapshot = popover_data::assemble(SnapshotInputs {
            now,
            logged_in,
            vault_unlocked,
            auth_expired,
            paused,
            email,
            engine,
            finder,
            finder_reason,
            finder_failure_elsewhere: false,
            storage,
            backlog,
            conflicts,
            in_flight,
            failed,
            recent,
            board,
            activity_limit: limit,
        });
        snapshot.update = macos_popover::update_offer();
        #[cfg(target_os = "macos")]
        {
            let sending = state
                .sign_out
                .phase
                .lock()
                .map(|phase| phase.is_some())
                .unwrap_or(false);
            if sending {
                let read = tokio::task::spawn_blocking(read_waiting_changes)
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|read| read);
                snapshot.sign_out_sending = Some(macos_popover::Sending {
                    changes_left: macos_popover::changes_left(read),
                });
            }
        }
        Ok(snapshot)
    })
}

// `pub(crate)` so `free_up_space_blocking` (also `pub(crate)`, reused by the
// Windows shell status-flyout command) can name it as its return type.

/// "Free up space now" — dehydrate every unpinned, fully-local file back to a
/// cloud-only placeholder and delete its on-disk cache copy. Pinned files
/// (effective pin = pinned) are always kept. Mirrors the Windows "Free up
/// space" action and the macOS Storage panel button.
///
/// Returns the number of bytes reclaimed from disk. Safe to call with no sync
/// root / no state DB (returns `bytes_freed: 0`).
///
/// `bytes_freed` is summed from the actual on-disk sizes of the cache files we
/// successfully delete — so the figure reflects bytes really removed from disk,
/// never an estimate. The DB transition (cache_path → NULL, status →
/// cloud_only) is done by `evict_unpinned_cache_until_under(0, now)`. File
/// deletes are bounded to paths under EITHER the sync root OR the engine's
/// system cache dir (`dirs::cache_dir()/beebeeb`) — the two locations the
/// engine writes cache copies — so a corrupt cache_path can never make us
/// delete outside the vault's cache.
///
/// The DB + filesystem work runs on a blocking thread (`spawn_blocking`) so the
/// IPC command thread isn't held by SQLite + `remove_file` syscalls.
#[tauri::command]
pub(super) async fn free_up_space() -> Result<FreeUpSpaceResult, String> {
    tokio::task::spawn_blocking(|| free_up_space_blocking(crate::APP))
        .await
        .map_err(|e| format!("free_up_space task join: {e}"))?
}

// The reclaim itself is the engine's (`beebeeb_engine::free_up_space`): the Explorer flyout calls it too.
pub(crate) use beebeeb_engine::free_up_space::{
    FreeUpSpaceResult, free_up_space_allowed_roots, free_up_space_blocking,
};

/// Version of the support-bundle shape. 2 = task 1685: `queue.last_error` is an
/// allow-list-filtered message plus `last_error_code` and `last_error_redactions`.
/// Paths and every file/folder name the state DB knows are removed. Residual: a
/// name the DB does not know that is a standard error word or digits only is
/// kept, and UUIDs and the URL scheme/host/port are kept on purpose.
pub(super) const DIAGNOSTICS_FORMAT: u32 = 2;

#[tauri::command(async)]
pub(super) fn export_diagnostics() -> Result<serde_json::Value, String> {
    let cfg = DesktopConfig::load()?;
    let bundle = match cfg.sync_root {
        None => export_diagnostics_from(None, Path::new(""))?,
        Some(root) => {
            let db_path = state_paths::beebeeb_state_dir()?.join(state_paths::STATE_DB_FILENAME);
            export_diagnostics_from(Some(&root), &db_path)?
        }
    };
    Ok(with_lifecycle_tail(
        bundle,
        lifecycle_log::tail(lifecycle_log::TAIL_LINES),
    ))
}

/// Spec 2026-10-06 §8: the support bundle carries the lifecycle log's last 200 lines (already
/// redacted when written). An additive key: `diagnostics_format` is unchanged. Empty on Windows
/// and Linux, where nothing writes the log.
pub(super) fn with_lifecycle_tail(mut bundle: serde_json::Value, tail: Vec<String>) -> serde_json::Value {
    if let Some(map) = bundle.as_object_mut() {
        map.insert("lifecycle_log".to_string(), serde_json::json!(tail));
    }
    bundle
}

/// Builds the support-bundle JSON. Separate from the command so the real export
/// path (including redaction) is unit-testable against a temp state DB.
pub(super) fn export_diagnostics_from(sync_root: Option<&Path>, db_path: &Path) -> Result<serde_json::Value, String> {
    let Some(sync_root) = sync_root else {
        return Ok(serde_json::json!({
            "diagnostics_format": DIAGNOSTICS_FORMAT,
            "sync_root_configured": false,
            "queue": serde_json::Value::Null,
        }));
    };
    if !db_path.exists() {
        return Ok(serde_json::json!({
            "diagnostics_format": DIAGNOSTICS_FORMAT,
            "sync_root_configured": true,
            "state_db_exists": false,
            "queue": serde_json::Value::Null,
        }));
    }
    let db = state_db::StateDb::open(db_path).map_err(|e| format!("open state.db: {e}"))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // The sync-root path (and so the account's home folder name) is a name too.
    let extra_paths = vec![sync_root.to_string_lossy().into_owned()];
    let queue = db
        .queue_diagnostics_with_paths(now, &extra_paths)
        .map_err(|e| format!("queue diagnostics: {e}"))?;
    Ok(serde_json::json!({
        "diagnostics_format": DIAGNOSTICS_FORMAT,
        "sync_root_configured": true,
        "state_db_exists": true,
        "queue": queue,
    }))
}

// ── IPC commands: conflict/version center ───────────────────────────────────

#[tauri::command]
pub(super) fn list_version_conflict_center() -> Result<Vec<engine_bridge::VersionConflictEntry>, String> {
    let cfg = DesktopConfig::load()?;
    let Some(_) = cfg.sync_root else {
        return Ok(Vec::new());
    };
    let db_path = state_paths::beebeeb_state_dir()?.join(state_paths::STATE_DB_FILENAME);
    if !db_path.exists() {
        return Ok(Vec::new());
    }
    let db = state_db::StateDb::open(&db_path).map_err(|e| format!("open state.db: {e}"))?;
    engine_bridge::version_conflict_feed_from_db(&db).map_err(|e| e.to_string())
}

/// Task 1640 (review N9): dismiss one informational review entry (`kept:<id>`, a kept copy). The
/// copy itself stays where it is.
#[tauri::command]
pub(super) fn dismiss_review_entry(entry_id: String) -> Result<(), String> {
    let Some(id) = entry_id.strip_prefix("kept:").and_then(|id| id.parse::<i64>().ok()) else {
        return Err("This entry can’t be dismissed.".to_string());
    };
    let db_path = state_paths::beebeeb_state_dir()?.join(state_paths::STATE_DB_FILENAME);
    let db = state_db::StateDb::open(&db_path).map_err(|e| format!("open state.db: {e}"))?;
    match db.dismiss_kept_copy(id) {
        Ok(true) => Ok(()),
        Ok(false) => Err("This entry can be dismissed once the file is closed and removed.".to_string()),
        Err(e) => Err(format!("dismiss: {e}")),
    }
}

pub(super) fn file_versions_payload_for_frontend(body: serde_json::Value) -> Result<serde_json::Value, String> {
    let Some(versions) = body.get("versions") else {
        return Err("list_file_versions response missing versions array".to_string());
    };
    if !versions.is_array() {
        return Err("list_file_versions response versions field is not an array".to_string());
    }
    Ok(body)
}
