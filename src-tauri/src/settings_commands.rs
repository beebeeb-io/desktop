//! `settings_commands` items split out of `lib.rs`.

use super::*;

pub(super) fn queued_restore_version_response(
    file_id: &str,
    version_id: &str,
    op: &state_db::PendingOperation,
) -> serde_json::Value {
    serde_json::json!({
        "status": "queued",
        "file_id": file_id,
        "version_id": version_id,
        "operation_id": op.op_id,
        "operation": "restore_version",
    })
}

#[tauri::command]
pub(super) async fn list_file_versions(state: State<'_, AppState>, file_id: String) -> Result<serde_json::Value, String> {
    session_command!(async {
        let acct = state.active_account()?;
        let (token, master_key) = {
            let guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
            match guard.as_ref() {
                Some(s) => (s.token.clone(), s.master_key),
                None => return Err("not signed in".into()),
            }
        };

        let api = api_client::ApiClient::new(runner::api_base_url(), token, master_key);
        let body = api.list_versions(&file_id).await.map_err(|e| e.to_string())?;
        file_versions_payload_for_frontend(body)
    })
}

#[tauri::command]
pub(super) async fn restore_file_version(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    file_id: String,
    version_id: String,
) -> Result<serde_json::Value, String> {
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
        if cfg.sync_root.is_none() {
            return Err("no sync root configured".to_string());
        }
        let db_path = state_paths::beebeeb_state_dir()?.join(state_paths::STATE_DB_FILENAME);
        let db = std::sync::Arc::new(state_db::StateDb::open(&db_path).map_err(|e| format!("open state.db: {e}"))?);
        let api = std::sync::Arc::new(api_client::ApiClient::new(runner::api_base_url(), token, master_key));
        let bridge = engine_bridge::EngineBridge::new(db, api);
        let op = bridge
            .queue_restore_version(&file_id, &version_id, None)
            .map_err(|e| format!("queue restore version: {e}"))?;
        let body = queued_restore_version_response(&file_id, &version_id, &op);
        let _ = app.emit(
            "version-center-review",
            serde_json::json!({
                "file_id": file_id,
                "version_id": version_id,
                "kind": "restore",
                "operation_id": op.op_id,
            }),
        );
        Ok(body)
    })
}

// ── IPC commands: sync-root config ────────────────────────────────────────────

/// Read the persisted sync root from `~/.config/beebeeb/desktop.toml`.
/// Returns `None` if the user hasn't picked one yet (first launch).
#[tauri::command]
pub(super) fn get_sync_root() -> Result<Option<String>, String> {
    let cfg = DesktopConfig::load()?;
    Ok(cfg.sync_root.map(|p| p.to_string_lossy().into_owned()))
}

/// Open the native folder picker so the user can choose a sync root.
/// Pre-points the dialog at `~/Beebeeb` (creates that directory first
/// if missing) so accepting the default is a single click.
///
/// On success, persists the chosen path to desktop.toml and returns
/// the absolute string. On user cancel, returns `Ok(None)`.
///
/// If a session is already in memory (user logged in before picking a
/// root), the engine runner is also spawned at this point.
///
/// Spec 030 §2 + §3 (lock acquired by the spawned engine).
#[tauri::command]
pub(super) async fn pick_sync_root(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<Option<String>, String> {
    #[cfg(target_os = "macos")]
    {
        let _ = app;
        let _ = state;
        return Err(
            "macOS uses the system-managed Beebeeb Finder location; local state is stored privately.".to_string(),
        );
    }

    #[cfg(not(target_os = "macos"))]
    {
        let suggestion = config::default_sync_root_suggestion();
        // Best-effort create — the dialog still opens even if this fails,
        // pointed at the parent dir.
        let _ = config::ensure_directory(&suggestion);

        let dialog = app.dialog().clone();
        // Tauri's blocking_pick_folder is synchronous from the JS caller's
        // perspective but runs the OS dialog modally. We're already on a
        // tokio task because the IPC fn is async, so this is safe.
        let chosen: Option<PathBuf> = dialog
            .file()
            .set_directory(&suggestion)
            .set_title("Choose your Beebeeb sync folder")
            .blocking_pick_folder()
            .and_then(|fp| fp.into_path().ok());

        let Some(path) = chosen else {
            // User cancelled. Don't write anything.
            return Ok(None);
        };

        #[cfg(target_os = "windows")]
        let _transition = SESSION_TRANSITION.lock().await;
        #[cfg(target_os = "windows")]
        let generation = {
            windows_cf::ensure_reactivation_allowed()?;
            SESSION_COMMANDS.generation()?
        };

        if !path.is_absolute() {
            return Err(format!(
                "picker returned a non-absolute path: {} (please report)",
                path.display()
            ));
        }
        config::ensure_directory(&path)?;

        // Task 1889: one load-change-save under the config-write lock; `cfg` is what was saved.
        let cfg = DesktopConfig::update(|cfg| {
            cfg.sync_root = Some(path.clone());
            (true, cfg.clone())
        })?;

        tracing::info!(path = %path.display(), "sync root set");

        // If a session is already loaded, start syncing immediately.
        // (Common path: the browser-login handoff calls apply_session before
        //  pick_sync_root, so the engine couldn't start there; it starts
        //  here instead.)
        // session / engine / sync_paused are per-account (decision 0800).
        let acct = state.active_account()?;
        {
            // Rehydrate the persisted pause state before spawning.
            rehydrate_sync_paused(&acct.sync_paused, cfg.pause_sync);
            let mut engine_slot = acct.engine.lock().await;
            engine_start_refusal(&acct)?;
            if let Some(prev) = engine_slot.take() {
                // Task 1538 Codex P1 — see `start_engine_if_possible`'s
                // identical respawn guard.
                if !stop_engine_in_slot(&acct, prev).await.is_stopped() {
                    #[cfg(target_os = "windows")]
                    return Err(
                        "Could not stop the previous sync engine; retry locking before restarting sync.".into(),
                    );
                    tracing::warn!("previous engine did not confirm termination before respawning a new one");
                }
            }
            // Fix round 1: an unconfirmed stop of the old engine has set the flag; refuse the new one.
            engine_start_refusal(&acct)?;
            #[cfg(target_os = "windows")]
            {
                windows_cf::ensure_reactivation_allowed()?;
                SESSION_COMMANDS.validate_start(generation)?;
            }
            // No session means nothing to start (the folder is saved either way); the keys are read under
            // the slot, so a Lock that got there first leaves nothing to start with.
            spawn_bound_engine(app, &state, &acct, &mut engine_slot, path.clone())?;
        }

        Ok(Some(path.to_string_lossy().into_owned()))
    }
}

/// Suggested default path for the sync root, used by the WebView to
/// show "We'll create ~/Beebeeb if you accept" before opening the
/// native picker.
#[tauri::command]
pub(super) fn default_sync_root() -> String {
    config::default_sync_root_suggestion().to_string_lossy().into_owned()
}

/// Read-only support snapshot. Tauri's embedded bundle marker is also used by
/// its updater; do not guess NSIS/MSI or AppImage from the host OS.
#[tauri::command]
pub(super) fn desktop_capabilities(app: tauri::AppHandle) -> desktop_capabilities::DesktopCapabilities {
    use desktop_capabilities::InstallFormat;
    use tauri::utils::config::BundleType;
    let format = match tauri::utils::platform::bundle_type() {
        Some(BundleType::App) => InstallFormat::App,
        Some(BundleType::Nsis) => InstallFormat::Nsis,
        Some(BundleType::Msi) => InstallFormat::Msi,
        Some(BundleType::AppImage) => InstallFormat::Appimage,
        Some(BundleType::Deb) => InstallFormat::Deb,
        Some(BundleType::Rpm) => InstallFormat::Rpm,
        _ => InstallFormat::Unknown,
    };
    desktop_capabilities::snapshot(
        desktop_capabilities::host_os(),
        format,
        app.tray_by_id("tray").is_some(),
    )
}

#[tauri::command]
pub(super) fn desktop_platform() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "macos"
    }
    #[cfg(target_os = "windows")]
    {
        "windows"
    }
    #[cfg(target_os = "linux")]
    {
        "linux"
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        "unknown"
    }
}

