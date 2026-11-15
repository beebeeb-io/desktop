//! The one press (Restart to update: download, verify, hand off, owner ruling 2026-11-15) and the hand-off, on the
//! part and on the real `EngineApply` with a RUNNING sync: sync stops before the installer starts; a stop that is not
//! confirmed (this one's, or any earlier one in this process) starts nothing and says only what is known; a failed
//! start asks for sync back, and so does a worker that ends in the middle of the hand-off; a started one quits the
//! window (through `frames::settings::poll`, as the shell runs it); the caption says each step; a background check
//! never downloads nor hands off. Also: the window keeps looking before the first report, "checked <time>" is the last
//! verified check, the worker's cadence, and the release glue.

use super::*;

/// A task in the engine's slot that runs until it is aborted, on a runtime that runs it: `Engine::is_syncing` says
/// true, as while sync runs, and a stop ends it (after the graceful wait, by the forced abort) and is CONFIRMED:
/// `SyncStop::Stopped`. Nothing syncs, so nothing is written in the process's state folder.
pub(super) fn hold_running_stoppable(engine: &Engine<MemoryCredentials>) -> tokio::runtime::Runtime {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("a runtime for the slot's task");
    let task = runtime.spawn(std::future::pending::<()>());
    let account = engine.accounts().active_account().expect("the account");
    *account.engine.try_lock().expect("the engine slot is free") = Some(EngineRunner::for_test_with_task(task));
    engine.set_view_for_test(|view| view.start = Start::Running);
    runtime
}

/// The tray's `hold_running`: the task sits on a runtime nobody runs, so no abort of it is ever confirmed, and a
/// stop says `SyncStop::Unconfirmed` (after its 3 s and 2 s waits).
#[cfg(not(windows))]
fn hold_running_unstoppable(engine: &Engine<MemoryCredentials>) -> tokio::runtime::Runtime {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("a runtime for the slot's task");
    let task = runtime.spawn(std::future::pending::<()>());
    let account = engine.accounts().active_account().expect("the account");
    *account.engine.try_lock().expect("the engine slot is free") = Some(EngineRunner::for_test_with_task(task));
    engine.set_view_for_test(|view| view.start = Start::Running);
    runtime
}

/// The real hand-off on a RUNNING sync: its stop ends sync and is confirmed before the proof is minted. A hand-off
/// that never stops sync, or that mints the proof without a confirmed stop, leaves `is_syncing` true. The installer
/// did not start: the sync this hand-off stopped is asked back, once; a sync that was not running is not.
#[test]
fn the_hand_off_stops_a_running_sync_and_only_then_mints_the_proof() {
    let engine = engine();
    let _running = hold_running_stoppable(&engine);
    assert!(engine.is_syncing(), "the slot's task runs");
    let sync_back = SyncBack::default();
    let never: fn() -> StartInstaller = || Box::new(|_, _| panic!("no installer starts here"));
    let mut apply = EngineApply::starting_with(engine.clone(), &TEST, sync_back.clone(), pressed(), never());
    assert!(apply.stop_engine().is_some(), "a confirmed stop mints the proof");
    assert!(!engine.is_syncing(), "sync stopped");
    apply.resume_engine();
    assert!(sync_back.take(), "the installer did not start: sync is asked back");
    apply.resume_engine();
    assert!(!sync_back.take(), "once");

    let mut idle = EngineApply::starting_with(engine, &TEST, sync_back.clone(), pressed(), never());
    assert!(idle.stop_engine().is_some(), "no sync running: stopped, and the proof");
    idle.resume_engine();
    assert!(
        !sync_back.take(),
        "a sync that was not running is not started by an update that did not happen"
    );
}

/// A stop the engine cannot confirm (the task sits where no abort reaches it): no proof, so no installer starts, and
/// nothing is asked back (no new start of sync is allowed in this process). Not compiled on Windows: there an
/// unconfirmed stop fences the whole test process (`windows_cf::refuse_unconfirmed_stop`) and later starts of sync in
/// the same test binary would be refused.
#[cfg(not(windows))]
#[test]
fn a_stop_that_is_not_confirmed_starts_no_installer() {
    let engine = engine();
    let _parked = hold_running_unstoppable(&engine);
    let sync_back = SyncBack::default();
    let mut apply = EngineApply::starting_with(
        engine,
        &TEST,
        sync_back.clone(),
        pressed(),
        Box::new(|_, _| panic!("no installer starts")),
    );
    assert!(apply.stop_engine().is_none(), "Unconfirmed mints no proof");
    apply.resume_engine();
    assert!(!sync_back.take());
    // The slot is empty now, so the engine would say `WasNotRunning`: a second Restart to update still mints no
    // proof in this process (the task that did not confirm may still run).
    assert!(apply.stop_engine().is_none(), "not until Beebeeb restarts");
}

