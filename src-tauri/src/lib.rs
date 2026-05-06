use std::path::PathBuf;
use std::sync::Mutex;

use tauri::{
    menu::{AboutMetadata, CheckMenuItemBuilder, Menu, MenuItemBuilder, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, State,
};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_autostart::ManagerExt as AutostartManagerExt;
use tauri_plugin_dialog::DialogExt;
use tracing_subscriber::EnvFilter;

mod config;
use config::DesktopConfig;

// ── Session bridge (web ↔ rust) ───────────────────────────────────────────────

/// Authenticated session pushed in from the WebView after login.
///
/// `master_key` is the user's 32-byte vault root key, derived from the
/// OPAQUE export_key (or, for legacy accounts, from Argon2id over the
/// password). It is the input to per-file `derive_file_key()` calls
/// inside `beebeeb-sync` — the engine cannot do anything without it.
///
/// The struct is intentionally NOT `Clone`: there should be exactly one
/// authoritative copy of the master key in this process at any moment.
/// `take()` removes it from the registry on logout; everything else
/// reads it through a borrow under the `Mutex`.
#[derive(Debug)]
pub struct Session {
    pub token: String,
    pub master_key: [u8; 32],
}

/// Tauri-managed shared state. Held behind a `Mutex` because the web
/// thread (IPC handlers) and the future sync-engine task both need
/// access. Cheap to lock — sessions change rarely.
#[derive(Default)]
pub struct AppState {
    pub session: Mutex<Option<Session>>,
}

// ── IPC commands: session ─────────────────────────────────────────────────────

/// Stash an authenticated session in app state. Called by the WebView
/// immediately after a successful OPAQUE (or legacy) login on the
/// frontend side. Idempotent — overwrites any previous session.
///
/// Returns `Err` if `master_key` is not exactly 32 bytes; the WebView
/// should treat that as a programming bug and surface it loudly.
#[tauri::command]
fn set_session(
    state: State<'_, AppState>,
    token: String,
    master_key: Vec<u8>,
) -> Result<(), String> {
    if master_key.len() != 32 {
        return Err(format!(
            "master_key must be 32 bytes, got {}",
            master_key.len()
        ));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&master_key);

    let mut guard = state
        .session
        .lock()
        .map_err(|_| "session mutex poisoned".to_string())?;
    *guard = Some(Session { token, master_key: arr });
    tracing::info!("session installed via IPC");
    Ok(())
}

/// Drop any cached session. Called by the WebView on logout. Best-effort
/// — a poisoned mutex is logged but not surfaced as an error to the JS
/// side because logout should always appear to succeed.
#[tauri::command]
fn clear_session(state: State<'_, AppState>) {
    match state.session.lock() {
        Ok(mut guard) => {
            guard.take();
            tracing::info!("session cleared via IPC");
        }
        Err(_) => {
            tracing::warn!("session mutex poisoned during clear_session");
        }
    }
}

/// Lightweight status endpoint for the WebView's settings page.
/// Reports whether the Rust side has a session, where the sync root
/// is on disk, and (eventually, once the engine ships) what the sync
/// engine is doing.
#[tauri::command]
fn sync_status(state: State<'_, AppState>) -> serde_json::Value {
    let logged_in = state
        .session
        .lock()
        .map(|g| g.is_some())
        .unwrap_or(false);
    let sync_root = DesktopConfig::load()
        .ok()
        .and_then(|c| c.sync_root)
        .map(|p| p.to_string_lossy().into_owned());
    serde_json::json!({
        "logged_in": logged_in,
        "sync_root": sync_root,
        // Placeholder until the engine is wired in (spec 030 step 4).
        "engine": "not_started",
    })
}

// ── IPC commands: sync-root config ────────────────────────────────────────────

/// Read the persisted sync root from `~/.config/beebeeb/desktop.toml`.
/// Returns `None` if the user hasn't picked one yet (first launch).
#[tauri::command]
fn get_sync_root() -> Result<Option<String>, String> {
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
/// Spec 030 §2.
#[tauri::command]
async fn pick_sync_root(app: tauri::AppHandle) -> Result<Option<String>, String> {
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

    if !path.is_absolute() {
        return Err(format!(
            "picker returned a non-absolute path: {} (please report)",
            path.display()
        ));
    }
    config::ensure_directory(&path)?;

    let mut cfg = DesktopConfig::load()?;
    cfg.sync_root = Some(path.clone());
    cfg.save()?;

    tracing::info!(path = %path.display(), "sync root set");
    Ok(Some(path.to_string_lossy().into_owned()))
}

/// Suggested default path for the sync root, used by the WebView to
/// show "We'll create ~/Beebeeb if you accept" before opening the
/// native picker.
#[tauri::command]
fn default_sync_root() -> String {
    config::default_sync_root_suggestion()
        .to_string_lossy()
        .into_owned()
}

// ── IPC commands: app metadata ────────────────────────────────────────────────

// ── Durations ─────────────────────────────────────────────────────────────────

/// Delay after launch before the first update check (don't block startup).
const UPDATE_CHECK_STARTUP_DELAY_SECS: u64 = 5;
/// How often to re-check for updates while the app is running.
const UPDATE_CHECK_INTERVAL_HOURS: u64 = 4;

/// Returns the app version string — displayed in the web UI Settings page.
#[tauri::command]
fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Download and install the pending update, then relaunch.
///
/// Called by the frontend when the user clicks "Restart to update" in the
/// update-available banner. Returns an error string if installation fails.
#[tauri::command]
async fn install_update(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;

    let updater = app.updater().map_err(|e| e.to_string())?;
    let update = updater.check().await.map_err(|e| e.to_string())?;

    match update {
        Some(u) => {
            tracing::info!(version = %u.version, "downloading update");

            u.download_and_install(
                |downloaded, total| {
                    // Emit progress so the frontend can show a progress bar.
                    // downloaded: usize, total: Option<u64>
                    let pct = total
                        .map(|t| if t > 0 { (downloaded as u64) * 100 / t } else { 0 })
                        .unwrap_or(0);
                    tracing::debug!("update download progress: {pct}%");
                },
                || {
                    tracing::info!("update installed — relaunching");
                },
            )
            .await
            .map_err(|e| e.to_string())?;

            app.restart();
        }
        None => {
            tracing::info!("install_update called but no update available");
        }
    }

    Ok(())
}

/// Toggle "Start at login" autostart setting.
#[tauri::command]
fn toggle_autostart(app: tauri::AppHandle) -> Result<bool, String> {
    let manager = app.autolaunch();
    let currently_enabled = manager.is_enabled().map_err(|e| e.to_string())?;
    if currently_enabled {
        manager.disable().map_err(|e| e.to_string())?;
        tracing::info!("autostart disabled");
    } else {
        manager.enable().map_err(|e| e.to_string())?;
        tracing::info!("autostart enabled");
    }
    Ok(!currently_enabled)
}

/// Return the current autostart state (for the frontend settings page).
#[tauri::command]
fn autostart_enabled(app: tauri::AppHandle) -> Result<bool, String> {
    app.autolaunch()
        .is_enabled()
        .map_err(|e| e.to_string())
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .try_init();

    tauri::Builder::default()
        // Shared state — session pushed in by the WebView via set_session
        // IPC after login. The future sync engine task reads from this.
        .manage(AppState::default())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        // Native folder picker for the first-launch sync-root chooser.
        .plugin(tauri_plugin_dialog::init())
        // Auto-updater — endpoints + pubkey configured in tauri.conf.json
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Autostart — LaunchAgent on macOS (no elevated privileges required),
        // registry key on Windows, .desktop file on Linux
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .invoke_handler(tauri::generate_handler![
            app_version,
            install_update,
            toggle_autostart,
            autostart_enabled,
            set_session,
            clear_session,
            sync_status,
            get_sync_root,
            pick_sync_root,
            default_sync_root,
        ])
        .setup(|app| {
            setup_native_menu(app)?;
            setup_tray(app)?;

            // Spawn background update checker
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                run_update_loop(handle).await;
            });

            Ok(())
        })
        // Cmd+W / "Close Window" → hide to tray
        .on_menu_event(|app, event| {
            if event.id().as_ref() == "close_window" {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.hide();
                }
            }
        })
        // Red-dot close button → hide to tray instead of quitting
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Beebeeb desktop");
}

