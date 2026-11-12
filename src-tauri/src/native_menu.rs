//! `native_menu` items split out of `lib.rs`.

use super::*;

/// Background task: wait 5 s on launch, then check every 4 hours.
pub(super) async fn run_update_loop(app: tauri::AppHandle) {
    // Give the app a few seconds to finish startup before hitting the network.
    tokio::time::sleep(std::time::Duration::from_secs(UPDATE_CHECK_STARTUP_DELAY_SECS)).await;

    loop {
        check_for_update(&app).await;
        tokio::time::sleep(std::time::Duration::from_secs(UPDATE_CHECK_INTERVAL_HOURS * 3600)).await;
    }
}

pub(super) async fn check_for_update(app: &tauri::AppHandle) {
    let channel = configured_release_channel();
    let updater = match updater_for_release_channel(app, channel) {
        Ok(u) => u,
        Err(e) => {
            tracing::debug!(channel = %channel.as_str(), "updater not available: {e}");
            return;
        }
    };

    match updater.check().await {
        Ok(Some(update)) => {
            tracing::info!(
                version = %update.version,
                channel = %channel.as_str(),
                "update available — notifying frontend"
            );
            let version = update.version.to_string();
            popover_data::note_update(channel.as_str(), Some(&version));
            // Emit to the webview so the frontend can show a banner.
            // Frontend listens with: import { listen } from "@tauri-apps/api/event"
            //   await listen("update-available", handler)
            let _ = app.emit(
                "update-available",
                serde_json::json!({
                    "version": version,
                    "channel": channel.as_str(),
                    "body": update.body.as_deref().unwrap_or(""),
                    "release_notes_url": release_notes_url_for_version(&version),
                }),
            );
        }
        Ok(None) => {
            popover_data::note_update(channel.as_str(), None);
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

pub(super) const MENU_PREFERENCES_ID: &str = "menu_preferences";
pub(super) const MENU_CHECK_UPDATES_ID: &str = "menu_check_updates";
pub(super) const MENU_TOGGLE_SYNC_ID: &str = "menu_toggle_sync";
pub(super) const MENU_SIGN_OUT_ID: &str = "menu_sign_out";
pub(super) const MENU_QUIT_ID: &str = "menu_quit";
pub(super) const MENU_OPEN_FOLDER_ID: &str = "menu_open_folder";
pub(super) const MENU_UPLOAD_FILES_ID: &str = "menu_upload_files";
pub(super) const MENU_NEW_FOLDER_ID: &str = "menu_new_folder";
pub(super) const MENU_OPEN_WEB_APP_ID: &str = "menu_open_web_app";
pub(super) const MENU_SETTINGS_ID: &str = "menu_settings";
pub(super) const MENU_VIEW_FILES_ID: &str = "menu_view_files";
pub(super) const MENU_VIEW_ACTIVITY_ID: &str = "menu_view_activity";
#[cfg(test)]
pub(super) const MENU_VIEW_SHARED_ID: &str = "menu_view_shared";
pub(super) const MENU_VIEW_TRASH_ID: &str = "menu_view_trash";
// The Beebeeb window's View items on a Mac (task 1683 slice 1, two-surface F11 and F15).
pub(super) const MENU_VIEW_HOME_ID: &str = "menu_view_home";
pub(super) const MENU_VIEW_BACK_ID: &str = "menu_view_back";
pub(super) const MENU_ZOOM_IN_ID: &str = "menu_zoom_in";
pub(super) const MENU_ZOOM_OUT_ID: &str = "menu_zoom_out";
pub(super) const MENU_ZOOM_RESET_ID: &str = "menu_zoom_reset";
pub(super) const MENU_HELP_DOCS_ID: &str = "menu_help_docs";
pub(super) const MENU_HELP_SHORTCUTS_ID: &str = "menu_help_shortcuts";
pub(super) const MENU_HELP_REPORT_ID: &str = "menu_help_report";
pub(super) const MENU_HELP_STATUS_ID: &str = "menu_help_status";

pub(super) const WEB_APP_URL: &str = "https://app.beebeeb.io";
// Pre-mortem 10 task 8: the old documentation subdomain has never had a DNS
// record. The plan's own literal replacement for KEYBOARD_SHORTCUTS_URL
// (beebeeb.io/faq) is ALSO a dead link -- verified via `curl -I` on the live
// marketing site (2026-09-22): /docs -> 200, /support -> 200, /faq -> 404.
// No dedicated keyboard-shortcuts or FAQ route exists (grepped
// repos/site/src for "faq" -- the only FAQ content lives embedded in
// support.astro's `allFaqs`), so KEYBOARD_SHORTCUTS_URL points at /support
// (the closest real "get help" surface) instead. DOCUMENTATION_URL points at
// /docs, a real, populated docs index (repos/site/src/pages/docs.astro),
// which is also a better topical match than the plan's own /support
// suggestion for a "Documentation" menu item.
pub(super) const DOCUMENTATION_URL: &str = "https://beebeeb.io/docs";
pub(super) const KEYBOARD_SHORTCUTS_URL: &str = "https://beebeeb.io/support";
pub(super) const SERVICE_STATUS_URL: &str = "https://status.beebeeb.io";
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
// The desktop app has no real Shared view yet (task 1186/1243), so any legacy
// Shared navigation falls through to Files instead of opening a fake placeholder.
pub(super) const WINDOWS_MAIN_APP_VIEW_ROUTES: &[&str] = &["files", "activity", "trash"];
pub(super) const MENU_ZOOM_MIN: f64 = 0.5;
pub(super) const MENU_ZOOM_MAX: f64 = 2.0;
pub(super) const MENU_ZOOM_STEP: f64 = 0.1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DesktopMenuView {
    Files,
    Activity,
    #[cfg_attr(not(test), allow(dead_code))]
    Shared,
    Trash,
}

impl DesktopMenuView {
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub(super) fn route(self) -> &'static str {
        match self {
            Self::Files => "files",
            Self::Activity => "activity",
            Self::Shared => "shared",
            Self::Trash => "trash",
        }
    }
}

/// Where a View item takes the Beebeeb window (macOS only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MainMenuNav {
    Home,
    Back,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MenuZoomAction {
    In,
    Out,
    Reset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DesktopMenuAction {
    OpenSettings,
    CheckForUpdates,
    ToggleSync,
    SignOut,
    Quit,
    OpenFolder,
    UploadFiles,
    NewFolder,
    OpenWebApp,
    OpenView(DesktopMenuView),
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    NavigateMain(MainMenuNav),
    Zoom(MenuZoomAction),
    OpenExternal(&'static str),
    ReportProblem,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct DesktopMenuSpec {
    pub(super) id: &'static str,
    pub(super) label: &'static str,
    pub(super) accelerator: Option<&'static str>,
    pub(super) action: DesktopMenuAction,
}

pub(super) const DESKTOP_MENU_SPECS: &[DesktopMenuSpec] = &[
    DesktopMenuSpec {
        id: MENU_PREFERENCES_ID,
        label: "Preferences...",
        accelerator: Some("CmdOrCtrl+Comma"),
        action: DesktopMenuAction::OpenSettings,
    },
    DesktopMenuSpec {
        id: MENU_CHECK_UPDATES_ID,
        label: "Check for updates...",
        accelerator: Some("CmdOrCtrl+Shift+U"),
        action: DesktopMenuAction::CheckForUpdates,
    },
    DesktopMenuSpec {
        id: MENU_TOGGLE_SYNC_ID,
        label: "Pause/Resume syncing",
        accelerator: Some("CmdOrCtrl+Shift+P"),
        action: DesktopMenuAction::ToggleSync,
    },
    DesktopMenuSpec {
        id: MENU_SIGN_OUT_ID,
        label: "Sign out",
        accelerator: Some("CmdOrCtrl+Shift+L"),
        action: DesktopMenuAction::SignOut,
    },
    DesktopMenuSpec {
        id: MENU_QUIT_ID,
        label: "Quit Beebeeb",
        accelerator: Some("CmdOrCtrl+Q"),
        action: DesktopMenuAction::Quit,
    },
    DesktopMenuSpec {
        id: MENU_OPEN_FOLDER_ID,
        label: "Open Beebeeb folder",
        accelerator: Some("CmdOrCtrl+O"),
        action: DesktopMenuAction::OpenFolder,
    },
    DesktopMenuSpec {
        id: MENU_UPLOAD_FILES_ID,
        label: "Upload files...",
        accelerator: Some("CmdOrCtrl+U"),
        action: DesktopMenuAction::UploadFiles,
    },
    DesktopMenuSpec {
        id: MENU_NEW_FOLDER_ID,
        label: "New folder...",
        accelerator: Some("CmdOrCtrl+Shift+N"),
        action: DesktopMenuAction::NewFolder,
    },
    DesktopMenuSpec {
        id: MENU_OPEN_WEB_APP_ID,
        label: "Open web app",
        accelerator: Some("CmdOrCtrl+Shift+O"),
        action: DesktopMenuAction::OpenWebApp,
    },
    DesktopMenuSpec {
        id: MENU_SETTINGS_ID,
        label: "Settings...",
        accelerator: Some("CmdOrCtrl+Comma"),
        action: DesktopMenuAction::OpenSettings,
    },
    DesktopMenuSpec {
        id: MENU_VIEW_FILES_ID,
        label: "Files",
        accelerator: Some("CmdOrCtrl+Digit1"),
        action: DesktopMenuAction::OpenView(DesktopMenuView::Files),
    },
    DesktopMenuSpec {
        id: MENU_VIEW_ACTIVITY_ID,
        label: "Activity",
        accelerator: Some("CmdOrCtrl+Digit2"),
        action: DesktopMenuAction::OpenView(DesktopMenuView::Activity),
    },
    DesktopMenuSpec {
        id: MENU_VIEW_TRASH_ID,
        label: "Trash",
        accelerator: Some("CmdOrCtrl+Digit4"),
        action: DesktopMenuAction::OpenView(DesktopMenuView::Trash),
    },
    DesktopMenuSpec {
        id: MENU_ZOOM_IN_ID,
        label: "Zoom in",
        accelerator: Some("CmdOrCtrl+Equal"),
        action: DesktopMenuAction::Zoom(MenuZoomAction::In),
    },
    DesktopMenuSpec {
        id: MENU_ZOOM_OUT_ID,
        label: "Zoom out",
        accelerator: Some("CmdOrCtrl+Minus"),
        action: DesktopMenuAction::Zoom(MenuZoomAction::Out),
    },
    DesktopMenuSpec {
        id: MENU_ZOOM_RESET_ID,
        label: "Actual size",
        accelerator: Some("CmdOrCtrl+Digit0"),
        action: DesktopMenuAction::Zoom(MenuZoomAction::Reset),
    },
    DesktopMenuSpec {
        id: MENU_HELP_DOCS_ID,
        label: "Documentation",
        accelerator: None,
        action: DesktopMenuAction::OpenExternal(DOCUMENTATION_URL),
    },
    DesktopMenuSpec {
        id: MENU_HELP_SHORTCUTS_ID,
        label: "Keyboard shortcuts",
        accelerator: Some("CmdOrCtrl+Slash"),
        action: DesktopMenuAction::OpenExternal(KEYBOARD_SHORTCUTS_URL),
    },
    DesktopMenuSpec {
        id: MENU_HELP_REPORT_ID,
        label: "Report a problem",
        accelerator: None,
        action: DesktopMenuAction::ReportProblem,
    },
    DesktopMenuSpec {
        id: MENU_HELP_STATUS_ID,
        label: "Service status",
        accelerator: None,
        action: DesktopMenuAction::OpenExternal(SERVICE_STATUS_URL),
    },
];

/// The macOS menu (task 1683 slice 1; two-surface spec §5.F rows F1-F15, §9.2): macOS title case and the ellipsis
/// character, one ⌘, (F10's duplicate File › Settings is gone), the View items of the Beebeeb window. Review (⌘2) and
/// Trash (⌘3) join with their panes in slice 4. Windows and Linux keep `DESKTOP_MENU_SPECS`, unchanged.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(super) const MACOS_MENU_SPECS: &[DesktopMenuSpec] = &[
    DesktopMenuSpec {
        id: MENU_PREFERENCES_ID,
        label: "Settings…",
        accelerator: Some("CmdOrCtrl+Comma"),
        action: DesktopMenuAction::OpenSettings,
    },
    DesktopMenuSpec {
        id: MENU_CHECK_UPDATES_ID,
        label: "Check for Updates…",
        accelerator: Some("CmdOrCtrl+Shift+U"),
        action: DesktopMenuAction::CheckForUpdates,
    },
    DesktopMenuSpec {
        id: MENU_TOGGLE_SYNC_ID,
        // F3: the label follows the state (`refresh_macos_pause_menu_label`); this is the unpaused one.
        label: MENU_PAUSE_SYNC_LABEL,
        accelerator: Some("CmdOrCtrl+Shift+P"),
        action: DesktopMenuAction::ToggleSync,
    },
    DesktopMenuSpec {
        id: MENU_SIGN_OUT_ID,
        // F4, slice 1: behaviour unchanged (slice 1b routes it through the confirm sheet once task 2175 passes).
        label: "Sign Out…",
        accelerator: Some("CmdOrCtrl+Shift+L"),
        action: DesktopMenuAction::SignOut,
    },
    DesktopMenuSpec {
        id: MENU_QUIT_ID,
        label: "Quit Beebeeb",
        accelerator: Some("CmdOrCtrl+Q"),
        action: DesktopMenuAction::Quit,
    },
    DesktopMenuSpec {
        id: MENU_OPEN_FOLDER_ID,
        label: "Open in Finder",
        accelerator: Some("CmdOrCtrl+O"),
        action: DesktopMenuAction::OpenFolder,
    },
    DesktopMenuSpec {
        id: MENU_UPLOAD_FILES_ID,
        label: "Upload Files…",
        accelerator: Some("CmdOrCtrl+U"),
        action: DesktopMenuAction::UploadFiles,
    },
    DesktopMenuSpec {
        id: MENU_NEW_FOLDER_ID,
        label: "New Folder…",
        accelerator: Some("CmdOrCtrl+Shift+N"),
        action: DesktopMenuAction::NewFolder,
    },
    DesktopMenuSpec {
        id: MENU_OPEN_WEB_APP_ID,
        label: "Open Web App",
        accelerator: Some("CmdOrCtrl+Shift+O"),
        action: DesktopMenuAction::OpenWebApp,
    },
    DesktopMenuSpec {
        id: MENU_VIEW_HOME_ID,
        label: "Home",
        accelerator: Some("CmdOrCtrl+Digit1"),
        action: DesktopMenuAction::NavigateMain(MainMenuNav::Home),
    },
    DesktopMenuSpec {
        id: MENU_VIEW_BACK_ID,
        label: "Back",
        accelerator: Some("CmdOrCtrl+BracketLeft"),
        action: DesktopMenuAction::NavigateMain(MainMenuNav::Back),
    },
    // F12 and F13: the zoom and Help items are unchanged, words included.
    DesktopMenuSpec {
        id: MENU_ZOOM_IN_ID,
        label: "Zoom in",
        accelerator: Some("CmdOrCtrl+Equal"),
        action: DesktopMenuAction::Zoom(MenuZoomAction::In),
    },
    DesktopMenuSpec {
        id: MENU_ZOOM_OUT_ID,
        label: "Zoom out",
        accelerator: Some("CmdOrCtrl+Minus"),
        action: DesktopMenuAction::Zoom(MenuZoomAction::Out),
    },
    DesktopMenuSpec {
        id: MENU_ZOOM_RESET_ID,
        label: "Actual size",
        accelerator: Some("CmdOrCtrl+Digit0"),
        action: DesktopMenuAction::Zoom(MenuZoomAction::Reset),
    },
    DesktopMenuSpec {
        id: MENU_HELP_DOCS_ID,
        label: "Documentation",
        accelerator: None,
        action: DesktopMenuAction::OpenExternal(DOCUMENTATION_URL),
    },
    DesktopMenuSpec {
        id: MENU_HELP_SHORTCUTS_ID,
        label: "Keyboard shortcuts",
        accelerator: Some("CmdOrCtrl+Slash"),
        action: DesktopMenuAction::OpenExternal(KEYBOARD_SHORTCUTS_URL),
    },
    DesktopMenuSpec {
        id: MENU_HELP_REPORT_ID,
        label: "Report a problem",
        accelerator: None,
        action: DesktopMenuAction::ReportProblem,
    },
    DesktopMenuSpec {
        id: MENU_HELP_STATUS_ID,
        label: "Service status",
        accelerator: None,
        action: DesktopMenuAction::OpenExternal(SERVICE_STATUS_URL),
    },
];

/// F3's two labels.
pub(super) const MENU_PAUSE_SYNC_LABEL: &str = "Pause Sync";
pub(super) const MENU_RESUME_SYNC_LABEL: &str = "Resume Sync";

#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(super) fn pause_menu_label(paused: bool) -> &'static str {
    if paused {
        MENU_RESUME_SYNC_LABEL
    } else {
        MENU_PAUSE_SYNC_LABEL
    }
}

/// F3: the label for what the file says now (an unreadable file reads as not paused, as the menu toggle does).
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(super) fn pause_menu_label_now() -> &'static str {
    pause_menu_label(DesktopConfig::load().map(|cfg| cfg.pause_sync).unwrap_or(false))
}

/// Every pause change calls this once it is written, through `set_sync_paused` (the menu, the tray commands, Home's
/// Resume, and a settings write from the compact flyout's Network page): the menu's label follows the file (F3).
pub(super) fn pause_state_changed(app: &tauri::AppHandle<impl tauri::Runtime>) {
    #[cfg(target_os = "macos")]
    refresh_macos_pause_menu_label(app);
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

/// One entry of the macOS menu bar: an item from `MACOS_MENU_SPECS` by id, or one AppKit draws itself.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MacosMenuEntry {
    Item(&'static str),
    About,
    Hide,
    HideOthers,
    ShowAll,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Minimize,
    Zoom,
    Close,
    BringAllToFront,
    Separator,
}

/// The macOS menu bar, top to bottom per menu (§9.2: "About Beebeeb · Settings… · Check for Updates… · Pause Sync ·
/// — · Sign Out… · — · Hide Beebeeb · Hide Others · Show All · — · Quit Beebeeb"; F15's Window menu and View › Back).
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(super) const MACOS_MENU_LAYOUT: &[(&str, &[MacosMenuEntry])] = {
    use MacosMenuEntry::*;
    &[
        (
            "Beebeeb",
            &[
                About,
                Item(MENU_PREFERENCES_ID),
                Item(MENU_CHECK_UPDATES_ID),
                Item(MENU_TOGGLE_SYNC_ID),
                Separator,
                Item(MENU_SIGN_OUT_ID),
                Separator,
                Hide,
                HideOthers,
                ShowAll,
                Separator,
                Item(MENU_QUIT_ID),
            ],
        ),
        (
            "File",
            &[
                Item(MENU_OPEN_FOLDER_ID),
                Item(MENU_UPLOAD_FILES_ID),
                Item(MENU_NEW_FOLDER_ID),
                Separator,
                Item(MENU_OPEN_WEB_APP_ID),
            ],
        ),
        ("Edit", &[Cut, Copy, Paste, SelectAll]),
        (
            "View",
            &[
                Item(MENU_VIEW_HOME_ID),
                Separator,
                Item(MENU_VIEW_BACK_ID),
                Separator,
                Item(MENU_ZOOM_IN_ID),
                Item(MENU_ZOOM_OUT_ID),
                Item(MENU_ZOOM_RESET_ID),
            ],
        ),
        ("Window", &[Minimize, Zoom, Close, Separator, BringAllToFront]),
        (
            "Help",
            &[
                Item(MENU_HELP_DOCS_ID),
                Item(MENU_HELP_SHORTCUTS_ID),
                Item(MENU_HELP_REPORT_ID),
                Separator,
                Item(MENU_HELP_STATUS_ID),
            ],
        ),
    ]
};

#[cfg(test)]
pub(super) fn desktop_menu_specs() -> &'static [DesktopMenuSpec] {
    DESKTOP_MENU_SPECS
}

/// The menu table this build installs: a Mac's own, else the shared one (Windows, Linux).
pub(super) fn installed_menu_specs() -> &'static [DesktopMenuSpec] {
    if cfg!(target_os = "macos") {
        MACOS_MENU_SPECS
    } else {
        DESKTOP_MENU_SPECS
    }
}

pub(super) fn desktop_menu_spec(id: &str) -> Option<&'static DesktopMenuSpec> {
    installed_menu_specs().iter().find(|spec| spec.id == id)
}

