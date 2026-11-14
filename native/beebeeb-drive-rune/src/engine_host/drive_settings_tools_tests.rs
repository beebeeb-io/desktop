//! Repair and turn off of File Explorer, the speed test, the support bundle, the controls that are not drawn, and the
//! reads that wait for their pane, against the fake engine of `drive_settings_tests`. Every name is invented.

use std::sync::atomic::AtomicBool;

use super::*;

/// What the fake OS was asked (a `fn` cannot capture, so the record is a static: these tests run one at a time on it).
static SHOWN: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
static DRAFTS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static DRAFT_OPENS: AtomicBool = AtomicBool::new(true);
static OS_LOCK: Mutex<()> = Mutex::new(());

fn fake_os() -> Os {
    Os {
        reveal: |path| {
            SHOWN.lock().unwrap().push(path.to_path_buf());
            true
        },
        open_draft: |url| {
            DRAFTS.lock().unwrap().push(url.to_string());
            DRAFT_OPENS.load(Ordering::SeqCst)
        },
    }
}

fn saved() -> SavedBundle {
    SavedBundle {
        path: PathBuf::from("somewhere/beebeeb-support-bundle-1.json"),
        file_name: "beebeeb-support-bundle-1.json".into(),
        folder: "Downloads",
    }
}

fn latency() -> Option<SpeedFigure> {
    Some(SpeedFigure::Latency {
        answered: 5,
        of: 5,
        min_ms: 18,
        median_ms: 24,
        max_ms: 41,
    })
}

fn throughput(mbps: f64) -> Option<SpeedFigure> {
    Some(SpeedFigure::Throughput {
        rounds_ok: 3,
        of: 3,
        mbps,
        bytes: 30_000_000,
    })
}

/// Nothing is read at launch: only the pane that opens reads, and Insights reads the file list once.
#[test]
fn nothing_is_read_at_launch_and_a_pane_reads_once_when_it_opens() {
    let fake = account();
    let mut source = EngineDriveSettings::new(fake.clone(), Arc::default());
    settle(&mut source);
    assert_eq!(
        fake.in_usage.passes.load(Ordering::SeqCst),
        0,
        "no read of Beebeeb at launch"
    );
    assert_eq!(
        fake.after_read.passes.load(Ordering::SeqCst),
        0,
        "no read of the local state at launch"
    );
    source.opened(Pane::Insights);
    settle(&mut source);
    assert_eq!(fake.in_usage.passes.load(Ordering::SeqCst), 1, "one read of the use");
    assert_eq!(
        fake.after_read.passes.load(Ordering::SeqCst),
        1,
        "one pass over the file list, not two"
    );
    source.opened(Pane::Sync);
    source.opened(Pane::Sync);
    settle(&mut source);
    assert_eq!(
        fake.after_read.passes.load(Ordering::SeqCst),
        2,
        "the Sync pane reads once, and opening it again while fresh asks nothing"
    );
}

#[test]
fn a_repair_runs_under_the_session_of_the_press_and_says_where_it_ended() {
    let fake = account();
    let mut source = source(&fake, Arc::default());
    let session = DriveEngine::session(&*fake);
    source.repair();
    assert_eq!(source.sync().repair, Repair::Running, "Repairing... at once");
    settle(&mut source);
    assert_eq!(source.sync().repair, Repair::Idle);
    assert_eq!(*fake.explorer_calls.lock().unwrap(), [("repair", session)]);

    *fake.explorer_answer.lock().unwrap() = Err(ExplorerChangeRefused::Failed);
    source.repair();
    settle(&mut source);
    assert_eq!(
        source.sync().repair,
        Repair::Failed,
        "Windows refused: the notice, nothing to undo"
    );
    assert_eq!(source.take_toast(), None);

    // Another sign-in's session is no one's to repair for.
    *fake.explorer_answer.lock().unwrap() = Err(ExplorerChangeRefused::NotSignedIn);
    source.repair();
    settle(&mut source);
    assert_eq!(source.sync().repair, Repair::Idle);
    assert_eq!(source.take_toast(), Some(ExplorerChangeRefused::NotSignedIn.words()));
}

#[test]
fn turn_off_is_asked_of_the_engine_and_a_refusal_while_changes_wait_is_said() {
    let fake = account();
    let mut source = source(&fake, Arc::default());
    let session = DriveEngine::session(&*fake);
    source.turn_off_explorer();
    settle(&mut source);
    assert_eq!(*fake.explorer_calls.lock().unwrap(), [("turn off", session)]);
    assert_eq!(source.take_toast(), None);

    *fake.explorer_answer.lock().unwrap() = Err(ExplorerChangeRefused::ChangesWait);
    source.turn_off_explorer();
    settle(&mut source);
    assert_eq!(source.take_toast(), Some(ExplorerChangeRefused::ChangesWait.words()));
}

