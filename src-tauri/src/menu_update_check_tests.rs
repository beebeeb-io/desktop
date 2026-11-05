    use super::*;

    #[test]
    fn pending_check_survives_other_windows_and_is_consumed_once() {
        let pending = AtomicBool::new(true);
        assert!(!take_menu_update_check(&pending, "tray"));
        assert!(!take_menu_update_check(&pending, "onboarding"));
        assert!(take_menu_update_check(&pending, update_check_window_label()));
        assert!(!take_menu_update_check(&pending, update_check_window_label()));
        pending.store(true, Ordering::SeqCst);
        assert!(take_menu_update_check(&pending, update_check_window_label()));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn menu_update_check_targets_the_beebeeb_window() {
        // Task 1683 slice 1 (F2): on macOS "Check for Updates…" opens the Beebeeb window on Settings › About, so
        // that window must be the one the pending request targets, and the About row answers inline.
        assert_eq!(update_check_window_label(), "main");
        assert_eq!(
            update_check_window_label(),
            surfaces::policy::main_window_label(surfaces::policy::Platform::Macos)
        );
    }
