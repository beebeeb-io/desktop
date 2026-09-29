//! Wiring guards complement the behavioral session/gate tests: a new command
//! or engine entrypoint must not silently bypass the Windows admission boundary.
const SOURCE: &str = include_str!("../src/lib.rs");
fn body<'a>(source: &'a str, name: &str) -> &'a str {
    let start = source.find(&format!("fn {name}(")).unwrap();
    let start = start + source[start..].find('{').unwrap();
    &source[start..start + source[start..].find("\n}").unwrap()]
}
fn unleased_credential_commands(source: &str) -> Vec<&str> {
    source
        .split("#[tauri::command]\nasync fn ")
        .skip(1)
        .filter_map(|section| {
            let name = section.split('(').next().unwrap();
            let body = &section[..section.find("\n}").unwrap()];
            let credentials = body.contains("api_client_from_session(")
                || body.contains(".token.clone()")
                || body.contains("s.master_key");
            (credentials && !body.contains("session_command!(") && !body.contains("SESSION_TRANSITION.lock().await"))
                .then_some(name)
        })
        .collect()
}
#[test]
fn every_credential_command_has_admission_before_credential_reads() {
    assert_eq!(SOURCE.matches("session_command!(async").count(), 31);
    assert!(
        unleased_credential_commands(SOURCE).is_empty(),
        "unleased commands: {:?}",
        unleased_credential_commands(SOURCE)
    );
    for name in ["lock_vault", "clear_session_impl"] {
        let code = body(SOURCE, name);
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
        let code = body(SOURCE, name);
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
    let code = body(SOURCE, "clear_session_impl");
    assert!(
        code.find("windows_cf::unregister_sync_root(&root)").unwrap()
            < code.find("windows_cf::unregister_shell_sync_root(&root)").unwrap()
    );
    assert!(code.contains("Could not remove Explorer registration: {e}\"))?"));
    assert!(code.contains("Could not unregister Cloud Files: {e}\"))?"));
}
#[test]
fn admission_guard_selftest_detects_a_bypassed_command() {
    let source = SOURCE.replacen("session_command!(async", "unprotected!(async", 1);
    assert_eq!(unleased_credential_commands(&source), vec!["desktop_storage_summary"]);
}
