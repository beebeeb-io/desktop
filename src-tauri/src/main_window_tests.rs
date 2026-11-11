use super::*;

#[test]
fn a_request_serialises_as_the_window_reads_it() {
    // `src/mainWindowModel.ts` `parseMainNav` reads exactly these three shapes.
    let json = |nav: MainNav| serde_json::to_value(nav).unwrap();
    assert_eq!(json(MainNav::Home), serde_json::json!({ "to": "home" }));
    assert_eq!(json(MainNav::Back), serde_json::json!({ "to": "back" }));
    assert_eq!(
        json(MainNav::settings(Some("about"))),
        serde_json::json!({ "to": "settings", "section": "about" })
    );
    assert_eq!(
        json(MainNav::settings(None)),
        serde_json::json!({ "to": "settings", "section": null })
    );
}

#[test]
fn the_slot_keeps_the_latest_request_and_gives_it_to_the_beebeeb_window_once() {
    let slot = std::sync::Mutex::new(None);
    let main = surfaces::policy::main_window_label(surfaces::policy::Platform::current());
    store_main_nav(&slot, MainNav::Home);
    store_main_nav(&slot, MainNav::settings(Some("account")));
    for other in ["settings", "onboarding", "tray", "main-app", "conflict-1"] {
        if other != main {
            assert_eq!(
                take_main_nav(&slot, other),
                None,
                "{other} must not take the window's request"
            );
        }
    }
    assert_eq!(take_main_nav(&slot, main), Some(MainNav::settings(Some("account"))));
    assert_eq!(take_main_nav(&slot, main), None, "taken once");
}

#[test]
fn the_menu_around_the_window_follows_section_8_2() {
    // No window: Settings… and Home open it; Back has nothing to go back from.
    let on = |back| MainMenuState {
        settings: true,
        home: true,
        back,
        check_updates: true,
    };
    assert_eq!(main_menu_state(false, false, false), on(false));
    // Home in the window: as above.
    assert_eq!(main_menu_state(true, false, false), on(false));
    // Settings shows: Back on.
    assert_eq!(main_menu_state(true, true, false), on(true));
    // A sheet is open: ⌘1, ⌘[, ⌘, and Check for Updates… (which opens Settings › About) are all off, so nothing
    // unmounts the section under the sheet (review item 6: disabled with the §8.2 items, not a second answer).
    let off = MainMenuState {
        settings: false,
        home: false,
        back: false,
        check_updates: false,
    };
    assert_eq!(main_menu_state(true, true, true), off);
    assert_eq!(main_menu_state(true, false, true), off);
    // A sheet with no window cannot exist; a stale report of one never disables the openers.
    assert_eq!(main_menu_state(false, true, true), on(false));
}

#[test]
fn the_pause_item_says_what_it_will_do() {
    assert_eq!(pause_menu_label(false), "Pause Sync");
    assert_eq!(pause_menu_label(true), "Resume Sync");
}

