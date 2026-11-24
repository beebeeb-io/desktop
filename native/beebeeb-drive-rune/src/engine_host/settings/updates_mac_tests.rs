//! A Mac's About updates: the check picks the `darwin-aarch64` entry, the one button is "Download update" and it opens
//! the download page; "Restart to update" is never taken and nothing is downloaded or installed. The page is opened by
//! a recording double (no test opens a browser), the release server is the testkit's map in memory.

use std::cell::Cell;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use beebeeb_engine::identity::TEST;
use beebeeb_updater::testkit::{self, FakeClock, FakeHttp, MemStore, Publish, TestKey};
use beebeeb_updater::{Channel as Line, Config, Event, Failure, State, Updater};

use super::*;

type Opened = Arc<Mutex<Vec<String>>>;

fn mac_part(key: &TestKey, installed: &str, publish: &Publish) -> (UpdatesPart, Opened) {
    let mut http = FakeHttp::default();
    publish.serve(&mut http, key);
    let config = Config {
        trust: testkit::darwin_trust(key),
        base_url: testkit::BASE.to_string(),
        installed: testkit::version(installed),
        staging_dir: tempfile::tempdir().expect("a temp dir").keep().join("updates"),
    };
    let opened = Opened::default();
    let recording = opened.clone();
    let make: MakeDrive = Box::new(move |_sync_back| {
        let apply = MacApply::opening_with(Box::new(move |address| {
            recording.lock().unwrap().push(address.to_string());
            true
        }));
        Box::new(Watching::new(
            config,
            http,
            FakeClock(Cell::new(testkit::NOW)),
            MemStore::default(),
            apply,
        )) as Box<dyn Drive>
    });
    let tuning = Tuning {
        look_every: Duration::from_secs(60),
        wake: Box::new(|| {}),
    };
    let part = UpdatesPart::live_tuned(
        make,
        &TEST,
        Channel::Stable,
        Channel::Stable,
        installed.to_string(),
        tuning,
        Presence::open(),
    )
    .download_only();
    (part, opened)
}

fn until(part: &mut UpdatesPart, what: &str, done: impl Fn(&UpdatesPart) -> bool) {
    let began = Instant::now();
    loop {
        let pumped = part.pump();
        if !pumped.busy && done(part) {
            return;
        }
        assert!(began.elapsed() < Duration::from_secs(10), "{what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn available(part: &UpdatesPart) -> bool {
    matches!(part.state_for_test(), Some(State::Available { .. }))
}

#[test]
fn a_mac_shows_download_update_and_never_restart() {
    let key = TestKey::generate();
    let release = Publish::for_darwin(Line::Stable, "0.9.1", testkit::artifact_bytes(4096));
    let (mut part, opened) = mac_part(&key, "0.9.0", &release);
    until(&mut part, "the offer", available);

    assert_eq!(part.button(), "Download update");
    assert_ne!(part.button(), "Restart to update");
    let Update::Available { version, .. } = part.update() else {
        panic!("a newer version is on offer")
    };
    assert_eq!(version, "0.9.1");

    // The press "Restart to update" does nothing on a Mac: no order, no download, no hand-off.
    part.restart_to_update();
    until(&mut part, "still the offer", available);
    assert!(opened.lock().unwrap().is_empty());

    // "Download update" opens the release's page, once; the offer stays and the app stays.
    part.download_update();
    until(&mut part, "the page opened", |_| opened.lock().unwrap().len() == 1);
    until(&mut part, "still the offer", available);
    assert_eq!(*opened.lock().unwrap(), [testkit::darwin_artifact_url("0.9.1")]);
    assert!(!part.pump().restart, "the app does not quit");
}

#[test]
fn nothing_to_download_when_the_manifest_has_no_mac_build_or_no_newer_version() {
    let key = TestKey::generate();
    let windows_only = Publish::new(Line::Stable, "0.9.1", testkit::artifact_bytes(4096));
    let (mut part, opened) = mac_part(&key, "0.9.0", &windows_only);
    until(&mut part, "the refusal", |part| {
        matches!(part.state_for_test(), Some(State::Failed { .. }))
    });
    part.download_update();
    part.pump();
    assert!(opened.lock().unwrap().is_empty());

    let current = Publish::for_darwin(Line::Stable, "0.9.1", testkit::artifact_bytes(4096));
    let (mut part, opened) = mac_part(&key, "0.9.1", &current);
    until(&mut part, "up to date", |part| {
        matches!(part.state_for_test(), Some(State::UpToDate { .. }))
    });
    part.download_update();
    part.pump();
    assert!(opened.lock().unwrap().is_empty());
}

#[test]
fn the_refusal_and_a_page_that_did_not_open_have_their_words() {
    assert_eq!(
        beebeeb_updater::install_unsupported_message(),
        "update_install_unsupported: Beebeeb on a Mac downloads updates and cannot install them in place."
    );
    let words = |failure| failed(failure, Freshness::Verified);
    assert!(
        matches!(words(Failure::InstallUnsupported), UpdateCheck::Checked(text) if text == beebeeb_updater::INSTALL_UNSUPPORTED)
    );
    assert!(matches!(words(Failure::DownloadPageDidNotOpen), UpdateCheck::Checked(text) if text == PAGE_NOT_OPENED));
}

#[test]
fn a_mac_trust_takes_the_darwin_target_only() {
    let key = TestKey::generate();
    let mut http = FakeHttp::default();
    Publish::for_darwin(Line::Stable, "0.9.1", testkit::artifact_bytes(64)).serve(&mut http, &key);
    let config = Config {
        trust: testkit::darwin_trust(&key),
        base_url: testkit::BASE.to_string(),
        installed: testkit::version("0.9.0"),
        staging_dir: tempfile::tempdir().expect("a temp dir").keep(),
    };
    let mut updater = Updater::new(
        config,
        http,
        FakeClock(Cell::new(testkit::NOW)),
        MemStore::default(),
        MacApply::opening_with(Box::new(|_| true)),
    );
    updater.step(Event::Check {
        channel: Line::Stable,
        intent: beebeeb_updater::Intent::Manual,
    });
    assert!(matches!(updater.step(Event::Tick), State::Available { .. }));
    assert!(matches!(
        updater.step(Event::Apply),
        State::Failed {
            failure: Failure::InstallUnsupported,
            ..
        }
    ));
}
