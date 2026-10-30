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
        .scope(state.path().to_path_buf(), async { claim_app_lock_with(&|| false).map(|_| ()) })
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
            let held = claim_app_lock_with(&|| false).unwrap();
            assert!(lock.exists());
            drop(held);
        })
        .await;
    assert!(!lock.exists());
}

/// The app's own lock as the running app writes it.
fn app_holder() -> String {
    serde_json::json!({
        "pid": std::process::id(),
        "agent": "desktop",
        "hostname": "test-host",
        "started_at": "2026-10-29T19:00:00Z",
    })
    .to_string()
}

/// Review P2 (mod.rs:160): the app holds its sync lock only while its runner syncs. A running app that is paused,
/// signed out, locked or still starting holds none, and must still refuse every phase that takes the lock.
#[tokio::test]
async fn a_phase_is_refused_while_an_app_process_runs_without_its_lock() {
    let state = tempfile::tempdir().unwrap();
    let lock = state.path().join(crate::state_paths::LOCK_FILENAME);
    let refused = crate::state_paths::STATE_DIR_OVERRIDE
        .scope(state.path().to_path_buf(), async { claim_app_lock_with(&|| true).map(|_| ()) })
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(refused, APP_RUNNING);
    assert!(!lock.exists(), "no lock written");

    // The app writes its lock with a plain write, so a running app's lock can be caught empty, mid-write. The tool
    // touches no lock file while the app runs: it does not take that one for stale.
    std::fs::write(&lock, "").unwrap();
    let refused = crate::state_paths::STATE_DIR_OVERRIDE
        .scope(state.path().to_path_buf(), async { claim_app_lock_with(&|| true).map(|_| ()) })
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(refused, APP_RUNNING);
    assert_eq!(std::fs::read_to_string(&lock).unwrap(), "", "the app's lock is left as it is");
}

/// An app that starts while the tool claims the lock is caught by the check after the claim: the phase is refused and
/// the tool's own lock is removed again.
#[tokio::test]
async fn a_phase_is_refused_when_the_app_starts_while_the_lock_is_claimed() {
    let state = tempfile::tempdir().unwrap();
    let lock = state.path().join(crate::state_paths::LOCK_FILENAME);
    let checks = std::cell::Cell::new(0);
    let started_meanwhile = || {
        checks.set(checks.get() + 1);
        checks.get() > 1
    };
    let refused = crate::state_paths::STATE_DIR_OVERRIDE
        .scope(state.path().to_path_buf(), async { claim_app_lock_with(&started_meanwhile).map(|_| ()) })
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(refused, APP_RUNNING);
    assert_eq!(checks.get(), 2);
    assert!(!lock.exists(), "the tool's lock is removed again");
}

/// A lock whose process is gone, or which does not read as a lock, is stale: the tool replaces it with its own.
#[tokio::test]
async fn a_stale_lock_is_replaced_by_the_tool_s_own() {
    for stale in [
        serde_json::json!({"pid": 0x7fff_fff0_u32, "agent": "desktop", "hostname": "h", "started_at": "2026-10-29T19:00:00Z"})
            .to_string(),
        "not a lock".to_string(),
    ] {
        let state = tempfile::tempdir().unwrap();
        let lock = state.path().join(crate::state_paths::LOCK_FILENAME);
        std::fs::write(&lock, &stale).unwrap();
        crate::state_paths::STATE_DIR_OVERRIDE
            .scope(state.path().to_path_buf(), async {
                let held = claim_app_lock_with(&|| false).unwrap();
                assert_eq!(holder(&lock), Some((std::process::id(), AGENT.to_string())), "{stale}");
                drop(held);
            })
            .await;
        assert!(!lock.exists(), "{stale}");
    }
}

/// Review P2 (runner.rs:1127): the check and the write are one step. When the app writes its lock after the tool
/// looked and before it wrote, the tool refuses and leaves the app's lock exactly as the app wrote it.
#[tokio::test]
async fn the_claim_never_overwrites_a_lock_the_app_wrote_after_the_check() {
    let state = tempfile::tempdir().unwrap();
    let lock = state.path().join(crate::state_paths::LOCK_FILENAME);
    let racing = lock.clone();
    BEFORE_LOCK_WRITE.with(|hook| *hook.borrow_mut() = Some(Box::new(move || std::fs::write(&racing, app_holder()).unwrap())));
    let claimed = crate::state_paths::STATE_DIR_OVERRIDE
        .scope(state.path().to_path_buf(), async { claim_app_lock_with(&|| false).map(drop) })
        .await;
    BEFORE_LOCK_WRITE.with(|hook| *hook.borrow_mut() = None);
    assert_eq!(claimed.unwrap_err().to_string(), APP_RUNNING);
    assert_eq!(std::fs::read_to_string(&lock).unwrap(), app_holder(), "the app's lock is untouched");
}

/// A lock the tool held but the app replaced is the app's: dropping the tool's guard leaves it.
#[tokio::test]
async fn releasing_the_tool_lock_never_removes_the_app_s_lock() {
    let state = tempfile::tempdir().unwrap();
    let lock = state.path().join(crate::state_paths::LOCK_FILENAME);
    crate::state_paths::STATE_DIR_OVERRIDE
        .scope(state.path().to_path_buf(), async {
            let held = claim_app_lock_with(&|| false).unwrap();
            std::fs::write(&lock, app_holder()).unwrap();
            drop(held);
        })
        .await;
    assert_eq!(std::fs::read_to_string(&lock).unwrap(), app_holder());
}

/// The app runs as `beebeeb-desktop.exe` (Windows names it with any case); the tool and other programs do not count.
#[test]
fn the_app_process_is_known_by_its_executable_name() {
    use std::ffi::OsStr;
    for name in ["beebeeb-desktop.exe", "Beebeeb-Desktop.EXE", "beebeeb-desktop"] {
        assert!(is_app_process(OsStr::new(name)), "{name}");
    }
    for name in ["duplicate_repair.exe", "beebeeb-desktop-updater.exe", "explorer.exe", "beebeeb.exe"] {
        assert!(!is_app_process(OsStr::new(name)), "{name}");
    }
}
