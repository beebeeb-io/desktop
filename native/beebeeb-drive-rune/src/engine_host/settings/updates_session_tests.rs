//! The press runs on by itself, so the hand-off runs only for the person who pressed (review of #444): a lock, or a
//! sign-out, while the download is held at the release server installs nothing (no sync stop, no installer, no quit),
//! keeps the verified file, says it waits, and offers the press again once the lock is open; a lock during the
//! hand-off's own stop starts no installer and asks the sync it ended back. Also the review's P3 items: a stop of
//! another account left unconfirmed DURING this stop, a second press while the order runs, and "Try again" pinned to
//! the version pressed. On a real engine signed in and unlocked against the engine's fake API; every name invented.
//!
//! Wave 2 (the fix round's review, window (c)): a sign-out or a vault lock that STARTS during the hand-off's stop
//! waits for the engine's step lock that stop holds, and moves the turn only once it runs; it is counted before it
//! waits, so no installer starts. A hand-off refused that way says the update waits for the person and offers the
//! press; an installer that really did not start says so, with "Try again".

use beebeeb_engine::app::lock_api::LockState;
use beebeeb_engine::test_api::FakeApi;
use beebeeb_engine::test_api::fake_account::{FakeAccount, TEST_PHRASE, account_handler};
use zeroize::Zeroizing;

use super::handoff::hold_running_stoppable;
use super::*;

const EMAIL: &str = "updates@example.eu";
const PASSWORD: &str = "an invented password for updates";

/// A real engine, signed in and unlocked on a fake account of its own: its lock is open.
fn signed_in() -> (Arc<Engine<MemoryCredentials>>, FakeApi) {
    let (account, phrase) = FakeAccount::register(EMAIL, PASSWORD, TEST_PHRASE);
    let api = FakeApi::start(account_handler(Arc::new(account)));
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "updates-presence-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        api_base: api.base.clone(),
    };
    let engine = Arc::new(Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens"));
    engine.restore();
    engine
        .sign_in_password(EMAIL, Zeroizing::new(PASSWORD.into()))
        .expect("the fake accepts the password");
    engine
        .unlock_with_phrase(Zeroizing::new(phrase))
        .expect("the phrase unlocks");
    assert_eq!(engine.lock_state(), LockState::Open);
    (engine, api)
}

/// A rig on `engine`'s presence, with a recording hand-off whose installer starts.
fn rig_on(engine: &Arc<Engine<MemoryCredentials>>, key: &TestKey) -> Rig {
    let release = Publish::new(Line::Stable, "0.9.2", testkit::artifact_bytes(4096));
    let options = Options {
        presence: Presence::of_engine(engine.clone()),
        ..Options::default()
    };
    rig_with(key, "0.9.1", Channel::Stable, serving(key, &[release]), options)
}

/// Presses, and waits until the download is said while the release server holds the file.
fn press_held(rig: &mut Rig) {
    rig.gate.close();
    rig.part.restart_to_update();
    until(&mut rig.part, "the download is said", |part| {
        words(part).starts_with("Downloading version 0.9.2: ")
    });
}

/// After a press whose hand-off waited: nothing stopped, nothing started, no quit, and the verified file kept.
fn nothing_handed_off(rig: &mut Rig) {
    rig.gate.open();
    assert_eq!(settle_counting(&mut rig.part), (0, 0), "no quit, nothing asked back");
    assert!(rig.did().is_empty(), "no sync stop, no installer: {:?}", rig.did());
    assert!(
        matches!(rig.part.state_for_test(), Some(State::ReadyToRestart { .. })),
        "{:?}",
        rig.part.state_for_test()
    );
    assert!(
        rig.staged().contains(&testkit::artifact_name("0.9.2")),
        "the verified file is kept: {:?}",
        rig.staged()
    );
}

/// The press, then Beebeeb locks (by hand, or away) while the file is on its way: the hand-off waits for the person.
/// The press is offered again once the lock is open, and only that press installs.
#[test]
fn a_lock_while_the_press_downloads_installs_nothing_and_waits_for_the_person() {
    use crate::frames::settings::source::Update;
    let (engine, _api) = signed_in();
    let key = TestKey::generate();
    let mut rig = rig_on(&engine, &key);
    settle(&mut rig.part);
    press_held(&mut rig);
    engine.lock_screen().expect("the screen locks");
    nothing_handed_off(&mut rig);
    assert!(
        matches!(rig.part.update(), Update::Available { press: Press::Wait, .. }),
        "no press while locked"
    );

    engine
        .unlock_screen_platform_verified(engine.session_turn())
        .expect("the screen opens");
    let waits = format!("Version 0.9.2 is downloaded and verified. {WAITS_FOR_YOU}");
    assert_eq!(
        rig.part.update(),
        Update::Available {
            version: "0.9.2".into(),
            press: Press::Restart,
            says: Some(waits),
        },
        "offered again, with words"
    );
    rig.part.restart_to_update();
    assert_eq!(
        settle_counting(&mut rig.part),
        (1, 0),
        "the person's own press installs"
    );
    assert_eq!(rig.did(), ["stop", "launch"]);
}

