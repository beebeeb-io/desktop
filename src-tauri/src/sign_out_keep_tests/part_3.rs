/// U45: the launch's reopen acts on the app's own state database (here the test's, through `in_own_dirs`), not
/// only on a directory its caller names, and it clears the key alone: the R10 owner record and a Finder removal
/// debt stay, or the restore's engine start would reset the local data as unowned.
#[tokio::test]
async fn the_launch_reopens_accepts_in_the_apps_state_db() {
    in_own_dirs(async {
        let db = state_db_in_scratch();
        let owner = crate::account_binding::Identity::new(Some("u-a"), Some("a@beebeeb.io"));
        db.set_owner(&owner).unwrap();
        db.set_finder_removal_owed(true).unwrap();
        db.close_finder_accepts().unwrap();
        reopen_finder_accepts_at_startup();
        assert!(
            !db.finder_accepts_closed().unwrap(),
            "the launch reopens accepts in the app's state.db"
        );
        assert_eq!(db.owner().unwrap(), Some(owner), "the launch's reopen leaves the owner");
        assert!(
            db.finder_removal_owed().unwrap(),
            "the launch's reopen leaves a removal debt"
        );
    })
    .await;
}

/// The launch's reopen is best effort. A state database it cannot open is logged by its step, never by the error,
/// whose text names the database's path (W10).
#[tokio::test]
async fn a_launch_reopen_that_cannot_open_the_state_db_logs_the_step() {
    in_own_dirs(async {
        let dir = state_paths::beebeeb_state_dir().unwrap();
        std::fs::create_dir_all(state_paths::state_db_path_from_state_dir(&dir)).unwrap();
        let logs = capture_logs(reopen_finder_accepts_at_startup);
        let path = dir.to_string_lossy().into_owned();
        assert!(!logs.contains(&path), "no log carries the path:\n{logs}");
        let warning = logs
            .lines()
            .find(|line| line.contains("could not reopen Finder accepts"))
            .unwrap_or_else(|| panic!("the failed open is logged:\n{logs}"));
        assert!(
            warning.contains("step=state_db"),
            "the warning names the step: {warning}"
        );
    })
    .await;
}

/// U26.
#[test]
fn show_in_finder_reveals_only_the_saved_folder() {
    assert_eq!(reveal_target(Some("/kept/a"), "/kept/a"), Ok("/kept/a"));
    assert!(reveal_target(Some("/kept/a"), "/Users/a/Desktop").is_err());
    assert!(reveal_target(None, "/kept/a").is_err());
}

/// U31.
#[test]
fn show_in_finder_uses_the_opener_reveal() {
    let production = super::finder_setup_command_tests::production_source();
    let body = super::finder_setup_command_tests::body_between(&production, "fn show_kept_unsent_folder(", "\n}\n");
    assert!(body.contains("reveal_item_in_dir("), "{body}");
    assert!(
        !body.contains("Command::new(\"open\")") && !body.contains("\"/usr/bin/open\""),
        "{body}"
    );
    assert!(
        body.contains("\"kept folder reveal requested\""),
        "D2 reads this line from stdout"
    );
}

/// Everything `tracing` emits on this thread at `info` and above while `body` runs, as text: a copy of
/// `engine_bridge.rs`'s `capture_logs_async`. `#[tokio::test]` runs the future on the test's own thread, so a
/// thread-local subscriber sees all of it.
async fn capture_logs_async(body: impl std::future::Future<Output = ()>) -> String {
    #[derive(Clone)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let writer = Capture(buffer.clone());
    // tracing caches each callsite's interest for the whole process. While only ONE dispatcher is alive,
    // tracing-core takes that interest from the registering thread's own default (`has_just_one`), so a callsite
    // first hit by another test thread, which has no subscriber, is cached as "never" and this capture misses its
    // events. A second live dispatcher keeps registration on the all-dispatchers path.
    let _second_dispatcher = tracing::Dispatch::new(tracing_subscriber::registry());
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    body.await;
    drop(guard);
    let bytes = buffer.lock().unwrap().clone();
    String::from_utf8(bytes).unwrap()
}

