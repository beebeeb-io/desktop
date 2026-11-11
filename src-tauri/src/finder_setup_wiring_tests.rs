use super::finder_setup_command_tests::{body_between, macos_compiled, production_source, without_items};

/// The name of the top-level function that contains byte `at`.
fn enclosing_fn(source: &str, at: usize) -> String {
    let head = &source[..at];
    let start = [
        "\nfn ",
        "\nasync fn ",
        "\npub(crate) async fn ",
        "\npub(crate) fn ",
        "\npub fn ",
        "\npub async fn ",
    ]
    .iter()
    .filter_map(|prefix| head.rfind(prefix).map(|i| i + prefix.len()))
    .max()
    .expect("inside a function");
    source[start..].split('(').next().unwrap().trim().to_string()
}

/// Spec §5.3 (2), corrected by plan "Spec issues" 3: keys arrive in exactly these functions.
#[test]
fn every_function_that_installs_keys_tells_the_finder_reconciler() {
    let production = production_source();
    let mut installers: Vec<String> = Vec::new();
    for marker in ["*guard = Some(Session {", "*guard = Some(session)"] {
        let mut from = 0;
        while let Some(found) = production[from..].find(marker) {
            installers.push(enclosing_fn(&production, from + found));
            from += found + marker.len();
        }
    }
    installers.sort();
    installers.dedup();
    assert_eq!(
        installers,
        vec![
            "install_new_session",
            "install_recovered_session",
            "install_unlocked_session",
            "restore_stored_session"
        ],
        "a new place that puts keys in memory must tell the Finder reconciler (and be added here)"
    );
    // The checked installs (Task 12) have one production caller each, which tells the reconciler (below).
    assert_eq!(
        production.matches("install_new_session(").count() - 1,
        1,
        "install_new_session_or_revoke alone calls it"
    );
    assert!(
        body_between(&production, "async fn install_new_session_or_revoke(", "\n}\n").contains("install_new_session(")
    );
    assert_eq!(
        production.matches("install_new_session_or_revoke(").count() - 1,
        1,
        "apply_session alone calls the revoking install"
    );
    assert!(
        body_between(&production, "pub(crate) async fn apply_session(", "\n}\n")
            .contains("install_new_session_or_revoke(")
    );
    assert_eq!(
        production.matches("install_recovered_session(").count() - 1,
        1,
        "the recovery-phrase unlock alone calls it"
    );
    assert!(
        body_between(&production, "async fn desktop_unlock_with_recovery_phrase(", "\n}\n")
            .contains("install_recovered_session(")
    );
    // install_unlocked_session's production callers: unlock_vault and reauth_swap_token (R8), which
    // reauth_in_place calls before its own engine start and keys_arrived.
    assert_eq!(
        production.matches("install_unlocked_session(").count() - 1,
        2,
        "callers, not counting its definition"
    );
    for (signature, engine_starts) in [
        ("pub(crate) async fn apply_session(", 1),
        ("async fn desktop_unlock_with_recovery_phrase(", 2),
        ("async fn unlock_vault(", 2),
        ("async fn reauth_in_place(", 1),
    ] {
        let body = body_between(&production, signature, "\n}\n");
        assert_eq!(
            body.matches("start_engine_if_possible(").count(),
            engine_starts,
            "{signature}"
        );
        assert_eq!(
            body.matches("keys_arrived(").count(),
            engine_starts,
            "{signature}: every keys-arrive branch tells the reconciler"
        );
    }
    // The startup restore is covered by the launch trigger (Task 8's test pins its order).
    // The two login commands store only a token: they never install keys.
    for signature in ["async fn desktop_login(", "async fn desktop_login_2fa("] {
        let body = body_between(&production, signature, "\n}\n");
        assert!(
            !body.contains("Some(Session {") && !body.contains("master_key"),
            "{signature} installs keys now: wire keys_arrived"
        );
    }
}

/// Spec §8 logs "sign-in completed", not an unlock after a Lock (area A M11): the Keychain unlock tells the
/// reconciler keys are here and writes no `signed_in` line; every sign-in path still does.
#[test]
fn only_a_sign_in_writes_signed_in_to_the_lifecycle_log() {
    let production = production_source();
    let arrived = body_between(&production, "fn keys_arrived(", "\n}\n");
    let logged = arrived
        .find("lifecycle_log::event(lifecycle_log::LifecycleEvent::SignedIn);")
        .expect("a sign-in is logged");
    assert!(
        arrived[..logged].contains("if from == KeysFrom::SignIn {"),
        "only a sign-in:\n{arrived}"
    );
    let unlock = body_between(&production, "async fn unlock_vault(", "\n}\n");
    assert_eq!(
        unlock.matches("keys_arrived(&state, KeysFrom::Unlock)").count(),
        2,
        "{unlock}"
    );
    for signature in [
        "pub(crate) async fn apply_session(",
        "async fn desktop_unlock_with_recovery_phrase(",
        "async fn reauth_in_place(",
    ] {
        let body = body_between(&production, signature, "\n}\n");
        assert!(
            !body.contains("KeysFrom::Unlock") && body.contains("KeysFrom::SignIn"),
            "{signature} is a sign-in"
        );
    }
}

#[test]
fn sign_out_lock_and_repair_reach_the_reconciler_before_the_engine_stops() {
    let production = production_source();
    let clear = body_between(&production, "async fn clear_session_impl(", "\n}\n");
    let remove = clear
        .find("finder_remove_for(state, finder_setup::core::Trigger::SignOut)")
        .expect("sign-out tells the reconciler");
    // The first engine stop in the body (the already-signed-out path's): the signed-in path's comes later.
    let engine = clear
        .find("take_slot_and_stop_engine_for_sign_out(&acct)")
        .expect("sign-out stops the engine");
    assert!(remove < engine, "the reconciler is told before the engine stops");
    assert!(
        !clear.contains("remove_file_provider_domain()"),
        "no second, direct removal"
    );
    let lock = body_between(&production, "async fn lock_vault(", "\n}\n");
    assert!(
        lock.find("finder_lock_for(&state)").expect("lock tells the reconciler")
            < lock.find("acct.engine.lock().await").unwrap(),
        "the reconciler holds before the engine stops"
    );
    let repair = body_between(&production, "async fn reset_macos_integration(", "\n}\n");
    assert!(repair.contains("finder_remove_for(&state, finder_setup::core::Trigger::Repair)"));
    assert!(
        !macos_compiled(repair).contains("remove_file_provider_domain("),
        "Repair removes through the reconciler on a Mac"
    );
}

