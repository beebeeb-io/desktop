//! Wiring guards complement the behavioral session/gate tests: a new command
//! or engine entrypoint must not silently bypass the Windows admission boundary.
static SOURCE: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| include_str!("../src/lib.rs").replace("\r\n", "\n"));
fn body<'a>(source: &'a str, name: &str) -> &'a str {
    let start = source.find(&format!("fn {name}(")).unwrap();
    let start = start + source[start..].find('{').unwrap();
    &source[start..start + source[start..].find("\n}").unwrap()]
}
fn unleased_credential_commands(source: &str) -> Vec<String> {
    let source = source.replace("\r\n", "\n");
    let commands: Vec<_> = source.split("#[tauri::command]\nasync fn ").skip(1).collect();
    assert!(!commands.is_empty(), "admission guard must scan commands");
    commands
        .into_iter()
        .filter_map(|section| {
            let name = section.split('(').next().unwrap();
            let body = &section[..section.find("\n}").unwrap()];
            let credentials = body.contains("api_client_from_session(")
                || body.contains(".token.clone()")
                || body.contains("s.master_key");
            (credentials && !body.contains("session_command!(") && !body.contains("SESSION_TRANSITION.lock().await"))
                .then(|| name.to_owned())
        })
        .collect()
}
#[test]
fn every_credential_command_has_admission_before_credential_reads() {
    assert_eq!(SOURCE.matches("session_command!(async").count(), 32);
    assert!(
        unleased_credential_commands(&SOURCE).is_empty(),
        "unleased commands: {:?}",
        unleased_credential_commands(&SOURCE)
    );
    for name in ["lock_vault", "clear_session_impl"] {
        let code = body(&SOURCE, name);
        assert!(code.find("close_session_commands().await?").unwrap() < code.find("acct.engine.lock().await").unwrap());
    }
}
#[test]
fn every_engine_start_requires_transition_and_generation_validation() {
    // R10 (task 1834, lead ruling T9-starts): ONE `EngineRunner::spawn(` in lib.rs, inside
    // `spawn_bound_engine`, which reads the keys under the engine slot and binds the local data to the
    // account before it spawns. Every start site calls it. Six sites: the four below, and two that belong
    // to the macOS Finder reconciler (spec 2026-10-06 §5.5), `ensure_sync_root_and_engine` and
    // `start_check_engine`. Both are macOS only, so no Windows admission boundary can apply to them; the
    // assertions pin that each stays macOS only and calls the gate exactly once. A new start that spawns
    // an engine anywhere else turns the count red; a new Windows-reachable start must be added below,
    // where it is checked for transition serialization and generation validation BEFORE the gate.
    assert_eq!(SOURCE.matches("EngineRunner::spawn(").count(), 1);
    let gate_at = SOURCE.find("fn spawn_bound_engine(").unwrap();
    let gate = &SOURCE[gate_at..];
    let gate = &gate[..gate.find("\n}").unwrap()];
    assert_eq!(
        gate.matches("EngineRunner::spawn(").count(),
        1,
        "the one start is inside the gate"
    );
    assert!(
        !SOURCE[..gate_at]
            .trim_end()
            .lines()
            .next_back()
            .unwrap()
            .trim_start()
            .starts_with("#[cfg"),
        "the gate is compiled on Windows"
    );
    for macos_only in ["ensure_sync_root_and_engine", "start_check_engine"] {
        assert_eq!(SOURCE.matches(&format!("fn {macos_only}(")).count(), 1);
        let at = SOURCE.find(&format!("async fn {macos_only}(")).unwrap();
        assert!(
            SOURCE[..at].trim_end().ends_with("#[cfg(target_os = \"macos\")]"),
            "{macos_only} must stay macOS only"
        );
        let start = &SOURCE[at..];
        let start = &start[..start.find("\n}").unwrap()];
        assert_eq!(
            start.matches("spawn_bound_engine(").count(),
            1,
            "{macos_only} starts through the gate once"
        );
    }
    for name in [
        "start_engine_if_possible",
        "persist_sync_root_and_start_engine",
        "start_engine_for_pending_finder_install",
        "pick_sync_root",
    ] {
        let code = body(&SOURCE, name);
        let validated = code
            .find("SESSION_COMMANDS.validate_start(generation)?")
            .unwrap_or_else(|| panic!("{name} bypasses generation validation"));
        let started = code
            .find("spawn_bound_engine(")
            .unwrap_or_else(|| panic!("{name} does not start through the gate"));
        assert!(
            validated < started,
            "{name} validates its generation after it has started the engine"
        );
        let function = &SOURCE[SOURCE.find(&format!("fn {name}(")).unwrap()..];
        let function = &function[..function.find("\n}").unwrap()];
        assert!(
            function.contains("MutexGuard<'_, ()>") || function.contains("SESSION_TRANSITION.lock().await"),
            "{name} bypasses transition serialization"
        );
    }
}
#[test]
fn cloud_files_unregister_precedes_shell_unregister_and_propagates_failure() {
    let code = body(&SOURCE, "clear_session_impl");
    assert!(
        code.find("windows_cf::unregister_sync_root(&root)").unwrap()
            < code.find("windows_cf::unregister_shell_sync_root(&root)").unwrap()
    );
    assert!(code.contains("Could not remove Explorer registration: {e}\"))?"));
    assert!(code.contains("Could not unregister Cloud Files: {e}\"))?"));
}
#[test]
fn admission_guard_selftest_detects_a_bypassed_command() {
    for ending in ["\n", "\r\n"] {
        let source = SOURCE.replace("\r\n", "\n").replace('\n', ending);
        let source = source.replacen("session_command!(async", "unprotected!(async", 1);
        assert_eq!(
            unleased_credential_commands(&source),
            vec!["desktop_storage_summary"],
            "line ending: {ending:?}"
        );
    }
}