/// U25 across a keeping sign-out: the keep step's own capture (`sign_out::keep`'s test) misses every line the
/// sign-out logs around it. No line of the whole sign-out names a kept path or a file name.
#[tokio::test]
async fn a_keeping_sign_out_logs_no_path_or_name() {
    in_own_dirs(async {
        let db = state_db_in_scratch();
        db.hold_lock_for_test()
            .execute(
                "INSERT INTO files (file_id, path) VALUES ('secret', 'Secret Project/plan for Q4.docx')",
                [],
            )
            .unwrap();
        queue_write(&db, "ws", "secret", b"plan"); // its `INSERT OR IGNORE` keeps the row above
        let (state, _acct) = signed_out_state();
        recording_finder(&state);
        let id = ask(&state, "main");
        let logs = capture_logs_async(async {
            let done = sign_out_with(&state, Some(&id), no_seam()).await.unwrap();
            assert_eq!(done.kept_unsent.map(|kept| kept.files), Some(1));
        })
        .await;
        assert!(logs.contains("kept unsent changes"), "{logs}");
        for leaked in ["Secret Project", "plan for Q4", ".docx", "Changes not sent"] {
            assert!(!logs.contains(leaked), "{leaked} in {logs}");
        }
    })
    .await;
}

/// W9, §9: both sign-out routes surface both folders, through one function.
#[test]
fn both_sign_out_routes_surface_both_folders() {
    let production = super::finder_setup_command_tests::production_source();
    let command = super::finder_setup_command_tests::body_between(&production, "async fn clear_session(", "\n}\n");
    assert!(command.contains("surface_sign_out_folders(&app, &result)"), "{command}");
    let menu = &production[production.find("DesktopMenuAction::SignOut =>").unwrap()..];
    let menu = &menu[..menu.find("DesktopMenuAction::Quit").unwrap()];
    assert!(menu.contains("surface_sign_out_folders(&app, &result)"), "{menu}");
}

/// W9 and §9, inside the one function both routes call: macOS's folder is saved for its row (which tells the
/// windows), ours (the keep step recorded it) only tells them, and the one alert names both, ours first. The
/// function needs the app's dialog, so its body is read; the folders it reads come out of the result, checked here
/// for a sign-out that completed and one that stopped after the keep step.
#[test]
fn the_sign_out_surface_names_our_folder_and_saves_macos_folder_for_its_row() {
    let production = super::finder_setup_command_tests::production_source();
    let surface = crate::source_pin::squeeze(super::finder_setup_command_tests::body_between(
        &production,
        "fn surface_sign_out_folders(",
        "\n}\n",
    ));
    for step in [
        "let (macos, ours) = sign_out_folders(result);",
        "Some(location) => save_kept_folder_for_row(app, location),",
        "None if ours.is_some() => emit_kept_folder_changed(app),",
        "sign_out::copy::sign_out_alert(ours.map(|kept| (kept.folder.as_str(), kept.files)), macos)",
    ] {
        assert!(
            surface.contains(&crate::source_pin::squeeze(step)),
            "surface_sign_out_folders: {step}\n{surface}"
        );
    }
    let kept = KeptUnsent {
        folder: "/ours".into(),
        files: 2,
    };
    let done: Result<SignedOut, SignOutFailure> = Ok(SignedOut {
        preserved_location: Some("/macos".into()),
        kept_unsent: Some(kept.clone()),
        ..SignedOut::confirmed(SignOutOutcome::Completed)
    });
    assert_eq!(
        sign_out_folders(&done),
        (Some("/macos"), Some(&kept)),
        "a sign-out that completed"
    );
    let stopped: Result<SignedOut, SignOutFailure> = Err(SignOutFailure {
        message: "stopped after the keep step".into(),
        preserved_location: Some("/macos".into()),
        kept_unsent: Some(kept.clone()),
    });
    assert_eq!(
        sign_out_folders(&stopped),
        (Some("/macos"), Some(&kept)),
        "a sign-out that stopped after the keep step"
    );
}

