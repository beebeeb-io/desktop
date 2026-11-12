/// U8 and U8b through the sign-out: the kept set is kept, one file per file, and then the queue is empty. The keep
/// step's own tests hold its half (names and bytes).
#[tokio::test]
async fn a_sign_out_keeps_the_kept_set_and_empties_the_queue() {
    in_own_dirs(async {
        let db = state_db_in_scratch();
        queue_write(&db, "s1", "report", b"first save");
        queue_write(&db, "s2", "report", b"second save");
        queue_write(&db, "c1", "notes", b"notes");
        let (state, _acct) = signed_out_state();
        recording_finder(&state);
        let id = ask(&state, "main");
        let done = sign_out_with(&state, Some(&id), no_seam()).await.unwrap();
        let kept = done.kept_unsent.expect("a kept folder");
        let folder = Path::new(&kept.folder);
        assert_eq!(kept.files, 2, "two saves of one file keep one file (U8b)");
        assert_eq!(std::fs::read(folder.join("report.bin")).unwrap(), b"second save");
        assert_eq!(std::fs::read(folder.join("notes.bin")).unwrap(), b"notes");
        assert_eq!(
            std::fs::read_dir(folder).unwrap().count(),
            2,
            "no earlier-save copy of a minted save"
        );
        assert!(db.waiting_rows().unwrap().is_empty(), "the queue is empty");
    })
    .await;
}

/// U7.
#[test]
fn the_gate_comes_before_anything_changes() {
    let production = super::finder_setup_command_tests::production_source();
    let clear = super::finder_setup_command_tests::body_between(&production, "async fn clear_session_impl(", "\n}\n");
    let gate = clear.find("gate_sign_out(").expect("the gate");
    for later in [
        "forget_last_signed_in_email(",
        "end_sessions_in_flight(",
        "finder_remove_for(",
        "take_slot_and_stop_engine_for_sign_out(",
        "purge_local_data_for_sign_out(",
    ] {
        assert!(
            gate < clear.find(later).unwrap_or_else(|| panic!("{later}")),
            "the gate comes before {later}"
        );
    }
}

/// U9: the gate counted nothing; a write that arrives before the keep step is kept and counted.
#[tokio::test]
async fn a_write_that_arrives_after_the_count_is_kept_too() {
    in_own_dirs(async {
        let db = state_db_in_scratch();
        let (state, _acct) = signed_out_state();
        recording_finder(&state);
        let seam: SignOutSeam = Arc::new(move |point| {
            if point == "before_keep" {
                let db = state_db_in_scratch();
                queue_write(&db, "late", "late", b"late bytes");
            }
            Ok(())
        });
        let done = sign_out_with(&state, None, seam).await.unwrap();
        let kept = done.kept_unsent.expect("a kept folder");
        assert_eq!(kept.files, 1);
        assert_eq!(
            std::fs::read(Path::new(&kept.folder).join("late.bin")).unwrap(),
            b"late bytes"
        );
        assert_eq!(db.waiting_rows().unwrap().len(), 0, "the queue is empty");
    })
    .await;
}

/// U10 (spec, amended for `184ac962`).
#[test]
fn nothing_that_deletes_runs_between_stop_keep_and_purge() {
    let production = super::finder_setup_command_tests::production_source();
    let clear = super::finder_setup_command_tests::body_between(&production, "async fn clear_session_impl(", "\n}\n");
    let slot = clear
        .find("let mut held_engine_slot = if already_signed_out {")
        .expect("the stop expression");
    let second_stop = clear.rfind("take_slot_and_stop_engine_for_sign_out(").unwrap();
    let close = slot + clear[slot..].find("\n    };\n").expect("the expression's end");
    let keep = clear.find("keep_for_sign_out(").expect("the keep call");
    let purge = clear.find("purge_local_data_for_sign_out(").unwrap();
    assert!(
        second_stop < close && close < keep && keep < purge,
        "stop arms, then `}};`, then keep, then purge"
    );
    let between = &clear[second_stop..purge];
    for denied in [
        "remove_file",
        "remove_dir",
        "purge_all_local_state(",
        "purge_macos_hydrate_cache(",
        "clear_session_holding_slot(",
        "clear_keychain_session(",
        "forget_last_signed_in_email(",
        "start_engine",
        "KeysArrived",
    ] {
        assert!(!between.contains(denied), "{denied} between the stop and the purge");
    }
}