/// Lead rulings T8-⚠lock (2) and T10-lockorder: the session is cleared BEFORE the engine slot is released, and
/// the clear is a function that needs the slot guard, so the other order does not compile. Source pins for the
/// shape (the compile-fail mutations are in the fix-round report): the helper takes the guard, it holds the only
/// clear, and Lock and Sign-out call it before their one release.
#[test]
fn lock_and_sign_out_clear_the_session_only_through_the_helper_that_needs_the_slot() {
    let production = production_source();
    let helper = body_between(&production, "fn clear_session_holding_slot(", "\n}\n");
    assert!(
        helper.contains("MutexGuard<'_, Option<EngineRunner>>"),
        "the helper needs the slot guard:\n{helper}"
    );
    assert!(
        helper.contains("guard.take();") && helper.contains("poisoned.into_inner().take();"),
        "{helper}"
    );

    let lock = body_between(&production, "async fn lock_vault(", "\n}\n");
    assert_eq!(
        lock.matches("clear_session_holding_slot(&acct, &engine_slot, account::SessionTransition::Lock)?;")
            .count(),
        1,
        "lock clears through the helper:\n{lock}"
    );
    assert!(
        !lock.contains("guard.take();") && !lock.contains(".into_inner().take()"),
        "no second way to clear:\n{lock}"
    );
    assert_eq!(
        lock.matches("drop(engine_slot)").count(),
        1,
        "one release of the slot:\n{lock}"
    );
    assert!(
        lock.find("clear_session_holding_slot(").unwrap() < lock.find("drop(engine_slot)").unwrap(),
        "the slot is released after the session is cleared:\n{lock}"
    );

    let clear = body_between(&production, "async fn clear_session_impl(", "\n}\n");
    assert_eq!(
            clear
                .matches(
                    "clear_session_holding_slot(&acct, &engine_slot, account::SessionTransition::SignOut).map_err(&kept_with)?;",
                )
                .count(),
            1,
            "sign-out clears through the helper:\n{clear}"
        );
    assert!(
        !clear.contains("guard.take();") && !clear.contains(".into_inner().take()"),
        "no second way to clear:\n{clear}"
    );
    // Releases of a slot named `engine_slot`: Windows' release after the clear; and (Task 12 fix round 1) on macOS and
    // Linux one in each branch AFTER the sign-out's last turn (`finish_sign_out_turn`, which clears memory once more).
    // The refusal for an unconfirmed earlier stop is the stop helper's (fix round 2, item 1).
    assert_eq!(clear.matches("drop(engine_slot)").count(), 3, "{clear}");
    for after_the_last_turn in [
        "finish_sign_out_turn(&acct, &engine_slot, true, &mut dropped).map_err(&kept_with)?;",
        "finish_sign_out_turn(&acct, &engine_slot, false, &mut dropped).map_err(&kept_with)?;",
    ] {
        let at = clear
            .find(after_the_last_turn)
            .unwrap_or_else(|| panic!("{after_the_last_turn}:\n{clear}"));
        let next_code = clear[at + after_the_last_turn.len()..]
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with("//"))
            .unwrap_or_default();
        assert_eq!(
            next_code, "drop(engine_slot);",
            "the slot is released right after the last turn:\n{clear}"
        );
    }
    let stop = body_between(&production, "async fn take_slot_and_stop_engine_for_sign_out(", "\n}\n");
    let refusal = &stop[stop.find("drop(engine_slot)").expect("the refusal releases the slot")..];
    assert!(
        refusal[..refusal.find("return Err(").expect("the refusal returns")]
            .lines()
            .count()
            < 8,
        "the first release is the refusal, which returns at once:\n{refusal}"
    );
    assert_eq!(
        stop.matches("drop(engine_slot)").count(),
        1,
        "the helper releases the slot only to refuse:\n{stop}"
    );
    // Both paths hold the slot from before the purge (fix round 2, item 1).
    let purge = clear.find("purge_local_data_for_sign_out(").expect("the purge");
    // (Task 1882 rebase: the call now ends `.map_err(&kept_with)?`, over several lines; both are found.)
    let takes: Vec<usize> = clear
        .match_indices("take_slot_and_stop_engine_for_sign_out(&acct)")
        .map(|(at, _)| at)
        .collect();
    assert_eq!(takes.len(), 2, "both paths:\n{clear}");
    for at in takes {
        assert!(at < purge, "the slot is taken before the purge:\n{clear}");
    }
    assert!(
        clear.rfind("drop(engine_slot)").unwrap() > clear.find("clear_session_holding_slot(").unwrap(),
        "the slot is released after the session is cleared:\n{clear}"
    );
    // Declared from the stop helper's slot on either path, then taken for the clear: any other use could release it
    // early. (Comments may name it.)
    let code: String = clear
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        code.matches("held_engine_slot").count(),
        2,
        "the held slot is touched in two places only:\n{clear}"
    );
}