/// §9: "Show in Finder" reveals a path only when it equals the saved record, so the webview cannot have the app
/// reveal any other path. U26 holds the comparison; this holds the command to it: it reads our record (never
/// 1882's), checks the path against it, and reveals only what the check returned.
#[test]
fn show_in_finder_reveals_only_what_the_saved_record_allows() {
    let production = super::finder_setup_command_tests::production_source();
    let body = crate::source_pin::squeeze(super::finder_setup_command_tests::body_between(
        &production,
        "fn show_kept_unsent_folder(",
        "\n}\n",
    ));
    let squeeze = crate::source_pin::squeeze;
    let read = body
        .find(&squeeze(
            "let saved = sign_out::kept_record::saved_kept_unsent_at(&DesktopConfig::path()?)?;",
        ))
        .unwrap_or_else(|| panic!("our record is read: {body}"));
    let check = body
        .find(&squeeze("let target = reveal_target(saved.as_deref(), &path)?;"))
        .unwrap_or_else(|| panic!("the path is checked against it: {body}"));
    let reveal = body
        .find(".reveal_item_in_dir(target)")
        .unwrap_or_else(|| panic!("only the checked path is revealed: {body}"));
    assert!(read < check && check < reveal, "read, check, then reveal: {body}");
    assert_eq!(body.matches("reveal_item_in_dir(").count(), 1, "one reveal: {body}");
}

/// §9, the row's other two commands: the row reads our record (never 1882's), its "Dismiss" goes through our
/// record's rule (only the folder the row showed) and then tells the windows, and the webview can call all three.
#[test]
fn the_rows_commands_use_our_record_and_are_registered() {
    let production = super::finder_setup_command_tests::production_source();
    let squeeze = crate::source_pin::squeeze;
    let read = squeeze(super::finder_setup_command_tests::body_between(
        &production,
        "fn kept_unsent_folder(",
        "\n}\n",
    ));
    assert!(
        read.contains(&squeeze(
            "sign_out::kept_record::saved_kept_unsent_at(&DesktopConfig::path()?)"
        )),
        "the row reads our record: {read}"
    );
    let dismiss = squeeze(super::finder_setup_command_tests::body_between(
        &production,
        "fn dismiss_kept_unsent_folder(",
        "\n}\n",
    ));
    let cleared = dismiss
        .find(&squeeze(
            "sign_out::kept_record::dismiss_saved_kept_unsent_at(&DesktopConfig::path()?, &path)?;",
        ))
        .unwrap_or_else(|| panic!("the dismiss goes through our record's rule: {dismiss}"));
    let told = dismiss
        .find("emit_kept_folder_changed(&app);")
        .unwrap_or_else(|| panic!("the windows are told: {dismiss}"));
    assert!(cleared < told, "dismissed, then told: {dismiss}");
    let handler = super::finder_setup_command_tests::body_between(&production, "tauri::generate_handler![", "])");
    for command in [
        "kept_unsent_folder",
        "dismiss_kept_unsent_folder",
        "show_kept_unsent_folder",
    ] {
        assert_eq!(
            handler
                .lines()
                .filter(|line| line.trim() == format!("{command},"))
                .count(),
            1,
            "{command} is registered once"
        );
    }
}

/// U34 (f), and (a)'s disarm.
#[tokio::test]
async fn the_send_phase_is_owned_by_rust() {
    in_own_dirs(async {
        let db = state_db_in_scratch();
        queue_write(&db, "w1", "f1", b"bytes");
        // Backing off far into the future (2100): only the phase's start makes it due now (§6.2).
        db.hold_lock_for_test()
            .execute(
                "UPDATE operation_queue SET next_retry_at = 4102444800 WHERE op_id = 'w1'",
                [],
            )
            .unwrap();
        let next_retry_at = || -> i64 {
            db.hold_lock_for_test()
                .query_row(
                    "SELECT next_retry_at FROM operation_queue WHERE op_id = 'w1'",
                    [],
                    |r| r.get(0),
                )
                .unwrap()
        };
        let state = AppState::default();
        let id = ask(&state, "main");
        let link = Arc::new(runner::SendPhaseLink::default());
        let stale = start_send_phase(&state.sign_out, "stale", "main", Some(link.clone()));
        assert!(stale.err().unwrap().starts_with("unsent_changes: "));
        assert_eq!(next_retry_at(), 4102444800, "a refused start makes nothing due");
        let run = start_send_phase(&state.sign_out, &id, "main", Some(link.clone())).unwrap();
        assert!(run.is_some() && link.is_armed());
        assert!(
            next_retry_at() <= now_unix_seconds(),
            "the start made the live write due now"
        );
        assert!(
            start_send_phase(&state.sign_out, &id, "main", Some(link.clone()))
                .unwrap()
                .is_none(),
            "a second start starts nothing"
        );
        end_send_phase(&state.sign_out, Some(&link));
        assert!(!link.is_armed() && state.sign_out.phase.lock().unwrap().is_none());
    })
    .await;
}

