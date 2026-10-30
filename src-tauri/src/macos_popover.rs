//! Task 1683 slice 3: the status-item popover. No sync or credential policy lives here;
//! actions use the same host functions as the existing menus and main window.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct UpdateOffer {
    pub channel: String,
    pub version: String,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct Sending {
    pub changes_left: usize,
}

/// The count the popover shows while sign-out sends the last changes. A read that fails leaves the count out for that
/// tick (`progressLine` shows none for 0), never the whole snapshot of the popover and the Beebeeb window (review P3-5
/// of #160).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn changes_left(read: Result<crate::sign_out::waiting::WaitingChanges, String>) -> usize {
    match read {
        Ok(waiting) => {
            let counts = waiting.counts();
            counts.files.saturating_add(counts.others)
        }
        Err(_) => {
            tracing::warn!("could not count the changes sign-out is still sending");
            0
        }
    }
}
static UPDATE: Mutex<Option<(String, String)>> = Mutex::new(None);

pub(super) fn note_update(channel: &str, version: Option<&str>) {
    *UPDATE.lock().unwrap_or_else(|p| p.into_inner()) = version.map(|v| (channel.to_owned(), v.to_owned()));
}

pub(super) fn update_offer() -> Option<UpdateOffer> {
    let channel = configured_release_channel();
    UPDATE
        .lock()
        .ok()?
        .as_ref()
        .filter(|(c, _)| c == channel.as_str())
        .map(|(channel, version)| UpdateOffer {
            channel: channel.clone(),
            version: version.clone(),
        })
}

#[cfg(target_os = "macos")]
static BLUR: LazyLock<Mutex<surfaces::blur_guard::BlurClickGuard>> =
    LazyLock::new(|| Mutex::new(surfaces::blur_guard::BlurClickGuard::new()));
#[cfg(target_os = "macos")]
static CLOCK: LazyLock<std::time::Instant> = LazyLock::new(std::time::Instant::now);
#[cfg(target_os = "macos")]
static GEAR_MENU_OPEN: AtomicBool = AtomicBool::new(false);

fn require_popover(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() == "popover" {
        Ok(())
    } else {
        Err("This action belongs to the menu-bar popover.".into())
    }
}

#[cfg(target_os = "macos")]
fn clock_ms() -> u64 {
    CLOCK.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}

#[cfg(target_os = "macos")]
pub(super) fn hide(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("popover") {
        let _ = conceal(&win);
    }
}

/// Hides the popover and tells it, so it stops reading its snapshot until it is shown again (review P2-2 of #160: the
/// window is hidden, never destroyed). Every way the popover hides goes through here.
fn conceal(win: &tauri::WebviewWindow) -> tauri::Result<()> {
    win.hide()?;
    win.emit(crate::popover_data::POPOVER_HIDDEN_EVENT, ())
}

#[cfg(target_os = "macos")]
fn session_present(app: &tauri::AppHandle) -> bool {
    let state = app.state::<AppState>();
    let Ok(acct) = state.active_account() else {
        return false;
    };
    let unlocked = acct.session.lock().map(|session| session.is_some()).unwrap_or(false);
    let present = state.auth_present.lock().map(|present| *present).unwrap_or(false);
    (unlocked || present) && !acct.auth_health.is_expired()
}

#[cfg(target_os = "macos")]
fn place(app: &tauri::AppHandle, win: &tauri::WebviewWindow) -> Result<(), String> {
    use surfaces::{
        anchor::{self, LRect, Screen},
        window_state::icon_display,
    };
    let (displays, primary) = macos_displays(win);
    let raw_icon = macos_status_item_rect(app);
    let icon = raw_icon.and_then(|raw| {
        icon_display(raw, &displays).map(|i| {
            let scale = displays[i].scale;
            LRect::new(
                raw.x as f64 / scale,
                raw.y as f64 / scale,
                raw.w as f64 / scale,
                raw.h as f64 / scale,
            )
        })
    });
    let screens: Vec<Screen> = displays
        .iter()
        .enumerate()
        .map(|(i, display)| {
            // `window_state` and `anchor` each have their own logical rectangle; same fields, distinct types.
            let logical = |r: surfaces::window_state::LRect| LRect::new(r.x, r.y, r.w, r.h);
            Screen {
                frame: logical(display.frame),
                visible: logical(display.work_area),
                primary: i == primary,
            }
        })
        .collect();
    let position = anchor::place_popover(icon, &screens).ok_or("Beebeeb couldn’t find a display for its menu.")?;
    let (origin, size) = popover_frame(&position);
    win.set_size(size).map_err(|e| e.to_string())?;
    win.set_position(origin).map_err(|e| e.to_string())
}