/// U13, the sign-out half: a failed copy leaves the keys in memory and asks the reconciler to restart sync.
#[tokio::test]
async fn a_failed_copy_keeps_the_keys_and_restarts_sync() {
    in_own_dirs(async {
        use std::os::unix::fs::PermissionsExt;
        let db = state_db_in_scratch();
        let staged = queue_write(&db, "w1", "f1", b"bytes");
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o000)).unwrap();
        let state = AppState::default();
        let acct = test_account(&state, "signout-1887-u13");
        let events = recording_finder(&state);
        *acct.session.lock().unwrap() = Some(super::Session {
            token: "t".into(),
            master_key: [7u8; 32],
            email: None,
        });
        set_auth_present(&state, true);
        let id = ask(&state, "main");
        let mut failure = None;
        let logs = capture_logs_async(async {
            failure = Some(sign_out_with(&state, Some(&id), no_seam()).await.unwrap_err());
        })
        .await;
        let failure = failure.unwrap();
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(
            failure.message.starts_with("unsent_copy_failed: "),
            "{}",
            failure.message
        );
        assert!(
            logs.contains("sign-out stopped: the waiting changes could not be kept; nothing was purged"),
            "the sign-out's failed keep is logged as the sign-out's: {logs}"
        );
        assert!(acct.session.lock().unwrap().is_some(), "the keys are in memory");
        // `std::time::Duration`: `lib.rs` imports no `Duration` at the crate level.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            events.lock().unwrap().iter().any(|event| event == "KeysArrived"),
            "sync restarts: {:?}",
            events.lock().unwrap()
        );
        assert_eq!(db.waiting_rows().unwrap().len(), 1);
    })
    .await;
}

/// U29 (b): a copy deleted before the call is a missing write the snapshot did not hold.
#[tokio::test]
async fn a_copy_missing_before_the_call_asks_again() {
    in_own_dirs(async {
        let db = state_db_in_scratch();
        let staged = queue_write(&db, "w1", "f1", b"bytes");
        let (state, _acct) = signed_out_state();
        recording_finder(&state);
        let id = ask(&state, "main");
        std::fs::remove_file(&staged).unwrap();
        let failure = sign_out_with(&state, Some(&id), no_seam()).await.unwrap_err();
        assert!(
            failure.message.starts_with("unsent_changes: "),
            "the gate refuses, the ask is shown again"
        );
        let fresh = count_ask("main").unwrap();
        assert_eq!(fresh.counts.missing_files, 1);
    })
    .await;
}

/// U29 (c), through the sign-out: a copy missing at the ask and held by the snapshot is acknowledged; the sign-out
/// completes, the purge's guard lets the missing write go, and the other write is kept.
#[tokio::test]
async fn a_copy_missing_at_the_ask_is_let_go_and_the_sign_out_completes() {
    in_own_dirs(async {
        let db = state_db_in_scratch();
        let gone = queue_write(&db, "w1", "f1", b"gone");
        queue_write(&db, "w2", "f2", b"kept");
        std::fs::remove_file(&gone).unwrap();
        let (state, _acct) = signed_out_state();
        recording_finder(&state);
        let id = ask(&state, "main");
        let done = sign_out_with(&state, Some(&id), no_seam())
            .await
            .expect("the ask showed the missing write, so the sign-out completes");
        let kept = done.kept_unsent.expect("a kept folder");
        assert_eq!(kept.files, 1, "the other write is kept");
        assert_eq!(std::fs::read(Path::new(&kept.folder).join("f2.bin")).unwrap(), b"kept");
        assert!(db.waiting_rows().unwrap().is_empty(), "the queue is empty");
    })
    .await;
}

