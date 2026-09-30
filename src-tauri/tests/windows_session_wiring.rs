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
    assert_eq!(SOURCE.matches("session_command!(async").count(), 31);
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
    assert_eq!(SOURCE.matches("EngineRunner::spawn(").count(), 4);
    for name in [
        "start_engine_if_possible",
        "persist_sync_root_and_start_engine",
        "start_engine_for_pending_finder_install",
        "pick_sync_root",
    ] {
        let code = body(&SOURCE, name);
        assert!(
            code.contains("SESSION_COMMANDS.validate_start(generation)?"),
            "{name} bypasses generation validation"
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
    validation > apply.find("SESSION_TRANSITION.lock().await").unwrap()
        && validation < apply.find("persist_session_to_keychain(").unwrap()
        && close.contains("AUTH_ATTEMPTS.close()")
        && close.contains("auth.drain(")
        && begin < handoff
        && start.contains("attempt.run(work).await")
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
