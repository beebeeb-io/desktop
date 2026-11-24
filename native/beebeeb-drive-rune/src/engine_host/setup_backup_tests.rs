//! Step 3, "Back up your folders", on the engine double of `setup_tests.rs`: the folders are read on a worker; the
//! step is a loading state while they are, a failure with Try again when the read failed, and the folders found
//! when it answered. Never "not in this version" while Backups is on.

use super::*;

/// A first run on `engine` whose folder read is held until the returned guard drops.
fn backup_read_held(engine: &Arc<Fake>) -> (EngineSetupSource<Fake>, std::sync::MutexGuard<'_, ()>) {
    engine.reads_choices.store(true, Ordering::SeqCst);
    let held = engine.choices_held.lock().unwrap();
    let (asked, platform, first_run) = (
        Arc::<Asked>::default(),
        Arc::new(FakePlatform::default()),
        Arc::<FirstRun>::default(),
    );
    first_run.read(&**engine);
    (
        EngineSetupSource::new(engine.clone(), hooks(&asked, platform, first_run)),
        held,
    )
}

#[test]
fn the_backup_step_is_not_called_unavailable_while_its_folders_are_still_being_read() {
    let engine = fake();
    let (mut source, held) = backup_read_held(&engine);
    // The read is on its worker and has not answered: the step must not say "not in this version".
    assert!(
        source.available(Step::Backup),
        "a read that is running is not an absence"
    );
    assert!(
        source.loading(Step::Backup),
        "it is drawn as loading until the read answers"
    );
    assert!(!source.loading(Step::Notifications), "no other step is loading");
    drop(held);
    let _began = Instant::now();
    // The answer: no folders were found. That is an answer, not an absence: the step stays (it is never "not in
    // this version" while Backups is on), is no longer loading, and did not fail.
    let began = Instant::now();
    while source.loading(Step::Backup) {
        assert!(began.elapsed() < Duration::from_secs(10), "the read never answered");
        source.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(source.available(Step::Backup) && !source.failed(Step::Backup));
    assert!(source.folders().is_empty());
}

#[test]
fn a_failed_folder_read_is_a_failure_with_try_again_never_not_in_this_version() {
    let engine = fake();
    engine.choices_fail.store(true, Ordering::SeqCst);
    let (mut source, held) = backup_read_held(&engine);
    drop(held);
    let began = Instant::now();
    while !source.failed(Step::Backup) {
        assert!(began.elapsed() < Duration::from_secs(10), "the read never answered");
        source.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(source.available(Step::Backup), "the step stays: it is not an absence");
    assert!(!source.loading(Step::Backup));
    // Try again: the read goes out again, and its answer replaces the failure.
    engine.choices_fail.store(false, Ordering::SeqCst);
    *engine.choices.lock().unwrap() = vec![BackupChoice {
        key: "documents".into(),
        name: "Documents".into(),
        on: true,
        items: Some(3),
    }];
    source.retry_backups();
    assert!(
        !source.failed(Step::Backup),
        "not drawn as failed while it is read again"
    );
    while source.folders().is_empty() {
        assert!(
            began.elapsed() < Duration::from_secs(10),
            "the second read never answered"
        );
        source.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!source.failed(Step::Backup) && !source.loading(Step::Backup));
}

#[test]
fn the_backup_step_offers_the_folders_the_read_found() {
    let engine = fake();
    *engine.choices.lock().unwrap() = vec![BackupChoice {
        key: "documents".into(),
        name: "Documents".into(),
        on: true,
        items: Some(3),
    }];
    let (mut source, held) = backup_read_held(&engine);
    drop(held);
    let began = Instant::now();
    while source.folders().is_empty() {
        assert!(began.elapsed() < Duration::from_secs(10), "the read never answered");
        source.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(source.available(Step::Backup) && !source.loading(Step::Backup));
    assert_eq!(source.folders()[0].name, "Documents");
}