/// Fix round 1 (item C): a poisoned session mutex still guards the master key. The one clear recovers the
/// guard, clears the session and clears the poison; neither Lock nor Sign-out skips it and returns `Ok`.
#[test]
fn a_poisoned_session_mutex_is_recovered_and_cleared_by_lock_and_sign_out() {
    let production = production_source();
    let helper = body_between(&production, "fn clear_session_holding_slot(", "\n}\n");
    assert!(
        helper.contains("poisoned.into_inner().take();"),
        "the helper does not clear a poisoned session"
    );
    assert!(
        helper.contains("acct.session.clear_poison();"),
        "the helper leaves the mutex poisoned"
    );
    assert!(
        !helper.contains("session mutex poisoned during"),
        "the helper only warns:\n{helper}"
    );
    for signature in ["async fn lock_vault(", "async fn clear_session_impl("] {
        let body = body_between(&production, signature, "\n}\n");
        assert!(
            body.contains("clear_session_holding_slot("),
            "{signature} does not clear through the helper"
        );
    }
}

/// Lead ruling T8-lockflag (3): every place that puts an engine in the slot refuses while an
/// earlier stop is unconfirmed, so two engines never run at once. Since R10 (ruling T9-starts) every
/// start goes through `spawn_bound_engine`, so the sites are the functions that call it; each takes
/// the slot, then refuses, then starts. The gate reads the keys through `keys_for_engine_start`, which
/// refuses first as well.
#[test]
fn every_engine_start_refuses_while_a_stop_is_unconfirmed() {
    let production = production_source();
    let gate_call = "spawn_bound_engine(";
    let mut sites: Vec<String> = production
        .match_indices(gate_call)
        .filter(|(at, _)| {
            let line = production[..*at].rsplit('\n').next().unwrap_or("");
            let line_start = line.trim_start();
            !line_start.starts_with("//") && !line_start.contains("fn ")
        })
        .map(|(at, _)| enclosing_fn(&production, at))
        .collect();
    sites.sort();
    sites.dedup();
    assert_eq!(
        sites,
        vec![
            "ensure_sync_root_and_engine",
            "persist_sync_root_and_start_engine",
            "pick_sync_root",
            "start_check_engine",
            "start_engine_for_pending_finder_install",
            "start_engine_if_possible",
            "start_engine_unless_running",
        ],
        "a new engine start must refuse while an earlier stop is unconfirmed (and be added here)"
    );
    for name in &sites {
        let body = body_between(&production, &format!("fn {name}("), "\n}\n");
        let guarded = body
            .find("engine_start_refusal(")
            .unwrap_or_else(|| panic!("{name} does not call engine_start_refusal("));
        let slot = ["engine.lock().await", "lock_engine_slot("]
            .iter()
            .filter_map(|take| body.find(take))
            .min()
            .unwrap_or_else(|| panic!("{name} takes the engine slot"));
        let spawned = body.find(gate_call).expect("it starts through the gate");
        assert!(
            slot < guarded && guarded < spawned,
            "{name}: slot, then the refusal, then the start:\n{body}"
        );
    }
    let refusal = body_between(&production, "fn keys_for_engine_start(", "\n}\n");
    let flag = refusal
        .find("engine_start_refusal(")
        .expect("the gate's key read refuses first");
    assert!(
        flag < refusal.find(".lock()").expect("and then reads the keys"),
        "{refusal}"
    );
}

/// Lead ruling T8-gate-all (4): every File Provider bridge call goes through the one gate. The
/// bridge module itself and the gate's own file are the only places that name a bridge function;
/// everything else calls a gated wrapper in `macos_ports`.
#[test]
fn no_file_provider_bridge_call_is_made_outside_the_bridge_gate() {
    const BRIDGE: [&str; 8] = [
        "remove",
        "add_domain",
        "domain_state",
        "wait_for_domain_ready",
        "resolve_location",
        "resolve_item",
        "cleanup_stale_domains",
        "signal_working_set",
    ];
    fn rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("read the source dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    assert!(
        files.len() > 20,
        "the census sees the source tree: {} files",
        files.len()
    );
    let (mut gated, mut checked_lines) = (0, 0);
    for file in files {
        let name = file.strip_prefix(&src).unwrap().to_string_lossy().replace('\\', "/");
        if name == "macos_file_provider.rs" {
            continue; // the bridge itself
        }
        if name == "tests.rs"
            || name.ends_with("/tests.rs")
            || name.ends_with("_tests.rs")
            || name.split('/').any(|part| part == "tests")
        {
            continue;
        }
        let text = std::fs::read_to_string(&file)
            .expect("read a source file")
            .replace("\r\n", "\n");
        let production = without_items(&text, &["#[cfg(test)]", "#[cfg(all(test"]);
        let production_lines: Vec<&str> = production.lines().collect();
        for (number, line) in production.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            checked_lines += 1;
            for (at, _) in line.match_indices("macos_file_provider::") {
                let ident: String = line[at + "macos_file_provider::".len()..]
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !BRIDGE.contains(&ident.as_str()) {
                    continue;
                }
                assert!(
                    name == "finder_setup/macos_ports.rs",
                    "{name}:{}: calls the bridge outside the gate: {line}",
                    number + 1
                );
                // The open and the reveal pass their resolve to `resolve_then` together with the shared gate (task 1885
                // fix round 1, I1); `resolve_then` takes the gate for it (pinned in `macos_ports`), and the open that
                // follows is not a File Provider call.
                let given_to_resolve_then_with_the_shared_gate = line.contains("resolve_then(&BridgeGate::shared(),")
                    || (number >= 2
                        && production_lines[number - 1].trim() == "&BridgeGate::shared(),"
                        && production_lines[number - 2].trim_end().ends_with("resolve_then("));
                assert!(
                    line.contains(".run(")
                        || line.contains(".run_sync(")
                        || line.contains(".run_waiting(")
                        || line.contains(".run_sync_waiting(")
                        || given_to_resolve_then_with_the_shared_gate,
                    "{name}:{}: a bridge call that does not take the gate: {line}",
                    number + 1
                );
                gated += 1;
            }
        }
    }
    assert!(
        checked_lines > 10_000,
        "the census read the production code: {checked_lines} lines"
    );
    assert_eq!(
        gated, 10,
        "the gate's callers: the reconciler's five bridge operations and the five wrappers for everyone else \
             (open, reveal, domain state, working-set signal, stale-domain sweep)"
    );
}