// ── IPC commands: Task 9 — settings page ──────────────────────────────────────

/// Read the current settings (bandwidth caps + notification toggles)
/// from `desktop.toml`. The on-disk `sync_root` is intentionally
/// elided from the response — see [`config::DesktopSettings`] for the
/// rationale.
///
/// Consumed by `repos/desktop/src/pages/Bandwidth.tsx` and
/// `repos/desktop/src/pages/Notifications.tsx` as their first read on
/// page mount. A missing file (no settings ever saved) returns the
/// `Default` values, matching the TS-side `DEFAULT_CONFIG`.
#[tauri::command]
pub(super) fn get_desktop_config() -> Result<config::DesktopSettings, String> {
    let cfg = DesktopConfig::load()?;
    Ok(config::DesktopSettings::from(&cfg))
}

/// Persist a new settings struct, leaving `sync_root` (and any future
/// non-settings fields) untouched. Performs a load → merge → save as one
/// update under the config-write lock (task 1889), so a concurrent
/// `pick_sync_root` or any other writer cannot race away a change between
/// this load and save.
#[tauri::command]
pub(super) async fn set_desktop_config(app: tauri::AppHandle, config: config::DesktopSettings) -> Result<(), String> {
    apply_desktop_settings(&app, config).await
}

/// A settings write. Every field but the pause is merged into the file in one update; a pause (the compact flyout's
/// Network page sends one) goes the way the menu's and the tray's go, `set_sync_paused`, so the file, the engine's
/// flag, the engine-status event and the menu's label agree (task 1683 slice 1, review round 2 P2-3). A write without
/// a pause touches neither the pause nor the label.
pub(super) async fn apply_desktop_settings(
    app: &tauri::AppHandle<impl tauri::Runtime>,
    mut config: config::DesktopSettings,
) -> Result<(), String> {
    let pause = config.pause_sync.take();
    save_desktop_settings(config)?;
    if let Some(paused) = pause {
        set_sync_paused(app, paused).await?;
    }
    Ok(())
}

/// The write behind `set_desktop_config`: one update under the config-write lock (task 1889).
pub(super) fn save_desktop_settings(config: config::DesktopSettings) -> Result<(), String> {
    DesktopConfig::update(|cfg| {
        cfg.apply_settings(config);
        (true, ())
    })
}

// ── IPC aliases for frontend compatibility ────────────────────────────────────

/// Alias so the frontend can call `desktop_config` (the name used in
/// `WindowsFirstRun.tsx` and the in-app settings view) in addition to the
/// canonical `get_desktop_config`. Both names are registered in
/// `generate_handler!`. Returns `None` when the config file doesn't
/// exist yet (first-run path), which the frontend uses as a signal that
/// sync-mode hasn't been persisted yet.
#[tauri::command]
pub(super) fn desktop_config() -> Result<Option<config::DesktopSettings>, String> {
    let path = DesktopConfig::path()?;
    if !path.exists() {
        return Ok(None);
    }
    let cfg = DesktopConfig::load()?;
    Ok(Some(config::DesktopSettings::from(&cfg)))
}

/// Open (or focus) the main Beebeeb app window. This is the new primary
/// surface on Windows — the full sidebar + content-router shell
/// (`WindowsApp.tsx`, selected by `?window=main-app&platform=windows`) that
/// hosts the data views. The tray flyout's "Settings" button and the
/// onboarding "Open control center" button both route here; Settings remains
/// reachable from a nav item inside the app.
///
/// On non-Windows platforms there is no dedicated main-app window yet, so we
/// fall back to the compact app window — keeping a single, predictable
/// entry point for callers regardless of OS.
#[tauri::command]
pub(super) fn show_main_app_window(app: tauri::AppHandle) -> Result<(), String> {
    show_main_app_window_impl(&app);
    Ok(())
}

/// Open (or focus) the main Beebeeb app window and select the consolidated
/// Settings section. Used by the Windows tray flyout's Settings gear.
#[tauri::command]
pub(super) fn show_main_app_settings(app: tauri::AppHandle) -> Result<(), String> {
    show_main_app_window_with_nav(&app, Some("settings"));
    Ok(())
}