// ── Update loop ───────────────────────────────────────────────────────────────

/// Background task: wait 5 s on launch, then check every 4 hours.
async fn run_update_loop(app: tauri::AppHandle) {
    // Give the app a few seconds to finish startup before hitting the network.
    tokio::time::sleep(std::time::Duration::from_secs(UPDATE_CHECK_STARTUP_DELAY_SECS)).await;

    loop {
        check_for_update(&app).await;
        tokio::time::sleep(std::time::Duration::from_secs(
            UPDATE_CHECK_INTERVAL_HOURS * 3600,
        ))
        .await;
    }
}

async fn check_for_update(app: &tauri::AppHandle) {
    use tauri_plugin_updater::UpdaterExt;

    let updater = match app.updater() {
        Ok(u) => u,
        Err(e) => {
            tracing::debug!("updater not available: {e}");
            return;
        }
    };

    match updater.check().await {
        Ok(Some(update)) => {
            tracing::info!(
                version = %update.version,
                "update available — notifying frontend"
            );
            // Emit to the webview so the frontend can show a banner.
            // Frontend listens with: import { listen } from "@tauri-apps/api/event"
            //   await listen("update-available", handler)
            let _ = app.emit(
                "update-available",
                serde_json::json!({
                    "version": update.version,
                    "body": update.body.as_deref().unwrap_or(""),
                }),
            );
        }
        Ok(None) => {
            tracing::debug!("no update available");
        }
        Err(e) => {
            // Network errors are expected when the release server isn't live
            // yet — log at debug to avoid noisy startup logs.
            tracing::debug!("update check failed: {e}");
        }
    }
}