/// PR #130 review, "Do not treat a dead runner handle as a running engine": a runner whose task ended at startup
/// (the state dir, the database or the lock failed) leaves its handle in the slot. That is not a running engine:
/// `send_conditions` must say so, hand the phase no link, and the count must go straight to the honest ask
/// (`engine_stopped`), not offer a send nobody would run.
#[tokio::test]
async fn a_runner_that_ended_is_not_a_running_engine() {
    use sign_out::waiting::{NotSending, readiness};
    let state = AppState::default();
    synthesize_single_account(&state, AccountId("dead-runner".to_string()));
    let acct = state.active_account().unwrap();
    *acct.session.lock().unwrap() = Some(Session {
        token: "tok".into(),
        master_key: [9u8; 32],
        email: None,
    });

    // A runner whose task is still going: running, with its link.
    let (hold, held) = tokio::sync::oneshot::channel::<()>();
    *acct.engine.lock().await = Some(EngineRunner::for_test_with_task(tokio::spawn(async move {
        let _ = held.await;
    })));
    let (conditions, link) = send_conditions(&state).await;
    assert!(
        conditions.engine_running && link.is_some(),
        "a live runner is a running engine"
    );
    assert_eq!(readiness(conditions, false, true), None, "and it can send");

    // The same slot, its task gone while the handle stays in it.
    let _ = hold.send(());
    let task = tokio::spawn(async {});
    while !task.is_finished() {
        tokio::task::yield_now().await;
    }
    *acct.engine.lock().await = Some(EngineRunner::for_test_with_task(task));
    let (conditions, link) = send_conditions(&state).await;
    assert!(
        !conditions.engine_running,
        "a runner whose task ended is not a running engine"
    );
    assert!(link.is_none(), "and the phase gets no link to arm");
    assert_eq!(
        readiness(conditions, false, true),
        Some(NotSending::EngineStopped),
        "the count goes straight to the ask with the engine-stopped line"
    );
}

/// U46, the chain past the bridge: what the runner noted on the phase's link ends the phase at the driver's first
/// observation of it, long before the limit: `account` for a refusal the account cures, and `offline` and
/// `session` for an attempt's link failure and refused session (U16 reads those from an attempt alone).
/// `observe_phase` is the driver's own step, so this is the driver's reading, not a copy of it.
#[test]
fn an_attempts_refusal_ends_the_phase_at_its_first_observation() {
    use crate::engine_bridge::TransferLoopOutcome;
    use sign_out::send_phase::{PhaseEnd, PhaseStep, SIGN_OUT_SEND_LIMIT, SendPhase};
    let waiting = sign_out::waiting::WaitingChanges {
        files: Vec::new(),
        others: vec![sign_out::waiting::OtherChange {
            op_id: "r1".into(),
            live: true,
        }],
    };
    let good = sign_out::waiting::SendConditions {
        engine_running: true,
        vault_unlocked: true,
        session_expired: false,
        link_failed: false,
        sync_paused: false,
    };
    for (outcome, end) in [
        (
            TransferLoopOutcome {
                account_refused: true,
                ..Default::default()
            },
            PhaseEnd::Account,
        ),
        (
            TransferLoopOutcome {
                link_failed: true,
                ..Default::default()
            },
            PhaseEnd::Offline,
        ),
        (
            TransferLoopOutcome {
                session_refused: true,
                ..Default::default()
            },
            PhaseEnd::Session,
        ),
    ] {
        let link = Arc::new(runner::SendPhaseLink::default());
        link.arm();
        let run = PhaseRun {
            owner: "main".into(),
            link: Some(link.clone()),
            stop: Default::default(),
            closed: Default::default(),
            started: std::time::Instant::now(),
            changes_at_start: 1,
        };
        let mut phase = SendPhase::default();
        assert_eq!(
            phase.observe(&observe_phase(&run, &waiting, good)),
            PhaseStep::Progress(1),
            "nothing noted yet: it runs"
        );
        link.note(&outcome);
        let observation = observe_phase(&run, &waiting, good);
        assert!(observation.elapsed < SIGN_OUT_SEND_LIMIT);
        assert_eq!(phase.observe(&observation), PhaseStep::End(end), "{outcome:?}");
        assert_eq!(
            serde_json::to_value(end).unwrap(),
            end.as_str(),
            "the reason the ask's A5 line is chosen by"
        );
    }
}