/// The words for a stop that was not confirmed say only what is known: never that sync runs ("did not stop"), never
/// that it stopped (on Windows the engine says `Unconfirmed` also when its task ended and File Explorer's connection
/// did not let go), never "Try again" (a new start needs a restart of Beebeeb).
#[test]
fn an_unconfirmed_stop_says_only_what_is_known() {
    let says = failed(Failure::EngineStillRunning, Freshness::Verified);
    assert_eq!(says.says(), SYNC_UNCONFIRMED);
    for claim in ["did not stop", "is still", "stopped.", "Try again"] {
        assert!(!says.says().contains(claim), "{claim}: {}", says.says());
    }
}

/// The one press on a RUNNING sync with the real hand-off: Restart to update downloads, verifies, and stops sync
/// BEFORE the installer starts, by itself; the installer did not start, so the part says so, keeps the download, asks
/// the shell for sync back, once (no quit), and the button says "Try again", which runs it again.
#[test]
fn restart_to_update_stops_a_running_sync_first_and_a_failed_start_asks_for_sync_back() {
    use crate::frames::settings::source::Update;
    let engine = engine();
    let _running = hold_running_stoppable(&engine);
    let syncing_at_start = Arc::new(Mutex::new(Vec::new()));
    let key = TestKey::generate();
    let release = Publish::new(Line::Stable, "0.9.2", testkit::artifact_bytes(4096));
    let options = Options {
        hand: Hand::Engine {
            engine: engine.clone(),
            starts: false,
            syncing_at_start: syncing_at_start.clone(),
        },
        ..Options::default()
    };
    let mut rig = rig_with(&key, "0.9.1", Channel::Stable, serving(&key, &[release]), options);
    settle(&mut rig.part);
    let offered = |press, says: Option<&str>| Update::Available {
        version: "0.9.2".into(),
        press,
        says: says.map(str::to_string),
    };
    assert_eq!(rig.part.update(), offered(Press::Restart, None));
    assert_eq!(about(&rig.part).notes.press, Press::Restart);
    assert!(
        engine.is_syncing() && rig.staged().is_empty(),
        "a check leaves sync running and stages nothing"
    );

    rig.part.restart_to_update();
    assert_eq!(rig.part.update(), offered(Press::Wait, Some(STARTING)));
    let (restarts, starts) = settle_counting(&mut rig.part);
    assert_eq!(
        *syncing_at_start.lock().unwrap(),
        [false],
        "sync had stopped when the installer was started"
    );
    assert!(!engine.is_syncing());
    assert!(matches!(
        rig.part.state_for_test(),
        Some(State::Failed {
            failure: Failure::InstallerDidNotStart,
            resume: Some(_)
        })
    ));
    assert_eq!(words(&rig.part), INSTALLER_DID_NOT_START);
    assert_eq!((restarts, starts), (0, 1), "no quit; sync asked back once");
    assert_eq!(rig.part.update(), offered(Press::Retry, Some(INSTALLER_DID_NOT_START)));
    assert_eq!(about(&rig.part).notes.press, Press::Retry);

    // Try again: the kept file is verified again and handed off again (sync is not running now: nothing to stop,
    // nothing asked back).
    rig.part.restart_to_update();
    assert_eq!(settle_counting(&mut rig.part), (0, 0));
    assert_eq!(
        *syncing_at_start.lock().unwrap(),
        [false, false],
        "a second start, of the file verified again"
    );
}