#[test]
fn becoming_active_and_updates_reach_the_reconciler_and_the_log() {
    let production = production_source();
    let run = body_between(&production, "pub fn run() {", "\n}\n");
    assert!(run.contains("tauri::WindowEvent::Focused(true) = event") && run.contains("handle.app_activated()"));
    for signature in ["async fn install_update(", "async fn install_channel_downgrade("] {
        let body = body_between(&production, signature, "\n}\n");
        // Tasks 2449/2451: both commands install through the one record-first path, which writes both events.
        assert!(body.contains("flow::download_record_install("), "{signature}");
    }
    // A Windows checkout may hold CRLF line endings; `production_source` already reads them as LF.
    let flow = include_str!("update_window/flow.rs").replace("\r\n", "\n");
    let install = body_between(&flow, "async fn download_record_install(", "\n}\n");
    assert!(install.contains("LifecycleEvent::UpdateDownloaded"));
    assert!(install.contains("LifecycleEvent::UpdateInstalled"));
}

/// F7 follow-up 2 (f7b concern 4): in `Ready` the user-enabled poll sets no timer while no window is visible, and
/// `Focused(true)` resumes it (`becoming_active_and_updates_reach_the_reconciler_and_the_log`). That holds only if
/// a window that becomes visible is also focused, so every `show()` of an app window is followed, in the same
/// block, by `set_focus()` on the same window. A `show()` without it would leave that window unpolled until the
/// person clicks it. Notifications and message dialogs also call `show`; they are not windows, and they are
/// counted here so that a `show()` of a shape the pin does not know is a red test, not a site it cannot see.
#[test]
fn every_window_shown_is_also_focused_so_the_ready_poll_resumes() {
    let production = production_source();
    let lines: Vec<(usize, &str)> = production
        .split_inclusive('\n')
        .scan(0, |offset, line| {
            let at = *offset;
            *offset += line.len();
            Some((at, line.trim_end_matches('\n')))
        })
        .collect();
    let indent = |line: &str| line.len() - line.trim_start().len();
    let (mut windows, mut notifications, mut dialogs, mut unhides) = (Vec::new(), 0, 0, 0);
    for (i, &(at, line)) in lines.iter().enumerate() {
        let code = line.trim();
        if code.starts_with("//") || !code.contains(".show(") {
            continue;
        }
        let site = enclosing_fn(&production, at);
        // Task 1683 slice 1 (review round 2, P2-2): unhiding the app (`AppHandle::show`) is not a window; it lives
        // in one helper, which every macOS opener calls before it shows (and focuses) its window.
        if site == "unhide_app" && code == "let _ = app.show();" {
            unhides += 1;
            continue;
        }
        // A parentless dialog: the `.show(|…| …)` line of a chain, or a whole chain on one line
        // (`app.dialog()…show(|_| {});`, which rustfmt keeps when it is short).
        if code.starts_with(".show(|") || (code.starts_with("app.dialog().") && code.contains(".show(|")) {
            dialogs += 1;
            continue;
        }
        // A notification builder: the method chain this `.show()` ends starts at `app.notification()`.
        let chain_starts_at_a_notification = lines[..i]
            .iter()
            .rev()
            .map(|&(_, l)| l.trim())
            .take_while(|l| !(l.is_empty() || l.ends_with(';') || l.ends_with('{') || l.ends_with('}')))
            .any(|l| l.contains(".notification()"));
        if code == ".show()" && chain_starts_at_a_notification {
            notifications += 1;
            continue;
        }
        let window = code
            .strip_prefix("let _ = ")
            .and_then(|rest| rest.strip_suffix(".show();"))
            .unwrap_or_else(|| panic!("{site}: a show() this pin does not know; is it a window? {code}"));
        let focus = format!("let _ = {window}.set_focus();");
        let focused = lines[i + 1..]
            .iter()
            .map(|&(_, l)| l)
            .filter(|l| !l.trim().is_empty())
            .take_while(|l| indent(l) >= indent(line))
            .any(|l| indent(l) == indent(line) && l.trim() == focus);
        assert!(
            focused,
            "{site}: `{code}` is not followed by `{focus}` in the same block, so the Ready poll would not resume"
        );
        windows.push(site);
    }
    // 12 when f7b counted them (2026-10-09). Task 1683 slice 1 replaced the Settings window's two with the Beebeeb
    // window's two (`show_main_window`) and added the reopen's (`reopen_main_window`): 13.
    assert_eq!(windows.len(), 13, "the window show() sites: {windows:?}");
    assert_eq!(unhides, 1, "the one app unhide, in unhide_app");
    assert_eq!(notifications, 3, "the three notification builders");
    // The menu's three sign-out message dialogs, and (task 1882, rebase 2026-10-10) the kept-folder alert,
    // `show_preserved_files_alert`, (task 1885, I2) the menu's failed-open alert, `show_menu_open_failure`,
    // (task 1889, fix round 2 N2) the menu's failed-pause alert, `show_menu_pause_failure`, and (task 1887) the
    // sign-out's kept-folders alert, `surface_sign_out_folders`, a late sign-out result, `show_late_result`, and
    // the R10 reset's kept-folder alert, `surface_reset_folder`. None of them is a window: macOS draws a parentless
    // alert itself.
    assert_eq!(
        dialogs, 9,
        "the three sign-out message dialogs, the three kept-folder alerts, the menu's failed-open and failed-pause \
             alerts and the late sign-out result"
    );
}

/// Lead ruling T8-t9 (10): the support bundle's lifecycle tail was done in Task 8
/// (`the_support_bundle_carries_the_lifecycle_tail_without_changing_its_format`), so Task 9 adds
/// nothing for it. This only pins that it is still one call.
#[test]
fn the_support_bundle_still_carries_the_lifecycle_tail() {
    let production = production_source();
    assert_eq!(
        production.matches("with_lifecycle_tail(").count(),
        2,
        "its definition and the one call"
    );
}