pub(super) fn desktop_menu_spec_required(id: &str) -> &'static DesktopMenuSpec {
    desktop_menu_spec(id).unwrap_or_else(|| panic!("missing desktop menu spec: {id}"))
}

/// The desktop a native menu is built for. Accelerators differ only where that platform's web view takes a key first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MenuPlatform {
    MacOs,
    Windows,
    Linux,
}

impl MenuPlatform {
    pub(super) fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Linux
        }
    }
}

/// The accelerator `spec` gets on `platform`.
///
/// Pause / Resume keeps Cmd+Shift+P on macOS only. On Windows, WebView2 acts on Ctrl+Shift+P first and prints the page
/// (0.8.14, Guus's PC); a Ctrl+Alt chord would be AltGr on European keyboards. So Windows and Linux get no shortcut for it:
/// the menu item, the tray menu and Settings › Sync still pause and resume.
pub(super) fn menu_accelerator(spec: &DesktopMenuSpec, platform: MenuPlatform) -> Option<&'static str> {
    match (spec.id, platform) {
        (MENU_TOGGLE_SYNC_ID, MenuPlatform::Windows | MenuPlatform::Linux) => None,
        _ => spec.accelerator,
    }
}

pub(super) fn build_desktop_menu_item<M: tauri::Manager<tauri::Wry>>(
    manager: &M,
    spec: &DesktopMenuSpec,
) -> tauri::Result<MenuItem<tauri::Wry>> {
    let mut builder = MenuItemBuilder::new(spec.label).id(spec.id);
    if let Some(accelerator) = menu_accelerator(spec, MenuPlatform::current()) {
        builder = builder.accelerator(accelerator);
    }
    builder.build(manager)
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(super) fn menu_view_nav_target(view: DesktopMenuView, supported_routes: &[&str]) -> &'static str {
    let route = view.route();
    if supported_routes.contains(&route) {
        route
    } else {
        "files"
    }
}