/// A started installer: the window is asked to quit, once (the installer waits for the app to exit).
#[test]
fn a_started_installer_quits_the_app_once() {
    let key = TestKey::generate();
    let release = Publish::new(Line::Stable, "0.9.2", testkit::artifact_bytes(4096));
    let mut rig = rig(&key, "0.9.1", Channel::Stable, serving(&key, &[release]));
    settle(&mut rig.part);
    assert!(rig.did().is_empty(), "a check hands nothing off");
    rig.part.restart_to_update();
    assert_eq!(settle_counting(&mut rig.part), (1, 0));
    assert_eq!(rig.did(), ["stop", "launch"]);
    assert_eq!(words(&rig.part), RESTARTING);
    assert!(!rig.part.pump().restart, "once");
    rig.part.restart_to_update();
    assert!(!rig.part.pump().busy, "no second press once the installer started");
    assert_eq!(rig.did(), ["stop", "launch"]);
}

/// The shell's part, through `frames::settings::poll` as the shell runs it: a started installer quits the app; an
/// installer that did not start after sync was stopped starts sync again with the shell's own start.
#[test]
fn the_window_quits_for_a_started_installer_and_starts_sync_after_a_failed_one() {
    use crate::shell::actions::{FixtureCall, FixtureEngine};
    fn window(starts: bool) -> (rune::Runtime<crate::State>, Arc<FixtureEngine>, tempfile::TempDir) {
        let key = TestKey::generate();
        let release = Publish::new(Line::Stable, "0.9.2", testkit::artifact_bytes(4096));
        let options = Options {
            hand: Hand::Recording { starts },
            ..Options::default()
        };
        let rig = rig_with(&key, "0.9.1", Channel::Stable, serving(&key, &[release]), options);
        let engine = engine();
        let fixture = Arc::new(FixtureEngine::ready());
        let shell = crate::shell::host::Shell::fixture_on(fixture.clone(), crate::shell::platform::current());
        let state = crate::shell::state::init_on_engine(shell, engine.clone());
        let env = rune::Env {
            system_dark: false,
            scale: 2.0,
        };
        let mut rt = rune::Runtime::new(state, crate::view, (958.0, 638.0), env);
        rt.poll = Some(|s: &mut crate::State, _: f64, tx: &mut rune::Tx| crate::frames::settings::poll(s, tx));
        let source = EngineSettingsSource::for_identity(engine, &TEST).with_updates(Some(rig.part));
        rt.update(move |s, _| s.settings.source = Box::new(source));
        // The rig's staging folder lives as long as the window.
        (rt, fixture, rig.staging)
    }
    fn press_when(rt: &mut rune::Runtime<crate::State>, press: Press) {
        let began = Instant::now();
        loop {
            let ready = matches!(
                rt.state.settings.source.general().update,
                crate::frames::settings::source::Update::Available { press: shown, .. } if shown == press
            );
            if ready {
                break;
            }
            assert!(began.elapsed() < Duration::from_secs(10), "never {press:?}");
            std::thread::sleep(Duration::from_millis(5));
            rt.frame(0.05);
        }
        rt.update(move |s, _| s.settings.press_update(press));
    }
    fn wait(rt: &mut rune::Runtime<crate::State>, what: &str, done: impl Fn(&rune::Runtime<crate::State>) -> bool) {
        let began = Instant::now();
        while !done(rt) {
            assert!(began.elapsed() < Duration::from_secs(10), "{what}");
            std::thread::sleep(Duration::from_millis(5));
            rt.frame(0.05);
        }
    }

    let (mut rt, fixture, _staging) = window(true);
    press_when(&mut rt, Press::Restart);
    wait(&mut rt, "the app quits", |rt| {
        rt.shell_request == Some(rune::Shell::Quit)
    });
    assert!(!fixture.calls().contains(&FixtureCall::StartSync));

    let (mut rt, fixture, _staging) = window(false);
    press_when(&mut rt, Press::Restart);
    wait(&mut rt, "sync is started again", |_| {
        fixture.calls().contains(&FixtureCall::StartSync)
    });
    assert_eq!(rt.shell_request, None, "no quit");
}