// ── Native menu bar ──────────────────────────────────────────────────────────

fn setup_native_menu(app: &mut tauri::App) -> tauri::Result<()> {
    // Beebeeb (app-name menu — macOS leftmost)
    let beebeeb_menu = Submenu::with_items(
        app,
        "Beebeeb",
        true,
        &[
            &PredefinedMenuItem::about(
                app,
                Some("About Beebeeb"),
                Some(AboutMetadata::default()),
            )?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::show_all(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, Some("Quit Beebeeb"))?,
        ],
    )?;

    // File
    let file_menu = Submenu::with_items(
        app,
        "File",
        true,
        &[
            // Cmd+W hides the window (standard macOS behaviour for apps that
            // live in the menu bar — it doesn't quit the app).
            &MenuItemBuilder::new("Close Window")
                .id("close_window")
                .accelerator("CmdOrCtrl+W")
                .build(app)?,
        ],
    )?;

    // Edit — essential for WebView text-input fields to get clipboard bindings
    let edit_menu = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;

    let menu = Menu::with_items(app, &[&beebeeb_menu, &file_menu, &edit_menu])?;
    app.set_menu(menu)?;
    Ok(())
}

// ── System tray ──────────────────────────────────────────────────────────────

/// Build the tray context menu with the current autostart checked state.
/// Called on first setup and again whenever the autostart state changes.
fn build_tray_menu<M: tauri::Manager<tauri::Wry>>(
    manager: &M,
    autostart_enabled: bool,
) -> tauri::Result<Menu<tauri::Wry>> {
    let show_item = MenuItemBuilder::new("Show Beebeeb")
        .id("tray_show")
        .build(manager)?;
    let hide_item = MenuItemBuilder::new("Hide")
        .id("tray_hide")
        .build(manager)?;
    let autostart_item = CheckMenuItemBuilder::new("Start at login")
        .id("tray_autostart")
        .checked(autostart_enabled)
        .build(manager)?;
    let sep1 = PredefinedMenuItem::separator(manager)?;
    let sep2 = PredefinedMenuItem::separator(manager)?;
    let quit_item = PredefinedMenuItem::quit(manager, Some("Quit"))?;
    Menu::with_items(
        manager,
        &[&show_item, &hide_item, &sep1, &autostart_item, &sep2, &quit_item],
    )
}

fn setup_tray(app: &mut tauri::App) -> tauri::Result<()> {
    // Read current autostart state so the check mark is correct on launch.
    let autostart_enabled = app
        .autolaunch()
        .is_enabled()
        .unwrap_or(false);
    let tray_menu = build_tray_menu(app, autostart_enabled)?;

    let icon = app
        .default_window_icon()
        .cloned()
        .expect("no app icon found — check icons/icon.png exists");

    let _tray = TrayIconBuilder::with_id("tray")
        .icon(icon)
        .icon_as_template(true) // macOS: monochrome, respects dark/light mode
        .menu(&tray_menu)
        .show_menu_on_left_click(false)
        .tooltip("Beebeeb")
        .on_menu_event(|app, event| match event.id().as_ref() {
            "tray_show" => show_main_window(app),
            "tray_hide" => {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.hide();
                }
            }
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
                if let Some(tray) = app.tray_by_id("tray") {
                    if let Ok(menu) = build_tray_menu(app, new_state) {
                        let _ = tray.set_menu(Some(menu));
                    }
                }
                tracing::info!(enabled = new_state, "autostart toggled via tray");
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // Left-click toggles window visibility
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(win) = app.get_webview_window("main") {
                    if win.is_visible().unwrap_or(false) {
                        let _ = win.hide();
                    } else {
                        show_main_window(app);
                    }
                }
            }
        })
        .build(app)?;

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Show and focus the main window.
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.set_focus();
    }
}