pub(super) fn compact_menu_page_for_view(view: DesktopMenuView) -> &'static str {
    match view {
        DesktopMenuView::Files => "finder",
        DesktopMenuView::Activity => "status",
        DesktopMenuView::Shared => "finder",
        // The compact macOS/Linux shell has no Trash page yet; send it to the
        // file surface instead of leaving a dead native item.
        DesktopMenuView::Trash => "finder",
    }
}

pub(super) fn next_menu_zoom_scale(current: f64, action: MenuZoomAction) -> f64 {
    let next = match action {
        MenuZoomAction::In => current + MENU_ZOOM_STEP,
        MenuZoomAction::Out => current - MENU_ZOOM_STEP,
        MenuZoomAction::Reset => 1.0,
    };
    ((next.clamp(MENU_ZOOM_MIN, MENU_ZOOM_MAX) * 10.0).round()) / 10.0
}

pub(super) fn about_metadata() -> AboutMetadata<'static> {
    let channel = installed_release_channel();
    AboutMetadata {
        name: Some("Beebeeb".to_string()),
        version: Some(real_app_version().to_string()),
        comments: Some(format!("Current channel: {}", channel.as_str())),
        website: Some("https://beebeeb.io".to_string()),
        website_label: Some("beebeeb.io".to_string()),
        ..AboutMetadata::default()
    }
}