/// Before the launch check's first report About says Checking…, and the window keeps looking (it is not woken
/// otherwise). The updater here waits at a gate inside its first step: no report can have come.
#[test]
fn the_window_keeps_looking_before_the_first_report() {
    struct Held(Gate);
    impl Drive for Held {
        fn step(&mut self, _event: Event) -> State {
            self.0.pass();
            State::Idle
        }
        fn background_check_due(&self) -> bool {
            false
        }
        fn freshness(&self) -> Freshness {
            Freshness::Verified
        }
        fn last_verified(&self) -> Option<u64> {
            None
        }
    }
    let gate = Gate::default();
    gate.close();
    let held = gate.clone();
    let make: MakeDrive = Box::new(move |_| Box::new(Held(held)) as Box<dyn Drive>);
    let mut part = UpdatesPart::live(make, &TEST, Channel::Stable, Channel::Stable, "0.9.1".into());
    let pumped = part.pump();
    assert!(pumped.busy, "busy before the first report");
    assert_eq!(about(&part).check, UpdateCheck::Checking);
    gate.open();
    settle(&mut part);
    assert!(!part.pump().busy, "and not after the answer");
}

/// "checked <time>" is the time of the last VERIFIED check, from the updater's record: a background check that gets
/// no answer keeps the offer on screen and does not make it look newer.
#[test]
fn checked_says_the_time_of_the_last_verified_check() {
    let key = TestKey::generate();
    let release = Publish::new(Line::Stable, "0.9.2", testkit::artifact_bytes(64));
    let options = Options {
        look_every: Duration::from_millis(5),
        ..Options::default()
    };
    let mut rig = rig_with(&key, "0.9.1", Channel::Stable, serving(&key, &[release]), options);
    settle(&mut rig.part);
    let verified = DateTime::from_timestamp(i64::try_from(testkit::NOW).unwrap(), 0)
        .unwrap()
        .with_timezone(&Local);
    let said = format!("Version 0.9.2 is available · checked {}", when(verified, Local::now()));
    assert_eq!(words(&rig.part), said, "the launch check's own time");

    // Five hours on, and offline: a background check is due, runs, gets no answer, and keeps the offer.
    let asked = rig.seen().len();
    rig.offline.store(true, Ordering::SeqCst);
    rig.clock.fetch_add(5 * 3_600, Ordering::SeqCst);
    until(&mut rig.part, "the background check ran", |_| {
        rig.seen.lock().unwrap().len() > asked
    });
    settle(&mut rig.part);
    assert_eq!(words(&rig.part), said, "not restamped by a check that got no answer");
}

/// The worker asks the updater whether a background check is due every `look_every`, and checks only when it is:
/// one that checked at every look would ask the release server every minute. Counted, not slept.
#[test]
fn a_background_check_runs_only_when_the_updater_says_one_is_due() {
    struct Counting {
        due: Arc<AtomicBool>,
        checks: Arc<AtomicUsize>,
        looks: Arc<AtomicUsize>,
    }
    impl Drive for Counting {
        fn step(&mut self, event: Event) -> State {
            if matches!(event, Event::Check { .. }) {
                self.checks.fetch_add(1, Ordering::SeqCst);
                self.due.store(false, Ordering::SeqCst);
            }
            State::Idle
        }
        fn background_check_due(&self) -> bool {
            self.looks.fetch_add(1, Ordering::SeqCst);
            self.due.load(Ordering::SeqCst)
        }
        fn freshness(&self) -> Freshness {
            Freshness::Verified
        }
        fn last_verified(&self) -> Option<u64> {
            None
        }
    }
    let (due, checks, looks) = (
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AtomicUsize::new(0)),
    );
    let counting = Counting {
        due: due.clone(),
        checks: checks.clone(),
        looks: looks.clone(),
    };
    let make: MakeDrive = Box::new(move |_| Box::new(counting) as Box<dyn Drive>);
    let tuning = Tuning {
        look_every: Duration::from_millis(1),
        wake: Box::new(|| {}),
    };
    let mut part = UpdatesPart::live_tuned(
        make,
        &TEST,
        Channel::Stable,
        Channel::Stable,
        "0.9.1".into(),
        tuning,
        Presence::open(),
    );
    let looked = |n: usize| {
        let looks = looks.clone();
        move |_: &UpdatesPart| looks.load(Ordering::SeqCst) >= n
    };
    until(&mut part, "20 looks", looked(20));
    assert_eq!(
        checks.load(Ordering::SeqCst),
        1,
        "the launch check alone while none is due"
    );
    due.store(true, Ordering::SeqCst);
    let checked = checks.clone();
    until(&mut part, "the due check", move |_| checked.load(Ordering::SeqCst) == 2);
    let after = looks.load(Ordering::SeqCst) + 20;
    until(&mut part, "20 more looks", looked(after));
    assert_eq!(
        checks.load(Ordering::SeqCst),
        2,
        "one check when due, then none until the next is due"
    );
}