/// Lead ruling T8-⚠lock (1): the waits are longer than the longest operation the reconciler can
/// be inside when the event arrives, or a slow-but-working check would make Lock and Sign-out fail.
#[cfg(target_os = "macos")]
#[test]
fn the_acknowledgement_timeouts_outlast_the_longest_operation_they_can_wait_behind() {
    use super::finder_setup::core::Op;
    use super::finder_setup::driver::op_limit;
    use std::time::Duration;
    // The reconciler answers only between operations. The longest one: a StartEngine whose
    // socket never comes up. It waits for the engine slot (its limit), the socket (3 s), then
    // stops the engine it started: the slot again (the stop's limit) and abort()'s own 3 s + 2 s.
    let s = Duration::from_secs;
    let start_that_fails = op_limit(Op::StartEngine) + s(3) + op_limit(Op::StopEngine) + s(5);
    for op in [
        Op::Observe,
        Op::ReadDomain,
        Op::AddDomain,
        Op::FinishReady,
        Op::RemoveDomain,
        Op::WaitStable(s(10)),
    ] {
        assert!(
            op_limit(op) + s(0) <= start_that_fails,
            "{op:?} is not the longest operation"
        );
    }
    assert_eq!(start_that_fails, s(38));
    assert!(super::FINDER_LOCK_TIMEOUT >= s(45), "ruling 1: at least 45 s");
    assert!(
        super::FINDER_LOCK_TIMEOUT > start_that_fails,
        "Lock waits behind one operation"
    );
    assert!(super::FINDER_REMOVE_TIMEOUT >= s(45), "ruling 1: at least 45 s");
    assert!(
        super::FINDER_REMOVE_TIMEOUT > start_that_fails + op_limit(Op::RemoveDomain),
        "a removal waits behind one operation and then runs its own"
    );
}

/// Lead ruling T5-⚠c (9): the lifecycle log survives sign-out. The purge's one gate for a file
/// to delete is `is_disposable_cache_path`; the OS temp dir is a root it allows, so the log is
/// placed under it and only the Logs/Beebeeb rule stands between it and removal. The same gate
/// protects Repair's cache cleanup, and Task 10's reset must keep using it.
#[test]
fn a_sign_out_purge_never_deletes_the_lifecycle_log() {
    use crate::state_db::{FileEntry, FileStatus, ItemKind, OperationKind, PendingOperation, StateDb};
    let db_dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(db_dir.path().join("state.db")).unwrap();
    let root = std::env::temp_dir().join(format!("bb-test-1834-t9-{}", uuid::Uuid::new_v4()));
    let logs = root.join("Library").join("Logs").join("Beebeeb");
    let container_logs = root
        .join("Library")
        .join("Containers")
        .join("io.beebeeb.app")
        .join("Data")
        .join("Library")
        .join("Logs")
        .join("Beebeeb");
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::create_dir_all(&container_logs).unwrap();
    let ordinary = root.join("ordinary-cache.bin");
    let log = logs.join("lifecycle.log");
    let rotated = container_logs.join("lifecycle.log.1");
    for path in [&ordinary, &log, &rotated] {
        std::fs::write(path, b"x").unwrap();
    }
    // The gate itself: a log path is never disposable, wherever it is.
    assert!(
        super::is_disposable_cache_path(&ordinary),
        "the OS temp dir is an allowed root (the test's premise)"
    );
    assert!(!super::is_disposable_cache_path(&log) && !super::is_disposable_cache_path(&rotated));
    assert!(!super::is_disposable_cache_path(&logs) && !super::is_disposable_cache_path(&container_logs));

    // And the purge: a state.db row that points at a log (a corrupt or unexpected one) removes
    // the ordinary cache file and leaves both logs.
    db.upsert_file(&FileEntry {
        file_id: "file-a".into(),
        path: "/A.txt".into(),
        status: FileStatus::Local,
        size_bytes: 1,
        modified_at: 0,
        content_hash: None,
        remote_updated_at: 0,
        parent_id: None,
        item_kind: ItemKind::File,
    })
    .unwrap();
    db.mark_cached("file-a", ordinary.to_str().unwrap(), 1, 10).unwrap();
    db.upsert_file(&FileEntry {
        file_id: "file-b".into(),
        path: "/B.txt".into(),
        status: FileStatus::Local,
        size_bytes: 1,
        modified_at: 0,
        content_hash: None,
        remote_updated_at: 0,
        parent_id: None,
        item_kind: ItemKind::File,
    })
    .unwrap();
    db.mark_cached("file-b", log.to_str().unwrap(), 1, 10).unwrap();
    db.enqueue_operation(&PendingOperation {
        op_id: "op-1".into(),
        kind: OperationKind::UploadVersion,
        file_id: Some("file-a".into()),
        parent_id: None,
        target_path: Some("/A.txt".into()),
        metadata_json: None,
        payload_path: Some(rotated.to_str().unwrap().to_string()),
        base_version: None,
        base_object_version_id: None,
        attempts: 0,
        max_attempts: 5,
        next_retry_at: 0,
        last_error: None,
        backup_source_key: None,
        created_at: 100,
        updated_at: 100,
    })
    .unwrap();
    let summary = super::purge_local_state_files(&db, None, None).expect("the purge runs");
    let survived = (log.exists(), rotated.exists(), ordinary.exists());
    std::fs::remove_dir_all(&root).ok();
    assert_eq!(
        survived,
        (true, true, false),
        "both logs survive the purge; the ordinary cache file does not"
    );
    assert_eq!((summary.files_removed, summary.files_skipped), (1, 2));
}