/// U30: the record is on disk before the purge, and only the keep step writes it. Two aborts: right after the keep
/// step, both keys name the folder; right after the purge, the rows are gone, so the marker is cleared and the row
/// stays (§7.3, "When it is cleared").
#[tokio::test]
async fn the_kept_folder_is_recorded_before_the_purge() {
    in_own_dirs(async {
        let db = state_db_in_scratch();
        for (point, marker_kept) in [("after_keep", true), ("after_purge", false)] {
            queue_write(&db, "w1", "f1", b"bytes");
            let (state, _acct) = signed_out_state();
            recording_finder(&state);
            let id = ask(&state, "main");
            let seam: SignOutSeam = Arc::new(move |at| {
                if at == point {
                    Err("aborted by the test".into())
                } else {
                    Ok(())
                }
            });
            let failure = sign_out_with(&state, Some(&id), seam).await.unwrap_err();
            let folder = failure.kept_unsent.expect("the failure carries the folder").folder;
            let config = DesktopConfig::load().unwrap();
            assert_eq!(config.kept_unsent_folder.as_deref(), Some(folder.as_str()), "{point}");
            let expected: Vec<String> = if marker_kept { vec![folder.clone()] } else { Vec::new() };
            assert_eq!(
                config.kept_unsent_unfinished, expected,
                "{point}: saved in the keep step's one write; cleared with the rows"
            );
            clean_scratch(&db);
        }
    })
    .await;
    let production = super::finder_setup_command_tests::production_source();
    let clear = super::finder_setup_command_tests::body_between(&production, "async fn clear_session_impl(", "\n}\n");
    assert!(
        !clear.contains("save_kept_unsent_durably("),
        "the Err arm writes neither key; only the keep step does"
    );
    // The keep step's save is its last act, after its last flush: `finish()` (every `sync_all`) comes before it,
    // and nothing after it copies or flushes.
    let keep = include_str!("../sign_out/keep.rs");
    let keep = &keep[..keep.find("#[cfg(test)]\nmod tests").expect("the tests")];
    let save = keep
        .find("save_kept_unsent_durably(inputs.config_path")
        .expect("the save");
    assert!(
        keep.find("writer.finish()").expect("the flushes") < save,
        "the flushes come first"
    );
    let after = &keep[save..];
    assert!(
        !after.contains("writer.keep(") && !after.contains(".finish()") && !after.contains("sync_all"),
        "{after}"
    );
    let writer = include_str!("../../../crates/beebeeb-engine/src/sign_out/kept_writer.rs");
    let finish = &writer[writer.find("pub fn finish(self)").expect("finish")..];
    assert!(
        finish[..finish.find("\n    }\n").unwrap()].contains("sync_all()"),
        "finish flushes"
    );
    assert!(
        clear.find("keep_for_sign_out(").expect("the keep call")
            < clear.find("purge_local_data_for_sign_out(").expect("the purge")
    );
}

/// U32, the macOS half: every new step is macOS only.
#[test]
fn the_unsent_gate_is_macos_only() {
    let production = super::finder_setup_command_tests::production_source();
    let clear = super::finder_setup_command_tests::body_between(&production, "async fn clear_session_impl(", "\n}\n");
    for call in ["gate_sign_out(", "keep_for_sign_out("] {
        let at = clear.find(call).unwrap_or_else(|| panic!("{call}"));
        let before = &clear[..at];
        let cfg = before.rfind("#[cfg(").unwrap();
        assert!(
            before[cfg..].starts_with("#[cfg(target_os = \"macos\")]"),
            "{call} is under cfg(macos)"
        );
    }
    for definition in ["fn gate_sign_out(", "fn keep_for_sign_out(", "fn count_ask("] {
        let at = production.find(definition).unwrap_or_else(|| panic!("{definition}"));
        let before = &production[..at];
        assert!(
            before[before.rfind("#[cfg(").unwrap()..].starts_with("#[cfg(target_os = \"macos\")]"),
            "{definition}"
        );
    }
}

/// U37, the keep step's snapshot: the one the gate accepted, by value, never a newer one.
#[tokio::test]
async fn the_keep_step_uses_the_snapshot_the_gate_accepted() {
    in_own_dirs(async {
        let db = state_db_in_scratch();
        queue_rename(&db, "r1");
        let (state, _acct) = signed_out_state();
        recording_finder(&state);
        let id = ask(&state, "main");
        let store = state.sign_out.snapshots.clone();
        let seam: SignOutSeam = Arc::new(move |point| {
            if point == "before_keep" {
                queue_rename(&state_db_in_scratch(), "r2");
                store.record(count_ask("main").unwrap()); // S′ holds both renames
            }
            Ok(())
        });
        let failure = sign_out_with(&state, Some(&id), seam).await.unwrap_err();
        assert!(failure.message.starts_with("unsent_changes: "), "{}", failure.message);
        assert_eq!(db.waiting_rows().unwrap().len(), 2, "nothing purged");
        // Second run: S′ recorded without changing the queue; the sign-out completes and empties the store.
        db.hold_lock_for_test()
            .execute("DELETE FROM operation_queue WHERE op_id = 'r2'", [])
            .unwrap();
        let id = ask(&state, "main");
        let store = state.sign_out.snapshots.clone();
        let seam: SignOutSeam = Arc::new(move |point| {
            if point == "before_keep" {
                store.record(count_ask("main").unwrap());
            }
            Ok(())
        });
        sign_out_with(&state, Some(&id), seam).await.unwrap();
        assert_eq!(
            state.sign_out.snapshots.newest_ask_id(),
            None,
            "a completed sign-out empties the store"
        );
    })
    .await;
}

