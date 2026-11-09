//! The main window's size and kept frame on Windows and Linux (v2 W1): the board's 1280 x 800 with a 960 x 640 minimum,
//! fitted to the work area on first open, and kept (logical points) when it moves, resizes or closes.

use super::*;
use std::sync::atomic::Ordering;

/// The board's main window: opens at 1280 x 800, never below 960 x 640 (logical points).
pub(super) const MAIN_DEFAULT_SIZE: (f64, f64) = (1280.0, 800.0);
pub(super) const MAIN_MINIMUM_SIZE: (f64, f64) = (960.0, 640.0);

/// Windows and Linux: places the main window from its kept frame (or fitted to the work area), before it is shown.
/// A maximized window reopens maximized on the display it was kept on.
#[cfg(not(target_os = "macos"))]
pub(super) fn place_desktop_main_window(app: &tauri::AppHandle, win: &tauri::WebviewWindow) {
    let (displays, primary) = macos_displays(win);
    let path = app
        .path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join(surfaces::window_state::MAIN_FRAME_FILENAME));
    let kept = path.as_deref().and_then(surfaces::window_state::read_kept_main);
    let Some(plan) = surfaces::window_state::plan_desktop_main_window(
        kept,
        &displays,
        primary,
        MAIN_DEFAULT_SIZE,
        MAIN_MINIMUM_SIZE,
    ) else {
        tracing::warn!("main window: no display to place it on");
        return;
    };
    let f = plan.frame;
    let _ = win.set_min_size(Some(tauri::Size::Logical(tauri::LogicalSize::new(
        plan.minimum.0,
        plan.minimum.1,
    ))));
    let _ = win.set_size(tauri::Size::Logical(tauri::LogicalSize::new(f.w, f.h)));
    let _ = win.set_position(tauri::Position::Logical(tauri::LogicalPosition::new(f.x, f.y)));
    if plan.maximized {
        let _ = win.maximize();
    }
}

/// Keeps the main window's frame and maximized state in the config directory.
#[cfg(not(target_os = "macos"))]
pub(super) fn save_desktop_main_window(app: &tauri::AppHandle) {
    let label = surfaces::policy::main_window_label(surfaces::policy::Platform::current());
    let Some(win) = app.get_webview_window(label) else {
        return;
    };
    if win.is_minimized().unwrap_or(false) {
        return;
    }
    let (Ok(position), Ok(size), Ok(scale)) = (win.outer_position(), win.inner_size(), win.scale_factor()) else {
        return;
    };
    let maximized = win.is_maximized().unwrap_or(false);
    let fullscreen = win.is_fullscreen().unwrap_or(false);
    let current = surfaces::window_state::frame_to_keep(
        (position.x, position.y),
        (size.width, size.height),
        scale,
        fullscreen,
        maximized,
    );
    let Some(path) = app
        .path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join(surfaces::window_state::MAIN_FRAME_FILENAME))
    else {
        return;
    };
    let previous = surfaces::window_state::read_kept_main(&path);
    let Some(next) = surfaces::window_state::next_kept(previous, current, maximized || fullscreen) else {
        return;
    };
    if previous != Some(next)
        && let Err(error) = surfaces::window_state::write_kept_main(&path, &next)
    {
        tracing::warn!(%error, "main window: its frame could not be kept");
    }
}