/// §5.1, the contract: `sign_out_count`'s answer.
#[test]
fn the_count_answers_the_contract_shape() {
    let snapshot = AskSnapshot::of(&WaitingChanges::default(), "o", Vec::new());
    let json = serde_json::to_value(SignOutCount {
        counts: snapshot.counts,
        can_send: false,
        not_sending: Some(sign_out::waiting::NotSending::Offline),
    })
    .unwrap();
    for key in [
        "askId",
        "files",
        "copiedFiles",
        "others",
        "missingFiles",
        "earlierFolders",
        "canSend",
        "notSending",
    ] {
        assert!(json.get(key).is_some(), "{key} in {json}");
    }
    assert_eq!(json["notSending"], "offline");
}

fn phase_owned_by(state: &AppState, owner: &str) -> Arc<std::sync::atomic::AtomicBool> {
    let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    *state.sign_out.phase.lock().unwrap() = Some(crate::sign_out::send_phase::RunningPhase {
        ask_id: "a".into(),
        owner: owner.into(),
        stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        closed: closed.clone(),
    });
    closed
}

/// U34 (b)-(e), (g), (h).
#[tokio::test]
async fn the_send_phase_is_owned_by_rust_and_a_close_ends_the_flow() {
    in_own_dirs(async {
        let db = state_db_in_scratch();
        queue_write(&db, "w1", "f1", b"bytes");
        let state = AppState::default();
        // (b) a close during that window's phase: ended `closed`, its snapshot dropped; a later Sign out
        // refuses.
        let id = ask(&state, "main");
        let closed = phase_owned_by(&state, "main");
        let outcome = flow_window_closed(&state.sign_out, "main");
        assert!(outcome.phase_closed && closed.load(Ordering::SeqCst) && outcome.snapshot_dropped);
        let refused = sign_out_with(&state, Some(&id), no_seam()).await.unwrap_err();
        assert!(refused.message.starts_with("unsent_changes: "));
        // (c) at the ask, no phase: the snapshot is dropped.
        *state.sign_out.phase.lock().unwrap() = None;
        ask(&state, "main");
        assert!(flow_window_closed(&state.sign_out, "main").snapshot_dropped);
        // (d) another window's close leaves a phase and a snapshot of `main` alone.
        ask(&state, "main");
        let closed = phase_owned_by(&state, "main");
        for other in ["settings", "onboarding"] {
            let outcome = flow_window_closed(&state.sign_out, other);
            assert!(!outcome.phase_closed && !outcome.snapshot_dropped, "{other}");
        }
        assert!(!closed.load(Ordering::SeqCst));
        *state.sign_out.phase.lock().unwrap() = None;
        // (d), its other half: a close of `main` leaves a snapshot `onboarding` owns alone. (g) A close
        // of `onboarding` drops it.
        ask(&state, "onboarding");
        let outcome = flow_window_closed(&state.sign_out, "main");
        assert!(!outcome.snapshot_dropped, "(d) onboarding's snapshot stays");
        assert!(
            flow_window_closed(&state.sign_out, "onboarding").snapshot_dropped,
            "(g)"
        );
        // (h) running when its window closes, then failing: the dialog, once.
        state.sign_out.late_results.begin("main");
        assert_eq!(flow_window_closed(&state.sign_out, "main").show, None);
        assert!(
            state
                .sign_out
                .late_results
                .complete("main", Some(crate::sign_out::flow_window::Shown::Failure("x".into())))
                .is_some()
        );
    })
    .await;
}