/// The release glue, as the Windows release path calls it: a release builds the pinned updater with its record in
/// this app's folder; a build that is not a release has none (and asks nothing); a release that cannot set its
/// updater up says the updater did not start (never only the versions).
#[test]
fn the_release_glue_builds_the_pinned_updater_or_says_why_not() {
    let folder = tempfile::tempdir().expect("a temp dir");
    let place = || Some(folder.path().to_path_buf());
    // A build made a day after the key: `release_setup` passing the key's creation time in place of the build's
    // would show here (review of #441).
    let built = beebeeb_updater::V2_KEY_CREATED + 86_400;
    assert!(matches!(
        release_setup(None, Some("0.9.1"), place()),
        ReleaseSetup::NotARelease
    ));
    assert!(matches!(
        release_setup(Some(built), None, place()),
        ReleaseSetup::NotARelease
    ));
    assert!(
        matches!(release_setup(Some(built), Some("0.9.1"), None), ReleaseSetup::Broken),
        "no folder"
    );
    assert!(matches!(
        release_setup(Some(built), Some("test build"), place()),
        ReleaseSetup::Broken
    ));
    let ReleaseSetup::Ready { config, record } = release_setup(Some(built), Some("0.9.1"), place()) else {
        panic!("a release build");
    };
    assert_eq!(record, folder.path().join("updater.json"));
    assert_eq!(config.staging_dir, folder.path().join("updates"));
    assert_eq!(config.base_url, "https://releases.beebeeb.io");
    assert_eq!(config.trust.not_before, built, "the build's own time");
    assert_eq!(config.trust.keys.len(), 1);
    assert_eq!(config.installed.to_string(), "0.9.1");

    let broken = UpdatesPart::with_mode(Mode::NoWorker, &TEST, Channel::Stable, Channel::Stable, "0.9.1".into());
    assert_eq!(words(&broken), NO_UPDATER);
}

/// A download the release server did not send (here a 404 for the file): the words say the update did not come,
/// never that a check failed or that the channel has no update information; the button offers "Try again".
#[test]
fn a_download_the_server_did_not_send_says_so() {
    use crate::frames::settings::source::Update;
    let key = TestKey::generate();
    let release = Publish::new(Line::Stable, "0.9.2", testkit::artifact_bytes(4096));
    let artifact = release.url.clone();
    let mut http = serving(&key, &[release]);
    http.files.remove(&artifact);
    let mut rig = rig(&key, "0.9.1", Channel::Stable, http);
    settle(&mut rig.part);
    rig.part.restart_to_update();
    settle(&mut rig.part);
    assert!(
        matches!(
            rig.part.state_for_test(),
            Some(State::Failed {
                failure: Failure::ServerStatus(404),
                resume: Some(_)
            })
        ),
        "{:?}",
        rig.part.state_for_test()
    );
    assert_eq!(words(&rig.part), NOT_SENT);
    assert_eq!(
        rig.part.update(),
        Update::Available {
            version: "0.9.2".into(),
            press: Press::Retry,
            says: Some(NOT_SENT.into()),
        }
    );
    assert!(rig.did().is_empty());
}