// ── IPC commands: Windows tray (WS1) ─────────────────────────────────────────

/// One row in the tray's recent-activity list.
#[derive(Debug, serde::Serialize)]
pub(super) struct TrayActivityItem {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) status: String,
    pub(super) icon: String,
    pub(super) ok: Option<bool>,
    pub(super) active: Option<bool>,
    pub(super) progress: Option<u8>,
    /// LD-0105 Amendment 4: why the row needs the person (`changed_on_this_device`); absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) attention: Option<&'static str>,
    /// The one action the row offers (`discard_and_download_again`); absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) action: Option<&'static str>,
}

pub(super) const TRAY_RECENT_ACTIVITY_LIMIT: usize = 50;

/// Basename of a stored `FileEntry.path`. Uses `std::path::Path::file_name`
/// so Windows cache paths stored with backslash separators yield the correct
/// basename (a naive `rsplit('/')` would return the whole `C:\…\foo.txt`
/// string on Windows). Falls back to the full path if it has no file-name
/// component.
pub(super) fn entry_basename(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

pub(super) fn tray_recent_activity_from_db(db: &state_db::StateDb) -> Vec<TrayActivityItem> {
    let mut items: Vec<TrayActivityItem> = Vec::new();

    // LD-0105 Amendment 4 first: a failed download whose file changed on this device since. It is kept
    // as it is (never emptied), and only "Discard and download again" ends the row.
    if let Ok(rows) = db.poisoned() {
        for row in rows.into_iter().filter(|r| r.needs_attention) {
            // Review of #225: a poison whose file row is gone has no exit (the discard answers `missing`
            // until the next process start clears it): no row, here as on the board.
            let Some(name) = db.get_file(&row.file_id).ok().flatten().map(|e| entry_basename(&e.path)) else {
                continue;
            };
            items.push(TrayActivityItem {
                id: row.file_id,
                name,
                status: "Couldn't finish downloading".to_string(),
                icon: "file".to_string(),
                ok: Some(false),
                active: None,
                progress: None,
                attention: Some(crate::transfer_progress::ATTENTION_CHANGED),
                action: Some(crate::transfer_progress::ACTION_DISCARD),
            });
        }
    }

    // In-flight transfers first (downloading + uploading), newest first.
    for status in [state_db::FileStatus::Downloading, state_db::FileStatus::Uploading] {
        let label = if matches!(status, state_db::FileStatus::Downloading) {
            "Downloading…"
        } else {
            "Uploading…"
        };
        let Ok(entries) = db.list_by_status(status) else {
            continue;
        };
        let mut entries = entries;
        entries.sort_by(|a, b| b.modified_at.cmp(&a.modified_at).then_with(|| a.path.cmp(&b.path)));
        for entry in entries {
            items.push(TrayActivityItem {
                id: entry.file_id,
                name: entry_basename(&entry.path),
                status: label.to_string(),
                icon: "cloud".to_string(),
                ok: None,
                active: Some(true),
                progress: None,
                attention: None,
                action: None,
            });
        }
    }

    if let Ok(events) = db.list_recent_local_activity(TRAY_RECENT_ACTIVITY_LIMIT) {
        for event in events.into_iter() {
            // Task 1640: a kept copy of changes not sent, beside a file deleted elsewhere.
            let (status, icon) = match event.event_type {
                state_db::LocalActivityKind::MovedToTrash => ("Moved to trash", "trash"),
                state_db::LocalActivityKind::KeptLocalCopy => ("Your changes, kept as a copy", "file"),
                state_db::LocalActivityKind::Restored => continue,
            };
            items.push(TrayActivityItem {
                id: event.file_id.unwrap_or_else(|| format!("activity-{}", event.id)),
                name: event.file_name,
                status: status.to_string(),
                icon: icon.to_string(),
                ok: Some(true),
                active: None,
                progress: None,
                attention: None,
                action: None,
            });
        }
    }

    if let Ok(locals) = db.list_by_status(state_db::FileStatus::Local) {
        // Sort by modified_at descending — most recently synced first.
        let mut locals = locals;
        locals.sort_by(|a, b| b.modified_at.cmp(&a.modified_at).then_with(|| a.path.cmp(&b.path)));
        for entry in locals {
            items.push(TrayActivityItem {
                id: entry.file_id,
                name: entry_basename(&entry.path),
                status: "Synced".to_string(),
                icon: "file".to_string(),
                ok: Some(true),
                active: None,
                progress: None,
                attention: None,
                action: None,
            });
        }
    }

    items.truncate(TRAY_RECENT_ACTIVITY_LIMIT);
    items
}

/// Return recent sync activity for the Windows tray flyout.
///
/// Derives activity from the state DB:
///   - Files currently downloading or uploading (active, with progress = None
///     because per-file byte progress isn't tracked at this layer)
///   - Recently-synced local files ordered by `modified_at` descending
///
/// Returns an EMPTY vec when there is no state DB (no sync root configured
/// or engine not yet started). Never fabricates rows.
#[tauri::command]
pub(super) fn tray_recent_activity() -> Vec<TrayActivityItem> {
    let Ok(cfg) = DesktopConfig::load() else {
        return Vec::new();
    };
    let Some(_) = cfg.sync_root else {
        return Vec::new();
    };
    let Ok(state_dir) = state_paths::beebeeb_state_dir() else {
        return Vec::new();
    };
    let db_path = state_dir.join(state_paths::STATE_DB_FILENAME);
    if !db_path.exists() {
        return Vec::new();
    }
    let Ok(db) = state_db::StateDb::open(&db_path) else {
        return Vec::new();
    };

    tray_recent_activity_from_db(&db)
}

/// LD-0105 Amendment 4: "Discard and download again" on a needs-attention row. Empties the file under
/// the exclusive handle (whatever was written to it), clears the poison, and fetches it again. The
/// error is a status word: `not_poisoned`, `in_use` (close the file first), `missing`, or a step.
#[tauri::command]
pub(super) async fn discard_and_download_again(file_id: String, runtime: State<'_, popover_data::PopoverRuntime>) -> Result<(), String> {
    // Delta review P2-1: an upload of the person's change is in flight; discarding would throw it away.
    if runtime.transfers.get(&file_id).is_some_and(|t| t.direction == crate::transfer_progress::Direction::Up) {
        return Err("uploading".to_string());
    }
    #[cfg(target_os = "windows")]
    {
        use crate::windows_cf::poison::DiscardRefused as R;
        let id = file_id.clone();
        // The exclusive open and the dehydrate block: off the main thread (review P2).
        let done = tokio::task::spawn_blocking(move || crate::windows_cf::poison::discard_and_download_again(&file_id))
            .await
            .map_err(|_| "failed".to_string())?;
        if done.is_ok() {
            // Review P3-6: the row is gone from the kept rows, and no read already running brings it back.
            let next = crate::transfer_progress::ATTENTION_READS.load(std::sync::atomic::Ordering::SeqCst);
            if let Ok(mut cache) = crate::transfer_progress::ATTENTION_CACHE.lock() {
                cache.forget(&id, next);
            }
        }
        done.map_err(|e| match e {
            R::NotPoisoned => "not_poisoned".to_string(),
            R::InUse => "in_use".to_string(),
            R::Missing => "missing".to_string(),
            R::Failed(step) => step.to_string(),
        })
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = file_id;
        Err("not_supported".to_string())
    }
}

/// Pause the sync engine at runtime. Sets the shared `sync_paused` flag
/// so the runner's next tick no-ops. Persists `pause_sync = true` to
/// `desktop.toml` so the paused state survives a restart.
///
/// Emits an `engine-status` event with `state = "paused"` so the tray
/// tooltip and any open app window update immediately.
/// Task 1889 fix round 1 (M6): the pause reaches the in-memory flag in the same step that persists it. The flag is stored
/// while the config-write lock is held, after the write succeeded, so it can never say paused while the file says
/// unpaused or the other way round: a write that fails leaves both as they were, and a rehydration
/// ([`rehydrate_sync_paused`]) or a sync-root write that reads the pause waits for this one and stores what it wrote.
pub(super) fn persist_sync_paused_at(path: &Path, flag: &AtomicBool, paused: bool) -> Result<(), String> {
    DesktopConfig::update_at_then(
        path,
        |cfg| {
            cfg.pause_sync = paused;
            (true, ())
        },
        |cfg| flag.store(cfg.pause_sync, Ordering::Relaxed),
    )
}

/// Stores the persisted pause state into the in-memory flag, under the config-write lock (see
/// [`persist_sync_paused_at`]). A restart that finds the file paused starts paused.
pub(super) fn rehydrate_sync_paused_at(path: &Path, flag: &AtomicBool) -> Result<(), String> {
    DesktopConfig::update_at_then(
        path,
        |_| (false, ()),
        |cfg| flag.store(cfg.pause_sync, Ordering::Relaxed),
    )
}

/// [`rehydrate_sync_paused_at`] on the real config. An unreadable config stores `fallback`, which is what each caller
/// stored before when it could not read the file (the value it had already loaded, or "not paused").
pub(super) fn rehydrate_sync_paused(flag: &AtomicBool, fallback: bool) {
    let stored = DesktopConfig::path().and_then(|path| rehydrate_sync_paused_at(&path, flag));
    if stored.is_err() {
        flag.store(fallback, Ordering::Relaxed);
    }
}

pub(super) async fn set_sync_paused(app: &tauri::AppHandle<impl tauri::Runtime>, paused: bool) -> Result<(), String> {
    let state = app.state::<AppState>();
    let acct = state.active_account()?;
    // Persist so the paused state survives a restart.
    let persisted = persist_sync_paused_at(&DesktopConfig::path()?, &acct.sync_paused, paused);
    // Two-surface F3: the menu's "Pause Sync" / "Resume Sync" follows what the file now says, also after a failure.
    pause_state_changed(app);
    persisted?;
    // Notify the WebView + tray tooltip listener.
    let event_state = if paused { "paused" } else { "idle" };
    let _ = app.emit("engine-status", serde_json::json!({ "state": event_state }));
    tracing::info!(paused, "sync pause state changed");
    Ok(())
}

#[tauri::command]
pub(super) async fn tray_pause_sync(app: tauri::AppHandle) -> Result<(), String> {
    set_sync_paused(&app, true).await
}

/// Resume the sync engine. Clears the `sync_paused` flag so the runner's
/// next tick proceeds normally. Persists `pause_sync = false` to
/// `desktop.toml`. Emits an `engine-status` event with `state = "idle"`.
#[tauri::command]
pub(super) async fn tray_resume_sync(app: tauri::AppHandle) -> Result<(), String> {
    set_sync_paused(&app, false).await
}

/// What sync is doing, for Settings › Sync and the Windows tray popover (0.8.15, see [`sync_activity`]): the live pause
/// flag, the engine's last state, what is moving, the queue in counts, and the startup check while it runs. Counts
/// only; reading it changes nothing and needs no session, so it answers while the vault is locked too.
#[tauri::command]
pub(super) async fn sync_activity_summary(
    state: State<'_, AppState>,
    runtime: State<'_, popover_data::PopoverRuntime>,
) -> Result<sync_activity::SyncActivitySummary, String> {
    let paused = state.active_account()?.sync_paused.load(Ordering::Relaxed);
    let engine = runtime.status.lock().map(|view| view.clone()).unwrap_or_default();
    let transfers_active = runtime.transfers.active().len();
    let queue = tokio::task::spawn_blocking(|| -> Result<Option<state_db::QueueHold>, String> {
        let Some(db) = DesktopConfig::load().and_then(|cfg| state_db_for_config(&cfg))? else {
            return Ok(None);
        };
        db.queue_hold_summary().map(Some).map_err(|e| format!("read the sync queue: {e}"))
    })
    .await
    .map_err(|e| format!("sync activity task failed: {e}"))??;
    Ok(sync_activity::summary(paused, &engine, transfers_active, queue))
}

/// The menu's Pause / Resume item: flips the pause the file holds. Returns which way it went (`true` = pausing) with the
/// result, so a failure can say which.
pub(super) async fn toggle_sync_paused_from_menu(app: &tauri::AppHandle) -> (bool, Result<(), String>) {
    let pausing = !DesktopConfig::load().map(|cfg| cfg.pause_sync).unwrap_or(false);
    (pausing, set_sync_paused(app, pausing).await)
}

/// The title of the alert a failed Pause from the menu raises.
pub(super) const MENU_PAUSE_FAILED_TITLE: &str = "Pause sync";
/// The title of the alert a failed Resume from the menu raises.
pub(super) const MENU_RESUME_FAILED_TITLE: &str = "Resume sync";
/// What a failed Pause says. Honest about the state: a pause that cannot be written changes neither the file nor the
/// running engine (task 1889, M6), so sync carries on as it was. It never carries the error behind it (a path, an OS
/// message); the caller logs that.
pub(super) const MENU_PAUSE_FAILED_SENTENCE: &str = "Beebeeb couldn’t pause syncing, so it carries on as before.";
/// What a failed Resume says: the pause stays.
pub(super) const MENU_RESUME_FAILED_SENTENCE: &str = "Beebeeb couldn’t resume syncing, so it stays paused.";

/// What the menu's Pause / Resume shows the person when its work ended in `result` (task 1889, fix round 2 N2): an `Err`
/// is an alert with the title and the one sentence for the way it was going (`pausing`); success shows nothing. The menu
/// works with every window closed, so it has no window to put a toast in and the alert is native, like 1885's failed
/// "Open in Finder" and the Sign-out menu action's. On every platform: the native menu is the one live caller of the
/// pause commands, and a pause that does nothing must not look like one that worked.
pub(super) fn menu_pause_failure_alert(result: &Result<(), String>, pausing: bool) -> Option<(&'static str, &'static str)> {
    match (result, pausing) {
        (Ok(()), _) => None,
        (Err(_), true) => Some((MENU_PAUSE_FAILED_TITLE, MENU_PAUSE_FAILED_SENTENCE)),
        (Err(_), false) => Some((MENU_RESUME_FAILED_TITLE, MENU_RESUME_FAILED_SENTENCE)),
    }
}

/// A failed Pause or Resume from the menu, shown to the person before the caller logs it (see
/// [`menu_pause_failure_alert`]).
pub(super) fn show_menu_pause_failure(app: &tauri::AppHandle, pausing: bool, result: &Result<(), String>) {
    let Some((title, message)) = menu_pause_failure_alert(result, pausing) else {
        return;
    };
    app.dialog()
        .message(message)
        .title(title)
        .kind(tauri_plugin_dialog::MessageDialogKind::Error)
        .show(|_| {});
}

#[tauri::command]
pub(super) async fn upload_files_to_sync_root(app: tauri::AppHandle) -> Result<usize, String> {
    upload_files_to_sync_root_impl(&app)
}

#[tauri::command]
pub(super) fn create_folder_in_sync_root(app: tauri::AppHandle) -> Result<String, String> {
    let path = create_folder_in_sync_root_impl(&app)?;
    Ok(path.to_string_lossy().into_owned())
}

/// Result of `report_problem` for the Account page. The bundle is written
/// before the email draft is attempted, so a failed mail open is a partial
/// success, not a failed save (task 1685).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(super) struct ProblemReport {
    pub(super) path: String,
    pub(super) email_opened: bool,
}