/// U34's pins: the hook is called on every close, destroy and reload, with the window's own label; and its premise.
/// Each call is read inside its own arm, so one arm's call cannot stand in for the other's.
#[test]
fn every_close_destroy_and_reload_reaches_the_hook() {
    let production = super::finder_setup_command_tests::production_source();
    let handler = &production[production
        .find(".on_window_event(|window, event| {")
        .expect("the global handler")..];
    let handler = &handler[..handler.find("\n        })\n").unwrap()];
    let block = |source: &'static str, text: &str, opening: &str, closing: &str| -> String {
        let at = text
            .find(opening)
            .unwrap_or_else(|| panic!("{source}: {opening}\n{text}"));
        let end = text[at..]
            .find(closing)
            .unwrap_or_else(|| panic!("{source}: {opening} ends"));
        text[at..at + end].to_string()
    };
    for arm in [
        "if let tauri::WindowEvent::CloseRequested { .. } = event {",
        "if let tauri::WindowEvent::Destroyed = event {",
    ] {
        let arm_block = block("the global handler", handler, arm, "\n            }\n");
        assert!(
            arm_block.contains("sign_out_flow_window_closed(window.app_handle(), window.label())"),
            "{arm}\n{arm_block}"
        );
    }
    let page_load = &production[production.find(".on_page_load(").expect("an on_page_load registration")..];
    let page_load = &page_load[..page_load.find("\n        })\n").unwrap()];
    let started = block(
        "on_page_load",
        page_load,
        "if matches!(payload.event(), tauri::webview::PageLoadEvent::Started) {",
        "\n            }\n",
    );
    assert!(
        started.contains("sign_out_flow_window_closed(webview.app_handle(), webview.label())"),
        "{page_load}"
    );
    // Task 1683 slice 1 (two-surface §8.2): the flow's window is now the Beebeeb window, and a close destroys
    // it. The premise moved: the hook runs on `CloseRequested` AND on `Destroyed` (both arms pinned above), so a
    // destroyed window still ends its flow; spec 1887 §5.1, "A close ends the flow", already covers a destroy.
    assert_eq!(
        surfaces::policy::close_policy(surfaces::policy::Platform::Macos, "main"),
        surfaces::registry::ClosePolicy::Destroy,
        "the premise: a close destroys the Beebeeb window"
    );
}

/// U32, extended: the late result is macOS only, defined and registered. It reads the first definition of each
/// name, so every macOS command sits above its non-macOS twin, and that item's own attribute lines, so the `cfg`
/// of the item above cannot stand in for its own; and each call's own statement.
#[test]
fn the_late_result_is_macos_only() {
    let production = super::finder_setup_command_tests::production_source();
    let macos = "#[cfg(target_os = \"macos\")]";
    for definition in [
        "fn sign_out_result_ack(",
        "fn sign_out_flow_window_closed(",
        "fn sign_out_count(",
        "fn sign_out_send_start(",
    ] {
        let at = production.find(definition).unwrap_or_else(|| panic!("{definition}"));
        let line = production[..at].rfind('\n').map_or(0, |end| end + 1);
        let attributes: Vec<&str> = production[..line]
            .lines()
            .rev()
            .map(str::trim)
            .take_while(|above| above.starts_with("#[") || above.starts_with("///"))
            .collect();
        assert!(attributes.contains(&macos), "{definition}: {attributes:?}");
    }
    let handler = super::finder_setup_command_tests::body_between(&production, "tauri::generate_handler![", "])");
    assert_eq!(
        handler
            .lines()
            .filter(|line| line.trim() == "sign_out_result_ack,")
            .count(),
        1,
        "registered once"
    );
    // Read squeezed, as rustfmt lays the `complete` statement over several lines.
    let squeeze = crate::source_pin::squeeze;
    let command = squeeze(super::finder_setup_command_tests::body_between(
        &production,
        "async fn clear_session(",
        "\n}\n",
    ));
    for statement in [
        "state.sign_out.late_results.begin(",
        "if let Some(shown) = state.sign_out.late_results.complete(",
    ] {
        let at = command
            .find(&squeeze(statement))
            .unwrap_or_else(|| panic!("{statement}\n{command}"));
        assert!(command[..at].ends_with(&squeeze(macos)), "{statement}\n{command}");
    }
}