/// U41: entry point 7, the already-signed-out arm, keeps before it purges.
#[tokio::test]
async fn an_already_signed_out_sign_out_keeps_before_it_purges() {
    in_own_dirs(async {
        let db = state_db_in_scratch();
        queue_write(&db, "w1", "f1", b"already signed out");
        let (state, _acct) = signed_out_state();
        recording_finder(&state);
        let id = ask(&state, "main");
        let done = sign_out_with(&state, Some(&id), no_seam()).await.unwrap();
        assert_eq!(
            done.outcome,
            SignOutOutcome::NotSignedIn,
            "the already-signed-out arm ran"
        );
        let kept = done.kept_unsent.unwrap();
        assert_eq!(
            std::fs::read(Path::new(&kept.folder).join("f1.bin")).unwrap(),
            b"already signed out"
        );
        assert!(db.waiting_rows().unwrap().is_empty());
    })
    .await;
}

/// U14 through the sign-out: the guard's refusal reaches the caller with its code, and the folder rides it.
#[tokio::test]
async fn a_guard_refusal_reaches_the_caller_with_its_code() {
    in_own_dirs(async {
        let db = state_db_in_scratch();
        queue_write(&db, "w1", "f1", b"bytes");
        let (state, _acct) = signed_out_state();
        recording_finder(&state);
        let id = ask(&state, "main");
        let seam: SignOutSeam = Arc::new(|point| {
            if point == "after_keep" {
                queue_write(&state_db_in_scratch(), "late", "late", b"late"); // only a test seam can add one now
            }
            Ok(())
        });
        let failure = sign_out_with(&state, Some(&id), seam).await.unwrap_err();
        assert!(
            failure.message.starts_with("unsent_copy_failed: "),
            "never wrapped: {}",
            failure.message
        );
        assert!(failure.kept_unsent.is_some(), "the kept folder rides the failure");
        assert_eq!(db.waiting_rows().unwrap().len(), 2, "nothing deleted");
    })
    .await;
}

/// Two sign-outs from one ask (a double click, or the menu's while Settings signs out): one sign-out, one folder.
/// The first one's gate takes the snapshot out of the store, so a second gate on the same ask refuses while the
/// first sign-out is still running, not only after its completion empties the store.
#[tokio::test]
async fn a_second_sign_out_with_the_same_ask_keeps_nothing_twice() {
    in_own_dirs(async {
        let db = state_db_in_scratch();
        queue_write(&db, "w1", "f1", b"once");
        let (state, _acct) = signed_out_state();
        recording_finder(&state);
        let id = ask(&state, "main");
        let (store, same_ask) = (state.sign_out.snapshots.clone(), id.clone());
        let second_gate = Arc::new(std::sync::Mutex::new(None));
        let seen = second_gate.clone();
        let seam: SignOutSeam = Arc::new(move |point| {
            if point == "before_keep" {
                // The second click's gate, on the queue as it is now, while the first sign-out runs.
                let waiting = crate::sign_out::waiting::WaitingChanges::read(&state_db_in_scratch()).unwrap();
                *seen.lock().unwrap() = Some(store.gate(&waiting, Some(&same_ask)));
            }
            Ok(())
        });
        let first = sign_out_with(&state, Some(&id), seam).await.unwrap();
        assert!(first.kept_unsent.is_some());
        assert_eq!(
            *second_gate.lock().unwrap(),
            Some(Err(crate::sign_out::snapshot::GateRefusal)),
            "the first sign-out took the snapshot: a second gate on the same ask refuses while it runs"
        );
        queue_write(&db, "w2", "f2", b"again"); // something waits again, so the stale ask matters
        let second = sign_out_with(&state, Some(&id), no_seam()).await.unwrap_err();
        assert!(second.message.starts_with("unsent_changes: "), "{}", second.message);
        assert_eq!(second.kept_unsent, None, "no second folder");
    })
    .await;
}

