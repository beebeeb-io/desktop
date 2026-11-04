    use super::MacosIntegrationResetResult;

    fn source() -> String {
        include_str!("lib.rs").replace("\r\n", "\n")
    }

    /// The text of the item starting at `start`, up to the next line that is exactly `}`.
    fn item(source: &str, start: &str) -> String {
        item_in(source, start, "\n}\n")
    }

    /// The text from `start` up to the first `end` after it.
    fn item_in(source: &str, start: &str, end: &str) -> String {
        let at = source.find(start).unwrap_or_else(|| panic!("{start} exists"));
        let to = source[at..].find(end).expect("item ends");
        source[at..at + to].to_string()
    }

    /// Layout-free text (`source_pin::squeeze`): rustfmt may wrap any of these calls.
    fn squash_ws(text: &str) -> String {
        crate::source_pin::squeeze(text)
    }

    /// Rebase re-review I2: once a kept folder is saved for the Settings › Sync row, every window is told with
    /// `kept-folder-changed` (no payload), so a Sync tab that is open reads the folder again. The event comes after
    /// the save, so that read finds it, and before the alert. Nothing kept: no save, no event, no alert. The emit and
    /// the listener are real (a mock app); the save and the alert are recorded.
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn rebase_i2_a_surfaced_folder_tells_the_windows_after_it_is_saved() {
        use std::sync::{Arc, Mutex};
        use tauri::Listener;
        const FOLDER: &str = "/Users/someone/Library/CloudStorage/Beebeeb-Beebeeb (10-10-2026 10:50)";
        assert_eq!(super::KEPT_FOLDER_CHANGED_EVENT, "kept-folder-changed");
        let app = tauri::test::mock_app();
        let steps: Arc<Mutex<Vec<String>>> = Arc::default();
        let heard = steps.clone();
        app.handle().listen(super::KEPT_FOLDER_CHANGED_EVENT, move |event| {
            heard.lock().unwrap().push(format!("told {}", event.payload()));
        });

        super::surface_kept_folder_with(
            Some(FOLDER),
            |location| {
                super::save_kept_folder_for_row_with(
                    location,
                    |location| steps.lock().unwrap().push(format!("saved {location}")),
                    || super::emit_kept_folder_changed(app.handle()),
                )
            },
            |location| steps.lock().unwrap().push(format!("alert {location}")),
        );
        assert_eq!(
            *steps.lock().unwrap(),
            vec![
                format!("saved {FOLDER}"),
                "told null".to_string(),
                format!("alert {FOLDER}")
            ],
            "saved, then the windows are told (no path in the event), then the alert"
        );

        steps.lock().unwrap().clear();
        super::surface_kept_folder_with(
            None,
            |location| {
                super::save_kept_folder_for_row_with(
                    location,
                    |location| steps.lock().unwrap().push(format!("saved {location}")),
                    || super::emit_kept_folder_changed(app.handle()),
                )
            },
            |location| steps.lock().unwrap().push(format!("alert {location}")),
        );
        assert!(steps.lock().unwrap().is_empty(), "nothing kept: nothing at all");
    }

    /// Device RB3-F1 (2026-10-10, signed QA build): Settings › Sync was open on the kept-folder row; a Repair started
    /// from the compact window's Finder page kept files in a newer folder and saved it, and the open tab went on
    /// showing the older one until a tab switch. Every save of the row's folder tells the windows, Repair's too: the
    /// event comes once, after the save (so the tab's read finds the folder), with no path in it. The emit and the
    /// listener are real (a mock app); the config is this test's own directory, read the way the tab reads it.
    #[cfg(not(target_os = "windows"))]
    #[tokio::test]
    async fn rb3_f1_a_repair_that_kept_a_folder_tells_the_windows_once_after_the_row_is_saved() {
        use std::sync::{Arc, Mutex};
        use tauri::Listener;
        const FOLDER: &str = "/Users/someone/Library/CloudStorage/Beebeeb-Beebeeb (10-10-2026 17:23)";
        let dir = tempfile::tempdir().expect("temp dir");
        let app = tauri::test::mock_app();
        // Each entry: the payload, and what the Sync tab's own read (`kept_unsynced_folder`) finds when it is told.
        type Heard = Arc<Mutex<Vec<(String, Option<String>)>>>;
        let heard: Heard = Arc::default();
        let into = heard.clone();
        app.handle().listen(super::KEPT_FOLDER_CHANGED_EVENT, move |event| {
            let row = super::kept_unsynced_folder().expect("the tab reads the config");
            into.lock().unwrap().push((event.payload().to_string(), row));
        });

        crate::config::CONFIG_DIR_OVERRIDE
            .scope(dir.path().to_path_buf(), async {
                assert_eq!(super::kept_unsynced_folder(), Ok(None), "no row before the Repair");
                super::save_kept_folder_for_row(app.handle(), FOLDER);
                assert_eq!(
                    super::kept_unsynced_folder(),
                    Ok(Some(FOLDER.to_string())),
                    "the Repair's folder is saved for the row"
                );
            })
            .await;
        assert_eq!(
            *heard.lock().unwrap(),
            vec![("null".to_string(), Some(FOLDER.to_string()))],
            "told exactly once, with no path, and the tab's read already finds the folder"
        );
    }

    /// RB3-F1: the row's folder is saved in ONE place, and that place tells the windows, so a future save path cannot
    /// forget the event. Source pins (the behaviour is `rb3_f1_a_repair_that_kept_a_folder_tells_the_windows_...` and
    /// `rebase_i2_a_surfaced_folder_...`): the one function saves and then tells; sign-out, the sweep and the
    /// reconciler (`surface_kept_folder`) and macOS Repair both go through it; and a census of every `.rs` file under
    /// `src` finds no other writer of the row. `remember_kept_folder_held`, which writes the folder onto the file as it is
    /// now from a flow that holds its own copy of the config (task 1889), is the one other way in, kept for Windows and
    /// Linux (no File Provider there, so nothing is kept, and the row is MacSettings' alone): its three sites are
    /// counted by name, so a fourth, or a Mac one, fails here until it tells the windows.
    #[test]
    fn rb3_f1_every_save_of_the_rows_folder_goes_through_the_one_function_that_tells_the_windows() {
        let full = source();
        let source = full[..full.find("\nmod finder_removal_wiring_tests;").expect("this module")].to_string();
        let code_only = |text: &str| {
            text.lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let helper = squash_ws(&code_only(&item(&source, "fn save_kept_folder_for_row<")));
        assert!(
            helper.contains(&squash_ws(
                "save_kept_folder_for_row_with(location, remember_kept_folder, || emit_kept_folder_changed(app));"
            )),
            "the one function saves with `remember_kept_folder`, then tells the windows:\n{helper}"
        );
        let with = code_only(&item(&source, "fn save_kept_folder_for_row_with("));
        let saved = with.find("save(location);").expect("it saves the folder");
        let told = with.find("changed();").expect("it tells the windows");
        assert!(saved < told, "saved first, told after:\n{with}");

        let surface = squash_ws(&code_only(&item(&source, "fn surface_kept_folder(")));
        assert!(
            surface.contains(&squash_ws(
                "surface_kept_folder_with(
                    preserved_location,
                    |location| save_kept_folder_for_row(app, location),
                    |location| show_preserved_files_alert(app, Some(location)),
                );"
            )),
            "sign-out, the sweep and the reconciler save through the one function:\n{surface}"
        );
        let repair = code_only(&item(&source, "async fn reset_macos_integration("));
        assert!(
            squash_ws(&repair).contains(&squash_ws(
                "#[cfg(target_os = \"macos\")]
                save_kept_folder_for_row(&app, location);"
            )),
            "macOS Repair saves the row through the one function:\n{repair}"
        );
        // Task 1889: Windows and Linux write the folder through `remember_kept_folder_held`, which is not the Mac's route.
        let without_held = repair.replace("remember_kept_folder_held", "");
        assert!(
            !without_held.contains("remember_kept_folder"),
            "Repair never saves the row on its own:\n{repair}"
        );

        // The census. `src` is read from disk; a file's test module starts at its first column-0 `#[cfg(test)]` before a `mod`, and
        // `lib.rs` is read up to this module (the test modules before it hold none of these names).
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        let mut stack = vec![src.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display())) {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                    let name = path.strip_prefix(&src).unwrap().to_string_lossy().replace('\\', "/");
                    if name == "tests.rs" || name.ends_with("/tests.rs") || name.ends_with("_tests.rs") || name.split('/').any(|part| part == "tests") { continue; }
                    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {name}: {e}"));
                    let text = text.replace("\r\n", "\n");
                    let production = if name == "lib.rs" {
                        source.clone()
                    } else {
                        let end = text.find("\n#[cfg(test)]\nmod ").unwrap_or(text.len());
                        text[..end].to_string()
                    };
                    files.push((name, code_only(&production)));
                }
            }
        }
        files.sort();
        assert!(
            files.len() >= 20 && files.iter().any(|(name, _)| name == "finder_setup/macos_ports.rs"),
            "a census that read fewer than 20 files, or not the reconciler's ports, scanned the wrong tree: {:?}",
            files.iter().map(|(name, _)| name).collect::<Vec<_>>()
        );
        // A name as a word: `remember_kept_folder_at` is another name.
        let words = |text: &str, word: &str| {
            text.match_indices(word)
                .filter(|(at, _)| {
                    let before = text[..*at].chars().next_back();
                    let after = text[at + word.len()..].chars().next();
                    let part_of_a_name = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
                    !part_of_a_name(before) && !part_of_a_name(after)
                })
                .count()
        };
        let count_in = |file: &str, word: &str| {
            let (_, text) = files
                .iter()
                .find(|(name, _)| name == file)
                .unwrap_or_else(|| panic!("{file}"));
            words(text, word)
        };

        // `remember_kept_folder`: defined in `finder_removal.rs`, wrapped once in `lib.rs`, used once, by the one function.
        assert_eq!(
            count_in("finder_removal.rs", "remember_kept_folder"),
            1,
            "its definition"
        );
        let mut lib = source.clone();
        for own in ["fn remember_kept_folder(", "fn save_kept_folder_for_row<"] {
            lib = lib.replace(&item(&source, own), "");
        }
        assert_eq!(
            words(&code_only(&lib), "remember_kept_folder"),
            0,
            "in lib.rs only the wrapper and the one function name `remember_kept_folder`"
        );
        for (name, text) in &files {
            if name != "lib.rs" && name != "finder_removal.rs" {
                assert_eq!(
                    words(text, "remember_kept_folder") + words(text, "save_kept_folder_for_row_with"),
                    0,
                    "{name} must save the row through `save_kept_folder_for_row`, not around it"
                );
            }
            if name != "finder_removal.rs" {
                assert_eq!(
                    text.matches(".kept_unsynced_folder =").count(),
                    0,
                    "{name} writes the row's folder on its own; only finder_removal.rs does, for the functions above"
                );
            }
        }

        // `remember_kept_folder_held` into a config the flow holds: Windows and Linux only, three sites, by name.
        let record = "finder_removal::remember_kept_folder_held(&mut cfg";
        let install = code_only(&item(&source, "async fn install_finder_location("));
        assert_eq!(
            install.matches(record).count(),
            2,
            "Add to Finder's two cleanups:\n{install}"
        );
        assert_eq!(
            code_only(&repair).matches(record).count(),
            1,
            "Repair's, under `#[cfg(not(target_os = \"macos\"))]`:\n{repair}"
        );
        assert!(
            squash_ws(&repair).contains(&squash_ws(
                "#[cfg(not(target_os = \"macos\"))]
                finder_removal::remember_kept_folder_held(&mut cfg, location);"
            )),
            "Repair's record is Windows and Linux only; on a Mac the row is saved through the one function:\n{repair}"
        );
        assert_eq!(
            words(&code_only(&source), "remember_kept_folder_held"),
            3,
            "a new site that records the row's folder must tell the windows after its own save, or be named here"
        );
        for (name, text) in &files {
            if name != "lib.rs" && name != "finder_removal.rs" {
                assert_eq!(
                    words(text, "remember_kept_folder_held"),
                    0,
                    "{name} records the row's folder around the pin"
                );
            }
        }
    }

    #[test]
    fn test_1882_the_repair_result_carries_the_kept_folder_to_the_frontend() {
        let result = MacosIntegrationResetResult {
            removed_file_provider_domain: true,
            preserved_location: Some("/Users/someone/Kept".to_string()),
            disabled_autostart: false,
            removed_socket: false,
            removed_cache_files: 0,
            skipped_cache_files: 0,
            pending_operations_preserved: 0,
            sync_root_preserved: None,
            engine_stop_unconfirmed: false,
            warnings: Vec::new(),
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["preserved_location"], "/Users/someone/Kept");

        let nothing_kept = MacosIntegrationResetResult {
            preserved_location: None,
            ..result
        };
        assert!(serde_json::to_value(&nothing_kept).unwrap()["preserved_location"].is_null());
    }

    /// Device K-F1 (round 2): the alert is drawn by macOS's UserNotificationCenter process, not a
    /// Beebeeb window, so a device check needs the app's own word that it raised the alert and
    /// that the person closed it. Both are debug lines without the path.
    #[test]
    fn test_1882_the_kept_folder_alert_logs_that_it_was_shown_and_closed() {
        let full = source();
        let source = full[..full.find("\nmod finder_removal_wiring_tests;").expect("this module")].to_string();
        let alert = item(&source, "fn show_preserved_files_alert(");
        assert!(
            alert.contains("let Some((title, message)) = finder_removal::kept_folder_alert(preserved_location) else {"),
            "the alert's one decision is the tested `kept_folder_alert`"
        );
        let show = alert.find(".show(").expect("the alert is shown");
        let closed = alert
            .find("tracing::debug!(\"kept-folder alert closed\")")
            .expect("the alert's callback logs that the person closed it");
        let shown = alert
            .find("tracing::debug!(\"kept-folder alert shown\")")
            .expect("the alert logs that it was raised");
        assert!(show < closed, "\"closed\" is logged inside the alert's callback");
        assert!(closed < shown, "\"shown\" is logged after show() returns");
        assert!(
            !alert[show..].contains("location"),
            "nothing after show() may carry the folder's path into a log"
        );
    }

    /// Review I2 (lead ruling, round 2): every removal path saves the kept folder for the
    /// Settings › Sync row. Paths that hold a loaded config and save it afterwards record into
    /// THAT config, so their own save cannot overwrite the record with a stale copy.
    #[test]
    fn test_1882_r2_every_removal_path_saves_the_kept_folder_for_the_sync_row() {
        let full = source();
        let source = full[..full.find("\nmod finder_removal_wiring_tests;").expect("this module")].to_string();
        let code_only = |text: &str| {
            text.lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        // Saved, then shown: one function for the paths without a config of their own. Rebase re-review I1: its
        // two steps are passed to `surface_kept_folder_with`, so a test can run them on its own file.
        // Rebase re-review I2 and RB3-F1: the save is `save_kept_folder_for_row`, which tells the windows after it
        // (its own pin is `rb3_f1_every_save_of_the_rows_folder_...`), so an open Sync tab reads the folder again.
        let surface = squash_ws(&code_only(&item(&source, "fn surface_kept_folder(")));
        assert!(
            surface.contains(&squash_ws(
                "surface_kept_folder_with(
                    preserved_location,
                    |location| save_kept_folder_for_row(app, location),
                    |location| show_preserved_files_alert(app, Some(location)),
                );"
            )),
            "it saves the folder for the row (and tells the windows), then raises the alert:\n{surface}"
        );
        let with = code_only(&item(&source, "fn surface_kept_folder_with("));
        let saved = with.find("save_for_row(location);").expect("it saves the folder");
        let shown = with.find("alert(location);").expect("it raises the alert");
        assert!(saved < shown, "{with}");
        let remember = code_only(&item(&source, "fn remember_kept_folder("));
        // Round 5: the save is one load-change-save under the config-write lock, not a
        // `DesktopConfig::load()` + `cfg.save()` of its own.
        assert!(remember.contains("finder_removal::remember_kept_folder(location)"));
        assert!(
            !remember.contains("DesktopConfig::load()") && !remember.contains("cfg.save()"),
            "no unserialized load-modify-save:\n{remember}"
        );
        assert!(!remember.contains("location ="), "the log line never carries the path");
        let dismiss = code_only(&item(&source, "fn dismiss_kept_unsynced_folder("));
        assert!(dismiss.contains("finder_removal::dismiss_saved_kept_folder(&path)"));
        assert!(
            dismiss.contains("Result<finder_removal::DismissOutcome, String>"),
            "the command reports whether it cleared the row"
        );
        assert!(
            !dismiss.contains("DesktopConfig::load()") && !dismiss.contains("cfg.save()"),
            "no unserialized load-modify-save:\n{dismiss}"
        );

        // Sign-out (command and menu) and the sweep.
        let command = code_only(&item(&source, "async fn clear_session("));
        assert!(command.contains("surface_sign_out_folders(&app, &result);"));
        let menu = &source[source.find("DesktopMenuAction::SignOut => {").expect("menu sign-out")..];
        let menu = &menu[..menu.find("DesktopMenuAction::Quit").expect("next arm")];
        assert!(code_only(menu).contains("surface_sign_out_folders(&app, &result);"));
        assert!(code_only(&source).contains("surface_kept_folder(&alert_app, Some(location));"));

        // Repair and the add rollback record into the config they save afterwards (Windows and Linux). On macOS
        // Repair saves no config (spec 2026-10-06 §6.1), so it saves the row's record on its own, under the
        // config-write lock, without the alert: its result carries the folder to the Sync tab. RB3-F1: it saves
        // through `save_kept_folder_for_row`, which also tells the windows, so an open Sync tab follows the Repair.
        let repair = code_only(&item(&source, "async fn reset_macos_integration("));
        assert!(
            squash_ws(&repair).contains(&squash_ws(
                "if let Some(location) = preserved_location.as_deref() {\n        #[cfg(target_os = \"macos\")]\n        save_kept_folder_for_row(&app, location);"
            )),
            "macOS Repair saves the folder for the row:\n{repair}"
        );
        let recorded = repair
            .find("finder_removal::remember_kept_folder_held(&mut cfg, location);")
            .expect("Repair records the folder into its config");
        // Round 3 (re-review P1): the save is the step that can fail after the removal, so it goes
        // through `finish_repair_after_removal`, which still shows the folder before the error.
        let squashed: String = repair.chars().filter(|c| !c.is_whitespace()).collect();
        let saved = repair
            .find("finish_repair_after_removal(")
            .expect("and saves it, through the step that still shows the folder when the save fails");
        assert!(recorded < saved);
        assert!(
            squashed.contains(
                "finish_repair_after_removal(&mutcfg,preserved_location.as_deref(),|cfg|persist_finder_install_result(cfg,false,None),|location|surface_kept_folder(&app,location),)?;"
            ),
            "Repair saves with the real save, and surfaces with the real alert:\n{repair}"
        );
        assert!(
            !repair.contains("persist_finder_install_result(&mut cfg, false, None)?;"),
            "no bare `?` save is left between the removal and the result"
        );
        let install = code_only(&item(&source, "async fn install_finder_location("));
        let recorded = install
            .find("finder_removal::remember_kept_folder_held(&mut cfg, &location);")
            .expect("the rollback records the folder into its config");
        let saved = install[recorded..]
            .find("return finder_install_failed(&mut cfg, error);")
            .expect("which is saved");
        assert!(saved > 0);

        // Spec A: a reconciler removal nobody waits for (an owed removal, the account binding's Repair, a waiter
        // that gave up) is surfaced by the port like the sweep's: saved, then the alert.
        let ports = include_str!("finder_setup/macos_ports.rs").replace("\r\n", "\n");
        let kept = code_only(&item_in(
            &ports,
            "    fn kept_folder(&mut self, kept: KeptFolder) {",
            "\n    }\n",
        ));
        assert!(
            kept.contains("surface_reconciler_kept(&self.app, kept, \"finder reconciler\");"),
            "the port saves and shows it:\n{kept}"
        );
        let surface = code_only(&item(&ports, "fn surface_reconciler_kept("));
        assert!(
            surface.contains("crate::surface_kept_folder(app, location.as_deref());"),
            "through the one function that saves, then shows:\n{surface}"
        );
        // And the port's removal hands on what macOS kept, with a removal that worked and one that failed (M2).
        let answer = squash_ws(&code_only(&item(&ports, "fn removal_answer(")));
        for answer_of in [
            "(Ok(Ok(removal)), _) => OpResult::Removed(Ok(()), removal.kept),",
            "(Ok(Err(failure)), _) => OpResult::Removed(Err(failure.error), failure.kept),",
        ] {
            assert!(answer.contains(&squash_ws(answer_of)), "{answer_of}");
        }
        // Rebase re-review I1: a removal that finishes after the reconciler stopped waiting surfaces its folder
        // from the blocking thread, through the same function as the port.
        let removal = &ports[ports.find("Op::RemoveDomain => {").expect("the removal arm")..];
        let removal = squash_ws(&removal[..removal.find("fn publish(").expect("publish follows run")]);
        assert!(
            removal.contains(&squash_ws(
                "surface_reconciler_kept(&late_app, kept, \"finder reconciler, after its time limit\")"
            )),
            "the late sink:\n{removal}"
        );
        assert!(
            removal.contains(&squash_ws("removal_answer(cut, answered)")),
            "{removal}"
        );
        let late = squash_ws(&code_only(&item(&ports, "fn answering(")));
        assert!(
            late.contains(&squash_ws(
                "if let Err(unheard) = answer.send(remove()) {\n            late("
            )),
            "an answer nobody listens for any more goes to the late sink:\n{late}"
        );

        // The row's two commands are registered.
        for name in ["kept_unsynced_folder", "dismiss_kept_unsynced_folder"] {
            assert_eq!(
                source.matches(&format!("            {name},\n")).count(),
                1,
                "{name} in generate_handler!"
            );
        }
    }

    /// Reviews M1 and M2 (round 2): the cleanup inside Add to Finder surfaces its kept folder like
    /// the rollback does, and a removal that failed but kept files still surfaces them, on the
    /// rollback and in the sweep.
    #[test]
    fn test_1882_r2_the_install_cleanup_and_failed_removals_surface_their_folder() {
        let full = source();
        let source = full[..full.find("\nmod finder_removal_wiring_tests;").expect("this module")].to_string();
        let code_only = |text: &str| {
            text.lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let install = code_only(&item(&source, "async fn install_finder_location("));
        let failed = &install[install
            .find("match install_file_provider_domain_blocking().await {")
            .expect("the install")..];
        let failed = &failed[..failed
            .find("Ok(FileProviderInstallOutcome::UserDisabled)")
            .expect("next arm")];
        let recorded = failed
            .find("finder_removal::remember_kept_folder_held(&mut cfg, location);")
            .expect("M1: the install cleanup's folder is saved for the row");
        let shown = failed
            .find("show_preserved_files_alert(&app, Some(location));")
            .expect("M1: and shown");
        let saved = failed
            .find("return finder_install_failed(&mut cfg, failure.message);")
            .expect("then saved");
        assert!(recorded < saved && shown < saved);

        // M1 on macOS: since spec 2026-10-06 there is no `install()` (the reconciler adds Beebeeb), and the
        // reconciler never removes a domain it added: a check that does not come up ends `Failed`, the domain stays.
        let provider = include_str!("macos_file_provider.rs").replace("\r\n", "\n");
        assert!(
            !provider.contains("pub fn install("),
            "no macOS install() to clean up after"
        );
        let core = { let facade = include_str!("finder_setup/core.rs").replace("\r\n", "\n"); format!("{}\n{}", &facade[..facade.find("\n#[cfg(test)]\nmod tests {").expect("tests follow")], [include_str!("finder_setup/core/decide.rs"), include_str!("finder_setup/core/landing.rs"), include_str!("finder_setup/core/results.rs"), include_str!("finder_setup/core/types.rs")].join("\n")) }.replace("\r\n", "\n");
        assert_eq!(
            core.matches("s.phase = Phase::Removing {").count(),
            4,
            "the reconciler removes on sign-out, on Repair, and for an owed removal (two places), never after an add"
        );

        // M2: the sweep keeps a folder that came back with an error ...
        let sweep = &provider[provider.find("pub fn cleanup_stale_domains(").expect("the sweep")..];
        let sweep = &sweep[..sweep.find("\n}\n").expect("the sweep ends")];
        assert!(sweep.contains("if let Some(location) = failure.kept_location(\"stale-domain sweep\") {"));
        // ... and the rollback surfaces a folder from a failed removal too.
        assert!(install.contains("let removal = remove_file_provider_domain_blocking().await;"));
        let squashed: String = install.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(squashed.contains(
            "removal.map_or_else(|failure|failure.kept_location(\"add-to-finderrollback\"),|removal|removal.kept_location(\"add-to-finderrollback\"),)"
        ));
    }

    #[test]
    fn test_1882_every_removal_path_surfaces_the_kept_folder() {
        let full = source();
        // Everything above this module, so the literals below cannot match themselves.
        let source = full[..full.find("\nmod finder_removal_wiring_tests;").expect("this module")].to_string();
        let code_only = |text: &str| {
            text.lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n")
        };

        // Sign-out: the removal's folder rides the result out of `clear_session_impl` ...
        let sign_out = code_only(&item(&source, "async fn clear_session_impl("));
        let removed = sign_out
            .find("let finder_cleanup = finder_remove_for(state, finder_setup::core::Trigger::SignOut).await;")
            .expect("spec A: the reconciler removes, first");
        let kept = sign_out
            .find("let preserved_location = finder_removal::sign_out_kept_location(finder_cleanup.clone());")
            .expect("the folder it kept");
        assert!(removed < kept);
        // Review I1: every failure after the removal carries it (the removal comes first in spec A, so that is
        // every failure from the engine stop on), and both Ok returns do.
        for failure in [
            "Some(take_slot_and_stop_engine_for_sign_out(&acct).await.map_err(&kept_with)?)",
            "let engine_slot = take_slot_and_stop_engine_for_sign_out(&acct).await.map_err(&kept_with)?;",
            "let kept = keep_for_sign_out(accepted.as_ref()).map_err(&kept_with)?;",
            "sign_out_seam(\"before_keep\").map_err(&kept_with)?;",
            "sign_out_seam(\"after_keep\").map_err(&kept_with)?;",
            "purge_local_data_for_sign_out(state_paths::beebeeb_state_dir(), cfg_sync_root.as_deref(), owe_removal, kept.as_ref()).map_err(&kept_with)?;",
            "sign_out_seam(\"after_purge\").map_err(&kept_with)?;",
            "clear_session_holding_slot(&acct, &engine_slot, account::SessionTransition::SignOut).map_err(&kept_with)?;",
            "finish_sign_out_turn(&acct, &engine_slot, true, &mut dropped).map_err(&kept_with)?;",
            "finish_sign_out_turn(&acct, &engine_slot, false, &mut dropped).map_err(&kept_with)?;",
            // Task 2191 (review thread on #157): the Windows keychain step too.
            "clear_keychain_session_into(acct.id.as_str(), &mut dropped).map_err(&kept_with)?;",
        ] {
            assert!(squash_ws(&sign_out).contains(&squash_ws(failure)), "{failure}");
        }
        // Task 1887: the keep step and its three test seams (test builds only) carry the folders too.
        assert_eq!(squash_ws(&sign_out).matches(".map_err(&kept_with)?").count(), 11);
        // Task 1886: both Ok returns are inside the local clear's block, which borrows the folder (`kept_with` does too,
        // so the field is counted per return).
        let returns: Vec<&str> = sign_out.split("Ok(SignedOut {").skip(1).collect();
        assert_eq!(returns.len(), 2, "two Ok returns");
        for ret in returns {
            let fields = &ret[..ret.find("})").expect("the literal's end")];
            assert!(fields.contains("preserved_location: preserved_location.clone(),"), "both Ok returns carry it");
        }
        // ... and both ways out of a sign-out raise the alert, on success and on failure (I1).
        let command = code_only(&item(&source, "async fn clear_session("));
        assert!(command.contains("surface_sign_out_folders(&app, &result);"));
        let menu = &source[source.find("DesktopMenuAction::SignOut => {").expect("menu sign-out")..];
        let menu = &menu[..menu.find("DesktopMenuAction::Quit").expect("next arm")];
        assert!(code_only(menu).contains("surface_sign_out_folders(&app, &result);"));

        // Repair: the folder goes into the result the Sync tab reads.
        let repair = code_only(&item(&source, "async fn reset_macos_integration("));
        assert!(repair.contains("let removal = finder_remove_for(&state, finder_setup::core::Trigger::Repair).await;"));
        assert!(repair.contains("let removal = remove_file_provider_domain_blocking().await;"));
        assert!(
            squash_ws(&repair).contains(&squash_ws(
                "let (removed_file_provider_domain, preserved_location) = finder_removal::repair_removal(removal, &mut warnings);"
            ))
        );
        assert!(repair.contains("        preserved_location,\n"));

        // The Add-to-Finder rollback and the app-start sweep raise the same alert.
        let install = code_only(&item(&source, "async fn install_finder_location("));
        assert!(install.contains("removal.kept_location(\"add-to-finder rollback\")"));
        assert!(install.contains("show_preserved_files_alert(&app, Some(&location));"));
        assert!(code_only(&source).contains("surface_kept_folder(&alert_app, Some(location));"));

        // And there is no other removal. macOS: two through the reconciler (sign-out, Repair). Windows and
        // Linux (no File Provider there): two through the wrapper that moves the call off the runtime (round 4).
        let production = super::finder_setup_command_tests::production_source();
        assert_eq!(
            code_only(&production).matches("finder_remove_for(").count(),
            3,
            "the definition and 2 calls"
        );
        assert_eq!(
            code_only(&production)
                .matches("remove_file_provider_domain_blocking().await")
                .count(),
            2,
            "2 calls through the blocking wrapper"
        );
        assert_eq!(
            code_only(&production).matches("remove_file_provider_domain()").count(),
            1,
            "only the definition: no inline call is left"
        );
    }

    /// 1882 r4 (concern 1): the removal, the install (whose cleanup is a removal) and the
    /// app-start sweep block for about a second when a folder reads as missing (`settle_kept_state`
    /// sleeps). None of them may run on the main thread or on an async runtime worker: every call
    /// goes through `on_blocking_pool`, which is `tokio::task::spawn_blocking`.
    ///
    /// Where each one runs (all async, so none is the main thread): the `clear_session` command and
    /// the menu's sign-out (`spawn_menu_task` -> `tauri::async_runtime::spawn`) call
    /// `clear_session_impl`; Repair and `install_finder_location` are async commands; the sweep is
    /// spawned from `setup`. The developer helper blocks its own main thread by design and sleeps on
    /// the completion handler's queue.
    #[test]
    fn test_1882_r4_no_blocking_file_provider_call_runs_on_the_main_thread_or_a_runtime_worker() {
        // Production code only: the census below must not count this file's tests, which name these calls.
        let source = super::finder_setup_command_tests::production_source();
        let code = source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let squash = |text: &str| text.chars().filter(|c| !c.is_whitespace()).collect::<String>();

        // The blocking primitives are called in exactly one place each: inside the pool helper (Windows and
        // Linux), or the reconciler's bridge gate (macOS, spec 2026-10-06), which this file never calls past.
        for (call, definitions) in [
            ("remove_file_provider_domain()", 1), // the other-OS definition; macOS removes through the reconciler
            ("install_file_provider_domain()", 1),
            ("macos_file_provider::remove()", 0), // only the reconciler's port calls it, in the bridge gate
            ("macos_file_provider::install()?", 0), // retired on macOS by spec 2026-10-06
        ] {
            assert_eq!(
                code.matches(call).count(),
                definitions,
                "`{call}` may appear only in its definition, never as an inline call"
            );
        }
        let pool = squash(&item(&code, "async fn on_blocking_pool"));
        assert!(
            pool.contains("tokio::task::spawn_blocking(work).await"),
            "the helper really uses the blocking pool: {pool}"
        );
        for (wrapper, primitive) in [
            (
                "async fn remove_file_provider_domain_blocking(",
                "on_blocking_pool(remove_file_provider_domain).await",
            ),
            (
                "async fn install_file_provider_domain_blocking(",
                "on_blocking_pool(install_file_provider_domain).await",
            ),
        ] {
            assert!(
                squash(&item(&code, wrapper)).contains(primitive),
                "{wrapper} goes through the pool with {primitive}"
            );
        }
        // macOS: the reconciler's removal and the app-start sweep run inside the bridge gate, whose one
        // blocking spawn runs every bridge call on the blocking pool (spec 2026-10-06, lead ruling T8-gate-all).
        let ports = include_str!("finder_setup/macos_ports.rs").replace("\r\n", "\n");
        assert!(
            squash(&item_in(&ports, "    async fn blocking_holding<", "\n    }\n"))
                .contains("tauri::async_runtime::spawn_blocking(move||{"),
            "the gate's one blocking spawn"
        );
        let run = &ports[ports.find("fn run(&mut self, op: Op)").expect("run")..];
        let run = &run[..run.find("fn publish(").expect("publish follows run")];
        let removal = squash(&run[run.find("Op::RemoveDomain =>").expect("the removal")..]);
        assert!(
            removal.contains("gate.run_waiting(BRIDGE_GATE_WAIT,answering(answer,macos_file_provider::remove,late))"),
            "the reconciler's removal goes through the gate"
        );
        assert!(
            squash(&item(&ports, "pub(crate) async fn cleanup_stale_domains("))
                .contains("BridgeGate::shared().run(macos_file_provider::cleanup_stale_domains)"),
            "so does the sweep"
        );

        // Every caller awaits the reconciler or the wrapper.
        let sign_out = squash(&item(&code, "async fn clear_session_impl("));
        assert!(sign_out.contains("finder_remove_for(state,finder_setup::core::Trigger::SignOut).await"));
        let repair = squash(&item(&code, "async fn reset_macos_integration("));
        assert!(repair.contains("finder_remove_for(&state,finder_setup::core::Trigger::Repair).await"));
        assert!(repair.contains("letremoval=remove_file_provider_domain_blocking().await;"));
        let install = squash(&item(&code, "async fn install_finder_location("));
        assert!(install.contains("matchinstall_file_provider_domain_blocking().await{"));
        assert!(install.contains("letremoval=remove_file_provider_domain_blocking().await;"));
        let sweep_at = code
            .find("match crate::finder_setup::macos_ports::cleanup_stale_domains().await")
            .expect("the sweep awaits the gate");
        assert!(
            squash(&code[sweep_at - 120..sweep_at]).ends_with("tauri::async_runtime::spawn(asyncmove{"),
            "and runs in a spawned task, off the startup path"
        );

        // The menu's sign-out is spawned, never run inline on the main thread.
        let menu = &source[source.find("DesktopMenuAction::SignOut => {").expect("menu sign-out")..];
        let menu = squash(&menu[..menu.find("DesktopMenuAction::Quit").expect("next arm")]);
        assert!(menu.contains("spawn_menu_task(spec.id,asyncmove{"));
        assert!(menu.contains("clear_session_impl(&state,false,None,crate::UnknownFilesArg::Keep,None).await"));
        assert!(
            !menu.contains("remove_file_provider_domain"),
            "no removal in the menu handler itself"
        );
        let spawn = squash(&item(&code, "fn spawn_menu_task"));
        assert!(spawn.contains("tauri::async_runtime::spawn(asyncmove{"));
    }
