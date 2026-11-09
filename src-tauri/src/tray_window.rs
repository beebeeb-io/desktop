//! `tray_window` items split out of `lib.rs`.

use super::*;

pub(super) fn handle_desktop_menu_action(app: &tauri::AppHandle, spec: &'static DesktopMenuSpec) {
    match spec.action {
        DesktopMenuAction::OpenSettings => {
            // F1: the Beebeeb window on Settings (the last section used in it, else General).
            #[cfg(target_os = "macos")]
            show_main_window(app, MainNav::settings(None));
            #[cfg(not(target_os = "macos"))]
            show_main_app_window_with_nav(app, Some("settings"));
        }
        DesktopMenuAction::CheckForUpdates => {
            request_menu_update_check(app);
        }
        DesktopMenuAction::ToggleSync => {
            let app = app.clone();
            // A failure is shown (task 1889, fix round 2 N2) before it is logged: the menu has no window to put it in.
            spawn_menu_task(spec.id, async move {
                let (pausing, result) = toggle_sync_paused_from_menu(&app).await;
                show_menu_pause_failure(&app, pausing, &result);
                result
            });
        }
        DesktopMenuAction::SignOut => {
            let app = app.clone();
            spawn_menu_task(spec.id, async move {
                let state = app.state::<AppState>();
                let result = clear_session_impl(&state, false, None, crate::UnknownFilesArg::Keep, None).await;
                // Task 1882: the menu's sign-out names the folder of kept
                // Finder files in the same alert as the `clear_session` command,
                // also when it failed after the removal (review I1), and saves it for
                // the Settings › Sync row (review I2).
                surface_sign_out_folders(&app, &result);
                // FB-24 (M6): on every platform the person sees a sign-out that stopped (its error) and one that
                // happened with a step it could not confirm (its warning); before, only Windows showed either.
                match &result {
                    // Spec 2026-10-10 §4 row 5: a waiting change opens the flow in Settings; never the error dialog.
                    #[cfg(target_os = "macos")]
                    Err(error) if error.message.starts_with("unsent_changes: ") => open_sign_out_flow(&app),
                    Err(error) => {
                        app.dialog()
                            .message(error.message.clone())
                            .title("Sign-out paused")
                            .kind(tauri_plugin_dialog::MessageDialogKind::Error)
                            .show(|_| {});
                    }
                    Ok(SignedOut {
                        warning: Some(warning), ..
                    }) => {
                        app.dialog()
                            .message(warning.sentence())
                            .title("Signed out")
                            .kind(tauri_plugin_dialog::MessageDialogKind::Info)
                            .show(|_| {});
                    }
                    // Nothing to tear down — tell the user instead of running
                    // (and possibly failing) the full teardown (Bug B). Windows only, as before.
                    #[cfg(target_os = "windows")]
                    Ok(SignedOut {
                        outcome: SignOutOutcome::NotSignedIn,
                        warning: None,
                        ..
                    }) => {
                        app.dialog()
                            .message("You are not signed in on this device.")
                            .title("Sign out")
                            .kind(tauri_plugin_dialog::MessageDialogKind::Info)
                            .show(|_| {});
                    }
                    Ok(_) => {}
                }
                result.map(|_| ()).map_err(|failure| failure.message)
            });
        }
        DesktopMenuAction::Quit => app.exit(0),
        DesktopMenuAction::OpenFolder => {
            let app = app.clone();
            // Off the main thread: on macOS the open waits up to 3 s for the bridge gate and 5 s for LaunchServices.
            // A failure is shown (task 1885, I2) before it is logged: the menu has no window to put it in.
            spawn_menu_task(spec.id, async move {
                let result = on_the_blocking_pool(OPEN_FOLDER_FAILED, open_current_finder_location).await;
                show_menu_open_failure(&app, &result);
                result
            });
        }
        DesktopMenuAction::UploadFiles => {
            let app = app.clone();
            spawn_menu_task(spec.id, async move { upload_files_to_sync_root_impl(&app).map(|_| ()) });
        }
        DesktopMenuAction::NewFolder => log_menu_result(spec.id, create_folder_in_sync_root_impl(app).map(|_| ())),
        DesktopMenuAction::OpenWebApp => log_menu_result(spec.id, open_external_url(app, WEB_APP_URL)),
        DesktopMenuAction::OpenView(view) => log_menu_result(spec.id, open_menu_view(app, view)),
        // F11 and F15: View › Home opens the Beebeeb window on Home; View › Back leaves Settings in an open window.
        DesktopMenuAction::NavigateMain(nav) => {
            #[cfg(target_os = "macos")]
            match nav {
                MainMenuNav::Home => show_main_window(app, MainNav::Home),
                MainMenuNav::Back => back_in_main_window(app),
            }
            #[cfg(not(target_os = "macos"))]
            let _ = nav;
        }
        DesktopMenuAction::Zoom(action) => log_menu_result(spec.id, apply_menu_zoom(app, action)),
        DesktopMenuAction::OpenExternal(url) => log_menu_result(spec.id, open_external_url(app, url)),
        DesktopMenuAction::ReportProblem => {
            // Off the main thread: the bundle build scans every known name.
            let app = app.clone();
            spawn_menu_task(spec.id, async move { report_problem_impl(&app).map(|_| ()) });
        }
    }
}

