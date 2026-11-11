//! The state machine against a scripted release server: the honest path, then each way a download can go wrong.

use std::fs;

use crate::machine::{
    CHECK_INTERVAL_SECS, CHUNK_BYTES, Event, Failure, Intent, MAX_INTERRUPTIONS, RETRY_AFTER_FAILURE_SECS, State,
};
use crate::manifest::{Channel, MAX_MANIFEST_BYTES};
use crate::policy::Refusal;
use crate::testkit::{
    FakeHttp, MemStore, Publish, Serve, TestKey, TestUpdater, artifact_bytes, artifact_name, artifact_url, drive,
    manifest_url, settle, updater, version,
};

const SIZE: usize = 300_000;
const CHECK: Event = Event::Check {
    channel: Channel::Stable,
    intent: Intent::Background,
};
const MANUAL_CHECK: Event = Event::Check {
    channel: Channel::Stable,
    intent: Intent::Manual,
};

struct World {
    key: TestKey,
    dir: tempfile::TempDir,
}

impl World {
    fn new() -> Self {
        Self {
            key: TestKey::generate(),
            dir: tempfile::tempdir().expect("temp dir"),
        }
    }

    /// An install of `installed` facing a server that honestly publishes `published` on stable.
    fn updater(&self, installed: &str, published: &str) -> TestUpdater {
        let mut http = FakeHttp::default();
        Publish::new(Channel::Stable, published, artifact_bytes(SIZE)).serve(&mut http, &self.key);
        updater(&self.key, installed, self.dir.path(), http, MemStore::default())
    }