/// The press, then a sign-out while the file is on its way: nothing is handed off, and no press is offered while
/// signed out.
#[test]
fn a_sign_out_while_the_press_downloads_installs_nothing() {
    use crate::frames::settings::source::Update;
    let (engine, _api) = signed_in();
    let key = TestKey::generate();
    let mut rig = rig_on(&engine, &key);
    settle(&mut rig.part);
    press_held(&mut rig);
    engine.sign_out_keeping_local_data().expect("signs out");
    nothing_handed_off(&mut rig);
    assert!(
        matches!(rig.part.update(), Update::Available { press: Press::Wait, .. }),
        "no press while signed out"
    );
    rig.part.restart_to_update();
    assert!(!rig.part.pump().busy, "the press sends nothing");
}

/// What a press whose hand-off ran on the real engine left: the quits and the asks for sync back, whether sync ran at
/// each installer start, what the step `during` answered, and how long it took from the moment the hand-off's stop
/// began.
struct DuringTheStop<T> {
    rig: Rig,
    restarts: usize,
    starts: usize,
    syncing_at_start: Vec<bool>,
    answer: T,
    took: Duration,
}

/// A press on a running sync (its stop takes seconds) whose hand-off is the real one on `engine`, its installer
/// starting; `during` runs on a thread of its own the moment the hand-off's stop began (never the window's thread:
/// it blocks on the engine's runtime).
fn press_with_during_the_stop<T: Send + 'static>(
    engine: &Arc<Engine<MemoryCredentials>>,
    during: impl FnOnce(&Engine<MemoryCredentials>) -> T + Send + 'static,
) -> DuringTheStop<T> {
    let running = hold_running_stoppable(engine);
    let syncing_at_start = Arc::new(Mutex::new(Vec::new()));
    let key = TestKey::generate();
    let release = Publish::new(Line::Stable, "0.9.2", testkit::artifact_bytes(4096));
    let options = Options {
        hand: Hand::Engine {
            engine: engine.clone(),
            starts: true,
            syncing_at_start: syncing_at_start.clone(),
        },
        presence: Presence::of_engine(engine.clone()),
        ..Options::default()
    };
    let mut rig = rig_with(&key, "0.9.1", Channel::Stable, serving(&key, &[release]), options);
    settle(&mut rig.part);
    let asker = engine.clone();
    let step = std::thread::spawn(move || {
        let began = Instant::now();
        while matches!(asker.view().start, Start::Running) {
            assert!(began.elapsed() < Duration::from_secs(10), "the stop never began");
            std::thread::sleep(Duration::from_millis(2));
        }
        let began = Instant::now();
        let answer = during(&asker);
        (answer, began.elapsed())
    });
    rig.part.restart_to_update();
    let (restarts, starts) = settle_counting(&mut rig.part);
    let (answer, took) = step.join().expect("the step's thread");
    drop(running);
    let syncing_at_start = syncing_at_start.lock().unwrap().clone();
    DuringTheStop {
        rig,
        restarts,
        starts,
        syncing_at_start,
        answer,
        took,
    }
}

/// A screen lock DURING the hand-off's stop (a running sync's stop takes seconds): no installer starts, and the sync
/// the stop ended is asked back, once; no quit. The row says the update waits for the person, and once the lock is
/// open again the press is offered ("Restart to update"), not "Try again".
#[test]
fn a_lock_during_the_stop_starts_no_installer_and_asks_sync_back() {
    use crate::frames::settings::source::Update;
    let (engine, _api) = signed_in();
    let done = press_with_during_the_stop(&engine, |engine| {
        engine.lock_screen().expect("the screen locks during the stop");
    });
    assert_eq!(
        (done.restarts, done.starts),
        (0, 1),
        "no quit; the sync the stop ended asked back once"
    );
    assert!(done.syncing_at_start.is_empty(), "no installer started");
    let waits = format!("Version 0.9.2 is downloaded and verified. {WAITS_FOR_YOU}");
    assert_eq!(words(&done.rig.part), waits);
    engine
        .unlock_screen_platform_verified(engine.session_turn())
        .expect("the screen opens");
    assert_eq!(
        done.rig.part.update(),
        Update::Available {
            version: "0.9.2".into(),
            press: Press::Restart,
            says: Some(waits),
        },
        "the press is offered with the words it promises"
    );
}