/// U39 (b), the ask's half: an interrupted sign-out's folder is named when the ask shows a missing write.
#[tokio::test]
async fn the_ask_names_the_earlier_folders_only_with_a_missing_write() {
    in_own_dirs(async {
        let db = state_db_in_scratch();
        let staged = queue_write(&db, "w1", "f1", b"bytes");
        DesktopConfig::update(|cfg| {
            cfg.kept_unsent_unfinished = vec!["/kept/earlier".into()];
            (true, ())
        })
        .unwrap();
        assert!(
            count_ask("o").unwrap().counts.earlier_folders.is_empty(),
            "no missing write: no A3e"
        );
        std::fs::remove_file(&staged).unwrap();
        assert_eq!(count_ask("o").unwrap().counts.earlier_folders, ["/kept/earlier"]);
        DesktopConfig::update(|cfg| {
            cfg.kept_unsent_unfinished.clear();
            (true, ())
        })
        .unwrap();
    })
    .await;
}

/// The sign-out of a person who is signed in, the usual one: it ends `Completed`, its result carries the kept folder
/// (the alert names it, W9), and it empties the store, also of a newer count another window recorded meanwhile.
#[tokio::test]
async fn a_signed_in_sign_out_completes_with_its_folder_and_empties_the_store() {
    in_own_dirs(async {
        let db = state_db_in_scratch();
        queue_write(&db, "w1", "f1", b"signed in");
        let state = AppState::default();
        let acct = test_account(&state, "signout-1887-completed");
        recording_finder(&state);
        *acct.session.lock().unwrap() = Some(super::Session {
            token: "t".into(),
            master_key: [7u8; 32],
            email: None,
        });
        set_auth_present(&state, true);
        let id = ask(&state, "main");
        let store = state.sign_out.snapshots.clone();
        let seam: SignOutSeam = Arc::new(move |point| {
            if point == "before_keep" {
                store.record(count_ask("onboarding").unwrap()); // another window counts while this one signs out
            }
            Ok(())
        });
        let done = sign_out_with(&state, Some(&id), seam).await.unwrap();
        assert_eq!(done.outcome, SignOutOutcome::Completed, "the signed-in arm ran");
        let kept = done.kept_unsent.expect("the Completed arm carries the kept folder");
        assert_eq!(kept.files, 1);
        assert_eq!(
            std::fs::read(Path::new(&kept.folder).join("f1.bin")).unwrap(),
            b"signed in"
        );
        assert_eq!(
            state.sign_out.snapshots.newest_ask_id(),
            None,
            "a completed signed-in sign-out empties the store"
        );
        assert!(acct.session.lock().unwrap().is_none(), "the keys are cleared");
        assert!(db.waiting_rows().unwrap().is_empty(), "the queue is empty");
    })
    .await;
}

/// W1 and W6 with no ask: nothing waited at the gate, so the sign-out passed no `keepUnsent`. An other change (a
/// rename, a move, a new folder or a deletion) that arrives before the keep step stops the sign-out with
/// `unsent_changes`, so the ask is shown, and nothing is purged. (A write that arrives then is kept: U9.)
#[tokio::test]
async fn a_late_other_change_with_no_ask_stops_the_sign_out() {
    in_own_dirs(async {
        let db = state_db_in_scratch();
        let (state, _acct) = signed_out_state();
        recording_finder(&state);
        let seam: SignOutSeam = Arc::new(|point| {
            if point == "before_keep" {
                queue_rename(&state_db_in_scratch(), "late-rename");
            }
            Ok(())
        });
        let result = sign_out_with(&state, None, seam).await;
        assert_eq!(
            db.waiting_rows().unwrap().len(),
            1,
            "the late rename is not purged unasked: {result:?}"
        );
        let failure = result.expect_err("the sign-out stops");
        assert_eq!(
            failure.message,
            coded(UNSENT_CHANGES_CODE, E1_UNSENT_CHANGES),
            "the ask is shown again"
        );
        assert_eq!(failure.kept_unsent, None, "nothing was kept");
    })
    .await;
}