    fn staged(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.dir.path())
            .expect("staging dir")
            .map(|entry| entry.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

fn available(updater: &mut TestUpdater) {
    let state = settle(updater, CHECK);
    assert!(
        matches!(state, State::Available { .. }),
        "expected an offer, got {state:?}"
    );
}

#[test]
fn checks_downloads_verifies_stages_and_hands_over_only_on_apply() {
    let world = World::new();
    let mut updater = world.updater("0.9.0", "0.9.1");

    let states = drive(&mut updater, CHECK);
    assert_eq!(
        states[0],
        State::Checking {
            channel: Channel::Stable,
            intent: Intent::Background
        }
    );
    let State::Available { offer } = states.last().expect("a state").clone() else {
        panic!("expected an offer, got {states:?}")
    };
    assert_eq!((offer.version, offer.size), (version("0.9.1"), SIZE as u64));
    assert_eq!(
        updater.http.log.len(),
        2,
        "a check reads the manifest and its signature, nothing else"
    );

    let states = drive(&mut updater, Event::Download);
    assert_eq!(
        states[0],
        State::Downloading {
            offer: offer.clone(),
            received: 0,
            total: SIZE as u64
        }
    );
    assert_eq!(
        states[1],
        State::Downloading {
            offer: offer.clone(),
            received: CHUNK_BYTES as u64,
            total: SIZE as u64
        }
    );
    let progress: Vec<u64> = states
        .iter()
        .filter_map(|state| {
            if let State::Downloading { received, .. } = state {
                Some(*received)
            } else {
                None
            }
        })
        .collect();
    assert!(
        progress.windows(2).all(|pair| pair[0] < pair[1]),
        "progress only grows: {progress:?}"
    );
    assert!(states.contains(&State::Verifying { offer: offer.clone() }));
    let installer = world.dir.path().join(artifact_name("0.9.1"));
    assert_eq!(
        states.last(),
        Some(&State::ReadyToRestart {
            offer: offer.clone(),
            installer: installer.clone()
        })
    );
    assert_eq!(fs::read(&installer).expect("staged installer"), artifact_bytes(SIZE));
    assert_eq!(world.staged(), [artifact_name("0.9.1")]);
    assert!(
        updater.apply.launched.is_empty(),
        "nothing is started before the person asks"
    );

    assert_eq!(updater.step(Event::Apply), &State::Restarting { offer });
    assert_eq!(updater.apply.launched, [(installer, version("0.9.1"))]);
    assert_eq!(
        updater.http.requests_for(&artifact_url("0.9.1")),
        [0],
        "one request for the whole file"
    );
}

#[test]
fn up_to_date_clears_old_staged_files_and_records_the_check() {
    let world = World::new();
    fs::write(world.dir.path().join("Beebeeb_0.9.0_x64-setup.exe"), b"old").expect("leftover");
    let mut updater = world.updater("0.9.1", "0.9.1");
    assert!(updater.background_check_due(), "never checked");
    assert_eq!(
        settle(&mut updater, CHECK),
        State::UpToDate {
            installed: version("0.9.1"),
            channel: Channel::Stable
        }
    );
    assert_eq!(world.staged(), Vec::<String>::new());
    assert!(!updater.background_check_due());
    updater.clock.0.set(updater.clock.0.get() + CHECK_INTERVAL_SECS - 1);
    assert!(!updater.background_check_due());
    updater.clock.0.set(updater.clock.0.get() + 1);
    assert!(updater.background_check_due(), "due after four hours");
    updater.clock.0.set(0);
    assert!(
        updater.background_check_due(),
        "a clock that went backwards does not silence checks"
    );
    assert_eq!(updater.step(Event::Dismiss), &State::Idle);
}

#[test]
fn a_file_that_is_not_the_signed_one_is_deleted_and_never_started() {
    let world = World::new();
    let mut updater = world.updater("0.9.0", "0.9.1");
    // Same length, one byte different: only the signature can tell.
    let mut tampered = artifact_bytes(SIZE);
    tampered[SIZE / 2] ^= 1;
    updater.http.files.insert(artifact_url("0.9.1"), tampered);
    available(&mut updater);
    assert_eq!(
        settle(&mut updater, Event::Download),
        State::Failed {
            failure: Failure::BadSignature,
            resume: None
        }
    );
    assert_eq!(world.staged(), Vec::<String>::new(), "the bad file is gone");
    assert_eq!(
        updater.step(Event::Apply),
        &State::Failed {
            failure: Failure::BadSignature,
            resume: None
        },
        "Apply means nothing here"
    );
    assert!(updater.apply.launched.is_empty());
}

#[test]
fn a_dropped_connection_resumes_from_the_bytes_it_has() {
    let world = World::new();
    let mut updater = world.updater("0.9.0", "0.9.1");
    let url = artifact_url("0.9.1");
    updater.http.next(&url, Serve::DropAfter(100_000));
    updater.http.next(&url, Serve::Unreachable);
    available(&mut updater);
    let state = settle(&mut updater, Event::Download);
    assert!(
        matches!(state, State::ReadyToRestart { .. }),
        "expected a staged installer, got {state:?}"
    );
    assert_eq!(
        updater.http.requests_for(&url),
        [0, 100_000, 100_000],
        "the second and third requests ask for the rest only"
    );
    assert_eq!(
        fs::read(world.dir.path().join(artifact_name("0.9.1"))).expect("staged"),
        artifact_bytes(SIZE)
    );
}

#[test]
fn a_download_that_keeps_ending_early_stops_and_resumes_later() {
    let world = World::new();
    let mut updater = world.updater("0.9.0", "0.9.1");
    let url = artifact_url("0.9.1");
    updater.http.next(&url, Serve::EndAfter(70_000));
    for _ in 0..MAX_INTERRUPTIONS {
        updater.http.next(&url, Serve::EndAfter(0));
    }
    available(&mut updater);
    let State::Failed {
        failure,
        resume: Some(offer),
    } = settle(&mut updater, Event::Download)
    else {
        panic!("expected a resumable failure")
    };
    assert_eq!(
        failure,
        Failure::DownloadInterrupted {
            received: 70_000,
            total: SIZE as u64
        }
    );
    assert_eq!(
        world.staged(),
        [format!("{}.part", artifact_name("0.9.1"))],
        "the truncated file is kept as a part, never as an installer"
    );
    assert_eq!(
        updater.step(Event::Apply),
        &State::Failed {
            failure,
            resume: Some(offer)
        },
        "a truncated file cannot be applied"
    );
    assert!(updater.apply.launched.is_empty());

    // The server is well again: the person tries again and only the rest is fetched.
    let state = settle(&mut updater, Event::Download);
    assert!(
        matches!(state, State::ReadyToRestart { .. }),
        "expected a staged installer, got {state:?}"
    );
    assert_eq!(updater.http.requests_for(&url).last(), Some(&70_000));
}

#[test]
fn a_server_that_ignores_the_range_starts_the_file_over() {
    let world = World::new();
    let mut updater = world.updater("0.9.0", "0.9.1");
    let url = artifact_url("0.9.1");
    updater.http.next(&url, Serve::DropAfter(100_000));
    updater.http.next(&url, Serve::IgnoreRange);
    available(&mut updater);
    let state = settle(&mut updater, Event::Download);
    assert!(
        matches!(state, State::ReadyToRestart { .. }),
        "expected a staged installer, got {state:?}"
    );
    assert_eq!(
        fs::read(world.dir.path().join(artifact_name("0.9.1"))).expect("staged"),
        artifact_bytes(SIZE),
        "not 100000 stale bytes followed by the whole file"
    );
}

#[test]
fn a_range_answer_from_another_offset_is_refused() {
    let world = World::new();
    let mut updater = world.updater("0.9.0", "0.9.1");
    let url = artifact_url("0.9.1");
    updater.http.next(&url, Serve::DropAfter(100_000));
    updater.http.next(&url, Serve::RangeFrom(50_000));
    available(&mut updater);
    assert!(matches!(
        settle(&mut updater, Event::Download),
        State::Failed {
            failure: Failure::ServerStatus(206),
            resume: Some(_)
        }
    ));
}

#[test]
fn a_length_that_is_not_the_manifests_is_refused() {
    let world = World::new();
    let mut updater = world.updater("0.9.0", "0.9.1");
    let url = artifact_url("0.9.1");

    // The server states another total than the signed manifest.
    updater.http.next(&url, Serve::ClaimTotal(SIZE as u64 + 1));
    available(&mut updater);
    assert_eq!(
        settle(&mut updater, Event::Download),
        State::Failed {
            failure: Failure::WrongSize,
            resume: None
        }
    );
    assert_eq!(world.staged(), Vec::<String>::new());

    // The server sends more than the manifest's length: the extra bytes are never read, and what was read is not
    // the signed file.
    let mut longer = vec![0xEE; 10];
    longer.extend(artifact_bytes(SIZE));
    updater.http.files.insert(url.clone(), longer);
    updater.http.next(&url, Serve::ClaimTotal(SIZE as u64));
    available(&mut updater);
    assert_eq!(
        settle(&mut updater, Event::Download),
        State::Failed {
            failure: Failure::BadSignature,
            resume: None
        }
    );
    assert_eq!(world.staged(), Vec::<String>::new());
}

#[test]
fn a_staged_installer_changed_before_apply_is_not_started() {
    let world = World::new();
    let mut updater = world.updater("0.9.0", "0.9.1");
    available(&mut updater);
    let State::ReadyToRestart { installer, .. } = settle(&mut updater, Event::Download) else {
        panic!("expected a staged installer")
    };

    let mut swapped = artifact_bytes(SIZE);
    swapped[7] ^= 0x80;
    fs::write(&installer, &swapped).expect("swap the staged file");
    assert_eq!(
        updater.step(Event::Apply),
        &State::Failed {
            failure: Failure::BadSignature,
            resume: None
        }
    );
    assert!(updater.apply.launched.is_empty());
    assert!(!installer.exists());

    // Cut short instead: refused by its length, before a byte is hashed.
    available(&mut updater);
    assert!(matches!(
        settle(&mut updater, Event::Download),
        State::ReadyToRestart { .. }
    ));
    fs::write(&installer, &artifact_bytes(SIZE)[..SIZE - 1]).expect("truncate the staged file");
    assert_eq!(
        updater.step(Event::Apply),
        &State::Failed {
            failure: Failure::WrongSize,
            resume: None
        }
    );
    assert!(updater.apply.launched.is_empty());
    assert!(!installer.exists());
}

#[test]
fn an_installer_that_does_not_start_keeps_the_download() {
    let world = World::new();
    let mut updater = world.updater("0.9.0", "0.9.1");
    available(&mut updater);
    assert!(matches!(
        settle(&mut updater, Event::Download),
        State::ReadyToRestart { .. }
    ));
    updater.apply.refuse = true;
    assert!(matches!(
        updater.step(Event::Apply),
        State::Failed {
            failure: Failure::InstallerDidNotStart,
            resume: Some(_)
        }
    ));

    updater.apply.refuse = false;
    let before = updater.http.log.len();
    assert!(
        matches!(settle(&mut updater, Event::Download), State::ReadyToRestart { .. }),
        "verified again from disk"
    );
    assert_eq!(updater.http.log.len(), before, "nothing is downloaded twice");
    assert!(matches!(updater.step(Event::Apply), State::Restarting { .. }));
}

#[test]
fn server_trouble_is_reported_as_what_it_was() {
    let world = World::new();
    let mut updater = world.updater("0.9.0", "0.9.1");
    let manifest = manifest_url(Channel::Stable);

    updater.http.next(&manifest, Serve::Unreachable);
    assert_eq!(
        settle(&mut updater, CHECK),
        State::Failed {
            failure: Failure::Offline,
            resume: None
        }
    );
    updater.http.next(&manifest, Serve::Status(503));
    assert_eq!(
        settle(&mut updater, CHECK),
        State::Failed {
            failure: Failure::ServerStatus(503),
            resume: None
        }
    );
    updater.http.next(&format!("{manifest}.sig"), Serve::Status(404));
    assert_eq!(
        settle(&mut updater, CHECK),
        State::Failed {
            failure: Failure::ServerStatus(404),
            resume: None
        },
        "a manifest without its signature is no manifest"
    );
    assert!(!updater.background_check_due(), "but the next one waits a while");
    updater.clock.0.set(updater.clock.0.get() + RETRY_AFTER_FAILURE_SECS);
    assert!(
        updater.background_check_due(),
        "a failed check does not count as a check"
    );

    updater.http.next(&artifact_url("0.9.1"), Serve::Status(404));
    available(&mut updater);
    assert!(matches!(
        settle(&mut updater, Event::Download),
        State::Failed {
            failure: Failure::ServerStatus(404),
            resume: Some(_)
        }
    ));

    updater.http.files.insert(manifest, vec![b' '; MAX_MANIFEST_BYTES + 1]);
    assert_eq!(
        settle(&mut updater, MANUAL_CHECK),
        State::Failed {
            failure: Failure::Refused(Refusal::ManifestUnreadable),
            resume: None
        }
    );
}

#[test]
fn a_replayed_older_manifest_is_refused_after_a_restart() {
    let world = World::new();
    let mut first = world.updater("0.9.0", "0.9.1");
    available(&mut first);
    let store = MemStore {
        persisted: first.store.persisted.clone(),
        refuse_writes: false,
    };
    assert_eq!(store.persisted.seen[&Channel::Stable].manifest_time, 1_800_000_000);

    // The app restarts; the server now replays an older, validly signed manifest.
    let mut http = FakeHttp::default();
    let mut replay = Publish::new(Channel::Stable, "0.9.1", artifact_bytes(SIZE));
    replay.manifest_time = 1_790_000_000;
    replay.serve(&mut http, &world.key);
    let mut second = updater(&world.key, "0.9.0", world.dir.path(), http, store);
    assert_eq!(
        settle(&mut second, CHECK),
        State::Failed {
            failure: Failure::Refused(Refusal::StaleManifest),
            resume: None
        }
    );
}

#[test]
fn a_check_that_cannot_be_recorded_fails() {
    let world = World::new();
    let mut updater = world.updater("0.9.0", "0.9.1");
    updater.store.refuse_writes = true;
    assert_eq!(
        settle(&mut updater, CHECK),
        State::Failed {
            failure: Failure::Disk,
            resume: None
        }
    );
    updater.clock.0.set(updater.clock.0.get() + RETRY_AFTER_FAILURE_SECS);
    assert!(
        updater.background_check_due(),
        "a check that was not recorded does not count as one"
    );
}

#[test]
fn an_older_channel_build_is_held_in_the_background_and_needs_its_version_named() {
    let world = World::new();
    let mut updater = world.updater("0.9.4", "0.9.2");
    assert_eq!(
        settle(&mut updater, CHECK),
        State::UpToDate {
            installed: version("0.9.4"),
            channel: Channel::Stable
        },
        "a background check never offers an older build"
    );

    let State::Available { offer } = settle(&mut updater, MANUAL_CHECK) else {
        panic!("a manual check reports the older build")
    };
    assert!(offer.downgrade);
    assert_eq!(
        updater.step(Event::Download),
        &State::Available { offer: offer.clone() },
        "the upgrade button does not start a downgrade"
    );
    assert_eq!(
        updater.step(Event::DownloadDowngrade(version("0.9.3"))),
        &State::Failed {
            failure: Failure::DowngradeChanged,
            resume: None
        }
    );

    assert!(matches!(settle(&mut updater, MANUAL_CHECK), State::Available { .. }));
    assert!(matches!(
        settle(&mut updater, Event::DownloadDowngrade(version("0.9.2"))),
        State::ReadyToRestart { .. }
    ));
}

#[test]
fn a_check_never_interrupts_a_download() {
    let world = World::new();
    let mut updater = world.updater("0.9.0", "0.9.1");
    available(&mut updater);
    let downloading = updater.step(Event::Download).clone();
    assert!(matches!(downloading, State::Downloading { .. }));
    assert_eq!(updater.step(MANUAL_CHECK), &downloading);
    assert_eq!(updater.step(Event::Dismiss), &downloading);
    assert_eq!(updater.step(Event::Apply), &downloading);
}

#[test]
fn a_newer_release_replaces_what_was_staged_for_an_older_one() {
    let world = World::new();
    let mut updater = world.updater("0.9.0", "0.9.1");
    available(&mut updater);
    assert!(matches!(
        settle(&mut updater, Event::Download),
        State::ReadyToRestart { .. }
    ));
    assert_eq!(updater.step(Event::Dismiss), &State::Idle);

    let mut next = Publish::new(Channel::Stable, "0.9.2", artifact_bytes(SIZE + 5));
    next.manifest_time += 3_600;
    next.serve(&mut updater.http, &world.key);
    available(&mut updater);
    assert!(matches!(
        settle(&mut updater, Event::Download),
        State::ReadyToRestart { .. }
    ));
    assert_eq!(world.staged(), [artifact_name("0.9.2")]);
}

#[test]
fn a_background_check_never_takes_away_an_offer_or_a_staged_update() {
    let world = World::new();
    let mut updater = world.updater("0.9.0", "0.9.1");
    available(&mut updater);
    let offered = updater.state().clone();
    // Offline now: a background check would end in Failed and lose the offer.
    updater.http.next(&manifest_url(Channel::Stable), Serve::Unreachable);
    assert_eq!(updater.step(CHECK), &offered);

    let ready = settle(&mut updater, Event::Download);
    assert!(matches!(ready, State::ReadyToRestart { .. }));
    assert_eq!(updater.step(CHECK), &ready, "\"Restart to update\" stays");
    assert!(
        matches!(updater.step(MANUAL_CHECK), State::Checking { .. }),
        "the person's own check always runs"
    );
}

#[test]
fn a_background_check_keeps_a_download_that_can_be_resumed() {
    let world = World::new();
    let mut updater = world.updater("0.9.0", "0.9.1");
    let url = artifact_url("0.9.1");
    for _ in 0..MAX_INTERRUPTIONS {
        updater.http.next(&url, Serve::Unreachable);
    }
    available(&mut updater);
    let stopped = settle(&mut updater, Event::Download);
    assert!(matches!(stopped, State::Failed { resume: Some(_), .. }));
    assert_eq!(updater.step(CHECK), &stopped);
}

#[test]
fn a_staged_file_that_cannot_be_read_is_kept_not_deleted() {
    let world = World::new();
    let mut updater = world.updater("0.9.0", "0.9.1");
    available(&mut updater);
    let State::ReadyToRestart { installer, .. } = settle(&mut updater, Event::Download) else {
        panic!("expected a staged installer")
    };

    // Unreadable for a moment (here: moved away, as a scanner's lock would make it): nothing is deleted or started.
    let aside = world.dir.path().join("held-by-a-scanner");
    fs::rename(&installer, &aside).expect("move aside");
    assert!(matches!(
        updater.step(Event::Apply),
        State::Failed {
            failure: Failure::Disk,
            resume: Some(_)
        }
    ));
    assert!(updater.apply.launched.is_empty());
    assert!(aside.exists());

    fs::rename(&aside, &installer).expect("move back");
    let before = updater.http.log.len();
    assert!(matches!(
        settle(&mut updater, Event::Download),
        State::ReadyToRestart { .. }
    ));
    assert_eq!(
        updater.http.log.len(),
        before,
        "the file on disk is used; nothing is downloaded again"
    );
}