pub(super) fn spawn_menu_task<F>(menu_id: &'static str, fut: F)
where
    F: std::future::Future<Output = Result<(), String>> + Send + 'static,
{
    tauri::async_runtime::spawn(async move {
        log_menu_result(menu_id, fut.await);
    });
}

/// A failed "Open in Finder" from the menu, shown to the person (task 1885, I2): a native alert, because the menu works
/// with every window closed and so has no window to put a toast in. It is the Sign-out menu action's kind of alert, and
/// carries the button's one sentence, never the error behind it (which is logged by the caller).
pub(super) fn show_menu_open_failure(app: &tauri::AppHandle, result: &Result<(), String>) {
    let Some((title, message)) = finder_open::menu_open_failure_alert(result, cfg!(target_os = "macos")) else {
        return;
    };
    app.dialog()
        .message(message)
        .title(title)
        .kind(tauri_plugin_dialog::MessageDialogKind::Error)
        .show(|_| {});
}

pub(super) fn log_menu_result(menu_id: &str, result: Result<(), String>) {
    if let Err(error) = result {
        tracing::warn!(menu_id, %error, "native menu action failed");
    }
}

pub(super) fn open_external_url(app: &tauri::AppHandle, url: &str) -> Result<(), String> {
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| format!("open {url}: {e}"))
}

pub(super) fn open_menu_view(app: &tauri::AppHandle, view: DesktopMenuView) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let nav = menu_view_nav_target(view, WINDOWS_MAIN_APP_VIEW_ROUTES);
        show_main_app_window_with_nav(app, Some(nav));
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    {
        let page = compact_menu_page_for_view(view);
        show_compact_app_window_with_nav(app, Some(page));
        Ok(())
    }
}

pub(super) fn apply_menu_zoom(app: &tauri::AppHandle, action: MenuZoomAction) -> Result<(), String> {
    let scale = {
        let state = app.state::<AppState>();
        let mut guard = state
            .menu_zoom_scale
            .lock()
            .map_err(|_| "menu zoom mutex poisoned".to_string())?;
        let next = next_menu_zoom_scale(*guard, action);
        *guard = next;
        next
    };

    let mut applied = false;
    for (label, window) in app.webview_windows() {
        if label == "tray" {
            continue;
        }
        if let Err(error) = window.set_zoom(scale) {
            tracing::warn!(window = %label, %error, "could not apply native menu zoom");
        } else {
            applied = true;
        }
    }

    if !applied {
        show_main_app_window_impl(app);
        // A Mac's main window is the Beebeeb window (F12); Windows and Linux keep today's order.
        #[cfg(target_os = "macos")]
        let labels = ["main"];
        #[cfg(not(target_os = "macos"))]
        let labels = ["main-app", "settings"];
        for label in labels {
            if let Some(window) = app.get_webview_window(label) {
                window
                    .set_zoom(scale)
                    .map_err(|e| format!("apply zoom to {label}: {e}"))?;
                return Ok(());
            }
        }
    }

    Ok(())
}

// ── System tray ──────────────────────────────────────────────────────────────

/// The b's menu and tooltip, from the same state the window draws (DW-1230, DW-1231). The words are the ones
/// `src/v2/tray/model.ts` gives the flyout, so the two surfaces cannot disagree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TrayMenuState {
    pub(super) signed_in: bool,
    pub(super) paused: bool,
    pub(super) locked: bool,
    pub(super) explorer_ready: bool,
    /// Which b Windows draws (TrayArt's `TrayState`). Locked and the quiet states keep the plain b.
    pub(super) icon: &'static str,
    /// The state in two or three words: the tooltip and the menu's first (disabled) line.
    pub(super) label: &'static str,
    pub(super) tooltip: &'static str,
}

impl TrayMenuState {
    pub(super) fn pause_label(&self) -> &'static str {
        if self.paused { "Resume syncing" } else { "Pause syncing" }
    }

    pub(super) fn lock_label(&self) -> &'static str {
        if self.locked { "Unlock Beebeeb" } else { "Lock Beebeeb" }
    }
}

/// What the b reads. `conflicts` is the one field that needs the state DB; the rest comes from the session and the last
/// `engine-status` the popover mirrors.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct TrayInputs {
    pub(super) signed_in: bool,
    pub(super) locked: bool,
    pub(super) paused: bool,
    pub(super) explorer_ready: bool,
    pub(super) update_available: bool,
    pub(super) conflicts: u32,
    pub(super) engine_state: String,
    pub(super) files_remaining: Option<u32>,
}

/// The same precedence the flyout uses (`popover_phase`, then `flyoutView` over it), for what the tray can read:
/// signed out, locked, paused, offline, stopped, a decision needed, an update, syncing, up to date. The storage
/// summary stays in the popover snapshot; the flyout shows its full and almost-full states, the tooltip does not.
pub(super) fn tray_menu_state(inputs: &TrayInputs) -> TrayMenuState {
    let (icon, label, tooltip) = if !inputs.signed_in {
        ("stopped", "Signed out", "Beebeeb: not signed in")
    } else if inputs.locked {
        ("up-to-date", "Locked", "Beebeeb: locked")
    } else if inputs.paused {
        ("paused", "Paused", "Beebeeb: paused")
    } else if inputs.engine_state == "offline" {
        ("offline", "Offline", "Beebeeb: no connection")
    } else if inputs.engine_state == "error" {
        ("stopped", "Stopped and storage full", "Beebeeb: stopped")
    } else if inputs.conflicts > 0 {
        ("needs-decision", "Needs a decision", "Beebeeb: needs a decision")
    } else if inputs.update_available {
        ("up-to-date", "Update available", "Beebeeb: update available")
    } else if inputs.engine_state == "syncing" || inputs.files_remaining.unwrap_or(0) > 0 {
        ("syncing", "Syncing", "Beebeeb: syncing")
    } else {
        ("up-to-date", "Up to date", "Beebeeb: up to date")
    };
    TrayMenuState {
        signed_in: inputs.signed_in,
        paused: inputs.paused,
        locked: inputs.locked,
        explorer_ready: inputs.explorer_ready,
        icon,
        label,
        tooltip,
    }
}