/// The menu items the app changes after it built the menu (macOS): Settings…, Check for Updates… and the View items
/// follow the Beebeeb window (§8.2, F15), Pause Sync follows the pause (F3).
#[cfg(target_os = "macos")]
pub(super) struct MacosMenuItems {
    pub(super) settings: MenuItem<tauri::Wry>,
    pub(super) check_updates: MenuItem<tauri::Wry>,
    pub(super) toggle_sync: MenuItem<tauri::Wry>,
    pub(super) view_home: MenuItem<tauri::Wry>,
    pub(super) view_back: MenuItem<tauri::Wry>,
}

#[cfg(target_os = "macos")]
impl MacosMenuItems {
    /// The one built item for `id` when it is one of these.
    pub(super) fn get(&self, id: &str) -> Option<&MenuItem<tauri::Wry>> {
        match id {
            MENU_PREFERENCES_ID => Some(&self.settings),
            MENU_CHECK_UPDATES_ID => Some(&self.check_updates),
            MENU_TOGGLE_SYNC_ID => Some(&self.toggle_sync),
            MENU_VIEW_HOME_ID => Some(&self.view_home),
            MENU_VIEW_BACK_ID => Some(&self.view_back),
            _ => None,
        }
    }
}

/// F3: the menu says "Resume Sync" while the file says paused, else "Pause Sync". Called through
/// `pause_state_changed` only.
#[cfg(target_os = "macos")]
pub(super) fn refresh_macos_pause_menu_label(app: &tauri::AppHandle<impl tauri::Runtime>) {
    let Some(items) = app.try_state::<MacosMenuItems>() else {
        return;
    };
    let _ = items.toggle_sync.set_text(pause_menu_label_now());
}

