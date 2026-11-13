//! S1, invariant I12 (contract §6) for an engine task that ends by itself right after a start (here: another process
//! of ours holds the engine lock, which the task finds when it takes it). The view stops saying sync runs, and the
//! ended engine leaves the slot, so the next start makes a new one instead of answering `AlreadyRunning` (the
//! refusal modal's "Try again" depends on it). Mutants: drop the runner's `EndedGuard`; answer `AlreadyRunning`
//! for any engine in the slot.

mod common;

#[cfg(not(target_os = "macos"))]
#[test]
fn an_engine_that_ends_by_itself_is_not_running() {
    use beebeeb_engine::app::{SyncStart, SyncStartRefused};
    use beebeeb_engine::sync_view::Start;

    let s = common::setup();
    s.sign_in();
    s.unlock();
    // A lock naming this live process: the engine task's own `LockFile::acquire` refuses and the task ends. The port
    // says the lock is free, so the start itself goes on (as when the lock is taken between the two).
    std::fs::create_dir_all(&s.state_dir).unwrap();
    let lock = format!(
        r#"{{"pid":{},"agent":"desktop","hostname":"pc","started_at":"2026-11-12T10:00:00Z"}}"#,
        std::process::id()
    );
    std::fs::write(s.state_dir.join(".beebeeb-sync.lock"), lock).unwrap();
    let port = common::Port::free();
    let folder = s.home.join("Beebeeb Test");

    assert_eq!(s.engine.start_sync_with(folder.clone(), &port), Ok(SyncStart::Started));
    let ended = common::wait_until(10, || s.engine.view().start != Start::Running);
    assert!(ended, "the view stopped saying sync runs");
    assert_eq!(s.engine.view().start, Start::Refused(SyncStartRefused::Other));
    assert!(!s.engine.is_syncing());

    let attempt = s.engine.view().attempt;
    assert_eq!(
        s.engine.start_sync_with(folder, &port),
        Ok(SyncStart::Started),
        "the ended engine left the slot"
    );
    assert_eq!(s.engine.view().attempt, attempt + 1, "a new attempt");
    assert!(common::wait_until(10, || s.engine.view().start != Start::Running));
}

#[cfg(target_os = "macos")]
#[test]
fn the_test_build_is_refused_on_macos() {
    common::assert_the_test_build_is_refused();
}