/// The current inputs, the way `popover_snapshot` reads them: the session, the live pause flag, the engine mirror,
/// the cached storage summary and the last update offer. No network and no fetch.
fn tray_inputs_now<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> TrayInputs {
    let state = app.try_state::<AppState>();
    let (signed_in, locked, paused) = match state.as_deref() {
        Some(state) => {
            let acct = state.active_account().ok();
            let session = acct
                .as_ref()
                .and_then(|acct| acct.session.lock().ok().map(|guard| guard.is_some()))
                .unwrap_or(false);
            let paused = acct
                .as_ref()
                .map(|acct| acct.sync_paused.load(Ordering::Relaxed))
                .unwrap_or(false);
            let auth_present = state.auth_present.lock().map(|guard| *guard).unwrap_or(false);
            let signed_in = session || auth_present;
            (signed_in, signed_in && !session, paused)
        }
        None => (false, false, false),
    };
    let (engine_state, files_remaining) = app
        .try_state::<popover_data::PopoverRuntime>()
        .and_then(|runtime| {
            runtime
                .status
                .lock()
                .ok()
                .map(|view| (view.state.clone(), view.files_remaining))
        })
        .unwrap_or_default();
    TrayInputs {
        signed_in,
        locked,
        paused,
        explorer_ready: true,
        update_available: crate::macos_popover::update_offer().is_some(),
        conflicts: 0,
        engine_state,
        files_remaining,
    }
}

/// How many files wait for a decision. Read only when the menu is rebuilt, never on a timer.
fn tray_conflict_count() -> u32 {
    use state_db::FileStatus;
    DesktopConfig::load()
        .and_then(|cfg| state_db_for_config(&cfg))
        .ok()
        .flatten()
        .and_then(|db| db.list_by_status(FileStatus::Conflict).ok())
        .map(|rows| u32::try_from(rows.len()).unwrap_or(u32::MAX))
        .unwrap_or(0)
}

/// The cheap part of a rebuild: everything but the conflict count. Equal keys mean the menu would not change.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TrayKey {
    signed_in: bool,
    locked: bool,
    paused: bool,
    update_available: bool,
    engine_state: String,
    files_remaining: Option<u32>,
}

fn tray_key(inputs: &TrayInputs) -> TrayKey {
    TrayKey {
        signed_in: inputs.signed_in,
        locked: inputs.locked,
        paused: inputs.paused,
        update_available: inputs.update_available,
        engine_state: inputs.engine_state.clone(),
        files_remaining: inputs.files_remaining,
    }
}

/// The key of the menu currently on the tray (shared by the listener threads).
static TRAY_KEY: Mutex<Option<TrayKey>> = Mutex::new(None);

/// Rebuild the b's menu and tooltip when the state the window shows changed. `force` runs it for an event that is not
/// in the cheap key (a conflict arrived or resolved): the count needs the state DB, so it is read off the event thread.
pub(super) fn refresh_tray_menu<R: tauri::Runtime>(app: &tauri::AppHandle<R>, force: bool) {
    if app.tray_by_id("tray").is_none() {
        return;
    }
    let inputs = tray_inputs_now(app);
    if !force {
        let key = tray_key(&inputs);
        let mut last = TRAY_KEY.lock().unwrap_or_else(|poison| poison.into_inner());
        if last.as_ref() == Some(&key) {
            return;
        }
        *last = Some(key);
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let conflicts = tray_conflict_count();
        let inputs = TrayInputs {
            conflicts,
            ..tray_inputs_now(&app)
        };
        let state = tray_menu_state(&inputs);
        let main = app.clone();
        let _ = app.run_on_main_thread(move || apply_tray_menu(&main, &state));
    });
}

fn apply_tray_menu<R: tauri::Runtime>(app: &tauri::AppHandle<R>, state: &TrayMenuState) {
    let Some(tray) = app.tray_by_id("tray") else {
        return;
    };
    #[cfg(not(target_os = "macos"))]
    match build_tray_menu(app, app.autolaunch().is_enabled().unwrap_or(false), state) {
        Ok(menu) => {
            if let Err(error) = tray.set_menu(Some(menu)) {
                tracing::warn!(%error, "could not set the tray menu");
            }
        }
        Err(error) => tracing::warn!(%error, "could not build the tray menu"),
    }
    if let Err(error) = tray.set_tooltip(Some(state.tooltip)) {
        tracing::warn!(%error, "failed to update tray tooltip");
    }
}