/// `clear_session` hands `keepUnsent: { askId }` on to the gate (spec §5.1, the contract). A unit test cannot build
/// the command's `AppHandle`, so this checks the name the webview sends and the command's call.
/// Task 2191 (review thread on #157): a Windows sign-out that fails after its purge kept
/// copies names their folder: every failure from the purge on carries it.
#[test]
fn task_2191_a_failure_after_the_purge_names_the_kept_folder() {
    let kept = Some(super::KeptUnsent {
        folder: "C:\\Beebeeb kept files".into(),
        files: 3,
    });
    let failure =
        super::sign_out_failure_after_purge("Could not unregister Cloud Files: x".into(), &None, &None, &kept);
    assert_eq!(failure.kept_unsent, kept);
    let production = super::finder_setup_command_tests::production_source();
    let body = crate::source_pin::squeeze(super::finder_setup_command_tests::body_between(
        &production,
        "async fn clear_session_impl(",
        "\n}\n",
    ));
    let after = &body[body.find("windows_cf::signout::purge(").expect("the purge")..];
    // Task 1886: the logout request is no step that can fail the sign-out any more (it is best effort on every OS,
    // `end_dropped_sessions`), so its "Could not initialize logout request" failure is gone.
    for step in [
        "Could not unregister Cloud Files",
        "Could not remove Explorer registration",
    ] {
        assert!(
            after.contains(&crate::source_pin::squeeze(&format!(
                "failed_after_purge(format!(\"{step}"
            ))),
            "{step} carries the kept folder"
        );
    }
    assert!(
            after.contains(&crate::source_pin::squeeze(
                "let kept_with = |message: String| sign_out_failure_after_purge(message, &preserved_location, &kept_unsent, &windows_kept);"
            )),
            "the later failures carry it too"
        );
    assert!(after.contains(&crate::source_pin::squeeze(
        "clear_keychain_session_into(acct.id.as_str(), &mut dropped).map_err(&kept_with)?;"
    )));
}

#[test]
fn clear_session_hands_keep_unsent_on_to_the_gate() {
    let arg: KeepUnsentArg =
        serde_json::from_value(serde_json::json!({ "askId": "a1" })).expect("the webview sends askId");
    assert_eq!(arg.ask_id, "a1");
    assert!(
        serde_json::from_value::<KeepUnsentArg>(serde_json::json!({ "ask_id": "a1" })).is_err(),
        "askId is the only name it takes"
    );
    let production = super::finder_setup_command_tests::production_source();
    let command = crate::source_pin::squeeze(super::finder_setup_command_tests::body_between(
        &production,
        "async fn clear_session(",
        "\n}\n",
    ));
    for expected in [
        // Tauri names the argument after the parameter, in camelCase: `keepUnsent`.
        "keep_unsent: Option<KeepUnsentArg>",
        "clear_session_impl(&state, forget_email.unwrap_or(false), keep_unsent.as_ref().map(|keep| keep.ask_id.as_str()),",
        // Task 2191: and the Windows answer for files of unknown content, kept when absent.
        "unknown_files.unwrap_or(UnknownFilesArg::Keep),",
        "kept_folder.map(std::path::PathBuf::from))",
    ] {
        assert!(
            command.contains(&crate::source_pin::squeeze(expected)),
            "clear_session passes keepUnsent on: {expected}\n{command}"
        );
    }
}

/// Spec §5.3, "4. Gate | Read fails | E3 | Nothing changed": a gate that cannot read the queue answers E3 before the
/// Finder removal and the engine stop, and the keys stay in memory.
#[tokio::test]
async fn a_gate_that_cannot_read_the_queue_changes_nothing() {
    in_own_dirs(async {
        // A directory where `state.db` should be: it exists, and it cannot be opened.
        let dir = state_paths::beebeeb_state_dir().unwrap();
        std::fs::create_dir_all(state_paths::state_db_path_from_state_dir(&dir)).unwrap();
        let state = AppState::default();
        let acct = test_account(&state, "signout-1887-unread");
        let events = recording_finder(&state);
        *acct.session.lock().unwrap() = Some(super::Session {
            token: "t".into(),
            master_key: [7u8; 32],
            email: None,
        });
        set_auth_present(&state, true);
        let failure = sign_out_with(&state, None, no_seam()).await.unwrap_err();
        assert!(
            events.lock().unwrap().iter().all(|event| event != "remove"),
            "the reconciler got no removal: {:?} ({})",
            events.lock().unwrap(),
            failure.message
        );
        assert_eq!(
            failure.message,
            coded(UNSENT_COUNT_FAILED_CODE, E3_UNSENT_COUNT_FAILED),
            "E3 with its code"
        );
        assert!(acct.session.lock().unwrap().is_some(), "the keys are in memory");
    })
    .await;
}