/// The caption says each step of the press while it runs, never "Checking…" for the whole of it: the download (held at
/// the release server), then the hand-off (held in the stop: the verified file is reported before the stop runs), then
/// the restart. Meanwhile the button waits, Check now asks nothing (About's button waits on `Working`), and a channel
/// is refused with words: neither could come after the press and blank what it ends on.
#[test]
fn the_press_says_each_step_and_takes_no_other_order_meanwhile() {
    use crate::frames::settings::source::Update;
    let key = TestKey::generate();
    let release = Publish::new(Line::Stable, "0.9.2", testkit::artifact_bytes(4096));
    let artifact = release.url.clone();
    let mut rig = rig(&key, "0.9.1", Channel::Stable, serving(&key, &[release]));
    settle(&mut rig.part);
    let asked = rig.seen().len();
    let waiting = |rig: &Rig| match rig.part.update() {
        Update::Available { press, says, .. } => (press, says.unwrap_or_default()),
        Update::Current => panic!("the offer is gone"),
    };

    rig.gate.close();
    rig.part.restart_to_update();
    until(&mut rig.part, "the download is said", |part| {
        words(part).starts_with("Downloading version 0.9.2: ")
    });
    assert_eq!(waiting(&rig).0, Press::Wait);
    assert!(
        matches!(about(&rig.part).check, UpdateCheck::Working(_)),
        "Check now waits"
    );
    rig.part.check_now();
    rig.part.set_channel(Channel::Beta);
    assert_eq!(rig.part.pump().toast, Some(UPDATE_UNDER_WAY));
    assert_eq!(about(&rig.part).channel, Channel::Stable);

    rig.stop_gate.close();
    rig.gate.open();
    until(&mut rig.part, "the hand-off is said", |part| words(part) == HANDING_OFF);
    assert_eq!(waiting(&rig), (Press::Wait, HANDING_OFF.to_string()));
    assert!(rig.did().is_empty(), "held in the stop");
    rig.stop_gate.open();
    assert_eq!(settle_counting(&mut rig.part), (1, 0));
    assert_eq!(words(&rig.part), RESTARTING);
    assert_eq!(rig.did(), ["stop", "launch"]);
    let after: Vec<String> = rig.seen()[asked..].to_vec();
    assert_eq!(
        after,
        [artifact],
        "the file alone: no check, no channel came in between"
    );
}

/// Only the press downloads and hands off: background checks (due every few milliseconds here, five hours on) find the
/// newer version and say it, and never ask for the file nor stop sync; after a press that failed at the start of the
/// installer, a background check keeps the kept download in sight and does not run the press again.
#[test]
fn a_background_check_never_downloads_nor_hands_off() {
    let key = TestKey::generate();
    let release = Publish::new(Line::Stable, "0.9.2", testkit::artifact_bytes(4096));
    let artifact = release.url.clone();
    let options = Options {
        hand: Hand::Recording { starts: false },
        look_every: Duration::from_millis(2),
        ..Options::default()
    };
    let mut rig = rig_with(&key, "0.9.1", Channel::Stable, serving(&key, &[release]), options);
    settle(&mut rig.part);
    // One background check each: five hours on, it is due (the last verified check is four hours old), and once it
    // ran it is not due again until the clock moves on.
    let background = |rig: &mut Rig, what: &str| {
        let asked = rig.seen().len();
        rig.clock.fetch_add(5 * 3_600, Ordering::SeqCst);
        until(&mut rig.part, what, |_| rig.seen.lock().unwrap().len() >= asked + 2);
        settle(&mut rig.part);
    };
    background(&mut rig, "a background check");
    background(&mut rig, "a second background check");
    assert!(!rig.seen().contains(&artifact), "never downloaded by a check");
    assert!(rig.did().is_empty() && rig.staged().is_empty());
    assert!(words(&rig.part).starts_with("Version 0.9.2 is available"));

    rig.part.restart_to_update();
    settle(&mut rig.part);
    assert_eq!(rig.did(), ["stop", "launch", "resume"], "the press, once");
    let fetched = rig.seen().iter().filter(|url| **url == artifact).count();
    background(&mut rig, "a background check after the press");
    background(&mut rig, "another background check after the press");
    assert_eq!(rig.did(), ["stop", "launch", "resume"], "no hand-off by a check");
    assert_eq!(
        rig.seen().iter().filter(|url| **url == artifact).count(),
        fetched,
        "no download by a check"
    );
    assert_eq!(
        words(&rig.part),
        INSTALLER_DID_NOT_START,
        "the kept download stays in sight"
    );
}