/// Build the b's context menu from the state the window shows. Called on first setup and every time that state
/// changes, not only when someone clicks. Windows draws the menu; Beebeeb supplies these items, top to bottom:
/// the state, Open Beebeeb, Open in File Explorer, Pause/Resume syncing, Lock/Unlock Beebeeb, Check for updates…,
/// Settings…, Start at login, Quit Beebeeb.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub(super) fn build_tray_menu<R: tauri::Runtime, M: tauri::Manager<R>>(
    manager: &M,
    autostart_enabled: bool,
    state: &TrayMenuState,
) -> tauri::Result<Menu<R>> {
    // The first line is not a command: it says the same state as the tooltip. `open_settings` keeps its id for the
    // existing event wiring; only its label and target window changed.
    let status_item = MenuItemBuilder::new(state.label)
        .id("tray_status")
        .enabled(false)
        .build(manager)?;
    let show_item = MenuItemBuilder::new("Open Beebeeb")
        .id("open_settings")
        .build(manager)?;
    let explorer_item = MenuItemBuilder::new("Open in File Explorer")
        .id("tray_explorer")
        .enabled(state.explorer_ready)
        .build(manager)?;
    let pause_item = MenuItemBuilder::new(state.pause_label())
        .id("tray_pause")
        .enabled(state.signed_in)
        .build(manager)?;
    let lock_item = MenuItemBuilder::new(state.lock_label())
        .id("tray_lock")
        .enabled(state.signed_in)
        .build(manager)?;
    let check_item = MenuItemBuilder::new("Check for updates…")
        .id("tray_check_updates")
        .build(manager)?;
    let settings_item = MenuItemBuilder::new("Settings…").id("tray_settings").build(manager)?;
    let autostart_item = CheckMenuItemBuilder::new("Start at login")
        .id("tray_autostart")
        .checked(autostart_enabled)
        .build(manager)?;
    let sep1 = PredefinedMenuItem::separator(manager)?;
    let sep2 = PredefinedMenuItem::separator(manager)?;
    let quit_item = PredefinedMenuItem::quit(manager, Some("Quit Beebeeb"))?;
    Menu::with_items(
        manager,
        &[
            &status_item,
            &show_item,
            &explorer_item,
            &sep1,
            &pause_item,
            &lock_item,
            &check_item,
            &settings_item,
            &autostart_item,
            &sep2,
            &quit_item,
        ],
    )
}

/// What a failed Lock says, in the flyout: honest about the state the person is left in.
const TRAY_LOCK_FAILED: &str = "Beebeeb couldn’t lock, so it stays unlocked.";
/// What a failed Unlock says: the lock stays, and the window can still unlock.
const TRAY_UNLOCK_FAILED: &str = "Beebeeb couldn’t unlock. Unlock it in the Beebeeb window.";

/// A refusal in the b's menu is said in the flyout, never dropped: the menu is Windows' and has no line of its own,
/// so this opens the flyout and tells it the one sentence. It stays until the next action, as the flyout's own do.
fn report_tray_refusal<R: tauri::Runtime>(app: &tauri::AppHandle<R>, sentence: &str) {
    if let Some(win) = app.get_webview_window("tray") {
        let _ = win.show();
        let _ = win.set_focus();
    }
    if let Err(error) = app.emit("tray-menu-notice", sentence) {
        tracing::warn!(%error, "could not report the tray menu's refusal in the flyout");
    }
}

/// Attach the listeners that keep the b's tooltip and menu on the state the window shows: every `engine-status`
/// (cheap: the mirror is already folded in), and a conflict arriving or resolving (the count needs the state DB).
pub(super) fn attach_tray_status_listener<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    use tauri::Listener;
    let app_for_listener = app.clone();
    app.listen("engine-status", move |event| {
        let payload: serde_json::Value = match serde_json::from_str(event.payload()) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "engine-status payload not valid JSON");
                return;
            }
        };
        // Task 1683 slice 2: `state` can now be `syncing` or `offline`. Nothing a user
        // sees today may change before the flip (slice 6), so `sync_status` keeps reading
        // the state the old emitter would have sent (`engine_status::legacy_state`).
        let state = engine_status::legacy_state(&payload);

        // The popover snapshot reads the NEW fields (real state, progress, last check,
        // reason) from this mirror.
        if let Some(runtime) = app_for_listener.try_state::<popover_data::PopoverRuntime>()
            && let Ok(mut view) = runtime.status.lock()
        {
            view.apply(&payload);
        }

        // Mirror the latest state into the active account's `engine_state`
        // (per-account, decision 0800) so the `sync_status` command can return
        // it on first paint without subscribing to the event stream itself.
        // Failures here (poisoned mutex, registry not synthesized yet) are
        // non-fatal — the b's tooltip update below still runs.
        if !state.is_empty()
            && let Some(app_state) = app_for_listener.try_state::<AppState>()
            && let Ok(acct) = app_state.active_account()
            && let Ok(mut guard) = acct.engine_state.lock()
        {
            *guard = state.to_string();
        }

        refresh_tray_menu(&app_for_listener, false);
    });

    for event_name in ["engine-conflict", "conflict-resolved", "conflict-auto-resolved"] {
        let listener = app.clone();
        app.listen(event_name, move |_event| refresh_tray_menu(&listener, true));
    }
}