/// `async` so Tauri runs it off the main thread: building the bundle reads every
/// known file name from the state DB and must not stall the window.
#[tauri::command(async)]
pub(super) fn report_problem(app: tauri::AppHandle) -> Result<ProblemReport, String> {
    report_problem_impl(&app)
}

pub(super) fn sync_root_for_menu_file_action() -> Result<PathBuf, String> {
    let root = DesktopConfig::load()
        .ok()
        .and_then(|cfg| cfg.sync_root)
        .unwrap_or_else(config::default_sync_root_suggestion);
    config::ensure_directory(&root)?;
    Ok(root)
}

/// Opens the Finder location after an upload or a new folder, WITHOUT making the caller wait (task 1885 fix round 1, m4).
/// The open can wait up to 3 s for the bridge gate and 7 s for LaunchServices. Its callers are the menu handler and a plain
/// command (the main thread), and a runtime worker, none of which may sit in that wait; and nobody needs its answer, so it
/// runs on the blocking pool and a failure is logged.
pub(super) fn open_finder_location_in_background(sync_root: String) {
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(error) = open_finder_location_blocking(Some(sync_root)) {
            tracing::warn!(%error, "could not open the Beebeeb folder after the menu action");
        }
    });
}

pub(super) fn upload_files_to_sync_root_impl(app: &tauri::AppHandle) -> Result<usize, String> {
    let sync_root = sync_root_for_menu_file_action()?;
    let selected = app
        .dialog()
        .file()
        .set_title("Upload files to Beebeeb")
        .blocking_pick_files()
        .unwrap_or_default();

    let mut copied = 0usize;
    for file in selected {
        let source = file
            .into_path()
            .map_err(|e| format!("resolve selected file path: {e}"))?;
        if !source.is_file() {
            continue;
        }
        let Some(name) = source.file_name().map(|name| name.to_string_lossy().into_owned()) else {
            continue;
        };
        let destination = unused_child_path(&sync_root, &name)?;
        std::fs::copy(&source, &destination).map_err(|e| {
            format!(
                "copy selected file into Beebeeb folder ({} -> {}): {e}",
                source.display(),
                destination.display()
            )
        })?;
        copied += 1;
    }

    if copied > 0 {
        let _ = app.emit("menu:files-added", serde_json::json!({ "count": copied }));
        open_finder_location_in_background(sync_root.to_string_lossy().into_owned());
    }

    Ok(copied)
}