static MAIN_SAVE_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A move or resize of the main window: saved about 500 ms after the last one; a close saves at once.
#[cfg(not(target_os = "macos"))]
pub(super) fn desktop_main_window_lifecycle(window: &tauri::Window, event: &tauri::WindowEvent) {
    if window.label() != surfaces::policy::main_window_label(surfaces::policy::Platform::current()) {
        return;
    }
    let app = window.app_handle().clone();
    match event {
        tauri::WindowEvent::CloseRequested { .. } => save_desktop_main_window(&app),
        tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
            let generation = MAIN_SAVE_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(500));
                if MAIN_SAVE_GENERATION.load(Ordering::SeqCst) == generation {
                    save_desktop_main_window(&app);
                }
            });
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn tray_flyout_geometry_matches_the_boards_tray_frame() {
        assert_eq!(crate::WINDOWS_TRAY_FLYOUT_WIDTH, 364);
        assert_eq!(crate::WINDOWS_TRAY_FLYOUT_HEIGHT, 592);
    }

    #[test]
    fn tray_flyout_position_uses_configured_height_and_gap() {
        // 364 x 592 logical, a 10 logical gap, at each scale.
        let cases = [
            (1.0, 3_840.0, 2_160.0, 3_476, 1_558),
            (1.5, 3_840.0, 2_160.0, 3_294, 1_257),
            (2.0, 3_840.0, 2_160.0, 3_112, 956),
        ];
        for (scale_factor, click_x, click_y, expected_x, expected_y) in cases {
            let position = crate::windows_tray_flyout_position(click_x, click_y, scale_factor);
            assert_eq!(position.x, expected_x, "x at scale factor {scale_factor}");
            assert_eq!(position.y, expected_y, "y at scale factor {scale_factor}");
        }
    }

    #[test]
    fn tray_flyout_position_clamps_to_left_screen_edge() {
        let position = crate::windows_tray_flyout_position(250.0, 1_080.0, 1.5);
        assert_eq!((position.x, position.y), (0, 177));
    }

    /// Desktop v2 (M3e): an opener in another window names a settings page; the word goes into the main window's URL,
    /// so only a known page passes, and anything else is refused instead of opening General as if it had worked. The
    /// Linux window's route word for any settings page is Settings.
    #[test]
    fn a_settings_opener_names_a_known_page_or_is_refused() {
        assert_eq!(crate::settings_nav(None).as_deref(), Ok("settings"));
        assert_eq!(crate::settings_nav(Some("backups")).as_deref(), Ok("settings/backups"));
        assert_eq!(
            crate::settings_nav(Some("security")).as_deref(),
            Ok("settings/security")
        );
        assert_eq!(crate::settings_nav(Some("about")).as_deref(), Ok("settings/about"));
        assert_eq!(crate::settings_nav(Some("report")).as_deref(), Ok("settings/report"));
        for bad in [
            "",
            "Backups",
            "backups&x=1",
            "../general",
            "compare",
            "device",
            "general/x",
        ] {
            assert!(crate::settings_nav(Some(bad)).is_err(), "{bad:?}");
        }
        assert_eq!(
            crate::main_nav_for_route(Some("settings/backups")),
            crate::MainNav::settings(None)
        );
        assert_eq!(crate::main_nav_for_route(Some("settingsx")), crate::MainNav::Home);
    }

    /// Desktop v2 M3e: the status line shows the flyout at the bottom-right of the work area (DW-0401#10 → DW-1201),
    /// at each scale, on a display whose work area starts away from 0 (a second display, a taskbar on the left), and
    /// never outside the work area on a display smaller than the flyout.
    #[test]
    fn the_flyout_opened_from_the_status_line_sits_at_the_work_areas_bottom_right() {
        let cases = [
            (1.0, (0, 0, 1_920, 1_032), (1_556, 440)),
            (1.5, (0, 0, 2_880, 1_548), (2_334, 660)),
            (2.0, (3_840, 0, 3_840, 2_064), (6_952, 880)),
            (1.0, (48, 0, 1_872, 1_080), (1_556, 488)),
        ];
        for (scale, (x, y, w, h), expected) in cases {
            let position = crate::windows_flyout_position_in_work_area(x, y, w, h, scale);
            assert_eq!(
                (position.x, position.y),
                expected,
                "work area {x},{y} {w}x{h} at {scale}"
            );
        }
        let small = crate::windows_flyout_position_in_work_area(100, 50, 300, 400, 1.0);
        assert_eq!(
            (small.x, small.y),
            (100, 50),
            "kept inside a work area smaller than the flyout"
        );
    }

    #[test]
    fn the_main_window_keeps_its_own_frame_so_the_plugin_leaves_it() {
        let labels = crate::window_state_denylist_labels();
        assert!(labels.contains(&"tray") && labels.contains(&"windows-onboarding") && labels.contains(&"main-app"));
        assert_eq!(
            (super::MAIN_DEFAULT_SIZE, super::MAIN_MINIMUM_SIZE),
            ((1280.0, 800.0), (960.0, 640.0))
        );
    }
}