pub(super) fn setup_tray(app: &mut tauri::App) -> tauri::Result<()> {
    // The same state the window draws, at launch: the first frame's menu is not a guess (the conflict count is
    // filled in by the refresh just below).
    let initial = tray_menu_state(&tray_inputs_now(app.handle()));
    // Read current autostart state so the check mark is correct on launch.
    #[cfg(not(target_os = "macos"))]
    let tray_menu = build_tray_menu(app, app.autolaunch().is_enabled().unwrap_or(false), &initial)?;

    // The tray ICON itself is built declaratively from `app.trayIcon` in
    // tauri.conf.json (iconPath, iconAsTemplate, tooltip,
    // showMenuOnLeftClick) — Tauri's `App::build()` constructs it from that
    // config BEFORE this `.setup()` closure runs. Building a second,
    // fully-independent `TrayIconBuilder::with_id("tray")` here — as this
    // function used to — registers a SECOND native status item with the
    // same string id (two blank/duplicate icons in the menu bar, task
    // 1384). Attach the menu + click behaviour to the config-built tray
    // instead of constructing a new one.
    let tray = app
        .tray_by_id("tray")
        .expect("app.trayIcon in tauri.conf.json did not register a tray with id \"tray\"");

    #[cfg(not(target_os = "macos"))]
    tray.set_menu(Some(tray_menu))?;
    let _ = tray.set_tooltip(Some(initial.tooltip));
    // The initial menu was built without the state DB read; this fills in a waiting conflict at launch.
    refresh_tray_menu(&app.handle().clone(), true);

    tray.on_menu_event(|app, event| match event.id().as_ref() {
        "open_settings" => show_main_app_window_impl(app),
        "tray_explorer" => {
            let app = app.clone();
            // Off the main thread, as the menu's own Open Folder is; a failure is shown, never dropped.
            spawn_menu_task("tray_explorer", async move {
                let result = on_the_blocking_pool(OPEN_FOLDER_FAILED, open_current_finder_location).await;
                show_menu_open_failure(&app, &result);
                result
            });
        }
        "tray_pause" => {
            let app = app.clone();
            // The same path the window's Pause uses (`set_sync_paused`); a refusal opens the flyout and says it there.
            spawn_menu_task("tray_pause", async move {
                let (pausing, result) = toggle_sync_paused_from_menu(&app).await;
                if result.is_err() {
                    report_tray_refusal(
                        &app,
                        if pausing {
                            MENU_PAUSE_FAILED_SENTENCE
                        } else {
                            MENU_RESUME_FAILED_SENTENCE
                        },
                    );
                }
                result
            });
        }
        "tray_lock" => {
            let app = app.clone();
            // Lock/Unlock is the window's own command (`lock_vault` / `unlock_vault`), read at click time so a stale
            // menu cannot lock twice.
            spawn_menu_task("tray_lock", async move {
                let result = if tray_inputs_now(&app).locked {
                    let state = app.state::<AppState>();
                    match unlock_vault(app.clone(), state).await {
                        Ok(()) => Ok(()),
                        Err(error) => {
                            report_tray_refusal(&app, TRAY_UNLOCK_FAILED);
                            Err(error)
                        }
                    }
                } else {
                    let state = app.state::<AppState>();
                    match lock_vault(state).await {
                        Ok(_) => Ok(()),
                        Err(error) => {
                            report_tray_refusal(&app, TRAY_LOCK_FAILED);
                            Err(error)
                        }
                    }
                };
                // A lock or unlock changes the menu at once, not at the next engine pulse.
                refresh_tray_menu(&app, true);
                result
            });
        }
        // The same check the window's Settings › About runs, and the Beebeeb menu's Check for Updates….
        "tray_check_updates" => request_menu_update_check(app),
        // The window's Settings page, opened by the same helper the menu's Open Settings uses.
        "tray_settings" => show_main_app_window_with_nav(app, Some("settings")),
        "tray_hide" => match surfaces::policy::tray_hide(surfaces::policy::Platform::current()) {
            // A Mac hides the app (⌘H): the Beebeeb window and the menu-bar popover; the Dock or a reopen unhides.
            surfaces::policy::TrayHide::App =>
            {
                #[cfg(target_os = "macos")]
                if let Err(error) = app.hide() {
                    tracing::warn!(%error, "the tray's Hide could not hide the app");
                }
            }
            surfaces::policy::TrayHide::Window(label) => {
                if let Some(win) = app.get_webview_window(label) {
                    let _ = win.hide();
                }
            }
        },
        "tray_autostart" => {
            // Toggle autostart and rebuild the tray menu so the check
            // mark reflects the new state. TrayIcon::set_menu replaces
            // the whole menu; there is no per-item getter in Tauri 2.
            let manager = app.autolaunch();
            let currently = manager.is_enabled().unwrap_or(false);
            if currently {
                let _ = manager.disable();
            } else {
                let _ = manager.enable();
            }
            let new_state = !currently;
            let menu_state = tray_menu_state(&tray_inputs_now(app));
            if let Some(tray) = app.tray_by_id("tray")
                && let Ok(menu) = build_tray_menu(app, new_state, &menu_state)
            {
                let _ = tray.set_menu(Some(menu));
            }
            tracing::info!(enabled = new_state, "autostart toggled via tray");
        }
        _ => {}
    });

    tray.on_tray_icon_event(|tray, event| {
        // Left-click toggles window visibility.
        // On Windows: show/hide the frameless tray flyout ("tray" window),
        // positioned near the bottom-right corner so it appears above the
        // system tray area. The main app opens from the context menu or
        // the tray flyout itself.
        // On macOS: show/hide the menu-bar popover (`macos_popover::toggle`,
        // task 1683 slice 3), placed under the status item by
        // `surfaces::anchor` from the status item's own frame.
        // On Linux: toggle the compact app window exactly as before.
        if let TrayIconEvent::Click {
            button,
            button_state: MouseButtonState::Up,
            position,
            rect,
            ..
        } = event
        {
            let app = tray.app_handle();
            #[cfg(not(target_os = "macos"))]
            if button != MouseButton::Left {
                return;
            }
            #[cfg(target_os = "macos")]
            let _ = button;

            #[cfg(target_os = "windows")]
            {
                let _ = &rect; // unused — Windows anchors from the click point
                if let Some(win) = app.get_webview_window("tray") {
                    if win.is_visible().unwrap_or(false) {
                        let _ = win.hide();
                    } else {
                        // Position the flyout above the tray icon.
                        // The tray event's position is physical desktop
                        // coordinates. The tray window size in tauri.conf.json
                        // is logical, so resolve the scale from the monitor
                        // containing the click; this handles a tray on a
                        // non-primary monitor or a hidden window whose current
                        // monitor is stale. If monitor lookup fails, fall back
                        // to the window scale, then 1.0.
                        let scale_factor = windows_tray_flyout_scale_factor(&win, position.x, position.y);
                        let flyout_position = windows_tray_flyout_position(position.x, position.y, scale_factor);
                        let _ = win.set_position(tauri::Position::Physical(flyout_position));
                        let _ = win.show();
                        let _ = win.set_focus();
                    }
                }
            }

            #[cfg(target_os = "macos")]
            {
                let _ = (position, rect);
                macos_popover::toggle(app);
            }

            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            {
                let _ = position;
                let _ = &rect;
                if let Some(win) = app.get_webview_window("settings") {
                    if win.is_visible().unwrap_or(false) {
                        let _ = win.hide();
                    } else {
                        show_compact_app_window_impl(app);
                    }
                } else {
                    show_compact_app_window_impl(app);
                }
            }
        }
    });

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Internal helper for non-Windows builds, where the compact app/settings
/// shell is still the only primary window.
#[cfg(not(target_os = "windows"))]
pub(super) fn show_compact_app_window_impl(app: &tauri::AppHandle) {
    show_compact_app_window_with_nav(app, None);
}

/// Internal helper for non-Windows builds, optionally selecting one of the
/// compact app's existing pages (`status`, `finder`, `shared`, ...).
///
/// On macOS this window doubles as BOTH the tray flyout and the regular
/// Settings/Preferences window (task 1384; review follow-up after 8d7b5a1
/// applied the flyout's chrome-less/always-on-top/hide-on-focus-loss
/// treatment to every macOS open, breaking "Open Beebeeb", the Preferences/
/// Settings menu items, and `show_main_app_window`). Which behavior it gets
/// is decided per-open by `macos_window_mode_for_anchor`, purely from
/// whether `MACOS_TRAY_FLYOUT_ANCHOR` has a pending position (consumed here
/// via `take()`; the tray click set it before the menu-bar popover replaced
/// the flyout, and nothing sets it now, so every open is `Regular`):
/// - `Popover` (anchor present): chrome-less, always-on-top, positioned
///   under the status item, hides on focus loss — reads as a system
///   popover.
/// - `Regular` (no anchor — "Open Beebeeb", Preferences/Settings menu
///   items, a notification, a deep link): decorated, centered, normal
///   window-manager behavior, stays open when focus moves elsewhere.
///
/// The window is built once and reused for every later open, so each open
/// explicitly re-applies decorations/always-on-top/position for the mode in
/// effect (`MACOS_FLYOUT_IS_POPOVER` records which one, for the
/// once-registered focus-loss handler to read live) rather than assuming
/// whatever the window happened to be built with.
///
/// The `?platform=macos` URL tag lets the frontend force the sidebar+
/// content two-column grid regardless of this window's 680px width — see
/// `design.css`'s `html.macos-flyout` rule; without it, the existing
/// `max-width: 820px` responsive breakpoint (meant for the browser/
/// main-app case) collapsed to a single stacked column and reproduced
/// 1173's overflow. Linux keeps the original centered/decorated/no-anchor
/// behaviour untouched.
#[cfg(not(target_os = "windows"))]
pub(super) fn show_compact_app_window_with_nav(app: &tauri::AppHandle, nav: Option<&str>) {
    let (label, width, height, resizable) = (
        surfaces::policy::compact_window_label(surfaces::policy::Platform::current()),
        MACOS_SETTINGS_WINDOW_WIDTH,
        MACOS_SETTINGS_WINDOW_HEIGHT,
        false,
    );
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut url = nav
        .map(|nav| format!("index.html?nav={nav}"))
        .unwrap_or_else(|| "index.html".to_string());
    #[cfg(target_os = "macos")]
    url.push_str(if nav.is_some() {
        "&platform=macos"
    } else {
        "?platform=macos"
    });

    #[cfg(target_os = "macos")]
    let pending_anchor = MACOS_TRAY_FLYOUT_ANCHOR.lock().unwrap().take();
    #[cfg(target_os = "macos")]
    let mode = macos_window_mode_for_anchor(pending_anchor);
    // `swap` both updates the flag for the focus-loss handler and hands back
    // the mode the window was actually left in after its last open, so a
    // Regular open can tell "just switched out of Popover" (needs a
    // recenter — the leftover tray-anchor position assumed no title bar and
    // is often right under the menu bar) from "still Regular" (the user may
    // have dragged this decorated window; leave it exactly where it is, as
    // the pre-1384 behavior did for every reopen).
    #[cfg(target_os = "macos")]
    let was_popover = MACOS_FLYOUT_IS_POPOVER.swap(mode == MacosWindowMode::Popover, Ordering::Relaxed);

    if let Some(win) = app.get_webview_window(label) {
        // The window already exists (this is at least its second open) —
        // make the mode switch explicit rather than leaving it however the
        // previous open left it.
        #[cfg(target_os = "macos")]
        match mode {
            MacosWindowMode::Popover => {
                let _ = win.set_decorations(false);
                let _ = win.set_always_on_top(true);
                if let Some(position) = pending_anchor {
                    let _ = win.set_position(tauri::Position::Physical(position));
                }
            }
            MacosWindowMode::Regular => {
                let _ = win.set_always_on_top(false);
                let _ = win.set_decorations(true);
                if was_popover {
                    let _ = win.center();
                }
            }
        }
        // The tray's Hide hides the app; a window shown in a hidden app stays hidden (review round 2, P2-2).
        #[cfg(target_os = "macos")]
        unhide_app(app);
        let _ = win.show();
        let _ = win.set_focus();
        if let Some(nav) = nav {
            let _ = win.emit("compact-app:navigate", nav);
        }
        return;
    }

    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut builder = tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::App(url.into()))
        .title("Beebeeb")
        .inner_size(width, height)
        .resizable(resizable)
        .center();

    #[cfg(target_os = "macos")]
    {
        // Built invisible either way, so the placement applied after
        // `.build()` below — the anchor position for a popover, or just the
        // `.center()` above for a regular window — never flashes at the
        // wrong spot first.
        builder = match mode {
            MacosWindowMode::Popover => builder.decorations(false).always_on_top(true).visible(false),
            MacosWindowMode::Regular => builder.decorations(true).always_on_top(false).visible(false),
        };
    }

    match builder.build() {
        Ok(win) => {
            #[cfg(target_os = "macos")]
            {
                if mode == MacosWindowMode::Popover {
                    if let Some(position) = pending_anchor {
                        let _ = win.set_position(tauri::Position::Physical(position));
                    }
                }

                // Hide on focus loss, like a system popover — but only
                // while the window is currently in popover mode
                // (`MACOS_FLYOUT_IS_POPOVER`); a regular open (Preferences,
                // "Open Beebeeb", the Settings menu item) must stay open
                // when focus moves elsewhere. Registered once, here, at
                // first build — the window is reused for every later open,
                // so this reads the mode flag live on each focus change
                // rather than one captured at registration time. Also
                // debounced against the spurious immediate blur some window
                // managers deliver right as a borderless always-on-top
                // window is shown (see `MACOS_FLYOUT_GAINED_FOCUS`).
                let hide_target = win.clone();
                win.on_window_event(move |event| {
                    if let tauri::WindowEvent::Focused(focused) = event {
                        if *focused {
                            MACOS_FLYOUT_GAINED_FOCUS.store(true, Ordering::Relaxed);
                        } else if MACOS_FLYOUT_GAINED_FOCUS.swap(false, Ordering::Relaxed)
                            && MACOS_FLYOUT_IS_POPOVER.load(Ordering::Relaxed)
                        {
                            let _ = hide_target.hide();
                        }
                    }
                });
            }

            #[cfg(target_os = "macos")]
            unhide_app(app);
            let _ = win.show();
            let _ = win.set_focus();
            if let Some(nav) = nav {
                let _ = win.emit("compact-app:navigate", nav);
            }
        }
        Err(error) => {
            tracing::warn!(%error, "failed to create compact app window");
        }
    }
}