/// The macOS menu bar from `MACOS_MENU_LAYOUT` (task 1683 slice 1). The items the app changes later are built from
/// `MACOS_MENU_SPECS` first and placed where the layout names them.
#[cfg(target_os = "macos")]
pub(super) fn setup_native_menu(app: &mut tauri::App) -> tauri::Result<()> {
    use tauri::menu::IsMenuItem;
    let kept = MacosMenuItems {
        settings: build_desktop_menu_item(app, desktop_menu_spec_required(MENU_PREFERENCES_ID))?,
        check_updates: build_desktop_menu_item(app, desktop_menu_spec_required(MENU_CHECK_UPDATES_ID))?,
        toggle_sync: build_desktop_menu_item(app, desktop_menu_spec_required(MENU_TOGGLE_SYNC_ID))?,
        view_home: build_desktop_menu_item(app, desktop_menu_spec_required(MENU_VIEW_HOME_ID))?,
        view_back: build_desktop_menu_item(app, desktop_menu_spec_required(MENU_VIEW_BACK_ID))?,
    };
    let mut menus: Vec<Submenu<tauri::Wry>> = Vec::new();
    for (title, entries) in MACOS_MENU_LAYOUT {
        let mut items: Vec<Box<dyn IsMenuItem<tauri::Wry>>> = Vec::new();
        for entry in entries.iter() {
            let item: Box<dyn IsMenuItem<tauri::Wry>> = match *entry {
                MacosMenuEntry::Item(id) => match kept.get(id) {
                    Some(item) => Box::new(item.clone()),
                    None => Box::new(build_desktop_menu_item(app, desktop_menu_spec_required(id))?),
                },
                MacosMenuEntry::About => Box::new(PredefinedMenuItem::about(
                    app,
                    Some("About Beebeeb"),
                    Some(about_metadata()),
                )?),
                MacosMenuEntry::Hide => Box::new(PredefinedMenuItem::hide(app, Some("Hide Beebeeb"))?),
                MacosMenuEntry::HideOthers => Box::new(PredefinedMenuItem::hide_others(app, Some("Hide Others"))?),
                MacosMenuEntry::ShowAll => Box::new(PredefinedMenuItem::show_all(app, Some("Show All"))?),
                MacosMenuEntry::Cut => Box::new(PredefinedMenuItem::cut(app, None)?),
                MacosMenuEntry::Copy => Box::new(PredefinedMenuItem::copy(app, None)?),
                MacosMenuEntry::Paste => Box::new(PredefinedMenuItem::paste(app, None)?),
                MacosMenuEntry::SelectAll => Box::new(PredefinedMenuItem::select_all(app, None)?),
                MacosMenuEntry::Minimize => Box::new(PredefinedMenuItem::minimize(app, Some("Minimize"))?),
                MacosMenuEntry::Zoom => Box::new(PredefinedMenuItem::maximize(app, Some("Zoom"))?),
                MacosMenuEntry::Close => Box::new(PredefinedMenuItem::close_window(app, Some("Close"))?),
                MacosMenuEntry::BringAllToFront => {
                    Box::new(PredefinedMenuItem::bring_all_to_front(app, Some("Bring All to Front"))?)
                }
                MacosMenuEntry::Separator => Box::new(PredefinedMenuItem::separator(app)?),
            };
            items.push(item);
        }
        let refs: Vec<&dyn IsMenuItem<tauri::Wry>> = items.iter().map(|item| item.as_ref()).collect();
        let submenu = Submenu::with_items(app, *title, true, &refs)?;
        if *title == "Window" {
            let _ = submenu.set_as_windows_menu_for_nsapp();
        } else if *title == "Help" {
            let _ = submenu.set_as_help_menu_for_nsapp();
        }
        menus.push(submenu);
    }
    let refs: Vec<&dyn IsMenuItem<tauri::Wry>> = menus.iter().map(|menu| menu as &dyn IsMenuItem<tauri::Wry>).collect();
    let menu = Menu::with_items(app, &refs)?;
    app.set_menu(menu)?;
    app.manage(kept);
    // No Beebeeb window yet: Back off, the rest on; and the pause label as the file says.
    apply_macos_main_menu_state(app.handle(), main_menu_state(false, false, false));
    pause_state_changed(app.handle());
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub(super) fn setup_native_menu(app: &mut tauri::App) -> tauri::Result<()> {
    let preferences = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_PREFERENCES_ID))?;
    let check_updates = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_CHECK_UPDATES_ID))?;
    let toggle_sync = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_TOGGLE_SYNC_ID))?;
    let sign_out = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_SIGN_OUT_ID))?;
    let quit = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_QUIT_ID))?;
    let open_folder = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_OPEN_FOLDER_ID))?;
    let upload_files = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_UPLOAD_FILES_ID))?;
    let new_folder = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_NEW_FOLDER_ID))?;
    let open_web_app = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_OPEN_WEB_APP_ID))?;
    let settings = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_SETTINGS_ID))?;
    let view_files = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_VIEW_FILES_ID))?;
    let view_activity = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_VIEW_ACTIVITY_ID))?;
    let view_trash = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_VIEW_TRASH_ID))?;
    let zoom_in = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_ZOOM_IN_ID))?;
    let zoom_out = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_ZOOM_OUT_ID))?;
    let zoom_reset = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_ZOOM_RESET_ID))?;
    let documentation = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_HELP_DOCS_ID))?;
    let keyboard_shortcuts = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_HELP_SHORTCUTS_ID))?;
    let report_problem = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_HELP_REPORT_ID))?;
    let service_status = build_desktop_menu_item(app, desktop_menu_spec_required(MENU_HELP_STATUS_ID))?;

    let beebeeb_menu = Submenu::with_items(
        app,
        "Beebeeb",
        true,
        &[
            &PredefinedMenuItem::about(app, Some("About Beebeeb"), Some(about_metadata()))?,
            &PredefinedMenuItem::separator(app)?,
            &preferences,
            &check_updates,
            &toggle_sync,
            &PredefinedMenuItem::separator(app)?,
            &sign_out,
            &quit,
        ],
    )?;

    let file_menu = Submenu::with_items(
        app,
        "File",
        true,
        &[
            &open_folder,
            &upload_files,
            &new_folder,
            &PredefinedMenuItem::separator(app)?,
            &open_web_app,
            &settings,
        ],
    )?;

    let edit_menu = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;

    let view_menu = Submenu::with_items(
        app,
        "View",
        true,
        &[
            &view_files,
            &view_activity,
            &view_trash,
            &PredefinedMenuItem::separator(app)?,
            &zoom_in,
            &zoom_out,
            &zoom_reset,
        ],
    )?;

    let help_menu = Submenu::with_items(
        app,
        "Help",
        true,
        &[
            &documentation,
            &keyboard_shortcuts,
            &report_problem,
            &PredefinedMenuItem::separator(app)?,
            &service_status,
        ],
    )?;

    let menu = Menu::with_items(app, &[&beebeeb_menu, &file_menu, &edit_menu, &view_menu, &help_menu])?;
    app.set_menu(menu)?;
    Ok(())
}