/// Fix round 1 (item A): there is ONE way an engine is stopped, and it records an unconfirmed stop.
/// Repair and the respawn inside a start used to consume the handle, leave the slot empty and not set
/// the flag, so the next start (Repair's own follow-up check on a Mac) spawned a second engine beside
/// one that might still run, holding the keys.
#[test]
fn every_engine_stop_goes_through_the_one_helper_and_every_respawn_refuses_after_it() {
    let production = production_source();
    let abort = concat!(".abort()", ".await");
    // Every production abort of an engine: exactly one, inside the helper.
    assert_eq!(
        production.matches(abort).count(),
        1,
        "an engine is aborted outside `stop_engine_in_slot`"
    );
    let helper = body_between(&production, "async fn stop_engine_in_slot(", "\n}\n");
    assert!(
        helper.contains(abort) && helper.contains("engine_stop_unconfirmed.store(true"),
        "{helper}"
    );
    assert!(
        helper.contains("task_confirmed()"),
        "the flag follows `task_confirmed`: {helper}"
    );
    for site in [
        "start_engine_if_possible",
        "persist_sync_root_and_start_engine",
        "pick_sync_root",
        "stop_pending_finder_install_engine",
        "stop_engine_for_lock",
        "stop_engine_for_sign_out",
        "stop_engine_for_repair",
        "stop_check_engine",
    ] {
        let body = body_between(&production, &format!("fn {site}("), "\n}\n");
        assert!(
            body.contains("stop_engine_in_slot("),
            "{site} stops an engine without the helper"
        );
    }
    // Lock and sign-out stop through their own step (it also empties upload staging after a confirmed stop).
    // The needle starts with a space: `body_between` starts at the signature, and
    // `fn take_slot_and_stop_engine_for_sign_out(` itself contains `stop_engine_for_sign_out(` (review of the
    // merge, I1), so only a call (`= stop_engine_for_sign_out(`) may match.
    for (site, step) in [
        ("lock_vault", " stop_engine_for_lock("),
        ("take_slot_and_stop_engine_for_sign_out", " stop_engine_for_sign_out("),
    ] {
        let body = body_between(&production, &format!("fn {site}("), "\n}\n");
        assert!(
            !format!("fn {site}(").contains(step),
            "the needle {step:?} must not match {site}'s own signature"
        );
        assert!(body.contains(step), "{site} stops its engine without{step}");
    }
    // Sign-out stops through its helper on both paths (Task 12 fix round 2, item 1).
    let clear = body_between(&production, "async fn clear_session_impl(", "\n}\n");
    assert_eq!(
        crate::source_pin::squeeze(clear)
            .matches(&crate::source_pin::squeeze(
                "take_slot_and_stop_engine_for_sign_out(&acct).await.map_err(&kept_with)?"
            ))
            .count(),
        2,
        "{clear}"
    );
    // A start that replaces a running engine checks the flag again between the stop and the spawn.
    let spawn = "spawn_bound_engine(";
    for site in [
        "start_engine_if_possible",
        "persist_sync_root_and_start_engine",
        "pick_sync_root",
    ] {
        let body = body_between(&production, &format!("fn {site}("), "\n}\n");
        let stopped = body
            .find("stop_engine_in_slot(")
            .unwrap_or_else(|| panic!("{site} stops the old engine"));
        let spawned = body.find(spawn).expect("it starts through the gate");
        assert!(
            body[stopped..spawned].contains("engine_start_refusal("),
            "{site}: an unconfirmed stop of the old engine must refuse the new one:\n{}",
            &body[stopped..spawned]
        );
    }
}

/// The helper itself, on a real `EngineRunner`: an engine that cannot be stopped closes the gate for
/// every start; one that stops does not.
#[cfg(not(target_os = "windows"))]
#[test]
fn the_stop_helper_records_an_unconfirmed_stop_and_only_that() {
    use crate::account::{AccountId, AccountRuntime};
    use crate::runner::EngineRunner;
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let acct = AccountRuntime::new(AccountId("stop-helper".into()));
        // A stopped engine: no flag.
        let done = tokio::spawn(async {});
        tokio::task::yield_now().await;
        let outcome = super::stop_engine_in_slot(&acct, EngineRunner::for_test_with_task(done)).await;
        assert!(outcome.is_stopped());
        assert!(
            !acct.engine_stop_unconfirmed.load(Ordering::SeqCst),
            "a confirmed stop leaves the gate open"
        );
        assert_eq!(super::engine_start_refusal(&acct), Ok(()));
        // An engine with no await point: tokio cannot force-abort it, so the stop cannot be confirmed.
        let deadline = Instant::now() + Duration::from_secs(7);
        let stuck = tokio::task::spawn_blocking(move || {
            while Instant::now() < deadline {
                std::hint::black_box(());
            }
        });
        let outcome = super::stop_engine_in_slot(&acct, EngineRunner::for_test_with_task(stuck)).await;
        assert!(!outcome.task_confirmed());
        assert!(
            acct.engine_stop_unconfirmed.load(Ordering::SeqCst),
            "an unconfirmed stop closes the gate by itself"
        );
        assert!(super::engine_start_refusal(&acct).is_err());
    });
}

#[test]
fn an_engine_start_refuses_while_an_earlier_stop_is_unconfirmed() {
    let acct = crate::account::AccountRuntime::new(crate::account::AccountId("refusal-test".into()));
    assert_eq!(super::engine_start_refusal(&acct), Ok(()));
    acct.engine_stop_unconfirmed
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let refused = super::engine_start_refusal(&acct).expect_err("the flag closes every start");
    assert!(refused.contains("Restart Beebeeb"), "{refused}");
    assert!(
        !refused.contains('/') && !refused.contains("NS"),
        "no path and no OS text: {refused}"
    );
}