/// A sign-out pressed DURING the hand-off's stop waits for the step lock that stop holds: it is counted before it
/// waits, so no installer starts and there is no quit; the sync the stop ended is asked back once, and the sign-out
/// finishes after the stop.
#[test]
fn a_sign_out_during_the_stop_starts_no_installer_and_the_sign_out_finishes() {
    let (engine, _api) = signed_in();
    let turn = engine.session_turn();
    let done = press_with_during_the_stop(&engine, move |engine| engine.sign_out_for(turn));
    eprintln!(
        "wave2-s1 stop: the sign-out returned {:?} after the stop began",
        done.took
    );
    assert_eq!(
        (done.restarts, done.starts),
        (0, 1),
        "no quit; the sync the stop ended asked back once"
    );
    assert!(done.syncing_at_start.is_empty(), "no installer started");
    assert_eq!(done.answer, Ok(()), "the sign-out finished");
    assert_eq!(engine.lock_state(), LockState::SignedOut);
}

/// A vault lock DURING the hand-off's stop: as the sign-out, and the closed mark it writes stays.
#[test]
fn a_vault_lock_during_the_stop_starts_no_installer_and_the_lock_finishes() {
    let (engine, _api) = signed_in();
    let ask_at_start = engine.ask_password_at_start();
    let done = press_with_during_the_stop(&engine, |engine| engine.lock_vault().survives_restart);
    eprintln!(
        "wave2-s1 stop: the vault lock returned {:?} after the stop began",
        done.took
    );
    assert_eq!(
        (done.restarts, done.starts),
        (0, 1),
        "no quit; the sync the stop ended asked back once"
    );
    assert!(done.syncing_at_start.is_empty(), "no installer started");
    assert!(done.answer, "the closed mark is stored");
    assert_eq!(engine.lock_state(), LockState::VaultClosed);
    assert_eq!(engine.ask_password_at_start(), ask_at_start, "the setting is untouched");
}

/// The count of session-ending steps refuses the hand-off by itself: the turn and the lock as at the press, one step
/// asked since.
#[test]
fn presence_refuses_once_a_session_step_was_asked() {
    let steps = Arc::new(AtomicU64::new(5));
    let asked = steps.clone();
    let presence = Presence::from_fns(
        || (Some(SessionGeneration::for_test(1)), true),
        move || asked.load(Ordering::SeqCst),
        || false,
    );
    assert!(presence.press());
    assert!(presence.holds(), "nothing moved since the press");
    steps.store(6, Ordering::SeqCst);
    assert!(!presence.holds(), "a session-ending step was asked since the press");
}