/// A stop of sync that was not confirmed EARLIER in this process (a vault lock's sets the account's
/// `engine_stop_unconfirmed`, and leaves the engine slot empty): the hand-off mints no proof, stops nothing and starts
/// nothing, and the words say only what is known. The press then waits, also after a check that offers the version
/// again: no press can install until Beebeeb restarts.
#[test]
fn any_unconfirmed_stop_in_this_process_refuses_the_hand_off() {
    use crate::frames::settings::source::Update;
    let engine = engine();
    let account = engine.accounts().active_account().expect("the account");
    let _running = hold_running_stoppable(&engine);
    account.engine_stop_unconfirmed.store(true, Ordering::SeqCst);

    let sync_back = SyncBack::default();
    let never: StartInstaller = Box::new(|_, _| panic!("no installer starts here"));
    let mut apply = EngineApply::starting_with(engine.clone(), &TEST, sync_back.clone(), pressed(), never);
    assert!(apply.stop_engine().is_none(), "no proof after an unconfirmed stop");
    assert!(engine.is_syncing(), "nothing was stopped");
    apply.resume_engine();
    drop(apply);
    assert!(!sync_back.take(), "nothing asked back");

    // The press asks the same (review of #444, P3): Wait and the words before anything is downloaded.
    let syncing_at_start = Arc::new(Mutex::new(Vec::new()));
    let key = TestKey::generate();
    let release = Publish::new(Line::Stable, "0.9.2", testkit::artifact_bytes(4096));
    let artifact = release.url.clone();
    let asked = engine.clone();
    let options = Options {
        hand: Hand::Engine {
            engine: engine.clone(),
            starts: true,
            syncing_at_start: syncing_at_start.clone(),
        },
        presence: Presence::from_fns(
            || (Some(SessionGeneration::for_test(1)), true),
            || 0,
            move || crate::engine_host::settings::updates::presence::any_stop_unconfirmed(&asked),
        ),
        ..Options::default()
    };
    let mut rig = rig_with(&key, "0.9.1", Channel::Stable, serving(&key, &[release]), options);
    settle(&mut rig.part);
    let waits = Update::Available {
        version: "0.9.2".into(),
        press: Press::Wait,
        says: Some(SYNC_UNCONFIRMED.into()),
    };
    rig.part.restart_to_update();
    assert_eq!(rig.part.update(), waits, "said at the press");
    assert!(!rig.part.pump().busy, "the press sends nothing");
    assert_eq!(settle_counting(&mut rig.part), (0, 0), "no quit, nothing asked back");
    assert!(!rig.seen().contains(&artifact), "nothing downloaded");
    assert!(syncing_at_start.lock().unwrap().is_empty(), "no installer started");
    assert!(engine.is_syncing());

    rig.part.check_now();
    settle(&mut rig.part);
    assert!(words(&rig.part).starts_with("Version 0.9.2 is available"));
    assert_eq!(
        rig.part.update(),
        waits,
        "a check does not make the press possible again"
    );
    rig.part.restart_to_update();
    assert!(!rig.part.pump().busy, "the press sends nothing");
    assert!(!rig.seen().contains(&artifact), "nothing downloaded");
    account.engine_stop_unconfirmed.store(false, Ordering::SeqCst);
}

/// The worker ends in the middle of the hand-off (the installer's start panics, after sync was stopped for it): the
/// sync it stopped is asked back, once, and About says the updater did not start; no quit.
#[test]
fn a_worker_that_ends_in_the_hand_off_asks_for_sync_back() {
    let engine = engine();
    let _running = hold_running_stoppable(&engine);
    let key = TestKey::generate();
    let release = Publish::new(Line::Stable, "0.9.2", testkit::artifact_bytes(4096));
    let options = Options {
        hand: Hand::EnginePanicsAtStart { engine: engine.clone() },
        ..Options::default()
    };
    let mut rig = rig_with(&key, "0.9.1", Channel::Stable, serving(&key, &[release]), options);
    settle(&mut rig.part);
    rig.part.restart_to_update();
    let (mut restarts, mut starts) = (0, 0);
    let began = Instant::now();
    while words(&rig.part) != NO_UPDATER || rig.part.pump().busy {
        assert!(began.elapsed() < Duration::from_secs(10), "{}", words(&rig.part));
        let pumped = rig.part.pump();
        restarts += usize::from(pumped.restart);
        starts += usize::from(pumped.start_sync);
        std::thread::sleep(Duration::from_millis(5));
    }
    let pumped = rig.part.pump();
    starts += usize::from(pumped.start_sync);
    assert_eq!(
        (restarts, starts),
        (0, 1),
        "no quit; the sync it stopped asked back once"
    );
    assert!(!engine.is_syncing(), "it had been stopped");
}
