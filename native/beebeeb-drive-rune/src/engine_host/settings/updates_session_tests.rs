//! The press runs on by itself, so the hand-off runs only for the person who pressed (review of #444): a lock, or a
//! sign-out, while the download is held at the release server installs nothing (no sync stop, no installer, no quit),
//! keeps the verified file, says it waits, and offers the press again once the lock is open; a lock during the
//! hand-off's own stop starts no installer and asks the sync it ended back. Also the review's P3 items: a stop of
//! another account left unconfirmed DURING this stop, a second press while the order runs, and "Try again" pinned to
//! the version pressed. On a real engine signed in and unlocked against the engine's fake API; every name invented.

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

/// A lock DURING the hand-off's stop (a running sync's stop takes seconds): no installer starts, and the sync the
/// stop ended is asked back, once; no quit.
#[test]
fn a_lock_during_the_stop_starts_no_installer_and_asks_sync_back() {
    let (engine, _api) = signed_in();
    let _running = hold_running_stoppable(&engine);
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
    let locker = engine.clone();
    let lock = std::thread::spawn(move || {
        let began = Instant::now();
        while matches!(locker.view().start, Start::Running) {
            assert!(began.elapsed() < Duration::from_secs(10), "the stop never began");
            std::thread::sleep(Duration::from_millis(2));
        }
        locker.lock_screen().expect("the screen locks during the stop");
    });
    rig.part.restart_to_update();
    let (restarts, starts) = settle_counting(&mut rig.part);
    lock.join().expect("the lock thread");
    assert_eq!(
        (restarts, starts),
        (0, 1),
        "no quit; the sync the stop ended asked back once"
    );
    assert!(syncing_at_start.lock().unwrap().is_empty(), "no installer started");
    assert_eq!(words(&rig.part), INSTALLER_DID_NOT_START);
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
