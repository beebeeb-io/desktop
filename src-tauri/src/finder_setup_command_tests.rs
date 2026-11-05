    use super::*;
    use finder_setup::core::Trigger;
    use finder_setup::driver::{Event, FinderSetupHandle, FinderSetupView};
    use finder_setup::launch_location::LaunchLocation;

    #[test]
    fn finder_setup_state_reads_the_reconcilers_view_and_says_so_when_there_is_none() {
        let state = AppState::default();
        assert_eq!(
            finder_setup_state_impl(&state),
            Err("Finder setup is only available on macOS.".to_string())
        );
        let view = FinderSetupView::initial(LaunchLocation::DiskImage);
        let _ = state.finder_setup.set(FinderSetupHandle::fixed(view.clone()));
        assert_eq!(finder_setup_state_impl(&state), Ok(view));
    }

    #[test]
    fn try_again_sends_exactly_one_try_again_trigger() {
        let state = AppState::default();
        let (handle, mut rx) = FinderSetupHandle::for_test(FinderSetupView::initial(LaunchLocation::Applications));
        let _ = state.finder_setup.set(handle);
        finder_setup_retry_impl(&state).expect("sent");
        assert!(matches!(rx.try_recv(), Ok(Event::Trigger(Trigger::TryAgain))));
        assert!(rx.try_recv().is_err(), "exactly one");
    }

    /// Area A M4: while the reconciler is held (after a sign-out or a Lock), "Try again" could do nothing. It says so,
    /// in the quiet `Missing` sentence, and sends nothing; once keys arrived it works again.
    #[test]
    fn try_again_while_held_says_what_adds_beebeeb_and_sends_nothing() {
        let state = AppState::default();
        let (handle, mut rx) = FinderSetupHandle::for_test(FinderSetupView::initial(LaunchLocation::Applications));
        handle.set_held_for_test(true);
        let _ = state.finder_setup.set(handle.clone());
        assert_eq!(
            finder_setup_retry_impl(&state),
            Err("Beebeeb adds itself to Finder when you’re signed in and the vault is unlocked.".to_string())
        );
        assert!(rx.try_recv().is_err(), "nothing is sent while held");
        handle.set_held_for_test(false);
        finder_setup_retry_impl(&state).expect("sent once keys are here");
        assert!(matches!(rx.try_recv(), Ok(Event::Trigger(Trigger::TryAgain))));
    }

    #[test]
    fn the_launch_trigger_follows_the_startup_restore() {
        // §5.3 (1): the launch check runs once the restore finished, so it sees the restored keys.
        let source = include_str!("lib.rs").replace("\r\n", "\n");
        let restore = source
            .find("restore_session_on_startup(&h).await;\n")
            .expect("the restore task");
        let launch = source[restore..]
            .find("finder_setup::core::Trigger::Launch")
            .expect("a launch trigger after it");
        assert!(
            launch < 400,
            "the launch trigger sits right after the restore, in the same task"
        );
        let driver = source
            .find("finder_setup::driver::spawn(")
            .expect("setup spawns the reconciler");
        assert!(
            driver < restore,
            "the reconciler exists before the restore task can trigger it"
        );
    }

    // ---- lead ruling T7-4 (rulings file item 3): a trigger that cannot be delivered is not silent ----

    #[test]
    fn try_again_without_a_reconciler_or_with_a_stopped_one_is_an_error_the_frontend_can_toast() {
        // No reconciler at all (Windows, Linux).
        let none = AppState::default();
        assert_eq!(
            finder_setup_retry_impl(&none),
            Err("Finder setup is only available on macOS.".to_string())
        );
        // One that stopped (it panicked): the receiving end is gone, `trigger` says so, and the
        // command turns that into the `Err` the frontend shows as "couldn't retry".
        let state = AppState::default();
        let (handle, rx) = FinderSetupHandle::for_test(FinderSetupView::initial(LaunchLocation::Applications));
        drop(rx);
        let _ = state.finder_setup.set(handle);
        let error = finder_setup_retry_impl(&state).unwrap_err();
        assert_eq!(
            error,
            "Beebeeb’s Finder setup isn’t running. Quit and reopen Beebeeb to start it again."
        );
        assert!(
            !error.contains('/') && !error.contains("NS"),
            "no path and no OS text: {error}"
        );
    }

    #[test]
    fn notify_finder_reaches_a_running_reconciler_and_survives_a_stopped_or_a_missing_one() {
        // Missing (Windows, Linux, tests): a no-op.
        notify_finder(&AppState::default(), Trigger::KeysArrived);
        // Running: exactly the trigger asked for.
        let state = AppState::default();
        let (handle, mut rx) = FinderSetupHandle::for_test(FinderSetupView::initial(LaunchLocation::Applications));
        let _ = state.finder_setup.set(handle);
        notify_finder(&state, Trigger::KeysArrived);
        assert!(matches!(rx.try_recv(), Ok(Event::Trigger(Trigger::KeysArrived))));
        assert!(rx.try_recv().is_err());
        // Stopped: logged, never a panic and never a lost `#[must_use]`.
        let stopped = AppState::default();
        let (handle, rx) = FinderSetupHandle::for_test(FinderSetupView::initial(LaunchLocation::Applications));
        drop(rx);
        let _ = stopped.finder_setup.set(handle);
        notify_finder(&stopped, Trigger::KeysArrived);
        let production = production_source();
        let body = body_between(&production, "fn notify_finder(", "\n}\n");
        assert!(
            body.contains("tracing::warn!"),
            "a trigger that could not be delivered is logged:\n{body}"
        );
        assert!(
            !body.contains("let _ ="),
            "the Result of `trigger` is handled, not discarded:\n{body}"
        );
    }

    #[test]
    fn copy_details_is_the_public_driver_function_with_the_view_and_the_lifecycle_tail() {
        let state = AppState::default();
        assert_eq!(
            finder_setup_copy_details_impl(&state),
            Err("Finder setup is only available on macOS.".to_string())
        );
        let view = FinderSetupView {
            setup: crate::surfaces::phase::FinderSetup::Failed,
            reason: Some(crate::surfaces::phase::FinderFailureReason::FolderTaken),
            last_failure: Some(finder_setup::error::FailureRecord {
                reason: crate::surfaces::phase::FinderFailureReason::FolderTaken,
                domain: "NSCocoaErrorDomain".into(),
                code: 516,
                at: 1_791_291_909,
            }),
            ..FinderSetupView::initial(LaunchLocation::Applications)
        };
        let _ = state.finder_setup.set(FinderSetupHandle::fixed(view));
        let text = finder_setup_copy_details_impl(&state).expect("details");
        for line in [
            "Finder setup: failed",
            "Reason: folder_taken",
            "Error: NSCocoaErrorDomain 516",
            "Launched from: applications",
        ] {
            assert!(text.lines().any(|l| l == line), "{line:?} missing:\n{text}");
        }
        assert!(text.starts_with("Beebeeb "), "{text}");
        assert!(
            text.contains("Lifecycle log (last 0 lines):"),
            "no test initialises the real log, so its tail is empty:\n{text}"
        );
        // The command goes through the one public `copy_details`, which reads the tail itself.
        let production = production_source();
        let body = body_between(&production, "fn finder_setup_copy_details_impl(", "\n}\n");
        assert!(body.contains("finder_setup::driver::copy_details("), "{body}");
        assert!(
            !production.contains("copy_details_text("),
            "the private formatter is not called from lib.rs"
        );
    }

    // ---- lead ruling T14-c3: the contract with the built TypeScript ----

    #[test]
    fn the_frontends_five_commands_are_registered_and_so_are_the_four_windows_and_linux_ones() {
        let production = production_source();
        let run = body_between(&production, "pub fn run() {", "\n}\n");
        let handler = &run[run.find("tauri::generate_handler![").expect("the handler list")..];
        let handler = &handler[..handler.find("])").expect("the handler list ends")];
        for command in [
            // Exactly what `src/finderSetup.ts` invokes (read from the Lane T worktree, 2026-10-07).
            "finder_setup_state",
            "finder_setup_retry",
            "finder_setup_copy_details",
            "finder_setup_show_app",
            "open_login_items_and_extensions_settings",
            // Windows and Linux keep these registered and working; a Mac registers them too, and
            // they refuse there (spec §4, §9).
            "install_finder_location",
            "continue_without_finder_location",
            "finder_location_state",
            "finder_domain_user_enabled",
        ] {
            let listed = handler.lines().filter(|l| l.trim() == format!("{command},")).count();
            assert_eq!(listed, 1, "{command} is registered exactly once");
        }
    }

    // ---- lead ruling T17-⚠1 and T6-⚠2: a Mac never installs through the old path ----

    /// Run on synthetic text so a bug in the helper is a red test, not a census that sees nothing.
    #[test]
    fn the_macos_census_helper_drops_exactly_what_a_mac_does_not_compile() {
        let synthetic = "fn kept() {\n    finder_location_state();\n}\n\
            #[cfg(not(target_os = \"macos\"))]\nfn gone() {\n    finder_location_state();\n}\n\
            #[cfg(test)]\nmod tests {\n    fn t() {\n        finder_location_state();\n    }\n}\n\
            #[cfg(target_os = \"windows\")]\n#[tauri::command]\nasync fn windows_only() {\n    finder_location_state();\n}\n\
            #[cfg(not(target_os = \"macos\"))]\nconst GONE: &str = \"a \\\n    b\";\n\
            #[cfg(target_os = \"macos\")]\nfn mac_only() {\n    finder_location_state();\n}\n";
        let mac = macos_compiled(synthetic);
        assert_eq!(
            call_sites(&mac, "finder_location_state").len(),
            2,
            "kept() and mac_only():\n{mac}"
        );
        for dropped in ["fn gone", "mod tests", "windows_only", "GONE"] {
            assert!(!mac.contains(dropped), "{dropped} is not compiled on a Mac:\n{mac}");
        }
        for kept in ["fn kept", "fn mac_only"] {
            assert!(mac.contains(kept), "{kept} is compiled on a Mac:\n{mac}");
        }

        // The same inside a function body: statements, blocks and arguments that a cfg removes on
        // a Mac go, and what a Mac compiles stays (indentation is the only thing that differs).
        let inside = "fn run() {\n\
            \x20   before();\n\
            \x20   #[cfg(not(target_os = \"macos\"))]\n\
            \x20   persist(\n\
            \x20       &mut cfg,\n\
            \x20   )?;\n\
            \x20   #[cfg(target_os = \"windows\")]\n\
            \x20   {\n\
            \x20       if let Some(root) = x {\n\
            \x20           windows_only_call();\n\
            \x20       } else {\n\
            \x20           windows_only_else();\n\
            \x20       }\n\
            \x20   }\n\
            \x20   call(\n\
            \x20       a,\n\
            \x20       #[cfg(target_os = \"windows\")]\n\
            \x20       &_transition,\n\
            \x20   )\n\
            \x20   .await?;\n\
            \x20   #[cfg(target_os = \"macos\")]\n\
            \x20   mac_call();\n\
            \x20   after();\n\
            }\n";
        let body = macos_compiled(inside);
        for gone in ["persist(", "windows_only_call", "windows_only_else", "_transition"] {
            assert!(!body.contains(gone), "{gone} is not compiled on a Mac:\n{body}");
        }
        for stays in ["before();", "call(", ".await?;", "mac_call();", "after();", "a,"] {
            assert!(body.contains(stays), "{stays} is compiled on a Mac:\n{body}");
        }
        // A parameter attribute that shares its line with the parameter is not an item attribute.
        let param = "async fn f(\n    app: A,\n    #[cfg(target_os = \"windows\")] _transition: &T,\n) -> R {\n    body();\n}\n";
        assert_eq!(macos_compiled(param), param, "nothing is dropped");

        // A name that merely ends with another one is not a call of it.
        assert!(call_sites("    my_finder_location_state();\n", "finder_location_state").is_empty());
        assert!(call_sites("    // finder_location_state();\n", "finder_location_state").is_empty());
        assert!(
            call_sites("fn finder_location_state() {\n", "finder_location_state").is_empty(),
            "a definition is not a call"
        );
    }

    #[test]
    fn no_macos_compiled_path_calls_the_install_era_commands_and_the_macos_variants_exist() {
        let mac = macos_compiled(&include_str!("lib.rs").replace("\r\n", "\n"));
        for name in [
            "install_finder_location",
            "continue_without_finder_location",
            "finder_location_state",
            "finder_domain_user_enabled",
        ] {
            let calls = call_sites(&mac, name);
            assert!(calls.is_empty(), "{name} is called by code a Mac compiles: {calls:?}");
            assert!(
                mac.contains(&format!("fn {name}(")),
                "{name} is still defined on macOS (the command table is the same everywhere)"
            );
        }
        // The three that Windows and Linux really run exist twice: the real body (not macOS) and
        // the refusal (macOS).
        let production = production_source();
        for name in [
            "install_finder_location",
            "continue_without_finder_location",
            "finder_location_state",
        ] {
            assert_eq!(
                production.matches(&format!("fn {name}(")).count(),
                2,
                "{name}: one body for Windows/Linux, one refusal for macOS"
            );
        }
    }

    #[test]
    fn nothing_a_mac_compiles_writes_the_old_finder_install_keys() {
        let mac = macos_compiled(&include_str!("lib.rs").replace("\r\n", "\n"));
        for key in [
            "finder_install_status",
            "finder_install_last_error",
            "finder_install_reason_category",
            "finder_install_last_attempt_at",
        ] {
            let writes: Vec<&str> = mac
                .lines()
                .map(str::trim)
                .filter(|line| !line.starts_with("//") && line.contains(&format!("{key} = ")))
                .collect();
            assert!(writes.is_empty(), "macOS-compiled code writes {key}: {writes:?}");
        }
        for helper in [
            "persist_finder_install_result(",
            "record_finder_install_result(",
            "begin_finder_install_attempt(",
            "finder_install_failed(",
        ] {
            assert!(
                call_sites(&mac, helper.trim_end_matches('(')).is_empty(),
                "{helper} is reachable on macOS"
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_install_era_commands_refuse_on_macos() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        assert_eq!(
            runtime
                .block_on(install_finder_location(Some("/Users/sam/Secret".into())))
                .unwrap_err(),
            "On macOS Beebeeb adds itself to Finder. Use finder_setup_retry."
        );
        assert_eq!(
            runtime
                .block_on(continue_without_finder_location(Some("/Users/sam/Secret".into())))
                .unwrap_err(),
            "Finder setup is automatic on macOS"
        );
        assert_eq!(finder_location_state().unwrap_err(), "On macOS use finder_setup_state.");
    }

    // ---- lead rulings T5-⚠a and T5-⚠e: the lifecycle log is initialised once, from setup() ----

    #[test]
    fn setup_initialises_the_lifecycle_log_once_with_a_names_function_that_takes_no_app_lock() {
        let production = production_source();
        let init = concat!("lifecycle_log::", "init(");
        assert_eq!(
            production.matches(init).count(),
            1,
            "exactly one call of the real init, outside every test"
        );
        let setup = &production[production.find(".setup(|app| {").expect("setup()")..];
        let at = setup.find(init).expect("init is called from setup()");
        let attribute = &setup[setup[..at].rfind("#[cfg(").expect("a cfg attribute above the init")..];
        assert!(
            attribute.starts_with("#[cfg(target_os = \"macos\")]"),
            "and on macOS only, in the block the nearest cfg guards:\n{attribute}"
        );
        assert!(
            setup[at..].starts_with(&format!("{init}dir, lifecycle_known_names)")),
            "with its own names function"
        );
        let names = body_between(&production, "fn lifecycle_known_names()", "\n}\n");
        for forbidden in [
            ".lock(",
            "lifecycle_log::event",
            "lifecycle_log::tail",
            ".unwrap()",
            ".expect(",
            "panic!",
            "AppState",
            "state::<",
            "block_on",
        ] {
            assert!(
                !names.contains(forbidden),
                "the names function must not use {forbidden}:\n{names}"
            );
        }
        assert!(
            names.contains("state_db_from_app_local_state_dir()"),
            "it opens its own StateDb, as the support bundle does"
        );
    }

    #[test]
    fn the_names_function_never_panics_without_a_state_db_and_never_touches_the_real_log() {
        // A test process has no app-local state dir: the support-bundle pattern's `Err` arm.
        let _ = lifecycle_known_names();
        assert!(
            lifecycle_log::tail(lifecycle_log::TAIL_LINES).is_empty(),
            "no test initialises the real log"
        );
    }

    // ---- lead ruling T5-b: the support bundle carries the lifecycle tail ----

    #[test]
    fn the_support_bundle_carries_the_lifecycle_tail_without_changing_its_format() {
        let bundle = super::with_lifecycle_tail(
            serde_json::json!({ "diagnostics_format": 2 }),
            vec!["2026-10-06T13:05:09Z signed_in".to_string()],
        );
        assert_eq!(
            bundle["lifecycle_log"],
            serde_json::json!(["2026-10-06T13:05:09Z signed_in"])
        );
        assert_eq!(
            bundle["diagnostics_format"], 2,
            "additive key: the format number is unchanged"
        );
        let production = production_source();
        let command = body_between(&production, "fn export_diagnostics()", "\n}\n");
        assert!(command.contains("with_lifecycle_tail("), "{command}");
        assert!(
            command.contains("lifecycle_log::tail(lifecycle_log::TAIL_LINES)"),
            "the last 200 lines: {command}"
        );
    }

    // ---- Task 8 fix round 1: a check's engine start and stop are cut off only while they wait ----

    /// A future dropped after it spawned an engine loses the `started` answer, and one dropped after it
    /// took the engine out of its slot detaches the engine task: it keeps running untracked, holding
    /// the session token and master key, while sign-out finds an empty slot and reports done. So the
    /// limit covers the wait for the slot and nothing after it.
    #[test]
    fn a_check_start_and_stop_are_cut_off_only_while_they_wait_for_the_engine_slot() {
        let production = production_source();
        let start = body_between(&production, concat!("async fn ", "start_check_engine("), "\n}\n");
        let lock = start
            .find("lock_engine_slot(")
            .expect("the slot is taken under a limit");
        let spawn = start
            .find("spawn_bound_engine(")
            .expect("it starts through the one gate");
        let ready = start
            .find("wait_for_file_provider_ipc_ready(")
            .expect("it waits for the socket");
        assert!(
            lock < spawn && spawn < ready,
            "limit, then spawn, then the self-bounded socket wait:\n{start}"
        );
        assert_eq!(
            start.matches("within(").count(),
            0,
            "no cut-off once the engine exists:\n{start}"
        );

        let stop = body_between(&production, "async fn stop_check_engine(", "\n}\n");
        let lock = stop.find("lock_engine_slot(").expect("the slot is taken under a limit");
        let take = stop.find(".take()").expect("the engine leaves its slot");
        let abort = stop
            .find("stop_engine_in_slot(")
            .expect("and is stopped through the one helper");
        assert!(
            lock < take && take < abort,
            "limit, then take, then an uncancelled abort:\n{stop}"
        );
        assert_eq!(
            stop.matches("within(").count(),
            0,
            "no cut-off once the engine is out of its slot:\n{stop}"
        );
        assert!(
            stop.contains("stop_engine_in_slot("),
            "an unconfirmed stop closes the sign-out gate, in the one helper:\n{stop}"
        );

        let helper = body_between(&production, "async fn lock_engine_slot(", "\n}\n");
        assert!(
            helper.contains("within(") && helper.contains("acct.engine.lock().await"),
            "{helper}"
        );

        // FinishReady is cut off as a whole; that is safe only while the slot is its one await.
        let ensure = body_between(
            &production,
            concat!("async fn ", "ensure_sync_root_and_engine("),
            "\n}\n",
        );
        assert_eq!(
            ensure.matches(".await").count(),
            1,
            "a second await could be cut off after the spawn:\n{ensure}"
        );
        assert!(ensure.contains("engine.lock().await"), "{ensure}");
    }

    // ---- F7 follow-up 2: the bounded wait for the bridge gate never runs on the main thread ----

    /// "Open in Finder" and the onboarding card's `finder_domain_user_enabled` read the domain through the bridge
    /// gate, which waits up to `BRIDGE_GATE_WAIT` (3 s) for a call in flight. Tauri runs a plain
    /// `#[tauri::command]` fn on the main thread, and the native menu ran "Open in Finder" inline there, so that
    /// wait froze every window. Both commands are `async` and hand their work to the blocking pool, and the menu
    /// action is spawned the same way. A source pin, like `bundle_commands_run_off_the_main_thread`: the thread a
    /// command runs on is not observable in a unit test.
    #[test]
    fn the_finder_reads_a_person_waits_on_never_wait_on_the_main_thread() {
        use crate::source_pin::squeeze;
        let production = production_source();
        for (command, work) in [
            ("open_finder_location", "move || open_finder_location_blocking(path)"),
            ("open_in_finder", "move || open_in_finder_blocking(item_id, path)"),
            (
                "open_login_items_and_extensions_settings",
                "open_login_items_and_extensions_settings_blocking",
            ),
            ("finder_domain_user_enabled", "file_provider_domain_user_enabled"),
        ] {
            assert_eq!(
                production.matches(&format!("fn {command}(")).count(),
                1,
                "{command} has one definition on every platform"
            );
            assert!(
                production.contains(&format!("#[tauri::command]\nasync fn {command}(")),
                "{command} is an async command: a plain #[tauri::command] fn runs on the main thread"
            );
            let body = body_between(&production, &format!("async fn {command}("), "\n}\n");
            let squeezed = squeeze(body);
            let pool = squeezed.find("on_the_blocking_pool(").unwrap_or_else(|| {
                panic!("{command} runs its work on the blocking pool, not inline on a runtime worker:\n{body}")
            });
            assert!(
                squeezed[pool..].contains(&squeeze(&format!(",{work}).await"))),
                "{command} hands exactly its blocking work to the pool:\n{body}"
            );
        }

        let open_folder = body_between(
            &production,
            "DesktopMenuAction::OpenFolder =>",
            "DesktopMenuAction::UploadFiles",
        );
        assert!(
            !open_folder.contains("log_menu_result("),
            "the menu's \"Open in Finder\" does not run inline on the main thread:\n{open_folder}"
        );
        assert!(
            squeeze(open_folder).contains(&squeeze("spawn_menu_task(spec.id, async move {"))
                && squeeze(open_folder).contains(&squeeze(
                    "on_the_blocking_pool(OPEN_FOLDER_FAILED, open_current_finder_location).await"
                )),
            "the menu's \"Open in Finder\" is spawned onto the blocking pool:\n{open_folder}"
        );
        assert_eq!(
            production.matches("open_current_finder_location").count(),
            2,
            "its definition and the spawned menu action, and no inline caller"
        );
    }

    /// The helper the two commands and the menu action use: the work runs on a blocking-pool thread, never the
    /// caller's, and its answer comes back unchanged. A call that panics answers the caller's fixed sentence; the
    /// panic's own text (which can carry an OS message) goes to the log only (lead ruling T1-4).
    #[tokio::test]
    async fn a_blocking_call_runs_on_the_blocking_pool_and_a_panic_answers_a_fixed_sentence() {
        let caller = std::thread::current().id();
        let ran_on = on_the_blocking_pool("unused", || Ok(std::thread::current().id()))
            .await
            .expect("the call answers");
        assert_ne!(ran_on, caller, "the work does not run on the caller's thread");

        assert_eq!(
            on_the_blocking_pool("unused", || Err::<(), _>("the call's own error".to_string())).await,
            Err("the call's own error".to_string()),
            "the call's error comes back unchanged"
        );

        let panicked = on_the_blocking_pool("Could not do the thing.", || -> Result<(), String> {
            panic!("an OS message that must not reach a person")
        })
        .await;
        assert_eq!(panicked, Err("Could not do the thing.".to_string()));
    }

    // ---- helpers ----

    /// The text from `start` to the first `end` after it.
    pub(super) fn body_between<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
        let from = source.find(start).unwrap_or_else(|| panic!("{start} exists"));
        let to = from
            + source[from..]
                .find(end)
                .unwrap_or_else(|| panic!("{start} ends with {end:?}"));
        &source[from..to]
    }

    /// The three files `lib.rs` already declared as `#[cfg(test)] mod x;` before the split
    /// (5e6a888): the scanners read them then, so `test_module_files` leaves them in.
    const DECLARED_AS_FILES_BEFORE_THE_SPLIT: [&str; 3] = ["test_sandbox.rs", "source_pin.rs", "extension_log_pin.rs"];

    /// Spec §4.8 (LD-0057): the files of the test modules the split moved out of `lib.rs`, the ones
    /// it declares as `#[cfg(test)] mod x;` (or `#[cfg(all(test, …))]`), named from `src/` the way
    /// the scanners name a file. Inline, these modules were `#[cfg(test)]` items of `lib.rs` that a
    /// scanner left out; as files of their own they are left out the same way.
    pub(super) fn test_module_files(src: &std::path::Path) -> std::collections::HashSet<String> {
        let lib = std::fs::read_to_string(src.join("lib.rs")).expect("read lib.rs").replace("\r\n", "\n");
        let lines: Vec<&str> = lib.lines().collect();
        let mut files = std::collections::HashSet::new();
        for (at, line) in lines.iter().enumerate() {
            let declaration = ["pub(crate) ", "pub "].iter().find_map(|vis| line.strip_prefix(vis)).unwrap_or(line);
            let Some(name) = declaration.strip_prefix("mod ").and_then(|rest| rest.strip_suffix(';')) else {
                continue;
            };
            if !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
                continue;
            }
            let (mut test_only, mut path) = (false, None);
            let above = lines[..at].iter().rev().take_while(|l| l.starts_with("#[") || l.starts_with("///"));
            for attribute in above {
                test_only |= attribute.starts_with("#[cfg(test)]") || attribute.starts_with("#[cfg(all(test");
                if let Some(value) = attribute.strip_prefix("#[path = \"").and_then(|v| v.strip_suffix("\"]")) {
                    path = Some(value.to_string());
                }
            }
            if !test_only {
                continue;
            }
            let candidates = match path {
                Some(path) => vec![path],
                None => vec![format!("{name}.rs"), format!("{name}/mod.rs")],
            };
            files.extend(candidates.into_iter().filter(|candidate| {
                src.join(candidate).is_file() && !DECLARED_AS_FILES_BEFORE_THE_SPLIT.contains(&candidate.as_str())
            }));
        }
        files
    }

    /// `lib` with the moved test modules' files after it: what `include_str!("lib.rs")` read
    /// before the split, when those modules were inline (spec §4.8).
    pub(super) fn with_test_modules(lib: &str) -> String {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files: Vec<String> = test_module_files(&src).into_iter().collect();
        files.sort();
        files.iter().fold(lib.to_string(), |all, file| {
            all + "\n" + &std::fs::read_to_string(src.join(file)).expect("read a test module").replace("\r\n", "\n")
        })
    }

    /// `source` without the items, blocks and statements whose `cfg` attribute stands alone on its
    /// line and starts with one of `attrs` (at any indentation). What an attribute covers ends at
    /// the first line that ends with `;`, or that opens and closes its braces on one line (`fn f() {}`), or, when a
    /// line ending with `{` comes first, at the `}`
    /// that closes it at the attribute's own indentation (`} else {` continues it). An attribute
    /// on an argument or field (the next line ends with `,`) covers that one line.
    pub(super) fn without_items(source: &str, attrs: &[&str]) -> String {
        let lines: Vec<&str> = source.lines().collect();
        let mut kept = String::new();
        let mut i = 0;
        while i < lines.len() {
            let trimmed = lines[i].trim_start();
            if attrs
                .iter()
                .any(|attr| trimmed.starts_with(attr) && trimmed.trim_end().ends_with(']'))
            {
                let indent = &lines[i][..lines[i].len() - trimmed.len()];
                let mut j = i + 1;
                while j < lines.len()
                    && (lines[j].trim_start().starts_with("#[") || lines[j].trim_start().starts_with("///"))
                {
                    j += 1;
                }
                if lines.get(j).is_some_and(|line| line.trim_end().ends_with(',')) {
                    i = j + 1;
                    continue;
                }
                let mut k = j;
                while k < lines.len() {
                    let line = lines[k].trim_end();
                    if line.ends_with(';') {
                        break;
                    }
                    // A one-line item (`fn f() {}`): its braces close on the line it starts on (fix round 1, m1).
                    if line.ends_with('}')
                        && line.contains('{')
                        && line.matches('{').count() == line.matches('}').count()
                    {
                        break;
                    }
                    if line.ends_with('{') {
                        let close = format!("{indent}}}");
                        while k < lines.len()
                            && lines[k].trim_end() != close
                            && lines[k].trim_end() != format!("{close};")
                        {
                            k += 1;
                        }
                        break;
                    }
                    k += 1;
                }
                i = k + 1;
                continue;
            }
            kept.push_str(lines[i]);
            kept.push('\n');
            i += 1;
        }
        kept
    }

    /// Fix round 1 (m1): an item covered by one of the attributes can be ONE line (`fn f() {}`). It used to be read as
    /// unfinished, because the line ends in `}` and not in `{` or `;`, and the next item was swallowed with it: the
    /// open's own wrapper was invisible to the spawn pin. The covered item ends at its own line.
    #[test]
    fn test_1885_without_items_drops_a_one_line_item_and_keeps_the_next_one() {
        let source = "\
fn before() {}
#[cfg(not(target_os = \"macos\"))]
pub(crate) fn dropped_one_liner(_context: &str) {}

#[cfg(target_os = \"macos\")]
fn survivor_after_the_one_liner() -> Result<(), String> {
    Ok(())
}
#[cfg(not(target_os = \"macos\"))]
fn two_lines() -> u8
{ 1 }
fn after() {}
";
        let kept = without_items(source, &["#[cfg(not(target_os = \"macos\"))]"]);
        assert!(
            !kept.contains("dropped_one_liner"),
            "the covered one-line item is dropped:\n{kept}"
        );
        assert!(
            kept.contains("survivor_after_the_one_liner"),
            "the item after a one-line item is kept:\n{kept}"
        );
        assert!(kept.contains("fn before()") && kept.contains("fn after()"), "{kept}");
        assert!(
            !kept.contains("two_lines"),
            "a covered item that spans lines is still dropped whole:\n{kept}"
        );
        // An item with a body on following lines, and a `;` item, behave as before.
        let kept = without_items(
            "#[cfg(test)]\nmod t {\n    fn x() {}\n}\nfn keep() {}\n#[cfg(test)]\nuse a::b;\nfn keep2() {}\n",
            &["#[cfg(test)]"],
        );
        assert!(
            kept.contains("fn keep()") && kept.contains("fn keep2()") && !kept.contains("mod t"),
            "{kept}"
        );
    }

    /// `lib.rs` without its test modules, for the tests that read production code.
    pub(super) fn production_source() -> String {
        without_items(
            &include_str!("lib.rs").replace("\r\n", "\n"),
            &["#[cfg(test)]", "#[cfg(all(test"],
        )
    }

    /// `source` as the macOS build compiles it: without the test modules and without what is
    /// Windows, Linux or "not macOS" only, so a census of call sites sees only code a Mac can run.
    pub(super) fn macos_compiled(source: &str) -> String {
        without_items(
            source,
            &[
                "#[cfg(test)]",
                "#[cfg(all(test",
                "#[cfg(any(target_os = \"windows\", test))]",
                "#[cfg(not(target_os = \"macos\"))]",
                "#[cfg(target_os = \"windows\")]",
                "#[cfg(target_os = \"linux\")]",
                "#[cfg(not(unix))]",
            ],
        )
    }

    /// The lines that call `name(`, not defining it and not comments. A longer name that ends
    /// with `name` is not a call of it.
    fn call_sites(source: &str, name: &str) -> Vec<String> {
        let needle = format!("{name}(");
        source
            .lines()
            .map(str::trim)
            .filter(|line| !line.starts_with("//") && !line.contains(&format!("fn {needle}")))
            .filter(|line| {
                line.match_indices(&needle).any(|(at, _)| {
                    !line[..at]
                        .chars()
                        .next_back()
                        .is_some_and(|c| c.is_alphanumeric() || c == '_')
                })
            })
            .map(str::to_string)
            .collect()
    }