pub(super) fn create_folder_in_sync_root_impl(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let sync_root = sync_root_for_menu_file_action()?;
    let path = unused_child_path(&sync_root, "New folder")?;
    std::fs::create_dir(&path).map_err(|e| format!("create Beebeeb folder {}: {e}", path.display()))?;
    let _ = app.emit(
        "menu:folder-created",
        serde_json::json!({ "path": path.to_string_lossy() }),
    );
    if let Err(error) = app.opener().reveal_item_in_dir(&path) {
        tracing::warn!(%error, path = %path.display(), "could not reveal new folder; opening sync root instead");
        open_finder_location_in_background(sync_root.to_string_lossy().into_owned());
    }
    Ok(path)
}

pub(super) fn diagnostics_bundle_path() -> Result<PathBuf, String> {
    let state_dir = state_paths::beebeeb_state_dir().or_else(|_| {
        DesktopConfig::path().map(|path| path.parent().map(Path::to_path_buf).unwrap_or_else(std::env::temp_dir))
    })?;
    let dir = state_dir.join("diagnostics");
    std::fs::create_dir_all(&dir).map_err(|e| format!("create diagnostics directory {}: {e}", dir.display()))?;
    Ok(dir.join(format!("beebeeb-diagnostics-{}.json", now_unix_seconds())))
}