/// Spec §5.6: a start refused on an unconfirmed engine stop leaves its own code and sentence on
/// `sync_status.engine_refusal` (so a surface says to quit and reopen, not "Try again"), and it is cleared like the
/// binding refusals, by every session transition; the next refused start records it again.
#[test]
fn a_start_refused_on_an_unconfirmed_stop_shows_its_own_refusal_until_a_transition_clears_it() {
    use crate::account::SessionTransition;
    let acct = crate::account::AccountRuntime::new(crate::account::AccountId("refusal-shown".into()));
    assert!(super::engine_start_refusal(&acct).is_ok());
    assert_eq!(
        super::engine_refusal_view(&acct),
        serde_json::Value::Null,
        "nothing refused yet"
    );
    acct.engine_stop_unconfirmed
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(super::engine_start_refusal(&acct).is_err());
    assert_eq!(
        super::engine_refusal_view(&acct),
        serde_json::json!({
            "code": "engine_stop_unconfirmed",
            "sentence": "Beebeeb’s sync didn’t confirm it stopped. Quit and reopen Beebeeb before syncing again.",
        })
    );
    for by in [
        SessionTransition::Write,
        SessionTransition::Lock,
        SessionTransition::SignOut,
    ] {
        {
            let _turn = super::SessionWrite::for_test();
            acct.advance_session_generation_holding_session_write_lock(by);
        }
        assert_eq!(
            super::engine_refusal_view(&acct),
            serde_json::Value::Null,
            "{by:?} cleared it"
        );
        assert!(super::engine_start_refusal(&acct).is_err(), "the flag still refuses");
        assert_eq!(
            super::engine_refusal_view(&acct)["code"],
            "engine_stop_unconfirmed",
            "and the next refused start records it again"
        );
    }
}

/// FA-I3 (spec §5.6): Repair reports an unconfirmed engine stop as a field, read from the account AFTER its own
/// stop, so it covers an earlier unconfirmed stop that left the slot empty as well as its own. A clean stop is
/// `false`. On Linux (and Windows) the warning string stays; a Mac's surfaces render the field's fixed sentence, so
/// there `warnings` does not repeat it.
#[cfg(not(target_os = "windows"))]
#[test]
fn repairs_engine_stop_reports_an_unconfirmed_stop_whoever_made_it() {
    use crate::account::{AccountId, AccountRuntime};
    use crate::runner::EngineRunner;
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        // A clean stop.
        let acct = AccountRuntime::new(AccountId("repair-clean".into()));
        let done = tokio::spawn(async {});
        tokio::task::yield_now().await;
        *acct.engine.lock().await = Some(EngineRunner::for_test_with_task(done));
        let mut warnings = Vec::new();
        assert!(
            !super::stop_engine_for_repair(&acct, &mut warnings, || Ok(false))
                .await
                .engine_stop_unconfirmed,
            "a clean stop"
        );
        assert!(warnings.is_empty());
        // An earlier stop (a Lock, a Sign-out or a check) was never confirmed and left the slot empty.
        let acct = AccountRuntime::new(AccountId("repair-earlier".into()));
        acct.engine_stop_unconfirmed.store(true, Ordering::SeqCst);
        let mut warnings = Vec::new();
        assert!(
            super::stop_engine_for_repair(&acct, &mut warnings, || Ok(false))
                .await
                .engine_stop_unconfirmed,
            "an earlier unconfirmed stop is reported too"
        );
        // Repair's own stop cannot be confirmed (an engine with no await point cannot be force-aborted).
        let acct = AccountRuntime::new(AccountId("repair-own".into()));
        let deadline = Instant::now() + Duration::from_secs(7);
        let stuck = tokio::task::spawn_blocking(move || {
            while Instant::now() < deadline {
                std::hint::black_box(());
            }
        });
        *acct.engine.lock().await = Some(EngineRunner::for_test_with_task(stuck));
        let mut warnings = Vec::new();
        assert!(
            super::stop_engine_for_repair(&acct, &mut warnings, || Ok(false))
                .await
                .engine_stop_unconfirmed,
            "its own unconfirmed stop"
        );
        #[cfg(not(target_os = "macos"))]
        assert_eq!(
            warnings,
            vec![super::REPAIR_ENGINE_UNCONFIRMED_WARNING.to_string()],
            "the warning stays"
        );
        #[cfg(target_os = "macos")]
        assert!(
            warnings.is_empty(),
            "a Mac renders the field's sentence; the warnings do not repeat it: {warnings:?}"
        );
    });
}

/// FB-24: `clear_session` and `lock_vault` answer `Ok` when the action happened, with `warning` always on the wire
/// (`null` when every step was confirmed), and the closed vocabulary `{code, sentence}` for what was not. One shape
/// per code, pinned exactly.
#[test]
fn session_action_outcome_json_is_the_frontends_contract() {
    use super::{ActionWarning, SessionActionOutcome};
    let json = |outcome: SessionActionOutcome| serde_json::to_value(outcome).unwrap();
    assert_eq!(
        json(SessionActionOutcome { warning: None }),
        serde_json::json!({ "warning": null }),
        "every step confirmed"
    );
    let cases = [
        (
            ActionWarning::FinderRemovalUnconfirmed,
            "finder_removal_unconfirmed",
            "You are signed out, but Beebeeb could not confirm that it was removed from Finder. If it still shows there, restart Beebeeb and sign out again.",
        ),
        (
            ActionWarning::FinderLockUnconfirmed,
            "finder_lock_unconfirmed",
            "The vault is locked, but Beebeeb could not confirm that its Finder setup stopped. Restart Beebeeb to be sure.",
        ),
        (
            ActionWarning::EngineStopUnconfirmed,
            "engine_stop_unconfirmed",
            "The vault is locked, but the sync engine did not confirm it stopped. Restart Beebeeb before unlocking again.",
        ),
    ];
    for (warning, code, sentence) in cases {
        assert_eq!(
            json(SessionActionOutcome { warning: Some(warning) }),
            serde_json::json!({ "warning": { "code": code, "sentence": sentence } }),
            "{code}"
        );
    }
    // Both commands answer with this shape, and an `Err` from them keeps meaning "it did not happen".
    let production = production_source();
    for (command, signature) in [
        ("clear_session", "async fn clear_session("),
        ("lock_vault", "async fn lock_vault("),
    ] {
        let head = crate::source_pin::squeeze(body_between(&production, signature, "{\n"));
        assert!(
            head.contains("->Result<SessionActionOutcome,String>"),
            "{command} answers SessionActionOutcome:\n{head}"
        );
    }
    for body in [
        body_between(&production, "async fn clear_session_impl(", "\n}\n"),
        body_between(&production, "async fn lock_vault(", "\n}\n"),
    ] {
        for warning in [
            "FINDER_SIGN_OUT_UNCONFIRMED_WARNING",
            "FINDER_LOCK_UNCONFIRMED_WARNING",
            "LOCK_ENGINE_UNCONFIRMED_WARNING",
        ] {
            assert!(
                !body.contains(warning),
                "a warning is never an error string ({warning})"
            );
        }
    }
}

