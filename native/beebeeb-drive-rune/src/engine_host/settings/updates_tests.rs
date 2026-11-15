//! About's updates on the updater (`UpdatesPart`), against the updater crate's own test doubles: a key made inside each
//! test, a scripted release server that is a map in memory (no socket, no network), a clock and a store. What is
//! checked: the words of each answer; that a check asks for the channel's manifest and nothing else, with nothing of
//! the person in it; that a file which is not the signed one is never handed off; that another host is refused (with
//! no request at all when it is the base); that an older build on a channel is said and never offered; that a build
//! which is not a release asks nothing; and the source's methods on the part.

use std::cell::Cell;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use beebeeb_engine::app::{Engine, EngineConfig};
use beebeeb_engine::host::ChannelHost;
use beebeeb_engine::identity::TEST;
use beebeeb_engine::signin::memory_store::MemoryCredentials;
use beebeeb_updater::testkit::{self, FakeClock, FakeHttp, MemStore, Publish, TestKey};
use beebeeb_updater::{
    Apply, Channel as Line, Config, EngineStopped, Event, Failure, Freshness, Http, Offer, Persisted, Refusal,
    Response, State, Unreachable, Updater, VerifiedInstaller,
};

use super::*;
use crate::engine_host::settings::{EngineSettingsSource, build_about};
use crate::frames::settings::source::SettingsSource;

/// Every address the fake release server was asked for, in order.
pub(crate) type Seen = Arc<Mutex<Vec<String>>>;

/// The testkit's server, with every request written down where the test can read it (the updater lives on the
/// part's worker thread).
struct Logged {
    inner: FakeHttp,
    seen: Seen,
}

impl Http for Logged {
    fn get(&mut self, url: &str, from: u64) -> Result<Response, Unreachable> {
        self.seen.lock().unwrap().push(url.to_string());
        self.inner.get(url, from)
    }
}

/// What the hand-off was asked to do, in order.
type Did = Arc<Mutex<Vec<&'static str>>>;

struct Recording {
    did: Did,
}

impl Apply for Recording {
    fn stop_engine(&mut self) -> Option<EngineStopped> {
        self.did.lock().unwrap().push("stop");
        Some(EngineStopped::engine_stopped_and_uploads_parked())
    }
    fn launch(&mut self, _installer: &VerifiedInstaller, _offer: &Offer, _stopped: EngineStopped) -> bool {
        self.did.lock().unwrap().push("launch");
        true
    }
    fn resume_engine(&mut self) {
        self.did.lock().unwrap().push("resume");
    }
}

pub(crate) struct Rig {
    pub(crate) part: UpdatesPart,
    pub(crate) seen: Seen,
    did: Did,
    staging: tempfile::TempDir,
}