/// U44 (b) and (e), through the classification `clear_session` hands to the record (§5.1 "A late result"): a
/// failure and a completed sign-out's warning need showing, each with its own text; an `unsent_changes:` refusal
/// and a completed sign-out with no warning do not, and leave nothing for a close to show.
#[test]
fn a_late_result_is_shown_only_when_it_needs_showing() {
    use crate::sign_out::flow_window::{LateResults, Shown};
    let failed = |message: &str| -> Result<SignedOut, SignOutFailure> {
        Err(SignOutFailure {
            message: message.into(),
            preserved_location: None,
            kept_unsent: None,
        })
    };
    let done = |warning| -> Result<SignedOut, SignOutFailure> {
        Ok(SignedOut {
            warning,
            ..SignedOut::confirmed(SignOutOutcome::Completed)
        })
    };
    let refused = coded(UNSENT_CHANGES_CODE, E1_UNSENT_CHANGES);
    let not_kept = coded(UNSENT_COPY_FAILED_CODE, E2_UNSENT_COPY_FAILED);
    let cases = [
        (failed(&refused), None),
        (failed(&not_kept), Some(Shown::Failure(not_kept.clone()))),
        (
            failed(SIGN_OUT_PURGE_FAILED),
            Some(Shown::Failure(SIGN_OUT_PURGE_FAILED.into())),
        ),
        (
            done(Some(ActionWarning::FinderRemovalUnconfirmed)),
            Some(Shown::Warning(FINDER_SIGN_OUT_UNCONFIRMED_WARNING.into())),
        ),
        (done(None), None),
    ];
    for (result, expected) in cases {
        assert_eq!(late_result_of(&result), expected, "{result:?}");
        // What a close then finds: that result once, or nothing.
        let late = LateResults::default();
        late.begin("main");
        assert_eq!(late.complete("main", late_result_of(&result)), None);
        assert_eq!(late.window_closed("main"), expected, "a close after {result:?}");
        assert_eq!(late.window_closed("main"), None);
    }
}

/// §5.1 "A late result", the glue the pure tests cannot run (it needs the app's dialog and a Wry window), read from
/// its source: the call's record begins before the sign-out and completes after it with the classified result,
/// and what that returns is shown; the hook reads the closed window's own flow state, tells that window, and shows
/// what it took out; the acknowledgement names the calling window; the dialog keeps the menu's two titles.
#[test]
fn the_late_result_reaches_the_dialog_from_the_call_and_the_hook() {
    use crate::source_pin::squeeze;
    let production = super::finder_setup_command_tests::production_source();
    let body = |start: &str| {
        squeeze(super::finder_setup_command_tests::body_between(
            &production,
            start,
            "\n}\n",
        ))
    };
    let command = body("async fn clear_session(");
    let begin = command
        .find(&squeeze("state.sign_out.late_results.begin(window.label());"))
        .unwrap_or_else(|| panic!("the record begins: {command}"));
    let sign_out = command
        .find("clear_session_impl(")
        .unwrap_or_else(|| panic!("the sign-out: {command}"));
    let complete = command
        .find(&squeeze(
            "if let Some(shown) = state.sign_out.late_results.complete(window.label(), late_result_of(&result)) {
                    show_late_result(&app, shown);
                }",
        ))
        .unwrap_or_else(|| panic!("the record completes with the classified result, and a Some is shown: {command}"));
    assert!(
        begin < sign_out && sign_out < complete,
        "begin, the sign-out, then complete: {command}"
    );
    let hook = body("fn sign_out_flow_window_closed(");
    for step in [
        "let closed = flow_window_closed(&state.sign_out, label);",
        "let _ = app.emit_to(label, \"sign-out-flow-closed\", ());",
        "if let Some(shown) = closed.show { show_late_result(app, shown); }",
    ] {
        assert!(hook.contains(&squeeze(step)), "the hook: {step}\n{hook}");
    }
    let ack = body("fn sign_out_result_ack(");
    assert!(
        ack.contains(&squeeze("state.sign_out.late_results.acknowledge(window.label());")),
        "the acknowledgement names the calling window: {ack}"
    );
    let dialog = body("fn show_late_result(");
    for step in [
        "Shown::Failure(message) => (\"Sign-out paused\", message, tauri_plugin_dialog::MessageDialogKind::Error)",
        "Shown::Warning(sentence) => (\"Signed out\", sentence, tauri_plugin_dialog::MessageDialogKind::Info)",
        ".message(message).title(title).kind(kind).show(",
    ] {
        assert!(dialog.contains(&squeeze(step)), "the dialog: {step}\n{dialog}");
    }
}
/// U19 (macOS half; the frontend half is F8, the Linux case is F8's last line).
#[tokio::test]
async fn the_switch_carries_a_snapshot_on_macos_only() {
    in_own_dirs(async {
            let db = state_db_in_scratch();
            queue_write(&db, "w1", "f1", b"bytes");
            queue_rename(&db, "r1");
            let state = AppState::default();
            let unsent = switch_snapshot(&state).unwrap().expect("a snapshot on macOS");
            assert_eq!((unsent.files, unsent.others), (1, 1));
            assert_eq!(
                state.sign_out.snapshots.newest_ask_id().as_deref(),
                Some(unsent.ask_id.as_str()),
                "recorded as the newest"
            );
            assert!(state.sign_out.phase.lock().unwrap().is_none(), "no send is started");
            let json = serde_json::to_value(LoginOutcome::account_mismatch(2, Some(unsent), None)).unwrap();
            assert_eq!(json["account_mismatch"]["pending_changes"], 2);
            assert!(json["account_mismatch"]["unsent"]["askId"].is_string());
            assert!(
                json["account_mismatch"].get("unsent_error").is_none(),
                "absent while the count worked"
            );
            assert!(
                state
                    .sign_out
                    .snapshots
                    .drop_owned_by(surfaces::policy::main_window_label(surfaces::policy::Platform::current())),
                "owned by the window the switch step runs in (the Beebeeb window on a Mac, not the retired onboarding window)"
            );
            let without = serde_json::to_value(LoginOutcome::account_mismatch(2, None, None)).unwrap();
            assert_eq!(
                without["account_mismatch"],
                serde_json::json!({ "pending_changes": 2 }),
                "elsewhere: unchanged"
            );
        })
        .await;
}

