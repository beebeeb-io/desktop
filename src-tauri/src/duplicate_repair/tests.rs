//! Task 2274: tests of the duplicates repair tool (spec `docs/superpowers/specs/2026-10-29-2274-duplicates-repair-design.md`
//! in the workspace; T-numbers are its test matrix).
use super::*;

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|arg| arg.to_string()).collect()
}

#[test]
fn an_unknown_subcommand_is_refused() {
    assert!(parse(&args(&["frobnicate"])).is_err());
    assert!(parse(&args(&[])).is_err());
}

#[test]
fn each_subcommand_parses_with_its_arguments() {
    assert_eq!(parse(&args(&["list"])).unwrap(), Command::List);
    assert_eq!(parse(&args(&["server-read"])).unwrap(), Command::ServerRead);
    assert_eq!(parse(&args(&["compare", "--download-ok", "12"])).unwrap(), Command::Compare { download_ok: 12 });
    assert_eq!(parse(&args(&["apply-one", "3"])).unwrap(), Command::ApplyOne { pair: 3 });
    assert_eq!(parse(&args(&["check-echo", "3"])).unwrap(), Command::CheckEcho { pair: 3 });
    // A2/A4: the numbers that gate a download or a write are never guessed.
    assert!(parse(&args(&["compare"])).is_err());
    assert!(parse(&args(&["compare", "--download-ok", "many"])).is_err());
    assert!(parse(&args(&["apply-one"])).is_err());
    assert!(parse(&args(&["apply-one", "0"])).is_err());
    assert!(parse(&args(&["apply-one", "3", "4"])).is_err());
}

#[cfg(not(target_os = "windows"))]
#[test]
fn every_subcommand_refuses_off_windows() {
    for argv in [&["list"][..], &["server-read"], &["compare", "--download-ok", "1"], &["apply-one", "1"], &["check-echo", "1"]] {
        let error = run(args(argv)).unwrap_err();
        assert!(error.to_string().contains("Windows only"), "{argv:?}: {error}");
    }
}

/// T15: a server phase never runs while the app holds its sync lock, and the refusal names no path (I5).
#[tokio::test]
async fn a_server_phase_is_refused_while_the_app_holds_its_lock() {
    let state = tempfile::tempdir().unwrap();
    let lock = state.path().join(crate::state_paths::LOCK_FILENAME);
    let holder = serde_json::json!({
        "pid": std::process::id(),
        "agent": "desktop",
        "hostname": "test-host",
        "started_at": "2026-10-29T19:00:00Z",
    });
    std::fs::write(&lock, holder.to_string()).unwrap();
    let refused = crate::state_paths::STATE_DIR_OVERRIDE
        .scope(state.path().to_path_buf(), async { claim_app_lock().map(|_| ()) })
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(refused, APP_RUNNING);
    assert!(!refused.contains(&state.path().display().to_string()));
    // The app's lock is left exactly as it was.
    assert_eq!(std::fs::read_to_string(&lock).unwrap(), holder.to_string());
}

#[tokio::test]
async fn the_lock_is_claimed_when_the_app_is_not_running_and_released_after() {
    let state = tempfile::tempdir().unwrap();
    let lock = state.path().join(crate::state_paths::LOCK_FILENAME);
    crate::state_paths::STATE_DIR_OVERRIDE
        .scope(state.path().to_path_buf(), async {
            let held = claim_app_lock().unwrap();
            assert!(lock.exists());
            drop(held);
        })
        .await;
    assert!(!lock.exists());
}