/// Body of the support email draft. Names only the bundle's file name, never
/// its folder: the folder sits under the user's home directory (task 1685).
pub(super) fn problem_report_email_body(bundle_path: &Path) -> String {
    let file_name = bundle_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "beebeeb-diagnostics.json".to_string());
    format!(
        "Describe the problem here.\n\nThe support bundle {file_name} was saved on this computer and shown in your file manager. Attach it to this email after you have read it."
    )
}

/// Turns the outcome of opening the mail draft into the report. Called after the
/// bundle has been written, so a mail error never hides the saved bundle.
pub(super) fn problem_report_outcome<E: std::fmt::Display>(path: &Path, mail: Result<(), E>) -> ProblemReport {
    if let Err(error) = &mail {
        tracing::warn!(%error, "support bundle saved but the email draft could not be opened");
    }
    ProblemReport {
        path: path.to_string_lossy().into_owned(),
        email_opened: mail.is_ok(),
    }
}

pub(super) fn report_problem_impl(app: &tauri::AppHandle) -> Result<ProblemReport, String> {
    let diagnostics = export_diagnostics()?;
    let path = diagnostics_bundle_path()?;
    let body = serde_json::to_vec_pretty(&diagnostics).map_err(|e| format!("serialize diagnostics: {e}"))?;
    std::fs::write(&path, body).map_err(|e| format!("write diagnostics bundle {}: {e}", path.display()))?;

    let _ = app.opener().reveal_item_in_dir(&path);
    let subject = urlencoding::encode("Beebeeb desktop problem report");
    let body_text = problem_report_email_body(&path);
    let body = urlencoding::encode(&body_text);
    let mail = app.opener().open_url(
        format!("mailto:support@beebeeb.io?subject={subject}&body={body}"),
        None::<&str>,
    );
    Ok(problem_report_outcome(&path, mail))
}

pub(super) fn unused_child_path(parent: &Path, preferred_name: &str) -> Result<PathBuf, String> {
    let preferred = Path::new(preferred_name)
        .file_name()
        .ok_or_else(|| "destination name is empty".to_string())?
        .to_string_lossy()
        .into_owned();
    let first = parent.join(&preferred);
    if !first.exists() {
        return Ok(first);
    }

    let path = Path::new(&preferred);
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| preferred.clone());
    let extension = path.extension().map(|ext| ext.to_string_lossy().into_owned());

    for suffix in 2..10_000 {
        let name = match &extension {
            Some(ext) if !ext.is_empty() => format!("{stem} {suffix}.{ext}"),
            _ => format!("{stem} {suffix}"),
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(format!(
        "could not find an unused destination name for {} in {}",
        preferred,
        parent.display()
    ))
}

/// Persist the user's chosen sync mode from the Windows first-run wizard.
///
/// `mode` is one of `"everything"` | `"smart"` | `"custom"` | `"online_only"`.
/// The value is stored in `desktop.toml` under `sync_mode`. The runner
/// does not yet act on these modes (full smart-sync requires selective-sync
/// logic in the engine); storing it now means a future version can read it
/// without a migration step.
///
/// The frontend (`WindowsFirstRun.tsx`) uses the presence of a persisted
/// mode as the signal that the sync-mode step was completed; `desktop_config`
/// returns `None` when the file doesn't exist yet, triggering that step on
/// first run.
#[tauri::command]
pub(super) fn set_sync_mode(mode: String) -> Result<(), String> {
    match mode.as_str() {
        "everything" | "smart" | "custom" | "online_only" => {}
        _ => return Err(format!("invalid sync mode: {mode}")),
    }
    DesktopConfig::update(|cfg| {
        cfg.sync_mode = Some(mode);
        (true, ())
    })
}

// ── IPC commands: Task 0090 — selective sync ──────────────────────────────────

/// Read the persisted list of top-level folder IDs the user has chosen
/// to keep cloud-only on this device. Returns an empty `Vec` (not
/// `Err`) when the key is missing from `desktop.toml`, which is the
/// common case on a fresh install.
///
/// The shape mirrors what `repos/desktop/src/pages/SelectiveSync.tsx`
/// renders: the page only needs the IDs, then cross-references them
/// against the folder list returned by [`list_vault_folders`] to mark
/// the matching checkboxes off.
#[tauri::command]
pub(super) fn get_selective_sync() -> Result<Vec<String>, String> {
    let cfg = DesktopConfig::load()?;
    Ok(cfg.excluded_folder_ids.unwrap_or_default())
}

/// Persist a new list of excluded folder IDs to `desktop.toml` AND
/// reclaim disk space for any folder that just became excluded (wave-2
/// of task 0090 — the real fix the original no-op comment promised).
///
/// Flow:
///   1. Snapshot the OLD excluded list, then persist the NEW one (an
///      empty list normalises to `None` so the on-disk file stays free of
///      `excluded_folder_ids = []` clutter). Persisting first guarantees
///      the choice survives even if the dehydrate step below errors.
///   2. Diff: folder ids in NEW but not OLD are "newly excluded."
///   3. For each newly-excluded folder, resolve its subtree's files from
///      the local state DB and DEHYDRATE every file that is currently
///      hydrated (`status == Local`) AND not effectively Pinned — the
///      exact pattern [`free_up_space_windows`] uses. The file stays
///      cloud-only (we NEVER delete user data; dehydrate only reclaims the
///      local cache, and the file re-hydrates on demand). A pinned file in
///      an excluded folder is kept — the user explicitly asked for it
///      offline, and an explicit pin wins over an exclusion.
///   4. Newly-INCLUDED (un-excluded) folders need no immediate action —
///      on-demand hydration re-downloads them when next opened.
///
/// Best-effort + idempotent: a dehydrate failure on one file logs and
/// continues; the persisted list is unaffected. On non-Windows builds the
/// DB transition still runs (rows flip to cloud_only via
/// `mark_cloud_only_after_dehydrate`) but the platform dehydrate is a
/// no-op there — the macOS File Provider / Linux FUSE cache eviction is
/// the engine's existing job and is not in scope for this task.
///
/// The DB + dehydrate work runs on a blocking thread so a large excluded
/// subtree doesn't hold the IPC command thread on SQLite + CF syscalls.
#[tauri::command]
pub(super) async fn set_selective_sync(excluded: Vec<String>) -> Result<(), String> {
    // 1. Snapshot old, persist new. Persist FIRST so the choice sticks
    //    regardless of the reclaim outcome.
    let old_excluded = DesktopConfig::update(|cfg| {
        let old_excluded = cfg.excluded_folder_ids.clone().unwrap_or_default();
        cfg.excluded_folder_ids = if excluded.is_empty() {
            None
        } else {
            Some(excluded.clone())
        };
        (true, old_excluded)
    })?;

    // 2. Diff for the subtrees we must reclaim now.
    let newly_excluded = newly_excluded_ids(&old_excluded, &excluded);
    if newly_excluded.is_empty() {
        return Ok(());
    }

    // 3. Reclaim on a blocking thread. Failures here never undo the persist.
    let join = tokio::task::spawn_blocking(move || dehydrate_excluded_subtrees_blocking(&newly_excluded));
    match join.await {
        Ok(Ok(freed)) => {
            tracing::info!(
                bytes_freed = freed,
                "set_selective_sync: dehydrated newly-excluded subtrees"
            );
        }
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "set_selective_sync: reclaim of excluded subtrees failed (list still persisted)");
        }
        Err(e) => {
            tracing::warn!(error = %e, "set_selective_sync: reclaim task join failed (list still persisted)");
        }
    }
    Ok(())
}