/// What a route-named opener (`show_main_app_window_with_nav`) asks of the Beebeeb window on a Mac: "settings" (or a
/// settings page, "settings/<page>") opens Settings (the last section used), anything else Home.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(super) fn main_nav_for_route(nav: Option<&str>) -> MainNav {
    match nav {
        Some(nav) if nav == "settings" || nav.starts_with("settings/") => MainNav::settings(None),
        _ => MainNav::Home,
    }
}

/// The settings pages an opener in another window may name (desktop v2's `section=settings&page=<page>`, M3e): the
/// setup window's summary rows, the menu's Check for Updates. The word goes into the window's URL, so only these: the
/// window's own page words (`PAGE_WORDS`, src/v2/sections/settings/shell/links.ts; tests/v2M3eWiring.test.ts keeps the
/// two lists equal). Compare and a device's page need a device, so no opener names them.
pub(super) const SETTINGS_PAGES: &[&str] = &[
    "general",
    "about",
    "account",
    "security",
    "devices",
    "notifications",
    "storage",
    "backups",
    "sync",
    "thumbnails",
    "bandwidth",
    "insights",
    "recent",
    "location",
    "diagnostics",
    "backup-failures",
    "report",
    "free-up",
    "signin-activity",
    "mail-backup",
];

/// The nav word for Settings, on `page` when one is named: "settings" or "settings/<page>". A page that isn't one of
/// [`SETTINGS_PAGES`] is refused, never passed on.
pub(super) fn settings_nav(page: Option<&str>) -> Result<String, String> {
    match page {
        None => Ok("settings".to_string()),
        Some(page) if SETTINGS_PAGES.contains(&page) => Ok(format!("settings/{page}")),
        Some(_) => Err("That settings page is not available.".to_string()),
    }
}