fn auth_wiring(source: &str, browser: &str) -> bool {
    let apply = body(source, "apply_session");
    let close = body(source, "close_session_commands");
    let start = body(browser, "start_browser_login");
    let (Some(begin), Some(handoff)) = (start.find("AUTH_ATTEMPTS.begin()?"), start.find("run_handoff(")) else {
        return false;
    };
    let Some(validation) = apply.find("AUTH_ATTEMPTS.validate(attempt)?") else {
        return false;
    };
    // The persistence is one call deeper since the session writes take checked turns (desktop 1834 Task 12): the
    // browser sign-in's in `install_new_session`, the password sign-ins' in `store_first_sign_in`.
    let persists = |helper: &str, writer: &str| body(source, helper).contains(writer);
    validation > apply.find("SESSION_TRANSITION.lock().await").unwrap()
        && validation < apply.find("install_new_session_or_revoke(").unwrap()
        && persists("install_new_session_or_revoke", "install_new_session(")
        && persists("install_new_session", "persist_session_to_keychain(")
        && persists("store_first_sign_in", "persist_session_token_to_keychain(")
        && close.contains("AUTH_ATTEMPTS.close()")
        && close.contains("auth.drain(")
        && begin < handoff
        && start.contains("attempt.run(work).await")
        && ["desktop_login", "desktop_login_2fa"].iter().all(|name| {
            let code = body(source, name);
            let network = code.find("fetch_session_profile(").unwrap();
            let persist = code.find("store_first_sign_in(").unwrap();
            code[..network].contains("drop(_transition)")
                && code[network..persist].contains("AUTH_ATTEMPTS.validate(&attempt)?")
        })
        && [
            "desktop_login",
            "desktop_login_2fa",
            "desktop_unlock_with_recovery_phrase",
            "unlock_vault",
        ]
        .iter()
        .all(|name| {
            let code = body(source, name);
            code.contains("AUTH_ATTEMPTS.begin()?")
                && code.contains("AUTH_ATTEMPTS.validate(&attempt)?")
                && code.contains("attempt.run(work).await")
        })
}
#[test]
fn round5_auth_attempts_admit_cancel_drain_and_validate_before_persistence() {
    let browser = include_str!("../src/browser_login.rs").replace("\r\n", "\n");
    assert!(auth_wiring(&SOURCE, &browser));
}
#[test]
fn round5_auth_wiring_guard_rejects_missing_validation_and_cancellation() {
    let browser = include_str!("../src/browser_login.rs").replace("\r\n", "\n");
    for mutation in [
        "AUTH_ATTEMPTS.validate(attempt)?",
        "AUTH_ATTEMPTS.close()",
        "auth.drain(",
    ] {
        assert!(
            !auth_wiring(&SOURCE.replace(mutation, "BYPASSED"), &browser),
            "surviving mutant: {mutation}"
        );
    }
    assert!(!auth_wiring(
        &SOURCE,
        &browser.replace("attempt.run(work).await", "work.await")
    ));
}

fn recovery_verification_is_fenced(source: &str) -> bool {
    let code = body(source, "desktop_unlock_with_recovery_phrase");
    let Some(verify) = code.find("verify_vault_key_from_phrase(") else {
        return false;
    };
    // The key is persisted by `store_recovered_vault_key`, which re-checks the stored account under the session lock
    // first, through `install_recovered_session` (desktop 1834 Task 12: the write and the install in one checked
    // turn); the unlock calls it after the transition is re-acquired and the attempt validated.
    let Some(persist) = code.find("install_recovered_session(") else {
        return false;
    };
    if !body(source, "install_recovered_session").contains("store_recovered_vault_key(") {
        return false;
    }
    let Some(release) = code[..verify].rfind("drop(_transition)") else {
        return false;
    };
    let Some(acquire) = code[verify..persist].find("SESSION_TRANSITION.lock().await") else {
        return false;
    };
    let Some(validate) = code[verify..persist].find("AUTH_ATTEMPTS.validate(&attempt)?") else {
        return false;
    };
    release < verify && acquire < validate && !code[verify..persist].contains("provision_vault_key_from_phrase")
}
#[test]
fn round7_recovery_verifies_outside_transition_then_fences_persistence() {
    assert!(recovery_verification_is_fenced(&SOURCE));
}
#[test]
fn round7_recovery_guard_rejects_held_transition_and_missing_final_fence() {
    for ending in ["\n", "\r\n"] {
        let source = SOURCE.replace('\n', ending).replace("\r\n", "\n");
        let code = body(&source, "desktop_unlock_with_recovery_phrase");
        for mutation in [
            code.replace("drop(_transition);", "/* held */"),
            code.replace("AUTH_ATTEMPTS.validate(&attempt)?;", "/* bypassed */"),
        ] {
            assert!(!recovery_verification_is_fenced(&source.replace(code, &mutation)));
        }
    }
}