/// The keep step opens `state.db` itself. When it cannot, the queue cannot be read: it answers E3 with its code, as a
/// failed read inside the keep step does, never the database's error, whose text names the database's absolute
/// path; and its log names the step, never the path (W10).
#[tokio::test]
async fn a_keep_step_that_cannot_open_the_queue_answers_e3_and_names_no_path() {
    in_own_dirs(async {
        let dir = state_paths::beebeeb_state_dir().unwrap();
        std::fs::create_dir_all(state_paths::state_db_path_from_state_dir(&dir)).unwrap();
        let mut kept = None;
        let logs = capture_logs(|| kept = Some(keep_for_sign_out(None)));
        let error = match kept.expect("the keep step ran") {
            Ok(_) => panic!("a state.db that cannot be opened keeps nothing"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            coded(UNSENT_COUNT_FAILED_CODE, E3_UNSENT_COUNT_FAILED),
            "E3 with its code"
        );
        let path = dir.to_string_lossy().into_owned();
        assert!(!logs.contains(&path), "no log carries the path:\n{logs}");
        let line = logs
            .lines()
            .find(|line| line.contains("the waiting changes could not be read"))
            .unwrap_or_else(|| panic!("the stop is logged:\n{logs}"));
        assert!(line.contains("step=state_db"), "the log names the step: {line}");
    })
    .await;
}

/// An ask that shows a missing write names the earlier folders (A3e). When the list cannot be read, the ask still
/// counts, without them, and its warning names the step that failed, never the error: a read error quotes a line of
/// the file, and the list's lines are kept folders' paths (W10).
#[tokio::test]
async fn an_ask_whose_unfinished_list_cannot_be_read_still_counts_and_logs_the_step() {
    in_own_dirs(async {
        let db = state_db_in_scratch();
        let staged = queue_write(&db, "w1", "f1", b"bytes");
        std::fs::remove_file(&staged).unwrap(); // a missing write: the ask reads the list
        let config = DesktopConfig::path().unwrap();
        // A line the parser stops at, and it names a kept folder (a hand edit, say).
        std::fs::write(
            &config,
            "kept_unsent_unfinished = [\"/kept/earlier\"]\nnote = \"/kept/earlier\" and more\n",
        )
        .unwrap();
        let mut counted = None;
        let logs = capture_logs(|| counted = Some(count_ask("o")));
        let snapshot = counted.expect("the count ran").expect("the ask still counts");
        assert_eq!(snapshot.counts.missing_files, 1);
        assert!(snapshot.counts.earlier_folders.is_empty(), "no folder could be read");
        let warning = logs
            .lines()
            .find(|line| line.contains("the unfinished sign-out list could not be read"))
            .unwrap_or_else(|| panic!("the failed read is logged:\n{logs}"));
        assert!(
            !logs.contains("/kept/earlier") && !logs.contains(&*config.to_string_lossy()),
            "no log carries a kept folder or the file's path:\n{logs}"
        );
        assert!(warning.contains("step=read"), "the warning names the step: {warning}");
    })
    .await;
}

/// U45 (spec 2026-10-10 §15 question 3): every sign-out that ends without purging leaves Finder accepts open at
/// once.
#[tokio::test]
async fn a_cancelled_or_failed_sign_out_leaves_accepts_open() {
    in_own_dirs(async {
        use std::os::unix::fs::PermissionsExt;
        let db = state_db_in_scratch();
        let staged = queue_write(&db, "w1", "f1", b"bytes");
        let (state, _acct) = signed_out_state();
        recording_finder(&state);
        // Cancel: the person was asked and chose Cancel, so no `clear_session` ran at all. The ask itself closes
        // nothing.
        ask(&state, "main");
        assert!(
            !db.finder_accepts_closed().unwrap(),
            "a cancelled sign-out: the ask closes nothing"
        );
        // The gate refuses: changes wait and no `keepUnsent`.
        assert!(sign_out_with(&state, None, no_seam()).await.is_err());
        assert!(!db.finder_accepts_closed().unwrap(), "refused at the gate");
        // The keep step fails: a staged copy it cannot read.
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o000)).unwrap();
        let id = ask(&state, "main");
        let failed = sign_out_with(&state, Some(&id), no_seam()).await;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(failed.unwrap_err().message.starts_with("unsent_copy_failed: "));
        assert!(!db.finder_accepts_closed().unwrap(), "a failed keep");
        // The purge guard refuses after a completed keep (only a test seam can add a write then). The seam records
        // that the keep step left accepts closed, so the reopen checked last is the guard's.
        let id = ask(&state, "main");
        let closed_after_the_keep = Arc::new(std::sync::Mutex::new(None));
        let seen = closed_after_the_keep.clone();
        let seam: SignOutSeam = Arc::new(move |point| {
            if point == "after_keep" {
                let db = state_db_in_scratch();
                *seen.lock().unwrap() = Some(db.finder_accepts_closed().unwrap());
                queue_write(&db, "late", "late", b"late");
            }
            Ok(())
        });
        let refused = sign_out_with(&state, Some(&id), seam).await.unwrap_err();
        assert_eq!(
            *closed_after_the_keep.lock().unwrap(),
            Some(true),
            "the completed keep left accepts closed for the purge"
        );
        assert_eq!(
            refused.message,
            coded(UNSENT_COPY_FAILED_CODE, E2_UNSENT_COPY_FAILED),
            "the purge guard refused (E2), never wrapped"
        );
        assert!(
            refused.kept_unsent.is_some(),
            "a refusal after a completed keep carries its folder"
        );
        assert!(!db.finder_accepts_closed().unwrap(), "a refused purge");
    })
    .await;
}