/// Open (or focus) the main Beebeeb app window on Settings: on `page` when one is named (desktop v2's
/// `section=settings&page=<page>`; the setup window's summary rows open their own page), else General. Used by the
/// Windows tray flyout's Settings and the setup window. A page that isn't a settings page is refused. (Here, beside
/// `settings_nav`, since M3e: settings_commands.rs is at its size limit.)
#[tauri::command]
pub(super) fn show_main_app_settings(app: tauri::AppHandle, page: Option<String>) -> Result<(), String> {
    let nav = settings_nav(page.as_deref())?;
    show_main_app_window_with_nav(&app, Some(&nav));
    Ok(())
}

/// Desktop v2 (M3e): the main window's status line shows the b's flyout (DW-0401#10 → DW-1201), the "tray" window
/// the b's click shows, at the bottom-right of the work area of the display the asking window is on (the click on the
/// b anchors from the click instead). Windows only: elsewhere the flyout is the Mac's popover or the compact window.
#[tauri::command]
pub(super) fn show_tray_flyout(app: tauri::AppHandle, window: tauri::WebviewWindow) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let flyout = app
            .get_webview_window("tray")
            .ok_or_else(|| "The flyout isn't available.".to_string())?;
        let monitor = window
            .current_monitor()
            .ok()
            .flatten()
            .or_else(|| window.primary_monitor().ok().flatten())
            .ok_or_else(|| "No display to show the flyout on.".to_string())?;
        let area = monitor.work_area();
        let position = windows_flyout_position_in_work_area(
            area.position.x,
            area.position.y,
            area.size.width,
            area.size.height,
            monitor.scale_factor(),
        );
        let _ = flyout.set_position(tauri::Position::Physical(position));
        let _ = flyout.show();
        let _ = flyout.set_focus();
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (app, window);
        Err("The flyout is available on Windows.".into())
    }
}