/// On the real engine, deterministically: a sign-out that waits for the step lock (held here, as the hand-off's
/// stop holds it) has not moved the turn yet, and the hand-off is refused all the same. The sign-out then finishes.
#[test]
fn a_sign_out_asked_while_the_step_is_held_refuses_the_hand_off() {
    let (engine, _api) = signed_in();
    let presence = Presence::of_engine(engine.clone());
    assert!(presence.press());
    assert!(presence.holds());
    let (turn, before) = (engine.session_turn(), engine.session_steps_asked());
    let hold = engine.hold_session_step_for_test();
    let signer = engine.clone();
    let sign_out = std::thread::spawn(move || signer.sign_out_for(turn));
    let began = Instant::now();
    while engine.session_steps_asked() == before {
        assert!(
            began.elapsed() < Duration::from_secs(2),
            "the sign-out was not counted while it waited"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        engine.session_turn(),
        turn,
        "the sign-out still waits: the turn did not move"
    );
    assert_eq!(engine.lock_state(), LockState::Open, "and the lock is open");
    assert!(!presence.holds(), "the hand-off is refused");
    drop(hold);
    assert_eq!(sign_out.join().expect("the sign-out's thread"), Ok(()));
    assert_eq!(engine.lock_state(), LockState::SignedOut);
}

/// An installer that does not start while the person is there (nothing locked, nothing signed out): the row says so,
/// and the press is "Try again".
#[test]
fn an_installer_that_fails_with_the_person_present_says_it_did_not_start() {
    use crate::frames::settings::source::Update;
    let (engine, _api) = signed_in();
    let key = TestKey::generate();
    let release = Publish::new(Line::Stable, "0.9.2", testkit::artifact_bytes(4096));
    let syncing_at_start = Arc::new(Mutex::new(Vec::new()));
    let options = Options {
        hand: Hand::Engine {
            engine: engine.clone(),
            starts: false,
            syncing_at_start: syncing_at_start.clone(),
        },
        presence: Presence::of_engine(engine.clone()),
        ..Options::default()
    };
    let mut rig = rig_with(&key, "0.9.1", Channel::Stable, serving(&key, &[release]), options);
    settle(&mut rig.part);
    rig.part.restart_to_update();
    assert_eq!(settle_counting(&mut rig.part), (0, 0), "no quit; no sync was running");
    assert_eq!(
        syncing_at_start.lock().unwrap().len(),
        1,
        "the installer was asked to start"
    );
    assert_eq!(
        rig.part.update(),
        Update::Available {
            version: "0.9.2".into(),
            press: Press::Retry,
            says: Some(INSTALLER_DID_NOT_START.into()),
        }
    );
}

/// Another account's stop left unconfirmed DURING this hand-off's stop (the flag is set once the stop began): no
/// proof, and the sync this confirmed stop ended is asked back, once.
#[test]
fn a_stop_left_unconfirmed_during_this_stop_mints_no_proof_and_asks_sync_back() {
    let engine = engine();
    let account = engine.accounts().active_account().expect("the account");
    let _running = hold_running_stoppable(&engine);
    let marker = engine.clone();
    let mark = std::thread::spawn(move || {
        let began = Instant::now();
        while matches!(marker.view().start, Start::Running) {
            assert!(began.elapsed() < Duration::from_secs(10), "the stop never began");
            std::thread::sleep(Duration::from_millis(2));
        }
        let account = marker.accounts().active_account().expect("the account");
        account.engine_stop_unconfirmed.store(true, Ordering::SeqCst);
    });
    let sync_back = SyncBack::default();
    let never: StartInstaller = Box::new(|_, _| panic!("no installer starts here"));
    let mut apply = EngineApply::starting_with(engine.clone(), &TEST, sync_back.clone(), pressed(), never);
    let proof = apply.stop_engine();
    mark.join().expect("the mark thread");
    assert!(proof.is_none(), "no proof after a stop left unconfirmed meanwhile");
    assert!(!engine.is_syncing(), "this stop ran and was confirmed");
    assert!(sync_back.take(), "the sync it ended is asked back");
    assert!(!sync_back.take(), "once");
    drop(apply);
    assert!(!sync_back.take(), "and not again by the drop");
    account.engine_stop_unconfirmed.store(false, Ordering::SeqCst);
}

/// A second press while the order runs does nothing: one download, one hand-off, also when that hand-off fails
/// (here: the installer does not start, so a queued second order would hand off again by itself).
#[test]
fn a_second_press_while_the_order_runs_does_nothing() {
    let key = TestKey::generate();
    let release = Publish::new(Line::Stable, "0.9.2", testkit::artifact_bytes(4096));
    let artifact = release.url.clone();
    let options = Options {
        hand: Hand::Recording { starts: false },
        ..Options::default()
    };
    let mut rig = rig_with(&key, "0.9.1", Channel::Stable, serving(&key, &[release]), options);
    settle(&mut rig.part);
    press_held(&mut rig);
    rig.part.restart_to_update();
    rig.gate.open();
    settle(&mut rig.part);
    assert_eq!(rig.did(), ["stop", "launch", "resume"], "one hand-off");
    assert_eq!(
        rig.seen().iter().filter(|url| **url == artifact).count(),
        1,
        "one download"
    );
}

/// "Try again" after a dropped offer runs only for the version pressed: the channel now offers 0.9.3, so the order
/// stops at that offer (said, with its own press), and nothing of 0.9.3 is downloaded or handed off.
#[test]
fn try_again_after_a_dropped_offer_runs_only_for_the_version_pressed() {
    use crate::frames::settings::source::Update;
    let key = TestKey::generate();
    let release = Publish::new(Line::Stable, "0.9.2", testkit::artifact_bytes(4096));
    let file = release.url.clone();
    let mut http = serving(&key, &[release]);
    let mut other = testkit::artifact_bytes(4096);
    other[100] ^= 0xff;
    http.files.insert(file, other);
    let mut rig = rig(&key, "0.9.1", Channel::Stable, http);
    settle(&mut rig.part);
    rig.part.restart_to_update();
    settle(&mut rig.part);
    assert!(matches!(
        rig.part.update(),
        Update::Available {
            press: Press::Retry,
            ..
        }
    ));

    let newer = Publish::new(Line::Stable, "0.9.3", testkit::artifact_bytes(4096));
    let newer_file = newer.url.clone();
    *rig.next.lock().unwrap() = Some(serving(&key, &[newer]));
    rig.part.restart_to_update();
    settle(&mut rig.part);
    assert_eq!(
        rig.part.update(),
        Update::Available {
            version: "0.9.3".into(),
            press: Press::Restart,
            says: None,
        },
        "the order stopped at the new offer"
    );
    assert!(!rig.seen().contains(&newer_file), "0.9.3 was not downloaded");
    assert!(rig.did().is_empty(), "nothing handed off: {:?}", rig.did());
}
