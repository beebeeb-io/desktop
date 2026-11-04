    use super::config_write_pin_tests::production_files;
    use super::finder_setup_command_tests::{body_between, macos_compiled, production_source};

    #[test]
    fn startup_asks_the_policy_with_how_the_app_was_launched_and_handles_every_answer() {
        let production = production_source();
        let setup = &production[production.find(".setup(|app| {").expect("setup()")..];
        let ask = setup.find("let launch_source = launch_source_now();").expect("setup reads how the app was launched");
        let policy = setup.find("surfaces::policy::startup_surface(").expect("setup asks the policy");
        assert!(ask < policy, "the launch is read before the policy is asked");
        assert!(
            setup[policy..].starts_with("surfaces::policy::startup_surface(surfaces::policy::Platform::current(), no_sync_root, launch_source)"),
            "the policy gets the platform, whether this is a first run, and the launch"
        );
        for answer in ["StartupSurface::Onboarding =>", "StartupSurface::MainWindow =>", "StartupSurface::Nothing =>"] {
            assert!(setup.contains(answer), "setup handles {answer}");
        }
        let nothing = &setup[setup.find("StartupSurface::Nothing =>").unwrap()..];
        let nothing = &nothing[..nothing.find("\n                }\n").expect("the arm ends")];
        assert!(!nothing.contains("show_") && !nothing.contains("open_"), "a launch that shows nothing opens nothing:\n{nothing}");
    }

    /// PR #146 review, P2: the app starts Regular (a Dock icon and the app menu) and Accessory is applied only from window
    /// events, so a launch that opens no window must switch it itself: a Dock icon and a menu bar with no window behind
    /// them are a Beebeeb that looks open and has nothing to show.
    #[test]
    fn a_launch_that_opens_nothing_takes_the_dock_icon_and_the_app_menu_away() {
        let production = production_source();
        let setup = &production[production.find(".setup(|app| {").expect("setup()")..];
        let arm = &setup[setup.find("StartupSurface::Nothing =>").unwrap()..];
        let arm = &arm[..arm.find("\n                }\n").expect("the arm ends")];
        assert!(
            arm.contains("#[cfg(target_os = \"macos\")]\n                    set_macos_dock_icon(app.handle(), false);"),
            "the Nothing arm sets Accessory, on a Mac only:\n{arm}"
        );
    }

    #[test]
    fn how_the_app_was_launched_comes_from_the_one_seam_and_the_flag_and_from_nothing_else() {
        let production = macos_compiled(&production_source());
        let body = body_between(&production, "fn launch_source_now() -> surfaces::launch::LaunchSource {", "\n}\n");
        assert!(body.contains("surfaces::launch::take_relaunch_flag("), "the flag is consumed here");
        assert!(body.contains("surfaces::launch::login_item_signal()"), "the login item's word comes from the seam");
        assert_eq!(
            production.matches("login_item_signal()").count(),
            1,
            "the seam is read in one place; a second reader is a second opinion on how the app was launched"
        );
        // Every production file, not only lib.rs (PR #146 review, P3): a guess at a login launch belongs to task 1845's
        // seam, `surfaces/launch.rs`, and to nothing else under `src-tauri/src`. `getppid(` is a call, not the letters.
        let files = production_files();
        assert!(files.len() > 20 && files.iter().any(|(name, _)| name == "surfaces/launch.rs"), "the scan found the tree");
        for (name, text) in files.iter().filter(|(name, _)| name != "surfaces/launch.rs") {
            for guess in ["XPC_SERVICE_NAME", "--autostart", "LaunchedAsLogInItem", "getppid("] {
                assert!(!text.contains(guess), "{name}: {guess}: a guess at a login launch belongs to task 1845's seam, not here");
            }
        }
    }

    #[test]
    fn the_updater_leaves_its_flag_before_every_restart_and_there_are_only_these_restarts() {
        // Every production file (PR #146 review, P3): a restart anywhere in `src-tauri/src` is a relaunch with no flag.
        let files = production_files();
        let restarts: Vec<&str> = files
            .iter()
            .flat_map(|(name, text)| {
                [".restart(", "process::restart", "request_restart", ".relaunch("]
                    .into_iter()
                    .flat_map(move |needle| text.matches(needle).map(move |_| name.as_str()))
            })
            .collect();
        assert_eq!(restarts, ["lib.rs", "lib.rs"], "a third restart is a relaunch with no flag: decide it here, on purpose");
        let production = macos_compiled(&production_source());
        assert_eq!(production.matches("app.restart(").count(), 2);
        assert_eq!(production.matches("note_updater_relaunch(&app);").count(), 2);
        for signature in ["async fn install_update(", "async fn install_channel_downgrade("] {
            let body = body_between(&production, signature, "\n}\n");
            let note = body.find("note_updater_relaunch(&app);").unwrap_or_else(|| panic!("{signature} leaves the flag"));
            let restart = body.find("app.restart(").unwrap_or_else(|| panic!("{signature} restarts"));
            assert!(note < restart, "{signature}: the flag first, then the restart");
        }
    }

    #[test]
    fn opening_the_app_again_asks_reopen_action() {
        let production = macos_compiled(&production_source());
        let body = body_between(&production, "fn reopen_main_window(", "\n}\n");
        assert!(body.contains("surfaces::launch::reopen_action("));
        assert!(body.contains("show_main_window(app, MainNav::Home)"), "no window: Home");
    }

    #[test]
    fn the_login_items_mechanism_is_the_one_it_was_and_task_1845_changes_it_on_purpose() {
        let production = production_source();
        assert_eq!(
            production.matches(".plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec![])))").count(),
            1,
            "slice 6 does not touch how the login item is registered: a LaunchAgent that passes no argument"
        );
        assert!(production.contains("fn toggle_autostart("), "and the switch that registers it is still there");
    }