/// FB-24 (M6): the native menu's Sign out shows, on every platform, the error of a sign-out that stopped and the
/// warning of one that happened with a step it could not confirm. Only "you are not signed in" stays Windows-only.
#[test]
fn the_native_menus_sign_out_shows_errors_and_warnings_on_every_platform() {
    let production = production_source();
    let at = production
        .find("DesktopMenuAction::SignOut => {")
        .expect("the menu's Sign out");
    let arm = &production[at..at + production[at..].find("DesktopMenuAction::Quit").expect("the next arm")];
    let matched = arm.find("match &result {").expect("the result is shown");
    assert!(!arm[..matched].contains("#[cfg("), "not behind a platform cfg:\n{arm}");
    let squeezed = crate::source_pin::squeeze(arm);
    for needle in [
        "Err(error)=>{app.dialog().message(error.message.clone())",
        "Ok(SignedOut{warning:Some(warning),..})=>{app.dialog().message(warning.sentence())",
    ] {
        assert!(squeezed.contains(needle), "{needle}:\n{arm}");
    }
    let windows_only = arm
        .find("#[cfg(target_os = \"windows\")]")
        .expect("the one Windows-only dialog");
    assert!(
        arm[windows_only..].contains("You are not signed in on this device."),
        "{arm}"
    );
}

/// Area A M2: Repair removes the stale IPC socket while it still holds the engine slot, so an engine that a waiting
/// check starts the moment the slot is free (with its fresh socket) is never the one whose socket is removed. The
/// remover is passed in: production passes `remove_stale_ipc_socket`, and this test one that records whether the
/// slot was held when it ran (it never touches the real socket path). A remover that fails is a warning.
#[test]
fn repair_removes_the_socket_while_it_still_holds_the_engine_slot() {
    use crate::account::{AccountId, AccountRuntime};
    use crate::runner::EngineRunner;
    use std::sync::atomic::{AtomicBool, Ordering};
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let acct = AccountRuntime::new(AccountId("repair-socket".into()));
        let done = tokio::spawn(async {});
        tokio::task::yield_now().await;
        *acct.engine.lock().await = Some(EngineRunner::for_test_with_task(done));
        let held = AtomicBool::new(false);
        let mut warnings = Vec::new();
        let stop = super::stop_engine_for_repair(&acct, &mut warnings, || {
            held.store(acct.engine.try_lock().is_err(), Ordering::SeqCst);
            Ok(true)
        })
        .await;
        assert!(
            held.load(Ordering::SeqCst),
            "the socket goes while the slot is still held"
        );
        assert!(stop.removed_socket);
        assert!(warnings.is_empty(), "{warnings:?}");
        let mut warnings = Vec::new();
        let stop =
            super::stop_engine_for_repair(&acct, &mut warnings, || Err("remove IPC socket: denied".into())).await;
        assert!(!stop.removed_socket);
        assert_eq!(warnings, vec!["remove IPC socket: denied".to_string()]);
    });
    let production = production_source();
    let repair = body_between(&production, "async fn reset_macos_integration(", "\n}\n");
    assert_eq!(
        repair.matches("remove_stale_ipc_socket").count(),
        1,
        "Repair removes the socket only through its engine stop:\n{repair}"
    );
}

/// The Repair result carries the flag on the wire, always (Lane T renders one fixed sentence for it), and
/// `reset_macos_integration` fills it from the stop above.
#[test]
fn the_repair_result_carries_engine_stop_unconfirmed() {
    let result = super::MacosIntegrationResetResult {
        removed_file_provider_domain: true,
        preserved_location: None,
        disabled_autostart: false,
        removed_socket: true,
        removed_cache_files: 0,
        skipped_cache_files: 0,
        pending_operations_preserved: 0,
        sync_root_preserved: None,
        engine_stop_unconfirmed: true,
        warnings: Vec::new(),
    };
    let value = serde_json::to_value(&result).unwrap();
    assert_eq!(value["engine_stop_unconfirmed"], serde_json::json!(true));
    let clean = super::MacosIntegrationResetResult {
        engine_stop_unconfirmed: false,
        ..result
    };
    assert_eq!(
        serde_json::to_value(&clean).unwrap()["engine_stop_unconfirmed"],
        serde_json::json!(false),
        "sent when false too"
    );
    let production = production_source();
    let repair = crate::source_pin::squeeze(body_between(&production, "async fn reset_macos_integration(", "\n}\n"));
    let stop = repair
            .find(&crate::source_pin::squeeze(
                "let RepairEngineStop { removed_socket, engine_stop_unconfirmed } = stop_engine_for_repair(&acct, &mut warnings, remove_stale_ipc_socket).await;",
            ))
            .expect("read from the stop's own answer");
    let field = repair
        .find(&crate::source_pin::squeeze(
            "sync_root_preserved,\n        engine_stop_unconfirmed,",
        ))
        .expect("into the result");
    assert!(stop < field, "{repair}");
}