// A native click can create the app webview before React has subscribed. Keep
// one pending request until that webview consumes it after registering its listener.
pub(super) static MENU_UPDATE_CHECK_PENDING: AtomicBool = AtomicBool::new(false);

pub(super) fn update_check_window_label() -> &'static str {
    if cfg!(target_os = "windows") {
        "main-app"
    } else if cfg!(target_os = "macos") {
        // Task 1683 slice 1: the Beebeeb window drains the pending menu check in Settings › About's update row.
        "main"
    } else {
        "settings"
    }
}

pub(super) fn request_menu_update_check(app: &tauri::AppHandle) {
    MENU_UPDATE_CHECK_PENDING.store(true, Ordering::SeqCst);
    // F2: the Beebeeb window on Settings › About, which checks and answers in its row.
    #[cfg(target_os = "macos")]
    show_main_window(app, MainNav::settings(Some("about")));
    // Desktop v2 (M3e): Settings › About, where the check answers in its row, as on a Mac.
    #[cfg(not(target_os = "macos"))]
    show_main_app_window_with_nav(app, Some("settings/about"));
    if let Some(window) = app.get_webview_window(update_check_window_label()) {
        log_menu_result(
            MENU_CHECK_UPDATES_ID,
            window.emit("menu:check-for-updates", ()).map_err(|e| e.to_string()),
        );
    }
}

#[tauri::command]
pub(super) fn consume_menu_update_check(window: tauri::WebviewWindow) -> bool {
    take_menu_update_check(&MENU_UPDATE_CHECK_PENDING, window.label())
}

pub(super) fn take_menu_update_check(pending: &AtomicBool, label: &str) -> bool {
    label == update_check_window_label() && pending.swap(false, Ordering::SeqCst)
}