/// The popover's frame from its placement, in points (review P2 of #160). tao turns a physical position into points
/// with the scale of the display the window is on now (tao 0.35.3 `set_outer_position`), which for a hidden window is
/// the display it was last shown on, so a physical origin is right only when both displays share a scale. macOS uses
/// points across every display, as `place_main_window` does. The size is the one `place_popover` fitted to the
/// display's usable area (review P3: a short display).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn popover_frame(placement: &surfaces::anchor::Placement) -> (tauri::Position, tauri::Size) {
    (
        tauri::Position::Logical(tauri::LogicalPosition::new(placement.origin_x, placement.origin_y)),
        tauri::Size::Logical(tauri::LogicalSize::new(placement.size.w, placement.size.h)),
    )
}

#[cfg(target_os = "macos")]
fn show(app: &tauri::AppHandle) -> Result<(), String> {
    let win = match app.get_webview_window("popover") {
        Some(win) => win,
        None => {
            let config = app
                .config()
                .app
                .windows
                .iter()
                .find(|w| w.label == "popover")
                .ok_or("The menu-bar window configuration is missing.")?;
            let win = tauri::WebviewWindowBuilder::from_config(app, config)
                .map_err(|e| e.to_string())?
                .build()
                .map_err(|e| e.to_string())?;
            let target = win.clone();
            let focused = Arc::new(AtomicBool::new(false));
            win.on_window_event(move |event| {
                if let tauri::WindowEvent::Focused(value) = event {
                    if *value {
                        focused.store(true, Ordering::Relaxed);
                    } else if focused.swap(false, Ordering::Relaxed) {
                        if target.is_visible().unwrap_or(false) && !GEAR_MENU_OPEN.load(Ordering::Relaxed) {
                            BLUR.lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .record_blur_hide(clock_ms());
                            let _ = conceal(&target);
                        }
                    }
                }
            });
            // Prefix these ids: app-level menu listeners must not run Pause/Resume a second time.
            win.on_menu_event(|window, event| {
                let Some(id) = event.id().as_ref().strip_prefix("popover:") else {
                    return;
                };
                let app = window.app_handle();
                if id == "open" {
                    hide(app);
                    reopen_main_window(app);
                } else if let Some(spec) = desktop_menu_spec(id) {
                    hide(app);
                    handle_desktop_menu_action(app, spec);
                }
            });
            win
        }
    };
    place(app, &win)?;
    unhide_app(app);
    win.show().map_err(|e| e.to_string())?;
    win.set_focus().map_err(|e| e.to_string())?;
    win.emit(crate::popover_data::POPOVER_SHOWN_EVENT, ()).map_err(|e| e.to_string())
}

/// Both mouse buttons behave alike. A revoked session goes straight to the in-window sign-in flow.
#[cfg(target_os = "macos")]
pub(super) fn toggle(app: &tauri::AppHandle) {
    if !session_present(app) {
        hide(app);
        show_main_window(app, MainNav::Home);
        return;
    }
    let visible = app
        .get_webview_window("popover")
        .is_some_and(|w| w.is_visible().unwrap_or(false));
    let action = BLUR
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .on_tray_click(clock_ms(), visible);
    match action {
        surfaces::blur_guard::TrayClickAction::Hide => hide(app),
        surfaces::blur_guard::TrayClickAction::StayHidden => {}
        surfaces::blur_guard::TrayClickAction::Show => {
            if let Err(error) = show(app) {
                tracing::warn!(%error, "could not show the menu-bar popover");
            }
        }
    }
}

#[tauri::command]
pub(super) fn popover_hide(window: tauri::WebviewWindow) -> Result<(), String> {
    require_popover(&window)?;
    conceal(&window).map_err(|e| e.to_string())
}

#[tauri::command]
pub(super) fn popover_quit(app: tauri::AppHandle, window: tauri::WebviewWindow) -> Result<(), String> {
    require_popover(&window)?;
    app.exit(0);
    Ok(())
}

#[tauri::command]
pub(super) fn popover_open_window(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    to: String,
) -> Result<(), String> {
    require_popover(&window)?;
    #[cfg(target_os = "macos")]
    {
        match to.as_str() {
            "front" => {
                hide(&app);
                reopen_main_window(&app);
            }
            "settings" => show_main_window(&app, MainNav::settings(None)),
            "review" => show_main_app_window_with_nav(&app, Some("review")),
            _ => return Err("That view is not available.".into()),
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, to);
        Err("The menu-bar popover is available on macOS.".into())
    }
}

#[tauri::command]
pub(super) async fn popover_retry_sync(app: tauri::AppHandle, window: tauri::WebviewWindow) -> Result<(), String> {
    require_popover(&window)?;
    // `start_engine_unless_running` is not built on Windows, and only macOS has the popover.
    #[cfg(target_os = "macos")]
    {
        let state = app.state::<AppState>();
        start_engine_unless_running(app.clone(), &state).await
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Err("The menu-bar popover is available on macOS.".into())
    }
}

#[tauri::command]
pub(super) async fn popover_unlock(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), String> {
    require_popover(&window)?;
    match unlock_vault(app.clone(), state).await {
        #[cfg(target_os = "macos")]
        Err(error) if error == NO_VAULT_KEY_UNLOCK_ERROR => {
            show_main_window(&app, MainNav::Setup);
            Ok(())
        }
        result => result,
    }
}