/// U45: the rule holds whatever the keep step kept. With nothing waiting, the keep step still closes accepts (an
/// accept could commit after its read), and a return before the purge reopens them, as after a keep that made a
/// folder.
#[tokio::test]
async fn a_sign_out_that_kept_nothing_and_stops_reopens_accepts() {
    in_own_dirs(async {
        let db = state_db_in_scratch();
        let (state, _acct) = signed_out_state();
        recording_finder(&state);
        let closed_after_the_keep = Arc::new(std::sync::Mutex::new(None));
        let seen = closed_after_the_keep.clone();
        let seam: SignOutSeam = Arc::new(move |point| {
            if point == "after_keep" {
                *seen.lock().unwrap() = Some(state_db_in_scratch().finder_accepts_closed().unwrap());
                return Err("stopped by the test".to_string());
            }
            Ok(())
        });
        let failure = sign_out_with(&state, None, seam).await.unwrap_err();
        assert_eq!(
            (failure.message.as_str(), &failure.kept_unsent),
            ("stopped by the test", &None),
            "precondition: stopped right after a keep that kept nothing"
        );
        assert_eq!(
            *closed_after_the_keep.lock().unwrap(),
            Some(true),
            "the keep step closed accepts, though nothing waited"
        );
        assert!(
            !db.finder_accepts_closed().unwrap(),
            "a sign-out that kept nothing and stopped reopens accepts"
        );
    })
    .await;
}

/// U45: a quit between the keep step and the purge leaves the key on disk; the next launch reopens accepts first,
/// before the restore can start an engine. The reopen is the first statement of `restore_session_inner`, never in
/// the wrapper, whose two statements the startup-restore pin holds exact.
#[test]
fn accepts_reopen_when_the_app_starts() {
    let dir = tempfile::tempdir().unwrap();
    let db = state_db::StateDb::open(state_paths::state_db_path_from_state_dir(dir.path())).unwrap();
    db.close_finder_accepts().unwrap();
    reopen_finder_accepts_in(dir.path());
    assert!(!db.finder_accepts_closed().unwrap());
    let production = super::finder_setup_command_tests::production_source();
    let restore =
        super::finder_setup_command_tests::body_between(&production, "async fn restore_session_inner(", "\n}\n");
    let reopen = restore
        .find("reopen_finder_accepts_at_startup()")
        .expect("the launch reopens accepts");
    assert!(
        reopen < restore.find("restore_stored_session(").expect("the restore"),
        "before the restore"
    );
    assert!(
        reopen < restore.find("start_engine_if_possible(").expect("the engine start"),
        "before any engine can start"
    );
    let first = restore
        .lines()
        .skip(1)
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("//"))
        .unwrap();
    assert_eq!(
        first, "#[cfg(target_os = \"macos\")]",
        "the reopen opens the body:\n{restore}"
    );
}