impl Rig {
    fn seen(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
    fn did(&self) -> Vec<&'static str> {
        self.did.lock().unwrap().clone()
    }
    /// Every file in the staging folder (none: the folder was never made).
    fn staged(&self) -> Vec<String> {
        std::fs::read_dir(self.staging.path().join("updates"))
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// A part on a test updater that trusts `key`, runs `installed`, and asks the base `base`. Its record holds a check
/// that passed an hour ago (an install that checks), unless `verified` is `None` (one that never did).
fn rig_with(
    key: &TestKey,
    installed: &str,
    channel: Channel,
    http: FakeHttp,
    base: &str,
    verified: Option<u64>,
) -> Rig {
    let staging = tempfile::tempdir().expect("a temp dir");
    let (seen, did) = (Seen::default(), Did::default());
    let config = Config {
        trust: testkit::trust(key),
        base_url: base.to_string(),
        installed: testkit::version(installed),
        staging_dir: staging.path().join("updates"),
    };
    let (logged, recording) = (
        Logged {
            inner: http,
            seen: seen.clone(),
        },
        Recording { did: did.clone() },
    );
    let make: MakeDrive = Box::new(move || {
        let clock = FakeClock(Cell::new(testkit::NOW));
        let store = MemStore {
            persisted: Persisted {
                last_check: verified,
                ..Persisted::default()
            },
            refuse_writes: false,
        };
        Box::new(Updater::new(config, logged, clock, store, recording)) as Box<dyn Drive>
    });
    let part = UpdatesPart::live(make, &TEST, channel, channel, installed.to_string());
    Rig {
        part,
        seen,
        did,
        staging,
    }
}

fn rig_at(key: &TestKey, installed: &str, channel: Channel, http: FakeHttp, base: &str) -> Rig {
    rig_with(key, installed, channel, http, base, Some(testkit::NOW - 3_600))
}

fn rig(key: &TestKey, installed: &str, channel: Channel, http: FakeHttp) -> Rig {
    rig_at(key, installed, channel, http, testkit::BASE)
}

/// A rig whose Stable channel is at `installed` (for the window's test in `settings::tests`).
pub(crate) fn rig_up_to_date(installed: &str) -> Rig {
    let key = TestKey::generate();
    let http = serving(
        &key,
        &[Publish::new(Line::Stable, installed, testkit::artifact_bytes(64))],
    );
    rig(&key, installed, Channel::Stable, http)
}

/// A server with `publish` on it, signed by `key`.
fn serving(key: &TestKey, publish: &[Publish]) -> FakeHttp {
    let mut http = FakeHttp::default();
    for release in publish {
        release.serve(&mut http, key);
    }
    http
}

/// Pumps until nothing is out (at most 10 s): the launch check and every order sent so far answered. Every toast
/// on the way, in order.
fn settle(part: &mut UpdatesPart) -> Vec<&'static str> {
    let began = Instant::now();
    let mut toasts = Vec::new();
    loop {
        let (_, toast, busy) = part.pump();
        toasts.extend(toast);
        if !busy && part.state_for_test().is_some() {
            return toasts;
        }
        assert!(began.elapsed() < Duration::from_secs(10), "the updater did not settle");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn about(part: &UpdatesPart) -> About {
    let mut about = build_about();
    part.fill(&mut about);
    about
}

fn words(part: &UpdatesPart) -> String {
    about(part).check.says().to_string()
}

fn manifests(channel: Line) -> [String; 2] {
    let url = testkit::manifest_url(channel);
    [format!("{url}.sig"), url]
}

#[test]
fn check_now_says_the_newest_version_and_asks_for_the_channels_manifest_only() {
    let key = TestKey::generate();
    let mut rig = rig(
        &key,
        "0.9.1",
        Channel::Stable,
        serving(
            &key,
            &[Publish::new(Line::Stable, "0.9.1", testkit::artifact_bytes(4096))],
        ),
    );
    settle(&mut rig.part);
    assert!(
        words(&rig.part).starts_with("0.9.1 is the newest version · checked "),
        "{}",
        words(&rig.part)
    );
    let launch = rig.seen().len();
    assert_eq!(launch, 2, "the launch check: the manifest and its signature");

    rig.part.check_now();
    assert_eq!(
        about(&rig.part).check,
        UpdateCheck::Checking,
        "Checking… until it answers"
    );
    assert!(rig.part.pump().2, "busy: the window keeps looking");
    settle(&mut rig.part);
    assert!(words(&rig.part).starts_with("0.9.1 is the newest version · checked "));

    let seen = rig.seen();
    assert_eq!(seen.len(), 4, "{seen:?}");
    let allowed = manifests(Line::Stable);
    for url in &seen {
        assert!(
            allowed.contains(url),
            "only the channel's manifest and its signature: {url}"
        );
        // Nothing about the person: no account, no address, no query, no file of theirs.
        assert!(
            !url.contains('?') && !url.contains('@') && !url.contains("account"),
            "{url}"
        );
    }
    assert!(rig.did().is_empty());
    assert!(rig.staged().is_empty());
}

#[test]
fn a_newer_version_is_said_and_nothing_is_downloaded_by_a_check() {
    let key = TestKey::generate();
    let release = Publish::new(Line::Stable, "0.9.2", testkit::artifact_bytes(4096));
    let artifact = release.url.clone();
    let mut rig = rig(&key, "0.9.1", Channel::Stable, serving(&key, &[release]));
    settle(&mut rig.part);
    rig.part.check_now();
    settle(&mut rig.part);
    assert!(
        words(&rig.part).starts_with("Version 0.9.2 is available · checked "),
        "{}",
        words(&rig.part)
    );
    assert_eq!(about(&rig.part).notes.version, "0.9.2");
    assert_eq!(about(&rig.part).older, None);
    assert!(!rig.seen().contains(&artifact), "a check downloads nothing");
    assert!(rig.did().is_empty() && rig.staged().is_empty());
}

/// The file on the server is not the one the release signed (same length, other bytes): it is checked before it is
/// staged as an installer, deleted, and never handed off; "Restart to update" then does nothing. The honest file,
/// on a rig of its own, is handed off: the hand-off is visible to this test when it happens.
#[test]
fn a_file_that_is_not_the_signed_one_is_deleted_and_never_handed_off() {
    let key = TestKey::generate();
    let release = Publish::new(Line::Stable, "0.9.2", testkit::artifact_bytes(4096));
    let (artifact, name) = (release.url.clone(), testkit::artifact_name("0.9.2"));

    let mut honest = rig(&key, "0.9.1", Channel::Stable, serving(&key, &[release]));
    settle(&mut honest.part);
    honest.part.send_for_test(Event::Download);
    settle(&mut honest.part);
    assert!(
        matches!(honest.part.state_for_test(), Some(State::ReadyToRestart { .. })),
        "{:?}",
        honest.part.state_for_test()
    );
    honest.part.send_for_test(Event::Apply);
    settle(&mut honest.part);
    assert_eq!(honest.did(), ["stop", "launch"], "the honest file is handed off");

    let mut http = serving(
        &key,
        &[Publish::new(Line::Stable, "0.9.2", testkit::artifact_bytes(4096))],
    );
    let mut other = testkit::artifact_bytes(4096);
    other[100] ^= 0xff;
    http.files.insert(artifact.clone(), other);
    let mut rig = rig(&key, "0.9.1", Channel::Stable, http);
    settle(&mut rig.part);
    rig.part.send_for_test(Event::Download);
    settle(&mut rig.part);
    assert_eq!(
        rig.part.state_for_test(),
        Some(&State::Failed {
            failure: Failure::BadSignature,
            resume: None
        })
    );
    assert_eq!(words(&rig.part), NOT_SIGNED_FILE);
    assert!(rig.seen().contains(&artifact), "it was downloaded");
    assert!(
        !rig.staged().iter().any(|file| file.starts_with(&name)),
        "deleted, never staged: {:?}",
        rig.staged()
    );
    rig.part.send_for_test(Event::Apply);
    settle(&mut rig.part);
    assert!(rig.did().is_empty(), "never handed off: {:?}", rig.did());
}

#[test]
fn a_manifest_another_key_signed_downloads_nothing() {
    let (key, stranger) = (TestKey::generate(), TestKey::generate());
    let release = Publish::new(Line::Stable, "0.9.2", testkit::artifact_bytes(4096));
    let artifact = release.url.clone();
    let mut rig = rig(&key, "0.9.1", Channel::Stable, serving(&stranger, &[release]));
    settle(&mut rig.part);
    assert_eq!(
        rig.part.state_for_test(),
        Some(&State::Failed {
            failure: Failure::Refused(Refusal::ManifestSignature),
            resume: None
        })
    );
    assert_eq!(words(&rig.part), NOT_VERIFIED);
    assert!(!rig.seen().contains(&artifact));
    assert!(rig.did().is_empty() && rig.staged().is_empty());
}

/// No check has passed for two weeks or more (here: none since this build was made, 65 days ago): a refusal says so
/// too, so "up to date" is never the only thing a blocked install hears.
#[test]
fn a_refusal_with_nothing_verified_for_two_weeks_says_so() {
    let (key, stranger) = (TestKey::generate(), TestKey::generate());
    let release = Publish::new(Line::Stable, "0.9.2", testkit::artifact_bytes(64));
    let mut rig = rig_with(
        &key,
        "0.9.1",
        Channel::Stable,
        serving(&stranger, &[release]),
        testkit::BASE,
        None,
    );
    settle(&mut rig.part);
    let days = (testkit::NOW - beebeeb_updater::V2_KEY_CREATED) / 86_400;
    assert_eq!(
        words(&rig.part),
        format!("{NOT_VERIFIED} No update has been verified for {days} days.")
    );
}

#[test]
fn another_host_is_refused() {
    let key = TestKey::generate();
    // The artefact on another host, in a manifest our key signed: refused before a byte of it is asked for.
    let mut release = Publish::new(Line::Stable, "0.9.2", testkit::artifact_bytes(4096));
    release.url = format!(
        "https://releases.example.org/desktop/artifacts/0.9.2/{}",
        testkit::artifact_name("0.9.2")
    );
    let mut rig = rig(&key, "0.9.1", Channel::Stable, serving(&key, &[release]));
    settle(&mut rig.part);
    assert_eq!(words(&rig.part), OTHER_HOST);
    assert!(
        rig.seen()
            .iter()
            .all(|url| url.starts_with("https://releases.beebeeb.io/")),
        "{:?}",
        rig.seen()
    );

    // A base on another host: refused with not one request.
    let mut elsewhere = rig_at(
        &key,
        "0.9.1",
        Channel::Stable,
        serving(
            &key,
            &[Publish::new(Line::Stable, "0.9.2", testkit::artifact_bytes(4096))],
        ),
        "https://releases.example.org",
    );
    settle(&mut elsewhere.part);
    elsewhere.part.check_now();
    settle(&mut elsewhere.part);
    assert_eq!(words(&elsewhere.part), OTHER_HOST);
    assert!(elsewhere.seen().is_empty(), "{:?}", elsewhere.seen());
}

/// This PC runs 0.9.1 from Beta, and Stable is at 0.9.0. A background check holds it back without a word; choosing
/// Stable says so (the row and the words), and offers nothing: a download press stages nothing.
#[test]
fn an_older_version_on_the_chosen_channel_is_said_and_never_offered() {
    let key = TestKey::generate();
    let older = Publish::new(Line::Stable, "0.9.0", testkit::artifact_bytes(4096));
    let artifact = older.url.clone();
    let http = serving(
        &key,
        &[older, Publish::new(Line::Beta, "0.9.1", testkit::artifact_bytes(4096))],
    );
    let mut rig = rig(&key, "0.9.1", Channel::Beta, http);
    settle(&mut rig.part);
    assert_eq!(about(&rig.part).older, None);
    assert!(words(&rig.part).starts_with("0.9.1 is the newest version"));

    rig.part.set_channel(Channel::Stable);
    assert_eq!(
        about(&rig.part).channel,
        Channel::Beta,
        "shown once the settings file took it"
    );
    assert_eq!(about(&rig.part).check, UpdateCheck::Checking);
    assert!(settle(&mut rig.part).is_empty(), "saved without a word");
    let shown = about(&rig.part);
    assert_eq!(shown.channel, Channel::Stable);
    assert_eq!(
        shown.older,
        Some(OlderChannel {
            channel: Channel::Stable,
            version: "0.9.0".into(),
            runs: "0.9.1".into(),
            from: Channel::Beta,
        })
    );
    assert!(
        shown
            .check
            .says()
            .starts_with("No newer version on the Stable channel · checked "),
        "{}",
        shown.check.says()
    );
    assert!(
        rig.seen().contains(&testkit::manifest_url(Line::Stable)),
        "the new channel was asked"
    );

    rig.part.send_for_test(Event::Download);
    settle(&mut rig.part);
    assert!(!rig.seen().contains(&artifact), "never downloaded");
    assert!(rig.did().is_empty() && rig.staged().is_empty());

    // Back to the default, for the settings file other tests read.
    rig.part.set_channel(Channel::Stable);
    settle(&mut rig.part);
}

#[test]
fn a_second_channel_while_one_is_saving_waits_and_says_so() {
    let key = TestKey::generate();
    let http = serving(
        &key,
        &[Publish::new(Line::Stable, "0.9.1", testkit::artifact_bytes(64))],
    );
    let mut rig = rig(&key, "0.9.1", Channel::Stable, http);
    settle(&mut rig.part);
    rig.part.set_channel(Channel::Beta);
    rig.part.set_channel(Channel::Alpha);
    let toasts = settle(&mut rig.part);
    assert_eq!(toasts, [crate::engine_host::settings::STILL_SAVING]);
    assert_eq!(about(&rig.part).channel, Channel::Beta);
    rig.part.set_channel(Channel::Stable);
    settle(&mut rig.part);
    assert_eq!(about(&rig.part).channel, Channel::Stable);
}

/// What a release build runs with (`release_config`, which the Windows release path calls): the pinned key alone, the
/// one host, nothing below 0.9.0, the build's own time; no config at all without a build time or a release version.
/// On that config a release our test key signed is refused before anything is downloaded: only the pinned key is
/// trusted.
#[test]
fn the_release_config_is_the_pinned_key_and_the_one_host() {
    let folder = tempfile::tempdir().expect("a temp dir");
    let built = beebeeb_updater::V2_KEY_CREATED;
    assert!(
        release_config(None, Some("0.9.1"), folder.path()).is_none(),
        "no build time"
    );
    assert!(
        release_config(Some(built), None, folder.path()).is_none(),
        "no release version"
    );
    assert!(release_config(Some(built), Some("test build"), folder.path()).is_none());
    let config = release_config(Some(built), Some("0.9.1"), folder.path()).expect("a release");
    assert_eq!(config.base_url, "https://releases.beebeeb.io");
    assert_eq!(config.trust.allowed_hosts, ["releases.beebeeb.io"]);
    assert_eq!(config.trust.keys.len(), 1, "one key");
    // The key itself is `PublicKey::updater_v2()` through `host::windows_trust_built_at` (pinned by the crate's own
    // `a_release_build_trusts_one_key_one_host_and_never_goes_below_0_9_0`); here: it is not a key made in a test.
    let other = TestKey::generate();
    let signed = beebeeb_updater::Signature::from_base64(&other.sign(b"bytes", testkit::NOW, "stable.json"))
        .expect("a signature");
    assert!(config.trust.signer(&signed).is_none(), "a test key is not trusted");
    assert_eq!(config.trust.floor.to_string(), "0.9.0");
    assert_eq!(config.trust.not_before, built);
    assert_eq!(config.staging_dir, folder.path().join("updates"));

    let release = Publish::new(Line::Stable, "0.9.2", testkit::artifact_bytes(64));
    let artifact = release.url.clone();
    let seen = Seen::default();
    let http = Logged {
        inner: serving(&other, &[release]),
        seen: seen.clone(),
    };
    let clock = FakeClock(Cell::new(testkit::NOW));
    let mut updater = Updater::new(
        config,
        http,
        clock,
        MemStore::default(),
        Recording { did: Did::default() },
    );
    let mut state = updater
        .step(Event::Check {
            channel: Line::Stable,
            intent: Intent::Manual,
        })
        .clone();
    while busy(&state) {
        state = updater.step(Event::Tick).clone();
    }
    assert_eq!(
        state,
        State::Failed {
            failure: Failure::Refused(Refusal::ManifestSignature),
            resume: None
        }
    );
    let manifest = testkit::manifest_url(Line::Stable);
    assert_eq!(*seen.lock().unwrap(), [manifest.clone(), format!("{manifest}.sig")]);
    assert!(!seen.lock().unwrap().contains(&artifact));
}

/// An updater that panics ends its worker: About says the updater did not start, and is not left on Checking…
#[test]
fn a_worker_that_ends_says_so_and_is_not_left_checking() {
    struct Panics;
    impl Drive for Panics {
        fn step(&mut self, _event: Event) -> State {
            panic!("an updater that fails (test)");
        }
        fn background_check_due(&self) -> bool {
            false
        }
        fn freshness(&self) -> Freshness {
            Freshness::Verified
        }
    }
    let make: MakeDrive = Box::new(|| Box::new(Panics) as Box<dyn Drive>);
    let mut part = UpdatesPart::live(make, &TEST, Channel::Stable, Channel::Stable, "0.9.1".into());
    let began = Instant::now();
    while words(&part) != NO_UPDATER {
        assert!(began.elapsed() < Duration::from_secs(10), "{}", words(&part));
        part.pump();
        std::thread::sleep(Duration::from_millis(5));
    }
    part.check_now();
    let (_, _, busy) = part.pump();
    assert!(!busy, "nothing is out");
    assert_eq!(words(&part), NO_UPDATER);
}

/// A refusal never says "Try again in a moment": that is for no answer, or the server's own trouble, alone.
#[test]
fn each_failure_says_why_and_only_no_answer_says_try_again() {
    let refusals = [
        Refusal::ManifestSignature,
        Refusal::WrongChannel,
        Refusal::ManifestUnreadable,
        Refusal::StaleManifest,
        Refusal::NoBuildForThisPc,
        Refusal::UntrustedAddress,
        Refusal::BadSize,
        Refusal::ArtifactSignature,
        Refusal::SignatureForOtherFile,
        Refusal::BelowFloor,
        Refusal::KeyNotValidForVersion,
        Refusal::SignedInTheFuture,
        Refusal::OlderThanThisBuild,
        Refusal::NoBuildTime,
    ];
    for refusal in refusals {
        let says = failed(Failure::Refused(refusal), Freshness::Verified);
        assert_ne!(says, UpdateCheck::Failed, "{refusal:?}");
        assert!(!says.says().contains("Try again"), "{refusal:?}");
    }
    assert_eq!(
        failed(Failure::Refused(Refusal::UntrustedAddress), Freshness::Verified).says(),
        OTHER_HOST
    );
    assert_eq!(
        failed(Failure::BadSignature, Freshness::Verified).says(),
        NOT_SIGNED_FILE
    );
    assert_eq!(
        failed(Failure::ServerStatus(404), Freshness::Verified).says(),
        NOTHING_PUBLISHED
    );
    assert_eq!(failed(Failure::Offline, Freshness::Verified), UpdateCheck::Failed);
    assert_eq!(
        failed(Failure::ServerStatus(503), Freshness::Verified),
        UpdateCheck::Failed
    );
    let stale = failed(Failure::Offline, Freshness::Unverified { days: 20 });
    assert!(
        stale.says().contains("No update has been verified for 20 days."),
        "{}",
        stale.says()
    );
}

fn engine() -> Arc<Engine<MemoryCredentials>> {
    let (host, _messages) = ChannelHost::new(|| {});
    let config = EngineConfig {
        account_id: "updates-test-slot".into(),
        state_dir: crate::engine_host::tests::process_state_dir(),
        // A closed loopback port: nothing here asks Beebeeb.
        api_base: "http://127.0.0.1:9".to_string(),
    };
    let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens");
    engine.restore();
    Arc::new(engine)
}

/// The source's three methods and About on the part: without one (every test build, macOS, Linux) nothing of the
/// updater is offered; with one, Check now and the channel go to it and About draws what it answered.
#[test]
fn the_settings_source_answers_from_its_updater() {
    let engine = engine();
    let mut without = EngineSettingsSource::for_identity(engine.clone(), &TEST);
    assert!(!without.updates_available());
    assert!(!without.opens("DW-0789"));
    without.check_for_updates();
    assert_eq!(without.about().check, UpdateCheck::Failed, "the rest value, not drawn");

    let key = TestKey::generate();
    let release = Publish::new(Line::Stable, "0.9.2", testkit::artifact_bytes(4096));
    let rig = rig(&key, "0.9.1", Channel::Stable, serving(&key, &[release]));
    let (seen, part) = (rig.seen.clone(), rig.part);
    let mut source = EngineSettingsSource::for_identity(engine, &TEST).with_updates(Some(part));
    assert!(source.updates_available());
    assert!(source.opens("DW-0789"), "the channel menu");
    assert!(
        !source.opens("DW-0789b"),
        "the notes' Restart to update has no route yet"
    );
    let pump = |source: &mut EngineSettingsSource<MemoryCredentials>| {
        let began = Instant::now();
        while source.pump().busy || seen.lock().unwrap().len() < 2 {
            assert!(began.elapsed() < Duration::from_secs(10), "the updater did not settle");
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    pump(&mut source);
    source.check_for_updates();
    assert_eq!(source.about().check, UpdateCheck::Checking);
    pump(&mut source);
    assert!(
        source.about().check.says().starts_with("Version 0.9.2 is available"),
        "{}",
        source.about().check.says()
    );
    assert_eq!(seen.lock().unwrap().len(), 4, "the launch check and Check now");
    assert_eq!(
        source.general().update,
        crate::frames::settings::source::Update::Current,
        "General offers no download: no press sends one yet"
    );
    source.set_channel(Channel::Beta);
    pump(&mut source);
    assert_eq!(source.about().channel, Channel::Beta);
    assert!(seen.lock().unwrap().contains(&testkit::manifest_url(Line::Beta)));
    source.set_channel(Channel::Stable);
    pump(&mut source);
}

/// The real hand-off's first step on every platform: sync stops before anything is started (an engine with no sync
/// running counts as stopped, and mints the proof).
#[test]
fn the_hand_off_stops_sync_first() {
    let mut apply = EngineApply::new(engine(), &TEST);
    assert!(apply.stop_engine().is_some(), "no sync running: stopped");
}

/// Outside Windows no installer is ever started: the machine says so and keeps the staged file. Not run on Windows:
/// there it would start the staged test bytes.
#[cfg(not(windows))]
#[test]
fn the_hand_off_starts_nothing_outside_windows() {
    let engine = engine();
    let key = TestKey::generate();
    let release = Publish::new(Line::Stable, "0.9.2", testkit::artifact_bytes(4096));
    let http = serving(&key, &[release]);
    let staging = tempfile::tempdir().expect("a temp dir");
    let config = Config {
        trust: testkit::trust(&key),
        base_url: testkit::BASE.to_string(),
        installed: testkit::version("0.9.1"),
        staging_dir: staging.path().join("updates"),
    };
    let clock = FakeClock(Cell::new(testkit::NOW));
    let mut updater = Updater::new(
        config,
        http,
        clock,
        MemStore::default(),
        EngineApply::new(engine, &TEST),
    );
    let mut settle = |event| {
        let mut state = updater.step(event).clone();
        while busy(&state) {
            state = updater.step(Event::Tick).clone();
        }
        state
    };
    settle(Event::Check {
        channel: Line::Stable,
        intent: Intent::Manual,
    });
    assert!(matches!(settle(Event::Download), State::ReadyToRestart { .. }));
    assert!(matches!(
        settle(Event::Apply),
        State::Failed {
            failure: Failure::InstallerDidNotStart,
            resume: Some(_)
        }
    ));
}

/// The menu shows the channel the settings file holds; the build came from `installed_release_channel`, else from
/// the channel chosen when the app started.
#[test]
fn the_saved_channel_and_the_one_the_build_came_from() {
    assert_eq!(channels(None), (Channel::Stable, Channel::Stable));
    let mut config = DesktopConfig::default();
    config.release_channel = ReleaseChannel::Stable;
    config.installed_release_channel = Some(ReleaseChannel::Beta);
    assert_eq!(channels(Some(&config)), (Channel::Stable, Channel::Beta));
    config.release_channel = ReleaseChannel::Alpha;
    config.installed_release_channel = None;
    assert_eq!(channels(Some(&config)), (Channel::Alpha, Channel::Alpha));
}