/// Spec §5.3 row 1: a switch count that cannot be read is E3, a line in the switch step. The sign-in settles as a
/// switch, with no snapshot and the coded E3; it is never a failed sign-in.
#[test]
fn a_switch_count_that_fails_is_a_line_in_the_step() {
    let e3 = sign_out::copy::coded(
        sign_out::copy::UNSENT_COUNT_FAILED_CODE,
        sign_out::copy::E3_UNSENT_COUNT_FAILED,
    );
    let SignInSettlement::AccountMismatch {
        pending_changes,
        unsent,
        unsent_error,
    } = switch_settlement(3, Err(e3.clone()))
    else {
        panic!("a switch, not a failure");
    };
    assert_eq!(
        (pending_changes, unsent, unsent_error.as_deref()),
        (3, None, Some(e3.as_str()))
    );
    let json = serde_json::to_value(LoginOutcome::account_mismatch(3, None, Some(e3.clone()))).unwrap();
    assert_eq!(
        json["account_mismatch"],
        serde_json::json!({ "pending_changes": 3, "unsent_error": e3 })
    );
    let production = super::finder_setup_command_tests::production_source();
    let settle = super::finder_setup_command_tests::body_between(&production, "async fn settle_sign_in(", "\n}\n");
    assert!(
        settle.contains("switch_settlement(pending_changes, switch_snapshot(state))"),
        "{settle}"
    );
}

/// U21: the menu never purges a waiting write and never shows the error dialog for it.
#[test]
fn the_menu_never_purges_a_waiting_write() {
    let state = AppState::default();
    state.sign_out.flow_request.store(true, Ordering::SeqCst);
    state.sign_out.flow_request.store(true, Ordering::SeqCst);
    assert!(take_sign_out_flow_request(&state.sign_out), "one request");
    assert!(!take_sign_out_flow_request(&state.sign_out), "not two");
    let production = super::finder_setup_command_tests::production_source();
    let menu = &production[production.find("DesktopMenuAction::SignOut =>").unwrap()..];
    let menu = &menu[..menu.find("DesktopMenuAction::Quit").unwrap()];
    let flow = menu
        .find("Err(error) if error.message.starts_with(\"unsent_changes: \") => open_sign_out_flow(&app),")
        .expect("the flow arm");
    let paused = menu.find(".title(\"Sign-out paused\")").unwrap();
    assert!(flow < paused, "the flow arm comes before the error dialog's arm");
    assert!(
        menu.contains("clear_session_impl(&state, false, None, crate::UnknownFilesArg::Keep, None)"),
        "the menu never keeps on its own"
    );
}

/// F9's Rust half: the compact page's request survives a Settings window that has not loaded yet.
#[test]
fn a_flow_request_waits_for_settings_to_consume_it() {
    let state = AppState::default();
    assert!(!take_sign_out_flow_request(&state.sign_out));
    assert!(
        !state.sign_out.flow_request.load(Ordering::SeqCst),
        "asking when nothing waits leaves nothing"
    );
    state.sign_out.flow_request.store(true, Ordering::SeqCst);
    assert!(take_sign_out_flow_request(&state.sign_out));
}