#[test]
fn a_speed_test_runs_its_three_steps_and_draws_what_was_measured() {
    let fake = account();
    *fake.speed_answers.lock().unwrap() = vec![latency(), throughput(35.2), throughput(94.0)];
    let mut source = source(&fake, Arc::default());
    assert_eq!(source.bandwidth().test, SpeedTest::Idle, "never run");
    source.run_speed_test();
    assert!(source.bandwidth().test.running(), "running as soon as it is pressed");
    settle(&mut source);
    assert_eq!(
        *fake.speed_asked.lock().unwrap(),
        [SpeedStep::Latency, SpeedStep::Upload, SpeedStep::Download]
    );
    let SpeedTest::Done(steps, line) = source.bandwidth().test else {
        panic!("the test finished: {:?}", source.bandwidth().test);
    };
    assert_eq!(steps[0], Step::Done("24".into(), "ms".into(), "18 to 41 ms".into()));
    assert_eq!(
        steps[1],
        Step::Done("35.2".into(), "Mbps".into(), "3 of 3 \u{b7} 30 MB".into())
    );
    assert_eq!(
        steps[2],
        Step::Done("94.0".into(), "Mbps".into(), "3 of 3 \u{b7} 30 MB".into())
    );
    assert!(line.contains("moved 60 MB"), "{line}");
}

#[test]
fn a_speed_test_that_cannot_reach_beebeeb_names_the_step() {
    let fake = account();
    let mut source = source(&fake, Arc::default());
    source.run_speed_test();
    settle(&mut source);
    assert_eq!(
        source.bandwidth().test,
        SpeedTest::Failed(
            "Beebeeb didn't answer any of the 5 pings. Check your internet connection and try again.".into()
        )
    );
    assert_eq!(
        *fake.speed_asked.lock().unwrap(),
        [SpeedStep::Latency],
        "no later step runs"
    );
}

#[test]
fn a_speed_test_landing_after_the_account_changed_is_dropped() {
    let fake = account();
    *fake.speed_answers.lock().unwrap() = vec![latency(), throughput(35.2), throughput(94.0)];
    let mut source = source(&fake, Arc::default());
    source.run_speed_test();
    // Another account is signed in before the next frame: nothing of the earlier test is drawn for it.
    *fake.account.lock().unwrap() = Some("other@beebeeb.io".into());
    fake.session.fetch_add(1, Ordering::SeqCst);
    settle(&mut source);
    assert_eq!(source.bandwidth().test, SpeedTest::Idle);
}

#[test]
fn the_speed_test_button_is_off_in_the_test_build() {
    assert!(!SPEED_TEST_ON);
    let source = EngineDriveSettings::new(account(), Arc::default());
    assert!(!source.shows(Control::SpeedTest));
    assert!(!source.shows(Control::SyncOnMetered), "no metered detection: no switch");
    assert!(!source.shows(Control::StatusOnFiles), "nothing honours it: no switch");
}

#[test]
fn a_press_on_the_metered_switch_changes_nothing_and_says_so() {
    let mut source = source(&account(), Arc::default());
    source.set_sync_on_metered(true);
    assert_eq!(source.take_toast(), Some(METERED_NOT_AVAILABLE));
}

#[test]
fn an_export_saves_shows_and_opens_a_draft_that_names_the_file_only() {
    let _one_at_a_time = OS_LOCK.lock().unwrap();
    SHOWN.lock().unwrap().clear();
    DRAFTS.lock().unwrap().clear();
    DRAFT_OPENS.store(true, Ordering::SeqCst);
    let fake = account();
    *fake.bundle_answer.lock().unwrap() = Ok(saved());
    let mut source = EngineDriveSettings::new(fake.clone(), Arc::default()).with_os(fake_os());
    source.opened(Pane::Diagnostics);
    assert_eq!(
        source.diagnostics().bundle,
        Bundle::None,
        "showing the page exports nothing"
    );
    source.export_bundle();
    settle(&mut source);
    assert_eq!(
        source.diagnostics().bundle,
        Bundle::Saved {
            folder: "Downloads".into(),
            file: "beebeeb-support-bundle-1.json".into(),
            draft_opened: true
        }
    );
    assert_eq!(SHOWN.lock().unwrap().len(), 1);
    let drafts = DRAFTS.lock().unwrap();
    assert_eq!(drafts.len(), 1);
    assert!(drafts[0].starts_with("mailto:support@beebeeb.io?"), "{}", drafts[0]);
    assert!(drafts[0].contains("beebeeb-support-bundle-1.json"));
    assert!(!drafts[0].contains("somewhere"), "never the folder");
    assert!(source.diagnostics().activity.is_empty(), "no live figure is drawn");
}

#[test]
fn an_export_that_cannot_save_says_why_in_a_fixed_sentence_and_no_draft_opens() {
    let _one_at_a_time = OS_LOCK.lock().unwrap();
    DRAFTS.lock().unwrap().clear();
    let fake = account();
    let mut source = EngineDriveSettings::new(fake.clone(), Arc::default()).with_os(fake_os());
    source.export_bundle();
    settle(&mut source);
    assert_eq!(
        source.diagnostics().bundle,
        Bundle::Failed(BundleRefused::NoQueue.words().to_string())
    );
    assert!(DRAFTS.lock().unwrap().is_empty());
}

#[test]
fn a_draft_is_percent_encoded() {
    assert_eq!(tools::percent("a b&c"), "a%20b%26c");
    assert!(tools::draft_for("x.json").contains("x.json"));
}