// ── IPC commands: Task 0797 — known-folder backup ("Manage backup") ───────────

/// One row in the "Manage backup" panel: a Windows known folder, its real
/// source path on this machine, whether the user has opted into backing it up,
/// and (when cheap to compute) a rough item count for the source.
///
/// Mirrors the selective-sync IPC shape (a flat DTO the TS side renders without
/// further lookups). `source_path` is `None` when the folder can't be resolved
/// on this OS — on non-Windows there ARE no Windows known folders, so every row
/// reports `source_path: None` + `enabled: false`, which the panel renders as
/// "not available on this platform".
#[derive(Debug, Clone, serde::Serialize)]
pub(super) struct KnownFolderStatus {
    /// Stable key, e.g. `"documents"`. Matches `desktop.toml`.
    pub(super) key: String,
    /// UI label / vault subfolder name, e.g. `"Documents"`.
    pub(super) display_name: String,
    /// Absolute source path on this machine, or `None` if unresolved.
    pub(super) source_path: Option<String>,
    /// `true` if this folder is in the persisted backup set.
    pub(super) enabled: bool,
    /// Rough count of files in the source tree (top-level + nested, bounded), or
    /// `None` when the source can't be enumerated. Cheap-ish, best-effort.
    pub(super) item_count: Option<u64>,
    /// LD-0113 Amendment 9: a folder OneDrive moved (`moved_path`, `source_choice`, `question`, `note`).
    #[serde(flatten)]
    pub(super) moved: crate::backup::kfm::MovedFolder,
}

/// List the backupable known folders with their resolved source paths + current
/// enabled state. Drives the "Manage backup" panel in the Files dashboard.
///
/// On Windows: resolves each `FOLDERID_*` via `SHGetKnownFolderPath` and counts
/// items (bounded). On other platforms: returns the catalog with `source_path:
/// None` so the panel can render a "Windows only" state rather than 404-ing.
#[tauri::command]
pub(super) fn get_known_folder_backup() -> Result<Vec<KnownFolderStatus>, String> {
    let cfg = DesktopConfig::load()?;
    let mut out = Vec::with_capacity(known_folder::KNOWN_FOLDERS.len());
    for kf in known_folder::KNOWN_FOLDERS {
        let enabled = cfg.known_folder_enabled(kf.key);
        let view = crate::backup::kfm::folder_view(kf.key, kf.display_name, enabled, cfg.known_folder_source.get(kf.key).copied());
        #[cfg(target_os = "windows")] // never counted inside OneDrive's folder (listing an online-only one can fetch it)
        let item_count = view.source_path.as_deref().filter(|_| view.reads_default).and_then(count_items_bounded);
        #[cfg(not(target_os = "windows"))]
        let item_count: Option<u64> = None;

        out.push(KnownFolderStatus {
            key: kf.key.to_string(),
            display_name: kf.display_name.to_string(),
            source_path: view.source_path.map(|p| p.to_string_lossy().into_owned()),
            enabled,
            item_count,
            moved: view.moved,
        });
    }
    Ok(out)
}

/// Bounded, best-effort count of regular files under `root` (≤ this many before
/// we stop — the panel only needs an "about this many" signal, not an exact
/// total, and an exact recursive count of a huge Documents folder would block
/// the IPC thread). Windows-only (the only caller is gated).
#[cfg(target_os = "windows")]
pub(super) fn count_items_bounded(root: &std::path::Path) -> Option<u64> {
    const CAP: u64 = 10_000;
    fn walk(dir: &std::path::Path, depth: usize, count: &mut u64) {
        if depth > 32 || *count >= CAP {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            if *count >= CAP {
                return;
            }
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                walk(&entry.path(), depth + 1, count);
            } else if ft.is_file() {
                *count += 1;
            }
        }
    }
    let mut count = 0u64;
    walk(root, 0, &mut count);
    Some(count)
}

