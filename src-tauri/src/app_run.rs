//! `app_run` items split out of `lib.rs`.

use super::*;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .try_init();

    let builder = tauri::Builder::default()
        // Shared state — session installed via apply_session (the
        // browser-login handoff) after login. The sync engine task reads
        // from this.
        .manage(AppState::default())
        // Task 1683 slice 2: transfer board, last engine status, cached storage summary.
        .manage(popover_data::PopoverRuntime::default())
        .plugin(tauri_plugin_opener::init())
        // 0.8.15: before any window exists, so its `on_webview_ready` sees every webview (Ctrl+Shift+P printed in 0.8.14).
        .plugin(webview_browser_keys::plugin())
        // v2 b10, task 2422: no web view offers to save or fill a password, a phrase or a code.
        .plugin(crate::auth_v2::webview_autofill::plugin());
    // Windows and Linux keep the window-state plugin; a Mac keeps the Beebeeb window's frame itself (task 1683 slice 1,
    // review item 5; see `window_state_plugin`).
    #[cfg(not(target_os = "macos"))]
    let builder = builder.plugin(window_state_plugin());
    builder
        // Native folder picker for the first-launch sync-root chooser.
        .plugin(tauri_plugin_dialog::init())
        // Native OS notifications — Task 11. The first conflict
        // notification on macOS triggers Apple's "allow notifications?"
        // permission prompt; we surface that, not a custom UI.
        .plugin(tauri_plugin_notification::init())
        // Auto-updater — endpoints + pubkey configured in tauri.conf.json
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Autostart — LaunchAgent on macOS (no elevated privileges required),
        // registry key on Windows, .desktop file on Linux
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec![])))
        .invoke_handler(tauri::generate_handler![
            drive_v2::drive_list, drive_v2::drive_details, drive_v2::drive_thumbnail, drive_v2::drive_versions, drive_v2::drive_search, drive_v2::drive_recent, drive_v2::drive_shared, drive_v2::drive_photos, drive_v2::drive_trash, drive_v2::drive_storage, drive_v2::drive_sync_summary, drive_v2::drive_create_folder, drive_v2::drive_rename, drive_v2::drive_move, drive_v2::drive_duplicate, drive_v2::drive_trash_items, drive_v2::drive_undo_trash, drive_v2::drive_restore_from_trash, drive_v2::drive_upload_pick, drive_v2::drive_upload_dropped, drive_v2::drive_open, drive_v2::drive_read_text, drive_v2::drive_save_text, drive_v2::drive_read_bytes, drive_v2::drive_save_bytes, drive_v2::drive_reveal, // v2 b02
            app_version,
            crate::v2_snap::v2_set_maximize_rect, // v2 b03
            account_v2::account_change_password, account_v2::account_change_email, account_v2::account_verify_email, account_v2::account_set_name, account_v2::account_totp_setup, account_v2::account_totp_confirm, account_v2::account_totp_disable, account_v2::account_totp_abandon, account_v2::account_set_activity_logging, account_v2::account_delete_check_password, account_v2::account_delete_confirm, account_v2::account_delete_cancel, account_v2::account_delete_undo, account_v2::account_display_name, account_v2::account_activity_logging, account_v2::account_save_backup_codes, // v2 b14
            crate::auth_v2::device::auth_device_request_start, crate::auth_v2::device::auth_device_request_poll, crate::auth_v2::device::auth_device_request_cancel, crate::auth_v2::approve::auth_approve_lookup, crate::auth_v2::approve::auth_approve_request, crate::auth_v2::hello::auth_windows_hello_available, crate::auth_v2::hello::auth_unlock_with_windows_hello, crate::auth_v2::phrase::auth_phrase_unknown_words, // v2 b10
            crate::passkey_commands::auth_passkey_sign_in, crate::passkey_commands::account_passkeys_list, crate::passkey_commands::account_passkeys_add, crate::passkey_commands::account_passkeys_remove, // v2 b15
            consume_main_navigation,
            set_pending_setup,
            pending_setup,
            #[cfg(target_os = "macos")]
            vault_unlock_route,
            main_window_state,
            check_for_updates_now,
            consume_menu_update_check,
            install_update,
            install_channel_downgrade,
            toggle_autostart,
            autostart_enabled,
            clear_session,
            unlock_vault,
            desktop_unlock_with_recovery_phrase,
            lock_vault,
            sync_status,
            popover_snapshot,
            macos_popover::popover_hide,
            macos_popover::popover_quit,
            macos_popover::popover_open_window,
            macos_popover::popover_retry_sync,
            macos_popover::popover_unlock,
            macos_popover::popover_gear_menu,
            desktop_storage_summary,
            free_up_space,
            export_diagnostics,
            list_version_conflict_center,
            dismiss_review_entry,
            list_file_versions,
            restore_file_version,
            get_sync_root,
            pick_sync_root,
            default_sync_root,
            desktop_platform,
            desktop_capabilities,
            finder_location_state,
            kept_unsynced_folder,
            dismiss_kept_unsynced_folder,
            // Spec 2026-10-10 §9: the kept folder's own row.
            kept_unsent_folder,
            dismiss_kept_unsent_folder,
            show_kept_unsent_folder,
            // Spec 2026-10-10 §5.1 and §6: the sign-out flow's count and send phase (a Mac; elsewhere they say so),
            // and the acknowledgement of a shown result (elsewhere nothing is pending, and it does nothing).
            sign_out_count,
            sign_out_unknown_files,
            sign_out_send_start,
            sign_out_send_stop,
            sign_out_result_ack,
            // §5.1: the compact page's and the menu's flow request, which Settings consumes (a Mac; elsewhere there is
            // no flow, and they say so).
            request_sign_out_flow,
            consume_sign_out_flow_request,
            // Spec 2026-10-06: the macOS Finder reconciler's commands (registered on every
            // platform; without a reconciler they say so).
            finder_setup_state,
            finder_setup_retry,
            finder_setup_copy_details,
            finder_setup_show_app,
            install_finder_location,
            continue_without_finder_location,
            // Task 1524 Issue 4 — user-disabled File Provider domain detection
            finder_domain_user_enabled,
            open_login_items_and_extensions_settings,
            // Windows shell integration (Cloud Files) — parallels the macOS finder cmds
            windows_shell_integration_state,
            install_windows_shell_integration,
            reset_macos_integration,
            open_finder_location,
            open_in_finder,
            // Task 9 — settings page IPC
            get_desktop_config,
            set_desktop_config,
            account_email,
            last_signed_in_email,
            // PKG-DATA — account / billing / devices / activity data layer.
            // Wrappers over existing server endpoints (the "all pages empty"
            // fix) plus the locally-computed storage breakdown.
            account_profile,
            account_profile_fresh,
            account_subscription,
            account_usage,
            account_region,
            account_set_region,
            account_activity,
            account_security_score,
            account_session_list,
            account_revoke_session,
            account_revoke_other_sessions,
            account_devices,
            account_client_sessions,
            account_activity_feed,
            account_notifications,
            account_notification_preferences,
            account_update_notification_preferences,
            desktop_confirm_action,
            desktop_trash_list,
            desktop_trash_restore,
            desktop_trash_delete_permanently,
            account_storage_breakdown,
            desktop_file_overview,
            desktop_recent_files,
            get_active_transfers,
            desktop_search_files,
            app_activity_snapshot,
            // Task 0090 — selective sync
            get_selective_sync,
            set_selective_sync,
            // Task 0797 — known-folder backup ("Manage backup")
            get_known_folder_backup,
            set_known_folder_backup,
            // LD-0113 Amendment 9 — the answer for a folder OneDrive moved
            crate::backup::commands::set_known_folder_source,
            // Task 0804 — known-folder backup first-run onboarding prompt
            get_known_folder_onboarding_seen,
            mark_known_folder_onboarding_seen,
            get_menu_bar_hint_seen,
            mark_menu_bar_hint_seen,
            list_shared_roots,
            list_vault_folders,
            list_remote_tree,
            set_recursive_pin,
            // Task 12 — conflict window IPC
            open_conflict_window,
            resolve_conflict,
            // Task 1546 finding 2 — real (non-placeholder) conflict content preview
            conflict_content_preview,
            parked_conflict_preview,
            resolve_parked_conflict,
            kept_conflict_copies,
            show_kept_conflict_copy,
            dismiss_kept_conflict_copy,
            // Task 11 — native conflict notification
            notify_conflict,
            // First-launch onboarding (login + folder picker + sync status)
            desktop_login,
            // 2FA second-factor completion for OPAQUE sign-in
            desktop_login_2fa,
            // Browser-based device-code login handoff (Windows primary path)
            browser_login::start_browser_login,
            open_onboarding_window,
            open_reauth_window,
            quit_beebeeb,
            // PKG-SHELL — open the Windows main app window
            show_main_app_window,
            show_main_app_settings,
            // WS1 — Windows UI aliases + tray commands
            desktop_config,
            tray_recent_activity,
            discard_and_download_again,
            tray_pause_sync,
            tray_resume_sync,
            sync_activity_summary,
            // Task 2409 — the "Not sent" list on Activity, and Try again
            unsent_changes,
            retry_unsent_change,
            // Task 2414 — the "Not downloaded" list on Activity
            not_downloaded,
            found_while_paused_review,
            found_while_paused_upload,
            found_while_paused_keep,
            show_kept_folder,
            upload_files_to_sync_root,
            create_folder_in_sync_root,
            report_problem,
            set_sync_mode,
            // Task 0812 — tray flyout: click file → reveal in Explorer + open
            reveal_and_open_file,
            // Task 0810 — Bandwidth view: speedtest + 20h traffic chart
            run_speedtest,
            get_bandwidth_history,
        ])
        .setup(|app| {
            let app_handle = app.handle().clone();
            state_paths::init_from_app(&app_handle).map_err(|e| tauri::Error::Anyhow(anyhow::anyhow!(e)))?;
            if let Ok(cfg) = DesktopConfig::load()
                && let Some(sync_root) = cfg.sync_root
                && let Err(error) = state_paths::prepare_beebeeb_state_dir_for_sync_root(&sync_root)
            {
                tracing::warn!(%error, sync_root = %sync_root.display(), "Beebeeb state migration did not complete during setup; runner will retry before opening state.db");
            }

            // Multi-account Phase 1 (decision 0800/0808): resolve the PERSISTED
            // account id, synthesize the single account under it, then lazily
            // migrate the legacy flat keychain trio into the id-segmented layout.
            // This ORDER is load-bearing and MUST run before anything reads
            // per-account credential state — the `auth_present` seed below, the
            // `restore_session_on_startup` task (which lands the keychain session
            // in `accounts[0].session`), and first-launch detection all depend on
            // both a resolvable active account AND the migration having run.
            //
            // Step 1 — persist/read the account id (mints + saves on first login,
            // returns the same value thereafter). On error we fall back to an
            // ephemeral id so the app still boots; the legacy READ FALLBACK in the
            // keychain helpers keeps auto-unlock working in that degraded case.
            let account_id = match DesktopConfig::ensure_account_id() {
                Ok(id) => id,
                Err(error) => {
                    tracing::warn!(%error, "could not persist account id; using ephemeral id this run");
                    AccountId::new_v4().0
                }
            };
            // Step 2 — synthesize the single account under the persisted id.
            // Idempotent; invisible at N=1.
            synthesize_single_account(&app.state::<AppState>(), AccountId(account_id.clone()));

            // Step 3 — lazy migration: legacy flat trio → id-segmented trio. Runs
            // SYNCHRONOUSLY here (decision 0808) BEFORE the auth seed + restore so
            // the auto-unlock below reads the segmented layout. Verify-before-delete
            // + the legacy READ FALLBACK are the two anti-lockout nets; a failure
            // here is LOGGED AND SWALLOWED (must not block startup — the fallback
            // keeps the vault usable).
            if let Err(error) = keychain::migrate_legacy_keychain_to_account(&account_id) {
                tracing::warn!(%error, "legacy keychain migration did not complete; legacy fallback remains in effect");
            }

            setup_native_menu(app)?;
            setup_tray(app)?;
            // Defensive: hide windows that older persisted state or other
            // startup paths may leave visible. Fixed auxiliary windows are now
            // excluded from window-state restore; main-app remains stateful.
            #[cfg(target_os = "windows")]
            {
                // macOS-labelled `settings` window is not used on Windows.
                if let Some(w) = app.get_webview_window("settings") {
                    let _ = w.hide();
                }
                // Tray popup — shown only on tray-icon click.
                if let Some(w) = app.get_webview_window("tray") {
                    let _ = w.hide();
                }
            }
            attach_tray_status_listener(&app.handle().clone());
            {
                let app_state = app.state::<AppState>();
                if let Ok(mut guard) = app_state.auth_present.lock() {
                    // Segmented under the persisted id (with the legacy READ
                    // FALLBACK inside `keychain_session_present` covering a not-yet-
                    // migrated install).
                    *guard = keychain_session_present(&account_id);
                }
            }

            // Spec 2026-10-06: the macOS Finder reconciler. One task for the app's life, created
            // before the restore below so the launch trigger always finds it.
            #[cfg(target_os = "macos")]
            {
                if let Some(dir) = lifecycle_log::default_dir() {
                    lifecycle_log::init(dir, lifecycle_known_names);
                }
                let launch = finder_setup::launch_location::current();
                lifecycle_log::event(lifecycle_log::LifecycleEvent::Launch {
                    app_version: real_app_version().to_string(),
                    macos_version: macos_version_string(),
                    launch_location: launch,
                });
                let persisted = DesktopConfig::load().ok().and_then(|cfg| cfg.finder_last_failure);
                let handle = finder_setup::driver::spawn(
                    finder_setup::macos_ports::MacosPorts::new(app.handle().clone()),
                    finder_setup::driver::SystemClock,
                    launch,
                    persisted,
                    finder_setup::policy::RetryPolicy::default(),
                );
                let _ = app.state::<AppState>().finder_setup.set(handle);
            }

            // Resume a fully unlocked session from the OS credential store.
            // When the per-user credential vault still holds the session token
            // AND the (DPAPI/Keychain-protected) raw master key, this loads the
            // key, unlocks the vault, and starts the engine — so quit+relaunch
            // returns to a working signed-in state with no recovery-phrase
            // prompt (the persistence guarantee users expect). When the master
            // key is genuinely absent, this is a no-op and onboarding still
            // routes to the recovery-phrase unlock step.
            {
                let h = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    restore_session_on_startup(&h).await;
                    // §5.3 (1): the launch check runs once the restore finished, whichever way it
                    // went, so it sees the keys the restore installed.
                    notify_finder(&h.state::<AppState>(), finder_setup::core::Trigger::Launch);
                });
            }

            // Task 1698 part 3 (closes 1696 / audit G8): sweep stale File
            // Provider domains at every app start — signed app context only
            // (the extension and the app both hold the identity; the
            // unsigned CLI cannot do this and never runs this hook).
            // Best-effort: a failure (no identity, transient NSFileProvider
            // Manager error) is logged and never blocks startup. Idempotent:
            // a clean registry removes nothing, and our own domain is never
            // a candidate.
            #[cfg(target_os = "macos")]
            {
                let alert_app = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    // Through the bridge gate like every other bridge call (lead ruling T8-gate-all).
                    match crate::finder_setup::macos_ports::cleanup_stale_domains().await {
                        Ok(cleanup) => {
                            if cleanup.removed_count() > 0 || !cleanup.skipped.is_empty() || !cleanup.ours_present {
                                tracing::info!(
                                    removed = cleanup.removed_count(),
                                    skipped = cleanup.skipped.len(),
                                    ours_present = cleanup.ours_present,
                                    preserved = cleanup.preserved_locations.len(),
                                    "stale File Provider domain sweep complete (task 1698)"
                                );
                            }
                            // Task 1882: a removed domain's un-synced files were kept;
                            // the alert names each folder (the log above only counts them).
                            for location in &cleanup.preserved_locations {
                                surface_kept_folder(&alert_app, Some(location));
                            }
                        }
                        Err(error) => {
                            tracing::warn!(
                                error = %error,
                                "stale File Provider domain sweep unavailable in this context (unsigned CLI gets -2001); \
                                 the next signed app start retries"
                            );
                        }
                    }
                });
            }

            // Spawn background update checker
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                run_update_loop(handle).await;
            });

            // First-launch detection: if no sync_root is configured and no
            // session is present, open the onboarding window automatically.
            // App windows stay hidden (visible:false in tauri.conf.json) until
            // onboarding or the configured-startup path opens the right one.
            let no_sync_root = DesktopConfig::load().map(|c| c.sync_root.is_none()).unwrap_or(true);
            // The decision is `surfaces::policy::startup_surface`: Windows and Linux ignore how the app was started; a Mac
            // follows it (task 1683 slice 6, two-surface spec §7.3). How it was started is `launch_source_now()`.
            let launch_source = launch_source_now();
            tracing::info!(?launch_source, no_sync_root, "launch");
            match surfaces::policy::startup_surface(surfaces::policy::Platform::current(), no_sync_root, launch_source) {
                surfaces::policy::StartupSurface::Onboarding => {
                    let h = app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        // Small delay so the tray and menu are fully initialised first.
                        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                        if let Err(e) = open_onboarding_window_impl(&h) {
                            tracing::warn!(error = %e, "failed to open onboarding window");
                        }
                    });
                }
                surfaces::policy::StartupSurface::MainWindow => {
                    // Already configured — show the main app window after startup
                    // settles. On Windows this is the full sidebar/content shell
                    // (`main-app`); on macOS/Linux `show_main_app_window_impl`
                    // delegates to the compact app window, so behaviour there
                    // is unchanged. The short delay mirrors onboarding and avoids
                    // racing Tauri's window setup.
                    let h = app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                        show_main_app_window_impl(&h);
                    });
                }
                // A Mac's login launch, or an update's relaunch with nothing open: the b in the menu bar is all there is.
                surfaces::policy::StartupSurface::Nothing => {
                    // The app starts Regular (a Dock icon and the app menu), and Accessory is otherwise applied only from window
                    // events: with no window to open, nothing would ever take them away (PR #146 review, P2).
                    #[cfg(target_os = "macos")]
                    set_macos_dock_icon(app.handle(), false);
                    tracing::info!("nothing opens by itself on this launch");
                }
            }

            Ok(())
        })
        .on_menu_event(|app, event| {
            if let Some(spec) = desktop_menu_spec(event.id().as_ref()) {
                handle_desktop_menu_action(app, spec);
            }
        })
        // Spec 2026-10-10 §5.1: a page load that starts in a window (a reload) ends its sign-out flow, as a close does.
        .on_page_load(|webview, payload| {
            #[cfg(target_os = "macos")]
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Started) {
                sign_out_flow_window_closed(webview.app_handle(), webview.label());
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (webview, payload);
        })
        // Red-dot close button: what happens is `surfaces::policy::close_policy`. Windows and Linux hide every
        // window (to the tray), as before. On a Mac the Beebeeb window (`main`) is destroyed (two-surface spec §4,
        // task 1683 slice 1) and every other window is hidden, as before.
        .on_window_event(|window, event| {
            // Spec §7: one read-only userEnabled check when the app becomes active.
            if let tauri::WindowEvent::Focused(true) = event
                && let Some(handle) = window.state::<AppState>().finder_setup.get()
            {
                handle.app_activated();
            }
            // Lead ruling 5 (Task 12): an unidentified session asks the server again when the app becomes active.
            #[cfg(not(target_os = "windows"))]
            if let tauri::WindowEvent::Focused(true) = event {
                tauri::async_runtime::spawn(on_app_activated(window.app_handle().clone()));
            }
            // Spec 2026-10-10 §5.1: a close (on macOS a hide), a destroy or a reload ends that window's sign-out flow.
            #[cfg(target_os = "macos")]
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                sign_out_flow_window_closed(window.app_handle(), window.label());
            }
            #[cfg(target_os = "macos")]
            if let tauri::WindowEvent::Destroyed = event {
                sign_out_flow_window_closed(window.app_handle(), window.label());
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if surfaces::policy::close_policy(surfaces::policy::Platform::current(), window.label())
                    == surfaces::registry::ClosePolicy::Hide
                {
                    api.prevent_close();
                    hide_on_close(window);
                    // Two-surface §9.1: a hidden onboarding or conflict window no longer holds the Dock icon.
                    #[cfg(target_os = "macos")]
                    refresh_macos_dock_icon(window.app_handle(), Some(window.label()));
                }
            }
            // Task 1683 slice 1: the Beebeeb window's end resets the menu and saves its frame; the Dock icon follows.
            #[cfg(target_os = "macos")]
            macos_window_lifecycle(window, event);
        })
        .build(tauri::generate_context!())
        .expect("error while building Beebeeb desktop")
        .run(|app, event| {
            // Review P1 on PR #134: on a Mac the Beebeeb window is destroyed on close, and when it was the last
            // window Tauri asks to exit (`code: None`). Beebeeb lives on in the menu bar; an explicit exit still exits.
            if let tauri::RunEvent::ExitRequested { code, api, .. } = &event
                && surfaces::policy::keep_running_on_exit_request(surfaces::policy::Platform::current(), *code)
            {
                api.prevent_exit();
            }
            // Task 1683 slice 1 (review item 4): a quit while the Beebeeb window is open keeps its frame.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Exit = event {
                save_main_window_frame(app);
            }
            // Review round 2, P2-1: opening the app again (Finder, Spotlight, the Dock) brings the window back.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                reopen_main_window(app);
            }
            #[cfg(not(target_os = "macos"))]
            let _ = app;
        });
}

// ── Update loop ───────────────────────────────────────────────────────────────