/// Review item 3: the label follows the file whichever way the pause was written: the menu or the tray
/// (`persist_sync_paused_at`) and a settings write from the compact flyout's Network page (`set_desktop_config`).
#[tokio::test]
async fn the_pause_label_follows_every_writer_of_the_pause() {
    let dir = tempfile::tempdir().expect("temp dir");
    crate::config::CONFIG_DIR_OVERRIDE
        .scope(dir.path().to_path_buf(), async {
            assert_eq!(pause_menu_label_now(), "Pause Sync");
            let patch: crate::config::DesktopSettings =
                serde_json::from_str(r#"{ "pause_sync": true }"#).expect("the Network page sends the pause");
            save_desktop_settings(patch).expect("saved");
            assert_eq!(pause_menu_label_now(), "Resume Sync");
            let path = DesktopConfig::path().expect("the scoped config path");
            let flag = std::sync::atomic::AtomicBool::new(true);
            persist_sync_paused_at(&path, &flag, false).expect("the menu resumes");
            assert_eq!(pause_menu_label_now(), "Pause Sync");
        })
        .await;
}

#[test]
fn every_pause_writer_refreshes_the_menu_label() {
    // Every pause goes through `set_sync_paused`, which refreshes the label; a settings write only when it carries
    // a pause (review round 2: no config read on every slider move).
    let production = super::finder_setup_command_tests::production_source();
    let body = |signature: &str| super::finder_setup_command_tests::body_between(&production, signature, "\n}\n");
    assert!(body("async fn set_sync_paused(").contains("pause_state_changed(app);"));
    assert!(body("async fn set_desktop_config(").contains("apply_desktop_settings(&app, config).await"));
    let apply = body("async fn apply_desktop_settings(");
    assert!(apply.contains("let pause = config.pause_sync.take();"), "{apply}");
    assert!(apply.contains("set_sync_paused(app, paused).await?;"), "{apply}");
    assert!(
        !apply.contains("pause_state_changed("),
        "a write without a pause refreshes nothing:\n{apply}"
    );
    assert!(
        !production.contains("refresh_macos_pause_menu_label(app);\n    persisted?;"),
        "set_sync_paused refreshes through the one shared call"
    );
}

/// Review round 2, P2-3: a pause sent with the settings (the compact flyout's Network page) pauses the engine as the
/// menu's and the tray's do: the file, the engine's flag, the engine-status event and the menu label agree. A
/// settings write without a pause leaves the pause alone.
// The mock runtime is not built on Windows (Cargo.toml: `tauri/test` is off there, after slice 1's 0xc0000139 at
// load); Linux and macOS run this.
#[cfg(not(target_os = "windows"))]
#[tokio::test]
async fn a_pause_sent_with_the_settings_pauses_the_engine() {
    let dir = tempfile::tempdir().expect("temp dir");
    crate::config::CONFIG_DIR_OVERRIDE
        .scope(dir.path().to_path_buf(), async {
            let app = tauri::test::mock_app();
            app.manage(AppState::default());
            synthesize_single_account(&*app.state::<AppState>(), account::AccountId("acct-test".into()));
            let flag = || {
                app.state::<AppState>()
                    .active_account()
                    .unwrap()
                    .sync_paused
                    .load(Ordering::Relaxed)
            };
            let send = |json: &str| serde_json::from_str::<crate::config::DesktopSettings>(json).unwrap();

            apply_desktop_settings(app.handle(), send(r#"{ "pause_sync": true }"#))
                .await
                .expect("saved");
            assert!(flag(), "the engine's flag is paused");
            assert!(DesktopConfig::load().unwrap().pause_sync, "and so is the file");
            assert_eq!(pause_menu_label_now(), "Resume Sync");

            apply_desktop_settings(app.handle(), send(r#"{ "upload_kbps_limit": 1000 }"#))
                .await
                .expect("saved");
            assert!(flag(), "a speed change leaves the pause alone");
            assert_eq!(DesktopConfig::load().unwrap().upload_kbps_limit, 1000);

            apply_desktop_settings(app.handle(), send(r#"{ "pause_sync": false }"#))
                .await
                .expect("saved");
            assert!(!flag(), "resumed");
            assert!(!DesktopConfig::load().unwrap().pause_sync);
        })
        .await;
}

/// Review round 2: a kept frame that is missing, empty, partial, not UTF-8, not a frame, or names a display that is
/// gone falls back to the default placement, and nothing panics.
#[test]
fn a_bad_kept_frame_file_is_no_frame_and_the_window_opens_at_its_default_place() {
    use surfaces::window_state::{Display, LRect, PRect, plan_main_window};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(surfaces::window_state::MAIN_FRAME_FILENAME);
    assert_eq!(read_kept_frame(&path), None, "missing");
    for bad in [
        &b""[..],
        b"{\"x\":12,\"y\"",
        b"\xff\xfe\x00",
        b"null",
        b"{\"x\":1,\"y\":2,\"w\":0,\"h\":3}",
    ] {
        std::fs::write(&path, bad).unwrap();
        assert_eq!(read_kept_frame(&path), None, "{bad:?}");
    }
    let built_in = Display::from_physical(
        PRect {
            x: 0,
            y: 0,
            w: 3024,
            h: 1964,
        },
        PRect {
            x: 0,
            y: 74,
            w: 3024,
            h: 1750,
        },
        2.0,
    );
    let default = plan_main_window(None, &[built_in], Some(0), 0, (920.0, 620.0));
    assert!(default.is_some());
    // A frame left on an external display that is gone: the default place, at the kept size.
    let gone = LRect {
        x: 2500.0,
        y: 100.0,
        w: 920.0,
        h: 620.0,
    };
    write_kept_frame(&path, &gone).unwrap();
    assert_eq!(read_kept_frame(&path), Some(gone));
    assert_eq!(
        plan_main_window(read_kept_frame(&path), &[built_in], Some(0), 0, (920.0, 620.0)),
        default
    );
}

#[test]
fn the_kept_frame_is_written_whole_or_not_at_all() {
    use surfaces::window_state::LRect;
    let dir = tempfile::tempdir().unwrap();
    let path = dir
        .path()
        .join("nested")
        .join(surfaces::window_state::MAIN_FRAME_FILENAME);
    let first = LRect {
        x: 100.0,
        y: 80.0,
        w: 920.0,
        h: 620.0,
    };
    let second = LRect {
        x: 2200.0,
        y: 300.0,
        w: 1000.0,
        h: 700.0,
    };
    write_kept_frame(&path, &first).unwrap();
    write_kept_frame(&path, &second).unwrap();
    assert_eq!(
        read_kept_frame(&path),
        Some(second),
        "the second write replaced the first"
    );
    assert!(!path.with_extension("json.tmp").exists(), "no temporary file is left");
    let production = super::finder_setup_command_tests::production_source();
    let body = super::finder_setup_command_tests::body_between(&production, "fn write_kept_frame(", "\n}\n");
    let written = body
        .find("std::fs::write(&temporary,")
        .expect("written to a temporary file");
    let renamed = body
        .find("std::fs::rename(&temporary, path)")
        .expect("then renamed over the frame");
    assert!(written < renamed, "{body}");
}

#[test]
fn the_menu_keeps_its_four_items_from_the_table_without_a_lookup_that_can_fail() {
    // Review item 13.
    let production = super::finder_setup_command_tests::production_source();
    let body = super::finder_setup_command_tests::body_between(
        &production,
        "#[cfg(target_os = \"macos\")]\nfn setup_native_menu(",
        "\n}\n",
    );
    assert!(!body.contains("HashMap"), "{body}");
    assert!(!body.contains("AssetNotFound"), "{body}");
    for id in [
        "MENU_PREFERENCES_ID",
        "MENU_CHECK_UPDATES_ID",
        "MENU_TOGGLE_SYNC_ID",
        "MENU_VIEW_HOME_ID",
        "MENU_VIEW_BACK_ID",
    ] {
        assert!(
            body.contains(&format!("desktop_menu_spec_required({id})")),
            "{id}:\n{body}"
        );
    }
}

#[test]
fn a_route_named_opener_asks_for_settings_or_home() {
    assert_eq!(main_nav_for_route(Some("settings")), MainNav::settings(None));
    assert_eq!(main_nav_for_route(None), MainNav::Home);
    assert_eq!(main_nav_for_route(Some("finder")), MainNav::Home);
}

fn mac(id: &str) -> &'static DesktopMenuSpec {
    MACOS_MENU_SPECS
        .iter()
        .find(|spec| spec.id == id)
        .unwrap_or_else(|| panic!("no macOS menu item {id}"))
}

#[test]
fn the_macos_items_have_the_words_keys_and_targets_of_f1_to_f11() {
    let rows: [(&str, &str, Option<&str>, DesktopMenuAction); 11] = [
        (
            MENU_PREFERENCES_ID,
            "Settings…",
            Some("CmdOrCtrl+Comma"),
            DesktopMenuAction::OpenSettings,
        ),
        (
            MENU_CHECK_UPDATES_ID,
            "Check for Updates…",
            Some("CmdOrCtrl+Shift+U"),
            DesktopMenuAction::CheckForUpdates,
        ),
        (
            MENU_TOGGLE_SYNC_ID,
            "Pause Sync",
            Some("CmdOrCtrl+Shift+P"),
            DesktopMenuAction::ToggleSync,
        ),
        (
            MENU_SIGN_OUT_ID,
            "Sign Out…",
            Some("CmdOrCtrl+Shift+L"),
            DesktopMenuAction::SignOut,
        ),
        (
            MENU_QUIT_ID,
            "Quit Beebeeb",
            Some("CmdOrCtrl+Q"),
            DesktopMenuAction::Quit,
        ),
        (
            MENU_OPEN_FOLDER_ID,
            "Open in Finder",
            Some("CmdOrCtrl+O"),
            DesktopMenuAction::OpenFolder,
        ),
        (
            MENU_UPLOAD_FILES_ID,
            "Upload Files…",
            Some("CmdOrCtrl+U"),
            DesktopMenuAction::UploadFiles,
        ),
        (
            MENU_NEW_FOLDER_ID,
            "New Folder…",
            Some("CmdOrCtrl+Shift+N"),
            DesktopMenuAction::NewFolder,
        ),
        (
            MENU_OPEN_WEB_APP_ID,
            "Open Web App",
            Some("CmdOrCtrl+Shift+O"),
            DesktopMenuAction::OpenWebApp,
        ),
        (
            MENU_VIEW_HOME_ID,
            "Home",
            Some("CmdOrCtrl+Digit1"),
            DesktopMenuAction::NavigateMain(MainMenuNav::Home),
        ),
        (
            MENU_VIEW_BACK_ID,
            "Back",
            Some("CmdOrCtrl+BracketLeft"),
            DesktopMenuAction::NavigateMain(MainMenuNav::Back),
        ),
    ];
    for (id, label, accelerator, action) in rows {
        let spec = mac(id);
        assert_eq!(
            (spec.label, spec.accelerator, spec.action),
            (label, accelerator, action),
            "{id}"
        );
    }
}

#[test]
fn the_zoom_and_help_items_keep_their_words_f12_f13() {
    // "Unchanged": the same words, keys and targets as the shared table.
    for id in [
        MENU_ZOOM_IN_ID,
        MENU_ZOOM_OUT_ID,
        MENU_ZOOM_RESET_ID,
        MENU_HELP_DOCS_ID,
        MENU_HELP_SHORTCUTS_ID,
        MENU_HELP_REPORT_ID,
        MENU_HELP_STATUS_ID,
    ] {
        let shared = DESKTOP_MENU_SPECS.iter().find(|spec| spec.id == id).unwrap();
        let spec = mac(id);
        assert_eq!(
            (spec.label, spec.accelerator, spec.action),
            (shared.label, shared.accelerator, shared.action),
            "{id}"
        );
    }
}

#[test]
fn macos_binds_each_key_once_and_drops_the_legacy_items_f10_f11() {
    let mut keys: Vec<&str> = MACOS_MENU_SPECS.iter().filter_map(|spec| spec.accelerator).collect();
    let total = keys.len();
    keys.sort();
    keys.dedup();
    assert_eq!(keys.len(), total, "a key is bound twice: {keys:?}");
    let ids: Vec<&str> = MACOS_MENU_SPECS.iter().map(|spec| spec.id).collect();
    // F10: File › Settings (the second ⌘,) is gone. F11: Files ⌘1, Activity ⌘2, Trash ⌘4 are gone.
    for gone in [
        MENU_SETTINGS_ID,
        MENU_VIEW_FILES_ID,
        MENU_VIEW_ACTIVITY_ID,
        MENU_VIEW_TRASH_ID,
        MENU_VIEW_SHARED_ID,
    ] {
        assert!(!ids.contains(&gone), "{gone}");
    }
    assert!(
        !MACOS_MENU_SPECS
            .iter()
            .any(|spec| matches!(spec.action, DesktopMenuAction::OpenView(_)))
    );
    for spec in MACOS_MENU_SPECS {
        assert!(
            !spec.label.contains("..."),
            "{}: macOS uses the ellipsis character",
            spec.id
        );
    }
}

fn entries(title: &str) -> &'static [MacosMenuEntry] {
    MACOS_MENU_LAYOUT
        .iter()
        .find(|(t, _)| *t == title)
        .map(|(_, e)| *e)
        .unwrap_or_else(|| panic!("no {title} menu"))
}

#[test]
fn the_macos_menu_bar_is_the_order_of_section_9_2_and_f15() {
    use MacosMenuEntry::*;
    let titles: Vec<&str> = MACOS_MENU_LAYOUT.iter().map(|(t, _)| *t).collect();
    assert_eq!(titles, ["Beebeeb", "File", "Edit", "View", "Window", "Help"]);
    assert_eq!(
        entries("Beebeeb"),
        [
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
        ]
    );
    assert_eq!(
        entries("File"),
        [
            Item(MENU_OPEN_FOLDER_ID),
            Item(MENU_UPLOAD_FILES_ID),
            Item(MENU_NEW_FOLDER_ID),
            Separator,
            Item(MENU_OPEN_WEB_APP_ID),
        ]
    );
    assert_eq!(entries("Edit"), [Cut, Copy, Paste, SelectAll]);
    assert_eq!(
        entries("View"),
        [
            Item(MENU_VIEW_HOME_ID),
            Separator,
            Item(MENU_VIEW_BACK_ID),
            Separator,
            Item(MENU_ZOOM_IN_ID),
            Item(MENU_ZOOM_OUT_ID),
            Item(MENU_ZOOM_RESET_ID),
        ]
    );
    assert_eq!(entries("Window"), [Minimize, Zoom, Close, Separator, BringAllToFront]);
}

#[test]
fn every_macos_item_is_in_the_menu_bar_exactly_once_and_every_entry_has_an_item() {
    let mut placed: Vec<&str> = MACOS_MENU_LAYOUT
        .iter()
        .flat_map(|(_, e)| e.iter())
        .filter_map(|entry| match entry {
            MacosMenuEntry::Item(id) => Some(*id),
            _ => None,
        })
        .collect();
    for id in &placed {
        mac(id);
    }
    placed.sort();
    let mut specs: Vec<&str> = MACOS_MENU_SPECS.iter().map(|spec| spec.id).collect();
    specs.sort();
    assert_eq!(placed, specs);
}

#[test]
fn the_installed_table_is_the_platforms_own() {
    // By content: a `const` slice has no fixed address to compare.
    let ids = |table: &[DesktopMenuSpec]| table.iter().map(|spec| (spec.id, spec.label)).collect::<Vec<_>>();
    let installed = ids(installed_menu_specs());
    if cfg!(target_os = "macos") {
        assert_eq!(installed, ids(MACOS_MENU_SPECS));
        assert_eq!(desktop_menu_spec(MENU_PREFERENCES_ID).unwrap().label, "Settings…");
        assert!(desktop_menu_spec(MENU_SETTINGS_ID).is_none());
    } else {
        assert_eq!(installed, ids(DESKTOP_MENU_SPECS));
        assert_eq!(desktop_menu_spec(MENU_PREFERENCES_ID).unwrap().label, "Preferences...");
    }
}

#[test]
fn the_window_commands_are_registered_once() {
    let production = super::finder_setup_command_tests::production_source();
    let handler = super::finder_setup_command_tests::body_between(&production, "tauri::generate_handler![", "])");
    for command in ["consume_main_navigation,", "main_window_state,"] {
        assert_eq!(
            handler.lines().filter(|line| line.trim() == command).count(),
            1,
            "{command}"
        );
    }
}
