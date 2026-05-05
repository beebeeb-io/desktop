use tauri::{
    menu::{AboutMetadata, Menu, MenuItemBuilder, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager,
};
use tracing_subscriber::EnvFilter;

/// Returns the app version string — exposed to the webview as an IPC command
/// so the web UI can display "Desktop 0.1.0" in its Settings page.
#[tauri::command]
fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .try_init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // Persist window position + size across restarts
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .invoke_handler(tauri::generate_handler![app_version])
        .setup(|app| {
            setup_native_menu(app)?;
            setup_tray(app)?;
            Ok(())
        })
        // Cmd+W / "Close Window" menu item → hide to tray
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

// ── Native menu bar ──────────────────────────────────────────────────────────

fn setup_native_menu(app: &mut tauri::App) -> tauri::Result<()> {
    // Beebeeb  (app-name menu — macOS leftmost)
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
            // live in the menu bar / tray — it doesn't quit).
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

fn setup_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let show_item = MenuItemBuilder::new("Show Beebeeb")
        .id("tray_show")
        .build(app)?;
    let hide_item = MenuItemBuilder::new("Hide")
        .id("tray_hide")
        .build(app)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit_item = PredefinedMenuItem::quit(app, Some("Quit"))?;

    let tray_menu = Menu::with_items(app, &[&show_item, &hide_item, &sep, &quit_item])?;

    let icon = app
        .default_window_icon()
        .cloned()
        .expect("no app icon found — check icons/icon.png exists");

    let _tray = TrayIconBuilder::with_id("tray")
        .icon(icon)
        .icon_as_template(true) // macOS template image (monochrome, respects dark/light)
        .menu(&tray_menu)
        .show_menu_on_left_click(false) // left click toggles window; right click shows menu
        .tooltip("Beebeeb")
        .on_menu_event(|app, event| match event.id().as_ref() {
            "tray_show" => show_main_window(app),
            "tray_hide" => {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.hide();
                }
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

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Show and focus the main window, creating it if needed.
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.set_focus();
    }
}