#[tauri::command]
pub(super) fn popover_gear_menu(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    x: f64,
    y: f64,
) -> Result<(), String> {
    require_popover(&window)?;
    #[cfg(target_os = "macos")]
    {
        let item = |id: &str| -> tauri::Result<MenuItem<tauri::Wry>> {
            let spec = desktop_menu_spec_required(id);
            let label = if id == MENU_TOGGLE_SYNC_ID {
                pause_menu_label_now()
            } else {
                spec.label
            };
            let mut builder = MenuItemBuilder::new(label).id(format!("popover:{id}"));
            if let Some(key) = spec.accelerator {
                builder = builder.accelerator(key);
            }
            if let Some(kept) = app
                .try_state::<MacosMenuItems>()
                .and_then(|items| items.get(id).cloned())
            {
                builder = builder.enabled(kept.is_enabled().unwrap_or(false));
            }
            builder.build(&app)
        };
        let build = || -> tauri::Result<Menu<tauri::Wry>> {
            let open = MenuItem::with_id(&app, "popover:open", "Open Beebeeb", true, None::<&str>)?;
            let help = Submenu::with_items(
                &app,
                "Help",
                true,
                &[
                    &item(MENU_HELP_DOCS_ID)?,
                    &item(MENU_HELP_SHORTCUTS_ID)?,
                    &item(MENU_HELP_REPORT_ID)?,
                    &item(MENU_HELP_STATUS_ID)?,
                ],
            )?;
            Menu::with_items(
                &app,
                &[
                    &open,
                    &item(MENU_PREFERENCES_ID)?,
                    &PredefinedMenuItem::separator(&app)?,
                    &item(MENU_TOGGLE_SYNC_ID)?,
                    &item(MENU_CHECK_UPDATES_ID)?,
                    &PredefinedMenuItem::separator(&app)?,
                    &help,
                    &PredefinedMenuItem::separator(&app)?,
                    &item(MENU_QUIT_ID)?,
                ],
            )
        };
        let menu = build().map_err(|e| e.to_string())?;
        GEAR_MENU_OPEN.store(true, Ordering::Relaxed);
        let result = window
            .popup_menu_at(
                &menu,
                tauri::LogicalPosition::new(
                    if x.is_finite() { x.clamp(0.0, 372.0) } else { 0.0 },
                    if y.is_finite() { y.clamp(0.0, 488.0) } else { 0.0 },
                ),
            )
            .map_err(|e| e.to_string());
        GEAR_MENU_OPEN.store(false, Ordering::Relaxed);
        if !window.is_focused().unwrap_or(false) && window.is_visible().unwrap_or(false) {
            BLUR.lock()
                .unwrap_or_else(|p| p.into_inner())
                .record_blur_hide(clock_ms());
            let _ = conceal(&window);
        }
        result
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, x, y);
        Err("The menu-bar menu is available on macOS.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::{changes_left, popover_frame};
    use crate::sign_out::waiting::{OtherChange, WaitingChanges, WaitingFile};
    use crate::surfaces::anchor::{LSize, Placement};

    /// Review P3-5 (#160): during sign-out's send phase, one failed read of the waiting changes failed the whole
    /// snapshot, so the popover and the Beebeeb window showed a read failure for that tick. It leaves the count out
    /// instead (`progressLine` shows none for 0); a read that works counts files and other changes.
    #[test]
    fn a_failed_count_while_sign_out_sends_leaves_the_count_out() {
        assert_eq!(changes_left(Err("database is locked".into())), 0);
        let file = |id: &str| WaitingFile { file_id: id.into(), path: None, writes: Vec::new() };
        let waiting = WaitingChanges {
            files: vec![file("a"), file("b")],
            others: vec![OtherChange { op_id: "op".into(), live: false }],
        };
        assert_eq!(changes_left(Ok(waiting)), 3);
    }

    /// Review P2 (#160, `place`): tao turns a physical position into points with the scale of the display the window
    /// is on NOW (tao 0.35.3 `set_outer_position`), not the display it is placed on. With a 2x laptop and a 1x
    /// external display, a physical origin computed for one display lands at double or half the coordinates on the
    /// other. The frame is set in points, which macOS uses across every display, whatever the target's scale. And the
    /// size `place_popover` fitted to a short display is applied (review P3), so the window never runs over the Dock.
    #[test]
    fn the_popover_frame_is_set_in_points_on_any_display() {
        let placement = Placement {
            origin_x: 1512.5,
            origin_y: 33.0,
            size: LSize { w: 372.0, h: 420.0 },
            screen: 1,
            used_fallback: false,
        };
        let (origin, size) = popover_frame(&placement);
        match origin {
            tauri::Position::Logical(origin) => assert_eq!((origin.x, origin.y), (1512.5, 33.0)),
            other => panic!("the origin is not in points: {other:?}"),
        }
        match size {
            tauri::Size::Logical(size) => assert_eq!((size.width, size.height), (372.0, 420.0)),
            other => panic!("the fitted size is not set in points: {other:?}"),
        }
    }
}
