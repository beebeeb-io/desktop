    use super::finder_setup_command_tests::without_items;
    use super::{DesktopConfig, ReleaseChannel};

    /// Every `.rs` file under `src` as `(name relative to src, production text)`: LF line ends, test modules
    /// dropped, and lines that are comments dropped.
    pub(super) fn production_files() -> Vec<(String, String)> {
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
                    let production = without_items(&text.replace("\r\n", "\n"), &["#[cfg(test)]", "#[cfg(all(test"]);
                    let code = production
                        .lines()
                        .filter(|line| !line.trim_start().starts_with("//"))
                        .collect::<Vec<_>>()
                        .join("\n");
                    files.push((name, code));
                }
            }
        }
        files.sort();
        files
    }

    /// The text of the item starting at `start`, up to the next line that closes it at the start's own indentation.
    fn item(text: &str, start: &str) -> String {
        let at = text.find(start).unwrap_or_else(|| panic!("{start} exists"));
        let line_start = text[..at].rfind('\n').map_or(0, |n| n + 1);
        let indent: String = text[line_start..].chars().take_while(|c| *c == ' ').collect();
        let end = format!("\n{indent}}}\n");
        let to = text[at..].find(&end).unwrap_or_else(|| panic!("{start} ends"));
        text[at..at + to].to_string()
    }

    const KEPT: &str = "/Users/someone/Kept";

    /// Runs `seed` on an empty config in its own directory, then `site` (the real writer, which finds the file through
    /// `DesktopConfig::path()`) while another update is between its load and its save: that update sets the kept folder
    /// and stays inside for 200 ms. Returns what is on disk once both have finished. A writer that loads its own copy and
    /// saves it later puts back the copy it loaded, without the kept folder.
    async fn with_a_save_in_flight<Fut: std::future::Future<Output = ()>>(
        seed: impl FnOnce(&mut DesktopConfig),
        site: impl FnOnce() -> Fut,
    ) -> DesktopConfig {
        let dir = tempfile::tempdir().expect("temp dir");
        crate::config::CONFIG_DIR_OVERRIDE
            .scope(dir.path().to_path_buf(), async {
                let path = DesktopConfig::path().expect("the scoped config path");
                DesktopConfig::update_at(&path, |cfg| {
                    seed(cfg);
                    (true, ())
                })
                .expect("seed");
                let (entered_tx, entered_rx) = std::sync::mpsc::channel();
                let slow_path = path.clone();
                let slow = std::thread::spawn(move || {
                    DesktopConfig::update_at(&slow_path, |cfg| {
                        cfg.kept_unsynced_folder = Some(KEPT.to_string());
                        entered_tx.send(()).expect("the test is waiting");
                        std::thread::sleep(std::time::Duration::from_millis(200));
                        (true, ())
                    })
                });
                entered_rx.recv().expect("the slow update started");
                site().await; // waits for the lock, then reads the file as the slow update left it
                assert_eq!(slow.join().expect("no panic"), Ok(()));
                DesktopConfig::load_from(&path).expect("reads back")
            })
            .await
    }

    /// The real writers, each run while a save of the kept folder is in flight: the folder and the writer's own change
    /// are both on disk afterwards. This is the behaviour the converted sites share; the pin below holds the rest.
    #[tokio::test]
    async fn test_1889_the_real_writers_land_next_to_a_save_that_was_in_flight() {
        let on_disk = with_a_save_in_flight(
            |_| {},
            || async {
                super::mark_menu_bar_hint_seen().expect("mark_menu_bar_hint_seen");
            },
        )
        .await;
        assert_eq!(
            on_disk.kept_unsynced_folder.as_deref(),
            Some(KEPT),
            "mark_menu_bar_hint_seen: the folder"
        );
        assert!(on_disk.menu_bar_hint_seen, "mark_menu_bar_hint_seen: its change");

        let on_disk = with_a_save_in_flight(
            |_| {},
            || async {
                super::set_sync_mode("smart".to_string()).expect("set_sync_mode");
            },
        )
        .await;
        assert_eq!(
            on_disk.kept_unsynced_folder.as_deref(),
            Some(KEPT),
            "set_sync_mode: the folder"
        );
        assert_eq!(on_disk.sync_mode.as_deref(), Some("smart"), "set_sync_mode: its change");

        let on_disk = with_a_save_in_flight(
            |_| {},
            || async {
                super::mark_known_folder_onboarding_seen().expect("mark_known_folder_onboarding_seen");
            },
        )
        .await;
        assert_eq!(
            on_disk.kept_unsynced_folder.as_deref(),
            Some(KEPT),
            "mark_known_folder_onboarding_seen: the folder"
        );
        assert!(
            on_disk.known_folder_onboarding_seen,
            "mark_known_folder_onboarding_seen: its change"
        );

        let on_disk = with_a_save_in_flight(
            |_| {},
            || async {
                super::record_installed_release_channel(ReleaseChannel::Beta)
                    .expect("record_installed_release_channel");
            },
        )
        .await;
        assert_eq!(
            on_disk.kept_unsynced_folder.as_deref(),
            Some(KEPT),
            "record_installed_release_channel: the folder"
        );
        assert_eq!(
            on_disk.installed_release_channel,
            Some(ReleaseChannel::Beta),
            "record_installed_release_channel: its change"
        );

        let on_disk = with_a_save_in_flight(
            |_| {},
            || async {
                super::persist_last_signed_in_email(Some("someone@beebeeb.io".to_string()));
            },
        )
        .await;
        assert_eq!(
            on_disk.kept_unsynced_folder.as_deref(),
            Some(KEPT),
            "persist_last_signed_in_email: the folder"
        );
        assert_eq!(
            on_disk.last_signed_in_email.as_deref(),
            Some("someone@beebeeb.io"),
            "persist_last_signed_in_email: its change"
        );

        let on_disk = with_a_save_in_flight(
            |cfg| cfg.last_signed_in_email = Some("previous@beebeeb.io".to_string()),
            || async {
                super::forget_last_signed_in_email().expect("forget_last_signed_in_email");
            },
        )
        .await;
        assert_eq!(
            on_disk.kept_unsynced_folder.as_deref(),
            Some(KEPT),
            "forget_last_signed_in_email: the folder"
        );
        assert_eq!(
            on_disk.last_signed_in_email, None,
            "forget_last_signed_in_email: its change"
        );

        let on_disk = with_a_save_in_flight(
            |_| {},
            || async {
                let mut settings = crate::config::DesktopSettings::from(&DesktopConfig::default());
                settings.upload_kbps_limit = Some(1234);
                super::save_desktop_settings(settings).expect("set_desktop_config");
            },
        )
        .await;
        assert_eq!(
            on_disk.kept_unsynced_folder.as_deref(),
            Some(KEPT),
            "set_desktop_config: the folder"
        );
        assert_eq!(on_disk.upload_kbps_limit, 1234, "set_desktop_config: its change");

        let on_disk = with_a_save_in_flight(
            |_| {},
            || async {
                super::set_known_folder_backup("documents".to_string(), true)
                    .await
                    .expect("set_known_folder_backup");
            },
        )
        .await;
        assert_eq!(
            on_disk.kept_unsynced_folder.as_deref(),
            Some(KEPT),
            "set_known_folder_backup: the folder"
        );
        assert!(
            on_disk.known_folder_enabled("documents"),
            "set_known_folder_backup: its change"
        );

        let on_disk = with_a_save_in_flight(
            |_| {},
            || async {
                crate::config::DesktopConfig::ensure_account_id().expect("ensure_account_id");
            },
        )
        .await;
        assert_eq!(
            on_disk.kept_unsynced_folder.as_deref(),
            Some(KEPT),
            "ensure_account_id: the folder"
        );
        assert!(on_disk.account_id.is_some(), "ensure_account_id: its change");
    }

    /// The census itself must have read the tree: a pin that scanned nothing passes on anything.
    #[test]
    fn test_1889_the_census_reads_the_whole_tree() {
        let files = production_files();
        let names: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
        assert!(files.len() >= 20, "read only {}: {names:?}", files.len());
        for must in [
            "lib.rs",
            "config.rs",
            "finder_removal.rs",
            "finder_setup/macos_ports.rs",
            "keychain.rs",
        ] {
            assert!(names.contains(&must), "{must} was not read: {names:?}");
        }
        let lib = &files.iter().find(|(name, _)| name == "lib.rs").unwrap().1;
        assert!(
            lib.contains("fn set_sync_mode(") && !lib.contains("mod config_write_pin_tests"),
            "production code is read, test modules are not"
        );
    }

    /// No load-modify-save span outside the one helper: nothing outside `config.rs` writes the file, and `config.rs`
    /// writes it in exactly one place, which holds the lock across the read, the change and the write.
    #[test]
    fn test_1889_no_desktop_config_load_modify_save_span_exists_outside_the_one_helper() {
        let files = production_files();
        for (name, text) in &files {
            if name == "config/store.rs" {
                continue;
            }
            for banned in [".save()", ".save_to("] {
                let lines: Vec<&str> = text.lines().filter(|line| line.contains(banned)).collect();
                assert!(
                    lines.is_empty(),
                    "{name} writes desktop.toml with `{banned}`; write it through DesktopConfig::update, update_at or update_held: {lines:?}"
                );
            }
        }

        let config = &files.iter().find(|(name, _)| name == "config/store.rs").unwrap().1;
        assert!(!config.contains("fn save("), "config/store.rs has no `save` to call");
        assert_eq!(
            config.matches("save_to(").count(),
            2,
            "config.rs writes in one place: the definition of `save_to` and its one call"
        );
        assert_eq!(config.matches("config_write_guard()").count() + files.iter().find(|(name, _)| name == "config/lock.rs").expect("config/lock.rs is production").1.matches("config_write_guard()").count(), 2, "the lock is defined once (config/lock.rs) and taken in one place (config/store.rs)");
        let one_place = item(config, "    pub(super) fn try_update_at_then<R>(");
        let lock = one_place
            .find("let _writing = config_write_guard();")
            .expect("it takes the lock");
        let read = one_place.find("Self::load_from(path)").expect("it reads");
        let write = one_place.find("cfg.save_to(path)").expect("it writes");
        let mirror = one_place.find("saved(&cfg);").expect("it runs the mirror step");
        assert!(
            lock < read && read < write && write < mirror,
            "the lock is held before the read and still held at the write and the mirror step:\n{one_place}"
        );
        // Task 1887: the keep step's durable save is the same one place with a durable write step, never a second
        // load-modify-save: `save_to_durable` is defined once and called once, from that place.
        assert_eq!(
            config.matches("save_to_durable(").count(),
            2,
            "the durable write is defined once and called once, from the one place"
        );
        assert!(one_place.contains("cfg.save_to_durable(path, fs_steps)"), "{one_place}");

        // Every other way in is that one place.
        for (start, calls) in [
            ("    pub(crate) fn update_at<R>(", "Self::try_update_at("),
            ("    pub(crate) fn try_update_at<R>(", "Self::try_update_at_then("),
            ("    pub(crate) fn update_at_then<R>(", "Self::try_update_at_then("),
            (
                "    pub(crate) fn update_at_durable_with<R>(",
                "Self::try_update_at_then(",
            ),
            (
                "    pub(crate) fn update<R>(",
                "Self::update_at(&Self::path()?, change)",
            ),
            ("    pub(crate) fn update_held_at<R>(", "DesktopConfig::update_at(path,"),
            ("    pub(crate) fn ensure_account_id_at(", "Self::try_update_at(path,"),
        ] {
            let body = item(if start.contains("update_held_at") { &files.iter().find(|(name, _)| name == "config/lock.rs").unwrap().1 } else { config }, start);
            assert!(body.contains(calls), "{start} is `{calls}`:\n{body}");
            assert!(!body.contains("load_from(") && !body.contains("save_to("), "{body}");
        }
    }

    /// The text inside the parentheses of the call whose `(` is at byte `open`.
    fn call_text(text: &str, open: usize) -> &str {
        let mut depth = 0usize;
        for (at, c) in text[open..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return &text[open + 1..open + at];
                    }
                }
                _ => {}
            }
        }
        &text[open + 1..]
    }

    /// The calls whose closure gets the config to change.
    const UPDATE_CALLS: [&str; 7] = [
        "DesktopConfig::update(",
        "DesktopConfig::update_at(",
        "DesktopConfig::try_update_at(",
        "DesktopConfig::update_at_then(",
        "DesktopConfig::update_at_durable_with(",
        ".update_held(",
        ".update_held_at(",
    ];

    /// Fix round 1 (M1), shape 1: a stale WHOLE copy written through an update closure. The closure's argument is the
    /// file as it is now; assigning over all of it (`*cfg = ...`), swapping or replacing or taking it, or `clone_from`
    /// puts back a copy loaded earlier, which is the bug in a new shape. Returns what it found, one line each.
    fn whole_copy_writes(text: &str) -> Vec<String> {
        let mut found = Vec::new();
        for needle in UPDATE_CALLS {
            for (at, _) in text.match_indices(needle) {
                let call = call_text(text, at + needle.len() - 1);
                let Some(open) = call.find('|') else { continue };
                let Some(close) = call[open + 1..].find('|') else {
                    continue;
                };
                let params = call[open + 1..open + 1 + close].trim();
                let name: String = params
                    .trim_start_matches("mut ")
                    .trim_start_matches("&mut ")
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if name.is_empty() || name == "_" {
                    continue;
                }
                let body = crate::source_pin::squeeze(&call[open + 1 + close + 1..]);
                let assign = format!("*{name}=");
                let assigns = body.matches(&assign).count() - body.matches(&format!("{assign}=")).count();
                if assigns > 0 {
                    found.push(format!(
                        "{needle} closure `{name}` assigns over the whole config: `{assign}`"
                    ));
                }
                for shape in [
                    format!("mem::replace({name},"),
                    format!("mem::swap({name},"),
                    format!("mem::take({name})"),
                    format!("{name}.clone_from("),
                ] {
                    if body.contains(&shape) {
                        found.push(format!(
                            "{needle} closure `{name}` overwrites the whole config: `{shape}`"
                        ));
                    }
                }
            }
        }
        found
    }

    /// The held-copy helpers: the only callees a flow may hand its copy to.
    const HELD_CALLEES: [&str; 6] = [
        "begin_finder_install_attempt",
        "finder_install_failed",
        "persist_finder_install_result",
        "persist_sync_root_and_start_engine",
        "finish_repair_after_removal",
        "remember_kept_folder_held",
    ];

    /// Fix round 1 (M1), shape 2: an edit to a held copy outside an update closure. The copy (`cfg`) is only read; it is
    /// changed through `cfg.update_held(..)`, or by handing `&mut cfg` to one of the held helpers. An assignment to one of
    /// its fields, a mutating method on one, or `&mut cfg` handed anywhere else is an edit that the refresh would drop (and
    /// that does not compile where `HeldConfig` has no `DerefMut`; this scan covers the code a Mac does not compile).
    fn held_copy_edits(flow: &str) -> Vec<String> {
        let mut found = Vec::new();
        for (at, _) in flow.match_indices("cfg.") {
            if flow[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_')
            {
                continue;
            }
            let path: String = flow[at + 4..]
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
                .collect();
            let after = flow[at + 4 + path.len()..].trim_start();
            let assigns = ["+=", "-=", "|=", "&="].iter().any(|op| after.starts_with(op))
                || (after.starts_with('=') && !after.starts_with("=="));
            let method = path.rsplit('.').next().unwrap_or("");
            let mutating = path.contains('.')
                && [
                    "push", "insert", "remove", "clear", "take", "retain", "extend", "sort", "truncate",
                ]
                .contains(&method);
            let whole = ["set_known_folder", "apply_settings", "ensure_account_id"].contains(&path.as_str());
            if assigns || mutating || whole {
                found.push(format!("`cfg.{path}` is edited outside update_held"));
            }
        }
        for (at, _) in flow.match_indices("&mut cfg") {
            let mut depth = 0usize;
            let mut open = None;
            for (i, c) in flow[..at].char_indices().rev() {
                match c {
                    ')' => depth += 1,
                    '(' if depth == 0 => {
                        open = Some(i);
                        break;
                    }
                    '(' => depth -= 1,
                    _ => {}
                }
            }
            let callee: String = open
                .map(|i| {
                    flow[..i]
                        .trim_end()
                        .chars()
                        .rev()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect()
                })
                .unwrap_or_default();
            if !HELD_CALLEES.contains(&callee.as_str()) {
                found.push(format!(
                    "`&mut cfg` is handed to `{callee}`, which is not a held helper"
                ));
            }
        }
        found
    }

    /// Fix round 1 (M6). The in-memory pause flag and `pause_sync` on disk are changed in one order: a pause is stored
    /// into the flag by the write that persists it, and a rehydration reads the file and stores into the flag, both while
    /// the config-write lock is held. A lock holder is in the middle of its write here, so a rehydration that did not
    /// wait for the lock would read the file from before it.
    #[test]
    fn test_1889_m6_a_rehydration_waits_for_the_write_in_flight_and_stores_what_it_wrote() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("desktop.toml");
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let holder_path = path.clone();
        let holder = std::thread::spawn(move || {
            DesktopConfig::update_at(&holder_path, |cfg| {
                cfg.pause_sync = true; // the pause is on its way to the file
                entered_tx.send(()).expect("the test is waiting");
                std::thread::sleep(std::time::Duration::from_millis(200));
                (true, ())
            })
        });
        entered_rx.recv().expect("the write started");
        super::rehydrate_sync_paused_at(&path, &flag).expect("rehydrate");
        holder.join().expect("no panic").expect("the pause was written");
        assert!(
            DesktopConfig::load_from(&path).expect("reads back").pause_sync,
            "the file says paused"
        );
        assert!(
            flag.load(std::sync::atomic::Ordering::Relaxed),
            "and so does the flag: the rehydration read the file after that write"
        );
    }

    #[test]
    fn test_1889_m6_a_pause_reaches_the_flag_only_with_the_write_that_persists_it() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("desktop.toml");
        let flag = std::sync::Arc::new(AtomicBool::new(false));
        // A write is in flight when the pause arrives.
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let holder_path = path.clone();
        let holder = std::thread::spawn(move || {
            DesktopConfig::update_at(&holder_path, |cfg| {
                cfg.kept_unsynced_folder = Some(KEPT.to_string());
                entered_tx.send(()).expect("the test is waiting");
                std::thread::sleep(std::time::Duration::from_millis(200));
                (true, ())
            })
        });
        entered_rx.recv().expect("the write started");
        let pause_path = path.clone();
        let pause_flag = flag.clone();
        let pause = std::thread::spawn(move || super::persist_sync_paused_at(&pause_path, &pause_flag, true));
        std::thread::sleep(std::time::Duration::from_millis(60));
        assert!(
            !flag.load(Ordering::Relaxed),
            "the pause has not been written yet, so the flag does not say paused yet"
        );
        holder.join().expect("no panic").expect("the holder wrote");
        pause.join().expect("no panic").expect("the pause was written");
        let on_disk = DesktopConfig::load_from(&path).expect("reads back");
        assert!(
            on_disk.pause_sync && flag.load(Ordering::Relaxed),
            "file and flag agree: paused"
        );
        assert_eq!(
            on_disk.kept_unsynced_folder.as_deref(),
            Some(KEPT),
            "and the other write survived"
        );

        // A pause that cannot be written is not applied either: the file and the flag cannot disagree.
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("desktop.toml");
        std::fs::create_dir(path.with_extension("toml.tmp")).expect("mkdir");
        let flag = AtomicBool::new(false);
        assert!(super::persist_sync_paused_at(&path, &flag, true).is_err());
        assert!(!path.exists(), "nothing was written");
        assert!(!flag.load(Ordering::Relaxed), "and the flag did not move");
    }

    /// Fix round 1 (M5), end to end through the real commands: the window opens (its copy has `pause_sync: false`), the tray
    /// pauses, the window changes a speed and sends only that field. The pause stays.
    #[tokio::test]
    async fn test_1889_m5_a_speed_changed_after_a_tray_pause_leaves_the_pause() {
        let dir = tempfile::tempdir().expect("temp dir");
        crate::config::CONFIG_DIR_OVERRIDE
            .scope(dir.path().to_path_buf(), async {
                let path = DesktopConfig::path().expect("the scoped config path");
                let window_copy = super::get_desktop_config().expect("the window reads on open");
                assert_eq!(window_copy.pause_sync, Some(false));

                let flag = std::sync::atomic::AtomicBool::new(false);
                super::persist_sync_paused_at(&path, &flag, true).expect("the tray pauses");

                let patch: crate::config::DesktopSettings =
                    serde_json::from_str(r#"{ "upload_kbps_limit": 2000 }"#).expect("the window sends one field");
                super::save_desktop_settings(patch).expect("the speed is saved");

                let on_disk = DesktopConfig::load_from(&path).expect("reads back");
                assert!(on_disk.pause_sync, "the tray's pause survived the speed change");
                assert_eq!(on_disk.upload_kbps_limit, 2000);
            })
            .await;
    }

    /// Fix round 2 (N2), in the menu: the ToggleSync arm waits for the toggle, then hands the result to
    /// `show_menu_pause_failure` before returning it to the logger, and that function raises a native error dialog for
    /// exactly what `menu_pause_failure_alert` says. (The same shape as 1885's `test_1885_the_menus_open_folder_shows_a_failure_before_it_logs_it`.)
    #[test]
    fn test_1889_n2_the_menus_pause_shows_a_failure_before_it_logs_it() {
        use crate::source_pin::squeeze;
        let files = production_files();
        let lib = &files.iter().find(|(name, _)| name == "lib.rs").unwrap().1;
        let arm_start = lib
            .find("DesktopMenuAction::ToggleSync =>")
            .expect("the menu's toggle arm");
        let arm_end = lib[arm_start..]
            .find("DesktopMenuAction::SignOut")
            .expect("the next arm");
        let arm = squeeze(&lib[arm_start..arm_start + arm_end]);
        let toggle = arm
            .find(&squeeze("toggle_sync_paused_from_menu(&app).await"))
            .expect("the arm waits for the toggle");
        let show = arm
            .find(&squeeze("show_menu_pause_failure(&app, pausing, &result);"))
            .expect("the arm shows a failure");
        let give_back = arm.rfind("result").expect("the arm returns the result");
        assert!(toggle < show, "the failure is shown after the toggle ended: {arm}");
        assert!(
            show < give_back,
            "the result goes on to the logger after it is shown: {arm}"
        );
        let shower = squeeze(&item(lib, "fn show_menu_pause_failure("));
        assert!(
            shower.contains(&squeeze("menu_pause_failure_alert(result, pausing)"))
                && shower.contains(".dialog()")
                && shower.contains(&squeeze("MessageDialogKind::Error")),
            "the alert is native, an error dialog, for exactly what the pure function says: {shower}"
        );
    }

    /// Fix round 2 (N2): the alert's words. A success shows nothing; a failure shows the sentence for the way it was
    /// going, and never the error behind it (an OS message can name a folder).
    #[test]
    fn test_1889_n2_a_failed_menu_pause_or_resume_says_which_and_never_the_error() {
        let os_text = "rename /Users/sam/Library/Application Support/beebeeb/desktop.toml.tmp: Permission denied";
        assert_eq!(super::menu_pause_failure_alert(&Ok(()), true), None);
        assert_eq!(super::menu_pause_failure_alert(&Ok(()), false), None);
        let pause = super::menu_pause_failure_alert(&Err(os_text.to_string()), true).expect("a failed pause is shown");
        let resume =
            super::menu_pause_failure_alert(&Err(os_text.to_string()), false).expect("a failed resume is shown");
        assert_eq!(
            pause,
            (
                "Pause sync",
                "Beebeeb couldn’t pause syncing, so it carries on as before."
            )
        );
        assert_eq!(
            resume,
            ("Resume sync", "Beebeeb couldn’t resume syncing, so it stays paused.")
        );
        for (title, sentence) in [pause, resume] {
            assert!(!title.contains("/Users") && !sentence.contains("/Users") && !sentence.contains("Permission"));
            assert_eq!(sentence.matches(". ").count(), 0, "one sentence: {sentence}");
            assert!(sentence.ends_with('.'));
        }
    }

    /// The scanners must be able to go red: each shape, written out, is found; the real shapes are not.
    #[test]
    fn test_1889_the_scanners_find_the_shapes_they_are_for_and_pass_the_real_ones() {
        // Shape 1.
        for bad in [
            "DesktopConfig::update(|cfg| { *cfg = stale.clone(); (true, ()) })",
            "DesktopConfig::update_at(&path, |cfg| { *cfg=stale; (true, ()) })",
            "held.update_held(|fresh| { std::mem::replace(fresh, stale); (true, ()) })",
            "DesktopConfig::try_update_at(&path, |cfg| { std::mem::swap(cfg, &mut stale); (true, ()) })",
            "DesktopConfig::update(|cfg| { cfg.clone_from(&stale); (true, ()) })",
            "DesktopConfig::update(|c| { let _old = std::mem::take(c); (true, ()) })",
            "DesktopConfig::update_at_then(&path, |cfg| { *cfg = stale.clone(); (true, ()) }, |_| {})",
            "DesktopConfig::update_at_durable_with(&path, |cfg| { *cfg = stale; (true, ()) }, &RealConfigFs)",
        ] {
            assert!(!whole_copy_writes(bad).is_empty(), "not found: {bad}");
        }
        for good in [
            "DesktopConfig::update(|cfg| { cfg.sync_mode = Some(mode); (true, ()) })",
            "DesktopConfig::update(|cfg| { if *cfg == other { return (false, ()); } (true, ()) })",
            "held.update_held(|held| (clear_finder_install_failure(held), ()))",
            "DesktopConfig::update_at_then(path, |cfg| { cfg.pause_sync = paused; (true, ()) }, |cfg| flag.store(cfg.pause_sync, Ordering::Relaxed))",
            "DesktopConfig::update_at_durable_with(path, |cfg| (true, record_kept_unsent(cfg, folder)), fs_steps)",
        ] {
            assert_eq!(whole_copy_writes(good), Vec::<String>::new(), "found in: {good}");
        }
        // Shape 2.
        for bad in [
            "let mut cfg = HeldConfig::load()?;\n cfg.sync_root = Some(root);",
            "cfg.finder_install_status = None;",
            "cfg.known_folder_backup.push(key);",
            "cfg.excluded_folder_ids.take();",
            "cfg.set_known_folder(&key, true);",
            "cfg.pause_sync |= paused;",
            "write_it(&mut cfg, 3)?;",
        ] {
            assert!(!held_copy_edits(bad).is_empty(), "not found: {bad}");
        }
        for good in [
            "begin_finder_install_attempt(&mut cfg)?;\n finder_install_failed(&mut cfg, failure.message)",
            "finish_repair_after_removal(&mut cfg, loc, |cfg| persist_finder_install_result(cfg, false, None), |l| s(l))",
            "finder_removal::remember_kept_folder_held(&mut cfg, location);",
            "persist_sync_root_and_start_engine(app, &state, &mut cfg, root.clone(), &t).await?;",
            "if cfg.finder_install_status.as_deref() != Some(\"installed\") { x }",
            "cfg.update_held(|held| { held.sync_root = Some(root.clone()); (true, ()) })?;",
            "let ok = cfg.pause_sync == true;",
        ] {
            assert_eq!(held_copy_edits(good), Vec::<String>::new(), "found in: {good}");
        }
    }

    /// Shape 1 on the real source: no update closure anywhere in production code writes over the whole config.
    /// (`config.rs` is the one place that holds the whole copy on purpose, in `HeldConfig::update_held_at`.)
    #[test]
    fn test_1889_no_stale_whole_copy_is_written_through_an_update_closure() {
        let mut found = Vec::new();
        for (name, text) in production_files() {
            if name == "config/store.rs" || name == "config/lock.rs" {
                continue;
            }
            for line in whole_copy_writes(&text) {
                found.push(format!("{name}: {line}"));
            }
        }
        assert!(found.is_empty(), "{found:#?}");
    }

    /// Shape 2 on the real source: the flows that hold a copy only read it, and change it through `update_held` or
    /// the held helpers. These bodies are Windows and Linux code that a Mac does not compile, so the text is the guard.
    #[test]
    fn test_1889_no_held_copy_is_edited_outside_update_held() {
        let files = production_files();
        let lib = &files.iter().find(|(name, _)| name == "lib.rs").unwrap().1;
        let mut found = Vec::new();
        for name in [
            "install_finder_location",
            "continue_without_finder_location",
            "install_windows_shell_integration",
            "finder_location_state",
            "reset_macos_integration",
            "persist_finder_install_result",
            "begin_finder_install_attempt",
            "finder_install_failed",
            "persist_sync_root_and_start_engine",
            "finish_repair_after_removal",
        ] {
            let body = item(lib, &format!("fn {name}("));
            for line in held_copy_edits(&body) {
                found.push(format!("{name}: {line}"));
            }
        }
        assert!(found.is_empty(), "{found:#?}");
    }

    /// The sweep, site by site: each place that loads the config in order to change it makes the change inside one
    /// update, and does not load a copy of its own first. `forbids` is what the site used to do.
    #[test]
    fn test_1889_every_writer_of_desktop_toml_changes_it_inside_one_update() {
        let files = production_files();
        let lib = &files.iter().find(|(name, _)| name == "lib.rs").unwrap().1;
        let update = "DesktopConfig::update(";
        let held = ".update_held(";
        // (the item, what it must contain). None loads a copy of its own: load, change and write are one update.
        let sites: [(&str, &str); 17] = [
            ("persist_last_signed_in_email", update),
            ("forget_last_signed_in_email", "DesktopConfig::try_update_at("),
            ("persist_finder_install_result", held),
            ("begin_finder_install_attempt", held),
            ("finder_install_failed", held),
            ("persist_sync_root_and_start_engine", held),
            ("ensure_sync_root_and_engine", update),
            ("pick_sync_root", update),
            // Task 1683 slice 1 (review item 3): the command is a wrapper that also refreshes the menu's pause label;
            // its write moved, unchanged, to `save_desktop_settings`.
            ("save_desktop_settings", update),
            ("set_sync_paused", "persist_sync_paused_at("),
            ("persist_sync_paused_at", "DesktopConfig::update_at_then("),
            ("set_sync_mode", update),
            ("set_selective_sync", update),
            ("set_known_folder_backup", update),
            ("mark_known_folder_onboarding_seen", update),
            ("mark_menu_bar_hint_seen", update),
            ("record_installed_release_channel", update),
        ];
        for (name, must) in sites {
            let start = format!("fn {name}(");
            let body = item(lib, &start);
            assert!(
                body.contains(must),
                "{name} changes the config through `{must}`:\n{body}"
            );
            assert!(
                !body.contains("DesktopConfig::load()"),
                "{name} loads a copy of its own; load, change and write are one update:\n{body}"
            );
        }
        assert!(
            lib.contains("match DesktopConfig::ensure_account_id() {"),
            "startup mints the account id through the one update"
        );
        // M6: the in-memory pause flag is stored from the file only under the config-write lock, through the helpers
        // (`persist_sync_paused_at`, `rehydrate_sync_paused_at`), never by a bare store next to a load.
        for (name, text) in &files {
            assert!(
                !text.contains(".sync_paused.store("),
                "{name} stores the pause flag outside the locked helpers"
            );
        }

        // The flows that keep a copy to READ across slow work write only through `update_held`, so the copy they loaded
        // is never what lands on the file.
        for name in [
            "install_finder_location",
            "continue_without_finder_location",
            "install_windows_shell_integration",
            "finder_location_state",
            "reset_macos_integration",
        ] {
            let body = item(lib, &format!("fn {name}("));
            assert!(!body.contains(".save()"), "{name}:\n{body}");
            assert!(
                body.contains("persist_finder_install_result(")
                    || body.contains("persist_sync_root_and_start_engine(")
                    || body.contains("remember_kept_folder_held("),
                "{name} writes through the held-copy helpers:\n{body}"
            );
        }
    }