/// Internal helper — show/create the main Beebeeb app window.
///
pub(super) fn show_main_app_window_impl(app: &tauri::AppHandle) {
    show_main_app_window_with_nav(app, None);
}

/// Internal helper — show/create the main Beebeeb app window, optionally
/// selecting a route once it is visible. On Windows this is the `main-app`
/// webview (the full sidebar + content-router shell, `WindowsApp.tsx`,
/// selected by `?window=main-app&platform=windows`). On other platforms there
/// is no dedicated main-app shell yet, so we delegate to the compact app window.
///
/// Focuses an existing window if present, otherwise builds one. The window is
/// declared `visible: false` in tauri.conf.json, so the explicit `show()` +
/// `set_focus()` are required.
pub(super) fn show_main_app_window_with_nav(app: &tauri::AppHandle, nav: Option<&str>) {
    // Task 1683 slice 1 (two-surface spec §7): on a Mac every opener that showed the compact window shows the Beebeeb
    // window, on Settings for "settings" and on Home otherwise. Until slice 3 only the tray click still shows the
    // compact flyout.
    #[cfg(target_os = "macos")]
    show_main_window(app, main_nav_for_route(nav));

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        // No dedicated main-app window on Linux: reuse the compact app. There is no separate compact Settings page;
        // Preferences and Settings both intentionally focus the same compact app window.
        let compact_nav = match nav {
            Some("settings") | None => None,
            Some(page) if page.starts_with("settings/") => None,
            Some(other) => Some(other),
        };
        show_compact_app_window_with_nav(app, compact_nav);
    }

    #[cfg(target_os = "windows")]
    {
        let label = surfaces::policy::main_window_label(surfaces::policy::Platform::current());
        let url = match nav {
            Some(nav) => format!("index.html?window=main-app&platform=windows&nav={nav}"),
            None => "index.html?window=main-app&platform=windows".to_string(),
        };

        if let Some(win) = app.get_webview_window(label) {
            let _ = win.show();
            let _ = win.set_focus();
            if let Some(nav) = nav {
                let _ = win.emit("windows-app:navigate", nav);
            }
            return;
        }

        match tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::App(url.into()))
            .title("Beebeeb")
            .inner_size(1280.0, 800.0)
            .min_inner_size(960.0, 640.0)
            .resizable(true)
            .visible(false)
            .build()
        {
            Ok(win) => {
                place_desktop_main_window(app, &win);
                let _ = win.show();
                let _ = win.set_focus();
                if let Some(nav) = nav {
                    let _ = win.emit("windows-app:navigate", nav);
                }
            }
            Err(error) => {
                tracing::warn!(%error, "failed to create main app window");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(update: impl FnOnce(&mut TrayInputs)) -> TrayInputs {
        let mut inputs = TrayInputs {
            signed_in: true,
            explorer_ready: true,
            ..TrayInputs::default()
        };
        update(&mut inputs);
        inputs
    }

    #[test]
    fn tray_menu_says_syncing_while_files_move() {
        let state = tray_menu_state(&inputs(|inputs| {
            inputs.engine_state = "syncing".into();
            inputs.files_remaining = Some(12);
        }));
        assert_eq!(state.icon, "syncing");
        assert_eq!(state.label, "Syncing");
        assert_eq!(state.tooltip, "Beebeeb: syncing");
        assert_eq!(state.pause_label(), "Pause syncing");
        assert_eq!(state.lock_label(), "Lock Beebeeb");
    }

    #[test]
    fn tray_menu_says_locked_and_offers_unlock_with_the_plain_b() {
        let state = tray_menu_state(&inputs(|inputs| {
            inputs.locked = true;
            inputs.engine_state = "idle".into();
        }));
        assert_eq!(state.icon, "up-to-date", "DW-1230 keeps the plain b while locked");
        assert_eq!(state.label, "Locked");
        assert_eq!(state.tooltip, "Beebeeb: locked");
        assert_eq!(state.lock_label(), "Unlock Beebeeb");
        assert_eq!(
            state.pause_label(),
            "Pause syncing",
            "sync keeps running behind the lock"
        );
    }

    #[test]
    fn tray_menu_says_needs_a_decision_with_a_waiting_conflict() {
        let state = tray_menu_state(&inputs(|inputs| {
            inputs.conflicts = 2;
            inputs.engine_state = "idle".into();
        }));
        assert_eq!(state.icon, "needs-decision");
        assert_eq!(state.label, "Needs a decision");
        assert_eq!(state.tooltip, "Beebeeb: needs a decision");
    }

    #[test]
    fn tray_menu_says_up_to_date_when_nothing_is_happening() {
        let state = tray_menu_state(&inputs(|inputs| inputs.engine_state = "idle".into()));
        assert_eq!(state.icon, "up-to-date");
        assert_eq!(state.label, "Up to date");
        assert_eq!(state.tooltip, "Beebeeb: up to date");
        assert_eq!(state.pause_label(), "Pause syncing");
    }
}
