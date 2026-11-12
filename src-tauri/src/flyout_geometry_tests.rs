//! Tests relocated from the deleted `desktop_main_window.rs`: tray flyout geometry, settings navigation,
//! status-line flyout position and the window-state denylist. The functions they test live in `update_window.rs`
//! and `tray_window.rs`.

#[test]
fn tray_flyout_geometry_matches_the_boards_tray_frame() {
    assert_eq!(crate::WINDOWS_TRAY_FLYOUT_WIDTH, 364);
    assert_eq!(crate::WINDOWS_TRAY_FLYOUT_HEIGHT, 592);
}

#[test]
fn tray_flyout_position_uses_configured_height_and_gap() {
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
/// so only a known page passes, and anything else is refused instead of opening General as if it had worked.
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

/// Desktop v2 M3e: the status line shows the flyout at the bottom-right of the work area (DW-0401#10 -> DW-1201),
/// at each scale, on a display whose work area starts away from 0, and never outside the work area on a display
/// smaller than the flyout.
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
fn the_tray_window_and_onboarding_are_in_the_state_denylist() {
    let labels = crate::window_state_denylist_labels();
    assert!(labels.contains(&"tray") && labels.contains(&"windows-onboarding") && labels.contains(&"main-app"));
}