/// Enable or disable known-folder backup for one folder key.
///
/// **Returns immediately (task 0811).** This IPC only flips `desktop.toml`, and
/// on disable purges the folder's pending queue. It NEVER runs a backup pass
/// inline: the runner's backup scheduler (LD-0113, `backup::scheduler`) sees the
/// new folder on its next tick and passes it (each pass queues a bounded batch,
/// so a first enable of a large folder such as Music, 5,344 files, can't
/// thunder). A pass reads the folder in place and uploads into
/// `Backups/<device>/<category>`; nothing is copied into the sync root.
///
/// **Enable is "Back up again"** for a backup that was turned off because its
/// folder was deleted elsewhere (rule 11): the stop notice is cleared.
///
/// **Disable purges the queue (founder design, task 0811).** Every still-pending
/// upload op of this folder goes, so uploads stop immediately and nothing
/// resumes on the next boot; an open server session of one is abandoned by the
/// runner (`abandon_orphaned_backup_sessions`). It does NOT delete what is already backed
/// up (that would delete the user's cloud content, which we never do silently).
///
/// Validates `key` against the catalog so a typo from the frontend can't write a
/// junk entry into the config.
#[tauri::command]
pub(super) async fn set_known_folder_backup(key: String, enabled: bool) -> Result<(), String> {
    // Reject unknown keys — the catalog is the source of truth.
    if known_folder::known_folder_by_key(&key).is_none() {
        return Err(format!("unknown known-folder key: {key}"));
    }

    let cfg = DesktopConfig::update(|cfg| {
        cfg.set_known_folder(&key, enabled);
        (true, cfg.clone())
    })?;
    let db = state_db_for_config(&cfg);
    // LD-0113 §6.2: protection follows the switch at once. With no state DB there is no drive to protect yet; the
    // runner's start sets it.
    if let Ok(Some(db)) = &db {
        crate::backup_protect::refresh_own_backups(db);
    }

    // On DISABLE, purge this folder's pending backup ops so they stop now and do
    // not resume on restart. Best-effort: a missing/unopenable state DB (no sync
    // root yet, or backup never ran) just means there is nothing to purge — the
    // persisted config flip already prevents future passes from re-enqueuing.
    if !enabled {
        match &db {
            Ok(Some(db)) => match db.purge_backup_source_ops(&key) {
                // Zero-knowledge: the key is a stable catalog identifier (not user
                // content); logging the purged COUNT confirms the cancel worked.
                Ok(purged) => {
                    tracing::info!(folder = %key, purged, "known-folder backup disabled: purged pending upload ops");
                }
                Err(e) => {
                    tracing::warn!(folder = %key, error = %e, "known-folder backup disable: queue purge failed");
                }
            },
            Ok(None) => {
                tracing::debug!(folder = %key, "known-folder backup disable: no state DB yet; nothing to purge");
            }
            Err(e) => {
                tracing::warn!(folder = %key, error = %e, "known-folder backup disable: could not open state DB to purge");
            }
        }
    }

    // On ENABLE the runner's next backup tick picks the folder up from `desktop.toml`; returning immediately is
    // the whole point of task 0811. LD-0113: a stop notice of this folder (rule 11) is cleared: "Back up again".
    if enabled {
        if let Ok(Some(db)) = &db {
            if let Err(e) = db.clear_backup_stop(&key) {
                tracing::warn!(folder = %key, error = %e, "known-folder backup enable: stop notice not cleared");
            }
        }
    }
    // (Decision 0797's pin of the mirrored vault copies, TODO(0797-pin), went with the mirror: LD-0113 copies nothing
    // into the sync root.)

    Ok(())
}

// ── IPC commands: Task 0804 — known-folder backup first-run onboarding ─────────

/// Read whether the one-time known-folder backup prompt has already been shown.
///
/// The frontend uses this — together with an empty `known_folder_backup` — to
/// decide whether to AUTOMATICALLY surface the first-run "Back up these folders?"
/// prompt on app start. `false` (the fresh-install default, and the value for an
/// older `desktop.toml` missing the key) means "not yet seen → show it once."
///
/// Mirrors the other simple config reads (`get_selective_sync`): returns the
/// plain boolean, never an error for a missing file (a missing file loads as the
/// default config).
#[tauri::command]
pub(super) fn get_known_folder_onboarding_seen() -> Result<bool, String> {
    let cfg = DesktopConfig::load()?;
    Ok(cfg.known_folder_onboarding_seen)
}

/// Mark the one-time known-folder backup prompt as shown so it never appears
/// automatically again. Called on BOTH paths of the prompt — "Back up these
/// folders" (after the folders are enabled + pinned) and "Not now" — because
/// either way the user has made their first-run choice and should not be
/// re-nagged.
///
/// Idempotent: setting an already-set flag is a no-op write. Load → set → save
/// so it leaves every other field (including `known_folder_backup`, which the
/// accept path writes via `set_known_folder_backup`) untouched.
#[tauri::command]
pub(super) fn mark_known_folder_onboarding_seen() -> Result<(), String> {
    DesktopConfig::update(|cfg| {
        cfg.known_folder_onboarding_seen = true;
        (true, ())
    })
}

/// Whether the Beebeeb window's one-time hint on Home ("Beebeeb lives in the menu bar", two-surface spec §7.6) has been
/// dismissed. `false` for a fresh install and for a `desktop.toml` that has no such key.
#[tauri::command]
pub(super) fn get_menu_bar_hint_seen() -> Result<bool, String> {
    Ok(DesktopConfig::load()?.menu_bar_hint_seen)
}

/// "Got it" on that hint: it is never shown again. Idempotent, and a write only when it changes something. One update, so
/// it lands next to any other save in flight (task 1889).
#[tauri::command]
pub(super) fn mark_menu_bar_hint_seen() -> Result<(), String> {
    DesktopConfig::update(|cfg| {
        let changed = !cfg.menu_bar_hint_seen;
        cfg.menu_bar_hint_seen = true;
        (changed, ())
    })
}

#[cfg(test)]
#[path = "settings_attention_tests.rs"]
mod attention_tests;
